mod backend;
mod frontend;
mod protocol;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "daemon") {
        if let Err(e) = backend::serve() {
            eprintln!("noiseman: {e}");
            return ExitCode::FAILURE;
        }
        return ExitCode::SUCCESS;
    }
    frontend::run(&args)
}
