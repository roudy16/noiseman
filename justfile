# noiseman — all dev workflows go through just

set positional-arguments

# list recipes
default:
    @just --list

# fetch dependencies and add the wasm32 target
setup:
    cargo fetch
    rustup target add wasm32-unknown-unknown

# compile
build:
    cargo build

# run tests
test:
    cargo test --workspace

# run the binary, passing args through
run *args:
    cargo run --quiet -- "$@"

# format the code
fmt:
    cargo fmt --all

# clippy with warnings as errors: default build, the gui feature, and the web crate for wasm32
lint:
    cargo clippy --workspace --all-targets -- -D warnings
    cargo clippy --workspace --all-targets --features noiseman/gui -- -D warnings
    cargo clippy -p noiseman-web --target wasm32-unknown-unknown -- -D warnings

# format check, lint, tests and quality gates (run before committing)
check:
    cargo fmt --all --check
    just lint
    just test
    just gates

# quality gates: artifact sizes, DSP speed, daemon memory/threads/CPU/wakeups (playing and muted), TUI and CLI
# memory; limits and baselines in gates.toml
gates: web
    cargo build -q --release
    cargo build -q --release -p noiseman-core --example noise_profile
    python3 tools/gates.py

# Windows runtime gates, the counterparts of gates' /proc ones; limits in gates.toml [windows]
gates-windows:
    cargo build -q --release
    powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/winperf.ps1 -Bin target/release/noiseman.exe

# dependency health (network, ~1 s): advisories for every locked crate (OSV/RustSec), and drift of direct
# dependencies and vendored crates from their latest stable release; exits 1 only on a vulnerability
deps:
    python3 tools/deps.py

# install noiseman and an `nsm` link to ~/.local/bin for the current user; on Linux also icons and a desktop entry
install:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build --release
    bin="$HOME/.local/bin"
    install -d "$bin"
    install -m755 target/release/noiseman "$bin/noiseman"
    # Never replace or shadow someone else's `nsm`: warn and leave it be.
    other=$(command -v nsm || true)
    if [[ (-e $bin/nsm || -L $bin/nsm) && $(readlink "$bin/nsm") != noiseman ]]; then
        echo "warning: $bin/nsm already exists and is not noiseman's; not linking nsm" >&2
    elif [[ -n $other && $other != "$bin/nsm" && $(basename "$(readlink -f "$other" 2>/dev/null || echo "$other")") != noiseman ]]; then
        echo "warning: nsm on PATH is $other, not noiseman; not linking nsm" >&2
    elif [[ ! -L $bin/nsm ]]; then
        ln -s noiseman "$bin/nsm"
    fi
    if [[ :$PATH: != *":$bin:"* ]]; then
        echo "note: $bin is not on PATH; add it to run noiseman by name" >&2
    fi
    [[ $(uname -s) == Linux ]] || exit 0
    share="$HOME/.local/share"
    install -Dm644 assets/noiseman.svg "$share/icons/hicolor/scalable/apps/noiseman.svg"
    for s in 48 64 128 256; do
        install -Dm644 assets/noiseman-$s.png "$share/icons/hicolor/${s}x${s}/apps/noiseman.png"
    done
    install -d "$share/applications"
    sed "s|@BIN@|$bin/noiseman|g" assets/noiseman.desktop > "$share/applications/noiseman.desktop"
    update-desktop-database "$share/applications" 2>/dev/null || true

# build a release archive (with gui, plus license texts) into target/dist; target defaults to this machine
package target=`rustc -vV | sed -n 's/^host: //p'`:
    tools/package.sh {{ target }}

# build the web demo (static files) into target/web
web:
    cargo build -q --release -p noiseman-web --target wasm32-unknown-unknown
    mkdir -p target/web
    cp web/site/* target/web/
    cp target/wasm32-unknown-unknown/release/noiseman_web.wasm target/web/noiseman.wasm
    cp assets/noiseman.svg target/web/favicon.svg
    mkdir -p target/web/architecture
    cp docs/architecture.html target/web/architecture/index.html
    mkdir -p target/web/preview
    cp assets/noiseman-web.png target/web/preview/noiseman.png

# serve the web demo on http://localhost:8000
web-serve: web
    python3 -m http.server -d target/web 8000

# remove build artifacts
clean:
    cargo clean

# profile the noise DSP (white|pink|brown|blue|violet|all) over secs of 48 kHz audio into profiles/dsp-<kind>.*
profile kind="all" secs="5000":
    mkdir -p profiles
    cargo build --profile profiling -p noiseman-core --example noise_profile
    perf record -e cycles:u -F 2999 --call-graph dwarf -o profiles/dsp-{{kind}}.perf.data -- target/profiling/examples/noise_profile {{kind}} {{secs}}
    samply import --save-only -o profiles/dsp-{{kind}}.json.gz profiles/dsp-{{kind}}.perf.data
    perf script -i profiles/dsp-{{kind}}.perf.data | inferno-collapse-perf | inferno-flamegraph > profiles/dsp-{{kind}}.svg
