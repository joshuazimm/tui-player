use crate::canvas::Canvas;
use crate::widget::{AbsRect, Widget};
use crate::state::{GlobalState, StateKey, StateValue};
use crossterm::event::{MouseButton, MouseEventKind};
use crossterm::style::Color;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

static PADDING: u16 = 1;
static INDENT_SIZE: usize = 2;

#[derive(Clone, Debug)]
pub enum FileSystemEntry {
    Directory {
        name: String,
        path: PathBuf,
        expanded: bool,
        children: Vec<FileSystemEntry>,
        depth: usize,
    },
    File {
        name: String,
        path: PathBuf,
        depth: usize,
    },
}

impl FileSystemEntry {
    fn name(&self) -> &str {
        match self {
            FileSystemEntry::Directory { name, .. } => name,
            FileSystemEntry::File { name, .. } => name,
        }
    }

    fn depth(&self) -> usize {
        match self {
            FileSystemEntry::Directory { depth, .. } => *depth,
            FileSystemEntry::File { depth, .. } => *depth,
        }
    }

    fn is_directory(&self) -> bool {
        matches!(self, FileSystemEntry::Directory { .. })
    }

    fn toggle_expanded(&mut self) {
        if let FileSystemEntry::Directory { expanded, .. } = self {
            *expanded = !*expanded;
        }
    }
}

pub struct FileList {
    align_center: bool,
    rainbow: bool,
    tick: usize,
    entries: Vec<FileSystemEntry>,
    flat_view: Vec<usize>,
    selected_index: u32,
    scroll_offset: AtomicUsize,
    needs_render: AtomicBool,
    state: GlobalState,
    modal_active: bool,
    // Content area on screen, from Layout via on_resize (for modal spawn coords)
    abs_x: u16,
    abs_y: u16,
    abs_w: u16,
}

fn rainbow_color(i: usize) -> Color {
    match i % 6 {
        0 => Color::Red, 1 => Color::Yellow, 2 => Color::Green,
        3 => Color::Cyan, 4 => Color::Blue, _ => Color::Magenta,
    }
}

fn scan_directory(path: &PathBuf, depth: usize) -> Vec<FileSystemEntry> {
    let mut entries = Vec::new();
    
    if let Ok(dir_entries) = fs::read_dir(path) {
        let mut dirs = Vec::new();
        let mut files = Vec::new();

        for entry in dir_entries.flatten() {
            if let Ok(file_type) = entry.file_type() {
                let entry_path = entry.path();
                let name = entry.file_name().to_string_lossy().into_owned();

                if file_type.is_dir() {
                    // Recursively scan subdirectories
                    let children = scan_directory(&entry_path, depth + 1);
                    // Only include directories that have audio files (directly or in subdirs)
                    if has_audio_content(&children) {
                        dirs.push(FileSystemEntry::Directory {
                            name,
                            path: entry_path,
                            expanded: false,
                            children,
                            depth,
                        });
                    }
                } else if file_type.is_file() && is_audio_file(&name) {
                    files.push(FileSystemEntry::File {
                        name,
                        path: entry_path,
                        depth,
                    });
                }
            }
        }

        // Sort directories and files separately, then combine
        dirs.sort_by(|a, b| a.name().cmp(b.name()));
        files.sort_by(|a, b| a.name().cmp(b.name()));
        
        entries.extend(dirs);
        entries.extend(files);
    }
    
    entries
}

fn has_audio_content(entries: &[FileSystemEntry]) -> bool {
    entries.iter().any(|entry| match entry {
        FileSystemEntry::File { .. } => true,
        FileSystemEntry::Directory { children, .. } => has_audio_content(children),
    })
}

fn is_audio_file(filename: &str) -> bool {
    let extensions = ["mp3", "flac", "ogg", "opus", "wav", "m4a", "aac", "wma"];
    if let Some(ext) = filename.split('.').last() {
        extensions.contains(&ext.to_lowercase().as_str())
    } else {
        false
    }
}

fn build_flat_view(entries: &[FileSystemEntry], flat_view: &mut Vec<usize>, base_index: &mut usize) {
    for entry in entries {
        let current_index = *base_index;
        flat_view.push(current_index);
        *base_index += 1;

        if let FileSystemEntry::Directory { expanded, children, .. } = entry {
            if *expanded {
                build_flat_view(children, flat_view, base_index);
            }
        }
    }
}

