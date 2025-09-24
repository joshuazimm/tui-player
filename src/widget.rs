use std::io::{Write};
use enum_dispatch::enum_dispatch;
use crate::widgets::WidgetKind;
use crate::state::{GlobalState, StateKey, StateValue};
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

#[derive(Debug, Clone, Copy)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

pub struct AbsRect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl Default for AbsRect {
    fn default() -> Self {
        Self { x: 0, y: 0, width: 0, height: 0 }
    }
}

impl Rect {
    pub fn to_absolute(&self, term_width: u16, term_height: u16) -> (u16, u16, u16, u16) {
        let max_x_idx = term_width.saturating_sub(1) as f32;
        let max_y_idx = term_height.saturating_sub(1) as f32;

        let abs_x = (self.x * max_x_idx).round().clamp(0.0, max_x_idx) as u16;
        let abs_y = (self.y * max_y_idx).round().clamp(0.0, max_y_idx) as u16;
        
        let abs_w = (self.width * term_width as f32).round() as u16;
        let abs_h = (self.height * term_height as f32).round() as u16;
        
        let abs_w = abs_w.min(term_width.saturating_sub(abs_x));
        let abs_h = abs_h.min(term_height.saturating_sub(abs_y));

        (abs_x, abs_y, abs_w, abs_h)
    }
}

#[enum_dispatch]
pub trait Widget {
    fn update(&mut self, state: &GlobalState);
    fn render(&self, area: &AbsRect, buf: &mut [Vec<Cell>]) -> std::io::Result<()>;
    fn needs_render(&self) -> bool;
    fn mark_rendered(&mut self);
    fn handle_state_change(&mut self, key: &StateKey, value: &StateValue);
    fn subscribed_keys(&self) -> Vec<StateKey>;
}

pub struct Section {
    pub rect: Rect,
    pub abs_rect: AbsRect,
    pub widget: WidgetKind,
    pub needs_render: bool
}

impl Section {
    pub fn new(rect: Rect, widget: WidgetKind) -> Self {
        Self {
            rect,
            abs_rect: AbsRect::default(),
            widget,
            needs_render: true
        }
    }
}

pub struct Layout {
    pub sections: Vec<Section>,
    last_terminal_size: (u16, u16),
    last_global_change_id: u64,
    screen: Vec<Vec<Cell>>, // cached last frame
    force_full_render: bool, // Flag to force complete re-render
}

impl Layout {
    pub fn new(sections: Vec<Section>) -> Self {
        let (width, height) = size().unwrap_or((80, 24));
        Self {
            sections,
            last_terminal_size: (width, height),
            last_global_change_id: 0,
            screen: make_screen_buffer(width, height),
            force_full_render: true, // Start with a full render
        }
    }

    pub fn init_all(&mut self, _state: &GlobalState) {
    }

    pub fn update_all(&mut self, state: &GlobalState) -> std::io::Result<()> {
        // Check if terminal size changed
        if let Ok((width, height)) = size() {
            if (width, height) != self.last_terminal_size {
                self.last_terminal_size = (width, height);
                state.update_terminal_size();
                self.screen = make_screen_buffer(width, height);
                self.force_full_render = true;
                for section in &mut self.sections {
                    section.needs_render = true;
                }
            }
        }

        // Get current change ID
        let current_change_id = state.change_id();
        let state_changed = current_change_id != self.last_global_change_id;
        
        if state_changed {
            // Get all changes and distribute them to interested widgets
            let changes = state.take_changes();
            
            for change_key in &changes {
                if let Some(value) = state.get(change_key) {
                    // Send changes to widgets that care about this key
                    for section in &mut self.sections {
                        if section.widget.subscribed_keys().contains(change_key) {
                            section.widget.handle_state_change(change_key, &value);
                        }
                    }
                }
            }
            
            self.last_global_change_id = current_change_id;
        }

        // Update all widgets
        for section in &mut self.sections {
            section.widget.update(state);
            
            // Update absolute rect based on current terminal size
            let (term_w, term_h) = self.last_terminal_size;
            let (abs_x, abs_y, abs_w, abs_h) = section.rect.to_absolute(term_w, term_h);
            section.abs_rect = AbsRect {
                x: abs_x.saturating_add(1),
                y: abs_y.saturating_add(1),
                width: abs_w.saturating_sub(2).max(0),
                height: abs_h.saturating_sub(2).max(0),
            };
            
            // Check if widget needs render
            if section.widget.needs_render() {
                section.needs_render = true;
                section.widget.mark_rendered();
            }
        }
        Ok(())
    }

