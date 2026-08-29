use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, bail};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{Receiver, Sender, unbounded};

use crate::command::{Command, TrackRuntimeUpdate};
use crate::config::Config;
use crate::terminal;

use super::decoder::decode_file;
use super::mixer::{Mixer, RuntimeTrackState};
use super::track::LoadedTrack;

#[derive(Debug, Clone)]
struct CachedAudio {
    samples: Arc<[f32]>,
    channels: usize,
    sample_rate: u32,
    frame_count: usize,
}

#[derive(Debug, Clone)]
pub struct AudioEngineInfo {
    pub device_name: String,
    pub sample_rate: u32,
    pub channels: usize,
    pub tracks: Vec<TrackInfo>,
}

#[derive(Debug, Clone)]
pub struct TrackInfo {
    pub id: String,
    pub name: String,
    pub duration_seconds: f64,
    pub frame_count: usize,
    pub sample_rate: u32,
}

pub struct AudioEngine {
    command_tx: Sender<Command>,
    _stream: cpal::Stream,
    info: AudioEngineInfo,
    runtime_state: Arc<Mutex<Vec<RuntimeTrackState>>>,
}

impl AudioEngine {
    pub fn start(config: &Config, base_dir: &Path) -> Result<Self> {
        Self::start_with_device(config, base_dir, None)
    }

    pub fn start_with_device(
        config: &Config,
        base_dir: &Path,
        preferred_device: Option<&str>,
    ) -> Result<Self> {
        let host = cpal::default_host();
        let device = select_output_device(&host, preferred_device)?;
        let device_name = device
            .name()
            .unwrap_or_else(|_| "unknown audio device".to_string());
        let supported_config = device
            .default_output_config()
            .context("failed to read default audio configuration")?;
        let sample_format = supported_config.sample_format();
        let stream_config: cpal::StreamConfig = supported_config.into();
        let sample_rate = stream_config.sample_rate.0;
        let channels = stream_config.channels as usize;

        let tracks = load_tracks(config, base_dir, sample_rate, channels)?;
        let track_infos = tracks
            .iter()
            .map(|track| TrackInfo {
                id: track.id.clone(),
                name: track.name.clone(),
                duration_seconds: track.duration_seconds(),
                frame_count: track.frame_count,
                sample_rate: track.sample_rate,
            })
            .collect();

        let (command_tx, command_rx) = unbounded();
        let mixer = Mixer::new(tracks);
        let runtime_state = Arc::new(Mutex::new(mixer.runtime_state()));
        let stream = build_stream(
            &device,
            &stream_config,
            sample_format,
            channels,
            mixer,
            command_rx,
            runtime_state.clone(),
        )?;

        stream.play().context("failed to start audio stream")?;

        Ok(Self {
            command_tx,
            _stream: stream,
            info: AudioEngineInfo {
                device_name,
                sample_rate,
                channels,
                tracks: track_infos,
            },
            runtime_state,
        })
    }

    pub fn sender(&self) -> Sender<Command> {
        self.command_tx.clone()
    }

    pub fn info(&self) -> &AudioEngineInfo {
        &self.info
    }

    pub fn runtime_state(&self) -> Vec<RuntimeTrackState> {
        self.runtime_state
            .lock()
            .expect("runtime state mutex")
            .clone()
    }

    pub fn shared_runtime_state(&self) -> Arc<Mutex<Vec<RuntimeTrackState>>> {
        self.runtime_state.clone()
    }
}

pub fn output_device_names() -> Result<Vec<String>> {
    let host = cpal::default_host();
    let devices = host
        .output_devices()
        .context("failed to enumerate audio output devices")?;
    let mut names = devices
        .filter_map(|device| device.name().ok())
        .collect::<Vec<_>>();
    names.sort_by_key(|name| name.to_lowercase());
    names.dedup();
    Ok(names)
}

fn select_output_device(host: &cpal::Host, preferred_device: Option<&str>) -> Result<cpal::Device> {
    if let Some(preferred_device) = preferred_device {
        let devices = host
            .output_devices()
            .context("failed to enumerate audio output devices")?;
        for device in devices {
            if device.name().ok().as_deref() == Some(preferred_device) {
                return Ok(device);
            }
        }
        bail!("audio output device not found: {preferred_device}");
    }

    host.default_output_device()
        .context("no output audio device available")
}

