//! cctk/sctk glue: compositor connection and toplevel/workspace tracking.
//!
//! Protocol objects are translated into the plain types from
//! [`crate::switcher`] at this boundary. [`Session`] is the long-lived
//! connection both flows build on: `airlock list` settles once and renders,
//! while the daemon keeps polling it for live state and sends `activate`
//! requests through the toplevel manager.

use std::collections::{HashMap, HashSet};
use std::thread;
use std::time::{Duration, Instant};

use cosmic_client_toolkit as cctk;
use cctk::cosmic_protocols::toplevel_info::v1::client::zcosmic_toplevel_handle_v1::State as CosmicState;
use cctk::cosmic_protocols::toplevel_info::v1::client::zcosmic_toplevel_handle_v1::ZcosmicToplevelHandleV1;
use cctk::sctk::compositor::{CompositorHandler, CompositorState};
use cctk::sctk::output::{OutputData, OutputHandler, OutputState};
use cctk::sctk::registry::{ProvidesRegistryState, RegistryState};
use cctk::sctk::seat::{
    keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers, RawModifiers},
    Capability, SeatHandler, SeatState,
};
use cctk::sctk::shell::WaylandSurface;
use cctk::sctk::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
    LayerSurfaceConfigure,
};
use cctk::sctk::shm::slot::SlotPool;
use cctk::sctk::shm::{Shm, ShmHandler};
use cctk::toplevel_info::{ToplevelInfoHandler, ToplevelInfoState};
use cctk::toplevel_management::{ToplevelManagerHandler, ToplevelManagerState};
use cctk::wayland_client::{Connection, DispatchError, Proxy, QueueHandle, WEnum};
use cctk::wayland_client::backend::WaylandError;
use cctk::wayland_client::globals::registry_queue_init;
use cctk::wayland_client::protocol::wl_output::WlOutput;
use cctk::wayland_client::protocol::wl_surface::WlSurface;
use cctk::wayland_client::protocol::{wl_keyboard, wl_output, wl_seat, wl_shm, wl_surface};
use cctk::wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1;
use cctk::wayland_protocols::ext::workspace::v1::client::ext_workspace_handle_v1::{ExtWorkspaceHandleV1, State as ExtWorkspaceState};
use cctk::wayland_protocols::wp::keyboard_shortcuts_inhibit::zv1::client::zwp_keyboard_shortcuts_inhibit_manager_v1::{
    self as ksi_manager, ZwpKeyboardShortcutsInhibitManagerV1,
};
use cctk::wayland_protocols::wp::keyboard_shortcuts_inhibit::zv1::client::zwp_keyboard_shortcuts_inhibitor_v1::{
    self as ksi_inhibitor, ZwpKeyboardShortcutsInhibitorV1,
};
use cctk::workspace::{WorkspaceHandler, WorkspaceState};

use crate::switcher::{Handle, Snapshot, Toplevel, Workspace, WorkspaceId};
use crate::ui::{self, GestureInput, OverlayModel};

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

/// Why connecting to (or serving over) the compositor failed.
#[derive(Debug)]
pub enum SessionError {
    Connect(cctk::wayland_client::ConnectError),
    Globals(cctk::wayland_client::globals::GlobalError),
    MissingProtocol(MissingProtocol),
    Dispatch(DispatchError),
    /// The compositor socket died underneath us (for example a compositor
    /// restart).
    Connection(WaylandError),
    NotSettled,
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionError::Connect(err) => write!(
                f,
                "cannot connect to a Wayland compositor ({err}); is WAYLAND_DISPLAY set?"
            ),
            SessionError::Globals(err) => write!(f, "compositor registry failed: {err}"),
            SessionError::MissingProtocol(missing) => write!(
                f,
                "compositor is missing a required protocol: {missing}; Airlock targets COSMIC (cosmic-comp)"
            ),
            SessionError::Dispatch(err) => {
                write!(f, "compositor connection failed: {err}")
            }
            SessionError::Connection(err) => {
                write!(f, "lost the compositor connection: {err}")
            }
            SessionError::NotSettled => {
                write!(f, "compositor did not send its initial state in time")
            }
        }
    }
}

impl std::error::Error for SessionError {}

