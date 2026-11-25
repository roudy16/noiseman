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
            eprintln!("noiseman: this build has no window; rebuild with `--features gui`");
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
