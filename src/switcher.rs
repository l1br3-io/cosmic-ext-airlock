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
    /// Every non-minimized window across all workspaces, workspace-major
    /// order (see [`eligible`]).
    All,
}

/// Positions of `workspaces` in workspace order — coordinates sorted
/// lexicographically, workspace id breaking ties — the order
/// `format_snapshot` lists by. (`settings::active_workspace` counts
/// positions in a coordinates-only sort; the two agree while coordinates
/// are distinct.) The grouping's comparator for this module; callers
/// hand it `&snapshot.workspaces`.
fn workspace_positions(workspaces: &[Workspace]) -> HashMap<WorkspaceId, usize> {
    let mut sorted: Vec<&Workspace> = workspaces.iter().collect();
    sorted.sort_by(|a, b| {
        a.coordinates
            .cmp(&b.coordinates)
            .then_with(|| a.id.cmp(&b.id))
    });
    sorted
        .into_iter()
        .enumerate()
        .map(|(position, workspace)| (workspace.id, position))
        .collect()
}

/// The workspace-major sort key of one toplevel: `(tier, workspace
/// position, Reverse(last_activated))`, ascending.
///
/// Tier 0 is the invoking workspace — members and sticky alike (sticky is
/// visible on every workspace, so it is part of this one by the same rule
/// `Scope::CurrentWorkspace` applies). Tier 1 is membership in some listed
/// workspace, keyed by the earliest such workspace's position (2026-11-14
/// decision 5: a window on several workspaces groups by the invoking
/// workspace if it is a member, else by its earliest member in workspace
/// order). Tier 2 is membership empty or entirely unlisted — not part of
/// the workspace, always at the bottom. MRU is the tiebreak everywhere, so
/// under `Scope::CurrentWorkspace` — where every eligible window is tier 0
/// — the key degenerates to pure MRU and the default view's output is
/// byte-identical (the degenerate case is the safety).
fn group_key(
    toplevel: &Toplevel,
    positions: &HashMap<WorkspaceId, usize>,
    active: WorkspaceId,
) -> (u8, usize, std::cmp::Reverse<u64>) {
    if toplevel.state.contains(State::STICKY) || toplevel.workspaces.contains(&active) {
        (0, 0, std::cmp::Reverse(toplevel.last_activated))
    } else {
        toplevel
            .workspaces
            .iter()
            .filter_map(|id| positions.get(id).copied())
            .min()
            .map_or(
                (2, 0, std::cmp::Reverse(toplevel.last_activated)),
                |position| (1, position, std::cmp::Reverse(toplevel.last_activated)),
            )
    }
}

