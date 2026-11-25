# noiseman

![The noiseman terminal UI playing brown noise at 50% volume](assets/noiseman-tui.png)

A noise generator (white, pink, brown, blue, violet). A small background
player owns the audio (cpal); a CLI and a terminal UI control it over a Unix
socket. An optional window with a system-tray icon is available with the `gui`
feature. The sound itself lives in a dependency-free core crate (`crates/core`) that also compiles to
WebAssembly for a browser demo (`web/`). See `DESIGN.md`.

- **Try it in the browser:** https://roudy16.github.io/noiseman/
- **How it works:** https://roudy16.github.io/noiseman/architecture/ (diagrams of the daemon, the sound core and the web demo; source `docs/architecture.html`)

## Install

**Supported systems:** Linux (PulseAudio or PipeWire via `pipewire-pulse`) and macOS (CoreAudio). Windows is not yet supported for the native app, though the browser demo works on any OS.

**Prerequisites:** A Rust toolchain (1.85+, edition 2024). On Linux, you also need `pkg-config` and ALSA development headers (`libasound2-dev` on Debian/Ubuntu, `alsa-lib-devel` on Fedora, `alsa-lib` on Arch) for cpal's ALSA fallback.

Clone the repository and install `noiseman` to `~/.cargo/bin`:

```sh
git clone https://github.com/roudy16/noiseman.git
cd noiseman
cargo install --path .
```

To include the optional window and (on Linux) system-tray icon:

```sh
cargo install --path . --features gui
```

For a shorter command, create an `nsm` symlink (`ln -sfn noiseman ~/.cargo/bin/nsm`) or shell alias (`alias nsm=noiseman`). On Linux with `just`, `just install` does this for you in `~/.local/bin` along with hicolor icons and a `.desktop` entry.

## Usage

```sh
noiseman                 # open the terminal UI (controls shown at the bottom)
noiseman gui             # open the desktop window and tray (needs --features gui)
noiseman play [KIND]     # start or unmute (white, pink, brown, blue, violet)
noiseman kind KIND       # switch noise kind
noiseman vol N|+N|-N     # set or adjust volume (0-100)
noiseman mute            # mute playback (also: unmute, toggle)
noiseman status          # show current state
noiseman stop            # stop the background player
noiseman daemon          # run the player in the foreground (--muted to start silent)
noiseman --help          # full command reference
```

## Development

Dev workflows go through [`just`](https://github.com/casey/just) (run `just` alone to list all recipes):

```sh
just setup                 # fetch dependencies and add the wasm32 target
just build                 # compile (use cargo --features gui for the window)
just run [args...]         # run the binary
just test                  # run workspace tests
just fmt                   # format with rustfmt
just lint                  # clippy across default, gui and wasm32 targets
just gates                 # check size, DSP and runtime limits (gates.toml)
just check                 # fmt check, lint, test and gates (run before committing)
just deps                  # check advisories and outdated dependencies (network)
just web                   # build the static browser demo into target/web
just web-serve             # serve the browser demo on http://localhost:8000
just profile [kind] [secs] # profile the DSP into profiles/dsp-<kind>.*
just install               # install binary, nsm alias, icons and desktop entry (Linux)
just clean                 # remove build artifacts
```

## License

MIT, see `LICENSE`, for everything except `vendor/`.

`vendor/` holds two third-party crates under their own licenses, each with small changes (described at the top of
each changed file and in `AGENTS.md`):

- `vendor/cpal`: cpal 0.18.2, Apache License 2.0 (`vendor/cpal/LICENSE`)
- `vendor/pulseaudio`: pulseaudio 0.3.2 by Colin Marc, MIT (`vendor/pulseaudio/LICENSE.md`)

All other dependencies come from crates.io under permissive licenses (MIT, Apache-2.0, BSD-2/3-Clause, ISC, Zlib,
Unlicense, Unicode-3.0); `cargo tree --features gui --target all -e normal,build -f '{p} {l}'` lists them. The web
demo's WebAssembly is built only from `crates/core` and `web/`, with no third-party crates.

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
