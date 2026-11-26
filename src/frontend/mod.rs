//! Everything a person touches. Talks to the backend only through `protocol` over `client`.

mod cli;
mod client;
#[cfg(feature = "gui")]
mod gui;
mod tui;

use std::process::ExitCode;

use crate::protocol::Kind;

const GREEN: [u8; 3] = [0x4c, 0xae, 0x5a];
const RED: [u8; 3] = [0xd9, 0x4f, 0x45];

/// A kind's accent, a muted track and a dark logo tile. Accents are mid-tones so they read on dark and light
/// backgrounds alike.
fn colors(kind: Kind) -> [[u8; 3]; 3] {
    let accent = match kind {
        Kind::White => [0x8c, 0x96, 0xa8],
        Kind::Pink => [0xd4, 0x70, 0x8f],
        Kind::Brown => [0xd0, 0x8f, 0x4a],
        Kind::Blue => [0x4f, 0x8f, 0xd6],
        Kind::Violet => [0x9b, 0x74, 0xd6],
    };
    let track = accent.map(|v| ((v as u16 * 35 + 0x8a * 65) / 100) as u8);
    let tile = accent.map(|v| (v as u16 * 22 / 100) as u8);
    [accent, track, tile]
}

pub fn run(args: &[String]) -> ExitCode {
    match args.first().map(String::as_str) {
        None | Some("tui") if args.len() <= 1 => tui::run(),
        #[cfg(feature = "gui")]
        Some("gui") if args.len() == 1 => gui::run(),
        #[cfg(not(feature = "gui"))]
        Some("gui") if args.len() == 1 => {
            eprintln!(
                "noiseman: this build has no window; the gui build has one. To get it:
  - run the installer with NOISEMAN_GUI=1 (install.sh or install.ps1)
  - or download noiseman-gui-<target> from https://github.com/roudy16/noiseman/releases
  - or rebuild with `--features gui`"
            );
            ExitCode::FAILURE
        }
        Some("version" | "-V" | "--version") => {
            println!("noiseman {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some("help" | "-h" | "--help") => {
            println!("{}", cli::USAGE);
            ExitCode::SUCCESS
        }
        _ => cli::run(args),
    }
}
