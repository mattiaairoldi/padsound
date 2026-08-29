use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use crossbeam_channel::Sender;

use crate::audio::engine::{AudioEngine, AudioEngineInfo};
use crate::command::Command;
use crate::config::Config;
use crate::input::midi::{self, MidiRuntime};
use crate::state::{AppState, TrackRuntimeSpec};

#[derive(Debug, Clone, Default)]
pub struct RuntimeOptions {
    pub audio_device: Option<String>,
    pub midi_device: Option<String>,
    pub disable_midi: bool,
}

pub struct PadsoundRuntime {
    app_state: AppState,
    command_tx: Sender<Command>,
    audio_info: AudioEngineInfo,
    midi_device_name: Option<String>,
    _audio_engine: AudioEngine,
    _midi_runtime: Option<MidiRuntime>,
}

impl PadsoundRuntime {
    pub fn load(config_path: impl AsRef<Path>, options: RuntimeOptions) -> Result<Self> {
        let config_path = config_path.as_ref().to_path_buf();
        let config = Config::load(&config_path)?;
        Self::start(config, config_path, options)
    }

    pub fn start(
        config: Config,
        config_path: impl Into<PathBuf>,
        options: RuntimeOptions,
    ) -> Result<Self> {
        let config_path = config_path.into();
        let base_dir = Config::base_dir(&config_path);
        let audio_engine = AudioEngine::start_with_device(
            &config,
            &base_dir,
            options.audio_device.as_deref(),
        )?;
        let command_tx = audio_engine.sender();
        let runtime_state = audio_engine.shared_runtime_state();
        let track_specs = audio_engine
            .info()
            .tracks
            .iter()
            .map(|track| {
                (
                    track.id.clone(),
                    TrackRuntimeSpec {
                        frame_count: track.frame_count,
                        sample_rate: track.sample_rate,
                    },
                )
            })
            .collect::<HashMap<_, _>>();
        let app_state = AppState::new(
            config.clone(),
            config_path,
            base_dir,
            command_tx.clone(),
            runtime_state,
            track_specs,
        );

        let midi_runtime = if options.disable_midi {
            None
        } else {
            midi::start_with_learn_on_device(
                &config,
                command_tx.clone(),
                Some(app_state.clone()),
                options.midi_device.as_deref(),
            )?
        };
        let midi_device_name = midi_runtime
            .as_ref()
            .map(|runtime| runtime.device_name().to_string());
        let audio_info = audio_engine.info().clone();

        Ok(Self {
            app_state,
            command_tx,
            audio_info,
            midi_device_name,
            _audio_engine: audio_engine,
            _midi_runtime: midi_runtime,
        })
    }

    pub fn app_state(&self) -> AppState {
        self.app_state.clone()
    }

    pub fn command_sender(&self) -> Sender<Command> {
        self.command_tx.clone()
    }

    pub fn audio_info(&self) -> &AudioEngineInfo {
        &self.audio_info
    }

    pub fn midi_device_name(&self) -> Option<&str> {
        self.midi_device_name.as_deref()
    }
}

impl Drop for PadsoundRuntime {
    fn drop(&mut self) {
        let _ = self.command_tx.send(Command::StopAll);
    }
}
