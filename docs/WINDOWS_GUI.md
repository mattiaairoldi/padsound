# Windows GUI implementation plan

This branch introduces a Windows graphical frontend while retaining the Linux
TUI and a single shared Rust audio/MIDI engine.

## Branch strategy

- `main` and tag `v0.2.2` preserve the existing Linux release;
- implementation is isolated on `feature/windows-gui`;
- the branch is intended to be merged after Linux and Windows CI both pass;
- after merge, platform differences remain compile-time Cargo features rather
  than long-lived divergent branches.

## Compile-time targets

The repository remains one Cargo package:

- `padsound`: existing TUI binary, enabled by the default `tui` feature;
- `padsound-gui`: Windows soundboard, enabled by `windows-gui`;
- `PadsoundRuntime`: common ownership of audio, MIDI and application state;
- decoder, mixer, configuration and command model: shared unchanged across
  frontends.

Build commands:

```bash
# Linux TUI
cargo build --release --locked --bin padsound
```

```powershell
# Windows GUI
cargo build --release --locked --no-default-features --features windows-gui --bin padsound-gui
```

## Implemented MVP scope

### Shared runtime

- selected or default CPAL audio output;
- selected or first available MIDI input;
- optional MIDI disable;
- shared `AppState` and command channel;
- deterministic resource lifetime and `STOP ALL` on shutdown.

### Realtime audio change

The audio callback now owns the mixer directly. The UI reads a separate runtime
snapshot updated non-blockingly by the callback. This removes the previous
mutex contention where a UI-state read could make the callback emit a silent
buffer.

### Windows setup screen

- show file path with drag-and-drop;
- audio-device selector;
- MIDI-device selector and disable option;
- device refresh;
- visible startup/configuration errors.

### Windows live screen

- responsive grid of cue cards;
- large PLAY/STOP button per track;
- per-track volume slider;
- elapsed and total time;
- loop, keyboard and MIDI mapping status;
- MIDI learn for trigger note and volume CC;
- keyboard trigger handling;
- permanent global `STOP ALL`;
- close-show action returning to setup.

## Deliberately excluded from the MVP

- waveform editing;
- DJ decks, beat analysis, EQ or stems;
- plugin hosting;
- advanced routing or multiple physical outputs;
- playlists or media-library management;
- installer and code signing;
- automatic recovery after physical device removal.

## Validation gates

Automated:

1. formatting;
2. Linux compile, Clippy and unit tests;
3. Windows GUI compile, Clippy and shared unit tests;
4. release-mode builds for both binaries.

Manual hardware validation:

1. MP3/WAV playback on the intended Windows PC;
2. selected theatre audio interface and stereo routing;
3. four or more simultaneous tracks;
4. Arturia MiniLab MKII pad note learn and trigger;
5. MiniLab knob CC learn in the configured absolute/relative mode;
6. repeated start/stop, fades and `STOP ALL`;
7. unplug/replug behaviour recorded as a known limitation if recovery is not
   yet implemented.