/// Checks that the compositor offers every protocol Airlock needs.
///
/// `toplevel_info_version` is the bound `zcosmic_toplevel_info_v1` version;
/// version 3 carries the workspace membership events Airlock is built on.
pub fn check_protocols(
    has_foreign_toplevel_list: bool,
    toplevel_info_version: Option<u32>,
    has_workspace_manager: bool,
    has_toplevel_manager: bool,
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
    if !has_toplevel_manager {
        return Err(MissingProtocol {
            interface: "zcosmic_toplevel_manager_v1",
            needed_for: "activating windows",
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

/// Event-loop state for the compositor connection, shared by the one-shot
/// `airlock list` flow and the daemon's serve loop.
pub(crate) struct App {
    registry_state: RegistryState,
    output_state: OutputState,
    toplevel_info_state: ToplevelInfoState,
    toplevel_manager_state: ToplevelManagerState,
    workspace_state: WorkspaceState,
    seat_state: SeatState,
    compositor: CompositorState,
    layer_shell: LayerShell,
    shm: Shm,
    shortcuts_inhibit: ZwpKeyboardShortcutsInhibitManagerV1,
    /// First seat seen; hands activation requests. Tracked by [`SeatHandler`]
    /// because the seat is also what binds the overlay keyboard.
    seat: Option<wl_seat::WlSeat>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    /// Last shift state the seat reported, for classifying Tab presses.
    shift_held: bool,
    /// First output seen; hosts the overlay surface.
    first_output: Option<wl_output::WlOutput>,
    overlay: Option<Overlay>,
    /// Gesture inputs queued by keyboard handlers during dispatch; the
    /// daemon's serve loop drains them after each poll.
    gesture_inputs: Vec<GestureInput>,
    /// Identity maps so plain ids stay stable across snapshots for the
    /// daemon's lifetime; first-seen order assigns them.
    workspace_ids: HashMap<ExtWorkspaceHandleV1, WorkspaceId>,
    next_workspace_id: u64,
    toplevel_ids: HashMap<ExtForeignToplevelHandleV1, Handle>,
    next_toplevel_id: u64,
    info_done: bool,
    workspace_done: bool,
}

impl App {
    fn workspace_id(&mut self, handle: &ExtWorkspaceHandleV1) -> WorkspaceId {
        if let Some(id) = self.workspace_ids.get(handle) {
            return *id;
        }
        self.next_workspace_id += 1;
        let id = WorkspaceId(self.next_workspace_id);
        self.workspace_ids.insert(handle.clone(), id);
        id
    }

    fn toplevel_id(&mut self, handle: &ExtForeignToplevelHandleV1) -> Handle {
        if let Some(id) = self.toplevel_ids.get(handle) {
            return *id;
        }
        self.next_toplevel_id += 1;
        let id = Handle(self.next_toplevel_id);
        self.toplevel_ids.insert(handle.clone(), id);
        id
    }
}

impl ProvidesRegistryState for App {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    cctk::sctk::registry_handlers!(OutputState, SeatState);
}

impl OutputHandler for App {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    // The first output hosts the overlay; others are ignored.
    fn new_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        if self.first_output.is_none() {
            self.first_output = Some(output);
        }
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
        output: wl_output::WlOutput,
    ) {
        if self.first_output.as_ref() == Some(&output) {
            self.first_output = None;
        }
    }
}

impl ToplevelInfoHandler for App {
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
        toplevel: &ExtForeignToplevelHandleV1,
    ) {
        // Forget the id so a recycled object never inherits a closed
        // window's identity.
        self.toplevel_ids.remove(toplevel);
    }

    fn info_done(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>) {
        self.info_done = true;
    }
}

impl WorkspaceHandler for App {
    fn workspace_state(&mut self) -> &mut WorkspaceState {
        &mut self.workspace_state
    }

    fn done(&mut self) {
        self.workspace_done = true;
    }
}

impl ToplevelManagerHandler for App {
    fn toplevel_manager_state(&mut self) -> &mut ToplevelManagerState {
        &mut self.toplevel_manager_state
    }

    fn capabilities(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _capabilities: Vec<
            WEnum<
                cctk::cosmic_protocols::toplevel_management::v1::client::zcosmic_toplevel_manager_v1::ZcosmicToplelevelManagementCapabilitiesV1,
            >,
        >,
    ) {
    }
}

impl SeatHandler for App {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, seat: wl_seat::WlSeat) {
        if self.seat.is_none() {
            eprintln!("airlock daemon: seat arrived");
            self.seat = Some(seat);
        }
    }

    fn new_capability(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard {
            // Pre-existing seats never trigger new_seat — the initial
            // registry burst is consumed before handlers exist — so the
            // seat is captured here, from its capability event.
            if self.seat.is_none() {
                eprintln!("airlock daemon: seat arrived (with keyboard)");
                self.seat = Some(seat.clone());
            }
            if self.keyboard.is_none() {
                eprintln!("airlock daemon: seat keyboard bound");
                self.keyboard = self.seat_state.get_keyboard(qh, &seat, None).ok();
            }
        }
    }

    fn remove_capability(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Keyboard
            && let Some(keyboard) = self.keyboard.take()
        {
            keyboard.release();
        }
    }

    fn remove_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, seat: wl_seat::WlSeat) {
        if self.seat.as_ref() == Some(&seat) {
            self.seat = None;
        }
    }
}

