use crate::widget::{AbsRect, Widget, Cell};
use crate::state::{GlobalState, StateKey, StateValue};
use crossterm::style::Color;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug)]
pub struct FpsCounter {
    cached_text: String,
    needs_render: AtomicBool,
    current_fps: f64,
}

impl FpsCounter {
    pub fn new() -> Self {
        Self {
            cached_text: "FPS: 0.0".to_string(),
            needs_render: AtomicBool::new(true),
            current_fps: 0.0,
        }
    }

    fn update_cached_text(&mut self) {
        let new_text = format!("FPS: {:.1}", self.current_fps);
        if self.cached_text != new_text {
            self.cached_text = new_text;
            self.needs_render.store(true, Ordering::Relaxed);
        }
    }
}

impl Widget for FpsCounter {
    fn update(&mut self, _state: &GlobalState) {
        // Updates come through handle_state_change
    }

    fn render(&self, area: &AbsRect, buf: &mut [Vec<Cell>]) -> std::io::Result<()> {
        use unicode_width::UnicodeWidthStr;
        
        // Center vertically
        let y = area.y as usize + (area.height as usize / 2);
        
        // Center horizontally based on text width
        let text_width = UnicodeWidthStr::width(self.cached_text.as_str());
        let x = area.x as usize + (area.width as usize).saturating_sub(text_width) / 2;

        for (i, ch) in self.cached_text.chars().enumerate() {
            if y < buf.len() && x + i < buf[0].len() {
                buf[y][x + i].ch = ch;
                buf[y][x + i].fg = Some(Color::Green);
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
            StateKey::Fps => {
                if let Some(fps) = value.as_f64() {
                    if (self.current_fps - fps).abs() > 0.1 {
                        self.current_fps = fps;
                        self.update_cached_text();
                    }
                }
            }
            _ => {}
        }
    }

    fn subscribed_keys(&self) -> Vec<StateKey> {
        vec![StateKey::Fps]
    }
}