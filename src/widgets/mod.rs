// A new module should be created in the widgets folder for
// each new widget

// Widgets can be appended into the WidgetKind enum using
// `<Struct>(<Struct>),`and exported below using
// `pub use <module>::<Struct>;` and `pub mod <module>;`

// enum_dispatch handles declaring the traits each new
// Widget, the update and render functions will just need
// to be implemented in the new module

pub mod controls;
pub mod file_list;
pub mod file_action_modal;
pub mod label;
pub mod album_art;
pub mod progress_bar;
pub mod fps_counter;
pub mod spectrum;

use crate::widget::{AbsRect, Widget, Cell};
use enum_dispatch::enum_dispatch;
use crate::state::{GlobalState, StateKey, StateValue};
use crossterm::event::MouseEventKind;

pub use controls::Controls;
pub use file_list::FileList;
pub use file_action_modal::FileActionModal;
pub use label::Label;
pub use album_art::AlbumArt;
pub use progress_bar::ProgressBar;
pub use fps_counter::FpsCounter;
pub use spectrum::Spectrum;

#[enum_dispatch(Widget)]
pub enum WidgetKind {
    Label(Label),
    Controls(Controls),
    FileList(FileList),
    AlbumArt(AlbumArt),
    ProgressBar(ProgressBar),
    FpsCounter(FpsCounter),
    Spectrum(Spectrum),
}

impl WidgetKind {
    pub fn handle_state_change(&mut self, key: &StateKey, value: &StateValue) {
        match self {
            WidgetKind::Controls(w) => w.handle_state_change(key, value),
            WidgetKind::FileList(w) => w.handle_state_change(key, value),
            WidgetKind::Label(w) => w.handle_state_change(key, value),
            WidgetKind::AlbumArt(w) => w.handle_state_change(key, value),
            WidgetKind::ProgressBar(w) => w.handle_state_change(key, value),
            WidgetKind::FpsCounter(w) => w.handle_state_change(key, value),
            WidgetKind::Spectrum(w) => w.handle_state_change(key, value),
        }
    }

    pub fn subscribed_keys(&self) -> Vec<StateKey> {
        match self {
            WidgetKind::Controls(w) => w.subscribed_keys(),
            WidgetKind::FileList(w) => w.subscribed_keys(),
            WidgetKind::Label(w) => w.subscribed_keys(),
            WidgetKind::AlbumArt(w) => w.subscribed_keys(),
            WidgetKind::ProgressBar(w) => w.subscribed_keys(),
            WidgetKind::FpsCounter(w) => w.subscribed_keys(),
            WidgetKind::Spectrum(w) => w.subscribed_keys(),
        }
    }

    pub fn handle_mouse(&mut self, local_col: u16, local_row: u16, abs_col: u16, abs_row: u16, kind: MouseEventKind) {
        match self {
            WidgetKind::FileList(w) => w.handle_mouse(local_col, local_row, abs_col, abs_row, kind),
            _ => {}
        }
    }
}