//! Windows: an AF_UNIX socket (Windows 10 1803+) in a private runtime dir, a detached process, console color support.

use std::fs;
use std::io;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

use windows_sys::Win32::Foundation::ERROR_ACCESS_DENIED;
use windows_sys::Win32::System::Console::{
    ENABLE_VIRTUAL_TERMINAL_PROCESSING, GetConsoleMode, GetStdHandle, STD_OUTPUT_HANDLE, SetConsoleMode,
};
use windows_sys::Win32::System::Threading::{CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS};

/// Whether stdout is a console that takes VT sequences: Windows Terminal and the Windows 10+ console host both render
/// them in 24-bit color, and neither sets `COLORTERM` or `TERM`. Turns VT processing on, as crossterm does anyway.
/// False for a pipe, a file, or a non-console terminal such as mintty, which advertise themselves through `TERM`.
pub fn terminal_truecolor() -> bool {
    // SAFETY: the std handle needs no closing; the mode calls fail cleanly on a handle that is not a console.
    unsafe {
        let out = GetStdHandle(STD_OUTPUT_HANDLE);
        let mut mode = 0;
        GetConsoleMode(out, &mut mode) != 0
            && (mode & ENABLE_VIRTUAL_TERMINAL_PROCESSING != 0
                || SetConsoleMode(out, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING) != 0)
    }
}

/// `$XDG_RUNTIME_DIR` if set (as on Unix, so tests and gates can run a private daemon), else `%LOCALAPPDATA%\noiseman`,
/// whose inherited ACL admits only the user, SYSTEM and administrators.
pub fn runtime_dir() -> Result<PathBuf, String> {
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        return Ok(PathBuf::from(dir));
    }
    let base = std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA is not set")?;
    let dir = PathBuf::from(base).join("noiseman");
    fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    Ok(dir)
}

/// Nothing to do: the socket file inherits the runtime dir's ACL, and connecting needs write access to it (which a
/// low-integrity process of the same user also lacks).
pub fn restrict_to_owner(_path: &Path) -> io::Result<()> {
    Ok(())
}

/// Spawns `cmd` with no console, in its own process group (so the terminal's Ctrl+C and closing do not reach it), and
/// out of the caller's job where the job allows it (some terminals kill their job when they close).
pub fn spawn_detached(cmd: &mut Command) -> io::Result<Child> {
    let flags = DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP;
    match cmd.creation_flags(flags | CREATE_BREAKAWAY_FROM_JOB).spawn() {
        Err(e) if e.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32) => cmd.creation_flags(flags).spawn(),
        result => result,
    }
}

pub mod net {
    //! The part of `std::os::unix::net` noiseman uses, which std does not offer on Windows yet.

    use std::io::{self, Read, Write};
    use std::mem::{size_of, zeroed};
    use std::net::Shutdown;
    use std::path::Path;
    use std::ptr::{null, null_mut};
    use std::sync::{Arc, OnceLock};
    use std::time::Duration;

    use windows_sys::Win32::Networking::WinSock::{
        AF_UNIX, INVALID_SOCKET, SD_BOTH, SD_RECEIVE, SD_SEND, SO_SNDTIMEO, SOCK_STREAM, SOCKADDR, SOCKADDR_UN, SOCKET,
        SOCKET_ERROR, SOL_SOCKET, WSA_FLAG_NO_HANDLE_INHERIT, WSA_FLAG_OVERLAPPED, WSADATA, WSAESHUTDOWN,
        WSAGetLastError, WSASocketW, WSAStartup, accept, bind, closesocket, connect, listen, recv, send, setsockopt,
        shutdown,
    };

    struct Socket(SOCKET);

    impl Drop for Socket {
        fn drop(&mut self) {
            // SAFETY: ours, closed once.
            unsafe { closesocket(self.0) };
        }
    }

    fn last_error() -> io::Error {
        // SAFETY: reads this thread's error code.
        io::Error::from_raw_os_error(unsafe { WSAGetLastError() })
    }

    fn check(result: i32) -> io::Result<i32> {
        if result == SOCKET_ERROR { Err(last_error()) } else { Ok(result) }
    }

    fn checked(s: SOCKET) -> io::Result<Socket> {
        if s == INVALID_SOCKET { Err(last_error()) } else { Ok(Socket(s)) }
    }

