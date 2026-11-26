//! The contract between the backend (the player daemon) and every frontend.
//!
//! One request per line on a Unix socket; every reply line is either `state <kind> <vol> <playing|muted>[ <error>]`
//! or `err <message>`, except that `protocol` gets the one-off reply `protocol <PROTOCOL>`. After `watch`, the daemon also
//! pushes a `state` line on every change.

use std::fmt;
use std::path::PathBuf;

use crate::platform;

/// The version of this protocol, independent of the app's, under semver as Cargo reads it (see `compatible`): an
/// addition clients may rely on bumps the minor version, any other change the major (while the major is 0: the patch
/// and the minor). noiseman 0.1.0 predates the `protocol` request (it answers `err unknown request "protocol"`) and
/// otherwise speaks 0.1.0.
pub const PROTOCOL: &str = "0.1.0";

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
    Protocol,
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
            "protocol" => Command::Protocol,
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
            Command::Protocol => f.write_str("protocol"),
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

pub fn protocol_reply() -> String {
    format!("protocol {PROTOCOL}")
}

/// The daemon's protocol version from its reply to `protocol`; 0.1.0 for a daemon that predates the request.
pub fn parse_protocol(line: &str) -> &str {
    line.strip_prefix("protocol ").unwrap_or("0.1.0")
}

/// Whether a daemon speaking `theirs` serves a client built for `ours`, by Cargo's caret rule: the same major version
/// (the same minor while the major is 0, the same patch while both are 0) and no older.
pub fn compatible(theirs: &str, ours: &str) -> bool {
    let parse = |v: &str| -> Option<[u64; 3]> {
        let mut it = v.split('.').map(|n| n.parse().ok());
        let parsed = [it.next()??, it.next()??, it.next()??];
        it.next().is_none().then_some(parsed)
    };
    let (Some(t), Some(o)) = (parse(theirs), parse(ours)) else { return false };
    let same = if o[0] > 0 {
        1
    } else if o[1] > 0 {
        2
    } else {
        3
    };
    t[..same] == o[..same] && t >= o
}

/// The per-user private directory for the socket, the lock file and the daemon log.
/// Absolute, so the daemon (which leaves its starting directory) and its clients agree on a relative
/// `$XDG_RUNTIME_DIR` on Windows (Unix passes over one, and makes its fallback absolute).
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
        for (cmd, text) in [
            (Command::Kind(Kind::Pink), "kind pink"),
            (Command::Vol(40), "vol 40"),
            (Command::VolBy(-5), "vol -5"),
            (Command::VolBy(5), "vol +5"),
            (Command::Mute(true), "mute"),
            (Command::Mute(false), "unmute"),
            (Command::ToggleMute, "toggle"),
            (Command::Status, "status"),
            (Command::Watch, "watch"),
            (Command::Protocol, "protocol"),
            (Command::Quit, "quit"),
        ] {
            assert_eq!(cmd.to_string(), text);
            assert_eq!(Command::parse(text), Ok(cmd));
        }
        let state = |kind, vol, muted, error: Option<&str>| State { kind, vol, muted, error: error.map(String::from) };
        for (s, text) in [
            (state(Kind::Brown, 50, false, None), "state brown 50 playing"),
            (
                state(Kind::Violet, 7, true, Some("no default output device")),
                "state violet 7 muted no default output device",
            ),
        ] {
            assert_eq!(s.to_string(), text);
            assert_eq!(State::parse(text), Ok(s));
        }
        assert_eq!(State::parse("err nope"), Err("nope".into()));
        assert_eq!(parse_protocol(&protocol_reply()), PROTOCOL);
        assert_eq!(parse_protocol("err unknown request \"protocol\""), "0.1.0");
        for (theirs, ours, ok) in [
            ("0.1.0", "0.1.0", true),
            ("0.1.3", "0.1.0", true),
            ("0.1.0", "0.1.3", false),
            ("0.2.0", "0.1.0", false),
            ("0.0.1", "0.0.2", false),
            ("1.4.0", "1.2.0", true),
            ("1.1.9", "1.2.0", false),
            ("2.0.0", "1.2.0", false),
            ("0.1", "0.1.0", false),
            ("0.1.0.1", "0.1.0", false),
        ] {
            assert_eq!(compatible(theirs, ours), ok, "{theirs} serving {ours}");
        }
        assert!(Command::parse("kind grey").is_err());
        assert!(Command::parse("mute now").is_err());
        assert!(Command::parse("vol 5 6").is_err());
        assert!(Command::parse("vol").is_err());
    }
}
