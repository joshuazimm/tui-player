use crate::widget::{Rect, Widget};
use crossterm::{
    cursor,
    style::{Color, ResetColor, SetForegroundColor},
};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct FileList {
    pub path: PathBuf,
    pub align_center: bool,
    pub rainbow: bool,
    pub tick: usize,
    pub files: Vec<String>,
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

fn get_files(path: PathBuf) -> Vec<String> {
    let mut files = Vec::new();
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            if let Ok(file_type) = entry.file_type() {
                // Only include files, skip directories
                if file_type.is_file() {
                    files.push(entry.file_name().to_string_lossy().into_owned());
                }
            }
        }
    }
    files
}

impl FileList {
    pub fn new(path: PathBuf) -> FileList {
        Self {
            path: path.clone(),
            align_center: true,
            rainbow: true,
            tick: 1,
            files: get_files(path),
        }
    }
}

impl Widget for FileList {
    fn update(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    fn render(&self, area: &Rect, buf: &mut String) -> std::io::Result<()> {
        let (abs_x, abs_y, abs_width, abs_height) = area.to_absolute();

        // Only render as many lines as fit in the area
        for (i, line) in self.files.iter().enumerate() {
            if i as u16 >= abs_height {
                break; // prevent overflow
            }

            let x_start = if self.align_center {
                abs_x + (abs_width.saturating_sub(line.len() as u16) / 2)
            } else {
                abs_x
            };
            let y = abs_y + i as u16;

            if self.rainbow {
                for (j, ch) in line.chars().enumerate() {
                    let x = x_start + j as u16;
                    let color = rainbow_color(j + self.tick);
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
                    line,
                    ResetColor
                ));
            }
        }

        Ok(())
    }
}
