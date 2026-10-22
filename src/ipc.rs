//! Unix-socket IPC between the spawn stubs and the daemon.
//!
//! Wire protocol: the stub connects, writes one command byte (`n` or `p`),
//! and blocks for one ack byte (`a`). One byte each way is the whole frame —
//! anything longer is a protocol error and the connection is dropped.

use std::io::{self, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

/// Socket file name inside the runtime directory.
pub const SOCKET_NAME: &str = "airlock.sock";

/// Ack byte the daemon writes after applying a command.
pub const ACK: u8 = b'a';

/// Pause between connect retries while waiting for the daemon to come up.
const RETRY_STEP: Duration = Duration::from_millis(25);

/// Which way the stub asks the daemon to cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Cycle to the next window in MRU order.
    Next,
    /// Cycle to the previous window in MRU order.
    Previous,
}

impl Direction {
    /// The wire byte for this direction.
    pub fn wire_byte(self) -> u8 {
        match self {
            Direction::Next => b'n',
            Direction::Previous => b'p',
        }
    }
}

/// Why IPC failed.
#[derive(Debug)]
pub enum IpcError {
    /// `XDG_RUNTIME_DIR` is not set; there is nowhere to put the socket.
    MissingRuntimeDir,
    /// Socket I/O failed.
    Io(io::Error),
    /// The deadline passed before the daemon acked.
    TimedOut,
}

impl std::fmt::Display for IpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IpcError::MissingRuntimeDir => {
                write!(f, "XDG_RUNTIME_DIR is not set; cannot reach the daemon")
            }
            IpcError::Io(err) => write!(f, "daemon socket: {err}"),
            IpcError::TimedOut => write!(
                f,
                "daemon did not answer in time; is a COSMIC session running?"
            ),
        }
    }
}

impl std::error::Error for IpcError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            IpcError::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<io::Error> for IpcError {
    fn from(err: io::Error) -> Self {
        IpcError::Io(err)
    }
}

/// Decodes a command byte. Junk is not a command.
pub fn parse_command(byte: u8) -> Option<Direction> {
    match byte {
        b'n' => Some(Direction::Next),
        b'p' => Some(Direction::Previous),
        _ => None,
    }
}

/// Resolves the socket path from an explicit runtime directory value.
pub fn socket_path_from(runtime_dir: Option<&str>) -> Option<PathBuf> {
    runtime_dir.map(|dir| Path::new(dir).join(SOCKET_NAME))
}

/// Resolves the socket path from `XDG_RUNTIME_DIR`.
///
/// # Errors
///
/// Returns [`IpcError::MissingRuntimeDir`] when the variable is unset.
pub fn socket_path() -> Result<PathBuf, IpcError> {
    socket_path_from(std::env::var("XDG_RUNTIME_DIR").ok().as_deref())
        .ok_or(IpcError::MissingRuntimeDir)
}

/// Outcome of trying to own the daemon socket.
#[derive(Debug)]
pub enum BindOutcome {
    /// No live daemon existed; this caller now owns the socket.
    Bound(UnixListener),
    /// A live daemon answered the probe; the caller should exit quietly.
    AlreadyRunning,
}

/// Whether a live daemon is answering on `path`.
pub fn probe_alive(path: &Path) -> bool {
    UnixStream::connect(path).is_ok()
}

/// Binds the daemon socket, detecting live daemons and clearing stale ones.
///
/// A bind failure is only meaningful for `AddrInUse`: any other path state
/// (stale socket file from a killed daemon, leftover regular file) is probed
/// and, if dead, unlinked and retried once.
///
/// # Errors
///
/// Propagates I/O errors from the bind attempts.
pub fn bind_exclusive(path: &Path) -> io::Result<BindOutcome> {
    match UnixListener::bind(path) {
        Ok(listener) => Ok(BindOutcome::Bound(listener)),
        Err(err) if err.kind() == io::ErrorKind::AddrInUse => {
            if probe_alive(path) {
                Ok(BindOutcome::AlreadyRunning)
            } else {
                // Stale socket from a killed daemon; nothing is listening.
                std::fs::remove_file(path)?;
                UnixListener::bind(path).map(BindOutcome::Bound)
            }
        }
        Err(err) => Err(err),
    }
}

