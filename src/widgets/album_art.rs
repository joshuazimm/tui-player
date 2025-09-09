use image::ImageReader;
use crate::widget::{AbsRect, Widget, Cell};
use crossterm::style::Color;

#[derive(Debug, Clone)]
pub struct AlbumArt {
    path: Option<String>,
    img: Option<image::RgbaImage>,
    stretch: bool,
}

impl AlbumArt {
    pub fn new(stretch: bool) -> Self {
        Self {
            path: None,
            img: None,
            stretch: stretch,
        }
    }
}

impl Widget for AlbumArt {
    fn update(&mut self) {
        let path = "test-tracks/cover.jpg";
        let img = ImageReader::open(path)
            .unwrap()
            .decode()
            .unwrap()
            .to_rgba8();

        self.path = Some(path.to_string());
        self.img = Some(img);
    }

    fn render(&self, area: &AbsRect, buf: &mut [Vec<Cell>]) -> std::io::Result<()> {
        if self.img.is_none() {
            return Ok(());
        }

        let img = self.img.as_ref().unwrap();
        let (img_w, img_h) = img.dimensions();
        let target_w = (area.width as u32).saturating_add(1);
        let target_h = (area.height as u32).saturating_add(1);

        if target_w == 0 || target_h == 0 {
            return Ok(());
        }

        // Multiply img_w by 2 to account for terminal character aspect ratio
        // being roughly 1:2 (width:height)
        let img_aspect = img_w as f32 / img_h as f32 * 1.9;
        let target_aspect = target_w as f32 / target_h as f32;

        let (draw_w, draw_h, offset_x, offset_y) = if self.stretch {
            // Fill entire area
            (target_w, target_h, 0, 0)
        } else if img_aspect > target_aspect {
            // Width limited
            let draw_w = target_w;
            let draw_h = (target_w as f32 / img_aspect).round().min(target_h as f32) as u32;
            let offset_x = 0;
            let offset_y = ((target_h as i32 - draw_h as i32) / 2).max(0);
            (draw_w, draw_h, offset_x, offset_y)
        } else {
            // Height limited
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

                // Write Both FG and BG Pixels to Buffer
                let x = area.x as i32 + bx as i32 + offset_x;
                let y = area.y as i32 + by as i32 + offset_y;

                buf[y as usize][x as usize].ch = '▀';
                buf[y as usize][x as usize].fg = Some(fg);
                buf[y as usize][x as usize].bg = Some(bg);
            }
        }

        Ok(())
    }
}
