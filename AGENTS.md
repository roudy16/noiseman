# noiseman

Noise generator: a dependency-free sound core (`crates/core`), a headless player daemon (backend) and frontends that control it over a Unix socket: CLI, crossterm TUI, and an optional winit/softbuffer window with tray (`--features gui`). The core also runs in the browser (`web/`). Design rationale: `DESIGN.md`.

## Verbs

`just setup | build | test | run | install | install-gui | clean`; `just` alone lists every recipe. Run `just check` (fmt check, lint for default, `gui` and wasm32, tests, then `just gates`) before committing. `just gates` (`tools/gates.py`) checks artifact sizes, DSP ns/sample and the daemon's memory, threads, CPU and wakeups against `gates.toml`; a change that needs a higher limit raises it there in the same commit, with the reason. On Windows, run `just` from Git Bash (recipes use `sh`); the `/proc`-based runtime gates SKIP there, and `just gates-windows` (`tools/winperf.ps1`, `gates.toml` `[windows]`) is their counterpart, counting only noiseman's own threads. CI (`.github/workflows/ci.yml`) runs `just ci` on Linux and clippy and tests on macOS and Windows. `just web` builds the demo into `target/web`, which `.github/workflows/pages.yml` publishes on every push to main. `just deps` (network) reports advisories and dependency drift and fails only on a vulnerability: run it before upgrading or vendoring; CI runs it weekly. Known informational advisories, all `gui`-only with no upstream fix yet: unmaintained `ttf-parser` (winit's decorations) and `zbus_names`/`zvariant`/`zvariant_derive` (ksni). `just metrics` is a code size and complexity report, never a gate. Formatting: `rustfmt.toml` (120 columns).

## Releasing

- Push a tag `vX.Y.Z`; `.github/workflows/release.yml` fails unless it matches `Cargo.toml`'s version and `CHANGELOG.md` has a `## [X.Y.Z]` section, whose text becomes the release notes.
- Each target job runs `tools/package.sh TARGET` and `tools/package.sh TARGET gui` (`just package [target] [variant]`) for x86_64/aarch64 Linux (in an `ubuntu:22.04` container), macOS (minimum 11, `MACOSX_DEPLOYMENT_TARGET`) and Windows. The release holds the archives, their `.sha256` files, `tools/install.sh` and `tools/install.ps1`, each with a build provenance attestation (`gh attestation verify`, see `SECURITY.md`). Every action in the workflows is pinned to a commit SHA with its version in a comment (Dependabot bumps both), and `actions/checkout` keeps no credentials.
- Asset names `noiseman-<target>` (terminal build) and `noiseman-gui-<target>` (`.tar.gz`, `.zip` on Windows) must stay in step with `[package.metadata.binstall]` in `Cargo.toml` (terminal build only) and both installers (gui build with `NOISEMAN_GUI=1`).
- The Linux jobs fail unless the newest glibc symbol either binary imports is exactly `GLIBC_2.34`; if that changes, update the README and `install.sh`'s hint. Linux and aarch64 macOS jobs also install both archives with `install.sh` (`NOISEMAN_BASE_URL=file://$PWD/target/dist`) and check `noiseman --version` against the tag; x86_64 macOS is not install-tested (it builds on an arm64 runner, where `install.sh` picks the aarch64 archive); Windows is checked by hand.
- Each archive holds the binary (`noiseman` in both variants, so they install at the same path), `LICENSE`, `README.md` and `THIRD-PARTY-LICENSES.txt` (`tools/licenses.py`, that variant's crates); on Linux also `share/` (icons and a desktop entry from `assets/noiseman.desktop`, rewritten by `sed` to run the TUI in a terminal for the terminal build); in the Windows gui build also `noiseman-gui.exe`, the same program linked for the Windows subsystem, which the Start menu shortcut runs.
- `install.ps1` installs per user and must touch only what it wrote (PATH entry only if it added it, directory only once empty), ending only processes running its own binaries, because `NOISEMAN_INSTALL_DIR` may be a shared directory. It tells its own `nsm.exe` by the product name `build.rs` embeds.
- Icons: `uv run tools/icons.py` draws `assets/noiseman.ico`, `noiseman.svg` and the PNGs; `package.sh` and `just install` install every `assets/noiseman-<size>.png`, so `PNG_SIZES` in `icons.py` is the one list of sizes.

## Layout

Cargo workspace: the root package `noiseman` (the binary), `crates/core` and `web`.

- `crates/core/` (`noiseman-core`; no dependencies, no I/O, no allocation)
  - `src/lib.rs`: `Kind`, `gain(vol)`
  - `src/noise.rs`: `Noise` generators and the level/bounds test
  - `src/voice.rs`: `Voice` (one buffer per `fill`, the `FADE_SECS` fade, kind change via silence) and its tests
  - `examples/noise_profile.rs`: DSP benchmark for `just profile` and the DSP gate
- `src/main.rs`: `daemon [--muted]` runs `backend::serve`; everything else goes to `frontend::run` (run as `noiseman-gui` with no arguments: `gui`)
- `build.rs`: on Windows, embeds the icon and version info (`winresource`)
- `src/protocol.rs`: the only contract between the halves: `Command`, `State`, their line encoding, `PROTOCOL` and `compatible()`, `socket_path()`
- `src/platform/` (shared by both halves; the only OS-specific code)
  - `mod.rs`: the contract both backends meet, and the socket behaviour test
  - `unix.rs`: runtime dir and its privacy check, `open_private`, `setsid`, std's Unix sockets
  - `windows.rs`: runtime dir, detached spawn, console truecolor check, `alert_if_no_console`, AF_UNIX `UnixListener`/`UnixStream`
- `src/backend/` (no frontend imports)
  - `mod.rs`: `log!`, `serve()`: lock file, socket, accept thread, client reader threads, the player loop, watcher broadcast, fade-out on quit
  - `player.rs`: `Player`: volume, mute, stream lifetime, audio-lost restart, `State`
  - `audio.rs`: `GAIN`/`KIND` atomics, `AudioError`, `start()` (on Linux PulseAudio, then ALSA), data and error callbacks
- `src/frontend/` (no backend imports)
  - `mod.rs`: argument dispatch; the kind colours both UIs draw with
  - `client.rs`: `Client`, `connect(autostart)` (with the protocol check), daemon autostart
  - `cli.rs`: subcommands and `USAGE`; a command that asked for sound and was left muted by an audio error exits 1
  - `tui.rs`: crossterm screen; one thread reads keys, one reads pushed states, the main thread draws
  - `gui/mod.rs`, `gui/tray.rs` (`gui` feature): the window and ksni tray as a protocol client
- `web/` (`noiseman-web`, cdylib for `wasm32-unknown-unknown`)
  - `src/lib.rs`: `synth_init` and `synth_render` over one static synth; no wasm-bindgen, no heap
  - `site/`: `index.html`, `style.css`, `app.js` (controls, keys, AudioContext lifecycle), `worklet.js` (AudioWorkletProcessor calling the wasm)

## Invariants

- Frontends reach the backend only through `protocol` over the endpoint, never by import; the backend never imports `frontend`.
- Any change to the protocol bumps `protocol::PROTOCOL` by semver (independent of the app version): an addition clients may rely on bumps the minor, any other change the major (while the major is 0: the patch and the minor), as `compatible()` reads them.
- Audio data callback: no allocation, locks, logging or syscalls. It reads `KIND`/`GAIN` once per buffer (`Relaxed`).
- Player thread: blocks in `recv` (or `recv_timeout` only while a mute fade is pending). Muted means no stream and 0 wakeups; the daemon then has 2 threads plus one per connected client.
- Client writes use a 200 ms timeout so a stalled client cannot stall the player; a watcher whose write fails is dropped.
- One daemon per user: `noiseman.lock` (`File::try_lock`) is held for the daemon's life, so a socket file found while holding it is stale and removed.
- Error callback (not RT) logs, and on `DeviceNotAvailable | StreamInvalidated` sends `AudioLost` to the player thread, which restarts the stream.
- Default state: brown, volume 50, playing. Quit mutes and waits for the fade before exiting.
- cpal uses the `pulseaudio` backend on Linux (CoreAudio on macOS, WASAPI on Windows); the default format is the sink's own (I32 on the PipeWire pulse server here), so keep the `SampleFormat::I32` arm alongside the others.
- `crates/core` stays dependency-free and I/O-free; both the daemon and the web demo get their sound only from it, so they sound the same.
- `docs/architecture.html` is the illustrated explainer, published at `/architecture/` by `just web`; update its diagrams and numbers when threads, states, the protocol, the fade or the gate baselines change.
- Web: the worklet does no allocation per block (cached `Float32Array` view); the page suspends the AudioContext after a pause; the wasm has no heap. Keep the module small (check its size after `just web`).
- Every noise type is normalised to about -18 dBFS RMS; the test enforces ±1.5 dB.
- Release profile uses `panic = "abort"`; never let a panic reach the callback. The TUI restores the terminal from a panic hook.
- GUI (Linux tray): closing the window hides to tray (drops window and surface); with no tray, closing quits and stops the player. Tray callbacks only `send_event`; zbus calls stay off the winit thread. The window is sized to `WIN_W` x `WIN_H` times the whole scale nearest the OS scale factor (`window_size`), at open and on `ScaleFactorChanged`, so `paint` fills it. ksni uses `blocking` + `tokio`.
- No GPU, no toolkit. New dependencies must be permissive (no GPL/LGPL/AGPL/MPL); check with `cargo tree --features gui --target all -e normal,build -f '{p} {l}'`. Our crates are MIT.
- OS-specific code lives only in `src/platform/`, whose `mod.rs` states the contract that `unix.rs` and `windows.rs` implement with the same signatures; nothing else names `std::os::*`, `libc` or `windows_sys`. Unix: the socket 0600 in a 0700 runtime dir (a non-private `$XDG_RUNTIME_DIR` falls back to `noiseman-<uid>`), lock and log opened 0600 with O_NOFOLLOW (`open_private`). Windows: the socket file relies on the runtime dir's ACL (`%LOCALAPPDATA%\noiseman`); `$XDG_RUNTIME_DIR` is taken as given there, which only the tests and gates set; socket paths are UTF-8 and under 108 bytes; delete the AF_UNIX copy in `windows.rs` once std has one.
- Vendored crates, wired in via `[patch.crates-io]`, each pruned to what builds: `vendor/pulseaudio` is pulseaudio 0.3.2 (`src/`, `Cargo.toml`, `LICENSE.md`), `vendor/cpal` is cpal 0.18.2 (`src/`, `build.rs`, `Cargo.toml`, `LICENSE`), and each `Cargo.toml` is stripped of readme, examples and dev-dependencies, so their tests (including pulseaudio's `socket_path_test`, which covers a patch) build only on a standalone copy with those dev-dependencies (`anyhow`, `pretty_assertions`, `assert_matches`) restored. Every other difference from upstream is a patch: the "Modified by noiseman" header at the top of each changed file (`Cargo.toml` included; Apache-2.0 §4(b) for cpal) says what changed and why, and a `noiseman patch` comment marks each change; `grep -rn 'noiseman patch' vendor` lists them. Keep each crate's LICENSE file, and the header and markers on any new change; re-apply or drop each patch when upgrading.