fn get_entry_by_flat_index<'a>(entries: &'a [FileSystemEntry], flat_index: usize, current_flat: &mut usize) -> Option<&'a FileSystemEntry> {
    for entry in entries {
        if *current_flat == flat_index {
            return Some(entry);
        }
        *current_flat += 1;

        if let FileSystemEntry::Directory { expanded, children, .. } = entry {
            if *expanded {
                if let Some(found) = get_entry_by_flat_index(children, flat_index, current_flat) {
                    return Some(found);
                }
            }
        }
    }
    None
}

fn get_entry_by_flat_index_mut<'a>(entries: &'a mut [FileSystemEntry], flat_index: usize, current_flat: &mut usize) -> Option<&'a mut FileSystemEntry> {
    for entry in entries {
        if *current_flat == flat_index {
            return Some(entry);
        }
        *current_flat += 1;

        if let FileSystemEntry::Directory { expanded, children, .. } = entry {
            if *expanded {
                if let Some(found) = get_entry_by_flat_index_mut(children, flat_index, current_flat) {
                    return Some(found);
                }
            }
        }
    }
    None
}

impl FileList {
    pub fn new(path: PathBuf, state: GlobalState) -> Self {
        let entries = scan_directory(&path, 0);
        let mut flat_view = Vec::new();
        let mut base_index = 0;
        build_flat_view(&entries, &mut flat_view, &mut base_index);

        Self {
            align_center: false,
            rainbow: false,
            tick: 0,
            entries,
            flat_view,
            scroll_offset: AtomicUsize::new(0),
            selected_index: 0,
            needs_render: AtomicBool::new(true),
            state,
            modal_active: false,
            abs_x: 0,
            abs_y: 0,
            abs_w: 0,
        }
    }

    fn rebuild_flat_view(&mut self) {
        self.flat_view.clear();
        let mut base_index = 0;
        build_flat_view(&self.entries, &mut self.flat_view, &mut base_index);
    }

    pub fn get_selected_file_path(&self) -> Option<PathBuf> {
        if self.flat_view.is_empty() {
            return None;
        }

        let flat_index = (self.selected_index as usize) % self.flat_view.len();
        let mut current_flat = 0;
        
        if let Some(entry) = get_entry_by_flat_index(&self.entries, flat_index, &mut current_flat) {
            match entry {
                FileSystemEntry::File { path, .. } => Some(path.clone()),
                FileSystemEntry::Directory { .. } => {
                    // Find first audio file in this directory
                    self.find_first_audio_in_entry(entry)
                }
            }
        } else {
            None
        }
    }

    fn find_first_audio_in_entry(&self, entry: &FileSystemEntry) -> Option<PathBuf> {
        match entry {
            FileSystemEntry::File { path, .. } => Some(path.clone()),
            FileSystemEntry::Directory { children, .. } => {
                for child in children {
                    if let Some(path) = self.find_first_audio_in_entry(child) {
                        return Some(path);
                    }
                }
                None
            }
        }
    }

    fn handle_navigation(&mut self, direction: i32) {
        if self.flat_view.is_empty() {
            return;
        }

        let item_count = self.flat_view.len() as u32;
        let new_index = if direction > 0 {
            (self.selected_index + 1) % item_count
        } else if self.selected_index == 0 {
            item_count - 1
        } else {
            self.selected_index - 1
        };

        self.selected_index = new_index;
        self.state.set_selected_file_index(new_index);
        self.needs_render.store(true, Ordering::Relaxed);
    }

    fn handle_toggle_expand(&mut self) {
        if self.flat_view.is_empty() {
            return;
        }

        let flat_index = (self.selected_index as usize) % self.flat_view.len();
        let mut current_flat = 0;
        
        if let Some(entry) = get_entry_by_flat_index_mut(&mut self.entries, flat_index, &mut current_flat) {
            if entry.is_directory() {
                entry.toggle_expanded();
                self.rebuild_flat_view();
                
                // Clamp selection to new view size
                if !self.flat_view.is_empty() {
                    self.selected_index = self.selected_index.min(self.flat_view.len() as u32 - 1);
                }
                
                self.needs_render.store(true, Ordering::Relaxed);
            }
        }
    }

