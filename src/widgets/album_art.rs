use image::ImageReader;
use crate::widget::{AbsRect, Widget, Cell};
use crate::state::{GlobalState, StateKey, StateValue};
use crossterm::style::Color;
use std::sync::atomic::{AtomicBool, Ordering};

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
        // Only load if the path is actually different
        if self.current_art_path.as_ref().map(|p| p.as_str()) == Some(path) {
            return false; // No change needed
        }

        if let Ok(reader) = ImageReader::open(path) {
            if let Ok(dynamic_img) = reader.decode() {
                let rgba_img = dynamic_img.to_rgba8();
                self.img = Some(rgba_img);
                self.current_art_path = Some(path.to_string());
                self.needs_render.store(true, Ordering::Relaxed);
                return true;
            }
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
        // If no image is loaded, show a placeholder
        if self.img.is_none() {
            // Draw a simple placeholder
            let placeholder_text = "♪ No Album Art ♪";
            let y = area.y as usize + (area.height as usize / 2);
            let x_start = area.x as usize + 
                (area.width as usize).saturating_sub(placeholder_text.len()) / 2;

            for (i, ch) in placeholder_text.chars().enumerate() {
                let x = x_start + i;
                if y < buf.len() && x < buf[0].len() {
                    buf[y][x].ch = ch;
                    buf[y][x].fg = Some(Color::DarkGrey);
                }
            }
            return Ok(());
        }

        let img = self.img.as_ref().unwrap();
        let (img_w, img_h) = img.dimensions();
        let target_w = (area.width as u32).saturating_add(1);
        let target_h = (area.height as u32).saturating_add(1);

        if target_w == 0 || target_h == 0 {
            return Ok(());
        }

        // Account for terminal character aspect ratio
        let img_aspect = img_w as f32 / img_h as f32 * 1.9;
        let target_aspect = target_w as f32 / target_h as f32;

        let (draw_w, draw_h, offset_x, offset_y) = if self.stretch {
            (target_w, target_h, 0, 0)
        } else if img_aspect > target_aspect {
            let draw_w = target_w;
            let draw_h = (target_w as f32 / img_aspect).round().min(target_h as f32) as u32;
            let offset_x = 0;
            let offset_y = ((target_h as i32 - draw_h as i32) / 2).max(0);
            (draw_w, draw_h, offset_x, offset_y)
        } else {
            let draw_h = target_h;
            let draw_w = (target_h as f32 * img_aspect).round().min(target_w as f32) as u32;
            let offset_x = ((target_w as i32 - draw_w as i32) / 2).max(0);
            let offset_y = 0;
            (draw_w, draw_h, offset_x, offset_y)
        };

        for by in 0..draw_h {
            for bx in 0..draw_w {
                let x0 = bx * img_w / draw_w;
                let x1 = (bx + 1) * img_w / draw_w;

                // Top half (FG)
                let y0_top = (by * 2) * img_h / (draw_h * 2);
                let y1_top = (by * 2 + 1) * img_h / (draw_h * 2);

                let mut r_sum: u64 = 0;
                let mut g_sum: u64 = 0;
                let mut b_sum: u64 = 0;
                let mut count: u64 = 0;

                for y in y0_top..y1_top {
                    for x in x0..x1 {
                        let p = img.get_pixel(x, y).0;
                        r_sum += p[0] as u64;
                        g_sum += p[1] as u64;
                        b_sum += p[2] as u64;
                        count += 1;
                    }
                }

                let fg = if count > 0 {
                    Color::Rgb {
                        r: (r_sum / count) as u8,
                        g: (g_sum / count) as u8,
                        b: (b_sum / count) as u8,
                    }
                } else {
                    Color::Black
                };

                // Bottom half (BG)
                let y0_bot = (by * 2 + 1) * img_h / (draw_h * 2);
                let y1_bot = (by * 2 + 2) * img_h / (draw_h * 2);

                let mut r_sum: u64 = 0;
                let mut g_sum: u64 = 0;
                let mut b_sum: u64 = 0;
                let mut count: u64 = 0;

                for y in y0_bot..y1_bot {
                    for x in x0..x1 {
                        let p = img.get_pixel(x, y).0;
                        r_sum += p[0] as u64;
                        g_sum += p[1] as u64;
                        b_sum += p[2] as u64;
                        count += 1;
                    }
                }

                let bg = if count > 0 {
                    Color::Rgb {
                        r: (r_sum / count) as u8,
                        g: (g_sum / count) as u8,
                        b: (b_sum / count) as u8,
                    }
                } else {
                    Color::Black
                };

                let x = area.x as i32 + bx as i32 + offset_x;
                let y = area.y as i32 + by as i32 + offset_y;

                if y >= 0 && x >= 0 && (y as usize) < buf.len() && (x as usize) < buf[0].len() {
                    buf[y as usize][x as usize].ch = '▀';
                    buf[y as usize][x as usize].fg = Some(fg);
                    buf[y as usize][x as usize].bg = Some(bg);
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