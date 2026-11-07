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
use crate::settings::{self, ReadError, Reload, SettingsSource};
use crate::switcher::{self, Handle, Scope, Snapshot, State, Toplevel, WorkspaceId};
use crate::ui::{Gesture, GestureInput, GestureOutput, PointerHover, PointerInput};
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

/// One open gesture session: the selection ring plus the workspace and
/// scope it was built for. Recording both lets a mid-gesture switch void
/// the session — the simpler correct behavior chosen in T8: re-filtering
/// would silently swap the list under the user's held Alt, so the session
/// is dismissed and the next press starts fresh against real state. The
/// frozen scope extends the same philosophy to settings: an edit landing
/// mid-gesture changes nothing until the next press.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Switch {
    ring: switcher::Ring,
    workspace: WorkspaceId,
    scope: Scope,
    /// The app a same-app session lists windows of, or `None` for a plain
    /// Alt+Tab session. Chosen once at open and never re-derived: fresh
    /// snapshots are re-filtered by it, but a mid-gesture press never
    /// re-anchors.
    app: Option<String>,
    /// The card this session paints, resolved once at open beside the
    /// scope and frozen for the session's life — a theme edit landing
    /// mid-gesture never repaints under held Alt (the scope philosophy).
    palette: crate::ui::Palette,
    /// Windows the card keeps in view, resolved once at open from the
    /// `rows` setting and frozen like the palette: a config edit landing
    /// mid-gesture never re-windows the open session.
    rows: usize,
    /// Whether this session groups its list by app, resolved once at
    /// open from the `grouping` setting and frozen like the rows: a
    /// config edit landing mid-gesture never regroups the open session.
    grouping: bool,
    /// The frozen tree a grouped plain session walks: the partition its
    /// ring was built over, kept beside it so Tab's parent jumps and
    /// →/←'s tree steps address the same frozen list. `None` for flat
    /// sessions and same-app sessions (the exemption — already
    /// app-scoped, rendered flat).
    tree: Option<switcher::Grouping>,
    /// The window the card is showing over the session's list — derived
    /// state carried between rebuilds so the sticky rule can continue
    /// it, never persisted across gestures: sessions are built fresh
    /// with `None`, and every open starts top-aligned on the MRU head.
    viewport: Option<crate::ui::Viewport>,
}

/// Whether an open session still matches reality: the workspace it was
/// built for is still active, and its selection is still switchable there
/// under the session's scope (not closed, minimized, or — for a
/// CurrentWorkspace session — moved or unstickied away).
pub fn session_holds(open: &Switch, snapshot: &Snapshot) -> bool {
    snapshot.active == Some(open.workspace)
        && open
            .ring
            .selected()
            .is_some_and(|handle| switcher::switchable(snapshot, handle, open.scope))
}

/// The handle a commit should activate: the session's selection, but only
/// while it is still switchable in the current snapshot under the
/// session's scope. A CurrentWorkspace selection that left its workspace
/// mid-gesture commits as a no-op instead of crossing workspaces; an All
/// session may legitimately activate a window on another workspace.
pub fn commit_target(open: &Switch, snapshot: &Snapshot) -> Option<Handle> {
    open.ring
        .selected()
        .filter(|&handle| switcher::switchable(snapshot, handle, open.scope))
}

/// The window a quick toggle should activate: the MRU-second eligible
/// window under `scope` — the same target one quick Alt+Tab tap reaches —
/// or `None` when fewer than two windows are eligible, which makes the
/// toggle a silent no-op. No session, no overlay: the caller activates the
/// target immediately or does nothing at all.
pub fn quick_toggle_target(snapshot: &Snapshot, scope: Scope) -> Option<Handle> {
    let active = snapshot.active?;
    switcher::eligible(&snapshot.toplevels, active, scope)
        .get(1)
        .copied()
}

/// Starts or advances the gesture session's selection ring by one step.
///
/// A session's first press builds the ring from the MRU front — the focused
/// window — so one step lands on the MRU-second (or wraps to the least
/// recent). Later presses reuse the same ring instead of rebuilding it:
/// walking three windows takes three presses, unlike T4's per-press rebuild
/// which toggled between the front two. `scope`, `palette`, `rows`, and
/// `grouping` are consumed only when the session opens; an already-open
/// session keeps the ones it was built with.
pub fn ring_step(
    open: &mut Option<Switch>,
    snapshot: &Snapshot,
    direction: Direction,
    scope: Scope,
    palette: crate::ui::Palette,
    rows: usize,
    grouping: bool,
) -> Option<Handle> {
    if open.is_none() {
        let active = snapshot.active?;
        // A grouped session opens on the flattened tree: ring and tree
        // freeze together, so every later walk — Tab's parent jumps,
        // →/←'s tree steps, the model's rows — addresses one frozen
        // list. Flat sessions keep the eligible list, no tree.
        let (handles, tree) = if grouping {
            let tree = switcher::Grouping::new(&snapshot.toplevels, active, scope);
            let handles = tree.rows().iter().map(|row| row.handle).collect();
            (handles, Some(tree))
        } else {
            (switcher::eligible(&snapshot.toplevels, active, scope), None)
        };
        *open = Some(Switch {
            ring: switcher::Ring::new(handles),
            workspace: active,
            scope,
            app: None,
            palette,
            rows,
            grouping,
            tree,
            viewport: None,
        });
        // The opening press is the alt-tab keybind itself — grouped
        // mode's first Tab: step to the next (previous) parent row, so
        // a quick tap activates the MRU-second app's frontmost window,
        // not the MRU-second window. A tree with no parent anywhere is
        // a flat list — the press steps a row exactly as grouping-off
        // would; a lone parent wraps to itself and the selection holds.
        if let Some(tree) = open.as_ref().and_then(|session| session.tree.as_ref()) {
            let target = match direction {
                Direction::Next => tree.next_parent(0),
                Direction::Previous => tree.previous_parent(0),
            };
            return match target {
                Some(0) => open.as_mut()?.ring.selected(),
                Some(delta) => open.as_mut()?.ring.step(delta),
                None => match direction {
                    Direction::Next => open.as_mut()?.ring.step(1),
                    Direction::Previous => open.as_mut()?.ring.previous(),
                },
            };
        }
    }
    match direction {
        Direction::Next => open.as_mut()?.ring.step(1),
        Direction::Previous => open.as_mut()?.ring.previous(),
    }
}

/// One step of the gesture session, plain or same-app: [`ring_step`] over
/// the list the session was opened on.
///
/// A first `same_app` press anchors on the current application — the app_id
/// of the MRU-front eligible window; an empty app_id never anchors — and
/// opens the ring on that app's eligible windows alone. Fewer than two of
/// them makes the press a silent no-op: no session is stored, nothing is
/// selected, and the next press starts fresh. An already-open session steps
/// its frozen ring whatever key stepped it; a same-app session re-filters
/// the fresh snapshot by its frozen anchor (see [`step_and_show`]), but the
/// ring itself never rebuilds — the same freeze philosophy as the scope and
/// the palette, both consumed only when the session opens.
// Eight open-path parameters: the frozen-at-open values (palette, rows,
// grouping) each ride individually. The grouped mode (GR3) re-shapes this
// seam, so no bundle is introduced here — an `expect`, so the day the
// count drops the attribute warns and comes out.
#[expect(clippy::too_many_arguments)]
pub fn session_step(
    open: &mut Option<Switch>,
    snapshot: &Snapshot,
    direction: Direction,
    scope: Scope,
    same_app: bool,
    palette: crate::ui::Palette,
    rows: usize,
    grouping: bool,
) -> Option<Handle> {
    // An open session steps its frozen ring; the snapshot, scope,
    // palette, rows, and grouping only matter when a session opens.
    // Plain opens go straight through.
    if open.is_some() || !same_app {
        return ring_step(open, snapshot, direction, scope, palette, rows, grouping);
    }
    // A new same-app session: anchor, narrow, and hold the two-window
    // floor — one window of the app is nothing to cycle.
    let active = snapshot.active?;
    let app = switcher::app_anchor(&snapshot.toplevels, active, scope)?;
    let view = switcher::app_view(snapshot, &app, scope);
    if view.toplevels.len() < 2 {
        return None;
    }
    // The same-app exemption rides here: the session is already
    // app-scoped and renders flat, so it opens on the app's eligible
    // windows with no tree whatever the `grouping` setting says.
    let selected = ring_step(open, &view, direction, scope, palette, rows, false);
    // ring_step stored the session on the filtered ring; freeze the anchor
    // into it so later steps re-filter by it.
    if let Some(session) = open.as_mut() {
        session.app = Some(app);
    }
    selected
}

/// Resolves one tree walk for the open session: `target` computed from
/// the frozen tree and the selection's position in it, then the frozen
/// ring stepped by the wrapped delta. A `None` target — the walk's own
/// no-op — leaves everything untouched. Returns whether the selection
/// moved (the caller refreshes only then).
fn tree_walk(
    open: &mut Option<Switch>,
    target: impl Fn(&switcher::Grouping, usize) -> Option<usize>,
) -> bool {
    let Some(session) = open.as_ref() else {
        return false;
    };
    let Some(tree) = session.tree.as_ref() else {
        return false;
    };
    let Some(current) = session
        .ring
        .selected()
        .and_then(|handle| tree.rows().iter().position(|row| row.handle == handle))
    else {
        return false;
    };
    let Some(target) = target(tree, current) else {
        return false;
    };
    let len = tree.rows().len();
    let delta = (target + len - current) % len;
    if delta == 0 {
        return false; // the walk's own wrap-to-self: nothing moves
    }
    let Some(session) = open.as_mut() else {
        return false;
    };
    session.ring.step(delta);
    true
}

/// Steps an open grouped session to its next or previous *parent* row —
/// Tab's app step, skipping the current app's children (the GNOME
/// mirror). Acts on the frozen tree: an app that appeared mid-gesture is
/// unreachable, exactly as a window opened mid-gesture is unselectable.
/// A tree with no parent anywhere is a flat list, so Tab steps one row —
/// the grouping-off behavior; a lone parent wraps to itself and nothing
/// moves.
fn step_parent(open: &mut Option<Switch>, direction: Direction) -> bool {
    // `next_parent` answers `None` exactly when the tree holds no
    // parent rows (any parent at all would be found by wrapping).
    let flat = open
        .as_ref()
        .and_then(|session| session.tree.as_ref())
        .is_none_or(|tree| tree.next_parent(0).is_none());
    if flat {
        return match direction {
            Direction::Next => open
                .as_mut()
                .is_some_and(|session| session.ring.step(1).is_some()),
            Direction::Previous => open
                .as_mut()
                .is_some_and(|session| session.ring.previous().is_some()),
        };
    }
    tree_walk(open, |tree, current| match direction {
        Direction::Next => tree.next_parent(current),
        Direction::Previous => tree.previous_parent(current),
    })
}

