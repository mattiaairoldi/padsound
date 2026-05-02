use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Result, bail};
use crossbeam_channel::Sender;
use serde::{Deserialize, Serialize};

use crate::audio::mixer::RuntimeTrackState;
use crate::command::Command;
use crate::config::{
    Config, FadeConfig, PlaybackMode, set_track_midi_note, set_track_midi_volume_cc,
};

#[derive(Debug, Clone)]
pub struct AppState {
    inner: Arc<AppStateInner>,
}

#[derive(Debug)]
struct AppStateInner {
    config: Mutex<Config>,
    config_path: PathBuf,
    base_dir: PathBuf,
    command_tx: Sender<Command>,
    runtime_state: Arc<Mutex<Vec<RuntimeTrackState>>>,
    track_specs: HashMap<String, TrackRuntimeSpec>,
    learn: Mutex<Option<LearnRequest>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LearnRequest {
    pub track_id: String,
    pub kind: LearnKind,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LearnKind {
    Trigger,
    Volume,
}

#[derive(Debug, Clone)]
pub struct TrackRuntimeSpec {
    pub frame_count: usize,
    pub sample_rate: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TrackConfigUpdate {
    pub track_id: String,
    pub name: String,
    pub key: Option<String>,
    pub mode: PlaybackMode,
    pub looping: bool,
    pub start_at: f64,
    pub stop_before_end: f64,
    pub fade_in: Option<FadeConfig>,
    pub fade_out: Option<FadeConfig>,
    pub volume: f32,
    pub midi_note: Option<u8>,
    pub midi_volume_cc: Option<u8>,
}

impl AppState {
    pub fn new(
        config: Config,
        config_path: PathBuf,
        base_dir: PathBuf,
        command_tx: Sender<Command>,
        runtime_state: Arc<Mutex<Vec<RuntimeTrackState>>>,
        track_specs: HashMap<String, TrackRuntimeSpec>,
    ) -> Self {
        Self {
            inner: Arc::new(AppStateInner {
                config: Mutex::new(config),
                config_path,
                base_dir,
                command_tx,
                runtime_state,
                track_specs,
                learn: Mutex::new(None),
            }),
        }
    }

    pub fn config(&self) -> Config {
        self.inner.config.lock().expect("config mutex").clone()
    }

    pub fn config_path(&self) -> &Path {
        &self.inner.config_path
    }

    pub fn base_dir(&self) -> &Path {
        &self.inner.base_dir
    }

    pub fn command_tx(&self) -> Sender<Command> {
        self.inner.command_tx.clone()
    }

    pub fn runtime_state(&self) -> Vec<RuntimeTrackState> {
        self.inner
            .runtime_state
            .lock()
            .expect("runtime state mutex")
            .clone()
    }

    pub fn start_learn(&self, request: LearnRequest) {
        *self.inner.learn.lock().expect("learn mutex") = Some(request);
    }

    pub fn cancel_learn(&self) -> Option<LearnRequest> {
        self.inner.learn.lock().expect("learn mutex").take()
    }

    pub fn pending_learn(&self) -> Option<LearnRequest> {
        self.inner.learn.lock().expect("learn mutex").clone()
    }

    pub fn finish_learn_note(&self, note: u8) -> Result<Option<LearnRequest>> {
        let Some(request) = self.inner.learn.lock().expect("learn mutex").take() else {
            return Ok(None);
        };

        if request.kind != LearnKind::Trigger {
            *self.inner.learn.lock().expect("learn mutex") = Some(request);
            return Ok(None);
        }

        {
            let mut config = self.inner.config.lock().expect("config mutex");
            set_track_midi_note(&mut config, &request.track_id, note)?;
            config.validate(&self.inner.base_dir, true)?;
            config.save(&self.inner.config_path)?;
        }

        Ok(Some(request))
    }

    pub fn finish_learn_cc(&self, cc: u8) -> Result<Option<LearnRequest>> {
        let Some(request) = self.inner.learn.lock().expect("learn mutex").take() else {
            return Ok(None);
        };

        if request.kind != LearnKind::Volume {
            *self.inner.learn.lock().expect("learn mutex") = Some(request);
            return Ok(None);
        }

        {
            let mut config = self.inner.config.lock().expect("config mutex");
            set_track_midi_volume_cc(&mut config, &request.track_id, cc)?;
            config.validate(&self.inner.base_dir, true)?;
            config.save(&self.inner.config_path)?;
        }

        Ok(Some(request))
    }

    pub fn update_track_config(&self, update: TrackConfigUpdate) -> Result<()> {
        let runtime_update = {
            let mut current_config = self.inner.config.lock().expect("config mutex");
            let mut config = current_config.clone();
            let Some(track_index) = config
                .tracks
                .iter()
                .position(|track| track.id == update.track_id)
            else {
                bail!("track not found: {}", update.track_id);
            };

            {
                let track = &mut config.tracks[track_index];
                track.name = update.name;
                track.key = normalize_optional_string(update.key);
                track.mode = update.mode;
                track.looping = update.looping;
                track.start_at = update.start_at;
                track.stop_before_end = update.stop_before_end;
                track.fade_in = normalize_fade(update.fade_in);
                track.fade_out = normalize_fade(update.fade_out);
                track.volume = update.volume;
                track.midi_note = update.midi_note;
                track.midi_volume_cc = update.midi_volume_cc;
            }

            config.validate(&self.inner.base_dir, true)?;

            let track = &config.tracks[track_index];
            let Some(spec) = self.inner.track_specs.get(&track.id) else {
                bail!("runtime track metadata not found: {}", track.id);
            };
            let runtime_update = crate::audio::engine::runtime_update_for_track(
                track,
                spec.frame_count,
                spec.sample_rate,
            )
            .ok_or_else(|| anyhow::anyhow!("invalid runtime offsets for track {}", track.id))?;

            config.save(&self.inner.config_path)?;
            *current_config = config;
            runtime_update
        };

        self.inner
            .command_tx
            .send(Command::UpdateTrackRuntime {
                track_id: update.track_id,
                update: runtime_update,
            })
            .map_err(|error| anyhow::anyhow!("failed to send runtime track update: {error}"))?;

        Ok(())
    }
}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn normalize_fade(value: Option<FadeConfig>) -> Option<FadeConfig> {
    value.filter(|fade| fade.seconds > 0.0)
}
