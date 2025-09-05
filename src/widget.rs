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
    // Maps to normalized coords in [0..term_size-1] with clamping
    pub fn to_absolute(&self, term_width: u16, term_height: u16) -> (u16, u16, u16, u16) {
        let max_x_idx = term_width.saturating_sub(1) as f32;
        let max_y_idx = term_height.saturating_sub(1) as f32;

        let mut abs_x = (self.x * max_x_idx).round() as i32;
        let mut abs_y = (self.y * max_y_idx).round() as i32;

        if abs_x < 0 { abs_x = 0; }
        if abs_y < 0 { abs_y = 0; }
        if abs_x as u32 > term_width as u32 { abs_x = max_x_idx as i32; }
        if abs_y as u32 > term_height as u32 { abs_y = max_y_idx as i32; }

        let mut abs_w = (self.width * term_width as f32).round() as i32;
        let mut abs_h = (self.height * term_height as f32).round() as i32;

        let remaining_x = (max_x_idx as i32).saturating_sub(abs_x) + 1;
        let remaining_y = (max_y_idx as i32).saturating_sub(abs_y) + 1;
        if abs_w > remaining_x { abs_w = remaining_x; }
        if abs_h > remaining_y { abs_h = remaining_y; }
        if abs_w < 0 { abs_w = 0; }
        if abs_h < 0 { abs_h = 0; }

        (abs_x as u16, abs_y as u16, abs_w as u16, abs_h as u16)
    }
}

pub struct Section {
    pub rect: Rect,
    pub abs_rect: AbsRect,
    pub widget: WidgetKind
}

impl Section {
    pub fn new(rect: Rect, widget: WidgetKind) -> Self {
        Self {
            rect,
            abs_rect: AbsRect::default(),
            widget
        }
    }
}

#[enum_dispatch]
pub trait Widget {
    fn update(&mut self);
    fn render(&self, area: &AbsRect, buf: &mut [Vec<Cell>]) -> std::io::Result<()>;
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
        let (term_w, term_h) = size().unwrap();
        let mut screen = make_screen_buffer(term_w, term_h);

        // TODO: optimize by only recalculating borders on screen or widget resize
        self.draw_unified_borders(&mut screen, term_w, term_h);

        for section in self.sections.iter_mut() {
            section.widget.render(&section.abs_rect, &mut screen)?;
        }

