# Padsound

Padsound is a small Rust application for triggering audio files quickly from a
keyboard, MIDI controller, or terminal TUI.

The current target is live theatre use on Linux.

## Platform Support

Padsound is currently developed and tested for:

- Linux;
- PipeWire audio, usually through the PulseAudio compatibility layer;
- terminal TUI usage for playback, MIDI learn, and core cue configuration.

Other platforms are not a current goal. ALSA, JACK, macOS, and Windows are not
part of the supported release target for now.

## Prerequisites

On Ubuntu/Debian-like systems, install the Rust toolchain and the native
libraries needed by the audio and MIDI stack:

```bash
sudo apt install build-essential pkg-config libasound2-dev libudev-dev
```

PipeWire should be installed and running. On most recent desktop Linux
distributions this is already the default. A practical check is:

```bash
pactl info
```

The output should show a PipeWire-backed audio server, for example
`PulseAudio (on PipeWire ...)`.

You also need Rust:

```bash
rustc --version
cargo --version
```

If Rust is missing, install it from <https://rustup.rs/>.

## Features

- TOML configuration;
- configuration validation at startup;
- automatic configuration generation from an audio directory;
- audio decoding with `symphonia`;
- audio output with `cpal`;
- multi-track mixer;
- `toggle` and `hold` playback modes;
- looping;
- fade in and fade out with selectable curves;
- start offset and stop-before-end offset;
- per-track volume;
- keyboard input;
- best-effort MIDI input on the first available device;
- MIDI notes for track triggering;
- MIDI CC for track volume;
- terminal TUI for live use;
- MIDI learn from the TUI;
- TOML configuration saving with `schema_version`.

## Usage

Try the bundled example configuration:

```bash
cargo run -- --config padsound.example.toml
```

Generate a configuration from the audio files in a directory:

```bash
cargo run -- --generate-config-from-dir ./audio --config show.padsound.toml
```

Audio files are sorted by filename. Generated tracks use `toggle` mode, volume
`1.0`, no loop, and automatic key assignment: `1`-`9`, `0`, then keyboard
letters in order.

If the target config file already exists, Padsound stops without overwriting it.
Move or delete the existing file before generating a new one.

Alternatively, create a configuration from the example:

```bash
cp padsound.example.toml show.padsound.toml
```

Edit the audio file paths, then validate the configuration:

```bash
cargo run -- --check
```

Start Padsound:

```bash
cargo run -- --config show.padsound.toml
```

If `--config` is not provided, the default is `show.padsound.toml`.

## Runtime Controls

When Padsound starts, it opens a terminal TUI showing:

- tracks;
- play/stop state;
- elapsed time;
- assigned key;
- runtime volume;
- MIDI trigger note and volume CC.

TUI controls:

- `Up`, `Down`, `PageUp`, `PageDown`, `Home`, `End`: select a track;
- `Enter`: start or stop the selected track;
- `Left`, `Right`: decrease or increase the selected track runtime volume;
- `f`: toggle full-screen table mode;
- `n`: toggle edit mode;
- `r` in edit mode: switch the selected track between repeat and single playback;
- `s` in edit mode: edit the selected track start time as `00.00` seconds
  (`.` and `,` are accepted as decimal separators);
- `m`: toggle MIDI learn mode; when leaving MIDI learn mode, cancel pending learn;
- `k` in MIDI learn mode: learn the selected track trigger note;
- `v` in MIDI learn mode: learn the selected track volume knob/CC;
- configured keys, for example `1`, `2`, `3`: trigger tracks;
- `x`: stop all tracks;
- `q`, `Esc`, or `Ctrl+C`: stop all tracks and exit.

From the TUI, MIDI learn can be started for each track:

- `Trigger`: saves the next received MIDI note to `midi_note`;
- `Volume`: saves the next received MIDI control change to `midi_volume_cc`.

TUI edit mode saves repeat and start-time changes to the TOML configuration
immediately. Runtime volume changes with `Left` and `Right` are not saved.

The configuration file is saved back as TOML. The recommended extension is
`.padsound.toml`, for example `show.padsound.toml`.

## Configuration Notes

Multiple tracks may point to the same audio file. This is useful for cue
variants: the same file can have different keyboard/MIDI triggers, offsets,
volume, loop mode, or fade settings. Padsound decodes shared audio files once at
startup and reuses the decoded audio in memory for all variants.

Fade settings are optional:

```toml
fade_in = { seconds = 1.0, curve = "linear" }
fade_out = { seconds = 2.0, curve = "equal_power" }
```

Supported fade curves are `linear`, `equal_power`, and `exponential`.

To disable the TUI and use the simpler terminal keyboard loop:

```bash
cargo run -- --no-tui
```

During execution:

- press configured keys to start or stop tracks;
- use configured MIDI pads/notes to trigger tracks;
- use configured MIDI knobs/CCs to control track volume;
- press `q`, `Esc`, or `Ctrl+C` to stop everything and exit.

## Example Assets

The WAV files in `example/` are short generated sounds included only for trying
Padsound quickly. They are part of this repository and covered by the repository
license.

## Releases

The supported release target is Linux x86_64 with PipeWire. Release packages are
created by GitHub Actions when a tag matching `v*` is pushed, for example:

```bash
git tag v0.1.0
git push origin v0.1.0
```

The package includes the `padsound` binary, README, license, example config, and
example audio files.

## License

MIT.
