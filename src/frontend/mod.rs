//! Everything a person touches. Talks to the backend only through `protocol` over `client`.

mod cli;
mod client;
#[cfg(feature = "gui")]
mod gui;
mod tui;

use std::process::ExitCode;

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
