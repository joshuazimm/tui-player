use image::{ImageBuffer, Rgba};
use crate::widget::{AbsRect, Widget, Cell};
use crate::state::{GlobalState, StateKey, StateValue};
use crossterm::style::Color;
use fast_image_resize as fir;
use std::cell::RefCell;
use std::num::NonZero;
use std::fs::{self, OpenOptions};
use std::io::Write as IoWrite;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use zune_jpeg::JpegDecoder;

/// What the background thread delivers: decoded image + (primary, secondary) colour.
type LoadResult = (image::RgbaImage, ((u8, u8, u8), (u8, u8, u8)));

/// Resized pixel data cached for a specific terminal area.
struct CachedResize {
    /// The (x, y, w, h) area that produced this resize.
    area: (u16, u16, u16, u16),
    data: Vec<u8>,
    draw_w: u32,
    draw_h: u32,
    offset_x: u32,
    offset_y: u32,
}

pub struct AlbumArt {
    /// Full-resolution decoded image.
    img: Option<image::RgbaImage>,
    /// Path of the currently displayed (or being loaded) image, prevents
    /// redundant reloads when the same track is re-selected.
    current_art_path: Option<String>,
    /// In-flight background load channel.  `None` when idle.
    pending: Option<Receiver<LoadResult>>,
    stretch: bool,
    needs_render: AtomicBool,
    last_terminal_size: (u16, u16),
    /// Cache downscale result so `render()` never repeats the expensive
    /// resize unless the terminal is resized or a new image arrives.
    cached_resize: RefCell<Option<CachedResize>>,
}

impl AlbumArt {
    pub fn new(stretch: bool) -> Self {
        Self {
            img: None,
            current_art_path: None,
            pending: None,
            stretch,
            needs_render: AtomicBool::new(true),
            last_terminal_size: (0, 0),
            cached_resize: RefCell::new(None),
        }
    }

    /// Kick off a background load for `path`, replacing any previous pending load.
    fn start_load(&mut self, path: String) {
        if self.current_art_path.as_deref() == Some(&path) {
            // Same cover, no need to reload.
            return;
        }
        self.current_art_path = Some(path.clone());

        let (tx, rx) = mpsc::channel::<LoadResult>();
        thread::spawn(move || {
            match load_image_auto(Path::new(&path)) {
                Some(img) => {
                    let palette = extract_palette(&img);
                    let _ = tx.send((img, palette));
                }
                None => {
                    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open("output.txt") {
                        let _ = writeln!(f, "[album_art] load_image_auto returned None for: {}", path);
                    }
                }
            }
        });

        // Show placeholder while loading
        self.pending = Some(rx);
        self.img = None;
        *self.cached_resize.borrow_mut() = None;
        self.needs_render.store(true, Ordering::Relaxed);
    }

    fn render_pixels(cache: &CachedResize, area: &AbsRect, buf: &mut [Vec<Cell>]) {
        let stride = (cache.draw_w * 4) as usize;
        for by in 0..(cache.draw_h / 2) {
            for bx in 0..cache.draw_w {
                let top_off = (by * 2 * stride as u32 + bx * 4) as usize;
                let bot_off = ((by * 2 + 1) * stride as u32 + bx * 4) as usize;
                if bot_off + 4 > cache.data.len() {
                    continue;
                }
                let tp = &cache.data[top_off..top_off + 4];
                let bp = &cache.data[bot_off..bot_off + 4];
                let x = area.x as usize + cache.offset_x as usize + bx as usize;
                let y = area.y as usize + (cache.offset_y / 2) as usize + by as usize;
                if y < buf.len() && x < buf[y].len() {
                    let cell = &mut buf[y][x];
                    cell.ch = '▀';
                    cell.fg = Some(Color::Rgb { r: tp[0], g: tp[1], b: tp[2] });
                    cell.bg = Some(Color::Rgb { r: bp[0], g: bp[1], b: bp[2] });
                }
            }
        }
    }
}

