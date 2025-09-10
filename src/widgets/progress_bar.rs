use crate::widget::{AbsRect, Widget, Cell};
use crossterm::style::Color;

const PADDING: usize = 2;

#[derive(Debug, Clone)]
pub struct ProgressBar {
    pub progress: f64,
    pub total: f64,
}

impl ProgressBar {
    pub fn new() -> Self {
        Self {
            progress: 0.0,
            total: 0.0,
        }
    }

    pub fn set_progress(&mut self, progress: f64) {
        self.progress = progress.min(self.total);
    }

    pub fn set_total(&mut self, total: f64) {
        self.total = total;
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

impl Widget for ProgressBar {
    fn update(&mut self) {
    }

    fn render(&self, area: &AbsRect, buf: &mut [Vec<Cell>]) -> std::io::Result<()> {
        if area.width.saturating_add(1) <= (PADDING * 2) as u16 {
            return Ok(());
        }

        // Format left and right times
        let left_time = format_time(self.progress);
        let right_time = format_time(self.total);

        let width = (area.width as usize)
            .saturating_sub(PADDING * 4)
            .saturating_sub(left_time.len())
            .saturating_sub(right_time.len())
            .saturating_add(1)
            .max(0);

        let frac = if self.total > 0.0 {
            self.progress / self.total
        } else {
            0.0
        };

        let filled_len = (frac * width as f64).round() as usize;
        let bar_y = area.y as usize + (area.height.saturating_add(1) as usize / 2);

        // Draw left time
        for (i, ch) in left_time.chars().enumerate() {
            let buf_x = area.x as usize + PADDING + i;
            if bar_y < buf.len() && buf_x < buf[bar_y].len() {
                buf[bar_y][buf_x].ch = ch;
                buf[bar_y][buf_x].fg = Some(Color::White);
            }
        }

        // Draw progress bar
        for i in 0..width {
            let buf_x = area.x as usize + PADDING * 2 + left_time.len() + i;
            if bar_y < buf.len() && buf_x < buf[bar_y].len() {
                let ch = if i < filled_len { '█' } else { '░' };
                buf[bar_y][buf_x].ch = ch;
                buf[bar_y][buf_x].fg = Some(Color::White);
            }
        }

        // Draw right time
        for (i, ch) in right_time.chars().enumerate() {
            let buf_x = area.x as usize + PADDING * 3 + left_time.len() + width + i;
            if bar_y < buf.len() && buf_x < buf[bar_y].len() {
                buf[bar_y][buf_x].ch = ch;
                buf[bar_y][buf_x].fg = Some(Color::White);
            }
        }

        Ok(())
    }
}