use crate::canvas::Canvas;
use crate::widget::AbsRect;
use crate::state::{GlobalState, StateKey, StateValue};
use crossterm::style::Color;
use std::sync::atomic::{AtomicBool, Ordering};

const OPTIONS: &[&str] = &["  Play", "  Add to Queue"];
const MODAL_W: u16 = 36;
const MODAL_H: u16 = 8;

// Explicit RGB so the bg covers album-art half-block characters fully.
// The fg must also be set (not None) so flush_buffer emits both channels
// and clears the foreground of any previous '▀' cell underneath.
const BG: Color = Color::Rgb { r: 35, g: 35, b: 40 };
const BORDER: Color = Color::Rgb { r: 130, g: 130, b: 160 };
const TEXT: Color = Color::Rgb { r: 220, g: 220, b: 230 };
const DIM: Color = Color::Rgb { r: 90, g: 90, b: 100 };
const SEL_BG: Color = Color::Rgb { r: 80, g: 80, b: 120 };
const SEL_FG: Color = Color::Rgb { r: 255, g: 255, b: 255 };

pub struct FileActionModal {
    active: bool,
    selected_option: usize,
    needs_render: AtomicBool,
    state: GlobalState,
    spawn_x: u16,
    spawn_y: u16,
}

impl FileActionModal {
    pub fn new(state: GlobalState) -> Self {
        Self {
            active: false,
            selected_option: 0,
            needs_render: AtomicBool::new(false),
            state,
            spawn_x: 0,
            spawn_y: 0,
        }
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn needs_render(&self) -> bool {
        self.needs_render.load(Ordering::Relaxed)
    }

    pub fn mark_rendered(&mut self) {
        self.needs_render.store(false, Ordering::Relaxed);
    }

    pub fn subscribed_keys(&self) -> Vec<StateKey> {
        vec![
            StateKey::ModalOpen,
            StateKey::NavigationUp,
            StateKey::NavigationDown,
            StateKey::TrackSelected,
        ]
    }

    fn compute_pos(&self, term_w: u16, term_h: u16) -> (u16, u16, u16, u16) {
        let w = MODAL_W.min(term_w.saturating_sub(2));
        let h = MODAL_H.min(term_h.saturating_sub(2));
        let x = self.spawn_x.min(term_w.saturating_sub(w));
        let y = self.spawn_y.min(term_h.saturating_sub(h));
        (x, y, w, h)
    }

    pub fn handle_mouse(&mut self, abs_col: u16, abs_row: u16, kind: crossterm::event::MouseEventKind) {
        use crossterm::event::{MouseButton, MouseEventKind};
        if !self.active { return; }
        let (term_w, term_h) = self.state.get(&StateKey::TerminalSize)
            .and_then(|v| v.as_terminal_size())
            .unwrap_or((80, 24));
        let (x, y, w, h) = self.compute_pos(term_w, term_h);

        let inside = abs_col >= x && abs_col < x + w && abs_row >= y && abs_row < y + h;

        match kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if !inside {
                    self.active = false;
                    self.state.set_modal_open(false);
                    self.needs_render.store(true, Ordering::Relaxed);
                    return;
                }
                for (i, _) in OPTIONS.iter().enumerate() {
                    if abs_row == y + 2 + i as u16 {
                        self.selected_option = i;
                        self.execute_selected();
                        return;
                    }
                }
            }
            MouseEventKind::Moved => {
                if inside {
                    for (i, _) in OPTIONS.iter().enumerate() {
                        if abs_row == y + 2 + i as u16 && self.selected_option != i {
                            self.selected_option = i;
                            self.needs_render.store(true, Ordering::Relaxed);
                            return;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    pub fn handle_state_change(&mut self, key: &StateKey, value: &StateValue) {
        match key {
            StateKey::ModalOpen => {
                if let Some(open) = value.as_bool() {
                    if self.active != open {
                        self.active = open;
                        if open {
                            self.selected_option = 0;
                            let (sx, sy) = self.state.modal_spawn_pos();
                            self.spawn_x = sx;
                            self.spawn_y = sy;
                        }
                        self.needs_render.store(true, Ordering::Relaxed);
                    }
                }
            }
            StateKey::NavigationUp => {
                if self.active {
                    if let Some(true) = value.as_bool() {
                        self.selected_option = if self.selected_option == 0 {
                            OPTIONS.len() - 1
                        } else {
                            self.selected_option - 1
                        };
                        self.needs_render.store(true, Ordering::Relaxed);
                        self.state.set(StateKey::NavigationUp, StateValue::Bool(false));
                    }
                }
            }
            StateKey::NavigationDown => {
                if self.active {
                    if let Some(true) = value.as_bool() {
                        self.selected_option = (self.selected_option + 1) % OPTIONS.len();
                        self.needs_render.store(true, Ordering::Relaxed);
                        self.state.set(StateKey::NavigationDown, StateValue::Bool(false));
                    }
                }
            }
            StateKey::TrackSelected => {
                if self.active {
                    if let Some(true) = value.as_bool() {
                        self.execute_selected();
                        self.state.set(StateKey::TrackSelected, StateValue::Bool(false));
                    }
                }
            }
            _ => {}
        }
    }

    fn execute_selected(&mut self) {
        match self.selected_option {
            0 => {
                if let Some(path) = self.state.pending_track_path() {
                    let title = path.file_stem()
                        .and_then(|s| s.to_str())
                        .map(|s| s.to_string());
                    self.state.set_current_track(path, title, None, None);
                    self.state.set_playing(true);
                    self.state.set_new_track_requested(true);
                }
            }
            1 => {
                // Add to Queue — placeholder until queue is implemented
            }
            _ => {}
        }
        self.active = false;
        self.state.set_modal_open(false);
    }

    /// The modal's box on screen. Layout hands `render` a canvas of exactly this area.
    pub fn area(&self, term_w: u16, term_h: u16) -> AbsRect {
        let (x, y, width, height) = self.compute_pos(term_w, term_h);
        AbsRect { x, y, width, height }
    }

    /// Draws the modal in local coordinates: (0, 0) is its top-left corner.
    pub fn render(&self, canvas: &mut Canvas) {
        let (w, h) = (canvas.width(), canvas.height());
        if !self.active || w < 2 || h < 2 {
            return;
        }

        // Fill background — explicit fg+bg so flush_buffer emits both color
        // channels and fully overwrites any half-block '▀' cell underneath
        // (album art uses both fg and bg; leaving fg=None would leave the
        // top-half color of the previous character showing on some terminals).
        for y in 0..h {
            for x in 0..w {
                canvas.set(x, y, ' ', Some(BG), Some(BG));
            }
        }

        // Border
        Self::draw_border(canvas, w, h);

        // Title embedded in top border
        let title = " File Actions ";
        let title_x = w.saturating_sub(title.len()) / 2;
        canvas.put_str(title_x, 0, title, Some(TEXT), Some(BG));

        // Options rows
        for (i, option) in OPTIONS.iter().enumerate() {
            let oy = 2 + i;
            if oy >= h - 1 {
                break;
            }
            let is_sel = i == self.selected_option;
            let (fg, bg) = if is_sel { (SEL_FG, SEL_BG) } else { (TEXT, BG) };
            for x in 1..w - 1 {
                canvas.set(x, oy, ' ', Some(fg), Some(bg));
            }
            let prefix = if is_sel { " > " } else { "   " };
            let text = format!("{}{}", prefix, option);
            canvas.put_str(1, oy, &text, Some(fg), Some(bg));
        }

        // Footer hint
        let hint = "↑↓ nav  enter confirm  esc cancel";
        let hint_y = h - 2;
        let hint_x = w.saturating_sub(hint.len() + 2) / 2;
        canvas.put_str(hint_x, hint_y, hint, Some(DIM), Some(BG));
    }

    fn draw_border(canvas: &mut Canvas, w: usize, h: usize) {
        let (right, bottom) = (w - 1, h - 1);
        canvas.set(0, 0, '┌', Some(BORDER), Some(BG));
        canvas.set(right, 0, '┐', Some(BORDER), Some(BG));
        canvas.set(0, bottom, '└', Some(BORDER), Some(BG));
        canvas.set(right, bottom, '┘', Some(BORDER), Some(BG));
        for x in 1..right {
            canvas.set(x, 0, '─', Some(BORDER), Some(BG));
            canvas.set(x, bottom, '─', Some(BORDER), Some(BG));
        }
        for y in 1..bottom {
            canvas.set(0, y, '│', Some(BORDER), Some(BG));
            canvas.set(right, y, '│', Some(BORDER), Some(BG));
        }
    }
}
