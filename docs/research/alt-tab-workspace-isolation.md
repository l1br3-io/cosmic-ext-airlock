# Alt+Tab workspace isolation on Pop!_OS COSMIC — research findings

**Date:** October 22, 2026 · **Scope:** Pop!_OS COSMIC DE (cosmic-comp `epoch-1.11.0`, tagged 2026-10-14, latest as of research date) · **Method:** primary sources only — cosmic-comp/cosmic-epoch/cosmic-launcher/cosmic-applets/cosmic-settings source code on GitHub, cosmic-protocols XML specs, GitHub issue trackers, official GNOME/KDE/Microsoft/Apple/System76 documentation.

---

## 1. TL;DR

**Not built in today — but fully implementable by a third-party tool, and System76 has publicly committed to a future config option.** COSMIC's Alt+Tab is *not* implemented inside the compositor: `Alt+Tab` is bound to `System(WindowSwitcher)`, which cosmic-comp resolves to an external command (by default `cosmic-launcher alt-tab`) and spawns. The window list comes from the `cosmic_toplevel` plugin in pop-os/launcher, which keeps every toplevel from every workspace in MRU order with **no workspace filter anywhere in the pipeline** (verified in source). There is **no config key or settings toggle** for workspace scoping (verified against the full `CosmicCompConfig` schema). The canonical tracking issue [pop-os/cosmic-epoch#1777](https://github.com/pop-os/cosmic-epoch/issues/1777) is **open** with an `enhancement` label; System76's leviport stated (Dec 2025) that "making this behavior configurable in the future is desired" and (May 2026) that "there is clearly demand for it." Crucially for an external tool ("Airlock"): cosmic-comp **does** implement everything a client needs — per-window workspace membership (`zcosmic_toplevel_info_v1` v3 `ext_workspace_enter/leave`), current-workspace tracking and switching (`ext-workspace-v1` + `zcosmic_workspace_v2`), and focus control (`zcosmic_toplevel_manager_v1.activate`) — and the Alt+Tab hook itself is spawn-redirectable via a user config file. Community switchers (`cosmic-switch-less`, `cosmic-ext-peek-switcher`) already prove the approach works on stock COSMIC. **Recommendation: external app, no cosmic-comp patch required.**

---

## 2. Built-in support (as of cosmic-comp `epoch-1.11.0`, Oct 2026)

### 2.1 How Alt+Tab actually works in COSMIC

The switcher is a *spawned external program*, not compositor UI:

