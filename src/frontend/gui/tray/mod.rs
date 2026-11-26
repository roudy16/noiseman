//! The tray icon, one backend per OS, each with the same `Tray::sync` and `spawn`. The backends are the one place
//! outside `src/platform/` that names OS APIs.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::{Tray, spawn};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::{Tray, spawn};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::{Tray, spawn};

#[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
mod none {
    pub enum Tray {}

    pub fn spawn(_: winit::event_loop::EventLoopProxy<super::Event>, _: super::TrayState) {}

    impl Tray {
        pub fn sync(&mut self, _: super::TrayState) {
            match *self {}
        }
    }
}
#[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
pub use none::{Tray, spawn};

#[cfg(any(target_os = "linux", windows, target_os = "macos"))]
use super::send_or_log;
use super::{Event, TrayState};

/// The tooltip's line under the app name, e.g. "Brown noise, muted".
#[cfg(any(target_os = "linux", windows, target_os = "macos"))]
fn description(s: TrayState) -> String {
    format!("{} noise{}", s.kind.label(), if s.muted { ", muted" } else { "" })
}