// The overlay gesture: key presses and modifier reports are normalized into
// gesture inputs here; the daemon's serve loop drains them and applies the
// outputs. Focus loss is queued the same way so every path that can close
// the overlay flows through one state machine.
impl KeyboardHandler for App {
    fn enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _surface: &wl_surface::WlSurface,
        _serial: u32,
        _raw: &[u32],
        _keysyms: &[Keysym],
    ) {
    }

    fn leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        surface: &wl_surface::WlSurface,
        _serial: u32,
    ) {
        let losing_focus = self
            .overlay
            .as_ref()
            .is_some_and(|overlay| overlay.wl_surface() == surface);
        if losing_focus {
            self.gesture_inputs.push(GestureInput::FocusLost);
        }
    }

    fn press_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        event: KeyEvent,
    ) {
        if self.overlay.is_some() {
            let input = ui::key_input(event.keysym, self.shift_held);
            self.gesture_inputs.push(input);
        }
    }

    fn release_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        _event: KeyEvent,
    ) {
    }

    fn repeat_key(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        _event: KeyEvent,
    ) {
    }

    fn update_modifiers(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _keyboard: &wl_keyboard::WlKeyboard,
        _serial: u32,
        modifiers: Modifiers,
        _raw_modifiers: RawModifiers,
        _layout: u32,
    ) {
        self.shift_held = modifiers.shift;
        if self.overlay.is_some() {
            self.gesture_inputs
                .push(GestureInput::Modifiers { alt: modifiers.alt });
        }
    }
}

impl CompositorHandler for App {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_factor: i32,
    ) {
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
    }

    // Airlock draws on state changes, never on frame callbacks.
    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
}

impl LayerShellHandler for App {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _layer: &LayerSurface) {
        self.overlay_hide("closed by compositor");
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        if let Some(overlay) = &mut self.overlay {
            let (width, height) = configure.new_size;
            overlay.on_configure(width, height);
            let model = overlay.model();
            eprintln!(
                "airlock daemon: overlay configured {}x{} ({} rows)",
                width,
                height,
                model.rows.len()
            );
        }
    }
}

impl ShmHandler for App {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

// The shortcuts-inhibit proxies carry no state Airlock tracks; the
// inhibitor's active/inactive events are logged because they are the
// hardware-visible proof the gesture grab engaged.
impl cctk::wayland_client::Dispatch<ZwpKeyboardShortcutsInhibitManagerV1, ()> for App {
    fn event(
        _state: &mut Self,
        _proxy: &ZwpKeyboardShortcutsInhibitManagerV1,
        _event: ksi_manager::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // This interface has no events.
    }
}

impl cctk::wayland_client::Dispatch<ZwpKeyboardShortcutsInhibitorV1, ()> for App {
    fn event(
        _state: &mut Self,
        _proxy: &ZwpKeyboardShortcutsInhibitorV1,
        event: ksi_inhibitor::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            ksi_inhibitor::Event::Active => {
                eprintln!("airlock daemon: keyboard shortcuts inhibited");
            }
            ksi_inhibitor::Event::Inactive => {
                eprintln!("airlock daemon: keyboard shortcuts uninhibited");
            }
            _ => {}
        }
    }
}

impl App {
    /// Unmaps the overlay if one is up. Dropping the layer surface is the
    /// unmap: its `Drop` destroys the role object.
    pub(crate) fn overlay_hide(&mut self, reason: &str) {
        if self.overlay.take().is_some() {
            eprintln!("airlock daemon: overlay unmapped ({reason})");
        }
    }
}

cctk::sctk::delegate_compositor!(App);
cctk::sctk::delegate_keyboard!(App);
cctk::sctk::delegate_layer!(App);
cctk::sctk::delegate_output!(App);
cctk::sctk::delegate_registry!(App);
cctk::sctk::delegate_seat!(App);
cctk::sctk::delegate_shm!(App);
cctk::delegate_toplevel_info!(App);
cctk::delegate_workspace!(App);
cctk::delegate_toplevel_manager!(App);

