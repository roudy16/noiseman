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
        Some("help" | "-h" | "--help") => {
            println!("{}", cli::USAGE);
            ExitCode::SUCCESS
        }
        _ => cli::run(args),
    }
}
