use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU32, Ordering},
};
use std::thread;
use std::time::Duration;

pub struct KeyHandler {
    pub should_quit: Arc<AtomicBool>,
    pub is_paused: Arc<AtomicBool>,
    pub volume: Arc<AtomicU32>,
    pub selected_index: Arc<AtomicU32>,
    pub enter_pressed: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl KeyHandler {
    pub fn new(
        is_paused: Arc<AtomicBool>,
        volume: Arc<AtomicU32>,
        selected_index: Arc<AtomicU32>,
        enter_pressed: Arc<AtomicBool>,
    ) -> Self {
        Self {
            should_quit: Arc::new(AtomicBool::new(false)),
            is_paused,
            volume,
            selected_index,
            enter_pressed,
            handle: None,
        }
    }

    pub fn start(&mut self) {
        let should_quit = Arc::clone(&self.should_quit);
        let is_paused = Arc::clone(&self.is_paused);
        let volume = Arc::clone(&self.volume);
        let selected_index = Arc::clone(&self.selected_index);
        let enter_pressed = Arc::clone(&self.enter_pressed);

        let handle = thread::spawn(move || {
            loop {
                if should_quit.load(Ordering::Relaxed) {
                    break;
                }

                // Poll for events with timeout
                if let Ok(true) = event::poll(Duration::from_millis(100)) {
                    if let Ok(Event::Key(key_event)) = event::read() {
                        if key_event.kind == KeyEventKind::Press {
                            match key_event.code {
                                KeyCode::Char('q') => {
                                    should_quit.store(true, Ordering::Relaxed);
                                }
                                KeyCode::Up | KeyCode::Char('j') => {
                                    selected_index.fetch_sub(1, Ordering::Relaxed);
                                }
                                KeyCode::Down | KeyCode::Char('k') => {
                                    selected_index.fetch_add(1, Ordering::Relaxed);
                                }
                                KeyCode::Char(' ') => {
                                    let current = is_paused.load(Ordering::Relaxed);
                                    is_paused.store(!current, Ordering::Relaxed);
                                }
                                KeyCode::Char('+') | KeyCode::Char('=') => {
                                    let current_vol =
                                        f32::from_bits(volume.load(Ordering::Relaxed));
                                    let new_vol = (current_vol + 0.01).min(1.0);
                                    volume.store(new_vol.to_bits(), Ordering::Relaxed);
                                }
                                KeyCode::Char('-') | KeyCode::Char('_') => {
                                    let current_vol =
                                        f32::from_bits(volume.load(Ordering::Relaxed));
                                    let new_vol = (current_vol - 0.01).max(0.0);
                                    volume.store(new_vol.to_bits(), Ordering::Relaxed);
                                }
                                KeyCode::Enter => {
                                    enter_pressed.store(true, Ordering::Relaxed);
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        });

        self.handle = Some(handle);
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit.load(Ordering::Relaxed)
    }

    pub fn stop(self) {
        // Signal the thread to stop
        self.should_quit.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle {
            let _ = handle.join();
        }
    }
}

