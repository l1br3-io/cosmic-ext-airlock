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

/// The app a same-app session on `active` under `scope` anchors to: the
/// app_id of the frontmost eligible window in MRU order. An empty app_id
/// never anchors — windows the compositor cannot attribute never group —
/// so anchoring walks down the MRU list to the first attributable window.
pub fn app_anchor(toplevels: &[Toplevel], active: WorkspaceId, scope: Scope) -> Option<String> {
    eligible(toplevels, active, scope)
        .iter()
        .find_map(|handle| {
            toplevels
                .iter()
                .find(|t| t.handle == *handle && !t.app_id.is_empty())
                .map(|t| t.app_id.clone())
        })
}

/// A same-app session's view of `snapshot`: the toplevels [`eligible`]
/// would list on the active workspace under `scope`, narrowed to `app`.
///
/// The result is an ordinary snapshot, so the ring, overlay, and commit
/// paths treat it like any other. Order follows [`eligible`] exactly (MRU
/// first, ties keep snapshot order); an empty `app` — like an empty
/// app_id — groups nothing.
pub fn app_view(snapshot: &Snapshot, app: &str, scope: Scope) -> Snapshot {
    let Some(active) = snapshot.active else {
        return Snapshot {
            toplevels: Vec::new(),
            workspaces: snapshot.workspaces.clone(),
            active: None,
        };
    };
    let toplevels = eligible(&snapshot.toplevels, active, scope)
        .iter()
        .filter_map(|handle| {
            snapshot
                .toplevels
                .iter()
                .find(|t| t.handle == *handle && !t.app_id.is_empty() && t.app_id == app)
                .cloned()
        })
        .collect();
    Snapshot {
        toplevels,
        workspaces: snapshot.workspaces.clone(),
        active: snapshot.active,
    }
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

    /// The currently selected handle, or `None` when the ring is empty.
    pub fn selected(&self) -> Option<Handle> {
        self.handles.get(self.index).copied()
    }

    /// Retreats the selection (wrapping) and returns it.
    pub fn previous(&mut self) -> Option<Handle> {
        self.step(self.handles.len().checked_sub(1)?)
    }

    /// Advances the selection by `delta` (wrapping) and returns it.
    pub fn step(&mut self, delta: usize) -> Option<Handle> {
        let len = self.handles.len();
        if len == 0 {
            return None;
        }
        self.index = (self.index + delta) % len;
        self.selected()
    }
}

