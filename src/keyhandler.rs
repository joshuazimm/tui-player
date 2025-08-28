use std::sync::{Arc, atomic::{AtomicBool, AtomicU32, Ordering}};
use std::thread;
use std::time::Duration;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};

pub struct KeyHandler {
    pub should_quit: Arc<AtomicBool>,
    pub is_paused: Arc<AtomicBool>,
    pub volume: Arc<AtomicU32>,
    handle: Option<thread::JoinHandle<()>>,
}

impl KeyHandler {
    pub fn new(is_paused: Arc<AtomicBool>, volume: Arc<AtomicU32>) -> Self {
        Self {
            should_quit: Arc::new(AtomicBool::new(false)),
            is_paused,
            volume,
            handle: None,
        }
    }
    
    pub fn start(&mut self) {
        let should_quit = Arc::clone(&self.should_quit);
        let is_paused = Arc::clone(&self.is_paused);
        let volume = Arc::clone(&self.volume);
        
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
                                KeyCode::Left | KeyCode::Char('p') => {
                                    // Previous track - TODO: implement playlist navigation
                                }
                                KeyCode::Right | KeyCode::Char('n') => {
                                    // Next track - TODO: implement playlist navigation  
                                }
                                KeyCode::Char(' ') => {
                                    let current = is_paused.load(Ordering::Relaxed);
                                    is_paused.store(!current, Ordering::Relaxed);
                                }
                                KeyCode::Char('+') | KeyCode::Char('=') => {
                                    let current_vol = f32::from_bits(volume.load(Ordering::Relaxed));
                                    let new_vol = (current_vol + 0.01).min(1.0);
                                    volume.store(new_vol.to_bits(), Ordering::Relaxed);
                                }
                                KeyCode::Char('-') | KeyCode::Char('_') => {
                                    let current_vol = f32::from_bits(volume.load(Ordering::Relaxed));
                                    let new_vol = (current_vol - 0.01).max(0.0);
                                    volume.store(new_vol.to_bits(), Ordering::Relaxed);
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