1. Default keybinding file maps Alt+Tab / Shift+Alt+Tab (and Super+Tab) to a *system action*:
   - [`data/keybindings.ron` lines 109–112](https://github.com/pop-os/cosmic-comp/blob/epoch-1.11.0/data/keybindings.ron#L109-L112) (identical on [`master`](https://github.com/pop-os/cosmic-comp/blob/master/data/keybindings.ron)):
     ```ron
     (modifiers: [Alt], key: "Tab"): System(WindowSwitcher),
     (modifiers: [Alt, Shift], key: "Tab"): System(WindowSwitcherPrevious),
     ```
   - `System::WindowSwitcher` is defined as *"Opens the (alt+tab) window switcher"* in [`cosmic-settings-config`](https://github.com/pop-os/cosmic-settings-daemon/blob/master/config/src/shortcuts/action.rs#L206-L209) (`config/src/shortcuts/action.rs` lines 206–209).
2. cosmic-comp executes system actions by **spawning a configured command**: [`src/input/actions.rs` lines 1044–1048](https://github.com/pop-os/cosmic-comp/blob/epoch-1.11.0/src/input/actions.rs#L1044-L1048):
   ```rust
   Action::System(system) => {
       if let Some(command) = self.common.config.system_actions.get(&system) {
           self.spawn_command(command.clone());
       }
   }
   ```
   The `system_actions` map is loaded from the `com.system76.CosmicSettings.Shortcuts` cosmic-config entry ([`src/config/mod.rs` lines 75–76](https://github.com/pop-os/cosmic-comp/blob/master/src/config/mod.rs#L75-L76)); user overrides live in `~/.config/cosmic/com.system76.CosmicSettings.Shortcuts/v1/system_actions` and take precedence over the system defaults in `/usr/share/cosmic/...` ([`config/src/shortcuts/mod.rs` `system_actions()`, lines 51–69](https://github.com/pop-os/cosmic-settings-daemon/blob/master/config/src/shortcuts/mod.rs#L51-L69)). The shipped default maps `WindowSwitcher` → `"cosmic-launcher alt-tab"` (documented in the [cosmic-altswitcher README](https://github.com/jibsta210/cosmic-altswitcher#install); the `AltTab`/`ShiftAltTab` CLI subcommands — *"Toggle the launcher and switch to the alt-tab view"* — are verified in [cosmic-launcher `src/app.rs`](https://github.com/pop-os/cosmic-launcher/blob/master/src/app.rs#L72-L81)).
3. cosmic-launcher's alt-tab view gets its window list from the **pop-launcher service** (`pop-os/launcher` repo, formerly `pop-launcher`), specifically the [`cosmic_toplevel` plugin](https://github.com/pop-os/launcher/blob/master/plugins/src/cosmic_toplevel/mod.rs). That plugin:
   - subscribes via `zcosmic_toplevel_info_v1` (built on `ext-foreign-toplevel-list-v1`) and keeps **all** toplevels in a Vec, moving the activated one to the end (MRU) — [`mod.rs` lines 73–103](https://github.com/pop-os/launcher/blob/master/plugins/src/cosmic_toplevel/mod.rs#L73-L103);
   - filters `search()` results **only** by title/app_id/keywords — there is no workspace reference anywhere in the plugin ([`mod.rs` lines 177–229](https://github.com/pop-os/launcher/blob/master/plugins/src/cosmic_toplevel/mod.rs#L177-L229));
   - activates a chosen window via `zcosmic_toplevel_manager_v1.activate(toplevel, seat)` ([`toplevel_handler.rs` lines 171–192](https://github.com/pop-os/launcher/blob/master/plugins/src/cosmic_toplevel/toplevel_handler.rs#L171-L192)).
   - The launcher sorts the returned window items to the front of the list, reversed (MRU-first) ([cosmic-launcher `src/app.rs` `pop_launcher::Response::Update` handler](https://github.com/pop-os/cosmic-launcher/blob/master/src/app.rs#L598-L615)).

**Conclusion:** windows from **all workspaces** appear in Alt+Tab, in MRU order. This matches the user-visible behavior reported in the issue tracker (§3).

### 2.2 No config option exists

- The complete compositor config schema is `CosmicCompConfig` in [`cosmic-comp-config/src/lib.rs`](https://github.com/pop-os/cosmic-comp/blob/master/cosmic-comp-config/src/lib.rs) (workspace member of the cosmic-comp repo). Enumerated in full: `workspaces`, `pinned_workspaces`, `input_*`, `xkb_config`, `keyboard_config`, `autotile`, `autotile_behavior`, `active_hint`, `focus_follows_cursor`, `cursor_follows_focus`, `focus_follows_cursor_delay`, `descale_xwayland`, `xwayland_eavesdropping`, `edge_snap_threshold`, `accessibility_zoom`, `appearance_settings`, `cursor_hide`, `cursor_shake_to_find`, `activation_policy`, `decoration_preference`. **Nothing relates to the window switcher or workspace scoping of any switcher.**
- The shortcut vocabulary itself (`Action`/`System` enums in [cosmic-settings-config `action.rs`](https://github.com/pop-os/cosmic-settings-daemon/blob/master/config/src/shortcuts/action.rs)) has `WindowSwitcher` / `WindowSwitcherPrevious` but **no workspace-scoped variant**.
- The closest built-in primitives: `Workspace(n)`, `NextWorkspace`/`PreviousWorkspace` bindings and `Focus(direction)` bindings (spatial, current-workspace-first; falls back to workspace switching at edges — [`src/input/actions.rs` lines 746–812](https://github.com/pop-os/cosmic-comp/blob/master/src/input/actions.rs#L746-L812)). None of these is a workspace-scoped window *cycler*.

**Verdict:** *No native workspace-isolated Alt+Tab as of cosmic-comp `epoch-1.11.0` (tagged 2026-10-14) / master (2026-10-22).*

---

## 3. Known issues & discussions

The master tracking issue is **pop-os/cosmic-epoch#1777**; System76 staff have consolidated several duplicates into it.

| Issue / PR | Title (abridged) | Opened | Status | Maintainer response |
|---|---|---|---|---|
| [pop-os/cosmic-epoch#1777](https://github.com/pop-os/cosmic-epoch/issues/1777) | "can we toggle alt-tab window switcher to only show windows on the current desktop" | 2025-04-06 (ThomasCorbin) | **Open**, label `enhancement` | leviport (System76): rationale for showing all windows ("many users notice that running applications are 'missing' from the picker"), then: *"making this behavior configurable in the future is desired. That's why the feature request remains open… the design direction we took initially, with plans to expand later"* (2025-12-23); called the Alt+Tab-scoped / Super+Tab-global hybrid *"definitely interesting"*; *"Don't worry, the feature request remains open. There is clearly demand for it."* (2026-05-01) |
| [pop-os/cosmic-launcher#450](https://github.com/pop-os/cosmic-launcher/issues/450) | "Option to show only current workspace windows in App Switcher (Alt+Tab)" | 2026-07-04 (HallanCosta, on cosmic-epoch) | **Open** (transferred to cosmic-launcher 2026-07-20 by leviport) | Contributor KodeBarista: "This issue needs to be transferred to COSMIC Launcher" |
| [pop-os/cosmic-launcher#355](https://github.com/pop-os/cosmic-launcher/issues/355) | "Configurable 'per-workspace' Alt+Tab" | 2025-11-04 (ervinpopescu) | Closed as **duplicate** of #1777 (2026-01-05) | leviport: "tracking in pop-os/cosmic-epoch#1777" |
| [pop-os/cosmic-epoch#2583](https://github.com/pop-os/cosmic-epoch/issues/2583) | "A new shortcut for switching between open windows of the current workspace only" | 2025 | Closed as duplicate of #1777 (2025-12-08, leviport) | — |
| [pop-os/cosmic-epoch#2729](https://github.com/pop-os/cosmic-epoch/issues/2729) | "FR: Switch between windows of current workspace" | 2025 | Closed as duplicate of #1777 (2025-12-18, leviport) | — |
| [pop-os/cosmic-epoch#3532](https://github.com/pop-os/cosmic-epoch/issues/3532) | "ALT+TAB option to show only open windows from the current workspace" | 2026 | Closed as duplicate of #1777 (2026-05-11, leviport) | — |
| [pop-os/cosmic-launcher#173](https://github.com/pop-os/cosmic-launcher/issues/173) | "Support Alt/Super + Shift + Tab in Window Switcher" | 2024-08-18 | **Closed as completed** 2025-01-14 | Implemented via [cosmic-launcher#183](https://github.com/pop-os/cosmic-launcher/pull/183) (`--reverse`), [cosmic-settings-daemon#49](https://github.com/pop-os/cosmic-settings-daemon/pull/49) (`WindowSwitcherReverse` action), [cosmic-comp#807](https://github.com/pop-os/cosmic-comp/pull/807) (keybinding), [cosmic-settings#675](https://github.com/pop-os/cosmic-settings/pull/675) |
| [pop-os/cosmic-launcher#135](https://github.com/pop-os/cosmic-launcher/issues/135) | "Alt/super+tab recency-based cycling" (MRU) | 2024 | Closed (alpha 3 milestone, 2024-12-16) | — |
| [pop-os/cosmic-applets#1236](https://github.com/pop-os/cosmic-applets/pull/1236) | "feat(app-list): implement workspace and configured-output filtering" | 2026-01-01 (TobiasSchaffner) | **Merged** 2026-04-09 (jackpot51; approved by leviport) | Adds `filter_top_levels` (`None`/`ActiveWorkspace`/`ConfiguredOutput`) to the **dock app-list applet only**. Author (2026-07-19): "I think the additional features you are missing [Alt+Tab] had to be implemented in cosmic-comp" |
| [pop-os/cosmic-applets#339](https://github.com/pop-os/cosmic-applets/issues/339) | workspace-aware app list (dock) | pre-2026 | Referenced from #1777 thread; addressed by #1236 above | — |

Related but distinct (switcher quality, not scoping): [cosmic-launcher#231](https://github.com/pop-os/cosmic-launcher/issues/231) "Alt-Tab has a delay and gets stuck if pressed too quickly" (closed 2026-06-04), [#130](https://github.com/pop-os/cosmic-launcher/issues/130) "Shortcut: Alt/Super+Tab behavior" (closed 2024-05-24, alpha 1). Source: [cosmic-launcher issue list, query "workspace switcher"](https://github.com/pop-os/cosmic-launcher/issues?q=is%3Aissue%20workspace%20switcher).

**Net:** well-known, wanted, tracked, not scheduled. No open PR implements workspace filtering for the launcher's alt-tab view as of 2026-10-22.

---

## 4. Protocol/API feasibility for a third-party tool ("Airlock")

### 4.1 What cosmic-comp implements (verified in source)

cosmic-comp depends on `cosmic-protocols` (git, `server` feature) — [`Cargo.toml`](https://github.com/pop-os/cosmic-comp/blob/master/Cargo.toml) — and registers the protocol state objects in [`src/state.rs` (lines ~27–29, 315–319)](https://github.com/pop-os/cosmic-comp/blob/master/src/state.rs):

| Capability | Protocol / interface | cosmic-comp implementation | Notes for Airlock |
|---|---|---|---|
| Enumerate windows + title/app_id/state/geometry | `zcosmic_toplevel_info_v1` v3 (extends `ext-foreign-toplevel-list-v1`) | [`src/wayland/handlers/toplevel_info.rs`](https://github.com/pop-os/cosmic-comp/blob/master/src/wayland/handlers/toplevel_info.rs), state machine in [`src/wayland/protocols/toplevel_info.rs`](https://github.com/pop-os/cosmic-comp/blob/master/src/wayland/protocols/toplevel_info.rs) | Events: `title`, `app_id`, `state`, `geometry`, `output_enter/leave` ([XML](https://github.com/pop-os/cosmic-protocols/blob/main/unstable/cosmic-toplevel-info-unstable-v1.xml)) |
| **Window → workspace membership** | `zcosmic_toplevel_handle_v1.ext_workspace_enter/leave` (since v3) | Emitted in [`protocols/toplevel_info.rs` lines 634–659](https://github.com/pop-os/cosmic-comp/blob/master/src/wayland/protocols/toplevel_info.rs#L634-L659) (`workspaces_changed` block) | **Queryable.** XML: *"This event is emitted whenever the toplevel becomes visible on the given workspace. A toplevel may be visible on multiple workspaces."* (sticky case) |
| Which workspace is current | `ext-workspace-v1` (`ext_workspace_handle_v1.state` includes `active`) + `zcosmic_workspace_v2` | [`src/wayland/protocols/workspace/`](https://github.com/pop-os/cosmic-comp/blob/master/src/wayland/protocols/workspace/mod.rs): `State::{Active, Urgent, Hidden, Pinned}` bitflags (lines 46–55) | `ext_workspace_handle_v1` also has an `Activate` request, implemented in [`workspace/ext.rs` lines 193–212](https://github.com/pop-os/cosmic-comp/blob/master/src/wayland/protocols/workspace/ext.rs#L193-L212) — a client may switch workspaces too |
| **Focus a window from a client** | `zcosmic_toplevel_manager_v1` `activate(toplevel, seat)` | [`src/wayland/protocols/toplevel_management.rs` line 188–190](https://github.com/pop-os/cosmic-comp/blob/master/src/wayland/protocols/toplevel_management.rs#L188-L190) (`Request::Activate { toplevel, seat } => state.activate(...)`) | Also: `close`, `set/unset_maximized|minimized|fullscreen`, `set_sticky`, `move_to_workspace` ([XML](https://github.com/pop-os/cosmic-protocols/blob/main/unstable/cosmic-toplevel-management-unstable-v1.xml)) |
| Overlay UI (a switcher popup) | `zwlr_layer_shell_v1` | Handler at [`src/wayland/handlers/layer_shell.rs`](https://github.com/pop-os/cosmic-comp/blob/master/src/wayland/handlers/layer_shell.rs) | cosmic-launcher itself is a layer-surface app; libcosmic/iced supports this |
| Grab all keys while switching | `zwp_keyboard_shortcuts_inhibit_v1` | Handler at [`src/wayland/handlers/keyboard_shortcuts_inhibit.rs`](https://github.com/pop-os/cosmic-comp/blob/master/src/wayland/handlers/keyboard_shortcuts_inhibit.rs) | Needed for robust "hold modifier, Tab to cycle, release to commit" UX |
| **NOT implemented** | `zwlr_foreign_toplevel_manager_v1` (wlr-foreign-toplevel-management) | **Absent** — no `wlr_foreign_toplevel` handler module, import, or delegate anywhere in cosmic-comp `src/` (grepped `state.rs`, `lib.rs`, `main.rs`, `wayland/`, handlers list); no such dependency in `Cargo.toml` | Breaks rofi/wofi window modes (§5.3) |

Client-side helpers exist in the same repo as the protocols: **cosmic-client-toolkit (`cctk`)** — [`client-toolkit/src/toplevel_info.rs`](https://github.com/pop-os/cosmic-protocols/blob/main/client-toolkit/src/toplevel_info.rs) exposes `ToplevelInfo { title, app_id, state, output, geometry, workspace: HashSet<ExtWorkspaceHandleV1>, … }` (line 40; "Requires zcosmic_toplevel_info_v1 version 3"), and [`client-toolkit/src/workspace.rs`](https://github.com/pop-os/cosmic-protocols/blob/main/client-toolkit/src/workspace.rs) exposes `Workspace { name, coordinates, state, capabilities, … }`. Reference examples: [`examples/workspaces.rs`](https://github.com/pop-os/cosmic-protocols/blob/main/client-toolkit/examples/workspaces.rs), [`examples/toplevel-monitor.rs`](https://github.com/pop-os/cosmic-protocols/blob/main/client-toolkit/examples/toplevel-monitor.rs), [`examples/toplevel-list.rs`](https://github.com/pop-os/cosmic-protocols/blob/main/examples/toplevel-list.rs).

### 4.2 Integration hooks (how Airlock gets invoked)

Two clean, zero-patch hooks, both user-configurable:

1. **Override the system action command** (recommended — keeps Alt+Tab semantics): copy `/usr/share/cosmic/com.system76.CosmicSettings.Shortcuts/v1/system_actions` to `~/.config/cosmic/com.system76.CosmicSettings.Shortcuts/v1/system_actions` and set e.g. `WindowSwitcher: "airlock alt-tab"` / `WindowSwitcherPrevious: "airlock shift-alt-tab"` (mechanism: [cosmic-settings-daemon `system_actions()`](https://github.com/pop-os/cosmic-settings-daemon/blob/master/config/src/shortcuts/mod.rs#L51-L69); worked example documented in [cosmic-altswitcher README](https://github.com/jibsta210/cosmic-altswitcher#install); [cosmic-ext-peek-switcher](https://github.com/kuriexmachina/cosmic-ext-peek-switcher) automates exactly this via `make install`). Caveat: **only one program can own the `WindowSwitcher` action** (peek-switcher README, "Notes").
2. **Custom shortcut bound to a command**: Settings → Input → Keyboard → Shortcuts → Custom Shortcuts binds any key combo to `Action::Spawn(command)` (source: [`cosmic-settings .../shortcuts/custom.rs` line 502](https://github.com/pop-os/cosmic-settings/blob/master/cosmic-settings/src/pages/input/keyboard/shortcuts/custom.rs#L502); compositor executes it via `Action::Spawn` → [`actions.rs` line 1050](https://github.com/pop-os/cosmic-comp/blob/master/src/input/actions.rs#L1050)). Official docs: [System76, *Modify Keyboard Shortcuts*](https://support.system76.com/articles/keyboard-shortcuts) (updated Aug 2026) — "Enter a name for the shortcut, the application or command to launch, and the key combination." cosmic-switch-less is deployed this way (§5.1).

### 4.3 Recommended architecture

**External app; do not patch cosmic-comp.** Everything required is exposed via `cctk`:

- maintain a live window list (`zcosmic_toplevel_info_v1`) and, per window, its workspace set (`ToplevelInfo.workspace`);
- maintain the active workspace (`cctk` `Workspace.state::Active`) and show only windows whose workspace set contains it (include sticky/all-workspace windows — they are "visible" on the current workspace per the XML);
- MRU ordering from `state` `Activated` transitions (same approach as the stock plugin, [`cosmic_toplevel/mod.rs` lines 73–87](https://github.com/pop-os/launcher/blob/master/plugins/src/cosmic_toplevel/mod.rs#L73-L87));
- activate selection with `zcosmic_toplevel_manager_v1.activate(toplevel, seat)`;
- run as a **single-instance daemon** (D-Bus or Unix signals) behind a tiny spawn stub so every Alt+Tab press is instant — this is precisely the pattern of cosmic-launcher, cosmic-altswitcher (D-Bus) and cosmic-switch-less (SIGUSR1/SIGUSR2);
- render via layer-shell + `keyboard_shortcuts_inhibit` for the hold-modifier/release-to-commit gesture (both protocols implemented by cosmic-comp, table above).

**Known cost of the external approach** (from #1777, lkonga, 2026-08-17): a libcosmic-based daemon measured ~170 MB RSS with ~33 threads (wgpu/Vulkan), and 50–500 ms cold-start unless pre-warmed. A minimal cctk + sctk implementation (no libcosmic app framework, no GPU thumbnails) can be far lighter; thumbnails would additionally require `ext-image-copy-capture` (implemented by cosmic-comp: handler [`image_copy_capture.rs`](https://github.com/pop-os/cosmic-comp/blob/master/src/wayland/handlers/image_copy_capture.rs)).

**Blocked / not available:** there is **no way to query or influence the *stock* launcher's list** from outside (the pop-launcher plugin protocol has no workspace filter field — [`Request`/`PluginResponse` types used by the plugin](https://github.com/pop-os/launcher/blob/master/plugins/src/cosmic_toplevel/mod.rs#L18-L21) carry no workspace data), and **no compositor API to reorder or filter the stock switcher**. Replacement, not augmentation, is the model. A cosmic-comp/cosmic-launcher patch would only be needed to make *stock* COSMIC workspace-aware — which is exactly what issue #1777 tracks upstream.

---

## 5. Workarounds available today (2026-10)

### 5.1 Workspace-scoped switchers that work on stock COSMIC

| Tool | Scope | Integration | Status / caveats | Source |
|---|---|---|---|---|
| **cosmic-switch-less** (riverfr0zen) | **Current workspace only**, MRU | *Custom Shortcuts*: `Alt+Tab` → `cosmic-switch-less-show`, `Alt+Shift+Tab` → `cosmic-switch-less-show --back` | Daemon + SIGUSR1/2 model; "works pretty well" per author; **heavy**: reported ~170 MB RSS / wgpu renderer / 50–500 ms cold start (issue #1777 comment by lkonga); prebuilt x86_64 tarball | [README](https://github.com/riverfr0zen/cosmic-switch-less) (released 2026-05, announced in [#1777](https://github.com/pop-os/cosmic-epoch/issues/1777#issuecomment-4452219717)) |
| **cosmic-ext-peek-switcher** (KuriExMachina) | **Current workspace**, live thumbnails | `make install` redirects the `WindowSwitcher` system action; Super+Tab to use | "Early but working… on COSMIC 1.7 (multi-monitor)"; fork of pop-os/cosmic-workspaces-epoch; only one program may own `WindowSwitcher` | [README](https://github.com/kuriexmachina/cosmic-ext-peek-switcher) |
| cosmic-ext-app-switcher (j0rdiun) | UNVERIFIED (listed as another `WindowSwitcher` override) | `WindowSwitcher` action override | mentioned only as a conflicting owner of the action in peek-switcher's README | [peek-switcher README note](https://github.com/kuriexmachina/cosmic-ext-peek-switcher#notes) |

### 5.2 Non-scoped replacement switchers (prove the mechanism; no workspace filter)

- **cosmic-altswitcher** (jibsta210): Windows-11-style thumbnails via `ext-image-copy-capture`; early prototype; no MRU, **no workspace filtering claimed**; documents the `system_actions` override recipe. [README](https://github.com/jibsta210/cosmic-altswitcher).
- **POP-Flow** (NicoArgo): "Windows-style Alt-Tab with live thumbnails"; workspace scope **UNVERIFIED** (README not inspected for filtering). [Repo](https://github.com/NicoArgo/POP-Flow).

### 5.3 Generic launchers — do **not** work for window switching on COSMIC

- **rofi**: classic window mode is X11/EWMH-based (`_NET_CLIENT_LIST_STACKING`, XCB-EWMH — [`source/modes/window.c`](https://github.com/davatorium/rofi/blob/next/source/modes/window.c)), which cannot see Wayland-native windows; its Wayland window mode requires `zwlr_foreign_toplevel_manager_v1` ([`source/modes/wayland-window.c`](https://github.com/davatorium/rofi/blob/next/source/modes/wayland-window.c)), which cosmic-comp does not implement (§4.1).
- **wofi**: binds `zwlr_layer_shell_v1` for its own window (verified in [`src/wofi.c`](https://hg.sr.ht/~scoopta/wofi/raw/src/wofi.c)) but has no cosmic toplevel source; same wlr-foreign-toplevel gap. No workspace filter possible either way.
- No `cosmic-ext-*` tool other than those in §5.1 advertises workspace-filtered switching (searched the `pop-os` GitHub topic and issue threads; negative result, 2026-10-22).

### 5.4 Adjacent native features (partial relief, not Alt+Tab)

- **Dock app-list workspace filtering** (shipped): `~/.config/cosmic/com.system76.CosmicAppList/v1/filter_top_levels` = `Some(ActiveWorkspace)` or `Some(ConfiguredOutput)` ([cosmic-applets#1236](https://github.com/pop-os/cosmic-applets/pull/1236), merged 2026-04-09). Filters the taskbar/dock, **not** Alt+Tab.
- **Super+arrows / Super+hjkl** `Focus(direction)` spatial navigation stays within the workspace until an edge is reached (then may switch workspace) — [`actions.rs` lines 746–812](https://github.com/pop-os/cosmic-comp/blob/master/src/input/actions.rs#L746-L812). Usable for two-window ping-pong, not a visual switcher.
- Typing a window title in the launcher (Super+/) surfaces windows via the same unfiltered plugin.

---

## 6. How other desktops behave by default

| Desktop | Alt+Tab default scope | Configurable? | Source |
|---|---|---|---|
| **KDE Plasma (KWin)** | **Current virtual desktop only** (`TabBox/DesktopMode` default `1` = `OnlyCurrentDesktopClients`; also hard-coded `defaultDesktopMode()` returns `OnlyCurrentDesktopClients`) | Yes — Task Switcher "Filter windows by: Virtual desktops: Current desktop / All desktops" | [`src/kwin.kcfg` group TabBox](https://invent.kde.org/plasma/kwin/-/blob/master/src/kwin.kcfg) (lines 275–280); [`src/tabbox/tabboxconfig.h` lines 44–50, 269–271](https://invent.kde.org/plasma/kwin/-/blob/master/src/tabbox/tabboxconfig.h) |
| **Windows 11** | Setting exists: *Settings → System → Multitasking → Desktops → "Show all open windows when I press Alt+Tab"* = `Only on the desktop I'm using` / `On all desktops`. **Default value not stated in official docs — UNVERIFIED** | Yes (taskbar has a parallel option) | [Configure Multiple Desktops in Windows](https://support.microsoft.com/en-us/windows/experience/configure-multiple-desktops-in-windows); [Make it easier to focus on tasks](https://support.microsoft.com/en-us/accessibility/windows/make-it-easier-to-focus-on-tasks) |
| **macOS (Spaces)** | App-based ⌘-Tab spans Spaces; by default switching to an app jumps to a Space with its windows ("When switching to an application, switch to a Space with open windows for the application", default **on**). Within a Space, only that Space's windows are shown | Yes (toggle the same setting in Desktop & Dock → Mission Control) | [Work in multiple spaces on Mac](https://support.apple.com/en-al/guide/mac-help/mh14112/mac); [Change Desktop & Dock settings](https://support.apple.com/guide/mac-help/change-desktop-dock-settings-mchlp1119/15.0/mac/15.0) |
| **GNOME (Shell)** | **Split personality:** app-switcher (Alt+Tab) `current-workspace-only` default **false** (all workspaces); window-switcher (the secondary per-window switcher) `current-workspace-only` default **true** | Yes, via gsettings key per switcher | [`data/org.gnome.shell.gschema.xml.in`](https://gitlab.gnome.org/GNOME/gnome-shell/-/raw/main/data/org.gnome.shell.gschema.xml.in) — `org.gnome.shell.app-switcher` (lines 307–318, default `false`) and `org.gnome.shell.window-switcher` (lines 337–343, default `true`) |
| **COSMIC** | **All workspaces** (see §2) | **Not yet** — tracked in cosmic-epoch#1777 | this document, §2–§3 |

Takeaway: "current workspace/desktop only" is a mainstream default (KWin hard-defaults to it; GNOME's window switcher defaults to it; Windows 11 and macOS offer first-class toggles), so the user's expectation matches industry practice — which is the framing the #1777 thread uses.

---

## 7. Sources

**pop-os source code (primary):**

1. cosmic-comp `Cargo.toml` (cosmic-protocols dep, smithay pin) — https://github.com/pop-os/cosmic-comp/blob/master/Cargo.toml
2. cosmic-comp default keybindings — https://github.com/pop-os/cosmic-comp/blob/epoch-1.11.0/data/keybindings.ron (also master)
3. cosmic-comp system-action spawning — https://github.com/pop-os/cosmic-comp/blob/epoch-1.11.0/src/input/actions.rs (L1044–1048; also L746–812 focus-direction fallbacks; L1050 `Action::Spawn`)
4. cosmic-comp config load of `system_actions` — https://github.com/pop-os/cosmic-comp/blob/master/src/config/mod.rs (L75–76)
5. cosmic-comp-config full schema (no switcher option) — https://github.com/pop-os/cosmic-comp/blob/master/cosmic-comp-config/src/lib.rs
6. cosmic-comp toplevel-info server impl (emits `ext_workspace_enter/leave`) — https://github.com/pop-os/cosmic-comp/blob/master/src/wayland/protocols/toplevel_info.rs (L634–659) and handler https://github.com/pop-os/cosmic-comp/blob/master/src/wayland/handlers/toplevel_info.rs
7. cosmic-comp toplevel-management server impl (`activate`) — https://github.com/pop-os/cosmic-comp/blob/master/src/wayland/protocols/toplevel_management.rs (L188–190)
8. cosmic-comp workspace protocol impl (ext-workspace Activate; State::Active) — https://github.com/pop-os/cosmic-comp/blob/master/src/wayland/protocols/workspace/ext.rs (L193–212) and https://github.com/pop-os/cosmic-comp/blob/master/src/wayland/protocols/workspace/mod.rs (L28–60)
9. cosmic-comp protocol state registration — https://github.com/pop-os/cosmic-comp/blob/master/src/state.rs (L27–29, 315–319)
10. cosmic-comp handlers index (incl. `layer_shell.rs`, `keyboard_shortcuts_inhibit.rs`, `image_copy_capture.rs`, `foreign_toplevel_list.rs`; **no** wlr-foreign-toplevel handler) — https://github.com/pop-os/cosmic-comp/blob/master/src/wayland/handlers/mod.rs
11. cosmic-comp tag epoch-1.11.0 (2026-10-14) — https://github.com/pop-os/cosmic-comp/releases/tag/epoch-1.11.0
12. cosmic-settings-config `Action`/`System` enums — https://github.com/pop-os/cosmic-settings-daemon/blob/master/config/src/shortcuts/action.rs
13. cosmic-settings-daemon `system_actions()` (system vs user config) — https://github.com/pop-os/cosmic-settings-daemon/blob/master/config/src/shortcuts/mod.rs
14. cosmic-settings custom shortcuts page (`Action::Spawn`) — https://github.com/pop-os/cosmic-settings/blob/master/cosmic-settings/src/pages/input/keyboard/shortcuts/custom.rs
15. cosmic-launcher `AltTab`/`ShiftAltTab` subcommands + alt-tab list handling — https://github.com/pop-os/cosmic-comp/blob/master/src/input/actions.rs and https://github.com/pop-os/cosmic-launcher/blob/master/src/app.rs
16. pop-os/launcher `cosmic_toplevel` plugin (no workspace filter; MRU; activate via toplevel-management) — https://github.com/pop-os/launcher/blob/master/plugins/src/cosmic_toplevel/mod.rs and https://github.com/pop-os/launcher/blob/master/plugins/src/cosmic_toplevel/toplevel_handler.rs
17. cosmic-protocols XMLs — toplevel-info: https://github.com/pop-os/cosmic-protocols/blob/main/unstable/cosmic-toplevel-info-unstable-v1.xml · toplevel-management: https://github.com/pop-os/cosmic-protocols/blob/main/unstable/cosmic-toplevel-management-unstable-v1.xml · workspace v2: https://github.com/pop-os/cosmic-protocols/blob/main/unstable/cosmic-workspace-unstable-v2.xml
18. cosmic-client-toolkit (ToplevelInfo.workspace; Workspace.state; examples) — https://github.com/pop-os/cosmic-protocols/blob/main/client-toolkit/src/toplevel_info.rs · https://github.com/pop-os/cosmic-protocols/blob/main/client-toolkit/src/workspace.rs · https://github.com/pop-os/cosmic-protocols/blob/main/client-toolkit/examples/workspaces.rs

**Issue tracker (primary):**

19. pop-os/cosmic-epoch#1777 (master tracking issue, open, maintainer statements) — https://github.com/pop-os/cosmic-epoch/issues/1777
20. pop-os/cosmic-launcher#450 (open, transferred) — https://github.com/pop-os/cosmic-launcher/issues/450
21. pop-os/cosmic-launcher#355 (closed as dup) — https://github.com/pop-os/cosmic-launcher/issues/355
22. pop-os/cosmic-launcher#173 + PRs #183, cosmic-settings-daemon#49, cosmic-comp#807, cosmic-settings#675 (reverse cycling, completed) — https://github.com/pop-os/cosmic-launcher/issues/173
23. pop-os/cosmic-launcher issue search (workspace switcher; incl. #130, #135, #231) — https://github.com/pop-os/cosmic-launcher/issues?q=is%3Aissue%20workspace%20switcher
24. pop-os/cosmic-applets#1236 (merged; dock app-list workspace filtering) — https://github.com/pop-os/cosmic-applets/pull/1236
25. pop-os/cosmic-applets#339 (app-list workspace awareness) — https://github.com/pop-os/cosmic-applets/issues/339

**Third-party tools (verified READMEs):**

26. cosmic-switch-less — https://github.com/riverfr0zen/cosmic-switch-less
27. cosmic-ext-peek-switcher — https://github.com/kuriexmachina/cosmic-ext-peek-switcher
28. cosmic-altswitcher — https://github.com/jibsta210/cosmic-altswitcher
29. POP-Flow — https://github.com/NicoArgo/POP-Flow (scope UNVERIFIED)

**Generic launchers (protocol verification):**

30. rofi X11/EWMH window mode — https://github.com/davatorium/rofi/blob/next/source/modes/window.c
31. rofi Wayland window mode (wlr-foreign-toplevel) — https://github.com/davatorium/rofi/blob/next/source/modes/wayland-window.c
32. wofi source (layer-shell use; no cosmic toplevel source) — https://hg.sr.ht/~scoopta/wofi/raw/src/wofi.c

**Official documentation (other DEs / System76):**

33. System76 — Modify Keyboard Shortcuts (custom shortcuts, Aug 2026) — https://support.system76.com/articles/keyboard-shortcuts
34. GNOME Shell gschema (app-switcher default false; window-switcher default true) — https://gitlab.gnome.org/GNOME/gnome-shell/-/raw/main/data/org.gnome.shell.gschema.xml.in
35. KWin kcfg (TabBox DesktopMode default 1) — https://invent.kde.org/plasma/kwin/-/raw/master/src/kwin.kcfg
36. KWin TabBoxConfig (OnlyCurrentDesktopClients default) — https://invent.kde.org/plasma/kwin/-/raw/master/src/tabbox/tabboxconfig.h
37. Microsoft — Configure Multiple Desktops in Windows (Alt+Tab desktop scope setting) — https://support.microsoft.com/en-us/windows/experience/configure-multiple-desktops-in-windows
38. Microsoft — Make it easier to focus on tasks (Multitasking → Desktops → Alt+Tab options) — https://support.microsoft.com/en-us/accessibility/windows/make-it-easier-to-focus-on-tasks
39. Apple — Work in multiple spaces on Mac — https://support.apple.com/en-al/guide/mac-help/mh14112/mac
40. Apple — Change Desktop & Dock settings ("When switching to an application…") — https://support.apple.com/guide/mac-help/change-desktop-dock-settings-mchlp1119/15.0/mac/15.0
