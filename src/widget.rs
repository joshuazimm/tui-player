use std::fmt::Write as _;
use std::io::{Write};
use std::sync::mpsc::Receiver;
use enum_dispatch::enum_dispatch;
use crate::canvas::Canvas;
use crate::widgets::WidgetKind;
use crate::widgets::FileActionModal;
use crate::state::{GlobalState, StateKey, StateValue};
use crossterm::{terminal::size, cursor::MoveTo, style::{SetForegroundColor, SetBackgroundColor, ResetColor, Color}};
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

#[derive(Clone, Copy, PartialEq)]
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

#[derive(Clone, Copy, PartialEq)]
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

/// A section's border box: the screen rows/columns its border lines sit on (inclusive).
/// Neighbouring sections share border lines.
#[derive(Clone, Copy)]
pub struct Bounds {
    pub left: u16,
    pub top: u16,
    pub right: u16,
    pub bottom: u16,
}

impl Bounds {
    /// The area a widget draws into: every cell strictly inside the border.
    fn content(&self) -> AbsRect {
        AbsRect {
            x: self.left.saturating_add(1),
            y: self.top.saturating_add(1),
            width: if self.right > self.left + 1 { self.right - self.left - 1 } else { 0 },
            height: if self.bottom > self.top + 1 { self.bottom - self.top - 1 } else { 0 },
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Axis {
    /// Children laid out left to right; dividers are vertical lines.
    Row,
    /// Children laid out top to bottom; dividers are horizontal lines.
    Column,
}

/// Declarative description of the screen layout, handed to `Layout::new`.
/// Each split's children carry a weight; weights are relative to their
/// siblings and need not sum to 1.
pub enum LayoutNode {
    Widget(WidgetKind),
    Split { axis: Axis, children: Vec<(f32, LayoutNode)> },
}

impl LayoutNode {
    pub fn row(children: Vec<(f32, LayoutNode)>) -> Self {
        LayoutNode::Split { axis: Axis::Row, children }
    }

    pub fn column(children: Vec<(f32, LayoutNode)>) -> Self {
        LayoutNode::Split { axis: Axis::Column, children }
    }
}

/// The layout tree as `Layout` stores it: leaves index into `Layout::sections`.
/// Each split has an id, assigned once by `flatten`; the tree's shape never
/// changes at runtime, so ids stay valid across every `compute`.
enum Node {
    Leaf(usize),
    Split { id: usize, axis: Axis, weights: Vec<f32>, children: Vec<Node> },
}

/// Identifies a divider across recomputes: the line between child `index`
/// and child `index + 1` of the split with id `split`.
#[derive(Clone, Copy, PartialEq)]
pub struct DividerKey {
    pub split: usize,
    pub index: usize,
}

/// A border line between two children of a split, as placed by the last `compute`.
#[derive(Clone, Copy)]
pub struct Divider {
    pub key: DividerKey,
    /// The axis of the split it divides: `Row` = a vertical line at column `pos`.
    pub axis: Axis,
    pub pos: u16,
    /// Extent along the other axis (inclusive), ending on the parent's border lines.
    pub start: u16,
    pub end: u16,
    /// Unrounded positions of the boundaries either side of it (where the first
    /// child starts and the second ends): the range a drag can move it in.
    pub before: f32,
    pub after: f32,
}

/// The fewest cells a drag leaves inside each of the two sections beside a divider.
const MIN_SECTION_SIZE: f32 = 2.0;

/// The colour of a divider under the pointer or being dragged.
const DIVIDER_HIGHLIGHT: Color = Color::Cyan;

#[enum_dispatch]
pub trait Widget {
    fn update(&mut self, state: &GlobalState);
    /// Draws into a canvas covering only this widget's content area,
    /// addressed from (0, 0).
    fn render(&self, canvas: &mut Canvas) -> std::io::Result<()>;
    fn needs_render(&self) -> bool;
    fn mark_rendered(&mut self);
    fn handle_state_change(&mut self, key: &StateKey, value: &StateValue);
    fn subscribed_keys(&self) -> Vec<StateKey>;
    /// Called by `Layout` when the widget's content area changes (resize or
    /// layout change). Widgets never compute their own geometry.
    fn on_resize(&mut self, _area: &AbsRect) {}
}

pub struct Section {
    /// Content area handed to the widget, set by `Layout::compute`.
    pub abs_rect: AbsRect,
    pub widget: WidgetKind,
    pub needs_render: bool,
}

impl Section {
    fn new(widget: WidgetKind) -> Self {
        Self {
            abs_rect: AbsRect::default(),
            widget,
            needs_render: true,
        }
    }

    /// Clears the widget's area and renders it through a canvas limited to that area.
    fn render(&mut self, screen: &mut [Vec<Cell>]) -> std::io::Result<()> {
        let mut canvas = Canvas::new(screen, self.abs_rect);
        for y in 0..canvas.height() {
            canvas.row_mut(y).fill(Cell::default());
        }
        self.widget.render(&mut canvas)?;
        self.needs_render = false;
        Ok(())
    }
}

pub struct Layout {
    pub sections: Vec<Section>,
    root: Node,
    dividers: Vec<Divider>,
    /// Every border cell with its glyph, precomputed by `compute`.
    border_cells: Vec<(u16, u16, char)>,
    last_terminal_size: (u16, u16),
    last_global_change_id: u64,
    screen: Vec<Vec<Cell>>, // frame being built
    last_flushed: Vec<Vec<Cell>>, // what the terminal currently shows
    output: String, // escape-sequence buffer, reused between flushes
    force_full_render: bool, // Flag to force complete re-render
    mouse_rx: Option<Receiver<MouseEvent>>,
    modal: FileActionModal,
    /// Whether the last rendered frame included the modal
    modal_drawn: bool,
    /// The divider being dragged; it receives every mouse event until release
    dragging: Option<DividerKey>,
    /// The divider under the pointer, drawn highlighted
    hovered: Option<DividerKey>,
    /// Borders need redrawing (the highlighted divider changed)
    borders_dirty: bool,
}

impl Layout {
    pub fn new(root: LayoutNode, state: GlobalState) -> Self {
        let mut sections = Vec::new();
        let root = flatten(root, &mut sections, &mut 0);
        let (width, height) = size().unwrap_or((80, 24));
        let mut layout = Self {
            sections,
            root,
            dividers: Vec::new(),
            border_cells: Vec::new(),
            last_terminal_size: (width, height),
            last_global_change_id: 0,
            screen: make_screen_buffer(width, height),
            last_flushed: Vec::new(),
            output: String::new(),
            force_full_render: true, // Start with a full render
            mouse_rx: None,
            modal: FileActionModal::new(state),
            modal_drawn: false,
            dragging: None,
            hovered: None,
            borders_dirty: false,
        };
        layout.compute();
        layout
    }

    pub fn set_mouse_receiver(&mut self, rx: Receiver<MouseEvent>) {
        self.mouse_rx = Some(rx);
    }

    pub fn init_all(&mut self, _state: &GlobalState) {
    }

    /// Gives a mouse event to exactly one thing. Unlike state changes, which go
    /// to every subscriber, a mouse event has a single owner, so the checks run
    /// from the top layer down and the first match takes the event:
    ///   1. a divider being dragged (it owns the pointer until release)
    ///   2. the modal, drawn over everything
    ///   3. a divider under the pointer (part of the layout, not of any widget)
    ///   4. the widget under the pointer
    fn route_mouse_event(&mut self, ev: MouseEvent) {
        let col = ev.column;
        let row = ev.row;

        // 1. A drag keeps every event until the button is released, even when
        //    the pointer moves off the divider or over another section.
        if let Some(key) = self.dragging {
            match ev.kind {
                MouseEventKind::Drag(MouseButton::Left) => {
                    self.drag_divider_to(key, col, row);
                    return;
                }
                MouseEventKind::Down(_) => {
                    // The release was lost (e.g. it happened outside the
                    // terminal): end the drag and handle this press normally.
                    self.dragging = None;
                    self.borders_dirty = true;
                }
                MouseEventKind::Up(_) => {
                    self.dragging = None;
                    self.borders_dirty = true;
                    return;
                }
                _ => return,
            }
        }

        // 2. The modal
        if self.modal.is_active() {
            self.modal.handle_mouse(col, row, ev.kind);
            return;
        }

        // 3. Dividers: hovering highlights one, a press starts dragging it.
        //    Events on a divider never reach a widget.
        let divider = self.divider_at(col, row);
        if divider != self.hovered {
            self.hovered = divider;
            self.borders_dirty = true;
        }
        if let Some(key) = divider {
            if ev.kind == MouseEventKind::Down(MouseButton::Left) {
                self.dragging = Some(key);
                self.borders_dirty = true;
            }
            return;
        }

        // 4. The widget under the pointer
        for section in &mut self.sections {
            let r = &section.abs_rect;
            if col >= r.x && col < r.x + r.width && row >= r.y && row < r.y + r.height {
                let local_col = col - r.x;
                let local_row = row - r.y;
                section.widget.handle_mouse(local_col, local_row, col, row, ev.kind);
                break;
            }
        }
    }

    /// The divider whose line passes through this cell. A divider's two end
    /// cells sit on its parent's line (a ┬ or ├ junction), so they belong to
    /// the parent: grabbing a junction moves the line running through it.
    fn divider_at(&self, col: u16, row: u16) -> Option<DividerKey> {
        self.dividers.iter()
            .find(|d| match d.axis {
                Axis::Row => col == d.pos && row > d.start && row < d.end,
                Axis::Column => row == d.pos && col > d.start && col < d.end,
            })
            .map(|d| d.key)
    }

    /// Moves a divider to the pointer by changing the weights of the two
    /// sections beside it; nothing else in the layout moves.
    fn drag_divider_to(&mut self, key: DividerKey, col: u16, row: u16) {
        let Some(d) = self.dividers.iter().find(|d| d.key == key).copied() else { return };
        let pointer = match d.axis {
            Axis::Row => col as f32,
            Axis::Column => row as f32,
        };

        // Keep MIN_SECTION_SIZE cells inside each section (plus the divider itself)
        let margin = MIN_SECTION_SIZE + 1.0;
        if d.after - d.before < margin * 2.0 {
            return; // no room to move it at all
        }
        let pos = pointer.clamp(d.before + margin, d.after - margin);

        // The two weights keep their sum and are split at the new position
        let share = (pos - d.before) / (d.after - d.before);
        let Some(weights) = split_weights_mut(&mut self.root, key.split) else { return };
        let pair = weights[key.index] + weights[key.index + 1];
        weights[key.index] = pair * share;
        weights[key.index + 1] = pair - weights[key.index];

        // Same path as a terminal resize: new geometry, then a full redraw
        self.compute();
        self.force_full_render = true;
    }

    /// Recomputes every section's geometry and the divider list from the tree
    /// for the current terminal size. Only needed when the size or the tree changes.
    fn compute(&mut self) {
        let (w, h) = self.last_terminal_size;
        self.dividers.clear();
        self.border_cells.clear();
        if w == 0 || h == 0 {
            for section in &mut self.sections {
                section.abs_rect = AbsRect::default();
            }
            return;
        }
        let frame = Bounds { left: 0, top: 0, right: w - 1, bottom: h - 1 };
        let frac = FracBounds { left: 0.0, top: 0.0, right: (w - 1) as f32, bottom: (h - 1) as f32 };
        layout_node(&self.root, frame, frac, &mut self.sections, &mut self.dividers);
        self.border_cells = border_cells(w, h, frame, &self.dividers);
    }

    /// Picks up a terminal resize: new screen buffer, new geometry, full redraw.
    /// Returns true if the size changed.
    fn sync_terminal_size(&mut self) -> bool {
        let Ok((width, height)) = size() else { return false };
        if (width, height) == self.last_terminal_size {
            return false;
        }
        self.last_terminal_size = (width, height);
        self.screen = make_screen_buffer(width, height);
        // The terminal's contents can't be trusted after a resize: flush every cell
        self.last_flushed.clear();
        self.compute();
        self.force_full_render = true;
        for section in &mut self.sections {
            section.needs_render = true;
        }
        true
    }

    pub fn update_all(&mut self, state: &GlobalState) -> std::io::Result<()> {
        // Drain and route pending mouse events
        let mouse_events: Vec<MouseEvent> = self.mouse_rx.as_ref()
            .map(|rx| rx.try_iter().collect())
            .unwrap_or_default();
        for ev in mouse_events {
            self.route_mouse_event(ev);
        }

        if self.sync_terminal_size() {
            state.update_terminal_size();
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
                    // Also route to modal
                    if self.modal.subscribed_keys().contains(change_key) {
                        self.modal.handle_state_change(change_key, &value);
                    }
                }
            }

            self.last_global_change_id = current_change_id;
        }

        // If modal changed, force a full re-render so the overlay is applied on top
        if self.modal.needs_render() {
            self.force_full_render = true;
            self.modal.mark_rendered();
        }

        // Update all widgets
        for section in &mut self.sections {
            section.widget.update(state);

            // Check if widget needs render
            if section.widget.needs_render() {
                section.needs_render = true;
                section.widget.mark_rendered();
            }
        }
        Ok(())
    }

    pub fn render_all(&mut self, out: &mut std::io::Stdout, force_render: bool) -> std::io::Result<()> {
        // A resize since update_all forces a complete re-render
        self.sync_terminal_size();

        // The modal is the one thing that draws over other sections' areas, so
        // when it opens or closes everything under it has to be redrawn. Layout
        // checks this itself instead of relying on the modal to report it.
        if self.modal.is_active() != self.modal_drawn {
            self.force_full_render = true;
        }

        // Check if any rendering is needed
        let any_needs_render = force_render || self.force_full_render || self.borders_dirty ||
                             self.sections.iter().any(|s| s.needs_render);
        
        if !any_needs_render {
            return Ok(());
        }

        // Use force render for complete re-renders
        if self.force_full_render || force_render {
            return self.force_render_all(out);
        }

        // Partial render. Widgets can't draw over borders, so they only need
        // redrawing when the highlighted divider changed.
        if self.borders_dirty {
            self.draw_borders();
        }

        // Render only the widgets that need updating
        for section in self.sections.iter_mut() {
            if section.needs_render {
                section.render(&mut self.screen)?;
            }
        }

        self.render_modal();
        self.flush_buffer(out)
    }

    pub fn force_render_all(&mut self, out: &mut std::io::Stdout) -> std::io::Result<()> {
        // Picks up a resize (new buffer + geometry); otherwise clear the existing buffer
        if !self.sync_terminal_size() {
            for row in &mut self.screen {
                row.fill(Cell::default());
            }
        }

        for section in &mut self.sections {
            section.needs_render = true;
        }

        // Draw borders first
        self.draw_borders();

        // Render all widgets
        for section in self.sections.iter_mut() {
            section.render(&mut self.screen)?;
        }

        self.render_modal();
        self.force_full_render = false;
        self.flush_buffer(out)
    }

    /// Draws the modal on top of everything, through a canvas covering only its box.
    fn render_modal(&mut self) {
        self.modal_drawn = self.modal.is_active();
        if !self.modal.is_active() {
            return;
        }
        let (term_w, term_h) = self.last_terminal_size;
        let mut canvas = Canvas::new(&mut self.screen, self.modal.area(term_w, term_h));
        self.modal.render(&mut canvas);
    }

    /// Stamps the precomputed border glyphs into the screen buffer. The divider
    /// being dragged (or else the one under the pointer) is highlighted.
    fn draw_borders(&mut self) {
        let highlighted = self.dragging.or(self.hovered)
            .and_then(|key| self.dividers.iter().find(|d| d.key == key));

        for &(x, y, ch) in &self.border_cells {
            let Some(cell) = self.screen.get_mut(y as usize).and_then(|row| row.get_mut(x as usize)) else {
                continue;
            };
            cell.ch = ch;
            cell.fg = None;

            if let Some(d) = highlighted {
                let on_divider = match d.axis {
                    Axis::Row => x == d.pos && y >= d.start && y <= d.end,
                    Axis::Column => y == d.pos && x >= d.start && x <= d.end,
                };
                if on_divider {
                    cell.fg = Some(DIVIDER_HIGHLIGHT);
                }
            }
        }
        self.borders_dirty = false;
    }

    /// Sends the terminal only the cells that changed since the last flush.
    /// A run of changed cells needs one cursor move, and colour codes are only
    /// sent when the colour changes.
    fn flush_buffer(&mut self, out: &mut std::io::Stdout) -> std::io::Result<()> {
        // Nothing flushed yet at this size: every cell has to be sent
        let send_everything = self.last_flushed.len() != self.screen.len();

        // (Writing into a String can't fail, so the fmt results are ignored.)
        self.output.clear();
        let _ = write!(self.output, "{}", ResetColor);

        // The colours the terminal is currently drawing with
        let mut fg: Option<Color> = None;
        let mut bg: Option<Color> = None;
        let mut anything_changed = false;

        for (y, row) in self.screen.iter().enumerate() {
            // Whether the terminal cursor already sits on this cell
            let mut cursor_here = false;

            for (x, cell) in row.iter().enumerate() {
                if !send_everything && *cell == self.last_flushed[y][x] {
                    cursor_here = false;
                    continue;
                }
                anything_changed = true;

                if !cursor_here {
                    let _ = write!(self.output, "{}", MoveTo(x as u16, y as u16));
                    cursor_here = true;
                }

                // There is no code for "no colour", so going back to none means a reset
                if (fg.is_some() && cell.fg.is_none()) || (bg.is_some() && cell.bg.is_none()) {
                    let _ = write!(self.output, "{}", ResetColor);
                    fg = None;
                    bg = None;
                }
                if let Some(color) = cell.fg {
                    if fg != Some(color) {
                        let _ = write!(self.output, "{}", SetForegroundColor(color));
                        fg = Some(color);
                    }
                }
                if let Some(color) = cell.bg {
                    if bg != Some(color) {
                        let _ = write!(self.output, "{}", SetBackgroundColor(color));
                        bg = Some(color);
                    }
                }

                self.output.push(cell.ch);
            }
        }

        if !anything_changed {
            return Ok(());
        }
        let _ = write!(self.output, "{}", ResetColor);

        out.write_all(self.output.as_bytes())?;
        out.flush()?;
        self.last_flushed.clone_from(&self.screen);
        Ok(())
    }
}

fn make_screen_buffer(width: u16, height: u16) -> Vec<Vec<Cell>> {
    vec![vec![Cell::default(); width as usize]; height as usize]
}

// Which arms of a box-drawing glyph a border cell has.
const UP: u8 = 1;
const DOWN: u8 = 2;
const LEFT: u8 = 4;
const RIGHT: u8 = 8;

/// Builds the glyph for every border cell from the frame and dividers. Each
/// line marks the arms it actually has at each of its cells, so junctions are
/// exact rather than guessed from neighbouring characters.
fn border_cells(w: u16, h: u16, frame: Bounds, dividers: &[Divider]) -> Vec<(u16, u16, char)> {
    let (w, h) = (w as usize, h as usize);
    let mut arms = vec![0u8; w * h];

    let mut horizontal = |y: u16, x0: u16, x1: u16| {
        let (y, x0, x1) = (y as usize, x0 as usize, (x1 as usize).min(w - 1));
        if y >= h { return; }
        for x in x0..=x1 {
            if x > x0 { arms[y * w + x] |= LEFT; }
            if x < x1 { arms[y * w + x] |= RIGHT; }
        }
    };
    horizontal(frame.top, frame.left, frame.right);
    horizontal(frame.bottom, frame.left, frame.right);
    for d in dividers.iter().filter(|d| d.axis == Axis::Column) {
        horizontal(d.pos, d.start, d.end);
    }

    let mut vertical = |x: u16, y0: u16, y1: u16| {
        let (x, y0, y1) = (x as usize, y0 as usize, (y1 as usize).min(h - 1));
        if x >= w { return; }
        for y in y0..=y1 {
            if y > y0 { arms[y * w + x] |= UP; }
            if y < y1 { arms[y * w + x] |= DOWN; }
        }
    };
    vertical(frame.left, frame.top, frame.bottom);
    vertical(frame.right, frame.top, frame.bottom);
    for d in dividers.iter().filter(|d| d.axis == Axis::Row) {
        vertical(d.pos, d.start, d.end);
    }

    arms.iter()
        .enumerate()
        .filter(|&(_, &a)| a != 0)
        .map(|(i, &a)| ((i % w) as u16, (i / w) as u16, glyph(a)))
        .collect()
}

fn glyph(arms: u8) -> char {
    match arms {
        a if a == UP | DOWN | LEFT | RIGHT => '┼',
        a if a == DOWN | LEFT | RIGHT => '┬',
        a if a == UP | LEFT | RIGHT => '┴',
        a if a == UP | DOWN | RIGHT => '├',
        a if a == UP | DOWN | LEFT => '┤',
        a if a == DOWN | RIGHT => '┌',
        a if a == DOWN | LEFT => '┐',
        a if a == UP | RIGHT => '└',
        a if a == UP | LEFT => '┘',
        a if a & (LEFT | RIGHT) != 0 => '─',
        _ => '│',
    }
}

/// Moves widgets out of the declarative tree into `sections`, leaving indices
/// behind, and numbers the splits.
fn flatten(node: LayoutNode, sections: &mut Vec<Section>, split_count: &mut usize) -> Node {
    match node {
        LayoutNode::Widget(widget) => {
            sections.push(Section::new(widget));
            Node::Leaf(sections.len() - 1)
        }
        LayoutNode::Split { axis, children } => {
            assert!(!children.is_empty(), "layout split has no children");
            let id = *split_count;
            *split_count += 1;
            let (weights, children): (Vec<f32>, Vec<Node>) = children
                .into_iter()
                .map(|(w, child)| {
                    assert!(w.is_finite() && w > 0.0, "layout weight must be positive, got {w}");
                    (w, flatten(child, sections, split_count))
                })
                .unzip();
            Node::Split { id, axis, weights, children }
        }
    }
}

/// Unrounded border positions in cell units, carried down the tree so each
/// divider is rounded exactly once (rounding per level would compound).
#[derive(Clone, Copy)]
struct FracBounds {
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
}

/// Assigns `bounds` to a node and recursively to its children. A split places
/// its dividers at the weight boundaries between its own border lines;
/// neighbouring children share the divider as a border.
fn layout_node(
    node: &Node,
    bounds: Bounds,
    frac: FracBounds,
    sections: &mut [Section],
    dividers: &mut Vec<Divider>,
) {
    match node {
        Node::Leaf(i) => {
            let section = &mut sections[*i];
            let area = bounds.content();
            if area != section.abs_rect {
                section.abs_rect = area;
                section.widget.on_resize(&area);
            }
        }
        Node::Split { id, axis, weights, children } => {
            let (start, end, frac_start, frac_end) = match axis {
                Axis::Row => (bounds.left, bounds.right, frac.left, frac.right),
                Axis::Column => (bounds.top, bounds.bottom, frac.top, frac.bottom),
            };

            // Where each child starts and ends, rounded and unrounded: one
            // boundary more than there are children
            let total: f32 = weights.iter().sum();
            let mut boundaries = vec![(start, frac_start)];
            let mut cumulative = 0.0;
            for weight in &weights[..weights.len() - 1] {
                cumulative += weight;
                let f = frac_start + cumulative / total * (frac_end - frac_start);
                let prev = boundaries.last().unwrap().0;
                boundaries.push(((f.round() as u16).clamp(prev, end), f));
            }
            boundaries.push((end, frac_end));

            for (k, child) in children.iter().enumerate() {
                let (prev, frac_prev) = boundaries[k];
                let (next, frac_next) = boundaries[k + 1];

                let (child_bounds, child_frac) = match axis {
                    Axis::Row => (
                        Bounds { left: prev, right: next, ..bounds },
                        FracBounds { left: frac_prev, right: frac_next, ..frac },
                    ),
                    Axis::Column => (
                        Bounds { top: prev, bottom: next, ..bounds },
                        FracBounds { top: frac_prev, bottom: frac_next, ..frac },
                    ),
                };
                layout_node(child, child_bounds, child_frac, sections, dividers);
            }

            // A divider on each boundary between two children
            let (along_start, along_end) = match axis {
                Axis::Row => (bounds.top, bounds.bottom),
                Axis::Column => (bounds.left, bounds.right),
            };
            for k in 1..children.len() {
                dividers.push(Divider {
                    key: DividerKey { split: *id, index: k - 1 },
                    axis: *axis,
                    pos: boundaries[k].0,
                    start: along_start,
                    end: along_end,
                    before: boundaries[k - 1].1,
                    after: boundaries[k + 1].1,
                });
            }
        }
    }
}

/// The weights of the split with this id.
fn split_weights_mut(node: &mut Node, id: usize) -> Option<&mut Vec<f32>> {
    match node {
        Node::Leaf(_) => None,
        Node::Split { id: split_id, weights, children, .. } => {
            if *split_id == id {
                return Some(weights);
            }
            children.iter_mut().find_map(|child| split_weights_mut(child, id))
        }
    }
}

