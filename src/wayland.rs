//! cctk/sctk glue: compositor connection and toplevel/workspace tracking.
//!
//! Protocol objects are translated into the plain types from
//! [`crate::switcher`] at this boundary. The one-shot `list` flow connects,
//! waits for the compositor's initial state, and renders it; live tracking
//! for the daemon builds on the same wiring.

use std::collections::{HashMap, HashSet};

use cosmic_client_toolkit as cctk;
use cctk::cosmic_protocols::toplevel_info::v1::client::zcosmic_toplevel_handle_v1::State as CosmicState;
use cctk::sctk::{
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
};
use cctk::toplevel_info::{ToplevelInfoHandler, ToplevelInfoState};
use cctk::wayland_client::{Connection, DispatchError, Proxy, QueueHandle};
use cctk::wayland_client::globals::registry_queue_init;
use cctk::wayland_client::protocol::wl_output;
use cctk::wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1;
use cctk::wayland_protocols::ext::workspace::v1::client::ext_workspace_handle_v1::{ExtWorkspaceHandleV1, State as ExtWorkspaceState};
use cctk::workspace::{WorkspaceHandler, WorkspaceState};

use crate::switcher::{Handle, Snapshot, Toplevel, Workspace, WorkspaceId};

/// A required protocol the compositor does not offer at a sufficient version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MissingProtocol {
    pub interface: &'static str,
    pub needed_for: &'static str,
}

impl std::fmt::Display for MissingProtocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "missing {}, needed for {}",
            self.interface, self.needed_for
        )
    }
}

impl std::error::Error for MissingProtocol {}

/// Why `list` could not produce a snapshot.
#[derive(Debug)]
pub enum ListError {
    Connect(cctk::wayland_client::ConnectError),
    Globals(cctk::wayland_client::globals::GlobalError),
    MissingProtocol(MissingProtocol),
    Dispatch(DispatchError),
    NotSettled,
}

impl std::fmt::Display for ListError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ListError::Connect(err) => write!(
                f,
                "cannot connect to a Wayland compositor ({err}); is WAYLAND_DISPLAY set?"
            ),
            ListError::Globals(err) => write!(f, "compositor registry failed: {err}"),
            ListError::MissingProtocol(missing) => write!(
                f,
                "compositor is missing a required protocol: {missing}; Airlock targets COSMIC (cosmic-comp)"
            ),
            ListError::Dispatch(err) => {
                write!(f, "compositor connection failed: {err}")
            }
            ListError::NotSettled => {
                write!(f, "compositor did not send its initial state in time")
            }
        }
    }
}

impl std::error::Error for ListError {}

/// Checks that the compositor offers every protocol Airlock needs.
///
/// `toplevel_info_version` is the bound `zcosmic_toplevel_info_v1` version;
/// version 3 carries the workspace membership events Airlock is built on.
pub fn check_protocols(
    has_foreign_toplevel_list: bool,
    toplevel_info_version: Option<u32>,
    has_workspace_manager: bool,
) -> Result<(), MissingProtocol> {
    if !has_foreign_toplevel_list {
        return Err(MissingProtocol {
            interface: "ext_foreign_toplevel_list_v1",
            needed_for: "tracking open windows",
        });
    }
    let version = toplevel_info_version.ok_or(MissingProtocol {
        interface: "zcosmic_toplevel_info_v1",
        needed_for: "window titles, states, and workspace membership",
    })?;
    if version < 3 {
        return Err(MissingProtocol {
            interface: "zcosmic_toplevel_info_v1 version 3",
            needed_for: "window-to-workspace membership",
        });
    }
    if !has_workspace_manager {
        return Err(MissingProtocol {
            interface: "ext_workspace_manager_v1",
            needed_for: "tracking workspaces",
        });
    }
    Ok(())
}

