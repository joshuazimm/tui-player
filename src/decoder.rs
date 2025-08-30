use claxon::FlacReader;
use minimp3::Decoder as Minimp3Decoder;
use minimp3::{Error, Frame};
use std::collections::VecDeque;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
// use lewton::inside_ogg::OggStreamReader;
// TODO: use above include for Vorbis support
use crate::resampler::pcm_to_f32;

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

pub struct OggTrack {
    // TODO: implement OGG decoder
    sample_rate: u32,
    channels: u32,
    bits_per_sample: u32,
}

pub enum AudioDecoder {
    Flac(FlacTrack),
    Mp3(Mp3Track),
    Ogg(OggTrack),
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
            AudioDecoder::Ogg(_track) => {
                // TODO: implement OGG sample reading
                None
            }
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
            // TODO: implement MP3 creation
            let path_str = path.to_str().ok_or("Invalid path")?;
            Ok(AudioDecoder::Mp3(Mp3Track::new(path_str)?))
        }
        "ogg" => {
            // TODO: implement OGG creation
            Err("OGG support not yet implemented".into())
        }
        _ => Err("Unsupported audio format".into()),
    }
}
