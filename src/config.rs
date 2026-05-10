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
                let id = unique_track_id(&name, &mut used_ids);
                let file = path_for_config(&file, &base_dir)?;

                Ok(TrackConfig {
                    id,
                    name,
                    file,
                    key: Some(keys[index].to_string()),
                    mode: PlaybackMode::Toggle,
                    looping: false,
                    start_at: 0.0,
                    stop_before_end: 0.0,
                    fade_in: None,
                    fade_out: None,
                    volume: 1.0,
                    midi_note: None,
                    midi_volume_cc: None,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(Self {
            schema_version: default_schema_version(),
            name: audio_dir
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .filter(|name| !name.trim().is_empty()),
            midi_volume_mode: MidiVolumeMode::default(),
            tracks,
        })
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            schema_version: default_schema_version(),
            name: None,
            midi_volume_mode: MidiVolumeMode::default(),
            tracks: Vec::new(),
        }
    }
}

impl TrackConfig {
    pub fn resolved_file(&self, base_dir: &Path) -> PathBuf {
        if self.file.is_absolute() {
            self.file.clone()
        } else {
            base_dir.join(&self.file)
        }
    }
}

pub fn set_track_midi_note(config: &mut Config, track_id: &str, note: u8) -> Result<()> {
    if !config.tracks.iter().any(|track| track.id == track_id) {
        bail!("track not found: {}", track_id);
    }

    for track in &mut config.tracks {
        if track.id == track_id {
            track.midi_note = Some(note);
        } else if track.midi_note == Some(note) {
            track.midi_note = None;
        }
    }

    Ok(())
}

pub fn set_track_midi_volume_cc(config: &mut Config, track_id: &str, cc: u8) -> Result<()> {
    if !config.tracks.iter().any(|track| track.id == track_id) {
        bail!("track not found: {}", track_id);
    }

    for track in &mut config.tracks {
        if track.id == track_id {
            track.midi_volume_cc = Some(cc);
        } else if track.midi_volume_cc == Some(cc) {
            track.midi_volume_cc = None;
        }
    }

    Ok(())
}

fn default_schema_version() -> u32 {
    1
}

fn default_volume() -> f32 {
    1.0
}

fn audio_files_in(audio_dir: &Path) -> Result<Vec<PathBuf>> {
    if !audio_dir.is_dir() {
        bail!("audio directory not found: {}", audio_dir.display());
    }

    let mut files = Vec::new();
    for entry in fs::read_dir(audio_dir)
        .with_context(|| format!("failed to read directory {}", audio_dir.display()))?
    {
        let entry =
            entry.with_context(|| format!("failed to read entry in {}", audio_dir.display()))?;
        let path = entry.path();
        if path.is_file() && is_supported_audio_file(&path) {
            files.push(path);
        }
    }

    Ok(files)
}

fn is_supported_audio_file(path: &Path) -> bool {
    let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
        return false;
    };

    matches!(
        extension.to_lowercase().as_str(),
        "wav" | "wave" | "mp3" | "flac" | "ogg" | "opus" | "aiff" | "aif" | "aac" | "m4a"
    )
}

fn default_generated_keys() -> Vec<&'static str> {
    vec![
        "1", "2", "3", "4", "5", "6", "7", "8", "9", "0", "w", "e", "r", "t", "y", "u", "i", "o",
        "p", "a", "s", "d", "f", "g", "h", "j", "k", "l", "z", "c", "v", "b", "n", "m",
    ]
}

fn unique_track_id(name: &str, used_ids: &mut HashSet<String>) -> String {
    let mut id = sanitize_track_id(name);
    if id.is_empty() {
        id = "track".to_string();
    }

    let original = id.clone();
    let mut suffix = 2;
    while !used_ids.insert(id.clone()) {
        id = format!("{original}_{suffix}");
        suffix += 1;
    }

    id
}

fn sanitize_track_id(name: &str) -> String {
    let mut id = String::new();
    let mut last_was_separator = false;

    for character in name.chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            id.push(character);
            last_was_separator = false;
        } else if !last_was_separator && !id.is_empty() {
            id.push('_');
            last_was_separator = true;
        }
    }

    id.trim_matches('_').to_string()
}

fn path_for_config(path: &Path, base_dir: &Path) -> Result<PathBuf> {
    let path = absolute_path(path)?;
    let base_dir = absolute_path(base_dir)?;

    Ok(path
        .strip_prefix(base_dir)
        .map(Path::to_path_buf)
        .unwrap_or(path)
        .to_path_buf())
}

