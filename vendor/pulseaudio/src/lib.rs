// Modified by noiseman (2026-11-25) from pulseaudio 0.3.2, under the MIT License (see ../LICENSE.md).
// Change, marked "noiseman patch": socket_path_from_env finds the server where libpulse does (upstream issue #24), so
// noiseman connects wherever other PulseAudio clients do: $PULSE_SERVER alone when set, else the first of
// $PULSE_RUNTIME_PATH/native and $XDG_RUNTIME_DIR/pulse/native that exists. Upstream returned None at the first
// missing location and looked for $PULSE_RUNTIME_PATH/pulse/native. The search moved to socket_path_from so
// socket_path_test can drive it without touching the environment.

//! A pure Rust implementation of the PulseAudio protocol, suitable for writing
//! servers and clients.

#![warn(
    anonymous_parameters,
    missing_copy_implementations,
    missing_debug_implementations,
    missing_docs,
    nonstandard_style,
    rust_2018_idioms,
    single_use_lifetimes,
    trivial_casts,
    trivial_numeric_casts,
    unreachable_pub,
    unused_extern_crates,
    unused_qualifications,
    variant_size_differences
)]

use std::path::PathBuf;

mod client;
pub mod protocol;
pub use client::*;

// noiseman patch: doc comment and body rewritten to search as libpulse does, in socket_path_from.
/// Attempts to determine the socket path from the runtime environment, as
/// libpulse does. If $PULSE_SERVER is set, it is the only server: a 'unix:'
/// path (or a bare absolute path) is used if it exists. Otherwise the first
/// of these that exists is used:
///   - $PULSE_RUNTIME_PATH/native
///   - $XDG_RUNTIME_DIR/pulse/native
///
/// Returns None if no socket can be found or if $PULSE_SERVER points to a
/// remote server, i.e. starts with a prefix other than 'unix:'.
pub fn socket_path_from_env() -> Option<PathBuf> {
    let var = |name| std::env::var(name).ok().filter(|v: &String| !v.is_empty());
    socket_path_from(
        var("PULSE_SERVER"),
        var("PULSE_RUNTIME_PATH"),
        var("XDG_RUNTIME_DIR"),
    )
}

fn socket_path_from(
    server: Option<String>,
    runtime_path: Option<String>,
    xdg_runtime_dir: Option<String>,
) -> Option<PathBuf> {
    let usable =
        |path: &PathBuf| std::fs::metadata(path).is_ok_and(|m| !m.permissions().readonly());

    if let Some(server) = server {
        let path = server.strip_prefix("unix:").unwrap_or(&server);
        let path = PathBuf::from(path);
        return (path.is_absolute() && usable(&path)).then_some(path);
    }

    [
        runtime_path.map(|p| PathBuf::from(p).join("native")),
        xdg_runtime_dir.map(|p| PathBuf::from(p).join("pulse/native")),
    ]
    .into_iter()
    .flatten()
    .find(usable)
}

/// Attempts to find the authentication cookie from the environment, checking
/// the following locations in order:
///
///   - $PULSE_COOKIE
///   - $HOME/.config/pulse/cookie
///   - $HOME/.pulse-cookie
pub fn cookie_path_from_env() -> Option<PathBuf> {
    #[allow(deprecated)]
    let home = std::env::home_dir()?;

    let mut paths = std::env::var("PULSE_COOKIE")
        .ok()
        .map(PathBuf::from)
        .into_iter()
        .chain(std::iter::once(home.join(".config/pulse/cookie")))
        .chain(std::iter::once(home.join(".pulse-cookie")));

    paths.find(|path| path.exists())
}

// noiseman patch: tests socket_path_from.
#[cfg(test)]
mod socket_path_test {
    use super::socket_path_from as find;

    fn s(path: &std::path::Path) -> Option<String> {
        Some(path.to_str().unwrap().to_owned())
    }

    #[test]
    fn socket_path_from_tries_each_location() {
        let dir = std::env::temp_dir().join(format!("pa-socket-test-{}", std::process::id()));
        let (runtime, xdg, missing) = (dir.join("runtime"), dir.join("xdg"), dir.join("missing"));
        let (runtime_sock, xdg_sock) = (runtime.join("native"), xdg.join("pulse/native"));
        std::fs::create_dir_all(&runtime).unwrap();
        std::fs::create_dir_all(xdg.join("pulse")).unwrap();
        std::fs::write(&xdg_sock, "").unwrap();

        assert_eq!(find(None, s(&runtime), s(&xdg)), Some(xdg_sock.clone()));
        assert_eq!(find(None, s(&missing), s(&xdg)), Some(xdg_sock.clone()));
        std::fs::write(&runtime_sock, "").unwrap();
        assert_eq!(find(None, s(&runtime), s(&xdg)), Some(runtime_sock));

        let unix = |p: &std::path::Path| Some(format!("unix:{}", p.display()));
        let tcp = Some("tcp:localhost".to_owned());
        assert_eq!(find(unix(&xdg_sock), None, None), Some(xdg_sock.clone()));
        assert_eq!(find(s(&xdg_sock), s(&runtime), None), Some(xdg_sock));
        assert_eq!(find(unix(&missing), s(&runtime), s(&xdg)), None);
        assert_eq!(find(tcp, s(&runtime), s(&xdg)), None);

        std::fs::remove_dir_all(&dir).unwrap();
    }
}

#[cfg(test)]
#[cfg(feature = "_integration-tests")]
mod integration_test_util {
    use std::{ffi::CString, io::BufReader, os::unix::net::UnixStream};

    use anyhow::Context;

    use super::*;
    use crate::protocol::*;

    #[test]
    fn socket_path() -> anyhow::Result<()> {
        let path = socket_path_from_env();
        assert!(path.is_some());
        assert!(path.unwrap().exists());

        Ok(())
    }

    pub(crate) fn connect_to_server() -> anyhow::Result<BufReader<UnixStream>> {
        let socket_path = socket_path_from_env().context("error finding pulse socket")?;
        let sock = UnixStream::connect(socket_path).context("error connecting to pulse socket")?;

        Ok(BufReader::new(sock))
    }

    pub(crate) fn init_client(mut sock: &mut BufReader<UnixStream>) -> anyhow::Result<u16> {
        let cookie = cookie_path_from_env()
            .and_then(|path| std::fs::read(path).ok())
            .unwrap_or_default();
        if cookie.is_empty() {
            eprintln!("warning: no pulseaudio cookie found");
        }

        let auth = AuthParams {
            version: MAX_VERSION,
            supports_shm: false,
            supports_memfd: false,
            cookie,
        };

        write_command_message(sock.get_mut(), 0, &Command::Auth(auth), MAX_VERSION)
            .context("sending auth command failed")?;
        let (_, auth_reply) =
            read_reply_message::<AuthReply>(sock, MAX_VERSION).context("auth command failed")?;

        let protocol_version = std::cmp::min(MAX_VERSION, auth_reply.version);

        let mut props = Props::new();
        props.set(Prop::ApplicationName, CString::new("pulseaudio-rs-tests")?);
        write_command_message(
            sock.get_mut(),
            1,
            &Command::SetClientName(props),
            protocol_version,
        )
        .context("sending set_client_name command failed")?;
        let _ = read_reply_message::<SetClientNameReply>(&mut sock, protocol_version)
            .context("set_client_name command failed")?;

        Ok(protocol_version)
    }

    pub(crate) fn connect_and_init() -> anyhow::Result<(BufReader<UnixStream>, u16)> {
        let mut sock = connect_to_server()?;
        let protocol_version = init_client(&mut sock)?;

        Ok((sock, protocol_version))
    }
}
