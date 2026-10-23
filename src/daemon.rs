//! Daemon core: activation tracking, cycle selection, and the serve loop.
//!
//! The pure pieces are unit-tested without a compositor; [`run`] wires
//! them to the live session and the IPC socket, which is only provable on
//! real COSMIC hardware.

use std::collections::{HashMap, HashSet};
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use crate::ipc::{self, Direction, IpcError};
use crate::switcher::{self, Handle, Snapshot, State, Toplevel};
use crate::ui::{Gesture, GestureInput, GestureOutput};
use crate::wayland;

/// How long after the last activity the serve loop keeps polling fast.
const ACTIVITY_WINDOW: Duration = Duration::from_millis(250);

/// Poll cadence while commands or compositor events are flowing.
const FAST_POLL: Duration = Duration::from_millis(1);

/// Poll cadence once everything has gone quiet.
const IDLE_POLL: Duration = Duration::from_millis(20);

/// Tracks activation order across successive snapshots.
///
/// Each newly observed `Activated` flag earns a monotonically increasing
/// tick; `stamp` writes those ticks into a snapshot's toplevels so
/// [`crate::switcher::eligible`] can order by them. Windows that disappear
/// from the snapshot are forgotten, so a reused handle number never
/// inherits stale history.
#[derive(Debug, Default)]
pub struct Mru {
    ticks: HashMap<Handle, u64>,
    activated: HashSet<Handle>,
    counter: u64,
}

impl Mru {
    /// Records activation transitions from one snapshot.
    pub fn observe(&mut self, toplevels: &[Toplevel]) {
        let present: HashSet<Handle> = toplevels.iter().map(|t| t.handle).collect();
        self.ticks.retain(|handle, _| present.contains(handle));
        self.activated.retain(|handle| present.contains(handle));

        let now_activated: HashSet<Handle> = toplevels
            .iter()
            .filter(|t| t.state.contains(State::ACTIVATED))
            .map(|t| t.handle)
            .collect();
        // Clear lost activations first so a window that regains focus in a
        // later snapshot earns a fresh tick.
        self.activated
            .retain(|handle| now_activated.contains(handle));

        for toplevel in toplevels {
            if toplevel.state.contains(State::ACTIVATED)
                && !self.activated.contains(&toplevel.handle)
            {
                self.counter += 1;
                self.ticks.insert(toplevel.handle, self.counter);
                self.activated.insert(toplevel.handle);
            }
        }
    }

    /// Writes the tracked ticks into `toplevels`.
    pub fn stamp(&self, toplevels: &mut [Toplevel]) {
        for toplevel in toplevels {
            toplevel.last_activated = self.ticks.get(&toplevel.handle).copied().unwrap_or(0);
        }
    }
}

/// Starts or advances the gesture session's selection ring by one step.
///
/// A session's first press builds the ring from the MRU front — the focused
/// window — so one step lands on the MRU-second (or wraps to the least
/// recent). Later presses reuse the same ring instead of rebuilding it:
/// walking three windows takes three presses, unlike T4's per-press rebuild
/// which toggled between the front two.
pub fn ring_step(
    ring: &mut Option<switcher::Ring>,
    snapshot: &Snapshot,
    direction: Direction,
) -> Option<Handle> {
    if ring.is_none() {
        let active = snapshot.active?;
        *ring = Some(switcher::Ring::new(switcher::eligible(
            &snapshot.toplevels,
            active,
        )));
    }
    match direction {
        Direction::Next => ring.as_mut()?.next(),
        Direction::Previous => ring.as_mut()?.previous(),
    }
}

/// Whether the overlay should close itself right now.
///
/// The idle deadline backs up lost input (a grab that stopped delivering
/// events); while Alt is held it is suspended, since a user quietly holding
/// the modifier produces no events. The hard deadline always applies so a
/// lost Alt-release can never stick a grabbed overlay.
fn should_close(idle_expired: bool, hard_expired: bool, alt_held: bool) -> bool {
    hard_expired || (idle_expired && !alt_held)
}

/// How long the serve loop should sleep, given time since last activity.
pub fn poll_interval(since_activity: Duration) -> Duration {
    if since_activity < ACTIVITY_WINDOW {
        FAST_POLL
    } else {
        IDLE_POLL
    }
}

