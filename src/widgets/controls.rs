use crate::widget::{Rect, Widget};
use std::sync::{Arc, atomic::{AtomicU32, Ordering, AtomicBool}};
use crossterm::{
    cursor,
    execute,
    style::{Color, Print, ResetColor, SetForegroundColor}
};
use std::io::{Result, Stdout};

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

    fn render(&self, area: &Rect, out: &mut Stdout) -> Result<()> {
        let x = area.x + (area.width.saturating_sub(self.cached_text.len() as u16) / 2);
        let y = area.y + (area.height / 2);
        execute!(
            out,
            cursor::MoveTo(x, y),
            SetForegroundColor(Color::Cyan),
            Print(&self.cached_text),
            ResetColor
        )
    }
}