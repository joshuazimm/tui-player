use crate::resampler::pcm_to_f32;
use claxon::FlacReader;
use hound::WavReader;
use lewton::VorbisError;
use lewton::inside_ogg::OggStreamReader;
use minimp3::Decoder as Minimp3Decoder;
use minimp3::{Error, Frame};
use mp3_duration::from_path;
use ogg::reading::PacketReader;
use opus::{Channels, Decoder as OpusDecoder};
use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufReader, Seek, SeekFrom, Read};
use std::path::Path;

pub struct FlacTrack {
    reader: FlacReader<BufReader<File>>,
    buffer: VecDeque<f32>,
    scratch: Vec<i32>, // reuse decoded frame buffer
    finished: bool,
    sample_rate: u32,
    channels: u32,
    bits_per_sample: u32,
    total_samples: u64,
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
            total_samples: si.samples.unwrap_or(0),
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
    reader: Minimp3Decoder<File>,
    current_frame: Option<Frame>,
    sample_index: usize,
    sample_rate: u32,
    channels: u32,
    total_samples: u64,
}

impl Mp3Track {
    pub fn new(path: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {

        let mut reader = Minimp3Decoder::new(File::open(path).unwrap());
        let _chunk_size = 1152;
        let current_frame = reader.next_frame().unwrap();
        let sample_rate = current_frame.sample_rate as u32;
        let channels = current_frame.channels as u32;
        let _bits_per_sample = current_frame.bitrate as u32;
        let duration = from_path(path)?;
        let samples = duration.as_nanos() * sample_rate as u128 / 1_000_000_000u128;

        Ok(Mp3Track {
            reader,
            current_frame: Some(current_frame),
            sample_index: 0,
            sample_rate,
            channels,
            total_samples: samples as u64,
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

fn get_total_samples(path: &str) -> Result<u64, Box<dyn std::error::Error + Send + Sync>> {
    let mut file = File::open(path)?;
    let file_size = file.metadata()?.len();
    
    // Search the last 64KB of the file for the final granule position
    let search_size = std::cmp::min(file_size, 65536);
    file.seek(SeekFrom::End(-(search_size as i64)))?;
    
    let mut buffer = vec![0u8; search_size as usize];
    file.read_exact(&mut buffer)?;
    
    let mut total_samples = 0u64;
    let mut pos = 0;
    
    // Look for "OggS" page signatures
    while pos + 14 <= buffer.len() {
        if &buffer[pos..pos + 4] == b"OggS" {
            // Extract granule position (8 bytes at offset 6)
            let granule_bytes = &buffer[pos + 6..pos + 14];
            let granule_pos = u64::from_le_bytes([
                granule_bytes[0], granule_bytes[1], granule_bytes[2], granule_bytes[3],
                granule_bytes[4], granule_bytes[5], granule_bytes[6], granule_bytes[7],
            ]);
            
            // Valid granule position (not -1/0xFFFFFFFFFFFFFFFF)
            if granule_pos != u64::MAX {
                total_samples = granule_pos;
            }
        }
        pos += 1;
    }
    
    Ok(total_samples)
}

pub struct VorbisTrack {
    reader: OggStreamReader<BufReader<File>>,
    sample_rate: u32,
    channels: u32,
    buffer: VecDeque<f32>,
    finished: bool,
    total_samples: u64,
}

impl VorbisTrack {
    pub fn new(path: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let reader = OggStreamReader::new(BufReader::new(File::open(path)?))?;
        let header = &reader.ident_hdr;
        let sample_rate = header.audio_sample_rate;
        let channels = header.audio_channels as u32;
        let _bits_per_sample = 16; // Vorbis decoded samples are i16
        let samples = get_total_samples(path);

        Ok(VorbisTrack {
            reader,
            sample_rate,
            channels,
            buffer: VecDeque::new(),
            finished: false,
            total_samples: samples?,
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
    total_samples: u64,
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
        let _version = ident_packet.data[8];
        let channels = ident_packet.data[9];
        let pre_skip = u16::from_le_bytes([ident_packet.data[10], ident_packet.data[11]]);
        let _original_sample_rate = u32::from_le_bytes([
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
        
        let file_size = packet_reader.seek_bytes(SeekFrom::End(0))?;
        packet_reader.seek_bytes(SeekFrom::Start(file_size - 1024))?;

        let mut final_granule_pos = None;
        loop {
            match packet_reader.read_packet() {
                Ok(Some(packet)) => {
                    final_granule_pos = Some(packet.absgp_page()); // granule pos of this packet
                }
                Ok(None) => break,
                Err(e) => return Err(e.into()),
            }
        }
        let total_samples = final_granule_pos.unwrap_or(0);

        Ok(OpusTrack {
            packet_reader,
            decoder,
            buffer: VecDeque::new(),
            finished: false,
            sample_rate: 48000, // Opus internal rate
            channels: channels as u32,
            pre_skip,
            samples_decoded: 0,
            total_samples: total_samples,
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

pub struct WavTrack {
    samples: Vec<f32>,
    pos: usize,
    sample_rate: u32,
    channels: u32,
    total_samples: u64,
}

impl WavTrack {
    pub fn new(path: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let mut reader = WavReader::open(path)?;
        let spec = reader.spec();
        let channels = spec.channels as u32;

        let samples: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Float => {
                reader.samples::<f32>().filter_map(Result::ok).collect()
            }
            hound::SampleFormat::Int => {
                let bits = spec.bits_per_sample;
                let max_val = (1i64 << (bits - 1)) as f32;
                if bits <= 16 {
                    reader.samples::<i16>().filter_map(Result::ok)
                        .map(|s| s as f32 / 32768.0)
                        .collect()
                } else {
                    reader.samples::<i32>().filter_map(Result::ok)
                        .map(|s| s as f32 / max_val)
                        .collect()
                }
            }
        };

        let total_samples = samples.len() as u64 / channels.max(1) as u64;

        Ok(WavTrack {
            samples,
            pos: 0,
            sample_rate: spec.sample_rate,
            channels,
            total_samples,
        })
    }

    fn next_sample(&mut self) -> Option<f32> {
        if self.pos < self.samples.len() {
            let s = self.samples[self.pos];
            self.pos += 1;
            Some(s)
        } else {
            None
        }
    }
}

pub enum AudioDecoder {
    Flac(FlacTrack),
    Mp3(Mp3Track),
    Opus(OpusTrack),
    Vorbis(VorbisTrack),
    Wav(WavTrack),
}

impl AudioDecoder {
    pub fn sample_rate(&self) -> usize {
        match self {
            AudioDecoder::Flac(track) => track.sample_rate as usize,
            AudioDecoder::Mp3(track) => track.sample_rate as usize,
            AudioDecoder::Vorbis(track) => track.sample_rate as usize,
            AudioDecoder::Opus(track) => track.sample_rate as usize,
            AudioDecoder::Wav(track) => track.sample_rate as usize,
        }
    }

    pub fn total_samples(&self) -> usize {
        match self {
            AudioDecoder::Flac(track) => track.total_samples as usize,
            AudioDecoder::Mp3(track) => track.total_samples as usize,
            AudioDecoder::Vorbis(track) => track.total_samples as usize,
            AudioDecoder::Opus(track) => track.total_samples as usize,
            AudioDecoder::Wav(track) => track.total_samples as usize,
        }
    }

    pub fn channels(&self) -> usize {
        match self {
            AudioDecoder::Flac(track) => track.channels as usize,
            AudioDecoder::Mp3(track) => track.channels as usize,
            AudioDecoder::Vorbis(track) => track.channels as usize,
            AudioDecoder::Opus(track) => track.channels as usize,
            AudioDecoder::Wav(track) => track.channels as usize,
        }
    }

    pub fn next_sample(&mut self) -> Option<f32> {
        match self {
            AudioDecoder::Flac(track) => track.next_sample(),
            AudioDecoder::Mp3(track) => track.next_sample(),
            AudioDecoder::Vorbis(track) => track.next_sample(),
            AudioDecoder::Opus(track) => track.next_sample(),
            AudioDecoder::Wav(track) => track.next_sample(),
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
            //TODO decide if we want to include opus contained in ogg
            let path_str = path.to_str().ok_or("Invalid path")?;
            Ok(AudioDecoder::Vorbis(VorbisTrack::new(path_str)?))
        }
        "opus" => {
            let path_str = path.to_str().ok_or("Invalid path")?;
            Ok(AudioDecoder::Opus(OpusTrack::new(path_str)?))
        }
        "wav" | "wave" => {
            let path_str = path.to_str().ok_or("Invalid path")?;
            Ok(AudioDecoder::Wav(WavTrack::new(path_str)?))
        }
        _ => Err("Unsupported audio format".into()),
    }
}