#[cfg(test)]
impl Ring {
    /// Number of windows in the ring (test assertions).
    pub fn len(&self) -> usize {
        self.handles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.handles.is_empty()
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

    /// Like [`toplevel`], with the app under test's choosing.
    fn app_window(
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

    #[test]
    fn app_anchor_picks_the_front_eligible_windows_app() {
        let toplevels = vec![
            app_window(1, "editor", &[1], State::ACTIVATED, 30),
            app_window(2, "browser", &[1], State::default(), 20),
        ];
        assert_eq!(
            app_anchor(&toplevels, WorkspaceId(1), Scope::CurrentWorkspace),
            Some("editor".to_string())
        );
    }

    #[test]
    fn app_anchor_skips_windows_the_compositor_cannot_attribute() {
        // MRU order is 1 then 2; the empty app_id never anchors, so the
        // front attributable window's app wins.
        let toplevels = vec![
            app_window(1, "", &[1], State::ACTIVATED, 30),
            app_window(2, "editor", &[1], State::default(), 20),
        ];
        assert_eq!(
            app_anchor(&toplevels, WorkspaceId(1), Scope::CurrentWorkspace),
            Some("editor".to_string())
        );
    }

    #[test]
    fn app_anchor_is_none_when_no_window_has_an_app() {
        let toplevels = vec![
            app_window(1, "", &[1], State::ACTIVATED, 30),
            app_window(2, "", &[1], State::default(), 20),
        ];
        assert_eq!(
            app_anchor(&toplevels, WorkspaceId(1), Scope::CurrentWorkspace),
            None
        );
    }

    #[test]
    fn app_anchor_follows_the_scope_not_just_recency() {
        // The MRU-front window lives on another workspace: under the v1
        // scope the anchor comes from this workspace's front window.
        let toplevels = vec![
            app_window(1, "browser", &[2], State::default(), 40),
            app_window(2, "editor", &[1], State::ACTIVATED, 30),
            app_window(3, "editor", &[1], State::default(), 20),
        ];
        assert_eq!(
            app_anchor(&toplevels, WorkspaceId(1), Scope::CurrentWorkspace),
            Some("editor".to_string())
        );
        assert_eq!(
            app_anchor(&toplevels, WorkspaceId(1), Scope::All),
            Some("browser".to_string())
        );
    }

    #[test]
    fn app_view_lists_only_the_anchored_apps_eligible_windows_in_mru_order() {
        let snapshot = Snapshot {
            toplevels: vec![
                app_window(1, "editor", &[1], State::ACTIVATED, 30),
                app_window(2, "browser", &[1], State::default(), 25),
                app_window(3, "editor", &[1], State::default(), 20),
                // Same app, but minimized and parked away: never listed.
                app_window(4, "editor", &[1], State::MINIMIZED, 15),
                app_window(5, "editor", &[2], State::default(), 10),
            ],
            workspaces: Vec::new(),
            active: Some(WorkspaceId(1)),
        };
        let view = app_view(&snapshot, "editor", Scope::CurrentWorkspace);
        let handles: Vec<Handle> = view.toplevels.iter().map(|t| t.handle).collect();
        assert_eq!(handles, vec![Handle(1), Handle(3)]);
    }

    #[test]
    fn app_view_keeps_sticky_app_windows_from_other_workspaces() {
        let snapshot = Snapshot {
            toplevels: vec![
                app_window(1, "editor", &[1], State::ACTIVATED, 30),
                // Sticky editor window parked on workspace 2 — visible
                // everywhere, exactly like any other sticky window.
                app_window(2, "editor", &[2], State::STICKY, 20),
            ],
            workspaces: Vec::new(),
            active: Some(WorkspaceId(1)),
        };
        let view = app_view(&snapshot, "editor", Scope::CurrentWorkspace);
        let handles: Vec<Handle> = view.toplevels.iter().map(|t| t.handle).collect();
        assert_eq!(handles, vec![Handle(1), Handle(2)]);
    }

    #[test]
    fn app_view_under_all_scope_crosses_workspaces() {
        let snapshot = Snapshot {
            toplevels: vec![
                app_window(1, "editor", &[1], State::ACTIVATED, 30),
                app_window(2, "editor", &[2], State::default(), 20),
            ],
            workspaces: Vec::new(),
            active: Some(WorkspaceId(1)),
        };
        let confined = app_view(&snapshot, "editor", Scope::CurrentWorkspace);
        assert_eq!(confined.toplevels.len(), 1);
        let everywhere = app_view(&snapshot, "editor", Scope::All);
        assert_eq!(everywhere.toplevels.len(), 2);
    }

    #[test]
    fn a_single_window_app_yields_a_one_entry_view() {
        let snapshot = Snapshot {
            toplevels: vec![
                app_window(1, "editor", &[1], State::ACTIVATED, 30),
                app_window(2, "browser", &[1], State::default(), 20),
            ],
            workspaces: Vec::new(),
            active: Some(WorkspaceId(1)),
        };
        let view = app_view(&snapshot, "editor", Scope::CurrentWorkspace);
        assert_eq!(view.toplevels.len(), 1);
    }

    #[test]
    fn app_view_excludes_windows_the_compositor_cannot_attribute() {
        let snapshot = Snapshot {
            toplevels: vec![
                app_window(1, "editor", &[1], State::ACTIVATED, 30),
                app_window(2, "", &[1], State::default(), 20),
            ],
            workspaces: Vec::new(),
            active: Some(WorkspaceId(1)),
        };
        // Even an empty anchor groups nothing.
        for app in ["editor", ""] {
            let view = app_view(&snapshot, app, Scope::CurrentWorkspace);
            let listed = view
                .toplevels
                .iter()
                .filter(|t| t.app_id.is_empty())
                .count();
            assert_eq!(listed, 0, "empty app_id grouped under anchor {app:?}");
        }
    }

    #[test]
    fn app_view_without_an_active_workspace_is_empty() {
        let snapshot = Snapshot {
            toplevels: vec![app_window(1, "editor", &[1], State::ACTIVATED, 30)],
            workspaces: Vec::new(),
            active: None,
        };
        let view = app_view(&snapshot, "editor", Scope::CurrentWorkspace);
        assert!(view.toplevels.is_empty());
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
        assert_eq!(ring.step(1), Some(Handle(2)));
        assert_eq!(ring.step(1), Some(Handle(3)));
        assert_eq!(ring.step(1), Some(Handle(1)));
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
        assert_eq!(ring.step(1), Some(Handle(9)));
        assert_eq!(ring.previous(), Some(Handle(9)));
    }

    #[test]
    fn ring_on_empty_list_selects_nothing() {
        let mut ring = Ring::new(vec![]);
        assert!(ring.is_empty());
        assert_eq!(ring.len(), 0);
        assert_eq!(ring.selected(), None);
        assert_eq!(ring.step(1), None);
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
