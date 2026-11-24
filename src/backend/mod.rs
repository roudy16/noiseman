//! The player daemon: owns the audio stream and serves the protocol on a Unix socket. Knows nothing of frontends.

mod audio;
mod noise;
mod player;

use std::fs::{self, File, TryLockError};
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use crate::protocol::{self, Command, State};
use player::Player;

/// A stalled client must not stall the player thread for longer than this per reply.
const WRITE_TIMEOUT: Duration = Duration::from_millis(200);
/// Longer than the slowest quit fade (100 ms + 3 device buffers).
const LOCK_WAIT: Duration = Duration::from_secs(1);

enum Msg {
    Request(Command, Arc<UnixStream>),
    AudioLost,
}

/// Runs the daemon until a `quit` request. Errors only if it cannot start.
pub fn serve() -> Result<(), String> {
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
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
        .map_err(|e| format!("cannot restrict {}: {e}", path.display()))?;

    let (tx, rx) = mpsc::channel();
    let accept_tx = tx.clone();
    thread::spawn(move || accept(listener, accept_tx));
    let on_lost = move || {
        if tx.send(Msg::AudioLost).is_err() {
            eprintln!("noiseman: audio lost after the player stopped");
        }
    };
    let mut player = Player::new(Arc::new(on_lost));
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
                    eprintln!("noiseman: cannot reply to client: {e}");
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
                eprintln!("noiseman: listener thread gone, stopping");
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
        eprintln!("noiseman: cannot remove {}: {e}", path.display());
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

fn accept(listener: UnixListener, tx: Sender<Msg>) {
    for conn in listener.incoming() {
        match conn {
            Ok(stream) => {
                let tx = tx.clone();
                thread::spawn(move || serve_client(stream, tx));
            }
            Err(e) => {
                eprintln!("noiseman: cannot accept client: {e}");
                // Errors like EMFILE persist; do not spin on them.
                thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

fn serve_client(stream: UnixStream, tx: Sender<Msg>) {
    if let Err(e) = stream.set_write_timeout(Some(WRITE_TIMEOUT)) {
        eprintln!("noiseman: cannot set client write timeout: {e}");
        return;
    }
    let stream = Arc::new(stream);
    for line in BufReader::new(&*stream).lines() {
        let line = match line {
            Ok(l) => l,
            // A client that exits with unread replies resets instead of closing.
            Err(e) if e.kind() == io::ErrorKind::ConnectionReset => return,
            Err(e) => {
                eprintln!("noiseman: cannot read from client: {e}");
                return;
            }
        };
        match Command::parse(&line) {
            Ok(cmd) => {
                if tx.send(Msg::Request(cmd, stream.clone())).is_err() {
                    return;
                }
            }
            Err(e) => {
                if let Err(e) = (&*stream).write_all(format!("err {e}\n").as_bytes()) {
                    eprintln!("noiseman: cannot reply to client: {e}");
                    return;
                }
            }
        }
    }
}
