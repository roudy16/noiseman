# noiseman: design v1

> **Revision (2026-11-23): GUI and audio backend changed after measurement.** This supersedes the wxDragon parts of §2, §3, §5 and §7:
> - **GUI:** wxDragon is replaced by a bare winit 0.30 window, drawn on the CPU with softbuffer 0.4, with text from the font8x8 bitmap font. Controls are drawn by hand; the event loop redraws only when an event arrives. No GPU, no toolkit, no C++ build.
> - **Why:** measured on Linux and playing, wxDragon used 53 MB RSS and 17.8 MB private, of which GTK3 and wxWidgets are about 45 MB. The bare window uses 8.2 MB RSS and 4.5 MB private, with a 2.5 MB binary.
> - **Cost:** no native look, no accessibility, no IME, integer HiDPI scaling and ASCII-only text. All accepted for this app.
> - **Audio:** cpal now uses the `pulseaudio` feature. That avoids the ALSA→PipeWire plugin and saves about 4 MB and one thread. The Pulse host reports I32 as its default format, so `audio.rs` handles `SampleFormat::I32`.
> - **Default noise:** brown.


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
| PRNG + DSP | Hand-written in `noise.rs` (~60 lines): xorshift64, Kellet pink, leaky-integrator brown, first differences for blue and violet | Measured at 1.07-2.84 ns per frame. The PRNG period is 2^64, about 12 million years at 48 kHz | `rand` (a dependency for 5 lines), rustfft |
| UI → audio | `static GAIN: AtomicU32` (f32 bits) and `static KIND: AtomicU8`, both `Relaxed` | Only two independent scalars cross threads. No lock, no allocation, no `Arc`. Mute is just a target gain of 0 | rtrb, triple_buffer, Mutex |
| Audio → UI (device lost) | The cpal error callback calls `wxdragon::call_after(Box::new(restart))` | Wakes the UI thread only when something is wrong. No polling, no timer | "Recover on next user action" (P1). A 1 s poll timer (P3) |
| Click-free changes | One-pole gain smoother per sample, τ = 10 ms, `k = 1 - exp(-1/(0.01·sr))`. A change of noise type dips to 0, switches, then ramps back up | One multiply-add per sample. A new stream fades in because the smoothed gain starts at 0 | Crossfade buffer or a second generator |
| Errors | Hand-written `AudioError { NoDevice, Format(SampleFormat), Cpal(cpal::Error) }` with `From` and `Display`. Errors are logged with `eprintln!` plus context and shown in the status line | Explicit errors with no extra dependency | thiserror, anyhow, log, env_logger |
| Build profile | `opt-level="s"`, `lto=true`, `codegen-units=1`, `panic="abort"`, `strip=true`. `#![windows_subsystem="windows"]` | Binary size is the cost that matters; DSP speed is too small to measure. `panic=abort` means a panic in the callback can never unwind across FFI. Compare against `opt-level=3` once | Default release profile |
| Packaging | Plain `cargo build --release` binary per OS | v1 is not being distributed yet | .msi, .app/.dmg, Flatpak, CI matrix (all deferred) |
| Tests | One `#[cfg(test)]` in `noise.rs` | Checks level and boundedness, which is where the bugs are (see §4) | Benchmark harness, per-function test suites |

**Build prerequisites (fact-checked against the wxDragon README):**
- **All platforms:** CMake and a C++ compiler.
- **Linux:** also `libgtk-3-dev`, `libclang-dev` (needed by bindgen) and `libasound2-dev` (needed by cpal).
- **Windows:** use MSVC (VS2019+). MinGW works only with exactly WinLibs GCC 15.1.0 UCRT.
- **First build:** downloads prebuilt wx libraries, so it needs network access.

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
| Blue | `p[n] - p[n-1]` on raw Kellet pink | **≈ ×0.120 (1.68 × the pink gain)**; P1-P3's ×1.09 was refuted and measured -21.8 dBFS | tune in the test | ~2.8 ns |
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
| UI (main) | Runs the wx event loop and blocks in the OS while idle. Owns the `cpal::Stream` and the widgets |
| cpal real-time | Runs the data callback. Owns `Voice` by `move`. Never allocates, locks, logs or makes syscalls |
| cpal error callback | Not real-time. Logs the error, and on device loss posts `restart` to the UI thread |

### Shared state sketch

