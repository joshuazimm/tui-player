// std includes
use std::fs::OpenOptions;
use std::io::Write;
use std::io::stdout;
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
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
use widgets::{AlbumArt, Controls, FileList, Label, WidgetKind};
mod keyhandler;
use keyhandler::KeyHandler;
mod decoder;
use decoder::{decoder_from_path};

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

fn process_audio_chunks<F>(
    mut resampler: SimpleResampler,
    channels: usize,
    chunk_size: usize,
    mut fill_buffers: F,
    prod: &mut HeapProd<f32>,
) where
    F: FnMut(&mut Vec<Vec<f32>>) -> (usize, bool),
{
    let mut buffers: Vec<Vec<f32>> = vec![Vec::with_capacity(chunk_size); channels];
    let mut out_buf = resampler.output_buffer_allocate(true);

    loop {
        // Clear buffers for the new chunk
        for ch in 0..channels {
            buffers[ch].truncate(0);
        }

        // Fill buffers using decoder callback
        let (frames_collected, eof) = fill_buffers(&mut buffers);

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

        if eof {
            break;
        }
    }
}

fn audio_thread(
    initial_path: PathBuf,
    mut prod: ringbuf::HeapProd<f32>,
    device_rate: usize,
    track_rx: Receiver<PathBuf>,
    frame_counter: Arc<AtomicU64>,
) {
    let chunk_size = 4096;
    let sub_chunks = 1;

    let mut current_path = initial_path;

    /*let mut decoder = match decoder_from_path(&current_path) {
        Ok(decoder) => decoder,
        Err(e) => {
            eprintln!("Failed to create decoder for {:?}: {}", current_path, e);
            return;
        }
    };*/
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
            return;
        }
    };
    let channels = decoder.channels();
    let file_rate = decoder.sample_rate();

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

                // Check for track change
                if let Ok(new_path) = track_rx.try_recv() {
                    current_path = new_path;
                    decoder = match decoder_from_path(&current_path) {
                        Ok(decoder) => decoder,
                        Err(e) => {
                            eprintln!("Failed to create decoder for {:?}: {}", current_path, e);
                            return (frames_collected, true); // Return what we have and signal EOF
                        }
                    };
                    frame_counter.store(0, Ordering::Relaxed);
                    break; // restart loop with new decoder
                }
            }

            (frames_collected, eof)
        },
        &mut prod,
    );
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

fn get_playback_time(counter: &AtomicU64, sample_rate: u64) -> f64 {
    let frames = counter.load(Ordering::Relaxed);
    frames as f64 / sample_rate as f64
}

