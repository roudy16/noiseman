//! The player daemon: owns the audio stream and serves the protocol on a Unix socket. Knows nothing of frontends.

/// Every line the daemon writes, prefixed with the UTC time: its stderr is `noiseman.log` when a frontend started it.
macro_rules! log {
    ($($arg:tt)*) => { eprintln!("{} {}", $crate::backend::utc_now(), format_args!($($arg)*)) };
}
pub(crate) use log;

mod audio;
mod player;

use std::fs::{self, File, TryLockError};
use std::io::{self, BufRead, BufReader, Write};
use std::net::Shutdown;
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::platform::{
    self,
    net::{UnixListener, UnixStream},
};
use crate::protocol::{self, Command, State};
use player::Player;

/// A stalled client must not stall the player thread for longer than this per reply.
const WRITE_TIMEOUT: Duration = Duration::from_millis(200);
/// Longer than the slowest quit fade (`FADE_SECS` + 20 ms + 4 device buffers: about 380 ms with 4096-frame buffers).
const LOCK_WAIT: Duration = Duration::from_secs(1);

enum Msg {
    Request(Command, Arc<UnixStream>),
    AudioLost,
}

/// Runs the daemon until a `quit` request. Errors only if it cannot start.
pub fn serve(muted: bool) -> Result<(), String> {
    log!("noiseman {} started (protocol {})", env!("CARGO_PKG_VERSION"), protocol::PROTOCOL);
    let lock_path = protocol::runtime_dir()?.join("noiseman.lock");
    let lock = File::create(&lock_path).map_err(|e| format!("cannot create {}: {e}", lock_path.display()))?;
    // A daemon that just quit holds the lock through its fade-out, after removing its socket.
    let until = Instant::now() + LOCK_WAIT;
    loop {
        match lock.try_lock() {
            Ok(()) => break,
            Err(TryLockError::WouldBlock) if Instant::now() < until => thread::sleep(Duration::from_millis(20)),
            Err(TryLockError::WouldBlock) => return Err("already running".into()),
            Err(TryLockError::Error(e)) => return Err(format!("cannot lock {}: {e}", lock_path.display())),
        }
    }
    // Holding the lock means any socket file left here is stale.
    let path = protocol::socket_path()?;
    match fs::remove_file(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("cannot remove stale {}: {e}", path.display())),
    }
    let listener = UnixListener::bind(&path).map_err(|e| format!("cannot listen on {}: {e}", path.display()))?;
    platform::restrict_to_owner(&path).map_err(|e| format!("cannot restrict {}: {e}", path.display()))?;
    // Leave the starting shell's directory, which the daemon would otherwise keep in use for its whole life (e.g. a
    // drive that then cannot be unmounted). Every path above is absolute.
    if let Some(Err(e)) = path.parent().map(std::env::set_current_dir) {
        log!("noiseman: cannot leave the starting directory: {e}");
    }

    let (tx, rx) = mpsc::channel();
    let accept_tx = tx.clone();
    thread::spawn(move || accept(listener, accept_tx));
    let on_lost = move || {
        if tx.send(Msg::AudioLost).is_err() {
            log!("noiseman: audio lost after the player stopped");
        }
    };
    let mut player = Player::new(Arc::new(on_lost), muted);
    let mut watchers: Vec<Arc<UnixStream>> = Vec::new();
    let mut last = player.state();

    loop {
        let msg = match player.deadline() {
            Some(t) => rx.recv_timeout(t.saturating_duration_since(Instant::now())),
            None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match msg {
            Ok(Msg::Request(cmd, client)) => {
                player.apply(cmd);
                let reply = player.state();
                if let Err(e) = send(&client, &reply) {
                    log!("noiseman: cannot reply to client: {e}");
                } else if cmd == Command::Watch {
                    watchers.push(client);
                }
                if cmd == Command::Quit {
                    break;
                }
            }
            Ok(Msg::AudioLost) => player.audio_lost(),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                log!("noiseman: listener thread gone, stopping");
                break;
            }
        }
        player.tick(Instant::now());
        let state = player.state();
        if state != last {
            watchers.retain(|w| send(w, &state).is_ok());
            last = state;
        }
    }
    if let Err(e) = fs::remove_file(&path) {
        log!("noiseman: cannot remove {}: {e}", path.display());
    }
    // Quit muted the player: let it fade instead of cutting the stream mid-buffer.
    if let Some(t) = player.deadline() {
        thread::sleep(t.saturating_duration_since(Instant::now()));
    }
    Ok(())
}

/// One write per line, so lines from different threads never interleave. A client that cannot
/// take a reply is disconnected, so it sees EOF instead of silently missing states.
fn send(client: &UnixStream, state: &State) -> io::Result<()> {
    let result = (&*client).write_all(format!("{state}\n").as_bytes());
    if result.is_err() {
        // Already failing; the shutdown error adds nothing.
        let _ = client.shutdown(Shutdown::Both);
    }
    result
}

pub(crate) fn utc_now() -> String {
    utc(SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()))
}

/// Seconds since the Unix epoch as `YYYY-MM-DDThh:mm:ssZ`, by Howard Hinnant's days-to-civil algorithm.
fn utc(secs: u64) -> String {
    let (days, rem) = (secs / 86_400, secs % 86_400);
    let (era, doe) = ((days + 719_468) / 146_097, (days + 719_468) % 146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = era * 400 + yoe + u64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem / 60 % 60, rem % 60)
}

fn accept(listener: UnixListener, tx: Sender<Msg>) {
    for conn in listener.incoming() {
        match conn {
            Ok(stream) => {
                let tx = tx.clone();
                thread::spawn(move || serve_client(stream, tx));
            }
            Err(e) => {
                log!("noiseman: cannot accept client: {e}");
                // Errors like EMFILE persist; do not spin on them.
                thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

fn serve_client(stream: UnixStream, tx: Sender<Msg>) {
    if let Err(e) = stream.set_write_timeout(Some(WRITE_TIMEOUT)) {
        log!("noiseman: cannot set client write timeout: {e}");
        return;
    }
    let stream = Arc::new(stream);
    for line in BufReader::new(&*stream).lines() {
        let line = match line {
            Ok(l) => l,
            // A client that exits with unread replies resets instead of closing.
            Err(e) if e.kind() == io::ErrorKind::ConnectionReset => return,
            Err(e) => {
                log!("noiseman: cannot read from client: {e}");
                return;
            }
        };
        // `protocol` needs no player state, so it is answered here, like a parse error.
        let reply = match Command::parse(&line) {
            Ok(Command::Protocol) => protocol::protocol_reply(),
            Ok(cmd) => {
                if tx.send(Msg::Request(cmd, stream.clone())).is_err() {
                    return;
                }
                continue;
            }
            Err(e) => format!("err {e}"),
        };
        if let Err(e) = (&*stream).write_all(format!("{reply}\n").as_bytes()) {
            log!("noiseman: cannot reply to client: {e}");
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn utc() {
        assert_eq!(super::utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(super::utc(946_684_799), "1999-12-31T23:59:59Z");
        assert_eq!(super::utc(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(super::utc(1_795_630_473), "2026-11-25T18:14:33Z");
        assert_eq!(super::utc(4_107_542_399), "2100-02-28T23:59:59Z");
        assert_eq!(super::utc(4_107_542_400), "2100-03-01T00:00:00Z");
    }
}
