use std::net::SocketAddr;

use anyhow::{Context, Result};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;

use crate::command::Command;
use crate::state::{AppState, LearnKind, LearnRequest};
use crate::terminal;

pub mod tui;

#[derive(Debug, Clone, Serialize)]
struct UiState {
    config_path: String,
    tracks: Vec<UiTrack>,
    pending_learn: Option<LearnRequest>,
}

#[derive(Debug, Clone, Serialize)]
struct UiTrack {
    id: String,
    name: String,
    key: Option<String>,
    mode: String,
    looping: bool,
    volume: f32,
    runtime_volume: f32,
    is_playing: bool,
    position_seconds: f64,
    midi_note: Option<u8>,
    midi_volume_cc: Option<u8>,
}

#[derive(Debug, Clone, Serialize)]
struct LearnResponse {
    pending: LearnRequest,
}

#[derive(Debug, Clone, Deserialize)]
struct CommandRequest {
    action: UiAction,
    track_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum UiAction {
    Play,
    Stop,
    Toggle,
    StopAll,
}

pub async fn serve(app_state: AppState, addr: SocketAddr) -> Result<SocketAddr> {
    let router = Router::new()
        .route("/", get(index))
        .route("/api/state", get(api_state))
        .route("/api/learn", post(api_learn))
        .route("/api/command", post(api_command))
        .with_state(app_state);

    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind UI on {addr}"))?;
    let local_addr = listener.local_addr().context("failed to read UI address")?;

    tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, router).await {
            terminal::error(format!("web UI error: {error}"));
        }
    });

    Ok(local_addr)
}

async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

async fn api_state(State(app_state): State<AppState>) -> Json<UiState> {
    Json(build_ui_state(&app_state))
}

async fn api_learn(
    State(app_state): State<AppState>,
    Json(request): Json<LearnRequest>,
) -> Result<Json<LearnResponse>, UiError> {
    let config = app_state.config();
    if !config
        .tracks
        .iter()
        .any(|track| track.id == request.track_id)
    {
        return Err(UiError::bad_request(format!(
            "track not found: {}",
            request.track_id
        )));
    }

    match request.kind {
        LearnKind::Trigger | LearnKind::Volume => app_state.start_learn(request.clone()),
    }

    Ok(Json(LearnResponse { pending: request }))
}

async fn api_command(
    State(app_state): State<AppState>,
    Json(request): Json<CommandRequest>,
) -> Result<StatusCode, UiError> {
    let command = match request.action {
        UiAction::Play => Command::Play {
            track_id: required_track_id(&request)?,
        },
        UiAction::Stop => Command::Stop {
            track_id: required_track_id(&request)?,
        },
        UiAction::Toggle => Command::Toggle {
            track_id: required_track_id(&request)?,
        },
        UiAction::StopAll => Command::StopAll,
    };

    app_state
        .command_tx()
        .send(command)
        .map_err(|error| UiError::internal(format!("failed to send command: {error}")))?;

    Ok(StatusCode::NO_CONTENT)
}

fn required_track_id(request: &CommandRequest) -> Result<String, UiError> {
    request
        .track_id
        .clone()
        .ok_or_else(|| UiError::bad_request("track_id is required".to_string()))
}

fn build_ui_state(app_state: &AppState) -> UiState {
    let config = app_state.config();
    let runtime_state = app_state.runtime_state();
    let tracks = config
        .tracks
        .into_iter()
        .map(|track| {
            let runtime = runtime_state
                .iter()
                .find(|runtime| runtime.track_id == track.id);

            UiTrack {
                id: track.id,
                name: track.name,
                key: track.key,
                mode: format!("{:?}", track.mode).to_lowercase(),
                looping: track.looping,
                volume: track.volume,
                runtime_volume: runtime
                    .map(|runtime| runtime.volume)
                    .unwrap_or(track.volume),
                is_playing: runtime.map(|runtime| runtime.is_playing).unwrap_or(false),
                position_seconds: runtime
                    .map(|runtime| runtime.position_seconds)
                    .unwrap_or(0.0),
                midi_note: track.midi_note,
                midi_volume_cc: track.midi_volume_cc,
            }
        })
        .collect();

    UiState {
        config_path: app_state.config_path().display().to_string(),
        tracks,
        pending_learn: app_state.pending_learn(),
    }
}