/// Maps `zcosmic_toplevel_handle_v1` state flags onto the plain [`crate::switcher::State`].
pub fn map_state(states: &HashSet<CosmicState>) -> crate::switcher::State {
    let mut mapped = crate::switcher::State::default();
    for &state in states {
        mapped = mapped.union(match state {
            CosmicState::Maximized => crate::switcher::State::MAXIMIZED,
            CosmicState::Minimized => crate::switcher::State::MINIMIZED,
            CosmicState::Activated => crate::switcher::State::ACTIVATED,
            CosmicState::Fullscreen => crate::switcher::State::FULLSCREEN,
            CosmicState::Sticky => crate::switcher::State::STICKY,
            // Combinations or unknown bits are not meaningful to Airlock.
            _ => continue,
        });
    }
    mapped
}

/// Renders a snapshot as the stable, human-readable `airlock list` output.
/// Workspaces are listed sorted by coordinates, toplevels by handle.
pub fn format_snapshot(snapshot: &Snapshot) -> String {
    let mut workspaces = snapshot.workspaces.clone();
    workspaces.sort_by(|a, b| {
        a.coordinates
            .cmp(&b.coordinates)
            .then_with(|| a.id.cmp(&b.id))
    });
    let mut toplevels = snapshot.toplevels.clone();
    toplevels.sort_by_key(|toplevel| toplevel.handle);

    let mut out = String::new();
    match snapshot.active {
        Some(id) => {
            let name = workspaces
                .iter()
                .find(|workspace| workspace.id == id)
                .map(|workspace| workspace.name.as_str())
                .unwrap_or("?");
            out.push_str(&format!("active workspace: {} ({name})\n", id.0));
        }
        None => out.push_str("active workspace: none\n"),
    }
    for workspace in &workspaces {
        out.push_str(&format!(
            "workspace {}: {}\n",
            workspace.id.0, workspace.name
        ));
    }
    for toplevel in &toplevels {
        let mut ids: Vec<u64> = toplevel.workspaces.iter().map(|id| id.0).collect();
        ids.sort_unstable();
        let ids = ids
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!(
            "toplevel {}: \"{}\" app_id \"{}\" state ({}) workspaces ({})\n",
            toplevel.handle.0,
            toplevel.title,
            toplevel.app_id,
            state_names(toplevel.state).join(", "),
            ids,
        ));
    }
    out
}

/// Flag names in wire order for stable `list` output.
fn state_names(state: crate::switcher::State) -> Vec<&'static str> {
    let mut names = Vec::new();
    for (flag, name) in [
        (crate::switcher::State::MAXIMIZED, "maximized"),
        (crate::switcher::State::MINIMIZED, "minimized"),
        (crate::switcher::State::ACTIVATED, "activated"),
        (crate::switcher::State::FULLSCREEN, "fullscreen"),
        (crate::switcher::State::STICKY, "sticky"),
    ] {
        if state.contains(flag) {
            names.push(name);
        }
    }
    names
}

/// Event-loop state for the one-shot `airlock list` connection.
#[derive(Debug)]
struct ListApp {
    registry_state: RegistryState,
    output_state: OutputState,
    toplevel_info_state: ToplevelInfoState,
    workspace_state: WorkspaceState,
    info_done: bool,
    workspace_done: bool,
}

impl ProvidesRegistryState for ListApp {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    cctk::sctk::registry_handlers!(OutputState);
}

impl OutputHandler for ListApp {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    // Outputs are bound so workspace groups and toplevels can reference them;
    // `list` does not otherwise care about output events.
    fn new_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }
}

impl ToplevelInfoHandler for ListApp {
    fn toplevel_info_state(&mut self) -> &mut ToplevelInfoState {
        &mut self.toplevel_info_state
    }

    fn new_toplevel(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _toplevel: &ExtForeignToplevelHandleV1,
    ) {
    }

    fn update_toplevel(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _toplevel: &ExtForeignToplevelHandleV1,
    ) {
    }

    fn toplevel_closed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _toplevel: &ExtForeignToplevelHandleV1,
    ) {
    }

    fn info_done(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>) {
        self.info_done = true;
    }
}

impl WorkspaceHandler for ListApp {
    fn workspace_state(&mut self) -> &mut WorkspaceState {
        &mut self.workspace_state
    }

    fn done(&mut self) {
        self.workspace_done = true;
    }
}

