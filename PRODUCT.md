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
  pipeline (SVG-first), 16-px text, ≤600-px card, exact-AA corners,
  HiDPI buffer scale; previewable headless via
  `cargo run --example overlay-preview -- out.png`.
- Constraints, durable: RSS < 30 MB and warm press < 50 ms are product
  budgets, not targets; no libcosmic/iced dependency; never write
  outside the user's config and data dirs (`/usr/share` untouched);
  GPL-3.0-only; near-opaque card (reads identically on light and dark
  wallpapers); no fade-in or animated selection (the gesture is the
  animation).
- Product commitments (owner-confirmed 2026-11-01): keyboard-only is
  final (pointer interaction deliberately out of scope, matching the
  platform consensus); non-ASCII titles rendering as `?` is accepted,
  documented degradation (a real font would be a deliberate future
  decision); glance legibility on both wallpaper polarities is a floor,
  not a preference.
- Open decisions: app ID / GitHub username for
  `io.github.<username>.Airlock` — the pre-existing packaging blocker,
  now near-term because the community release is first; crates.io /
  Flathub publishing; icon + `.desktop` install loop waits on it.

## Brand Commitments

- Name **Airlock**, package/repo/crate `cosmic-ext-airlock`, tagline
  "Nothing crosses workspaces." (naming research record, locked
  2026-10-22; runner-up Asterism reserved as the zero-collision fallback).
- COSMIC trademark rules are binding: `cosmic-ext-` prefix, app ID
  outside `com.system76.`, wording "for the COSMIC™ desktop", explicit
  not-affiliated statement (research record
  `naming-trademark-clearance.md`).
- Own icon direction: a circular airlock hatch
  (`res/icons/hicolor/scalable/apps/airlock.svg`), monochrome with a
  single orange hub accent.

## Evidence on Hand

- Primary-source research records in `docs/research/`: workspace
  isolation feasibility, naming/trademark clearance, VM environment,
  switcher behavior adaptation, switcher UX + icon pipeline (all
  complete; the records are never edited after the fact).
- Measured performance (2026-10-25, `docs/hardware-checklist.md`):
  4.0 MB RSS / 1 thread, 3–4 ms warm press, ~300 ms cold press.
- Design preview sheet via the `overlay-preview` example — the honest
  stand-in for visual review off hardware.
- Absences future work must not fabricate: no screenshots of the new
  overlay on real hardware yet (CP-OV VM pass pending); no published
  package, no user counts, no testimonials.

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

- Keyboard-only by design (owner-confirmed commitment): the overlay is
  operated entirely by keys, and pointer pass-through matches the
  platform's own switchers.
- Glance legibility floor: both wallpaper polarities, 16-px text,
  icon-shape recognition (airlock-design skill, pinned).
- Non-ASCII titles render as `?` — accepted degradation, documented in
  README and the design skill; revisiting it is an explicit product
  decision, not a bug fix.
