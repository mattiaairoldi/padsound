use std::collections::HashMap;

use anyhow::{Context, Result, anyhow};
use crossbeam_channel::Sender;
use midir::{Ignore, MidiInput, MidiInputConnection};

use crate::command::Command;
use crate::config::{Config, MidiVolumeMode, PlaybackMode};
use crate::state::AppState;
use crate::terminal;

const RELATIVE_VOLUME_STEP: f32 = 0.02;

#[derive(Debug, Clone)]
struct NoteBinding {
    track_id: String,
    mode: PlaybackMode,
}

#[derive(Debug, Clone)]
struct VolumeBinding {
    track_id: String,
    volume: f32,
}

#[derive(Debug, Clone, Default)]
struct MidiBindings {
    note_bindings: HashMap<u8, NoteBinding>,
    cc_bindings: HashMap<u8, VolumeBinding>,
    volume_mode: MidiVolumeMode,
}

pub struct MidiRuntime {
    device_name: String,
    _connection: MidiInputConnection<()>,
}

impl MidiRuntime {
    pub fn device_name(&self) -> &str {
        &self.device_name
    }
}

pub fn start(config: &Config, command_tx: Sender<Command>) -> Result<Option<MidiRuntime>> {
    start_with_learn(config, command_tx, None)
}

pub fn start_with_learn(
    config: &Config,
    command_tx: Sender<Command>,
    app_state: Option<AppState>,
) -> Result<Option<MidiRuntime>> {
    let mut bindings = MidiBindings::from_config(config);

    if bindings.is_empty() && app_state.is_none() {
        return Ok(None);
    }

    let mut midi_in = MidiInput::new("padsound-midi").context("failed to initialize MIDI")?;
    midi_in.ignore(Ignore::None);

    let ports = midi_in.ports();
    let Some(port) = ports.first() else {
        return Ok(None);
    };

    let device_name = midi_in
        .port_name(port)
        .unwrap_or_else(|_| "unknown MIDI device".to_string());

    let connection = midi_in
        .connect(
            port,
            "padsound-midi-in",
            move |_timestamp, message, _| {
                if let Some(app_state) = app_state.as_ref() {
                    handle_message_with_app_state(message, app_state, &command_tx, &mut bindings);
                } else {
                    handle_message(message, &mut bindings, &command_tx, None);
                }
            },
            (),
        )
        .map_err(|error| anyhow!("failed to connect MIDI input: {error}"))?;

    Ok(Some(MidiRuntime {
        device_name,
        _connection: connection,
    }))
}

impl MidiBindings {
    fn from_config(config: &Config) -> Self {
        let note_bindings = config
            .tracks
            .iter()
            .filter_map(|track| {
                track.midi_note.map(|note| {
                    (
                        note,
                        NoteBinding {
                            track_id: track.id.clone(),
                            mode: track.mode,
                        },
                    )
                })
            })
            .collect();

        let cc_bindings = config
            .tracks
            .iter()
            .filter_map(|track| {
                track.midi_volume_cc.map(|controller| {
                    (
                        controller,
                        VolumeBinding {
                            track_id: track.id.clone(),
                            volume: track.volume,
                        },
                    )
                })
            })
            .collect();

        Self {
            note_bindings,
            cc_bindings,
            volume_mode: config.midi_volume_mode,
        }
    }

    fn refresh_from_config(&mut self, config: &Config) {
        let previous = self.cc_bindings.clone();
        *self = Self::from_config(config);

        for binding in self.cc_bindings.values_mut() {
            if let Some(volume) = previous
                .values()
                .find(|previous_binding| previous_binding.track_id == binding.track_id)
                .map(|previous_binding| previous_binding.volume)
            {
                binding.volume = volume;
            }
        }
    }

    fn is_empty(&self) -> bool {
        self.note_bindings.is_empty() && self.cc_bindings.is_empty()
    }
}

fn handle_message_with_app_state(
    message: &[u8],
    app_state: &AppState,
    command_tx: &Sender<Command>,
    bindings: &mut MidiBindings,
) {
    bindings.refresh_from_config(&app_state.config());
    handle_message(message, bindings, command_tx, Some(app_state));
}

fn handle_message(
    message: &[u8],
    bindings: &mut MidiBindings,
    command_tx: &Sender<Command>,
    app_state: Option<&AppState>,
) {
    if message.len() < 3 {
        return;
    }

    let status = message[0] & 0xF0;
    let data_1 = message[1];
    let data_2 = message[2];

    match status {
        0x80 => handle_note_off(data_1, &bindings.note_bindings, command_tx),
        0x90 if data_2 == 0 => handle_note_off(data_1, &bindings.note_bindings, command_tx),
        0x90 => {
            if let Some(app_state) = app_state
                && consume_note_for_learn(app_state, data_1)
            {
                return;
            }
            handle_note_on(data_1, &bindings.note_bindings, command_tx);
        }
        0xB0 => {
            if let Some(app_state) = app_state
                && consume_cc_for_learn(app_state, data_1)
            {
                return;
            }
            handle_control_change(data_1, data_2, bindings, command_tx);
        }
        _ => {}
    }
}

fn consume_note_for_learn(app_state: &AppState, note: u8) -> bool {
    match app_state.finish_learn_note(note) {
        Ok(Some(_)) => true,
        Ok(None) => false,
        Err(error) => {
            terminal::error(format!("MIDI note learn error: {error}"));
            true
        }
    }
}

