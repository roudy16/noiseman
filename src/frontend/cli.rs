use std::process::ExitCode;

use super::client::Client;
use crate::protocol::{Command, Kind, State};

pub const USAGE: &str = "\
noiseman - background noise generator (brown, white, blue, pink, violet)

usage: noiseman [COMMAND]

Commands (non-interactive; print state and exit):
  play [KIND]          start or unmute, optionally switching noise kind
  stop                 stop the background daemon (no-op if not running)
  mute                 mute playback (does not start the daemon)
  unmute               unmute playback (starts the daemon if needed)
  toggle               toggle mute (starts playing if not running)
  kind KIND            switch noise: brown | white | blue | pink | violet
  vol N | +N | -N      set volume (0-100) or adjust relatively (e.g. vol +5, vol -5)
  status               show current state (does not start the daemon)

Interactive & daemon:
  (none) | tui         interactive terminal UI (requires a TTY)
  gui                  desktop window (gui build; tray on Linux)
  daemon [--muted]     run the player in the foreground (--muted: start silent)
  help | -h | --help   show this help
  version | -V         show the version (also --version)

Notes:
  - Commands that play or change settings auto-start the background daemon
    (default: brown, 50%, playing). `status`, `mute` and `stop` never start it.
  - Output format: `<Kind> <vol>% <playing|muted>` (or `stopped` / `not running`).
  - Exit status: 0 on success, 1 on error (errors go to stderr, e.g. `status` when stopped).
  - Runtime files: $XDG_RUNTIME_DIR/noiseman.{sock,lock,log} (fallback: $TMPDIR/noiseman-<uid>/,
    or %LOCALAPPDATA%\\noiseman on Windows).";

pub fn run(args: &[String]) -> ExitCode {
    match exec(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("noiseman: {e}");
            ExitCode::FAILURE
        }
    }
}

fn exec(args: &[String]) -> Result<(), String> {
    let (cmds, autostart) = match args {
        [play] if play == "play" => (vec![Command::Mute(false)], true),
        [play, kind] if play == "play" => {
            let k = Kind::parse(kind).ok_or_else(|| format!("unknown noise kind {kind:?}"))?;
            (vec![Command::Kind(k), Command::Mute(false)], true)
        }
        [stop] if stop == "stop" => (vec![Command::Quit], false),
        // A fresh player starts unmuted, so toggling it would start it silent.
        [toggle] if toggle == "toggle" => match Client::connect(false) {
            Err(e) if e == "not running" => (vec![Command::Mute(false)], true),
            Err(e) => return Err(e),
            Ok(mut client) => return report(&[Command::ToggleMute], &mut client),
        },
        _ => {
            let cmd = Command::parse(&args.join(" ")).map_err(|e| format!("{e}\n{USAGE}"))?;
            if cmd == Command::Watch {
                return Err(format!("unknown command \"watch\"\n{USAGE}"));
            }
            (vec![cmd], !matches!(cmd, Command::Status | Command::Mute(true) | Command::Quit))
        }
    };
    let mut client = match Client::connect(autostart) {
        Err(e) if e == "not running" && cmds == [Command::Quit] => {
            println!("not running");
            return Ok(());
        }
        other => other?,
    };
    report(&cmds, &mut client)
}

fn report(cmds: &[Command], client: &mut Client) -> Result<(), String> {
    if let Some(w) = client.warning.as_ref().filter(|_| cmds != [Command::Quit]) {
        eprintln!("noiseman: warning: {w}");
    }
    let mut state = None;
    for &cmd in cmds {
        state = Some(client.request(cmd)?);
    }
    let Some(s) = state else { return Ok(()) };
    if cmds == [Command::Quit] {
        println!("stopped");
        return Ok(());
    }
    let wanted_sound = cmds.iter().any(|c| matches!(c, Command::Mute(false) | Command::ToggleMute));
    if wanted_sound && s.muted && s.error.is_some() {
        return Err(describe(&s));
    }
    println!("{}", describe(&s));
    Ok(())
}

pub fn describe(s: &State) -> String {
    let play = if s.muted { "muted" } else { "playing" };
    match &s.error {
        Some(e) => format!("{} {}% {play} ({e})", s.kind.label(), s.vol),
        None => format!("{} {}% {play}", s.kind.label(), s.vol),
    }
}
