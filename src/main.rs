// std includes
use std::fs::File;
use std::io::{BufReader, Write, stdout};
use std::path::Path;
use std::thread;
use std::time::Duration;

// external includes
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
            let file_rate = (reader.streaminfo().sample_rate * 2) as usize;
            let bits = reader.streaminfo().bits_per_sample;
            let sub_chunks = 1;
            let channels = reader.streaminfo().channels as usize;
            let chunk_size = 1024;

            let mut resampler =
                FftFixedIn::<f32>::new(file_rate, device_rate, chunk_size, sub_chunks, channels)
                    .unwrap();

            // buffers per channel
            let mut buffers: Vec<Vec<f32>> = vec![Vec::with_capacity(chunk_size); channels];

            for sample_res in reader.samples() {
                let sample = match sample_res {
                    Ok(s) => s,
                    Err(_) => continue,
                };

                // push sample to each channel in round-robin
                for ch in 0..channels {
                    if buffers[ch].len() < chunk_size {
                        buffers[ch].push((sample << (32 - bits)) as f32 / i32::MAX as f32);
                    }
                }

                // once first channel reaches chunk_size, resample
                if buffers[0].len() >= chunk_size {
                    // pad any channels that are short
                    for ch in 0..channels {
                        while buffers[ch].len() < chunk_size {
                            buffers[ch].push(0.0);
                        }
                    }

                    let output = resampler.process(&buffers, None).unwrap();
                    interleave_and_push(&output, &mut prod);

                    // clear buffers for next chunk
                    for ch in 0..channels {
                        buffers[ch].clear();
                    }
                }
            }

            // flush any remaining samples by padding to chunk_size
            if !buffers[0].is_empty() {
                for ch in 0..channels {
                    while buffers[ch].len() < chunk_size {
                        buffers[ch].push(0.0);
                    }
                }
                let output = resampler.process(&buffers, None).unwrap();
                interleave_and_push(&output, &mut prod);
            }
        }

        "mp3" => {
            let mut decoder = Minimp3Decoder::new(File::open(path).unwrap());
            let chunk_size = 1152; // typical MP3 frame size
            let mut first_frame = decoder.next_frame().unwrap();
            let file_rate = first_frame.sample_rate as usize;
            let sub_chunks = 1;
            let channels = first_frame.channels;

            let mut resampler =
                FftFixedIn::<f32>::new(file_rate, device_rate, chunk_size, sub_chunks, channels)
                    .unwrap();

            // buffers per channel
            let mut buffers: Vec<Vec<f32>> = vec![Vec::with_capacity(chunk_size); channels];

            // process first frame
            for (i, sample) in first_frame.data.iter().enumerate() {
                buffers[i % channels].push(*sample as f32 / 32768.0);
            }

            // resample first frame if enough samples
            if buffers[0].len() >= chunk_size {
                for ch in 0..channels {
                    while buffers[ch].len() < chunk_size {
                        buffers[ch].push(0.0);
                    }
                }
                let output = resampler.process(&buffers, None).unwrap();
                interleave_and_push(&output, &mut prod);
                buffers.iter_mut().for_each(|b| b.clear());
            }

            // now loop over remaining frames
            while let Ok(frame) = decoder.next_frame() {
                for (i, sample) in frame.data.iter().enumerate() {
                    buffers[i % channels].push(*sample as f32 / 32768.0);
                }

                if buffers[0].len() >= chunk_size {
                    for ch in 0..channels {
                        while buffers[ch].len() < chunk_size {
                            buffers[ch].push(0.0);
                        }
                    }
                    let output = resampler.process(&buffers, None).unwrap();
                    interleave_and_push(&output, &mut prod);
                    buffers.iter_mut().for_each(|b| b.clear());
                }
            }

            // flush any remaining samples
            if !buffers[0].is_empty() {
                for ch in 0..channels {
                    while buffers[ch].len() < chunk_size {
                        buffers[ch].push(0.0);
                    }
                }
                let output = resampler.process(&buffers, None).unwrap();
                interleave_and_push(&output, &mut prod);
            }
        }

        "ogg" => {
            let file = BufReader::new(File::open(path).unwrap());
            let ogg_reader = OggStreamReader::new(file).unwrap();
            let file_rate = ogg_reader.ident_hdr.audio_sample_rate as usize;
            let channels = ogg_reader.ident_hdr.audio_channels as usize;

            let file = BufReader::new(File::open(path).unwrap());
            let mut decoder = OpusDecoder::new(48000, Channels::Stereo).unwrap();
            let mut packet_reader = PacketReader::new(file);
            let sub_chunks = 1; // I have no idea what this should be
            let chunk_size = 960;

            let mut resampler =
                FftFixedIn::<f32>::new(file_rate, device_rate, chunk_size, sub_chunks, channels)
                    .unwrap();

            while let Some(packet) = packet_reader.read_packet().ok().flatten() {
                let mut pcm_tmp = vec![0i16; chunk_size * channels];
                if let Ok(samples_decoded) = decoder.decode(&packet.data, &mut pcm_tmp, false) {
                    let mut left = Vec::with_capacity(samples_decoded);
                    let mut right = Vec::with_capacity(samples_decoded);
                    for frame in pcm_tmp[..samples_decoded * channels].chunks(channels) {
                        left.push(frame[0] as f32 / 32768.0);
                        right.push(frame[1] as f32 / 32768.0);
                    }
                    while left.len() < chunk_size {
                        left.push(0.0);
                        right.push(0.0);
                    }
                    let output = resampler.process(&vec![left, right], None).unwrap();
                    interleave_and_push(&output, &mut prod);
                }
            }
        }

        _ => panic!("Unsupported file type"),
    }
}

fn interleave_and_push(buffers: &[Vec<f32>], prod: &mut HeapProd<f32>) {
    let channels = buffers.len();
    for i in 0..buffers[0].len() {
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

    let rb = HeapRb::<f32>::new(48000 * 2); // 1 second buffer at 48kHz stereo
    let (mut prod, mut cons): (HeapProd<f32>, HeapCons<f32>) = rb.split();
    let volume = 0.1; // distortion at low volumes using u8 output

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

    // spawn decoder
    let path = Path::new("test-tracks/Stare Into Death and Be Still MP3.mp3");
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
        out.flush()?;

        tick = tick.wrapping_add(1);
        std::thread::sleep(Duration::from_millis(120));
    }

    terminal::disable_raw_mode()?;
    Ok(())
}

