use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Context, Result};
use crossbeam_channel::Sender;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::terminal;

use crate::command::Command;
use crate::config::{Config, PlaybackMode};

#[derive(Debug, Clone)]
struct KeyBinding {
    track_id: String,
    mode: PlaybackMode,
}

pub fn run(config: &Config, command_tx: Sender<Command>) -> Result<()> {
    let _raw_mode = RawModeGuard::enable()?;
    let bindings = build_bindings(config);

    loop {
        if !event::poll(Duration::from_millis(50)).context("error reading keyboard input")? {
            continue;
        }

        let Event::Key(key_event) = event::read().context("error reading keyboard event")? else {
            continue;
        };

        if should_quit(key_event) {
            command_tx
                .send(Command::StopAll)
                .context("failed to send stop all")?;
            break;
        }

        let Some(key) = key_label(key_event.code) else {
            continue;
        };
        let Some(binding) = bindings.get(&key) else {
            continue;
        };

        match (binding.mode, key_event.kind) {
            (PlaybackMode::Toggle, KeyEventKind::Press) => {
                command_tx
                    .send(Command::Toggle {
                        track_id: binding.track_id.clone(),
                    })
                    .context("failed to send toggle command")?;
            }
            (PlaybackMode::Hold, KeyEventKind::Press) => {
                command_tx
                    .send(Command::HoldStart {
                        track_id: binding.track_id.clone(),
                    })
                    .context("failed to send hold start command")?;
            }
            (PlaybackMode::Hold, KeyEventKind::Release) => {
                command_tx
                    .send(Command::HoldEnd {
                        track_id: binding.track_id.clone(),
                    })
                    .context("failed to send hold end command")?;
            }
            _ => {}
        }
    }

    Ok(())
}

fn build_bindings(config: &Config) -> HashMap<String, KeyBinding> {
    config
        .tracks
        .iter()
        .filter_map(|track| {
            track.key.as_ref().map(|key| {
                (
                    key.to_lowercase(),
                    KeyBinding {
                        track_id: track.id.clone(),
                        mode: track.mode,
                    },
                )
            })
        })
        .collect()
}

fn should_quit(key_event: KeyEvent) -> bool {
    matches!(key_event.code, KeyCode::Esc)
        || matches!(key_event.code, KeyCode::Char('c') if key_event.modifiers.contains(KeyModifiers::CONTROL))
        || matches!(key_event.code, KeyCode::Char('q') if key_event.kind == KeyEventKind::Press)
}

fn key_label(code: KeyCode) -> Option<String> {
    match code {
        KeyCode::Char(' ') => Some("space".to_string()),
        KeyCode::Char(character) => Some(character.to_lowercase().to_string()),
        KeyCode::Null => None,
        KeyCode::Enter => Some("enter".to_string()),
        KeyCode::Tab => Some("tab".to_string()),
        KeyCode::BackTab => Some("backtab".to_string()),
        KeyCode::Backspace => Some("backspace".to_string()),
        KeyCode::CapsLock => Some("capslock".to_string()),
        KeyCode::ScrollLock => Some("scrolllock".to_string()),
        KeyCode::NumLock => Some("numlock".to_string()),
        KeyCode::PrintScreen => Some("printscreen".to_string()),
        KeyCode::Pause => Some("pause".to_string()),
        KeyCode::Menu => Some("menu".to_string()),
        KeyCode::KeypadBegin => Some("keypadbegin".to_string()),
        KeyCode::Media(media_key_code) => Some(format!("{media_key_code:?}").to_lowercase()),
        KeyCode::Modifier(modifier_key_code) => {
            Some(format!("{modifier_key_code:?}").to_lowercase())
        }
        KeyCode::Delete => Some("delete".to_string()),
        KeyCode::Insert => Some("insert".to_string()),
        KeyCode::Home => Some("home".to_string()),
        KeyCode::End => Some("end".to_string()),
        KeyCode::PageUp => Some("pageup".to_string()),
        KeyCode::PageDown => Some("pagedown".to_string()),
        KeyCode::Left => Some("left".to_string()),
        KeyCode::Right => Some("right".to_string()),
        KeyCode::Up => Some("up".to_string()),
        KeyCode::Down => Some("down".to_string()),
        KeyCode::F(index) => Some(format!("f{index}")),
        KeyCode::Esc => Some("esc".to_string()),
    }
}

struct RawModeGuard;

impl RawModeGuard {
    fn enable() -> Result<Self> {
        terminal::enable_raw_mode().context("failed to enable terminal raw mode")?;
        Ok(Self)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
    }
}
