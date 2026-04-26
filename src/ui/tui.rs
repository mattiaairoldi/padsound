use std::collections::{HashMap, HashSet};
use std::io::{self, Stdout};
use std::time::Duration;

use anyhow::{Context, Result};
use crossbeam_channel::Sender;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};

use crate::command::Command;
use crate::config::{Config, PlaybackMode};
use crate::state::AppState;

#[derive(Debug, Clone)]
struct KeyBinding {
    track_id: String,
    mode: PlaybackMode,
}

pub fn run(app_state: AppState, command_tx: Sender<Command>) -> Result<()> {
    let mut terminal = setup_terminal()?;
    let mut held_keys = HashSet::new();
    let result = run_loop(&mut terminal, app_state, command_tx, &mut held_keys);
    restore_terminal(&mut terminal)?;
    result
}

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app_state: AppState,
    command_tx: Sender<Command>,
    held_keys: &mut HashSet<String>,
) -> Result<()> {
    loop {
        let config = app_state.config();
        let bindings = build_bindings(&config);
        draw(terminal, &config, &app_state)?;

        if !event::poll(Duration::from_millis(80)).context("error reading keyboard input")? {
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

        if matches!(key_event.code, KeyCode::Char('x') | KeyCode::Char('X'))
            && key_event.kind == KeyEventKind::Press
        {
            command_tx
                .send(Command::StopAll)
                .context("failed to send stop all")?;
            continue;
        }

        handle_track_key(key_event, &bindings, held_keys, &command_tx)?;
    }

    Ok(())
}

fn draw(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    config: &Config,
    app_state: &AppState,
) -> Result<()> {
    let runtime_state = app_state.runtime_state();
    terminal.draw(|frame| {
        let area = frame.area();
        let chunks = Layout::vertical([Constraint::Length(4), Constraint::Min(5)]).split(area);

        let header = Paragraph::new(vec![
            Line::from("Padsound"),
            Line::from("Keys: use configured keys. x = stop all. q / Esc / Ctrl+C = exit."),
            Line::from(format!("Config: {}", app_state.config_path().display())),
        ])
        .block(Block::default().borders(Borders::ALL));
        frame.render_widget(header, chunks[0]);

        let rows = config.tracks.iter().map(|track| {
            let runtime = runtime_state
                .iter()
                .find(|runtime| runtime.track_id == track.id);
            let is_playing = runtime.map(|runtime| runtime.is_playing).unwrap_or(false);
            let position = runtime
                .map(|runtime| format_time(runtime.position_seconds))
                .unwrap_or_else(|| "0:00".to_string());
            let volume = runtime
                .map(|runtime| runtime.volume)
                .unwrap_or(track.volume);
            let status = if is_playing { "PLAY" } else { "STOP" };
            let style = if is_playing {
                Style::default()
                    .fg(Color::LightGreen)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            Row::new(vec![
                Cell::from(track.key.clone().unwrap_or_default()),
                Cell::from(track.name.clone()),
                Cell::from(status),
                Cell::from(format!("{:?}", track.mode).to_lowercase()),
                Cell::from(if track.looping { "yes" } else { "no" }),
                Cell::from(format!("{volume:.2}")),
                Cell::from(position),
            ])
            .style(style)
        });

        let table = Table::new(
            rows,
            [
                Constraint::Length(6),
                Constraint::Percentage(36),
                Constraint::Length(8),
                Constraint::Length(8),
                Constraint::Length(6),
                Constraint::Length(8),
                Constraint::Length(8),
            ],
        )
        .header(
            Row::new(["Key", "Track", "Status", "Mode", "Loop", "Volume", "Time"]).style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
        )
        .block(Block::default().title("Tracks").borders(Borders::ALL))
        .row_highlight_style(Style::default().bg(Color::DarkGray));

        frame.render_widget(table, chunks[1]);
    })?;

    Ok(())
}

fn handle_track_key(
    key_event: KeyEvent,
    bindings: &HashMap<String, KeyBinding>,
    held_keys: &mut HashSet<String>,
    command_tx: &Sender<Command>,
) -> Result<()> {
    let Some(key) = key_label(key_event.code) else {
        return Ok(());
    };
    let Some(binding) = bindings.get(&key) else {
        return Ok(());
    };

    match (binding.mode, key_event.kind) {
        (PlaybackMode::Toggle, KeyEventKind::Press) => {
            command_tx
                .send(Command::Toggle {
                    track_id: binding.track_id.clone(),
                })
                .context("failed to send toggle command")?;
        }
        (PlaybackMode::Hold, KeyEventKind::Press) if held_keys.insert(key.clone()) => {
            command_tx
                .send(Command::HoldStart {
                    track_id: binding.track_id.clone(),
                })
                .context("failed to send hold start command")?;
        }
        (PlaybackMode::Hold, KeyEventKind::Release) if held_keys.remove(&key) => {
            command_tx
                .send(Command::HoldEnd {
                    track_id: binding.track_id.clone(),
                })
                .context("failed to send hold end command")?;
        }
        _ => {}
    }

    Ok(())
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode().context("failed to enable terminal raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).context("failed to enter alternate screen")?;
    Terminal::new(CrosstermBackend::new(stdout)).context("failed to create TUI terminal")
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode().context("failed to disable terminal raw mode")?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)
        .context("failed to leave alternate screen")?;
    terminal.show_cursor().context("failed to show cursor")?;
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

fn format_time(seconds: f64) -> String {
    let safe_seconds = seconds.max(0.0).floor() as u64;
    format!("{}:{:02}", safe_seconds / 60, safe_seconds % 60)
}
