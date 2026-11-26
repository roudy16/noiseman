# Why noiseman is built this way

This file records the decisions behind noiseman: what was chosen, what it costs, and what was rejected. `AGENTS.md`
describes the layout and the rules that keep these decisions true; [docs/architecture.html](docs/architecture.html)
walks through the threads, states, protocol and sound core with diagrams. Earlier plans, including the original
single-window wxDragon design, are in the git history.

The aim throughout: a background noise player that costs nothing while muted and close to nothing while playing.
Current limits and baselines are in `gates.toml`.

## A daemon with thin frontends

`noiseman daemon` owns the audio stream and the volume and mute state; the CLI, the TUI and the window are clients of
a line protocol on a Unix socket (`src/protocol.rs`) and never touch audio. The daemon outlives them: the TUI and the
window start it when they open, CLI commands other than `status`, `mute` and `stop` start it if needed, and `stop` (or
`s` in the TUI and the window, Quit in the tray menu) ends it.

Why: measured playing on Linux on 2026-11-24, the earlier single process used 8.6 MB RSS and 4.8 MB private memory.
The daemon used 4.0 MB RSS and 1.2 MB private with a 0.79 MB binary (was 3.85 MB), and an open TUI added 3.2 MB RSS,
0.2 MB private. Keeping the window out of the process that plays also means its toolkit's memory is paid only while
it is open.

### The transport on Windows

Windows 10 1803 and later have AF_UNIX sockets, so the daemon listens on `noiseman.sock` there too, and the backend,
client and protocol are the same code on every OS. std does not offer AF_UNIX on Windows, so `src/platform/windows.rs`
carries the subset of its API noiseman uses. The socket file inherits the runtime dir's ACL
(`%LOCALAPPDATA%\noiseman`: the user, SYSTEM and administrators), and connecting needs write access, which a
low-integrity process of the same user lacks: the same trust as the Unix socket in a 0700 directory.

Rejected: a named pipe. The pipe namespace is global, so the first version needed a random published name, an
owner-only ACL, owner and integrity checks and overlapped handles with cancellable writes; replacing it with AF_UNIX
took `windows.rs` from 704 to 222 lines.

## Generating the noise

All noise is generated live, per sample. None of these signals has a period: they are stationary random processes
with continuous spectra, and the only period in the system is the PRNG's, 2^64 samples (about 12 million years at
48 kHz). Looping a buffer means frozen noise, a different signal: listeners detect repetition in loops of 10 s or more
(Warren et al. 2001), and people learn repeated noise within a session and remember it for weeks (Agus et al. 2010).
A loop long enough to hide that (30-60 s, an extrapolation rather than a cited finding) would take 5.8-11.5 MB per
kind as mono f32 at 48 kHz, and pink and brown loops would need a crossfade or IFFT synthesis to hide the seam. Live
generation needs under 64 bytes of state. A one-period wavetable is kept in mind for genuinely periodic sources
(tones, binaural beats), if any are added.

The generators, in `crates/core/src/noise.rs`:

- **White:** xorshift64. A sample is the top 23 bits used as the mantissa of a float in [1, 2), shifted to [-1, 1);
  the comment on `white()` explains why this beats an integer-to-float conversion on x86.
- **Pink:** Paul Kellet's "refined" filter: six one-pole sections plus direct and one-sample-delayed white terms, with
  fixed coefficients. Above 48 kHz its spectrum tilts slightly; accepted, and a pink design that scales with the
  sample rate is the fix if anyone notices.
- **Brown:** a leaky integrator of white noise, `y = a·y + b·w`, with `a` and `b` computed from the device's actual
  sample rate. The leak puts the corner at about 15 Hz and is the only DC control.
- **Blue:** the first difference of raw Kellet pink. **Violet:** the first difference of white.

Every kind is normalised to about -18 dBFS RMS, so switching kinds does not change loudness, and clamped to [-1, 1].
The test checks the level to ±1.5 dB over 10 s of each kind, plus finiteness, bounds and the mean: `|mean| < 0.01`,
except brown at 0.03, because brown fails the tighter bound for about 10% of seeds (worst seen 0.0152).

The generator is chosen once per buffer, not per sample (`Voice::fill`), so the inner loop is one kind's generator
plus a well-predicted branch for the fade. That took brown from about 2.7 to 1.2 ns per sample; the worst kind, blue,
runs at about 2.5 ns on the Linux baseline machine (`gates.toml` `[dsp]`), about 120 µs per second of audio at 48 kHz,
around 0.01% of one core. Rejected: the `rand` crate (a dependency for five lines) and rustfft.

