use crate::widget::{AbsRect, Widget, Cell};
use std::sync::{Arc, atomic::{AtomicU32, Ordering, AtomicBool}};
use crossterm::style::Color;
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone)]
pub struct Controls {
    pub is_paused: Arc<AtomicBool>,
    pub volume: Arc<AtomicU32>,
    cached_text: String,
}

impl Controls {
    pub fn new(is_paused: Arc<AtomicBool>, volume: Arc<AtomicU32>) -> Self {
        Self {
            is_paused,
            volume,
            cached_text: String::new(),
        }
    }
}

impl Widget for Controls {
    fn update(&mut self) {
        let paused = self.is_paused.load(Ordering::Relaxed);
        let vol = f32::from_bits(self.volume.load(Ordering::Relaxed)) * 100.0;

        self.cached_text = format!(
            "{}  |  Vol: {:.0}%",
            if paused { "▶ Play" } else { "⏸ Pause" },
            vol
        );
    }

    fn render(&self, area: &AbsRect, buf: &mut [Vec<Cell>]) -> std::io::Result<()> {
        let (abs_x, abs_y, abs_width, abs_height) = (area.x, area.y, area.width, area.height);
        let y = abs_y as usize + (abs_height as usize / 2);

        let text_width = UnicodeWidthStr::width(self.cached_text.as_str());
        let x = abs_x as usize + (abs_width as usize).saturating_sub(text_width) / 2;

        for (i, ch) in self.cached_text.chars().enumerate() {
            if y < buf.len() && x + i < buf[0].len() {
                buf[y][x + i].ch = ch;
                buf[y][x + i].fg = Some(Color::Cyan);
            }
        }

        Ok(())
    }
}