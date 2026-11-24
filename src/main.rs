mod backend;
mod frontend;
mod protocol;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
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