## Volume and fades

Volume 0-100 maps to gain on a square law (`gain()`), so the low end of the range stays usable.

Every gain change, mute included, reaches its target in exactly 20 ms (`FADE_SECS`; 960 samples at 48 kHz) along the
smoothstep curve `3t² - 2t³`. The duration is the same on every machine and rate, the slope is zero at both ends, and
the fade lands exactly, so there is no settling heuristic. The level is evaluated directly from the samples left,
because forward differences in f32 drift by up to 1.6e-4 of the span. A new target mid-fade starts a new fade from the
current level. A kind change fades to exactly 0, switches generators, and fades back up (40 ms in all). A new stream
fades in because the level starts at 0, which also covers a pop where a device makes one on opening. The fade was
100 ms at first and sounded slow. Rejected: a crossfade buffer or a second generator.

## Between the player and the audio callback

Only two independent scalars cross from the player thread to the real-time callback: the target gain and the kind.
They are two atomics (`GAIN` as f32 bits, `KIND`), read once per buffer with `Relaxed` ordering; mute is a target gain
of 0. No lock, no allocation, no `Arc`. Rejected: rtrb, triple_buffer, a `Mutex`.

The other direction happens only when something goes wrong: the cpal error callback sends `AudioLost` over the player
thread's channel on device loss, and the player rebuilds the stream on the current default device. Rejected: polling
on a timer, or recovering only on the next user action.

## Buffer size

`BUFFER_FRAMES` is 4096 frames, about 85 ms at 48 kHz. PulseAudio keeps two buffers queued, so a change of volume or
kind is heard about 170 ms after the key press, plus the fade. In exchange the daemon wakes about 20 times a second
while playing. Measured on 2026-11-25: 2048 frames gives about 85 ms of lag for 45 wakeups/s, 1024 about 43 ms for
90/s, 512 about 21 ms for 185/s. The lag was judged the better trade. Rejected: flushing the queue on each change,
which would need another vendored PulseAudio patch, separate work on Windows and macOS, and could click on brown noise.

`BufferSize::Fixed` is only a request. WASAPI shared mode wakes the stream every 10 ms whatever is asked (the Windows
gates allow for it), and PipeWire may choose its own quantum, so CPU while playing varies between machines.

## Muted means no stream

Muting drops the audio stream once the fade has played out, so a muted daemon makes no wakeups and uses no CPU. The
deadline is `FADE_SECS` plus 20 ms of scheduling slack plus four device buffers: one for the callback to pick up the
new gain and up to three already queued, about 380 ms with 4096-frame buffers. The buffer time comes from the stream
when the host reports it, else from the requested size. Quitting mutes and waits out the same deadline, so the sound
fades instead of being cut mid-buffer. Unmuting a dropped stream opens a new one, which fades in.

The cost is a stream open on every unmute and on device loss. A start failure is logged, shown to frontends in the
state's error field, and leaves the player muted, so unmuting retries.

## Audio on Linux

cpal uses its `pulseaudio` host, which talks to PulseAudio or PipeWire's pulse server directly. Through ALSA's
`default` device, audio went through the ALSA-to-PipeWire plugin, showed as "ALSA plug-in" in the mixer, followed
changes of default sink poorly, and cost about 4 MB and one more thread (measured 2026-11-23). ALSA's `default`
device is the fallback when the pulse host cannot connect. If both fail, the error is `AudioError::NoPulse`, because
ALSA's default device usually routes to the same pulse server and its own error would hide the real cause. cpal's ALSA
host is always compiled in, so building still needs the ALSA headers. The pulse host reports the sink's own format as
the default, I32 on PipeWire, so the callback is built for every common sample format.

cpal and pulseaudio are vendored with small patches (latency-thread wakeups, 64 KiB socket reads, a hang on a killed
stream, finding the server socket, following the default sink); `AGENTS.md` lists each one and its reason. Rejected:
rodio (a mixer and decoders noiseman does not need), and cpal's `pipewire`, `jack` and `realtime` features (extra
build dependencies, and the buffers are large enough not to need real-time scheduling).

## The window

The window is a bare winit window drawn on the CPU with softbuffer, with text from the font8x8 bitmap font and
controls drawn by hand; it redraws only when an event arrives. No GPU, no toolkit, no C++ build.

