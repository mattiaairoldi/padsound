use std::collections::HashMap;

use crate::command::{Command, TrackRuntimeUpdate};
use crate::config::{FadeConfig, FadeCurve};

use super::track::{LoadedTrack, ManualFadeOut, TrackRuntime};

#[derive(Debug, Clone)]
pub struct RuntimeTrackState {
    pub track_id: String,
    pub is_playing: bool,
    pub volume: f32,
    pub position_seconds: f64,
}

#[derive(Debug)]
pub struct Mixer {
    tracks: Vec<LoadedTrack>,
    track_indexes: HashMap<String, usize>,
    volumes: Vec<f32>,
    active: Vec<TrackRuntime>,
}

impl Mixer {
    pub fn new(tracks: Vec<LoadedTrack>) -> Self {
        let track_indexes = tracks
            .iter()
            .enumerate()
            .map(|(index, track)| (track.id.clone(), index))
            .collect();

        let volumes = tracks.iter().map(|track| track.default_volume).collect();

        Self {
            tracks,
            track_indexes,
            volumes,
            active: Vec::new(),
        }
    }

    pub fn tracks(&self) -> &[LoadedTrack] {
        &self.tracks
    }

    pub fn runtime_state(&self) -> Vec<RuntimeTrackState> {
        self.tracks
            .iter()
            .enumerate()
            .map(|(track_index, track)| {
                let active = self
                    .active
                    .iter()
                    .find(|runtime| runtime.track_index == track_index);
                let position_frame = active
                    .map(|runtime| runtime.position_frame)
                    .unwrap_or(track.start_frame);
                let volume = active
                    .map(|runtime| runtime.volume)
                    .unwrap_or(self.volumes[track_index]);

                RuntimeTrackState {
                    track_id: track.id.clone(),
                    is_playing: active.is_some(),
                    volume,
                    position_seconds: position_frame.saturating_sub(track.start_frame) as f64
                        / track.sample_rate as f64,
                }
            })
            .collect()
    }

    pub fn handle_command(&mut self, command: Command) {
        match command {
            Command::Play { track_id } | Command::HoldStart { track_id } => {
                self.play(&track_id);
            }
            Command::Stop { track_id } | Command::HoldEnd { track_id } => {
                self.stop(&track_id);
            }
            Command::Toggle { track_id } => {
                if self.is_active(&track_id) {
                    self.stop(&track_id);
                } else {
                    self.play(&track_id);
                }
            }
            Command::SetVolume { track_id, volume } => {
                self.set_volume(&track_id, volume);
            }
            Command::UpdateTrackRuntime { track_id, update } => {
                self.update_track_runtime(&track_id, update);
            }
            Command::StopAll => {
                self.active.clear();
            }
        }
    }

    pub fn mix_frame(&mut self, output_channels: usize, out: &mut [f32]) {
        out.fill(0.0);

        let mut active_index = 0;
        while active_index < self.active.len() {
            let runtime = &mut self.active[active_index];
            let track = &self.tracks[runtime.track_index];

            if runtime.position_frame >= track.end_frame {
                if track.looping {
                    runtime.position_frame = track.start_frame;
                } else {
                    self.active.swap_remove(active_index);
                    continue;
                }
            }

            let frame_offset = runtime.position_frame * track.channels;
            let fade_factor = fade_factor(track, runtime);
            for (channel, sample) in out.iter_mut().enumerate().take(output_channels) {
                let source_channel = channel.min(track.channels - 1);
                *sample +=
                    track.samples[frame_offset + source_channel] * runtime.volume * fade_factor;
            }

            runtime.position_frame += 1;
            if let Some(fade_out) = &mut runtime.manual_fade_out {
                fade_out.elapsed_frames += 1;
                if fade_out.elapsed_frames >= fade_out.total_frames {
                    self.active.swap_remove(active_index);
                    continue;
                }
            }
            active_index += 1;
        }

        for sample in out {
            *sample = sample.clamp(-1.0, 1.0);
        }
    }

    fn play(&mut self, track_id: &str) {
        let Some(&track_index) = self.track_indexes.get(track_id) else {
            return;
        };

        self.stop(track_id);

        let track = &self.tracks[track_index];
        if track.start_frame >= track.end_frame {
            return;
        }

        self.active.push(TrackRuntime {
            track_index,
            position_frame: track.start_frame,
            volume: self.volumes[track_index],
            manual_fade_out: None,
        });
    }

