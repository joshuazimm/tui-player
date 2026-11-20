use crossterm::event::{self, Event, KeyCode, KeyEventKind, MouseEvent};
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;
use crate::state::GlobalState;

pub struct KeyHandler {
    state: GlobalState,
    mouse_tx: Sender<MouseEvent>,
    handle: Option<thread::JoinHandle<()>>,
}

impl KeyHandler {
    pub fn new(state: GlobalState, mouse_tx: Sender<MouseEvent>) -> Self {
        Self {
            state,
            mouse_tx,
            handle: None,
        }
    }

    pub fn start(&mut self) {
        let state = self.state.clone();
        let mouse_tx = self.mouse_tx.clone();

        let handle = thread::spawn(move || {
            loop {
                if state.should_quit() {
                    break;
                }

                // Poll for events with timeout
                if let Ok(true) = event::poll(Duration::from_millis(100)) {
                    match event::read() {
                    Ok(Event::Mouse(mouse_event)) => {
                        let _ = mouse_tx.send(mouse_event);
                    }
                    Ok(Event::Key(key_event)) if key_event.kind == KeyEventKind::Press => {
                        match key_event.code {
                            KeyCode::Char('q') => {
                                state.set_should_quit(true);
                            }
                            KeyCode::Up | KeyCode::Char('k') => {
                                use crate::state::{StateKey, StateValue};
                                state.set(StateKey::NavigationUp, StateValue::Bool(true));
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                use crate::state::{StateKey, StateValue};
                                state.set(StateKey::NavigationDown, StateValue::Bool(true));
                            }
                            KeyCode::Char(' ') => {
                                let is_playing = state.is_playing();
                                state.set_playing(!is_playing);
                            }
                            KeyCode::Char('+') | KeyCode::Char('=') => {
                                let current_vol = state.volume();
                                state.set_volume((current_vol + 0.01).min(1.0));
                            }
                            KeyCode::Char('-') | KeyCode::Char('_') => {
                                let current_vol = state.volume();
                                state.set_volume((current_vol - 0.01).max(0.0));
                            }
                            KeyCode::Enter => {
                                use crate::state::{StateKey, StateValue};
                                state.set(StateKey::TrackSelected, StateValue::Bool(true));
                            }
                            KeyCode::Esc => {
                                if state.modal_open() {
                                    state.set_modal_open(false);
                                }
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                    }
                }
            }
        });

        self.handle = Some(handle);
    }

    pub fn stop(self) {
        self.state.set_should_quit(true);
        if let Some(handle) = self.handle {
            let _ = handle.join();
        }
    }
}