    pub fn render_all(&mut self, out: &mut std::io::Stdout, force_render: bool) -> std::io::Result<()> {
        let (term_w, term_h) = size().unwrap();
        
        // Check for terminal resize - this forces a complete re-render
        if (term_w, term_h) != self.last_terminal_size {
            self.last_terminal_size = (term_w, term_h);
            return self.force_render_all(out);
        }

        // Check if any rendering is needed
        let any_needs_render = force_render || self.force_full_render || 
                             self.sections.iter().any(|s| s.needs_render);
        
        if !any_needs_render {
            return Ok(());
        }

        // Use force render for complete re-renders
        if self.force_full_render || force_render {
            return self.force_render_all(out);
        }

        // Partial render: only update changed sections
        let areas_to_clear: Vec<Rect> = self.sections.iter()
            .filter(|s| s.needs_render)
            .map(|s| s.rect)
            .collect();

        if areas_to_clear.is_empty() {
            return Ok(());
        }

        // Clear areas that need updating
        for area_rect in &areas_to_clear {
            self.clear_rect_area(area_rect, term_w, term_h);
        }

        // Redraw borders (they may have been cleared)
        self.draw_unified_borders(term_w, term_h);

        // Render only the widgets that need updating
        for section in self.sections.iter_mut() {
            if section.needs_render {
                section.widget.render(&section.abs_rect, &mut self.screen)?;
                section.needs_render = false;
            }
        }

        Self::flush_buffer(out, &self.screen)
    }

    pub fn force_render_all(&mut self, out: &mut std::io::Stdout) -> std::io::Result<()> {
        let (term_w, term_h) = size().unwrap();
        
        // Only recreate screen buffer if size actually changed
        if (term_w, term_h) != (self.screen[0].len() as u16, self.screen.len() as u16) {
            self.screen = make_screen_buffer(term_w, term_h);
        } else {
            // Just clear the existing buffer
            for row in &mut self.screen {
                for cell in row {
                    *cell = Cell::default();
                }
            }
        }

        // Update terminal size and mark all widgets for render
        self.last_terminal_size = (term_w, term_h);
        for section in &mut self.sections {
            section.needs_render = true;
        }

        // Draw borders first
        self.draw_unified_borders(term_w, term_h);

        // Render all widgets
        for section in self.sections.iter_mut() {
            section.widget.render(&section.abs_rect, &mut self.screen)?;
            section.needs_render = false;
        }

        self.force_full_render = false;
        Self::flush_buffer(out, &self.screen)
    }

    fn clear_rect_area(&mut self, rect: &Rect, term_width: u16, term_height: u16) {
        let (abs_x, abs_y, abs_w, abs_h) = rect.to_absolute(term_width, term_height);

        for y in abs_y..(abs_y + abs_h).min(self.screen.len() as u16) {
            for x in abs_x..(abs_x + abs_w).min(self.screen[0].len() as u16) {
                self.screen[y as usize][x as usize] = Cell::default();
            }
        }
    }

