# noiseman: design v1

> **Revision (2026-11-24, Windows): the transport and other OS-specific parts sit behind `src/platform/`.** One contract (`platform/mod.rs`: `runtime_dir`, `restrict_to_owner`, `spawn_detached`, `terminal_truecolor`, `net::{UnixListener, UnixStream}`), implemented by `unix.rs` (std's Unix sockets, the 0700 runtime dir and `setsid` described below, unchanged) and `windows.rs`. Windows 10 1803 and later have AF_UNIX sockets, so the daemon listens on `noiseman.sock` there too, and the backend and client are the same code on every OS. std does not offer AF_UNIX on Windows yet, so `windows.rs` carries the subset of its API noiseman uses (about 150 lines over WinSock). The socket file inherits the runtime dir's ACL, `%LOCALAPPDATA%\noiseman` (or `$XDG_RUNTIME_DIR`): only the user, SYSTEM and admins, and connecting needs write access, which a low-integrity process of the same user lacks. That is the trust of the Unix socket's 0700 dir. A first version used a named pipe and needed roughly 700 lines for the same guarantees: the pipe namespace is global, so it took a random published name, an owner-only ACL, owner and integrity checks, and overlapped handles with cancellable writes. Autostart spawns the daemon detached (`DETACHED_PROCESS`, its own process group, out of the caller's job where allowed). The lock file, protocol, threads and wakeup invariants are the same on every OS; `just gates-windows` measures them (`gates.toml` `[windows]`). This lifts the "Unix only" cost in the next note.

> **Revision (2026-11-24, later): the sound is a reusable core crate, with a web demo.** `crates/core` (`noiseman-core`, no dependencies, no I/O) holds `Kind`, `Noise` (was `src/backend/noise.rs`), `Voice` (the click-free gain/kind fade, was in `audio.rs`) and `gain()` (the volume curve, was in `player.rs`), with their tests. The daemon's `audio.rs` keeps only cpal and the atomics. `web/` builds the core for `wasm32-unknown-unknown` as two raw exports over one static synth (no wasm-bindgen, no heap: ~6 KB) and serves it from a static page through an AudioWorklet (mono, `latencyHint: "playback"`; the AudioContext is suspended while paused). References below to `noise.rs` mean `crates/core/src/noise.rs`; `Kind` is in `crates/core/src/lib.rs` and `Voice` in `crates/core/src/voice.rs`.

> **Revision (2026-11-24): headless player daemon with CLI, TUI and optional GUI frontends.** This supersedes the single-process structure below and in the 2026-11-23 note:
> - **Backend** (`src/backend/`): `noiseman daemon` owns the audio stream and the mute/volume state (`Player`) and serves a line protocol (`src/protocol.rs`) on `noiseman.sock` (0600) in `$XDG_RUNTIME_DIR`, or in a 0700 `noiseman-<uid>` directory under the temp dir when that is unset. One instance per user, enforced by a lock file; the lock file and the daemon log live in the same directory. A per-connection reader thread forwards requests to the player thread over an mpsc channel, which waits with `recv_timeout` only while a mute fade is pending: 0 wakeups when muted. On device loss the cpal error callback sends `AudioLost` over the same channel, and the player thread rebuilds the stream (`Player::audio_lost`).
> - **Frontends** (`src/frontend/`): CLI subcommands, a crossterm TUI (default with no arguments) and the winit/softbuffer/tray window behind the `gui` cargo feature. All are clients of the protocol; none touches audio. The TUI and GUI start the daemon when they open; CLI commands other than `status`, `mute` and `stop` start it if needed (`setsid`, log in the runtime dir as `noiseman.log`). The daemon outlives them; `stop` (or `s` in the TUI, Quit in the GUI) ends it.
> - **Why:** measured playing on Linux, the old single process used 8.6 MB RSS, 5.0 MB PSS, 4.8 MB private. The daemon uses 4.0 MB RSS and 1.2 MB private, with a 0.79 MB binary (was 3.85 MB); the TUI while open adds 3.2 MB RSS, 0.2 MB private.
> - **Cost:** Unix only (Linux, macOS) because of the Unix socket; Windows would need a named-pipe transport.

