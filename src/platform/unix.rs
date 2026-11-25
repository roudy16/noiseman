//! Unix (Linux, macOS): a Unix socket in a private runtime dir, `setsid` to detach.

use std::fs;
use std::io;
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
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

pub mod ipc {
    use std::fmt;
    use std::fs;
    use std::io::{self, Read, Write};
    use std::net::Shutdown;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    /// `noiseman.sock` in the runtime dir.
    pub struct Endpoint(PathBuf);

    impl Endpoint {
        pub fn in_dir(dir: &Path) -> io::Result<Endpoint> {
            Ok(Endpoint(dir.join("noiseman.sock")))
        }
    }

    impl fmt::Display for Endpoint {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            self.0.display().fmt(f)
        }
    }

    pub struct Listener {
        inner: UnixListener,
        path: PathBuf,
    }

    /// Removes the socket file: new clients get `NotFound`. A blocked `accept` stays blocked.
    pub struct Closer(PathBuf);

    impl Listener {
        /// Replaces a stale socket file (the caller holds the lock, so no live daemon owns it); owner-only.
        pub fn bind(ep: &Endpoint) -> io::Result<Listener> {
            let path = &ep.0;
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(io::Error::new(e.kind(), format!("cannot remove stale {}: {e}", path.display()))),
            }
            let inner = UnixListener::bind(path)?;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))
                .map_err(|e| io::Error::new(e.kind(), format!("cannot restrict {}: {e}", path.display())))?;
            Ok(Listener { inner, path: path.clone() })
        }

        pub fn accept(&self) -> io::Result<Option<Stream>> {
            self.inner.accept().map(|(s, _)| Some(Stream(s)))
        }

        pub fn closer(&self) -> io::Result<Closer> {
            Ok(Closer(self.path.clone()))
        }
    }

    impl Closer {
        pub fn close(self) -> io::Result<()> {
            fs::remove_file(&self.0)
        }
    }

    pub struct Stream(UnixStream);

    impl Stream {
        pub fn connect(ep: &Endpoint) -> io::Result<Stream> {
            UnixStream::connect(&ep.0).map(Stream)
        }

        pub fn try_clone(&self) -> io::Result<Stream> {
            self.0.try_clone().map(Stream)
        }

        pub fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
            self.0.set_write_timeout(timeout)
        }

        pub fn shutdown(&self) -> io::Result<()> {
            self.0.shutdown(Shutdown::Both)
        }
    }

    /// A peer that exits with unread data resets instead of closing; either way it has gone.
    fn eof_if_reset(r: io::Result<usize>) -> io::Result<usize> {
        match r {
            Err(e) if e.kind() == io::ErrorKind::ConnectionReset => Ok(0),
            r => r,
        }
    }

    impl Read for &Stream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            eof_if_reset((&self.0).read(buf))
        }
    }

    impl Write for &Stream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            (&self.0).write(buf)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Read for Stream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            (&*self).read(buf)
        }
    }

    impl Write for Stream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            (&*self).write(buf)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
}
