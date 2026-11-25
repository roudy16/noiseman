# noiseman

White, pink, brown, blue or violet noise for focus or sleep. A tiny background player that you control from the
terminal, from one-line commands in your scripts and keybindings, or from a small window.

**[▶ Try it in your browser](https://roudy16.github.io/noiseman/)**, with nothing to install.

![The noiseman terminal UI playing brown noise at 50% volume](assets/noiseman-tui.png)

## Install

Linux and macOS:

```sh
curl -fsSL https://github.com/roudy16/noiseman/releases/latest/download/install.sh | sh
```

Windows 10 (1803 or later) and 11, in PowerShell:

```powershell
irm https://github.com/roudy16/noiseman/releases/latest/download/install.ps1 | iex
```

Both install the terminal build (commands and terminal UI) for your user only, with `nsm` as a short alias. For the
window too, install the gui build: pipe into `NOISEMAN_GUI=1 sh` instead of `sh`, or on Windows run
`$env:NOISEMAN_GUI = '1'` first. Linux needs PulseAudio or PipeWire and glibc 2.35 or newer (Ubuntu 22.04, Debian 12,
Fedora 36); macOS needs version 11 or newer. Archives for a manual download (`noiseman-<target>`, or
`noiseman-gui-<target>` with the window) are on [Releases](https://github.com/roudy16/noiseman/releases); to build
from source, see [CONTRIBUTING.md](CONTRIBUTING.md).

The binaries are not signed. The installers are not affected, but a downloaded archive may be blocked: on macOS, run
`xattr -d com.apple.quarantine noiseman`, or use System Settings > Privacy & Security > Open Anyway; on Windows, pick
More info > Run anyway at the SmartScreen prompt, or run `Unblock-File` on the .zip before extracting it.

## Use

```sh
noiseman              # terminal UI
noiseman gui          # window, in the gui build (with a tray icon on Linux)
noiseman play brown   # play in the background: brown, white, blue, pink or violet
noiseman vol +10      # louder (or vol 30, vol -10)
noiseman toggle       # mute or unmute
noiseman stop         # stop the player
noiseman help         # every command
```

In the terminal UI, `1`-`5` pick a noise, `↑`/`↓` or `k`/`j` cycle it, `←`/`→` or `h`/`l` change the volume, `m` or
space mutes, and `s` stops. `q` or `Esc` closes the UI and leaves the noise playing. The window takes the same keys
(there `q` hides it to the tray); the browser demo takes the noise, volume and mute ones.

Muted, the player releases the audio device and uses no CPU.
[How it works](https://roudy16.github.io/noiseman/architecture/) explains the design.

## Troubleshooting

- **No sound:** on Linux, noiseman plays through PulseAudio or PipeWire's pulse server; `pactl info` should show one.
- **The noise keeps playing after the UI closes:** that is by design; `noiseman stop` (or `s` in the UI) stops it.
- **Log:** the player writes `noiseman.log` in `$XDG_RUNTIME_DIR`, else in `noiseman-<uid>/` under `$TMPDIR` (or
  `/tmp`), or on Windows in `%LOCALAPPDATA%\noiseman`; each run starts with its version, and every line has a UTC time.
- **Bug reports:** include the output of `noiseman --version`.

## Uninstall

On Windows, use Settings > Apps > noiseman > Uninstall. On Linux and macOS:

```sh
noiseman stop
[ "$(readlink ~/.local/bin/nsm)" = noiseman ] && rm ~/.local/bin/nsm
rm ~/.local/bin/noiseman
# Linux only: the desktop entry and icons
rm ~/.local/share/applications/noiseman.desktop ~/.local/share/icons/hicolor/*/apps/noiseman.*
```

## License

MIT, except the vendored crates in `vendor/`, which keep their own licenses (see [CONTRIBUTING.md](CONTRIBUTING.md)).
