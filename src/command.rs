use crate::config::{FadeConfig, PlaybackMode};

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Play {
        track_id: String,
    },
    Stop {
        track_id: String,
    },
    Toggle {
        track_id: String,
    },
    HoldStart {
        track_id: String,
    },
    HoldEnd {
        track_id: String,
    },
    SetVolume {
        track_id: String,
        volume: f32,
    },
    UpdateTrackRuntime {
        track_id: String,
        update: TrackRuntimeUpdate,
    },
    StopAll,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TrackRuntimeUpdate {
    pub mode: PlaybackMode,
    pub looping: bool,
    pub start_frame: usize,
    pub end_frame: usize,
    pub fade_in: Option<FadeConfig>,
    pub fade_out: Option<FadeConfig>,
    pub default_volume: f32,
}
