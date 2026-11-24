# noiseman

Noise generator split into a headless player daemon (backend) and frontends that control it over a Unix socket: CLI, crossterm TUI, and an optional winit/softbuffer window with tray (`--features gui`). Spec: `DESIGN.md` (see its revision notes).

## Verbs

`just setup | build | test | run | install | clean`, plus `just fmt`, `just lint` (clippy, `-D warnings`, default and `--features gui`) and `just check` (fmt check, lint, tests): run `just check` before committing. Formatting: `rustfmt.toml` (120 columns, `use_small_heuristics = "Max"`).

## Layout

- `src/main.rs`: `daemon` runs `backend::serve`; everything else goes to `frontend::run`
- `src/protocol.rs`: the only contract between the halves: `Kind`, `Command`, `State`, their line encoding, socket path, round-trip test
- `src/backend/` (no frontend imports)
  - `mod.rs`: `serve()`: lock file, socket, accept thread, one reader thread per client, the player loop (`mpsc` + `recv_timeout`), watcher broadcast, fade-out on quit
  - `player.rs`: `Player`: volume, mute, stream lifetime (drop after fade), audio-lost restart, `State`
  - `audio.rs`: `GAIN`/`KIND` atomics (private; `kind()`, `set_kind()`, `set_gain()`), `AudioError`, `start()` returning `Output` (`buffer_time()`), `build::<T>()`, `Voice::step`, data and error callbacks
  - `noise.rs`: `Noise` generators, the one level/bounds test
- `src/frontend/` (no backend imports)
  - `mod.rs`: argument dispatch (none/`tui`, `gui`, `help`, else CLI)
  - `client.rs`: `Client { sender, receiver }`, `connect(autostart)`, daemon autostart (`setsid`, log to runtime dir)
  - `cli.rs`: subcommands and `USAGE`
  - `tui.rs`: crossterm screen; one thread reads keys, one reads pushed states, the main thread draws
  - `gui/mod.rs`, `gui/tray.rs` (`gui` feature): the window and ksni tray as a protocol client

## Invariants

- Frontends reach the backend only through `protocol` over the socket, never by import; the backend never imports `frontend`.
- Audio data callback: no allocation, locks, logging or syscalls. It reads `KIND`/`GAIN` once per buffer (`Relaxed`).
- Player thread: blocks in `recv` (or `recv_timeout` only while a mute fade is pending). Muted means no stream and 0 wakeups; the daemon then has 2 threads plus one per connected client.
- Client writes use a 200 ms timeout so a stalled client cannot stall the player; a watcher whose write fails is dropped.
- One daemon per user: `noiseman.lock` (`File::try_lock`) is held for the daemon's life, so a socket file found while holding it is stale and removed.
- Error callback (not RT) logs, and on `DeviceNotAvailable | StreamInvalidated` sends `AudioLost` to the player thread, which restarts the stream.
- Default state: brown, volume 50, playing. Quit mutes and waits for the fade before exiting.
- cpal uses the `pulseaudio` backend; it reports I32 as the default format, so keep the `SampleFormat::I32` arm.
- Every noise type is normalised to about -18 dBFS RMS; the test enforces ±1.5 dB.
- Release profile uses `panic = "abort"`; never let a panic reach the callback. The TUI restores the terminal from a panic hook.
- GUI (Linux tray): closing the window hides to tray (drops window and surface); with no tray, closing quits and stops the player. Tray callbacks only `send_event`; zbus calls stay off the winit thread. ksni uses `blocking` + `tokio`.
- Dependencies: cpal, crossterm, libc; with `gui`: winit, softbuffer, font8x8, ksni (Linux). No GPU, no toolkit.
- Unix only (the socket); Windows would need another transport.
- Vendored crates, wired in via `[patch.crates-io]`; re-apply or drop each change when upgrading:
  - `vendor/pulseaudio`: pulseaudio 0.3.2 (pruned to `src/`, `Cargo.toml`, `LICENSE.md`) with two changes in `client/reactor.rs`: `Reactor::recv` reads in 64 KiB chunks instead of zero-filling 1 MiB per read, and `handle_command` removes a stream on `PlaybackStreamKilled` (upstream ignored it, so a server-killed stream was never reported and dropping it hung).
  - `vendor/cpal`: cpal 0.18.2 (pruned to `src/`, `build.rs`, `Cargo.toml`, `LICENSE`) with one change in `host/pulseaudio/stream.rs`: the playback latency thread no longer wakes per write or every 100 ms, only on play/pause/drop, so `OutputCallbackInfo` timestamps go stale (noiseman ignores them).
