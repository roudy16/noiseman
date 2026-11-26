//! Unix (Linux, macOS): std's Unix sockets in a private runtime dir, `setsid` to detach.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

fn uid() -> u32 {
    // SAFETY: getuid cannot fail and touches no memory.
    unsafe { libc::getuid() }
}

/// An absolute path to a real directory (not a symlink to one) owned by `uid`, with no group or other access.
fn check_private(dir: &Path, uid: u32) -> Result<(), String> {
    if !dir.is_absolute() {
        return Err(format!("{} is not an absolute path", dir.display()));
    }
    let meta = fs::symlink_metadata(dir).map_err(|e| format!("cannot stat {}: {e}", dir.display()))?;
    if !meta.is_dir() || meta.uid() != uid || meta.mode() & 0o077 != 0 {
        return Err(format!(
            "{} (owner uid {}, mode {:o}) is not a private directory owned by uid {uid}",
            dir.display(),
            meta.uid(),
            meta.mode() & 0o7777
        ));
    }
    Ok(())
}

/// Creates `dir` 0700 unless it exists, then checks that it is private.
fn make_private(dir: &Path, uid: u32) -> Result<(), String> {
    match fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(format!("cannot create {}: {e}", dir.display())),
    }
    check_private(dir, uid)
}

/// `$XDG_CACHE_HOME`, else `~/.cache`, if absolute.
fn cache_home() -> Option<PathBuf> {
    let xdg = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).filter(|d| d.is_absolute());
    xdg.or_else(|| std::env::home_dir().map(|h| h.join(".cache"))).filter(|d| d.is_absolute())
}

/// The runtime dir, and why each candidate before it was passed over. Only the environment and the file system decide,
/// so the daemon and its clients agree:
///
/// 1. `$XDG_RUNTIME_DIR` when it is private: missing without pam_systemd (ssh, cron, containers, `sudo -u`).
/// 2. A 0700 `noiseman-<uid>` in the temp dir.
/// 3. When that fails, typically because another user made it first (the sticky temp dir keeps the user from removing
///    it), a 0700 `noiseman` in the cache home, which no other user can create there: per-user, and like the runtime
///    dir disposable. Its socket path must fit `sun_path`, which a deep home can exceed.
fn choose(
    uid: u32,
    xdg_runtime_dir: Option<PathBuf>,
    temp_dir: &Path,
    cache_home: Option<PathBuf>,
) -> (Result<PathBuf, String>, Vec<String>) {
    let mut passed_over = Vec::new();
    if let Some(dir) = xdg_runtime_dir {
        match check_private(&dir, uid) {
            Ok(()) => return (Ok(dir), passed_over),
            Err(why) => passed_over.push(format!("ignoring $XDG_RUNTIME_DIR: {why}")),
        }
    }
    let tmp_why = match std::path::absolute(temp_dir) {
        Ok(temp_dir) => {
            let tmp = temp_dir.join(format!("noiseman-{uid}"));
            match make_private(&tmp, uid) {
                Ok(()) => return (Ok(tmp), passed_over),
                Err(why) => why,
            }
        }
        Err(e) => format!("cannot resolve the temp dir: {e}"),
    };
    passed_over.push(format!("ignoring the temp dir: {tmp_why}"));
    let fallback = cache_home.ok_or_else(|| "no absolute $XDG_CACHE_HOME or $HOME".to_string()).and_then(|cache| {
        let dir = cache.join("noiseman");
        let sun_path = std::mem::size_of::<libc::sockaddr_un>() - std::mem::offset_of!(libc::sockaddr_un, sun_path);
        let socket = dir.join(crate::protocol::SOCKET_NAME);
        if socket.as_os_str().len() >= sun_path {
            return Err(format!("{} is too long for a socket path (limit {} bytes)", socket.display(), sun_path - 1));
        }
        fs::create_dir_all(&cache).map_err(|e| format!("cannot create {}: {e}", cache.display()))?;
        make_private(&dir, uid)?;
        Ok(dir)
    });
    let dir = fallback.map_err(|e| {
        format!("{tmp_why}, and {e}; set TMPDIR or XDG_RUNTIME_DIR to a private (0700) directory of yours")
    });
    (dir, passed_over)
}

fn choose_from_env() -> (Result<PathBuf, String>, Vec<String>) {
    choose(uid(), std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from), &std::env::temp_dir(), cache_home())
}

/// See `choose`. Falling back rather than failing keeps noiseman working.
pub fn runtime_dir() -> Result<PathBuf, String> {
    choose_from_env().0
}

/// Why `runtime_dir` passed over each candidate before the one it chose.
pub fn runtime_dir_rejected() -> Vec<String> {
    choose_from_env().1
}

