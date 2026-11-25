//! The contract between the backend (the player daemon) and every frontend.
//!
//! One request per line on a Unix socket; every reply line is either `state <kind> <vol> <playing|muted>[ <error>]`
//! or `err <message>`. After `watch`, the daemon also pushes a `state` line on every change.

use std::fmt;
use std::path::PathBuf;

use crate::platform;

pub use noiseman_core::Kind;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Command {
    Kind(Kind),
    Vol(i32),
    VolBy(i32),
    Mute(bool),
    ToggleMute,
    Status,
    Watch,
    Quit,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct State {
    pub kind: Kind,
    pub vol: i32,
    pub muted: bool,
    pub error: Option<String>,
}

impl Command {
    pub fn parse(line: &str) -> Result<Command, String> {
        let mut words = line.split_whitespace();
        let cmd = words.next().unwrap_or("");
        let arg = words.next();
        let need = |what: &str| arg.ok_or_else(|| format!("{cmd} needs {what}"));
        let parsed = match cmd {
            "kind" => {
                let name = need("a noise kind")?;
                Command::Kind(Kind::parse(name).ok_or_else(|| format!("unknown noise kind {name:?}"))?)
            }
            "vol" => {
                let v = need("a volume")?;
                let n: i32 = v.parse().map_err(|_| format!("bad volume {v:?}"))?;
                if v.starts_with(['+', '-']) { Command::VolBy(n) } else { Command::Vol(n) }
            }
            "mute" => Command::Mute(true),
            "unmute" => Command::Mute(false),
            "toggle" => Command::ToggleMute,
            "status" => Command::Status,
            "watch" => Command::Watch,
            "quit" => Command::Quit,
            "" => return Err("empty request".into()),
            other => return Err(format!("unknown request {other:?}")),
        };
        let takes_arg = matches!(parsed, Command::Kind(_) | Command::Vol(_) | Command::VolBy(_));
        match if takes_arg { words.next() } else { arg } {
            Some(extra) => Err(format!("unexpected argument {extra:?}")),
            None => Ok(parsed),
        }
    }
}

impl fmt::Display for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Command::Kind(k) => write!(f, "kind {}", k.label().to_ascii_lowercase()),
            Command::Vol(v) => write!(f, "vol {v}"),
            Command::VolBy(v) => write!(f, "vol {v:+}"),
            Command::Mute(true) => f.write_str("mute"),
            Command::Mute(false) => f.write_str("unmute"),
            Command::ToggleMute => f.write_str("toggle"),
            Command::Status => f.write_str("status"),
            Command::Watch => f.write_str("watch"),
            Command::Quit => f.write_str("quit"),
        }
    }
}

impl State {
    pub fn parse(line: &str) -> Result<State, String> {
        if let Some(msg) = line.strip_prefix("err ") {
            return Err(msg.to_owned());
        }
        let bad = || format!("bad reply {line:?}");
        let mut parts = line.splitn(5, ' ');
        if parts.next() != Some("state") {
            return Err(bad());
        }
        let kind = parts.next().and_then(Kind::parse).ok_or_else(bad)?;
        let vol = parts.next().and_then(|v| v.parse().ok()).ok_or_else(bad)?;
        let muted = match parts.next() {
            Some("muted") => true,
            Some("playing") => false,
            _ => return Err(bad()),
        };
        Ok(State { kind, vol, muted, error: parts.next().map(str::to_owned) })
    }
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let play = if self.muted { "muted" } else { "playing" };
        write!(f, "state {} {} {play}", self.kind.label().to_ascii_lowercase(), self.vol)?;
        if let Some(e) = &self.error {
            write!(f, " {}", e.replace(['\n', '\r'], " "))?;
        }
        Ok(())
    }
}

/// The per-user private directory for the socket, the lock file and the daemon log.
/// Absolute, so the daemon (which leaves its starting directory) and its clients agree on a relative
/// `$XDG_RUNTIME_DIR` or `$TMPDIR`.
pub fn runtime_dir() -> Result<PathBuf, String> {
    let dir = platform::runtime_dir()?;
    std::path::absolute(&dir).map_err(|e| format!("cannot resolve {}: {e}", dir.display()))
}

pub fn socket_path() -> Result<PathBuf, String> {
    Ok(runtime_dir()?.join("noiseman.sock"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        for cmd in [
            Command::Kind(Kind::Pink),
            Command::Vol(40),
            Command::VolBy(-5),
            Command::VolBy(5),
            Command::Mute(true),
            Command::Mute(false),
            Command::ToggleMute,
            Command::Status,
            Command::Watch,
            Command::Quit,
        ] {
            assert_eq!(Command::parse(&cmd.to_string()), Ok(cmd));
        }
        for error in [None, Some("no default output device".to_owned())] {
            let s = State { kind: Kind::Violet, vol: 7, muted: true, error };
            assert_eq!(State::parse(&s.to_string()), Ok(s));
        }
        assert_eq!(State::parse("err nope"), Err("nope".into()));
        assert!(Command::parse("kind grey").is_err());
        assert!(Command::parse("mute now").is_err());
        assert!(Command::parse("vol 5 6").is_err());
        assert!(Command::parse("vol").is_err());
    }
}