#[derive(Debug)]
struct UiError {
    status: StatusCode,
    message: String,
}

impl UiError {
    fn bad_request(message: String) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message,
        }
    }

    fn internal(message: String) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message,
        }
    }
}

impl IntoResponse for UiError {
    fn into_response(self) -> axum::response::Response {
        (self.status, self.message).into_response()
    }
}

const INDEX_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Padsound</title>
  <style>
    :root {
      color-scheme: light dark;
      font-family: system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
      background: #111;
      color: #eee;
    }
    body {
      margin: 0;
      padding: 24px;
      background: #111;
      color: #eee;
    }
    header {
      display: flex;
      align-items: baseline;
      justify-content: space-between;
      gap: 16px;
      margin-bottom: 20px;
    }
    h1 {
      margin: 0;
      font-size: 24px;
      font-weight: 650;
    }
    .meta {
      color: #aaa;
      font-size: 13px;
    }
    table {
      width: 100%;
      border-collapse: collapse;
      background: #181818;
      border: 1px solid #333;
    }
    th, td {
      padding: 10px 12px;
      border-bottom: 1px solid #2c2c2c;
      text-align: left;
      font-size: 14px;
    }
    th {
      color: #bbb;
      font-weight: 600;
      background: #202020;
    }
    tr.playing {
      background: #183024;
    }
    tr.playing td:first-child {
      border-left: 4px solid #25d07f;
    }
    .status {
      display: inline-flex;
      align-items: center;
      gap: 6px;
      white-space: nowrap;
    }
    .dot {
      width: 9px;
      height: 9px;
      border-radius: 50%;
      background: #555;
      display: inline-block;
    }
    .dot.playing {
      background: #25d07f;
      box-shadow: 0 0 0 4px rgba(37, 208, 127, 0.12);
    }
    button {
      border: 1px solid #555;
      border-radius: 6px;
      background: #2b2b2b;
      color: #eee;
      padding: 7px 10px;
      cursor: pointer;
    }
    button:hover {
      background: #383838;
    }
    .actions {
      display: flex;
      gap: 8px;
      flex-wrap: wrap;
    }
    .toolbar {
      display: flex;
      justify-content: flex-end;
      margin: 0 0 16px;
    }
    .pending {
      margin: 0 0 16px;
      padding: 10px 12px;
      border: 1px solid #6b5f27;
      background: #2b2612;
      color: #f1df8a;
      display: none;
    }
    .hint {
      color: #aaa;
      font-size: 13px;
      margin: -8px 0 16px;
    }
  </style>
