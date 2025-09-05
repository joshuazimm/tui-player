use crate::widget::{AbsRect, Widget, Cell};
use crossterm::style::Color;
use std::fs;
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
};

static PADDING: u16 = 1;

#[derive(Debug, Clone)]
pub struct FileList {
    pub path: PathBuf,
    pub align_center: bool,
    pub rainbow: bool,
    pub tick: usize,
    pub files: Vec<String>,
    pub selected: Arc<AtomicU32>,
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
    pub fn new(path: PathBuf, selected: Arc<AtomicU32>) -> FileList {
        let files = get_files(path.clone());
        Self {
            path: path.clone(),
            align_center: false,
            rainbow: false,
            tick: 1,
            files,
            selected,
        }
    }
    pub fn get_file_from_index(&self, index: u32) -> Option<&String> {
        let index = index as usize % self.files.len();
        self.files.get(index as usize)
    }
    pub fn get_path(&self) -> PathBuf {
        self.path.clone()
    }
}

impl Widget for FileList {
    fn update(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    /*fn render(&self, area: &Rect, buf: &mut String) -> std::io::Result<()> {
        let (abs_x, abs_y, abs_width, abs_height) = area.to_absolute();

        let selected_index = self.selected.load(Ordering::Relaxed) as usize % self.files.len();
        let end_index = (selected_index + abs_height as usize).min(self.files.len());

        // Only render as many lines as fit in the area
        for (i, line) in self.files[selected_index..end_index].iter().enumerate() {
            if i as u16 > abs_height - PADDING {
                break;
            }

            let x_start = if self.align_center {
                abs_x + (abs_width.saturating_sub(line.len() as u16) / 2)
            } else {
                abs_x + PADDING
            };
            let y = (abs_y + PADDING) + i as u16;

            // set up highlight colors
            let (default_fg, bg) = if i == selected_index {
                (Color::Black, Some(Color::White))
            } else {
                (Color::White, None)
            };

            if self.rainbow {
                for (j, ch) in line.chars().enumerate() {
                    let x = x_start + j as u16;
                    let fg = if i == selected_index {
                        Color::Black // override
                    } else {
                        rainbow_color(j + self.tick)
                    };
                    buf.push_str(&format!(
                        "{}{}{}{}{}",
                        cursor::MoveTo(x, y),
                        SetForegroundColor(fg),
                        bg.map(|c| SetBackgroundColor(c).to_string())
                            .unwrap_or_default(),
                        ch,
                        ResetColor
                    ));
                }
            } else {
                buf.push_str(&format!(
                    "{}{}{}{}{}",
                    cursor::MoveTo(x_start, y),
                    SetForegroundColor(default_fg),
                    bg.map(|c| SetBackgroundColor(c).to_string())
                        .unwrap_or_default(),
                    line,
                    ResetColor
                ));
            }
        }

        Ok(())
    }*/
    fn render(&self, area: &AbsRect, buf: &mut [Vec<Cell>]) -> std::io::Result<()> {
        let (abs_x, abs_y, abs_width, abs_height) = (area.x, area.y, area.width, area.height);
        let visible_lines = (abs_height.saturating_sub(PADDING)) as usize;
        let total_files = self.files.len();

        if total_files == 0 {
            return Ok(());
        }

        let selected_index = self.selected.load(Ordering::Relaxed) as usize % total_files;

        let scroll_offset = if selected_index >= visible_lines {
            selected_index + 1 - visible_lines
        } else {
            0
        };

        let end_index = (scroll_offset + visible_lines).min(total_files);
        let max_width = abs_width.saturating_sub(PADDING);

        for (i, line) in self.files[scroll_offset..end_index].iter().enumerate() {
            let line_index = scroll_offset + i;

            let x_start = if self.align_center {
                abs_x as usize
                    + ((abs_width as usize).saturating_sub(line.len()) / 2)
            } else {
                abs_x as usize + PADDING as usize
            };
            let y = abs_y as usize + PADDING as usize + i;

            // prevent wrap-around
            let display_line: String = if line.len() as u16 > max_width {
                line.chars().take(max_width as usize).collect()
            } else {
                line.clone()
            };

            // highlight current selection
            let (default_fg, default_bg) = if line_index == selected_index {
                (Color::Black, Some(Color::White))
            } else {
                (Color::White, None)
            };

            if y < buf.len() {
                if self.rainbow {
                    for (j, ch) in display_line.chars().enumerate() {
                        let x = x_start + j;
                        if x < buf[0].len() {
                            let fg = if line_index == selected_index {
                                Color::Black
                            } else {
                                rainbow_color(j + self.tick)
                            };
                            buf[y][x].ch = ch;
                            buf[y][x].fg = Some(fg);
                        }
                    }
                } else {
                    for (j, ch) in display_line.chars().enumerate() {
                        let x = x_start + j;
                        if x < buf[0].len() {
                            buf[y][x].ch = ch;
                            buf[y][x].fg = Some(default_fg);
                            buf[y][x].bg = default_bg;
                        }
                    }
                }
            }
        }

        Ok(())
    }
}
