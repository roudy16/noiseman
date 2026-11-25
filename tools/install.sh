#!/bin/sh
# Install the latest noiseman release for this machine, with an `nsm` link:
#   curl -fsSL https://github.com/roudy16/noiseman/releases/latest/download/install.sh | sh
# NOISEMAN_INSTALL_DIR overrides the directory (default ~/.local/bin); NOISEMAN_VERSION (e.g. v0.1.0) picks a release.
# On Linux it also installs icons and a desktop entry to ~/.local/share, as `just install` does.
# Everything is inside main, called on the last line, so a download cut short runs nothing.
set -eu

repo=https://github.com/roudy16/noiseman

die() {
    echo "noiseman: $*" >&2
    exit 1
}

fetch() {
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL -o "$2" "$1"
    elif command -v wget >/dev/null 2>&1; then
        wget -q -O "$2" "$1"
    else
        die "needs curl or wget"
    fi || die "cannot download $1"
}

verify() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum -c "$1" >/dev/null
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 -c "$1" >/dev/null
    else
        die "needs sha256sum or shasum to verify the download"
    fi || die "checksum mismatch, not installing"
}

link_nsm() {
    dir=$1
    # Never replace or shadow someone else's `nsm`: warn and leave it be.
    other=$(command -v nsm || true)
    if { [ -e "$dir/nsm" ] || [ -L "$dir/nsm" ]; } && [ "$(readlink "$dir/nsm" || true)" != noiseman ]; then
        echo "warning: $dir/nsm already exists and is not noiseman's; not linking nsm" >&2
    elif [ -n "$other" ] && [ "$other" != "$dir/nsm" ] &&
        [ "$(basename "$(readlink -f "$other" 2>/dev/null || echo "$other")")" != noiseman ]; then
        echo "warning: nsm on PATH is $other, not noiseman; not linking nsm" >&2
    elif [ ! -L "$dir/nsm" ]; then
        ln -s noiseman "$dir/nsm"
        echo "Linked $dir/nsm"
    fi
}

desktop_files() {
    src=$1 bin=$2 share=${XDG_DATA_HOME:-$HOME/.local/share}
    for icon in "$src"/icons/*/apps/noiseman.*; do
        size=$(basename "$(dirname "$(dirname "$icon")")")
        mkdir -p "$share/icons/hicolor/$size/apps"
        cp "$icon" "$share/icons/hicolor/$size/apps/"
    done
    mkdir -p "$share/applications"
    sed "s|@BIN@|$bin|g" "$src/noiseman.desktop" >"$share/applications/noiseman.desktop"
    update-desktop-database "$share/applications" 2>/dev/null || true
    echo "Installed icons and a desktop entry to $share"
}

main() {
    dir=${NOISEMAN_INSTALL_DIR:-$HOME/.local/bin}
    os=$(uname -s)
    case "$os-$(uname -m)" in
        Linux-x86_64) target=x86_64-unknown-linux-gnu ;;
        Linux-aarch64 | Linux-arm64) target=aarch64-unknown-linux-gnu ;;
        Darwin-arm64) target=aarch64-apple-darwin ;;
        Darwin-x86_64) target=x86_64-apple-darwin ;;
        *) die "no prebuilt binary for $(uname -s) $(uname -m); build from source ($repo)" ;;
    esac
    if [ -n "${NOISEMAN_VERSION:-}" ]; then
        base=$repo/releases/download/$NOISEMAN_VERSION
    else
        base=$repo/releases/latest/download
    fi
    name=noiseman-$target

    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT INT TERM
    echo "Downloading $base/$name.tar.gz"
    fetch "$base/$name.tar.gz" "$tmp/$name.tar.gz"
    fetch "$base/$name.tar.gz.sha256" "$tmp/$name.tar.gz.sha256"
    (cd "$tmp" && verify "$name.tar.gz.sha256")
    tar -xzf "$tmp/$name.tar.gz" -C "$tmp"
    # uname reports the kernel: a musl, 32-bit or too-old userland only shows when the binary runs.
    "$tmp/$name/noiseman" help >/dev/null 2>&1 ||
        die "the $target binary does not run here (it needs glibc 2.35+ on Linux); build from source ($repo)"

    mkdir -p "$dir"
    # Write beside the target, then rename: replacing a running binary in place would corrupt it.
    cp "$tmp/$name/noiseman" "$dir/.noiseman.new"
    chmod 755 "$dir/.noiseman.new"
    mv -f "$dir/.noiseman.new" "$dir/noiseman"
    echo "Installed $dir/noiseman"
    link_nsm "$dir"
    if [ "$os" = Linux ]; then
        desktop_files "$tmp/$name/share" "$dir/noiseman"
    fi
    case ":$PATH:" in
        *":$dir:"*) ;;
        *) echo "note: $dir is not on PATH; add it to run noiseman by name" >&2 ;;
    esac
}

main "$@"
