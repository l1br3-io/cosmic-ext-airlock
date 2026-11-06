# Changelog

Consumer-facing changes, newest first. Phrased around what you can
observe, not internal mechanics.

## [0.1.0] — 2026-11-06

Initial release.

### Added

- **Workspace-isolated Alt+Tab**: the overlay lists only the current
  workspace's windows in most-recently-used order — sticky windows
  appear everywhere, minimized windows nowhere, other workspaces'
  windows never. Tab / Shift+Tab / arrow keys walk the list; releasing
  Alt or pressing Enter commits; Escape dismisses; a quick tap still
  switches before the overlay appears.
- **Application icons in the overlay**, resolved through the same
  desktop-entry and icon-theme pipeline the COSMIC launcher uses; apps
  without a themed icon fall back honestly. Text renders at 8 px and
  stays sharp on HiDPI displays.
- **Long lists stay glanceable**: the card shows at most eight rows at
  a time; `…` rows mark where the list continues, and the window
  follows the selection when stepping runs past an edge.
- **Same-application cycling** (Alt+grave, Alt+Shift+grave) and a
  **no-overlay quick toggle** (Alt+Esc) to the previous window.
- **Pointer input on the overlay**: crossing rows with the mouse moves
  the selection, clicking a row focuses it, and the scroll wheel steps
  the list — the same outcomes as the keyboard gesture.
- **Direct row selection with Ctrl+1..9**: while the overlay is up,
  Ctrl plus a number key activates that row immediately; each row
  shows its key next to it.
- **Per-workspace isolation config**: a workspace can opt into the
  global switcher (`isolated = false`) by name or position.
- **Theme setting**: `theme = "dark"`, `"light"`, or `"auto"` (default
  `auto`) — the card follows the desktop's dark/light preference, with
  a light variant that stays readable on bright wallpapers.
- **`airlock install`**: the complete setup — claims Alt+Tab at user
  level and places the app icon and desktop entry in `~/.local/share`;
  `--uninstall` reverts everything. Already-bound keys are skipped and
  named, never clobbered; a hand-edited asset file is kept and named,
  never deleted.
- Small print: ~8 MB RSS daemon, single-threaded, warm presses in
  single-digit milliseconds.