    fn stop(&mut self, track_id: &str) {
        let Some(&track_index) = self.track_indexes.get(track_id) else {
            return;
        };
        let track = &self.tracks[track_index];
        let fade_out_frames = fade_frames(track.fade_out, track.sample_rate);
        if fade_out_frames == 0 {
            self.active
                .retain(|runtime| runtime.track_index != track_index);
            return;
        }

        for runtime in &mut self.active {
            if runtime.track_index == track_index && runtime.manual_fade_out.is_none() {
                runtime.manual_fade_out = Some(ManualFadeOut {
                    elapsed_frames: 0,
                    total_frames: fade_out_frames,
                });
            }
        }
    }

    fn is_active(&self, track_id: &str) -> bool {
        let Some(&track_index) = self.track_indexes.get(track_id) else {
            return false;
        };
        self.active
            .iter()
            .any(|runtime| runtime.track_index == track_index)
    }

    fn set_volume(&mut self, track_id: &str, volume: f32) {
        let Some(&track_index) = self.track_indexes.get(track_id) else {
            return;
        };
        let volume = volume.clamp(0.0, 1.0);
        self.volumes[track_index] = volume;
        for runtime in &mut self.active {
            if runtime.track_index == track_index {
                runtime.volume = volume;
            }
        }
    }

    fn update_track_runtime(&mut self, track_id: &str, update: TrackRuntimeUpdate) {
        let Some(&track_index) = self.track_indexes.get(track_id) else {
            return;
        };
        let track = &mut self.tracks[track_index];
        if update.start_frame >= update.end_frame || update.end_frame > track.frame_count {
            return;
        }

        track.mode = update.mode;
        track.looping = update.looping;
        track.start_frame = update.start_frame;
        track.end_frame = update.end_frame;
        track.fade_in = update.fade_in;
        track.fade_out = update.fade_out;
        track.default_volume = update.default_volume.clamp(0.0, 1.0);
        self.volumes[track_index] = track.default_volume;

        self.active.retain_mut(|runtime| {
            if runtime.track_index != track_index {
                return true;
            }
            runtime.volume = track.default_volume;
            runtime.position_frame >= track.start_frame && runtime.position_frame < track.end_frame
        });
    }
}

fn fade_factor(track: &LoadedTrack, runtime: &TrackRuntime) -> f32 {
    let mut factor: f32 = 1.0;

    let fade_in_frames = fade_frames(track.fade_in, track.sample_rate);
    if fade_in_frames > 0 {
        let elapsed = runtime.position_frame.saturating_sub(track.start_frame);
        factor = factor.min(curve_factor(
            elapsed as f64 / fade_in_frames as f64,
            track.fade_in,
        ));
    }

    let fade_out_frames = fade_frames(track.fade_out, track.sample_rate);
    if fade_out_frames > 0 {
        let remaining = track.end_frame.saturating_sub(runtime.position_frame);
        if remaining < fade_out_frames {
            factor = factor.min(curve_factor(
                remaining as f64 / fade_out_frames as f64,
                track.fade_out,
            ));
        }
    }

    if let Some(manual_fade_out) = &runtime.manual_fade_out {
        let remaining = manual_fade_out
            .total_frames
            .saturating_sub(manual_fade_out.elapsed_frames);
        factor = factor.min(curve_factor(
            remaining as f64 / manual_fade_out.total_frames as f64,
            track.fade_out,
        ));
    }

    factor
}

fn fade_frames(fade: Option<FadeConfig>, sample_rate: u32) -> usize {
    fade.map(|fade| (fade.seconds * sample_rate as f64).round().max(0.0) as usize)
        .unwrap_or(0)
}

