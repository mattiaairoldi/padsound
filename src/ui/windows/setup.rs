use std::path::PathBuf;

use eframe::egui;

use crate::audio::engine::output_device_names;
use crate::input::midi::input_device_names;
use crate::runtime::{PadsoundRuntime, RuntimeOptions};

use super::PadsoundGui;

impl PadsoundGui {
    pub(super) fn refresh_devices(&mut self) {
        let mut errors = Vec::new();
        match output_device_names() {
            Ok(devices) => self.audio_devices = devices,
            Err(error) => errors.push(format!("Audio devices: {error:#}")),
        }
        match input_device_names() {
            Ok(devices) => self.midi_devices = devices,
            Err(error) => errors.push(format!("MIDI devices: {error:#}")),
        }
        self.error = if errors.is_empty() {
            None
        } else {
            Some(errors.join("\n"))
        };
    }

    fn open_show(&mut self) {
        let config_path = self.config_path.trim();
        if config_path.is_empty() {
            self.error = Some("Select a .padsound.toml show file.".to_string());
            return;
        }

        let options = RuntimeOptions {
            audio_device: self.selected_audio.clone(),
            midi_device: self.selected_midi.clone(),
            disable_midi: self.disable_midi,
        };
        match PadsoundRuntime::load(PathBuf::from(config_path), options) {
            Ok(runtime) => {
                self.runtime = Some(runtime);
                self.error = None;
            }
            Err(error) => self.error = Some(format!("Unable to open the show:\n{error:#}")),
        }
    }

    pub(super) fn show_setup(&mut self, ui: &mut egui::Ui) {
        self.apply_dropped_file(ui);

        egui::CentralPanel::default().show(ui, |ui| {
            ui.add_space(16.0);
            ui.heading("Padsound");
            ui.label("Minimal soundboard for theatre playback");
            ui.add_space(20.0);

            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_min_width(640.0);
                ui.label(egui::RichText::new("Show file").strong());
                ui.add(
                    egui::TextEdit::singleline(&mut self.config_path)
                        .desired_width(f32::INFINITY)
                        .hint_text("show.padsound.toml"),
                );
                ui.small("You can also drag a .padsound.toml file onto this window.");

                ui.add_space(14.0);
                ui.label(egui::RichText::new("Audio output").strong());
                let audio_label = self
                    .selected_audio
                    .as_deref()
                    .unwrap_or("Windows default output")
                    .to_string();
                let audio_devices = self.audio_devices.clone();
                egui::ComboBox::from_id_salt("audio-device")
                    .selected_text(audio_label)
                    .width(480.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.selected_audio,
                            None,
                            "Windows default output",
                        );
                        for device in audio_devices {
                            ui.selectable_value(
                                &mut self.selected_audio,
                                Some(device.clone()),
                                device,
                            );
                        }
                    });

                ui.add_space(14.0);
                ui.label(egui::RichText::new("MIDI input").strong());
                ui.checkbox(&mut self.disable_midi, "Disable MIDI");
                ui.add_enabled_ui(!self.disable_midi, |ui| {
                    let midi_label = self
                        .selected_midi
                        .as_deref()
                        .unwrap_or("First available MIDI input")
                        .to_string();
                    let midi_devices = self.midi_devices.clone();
                    egui::ComboBox::from_id_salt("midi-device")
                        .selected_text(midi_label)
                        .width(480.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.selected_midi,
                                None,
                                "First available MIDI input",
                            );
                            for device in midi_devices {
                                ui.selectable_value(
                                    &mut self.selected_midi,
                                    Some(device.clone()),
                                    device,
                                );
                            }
                        });
                });

                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    if ui.button("Refresh devices").clicked() {
                        self.refresh_devices();
                    }
                    if ui
                        .add_sized([180.0, 38.0], egui::Button::new("Open show"))
                        .clicked()
                    {
                        self.open_show();
                    }
                });
            });

            self.show_error_panel(ui);
        });
    }

    fn apply_dropped_file(&mut self, ui: &egui::Ui) {
        let dropped_files = ui.input(|input| input.raw.dropped_files.clone());
        if let Some(path) = dropped_files
            .into_iter()
            .find_map(|file| file.path().map(|path| path.to_path_buf())) {
            self.config_path = path.to_string_lossy().into_owned();
        }
    }

    fn show_error_panel(&self, ui: &mut egui::Ui) {
        if let Some(error) = &self.error {
            ui.add_space(14.0);
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.colored_label(egui::Color32::from_rgb(210, 70, 70), error);
            });
        }
    }
}
