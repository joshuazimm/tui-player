// std includes
use std::fs::File;
use std::io::{BufReader, Write, stdout};
use std::path::Path;
use std::thread;
use std::time::Duration;
use std::sync::{Arc, atomic::{AtomicU32, Ordering}};

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
    terminal::{self},
};
// TODO use lewton::inside_ogg::OggStreamReader;
// use above include for Vorbuis support
use minimp3::Decoder as Minimp3Decoder;
use ogg::reading::PacketReader;
use opus::{Channels, Decoder as OpusDecoder};
use ringbuf::{
    HeapCons, HeapProd, HeapRb,
    traits::{Consumer, Producer, Split},
};

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

fn process_audio_chunks<F>(
    mut resampler: SimpleResampler,
    channels: usize,
    chunk_size: usize,
    mut fill_buffers: F,
    prod: &mut HeapProd<f32>,
) where
    F: FnMut(&mut Vec<Vec<f32>>) -> (usize, bool), // returns (frames_collected, eof)
{
    let mut buffers: Vec<Vec<f32>> = vec![Vec::with_capacity(chunk_size); channels];
    let mut out_buf = resampler.output_buffer_allocate(true);

    loop {
        // Clear buffers for the new chunk
        for ch in 0..channels {
            buffers[ch].truncate(0);
        }

        // Fill buffers using the provided closure
        let (frames_collected, eof) = fill_buffers(&mut buffers);

        if frames_collected > 0 {
            // Process the chunk
            let input_refs: Vec<&[f32]> = buffers.iter().map(|b| b.as_slice()).collect();
            let (_in_used, out_written) = if frames_collected < chunk_size {
                resampler.process_partial_into_buffer(&input_refs, &mut out_buf, None).unwrap()
            } else {
                resampler.process_into_buffer(&input_refs, &mut out_buf, None).unwrap()
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

            let resampler = SimpleResampler::new(
                ResamplerKind::Fast,
                file_rate,
                device_rate,
                chunk_size,
                sub_chunks,
                channels,
            ).unwrap();

            let mut sample_iter = reader.samples();

            process_audio_chunks(
                resampler,
                channels,
                chunk_size,
                |buffers| {
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
                    
                    (frames_collected, eof)
                },
                &mut prod,
            );
        }

        "mp3" => {
            let mut decoder = Minimp3Decoder::new(File::open(path).unwrap());
            let chunk_size = 1152;
            let first_frame = decoder.next_frame().unwrap();
            let file_rate = first_frame.sample_rate as usize;
            let channels = first_frame.channels;
            let sub_chunks = 1;
            
            let resampler = SimpleResampler::new(
                ResamplerKind::Sinc,
                file_rate,
                device_rate,
                chunk_size,
                sub_chunks,
                channels,
            ).unwrap();
            
            let mut frame_iter = std::iter::once(Ok(first_frame)).chain(std::iter::from_fn(|| {
                Some(decoder.next_frame())
            }));
            
            process_audio_chunks(
                resampler,
                channels,
                chunk_size,
                |buffers| {
                    let mut frames_collected = 0;
                    let mut eof = false;

                    while frames_collected < chunk_size {
                        if let Some(Ok(frame)) = frame_iter.next() {
                            for (i, sample) in frame.data.iter().enumerate() {
                                buffers[i % channels].push(*sample as f32 / 32768.0);
                            }
                            frames_collected += frame.data.len() / channels;
                        } else {
                            eof = true;
                            break;
                        }
                    }

                    (frames_collected, eof)
                },
                &mut prod,
            );
        }

        "ogg" => {
            let file_rate = 48000; // Opus always decodes to 48kHz
            let channels = 2; // TODO: Get actual channel count from Opus Header
            let chunk_size = 960;
            let sub_chunks = 1;
            
            let resampler = SimpleResampler::new(
                ResamplerKind::Fft,
                file_rate,
                device_rate,
                chunk_size,
                sub_chunks,
                channels,
            ).unwrap();
            
            // Open file handle for packet reading
            let file_for_packets = BufReader::new(File::open(path).unwrap());
            let mut decoder = OpusDecoder::new(file_rate as u32, 
                if channels == 1 { Channels::Mono } else { Channels::Stereo }).unwrap();
            let mut packet_reader = PacketReader::new(file_for_packets);
            
            process_audio_chunks(
                resampler,
                channels,
                chunk_size,
                |buffers| {
                    let mut frames_collected = 0;
                    let mut eof = false;

                    while frames_collected < chunk_size {
                        if let Some(packet) = packet_reader.read_packet().ok().flatten() {
                            let mut pcm_tmp = vec![0i16; chunk_size * channels];
                            if let Ok(samples_decoded) = decoder.decode(&packet.data, &mut pcm_tmp, false) {
                                for frame in pcm_tmp[..samples_decoded * channels].chunks(channels) {
                                    for ch in 0..channels {
                                        buffers[ch].push(frame[ch] as f32 / 32768.0);
                                    }
                                    frames_collected += 1;
                                    if frames_collected >= chunk_size {
                                        break;
                                    }
                                }
                            }
                        } else {
                            eof = true;
                            break;
                        }
                    }

                    (frames_collected, eof)
                },
                &mut prod,
            );
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

    let volume = Arc::new(std::sync::atomic::AtomicU32::new(0.1f32.to_bits()));
    let vol_state = Arc::clone(&volume);

    let mut is_paused = false;
    let mut last_paused_state = false;

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
    let path = Path::new("test-tracks/Stare Into Death and Be Still Flac.flac");
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
                let vol = f32::from_bits(vol_state.load(Ordering::Relaxed));
                *sample = (last_sample * vol).clamp(-1.0, 1.0);
            }
        },
        |err| eprintln!("Stream error: {:?}", err),
        None,
    )?;
    stream.play()?;

    loop {
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key_event) = event::read()? {
                if key_event.kind == event::KeyEventKind::Press {
                    match key_event.code {
                        KeyCode::Char('q') => break,
                        KeyCode::Left | KeyCode::Char('p') => {
                            // Previous track - TODO: implement playlist navigation
                        }
                        KeyCode::Right | KeyCode::Char('n') => {
                            // Next track - TODO: implement playlist navigation  
                        }
                        KeyCode::Char(' ') => {
                            is_paused = !is_paused;
                        }
                        KeyCode::Char('+') | KeyCode::Char('=') => {
                            let current_vol = f32::from_bits(volume.load(Ordering::Relaxed));
                            let new_vol = (current_vol + 0.01).min(1.0);
                            volume.store(new_vol.to_bits(), Ordering::Relaxed);
                        }
                        KeyCode::Char('-') | KeyCode::Char('_') => {
                            let current_vol = f32::from_bits(volume.load(Ordering::Relaxed));
                            let new_vol = (current_vol - 0.01).max(0.0);
                            volume.store(new_vol.to_bits(), Ordering::Relaxed);
                        }
                        _ => {}
                    }
                }
                // TODO: handle mouse events for clicking on controls
            }
        }

        if is_paused != last_paused_state {
            if is_paused {
                stream.pause()?;
            } else {
                stream.play()?;
            }
            last_paused_state = is_paused;
        }

        // Clear the screen area we'll be drawing on
        let (cols, rows) = terminal::size()?;
        
        // Draw the current track title
        let track_text = "Now Playing: Stare Into Death and Be Still";
        let x = (cols as usize / 2).saturating_sub(track_text.len() / 2);
        let y = rows as usize / 2;
        
        execute!(out, cursor::MoveTo(x as u16, y as u16))?;
        for (i, ch) in track_text.chars().enumerate() {
            let color = rainbow_color(i + tick);
            execute!(out, SetForegroundColor(color), Print(ch), ResetColor)?;
        }

        // Draw playback controls
        let controls = format!("  ◀◀    {}    ▶▶  ", if is_paused { "▶" } else { "⏸" });
        let control_x = (cols as usize / 2).saturating_sub(controls.len() / 2);
        let control_y = y + 2;
        
        execute!(out, cursor::MoveTo(control_x as u16, control_y as u16))?;
        
        // Color the controls
        for (i, ch) in controls.chars().enumerate() {
            let color = match ch {
                '◀' => Color::Cyan,
                '⏸' => Color::Yellow,
                '▶' => {
                    if controls.chars().nth(i-1) == Some(' ') && controls.chars().nth(i+1) == Some(' ') {
                        Color::Green
                    } else {
                        Color::Cyan
                    }
                },
                _ => Color::White,
            };
            execute!(out, SetForegroundColor(color), Print(ch), ResetColor)?;
        }

        // Draw instructions
        let instructions = format!(
            "◀/P=Prev | SPACE=Pause/Play | ▶/N=Next | +/-=Vol({:.0}%) | Q=Quit",
            f32::from_bits(volume.load(Ordering::Relaxed)) * 100.0
        );
        let inst_x = (cols as usize / 2).saturating_sub(instructions.len() / 2);
        let inst_y = control_y + 2;
        
        execute!(out, cursor::MoveTo(inst_x as u16, inst_y as u16))?;
        execute!(out, SetForegroundColor(Color::DarkGrey), Print(instructions), ResetColor)?;

        out.flush()?;
        
        // Only increment tick if not paused
        if !is_paused {
            tick = tick.wrapping_add(1);
        }
        
        std::thread::sleep(Duration::from_millis(120));
    }

    terminal::disable_raw_mode()?;
    Ok(())
}

