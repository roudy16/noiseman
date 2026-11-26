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
        return Err(format!("{} is not a private directory owned by uid {uid}", dir.display()));
    }
    Ok(())
}

fn xdg_runtime_dir() -> Option<Result<PathBuf, String>> {
    let dir = PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?);
    Some(check_private(&dir, uid()).map(|()| dir))
}

/// `$XDG_RUNTIME_DIR` when it is private, else a 0700 `noiseman-<uid>` dir in the temp dir. Falling back rather than
/// failing keeps noiseman working, and the daemon and its clients still meet, since they all fall back alike.
pub fn runtime_dir() -> Result<PathBuf, String> {
    if let Some(Ok(dir)) = xdg_runtime_dir() {
        return Ok(dir);
    }
    let uid = uid();
    let dir = std::path::absolute(std::env::temp_dir().join(format!("noiseman-{uid}")))
        .map_err(|e| format!("cannot resolve the temp dir: {e}"))?;
    match fs::DirBuilder::new().mode(0o700).create(&dir) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(format!("cannot create {}: {e}", dir.display())),
    }
    check_private(&dir, uid)?;
    Ok(dir)
}

/// Why `runtime_dir` passed over `$XDG_RUNTIME_DIR`, if it did.
pub fn runtime_dir_rejected() -> Option<String> {
    xdg_runtime_dir()?.err()
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
}
