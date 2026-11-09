# Settings

By default, Airlock seals Alt+Tab to the current workspace. A config file
can opt individual workspaces out: there the switcher lists every
non-minimized window across **all** workspaces, most-recently-used first,
while workspaces without an entry stay isolated exactly as before. The
same file also picks the switcher's card color — light, dark, or
following the desktop — its typeface and text size, and how many windows
the card keeps in view.

Airlock reads the file; you edit it. `airlock install` drops a starter
copy at the path below — every setting commented out, so it means the
defaults until you uncomment something. Airlock never rewrites it after
that, and never creates, renames, or rearranges workspaces — names and
positions come from COSMIC itself.

## The file

```
~/.config/airlock/config.toml
```

More precisely: `airlock/config.toml` under an absolute `$XDG_CONFIG_HOME`
when one is set, else under `$HOME/.config` — the same rule the installer
uses. No file, an empty one, or the all-commented starter all mean the
same thing: every workspace isolated, the default behavior, nothing to
configure.

## Example

```toml
# ~/.config/airlock/config.toml — Airlock reads, you edit.
# Workspaces without an entry keep the default: isolated.

theme = "auto"         # match the desktop's dark/light mode (the default)
# font = "Fira Sans"   # absent (the default): follow the COSMIC interface font
# font_size = 12       # card text size, logical px: 8–24 (the default: 12)
rows = 8               # windows the card keeps in view (the default)
layout = "vertical"    # the tall card; "horizontal" lays it out as a band of tiles
# columns = "auto"     # the band's tile budget; or a whole number 3–12 (the default: "auto")
# width = "auto"       # the card's width floor; "100%" fills the screen, or a pixel count (the default: "auto")
# min-width = "auto"   # the card's never-below width; same values as width (the default: "auto")
grouping = false       # the flat list (the default); true groups it by app
# show_workspace = false # label the card with the workspace you pressed on (the default: false)

[[workspace]]
name = "code"          # matches a workspace COSMIC reports as named "code"
isolated = true        # the default — shown for completeness

[[workspace]]
index = 2              # the 2nd workspace in COSMIC's order, named or not
isolated = false       # Alt+Tab here cycles all windows, MRU
```

## Keys

Each `[[workspace]]` entry takes exactly one reference plus an optional
flag:

| Key | Type | Meaning |
|-----|------|---------|
| `name` | string | Matches a workspace COSMIC reports with exactly this name |
| `index` | whole number ≥ 1 | Matches the workspace at this 1-based position |
| `isolated` | boolean | `true` (default): only this workspace's windows; `false`: all windows, MRU |

An entry needs `name` or `index` — never both.

