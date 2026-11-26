# noiseman

White, pink, brown, blue or violet noise for focus or sleep. A tiny background player that you control from the
terminal, from one-line commands in your scripts and keybindings, or from a small window.

**[▶ Try it in your browser](https://roudy16.github.io/noiseman/)**, with nothing to install.

![The noiseman terminal UI playing brown noise at 50% volume](assets/noiseman-tui.png)

## Install

Linux and macOS:

```sh
curl --proto '=https' --tlsv1.2 -fsSL https://github.com/roudy16/noiseman/releases/latest/download/install.sh | sh
```

Windows 10 (1803 or later) and 11, in PowerShell:

```powershell
irm https://github.com/roudy16/noiseman/releases/latest/download/install.ps1 | iex
```

Both install the terminal build (commands and terminal UI) for your user only, with `nsm` as a short alias. For the
window too, install the gui build: pipe into `NOISEMAN_GUI=1 sh` instead of `sh`, or on Windows run
`$env:NOISEMAN_GUI = '1'` first. Linux needs PulseAudio or PipeWire and glibc 2.34 or newer (Ubuntu 22.04, Debian 12,
Fedora 35, RHEL 9); macOS needs version 11 or newer. Archives for a manual download (`noiseman-<target>`, or
`noiseman-gui-<target>` with the window) are on [Releases](https://github.com/roudy16/noiseman/releases); to build
from source, see [CONTRIBUTING.md](CONTRIBUTING.md). To check that a file came from this repository's release
workflow: `gh attestation verify <file> --repo roudy16/noiseman`.

The binaries are not signed. The installers are not affected, but a downloaded archive may be blocked: on macOS, run
`xattr -d com.apple.quarantine noiseman`, or use System Settings > Privacy & Security > Open Anyway; on Windows, pick
More info > Run anyway at the SmartScreen prompt, or run `Unblock-File` on the .zip before extracting it.

## Use

```sh
nsm              # terminal UI
nsm gui          # window, in the gui build (with a tray icon, in the menu bar on macOS)
nsm play brown   # play in the background: brown, white, blue, pink or violet
nsm vol +10      # louder (or vol 30, vol -10)
nsm toggle       # mute or unmute
nsm stop         # stop the player
nsm help         # every command
```

`nsm` is the installers' short alias for `noiseman`. They skip it if another `nsm` is already installed; then use
`noiseman`, or add `alias nsm=noiseman` to your shell's startup file (in PowerShell, `Set-Alias nsm noiseman` to your
profile).

In the terminal UI, `1`-`5` pick a noise, `↑`/`↓` or `k`/`j` cycle it, `←`/`→` or `h`/`l` change the volume, `m` or
space mutes, and `s` stops. `q` or `Esc` closes the UI and leaves the noise playing. The window takes the same keys
(there `q` hides it to the tray; on macOS so does Cmd-W, and Cmd-Q stops); the browser demo takes the noise, volume and
mute ones.

The player starts with the noise and volume it last had, saved in `~/.local/state/noiseman/state` (or under
`$XDG_STATE_HOME`; on macOS `~/Library/Application Support/noiseman`, on Windows `%LOCALAPPDATA%\noiseman`).

Muted, the player releases the audio device and uses no CPU.
[How it works](https://roudy16.github.io/noiseman/architecture/) explains the design.

## Troubleshooting

- **No sound:** on Linux, noiseman plays through PulseAudio or PipeWire's pulse server; `pactl info` should show one.
- **The noise keeps playing after the UI closes:** that is by design; `noiseman stop` (or `s` in the UI) stops it.
- **Log:** the player writes `noiseman.log` in `$XDG_RUNTIME_DIR` when it is private (0700), else in `noiseman-<uid>/`
  under `$TMPDIR` (or `/tmp`), else (when another user made that one first) in `noiseman/` under `$XDG_CACHE_HOME` (or
  `~/.cache`), or on Windows in `%LOCALAPPDATA%\noiseman`; each run starts with its version and any directory it
  passed over, and every line has a UTC time.
- **Bug reports:** include the output of `noiseman --version`.

## Uninstall

On Windows, use Settings > Apps > noiseman > Uninstall. On Linux and macOS:

```sh
noiseman stop
[ "$(readlink ~/.local/bin/nsm)" = noiseman ] && rm ~/.local/bin/nsm
rm ~/.local/bin/noiseman
rm -r ~/.local/state/noiseman   # the saved noise and volume (macOS: ~/Library/Application\ Support/noiseman)
# Linux only: the desktop entry and icons
rm ~/.local/share/applications/noiseman.desktop ~/.local/share/icons/hicolor/*/apps/noiseman.*
```

## Status

noiseman aims to stay small: one job, few features, little memory and CPU. One maintainer; only the latest release is
supported. In scope: colored noise, played in the background and controlled from the terminal, scripts or a window.
Out of scope: other sounds (rain, music), timers, presets, EQ, device settings.

## License

MIT, except the vendored crates in `vendor/`, which keep their own licenses (see [CONTRIBUTING.md](CONTRIBUTING.md)).
