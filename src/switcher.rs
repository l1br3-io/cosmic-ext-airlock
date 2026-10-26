//! Pure switcher types shared by the wayland layer and the switcher core.
//!
//! No wayland or cctk imports ever live in this file: protocol data is
//! translated into these plain types at the `wayland` boundary so all
//! switching logic stays unit-testable without a compositor.

use std::collections::HashSet;

/// Opaque identifier for a toplevel, assigned by the wayland layer and stable
/// for the daemon's connection lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Handle(pub u64);

/// Opaque identifier for a workspace, same lifetime rules as [`Handle`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct WorkspaceId(pub u64);

/// Toplevel state flags mirroring the `zcosmic_toplevel_handle_v1` states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct State(u8);

impl State {
    pub const MAXIMIZED: Self = Self(1 << 0);
    pub const MINIMIZED: Self = Self(1 << 1);
    pub const ACTIVATED: Self = Self(1 << 2);
    pub const FULLSCREEN: Self = Self(1 << 3);
    pub const STICKY: Self = Self(1 << 4);

    pub fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

/// A toplevel as plain data.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Toplevel {
    pub handle: Handle,
    pub title: String,
    pub app_id: String,
    pub state: State,
    /// Workspaces this toplevel is visible on; sticky toplevels are visible
    /// on all of them.
    pub workspaces: HashSet<WorkspaceId>,
    /// Daemon-maintained MRU tick; 0 until an activation is observed.
    pub last_activated: u64,
}

/// A workspace as plain data.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Workspace {
    pub id: WorkspaceId,
    pub name: String,
    pub coordinates: Vec<u32>,
}

/// Point-in-time view of compositor state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub toplevels: Vec<Toplevel>,
    pub workspaces: Vec<Workspace>,
    pub active: Option<WorkspaceId>,
}

/// Which windows the switcher lists when invoked on a workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Scope {
    /// Only windows visible on the invoking workspace (sticky included) —
    /// the v1 behavior and the default for workspaces without a setting.
    #[default]
    CurrentWorkspace,
    /// Every non-minimized window across all workspaces, MRU order.
    All,
}

/// Windows eligible for switching on `active` under `scope`: visible there
/// for [`Scope::CurrentWorkspace`] (sticky counts — it is visible on every
/// workspace per zcosmic_toplevel_info_v1), or anywhere at all for
/// [`Scope::All`].
///
/// Returns handles in MRU order (most recently activated first); ties keep
/// snapshot order.
pub fn eligible(toplevels: &[Toplevel], active: WorkspaceId, scope: Scope) -> Vec<Handle> {
    let mut matches: Vec<&Toplevel> = toplevels
        .iter()
        .filter(|toplevel| is_switchable(toplevel, active, scope))
        .collect();
    // Stable sort: MRU ties keep snapshot order.
    matches.sort_by_key(|toplevel| std::cmp::Reverse(toplevel.last_activated));
    matches
        .into_iter()
        .map(|toplevel| toplevel.handle)
        .collect()
}

/// Whether one toplevel can be switched to on `active` under `scope`: not
/// minimized, and — for [`Scope::CurrentWorkspace`] — visible there,
/// directly or by being sticky. The per-window half of [`eligible`],
/// reused by the daemon to re-validate a selection against live state
/// mid-gesture.
pub fn is_switchable(toplevel: &Toplevel, active: WorkspaceId, scope: Scope) -> bool {
    !toplevel.state.contains(State::MINIMIZED)
        && match scope {
            Scope::CurrentWorkspace => {
                toplevel.state.contains(State::STICKY) || toplevel.workspaces.contains(&active)
            }
            Scope::All => true,
        }
}

/// Whether `handle` names a window that can be switched to right now:
/// present in `snapshot` and [`is_switchable`] on the active workspace
/// under `scope`. A missing active workspace switches to nothing under
/// every scope.
pub fn switchable(snapshot: &Snapshot, handle: Handle, scope: Scope) -> bool {
    let Some(active) = snapshot.active else {
        return false;
    };
    snapshot
        .toplevels
        .iter()
        .any(|toplevel| toplevel.handle == handle && is_switchable(toplevel, active, scope))
}

/// A selection ring over the MRU-ordered eligible list: `next`/`previous`
/// move the selection with wrap-around, and an empty ring selects nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ring {
    handles: Vec<Handle>,
    index: usize,
}

impl Ring {
    pub fn new(handles: Vec<Handle>) -> Self {
        Self { handles, index: 0 }
    }

