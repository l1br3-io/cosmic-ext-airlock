//! Pure switcher types shared by the wayland layer and the switcher core.
//!
//! No wayland or cctk imports ever live in this file: protocol data is
//! translated into these plain types at the `wayland` boundary so all
//! switching logic stays unit-testable without a compositor.

use std::collections::{HashMap, HashSet};

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

    /// Moves the selection to `handle` when the ring holds it, returning
    /// it. A handle the frozen ring does not hold — a window opened after
    /// the session froze its list — selects nothing and leaves the ring
    /// exactly as it was; an empty ring selects nothing.
    pub fn select(&mut self, handle: Handle) -> Option<Handle> {
        let index = self.handles.iter().position(|&held| held == handle)?;
        self.index = index;
        Some(handle)
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

/// A row's place in the grouped tree.
//
// The grouped-mode API lands unconsumed, the LV1/S1 precedent: dead in
// the binary until the grouped overlay consumes it, exercised by the
// tests here and public in the dev lib target — an `allow`, not an
// `expect`, for exactly that reason (see `ui::Gesture::showing`).
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    /// A top-level row nothing nests under: a single-window app (the W3C
    /// end-node rule — no chevron, no count) or an un-attributable
    /// window, which never groups.
    Leaf,
    /// A multi-window app's top-level row, immediately followed by its
    /// [`RowKind::Child`] rows.
    Parent,
    /// A window nested under the nearest preceding parent of its app.
    Child,
}

/// One row of the grouped switcher list: the window the row commits to,
/// plus its place in the app tree.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupedRow {
    /// The window this row resolves to at commit time. A parent carries
    /// its app's MRU-front eligible window — GNOME's `cachedWindows[0]`
    /// semantics — so Alt-release on a parent row activates that window.
    pub handle: Handle,
    /// The app the row belongs to: the grouping key for parents and
    /// children, empty for un-attributable top-level leaves.
    pub app_id: String,
    pub kind: RowKind,
}

/// The eligible list re-partitioned into an app → windows tree, flattened
/// to the all-expanded row sequence: parent rows in MRU first-appearance
/// order, each followed by its children in MRU order, top-level leaves at
/// their own first-appearance positions. All parents expanded is the v1
/// model — no collapse state exists, and the sequence freezes at open
/// exactly as the flat list does.
///
/// Windows of `app` in [`eligible`] order, or the empty count it never
/// gets: an empty app_id groups nothing (the [`app_anchor`] rule), so
/// unattributable windows stay top-level leaf rows.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grouping {
    rows: Vec<GroupedRow>,
}

#[cfg_attr(not(test), allow(dead_code))]
impl Grouping {
    /// Partitions [`eligible`]'s MRU list on `active` under `scope` into
    /// the grouped row sequence.
    pub fn new(toplevels: &[Toplevel], active: WorkspaceId, scope: Scope) -> Self {
        let handles = eligible(toplevels, active, scope);
        // App of each eligible handle, parallel to `handles`; empty for
        // windows the compositor cannot attribute.
        let apps: Vec<String> = handles
            .iter()
            .map(|&handle| {
                toplevels
                    .iter()
                    .find(|t| t.handle == handle)
                    .map_or_else(String::new, |t| t.app_id.clone())
            })
            .collect();
        // Windows per app, empty app_ids excluded — they never count
        // toward parenthood.
        let mut windows_per_app: HashMap<&str, usize> = HashMap::new();
        for app in &apps {
            if !app.is_empty() {
                *windows_per_app.entry(app.as_str()).or_default() += 1;
            }
        }
        let mut emitted: HashSet<&str> = HashSet::new();
        let mut rows = Vec::new();
        for (&handle, app) in handles.iter().zip(&apps) {
            if app.is_empty() || windows_per_app[app.as_str()] == 1 {
                // An un-attributable window or a single-window app: a
                // top-level leaf either way.
                rows.push(GroupedRow {
                    handle,
                    app_id: app.clone(),
                    kind: RowKind::Leaf,
                });
            } else if emitted.insert(app.as_str()) {
                // First appearance fronts its app's block; the parent
                // carries this, the MRU-front window. The app's other
                // windows follow as children, MRU within the parent.
                rows.push(GroupedRow {
                    handle,
                    app_id: app.clone(),
                    kind: RowKind::Parent,
                });
                for (&sibling, sibling_app) in handles.iter().zip(&apps) {
                    if sibling != handle && sibling_app == app {
                        rows.push(GroupedRow {
                            handle: sibling,
                            app_id: app.clone(),
                            kind: RowKind::Child,
                        });
                    }
                }
            }
            // Any other window of an already-emitted app went out with
            // its parent — nothing to push here.
        }
        Self { rows }
    }

