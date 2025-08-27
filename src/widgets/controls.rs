use crate::widget::{Rect, Widget};
use std::sync::{Arc, atomic::{AtomicU32, Ordering, AtomicBool}};
use crossterm::{style::{SetForegroundColor, ResetColor, Color}, cursor};

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

    fn render(&self, area: &Rect, buf: &mut String) -> std::io::Result<()> {
        let y = area.y + (area.height / 2);

        // clear line avoiding vertical borders
        buf.push_str(&format!(
            "{}{}",
            cursor::MoveTo(area.x+1, y),
            " ".repeat((area.width - 2) as usize)
        ));

        let x = area.x + (area.width.saturating_sub(self.cached_text.len() as u16) / 2);
        buf.push_str(&format!(
            "{}{}{}{}",
            cursor::MoveTo(x, y),
            SetForegroundColor(Color::Cyan),
            self.cached_text,
            ResetColor
        ));
        Ok(())
    }
}