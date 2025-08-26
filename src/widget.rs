use enum_dispatch::enum_dispatch;
use crossterm::{cursor, style::Print, execute};
use crate::widgets::WidgetKind;

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
    fn render(&self, area: &Rect, out: &mut std::io::Stdout) -> std::io::Result<()>;
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
        // TODO: Would be better to draw layout to a String buffer
        // and then print the whole buffer at once to avoid some elements
        // being drawn when others are not yet drawn

        // I have a feeling this is going to be pretty hard to implement
        for section in &self.sections {
            if section.border {
                Self::draw_border(&section.rect, out)?;
            }
            section.widget.render(&section.rect, out)?;
        }
        Ok(())
    }

    fn draw_border(rect: &Rect, out: &mut std::io::Stdout) -> std::io::Result<()> {
        for dx in 0..rect.width {
            execute!(out, cursor::MoveTo(rect.x + dx, rect.y), Print("─"))?;
            execute!(out, cursor::MoveTo(rect.x + dx, rect.y + rect.height - 1), Print("─"))?;
        }
        for dy in 0..rect.height {
            execute!(out, cursor::MoveTo(rect.x, rect.y + dy), Print("│"))?;
            execute!(out, cursor::MoveTo(rect.x + rect.width - 1, rect.y + dy), Print("│"))?;
        }
        execute!(out, cursor::MoveTo(rect.x, rect.y), Print("┌"))?;
        execute!(out, cursor::MoveTo(rect.x + rect.width - 1, rect.y), Print("┐"))?;
        execute!(out, cursor::MoveTo(rect.x, rect.y + rect.height - 1), Print("└"))?;
        execute!(out, cursor::MoveTo(rect.x + rect.width - 1, rect.y + rect.height - 1), Print("┘"))?;
        Ok(())
    }
}