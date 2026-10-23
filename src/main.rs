//! Airlock — workspace-isolated Alt+Tab for the COSMIC desktop.

mod cli;
mod daemon;
mod ipc;
mod switcher;
mod ui;
mod wayland;

use std::process::Stdio;
use std::time::{Duration, Instant};

use cli::Command;
use ipc::Direction;

/// How long a stub waits for a (possibly auto-spawned) daemon to answer.
const STUB_DEADLINE: Duration = Duration::from_secs(3);

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
        Command::Daemon => {
            if let Err(error) = daemon::run() {
                eprintln!("airlock daemon: {error}");
                std::process::exit(1);
            }
        }
        Command::AltTab => cycle(Direction::Next),
        Command::ShiftAltTab => cycle(Direction::Previous),
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

/// Asks the daemon to cycle one window, spawning it first if needed. The
/// stub is silent on success: it usually runs from a keybind with nowhere
/// to print.
fn cycle(direction: Direction) {
    let path = match ipc::socket_path() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("airlock: {error}");
            std::process::exit(1);
        }
    };
    if let Err(error) = ipc::send_with_deadline(
        &path,
        direction,
        Instant::now() + STUB_DEADLINE,
        spawn_daemon_detached,
    ) {
        eprintln!("airlock: {error}");
        std::process::exit(1);
    }
}

/// Starts a detached daemon so a keypress recovers from a dead one. The
/// daemon gets its own process group (no terminal SIGHUP), no stdin, and
/// inherits stderr so startup diagnostics stay visible when run by hand.
fn spawn_daemon_detached() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    use std::os::unix::process::CommandExt as _;
    let _ = std::process::Command::new(exe)
        .arg("daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .process_group(0)
        .spawn();
}

/// Every stubbed subcommand exits non-zero so callers never mistake a stub
/// for success.
fn stub(name: &str) -> ! {
    eprintln!("airlock {name}: not implemented yet");
    std::process::exit(1);
}
