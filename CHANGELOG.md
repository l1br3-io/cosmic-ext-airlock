# Changelog

Consumer-facing changes, newest first. Phrased around what you can
observe, not internal mechanics.

## [Unreleased]

### Added

- **The `layout` setting — a horizontal switcher card** —
  `layout = "horizontal"` lays the same MRU list out as a wide band of
  tiles in a short card: each tile the window's app icon with its
  title beneath (clamped to the tile), a `…` tile at the truncated
  edge, the band scrolling just enough as you step past it. Left and
  Right become the primary step keys; Ctrl+1..9, hover, click, and the
  wheel address tiles exactly as they address rows. `columns` sizes
  the band — `"auto"` (the default) fits two-thirds of the invoking
  output's logical width per monitor, or a whole number 3–12 pins the
  tile count. Both live-apply at the next gesture; the default
  vertical card is unchanged, and a grouped session still renders the
  vertical grouped card.
- **The `font` setting and real typography** — titles render in the
  desktop's own interface font by default (Open Sans on a default
  desktop; flip COSMIC Settings' font and the next Alt+Tab follows),
  and `font = "Fira Sans"` — any installed family — overrides it,
  live-applying at the next gesture. Unicode titles render their real
  glyphs — CJK, emoji, accents — through fallback chains, replacing
  the ASCII folding of every earlier version (the fold survives only
  as the zero-fonts bitmap floor, the paint of last resort on a system
  with no loadable fonts). Size stays 12 px and weight Regular —
  family only. A family nothing installs logs one diagnostic naming
  the key and paints the interface font; font I/O can never break
  switching. Note: fonts are scanned once at daemon startup, so a font
  installed later needs a daemon restart to appear.

- **Desktop-followed geometry** — the card's shape follows the COSMIC
  desktop, no keys. The corner radius follows Appearance → Style
  (Round / Slightly-Round / Square → 12 / 8 / 2 px, the selection
  band rounding with the card, capped at 8), and the row rhythm
  follows the interface density (Compact / Comfortable / Spacious →
  40 / 48 / 56-px icon-driven rows; the card's width never moves).
  Both apply at the next Alt+Tab after the desktop setting flips; a
  factory-default desktop (Round + Comfortable) renders exactly the
  card every prior version painted. At Spacious density the `rows`
  budget auto-clamps to the 720-px fit — `rows = 9` or `10` open with
  eight windows in view and one daemon-log diagnostic naming the
  clamp; the setting keeps its value.

- **App grouping** — the `grouping = true` setting re-partitions the
  switcher into a two-level tree. Level 1 is the app layer, collapsed
  by default: the card opens on app entries alone, each committing its
  app's most recently used window. Tab, Shift+Tab, the arrows, and the
  mouse wheel are one linear walk over the visible rows — entries,
  leaves, and expanded groups' windows together, wrapping, with an
  expanded entry a normal row like any other. → expands the selected
  group and enters it (landing one past the window you just left when
  it's your own app); ← returns a child to its entry (the group stays
  expanded) and collapses an expanded entry; each app's
  expanded/collapsed state persists across gestures; Ctrl+N activates
  an app entry's window instantly. Nothing ever descends into a
  collapsed group. An entry's structure reads at a glance: a solid
  disclosure triangle (right = collapsed, down = expanded) and a count
  chip beside its title. See
  docs/settings.md — Grouping.
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
- The `…` overflow marker sits centered on the card again (it drifted
  2 px right), and the selection band's ends round to match the card's
  corners instead of ending as clipped stripes; the card edge now
  breathes twice the row gap.
- The starter config's "full reference" comment links straight to the
  settings doc instead of the repository root. Existing configs are
  never rewritten; only fresh installs write the new line.

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
