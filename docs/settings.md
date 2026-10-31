# Settings

By default, Airlock seals Alt+Tab to the current workspace. A config file
can opt individual workspaces out: there the switcher lists every
non-minimized window across **all** workspaces, most-recently-used first,
while workspaces without an entry stay isolated exactly as before.

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
- **Unknown keys are ignored** — extra keys inside an entry cost nothing.
- **Size cap** — files over 1 MiB are ignored entirely (defaults apply);
  the read runs on the gesture path and must stay cheap.

## Verifying it

- Edit the config while the daemon runs; the next Alt+Tab reflects it
- Rename a matched workspace in COSMIC; the entry follows the name on
  the next press
- On an `isolated = false` workspace, Alt+Tab walks every non-minimized
  window in MRU order, and committing one parked elsewhere switches to
  it
