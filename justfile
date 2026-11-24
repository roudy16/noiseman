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

# install binary to ~/.local/bin, icon and desktop entry for the current user
install:
    cargo build --release
    install -Dm755 target/release/noiseman ~/.local/bin/noiseman
    install -Dm644 assets/noiseman.svg ~/.local/share/icons/noiseman/noiseman.svg
    sed "s|/home/roudy|$HOME|g" assets/noiseman.desktop > ~/.local/share/applications/noiseman.desktop
    -update-desktop-database ~/.local/share/applications

# remove build artifacts
clean:
    cargo clean