    fn get_display_prefix(&self, entry: &FileSystemEntry) -> String {
        let indent = " ".repeat(entry.depth() * INDENT_SIZE);
        let icon = match entry {
            FileSystemEntry::Directory { expanded, .. } => {
                if *expanded { "▼ " } else { "▶ " }
            }
            FileSystemEntry::File { .. } => "  ",
        };
        format!("{}{}", indent, icon)
    }

    pub fn handle_mouse(&mut self, _local_col: u16, local_row: u16, abs_col: u16, abs_row: u16, kind: MouseEventKind) {
        match kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if local_row < PADDING || self.flat_view.is_empty() {
                    return;
                }
                let scroll = self.scroll_offset.load(Ordering::Relaxed);
                let flat_index = scroll + (local_row - PADDING) as usize;
                if flat_index >= self.flat_view.len() {
                    return;
                }

                self.selected_index = flat_index as u32;
                self.state.set_selected_file_index(flat_index as u32);

                let is_dir = {
                    let mut cur = 0;
                    get_entry_by_flat_index(&self.entries, flat_index, &mut cur)
                        .map(|e| e.is_directory())
                        .unwrap_or(false)
                };

                if is_dir {
                    self.handle_toggle_expand();
                } else if let Some(path) = self.get_selected_file_path() {
                    self.state.set_modal_spawn_pos(abs_col, abs_row);
                    self.state.set_pending_track_path(path);
                    self.state.set_modal_open(true);
                }
                self.needs_render.store(true, Ordering::Relaxed);
            }
            MouseEventKind::ScrollUp => {
                if self.selected_index > 0 {
                    self.selected_index -= 1;
                    self.state.set_selected_file_index(self.selected_index);
                    self.needs_render.store(true, Ordering::Relaxed);
                }
            }
            MouseEventKind::ScrollDown => {
                if !self.flat_view.is_empty()
                    && (self.selected_index as usize) < self.flat_view.len() - 1
                {
                    self.selected_index += 1;
                    self.state.set_selected_file_index(self.selected_index);
                    self.needs_render.store(true, Ordering::Relaxed);
                }
            }
            _ => {}
        }
    }
}

impl Widget for FileList {
    fn on_resize(&mut self, area: &AbsRect) {
        self.abs_x = area.x;
        self.abs_y = area.y;
        self.abs_w = area.width;
    }

    fn update(&mut self, state: &GlobalState) {
        self.tick = self.tick.wrapping_add(1);

        // Update selected index from state
        let current_index = state.selected_file_index();
        if !self.flat_view.is_empty() && self.selected_index != current_index {
            self.selected_index = current_index % self.flat_view.len() as u32;
            self.needs_render.store(true, Ordering::Relaxed);
        }
    }