/// Sends one command and waits for its ack, retrying until `deadline`.
///
/// When the socket is dead or missing, `spawn_daemon` runs once (the stub
/// auto-spawns the daemon) and the retry loop waits for it to settle. A
/// garbage ack is reported immediately rather than retried: a retry would
/// send a second cycle command, and a daemon writing junk bytes will not
/// produce a good ack the second time either.
///
/// # Errors
///
/// Returns [`IpcError::TimedOut`] when `deadline` passes first.
pub fn send_with_deadline(
    path: &Path,
    direction: Direction,
    deadline: Instant,
    spawn_daemon: impl FnOnce(),
) -> Result<(), IpcError> {
    // FnOnce values cannot be called from a loop body directly; Option::take
    // runs the spawn exactly once.
    let mut spawn_daemon = Some(spawn_daemon);
    loop {
        let now = Instant::now();
        if now >= deadline {
            return Err(IpcError::TimedOut);
        }
        match UnixStream::connect(path) {
            Ok(mut stream) => {
                let remaining = deadline.saturating_duration_since(now);
                stream.set_read_timeout(Some(remaining.max(Duration::from_millis(1))))?;
                match send_once(&mut stream, direction) {
                    Ok(()) => return Ok(()),
                    Err(err) if err.kind() == io::ErrorKind::InvalidData => {
                        return Err(IpcError::Io(err));
                    }
                    // Lost ack or a daemon that died mid-reply: retrying may
                    // double-step the cycle, but a dropped reply most often
                    // means the command never landed.
                    Err(_) => {}
                }
            }
            Err(_) => {
                if let Some(spawn) = spawn_daemon.take() {
                    spawn();
                }
            }
        }
        let nap = RETRY_STEP.min(deadline.saturating_duration_since(Instant::now()));
        thread::sleep(nap);
    }
}

