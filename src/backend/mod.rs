//! The player daemon: owns the audio stream and serves the protocol on its endpoint (`platform::ipc`). Knows nothing
//! of frontends.

mod audio;
mod player;

use std::fs::{File, TryLockError};
use std::io::{self, BufRead, BufReader, Write};
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use crate::platform::ipc::{Listener, Stream};
use crate::protocol::{self, Command, State};
use player::Player;

/// A stalled client must not stall the player thread for longer than this per reply.
const WRITE_TIMEOUT: Duration = Duration::from_millis(200);
/// Longer than the slowest quit fade (100 ms + 3 device buffers).
const LOCK_WAIT: Duration = Duration::from_secs(1);

enum Msg {
    Request(Command, Arc<Stream>),
    AudioLost,
}

/// Runs the daemon until a `quit` request. Errors only if it cannot start.
pub fn serve(muted: bool) -> Result<(), String> {
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
    // Holding the lock means any endpoint left over is stale.
    let endpoint = protocol::endpoint()?;
    let listener = Listener::bind(&endpoint).map_err(|e| format!("cannot listen on {endpoint}: {e}"))?;
    let closer = listener.closer().map_err(|e| format!("cannot listen on {endpoint}: {e}"))?;

    let (tx, rx) = mpsc::channel();
    let accept_tx = tx.clone();
    thread::spawn(move || accept(listener, accept_tx));
    let on_lost = move || {
        if tx.send(Msg::AudioLost).is_err() {
            eprintln!("noiseman: audio lost after the player stopped");
        }
    };
    let mut player = Player::new(Arc::new(on_lost), muted);
    let mut watchers: Vec<Arc<Stream>> = Vec::new();
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
    // New clients now find no daemon; connected ones read EOF when it exits.
    if let Err(e) = closer.close() {
        eprintln!("noiseman: cannot close {endpoint}: {e}");
    }
    // Quit muted the player: let it fade instead of cutting the stream mid-buffer.
    if let Some(t) = player.deadline() {
        thread::sleep(t.saturating_duration_since(Instant::now()));
    }
    Ok(())
}

/// One write per line, so lines from different threads never interleave. A client that cannot
/// take a reply is disconnected, so it sees EOF instead of silently missing states.
fn send(client: &Stream, state: &State) -> io::Result<()> {
    let result = (&*client).write_all(format!("{state}\n").as_bytes());
    if result.is_err() {
        // Already failing; the shutdown error adds nothing.
        let _ = client.shutdown();
    }
    result
}

fn accept(listener: Listener, tx: Sender<Msg>) {
    loop {
        match listener.accept() {
            Ok(None) => return,
            Ok(Some(stream)) => {
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

fn serve_client(stream: Stream, tx: Sender<Msg>) {
    if let Err(e) = stream.set_write_timeout(Some(WRITE_TIMEOUT)) {
        eprintln!("noiseman: cannot set client write timeout: {e}");
        return;
    }
    let stream = Arc::new(stream);
    for line in BufReader::new(&*stream).lines() {
        let line = match line {
            Ok(l) => l,
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
