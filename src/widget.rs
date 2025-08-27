use std::io::Write;

use enum_dispatch::enum_dispatch;
use crate::widgets::WidgetKind;
use crossterm::{cursor};


#[derive(Debug, Clone, Copy)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

pub struct Section {
    pub rect: Rect,
    pub border: bool,
    pub widget: WidgetKind
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

    pub fn render_all(&self, out: &mut std::io::Stdout) -> std::io::Result<()> {
        let mut buf = String::new();
        for section in &self.sections {
            if section.border {
                let _ = Self::draw_border(&section.rect, &mut buf);
            }
            section.widget.render(&section.rect, &mut buf)?;
        }
        write!(out, "{}", buf)?;
        out.flush()
    }

    fn draw_border(rect: &Rect, buf: &mut String) -> std::io::Result<()> {
        let top = rect.y;
        let bottom = rect.y + rect.height - 1;
        let left = rect.x;
        let right = rect.x + rect.width - 1;

        // corners
        buf.push_str(&format!("{}┌", cursor::MoveTo(left, top)));
        buf.push_str(&format!("{}┐", cursor::MoveTo(right, top)));
        buf.push_str(&format!("{}└", cursor::MoveTo(left, bottom)));
        buf.push_str(&format!("{}┘", cursor::MoveTo(right, bottom)));

        // horizontal lines
        for dx in 1..(rect.width - 1) {
            buf.push_str(&format!("{}─", cursor::MoveTo(left + dx, top)));
            buf.push_str(&format!("{}─", cursor::MoveTo(left + dx, bottom)));
        }

        // vertical lines
        for dy in 1..(rect.height - 1) {
            buf.push_str(&format!("{}│", cursor::MoveTo(left, top + dy)));
            buf.push_str(&format!("{}│", cursor::MoveTo(right, top + dy)));
        }

        Ok(())
    }

}