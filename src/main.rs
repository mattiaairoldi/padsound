use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::Parser;
use padsound::audio::engine::output_device_names;
use padsound::config::Config;
use padsound::input::midi::input_device_names;
use padsound::runtime::{PadsoundRuntime, RuntimeOptions};
use padsound::ui::tui;

#[derive(Debug, Parser)]
#[command(
    version,
    about = "Padsound audio trigger for live theatre use",
    after_help = "\
Common commands:
  padsound
      Start with show.padsound.toml.
  padsound --config show.padsound.toml
      Start with the selected configuration file.
  padsound --check
      Validate the configuration without starting audio, keyboard, MIDI, or TUI.
  padsound --list-devices
      List audio outputs and MIDI inputs, then exit.
  padsound --generate-config-from-dir ./audio --config show.padsound.toml
      Generate a configuration from audio files in ./audio and exit.
  padsound --no-tui
      Start without the TUI, using the simple keyboard input loop.

Runtime controls:
  configured keys
      Start/stop toggle tracks or keep hold tracks active while pressed.
  x
      Stop all tracks in the TUI.
  q, Esc, Ctrl+C
      Stop everything and exit.
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
        help = "Disable the terminal TUI and use the simple keyboard input loop"
    )]
    no_tui: bool,

    #[arg(long, help = "List audio and MIDI devices, then exit")]
    list_devices: bool,

    #[arg(long, value_name = "NAME", help = "Use a specific audio output device")]
    audio_device: Option<String>,

    #[arg(long, value_name = "NAME", help = "Use a specific MIDI input device")]
    midi_device: Option<String>,

    #[arg(long, help = "Disable MIDI input")]
    no_midi: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    if args.list_devices {
        print_devices()?;
        return Ok(());
    }

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

    let runtime = PadsoundRuntime::start(
        config,
        args.config,
        RuntimeOptions {
            audio_device: args.audio_device,
            midi_device: args.midi_device,
            disable_midi: args.no_midi,
        },
    )?;
    let info = runtime.audio_info();
    println!(
        "Audio started: {} - {} Hz - {} channels",
        info.device_name, info.sample_rate, info.channels
    );

    if let Some(device_name) = runtime.midi_device_name() {
        println!("MIDI active: {device_name}");
    } else {
        println!("MIDI inactive: disabled, unmapped, or no device found.");
    }

    let command_tx = runtime.command_sender();
    if args.no_tui {
        println!("Controls: press configured keys in the terminal.");
        println!("Exit: q, Esc, or Ctrl+C.");
        println!();
        let config = runtime.app_state().config();
        padsound::input::keyboard::run(&config, command_tx)?;
    } else {
        println!("Opening terminal TUI.");
        tui::run(runtime.app_state(), command_tx)?;
    }

    Ok(())
}

fn print_devices() -> Result<()> {
    println!("Audio outputs:");
    let audio_devices = output_device_names()?;
    if audio_devices.is_empty() {
        println!("  (none)");
    } else {
        for device in audio_devices {
            println!("  {device}");
        }
    }

    println!();
    println!("MIDI inputs:");
    let midi_devices = input_device_names()?;
    if midi_devices.is_empty() {
        println!("  (none)");
    } else {
        for device in midi_devices {
            println!("  {device}");
        }
    }

    Ok(())
}
