#!/usr/bin/env bash
# Build the release archive for one target: target/dist/noiseman-<target>.tar.gz and its .sha256.
# The binary has the `gui` feature, since users of a prebuilt binary cannot turn it on later.
set -euo pipefail
target=${1:?usage: tools/package.sh TARGET}
cd "$(dirname "$0")/.."

cargo build --release --locked --features gui --target "$target"
name=noiseman-$target
dist=target/dist
rm -rf "${dist:?}/$name" "$dist/$name.tar.gz" "$dist/$name.tar.gz.sha256"
mkdir -p "$dist/$name"
cp "target/$target/release/noiseman" LICENSE README.md "$dist/$name/"
python3 tools/licenses.py "$target" > "$dist/$name/THIRD-PARTY-LICENSES.txt"
if [[ $target == *linux* ]]; then
    # Icons and desktop entry for install.sh: the window and tray look the icon up by name.
    install -Dm644 assets/noiseman.svg "$dist/$name/share/icons/scalable/apps/noiseman.svg"
    for s in 48 64 128 256; do
        install -Dm644 "assets/noiseman-$s.png" "$dist/$name/share/icons/${s}x${s}/apps/noiseman.png"
    done
    install -m644 assets/noiseman.desktop "$dist/$name/share/noiseman.desktop"
fi
# COPYFILE_DISABLE keeps macOS tar from adding ._ metadata files.
COPYFILE_DISABLE=1 tar -C "$dist" -czf "$dist/$name.tar.gz" "$name"
sum=$(command -v sha256sum || echo "shasum -a 256")
(cd "$dist" && $sum "$name.tar.gz" > "$name.tar.gz.sha256")
rm -rf "${dist:?}/$name"
echo "$dist/$name.tar.gz"
