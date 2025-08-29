// std includes
use std::io::{BufReader, stdout};
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU32, Ordering},
    mpsc::{Receiver, TryRecvError},
};
use std::thread;
use std::time::Duration;

// internal includes
mod resampler;
use resampler::{ResamplerKind, SimpleResampler, pcm_to_f32};
mod widget;
use widget::{Layout, Rect, Section};
mod widgets;
use widgets::{Controls, FileList, Label, WidgetKind};
mod keyhandler;
use keyhandler::KeyHandler;
mod decoder;
use decoder::{AudioDecoder, decoder_from_path};

// external includes
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossterm::{
    cursor::{Hide, Show},
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
// use lewton::inside_ogg::OggStreamReader;
// TODO: use above include for Vorbis support
use ringbuf::{
    HeapCons, HeapProd, HeapRb,
    traits::{Consumer, Producer, Split},
};

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
) {
    let chunk_size = 4096;
    let sub_chunks = 1;

    let mut current_path = initial_path;

    let mut decoder = match decoder_from_path(&current_path) {
        Ok(decoder) => decoder,
        Err(e) => {
            eprintln!("Failed to create decoder for {:?}: {}", current_path, e);
            return; // or handle error appropriately
        }
    };
    let channels = decoder.channels();
    let file_rate = decoder.sample_rate();
    //let mut iter = decoder.get_reader().samples();

    let mut resampler = SimpleResampler::new(
        ResamplerKind::Fast,
        file_rate,
        device_rate,
        chunk_size,
        sub_chunks,
        channels,
    )
    .unwrap();

    let mut buffers: Vec<Vec<f32>> = vec![Vec::with_capacity(chunk_size); channels];
    let mut out_buf = resampler.output_buffer_allocate(true);

    loop {
        // Clear buffers for new chunk
        for ch in 0..channels {
            buffers[ch].truncate(0);
        }

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
                        return; // or handle error appropriately
                    }
                };
                break; // restart loop with new decoder
            }
        }

        // Resample and push
        if frames_collected > 0 {
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
            let output_vecs: Vec<Vec<f32>> = (0..channels)
                .map(|ch| out_buf[ch][..out_written].to_vec())
                .collect();
            interleave_and_push(&output_vecs, out_written, &mut prod);
        }

        if eof {
            break; // end of file
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

    let file_list = FileList::new(PathBuf::from("test-tracks"), Arc::clone(&select_index));

    // Define view layout
    // TODO: Move to separate config file & add xml parsing
    let mut layout = Layout {
        sections: vec![
            Section::new(
                Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 0.5,
                    height: 0.2,
                },
                true,
                WidgetKind::Label(Label {
                    text: "Now Playing: Stare Into Death and Be Still".into(),
                    align_center: true,
                    rainbow: true,
                    tick: 0,
                }),
            ),
            /*
            Section::new(
                Rect {
                    x: 0.5,
                    y: 0.0,
                    width: 0.5,
                    height: 0.2,
                },
                true,
                WidgetKind::Controls(Controls::new(Arc::clone(&is_paused), Arc::clone(&volume))),
            ),*/
            Section::new(
                Rect {
                    x: 0.5,
                    y: 0.0,
                    width: 0.5,
                    height: 0.2,
                },
                true,
                WidgetKind::FileList(file_list),
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

    // Spawn decoder
    // TODO: separate track header collection from decoding for up or downmixing
    //       channels. (Stereo -> Mono, 5.1 -> Stereo, etc)
    thread::spawn(move || {
        if let Ok(path) = track_rx.recv() {
            audio_thread(path, prod, device_rate, track_rx);
        }
    });

    // CPAL output
    let mut last_sample = 0.0f32;
    let stream = device.build_output_stream(
        &wanted_config.into(),
        move |output: &mut [f32], _: &cpal::OutputCallbackInfo| {
            for sample in output.iter_mut() {
                if let Some(s) = cons.try_pop() {
                    last_sample = s;
                }
                let vol = f32::from_bits(vol_state.load(Ordering::Relaxed));
                *sample = (last_sample * vol).clamp(-1.0, 1.0);
            }
        },
        |err| eprintln!("Stream error: {:?}", err),
        None,
    )?;
    stream.play()?;

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
            if let Some(file_list) = layout.sections[1].widget.as_file_list() {
                if let Some(path_str) = file_list.get_file_from_index(index) {
                    let mut path = file_list.get_path();
                    path.push(path_str);
                    let _ = track_tx.send(path);
                    enter_pressed.store(false, Ordering::Relaxed);
                }
            }
        }

        // I imagine we'll want to throttle updates depending on
        // the widget at some point so this is just for convenience

        // TODO: We should define a periodic rate for updates on each
        // Widget and deal with them in a separate thread
        layout.update_all()?;

        layout.render_all(&mut out)?;

        std::thread::sleep(Duration::from_millis(120));
    }

    key_handler.stop();
    execute!(out, LeaveAlternateScreen, Show)?;
    terminal::disable_raw_mode()?;
    Ok(())
}
