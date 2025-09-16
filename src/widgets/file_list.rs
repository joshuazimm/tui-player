use crate::widget::{AbsRect, Widget, Cell};
use crate::state::{GlobalState, StateKey, StateValue};
use crossterm::style::Color;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

static PADDING: u16 = 1;

pub struct FileList {
    path: PathBuf,
    align_center: bool,
    rainbow: bool,
    tick: usize,
    files: Vec<String>,
    selected_index: u32,
    needs_render: AtomicBool,
    state: GlobalState, // Keep a reference to state for sending track changes
}

fn rainbow_color(i: usize) -> Color {
    match i % 6 {
        0 => Color::Red, 1 => Color::Yellow, 2 => Color::Green,
        3 => Color::Cyan, 4 => Color::Blue, _ => Color::Magenta,
    }
}

fn get_files(path: &PathBuf) -> Vec<String> {
    let mut files = Vec::new();
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            if let Ok(file_type) = entry.file_type() {
                if file_type.is_file() {
                    let filename = entry.file_name().to_string_lossy().into_owned();
                    if is_audio_file(&filename) {
                        files.push(filename);
                    }
                }
            }
        }
    }
    files.sort();
    files
}

fn is_audio_file(filename: &str) -> bool {
    let extensions = ["mp3", "flac", "ogg", "opus", "wav", "m4a"];
    if let Some(ext) = filename.split('.').last() {
        extensions.contains(&ext.to_lowercase().as_str())
    } else {
        false
    }
}

impl FileList {
    pub fn new(path: PathBuf, state: GlobalState) -> Self {
        let files = get_files(&path);
        Self {
            path,
            align_center: false,
            rainbow: false,
            tick: 0,
            files,
            selected_index: 0,
            needs_render: AtomicBool::new(true),
            state,
        }
    }

    pub fn get_selected_file_path(&self) -> Option<PathBuf> {
        if self.files.is_empty() {
            return None;
        }
        let index = self.selected_index as usize % self.files.len();
        self.files.get(index).map(|filename| {
            let mut path = self.path.clone();
            path.push(filename);
            path
        })
    }

    fn handle_navigation(&mut self, direction: i32) {
        if self.files.is_empty() {
            return;
        }

        let file_count = self.files.len() as u32;
        let new_index = if direction > 0 {
            (self.selected_index + 1) % file_count
        } else if self.selected_index == 0 {
            file_count - 1
        } else {
            self.selected_index - 1
        };

        self.selected_index = new_index;
        self.state.set_selected_file_index(new_index);
        self.needs_render.store(true, Ordering::Relaxed);

        // Update current track when selection changes
        if let Some(path) = self.get_selected_file_path() {
            let title = path.file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string());
            self.state.set_current_track(path, title, None, None);
        }
    }

    fn handle_track_selection(&self) -> Option<PathBuf> {
        if let Some(path) = self.get_selected_file_path() {
            let title = path.file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string());
            self.state.set_current_track(path.clone(), title, None, None);
            self.state.set_playing(true); // Start playing the selected track

            self.state.set_new_track_requested(true);

            Some(path)
        } else {
            None
        }
    }
}

impl Widget for FileList {
    fn update(&mut self, state: &GlobalState) {
        self.tick = self.tick.wrapping_add(1);
        
        // Update selected index from state
        let current_index = state.selected_file_index();
        if !self.files.is_empty() && self.selected_index != current_index {
            self.selected_index = current_index % self.files.len() as u32;
            self.needs_render.store(true, Ordering::Relaxed);
        }
    }

