# Padsound

Padsound is a small Rust application for triggering audio files quickly from a
keyboard, MIDI controller, terminal TUI, or minimal graphical soundboard.

It is designed for live theatre use: audio files are decoded when a show opens,
then played from memory with independent trigger, volume, loop, offset and fade
settings.

## Platform support

Padsound now has two frontends over the same audio/MIDI engine:

- **Linux TUI**: the existing default target, developed and tested with PipeWire
  through its PulseAudio compatibility layer;
- **Windows GUI**: an optional `egui/eframe` frontend built with the
  `windows-gui` Cargo feature and using the Windows audio backend exposed by
  CPAL.

The Linux TUI remains the default build. The Windows GUI is an MVP and must be
validated on the intended theatre PC, audio interface and MIDI controller before
live use.

## Features

- TOML show configuration and validation;
- automatic configuration generation from an audio directory;
- MP3, WAV, FLAC, OGG, Opus, AIFF, AAC and M4A decoding with `symphonia`;
- audio output with `cpal`;
- multi-track mixer with simultaneous playback;
- `toggle` and `hold` playback modes;
- looping, start offset and stop-before-end offset;
- fade in/out with linear, equal-power or exponential curves;
- per-track runtime volume;
- keyboard triggers;
- selectable MIDI input;
- MIDI notes for triggering and MIDI CC for volume;
- absolute and relative MIDI knob modes;
- MIDI learn from both the TUI and Windows GUI;
- explicit audio-device selection;
- permanent `STOP ALL` control.

## Linux prerequisites

On Ubuntu/Debian-like systems:

```bash
sudo apt install build-essential pkg-config libasound2-dev libudev-dev
```

PipeWire should be installed and running. A practical check is:

```bash
pactl info
```

Install Rust through `rustup` if needed, then verify:

```bash
rustc --version
cargo --version
```

## Linux TUI

Build and run the existing frontend:

```bash
cargo build --release --locked --bin padsound
cargo run --bin padsound -- --config padsound.example.toml
```

Generate a show configuration from a directory:

```bash
cargo run --bin padsound -- --generate-config-from-dir ./audio --config show.padsound.toml
```

List available audio and MIDI devices:

```bash
cargo run --bin padsound -- --list-devices
```

Select devices explicitly or disable MIDI:

```bash
cargo run --bin padsound -- \
  --config show.padsound.toml \
  --audio-device "Yamaha AG03" \
  --midi-device "Arturia MiniLab mkII"

cargo run --bin padsound -- --config show.padsound.toml --no-midi
```

Validate configuration without starting audio:

```bash
cargo run --bin padsound -- --config show.padsound.toml --check
```

### TUI controls

- `Up`, `Down`, `PageUp`, `PageDown`, `Home`, `End`: select a track;
- `Enter`: start or stop the selected track;
- `Left`, `Right`: change its runtime volume;
- `f`: toggle full-screen table mode;
- `n`: toggle edit mode;
- `r` in edit mode: switch repeat/single playback;
- `s` in edit mode: edit start time;
- `m`: enter or leave MIDI learn mode;
- `k` in MIDI learn mode: learn the trigger note;
- `v` in MIDI learn mode: learn the volume CC;
- configured keys: trigger tracks;
- `x`: stop all tracks;
- `q`, `Esc`, or `Ctrl+C`: stop everything and exit.

## Windows GUI

Install the current Rust MSVC toolchain and Microsoft C++ build tools, then
build:

```powershell
cargo build --release --locked --no-default-features --features windows-gui --bin padsound-gui
```

Run:

```powershell
.\target\release\padsound-gui.exe
```

The setup screen lets you:

- enter or drag-and-drop a `.padsound.toml` show file;
- choose the Windows audio output;
- choose a MIDI input or disable MIDI;
- refresh connected devices before opening the show.

The live screen provides large PLAY/STOP controls, per-track sliders, elapsed and
total time, loop and mapping status, MIDI learn, keyboard triggers and a global
`STOP ALL` button.

Optional command-line arguments preselect setup values:

```powershell
.\padsound-gui.exe `
  --config .\shows\spettacolo.padsound.toml `
  --audio-device "Yamaha AG03" `
  --midi-device "Arturia MiniLab mkII"
```

## Configuration notes

MIDI volume knobs use relative mode by default, suitable for encoders that send
values such as `64` for down and `65` for up. For knobs or faders that send
absolute values from `0` to `127`, set:

```toml
midi_volume_mode = "absolute"
```

Track fades are optional:

```toml
fade_in = { seconds = 1.0, curve = "linear" }
fade_out = { seconds = 2.0, curve = "equal_power" }
```

Multiple tracks may reference the same file. Padsound decodes that file once at
startup and reuses the samples for all cue variants.

## Release targets

Tagged releases are configured to produce:

```text
padsound-vX.Y.Z-linux-x86_64.tar.gz
padsound-vX.Y.Z-windows-x86_64.zip
```

## License

MIT.
