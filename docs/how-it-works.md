# How the switcher behaves

The full-fidelity description of the overlay and its gestures — the
README keeps the short version. Per-workspace configuration lives in
[settings.md](settings.md); the release-by-release verification of this
behavior is the [hardware checklist](hardware-checklist.md).

## The keyboard gesture

Hold **Alt** and tap **Tab**: an overlay lists the windows of the current
workspace in most-recently-used order. **Tab** walks forward, **Shift+Tab**
walks back, and the arrow keys step too — **Up** or **Left** back,
**Down** or **Right** forward. Releasing **Alt** focuses the selection and
dismisses the overlay; **Enter** confirms the same outcome without letting
go of **Alt**. **Escape** dismisses without switching. A quick tap —
released before the overlay even appears — still switches.

## The pointer gesture

The mouse drives the same gesture while the overlay is up. Crossing
between rows moves the selection with the cursor; wherever the pointer
first lands — including resting right where the card appears — claims
nothing. **Clicking** a row focuses that window and dismisses the overlay
— the same outcome as releasing **Alt**. The **scroll wheel** steps the
list one row per notch, forward or back, wrapping like Tab, and a slow
scan never times the overlay out.

## The frozen list

The list is frozen for the gesture: a window that opened after the
overlay appeared is not on it, so neither the pointer nor the wheel can
reach it mid-gesture.

## The card

Each row shows the window's application icon — resolved through the same
desktop-entry and icon-theme pipeline the COSMIC launcher uses — next to
its title; an application with no themed icon falls back to a generic
window glyph. The card is a rounded rectangle with a hairline border, at
most 660 px wide, with the selected row highlighted; text renders at
8 px and stays sharp on HiDPI displays (the overlay renders at the
output's scale). The `theme` setting picks the card — `auto` (the
default) follows the desktop's dark/light preference; see
[settings.md](settings.md#theme).

## Workspace scoping

Only windows whose workspace set includes the current workspace are
listed: sticky windows appear everywhere, minimized windows nowhere, and
a window parked on another workspace can never be focused through
Airlock. The isolation is per-workspace configurable: a workspace can
opt into the global switcher through a config file — see
[settings.md](settings.md).

## The extra keys

`airlock install` also claims three more keys. Hold **Alt** and tap
**grave** (the key above Tab): the overlay lists only the current
application's windows, MRU order — the same gesture, the same isolation
rule (`isolated = false` widens the list to that application's windows
everywhere). **Alt+Shift+grave** opens it stepping backwards. An
application with fewer than two windows in scope does nothing — there is
nothing to cycle. **Alt+Esc** switches to the MRU-second window under the
same scope — the window a quick Alt+Tab tap would reach — with no overlay
at all.

## The daemon

A small daemon keeps a live snapshot of the compositor state; the keybind
stubs are milliseconds-thin and auto-start the daemon on first press. The
overlay is software-rendered — no GPU required.
