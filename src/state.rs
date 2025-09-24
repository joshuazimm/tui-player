use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::sync::atomic::{AtomicU64, Ordering};
use crossterm::terminal::size;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum StateKey {
    // Playback state
    IsPlaying,
    IsPaused,
    Volume,
    CurrentTrackPath,
    CurrentTrackTitle,
    CurrentTrackArtist,
    CurrentTrackAlbum,
    NewTrackRequested,
    PlaybackPosition,
    TrackDuration,
    
    // UI state
    TerminalSize,
    SelectedFileIndex,
    Fps,
    
    // System state
    ShouldQuit,
    NavigationUp,
    NavigationDown,
    TrackSelected,
}

#[derive(Debug, Clone)]
pub enum StateValue {
    Bool(bool),
    U32(u32),
    //U64(u64),
    F32(f32),
    F64(f64),
    String(String),
    PathBuf(PathBuf),
    TerminalSize(u16, u16),
}

impl StateValue {
    pub fn as_bool(&self) -> Option<bool> {
        match self { StateValue::Bool(v) => Some(*v), _ => None, }
    }
    pub fn as_u32(&self) -> Option<u32> {
        match self { StateValue::U32(v) => Some(*v), _ => None, }
    }
    // pub fn as_u64(&self) -> Option<u64> {
    //     match self { StateValue::U64(v) => Some(*v), _ => None, }
    // }
    pub fn as_f32(&self) -> Option<f32> {
        match self { StateValue::F32(v) => Some(*v), _ => None, }
    }
    pub fn as_f64(&self) -> Option<f64> {
        match self { StateValue::F64(v) => Some(*v), _ => None, }
    }
    pub fn as_string(&self) -> Option<&String> {
        match self { StateValue::String(v) => Some(v), _ => None, }
    }
    pub fn as_pathbuf(&self) -> Option<&PathBuf> {
        match self { StateValue::PathBuf(v) => Some(v), _ => None, }
    }
    pub fn as_terminal_size(&self) -> Option<(u16, u16)> {
        match self { StateValue::TerminalSize(w, h) => Some((*w, *h)), _ => None, }
    }
}

// Simple change tracking system
#[derive(Default)]
struct ChangeTracker {
    // Track what changed since last check
    changed_keys: Vec<StateKey>,
    change_counter: AtomicU64,
}

impl ChangeTracker {
    fn mark_changed(&mut self, key: StateKey) {
        self.changed_keys.push(key);
        self.change_counter.fetch_add(1, Ordering::SeqCst);
    }
    
    fn take_changes(&mut self) -> Vec<StateKey> {
        std::mem::take(&mut self.changed_keys)
    }
    
    // fn has_changes(&self) -> bool {
    //     !self.changed_keys.is_empty()
    // }
    
    fn change_id(&self) -> u64 {
        self.change_counter.load(Ordering::SeqCst)
    }
}

#[derive(Clone)]
pub struct GlobalState {
    state: Arc<RwLock<HashMap<StateKey, StateValue>>>,
    changes: Arc<RwLock<ChangeTracker>>,
}

impl GlobalState {
    pub fn new() -> Self {
        let mut initial_state = HashMap::new();
        
        // Initialize with default values
        initial_state.insert(StateKey::IsPlaying, StateValue::Bool(false));
        initial_state.insert(StateKey::IsPaused, StateValue::Bool(false));
        initial_state.insert(StateKey::Volume, StateValue::F32(0.05));
        initial_state.insert(StateKey::PlaybackPosition, StateValue::F64(0.0));
        initial_state.insert(StateKey::TrackDuration, StateValue::F64(0.0));
        initial_state.insert(StateKey::SelectedFileIndex, StateValue::U32(0));
        initial_state.insert(StateKey::ShouldQuit, StateValue::Bool(false));
        initial_state.insert(StateKey::CurrentTrackTitle, StateValue::String("No Track".to_string()));
        initial_state.insert(StateKey::CurrentTrackArtist, StateValue::String("Unknown Artist".to_string()));
        initial_state.insert(StateKey::CurrentTrackAlbum, StateValue::String("Unknown Album".to_string()));
        initial_state.insert(StateKey::NewTrackRequested, StateValue::Bool(false));
        initial_state.insert(StateKey::Fps, StateValue::F64(0.0));

        Self {
            state: Arc::new(RwLock::new(initial_state)),
            changes: Arc::new(RwLock::new(ChangeTracker::default())),
        }
    }

