---
name: Airlock
description: Nothing crosses workspaces — a workspace-isolated Alt+Tab switcher for the COSMIC desktop.
colors:
  # The two chosen anchors per polarity. The blend system derives
  # everything else (band 20%, hairline 18/22%, muted 55%); custom
  # card/text pairs recompute those three, never pick them by hand.
  lock-chamber-charcoal: "#1E2126fa" # dark card fill; also the light card's ink (#1E2126)
  hatch-white: "#F2F4F8" # dark card ink: titles, glyphs, hint digits
  warm-airlock-cream: "#F7F4F0fa" # light card fill
  # Derived, dark card (byte-exact from src/ui.rs)
  chamber-band: "#484B50fa" # selection: ink at 20% over the fill
  chamber-hairline: "#44464Bfa" # card edge: ink at 18% over the fill
  chamber-muted: "#929599fa" # hint digits: ink at 55% over the fill
  # Derived, light card
  cream-band: "#CBC9C7fa"
  cream-hairline: "#C7C5C3fa" # 22% on the light card — one step past the band
  cream-muted: "#7F7F80fa"
typography:
  title:
    fontFamily: "Open Sans" # the desktop's interface font, followed at runtime; any installed family via the `font` key
    fontSize: "12px" # default; `font_size` range 8–24
    fontWeight: 400
  hint:
    fontFamily: "Open Sans"
    fontSize: "12px"
    fontWeight: 400
rounded:
  card: "12px" # Round default; follows the desktop style ladder 12 / 8 / 2
  band: "8px" # selection band, capped at 8 on every style
spacing:
  grid: "8px" # the layout system; Compact steps at 4 (COSMIC's space_xxs, row pad/gap only)
  half: "4px"
  pad-x: "20px" # card inner padding, horizontal
  pad-y: "16px" # card inner padding, vertical
  row-gap: "8px" # between rows only; rows own their padding symmetrically
  select-inset: "4px" # selection band inset from the card edge
  icon: "32px" # list-row app icon
  icon-gap: "12px" # icon-to-title gap
  tile: "64px" # horizontal tile cell width
  tile-icon: "48px" # tile app icon
  tile-gap: "8px"
  tree-indent: "16px" # grouped-card child indent
components:
  card:
    backgroundColor: "{colors.lock-chamber-charcoal}"
    textColor: "{colors.hatch-white}"
    rounded: "{rounded.card}"
    padding: "20px 20px 16px"
  card-light:
    backgroundColor: "{colors.warm-airlock-cream}"
    textColor: "{colors.lock-chamber-charcoal}"
    rounded: "{rounded.card}"
    padding: "20px 20px 16px"
  row:
    backgroundColor: "{colors.lock-chamber-charcoal}"
    textColor: "{colors.hatch-white}"
    height: "48px" # Standard density; 40 Compact / 56 Spacious; icon-driven
  row-selected:
    backgroundColor: "{colors.chamber-band}"
    textColor: "{colors.hatch-white}"
    rounded: "{rounded.band}"
  hint-digit:
    textColor: "{colors.chamber-muted}"
---

<!-- Narrative language (North Star, color names, component character) was
     inferred from PRODUCT.md, the airlock-design skill, and the research
     records after an unanswered owner probe on 2026-11-12. Renaming any of
     it is a one-pass edit; the token values below are code-extracted and
     normative. -->

# Design System: Airlock

## Overview

**Creative North Star: "The 200-ms Glance"**

Airlock's only surface is a switcher card that exists for a fraction of
a second, mid-gesture, often read in peripheral vision. Its whole job
is to make the next window findable in one glance and get out. Every
token in this file answers to that glance: two near-opaque fills, one
ink, one glyph vocabulary, on an 8-pixel grid that follows the desktop
itself. The overlay competes with muscle memory and wins by being
legible before the second Tab press — never by being pretty.

The content is other apps' titles; Airlock's identity is the card that
holds them. That austerity is deliberate — a hand-composited
software-rendered layer-shell surface with no framework, no shadows,
no motion vocabulary. The card appears at full fidelity in its first
frame; the only movement the user ever sees is the selection fill
walking the rows at Tab speed. Restraint is the house style because
every pixel is hand-placed: don't furnish the room.

The desktop is the config. Corner radius, row rhythm, typeface family,
and (optionally) the selection band's tint are followed from COSMIC's
appearance stores at every open — no keys exist for them. A
factory-default desktop renders exactly the card every prior version
painted.

**Key Characteristics:**

- Near-opaque by compile-time contract (alpha ≥ 0xFA): the card reads
  identically on light and dark wallpapers
- The blend system is the design system: two chosen colors per
  polarity, three derived fills (20% band, 18/22% hairline, 55%
  muted)
- No shadows, no blur, no fade — the 1-px hairline is the only
  card/backdrop separation this renderer has
