# noiseman

A noise generator (white, pink, brown, blue, violet). A small background
player owns the audio (cpal); a CLI and a terminal UI control it over a Unix
socket. An optional window with a system-tray icon is available with the `gui`
feature. The sound itself lives in a dependency-free core crate (`crates/core`) that also compiles to
WebAssembly for a browser demo (`web/`). See `DESIGN.md`.

## Usage

- `noiseman`: terminal UI. `1`–`5`, `↑`/`↓` or Tab noise, `←`/`→` volume, `m`/Space mute,
  `q` leave (noise keeps playing), `s` stop the player. Monochrome under
  `NO_COLOR` or on terminals with fewer than 256 colors.
- `noiseman play [KIND]`, `stop`, `mute`, `unmute`, `toggle`,
  `kind KIND`, `vol N|+N|-N`, `status`.
- `noiseman daemon [--muted]`: run the player in the foreground (`--muted`
  starts it silent, e.g. at login). Commands that should
  make sound start it automatically, detached from the terminal, logging to
  `$XDG_RUNTIME_DIR/noiseman.log`.
- `noiseman gui` (built with `--features gui`): the window. On Linux with a
  system tray (StatusNotifierItem, e.g. COSMIC or KDE), closing the window
  hides it to the tray; without a tray, closing it stops the player.

## Building

Prerequisites: a Rust toolchain; on Linux also `libasound2-dev` (cpal's ALSA
fallback). Audio uses PulseAudio/pipewire-pulse when available.

All dev workflows go through `just` (run `just` alone to list recipes):

- `just setup` — fetch dependencies and add the `wasm32-unknown-unknown` target
- `just build` — compile
- `just test` — run tests (all workspace crates)
- `just fmt` — format the code (rustfmt)
- `just lint` — clippy with warnings as errors: with and without the `gui` feature, and for the wasm target
- `just check` — format check, lint, tests and quality gates
- `just gates` — size, CPU and memory limits from `gates.toml` (about 2 s; needs PulseAudio for the runtime part)
- `just web` / `just web-serve` — build / serve the browser demo
- `just run [args...]` — run the binary
- `just install` — install to `~/.local/bin` (with `nsm` as a short alias), plus hicolor icons (`~/.local/share/icons/hicolor/*/apps/noiseman.*`) and desktop entry (`~/.local/share/applications/noiseman.desktop`, opens in a terminal)
- `just clean` — remove build artifacts

For the window: `cargo build --release --features gui`.

Web demo: `just web` builds a static site into `target/web` (needs the `wasm32-unknown-unknown` target, which
`just setup` adds); `just web-serve` serves it at http://localhost:8000. Any static
host works; the page is about 16 KB including the ~6 KB WebAssembly module.

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
  accent for thunder). Adds `Kind` variants, so it touches the core's
  generators (and their level test), the protocol, the TUI's `1`-`5` keys,
  the GUI and the web demo's kinds and accents.