/// Writes the command byte and reads the ack on an established stream.
fn send_once(stream: &mut UnixStream, direction: Direction) -> io::Result<()> {
    stream.write_all(&[direction.wire_byte()])?;
    let mut ack = [0u8; 1];
    stream.read_exact(&mut ack)?;
    if ack[0] == ACK {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "daemon sent a malformed ack",
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;

    use super::*;

    /// Test-only temp directory; std has no tempfile, and adding the crate
    /// for this would violate the dependency discipline.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "airlock-ipc-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).expect("create temp dir");
            Self(path)
        }

        fn socket(&self) -> PathBuf {
            self.0.join(SOCKET_NAME)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// What the fake daemon does with one accepted connection.
    enum Server {
        /// Read the byte, write a real ack.
        Ack,
        /// Read the byte, write nothing.
        Silent,
        /// Read the byte, write a junk byte.
        Garbage,
    }

    /// Runs a one-connection fake daemon on a background thread. The socket
    /// is bound synchronously so callers can connect immediately; kernels
    /// queue connections between `bind` and `accept`.
    fn fake_daemon(path: &Path, behavior: Server) -> thread::JoinHandle<()> {
        let listener = UnixListener::bind(path).expect("test daemon binds");
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("test daemon accepts");
            let mut byte = [0u8; 1];
            stream.read_exact(&mut byte).expect("test daemon reads");
            match behavior {
                Server::Ack => stream.write_all(&[ACK]).expect("test daemon acks"),
                Server::Silent => thread::sleep(Duration::from_secs(5)),
                Server::Garbage => stream.write_all(b"x").expect("test daemon junks"),
            }
        })
    }

    #[test]
    fn socket_path_joins_the_runtime_dir() {
        assert_eq!(
            socket_path_from(Some("/run/user/1000")),
            Some(PathBuf::from("/run/user/1000/airlock.sock"))
        );
    }

    #[test]
    fn socket_path_without_runtime_dir_is_none() {
        assert_eq!(socket_path_from(None), None);
    }

    #[test]
    fn parse_command_maps_wire_bytes() {
        assert_eq!(parse_command(b'n'), Some(Direction::Next));
        assert_eq!(parse_command(b'p'), Some(Direction::Previous));
    }

    #[test]
    fn parse_command_rejects_junk() {
        assert_eq!(parse_command(b'x'), None);
        assert_eq!(parse_command(b'N'), None);
        assert_eq!(parse_command(0), None);
    }

    #[test]
    fn wire_bytes_round_trip_through_parse_command() {
        for direction in [Direction::Next, Direction::Previous] {
            assert_eq!(parse_command(direction.wire_byte()), Some(direction));
        }
    }

    #[test]
    fn bind_exclusive_binds_a_free_path() {
        let dir = TempDir::new();
        assert!(matches!(
            bind_exclusive(&dir.socket()),
            Ok(BindOutcome::Bound(_))
        ));
    }

    #[test]
    fn bind_exclusive_detects_a_live_daemon() {
        let dir = TempDir::new();
        let live = UnixListener::bind(dir.socket()).expect("first daemon binds");
        assert!(matches!(
            bind_exclusive(&dir.socket()),
            Ok(BindOutcome::AlreadyRunning)
        ));
        drop(live);
    }

    #[test]
    fn bind_exclusive_clears_a_stale_socket_file() {
        let dir = TempDir::new();
        let path = dir.socket();
        // A socket file left behind by a killed daemon: present, nothing listening.
        let stale = UnixListener::bind(&path).expect("stale socket binds");
        drop(stale);
        assert!(path.exists());

        assert!(matches!(bind_exclusive(&path), Ok(BindOutcome::Bound(_))));
    }

    #[test]
    fn send_round_trips_next_command() {
        let dir = TempDir::new();
        let server = fake_daemon(&dir.socket(), Server::Ack);
        let result = send_with_deadline(
            &dir.socket(),
            Direction::Next,
            Instant::now() + Duration::from_secs(5),
            || panic!("no spawn expected when the daemon answers"),
        );
        assert!(result.is_ok());
        server.join().expect("test daemon exits");
    }

    #[test]
    fn send_round_trips_previous_command() {
        let dir = TempDir::new();
        let server = fake_daemon(&dir.socket(), Server::Ack);
        let result = send_with_deadline(
            &dir.socket(),
            Direction::Previous,
            Instant::now() + Duration::from_secs(5),
            || panic!("no spawn expected when the daemon answers"),
        );
        assert!(result.is_ok());
        server.join().expect("test daemon exits");
    }

    #[test]
    fn send_times_out_when_the_daemon_stays_silent() {
        let dir = TempDir::new();
        // Detached: the silent server outlives the deadline by design.
        let _silent = fake_daemon(&dir.socket(), Server::Silent);
        let result = send_with_deadline(
            &dir.socket(),
            Direction::Next,
            Instant::now() + Duration::from_millis(200),
            || (),
        );
        assert!(matches!(result, Err(IpcError::TimedOut)));
    }

    #[test]
    fn send_fails_fast_on_a_garbage_ack() {
        let dir = TempDir::new();
        let server = fake_daemon(&dir.socket(), Server::Garbage);
        let result = send_with_deadline(
            &dir.socket(),
            Direction::Next,
            Instant::now() + Duration::from_secs(5),
            || (),
        );
        // A garbage ack must surface as an immediate protocol error, not a
        // timeout after retrying into more cycle commands.
        assert!(
            matches!(result, Err(IpcError::Io(err)) if err.kind() == io::ErrorKind::InvalidData)
        );
        server.join().expect("test daemon exits");
    }

    #[test]
    fn send_retries_until_a_spawned_daemon_appears() {
        let dir = TempDir::new();
        let path = dir.socket();
        // The "daemon" takes a moment to settle before owning the socket —
        // the spawn itself is a no-op stub, standing in for the real detach.
        let server = thread::spawn(move || {
            thread::sleep(Duration::from_millis(150));
            let listener = UnixListener::bind(&path).expect("late daemon binds");
            let (mut stream, _) = listener.accept().expect("late daemon accepts");
            let mut byte = [0u8; 1];
            stream.read_exact(&mut byte).expect("late daemon reads");
            stream.write_all(&[ACK]).expect("late daemon acks");
        });
        let result = send_with_deadline(
            &dir.socket(),
            Direction::Next,
            Instant::now() + Duration::from_secs(5),
            || (),
        );
        assert!(result.is_ok());
        server.join().expect("test daemon exits");
    }

    #[test]
    fn send_times_out_when_nothing_ever_listens() {
        let dir = TempDir::new();
        let result = send_with_deadline(
            &dir.socket(),
            Direction::Next,
            Instant::now() + Duration::from_millis(250),
            || (),
        );
        assert!(matches!(result, Err(IpcError::TimedOut)));
    }
}