    fn draw_unified_borders(&mut self, term_width: u16, term_height: u16) {
        // use the internal cached buffer directly to avoid overlapping borrows
        let buf = &mut self.screen;

        let mut xs_f = Vec::new();
        let mut ys_f = Vec::new();

        for section in &self.sections {
            xs_f.push(section.rect.x);
            xs_f.push(section.rect.x + section.rect.width);
            ys_f.push(section.rect.y);
            ys_f.push(section.rect.y + section.rect.height);
        }

        let epsilon = 0.02_f32;
        let xs = unify_edges(&xs_f, term_width, epsilon);
        let ys = unify_edges(&ys_f, term_height, epsilon);

        let mut horizontal_edges: Vec<(u16, u16, u16)> = Vec::new();
        let mut vertical_edges: Vec<(u16, u16, u16)> = Vec::new();
        let mut intersections = std::collections::HashSet::new();

        let max_x = term_width.saturating_sub(1);
        let max_y = term_height.saturating_sub(1);

        for section in &mut self.sections {
            let (abs_x, abs_y, abs_w, abs_h) = section.rect.to_absolute(term_width, term_height);

            if abs_w == 0 || abs_h == 0 { continue; }

            let right_idx = abs_x.saturating_add(abs_w.saturating_sub(1));
            let bottom_idx = abs_y.saturating_add(abs_h.saturating_sub(1));

            let left = nearest(abs_x, &xs, max_x);
            let right = nearest(right_idx, &xs, max_x);
            let top = nearest(abs_y, &ys, max_y);
            let bottom = nearest(bottom_idx, &ys, max_y);

            let inner_width = if right > left + 2 { right - left - 2 } else { 0 };
            let inner_height = if bottom > top + 2 { bottom - top - 2 } else { 0 };

            section.abs_rect = AbsRect {
                x: left.saturating_add(1),
                y: top.saturating_add(1),
                width: inner_width,
                height: inner_height,
            };

            if top <= max_y && left <= max_x && right <= max_x && left <= right {
                horizontal_edges.push((top, left, right));
            }
            if bottom <= max_y && left <= max_x && right <= max_x && left <= right {
                horizontal_edges.push((bottom, left, right));
            }
            if left <= max_x && top <= max_y && bottom <= max_y && top <= bottom {
                vertical_edges.push((left, top, bottom));
            }
            if right <= max_x && top <= max_y && bottom <= max_y && top <= bottom {
                vertical_edges.push((right, top, bottom));
            }

            intersections.insert((left, top));
            intersections.insert((right, top));
            intersections.insert((left, bottom));
            intersections.insert((right, bottom));
        }

        // Draw edges
        for (y, x_start, x_end) in &horizontal_edges {
            if *y <= max_y {
                draw_horizontal(buf, *y as usize, *x_start as usize, *x_end as usize, '─');
            }
        }

        for (x, y_start, y_end) in &vertical_edges {
            if *x <= max_x {
                draw_vertical(buf, *x as usize, *y_start as usize, *y_end as usize, '│');
            }
        }

        // Draw corner characters
        for (x, y) in intersections {
            if x <= max_x && y <= max_y {
                let corner_char = get_corner_character(buf, x, y);
                buf[y as usize][x as usize].ch = corner_char;
            }
        }
    }

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

fn make_screen_buffer(width: u16, height: u16) -> Vec<Vec<Cell>> {
    vec![vec![Cell::default(); width as usize]; height as usize]
}

fn get_corner_character(buf: &[Vec<Cell>], x: u16, y: u16) -> char {
    let (w, h) = (buf[0].len() as i32, buf.len() as i32);
    let (x, y) = (x as i32, y as i32);

    let is_line = |cx: i32, cy: i32| {
        if cx < 0 || cy < 0 || cx >= w || cy >= h {
            return false;
        }
        matches!(buf[cy as usize][cx as usize].ch,
            '─' | '│' | '┌' | '┐' | '└' | '┘' | '├' | '┤' | '┬' | '┴' | '┼')
    };

    let up = is_line(x, y - 1);
    let down = is_line(x, y + 1);
    let left = is_line(x - 1, y);
    let right = is_line(x + 1, y);

    match (up, down, left, right) {
        (true, true, true, true) => '┼',
        (false, true, true, true) => '┬',
        (true, false, true, true) => '┴',
        (true, true, false, true) => '├',
        (true, true, true, false) => '┤',
        (false, false, true, true) => '─',
        (true, true, false, false) => '│',
        (false, true, false, true) => '┌',
        (false, true, true, false) => '┐',
        (true, false, false, true) => '└',
        (true, false, true, false) => '┘',
        _ => ' ',
    }
}

fn unify_edges(edges: &[f32], term_size: u16, epsilon: f32) -> Vec<u16> {
    if edges.is_empty() {
        return Vec::new();
    }

    let mut sorted = edges.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());

    let mut snapped = Vec::new();
    let mut current_group: Vec<f32> = Vec::new();
    current_group.push(sorted[0]);

    for &e in &sorted[1..] {
        let last = *current_group.last().unwrap();
        if (e - last).abs() <= epsilon {
            current_group.push(e);
        } else {
            let avg = current_group.iter().sum::<f32>() / current_group.len() as f32;
            let mapped = (avg * (term_size.saturating_sub(1) as f32)).round() as u16;
            snapped.push(mapped);
            current_group.clear();
            current_group.push(e);
        }
    }

    if !current_group.is_empty() {
        let avg = current_group.iter().sum::<f32>() / current_group.len() as f32;
        let mapped = (avg * (term_size.saturating_sub(1) as f32)).round() as u16;
        snapped.push(mapped);
    }

    snapped.sort_unstable();
    snapped.dedup();
    snapped
}

fn nearest(val: u16, unified: &[u16], max_index: u16) -> u16 {
    if unified.is_empty() {
        return val.min(max_index);
    }
    let mut best = unified[0];
    let mut best_dist = (val as i32 - best as i32).abs();
    for &u in unified.iter().skip(1) {
        let d = (val as i32 - u as i32).abs();
        if d < best_dist {
            best_dist = d;
            best = u;
        }
    }
    best.min(max_index)
}

fn draw_horizontal(buf: &mut [Vec<Cell>], y: usize, x_start: usize, x_end: usize, ch: char) {
    if y >= buf.len() { return; }
    let row = &mut buf[y];
    if row.is_empty() { return; }
    let max_x = row.len() - 1;
    let xs = x_start.min(max_x);
    let xe = x_end.min(max_x);
    if xs > xe { return; }
    for x in xs..=xe {
        row[x].ch = ch;
    }
}

fn draw_vertical(buf: &mut [Vec<Cell>], x: usize, y_start: usize, y_end: usize, ch: char) {
    if buf.is_empty() { return; }
    let max_y = buf.len() - 1;
    let ys = y_start.min(max_y);
    let ye = y_end.min(max_y);
    if ys > ye { return; }
    if x >= buf[0].len() { return; }
    for y in ys..=ye {
        buf[y][x].ch = ch;
    }
}