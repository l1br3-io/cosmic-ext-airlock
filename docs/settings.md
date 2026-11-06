# Settings

By default, Airlock seals Alt+Tab to the current workspace. A config file
can opt individual workspaces out: there the switcher lists every
non-minimized window across **all** workspaces, most-recently-used first,
while workspaces without an entry stay isolated exactly as before. The
same file also picks the switcher's card color — light, dark, or
following the desktop — and how many windows the card keeps in view.

Airlock reads the file; you write it. Airlock never creates, renames, or
rearranges workspaces — names and positions come from COSMIC itself.

## The file

```
~/.config/airlock/config.toml
```

More precisely: `airlock/config.toml` under an absolute `$XDG_CONFIG_HOME`
when one is set, else under `$HOME/.config` — the same rule the installer
uses. No file (or an empty one) means every workspace isolated: the
default behavior, nothing to configure.

## Example

```toml
# ~/.config/airlock/config.toml — Airlock reads, you write.
# Workspaces without an entry keep the default: isolated.

theme = "auto"         # match the desktop's dark/light mode (the default)
rows = 8                # windows the card keeps in view (the default)

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

The two top-level keys: `theme` picks the card (see
[Theme](#theme)); `rows` picks how many windows it keeps in view (see
[Rows](#rows)).

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
apply.

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
