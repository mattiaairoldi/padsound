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
use crate::config::{Config, PlaybackMode, TrackConfig};
use crate::state::{AppState, LearnKind, LearnRequest, TrackConfigUpdate};

const VOLUME_STEP: f32 = 0.05;
const MAX_START_INPUT_CHARS: usize = 7;

#[derive(Debug, Clone)]
struct KeyBinding {
    track_id: String,
    mode: PlaybackMode,
}

#[derive(Debug, Default)]
struct TuiState {
    selected: usize,
    fullscreen: bool,
    midi_mode: bool,
    edit_mode: bool,
    start_input: Option<StartInput>,
    message: Option<TuiMessage>,
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

#[derive(Debug, Clone)]
struct StartInput {
    track_id: String,
    text: String,
    pristine: bool,
}

impl StartInput {
    fn from_track(track: &TrackConfig) -> Self {
        Self {
            track_id: track.id.clone(),
            text: format_seconds(track.start_at),
            pristine: true,
        }
    }

    fn push_digit(&mut self, digit: char) {
        self.prepare_for_edit();
        if self.text.len() >= MAX_START_INPUT_CHARS || self.decimal_digits() >= 2 {
            return;
        }
        self.text.push(digit);
    }

    fn push_decimal_separator(&mut self) {
        self.prepare_for_edit();
        if self.text.contains('.') || self.text.len() >= MAX_START_INPUT_CHARS {
            return;
        }
        if self.text.is_empty() {
            self.text.push('0');
        }
        self.text.push('.');
    }

    fn backspace(&mut self) {
        self.prepare_for_edit();
        self.text.pop();
    }

    fn seconds(&self) -> Option<f64> {
        if self.text.is_empty() {
            return Some(0.0);
        }
        self.text.parse::<f64>().ok().filter(|value| *value >= 0.0)
    }

    fn display_value(&self) -> String {
        if self.text.is_empty() {
            "00.00".to_string()
        } else {
            self.text.clone()
        }
    }

    fn prepare_for_edit(&mut self) {
        if self.pristine {
            self.text.clear();
            self.pristine = false;
        }
    }