    /// The flattened row sequence — parents each followed by their
    /// children, top-level leaves at their own positions — the sequence
    /// ↑/↓ stepping and the `rows` budget apply over.
    pub fn rows(&self) -> &[GroupedRow] {
        &self.rows
    }

    /// Index of the next [`RowKind::Parent`] row strictly after `from`,
    /// wrapping past the end — Tab's step, skipping the current app's
    /// children. `None` when no parent exists or `from` is out of range.
    pub fn next_parent(&self, from: usize) -> Option<usize> {
        let len = self.rows.len();
        if from >= len {
            return None;
        }
        (1..=len)
            .map(|offset| (from + offset) % len)
            .find(|&index| self.rows[index].kind == RowKind::Parent)
    }

    /// Index of the previous [`RowKind::Parent`] row strictly before
    /// `from`, wrapping past the front — Shift+Tab's step. `None` when no
    /// parent exists or `from` is out of range.
    pub fn previous_parent(&self, from: usize) -> Option<usize> {
        let len = self.rows.len();
        if from >= len {
            return None;
        }
        (1..=len)
            .map(|offset| (from + len - offset) % len)
            .find(|&index| self.rows[index].kind == RowKind::Parent)
    }

    /// Index of the parent row the child at `from` nests under — ←'s
    /// exit-to-parent. `None` for parents, leaves, and out-of-range
    /// positions: only a [`RowKind::Child`] has a parent, and children
    /// sit contiguously under theirs, so the nearest preceding parent
    /// row is the one.
    pub fn parent_of(&self, child: usize) -> Option<usize> {
        if self.rows.get(child)?.kind != RowKind::Child {
            return None;
        }
        self.rows[..child]
            .iter()
            .rposition(|row| row.kind == RowKind::Parent)
    }