- An 8-px grid stepped by the desktop's density (Compact 4 / Standard
  8 / Spacious 12), rows icon-driven at 40/48/56 px
- One text weight (Regular), one glyph vocabulary, real Unicode
  through fallback chains, honest degradation at the floor

## Colors

Two chosen anchors per polarity; everything else is the ink blended
over the fill at a fixed percent — so no variant can drift from the
system. A custom `card`/`text` pair recomputes the three derived fills
by the same blends and is rejected whole below WCAG 2.2 AA (4.5:1) for
12-px text.

### Primary

- **Lock-Chamber Charcoal** (#1E2126, alpha 0xFA): the dark card's
  fill — the near-opaque chamber. Doubles as the light card's ink
  (#1E2126, alpha 0xFF there): the two polarities are each other's
  inverse.

### Neutral

- **Hatch White** (#F2F4F8): the dark card's ink — titles, glyphs,
  hint digits, the close badge's ×.
- **Warm Airlock Cream** (#F7F4F0, alpha 0xFA): the light card's fill,
  a warm paper white.
- **Chamber Band** (#484B50): the dark card's selection fill — ink at
  20% over the fill, GNOME's `:selected` formula. A neutral lift that
  reads on both polarities without borrowing the user's accent.
- **Chamber Hairline** (#44464B): the dark card's 1-px edge — ink at
  18%, deliberately one step *before* the band so the two never read
  as one.
- **Chamber Muted** (#929599): hint digits — ink at 55%, the mid-band
  of the judged 50–70 range, frozen on the preview sheet 2026-11-04.
- **Cream Band / Cream Hairline / Cream Muted** (#CBC9C7 / #C7C5C3 /
  #7F7F80): the light card's derived trio; its hairline runs at 22%,
  one step *past* the band (against light wallpapers the card is only
  ~12 RGB points brighter than the backdrop, so the symmetric 18% let
  the edges nearly dissolve).

### Named Rules

**The Near-Opaque Contract.** Fill alphas are ≥ 0xFA, asserted at
compile time. Translucency on this renderer means wallpaper bleed; a
new color states what it does on both polarities or it doesn't ship.

**The Two-Chosen-Colors Rule.** A palette is two picked colors and a
polarity; the band, hairline, and muted fills are always computed
blends (20% / 18-or-22% / 55%). Nobody picks a third color by hand.

**The Accent Is a Guest, Not a Resident.** The selection band may
follow the desktop accent (`selection = "accent"`), blended at the
same 20% and forced opaque; the neutral text-at-20% band is the
default, and the accent appears nowhere else on the card.

## Typography

**Display Font:** none — the card has no display tier.
**Body Font:** the desktop's interface font, followed at runtime
(Open Sans on a default COSMIC desktop; any installed family via the
`font` key; a miss falls back to Open Sans with one diagnostic).
**Fallback Font:** the 8×8 bitmap floor, whole-pixel, no
anti-aliasing, drawn at a 3/2 scale when no fonts load at all.

**Character:** one quiet tenant. The face is the desktop's own, at
Regular weight only — the icons carry the row's identity, the title
confirms it.

### Hierarchy

- **Title** (Regular, 12 logical px default, `font_size` 8–24):
  window titles on rows, tiles, and the band caption; grayscale AA,
  real `…` ellipsis, Unicode through fallback chains (CJK, emoji,
  accents render true glyphs).
- **Hint** (Regular, same size, muted ink): the `Ctrl+N` digits —
  same face, lower emphasis, never a second weight.

### Named Rules

**The One Weight Rule.** Regular, always. No weight knob exists;
hierarchy is color (muted vs full ink) and position, never boldness.

**The Band Keeps Its Width.** Larger text fits fewer columns instead
of stretching the card — a bigger size elides long titles sooner
rather than widening the title band.

## Layout

The card is a single column of at most `rows` windows (default 8,
3–10; the `…` ellipsis rows ride on top of that budget). Content-sized
by default; `width`/`min-width` set optional floors (`"100%"` spans
the invoking output, a % or px count floors it), and the card grows to
the larger of floor and content so a title never clips. The height
budget fits 720 px (`CARD_FIT_PX`) — at Spacious density or large
`font_size` the rows budget auto-clamps with one diagnostic; the fit
is guaranteed by construction.

Three presentations share one geometry:

- **Vertical card** (default): icon-led rows, 32-px icon, 12-px
  icon-to-title gap, hint digits in a right-hand band.
- **Horizontal band** (`layout = "horizontal"`): 64-px tiles with
  48-px icons and the title clamped beneath; `columns` sizes it
  (two-thirds of the output width, or 3–12 pinned); one caption names
  the commit target at full card width.
- **Grouped tree** (`grouping = true`): the vertical card with app
  entries as parents, children indented 16 px, chevrons on
  multi-window apps only, count pips (6 px, 10-px pitch) on parents.

The grid is 8 px; Compact density steps at 4 (COSMIC's own
`space_xxs`, row pad and gap only — the one deliberate half-grid
exception). Rows are icon-driven: 40 / 48 / 56 px across Compact /
Standard / Spacious, growing with `font_size` above 12.

**The Overlay Never Lingers.** The card closes 2 s after idle and at
30 s hard, grab or no grab; visual changes add zero mapped time.

## Elevation & Depth

No shadows, ever. The renderer composites exact pixels and cannot
blur; a fake soft edge would be dishonest. Depth is conveyed by one
device only: the 1-px hairline border, tuned per polarity (18% ink on
dark, 22% on light) so the card edge survives both wallpaper
polarities. Everything else is flat fill on fill — the selection band,
the hint digits, the badge, all flat.

### Named Rules

**The Hairline Carries the Card.** Without a shadow, the stroke is the
only card/backdrop separation the overlay has. It stays one step off
the selection band's strength so edge and selection never merge.

## Shapes

Corners are rounded by exact area-sampled coverage (`shape_cov`) —
never blurred, never aliased; the one deliberate exception to the
renderer's hard edges. The radius is a derived value: it follows the
desktop's style choice (Round → 12 px, SlightlyRound → 8, Square → 2 —
a hair of rounding where 0 would read as a bug). The selection band
rounds with the card, capped at 8 px. On a Square desktop everything
tightens to 2-px corners and the card still reads as finished.

## Components

Character: a quiet instrument. Every fill is information; nothing on
the card is decoration.

### Card

- **Shape:** radius per the desktop style ladder (12 px default),
  exact-AA corners
- **Fill:** Lock-Chamber Charcoal / Warm Airlock Cream per polarity,
  near-opaque
- **Edge:** the 1-px hairline; no shadow
- **Padding:** 20 px horizontal, 16 px vertical
- **Behavior:** appears at full fidelity in its first frame; no
  fade-in, no scale, no transition

### Row (vertical card)

- **Shape:** flat; 48 px tall (Standard), icon-driven
- **Rest:** transparent over the card fill; 32-px themed app icon
  (pop-launcher SVG pipeline), 12-px gap, title at full ink,
  right-aligned muted hint digit
- **Selected:** the band — ink at 20% over the fill, inset 4 px from
  the card edge, rounding with the card (capped 8); one flat fill
  moving at Tab speed. No scale-up, no accent bar, no glow.
- **Degraded:** iconless apps draw the one generic window glyph,
  scaled to hold its weight next to real icons; untitled windows show
  `(untitled)` — the overlay's only authored string

### Tile (horizontal band)

- 64-px cell, 48-px icon, title clamped beneath, 8-px gaps; a `…`
  tile at the truncated edge; the band scrolls just enough as the
  selection steps past it. Ctrl+1..9, hover, click, and the wheel
  address tiles exactly as they address rows.

### Close badge

- Hover-revealed on a row or tile: 16-px badge, 20-px click
  territory, the × in full ink; clicking closes the window from the
  card.

### Group affordances

- **Chevron:** multi-window apps only, 8-px gap, no per-app decoration
- **Count pips:** 6-px dots, 10-px pitch, multi-window parents only
- **Tree indent:** 16 px per level

### Caption (horizontal band)

- One line at full card width, end-ellipsized, naming the commit
  target's window title; its line is always reserved.

### Hint digits

- `Ctrl+N` positions 1–9 in muted ink, right-aligned band; the glue
  between glance and commit.

## Do's and Don'ts

### Do:

- **Do** verify every color and contrast change on a light *and* a
  dark wallpaper — the near-opaque contract exists so the card reads
  the same on both
- **Do** land every new visual value as a named, doc-commented
  constant in the `src/ui.rs` constants block — never a literal
  inside render math
- **Do** let the desktop decide radius, rhythm, family, and (if
  asked) the band tint; the factory default renders the card every
  prior version painted
- **Do** keep degradation honest and documented: generic glyph,
  `(untitled)`, the bitmap floor — no fake polish
- **Do** judge the preview sheet (`cargo run --example overlay-preview
  -- out.png`), not the code

### Don't:

- **Don't** add translucency, blur, or a glass fill — the renderer
  bleeds, and the near-opaque contract exists to prevent it
- **Don't** add fade-in, fade-out, or any choreography — the gesture
  is the animation; motion costs first-frame latency
- **Don't** add drop shadows or any soft edge — exact-AA corners are
  the one deliberate exception
- **Don't** let icons stand in for titles — icons accompany, never
  replace; and never per-app decorative glyphs beyond the one window
  glyph
- **Don't** dress rows with metadata: app ids under titles, middle-dot
  joins, tracked ALL-CAPS eyebrows, or a "Switch windows" header
- **Don't** make the selection louder — one flat fill moving at Tab
  speed is all the hierarchy a glance needs
- **Don't** introduce a second text weight or a per-element font size
  outside the one `font_size` scale
