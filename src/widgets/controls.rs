use crate::canvas::Canvas;
use crate::widget::Widget;
use crate::state::{GlobalState, StateKey, StateValue};
use crossterm::style::Color;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug)]
pub struct Controls {
    cached_text: String,
    needs_render: AtomicBool,
    is_paused: bool,
    volume: f32,
}

impl Controls {
    pub fn new() -> Self {
        Self {
            cached_text: String::new(),
            needs_render: AtomicBool::new(true),
            is_paused: false,
            volume: 0.05,
        }
    }

    fn update_cached_text(&mut self) {
        let new_text = format!(
            "{}  |  Vol: {:.0}%",
            if self.is_paused { "▶ Play" } else { "⏸ Pause" },
            self.volume * 100.0
        );
        
        if self.cached_text != new_text {
            self.cached_text = new_text;
            self.needs_render.store(true, Ordering::Relaxed);
        }
    }
}

impl Widget for Controls {
    fn update(&mut self, state: &GlobalState) {
        // Get current state values
        let current_paused = state.is_paused();
        let current_volume = state.volume();
        
        // Update if changed
        if self.is_paused != current_paused || (self.volume - current_volume).abs() > 0.01 {
            self.is_paused = current_paused;
            self.volume = current_volume;
            self.update_cached_text();
        }
    }

    fn render(&self, canvas: &mut Canvas) -> std::io::Result<()> {
        use unicode_width::UnicodeWidthStr;

        let y = canvas.height() / 2;
        let text_width = UnicodeWidthStr::width(self.cached_text.as_str());
        let x = canvas.width().saturating_sub(text_width) / 2;

        canvas.put_str(x, y, &self.cached_text, Some(Color::Cyan), None);
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
            StateKey::IsPaused => {
                if let Some(is_paused) = value.as_bool() {
                    if self.is_paused != is_paused {
                        self.is_paused = is_paused;
                        self.update_cached_text();
                    }
                }
            }
            StateKey::Volume => {
                if let Some(volume) = value.as_f32() {
                    if (self.volume - volume).abs() > 0.01 {
                        self.volume = volume;
                        self.update_cached_text();
                    }
                }
            }
            _ => {}
        }
    }

    fn subscribed_keys(&self) -> Vec<StateKey> {
        vec![StateKey::IsPaused, StateKey::Volume]
    }
}