//! Unix (Linux, macOS): std's Unix sockets in a private runtime dir, `setsid` to detach.

use std::fs;
use std::io;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

/// `$XDG_RUNTIME_DIR` (per-user, mode 0700), else a 0700 `noiseman-<uid>` dir in the temp dir.
pub fn runtime_dir() -> Result<PathBuf, String> {
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        return Ok(PathBuf::from(dir));
    }
    // SAFETY: getuid cannot fail and touches no memory.
    let uid = unsafe { libc::getuid() };
    let dir = std::env::temp_dir().join(format!("noiseman-{uid}"));
    match fs::DirBuilder::new().mode(0o700).create(&dir) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(format!("cannot create {}: {e}", dir.display())),
    }
    let meta = fs::symlink_metadata(&dir).map_err(|e| format!("cannot stat {}: {e}", dir.display()))?;
    if !meta.is_dir() || meta.uid() != uid || meta.mode() & 0o077 != 0 {
        return Err(format!("{} is not a private directory owned by uid {uid}", dir.display()));
    }
    Ok(dir)
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

/// Mode 0600, though the runtime dir is private already.
pub fn restrict_to_owner(path: &Path) -> io::Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

pub mod net {
    pub use std::os::unix::net::{UnixListener, UnixStream};
}