    fn render(&self, canvas: &mut Canvas) -> std::io::Result<()> {
        let padding = PADDING as usize;

        // Calculate visible lines from the actual render area
        let visible_lines = canvas.height().saturating_sub(padding * 2);
        let total_items = self.flat_view.len();

        if total_items == 0 {
            let message = "No audio files found";
            canvas.put_str(padding, padding, message, Some(Color::DarkGrey), None);
            return Ok(());
        }

        let selected_index = (self.selected_index as usize).min(total_items - 1);

        // Smart scroll logic: only adjust scroll_offset if selected item goes OFF screen
        // Keep the current scroll_offset unless selection is outside the visible window
        let current_window_start = self.scroll_offset.load(Ordering::Relaxed);
        let scroll_offset = if visible_lines > 0 {
            let current_window_end = current_window_start + visible_lines;
            
            // Selection is ABOVE the visible window - scroll up to show it
            if selected_index < current_window_start {
                selected_index
            }
            // Selection is BELOW the visible window - scroll down to show it
            else if selected_index >= current_window_end {
                selected_index.saturating_sub(visible_lines - 1)
            }
            // Selection is WITHIN the visible window - don't scroll!
            else {
                current_window_start
            }
        } else {
            current_window_start
        };

        // Persist computed scroll_offset so subsequent renders/use see the window move
        // TODO: Consider making a mutable update call in the architecture to avoid
        // using atomics for persistent state
        self.scroll_offset.store(scroll_offset, Ordering::Relaxed);

        let end_index = (scroll_offset + visible_lines).min(total_items);
        let max_width = canvas.width().saturating_sub(padding * 2);

        // Render visible items
        for i in scroll_offset..end_index {
            let display_index = i - scroll_offset;
            let flat_index = i;
            let mut current_flat = 0;

            if let Some(entry) = get_entry_by_flat_index(&self.entries, flat_index, &mut current_flat) {
                let prefix = self.get_display_prefix(entry);
                let name = entry.name();
                let display_line = format!("{}{}", prefix, name);

                let x_start = if self.align_center {
                    canvas.width().saturating_sub(display_line.len()) / 2
                } else {
                    padding
                };
                let y = padding + display_index;

                let truncated_line: String = if display_line.len() > max_width && max_width > 0 {
                    let mut truncated: String = display_line.chars().take(max_width.saturating_sub(1)).collect();
                    truncated.push('…');
                    truncated
                } else {
                    display_line.clone()
                };

                let is_selected = flat_index == selected_index;
                let (default_fg, default_bg) = if is_selected {
                    (Color::Black, Some(Color::White))
                } else if entry.is_directory() {
                    (Color::Cyan, None)
                } else {
                    (Color::White, None)
                };

                if self.rainbow {
                    for (j, ch) in truncated_line.chars().enumerate() {
                        let fg = if is_selected {
                            Color::Black
                        } else {
                            rainbow_color(j + self.tick)
                        };
                        canvas.set(x_start + j, y, ch, Some(fg), default_bg);
                    }
                } else {
                    canvas.put_str(x_start, y, &truncated_line, Some(default_fg), default_bg);
                }
            }
        }

        // Show scroll indicators
        if total_items > visible_lines {
            let scroll_x = canvas.width().saturating_sub(1);

            if scroll_offset > 0 {
                canvas.set(scroll_x, padding, '↑', Some(Color::Yellow), None);
            }

            if scroll_offset + visible_lines < total_items {
                let bottom_y = canvas.height().saturating_sub(padding);
                canvas.set(scroll_x, bottom_y, '↓', Some(Color::Yellow), None);
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
            StateKey::ModalOpen => {
                if let Some(open) = value.as_bool() {
                    self.modal_active = open;
                }
            }
            StateKey::NavigationUp => {
                if let Some(true) = value.as_bool() {
                    if !self.modal_active {
                        self.handle_navigation(-1);
                        self.state.set(StateKey::NavigationUp, StateValue::Bool(false));
                    }
                }
            }
            StateKey::NavigationDown => {
                if let Some(true) = value.as_bool() {
                    if !self.modal_active {
                        self.handle_navigation(1);
                        self.state.set(StateKey::NavigationDown, StateValue::Bool(false));
                    }
                }
            }
            StateKey::TrackSelected => {
                if let Some(true) = value.as_bool() {
                    if !self.modal_active && !self.flat_view.is_empty() {
                        let flat_index = (self.selected_index as usize) % self.flat_view.len();
                        let mut current_flat = 0;

                        if let Some(entry) = get_entry_by_flat_index(&self.entries, flat_index, &mut current_flat) {
                            if entry.is_directory() {
                                self.handle_toggle_expand();
                            } else if let Some(path) = self.get_selected_file_path() {
                                let scroll = self.scroll_offset.load(Ordering::Relaxed);
                                let visual_row = flat_index.saturating_sub(scroll);
                                let spawn_x = self.abs_x + self.abs_w;
                                let spawn_y = self.abs_y + PADDING as u16 + visual_row as u16;
                                self.state.set_modal_spawn_pos(spawn_x, spawn_y);
                                self.state.set_pending_track_path(path);
                                self.state.set_modal_open(true);
                            }
                        }
                        self.state.set(StateKey::TrackSelected, StateValue::Bool(false));
                    }
                }
            }
            StateKey::SelectedFileIndex => {
                if let Some(new_index) = value.as_u32() {
                    if !self.flat_view.is_empty() {
                        let clamped_index = new_index % self.flat_view.len() as u32;
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
            StateKey::TrackSelected,
            StateKey::ModalOpen,
        ]
    }
}