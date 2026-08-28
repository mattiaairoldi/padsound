use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default)]
    pub midi_volume_mode: MidiVolumeMode,
    pub tracks: Vec<TrackConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TrackConfig {
    pub id: String,
    pub name: String,
    pub file: PathBuf,
    pub key: Option<String>,
    pub mode: PlaybackMode,
    #[serde(default)]
    pub looping: bool,
    #[serde(default)]
    pub start_at: f64,
    #[serde(default)]
    pub stop_before_end: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade_in: Option<FadeConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade_out: Option<FadeConfig>,
    #[serde(default = "default_volume")]
    pub volume: f32,
    pub midi_note: Option<u8>,
    pub midi_volume_cc: Option<u8>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PlaybackMode {
    Toggle,
    Hold,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum MidiVolumeMode {
    #[default]
    Relative,
    Absolute,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq)]
pub struct FadeConfig {
    pub seconds: f64,
    #[serde(default)]
    pub curve: FadeCurve,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum FadeCurve {
    #[default]
    Linear,
    EqualPower,
    Exponential,
}

impl Config {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed to read config {}", path.display()))?;
        let config: Self = toml::from_str(&raw)
            .with_context(|| format!("invalid TOML config in {}", path.display()))?;
        config.validate(path.parent().unwrap_or_else(|| Path::new(".")), true)?;
        Ok(config)
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        let raw = toml::to_string_pretty(self).context("failed to serialize TOML config")?;
        fs::write(path, raw).with_context(|| format!("failed to write {}", path.display()))?;
        Ok(())
    }

    pub fn validate(&self, base_dir: &Path, check_files: bool) -> Result<()> {
        if self.schema_version != default_schema_version() {
            bail!(
                "unsupported schema_version: {} (supported: {})",
                self.schema_version,
                default_schema_version()
            );
        }
        if self.tracks.is_empty() {
            bail!("configuration must contain at least one track");
        }

        let mut ids = HashSet::new();
        let mut keys = HashSet::new();
        let mut midi_notes = HashSet::new();
        let mut midi_ccs = HashSet::new();

        for track in &self.tracks {
            let label = if track.id.trim().is_empty() {
                "<empty id>"
            } else {
                track.id.as_str()
            };

            if track.id.trim().is_empty() {
                bail!("a track has an empty id");
            }
            if !ids.insert(track.id.as_str()) {
                bail!("duplicate track id: {}", track.id);
            }
            if track.name.trim().is_empty() {
                bail!("track {} has an empty name", label);
            }
            if track.file.as_os_str().is_empty() {
                bail!("track {} has no configured file", label);
            }
            if track.start_at < 0.0 {
                bail!("track {} has a negative start_at", label);
            }
            if track.stop_before_end < 0.0 {
                bail!("track {} has a negative stop_before_end", label);
            }
            if let Some(fade_in) = track.fade_in
                && fade_in.seconds < 0.0
            {
                bail!("track {} has a negative fade_in duration", label);
            }
            if let Some(fade_out) = track.fade_out
                && fade_out.seconds < 0.0
            {
                bail!("track {} has a negative fade_out duration", label);
            }
            if !(0.0..=1.0).contains(&track.volume) {
                bail!("track {} has volume outside range 0.0-1.0", label);
            }

            if let Some(key) = &track.key {
                if key.trim().is_empty() {
                    bail!("track {} has an empty key", label);
                }
                if !keys.insert(key.as_str()) {
                    bail!("duplicate key in configuration: {}", key);
                }
            }

            if let Some(note) = track.midi_note
                && !midi_notes.insert(note)
            {
                bail!("duplicate MIDI note in configuration: {}", note);
            }

            if let Some(cc) = track.midi_volume_cc
                && !midi_ccs.insert(cc)
            {
                bail!("duplicate MIDI volume CC in configuration: {}", cc);
            }

            if track.key.is_none() && track.midi_note.is_none() {
                bail!(
                    "track {} must have at least one trigger: key or midi_note",
                    label
                );
            }

            if check_files {
                let file = if track.file.is_absolute() {
                    track.file.clone()
                } else {
                    base_dir.join(&track.file)
                };
                if !file.is_file() {
                    bail!(
                        "audio file not found for track {}: {}",
                        label,
                        file.display()
                    );
                }
            }
        }

        Ok(())
    }

    pub fn base_dir(config_path: impl AsRef<Path>) -> PathBuf {
        config_path
            .as_ref()
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf()
    }

    pub fn generate_from_audio_dir(
        audio_dir: impl AsRef<Path>,
        config_path: impl AsRef<Path>,
    ) -> Result<Self> {
        let audio_dir = audio_dir.as_ref();
        let config_path = config_path.as_ref();
        let mut files = audio_files_in(audio_dir)?;
        files.sort_by_key(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().to_lowercase())
                .unwrap_or_default()
        });

        if files.is_empty() {
            bail!("no audio files found in {}", audio_dir.display());
        }

        let keys = default_generated_keys();
        if files.len() > keys.len() {
            bail!(
                "too many audio files: {} found, {} automatic keys available",
                files.len(),
                keys.len()
            );
        }

        let base_dir = Self::base_dir(config_path);
        let mut used_ids = HashSet::new();
        let tracks = files
            .into_iter()
            .enumerate()
            .map(|(index, file)| {
                let name = file
                    .file_stem()
                    .map(|stem| stem.to_string_lossy().to_string())
                    .unwrap_or_else(|| format!("Track {}", index + 1));
                let id = unique_track_id(&name, &