        Self::flush_buffer(out, &screen)
    }

    /*
    This function:
      - collects floats edges in normalized space
      - greedily merges edges within `epsilon`
      - maps merged edges to cell indices (0..term_size-1)
      - rebuilds snapped absolute rects and draws borders (inclusive)
    */
    fn draw_unified_borders(&mut self, buf: &mut [Vec<Cell>], term_width: u16, term_height: u16) {
        let mut xs_f = Vec::new();
        let mut ys_f = Vec::new();

        for section in &self.sections {
            xs_f.push(section.rect.x);
            xs_f.push(section.rect.x + section.rect.width);
            ys_f.push(section.rect.y);
            ys_f.push(section.rect.y + section.rect.height);
        }

        // epsilon is in normalized units (0.0..1.0). 0.02 ~ 2% of terminal dimension.
        let epsilon = 0.02_f32;
        let xs = unify_edges(&xs_f, term_width, epsilon);
        let ys = unify_edges(&ys_f, term_height, epsilon);

        // Collect all border edges that need to be drawn
        let mut horizontal_edges: Vec<(u16, u16, u16)> = Vec::new(); // (y, x_start, x_end) inclusive
        let mut vertical_edges: Vec<(u16, u16, u16)> = Vec::new();   // (x, y_start, y_end) inclusive
        let mut intersections = std::collections::HashSet::new();

        let max_x = term_width.saturating_sub(1);
        let max_y = term_height.saturating_sub(1);

        for section in &mut self.sections {
            let (abs_x, abs_y, abs_w, abs_h) = section.rect.to_absolute(term_width, term_height);

            if abs_w == 0 || abs_h == 0 { continue; }

            // Convert width/height counts to inclusive indices for right/bottom
            let right_idx = abs_x.saturating_add(abs_w.saturating_sub(1));
            let bottom_idx = abs_y.saturating_add(abs_h.saturating_sub(1));

            // Snap edges to nearest unified positions (or keep within bounds)
            let left = nearest(abs_x, &xs, max_x);
            let right = nearest(right_idx, &xs, max_x);
            let top = nearest(abs_y, &ys, max_y);
            let bottom = nearest(bottom_idx, &ys, max_y);

            // Compute inner area (the widget area inside the borders).
            let inner_width = if right > left + 2 { right - left - 2 } else { 0 };
            let inner_height = if bottom > top + 2 { bottom - top - 2 } else { 0 };

            section.abs_rect = AbsRect {
                x: left.saturating_add(1),
                y: top.saturating_add(1),
                width: inner_width,
                height: inner_height,
            };

            // Save border line segments (top bottom left right)
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

            // Add corner intersections from snapped coords
            intersections.insert((left, top));
            intersections.insert((right, top));
            intersections.insert((left, bottom));
            intersections.insert((right, bottom));
        }

        // Draw horizontal edges (inclusive)
        for (y, x_start, x_end) in &horizontal_edges {
            if *y <= max_y {
                draw_horizontal(buf, *y as usize, *x_start as usize, *x_end as usize, '─');
            }
        }

        // Draw vertical edges (inclusive)
        for (x, y_start, y_end) in &vertical_edges {
            if *x <= max_x {
                draw_vertical(buf, *x as usize, *y_start as usize, *y_end as usize, '│');
            }
        }

        // Draw intersections/corners using the drawn lines to determine the correct corner char
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

fn get_corner_character(buf: &[Vec<Cell>], x: u16, y: u16) -> char {
    let (w, h) = (buf[0].len() as i32, buf.len() as i32);
    let (x, y) = (x as i32, y as i32);

    // Helper to check if neighbor has a line char
    let is_line = |cx: i32, cy: i32| {
        if cx < 0 || cy < 0 || cx >= w || cy >= h {
            return false;
        }
        matches!(buf[cy as usize][cx as usize].ch,
            '─' | '│' | '┌' | '┐' | '└' | '┘' | '├' | '┤' | '┬' | '┴' | '┼')
    };

    let up    = is_line(x, y - 1);
    let down  = is_line(x, y + 1);
    let left  = is_line(x - 1, y);
    let right = is_line(x + 1, y);

    match (up, down, left, right) {
        (true,  true,  true,  true ) => '┼',
        (false, true,  true,  true ) => '┬',
        (true,  false, true,  true ) => '┴',
        (true,  true,  false, true ) => '├',
        (true,  true,  true,  false) => '┤',
        (false, false, true,  true ) => '─',
        (true,  true,  false, false) => '│',
        (false, true,  false, true ) => '┌',
        (false, true,  true,  false) => '┐',
        (true,  false, false, true ) => '└',
        (true,  false, true,  false) => '┘',
        _ => ' ', // no connections
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
            // snap group -> average position, then map to cell index 0..term_size-1
            let avg = current_group.iter().sum::<f32>() / current_group.len() as f32;
            let mapped = (avg * (term_size.saturating_sub(1) as f32)).round() as u16;
            snapped.push(mapped);
            current_group.clear();
            current_group.push(e);
        }
    }

    // flush last group
    if !current_group.is_empty() {
        let avg = current_group.iter().sum::<f32>() / current_group.len() as f32;
        let mapped = (avg * (term_size.saturating_sub(1) as f32)).round() as u16;
        snapped.push(mapped);
    }

    snapped.sort_unstable();
    snapped.dedup();
    snapped
}

// Find nearest unified coordinate to val, clamped to max_index when unified is empty
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