> **Revision (2026-11-23): GUI and audio backend changed after measurement.** This supersedes the wxDragon parts of §1, §2, §3, §7 and §8 (§5, §6 and §9 have since been updated to the current code):
> - **GUI:** wxDragon is replaced by a bare winit 0.30 window, drawn on the CPU with softbuffer 0.4, with text from the font8x8 bitmap font. Controls are drawn by hand; the event loop redraws only when an event arrives. No GPU, no toolkit, no C++ build.
> - **Why:** measured on Linux and playing, wxDragon used 53 MB RSS and 17.8 MB private, of which GTK3 and wxWidgets are about 45 MB. The bare window uses 8.2 MB RSS and 4.5 MB private, with a 2.5 MB binary.
> - **Cost:** no native look, no accessibility, no IME, integer HiDPI scaling and ASCII-only text. All accepted for this app.
> - **Audio:** cpal now uses the `pulseaudio` feature. That avoids the ALSA→PipeWire plugin and saves about 4 MB and one thread. The Pulse host reports I32 as its default format, so `audio.rs` handles `SampleFormat::I32`.
> - **Default noise:** brown.
> - **System tray (Linux only):** `ksni` 0.3.7 (pure-Rust StatusNotifierItem + dbusmenu), features `blocking` + `tokio`. Closing the window drops the window and surface (Wayland cannot hide a window) and keeps audio playing; the tray icon toggles it, and the menu has Show/Hide, Mute and Quit. Without a tray, closing quits. D-Bus work runs on a worker thread so a stalled watcher cannot freeze the window. Cost, measured: +1.3 MB binary, two idle threads with 0 wakeups, about +1.4 MB RSS while playing; muted and hidden is 8.4 MB RSS, 4.8 MB private. The `async-io` backend was rejected: 4 extra threads, one of which wakes every 500 ms.


## 1. Summary

noiseman is a single-window Rust desktop app for Linux, Windows and macOS. It plays white, pink, brown, blue or violet noise, and the window has three controls: noise type, volume and mute.
- **Stack:** wxDragon for native OS widgets and cpal for audio. That is 2 direct dependencies and 3 source files.
- **Generation:** all noise is generated live, per sample, in the audio callback. Looping a buffer is rejected because none of these signals is periodic (§4).
- **Muted means fully idle:** mute drops the audio stream, so a muted or idle app makes 0 wakeups and uses 0% CPU. Playing costs about 0.014% of a core for the DSP itself.
- **Blocking question:** one question needs an answer before any code is written: "native widgets" or just "a native window" (§3, §8).

## 2. Tech stack

