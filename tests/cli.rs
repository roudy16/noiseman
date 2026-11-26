//! The CLI and a muted daemon as separate processes, in a private runtime and state dir: no audio device needed,
//! and the user's saved kind and volume are never read or written.

use std::fs::{self, File};
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_noiseman");

/// Kills the daemon and removes the runtime dir (also the state dir), even when an assertion fails.
struct Runtime {
    dir: PathBuf,
    daemon: Option<Child>,
}

impl Runtime {
    fn new() -> Runtime {
        static COUNT: AtomicU32 = AtomicU32::new(0);
        // Short, since a socket path must fit in about 100 bytes (104 on macOS, whose temp dir alone is about 50).
        let base = if cfg!(unix) { PathBuf::from("/tmp") } else { std::env::temp_dir() };
        let dir = base.join(format!("nsm-{}-{}", std::process::id(), COUNT.fetch_add(1, Ordering::Relaxed)));
        fs::create_dir(&dir).unwrap_or_else(|e| panic!("cannot create {}: {e}", dir.display()));
        // noiseman passes over an `$XDG_RUNTIME_DIR` that others can enter.
        #[cfg(unix)]
        fs::set_permissions(&dir, std::os::unix::fs::PermissionsExt::from_mode(0o700)).expect("cannot chmod 0700");
        Runtime { dir, daemon: None }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(BIN);
        cmd.args(args).env("XDG_RUNTIME_DIR", &self.dir).env("XDG_STATE_HOME", &self.dir).stdin(Stdio::null());
        cmd
    }

    fn run(&self, args: &[&str]) -> (Option<i32>, String, String) {
        let Output { status, stdout, stderr } = self.command(args).output().expect("cannot run noiseman");
        (status.code(), String::from_utf8_lossy(&stdout).into(), String::from_utf8_lossy(&stderr).into())
    }

    fn start_muted_daemon(&mut self) {
        let log = File::create(self.dir.join("daemon.log")).expect("cannot create the daemon log");
        let daemon = self.command(&["daemon", "--muted"]).stdout(Stdio::null()).stderr(log).spawn();
        self.daemon = Some(daemon.expect("cannot start the daemon"));
        // Not the socket file: it exists before listen(), and a refused `vol` would autostart an unmuted daemon.
        self.wait_for("the daemon", |rt| rt.run(&["status"]).0 == Some(0));
    }

    fn stop_daemon(&mut self) {
        assert_eq!(self.run(&["stop"]), (Some(0), "stopped\n".into(), String::new()));
        let sock = self.dir.join("noiseman.sock");
        self.wait_for("the socket to go", |_| !sock.exists());
        self.wait_for("the daemon to exit", |rt| {
            rt.daemon.as_mut().is_some_and(|d| matches!(d.try_wait(), Ok(Some(_))))
        });
    }

    fn wait_for(&mut self, what: &str, mut done: impl FnMut(&mut Runtime) -> bool) {
        let until = Instant::now() + Duration::from_secs(10);
        while !done(self) {
            if Instant::now() >= until {
                let log = fs::read_to_string(self.dir.join("daemon.log")).unwrap_or_default();
                panic!("timed out waiting for {what}; daemon log:\n{log}");
            }
            thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        if let Some(mut daemon) = self.daemon.take() {
            // Already gone after a passing run; the kill then fails harmlessly.
            let _ = daemon.kill();
            if let Err(e) = daemon.wait() {
                eprintln!("cannot reap the daemon: {e}");
            }
        }
        if let Err(e) = fs::remove_dir_all(&self.dir) {
            eprintln!("cannot remove {}: {e}", self.dir.display());
        }
    }
}

#[test]
fn status_vol_stop_across_processes() {
    let mut rt = Runtime::new();
    assert_eq!(rt.run(&["status"]), (Some(1), String::new(), "noiseman: not running\n".into()));
    rt.start_muted_daemon();

    // The daemon runs, so neither command can start an unmuted one.
    assert_eq!(rt.run(&["vol", "30"]), (Some(0), "Brown 30% muted\n".into(), String::new()));
    assert_eq!(rt.run(&["status"]), (Some(0), "Brown 30% muted\n".into(), String::new()));
    rt.stop_daemon();
    assert_eq!(rt.run(&["stop"]), (Some(0), "not running\n".into(), String::new()));
}

#[test]
fn restart_restores_kind_and_volume() {
    let mut rt = Runtime::new();
    rt.start_muted_daemon();
    assert_eq!(rt.run(&["status"]), (Some(0), "Brown 50% muted\n".into(), String::new()), "no saved state yet");
    assert_eq!(rt.run(&["kind", "pink"]), (Some(0), "Pink 50% muted\n".into(), String::new()));
    assert_eq!(rt.run(&["vol", "30"]), (Some(0), "Pink 30% muted\n".into(), String::new()));
    // Within the save delay: quitting writes the pending change.
    rt.stop_daemon();
    let saved = fs::read_to_string(rt.dir.join("noiseman").join("state")).expect("cannot read the saved state");
    assert_eq!(saved, "kind pink\nvol 30\n");

    rt.start_muted_daemon();
    assert_eq!(rt.run(&["status"]), (Some(0), "Pink 30% muted\n".into(), String::new()));
    rt.stop_daemon();
}

#[cfg(unix)]
#[test]
fn lock_symlink_is_refused_not_truncated() {
    let mut rt = Runtime::new();
    let victim = rt.dir.join("victim");
    fs::write(&victim, "keep").expect("cannot write the victim file");
    std::os::unix::fs::symlink(&victim, rt.dir.join("noiseman.lock")).expect("cannot plant the symlink");

    let log = File::create(rt.dir.join("daemon.log")).expect("cannot create the daemon log");
    let daemon = rt.command(&["daemon", "--muted"]).stdout(Stdio::null()).stderr(log).spawn();
    rt.daemon = Some(daemon.expect("cannot start the daemon"));
    let mut status = None;
    rt.wait_for("the daemon to refuse the lock", |rt| {
        status = rt.daemon.as_mut().and_then(|d| d.try_wait().ok().flatten());
        status.is_some()
    });
    assert!(status.is_some_and(|s| !s.success()));
    let log = fs::read_to_string(rt.dir.join("daemon.log")).expect("cannot read the daemon log");
    assert!(log.contains("cannot open") && log.contains("noiseman.lock"), "unexpected daemon failure: {log}");
    assert_eq!(fs::read_to_string(&victim).expect("cannot read the victim file"), "keep");
}
