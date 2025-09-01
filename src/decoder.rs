use crate::resampler::pcm_to_f32;
use claxon::FlacReader;
use lewton::VorbisError;
use lewton::inside_ogg::OggStreamReader;
use minimp3::Decoder as Minimp3Decoder;
use minimp3::{Error, Frame};
use ogg::{Packet, reading::PacketReader};
use opus::{Application, Channels, Decoder as OpusDecoder};
use std::collections::VecDeque;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

pub struct FlacTrack {
    reader: FlacReader<BufReader<File>>,
    buffer: VecDeque<f32>,
    scratch: Vec<i32>, // reuse decoded frame buffer
    finished: bool,
    sample_rate: u32,
    channels: u32,
    bits_per_sample: u32,
}

impl FlacTrack {
    pub fn new(path: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let file = std::fs::File::open(path)?;
        let reader = FlacReader::new(std::io::BufReader::new(file))?;
        let si = reader.streaminfo();
        Ok(FlacTrack {
            reader,
            buffer: VecDeque::new(),
            scratch: Vec::new(),
            finished: false,
            sample_rate: si.sample_rate,
            channels: si.channels,
            bits_per_sample: si.bits_per_sample,
        })
    }

    fn fill_buffer(&mut self) {
        if self.finished {
            return;
        }

        // Decode one or more **whole** frames; never drop mid-block.
        let mut fr = self.reader.blocks();
        match fr.read_next_or_eof(std::mem::take(&mut self.scratch)) {
            Ok(Some(block)) => {
                // Interleave correctly.
                let ch = block.channels() as usize;
                let n = block.duration() as usize; // samples per channel

                if ch == 2 {
                    for (l, r) in block.stereo_samples() {
                        self.buffer
                            .push_back(pcm_to_f32(l, self.bits_per_sample, true));
                        self.buffer
                            .push_back(pcm_to_f32(r, self.bits_per_sample, true));
                    }
                } else {
                    for i in 0..n {
                        for c in 0..ch {
                            let s = block.sample(c as u32, i as u32);
                            self.buffer
                                .push_back(pcm_to_f32(s, self.bits_per_sample, true));
                        }
                    }
                }

                // Reuse buffer next call.
                self.scratch = block.into_buffer();
            }
            Ok(None) => {
                self.finished = true;
            }
            Err(_e) => {
                self.finished = true;
            }
        }
    }

    fn next_sample(&mut self) -> Option<f32> {
        if let Some(s) = self.buffer.pop_front() {
            return Some(s);
        }
        if self.finished {
            return None;
        }
        self.fill_buffer();
        self.buffer.pop_front()
    }
}
// Placeholder structs for other formats
pub struct Mp3Track {
    // TODO: implement MP3 decoder
    reader: Minimp3Decoder<File>,
    current_frame: Option<Frame>,
    sample_index: usize,
    sample_rate: u32,
    channels: u32,
    bits_per_sample: u32,
    chunk_size: u32,
}

impl Mp3Track {
    pub fn new(path: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let mut reader = Minimp3Decoder::new(File::open(path).unwrap());
        let chunk_size = 1152;
        let current_frame = reader.next_frame().unwrap();
        let sample_rate = current_frame.sample_rate as u32;
        let channels = current_frame.channels as u32;
        let bits_per_sample = current_frame.bitrate as u32;
        Ok(Mp3Track {
            reader,
            current_frame: Some(current_frame),
            sample_index: 0,
            chunk_size,
            sample_rate,
            channels,
            bits_per_sample,
        })
    }

    fn next_sample(&mut self) -> Option<f32> {
        loop {
            // Check if we have a current frame
            if let Some(ref frame) = self.current_frame {
                // Check if we're still within the current frame
                if self.sample_index < frame.data.len() {
                    // Get the current sample and convert from i16 to f32
                    let sample = frame.data[self.sample_index] as f32 / 32768.0;

                    // Advance to next sample
                    self.sample_index += 1;

                    return Some(sample);
                }
            }

            // We've exhausted the current frame, try to get the next one
            match self.reader.next_frame() {
                Ok(frame) => {
                    self.current_frame = Some(frame);
                    self.sample_index = 0;
                    // Continue the loop to return the first sample of this frame
                }
                Err(Error::Eof) => {
                    return None; // End of file
                }
                Err(_) => {
                    // Skip corrupted frames and try the next one
                    continue;
                }
            }
        }
    }
}

