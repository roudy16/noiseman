# Changelog

All notable changes to noiseman. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and
versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- Issue forms for bug reports and feature requests, and a README note on support and scope; questions go to
  GitHub Discussions.

### Changed

- When another `nsm` is installed and the installers skip theirs, the warning now says how to alias `nsm` to
  noiseman in your shell anyway.

### Fixed

- Web demo: with a noise button focused, the arrow keys move between noises (Left/Up previous, Right/Down next) as a
  radio group should, instead of changing the volume.

## [0.1.2] - 2026-11-25

### Changed

- Volume changes, mute and noise changes now glide over 20 ms on an S-curve (smoothstep) and land exactly on the new
  volume, instead of an exponential fade that took about 50 ms to settle; the length is the same at every sample rate.
  A noise change fades out and back in over 40 ms.
- `install.sh` says why noiseman cannot run on a system instead of blaming the glibc version: a missing ALSA library
  (with the package to install), a glibc older than the binary needs (with both versions), musl (Alpine), NixOS
  without nix-ld, a 32-bit system, a CPU other than 64-bit x86 or ARM, or macOS older than 11. The checks that need no
  download run before it.
- Linux needs glibc 2.34, not 2.35 as stated before: the binaries never needed more, so RHEL 9 and its rebuilds,
  Amazon Linux 2023 and Fedora 35 can run them too.
- Releases check what they ship: each Linux and Apple Silicon build installs its own archives with `install.sh` and
  checks the installed version, and the Linux binaries must need exactly glibc 2.34, as the README states; a failure
  stops the release before publishing. The release notes link to the README's install section instead of repeating it.

### Fixed

- At volumes from about 30% up, the volume fade never quite reached its target, so the player kept computing it for
  every sample: playing now costs about half the CPU it did in the sound code, and brown noise (the default) about
  half again, since the noise generator is chosen once per buffer instead of every sample. The web demo's module
  shrinks from 6.1 to 4.2 KB.

### Security

- On Linux and macOS, `$XDG_RUNTIME_DIR` is used only when it is an absolute path to a directory owned by the user with
  mode 0700 (not a symlink); otherwise noiseman falls back to its private `noiseman-<uid>` directory in the temp dir,
  and the player's log says why. The lock file and log are created 0600 and opened without following symlinks, and
  the lock file is no longer truncated, so a symlink planted in a shared runtime directory can no longer empty another
  file.
- The Windows uninstaller waits for and ends only noiseman's own processes (`noiseman.exe`, `noiseman-gui.exe`, its own
  `nsm.exe` and renamed-aside copies), instead of every program running from the install directory, which may be
  shared through `NOISEMAN_INSTALL_DIR`.

## [0.1.1] - 2026-11-25

### Added

- The window takes `s` to stop the player, `+`/`=` and `-` for the volume, and Tab/Shift-Tab to cycle the noise,
  like the terminal UI. The browser demo also takes `+`/`=` and `-`.
- `noiseman --version` (or `-V`, or `version`) prints the version.
- The protocol has its own semver version, 0.1.0. Commands, the terminal UI and the window warn when the running
  player's protocol is incompatible, and say to run `noiseman stop` to restart it.
- The player's log, `noiseman.log`, starts each run with the version and protocol, and timestamps every line (UTC).
- On Windows, the window (`noiseman-gui.exe`, which has no console) shows an error that stops it in a message box.

### Changed

- A new icon matches the apps' logo: six bars of uneven heights on a dark tile. Linux also gets 16-32 px sizes, so
  trays and title bars show crisp bars rather than a scaled-down large icon.
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

- Linux: playback follows a new default output, such as headphones chosen in the sound settings; it stayed on the
  previous device.
- The installers stop a running player before replacing the binary, so an upgrade no longer leaves the old version
  playing.
- `noiseman help` says the window needs the gui build instead of `--features gui`, and lists the noises in the new
  order, as does the desktop entry.
- `noiseman gui` in a build without the window says how to get one, instead of rejecting `gui` as an unknown command.
- `noiseman` (or `noiseman tui`) run without a terminal, as from a launcher or a pipe, says it needs one and exits,
  instead of starting the player.
- On Linux with no PulseAudio or pipewire-pulse server, the player says `no PulseAudio server` (and its log asks
  whether it is running) instead of an ALSA error. A command that was meant to start sound, including any command
  that started the player, now exits 1 when it could not play (`vol` and `kind` exited 0).
- The player no longer keeps the directory it was started from in use (which could keep a drive from unmounting); it
  runs from the runtime dir.
- At a fractional display scale (such as 250% on Windows) the window no longer leaves an empty band on the right and
  bottom: it is sized to the nearest whole scale (3x at 250%), and again when moved to a screen with another scale.
- On Linux the player finds the PulseAudio (or PipeWire) server where libpulse does: with `PULSE_RUNTIME_PATH` set it
  looks for `$PULSE_RUNTIME_PATH/native`, then `$XDG_RUNTIME_DIR/pulse/native`, instead of giving up after the first
  missing location and falling back to ALSA.

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

[Unreleased]: https://github.com/roudy16/noiseman/compare/v0.1.2...HEAD
[0.1.2]: https://github.com/roudy16/noiseman/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/roudy16/noiseman/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/roudy16/noiseman/releases/tag/v0.1.0
