# noiseman

A single-window desktop noise generator (white, pink, brown, blue, violet).
The window is a bare native window with hand-drawn controls (winit +
softbuffer, no GPU, no toolkit); audio goes through cpal. See `DESIGN.md`.

Controls: click a noise button or press 1–5; click/drag the volume bar or use
Left/Right; click Mute or press M/Space.

Build prerequisites: a Rust toolchain; on Linux also `libasound2-dev`
(cpal's ALSA fallback). Audio uses PulseAudio/pipewire-pulse when available.

All dev workflows go through `just` (run `just` alone to list recipes):

- `just setup` — fetch dependencies (cargo fetch)
- `just build` — compile
- `just test` — run tests (cargo test)
- `just run [args...]` — run the binary
- `just install` — install the binary (cargo install)
- `just clean` — remove build artifacts