cctk::sctk::delegate_output!(ListApp);
cctk::sctk::delegate_registry!(ListApp);
cctk::delegate_toplevel_info!(ListApp);
cctk::delegate_workspace!(ListApp);

/// Wall-clock budget for the compositor's initial state; a healthy COSMIC
/// session settles in a handful of dispatches.
const SETTLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Connects to the compositor, waits for initial state, and returns the
/// rendered snapshot for `airlock list`.
pub fn list() -> Result<String, ListError> {
    let conn = Connection::connect_to_env().map_err(ListError::Connect)?;
    let (globals, mut event_queue) = registry_queue_init(&conn).map_err(ListError::Globals)?;
    let qh = event_queue.handle();

    let registry_state = RegistryState::new(&globals);
    let toplevel_info_state = match ToplevelInfoState::try_new(&registry_state, &qh) {
        Some(state) => state,
        None => {
            // try_new only fails when the foreign toplevel list global is
            // absent; every other gap is caught by check_protocols below.
            return Err(ListError::MissingProtocol(MissingProtocol {
                interface: "ext_foreign_toplevel_list_v1",
                needed_for: "tracking open windows",
            }));
        }
    };
    let mut app = ListApp {
        output_state: OutputState::new(&globals, &qh),
        toplevel_info_state,
        workspace_state: WorkspaceState::new(&registry_state, &qh),
        registry_state,
        info_done: false,
        workspace_done: false,
    };

    let toplevel_info_version = app
        .toplevel_info_state
        .cosmic_toplevel_info
        .as_ref()
        .map(|info| info.version());
    check_protocols(
        true,
        toplevel_info_version,
        app.workspace_state.workspace_manager().get().is_ok(),
    )
    .map_err(ListError::MissingProtocol)?;

    // cosmic-comp pushes its initial toplevel batch asynchronously: a
    // blocking dispatch loop (the mode cctk's examples use) is what delivers
    // it, while a roundtrip spin can miss it entirely. A watchdog bounds the
    // wait so a broken compositor cannot hang the one-shot.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        while !(app.info_done && app.workspace_done) {
            if let Err(err) = event_queue.blocking_dispatch(&mut app) {
                let _ = tx.send(Err(err));
                return;
            }
        }
        let _ = tx.send(Ok(format_snapshot(&snapshot(&app))));
    });
    match rx.recv_timeout(SETTLE_TIMEOUT) {
        Ok(Ok(text)) => Ok(text),
        Ok(Err(err)) => Err(ListError::Dispatch(err)),
        // Timeout or a panicked dispatch thread: either way the compositor
        // never delivered a complete initial state.
        Err(_) => Err(ListError::NotSettled),
    }
}