    /// Overlapped, as std's sockets are, so a read blocked on one thread does not hold up writes from another.
    fn socket() -> io::Result<Socket> {
        static STARTED: OnceLock<i32> = OnceLock::new();
        // SAFETY: WSAStartup only fills the WSADATA. Never paired with WSACleanup, as in std.
        let code = *STARTED.get_or_init(|| unsafe { WSAStartup(0x202, &mut zeroed::<WSADATA>()) });
        if code != 0 {
            return Err(io::Error::from_raw_os_error(code));
        }
        let flags = WSA_FLAG_OVERLAPPED | WSA_FLAG_NO_HANDLE_INHERIT;
        // SAFETY: no protocol info; the result is checked.
        checked(unsafe { WSASocketW(AF_UNIX as i32, SOCK_STREAM, 0, null(), 0, flags) })
    }

    /// A new socket, passed to `f` with the address of `path` (UTF-8, as Windows expects, and under 108 bytes).
    fn with_address(path: &Path, f: impl FnOnce(SOCKET, *const SOCKADDR, i32) -> i32) -> io::Result<Socket> {
        let bytes = path.to_str().ok_or_else(|| io::Error::other(format!("{} is not UTF-8", path.display())))?;
        // SAFETY: all-zero is a valid, empty SOCKADDR_UN.
        let mut addr: SOCKADDR_UN = unsafe { zeroed() };
        if bytes.len() >= addr.sun_path.len() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("{} is too long", path.display())));
        }
        addr.sun_family = AF_UNIX;
        for (d, &b) in addr.sun_path.iter_mut().zip(bytes.as_bytes()) {
            *d = b as _;
        }
        let s = socket()?;
        check(f(s.0, (&raw const addr).cast(), size_of::<SOCKADDR_UN>() as i32))?;
        Ok(s)
    }

    pub struct UnixListener(Socket);

    impl UnixListener {
        pub fn bind(path: impl AsRef<Path>) -> io::Result<UnixListener> {
            // SAFETY: a valid address of the given size.
            let s = with_address(path.as_ref(), |s, addr, len| unsafe { bind(s, addr, len) })?;
            // SAFETY: a bound socket.
            check(unsafe { listen(s.0, 128) })?;
            Ok(UnixListener(s))
        }

        pub fn accept(&self) -> io::Result<UnixStream> {
            // SAFETY: a listening socket; no peer address wanted.
            checked(unsafe { accept(self.0.0, null_mut(), null_mut()) }).map(|s| UnixStream(Arc::new(s)))
        }

        pub fn incoming(&self) -> impl Iterator<Item = io::Result<UnixStream>> + '_ {
            std::iter::repeat_with(|| self.accept())
        }
    }

    /// Clones share the socket, which closes when the last one drops, as with a duplicated handle.
    pub struct UnixStream(Arc<Socket>);

    impl UnixStream {
        pub fn connect(path: impl AsRef<Path>) -> io::Result<UnixStream> {
            // SAFETY: a valid address of the given size.
            let s = with_address(path.as_ref(), |s, addr, len| unsafe { connect(s, addr, len) })?;
            Ok(UnixStream(Arc::new(s)))
        }

        pub fn try_clone(&self) -> io::Result<UnixStream> {
            Ok(UnixStream(self.0.clone()))
        }

        /// A write that times out fails with `TimedOut`.
        pub fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
            if timeout == Some(Duration::ZERO) {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "zero timeout"));
            }
            let ms = timeout.map_or(0, |t| t.as_millis().clamp(1, u32::MAX as u128) as u32);
            // SAFETY: a u32 option value, of its own size.
            check(unsafe { setsockopt(self.0.0, SOL_SOCKET, SO_SNDTIMEO, (&raw const ms).cast(), 4) }).map(drop)
        }

        pub fn shutdown(&self, how: Shutdown) -> io::Result<()> {
            let how = match how {
                Shutdown::Read => SD_RECEIVE,
                Shutdown::Write => SD_SEND,
                Shutdown::Both => SD_BOTH,
            };
            // SAFETY: a connected socket.
            check(unsafe { shutdown(self.0.0, how) }).map(drop)
        }
    }

    impl Read for &UnixStream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let len = buf.len().min(i32::MAX as usize) as i32;
            // SAFETY: `buf` is valid for `len` bytes.
            match check(unsafe { recv(self.0.0, buf.as_mut_ptr(), len, 0) }) {
                // Reading after our own shutdown: EOF, as on Unix.
                Err(e) if e.raw_os_error() == Some(WSAESHUTDOWN) => Ok(0),
                r => r.map(|n| n as usize),
            }
        }
    }

    impl Write for &UnixStream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let len = buf.len().min(i32::MAX as usize) as i32;
            // SAFETY: `buf` is valid for `len` bytes.
            check(unsafe { send(self.0.0, buf.as_ptr(), len, 0) }).map(|n| n as usize)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Read for UnixStream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            (&*self).read(buf)
        }
    }

    impl Write for UnixStream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            (&*self).write(buf)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
}
