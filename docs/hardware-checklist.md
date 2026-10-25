# Hardware verification checklist

Airlock is a COSMIC Wayland app: runtime behavior can't be exercised
headless. This is the checklist a human runs on a real COSMIC session —
after meaningful changes, and always before calling a build release-ready.

Setup: a COSMIC session with windows on **at least two workspaces**, one of
them sticky, and at least one minimized. Build and install first
(`just install`), so the presses below exercise the installed binary.

## 1. State layer

- [ ] `airlock list` prints every toplevel with title, app id, state flags,
      and workspace set, and names the active workspace
- [ ] Output matches what the dock/overview shows for the same windows

## 2. Terminal cycling and isolation

- [ ] `airlock alt-tab` focuses the next window of the **current** workspace
      in most-recently-used order; `airlock shift-alt-tab` reverses
- [ ] A window parked on another workspace is never focused, no matter how
      many presses
- [ ] The sticky window appears in the list on every workspace; minimized
      windows never do

## 3. Gesture (with the takeover installed)

- [ ] Hold Alt, tap Tab repeatedly — selection walks the list in MRU order;
      Shift+Tab reverses
- [ ] Releasing Alt focuses the selected window and dismisses the overlay
- [ ] Escape dismisses without switching
- [ ] A quick Alt+Tab tap (released before the overlay appears) still
      switches
- [ ] Two rapid invokes in a row behave as one continuous gesture session

## 4. Takeover and restore

- [ ] `airlock install` redirects Alt+Tab / Shift+Alt+Tab to Airlock at user
      level; the override survives re-login; `/usr/share` is untouched
- [ ] `airlock install --uninstall` brings the previous switcher back
      exactly (diff `~/.config/cosmic/com.system76.CosmicSettings.Shortcuts/v1/system_actions`
      before install and after uninstall: byte-identical)

## 5. Live-state edge cases

- [ ] Close the selected window while the overlay is up — commit is a no-op,
      the daemon survives
- [ ] Move a listed window to another workspace mid-gesture — the gesture
      dismisses (the list is never silently swapped under a held Alt)
- [ ] Toggle a window sticky while listed — the next invoke reflects it
- [ ] Switch workspaces mid-gesture — dismisses; the next press starts fresh
      against the new workspace
- [ ] Invoke on an empty workspace — the overlay shows nothing, commit is a
      no-op
- [ ] Restart the compositor (or log out/in) — the daemon exits loudly; the
      next press auto-spawns a fresh one and cycling works again

## 6. Soak

- [ ] 10+ rapid presses in a row: no panics, no stuck keyboard grab, the
      overlay never lingers past the gesture

## 7. Performance

Measure and compare against the targets:

| Measurement | How | Target | Reference (2026-10-25) |
|---|---|---|---|
| Daemon RSS / threads | `ps -o rss=,nlwp= -C airlock` | < 30 MB | 4.0 MB / 1 thread |
| Warm press round-trip | `time airlock alt-tab` (daemon running) | < 50 ms | 3–4 ms |
| Cold press (respawn + settle + switch) | `time airlock alt-tab` (daemon stopped) | — | ~300 ms |

For scale: the switcher path this replaces (cosmic-launcher's
cosmic-switch-less) measured ≈ 170 MB RSS.

## Latest pass

2026-10-25 — full pass (sections 1–6) by the developer on the live session,
plus the reference measurements above with the overlay-polish build.
