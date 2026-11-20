// A widget's view of the screen buffer. It only reaches the cells inside the
// widget's own area, addressed from (0, 0) at the area's top-left corner.
// Writes outside the area are dropped.

use crossterm::style::Color;
use crate::widget::{AbsRect, Cell};

pub struct Canvas<'a> {
    screen: &'a mut [Vec<Cell>],
    area: AbsRect,
}

impl<'a> Canvas<'a> {
    /// A canvas over `area` of `screen`. The area is clipped to the screen,
    /// so a canvas can never index outside the buffer.
    pub fn new(screen: &'a mut [Vec<Cell>], mut area: AbsRect) -> Self {
        let screen_h = screen.len() as u16;
        let screen_w = screen.first().map_or(0, |row| row.len()) as u16;
        area.width = area.width.min(screen_w.saturating_sub(area.x));
        area.height = area.height.min(screen_h.saturating_sub(area.y));

        Self { screen, area }
    }

    pub fn width(&self) -> usize {
        self.area.width as usize
    }

    pub fn height(&self) -> usize {
        self.area.height as usize
    }

    /// Sets one cell; a cell outside the canvas is ignored.
    pub fn set(&mut self, x: usize, y: usize, ch: char, fg: Option<Color>, bg: Option<Color>) {
        if x >= self.width() || y >= self.height() {
            return;
        }
        let row = self.area.y as usize + y;
        let col = self.area.x as usize + x;
        self.screen[row][col] = Cell { ch, fg, bg };
    }

    /// Writes text left to right from (x, y), one cell per char, clipped to the canvas.
    pub fn put_str(&mut self, x: usize, y: usize, text: &str, fg: Option<Color>, bg: Option<Color>) {
        for (i, ch) in text.chars().enumerate() {
            self.set(x + i, y, ch, fg, bg);
        }
    }

    /// One row of the canvas as a slice exactly `width()` cells wide, for
    /// widgets that fill whole rows. A row outside the canvas is an empty slice.
    pub fn row_mut(&mut self, y: usize) -> &mut [Cell] {
        if y >= self.height() {
            return &mut [];
        }
        let start = self.area.x as usize;
        let end = start + self.width();
        &mut self.screen[self.area.y as usize + y][start..end]
    }
}
