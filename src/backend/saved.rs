//! The kind and volume, saved across player restarts in the state dir as `kind <kind>\nvol <0-100>\n`.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;

use crate::platform;
use crate::protocol::Kind;

pub const DEFAULT: (Kind, i32) = (Kind::Brown, 50);

fn path() -> Result<PathBuf, String> {
    Ok(platform::state_dir()?.join("state"))
}

/// The saved kind and volume; a missing file gives the defaults, and each unreadable field its own default.
pub fn load() -> (Kind, i32) {
    let path = match path() {
        Ok(p) => p,
        Err(e) => {
            log!("noiseman: not restoring the last kind and volume: {e}");
            return DEFAULT;
        }
    };
    match fs::read_to_string(&path) {
        Ok(text) => {
            let (saved, ok) = parse(&text);
            if !ok {
                log!("noiseman: {} is malformed; using the default for each unreadable field", path.display());
            }
            saved
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => DEFAULT,
        Err(e) => {
            log!("noiseman: cannot read {}: {e}", path.display());
            DEFAULT
        }
    }
}

/// Replaces the file atomically: a reader sees either the old content or the new.
pub fn save((kind, vol): (Kind, i32)) -> Result<(), String> {
    let path = path()?;
    let tmp = path.with_extension("tmp");
    let mut file = platform::open_private(OpenOptions::new().write(true).create(true).truncate(true), &tmp)
        .map_err(|e| format!("cannot open {}: {e}", tmp.display()))?;
    let text = format!("kind {}\nvol {vol}\n", kind.label().to_ascii_lowercase());
    file.write_all(text.as_bytes()).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
    fs::rename(&tmp, &path).map_err(|e| format!("cannot replace {}: {e}", path.display()))
}

/// The values found, defaults for the rest, and whether the text was exactly one valid line per field.
fn parse(text: &str) -> ((Kind, i32), bool) {
    let (mut kind, mut vol, mut bad) = (None, None, false);
    for line in text.lines() {
        match line.split_once(' ') {
            Some(("kind", k)) if kind.is_none() => kind = Kind::parse(k),
            Some(("vol", v)) if vol.is_none() => vol = v.parse().ok().filter(|v| (0..=100).contains(v)),
            _ => bad = true,
        }
    }
    let ok = !bad && kind.is_some() && vol.is_some();
    ((kind.unwrap_or(DEFAULT.0), vol.unwrap_or(DEFAULT.1)), ok)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_falls_back_field_by_field() {
        assert_eq!(parse("kind pink\nvol 30\n"), ((Kind::Pink, 30), true));
        assert_eq!(parse("kind grey\nvol 30\n"), ((Kind::Brown, 30), false));
        assert_eq!(parse("kind pink\nvol 300\n"), ((Kind::Pink, 50), false));
        assert_eq!(parse("kind pink\n"), ((Kind::Pink, 50), false));
        assert_eq!(parse("vol 7\nkind violet\nmuted\n"), ((Kind::Violet, 7), false));
        assert_eq!(parse(""), (DEFAULT, false));
    }
}
