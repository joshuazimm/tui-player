use std::io::Write;

use crate::widgets::WidgetKind;
use crossterm::{cursor, terminal::size};
use enum_dispatch::enum_dispatch;

#[derive(Debug, Clone, Copy)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn to_absolute(&self) -> (u16, u16, u16, u16) {
        let (term_width, term_height) = size().unwrap_or((80, 24));
        let x = (self.x * term_width as f32) as u16;
        let y = (self.y * term_height as f32) as u16;
        let width = (self.width * term_width as f32) as u16;
        let height = (self.height * term_height as f32) as u16;
        (x, y, width, height)
    }
}

pub struct Section {
    pub rect: Rect,
    pub last_absolute_size: Option<(u16, u16, u16, u16)>,
    pub border: bool,
    pub widget: WidgetKind,
}

impl Section {
    pub fn new(rect: Rect, border: bool, widget: WidgetKind) -> Self {
        Self {
            rect,
            border,
            widget,
            last_absolute_size: None,
        }
    }

    pub fn clear_area(&self, buf: &mut String) -> (u16, u16, u16, u16) {
        let current_abs = self.rect.to_absolute();

        // Get the area to clear (max of current and last size)
        let (clear_x, clear_y, clear_width, clear_height) =
            if let Some(last_abs) = self.last_absolute_size {
                let (curr_x, curr_y, curr_w, curr_h) = current_abs;
                let (last_x, last_y, last_w, last_h) = last_abs;

                let min_x = curr_x.min(last_x);
                let min_y = curr_y.min(last_y);
                let max_right = (curr_x + curr_w).max(last_x + last_w);
                let max_bottom = (curr_y + curr_h).max(last_y + last_h);

                (min_x, min_y, max_right - min_x, max_bottom - min_y)
            } else {
                // First render, just use current size
                current_abs
            };

        // Clear the calculated area
        for dy in 0..clear_height {
            buf.push_str(&format!(
                "{}{}",
                cursor::MoveTo(clear_x, clear_y + dy),
                " ".repeat(clear_width as usize)
            ));
        }

        current_abs
    }
}

#[enum_dispatch]
pub trait Widget {
    fn update(&mut self);
    fn render(&self, area: &Rect, buf: &mut String) -> std::io::Result<()>;
}

pub struct Layout {
    pub sections: Vec<Section>,
}

impl Layout {
    pub fn update_all(&mut self) -> std::io::Result<()> {
        for section in &mut self.sections {
            section.widget.update();
        }
        Ok(())
    }

    pub fn render_all(&mut self, out: &mut std::io::Stdout) -> std::io::Result<()> {
        let mut buf = String::new();
        for section in &mut self.sections {
            section.last_absolute_size = Some(section.clear_area(&mut buf));
            if section.border {
                let _ = Self::draw_border(&section.rect, &mut buf);
            }
            section.widget.render(&section.rect, &mut buf)?;
        }
        write!(out, "{}", buf)?;
        out.flush()
    }

    fn draw_border(rect: &Rect, buf: &mut String) -> std::io::Result<()> {
        let (abs_x, abs_y, abs_width, abs_height) = rect.to_absolute();

        let top = abs_y;
        let bottom = abs_y + abs_height.saturating_sub(1);
        let left = abs_x;
        let right = abs_x + abs_width.saturating_sub(1);

        // Corners
        buf.push_str(&format!("{}┌", cursor::MoveTo(left, top)));
        buf.push_str(&format!("{}┐", cursor::MoveTo(right, top)));
        buf.push_str(&format!("{}└", cursor::MoveTo(left, bottom)));
        buf.push_str(&format!("{}┘", cursor::MoveTo(right, bottom)));

        // Horizontal lines
        for dx in 1..abs_width.saturating_sub(1) {
            buf.push_str(&format!("{}─", cursor::MoveTo(left + dx, top)));
            buf.push_str(&format!("{}─", cursor::MoveTo(left + dx, bottom)));
        }

        // Vertical lines
        for dy in 1..abs_height.saturating_sub(1) {
            buf.push_str(&format!("{}│", cursor::MoveTo(left, top + dy)));
            buf.push_str(&format!("{}│", cursor::MoveTo(right, top + dy)));
        }

        Ok(())
    }
}

