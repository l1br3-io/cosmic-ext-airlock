//! Airlock — workspace-isolated Alt+Tab for the COSMIC desktop.

mod cli;
mod daemon;
mod ipc;
mod switcher;
mod wayland;

use cli::Command;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = match cli::parse(&args) {
        Ok(command) => command,
        Err(error) => {
            if let cli::ParseError::Unknown(token) | cli::ParseError::Unexpected(token) = &error {
                eprintln!("airlock: unrecognized argument: {token}");
            }
            eprint!("{}", cli::usage());
            std::process::exit(2);
        }
    };

    match command {
        Command::Daemon => stub("daemon"),
        Command::AltTab => stub("alt-tab"),
        Command::ShiftAltTab => stub("shift-alt-tab"),
        Command::Install { .. } => stub("install"),
        Command::List => match wayland::list() {
            Ok(text) => {
                print!("{text}");
            }
            Err(error) => {
                eprintln!("airlock list: {error}");
                std::process::exit(1);
            }
        },
    }
}

/// Every subcommand is a stub until its task lands. Exit non-zero so callers
/// never mistake a stub for success.
fn stub(name: &str) -> ! {
    eprintln!("airlock {name}: not implemented yet");
    std::process::exit(1);
}
