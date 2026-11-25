# TODO

- [ ] **Test cargo-binstall.** `[package.metadata.binstall]` in `Cargo.toml`
  (`.tar.gz`, with a `cfg(windows)` override for `.zip`) has not been tried
  against a real release. Run `cargo binstall noiseman --git
  https://github.com/roudy16/noiseman` on Linux, macOS and Windows, and check
  it installs `noiseman` from the archive rather than building from source.
- [ ] **GUI on fractional display scaling.** The window asks for 312x124
  logical pixels, but the bitmap font only draws at whole-number scales, so at
  a fractional OS scale the controls leave an empty band on the right and at
  the bottom (seen on Windows at 250%: a 780x310 window drawn at 2x, 624x248).
  Linux with fractional scaling has the same gap. Fix: pick a whole scale from
  the scale factor (e.g. round 2.5 up to 3), size the window to exactly
  WIN_W x WIN_H times that in physical pixels, and redo it on
  `WindowEvent::ScaleFactorChanged` (moving between monitors). Measure the
  GUI at idle before and after (Windows baseline at 2x, attached to a muted
  daemon: 2.8 MB private working set, 12.5 MB working set, 3 own threads,
  0 wakeups/s); a 3x framebuffer is larger.
- [ ] **Rain and thunderstorm.** Two more kinds: steady rain, and rain with
  occasional thunder (a noise bed plus sparse droplet transients; thunder as
  rare low-passed rumbles). Each sets its own readable TUI theme, as the
  noise kinds do (e.g. slate-teal for rain, deep indigo with a pale flash
  accent for thunder). Adds `Kind` variants, so it touches the core's
  generators (and their level test), the protocol, the TUI's `1`-`5` keys,
  the GUI and the web demo's kinds and accents.
