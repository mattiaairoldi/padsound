#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Play { track_id: String },
    Stop { track_id: String },
    Toggle { track_id: String },
    HoldStart { track_id: String },
    HoldEnd { track_id: String },
    SetVolume { track_id: String, volume: f32 },
    StopAll,
}
