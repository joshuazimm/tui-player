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
        let (abs_x, abs_y, abs_width, abs_height) = (area.x, area.y, area.width, area.height);
        let x_start = if self.align_center {
            abs_x as usize + (abs_width as usize).saturating_sub(self.text.len()) / 2
        } else {
            abs_x as usize
        };
        let y = abs_y as usize + abs_height as usize / 2;

        if self.rainbow {
            for (i, ch) in self.text.chars().enumerate() {
                let x = x_start + i;
                if y < buf.len() && x < buf[0].len() {
                    buf[y][x].ch = ch;
                    buf[y][x].fg = Some(rainbow_color(i + self.tick));
                }
            }
        } else {
            for (i, ch) in self.text.chars().enumerate() {
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
