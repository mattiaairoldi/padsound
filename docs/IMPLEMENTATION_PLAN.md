# Windows GUI implementation plan

This plan keeps the Linux TUI operational while adding Windows as a second
compile-time target over the same Rust engine.

## Constraints

- keep `main` and tag `v0.2.2` as the known Linux baseline;
- develop on `feature/windows-gui`;
- do not create a permanent Windows-only code branch;
- keep show files portable between Linux and Windows;
- do not introduce DJ, waveform, EQ or stem features;
- require a real hardware test before calling the Windows build stable.

## Milestone 1 — shared runtime

Status: implemented on the feature branch.

- extract audio, application-state and MIDI startup from `main.rs`;
- retain the current TUI as the default executable;
- add optional exact-name selection for audio and MIDI devices;
- preserve current TOML schema and playback behaviour.

Acceptance criteria:

- existing Linux commands still work;
- Linux tests pass unchanged;
- no duplicated mixer or MIDI implementation exists.

## Milestone 2 — compile-time frontend selection

Status: implemented on the feature branch.

- add `tui` and `windows-gui` Cargo features;
- build `padsound` for the terminal frontend;
- build `padsound-gui` for the Windows frontend;
- keep frontend-specific modules behind `cfg`/feature gates.

Acceptance criteria:

- Linux TUI builds with default features;
- Windows GUI builds with only `windows-gui` enabled;
- the two binaries use the same configuration, engine, mixer and MIDI code.

## Milestone 3 — minimal Windows operational GUI

Status: implemented, pending CI and hardware verification.

- create one large PLAY/STOP control per configured track;
- show playback position, volume, loop, key and MIDI mapping;
- provide per-track volume controls;
- preserve configured keyboard and MIDI triggers;
- provide an always-visible STOP ALL control;
- show the active audio and MIDI device;
- provide show-file drag-and-drop and explicit device selection;
- expose MIDI learn for trigger notes and volume CCs.

Acceptance criteria:

- four simultaneous tracks can be controlled independently;
- MiniLab notes toggle tracks and CCs change volume;
- closing the window stops and releases audio resources;
- startup failures are visible in a native Windows error dialog.

## Milestone 4 — automated validation and packages

Status: implemented, pending first CI run.

- run formatting, Clippy and tests on Linux;
- compile, lint and test the Windows GUI independently;
- extend tagged releases with a Windows ZIP while preserving the Linux archive.

Acceptance criteria:

- both CI jobs pass on the pull request;
- future tags produce both platform packages.

## Milestone 5 — hardware validation

Status: manual, not automatable in GitHub Actions.

Test on the intended theatre PC:

1. start with the real `.padsound.toml` and MP3 files;
2. verify the chosen Windows output and sample format;
3. verify Arturia MiniLab note and CC messages;
4. run multiple tracks together for an extended period;
5. exercise fade, loop, hold, toggle and STOP ALL;
6. test startup with missing/renamed devices;
7. test suspend, USB disconnect and reconnect behaviour;
8. keep VirtualDJ or another known player available during initial field trials.

The Windows target should remain pre-release until this checklist is completed.

## Deferred hardening

These changes are deliberately separate from the first port so they can be
reviewed and tested without enlarging the Windows MVP:

- add master headroom and clipping indication;
- persist machine-specific device preferences outside the show TOML;
- add a native browse dialog in addition to path entry and drag-and-drop;
- handle hot-plug/reconnect for audio and MIDI devices;
- add paging and search tools for unusually large shows;
- add installer, icon resources and optional code signing.
