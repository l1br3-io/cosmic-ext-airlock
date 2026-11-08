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
/// The combined card's addressing map: band 1's apps and band 2's
/// windows (tree indices) exactly as the last model rendered them, beside
/// each band's viewport — so a pointer row maps back to a tree index
/// without handle guessing and the per-band `…` follow carries on.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CombinedMap {
    band1: crate::ui::Viewport,
    band2: crate::ui::Viewport,
    apps: Vec<usize>,
    windows: Vec<usize>,
}

/// The frozen session — the selection ring over a scope plus the settings
/// frozen at open. See [`session_step`] for how sessions are built.
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
    /// `rows` setting — clamped to the density's 720-px fit
    /// (`ui::effective_rows`, one diagnostic when it binds) — and frozen
    /// like the palette: a config edit landing mid-gesture never
    /// re-windows the open session.
    rows: usize,
    /// The card's shape — corner radius and row rhythm derived from the
    /// desktop's style and density at open, frozen like the palette and
    /// rows: a desktop settings flip landing mid-gesture never
    /// re-rhythms the open card; the next press applies it.
    profile: crate::ui::StyleProfile,
    /// The card's face — the Bitmap floor or the System stack in one
    /// resolved family, resolved once at open from the `font` key, the
    /// interface-font follow, and the zero-fonts floor, frozen like the
    /// palette: a font edit landing mid-gesture never repaints the open
    /// card; the next press applies it.
    text: crate::text::TextDecision,
    /// Whether this session groups its list by app, resolved once at
    /// open from the `grouping` setting and frozen like the rows: a
    /// config edit landing mid-gesture never regroups the open session.
    grouping: bool,
    /// Which way this session's card lays out, resolved once at open
    /// from the `layout` setting and frozen like the palette: a config
    /// edit landing mid-gesture never re-lays-out the open session.
    layout: crate::ui::LayoutSetting,
    /// The horizontal card's tile budget, resolved once at open from
    /// the `columns` setting and frozen with the layout. Binds
    /// `layout = "horizontal"` only; the vertical card never reads it.
    columns: crate::ui::ColumnsSetting,
    /// The `columns` setting resolved against the invoking output's
    /// logical width — computed by the serve loop at open (the one
    /// place that sees both) and frozen with everything else: an
    /// output change mid-gesture never re-windows the open band.
    tiles: usize,
    /// The frozen tree a grouped plain session walks: the partition of
    /// the eligible list at open, kept so every grouped input — Tab's
    /// linear steps, →/←'s level walks, the card's rows — addresses one
    /// frozen window set. `None` for flat sessions and same-app sessions
    /// (the exemption — already app-scoped, rendered flat).
    tree: Option<switcher::Grouping>,
    /// The selected row of the frozen tree — a parent or leaf (level 1)
    /// or a child (level 2). The level is the row's kind; the selection
    /// is the tree index, stable for the session's life. Meaningless
    /// whenever `tree` is `None` (those sessions select through `ring`).
    tree_sel: usize,
    /// The type-to-filter query (`type-filter`). Born empty, dies with
    /// the session — never persisted. While non-empty,
    /// [`Self::filtered`] holds the narrowed selection ring and the
    /// card renders flat.
    query: String,
    /// The query-narrowed ring: the frozen window set filtered by
    /// [`switcher::matches`], re-derived on every query change. `Some`
    /// exactly while `query` is non-empty; `None` is the unfiltered
    /// session. Stepping, committing, and the card all read this first.
    filtered: Option<switcher::Ring>,
    /// The window set the session froze at open, in MRU order — the
    /// flat ring's own list, the grouped tree's eligible set, or the
    /// same-app view's. The query narrows exactly this list, never a
    /// re-derived one (the freeze philosophy).
    frozen: Vec<Handle>,
    /// The pointer's resting place (`close-window`'s × reveal): the row
    /// and its window as the last hover observation carried them —
    /// presence, not crossing intent, so the × reveals even on the
    /// seeding contact the crossing policy ignores. `None` on leave,
    /// gap, continuation rows, and after every close (the card
    /// recomposes; the pair is stale the moment rows shift). Paint-time
    /// validation against the freshly built rows keeps a stale index
    /// from ever offering a wrong window's close.
    hovered: Option<(usize, Handle)>,
    /// The tree rows the card is showing right now, recomputed on every
    /// show from the frozen tree, the expansion state, and liveness —
    /// the exact sequence the model rendered, so pointer rows map back
    /// to tree rows without handle guessing.
    visible: Vec<usize>,
    /// The window the card is showing over the session's list — derived
    /// state carried between rebuilds so the sticky rule can continue
    /// it, never persisted across gestures: sessions are built fresh
    /// with `None`, and every open starts top-aligned on the MRU head.
    viewport: Option<crate::ui::Viewport>,
    /// The grouped-horizontal card's focus band — band 1 (app tiles) or
    /// band 2 (window tiles). `None` on every non-combined session; the
    /// band is focus, not collapse: the combined card has no collapse
    /// state, so band 2 always shows the focused app's whole window set.
    band: Option<crate::ui::Band>,
    /// The combined card's addressing map for the model last rendered:
    /// band 1's apps and band 2's windows (tree indices) beside their
    /// viewports, so a pointer row maps back to a tree index without
    /// handle guessing and each band's `…` follow carries on. `None` on
    /// every non-combined session.
    combined_map: Option<CombinedMap>,
}

impl Switch {
    /// The window the selection resolves to: the query-narrowed ring's
    /// while one stands, else the selected tree row's handle for a
    /// grouped session (a parent carries its app's MRU-front window —
    /// the entry's implied window), the ring's for every flat one.
    fn selected_handle(&self) -> Option<Handle> {
        if let Some(filtered) = &self.filtered {
            return filtered.selected();
        }
        match &self.tree {
            Some(tree) => tree.rows().get(self.tree_sel).map(|row| row.handle),
            None => self.ring.selected(),
        }
    }

    /// Selects `handle`'s row — the flow pointer clicks and number
    /// chords share. A querying session selects its narrowed ring;
    /// otherwise flat sessions select their ring; a grouped session
    /// selects the first visible row carrying the handle (a parent
    /// precedes its children, so an ambiguous handle — a group's front
    /// window — lands on the parent row). A handle not on the card
    /// selects nothing and leaves the session exactly as it was.
    fn select_handle(&mut self, handle: Handle) -> Option<Handle> {
        if let Some(filtered) = &mut self.filtered {
            return filtered.select(handle);
        }
        if let Some(tree) = &self.tree {
            if let Some(map) = &self.combined_map {
                // Combined: search band 1's apps then band 2's windows;
                // the band follows which sequence the hit lives in (a
                // leaf window tile is band 2 even though its kind is
                // Leaf). A group's front window is both its parent and
                // its first child, so the first hit lands on the parent
                // — harmless, since a click/Ctrl+N commits immediately.
                let found = map
                    .apps
                    .iter()
                    .chain(map.windows.iter())
                    .copied()
                    .find(|&index| tree.rows()[index].handle == handle)?;
                self.tree_sel = found;
                self.band = Some(if map.apps.contains(&found) {
                    crate::ui::Band::Apps
                } else {
                    crate::ui::Band::Windows
                });
                return Some(handle);
            }
            let index = self
                .visible
                .iter()
                .copied()
                .find(|&index| tree.rows()[index].handle == handle)?;
            self.tree_sel = index;
            Some(handle)
        } else {
            self.ring.select(handle)
        }
    }

    /// The tree row a pointer crossing of model row `row` addresses.
    /// Content rows map through the stored visible sequence and the
    /// viewport the last model reported; continuation rows and rows
    /// past the window are `None`. The row index is exact where a
    /// handle alone is ambiguous — a group's front window is both its
    /// parent row and its first child, and hovering the child must
    /// select the child. The combined card maps through its two bands
    /// instead: band 1's apps then band 2's windows, each behind its own
    /// viewport.
    fn tree_row_at(&self, row: usize) -> Option<usize> {
        if let Some(map) = &self.combined_map {
            let s1 = map.band1.content + map.band1.bottom as usize;
            if row < s1 {
                return (row < map.band1.content)
                    .then(|| map.apps.get(map.band1.start + row).copied())
                    .flatten();
            }
            let r2 = row - s1;
            return (r2 < map.band2.content)
                .then(|| map.windows.get(map.band2.start + r2).copied())
                .flatten();
        }
        let viewport = self.viewport?;
        let top = viewport.top as usize;
        if row < top || row >= top + viewport.content {
            return None; // a continuation row or past the window
        }
        self.visible.get(viewport.start + row).copied()
    }

    /// Lands the selection on the card's first row — the restore rule's
    /// fallback when the kept window left the card. A grouped session
    /// takes the first visible tree row; a flat one the frozen list's
    /// MRU head. An empty card leaves the selection exactly where it
    /// was (the caller's rebuild decides what an empty card means).
    fn select_first(&mut self, snapshot: &Snapshot, expanded: &std::collections::HashSet<String>) {
        if self.is_combined() {
            if let Some(&first) = combined_apps(self, snapshot).first() {
                self.tree_sel = first;
                self.band = Some(crate::ui::Band::Apps);
            }
        } else if self.tree.is_some() {
            let visible = grouped_visible(self, snapshot, expanded);
            if let Some(&first) = visible.first() {
                self.tree_sel = first;
            }
        } else if let Some(&front) = self.frozen.first() {
            // The MRU head: a dead one simply fails to select, and the
            // rebuild's liveness filtering handles the rest of the card.
            self.ring.select(front);
        }
    }

    /// Applies a pointer hover crossing: flat sessions select by the
    /// event-time handle; grouped sessions by the event-time row — both
    /// captured against the model the user saw. Rev 5: every visible
    /// row selects, headers included — the divider rule is gone. The
    /// combined card derives its band from the row's position (band 1's
    /// tiles come first), so a leaf window tile is never mistaken for
    /// its band-1 app tile. Returns whether the selection moved.
    fn select_pointer_row(&mut self, row: usize, handle: Option<Handle>) -> bool {
        if self.is_combined() {
            let Some(index) = self.tree_row_at(row) else {
                return false;
            };
            self.tree_sel = index;
            self.band = Some(self.band_at_row(row).unwrap_or(crate::ui::Band::Apps));
            return true;
        }
        match &self.tree {
            Some(_) => {
                let Some(index) = self.tree_row_at(row) else {
                    return false;
                };
                self.tree_sel = index;
                true
            }
            None => handle.is_some_and(|handle| self.ring.select(handle).is_some()),
        }
    }

    /// Whether this session is the grouped-horizontal combination: the
    /// frozen tree plus a horizontal layout.
    fn is_combined(&self) -> bool {
        self.tree.is_some() && self.layout == crate::ui::LayoutSetting::Horizontal
    }

    /// The focused app driving band 2 — a level-1 tree index: the
    /// selection's parent when band 2 holds focus, the selection itself
    /// when band 1 does.
    fn combined_focused(&self) -> Option<usize> {
        let tree = self.tree.as_ref()?;
        match self.band {
            Some(crate::ui::Band::Apps) => Some(self.tree_sel),
            Some(crate::ui::Band::Windows) => {
                Some(tree.parent_of(self.tree_sel).unwrap_or(self.tree_sel))
            }
            None => None,
        }
    }

    /// The band a model row lives in: band 1 for rows before band 1's
    /// slots, band 2 for the rest. `None` off the card or on a
    /// continuation tile.
    fn band_at_row(&self, row: usize) -> Option<crate::ui::Band> {
        let map = self.combined_map.as_ref()?;
        let s1 = map.band1.content + map.band1.bottom as usize;
        if row < s1 {
            (row < map.band1.content).then_some(crate::ui::Band::Apps)
        } else {
            (row - s1 < map.band2.content).then_some(crate::ui::Band::Windows)
        }
    }
}

/// The tree rows a grouped session can address right now: [`Grouping`'s
/// visible sequence](switcher::Grouping::visible) under the daemon's
/// persistent expansion state, minus windows that left the eligible set
/// since the freeze — dead rows drop off the card instead of ghosting,
/// and a dead *selection* closes the session through `session_holds`
/// before this ever runs.
fn grouped_visible(
    session: &Switch,
    snapshot: &Snapshot,
    expanded: &std::collections::HashSet<String>,
) -> Vec<usize> {
    let Some(tree) = session.tree.as_ref() else {
        return Vec::new();
    };
    tree.visible(expanded)
        .into_iter()
        .filter(|&index| switcher::switchable(snapshot, tree.rows()[index].handle, session.scope))
        .collect()
}

/// Whether an open session still matches reality: the workspace it was
/// built for is still active, and its selection is still switchable there
/// under the session's scope (not closed, minimized, or — for a
/// CurrentWorkspace session — moved or unstickied away).
pub fn session_holds(open: &Switch, snapshot: &Snapshot) -> bool {
    snapshot.active == Some(open.workspace)
        && open
            .selected_handle()
            .is_some_and(|handle| switcher::switchable(snapshot, handle, open.scope))
}