/// Wall-clock budget for the compositor's initial state; a healthy COSMIC
/// session settles in a handful of dispatches.
pub const SETTLE_TIMEOUT: Duration = Duration::from_secs(5);

/// A long-lived compositor connection with tracked toplevel/workspace state.
///
/// All polling is non-blocking: [`Session::poll`] reads whatever is ready
/// and dispatches it, so one thread can multiplex the compositor socket
/// with the daemon's IPC socket by interleaving polls. This is the same
/// read path `blocking_dispatch` performs internally — cosmic-comp only
/// delivers its initial toplevel batch through a socket read, never
/// through a `roundtrip` spin.
pub struct Session {
    conn: Connection,
    queue: cctk::wayland_client::EventQueue<App>,
    app: App,
    /// Set whenever dispatched events may have changed tracked state; the
    /// next [`Session::snapshot`] rebuilds and clears it.
    dirty: bool,
    /// Handles that can currently be passed to [`Session::activate`],
    /// refreshed with each snapshot.
    activatable: HashMap<Handle, ZcosmicToplevelHandleV1>,
}

/// Why creating the overlay failed. The daemon logs this and carries on
/// without a switcher surface rather than dying mid-press.
#[derive(Debug)]
pub struct OverlayError(pub String);

impl std::fmt::Display for OverlayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for OverlayError {}

/// The mapped switcher overlay: one layer-shell surface backed by a shared
/// memory pool, showing one [`OverlayModel`].
///
/// Creating the surface sends an initial empty commit; the compositor
/// answers with a configure, and only then may buffers attach (attaching
/// earlier is a protocol error). While mapped, a keyboard-shortcuts
/// inhibitor keeps the compositor's own Alt+Tab from stealing the gesture;
/// dropping the `LayerSurface` unmaps the overlay and dropping the
/// inhibitor uninhibits.
///
/// The model is logical pixels; buffers are `scale ×` that, with
/// `set_buffer_scale` declared before the first commit, so HiDPI outputs
/// get native pixels instead of an upscaled blur.
pub struct Overlay {
    /// Declared before `layer` so the inhibit lifts before the unmap.
    inhibitor: ksi_inhibitor::ZwpKeyboardShortcutsInhibitorV1,
    layer: LayerSurface,
    pool: SlotPool,
    model: OverlayModel,
    /// Device pixels per logical pixel this overlay renders at.
    scale: u32,
    /// Size granted by the compositor; `None` until the first configure.
    size: Option<(u32, u32)>,
    close_at: Instant,
    hard_close_at: Instant,
}

/// Everything a new overlay borrows from the live session.
pub struct OverlayEnv<'a> {
    pub compositor: &'a CompositorState,
    pub layer_shell: &'a LayerShell,
    pub shm: &'a Shm,
    pub shortcuts: &'a ksi_manager::ZwpKeyboardShortcutsInhibitManagerV1,
    pub seat: &'a wl_seat::WlSeat,
    pub qh: &'a QueueHandle<App>,
    pub output: Option<&'a WlOutput>,
    /// Render scale for this overlay: device pixels per logical pixel
    /// (`max(2, output scale)` upstream).
    pub scale: u32,
}

impl Overlay {
    /// Creates the overlay surface for `model` (centered on `env.output`,
    /// overlay layer, exclusive keyboard interactivity) and requests its
    /// first configure.
    ///
    /// # Errors
    ///
    /// Fails when the shared-memory pool cannot be created.
    pub fn new(model: OverlayModel, env: OverlayEnv<'_>) -> Result<Self, OverlayError> {
        let OverlayEnv {
            compositor,
            layer_shell,
            shm,
            shortcuts,
            seat,
            qh,
            output,
            scale,
        } = env;
        let surface = compositor.create_surface(qh);
        let layer =
            layer_shell.create_layer_surface(qh, surface, Layer::Overlay, Some("airlock"), output);
        // No anchor edges: the surface centers in its output.
        layer.set_anchor(Anchor::empty());
        layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
        // Declared before the first commit: buffers will arrive at
        // scale × logical size.
        layer.wl_surface().set_buffer_scale(scale as i32);
        layer.set_size(model.width, model.height);
        layer.commit();

        // With shortcuts inhibited, the compositor forwards Alt (and every
        // other key) to the grabbed surface instead of acting on its own
        // bindings — the release that commits the gesture arrives as a
        // modifiers event.
        let inhibitor = shortcuts.inhibit_shortcuts(layer.wl_surface(), seat, qh, ());

        let len =
            model.width as usize * scale as usize * model.height as usize * scale as usize * 4;
        let pool = SlotPool::new(len, shm)
            .map_err(|err| OverlayError(format!("shared memory pool: {err}")))?;

        Ok(Self {
            inhibitor,
            layer,
            pool,
            model,
            scale,
            size: None,
            close_at: Instant::now() + ui::CLOSE_AFTER,
            hard_close_at: Instant::now() + ui::HARD_CLOSE,
        })
    }

