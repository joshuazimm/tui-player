use crate::canvas::Canvas;
use crate::widget::Widget;
use crate::state::{GlobalState, StateKey, StateValue};
use crossterm::style::Color;

/// Eight levels of vertical block fill (▁ = 1/8 through █ = 8/8).
const BLOCK: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// Display floor in dBFS. Signals below this are invisible.
const FLOOR_DB: f32 = -60.0;

/// Key frequencies (Hz) to attempt to label on the x-axis.
/// Listed densest-to-sparsest so the collision check keeps the most
/// informative labels when the widget is narrow.
const LABEL_FREQS: &[f32] = &[
    20.0, 30.0, 50.0, 100.0, 200.0, 300.0, 500.0,
    1_000.0, 2_000.0, 3_000.0, 5_000.0, 10_000.0, 20_000.0,
];

/// Bar colour before any album art is loaded.
const DEFAULT_BAR_COLOR: Color = Color::Rgb { r: 100, g: 149, b: 237 }; // cornflower blue
/// Peak-hold colour before any album art is loaded.
const DEFAULT_PEAK_COLOR: Color = Color::Rgb { r: 255, g: 255, b: 255 }; // white

pub struct Spectrum {
    /// Per-band display values in [0, 1] (dB-scaled).
    bands: Vec<f32>,
    /// Peak-hold values in [0, 1]; decay slowly.
    peaks: Vec<f32>,
    /// Uniform bar colour, driven by the album art primary colour.
    bar_color: Color,
    /// Peak-hold marker colour, driven by the album art secondary colour.
    peak_color: Color,
    /// Center frequency of each band in Hz. Used for x-axis labels.
    band_centers: Vec<f32>,
    dirty: bool,
}

impl Spectrum {
    pub fn new(band_centers: Vec<f32>) -> Self {
        let n = band_centers.len();
        Self {
            bands: vec![0.0; n],
            peaks: vec![0.0; n],
            bar_color: DEFAULT_BAR_COLOR,
            peak_color: DEFAULT_PEAK_COLOR,
            band_centers,
            dirty: true,
        }
    }

    /// Convert raw RMS magnitude → [0, 1] display value via dB scaling.
    /// FLOOR_DB → 0.0, 0 dBFS → 1.0.
    fn to_display(raw: f32) -> f32 {
        let db = 20.0 * raw.max(1e-10_f32).log10();
        ((db - FLOOR_DB) / (-FLOOR_DB)).clamp(0.0, 1.0)
    }

    /// Draw frequency labels on the bottom row, below the bars.
    fn draw_labels(&self, canvas: &mut Canvas) {
        if self.band_centers.is_empty() || canvas.height() == 0 {
            return;
        }
        let w = canvas.width();
        let n = self.band_centers.len();
        let label_y = canvas.height() - 1;

        let mut last_end_x: usize = 0;
        for &freq in LABEL_FREQS {
            let Some(band_idx) = self
                .band_centers
                .iter()
                .enumerate()
                .min_by(|&(_, a), &(_, b)| {
                    (a - freq).abs().partial_cmp(&(b - freq).abs()).unwrap()
                })
                .map(|(i, _)| i)
            else {
                continue;
            };

            let col_start = band_idx * w / n;
            let col_width = (w / n).max(1);
            let band_mid_x = col_start + col_width / 2;

            let label = if freq >= 1_000.0 {
                format!("{}k", (freq / 1_000.0) as u32)
            } else {
                format!("{}", freq as u32)
            };

            let label_x = band_mid_x.saturating_sub(label.len() / 2);
            if label_x < last_end_x + 1 {
                continue;
            }

            canvas.put_str(label_x, label_y, &label, Some(Color::DarkGrey), None);
            last_end_x = label_x + label.len();
        }
    }
}

impl Widget for Spectrum {
    fn update(&mut self, _state: &GlobalState) {
        let mut changed = false;

        for peak in &mut self.peaks {
            if *peak > 0.001 {
                *peak *= 0.97;
                changed = true;
            }
        }

        if changed {
            self.dirty = true;
        }
    }

    fn render(&self, canvas: &mut Canvas) -> std::io::Result<()> {
        if self.bands.is_empty() {
            return Ok(());
        }
        let w = canvas.width();
        // The bottom row holds the frequency labels; bars use the rows above it
        let h = canvas.height().saturating_sub(1);
        if w == 0 || h == 0 {
            return Ok(());
        }

        let n = self.bands.len();
        let base_y = h;

        for band_idx in 0..n {
            let x_start = band_idx * w / n;
            let x_end = (band_idx + 1) * w / n;
            if x_start >= x_end {
                continue;
            }

            let amp = self.bands[band_idx].clamp(0.0, 1.0);
            let peak = if band_idx < self.peaks.len() {
                self.peaks[band_idx].clamp(0.0, 1.0)
            } else {
                0.0
            };

            let bar_eighths = (amp * h as f32 * 8.0) as usize;
            let full_cells = bar_eighths / 8;
            let remainder = bar_eighths % 8;
            let peak_row = (peak * h as f32) as usize;

            for x in x_start..x_end {
                // Solid cells upward from the base
                for row_up in 0..full_cells.min(h) {
                    let y = base_y.saturating_sub(1 + row_up);
                    canvas.set(x, y, '█', Some(self.bar_color), None);
                }

                // Partial block at the bar tip
                if remainder > 0 && full_cells < h {
                    let y = base_y.saturating_sub(1 + full_cells);
                    canvas.set(x, y, BLOCK[remainder - 1], Some(self.bar_color), None);
                }

                // Peak-hold marker above the current bar
                if peak_row > 0 && peak_row <= h {
                    let y = base_y.saturating_sub(peak_row);
                    let bar_top_y = base_y.saturating_sub(full_cells.max(1));
                    if y < bar_top_y {
                        canvas.set(x, y, '▄', Some(self.peak_color), None);
                    }
                }
            }
        }

        self.draw_labels(canvas);
        Ok(())
    }

    fn needs_render(&self) -> bool {
        self.dirty
    }

    fn mark_rendered(&mut self) {
        self.dirty = false;
    }

    fn handle_state_change(&mut self, key: &StateKey, value: &StateValue) {
        match key {
            StateKey::FrequencyBands => {
                if let Some(new_bands) = value.as_frequency_bands() {
                    if self.bands.len() != new_bands.len() {
                        self.bands.resize(new_bands.len(), 0.0);
                        self.peaks.resize(new_bands.len(), 0.0);
                    }
                    for (i, &raw) in new_bands.iter().enumerate() {
                        let scaled = Self::to_display(raw);
                        if scaled >= self.bands[i] {
                            self.bands[i] = scaled;
                        } else {
                            self.bands[i] = self.bands[i] * 0.65 + scaled * 0.35;
                        }
                        if self.bands[i] > self.peaks[i] {
                            self.peaks[i] = self.bands[i];
                        }
                    }
                    self.dirty = true;
                }
            }
            StateKey::AlbumArtColor => {
                if let Some((r, g, b)) = value.as_rgb_color() {
                    self.bar_color = Color::Rgb { r, g, b };
                    self.dirty = true;
                }
            }
            StateKey::AlbumArtColorSecondary => {
                if let Some((r, g, b)) = value.as_rgb_color() {
                    self.peak_color = Color::Rgb { r, g, b };
                    self.dirty = true;
                }
            }
            _ => {}
        }
    }

    fn subscribed_keys(&self) -> Vec<StateKey> {
        vec![
            StateKey::FrequencyBands,
            StateKey::AlbumArtColor,
            StateKey::AlbumArtColorSecondary,
        ]
    }
}
