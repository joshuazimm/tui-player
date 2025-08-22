// std includes
use std::fs::File;
use std::io::{stdout, Write, BufReader};
use std::path::Path;
use std::thread;
use std::time::{Duration};

// external includes
use crossterm::{
    cursor,
    event::{self, Event, KeyCode},
    style::{Color, Print, SetForegroundColor, ResetColor},
    terminal::{self, Clear, ClearType},
    execute,
};
use ringbuf::{HeapRb, traits::{Split, Producer, Consumer}};
use claxon::FlacReader;
use ogg::reading::PacketReader;
use minimp3::Decoder as Minimp3Decoder;
use opus::{Decoder as OpusDecoder, Channels};
use lewton::inside_ogg::OggStreamReader;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

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
    let (mut prod, mut cons) = rb.split();
    let volume = 0.1; // distortion at low volumes using u8 output

    // Test minimp3 (will play too fast due to sample rate mismatch)
    // let mp3_path = Path::new("test-tracks/Stare Into Death and Be Still MP3.mp3");
    // let mut decoder = Minimp3Decoder::new(File::open(mp3_path)?);
    
    // thread::spawn(move || {
    //     while let Ok(frame) = decoder.next_frame() {
    //         for &sample in &frame.data {
    //             let s_f32 = sample as f32 / 32768.0;
    //             loop {
    //                 if prod.try_push(s_f32).is_ok() { break; }
    //                 std::thread::sleep(Duration::from_micros(50));
    //             }
    //         }
    //     }
    // });

    // Test Claxton (will play too fast due to sample rate mismatch)
    let flac_path = Path::new("test-tracks/Stare Into Death and Be Still Flac.flac");
    thread::spawn(move || {
        let mut reader = FlacReader::open(flac_path).unwrap();
        let bits = reader.streaminfo().bits_per_sample;

        for sample_res in reader.samples() {
            let sample = match sample_res {
                Ok(s) => s,
                Err(_) => continue, // skip errors
            };

            let s_i32_full = sample << (32 - bits); // shift left to fill 32 bits
            let s_f32 = s_i32_full as f32 / i32::MAX as f32; // scale to [-1.0, 1.0]

            loop {
                if prod.try_push(s_f32).is_ok() { break; }
                std::thread::sleep(Duration::from_micros(50));
            }
        }
    });

    // Test Lewton (will play too fast due to sample rate mismatch)
    // let vorbis_path = Path::new("test-tracks/Stare Into Death and Be Still Vorbis.ogg");
    // thread::spawn(move || {
    //     let file = File::open(vorbis_path).unwrap();
    //     let mut ogg_reader = OggStreamReader::new(file).unwrap();

    //     while let Ok(Some(pcm)) = ogg_reader.read_dec_packet_itl() {
    //         for &sample in &pcm {
    //             let s_f32 = sample as f32 / 32768.0;
    //             loop {
    //                 if prod.try_push(s_f32).is_ok() { break; }
    //                 std::thread::sleep(Duration::from_micros(50));
    //             }
    //         }
    //     }
    // });

    // Test Opus
    // let opus_path = Path::new("test-tracks/Stare Into Death and Be Still Opus.ogg"); 
    // let file = BufReader::new(File::open(opus_path)?);

    // thread::spawn(move || {
    //     let mut decoder = OpusDecoder::new(48000, Channels::Stereo).unwrap();
    //     let mut packet_reader = PacketReader::new(file);

    //     for _ in 0..2 { packet_reader.read_packet().ok(); }

    //     while let Some(packet) = packet_reader.read_packet().ok().flatten() {
    //         let mut pcm_tmp = vec![0i16; 960*2];
    //         if let Ok(samples_decoded) = decoder.decode(&packet.data, &mut pcm_tmp, false) {
    //             for &s in &pcm_tmp[..samples_decoded*2] {
    //                 let s_f32 = s as f32 / 32768.0;
    //                 loop {
    //                     if prod.try_push(s_f32).is_ok() { break; }
    //                     std::thread::sleep(Duration::from_micros(50));
    //                 }
    //             }
    //         }
    //     }
    // });

    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .expect("No output device available");

    let mut supported_configs_range = device.supported_output_configs()?;
    let supported_configs: Vec<_> = supported_configs_range.collect();

    // Prefer f32, then u8 as last resort
    let wanted_config = supported_configs
        .iter()
        .find(|c| c.sample_format() == cpal::SampleFormat::F32)
        .unwrap_or(&supported_configs[0]);

    let config = wanted_config.clone().with_max_sample_rate();

    println!(
        "Output config: {:?}, channels: {}, sample rate: {}",
        config.sample_format(),
        config.channels(),
        config.sample_rate().0
    );

    // Define output stream based on sample format
    let mut last_sample = 0.0f32;

    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => device.build_output_stream(
            &config.into(),
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
        )?,
        cpal::SampleFormat::U8 => device.build_output_stream(
            &config.into(),
            move |output: &mut [u8], _: &cpal::OutputCallbackInfo| {
                for sample in output.iter_mut() {
                    if let Some(s) = cons.try_pop() {
                        last_sample = s;
                    }
                    *sample = ((last_sample * volume * 127.0) + 128.0)
                        .round()
                        .clamp(0.0, 255.0) as u8;
                }
            },
            |err| eprintln!("Stream error: {:?}", err),
            None,
        )?,
        _ => unimplemented!(),
    };

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
        let x = (cols as usize / 2).saturating_sub("Now Playing: Stare Into Death and Be Still".len() / 2);
        let y = rows as usize / 2;

        execute!(out, cursor::MoveTo(x as u16, y as u16))?;
        for (i, ch) in "Now Playing: Stare Into Death and Be Still".chars().enumerate() {
            let color = rainbow_color(i + tick);
            execute!(
                out,
                SetForegroundColor(color),
                Print(ch),
                ResetColor
            )?;
        }
        out.flush()?;

        tick = tick.wrapping_add(1);
        std::thread::sleep(Duration::from_millis(120));
    }

    terminal::disable_raw_mode()?;
    Ok(())
}