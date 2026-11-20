use crate::canvas::Canvas;
use crate::widget::Widget;
use crate::state::{GlobalState, StateKey, StateValue};
use crossterm::style::Color;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug)]
pub struct Controls {
    cached_text: String,
    needs_render: AtomicBool,
    is_playing: bool,
    volume: f32,
}

impl Controls {
    pub fn new() -> Self {
        let mut controls = Self {
            cached_text: String::new(),
            needs_render: AtomicBool::new(true),
            is_playing: false,
            volume: GlobalState::DEFAULT_VOLUME,
        };
        // The text has to be built here, not left to the first `update`: the
        // fields above already match what a fresh `GlobalState` reports, so
        // `update` sees no change and would never fill the text in, leaving the
        // widget drawing an empty string until the first keypress.
        controls.rebuild_text();
        controls
    }

    /// The transport line: the action the play/pause key would take, then volume.
    ///
    /// Both glyphs are deliberately single-width. `flush_buffer` writes one
    /// cell per char and emits a run of changed cells after a single `MoveTo`,
    /// so a glyph the terminal draws double-width (U+23F8 has emoji
    /// presentation almost everywhere) would shift the rest of the line one
    /// column right and leave a stale cell behind.
    fn rebuild_text(&mut self) {
        let new_text = format!(
            "{}  |  Vol: {:.0}%",
            if self.is_playing { "\u{258c}\u{258c} Pause" } else { "\u{25ba} Play" },
            self.volume * 100.0
        );

        if self.cached_text != new_text {
            self.cached_text = new_text;
            self.needs_render.store(true, Ordering::Relaxed);
        }
    }

    /// Volume only shows as a whole percent, so a change is worth a redraw
    /// exactly when that number moves. (A plain `> 0.01` epsilon was the same
    /// size as the keyboard's volume step, so single presses registered or not
    /// depending on float rounding.)
    fn volume_changed(&self, volume: f32) -> bool {
        (self.volume * 100.0).round() != (volume * 100.0).round()
    }
}

impl Widget for Controls {
    fn update(&mut self, state: &GlobalState) {
        let is_playing = state.is_playing();
        let volume = state.volume();

        if self.is_playing != is_playing || self.volume_changed(volume) {
            self.is_playing = is_playing;
            self.volume = volume;
            self.rebuild_text();
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
            StateKey::IsPlaying => {
                if let Some(is_playing) = value.as_bool() {
                    if self.is_playing != is_playing {
                        self.is_playing = is_playing;
                        self.rebuild_text();
                    }
                }
            }
            StateKey::Volume => {
                if let Some(volume) = value.as_f32() {
                    if self.volume_changed(volume) {
                        self.volume = volume;
                        self.rebuild_text();
                    }
                }
            }
            _ => {}
        }
    }

    fn subscribed_keys(&self) -> Vec<StateKey> {
        vec![StateKey::IsPlaying, StateKey::Volume]
    }
}
