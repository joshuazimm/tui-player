use std::thread;
use std::time::Duration;

use ringbuf::{HeapCons, traits::Consumer};
use rustfft::{Fft, FftPlanner, num_complex::Complex};

use crate::state::GlobalState;

// 4096-point FFT gives ~11 Hz/bin at 48 kHz, enough to resolve the lowest
// logarithmic bands individually.  With 2048 the first several bands all
// collapsed onto bin 1 and moved in lockstep.
pub const FFT_SIZE: usize = 4096;
const HOP_SIZE: usize = 2048;
pub const DEFAULT_NUM_BANDS: usize = 32;

/// Returns `num_bands + 1` logarithmically-spaced frequency edges in Hz,
/// spanning 20 Hz to min(sample_rate/2, 20 kHz).
fn log_band_edges(num_bands: usize, sample_rate: f32) -> Vec<f32> {
    let f_min = 20.0_f32;
    let f_max = (sample_rate / 2.0).min(20_000.0);
    let log_min = f_min.ln();
    let log_max = f_max.ln();
    (0..=num_bands)
        .map(|i| (log_min + (log_max - log_min) * i as f32 / num_bands as f32).exp())
        .collect()
}

/// Returns the geometric-mean center frequency of each band.
pub fn band_centers(num_bands: usize, sample_rate: f32) -> Vec<f32> {
    let edges = log_band_edges(num_bands, sample_rate);
    (0..num_bands)
        .map(|i| (edges[i] * edges[i + 1]).sqrt())
        .collect()
}

// ---------------------------------------------------------------------------
// SpectrumAnalyzer
// ---------------------------------------------------------------------------

struct SpectrumAnalyzer {
    fft_size: usize,
    window: Vec<f32>,
    band_bin_ranges: Vec<(usize, usize)>,
    fft: std::sync::Arc<dyn Fft<f32>>,
}

impl SpectrumAnalyzer {
    fn new(fft_size: usize, sample_rate: f32, num_bands: usize) -> Self {
        let window: Vec<f32> = (0..fft_size)
            .map(|i| {
                0.5 * (1.0
                    - (2.0 * std::f32::consts::PI * i as f32 / (fft_size - 1) as f32).cos())
            })
            .collect();

        let edges = log_band_edges(num_bands, sample_rate);
        let hz_per_bin = sample_rate / fft_size as f32;
        let nyquist_bins = fft_size / 2;

        let band_bin_ranges: Vec<(usize, usize)> = (0..num_bands)
            .map(|b| {
                let mut start = (edges[b] / hz_per_bin).floor() as usize;
                let mut end = (edges[b + 1] / hz_per_bin).floor() as usize;
                start = start.max(1).min(nyquist_bins - 1);
                end = end.max(start).min(nyquist_bins - 1);
                (start, end)
            })
            .collect();

        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(fft_size);

        Self { fft_size, window, band_bin_ranges, fft }
    }

    /// Returns RMS energy per logarithmic band.
    fn compute_bands(&self, samples: &[f32]) -> Vec<f32> {
        let mut buffer: Vec<Complex<f32>> = samples[..self.fft_size]
            .iter()
            .enumerate()
            .map(|(i, &s)| Complex::new(s * self.window[i], 0.0))
            .collect();

        self.fft.process(&mut buffer);

        let scale = 1.0 / self.fft_size as f32;
        let magnitudes: Vec<f32> = buffer[..self.fft_size / 2]
            .iter()
            .map(|c| c.norm() * scale)
            .collect();

        self.band_bin_ranges
            .iter()
            .map(|&(start, end)| {
                let count = (end - start + 1) as f32;
                let energy: f32 = magnitudes[start..=end].iter().map(|&m| m * m).sum();
                (energy / count).sqrt()
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Spawns a background thread that reads interleaved stereo f32 samples from
/// `cons`, runs FFT analysis, and pushes `FrequencyBands` into `state`.
pub fn spawn_analysis_thread(
    mut cons: HeapCons<f32>,
    channels: usize,
    device_rate: usize,
    num_bands: usize,
    state: GlobalState,
) {
    thread::spawn(move || {
        let analyzer = SpectrumAnalyzer::new(FFT_SIZE, device_rate as f32, num_bands);
        let mut mono_buf: Vec<f32> = Vec::with_capacity(FFT_SIZE * 4);

        loop {
            // ── drain ring buffer, downmix to mono ───────────────────────
            let mut had_samples = false;
            loop {
                let l = match cons.try_pop() {
                    Some(s) => s,
                    None => break,
                };
                let r = if channels > 1 { cons.try_pop().unwrap_or(l) } else { l };
                mono_buf.push((l + r) * 0.5);
                had_samples = true;
                if mono_buf.len() >= FFT_SIZE * 4 {
                    break;
                }
            }

            // ── process all complete windows ─────────────────────────────
            while mono_buf.len() >= FFT_SIZE {
                let bands = analyzer.compute_bands(&mono_buf);
                state.set_frequency_bands(bands);

                if mono_buf.len() > HOP_SIZE {
                    mono_buf.drain(..HOP_SIZE);
                } else {
                    mono_buf.clear();
                }
            }

            if !had_samples {
                thread::sleep(Duration::from_millis(5));
            }
        }
    });
}