impl Widget for AlbumArt {
    fn update(&mut self, state: &GlobalState) {
        // Poll the background channel without keeping a borrow of self.pending
        // open `as_ref().map(…)` ends the borrow before the match.
        let poll = self.pending.as_ref().map(|rx| rx.try_recv());
        match poll {
            Some(Ok((img, (primary, secondary)))) => {
                self.img = Some(img);
                *self.cached_resize.borrow_mut() = None;
                self.needs_render.store(true, Ordering::Relaxed);
                self.pending = None;
                state.set_album_art_colors(primary, secondary);
            }
            Some(Err(mpsc::TryRecvError::Disconnected)) => {
                // Thread exited without sending, file missing, or decode error.
                // Reset current_art_path so the same track can be retried.
                self.pending = None;
                self.current_art_path = None;
                self.needs_render.store(true, Ordering::Relaxed);
            }
            _ => {} // Empty: still loading
        }
    }

    fn render(&self, area: &AbsRect, buf: &mut [Vec<Cell>]) -> std::io::Result<()> {
        let img = match &self.img {
            Some(img) => img,
            None => {
                let placeholder = "♪ No Album Art ♪";
                let y = area.y as usize + area.height as usize / 2;
                let x_start = area.x as usize
                    + (area.width as usize).saturating_sub(placeholder.len()) / 2;
                for (i, ch) in placeholder.chars().enumerate() {
                    if y < buf.len() && x_start + i < buf[y].len() {
                        buf[y][x_start + i].ch = ch;
                        buf[y][x_start + i].fg = Some(Color::DarkGrey);
                    }
                }
                return Ok(());
            }
        };

        let area_key = (area.x, area.y, area.width, area.height);

        // Return cached resize if the area matches.
        {
            let cache = self.cached_resize.borrow();
            if let Some(c) = cache.as_ref() {
                if c.area == area_key {
                    Self::render_pixels(c, area, buf);
                    return Ok(());
                }
            }
        }

        // Compute a fresh resize for the new area dimensions.
        let orig_w = img.width();
        let orig_h = img.height();
        let target_w = (area.width + 1) as u32;
        let target_h = (area.height + 1) as u32 * 2;

        let img_aspect = orig_w as f32 / orig_h as f32;
        let target_aspect = target_w as f32 / target_h as f32;

        let (draw_w, draw_h, offset_x, offset_y) = if self.stretch {
            (target_w, target_h, 0u32, 0u32)
        } else if img_aspect > target_aspect {
            let dw = target_w;
            let dh = (target_w as f32 / img_aspect).round().min(target_h as f32) as u32;
            let oy = ((target_h as i32 - dh as i32) / 2).max(0) as u32;
            (dw, dh, 0, oy)
        } else {
            let dh = target_h;
            let dw = (target_h as f32 * img_aspect).round().min(target_w as f32) as u32;
            let ox = ((target_w as i32 - dw as i32) / 2).max(0) as u32;
            (dw, dh, ox, 0)
        };

        if draw_w == 0 || draw_h == 0 {
            return Ok(());
        }

        let src = fir::Image::from_vec_u8(
            NonZero::new(orig_w).unwrap(),
            NonZero::new(orig_h).unwrap(),
            img.clone().into_raw(),
            fir::PixelType::U8x4,
        )
        .unwrap();

        let mut dst = fir::Image::new(
            NonZero::new(draw_w).unwrap(),
            NonZero::new(draw_h).unwrap(),
            fir::PixelType::U8x4,
        );

        let mut resizer =
            fir::Resizer::new(fir::ResizeAlg::Convolution(fir::FilterType::CatmullRom));
        resizer.resize(&src.view(), &mut dst.view_mut()).unwrap();

        let cache = CachedResize {
            area: area_key,
            data: dst.buffer().to_vec(),
            draw_w,
            draw_h,
            offset_x,
            offset_y,
        };

        Self::render_pixels(&cache, area, buf);
        *self.cached_resize.borrow_mut() = Some(cache);
        Ok(())
    }