fn load_tracks(
    config: &Config,
    base_dir: &Path,
    sample_rate: u32,
    channels: usize,
) -> Result<Vec<LoadedTrack>> {
    let mut tracks = Vec::with_capacity(config.tracks.len());
    let mut audio_cache = HashMap::<PathBuf, CachedAudio>::new();

    for track in &config.tracks {
        let path = track.resolved_file(base_dir);
        let decoded = if let Some(decoded) = audio_cache.get(&path) {
            decoded.clone()
        } else {
            let decoded = decode_file(&path)
                .with_context(|| format!("error loading track {}", track.id))?
                .into_output_format(sample_rate, channels)?;
            let decoded = CachedAudio {
                frame_count: decoded.frame_count(),
                channels: decoded.channels,
                sample_rate: decoded.sample_rate,
                samples: decoded.samples.into(),
            };
            audio_cache.insert(path.clone(), decoded.clone());
            decoded
        };

        let frame_count = decoded.frame_count;
        let start_frame = seconds_to_frame(track.start_at, sample_rate).min(frame_count);
        let stop_before_frames = seconds_to_frame(track.stop_before_end, sample_rate);
        let end_frame = frame_count.saturating_sub(stop_before_frames);

        if start_frame >= end_frame {
            bail!(
                "invalid offsets for track {}: start_at={} stop_before_end={}",
                track.id,
                track.start_at,
                track.stop_before_end
            );
        }

        tracks.push(LoadedTrack {
            id: track.id.clone(),
            name: track.name.clone(),
            samples: decoded.samples,
            channels: decoded.channels,
            sample_rate: decoded.sample_rate,
            frame_count,
            start_frame,
            end_frame,
            mode: track.mode,
            looping: track.looping,
            fade_in: track.fade_in,
            fade_out: track.fade_out,
            default_volume: track.volume,
        });
    }

    Ok(tracks)
}

fn seconds_to_frame(seconds: f64, sample_rate: u32) -> usize {
    (seconds * sample_rate as f64).round().max(0.0) as usize
}

pub fn runtime_update_for_track(
    track: &crate::config::TrackConfig,
    frame_count: usize,
    sample_rate: u32,
) -> Option<TrackRuntimeUpdate> {
    let start_frame = seconds_to_frame(track.start_at, sample_rate).min(frame_count);
    let stop_before_frames = seconds_to_frame(track.stop_before_end, sample_rate);
    let end_frame = frame_count.saturating_sub(stop_before_frames);
    if start_frame >= end_frame {
        return None;
    }

    Some(TrackRuntimeUpdate {
        mode: track.mode,
        looping: track.looping,
        start_frame,
        end_frame,
        fade_in: track.fade_in,
        fade_out: track.fade_out,
        default_volume: track.volume,
    })
}

#[allow(clippy::too_many_arguments)]
fn build_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    sample_format: cpal::SampleFormat,
    channels: usize,
    mixer: Mixer,
    command_rx: Receiver<Command>,
    runtime_state: Arc<Mutex<Vec<RuntimeTrackState>>>,
) -> Result<cpal::Stream> {
    match sample_format {
        cpal::SampleFormat::F32 => build_typed_stream::<f32>(
            device,
            config,
            channels,
            mixer,
            command_rx,
            runtime_state,
        ),
        cpal::SampleFormat::I16 => build_typed_stream::<i16>(
            device,
            config,
            channels,
            mixer,
            command_rx,
            runtime_state,
        ),
        cpal::SampleFormat::U16 => build_typed_stream::<u16>(
            device,
            config,
            channels,
            mixer,
            command_rx,
            runtime_state,
        ),
        other => bail!("unsupported audio sample format: {other:?}"),
    }
}

fn build_typed_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    mut mixer: Mixer,
    command_rx: Receiver<Command>,
    runtime_state: Arc<Mutex<Vec<RuntimeTrackState>>>,
) -> Result<cpal::Stream>
where
    T: cpal::SizedSample + cpal::FromSample<f32> + Send + 'static,
{
    let mut frame = vec![0.0; channels];
    let snapshot_interval_frames = (config.sample_rate.0 / 20).max(1) as usize;
    let mut frames_since_snapshot = 0usize;
    let error_callback = |error| terminal::error(format!("audio stream error: {error}"));

    device
        .build_output_stream(
            config,
            move |data: &mut [T], _| {
                write_output(
                    data,
                    channels,
                    &mut mixer,
                    &command_rx,
                    &mut frame,
                    &runtime_state,
                    &mut frames_since_snapshot,
                    snapshot_interval_frames,
                );
            },
            error_callback,
            None,
        )
        .context("failed to create audio output stream")
}

#[allow(clippy::too_many_arguments)]
fn write_output<T>(
    data: &mut [T],
    channels: usize,
    mixer: &mut Mixer,
    command_rx: &Receiver<Command>,
    frame: &mut [f32],
    runtime_state: &Arc<Mutex<Vec<RuntimeTrackState>>>,
    frames_since_snapshot: &mut usize,
    snapshot_interval_frames: usize,
) where
    T: cpal::Sample + cpal::FromSample<f32>,
{
    for command in command_rx.try_iter() {
        mixer.handle_command(command);
    }

    for output_frame in data.chunks_mut(channels) {
        mixer.mix_frame(channels, frame);
        for (sample, value) in output_frame.iter_mut().zip(frame.iter()) {
            *sample = T::from_sample(*value);
        }
    }

    *frames_since_snapshot = frames_since_snapshot.saturating_add(data.len() / channels);
    if *frames_since_snapshot >= snapshot_interval_frames {
        if let Ok(mut state) = runtime_state.try_lock() {
            mixer.update_runtime_state(&mut state);
        }
        *frames_since_snapshot = 0;
    }
}
