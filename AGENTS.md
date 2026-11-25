# noiseman

Noise generator: a dependency-free sound core (`crates/core`), a headless player daemon (backend) and frontends that control it over a Unix socket: CLI, crossterm TUI, and an optional winit/softbuffer window with tray (`--features gui`). The core also runs in the browser (`web/`). Spec: `DESIGN.md` (see its revision notes).

## Verbs

`just setup | build | test | run | install | clean`, plus `just fmt`, `just lint` (clippy, `-D warnings`, default and `--features gui`) and `just check` (fmt check, lint, tests, all workspace crates, then `just gates`; lint also runs clippy for the wasm target): run `just check` before committing. `just gates` (`tools/gates.py`, ~2 s after the release builds) checks artifact sizes, DSP ns/sample, and the daemon's memory, threads, CPU and wakeups playing and muted, plus TUI and CLI memory, against `gates.toml`; it runs a private daemon (`daemon --muted`, then `vol 0` + `unmute`: a real stream, silent) and skips the runtime gates without a PulseAudio server. A change that needs a higher limit raises it in `gates.toml` in the same commit, with the reason. `just web` builds the demo into `target/web`; `just web-serve` serves it on :8000. Formatting: `rustfmt.toml` (120 columns, `use_small_heuristics = "Max"`).

## Layout

Cargo workspace: the root package `noiseman` (the binary), `crates/core` and `web`.

- `crates/core/` (`noiseman-core`; no dependencies, no I/O, no allocation)
  - `src/lib.rs`: `Kind` (labels, `from_u8`, `parse`), `gain(vol)` (square law)
  - `src/noise.rs`: `Noise` generators, the one level/bounds test
  - `src/voice.rs`: `Voice::new(rate, seed, kind)` and `step(kind, gain)`: 10 ms gain fade, kind change via silence; its test
  - `examples/noise_profile.rs`: DSP benchmark for `just profile`
- `src/main.rs`: `daemon [--muted]` runs `backend::serve`; everything else goes to `frontend::run`
- `src/protocol.rs`: the only contract between the halves: `Kind` (re-exported from core), `Command`, `State`, their line encoding, socket path, round-trip test
- `src/backend/` (no frontend imports)
  - `mod.rs`: `serve()`: lock file, socket, accept thread, one reader thread per client, the player loop (`mpsc` + `recv_timeout`), watcher broadcast, fade-out on quit
  - `player.rs`: `Player`: volume, mute, stream lifetime (drop after fade), audio-lost restart, `State`
  - `audio.rs`: `GAIN`/`KIND` atomics (private; `kind()`, `set_kind()`, `set_gain()`), `AudioError`, `start()` returning `Output` (`buffer_time()`), `build::<T>()`, data and error callbacks (a core `Voice` per stream)
- `src/frontend/` (no backend imports)
  - `mod.rs`: argument dispatch (none/`tui`, `gui`, `help`, else CLI)
  - `client.rs`: `Client { sender, receiver }`, `connect(autostart)`, daemon autostart (`setsid`, log to runtime dir)
  - `cli.rs`: subcommands and `USAGE`
  - `tui.rs`: crossterm screen; one thread reads keys, one reads pushed states, the main thread draws
  - `gui/mod.rs`, `gui/tray.rs` (`gui` feature): the window and ksni tray as a protocol client
- `web/` (`noiseman-web`, cdylib for `wasm32-unknown-unknown`)
  - `src/lib.rs`: `synth_init(rate, seed, kind)`, `synth_render(kind, vol, frames) -> *const f32` over one static synth; no wasm-bindgen, no heap
  - `site/`: `index.html`, `style.css`, `app.js` (controls, keys, localStorage, AudioContext lifecycle), `worklet.js` (AudioWorkletProcessor calling the wasm)

## Invariants