    fn render(&self, area: &AbsRect, buf: &mut [Vec<Cell>]) -> std::io::Result<()> {
        let visible_lines = (area.height.saturating_sub(PADDING)) as usize;
        let total_files = self.files.len();

        if total_files == 0 {
            let message = "No audio files found";
            let x_start = area.x as usize + PADDING as usize;
            let y = area.y as usize + PADDING as usize;
            
            if y < buf.len() {
                for (i, ch) in message.chars().enumerate() {
                    let x = x_start + i;
                    if x < buf[0].len() {
                        buf[y][x].ch = ch;
                        buf[y][x].fg = Some(Color::DarkGrey);
                    }
                }
            }
            return Ok(());
        }

        let selected_index = (self.selected_index as usize).min(total_files - 1);

        // Calculate scroll offset
        let scroll_offset = if selected_index >= visible_lines {
            selected_index + 1 - visible_lines
        } else {
            0
        };

        let end_index = (scroll_offset + visible_lines).min(total_files);
        let max_width = area.width.saturating_sub(PADDING * 2);

        // Render visible files
        for (i, line) in self.files[scroll_offset..end_index].iter().enumerate() {
            let line_index = scroll_offset + i;

            let x_start = if self.align_center {
                area.x as usize + ((area.width as usize).saturating_sub(line.len()) / 2)
            } else {
                area.x as usize + PADDING as usize
            };
            let y = area.y as usize + PADDING as usize + i;

            // Handle text truncation
            let display_line: String = if line.len() as u16 > max_width && max_width > 0 {
                let mut truncated: String = line.chars().take(max_width.saturating_sub(1) as usize).collect();
                truncated.push('…');
                truncated
            } else {
                line.clone()
            };

            // Highlight current selection
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
                            buf[y][x].bg = default_bg;
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

        // Show scroll indicators
        if total_files > visible_lines {
            let scroll_x = (area.x + area.width).saturating_sub(1) as usize;
            
            if scroll_offset > 0 {
                let scroll_y = area.y as usize + PADDING as usize;
                if scroll_y < buf.len() && scroll_x < buf[0].len() {
                    buf[scroll_y][scroll_x].ch = '↑';
                    buf[scroll_y][scroll_x].fg = Some(Color::Yellow);
                }
            }
            
            if scroll_offset + visible_lines < total_files {
                let bottom_y = (area.y + area.height).saturating_sub(1) as usize;
                if bottom_y < buf.len() && scroll_x < buf[0].len() {
                    buf[bottom_y][scroll_x].ch = '↓';
                    buf[bottom_y][scroll_x].fg = Some(Color::Yellow);
                }
            }
        }

        Ok(())
    }

    fn needs_render(&self) -> bool {
        self.needs_render.load(Ordering::Relaxed)
    }

    fn mark_rendered(&mut self) {
        self.needs_render.store(false, Ordering::Relaxed);
    }

    fn handle_state_change(&mut self, key: &StateKey, value: &StateValue) {
        match key {
            StateKey::NavigationUp => {
                if let Some(true) = value.as_bool() {
                    self.handle_navigation(-1);
                    // Reset the navigation flag
                    self.state.set(StateKey::NavigationUp, StateValue::Bool(false));
                }
            }
            StateKey::NavigationDown => {
                if let Some(true) = value.as_bool() {
                    self.handle_navigation(1);
                    // Reset the navigation flag
                    self.state.set(StateKey::NavigationDown, StateValue::Bool(false));
                }
            }
            StateKey::TrackSelected => {
                if let Some(true) = value.as_bool() {
                    if let Some(_path) = self.handle_track_selection() {
                        // Track selection handled, path returned for potential use
                        // The audio thread should pick up the new track from state
                    }
                    // Reset the selection flag
                    self.state.set(StateKey::TrackSelected, StateValue::Bool(false));
                }
            }
            StateKey::SelectedFileIndex => {
                if let Some(new_index) = value.as_u32() {
                    if !self.files.is_empty() {
                        let clamped_index = new_index % self.files.len() as u32;
                        if self.selected_index != clamped_index {
                            self.selected_index = clamped_index;
                            self.needs_render.store(true, Ordering::Relaxed);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn subscribed_keys(&self) -> Vec<StateKey> {
        vec![
            StateKey::SelectedFileIndex,
            StateKey::NavigationUp,
            StateKey::NavigationDown,
            StateKey::TrackSelected
        ]
    }
}