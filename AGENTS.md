# noiseman

Desktop noise generator: a bare winit window with hand-drawn controls (softbuffer CPU framebuffer, font8x8 bitmap text) and cpal audio. Spec: `DESIGN.md` (see its revision note).

## Verbs

`just setup | build | test | run | install | clean`. Lint: `cargo clippy --all-targets -- -D warnings`.

## Layout

- `src/main.rs`: window, event loop, hit-testing, drawing (`Canvas`), mute timer via `ControlFlow::WaitUntil`, device-loss restart via `EventLoopProxy`
- `src/audio.rs`: `GAIN`/`KIND` atomics, `AudioError`, `start()`, `build::<T>()`, data and error callbacks
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
- Dependencies: cpal, winit, softbuffer, font8x8. No GPU, no toolkit.
