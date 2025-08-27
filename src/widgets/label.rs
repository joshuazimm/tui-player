// TODO: Since this will be a commonly repeated filetype
//       consider making autogen or at least templating
//       common parts of the code
use crossterm::{style::{SetForegroundColor, ResetColor, Color}, cursor};
use crate::widget::{Rect, Widget};

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

    fn render(&self, area: &Rect, buf: &mut String) -> std::io::Result<()> {
        let x_start = if self.align_center {
            area.x + (area.width.saturating_sub(self.text.len() as u16) / 2)
        } else {
            area.x
        };
        let y = area.y + (area.height / 2);

        if self.rainbow {
            for (i, ch) in self.text.chars().enumerate() {
                let x = x_start + i as u16;
                let color = rainbow_color(i + self.tick);
                buf.push_str(&format!(
                    "{}{}{}{}",
                    cursor::MoveTo(x, y),
                    SetForegroundColor(color),
                    ch,
                    ResetColor
                ));
            }
        } else {
            buf.push_str(&format!(
                "{}{}{}{}",
                cursor::MoveTo(x_start, y),
                SetForegroundColor(Color::White),
                self.text,
                ResetColor
            ));
        }
        Ok(())
    }
}
