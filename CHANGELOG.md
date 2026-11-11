# Changelog

Consumer-facing changes, newest first. Phrased around what you can
observe, not internal mechanics.

## [Unreleased]

### Fixed

- **The × closes again on the labeled vertical card** — with the
  workspace label showing (`show_workspace = true`), the vertical
  card's × badge revealed in the right place but its click territory
  sat 16 px above it, so a click on the visible × fell through and
  activated the window instead of closing it. The territory now sits
  exactly under the badge it belongs to.

### Added

- **The pointing-hand cursor over the card** — the cursor turns into
  the hand the moment it enters the card and stays until it leaves:
  everything on the card is a click target. Needs the compositor's
  cursor-shape protocol (COSMIC has it); without it the cursor simply
  stays whatever it was.

### Changed

- **A bigger × on every thumbnail** — the close badge a hovered window
  tile reveals on its icon's top-right corner grows from 12 to 16
  logical px (the × ink keeps its size, gaining air), and its click
  territory grows with it (16 → 20 px), so the smallest target on the
  card is easier to see and to hit. Both cards share the one badge, so
  the vertical rows' × grows with it.

## [0.4.1] — 2026-11-10

### Changed

- **Tab keeps an open run open on the strip** — in the grouped
  horizontal card, pressing Alt+Tab from inside an app's open window
  run used to jump straight to the next app tile and close the run.
  It now steps the run's windows one at a time, like the arrows and
  like the vertical tree's Tab through an expanded group, and the run
  closes only when the walk steps off its edge. From an app tile, Tab
  still steps app tiles.
- **The key above Tab collapses an open group** — pressing ` on an app
  entry that is already expanded now collapses the group in place
  instead of opening it again; a collapsed entry still opens on the
  most recently used window, **Shift+`** on the last. Inside a group
  the key is unchanged: ` cycles to the app's next window and
  **Shift+`** to the previous one, wrapping within the app.

## [0.4.0] — 2026-11-09

### Added

- **The `font_size` setting** — set the card's text size in logical
  pixels, 8–24 (default 12). Titles, the query strip, hint digits, and
  the horizontal band's caption and badges all follow, as do the lines
  and heights that hold them: rows grow to keep the count pips clear,
  and the query strip and band caption grow with their text. The title
  band keeps its physical width by fitting fewer characters instead of
  stretching, so a larger size elides long titles sooner rather than
  widening the card. Rows growing means fewer windows fit the 720-px
  panel, so the `rows` budget clamps as it does at Spacious density;
  the daemon log names the density, text size, and clamp. The 8×8
  bitmap floor scales too. Read when a gesture starts; a value outside
  8–24 is named in a diagnostic and the last-good settings apply.
- **The key above Tab walks the vertical tree too** — the ` carve-out
  that steps the selected app's windows no longer belongs to the
  horizontal strip alone: a vertical grouped card steps as well,
  where it used to type a backtick into the query. From an app entry,
  ` opens that app's windows on their most recently used one
  (Shift+` on the last), and the group expands for it — the same
  persistent expansion state the → key writes; from a window row it
  steps within the same app, wrapping, never crossing into another,
  and a single-window app holds without dead ends. With a query
  standing the key types its own character as before, and the flat
  card keeps typing it outright. See docs/settings.md — Grouping.

## [0.3.0] — 2026-11-09

### Added

- **The `width` and `min-width` settings** — hold either card at a
  width floor: `"auto"` (the default) keeps today's content-sized
  card, `"100%"` spans the invoking output's logical width, and any
  percentage 1–100 or whole pixel count sets the floor. The card grows
  to the larger of the two keys and its content, so a value below what
  the card needs changes nothing and a title never clips; a pixel
  count past the output clamps to it with a logged note. Rows and
  tiles stay left-aligned in the air the floor buys, and the edit
  live-applies at the next gesture.
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
- **Type-to-filter** — while the card is up, typing narrows it: the
  query collects in a strip above the rows (a static caret marking its
  end, the tail shown when it overflows), matching windows by title or
  application name case-insensitively, selecting the first match. Tab,
  the arrows, and the wheel step the matches; Enter or Alt-release
  commits the selected one. Backspace edits; Escape clears a standing
  query before dismissing; a query matching nothing keeps the card with
  a one-word verdict instead of closing it. Works in every mode — flat,
  grouped (the tree flattens while a query stands), and the same-app
  view — because it narrows the frozen list, and it never re-derives
  that list mid-gesture. See docs/how-it-works.md — Type-to-filter.
- **Close windows from the card** — Alt+minus (top-row or keypad) closes
  the selected row's window; hovering a window row reveals a small ×
  badge on its icon's top-right corner that closes on click. The
  horizontal band and grouped strip paint the same badge on their
  tiles, so the pointer affordance is identical in either layout; the
  rest of the row or tile still activates. A group entry closes its app's front window
  — every row closes what it activates. After each close the selection
  advances to the next visible row and the card stays up for repeated
  closes; closing the last eligible window dismisses the overlay.
  Alt+minus works in every mode; the × is the pointer's window-row and
  window-tile affordance (never group entries, app tiles, or `…` rows).
  See docs/how-it-works.md — Closing windows.
- **A starter config on install**: `airlock install` now drops a fully
  commented `~/.config/airlock/config.toml` — every setting documented,
  nothing enabled, so behavior is unchanged until you uncomment a line.
  An existing config is never touched, and `--uninstall` removes the
  starter only if you never edited it.

### Changed

- **The grouped tree's marks quieted.** A grouped row now shows a bare
  digit for its Ctrl+N key — the same digit the horizontal tiles paint —
  instead of repeating `Ctrl+N` on every row, and the card gives that
  column back (it narrows by 60 px). An app entry's window count is the
  horizontal band's pip vocabulary, adapted below the app name: muted
  dots that saturate at three, with the small caret on the title line
  replacing the large solid triangle that floated at the card's right
  edge.
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

### Fixed

- **Config edits reliably live-apply at the next gesture.** The reload
  check compared mtimes, which miss edits on filesystems with coarse or
  stale timestamps (VM shared folders are the known case) and never move
  when a boolean flips — `grouping = true` to `false` keeps the byte
  count — so a changed config could serve stale settings until the
  daemon restarted. The check now compares the file's bytes, and every
  adoption logs one line (`config change adopted: layout=… grouping=…`)
  so an ignored edit is traceable in the journal.
- **Alt+Shift+` steps the focused app's windows backwards** in the
  grouped strip. Holding Shift delivers the shifted keysym (`~` on a US
  layout), which the key match did not recognize, so the documented
  backwards step never fired. Both levels now map; typing ` and `~`
  into a query still works everywhere.

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