    /// Number of windows in the ring.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "the overlay renders the ring length in task T5")
    )]
    pub fn len(&self) -> usize {
        self.handles.len()
    }

    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "the overlay renders the ring length in task T5")
    )]
    pub fn is_empty(&self) -> bool {
        self.handles.is_empty()
    }

    /// The currently selected handle, or `None` when the ring is empty.
    pub fn selected(&self) -> Option<Handle> {
        self.handles.get(self.index).copied()
    }

    /// Advances the selection (wrapping) and returns it.
    pub fn next(&mut self) -> Option<Handle> {
        self.step(1)
    }

    /// Retreats the selection (wrapping) and returns it.
    pub fn previous(&mut self) -> Option<Handle> {
        self.step(self.handles.len().checked_sub(1)?)
    }

    fn step(&mut self, delta: usize) -> Option<Handle> {
        let len = self.handles.len();
        if len == 0 {
            return None;
        }
        self.index = (self.index + delta) % len;
        self.selected()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_union_combines_flags() {
        let combined = State::MINIMIZED.union(State::ACTIVATED);
        assert!(combined.contains(State::MINIMIZED));
        assert!(combined.contains(State::ACTIVATED));
        assert!(!combined.contains(State::MAXIMIZED));
    }

    #[test]
    fn state_union_is_idempotent() {
        let once = State::FULLSCREEN.union(State::STICKY);
        let twice = once.union(State::FULLSCREEN);
        assert_eq!(once, twice);
    }

    #[test]
    fn empty_state_contains_nothing() {
        assert!(!State::default().contains(State::MINIMIZED));
        assert!(!State::default().contains(State::MAXIMIZED));
    }

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

    #[test]
    fn eligible_keeps_only_toplevels_on_the_active_workspace() {
        let toplevels = vec![
            toplevel(1, &[1], State::default(), 0),
            toplevel(2, &[2], State::default(), 0),
            toplevel(3, &[3], State::default(), 0),
        ];
        let handles = eligible(&toplevels, WorkspaceId(2), Scope::CurrentWorkspace);
        assert_eq!(handles, vec![Handle(2)]);
    }

    #[test]
    fn eligible_includes_multi_workspace_toplevels() {
        let toplevels = vec![
            toplevel(1, &[1, 2, 3], State::default(), 0),
            toplevel(2, &[1], State::default(), 0),
        ];
        let handles = eligible(&toplevels, WorkspaceId(3), Scope::CurrentWorkspace);
        assert_eq!(handles, vec![Handle(1)]);
    }

    #[test]
    fn eligible_includes_sticky_toplevels_on_any_workspace() {
        // Sticky windows are visible everywhere, even if the compositor does
        // not list every workspace in their workspace set.
        let toplevels = vec![toplevel(1, &[1], State::STICKY, 0)];
        let handles = eligible(&toplevels, WorkspaceId(4), Scope::CurrentWorkspace);
        assert_eq!(handles, vec![Handle(1)]);
    }

    #[test]
    fn eligible_excludes_minimized_toplevels() {
        let toplevels = vec![
            toplevel(1, &[1], State::MINIMIZED, 0),
            toplevel(2, &[1], State::default(), 0),
        ];
        let handles = eligible(&toplevels, WorkspaceId(1), Scope::CurrentWorkspace);
        assert_eq!(handles, vec![Handle(2)]);
    }

    #[test]
    fn eligible_orders_by_most_recently_activated() {
        let toplevels = vec![
            toplevel(1, &[1], State::default(), 10),
            toplevel(2, &[1], State::default(), 30),
            toplevel(3, &[1], State::default(), 20),
        ];
        let handles = eligible(&toplevels, WorkspaceId(1), Scope::CurrentWorkspace);
        assert_eq!(handles, vec![Handle(2), Handle(3), Handle(1)]);
    }

    #[test]
    fn eligible_breaks_mru_ties_by_snapshot_order() {
        let toplevels = vec![
            toplevel(1, &[1], State::default(), 7),
            toplevel(2, &[1], State::default(), 7),
        ];
        let handles = eligible(&toplevels, WorkspaceId(1), Scope::CurrentWorkspace);
        assert_eq!(handles, vec![Handle(1), Handle(2)]);
    }

    #[test]
    fn eligible_on_no_matching_toplevels_is_empty() {
        let toplevels = vec![toplevel(1, &[2], State::default(), 0)];
        assert!(eligible(&toplevels, WorkspaceId(1), Scope::CurrentWorkspace).is_empty());
    }

    #[test]
    fn all_scope_lists_cross_workspace_toplevels_in_mru_order() {
        let toplevels = vec![
            toplevel(1, &[1], State::default(), 5),
            // Sticky windows are visible everywhere, so All lists them like
            // any other window.
            toplevel(2, &[1], State::STICKY, 40),
            toplevel(3, &[2], State::default(), 30),
            toplevel(4, &[1], State::default(), 20),
        ];
        let handles = eligible(&toplevels, WorkspaceId(1), Scope::All);
        assert_eq!(handles, vec![Handle(2), Handle(3), Handle(4), Handle(1)]);
    }

    #[test]
    fn all_scope_excludes_minimized_toplevels() {
        let toplevels = vec![
            toplevel(1, &[2], State::MINIMIZED, 30),
            toplevel(2, &[1], State::default(), 10),
        ];
        let handles = eligible(&toplevels, WorkspaceId(1), Scope::All);
        assert_eq!(handles, vec![Handle(2)]);
    }

    #[test]
    fn ring_selects_first_item_initially() {
        let ring = Ring::new(vec![Handle(1), Handle(2), Handle(3)]);
        assert_eq!(ring.selected(), Some(Handle(1)));
        assert_eq!(ring.len(), 3);
        assert!(!ring.is_empty());
    }

    #[test]
    fn ring_next_advances_and_wraps_around() {
        let mut ring = Ring::new(vec![Handle(1), Handle(2), Handle(3)]);
        assert_eq!(ring.next(), Some(Handle(2)));
        assert_eq!(ring.next(), Some(Handle(3)));
        assert_eq!(ring.next(), Some(Handle(1)));
    }

    #[test]
    fn ring_previous_retreats_and_wraps_around() {
        let mut ring = Ring::new(vec![Handle(1), Handle(2), Handle(3)]);
        assert_eq!(ring.previous(), Some(Handle(3)));
        assert_eq!(ring.previous(), Some(Handle(2)));
        assert_eq!(ring.previous(), Some(Handle(1)));
    }

    #[test]
    fn ring_on_single_item_returns_itself_in_both_directions() {
        let mut ring = Ring::new(vec![Handle(9)]);
        assert_eq!(ring.next(), Some(Handle(9)));
        assert_eq!(ring.previous(), Some(Handle(9)));
    }

    #[test]
    fn ring_on_empty_list_selects_nothing() {
        let mut ring = Ring::new(vec![]);
        assert!(ring.is_empty());
        assert_eq!(ring.len(), 0);
        assert_eq!(ring.selected(), None);
        assert_eq!(ring.next(), None);
        assert_eq!(ring.previous(), None);
    }

    #[test]
    fn is_switchable_requires_presence_on_the_workspace() {
        let on_workspace = toplevel(1, &[1], State::default(), 0);
        assert!(is_switchable(
            &on_workspace,
            WorkspaceId(1),
            Scope::CurrentWorkspace
        ));
        assert!(!is_switchable(
            &on_workspace,
            WorkspaceId(2),
            Scope::CurrentWorkspace
        ));
    }

    #[test]
    fn is_switchable_accepts_sticky_windows_on_any_workspace() {
        let sticky = toplevel(1, &[1], State::STICKY, 0);
        assert!(is_switchable(
            &sticky,
            WorkspaceId(9),
            Scope::CurrentWorkspace
        ));
    }

    #[test]
    fn is_switchable_rejects_minimized_windows() {
        let minimized = toplevel(1, &[1], State::MINIMIZED, 0);
        assert!(!is_switchable(
            &minimized,
            WorkspaceId(1),
            Scope::CurrentWorkspace
        ));
    }

    #[test]
    fn switchable_names_eligible_windows_of_the_active_workspace() {
        let snapshot = Snapshot {
            toplevels: vec![
                toplevel(1, &[1], State::default(), 0),
                toplevel(2, &[2], State::default(), 0),
                toplevel(3, &[1], State::MINIMIZED, 0),
            ],
            workspaces: Vec::new(),
            active: Some(WorkspaceId(1)),
        };
        assert!(switchable(&snapshot, Handle(1), Scope::CurrentWorkspace));
        // Parked on another workspace.
        assert!(!switchable(&snapshot, Handle(2), Scope::CurrentWorkspace));
        // Minimized on this one.
        assert!(!switchable(&snapshot, Handle(3), Scope::CurrentWorkspace));
        // Closed since the session opened.
        assert!(!switchable(&snapshot, Handle(99), Scope::CurrentWorkspace));
    }

    #[test]
    fn switchable_without_an_active_workspace_is_false() {
        let snapshot = Snapshot {
            toplevels: vec![toplevel(1, &[1], State::default(), 0)],
            workspaces: Vec::new(),
            active: None,
        };
        assert!(!switchable(&snapshot, Handle(1), Scope::CurrentWorkspace));
    }

    #[test]
    fn switchable_accepts_cross_workspace_handles_only_under_all() {
        let snapshot = Snapshot {
            toplevels: vec![toplevel(1, &[2], State::default(), 0)],
            workspaces: Vec::new(),
            active: Some(WorkspaceId(1)),
        };
        assert!(switchable(&snapshot, Handle(1), Scope::All));
        assert!(!switchable(&snapshot, Handle(1), Scope::CurrentWorkspace));
    }

    #[test]
    fn switchable_without_an_active_workspace_selects_nothing_under_either_scope() {
        let snapshot = Snapshot {
            toplevels: vec![toplevel(1, &[1], State::default(), 0)],
            workspaces: Vec::new(),
            active: None,
        };
        assert!(!switchable(&snapshot, Handle(1), Scope::CurrentWorkspace));
        assert!(!switchable(&snapshot, Handle(1), Scope::All));
    }
}
