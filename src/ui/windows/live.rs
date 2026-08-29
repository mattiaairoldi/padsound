use eframe::egui;

use crate::command::Command;
use crate::config::{PlaybackMode, TrackConfig};
use crate::runtime::PadsoundRuntime;
use crate::state::{AppState, LearnKind, LearnRequest};

use super::PadsoundGui;

impl PadsoundGui {
    pub(super) fn show_running(&mut self, ui: &mut egui::Ui) {
        let mut close_show = false;
        let Some(runtime) = self.runtime.as_ref() else {
            return;
        };

        Self::handle_keyboard(runtime, ui);
        let app_state = runtime.app_state();
        let config = app_state.config();
        let states = app_state.runtime_state();
        let pending_learn = app_state.pending_learn();
        let active_count = states.iter().filter(|state| state.is_playing).count();
        let command_tx = runtime.command_sender();

        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.heading(config.name.as_deref().unwrap_or("Padsound show"));
                    ui.label(format!(
                        "Audio: {} · {} Hz · {} ch",
                        runtime.audio_info().device_name,
                        runtime.audio_info().sample_rate,
                        runtime.audio_info().channels
                    ));
                    ui.label(format!(
                        "MIDI: {} · Playing: {}",
                        runtime.midi_device_name().unwrap_or("disabled / unavailable"),
                        active_count
                    ));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_sized(
                            [150.0, 48.0],
                            egui::Button::new(
                                egui::RichText::new("STOP ALL").strong().size(18.0),
                            )
                            .fill(egui::Color32::from_rgb(145, 36, 36)),
                        )
                        .clicked()
                    {
                        let _ = command_tx.send(Command::StopAll);
                    }
                    if ui.button("Close show").clicked() {
                        close_show = true;
                    }
                });
            });

            if let Some(request) = pending_learn.as_ref() {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!(
                            "MIDI learn armed: {} / {}",
                            request.track_id,
                            learn_kind_label(request.kind)
                        ))
                        .strong(),
                    );
                    if ui.button("Cancel learn").clicked() {
                        app_state.cancel_learn();
                    }
                });
            }

            ui.separator();
            egui::ScrollArea::vertical().show(ui, |ui| {
                let available_width = ui.available_width().max(320.0);
                let columns: usize = if available_width >= 1120.0 {
                    4
                } else if available_width >= 820.0 {
                    3
                } else if available_width >= 540.0 {
                    2
                } else {
                    1
                };
                let spacing = 12.0;
                let card_width = ((available_width
                    - spacing * columns.saturating_sub(1) as f32)
                    / columns as f32)
                    .max(250.0);

                egui::Grid::new("padsound-track-grid")
                    .num_columns(columns)
                    .spacing([spacing, spacing])
                    .show(ui, |ui| {
                        for (index, track) in config.tracks.iter().enumerate() {
                            let state = states.iter().find(|state| state.track_id == track.id);
                            let is_playing = state.map(|state| state.is_playing).unwrap_or(false);
                            let volume = state.map(|state| state.volume).unwrap_or(track.volume);
                            let position = state
                                .map(|state| state.position_seconds)
                                .unwrap_or_default();
                            let duration = runtime
                                .audio_info()
                                .tracks
                                .iter()
                                .find(|info| info.id == track.id)
                                .map(|info| info.duration_seconds)
                                .unwrap_or_default();

                            egui::Frame::group(ui.style()).show(ui, |ui| {
                                ui.set_min_width(card_width - 8.0);
                                show_track_card(
                                    ui,
                                    track,
                                    is_playing,
                                    volume,
                                    position,
                                    duration,
                                    &command_tx,
                                    &app_state,
                                );
                            });

                            if (index + 1) % columns == 0 {
                                ui.end_row();
                            }
                        }
                    });
            });
        });

        if close_show {
            let _ = command_tx.send(Command::StopAll);
            self.runtime = None;
        }
    }

    fn handle_keyboard(runtime: &PadsoundRuntime, ui: &egui::Ui) {
        let events = ui.input(|input| input.events.clone());
        if events.is_empty() {
            return;
        }
        let config = runtime.app_state().config();
        let command_tx = runtime.command_sender();

        for event in events {
            let egui::Event::Key {
                key,
                pressed,
                repeat,
                modifiers,
                ..
            } = event
            else {
                continue;
            };
            if modifiers.ctrl || modifiers.alt || modifiers.command {
                continue;
            }
            let label = key_label(key);
            for track in config
                .tracks
                .iter()
                .filter(|track| track.key.as_deref() == Some(label.as_str()))
            {
                let command = match (track.mode, pressed, repeat) {
                    (PlaybackMode::Toggle, true, false) => Some(Command::Toggle {
                        track_id: track.id.clone(),
                    }),
                    (PlaybackMode::Hold, true, false) => Some(Command::HoldStart {
                        track_id: track.id.clone(),
                    }),
                    (PlaybackMode::Hold, false, _) => Some(Command::HoldEnd {
                        track_id: track.id.clone(),
                    }),
                    _ => None,
                };
                if let Some(command) = command {
                    let _ = command_tx.send(command);
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn show_track_card(
    ui: &mut egui::Ui,
    track: &TrackConfig,
    is_playing: bool,
    current_volume: f32,
    position: f64,
    duration: f64,
    command_tx: &crossbeam_channel::Sender<Command>,
    app_state: &AppState,
) {
    let action = if is_playing { "STOP" } else { "PLAY" };
    let fill = if is_playing {
        egui::Color32::from_rgb(38, 118, 72)
    } else {
        ui.visuals().widgets.inactive.bg_fill
    };
    if ui
        .add_sized(
            [ui.available_width(), 72.0],
            egui::Button::new(
                egui::RichText::new(format!("{action}\n{}", track.name))
                    .strong()
                    .size(17.0),
            )
            .fill(fill),
        )
        .clicked()
    {
        let _ = command_tx.send(Command::Toggle {
            track_id: track.id.clone(),
        });
    }

    ui.label(format!(
        "{} / {}{}",
        format_time(position),
        format_time(duration),
        if track.looping { " · LOOP" } else { "" }
    ));

    let mut volume = current_volume;
    let volume_label = format!("Volume {}%", (volume * 100.0).round() as u32);
    if ui
        .add(
            egui::Slider::new(&mut volume, 0.0..=1.0)
                .text(volume_label)
                .show_value(false),
        )
        .changed()
    {
        let _ = command_tx.send(Command::SetVolume {
            track_id: track.id.clone(),
            volume,
        });
    }

    let key = track.key.as_deref().unwrap_or("—");
    let note = track
        .midi_note
        .map(|note| note.to_string())
        .unwrap_or_else(|| "—".to_string());
    let cc = track
        .midi_volume_cc
        .map(|cc| cc.to_string())
        .unwrap_or_else(|| "—".to_string());
    ui.small(format!("Key: {key} · MIDI note: {note} · CC: {cc}"));

    ui.horizontal(|ui| {
        if ui.small_button("Learn MIDI pad").clicked() {
            app_state.start_learn(LearnRequest {
                track_id: track.id.clone(),
                kind: LearnKind::Trigger,
            });
        }
        if ui.small_button("Learn volume knob").clicked() {
            app_state.start_learn(LearnRequest {
                track_id: track.id.clone(),
                kind: LearnKind::Volume,
            });
        }
    });
}

fn learn_kind_label(kind: LearnKind) -> &'static str {
    match kind {
        LearnKind::Trigger => "trigger note",
        LearnKind::Volume => "volume CC",
    }
}

fn format_time(seconds: f64) -> String {
    let seconds = seconds.max(0.0).round() as u64;
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

fn key_label(key: egui::Key) -> String {
    match key {
        egui::Key::ArrowDown => "down".to_string(),
        egui::Key::ArrowLeft => "left".to_string(),
        egui::Key::ArrowRight => "right".to_string(),
        egui::Key::ArrowUp => "up".to_string(),
        egui::Key::PageDown => "pagedown".to_string(),
        egui::Key::PageUp => "pageup".to_string(),
        egui::Key::Space => "space".to_string(),
        other => other.name().to_ascii_lowercase(),
    }
}