| Concern | Choice | Why | Alternatives rejected |
|---|---|---|---|
| GUI | `wxdragon = "=0.9.25"` (released 2026-11-17, bundles wxWidgets 3.3.4). Widgets: `Choice`, `Slider` 0..100, `CheckBox`, `StaticText` status line, in one `Frame` with a `BoxSizer` | The only maintained safe Rust crate that uses the real Win32, Cocoa and GTK3 widgets. It is event-driven with no GPU, so it uses ~0 CPU when idle. It has `timer::Timer` (one-shot) and `app::call_after` (cross-thread), both verified in 0.9.25. Pinned exactly because it is pre-1.0 and ships about 5 releases a quarter | fltk-rs (draws its own widgets; documented swap in §3). egui/iced (GPU stack, 40-100 MB RSS, unverified estimate). Slint (licence terms, non-native look). GTK4 (bundling GTK on Windows and macOS). Webviews (ruled out by the requirement). boing/libui-ng (abandoned: last release 2023-02) |
| Audio | `cpal = "0.18"` (0.18.2, 2026-08-16), default features (WASAPI / CoreAudio / ALSA) | Thin callback API with no mixer thread. Streams on the default device reroute themselves when the output changes. Pinning `0.18` keeps `play()`; master renames it to `start()` and removes `ErrorKind::Xrun` | rodio (mixer and decoders we don't need). The `pipewire`, `jack` and `realtime*` features (extra build deps; our buffers are large) |
| PRNG + DSP | Hand-written in `crates/core/src/noise.rs` (~80 lines, plus the test): xorshift64, Kellet pink, leaky-integrator brown, first differences for blue and violet | Measured at 1.07-2.84 ns per frame. The PRNG period is 2^64, about 12 million years at 48 kHz | `rand` (a dependency for 5 lines), rustfft |
| Player → audio | `static GAIN: AtomicU32` (f32 bits) and `static KIND: AtomicU8`, both `Relaxed`, private to `audio.rs` behind `set_gain()`, `kind()` and `set_kind()` | Only two independent scalars cross threads. No lock, no allocation, no `Arc`. Mute is just a target gain of 0 | rtrb, triple_buffer, Mutex |
| Audio → player (device lost) | The cpal error callback calls `on_lost()`, which sends `AudioLost` over the player's mpsc channel | Wakes the player thread only when something is wrong. No polling, no timer | "Recover on next user action" (P1). A 1 s poll timer (P3) |
| Click-free changes | One-pole gain smoother per sample, τ = 10 ms, `k = 1 - exp(-1/(0.01·sr))`. A change of noise type dips to 0, switches, then ramps back up | One multiply-add per sample. A new stream fades in because the smoothed gain starts at 0 | Crossfade buffer or a second generator |
| Errors | Hand-written `AudioError { NoDevice, Format(SampleFormat), Cpal(cpal::Error) }` with `From` and `Display`. Errors are logged with `eprintln!` plus context and reported to frontends in `State.error` | Explicit errors with no extra dependency | thiserror, anyhow, log, env_logger |
| Build profile | `opt-level="s"`, `lto=true`, `codegen-units=1`, `panic="abort"`, `strip=true` | Binary size is the cost that matters; DSP speed is too small to measure. `panic=abort` means a panic in the callback can never unwind across FFI. Compare against `opt-level=3` once | Default release profile |
| Packaging | Plain `cargo build --release` binary per OS | v1 is not being distributed yet | .msi, .app/.dmg, Flatpak, CI matrix (all deferred) |
| Tests | `#[cfg(test)]` in `noise.rs` (level and bounds), `voice.rs` (a kind change fades through silence), `protocol.rs` (round trip) and `tui.rs` (color mapping) | Checks level and boundedness, which is where the bugs are (see §4), plus the fade and the wire format | Benchmark harness, per-function test suites |

**Build prerequisites (Linux):** a Rust toolchain, `pkg-config` and `libasound2-dev` (cpal's ALSA host is always compiled in, even with the `pulseaudio` feature). No C++ toolchain. The wxDragon prerequisites (CMake, a C++ compiler, GTK3 and libclang headers, MSVC on Windows) went with wxDragon on 2026-11-23.

## 3. The "native windows" interpretation

The phrase can mean either of two things, and the choice changes the toolkit, the binary size and the toolchain:

- **Strict reading: the OS's own widgets.** This is the default in this design. It uses wxDragon and gives a native look, theming, keyboard navigation and screen-reader support.
  - Costs: a C++/CMake toolchain and a binary of about 5-12 MB (an estimate I could not verify).
- **Loose reading: a real OS window, not a webview.** This uses `fltk = "1.5"` (latest is 1.5.24). The README says the binary is about 1 MB stripped.
  - Costs: the widgets do not look native, screen-reader support is weak, and on Wayland sessions the app runs through XWayland unless you enable the `use-wayland` feature, which needs extra -dev packages.
  - With `fltk-bundled` you no longer need CMake, but you still need a C++ linker and runtime.
  - Only `main.rs` would change (about 100 lines): `menu::Choice`, `valuator::HorNiceSlider` and `button::CheckButton` for the controls; `app::channel::<Msg>()` instead of `call_after` (its `send()` calls `Fl::awake`, which wakes `app::wait()`); `app::add_timeout3` instead of `Timer`.

`audio.rs` and `noise.rs` are written without depending on either toolkit, so the swap is cheap. Even so, **ask the user before scaffolding `main.rs`**.

## 4. Noise generation

### Algorithms

All rates below are 48 kHz. `w` is xorshift64 output, top 24 bits mapped to [-1, 1). Every type is normalised to about -18 dBFS RMS so switching types does not change loudness, and the output is clamped to [-1, 1].

| Type | Algorithm | Output gain | Level, simulated over 10 s | Cost per frame* |
|---|---|---|---|---|
| White | `w` | ×0.218 | -18.00 dBFS | 1.07 ns |
| Pink | Paul Kellet "refined": 7 one-pole filters, fixed coefficients | ×0.0715 | -18.13 dBFS | ~2.7 ns |
| Brown | `y = a·y + b·w`, with `a = exp(-2π·15.3/sr)` (= 0.998 at 48 kHz) and `b = 0.0138·sqrt((1-a²)/(1-0.998²))` | built into `b` | -18.32 dBFS | ~2.8 ns |
| Blue | `p[n] - p[n-1]` on raw Kellet pink | ×0.120 (1.68 × the pink gain); P1-P3's ×1.09 was refuted and measured -21.8 dBFS | within ±1.5 dB of -18 dBFS (test) | ~2.8 ns |
| Violet | `w[n] - w[n-1]` | ×0.154 | -18.00 dBFS | 1.07 ns |

\* Fact-check bench: rustc `-O`, Ryzen 9 9950X3D, `kind` matched at runtime. The proposals' figure of 1.06-1.67 ns was refuted. The worst case, about 2.84 ns × 48 000, is roughly 136 µs per second of audio, or 0.014% of one core.

Notes on the coefficients:
- **Brown:** `a` and `b` are computed from the device's actual sample rate when the stream is built. The leak sets the corner at about 15 Hz and is also the only DC control.
- **Kellet pink** stays at fixed coefficients. Above 48 kHz its spectrum tilts slightly.
  - ponytail: accepted. If anyone notices, switch to a pink design that scales with the sample rate.

The single test in `noise.rs` generates 10 s of each type and checks:
- every sample is finite and within [-1, 1];
- RMS is within ±1.5 dB of -18 dBFS;
- `|mean| < 0.01` for white, pink, blue and violet, and `|mean| < 0.03` for brown. Brown fails the 0.01 bound for about 10% of seeds (worst case 0.0152).

### Periodicity: why nothing is looped

- **None of these signals has a period.** They are stationary random processes with continuous spectra. The only period in the system is the PRNG's: 2^64 samples, about 12.2 million years at 48 kHz.
- **"Generate one period and loop it" really means frozen noise, which is a different signal.** Listeners detect repetition at loops of 10 s or more (Warren et al. 2001). Agus et al. 2010 showed that people learn repeated noise within a single session and remember it for weeks.
- **A loop long enough to hide that would need 30-60 s.** This figure is an extrapolation, not a cited finding.
  - Memory: 5.8-11.5 MB per type (mono f32 at 48 kHz), or 29-58 MB for all 5 types. Live generation needs under 64 B of state.
  - Seams: pink and brown loops would also need a crossfade or IFFT synthesis to avoid a click where the loop restarts.
  - CPU saved: at most about 130 µs per second (live ≤ 2.84 ns/frame versus a ~0.1 ns copy). That is invisible next to the OS mixer.
- **Decision: generate everything live.** The one-period wavetable idea is reserved for genuinely periodic sources added later, such as tones or binaural beats.

## 5. Architecture

### Threads

| Thread | What it does |
|---|---|
| Player (daemon main) | Owns `Player` and with it the stream. Blocks on the mpsc channel; uses `recv_timeout` only while a mute fade is pending |
| Accept | Accepts socket connections and spawns a reader per client |
| Client reader (one per connection) | Parses request lines and forwards them to the player thread as `Msg::Request` |
| cpal real-time | Runs the data callback. Owns `Voice` by `move`. Never allocates, locks, logs or makes syscalls |
| cpal error callback | Not real-time. Logs the error, and on device loss sends `Msg::AudioLost` to the player thread |

### Shared state sketch

```rust
// crates/core/src/lib.rs
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Kind { White, Pink, Brown, Blue, Violet }
impl Kind {
    pub const ALL: [Kind; 5] = [Kind::White, Kind::Pink, Kind::Brown, Kind::Blue, Kind::Violet];
    pub fn label(self) -> &'static str { /* match */ }
    pub fn from_u8(v: u8) -> Kind { Self::ALL.get(v as usize).copied().unwrap_or(Kind::White) }
    pub fn parse(s: &str) -> Option<Kind>;                // case-insensitive label
}
pub fn gain(vol: i32) -> f32;                             // (vol/100)^2, vol clamped to 0..=100

// crates/core/src/noise.rs
pub struct Noise { rng: u64, pink: [f32; 7], brown: f32, brown_a: f32, brown_b: f32, prev: f32 }
impl Noise {
    pub fn new(sample_rate: u32, seed: u64) -> Self;      // rng = seed | 1
    pub fn next(&mut self, kind: Kind) -> f32;            // normalised, clamped
}

// crates/core/src/voice.rs
pub struct Voice { noise: Noise, playing: Kind, level: f32, smoothing: f32 }
impl Voice {
    pub fn new(sample_rate: u32, seed: u64, kind: Kind) -> Self;   // level starts at 0
    pub fn step(&mut self, want: Kind, gain: f32) -> f32;          // one smoothed sample
}

// src/backend/audio.rs
static GAIN: AtomicU32 = AtomicU32::new(0);                   // target linear gain, f32 bits; written via set_gain()
static KIND: AtomicU8  = AtomicU8::new(Kind::Brown as u8);    // read/written via kind()/set_kind()

pub enum AudioError { NoDevice, Format(cpal::SampleFormat), Cpal(cpal::Error) }

pub struct Output { stream: cpal::Stream, rate: SampleRate }  // buffer_time(): one device buffer
pub fn start(on_lost: impl Fn() + Send + 'static) -> Result<Output, AudioError>;
// default device + default config; BufferSize::Fixed(4096) clamped to the supported range (Default if Unknown);
// dispatch F32 / I16 / U16 / I32 / I24 into build::<T: SizedSample + FromSample<f32>>; any other format -> AudioError::Format
// build() seeds a Voice from SystemTime nanos and moves it into the data closure

// src/backend/player.rs (player thread)
pub struct Player {
    on_lost: Arc<dyn Fn() + Send + Sync>,
    stream: Option<audio::Output>,
    drop_stream_at: Option<Instant>,
    vol: i32,
    muted: bool,
    error: Option<String>,
}
```

### Callback, per buffer

```rust
let want = kind();
let gain = f32::from_bits(GAIN.load(Relaxed));
for frame in data.chunks_mut(channels) {
    frame.fill(T::from_sample(v.step(want, gain)));   // mono to all channels
}

// Voice::step
let target = if want == self.playing { gain } else { 0.0 };
self.level += (target - self.level) * self.smoothing;
if (target - self.level).abs() < 1e-6 { self.level = target; }
if want != self.playing && self.level < 1e-4 { self.playing = want; }
self.noise.next(self.playing) * self.level
```

### Data flow

```
 frontends (CLI, TUI, GUI)
   │ request lines / state lines over noiseman.sock
   ▼
 client reader threads ── Msg::Request ──┐
                                         ▼
 player thread (blocks when idle)                      cpal RT thread (only while stream exists)
 ┌──────────────────────────────────────┐              ┌──────────────────────────────────────┐
 │ Kind cmd ── set_kind ────────────────┼──► KIND ────►│ load once per buffer                 │
 │ Vol cmd  ── set_gain(gain(vol)) ─────┼──► GAIN ────►│ Voice::step(want, gain)              │
 │ Mute cmd ── mute/unmute (below)      │              │ → T::from_sample → every channel     │
 │ Player.stream: Option<Output>        │              └──────────────────┬───────────────────┘
 │ audio_lost() ◄── Msg::AudioLost ─────┼──────────── cpal err callback ◄─┘ (DeviceNotAvailable |
 │ State.error ◄── start() errors       │               logs every error     StreamInvalidated)
 └──────────────────────────────────────┘
```

### Lifecycle

All of this lives in `Player`, on the daemon's player thread.
- **Startup:** `Player::new(on_lost, muted)` starts at volume 50 and, unless `--muted`, sets `GAIN = gain(vol)` and calls `start()`. The new stream fades in because the `Voice` level starts at 0.
- **Volume:** `set_vol` writes `GAIN`, or 0 while muted. The smoother removes zipper noise.
- **Type change:** `set_kind`. The audio callback ramps to about 0 (≤ ~90 ms at full volume), switches generator, and ramps back up.
- **Mute:**
  1. Set `GAIN = 0`.
  2. Set the deadline `drop_stream_at = now + 100 ms + 3 × Output::buffer_time()` (PulseAudio queues up to 3 buffers). The buffer time comes from `Stream::buffer_size()` when the host reports it (the cpal docs say not to trust it as a bound), otherwise from the requested 4096 frames. That is about 355 ms at 48 kHz.
  3. The player loop waits with `recv_timeout` until the deadline, then `tick()` drops the stream. That closes the device: 0 callbacks, 0 CPU.
  - The 100 ms term covers the fade-out. P1's "2 × buffer + 20 ms" would cut the fade short on hosts with 10 ms buffers.
- **Unmute:** clears the deadline and restores `GAIN`. If the stream is already gone, calls `start()`, which fades in.
- **Start failure:** `start()` errors are logged, stored in `State.error` for frontends, and set `muted = true`, so the user can retry by unmuting.
- **Device lost:** the error callback matches `DeviceNotAvailable | StreamInvalidated` and calls `on_lost()`, which sends `Msg::AudioLost` to the player thread. Every other error kind is only logged. Do not match `Xrun` explicitly, because it is gone in 0.19.
  - `audio_lost()` drops the stream and, if not muted, calls `start()` on the current default device.
  - Duplicate messages are harmless: each one just rebuilds the stream.
- **Quit:** mutes, removes the socket, then sleeps until the mute deadline so the stream fades out instead of being cut mid-buffer.

## 6. Module and file layout

```
noiseman/
  Cargo.toml          workspace (crates/core, web); deps noiseman-core, cpal 0.18 (pulseaudio), crossterm, libc;
                      gui feature: winit, softbuffer, font8x8, ksni (Linux); [patch.crates-io] vendor/cpal, vendor/pulseaudio
  crates/core/src/    lib.rs (Kind, gain), noise.rs (Noise + test), voice.rs (Voice + test)
  src/main.rs         daemon vs frontend dispatch
  src/protocol.rs     Command, State, line encoding, runtime dir and socket path
  src/backend/        mod.rs (serve, lock, socket, player loop), player.rs (Player), audio.rs (atomics, start, build)
  src/frontend/       mod.rs, client.rs, cli.rs, tui.rs, gui/{mod,tray}.rs
  vendor/             patched cpal and pulseaudio
  web/                wasm cdylib (src/lib.rs) + site/
  assets/             .desktop file and icons
```

A three-member workspace (the root binary, `crates/core` and `web`). There are no traits and no config module.

## 7. Efficiency budget

All figures except the DSP cost are targets to check on the first release build. The RSS, binary size and startup figures are **unverified estimates**.

| Metric | Target | How to measure |
|---|---|---|
| Wakeups while muted or idle | **0/s from our process. This is a hard fail if it is not met** | Linux: `pidstat -w -p $PID 1` (cswch/s ≈ 0) and `perf stat -e context-switches -p $PID -- sleep 10`. macOS: Activity Monitor, "Idle Wake Ups". Windows: Process Explorer, context-switch delta |
| CPU while muted or idle | 0.0% | `top` / Task Manager / Activity Monitor |
| Callbacks while playing | ~12/s where 4096 frames is honoured; ~100/s on WASAPI shared mode; set by the graph quantum on PipeWire | `perf stat -e context-switches` |
| CPU while playing (our process) | < 0.3% of one core. DSP ≈ 0.014%; the rest is wakeups | `pidstat -u -p $PID 1` |
| RSS | Linux/GTK3 ≤ 30 MB; Windows and macOS ≤ 25 MB. The fltk swap would be ≤ 10 MB | `/proc/$PID/status` VmRSS, or the OS tools |
| Heap growth while playing | 0 (our heap < 100 KB; DSP state ~72 B per stream: `Noise` 56 B + fade state) | VmRSS stays flat over 10 min |
| Binary, stripped | ≤ 12 MB (wx); ~1-3 MB with fltk | `ls -l target/release/noiseman`. Compare `opt-level="s"` against `3` once |
| Startup to first window | < 300 ms | `time`, or a stopwatch on first frame |

## 8. Risks and open questions

**Questions for the user. Q1 blocks work; the others have defaults.**
1. Do you need native widgets (wxDragon: 5-12 MB, C++ toolchain) or just a real OS window (fltk-rs: ~1-3 MB, non-native look)?
2. Is a C++/CMake toolchain acceptable on every build machine? On Windows this design assumes MSVC.
3. Is the noise set right? The default is white, pink, brown, blue and violet. Grey noise is deferred.
4. Is mono output to every channel acceptable, rather than decorrelated stereo?

**Risks**
- **wxDragon build friction:** prebuilt wx libraries are downloaded on the first build, MinGW needs an exact GCC version, and the API can change between pre-1.0 releases. Mitigation: exact pin, MSVC only on Windows.
- **Windows manifest:** *unverified* whether wxDragon embeds a Common Controls v6 and PerMonitorV2 DPI manifest. Without one, the native controls look dated and blur on HiDPI screens. Check on the first Windows build. If it is missing, add `assets/noiseman.manifest` plus a `build.rs` (one build dependency, e.g. `embed-resource`).
- *Unverified:* whether wxDragon widget handles are `Send`. The `thread_local` design avoids needing them to be.
- *Unverified:* the exact `call_after` signature. The fact-check confirmed `call_after<F: FnOnce() + Send + 'static>(Box<F>)` in the 0.9.25 docs; the compiler will catch any drift.
- **Buffer size:** `BufferSize::Fixed` is only a request. WASAPI shared mode and PipeWire choose the real rate, so CPU while playing varies between machines. Correctness is unaffected.
- **Linux audio path (resolved):** with default cpal features, audio went through ALSA `default` → pipewire-alsa or pulse plugin, appeared as "ALSA plug-in" in the mixer and followed changes of default sink poorly. cpal now uses the pure-Rust `pulseaudio` feature, and cpal and pulseaudio are both vendored with patches (`[patch.crates-io]` in `Cargo.toml`).
- **Stream rebuild on unmute:** costs tens of ms, and Bluetooth sinks may take longer to wake. Some backends may pop when the device opens; the fade-in from level 0 should cover it. Test on Linux and macOS.
- **macOS:** a bare binary has no Dock identity or app menu, and Gatekeeper blocks unsigned downloads. Fine for local v1.
- **Sample-rate tilt:** Kellet pink tilts slightly above 48 kHz (accepted, see §4).
- **Unmeasured figures:** the RSS, binary size and startup numbers are estimates. The design is judged on the measurements from §7.

## 9. Deliberately skipped in v1

- Looped or one-period noise buffers, FFT synthesis and rustfft: the signals are aperiodic (§4).
- Grey noise (needs ISO 226 equal-loudness tuning), velvet noise and decorrelated stereo: add when asked.
- Saving the last noise type and volume: add when asked (one small `std::fs` file, no serde).
- Device picker, sample-rate, buffer or latency settings: the app always uses the default device at its native format.
- Global or media keys, sleep timer, EQ, presets. (A Linux tray icon shipped behind the `gui` feature; see the 2026-11-23 revision.)
- cpal `pipewire`, `jack`, `realtime` and `realtime-dbus` features.
- Crates: rodio, rand, rtrb, triple_buffer, thiserror, anyhow, log, env_logger, serde.
- Trait abstractions over the GUI toolkit or audio backend: there is one implementation of each, and swapping means rewriting `src/frontend/gui/` or `src/backend/audio.rs`.
- Installers, the .app bundle and Info.plist, code signing and a CI build matrix: add when the app is distributed (a .desktop file and icons are in `assets/`).
- A benchmark harness and per-function test suites. Tests cover the noise level, the voice fade, the protocol round trip and the TUI color mapping, plus a manual listen on Linux and macOS.