/// Translates the cctk state into plain types, assigning stable ids to
/// workspace handles (and opaque handles to toplevels) in first-seen order.
fn snapshot(app: &ListApp) -> Snapshot {
    let mut workspace_ids: HashMap<ExtWorkspaceHandleV1, WorkspaceId> = HashMap::new();
    let mut next_workspace_id = 0u64;
    let mut workspace = |handle: &ExtWorkspaceHandleV1| -> WorkspaceId {
        next_workspace_id += 1;
        *workspace_ids
            .entry(handle.clone())
            .or_insert(WorkspaceId(next_workspace_id))
    };

    let mut workspaces = Vec::new();
    let mut active = None;
    for ws in app.workspace_state.workspaces() {
        let id = workspace(&ws.handle);
        if ws.state.contains(ExtWorkspaceState::Active) {
            active = Some(id);
        }
        workspaces.push(Workspace {
            id,
            name: ws.name.clone(),
            coordinates: ws.coordinates.clone(),
        });
    }

    let mut next_handle = 0u64;
    let toplevels = app
        .toplevel_info_state
        .toplevels()
        .map(|info| {
            next_handle += 1;
            let workspaces = info.workspace.iter().map(&mut workspace).collect();
            Toplevel {
                handle: Handle(next_handle),
                title: info.title.clone(),
                app_id: info.app_id.clone(),
                state: map_state(&info.state),
                workspaces,
                // One-shot snapshot: activation history is daemon state (T4).
                last_activated: 0,
            }
        })
        .collect();

    Snapshot {
        toplevels,
        workspaces,
        active,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn check_passes_when_all_protocols_present() {
        assert_eq!(check_protocols(true, Some(3), true), Ok(()));
    }

    #[test]
    fn missing_foreign_toplevel_list_is_named() {
        let err = check_protocols(false, Some(3), true).unwrap_err();
        assert_eq!(err.interface, "ext_foreign_toplevel_list_v1");
    }

    #[test]
    fn missing_toplevel_info_is_named() {
        let err = check_protocols(true, None, true).unwrap_err();
        assert_eq!(err.interface, "zcosmic_toplevel_info_v1");
    }

    #[test]
    fn toplevel_info_below_v3_is_named() {
        let err = check_protocols(true, Some(2), true).unwrap_err();
        assert!(err.interface.contains("zcosmic_toplevel_info_v1"));
        assert!(err.interface.contains("3"));
    }

    #[test]
    fn missing_workspace_manager_is_named() {
        let err = check_protocols(true, Some(3), false).unwrap_err();
        assert_eq!(err.interface, "ext_workspace_manager_v1");
    }

    #[test]
    fn map_state_translates_each_flag() {
        let states = HashSet::from([
            CosmicState::Maximized,
            CosmicState::Minimized,
            CosmicState::Activated,
            CosmicState::Fullscreen,
            CosmicState::Sticky,
        ]);
        let mapped = map_state(&states);
        assert!(mapped.contains(crate::switcher::State::MAXIMIZED));
        assert!(mapped.contains(crate::switcher::State::MINIMIZED));
        assert!(mapped.contains(crate::switcher::State::ACTIVATED));
        assert!(mapped.contains(crate::switcher::State::FULLSCREEN));
        assert!(mapped.contains(crate::switcher::State::STICKY));
    }

    #[test]
    fn map_state_empty_input_maps_to_no_flags() {
        let mapped = map_state(&HashSet::new());
        assert!(!mapped.contains(crate::switcher::State::MINIMIZED));
        assert!(!mapped.contains(crate::switcher::State::ACTIVATED));
    }

    fn sample_snapshot() -> Snapshot {
        Snapshot {
            toplevels: vec![
                Toplevel {
                    handle: Handle(2),
                    title: "Second".into(),
                    app_id: "org.example.b".into(),
                    state: crate::switcher::State::ACTIVATED,
                    workspaces: HashSet::from([WorkspaceId(2)]),
                    last_activated: 0,
                },
                Toplevel {
                    handle: Handle(1),
                    title: "First".into(),
                    app_id: "org.example.a".into(),
                    state: crate::switcher::State::MINIMIZED.union(crate::switcher::State::STICKY),
                    workspaces: HashSet::from([WorkspaceId(1), WorkspaceId(2)]),
                    last_activated: 0,
                },
            ],
            workspaces: vec![
                Workspace {
                    id: WorkspaceId(2),
                    name: "Work".into(),
                    coordinates: vec![1, 0],
                },
                Workspace {
                    id: WorkspaceId(1),
                    name: "Home".into(),
                    coordinates: vec![0, 0],
                },
            ],
            active: Some(WorkspaceId(2)),
        }
    }

    #[test]
    fn format_renders_active_workspace_toplevels_and_flags() {
        let text = format_snapshot(&sample_snapshot());
        assert_eq!(
            text,
            concat!(
                "active workspace: 2 (Work)\n",
                "workspace 1: Home\n",
                "workspace 2: Work\n",
                "toplevel 1: \"First\" app_id \"org.example.a\" state (minimized, sticky) workspaces (1, 2)\n",
                "toplevel 2: \"Second\" app_id \"org.example.b\" state (activated) workspaces (2)\n",
            )
        );
    }

    #[test]
    fn format_reports_no_active_workspace() {
        let text = format_snapshot(&Snapshot::default());
        assert_eq!(text, "active workspace: none\n");
    }
}
