use std::io::{Write};

use enum_dispatch::enum_dispatch;
use crate::widgets::WidgetKind;
use crossterm::{terminal::size, cursor::MoveTo, style::{SetForegroundColor, SetBackgroundColor, ResetColor, Color}};

#[derive(Clone, Copy)]
pub struct Cell {
    pub ch: char,
    pub fg: Option<Color>,
    pub bg: Option<Color>
}

impl Default for Cell {
    fn default() -> Self {
        Self { ch: ' ', fg: None, bg: None }
    }
}

fn make_screen_buffer(width: u16, height: u16) -> Vec<Vec<Cell>> {
    vec![vec![Cell::default(); width as usize]; height as usize]
}

fn draw_horizontal(buf: &mut [Vec<Cell>], y: usize, x_start: usize, x_end: usize, ch: char) {
    for x in x_start..x_end {
        buf[y][x].ch = ch;
    }
}

fn draw_vertical(buf: &mut [Vec<Cell>], x: usize, y_start: usize, y_end: usize, ch: char) {
    for y in y_start..y_end {
        buf[y][x].ch = ch;
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn to_absolute(&self, term_width: u16, term_height: u16) -> (u16, u16, u16, u16) {
        let x = (self.x * term_width as f32) as u16;
        let y = (self.y * term_height as f32) as u16;
        let width = (self.width * term_width as f32) as u16;
        let height = (self.height * term_height as f32) as u16;
        (x, y, width, height)
    }
}

pub struct Section {
    pub rect: Rect,
    pub border: bool,
    pub widget: WidgetKind
}

impl Section {
    pub fn new(rect: Rect, border: bool, widget: WidgetKind) -> Self {
        Self {
            rect,
            border,
            widget
        }
    }
}

#[enum_dispatch]
pub trait Widget {
    fn update(&mut self);
    fn render(&self, area: &Rect, buf: &mut [Vec<Cell>], term_width: u16, term_height: u16) -> std::io::Result<()>;
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
        // Use one terminal size per render call
        let (term_w, term_h) = size().unwrap();
        let mut screen = make_screen_buffer(term_w, term_h);

        for section in &self.sections {
            if section.border {
                Self::draw_border(&mut screen, &section.rect, term_w, term_h);
            }
        }

        for section in &mut self.sections {
            section.widget.render(&section.rect, &mut screen, term_w, term_h)?;
        }

        Self::flush_buffer(out, &screen)
    }

    fn draw_border(buf: &mut [Vec<Cell>], rect: &crate::widget::Rect, term_width: u16, term_height: u16) {
        let (abs_x, abs_y, abs_w, abs_h) = rect.to_absolute(term_width, term_height);

        if abs_w < 2 || abs_h < 2 { return; }

        let left = abs_x as usize;
        let right = (abs_x + abs_w - 1) as usize;
        let top = abs_y as usize;
        let bottom = (abs_y + abs_h - 1) as usize;

        // Corners
        buf[top][left].ch = '┌';
        buf[top][right].ch = '┐';
        buf[bottom][left].ch = '└';
        buf[bottom][right].ch = '┘';

        // Horizontal edges
        draw_horizontal(buf, top, left + 1, right, '─');
        draw_horizontal(buf, bottom, left + 1, right, '─');

        // Vertical edges
        draw_vertical(buf, left, top + 1, bottom, '│');
        draw_vertical(buf, right, top + 1, bottom, '│');
    }

    // Construct a Chronological Screen string to optimize speed
    fn flush_buffer(out: &mut std::io::Stdout, buf: &[Vec<Cell>]) -> std::io::Result<()> {
        let mut output = String::new();

        for (row_idx, row) in buf.iter().enumerate() {
            output.push_str(&format!("{}", MoveTo(0, row_idx as u16)));

            for cell in row {
                if let Some(fg) = cell.fg {
                    output.push_str(&format!("{}", SetForegroundColor(fg)));
                }
                if let Some(bg) = cell.bg {
                    output.push_str(&format!("{}", SetBackgroundColor(bg)));
                }

                output.push(cell.ch);

                if cell.fg.is_some() || cell.bg.is_some() {
                    output.push_str(&format!("{}", ResetColor));
                }
            }
        }

        write!(out, "{}", output)?;
        out.flush()
    }

}