/// → and ← over the frozen tree: `enter` resolves the selected parent's
/// first child, exit the selected child's parent row. The W3C no-ops —
/// → on a leaf (the end-node rule), ← on a top-level row (the root
/// rule) — resolve to no target and nothing moves. Returns whether the
/// selection moved.
fn tree_step(open: &mut Option<Switch>, enter: bool) -> bool {
    tree_walk(open, |tree, current| {
        if enter {
            tree.first_child(current)
        } else {
            tree.parent_of(current)
        }
    })
}

/// One step request into [`step_and_show`]: the direction, and whether a
/// *new* session should narrow to the current application.
struct Step {
    direction: Direction,
    same_app: bool,
}

/// Live settings state: the parsed rules plus the source that watches
/// them. Refreshed only at session open — one stat per invocation, so
/// nothing on the gesture path blocks on file reads.
struct LiveSettings {
    source: SettingsSource,
    settings: settings::Settings,
}

impl LiveSettings {
    /// Startup load: read once (missing file means defaults, silently;
    /// anything else diagnostic-worthy degrades to defaults too), then
    /// seed the mtime so the first gesture does not re-read for nothing.
    fn load() -> Self {
        let mut source = SettingsSource::new();
        let settings = match source.read() {
            Ok(settings) => settings,
            Err(ReadError::Missing) => settings::Settings::default(),
            Err(error) => {
                eprintln!("airlock daemon: {error}");
                settings::Settings::default()
            }
        };
        let _ = source.check();
        Self { source, settings }
    }

    /// One-stat reload check: adopt a changed file, fall back to defaults
    /// when it is gone, and keep last-good settings on a malformed read —
    /// settings can never break switching.
    fn refresh(&mut self) {
        match self.source.check() {
            Reload::Unchanged => {}
            Reload::NotPresent => self.settings = settings::Settings::default(),
            Reload::Changed => match self.source.read() {
                Ok(fresh) => self.settings = fresh,
                Err(ReadError::Missing) => self.settings = settings::Settings::default(),
                Err(error) => {
                    eprintln!("airlock daemon: keeping last-good settings: {error}");
                }
            },
        }
    }
}

