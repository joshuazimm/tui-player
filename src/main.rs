// std includes
use std::fs::File;
use std::io::{BufReader, Write, stdout};
use std::path::Path;
use std::thread;
use std::time::Duration;

//internal includes
mod helpers;
use helpers::{pcm_to_f32, SimpleResampler, ResamplerKind};

// external includes
// TODO: Find Filesystem crate for cross platform path handling
use claxon::FlacReader;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossterm::{
    cursor,
    event::{self, Event, KeyCode},
    execute,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{self, Clear, ClearType},
};
use lewton::inside_ogg::OggStreamReader;
use minimp3::Decoder as Minimp3Decoder;
use ogg::reading::PacketReader;
use opus::{Channels, Decoder as OpusDecoder};
use ringbuf::{
    HeapCons, HeapProd, HeapRb,
    traits::{Consumer, Producer, Split},
};
use rubato::{FftFixedIn, Resampler};

fn rainbow_color(i: usize) -> Color {
    match i % 6 {
        0 => Color::Red,
        1 => Color::Yellow,
        2 => Color::Green,
        3 => Color::Cyan,
        4 => Color::Blue,
        _ => Color::Magenta,
    }
}

fn play_audio_file(path: &Path, mut prod: ringbuf::HeapProd<f32>, device_rate: usize) {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "flac" => {
            let mut reader = FlacReader::open(path).unwrap();
            let file_rate = reader.streaminfo().sample_rate as usize;
            let bits = reader.streaminfo().bits_per_sample;
            let channels = reader.streaminfo().channels as usize;
            let chunk_size = 4096;
            let sub_chunks = 1;

            // create resampler
            let mut resampler = SimpleResampler::new(
                ResamplerKind::Fft,
                file_rate,
                device_rate,
                chunk_size,
                sub_chunks,
                channels,
            ).unwrap();

            // streaming buffers: one Vec<f32> per channel
            let mut buffers: Vec<Vec<f32>> = vec![Vec::with_capacity(chunk_size); channels];

            // allocate resampler output buffer once, reuse it
            let mut out_buf = resampler.output_buffer_allocate(true);
            let mut sample_iter = reader.samples();

            loop {
                // Clear buffers for the new chunk
                for ch in 0..channels {
                    buffers[ch].truncate(0);
                }
                
                // Collect exactly chunk_size frames
                let mut frames_collected = 0;
                let mut eof = false;
                
                while frames_collected < chunk_size {
                    for ch in 0..channels {
                        match sample_iter.next() {
                            Some(Ok(sample)) => {
                                buffers[ch].push(pcm_to_f32(sample, bits, true));
                            }
                            Some(Err(_)) => continue,
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
                
                // Process the chunk
                let input_refs: Vec<&[f32]> = buffers.iter().map(|b| b.as_slice()).collect();
                let (_in_used, out_written) = if frames_collected < chunk_size {
                    resampler.process_partial_into_buffer(&input_refs, &mut out_buf, None).unwrap()
                } else {
                    resampler.process_into_buffer(&input_refs, &mut out_buf, None).unwrap()
                };
                interleave_and_push(&out_buf, out_written, &mut prod);
                
                if eof {
                    break;
                }
            }
    }

        _ => panic!("Unsupported file type"),
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

// --- Usage ---
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut tick = 0usize;
    let mut out = stdout();
    terminal::enable_raw_mode()?;

    execute!(
        out,
        terminal::Clear(terminal::ClearType::All),
        cursor::MoveTo(0, 0),
        Print("Audio Library Test - Press 'q' to quit\n\n")
    )?;

    let volume = 0.1;

    let host = cpal::default_host();
    let device = host.default_output_device().unwrap();
    let wanted_config = device
        .supported_output_configs()?
        .find(|c| c.sample_format() == cpal::SampleFormat::F32)
        .unwrap()
        .clone()
        .with_max_sample_rate();

    let device_rate = wanted_config.sample_rate().0 as usize;
    println!("Output sample rate: {}", device_rate);

    // TODO: change buffer size based on number of output channels
    let rb = HeapRb::<f32>::new(device_rate * 2);
    let (prod, mut cons): (HeapProd<f32>, HeapCons<f32>) = rb.split();

    // spawn decoder
    // TODO: separate track header collection from decoding for up or downmixing
    //       channels. (Stereo -> Mono, 5.1 -> Stereo, etc)
    let path = Path::new("test-tracks/Stare Into Death and Be Still FLAC.flac");
    thread::spawn(move || play_audio_file(path, prod, device_rate));

    // CPAL output
    let mut last_sample = 0.0f32;
    let stream = device.build_output_stream(
        &wanted_config.into(),
        move |output: &mut [f32], _: &cpal::OutputCallbackInfo| {
            for sample in output.iter_mut() {
                if let Some(s) = cons.try_pop() {
                    last_sample = s;
                }
                *sample = (last_sample * volume).clamp(-1.0, 1.0);
            }
        },
        |err| eprintln!("Stream error: {:?}", err),
        None,
    )?;
    stream.play()?;

    loop {
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key_event) = event::read()? {
                if key_event.code == KeyCode::Char('q') {
                    break;
                }
            }
        }

        // redraw rainbow text every tick
        let (cols, rows) = terminal::size()?;
        let x = (cols as usize / 2)
            .saturating_sub("Now Playing: Stare Into Death and Be Still".len() / 2);
        let y = rows as usize / 2;

        execute!(out, cursor::MoveTo(x as u16, y as u16))?;
        for (i, ch) in "Now Playing: Stare Into Death and Be Still"
            .chars()
            .enumerate()
        {
            let color = rainbow_color(i + tick);
            execute!(out, SetForegroundColor(color), Print(ch), ResetColor)?;
        }
        
        // TODO: Basic playback controls (Pause, Stop, Next, Previous)
        //       - Next and Previous would require some kind of hardcoded playlist for now
        // TODO: Basic Track status (Progress bar, time elapsed, etc)

        out.flush()?;

        tick = tick.wrapping_add(1);
        std::thread::sleep(Duration::from_millis(120));
    }

    terminal::disable_raw_mode()?;
    Ok(())
}

