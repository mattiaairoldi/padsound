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
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState};

use crate::command::Command;
use crate::config::{Config, PlaybackMode};
use crate::state::{AppState, LearnKind, LearnRequest};

#[derive(Debug, Clone)]
struct KeyBinding {
    track_id: String,
    mode: PlaybackMode,
}

#[derive(Debug, Default)]
struct TuiState {
    selected: usize,
    midi_mode: bool,
}

impl TuiState {
    fn clamp_selection(&mut self, track_count: usize) {
        if track_count == 0 {
            self.selected = 0;
        } else if self.selected >= track_count {
            self.selected = track_count - 1;
        }
    }
}

pub fn run(app_state: AppState, command_tx: Sender<Command>, ui_url: String) -> Result<()> {
    let mut terminal = setup_terminal()?;
    let mut held_keys = HashSet::new();
    let mut tui_state = TuiState::default();
    let result = run_loop(
        &mut terminal,
        app_state,
        command_tx,
        &mut held_keys,
        &mut tui_state,
        &ui_url,
    );
    restore_terminal(&mut terminal)?;
    result
}

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app_state: AppState,
    command_tx: Sender<Command>,
    held_keys: &mut HashSet<String>,
    tui_state: &mut TuiState,
    ui_url: &str,
) -> Result<()> {
    loop {
        let config = app_state.config();
        let bindings = build_bindings(&config);
        tui_state.clamp_selection(config.tracks.len());
        draw(terminal, &config, &app_state, ui_url, tui_state)?;

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

        if handle_tui_key(key_event, &config, &app_state, &command_tx, tui_state)? {
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
    ui_url: &str,
    tui_state: &TuiState,
) -> Result<()> {
    let runtime_state = app_state.runtime_state();
    let pending_learn = app_state.pending_learn();
    terminal.draw(|frame| {
        let area = frame.area();
        let chunks = Layout::vertical([Constraint::Length(9), Constraint::Min(5)]).split(area);

        let header = Paragraph::new(vec![
            Line::styled(
                "Padsound",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Line::from(format!("Web UI: {ui_url}")),
            Line::from(format!("Config: {}", app_state.config_path().display())),
            Line::from("Select: Up/Down/PgUp/PgDn/Home/End. Enter = toggle selected."),
            Line::from(
                "m = MIDI mode/cancel learn. In MIDI mode: k = trigger note, v = volume knob.",
            ),
            mode_line(tui_state.midi_mode),
            learn_line(config, pending_learn.as_ref()),
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
                Cell::from(volume_bar(volume)),
                Cell::from(position),
                Cell::from(midi_label(track.midi_note, track.midi_volume_cc)),
            ])
            .style(style)
        });

        let table = Table::new(
            rows,
            [
                Constraint::Length(6),
                Constraint::Percentage(30),
                Constraint::Length(8),
                Constraint::Length(8),
                Constraint::Length(6),
                Constraint::Length(18),
                Constraint::Length(8),
                Constraint::Length(16),
            ],
        )
        .header(
            Row::new([
                "Key", "Track", "Status", "Mode", "Loop", "Volume", "Time", "MIDI",
            ])
            .style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
        )
        .block(Block::default().title("Tracks").borders(Borders::ALL))
        .highlight_symbol(">> ")
        .row_highlight_style(
            Style::default()
                .fg(Color::White)
                .bg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        );

        let mut table_state = TableState::default();
        table_state.select(Some(tui_state.selected));
        frame.render_stateful_widget(table, chunks[1], &mut table_state);
    })?;

    Ok(())
}

fn handle_tui_key(
    key_event: KeyEvent,
    config: &Config,
    app_state: &AppState,
    command_tx: &Sender<Command>,
    tui_state: &mut TuiState,
) -> Result<bool> {
    if matches!(key_event.code, KeyCode::Char('x') | KeyCode::Char('X'))
        && key_event.kind == KeyEventKind::Press
    {
        command_tx
            .send(Command::StopAll)
            .context("failed to send stop all")?;
        return Ok(true);
    }

    if handle_selection_key(key_event, config.tracks.len(), tui_state) {
        return Ok(true);
    }

    if key_event.kind != KeyEventKind::Press {
        return Ok(false);
    }

    match key_event.code {
        KeyCode::Char('m') | KeyCode::Char('M') => {
            toggle_midi_mode(app_state, tui_state);
            Ok(true)
        }
        KeyCode::Enter => {
            let Some(track) = config.tracks.get(tui_state.selected) else {
                return Ok(false);
            };
            command_tx
                .send(Command::Toggle {
                    track_id: track.id.clone(),
                })
                .context("failed to send toggle command")?;
            Ok(true)
        }
        KeyCode::Char('k') | KeyCode::Char('K') if tui_state.midi_mode => {
            start_midi_learn(config, app_state, tui_state.selected, LearnKind::Trigger)
        }
        KeyCode::Char('v') | KeyCode::Char('V') if tui_state.midi_mode => {
            start_midi_learn(config, app_state, tui_state.selected, LearnKind::Volume)
        }
        _ => Ok(false),
    }
}

fn toggle_midi_mode(app_state: &AppState, tui_state: &mut TuiState) {
    if tui_state.midi_mode {
        tui_state.midi_mode = false;
        app_state.cancel_learn();
    } else {
        tui_state.midi_mode = true;
    }
}

fn start_midi_learn(
    config: &Config,
    app_state: &AppState,
    selected: usize,
    kind: LearnKind,
) -> Result<bool> {
    let Some(track) = config.tracks.get(selected) else {
        return Ok(false);
    };

    app_state.start_learn(LearnRequest {
        track_id: track.id.clone(),
        kind,
    });
    Ok(true)
}

fn handle_selection_key(key_event: KeyEvent, track_count: usize, tui_state: &mut TuiState) -> bool {
    if track_count == 0 || !matches!(key_event.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
        return false;
    }

    match key_event.code {
        KeyCode::Up => tui_state.selected = tui_state.selected.saturating_sub(1),
        KeyCode::Down => tui_state.selected = (tui_state.selected + 1).min(track_count - 1),
        KeyCode::PageUp => tui_state.selected = tui_state.selected.saturating_sub(10),
        KeyCode::PageDown => tui_state.selected = (tui_state.selected + 10).min(track_count - 1),
        KeyCode::Home => tui_state.selected = 0,
        KeyCode::End => tui_state.selected = track_count - 1,
        _ => return false,
    }

    true
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

fn volume_bar(volume: f32) -> String {
    let clamped = volume.clamp(0.0, 1.0);
    let width = 10;
    let filled = (clamped * width as f32).round() as usize;
    format!(
        "[{}{}] {:.2}",
        "#".repeat(filled),
        "-".repeat(width - filled),
        clamped
    )
}

fn midi_label(note: Option<u8>, cc: Option<u8>) -> String {
    let note = note
        .map(|note| note.to_string())
        .unwrap_or_else(|| "-".to_string());
    let cc = cc
        .map(|cc| cc.to_string())
        .unwrap_or_else(|| "-".to_string());
    format!("N:{note} CC:{cc}")
}

fn mode_line(midi_mode: bool) -> Line<'static> {
    if midi_mode {
        Line::styled(
            "Mode: MIDI learn",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Line::styled("Mode: playback", Style::default().fg(Color::DarkGray))
    }
}

fn learn_line(config: &Config, pending_learn: Option<&LearnRequest>) -> Line<'static> {
    let Some(request) = pending_learn else {
        return Line::styled("MIDI learn: idle", Style::default().fg(Color::DarkGray));
    };

    let kind = match request.kind {
        LearnKind::Trigger => "trigger note",
        LearnKind::Volume => "volume knob",
    };
    let track_name = config
        .tracks
        .iter()
        .find(|track| track.id == request.track_id)
        .map(|track| track.name.as_str())
        .unwrap_or(request.track_id.as_str());

    Line::styled(
        format!("MIDI learn: waiting for {kind} on {track_name}"),
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )
}
