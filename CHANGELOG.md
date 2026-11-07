# Changelog

Consumer-facing changes, newest first. Phrased around what you can
observe, not internal mechanics.

## [Unreleased]

### Added

- **A starter config on install**: `airlock install` now drops a fully
  commented `~/.config/airlock/config.toml` — every setting documented,
  nothing enabled, so behavior is unchanged until you uncomment a line.
  An existing config is never touched, and `--uninstall` removes the
  starter only if you never edited it.

### Changed

- **Titles degrade less, stated the same**: Latin accents fold to their
  base letter (`é` → `e`) and dropped combining marks no longer read as
  `?` (a macOS-built `café.txt` shows `cafe.txt`). Punctuation folds as
  before; anything with no ASCII form — CJK, Cyrillic, ligatures like
  `æ` — still renders as `?`.

### Fixed

- **The `grouping` setting has its reference at last**: the
  docs/settings.md section planned when the setting shipped — the app
  tree's shape, the re-bound keys (Tab steps apps, → enters a parent's
  children, ← exits), and the read-at-gesture-start contract.
- The `…` overflow marker sits centered on the card again (it drifted
  2 px right), and the selection band's ends round to match the card's
  corners instead of ending as clipped stripes; the card edge now
  breathes twice the row gap.

## [0.2.0] — 2026-11-07

### Added

- **`rows` setting**: how many windows the card keeps in view on a long
  list (`rows = 3`–`10` in `~/.config/airlock/config.toml`, default 8).
  The `…` continuation rows now ride on top of that budget instead of
  spending it — a long list keeps eight windows in view mid-list where
  it showed six.
- **Ctrl+1..9 picks a row directly**: with the overlay open, press Ctrl
  plus a number (top row or keypad) to switch to that row's window at
  once — the same outcome as clicking it. The first nine rows show
  their key at the right edge. (This shipped in 0.1.1's binary; its
  notes missed it.)

### Changed

- **Larger titles**: overlay text renders at 12 px (was 8), staying
  legible at a glance on 100%-scaled displays; the card widens to at
  most 748 px and long titles truncate a little sooner to match.

## [0.1.1] — 2026-11-07

### Fixed

- **Installing no longer resets every shortcut to defaults.** 0.1.0 wrote
  its Alt+grave / Alt+Shift+grave / Alt+Esc bindings in a form COSMIC
  rejects, which made cosmic-comp discard the `custom` shortcuts file —
  the file that holds all your customizations — and run factory defaults
  for the live session. Your entries were never deleted from disk:
  `airlock install --uninstall` (any version) restores them byte-exactly,
  or upgrade and re-run `airlock install` to repair the file in place.
- **Alt+Esc now actually fires** — 0.1.0 bound the non-existent `Esc`
  keysym; the key's real name is `Escape`.
- Re-running install over a 0.1.0 machine replaces the malformed entries
  with valid ones instead of leaving them beside the new ones.

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
