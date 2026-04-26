use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::Result;
use crossbeam_channel::Sender;
use serde::{Deserialize, Serialize};

use crate::audio::mixer::RuntimeTrackState;
use crate::command::Command;
use crate::config::{Config, set_track_midi_note, set_track_midi_volume_cc};

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

impl AppState {
    pub fn new(
        config: Config,
        config_path: PathBuf,
        base_dir: PathBuf,
        command_tx: Sender<Command>,
        runtime_state: Arc<Mutex<Vec<RuntimeTrackState>>>,
    ) -> Self {
        Self {
            inner: Arc::new(AppStateInner {
                config: Mutex::new(config),
                config_path,
                base_dir,
                command_tx,
                runtime_state,
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
}