    fn decimal_digits(&self) -> usize {
        self.text
            .split_once('.')
            .map(|(_, decimals)| decimals.len())
            .unwrap_or(0)
    }
}

#[derive(Debug, Clone)]
struct TuiMessage {
    text: String,
    is_error: bool,
}

pub fn run(app_state: AppState, command_tx: Sender<Command>) -> Result<()> {
    let mut terminal = setup_terminal()?;
    let mut held_keys = HashSet::new();
    let mut tui_state = TuiState::default();
    let result = run_loop(
        &mut terminal,
        app_state,
        command_tx,
        &mut held_keys,
        &mut tui_state,
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
) -> Result<()> {
    loop {
        let config = app_state.config();
        let bindings = build_bindings(&config);
        tui_state.clamp_selection(config.tracks.len());
        draw(terminal, &config, &app_state, tui_state)?;

        if !event::poll(Duration::from_millis(80)).context("error reading keyboard input")? {
            continue;
        }

        let Event::Key(key_event) = event::read().context("error reading keyboard event")? else {
            continue;
        };

        if tui_state.start_input.is_some()
            && handle_start_input_key(key_event, &config, &app_state, tui_state)?
        {
            continue;
        }

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
    tui_state: &TuiState,
) -> Result<()> {
    let runtime_state = app_state.runtime_state();
    let pending_learn = app_state.pending_learn();
    terminal.draw(|frame| {
        let area = frame.area();
        let table_area = if tui_state.fullscreen {
            area
        } else {
            let chunks = Layout::vertical([Constraint::Length(11), Constraint::Min(5)]).split(area);

            let header = Paragraph::new(vec![
                Line::styled(
                    "Padsound",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Line::from(format!("Config: {}", app_state.config_path().display())),
                Line::from(
                    "Select: Up/Down/PgUp/PgDn/Home/End. Enter = toggle. Left/Right = volume.",
                ),
                Line::from("f = full screen. n = edit mode. In edit mode: r = repeat/single, s = start time."),
                Line::from(
                    "m = MIDI mode/cancel learn. In MIDI mode: k = trigger note, v = volume knob.",
                ),
                mode_line(tui_state),
                status_line(tui_state),
                learn_line(config, pending_learn.as_ref()),
            ])
            .block(Block::default().borders(Borders::ALL));
            frame.render_widget(header, chunks[0]);
            chunks[1]
        };

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
                Cell::from(format_seconds(track.start_at)),
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
                Constraint::Percentage(24),
                Constraint::Length(8),
                Constraint::Length(8),
                Constraint::Length(6),
                Constraint::Length(8),
                Constraint::Length(16),
                Constraint::Length(8),
                Constraint::Length(16),
            ],
        )
        .header(
            Row::new([
                "Key", "Track", "Status", "Mode", "Loop", "Start", "Volume", "Time", "MIDI",
            ])
            .style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
        )
        .block(
            Block::default()
                .title(table_title(config, pending_learn.as_ref(), tui_state))
                .borders(Borders::ALL),
        )
        .highlight_symbol(">> ")
        .row_highlight_style(
            Style::default()
                .fg(Color::White)
                .bg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        );

        let mut table_state = TableState::default();
        table_state.select(Some(tui_state.selected));
        frame.render_stateful_widget(table, table_area, &mut table_state);
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

    if handle_volume_key(key_event, config, app_state, command_tx, tui_state.selected)? {
        return Ok(true);
    }

    if key_event.kind != KeyEventKind::Press {
        return Ok(false);
    }

    match key_event.code {
        KeyCode::Char('f') | KeyCode::Char('F') => {
            tui_state.fullscreen = !tui_state.fullscreen;
            Ok(true)
        }
        KeyCode::Char('n') | KeyCode::Char('N') => {
            toggle_edit_mode(app_state, tui_state);
            Ok(true)
        }
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
        KeyCode::Char('r') | KeyCode::Char('R') if tui_state.edit_mode => {
            toggle_selected_loop(config, app_state, tui_state);
            Ok(true)
        }
        KeyCode::Char('s') | KeyCode::Char('S') if tui_state.edit_mode => {
            start_start_input(config, tui_state);
            Ok(true)
        }
        _ => Ok(false),
    }
}

fn handle_start_input_key(
    key_event: KeyEvent,
    config: &Config,
    app_state: &AppState,
    tui_state: &mut TuiState,
) -> Result<bool> {
    if matches!(key_event.code, KeyCode::Char('c'))
        && key_event.modifiers.contains(KeyModifiers::CONTROL)
    {
        return Ok(false);
    }

    if !matches!(key_event.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
        return Ok(true);
    }

    match key_event.code {
        KeyCode::Esc => {
            tui_state.start_input = None;
            tui_state.message = Some(TuiMessage {
                text: "Start time edit cancelled".to_string(),
                is_error: false,
            });
        }
        KeyCode::Enter => save_start_input(config, app_state, tui_state),
        KeyCode::Backspace => {
            if let Some(input) = &mut tui_state.start_input {
                input.backspace();
            }
        }
        KeyCode::Char(character) if character.is_ascii_digit() => {
            if let Some(input) = &mut tui_state.start_input {
                input.push_digit(character);
            }
        }
        KeyCode::Char('.') | KeyCode::Char(',') => {
            if let Some(input) = &mut tui_state.start_input {
                input.push_decimal_separator();
            }
        }
        _ => {}
    }

    Ok(true)
}

fn toggle_edit_mode(app_state: &AppState, tui_state: &mut TuiState) {
    if tui_state.edit_mode {
        tui_state.edit_mode = false;
        tui_state.start_input = None;
        tui_state.message = None;
    } else {
        tui_state.edit_mode = true;
        tui_state.midi_mode = false;
        tui_state.start_input = None;
        app_state.cancel_learn();
    }
}

fn toggle_midi_mode(app_state: &AppState, tui_state: &mut TuiState) {
    if tui_state.midi_mode {
        tui_state.midi_mode = false;
        app_state.cancel_learn();
    } else {
        tui_state.midi_mode = true;
        tui_state.edit_mode = false;
        tui_state.start_input = None;
        tui_state.message = None;
    }
}

fn handle_volume_key(
    key_event: KeyEvent,
    config: &Config,
    app_state: &AppState,
    command_tx: &Sender<Command>,
    selected: usize,
) -> Result<bool> {
    if !matches!(key_event.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
        return Ok(false);
    }

    let delta = match key_event.code {
        KeyCode::Left => -VOLUME_STEP,
        KeyCode::Right => VOLUME_STEP,
        _ => return Ok(false),
    };

    let Some(track) = config.tracks.get(selected) else {
        return Ok(false);
    };

    let current_volume = app_state
        .runtime_state()
        .iter()
        .find(|runtime| runtime.track_id == track.id)
        .map(|runtime| runtime.volume)
        .unwrap_or(track.volume);
    let volume = (current_volume + delta).clamp(0.0, 1.0);

    command_tx
        .send(Command::SetVolume {
            track_id: track.id.clone(),
            volume,
        })
        .context("failed to send volume command")?;

    Ok(true)
}

fn toggle_selected_loop(config: &Config, app_state: &AppState, tui_state: &mut TuiState) {
    let Some(track) = config.tracks.get(tui_state.selected) else {
        return;
    };

    let looping = !track.looping;
    let mut update = update_from_track(track);
    update.looping = looping;

    match app_state.update_track_config(update) {
        Ok(()) => {
            tui_state.message = Some(TuiMessage {
                text: format!(
                    "{} set to {}",
                    track.name,
                    if looping { "repeat" } else { "single" }
                ),
                is_error: false,
            });
        }
        Err(error) => {
            tui_state.message = Some(TuiMessage {
                text: format!("Loop update failed: {error}"),
                is_error: true,
            });
        }
    }
}

fn start_start_input(config: &Config, tui_state: &mut TuiState) {
    let Some(track) = config.tracks.get(tui_state.selected) else {
        return;
    };

    tui_state.start_input = Some(StartInput::from_track(track));
    tui_state.message = Some(TuiMessage {
        text: "Type start time as 00.00, Enter saves, Esc cancels".to_string(),
        is_error: false,
    });
}

fn save_start_input(config: &Config, app_state: &AppState, tui_state: &mut TuiState) {
    let Some(input) = tui_state.start_input.clone() else {
        return;
    };

    let Some(track) = config
        .tracks
        .iter()
        .find(|track| track.id == input.track_id)
    else {
        tui_state.message = Some(TuiMessage {
            text: "Start time update failed: track not found".to_string(),
            is_error: true,
        });
        return;
    };

    let Some(seconds) = input.seconds() else {
        tui_state.message = Some(TuiMessage {
            text: "Start time update failed: invalid number".to_string(),
            is_error: true,
        });
        return;
    };
    let mut update = update_from_track(track);
    update.start_at = seconds;

    match app_state.update_track_config(update) {
        Ok(()) => {
            tui_state.start_input = None;
            tui_state.message = Some(TuiMessage {
                text: format!(
                    "{} start time saved at {}",
                    track.name,
                    format_seconds(seconds)
                ),
                is_error: false,
            });
        }
        Err(error) => {
            tui_state.message = Some(TuiMessage {
                text: format!("Start time update failed: {error}"),
                is_error: true,
            });
        }
    }
}

fn update_from_track(track: &TrackConfig) -> TrackConfigUpdate {
    TrackConfigUpdate {
        track_id: track.id.clone(),
        name: track.name.clone(),
        key: track.key.clone(),
        mode: track.mode,
        looping: track.looping,
        start_at: track.start_at,
        stop_before_end: track.stop_before_end,
        fade_in: track.fade_in,
        fade_out: track.fade_out,
        volume: track.volume,
        midi_note: track.midi_note,
        midi_volume_cc: track.midi_volume_cc,
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

fn format_seconds(seconds: f64) -> String {
    let centiseconds = (seconds.max(0.0) * 100.0).round() as u64;
    format_centiseconds(centiseconds)
}

fn format_centiseconds(centiseconds: u64) -> String {
    format!("{:02}.{:02}", centiseconds / 100, centiseconds % 100)
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

fn table_title(
    config: &Config,
    pending_learn: Option<&LearnRequest>,
    tui_state: &TuiState,
) -> String {
    if !tui_state.fullscreen {
        return "Tracks".to_string();
    }

    let mut parts = vec![
        "Tracks".to_string(),
        "full screen".to_string(),
        "f = normal".to_string(),
    ];
    parts.push(format!("mode: {}", compact_mode(tui_state)));

    if let Some(message) = &tui_state.message {
        parts.push(message.text.clone());
    }

    if let Some(request) = pending_learn {
        let kind = match request.kind {
            LearnKind::Trigger => "trigger",
            LearnKind::Volume => "volume",
        };
        let track_name = config
            .tracks
            .iter()
            .find(|track| track.id == request.track_id)
            .map(|track| track.name.as_str())
            .unwrap_or(request.track_id.as_str());
        parts.push(format!("learn {kind}: {track_name}"));
    }

    parts.join(" | ")
}

fn compact_mode(tui_state: &TuiState) -> &'static str {
    if tui_state.start_input.is_some() {
        "start input"
    } else if tui_state.edit_mode {
        "edit"
    } else if tui_state.midi_mode {
        "midi"
    } else {
        "playback"
    }
}

fn mode_line(tui_state: &TuiState) -> Line<'static> {
    if let Some(input) = &tui_state.start_input {
        Line::styled(
            format!(
                "Mode: edit start time {} (digits, Backspace, Enter save, Esc cancel)",
                input.display_value()
            ),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
    } else if tui_state.edit_mode {
        Line::styled(
            "Mode: edit",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
    } else if tui_state.midi_mode {
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

fn status_line(tui_state: &TuiState) -> Line<'static> {
    let Some(message) = &tui_state.message else {
        return Line::styled("Status: ready", Style::default().fg(Color::DarkGray));
    };

    let style = if message.is_error {
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Green)
    };

    Line::styled(format!("Status: {}", message.text), style)
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