fn consume_cc_for_learn(app_state: &AppState, cc: u8) -> bool {
    match app_state.finish_learn_cc(cc) {
        Ok(Some(_)) => true,
        Ok(None) => false,
        Err(error) => {
            terminal::error(format!("MIDI CC learn error: {error}"));
            true
        }
    }
}

fn handle_note_on(
    note: u8,
    note_bindings: &HashMap<u8, NoteBinding>,
    command_tx: &Sender<Command>,
) {
    let Some(binding) = note_bindings.get(&note) else {
        return;
    };

    let command = match binding.mode {
        PlaybackMode::Toggle => Command::Toggle {
            track_id: binding.track_id.clone(),
        },
        PlaybackMode::Hold => Command::HoldStart {
            track_id: binding.track_id.clone(),
        },
    };
    let _ = command_tx.send(command);
}

fn handle_note_off(
    note: u8,
    note_bindings: &HashMap<u8, NoteBinding>,
    command_tx: &Sender<Command>,
) {
    let Some(binding) = note_bindings.get(&note) else {
        return;
    };
    if binding.mode == PlaybackMode::Hold {
        let _ = command_tx.send(Command::HoldEnd {
            track_id: binding.track_id.clone(),
        });
    }
}

fn handle_control_change(
    controller: u8,
    value: u8,
    bindings: &mut MidiBindings,
    command_tx: &Sender<Command>,
) {
    let Some(binding) = bindings.cc_bindings.get_mut(&controller) else {
        return;
    };
    let volume = match bindings.volume_mode {
        MidiVolumeMode::Absolute => value as f32 / 127.0,
        MidiVolumeMode::Relative => {
            let delta = relative_volume_delta(value);
            (binding.volume + delta).clamp(0.0, 1.0)
        }
    };
    binding.volume = volume;

    let _ = command_tx.send(Command::SetVolume {
        track_id: binding.track_id.clone(),
        volume,
    });
}

fn relative_volume_delta(value: u8) -> f32 {
    let steps = if value >= 65 {
        value as i16 - 64
    } else {
        value as i16 - 65
    };
    steps as f32 * RELATIVE_VOLUME_STEP
}

#[cfg(test)]
mod tests {
    use crossbeam_channel::unbounded;

    use super::*;

    #[test]
    fn note_on_sends_toggle_for_toggle_track() {
        let (tx, rx) = unbounded();
        let mut bindings = MidiBindings {
            note_bindings: HashMap::from([(
                36,
                NoteBinding {
                    track_id: "intro".to_string(),
                    mode: PlaybackMode::Toggle,
                },
            )]),
            cc_bindings: HashMap::new(),
            volume_mode: MidiVolumeMode::Relative,
        };

        handle_message(&[0x90, 36, 100], &mut bindings, &tx, None);

        assert_eq!(
            rx.try_recv().expect("command"),
            Command::Toggle {
                track_id: "intro".to_string()
            }
        );
    }

    #[test]
    fn absolute_control_change_sends_normalized_volume() {
        let (tx, rx) = unbounded();
        let mut bindings = MidiBindings {
            note_bindings: HashMap::new(),
            cc_bindings: HashMap::from([(
                21,
                VolumeBinding {
                    track_id: "intro".to_string(),
                    volume: 1.0,
                },
            )]),
            volume_mode: MidiVolumeMode::Absolute,
        };

        handle_message(&[0xB0, 21, 64], &mut bindings, &tx, None);

        assert_eq!(
            rx.try_recv().expect("command"),
            Command::SetVolume {
                track_id: "intro".to_string(),
                volume: 64.0 / 127.0
            }
        );
    }

    #[test]
    fn relative_control_change_adjusts_current_volume() {
        let (tx, rx) = unbounded();
        let mut bindings = MidiBindings {
            note_bindings: HashMap::new(),
            cc_bindings: HashMap::from([(
                21,
                VolumeBinding {
                    track_id: "intro".to_string(),
                    volume: 0.5,
                },
            )]),
            volume_mode: MidiVolumeMode::Relative,
        };

        handle_message(&[0xB0, 21, 65], &mut bindings, &tx, None);
        handle_message(&[0xB0, 21, 64], &mut bindings, &tx, None);

        assert_eq!(
            rx.try_recv().expect("command"),
            Command::SetVolume {
                track_id: "intro".to_string(),
                volume: 0.52
            }
        );
        assert_eq!(
            rx.try_recv().expect("command"),
            Command::SetVolume {
                track_id: "intro".to_string(),
                volume: 0.5
            }
        );
    }

    #[test]
    fn bindings_reflect_current_config() {
        let mut config: Config = toml::from_str(
            r#"
                [[tracks]]
                id = "intro"
                name = "Intro"
                file = "intro.wav"
                key = "1"
                mode = "toggle"
                midi_note = 36

                [[tracks]]
                id = "drone"
                name = "Drone"
                file = "drone.wav"
                key = "2"
                mode = "hold"
            "#,
        )
        .expect("config should parse");

        let first = MidiBindings::from_config(&config);
        assert_eq!(
            first.note_bindings.get(&36).expect("note").track_id,
            "intro"
        );

        config.tracks[0].midi_note = None;
        config.tracks[1].midi_note = Some(36);

        let second = MidiBindings::from_config(&config);
        assert_eq!(
            second.note_bindings.get(&36).expect("note").track_id,
            "drone"
        );
    }
}