    /// Swaps in a fresh model. Same-sized redraws happen immediately; a size
    /// change goes through `set_size` + commit and draws on the next
    /// configure.
    pub fn update(&mut self, model: OverlayModel) {
        let unchanged = self.size == Some((model.width, model.height));
        self.close_at = Instant::now() + ui::CLOSE_AFTER;
        let (width, height) = (model.width, model.height);
        self.model = model;
        if unchanged {
            self.draw();
        } else {
            self.layer.set_size(width, height);
            self.layer.commit();
        }
    }

    /// Applies a configure: remember the granted size and draw. A zero
    /// dimension means "pick your own", which for this surface is the model
    /// size.
    pub fn on_configure(&mut self, width: u32, height: u32) {
        let width = if width == 0 { self.model.width } else { width };
        let height = if height == 0 {
            self.model.height
        } else {
            height
        };
        self.size = Some((width, height));
        self.draw();
    }

    /// Whether the idle close deadline has passed at `now`.
    pub fn expired(&self, now: Instant) -> bool {
        now >= self.close_at
    }

    /// Whether the hard close deadline has passed at `now`.
    pub fn hard_expired(&self, now: Instant) -> bool {
        now >= self.hard_close_at
    }

    /// Pushes the idle close deadline out again; called for every gesture
    /// input so the timeout only bites when input has truly gone quiet.
    pub fn touch(&mut self) {
        self.close_at = Instant::now() + ui::CLOSE_AFTER;
    }

    /// The model currently shown.
    pub fn model(&self) -> &OverlayModel {
        &self.model
    }

    /// The overlay's surface, for matching keyboard focus events.
    pub fn wl_surface(&self) -> &WlSurface {
        self.layer.wl_surface()
    }

    /// Renders the model into a fresh buffer and commits it. A no-op before
    /// the first configure; draw failures are logged and leave the surface
    /// as-is — the next press retries with a fresh pool slot.
    fn draw(&mut self) {
        let Some((width, height)) = self.size else {
            return;
        };
        // Buffers carry device pixels: logical size × render scale.
        let (bw, bh) = (width * self.scale, height * self.scale);
        let stride = bw.saturating_mul(4);
        let Ok((buffer, canvas)) = self.pool.create_buffer(
            bw as i32,
            bh as i32,
            stride as i32,
            wl_shm::Format::Argb8888,
        ) else {
            eprintln!("airlock daemon: overlay buffer allocation failed");
            return;
        };
        ui::render(&self.model, canvas, bw, bh, stride, self.scale);
        let surface = self.layer.wl_surface();
        surface.damage_buffer(0, 0, bw as i32, bh as i32);
        if let Err(err) = buffer.attach_to(surface) {
            eprintln!("airlock daemon: overlay buffer attach failed: {err}");
            return;
        }
        self.layer.commit();
    }
}

impl Drop for Overlay {
    /// Sends the inhibitor's destroy request: dropping the proxy alone does
    /// not uninhibit, and a leaked inhibitor would keep the compositor
    /// swallowing its own keyboard shortcuts after the overlay is gone.
    /// The layer surface (and the unmap) follows in field drops.
    fn drop(&mut self) {
        self.inhibitor.destroy();
    }
}