The top-level keys: `theme` picks the card (see
[Theme](#theme)); `font` picks its typeface and `font_size` its text
size (see [Font](#font) and [Font size](#font-size)); `rows` picks how
many windows it keeps in view (see [Rows](#rows)); `layout` picks the
card's axis and `columns` the horizontal band's width (see
[Layout](#layout)); `grouping` picks the list's shape (see
[Grouping](#grouping)); `show_workspace` labels the card with the
workspace you pressed on (see [Show workspace](#show-workspace)).

## Referencing workspaces

- **Name-first.** A named workspace matches `name` rules before `index`
  rules. `index` is the fallback (and covers unnamed workspaces, which
  have nothing to match by name).
- **A `name` rule follows the name, not the workspace.** Names are read
  live on every press: rename `code` to `dev` in COSMIC and the rule
  stops matching that workspace on the next invocation — and matches
  whichever workspace later takes the name `code`.
- **Positions shift with the grid.** `index` is the workspace's 1-based
  place in COSMIC's order (sorted by grid coordinates). Creating,
  removing, or moving workspaces re-points an `index` rule at whatever
  now sits there. Prefer `name` for workspaces you have named.
- **Out-of-range `index` matches nothing** — that workspace falls back to
  the isolated default.
- **Duplicates are allowed; the last entry wins.** If several entries
  reference the same workspace, the one latest in file order applies. No
  error.
- **No match at all** → the workspace stays isolated. An entry can only
  opt out, never opt others in.

## Theme

`theme` picks the card the switcher paints — the near-opaque panel the
window list sits on. Three values:

- **`"auto"` (default)** — follow the COSMIC system dark/light
  preference: a dark-mode desktop paints the dark card, a light-mode
  desktop the light one. The preference is checked each time a gesture
  starts, so flipping dark mode — in COSMIC Settings or on a schedule —
  shows up on the next Alt+Tab. No daemon restart.
- **`"dark"`** — always the dark card, whatever the desktop prefers.
  The escape hatch for the previous always-dark behavior.
- **`"light"`** — always the light card.

Theme can never break switching. An unreadable system preference paints
the dark card (today's look) with one diagnostic in the daemon log; a
malformed `theme` value rides the same last-good fallback as every
other config error.

And like every setting, `theme` is read when a gesture starts. An edit
mid-gesture changes nothing until the next press — a session keeps the
card it was opened with until Alt is released.

## Rows

`rows` sets how many windows the card keeps in view on a long list —
a whole number from 3 to 10, default 8. The `…` continuation rows at
truncated edges ride on top of that budget, never inside it: with the
default, a long list shows eight windows mid-list, nine at either
end, and a list of ten or fewer shows every window with no `…` at
all.

The card grows by two rows once the list outgrows `rows` (the `…`
slots). At the default text size that is 584 logical pixels tall —
fine on any 100%-scaled display and on a 1440p screen at 200%; a 1080p
panel at 200% (a 540-logical screen) is the one configuration that
wants less — `rows = 7` keeps the card inside that height. A larger
[`font_size`](#font-size) grows each row, so fewer windows fit and the
budget clamps (see [Follows](#follows)).

Like every setting, `rows` is read when a gesture starts. An edit
mid-gesture changes nothing until the next press; a session keeps the
budget it opened with until Alt is released. A value outside 3–10 (or
a non-integer) is named in a diagnostic and the last-good settings
apply. At Spacious density the effective budget clamps to what fits —
see [Follows](#follows).

## Font

`font` picks the typeface the card's titles paint in — a font family
name, exactly as your font picker shows it. Family only: weight stays
Regular, and size stays the card's own 12 px unless
[`font_size`](#font-size) moves it.

- **Absent key (the default)** — follow the COSMIC interface font: the
  family your desktop already paints its UI in. Open Sans on a default
  desktop, your own COSMIC Settings pick when you have made one. Every
  card upgrades to real typography without configuring anything.
- **`font = "Fira Sans"`** — any installed family; the next Alt+Tab
  paints it.

Any string is valid as far as the config is concerned — a family
nothing installs is not a config error. Resolution happens when a
gesture starts: a name the system does not know logs one diagnostic
naming the `font` key and paints the interface font instead. A system
with no loadable fonts at all paints the built-in bitmap face — the
deterministic floor — and switching never breaks on font I/O either
way.

Real text is the point: Unicode titles render their actual glyphs —
`未設定 — 端末`, `café.txt` — through the font's fallback chains, with
grayscale anti-aliasing and proportional widths. (The ASCII folding
older versions documented is the bitmap floor's behavior, kept for the
no-fonts case only.)

Fonts are scanned once at daemon startup: a font installed while the
daemon runs is picked up at the next daemon restart, not before.

Like every setting, `font` is read when a gesture starts. An edit
mid-gesture changes nothing until the next press — a session keeps the
face it was opened with until Alt is released.

## Font size

`font_size` sets the card's text size in logical pixels — a whole
number from 8 to 24, default 12. It sizes every piece of text the card
paints: titles, the query strip, the Ctrl+N digits, and the horizontal
band's caption and tile badges. The 8×8 bitmap floor scales with it
too, so a system with no loadable fonts still obeys the setting.

The geometry that depends on text follows. A taller line grows each
row and the query strip, and the horizontal band's caption row; the
band card's height grows with its caption. The title band keeps its
roughly 600-px physical budget by fitting fewer characters rather than
stretching: a 24-px card shows about half the columns of a 12-px one,
and a long title elides sooner. Icons, paddings, gaps, and weight stay
fixed — this is a text-size knob, not a UI zoom. Because the rows grow,
a larger size fits fewer windows in the 720-px panel and the `rows`
budget clamps exactly as it does at Spacious density.

`font_size` is read when a gesture starts and frozen for the session,
like `font`: an edit applies on the next press, never to the open card.
A value outside 8–24, or a non-integer, is named in a diagnostic and
the last-good settings apply.

## Follows

Three of the card's values follow the COSMIC desktop itself. Two have
no keys because the desktop already owns the choice — flip the setting
in COSMIC Settings and the next Alt+Tab already matches — and the
third is the *default* of its key (`font`), so it follows until you
override it.

- **Corner radius follows Appearance → Style.** Round, Slightly-Round,
  and Square round the card's corners at 12, 8, and 2 logical pixels.
  The selection band rounds with the card, never past 8.
- **Row rhythm follows Appearance → Interface density.** Compact,
  Comfortable, and Spacious set the rows' height, gap, and the card's
  vertical padding — rows stay icon-driven, 40/48/56 px tall by
  density. The card's width never changes with density.
- **The typeface follows the desktop's interface font** (the `font`
  key's default — see [Font](#font)): the family COSMIC Settings'
  own font picker writes, family-only, at the card's default 12 px
  (`font_size` moves the size). An unreadable store keeps Open Sans —
  the desktop's own default — with one diagnostic in the daemon log.

Both are read when a gesture starts, like every setting: a flip
mid-gesture changes nothing until the next press. An unreadable or
missing COSMIC store keeps the factory-default geometry (round
corners, comfortable rhythm — the card Airlock has always painted)
with one diagnostic in the daemon log; switching is never affected.

**The rows clamp.** Every card must fit a 720-logical-pixel panel. At
Spacious the tallest card the `rows` ceiling could build — ten windows
plus the two `…` rows — would not, so the budget clamps to what fits:
`rows = 9` or `10` at Spacious opens with eight windows in view and
one diagnostic naming the density, the text size, the setting, and the
clamp. The setting keeps its value — back at Comfortable the full
budget returns. A larger `font_size` grows the rows and can bind the
same clamp at any density; at 24 px even Spacious's default eight
clamps to six.

## Layout

`layout` picks the card's axis. Two values:

- **`"vertical"` (default)** — today's card: the tall column of rows.
- **`"horizontal"`** — the band: the same MRU window list laid out as
  a single row of compact icon-only tiles in a wide, short card. A
  tile is the window's application icon with its chord's digit as a
  corner badge; an app holding several live windows draws up to three
  count dots under its icon. The card's one caption, fixed top-center,
  names the commit target — the selected window's own title — and
  truncates with an ellipsis when it outgrows the card. A list longer
  than the strip ends in a `…` tile, and stepping past it scrolls
  just enough to keep the selection on a tile. **Left** and
  **Right** become the primary step keys — **Up** and **Down** still
  step, as aliases — and **Ctrl+1..9** addresses tiles in reading
  order, left to right, the digit shown on each of the first nine.
  The pointer drives the same gesture per tile: hover moves the
  selection, a click commits, the wheel steps.

`rows` binds the vertical card only — the band's budget is
[Columns](#columns). A grouped session (`grouping = true`) renders
the grouped card; `layout` still binds it: vertical keeps the tall
tree, horizontal renders the grouped strip (see
[Grouping](#grouping)).

Like every setting, `layout` is read when a gesture starts. An edit
mid-gesture changes nothing until the next press — a session keeps
the axis it opened with until Alt is released.

## Columns

`columns` sizes the horizontal band — `"auto"` (the default) or a
whole number from 3 to 12. The vertical card ignores it.

- **`"auto"`** — the strip holds as many tiles as two-thirds of the
  invoking output's logical width fits, never fewer than three: a
  1920-logical output opens seventeen tiles, a 2560 one twenty-three.
  The share is read from the output the gesture starts on and frozen
  for the session — auto is responsive per monitor, not mid-gesture.
- **`columns = N`** — a pinned budget: exactly N tiles, whatever the
  output. Three is the shortest strip that still reads as one; twelve
  spans 904 logical pixels.

Whatever the budget, the card keeps a caption-width floor: a short
strip renders left-aligned inside a card wide enough that the caption
always reads, so the card never narrows below it.

`columns` is read with `layout` when a gesture starts and frozen for
the session like it — a pinned value live-applies on the next press.

## Width

`width` and `min-width` set a floor under the card's width — with the
same values and the same meaning, on both the vertical card and the
horizontal band:

- **`"auto"`** (the default) — the card sizes from its content: the
  widest title row on the vertical card, the tile shape on the band.
  An unset key is `"auto"`.
- **`"100%"`** — the whole invoking output's logical width. A
  percentage from `"1%"` to `"100%"` is a share of that width.
- **A whole number** — that many logical pixels.

The card grows to the larger of the two keys and its content: a value
below what the card needs changes nothing, so a title never clips. A
pixel count past the invoking output clamps back to the output — the
card stays whole on the screen — and the daemon log names the clamp.

On the horizontal band, `columns` still counts tiles: a widened card
keeps its tiles left-aligned with air on the right, exactly as the
caption floor already does for a short strip.

Both keys are read when a gesture starts and frozen for the session
like `columns` — an edit applies on the next press.

## Grouping

`grouping = true` re-partitions the same MRU window list into a
two-level tree. Level 1 is the app layer: one entry per application —
ordered by each app's most-recently-used window — plus one row per
window COSMIC cannot attribute to any app. An entry is a pure label,
never a window itself, but committing it (Alt-release or Enter)
activates its app's most recently used window — the same window its
first child shows.

Groups start **collapsed**: the card opens showing only the level-1
entries. Expanding is part of entering (→ below), and the expanded or
collapsed state of every app **persists across gestures** for as long
as the daemon runs.

The keys re-bind to the tree:

- **Alt+Tab / Alt+Shift+Tab, ↑/↓, and the mouse wheel** are one and
  the same walk: one *visible* row at a time, linearly, wrapping —
  entries, leaves, and the windows of expanded groups together, group
  boundaries crossed without ceremony. A collapsed group contributes
  only its entry; nothing ever descends into a collapsed group, and a
  workspace holding a single collapsed entry simply keeps the
  selection where it is.
- **→** expands the selected group — the state persists — and enters
  its windows, landing on the most recently used one, one past the
  window you just left when entering your own app. On a leaf it does
  nothing.
- **←** on a child returns to its entry (the group stays expanded);
  on an expanded entry it collapses the group; elsewhere it does
  nothing.
- **Ctrl+1..9** activates the Nth level-1 entry's window instantly.
- **`** (the key above Tab) works the selected app's group in either
  grouped card — the vertical tree and the horizontal strip alike. On
  an app entry or tile it toggles the group: a collapsed entry opens
  that app's windows on their most recently used one and **Shift+`**
  on the last — in the vertical tree the group expands for it, the
  state → shares — and an expanded entry collapses in place, keeping
  the selection. From a window row or tile it cycles the same app's
  windows, ` forward and **Shift+`** backward, wrapping within the app
  and never crossing into another. (The strip replaces an open app's
  tile with its run, so its tiles are always collapsed; there the
  entry press always opens.) Everywhere else the key keeps its filter
  meaning: it types into the query — in the flat card always, and in
  either grouped card while a query stands (a querying session is
  flat).
- Flat mode (`grouping = false`, the default) is unchanged: Tab steps
  rows and ←/→ ride with ↑/↓ as step aliases.

**Grouping on the band** (`layout = "horizontal"`) trades the tree for
**the strip**: one row of tiles where the selected app's tile is
replaced, in place, by its window run — nothing sits below, nothing is
always open. **↓** opens the run on the app's most recently used
window and **↑** closes it back to the app's tile, and the card keeps
one width across the two. An app holds a run only while it has at
least two live windows: **↓** on anything else does nothing, and
committing that tile activates its one window. **Left/Right** step the
visible tiles linearly, wrapping — stepping off a run closes it —
**Tab/Shift+Tab** step app tiles from anywhere, **Ctrl+1..9** address
the visible tiles in reading order, and Alt+minus keeps the parent
rule (an app tile closes its front window). The expansion state above
plays no part here: the strip's shape follows the selection alone.

**The quick tap** — one Tab press with Alt released before the card can
be read — is the first step of the same walk: your own app's next
window when its group is expanded (the flat list's MRU-second window),
the next app entry's window while it is collapsed — skipping your own
app's other windows.

The `rows` budget applies to the visible rows. `grouping` is read when
a gesture starts like every setting: a session keeps the window list
it opened with until Alt is released. A non-boolean value
(`grouping = 1`, `grouping = "yes"`) is named in a diagnostic and the
last-good settings apply.

## Show workspace

`show_workspace = true` labels the card with the workspace you pressed
on: its name, small — used verbatim when it is real words. A workspace
whose name is itself just digits (COSMIC's numeric default) or that has
no name gets the synthesized **"# Workspace N"** — the digits, or the
same 1-based position `index` rules count, so the label and the config
name the same workspace the same way; the `#` mark is the one glyph
every text face paints. The line sits at the top left of a vertical
card and the bottom center of a horizontal one. A long name truncates
with `…`; the label never widens the card.

The label answers one question at a glance — *which workspace is this
card anchored to* — which matters most where `isolated = false` makes
the list cross workspaces. The card's other lines are unchanged: the
caption still names the window Enter would switch to.

The default is `false`: no label, and the card renders exactly as
before. The label is read when a gesture starts like every setting and
freezes with the session. A non-boolean value
(`show_workspace = 1`, `show_workspace = "yes"`) is named in a
diagnostic and the last-good settings apply.

## What `isolated = false` changes

Alt+Tab / Shift+Alt+Tab on that workspace lists every non-minimized
window across all workspaces in MRU order (sticky windows are naturally
included everywhere). Committing a selection parked on another workspace
switches to it and focuses it — the gesture can take you across
workspaces, like the stock switcher. Mid-gesture the rules of the core
still hold: the list is never swapped under a held Alt, and a workspace
switch voids the session.

## Live apply

The daemon checks the file's modification time each time a gesture
starts. Edit the config while the daemon runs and the next Alt+Tab uses
it — no restart, no re-login. The check happens only at gesture start: a
session keeps the scope it opened with until Alt is released.

## Errors and limits

A bad config never breaks switching:

- **Missing or empty file** — every workspace isolated, silently.
- **Malformed file** — the daemon logs a diagnostic naming the offending
  line/column or the 1-based entry number, and keeps using the last-good
  settings (defaults, if none ever parsed). Editing the file again
  re-applies it once valid.
- **Bad entries** — both `name` and `index`, neither, `index = 0` or
  negative, or a wrong type (`name = 3`, `isolated = "yes"`) each name
  their entry number.
- **`[workspace]` with single brackets** is rejected: entries must be
  `[[workspace]]` (an array of tables). One `[workspace]` table cannot
  carry several rules anyway.
- **A bad `theme` value** — a wrong type (`theme = 3`) or an unknown
  string (`theme = "blue"`) — is named in a diagnostic and the
  last-good settings apply, like any malformed edit.
- **A bad `rows` value** — a wrong type (`rows = 8.5`, `rows = "8"`)
  or a number outside 3–10 — is named in a diagnostic ("must be a
  whole number" / "must be between 3 and 10") and the last-good
  settings apply, like any malformed edit.
- **A bad `font` value** — a wrong type (`font = 12`, `[font]`) — is
  named in a diagnostic ("`font` must be a font family name (a
  string)") and the last-good settings apply, like any malformed edit.
  A family name that simply is not installed is *not* a config error:
  the daemon logs that it fell back to the interface font.
- **A bad `font_size` value** — a wrong type (`font_size = 12.5`,
  `font_size = "12"`) or a number outside 8–24 — is named in a
  diagnostic ("must be a whole number" / "must be between 8 and 24")
  and the last-good settings apply, like any malformed edit.
- **A bad `layout` value** — a wrong type (`layout = 1`) or an unknown
  string (`layout = "side"`) — is named in a diagnostic ("`layout`
  must be a string" / "`layout` must be \"vertical\" or
  \"horizontal\"") and the last-good settings apply, like any
  malformed edit.
- **A bad `columns` value** — anything but `"auto"` or a whole number
  3–12 (`columns = "many"`, `columns = 2`, `columns = 12.5`) — is
  named in one diagnostic ("`columns` must be \"auto\" or a whole
  number between 3 and 12") and the last-good settings apply.
- **A bad `width` or `min-width` value** — anything but `"auto"`, a
  percentage 1–100 (`"0%"`, `"101%"`), or a whole pixel count of 1 or
  more (`0`, `-640`, `"640"`, `640.5`) — is named per key ("`width`
  must be \"auto\", a percentage like \"100%\" (1–100), or a pixel
  count (1 at least)") and the last-good settings apply. A pixel
  count past the invoking output is not an error: it clamps to the
  output with a logged note, and the setting keeps its value.
- **A bad `grouping` value** — `grouping = 1`, `grouping = "yes"` —
  is named in a diagnostic ("`grouping` must be a boolean") and the
  last-good settings apply, like any malformed edit.
- **Unknown keys are ignored** — extra keys inside an entry cost nothing.
- **Size cap** — files over 1 MiB are ignored entirely (defaults apply);
  the read runs on the gesture path and must stay cheap.

## Verifying it

- Edit the config while the daemon runs; the next Alt+Tab reflects it
- Rename a matched workspace in COSMIC; the entry follows the name on the
  next press
- On an `isolated = false` workspace, Alt+Tab walks every non-minimized
  window in MRU order, and committing one parked elsewhere switches to
  it
- Set `theme = "light"` on a dark-mode desktop; the next Alt+Tab paints
  the light card (`"dark"` paints the dark card on a light-mode desktop)
- With `theme = "auto"`, flip COSMIC's dark mode; the next Alt+Tab
  already matches
- Set `rows = 10`, then `rows = 3`; the next Alt+Tab keeps that many
  windows in view, `…` rows riding on top — no restart
- Set `layout = "horizontal"`; the next Alt+Tab lays the same windows
  out as a strip of icon-only tiles under the one caption —
  Left/Right step, the first nine tiles show their digit badges, a
  long list ends in a `…` tile
- With the band open, set `columns = 3`, then `columns = 12` — each
  applies on the next press; on `"auto"` the band refits when the
  gesture starts on a different-sized output
- Set `width = "100%"`; the next Alt+Tab's card spans the output's
  logical width, its rows or tiles left-aligned inside it. Set
  `width = 400` on a card that needs more and the card stays as wide
  as its content — the floor never clips. A pixel count past the
  output clamps to it and the daemon log names the clamp
- Set `layout = "vertical"`; the next Alt+Tab is the tall card again,
  unchanged — and `grouping = true` renders the grouped card, vertical
  whatever `layout` says
- Set `grouping = true` with `layout = "horizontal"`: the next Alt+Tab
  is the strip — ↓ opens the selected app's windows in place, ↑ closes
  them, and the card never changes width between the two; Tab steps
  apps, Left/Right step the tiles
- Flip COSMIC's Appearance style (Round / Slightly-Round / Square); the
  next Alt+Tab's corners already match — 12 / 8 / 2 px
- Flip COSMIC's interface density (Compact / Comfortable / Spacious);
  the next Alt+Tab's rows tighten or breathe — 40 / 48 / 56 px — and
  the card's width stays put
- At Spacious density, set `rows = 10`; the card opens clamped to eight
  windows and the daemon log names the clamp — the setting keeps its
  value
- Set `font = "<an installed family>"`; the next Alt+Tab paints it —
  no restart. Edit it under held Alt: the open card does not repaint
  until the next press
- Set `font_size = 16` (or 8, or 24); the next Alt+Tab paints larger or
  smaller text, the rows and the query strip grow, the title band fits
  fewer columns instead of stretching wider, and a long title elides
  sooner. At 24 px on a Spacious desktop the default `rows = 8` clamps
  to six and the daemon log names the clamp
- Set `font_size = 30`; the daemon log names the range and the
  last-good settings apply
- Unset `font`: the next Alt+Tab paints the desktop's interface font —
  flip COSMIC Settings' own font and the next press follows
- Open a window with a CJK or emoji title: the glyphs render for real
- Set `font = "Not Installed"`; the daemon log names the key and the
  card paints the interface font
- With `grouping = true`: expand a group (→), release Alt, press
  Alt+Tab again — the step continues the linear walk from where it
  left off, through the expanded group's windows and on to the next
  app; Tab and ↓ land on the expanded entry like any row. A config
  still carrying the removed `scoped_cycling` key parses fine —
  unknown keys are ignored