    pub fn get(&self, key: &StateKey) -> Option<StateValue> {
        self.state.read().ok()?.get(key).cloned()
    }

    pub fn set(&self, key: StateKey, value: StateValue) {
        if let Ok(mut state) = self.state.write() {
            state.insert(key.clone(), value);
        }
        
        if let Ok(mut changes) = self.changes.write() {
            changes.mark_changed(key);
        }
    }
    
    pub fn take_changes(&self) -> Vec<StateKey> {
        self.changes.write().ok().map(|mut c| c.take_changes()).unwrap_or_default()
    }
    
    // pub fn has_changes(&self) -> bool {
    //     self.changes.read().ok().map(|c| c.has_changes()).unwrap_or(false)
    // }
    
    pub fn change_id(&self) -> u64 {
        self.changes.read().ok().map(|c| c.change_id()).unwrap_or(0)
    }

    // Convenience methods
    pub fn update_terminal_size(&self) {
        if let Ok((width, height)) = size() {
            let current_size = self.get(&StateKey::TerminalSize)
                .and_then(|v| v.as_terminal_size());
            
            if current_size != Some((width, height)) {
                self.set(StateKey::TerminalSize, StateValue::TerminalSize(width, height));
            }
        }
    }

    pub fn set_playing(&self, playing: bool) {
        self.set(StateKey::IsPlaying, StateValue::Bool(playing));
        self.set(StateKey::IsPaused, StateValue::Bool(!playing));
    }

    pub fn set_volume(&self, volume: f32) {
        self.set(StateKey::Volume, StateValue::F32(volume.clamp(0.0, 1.0)));
    }

    pub fn set_current_track(&self, path: PathBuf, title: Option<String>, artist: Option<String>, album: Option<String>) {
        self.set(StateKey::CurrentTrackPath, StateValue::PathBuf(path));
        
        if let Some(title) = title {
            self.set(StateKey::CurrentTrackTitle, StateValue::String(title));
        }
        if let Some(artist) = artist {
            self.set(StateKey::CurrentTrackArtist, StateValue::String(artist));
        }
        if let Some(album) = album {
            self.set(StateKey::CurrentTrackAlbum, StateValue::String(album));
        }
    }

    pub fn set_playback_position(&self, position: f64) {
        self.set(StateKey::PlaybackPosition, StateValue::F64(position));
    }

    pub fn set_new_track_requested(&self, new_track: bool) {
        self.set(StateKey::NewTrackRequested, StateValue::Bool(new_track));
    }

    pub fn set_track_duration(&self, duration: f64) {
        self.set(StateKey::TrackDuration, StateValue::F64(duration));
    }

    pub fn set_selected_file_index(&self, index: u32) {
        self.set(StateKey::SelectedFileIndex, StateValue::U32(index));
    }

    pub fn set_should_quit(&self, should_quit: bool) {
        self.set(StateKey::ShouldQuit, StateValue::Bool(should_quit));
    }

    pub fn set_fps(&self, fps: f64) {
        self.set(StateKey::Fps, StateValue::F64(fps));
    }

    // Getters
    pub fn is_playing(&self) -> bool {
        self.get(&StateKey::IsPlaying).and_then(|v| v.as_bool()).unwrap_or(false)
    }

    pub fn is_paused(&self) -> bool {
        self.get(&StateKey::IsPaused).and_then(|v| v.as_bool()).unwrap_or(false)
    }

    pub fn volume(&self) -> f32 {
        self.get(&StateKey::Volume).and_then(|v| v.as_f32()).unwrap_or(0.05)
    }

    pub fn current_track_path(&self) -> Option<PathBuf> {
        self.get(&StateKey::CurrentTrackPath).and_then(|v| v.as_pathbuf().cloned())
    }

    pub fn playback_position(&self) -> f64 {
        self.get(&StateKey::PlaybackPosition).and_then(|v| v.as_f64()).unwrap_or(0.0)
    }

    pub fn track_duration(&self) -> f64 {
        self.get(&StateKey::TrackDuration).and_then(|v| v.as_f64()).unwrap_or(0.0)
    }

    pub fn selected_file_index(&self) -> u32 {
        self.get(&StateKey::SelectedFileIndex).and_then(|v| v.as_u32()).unwrap_or(0)
    }

    pub fn should_quit(&self) -> bool {
        self.get(&StateKey::ShouldQuit).and_then(|v| v.as_bool()).unwrap_or(false)
    }
}

impl Default for GlobalState {
    fn default() -> Self {
        Self::new()
    }
}