---
name: airlock-design
description: Design direction and quality bar for Airlock's COSMIC Alt+Tab overlay and every user-facing surface around it — overlay colors, spacing, glyphs, and glance legibility, gesture-time presentation, and the copy in CLI output, config diagnostics, and docs. Use whenever touching src/ui.rs render constants or layout math, changing what the overlay shows or how it reads, writing diagnostics or user-facing docs, or reviewing any of these — even for small tweaks.
---

<!-- Adapted from Pomotoro's `pomotoro-design` skill (itself adapted from
     Anthropic's `frontend-design` agent skill, Apache-2.0). The carried-over
     methodology: ground in the subject, calibrate against generic output,
     plan tokens before building, spend boldness in one place, treat copy as
     design. The mechanics differ: not a themed iced window but a
     software-rendered, hand-composited Wayland overlay that exists only
     mid-gesture. -->

# Airlock Design

Approach this as the design lead for a surface most users will never
consciously see. The overlay competes with muscle memory: it appears for a
couple of seconds during a keyboard gesture and must be legible before the
second Tab press. Make deliberate, opinionated choices about the card, the
list, and the words around them — and treat restraint as the house style,
because every pixel here is hand-placed.

For build mechanics, the source of truth is `src/ui.rs` itself: a pure
render model (plain data in) and a software renderer (bytes out),
presented by a layer-shell surface. This skill decides *what the overlay
and its copy should be*; the module docs and tests cover *how it draws*.

## Ground the design in the subject

Airlock's subject is **a fraction of a second of recognition**. Someone
mid-switch is scanning a short most-recently-used list for the window they
just left — recognizing, not reading, often in peripheral vision, centered
over their work. The overlay's whole job: make the next window findable in
one glance and get out of the way.

Every visual choice answers to that. The card must read identically on any
wallpaper; the list must survive a 200 ms glance; the overlay must appear
at full fidelity in its first frame. Where a decision is already pinned in
the code or docs — titles-only rows, the generic window glyph, the
near-opaque card, the mid-gesture rules in the README — the design follows
it; pinned decisions win.

The overlay's content is other apps' titles. Airlock's own visual identity
is the card that holds them: two near-opaque fills, one text color, one
glyph, on an 8-pixel grid. That austerity is deliberate — it is what a
hand-composited surface does well — and it is the identity. Don't furnish
the room.

## Tokens before values

- **The design system is the constants block in `src/ui.rs`.** Glyph and
  icon cells, gaps, paddings, `MAX_COLS`, the render-scale floor, and the
  color values are named, documented constants. Every new visual value
  lands there with a doc comment — never as a literal inside render math.
- **Near-opaque is a contract, not a taste.** The background and selection
  fills carry alpha ≥ 0xFA, asserted at compile time, so the card reads
  the same on light and dark wallpapers. A new color states what it does
  on both polarities or it doesn't ship.
- **The 8-pixel grid is the layout system.** One 8×8 bitmap font (drawn at
  `FONT_SCALE`, so 16-px text), no font anti-aliasing: metrics are whole
  logical pixels in the 4/8 step scale the constants already use,
  symmetric padding mirrors top and bottom, and the row gap lives between
  rows only. Corners are the one deliberate exception — rounded by exact
  area-sampled coverage (`shape_cov`), never blurred.
- **Rows carry the app's themed icon; the generic window glyph is the
  fallback.** Re-pinned 2026-11-01 by the owner (`overlay-visuals`
  module): titles stay primary, icons accompany them at 32 px from the
  pop-launcher pipeline in `src/icons.rs`. Iconless rows draw the one
  window glyph, scaled (`GLYPH_SCALE`) to hold its weight next to real
  icons — still one glyph vocabulary, never per-app decoration.

## Calibrate against generic output

AI-designed switcher overlays cluster around tells. Each is legitimate for
some briefs; here most are wrong:

1. The glass card — semi-transparent fill, blur, wallpaper showing
   through. Translucency on a hand-composited shm surface means bleed, and
   the near-opaque contract exists precisely to prevent it.