/// Scope for a session starting on this snapshot: the active workspace's
/// name and coordinates-sorted position looked up in `settings`. Anything
/// unresolvable — no active workspace, or one missing from the workspace
/// list — keeps the v1 default.
fn resolve_scope(snapshot: &Snapshot, settings: &settings::Settings) -> Scope {
    snapshot
        .active
        .and_then(|active| settings::active_workspace(&snapshot.workspaces, active))
        .map_or(Scope::CurrentWorkspace, |active| {
            settings::scope_for(active, settings)
        })
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
    let mut settings = LiveSettings::load();
    // Desktop-entry enumeration once per daemon (tens of ms, off the
    // press path); rasterization happens per first-seen app.
    let mut icons = crate::icons::Cache::new();
    let mut pending: Vec<UnixStream> = Vec::new();
    let mut last_activity = Instant::now();

    // One gesture session at a time: the machine decides what each input
    // means, the session is the selection over the eligible list.
    let mut gesture = Gesture::default();
    let mut open: Option<Switch> = None;
    // Hover-selection policy for the overlay's pointer inputs, reset
    // whenever no session is open (see the pointer drain).
    let mut hover = PointerHover::default();

    loop {
        let mut activity = session.poll().map_err(RunError::Session)?;
        // Refresh first so input processing in this iteration sees the
        // state the poll just delivered.
        if session.is_dirty() {
            snapshot = fresh_snapshot(&mut session, &mut mru);
            activity = true;
        }

        // Live-state hardening: an open session is voided the moment its
        // premise breaks — the workspace switched under it, or the
        // selection closed, moved, was minimized, or lost its sticky flag.
        // Dismiss, not re-filter: swapping the list under a held Alt would
        // change what the user is about to commit to.
        if let Some(current) = &open
            && !session_holds(current, &snapshot)
        {
            session.overlay_hide("session stale");
            gesture.close();
            open = None;
        }

        // Keyboard input on the grabbed overlay, queued during dispatch.
        // Drained before IPC so a release committing the previous session
        // lands before any new press opens the next one.
        for input in session.take_gesture_inputs() {
            activity = true;
            session.overlay_touch();
            match gesture.feed(input) {
                GestureOutput::Step(direction) => {
                    // Keyboard steps only arrive while a session is open,
                    // so which ring a new session would open is moot here.
                    step_and_show(
                        &mut session,
                        &mut gesture,
                        &mut open,
                        &snapshot,
                        Step {
                            direction,
                            same_app: false,
                        },
                        &mut settings,
                        &mut icons,
                    );
                }
                GestureOutput::StepParent(direction) => {
                    // Grouped Tab: jump to the next/previous parent row
                    // against the session's frozen tree. Flat sessions
                    // never see this output — their key map produces
                    // plain Step — and a walk that lands where it
                    // started refreshes nothing.
                    if step_parent(&mut open, direction) {
                        show_selection(
                            &mut session,
                            &mut gesture,
                            &mut open,
                            &snapshot,
                            &mut icons,
                        );
                    }
                }
                GestureOutput::EnterChild => {
                    // → enters the selected parent's first child; on a
                    // leaf the W3C end-node rule makes it a no-op.
                    if tree_step(&mut open, true) {
                        show_selection(
                            &mut session,
                            &mut gesture,
                            &mut open,
                            &snapshot,
                            &mut icons,
                        );
                    }
                }
                GestureOutput::ExitParent => {
                    // ← exits to the selected child's parent; on a
                    // top-level row the W3C root rule makes it a no-op.
                    if tree_step(&mut open, false) {
                        show_selection(
                            &mut session,
                            &mut gesture,
                            &mut open,
                            &snapshot,
                            &mut icons,
                        );
                    }
                }
                GestureOutput::Commit => {
                    // The ring was built when the session opened; the
                    // window may have moved or closed since. Only a
                    // still-eligible selection is activated — anything else
                    // commits as a no-op rather than crossing workspaces.
                    commit_session(&mut session, &mut open, &snapshot)?;
                }
                GestureOutput::Dismiss => {
                    session.overlay_hide("dismissed");
                    open = None;
                }
                GestureOutput::Activate { handle } => {
                    // Ctrl+1..9: select and commit in one motion, the
                    // same flow a pointer click takes — addressing the
                    // handle the boundary captured at the key press. A
                    // `None` handle (a continuation or out-of-range row)
                    // no-ops with the session and gesture untouched.
                    if let Some(handle) = handle {
                        activate_row(&mut session, &mut gesture, &mut open, &snapshot, handle)?;
                    }
                }
                GestureOutput::Nothing => {}
            }
        }

        // Pointer input on the overlay, queued during dispatch beside the
        // keyboard's. A click selects the clicked row and commits in one
        // motion; a hover crossing moves the selection; the wheel steps
        // it.
        //
        // Every overlay end — commit, dismiss, timeout, stale — funnels
        // through `open = None`, so one reset guard at the drain's top
        // covers all of them: each session's first pointer contact seeds
        // the hover baseline instead of selecting.
        if open.is_none() {
            hover.reset();
        }
        for input in session.take_pointer_inputs() {
            activity = true;
            session.overlay_touch();
            match input {
                PointerInput::Click { handle } => {
                    activate_row(&mut session, &mut gesture, &mut open, &snapshot, handle)?;
                }
                PointerInput::Hover { row, handle } => {
                    // A crossing selects by the event-time target — the
                    // handle captured against the model the user saw —
                    // and only a handle the frozen ring still holds moves
                    // the selection (a window opened mid-gesture is not
                    // hoverable into the session). A continuation-row
                    // crossing (handle `None`) still counts as a row
                    // observation for the crossing policy and selects
                    // nothing; rows the ring refuses leave the overlay
                    // untouched.
                    if hover.observe(row).is_some()
                        && let Some(handle) = handle
                        && open
                            .as_mut()
                            .is_some_and(|open| open.ring.select(handle).is_some())
                    {
                        show_selection(
                            &mut session,
                            &mut gesture,
                            &mut open,
                            &snapshot,
                            &mut icons,
                        );
                    }
                }
                PointerInput::Wheel(direction) => {
                    // One notch = one step of the open session's ring,
                    // exactly the keyboard arrow arm (a ±240 event
                    // arrives as two inputs, one per notch, from the
                    // wayland handler). Structurally wheel only arrives
                    // while the overlay is mapped; the guard keeps that
                    // honest if delivery ever changes.
                    if open.is_some() {
                        step_and_show(
                            &mut session,
                            &mut gesture,
                            &mut open,
                            &snapshot,
                            Step {
                                direction,
                                same_app: false,
                            },
                            &mut settings,
                            &mut icons,
                        );
                    }
                }
            }
        }

        if let Some((idle, hard)) = session.overlay_deadlines()
            && should_close(idle, hard, gesture.alt_held())
        {
            let reason = if hard { "hard timeout" } else { "timeout" };
            session.overlay_hide(reason);
            gesture.close();
            open = None;
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
                    if let Some(command) = ipc::parse_command(byte[0]) {
                        match command {
                            // A stub press is an Invoke: open (or continue)
                            // the gesture session and step the selection.
                            // Alt+Tab opens the full ring; Alt+grave
                            // (CycleApp) opens it narrowed to the current
                            // application — and once a session is open, any
                            // invoke steps its frozen ring. Activation waits
                            // for the Alt release — or Escape, focus loss,
                            // or the timeouts to end the session instead.
                            ipc::Invoke::Cycle(direction) | ipc::Invoke::CycleApp(direction) => {
                                let same_app = matches!(command, ipc::Invoke::CycleApp(_));
                                if let GestureOutput::Step(direction) =
                                    gesture.feed(GestureInput::Invoke(direction))
                                {
                                    step_and_show(
                                        &mut session,
                                        &mut gesture,
                                        &mut open,
                                        &snapshot,
                                        Step {
                                            direction,
                                            same_app,
                                        },
                                        &mut settings,
                                        &mut icons,
                                    );
                                }
                            }
                            ipc::Invoke::QuickToggle => {
                                // No session, no overlay: refresh settings
                                // like a session open would, then activate
                                // the MRU-second window — or nothing, when
                                // there is nothing to toggle to.
                                settings.refresh();
                                let scope = resolve_scope(&snapshot, &settings.settings);
                                if let Some(handle) = quick_toggle_target(&snapshot, scope) {
                                    session.activate(handle).map_err(RunError::Session)?;
                                }
                            }
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

/// Commits the open session: activate its selection — but only a target
/// still switchable in `snapshot`, so a window that moved or closed since
/// the ring was built commits as a no-op rather than crossing workspaces
/// — then unmap the overlay and end the session. The keyboard Commit arm
/// and the pointer click path share it.
fn commit_session(
    session: &mut wayland::Session,
    open: &mut Option<Switch>,
    snapshot: &Snapshot,
) -> Result<(), RunError> {
    if let Some(handle) = open.as_ref().and_then(|open| commit_target(open, snapshot)) {
        session.activate(handle).map_err(RunError::Session)?;
    }
    session.overlay_hide("commit");
    *open = None;
    Ok(())
}

/// Selects `handle`'s window and commits the session in one motion —
/// the flow a pointer click and a Ctrl+N activation share. The handle
/// was captured at event receipt against the model the user saw; the
/// frozen ring must still hold it — a window opened mid-gesture is not
/// addressable into the session. Anything else leaves the session and
/// overlay exactly as they are. On success the gesture session is
/// closed explicitly: a click never reached the gesture machine, and a
/// Ctrl+N deliberately left it open (an unaddressable row must not
/// spend it) — either way a later Alt-release cannot re-commit and a
/// Tab cannot reopen the spent session.
fn activate_row(
    session: &mut wayland::Session,
    gesture: &mut Gesture,
    open: &mut Option<Switch>,
    snapshot: &Snapshot,
    handle: Handle,
) -> Result<(), RunError> {
    if open
        .as_mut()
        .is_some_and(|open| open.ring.select(handle).is_some())
    {
        commit_session(session, open, snapshot)?;
        gesture.close();
    }
    Ok(())
}

/// Applies one `Step`: advance the session ring and refresh the overlay.
///
/// `same_app` decides which ring a *new* session opens (see
/// [`session_step`]); an already-open session is stepped as-is whatever
/// key stepped it.
///
/// The overlay lists what the session lists: a same-app session's frozen
/// anchor re-filters the fresh snapshot, a plain session lists every
/// eligible window. A session with nothing to show (empty workspace, or a
/// selection that is no longer eligible) is closed on the spot so the next
/// press starts fresh.
///
/// Session open is the only moment settings matter: the config is
/// stat-checked and re-resolved exactly when a session starts, and an
/// already-open session keeps the scope and palette it was built with — a
/// mid-gesture edit must not swap the list or the card under the user's
/// held Alt.
///
/// Icons are prepared in [`show_selection`], at the session's render
/// scale: every app already seen by the daemon is a cache hit, and a
/// first-seen app rasterizes once (~1 ms, benchmarked in `src/icons.rs`)
/// — icon I/O never lands on the warm-press path twice for the same app.
fn step_and_show(
    session: &mut wayland::Session,
    gesture: &mut Gesture,
    open: &mut Option<Switch>,
    snapshot: &Snapshot,
    step: Step,
    settings: &mut LiveSettings,
    icons: &mut crate::icons::Cache,
) {
    let (scope, palette, rows, grouping) = match open {
        Some(open) => (open.scope, open.palette, open.rows, open.grouping),
        None => {
            settings.refresh();
            // `auto` asks the system once per open — two small file reads
            // at most, and pinned values never touch the disk at all.
            // `None` (unreadable preference) paints dark, never a broken
            // switch.
            let system_prefers_dark = (settings.settings.theme == crate::ui::ThemeSetting::Auto)
                .then(crate::theme::system_prefers_dark)
                .flatten();
            (
                resolve_scope(snapshot, &settings.settings),
                crate::ui::Palette::resolve(settings.settings.theme, system_prefers_dark),
                settings.settings.rows,
                settings.settings.grouping,
            )
        }
    };
    session_step(
        open,
        snapshot,
        step.direction,
        scope,
        step.same_app,
        palette,
        rows,
        grouping,
    );
    show_selection(session, gesture, open, snapshot, icons);
}

/// Builds and shows the overlay for the open session's current selection
/// — or closes the session when there is nothing eligible left to show.
///
/// Shared by [`step_and_show`] (after a step moved the selection) and the
/// pointer hover arm (which moves it without stepping): a same-app
/// session re-filters the fresh snapshot by its frozen anchor, a plain
/// one lists every eligible window. Icons are already cached, so this is
/// a rebuild-and-repaint, never new I/O.
fn show_selection(
    session: &mut wayland::Session,
    gesture: &mut Gesture,
    open: &mut Option<Switch>,
    snapshot: &Snapshot,
    icons: &mut crate::icons::Cache,
) {
    // The card this session paints: the palette frozen at open. A closed
    // session has nothing to paint — dark is the never-drawn default the
    // icon prepare below runs under.
    let palette = open
        .as_ref()
        .map_or_else(crate::ui::Palette::dark, |session| session.palette);
    let scale = session.render_scale();
    icons.prepare(
        snapshot
            .toplevels
            .iter()
            .map(|toplevel| toplevel.app_id.as_str()),
        scale,
        &palette,
    );
    let model = open.as_ref().and_then(|current| {
        // The tree flag renders the mode the session froze: grouped
        // plain sessions paint the tree, same-app sessions stay flat
        // (the exemption), flat sessions never grew one.
        let grouping = current.tree.is_some();
        let model_of = |list: &Snapshot| {
            current.ring.selected().and_then(|handle| {
                crate::ui::overlay_model(
                    list,
                    handle,
                    current.scope,
                    icons,
                    scale,
                    &palette,
                    grouping,
                    crate::ui::Windowing {
                        rows: current.rows,
                        prev: current.viewport,
                    },
                )
            })
        };
        match current.app.as_deref() {
            Some(app) => model_of(&switcher::app_view(snapshot, app, current.scope)),
            None => model_of(snapshot),
        }
    });
    match model {
        Some(model) => {
            // Carry the window this model computed into the next rebuild
            // — every path through this funnel (step, hover, same-app)
            // continues the sticky rule for free.
            if let Some(session) = open.as_mut() {
                session.viewport = Some(model.viewport);
            }
            // The key map rides the shown card: Tab/→/← mean tree
            // walks exactly when the card paints the tree.
            let grouped = open.as_ref().is_some_and(|session| session.tree.is_some());
            session.overlay_show(model, palette, grouped)
        }
        None => {
            session.overlay_hide("nothing eligible");
            gesture.close();
            *open = None;
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
    use crate::ui::{DEFAULT_ROWS, GestureKey, OverlayModel, Palette, RowModel};

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

    /// An overlay model whose rows carry `handles` in order — titles and
    /// geometry are default; only the row→window addressing matters at
    /// this seam.
    fn overlay_model(handles: &[u64]) -> OverlayModel {
        OverlayModel {
            rows: handles
                .iter()
                .map(|&handle| RowModel {
                    handle: Some(Handle(handle)),
                    ..RowModel::default()
                })
                .collect(),
            ..OverlayModel::default()
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
        let handles = switcher::eligible(&stamped, WorkspaceId(1), Scope::CurrentWorkspace);
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
        let mut open = None;
        assert_eq!(
            ring_step(
                &mut open,
                &snap,
                Direction::Next,
                Scope::CurrentWorkspace,
                Palette::dark(),
                DEFAULT_ROWS,
                false
            ),
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
        let mut open = None;
        assert_eq!(
            ring_step(
                &mut open,
                &snap,
                Direction::Previous,
                Scope::CurrentWorkspace,
                Palette::dark(),
                DEFAULT_ROWS,
                false
            ),
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
        let mut open = None;
        assert_eq!(
            ring_step(
                &mut open,
                &snap,
                Direction::Next,
                Scope::CurrentWorkspace,
                Palette::dark(),
                DEFAULT_ROWS,
                false
            ),
            Some(Handle(2))
        );
        assert_eq!(
            ring_step(
                &mut open,
                &snap,
                Direction::Next,
                Scope::CurrentWorkspace,
                Palette::dark(),
                DEFAULT_ROWS,
                false
            ),
            Some(Handle(3))
        );
        assert_eq!(
            ring_step(
                &mut open,
                &snap,
                Direction::Next,
                Scope::CurrentWorkspace,
                Palette::dark(),
                DEFAULT_ROWS,
                false
            ),
            Some(Handle(1))
        );
        assert_eq!(
            ring_step(
                &mut open,
                &snap,
                Direction::Previous,
                Scope::CurrentWorkspace,
                Palette::dark(),
                DEFAULT_ROWS,
                false
            ),
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
        let mut open = None;
        assert_eq!(
            ring_step(
                &mut open,
                &snap,
                Direction::Next,
                Scope::CurrentWorkspace,
                Palette::dark(),
                DEFAULT_ROWS,
                false
            ),
            Some(Handle(4))
        );
    }

    #[test]
    fn ring_step_on_an_empty_workspace_selects_nothing() {
        let snap = snapshot(vec![toplevel(1, &[2], State::ACTIVATED, 30)], Some(1));
        let mut open = None;
        assert_eq!(
            ring_step(
                &mut open,
                &snap,
                Direction::Next,
                Scope::CurrentWorkspace,
                Palette::dark(),
                DEFAULT_ROWS,
                false
            ),
            None
        );
        assert_eq!(
            ring_step(
                &mut open,
                &snap,
                Direction::Previous,
                Scope::CurrentWorkspace,
                Palette::dark(),
                DEFAULT_ROWS,
                false
            ),
            None
        );
    }

    #[test]
    fn ring_step_without_an_active_workspace_selects_nothing() {
        let snap = snapshot(vec![toplevel(1, &[1], State::ACTIVATED, 30)], None);
        let mut open = None;
        assert_eq!(
            ring_step(
                &mut open,
                &snap,
                Direction::Next,
                Scope::CurrentWorkspace,
                Palette::dark(),
                DEFAULT_ROWS,
                false
            ),
            None
        );
        // No ring was cached, so a later snapshot with an active workspace
        // can still open one.
        let snap = snapshot(vec![toplevel(1, &[1], State::ACTIVATED, 30)], Some(1));
        assert_eq!(
            ring_step(
                &mut open,
                &snap,
                Direction::Next,
                Scope::CurrentWorkspace,
                Palette::dark(),
                DEFAULT_ROWS,
                false
            ),
            Some(Handle(1))
        );
    }

    #[test]
    fn session_holds_while_workspace_and_selection_stay_valid() {
        let snap = snapshot(
            vec![
                toplevel(1, &[1], State::ACTIVATED, 30),
                toplevel(2, &[1], State::default(), 20),
            ],
            Some(1),
        );
        let mut open = None;
        ring_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            Palette::dark(),
            DEFAULT_ROWS,
            false,
        );
        let open = open.expect("session opened");
        assert!(session_holds(&open, &snap));
    }

    #[test]
    fn session_ends_when_the_workspace_switches_mid_gesture() {
        let snap = snapshot(
            vec![
                toplevel(1, &[1], State::ACTIVATED, 30),
                toplevel(2, &[1], State::STICKY, 20),
            ],
            Some(1),
        );
        let mut open = None;
        ring_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            Palette::dark(),
            DEFAULT_ROWS,
            false,
        );
        let open = open.expect("session opened");

        // Even a sticky selection — still switchable everywhere — is voided
        // by the switch: the list it came from is no longer the right one.
        let switched = snapshot(
            vec![
                toplevel(1, &[1], State::default(), 30),
                toplevel(2, &[1], State::STICKY, 20),
            ],
            Some(2),
        );
        assert!(!session_holds(&open, &switched));
    }

    #[test]
    fn session_ends_when_the_selected_window_closes() {
        let snap = snapshot(
            vec![
                toplevel(1, &[1], State::ACTIVATED, 30),
                toplevel(2, &[1], State::default(), 20),
            ],
            Some(1),
        );
        let mut open = None;
        ring_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            Palette::dark(),
            DEFAULT_ROWS,
            false,
        );
        let open = open.expect("session opened");

        let closed = snapshot(vec![toplevel(1, &[1], State::default(), 30)], Some(1));
        assert!(!session_holds(&open, &closed));
    }

    #[test]
    fn session_ends_when_the_selected_window_moves_away() {
        let snap = snapshot(
            vec![
                toplevel(1, &[1], State::ACTIVATED, 30),
                toplevel(2, &[1], State::default(), 20),
            ],
            Some(1),
        );
        let mut open = None;
        ring_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            Palette::dark(),
            DEFAULT_ROWS,
            false,
        );
        let open = open.expect("session opened");

        let moved = snapshot(
            vec![
                toplevel(1, &[1], State::default(), 30),
                toplevel(2, &[2], State::default(), 20),
            ],
            Some(1),
        );
        assert!(!session_holds(&open, &moved));
    }

    #[test]
    fn session_ends_when_the_selected_window_is_minimized() {
        let snap = snapshot(
            vec![
                toplevel(1, &[1], State::ACTIVATED, 30),
                toplevel(2, &[1], State::default(), 20),
            ],
            Some(1),
        );
        let mut open = None;
        ring_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            Palette::dark(),
            DEFAULT_ROWS,
            false,
        );
        let open = open.expect("session opened");

        let minimized = snapshot(
            vec![
                toplevel(1, &[1], State::default(), 30),
                toplevel(2, &[1], State::MINIMIZED, 20),
            ],
            Some(1),
        );
        assert!(!session_holds(&open, &minimized));
    }

    #[test]
    fn session_ends_when_sticky_is_taken_off_the_selection() {
        let snap = snapshot(
            vec![
                toplevel(1, &[1], State::ACTIVATED, 30),
                // Sticky, parked on workspace 2: eligible on 1 via sticky.
                toplevel(2, &[2], State::STICKY, 20),
            ],
            Some(1),
        );
        let mut open = None;
        ring_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            Palette::dark(),
            DEFAULT_ROWS,
            false,
        );
        let open = open.expect("session opened");

        let unstickied = snapshot(
            vec![
                toplevel(1, &[1], State::default(), 30),
                toplevel(2, &[2], State::default(), 20),
            ],
            Some(1),
        );
        assert!(!session_holds(&open, &unstickied));
    }

    #[test]
    fn commit_targets_the_selection_while_it_stays_eligible() {
        let snap = snapshot(
            vec![
                toplevel(1, &[1], State::ACTIVATED, 30),
                toplevel(2, &[1], State::default(), 20),
            ],
            Some(1),
        );
        let mut open = None;
        ring_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            Palette::dark(),
            DEFAULT_ROWS,
            false,
        );
        let open = open.expect("session opened");
        assert_eq!(commit_target(&open, &snap), Some(Handle(2)));
    }

    #[test]
    fn commit_skips_a_selection_that_left_the_workspace() {
        let snap = snapshot(
            vec![
                toplevel(1, &[1], State::ACTIVATED, 30),
                toplevel(2, &[1], State::default(), 20),
            ],
            Some(1),
        );
        let mut open = None;
        ring_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            Palette::dark(),
            DEFAULT_ROWS,
            false,
        );
        let open = open.expect("session opened");

        let moved = snapshot(
            vec![
                toplevel(1, &[1], State::default(), 30),
                toplevel(2, &[2], State::default(), 20),
            ],
            Some(1),
        );
        assert_eq!(commit_target(&open, &moved), None);
    }

    #[test]
    fn a_click_selects_then_commits_the_clicked_row() {
        // The pure seam the pointer Click arm composes: the boundary's
        // captured handle → Ring::select, commit_target. Clicking the
        // last row commits that window — not the selection the keyboard
        // left.
        let snap = snapshot(
            vec![
                toplevel(1, &[1], State::ACTIVATED, 30),
                toplevel(2, &[1], State::default(), 20),
                toplevel(3, &[1], State::default(), 10),
            ],
            Some(1),
        );
        let mut open = None;
        ring_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            Palette::dark(),
            DEFAULT_ROWS,
            false,
        );
        let mut open = open.expect("session opened");
        assert_eq!(open.ring.selected(), Some(Handle(2)));
        assert_eq!(open.ring.select(Handle(3)), Some(Handle(3)));
        assert_eq!(commit_target(&open, &snap), Some(Handle(3)));
    }

    #[test]
    fn a_click_on_a_window_the_ring_never_held_selects_nothing() {
        // The frozen ring is the session's truth: a handle it never held
        // (a window opened mid-gesture) cannot be clicked into the
        // session, and the flow stops at select — the prior selection is
        // never committed and the overlay stays.
        let snap = snapshot(
            vec![
                toplevel(1, &[1], State::ACTIVATED, 30),
                toplevel(2, &[1], State::default(), 20),
            ],
            Some(1),
        );
        let mut open = None;
        ring_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            Palette::dark(),
            DEFAULT_ROWS,
            false,
        );
        let mut open = open.expect("session opened");
        assert_eq!(open.ring.select(Handle(4)), None);
        assert_eq!(open.ring.selected(), Some(Handle(2)));
    }

    #[test]
    fn a_click_on_a_window_that_died_after_render_commits_a_no_op_activation() {
        // Keyboard parity via commit_target: the frozen ring still holds
        // the handle, live state no longer does, so the commit activates
        // nothing (the daemon still hides the overlay and ends the
        // session).
        let snap = snapshot(
            vec![
                toplevel(1, &[1], State::ACTIVATED, 30),
                toplevel(2, &[1], State::default(), 20),
            ],
            Some(1),
        );
        let mut open = None;
        ring_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            Palette::dark(),
            DEFAULT_ROWS,
            false,
        );
        let mut open = open.expect("session opened");
        assert_eq!(open.ring.select(Handle(2)), Some(Handle(2)));
        let closed = snapshot(vec![toplevel(1, &[1], State::default(), 30)], Some(1));
        assert_eq!(commit_target(&open, &closed), None);
    }

    #[test]
    fn a_click_captured_on_model_a_activates_as_window_across_a_same_batch_model_swap() {
        // The /ship NO-GO race, pinned at this seam. The press arrives
        // while the overlay shows model A; an input drained earlier in
        // the same batch (a Tab, a wheel notch) rebuilds the card as
        // model B — the same eligible list, shifted one row — before the
        // click drains. The user clicked A's row 0, Handle(1); whatever
        // the model has become by drain time, Handle(1) is the window
        // that must activate. (Run red against the row-addressed drain:
        // resolving row 0 against B selected Handle(9).)
        let snap = snapshot(
            vec![
                toplevel(9, &[1], State::default(), 40),
                toplevel(1, &[1], State::ACTIVATED, 30),
                toplevel(2, &[1], State::default(), 20),
                toplevel(3, &[1], State::default(), 10),
            ],
            Some(1),
        );
        let mut open = None;
        ring_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            Palette::dark(),
            DEFAULT_ROWS,
            false,
        );
        let mut open = open.expect("session opened");

        // Event receipt against model A: the boundary resolves the press
        // to A's row 0 and captures its handle — Click { handle } — the
        // target fixed against the model the user saw.
        let model_a = overlay_model(&[1, 2, 3]);
        let click = model_a
            .rows
            .first()
            .and_then(|row| row.handle)
            .expect("content row");
        assert_eq!(click, Handle(1));

        // The same-batch swap happened: B's row 0 is Handle(9), the
        // window the user never chose. The captured handle crosses the
        // queue untouched — the drain never consults the model — and the
        // ring takes Handle(1) wherever B put it.
        let model_b = overlay_model(&[9, 1, 2]);
        assert_eq!(
            model_b.rows.first().and_then(|row| row.handle),
            Some(Handle(9))
        );
        assert_eq!(
            open.ring.select(click),
            Some(Handle(1)),
            "the click's target is fixed at event receipt; a same-batch model shift must not retarget it"
        );
        assert_eq!(commit_target(&open, &snap), Some(Handle(1)));
    }

    #[test]
    fn a_hover_crossing_captured_before_a_model_shift_selects_the_pre_shift_window() {
        // The hover door of the same race: the crossing is observed
        // while the overlay shows model A, an earlier-drained input has
        // already shifted the model to B, and a later Alt release
        // commits. The selection target is the handle captured at the
        // observation — A's row — never B's row under the cursor.
        let snap = snapshot(
            vec![
                toplevel(9, &[1], State::default(), 40),
                toplevel(1, &[1], State::ACTIVATED, 30),
                toplevel(2, &[1], State::default(), 20),
                toplevel(3, &[1], State::default(), 10),
            ],
            Some(1),
        );
        let mut open = None;
        ring_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            Palette::dark(),
            DEFAULT_ROWS,
            false,
        );
        let mut open = open.expect("session opened");

        let model_a = overlay_model(&[1, 2, 3]);
        let model_b = overlay_model(&[9, 1, 2]);
        let mut hover = PointerHover::default();
        // Map-time contact seeds on A's row 2 without selecting; the
        // crossing onto A's row 0 captures Handle(1) — the event-time
        // input, exactly as the boundary queues it.
        assert_eq!(hover.observe(Some(2)), None, "map-time contact seeds");
        let PointerInput::Hover { row, handle } = (PointerInput::Hover {
            row: Some(0),
            handle: model_a.rows.first().and_then(|row| row.handle),
        }) else {
            unreachable!("constructed as a Hover")
        };
        // Drain after the shift: the crossing fires, and the selection
        // follows the captured handle — B's row 0 (Handle(9)) is
        // irrelevant.
        assert_eq!(
            model_b.rows.first().and_then(|row| row.handle),
            Some(Handle(9))
        );
        assert_eq!(
            hover.observe(row),
            Some(0),
            "crossing onto A's row 0 fires the policy"
        );
        let handle = handle.expect("a content row captures its handle");
        assert_eq!(open.ring.select(handle), Some(Handle(1)));
        assert_eq!(commit_target(&open, &snap), Some(Handle(1)));
    }

    #[test]
    fn a_hover_crossing_onto_a_continuation_row_observes_but_selects_nothing() {
        // A continuation row (handle `None`) is a real row for the
        // crossing policy — last_row moves, so a later crossing onto a
        // content row still selects — but it never selects itself.
        let snap = snapshot(
            vec![
                toplevel(1, &[1], State::ACTIVATED, 30),
                toplevel(3, &[1], State::default(), 10),
            ],
            Some(1),
        );
        let mut open = None;
        ring_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            Palette::dark(),
            DEFAULT_ROWS,
            false,
        );
        let mut open = open.expect("session opened");
        assert_eq!(open.ring.selected(), Some(Handle(3)));

        // Rows [h1, …, h3]: the middle slot is a continuation affordance.
        let model = OverlayModel {
            rows: vec![
                RowModel {
                    handle: Some(Handle(1)),
                    ..RowModel::default()
                },
                RowModel::default(),
                RowModel {
                    handle: Some(Handle(3)),
                    ..RowModel::default()
                },
            ],
            ..OverlayModel::default()
        };
        let mut hover = PointerHover::default();
        assert_eq!(hover.observe(Some(2)), None, "map-time contact seeds");
        // The crossing onto the continuation row is observed — the
        // policy registers the row — but its missing handle selects
        // nothing, exactly as the drain's guard runs.
        let PointerInput::Hover { row, handle } = (PointerInput::Hover {
            row: Some(1),
            handle: model.rows.get(1).and_then(|row| row.handle),
        }) else {
            unreachable!("constructed as a Hover")
        };
        assert_eq!(hover.observe(row), Some(1), "crossing is observed");
        assert!(handle.is_none(), "continuation rows carry no handle");
        assert_eq!(
            open.ring.selected(),
            Some(Handle(3)),
            "selection stands where the keyboard left it"
        );
        // The observation counted: crossing back onto a content row
        // selects it.
        let PointerInput::Hover { row, handle } = (PointerInput::Hover {
            row: Some(0),
            handle: model.rows.first().and_then(|row| row.handle),
        }) else {
            unreachable!("constructed as a Hover")
        };
        assert_eq!(hover.observe(row), Some(0));
        assert_eq!(
            open.ring.select(handle.expect("content row")),
            Some(Handle(1))
        );
        assert_eq!(commit_target(&open, &snap), Some(Handle(1)));
    }

    #[test]
    fn a_number_selection_captured_on_model_a_activates_as_window_across_a_same_batch_model_swap() {
        // The keyboard twin of the click race, pinned at this seam. The
        // digit press arrives while the overlay shows model A; a wheel
        // notch drained earlier in the same batch rebuilds the card as
        // model B before the key press drains. The user addressed A's
        // row 2 (Handle(3)); whatever the model has become by drain
        // time, Handle(3) is the window that must activate. (Run red
        // against the row-addressed drain: resolving row 2 against B
        // selected Handle(2).)
        let snap = snapshot(
            vec![
                toplevel(9, &[1], State::default(), 40),
                toplevel(1, &[1], State::ACTIVATED, 30),
                toplevel(2, &[1], State::default(), 20),
                toplevel(3, &[1], State::default(), 10),
            ],
            Some(1),
        );
        let mut open = None;
        ring_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            Palette::dark(),
            DEFAULT_ROWS,
            false,
        );
        let mut open = open.expect("session opened");

        // Event receipt against model A: the boundary resolves the
        // digit's row to A's row 2 and captures its handle — the
        // SelectHandle twin — the target fixed against the model the
        // user saw.
        let model_a = overlay_model(&[1, 2, 3]);
        let captured = model_a.rows.get(2).and_then(|row| row.handle);
        assert_eq!(captured, Some(Handle(3)));
        let mut gesture = Gesture::default();
        gesture.feed(GestureInput::Invoke(Direction::Next));
        assert_eq!(
            gesture.feed(GestureInput::Press {
                key: GestureKey::SelectHandle(captured),
                shift: false
            }),
            GestureOutput::Activate {
                handle: Some(Handle(3))
            }
        );

        // The same-batch swap happened: B's row 2 is Handle(2), the
        // window the user never chose. The captured handle crosses the
        // machine and the queue untouched — the drain never consults
        // the model — and the ring takes Handle(3) wherever B put it.
        let model_b = overlay_model(&[9, 1, 2]);
        assert_eq!(
            model_b.rows.get(2).and_then(|row| row.handle),
            Some(Handle(2))
        );
        assert_eq!(
            open.ring.select(captured.expect("content row")),
            Some(Handle(3)),
            "the Ctrl+N target is fixed at event receipt; a same-batch model shift must not retarget it"
        );
        assert_eq!(commit_target(&open, &snap), Some(Handle(3)));
    }

    #[test]
    fn a_number_activation_commits_that_row_and_spends_the_session() {
        // The seam the Ctrl+N `Activate` arm composes through the same
        // flow a pointer click takes: the boundary resolved row 2 to
        // Handle(3) at the press, the machine yields the handle,
        // Ring::select takes it, commit_target honors it — and the
        // daemon-side close() leaves a later Alt-release and Tab inert.
        let snap = snapshot(
            vec![
                toplevel(1, &[1], State::ACTIVATED, 30),
                toplevel(2, &[1], State::default(), 20),
                toplevel(3, &[1], State::default(), 10),
            ],
            Some(1),
        );
        let mut open = None;
        ring_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            Palette::dark(),
            DEFAULT_ROWS,
            false,
        );
        let mut open = open.expect("session opened");

        let mut gesture = Gesture::default();
        gesture.feed(GestureInput::Invoke(Direction::Next));
        assert_eq!(
            gesture.feed(GestureInput::Press {
                key: GestureKey::SelectHandle(Some(Handle(3))),
                shift: false
            }),
            GestureOutput::Activate {
                handle: Some(Handle(3))
            }
        );
        // Daemon side: the captured handle selects, then commits.
        assert_eq!(open.ring.select(Handle(3)), Some(Handle(3)));
        assert_eq!(commit_target(&open, &snap), Some(Handle(3)));
        gesture.close();
        assert_eq!(
            gesture.feed(GestureInput::Modifiers { alt: false }),
            GestureOutput::Nothing,
            "a later Alt-release cannot re-commit"
        );
        assert_eq!(
            gesture.feed(GestureInput::Press {
                key: GestureKey::Tab,
                shift: false
            }),
            GestureOutput::Nothing,
            "a later Tab cannot reopen the spent session"
        );
    }

    #[test]
    fn an_out_of_range_number_activation_leaves_the_session_steppable() {
        // Ctrl+7 with two rows on screen: the boundary's event-time
        // resolution yields no handle (row 6 is past the model's end),
        // so the output's `None` no-ops at the daemon exactly as an
        // off-model click stops — the selection and the session are
        // untouched, and a following Tab still steps.
        let snap = snapshot(
            vec![
                toplevel(1, &[1], State::ACTIVATED, 30),
                toplevel(2, &[1], State::default(), 20),
            ],
            Some(1),
        );
        let mut open = None;
        ring_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            Palette::dark(),
            DEFAULT_ROWS,
            false,
        );
        let open = open.expect("session opened");
        assert_eq!(open.ring.selected(), Some(Handle(2)));

        let mut gesture = Gesture::default();
        gesture.feed(GestureInput::Invoke(Direction::Next));
        assert_eq!(
            gesture.feed(GestureInput::Press {
                key: GestureKey::SelectHandle(None),
                shift: false
            }),
            GestureOutput::Activate { handle: None }
        );
        // No handle means nothing was selected: the frozen selection
        // stands and the session is still open for a Tab.
        assert_eq!(open.ring.selected(), Some(Handle(2)));
        assert_eq!(
            gesture.feed(GestureInput::Press {
                key: GestureKey::Tab,
                shift: false
            }),
            GestureOutput::Step(Direction::Next)
        );
    }

    #[test]
    fn a_number_activation_on_a_continuation_row_no_ops_with_the_session_intact() {
        // A continuation row exists on screen but carries no handle —
        // not addressable by any input. The digit addressing it resolves
        // to `None` at the boundary, the `None` flows through the
        // machine, and the daemon's guard leaves the session and the
        // selection exactly as they were.
        let snap = snapshot(
            vec![
                toplevel(1, &[1], State::ACTIVATED, 30),
                toplevel(3, &[1], State::default(), 10),
            ],
            Some(1),
        );
        let mut open = None;
        ring_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            Palette::dark(),
            DEFAULT_ROWS,
            false,
        );
        let open = open.expect("session opened");
        assert_eq!(open.ring.selected(), Some(Handle(3)));

        // Rows [h1, …, h3]: the middle slot is a continuation affordance.
        let model = OverlayModel {
            rows: vec![
                RowModel {
                    handle: Some(Handle(1)),
                    ..RowModel::default()
                },
                RowModel::default(),
                RowModel {
                    handle: Some(Handle(3)),
                    ..RowModel::default()
                },
            ],
            ..OverlayModel::default()
        };
        let mut gesture = Gesture::default();
        gesture.feed(GestureInput::Invoke(Direction::Next));
        assert_eq!(
            gesture.feed(GestureInput::Press {
                key: GestureKey::SelectHandle(model.rows.get(1).and_then(|row| row.handle)),
                shift: false
            }),
            GestureOutput::Activate { handle: None }
        );
        assert_eq!(
            open.ring.selected(),
            Some(Handle(3)),
            "the selection stands where the keyboard left it"
        );
        assert!(
            gesture.showing(),
            "the session survives the unaddressable row"
        );
    }

    #[test]
    fn a_number_activation_on_a_row_the_ring_never_held_selects_nothing() {
        // A window opened mid-gesture can render onto the model but
        // never enters the frozen ring: Ctrl+N addressing its row stops
        // at Ring::select — the prior selection is never committed and
        // the overlay stays (the session-freeze philosophy, shared with
        // the click path).
        let snap = snapshot(
            vec![
                toplevel(1, &[1], State::ACTIVATED, 30),
                toplevel(2, &[1], State::default(), 20),
            ],
            Some(1),
        );
        let mut open = None;
        ring_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            Palette::dark(),
            DEFAULT_ROWS,
            false,
        );
        let mut open = open.expect("session opened");
        assert_eq!(open.ring.select(Handle(4)), None);
        assert_eq!(open.ring.selected(), Some(Handle(2)));
    }

    #[test]
    fn a_hover_crossing_moves_the_selection_to_the_hovered_row() {
        // The pure seam the pointer Hover arm composes: crossing →
        // Ring::select. The cursor seeds on row 2 at map (no steal of
        // the MRU step target), crosses to row 3, and the selection
        // follows — a later commit targets the hovered window.
        let snap = snapshot(
            vec![
                toplevel(1, &[1], State::ACTIVATED, 30),
                toplevel(2, &[1], State::default(), 20),
                toplevel(3, &[1], State::default(), 10),
            ],
            Some(1),
        );
        let mut open = None;
        ring_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            Palette::dark(),
            DEFAULT_ROWS,
            false,
        );
        let mut open = open.expect("session opened");
        let mut hover = PointerHover::default();
        assert_eq!(hover.observe(Some(2)), None, "map-time contact seeds");
        assert_eq!(hover.observe(Some(3)), Some(3));
        assert_eq!(open.ring.select(Handle(3)), Some(Handle(3)));
        assert_eq!(commit_target(&open, &snap), Some(Handle(3)));
    }

    #[test]
    fn a_hover_on_a_window_the_ring_never_held_moves_nothing() {
        // The crossing is observed, but the frozen ring is the session's
        // truth: a window opened mid-gesture is not hoverable into it,
        // and the selection stays where the keyboard left it.
        let snap = snapshot(
            vec![
                toplevel(1, &[1], State::ACTIVATED, 30),
                toplevel(2, &[1], State::default(), 20),
            ],
            Some(1),
        );
        let mut open = None;
        ring_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            Palette::dark(),
            DEFAULT_ROWS,
            false,
        );
        let mut open = open.expect("session opened");
        let mut hover = PointerHover::default();
        assert_eq!(hover.observe(Some(2)), None, "map-time contact seeds");
        assert_eq!(
            hover.observe(Some(4)),
            Some(4),
            "crossing onto the new window"
        );
        assert_eq!(open.ring.select(Handle(4)), None);
        assert_eq!(open.ring.selected(), Some(Handle(2)));
    }

    #[test]
    fn quick_toggle_targets_the_mru_second_window() {
        let snap = snapshot(
            vec![
                toplevel(1, &[1], State::ACTIVATED, 30),
                toplevel(2, &[1], State::default(), 20),
                toplevel(3, &[1], State::default(), 10),
            ],
            Some(1),
        );
        assert_eq!(
            quick_toggle_target(&snap, Scope::CurrentWorkspace),
            Some(Handle(2))
        );
    }

    #[test]
    fn quick_toggle_with_fewer_than_two_eligible_windows_targets_nothing() {
        let one = snapshot(vec![toplevel(1, &[1], State::ACTIVATED, 30)], Some(1));
        assert_eq!(quick_toggle_target(&one, Scope::CurrentWorkspace), None);
        let none = snapshot(vec![], Some(1));
        assert_eq!(quick_toggle_target(&none, Scope::CurrentWorkspace), None);
    }

    #[test]
    fn quick_toggle_respects_the_workspace_scope() {
        let snap = snapshot(
            vec![
                toplevel(1, &[1], State::ACTIVATED, 30),
                // On workspace 2: the MRU-second under All, invisible here.
                toplevel(2, &[2], State::default(), 20),
                toplevel(3, &[1], State::default(), 10),
            ],
            Some(1),
        );
        assert_eq!(quick_toggle_target(&snap, Scope::All), Some(Handle(2)));
        assert_eq!(
            quick_toggle_target(&snap, Scope::CurrentWorkspace),
            Some(Handle(3))
        );
    }

    #[test]
    fn quick_toggle_skips_minimized_windows() {
        let snap = snapshot(
            vec![
                toplevel(1, &[1], State::ACTIVATED, 30),
                // The MRU-second, but minimized: not a toggle target.
                toplevel(2, &[1], State::MINIMIZED, 20),
                toplevel(3, &[1], State::default(), 10),
            ],
            Some(1),
        );
        assert_eq!(
            quick_toggle_target(&snap, Scope::CurrentWorkspace),
            Some(Handle(3))
        );
    }

    #[test]
    fn quick_toggle_without_an_active_workspace_targets_nothing() {
        let snap = snapshot(vec![toplevel(1, &[1], State::ACTIVATED, 30)], None);
        assert_eq!(quick_toggle_target(&snap, Scope::CurrentWorkspace), None);
    }

    /// Like [`toplevel`], with the app under test's choosing.
    fn app_toplevel(
        handle: u64,
        app: &str,
        workspaces: &[u64],
        state: State,
        last_activated: u64,
    ) -> Toplevel {
        Toplevel {
            app_id: app.to_string(),
            ..toplevel(handle, workspaces, state, last_activated)
        }
    }

    /// Editor windows 1, 3, 4 with a browser interleaved by MRU: a same-app
    /// session on "editor" must never let the browser surface.
    fn same_app_snapshot() -> Snapshot {
        snapshot(
            vec![
                app_toplevel(1, "editor", &[1], State::ACTIVATED, 40),
                app_toplevel(2, "browser", &[1], State::default(), 35),
                app_toplevel(3, "editor", &[1], State::default(), 30),
                app_toplevel(4, "editor", &[1], State::default(), 20),
            ],
            Some(1),
        )
    }

    #[test]
    fn same_app_first_press_selects_the_apps_mru_second_window() {
        let snap = same_app_snapshot();
        let mut open = None;
        assert_eq!(
            session_step(
                &mut open,
                &snap,
                Direction::Next,
                Scope::CurrentWorkspace,
                true,
                Palette::dark(),
                DEFAULT_ROWS,
                false
            ),
            Some(Handle(3)),
            "the browser never enters the ring; the app's own MRU-second lands first"
        );
        let session = open.expect("same-app session opened");
        assert_eq!(session.app.as_deref(), Some("editor"));
        assert_eq!(session.scope, Scope::CurrentWorkspace);
    }

    #[test]
    fn same_app_first_previous_press_wraps_to_the_apps_least_recent_window() {
        let snap = same_app_snapshot();
        let mut open = None;
        assert_eq!(
            session_step(
                &mut open,
                &snap,
                Direction::Previous,
                Scope::CurrentWorkspace,
                true,
                Palette::dark(),
                DEFAULT_ROWS,
                false
            ),
            Some(Handle(4))
        );
    }

    #[test]
    fn same_app_presses_walk_only_the_apps_windows() {
        let snap = same_app_snapshot();
        let mut open = None;
        let steps = [
            (Direction::Next, Some(Handle(3))),
            (Direction::Next, Some(Handle(4))),
            (Direction::Next, Some(Handle(1))),
            (Direction::Previous, Some(Handle(4))),
        ];
        for (direction, expected) in steps {
            assert_eq!(
                session_step(
                    &mut open,
                    &snap,
                    direction,
                    Scope::CurrentWorkspace,
                    true,
                    Palette::dark(),
                    DEFAULT_ROWS,
                    false
                ),
                expected
            );
        }
    }

    #[test]
    fn same_app_with_fewer_than_two_app_windows_is_a_silent_no_op() {
        let snap = snapshot(
            vec![
                app_toplevel(1, "editor", &[1], State::ACTIVATED, 30),
                app_toplevel(2, "browser", &[1], State::default(), 20),
            ],
            Some(1),
        );
        let mut open = None;
        assert_eq!(
            session_step(
                &mut open,
                &snap,
                Direction::Next,
                Scope::CurrentWorkspace,
                true,
                Palette::dark(),
                DEFAULT_ROWS,
                false
            ),
            None
        );
        assert!(open.is_none(), "no session, no overlay");
    }

    #[test]
    fn same_app_without_an_anchorable_window_opens_nothing() {
        let snap = snapshot(
            vec![
                app_toplevel(1, "", &[1], State::ACTIVATED, 30),
                app_toplevel(2, "", &[1], State::default(), 20),
            ],
            Some(1),
        );
        let mut open = None;
        assert_eq!(
            session_step(
                &mut open,
                &snap,
                Direction::Next,
                Scope::CurrentWorkspace,
                true,
                Palette::dark(),
                DEFAULT_ROWS,
                false
            ),
            None
        );
        assert!(open.is_none());
    }

    #[test]
    fn same_app_session_keeps_its_frozen_anchor_when_reinvoked() {
        let snap = same_app_snapshot();
        let mut open = None;
        session_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            true,
            Palette::dark(),
            DEFAULT_ROWS,
            false,
        );

        // Mid-gesture, focus moves to the browser: the next press must
        // step the frozen editor ring, never re-anchor on the browser.
        let refocused = snapshot(
            vec![
                app_toplevel(2, "browser", &[1], State::ACTIVATED, 50),
                app_toplevel(1, "editor", &[1], State::default(), 40),
                app_toplevel(3, "editor", &[1], State::default(), 30),
                app_toplevel(4, "editor", &[1], State::default(), 20),
            ],
            Some(1),
        );
        assert_eq!(
            session_step(
                &mut open,
                &refocused,
                Direction::Next,
                Scope::CurrentWorkspace,
                true,
                Palette::dark(),
                DEFAULT_ROWS,
                false
            ),
            Some(Handle(4))
        );
        assert_eq!(
            open.as_ref().expect("session survives").app.as_deref(),
            Some("editor")
        );
    }

    #[test]
    fn plain_presses_step_an_open_same_app_ring() {
        // Once the same-app session is open, any invoke steps its frozen
        // ring — including a plain Alt+Tab press.
        let snap = same_app_snapshot();
        let mut open = None;
        session_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            true,
            Palette::dark(),
            DEFAULT_ROWS,
            false,
        );
        assert_eq!(
            session_step(
                &mut open,
                &snap,
                Direction::Next,
                Scope::CurrentWorkspace,
                false,
                Palette::dark(),
                DEFAULT_ROWS,
                false
            ),
            Some(Handle(4))
        );
    }

    #[test]
    fn same_app_presses_step_an_open_plain_ring() {
        // And the mirror image: a same-app press during a plain session
        // steps that session's ring instead of re-anchoring.
        let snap = same_app_snapshot();
        let mut open = None;
        assert_eq!(
            session_step(
                &mut open,
                &snap,
                Direction::Next,
                Scope::CurrentWorkspace,
                false,
                Palette::dark(),
                DEFAULT_ROWS,
                false
            ),
            Some(Handle(2)),
            "a plain session steps the full ring: the global MRU-second"
        );
        assert_eq!(
            session_step(
                &mut open,
                &snap,
                Direction::Next,
                Scope::CurrentWorkspace,
                true,
                Palette::dark(),
                DEFAULT_ROWS,
                false
            ),
            Some(Handle(3)),
            "the frozen plain ring keeps walking; the anchor never applies"
        );
        assert_eq!(open.as_ref().expect("session survives").app, None);
    }

    #[test]
    fn same_app_commit_validates_against_the_full_snapshot() {
        // The session ring is filtered; commit validation is not. Under
        // All, a cross-workspace window of the anchored app commits.
        let snap = snapshot(
            vec![
                app_toplevel(1, "editor", &[1], State::ACTIVATED, 30),
                app_toplevel(2, "editor", &[2], State::default(), 20),
                app_toplevel(3, "browser", &[1], State::default(), 10),
            ],
            Some(1),
        );
        let mut open = None;
        session_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::All,
            true,
            Palette::dark(),
            DEFAULT_ROWS,
            false,
        );
        let open = open.expect("same-app session opened");
        assert_eq!(commit_target(&open, &snap), Some(Handle(2)));
    }

    #[test]
    fn same_app_session_voids_when_its_selection_minimizes() {
        // Session validity runs against the full snapshot under the
        // session's scope, exactly like a plain session.
        let snap = same_app_snapshot();
        let mut open = None;
        session_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            true,
            Palette::dark(),
            DEFAULT_ROWS,
            false,
        );
        let open = open.expect("same-app session opened");

        let minimized = snapshot(
            vec![
                app_toplevel(1, "editor", &[1], State::default(), 40),
                app_toplevel(2, "browser", &[1], State::default(), 35),
                app_toplevel(3, "editor", &[1], State::MINIMIZED, 30),
                app_toplevel(4, "editor", &[1], State::default(), 20),
            ],
            Some(1),
        );
        assert!(!session_holds(&open, &minimized));
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

    #[test]
    fn ring_step_under_all_lists_cross_workspace_windows_in_mru_order() {
        let snap = snapshot(
            vec![
                toplevel(1, &[1], State::ACTIVATED, 40),
                // On workspace 2: excluded by CurrentWorkspace, listed by All.
                toplevel(2, &[2], State::default(), 30),
                toplevel(3, &[1], State::default(), 20),
            ],
            Some(1),
        );
        let mut open = None;
        assert_eq!(
            ring_step(
                &mut open,
                &snap,
                Direction::Next,
                Scope::All,
                Palette::dark(),
                DEFAULT_ROWS,
                false
            ),
            Some(Handle(2))
        );
        assert_eq!(
            ring_step(
                &mut open,
                &snap,
                Direction::Next,
                Scope::All,
                Palette::dark(),
                DEFAULT_ROWS,
                false
            ),
            Some(Handle(3))
        );
        assert_eq!(
            ring_step(
                &mut open,
                &snap,
                Direction::Next,
                Scope::All,
                Palette::dark(),
                DEFAULT_ROWS,
                false
            ),
            Some(Handle(1))
        );
    }

    #[test]
    fn commit_of_a_cross_workspace_selection_validates_under_all_only() {
        let snap = snapshot(
            vec![
                toplevel(1, &[1], State::ACTIVATED, 30),
                toplevel(2, &[2], State::default(), 20),
            ],
            Some(1),
        );
        let mut open = None;
        ring_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::All,
            Palette::dark(),
            DEFAULT_ROWS,
            false,
        );
        let open = open.expect("session opened");
        // The selection lives on workspace 2; under All that is commitable.
        assert_eq!(commit_target(&open, &snap), Some(Handle(2)));

        // The same selection under the v1 scope is not switchable — and a
        // CurrentWorkspace session could never hold it in the first place.
        let confined = Switch {
            ring: open.ring.clone(),
            workspace: WorkspaceId(1),
            scope: Scope::CurrentWorkspace,
            app: None,
            palette: Palette::dark(),
            rows: DEFAULT_ROWS,
            grouping: false,
            tree: None,
            viewport: None,
        };
        assert_eq!(commit_target(&confined, &snap), None);
    }

    #[test]
    fn a_session_opens_with_no_carried_window() {
        // The sticky window is gesture-scoped state: sessions are built
        // fresh with `viewport: None`, so every gesture opens
        // top-aligned on the MRU head and never inherits the previous
        // gesture's window.
        let snap = snapshot(
            (1..=15)
                .map(|handle| toplevel(handle, &[1], State::default(), 16 - handle))
                .collect(),
            Some(1),
        );
        let mut open = None;
        ring_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::All,
            Palette::dark(),
            DEFAULT_ROWS,
            false,
        );
        assert!(
            open.as_ref().is_some_and(|open| open.viewport.is_none()),
            "a fresh session must open unwindowed"
        );
    }

    #[test]
    fn session_voids_on_a_workspace_switch_under_both_scopes() {
        // A sticky window stays switchable everywhere, so only the premise
        // break (the workspace switch) can end these sessions.
        let snap = snapshot(
            vec![
                toplevel(1, &[1], State::ACTIVATED, 30),
                toplevel(2, &[1], State::STICKY, 20),
            ],
            Some(1),
        );
        let switched = snapshot(
            vec![
                toplevel(1, &[1], State::default(), 30),
                toplevel(2, &[1], State::STICKY, 20),
            ],
            Some(2),
        );
        for scope in [Scope::CurrentWorkspace, Scope::All] {
            let mut open = None;
            ring_step(
                &mut open,
                &snap,
                Direction::Next,
                scope,
                Palette::dark(),
                DEFAULT_ROWS,
                false,
            );
            let open = open.expect("session opened");
            assert!(session_holds(&open, &snap), "holds under {scope:?}");
            assert!(!session_holds(&open, &switched), "voids under {scope:?}");
        }
    }

    #[test]
    fn resolve_scope_applies_settings_for_the_active_workspace() {
        let snap = Snapshot {
            toplevels: Vec::new(),
            workspaces: vec![crate::switcher::Workspace {
                id: WorkspaceId(1),
                name: "browse".to_owned(),
                coordinates: vec![0, 0],
            }],
            active: Some(WorkspaceId(1)),
        };
        let not_isolated = crate::settings::Settings {
            theme: crate::ui::ThemeSetting::Auto,
            rows: DEFAULT_ROWS,
            grouping: false,
            workspaces: vec![crate::settings::WorkspaceRule {
                reference: crate::settings::WorkspaceRef::Name("browse".to_owned()),
                isolated: false,
            }],
        };
        assert_eq!(resolve_scope(&snap, &not_isolated), Scope::All);
        // No settings, no rule, or an unknown workspace: v1 default.
        assert_eq!(
            resolve_scope(&snap, &crate::settings::Settings::default()),
            Scope::CurrentWorkspace
        );
        let unknown = Snapshot {
            toplevels: Vec::new(),
            workspaces: Vec::new(),
            active: Some(WorkspaceId(9)),
        };
        assert_eq!(
            resolve_scope(&unknown, &not_isolated),
            Scope::CurrentWorkspace
        );
    }

    /// Unique disposable directory (no tempfile dependency).
    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "airlock-daemon-{tag}-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            ));
            std::fs::create_dir_all(&path).expect("create temp dir");
            TempDir(path)
        }

        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn live_at(path: std::path::PathBuf) -> LiveSettings {
        LiveSettings {
            source: crate::settings::SettingsSource::at(path),
            settings: crate::settings::Settings::default(),
        }
    }

    #[test]
    fn refresh_adopts_a_changed_config_file() {
        let dir = TempDir::new("adopt");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, b"[[workspace]]\nname = \"code\"\nisolated = false\n")
            .expect("write");
        let mut live = live_at(path);
        live.refresh();
        assert_eq!(live.settings.workspaces.len(), 1);
        // Unchanged file: same settings, no churn.
        live.refresh();
        assert_eq!(live.settings.workspaces.len(), 1);
    }

    #[test]
    fn refresh_keeps_last_good_settings_when_the_file_turns_malformed() {
        let dir = TempDir::new("last-good");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, b"[[workspace]]\nname = \"code\"\nisolated = false\n")
            .expect("write");
        let mut live = live_at(path.clone());
        live.refresh();
        assert_eq!(live.settings.workspaces.len(), 1);

        std::fs::write(&path, b"[[[[broken").expect("break");
        live.refresh();
        assert_eq!(
            live.settings.workspaces.len(),
            1,
            "last-good settings survive a malformed edit"
        );
    }

    #[test]
    fn refresh_without_a_config_file_falls_back_to_defaults() {
        let dir = TempDir::new("absent");
        let mut live = live_at(dir.path().join("config.toml"));
        live.settings.workspaces = vec![crate::settings::WorkspaceRule {
            reference: crate::settings::WorkspaceRef::Name("stale".to_owned()),
            isolated: false,
        }];
        live.refresh();
        assert_eq!(live.settings, crate::settings::Settings::default());
    }

    #[test]
    fn refresh_adopts_a_theme_change() {
        // Live apply: editing `theme` takes effect on the refresh a next
        // invocation performs — no daemon restart.
        let dir = TempDir::new("theme-flip");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, b"theme = \"dark\"\n").expect("write");
        let mut live = live_at(path.clone());
        live.refresh();
        assert_eq!(live.settings.theme, crate::ui::ThemeSetting::Dark);

        std::fs::write(&path, b"theme = \"light\"\n").expect("rewrite");
        live.refresh();
        assert_eq!(live.settings.theme, crate::ui::ThemeSetting::Light);
    }

    #[test]
    fn refresh_adopts_a_rows_change() {
        // Live apply, the rows twin: editing `rows` takes effect on the
        // refresh a next invocation performs — no daemon restart.
        let dir = TempDir::new("rows-edit");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, b"rows = 5\n").expect("write");
        let mut live = live_at(path.clone());
        live.refresh();
        assert_eq!(live.settings.rows, 5);

        std::fs::write(&path, b"rows = 9\n").expect("rewrite");
        live.refresh();
        assert_eq!(live.settings.rows, 9);

        // A bad edit keeps the last-good value, like every setting.
        std::fs::write(&path, b"rows = 42\n").expect("rewrite bad");
        live.refresh();
        assert_eq!(live.settings.rows, 9, "an out-of-range rows must not apply");
    }

    #[test]
    fn refresh_adopts_a_grouping_change() {
        // Live apply, the grouping twin: editing `grouping` takes effect
        // on the refresh a next invocation performs — no daemon restart.
        let dir = TempDir::new("grouping-edit");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, b"grouping = false\n").expect("write");
        let mut live = live_at(path.clone());
        live.refresh();
        assert!(!live.settings.grouping);

        std::fs::write(&path, b"grouping = true\n").expect("rewrite");
        live.refresh();
        assert!(live.settings.grouping);

        // A non-bool edit keeps the last-good value, like every setting.
        std::fs::write(&path, b"grouping = \"yes\"\n").expect("rewrite bad");
        live.refresh();
        assert!(live.settings.grouping, "a non-bool grouping must not apply");
    }

    /// The palette twin of the frozen-scope seam: the card is resolved
    /// once at session open and frozen for the session's life — a theme
    /// edit landing mid-gesture (here: the palette the next invocation
    /// would resolve) changes nothing until the session closes and the
    /// next press starts fresh.
    #[test]
    fn an_open_session_keeps_its_frozen_palette_across_a_theme_edit() {
        let snap = snapshot(
            vec![
                toplevel(1, &[1], State::ACTIVATED, 30),
                toplevel(2, &[1], State::default(), 20),
            ],
            Some(1),
        );
        let mut open = None;
        session_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            false,
            crate::ui::Palette::dark(),
            DEFAULT_ROWS,
            false,
        );
        assert_eq!(
            open.as_ref().expect("session opened").palette,
            crate::ui::Palette::dark()
        );

        // The config flips to light between presses; the open session
        // still paints the card it was opened with.
        session_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            false,
            crate::ui::Palette::light(),
            DEFAULT_ROWS,
            false,
        );
        assert_eq!(
            open.as_ref().expect("session still open").palette,
            crate::ui::Palette::dark(),
            "a mid-gesture theme edit must not repaint the open session"
        );

        // The next press after the session ends starts fresh: light.
        open = None;
        session_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            false,
            crate::ui::Palette::light(),
            DEFAULT_ROWS,
            false,
        );
        assert_eq!(
            open.as_ref().expect("fresh session opened").palette,
            crate::ui::Palette::light()
        );
    }

    /// The rows twin of the frozen-palette seam: the budget is resolved
    /// once at session open and frozen for the session's life — a `rows`
    /// edit landing mid-gesture (here: the value the next invocation
    /// would read) changes nothing until the session closes and the next
    /// press starts fresh.
    #[test]
    fn an_open_session_keeps_its_frozen_rows_across_a_config_edit() {
        let snap = snapshot(
            vec![
                toplevel(1, &[1], State::ACTIVATED, 30),
                toplevel(2, &[1], State::default(), 20),
            ],
            Some(1),
        );
        let mut open = None;
        session_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            false,
            Palette::dark(),
            5,
            false,
        );
        assert_eq!(open.as_ref().expect("session opened").rows, 5);

        // The config moves to nine between presses; the open session
        // still windows to the budget it was opened with.
        session_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            false,
            Palette::dark(),
            9,
            false,
        );
        assert_eq!(
            open.as_ref().expect("session still open").rows,
            5,
            "a mid-gesture rows edit must not re-window the open session"
        );

        // The next press after the session ends starts fresh: nine.
        open = None;
        session_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            false,
            Palette::dark(),
            9,
            false,
        );
        assert_eq!(open.as_ref().expect("fresh session opened").rows, 9);
    }

    /// The grouping twin of the frozen-rows seam: the mode is resolved
    /// once at session open and frozen for the session's life — a
    /// `grouping` edit landing mid-gesture (here: the value the next
    /// invocation would read) changes nothing until the session closes
    /// and the next press starts fresh.
    #[test]
    fn an_open_session_keeps_its_frozen_grouping_across_a_config_edit() {
        let snap = snapshot(
            vec![
                toplevel(1, &[1], State::ACTIVATED, 30),
                toplevel(2, &[1], State::default(), 20),
            ],
            Some(1),
        );
        let mut open = None;
        session_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            false,
            Palette::dark(),
            DEFAULT_ROWS,
            false,
        );
        assert!(!open.as_ref().expect("session opened").grouping);

        // The config flips to grouped between presses; the open session
        // still steps the flat list it was opened with.
        session_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            false,
            Palette::dark(),
            DEFAULT_ROWS,
            true,
        );
        assert!(
            !open.as_ref().expect("session still open").grouping,
            "a mid-gesture grouping edit must not regroup the open session"
        );

        // The next press after the session ends starts fresh: grouped.
        open = None;
        session_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            false,
            Palette::dark(),
            DEFAULT_ROWS,
            true,
        );
        assert!(open.as_ref().expect("fresh session opened").grouping);
    }

    // --- Grouped sessions --------------------------------------------------

    /// Two two-window apps, MRU a1, a2, b1, b2 — the fixture whose flat
    /// and grouped quick-taps differ: flat steps to a2, grouped to b1.
    fn grouped_two_apps() -> Snapshot {
        snapshot(
            vec![
                app_toplevel(1, "app.a", &[1], State::ACTIVATED, 40),
                app_toplevel(2, "app.a", &[1], State::default(), 30),
                app_toplevel(3, "app.b", &[1], State::default(), 20),
                app_toplevel(4, "app.b", &[1], State::default(), 10),
            ],
            Some(1),
        )
    }

    #[test]
    fn a_grouped_opening_press_selects_the_second_apps_front_window() {
        // The quick-tap pin: one Alt+Tab tap in grouped mode activates
        // the MRU-second *app's* frontmost window, not the MRU-second
        // window — here b1, where the flat ring would step to a2.
        let snap = grouped_two_apps();
        let mut open = None;
        let grouped = session_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            false,
            Palette::dark(),
            DEFAULT_ROWS,
            true,
        );
        assert_eq!(grouped, Some(Handle(3)), "quick-tap lands on b's front");

        let mut flat = None;
        let flat_target = session_step(
            &mut flat,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            false,
            Palette::dark(),
            DEFAULT_ROWS,
            false,
        );
        assert_eq!(flat_target, Some(Handle(2)), "flat keeps the MRU-second");
    }

    #[test]
    fn grouped_sessions_step_the_flattened_tree() {
        // ↑/↓ walk parents and children as one sequence: the ring holds
        // the flattened tree, so a plain step from b1 reaches b2 and
        // wraps around to a1.
        let snap = grouped_two_apps();
        let mut open = None;
        session_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            false,
            Palette::dark(),
            DEFAULT_ROWS,
            true,
        );
        assert_eq!(
            open.as_ref().expect("opened").ring.selected(),
            Some(Handle(3))
        );
        for expected in [Handle(4), Handle(1)] {
            assert_eq!(
                session_step(
                    &mut open,
                    &snap,
                    Direction::Next,
                    Scope::CurrentWorkspace,
                    false,
                    Palette::dark(),
                    DEFAULT_ROWS,
                    true,
                ),
                Some(expected)
            );
        }
    }

    #[test]
    fn tab_steps_to_parent_rows_skipping_children() {
        // From inside a's children, Tab lands on b's parent; from b's
        // parent it wraps to a's parent — parents only, either way.
        let snap = grouped_two_apps();
        let mut open = None;
        session_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            false,
            Palette::dark(),
            DEFAULT_ROWS,
            true,
        );
        open.as_mut().expect("opened").ring.select(Handle(2)); // a's child
        assert!(step_parent(&mut open, Direction::Next));
        assert_eq!(
            open.as_ref().expect("open").ring.selected(),
            Some(Handle(3)),
            "Tab from a's child lands on b's parent"
        );
        assert!(step_parent(&mut open, Direction::Next));
        assert_eq!(
            open.as_ref().expect("open").ring.selected(),
            Some(Handle(1)),
            "Tab wraps around to a's parent"
        );
        assert!(step_parent(&mut open, Direction::Previous));
        assert_eq!(
            open.as_ref().expect("open").ring.selected(),
            Some(Handle(3)),
            "Shift+Tab steps back to b's parent"
        );
    }

    #[test]
    fn tab_on_a_lone_parent_stays_put() {
        // One multi-window app and a single window beside it: the only
        // parent is a's, so Tab wraps to itself — no movement, the
        // selection holds. (The VM walk judges this edge; the pin keeps
        // it deliberate.)
        let snap = snapshot(
            vec![
                app_toplevel(1, "app.a", &[1], State::ACTIVATED, 40),
                app_toplevel(2, "app.a", &[1], State::default(), 30),
                app_toplevel(3, "app.c", &[1], State::default(), 20),
            ],
            Some(1),
        );
        let mut open = None;
        session_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            false,
            Palette::dark(),
            DEFAULT_ROWS,
            true,
        );
        let session = open.as_ref().expect("opened");
        assert_eq!(session.ring.selected(), Some(Handle(1)));
        assert!(!step_parent(&mut open, Direction::Next));
        assert_eq!(
            open.as_ref().expect("open").ring.selected(),
            Some(Handle(1)),
            "Tab on the lone parent moves nothing"
        );
    }

    #[test]
    fn tab_without_parents_steps_rows_like_flat() {
        // All single-window apps: the tree is flat, so Tab falls back to
        // the flat step — the opening press reaches the MRU-second
        // window exactly as grouping-off would.
        let snap = snapshot(
            vec![
                app_toplevel(1, "app.a", &[1], State::ACTIVATED, 30),
                app_toplevel(2, "app.b", &[1], State::default(), 20),
                app_toplevel(3, "app.c", &[1], State::default(), 10),
            ],
            Some(1),
        );
        let mut open = None;
        let opened = session_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            false,
            Palette::dark(),
            DEFAULT_ROWS,
            true,
        );
        assert_eq!(opened, Some(Handle(2)), "no parents: flat quick-tap");
        assert!(step_parent(&mut open, Direction::Next));
        assert_eq!(
            open.as_ref().expect("open").ring.selected(),
            Some(Handle(3)),
            "no parents: Tab steps one row"
        );
    }

    #[test]
    fn enter_child_jumps_to_the_first_child_and_no_ops_elsewhere() {
        // → on a parent lands on its first child; on a leaf the W3C
        // end-node rule makes it a silent no-op.
        let snap = grouped_two_apps();
        let mut open = None;
        session_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            false,
            Palette::dark(),
            DEFAULT_ROWS,
            true,
        );
        open.as_mut().expect("opened").ring.select(Handle(1)); // a's parent
        assert!(tree_step(&mut open, true));
        assert_eq!(
            open.as_ref().expect("open").ring.selected(),
            Some(Handle(2)),
            "→ enters a's first child"
        );
        // b's parent's first child is b2.
        open.as_mut().expect("open").ring.select(Handle(3));
        assert!(tree_step(&mut open, true));
        assert_eq!(
            open.as_ref().expect("open").ring.selected(),
            Some(Handle(4))
        );

        // A leaf never enters. The lone single-window fixture:
        let leaves = snapshot(
            vec![
                app_toplevel(1, "app.a", &[1], State::ACTIVATED, 30),
                app_toplevel(2, "app.b", &[1], State::default(), 20),
            ],
            Some(1),
        );
        let mut flat_tree = None;
        session_step(
            &mut flat_tree,
            &leaves,
            Direction::Next,
            Scope::CurrentWorkspace,
            false,
            Palette::dark(),
            DEFAULT_ROWS,
            true,
        );
        assert!(!tree_step(&mut flat_tree, true));
        assert_eq!(
            flat_tree.as_ref().expect("open").ring.selected(),
            Some(Handle(2)),
            "→ on a leaf moves nothing"
        );
    }

    #[test]
    fn exit_parent_returns_to_the_parent_and_no_ops_on_top_level() {
        // ← on a child lands on its parent; on a parent or leaf — a
        // top-level row — the W3C root rule makes it a no-op.
        let snap = grouped_two_apps();
        let mut open = None;
        session_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            false,
            Palette::dark(),
            DEFAULT_ROWS,
            true,
        );
        open.as_mut().expect("opened").ring.select(Handle(2)); // a's child
        assert!(tree_step(&mut open, false));
        assert_eq!(
            open.as_ref().expect("open").ring.selected(),
            Some(Handle(1)),
            "← exits to a's parent"
        );
        open.as_mut().expect("open").ring.select(Handle(4)); // b's child
        assert!(tree_step(&mut open, false));
        assert_eq!(
            open.as_ref().expect("open").ring.selected(),
            Some(Handle(3))
        );

        // Top-level rows never exit.
        open.as_mut().expect("open").ring.select(Handle(3)); // b's parent
        assert!(!tree_step(&mut open, false));
        assert_eq!(
            open.as_ref().expect("open").ring.selected(),
            Some(Handle(3)),
            "← on a parent moves nothing"
        );
    }

    #[test]
    fn same_app_sessions_stay_flat_under_grouping() {
        // The exemption: a same-app session is already app-scoped, so
        // grouping never re-partitions it — no tree, the flat app ring.
        let snap = same_app_snapshot();
        let mut open = None;
        session_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            true,
            Palette::dark(),
            DEFAULT_ROWS,
            true,
        );
        let session = open.as_ref().expect("session opened");
        assert!(session.tree.is_none(), "same-app sessions carry no tree");
        assert_eq!(session.ring.selected(), Some(Handle(3)));
    }

    #[test]
    fn the_grouped_tree_freezes_at_open() {
        // A window of a new app activating mid-gesture changes nothing:
        // the frozen tree has no parent for it, the frozen ring holds no
        // handle for it — regrouping waits for the next session.
        let snap = grouped_two_apps();
        let mut open = None;
        session_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            false,
            Palette::dark(),
            DEFAULT_ROWS,
            true,
        );
        open.as_mut().expect("opened").ring.select(Handle(3)); // b's parent

        let mut drifted = grouped_two_apps();
        drifted
            .toplevels
            .push(app_toplevel(5, "app.c", &[1], State::ACTIVATED, 99));
        assert!(step_parent(&mut open, Direction::Next));
        assert_eq!(
            open.as_ref().expect("open").ring.selected(),
            Some(Handle(1)),
            "Tab wraps within the frozen parents — app.c is unreachable"
        );
        assert_eq!(
            open.as_mut().expect("open").ring.select(Handle(5)),
            None,
            "a window opened mid-gesture cannot be selected into the session"
        );
    }
}