```rust
// noise.rs
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind { White, Pink, Brown, Blue, Violet }
impl Kind {
    pub const ALL: [Kind; 5] = [Kind::White, Kind::Pink, Kind::Brown, Kind::Blue, Kind::Violet];
    pub fn label(self) -> &'static str { /* match */ }
    pub fn from_u8(v: u8) -> Kind { Self::ALL.get(v as usize).copied().unwrap_or(Kind::White) }
}
pub struct Noise { rng: u64, pink: [f32; 7], brown: f32, brown_a: f32, brown_b: f32, prev: f32 }
impl Noise {
    pub fn new(sample_rate: u32, seed: u64) -> Self;   // seed: SystemTime nanos | 1
    pub fn next(&mut self, kind: Kind) -> f32;         // normalised, clamped
}

// audio.rs
pub static GAIN: AtomicU32 = AtomicU32::new(0);                  // target linear gain, f32 bits (0.0 == 0)
pub static KIND: AtomicU8  = AtomicU8::new(Kind::Pink as u8);

pub enum AudioError { NoDevice, Format(cpal::SampleFormat), Cpal(cpal::Error) }

struct Voice { noise: Noise, cur: Kind, g: f32, k: f32 }         // moved into the data closure

pub fn start(on_lost: impl Fn() + Send + 'static) -> Result<cpal::Stream, AudioError>;
// default device + default config; BufferSize::Fixed(4096) clamped to the supported range (Default if Unknown);
// dispatch F32 / I16 / U16 into build::<T: SizedSample + FromSample<f32>>; any other format -> AudioError::Format

// main.rs
// ponytail: thread_local because call_after needs a Send closure and Stream/widgets are not Send everywhere.
thread_local! { static APP: RefCell<Option<App>> = const { RefCell::new(None) }; }
struct App { stream: Option<cpal::Stream>, status: StaticText, mute: CheckBox, vol: Slider }
```

### Callback, per buffer

```rust
let want = Kind::from_u8(KIND.load(Relaxed));
let gain = f32::from_bits(GAIN.load(Relaxed));
for frame in data.chunks_mut(channels) {
    let target = if want != v.cur { 0.0 } else { gain };
    v.g += (target - v.g) * v.k;
    if want != v.cur && v.g < 1e-4 { v.cur = want; }
    frame.fill(T::from_sample(v.noise.next(v.cur) * v.g));   // mono to all channels
}
```

### Data flow

```
 UI thread (wx loop, blocks when idle)                 cpal RT thread (only while stream exists)
 ┌──────────────────────────────────────┐              ┌──────────────────────────────────────┐
 │ Choice  ── KIND.store ───────────────┼──► KIND ────►│ load once per buffer                 │
 │ Slider  ── GAIN.store((v/100)^2) ────┼──► GAIN ────►│ smoother g → Noise::next(cur) * g    │
 │ CheckBox── mute/unmute (below)       │              │ → T::from_sample → every channel     │
 │ APP.stream: Option<cpal::Stream>     │              └──────────────────┬───────────────────┘
 │ restart() ◄── call_after ────────────┼──────────── cpal err callback ◄─┘ (DeviceNotAvailable |
 │ status: StaticText ◄── errors        │               logs every error     StreamInvalidated)
 └──────────────────────────────────────┘
```

### Lifecycle

- **Startup:** `GAIN = vol²`, then `stream = start(...)`. The new stream fades in because `g` starts at 0. On `Err`, show the message in the status line, check the mute box, and log it.
- **Slider:** writes `GAIN` only while not muted. The smoother removes zipper noise.
- **Type change:** `KIND.store`. The audio callback ramps to about 0 (≤ ~90 ms at full volume), switches generator, and ramps back up.
- **Mute:**
  1. Set `GAIN = 0`.
  2. Start a one-shot `Timer` of `100 ms + 2 × buffer duration`. The buffer duration comes from `Stream::buffer_size()` when the host reports it (the cpal docs say not to trust it as a bound), otherwise from the requested 4096 frames. That is about 270 ms at 48 kHz.
  3. When the timer fires, if the box is still checked, set `stream = None`. That closes the device: 0 callbacks, 0 CPU.
  - The 100 ms term covers the fade-out. P1's "2 × buffer + 20 ms" would cut the fade short on hosts with 10 ms buffers.