- Frontends reach the backend only through `protocol` over the socket, never by import; the backend never imports `frontend`.
- Audio data callback: no allocation, locks, logging or syscalls. It reads `KIND`/`GAIN` once per buffer (`Relaxed`).
- Player thread: blocks in `recv` (or `recv_timeout` only while a mute fade is pending). Muted means no stream and 0 wakeups; the daemon then has 2 threads plus one per connected client.
- Client writes use a 200 ms timeout so a stalled client cannot stall the player; a watcher whose write fails is dropped.
- One daemon per user: `noiseman.lock` (`File::try_lock`) is held for the daemon's life, so a socket file found while holding it is stale and removed.
- Error callback (not RT) logs, and on `DeviceNotAvailable | StreamInvalidated` sends `AudioLost` to the player thread, which restarts the stream.
- Default state: brown, volume 50, playing. Quit mutes and waits for the fade before exiting.
- cpal uses the `pulseaudio` backend; the default format is the sink's own (I32 on the PipeWire pulse server here), so keep the `SampleFormat::I32` arm alongside the others.
- `crates/core` stays dependency-free and I/O-free; both the daemon and the web demo get their sound only from it, so they sound the same.
- `docs/architecture.html` is the illustrated explainer, published at `/architecture/` by `just web`; update its diagrams and numbers when threads, states, the protocol, the fade or the gate baselines change.
- Web: the worklet does no allocation per block (cached `Float32Array` view); the page suspends the AudioContext after a pause; the wasm has no heap. Keep the module small (check its size after `just web`).
- Every noise type is normalised to about -18 dBFS RMS; the test enforces ±1.5 dB.
- Release profile uses `panic = "abort"`; never let a panic reach the callback. The TUI restores the terminal from a panic hook.
- GUI (Linux tray): closing the window hides to tray (drops window and surface); with no tray, closing quits and stops the player. Tray callbacks only `send_event`; zbus calls stay off the winit thread. ksni uses `blocking` + `tokio`.
- Dependencies: noiseman-core, cpal, crossterm, libc; with `gui`: winit, softbuffer, font8x8, ksni (Linux). No GPU, no toolkit.
- Unix only (the socket); Windows would need another transport.
- `just deps` (`tools/deps.py`, network) reports advisories for every locked crate (OSV, which mirrors RustSec) and drift of direct dependencies and vendored crates from crates.io's latest stable (via the sparse index); it fails only on a vulnerability. Run it before upgrading or vendoring. Known informational advisories (2026-11-24): unmaintained `ttf-parser` (winit's decorations) and `zbus_names`/`zvariant`/`zvariant_derive` (ksni), all `gui`-only, no fix upstream yet.
- Licenses: our crates are MIT (`LICENSE`, `license = "MIT"`). New dependencies must be permissive (no GPL/LGPL/AGPL/MPL); check with `cargo tree --features gui --target all -e normal,build -f '{p} {l}'`.
- Vendored crates, wired in via `[patch.crates-io]`; re-apply or drop each change when upgrading. Keep each crate's LICENSE file, and the "Modified by noiseman" header at the top of every changed file, `Cargo.toml` included (Apache-2.0 §4(b) for cpal):
  - `vendor/pulseaudio`: pulseaudio 0.3.2 (pruned to `src/`, `Cargo.toml`, `LICENSE.md`; `Cargo.toml` stripped of readme, examples and dev-dependencies so the pruned crate builds) with two code changes in `client/reactor.rs`: `Reactor::recv` reads in 64 KiB chunks instead of zero-filling 1 MiB per read, and `handle_command` removes a stream on `PlaybackStreamKilled` (upstream ignored it, so a server-killed stream was never reported and dropping it hung).
  - `vendor/cpal`: cpal 0.18.2 (pruned to `src/`, `build.rs`, `Cargo.toml`, `LICENSE`; `Cargo.toml` stripped likewise) with one code change in `host/pulseaudio/stream.rs`: the playback latency thread no longer wakes per write or every 100 ms, only on play/pause/drop, so `OutputCallbackInfo` timestamps go stale (noiseman ignores them).
