use claxon::FlacReader;
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
    sample_rate: u32,
    channels: u32,
    bits_per_sample: u32,
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
            AudioDecoder::Mp3(_track) => {
                // TODO: implement MP3 sample reading
                None
            }
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
            Err("MP3 support not yet implemented".into())
        }
        "ogg" => {
            // TODO: implement OGG creation
            Err("OGG support not yet implemented".into())
        }
        _ => Err("Unsupported audio format".into()),
    }
}