pub struct VorbisTrack {
    reader: OggStreamReader<BufReader<File>>,
    sample_rate: u32,
    channels: u32,
    bits_per_sample: u32,
    buffer: VecDeque<f32>,
    finished: bool,
}

impl VorbisTrack {
    pub fn new(path: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let reader = OggStreamReader::new(BufReader::new(File::open(path)?))?;
        let header = &reader.ident_hdr;
        let sample_rate = header.audio_sample_rate;
        let channels = header.audio_channels as u32;
        let bits_per_sample = 16; // Vorbis decoded samples are i16

        Ok(VorbisTrack {
            reader,
            sample_rate,
            channels,
            bits_per_sample,
            buffer: VecDeque::new(),
            finished: false,
        })
    }

    pub fn next_sample(&mut self) -> Option<f32> {
        loop {
            // If we have samples in the buffer, return one
            if let Some(sample) = self.buffer.pop_front() {
                return Some(sample);
            }

            // If we're finished and buffer is empty, return None
            if self.finished {
                return None;
            }

            // Try to fill the buffer with the next packet
            self.fill_buffer();
        }
    }

    fn fill_buffer(&mut self) {
        if self.finished {
            return;
        }

        match self.reader.read_dec_packet_itl() {
            Ok(Some(packet_samples)) => {
                if !packet_samples.is_empty() {
                    for &sample in packet_samples.iter() {
                        let f32_sample = sample as f32 / 32768.0;
                        self.buffer.push_back(f32_sample);
                    }
                } else {
                    // Empty packet, but continue trying
                }
            }
            Ok(None) => {
                // End of stream
                self.finished = true;
            }
            Err(VorbisError::BadAudio(_)) | Err(VorbisError::OggError(_)) => {
                // Skip bad packets and continue
            }
            Err(_) => {
                // Other errors - mark as finished
                self.finished = true;
            }
        }
    }
}

pub struct OpusTrack {
    packet_reader: PacketReader<BufReader<File>>,
    decoder: OpusDecoder,
    buffer: VecDeque<f32>,
    finished: bool,
    sample_rate: u32,
    channels: u32,
    bits_per_sample: u32,
    // Opus-specific
    pre_skip: u16,
    samples_decoded: u64,
}

impl OpusTrack {
    pub fn new(path: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let file = File::open(path)?;
        let mut packet_reader = PacketReader::new(BufReader::new(file));

        // Read identification header
        let ident_packet = packet_reader
            .read_packet()?
            .ok_or("No identification packet")?;
        if !ident_packet.data.starts_with(b"OpusHead") {
            return Err("Not an Opus file".into());
        }

        // Parse Opus identification header
        let version = ident_packet.data[8];
        let channels = ident_packet.data[9];
        let pre_skip = u16::from_le_bytes([ident_packet.data[10], ident_packet.data[11]]);
        let original_sample_rate = u32::from_le_bytes([
            ident_packet.data[12],
            ident_packet.data[13],
            ident_packet.data[14],
            ident_packet.data[15],
        ]);

        // Skip comment packet (Opus tags)
        let _comment_packet = packet_reader.read_packet()?.ok_or("No comment packet")?;

        // Create Opus decoder
        let opus_channels = match channels {
            1 => Channels::Mono,
            2 => Channels::Stereo,
            _ => return Err(format!("Unsupported channel count: {}", channels).into()),
        };

        let decoder = OpusDecoder::new(48000, opus_channels)?; // Opus always decodes at 48kHz

        Ok(OpusTrack {
            packet_reader,
            decoder,
            buffer: VecDeque::new(),
            finished: false,
            sample_rate: 48000, // Opus internal rate
            channels: channels as u32,
            bits_per_sample: 16, // Opus typically outputs 16-bit equivalent
            pre_skip,
            samples_decoded: 0,
        })
    }

