use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Result, bail};
use clap::Parser;
use padsound::audio::engine::AudioEngine;
use padsound::audio::mixer::RuntimeTrackState;
use padsound::config::Config;
use padsound::input::midi;
use padsound::state::AppState;
use padsound::state::TrackRuntimeSpec;
use padsound::ui::{self, tui};

#[derive(Debug, Parser)]
#[command(
    version,
    about = "Padsound audio trigger for Linux theatre use",
    after_help = "\
Common commands:
  padsound
      Start with show.padsound.toml.
  padsound --config show.padsound.toml
      Start with the selected configuration file.
  padsound --check
      Validate the configuration without starting audio, keyboard, MIDI, or UI.
  padsound --generate-config-from-dir ./audio --config show.padsound.toml
      Generate a configuration from audio files in ./audio and exit.
      If show.padsound.toml already exists, Padsound stops without overwriting it.
  padsound --no-tui
      Start without the TUI, using the simple keyboard input loop.

Runtime controls:
  configured keys
      Start/stop toggle tracks or keep hold tracks active while pressed.
  x
      Stop all tracks in the TUI.
  q, Esc, Ctrl+C
      Stop everything and exit.
  local web UI
      Default address: http://127.0.0.1:34567
      Use --ui-port to choose a different port.
  MIDI
      Configured notes and CCs control track triggers and volume.
"
)]
struct Args {
    #[arg(short, long, default_value = "show.padsound.toml")]
    config: PathBuf,

    #[arg(
        long,
        value_name = "DIR",
        help = "Generate a configuration from audio files in the selected directory"
    )]
    generate_config_from_dir: Option<PathBuf>,

    #[arg(
        long,
        help = "Validate the configuration without starting audio or input"
    )]
    check: bool,

    #[arg(
        long,
        default_value = "127.0.0.1:34567",
        help = "Local web UI socket address"
    )]
    ui_addr: SocketAddr,

    #[arg(
        long,
        value_name = "PORT",
        help = "Local web UI port; overrides the port part of --ui-addr"
    )]
    ui_port: Option<u16>,

    #[arg(
        long,
        help = "Disable the terminal TUI and use the simple keyboard input loop"
    )]
    no_tui: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    if let Some(audio_dir) = &args.generate_config_from_dir {
        if args.config.exists() {
            bail!(
                "config {} already exists: move or delete it before generating a new one",
                args.config.display()
            );
        }

        let config = Config::generate_from_audio_dir(audio_dir, &args.config)?;
        config.save(&args.config)?;
        println!(
            "Generated configuration: {} tracks from {} into {}",
            config.tracks.len(),
            audio_dir.display(),
            args.config.display()
        );
        return Ok(());
    }

    let config = Config::load(&args.config)?;
    let base_dir = Config::base_dir(&args.config);

    println!(
        "Loaded configuration: {} tracks from {}",
        config.tracks.len(),
        args.config.display()
    );

    for track in &config.tracks {
        println!(
            "- {} ({}) file={} mode={:?} loop={} volume={:.2}",
            track.name,
            track.id,
            track.file.display(),
            track.mode,
            track.looping,
            track.volume
        );
    }

    if args.check {
        println!("Check complete: configuration is valid.");
        return Ok(());
    }

    let engine = AudioEngine::start(&config, &base_dir)?;
    let command_tx = engine.sender();
    let runtime_state: Arc<Mutex<Vec<RuntimeTrackState>>> =
        Arc::new(Mutex::new(engine.runtime_state()));
    let track_specs = engine
        .info()
        .tracks
        .iter()
        .map(|track| {
            (
                track.id.clone(),
                TrackRuntimeSpec {
                    frame_count: track.frame_count,
                    sample_rate: track.sample_rate,
                },
            )
        })
        .collect::<HashMap<_, _>>();
    let app_state = AppState::new(
        config.clone(),
        args.config.clone(),
        base_dir.clone(),
        command_tx.clone(),
        runtime_state.clone(),
        track_specs,
    );
    let info = engine.info();
    println!(
        "Audio started: {} - {} Hz - {} channels",
        info.device_name, info.sample_rate, info.channels
    );

    let requested_ui_addr = ui_addr(args.ui_addr, args.ui_port);
    let ui_addr = ui::serve(app_state.clone(), requested_ui_addr).await?;
    let ui_url = local_ui_url(ui_addr);
    println!("Web UI: {ui_url}");

    tokio::spawn({
        let runtime_state = runtime_state.clone();
        async move {
            loop {
                if let Ok(mut state) = runtime_state.lock() {
                    *state = engine.runtime_state();
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    });

    let midi_runtime =
        midi::start_with_learn(&config, command_tx.clone(), Some(app_state.clone()))?;
    if let Some(midi_runtime) = &midi_runtime {
        println!("MIDI active: {}", midi_runtime.device_name());
    } else {
        println!("MIDI inactive: no mapping configured or no device found.");
    }

    if args.no_tui {
        println!("Controls: use the web UI or press configured keys in the terminal.");
        println!("Exit: q, Esc, or Ctrl+C.");
        println!();
        padsound::input::keyboard::run(&config, command_tx)?;
    } else {
        println!("Opening terminal TUI. Web UI remains active for MIDI learn/config.");
        tui::run(app_state, command_tx, ui_url)?;
    }

    Ok(())
}

fn ui_addr(mut addr: SocketAddr, port: Option<u16>) -> SocketAddr {
    if let Some(port) = port {
        addr.set_port(port);
    }
    addr
}

fn local_ui_url(addr: SocketAddr) -> String {
    let host = match addr.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => Ipv4Addr::LOCALHOST.to_string(),
        IpAddr::V6(ip) if ip.is_unspecified() => "::1".to_string(),
        ip => ip.to_string(),
    };

    match addr.ip() {
        IpAddr::V6(_) => format!("http://[{host}]:{}", addr.port()),
        IpAddr::V4(_) => format!("http://{host}:{}", addr.port()),
    }
}
