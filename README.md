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

Both install for your user only, with `nsm` as a short alias. Linux needs PulseAudio or PipeWire and glibc 2.35 or
newer (Ubuntu 22.04, Debian 12, Fedora 36). Archives for a manual download are on
[Releases](https://github.com/roudy16/noiseman/releases); to build from source, see [CONTRIBUTING.md](CONTRIBUTING.md).

## Use

```sh
noiseman              # terminal UI
noiseman gui          # window (with a tray icon on Linux)
noiseman play brown   # play in the background: brown, white, blue, pink or violet
noiseman vol +10      # louder (or vol 30, vol -10)
noiseman toggle       # mute or unmute
noiseman stop         # stop the player
noiseman help         # every command
```

In the terminal UI, `1`-`5` pick a noise, `↑`/`↓` or `k`/`j` cycle it, `←`/`→` or `h`/`l` change the volume, `m` or
space mutes, and `s` stops. `q` or `Esc` closes the UI and leaves the noise playing. The window and the browser demo
take the same noise, volume and mute keys.

Muted, the player releases the audio device and uses no CPU.
[How it works](https://roudy16.github.io/noiseman/architecture/) explains the design.

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