    pub fn next_sample(&mut self) -> Option<f32> {
        loop {
            if let Some(sample) = self.buffer.pop_front() {
                return Some(sample);
            }

            if self.finished {
                return None;
            }

            self.fill_buffer();
        }
    }

    fn fill_buffer(&mut self) {
        if self.finished {
            return;
        }

        match self.packet_reader.read_packet() {
            Ok(Some(packet)) => {
                // Decode Opus packet
                let mut output = vec![0i16; 5760 * self.channels as usize]; // Max Opus frame size

                match self.decoder.decode(&packet.data, &mut output, false) {
                    Ok(samples_per_channel) => {
                        let total_samples = samples_per_channel * self.channels as usize;

                        // Handle pre-skip (samples to skip at beginning)
                        let skip_samples = if self.samples_decoded < self.pre_skip as u64 {
                            let remaining_skip = self.pre_skip as u64 - self.samples_decoded;
                            std::cmp::min(remaining_skip as usize, samples_per_channel)
                                * self.channels as usize
                        } else {
                            0
                        };

                        // Convert i16 to f32 and add to buffer (same as Vorbis/MP3)
                        for &sample in output[skip_samples..total_samples].iter() {
                            let f32_sample = sample as f32 / 32768.0;
                            self.buffer.push_back(f32_sample);
                        }

                        self.samples_decoded += samples_per_channel as u64;
                    }
                    Err(_) => {
                        // Skip corrupted packets
                    }
                }
            }
            Ok(None) => {
                self.finished = true;
            }
            Err(_) => {
                self.finished = true;
            }
        }
    }
}

pub enum AudioDecoder {
    Flac(FlacTrack),
    Mp3(Mp3Track),
    Ogg(VorbisTrack),
}

impl AudioDecoder {
    pub fn sample_rate(&self) -> usize {
        match self {
            AudioDecoder::Flac(track) => track.sample_rate as usize,
            AudioDecoder::Mp3(track) => track.sample_rate as usize,
            AudioDecoder::Ogg(track) => track.sample_rate as usize,
        }
    }

    pub fn channels(&self) -> usize {
        match self {
            AudioDecoder::Flac(track) => track.channels as usize,
            AudioDecoder::Mp3(track) => track.channels as usize,
            AudioDecoder::Ogg(track) => track.channels as usize,
        }
    }

    pub fn bits(&self) -> u32 {
        match self {
            AudioDecoder::Flac(track) => track.bits_per_sample,
            AudioDecoder::Mp3(track) => track.bits_per_sample,
            AudioDecoder::Ogg(track) => track.bits_per_sample,
        }
    }

    pub fn next_sample(&mut self) -> Option<f32> {
        match self {
            AudioDecoder::Flac(track) => track.next_sample(),
            AudioDecoder::Mp3(track) => track.next_sample(),
            AudioDecoder::Ogg(track) => track.next_sample(),
        }
    }
}

pub fn decoder_from_path(
    path: &Path,
) -> Result<AudioDecoder, Box<dyn std::error::Error + Send + Sync>> {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase()
        .as_str()
    {
        "flac" => {
            let path_str = path.to_str().ok_or("Invalid path")?;
            Ok(AudioDecoder::Flac(FlacTrack::new(path_str)?))
        }
        "mp3" => {
            let path_str = path.to_str().ok_or("Invalid path")?;
            Ok(AudioDecoder::Mp3(Mp3Track::new(path_str)?))
        }
        "ogg" => {
            let path_str = path.to_str().ok_or("Invalid path")?;
            Ok(AudioDecoder::Ogg(VorbisTrack::new(path_str)?))
        }
        _ => Err("Unsupported audio format".into()),
    }
}
