#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(target_os = "windows")]
mod windows_app {
    use std::path::PathBuf;

    use anyhow::Result;
    use clap::Parser;

    #[derive(Debug, Parser)]
    #[command(version, about = "Padsound graphical soundboard for Windows")]
    struct Args {
        #[arg(short, long, default_value = "show.padsound.toml")]
        config: PathBuf,

        #[arg(long, value_name = "NAME", help = "Preselect an audio output device")]
        audio_device: Option<String>,

        #[arg(long, value_name = "NAME", help = "Preselect a MIDI input device")]
        midi_device: Option<String>,

        #[arg(long, help = "Start with MIDI disabled")]
        no_midi: bool,
    }

    pub fn run() -> Result<()> {
        let args = Args::parse();
        padsound::ui::windows::run(
            args.config,
            args.audio_device,
            args.midi_device,
            args.no_midi,
        )
    }
}

#[cfg(target_os = "windows")]
fn main() {
    if let Err(error) = windows_app::run() {
        padsound::ui::windows::show_error(&format!("{error:#}"));
        std::process::exit(1);
    }
}

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("padsound-gui is currently available on Windows; use the padsound TUI on Linux.");
}
