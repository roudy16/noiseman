mod backend;
mod frontend;
mod platform;
mod protocol;

use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    // Run as `noiseman-gui` (on Windows, the release archive's copy linked without a console), it opens the window.
    let name = std::env::args_os().next().map(std::path::PathBuf::from);
    let stem = name.as_deref().and_then(|p| p.file_stem()).and_then(|s| s.to_str());
    // Any case: Windows file names are case-insensitive, and argv[0] is spelled as the caller typed it.
    if args.is_empty() && stem.is_some_and(|s| s.eq_ignore_ascii_case("noiseman-gui")) {
        args.push("gui".into());
    }
    if args.first().is_some_and(|a| a == "daemon") {
        let muted = match &args[1..] {
            [] => false,
            [flag] if flag == "--muted" => true,
            _ => {
                eprintln!("noiseman: usage: noiseman daemon [--muted]");
                return ExitCode::FAILURE;
            }
        };
        if let Err(e) = backend::serve(muted) {
            eprintln!("noiseman: {e}");
            return ExitCode::FAILURE;
        }
        return ExitCode::SUCCESS;
    }
    frontend::run(&args)
}
