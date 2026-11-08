# Settings

By default, Airlock seals Alt+Tab to the current workspace. A config file
can opt individual workspaces out: there the switcher lists every
non-minimized window across **all** workspaces, most-recently-used first,
while workspaces without an entry stay isolated exactly as before. The
same file also picks the switcher's card color — light, dark, or
following the desktop — its typeface, and how many windows the card
keeps in view.

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
rows = 8               # windows the card keeps in view (the default)
layout = "vertical"    # the tall card; "horizontal" lays it out as a band of tiles
# columns = "auto"     # the band's tile budget; or a whole number 3–12 (the default: "auto")
grouping = false       # the flat list; true groups it by app (the default)

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
[Theme](#theme)); `font` picks its typeface (see [Font](#font));
`rows` picks how many windows it keeps in view (see [Rows](#rows));
`layout` picks the card's axis and `columns` the horizontal band's
width (see [Layout](#layout)); `grouping` picks the list's shape (see
[Grouping](#grouping)).

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
slots). At the default that is 576 logical pixels tall — fine on any
100%-scaled display and on a 1440p screen at 200%; a 1080p panel at
200% (a 540-logical screen) is the one configuration that wants less —
`rows = 7` keeps the card inside that height.

Like every setting, `rows` is read when a gesture starts. An edit
mid-gesture changes nothing until the next press; a session keeps the
budget it opened with until Alt is released. A value outside 3–10 (or
a non-integer) is named in a diagnostic and the last-good settings
apply. At Spacious density the effective budget clamps to what fits —
see [Follows](#follows).

## Font

`font` picks the typeface the card's titles paint in — a font family
name, exactly as your font picker shows it. Family only: size stays
the card's own 12 px and weight stays Regular, whatever is set.

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
  own font picker writes, family-only, at the card's pinned 12 px. An
  unreadable store keeps Open Sans — the desktop's own default — with
  one diagnostic in the daemon log.

Both are read when a gesture starts, like every setting: a flip
mid-gesture changes nothing until the next press. An unreadable or
missing COSMIC store keeps the factory-default geometry (round
corners, comfortable rhythm — the card Airlock has always painted)
with one diagnostic in the daemon log; switching is never affected.

**The rows clamp.** Every card must fit a 720-logical-pixel panel. At
Spacious the tallest card the `rows` ceiling could build — ten windows
plus the two `…` rows — would not, so the budget clamps to what fits:
`rows = 9` or `10` at Spacious opens with eight windows in view and
one diagnostic naming the density, the setting, and the clamp. The
setting keeps its value — back at Comfortable the full budget returns.

## Layout

`layout` picks the card's axis. Two values:

- **`"vertical"` (default)** — today's card: the tall column of rows.
- **`"horizontal"`** — the band: the same MRU window list laid out as
  a single row of tiles in a wide, short card. A tile is the window's
  application icon with its title beneath, clamped to the tile's
  width; a list longer than the band ends in a `…` tile, and stepping
  past the band scrolls it just enough to keep the selection on a
  tile. **Left** and **Right** become the primary step keys — **Up**
  and **Down** still step, as aliases — and **Ctrl+1..9** addresses
  tiles in reading order, left to right, the chord shown as a corner
  badge on each of the first nine tiles. The pointer drives the same
  gesture per tile: hover moves the selection, a click commits, the
  wheel steps.

`rows` binds the vertical card only — the band's budget is
[Columns](#columns). A grouped session (`grouping = true`) renders
the grouped card, which is vertical; `layout` binds the flat list.

Like every setting, `layout` is read when a gesture starts. An edit
mid-gesture changes nothing until the next press — a session keeps
the axis it opened with until Alt is released.

## Columns

`columns` sizes the horizontal band — `"auto"` (the default) or a
whole number from 3 to 12. The vertical card ignores it.

- **`"auto"`** — the band holds as many tiles as two-thirds of the
  invoking output's logical width fits, never fewer than three: a
  1920-logical output opens eight tiles, a 2560 one ten. The share is
  read from the output the gesture starts on and frozen for the
  session — auto is responsive per monitor, not mid-gesture.
- **`columns = N`** — a pinned budget: exactly N tiles, whatever the
  output. Three is the shortest band that still reads as a strip;
  twelve spans 1960 logical pixels — past the full logical width of a
  4-K panel at 200 %.

`columns` is read with `layout` when a gesture starts and frozen for
the session like it — a pinned value live-applies on the next press.

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
- Flat mode (`grouping = false`, the default) is unchanged: Tab steps
  rows and ←/→ ride with ↑/↓ as step aliases.

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
- **A bad `layout` value** — a wrong type (`layout = 1`) or an unknown
  string (`layout = "side"`) — is named in a diagnostic ("`layout`
  must be a string" / "`layout` must be \"vertical\" or
  \"horizontal\"") and the last-good settings apply, like any
  malformed edit.
- **A bad `columns` value** — anything but `"auto"` or a whole number
  3–12 (`columns = "many"`, `columns = 2`, `columns = 12.5`) — is
  named in one diagnostic ("`columns` must be \"auto\" or a whole
  number between 3 and 12") and the last-good settings apply.
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
  out as a band of tiles — Left/Right step, the first nine tiles show
  their Ctrl+N corner badges, a long list ends in a `…` tile
- With the band open, set `columns = 3`, then `columns = 12` — each
  applies on the next press; on `"auto"` the band refits when the
  gesture starts on a different-sized output
- Set `layout = "vertical"`; the next Alt+Tab is the tall card again,
  unchanged — and `grouping = true` renders the grouped card, vertical
  whatever `layout` says
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
