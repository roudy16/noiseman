# Changelog

All notable changes to noiseman. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and
versions follow [Semantic Versioning](https://semver.org/).

## [0.1.1] - 2026-11-25

### Added

- The window takes `s` to stop the player, `+`/`=` and `-` for the volume, and Tab/Shift-Tab to cycle the noise,
  like the terminal UI. The browser demo also takes `+`/`=` and `-`.
- `noiseman --version` (or `-V`, or `version`) prints the version.
- The protocol has its own semver version, 0.1.0. Commands, the terminal UI and the window warn when the running
  player's protocol is incompatible, and say to run `noiseman stop` to restart it.

### Changed

- The noises are now ordered Brown, White, Blue, Pink, Violet, so keys `1`-`5` pick them in that order
  (they were White, Pink, Brown, Blue, Violet). Brown stays the default.
- One key map everywhere: `↑`/`k` picks the next noise and `↓`/`j` the previous one, in the terminal UI, the window
  and the browser demo (all three had it the other way round).
- The window and the browser demo are redesigned to look like the terminal UI: logo, Playing/Muted status, numbered
  noise row, volume bar and key hints, coloured by the current noise, with a full-width Play/Mute button. The window
  is slightly larger (328x146).
- The browser demo says Muted/Mute instead of Paused/Pause, and remembers the noise by name.
- Easier-to-read key hints at the bottom of the terminal UI.
- The README is now a short quick start, with a troubleshooting section; build instructions moved to CONTRIBUTING.md.
- Release archives and the installers are now the terminal build (commands and terminal UI). The window ships as a
  second build, the `noiseman-gui-<target>` archives, which the installers fetch with `NOISEMAN_GUI=1`. In the gui
  build, the Linux desktop entry and the Windows Start menu shortcut open the window; in the terminal build, they open
  the terminal UI in a terminal. If you use the window, reinstall with `NOISEMAN_GUI=1`.
- `just install` builds the terminal version, as in 0.1.0; the new `just install-gui` builds the one with the window.
- When its install directory is not on PATH, `install.sh` prints the line that adds it for zsh, bash or fish.
- The macOS archives state macOS 11 as their minimum (the x86_64 one said 10.12).

### Fixed

- The installers stop a running player before replacing the binary, so an upgrade no longer leaves the old version
  playing.
- `noiseman help` says the window needs the gui build instead of `--features gui`, and lists the noises in the new
  order, as does the desktop entry.
- `noiseman gui` in a build without the window says how to get one, instead of rejecting `gui` as an unknown command.
- `noiseman` (or `noiseman tui`) run without a terminal, as from a launcher or a pipe, says it needs one and exits,
  instead of starting the player.
- The player no longer keeps the directory it was started from in use (which could keep a drive from unmounting); it
  runs from the runtime dir.

## [0.1.0] - 2026-11-25

First release.

### Added

- White, pink, brown, blue and violet noise from a small background player, each at about the same loudness.
  Muted, the player releases the audio device and uses no CPU.
- Control from a terminal UI, from one-line commands (`noiseman play brown`, `vol +10`, `toggle`, `stop`, ...) for
  scripts and keybindings, or from a small window (with a tray icon on Linux). `nsm` is installed as a short alias.
- Linux (PulseAudio or PipeWire), macOS and Windows 10 (1803 or later) and 11, on x86_64 and ARM64.
- Installers: `curl ... | sh` for Linux and macOS, `irm ... | iex` for Windows (per user, with a Start menu shortcut
  and an entry in Settings > Apps). Prebuilt archives, with SHA-256 checksums, also work with `cargo binstall`.
- A [browser demo](https://roudy16.github.io/noiseman/) that plays the same sound, with nothing to install.

[0.1.1]: https://github.com/roudy16/noiseman/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/roudy16/noiseman/releases/tag/v0.1.0
