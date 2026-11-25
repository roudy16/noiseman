#!/bin/sh
# Install the latest noiseman release for this machine, with an `nsm` link:
#   curl -fsSL https://github.com/roudy16/noiseman/releases/latest/download/install.sh | sh
# It installs the terminal build (CLI + TUI); NOISEMAN_GUI=1 installs the build with the desktop window instead:
#   curl -fsSL https://github.com/roudy16/noiseman/releases/latest/download/install.sh | NOISEMAN_GUI=1 sh
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

# Refuse, before downloading, the systems the prebuilt binaries cannot run on, saying why. What only shows when the
# binary runs (glibc too old, a missing library) is explained by explain_failure.
check_system() {
    os=$1 target=$2
    if [ "$os" = Darwin ]; then
        major=$(sw_vers -productVersion 2>/dev/null | cut -d. -f1)
        if [ -n "$major" ] && [ "$major" -lt 11 ]; then
            die "needs macOS 11 or newer; this Mac has $(sw_vers -productVersion)"
        fi
        return
    fi
    if [ -e /etc/NIXOS ] && [ -z "${NIX_LD:-}" ]; then
        die "NixOS runs prebuilt Linux binaries only with nix-ld (programs.nix-ld.enable = true); or build from source ($repo)"
    fi
    if ldd --version 2>&1 | grep -qi musl; then
        die "this system uses musl libc (as Alpine does); the prebuilt binaries need glibc, so build from source ($repo)"
    fi
    case $target in
        x86_64-*) loader=/lib64/ld-linux-x86-64.so.2 ;;
        *) loader=/lib/ld-linux-aarch64.so.1 ;;
    esac
    [ -e "$loader" ] ||
        die "no 64-bit glibc loader ($loader): the prebuilt binaries need 64-bit glibc, and this looks like a 32-bit or non-glibc system on a 64-bit kernel; build from source ($repo)"
    for tool in tar gzip; do
        command -v "$tool" >/dev/null 2>&1 || die "needs $tool to unpack the download"
    done
}

# Why the downloaded binary would not run, from the loader's message.
explain_failure() {
    err=$1
    case $err in
        *libasound.so.2*)
            die "needs the ALSA library (libasound.so.2), which is missing: install libasound2 (Debian, Ubuntu, openSUSE) or alsa-lib (Fedora, RHEL, Arch), then run this again"
            ;;
        *GLIBC_*)
            need=$(echo "$err" | grep -o 'GLIBC_[0-9.]*' | sed 's/GLIBC_//' | sort -t. -k1,1n -k2,2n | tail -n 1)
            have=$(getconf GNU_LIBC_VERSION 2>/dev/null | cut -d' ' -f2)
            die "needs glibc $need or newer; this system has ${have:-an older one} (Ubuntu 22.04, Debian 12, Fedora 35, RHEL 9 and later have it); build from source ($repo)"
            ;;
        *)
            die "the $target binary does not run here: ${err:-no error message}; build from source ($repo)"
            ;;
    esac
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

path_hint() {
    dir=$1 rc=
    case "$(basename "${SHELL:-}")" in
        zsh) rc=.zshrc ;;
        bash) [ "$2" = Darwin ] && rc=.bash_profile || rc=.bashrc ;;
        fish)
            echo "note: $dir is not on PATH; to add it, run: fish_add_path $dir" >&2
            return
            ;;
    esac
    if [ -n "$rc" ]; then
        echo "note: $dir is not on PATH; to add it, run:" >&2
        echo "  echo 'export PATH=\"$dir:\$PATH\"' >> ~/$rc" >&2
    else
        echo "note: $dir is not on PATH; add it to run noiseman by name" >&2
    fi
}

main() {
    dir=${NOISEMAN_INSTALL_DIR:-$HOME/.local/bin}
    os=$(uname -s)
    case "$os-$(uname -m)" in
        Linux-x86_64) target=x86_64-unknown-linux-gnu ;;
        Linux-aarch64 | Linux-arm64) target=aarch64-unknown-linux-gnu ;;
        Darwin-arm64) target=aarch64-apple-darwin ;;
        Darwin-x86_64) target=x86_64-apple-darwin ;;
        Linux-* | Darwin-*) die "prebuilt binaries are for 64-bit x86 and ARM only; this is $(uname -m), so build from source ($repo)" ;;
        *) die "no prebuilt binary for $(uname -s); build from source ($repo)" ;;
    esac
    check_system "$os" "$target"
    if [ -n "${NOISEMAN_VERSION:-}" ]; then
        base=$repo/releases/download/$NOISEMAN_VERSION
    else
        base=$repo/releases/latest/download
    fi
    if [ "${NOISEMAN_GUI:-}" = 1 ]; then
        name=noiseman-gui-$target
        variant="gui build (CLI, TUI and desktop window)"
    else
        name=noiseman-$target
        variant="terminal build (CLI and TUI; for the desktop window, install with NOISEMAN_GUI=1)"
    fi

    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' EXIT INT TERM
    echo "Downloading $base/$name.tar.gz"
    fetch "$base/$name.tar.gz" "$tmp/$name.tar.gz"
    fetch "$base/$name.tar.gz.sha256" "$tmp/$name.tar.gz.sha256"
    (cd "$tmp" && verify "$name.tar.gz.sha256")
    tar -xzf "$tmp/$name.tar.gz" -C "$tmp"
    err=$("$tmp/$name/noiseman" help 2>&1 >/dev/null) || explain_failure "$err"
    # A running player keeps running the replaced binary; stop it (`status` never starts one) so this version takes over.
    stopped=
    if [ -x "$dir/noiseman" ] && "$dir/noiseman" status >/dev/null 2>&1; then
        if "$dir/noiseman" stop >/dev/null; then
            stopped=1
        else
            echo "warning: cannot stop the running player; run \`noiseman stop\` to restart it with this version" >&2
        fi
    fi

    mkdir -p "$dir"
    # Write beside the target, then rename: replacing a running binary in place would corrupt it.
    cp "$tmp/$name/noiseman" "$dir/.noiseman.new"
    chmod 755 "$dir/.noiseman.new"
    mv -f "$dir/.noiseman.new" "$dir/noiseman"
    echo "Installed $dir/noiseman, the $variant"
    if [ -n "$stopped" ]; then
        echo "Stopped the running player so this version takes over; \`noiseman\` starts it again"
    fi
    link_nsm "$dir"
    if [ "$os" = Linux ]; then
        desktop_files "$tmp/$name/share" "$dir/noiseman"
    fi
    case ":$PATH:" in
        *":$dir:"*) ;;
        *) path_hint "$dir" "$os" ;;
    esac
}

main "$@"
