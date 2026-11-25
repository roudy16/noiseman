//! Windows: a named pipe per user and runtime dir, overlapped I/O, a detached process, console color support.

use std::fs;
use std::io;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
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
/// which the user profile's ACL keeps private. The pipe's own ACL does not depend on it.
pub fn runtime_dir() -> Result<PathBuf, String> {
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        return Ok(PathBuf::from(dir));
    }
    let base = std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA is not set")?;
    let dir = PathBuf::from(base).join("noiseman");
    fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    Ok(dir)
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

pub mod ipc {
    //! The daemon listens on `\\.\pipe\noiseman-<128 random bits>`, a fresh name each run, and publishes it in
    //! `noiseman.pipe` in the runtime dir; clients read the name from there. The pipe namespace is global and anyone
    //! may create a name in it, so a predictable name could be taken first (by another user, or by a sandboxed
    //! low-integrity process of this one) to lock the daemon out or to pose as it. A random name cannot be known
    //! before the daemon creates it, and only this user at medium integrity or above can write the runtime dir, the
    //! same trust as the Unix socket's 0700 dir. The file system also resolves every spelling of the dir to one file.
    //! Every instance has an ACL that admits only the user and is owned by the user (clients check that owner too),
    //! rejects remote clients, and the first is created with `FILE_FLAG_FIRST_PIPE_INSTANCE`. Handles are overlapped: a
    //! synchronous pipe handle serialises I/O, which would block the player's writes behind a client's pending read.

