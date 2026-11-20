// std includes
use std::fs::OpenOptions;
use std::io::Write;
use std::io::stdout;
use std::path::PathBuf;
use std::sync::{
    mpsc::Receiver,
    Arc,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

// internal includes
mod resampler;
use resampler::{ResamplerKind, SimpleResampler};
mod widget;
use widget::{Layout, Rect, Section};
mod widgets;
use widgets::{AlbumArt, Controls, FileList, Label, ProgressBar, Spectrum, WidgetKind};
mod keyhandler;
use keyhandler::KeyHandler;
mod decoder;
use decoder::{decoder_from_path};
mod state;
use state::{GlobalState, StateKey};
mod analysis;
use analysis::{DEFAULT_NUM_BANDS, FFT_SIZE, spawn_analysis_thread};

// external includes
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossterm::{
    cursor::{Hide, Show},
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use ringbuf::{
    HeapCons, HeapProd, HeapRb,
    traits::{Consumer, Producer, Split},
};
use spin_sleep::SpinSleeper;

use crate::widgets::fps_counter;

fn process_audio_chunks<F, C>(
    mut resampler: SimpleResampler,
    channels: usize,
    chunk_size: usize,
    mut fill_buffers: F,
    mut check_path: C,
    prod: &mut HeapProd<f32>,
) where
    F: FnMut(&mut Vec<Vec<f32>>) -> (usize, bool),
    C: FnMut() -> bool,
{
    let mut buffers: Vec<Vec<f32>> = vec![Vec::with_capacity(chunk_size); channels];
    let mut out_buf = resampler.output_buffer_allocate(true);

    let mut eof = false;
    loop {
        if !eof {
            for ch in 0..channels {
                buffers[ch].truncate(0);
            }

            // Fill buffers using decoder callback
            let (frames_collected, temp_eof) = fill_buffers(&mut buffers);
            eof = temp_eof;

            if frames_collected > 0 {
                // Process the chunk
                let input_refs: Vec<&[f32]> = buffers.iter().map(|b| b.as_slice()).collect();
                let (_in_used, out_written) = if frames_collected < chunk_size {
                    resampler
                        .process_partial_into_buffer(&input_refs, &mut out_buf, None)
                        .unwrap()
                } else {
                    resampler
                        .process_into_buffer(&input_refs, &mut out_buf, None)
                        .unwrap()
                };

                // Convert resampler output buffer to Vec<Vec<f32>> format
                let output_vecs: Vec<Vec<f32>> = (0..channels)
                    .map(|ch| out_buf[ch][..out_written].to_vec())
                    .collect();
                interleave_and_push(&output_vecs, out_written, prod);
            }
        } else {
            thread::sleep(Duration::from_millis(50));
        }

        if check_path() {
            break;
        }
    }
}

// Audio processing remains mostly the same but now updates global state
fn audio_thread(
    initial_path: PathBuf,
    mut prod: ringbuf::HeapProd<f32>,
    device_rate: usize,
    track_rx: Receiver<PathBuf>,
    state: GlobalState,
    flush_flag: Arc<AtomicBool>,
) {
    let chunk_size = 4096;
    let sub_chunks = 1;

    let mut current_path = initial_path;

    loop {
        let mut decoder = match decoder_from_path(&current_path) {
            Ok(decoder) => decoder,
            Err(e) => {
                let mut file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open("output.txt")
                    .expect("Failed to open output.txt");
                writeln!(
                    file,
                    "Failed to create decoder for {:?}: {}",
                    current_path, e
                )
                .expect("Failed to write to output.txt");
                
                match track_rx.recv() {
                    Ok(new_path) => {
                        current_path = new_path;
                        continue;
                    }
                    Err(_) => return,
                }
            }
        };
        
        let channels = decoder.channels();
        let file_rate = decoder.sample_rate();
        
        // Update state instead of atomic counters
        let duration_seconds = decoder.total_samples() as f64 / decoder.sample_rate() as f64;
        state.set_track_duration(duration_seconds);
        state.set_current_track(current_path.clone(), None, None, None);

        let resampler = SimpleResampler::new(
            ResamplerKind::Fast,
            file_rate,
            device_rate,
            chunk_size,
            sub_chunks,
            channels,
        )
        .unwrap();

        process_audio_chunks(
            resampler,
            channels,
            chunk_size,
            |buffers: &mut Vec<Vec<f32>>| -> (usize, bool) {
                let mut frames_collected = 0;
                let mut eof = false;

                while frames_collected < chunk_size {
                    for ch in 0..channels {
                        match decoder.next_sample() {
                            Some(sample) => buffers[ch].push(sample),
                            None => {
                                eof = true;
                                break;
                            }
                        }
                    }
                    if eof {
                        break;
                    }
                    frames_collected += 1;
                }

                (frames_collected, eof)
            },
            || -> bool {
                if let Ok(new_path) = track_rx.try_recv() {
                    current_path = new_path;
                    state.set_playback_position(0.0);
                    // Signal CPAL to drain the ring buffer before new audio starts
                    flush_flag.store(true, Ordering::SeqCst);
                    true
                } else {
                    false
                }
            },
            &mut prod,
        );

        // Wait for the CPAL callback to finish draining the ring buffer
        // before the new decoder starts pushing audio into it.
        while flush_flag.load(Ordering::Acquire) {
            thread::sleep(Duration::from_millis(5));
        }
    }
}

fn interleave_and_push(buffers: &[Vec<f32>], frames: usize, prod: &mut HeapProd<f32>) {
    let channels = buffers.len();
    for i in 0..frames {
        for ch in 0..channels {
            loop {
                if prod.try_push(buffers[ch][i]).is_ok() {
                    break;
                }
                thread::sleep(Duration::from_micros(50));
            }
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    const FRAME_TIME: Duration = Duration::from_nanos(1_000_000_000 / 60); // 60 FPS

    let mut out = stdout();
    terminal::enable_raw_mode()?;
    execute!(out, EnterAlternateScreen, Hide, EnableMouseCapture)?;

    // Initialize global state
    let state = GlobalState::new();

    let (track_tx, track_rx) = std::sync::mpsc::channel::<PathBuf>();

    let host = cpal::default_host();
    let device = host.default_output_device().unwrap();
    let wanted_config = device
        .supported_output_configs()?
        .find(|c| c.channels() == 2 && c.sample_format() == cpal::SampleFormat::F32)
        .unwrap()
        .clone()
        .with_sample_rate(device.default_output_config()?.sample_rate());

    let device_rate = wanted_config.sample_rate().0 as usize;

    let flush_flag = Arc::new(AtomicBool::new(false));

    let rb = HeapRb::<f32>::new(device_rate * 2);
    let (prod, mut cons): (HeapProd<f32>, HeapCons<f32>) = rb.split();

    // Secondary ring buffer fed from the CPAL callback for spectrum analysis.
    // Capacity: 4 FFT windows × stereo.
    let analysis_rb = HeapRb::<f32>::new(FFT_SIZE * 4 * 2);
    let (mut analysis_prod, analysis_cons) = analysis_rb.split();

    // Create widgets with simplified constructors
    let file_list_rect = Rect { x: 0.0, y: 0.0, width: 0.3, height: 0.5 };
    let file_list = FileList::new(PathBuf::from("C:/Users/Joshua Zimmerman/Media/Music"), file_list_rect, state.clone());
    //let controls = Controls::new();
    let progress_bar = ProgressBar::new();
    let album_art = AlbumArt::new(false);
    let band_centers = analysis::band_centers(DEFAULT_NUM_BANDS, device_rate as f32);
    let spectrum = Spectrum::new(band_centers);
    let track_title_label = Label::new("No Track".to_string(), true, false)
        .with_subscription(vec![StateKey::CurrentTrackTitle]);

    // Create layout with updated sections
    let sections = vec![
        Section::new(file_list_rect, WidgetKind::FileList(file_list)),
        Section::new(
            Rect {
                x: 0.0,
                y: 0.5,
                width: 0.3,
                height: 0.2,
            },
            WidgetKind::Label(track_title_label),
        ),
        Section::new(
            Rect {
                x: 0.0,
                y: 0.7,
                width: 0.3,
                height: 0.2,
            },
            WidgetKind::FpsCounter(fps_counter::FpsCounter::new()),
        ),
        Section::new(
            Rect {
                x: 0.0,
                y: 0.9,
                width: 1.0,
                height: 0.1,
            },
            WidgetKind::ProgressBar(progress_bar),
        ),
        // Album art takes the upper portion of the right panel
        Section::new(
            Rect {
                x: 0.3,
                y: 0.0,
                width: 0.7,
                height: 0.55,
            },
            WidgetKind::AlbumArt(album_art),
        ),
        // Spectrum visualiser occupies the lower portion of the right panel
        Section::new(
            Rect {
                x: 0.3,
                y: 0.55,
                width: 0.7,
                height: 0.35,
            },
            WidgetKind::Spectrum(spectrum),
        ),
    ];

    let mut layout = Layout::new(sections, state.clone());
    
    // Initialize all widgets with state (this sets up subscriptions)
    layout.init_all(&state);

    // Spawn unified key handler
    let (mouse_tx, mouse_rx) = std::sync::mpsc::channel();
    let mut key_handler = KeyHandler::new(state.clone(), mouse_tx);
    key_handler.start();
    layout.set_mouse_receiver(mouse_rx);

    let state_clone = state.clone();

    let flush_flag_audio = flush_flag.clone();

    // Spawn decoder thread
    thread::spawn(move || {
        // Wait for initial track to be selected
        if let Ok(path) = track_rx.recv() {
            audio_thread(path, prod, device_rate, track_rx, state_clone, flush_flag_audio);
        }
    });

    // CPAL output with state integration
    let mut last_sample = 0.0f32;
    let channels = wanted_config.channels() as u64;
    let state_for_audio = state.clone();
    let mut frames_played = 0u64;
    let mut last_track_path = state_for_audio.current_track_path();

    let flush_flag_cpal = flush_flag.clone();

    let stream = device.build_output_stream(
        &wanted_config.into(),
        move |output: &mut [f32], _: &cpal::OutputCallbackInfo| {
            // If the audio thread signalled a track change, drain stale audio
            // from the ring buffer and output silence until it's empty.
            if flush_flag_cpal.load(Ordering::Acquire) {
                while cons.try_pop().is_some() {}
                last_sample = 0.0;
                flush_flag_cpal.store(false, Ordering::Release);
                for sample in output.iter_mut() {
                    *sample = 0.0;
                    let _ = analysis_prod.try_push(0.0);
                }
                return;
            }

            let mut consumed_any = false;

            // If the current track changed, reset the local frame counter so playback position restarts at 0
            let current_track = state_for_audio.current_track_path();
            if current_track != last_track_path {
                frames_played = 0;
                last_track_path = current_track;
                // keep state consistent
                state_for_audio.set_playback_position(0.0);
            }

            // Check if we should be playing
            if state_for_audio.is_playing() {
                for sample in output.iter_mut() {
                    if let Some(s) = cons.try_pop() {
                        last_sample = s;
                        consumed_any = true;
                    }
                    let vol = state_for_audio.volume();
                    *sample = (last_sample * vol).clamp(-1.0, 1.0);
                    // Feed the PRE-volume sample so the spectrum is volume-independent
                    let _ = analysis_prod.try_push(last_sample);
                }

                if consumed_any {
                    let frames_written = (output.len() as u64) / channels;
                    frames_played += frames_written;

                    // Update playback position in state
                    let position = frames_played as f64 / device_rate as f64;
                    state_for_audio.set_playback_position(position);
                }
            } else {
                // Paused - output silence and push zeros so analysis sees silence
                for sample in output.iter_mut() {
                    *sample = 0.0;
                    let _ = analysis_prod.try_push(0.0);
                }
            }
        },
        |err| eprintln!("Stream error: {:?}", err),
        None,
    )?;
    
    stream.play()?;

    // Spawn the FFT analysis + beat detection thread
    spawn_analysis_thread(analysis_cons, 2, device_rate, DEFAULT_NUM_BANDS, state.clone());

    let sleeper = SpinSleeper::default();
    let mut next_frame = Instant::now();
    let mut last_resync = Instant::now();
    let mut fps_update_timer = Instant::now();
    let mut frame_count = 0u64;

    // Main render loop
    loop {
        if state.should_quit() {
            break;
        }

        // Update terminal size in state
        state.update_terminal_size();

        // Update widgets/state
        layout.update_all(&state)?;
        
        // If UI requested a track change, forward current path to audio thread
        if state.get(&StateKey::NewTrackRequested).and_then(|v| v.as_bool()) == Some(true) {
            if let Some(path) = state.current_track_path() {
                // Ignore send errors (audio thread may have exited)
                let _ = track_tx.send(path);
            }
            state.set_new_track_requested(false);
        }

        frame_count += 1;
        if fps_update_timer.elapsed() >= Duration::from_millis(500) {
            let elapsed_seconds = fps_update_timer.elapsed().as_secs_f64();
            let current_fps = frame_count as f64 / elapsed_seconds;
            
            // Update FPS in global state
            state.set_fps(current_fps);
            
            frame_count = 0;
            fps_update_timer = Instant::now();
        }

        // Only render if something actually changed
        layout.render_all(&mut out, false)?;
        
        // Frame pacing
        next_frame += FRAME_TIME;
        let now = Instant::now();

        if next_frame > now {
            sleeper.sleep(next_frame - now);
        } else {
            next_frame = now;
        }

        // Periodic resync
        if last_resync.elapsed() >= Duration::from_secs(2) {
            next_frame = Instant::now();
            last_resync = next_frame;
        }
    }

    key_handler.stop();
    execute!(out, LeaveAlternateScreen, Show, DisableMouseCapture)?;
    terminal::disable_raw_mode()?;
    Ok(())
}