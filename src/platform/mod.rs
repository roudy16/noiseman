//! Everything that differs between operating systems, behind one interface. Both halves use it; nothing else in the
//! crate may name an OS API (`std::os::*`, `libc`, `windows_sys`) or use `#[cfg(unix)]`/`#[cfg(windows)]` for IPC,
//! processes or paths.
//!
//! Each backend (`unix.rs`, `windows.rs`) provides the same items with the same signatures:
//!
//! - [`runtime_dir`]: a per-user private directory for the lock file and the daemon log.
//! - [`spawn_detached`]: starts a process that outlives the terminal (and the frontend) that started it.
//! - [`terminal_truecolor`]: whether the terminal on stdout renders 24-bit color though its environment does not say
//!   so (`COLORTERM`, `TERM`); callers check those variables and `NO_COLOR` themselves.
//! - [`ipc`]: the daemon's endpoint, one per runtime dir, reachable only by the same user:
//!   - `Endpoint::in_dir(&Path)` names it; it implements `Display`.
//!   - `Listener::bind(&Endpoint)` takes it over. The caller must hold the daemon lock: a leftover endpoint from a
//!     dead daemon is stale. `Listener::accept()` waits for the next client and returns `Ok(None)` once closed.
//!   - `Listener::closer()` returns a `Closer`, `Send`, whose `close()` stops new connections at once (clients then
//!     get `NotFound`) while connected clients stay connected.
//!   - `Stream::connect(&Endpoint)` connects; `NotFound` or `ConnectionRefused` means no daemon is listening.
//!   - `Stream` is `Send + Sync`, and `&Stream` implements `Read` and `Write`, so one thread can block reading while
//!     others write. `try_clone()` gives another handle to the same connection. `set_write_timeout()` bounds each
//!     write (a timed-out write fails with `WouldBlock` or `TimedOut`). `shutdown()` on an accepted stream ends the
//!     connection both ways: the client sees EOF, and reads already blocked on the daemon's side return EOF (only the
//!     daemon calls it). A peer that has gone reads as EOF, not an error.

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

pub use imp::{runtime_dir, spawn_detached, terminal_truecolor};

pub mod ipc {
    pub use super::imp::ipc::{Endpoint, Listener, Stream};
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, ErrorKind, Write};
    use std::sync::{Arc, Barrier};
    use std::thread;
    use std::time::{Duration, Instant};

    use super::ipc::{Endpoint, Listener, Stream};

    /// A fresh private dir per test, so tests never meet a real daemon or each other.
    fn endpoint(name: &str) -> (Endpoint, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("noiseman-test-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        (Endpoint::in_dir(&dir).unwrap(), dir)
    }

    #[test]
    fn ipc_contract() {
        let (ep, dir) = endpoint("contract");
        assert_eq!(Stream::connect(&ep).err().map(|e| e.kind()), Some(ErrorKind::NotFound));
        let listener = Listener::bind(&ep).unwrap();
        let closer = listener.closer().unwrap();
        let server = thread::spawn(move || {
            let conn = listener.accept().unwrap().expect("a client");
            conn.set_write_timeout(Some(Duration::from_millis(200))).unwrap();
            // Reads on one thread while another writes: the backend's reader and player threads.
            let writer = conn.try_clone().unwrap();
            let echo = thread::spawn(move || {
                for line in BufReader::new(&conn).lines() {
                    (&conn).write_all(format!("echo {}\n", line.unwrap()).as_bytes()).unwrap();
                }
            });
            (&writer).write_all(b"hello\n").unwrap();
            echo.join().unwrap();
            // The client is gone: a write fails instead of blocking.
            let deadline = Instant::now() + Duration::from_secs(2);
            while (&writer).write_all(b"late\n").is_ok() {
                assert!(Instant::now() < deadline, "writes to a closed peer keep succeeding");
                thread::sleep(Duration::from_millis(10));
            }
            listener
        });

        // Another spelling of the same dir reaches the same daemon.
        let client = Stream::connect(&Endpoint::in_dir(&dir.join(".")).unwrap()).unwrap();
        let mut reader = BufReader::new(client.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert_eq!(line, "hello\n");
        (&client).write_all(b"ping\n").unwrap();
        line.clear();
        reader.read_line(&mut line).unwrap();
        assert_eq!(line, "echo ping\n");
        drop(reader);
        drop(client);
        let listener = server.join().unwrap();

        // Closing stops new clients, and a blocked accept returns.
        let accepting = thread::spawn(move || listener.accept().map(|c| c.is_none()));
        thread::sleep(Duration::from_millis(50));
        closer.close().unwrap();
        let refused = Stream::connect(&ep).err().map(|e| e.kind());
        assert!(matches!(refused, Some(ErrorKind::NotFound | ErrorKind::ConnectionRefused)), "{refused:?}");
        if cfg!(windows) {
            assert!(accepting.join().unwrap().unwrap(), "accept after close returns None");
        }
        std::fs::remove_dir_all(dir).ok();
    }

    /// A client connecting while the daemon quits must not delay `close` (the daemon's exit) or be left unanswered
    /// with a stream `accept` handed out after `close`.
    #[test]
    fn close_racing_connect_is_prompt() {
        let (ep, dir) = endpoint("race");
        for round in 0..300 {
            let listener = Listener::bind(&ep).unwrap();
            let closer = listener.closer().unwrap();
            // Windows: a blocked accept, which close must wake. Unix: none (close leaves accept blocked forever, and
            // a connect completes against the backlog), so the listener just drops at the end of the round.
            let (accepting, held) = if cfg!(windows) {
                let accepting = thread::spawn(move || {
                    let mut handed_out = Vec::new();
                    while let Ok(Some(conn)) = listener.accept() {
                        handed_out.push(conn);
                    }
                    handed_out
                });
                (Some(accepting), None)
            } else {
                (None, Some(listener))
            };
            // Start the connect and the close together, then stagger the close by 0-150 us so some rounds land on
            // the connect completing.
            let go = Arc::new(Barrier::new(2));
            let connecting = {
                let (ep, go) = (Endpoint::in_dir(&dir).unwrap(), go.clone());
                thread::spawn(move || {
                    go.wait();
                    Stream::connect(&ep).ok()
                })
            };
            go.wait();
            let stagger = Instant::now() + Duration::from_micros(round % 16 * 10);
            while Instant::now() < stagger {
                std::hint::spin_loop();
            }
            let started = Instant::now();
            closer.close().unwrap();
            let took = started.elapsed();
            assert!(took < Duration::from_millis(500), "round {round}: close took {took:?}");
            drop(connecting.join().unwrap());
            if let Some(accepting) = accepting {
                drop(accepting.join().unwrap());
            }
            drop(held);
        }
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn shutdown_gives_eof_to_both_sides() {
        let (ep, dir) = endpoint("shutdown");
        let listener = Listener::bind(&ep).unwrap();
        let client = Stream::connect(&ep).unwrap();
        let conn = listener.accept().unwrap().unwrap();
        let blocked = conn.try_clone().unwrap();
        let reading = thread::spawn(move || {
            let mut line = String::new();
            BufReader::new(&blocked).read_line(&mut line).map(|n| n == 0)
        });
        thread::sleep(Duration::from_millis(50));
        conn.shutdown().unwrap();
        assert!(reading.join().unwrap().unwrap(), "a read blocked during shutdown returns EOF");
        let mut line = String::new();
        assert_eq!(BufReader::new(&client).read_line(&mut line).unwrap(), 0, "the peer sees EOF");
        std::fs::remove_dir_all(dir).ok();
    }
}
