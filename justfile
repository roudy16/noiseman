# noiseman — all dev workflows go through just

set positional-arguments

# list recipes
default:
    @just --list

# fetch dependencies
setup:
    cargo fetch

# compile
build:
    cargo build

# run tests
test:
    cargo test

# run the binary, passing args through
run *args:
    cargo run --quiet -- "$@"

# format the code
fmt:
    cargo fmt

# clippy with warnings as errors, default build and with the gui feature
lint:
    cargo clippy --all-targets -- -D warnings
    cargo clippy --all-targets --features gui -- -D warnings

# format check, lint and tests (run before committing)
check:
    cargo fmt --check
    just lint
    just test

# install binary (plus an `nsm` alias) to ~/.local/bin, hicolor icons and desktop entry for the current user
install:
    cargo build --release
    install -Dm755 target/release/noiseman ~/.local/bin/noiseman
    ln -sfn noiseman ~/.local/bin/nsm
    install -Dm644 assets/noiseman.svg ~/.local/share/icons/hicolor/scalable/apps/noiseman.svg
    for s in 48 64 128 256; do install -Dm644 assets/noiseman-$s.png ~/.local/share/icons/hicolor/${s}x${s}/apps/noiseman.png; done
    sed "s|/home/roudy|$HOME|g" assets/noiseman.desktop > ~/.local/share/applications/noiseman.desktop
    -update-desktop-database ~/.local/share/applications

# remove build artifacts
clean:
    cargo clean

# profile the noise DSP (white|pink|brown|blue|violet|all) over secs of 48 kHz audio into profiles/dsp-<kind>.*
profile kind="all" secs="5000":
    mkdir -p profiles
    cargo build --profile profiling --example noise_profile
    perf record -e cycles:u -F 2999 --call-graph dwarf -o profiles/dsp-{{kind}}.perf.data -- target/profiling/examples/noise_profile {{kind}} {{secs}}
    samply import --save-only -o profiles/dsp-{{kind}}.json.gz profiles/dsp-{{kind}}.perf.data
    perf script -i profiles/dsp-{{kind}}.perf.data | inferno-collapse-perf | inferno-flamegraph > profiles/dsp-{{kind}}.svg
