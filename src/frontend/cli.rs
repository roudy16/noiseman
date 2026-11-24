use std::process::ExitCode;

use super::client::Client;
use crate::protocol::{Command, Kind, State};

pub const USAGE: &str = "\
usage: noiseman [COMMAND]

  (none) | tui         interactive terminal controls (starts the player if needed)
  play [KIND]          start or unmute, optionally switching noise
  stop                 stop the player
  mute | unmute | toggle
  kind KIND            white | pink | brown | blue | violet
  vol N | +N | -N      volume 0-100, absolute or relative
  status               show the current state
  gui                  window and tray controls (needs a build with --features gui)
  daemon [--muted]     run the player in the foreground (--muted: start silent)";

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
