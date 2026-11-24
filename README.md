# noiseman

A noise generator (white, pink, brown, blue, violet). A small background
player owns the audio (cpal); a CLI and a terminal UI control it over a Unix
socket. An optional window with a system-tray icon is available with the `gui`
feature. See `DESIGN.md`.

## Usage

- `noiseman`: terminal UI. `1`–`5`, `↑`/`↓` or Tab noise, `←`/`→` volume, `m`/Space mute,
  `q` leave (noise keeps playing), `s` stop the player. Monochrome under
  `NO_COLOR` or on terminals with fewer than 256 colors.
- `noiseman play [KIND]`, `stop`, `mute`, `unmute`, `toggle`,
  `kind KIND`, `vol N|+N|-N`, `status`.
- `noiseman daemon`: run the player in the foreground. Commands that should
  make sound start it automatically, detached from the terminal, logging to
  `$XDG_RUNTIME_DIR/noiseman.log`.
- `noiseman gui` (built with `--features gui`): the window. On Linux with a
  system tray (StatusNotifierItem, e.g. COSMIC or KDE), closing the window
  hides it to the tray; without a tray, closing it stops the player.

## Building

Prerequisites: a Rust toolchain; on Linux also `libasound2-dev` (cpal's ALSA
fallback). Audio uses PulseAudio/pipewire-pulse when available.

All dev workflows go through `just` (run `just` alone to list recipes):

- `just setup` — fetch dependencies (cargo fetch)
- `just build` — compile
- `just test` — run tests (cargo test)
- `just fmt` — format the code (rustfmt)
- `just lint` — clippy with warnings as errors, with and without the `gui` feature
- `just check` — format check, lint and tests
- `just run [args...]` — run the binary
- `just install` — install to `~/.local/bin`, plus hicolor icons (`~/.local/share/icons/hicolor/*/apps/noiseman.*`) and desktop entry (`~/.local/share/applications/noiseman.desktop`, opens in a terminal)
- `just clean` — remove build artifacts

For the window: `cargo build --release --features gui`.

## TODO

- [ ] **Windows support.** The player and its clients talk over a Unix socket
  (`std::os::unix::net`), so the crate builds only on Linux and macOS. Windows
  needs another transport for `src/protocol.rs`'s line protocol, e.g. a named
  pipe (`\\.\pipe\noiseman-<user>`), plus a replacement for `setsid` when
  autostarting the player and for the lock/runtime-dir logic. The TUI
  (crossterm) and the audio (cpal/WASAPI) already support Windows.
- [ ] **Rain and thunderstorm.** Two more kinds: steady rain, and rain with
  occasional thunder (a noise bed plus sparse droplet transients; thunder as
  rare low-passed rumbles). Each sets its own readable TUI theme, as the
  noise kinds do (e.g. slate-teal for rain, deep indigo with a pale flash
  accent for thunder). Adds `Kind` variants, so it touches the protocol, the
  backend generators (and their level test), the TUI's `1`-`5` keys and the
  GUI.
