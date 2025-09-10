// TODO: Since this will be a commonly repeated filetype
//       consider making autogen or at least templating
//       common parts of the code
use crossterm::style::Color;
use crate::widget::{AbsRect, Widget, Cell};

#[derive(Debug, Clone)]
pub struct Label {
    pub text: String,
    pub align_center: bool,
    pub rainbow: bool,
    pub tick: usize,
}

impl Label {
    pub fn set_text(&mut self, new_text: impl Into<String>) {
        self.text = new_text.into();
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
    fn update(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    fn render(&self, area: &AbsRect, buf: &mut [Vec<Cell>]) -> std::io::Result<()> {
        let (abs_x, abs_y, abs_width, abs_height) = (area.x, area.y, area.width.saturating_add(1), area.height.saturating_add(1));

        // truncate + ellipsis if text does not fit
        let mut display_text = self.text.clone();
        if display_text.len() > abs_width as usize {
            if abs_width > 1 {
                display_text.truncate(abs_width as usize - 1);
                display_text.push('…');
            } else {
                display_text = "…".to_string();
            }
        }

        // Horizontal positioning
        let x_start = if self.align_center {
            abs_x as usize + (abs_width as usize).saturating_sub(display_text.len()) / 2
        } else {
            abs_x as usize
        };
        
        // Vertical positioning (always center)
        let y = abs_y as usize + abs_height as usize / 2;

        // Render text
        if self.rainbow {
            for (i, ch) in display_text.chars().enumerate() {
                let x = x_start + i;
                if y < buf.len() && x < buf[0].len() {
                    buf[y][x].ch = ch;
                    buf[y][x].fg = Some(rainbow_color(i + self.tick));
                }
            }
        } else {
            for (i, ch) in display_text.chars().enumerate() {
                let x = x_start + i;
                if y < buf.len() && x < buf[0].len() {
                    buf[y][x].ch = ch;
                    buf[y][x].fg = Some(Color::White);
                }
            }
        }

        Ok(())
    }
}
