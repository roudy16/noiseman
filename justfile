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

# install the binary (cargo install)
install:
    cargo install --path .

# remove build artifacts
clean:
    cargo clean