/// Why the daemon stopped.
#[derive(Debug)]
pub enum RunError {
    /// The compositor connection failed or died.
    Session(wayland::SessionError),
    /// The IPC environment is unusable.
    Ipc(IpcError),
    /// The IPC socket itself failed.
    Io(io::Error),
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::Session(err) => write!(f, "{err}"),
            RunError::Ipc(err) => write!(f, "{err}"),
            RunError::Io(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for RunError {}

/// Runs the daemon in the foreground: settle the compositor state, own the
/// socket, then alternate between the compositor and IPC polls forever.
///
/// The socket is bound only after settling, so a successful connect from a
/// stub always implies a daemon with a complete snapshot behind it.
///
/// # Errors
///
/// Any error is fatal and exits loudly: a missing protocol at startup, a
/// dead compositor mid-run, or an unusable IPC path.
pub fn run() -> Result<(), RunError> {
    let mut session = wayland::Session::connect().map_err(RunError::Session)?;
    session
        .wait_until_settled(wayland::SETTLE_TIMEOUT)
        .map_err(RunError::Session)?;

    let path = ipc::socket_path().map_err(RunError::Ipc)?;
    let listener = match ipc::bind_exclusive(&path).map_err(RunError::Io)? {
        ipc::BindOutcome::Bound(listener) => listener,
        ipc::BindOutcome::AlreadyRunning => {
            eprintln!("airlock daemon: another daemon is already running");
            return Ok(());
        }
    };
    listener.set_nonblocking(true).map_err(RunError::Io)?;

    let mut mru = Mru::default();
    let mut snapshot = fresh_snapshot(&mut session, &mut mru);
    let mut pending: Vec<UnixStream> = Vec::new();
    let mut last_activity = Instant::now();

    // One gesture session at a time: the machine decides what each input
    // means, the ring is the session's selection over the eligible list.
    let mut gesture = Gesture::default();
    let mut ring: Option<switcher::Ring> = None;

    loop {
        let mut activity = session.poll().map_err(RunError::Session)?;
        // Refresh first so input processing in this iteration sees the
        // state the poll just delivered.
        if session.is_dirty() {
            snapshot = fresh_snapshot(&mut session, &mut mru);
            activity = true;
        }

        // Keyboard input on the grabbed overlay, queued during dispatch.
        // Drained before IPC so a release committing the previous session
        // lands before any new press opens the next one.
        for input in session.take_gesture_inputs() {
            activity = true;
            session.overlay_touch();
            match gesture.feed(input) {
                GestureOutput::Step(direction) => {
                    step_and_show(&mut session, &mut gesture, &mut ring, &snapshot, direction);
                }
                GestureOutput::Commit => {
                    if let Some(handle) = ring.as_ref().and_then(|ring| ring.selected()) {
                        session.activate(handle).map_err(RunError::Session)?;
                    }
                    session.overlay_hide("commit");
                    ring = None;
                }
                GestureOutput::Dismiss => {
                    session.overlay_hide("dismissed");
                    ring = None;
                }
                GestureOutput::Nothing => {}
            }
        }

        if let Some((idle, hard)) = session.overlay_deadlines()
            && should_close(idle, hard, gesture.alt_held())
        {
            let reason = if hard { "hard timeout" } else { "timeout" };
            session.overlay_hide(reason);
            gesture.close();
            ring = None;
        }

        // Accept whatever is queued; read one byte off each connection.
        loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    let _ = stream.set_nonblocking(true);
                    pending.push(stream);
                }
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => break,
                Err(err) => return Err(RunError::Io(err)),
            }
        }
        let mut unfinished = Vec::with_capacity(pending.len());
        for mut stream in pending.drain(..) {
            let mut byte = [0u8; 1];
            match stream.read(&mut byte) {
                // Stub closed without sending; drop the connection.
                Ok(0) => {}
                Ok(_) => {
                    activity = true;
                    if let Some(direction) = ipc::parse_command(byte[0]) {
                        // A stub press is an Invoke: open (or continue) the
                        // gesture session and step the selection. Activation
                        // waits for the Alt release — or Escape, focus loss,
                        // or the timeouts to end the session instead.
                        if let GestureOutput::Step(direction) =
                            gesture.feed(GestureInput::Invoke(direction))
                        {
                            step_and_show(
                                &mut session,
                                &mut gesture,
                                &mut ring,
                                &snapshot,
                                direction,
                            );
                        }
                        // One ack byte cannot realistically hit a full
                        // send buffer; a dropped ack makes the stub retry.
                        let _ = stream.write_all(&[ipc::ACK]);
                    }
                }
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => unfinished.push(stream),
                // Broken connection; drop it.
                Err(_) => {}
            }
        }
        pending = unfinished;

        if activity {
            last_activity = Instant::now();
        }
        std::thread::sleep(poll_interval(last_activity.elapsed()));
    }
}