Why: measured playing on Linux on 2026-11-23, the planned wxDragon window used 53 MB RSS and 17.8 MB private, about
45 MB of it GTK3 and wxWidgets. The bare window used 8.2 MB RSS and 4.5 MB private, with a 2.5 MB binary. The cost is
accepted for this app: no native look, no accessibility, no IME, integer HiDPI scaling, and ASCII text with fake bold
plus a few hand-made glyphs. GPU toolkits (egui, iced) were rejected unmeasured, on an estimate of 40-100 MB RSS.

The TUI is the reference look: the window and the web demo copy its card (logo, status, kinds, volume, error line,
key hints, themed to the current kind) and its keys mean the same in all three.

The Linux tray is ksni (pure-Rust StatusNotifierItem and dbusmenu) with its `blocking` and `tokio` features: measured
at +1.3 MB of binary, two idle threads with no wakeups and about +1.4 MB RSS while playing. Its `async-io` backend was
rejected: four more threads, one of them waking every 500 ms. D-Bus calls have no timeout, so they run on their own
thread, and a stalled tray watcher cannot freeze the window. Wayland cannot hide a window, so closing it to the tray
drops the window and its surface; without a tray, closing quits.

## The web demo

The browser demo runs the same `crates/core` compiled to `wasm32-unknown-unknown`, so it sounds the same as the
daemon. It exports two raw functions over one static synth, with no wasm-bindgen and no heap, which keeps the module
to a few kilobytes (`gates.toml` `[size]`). An AudioWorklet calls it for mono output with `latencyHint: "playback"`,
and the page suspends the AudioContext shortly after every pause, so a paused page does no audio work.

## Errors and logging

`AudioError` is a hand-written enum with `From` and `Display`. The daemon's `log!` macro writes every line to stderr
with a UTC timestamp; when a frontend starts the daemon, stderr is `noiseman.log` in the runtime dir. Frontends see
audio errors in the state's error field. Rejected: thiserror, anyhow, log, env_logger.

## Build and release

The release profile is `opt-level = "s"`, LTO, one codegen unit, `panic = "abort"` and stripped: binary size is the
cost that matters, and the DSP is too cheap for `opt-level = 3` to matter. Where `"s"` did cost measurable time, a
generator is marked `#[inline(always)]` instead (see `noise.rs`). `panic = "abort"` also means a panic can never
unwind across the FFI boundary into the audio callback.

Releases are prebuilt archives per target, in two variants: the terminal build (default features) and the gui build.
Both name the binary `noiseman`, so they are alternatives installed at the same path. On Windows the gui archive adds
`noiseman-gui.exe`, the same program linked for the Windows subsystem, so the Start menu shortcut opens the window
without a console beside it.

The Linux binaries are built against glibc 2.35 in an `ubuntu:22.04` container (the 22.04 runner images are being
retired), so they also run on older distributions than a newer build host would allow. They import nothing newer than
`GLIBC_2.34` (the pthread and dl functions 2.34 moved into libc, and `__libc_start_main`), so 2.34 is the stated
floor: Ubuntu 22.04, Debian 12, Fedora 35, RHEL 9. macOS archives target macOS 11 (`MACOSX_DEPLOYMENT_TARGET`),
one minimum for both architectures, where rustc's defaults are 10.12 for x86_64 and 11.0 for aarch64. Windows needs
10 1803 for AF_UNIX.

## Deliberately not done

- Looped or wavetable noise, FFT synthesis: the signals are aperiodic (see above).
- Grey noise (needs ISO 226 equal-loudness tuning), velvet noise, decorrelated stereo: add when asked. Output is mono,
  written to every channel.
- Saving the last kind and volume: add when asked (one small `std::fs` file, no serde).
- A device picker or sample-rate, buffer or latency settings: the player always uses the default device in its own
  format.
- Global or media keys, a sleep timer, EQ, presets.
- Traits over the GUI toolkit or the audio backend: there is one implementation of each, and swapping one means
  rewriting `src/frontend/gui/` or `src/backend/audio.rs`.
- Native installers (.msi, .dmg), an .app bundle, code signing, Flatpak, Scoop and winget manifests: add when needed.
  Unsigned binaries downloaded in a browser meet Gatekeeper and SmartScreen; the README says how to get past them.
- A benchmark harness and per-function test suites. The tests cover where the bugs are likely: noise level and
  bounds, the fade, the protocol's wire format and the socket contract both OS backends must meet. The DSP gate is
  the one benchmark.