    fn needs_render(&self) -> bool {
        self.needs_render.load(Ordering::Relaxed)
    }

    fn mark_rendered(&mut self) {
        self.needs_render.store(false, Ordering::Relaxed);
    }

    fn handle_state_change(&mut self, key: &StateKey, value: &StateValue) {
        match key {
            StateKey::CurrentTrackPath => {
                if let Some(path) = value.as_pathbuf() {
                    const ART_CANDIDATES: &[&str] = &[
                        "cover.jpg", "cover.jpeg", "cover.png", "cover.webp",
                        "folder.jpg", "folder.jpeg", "folder.png",
                        "album.jpg", "album.jpeg", "album.png",
                    ];

                    let found = ART_CANDIDATES.iter().find_map(|name| {
                        let mut p = path.clone();
                        p.set_file_name(name);
                        if p.exists() { Some(p) } else { None }
                    });

                    if let Some(art_path) = found {
                        if let Some(s) = art_path.to_str() {
                            self.start_load(s.to_string());
                        }
                    } else {
                        // No cover art for this track.
                        self.current_art_path = None;
                        self.pending = None;
                        if self.img.is_some() {
                            self.img = None;
                            *self.cached_resize.borrow_mut() = None;
                            self.needs_render.store(true, Ordering::Relaxed);
                        }
                    }
                }
            }
            StateKey::TerminalSize => {
                if let Some((w, h)) = value.as_terminal_size() {
                    if self.last_terminal_size != (w, h) {
                        self.last_terminal_size = (w, h);
                        if self.img.is_some() {
                            *self.cached_resize.borrow_mut() = None;
                            self.needs_render.store(true, Ordering::Relaxed);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn subscribed_keys(&self) -> Vec<StateKey> {
        vec![StateKey::CurrentTrackPath, StateKey::TerminalSize]
    }
}

// ---------------------------------------------------------------------------
// Palette extraction
// ---------------------------------------------------------------------------

fn rgb_to_hsv(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;

    let v = max;
    let s = if max > 0.0 { delta / max } else { 0.0 };

    let h = if delta < 1e-6 {
        0.0
    } else if (max - r).abs() < 1e-6 {
        60.0 * (((g - b) / delta) % 6.0)
    } else if (max - g).abs() < 1e-6 {
        60.0 * ((b - r) / delta + 2.0)
    } else {
        60.0 * ((r - g) / delta + 4.0)
    };

    let h = if h < 0.0 { h + 360.0 } else { h };
    (h, s, v)
}

fn hue_distance(a: f32, b: f32) -> f32 {
    let d = (a - b).abs() % 360.0;
    d.min(360.0 - d)
}

/// Sample the image on a coarse grid and return `(primary, secondary)` colours.
///
/// Colours are chosen by accumulating per-pixel quality scores into coarse hue
/// buckets.  A bucket's total score rises with *both* the number of pixels that
/// fall into it *and* their individual saturation/brightness quality, so a
/// large block of moderately-saturated colour wins over a single vivid accent
/// pixel. The bucket's representative colour is the most vivid individual
/// pixel in that bucket, keeping the output clean rather than muddied by
/// averaging.
fn extract_palette(img: &image::RgbaImage) -> ((u8, u8, u8), (u8, u8, u8)) {
    // 24 buckets → 15° each; fine enough to distinguish adjacent hues while
    // still grouping nearby shades into a single majority block.
    const HUE_BUCKETS: usize = 24;

    // Per-bucket state: (accumulated_weight, best_pixel_quality, r, g, b, hue)
    let mut buckets: Vec<(f32, f32, u8, u8, u8, f32)> = vec![(0.0, 0.0, 0, 0, 0, 0.0); HUE_BUCKETS];

    let w = img.width();
    let h = img.height();
    let step = ((w.min(h) / 40).max(1)) as usize;

    for y in (0..h as usize).step_by(step) {
        for x in (0..w as usize).step_by(step) {
            let px = img.get_pixel(x as u32, y as u32);
            let (rf, gf, bf) = (
                px[0] as f32 / 255.0,
                px[1] as f32 / 255.0,
                px[2] as f32 / 255.0,
            );
            let (hue, sat, val) = rgb_to_hsv(rf, gf, bf);

            // Skip near-black, near-white, and near-grey pixels, they carry
            // no useful hue information and inflate whichever bucket they fall
            // into, washing out the true dominant colour.
            if sat < 0.15 || val < 0.12 || val > 0.97 {
                continue;
            }

            let brightness_penalty = ((val - 0.55) * 2.0).abs().min(1.0);
            let quality = sat * (1.0 - brightness_penalty * 0.7);
            if quality < 0.05 {
                continue;
            }

            let bin = ((hue / 360.0) * HUE_BUCKETS as f32) as usize % HUE_BUCKETS;
            let bucket = &mut buckets[bin];
            // Accumulate weight: every pixel contributes, so dominant areas
            // naturally outweigh isolated accent pixels.
            bucket.0 += quality;
            // Track the most vivid individual pixel as the representative so
            // the final colour is clean, not an averaged muddy blend.
            if quality > bucket.1 {
                bucket.1 = quality;
                bucket.2 = px[0];
                bucket.3 = px[1];
                bucket.4 = px[2];
                bucket.5 = hue;
            }
        }
    }

    // Sort buckets by accumulated weight descending.
    let mut ranked: Vec<(f32, f32, u8, u8, u8, f32)> = buckets
        .into_iter()
        .filter(|b| b.0 > 0.0)
        .collect();
    ranked.sort_unstable_by(|a, b| {
        b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal)
    });

    let default_primary = (100u8, 149u8, 237u8);  // cornflower blue
    let default_secondary = (255u8, 255u8, 255u8); // white

    let (primary, primary_hue) = ranked
        .first()
        .map(|b| ((b.2, b.3, b.4), b.5))
        .unwrap_or((default_primary, 0.0));

    // Secondary: highest-weight bucket whose hue is ≥60° from primary.
    // Fall back to ≥30° if nothing qualifies, then white.
    let secondary = ranked
        .iter()
        .find(|b| hue_distance(b.5, primary_hue) >= 60.0)
        .or_else(|| ranked.iter().find(|b| hue_distance(b.5, primary_hue) >= 30.0))
        .map(|b| (b.2, b.3, b.4))
        .unwrap_or(default_secondary);

    (primary, secondary)
}

// ---------------------------------------------------------------------------
// Image loading
// ---------------------------------------------------------------------------

pub fn load_image_auto(path: &Path) -> Option<image::RgbaImage> {
    let bytes = fs::read(path).ok()?;
    if bytes.len() < 8 {
        return None;
    }

    let is_jpg = bytes[0] == 0xFF && bytes[1] == 0xD8;

    if is_jpg {
        // zune-jpeg is faster than the image crate for JPEG; output is always RGB.
        let mut decoder = JpegDecoder::new(&bytes);
        let pixels = decoder.decode().ok()?;
        let info = decoder.info()?;
        let mut rgba_data = Vec::with_capacity((info.width * info.height * 4) as usize);
        for chunk in pixels.chunks_exact(3) {
            rgba_data.extend_from_slice(&[chunk[0], chunk[1], chunk[2], 255]);
        }
        ImageBuffer::<Rgba<u8>, _>::from_raw(info.width as u32, info.height as u32, rgba_data)
    } else {
        // PNG, WebP, BMP, etc. image crate handles all of them and always
        // produces correct RGBA output regardless of source channel count.
        image::load_from_memory(&bytes).ok().map(|img| img.to_rgba8())
    }
}
