//! Everything that differs between operating systems, behind one interface. Nothing else in the crate may name an OS
//! API (`std::os::*`, `libc`, `windows_sys`) or use `#[cfg(unix)]`/`#[cfg(windows)]` for IPC, processes or paths.
//!
//! Each backend (`unix.rs`, `windows.rs`) provides the same items with the same signatures:
//!
//! - [`runtime_dir`]: a per-user private directory for the socket, the lock file and the daemon log.
//! - [`restrict_to_owner`]: makes a just-bound socket file reachable only by its owner.
//! - [`spawn_detached`]: starts a process that outlives the terminal (and the frontend) that started it.
//! - [`terminal_truecolor`]: whether the terminal on stdout renders 24-bit color though its environment does not say
//!   so (`COLORTERM`, `TERM`); callers check those variables and `NO_COLOR` themselves.
//! - [`net`]: `UnixListener` and `UnixStream`, std's on Unix and the same subset of their API on Windows (AF_UNIX,
//!   Windows 10 1803+), which std lacks there.

#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix as imp;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as imp;

#[cfg(not(any(unix, windows)))]
compile_error!("noiseman supports Unix (Linux, macOS) and Windows");

pub use imp::{net, restrict_to_owner, runtime_dir, spawn_detached, terminal_truecolor};

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
    use std::net::Shutdown;
    use std::thread;
    use std::time::{Duration, Instant};

    use super::net::{UnixListener, UnixStream};

    /// What the backend and client rely on, so Windows' own `net` behaves as std's does on Unix.
    #[test]
    fn unix_socket() {
        let dir = std::env::temp_dir().join(format!("noiseman-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("noiseman.sock");
        let gone = |r: std::io::Result<UnixStream>| {
            matches!(r.err().map(|e| e.kind()), Some(ErrorKind::NotFound | ErrorKind::ConnectionRefused))
        };
        assert!(gone(UnixStream::connect(&path)), "no listener yet");

        let listener = UnixListener::bind(&path).unwrap();
        super::restrict_to_owner(&path).unwrap();
        let client = UnixStream::connect(&path).unwrap();
        let conn = listener.incoming().next().unwrap().unwrap();

        // One thread blocks reading while another writes on the same connection: the daemon's reader and player.
        let reader = conn.try_clone().unwrap();
        let echo = thread::spawn(move || {
            let mut line = String::new();
            BufReader::new(&reader).read_line(&mut line).unwrap();
            line
        });
        thread::sleep(Duration::from_millis(50));
        (&conn).write_all(b"hello\n").unwrap();
        let mut from_server = BufReader::new(client.try_clone().unwrap());
        let mut line = String::new();
        from_server.read_line(&mut line).unwrap();
        assert_eq!(line, "hello\n");
        (&client).write_all(b"ping\n").unwrap();
        assert_eq!(echo.join().unwrap(), "ping\n");

        // A client that stops reading: writes time out instead of blocking the daemon.
        conn.set_write_timeout(Some(Duration::from_millis(100))).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let err = loop {
            if let Err(e) = (&conn).write_all(&[b'x'; 65536]) {
                break e;
            }
            assert!(Instant::now() < deadline, "writes to a stalled peer never time out");
        };
        assert!(matches!(err.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock), "{err:?}");

        // Shutdown: the peer reads the rest, then EOF.
        conn.shutdown(Shutdown::Both).unwrap();
        let mut rest = Vec::new();
        assert!(from_server.read_to_end(&mut rest).is_ok(), "the peer sees EOF");

        // With the socket file gone, clients find no daemon.
        std::fs::remove_file(&path).unwrap();
        assert!(gone(UnixStream::connect(&path)), "socket file removed");
        std::fs::remove_dir_all(dir).ok();
    }
}
