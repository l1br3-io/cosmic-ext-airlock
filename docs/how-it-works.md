# How the switcher behaves

The full-fidelity description of the overlay and its gestures: the
README keeps the short version. Per-workspace configuration lives in
[settings.md](settings.md); the release-by-release verification of this
behavior is the [hardware checklist](hardware-checklist.md).

## The keyboard gesture

Hold **Alt** and tap **Tab**: an overlay lists the windows of the
current workspace in most-recently-used (MRU) order.

- **Tab** walks forward, **Shift+Tab** walks back. The arrow keys step
  too: **Up** or **Left** back, **Down** or **Right** forward.
- Releasing **Alt** focuses the selection and dismisses the overlay.
- **Enter** confirms the selection without letting go of **Alt**.
- **Escape** dismisses without switching.
- A tap released before the overlay appears still switches.
- **Ctrl+1** through **Ctrl+9**, keypad digits included, switch to
  that row's window. The first nine rows show their key at the right
  edge; deeper rows show none. Without **Ctrl**, the number row does
  nothing.

## The pointer gesture

The mouse drives the same gesture while the overlay is up. Crossing
between rows, or tiles on the horizontal card, moves the selection
with the cursor. Wherever the pointer rests when the overlay appears,
including where the card itself appears, it selects nothing.

**Clicking** a row focuses that window and dismisses the overlay: the
same outcome as releasing **Alt**. On the column card, resting the
pointer on a window row reveals an **×** at its right edge; clicking
that closes the window instead (see
[Closing windows](#closing-windows)). The **scroll wheel** steps the
list one row per notch, forward or back, wrapping like **Tab**. A slow
scan never times the overlay out.

## The frozen list

The list is frozen for the gesture: a window that opened after the
overlay appeared is not on it. Neither the pointer, the wheel, nor a
number key can reach it mid-gesture.

## Grouped mode

The `grouping = true` setting re-partitions the same list into a
two-level tree: one entry per application, ordered by each app's most
recently used window, with that app's whole window set nested beneath
it. A top-level row stands for each window COSMIC cannot attribute to
any app, and for each single-window app. Groups start collapsed;
expanding one with **→** is remembered for as long as the daemon runs.
Committing an entry activates that app's most recently used window.

The keys re-bind to the tree, one walk:

- **Tab**, **↑/↓**, and the **wheel** are the same linear step: one
  visible row at a time, wrapping, through entries, leaves, and the
  windows of expanded groups alike.
- A collapsed group contributes only its entry.
- **→** expands and enters, landing one past the window you just left
  when it is your own app.
- **←** on a child returns to its entry, and the group stays expanded;
  on an expanded entry it collapses the group.
- **Ctrl+N** activates the Nth app entry's window.

The pointer picks rows like the keyboard does, expanded entries
included. The frozen-list rule holds for the tree: the window set
never changes mid-gesture, only what stands expanded. The full table,
including how a fast tap lands, lives in
[settings.md](settings.md#grouping).

## Type-to-filter

While the card is up, typing filters it. Printable keys type straight
through the held **Alt**: the query collects in a strip above the
rows, and each keystroke narrows the card to the windows whose title
or application name contains it, case-insensitive. The selection
lands on the first match.

- **Tab**, the arrows, and the wheel step the matches.
- **Enter** or releasing **Alt** switches to the selected one.
- **Backspace** erases the last character; on an empty query it does
  nothing.
- **Escape** clears a standing query first. Only an empty query
  dismisses.

It works in every mode: the flat list, the grouped tree, which
flattens to its matching windows while a query stands, and the
same-app view. **Ctrl+N** keeps its chord; every other printable key,
digits included, types. One carve-out: **grave** in the grouped strip
(grouping on the horizontal layout) steps the selected app's windows
instead, and types only while a query stands. See
[Grouping](settings.md#grouping).

Clearing the query restores the card you had, keeping the window you
had selected when it is still there. When nothing matches, the card
keeps the query with a one-word verdict and commits as a no-op. The
query dies with the gesture: nothing persists, and the window list
never re-derives mid-gesture.

## Closing windows

Windows close from the card without leaving the gesture.

- **Alt+minus**, the top-row minus or the keypad's, closes the
  selected row's window.
- Clicking the **×** on a window row closes that row's window instead
  of activating it.

Every row closes exactly what it activates: a group entry closes its
app's most recently used window, a query row closes the window it
shows.

After each close the selection moves to the next visible row,
wrapping, if it rested on the closing window. The card stays up, so
one held gesture can prune a stack of windows: Alt+minus, Alt+minus,
then Tab to what survives. Closing the last eligible window dismisses
the overlay.

The × is the column card's affordance: window rows only, never group
entries or the `…` rows. Alt+minus works in every mode, and through a
standing query. A window someone else closes mid-gesture drops off
the card the same way, and closing never adds one back: the
frozen-list rule holds.

## The card

Each row shows the window's application icon next to its title,
resolved through the same desktop-entry and icon-theme pipeline the
COSMIC launcher uses. An application with no themed icon falls back to
a generic window glyph. The card is a rounded rectangle with a
hairline border, at most 748 px wide, with the selected row
highlighted. Text renders at 12 px and stays sharp on HiDPI displays:
the overlay renders at the output's scale. The `theme` setting picks
the card; `auto`, the default, follows the desktop's dark/light
preference. See [settings.md](settings.md#theme).

The `layout = "horizontal"` setting trades the column for a wide,
short **band**: the same windows as compact, icon-only tiles, each
tile the app icon with its chord's digit, under one caption that
names the selected window, ended by a `…` tile when the list
outgrows the strip.

- **Left** and **Right** step the strip.
- **Ctrl+N** and the pointer address its tiles exactly as they
  address rows.

The `columns` setting sizes it: two-thirds of the invoking output
(`"auto"`, the default) or a pinned 3 to 12 tiles. See
[settings.md](settings.md#layout).

## Workspace scoping

Only windows whose workspace set includes the current workspace are
listed: sticky windows appear everywhere, minimized windows nowhere,
and a window parked on another workspace can never be focused through
Airlock. The isolation is per-workspace configurable: a workspace can
opt into the global switcher through a config file. See
[settings.md](settings.md).

## The extra keys

`airlock install` also claims three more keys.

- Hold **Alt** and tap **grave** (the key above **Tab**): the overlay
  lists only the current application's windows in MRU order. The same
  gesture, the same isolation rule; `isolated = false` widens the list
  to that application's windows everywhere. An application with fewer
  than two windows in scope does nothing.
- **Alt+Shift+grave** opens it stepping backwards.
- **Alt+Esc** switches to the MRU-second window under the same scope:
  the window a fast Alt+Tab tap reaches, with no overlay.

## The daemon

A small daemon keeps a live snapshot of the compositor state; the
keybind stubs are milliseconds-thin and auto-start the daemon on first
press. The overlay is software-rendered: no GPU required.
