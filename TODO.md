# TODO

- [ ] **Test cargo-binstall.** `[package.metadata.binstall]` in `Cargo.toml`
  (`.tar.gz`, with a `cfg(windows)` override for `.zip`) has not been tried
  against a real release. Run `cargo binstall noiseman --git
  https://github.com/roudy16/noiseman` on Linux, macOS and Windows, and check
  it installs `noiseman` from the archive rather than building from source.
- [ ] **Measure the GUI at 3x.** At a fractional OS scale the window now draws
  at the nearest whole scale (3x at 250%), so its framebuffer is larger.
  Measure it at idle on Windows at 250% against the 2x baseline (attached to a
  muted daemon: 2.8 MB private working set, 12.5 MB working set, 3 own
  threads, 0 wakeups/s).
- [ ] **Rain and thunderstorm.** Two more kinds: steady rain, and rain with
  occasional thunder (a noise bed plus sparse droplet transients; thunder as
  rare low-passed rumbles). Each sets its own readable TUI theme, as the
  noise kinds do (e.g. slate-teal for rain, deep indigo with a pale flash
  accent for thunder). Adds `Kind` variants, so it touches the core's
  generators (and their level test), the protocol, the TUI's `1`-`5` keys,
  the GUI and the web demo's kinds and accents.