impl Session {
    /// Connects to the compositor and binds every protocol Airlock needs.
    ///
    /// # Errors
    ///
    /// Fails with [`SessionError::MissingProtocol`] naming the first
    /// missing protocol when the compositor is not COSMIC-compatible.
    pub fn connect() -> Result<Self, SessionError> {
        let conn = Connection::connect_to_env().map_err(SessionError::Connect)?;
        let (globals, event_queue) = registry_queue_init(&conn).map_err(SessionError::Globals)?;
        let qh = event_queue.handle();

        let registry_state = RegistryState::new(&globals);
        let toplevel_info_state = match ToplevelInfoState::try_new(&registry_state, &qh) {
            Some(state) => state,
            None => {
                // try_new only fails when the foreign toplevel list global is
                // absent; every other gap is caught by check_protocols below.
                return Err(SessionError::MissingProtocol(MissingProtocol {
                    interface: "ext_foreign_toplevel_list_v1",
                    needed_for: "tracking open windows",
                }));
            }
        };
        let toplevel_manager_state = ToplevelManagerState::try_new(&registry_state, &qh);
        let workspace_state = WorkspaceState::new(&registry_state, &qh);

        let toplevel_info_version = toplevel_info_state
            .cosmic_toplevel_info
            .as_ref()
            .map(|info| info.version());
        check_protocols(
            true,
            toplevel_info_version,
            workspace_state.workspace_manager().get().is_ok(),
            toplevel_manager_state.is_some(),
        )
        .map_err(SessionError::MissingProtocol)?;

        // Overlay plumbing, bound after the COSMIC checks so a non-COSMIC
        // compositor still gets the diagnostic that names the real gap.
        let compositor = CompositorState::bind(&globals, &qh).map_err(|_| {
            SessionError::MissingProtocol(MissingProtocol {
                interface: "wl_compositor",
                needed_for: "the switcher overlay",
            })
        })?;
        let layer_shell = LayerShell::bind(&globals, &qh).map_err(|_| {
            SessionError::MissingProtocol(MissingProtocol {
                interface: "zwlr_layer_shell_v1",
                needed_for: "the switcher overlay",
            })
        })?;
        let shm = Shm::bind(&globals, &qh).map_err(|_| {
            SessionError::MissingProtocol(MissingProtocol {
                interface: "wl_shm",
                needed_for: "the switcher overlay",
            })
        })?;
        let shortcuts_inhibit = globals
            .bind::<ZwpKeyboardShortcutsInhibitManagerV1, App, ()>(&qh, 1..=1, ())
            .map_err(|_| {
                SessionError::MissingProtocol(MissingProtocol {
                    interface: "zwp_keyboard_shortcuts_inhibit_manager_v1",
                    needed_for: "receiving Alt+Tab on the switcher overlay",
                })
            })?;
        let seat_state = SeatState::new(&globals, &qh);

        let toplevel_manager_state =
            toplevel_manager_state.expect("check_protocols verified the manager global");

        let app = App {
            output_state: OutputState::new(&globals, &qh),
            toplevel_info_state,
            toplevel_manager_state,
            workspace_state,
            seat_state,
            compositor,
            layer_shell,
            shm,
            shortcuts_inhibit,
            seat: None,
            keyboard: None,
            shift_held: false,
            first_output: None,
            overlay: None,
            gesture_inputs: Vec::new(),
            registry_state,
            workspace_ids: HashMap::new(),
            next_workspace_id: 0,
            toplevel_ids: HashMap::new(),
            next_toplevel_id: 0,
            info_done: false,
            workspace_done: false,
        };

        // The queue handle is recreated for the App type the session owns;
        // the original `qh` shares the same underlying queue.
        Ok(Self {
            queue: event_queue,
            conn,
            app,
            dirty: true,
            activatable: HashMap::new(),
        })
    }

    /// Pumps the compositor socket without blocking.
    ///
    /// Returns whether anything was dispatched, so callers can distinguish
    /// activity from a quiet poll for their own pacing.
    ///
    /// # Errors
    ///
    /// Fails with [`SessionError::Connection`] when the compositor socket
    /// is gone (for example after a compositor crash).
    pub fn poll(&mut self) -> Result<bool, SessionError> {
        let Self {
            conn,
            queue,
            app,
            dirty,
            ..
        } = self;
        let mut activity = false;
        if let Some(guard) = queue.prepare_read() {
            match guard.read() {
                Ok(_) => activity = true,
                Err(WaylandError::Io(err)) if err.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(err) => return Err(SessionError::Connection(err)),
            }
        }
        let dispatched = queue
            .dispatch_pending(app)
            .map_err(SessionError::Dispatch)?;
        if dispatched > 0 {
            activity = true;
            *dirty = true;
        }
        // Handlers dispatch new requests (lazy binds, info attachments) that
        // must reach the compositor before it sends the follow-up state;
        // blocking_dispatch flushes every pass and so must this loop.
        conn.flush().map_err(SessionError::Connection)?;
        Ok(activity)
    }

