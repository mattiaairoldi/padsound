use std::sync::Arc;

use crate::config::{FadeConfig, PlaybackMode};

#[derive(Debug, Clone)]
pub struct LoadedTrack {
    pub id: String,
    pub name: String,
    pub samples: Arc<[f32]>,
    pub channels: usize,
    pub sample_rate: u32,
    pub frame_count: usize,
    pub start_frame: usize,
    pub end_frame: usize,
    pub mode: PlaybackMode,
    pub looping: bool,
    pub fade_in: Option<FadeConfig>,
    pub fade_out: Option<FadeConfig>,
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
    pub manual_fade_out: Option<ManualFadeOut>,
}

#[derive(Debug, Clone)]
pub struct ManualFadeOut {
    pub elapsed_frames: usize,
    pub total_frames: usize,
}