- **Unmute:** if the stream still exists (the timer hasn't fired yet), just restore `GAIN`. Otherwise set `GAIN = vol²` and call `start()`, which fades in. If `start()` fails, the status line shows why and the box stays checked, so the user can retry by unchecking it.
- **Device lost:** the error callback matches `DeviceNotAvailable | StreamInvalidated` and calls `on_lost()`, which is `call_after(restart)`. Every other error kind goes to a wildcard arm that only logs it. Do not match `Xrun` explicitly, because it is gone in 0.19.
  - On the UI thread, `restart` drops the stream and, if not muted, calls `start()` on the current default device. Errors go to the status line.
  - Duplicate posts are harmless: each one just rebuilds the stream.

## 6. Module and file layout

```
noiseman/
  Cargo.toml     wxdragon "=0.9.25", cpal "0.18", release profile
  src/main.rs    ~100 lines: Frame and widgets, APP thread_local, event handlers, mute timer, restart()
  src/audio.rs   ~90 lines:  GAIN/KIND, AudioError, start(), build::<T>(), Voice + callback, error callback
  src/noise.rs   ~70 lines:  Kind, Noise, next(), the one #[cfg(test)]
```

There is no lib.rs, no traits, no config module and no workspace.

## 7. Efficiency budget

All figures except the DSP cost are targets to check on the first release build. The RSS, binary size and startup figures are **unverified estimates**.

| Metric | Target | How to measure |
|---|---|---|
| Wakeups while muted or idle | **0/s from our process. This is a hard fail if it is not met** | Linux: `pidstat -w -p $PID 1` (cswch/s ≈ 0) and `perf stat -e context-switches -p $PID -- sleep 10`. macOS: Activity Monitor, "Idle Wake Ups". Windows: Process Explorer, context-switch delta |
| CPU while muted or idle | 0.0% | `top` / Task Manager / Activity Monitor |
| Callbacks while playing | ~12/s where 4096 frames is honoured; ~100/s on WASAPI shared mode; set by the graph quantum on PipeWire | `perf stat -e context-switches` |
| CPU while playing (our process) | < 0.3% of one core. DSP ≈ 0.014%; the rest is wakeups | `pidstat -u -p $PID 1` |
| RSS | Linux/GTK3 ≤ 30 MB; Windows and macOS ≤ 25 MB. The fltk swap would be ≤ 10 MB | `/proc/$PID/status` VmRSS, or the OS tools |
| Heap growth while playing | 0 (our heap < 100 KB; DSP state < 64 B) | VmRSS stays flat over 10 min |
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
- **Linux audio path:** with default cpal features, audio goes through ALSA `default` → pipewire-alsa or pulse plugin. If that plugin is missing, there is no sound. The app appears as "ALSA plug-in" in the mixer and follows changes of default sink poorly.
  - Fix: one line, the pure-Rust `pulseaudio` feature (adds the `futures` and `portable-atomic` crates).
- **Stream rebuild on unmute:** costs tens of ms, and Bluetooth sinks may take longer to wake. Some backends may pop when the device opens; the fade-in from `g = 0` should cover it. Test on all three OSes.
- **macOS:** a bare binary has no Dock identity or app menu, and Gatekeeper blocks unsigned downloads. Fine for local v1.
- **Sample-rate tilt:** Kellet pink tilts slightly above 48 kHz (accepted, see §4).
- **Unmeasured figures:** the RSS, binary size and startup numbers are estimates. The design is judged on the measurements from §7.

## 9. Deliberately skipped in v1

- Looped or one-period noise buffers, FFT synthesis and rustfft: the signals are aperiodic (§4).
- Grey noise (needs ISO 226 equal-loudness tuning), velvet noise and decorrelated stereo: add when asked.
- Saving the last noise type and volume: add when asked (one small `std::fs` file, no serde).
- Device picker, sample-rate, buffer or latency settings: the app always uses the default device at its native format.
- Tray icon, global or media keys, sleep timer, EQ, presets.
- cpal `pipewire`, `jack`, `realtime` and `realtime-dbus` features.
- Crates: rodio, rand, rtrb, triple_buffer, thiserror, anyhow, log, env_logger, serde.
- Trait abstractions over the GUI toolkit or audio backend: there is one implementation of each, and swapping means rewriting `main.rs`.
- Installers, the .app bundle and Info.plist, a .desktop file, code signing and a CI matrix: add when the app is distributed.
- Tests beyond the one `noise.rs` check, plus a manual listen on each OS.