fn absolute_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }

    Ok(std::env::current_dir()
        .context("failed to determine current directory")?
        .join(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn parses_valid_config_without_file_check() {
        let config: Config = toml::from_str(
            r#"
                [[tracks]]
                id = "intro"
                name = "Intro"
                file = "audio/intro.wav"
                key = "1"
                mode = "toggle"

                [[tracks]]
                id = "drone"
                name = "Drone"
                file = "audio/drone.wav"
                midi_note = 37
                mode = "hold"
                looping = true
                fade_in = { seconds = 1.0, curve = "linear" }
                fade_out = { seconds = 2.0, curve = "equal_power" }
                volume = 0.5
            "#,
        )
        .expect("config should parse");

        config
            .validate(Path::new("."), false)
            .expect("config should validate");
        assert_eq!(config.tracks.len(), 2);
        assert_eq!(config.tracks[0].mode, PlaybackMode::Toggle);
        assert_eq!(config.tracks[1].mode, PlaybackMode::Hold);
        assert_eq!(config.tracks[1].fade_out.expect("fade out").seconds, 2.0);
    }

    #[test]
    fn rejects_duplicate_track_ids() {
        let config: Config = toml::from_str(
            r#"
                [[tracks]]
                id = "same"
                name = "One"
                file = "one.wav"
                key = "1"
                mode = "toggle"

                [[tracks]]
                id = "same"
                name = "Two"
                file = "two.wav"
                key = "2"
                mode = "toggle"
            "#,
        )
        .expect("config should parse");

        let error = config
            .validate(Path::new("."), false)
            .expect_err("config should fail");

        assert!(error.to_string().contains("duplicate"));
    }

    #[test]
    fn learned_midi_note_moves_existing_conflict() {
        let mut config: Config = toml::from_str(
            r#"
                [[tracks]]
                id = "one"
                name = "One"
                file = "one.wav"
                key = "1"
                mode = "toggle"
                midi_note = 36

                [[tracks]]
                id = "two"
                name = "Two"
                file = "two.wav"
                key = "2"
                mode = "toggle"
            "#,
        )
        .expect("config should parse");

        set_track_midi_note(&mut config, "two", 36).expect("learn should update");

        assert_eq!(config.tracks[0].midi_note, None);
        assert_eq!(config.tracks[1].midi_note, Some(36));
    }

    #[test]
    fn generates_config_from_audio_dir_in_filename_order() {
        let temp_dir = test_temp_dir("ordered");
        fs::create_dir_all(&temp_dir).expect("temp dir should be created");
        fs::write(temp_dir.join("b.mp3"), []).expect("file should be written");
        fs::write(temp_dir.join("a.wav"), []).expect("file should be written");
        fs::write(temp_dir.join("ignored.txt"), []).expect("file should be written");

        let config =
            Config::generate_from_audio_dir(&temp_dir, temp_dir.join("show.padsound.toml"))
                .expect("config should generate");

        assert_eq!(config.tracks.len(), 2);
        assert_eq!(config.tracks[0].name, "a");
        assert_eq!(config.tracks[0].key.as_deref(), Some("1"));
        assert_eq!(config.tracks[0].mode, PlaybackMode::Toggle);
        assert!(!config.tracks[0].looping);
        assert_eq!(config.tracks[1].name, "b");
        assert_eq!(config.tracks[1].key.as_deref(), Some("2"));

        fs::remove_dir_all(temp_dir).expect("temp dir should be removed");
    }

    #[test]
    fn generated_ids_are_sanitized_and_unique() {
        let temp_dir = test_temp_dir("ids");
        fs::create_dir_all(&temp_dir).expect("temp dir should be created");
        fs::write(temp_dir.join("My Track!.wav"), []).expect("file should be written");
        fs::write(temp_dir.join("My Track?.mp3"), []).expect("file should be written");

        let config =
            Config::generate_from_audio_dir(&temp_dir, temp_dir.join("show.padsound.toml"))
                .expect("config should generate");

        assert_eq!(config.tracks[0].id, "my_track");
        assert_eq!(config.tracks[1].id, "my_track_2");

        fs::remove_dir_all(temp_dir).expect("temp dir should be removed");
    }

    #[test]
    fn generated_paths_remain_valid_when_config_is_elsewhere() {
        let temp_dir = test_temp_dir("paths");
        let audio_dir = temp_dir.join("audio");
        let config_dir = temp_dir.join("configs");
        fs::create_dir_all(&audio_dir).expect("audio dir should be created");
        fs::create_dir_all(&config_dir).expect("config dir should be created");
        fs::write(audio_dir.join("intro.wav"), []).expect("file should be written");

        let config =
            Config::generate_from_audio_dir(&audio_dir, config_dir.join("show.padsound.toml"))
                .expect("config should generate");

        assert!(config.tracks[0].file.is_absolute());

        fs::remove_dir_all(temp_dir).expect("temp dir should be removed");
    }

    fn test_temp_dir(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("padsound_config_test_{label}_{unique}"))
    }
}