/// A 0700 `noiseman` under `$XDG_STATE_HOME` if that is absolute (on macOS too, so the tests and gates can keep their
/// own), else under `~/.local/state` (macOS: `~/Library/Application Support`). Not the runtime dir: it must survive a
/// reboot.
pub fn state_dir() -> Result<PathBuf, String> {
    let xdg = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).filter(|d| d.is_absolute());
    let home_state = if cfg!(target_os = "macos") { "Library/Application Support" } else { ".local/state" };
    let base = xdg.or_else(|| std::env::home_dir().map(|h| h.join(home_state))).filter(|d| d.is_absolute());
    let base = base.ok_or("no absolute $XDG_STATE_HOME or $HOME")?;
    fs::create_dir_all(&base).map_err(|e| format!("cannot create {}: {e}", base.display()))?;
    let dir = base.join("noiseman");
    make_private(&dir, uid())?;
    Ok(dir)
}

/// Opens `path` with `opts`, refusing a symlink there (one planted in the dir would redirect the write), and creating
/// the file 0600.
pub fn open_private(opts: &mut OpenOptions, path: &Path) -> io::Result<File> {
    opts.custom_flags(libc::O_NOFOLLOW).mode(0o600).open(path)
}

/// Spawns `cmd` in its own session, so it outlives the terminal.
pub fn spawn_detached(cmd: &mut Command) -> io::Result<Child> {
    // SAFETY: setsid is async-signal-safe and touches no memory.
    unsafe {
        cmd.pre_exec(|| if libc::setsid() == -1 { Err(io::Error::last_os_error()) } else { Ok(()) });
    }
    cmd.spawn()
}

/// Unix terminals advertise 24-bit color themselves, through `COLORTERM`; nothing beyond that to detect.
pub fn terminal_truecolor() -> bool {
    false
}

/// Nothing to do: a window started from a desktop entry has its stderr in the session's journal.
#[cfg(feature = "gui")]
pub fn alert_if_no_console(_msg: &str) {}

/// Mode 0600, though the runtime dir is private already.
pub fn restrict_to_owner(path: &Path) -> io::Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

pub mod net {
    pub use std::os::unix::net::{UnixListener, UnixStream};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_private_dirs_pass() {
        let base = std::env::temp_dir().join(format!("nsm-private-{}", std::process::id()));
        fs::create_dir(&base).expect("cannot create the test dir");
        let dir = |name: &str, mode: u32| {
            let d = base.join(name);
            fs::create_dir(&d).expect("cannot create a dir");
            fs::set_permissions(&d, fs::Permissions::from_mode(mode)).expect("cannot chmod");
            d
        };
        let (private, open) = (dir("private", 0o700), dir("open", 0o755));
        let (link, file) = (base.join("link"), base.join("file"));
        std::os::unix::fs::symlink(&private, &link).expect("cannot symlink");
        File::create(&file).expect("cannot create a file");
        let passed = [&*private, &open, &link, &file, Path::new("relative")].map(|d| check_private(d, uid()).is_ok());
        fs::remove_dir_all(&base).expect("cannot remove the test dir");
        assert_eq!(passed, [true, false, false, false, false]);
    }

    #[test]
    fn falls_back_from_a_squatted_temp_dir() {
        let base = std::env::temp_dir().join(format!("nsm-choose-{}", std::process::id()));
        let (tmp, cache, uid) = (base.join("tmp"), base.join("cache"), uid());
        fs::create_dir_all(&tmp).expect("cannot create the test dir");
        let open = base.join("open");
        fs::DirBuilder::new().mode(0o755).create(&open).expect("cannot create a dir");

        let (dir, passed_over) = choose(uid, Some(open.clone()), &tmp, Some(cache.clone()));
        let own = tmp.join(format!("noiseman-{uid}"));
        assert_eq!(dir, Ok(own.clone()));
        assert_eq!(passed_over.len(), 1, "{passed_over:?}");
        assert!(passed_over[0].contains("$XDG_RUNTIME_DIR"));

        fs::set_permissions(&own, fs::Permissions::from_mode(0o755)).expect("cannot chmod");
        let (dir, passed_over) = choose(uid, None, &tmp, Some(cache.clone()));
        assert_eq!(dir, Ok(cache.join("noiseman")));
        assert!(passed_over[0].contains(&*own.to_string_lossy()), "{passed_over:?}");
        assert_eq!(fs::metadata(cache.join("noiseman")).expect("no fallback").mode() & 0o777, 0o700);
        assert_eq!(choose(uid, None, &tmp, Some(cache.clone())).0, dir, "the choice is deterministic");

        let xdg = base.join("xdg");
        fs::DirBuilder::new().mode(0o700).create(&xdg).expect("cannot create a dir");
        let (dir, passed_over) = choose(uid, Some(xdg.clone()), Path::new(""), None);
        assert_eq!((dir, passed_over.len()), (Ok(xdg), 0), "a private $XDG_RUNTIME_DIR needs no temp dir");

        let long = base.join("x".repeat(120));
        let err = choose(uid, None, &tmp, Some(long)).0.expect_err("socket path fits");
        assert!(err.contains("too long"), "{err}");
        let err = choose(uid, None, &tmp, None).0.expect_err("no cache home");
        assert!(err.contains(&*own.to_string_lossy()) && err.contains("owner uid") && err.contains("TMPDIR"), "{err}");
        fs::remove_dir_all(&base).expect("cannot remove the test dir");
    }
}