/// Applies one `Step`: advance the session ring and refresh the overlay.
///
/// A session with nothing to show (empty workspace, or a selection that is
/// no longer eligible) is closed on the spot so the next press starts
/// fresh.
fn step_and_show(
    session: &mut wayland::Session,
    gesture: &mut Gesture,
    ring: &mut Option<switcher::Ring>,
    snapshot: &Snapshot,
    direction: Direction,
) {
    let selected = ring_step(ring, snapshot, direction);
    let model = selected.and_then(|handle| crate::ui::overlay_model(snapshot, handle));
    match model {
        Some(model) => session.overlay_show(model),
        None => {
            session.overlay_hide("nothing eligible");
            gesture.close();
            *ring = None;
        }
    }
}

/// Rebuilds the snapshot with MRU ticks stamped in.
fn fresh_snapshot(session: &mut wayland::Session, mru: &mut Mru) -> Snapshot {
    let mut snapshot = session.snapshot();
    mru.observe(&snapshot.toplevels);
    mru.stamp(&mut snapshot.toplevels);
    snapshot
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::switcher::WorkspaceId;

    fn toplevel(handle: u64, workspaces: &[u64], state: State, last_activated: u64) -> Toplevel {
        Toplevel {
            handle: Handle(handle),
            title: format!("toplevel {handle}"),
            app_id: format!("app.{handle}"),
            state,
            workspaces: workspaces.iter().copied().map(WorkspaceId).collect(),
            last_activated,
        }
    }

    fn snapshot(toplevels: Vec<Toplevel>, active: Option<u64>) -> Snapshot {
        Snapshot {
            toplevels,
            workspaces: Vec::new(),
            active: active.map(WorkspaceId),
        }
    }

    #[test]
    fn first_observe_gives_the_focused_window_the_only_tick() {
        let mut mru = Mru::default();
        let snap = snapshot(
            vec![
                toplevel(1, &[1], State::ACTIVATED, 0),
                toplevel(2, &[1], State::default(), 0),
            ],
            Some(1),
        );
        mru.observe(&snap.toplevels);
        let mut stamped = snap.toplevels.clone();
        mru.stamp(&mut stamped);
        assert_eq!(stamped[0].last_activated, 1);
        assert_eq!(stamped[1].last_activated, 0);
    }

    #[test]
    fn a_newly_activated_window_moves_to_the_front() {
        let mut mru = Mru::default();
        let first = snapshot(
            vec![
                toplevel(1, &[1], State::ACTIVATED, 0),
                toplevel(2, &[1], State::default(), 0),
            ],
            Some(1),
        );
        mru.observe(&first.toplevels);

        // Focus moved to window 2.
        let second = snapshot(
            vec![
                toplevel(1, &[1], State::default(), 0),
                toplevel(2, &[1], State::ACTIVATED, 0),
            ],
            Some(1),
        );
        mru.observe(&second.toplevels);
        let mut stamped = second.toplevels.clone();
        mru.stamp(&mut stamped);
        assert!(stamped[1].last_activated > stamped[0].last_activated);

        // And the MRU order follows: window 2 sorts first.
        let handles = switcher::eligible(&stamped, WorkspaceId(1));
        assert_eq!(handles, vec![Handle(2), Handle(1)]);
    }

    #[test]
    fn deactivation_and_reactivation_earns_a_fresh_tick() {
        let mut mru = Mru::default();
        let activated = vec![toplevel(1, &[1], State::ACTIVATED, 0)];
        mru.observe(&activated);
        let first_tick = tick_of(&mru, Handle(1));

        // Window 1 loses focus, then regains it.
        let deactivated = vec![toplevel(1, &[1], State::default(), 0)];
        mru.observe(&deactivated);
        mru.observe(&activated);
        let second_tick = tick_of(&mru, Handle(1));

        assert!(second_tick > first_tick);
    }

    #[test]
    fn closed_windows_are_forgotten() {
        let mut mru = Mru::default();
        mru.observe(&[toplevel(1, &[1], State::ACTIVATED, 0)]);
        assert_eq!(tick_of(&mru, Handle(1)), 1);

        // Window 1 closes; a later window reusing its handle number starts
        // from a clean slate.
        mru.observe(&[]);
        mru.observe(&[toplevel(1, &[1], State::default(), 0)]);
        assert_eq!(tick_of(&mru, Handle(1)), 0);
    }

    #[test]
    fn simultaneous_activations_follow_snapshot_order() {
        let mut mru = Mru::default();
        // Two windows report Activated in the same batch (a transient
        // compositor state): the later one is treated as more recent.
        mru.observe(&[
            toplevel(1, &[1], State::ACTIVATED, 0),
            toplevel(2, &[1], State::ACTIVATED, 0),
        ]);
        assert!(tick_of(&mru, Handle(2)) > tick_of(&mru, Handle(1)));
    }

    /// Test-only view of a single window's tick.
    fn tick_of(mru: &Mru, handle: Handle) -> u64 {
        let mut toplevel = toplevel(handle.0, &[1], State::default(), 0);
        mru.stamp(std::slice::from_mut(&mut toplevel));
        toplevel.last_activated
    }

    #[test]
    fn poll_interval_is_fast_while_active_and_slow_when_idle() {
        assert_eq!(poll_interval(Duration::from_millis(0)), FAST_POLL);
        assert_eq!(
            poll_interval(ACTIVITY_WINDOW - Duration::from_millis(1)),
            FAST_POLL
        );
        assert_eq!(poll_interval(ACTIVITY_WINDOW), IDLE_POLL);
    }

    #[test]
    fn first_press_targets_the_second_most_recent_window() {
        let snap = snapshot(
            vec![
                toplevel(1, &[1], State::ACTIVATED, 30),
                toplevel(2, &[1], State::default(), 20),
                toplevel(3, &[1], State::default(), 10),
            ],
            Some(1),
        );
        let mut ring = None;
        assert_eq!(
            ring_step(&mut ring, &snap, Direction::Next),
            Some(Handle(2))
        );
    }

    #[test]
    fn first_reverse_press_wraps_to_the_least_recent_window() {
        let snap = snapshot(
            vec![
                toplevel(1, &[1], State::ACTIVATED, 30),
                toplevel(2, &[1], State::default(), 20),
                toplevel(3, &[1], State::default(), 10),
            ],
            Some(1),
        );
        let mut ring = None;
        assert_eq!(
            ring_step(&mut ring, &snap, Direction::Previous),
            Some(Handle(3))
        );
    }

    #[test]
    fn consecutive_presses_walk_one_ring_without_rebuilding() {
        let snap = snapshot(
            vec![
                toplevel(1, &[1], State::ACTIVATED, 30),
                toplevel(2, &[1], State::default(), 20),
                toplevel(3, &[1], State::default(), 10),
            ],
            Some(1),
        );
        let mut ring = None;
        assert_eq!(
            ring_step(&mut ring, &snap, Direction::Next),
            Some(Handle(2))
        );
        assert_eq!(
            ring_step(&mut ring, &snap, Direction::Next),
            Some(Handle(3))
        );
        assert_eq!(
            ring_step(&mut ring, &snap, Direction::Next),
            Some(Handle(1))
        );
        assert_eq!(
            ring_step(&mut ring, &snap, Direction::Previous),
            Some(Handle(3))
        );
    }

    #[test]
    fn ring_step_skips_minimized_and_other_workspace_windows() {
        let snap = snapshot(
            vec![
                toplevel(1, &[1], State::ACTIVATED, 30),
                // Minimized on this workspace: skipped.
                toplevel(2, &[1], State::MINIMIZED, 25),
                // Live, but parked on workspace 2: skipped.
                toplevel(3, &[2], State::default(), 20),
                toplevel(4, &[1], State::default(), 10),
            ],
            Some(1),
        );
        let mut ring = None;
        assert_eq!(
            ring_step(&mut ring, &snap, Direction::Next),
            Some(Handle(4))
        );
    }

    #[test]
    fn ring_step_on_an_empty_workspace_selects_nothing() {
        let snap = snapshot(vec![toplevel(1, &[2], State::ACTIVATED, 30)], Some(1));
        let mut ring = None;
        assert_eq!(ring_step(&mut ring, &snap, Direction::Next), None);
        assert_eq!(ring_step(&mut ring, &snap, Direction::Previous), None);
    }

    #[test]
    fn ring_step_without_an_active_workspace_selects_nothing() {
        let snap = snapshot(vec![toplevel(1, &[1], State::ACTIVATED, 30)], None);
        let mut ring = None;
        assert_eq!(ring_step(&mut ring, &snap, Direction::Next), None);
        // No ring was cached, so a later snapshot with an active workspace
        // can still open one.
        let snap = snapshot(vec![toplevel(1, &[1], State::ACTIVATED, 30)], Some(1));
        assert_eq!(
            ring_step(&mut ring, &snap, Direction::Next),
            Some(Handle(1))
        );
    }

    #[test]
    fn idle_deadline_closes_only_when_alt_is_not_held() {
        assert!(should_close(true, false, false));
        assert!(!should_close(true, false, true));
        assert!(!should_close(false, false, false));
    }

    #[test]
    fn hard_deadline_closes_even_while_alt_is_held() {
        assert!(should_close(true, true, true));
        assert!(should_close(false, true, true));
    }
}