    use std::ffi::OsStr;
    use std::fmt;
    use std::fs;
    use std::io::{self, Read, Write};
    use std::mem::{size_of, zeroed};
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::ptr::{null, null_mut};
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
    use std::thread;
    use std::time::{Duration, Instant};

    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_ACCESS_DENIED, ERROR_BROKEN_PIPE, ERROR_FILE_NOT_FOUND, ERROR_IO_PENDING, ERROR_NO_DATA,
        ERROR_NOT_FOUND, ERROR_OPERATION_ABORTED, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, ERROR_PIPE_NOT_CONNECTED,
        GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE, LocalFree, WAIT_OBJECT_0, WAIT_TIMEOUT,
    };
    use windows_sys::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo, SDDL_REVISION_1,
        SE_KERNEL_OBJECT,
    };
    use windows_sys::Win32::Security::Cryptography::{BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom};
    use windows_sys::Win32::Security::{
        EqualSid, GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, OWNER_SECURITY_INFORMATION,
        PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES, TOKEN_INFORMATION_CLASS, TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
        TOKEN_USER, TokenIntegrityLevel, TokenUser,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
        READ_CONTROL, ReadFile, SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT, WriteFile,
    };
    use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
    use windows_sys::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeServerProcessId, PIPE_READMODE_BYTE,
        PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT, WaitNamedPipeW,
    };
    use windows_sys::Win32::System::Threading::{
        CreateEventW, GetCurrentProcess, INFINITE, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
        SetEvent, WaitForMultipleObjects,
    };
    use windows_sys::core::BOOL;

    /// Per direction; a state line is under 100 bytes.
    const BUFFER: u32 = 4096;
    /// How long `connect` waits while every instance is busy (the daemon is between accepts, or exiting).
    const BUSY_WAIT: Duration = Duration::from_secs(2);
    /// How long `Closer::close` waits for a blocked `accept` to let go of the listening instance.
    const CLOSE_WAIT: Duration = Duration::from_secs(1);
    /// `write_timeout` value meaning none.
    const NO_TIMEOUT: u32 = u32::MAX;

    /// An owned kernel handle, closed on drop.
    struct Handle(HANDLE);

    // SAFETY: a kernel handle is a process-wide value, usable from any thread.
    unsafe impl Send for Handle {}
    // SAFETY: as above; every operation on it here is thread-safe in the kernel.
    unsafe impl Sync for Handle {}

    impl Handle {
        fn new(raw: HANDLE) -> io::Result<Handle> {
            if raw.is_null() || raw == INVALID_HANDLE_VALUE { Err(io::Error::last_os_error()) } else { Ok(Handle(raw)) }
        }
    }

    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: we own the handle and close it exactly once.
            unsafe { CloseHandle(self.0) };
        }
    }

    fn wide(s: &OsStr) -> Vec<u16> {
        s.encode_wide().chain([0]).collect()
    }

    fn is(e: &io::Error, code: u32) -> bool {
        e.raw_os_error() == Some(code as i32)
    }

    /// The peer has closed or disconnected: EOF for a read.
    fn gone(e: &io::Error) -> bool {
        [ERROR_BROKEN_PIPE, ERROR_PIPE_NOT_CONNECTED, ERROR_NO_DATA].iter().any(|&c| is(e, c))
    }

    fn check(ok: BOOL) -> io::Result<()> {
        if ok == 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
    }

    /// A manual-reset event, initially unsignalled.
    fn event() -> io::Result<Handle> {
        // SAFETY: no attributes and no name; the result is checked.
        Handle::new(unsafe { CreateEventW(null(), 1, 0, null()) })
    }

    /// Issues one overlapped operation on `file` with `start` and waits for it: without limit, up to `timeout`, or
    /// until `stop` is signalled. An operation cut short is cancelled and fails with `TimedOut` or `Interrupted`.
    /// Returns the bytes transferred.
    fn overlapped(
        file: &Handle,
        timeout: Option<Duration>,
        stop: Option<&Handle>,
        start: impl FnOnce(*mut OVERLAPPED) -> BOOL,
    ) -> io::Result<u32> {
        let done = event()?;
        // SAFETY: all-zero is a valid OVERLAPPED.
        let mut ov: OVERLAPPED = unsafe { zeroed() };
        ov.hEvent = done.0;
        if start(&mut ov) == 0 {
            let err = io::Error::last_os_error();
            if !is(&err, ERROR_IO_PENDING) {
                return Err(err);
            }
            let ms = timeout.map_or(INFINITE, |t| t.as_millis().clamp(1, u128::from(INFINITE - 1)) as u32);
            let waits = [done.0, stop.map_or(null_mut(), |s| s.0)];
            // SAFETY: the first `count` handles are live events.
            let woke = unsafe { WaitForMultipleObjects(if stop.is_some() { 2 } else { 1 }, waits.as_ptr(), 0, ms) };
            if woke != WAIT_OBJECT_0 {
                let cut = match woke {
                    WAIT_TIMEOUT => io::ErrorKind::TimedOut.into(),
                    w if w == WAIT_OBJECT_0 + 1 => io::ErrorKind::Interrupted.into(),
                    _ => io::Error::last_os_error(),
                };
                // `ov` must outlive the operation: cancel it, then wait for it below.
                // SAFETY: `ov` is this operation's, and still live.
                unsafe { CancelIoEx(file.0, &ov) };
                let mut n = 0;
                // SAFETY: as above; waits for the cancellation (or a completion that beat it).
                return match check(unsafe { GetOverlappedResult(file.0, &ov, &mut n, 1) }) {
                    Ok(()) => Ok(n),
                    Err(e) if is(&e, ERROR_OPERATION_ABORTED) => Err(cut),
                    Err(e) => Err(e),
                };
            }
        }
        let mut n = 0;
        // SAFETY: `ov` belongs to an operation on `file` that has completed or will; this waits for it.
        check(unsafe { GetOverlappedResult(file.0, &ov, &mut n, 1) })?;
        Ok(n)
    }

    /// The access token of the process `process` (a process handle).
    fn token(process: HANDLE) -> io::Result<Handle> {
        let mut raw = null_mut();
        // SAFETY: a live process handle; `raw` receives a new handle, checked.
        check(unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut raw) })?;
        Handle::new(raw)
    }

    /// One `GetTokenInformation` class, in a u64 buffer so the structure it holds is aligned.
    fn token_info(token: &Handle, class: TOKEN_INFORMATION_CLASS) -> io::Result<Vec<u64>> {
        let mut len = 0;
        // SAFETY: a size query: no buffer, fails with ERROR_INSUFFICIENT_BUFFER and sets `len`.
        unsafe { GetTokenInformation(token.0, class, null_mut(), 0, &mut len) };
        let mut buf = vec![0u64; (len as usize).div_ceil(8).max(1)];
        let size = (buf.len() * 8) as u32;
        // SAFETY: `buf` has `size` writable bytes.
        check(unsafe { GetTokenInformation(token.0, class, buf.as_mut_ptr().cast(), size, &mut len) })?;
        Ok(buf)
    }

    /// The token's mandatory integrity level, as the label SID's last RID (low 0x1000, medium 0x2000, high 0x3000).
    fn integrity(token: &Handle) -> io::Result<u32> {
        let buf = token_info(token, TokenIntegrityLevel)?;
        // SAFETY: the buffer holds a TOKEN_MANDATORY_LABEL whose SID points into it; an integrity SID has one or more
        // sub-authorities, and the last is the level.
        unsafe {
            let sid = (*buf.as_ptr().cast::<TOKEN_MANDATORY_LABEL>()).Label.Sid;
            let count = *GetSidSubAuthorityCount(sid);
            if count == 0 {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "integrity label without a level"));
            }
            Ok(*GetSidSubAuthority(sid, u32::from(count) - 1))
        }
    }

    /// Whether the process serving `pipe` runs at an integrity level at least this process's. A lower one is not the
    /// daemon: a sandboxed process of the same user that took the name after the daemon died.
    fn server_trusted(pipe: &Handle) -> io::Result<bool> {
        let mut pid = 0;
        // SAFETY: a client end of a pipe.
        check(unsafe { GetNamedPipeServerProcessId(pipe.0, &mut pid) })?;
        // SAFETY: the result is checked; limited query rights are enough for the token.
        let process = Handle::new(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) })?;
        // SAFETY: the current-process pseudo-handle needs no closing.
        let ours = integrity(&token(unsafe { GetCurrentProcess() })?)?;
        Ok(integrity(&token(process.0)?)? >= ours)
    }

    /// The current user's SID, inside the `TOKEN_USER` that holds it.
    struct UserSid(Vec<u64>);

    impl UserSid {
        fn current() -> io::Result<UserSid> {
            // SAFETY: the current-process pseudo-handle needs no closing.
            token_info(&token(unsafe { GetCurrentProcess() })?, TokenUser).map(UserSid)
        }

        fn psid(&self) -> PSID {
            // SAFETY: the buffer holds a TOKEN_USER whose SID points into the same (never moved) heap buffer.
            unsafe { (*self.0.as_ptr().cast::<TOKEN_USER>()).User.Sid }
        }

        fn string(&self) -> io::Result<String> {
            let mut s = null_mut();
            // SAFETY: a valid SID; `s` receives a LocalAlloc'd, NUL-terminated string, freed below.
            check(unsafe { ConvertSidToStringSidW(self.psid(), &mut s) })?;
            // SAFETY: NUL-terminated as above.
            let len = (0..).take_while(|&i| unsafe { *s.add(i) } != 0).count();
            // SAFETY: `len` u16s are initialised.
            let out = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(s, len) });
            // SAFETY: allocated by ConvertSidToStringSidW with LocalAlloc.
            unsafe { LocalFree(s.cast()) };
            Ok(out)
        }

        /// Whether `pipe`'s owner is this user.
        fn owns(&self, pipe: &Handle) -> io::Result<bool> {
            let mut owner: PSID = null_mut();
            let mut sd: PSECURITY_DESCRIPTOR = null_mut();
            // SAFETY: `pipe` was opened with READ_CONTROL; `owner` points into `sd`, freed below.
            let err = unsafe {
                GetSecurityInfo(
                    pipe.0,
                    SE_KERNEL_OBJECT,
                    OWNER_SECURITY_INFORMATION,
                    &mut owner,
                    null_mut(),
                    null_mut(),
                    null_mut(),
                    &mut sd,
                )
            };
            if err != 0 {
                return Err(io::Error::from_raw_os_error(err as i32));
            }
            // SAFETY: both SIDs are valid.
            let same = !owner.is_null() && unsafe { EqualSid(owner, self.psid()) } != 0;
            // SAFETY: allocated by GetSecurityInfo with LocalAlloc.
            unsafe { LocalFree(sd) };
            Ok(same)
        }
    }

    /// A self-relative security descriptor from LocalAlloc.
    struct SecurityDescriptor(PSECURITY_DESCRIPTOR);

    // SAFETY: an immutable heap block, only read after creation.
    unsafe impl Send for SecurityDescriptor {}
    // SAFETY: as above.
    unsafe impl Sync for SecurityDescriptor {}

    impl SecurityDescriptor {
        /// Owned by `sid`, with a protected DACL granting `sid` full access and no one else anything.
        fn owner_only(sid: &str) -> io::Result<SecurityDescriptor> {
            let sddl = wide(OsStr::new(&format!("O:{sid}D:P(A;;GA;;;{sid})")));
            let mut sd = null_mut();
            // SAFETY: NUL-terminated SDDL; `sd` receives a LocalAlloc'd descriptor, freed on drop.
            check(unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    sddl.as_ptr(),
                    SDDL_REVISION_1,
                    &mut sd,
                    null_mut(),
                )
            })?;
            Ok(SecurityDescriptor(sd))
        }
    }

    impl Drop for SecurityDescriptor {
        fn drop(&mut self) {
            // SAFETY: allocated with LocalAlloc, freed once.
            unsafe { LocalFree(self.0) };
        }
    }

    const PIPE_PREFIX: &str = r"\\.\pipe\noiseman-";

    /// `noiseman.pipe` in the runtime dir: holds the running daemon's pipe name.
    pub struct Endpoint(PathBuf);

    impl Endpoint {
        pub fn in_dir(dir: &Path) -> io::Result<Endpoint> {
            Ok(Endpoint(dir.join("noiseman.pipe")))
        }

        /// The published pipe name; `NotFound` when no daemon has published one, or the file is damaged (say, empty
        /// after a power loss): either way there is no daemon to reach, and the next one rewrites it.
        fn read(&self) -> io::Result<String> {
            let name = fs::read_to_string(&self.0)?;
            let name = name.trim();
            let random = name.strip_prefix(PIPE_PREFIX).unwrap_or_default();
            if random.len() != 32 || !random.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(io::Error::new(io::ErrorKind::NotFound, format!("{self} holds no pipe name")));
            }
            Ok(name.to_owned())
        }

        /// Writes the name whole and durable: to a temporary file, flushed, then renamed over the old one.
        fn publish(&self, name: &str) -> io::Result<()> {
            let tmp = self.0.with_extension("pipe.tmp");
            let mut file = fs::File::create(&tmp)?;
            file.write_all(name.as_bytes())?;
            file.sync_all()?;
            drop(file);
            fs::rename(&tmp, &self.0)
        }
    }

    impl fmt::Display for Endpoint {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            self.0.display().fmt(f)
        }
    }

    /// `\\.\pipe\noiseman-` and 128 bits from the system RNG.
    fn random_pipe_name() -> io::Result<String> {
        let mut bytes = [0u8; 16];
        // SAFETY: a 16-byte buffer; no algorithm handle with the system-preferred RNG flag.
        let status = unsafe {
            BCryptGenRandom(null_mut(), bytes.as_mut_ptr(), bytes.len() as u32, BCRYPT_USE_SYSTEM_PREFERRED_RNG)
        };
        if status != 0 {
            return Err(io::Error::other(format!("system RNG failed (NTSTATUS {status:#x})")));
        }
        Ok(bytes.iter().fold(PIPE_PREFIX.to_owned(), |s, b| s + &format!("{b:02x}")))
    }

    /// One end of a connection.
    struct Pipe {
        handle: Handle,
        /// The daemon's end: `shutdown` disconnects the client.
        server: bool,
        closed: AtomicBool,
        /// Milliseconds, or `NO_TIMEOUT`.
        write_timeout: AtomicU32,
    }

    impl Pipe {
        fn new(handle: Handle, server: bool) -> Pipe {
            Pipe { handle, server, closed: AtomicBool::new(false), write_timeout: AtomicU32::new(NO_TIMEOUT) }
        }
    }

    struct Shared {
        /// NUL-terminated.
        name: Vec<u16>,
        shown: String,
        /// Where the name is published; removed on close.
        published: PathBuf,
        sd: SecurityDescriptor,
        /// Signalled by `Closer::close`; wakes a blocked `accept`.
        stop: Handle,
        /// The listening instance, and whether the listener is closed (set under the same lock).
        slot: Mutex<Slot>,
    }

    struct Slot {
        listening: Option<Arc<Pipe>>,
        closed: bool,
    }

    impl Shared {
        fn slot(&self) -> MutexGuard<'_, Slot> {
            self.slot.lock().unwrap_or_else(PoisonError::into_inner)
        }

        fn instance(&self, first: bool) -> io::Result<Arc<Pipe>> {
            let sa = SECURITY_ATTRIBUTES {
                nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: self.sd.0,
                bInheritHandle: 0,
            };
            let open =
                PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED | if first { FILE_FLAG_FIRST_PIPE_INSTANCE } else { 0 };
            let mode = PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS;
            // SAFETY: NUL-terminated name, a valid descriptor; the result is checked.
            let raw = unsafe {
                CreateNamedPipeW(self.name.as_ptr(), open, mode, PIPE_UNLIMITED_INSTANCES, BUFFER, BUFFER, 0, &sa)
            };
            match Handle::new(raw) {
                Ok(h) => Ok(Arc::new(Pipe::new(h, true))),
                Err(e) if first && is(&e, ERROR_ACCESS_DENIED) => Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    format!("{} is already in use by another process", self.shown),
                )),
                Err(e) => Err(e),
            }
        }
    }

    pub struct Listener(Arc<Shared>);

    /// Stops new connections: the listening instance is closed before `close` returns.
    pub struct Closer(Arc<Shared>);

    impl Listener {
        /// Creates a pipe under a fresh random name, then publishes the name, replacing a stale one (the pipe it named
        /// vanished with its process).
        pub fn bind(ep: &Endpoint) -> io::Result<Listener> {
            let sid = UserSid::current()?.string()?;
            let shown = random_pipe_name()?;
            let shared = Shared {
                name: wide(OsStr::new(&shown)),
                shown: shown.clone(),
                published: ep.0.clone(),
                sd: SecurityDescriptor::owner_only(&sid)?,
                stop: event()?,
                slot: Mutex::new(Slot { listening: None, closed: false }),
            };
            let first = shared.instance(true)?;
            shared.slot().listening = Some(first);
            ep.publish(&shown).map_err(|e| io::Error::new(e.kind(), format!("cannot write {ep}: {e}")))?;
            Ok(Listener(Arc::new(shared)))
        }

        pub fn accept(&self) -> io::Result<Option<Stream>> {
            let shared = &*self.0;
            loop {
                let pipe = {
                    let mut slot = shared.slot();
                    if slot.closed {
                        return Ok(None);
                    }
                    match &slot.listening {
                        Some(p) => p.clone(),
                        None => slot.listening.insert(shared.instance(false)?).clone(),
                    }
                };
                let h = pipe.handle.0;
                // SAFETY: a listening instance; `ov` is supplied and kept alive by `overlapped`.
                match overlapped(&pipe.handle, None, Some(&shared.stop), |ov| unsafe { ConnectNamedPipe(h, ov) }) {
                    Ok(_) => {}
                    // A client connected between creating the instance and this call.
                    Err(e) if is(&e, ERROR_PIPE_CONNECTED) => {}
                    // It connected and left already: reuse the instance.
                    Err(e) if is(&e, ERROR_NO_DATA) => {
                        // SAFETY: our instance.
                        unsafe { DisconnectNamedPipe(h) };
                        continue;
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => return Ok(None),
                    Err(e) => return Err(e),
                }
                let mut slot = shared.slot();
                // A client that won the race with `close` (its connect completed as `stop` was set) is turned away:
                // after `close`, `accept` hands out nothing, and `close` is not left waiting on this instance.
                if slot.closed {
                    // SAFETY: our instance.
                    unsafe { DisconnectNamedPipe(h) };
                    return Ok(None);
                }
                // Hand this instance out and keep the name alive with a new listening one; if that fails, the next
                // `accept` tries again.
                slot.listening = shared.instance(false).ok();
                return Ok(Some(Stream(pipe)));
            }
        }

        pub fn closer(&self) -> io::Result<Closer> {
            Ok(Closer(self.0.clone()))
        }
    }

    impl Closer {
        pub fn close(self) -> io::Result<()> {
            // Unpublished first: new clients find no name and report NotFound.
            let unpublished = match fs::remove_file(&self.0.published) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            };
            let listening = {
                let mut slot = self.0.slot();
                slot.closed = true;
                slot.listening.take()
            };
            // SAFETY: our event.
            check(unsafe { SetEvent(self.0.stop.0) })?;
            // A blocked `accept` holds a clone until it sees `stop`; the instance closes when the last one drops.
            if let Some(pipe) = listening {
                let until = Instant::now() + CLOSE_WAIT;
                while Arc::strong_count(&pipe) > 1 && Instant::now() < until {
                    thread::sleep(Duration::from_millis(1));
                }
            }
            unpublished
        }
    }

    pub struct Stream(Arc<Pipe>);

    impl Stream {
        pub fn connect(ep: &Endpoint) -> io::Result<Stream> {
            let shown = ep.read()?;
            let name = wide(OsStr::new(&shown));
            let user = UserSid::current()?;
            let until = Instant::now() + BUSY_WAIT;
            loop {
                // SAFETY: NUL-terminated name; SQOS lets the daemon identify us but not impersonate us.
                let raw = unsafe {
                    CreateFileW(
                        name.as_ptr(),
                        GENERIC_READ | GENERIC_WRITE | READ_CONTROL,
                        0,
                        null(),
                        OPEN_EXISTING,
                        FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                        null_mut(),
                    )
                };
                // A pipe under the published name that is not our daemon means the daemon died without unpublishing
                // and someone took the name: another user (access denied, or another owner) or a sandboxed process of
                // ours (lower integrity). Report it as no daemon, never talk to it: autostart then starts the real one,
                // which publishes a fresh name.
                let impostor = |why: &str| {
                    io::Error::new(io::ErrorKind::NotFound, format!("{shown} (named in {ep}) is not the daemon: {why}"))
                };
                let err = match Handle::new(raw) {
                    Ok(handle) => {
                        if !user.owns(&handle)? {
                            return Err(impostor("owned by another user"));
                        }
                        if !server_trusted(&handle)? {
                            return Err(impostor("served at a lower integrity level"));
                        }
                        return Ok(Stream(Arc::new(Pipe::new(handle, false))));
                    }
                    Err(e) if is(&e, ERROR_ACCESS_DENIED) => return Err(impostor("access denied")),
                    Err(e) => e,
                };
                if !is(&err, ERROR_PIPE_BUSY) {
                    return Err(err);
                }
                // The daemon is busy only between accepts; a name busy for this long is not a working daemon (say, an
                // impostor holding its one instance), so it counts as none and autostart replaces it.
                let left = until.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return Err(impostor("every instance stayed busy"));
                }
                // Every instance is connected: the daemon is between accepts, or closed and about to exit.
                // SAFETY: NUL-terminated name.
                if unsafe { WaitNamedPipeW(name.as_ptr(), left.as_millis().clamp(1, 60_000) as u32) } == 0 {
                    let e = io::Error::last_os_error();
                    if is(&e, ERROR_FILE_NOT_FOUND) {
                        return Err(e);
                    }
                }
            }
        }

        pub fn try_clone(&self) -> io::Result<Stream> {
            Ok(Stream(self.0.clone()))
        }

        pub fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
            let ms = match timeout {
                None => NO_TIMEOUT,
                Some(t) if t.is_zero() => {
                    return Err(io::Error::new(io::ErrorKind::InvalidInput, "cannot set a 0 duration timeout"));
                }
                Some(t) => t.as_millis().clamp(1, u128::from(NO_TIMEOUT - 1)) as u32,
            };
            self.0.write_timeout.store(ms, Ordering::Relaxed);
            Ok(())
        }

        /// On the daemon's end, disconnects the client (it reads EOF). On either end, reads blocked on this
        /// connection return EOF and later reads and writes fail fast. A client's daemon sees EOF only once every
        /// clone of the client's stream is dropped.
        pub fn shutdown(&self) -> io::Result<()> {
            let p = &*self.0;
            p.closed.store(true, Ordering::Release);
            // SAFETY: our handle.
            if p.server
                && let Err(e) = check(unsafe { DisconnectNamedPipe(p.handle.0) })
                && !gone(&e)
            {
                return Err(e);
            }
            // SAFETY: our handle; cancels every pending operation on it, from any thread.
            match check(unsafe { CancelIoEx(p.handle.0, null()) }) {
                Err(e) if !is(&e, ERROR_NOT_FOUND) => Err(e),
                _ => Ok(()),
            }
        }
    }

    impl Read for &Stream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let p = &*self.0;
            if p.closed.load(Ordering::Acquire) {
                return Ok(0);
            }
            let (h, ptr, len) = (p.handle.0, buf.as_mut_ptr(), buf.len().min(u32::MAX as usize) as u32);
            // SAFETY: `buf` outlives the operation, which `overlapped` waits for.
            match overlapped(&p.handle, None, None, |ov| unsafe { ReadFile(h, ptr, len, null_mut(), ov) }) {
                Ok(n) => Ok(n as usize),
                Err(e) if gone(&e) => Ok(0),
                Err(e) if is(&e, ERROR_OPERATION_ABORTED) && p.closed.load(Ordering::Acquire) => Ok(0),
                Err(e) => Err(e),
            }
        }
    }

    impl Write for &Stream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let p = &*self.0;
            if p.closed.load(Ordering::Acquire) {
                return Err(io::ErrorKind::BrokenPipe.into());
            }
            let timeout = match p.write_timeout.load(Ordering::Relaxed) {
                NO_TIMEOUT => None,
                ms => Some(Duration::from_millis(u64::from(ms))),
            };
            let (h, ptr, len) = (p.handle.0, buf.as_ptr(), buf.len().min(u32::MAX as usize) as u32);
            // SAFETY: `buf` outlives the operation, which `overlapped` waits for.
            overlapped(&p.handle, timeout, None, |ov| unsafe { WriteFile(h, ptr, len, null_mut(), ov) })
                .map(|n| n as usize)
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