    /// Index of `parent`'s first child — the row right after it, since a
    /// parent's children are contiguous — →'s enter. `None` for leaves
    /// and out-of-range positions; a parent by construction always has
    /// at least one child.
    pub fn first_child(&self, parent: usize) -> Option<usize> {
        if self.rows.get(parent)?.kind != RowKind::Parent {
            return None;
        }
        let child = parent + 1;
        (self.rows.get(child)?.kind == RowKind::Child).then_some(child)
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
    fn select_moves_the_selection_to_a_handle_the_ring_holds() {
        let mut ring = Ring::new(vec![Handle(1), Handle(2), Handle(3)]);
        assert_eq!(ring.select(Handle(3)), Some(Handle(3)));
        assert_eq!(ring.selected(), Some(Handle(3)));
        // Stepping continues from the moved selection.
        assert_eq!(ring.step(1), Some(Handle(1)));
    }

    #[test]
    fn select_on_a_handle_the_ring_never_held_selects_nothing_and_changes_nothing() {
        // The ring freezes when the session opens; a window opened after
        // that is not selectable into the session.
        let mut ring = Ring::new(vec![Handle(1), Handle(2)]);
        ring.step(1); // selected Handle(2)
        assert_eq!(ring.select(Handle(9)), None);
        assert_eq!(ring.selected(), Some(Handle(2)));
        assert_eq!(ring.step(1), Some(Handle(1)));
    }

    #[test]
    fn select_on_an_empty_ring_selects_nothing() {
        let mut ring = Ring::new(vec![]);
        assert_eq!(ring.select(Handle(1)), None);
        assert_eq!(ring.selected(), None);
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

    // --- Grouping ---------------------------------------------------------

    /// Handles+app_ids of a grouped row sequence, in row order — the
    /// shape contract asserted throughout the grouping tests.
    fn shape(grouping: &Grouping) -> Vec<(RowKind, Handle)> {
        grouping
            .rows()
            .iter()
            .map(|row| (row.kind, row.handle))
            .collect()
    }

    #[test]
    fn grouped_partitions_multi_window_apps_into_parent_then_children() {
        // MRU order: a1, b1, a2, c1, a3, b2. Parents appear in MRU
        // first-appearance order (a, then b); children follow their
        // parent MRU within the app; single-window c is a leaf.
        let toplevels = vec![
            app_window(1, "a", &[1], State::ACTIVATED, 30),
            app_window(2, "b", &[1], State::default(), 25),
            app_window(3, "a", &[1], State::default(), 20),
            app_window(4, "c", &[1], State::default(), 15),
            app_window(5, "a", &[1], State::default(), 10),
            app_window(6, "b", &[1], State::default(), 5),
        ];
        let grouping = Grouping::new(&toplevels, WorkspaceId(1), Scope::CurrentWorkspace);
        assert_eq!(
            shape(&grouping),
            vec![
                (RowKind::Parent, Handle(1)),
                (RowKind::Child, Handle(3)),
                (RowKind::Child, Handle(5)),
                (RowKind::Parent, Handle(2)),
                (RowKind::Child, Handle(6)),
                (RowKind::Leaf, Handle(4)),
            ]
        );
    }

    #[test]
    fn grouped_leaves_windows_the_compositor_cannot_attribute_top_level() {
        // MRU: u1, a1, a2, u2. The empty app_ids never group — the
        // `app_anchor` rule — so both stay top-level leaves at their own
        // first-appearance positions; only a nests.
        let toplevels = vec![
            app_window(1, "", &[1], State::ACTIVATED, 40),
            app_window(2, "a", &[1], State::default(), 30),
            app_window(3, "a", &[1], State::default(), 20),
            app_window(4, "", &[1], State::default(), 10),
        ];
        let grouping = Grouping::new(&toplevels, WorkspaceId(1), Scope::CurrentWorkspace);
        assert_eq!(
            shape(&grouping),
            vec![
                (RowKind::Leaf, Handle(1)),
                (RowKind::Parent, Handle(2)),
                (RowKind::Child, Handle(3)),
                (RowKind::Leaf, Handle(4)),
            ]
        );
    }

    #[test]
    fn grouped_renders_single_window_apps_as_leaf_rows() {
        // One window of `a`, one of `solo`: neither earns a parent row —
        // the W3C end-node rule, no chevron and no count.
        let toplevels = vec![
            app_window(1, "a", &[1], State::ACTIVATED, 30),
            app_window(2, "solo", &[1], State::default(), 20),
        ];
        let grouping = Grouping::new(&toplevels, WorkspaceId(1), Scope::CurrentWorkspace);
        assert_eq!(
            shape(&grouping),
            vec![(RowKind::Leaf, Handle(1)), (RowKind::Leaf, Handle(2))]
        );
    }

    #[test]
    fn a_parent_row_carries_its_apps_mru_front_window() {
        // b's front window is handle 2 (MRU 25): committing the parent
        // row activates it — GNOME's `cachedWindows[0]` semantics — not
        // a later child.
        let toplevels = vec![
            app_window(1, "a", &[1], State::ACTIVATED, 30),
            app_window(2, "b", &[1], State::default(), 25),
            app_window(3, "b", &[1], State::default(), 20),
        ];
        let grouping = Grouping::new(&toplevels, WorkspaceId(1), Scope::CurrentWorkspace);
        let parent = &grouping.rows()[1];
        assert_eq!(parent.kind, RowKind::Parent);
        assert_eq!(parent.handle, Handle(2));
        assert_eq!(parent.app_id, "b");
    }

    #[test]
    fn grouped_children_break_mru_ties_by_snapshot_order() {
        // a2 and a3 tie on last_activated; children keep snapshot order
        // exactly as `eligible` does.
        let toplevels = vec![
            app_window(1, "a", &[1], State::ACTIVATED, 30),
            app_window(2, "a", &[1], State::default(), 7),
            app_window(3, "a", &[1], State::default(), 7),
        ];
        let grouping = Grouping::new(&toplevels, WorkspaceId(1), Scope::CurrentWorkspace);
        assert_eq!(
            shape(&grouping),
            vec![
                (RowKind::Parent, Handle(1)),
                (RowKind::Child, Handle(2)),
                (RowKind::Child, Handle(3)),
            ]
        );
    }

    #[test]
    fn grouped_partitions_follow_the_scope() {
        // a spans workspaces 1 and 2, b lives on 2 alone: under the v1
        // scope a's second window is out of view (one window, one leaf),
        // under All it counts and a grows a parent+child block with b's
        // leaf at its own first-appearance position.
        let toplevels = vec![
            app_window(1, "a", &[1], State::ACTIVATED, 30),
            app_window(2, "a", &[2], State::default(), 20),
            app_window(3, "b", &[2], State::default(), 25),
        ];
        let confined = Grouping::new(&toplevels, WorkspaceId(1), Scope::CurrentWorkspace);
        assert_eq!(shape(&confined), vec![(RowKind::Leaf, Handle(1))]);
        // All-scope MRU: a1, b, a2 — first appearance a then b.
        let everywhere = Grouping::new(&toplevels, WorkspaceId(1), Scope::All);
        assert_eq!(
            shape(&everywhere),
            vec![
                (RowKind::Parent, Handle(1)),
                (RowKind::Child, Handle(2)),
                (RowKind::Leaf, Handle(3)),
            ]
        );
    }

    #[test]
    fn grouped_on_an_empty_eligible_list_is_empty() {
        let toplevels = vec![app_window(1, "a", &[2], State::default(), 30)];
        let grouping = Grouping::new(&toplevels, WorkspaceId(1), Scope::CurrentWorkspace);
        assert!(grouping.rows().is_empty());
    }

    /// The grouped shape the walk tests step through: parent a + two
    /// children, parent b + one child, single-window leaf c.
    fn grouped_fixture() -> Grouping {
        let toplevels = vec![
            app_window(1, "a", &[1], State::ACTIVATED, 60),
            app_window(2, "a", &[1], State::default(), 50),
            app_window(3, "a", &[1], State::default(), 40),
            app_window(4, "b", &[1], State::default(), 30),
            app_window(5, "b", &[1], State::default(), 20),
            app_window(6, "c", &[1], State::default(), 10),
        ];
        Grouping::new(&toplevels, WorkspaceId(1), Scope::CurrentWorkspace)
    }

    #[test]
    fn next_parent_from_a_parent_steps_past_its_own_children() {
        // Tab from parent a lands on parent b, never back inside a's
        // children.
        let grouping = grouped_fixture();
        assert_eq!(grouping.next_parent(0), Some(3));
    }

    #[test]
    fn next_parent_from_a_child_or_leaf_finds_the_following_parent() {
        let grouping = grouped_fixture();
        assert_eq!(grouping.next_parent(2), Some(3), "last a-child → parent b");
        assert_eq!(grouping.next_parent(5), Some(0), "trailing leaf wraps to a");
    }

    #[test]
    fn next_parent_wraps_from_the_last_parent_to_the_first() {
        let grouping = grouped_fixture();
        assert_eq!(grouping.next_parent(3), Some(0));
    }

    #[test]
    fn next_parent_lands_only_on_parent_rows() {
        let grouping = grouped_fixture();
        for from in 0..grouping.rows().len() {
            let next = grouping.next_parent(from).expect("fixture has parents");
            assert_eq!(
                grouping.rows()[next].kind,
                RowKind::Parent,
                "next_parent({from}) landed on a non-parent"
            );
        }
    }

    #[test]
    fn previous_parent_wraps_from_the_first_parent_to_the_last() {
        let grouping = grouped_fixture();
        assert_eq!(grouping.previous_parent(0), Some(3));
    }

    #[test]
    fn previous_parent_from_a_child_or_leaf_finds_the_preceding_parent() {
        let grouping = grouped_fixture();
        assert_eq!(grouping.previous_parent(4), Some(3), "b-child → parent b");
        assert_eq!(
            grouping.previous_parent(5),
            Some(3),
            "trailing leaf → parent b"
        );
        assert_eq!(grouping.previous_parent(1), Some(0), "a-child → parent a");
    }

    #[test]
    fn previous_parent_lands_only_on_parent_rows() {
        let grouping = grouped_fixture();
        for from in 0..grouping.rows().len() {
            let previous = grouping.previous_parent(from).expect("fixture has parents");
            assert_eq!(
                grouping.rows()[previous].kind,
                RowKind::Parent,
                "previous_parent({from}) landed on a non-parent"
            );
        }
    }

    #[test]
    fn parent_walks_without_any_parents_are_none() {
        // All leaves: two single-window apps and an un-attributable
        // window — Tab has nowhere to step.
        let toplevels = vec![
            app_window(1, "a", &[1], State::ACTIVATED, 30),
            app_window(2, "b", &[1], State::default(), 20),
            app_window(3, "", &[1], State::default(), 10),
        ];
        let grouping = Grouping::new(&toplevels, WorkspaceId(1), Scope::CurrentWorkspace);
        assert_eq!(grouping.next_parent(0), None);
        assert_eq!(grouping.previous_parent(0), None);
    }

    #[test]
    fn parent_walks_from_out_of_range_positions_are_none() {
        let grouping = grouped_fixture();
        assert_eq!(grouping.next_parent(99), None);
        assert_eq!(grouping.previous_parent(99), None);
    }

    #[test]
    fn parent_of_a_child_is_the_row_that_heads_its_block() {
        let grouping = grouped_fixture();
        assert_eq!(grouping.parent_of(1), Some(0));
        assert_eq!(grouping.parent_of(2), Some(0));
        assert_eq!(grouping.parent_of(4), Some(3));
    }

    #[test]
    fn parent_of_a_parent_or_leaf_is_none() {
        // Only a child nests; parents and leaves are top-level.
        let grouping = grouped_fixture();
        assert_eq!(grouping.parent_of(0), None);
        assert_eq!(grouping.parent_of(3), None);
        assert_eq!(grouping.parent_of(5), None);
    }

    #[test]
    fn parent_of_an_out_of_range_position_is_none() {
        let grouping = grouped_fixture();
        assert_eq!(grouping.parent_of(9), None);
    }

    #[test]
    fn first_child_of_a_multi_window_parent_is_the_row_after_it() {
        // A parent's children are contiguous, so its first child sits
        // immediately below it — →'s target.
        let grouping = grouped_fixture();
        assert_eq!(grouping.first_child(0), Some(1));
        assert_eq!(grouping.first_child(3), Some(4));
    }

    #[test]
    fn first_child_of_a_leaf_is_none() {
        // The W3C end-node rule: → does nothing on a leaf.
        let grouping = grouped_fixture();
        assert_eq!(grouping.first_child(5), None);
    }

    #[test]
    fn first_child_of_an_out_of_range_position_is_none() {
        let grouping = grouped_fixture();
        assert_eq!(grouping.first_child(9), None);
    }

    #[test]
    fn walks_on_an_empty_grouping_are_none() {
        let grouping = Grouping::new(&[], WorkspaceId(1), Scope::CurrentWorkspace);
        assert_eq!(grouping.next_parent(0), None);
        assert_eq!(grouping.previous_parent(0), None);
        assert_eq!(grouping.parent_of(0), None);
        assert_eq!(grouping.first_child(0), None);
    }
}