    /// Waits until the compositor has delivered its initial state.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::NotSettled`] when `timeout` elapses first.
    pub fn wait_until_settled(&mut self, timeout: Duration) -> Result<(), SessionError> {
        let deadline = Instant::now() + timeout;
        while !(self.app.info_done && self.app.workspace_done) {
            if Instant::now() >= deadline {
                return Err(SessionError::NotSettled);
            }
            self.poll()?;
            thread::sleep(Duration::from_millis(1));
        }
        Ok(())
    }

    /// Whether dispatched events have changed tracked state since the last
    /// [`Session::snapshot`].
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Translates the tracked cctk state into plain types.
    ///
    /// Ids are persistent for the session's lifetime. Toplevels without a
    /// `zcosmic_toplevel_handle_v1` are dropped: they cannot be activated,
    /// so offering them to the switcher would be a dead end.
    pub fn snapshot(&mut self) -> Snapshot {
        self.dirty = false;

        // Clone the protocol state first: the identity maps mutate while
        // translating, and both borrow `app`.
        let workspaces: Vec<_> = self
            .app
            .workspace_state
            .workspaces()
            .map(|ws| {
                (
                    ws.handle.clone(),
                    ws.name.clone(),
                    ws.coordinates.clone(),
                    ws.state,
                )
            })
            .collect();
        let live_workspaces: HashSet<ExtWorkspaceHandleV1> = workspaces
            .iter()
            .map(|(handle, ..)| handle.clone())
            .collect();
        self.app
            .workspace_ids
            .retain(|handle, _| live_workspaces.contains(handle));

        let mut active = None;
        let mut plain_workspaces = Vec::with_capacity(workspaces.len());
        for (handle, name, coordinates, state) in &workspaces {
            let id = self.app.workspace_id(handle);
            if state.contains(ExtWorkspaceState::Active) {
                active = Some(id);
            }
            plain_workspaces.push(Workspace {
                id,
                name: name.clone(),
                coordinates: coordinates.clone(),
            });
        }

        let toplevels: Vec<_> = self
            .app
            .toplevel_info_state
            .toplevels()
            .map(|info| {
                (
                    info.foreign_toplevel.clone(),
                    info.title.clone(),
                    info.app_id.clone(),
                    info.state.clone(),
                    info.workspace.clone(),
                    info.cosmic_toplevel.clone(),
                )
            })
            .collect();
        let live_toplevels: HashSet<ExtForeignToplevelHandleV1> = toplevels
            .iter()
            .map(|(handle, ..)| handle.clone())
            .collect();
        self.app
            .toplevel_ids
            .retain(|handle, _| live_toplevels.contains(handle));

        self.activatable.clear();
        let mut plain_toplevels = Vec::with_capacity(toplevels.len());
        for (foreign, title, app_id, state, workspaces, cosmic) in toplevels {
            let Some(cosmic) = cosmic else { continue };
            let handle = self.app.toplevel_id(&foreign);
            let plain_workspaces = workspaces
                .iter()
                .map(|ws| self.app.workspace_id(ws))
                .collect();
            self.activatable.insert(handle, cosmic);
            plain_toplevels.push(Toplevel {
                handle,
                title,
                app_id,
                state: map_state(&state),
                workspaces: plain_workspaces,
                // MRU ticks are daemon state, stamped on top later.
                last_activated: 0,
            });
        }

        Snapshot {
            toplevels: plain_toplevels,
            workspaces: plain_workspaces,
            active,
        }
    }

    /// Asks the compositor to focus `handle`.
    ///
    /// Returns `false` when `handle` is not in the current snapshot (a
    /// window that closed between selection and activation) or no seat has
    /// arrived yet.
    ///
    /// # Errors
    ///
    /// Fails with [`SessionError::Connection`] when the request cannot be
    /// flushed.
    pub fn activate(&self, handle: Handle) -> Result<bool, SessionError> {
        let Some(toplevel) = self.activatable.get(&handle) else {
            return Ok(false);
        };
        let Some(seat) = self.app.seat.as_ref() else {
            return Ok(false);
        };
        self.app
            .toplevel_manager_state
            .manager
            .activate(toplevel, seat);
        self.conn.flush().map_err(SessionError::Connection)?;
        Ok(true)
    }

    /// The render scale for overlays hosted on the first output:
    /// `max(2, output scale)` — text is never smaller than 16 logical
    /// px, and HiDPI outputs get native pixels instead of an upscaled
    /// blur. Re-read on every overlay build, so a display-scale change
    /// applies from the next gesture session.
    pub fn render_scale(&self) -> u32 {
        let output_scale = self
            .app
            .first_output
            .as_ref()
            .and_then(|output| output.data::<OutputData>())
            .map(|data| data.scale_factor())
            .unwrap_or(1);
        u32::try_from(output_scale).unwrap_or(1).max(2)
    }

