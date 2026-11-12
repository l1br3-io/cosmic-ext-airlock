# Product

<!-- impeccable:product-schema 1 -->

## Platform

desktop-linux (not one of the schema's four values: Airlock is a native
Wayland overlay for the COSMIC desktop, so neither the web tooling nor the
iOS/Android guidance applies; the visual surface is a software-rendered
layer-shell card)

## Users

A Pop!_OS / COSMIC power user mid-gesture: holding Alt, scanning a short
most-recently-used list for the window they just left, in peripheral
vision, on the workspace they are already on. Owned by jpt and used as a
daily driver; per the owner's 2026-11-01 decision, built for the COSMIC
community as a released third-party extension from the start — not a
personal-only tool.

## Product Purpose

Airlock is a workspace-isolated Alt+Tab switcher for the COSMIC desktop:
the overlay lists and cycles only the windows of the current workspace
(sticky windows appear everywhere, minimized windows nowhere, other
workspaces' windows never), so nothing parked elsewhere leaks into focus.
Success means the tool disappears: the user switches windows without ever
noticing the switcher — sub-50 ms warm presses, zero lingering overlay,
no panics, no stuck keyboard grabs.

## Positioning

The only switcher surface an external Wayland client can honestly build
is icon + title (thumbnails are protocol-impossible; research record
`switcher-ux-and-icons.md` §2.9) — and Airlock does it as a 4 MB RSS,
single-thread daemon against cosmic-launcher's ≈170 MB alt-tab process,
  at 3–4 ms warm round-trips, with per-workspace isolation and a
  same-application cycling gesture the platform default does not offer.
  The icon+title ceiling stands through every presentation — the flat
  card, the horizontal tile band, and the grouped tree are all
  icon-led; no real thumbnails anywhere.

## Operating Context

- A COSMIC (epoch) Wayland session; other compositors out of scope for v1.
- User-level keybind takeover (`airlock install`) of
  `WindowSwitcher`/`WindowSwitcherPrevious` plus three custom keys
  (Alt+grave family, Alt+Esc); byte-exact uninstall via in-file backups.
- Daemon + millisecond stubs: first key press auto-spawns the daemon;
  IPC over a single-instance socket.
- Runtime behavior is only testable on real COSMIC hardware or the
  COSMIC VM (`just vm-*`); the app is never run on the dev host
  (AGENTS.md). Headless gates: `just test / check / fmt-check`.
- Development docs: README, `docs/` (settings, hardware checklist, VM
  guide, research records), local spec/task artifacts (gitignored).

## Capabilities and Constraints

- Gesture vocabulary: Tab / Shift+Tab / arrows step, Alt-release or
  Enter commits, Escape dismisses, quick-tap still switches, focus loss
  dismisses; Alt+grave cycles the current application's windows,
  Alt+Esc quick-toggles to the MRU-second window with no overlay.
- Per-workspace isolation is configurable (`isolated = false` widens a
  workspace to global scope).
- Overlay visuals (2026-11-01): themed app icons via the pop-launcher
  pipeline (SVG-first), 16-px text, ≤660-px card (measured; widened by
  the Ctrl+N hint band), exact-AA corners, HiDPI buffer scale;
  previewable headless via `cargo run --example overlay-preview --
  out.png`. Re-decided 2026-11-07 (`overlay-density`): **12-px text,
  ≤688-px card** (measured: 48 title columns at 12 px plus the
  single-digit hint band). The 16-px figure above never shipped — the
  owner re-pinned
  8 px on 2026-11-02 judging on the 200 % VM (16 physical px), then
  found the same value tiny on a 100 % output (8 physical) and moved to
  the middle step. The card keeps `rows` windows in view on a long
  list (default 8, configurable 3–10), the `…` rows riding on top of
  that budget. Re-decided 2026-11-08 (`geometry`): **the 12-px corner
  radius and the 8-grid rhythm are derived values now** — the radius
  follows the COSMIC style choice (Round / Slightly-Round / Square →
  12 / 8 / 2 px, band capped at 8), the rhythm the interface density
  (Compact / Comfortable / Spacious → 40 / 48 / 56-px icon-driven
  rows; Compact's 4-px steps are COSMIC's own `space_xxs`, the one
  deliberate half-grid exception). The card's height varies with the
  desktop's density by design; its width never moves. At Spacious the
  `rows` budget auto-clamps to the 720-px fit with one diagnostic —
  the fit is guaranteed by construction. Follow-only by decision: no
  keys exist, the desktop is the config. Re-decided 2026-11-09
  (`width`, `min-width`): **the card is content-sized** — those keys
  set optional floors (`"auto"` keeps content sizing as the default,
  `"100%"` spans the invoking output, a % or px count sets a floor),
  superseding the same day's pin that the width never moves.
- Card presentations (2026-11-09): the flat vertical MRU card is the
  default; `layout = "horizontal"` renders the same MRU list as an
  icon+title tile band (`columns` sizes it — `"auto"` fits
  two-thirds of the output width or 3–12 pinned; Ctrl+1..9, hover,
  click, and the wheel address tiles exactly as they address rows);
  `grouping = true` re-partitions the list into a two-level
  app/window tree (→ expands and enters, ← collapses and exits, Tab
  walks the visible rows, ` steps the selected app's windows —
  0.4.1: from inside a run Tab keeps the run open, and ` on an open
  group collapses it in place); `show_workspace = true` labels the
  card with the workspace. Custom card+text colors via the `Theme`
  keys, contrast-guarded (WCAG 2.2 AA 4.5:1 for titles; the 2026-11-04
  no-accent hard line stands). Pointer extras: each row/tile carries a
  hover-revealed × close badge (0.4.2: 16-px ink, 20-px click
  territory) that closes the window from the card, and the cursor
  turns to a pointing hand while over the card (needs the compositor's
  cursor-shape protocol; without it the cursor simply stays).
- Constraints, durable: RSS < 30 MB and warm press < 50 ms are product
  budgets, not targets; no libcosmic/iced dependency; never write
  outside the user's config and data dirs (`/usr/share` untouched);
  GPL-3.0-only; near-opaque card (reads identically on light and dark
  wallpapers); no fade-in or animated selection (the gesture is the
  animation).
- Product commitments (owner-confirmed 2026-11-01): keyboard-only is
  final (pointer interaction deliberately out of scope, matching the
  platform consensus) — superseded 2026-11-05, retroactive to the
  2026-11-03 `pointer-select` approval: pointer interaction is in
  scope and shipped (hover moves the selection, click activates the
  row, the wheel steps the list); non-ASCII titles rendering as `?` is
  accepted, documented degradation (a real font would be a deliberate
  future decision) — superseded 2026-11-07, retroactive to that day's
  owner note ("we should be able to update the font style through
  settings"): titles render in the desktop's interface font by default
  (a `font` key overrides the family; size stays the pinned 12 px),
  Unicode renders true glyphs, and the ASCII fold is the zero-fonts
  bitmap floor only; glance legibility on both wallpaper polarities is a floor,
  not a preference. Superseded 2026-11-09: `font_size` makes the text
  size configurable — 8–24 logical px, default 12; titles, the query
  strip, hint digits, and captions follow, rows grow and clamp to the
  720-px fit as at Spacious, and the 8×8 bitmap floor scales too.
- Decided (2026-11-02): app ID **`io.github.l1br3-io.Airlock`** (the
  real GitHub org), crate `cosmic-ext-airlock` owned personally on
  crates.io, v0.1.0 channels GitHub Release + crates.io (Flathub out of
  scope — a sandboxed keybind daemon fights the Flatpak model).
  Superseded 2026-11-08 on Flathub only
  (`docs/research/flatpak-publishing.md`): the submission path is
  unremarkable and the README roadmap carries it without dates, but the
  blocker is keybind capture in the sandbox — no GlobalShortcuts
  backend in xdg-desktop-portal-cosmic, and a `<10 ms` spawn path that
  is unmeasured; a one-hour VM latency experiment decides submission.
  The app ID's middle-component dash violates the Flathub naming rule
  (open owner decision). Channels realized and live: **crates.io
  (0.4.2 published 2026-11-11, the newest)** with GitHub Release
  artifacts and cargo-binstall prebuilt binaries.

## Brand Commitments

- Name **Airlock**, package/repo/crate `cosmic-ext-airlock`, tagline
  "Nothing crosses workspaces." (naming research record, locked
  2026-10-22; runner-up Asterism reserved as the zero-collision fallback).
- COSMIC trademark rules are binding: `cosmic-ext-` prefix, app ID
  outside `com.system76.`, wording "for the COSMIC™ desktop", explicit
  not-affiliated statement (research record
  `naming-trademark-clearance.md`).
- Own icon direction: a circular airlock hatch
  (`res/icons/hicolor/scalable/apps/io.github.l1br3-io.Airlock.svg`),
  monochrome with a single orange hub accent.

## Evidence on Hand

- Primary-source research records in `docs/research/`, all complete
  and never edited after the fact: workspace isolation feasibility,
  naming/trademark clearance, VM environment, switcher behavior
  adaptation, switcher UX + icon pipeline — then five more closed
  2026-11-07 through 2026-11-09: app grouping as treeview (owner
  superseded its keep-vertical verdict; the horizontal band shipped as
  the configurable layout), appearance settings, COSMIC appearance
  stores, Flathub publishing, grouped-window presentation.
- Published release: live on crates.io through 0.4.2 (2026-11-11),
  with GitHub Release artifacts and cargo-binstall prebuilt binaries.
  Prebuilt-binary and system-build-prerequisite install paths are
  documented in README.
- Measured performance (2026-10-25, `docs/hardware-checklist.md`):
  4.0 MB RSS / 1 thread, 3–4 ms warm press, ~300 ms cold press.
- Design preview sheet via the `overlay-preview` example — the honest
  stand-in for visual review off hardware.
- Absences future work must not fabricate: no screenshots of the new
  overlay on real hardware yet (CP-OV VM pass pending); no user
  counts, no testimonials.

## Product Principles

1. The gesture outranks the visual: every millisecond of mapped time and
   every pixel of the overlay serves a 200-ms glance, then gets out.
2. Isolation is the feature — the switcher never crosses workspaces,
   and neither does its state.
3. Degrade honestly: iconless apps show the generic glyph, non-ASCII
   titles show `?`, and the docs say so; no fake polish.
4. Stay small enough to trust: a single-thread, low-MB daemon that a
   user can read end to end and that never sticks a keyboard grab.
5. Third-party in fact and in law: community-first release, honest
   trademark posture, byte-exact install/uninstall.

## Accessibility & Inclusion

- Keyboard operation is complete, and pointer input is equivalent:
  crossing rows moves the selection, a click or Ctrl+N activates the
  row (the same outcome as releasing Alt), the wheel steps the list
  (re-decided 2026-11-05, retroactive to the 2026-11-03
  `pointer-select` approval; the 2026-11-01 commitment read
  "keyboard-only, with pointer pass-through").
- Glance legibility floor: both wallpaper polarities, 12-px text
  (re-pinned 2026-11-07; 16 px was the 2026-11-01 figure, 8 px the
  2026-11-02 one), icon-shape recognition (airlock-design skill,
  pinned).
- Non-ASCII titles render as `?` — accepted degradation, documented in
  README and the design skill; revisiting it is an explicit product
  decision, not a bug fix.
