use crossterm::style::Color;
use crate::canvas::Canvas;
use crate::widget::Widget;
use crate::state::{GlobalState, StateKey, StateValue};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug)]
pub struct Label {
    pub text: String,
    pub align_center: bool,
    pub rainbow: bool,
    pub tick: usize,
    needs_render: AtomicBool,
    subscribed_keys: Vec<StateKey>,
}

impl Label {
    pub fn new(text: String, align_center: bool, rainbow: bool) -> Self {
        Self {
            text,
            align_center,
            rainbow,
            tick: 0,
            needs_render: AtomicBool::new(true),
            subscribed_keys: Vec::new(),
        }
    }

    pub fn with_subscription(mut self, keys: Vec<StateKey>) -> Self {
        self.subscribed_keys = keys;
        self
    }

    pub fn set_text(&mut self, new_text: impl Into<String>) {
        let new_text = new_text.into();
        if self.text != new_text {
            self.text = new_text;
            self.needs_render.store(true, Ordering::Relaxed);
        }
    }
}

fn rainbow_color(i: usize) -> Color {
    match i % 6 {
        0 => Color::Red,
        1 => Color::Yellow,
        2 => Color::Green,
        3 => Color::Cyan,
        4 => Color::Blue,
        _ => Color::Magenta,
    }
}

impl Widget for Label {
    fn update(&mut self, _state: &GlobalState) {
        self.tick = self.tick.wrapping_add(1);
    }

    fn render(&self, canvas: &mut Canvas) -> std::io::Result<()> {
        let width = canvas.width();

        // Truncate + ellipsis if text doesn't fit
        let mut display_text = self.text.clone();
        if display_text.len() > width {
            if width > 1 {
                display_text.truncate(width - 1);
                display_text.push('…');
            } else {
                display_text = "…".to_string();
            }
        }

        // Horizontal positioning
        let x_start = if self.align_center {
            width.saturating_sub(display_text.len()) / 2
        } else {
            0
        };

        // Vertical positioning (always center)
        let y = canvas.height() / 2;

        // Render text
        if self.rainbow {
            for (i, ch) in display_text.chars().enumerate() {
                canvas.set(x_start + i, y, ch, Some(rainbow_color(i + self.tick)), None);
            }
        } else {
            canvas.put_str(x_start, y, &display_text, Some(Color::White), None);
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
            StateKey::CurrentTrackTitle => {
                if let Some(title) = value.as_string() {
                    self.set_text(title);
                }
            }
            StateKey::CurrentTrackArtist => {
                if let Some(artist) = value.as_string() {
                    self.set_text(artist);
                }
            }
            StateKey::CurrentTrackAlbum => {
                if let Some(album) = value.as_string() {
                    self.set_text(album);
                }
            }
            _ => {
                self.needs_render.store(true, Ordering::Relaxed);
            }
        }
    }

    fn subscribed_keys(&self) -> Vec<StateKey> {
        self.subscribed_keys.clone()
    }
}