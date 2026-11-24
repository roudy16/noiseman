use std::fs::OpenOptions;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::process::{self, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::protocol::{self, Command, State};

const START_TIMEOUT: Duration = Duration::from_secs(3);

/// A connection to the player daemon. The halves are public so a frontend can wait for
/// pushed states on one thread while sending from another.
pub struct Client {
    pub sender: Sender,
    pub receiver: Receiver,
}

pub struct Sender(UnixStream);

pub struct Receiver(BufReader<UnixStream>);

impl Client {
    /// Connects to the running daemon; with `autostart`, starts one first if none is running.
    pub fn connect(autostart: bool) -> Result<Client, String> {
        let path = protocol::socket_path()?;
        let stream = match UnixStream::connect(&path) {
            Ok(s) => s,
            Err(_) if autostart => start_daemon()?,
            Err(e) if matches!(e.kind(), io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused) => {
                return Err("not running".into());
            }
            Err(e) => return Err(format!("cannot connect to {}: {e}", path.display())),
        };
        let writer = stream.try_clone().map_err(|e| format!("cannot clone socket: {e}"))?;
        Ok(Client { sender: Sender(writer), receiver: Receiver(BufReader::new(stream)) })
    }

    pub fn request(&mut self, cmd: Command) -> Result<State, String> {
        self.sender.send(cmd)?;
        self.receiver.recv()?.ok_or("daemon closed the connection")?
    }
}

impl Sender {
    pub fn send(&mut self, cmd: Command) -> Result<(), String> {
        writeln!(self.0, "{cmd}").map_err(|e| format!("cannot send {cmd}: {e}"))
    }
}

impl Receiver {
    /// The next reply or pushed state (`Err` inside for an `err` line); `Ok(None)` once the daemon has gone.
    pub fn recv(&mut self) -> Result<Option<Result<State, String>>, String> {
        let mut line = String::new();
        match self.0.read_line(&mut line) {
            Ok(0) => Ok(None),
            Ok(_) => Ok(Some(State::parse(line.trim_end()))),
            Err(e) => Err(format!("cannot read from daemon: {e}")),
        }
    }
}

/// Starts `noiseman daemon` in its own session (it outlives the terminal), logging to the runtime dir.
fn start_daemon() -> Result<UnixStream, String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot find own executable: {e}"))?;
    let log_path = protocol::runtime_dir()?.join("noiseman.log");
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|e| format!("cannot open {}: {e}", log_path.display()))?;
    let mut cmd = process::Command::new(exe);
    cmd.arg("daemon").stdin(Stdio::null()).stdout(Stdio::null()).stderr(log);
    // SAFETY: setsid is async-signal-safe and touches no memory.
    unsafe {
        cmd.pre_exec(|| if libc::setsid() == -1 { Err(io::Error::last_os_error()) } else { Ok(()) });
    }
    let mut child = cmd.spawn().map_err(|e| format!("cannot start daemon: {e}"))?;
    // Reaped on a thread so it never lingers as a zombie under a long-running frontend.
    thread::spawn(move || child.wait());
    let path = protocol::socket_path()?;
    let until = Instant::now() + START_TIMEOUT;
    // If another frontend started a daemon at the same moment, ours exits and theirs answers here.
    loop {
        match UnixStream::connect(&path) {
            Ok(s) => return Ok(s),
            Err(e) if Instant::now() >= until => {
                return Err(format!("daemon did not start ({e}); see {}", log_path.display()));
            }
            Err(_) => thread::sleep(Duration::from_millis(20)),
        }
    }
}
