# Contributing

`AGENTS.md` describes the layout, invariants and release process; `DESIGN.md` explains the design decisions. Report bugs
and propose changes as [GitHub issues](https://github.com/roudy16/noiseman/issues/new/choose), ask questions in
[Discussions](https://github.com/roudy16/noiseman/discussions), and report vulnerabilities privately, as `SECURITY.md`
describes. Run `just check` before committing. CI runs `just ci` (fmt check, lint, tests, the web build and the size
gates; the timing and runtime gates are for local runs) on Linux, and clippy and tests on macOS and Windows.

## Build from source

**Prerequisites:** Rust 1.89 or newer (edition 2024; the minimum is `rust-version` in `Cargo.toml`'s
`[workspace.package]`). On Linux, also `pkg-config` and the ALSA development headers (`libasound2-dev` on
Debian/Ubuntu, `alsa-lib-devel` on Fedora, `alsa-lib` on Arch). On Windows, the MSVC toolchain: Visual Studio or its
Build Tools with the "Desktop development with C++" workload, which includes the Windows SDK's `rc.exe` for the
embedded icon.

```sh
git clone https://github.com/roudy16/noiseman.git
cd noiseman
cargo install --path . --features gui   # drop --features gui for a terminal-only build
```

`cargo install` puts `noiseman` in `~/.cargo/bin`; make the short alias yourself (`ln -s noiseman ~/.cargo/bin/nsm`)
or use `alias nsm=noiseman`. On Linux and macOS, `just install` instead builds the terminal version and installs to
`~/.local/bin` with the `nsm` link (leaving an existing `nsm` alone), plus the icons and desktop entry on Linux;
`just install-gui` does the same with the window, and its desktop entry opens the window.

## Development

Dev workflows go through [`just`](https://github.com/casey/just); run `just` alone to list the recipes. Besides the
build prerequisites, they need `just` and Python 3.11 or newer: `tools/gates.py` imports `tomllib`, so `just check` and
`just ci` fail on an older Python such as Ubuntu 22.04's 3.10. `just setup` adds the `wasm32-unknown-unknown` target
the lint and the web demo need. GNU time (`/usr/bin/time`) is optional: without it `just gates` skips the CLI memory
gate.

## Licenses

noiseman is MIT (`LICENSE`), except `vendor/`, which holds two third-party crates under their own licenses, each with
small changes (described at the top of each changed file and in `AGENTS.md`):

- `vendor/cpal`: cpal 0.18.2, Apache License 2.0 (`vendor/cpal/LICENSE`)
- `vendor/pulseaudio`: pulseaudio 0.3.2 by Colin Marc, MIT (`vendor/pulseaudio/LICENSE.md`)

All other dependencies come from crates.io under permissive licenses (MIT, Apache-2.0, BSD-2/3-Clause, ISC, Zlib,
Unlicense, Unicode-3.0); `cargo tree --features gui --target all -e normal,build -f '{p} {l}'` lists them. New
dependencies must be permissive too. The web demo's WebAssembly is built only from `crates/core` and `web/`, with no
third-party crates.
