use image::{ImageBuffer, Rgba};
use crate::widget::{AbsRect, Widget, Cell};
use crate::state::{GlobalState, StateKey, StateValue};
use crossterm::style::Color;
use fast_image_resize as fir;
use std::sync::atomic::{AtomicBool, Ordering};
use std::num::NonZero;
use std::fs;
use std::path::Path;
use zune_jpeg::JpegDecoder;
use zune_png::PngDecoder;
use zune_core::result::DecodingResult;

pub struct AlbumArt {
    current_art_path: Option<String>,
    img: Option<image::RgbaImage>,
    stretch: bool,
    needs_render: AtomicBool,
    last_terminal_size: (u16, u16),
}

impl AlbumArt {
    pub fn new(stretch: bool) -> Self {
        Self {
            current_art_path: None,
            img: None,
            stretch,
            needs_render: AtomicBool::new(true),
            last_terminal_size: (0, 0),
        }
    }

    fn load_image(&mut self, path: &str) -> bool {
        if self.current_art_path.as_ref().map(|p| p.as_str()) == Some(path) {
            return false;
        }

        let path_obj = std::path::Path::new(path);
        if let Some(rgba_img) = load_image_auto(path_obj) {
            self.img = Some(rgba_img);
            self.current_art_path = Some(path.to_string());
            self.needs_render.store(true, Ordering::Relaxed);
            return true;
        }

        false
    }
}

impl Widget for AlbumArt {
    fn update(&mut self, _state: &GlobalState) {
        // Load initial image if none is loaded
        if self.img.is_none() && self.current_art_path.is_none() {
            // Try to load a default image
            self.load_image("test-tracks/Woodland/cover.jpg");
        }
    }

