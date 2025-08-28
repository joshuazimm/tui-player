use anyhow::Error;
use rubato::{
    FastFixedIn, FftFixedIn, PolynomialDegree, Resampler, SincFixedIn, SincInterpolationParameters,
    SincInterpolationType, WindowFunction,
};

pub fn pcm_to_f32(sample: i32, bits: u32, signed: bool) -> f32 {
    if signed {
        let max_val = (1i64 << (bits - 1)) as f32;
        sample as f32 / max_val
    } else {
        let max_val = (1i64 << bits) as f32;
        (sample as f32 - max_val / 2.0) / (max_val / 2.0)
    }
}

fn calculate_optimal_sinc_len(output_rate: usize, channels: usize, base_len: usize) -> usize {
    const MAX_OPS_PER_SEC: f64 = 20_000_000.0;

    // Estimate operations per second using sinc filtering
    let ops_per_sec = output_rate as f64 * channels as f64 * base_len as f64;

    if ops_per_sec <= MAX_OPS_PER_SEC {
        return base_len;
    }

    let scale_factor = MAX_OPS_PER_SEC / ops_per_sec;
    let adjusted_len = (base_len as f64 * scale_factor) as usize;

    // Round to nearest multiple of 8 with a minimum of 32
    // The Rubato library does this internally anyways
    let min_len = adjusted_len.max(32);
    (((min_len + 4) / 8) * 8) as usize
}

// This Enum allows common handling of different resampler types
#[allow(dead_code)]
pub enum ResamplerKind {
    Fast, // Polynomial interpolation, fastest but lower quality
    Fft,  // Fast Fourier Transform based, good quality and speed balance
    Sinc, // Convolution with Sinc function, highest quality but slower
}

pub enum SimpleResampler {
    Fast(FastFixedIn<f32>),
    Fft(FftFixedIn<f32>),
    Sinc(SincFixedIn<f32>),
}

impl SimpleResampler {
    pub fn new(
        kind: ResamplerKind,
        input_rate: usize,
        output_rate: usize,
        chunk_size: usize,
        sub_chunks: usize,
        channels: usize,
    ) -> Result<Self, Error> {
        match kind {
            ResamplerKind::Fast => {
                Ok(SimpleResampler::Fast(FastFixedIn::<f32>::new(
                    output_rate as f64 / input_rate as f64,
                    10.0,
                    PolynomialDegree::Septic, // 7th order polynomial
                    chunk_size,
                    channels,
                )?))
            }
            ResamplerKind::Fft => Ok(SimpleResampler::Fft(FftFixedIn::<f32>::new(
                input_rate,
                output_rate,
                chunk_size,
                sub_chunks,
                channels,
            )?)),
            ResamplerKind::Sinc => {
                let base_sinc_len = 256;
                let adaptive_sinc_len =
                    calculate_optimal_sinc_len(output_rate, channels, base_sinc_len);
                let params = SincInterpolationParameters {
                    sinc_len: adaptive_sinc_len,
                    f_cutoff: 0.90,
                    interpolation: SincInterpolationType::Linear,
                    oversampling_factor: (adaptive_sinc_len / 2).max(32),
                    window: WindowFunction::BlackmanHarris2,
                };
                Ok(SimpleResampler::Sinc(SincFixedIn::<f32>::new(
                    output_rate as f64 / input_rate as f64,
                    10.0,
                    params,
                    chunk_size,
                    channels,
                )?))
            }
        }
    }

    /// Allocate an output buffer for this resampler
    pub fn output_buffer_allocate(&self, filled: bool) -> Vec<Vec<f32>> {
        match self {
            SimpleResampler::Fast(r) => r.output_buffer_allocate(filled),
            SimpleResampler::Fft(r) => r.output_buffer_allocate(filled),
            SimpleResampler::Sinc(r) => r.output_buffer_allocate(filled),
        }
    }

    /// Process a full chunk
    pub fn process_into_buffer(
        &mut self,
        input: &[&[f32]],
        output: &mut [Vec<f32>],
        end: Option<&[bool]>,
    ) -> Result<(usize, usize), Error> {
        match self {
            SimpleResampler::Fast(r) => Ok(r.process_into_buffer(input, output, end)?),
            SimpleResampler::Fft(r) => Ok(r.process_into_buffer(input, output, end)?),
            SimpleResampler::Sinc(r) => Ok(r.process_into_buffer(input, output, end)?),
        }
    }

    /// Process a partial chunk (only for Sinc and Fast resamplers)
    pub fn process_partial_into_buffer(
        &mut self,
        input: &[&[f32]],
        output: &mut [Vec<f32>],
        end: Option<&[bool]>,
    ) -> Result<(usize, usize), Error> {
        match self {
            SimpleResampler::Fast(r) => {
                let input_opt = if input.is_empty() { None } else { Some(input) };
                Ok(r.process_partial_into_buffer(input_opt, output, end)?)
            }
            SimpleResampler::Fft(r) => {
                // FFT resampler doesn't have a separate partial method
                Ok(r.process_into_buffer(input, output, end)?)
            }
            SimpleResampler::Sinc(r) => {
                let input_opt = if input.is_empty() { None } else { Some(input) };
                Ok(r.process_partial_into_buffer(input_opt, output, end)?)
            }
        }
    }
}

