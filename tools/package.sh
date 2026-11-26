#!/usr/bin/env bash
# Build one release archive for a target into target/dist with its .sha256: noiseman-<target>.tar.gz (.zip for
# Windows), the terminal build (CLI + TUI, the default features), or with `gui` noiseman-gui-<target>, built with the
# window. Either way the binary is named noiseman: the two are alternatives installed at the same path.
set -euo pipefail
usage="usage: tools/package.sh TARGET [gui]"
target=${1:?$usage}
variant=${2:-}
[[ -z $variant || $variant == gui ]] || { echo "$usage" >&2; exit 2; }
cd "$(dirname "$0")/.."

name=noiseman${variant:+-$variant}-$target
dist=target/dist
out=target/$target/release
windows=false
[[ $target == *windows* ]] && windows=true
# One stated minimum for both macOS archives (rustc's x86_64 default is 10.12, aarch64's 11.0).
[[ $target == *apple* ]] && export MACOSX_DEPLOYMENT_TARGET=11.0
rm -rf "${dist:?}/$name" "$dist/$name".{tar.gz,zip}{,.sha256}
mkdir -p "$dist/$name"

if $windows && [[ $variant == gui ]]; then
    # noiseman-gui.exe: the same program linked for the Windows subsystem, so the Start Menu shortcut opens the window
    # without a console beside it (run under that name with no arguments, it opens the window).
    cargo rustc --release --locked --features gui --target "$target" --bin noiseman -- \
        -C link-arg=/SUBSYSTEM:WINDOWS -C link-arg=/ENTRY:mainCRTStartup
    cp "$out/noiseman.exe" "$dist/$name/noiseman-gui.exe"
fi
cargo build --release --locked ${variant:+--features "$variant"} --target "$target"
cp "$out/noiseman$($windows && echo .exe)" LICENSE README.md "$dist/$name/"
# The first interpreter that runs: on Windows, `python3` may be the Microsoft Store stub, which only prints a hint.
for python in python3 python py ""; do "$python" -c "" 2>/dev/null && break; done
"${python:?no working Python found}" tools/licenses.py "$target" ${variant:+"$variant"} \
    >"$dist/$name/THIRD-PARTY-LICENSES.txt"
if [[ $target == *linux* ]]; then
    # Icons and desktop entry for install.sh: the window and tray look the icon up by name.
    install -Dm644 assets/noiseman.svg "$dist/$name/share/icons/scalable/apps/noiseman.svg"
    for png in assets/noiseman-[0-9]*.png; do
        s=${png//[!0-9]/}
        install -Dm644 "$png" "$dist/$name/share/icons/${s}x${s}/apps/noiseman.png"
    done
    # The template opens the window; the terminal build's entry runs the TUI in a terminal instead.
    if [[ $variant == gui ]]; then
        install -m644 assets/noiseman.desktop "$dist/$name/share/noiseman.desktop"
    else
        sed -e 's/^Exec=@BIN@ gui$/Exec=@BIN@/' -e 's/^Terminal=false$/Terminal=true/' assets/noiseman.desktop \
            >"$dist/$name/share/noiseman.desktop"
    fi
fi

if $windows; then
    archive=$name.zip
    # Windows opens zip archives natively (Expand-Archive, Explorer); the icon is embedded in the binaries. Windows'
    # own bsdtar (Git Bash's tar is GNU tar), since Windows PowerShell's Compress-Archive writes \ into entry names.
    (cd "$dist" && "$SYSTEMROOT/System32/tar.exe" -a -cf "$archive" "$name")
else
    archive=$name.tar.gz
    # COPYFILE_DISABLE keeps macOS tar from adding ._ metadata files.
    COPYFILE_DISABLE=1 tar -C "$dist" -czf "$dist/$archive" "$name"
fi
sum=$(command -v sha256sum || echo "shasum -a 256")
(cd "$dist" && $sum "$archive" > "$archive.sha256")
rm -rf "${dist:?}/$name"
echo "$dist/$archive"
