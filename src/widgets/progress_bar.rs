use crate::canvas::Canvas;
use crate::widget::Widget;
use crate::state::{GlobalState, StateKey, StateValue};
use crossterm::style::Color;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug)]
pub struct ProgressBar {
    progress: f64,
    total: f64,
    needs_render: AtomicBool,
}

impl ProgressBar {
    pub fn new() -> Self {
        Self {
            progress: 0.0,
            total: 0.0,
            needs_render: AtomicBool::new(true),
        }
    }

    fn format_time(seconds: f64) -> String {
        let total_seconds = seconds.floor() as u64;
        let hours = total_seconds / 3600;
        let minutes = (total_seconds % 3600) / 60;
        let secs = total_seconds % 60;

        if hours > 0 {
            format!("{:02}:{:02}:{:02}", hours, minutes, secs)
        } else {
            format!("{:02}:{:02}", minutes, secs)
        }
    }
}

impl Widget for ProgressBar {
    fn update(&mut self, state: &GlobalState) {
        // Get current values from state
        let current_progress = state.playback_position();
        let current_total = state.track_duration();
        
        // Update if changed significantly
        if (self.progress - current_progress).abs() > 0.1 || self.total != current_total {
            self.progress = current_progress;
            self.total = current_total;
            self.needs_render.store(true, Ordering::Relaxed);
        }
    }

    fn render(&self, canvas: &mut Canvas) -> std::io::Result<()> {
        const PADDING: usize = 2;

        if canvas.width() <= PADDING * 2 {
            return Ok(());
        }

        let left_time = Self::format_time(self.progress);
        let right_time = Self::format_time(self.total);

        let width = canvas.width()
            .saturating_sub(PADDING * 4)
            .saturating_sub(left_time.len())
            .saturating_sub(right_time.len());

        let frac = if self.total > 0.0 { self.progress / self.total } else { 0.0 };
        let filled_len = (frac * width as f64).round() as usize;
        let bar_y = canvas.height() / 2;

        // Draw left time
        canvas.put_str(PADDING, bar_y, &left_time, Some(Color::White), None);

        // Draw progress bar
        let bar_x = PADDING * 2 + left_time.len();
        for i in 0..width {
            let ch = if i < filled_len { '█' } else { '░' };
            canvas.set(bar_x + i, bar_y, ch, Some(Color::White), None);
        }

        // Draw right time
        let right_x = PADDING * 3 + left_time.len() + width;
        canvas.put_str(right_x, bar_y, &right_time, Some(Color::White), None);

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
            StateKey::PlaybackPosition => {
                if let Some(pos) = value.as_f64() {
                    if (self.progress - pos).abs() > 0.1 {
                        self.progress = pos;
                        self.needs_render.store(true, Ordering::Relaxed);
                    }
                }
            }
            StateKey::TrackDuration => {
                if let Some(duration) = value.as_f64() {
                    if self.total != duration {
                        self.total = duration;
                        self.needs_render.store(true, Ordering::Relaxed);
                    }
                }
            }
            _ => {}
        }
    }

    fn subscribed_keys(&self) -> Vec<StateKey> {
        vec![StateKey::PlaybackPosition, StateKey::TrackDuration]
    }
}