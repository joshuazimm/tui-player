use crate::resampler::pcm_to_f32;
use claxon::input::BufferedReader;
use claxon::{FlacReader, FlacSamples};
use minimp3::Decoder as Minimp3Decoder;
use ogg::reading::PacketReader;
use opus::{Channels, Decoder as OpusDecoder};
use std::fs::File;
use std::path::Path;

pub trait TrackDecoder {
    fn sample_rate(&self) -> usize;
    fn channels(&self) -> usize;
    fn get_reader(&mut self) -> &FlacReader<File>; // normalized f32
    fn bits(&self) -> u32;
}

pub struct FlacTrack {
    reader: FlacReader<File>,
    bits: u32,
    channels: usize,
    rate: u32,
    count: u32,
}

impl FlacTrack {
    fn new(path: &Path) -> Self {
        let mut reader = FlacReader::open(path).unwrap();
        let bits = reader.streaminfo().bits_per_sample;
        let channels = reader.streaminfo().channels as usize;
        let rate = reader.streaminfo().sample_rate;
        let count = 0;
        Self {
            reader,
            rate,
            bits,
            channels,
            count,
        }
    }
}

impl TrackDecoder for FlacTrack {
    fn sample_rate(&self) -> usize {
        self.rate as usize
    }
    fn channels(&self) -> usize {
        self.channels
    }
    fn get_reader(&mut self) -> &FlacReader<File> {
        &mut self.reader
    }
    fn bits(&self) -> u32 {
        self.bits
    }
}

/* Repeat similarly for OGG and MP3
struct OggTrack {/* implement OggReader */}
impl TrackDecoder for OggTrack {
    /* same next_sample interface */
}

struct Mp3Track {/* implement MP3Reader */}
impl TrackDecoder for Mp3Track {
    /* same next_sample interface */
}*/

pub fn decoder_from_path(path: &Path) -> Box<dyn TrackDecoder + Send> {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase()
        .as_str()
    {
        "flac" => Box::new(FlacTrack::new(path)),
        //"ogg" => Box::new(OggTrack::new(path)),
        //"mp3" => Box::new(Mp3Track::new(path)),
        _ => panic!("Unsupported audio format"),
    }
}