    /// Maps the switcher overlay with `model`, or refreshes it if it is
    /// already up. Creation failures are logged, not fatal: a press that
    /// cannot draw still cycles focus.
    pub fn overlay_show(&mut self, model: OverlayModel) {
        if let Some(overlay) = self.app.overlay.as_mut() {
            overlay.update(model);
            let rows = overlay.model().rows.len();
            eprintln!("airlock daemon: overlay updated ({rows} rows)");
            let _ = self.conn.flush();
            return;
        }
        // The inhibitor needs a seat; without one there is no gesture to
        // protect either, so skip mapping entirely and keep cycling blind.
        let Some(seat) = self.app.seat.clone() else {
            eprintln!("airlock daemon: overlay skipped: no seat yet");
            return;
        };
        let qh = self.queue.handle();
        let scale = self.render_scale();
        let overlay = Overlay::new(
            model,
            OverlayEnv {
                compositor: &self.app.compositor,
                layer_shell: &self.app.layer_shell,
                shm: &self.app.shm,
                shortcuts: &self.app.shortcuts_inhibit,
                seat: &seat,
                qh: &qh,
                output: self.app.first_output.as_ref(),
                scale,
            },
        );
        match overlay {
            Ok(overlay) => {
                let model = overlay.model();
                eprintln!(
                    "airlock daemon: overlay mapped {}x{} ({} rows)",
                    model.width,
                    model.height,
                    model.rows.len()
                );
                self.app.overlay = Some(overlay);
                let _ = self.conn.flush();
            }
            Err(err) => eprintln!("airlock daemon: overlay failed: {err}"),
        }
    }

    /// Unmaps the overlay if one is up.
    pub fn overlay_hide(&mut self, reason: &str) {
        self.app.overlay_hide(reason);
    }

    /// Idle and hard close deadlines of a mapped overlay, as
    /// `(idle_expired, hard_expired)`; `None` when no overlay is up. The
    /// daemon owns the closing policy because the Alt state it tracks
    /// decides whether an idle expiry should bite.
    pub fn overlay_deadlines(&self) -> Option<(bool, bool)> {
        let now = Instant::now();
        self.app
            .overlay
            .as_ref()
            .map(|overlay| (overlay.expired(now), overlay.hard_expired(now)))
    }

    /// Pushes the overlay's idle deadline out again.
    pub fn overlay_touch(&mut self) {
        if let Some(overlay) = self.app.overlay.as_mut() {
            overlay.touch();
        }
    }

    /// Drains gesture inputs queued by keyboard handlers during dispatch.
    pub fn take_gesture_inputs(&mut self) -> Vec<GestureInput> {
        std::mem::take(&mut self.app.gesture_inputs)
    }
}

/// Connects to the compositor, waits for initial state, and returns the
/// rendered snapshot for `airlock list`.
pub fn list() -> Result<String, SessionError> {
    let mut session = Session::connect()?;
    session.wait_until_settled(SETTLE_TIMEOUT)?;
    Ok(format_snapshot(&session.snapshot()))
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn check_passes_when_all_protocols_present() {
        assert_eq!(check_protocols(true, Some(3), true, true), Ok(()));
    }

    #[test]
    fn missing_foreign_toplevel_list_is_named() {
        let err = check_protocols(false, Some(3), true, true).unwrap_err();
        assert_eq!(err.interface, "ext_foreign_toplevel_list_v1");
    }

    #[test]
    fn missing_toplevel_info_is_named() {
        let err = check_protocols(true, None, true, true).unwrap_err();
        assert_eq!(err.interface, "zcosmic_toplevel_info_v1");
    }

    #[test]
    fn toplevel_info_below_v3_is_named() {
        let err = check_protocols(true, Some(2), true, true).unwrap_err();
        assert!(err.interface.contains("zcosmic_toplevel_info_v1"));
        assert!(err.interface.contains("3"));
    }

    #[test]
    fn missing_workspace_manager_is_named() {
        let err = check_protocols(true, Some(3), false, true).unwrap_err();
        assert_eq!(err.interface, "ext_workspace_manager_v1");
    }

    #[test]
    fn missing_toplevel_manager_is_named() {
        let err = check_protocols(true, Some(3), true, false).unwrap_err();
        assert_eq!(err.interface, "zcosmic_toplevel_manager_v1");
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