2. Per-app icons *standing in for* titles — icons accompany, never
   replace, the title (re-pinned 2026-11-01; see the tokens above).
3. Fade-in/fade-out choreography. Motion here costs first-frame latency —
   and the gesture itself is the animation.
4. Metadata dressing: app ids under titles, middle-dot joins, tracked
   ALL-CAPS eyebrows, a header saying "Switch windows".
5. Drop shadows and blur — impossible without soft edges on this
   renderer. Rounded corners are the deliberate exception: radius 12 with
   exactly area-sampled coverage anti-aliasing, plus the 1-px border that
   keeps the card edge readable on dark wallpapers (two preview-review
   rounds set its 18% strength).
6. A louder selection — scaled-up row, accent bar, glow. Selection is one
   flat fill moving at Tab speed; that is enough hierarchy for a glance.

Where an axis is genuinely free, don't spend the freedom on a default.

## Glance-quality floor

Build to this floor without announcing it:

- **Both wallpaper polarities pass.** Verify every color and contrast
  change on a light and a dark wallpaper, not just the one on screen.
- **Degradation is honest.** Non-ASCII titles render as `?` today; a CJK
  title reads as `?????`. If that's no longer acceptable, the fix is a
  font decision with its costs stated — never a special case for one
  script.
- **Shape the edges with data**: one row and fifteen; the 48-column
  truncated title and the empty one (falling back to `(untitled)`);
  selection at the top, middle, and bottom of the list.
- **The overlay never lingers.** `CLOSE_AFTER` and `HARD_CLOSE` are safety
  behavior, not styling; visual changes add zero mapped time.
- **Layout math stays headless.** The model/renderer split (plain data in,
  bytes out, unit-tested without a compositor) is a design constraint too:
  what can't be asserted in a test gets verified in the COSMIC VM — never
  by running the app on the host (see AGENTS.md) — against the gesture and
  soak sections of `docs/hardware-checklist.md`. The design-review loop
  itself is headless: `cargo run --example overlay-preview -- out.png`
  renders the scenarios over light and dark backdrops; judge the image,
  not the code.

## Motion

The overlay's appearance is the one orchestrated moment, and it costs zero
frames: one state, drawn whole. Resist compensating for the framework's
stillness — there is no framework here, and the < 50 ms warm-press budget
(reference: 3–4 ms) is the whole point of the tool. The only motion the
user ever sees is the selection fill walking the rows at Tab speed. That's
the motion vocabulary; adding to it means subtracting from the gesture.

## Copy is design

The overlay shows almost no authored language — one string, `(untitled)`.
Airlock's words live where users actually meet them: CLI output
(`airlock list`, the usage text), config diagnostics in the daemon log,
and the docs.

- Diagnostics name the culprit and the way out — the line/column or the
  1-based entry number — and always land in a state where switching still
  works ("keeps using the last-good settings").
- Docs keep the established plain voice: short declaratives, the user as
  actor ("Airlock reads the file; you write it"). Match it; don't
  formalize it.
- Vocabulary is one system across README, docs, CLI, and diagnostics —
  workspace, sticky, MRU, and isolated mean the same thing everywhere.
- Errors state what happened and which rule applied. No apologies, no
  vagueness, no stack traces in user-facing text.

## Process: plan, critique, build, review

1. **Plan** the token change before touching render code: which constants
   move, what each does on light and dark wallpapers, and the glance
   hypothesis — "at 200 ms the user extracts ___". An ASCII sketch of the
   card settles alignment arguments cheaply.
2. **Critique** the plan against the generic switcher answer above;
   wherever the plan matches it by default rather than by decision, revise
   that part and say what changed and why.
3. **Build** in the constants block, keep the compile-time asserts
   passing, and extend the headless tests for any layout-math change.
4. **Review** the preview sheet (`overlay-preview`) and the running
   overlay in the VM, not the code: a light and a dark wallpaper, a long
   list and a one-row list, selection at the edges; walk the gesture and
   overlay-visuals sections of `docs/hardware-checklist.md`; and remove
   one accessory before calling it done.