</head>
<body>
  <header>
    <h1>Padsound</h1>
    <div class="meta" id="config-path"></div>
  </header>
  <div class="pending" id="pending"></div>
  <div class="hint">Shortcuts also work from this page while the browser window has focus.</div>
  <div class="toolbar">
    <button onclick="sendCommand('stop_all')">Stop all</button>
  </div>
  <table>
    <thead>
      <tr>
        <th>Track</th>
        <th>Status</th>
        <th>Key</th>
        <th>Mode</th>
        <th>Loop</th>
        <th>Volume</th>
        <th>MIDI note</th>
        <th>MIDI CC</th>
        <th>Play</th>
        <th>Learn</th>
      </tr>
    </thead>
    <tbody id="tracks"></tbody>
  </table>
  <script>
    let currentTracks = [];
    const heldKeys = new Set();

    async function loadState() {
      const response = await fetch('/api/state');
      const state = await response.json();
      currentTracks = state.tracks;
      document.getElementById('config-path').textContent = state.config_path;

      const pending = document.getElementById('pending');
      if (state.pending_learn) {
        pending.style.display = 'block';
        pending.textContent = `Waiting for MIDI: ${state.pending_learn.kind} for ${state.pending_learn.track_id}`;
      } else {
        pending.style.display = 'none';
      }

      const rows = state.tracks.map(track => `
        <tr class="${track.is_playing ? 'playing' : ''}">
          <td>${escapeHtml(track.name)} <span class="meta">${escapeHtml(track.id)}</span></td>
          <td>
            <span class="status">
              <span class="dot ${track.is_playing ? 'playing' : ''}"></span>
              ${track.is_playing ? `Playing ${formatTime(track.position_seconds)}` : 'Stopped'}
            </span>
          </td>
          <td>${track.key ?? ''}</td>
          <td>${track.mode}</td>
          <td>${track.looping ? 'yes' : 'no'}</td>
          <td>${track.runtime_volume.toFixed(2)}</td>
          <td>${track.midi_note ?? ''}</td>
          <td>${track.midi_volume_cc ?? ''}</td>
          <td>
            <div class="actions">
              <button onclick="sendCommand('toggle', '${track.id}')">Toggle</button>
              <button onclick="sendCommand('stop', '${track.id}')">Stop</button>
            </div>
          </td>
          <td>
            <div class="actions">
              <button onclick="learn('${track.id}', 'trigger')">Trigger</button>
              <button onclick="learn('${track.id}', 'volume')">Volume</button>
            </div>
          </td>
        </tr>
      `).join('');
      document.getElementById('tracks').innerHTML = rows;
    }

    async function learn(trackId, kind) {
      await fetch('/api/learn', {
        method: 'POST',
        headers: {'content-type': 'application/json'},
        body: JSON.stringify({track_id: trackId, kind})
      });
      await loadState();
    }

    async function sendCommand(action, trackId = null) {
      await fetch('/api/command', {
        method: 'POST',
        headers: {'content-type': 'application/json'},
        body: JSON.stringify({action, track_id: trackId})
      });
    }

    document.addEventListener('keydown', event => {
      if (event.target && ['INPUT', 'TEXTAREA', 'SELECT'].includes(event.target.tagName)) {
        return;
      }

      if (event.key === 'Escape') {
        event.preventDefault();
        sendCommand('stop_all');
        return;
      }

      const key = browserKeyLabel(event);
      const track = currentTracks.find(track => track.key && track.key.toLowerCase() === key);
      if (!track) {
        return;
      }

      event.preventDefault();

      if (track.mode === 'hold') {
        if (heldKeys.has(key)) {
          return;
        }
        heldKeys.add(key);
        sendCommand('play', track.id);
        return;
      }

      if (!event.repeat) {
        sendCommand('toggle', track.id);
      }
    });

    document.addEventListener('keyup', event => {
      const key = browserKeyLabel(event);
      const track = currentTracks.find(track => track.key && track.key.toLowerCase() === key);
      if (!track || track.mode !== 'hold') {
        return;
      }

      event.preventDefault();
      heldKeys.delete(key);
      sendCommand('stop', track.id);
    });

    function browserKeyLabel(event) {
      if (event.key === ' ') {
        return 'space';
      }
      if (event.key.length === 1) {
        return event.key.toLowerCase();
      }

      const aliases = {
        ArrowLeft: 'left',
        ArrowRight: 'right',
        ArrowUp: 'up',
        ArrowDown: 'down',
        PageUp: 'pageup',
        PageDown: 'pagedown',
        Backspace: 'backspace',
        Delete: 'delete',
        Insert: 'insert',
        Escape: 'esc',
        Enter: 'enter',
        Tab: 'tab',
        Home: 'home',
        End: 'end'
      };

      return aliases[event.key] ?? event.key.toLowerCase();
    }

    function formatTime(seconds) {
      const safeSeconds = Math.max(0, Math.floor(seconds));
      const minutes = Math.floor(safeSeconds / 60);
      const rest = String(safeSeconds % 60).padStart(2, '0');
      return `${minutes}:${rest}`;
    }

    function escapeHtml(value) {
      return String(value).replace(/[&<>"']/g, char => ({
        '&': '&amp;',
        '<': '&lt;',
        '>': '&gt;',
        '"': '&quot;',
        "'": '&#039;'
      }[char]));
    }

    loadState();
    setInterval(loadState, 250);
  </script>
</body>
</html>
"#;
