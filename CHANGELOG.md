# Changelog

Consumer-facing changes, newest first. Phrased around what you can
observe, not internal mechanics.

## [0.1.0] — 2026-11-03

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
- **Same-application cycling** (Alt+grave, Alt+Shift+grave) and a
  **no-overlay quick toggle** (Alt+Esc) to the previous window.
- **Per-workspace isolation config**: a workspace can opt into the
  global switcher (`isolated = false`) by name or position.
- **`airlock install`**: the complete setup — claims Alt+Tab at user
  level and places the app icon and desktop entry in `~/.local/share`;
  `--uninstall` reverts everything. Already-bound keys are skipped and
  named, never clobbered; a hand-edited asset file is kept and named,
  never deleted.
- Small print: ~8 MB RSS daemon, single-threaded, warm presses in
  single-digit milliseconds.