    fn render(&self, area: &AbsRect, buf: &mut [Vec<Cell>]) -> std::io::Result<()> {
        let img = match &self.img {
            Some(img) => img,
            None => {
                // Draw placeholder
                let placeholder = "♪ No Album Art ♪";
                let y = area.y as usize + area.height as usize / 2;
                let x_start = area.x as usize + (area.width as usize - placeholder.len()) / 2;
                for (i, ch) in placeholder.chars().enumerate() {
                    if y < buf.len() && x_start + i < buf[y].len() {
                        buf[y][x_start + i].ch = ch;
                        buf[y][x_start + i].fg = Some(Color::DarkGrey);
                    }
                }
                return Ok(());
            }
        };

        let orig_w = img.width();
        let orig_h = img.height();
        let target_w = (area.width + 1) as u32;
        let target_h = (area.height + 1) as u32 * 2; // double vertical for half-block mapping

        // Calculate aspect ratios
        let img_aspect = orig_w as f32 / orig_h as f32;
        let target_aspect = target_w as f32 / target_h as f32;

        // Determine draw dimensions and offsets based on aspect ratio
        let (draw_w, draw_h, offset_x, offset_y) = if self.stretch {
            (target_w, target_h, 0, 0)
        } else if img_aspect > target_aspect {
            // Image is wider than target - fit to width
            let draw_w = target_w;
            let draw_h = (target_w as f32 / img_aspect).round().min(target_h as f32) as u32;
            let offset_x = 0;
            let offset_y = ((target_h as i32 - draw_h as i32) / 2).max(0) as u32;
            (draw_w, draw_h, offset_x, offset_y)
        } else {
            // Image is taller than target - fit to height
            let draw_h = target_h;
            let draw_w = (target_h as f32 * img_aspect).round().min(target_w as f32) as u32;
            let offset_x = ((target_w as i32 - draw_w as i32) / 2).max(0) as u32;
            let offset_y = 0;
            (draw_w, draw_h, offset_x, offset_y)
        };

        // Create source image with ORIGINAL dimensions
        let src_img = fir::Image::from_vec_u8(
            NonZero::new(orig_w).unwrap(),
            NonZero::new(orig_h).unwrap(),
            img.clone().into_raw(),
            fir::PixelType::U8x4,
        ).unwrap();

        // Create destination image with CALCULATED dimensions (not full target)
        let mut dst_img = fir::Image::new(
            NonZero::new(draw_w).unwrap(),
            NonZero::new(draw_h).unwrap(),
            fir::PixelType::U8x4
        );

        let mut resizer = fir::Resizer::new(
            fir::ResizeAlg::Convolution(fir::FilterType::CatmullRom)
        );
        resizer.resize(&src_img.view(), &mut dst_img.view_mut()).unwrap();

        // Get scaled pixel data
        let scaled = dst_img.buffer();
        let stride = (draw_w * 4) as usize;

        // Map two vertical pixels to one terminal cell using '▀'
        for by in 0..(draw_h / 2) {
            for bx in 0..draw_w {
                let top_offset = (by * 2 * stride as u32 + bx * 4) as usize;
                let bot_offset = ((by * 2 + 1) * stride as u32 + bx * 4) as usize;

                let top_px = &scaled[top_offset..top_offset + 4];
                let bot_px = &scaled[bot_offset..bot_offset + 4];

                let fg = Color::Rgb {
                    r: top_px[0],
                    g: top_px[1],
                    b: top_px[2],
                };
                let bg = Color::Rgb {
                    r: bot_px[0],
                    g: bot_px[1],
                    b: bot_px[2],
                };

                // Apply offsets when writing to buffer
                let x = area.x as usize + offset_x as usize + bx as usize;
                let y = area.y as usize + (offset_y / 2) as usize + by as usize;

                if y < buf.len() && x < buf[y].len() {
                    let cell = &mut buf[y][x];
                    cell.ch = '▀';
                    cell.fg = Some(fg);
                    cell.bg = Some(bg);
                }
            }
        }

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
                    let mut art_path = path.clone();
                    art_path.set_file_name("cover.jpg");
                    
                    if art_path.exists() {
                        if let Some(path_str) = art_path.to_str() {
                            // Only load if the path is different - this prevents flickering
                            self.load_image(path_str);
                        }
                    } else {
                        // If no cover.jpg exists, only clear if we had a previous image
                        if self.current_art_path.is_some() {
                            self.img = None;
                            self.current_art_path = None;
                            self.needs_render.store(true, Ordering::Relaxed);
                        }
                    }
                }
            }
            StateKey::TerminalSize => {
                if let Some((w, h)) = value.as_terminal_size() {
                    if self.last_terminal_size != (w, h) {
                        self.last_terminal_size = (w, h);
                        // Only mark for re-render if we have an image to render
                        if self.img.is_some() {
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

pub fn load_image_auto(path: &Path) -> Option<image::RgbaImage> {
    let bytes = fs::read(path).ok()?;
    if bytes.len() < 8 {
        return None;
    }

    let is_png = bytes.starts_with(&[0x89, b'P', b'N', b'G']);
    let is_jpg = bytes[0] == 0xFF && bytes[1] == 0xD8;

    if is_png {
        let mut decoder = PngDecoder::new(&bytes);
        let pixels = decoder.decode().ok()?;
        let (width, height) = decoder.get_dimensions()?;
        
        // Extract RGBA data from DecodingResult
        let rgba_data = match pixels {
            DecodingResult::U8(data) => data,
            _ => return None,
        };

        ImageBuffer::<Rgba<u8>, _>::from_raw(width as u32, height as u32, rgba_data)
    } else if is_jpg {
        let mut decoder = JpegDecoder::new(&bytes);
        let pixels = decoder.decode().ok()?; // returns Vec<u8>
        let info = decoder.info().unwrap();
        let width = info.width as u32;
        let height = info.height as u32;

        // Convert RGB → RGBA
        let mut rgba_data = Vec::with_capacity((width * height * 4) as usize);
        for chunk in pixels.chunks_exact(3) {
            rgba_data.extend_from_slice(&[chunk[0], chunk[1], chunk[2], 255]);
        }

        ImageBuffer::<Rgba<u8>, _>::from_raw(width, height, rgba_data)
    } else {
        None
    }
}