fn format_time(seconds: f64) -> String {
    let total_seconds = seconds.floor() as u64;
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let secs = total_seconds % 60;

    if hours > 0 {
        format!("{:02}:{:02}:{:02}", hours, minutes, secs)
    } else {
        format!("{:02}:{:02}", minutes, secs)
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    const FRAME_TIME: Duration = Duration::from_nanos(1_000_000_000 / 60); // 60 FPS

    let mut out = stdout();
    terminal::enable_raw_mode()?;
    execute!(out, EnterAlternateScreen, Hide)?;

    let (track_tx, track_rx) = std::sync::mpsc::channel::<PathBuf>();

    let volume = Arc::new(AtomicU32::new(0.05f32.to_bits()));
    let vol_state = Arc::clone(&volume);

    let is_paused = Arc::new(AtomicBool::new(false));
    let mut last_paused_state = false;

    let select_index = Arc::new(AtomicU32::new(0));
    let enter_pressed = Arc::new(AtomicBool::new(false));

    let host = cpal::default_host();
    let device = host.default_output_device().unwrap();
    let wanted_config = device
        .supported_output_configs()?
        .find(|c| c.channels() == 2 && c.sample_format() == cpal::SampleFormat::F32)
        .unwrap()
        .clone()
        .with_sample_rate(device.default_output_config()?.sample_rate());

    let device_rate = wanted_config.sample_rate().0 as usize;

    // TODO: change buffer size based on number of output channels
    let rb = HeapRb::<f32>::new(device_rate * 2);
    let (prod, mut cons): (HeapProd<f32>, HeapCons<f32>) = rb.split();

    let file_list = FileList::new(PathBuf::from("test-tracks/Woodland"), Arc::clone(&select_index));

    // Define view layout
    // TODO: Move to separate config file & add xml parsing
    let mut layout = Layout {
        sections: vec![
            Section::new(
                Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 0.3,
                    height: 0.6,
                },
                WidgetKind::FileList(file_list),
            ),
            Section::new(
                Rect {
                    x: 0.0,
                    y: 0.6,
                    width: 0.3,
                    height: 0.2,
                },
                WidgetKind::Label(Label {
                    text: "Test Circle Test Circle Test Circle".into(),
                    align_center: true,
                    rainbow: true,
                    tick: 0,
                }),
            ),
            Section::new(
                Rect {
                    x: 0.0,
                    y: 0.8,
                    width: 0.3,
                    height: 0.2,
                },
                WidgetKind::Controls(Controls::new(Arc::clone(&is_paused), Arc::clone(&volume))),
            ),
            Section::new(
                Rect {
                    x: 0.3,
                    y: 0.0,
                    width: 0.7,
                    height: 1.0,
                },
                WidgetKind::AlbumArt(AlbumArt::new(false)),
            ),
        ],
    };

    // Spawn keyhandler
    let mut key_handler = KeyHandler::new(
        Arc::clone(&is_paused),
        Arc::clone(&volume),
        Arc::clone(&select_index),
        Arc::clone(&enter_pressed),
    );
    key_handler.start();

    let frame_counter = Arc::new(AtomicU64::new(0));
    let counter_clone = frame_counter.clone();
    let counter_clone_2 = frame_counter.clone();

    // Spawn decoder
    // TODO: separate track header collection from decoding for up or downmixing
    //       channels. (Stereo -> Mono, 5.1 -> Stereo, etc)
    thread::spawn(move || {
        if let Ok(path) = track_rx.recv() {
            audio_thread(path, prod, device_rate, track_rx, frame_counter);
        }
    });

    // CPAL output
    let mut last_sample = 0.0f32;
    let channels = wanted_config.channels() as u64;
    
    let stream = device.build_output_stream(
        &wanted_config.into(),
        move |output: &mut [f32], _: &cpal::OutputCallbackInfo| {
            let mut consumed_any = false;

            for sample in output.iter_mut() {
                if let Some(s) = cons.try_pop() {
                    last_sample = s;
                    consumed_any = true;
                }
                let vol = f32::from_bits(vol_state.load(Ordering::Relaxed));
                *sample = (last_sample * vol).clamp(-1.0, 1.0);
            }

            if consumed_any {
                let frames_written = (output.len() as u64) / channels;
                counter_clone.fetch_add(frames_written, Ordering::Relaxed);
            }
        },
        |err| eprintln!("Stream error: {:?}", err),
        None,
    )?;
    stream.play()?;

    // Spin sleeper sleeps for the bulk of the allocated wait time,
    // then spins (busy waits) for the last few microseconds to get 
    // better accuracy
    let sleeper = SpinSleeper::default();
    let mut next_frame = Instant::now();
    let mut last_resync = Instant::now();

    // Main view loop
    loop {
        if key_handler.should_quit() {
            break;
        }

        let pause = is_paused.load(Ordering::Relaxed);
        if pause != last_paused_state {
            if pause {
                stream.pause()?;
            } else {
                stream.play()?;
            }
            last_paused_state = pause;
        }

        let new_track = enter_pressed.load(Ordering::Relaxed);
        if new_track {
            let index = select_index.load(Ordering::Relaxed);
            if let Some(file_list) = layout.sections.iter().find_map(|section| section.widget.as_file_list()) {
                if let Some(path_str) = file_list.get_file_from_index(index) {
                    let mut path = file_list.get_path();
                    path.push(path_str);
                    let _ = track_tx.send(path);
                    enter_pressed.store(false, Ordering::Relaxed);
                }
            }
        }

        if let Some(label) = layout.sections.iter_mut().find_map(|section| section.widget.as_label()) {
            let secs = get_playback_time(&counter_clone_2, device_rate as u64);
            label.set_text(format_time(secs));
        }

        // I imagine we'll want to throttle updates depending on
        // the widget at some point so this is just for convenience

        // TODO: We should define a pubsub system for widgets, where
        // widgets are rendered only when a state they care about changes
        // (e.g. volume, track name, etc)
        // I think we may also want to consider moving updates into a
        // separate thread while still keeping it event driven and change
        // widget states to be shared between render and update threads.
        layout.update_all()?;
        layout.render_all(&mut out)?;
        
        // Frame pacing for target FPS
        next_frame += FRAME_TIME;
        let now = Instant::now();

        if next_frame > now {
            sleeper.sleep(next_frame - now);
        } else {
            // Running behind, catch up
            next_frame = now;
        }

        // Periodic resync every ~2 seconds to avoid long-term drift 
        // and overflow in next_frame
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
