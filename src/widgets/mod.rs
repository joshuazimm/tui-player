// A new module should be created in the widgets folder for
// each new widget

// Widgets can be appended into the WidgetKind enum using
// `<Struct>(<Struct>),`and exported below using 
// `pub use <module>::<Struct>;` and `pub mod <module>;`

// enum_dispatch handles declaring the traits each new 
// Widget, the update and render functions will just need
// to be implemented in the new module

pub mod label;
pub mod controls;
//pub mod progress_bar;
//pub mod scroll_panel;
//pub mod image;

use crate::widget::{Widget, Rect};
use enum_dispatch::enum_dispatch;

pub use label::Label;
pub use controls::Controls;
//pub use progress_bar::ProgressBar;
//pub use scroll_panel::ScrollPanel;
//pub use image::ImageWidget;

// This Enum allows common handling of different Widget types
#[enum_dispatch(Widget)]
#[derive(Debug, Clone)]
pub enum WidgetKind {
    Label(Label),
    Controls(Controls),
    //ProgressBar(ProgressBar),
    //ScrollPanel(ScrollPanel),
    //Image(ImageWidget)
}