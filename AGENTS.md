# noiseman

Desktop noise generator: a bare winit window with hand-drawn controls (softbuffer CPU framebuffer, font8x8 bitmap text) and cpal audio. Spec: `DESIGN.md` (see its revision note).

## Verbs

`just setup | build | test | run | install | clean`. Lint: `cargo clippy --all-targets -- -D warnings`.

## Layout

- `src/main.rs`: window, event loop, hit-testing, drawing (`Canvas`), mute timer via `ControlFlow::WaitUntil`, `Event` user events (device loss, tray actions) via `EventLoopProxy`, hide/show to tray, non-Linux `tray` stub
- `src/tray.rs` (Linux only): ksni StatusNotifierItem `Item` (icon `noiseman`, tooltip, menu) and `spawn()`: a worker thread does the D-Bus handshake, sends `Event::TrayReady(Tray)`, then applies `Tray::sync` states
- `src/audio.rs`: `GAIN`/`KIND` atomics (private; `kind()`, `set_kind()`, `set_gain()`), `AudioError`, `start()` returning `Output` (`buffer_time()`), `build::<T>()`, `Voice::step`, data and error callbacks
- `src/noise.rs`: `Kind`, `Noise` generators, the one level/bounds test

## Invariants

- Audio data callback: no allocation, locks, logging or syscalls. It reads `KIND`/`GAIN` once per buffer (`Relaxed`).
- UI → audio only through `GAIN` (f32 bits) and `KIND`. Mute is target gain 0, then the stream is dropped after the fade.
- Event loop uses `ControlFlow::Wait` (or `WaitUntil` only while a mute fade is pending); redraw only on events. Muted or idle means no stream and 0 wakeups.
- Error callback (not RT) logs, and on `DeviceNotAvailable | StreamInvalidated` wakes the loop with `EventLoopProxy::send_event`, which restarts the stream.
- cpal uses the `pulseaudio` backend; it reports I32 as the default format, so keep the `SampleFormat::I32` arm.
- Default noise is brown (`KIND` initial value).
- Every noise type is normalised to about -18 dBFS RMS; the test enforces ±1.5 dB.
- Release profile uses `panic = "abort"`; never let a panic reach the callback.
- Tray (Linux): closing the window hides to tray by dropping the window and surface (winit cannot hide on Wayland); audio keeps playing. With no tray (spawn error or watcher offline) closing quits, and a hidden window is restored, so the app never runs with neither.
- Tray callbacks run on ksni's thread and only `send_event` an `Event`; all state stays on the main thread, which pushes changes back with `Tray::sync` (no-op unless shown/muted/kind changed). zbus calls have no timeout, so the main thread never calls ksni; it only sends on a channel. Until `TrayReady` arrives, closing quits.
- ksni uses `blocking` + `tokio` (current-thread runtime): two extra idle threads (ksni service, tray worker), 0 wakeups. The `async-io` backend was measured worse (4 extra threads; `blocking` 1.7's pool thread wakes every 500 ms forever).
- Dependencies: cpal, winit, softbuffer, font8x8, ksni (Linux). No GPU, no toolkit.
- `vendor/pulseaudio` is pulseaudio 0.3.2 (pruned to `src/`, `Cargo.toml`, `LICENSE.md`) with one change in `Reactor::recv` (64 KiB read chunk instead of a 1 MiB zero-fill per read), wired in via `[patch.crates-io]`. Re-apply or drop it when upgrading.