/// Windows eligible for switching on `active` under `scope`: visible there
/// for [`Scope::CurrentWorkspace`] (sticky counts — it is visible on every
/// workspace per zcosmic_toplevel_info_v1), or anywhere at all for
/// [`Scope::All`].
///
/// Returns handles in the switcher's listing order, and every consumer —
/// ring, overlay, commit paths — treats it like any other. Under
/// [`Scope::CurrentWorkspace`] that order is plain MRU (most recently
/// activated first; ties keep snapshot order). Under [`Scope::All`] it is
/// workspace-major (2026-11-14): the invoking workspace's windows first
/// (sticky among them), the other workspaces in workspace order
/// (coordinates sorted, id breaking ties — the `format_snapshot`
/// convention), workspace-less windows last; MRU within each group, ties
/// keep snapshot order.
pub fn eligible(
    toplevels: &[Toplevel],
    workspaces: &[Workspace],
    active: WorkspaceId,
    scope: Scope,
) -> Vec<Handle> {
    let positions = workspace_positions(workspaces);
    let mut matches: Vec<&Toplevel> = toplevels
        .iter()
        .filter(|toplevel| is_switchable(toplevel, active, scope))
        .collect();
    // Stable sort: equal keys keep snapshot order, the existing
    // convention.
    matches.sort_by_key(|toplevel| group_key(toplevel, &positions, active));
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
/// app_id of the frontmost window of [`eligible`]'s order — the invoking
/// workspace's front under [`Scope::All`] (2026-11-14: here outranks
/// recent), the MRU front under [`Scope::CurrentWorkspace`]. An empty
/// app_id never anchors — windows the compositor cannot attribute never
/// group — so anchoring walks down the eligible list to the first
/// attributable window.
pub fn app_anchor(
    toplevels: &[Toplevel],
    workspaces: &[Workspace],
    active: WorkspaceId,
    scope: Scope,
) -> Option<String> {
    eligible(toplevels, workspaces, active, scope)
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
/// paths treat it like any other. Order follows [`eligible`] exactly
/// (workspace-major under [`Scope::All`], MRU otherwise); an empty `app` —
/// like an empty app_id — groups nothing.
pub fn app_view(snapshot: &Snapshot, app: &str, scope: Scope) -> Snapshot {
    let Some(active) = snapshot.active else {
        return Snapshot {
            toplevels: Vec::new(),
            workspaces: snapshot.workspaces.clone(),
            active: None,
        };
    };
    let toplevels = eligible(&snapshot.toplevels, &snapshot.workspaces, active, scope)
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

/// Windows of `handles` whose title or app_id contains `query`
/// (case-insensitive substring), in `handles`' order — the type-to-filter
/// narrow (`type-filter`).
///
/// The input is a *frozen* handle list — the session's ring, or the
/// grouped tree's windows — and the query only narrows it: the matcher
/// never re-derives the window set, honoring the freeze philosophy. A
/// handle the snapshot no longer carries (a window that closed
/// mid-gesture) never matches — liveness rides the snapshot, exactly as
/// every rebuild's does. The empty query is the identity: an empty
/// filter filters nothing.
pub fn matches(toplevels: &[Toplevel], handles: &[Handle], query: &str) -> Vec<Handle> {
    if query.is_empty() {
        // Identity over live handles only: a dead window drops off even
        // with nothing typed, mirroring every other listing path.
        return handles
            .iter()
            .copied()
            .filter(|handle| toplevels.iter().any(|t| t.handle == *handle))
            .collect();
    }
    let needle = query.to_lowercase();
    handles
        .iter()
        .copied()
        .filter(|handle| {
            toplevels.iter().any(|t| {
                t.handle == *handle
                    && (t.title.to_lowercase().contains(&needle)
                        || t.app_id.to_lowercase().contains(&needle))
            })
        })
        .collect()
}

/// A selection ring over the eligible list ([`eligible`]'s order):
/// `next`/`previous` move the selection with wrap-around, and an empty
/// ring selects nothing.
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

    /// The frozen handle list the ring walks, in ring order — the
    /// type-to-filter matcher's input (`matches`).
    pub fn handles(&self) -> &[Handle] {
        &self.handles
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
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    /// A top-level row nothing nests under: a single-window app (the W3C
    /// end-node rule — no chevron, no count) or an un-attributable
    /// window, which never groups. Always visible whatever the
    /// expansion state.
    #[default]
    Leaf,
    /// A multi-window app's top-level row — the level-1 "layer" entry.
    /// Its children sit contiguously after it in [`Grouping::rows`] and
    /// join the visible sequence only while the app is expanded.
    Parent,
    /// A window nested under the nearest preceding parent of its app —
    /// a level-2 row.
    Child,
}

/// One row of the grouped switcher list: the window the row commits to,
/// plus its place in the app tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupedRow {
    /// The window this row resolves to at commit time. A parent carries
    /// its app's front eligible window — the entry's implied
    /// window, `[alt]+1` — so Alt-release on a parent row activates
    /// that window.
    pub handle: Handle,
    /// The app the row belongs to: the grouping key for parents and
    /// children, empty for un-attributable top-level leaves.
    pub app_id: String,
    pub kind: RowKind,
}

/// The eligible list re-partitioned into an app → windows tree: parent
/// rows in first-appearance order over [`eligible`]'s sequence, each
/// followed by its children in the same order, top-level leaves at their
/// own first-appearance positions.
/// The window set freezes at open exactly as the flat list does.
///
/// Expansion is *caller state*, not tree state: groups render collapsed
/// until the app's id appears in the caller's expanded set, and
/// [`Grouping::visible`] cuts the frozen rows down to what the card
/// shows. An empty app_id groups nothing (the [`app_anchor`] rule), so
/// unattributable windows stay top-level leaf rows.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grouping {
    rows: Vec<GroupedRow>,
}

#[cfg_attr(not(test), allow(dead_code))]
impl Grouping {
    /// Partitions [`eligible`]'s list on `active` under `scope` (over
    /// `workspaces`, workspace-major under [`Scope::All`]) into the grouped
    /// row sequence.
    pub fn new(
        toplevels: &[Toplevel],
        workspaces: &[Workspace],
        active: WorkspaceId,
        scope: Scope,
    ) -> Self {
        let handles = eligible(toplevels, workspaces, active, scope);
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
                // First appearance fronts its app's block as the parent
                // — the level-1 layer entry, carrying the app's
                // front window (the head of the eligible list) as its
                // implied window. The app's *whole* window set follows as
                // children in eligible order,
                // front window first: the parent commits what the
                // sketch numbers `[alt]+1`, so that window is the first
                // child the card shows.
                rows.push(GroupedRow {
                    handle,
                    app_id: app.clone(),
                    kind: RowKind::Parent,
                });
                for (&sibling, sibling_app) in handles.iter().zip(&apps) {
                    if sibling_app == app {
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
    /// children, top-level leaves at their own positions — the frozen
    /// window set every walk addresses.
    pub fn rows(&self) -> &[GroupedRow] {
        &self.rows
    }

    /// The rows the card shows under `expanded`: every level-1 row,
    /// plus the children of each expanded app in their frozen order —
    /// the sequence ↓/↑ stepping, the `rows` budget, and pointer rows
    /// apply over. A collapsed group contributes only its parent row.
    pub fn visible(&self, expanded: &std::collections::HashSet<String>) -> Vec<usize> {
        let mut visible = Vec::with_capacity(self.rows.len());
        let mut parent_app: Option<&str> = None;
        for (index, row) in self.rows.iter().enumerate() {
            match row.kind {
                RowKind::Child => {
                    // Children ride along only while their app is
                    // expanded; `parent_app` tracks the block they
                    // belong to (children sit contiguously under theirs).
                    if let Some(app) = parent_app
                        && expanded.contains(app)
                    {
                        visible.push(index);
                    }
                }
                _ => {
                    parent_app = (row.kind == RowKind::Parent).then_some(row.app_id.as_str());
                    visible.push(index);
                }
            }
        }
        visible
    }

    /// The children of the parent at `parent`: the contiguous
    /// [`RowKind::Child`] block after it, in its frozen eligible order —
    /// empty for leaves and out-of-range positions.
    pub fn children_of(&self, parent: usize) -> Vec<usize> {
        if self
            .rows
            .get(parent)
            .is_none_or(|row| row.kind != RowKind::Parent)
        {
            return Vec::new();
        }
        let mut children = Vec::new();
        let mut index = parent + 1;
        while self
            .rows
            .get(index)
            .is_some_and(|row| row.kind == RowKind::Child)
        {
            children.push(index);
            index += 1;
        }
        children
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

    /// A workspace for the All-scope grouping pins — `id` and coordinates
    /// under test's choosing.
    fn workspace(id: u64, coordinates: &[u32]) -> Workspace {
        Workspace {
            id: WorkspaceId(id),
            name: format!("workspace {id}"),
            coordinates: coordinates.to_vec(),
        }
    }

    #[test]
    fn eligible_keeps_only_toplevels_on_the_active_workspace() {
        let toplevels = vec![
            toplevel(1, &[1], State::default(), 0),
            toplevel(2, &[2], State::default(), 0),
            toplevel(3, &[3], State::default(), 0),
        ];
        let handles = eligible(&toplevels, &[], WorkspaceId(2), Scope::CurrentWorkspace);
        assert_eq!(handles, vec![Handle(2)]);
    }

    #[test]
    fn eligible_includes_multi_workspace_toplevels() {
        let toplevels = vec![
            toplevel(1, &[1, 2, 3], State::default(), 0),
            toplevel(2, &[1], State::default(), 0),
        ];
        let handles = eligible(&toplevels, &[], WorkspaceId(3), Scope::CurrentWorkspace);
        assert_eq!(handles, vec![Handle(1)]);
    }

    #[test]
    fn eligible_includes_sticky_toplevels_on_any_workspace() {
        // Sticky windows are visible everywhere, even if the compositor does
        // not list every workspace in their workspace set.
        let toplevels = vec![toplevel(1, &[1], State::STICKY, 0)];
        let handles = eligible(&toplevels, &[], WorkspaceId(4), Scope::CurrentWorkspace);
        assert_eq!(handles, vec![Handle(1)]);
    }

    #[test]
    fn eligible_excludes_minimized_toplevels() {
        let toplevels = vec![
            toplevel(1, &[1], State::MINIMIZED, 0),
            toplevel(2, &[1], State::default(), 0),
        ];
        let handles = eligible(&toplevels, &[], WorkspaceId(1), Scope::CurrentWorkspace);
        assert_eq!(handles, vec![Handle(2)]);
    }

    #[test]
    fn eligible_orders_by_most_recently_activated() {
        let toplevels = vec![
            toplevel(1, &[1], State::default(), 10),
            toplevel(2, &[1], State::default(), 30),
            toplevel(3, &[1], State::default(), 20),
        ];
        let handles = eligible(&toplevels, &[], WorkspaceId(1), Scope::CurrentWorkspace);
        assert_eq!(handles, vec![Handle(2), Handle(3), Handle(1)]);
    }

    #[test]
    fn eligible_breaks_mru_ties_by_snapshot_order() {
        let toplevels = vec![
            toplevel(1, &[1], State::default(), 7),
            toplevel(2, &[1], State::default(), 7),
        ];
        let handles = eligible(&toplevels, &[], WorkspaceId(1), Scope::CurrentWorkspace);
        assert_eq!(handles, vec![Handle(1), Handle(2)]);
    }

    #[test]
    fn eligible_on_no_matching_toplevels_is_empty() {
        let toplevels = vec![toplevel(1, &[2], State::default(), 0)];
        assert!(eligible(&toplevels, &[], WorkspaceId(1), Scope::CurrentWorkspace).is_empty());
    }

    #[test]
    fn all_scope_lists_workspace_major_with_the_invoking_workspace_first() {
        // Three workspaces in shuffled snapshot order — coordinates (then
        // id) decide position, never snapshot order. Invoked from the
        // middle workspace (2).
        let workspaces = vec![
            workspace(3, &[2, 0]),
            workspace(1, &[0, 0]),
            workspace(2, &[1, 0]),
        ];
        let toplevels = vec![
            // Invoking-workspace members (tier 0)...
            toplevel(1, &[2], State::default(), 10),
            toplevel(2, &[2], State::default(), 30),
            // ...sticky among them — visible everywhere means visible
            // here.
            toplevel(3, &[1], State::STICKY, 50),
            // A multi-workspace window that includes the invoking
            // workspace groups with it.
            toplevel(8, &[2, 3], State::default(), 5),
            // Workspace 1's group (coordinates position 0), MRU within...
            toplevel(4, &[1], State::default(), 20),
            toplevel(5, &[1], State::default(), 40),
            // ...and a multi-workspace non-member landing in its earliest
            // listed workspace (1, not 3).
            toplevel(7, &[1, 3], State::default(), 35),
            // Workspace 3's group (position 2) after workspace 1's.
            toplevel(6, &[3], State::default(), 25),
            // Workspace-less tail (tier 2) in MRU — even the globally
            // most recent window (11, membership unlisted) waits at the
            // bottom.
            toplevel(9, &[], State::default(), 15),
            toplevel(10, &[], State::default(), 45),
            toplevel(11, &[99], State::default(), 55),
        ];
        let handles = eligible(&toplevels, &workspaces, WorkspaceId(2), Scope::All);
        assert_eq!(
            handles,
            vec![
                // Tier 0: invoking workspace plus sticky, MRU within.
                Handle(3),
                Handle(2),
                Handle(1),
                Handle(8),
                // Tier 1: workspace 1 (coordinates first)...
                Handle(5),
                Handle(7),
                Handle(4),
                // ...then workspace 3.
                Handle(6),
                // Tier 2: workspace-less, MRU within.
                Handle(11),
                Handle(10),
                Handle(9),
            ]
        );
    }

    #[test]
    fn all_scope_breaks_mru_ties_by_snapshot_order() {
        // The two windows tie on last_activated within the same tier:
        // snapshot order decides, exactly as the CurrentWorkspace sort
        // does.
        let workspaces = vec![workspace(1, &[0, 0]), workspace(2, &[1, 0])];
        let toplevels = vec![
            toplevel(1, &[1], State::default(), 7),
            toplevel(2, &[2], State::default(), 7),
        ];
        let handles = eligible(&toplevels, &workspaces, WorkspaceId(1), Scope::All);
        assert_eq!(handles, vec![Handle(1), Handle(2)]);
    }

    #[test]
    fn all_scope_orders_equal_coordinate_workspaces_by_id() {
        // The workspace-order convention: coordinates lexicographic, id
        // breaking ties (the format_snapshot rule).
        let workspaces = vec![workspace(5, &[0, 0]), workspace(4, &[0, 0])];
        let toplevels = vec![
            toplevel(1, &[5], State::default(), 10),
            toplevel(2, &[4], State::default(), 5),
        ];
        let handles = eligible(&toplevels, &workspaces, WorkspaceId(9), Scope::All);
        // Workspace 4 sorts before 5 despite the snapshot listing 5
        // first, so window 2's group leads.
        assert_eq!(handles, vec![Handle(2), Handle(1)]);
    }

    #[test]
    fn all_scope_excludes_minimized_toplevels() {
        let toplevels = vec![
            toplevel(1, &[2], State::MINIMIZED, 30),
            toplevel(2, &[1], State::default(), 10),
        ];
        let handles = eligible(&toplevels, &[], WorkspaceId(1), Scope::All);
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
            app_anchor(&toplevels, &[], WorkspaceId(1), Scope::CurrentWorkspace),
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
            app_anchor(&toplevels, &[], WorkspaceId(1), Scope::CurrentWorkspace),
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
            app_anchor(&toplevels, &[], WorkspaceId(1), Scope::CurrentWorkspace),
            None
        );
    }

    #[test]
    fn app_anchor_under_all_anchors_the_invoking_workspaces_front() {
        // The global MRU front (browser, 40) lives on workspace 2; the
        // invoking workspace is 1. Under All the eligible front is the
        // invoking workspace's front — here outranks recent
        // (workspace-order, 2026-11-14) — so editor anchors. The
        // CurrentWorkspace scope reaches the same anchor by confinement.
        let toplevels = vec![
            app_window(1, "browser", &[2], State::default(), 40),
            app_window(2, "editor", &[1], State::ACTIVATED, 30),
            app_window(3, "editor", &[1], State::default(), 20),
        ];
        assert_eq!(
            app_anchor(&toplevels, &[], WorkspaceId(1), Scope::CurrentWorkspace),
            Some("editor".to_string())
        );
        let workspaces = vec![workspace(1, &[0, 0]), workspace(2, &[1, 0])];
        assert_eq!(
            app_anchor(&toplevels, &workspaces, WorkspaceId(1), Scope::All),
            Some("editor".to_string())
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
    fn app_view_under_all_lists_the_apps_windows_grouped() {
        // Editor windows across three workspaces, invoked from the middle
        // one (2): the app's run inside the grouped order — invoking
        // workspace first, then the others in workspace order,
        // workspace-less last, MRU within each group. The app's old flat
        // MRU run (1, 2, 3, 4, 6 by recency) is what dies here.
        let snapshot = Snapshot {
            toplevels: vec![
                app_window(1, "editor", &[2], State::default(), 30),
                app_window(2, "editor", &[1], State::default(), 25),
                app_window(3, "editor", &[3], State::default(), 20),
                app_window(4, "editor", &[2], State::default(), 15),
                // Another app's window on the invoking workspace — never
                // in the view.
                app_window(5, "browser", &[2], State::ACTIVATED, 40),
                // Workspace-less editor window: the tail.
                app_window(6, "editor", &[], State::default(), 10),
            ],
            workspaces: vec![
                workspace(1, &[0, 0]),
                workspace(2, &[1, 0]),
                workspace(3, &[2, 0]),
            ],
            active: Some(WorkspaceId(2)),
        };
        let view = app_view(&snapshot, "editor", Scope::All);
        let handles: Vec<Handle> = view.toplevels.iter().map(|t| t.handle).collect();
        assert_eq!(
            handles,
            vec![Handle(1), Handle(4), Handle(2), Handle(3), Handle(6)]
        );
        // The confined view still lists only this workspace's editor
        // windows, MRU within.
        let confined = app_view(&snapshot, "editor", Scope::CurrentWorkspace);
        let confined_handles: Vec<Handle> = confined.toplevels.iter().map(|t| t.handle).collect();
        assert_eq!(confined_handles, vec![Handle(1), Handle(4)]);
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

    /// Like [`toplevel`], with the title and app under test's choosing.
    fn window(handle: u64, title: &str, app_id: &str) -> Toplevel {
        Toplevel {
            handle: Handle(handle),
            title: title.to_string(),
            app_id: app_id.to_string(),
            state: State::default(),
            workspaces: [1u64].iter().copied().map(WorkspaceId).collect(),
            last_activated: 0,
        }
    }

    #[test]
    fn matches_hits_on_title_and_app_id_case_insensitively() {
        let toplevels = vec![
            window(1, "Invoice — March", "libreoffice.writer"),
            window(2, "terminal", "org.contour.Terminal"),
            window(3, "Inbox — 42 unread", "mail.desktop"),
        ];
        let frozen = vec![Handle(1), Handle(2), Handle(3)];
        assert_eq!(
            matches(&toplevels, &frozen, "inv"),
            vec![Handle(1)],
            "title hit, prefix"
        );
        assert_eq!(
            matches(&toplevels, &frozen, "UNREAD"),
            vec![Handle(3)],
            "case-insensitive both ways"
        );
        assert_eq!(
            matches(&toplevels, &frozen, "terminal"),
            vec![Handle(2)],
            "title and app_id hit on the same window, once"
        );
        assert_eq!(
            matches(&toplevels, &frozen, "in"),
            vec![Handle(1), Handle(2), Handle(3)],
            "every window hits, order preserved"
        );
    }

    #[test]
    fn matches_empty_query_is_the_identity_over_live_handles() {
        let toplevels = vec![window(1, "a", "app.a"), window(2, "b", "app.b")];
        let frozen = vec![Handle(1), Handle(2), Handle(9)];
        assert_eq!(
            matches(&toplevels, &frozen, ""),
            vec![Handle(1), Handle(2)],
            "identity — minus the dead handle, which never lists"
        );
    }

    #[test]
    fn matches_no_hit_is_empty_and_dead_handles_never_match() {
        let toplevels = vec![window(1, "editor", "app.a")];
        let frozen = vec![Handle(1), Handle(2)];
        assert!(matches(&toplevels, &frozen, "zzz").is_empty());
        // Handle 2 is not in the snapshot (it closed mid-gesture): even
        // a query its frozen title would match cannot resurrect it.
        assert!(matches(&toplevels, &frozen, "toplevel 2").is_empty());
        assert_eq!(
            matches(&toplevels, &frozen, "editor"),
            vec![Handle(1)],
            "the live hit survives beside the dead handle"
        );
    }

    #[test]
    fn matches_on_an_empty_app_id_uses_the_title_alone() {
        let toplevels = vec![window(1, "nowhere man", "")];
        assert_eq!(matches(&toplevels, &[Handle(1)], "where"), vec![Handle(1)]);
        // And the empty app_id matches nothing as an app_id — an empty
        // needle is the identity, handled above; an empty *haystack*
        // field simply never contains a non-empty needle.
        assert_eq!(
            matches(&toplevels, &[Handle(1)], "app"),
            Vec::<Handle>::new()
        );
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
        // first-appearance order (a, then b); the app's whole window
        // set follows as children, front window first, MRU within the
        // app; single-window c is a leaf.
        let toplevels = vec![
            app_window(1, "a", &[1], State::ACTIVATED, 30),
            app_window(2, "b", &[1], State::default(), 25),
            app_window(3, "a", &[1], State::default(), 20),
            app_window(4, "c", &[1], State::default(), 15),
            app_window(5, "a", &[1], State::default(), 10),
            app_window(6, "b", &[1], State::default(), 5),
        ];
        let grouping = Grouping::new(&toplevels, &[], WorkspaceId(1), Scope::CurrentWorkspace);
        assert_eq!(
            shape(&grouping),
            vec![
                (RowKind::Parent, Handle(1)),
                (RowKind::Child, Handle(1)),
                (RowKind::Child, Handle(3)),
                (RowKind::Child, Handle(5)),
                (RowKind::Parent, Handle(2)),
                (RowKind::Child, Handle(2)),
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
        let grouping = Grouping::new(&toplevels, &[], WorkspaceId(1), Scope::CurrentWorkspace);
        assert_eq!(
            shape(&grouping),
            vec![
                (RowKind::Leaf, Handle(1)),
                (RowKind::Parent, Handle(2)),
                (RowKind::Child, Handle(2)),
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
        let grouping = Grouping::new(&toplevels, &[], WorkspaceId(1), Scope::CurrentWorkspace);
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
        let grouping = Grouping::new(&toplevels, &[], WorkspaceId(1), Scope::CurrentWorkspace);
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
        let grouping = Grouping::new(&toplevels, &[], WorkspaceId(1), Scope::CurrentWorkspace);
        assert_eq!(
            shape(&grouping),
            vec![
                (RowKind::Parent, Handle(1)),
                (RowKind::Child, Handle(1)),
                (RowKind::Child, Handle(2)),
                (RowKind::Child, Handle(3)),
            ]
        );
    }

    #[test]
    fn grouped_under_all_follows_the_workspace_major_order() {
        // Two multi-window apps spread over two workspaces plus a
        // single-window app on a third, invoked from workspace 2 (the
        // coordinates-middle one). Parent rows appear in grouped
        // first-appearance order — a's invoking-workspace window fronts,
        // b's follows — and each app's children follow in grouped order:
        // the invoking workspace's windows before the other workspace's.
        // The flat MRU run (1, 7, 5, 3, 4, 6) is what dies here.
        let toplevels = vec![
            // a on workspace 1: handles 3 (25), 4 (10).
            app_window(3, "a", &[1], State::default(), 25),
            app_window(4, "a", &[1], State::default(), 10),
            // a on the invoking workspace 2: handle 1 (40) — the front.
            app_window(1, "a", &[2], State::ACTIVATED, 40),
            // b on the invoking workspace 2: handle 5 (30); on
            // workspace 1: handle 6 (5).
            app_window(5, "b", &[2], State::default(), 30),
            app_window(6, "b", &[1], State::default(), 5),
            // Single-window c on workspace 3: a leaf at its grouped
            // position, behind workspaces 1 and 2.
            app_window(7, "c", &[3], State::default(), 35),
        ];
        let workspaces = vec![
            workspace(1, &[0, 0]),
            workspace(2, &[1, 0]),
            workspace(3, &[2, 0]),
        ];
        let grouping = Grouping::new(&toplevels, &workspaces, WorkspaceId(2), Scope::All);
        // Grouped order: 1, 5 (workspace 2, MRU), 3, 4, 6 (workspace 1),
        // 7 (workspace 3) — first appearances a, b, then c's leaf. Each
        // app's children follow in that same order: a's run 1, 3, 4
        // (invoking workspace first), b's run 5, 6.
        assert_eq!(
            shape(&grouping),
            vec![
                (RowKind::Parent, Handle(1)),
                (RowKind::Child, Handle(1)),
                (RowKind::Child, Handle(3)),
                (RowKind::Child, Handle(4)),
                (RowKind::Parent, Handle(5)),
                (RowKind::Child, Handle(5)),
                (RowKind::Child, Handle(6)),
                (RowKind::Leaf, Handle(7)),
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
        let confined = Grouping::new(&toplevels, &[], WorkspaceId(1), Scope::CurrentWorkspace);
        assert_eq!(shape(&confined), vec![(RowKind::Leaf, Handle(1))]);
        // All-scope grouped: a1 fronts from the invoking workspace, then
        // workspace 2's pair MRU within (b before a2) — first appearance
        // a then b.
        let workspaces = vec![workspace(1, &[0, 0]), workspace(2, &[1, 0])];
        let everywhere = Grouping::new(&toplevels, &workspaces, WorkspaceId(1), Scope::All);
        assert_eq!(
            shape(&everywhere),
            vec![
                (RowKind::Parent, Handle(1)),
                (RowKind::Child, Handle(1)),
                (RowKind::Child, Handle(2)),
                (RowKind::Leaf, Handle(3)),
            ]
        );
    }

    #[test]
    fn grouped_on_an_empty_eligible_list_is_empty() {
        let toplevels = vec![app_window(1, "a", &[2], State::default(), 30)];
        let grouping = Grouping::new(&toplevels, &[], WorkspaceId(1), Scope::CurrentWorkspace);
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
        Grouping::new(&toplevels, &[], WorkspaceId(1), Scope::CurrentWorkspace)
    }

    #[test]
    fn parent_of_a_child_is_the_row_that_heads_its_block() {
        // The fixture's tree: P-a1, C-a1, C-a2, C-a3, P-b1, C-b1,
        // C-b2, L-c1 — every child's parent is the row heading its
        // contiguous block.
        let grouping = grouped_fixture();
        for child in 1..=3 {
            assert_eq!(grouping.parent_of(child), Some(0));
        }
        for child in 5..=6 {
            assert_eq!(grouping.parent_of(child), Some(4));
        }
    }

    #[test]
    fn parent_of_a_parent_or_leaf_is_none() {
        // Only a child nests; parents and leaves are top-level.
        let grouping = grouped_fixture();
        assert_eq!(grouping.parent_of(0), None);
        assert_eq!(grouping.parent_of(4), None);
        assert_eq!(grouping.parent_of(7), None);
    }

    #[test]
    fn parent_of_an_out_of_range_position_is_none() {
        let grouping = grouped_fixture();
        assert_eq!(grouping.parent_of(9), None);
    }

    #[test]
    fn walks_on_an_empty_grouping_are_none() {
        let grouping = Grouping::new(&[], &[], WorkspaceId(1), Scope::CurrentWorkspace);
        assert_eq!(grouping.parent_of(0), None);
    }
}