fn curve_factor(progress: f64, fade: Option<FadeConfig>) -> f32 {
    let progress = progress.clamp(0.0, 1.0);
    let curve = fade.map(|fade| fade.curve).unwrap_or(FadeCurve::Linear);
    match curve {
        FadeCurve::Linear => progress as f32,
        FadeCurve::EqualPower => (progress * std::f64::consts::FRAC_PI_2).sin() as f32,
        FadeCurve::Exponential => (progress * progress) as f32,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::config::PlaybackMode;

    use super::*;

    #[test]
    fn toggle_starts_and_stops_track() {
        let track = LoadedTrack {
            id: "intro".to_string(),
            name: "Intro".to_string(),
            samples: Arc::from([0.5, 0.5, 0.25, 0.25]),
            channels: 2,
            sample_rate: 48_000,
            frame_count: 2,
            start_frame: 0,
            end_frame: 2,
            mode: PlaybackMode::Toggle,
            looping: false,
            fade_in: None,
            fade_out: None,
            default_volume: 1.0,
        };
        let mut mixer = Mixer::new(vec![track]);
        let mut frame = [0.0, 0.0];

        mixer.handle_command(Command::Toggle {
            track_id: "intro".to_string(),
        });
        mixer.mix_frame(2, &mut frame);
        assert_eq!(frame, [0.5, 0.5]);

        mixer.handle_command(Command::Toggle {
            track_id: "intro".to_string(),
        });
        mixer.mix_frame(2, &mut frame);
        assert_eq!(frame, [0.0, 0.0]);
    }

    #[test]
    fn volume_change_applies_to_future_playback() {
        let track = LoadedTrack {
            id: "intro".to_string(),
            name: "Intro".to_string(),
            samples: Arc::from([1.0, 1.0]),
            channels: 2,
            sample_rate: 48_000,
            frame_count: 1,
            start_frame: 0,
            end_frame: 1,
            mode: PlaybackMode::Toggle,
            looping: false,
            fade_in: None,
            fade_out: None,
            default_volume: 1.0,
        };
        let mut mixer = Mixer::new(vec![track]);
        let mut frame = [0.0, 0.0];

        mixer.handle_command(Command::SetVolume {
            track_id: "intro".to_string(),
            volume: 0.25,
        });
        mixer.handle_command(Command::Play {
            track_id: "intro".to_string(),
        });
        mixer.mix_frame(2, &mut frame);

        assert_eq!(frame, [0.25, 0.25]);
    }

    #[test]
    fn fade_in_scales_start_of_playback() {
        let track = LoadedTrack {
            id: "intro".to_string(),
            name: "Intro".to_string(),
            samples: Arc::from([1.0, 1.0, 1.0, 1.0]),
            channels: 2,
            sample_rate: 2,
            frame_count: 2,
            start_frame: 0,
            end_frame: 2,
            mode: PlaybackMode::Toggle,
            looping: false,
            fade_in: Some(FadeConfig {
                seconds: 0.5,
                curve: FadeCurve::Linear,
            }),
            fade_out: None,
            default_volume: 1.0,
        };
        let mut mixer = Mixer::new(vec![track]);
        let mut frame = [0.0, 0.0];

        mixer.handle_command(Command::Play {
            track_id: "intro".to_string(),
        });
        mixer.mix_frame(2, &mut frame);
        assert_eq!(frame, [0.0, 0.0]);
        mixer.mix_frame(2, &mut frame);
        assert_eq!(frame, [1.0, 1.0]);
    }

    #[test]
    fn manual_stop_uses_fade_out() {
        let track = LoadedTrack {
            id: "intro".to_string(),
            name: "Intro".to_string(),
            samples: Arc::from([1.0, 1.0, 1.0, 1.0, 1.0, 1.0]),
            channels: 2,
            sample_rate: 2,
            frame_count: 3,
            start_frame: 0,
            end_frame: 3,
            mode: PlaybackMode::Toggle,
            looping: false,
            fade_in: None,
            fade_out: Some(FadeConfig {
                seconds: 1.0,
                curve: FadeCurve::Linear,
            }),
            default_volume: 1.0,
        };
        let mut mixer = Mixer::new(vec![track]);
        let mut frame = [0.0, 0.0];

        mixer.handle_command(Command::Play {
            track_id: "intro".to_string(),
        });
        mixer.handle_command(Command::Stop {
            track_id: "intro".to_string(),
        });
        mixer.mix_frame(2, &mut frame);
        assert_eq!(frame, [1.0, 1.0]);
        mixer.mix_frame(2, &mut frame);
        assert_eq!(frame, [0.5, 0.5]);
        mixer.mix_frame(2, &mut frame);
        assert_eq!(frame, [0.0, 0.0]);
    }
}