/// The handle a commit should activate: the session's selection, but only
/// while it is still switchable in the current snapshot under the
/// session's scope. A CurrentWorkspace selection that left its workspace
/// mid-gesture commits as a no-op instead of crossing workspaces; an All
/// session may legitimately activate a window on another workspace.
pub fn commit_target(open: &Switch, snapshot: &Snapshot) -> Option<Handle> {
    open.selected_handle()
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

/// Starts or advances the gesture session's selection by one step.
///
/// A session's first press builds it from the MRU front — the focused
/// window's row — so the opening Alt+Tab (Alt+Shift+Tab) is the first
/// step of the session's linear walk in grouped mode: one visible row
/// past the active window — the next app entry while the active group
/// stands collapsed (a quick tap activates the MRU-second app's front
/// window), the next window while it stands expanded (flat quick-tap
/// parity). Later
/// presses reuse the same frozen selection basis instead of rebuilding
/// it: walking three windows takes three presses, unlike T4's
/// per-press rebuild which toggled between the front two. `scope`,
/// `palette`, `profile`, `rows`, `grouping`,
/// `layout`, `columns`, and `text` are consumed only when the session
/// opens; an already-open session keeps the ones it was built with.
// Thirteen open-path parameters: the frozen-at-open values ride
// individually (see session_step's note).
#[expect(clippy::too_many_arguments)]
pub fn ring_step(
    open: &mut Option<Switch>,
    snapshot: &Snapshot,
    direction: Direction,
    scope: Scope,
    palette: crate::ui::Palette,
    profile: crate::ui::StyleProfile,
    rows: usize,
    grouping: bool,
    text: crate::text::TextDecision,
    layout: crate::ui::LayoutSetting,
    columns: crate::ui::ColumnsSetting,
    tiles: usize,
    expanded: &mut std::collections::HashSet<String>,
) -> Option<Handle> {
    if open.is_none() {
        let active = snapshot.active?;
        // A grouped session opens on the frozen tree with its selection
        // on row 0 — the focused window's entry, its app's parent when
        // the app is multi-window — and the opening press is the first
        // linear step past the active window's rows. Flat sessions keep
        // the eligible-list ring, no tree. The frozen window set — the
        // query's narrow domain — is the eligible list in both shapes.
        let eligible = switcher::eligible(&snapshot.toplevels, active, scope);
        let (handles, tree) = if grouping {
            (
                Vec::new(), // grouped selections live on the tree, not a ring
                Some(switcher::Grouping::new(&snapshot.toplevels, active, scope)),
            )
        } else {
            (eligible.clone(), None)
        };
        let combined = grouping && layout == crate::ui::LayoutSetting::Horizontal;
        *open = Some(Switch {
            ring: switcher::Ring::new(handles),
            workspace: active,
            scope,
            app: None,
            palette,
            rows,
            profile,
            text,
            grouping,
            layout,
            columns,
            tiles,
            tree,
            tree_sel: 0,
            query: String::new(),
            filtered: None,
            frozen: eligible,
            hovered: None,
            visible: Vec::new(),
            viewport: None,
            band: combined.then_some(crate::ui::Band::Apps),
            combined_map: None,
        });
        if open.as_ref().is_some_and(|session| session.tree.is_some()) {
            let session = open.as_mut()?;
            if session.is_combined() {
                // Combined: the opening press steps apps (lands on the
                // second app tile, band 1) — Tab's behavior, the
                // quick-tap criterion.
                step_apps(session, snapshot, direction);
            } else {
                step_open(session, snapshot, expanded, direction);
            }
            return session.selected_handle();
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
/// its frozen selection whatever key stepped it — a grouped session's ↓/↑
/// and wheel steps walk its visible rows ([`step_visible`]); a same-app
/// session re-filters the fresh snapshot by its frozen anchor (see
/// [`step_and_show`]), but the selection basis never rebuilds — the same
/// freeze philosophy as the scope and the palette, both consumed only when
/// the session opens.
// Fourteen open-path parameters: the frozen-at-open values (palette,
// profile, rows, grouping, layout, columns + its
// resolved tiles, text) each ride individually and the expansion state
// is daemon memory the open may write — no bundle introduced (the GR3
// note).
#[expect(clippy::too_many_arguments)]
pub fn session_step(
    open: &mut Option<Switch>,
    snapshot: &Snapshot,
    direction: Direction,
    scope: Scope,
    same_app: bool,
    palette: crate::ui::Palette,
    profile: crate::ui::StyleProfile,
    rows: usize,
    grouping: bool,
    text: crate::text::TextDecision,
    layout: crate::ui::LayoutSetting,
    columns: crate::ui::ColumnsSetting,
    tiles: usize,
    expanded: &mut std::collections::HashSet<String>,
) -> Option<Handle> {
    // An open querying session steps its narrowed ring — every step key
    // is the one linear walk, whatever the session's unfiltered shape.
    if let Some(session) = open.as_mut()
        && let Some(filtered) = session.filtered.as_mut()
    {
        return match direction {
            Direction::Next => filtered.step(1),
            Direction::Previous => filtered.previous(),
        };
    }
    // An open grouped session steps its visible rows (Tab, ↓/↑, and
    // the wheel share the one linear walk); the snapshot, scope,
    // palette, rows, and grouping only matter when a session opens.
    // Plain opens go straight through. The combined card's step is
    // within-band, not the linear walk.
    if let Some(session) = open.as_mut()
        && session.tree.is_some()
    {
        if session.is_combined() {
            step_band(session, snapshot, direction);
        } else {
            step_visible(session, snapshot, expanded, direction);
        }
        return session.selected_handle();
    }
    if open.is_some() || !same_app {
        return ring_step(
            open, snapshot, direction, scope, palette, profile, rows, grouping, text, layout,
            columns, tiles, expanded,
        );
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
    let selected = ring_step(
        open, &view, direction, scope, palette, profile, rows, false, text, layout, columns, tiles,
        expanded,
    );
    // ring_step stored the session on the filtered ring; freeze the anchor
    // into it so later steps re-filter by it.
    if let Some(session) = open.as_mut() {
        session.app = Some(app);
    }
    selected
}

/// The next/previous member of `order` after `current` (wrapping), or
/// `current` itself when `order` holds nothing else to move to.
fn step_in(order: &[usize], current: usize, direction: Direction) -> usize {
    let Some(position) = order.iter().position(|&index| index == current) else {
        return current;
    };
    let len = order.len();
    if len < 2 {
        return current;
    }
    let delta = match direction {
        Direction::Next => 1,
        Direction::Previous => len - 1,
    };
    order[(position + delta) % len]
}

/// The opening press of a grouped session: one step of the linear walk
/// past the active window's rows. The basis is the *last* visible row
/// carrying the active window — its app's header while the group stands
/// collapsed, its first child while expanded — so the opening Alt+Tab
/// lands one window past the active one (flat quick-tap parity) when
/// the group is expanded, and on the next app entry when it is
/// collapsed. A lone collapsed entry wraps onto itself: rev 5 never
/// descends into a collapsed group, so the press is a stay, never an
/// entry.
fn step_open(
    session: &mut Switch,
    snapshot: &Snapshot,
    expanded: &std::collections::HashSet<String>,
    direction: Direction,
) {
    let visible = grouped_visible(session, snapshot, expanded);
    let Some(tree) = session.tree.as_ref() else {
        return;
    };
    // The gesture's starting window: the frozen tree's MRU front (the
    // session opened on it).
    let Some(active) = tree.rows().first().map(|row| row.handle) else {
        return;
    };
    let basis = visible
        .iter()
        .rposition(|&index| tree.rows()[index].handle == active)
        .and_then(|position| visible.get(position).copied())
        .unwrap_or(session.tree_sel);
    session.tree_sel = step_in(&visible, basis, direction);
}

/// → : expand the
/// selected parent — the expansion persists across gestures — and land
/// on its enter target: the group's most recently used window, one past
/// the gesture's still-active window when that is the front (entering
/// your own app never lands you back on the window you just left; the
/// owner's Firefox trace). The W3C end-node rule keeps a leaf's enter a
/// no-op. Returns whether anything moved.
fn enter_group(
    session: &mut Switch,
    snapshot: &Snapshot,
    expanded: &mut std::collections::HashSet<String>,
    direction: Direction,
) -> bool {
    let Some(tree) = session.tree.as_ref() else {
        return false;
    };
    let parent = session.tree_sel;
    let Some(app) = tree.rows().get(parent).map(|row| row.app_id.clone()) else {
        return false;
    };
    if tree
        .rows()
        .get(parent)
        .is_none_or(|row| row.kind != switcher::RowKind::Parent)
    {
        return false; // a leaf's enter: the W3C no-op
    }
    // The group's frozen children, liveness-filtered — not the visible
    // list, which still shows the group collapsed: entering is what
    // expands it.
    let children: Vec<usize> = tree
        .children_of(parent)
        .into_iter()
        .filter(|index| switcher::switchable(snapshot, tree.rows()[*index].handle, session.scope))
        .collect();
    let Some(front) = children.first().copied() else {
        return false; // every child died mid-gesture: nothing to enter
    };
    // Expanding is part of entering — the card shows what was walked
    // into, and the state persists for the next gesture.
    expanded.insert(app);
    // The gesture's starting window: the frozen tree's MRU front (the
    // session opened on it). Entering past it mirrors how the opening
    // Alt+Tab steps past the active window.
    let active = tree.rows()[0].handle;
    let target = match children
        .iter()
        .position(|&index| tree.rows()[index].handle == active)
    {
        None => match direction {
            Direction::Next => front,
            Direction::Previous => *children.last().unwrap_or(&front),
        },
        Some(position) => {
            let len = children.len();
            let delta = match direction {
                Direction::Next => 1,
                Direction::Previous => len - 1,
            };
            children[(position + delta) % len]
        }
    };
    session.tree_sel = target;
    true
}

/// ← : a child exits to its parent row (the group stays expanded — the
/// header holds the selection under rev 5); an expanded parent
/// collapses — the state persists across gestures; a collapsed
/// top-level row is the W3C root no-op. Returns whether the selection
/// or the visible list changed.
fn exit_group(session: &mut Switch, expanded: &mut std::collections::HashSet<String>) -> bool {
    let Some(tree) = session.tree.as_ref() else {
        return false;
    };
    let Some(row) = tree.rows().get(session.tree_sel) else {
        return false;
    };
    match row.kind {
        switcher::RowKind::Child => {
            let Some(parent) = tree.parent_of(session.tree_sel) else {
                return false;
            };
            session.tree_sel = parent;
            true
        }
        switcher::RowKind::Parent => {
            // Collapsing changes the card even though the selection
            // holds — the caller must refresh either way.
            expanded.remove(&row.app_id)
        }
        switcher::RowKind::Leaf => false,
    }
}

/// Tab, ↓/↑, and the wheel: one row of the linear walk — everything
/// the card shows, headers and expanded children alike, crossing group
/// boundaries. It wraps, and a collapsed group's children are skipped
/// exactly as they are not shown (rev 5: nothing ever descends into a
/// collapsed group).
fn step_visible(
    session: &mut Switch,
    snapshot: &Snapshot,
    expanded: &std::collections::HashSet<String>,
    direction: Direction,
) {
    let visible = grouped_visible(session, snapshot, expanded);
    session.tree_sel = step_in(&visible, session.tree_sel, direction);
}

// --- grouped-horizontal: the combined card's session walks -----------
//
// The combination has no collapse state: band 1 is the whole level-1
// sequence (apps), band 2 the focused app's whole window set. The walks
// below resolve against those two domains directly — the expansion set
// plays no part.

/// Band 1's domain: the level-1 tree rows (parents and leaves),
/// liveness-filtered — the whole app list (no collapse state).
fn combined_apps(session: &Switch, snapshot: &Snapshot) -> Vec<usize> {
    let Some(tree) = session.tree.as_ref() else {
        return Vec::new();
    };
    tree.visible(&std::collections::HashSet::new())
        .into_iter()
        .filter(|&index| switcher::switchable(snapshot, tree.rows()[index].handle, session.scope))
        .collect()
}

/// Band 2's domain: the focused app's windows, liveness-filtered. A
/// parent contributes its whole child set; a leaf its single window.
fn combined_windows(session: &Switch, snapshot: &Snapshot, focused: usize) -> Vec<usize> {
    let Some(tree) = session.tree.as_ref() else {
        return Vec::new();
    };
    let indices = match tree.rows().get(focused).map(|row| row.kind) {
        Some(switcher::RowKind::Parent) => tree.children_of(focused),
        _ => vec![focused],
    };
    indices
        .into_iter()
        .filter(|&index| switcher::switchable(snapshot, tree.rows()[index].handle, session.scope))
        .collect()
}

/// The combined card's visible walk, in reading order: band 1's apps
/// then the focused app's windows (tree indices).
fn combined_sequence(session: &Switch, snapshot: &Snapshot) -> (Vec<usize>, usize) {
    let apps = combined_apps(session, snapshot);
    let focused = session.combined_focused().unwrap_or(session.tree_sel);
    let windows = combined_windows(session, snapshot, focused);
    let apps_len = apps.len();
    let mut seq = apps;
    seq.extend(windows);
    (seq, apps_len)
}

/// ←/→ in the combined card: step the focused band's tiles, wrapping.
/// Band 1 steps the app list (band 2 swaps to the newly focused app);
/// band 2 steps the focused app's windows.
fn step_band(session: &mut Switch, snapshot: &Snapshot, direction: Direction) {
    if !session.is_combined() {
        return;
    }
    match session.band {
        Some(crate::ui::Band::Apps) => {
            let apps = combined_apps(session, snapshot);
            if !apps.is_empty() {
                session.tree_sel = step_in(&apps, session.tree_sel, direction);
            }
        }
        Some(crate::ui::Band::Windows) => {
            let focused = session.combined_focused().unwrap_or(session.tree_sel);
            let windows = combined_windows(session, snapshot, focused);
            if !windows.is_empty() {
                session.tree_sel = step_in(&windows, session.tree_sel, direction);
            }
        }
        None => {}
    }
}

/// Tab in the combined card: step the app list from anywhere, landing in
/// band 1 (the spec's "Tab steps apps from anywhere").
fn step_apps(session: &mut Switch, snapshot: &Snapshot, direction: Direction) {
    if !session.is_combined() {
        return;
    }
    let apps = combined_apps(session, snapshot);
    if apps.is_empty() {
        return;
    }
    let current = session.combined_focused().unwrap_or(session.tree_sel);
    session.tree_sel = step_in(&apps, current, direction);
    session.band = Some(crate::ui::Band::Apps);
}

/// ↓ in the combined card: from an app tile, enter band 2 on that app's
/// MRU-front window; from a window tile, the W3C deepest-level no-op.
fn enter_band(session: &mut Switch, snapshot: &Snapshot) -> bool {
    if !session.is_combined() || session.band == Some(crate::ui::Band::Windows) {
        return false;
    }
    let focused = session.combined_focused().unwrap_or(session.tree_sel);
    let windows = combined_windows(session, snapshot, focused);
    let Some(&front) = windows.first() else {
        return false;
    };
    session.tree_sel = front;
    session.band = Some(crate::ui::Band::Windows);
    true
}

/// ↑ in the combined card: from a window tile, return to band 1 on its
/// app tile (selection intact); from an app tile, the W3C root no-op.
fn exit_band(session: &mut Switch) -> bool {
    if !session.is_combined() || session.band == Some(crate::ui::Band::Apps) {
        return false;
    }
    let focused = session.combined_focused().unwrap_or(session.tree_sel);
    session.tree_sel = focused;
    session.band = Some(crate::ui::Band::Apps);
    true
}

/// One character into the session's query (`type-filter`): append —
/// the query caps at [`QUERY_MAX`], input past it ignored — then
/// re-derive the narrowed ring over the frozen set and reselect its
/// first match. Returns whether a session was touched (`false` with no
/// session — structurally unreachable while showing).
fn type_into(open: &mut Option<Switch>, snapshot: &Snapshot, ch: char) -> bool {
    let Some(session) = open.as_mut() else {
        return false;
    };
    if session.query.chars().count() < QUERY_MAX {
        session.query.push(ch);
    }
    refilter(session, snapshot);
    true
}

/// Backspace: erase the query's last character. An empty query is a
/// no-op — Backspace never dismisses. Erasing to empty restores the
/// unfiltered session (the shared restore rule).
fn erase_query(
    open: &mut Option<Switch>,
    snapshot: &Snapshot,
    expanded: &std::collections::HashSet<String>,
) -> bool {
    let Some(session) = open.as_mut() else {
        return false;
    };
    if session.query.pop().is_none() {
        return true; // nothing to erase; still a live session
    }
    if session.query.is_empty() {
        restore_unfiltered(session, snapshot, expanded);
    } else {
        refilter(session, snapshot);
    }
    true
}

/// Clears the query outright (Escape's first press). Returns whether a
/// session stood with a query to clear — `false` means no session or an
/// already-empty query, and the caller dismisses.
fn clear_query(
    open: &mut Option<Switch>,
    snapshot: &Snapshot,
    expanded: &std::collections::HashSet<String>,
) -> bool {
    let Some(session) = open.as_mut() else {
        return false;
    };
    if session.query.is_empty() {
        return false;
    }
    session.query.clear();
    restore_unfiltered(session, snapshot, expanded);
    true
}

/// The restore rule erase-to-empty and clear share: dissolve the
/// narrowed ring, keep the selected window when the unfiltered card
/// still holds it, else land on the first row.
fn restore_unfiltered(
    session: &mut Switch,
    snapshot: &Snapshot,
    expanded: &std::collections::HashSet<String>,
) {
    let selected = session.filtered.as_ref().and_then(switcher::Ring::selected);
    session.filtered = None;
    if selected.is_none_or(|handle| session.select_handle(handle).is_none()) {
        session.select_first(snapshot, expanded);
    }
}

/// Re-derives the narrowed ring from the standing query — the one
/// recompute every query change shares. The empty query dissolves the
/// ring (the unfiltered session); dead handles never match, and the
/// empty ring selects nothing.
fn refilter(session: &mut Switch, snapshot: &Snapshot) {
    if session.query.is_empty() {
        session.filtered = None;
    } else {
        let matched = switcher::matches(&snapshot.toplevels, &session.frozen, &session.query);
        session.filtered = Some(switcher::Ring::new(matched));
    }
}

/// Advance-then-close (`close-window`): move the selection off the
/// doomed window *before* the compositor request — `session_holds`
/// must never observe a dead selection, and the card recomposes around
/// a selection already past the closing row. The next *live* row of
/// the card's current sequence (the querying ring, the tree's visible
/// rows, or the flat ring), wrapping, rows carrying the doomed window
/// excluded (a group's header and its front child are one window).
/// Returns whether the session holds anything left; the caller sends
/// the close request only for a session that does, and dismisses one
/// that does not.
fn advance_off_doomed(
    session: &mut Switch,
    snapshot: &Snapshot,
    expanded: &std::collections::HashSet<String>,
    doomed: Handle,
) -> bool {
    // A close that does not sit under the selection moves nothing: the
    // pointer path (the ×) closes any hovered window while the
    // keyboard's selection stays on its own. The key path always
    // passes the selection's own handle, so the rule changes nothing
    // there.
    if session.selected_handle() != Some(doomed) {
        return true;
    }
    // The querying walk: the narrowed ring's handles.
    let sequence: Vec<Handle> = if let Some(filtered) = &session.filtered {
        filtered.handles().to_vec()
    } else if session.is_combined() {
        let (seq, _) = combined_sequence(session, snapshot);
        let tree = session
            .tree
            .as_ref()
            .expect("combined sessions carry a tree");
        seq.into_iter()
            .map(|index| tree.rows()[index].handle)
            .collect()
    } else if let Some(tree) = &session.tree {
        grouped_visible(session, snapshot, expanded)
            .into_iter()
            .map(|index| tree.rows()[index].handle)
            .collect()
    } else {
        session.ring.handles().to_vec()
    };
    // The doomed row's own position — the scan starts one past it and
    // wraps, so the selection moves to the walk's NEXT live row, not
    // the list's first (repeated handles — a header and its front
    // child — sit before or on the position and never resurface).
    let Some(position) = sequence.iter().position(|&handle| handle == doomed) else {
        return session.selected_handle().is_some();
    };
    let len = sequence.len();
    for step in 1..=len {
        let handle = sequence[(position + step) % len];
        if handle != doomed
            && switcher::switchable(snapshot, handle, session.scope)
            && select_fresh(session, snapshot, expanded, handle)
        {
            return true;
        }
    }
    false // nothing eligible remains: the caller dismisses
}

/// Selects `handle` against freshly computed sequences — the advance
/// rule's twin of [`Switch::select_handle`], which addresses the stored
/// `visible` the last show built. The advance must not trust stale
/// geometry: it selects whatever the card would show now.
fn select_fresh(
    session: &mut Switch,
    snapshot: &Snapshot,
    expanded: &std::collections::HashSet<String>,
    handle: Handle,
) -> bool {
    if let Some(filtered) = &mut session.filtered {
        return filtered.select(handle).is_some();
    }
    if session.is_combined() {
        let tree = session
            .tree
            .as_ref()
            .expect("combined sessions carry a tree");
        let (seq, apps_len) = combined_sequence(session, snapshot);
        return seq
            .iter()
            .position(|&index| tree.rows()[index].handle == handle)
            .is_some_and(|pos| {
                let index = seq[pos];
                session.tree_sel = index;
                session.band = Some(if pos < apps_len {
                    crate::ui::Band::Apps
                } else {
                    crate::ui::Band::Windows
                });
                true
            });
    }
    if let Some(tree) = &session.tree {
        let target = grouped_visible(session, snapshot, expanded)
            .into_iter()
            .find(|&index| tree.rows()[index].handle == handle);
        return target.is_some_and(|index| {
            session.tree_sel = index;
            true
        });
    }
    session.ring.select(handle).is_some()
}

/// The session-side half of a close (`close-window`), pure over the
/// open session: forget the pointer's hovered row (the card
/// recomposes; the × returns on the next motion), move the selection
/// off the doomed window when it rests there, and report whether
/// anything eligible remains — the caller dismisses a card that does
/// not.
fn prepare_close(
    session: &mut Switch,
    snapshot: &Snapshot,
    expanded: &std::collections::HashSet<String>,
    doomed: Handle,
) -> bool {
    session.hovered = None;
    advance_off_doomed(session, snapshot, expanded, doomed)
}

/// Closes one window of the open card (`close-window`'s shared path —
/// the Alt+minus key and the pointer's ×): prepare the session
/// (advance off the doomed selection, forget the hover), send the
/// request, repaint what survives. The session survives for repeated
/// closes; a card with nothing left dismisses.
fn close_window(
    session: &mut wayland::Session,
    gesture: &mut Gesture,
    open: &mut Option<Switch>,
    snapshot: &Snapshot,
    expanded: &std::collections::HashSet<String>,
    icons: &mut crate::icons::Cache,
    doomed: Handle,
) -> Result<(), RunError> {
    let holds = open
        .as_mut()
        .is_some_and(|current| prepare_close(current, snapshot, expanded, doomed));
    if !holds {
        session.overlay_hide("nothing left to close");
        gesture.close();
        *open = None;
    }
    // The request rides whatever the compositor does with it: honored,
    // the closed event drops the row off the card on the next poll;
    // refused, the card keeps the window and the walk continues.
    session.close(doomed).map_err(RunError::Session)?;
    if holds {
        show_selection(session, gesture, open, snapshot, expanded, icons);
    }
    Ok(())
}

/// Closes the selected row's window (`close-window`'s key path):
/// resolve the selection's handle and hand it to the shared
/// close.
fn close_selected(
    session: &mut wayland::Session,
    gesture: &mut Gesture,
    open: &mut Option<Switch>,
    snapshot: &Snapshot,
    expanded: &std::collections::HashSet<String>,
    icons: &mut crate::icons::Cache,
) -> Result<(), RunError> {
    let Some(doomed) = open.as_ref().and_then(Switch::selected_handle) else {
        return Ok(());
    };
    close_window(session, gesture, open, snapshot, expanded, icons, doomed)
}

/// The row a hover observation reveals the × on (`close-window`): the
/// pointer's pair (row, handle), confirmed against the rows the model
/// just built — a stale index (the card recomposed under a resting
/// pointer) never offers a wrong window's close.
fn hovered_row(
    model: &crate::ui::OverlayModel,
    observed: Option<(usize, Handle)>,
) -> Option<usize> {
    observed
        .filter(|&(row, handle)| {
            model
                .rows
                .get(row)
                .is_some_and(|row| row.handle == Some(handle))
        })
        .map(|(row, _)| row)
}

/// The type-to-filter query cap: input past it is ignored, so the
/// session's state stays bounded whatever holds the key down. The strip
/// shows the query's tail (TF3).
const QUERY_MAX: usize = 64;

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
    // The card's text backend (`typography`): the one font scan paid
    // here at daemon start — OQ 1 puts its tens-of-ms cost at login,
    // never on the first gesture. Fonts installed after startup stay
    // unseen until the daemon restarts (documented limitation).
    session.set_text_system(crate::text::TextSystem::scan_system());

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
    // Which grouped apps stand expanded — daemon memory, deliberately
    // not config: groups collapse by default at login, and every
    // expand/collapse a gesture makes persists into the next one.
    let mut expanded: std::collections::HashSet<String> = std::collections::HashSet::new();
    // Hover-selection policy for the overlay's pointer inputs, reset
    // whenever no session is open (see the pointer drain).
    let mut hover = PointerHover::default();

    loop {
        let mut activity = session.poll().map_err(RunError::Session)?;
        // Refresh first so input processing in this iteration sees the
        // state the poll just delivered.
        let world_changed = session.is_dirty();
        if world_changed {
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

        // A changed world recomposes the open card (`close-window`):
        // the poll that confirms a close drops the doomed row off the
        // card the moment the compositor reports it. Without this the
        // row ghosts until the next input repaints — the pointer's ×
        // path hides behind the motion that follows a click, but
        // Alt+minus sends no event after the request.
        if world_changed && open.is_some() {
            show_selection(
                &mut session,
                &mut gesture,
                &mut open,
                &snapshot,
                &expanded,
                &mut icons,
            );
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
                    // Tab, ↓/↑ share this arm: rev 5's one linear walk.
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
                        &mut expanded,
                    );
                }
                GestureOutput::EnterChild => {
                    // → expands the selected parent (persistently) and
                    // enters its children; on a leaf the W3C end-node
                    // rule makes it a no-op. ↓ in the combined card
                    // enters band 2 on the focused app's front window.
                    let entered = if open.as_ref().is_some_and(|session| session.is_combined()) {
                        open.as_mut()
                            .is_some_and(|session| enter_band(session, &snapshot))
                    } else {
                        open.as_mut().is_some_and(|session| {
                            enter_group(session, &snapshot, &mut expanded, Direction::Next)
                        })
                    };
                    if entered {
                        show_selection(
                            &mut session,
                            &mut gesture,
                            &mut open,
                            &snapshot,
                            &expanded,
                            &mut icons,
                        );
                    }
                }
                GestureOutput::ExitParent => {
                    // ← exits a child to its parent row; an expanded
                    // parent collapses; a collapsed top-level row is the
                    // W3C root no-op. ↑ in the combined card returns to
                    // band 1 on the focused app's tile.
                    let exited = if open.as_ref().is_some_and(|session| session.is_combined()) {
                        open.as_mut().is_some_and(exit_band)
                    } else {
                        open.as_mut()
                            .is_some_and(|session| exit_group(session, &mut expanded))
                    };
                    if exited {
                        show_selection(
                            &mut session,
                            &mut gesture,
                            &mut open,
                            &snapshot,
                            &expanded,
                            &mut icons,
                        );
                    }
                }
                GestureOutput::StepApp(direction) => {
                    // Tab in the combined card steps the app list from
                    // anywhere, landing in band 1.
                    if let Some(current) = open.as_mut()
                        && current.is_combined()
                    {
                        step_apps(current, &snapshot, direction);
                        show_selection(
                            &mut session,
                            &mut gesture,
                            &mut open,
                            &snapshot,
                            &expanded,
                            &mut icons,
                        );
                    }
                }
                GestureOutput::StepWindow(_direction) => {
                    // The key above Tab (B1 wires the key; the walk
                    // lands in B2). No-op keeps the tree compiling.
                }
                GestureOutput::Type(ch) => {
                    // One character into the query: narrow the frozen
                    // set, reselect the first match, repaint. No open
                    // session, no query (structurally unreachable —
                    // keys only arrive while showing).
                    if type_into(&mut open, &snapshot, ch) {
                        show_selection(
                            &mut session,
                            &mut gesture,
                            &mut open,
                            &snapshot,
                            &expanded,
                            &mut icons,
                        );
                    }
                }
                GestureOutput::Erase => {
                    // Backspace edits the query; never dismisses.
                    if erase_query(&mut open, &snapshot, &expanded) {
                        show_selection(
                            &mut session,
                            &mut gesture,
                            &mut open,
                            &snapshot,
                            &expanded,
                            &mut icons,
                        );
                    }
                }
                GestureOutput::Escape => {
                    // A standing query clears first — the session, the
                    // gesture, and the card all keep going; only the
                    // empty query dismisses.
                    if clear_query(&mut open, &snapshot, &expanded) {
                        show_selection(
                            &mut session,
                            &mut gesture,
                            &mut open,
                            &snapshot,
                            &expanded,
                            &mut icons,
                        );
                    } else {
                        gesture.close();
                        session.overlay_hide("dismissed");
                        open = None;
                    }
                }
                GestureOutput::CloseSelected => {
                    // Alt+minus: the selection's window closes; the
                    // advance-then-close ordering keeps the session's
                    // premise alive and the session itself open for
                    // repeated closes.
                    close_selected(
                        &mut session,
                        &mut gesture,
                        &mut open,
                        &snapshot,
                        &expanded,
                        &mut icons,
                    )?;
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
                    // Ctrl+1..9 and clicks: select and commit
                    // in one motion, the same flow each shares —
                    // addressing the handle the boundary captured at the
                    // key press or click. A `None` handle (a continuation
                    // or out-of-range row) no-ops with the session and
                    // gesture untouched.
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
                PointerInput::Close { handle } => {
                    // The × (`close-window`): the press-time capture
                    // closes the hovered row's window through the
                    // shared close path — the selection and query stay
                    // wherever the gesture left them unless the doomed
                    // window sits under the selection.
                    close_window(
                        &mut session,
                        &mut gesture,
                        &mut open,
                        &snapshot,
                        &expanded,
                        &mut icons,
                        handle,
                    )?;
                }
                PointerInput::Hover { row, handle } => {
                    // Presence tracking (`close-window`): the pointer's
                    // pair rides the session beside the crossing policy
                    // — the × reveals wherever the pointer rests, even
                    // on the seeding contact the policy ignores, and
                    // hides the moment it leaves the row.
                    let hovered = row.zip(handle);
                    let revealed = open.as_ref().is_some_and(|open| open.hovered != hovered);
                    if let Some(open) = open.as_mut() {
                        open.hovered = hovered;
                    }
                    // A crossing selects by the event-time target — the
                    // row and handle captured against the model the user
                    // saw — and only a row the session still shows moves
                    // the selection (a window opened mid-gesture is not
                    // hoverable into the session). A continuation-row
                    // crossing (handle `None`) still counts as a row
                    // observation for the crossing policy and selects
                    // nothing; rows the session refuses leave the
                    // overlay untouched. Grouped sessions resolve the
                    // row through their stored visible sequence — the
                    // row index is exact where a handle alone is
                    // ambiguous (a group's front window is both its
                    // parent row and its first child).
                    if hover.observe(row).is_some()
                        && let Some(content_row) = row
                        && handle.is_some()
                        && open
                            .as_mut()
                            .is_some_and(|open| open.select_pointer_row(content_row, handle))
                    {
                        show_selection(
                            &mut session,
                            &mut gesture,
                            &mut open,
                            &snapshot,
                            &expanded,
                            &mut icons,
                        );
                    } else if revealed {
                        // The pointer moved rows without a selecting
                        // crossing (seeding contact, a gap, a header):
                        // the × moved, and the card says so.
                        show_selection(
                            &mut session,
                            &mut gesture,
                            &mut open,
                            &snapshot,
                            &expanded,
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
                            &mut expanded,
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
                                        &mut expanded,
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

/// Selects `handle`'s row and commits the session in one motion — the
/// flow a pointer click and the Ctrl+N / Alt+N chords share. The handle
/// was captured at event receipt against the model the user saw; the
/// frozen session must still hold it — a window opened mid-gesture is
/// not addressable into the session. Anything else leaves the session
/// and overlay exactly as they are. On success the gesture session is
/// closed explicitly: a click never reached the gesture machine, and a
/// chord deliberately left it open (an unaddressable row must not spend
/// it) — either way a later Alt-release cannot re-commit and a Tab
/// cannot reopen the spent session.
fn activate_row(
    session: &mut wayland::Session,
    gesture: &mut Gesture,
    open: &mut Option<Switch>,
    snapshot: &Snapshot,
    handle: Handle,
) -> Result<(), RunError> {
    if open
        .as_mut()
        .is_some_and(|open| open.select_handle(handle).is_some())
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
// Nine parameters: the serve loop's own plumbing (settings, icons,
// expansion memory) beside the step — see session_step's note.
#[expect(clippy::too_many_arguments)]
fn step_and_show(
    session: &mut wayland::Session,
    gesture: &mut Gesture,
    open: &mut Option<Switch>,
    snapshot: &Snapshot,
    step: Step,
    settings: &mut LiveSettings,
    icons: &mut crate::icons::Cache,
    expanded: &mut std::collections::HashSet<String>,
) {
    let (scope, palette, profile, rows, grouping, layout, columns, tiles, text) = match open {
        Some(open) => (
            open.scope,
            open.palette,
            open.profile,
            open.rows,
            open.grouping,
            open.layout,
            open.columns,
            open.tiles,
            open.text.clone(),
        ),
        None => {
            settings.refresh();
            // `auto` asks the system once per open — two small file reads
            // at most — and pinned values never touch the disk at all; a
            // present `card`/`text` pair pins the card over every
            // `theme` value, so it skips the read too (its polarity is
            // its own, the constructor's luminance rule). `None`
            // (unreadable preference) paints dark, never a broken
            // switch.
            let system_prefers_dark = (settings.settings.theme == crate::ui::ThemeSetting::Auto
                && settings.settings.pair.is_none())
            .then(crate::theme::system_prefers_dark)
            .flatten();
            // The card first, then its band: the accent side follows the
            // effective card's polarity — the resolved card's own
            // `is_dark` — through the store ladder, read only when the
            // band follows (`neutral` never touches the accent store).
            // A ladder miss is the neutral lift (with the one aggregated
            // diagnostic the reader prints), and the whole answer
            // freezes onto the session with everything else.
            let mut palette = crate::ui::Palette::resolve(
                settings.settings.theme,
                system_prefers_dark,
                settings.settings.pair,
            );
            let accent_base = (settings.settings.selection == crate::ui::SelectionSetting::Accent)
                .then(|| crate::theme::accent_base(palette.is_dark))
                .flatten();
            palette = palette.with_selection(settings.settings.selection, accent_base);
            // The shape follows, per open like the palette: the
            // desktop's style and density derive the card's radius and
            // rhythm (a store miss is the Round + Standard default,
            // with the reader's one aggregated diagnostic), and the
            // rows budget clamps to what that rhythm fits — the fit is
            // by construction, the diagnostic names density, setting,
            // and clamp when it binds, and the setting keeps its value.
            let style = crate::theme::style().unwrap_or_default();
            let density = crate::theme::density().unwrap_or_default();
            let profile = crate::ui::StyleProfile::resolve(style, density);
            let configured_rows = settings.settings.rows;
            let rows = crate::ui::effective_rows(density, configured_rows);
            // The band's budget resolves once, here: `auto` reads
            // the invoking output's logical width (a 1080p
            // fallback when no output has reported one) and
            // freezes with the open.
            let tiles = crate::ui::resolved_tiles(
                settings.settings.columns,
                session
                    .output_logical_width()
                    .unwrap_or(crate::ui::FALLBACK_OUTPUT_WIDTH),
            );
            if rows < configured_rows {
                eprintln!(
                    "airlock daemon: {density:?} density: rows {configured_rows} clamps to {rows} to keep the card inside {} px; the setting keeps its value",
                    crate::ui::CARD_FIT_PX
                );
            }
            // The face follows, per open like the palette: the `font`
            // key, the interface-font follow, and the zero-fonts floor
            // resolve to one decision, frozen onto the session with
            // everything else (an edit mid-gesture never repaints the
            // open card; the next press applies it).
            let text = resolve_text(session.text_system(), &settings.settings.font, || {
                crate::theme::interface_font(palette.is_dark)
            });
            (
                resolve_scope(snapshot, &settings.settings),
                palette,
                profile,
                rows,
                settings.settings.grouping,
                settings.settings.layout,
                settings.settings.columns,
                tiles,
                text,
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
        profile,
        rows,
        grouping,
        text,
        layout,
        columns,
        tiles,
        expanded,
    );
    show_selection(session, gesture, open, snapshot, expanded, icons);
}

/// The session's text decision, resolved once at open: the zero-fonts
/// floor first (no loadable faces paints the bitmap card — SC 5), then
/// the `font` key — a configured family the database knows, or a
/// diagnostic naming the key and `follow`'s answer when it does not —
/// and `Default` is `follow` itself. The caller passes the interface-
/// font follow as a closure so it is read only on the paths that need
/// it (a configured, installed family never touches the store); tests
/// inject their own answer. Never a broken switch: every miss lands on
/// a paintable answer.
fn resolve_text(
    system: &mut crate::text::TextSystem,
    setting: &crate::text::FontSetting,
    follow: impl FnOnce() -> String,
) -> crate::text::TextDecision {
    if system.face_count() == 0 {
        eprintln!("airlock daemon: no loadable fonts; the card paints the built-in bitmap face");
        return crate::text::TextDecision::Bitmap;
    }
    match setting {
        crate::text::FontSetting::Family(name) => {
            if system.knows_family(name) {
                crate::text::TextDecision::System {
                    family: name.clone(),
                }
            } else {
                eprintln!(
                    "airlock daemon: font family \"{name}\" is not installed (font key); painting the desktop's interface font instead"
                );
                crate::text::TextDecision::System { family: follow() }
            }
        }
        crate::text::FontSetting::Default => crate::text::TextDecision::System { family: follow() },
    }
}

/// Builds and shows the overlay for the open session's current selection
/// — or closes the session when there is nothing eligible left to show.
///
/// Shared by [`step_and_show`] (after a step moved the selection) and the
/// pointer / tree-key arms (which move it without stepping): a same-app
/// session re-filters the fresh snapshot by its frozen anchor, a plain
/// flat one lists every eligible window, and a grouped one paints its
/// frozen tree cut to the current expansion. Icons are already cached,
/// so this is a rebuild-and-repaint, never new I/O.
fn show_selection(
    session: &mut wayland::Session,
    gesture: &mut Gesture,
    open: &mut Option<Switch>,
    snapshot: &Snapshot,
    expanded: &std::collections::HashSet<String>,
    icons: &mut crate::icons::Cache,
) {
    // The card this session paints: the palette frozen at open. A closed
    // session has nothing to paint — dark is the never-drawn default the
    // icon prepare below runs under.
    let palette = open
        .as_ref()
        .map_or_else(crate::ui::Palette::dark, |session| session.palette);
    let scale = session.render_scale();
    // Icons rasterize at the side the shown card paints: the row's own
    // 32 for every vertical card, the tile's 48 for any horizontal card
    // (flat band or the combined two-band card — both are tiles).
    let horizontal = open
        .as_ref()
        .is_some_and(|current| current.layout == crate::ui::LayoutSetting::Horizontal);
    let icon_side = if horizontal {
        crate::ui::TILE_ICON
    } else {
        crate::ui::ICON
    };
    icons.prepare(
        snapshot
            .toplevels
            .iter()
            .map(|toplevel| toplevel.app_id.as_str()),
        icon_side,
        scale,
        &palette,
    );
    // A grouped session stores the visible sequence its card renders —
    // the exact list the pointer's row indexes address — beside the
    // model that rendered it.
    if let Some(current) = open.as_mut() {
        current.visible = grouped_visible(current, snapshot, expanded);
    }
    let model = open.as_mut().and_then(|current| {
        let windowing = crate::ui::Windowing {
            rows: current.rows,
            prev: current.viewport,
        };
        // The frozen decision builds what the frozen decision paints:
        // compose and render share one backend for the session's life.
        let text = current.text.clone();
        // A querying session (`type-filter`) renders flat over its
        // narrowed set — the grouped tree, headers, and chevrons are
        // bypassed while the query stands (the same-app exemption's
        // precedent), and an empty match set keeps the card with the
        // inert no-matches row instead of closing the session.
        if let Some(filtered) = &current.filtered {
            if filtered.handles().is_empty() {
                return crate::ui::no_matches_model(&current.query, current.profile);
            }
            let matched = filtered.handles().to_vec();
            let pruned = Snapshot {
                toplevels: matched
                    .iter()
                    .filter_map(|handle| {
                        snapshot
                            .toplevels
                            .iter()
                            .find(|t| t.handle == *handle)
                            .cloned()
                    })
                    .collect(),
                workspaces: snapshot.workspaces.clone(),
                active: snapshot.active,
            };
            // A matched selection is presence-guaranteed by refilter;
            // a dead one (closed between keystroke and rebuild) falls
            // through to None and the session closes through the
            // existing stale rule.
            return filtered
                .selected()
                .and_then(|handle| {
                    if current.layout == crate::ui::LayoutSetting::Horizontal {
                        crate::ui::overlay_model_horizontal(
                            &pruned,
                            handle,
                            current.scope,
                            icons,
                            scale,
                            &palette,
                            current.profile,
                            crate::ui::Banding {
                                tiles: current.tiles,
                                prev: current.viewport,
                            },
                            crate::ui::text_backend(&text, session.text_system()),
                        )
                    } else {
                        crate::ui::overlay_model(
                            &pruned,
                            handle,
                            current.scope,
                            icons,
                            scale,
                            &palette,
                            current.profile,
                            windowing,
                            crate::ui::text_backend(&text, session.text_system()),
                        )
                    }
                })
                .map(|model| crate::ui::with_query(model, &current.query.clone()));
        }
        match (&current.tree, current.app.as_deref()) {
            // Grouped plain session: the frozen tree cut to the stored
            // visible sequence, selection by tree row — or, combined,
            // the two-band card.
            (Some(tree), None) => {
                if current.is_combined() {
                    let apps = combined_apps(current, snapshot);
                    let focused = current.combined_focused().unwrap_or(current.tree_sel);
                    let windows = combined_windows(current, snapshot, focused);
                    let band = current.band.unwrap_or(crate::ui::Band::Apps);
                    let prev = current
                        .combined_map
                        .as_ref()
                        .map(|map| (map.band1, map.band2));
                    let model = crate::ui::overlay_model_grouped_horizontal(
                        tree,
                        &apps,
                        &windows,
                        current.tree_sel,
                        band,
                        snapshot,
                        icons,
                        scale,
                        &palette,
                        current.profile,
                        crate::ui::CombinedBanding {
                            tiles: current.tiles,
                            band1: prev.map(|(band1, _)| band1),
                            band2: prev.map(|(_, band2)| band2),
                        },
                        crate::ui::text_backend(&text, session.text_system()),
                    );
                    // Remember the two band sequences and viewports the
                    // model just rendered — the pointer's row indexes and
                    // the per-band follow address this exact map.
                    if let Some(model) = &model
                        && let Some(combined) = model.combined
                    {
                        current.combined_map = Some(CombinedMap {
                            band1: combined.band1,
                            band2: combined.band2,
                            apps,
                            windows,
                        });
                    }
                    return model;
                }
                crate::ui::overlay_model_grouped(
                    tree,
                    &current.visible,
                    current.tree_sel,
                    snapshot,
                    icons,
                    scale,
                    &palette,
                    current.profile,
                    windowing,
                    crate::ui::text_backend(&text, session.text_system()),
                )
            }
            // Flat and same-app sessions (the exemption): the flat
            // card over the (possibly re-filtered) snapshot, selection
            // by ring — the horizontal band for a horizontal session
            // (the grouped tree above stays vertical whatever the
            // layout says; the combination is the grouped-horizontal
            // module's, not this card's).
            _ => {
                let list = match current.app.as_deref() {
                    Some(app) => switcher::app_view(snapshot, app, current.scope),
                    None => snapshot.clone(),
                };
                current.ring.selected().and_then(|handle| {
                    if current.layout == crate::ui::LayoutSetting::Horizontal {
                        crate::ui::overlay_model_horizontal(
                            &list,
                            handle,
                            current.scope,
                            icons,
                            scale,
                            &palette,
                            current.profile,
                            crate::ui::Banding {
                                tiles: current.tiles,
                                prev: current.viewport,
                            },
                            crate::ui::text_backend(&text, session.text_system()),
                        )
                    } else {
                        crate::ui::overlay_model(
                            &list,
                            handle,
                            current.scope,
                            icons,
                            scale,
                            &palette,
                            current.profile,
                            windowing,
                            crate::ui::text_backend(&text, session.text_system()),
                        )
                    }
                })
            }
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
            // The × reveal (`close-window`): the pointer's presence
            // paints the close glyph on the hovered row — the pair the
            // last hover observation carried, validated against the
            // rows this model just built, so a stale index reveals
            // nothing.
            let observed = open.as_ref().and_then(|session| session.hovered);
            let mut model = model;
            model.hovered = hovered_row(&model, observed);
            // The key map rides the shown card: flat, vertical grouped,
            // or the grouped-horizontal combination (tree × horizontal).
            // The text decision rides it too — the overlay freezes what
            // it draws under.
            let open_session = open.as_ref();
            let mode = match open_session {
                Some(session) if session.tree.is_some() => {
                    if session.layout == crate::ui::LayoutSetting::Horizontal {
                        crate::ui::KeyMode::GroupedHorizontal
                    } else {
                        crate::ui::KeyMode::Grouped
                    }
                }
                _ => crate::ui::KeyMode::Flat,
            };
            let text = open_session.map_or_else(
                || crate::text::TextDecision::Bitmap,
                |session| session.text.clone(),
            );
            session.overlay_show(model, palette, text, mode)
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
    use crate::ui::{DEFAULT_ROWS, GestureKey, OverlayModel, Palette, RowModel, StyleProfile};
    use std::collections::HashSet;

    /// The tile budget tests open sessions with — `auto` resolved at
    /// the 1080p fallback width (1920 → ⅔ × 1920 / 160 = 8).
    const TILES: usize = 8;

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
                StyleProfile::default(),
                DEFAULT_ROWS,
                false,
                crate::text::TextDecision::Bitmap,
                crate::ui::LayoutSetting::Vertical,
                crate::ui::ColumnsSetting::Auto,
                TILES,
                &mut HashSet::new()
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
                StyleProfile::default(),
                DEFAULT_ROWS,
                false,
                crate::text::TextDecision::Bitmap,
                crate::ui::LayoutSetting::Vertical,
                crate::ui::ColumnsSetting::Auto,
                TILES,
                &mut HashSet::new()
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
                StyleProfile::default(),
                DEFAULT_ROWS,
                false,
                crate::text::TextDecision::Bitmap,
                crate::ui::LayoutSetting::Vertical,
                crate::ui::ColumnsSetting::Auto,
                TILES,
                &mut HashSet::new()
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
                StyleProfile::default(),
                DEFAULT_ROWS,
                false,
                crate::text::TextDecision::Bitmap,
                crate::ui::LayoutSetting::Vertical,
                crate::ui::ColumnsSetting::Auto,
                TILES,
                &mut HashSet::new()
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
                StyleProfile::default(),
                DEFAULT_ROWS,
                false,
                crate::text::TextDecision::Bitmap,
                crate::ui::LayoutSetting::Vertical,
                crate::ui::ColumnsSetting::Auto,
                TILES,
                &mut HashSet::new()
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
                StyleProfile::default(),
                DEFAULT_ROWS,
                false,
                crate::text::TextDecision::Bitmap,
                crate::ui::LayoutSetting::Vertical,
                crate::ui::ColumnsSetting::Auto,
                TILES,
                &mut HashSet::new()
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
                StyleProfile::default(),
                DEFAULT_ROWS,
                false,
                crate::text::TextDecision::Bitmap,
                crate::ui::LayoutSetting::Vertical,
                crate::ui::ColumnsSetting::Auto,
                TILES,
                &mut HashSet::new()
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
                StyleProfile::default(),
                DEFAULT_ROWS,
                false,
                crate::text::TextDecision::Bitmap,
                crate::ui::LayoutSetting::Vertical,
                crate::ui::ColumnsSetting::Auto,
                TILES,
                &mut HashSet::new()
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
                StyleProfile::default(),
                DEFAULT_ROWS,
                false,
                crate::text::TextDecision::Bitmap,
                crate::ui::LayoutSetting::Vertical,
                crate::ui::ColumnsSetting::Auto,
                TILES,
                &mut HashSet::new()
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
                StyleProfile::default(),
                DEFAULT_ROWS,
                false,
                crate::text::TextDecision::Bitmap,
                crate::ui::LayoutSetting::Vertical,
                crate::ui::ColumnsSetting::Auto,
                TILES,
                &mut HashSet::new()
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
                StyleProfile::default(),
                DEFAULT_ROWS,
                false,
                crate::text::TextDecision::Bitmap,
                crate::ui::LayoutSetting::Vertical,
                crate::ui::ColumnsSetting::Auto,
                TILES,
                &mut HashSet::new()
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
            StyleProfile::default(),
            DEFAULT_ROWS,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut HashSet::new(),
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
            StyleProfile::default(),
            DEFAULT_ROWS,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut HashSet::new(),
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
            StyleProfile::default(),
            DEFAULT_ROWS,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut HashSet::new(),
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
            StyleProfile::default(),
            DEFAULT_ROWS,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut HashSet::new(),
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
            StyleProfile::default(),
            DEFAULT_ROWS,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut HashSet::new(),
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
            StyleProfile::default(),
            DEFAULT_ROWS,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut HashSet::new(),
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
            StyleProfile::default(),
            DEFAULT_ROWS,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut HashSet::new(),
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
            StyleProfile::default(),
            DEFAULT_ROWS,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut HashSet::new(),
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
            StyleProfile::default(),
            DEFAULT_ROWS,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut HashSet::new(),
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
            StyleProfile::default(),
            DEFAULT_ROWS,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut HashSet::new(),
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
            StyleProfile::default(),
            DEFAULT_ROWS,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut HashSet::new(),
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
            StyleProfile::default(),
            DEFAULT_ROWS,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut HashSet::new(),
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
            StyleProfile::default(),
            DEFAULT_ROWS,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut HashSet::new(),
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
            StyleProfile::default(),
            DEFAULT_ROWS,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut HashSet::new(),
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
            StyleProfile::default(),
            DEFAULT_ROWS,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut HashSet::new(),
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
            StyleProfile::default(),
            DEFAULT_ROWS,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut HashSet::new(),
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
            StyleProfile::default(),
            DEFAULT_ROWS,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut HashSet::new(),
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
            StyleProfile::default(),
            DEFAULT_ROWS,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut HashSet::new(),
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
            StyleProfile::default(),
            DEFAULT_ROWS,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut HashSet::new(),
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
            StyleProfile::default(),
            DEFAULT_ROWS,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut HashSet::new(),
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
            StyleProfile::default(),
            DEFAULT_ROWS,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut HashSet::new(),
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
                StyleProfile::default(),
                DEFAULT_ROWS,
                false,
                crate::text::TextDecision::Bitmap,
                crate::ui::LayoutSetting::Vertical,
                crate::ui::ColumnsSetting::Auto,
                TILES,
                &mut HashSet::new()
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
                StyleProfile::default(),
                DEFAULT_ROWS,
                false,
                crate::text::TextDecision::Bitmap,
                crate::ui::LayoutSetting::Vertical,
                crate::ui::ColumnsSetting::Auto,
                TILES,
                &mut HashSet::new()
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
                    StyleProfile::default(),
                    DEFAULT_ROWS,
                    false,
                    crate::text::TextDecision::Bitmap,
                    crate::ui::LayoutSetting::Vertical,
                    crate::ui::ColumnsSetting::Auto,
                    TILES,
                    &mut HashSet::new()
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
                StyleProfile::default(),
                DEFAULT_ROWS,
                false,
                crate::text::TextDecision::Bitmap,
                crate::ui::LayoutSetting::Vertical,
                crate::ui::ColumnsSetting::Auto,
                TILES,
                &mut HashSet::new()
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
                StyleProfile::default(),
                DEFAULT_ROWS,
                false,
                crate::text::TextDecision::Bitmap,
                crate::ui::LayoutSetting::Vertical,
                crate::ui::ColumnsSetting::Auto,
                TILES,
                &mut HashSet::new()
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
            StyleProfile::default(),
            DEFAULT_ROWS,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut HashSet::new(),
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
                StyleProfile::default(),
                DEFAULT_ROWS,
                false,
                crate::text::TextDecision::Bitmap,
                crate::ui::LayoutSetting::Vertical,
                crate::ui::ColumnsSetting::Auto,
                TILES,
                &mut HashSet::new()
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
            StyleProfile::default(),
            DEFAULT_ROWS,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut HashSet::new(),
        );
        assert_eq!(
            session_step(
                &mut open,
                &snap,
                Direction::Next,
                Scope::CurrentWorkspace,
                false,
                Palette::dark(),
                StyleProfile::default(),
                DEFAULT_ROWS,
                false,
                crate::text::TextDecision::Bitmap,
                crate::ui::LayoutSetting::Vertical,
                crate::ui::ColumnsSetting::Auto,
                TILES,
                &mut HashSet::new()
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
                StyleProfile::default(),
                DEFAULT_ROWS,
                false,
                crate::text::TextDecision::Bitmap,
                crate::ui::LayoutSetting::Vertical,
                crate::ui::ColumnsSetting::Auto,
                TILES,
                &mut HashSet::new()
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
                StyleProfile::default(),
                DEFAULT_ROWS,
                false,
                crate::text::TextDecision::Bitmap,
                crate::ui::LayoutSetting::Vertical,
                crate::ui::ColumnsSetting::Auto,
                TILES,
                &mut HashSet::new()
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
            StyleProfile::default(),
            DEFAULT_ROWS,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut HashSet::new(),
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
            StyleProfile::default(),
            DEFAULT_ROWS,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut HashSet::new(),
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
                StyleProfile::default(),
                DEFAULT_ROWS,
                false,
                crate::text::TextDecision::Bitmap,
                crate::ui::LayoutSetting::Vertical,
                crate::ui::ColumnsSetting::Auto,
                TILES,
                &mut HashSet::new()
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
                StyleProfile::default(),
                DEFAULT_ROWS,
                false,
                crate::text::TextDecision::Bitmap,
                crate::ui::LayoutSetting::Vertical,
                crate::ui::ColumnsSetting::Auto,
                TILES,
                &mut HashSet::new()
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
                StyleProfile::default(),
                DEFAULT_ROWS,
                false,
                crate::text::TextDecision::Bitmap,
                crate::ui::LayoutSetting::Vertical,
                crate::ui::ColumnsSetting::Auto,
                TILES,
                &mut HashSet::new()
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
            StyleProfile::default(),
            DEFAULT_ROWS,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut HashSet::new(),
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
            layout: crate::ui::LayoutSetting::Vertical,
            columns: crate::ui::ColumnsSetting::Auto,
            tiles: TILES,
            tree: None,
            tree_sel: 0,
            query: String::new(),
            filtered: None,
            frozen: Vec::new(),
            hovered: None,
            visible: Vec::new(),
            profile: StyleProfile::default(),
            text: crate::text::TextDecision::Bitmap,
            viewport: None,
            band: None,
            combined_map: None,
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
            StyleProfile::default(),
            DEFAULT_ROWS,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut HashSet::new(),
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
                StyleProfile::default(),
                DEFAULT_ROWS,
                false,
                crate::text::TextDecision::Bitmap,
                crate::ui::LayoutSetting::Vertical,
                crate::ui::ColumnsSetting::Auto,
                TILES,
                &mut HashSet::new(),
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
            layout: crate::ui::LayoutSetting::Vertical,
            columns: crate::ui::ColumnsSetting::Auto,
            pair: None,
            selection: crate::ui::SelectionSetting::Accent,
            font: crate::text::FontSetting::Default,
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
    fn refresh_adopts_a_pair_change_with_last_good() {
        // The custom-palette twin of the theme flip: a present pair
        // lands on the refresh a next invocation performs — no daemon
        // restart — and a below-floor rewrite keeps the last-good pair
        // (the floor rejects the whole file).
        let dir = TempDir::new("pair-flip");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, b"card = \"#2A1E30\"\ntext = \"#E8E6E0\"\n").expect("write");
        let mut live = live_at(path.clone());
        live.refresh();
        assert_eq!(live.settings.pair, Some((0x00_2A_1E_30, 0x00_E8_E6_E0)));

        std::fs::write(&path, b"card = \"#FFFFFF\"\ntext = \"#ADADAD\"\n").expect("rewrite bad");
        live.refresh();
        assert_eq!(
            live.settings.pair,
            Some((0x00_2A_1E_30, 0x00_E8_E6_E0)),
            "a below-floor pair must not apply"
        );
    }

    #[test]
    fn refresh_adopts_a_selection_change_with_last_good() {
        // The selection twin of the theme flip: `neutral` lands on the
        // refresh a next invocation performs, and an unknown value
        // keeps the last-good selection (the whole file fails).
        let dir = TempDir::new("selection-flip");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, b"selection = \"neutral\"\n").expect("write");
        let mut live = live_at(path.clone());
        live.refresh();
        assert_eq!(
            live.settings.selection,
            crate::ui::SelectionSetting::Neutral
        );

        std::fs::write(&path, b"selection = \"blue\"\n").expect("rewrite bad");
        live.refresh();
        assert_eq!(
            live.settings.selection,
            crate::ui::SelectionSetting::Neutral,
            "an unknown selection value must not apply"
        );
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
            StyleProfile::default(),
            DEFAULT_ROWS,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut HashSet::new(),
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
            StyleProfile::default(),
            DEFAULT_ROWS,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut HashSet::new(),
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
            StyleProfile::default(),
            DEFAULT_ROWS,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut HashSet::new(),
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
            StyleProfile::default(),
            5,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut HashSet::new(),
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
            StyleProfile::default(),
            9,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut HashSet::new(),
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
            StyleProfile::default(),
            9,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut HashSet::new(),
        );
        assert_eq!(open.as_ref().expect("fresh session opened").rows, 9);
    }

    /// The layout twin of the frozen-palette seam: the card's axis is
    /// resolved once at session open and frozen for the session's life
    /// — a `layout` edit landing mid-gesture (here: the value the next
    /// invocation would read) never re-lays-out the open card; the next
    /// press starts fresh against it.
    #[test]
    fn an_open_session_keeps_its_frozen_layout_across_a_config_edit() {
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
            StyleProfile::default(),
            DEFAULT_ROWS,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Horizontal,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut HashSet::new(),
        );
        assert_eq!(
            open.as_ref().expect("session opened").layout,
            crate::ui::LayoutSetting::Horizontal
        );

        // The config moves back to vertical between presses; the open
        // session still lays out the way it was opened with.
        session_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            false,
            Palette::dark(),
            StyleProfile::default(),
            DEFAULT_ROWS,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut HashSet::new(),
        );
        assert_eq!(
            open.as_ref().expect("session still open").layout,
            crate::ui::LayoutSetting::Horizontal,
            "a mid-gesture layout edit must not re-lay-out the open session"
        );

        // The next press after the session ends starts fresh: vertical.
        open = None;
        session_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            false,
            Palette::dark(),
            StyleProfile::default(),
            DEFAULT_ROWS,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut HashSet::new(),
        );
        assert_eq!(
            open.as_ref().expect("fresh session opened").layout,
            crate::ui::LayoutSetting::Vertical
        );
    }

    /// The columns twin of the frozen-palette seam: the horizontal
    /// card's tile budget is resolved once at session open and frozen
    /// for the session's life — a `columns` edit landing mid-gesture
    /// (here: the value the next invocation would read) never re-windows
    /// the open band; the next press starts fresh against it.
    #[test]
    fn an_open_session_keeps_its_frozen_columns_across_a_config_edit() {
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
            StyleProfile::default(),
            DEFAULT_ROWS,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Horizontal,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut HashSet::new(),
        );
        assert_eq!(
            open.as_ref().expect("session opened").columns,
            crate::ui::ColumnsSetting::Auto
        );

        // The config pins five between presses; the open session keeps
        // the budget it was opened with.
        session_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            false,
            Palette::dark(),
            StyleProfile::default(),
            DEFAULT_ROWS,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Horizontal,
            crate::ui::ColumnsSetting::Pinned(5),
            TILES,
            &mut HashSet::new(),
        );
        assert_eq!(
            open.as_ref().expect("session still open").columns,
            crate::ui::ColumnsSetting::Auto,
            "a mid-gesture columns edit must not re-window the open band"
        );

        // The next press after the session ends starts fresh: pinned.
        open = None;
        session_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            false,
            Palette::dark(),
            StyleProfile::default(),
            DEFAULT_ROWS,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Horizontal,
            crate::ui::ColumnsSetting::Pinned(5),
            TILES,
            &mut HashSet::new(),
        );
        assert_eq!(
            open.as_ref().expect("fresh session opened").columns,
            crate::ui::ColumnsSetting::Pinned(5)
        );
    }

    /// The tiles twin of the frozen-palette seam: the resolved budget
    /// is computed by the serve loop at open (the `columns` setting
    /// against the invoking output's logical width) and frozen for the
    /// session's life — an output change landing mid-gesture (here:
    /// the budget the next invocation would resolve) never re-windows
    /// the open band; the next press starts fresh against it.
    #[test]
    fn an_open_session_keeps_its_frozen_tiles_across_an_output_change() {
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
            StyleProfile::default(),
            DEFAULT_ROWS,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Horizontal,
            crate::ui::ColumnsSetting::Auto,
            8,
            &mut HashSet::new(),
        );
        assert_eq!(open.as_ref().expect("session opened").tiles, 8);

        // The output widens between presses (auto would resolve 12);
        // the open session keeps the band it was opened with.
        session_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            false,
            Palette::dark(),
            StyleProfile::default(),
            DEFAULT_ROWS,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Horizontal,
            crate::ui::ColumnsSetting::Auto,
            12,
            &mut HashSet::new(),
        );
        assert_eq!(
            open.as_ref().expect("session still open").tiles,
            8,
            "a mid-gesture output change must not re-window the open band"
        );

        // The next press after the session ends starts fresh: twelve.
        open = None;
        session_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            false,
            Palette::dark(),
            StyleProfile::default(),
            DEFAULT_ROWS,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Horizontal,
            crate::ui::ColumnsSetting::Auto,
            12,
            &mut HashSet::new(),
        );
        assert_eq!(open.as_ref().expect("fresh session opened").tiles, 12);
    }

    #[test]
    fn refresh_adopts_a_layout_change() {
        // Live apply, the layout twin: editing `layout` takes effect on
        // the refresh a next invocation performs — no daemon restart.
        let dir = TempDir::new("layout-edit");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, b"layout = \"vertical\"\n").expect("write");
        let mut live = live_at(path.clone());
        live.refresh();
        assert_eq!(live.settings.layout, crate::ui::LayoutSetting::Vertical);

        std::fs::write(&path, b"layout = \"horizontal\"\n").expect("rewrite");
        live.refresh();
        assert_eq!(live.settings.layout, crate::ui::LayoutSetting::Horizontal);

        // A bad edit keeps the last-good value, like every setting.
        std::fs::write(&path, b"layout = \"sideways\"\n").expect("rewrite bad");
        live.refresh();
        assert_eq!(
            live.settings.layout,
            crate::ui::LayoutSetting::Horizontal,
            "an unknown layout value must not apply"
        );
    }

    #[test]
    fn refresh_adopts_a_columns_change() {
        // Live apply, the columns twin: editing `columns` takes effect
        // on the refresh a next invocation performs — no daemon restart.
        let dir = TempDir::new("columns-edit");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, b"columns = \"auto\"\n").expect("write");
        let mut live = live_at(path.clone());
        live.refresh();
        assert_eq!(live.settings.columns, crate::ui::ColumnsSetting::Auto);

        std::fs::write(&path, b"columns = 5\n").expect("rewrite");
        live.refresh();
        assert_eq!(live.settings.columns, crate::ui::ColumnsSetting::Pinned(5));

        // A bad edit keeps the last-good value, like every setting.
        std::fs::write(&path, b"columns = 42\n").expect("rewrite bad");
        live.refresh();
        assert_eq!(
            live.settings.columns,
            crate::ui::ColumnsSetting::Pinned(5),
            "an out-of-range columns must not apply"
        );
    }

    /// The geometry twin of the frozen-palette seam: the style profile
    /// is resolved once at session open and frozen for the session's
    /// life — a density (or style) flip landing mid-gesture (here: the
    /// profile the next invocation would resolve) never re-rhythms the
    /// open card; the next press starts fresh against it.
    #[test]
    fn an_open_session_keeps_its_frozen_profile_across_a_density_flip() {
        let snap = snapshot(
            vec![
                toplevel(1, &[1], State::ACTIVATED, 30),
                toplevel(2, &[1], State::default(), 20),
            ],
            Some(1),
        );
        let spacious = crate::ui::StyleProfile::resolve(
            crate::theme::Style::Round,
            crate::theme::Density::Spacious,
        );
        let compact = crate::ui::StyleProfile::resolve(
            crate::theme::Style::Square,
            crate::theme::Density::Compact,
        );
        let mut open = None;
        session_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            false,
            Palette::dark(),
            spacious,
            DEFAULT_ROWS,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut HashSet::new(),
        );
        assert_eq!(open.as_ref().expect("session opened").profile, spacious);

        // The desktop flips Square + Compact between presses; the open
        // session still shapes the card it was opened with.
        session_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            false,
            Palette::dark(),
            compact,
            DEFAULT_ROWS,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut HashSet::new(),
        );
        assert_eq!(
            open.as_ref().expect("session still open").profile,
            spacious,
            "a mid-gesture density flip must not re-rhythm the open session"
        );

        // The next press after the session ends starts fresh: compact.
        open = None;
        session_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            false,
            Palette::dark(),
            compact,
            DEFAULT_ROWS,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut HashSet::new(),
        );
        assert_eq!(
            open.as_ref().expect("fresh session opened").profile,
            compact
        );
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
            StyleProfile::default(),
            DEFAULT_ROWS,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut HashSet::new(),
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
            StyleProfile::default(),
            DEFAULT_ROWS,
            true,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut HashSet::new(),
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
            StyleProfile::default(),
            DEFAULT_ROWS,
            true,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut HashSet::new(),
        );
        assert!(open.as_ref().expect("fresh session opened").grouping);
    }

    // --- Grouped sessions --------------------------------------------------

    /// Two two-window apps, MRU a1, a2, b1, b2. The frozen tree reads
    /// [P-a1, C-a1, C-a2, P-b1, C-b1, C-b2] — the last-active app
    /// fronts the list, each parent fronting its own group with its
    /// whole window set as children (front window first).
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

    /// One multi-window app beside one single-window app: MRU a1, a2,
    /// c1. Tree: [P-a1, C-a1, C-a2, L-c1] — the lone-parent shape that
    /// dead-keyed the first grouped Tab.
    fn grouped_lone_parent() -> Snapshot {
        snapshot(
            vec![
                app_toplevel(1, "app.a", &[1], State::ACTIVATED, 40),
                app_toplevel(2, "app.a", &[1], State::default(), 30),
                app_toplevel(3, "app.c", &[1], State::default(), 20),
            ],
            Some(1),
        )
    }

    /// Opens a flat session on `snap` (one opening Alt+Tab) and
    /// returns it with empty expansion state — the `type-filter` pins'
    /// base.
    fn flat_session(snap: &Snapshot) -> (Option<Switch>, HashSet<String>) {
        let mut open = None;
        let mut expanded = HashSet::new();
        session_step(
            &mut open,
            snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            false,
            Palette::dark(),
            StyleProfile::default(),
            DEFAULT_ROWS,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut expanded,
        );
        (open, expanded)
    }

    /// Opens a grouped session on `snap` (one opening Alt+Tab) and
    /// returns it with its expansion state.
    fn grouped_session(snap: &Snapshot) -> (Option<Switch>, HashSet<String>) {
        let mut open = None;
        let mut expanded = HashSet::new();
        session_step(
            &mut open,
            snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            false,
            Palette::dark(),
            StyleProfile::default(),
            DEFAULT_ROWS,
            true,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut expanded,
        );
        (open, expanded)
    }

    #[test]
    fn rev5_opening_press_steps_past_the_active_window() {
        // The active app's group stood expanded (the state persists), so
        // the header and its first child both carry the active window
        // a1: the opening press steps past *both* rows and lands on
        // a2 — flat quick-tap parity — never resting on the window the
        // gesture started from.
        let snap = grouped_two_apps();
        let mut expanded = HashSet::new();
        expanded.insert("app.a".to_string());
        let mut open = None;
        session_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            false,
            Palette::dark(),
            StyleProfile::default(),
            DEFAULT_ROWS,
            true,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut expanded,
        );
        assert_eq!(
            open.as_ref().expect("opened").tree_sel,
            2,
            "one past the active a1, onto a2"
        );
        assert_eq!(
            open.as_ref().expect("opened").selected_handle(),
            Some(Handle(2))
        );
    }

    #[test]
    fn rev5_collapsed_tab_steps_the_entries_linearly() {
        // Nothing expanded: the visible sequence is exactly the
        // level-1 entries, and Tab steps them linearly, wrapping.
        let snap = grouped_two_apps();
        let (mut open, expanded) = grouped_session(&snap);
        assert_eq!(
            open.as_ref().expect("open").tree_sel,
            3,
            "the opening press steps to b's entry"
        );
        step_visible(
            open.as_mut().expect("open"),
            &snap,
            &expanded,
            Direction::Next,
        );
        assert_eq!(open.as_ref().expect("open").tree_sel, 0);
        step_visible(
            open.as_mut().expect("open"),
            &snap,
            &expanded,
            Direction::Previous,
        );
        assert_eq!(
            open.as_ref().expect("open").tree_sel,
            3,
            "Previous returns to b's entry"
        );
    }

    #[test]
    fn rev5_linear_steps_stop_on_expanded_headers() {
        // Rev 5: headers are normal rows — the walk from a's last child
        // stops on b's entry, then its children, and the wrap from b's
        // last child lands on a's entry. Nothing is skipped, nothing is
        // unrestable.
        let snap = grouped_two_apps();
        let (mut open, mut expanded) = grouped_session(&snap);
        expanded.insert("app.a".to_string());
        expanded.insert("app.b".to_string());
        open.as_mut().expect("open").tree_sel = 2; // a's last child
        step_visible(
            open.as_mut().expect("open"),
            &snap,
            &expanded,
            Direction::Next,
        );
        assert_eq!(
            open.as_ref().expect("open").tree_sel,
            3,
            "b's entry is a stop"
        );
        step_visible(
            open.as_mut().expect("open"),
            &snap,
            &expanded,
            Direction::Next,
        );
        assert_eq!(
            open.as_ref().expect("open").tree_sel,
            4,
            "then its first child"
        );
        open.as_mut().expect("open").tree_sel = 5; // b's last child
        step_visible(
            open.as_mut().expect("open"),
            &snap,
            &expanded,
            Direction::Next,
        );
        assert_eq!(
            open.as_ref().expect("open").tree_sel,
            0,
            "wraps onto a's entry"
        );
    }

    #[test]
    fn rev5_a_lone_collapsed_group_is_never_entered() {
        // Rev 5: nothing descends into a collapsed group — a lone entry
        // keeps the selection, and the press is a stay, never an entry.
        let snap = snapshot(
            vec![
                app_toplevel(1, "app.a", &[1], State::ACTIVATED, 40),
                app_toplevel(2, "app.a", &[1], State::default(), 30),
            ],
            Some(1),
        );
        let (mut open, expanded) = grouped_session(&snap);
        assert_eq!(
            open.as_ref().expect("opened").tree_sel,
            0,
            "the entry holds the selection"
        );
        assert_eq!(
            open.as_ref().expect("opened").selected_handle(),
            Some(Handle(1)),
            "committing it activates the app's front window"
        );
        step_visible(
            open.as_mut().expect("open"),
            &snap,
            &expanded,
            Direction::Next,
        );
        assert_eq!(
            open.as_ref().expect("open").tree_sel,
            0,
            "a lone row wraps onto itself"
        );
        assert!(expanded.is_empty(), "nothing expanded the group");
    }

    #[test]
    fn rev5_left_on_a_child_exits_without_collapsing() {
        // Rev 5: the header can hold the selection, so ←'s exit no
        // longer merges with a collapse — the group stays expanded.
        let snap = grouped_two_apps();
        let (mut open, mut expanded) = grouped_session(&snap);
        expanded.insert("app.a".to_string());
        open.as_mut().expect("open").tree_sel = 2; // a's second child
        assert!(exit_group(open.as_mut().expect("open"), &mut expanded));
        assert_eq!(
            open.as_ref().expect("open").tree_sel,
            0,
            "lands on a's entry"
        );
        assert!(expanded.contains("app.a"), "the group stays expanded");
    }

    #[test]
    fn rev5_the_pointer_selects_an_expanded_header() {
        // Rev 5: the divider rule is gone — hovering the expanded
        // entry selects that row itself, exactly like any other row.
        let snap = grouped_two_apps();
        let (mut open, mut expanded) = grouped_session(&snap);
        expanded.insert("app.a".to_string());
        let session = open.as_mut().expect("open");
        session.visible = grouped_visible(session, &snap, &expanded);
        session.viewport = Some(crate::ui::Viewport {
            start: 0,
            content: 4, // P-a1, C-a1, C-a2, P-b1 — b collapsed
            top: false,
            bottom: false,
        });
        assert!(
            open.as_mut()
                .expect("open")
                .select_pointer_row(0, Some(Handle(1))),
            "the expanded header selects"
        );
        assert_eq!(
            open.as_ref().expect("open").tree_sel,
            0,
            "the header row holds the selection"
        );
    }

    /// Like [`app_toplevel`], with the title under test's choosing.
    fn titled(handle: u64, title: &str, app: &str, last: u64) -> Toplevel {
        Toplevel {
            title: title.to_string(),
            app_id: app.to_string(),
            ..toplevel(handle, &[1], State::default(), last)
        }
    }

    #[test]
    fn typing_narrows_a_flat_session_and_selects_the_first_match() {
        // "fir" matches both Firefox windows in MRU order; the first
        // (most recent) is selected, and stepping stays inside the
        // narrowed set — the editor never surfaces until the query
        // clears.
        let snap = snapshot(
            vec![
                titled(1, "invoice.xlsx", "office", 40),
                titled(2, "firefox — docs", "firefox", 30),
                titled(3, "firefox — mail", "firefox", 20),
                titled(4, "editor", "code", 10),
            ],
            Some(1),
        );
        let (mut open, mut expanded) = flat_session(&snap);
        assert!(type_into(&mut open, &snap, 'f'));
        assert!(type_into(&mut open, &snap, 'i'));
        assert!(type_into(&mut open, &snap, 'r'));
        let session = open.as_ref().expect("open");
        assert_eq!(session.selected_handle(), Some(Handle(2)), "first match");
        // One step lands on the second match, never the editor.
        session_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            false,
            Palette::dark(),
            StyleProfile::default(),
            DEFAULT_ROWS,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut expanded,
        );
        assert_eq!(
            open.as_ref().expect("open").selected_handle(),
            Some(Handle(3))
        );
    }

    #[test]
    fn typing_flattens_a_grouped_session() {
        // The querying grouped card is the flat narrowed list: the tree,
        // headers, and expansion state are bypassed while the query
        // stands — a window inside a collapsed group matches all the
        // same.
        let snap = snapshot(
            vec![
                titled(1, "shell 1", "term", 40),
                titled(2, "shell 2", "term", 30),
                titled(3, "report.pdf", "office", 20),
            ],
            Some(1),
        );
        let (mut open, expanded) = grouped_session(&snap);
        assert!(expanded.is_empty(), "collapsed by default");
        assert!(type_into(&mut open, &snap, 's'));
        assert!(type_into(&mut open, &snap, 'h'));
        let session = open.as_ref().expect("open");
        assert_eq!(session.query, "sh");
        assert_eq!(
            session.filtered.as_ref().expect("narrowed").handles(),
            &[Handle(1), Handle(2)]
        );
        assert_eq!(session.selected_handle(), Some(Handle(1)));
    }

    #[test]
    fn erasing_to_empty_restores_the_kept_selection() {
        // Type, step onto the second match, backspace it all away: the
        // unfiltered session returns with the same window still
        // selected — the kept-window restore rule.
        let snap = snapshot(
            vec![
                titled(1, "invoice.xlsx", "office", 40),
                titled(2, "firefox — docs", "firefox", 30),
                titled(3, "editor", "code", 20),
            ],
            Some(1),
        );
        let (mut open, expanded) = flat_session(&snap);
        assert!(type_into(&mut open, &snap, 'f'));
        if let Some(filtered) = open.as_mut().expect("open").filtered.as_mut() {
            filtered.select(Handle(2));
        }
        assert!(erase_query(&mut open, &snap, &expanded));
        assert!(erase_query(&mut open, &snap, &expanded));
        let session = open.as_ref().expect("open");
        assert!(session.query.is_empty());
        assert!(session.filtered.is_none(), "the narrowed ring dissolved");
        assert_eq!(
            session.selected_handle(),
            Some(Handle(2)),
            "the kept window"
        );
    }

    #[test]
    fn backspace_on_an_empty_query_changes_nothing() {
        let snap = snapshot(vec![titled(1, "solo", "app", 1)], Some(1));
        let (mut open, expanded) = flat_session(&snap);
        assert!(erase_query(&mut open, &snap, &expanded));
        let session = open.as_ref().expect("still open");
        assert!(session.query.is_empty());
        assert_eq!(session.selected_handle(), Some(Handle(1)));
    }

    #[test]
    fn the_query_caps_at_sixty_four_characters() {
        let snap = snapshot(vec![titled(1, "a", "app", 1)], Some(1));
        let (mut open, _) = flat_session(&snap);
        for _ in 0..80 {
            assert!(type_into(&mut open, &snap, 'a'));
        }
        assert_eq!(open.as_ref().expect("open").query.chars().count(), 64);
    }

    #[test]
    fn no_match_holds_the_session_with_nothing_selected() {
        // Nothing matches: the narrowed ring is empty, the selection is
        // None (committing is a no-op), and the session stands — the
        // card's no-matches row is the render's business.
        let snap = snapshot(vec![titled(1, "editor", "code", 1)], Some(1));
        let (mut open, _) = flat_session(&snap);
        assert!(type_into(&mut open, &snap, 'z'));
        let session = open.as_ref().expect("open");
        assert_eq!(session.selected_handle(), None);
        assert_eq!(
            commit_target(session, &snap),
            None,
            "a no-match commit no-ops"
        );
    }

    #[test]
    fn clearing_the_query_reports_whether_it_cleared() {
        // Escape's decision rule: a standing query clears (true), an
        // empty one reports false — the caller dismisses.
        let snap = snapshot(vec![titled(1, "editor", "code", 1)], Some(1));
        let (mut open, expanded) = flat_session(&snap);
        assert!(
            !clear_query(&mut open, &snap, &expanded),
            "empty query: dismiss"
        );
        assert!(type_into(&mut open, &snap, 'e'));
        assert!(
            clear_query(&mut open, &snap, &expanded),
            "standing query: clear"
        );
        assert!(open.as_ref().expect("open").query.is_empty());
    }

    #[test]
    fn closing_advances_to_the_next_live_row() {
        // Flat session over a, b, c (MRU): closing the selected b moves
        // the selection to c — the walk's next live row, the doomed one
        // excluded — and closing the last row wraps to the front.
        let snap = snapshot(
            vec![
                titled(1, "a", "app.a", 30),
                titled(2, "b", "app.b", 20),
                titled(3, "c", "app.c", 10),
            ],
            Some(1),
        );
        let (mut open, expanded) = flat_session(&snap);
        open.as_mut().expect("open").ring.select(Handle(2));
        assert!(advance_off_doomed(
            open.as_mut().expect("open"),
            &snap,
            &expanded,
            Handle(2)
        ));
        assert_eq!(
            open.as_ref().expect("open").selected_handle(),
            Some(Handle(3))
        );
        // The wrap: closing c (now selected) falls to a.
        assert!(advance_off_doomed(
            open.as_mut().expect("open"),
            &snap,
            &expanded,
            Handle(3)
        ));
        assert_eq!(
            open.as_ref().expect("open").selected_handle(),
            Some(Handle(1))
        );
    }

    #[test]
    fn closing_the_sole_window_reports_nothing_left() {
        let snap = snapshot(vec![titled(1, "solo", "app", 1)], Some(1));
        let (mut open, expanded) = flat_session(&snap);
        assert!(!advance_off_doomed(
            open.as_mut().expect("open"),
            &snap,
            &expanded,
            Handle(1)
        ));
    }

    #[test]
    fn closing_a_group_header_advances_past_its_whole_window() {
        // The header and its front child are one window: closing the
        // selected header advances to the *other* app's entry, never
        // onto the front child that carries the same doomed handle.
        let snap = grouped_two_apps();
        let (mut open, expanded) = grouped_session(&snap);
        // The opening press left the selection on b's entry (row 3);
        // walk home to a's header first.
        open.as_mut().expect("open").tree_sel = 0;
        assert!(advance_off_doomed(
            open.as_mut().expect("open"),
            &snap,
            &expanded,
            Handle(1)
        ));
        assert_eq!(
            open.as_ref().expect("open").selected_handle(),
            Some(Handle(3)),
            "lands on b's entry — a's own child a2 is not next in sequence"
        );
    }

    #[test]
    fn closing_within_a_query_stays_inside_the_matches() {
        // A querying session closes in its narrowed sequence: the
        // doomed match's successor is another match, not the unfiltered
        // list's next row.
        let snap = snapshot(
            vec![
                titled(1, "firefox — docs", "firefox", 40),
                titled(2, "editor", "code", 30),
                titled(3, "firefox — mail", "firefox", 20),
            ],
            Some(1),
        );
        let (mut open, expanded) = flat_session(&snap);
        assert!(type_into(&mut open, &snap, 'f'));
        assert!(advance_off_doomed(
            open.as_mut().expect("open"),
            &snap,
            &expanded,
            Handle(1)
        ));
        assert_eq!(
            open.as_ref().expect("open").selected_handle(),
            Some(Handle(3)),
            "the second match, never the editor"
        );
    }

    #[test]
    fn closing_a_window_the_selection_is_not_on_moves_nothing() {
        // The pointer path: the × closes the hovered row's window while
        // the keyboard's selection rests elsewhere — a close that does
        // not sit under the selection never moves it.
        let snap = snapshot(
            vec![
                titled(1, "a", "app.a", 30),
                titled(2, "b", "app.b", 20),
                titled(3, "c", "app.c", 10),
            ],
            Some(1),
        );
        let (mut open, expanded) = flat_session(&snap);
        open.as_mut().expect("open").ring.select(Handle(3));
        assert!(prepare_close(
            open.as_mut().expect("open"),
            &snap,
            &expanded,
            Handle(1)
        ));
        assert_eq!(
            open.as_ref().expect("open").selected_handle(),
            Some(Handle(3)),
            "the selection stays on its own window"
        );
    }

    #[test]
    fn a_close_forgets_the_pointer_s_hovered_row() {
        // The card recomposes around a close, so the pointer's resting
        // row is stale the moment rows shift: the hover is forgotten,
        // and the × returns only when the pointer moves again.
        let snap = snapshot(
            vec![titled(1, "a", "app.a", 30), titled(2, "b", "app.b", 20)],
            Some(1),
        );
        let (mut open, expanded) = flat_session(&snap);
        open.as_mut().expect("open").hovered = Some((1, Handle(2)));
        assert!(prepare_close(
            open.as_mut().expect("open"),
            &snap,
            &expanded,
            Handle(2)
        ));
        assert_eq!(
            open.as_ref().expect("open").hovered,
            None,
            "the recomposed card reveals no × until the pointer moves"
        );
    }

    #[test]
    fn hovered_row_validates_against_the_built_model() {
        // The × paints only where the pointer's pair (row, handle)
        // still matches the rows the card just built: a row past the
        // end, or a row whose window is no longer there, reveals
        // nothing — a stale index never offers a wrong window's close.
        let model = OverlayModel {
            rows: vec![RowModel {
                handle: Some(Handle(1)),
                ..RowModel::default()
            }],
            ..OverlayModel::default()
        };
        assert_eq!(hovered_row(&model, Some((0, Handle(1)))), Some(0));
        assert_eq!(
            hovered_row(&model, Some((1, Handle(1)))),
            None,
            "a row past the model's end"
        );
        assert_eq!(
            hovered_row(&model, Some((0, Handle(2)))),
            None,
            "the row no longer carries that window"
        );
        assert_eq!(hovered_row(&model, None), None);
    }

    #[test]
    fn a_grouped_quick_tap_selects_the_second_apps_front_window() {
        // The opening Alt+Tab steps to the next *app entry* — level 1,
        // never descending — so a quick tap activates the MRU-second
        // app's front window: here b1 (the implied [alt]+1 of the b
        // entry), where the flat ring would step to a2.
        let snap = grouped_two_apps();
        let (open, _) = grouped_session(&snap);
        assert_eq!(
            open.as_ref().expect("opened").selected_handle(),
            Some(Handle(3)),
            "quick-tap lands on b's front window"
        );

        let mut flat = None;
        let flat_target = session_step(
            &mut flat,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            false,
            Palette::dark(),
            StyleProfile::default(),
            DEFAULT_ROWS,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut HashSet::new(),
        );
        assert_eq!(flat_target, Some(Handle(2)), "flat keeps the MRU-second");
    }

    #[test]
    fn the_last_active_app_fronts_its_group_as_the_top_parent_row() {
        // The owner's stated contract: the most recently active app
        // sits at the top of the list as its group's parent — a pure
        // layer entry whose implied window is the app's own most
        // recently active window, its first child.
        let snap = grouped_two_apps();
        let (open, expanded) = grouped_session(&snap);
        let session = open.as_ref().expect("opened");
        let tree = session.tree.as_ref().expect("grouped session");
        assert_eq!(tree.rows()[0].kind, switcher::RowKind::Parent);
        assert_eq!(tree.rows()[0].app_id, "app.a");
        assert_eq!(
            tree.rows()[0].handle,
            Handle(1),
            "the entry carries its app's front window"
        );
        // Expanded, the first child is that same window — [alt]+1.
        let mut expanded = expanded;
        expanded.insert("app.a".to_string());
        let visible = grouped_visible(session, &snap, &expanded);
        assert_eq!(tree.rows()[1].kind, switcher::RowKind::Child);
        assert_eq!(tree.rows()[1].handle, Handle(1));
        assert!(visible.contains(&1), "expansion shows the front child");
    }

    #[test]
    fn collapsed_tab_steps_entries_and_wraps() {
        // Collapsed, the visible sequence is the entries — parents and
        // leaves alike — and Tab steps it linearly, wrapping; a child
        // is never visible, so never landed on, while collapsed.
        let snap = grouped_lone_parent();
        let (mut open, expanded) = grouped_session(&snap);
        assert_eq!(
            open.as_ref().expect("open").selected_handle(),
            Some(Handle(3)),
            "opening press: a's entry → c's leaf"
        );
        step_visible(
            open.as_mut().expect("open"),
            &snap,
            &expanded,
            Direction::Next,
        );
        assert_eq!(
            open.as_ref().expect("open").selected_handle(),
            Some(Handle(1)),
            "wraps back to a's entry"
        );
        step_visible(
            open.as_mut().expect("open"),
            &snap,
            &expanded,
            Direction::Previous,
        );
        assert_eq!(
            open.as_ref().expect("open").selected_handle(),
            Some(Handle(3)),
            "Previous returns to c's leaf"
        );
    }

    #[test]
    fn visible_steps_walk_the_shown_rows_linearly() {
        // ↓/↑ and the wheel walk the *visible* sequence: collapsed, the
        // children are skipped exactly as they are not shown; expanded,
        // the walk threads through them and across group boundaries.
        let snap = grouped_lone_parent();
        let (mut open, expanded) = grouped_session(&snap);
        // Collapsed: [P-a1, L-c1] — the step from c1 wraps to a1.
        step_visible(
            open.as_mut().expect("open"),
            &snap,
            &expanded,
            Direction::Previous,
        );
        assert_eq!(open.as_ref().expect("open").tree_sel, 0);
        // Expanded: [P-a1, C-a1, C-a2, L-c1] — the walk threads the
        // children, one row per step.
        let mut expanded = expanded;
        expanded.insert("app.a".to_string());
        step_visible(
            open.as_mut().expect("open"),
            &snap,
            &expanded,
            Direction::Next,
        );
        assert_eq!(open.as_ref().expect("open").tree_sel, 1);
        step_visible(
            open.as_mut().expect("open"),
            &snap,
            &expanded,
            Direction::Next,
        );
        assert_eq!(open.as_ref().expect("open").tree_sel, 2);
        step_visible(
            open.as_mut().expect("open"),
            &snap,
            &expanded,
            Direction::Next,
        );
        assert_eq!(
            open.as_ref().expect("open").tree_sel,
            3,
            "crosses to the leaf"
        );
    }

    #[test]
    fn entering_owns_group_skips_the_active_window() {
        // The owner's Firefox trace: entering your own app's group
        // never lands on the window the gesture started from — the
        // selection steps one past it (a2), exactly as the opening
        // Alt+Tab steps past the active window.
        let snap = grouped_two_apps();
        let (mut open, mut expanded) = grouped_session(&snap);
        // The opening press moved the selection to b; walk back to a's
        // entry and enter it.
        open.as_mut().expect("open").tree_sel = 0; // a's entry; active = a1
        assert!(enter_group(
            open.as_mut().expect("open"),
            &snap,
            &mut expanded,
            Direction::Next
        ));
        assert_eq!(
            open.as_ref().expect("open").tree_sel,
            2,
            "enters past the active a1 onto a2"
        );
        assert!(
            expanded.contains("app.a"),
            "entering expands the group, persistently"
        );
    }

    #[test]
    fn entering_another_group_lands_on_its_front_window() {
        // Any other app's group: → lands on its most recently used
        // window — the implied [alt]+1 — and expands it too.
        let snap = grouped_two_apps();
        let (mut open, mut expanded) = grouped_session(&snap);
        // The opening press left the selection on b's entry.
        assert!(enter_group(
            open.as_mut().expect("open"),
            &snap,
            &mut expanded,
            Direction::Next
        ));
        assert_eq!(
            open.as_ref().expect("open").tree_sel,
            4,
            "b's front child ([alt]+1)"
        );
        assert!(expanded.contains("app.b"));
    }

    #[test]
    fn entering_a_leaf_is_the_w3c_no_op() {
        // The end-node rule: a single-window app has no children to
        // enter, and nothing expands.
        let snap = grouped_lone_parent();
        let (mut open, mut expanded) = grouped_session(&snap);
        // The opening press left the selection on c's leaf.
        assert!(!enter_group(
            open.as_mut().expect("open"),
            &snap,
            &mut expanded,
            Direction::Next
        ));
        assert_eq!(open.as_ref().expect("open").tree_sel, 3);
        assert!(expanded.is_empty());
    }

    #[test]
    fn left_arrow_exits_a_child_and_collapses_a_parent() {
        // ← on a child returns to its parent row; on an expanded
        // parent it collapses the group (the state persists); on a
        // collapsed entry and on a leaf it is the W3C root no-op.
        let snap = grouped_two_apps();
        let (mut open, expanded) = grouped_session(&snap);
        let session = open.as_mut().expect("open");
        session.tree_sel = 2; // a's second child
        let mut expanded = expanded;
        assert!(exit_group(open.as_mut().expect("open"), &mut expanded));
        assert_eq!(
            open.as_ref().expect("open").tree_sel,
            0,
            "exits to a's entry"
        );
        // A collapsed entry's exit moves nothing.
        assert!(!exit_group(open.as_mut().expect("open"), &mut expanded));
        // Expanded entry: the exit collapses it, selection holding.
        expanded.insert("app.a".to_string());
        assert!(exit_group(open.as_mut().expect("open"), &mut expanded));
        assert_eq!(open.as_ref().expect("open").tree_sel, 0);
        assert!(!expanded.contains("app.a"), "collapsed, persistently");
        // A leaf's exit moves nothing either.
        open.as_mut().expect("open").tree_sel = 3; // b's entry, collapsed
        let _ = &mut expanded;
        open.as_mut().expect("open").tree_sel = 5; // b's second child
        assert!(exit_group(open.as_mut().expect("open"), &mut expanded));
        open.as_mut().expect("open").tree_sel = 4;
    }

    #[test]
    fn expansion_state_persists_across_gestures() {
        // Collapsed by default at the first gesture; whatever the
        // gesture expanded, the next gesture's card shows expanded.
        let snap = grouped_two_apps();
        let (mut open, mut expanded) = grouped_session(&snap);
        enter_group(
            open.as_mut().expect("open"),
            &snap,
            &mut expanded,
            Direction::Next,
        );
        let second = grouped_visible(open.as_ref().expect("open"), &snap, &expanded);
        assert!(
            second.contains(&4) && second.contains(&5),
            "b's children show"
        );
        // A fresh session over the same expansion state starts
        // expanded where the last one left it.
        open = None;
        session_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            false,
            Palette::dark(),
            StyleProfile::default(),
            DEFAULT_ROWS,
            true,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut expanded,
        );
        let visible = grouped_visible(open.as_ref().expect("reopened"), &snap, &expanded);
        assert!(
            visible.contains(&4),
            "the next gesture inherits the expansion"
        );
    }

    #[test]
    fn the_grouped_tree_freezes_at_open() {
        // A window of a new app activating mid-gesture changes nothing:
        // the frozen tree has no row for it, so it is neither visible
        // on the card nor addressable into the session — regrouping
        // waits for the next session.
        let snap = grouped_two_apps();
        let (mut open, expanded) = grouped_session(&snap);
        let mut drifted = grouped_two_apps();
        drifted
            .toplevels
            .push(app_toplevel(5, "app.c", &[1], State::ACTIVATED, 99));
        let visible = grouped_visible(open.as_ref().expect("open"), &drifted, &expanded);
        assert!(
            !visible.iter().any(|&index| index > 5),
            "app.c never joined the card"
        );
        assert_eq!(
            open.as_mut()
                .expect("open")
                .select_handle(Handle(5))
                .map(|_| ()),
            None,
            "a window opened mid-gesture cannot be selected into the session"
        );
    }

    #[test]
    fn a_group_front_window_selects_its_parent_row() {
        // The parent and its first child carry the same window; a
        // handle-only selection (a click, a Ctrl+N commit) resolves to
        // the parent row — the entry — and commits the same window
        // either way.
        let snap = grouped_two_apps();
        let (mut open, expanded) = grouped_session(&snap);
        let session = open.as_mut().expect("open");
        session.visible = grouped_visible(session, &snap, &expanded);
        assert!(
            open.as_mut()
                .expect("open")
                .select_handle(Handle(1))
                .is_some()
        );
        assert_eq!(
            open.as_ref().expect("open").tree_sel,
            0,
            "the ambiguous front window selects the entry row"
        );
    }

    #[test]
    fn pointer_rows_map_through_the_stored_visible_sequence() {
        // A hover crossing addresses its model row exactly: content
        // rows map through visible + viewport; continuation rows are
        // inert. The row index is what disambiguates a group's front
        // window from its entry.
        let snap = grouped_two_apps();
        let (mut open, expanded) = grouped_session(&snap);
        let mut expanded = expanded;
        expanded.insert("app.a".to_string());
        expanded.insert("app.b".to_string());
        let session = open.as_mut().expect("open");
        session.visible = grouped_visible(session, &snap, &expanded);
        // A synthetic viewport: rows 1..3 of the 6-row visible list,
        // the bottom slot a continuation row.
        session.viewport = Some(crate::ui::Viewport {
            start: 1,
            content: 2,
            top: false,
            bottom: true,
        });
        let session = open.as_ref().expect("open");
        assert_eq!(session.tree_row_at(0), Some(1));
        assert_eq!(session.tree_row_at(1), Some(2));
        assert_eq!(session.tree_row_at(2), None, "the bottom `…` row is inert");
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
            StyleProfile::default(),
            DEFAULT_ROWS,
            true,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut HashSet::new(),
        );
        let session = open.as_ref().expect("session opened");
        assert!(session.tree.is_none(), "same-app sessions carry no tree");
        assert_eq!(session.ring.selected(), Some(Handle(3)));
    }

    // --- the text decision (typography TY4) --------------------------------

    /// The committed fixture's text system (see `res/test-fonts`).
    fn fixture_text_system() -> crate::text::TextSystem {
        crate::text::TextSystem::from_dir(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("res/test-fonts"),
        )
    }

    #[test]
    fn resolve_text_paints_the_bitmap_floor_when_no_fonts_load() {
        // SC 5: an injected empty fontdb paints the font8x8 card.
        let mut empty = crate::text::TextSystem::empty();
        assert_eq!(empty.face_count(), 0);
        assert_eq!(
            resolve_text(&mut empty, &crate::text::FontSetting::Default, || {
                "Follow Face".to_owned()
            }),
            crate::text::TextDecision::Bitmap
        );
        // Even a configured family cannot reach the System stack
        // against an empty database — the floor is the floor.
        assert_eq!(
            resolve_text(
                &mut empty,
                &crate::text::FontSetting::Family("Open Sans".to_owned()),
                || "Follow Face".to_owned()
            ),
            crate::text::TextDecision::Bitmap
        );
    }

    #[test]
    fn resolve_text_keeps_a_configured_family_the_database_knows() {
        let mut system = fixture_text_system();
        assert_eq!(
            resolve_text(
                &mut system,
                &crate::text::FontSetting::Family("Open Sans".to_owned()),
                || unreachable!("an installed family never reads the follow store")
            ),
            crate::text::TextDecision::System {
                family: "Open Sans".to_owned()
            }
        );
    }

    #[test]
    fn resolve_text_falls_back_to_the_follow_for_unknown_and_default() {
        let mut system = fixture_text_system();
        // A configured-but-missing family: the diagnostic names the key
        // (stderr) and the follow's answer paints.
        assert_eq!(
            resolve_text(
                &mut system,
                &crate::text::FontSetting::Family("Not Installed".to_owned()),
                || "Follow Face".to_owned()
            ),
            crate::text::TextDecision::System {
                family: "Follow Face".to_owned()
            }
        );
        // Default: the follow's answer, nothing else consulted.
        assert_eq!(
            resolve_text(&mut system, &crate::text::FontSetting::Default, || {
                "Follow Face".to_owned()
            }),
            crate::text::TextDecision::System {
                family: "Follow Face".to_owned()
            }
        );
    }

    /// The text twin of the frozen-palette seam: the decision resolves
    /// once at open and freezes for the session's life — a font edit
    /// landing mid-gesture (here: the decision the next invocation
    /// would resolve) changes nothing until the session closes and the
    /// next press starts fresh.
    #[test]
    fn an_open_session_keeps_its_frozen_text_across_a_font_edit() {
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
            StyleProfile::default(),
            DEFAULT_ROWS,
            false,
            crate::text::TextDecision::System {
                family: "Open Sans".to_owned(),
            },
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut HashSet::new(),
        );
        assert_eq!(
            open.as_ref().expect("session opened").text,
            crate::text::TextDecision::System {
                family: "Open Sans".to_owned()
            }
        );

        // The config flips fonts between presses; the open session
        // still paints the face it was opened with.
        session_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            false,
            Palette::dark(),
            StyleProfile::default(),
            DEFAULT_ROWS,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut HashSet::new(),
        );
        assert_eq!(
            open.as_ref().expect("session still open").text,
            crate::text::TextDecision::System {
                family: "Open Sans".to_owned()
            },
            "a mid-gesture font edit must not repaint the open session"
        );

        // The next press after the session ends starts fresh: the new
        // decision applies.
        open = None;
        session_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            false,
            Palette::dark(),
            StyleProfile::default(),
            DEFAULT_ROWS,
            false,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Vertical,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut HashSet::new(),
        );
        assert_eq!(
            open.as_ref().expect("fresh session opened").text,
            crate::text::TextDecision::Bitmap
        );
    }

    // --- grouped-horizontal: the combined session (GH1) -------------------

    /// Opens a combined (grouped-horizontal) session on `snap` with one
    /// opening Alt+Tab — the grouped twin with a horizontal layout.
    fn combined_session(snap: &Snapshot) -> (Option<Switch>, HashSet<String>) {
        let mut open = None;
        let mut expanded = HashSet::new();
        session_step(
            &mut open,
            snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            false,
            Palette::dark(),
            StyleProfile::default(),
            DEFAULT_ROWS,
            true,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Horizontal,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut expanded,
        );
        (open, expanded)
    }

    /// Three two-window apps, MRU a1, a2, b1, b2, c1, c2. The frozen
    /// tree reads [P-a1, C-a1, C-a2, P-b1, C-b1, C-b2, P-c1, C-c1,
    /// C-c2] — apps at tree indices 0, 3, 6.
    fn combined_three_apps() -> Snapshot {
        snapshot(
            vec![
                app_toplevel(1, "app.a", &[1], State::ACTIVATED, 60),
                app_toplevel(2, "app.a", &[1], State::default(), 50),
                app_toplevel(3, "app.b", &[1], State::default(), 40),
                app_toplevel(4, "app.b", &[1], State::default(), 30),
                app_toplevel(5, "app.c", &[1], State::default(), 20),
                app_toplevel(6, "app.c", &[1], State::default(), 10),
            ],
            Some(1),
        )
    }

    #[test]
    fn combined_opening_press_lands_on_the_second_app_tile() {
        // The quick-tap criterion: the opening press steps the app band
        // once — the second app tile — and release activates that app's
        // MRU-front window.
        let snap = grouped_two_apps();
        let (open, _expanded) = combined_session(&snap);
        let session = open.as_ref().expect("session opened");
        assert_eq!(session.tree_sel, 3, "b's parent tile");
        assert_eq!(session.band, Some(crate::ui::Band::Apps));
        assert_eq!(session.selected_handle(), Some(Handle(3)));
    }

    #[test]
    fn combined_tab_steps_apps_from_anywhere() {
        // Tab steps the app list from either band: from band 2 it lands
        // on the next app after the focused one, focus returning to
        // band 1; Shift+Tab wraps.
        let snap = grouped_two_apps();
        let (mut open, _expanded) = combined_session(&snap);
        let session = open.as_mut().expect("open");
        session.tree_sel = 1; // a's front child, band 2
        session.band = Some(crate::ui::Band::Windows);
        step_apps(session, &snap, Direction::Next);
        assert_eq!(session.tree_sel, 3, "the next app after a");
        assert_eq!(session.band, Some(crate::ui::Band::Apps));
        step_apps(session, &snap, Direction::Next);
        assert_eq!(session.tree_sel, 0, "wraps to a");
        step_apps(session, &snap, Direction::Previous);
        assert_eq!(session.tree_sel, 3, "wraps back");
    }

    #[test]
    fn combined_arrows_step_the_focused_band() {
        // ←/→ step within whichever band holds the selection: the app
        // list in band 1 (band 2 swaps with the new app), the focused
        // app's windows in band 2 — wrapping per the ring semantics.
        let snap = grouped_two_apps();
        let (mut open, _expanded) = combined_session(&snap);
        // Band 1: b's parent (3) → wraps to a (0).
        let session = open.as_mut().expect("open");
        session.tree_sel = 3;
        session.band = Some(crate::ui::Band::Apps);
        step_band(session, &snap, Direction::Next);
        assert_eq!(session.tree_sel, 0, "band 1 wraps");
        // Band 2: a's front child (1) → a's second (2) → wraps to 1.
        session.band = Some(crate::ui::Band::Windows);
        session.tree_sel = 1;
        step_band(session, &snap, Direction::Next);
        assert_eq!(session.tree_sel, 2);
        step_band(session, &snap, Direction::Next);
        assert_eq!(session.tree_sel, 1, "band 2 wraps");
    }

    #[test]
    fn combined_down_enters_band_2_and_up_exits() {
        // ↓ from an app tile enters band 2 on that app's MRU-front
        // window; ↑ from a window tile returns to its app tile,
        // selection intact (the focused app never changed).
        let snap = grouped_two_apps();
        let (mut open, _expanded) = combined_session(&snap);
        let session = open.as_mut().expect("open");
        session.tree_sel = 0; // a's parent
        session.band = Some(crate::ui::Band::Apps);
        assert!(enter_band(session, &snap));
        assert_eq!(session.tree_sel, 1, "a's front child");
        assert_eq!(session.band, Some(crate::ui::Band::Windows));
        assert!(exit_band(session));
        assert_eq!(session.tree_sel, 0, "back on a's tile");
        assert_eq!(session.band, Some(crate::ui::Band::Apps));
    }

    #[test]
    fn combined_level_keys_no_op_at_their_band_edges() {
        // The four W3C no-op cases: ↓ from a window tile (deepest
        // level) and ↑ from an app tile (root) — across child and leaf
        // kinds alike, the selection and the band never move.
        let two = grouped_two_apps();
        let (mut open, _expanded) = combined_session(&two);
        let session = open.as_mut().expect("open");
        session.tree_sel = 1; // a's child, band 2
        session.band = Some(crate::ui::Band::Windows);
        assert!(!enter_band(session, &two), "↓ on a window tile");
        assert_eq!(
            (session.tree_sel, session.band),
            (1, Some(crate::ui::Band::Windows))
        );
        session.tree_sel = 0; // a's parent, band 1
        session.band = Some(crate::ui::Band::Apps);
        assert!(!exit_band(session), "↑ on a parent tile");
        assert_eq!(
            (session.tree_sel, session.band),
            (0, Some(crate::ui::Band::Apps))
        );
        // The leaf kinds: ↓ enters a leaf's single-tile band 2 — and ↑
        // from it exits; ↑ on the band-1 leaf and ↓ on the band-2 leaf
        // window are the no-ops.
        let lone = grouped_lone_parent();
        let (mut lone_open, _) = combined_session(&lone);
        let session = lone_open.as_mut().expect("open");
        session.tree_sel = 3; // c's leaf
        session.band = Some(crate::ui::Band::Apps);
        assert!(
            enter_band(session, &lone),
            "a single-window app enters its one tile"
        );
        assert_eq!(session.tree_sel, 3, "the leaf is its own band-2 tile");
        assert_eq!(session.band, Some(crate::ui::Band::Windows));
        assert!(!enter_band(session, &lone), "↓ on the leaf's window tile");
        assert!(exit_band(session), "↑ exits to the leaf's app tile");
        session.band = Some(crate::ui::Band::Apps);
        assert!(!exit_band(session), "↑ on the leaf's band-1 tile");
    }

    #[test]
    fn combined_down_enters_each_of_three_apps_children() {
        // No collapse state anywhere: ↓ from *each* app tile enters that
        // app's children in one press — the walk over three apps — and
        // the expansion set stays empty throughout (the combined card
        // never touches it).
        let snap = combined_three_apps();
        let (mut open, expanded) = combined_session(&snap);
        for (parent, first_child) in [(0usize, 1usize), (3, 4), (6, 7)] {
            let session = open.as_mut().expect("open");
            session.tree_sel = parent;
            session.band = Some(crate::ui::Band::Apps);
            assert!(enter_band(session, &snap), "enters from tree row {parent}");
            assert_eq!(session.tree_sel, first_child, "the app's MRU-front window");
            assert_eq!(session.band, Some(crate::ui::Band::Windows));
            assert!(
                !session.tree.as_ref().expect("tree").rows()[first_child]
                    .app_id
                    .is_empty()
            );
        }
        assert!(expanded.is_empty(), "no collapse state exists");
    }

    #[test]
    fn combined_steps_flow_through_session_step() {
        // The Step arm (Tab-alias arrows, wheel) routes through
        // session_step into the within-band walk, not the vertical
        // linear walk: from band 2 on a's front child, one step lands
        // on a's second window — never on b's rows.
        let snap = grouped_two_apps();
        let (mut open, mut expanded) = combined_session(&snap);
        open.as_mut().expect("open").tree_sel = 1;
        open.as_mut().expect("open").band = Some(crate::ui::Band::Windows);
        let selected = session_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            false,
            Palette::dark(),
            StyleProfile::default(),
            DEFAULT_ROWS,
            true,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Horizontal,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut expanded,
        );
        assert_eq!(selected, Some(Handle(2)), "stays inside band 2");
        assert_eq!(open.as_ref().expect("open").tree_sel, 2);
    }

    #[test]
    fn combined_pointer_rows_map_to_the_exact_band() {
        // A pointer crossing maps through the stored two-band map: the
        // row index carries the band, so hovering the child tile
        // selects the child (band 2) even though its handle is also its
        // app's parent row — and hovering an app tile selects band 1.
        let snap = grouped_two_apps();
        let (mut open, _expanded) = combined_session(&snap);
        let session = open.as_mut().expect("open");
        let whole = crate::ui::Viewport {
            start: 0,
            content: 2,
            top: false,
            bottom: false,
        };
        session.combined_map = Some(CombinedMap {
            band1: whole,
            band2: whole,
            apps: vec![0, 3],
            windows: vec![1, 2],
        });
        session.band = Some(crate::ui::Band::Apps);
        assert!(session.select_pointer_row(2, Some(Handle(1))));
        assert_eq!(session.tree_sel, 1, "the child, not its parent");
        assert_eq!(session.band, Some(crate::ui::Band::Windows));
        assert!(session.select_pointer_row(0, Some(Handle(1))));
        assert_eq!(session.tree_sel, 0, "the parent tile");
        assert_eq!(session.band, Some(crate::ui::Band::Apps));
    }

    #[test]
    fn combined_close_advances_the_reading_order() {
        // Advance-then-close walks the card's own reading order — apps
        // then the focused app's windows — skipping rows carrying the
        // doomed window (a header and its front child are one window);
        // the session stays open for repeated closes.
        let snap = grouped_two_apps();
        let (mut open, expanded) = combined_session(&snap);
        let session = open.as_mut().expect("open");
        session.tree_sel = 1; // a's front child, band 2
        session.band = Some(crate::ui::Band::Windows);
        assert!(prepare_close(session, &snap, &expanded, Handle(1)));
        assert_eq!(session.tree_sel, 3, "the next live row: b's app tile");
        assert_eq!(session.band, Some(crate::ui::Band::Apps));
        // Closing a's *second* window instead wraps onto a's own parent
        // tile — its handle is the still-live front window.
        let session = open.as_mut().expect("open");
        session.tree_sel = 2; // a's second child
        session.band = Some(crate::ui::Band::Windows);
        assert!(prepare_close(session, &snap, &expanded, Handle(2)));
        assert_eq!(session.tree_sel, 0, "wraps to a's tile");
    }

    #[test]
    fn combined_sessions_ignore_the_expansion_state() {
        // The combination has no collapse state: a pre-populated
        // expansion set (a leftover from vertical grouped gestures)
        // changes nothing — band 2 renders the focused app's whole
        // window set without any enter, and entering never inserts.
        let snap = grouped_two_apps();
        let mut expanded = HashSet::new();
        expanded.insert("app.a".to_string());
        let mut open = None;
        session_step(
            &mut open,
            &snap,
            Direction::Next,
            Scope::CurrentWorkspace,
            false,
            Palette::dark(),
            StyleProfile::default(),
            DEFAULT_ROWS,
            true,
            crate::text::TextDecision::Bitmap,
            crate::ui::LayoutSetting::Horizontal,
            crate::ui::ColumnsSetting::Auto,
            TILES,
            &mut expanded,
        );
        let session = open.as_ref().expect("open");
        let focused = session.combined_focused().expect("a level-1 focus");
        let windows = combined_windows(session, &snap, focused);
        assert_eq!(windows, vec![4, 5], "b's whole set, expansion ignored");
        assert_eq!(
            open.as_mut().expect("open").combined_focused(),
            Some(3),
            "the opening press focused b"
        );
    }
}
