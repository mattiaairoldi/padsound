use std::sync::Arc;

use crate::config::PlaybackMode;

#[derive(Debug, Clone)]
pub struct LoadedTrack {
    pub id: String,
    pub name: String,
    pub samples: Arc<[f32]>,
    pub channels: usize,
    pub sample_rate: u32,
    pub start_frame: usize,
    pub end_frame: usize,
    pub mode: PlaybackMode,
    pub looping: bool,
    pub default_volume: f32,
}

impl LoadedTrack {
    pub fn duration_seconds(&self) -> f64 {
        self.end_frame.saturating_sub(self.start_frame) as f64 / self.sample_rate as f64
    }
}

#[derive(Debug, Clone)]
pub struct TrackRuntime {
    pub track_index: usize,
    pub position_frame: usize,
    pub volume: f32,
}
