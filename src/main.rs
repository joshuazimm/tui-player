// std includes
use std::fs::OpenOptions;
use std::io::Write;
use std::io::stdout;
use std::path::PathBuf;
use std::sync::{
    mpsc::Receiver,
};
use std::thread;
use std::time::{Duration, Instant};

// internal includes
mod resampler;
use resampler::{ResamplerKind, SimpleResampler};
mod widget;
use widget::{Layout, Rect, Section};
mod widgets;
use widgets::{AlbumArt, Controls, FileList, Label, ProgressBar, WidgetKind};
mod keyhandler;
use keyhandler::KeyHandler;
mod decoder;
use decoder::{decoder_from_path};
mod state;
use state::{GlobalState, StateKey};

// external includes
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossterm::{
    cursor::{Hide, Show},
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
                    state.set_playback_position(0.0); // Reset position for new track
                    true
                } else {
                    false
                }
            },
            &mut prod,
        );
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
    execute!(out, EnterAlternateScreen, Hide)?;

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

    let rb = HeapRb::<f32>::new(device_rate * 2);
    let (prod, mut cons): (HeapProd<f32>, HeapCons<f32>) = rb.split();

    // Create widgets with simplified constructors
    let file_list = FileList::new(PathBuf::from("test-tracks/Woodland"), state.clone());
    //let controls = Controls::new();
    let progress_bar = ProgressBar::new();
    let album_art = AlbumArt::new(false);
    let track_title_label = Label::new("No Track".to_string(), true, false)
        .with_subscription(vec![StateKey::CurrentTrackTitle]);

    // Create layout with updated sections
    let sections = vec![
        Section::new(
            Rect {
                x: 0.0,
                y: 0.0,
                width: 0.3,
                height: 0.5,
            },
            WidgetKind::FileList(file_list),
        ),
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
        Section::new(
            Rect {
                x: 0.3,
                y: 0.0,
                width: 0.7,
                height: 0.9,
            },
            WidgetKind::AlbumArt(album_art),
        ),
    ];

    let mut layout = Layout::new(sections);
    
    // Initialize all widgets with state (this sets up subscriptions)
    layout.init_all(&state);

    // Spawn unified key handler
    let mut key_handler = KeyHandler::new(state.clone());
    key_handler.start();

    let state_clone = state.clone();

    // Spawn decoder thread
    thread::spawn(move || {
        // Wait for initial track to be selected
        if let Ok(path) = track_rx.recv() {
            audio_thread(path, prod, device_rate, track_rx, state_clone);
        }
    });

    // CPAL output with state integration
    let mut last_sample = 0.0f32;
    let channels = wanted_config.channels() as u64;
    let state_for_audio = state.clone();
    let mut frames_played = 0u64;
    
    let stream = device.build_output_stream(
        &wanted_config.into(),
        move |output: &mut [f32], _: &cpal::OutputCallbackInfo| {
            let mut consumed_any = false;

            // Check if we should be playing
            if state_for_audio.is_playing() {
                for sample in output.iter_mut() {
                    if let Some(s) = cons.try_pop() {
                        last_sample = s;
                        consumed_any = true;
                    }
                    let vol = state_for_audio.volume();
                    *sample = (last_sample * vol).clamp(-1.0, 1.0);
                }

                if consumed_any {
                    let frames_written = (output.len() as u64) / channels;
                    frames_played += frames_written;
                    
                    // Update playback position in state
                    let position = frames_played as f64 / device_rate as f64;
                    state_for_audio.set_playback_position(position);
                }
            } else {
                // Paused - output silence
                for sample in output.iter_mut() {
                    *sample = 0.0;
                }
            }
        },
        |err| eprintln!("Stream error: {:?}", err),
        None,
    )?;
    
    stream.play()?;

    let sleeper = SpinSleeper::default();
    let mut next_frame = Instant::now();
    let mut last_resync = Instant::now();
    let mut fps_update_timer = Instant::now();
    let mut frame_count = 0u64;

    // Main render loop - much simpler now!
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
    execute!(out, LeaveAlternateScreen, Show)?;
    terminal::disable_raw_mode()?;
    Ok(())
}