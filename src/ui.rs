//! Switcher overlay: a pure render model, a software renderer, and the
//! hold-Alt gesture state machine.
//!
//! The model and renderer are plain data in, bytes out — unit-tested
//! without a compositor. The layer-shell surface that presents them lives
//! with the protocol code in [`crate::wayland`] and is only provable on
//! real COSMIC hardware.

use std::rc::Rc;
use std::time::Duration;

use cctk::sctk::seat::keyboard::Keysym;
use cosmic_client_toolkit as cctk;

use crate::ipc::Direction;
use crate::switcher::{self, Handle, Scope, Snapshot};
use crate::theme::{Density, Style};

pub use crate::switcher::RowKind;

/// Glyph cell width in logical pixels.
pub const GLYPH_W: u32 = 8;

/// Glyph cell height in logical pixels.
pub const GLYPH_H: u32 = 8;

/// A rasterized app icon, ready to blit: square, premultiplied ARGB — one
/// `0xAARRGGBB` per pixel, the renderer's write order — with `side` in
/// device pixels (logical size × render scale). Produced by
/// [`crate::icons`]; the renderer never decodes anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IconBitmap {
    /// Edge length in device pixels.
    pub side: u32,
    /// Premultiplied ARGB pixels, row-major, `side × side`.
    pub argb: Vec<u32>,
}

/// Font scale as an exact rational (numerator over denominator): text is
/// 12 logical px tall — re-pinned 2026-11-07 by the owner. The 11-02
/// re-pin to 8 px ("even 12 read oversized next to the 32-px icons") was
/// judged on the 200 % VM, where 8 logical px paint as 16 physical; on a
/// 100 % output the same value is 8 physical pixels, a terminal's
/// smallest font, and the owner read it as too small. The value didn't
/// change, the display did — so the pin moves to the middle step, still
/// under the icons in weight (they carry the row's identity, the title
/// confirms it). Whole-pixel at the render-scale floor of 2 (each font
/// pixel is 3 device px), which covers every output up to 200 %; at a 3×
/// output the glyph block floors to 4 device px inside a 4.5-px cell and
/// the text letter-spaces — stated here, not hidden. 16 px (`2/1`) is
/// exact at every scale and stays the alternative if that ever matters.
/// The studied switchers' 13–16 px consensus (research record §2.10)
/// spends its height on richer fonts, not on proportion.
pub const FONT_SCALE_NUM: u32 = 3;
pub const FONT_SCALE_DEN: u32 = 2;

/// Title text cell width in logical pixels (glyph cell × font scale).
pub const TEXT_W: u32 = GLYPH_W * FONT_SCALE_NUM / FONT_SCALE_DEN;

/// Title text height in logical pixels.
pub const TEXT_H: u32 = GLYPH_H * FONT_SCALE_NUM / FONT_SCALE_DEN;

/// The card's text size in logical pixels when the config sets no
/// `font_size` — the 12-px pin the card has carried since 2026-11-07.
/// A configured size resolves once at open into a [`TextProfile`], which
/// every text-derived metric reads from.
pub const DEFAULT_FONT_SIZE: u32 = TEXT_W;

/// Smallest `font_size` the setting accepts: 8 px is the bitmap floor's
/// own 1× cell. Below it the 8×8 glyphs would need to downscale, and the
/// card's 8-px disclosure caret no longer has a whole-cell home.
pub const FONT_SIZE_MIN: u32 = 8;

/// Largest `font_size` the setting accepts. 24 px still clears the
/// icon-driven row (the Compact row is 40 high), and the title band
/// narrows its column budget instead of growing past its ~600-px cap
/// (see [`TextProfile::title_cols`]).
pub const FONT_SIZE_MAX: u32 = 24;

/// App-icon cell, square, in logical pixels. 32 is the converged switcher
/// row-icon size — cosmic-launcher's alt-tab row uses exactly this
/// (research record §2.6); GNOME's app badge is 48 inside much larger
/// tiles. Icons arrive pre-rasterized at `ICON × render scale` from
/// [`crate::icons`].
pub const ICON: u32 = 32;

/// Gap between the icon cell and the title text.
pub const ICON_GAP: u32 = 12;

/// Row height in logical pixels: the 32-px icon cell with 8 px of
/// breathing above and below (cosmic-launcher's row `padding([8, 24])`
/// rhythm; research record §2.6). The selection band fills this whole
/// row rect, so the fill holds icon and text as one object.
/// Derived default (Round + Standard) since 2026-11-07: [`rhythm`]
/// re-derives it per open — 40/48/56 across the densities at the default
/// text size; [`Rhythm::with_text`] grows it for a larger `font_size`.
pub const ROW_H: u32 = ICON + 2 * ROW_PAD_Y;

/// Vertical padding inside one row. Derived default (Standard) since
/// 2026-11-07: [`rhythm`] re-derives it per open — 4/8/12 across the
/// densities.
pub const ROW_PAD_Y: u32 = 8;

/// Gap between rows; selection bands never merge across it. Derived
/// default (Standard) since 2026-11-07: [`rhythm`] re-derives it per
/// open — 4/8/12 across the densities.
pub const ROW_GAP: u32 = 8;

/// Horizontal inset of the selection band from the card edges, so the
/// highlight reads as a row button (cosmic-launcher's inset row fills)
/// rather than a full-bleed stripe clipped by the border.
pub const SELECT_INSET: u32 = 4;

/// Corner radius of the selection band's own ends, logical px. The band
/// is a row button, so its ends round into the card's corner vocabulary
/// instead of ending as clipped stripes against the 12-px card radius —
/// drawn by the same coverage sampler, still one flat fill (no accent,
/// no glow). `PAD_Y ≥ RADIUS` holds the band clear of the card's corner
/// arcs. 2026-11-07 layout pass, preview-frozen beside [`PAD_Y`];
/// since the style follow same day it is the cap of [`band_radius`],
/// following the card's radius at 8 and below.
pub const BAND_RADIUS: u32 = 8;

/// Horizontal padding inside the card.
pub const PAD_X: u32 = 20;

/// Vertical padding inside the card; rows start and end inside it.
/// Double the row gap: the card edge breathes one deliberate step above
/// the row rhythm, so an edge selection reads as held by the container
/// rather than parked beside its neighbor. Also keeps `PAD_Y ≥ RADIUS`
/// with real slack — 12 sat at exact equality, one padding trim away
/// from clipping a corner arc. 2026-11-07 layout pass (16 = 2 × the
/// 8-px row gap). Derived default (Standard) since 2026-11-07:
/// [`rhythm`] re-derives it per open — 8/16/24 across the densities;
/// the corner clearance is pinned per profile now (`pad_y +
/// SELECT_INSET ≥ radius` — Compact's 8 spends the inset, with the
/// sampled proof in the tests).
pub const PAD_Y: u32 = 16;

/// Card corner radius in logical pixels. COSMIC's house radius for cards
/// is `radius_s + 4` ≈ 12 (research record §2.6). Corners are coverage
/// anti-aliased by area sampling — see [`shape_cov`] — not aliased steps;
/// nothing is blurred. Derived default (Round) since 2026-11-07:
/// [`style_radius`] re-derives it per open — 12/8/2 across the styles.
pub const RADIUS: u32 = 12;

/// Hairline card border thickness in logical pixels.
pub const BORDER: u32 = 1;

/// Maximum title columns shown before elision at the default 12-px
/// cell — 48 columns. A session's [`TextProfile`] derives its own column
/// count from [`TITLE_BAND_PX`], so a larger `font_size` fits fewer
/// columns in the same physical band. The rest of this note describes
/// the default's arithmetic: 48 columns
/// keeps the title territory at 660 logical px — within reach of
/// cosmic-launcher's 596 fixed switcher width and the cross-platform
/// consensus cap of ~600 (research record §2.6, §2.10); the full card,
/// hint band included, is 688 since the 2026-11-09 digit re-pin.
/// Re-pinned 2026-11-07 with the 12-px text from the 8-px era's 64
/// columns (596 territory, 660 card): the band consensus holds, the
/// column budget pays for the larger glyphs. The hint band rides on top
/// of the cap rather than under it (rev 2's recorded decision: the card
/// widens by the band, the title budget does not fund it).
pub const MAX_COLS: u32 = 48;

/// The title band's width budget in logical pixels — [`MAX_COLS`] at the
/// default 12-px cell. A [`TextProfile`] divides this budget by its own
/// cell to derive its column count, so a larger `font_size` fits fewer
/// characters in the same band instead of widening the card past the
/// ~600-px switcher consensus.
pub const TITLE_BAND_PX: u32 = MAX_COLS * TEXT_W;

/// Gap between the title band's right edge and the hint band, logical
/// px. The hint's territory is its own, never the title's — a
/// max-columns title still clears it by the full gap. Two 8-grid
/// steps; judged beside a 64-column title on the preview sheet
/// (2026-11-04) before freezing.
pub const HINT_GAP: u32 = 16;

/// Reserved hint columns (the longest label, a single digit `9`).
/// Structural: every card carries the band regardless of row count;
/// only rows 1..9 paint in it. Re-pinned 2026-11-09 (owner, in-thread)
/// from the full `Ctrl+9` chord, which repeated on every row and read
/// as a table column of metadata. The vertical cards now share the
/// horizontal tiles' bare-digit vocabulary; the reservation shrank
/// 6 → 1 column and the card narrowed with it.
pub const HINT_COLS: u32 = 1; // len("9")

/// The query strip's breathing above its text, logical px
/// (`type-filter`). One 4-grid step, the HINT_GAP family; the strip's
/// own top zone, separate from the card's row padding, so the query
/// never crowds the first row — the row gap below the strip's text
/// supplies that separation (the strip is not a row and earns no
/// row height).
pub const STRIP_PAD_Y: u32 = 8;

/// Clear between the query's last glyph and the caret bar, logical px:
/// one 4-grid step — the caret reads as the text's terminal, not its
/// neighbor.
pub const STRIP_CARET_GAP: u32 = 4;

/// The caret bar's width, logical px: Square's own hair (the one 2-px
/// exception's precedent) — a static terminal, not a blinking one (the
/// motion vocabulary is the selection fill and nothing else).
pub const STRIP_CARET_W: u32 = 2;

/// Indent of a grouped child row's whole cell — icon and title — from
/// the top-level column, logical px: two 8-grid steps, deep enough to
/// read as nesting at a glance without re-parenting the row rhythm (the
/// band still spans the card; only the content indents). Frozen
/// 2026-11-08 on the GR4 sheet, both palettes over both backdrops (the
/// LV3 pattern).
pub const TREE_INDENT: u32 = 16;

/// Title-band columns reserved after a parent row's title for its
/// disclosure caret and its clearing: the 8-px caret plus one
/// [`CHEVRON_GAP`]. Re-pinned 2026-11-09 (owner, in-thread) from the
/// numeric count chip and then from the inline pips: the pips moved
/// below the app name, so they take no title width, and only the caret
/// remains on the title line. Two columns, down from five, give parent
/// titles back three columns. This is the default profile's value;
/// [`TextProfile::chevron_cols`] derives the same reservation from the
/// session's cell size.
pub const CHEVRON_COLS: u32 = 2;

/// Clearing between a parent's title and its disclosure caret, logical
/// px — one 8-grid step. The count pips no longer spend this gap: they
/// sit below the title, in the row's own bottom padding.
pub const CHEVRON_GAP: u32 = 8;

/// The disclosure chevron's draw scale: the 8×8 caret glyphs paint at
/// 1× = 8 logical px, inline beside the parent's title. Re-pinned
/// 2026-11-09 (owner, in-thread): the 2× solid triangle floating alone
/// in the hint-gap column read as a play button, disconnected from the
/// row it described. The mark moved onto the title line and shrank; it
/// reads as tree structure, not a control. The hand-drawn glyph keeps
/// a two-pixel stroke, so 8 px still holds its weight where the old
/// muted `>`/`v` text glyphs failed.
pub const CHEVRON_SCALE: u32 = 1;

/// Windows the card keeps in view on a long list when the config sets
/// no `rows` — the setting's default. The budget counts *windows*, not
/// slots: a longer list reserves [`ROWS_SLACK`] slots beyond it, so the
/// `…` continuation rows ride on top of the budget (`slots =
/// min(len, rows + ROWS_SLACK)`) and eight windows stay in view
/// mid-list, nine at either end, every window of a ≤10-long list.
/// Re-pinned 2026-11-07 from the 11-04 "8 slots" rule, whose `…` rows
/// spent two of the budget and left six windows mid-list. Worst case at
/// the default is 10 slots = 584 logical px — inside a 720-logical
/// screen (1440p@2x, any 1× panel); a 1080p panel at 2× (540 logical)
/// wants `rows = 7`.
pub const DEFAULT_ROWS: usize = 8;

/// Smallest `rows` the setting accepts — the 11-04 floor, kept: with
/// both edges truncated the window must still hold a content row, and a
/// card showing fewer than three windows stops reading as a list.
pub const ROWS_MIN: usize = 3;

/// Largest `rows` the setting accepts: its worst case, `ROWS_MAX +
/// ROWS_SLACK` = 12 slots = 696 logical px, still fits a 720-logical
/// screen.
pub const ROWS_MAX: usize = 10;

/// Slots a long list reserves beyond its `rows` budget: one per
/// truncatable edge, so the `…` affordance rows never spend window
/// budget.
pub const ROWS_SLACK: usize = 2;

/// Smallest `columns` the setting accepts — the floor below which the
/// horizontal band stops reading as a strip (the [`ROWS_MIN`]
/// reasoning, one axis over): three tiles is the shortest thing that
/// still looks like a band rather than a pair.
pub const COLUMNS_MIN: usize = 3;

/// Largest `columns` the setting accepts — the ceiling that keeps a
/// pinned band a glance, not a wall: twelve tiles at the compact
/// pitch (glance-tiles G1, 2026-11-09: 72 px a slot) already span half
/// a 4-K output's logical width.
pub const COLUMNS_MAX: usize = 12;

// --- The horizontal band's tokens (glance-tiles; G1) -------------------
//
// Re-pinned 2026-11-09 (glance-tiles G1) from the CL4-frozen 152-px
// tiles: the tiles go icon-only at a compact pitch — a 64-px slot, the
// ~72-px pitch of the research record's candidate geometry, 2.2×
// denser than CL4 — and the card gains the one caption line that took
// the per-tile title lines' place. The pip decomposition is the
// owner's pick the same day (the cell grows a pip territory under the
// icon so the pips sit in their own space, GNOME-arrow style, never
// painted over the icon).
//
// Frozen 2026-11-09 on the G4 sheet — both horizontal families
// (combined and flat) over both polarities and both text faces, the
// acceptance scenarios (1/3/12 tiles, the swept selection, the
// truncated edge, both strip shapes, the over-budget caption) — every
// value kept as built except the pips: both sheet judgments named
// them the weakest element at 4 px, so PIP_SIDE grew to 6 (2.25× the
// ink) inside the same 12-px territory and the re-judged sheet
// passed. The caption floor's air over short strips reads calm; the
// muted hint over a selection fill stays the vertical card's own
// frozen vocabulary (2.4–2.9:1, measured — the badge digits' ratios);
// the light palette's fill is pre-existing through two freezes and
// out of this module's byte-identity boundary.

/// One tile's width, logical px — the compact slot of the research
/// record's candidate geometry (a 48-px icon in a 64-px cell), on the
/// 8-grid. Re-pinned 2026-11-09 (glance-tiles G1) from CL4's 152.
pub const TILE_W: u32 = 64;
/// Gap between tiles: selection fills never merge across it.
pub const TILE_GAP: u32 = 8;
/// The tile's app icon side — the icon keeps its weight while the tile
/// sheds its title line (glance-tiles G1, 2026-11-09; larger than the
/// row's 32: the tile is the card's whole content, the row's icon one
/// column of several).
pub const TILE_ICON: u32 = 48;
/// The band card's horizontal padding, logical px.
pub const TILE_PAD_X: u32 = 24;
/// The band card's vertical padding, logical px. Re-pinned 40 → 32
/// (glance-tiles G1, 2026-11-09): the caption line and the pip
/// territory joined the card, and 32 + caption + gap + cell keeps the
/// working height at exactly 168 — the padding pays for the caption
/// the title lines used to.
pub const TILE_PAD_Y: u32 = 32;
/// Clearing between the badge row and the icon.
pub const TILE_PARENT_GAP: u32 = 4;
/// Clearing between the icon's bottom and the pip row (glance-tiles
/// G1, 2026-11-09: the owner's pip decomposition). 6 after the G4
/// sheet: the territory stays 12 (the cell stays 76, the card 168)
/// while the dot itself grows to its judged legibility.
pub const PIP_GAP: u32 = 6;
/// One count pip's side, logical px. Saturates at three pips
/// (Unity/GNOME/dash-to-dock precedent, research record R4). Frozen
/// at 6 on the G4 sheet, 2026-11-09: the working 4 read as texture at
/// glance scale — both sheet judgments named the pips the weakest
/// element, gone against the selection fill and the light backdrop —
/// and 6 (2.25× the ink) holds the muted color's contrast on both
/// fills where 4 demanded a squint.
pub const PIP_SIDE: u32 = 6;
/// Pitch between pip centers, logical px — `PIP_SIDE` plus a 4-px
/// clear between neighbors; three pips span 26 px centered on the
/// tile's line. Re-frozen beside [`PIP_SIDE`] on the G4 sheet,
/// 2026-11-09.
pub const PIP_STEP: u32 = 10;
/// Clearing between a parent's title line and its count pips, logical
/// px — the gap the default row's centered line already leaves. The row
/// height is derived from it (see [`Rhythm::with_text`]) so the pips
/// never share the text line at any `font_size`.
pub const PIP_TEXT_CLEAR: u32 = 4;
/// The tile's content cell at one text size: badge row, icon, pip
/// territory — no title line. The selection fill's extent; the pip row
/// is structural on every tile (pips paint only on multi-window app
/// tiles), so the cell never varies per tile.
pub const fn tile_cell_h_at(cell: u32) -> u32 {
    cell + TILE_PARENT_GAP + TILE_ICON + PIP_GAP + PIP_SIDE
}
/// The tile cell at the default text size (the constants block).
pub const TILE_CELL_H: u32 = tile_cell_h_at(TEXT_H);
/// Clearing between the caption line and the strip's badge row, two
/// 8-grid steps (glance-tiles G1, 2026-11-09).
pub const CAPTION_GAP: u32 = 16;
/// The horizontal card's height at one text size: the caption line, its
/// gap, the tile cell, and the card padding — 168 logical px at the
/// default 12-px cell, the working target the G4 sheet freezes
/// (glance-tiles G1, 2026-11-09): the caption line (12) + its gap (16)
/// take the per-tile title lines' place (12 + 12) and the padding pays
/// the difference (40 → 32).
pub const fn band_card_height_at(cell: u32) -> u32 {
    2 * TILE_PAD_Y + cell + CAPTION_GAP + tile_cell_h_at(cell)
}
/// [`band_card_height_at`] at the default text size.
pub const fn band_card_height() -> u32 {
    band_card_height_at(TEXT_H)
}
/// The band card's width: `slots` tiles plus gaps inside the padding —
/// the `…` tile lives inside these slots, so the card never widens for
/// it — floored at [`CAPTION_FLOOR_W`] so a short strip's caption
/// still reads at a glance. The flat band and the grouped strip join
/// the same floor; tiles stay left-aligned in a floored width.
pub const fn band_width(slots: u32) -> u32 {
    let shape = 2 * TILE_PAD_X + slots * TILE_W + slots.saturating_sub(1) * TILE_GAP;
    if shape > CAPTION_FLOOR_W {
        shape
    } else {
        CAPTION_FLOOR_W
    }
}
/// The width floor every horizontal card carries, logical px — the
/// caption's guarantee: the spec's ≥ 40-bitmap-column sizing target at
/// the ~512 working estimate, on the 8-grid (glance-tiles G1,
/// 2026-11-09). At the floor the caption's ink budget (width minus
/// the card's horizontal padding) is 464 px ≈ 38 bitmap columns; the
/// 528 that buys the full 40 is the G4 sheet's call, judged beside a
/// floored three-tile card's air.
pub const CAPTION_FLOOR_W: u32 = 512;
/// The workspace label's size step below the resolved text cell
/// (workspace-label W2, 2026-11-09): the name reads smaller than the
/// caption it shares the line with, one face and one color — the
/// hierarchy is size and position alone. The cell floors at
/// [`FONT_SIZE_MIN`], so at the 8-px text size the label paints at the
/// one size the card has.
pub const WS_LABEL_STEP: u32 = 2;
/// The workspace label's ink cap, in label cells (workspace-label W6,
/// 2026-11-09 — column-relative since the numbered form grew a
/// prefix): a name longer than the cap truncates with the real `…`,
/// and the label never widens the card — it loses to the width the
/// shapes already chose. Fourteen cells hold the longest synthesized
/// form, "# Workspace 12", with room; working value, the preview sheet
/// freezes it.
pub const WS_LABEL_MAX_COLS: u32 = 14;
/// Clearing around the label's line (workspace-label W2, 2026-11-09;
/// its own line since W7): below the tiles on the horizontal cards,
/// above the first row on the vertical ones — the caption gap's own
/// double 8-grid step.
pub const WS_LABEL_GAP: u32 = 16;
/// The workspace label's text cell at one text size — the resolved
/// cell minus [`WS_LABEL_STEP`], floored at [`FONT_SIZE_MIN`].
pub const fn ws_label_cell_at(cell: u32) -> u32 {
    let stepped = cell.saturating_sub(WS_LABEL_STEP);
    if stepped < FONT_SIZE_MIN {
        FONT_SIZE_MIN
    } else {
        stepped
    }
}
/// Inset of the badge from a tile's side.
pub const TILE_TEXT_PAD: u32 = 8;
/// The strip's top edge inside the card at one text size: the top
/// padding, the reserved caption line, and the caption-to-strip gap —
/// the y0 every tile paints from and the line `cell_at`'s y footprint
/// starts at (glance-tiles G2, 2026-11-09). The reserved line exists on
/// every frame whether or not the caption has text.
pub const fn band_strip_top_at(cell: u32) -> u32 {
    TILE_PAD_Y + cell + CAPTION_GAP
}
/// [`band_strip_top_at`] at the default text size.
pub const fn band_strip_top() -> u32 {
    band_strip_top_at(TEXT_H)
}
/// The icon's top edge inside a tile at one text size, logical px: the
/// strip top, the badge row, and its clearing — the line `render_band`
/// blits from and the vertical anchor of the tile's × badge.
pub const fn band_icon_top_at(cell: u32) -> u32 {
    band_strip_top_at(cell) + cell + TILE_PARENT_GAP
}
/// [`band_icon_top_at`] at the default text size.
pub const fn band_icon_top() -> u32 {
    band_icon_top_at(TEXT_H)
}
/// `auto`'s share of the invoking output's logical width — the spec's
/// ~66 % working range, two thirds exactly; frozen on the CL4 sheet
/// 2026-11-08.
pub const AUTO_FRACTION_NUM: u32 = 2;
pub const AUTO_FRACTION_DEN: u32 = 3;
/// The width `auto` resolves against when the invoking output's
/// logical size is unknown (no output yet, or none reported one): a
/// 1080p panel's logical width, so the responsive band still behaves
/// before the first configure.
pub const FALLBACK_OUTPUT_WIDTH: u32 = 1920;

/// The panel the card must always fit: 720 logical px (1440p@2x, any
/// 1× panel) — `overlay-density`'s headroom rule, unchanged. Since the
/// density follow the guarantee is by construction: the effective rows
/// budget clamps to what fits (see [`effective_rows`]).
pub const CARD_FIT_PX: u32 = 720;

/// The effective `rows` budget at one density: the largest budget at
/// or under `rows` (never past [`ROWS_MAX`]) whose worst-case card —
/// budget + [`ROWS_SLACK`] slots — fits [`CARD_FIT_PX`]. Spacious is
/// the binding density at the default text size (68-px slot pitch: 8 +
/// slack = 716 fits, 9 + slack = 784 does not); Standard and Compact
/// never clamp there. A larger `font_size` grows the rows, so the clamp
/// follows the session's resolved rhythm ([`effective_rows_rhythm`]).
/// The setting keeps its value — this is the fit, and the daemon's one
/// diagnostic names density, text size, setting, and clamp when it
/// binds.
pub const fn effective_rows(density: Density, rows: usize) -> usize {
    effective_rows_rhythm(rhythm(density), rows)
}

/// [`effective_rows`] at a session's resolved rhythm — the density's
/// rows grown for the `font_size` (see [`Rhythm::with_text`]). The
/// accepted size range guarantees at least [`ROWS_MIN`] windows still
/// fit at every density (the test pins the worst case).
pub const fn effective_rows_rhythm(rhythm: Rhythm, rows: usize) -> usize {
    let mut budget = if rows > ROWS_MAX { ROWS_MAX } else { rows };
    while budget > 0 && card_height_rhythm(rhythm, (budget + ROWS_SLACK) as u32) > CARD_FIT_PX {
        budget -= 1;
    }
    budget
}

/// Card height in logical px for `slots` rows: the padding top and
/// bottom, every row, and a gap between rows only. The one place the
/// formula lives — the compile-time pins and [`overlay_model`] agree by
/// construction. Takes the profile's rhythm since the density follow
/// (GE3); the constants above stay as the Standard row.
pub const fn card_height(slots: u32) -> u32 {
    card_height_rhythm(rhythm(Density::Standard), slots)
}

/// Card height for `slots` rows under one density's rhythm — the
/// formula itself, shared by the const pins and the model builders.
pub const fn card_height_rhythm(rhythm: Rhythm, slots: u32) -> u32 {
    2 * rhythm.pad_y + slots * rhythm.row_h + (slots - 1) * rhythm.row_gap
}

// --- The geometry ladders ---------------------------------------------------
//
// The card's shape follows the desktop (owner-ratified 2026-11-07):
// COSMIC's style choice derives the corner radius, its interface
// density the row rhythm. Both are Airlock design decisions on the
// store's evidence, never passthroughs — the store's raw radii feed the
// judgment, the 8-grid values stay ours. The constants block above is
// the Round + Standard row these ladders re-derive per open; the
// profile freezes onto the session like the palette (GE3).

/// Row rhythm per desktop density — COSMIC's own spacing ratios
/// (compact ≈ ×2/3, spacious ≈ ×4/3, store record §6) on Airlock's
/// 8-grid anchors. At the default text size [`Rhythm::row_h`] is
/// icon-driven (ICON + 2 × `row_pad_y`); [`Rhythm::with_text`] grows it
/// for a larger `font_size`. `PAD_X` and the band inset have no density
/// analogue and stay pinned. Compact's 4-px steps are the one deliberate
/// half-grid exception (OQ 2, ratified): `row_pad_y`/`row_gap` only —
/// COSMIC's own `space_xxs` compact step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rhythm {
    /// Vertical padding inside one row.
    pub row_pad_y: u32,
    /// Gap between rows; selection bands never merge across it.
    pub row_gap: u32,
    /// Vertical padding inside the card.
    pub pad_y: u32,
    /// Row height: the icon cell plus its breathing, or the text line
    /// plus the parent pips' clearing when a larger `font_size` needs
    /// the room ([`Rhythm::with_text`]).
    pub row_h: u32,
}

/// The rhythm ladder. Standard's row *is* the constants block above —
/// the documented default the follow re-derives — pinned against its
/// literals by the ladder table test and against the pre-follow render
/// by the digest test. Compact and Spacious are the table's own values.
pub const fn rhythm(density: Density) -> Rhythm {
    match density {
        Density::Compact => Rhythm {
            row_pad_y: 4,
            row_gap: 4,
            pad_y: 8,
            row_h: 40,
        },
        Density::Standard => Rhythm {
            row_pad_y: ROW_PAD_Y,
            row_gap: ROW_GAP,
            pad_y: PAD_Y,
            row_h: ROW_H,
        },
        Density::Spacious => Rhythm {
            row_pad_y: 12,
            row_gap: 12,
            pad_y: 24,
            row_h: 56,
        },
    }
}

impl Rhythm {
    /// The same density rhythm with its row tall enough for one `cell`
    /// of text and the parent pips' own clearing. The icon, gaps, and
    /// padding stay fixed; above the default 12-px cell the row grows by
    /// the line's extra height. At 12 px the formula lands exactly on
    /// the density rows (40/48/56), so the default card is unchanged.
    pub const fn with_text(mut self, cell: u32) -> Self {
        let icon_driven = ICON + 2 * self.row_pad_y;
        let text_driven = cell + 2 * self.row_pad_y + 2 * (PIP_SIDE + PIP_TEXT_CLEAR);
        self.row_h = if text_driven > icon_driven {
            text_driven
        } else {
            icon_driven
        };
        self
    }
}

/// The card's corner radius, derived from the desktop's style choice.
/// Judged on the sheet 2026-11-07, not transcribed from the store:
/// Square's 2 keeps a hair of rounding on a hairline-border card where
/// 0 would read as a bug; SlightlyRound takes the band's own 8 so card
/// and band round together; Round is today's pin, unchanged.
pub const fn style_radius(style: Style) -> u32 {
    match style {
        // [`RADIUS`] itself — the documented default rung.
        Style::Round => RADIUS,
        Style::SlightlyRound => 8,
        Style::Square => 2,
    }
}

/// The band rounds with the card, never past its own pin
/// ([`BAND_RADIUS`]).
pub const fn band_radius(radius: u32) -> u32 {
    if radius > BAND_RADIUS {
        BAND_RADIUS
    } else {
        radius
    }
}

/// The card's text sizing, resolved once at open from the `font_size`
/// key and frozen with the palette and shape. `size_px` is the em box in
/// logical pixels: the System backend shapes at it, the Bitmap floor
/// scales its 8×8 glyph cells to it. Every text-derived metric — the
/// title band's column budget, the query strip's line, the band's
/// caption line, the parent caret's reservation — reads from this
/// profile, so a larger face fits fewer characters in the same band
/// instead of widening the card past its design cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextProfile {
    /// The em box and glyph cell, logical px.
    pub size_px: u32,
}

impl TextProfile {
    /// A profile at `size_px` logical px. The `font_size` parser bounds
    /// it to [`FONT_SIZE_MIN`]..=[`FONT_SIZE_MAX`].
    pub const fn new(size_px: u32) -> Self {
        Self { size_px }
    }

    /// The square cell one bitmap glyph advances, logical px.
    pub const fn cell(self) -> u32 {
        self.size_px
    }

    /// The cell used by the divisors, floored at 1: the parser can never
    /// hand a zero over, but the arithmetic must not panic if it does.
    const fn safe_size(self) -> u32 {
        if self.size_px == 0 { 1 } else { self.size_px }
    }

    /// Title columns the band holds at this size: [`TITLE_BAND_PX`]
    /// divided by the cell, floored (a partial column is no column).
    /// 48 at the default 12-px cell, 72 at 8, 24 at 24 — the band's
    /// physical width stays inside its ~600-px cap.
    pub const fn title_cols(self) -> u32 {
        TITLE_BAND_PX / self.safe_size()
    }

    /// The title band's width at this size, logical px.
    pub const fn title_band(self) -> u32 {
        self.title_cols() * self.size_px
    }

    /// Columns a parent row reserves after its title for the disclosure
    /// caret and its clearing: the caret's ink plus [`CHEVRON_GAP`],
    /// rounded up to whole cells. Never less than one — a caret is
    /// always reserved.
    pub const fn chevron_cols(self) -> u32 {
        let needed = CHEVRON_GAP + GLYPH_W * CHEVRON_SCALE;
        let cols = needed.div_ceil(self.safe_size());
        if cols == 0 { 1 } else { cols }
    }

    /// Columns a child row's [`TREE_INDENT`] spends from the title
    /// budget (floored, the shipped rule).
    pub const fn indent_cols(self) -> u32 {
        TREE_INDENT / self.safe_size()
    }
}

impl Default for TextProfile {
    fn default() -> Self {
        Self::new(DEFAULT_FONT_SIZE)
    }
}

/// The card's resolved shape: the style's radius and the density's
/// rhythm, frozen onto a session at open (beside the palette) so a
/// desktop flip mid-gesture never re-shapes the open card — GE3's
/// freeze. Default is Round + Standard, the constants block verbatim
/// (pinned by test): COSMIC's factory default and the store-miss
/// default render byte-identically to every card shipped before the
/// follow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StyleProfile {
    /// Card corner radius ([`RADIUS`] on the default profile).
    pub radius: u32,
    /// Selection-band end radius ([`BAND_RADIUS`] on the default).
    pub band_radius: u32,
    /// Row rhythm (Standard's values on the default).
    pub rhythm: Rhythm,
    /// Text sizing ([`TextProfile::default`] on the default).
    pub text: TextProfile,
}

impl StyleProfile {
    /// The profile one open resolves from the followed style and
    /// density. Text starts at the default 12-px pin; the daemon
    /// applies the session's `font_size` through [`Self::with_text`].
    pub const fn resolve(style: Style, density: Density) -> Self {
        let radius = style_radius(style);
        StyleProfile {
            radius,
            band_radius: band_radius(radius),
            rhythm: rhythm(density),
            text: TextProfile::new(DEFAULT_FONT_SIZE),
        }
    }

    /// The same profile at a resolved text size — the one seam the
    /// `font_size` setting crosses. The row rhythm grows with the line
    /// (icons, gaps, and padding stay put), so the parent's pips keep
    /// their clearing and the card's height follows the text.
    pub const fn with_text(mut self, text: TextProfile) -> Self {
        self.text = text;
        self.rhythm = self.rhythm.with_text(text.cell());
        self
    }
}

impl Default for StyleProfile {
    /// Round + Standard — the derived-today row.
    fn default() -> Self {
        Self::resolve(Style::Round, Density::Standard)
    }
}

/// How long the overlay stays up before closing itself when idle.
pub const CLOSE_AFTER: Duration = Duration::from_secs(2);

/// Longest a grabbed overlay may stay mapped, even mid-gesture: a lost
/// Alt-release or a dead input path must never stick the keyboard grab.
pub const HARD_CLOSE: Duration = Duration::from_secs(30);

/// Overlay background (ARGB). Near opaque: the card must read identically
/// on light and dark wallpapers — within a few RGB points of GNOME's OSD
/// panel and cosmic-launcher's themed card (research record TL;DR #13).
pub const COLOR_BACKGROUND: u32 = 0xFA_1E_21_26;

/// Row background for the selected row: the text color at 20% blended
/// over the card — GNOME's `:selected` formula (research record §2.1),
/// a neutral lift that reads on both wallpaper polarities without
/// borrowing the user's accent color.
pub const COLOR_SELECTED_BG: u32 = blend_over(COLOR_BACKGROUND, COLOR_TEXT, 20);

/// Card border: the text color at 18% over the card — the hairline role
/// cosmic-launcher calls `bg_divider` (research record §2.6). Deliberately
/// stronger than the usual divider: two preview review rounds showed
/// anything subtler dissolves against dark wallpapers, and without a
/// shadow the stroke is the only card/backdrop separation the overlay
/// has. Stays below the selection's 20% so the two never read as one.
pub const COLOR_BORDER: u32 = blend_over(COLOR_BACKGROUND, COLOR_TEXT, 18);

/// Text color.
pub const COLOR_TEXT: u32 = 0xFF_F2_F4_F8;

/// Blends `over` at `percent` opacity on top of `under`, alpha channel
/// kept from `under`. Both colors straight (non-premultiplied).
const fn blend_over(under: u32, over: u32, percent: u32) -> u32 {
    const fn mix(under: u32, over: u32, channel: u32, percent: u32) -> u32 {
        let base = (under >> channel) & 0xFF;
        let top = (over >> channel) & 0xFF;
        ((base * (100 - percent) + top * percent) / 100) & 0xFF
    }
    (under & 0xFF00_0000)
        | (mix(under, over, 16, percent) << 16)
        | (mix(under, over, 8, percent) << 8)
        | mix(under, over, 0, percent)
}

// Near-opaque card, enforced at compile time: the background bleed through
// the overlay must stay imperceptible on light and dark wallpapers alike.
const _: () = {
    assert!(COLOR_BACKGROUND >> 24 >= 0xFA);
    assert!(COLOR_SELECTED_BG >> 24 >= 0xFA);
};

/// WCAG 2.x relative luminance of one straight sRGB color — the alpha
/// byte ignored: channels linearized (`c/12.92` below the 0.04045 knee,
/// `((c+0.055)/1.055)^2.4` above), weighted `0.2126 R + 0.7152 G +
/// 0.0722 B`. The shared measure behind both the custom pair's floor
/// and its card's icon-tint polarity.
pub fn relative_luminance(argb: u32) -> f32 {
    fn channel(argb: u32, shift: u32) -> f32 {
        let c = f32::from(((argb >> shift) & 0xFF) as u8) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * channel(argb, 16) + 0.7152 * channel(argb, 8) + 0.0722 * channel(argb, 0)
}

/// WCAG 2.x contrast ratio of two colors, lighter luminance first:
/// `(L1 + 0.05) / (L2 + 0.05)`, spanning 1 (same color) to 21 (black on
/// white). Argument order never matters.
pub fn contrast_ratio(a: u32, b: u32) -> f32 {
    let (a, b) = (relative_luminance(a), relative_luminance(b));
    let (lighter, darker) = if a >= b { (a, b) } else { (b, a) };
    (lighter + 0.05) / (darker + 0.05)
}

/// The custom `card`/`text` pair's hard floor: WCAG 2.2 AA for 12-px
/// text, checked when the pair parses. Below it the file is rejected
/// whole (`settings` names both keys and the measured ratio), so
/// last-good settings hold — the floor is never a silent clamp.
pub const CONTRAST_FLOOR: f32 = 4.5;

/// One card polarity: the five colors an overlay render paints with.
/// `selected_bg`, `border`, and `muted` stay *derived* — the text color
/// at 20% / 18% / 55% over the card ([`COLOR_SELECTED_BG`] /
/// [`COLOR_BORDER`] for the dark card) — so a palette is two chosen
/// colors, three computed ones, and the polarity itself; the blend
/// system is the design system, and no variant can drift from it.
/// `is_dark` is the icon-tint polarity: symbolic icons cache per
/// polarity because their tint is the palette's text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    /// Card fill ([`COLOR_BACKGROUND`] on the dark card).
    pub background: u32,
    /// Title and glyph color ([`COLOR_TEXT`] on the dark card).
    pub text: u32,
    /// Selected-row band: text at 20% over the card.
    pub selected_bg: u32,
    /// Card hairline: text at 18% over the card.
    pub border: u32,
    /// Row hints (`Ctrl+N`): the text color at 55% over the card. Reads
    /// on both polarities and over the selection band, whose fill shifts
    /// by the selection's own 20% blend. Mid-band of the judged 50–70
    /// range; frozen on the preview sheet 2026-11-04, where the tightest
    /// read was muted-over-selection on the light card.
    pub muted: u32,
    /// Which polarity this card is — the icon cache's tint key.
    pub is_dark: bool,
}

impl Palette {
    /// The card Airlock has always painted: the named consts above,
    /// byte-identical to them.
    pub const fn dark() -> Palette {
        Palette {
            background: COLOR_BACKGROUND,
            text: COLOR_TEXT,
            selected_bg: COLOR_SELECTED_BG,
            border: COLOR_BORDER,
            muted: blend_over(COLOR_BACKGROUND, COLOR_TEXT, 55),
            is_dark: true,
        }
    }

    /// The light-mode card: a near-opaque warm-white fill with
    /// near-black text — the dark palette's inverse. Selection derives
    /// by the same 20% blend as the dark card; the border is 22%, two
    /// points *past* the band where the dark border sits two points
    /// *before* it (band minus one step on either polarity, the same
    /// 4-RGB-point gap so the two never read as one). One preview
    /// review round strengthened it from the symmetric 18%: against
    /// light wallpapers the card is only ~12 RGB points brighter than
    /// the backdrop, and the 18% hairline let the card's edges nearly
    /// dissolve — without a shadow, the stroke is the only separation
    /// this renderer has.
    pub const fn light() -> Palette {
        const BACKGROUND: u32 = 0xFA_F7_F4_F0;
        const TEXT: u32 = 0xFF_1E_21_26;
        Palette {
            background: BACKGROUND,
            text: TEXT,
            selected_bg: blend_over(BACKGROUND, TEXT, 20),
            border: blend_over(BACKGROUND, TEXT, 22),
            muted: blend_over(BACKGROUND, TEXT, 55),
            is_dark: false,
        }
    }

    /// Points the selection band at its source, resolved once at open
    /// beside the card: [`SelectionSetting::Accent`] blends the
    /// followed accent's `base` at 20 % over this card — a `None` read
    /// is the neutral lift, the band is never absent — and
    /// [`SelectionSetting::Neutral`] keeps the constructor's own
    /// text-at-20 % band, byte-identical to every card shipped before
    /// the follow. Followed alpha cannot leak: the accent arrives as
    /// straight `0xRRGGBB` and [`blend_over`] keeps the card's own
    /// near-opaque byte.
    pub fn with_selection(
        mut self,
        selection: SelectionSetting,
        accent_base: Option<u32>,
    ) -> Palette {
        let band_source = match selection {
            SelectionSetting::Neutral => self.text,
            SelectionSetting::Accent => accent_base.unwrap_or(self.text),
        };
        self.selected_bg = blend_over(self.background, band_source, 20);
        self
    }

    /// The custom card from the paired `card`/`text` keys: the two
    /// chosen colors as given, the three computed colors by the same
    /// blend system as the built-ins (band 20 %, muted 55 %, the border
    /// one step off the band per polarity — 18 % on a dark card, 22 %
    /// on a light one), and the icon-tint polarity from the card's own
    /// relative luminance (dark under 0.5). The alpha bytes are owned
    /// here — background `0xFA`, text `0xFF` — so the near-opaque
    /// contract holds whatever the pair. Infallible by construction:
    /// the contrast floor already rejected any pair that parses, and
    /// alpha is unreachable through the keys.
    pub fn custom(card: u32, text: u32) -> Palette {
        let background = 0xFA00_0000 | (card & 0x00FF_FFFF);
        let text = 0xFF00_0000 | (text & 0x00FF_FFFF);
        let is_dark = relative_luminance(background) < 0.5;
        let border = if is_dark { 18 } else { 22 };
        let palette = Palette {
            background,
            text,
            selected_bg: blend_over(background, text, 20),
            border: blend_over(background, text, border),
            muted: blend_over(background, text, 55),
            is_dark,
        };
        debug_assert!(palette.background >> 24 >= 0xFA);
        debug_assert!(palette.selected_bg >> 24 >= 0xFA);
        palette
    }

    /// The card the keys paint: a present pair wins over every
    /// [`ThemeSetting`] — the user's own card, polarity and all.
    /// Otherwise a pinned value wins regardless of the system; `Auto`
    /// follows the system's dark/light answer, and an unreadable one
    /// (`None`) paints dark — today's look, never a broken switch.
    pub fn resolve(
        setting: ThemeSetting,
        system_prefers_dark: Option<bool>,
        pair: Option<(u32, u32)>,
    ) -> Palette {
        if let Some((card, text)) = pair {
            return Palette::custom(card, text);
        }
        let dark = match setting {
            ThemeSetting::Dark => true,
            ThemeSetting::Light => false,
            ThemeSetting::Auto => system_prefers_dark.unwrap_or(true),
        };
        if dark {
            Palette::dark()
        } else {
            Palette::light()
        }
    }
}

/// Which card the overlay paints. `Auto` defers to the COSMIC system
/// dark/light preference, read per invocation; a missing or unreadable
/// system answer resolves to dark (today's look). The vocabulary lives
/// here beside [`Palette`] — the switcher's decision type, parsed by
/// `settings` from the config's `theme` key (the arrangement `Scope`
/// already uses: `switcher` owns the type, `settings` reads into it).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ThemeSetting {
    /// Follow the system dark/light preference (default).
    #[default]
    Auto,
    /// Today's card, pinned regardless of the system preference.
    Dark,
    /// The light card, pinned regardless of the system preference.
    Light,
}

/// What the selection band is made of. `Accent` (the default) follows
/// the COSMIC system accent — its `base` color at 20 % over the
/// effective card, the desktop's personality in the one place a glance
/// tracks it. `Neutral` is the 2026-11-04 lift it replaces: the text
/// color at 20 %, byte-identical to every card shipped before the
/// follow. Parsed by `settings` from the `selection` key, resolved once
/// at open beside the palette (the `ThemeSetting` arrangement).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SelectionSetting {
    /// Follow the system accent (default). A store miss falls back to
    /// the neutral lift, so the band is never absent.
    #[default]
    Accent,
    /// The text color at 20 % over the card — the shipped band.
    Neutral,
}

/// Which way the card lays its list out. `Vertical` (the default) is
/// today's card — the tall column of rows, byte-identical. `Horizontal`
/// is the wide, short band of window tiles, centered. Parsed by
/// `settings` from the `layout` key, resolved once at open and frozen
/// like the palette (the `ThemeSetting` arrangement).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LayoutSetting {
    /// Today's column of rows (default).
    #[default]
    Vertical,
    /// The horizontal band of tiles.
    Horizontal,
}

/// The horizontal card's tile budget. `Auto` (the default) fits as many
/// tiles as a fraction of the invoking output's logical width allows —
/// read at map, frozen for the session; `Pinned(n)` fixes the budget at
/// [`COLUMNS_MIN`]..=[`COLUMNS_MAX`] tiles, card width = n × pitch.
/// Parsed by `settings` from the `columns` key; binds
/// [`LayoutSetting::Horizontal`] only — the vertical card ignores it,
/// `rows` never constrains the band.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ColumnsSetting {
    /// Fit tiles to the invoking output (default).
    #[default]
    Auto,
    /// A fixed tile budget, `COLUMNS_MIN..=COLUMNS_MAX`.
    Pinned(usize),
}

/// One card-width key's answer. `Auto` (the default) keeps the
/// content-sized card — the widest title row on the vertical card, the
/// tile shape on the band; `Px(n)` floors the card at n logical px;
/// `Percent(n)` at n % of the invoking output's logical width (`"100%"`
/// the whole output), the percentage parser-guaranteed ≤ 100. Parsed by
/// `settings` from the `width`/`min-width` keys; both layouts read them,
/// and neither ever shrinks the card below its content — the floor only
/// widens, so nothing clips mid-gesture.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WidthSetting {
    /// The content-sized card (default).
    #[default]
    Auto,
    /// A fixed width floor, logical px.
    Px(u32),
    /// A share of the invoking output's logical width, 1..=100.
    Percent(u32),
}

/// The text backend one card paints through, chosen once at session
/// open beside the palette and frozen for the gesture's life (the
/// palette freeze philosophy): `Bitmap` is the font8x8 floor — today's
/// paint, byte-identical, and the zero-fonts fallback — and `System`
/// shapes and rasterizes through the daemon's
/// [`crate::text::TextSystem`] in the family the session resolved.
/// The model builders and the renderer take the same enum, so a card's
/// titles are composed, truncated, and painted under one backend end
/// to end.
pub enum TextBackend<'a> {
    /// The font8x8 paint: folded titles, column caps, the hand-drawn
    /// `…` — every byte as shipped.
    Bitmap,
    /// The system stack: real glyphs, fallback chains, pixel budgets,
    /// the font's own `…`. `query` is the family the session froze
    /// (`FontQuery::SansSerif` is the default face — the interface-font
    /// follow resolves before this, at open).
    System {
        system: &'a mut crate::text::TextSystem,
        query: crate::text::FontQuery<'a>,
    },
}

/// The backend a frozen [`crate::text::TextDecision`] paints through,
/// borrowing the daemon's [`crate::text::TextSystem`]: the floor maps
/// to [`TextBackend::Bitmap`], the system decision to
/// [`TextBackend::System`] in its resolved family. The one mapping —
/// the model builders and the renderer share it, so a card is composed
/// and painted under one decision end to end.
pub fn text_backend<'a>(
    decision: &'a crate::text::TextDecision,
    system: &'a mut crate::text::TextSystem,
) -> TextBackend<'a> {
    match decision {
        crate::text::TextDecision::Bitmap => TextBackend::Bitmap,
        crate::text::TextDecision::System { family } => TextBackend::System {
            system,
            query: crate::text::FontQuery::Named(family),
        },
    }
}

// Geometry pins. The 8-grid holds for every stepped metric; the row is
// exactly the icon cell plus its breathing; rows stay clear of the
// corner arcs, so a selection band can never clip a corner, and the
// band's own ends round within its height; and the border is a
// hairline, not a frame. Since the geometry follow (GE2) the
// rhythm/radius pins run parametrized over every style × density
// profile the desktop can resolve — the values are derived ([`rhythm`],
// [`style_radius`]), so every combination must hold, not just today's
// row; the constants themselves stay pinned to the default profile by
// the const-equality test.
const _: () = {
    // The default text profile is the constants block's own 12-px cell.
    assert!(DEFAULT_FONT_SIZE == TEXT_W);
    assert!(FONT_SIZE_MIN > 0 && FONT_SIZE_MIN <= DEFAULT_FONT_SIZE);
    assert!(DEFAULT_FONT_SIZE <= FONT_SIZE_MAX);
    // The title band's physical budget is the default's 48 × 12.
    assert!(TITLE_BAND_PX == MAX_COLS * TEXT_W);
    // The band's default-profile helpers are the constants block's own
    // values, so every runtime `_at` call passes the same numbers.
    assert!(TILE_CELL_H == tile_cell_h_at(TEXT_H));
    assert!(TILE_CELL_H == TEXT_H + TILE_PARENT_GAP + TILE_ICON + PIP_GAP + PIP_SIDE);
    assert!(band_card_height() == 168);
    assert!(band_strip_top() == TILE_PAD_Y + TEXT_H + CAPTION_GAP);
    assert!(band_icon_top() == band_strip_top() + TEXT_H + TILE_PARENT_GAP);
    assert!(TEXT_H == GLYPH_H * FONT_SCALE_NUM / FONT_SCALE_DEN);
    assert!(TEXT_W == TEXT_H);
    // The glyph block is whole device pixels at the render-scale floor.
    assert!((TEXT_W * 2).is_multiple_of(GLYPH_W));
    // The title band stays under the ~600-px switcher-width consensus
    // (research record §2.6, §2.10); the hint band rides on top of it.
    assert!(MAX_COLS * TEXT_W <= 600);
    assert!(ICON.is_multiple_of(8));
    assert!(ICON_GAP.is_multiple_of(4));
    assert!(HINT_GAP.is_multiple_of(4));
    // A parent row's disclosure caret and its clearing fit the
    // CHEVRON_COLS budget — the render guard is defense in depth, this
    // is the reservation's truth. (The count pips sit below the title
    // and spend no title width.)
    assert!(CHEVRON_GAP + GLYPH_W * CHEVRON_SCALE <= CHEVRON_COLS * TEXT_W);
    // The × badge, shared by both cards: the 8-px ink keeps 2 px of
    // knockout on every side, the hit territory forgives the small
    // target, and the whole badge stays on its icon's corner.
    assert!(CLOSE_BADGE.is_multiple_of(4));
    assert!(CLOSE_BADGE_HIT.is_multiple_of(4));
    assert!(GLYPH_W * CLOSE_BADGE_SCALE + 2 * 2 == CLOSE_BADGE);
    assert!(CLOSE_BADGE_RADIUS < CLOSE_BADGE);
    assert!(CLOSE_BADGE_HIT >= CLOSE_BADGE);
    assert!(CLOSE_BADGE_HIT <= ICON && CLOSE_BADGE_HIT <= TILE_ICON);
    assert!(STRIP_PAD_Y.is_multiple_of(4));
    assert!(STRIP_CARET_GAP.is_multiple_of(4));
    assert!(STRIP_CARET_W == 2);
    assert!(PAD_X.is_multiple_of(4));
    assert!(BORDER == 1);
    assert!(SELECT_INSET.is_multiple_of(4));
    assert!(SELECT_INSET > BORDER);
    assert!(SELECT_INSET < PAD_X);
    assert!(ROWS_MIN >= 3);
    assert!(ROWS_MIN <= DEFAULT_ROWS && DEFAULT_ROWS <= ROWS_MAX);

    // Every profile the desktop can resolve: three styles × three
    // densities. `while` loops — const evaluation has no `for`.
    let styles = [Style::Round, Style::SlightlyRound, Style::Square];
    let densities = [Density::Compact, Density::Standard, Density::Spacious];
    let mut si = 0;
    while si < styles.len() {
        let mut di = 0;
        while di < densities.len() {
            let profile = StyleProfile::resolve(styles[si], densities[di]);
            let rhythm = profile.rhythm;
            // The default text size lands exactly on the icon-driven
            // density rows; the row grows only past 12 px.
            assert!(rhythm.with_text(DEFAULT_FONT_SIZE).row_h == rhythm.row_h);
            // The row is the icon cell plus its breathing at every
            // density (the icon-driven rule survives).
            assert!(rhythm.row_h == ICON + 2 * rhythm.row_pad_y);
            assert!(rhythm.row_h >= TEXT_H);
            // The band clears the card's corner arcs. The pinned era's
            // `PAD_Y ≥ RADIUS` was a sufficient condition while pad_y
            // was a pinned 16; Compact's derived 8 spends the band's
            // inset instead — the exact sufficient condition is
            // `pad_y + SELECT_INSET ≥ radius` (the sampled proof, over
            // the band's own corner arc, runs in the tests).
            assert!(rhythm.pad_y + SELECT_INSET >= profile.radius);
            // The 8-grid per profile — 4-multiples, with the two
            // ratified exceptions: Square's 2 (OQ 1 — a hair of
            // rounding where 0 would read as a bug) and Compact's
            // row steps (OQ 2, the half-grid).
            assert!(profile.radius == 2 || profile.radius.is_multiple_of(4));
            assert!(profile.band_radius == 2 || profile.band_radius.is_multiple_of(4));
            assert!(rhythm.row_pad_y.is_multiple_of(4));
            assert!(rhythm.row_gap.is_multiple_of(4));
            assert!(rhythm.pad_y.is_multiple_of(4));
            // The band's own ends round within its height.
            assert!(2 * profile.band_radius <= rhythm.row_h);
            // The card fits the panel at this density's clamped ceiling
            // (the fit is by construction — [`effective_rows`] — these
            // pins hold it at the worst case; Spacious binds at a
            // budget of 8, Standard's 696 and Compact's 540 never do).
            let budget = effective_rows(densities[di], ROWS_MAX);
            assert!(card_height_rhythm(rhythm, (budget + ROWS_SLACK) as u32) <= CARD_FIT_PX);
            di += 1;
        }
        si += 1;
    }
    // The default-budget worst case (584, Standard's row) — the
    // overlay-density pin, unchanged.
    assert!(card_height((DEFAULT_ROWS + ROWS_SLACK) as u32) <= CARD_FIT_PX);
};

/// Placeholder shown when a window has neither title nor app_id.
const UNTITLED: &str = "(untitled)";

/// Scale of the fallback glyph, in font cells: 24 logical px, close
/// enough to the 32 px icon cell that an iconless row holds its weight
/// next to themed icons (16 px read as underweight in the preview review).
pub const GLYPH_SCALE: u32 = 3;

/// Generic window glyph, the icon-cell fallback: an 8×8 frame with a
/// title-bar divider, drawn in the palette's text color scaled to
/// `GLYPH_SCALE × 8` logical px and centered in the cell. One glyph
/// vocabulary for every fallback row — the icon column belongs to the
/// apps, the glyph to "no icon".
const WINDOW_GLYPH: [u8; 8] = [0xFF, 0x81, 0x81, 0xFF, 0x81, 0x81, 0x81, 0xFF];

/// Hand-drawn ellipsis for the truncation marker — the one intentional
/// non-ASCII glyph (research record §5.2.3 says elide with `…`; font8x8
/// has none, so this is it). Dots on the period's baseline, columns
/// 1/3/5.
const ELLIPSIS_GLYPH: [u8; 8] = [0x00, 0x00, 0x00, 0x00, 0x00, 0x2A, 0x2A, 0x00];

/// Middle dot for `·`/`•` separators (GNOME-style `file.py · src` titles)
/// — same hand-drawn family as the ellipsis: one 2×2 dot at the x-height
/// middle, where a middle dot belongs rather than on the period's
/// baseline.
const MIDDLE_DOT_GLYPH: [u8; 8] = [0x00, 0x00, 0x00, 0x18, 0x18, 0x00, 0x00, 0x00];

/// Disclosure caret, collapsed (points right) — the grouped parent's
/// expansion mark, drawn at [`CHEVRON_SCALE`] in the text color. A
/// two-pixel-stroke caret, not the filled triangle of the first
/// grouping pass: inline on the title line the caret reads as tree
/// structure, where the solid wedge read as a media control.
const CHEVRON_RIGHT_GLYPH: [u8; 8] = [0x00, 0x0C, 0x18, 0x30, 0x30, 0x18, 0x0C, 0x00];

/// Disclosure caret, expanded (points down) — the right glyph rotated a
/// quarter turn and centered on the cell.
const CHEVRON_DOWN_GLYPH: [u8; 8] = [0x00, 0x00, 0x42, 0x66, 0x3C, 0x18, 0x00, 0x00];

/// The close affordance's × (`close-window`): two 2-px diagonal arms
/// meeting at a 2×2 center — the chevron family's solid weight, hand
/// drawn like every glyph in the vocabulary, never a typed character.
const CLOSE_GLYPH: [u8; 8] = [0xC3, 0xE7, 0x66, 0x3C, 0x3C, 0x66, 0xE7, 0xC3];

/// The × badge's side, logical px — a 12-px knockout at the icon's
/// top-right corner, shared by the vertical rows and the horizontal
/// tiles so the pointer affordance is identical in either layout. The
/// knockout (the card background) separates the mark from a colorful
/// icon, so it reads on every app, and the mark stays secondary to the
/// thumbnail it closes.
const CLOSE_BADGE: u32 = 12;
/// The badge's corner radius, logical px — one third of its side, the
/// small-chip rounding that keeps the knockout from reading as a
/// square hole cut in the icon.
const CLOSE_BADGE_RADIUS: u32 = 4;
/// The × ink's draw scale inside the badge: the 8×8 glyph at 1×
/// (8 logical px), leaving 2 px of knockout on every side.
const CLOSE_BADGE_SCALE: u32 = 1;
/// The badge's click territory, logical px — the 12-px badge plus 2 px
/// of forgiveness on its left and bottom edges, so the small target
/// stays reachable without widening what the eye reads as the button.
const CLOSE_BADGE_HIT: u32 = 16;

/// The number chord a row advertises in its hint band — the single
/// source of "which N selects this row" for both the renderer (the
/// painted digit) and the input map (the wired key): Ctrl+N addresses
/// level-1 entries, and a row with no hint (`None`, rows past nine,
/// continuation rows, and — since rev 5 — every group child) is not
/// number-addressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowHint {
    /// `Ctrl+N` — the Nth level-1 entry (1-based), flat rows included
    /// (the flat card's every-row numbering is the same rule over a
    /// one-level tree).
    Ctrl(usize),
}

impl RowHint {
    /// The Ctrl+N chord for the level-1 entry at `index` (0-based):
    /// `None` past the ninth — the tenth entry is not number-addressed,
    /// and no chord is ever shown for it.
    fn ctrl(index: usize) -> Option<Self> {
        (index < MAX_HINTS).then_some(RowHint::Ctrl(index + 1))
    }

    /// The label both cards paint: the bare digit, `1`..`9`. "Ctrl+9"
    /// is six bitmap columns against the one column the hint band
    /// reserves (and the four a 64-px tile funds) — glance-tiles G2,
    /// 2026-11-09; the vertical band followed on 2026-11-09 when the
    /// full chord repeated on every row read as a table column. The
    /// digit stays wired to the same reading-order ordinal.
    fn digit(self) -> String {
        match self {
            RowHint::Ctrl(n) => n.to_string(),
        }
    }
}

/// One rendered row: a title, an optional app icon, and whether it is
/// the selection. `icon: None` draws the generic window glyph. A
/// continuation row at a truncated edge is the same shape with `handle:
/// None` and nothing else — the missing handle is what makes every input
/// guard (hover, click, Ctrl+N) treat it as inert.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RowModel {
    pub title: String,
    pub icon: Option<Rc<IconBitmap>>,
    pub selected: bool,
    /// The window this row renders — the truth a pointer click selects.
    /// The renderer ignores it; hit-testing reads it. `None` marks a
    /// continuation row. A grouped parent row carries its app's
    /// MRU-front window, so every row-addressed input resolves to
    /// exactly one window whatever the mode.
    pub handle: Option<Handle>,
    /// The row's place in the grouped tree; [`RowKind::Leaf`] on every
    /// flat row. Drives the child indent, the parent's count pips and
    /// caret, and nothing else — input addressing reads `handle`.
    pub kind: RowKind,
    /// On a parent row, the app's window count the pips show; zero
    /// everywhere else.
    pub count: usize,
    /// The number chord the hint band paints for this row; `None` on
    /// continuation rows and rows past nine (see [`RowHint`]). Set by
    /// the model builders — the position-to-chord rule is theirs, the
    /// render and the input map only read it.
    pub hint: Option<RowHint>,
    /// A parent row's expansion state — the disclosure triangle's
    /// right (collapsed) / down (expanded) direction. Meaningless on
    /// every other kind.
    pub expanded: bool,
}

/// What the overlay should draw: the window's rows plus the logical
/// pixel size the surface should have (the renderer multiplies by the
/// render scale for device pixels).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OverlayModel {
    pub rows: Vec<RowModel>,
    pub width: u32,
    pub height: u32,
    /// The window these rows show — what a session carries into the
    /// next rebuild as [`visible_window`]'s `prev`, keeping the sticky
    /// rule moving.
    pub viewport: Viewport,
    /// The shape these rows rendered under — the style/density profile
    /// frozen at open. The renderer and the pointer's `row_at` read it
    /// back, so what a click addresses is always the geometry the user
    /// saw, whatever the desktop did mid-gesture.
    pub profile: StyleProfile,
    /// The axis the entries lay on: `false` (the default) paints the
    /// vertical column of rows, `true` the horizontal band of tiles.
    /// The renderer and the pointer's `row_at`/`cell_at` dispatch read
    /// it back — hit-testing always addresses the geometry the user
    /// saw (the `profile` contract, one axis over).
    pub horizontal: bool,
    /// The type-to-filter query this card is narrowed by (`type-filter`)
    /// — `None` on every unfiltered card, so the strip and its height
    /// exist exactly while a query stands and zero-query cards stay
    /// byte-identical.
    pub query: Option<String>,
    /// The row the pointer rests on (`close-window`'s × reveal) —
    /// presence, not selection: the daemon sets it from hover
    /// observations, validated against the rows this model just built,
    /// and the renderer paints the close badge on the row's icon —
    /// both cards share the badge, on the icon's top-right corner —
    /// when the row is a window row (leaves, children, query rows —
    /// never headers or continuations). `None` on every pointerless
    /// card, so nothing paints without pointer presence.
    pub hovered: Option<usize>,
    /// The one caption (glance-tiles G3, 2026-11-09): the commit
    /// target's title — the selected window tile's own title, an app
    /// tile's MRU-front window's title, the flat band's selected
    /// window — composed through `display_title` and truncated to the
    /// card's caption budget with the real `…`. The horizontal cards
    /// paint it fixed top-center, full text color, never moving with
    /// the selection; the line is reserved on every frame whether or
    /// not this holds text. Empty on every vertical card (they paint
    /// nothing of it).
    pub caption: String,
    /// The workspace label (workspace-label W2, 2026-11-09): the
    /// invoking workspace's name — or its synthesized "# Workspace N"
    /// — truncated to [`WS_LABEL_MAX_COLS`] with the real `…`. Empty on
    /// every default-off card; when it stands, each card reserves its
    /// line — above the first row on the vertical ones, below the
    /// tiles on the horizontal ones. Written once at open, through
    /// [`with_workspace`], and frozen with the session.
    pub workspace: String,
}

impl OverlayModel {
    /// The row whose band contains the device-pixel point (`x`, `y`) —
    /// a surface-local logical pointer position × render `scale`. The
    /// model and its row rhythm are logical; only buffers are device
    /// pixels, so the point scales down to the model's space. The
    /// footprint is the full row width (the card itself); gap seams,
    /// card padding, and points off the card are `None`.
    pub fn row_at(&self, x: f64, y: f64, scale: u32) -> Option<usize> {
        let in_card = x >= 0.0
            && y >= 0.0
            && x < self.width as f64 * scale as f64
            && y < self.height as f64 * scale as f64;
        if !in_card {
            return None;
        }
        // The rhythm the model rendered under, not the default's. The
        // query strip, when one stands, shifts every row down by its
        // height — the strip zone itself is the top padding's twin: no
        // row, no selection. The workspace label's line, when one
        // stands, sits above the strip and shifts with the same rule.
        let rhythm = self.profile.rhythm;
        let strip = strip_height_at(&self.query, rhythm, self.profile.text.cell()) as f64;
        let label = if self.workspace.is_empty() {
            0.0
        } else {
            ws_label_height_at(rhythm, self.profile.text.cell()) as f64
        };
        let y_px = y / scale as f64 - rhythm.pad_y as f64 - label - strip;
        let pitch = (rhythm.row_h + rhythm.row_gap) as f64;
        if y_px < 0.0 || y_px >= self.rows.len() as f64 * pitch {
            return None; // card padding above the first row / below the last
        }
        let row = (y_px / pitch) as usize;
        (y_px - row as f64 * pitch < rhythm.row_h as f64).then_some(row) // gap seam
    }

    /// The tile whose cell contains the device-pixel point (`x`, `y`) —
    /// the horizontal twin of [`OverlayModel::row_at`]: a surface-local
    /// logical pointer position × render `scale`, the point scaled down
    /// to the model's space. The footprint is the full band height
    /// (the cell is the unit) below the reserved caption line — the
    /// caption's line and gap are the top padding's twin (the query
    /// strip's rule, one axis over): no tile paints there, so nothing
    /// is addressed from there (glance-tiles G2, 2026-11-09). Tile
    /// gaps, card padding, and points off the card are `None`.
    pub fn cell_at(&self, x: f64, y: f64, scale: u32) -> Option<usize> {
        let in_card = x >= 0.0
            && y >= 0.0
            && x < self.width as f64 * scale as f64
            && y < self.height as f64 * scale as f64;
        if !in_card {
            return None;
        }
        let cell = self.profile.text.cell();
        let strip_top = band_strip_top_at(cell) as f64 * scale as f64;
        // The label's own bottom line (workspace-label W7) bounds the
        // band from below: tiles live in [strip_top, strip_top + cell),
        // and the clearing and the label line under it address nothing —
        // like the caption line above, no tile from there.
        let strip_bottom = strip_top
            + (tile_cell_h_at(cell)
                + if self.workspace.is_empty() {
                    0
                } else {
                    WS_LABEL_GAP
                }) as f64
                * scale as f64;
        if y < strip_top || y >= strip_bottom {
            return None; // the caption line, the clearing, or the label line
        }
        // The single band admits every y inside the tile cell; x alone
        // picks the tile.
        let x_px = x / scale as f64 - TILE_PAD_X as f64;
        let pitch = (TILE_W + TILE_GAP) as f64;
        if x_px < 0.0 || x_px >= self.rows.len() as f64 * pitch {
            return None; // card padding before the first tile / past the last
        }
        let cell = (x_px / pitch) as usize;
        (x_px - cell as f64 * pitch < TILE_W as f64).then_some(cell) // gap seam
    }

    /// The window the Nth *content* tile in reading order resolves to
    /// (0-based) — continuation affordances skipped, left→right — the
    /// address a horizontal card's Ctrl+N chords resolve against at
    /// event receipt. The hint ordinals number the same sequence, so
    /// every displayed hint is wired. `None` past the content tiles.
    pub fn content_tile_handle(&self, n: usize) -> Option<Handle> {
        self.rows
            .iter()
            .filter(|row| row.handle.is_some())
            .nth(n)
            .and_then(|row| row.handle)
    }

    /// The row whose close target contains the device-pixel point (`x`,
    /// `y`) — `close-window`'s click territory. Both axes use the same
    /// × badge on the icon's top-right corner: a vertical row's icon
    /// cell (the same footprint [`OverlayModel::row_at`] resolves, the
    /// profile's rhythm, the query strip's shift, the child indent) or
    /// a horizontal tile's icon cell (via [`OverlayModel::cell_at`]).
    /// Both gate to window rows: a group header never offers a close,
    /// and a continuation row has no window to close. The boundary
    /// evaluates this at the press and closes the row's own event-time
    /// handle.
    pub fn close_zone_at(&self, x: f64, y: f64, scale: u32) -> Option<usize> {
        if self.horizontal {
            let tile = self.cell_at(x, y, scale)?;
            let left = TILE_PAD_X + tile as u32 * (TILE_W + TILE_GAP);
            let (zone_left, zone_top, zone_right, zone_bottom) =
                close_tile_zone_at(left, self.profile.text.cell());
            let logical_x = x / scale as f64;
            let logical_y = y / scale as f64;
            if logical_x < zone_left as f64
                || logical_x >= zone_right as f64
                || logical_y < zone_top as f64
                || logical_y >= zone_bottom as f64
            {
                return None; // the tile's icon and title land, or its margins
            }
            let row_model = self.rows.get(tile)?;
            return closeable(row_model).then_some(tile);
        }
        let row = self.row_at(x, y, scale)?;
        let row_model = self.rows.get(row)?;
        if !closeable(row_model) {
            return None;
        }
        let (zone_left, zone_top, zone_right, zone_bottom) = close_row_zone(self, row)?;
        let logical_x = x / scale as f64;
        let logical_y = y / scale as f64;
        (logical_x >= zone_left as f64
            && logical_x < zone_right as f64
            && logical_y >= zone_top as f64
            && logical_y < zone_bottom as f64)
            .then_some(row)
    }
}

/// Whether a row offers the × (`close-window`): a window row — a leaf,
/// a group child, or a query row — whose handle resolves the window it
/// closes. Group headers close their app's front window through
/// Alt+minus but never reveal a pointer target, and a continuation row
/// has no window at all.
fn closeable(row: &RowModel) -> bool {
    row.handle.is_some() && row.kind != RowKind::Parent
}

/// The × badge's rectangle over an icon cell, logical px: the icon's
/// top-right corner, where the mark sits on the thumbnail it closes.
/// Both cards use this one badge — a vertical row's 32-px icon cell and
/// a horizontal tile's 48-px icon cell — so the affordance is identical
/// in either layout.
fn close_badge_rect(icon_left: u32, icon_top: u32, icon_side: u32) -> (u32, u32, u32, u32) {
    let icon_right = icon_left + icon_side;
    (
        icon_right - CLOSE_BADGE,
        icon_top,
        icon_right,
        icon_top + CLOSE_BADGE,
    )
}

/// The ×'s click territory over an icon cell, logical px — the badge's
/// top-right anchor with [`CLOSE_BADGE_HIT`] of forgiveness on the left
/// and bottom edges, so the small target stays reachable. The badge
/// paints inside this rectangle.
fn close_zone_rect(icon_left: u32, icon_top: u32, icon_side: u32) -> (u32, u32, u32, u32) {
    let icon_right = icon_left + icon_side;
    (
        icon_right - CLOSE_BADGE_HIT,
        icon_top,
        icon_right,
        icon_top + CLOSE_BADGE_HIT,
    )
}

/// The ×'s click territory on a horizontal tile, logical px — the
/// tile's icon cell, centred in the slot, at the default text size.
#[allow(dead_code)] // the default-profile seam for tests; the daemon uses `_at`
fn close_tile_zone(left: u32) -> (u32, u32, u32, u32) {
    close_tile_zone_at(left, TEXT_H)
}

/// [`close_tile_zone`] at one text size: the tile's caption line moves
/// the icon line, so the badge's anchor moves with it.
fn close_tile_zone_at(left: u32, cell: u32) -> (u32, u32, u32, u32) {
    close_zone_rect(
        left + (TILE_W - TILE_ICON) / 2,
        band_icon_top_at(cell),
        TILE_ICON,
    )
}

/// The icon cell's top-left corner on a vertical row, logical px — the
/// row rhythm, the query strip's shift, and the child indent, the same
/// geometry the renderer paints the icon under. `None` past the model's
/// rows.
fn row_icon_origin(model: &OverlayModel, row: usize) -> Option<(u32, u32)> {
    let row_model = model.rows.get(row)?;
    let rhythm = model.profile.rhythm;
    let strip = strip_height_at(&model.query, rhythm, model.profile.text.cell());
    let top = rhythm.pad_y + strip + row as u32 * (rhythm.row_h + rhythm.row_gap);
    let indent = (row_model.kind == RowKind::Child) as u32 * TREE_INDENT;
    Some((PAD_X + indent, top + (rhythm.row_h - ICON) / 2))
}

/// The ×'s click territory on a vertical row, logical px — the same
/// badge anchor over the row's icon cell.
fn close_row_zone(model: &OverlayModel, row: usize) -> Option<(u32, u32, u32, u32)> {
    let (icon_left, icon_top) = row_icon_origin(model, row)?;
    Some(close_zone_rect(icon_left, icon_top, ICON))
}

/// Paints the × badge over an icon already blitted beneath it
/// (`close-window`): a rounded knockout in the card's own background,
/// so the mark reads on a colorful thumbnail, with the 8-px × cut in
/// the text color. Both layouts paint this one badge, which is what
/// keeps the pointer affordance identical in either direction.
fn paint_close_badge(
    canvas: &mut [u8],
    stride: u32,
    scale: u32,
    icon_left: u32,
    icon_top: u32,
    icon_side: u32,
    palette: &Palette,
) {
    let (badge_x, badge_y, _, _) = close_badge_rect(icon_left, icon_top, icon_side);
    // The knockout: the card background over the icon, antialiased on
    // the badge's corner arcs like every shape in the vocabulary.
    for ly in badge_y..badge_y + CLOSE_BADGE {
        for lx in badge_x..badge_x + CLOSE_BADGE {
            let cov = shape_cov(
                lx - badge_x,
                ly - badge_y,
                CLOSE_BADGE,
                CLOSE_BADGE,
                CLOSE_BADGE_RADIUS,
                0,
            );
            if cov == 0 {
                continue;
            }
            for dy in 0..scale {
                let y = ly * scale + dy;
                for x in lx * scale..(lx + 1) * scale {
                    let index = y as usize * stride as usize + x as usize * 4;
                    if let Some(slot) = canvas.get_mut(index..index + 4) {
                        let dst = u32::from_le_bytes(slot.try_into().expect("pixel is four bytes"));
                        let out = mix_pixel(dst, palette.background, cov);
                        slot.copy_from_slice(&out.to_le_bytes());
                    }
                }
            }
        }
    }
    let glyph_x = badge_x + (CLOSE_BADGE - GLYPH_W * CLOSE_BADGE_SCALE) / 2;
    let glyph_y = badge_y + (CLOSE_BADGE - GLYPH_H * CLOSE_BADGE_SCALE) / 2;
    draw_glyph(
        canvas,
        stride,
        CLOSE_GLYPH,
        glyph_x * scale,
        glyph_y * scale,
        CLOSE_BADGE_SCALE * scale,
        palette.text,
    );
}

/// The slice of a long eligible list one overlay card shows: where the
/// window starts, how many content rows it holds, and whether a
/// continuation row replaces the outermost slot at each truncated edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Viewport {
    /// The list index the window's first slot covers — a continuation
    /// row when `top`, a content row otherwise.
    pub start: usize,
    /// Content rows in the window: its slots that are not continuations.
    pub content: usize,
    /// Rows are hidden above the window: its first slot is an `…` row.
    pub top: bool,
    /// Rows are hidden below the window: its last slot is an `…` row.
    pub bottom: bool,
}

/// The two inputs to the windowing rule, carried together into
/// [`overlay_model`]: the session's `rows` budget (frozen at open from
/// the setting), the card's resolved width floor (frozen at open from
/// the `width`/`min-width` settings; `None` sizes from content), and
/// the window the previous rebuild computed, `None` at open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Windowing {
    pub rows: usize,
    pub width_floor: Option<u32>,
    pub prev: Option<Viewport>,
}

/// The horizontal band's windowing inputs, carried into
/// [`overlay_model_horizontal`]: the session's resolved tile budget
/// (frozen at open from the `columns` setting and the invoking
/// output's logical width), the card's resolved width floor (frozen at
/// open beside it; `None` sizes from the tile shape), and the window
/// the previous rebuild computed, `None` at open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Banding {
    pub tiles: usize,
    pub width_floor: Option<u32>,
    pub prev: Option<Viewport>,
}

/// Resolves one card-width key against the invoking output's logical
/// width: `Auto` stays `None` (the content-sized card), `Px(n)` is `n`
/// verbatim, `Percent(n)` is `output × n / 100` (whole logical px,
/// parser-guaranteed ≤ 100 so never past the output). Like
/// [`resolved_tiles`]: read once at map and frozen for the session.
/// The caller owns the one clamp left — a pixel count past the output
/// rides back down to it with the daemon's diagnostic.
pub fn resolved_width(setting: WidthSetting, output_width: u32) -> Option<u32> {
    match setting {
        WidthSetting::Auto => None,
        WidthSetting::Px(px) => Some(px),
        // Saturating: the output's width comes from the compositor, and
        // an implausible one must not panic the gesture path.
        WidthSetting::Percent(percent) => Some(output_width.saturating_mul(percent) / 100),
    }
}

/// Resolves the `columns` setting against the invoking output's
/// logical width: `Pinned` is the budget verbatim (parser-guaranteed
/// [`COLUMNS_MIN`]..=[`COLUMNS_MAX`]), `Auto` fits
/// `floor(AUTO_FRACTION × width / tile pitch)` tiles — floored at
/// [`COLUMNS_MIN`], uncapped above (a ceiling is the pinned form's
/// contract; auto's is the output). Read once at map and frozen for
/// the session, like every session-scoped value.
pub fn resolved_tiles(columns: ColumnsSetting, output_width: u32) -> usize {
    match columns {
        ColumnsSetting::Pinned(count) => count,
        ColumnsSetting::Auto => {
            let pitch = TILE_W + TILE_GAP;
            let budget = output_width * AUTO_FRACTION_NUM / AUTO_FRACTION_DEN;
            (budget / pitch).max(COLUMNS_MIN as u32) as usize
        }
    }
}

/// The window over a `len`-long eligible list with `sel` selected,
/// keeping `rows` windows in view, continuing from `prev` — `None` is a
/// fresh session, top-aligned.
///
/// A list of at most `rows + ROWS_SLACK` windows shows whole; a longer
/// one holds `rows + ROWS_SLACK` slots, so the `…` affordance at each
/// truncated edge rides on top of the budget and `rows` windows stay
/// content mid-list, `rows + 1` at either end.
///
/// The window moves the minimum needed to keep the selection on a
/// *content* row, never a continuation slot: nothing while the selection
/// stays interior (the sticky rule — the walking fill keeps its motion
/// vocabulary), exactly one row when stepping past an edge. The starts
/// keeping `sel` content-visible form one interval, so the rule is a
/// clamp of the carried start into it; the interval's upper end widens
/// at the last window, whose bottom edge hides nothing. A start from a
/// longer list re-clamps after a mid-gesture shrink.
pub fn visible_window(len: usize, sel: usize, rows: usize, prev: Option<Viewport>) -> Viewport {
    debug_assert!(sel < len, "the caller's selection indexes the list");
    // The slots this list gets: all of it within the budget plus slack,
    // the budget plus one slot per truncatable edge beyond that.
    let visible = if len > rows {
        (rows + ROWS_SLACK).min(len)
    } else {
        len
    };
    let max_start = len - visible;
    // The smallest and largest starts keeping `sel` on a content row.
    let lo = sel.saturating_add(2).saturating_sub(visible).min(max_start);
    let hi = if sel == 0 {
        0
    } else {
        (sel - 1).min(max_start)
    };
    let start = prev.map_or(0, |vp| vp.start).min(max_start).clamp(lo, hi);
    let (top, bottom) = (start > 0, start + visible < len);
    Viewport {
        start,
        content: visible - top as usize - bottom as usize,
        top,
        bottom,
    }
}

/// The horizontal band's window over a `len`-long eligible list with
/// `sel` selected, under a `tiles`-tile budget, continuing from `prev`
/// — `None` is a fresh session, left-aligned on the MRU head. The
/// one-axis mirror of [`visible_window`] (CL2): same clamp-of-carried-
/// start shape, different slot accounting.
///
/// The band is `min(len, tiles)` tiles wide, never wider — the spec's
/// "card width = N × tile pitch": the `…` affordance lives *inside*
/// the budget, replacing the trailing tile when the window's end hides
/// windows (unlike the vertical card, whose `…` rows ride on top of
/// its `rows`). A truncated band shows `tiles − 1` windows; the tail
/// window, whose end hides nothing, shows all `tiles`. There is no
/// leading affordance — the list reads left to right, so only a
/// truncated *trailing* edge gets one.
///
/// The window moves the minimum needed to keep the selection on a
/// *content* tile, never the affordance slot: nothing while the
/// selection stays interior (the sticky rule), one tile per step past
/// the trailing edge. A start carried from a longer list re-clamps
/// after a mid-gesture shrink.
pub fn band_window(len: usize, sel: usize, tiles: usize, prev: Option<Viewport>) -> Viewport {
    debug_assert!(sel < len, "the caller's selection indexes the list");
    debug_assert!(
        tiles >= 2,
        "a band needs a content tile beside the affordance"
    );
    // The last start whose window's end hides nothing: the tail window.
    let max_start = len.saturating_sub(tiles);
    // The smallest and largest starts keeping `sel` on a content tile:
    // no leading slot, so the window may sit flush on the selection.
    let lo = sel.saturating_add(2).saturating_sub(tiles).min(max_start);
    let hi = sel.min(max_start);
    let start = prev.map_or(0, |vp| vp.start).min(max_start).clamp(lo, hi);
    let bottom = start + tiles < len;
    Viewport {
        start,
        content: if bottom { tiles - 1 } else { len.min(tiles) },
        top: false,
        bottom,
    }
}

/// Builds the flat overlay contents for `snapshot` with `selected`
/// highlighted, listing windows under `scope`, with app icons from
/// `icons` prepared at `scale` for the card `palette` paints, windowed
/// by `windowing` — the session's `rows` budget and the sticky viewport
/// it carries (`None` at open) — composing titles through `text` (the
/// backend the session froze; see [`TextBackend`]). Rows follow
/// [`switcher::eligible`] order (most recently activated first); every
/// row hints its Ctrl+N chord ([`RowHint::Ctrl`]).
///
/// One list, one budget: window per [`visible_window`], a continuation
/// row replacing the outermost slot at each truncated edge, and the
/// model reports the window it computed for the session to carry next.
/// Returns `None` when there is nothing to show: no active workspace,
/// no eligible windows, or a selection that is not in the eligible set.
// Nine render inputs — the text backend joined the seam beside the
// windowing and the shape profile (the geometry follow's note).
#[expect(clippy::too_many_arguments)]
pub fn overlay_model(
    snapshot: &Snapshot,
    selected: Handle,
    scope: Scope,
    icons: &crate::icons::Cache,
    scale: u32,
    palette: &Palette,
    profile: StyleProfile,
    windowing: Windowing,
    mut text: TextBackend<'_>,
) -> Option<OverlayModel> {
    let active = snapshot.active?;
    let entries = switcher::eligible(&snapshot.toplevels, active, scope);
    if entries.is_empty() {
        return None;
    }
    let sel = entries.iter().position(|&handle| handle == selected)?;
    let mut rows = Vec::with_capacity(entries.len());
    let mut bands = Vec::with_capacity(entries.len());
    for (index, &handle) in entries.iter().enumerate() {
        // Unreachable in practice: the sequence draws from this same
        // snapshot.
        let toplevel = snapshot.toplevels.iter().find(|t| t.handle == handle)?;
        let (title, band) = compose_title(
            &toplevel.title,
            &toplevel.app_id,
            RowKind::Leaf,
            &mut text,
            scale,
            profile.text,
        );
        rows.push(RowModel {
            title,
            icon: icons.bitmap(&toplevel.app_id, ICON, scale, palette),
            selected: index == sel,
            handle: Some(handle),
            kind: RowKind::Leaf,
            count: 0,
            hint: RowHint::ctrl(index),
            expanded: false,
        });
        bands.push(band);
    }
    finish_model(rows, sel, windowing, profile, bands)
}

/// Builds the horizontal band for `snapshot` with `selected`
/// highlighted — the flat list as one row of tiles, MRU-first from the
/// left, windowed by `banding` (the session's resolved tile budget and
/// the sticky band it carries, `None` at open) per [`band_window`].
/// A tile is the honest analogue the research locked: the app icon
/// over the clamped title, the selection filling the whole tile cell,
/// the Ctrl+N chord as a corner badge — reading order over the
/// displayed content tiles, ≤ 9, every displayed hint wired. A
/// truncated trailing edge spends its last tile on the wordless `…`.
/// Returns `None` under the same emptiness contract as
/// [`overlay_model`].
// Nine render inputs — the banding joined the seam beside the
// windowing (the geometry follow's note).
#[expect(clippy::too_many_arguments)]
pub fn overlay_model_horizontal(
    snapshot: &Snapshot,
    selected: Handle,
    scope: Scope,
    icons: &crate::icons::Cache,
    scale: u32,
    palette: &Palette,
    profile: StyleProfile,
    banding: Banding,
    mut text: TextBackend<'_>,
) -> Option<OverlayModel> {
    let active = snapshot.active?;
    let entries = switcher::eligible(&snapshot.toplevels, active, scope);
    if entries.is_empty() {
        return None;
    }
    let sel = entries.iter().position(|&handle| handle == selected)?;
    let band = band_window(entries.len(), sel, banding.tiles, banding.prev);
    let slots = band.content + band.bottom as usize;
    let mut rows = Vec::with_capacity(slots);
    for (position, &handle) in entries
        .iter()
        .skip(band.start)
        .take(band.content)
        .enumerate()
    {
        // Unreachable in practice: the sequence draws from this same
        // snapshot.
        let toplevel = snapshot.toplevels.iter().find(|t| t.handle == handle)?;
        rows.push(RowModel {
            // The display title unclamped — nothing paints it at the
            // compact pitch; the caption reads the selected tile's
            // title and truncates against its own budget.
            title: display_title(&toplevel.title, &toplevel.app_id),
            icon: icons.bitmap(&toplevel.app_id, TILE_ICON, scale, palette),
            selected: handle == selected,
            handle: Some(handle),
            kind: RowKind::Leaf,
            count: 0,
            hint: RowHint::ctrl(position),
            expanded: false,
        });
    }
    if band.bottom {
        rows.push(continuation_row());
    }
    // The card is the tile grid: wide-short, the `…` tile inside the
    // slots (the spec's "card width = N × tile pitch"), floored at the
    // caption's guarantee and — when the session carries one — the
    // `width`/`min-width` floor; the floor only widens, the tiles stay
    // left-aligned in the air it buys. The caption names the selected
    // window — the commit target of a flat band is always a window.
    let width = band_width(slots as u32).max(banding.width_floor.unwrap_or(0));
    let caption = compose_caption_at(
        rows.iter()
            .find(|row| row.selected)
            .map(|row| row.title.as_str())
            .unwrap_or_default(),
        width,
        &mut text,
        scale,
        profile.text,
    );
    Some(OverlayModel {
        workspace: String::new(),
        rows,
        width,
        height: band_card_height_at(profile.text.cell()),
        viewport: band,
        profile,
        horizontal: true,
        query: None,
        hovered: None,
        caption,
    })
}

/// Builds the grouped overlay contents: the session's frozen tree cut to
/// `visible` (its expansion state, liveness-filtered by the caller),
/// with the tree row `selected` highlighted. Titles and icons draw from
/// `snapshot`; the window set does not — a window that appeared after
/// the session froze never joins the card, and one that died was
/// filtered out of `visible` by the caller. Titles compose through
/// `text` (the session's frozen backend; see [`TextBackend`]).
///
/// Level-1 rows (app entries and leaves) hint their Ctrl+N chord by
/// position among level 1; children hint nothing (rev 5 — the Alt+N
/// chords are gone with the digits freed for the query). A parent's
/// pips count its app's frozen windows and its caret states the
/// expansion the caller passed — expanded iff its children follow it
/// in `visible`. Width derives from the visible rows' widest title
/// plus the child indent and the parent's caret terrace, so the card is
/// stable under stepping and breathes only when the user expands or
/// collapses. The pips themselves sit in the row's bottom padding and
/// spend no width. Returns `None` when
/// nothing is visible or the selection is not on a visible row.
// Eleven render inputs — the text backend joined the flat builder's
// seam one level deeper (its note).
#[expect(clippy::too_many_arguments)]
pub fn overlay_model_grouped(
    tree: &switcher::Grouping,
    visible: &[usize],
    selected: usize,
    snapshot: &Snapshot,
    icons: &crate::icons::Cache,
    scale: u32,
    palette: &Palette,
    profile: StyleProfile,
    windowing: Windowing,
    mut text: TextBackend<'_>,
) -> Option<OverlayModel> {
    let sel = visible.iter().position(|&index| index == selected)?;
    let mut rows = Vec::with_capacity(visible.len());
    let mut bands = Vec::with_capacity(visible.len());
    // The hint ordinal: level-1 position (Ctrl). Children hint nothing.
    let mut level1 = 0;
    for &index in visible {
        let grow = tree.rows().get(index)?;
        // Unreachable in practice: the caller filtered `visible` to
        // live windows.
        let toplevel = snapshot
            .toplevels
            .iter()
            .find(|t| t.handle == grow.handle)?;
        match grow.kind {
            RowKind::Child => {
                let (title, band) = compose_title(
                    &toplevel.title,
                    &toplevel.app_id,
                    RowKind::Child,
                    &mut text,
                    scale,
                    profile.text,
                );
                rows.push(RowModel {
                    title,
                    icon: icons.bitmap(&toplevel.app_id, ICON, scale, palette),
                    selected: index == selected,
                    handle: Some(grow.handle),
                    kind: RowKind::Child,
                    count: 0,
                    hint: None,
                    expanded: false,
                });
                bands.push(band);
            }
            kind => {
                let ordinal = level1;
                level1 += 1;
                if kind == RowKind::Parent {
                    // Children sit contiguously after their parent:
                    // the next visible row's kind is the state.
                    let expanded = tree
                        .rows()
                        .get(index + 1)
                        .is_some_and(|next| next.kind == RowKind::Child)
                        && visible.contains(&(index + 1));
                    // The parent's label is its app_id — the icon
                    // cache's own identity (spec OQ default; a
                    // desktop-entry display name is a cosmetic
                    // follow-up); the pips count the app's frozen
                    // windows, expanded state included in the card
                    // only through the caret. The label composes
                    // with no app_id fallback — the grouping tree only
                    // parents non-empty app_ids.
                    let hint = RowHint::ctrl(ordinal);
                    let (title, band) = compose_title(
                        &grow.app_id,
                        "",
                        RowKind::Parent,
                        &mut text,
                        scale,
                        profile.text,
                    );
                    rows.push(RowModel {
                        title,
                        icon: icons.bitmap(&grow.app_id, ICON, scale, palette),
                        selected: index == selected,
                        handle: Some(grow.handle),
                        kind: RowKind::Parent,
                        count: tree.children_of(index).len(),
                        hint,
                        expanded,
                    });
                    bands.push(band);
                } else {
                    let (title, band) = compose_title(
                        &toplevel.title,
                        &toplevel.app_id,
                        RowKind::Leaf,
                        &mut text,
                        scale,
                        profile.text,
                    );
                    rows.push(RowModel {
                        title,
                        icon: icons.bitmap(&toplevel.app_id, ICON, scale, palette),
                        selected: index == selected,
                        handle: Some(grow.handle),
                        kind: RowKind::Leaf,
                        count: 0,
                        hint: RowHint::ctrl(ordinal),
                        expanded: false,
                    });
                    bands.push(band);
                }
            }
        }
    }
    finish_model(rows, sel, windowing, profile, bands)
}

/// Builds the grouped-horizontal strip (`inplace-strip`): the frozen
/// tree as **one** sequence of tiles — the app tiles with the selected
/// app's tile replaced in place by its window run, exactly as the
/// daemon derived it — behind one [`band_window`], at one tile pitch.
///
/// The open group is derived, never carried: `sequence` is the pure
/// function of the tree selection the daemon computed (a child
/// selection ⇒ its parent's run at the parent's slot; a level-1
/// selection ⇒ plain app tiles), so the card holds no band or collapse
/// state of its own. The width arrives **frozen**: `width_slots` is the
/// daemon's max of the closed and open shapes' slots, so ↓ and ↑ never
/// resize the card — the narrower shape renders left-aligned in the
/// wider grid.
///
/// `sequence` holds tree indices, liveness-filtered by the caller (the
/// frozen window set) in the strip's reading order; `selected` is the
/// tree index of the selection. The strip windows per [`band_window`]
/// with the carried viewport (the `…` tile spends the last slot at a
/// truncated trailing edge and never holds the selection). Ctrl+N
/// chords read the strip left→right, ≤ 9. App tiles carry icon, label,
/// and count (a parent commits its MRU-front window); window tiles
/// carry icon and title — activation always resolves the tile's own
/// event-time handle.
// Eleven render inputs — the banding joined the seam beside the
// windowing (the geometry follow's note).
#[expect(clippy::too_many_arguments)]
pub fn overlay_model_grouped_horizontal(
    tree: &switcher::Grouping,
    sequence: &[usize],
    selected: usize,
    width_slots: usize,
    snapshot: &Snapshot,
    icons: &crate::icons::Cache,
    scale: u32,
    palette: &Palette,
    profile: StyleProfile,
    banding: Banding,
    mut text: TextBackend<'_>,
) -> Option<OverlayModel> {
    let sel = sequence.iter().position(|&index| index == selected)?;
    let window = band_window(sequence.len(), sel, banding.tiles, banding.prev);
    let mut rows = Vec::with_capacity(window.content + window.bottom as usize);
    for (hint_pos, position) in (0..window.content).enumerate() {
        let index = sequence[window.start + position];
        let grow = tree.rows().get(index)?;
        // Unreachable in practice: the caller filtered to live windows.
        let toplevel = snapshot
            .toplevels
            .iter()
            .find(|t| t.handle == grow.handle)?;
        // The display title unclamped — nothing paints it at the
        // compact pitch; the caption reads the commit target's title
        // and truncates against its own budget.
        let title = match grow.kind {
            RowKind::Parent => display_title(&grow.app_id, ""),
            _ => display_title(&toplevel.title, &toplevel.app_id),
        };
        rows.push(RowModel {
            title,
            icon: icons.bitmap(&grow.app_id, TILE_ICON, scale, palette),
            selected: index == selected,
            handle: Some(grow.handle),
            kind: grow.kind,
            count: if grow.kind == RowKind::Parent {
                tree.children_of(index).len()
            } else {
                0
            },
            hint: RowHint::ctrl(hint_pos),
            expanded: false,
        });
    }
    if window.bottom {
        rows.push(continuation_row());
    }
    // The frozen grid: the daemon's max of the two shapes, whatever
    // the strip is showing now, floored at the caption's guarantee and
    // — when the session carries one — the `width`/`min-width` floor;
    // the floor only widens, the strip stays left-aligned in it.
    let width = band_width(width_slots as u32).max(banding.width_floor.unwrap_or(0));
    // The caption names the commit target: a window tile's own title;
    // an app tile's MRU-front window's title (committing the app
    // activates that window — the parent row's handle is that window,
    // and the tile's own label names the app, not the target).
    let caption = match rows.iter().find(|row| row.selected) {
        Some(row) if row.kind == RowKind::Parent => {
            match snapshot
                .toplevels
                .iter()
                .find(|t| Some(t.handle) == row.handle)
            {
                Some(front) => compose_caption_at(
                    &display_title(&front.title, &front.app_id),
                    width,
                    &mut text,
                    scale,
                    profile.text,
                ),
                None => String::new(),
            }
        }
        Some(row) => compose_caption_at(&row.title, width, &mut text, scale, profile.text),
        None => String::new(),
    };
    Some(OverlayModel {
        workspace: String::new(),
        rows,
        width,
        height: band_card_height_at(profile.text.cell()),
        viewport: window,
        profile,
        horizontal: true,
        query: None,
        hovered: None,
        caption,
    })
}

/// Composes one row's title under `text`'s backend and returns it with
/// the title-band width the card sizes from (the title's own px — the
/// kind's extras are geometry and ride in the same number so
/// [`finish_model`] sees one width per row).
///
/// The two backends are two honest faces of one budget: `Bitmap` folds
/// and column-caps exactly as shipped (byte-identity at the default
/// 12-px profile; the floor's every pin depends on it); `System`
/// measures the un-folded title through the real font and pixel-budgets
/// it — real glyphs, real accents, fallback chains — with the kind's
/// extras paid out of the same [`TITLE_BAND_PX`] territory (the child
/// indent and the parent's caret always fit, exactly as the column caps
/// paid for them). `text_profile` chooses the cell and column budget, so
/// a larger `font_size` fits fewer characters in the same band.
fn compose_title(
    title: &str,
    app_id: &str,
    kind: RowKind,
    text: &mut TextBackend<'_>,
    scale: u32,
    text_profile: TextProfile,
) -> (String, u32) {
    // A parent's label is its app_id verbatim — no `display_title`
    // fallback (the grouping tree only parents non-empty app_ids, and
    // the floor's byte-identity keeps the raw path).
    let label = |title: &str| -> String {
        if kind == RowKind::Parent {
            title.to_owned()
        } else {
            display_title(title, app_id)
        }
    };
    let cell = text_profile.cell();
    let cols = text_profile.title_cols();
    match text {
        TextBackend::Bitmap => {
            let cap = match kind {
                RowKind::Child => cols.saturating_sub(text_profile.indent_cols()),
                RowKind::Parent => cols.saturating_sub(text_profile.chevron_cols()),
                RowKind::Leaf => cols,
            };
            let folded = truncate_cols(fold_title(&label(title)), cap);
            let band = folded.chars().count() as u32 * cell
                + (kind == RowKind::Child) as u32 * TREE_INDENT
                + (kind == RowKind::Parent) as u32 * text_profile.chevron_cols() * cell;
            (folded, band)
        }
        TextBackend::System { system, query } => {
            let extras = match kind {
                RowKind::Child => TREE_INDENT,
                RowKind::Parent => text_profile.chevron_cols() * cell,
                RowKind::Leaf => 0,
            };
            let budget = text_profile.title_band().saturating_sub(extras) as f32;
            let (display, run) =
                system.shape_budgeted(&label(title), *query, scale, budget, cell as f32);
            // Ceiling: the card must be at least as wide as the ink it
            // holds, whole logical px.
            (display, run.width_px.ceil() as u32 + extras)
        }
    }
}

/// The one caption's composition (glance-tiles G3, 2026-11-09): the
/// commit target's display title, truncated to the card's caption
/// budget — the card's width minus its horizontal padding — with the
/// real `…` in both backends (the row composer's truncation
/// vocabulary, the budget the card instead of a title band). The
/// preview sheet composes through the same rule, so its captions judge
/// the shipped truncation. Default text profile (the preview's own
/// seam is [`compose_caption_at`]).
#[allow(dead_code)] // the preview sheet's default-profile entry; the daemon uses `_at`
pub fn compose_caption(label: &str, width: u32, text: &mut TextBackend<'_>, scale: u32) -> String {
    compose_caption_at(label, width, text, scale, TextProfile::default())
}
/// [`compose_caption`] at one session's text profile.
pub fn compose_caption_at(
    label: &str,
    width: u32,
    text: &mut TextBackend<'_>,
    scale: u32,
    text_profile: TextProfile,
) -> String {
    let cell = text_profile.cell();
    let budget = width - 2 * TILE_PAD_X;
    match text {
        TextBackend::Bitmap => truncate_cols(fold_title(label), budget / cell),
        TextBackend::System { system, query } => {
            system
                .shape_budgeted(label, *query, scale, budget as f32, cell as f32)
                .0
        }
    }
}

/// The workspace label's composition (workspace-label W2, 2026-11-09):
/// the invoking workspace's name truncated to [`WS_LABEL_MAX_COLS`] with
/// the real `…` — the caption composer's own vocabulary at the label's
/// smaller cell.
fn compose_workspace(
    name: &str,
    text: &mut TextBackend<'_>,
    scale: u32,
    text_profile: TextProfile,
) -> String {
    let cell = ws_label_cell_at(text_profile.cell());
    match text {
        TextBackend::Bitmap => truncate_cols(fold_title(name), WS_LABEL_MAX_COLS),
        TextBackend::System { system, query } => {
            let budget = WS_LABEL_MAX_COLS as f32 * cell as f32;
            system
                .shape_budgeted(name, *query, scale, budget, cell as f32)
                .0
        }
    }
}

/// The builders' shared tail: widest-row sizing, the windowing rule, and
/// the continuation rows at truncated edges. `sel` indexes `rows`;
/// `bands` carries each row's title-band width (one per row, the
/// backend's own measure); the profile sizes the card's height and
/// rides onto the model for the renderer and the pointer to read back.
fn finish_model(
    rows: Vec<RowModel>,
    sel: usize,
    windowing: Windowing,
    profile: StyleProfile,
    bands: Vec<u32>,
) -> Option<OverlayModel> {
    let band = bands.iter().copied().max().unwrap_or(0);
    let viewport = visible_window(rows.len(), sel, windowing.rows, windowing.prev);
    let mut windowed =
        Vec::with_capacity(viewport.top as usize + viewport.content + viewport.bottom as usize);
    if viewport.top {
        windowed.push(continuation_row());
    }
    let first = viewport.start + viewport.top as usize;
    windowed.extend(rows.into_iter().skip(first).take(viewport.content));
    if viewport.bottom {
        windowed.push(continuation_row());
    }
    let count = windowed.len() as u32;
    // The title band's width is the widest row's own measure — columns ×
    // TEXT_W on the Bitmap floor, measured ink on System — under the same
    // ceiling: the band never exceeds MAX_COLS × TEXT_W (the compile-time
    // pin above its formula). The card adds its chrome and, when the
    // session carries one, the `width`/`min-width` floor — the floor only
    // widens, so the ceiling caps the title's ink, never the card; rows
    // stay left-aligned and the hint band right-anchors to the wider
    // edge, the band's floored-width behavior on end.
    let cell = profile.text.cell();
    let width = (2 * PAD_X + ICON + ICON_GAP + band + HINT_GAP + HINT_COLS * cell)
        .max(windowing.width_floor.unwrap_or(0));
    Some(OverlayModel {
        workspace: String::new(),
        rows: windowed,
        width,
        // The row gap lives between rows only; the bottom edge mirrors
        // the top. Height follows the profile's rhythm — width never
        // does (density has no width analogue).
        height: card_height_rhythm(profile.rhythm, count),
        viewport,
        profile,
        horizontal: false,
        query: None,
        hovered: None,
        // Vertical cards paint no caption — the field stays empty and
        // the horizontal-only paint never sees them.
        caption: String::new(),
    })
}

/// The continuation row at a truncated edge: the `…` affordance in a
/// full row slot, riding the row rhythm. Not hoverable, not clickable,
/// not number-addressable, never selected — its missing handle is what
/// every input guard already reads.
fn continuation_row() -> RowModel {
    RowModel {
        title: String::new(),
        icon: None,
        selected: false,
        handle: None,
        ..RowModel::default()
    }
}

/// The empty-match card (`type-filter`): the query's strip over its
/// one-word verdict — a single inert row no input can address (the
/// continuation-row precedent: no handle, no hint, nothing selected),
/// so the session and the query both stand while nothing matches.
pub fn no_matches_model(
    query: &str,
    profile: StyleProfile,
    width_floor: Option<u32>,
) -> Option<OverlayModel> {
    let mut text = TextBackend::Bitmap;
    let (title, band) = compose_title("no matches", "", RowKind::Leaf, &mut text, 2, profile.text);
    let row = RowModel {
        title,
        icon: None,
        selected: false,
        handle: None,
        kind: RowKind::Leaf,
        count: 0,
        hint: None,
        expanded: false,
    };
    let model = finish_model(
        vec![row],
        0,
        Windowing {
            rows: DEFAULT_ROWS,
            width_floor,
            prev: None,
        },
        profile,
        vec![band],
    )?;
    Some(with_query(model, query))
}

/// The height the query strip adds to a card at the default text size:
/// its pad, its text line, and the row gap that separates it from the
/// first row — zero with no query (`type-filter`). The strip is not a
/// row and earns no row height; the gap is the separation the rows
/// themselves use. [`strip_height_at`] is the same math at one
/// session's text size.
#[allow(dead_code)] // the default-profile seam for tests; the daemon uses `_at`
pub fn strip_height(query: &Option<String>, rhythm: Rhythm) -> u32 {
    strip_height_at(query, rhythm, TEXT_H)
}

/// [`strip_height`] at one text size.
pub fn strip_height_at(query: &Option<String>, rhythm: Rhythm, cell: u32) -> u32 {
    if query.is_some() {
        STRIP_PAD_Y + cell + rhythm.row_gap
    } else {
        0
    }
}

/// The strip's text: the query's *tail*, an ellipsis on the head when
/// it overflows — what was just typed stays visible, matching how the
/// query itself ignores everything before its last 64 characters. The
/// cap shares the row title's column budget minus the one column the
/// leading ellipsis spends.
fn strip_text(query: &str, cols: u32) -> String {
    let cap = cols as usize;
    let count = query.chars().count();
    if count > cap {
        let tail: String = query.chars().skip(count - (cap - 1)).collect();
        format!("\u{2026}{tail}")
    } else {
        query.to_string()
    }
}

/// Sets `model`'s query and grows its height by the strip — the one
/// seam between a querying session and its card. Pure, so the height
/// math and the zero-query identity pin headlessly.
pub fn with_query(mut model: OverlayModel, query: &str) -> OverlayModel {
    let cell = model.profile.text.cell();
    model.height += strip_height_at(&Some(query.to_string()), model.profile.rhythm, cell);
    model.query = Some(strip_text(query, model.profile.text.title_cols()));
    model
}

/// The workspace label's line at one rhythm and text size — the label's
/// own (smaller) cell plus the row gap, the strip's twin separator
/// (workspace-label W2, 2026-11-09).
pub fn ws_label_height_at(rhythm: Rhythm, cell: u32) -> u32 {
    ws_label_cell_at(cell) + rhythm.row_gap
}

/// Sets `model`'s workspace label and pays for its line (workspace-label
/// W2, 2026-11-09) — the `with_query` seam's twin, pure so the height
/// math and the empty-name identity pin headlessly. The vertical cards
/// reserve the label's line above the first row (`row_at` walks past it
/// like the strip's); the horizontal cards keep their height — the
/// label rides the caption line the card already reserves — and the
/// caption recomposes against the budget the label's zone takes, so a
/// full-length caption never collides with it. The empty name is the
/// identity: the default-off card is unchanged.
pub fn with_workspace(
    mut model: OverlayModel,
    name: &str,
    text: &mut TextBackend<'_>,
    scale: u32,
) -> OverlayModel {
    if name.is_empty() {
        return model;
    }
    model.workspace = compose_workspace(name, text, scale, model.profile.text);
    let cell = model.profile.text.cell();
    if model.horizontal {
        // The bottom line (W7): below the tiles, centered — the card
        // grows by the clearing and the label's own smaller cell, the
        // caption's line untouched above.
        model.height += WS_LABEL_GAP + ws_label_cell_at(cell);
    } else {
        model.height += ws_label_height_at(model.profile.rhythm, cell);
    }
    model
}

/// The horizontal spans of logical row `ly` that lie within `reach` of a
/// card edge: full width near a horizontal edge, the two end bands
/// otherwise. An empty span is expressed as `x0 == x1`.
fn edge_spans(ly: u32, lh: u32, lw: u32, reach: u32) -> [(u32, u32); 2] {
    if ly < reach || ly + reach >= lh {
        [(0, lw), (0, 0)]
    } else {
        [(0, reach), (lw.saturating_sub(reach), lw)]
    }
}

/// Renders `model` into `canvas` as ARGB8888 (`stride` bytes per row) at
/// `scale` device pixels per logical pixel, in the card colors of
/// `palette`, painting text through `text` (the session's frozen
/// backend; see [`TextBackend`]).
///
/// Paint order, back to front: card fill through the rounded-corner
/// coverage mask, selection band per selected row, title glyphs, app
/// icons, and the border hairline last so the stroke reads above the
/// selection. Every device pixel inside `width`/`height` is written;
/// coordinates outside are skipped, so an oversized canvas is harmless.
// Nine render inputs — the text backend joined the seam beside the
/// windowing and the shape profile (the geometry follow's note).
#[expect(clippy::too_many_arguments)]
pub fn render(
    model: &OverlayModel,
    canvas: &mut [u8],
    width: u32,
    height: u32,
    stride: u32,
    scale: u32,
    palette: &Palette,
    mut text: TextBackend<'_>,
) {
    // Unreachable in production (render_scale clamps with max(2, ·)),
    // but render is pub: fail loudly in debug, clamp in release, never
    // divide by zero.
    debug_assert!(scale > 0, "render scale must be positive");
    let scale = scale.max(1);
    let lw = width / scale;
    let lh = height / scale;
    // The shape the model was built under — every radius and rhythm
    // below reads the profile, so the render always matches the
    // geometry `row_at` addresses (the constants are only the default
    // row).
    let profile = model.profile;
    let radius = profile.radius;
    let band_radius = profile.band_radius;
    let rhythm = profile.rhythm;
    // The session's text profile: every line, cell, and column budget
    // below derives from it, so the paint matches the model's sizing.
    let text_profile = profile.text;
    let cell = text_profile.cell();
    // Coverage masks on the logical grid, computed once per render:
    // the card shape and the same shape eroded by the border width
    // (their difference is the border ring).
    let mut outer = vec![0u8; (lw * lh) as usize];
    for ly in 0..lh {
        for lx in 0..lw {
            outer[(ly * lw + lx) as usize] = shape_cov(lx, ly, lw, lh, radius, 0);
        }
    }
    // The eroded mask is only read near the card edges (the border
    // pass), so computing the interior is wasted work at Tab speed.
    let mut inner = vec![0u8; (lw * lh) as usize];
    let reach = radius + BORDER;
    for ly in 0..lh {
        let spans = edge_spans(ly, lh, lw, reach);
        for (x0, x1) in spans {
            for lx in x0..x1 {
                inner[(ly * lw + lx) as usize] = shape_cov(lx, ly, lw, lh, radius, BORDER);
            }
        }
    }
    for ly in 0..lh {
        let cov_row = &outer[(ly * lw) as usize..(ly * lw + lw) as usize];
        for (lx, cov) in cov_row.iter().enumerate() {
            let argb = if *cov == 255 {
                palette.background
            } else {
                coverage_argb(palette.background, *cov)
            };
            let x0 = lx as u32 * scale;
            for dy in 0..scale {
                let y = ly * scale + dy;
                for x in x0..x0 + scale {
                    put_pixel(canvas, stride, x, y, argb);
                }
            }
        }
    }
    if model.horizontal {
        // The one caption (glance-tiles G3, 2026-11-09): the commit
        // target's title, fixed top-center in the full text color —
        // the line the card always reserves, empty or not. Centered
        // ink; the paint loop's edge is the card's own padding,
        // defense in depth behind the composer's budget. The workspace
        // label no longer shares this line (W7): it takes its own at
        // the card's bottom, and the caption's budget is whole again.
        match &mut text {
            TextBackend::Bitmap => {
                let ink = model.caption.chars().count() as u32 * cell;
                let edge = lw.saturating_sub(TILE_PAD_X);
                let mut lx = lw.saturating_sub(ink) / 2;
                for ch in model.caption.chars() {
                    if lx + cell > edge {
                        break;
                    }
                    draw_glyph(
                        canvas,
                        stride,
                        glyph(ch),
                        lx * scale,
                        TILE_PAD_Y * scale,
                        cell * scale / GLYPH_W,
                        palette.text,
                    );
                    lx += cell;
                }
            }
            TextBackend::System { system, query } => {
                let run = system.shape(&model.caption, *query, scale, cell as f32);
                let ink = run.width_px.ceil() as u32;
                let baseline =
                    TILE_PAD_Y + crate::text::centering(run.ascent_px, run.descent_px, cell);
                paint_run(
                    system,
                    &run,
                    canvas,
                    stride,
                    scale,
                    palette.text,
                    lw.saturating_sub(ink) / 2,
                    baseline,
                    lw.saturating_sub(TILE_PAD_X),
                );
            }
        }
        // The workspace label's own line (workspace-label W7,
        // 2026-11-09): below the tiles, centered on the card — one text
        // color, the smaller cell, the caption's quiet counterpart at
        // the other end. The line exists only when the label stands;
        // the seam grew the card for it.
        if !model.workspace.is_empty() {
            let label_cell = ws_label_cell_at(cell);
            let y = band_card_height_at(cell) + WS_LABEL_GAP;
            match &mut text {
                TextBackend::Bitmap => {
                    let ink = model.workspace.chars().count() as u32 * label_cell;
                    let mut lx = lw.saturating_sub(ink) / 2;
                    for ch in model.workspace.chars() {
                        draw_glyph(
                            canvas,
                            stride,
                            glyph(ch),
                            lx * scale,
                            y * scale,
                            label_cell * scale / GLYPH_W,
                            palette.text,
                        );
                        lx += label_cell;
                    }
                }
                TextBackend::System { system, query } => {
                    let run = system.shape(&model.workspace, *query, scale, label_cell as f32);
                    let ink = run.width_px.ceil() as u32;
                    let baseline =
                        y + crate::text::centering(run.ascent_px, run.descent_px, label_cell);
                    paint_run(
                        system,
                        &run,
                        canvas,
                        stride,
                        scale,
                        palette.text,
                        lw.saturating_sub(ink) / 2,
                        baseline,
                        lw.saturating_sub(TILE_PAD_X),
                    );
                }
            }
        }
        // One tile renderer, one band top: the flat band and the
        // grouped strip share the single-band paint, starting below
        // the reserved caption line.
        render_band(
            model,
            canvas,
            stride,
            lw,
            scale,
            palette,
            &mut text,
            band_strip_top_at(cell),
            &model.rows,
        );
        border_ring(
            canvas,
            stride,
            lw,
            lh,
            scale,
            radius,
            palette.border,
            &outer,
            &inner,
        );
        return;
    }
    // The workspace label's line (workspace-label W3, 2026-11-09; the
    // left edge W7): the reserved twin above the query strip — the
    // name left-aligned at the text column's spine (the strip's own
    // x), its smaller cell at the line's top, the row gap below it the
    // separator. Every strip and row below shifts by its height, the
    // same height `row_at` subtracts.
    let label_line = if model.workspace.is_empty() {
        0
    } else {
        let label_cell = ws_label_cell_at(cell);
        let left = PAD_X + ICON + ICON_GAP;
        let y = rhythm.pad_y;
        match &mut text {
            TextBackend::Bitmap => {
                let mut lx = left;
                for ch in model.workspace.chars() {
                    draw_glyph(
                        canvas,
                        stride,
                        glyph(ch),
                        lx * scale,
                        y * scale,
                        label_cell * scale / GLYPH_W,
                        palette.text,
                    );
                    lx += label_cell;
                }
            }
            TextBackend::System { system, query } => {
                let run = system.shape(&model.workspace, *query, scale, label_cell as f32);
                let baseline =
                    y + crate::text::centering(run.ascent_px, run.descent_px, label_cell);
                paint_run(
                    system,
                    &run,
                    canvas,
                    stride,
                    scale,
                    palette.text,
                    left,
                    baseline,
                    lw.saturating_sub(PAD_X),
                );
            }
        }
        ws_label_height_at(rhythm, cell)
    };
    // The query strip (`type-filter`): the query in the titles' own
    // column and voice — full text color, the session's frozen backend —
    // with a static caret bar for its terminal. The strip exists only
    // while a query stands; every row below shifts by its height, the
    // same height `row_at` subtracts, so pointer rows and paint stay
    // one geometry.
    let strip = strip_height_at(&model.query, rhythm, cell);
    if let Some(query) = &model.query {
        let strip_y = rhythm.pad_y + label_line + STRIP_PAD_Y;
        let title_edge = lw.saturating_sub(PAD_X + HINT_GAP + HINT_COLS * cell);
        let title_x = PAD_X + ICON + ICON_GAP;
        let text_end = match &mut text {
            TextBackend::Bitmap => {
                let mut lx = title_x;
                for ch in query.chars() {
                    if lx + cell > title_edge {
                        break;
                    }
                    draw_glyph(
                        canvas,
                        stride,
                        glyph(ch),
                        lx * scale,
                        strip_y * scale,
                        cell * scale / GLYPH_W,
                        palette.text,
                    );
                    lx += cell;
                }
                lx
            }
            TextBackend::System {
                system,
                query: fontq,
            } => {
                let run = system.shape(query, *fontq, scale, cell as f32);
                let baseline =
                    strip_y + crate::text::centering(run.ascent_px, run.descent_px, cell);
                paint_run(
                    system,
                    &run,
                    canvas,
                    stride,
                    scale,
                    palette.text,
                    title_x,
                    baseline,
                    title_edge,
                );
                title_x + run.width_px.ceil() as u32
            }
        };
        // The caret: a 2-px hair in the full text color, the text's own
        // height — static, the motion vocabulary's one-line budget
        // spent elsewhere.
        let caret_x = text_end + STRIP_CARET_GAP;
        if caret_x + STRIP_CARET_W <= lw {
            for ly in strip_y..strip_y + cell {
                for dy in 0..scale {
                    let y = ly * scale + dy;
                    for lx in caret_x..caret_x + STRIP_CARET_W {
                        let x0 = lx * scale;
                        for x in x0..x0 + scale {
                            put_pixel(canvas, stride, x, y, palette.text);
                        }
                    }
                }
            }
        }
    }
    for (index, row) in model.rows.iter().enumerate() {
        let top =
            rhythm.pad_y + label_line + strip + index as u32 * (rhythm.row_h + rhythm.row_gap);
        // Continuation row: the `…` glyph card-centered on the titles'
        // own line, in the muted derivation — the hint color, quieter
        // than any title and legible on both card fills and both
        // wallpaper polarities. Nothing else paints on this row: no
        // band, no icon, no title, no hint — a row beyond addressability
        // earns nothing but its dots. The glyph paints `TEXT_W` wide, so
        // its box centers on `TEXT_W` exactly as `dots_y` centers on
        // `TEXT_H`; centering on the font cell biased the ink 2 px right
        // of the card's center line. Frozen 2026-11-08 on the LV3 sheet
        // after a two-round preview loop (the TH4 pattern): every token
        // kept as built — the muted reuse (55%, the hints' own color),
        // card-centered placement, the font's own glyph at text scale —
        // the native-resolution round passing both palettes over both
        // backdrops at even perceived weight.
        if row.handle.is_none() {
            // A titled addressable-nothing is the no-matches verdict
            // (`type-filter`): its words paint in the muted derivation —
            // the beyond-addressability color, the dots' own — in the
            // titles' column, wordless dots stay for the untitled
            // continuation row.
            if !row.title.is_empty() {
                let verdict_x = PAD_X + ICON + ICON_GAP;
                let verdict_y = top + rhythm.row_h.saturating_sub(cell) / 2;
                match &mut text {
                    TextBackend::Bitmap => {
                        let mut lx = verdict_x;
                        for ch in row.title.chars() {
                            draw_glyph(
                                canvas,
                                stride,
                                glyph(ch),
                                lx * scale,
                                verdict_y * scale,
                                cell * scale / GLYPH_W,
                                palette.muted,
                            );
                            lx += cell;
                        }
                    }
                    TextBackend::System {
                        system,
                        query: fontq,
                    } => {
                        let run = system.shape(&row.title, *fontq, scale, cell as f32);
                        let baseline = top
                            + crate::text::centering(run.ascent_px, run.descent_px, rhythm.row_h);
                        paint_run(
                            system,
                            &run,
                            canvas,
                            stride,
                            scale,
                            palette.muted,
                            verdict_x,
                            baseline,
                            lw.saturating_sub(PAD_X),
                        );
                    }
                }
                continue;
            }
            match &mut text {
                TextBackend::Bitmap => {
                    let dots_x = lw.saturating_sub(cell) / 2;
                    let dots_y = top + rhythm.row_h.saturating_sub(cell) / 2;
                    draw_glyph(
                        canvas,
                        stride,
                        ELLIPSIS_GLYPH,
                        dots_x * scale,
                        dots_y * scale,
                        cell * scale / GLYPH_W,
                        palette.muted,
                    );
                }
                TextBackend::System { system, query } => {
                    // The font's own `…` on the same muted line: the
                    // measured ink centers on the card (the LV3
                    // card-centered rule, now against real metrics),
                    // the baseline by the ink box.
                    let run = system.shape("…", *query, scale, cell as f32);
                    let dots_x = lw.saturating_sub(run.width_px.ceil() as u32) / 2;
                    let baseline =
                        top + crate::text::centering(run.ascent_px, run.descent_px, rhythm.row_h);
                    paint_run(
                        system,
                        &run,
                        canvas,
                        stride,
                        scale,
                        palette.muted,
                        dots_x,
                        baseline,
                        lw,
                    );
                }
            }
            continue;
        }
        if row.selected {
            // The band is its own rounded rect: `BAND_RADIUS` ends
            // through the same coverage sampler as the card corners.
            // The ramp interpolates toward the band fill between two
            // painted fills — full coverage is the exact band color, zero
            // coverage leaves the card untouched, and because both are
            // near-opaque the arc never thins the card. The
            // profile's `pad_y + SELECT_INSET ≥ radius` holds the band
            // clear of the card's corner arcs, so the card mask
            // doesn't ride along.
            let band_w = lw.saturating_sub(2 * SELECT_INSET);
            for ly in top..top + rhythm.row_h {
                for dy in 0..scale {
                    let y = ly * scale + dy;
                    for lx in SELECT_INSET..lw.saturating_sub(SELECT_INSET) {
                        let cov = shape_cov(
                            lx - SELECT_INSET,
                            ly - top,
                            band_w,
                            rhythm.row_h,
                            band_radius,
                            0,
                        );
                        if cov == 0 {
                            continue;
                        }
                        let x0 = lx * scale;
                        for x in x0..x0 + scale {
                            let index = y as usize * stride as usize + x as usize * 4;
                            if let Some(slot) = canvas.get_mut(index..index + 4) {
                                let dst = u32::from_le_bytes(
                                    slot.try_into().expect("pixel is four bytes"),
                                );
                                let out = mix_pixel(dst, palette.selected_bg, cov);
                                slot.copy_from_slice(&out.to_le_bytes());
                            }
                        }
                    }
                }
            }
        }
        // Icon cell: a bitmap prepared at ICON × scale blits 1:1; a
        // mismatched size or a missing icon draws the generic glyph. A
        // child row indents its whole cell — icon and title move
        // together, one TREE_INDENT right of the top-level column.
        let indent = (row.kind == RowKind::Child) as u32 * TREE_INDENT;
        let cell_x = PAD_X + indent;
        let cell_y = top + (rhythm.row_h - ICON) / 2;
        match row.icon.as_deref() {
            Some(bitmap) if bitmap.side == ICON * scale => {
                blit_bitmap(canvas, stride, bitmap, cell_x * scale, cell_y * scale);
            }
            _ => {
                let side = GLYPH_W * GLYPH_SCALE;
                draw_glyph(
                    canvas,
                    stride,
                    WINDOW_GLYPH,
                    (cell_x + (ICON - side) / 2) * scale,
                    (cell_y + (ICON - side) / 2) * scale,
                    GLYPH_SCALE * scale,
                    palette.text,
                );
            }
        }
        // The × reveal (`close-window`): pointer presence on a window
        // row claims the icon's top-right corner — the same knockout
        // badge the horizontal tiles paint, so the affordance is one
        // vocabulary on both cards. Headers and continuations never
        // reveal, whatever the pointer does.
        if model.hovered == Some(index) && closeable(row) {
            paint_close_badge(canvas, stride, scale, cell_x, cell_y, ICON, palette);
        }
        // Title: vertically centered against the row, stopping at the
        // title band's right edge — the hint's territory is its own, so
        // even a max-cols title can never touch a hint (the model sizes
        // for the band; the edge guard is the defense in depth). The
        // match also fixes the row's text line for the hint below: on
        // Bitmap that is the TEXT_H cell's top y, on System the shaped
        // run's baseline — each backend's own anchor — and `title_end`
        // is where the title's ink stopped (the caret's clearing
        // measures from it).
        let title_edge = lw.saturating_sub(PAD_X + HINT_GAP + HINT_COLS * cell);
        let title_x = cell_x + ICON + ICON_GAP;
        let (text_line_y, title_end) = match &mut text {
            TextBackend::Bitmap => {
                let text_y = top + rhythm.row_h.saturating_sub(cell) / 2;
                let mut lx = title_x;
                for ch in row.title.chars() {
                    if lx + cell > title_edge {
                        break;
                    }
                    draw_glyph(
                        canvas,
                        stride,
                        glyph(ch),
                        lx * scale,
                        text_y * scale,
                        cell * scale / GLYPH_W,
                        palette.text,
                    );
                    lx += cell;
                }
                (text_y, lx)
            }
            TextBackend::System { system, query } => {
                let run = system.shape(&row.title, *query, scale, cell as f32);
                let baseline =
                    top + crate::text::centering(run.ascent_px, run.descent_px, rhythm.row_h);
                paint_run(
                    system,
                    &run,
                    canvas,
                    stride,
                    scale,
                    palette.text,
                    title_x,
                    baseline,
                    title_edge,
                );
                (baseline, title_x + run.width_px.ceil() as u32)
            }
        };
        // Parent count pips and disclosure caret. The pips are the
        // horizontal tiles' count vocabulary (glance-tiles G2), adapted
        // below the app name: up to three muted squares, saturating,
        // never a fill shape, sitting in the row's own bottom padding
        // (the 4-px clearing above them falls out of the centered text
        // line at every density). Re-pinned 2026-11-09 (owner,
        // in-thread) from the numeric count chip and then from the
        // inline pips — the count no longer spends title width. The
        // caret stays on the title line: an 8-px, two-pixel-stroke
        // mark one CHEVRON_GAP after the title's ink, right when
        // collapsed and down when expanded (the 16-px solid wedge
        // floated in the hint-gap column and read as a play button;
        // re-pinned 2026-11-09). Leaves and children earn neither.
        if row.kind == RowKind::Parent {
            if row.count >= 2 {
                let pips = row.count.min(3) as u32;
                let pip_y = top + rhythm.row_h - rhythm.row_pad_y - PIP_SIDE;
                paint_pips(canvas, stride, scale, title_x, pip_y, pips, palette.muted);
            }
            let chevron = if row.expanded {
                CHEVRON_DOWN_GLYPH
            } else {
                CHEVRON_RIGHT_GLYPH
            };
            let side = GLYPH_W * CHEVRON_SCALE;
            let chevron_x = title_end + CHEVRON_GAP;
            // Defense in depth: the model's CHEVRON_COLS reservation
            // already funds the caret and its clearing, so only a
            // hand-built over-budget model ever clips it.
            if chevron_x + side <= title_edge {
                let chevron_y = top + (rhythm.row_h - side) / 2;
                draw_glyph(
                    canvas,
                    stride,
                    chevron,
                    chevron_x * scale,
                    chevron_y * scale,
                    CHEVRON_SCALE * scale,
                    palette.text,
                );
            }
        }
        // Hint: the row's own number digit, right-aligned in the band
        // the card always reserves and vertically centered on the
        // titles' own line (`text_line_y`: the Bitmap cell top, the
        // System baseline). Rows past nine paint nothing — every
        // displayed hint is a wired key (the model's [`RowHint`] is
        // that truth's single source, paired with `key_input`). The
        // digit stays put under pointer presence: the × lives on the
        // icon now, in its own corner, and never takes the band.
        if let Some(label) = row.hint.map(RowHint::digit) {
            let band_right = lw.saturating_sub(PAD_X);
            match &mut text {
                TextBackend::Bitmap => {
                    let mut hx = band_right.saturating_sub(label.chars().count() as u32 * cell);
                    for ch in label.chars() {
                        if hx + cell > band_right {
                            break; // a degenerate canvas: never paint past the card
                        }
                        draw_glyph(
                            canvas,
                            stride,
                            glyph(ch),
                            hx * scale,
                            text_line_y * scale,
                            cell * scale / GLYPH_W,
                            palette.muted,
                        );
                        hx += cell;
                    }
                }
                TextBackend::System { system, query } => {
                    let run = system.shape(&label, *query, scale, cell as f32);
                    let hx = band_right.saturating_sub(run.width_px.ceil() as u32);
                    paint_run(
                        system,
                        &run,
                        canvas,
                        stride,
                        scale,
                        palette.muted,
                        hx,
                        text_line_y,
                        band_right,
                    );
                }
            }
        }
    }
    border_ring(
        canvas,
        stride,
        lw,
        lh,
        scale,
        radius,
        palette.border,
        &outer,
        &inner,
    );
}

/// The hairline border ring, last on both axes: inside the card but
/// not fully inside the eroded shape, so the stroke follows the
/// rounded corners with the same anti-aliasing. The
/// difference-of-masks formulation could open gaps where both masks
/// quantize alike on an arc; this rule cannot. The ring lives only
/// within radius + BORDER of the card boundary — scanning the interior
/// is wasted work at Tab speed.
// Nine paint inputs — render's own seam, extracted for the horizontal
// branch (the geometry follow's note).
#[expect(clippy::too_many_arguments)]
fn border_ring(
    canvas: &mut [u8],
    stride: u32,
    lw: u32,
    lh: u32,
    scale: u32,
    radius: u32,
    border: u32,
    outer: &[u8],
    inner: &[u8],
) {
    for ly in 0..lh {
        for (x0, x1) in edge_spans(ly, lh, lw, radius + BORDER) {
            for lx in x0..x1 {
                let index = (ly * lw + lx) as usize;
                if outer[index] == 0 || inner[index] == 255 {
                    continue;
                }
                let argb = coverage_argb(border, outer[index]);
                for dy in 0..scale {
                    for dx in 0..scale {
                        put_pixel(canvas, stride, lx * scale + dx, ly * scale + dy, argb);
                    }
                }
            }
        }
    }
}

/// Paints the horizontal band's tiles — the card background and border
/// are the shared passes in [`render`]; this is the content loop. A
/// content tile carries the selection fill (its own rounded rect over
/// the whole tile cell, the row band's own vocabulary), the app icon
/// centered on the icon line, the clamped title beneath, and the Ctrl+N
/// chord as a corner badge (top-left of the badge row, muted like every
/// hint — placement frozen on the CL4 sheet, 2026-11-08). The `…` tile
/// paints its
/// muted glyph centered on the icon line and nothing else — no band,
/// no icon, no title, no badge (a tile beyond addressability earns
/// nothing but its dots).
/// Paints one band of tiles — `tiles` at a card top of `y0` — the shared
/// tile renderer behind the flat horizontal band and the grouped strip.
/// A tile is the honest analogue the
/// research locked: the app icon over the clamped title, the selection
/// filling the whole tile cell, the Ctrl+N chord as a corner badge, and
/// a handleless tile drawing the wordless `…` centered in the cell.
// Nine paint inputs — the band top and slice joined the render seam
// (the geometry follow's note).
#[expect(clippy::too_many_arguments)]
fn render_band(
    model: &OverlayModel,
    canvas: &mut [u8],
    stride: u32,
    lw: u32,
    scale: u32,
    palette: &Palette,
    text: &mut TextBackend<'_>,
    y0: u32,
    tiles: &[RowModel],
) {
    let band_radius = model.profile.band_radius;
    let cell = model.profile.text.cell();
    let tile_cell = tile_cell_h_at(cell);
    for (index, tile) in tiles.iter().enumerate() {
        let left = TILE_PAD_X + index as u32 * (TILE_W + TILE_GAP);
        if tile.handle.is_none() {
            match text {
                TextBackend::Bitmap => {
                    let dots_x = left + (TILE_W.saturating_sub(cell)) / 2;
                    let dots_y = y0 + cell + TILE_PARENT_GAP + TILE_ICON.saturating_sub(cell) / 2;
                    draw_glyph(
                        canvas,
                        stride,
                        ELLIPSIS_GLYPH,
                        dots_x * scale,
                        dots_y * scale,
                        cell * scale / GLYPH_W,
                        palette.muted,
                    );
                }
                TextBackend::System { system, query } => {
                    let run = system.shape("\u{2026}", *query, scale, cell as f32);
                    let dots_x = left + (TILE_W - run.width_px.ceil() as u32) / 2;
                    let baseline = y0
                        + cell
                        + TILE_PARENT_GAP
                        + crate::text::centering(run.ascent_px, run.descent_px, TILE_ICON);
                    paint_run(
                        system,
                        &run,
                        canvas,
                        stride,
                        scale,
                        palette.muted,
                        dots_x,
                        baseline,
                        lw,
                    );
                }
            }
            continue;
        }
        if tile.selected {
            for ly in y0..y0 + tile_cell {
                for dy in 0..scale {
                    let y = ly * scale + dy;
                    for lx in left..left + TILE_W {
                        let cov = shape_cov(lx - left, ly - y0, TILE_W, tile_cell, band_radius, 0);
                        if cov == 0 {
                            continue;
                        }
                        let x0 = lx * scale;
                        for x in x0..x0 + scale {
                            let index = y as usize * stride as usize + x as usize * 4;
                            if let Some(slot) = canvas.get_mut(index..index + 4) {
                                let dst = u32::from_le_bytes(
                                    slot.try_into().expect("pixel is four bytes"),
                                );
                                let out = mix_pixel(dst, palette.selected_bg, cov);
                                slot.copy_from_slice(&out.to_le_bytes());
                            }
                        }
                    }
                }
            }
        }
        // The icon, centered on its line: a bitmap prepared at
        // TILE_ICON × scale blits 1:1; a mismatched size or a miss
        // draws the generic window glyph (the row cell's rule).
        let icon_y = y0 + cell + TILE_PARENT_GAP;
        match tile.icon.as_deref() {
            Some(bitmap) if bitmap.side == TILE_ICON * scale => {
                blit_bitmap(
                    canvas,
                    stride,
                    bitmap,
                    (left + (TILE_W - TILE_ICON) / 2) * scale,
                    icon_y * scale,
                );
            }
            _ => {
                let side = GLYPH_W * GLYPH_SCALE;
                draw_glyph(
                    canvas,
                    stride,
                    WINDOW_GLYPH,
                    (left + (TILE_W - side) / 2) * scale,
                    (icon_y + (TILE_ICON - side) / 2) * scale,
                    GLYPH_SCALE * scale,
                    palette.text,
                );
            }
        }
        // No title line: the tile is icon-only at the compact pitch
        // (glance-tiles G1, 2026-11-09) — the one caption names the
        // commit target; the per-tile title paint died with the
        // title-gap token it rode on.
        // The count pips (glance-tiles G2, 2026-11-09; research record
        // R4): under the icon, only while the app holds ≥ 2 live
        // windows, saturating at three — muted squares in their own
        // territory, never a fill shape, never a corner digit (the
        // notification-count reading R4 cautions against).
        if tile.count >= 2 {
            let pips = tile.count.min(3) as u32;
            let pip_y = icon_y + TILE_ICON + PIP_GAP;
            let span = (pips - 1) * PIP_STEP;
            let first_x = left + TILE_W / 2 - span / 2 - PIP_SIDE / 2;
            paint_pips(canvas, stride, scale, first_x, pip_y, pips, palette.muted);
        }
        // The corner badge — the chord's digit alone, the chord's own
        // muted color, top-left of the tile's badge row (CL4-frozen
        // placement, 2026-11-08; digit-only since glance-tiles G2,
        // 2026-11-09).
        if let Some(hint) = tile.hint {
            let label = hint.digit();
            match text {
                TextBackend::Bitmap => {
                    let mut lx = left + TILE_TEXT_PAD;
                    for ch in label.chars() {
                        draw_glyph(
                            canvas,
                            stride,
                            glyph(ch),
                            lx * scale,
                            y0 * scale,
                            cell * scale / GLYPH_W,
                            palette.muted,
                        );
                        lx += cell;
                    }
                }
                TextBackend::System { system, query } => {
                    let run = system.shape(&label, *query, scale, cell as f32);
                    paint_run(
                        system,
                        &run,
                        canvas,
                        stride,
                        scale,
                        palette.muted,
                        left + TILE_TEXT_PAD,
                        y0 + crate::text::centering(run.ascent_px, run.descent_px, cell),
                        left + TILE_W - TILE_TEXT_PAD,
                    );
                }
            }
        }
        // The × reveal (`close-window`): pointer presence on a window
        // tile claims the top-right corner of its icon — the same
        // knockout badge the vertical rows paint. App tiles (group
        // headers) and the `…` tile never reveal, whatever the pointer
        // does; app tiles still close their front window through
        // Alt+minus.
        if model.hovered == Some(index) && closeable(tile) {
            paint_close_badge(
                canvas,
                stride,
                scale,
                left + (TILE_W - TILE_ICON) / 2,
                icon_y,
                TILE_ICON,
                palette,
            );
        }
    }
}

/// Coverage of the logical pixel `(lx, ly)` by a `w × h` rounded
/// rectangle inset by `inset` on every side (radius shrinking with it),
/// in 0..=255.
///
/// Exact along the straight edges (a single comparison); 4×4 area
/// sampling inside the corner quadrants only — 16 subsamples per
/// boundary pixel, which is anti-aliasing by measurement, not blur.
fn shape_cov(lx: u32, ly: u32, w: u32, h: u32, radius: u32, inset: u32) -> u8 {
    let radius = radius.saturating_sub(inset);
    let (left, top) = (inset, inset);
    let (right, bottom) = (w.saturating_sub(inset), h.saturating_sub(inset));
    if right <= left || bottom <= top {
        return 0;
    }
    // Distance to the straight edges; positive = inside.
    let dx = (lx as i64 - left as i64).min(right as i64 - 1 - lx as i64);
    let dy = (ly as i64 - top as i64).min(bottom as i64 - 1 - ly as i64);
    if dx >= radius as i64 && dy >= radius as i64 {
        // Interior of the shape, far from every corner.
        return 255;
    }
    if dx >= radius as i64 || dy >= radius as i64 {
        // A straight-edge band: exact.
        return if dx >= 0 && dy >= 0 { 255 } else { 0 };
    }
    // Corner quadrant: area-sample the pixel square against the rounded
    // corner, 8×8 — 64 coverage steps turn the arc into a smooth ramp;
    // coarser sampling read as ticks against light backdrops. Arc
    // centers sit at (left + r, top + r) and mirrored.
    let corner_x = if lx < left + radius {
        left + radius
    } else {
        right.saturating_sub(radius)
    };
    let corner_y = if ly < top + radius {
        top + radius
    } else {
        bottom.saturating_sub(radius)
    };
    let r_sq = (radius as f32) * (radius as f32);
    let mut hits = 0u32;
    for sy in 0..8 {
        for sx in 0..8 {
            let px = lx as f32 + (sx as f32 + 0.5) / 8.0;
            let py = ly as f32 + (sy as f32 + 0.5) / 8.0;
            let dx = px - corner_x as f32;
            let dy = py - corner_y as f32;
            if dx * dx + dy * dy <= r_sq {
                hits += 1;
            }
        }
    }
    (hits * 255 / 64) as u8
}

/// A straight color under a 0..=255 coverage factor: channels scale by
/// coverage, 255 is the identity. Partial pixels are premultiplied by
/// coverage so the compositor blends the corner against any backdrop.
fn coverage_argb(color: u32, coverage: u8) -> u32 {
    if coverage == 255 {
        return color;
    }
    let c = coverage as u32;
    let scale = |shift: u32| -> u32 { (((color >> shift) & 0xFF) * c + 127) / 255 };
    (scale(24) << 24) | (scale(16) << 16) | (scale(8) << 8) | scale(0)
}

/// Composites one premultiplied-ARGB pixel over the canvas (src over
/// dst). Full-alpha source replaces; anything else blends.
fn over_pixel(canvas: &mut [u8], stride: u32, x: u32, y: u32, src: u32) {
    let sa = src >> 24;
    let index = y as usize * stride as usize + x as usize * 4;
    let Some(slot) = canvas.get_mut(index..index + 4) else {
        return;
    };
    if sa == 255 {
        slot.copy_from_slice(&src.to_le_bytes());
        return;
    }
    let dst = u32::from_le_bytes(slot.try_into().expect("pixel is four bytes"));
    let out_a = sa + ((dst >> 24) * (255 - sa) + 127) / 255;
    let blend = |sc: u32, dc: u32| -> u32 { sc + ((dc * (255 - sa) + 127) / 255) };
    let out = (out_a << 24)
        | (blend((src >> 16) & 0xFF, (dst >> 16) & 0xFF) << 16)
        | (blend((src >> 8) & 0xFF, (dst >> 8) & 0xFF) << 8)
        | blend(src & 0xFF, dst & 0xFF);
    slot.copy_from_slice(&out.to_le_bytes());
}

/// Composites a premultiplied-ARGB bitmap 1:1 over the canvas at device
/// coordinates (`src` over `dst`, both premultiplied at the boundary).
fn blit_bitmap(canvas: &mut [u8], stride: u32, bitmap: &IconBitmap, x: u32, y: u32) {
    for py in 0..bitmap.side {
        for px in 0..bitmap.side {
            let src = bitmap.argb[(py * bitmap.side + px) as usize];
            over_pixel(canvas, stride, x + px, y + py, src);
        }
    }
}

/// Writes one ARGB8888 pixel; out-of-canvas coordinates are skipped.
fn put_pixel(canvas: &mut [u8], stride: u32, x: u32, y: u32, argb: u32) {
    let index = y as usize * stride as usize + x as usize * 4;
    if let Some(slot) = canvas.get_mut(index..index + 4) {
        slot.copy_from_slice(&argb.to_le_bytes());
    }
}

/// Interpolates one painted pixel toward `src` by `coverage`/255: the
/// anti-aliasing ramp between two fills. Full coverage is the exact
/// source color, zero coverage the untouched destination — and because
/// both fills are near-opaque, the ramp never thins the card's alpha.
fn mix_pixel(dst: u32, src: u32, coverage: u8) -> u32 {
    let c = coverage as u32;
    let mix = |sc: u32, dc: u32| (sc * c + dc * (255 - c) + 127) / 255;
    (mix(src >> 24, dst >> 24) << 24)
        | (mix((src >> 16) & 0xFF, (dst >> 16) & 0xFF) << 16)
        | (mix((src >> 8) & 0xFF, (dst >> 8) & 0xFF) << 8)
        | mix(src & 0xFF, dst & 0xFF)
}

/// Paints one shaped run through the System backend: each glyph's
/// cached-or-rasterized mask composites over the canvas at its
/// whole-device-pixel position — pen `x_logical`, baseline
/// `baseline_logical` — with every pixel past `edge_logical` (logical)
/// skipped, the title-band guard the Bitmap loop's break has always
/// been. Coverage blends toward `color` by the mask's ramp (grayscale
/// AA, the corners' own arithmetic); a color-bitmap glyph (the fallback
/// chains' emoji pick) paints its own RGB under its alpha instead — a
/// small image, exactly the honesty the themed icons practice.
// Eight paint inputs, the render seam's own (canvas, stride, scale,
// run, color, position, edge) — the Bitmap twin's draw_glyph signature
// flattened one level up.
#[expect(clippy::too_many_arguments)]
fn paint_run(
    system: &mut crate::text::TextSystem,
    run: &crate::text::GlyphRun,
    canvas: &mut [u8],
    stride: u32,
    scale: u32,
    color: u32,
    x_logical: u32,
    baseline_logical: u32,
    edge_logical: u32,
) {
    let pen = i64::from(x_logical) * i64::from(scale);
    let baseline = i64::from(baseline_logical) * i64::from(scale);
    let edge = i64::from(edge_logical) * i64::from(scale);
    for glyph in &run.glyphs {
        // The immutable mask borrow outlives only the pixel loop — the
        // canvas is not the system's, so no borrow crosses a mutation.
        let Some(mask) = system.mask(&glyph.cache_key) else {
            continue; // nothing rasterizable: skip, as cosmic's own renderer would
        };
        for row in 0..mask.height as i64 {
            for col in 0..mask.width as i64 {
                let index = (row * mask.width as i64 + col) as usize;
                let coverage = mask.coverage[index];
                if coverage == 0 {
                    continue;
                }
                let x = pen + i64::from(glyph.x) + i64::from(mask.left) + col;
                if x < 0 || x >= edge {
                    continue; // the band guard
                }
                let y = baseline + i64::from(glyph.y) + i64::from(mask.top) + row;
                let pixel = y * i64::from(stride) + x * 4;
                if pixel < 0 {
                    continue;
                }
                let Some(slot) = canvas.get_mut(pixel as usize..pixel as usize + 4) else {
                    continue; // off the canvas: an oversized canvas is harmless
                };
                let dst = u32::from_le_bytes(slot.try_into().expect("pixel is four bytes"));
                let paint = mask.colors.as_ref().map_or(color, |rgb| {
                    let [r, g, b] = rgb[index];
                    0xFF00_0000 | (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)
                });
                let out = mix_pixel(dst, paint, coverage);
                slot.copy_from_slice(&out.to_le_bytes());
            }
        }
    }
}

/// Draws one 8×8 glyph in `text` color with `block` device pixels per
/// font pixel at device coordinates (`x`, `y`).
fn draw_glyph(
    canvas: &mut [u8],
    stride: u32,
    glyph: [u8; 8],
    x: u32,
    y: u32,
    block: u32,
    text: u32,
) {
    for (row, bits) in glyph.iter().enumerate() {
        for col in 0..8 {
            if bits & (1 << col) != 0 {
                for dy in 0..block {
                    for dx in 0..block {
                        put_pixel(
                            canvas,
                            stride,
                            x + col * block + dx,
                            y + row as u32 * block + dy,
                            text,
                        );
                    }
                }
            }
        }
    }
}

/// Paints `pips` count squares — [`PIP_SIDE`] on a side, [`PIP_STEP`]
/// centers apart — starting at `first_x` with their top at `y`, in
/// `color`. The one count vocabulary shared by the horizontal tiles and
/// the grouped rows' parent entries.
fn paint_pips(
    canvas: &mut [u8],
    stride: u32,
    scale: u32,
    first_x: u32,
    y: u32,
    pips: u32,
    color: u32,
) {
    for n in 0..pips {
        for dy in 0..PIP_SIDE {
            for dx in 0..PIP_SIDE {
                let (lx, ly) = (first_x + n * PIP_STEP + dx, y + dy);
                for sy in 0..scale {
                    for sx in 0..scale {
                        put_pixel(canvas, stride, lx * scale + sx, ly * scale + sy, color);
                    }
                }
            }
        }
    }
}

/// Glyph bitmap for one character: ASCII printables come from [`FONT`],
/// the ellipsis is [`ELLIPSIS_GLYPH`], typographic punctuation folds to
/// its ASCII lookalike ([`fold_punct`]), everything else renders as `?`
/// — the per-glyph backstop behind [`fold_title`]'s string-level fold.
fn glyph(ch: char) -> [u8; 8] {
    match fold_punct(ch) {
        '…' => ELLIPSIS_GLYPH,
        '·' | '•' => MIDDLE_DOT_GLYPH,
        ch if ch.is_ascii() && !ch.is_ascii_control() => FONT[(ch as usize) - 0x20],
        _ => FONT[('?' as usize) - 0x20],
    }
}

/// Folds the typographic punctuation window titles routinely carry onto
/// its ASCII lookalike: `main.rs — airlock` and `Notes “draft”` render as
/// punctuation, not as a row of `?` where a title's own separator went.
/// At the 8×8 cell the fold is invisible — the ASCII glyph and a
/// purpose-drawn one differ by pixels, not shape. Every fold is one char
/// to one char, so column math is unchanged. Letters fold at the string
/// level in [`fold_title`]; whatever survives both folds falls through
/// to `?` in [`glyph`] — the honest floor of an ASCII font, stated
/// rather than papered over with lookalike letters.
fn fold_punct(ch: char) -> char {
    match ch {
        // Hyphen, figure dash, en dash, em dash, horizontal bar, minus.
        '‐' | '‑' | '‒' | '–' | '—' | '―' | '−' => '-',
        '‘' | '’' | '‛' => '\'',
        '“' | '”' | '‟' => '"',
        // Every Unicode space is a space.
        '\u{00A0}'
        | '\u{1680}'
        | '\u{2000}'..='\u{200A}'
        | '\u{202F}'
        | '\u{205F}'
        | '\u{3000}' => ' ',
        _ => ch,
    }
}

/// Latin Extended-A members whose canonical decomposition is an ASCII
/// base plus a combining mark, as `(first, last, upper, lower)` — the
/// block pairs upper/lower alternately from each entry's first, which
/// handles `Ÿ`'s run of capitals at the top of the Z block (its `ÿ`
/// lives in Latin-1, so `Ź` is odd-numbered). Strokes (`Đ Ħ Ł Ŧ`),
/// ligatures (`Ĳ Œ`), and letterforms with no ASCII base (`ı ŉ ſ`) are
/// out by construction: folding them would be invention, not
/// decomposition.
const LATIN_EXTENDED_A: &[(u32, u32, char, char)] = &[
    (0x0100, 0x0105, 'A', 'a'),
    (0x0106, 0x010D, 'C', 'c'),
    (0x010E, 0x010F, 'D', 'd'),
    (0x0112, 0x011B, 'E', 'e'),
    (0x011C, 0x0123, 'G', 'g'),
    (0x0124, 0x0125, 'H', 'h'),
    (0x0128, 0x0130, 'I', 'i'),
    (0x0134, 0x0135, 'J', 'j'),
    (0x0136, 0x0137, 'K', 'k'),
    (0x0139, 0x013E, 'L', 'l'),
    (0x0143, 0x0148, 'N', 'n'),
    (0x014C, 0x0151, 'O', 'o'),
    (0x0154, 0x0159, 'R', 'r'),
    (0x015A, 0x0161, 'S', 's'),
    (0x0162, 0x0165, 'T', 't'),
    (0x0168, 0x0173, 'U', 'u'),
    (0x0174, 0x0175, 'W', 'w'),
    (0x0176, 0x0178, 'Y', 'y'),
    (0x0179, 0x017E, 'Z', 'z'),
];

/// The ASCII base of a precomposed Latin letter whose canonical
/// decomposition is that base plus a combining mark: `é` → `e`. `None`
/// for everything else, so [`fold_title`] passes it through untouched.
fn fold_diacritic(ch: char) -> Option<char> {
    let base = match ch {
        // Latin-1 Supplement letters. `×` and `÷` are math signs, and
        // `ß æ ð ø þ` have no ASCII base — all of them stay `?`.
        'À'..='Å' => 'A',
        'Ç' => 'C',
        'È'..='Ë' => 'E',
        'Ì'..='Ï' => 'I',
        'Ñ' => 'N',
        'Ò'..='Ö' => 'O',
        'Ù'..='Ü' => 'U',
        'Ý' => 'Y',
        'à'..='å' => 'a',
        'ç' => 'c',
        'è'..='ë' => 'e',
        'ì'..='ï' => 'i',
        'ñ' => 'n',
        'ò'..='ö' => 'o',
        'ù'..='ü' => 'u',
        'ý' | 'ÿ' => 'y',
        _ => {
            let cp = ch as u32;
            let &(first, _, upper, lower) = LATIN_EXTENDED_A
                .iter()
                .find(|&&(first, last, _, _)| (first..=last).contains(&cp))?;
            return Some(if (cp - first).is_multiple_of(2) {
                upper
            } else {
                lower
            });
        }
    };
    Some(base)
}

/// Folds a display title onto the card's renderable text: precomposed
/// Latin letters with an ASCII canonical base fold to it (`é` → `e`),
/// combining marks are dropped so NFD decompositions fold too (a
/// macOS-built `café.txt` reads `cafe.txt`), and [`fold_punct`] still
/// handles punctuation per glyph. Runs before [`truncate_cols`], so the
/// deleted marks never spend columns. Everything else — CJK, Cyrillic,
/// stroke letters, ligatures — still falls through to `?` in
/// [`glyph`]. The fold deletes a phonemic distinction: `café.txt` and
/// `cafe.txt` render identically by decision, and that is documented
/// degradation (README, the design skill), not fidelity.
/// 2026-11-07 typeset pass, the critique's user-approved item.
pub fn fold_title(title: &str) -> String {
    title
        .chars()
        .filter(|ch| !matches!(u32::from(*ch), 0x0300..=0x036F))
        .map(|ch| fold_diacritic(ch).unwrap_or(ch))
        .collect()
}

/// The title shown for a toplevel: its title if non-blank, else its
/// app_id, else a placeholder. The horizontal tiles' unclamped label
/// since glance-tiles G1 (2026-11-09) — nothing paints it at the
/// compact pitch; the caption composer (G3) reads the commit target's
/// title through this rule and truncates against its own budget.
pub fn display_title(title: &str, app_id: &str) -> String {
    if !title.trim().is_empty() {
        title.trim().to_string()
    } else if !app_id.trim().is_empty() {
        app_id.trim().to_string()
    } else {
        UNTITLED.to_string()
    }
}

/// Caps a title at `cap` columns, marking the cut with `…`. The cap is
/// [`MAX_COLS`] for flat rows; grouped rows pay for the child indent and
/// the parent's caret out of the same budget, so the card's width
/// ceiling holds in the tree too.
fn truncate_cols(title: String, cap: u32) -> String {
    if title.chars().count() > cap as usize {
        let keep: String = title.chars().take(cap as usize - 1).collect();
        format!("{keep}…")
    } else {
        title
    }
}

/// The chords addressing the first nine rows, in row order — the render
/// twin of `key_input`'s Ctrl+digit mapping. The pairing is unit-pinned:
/// a label that outlives its wiring would be a dead key advertised on
/// the card.
/// How many rows the number chords reach: Ctrl+1..9 at level 1,
/// Alt+1..9 within a group — the tenth and deeper are never hinted and
/// never wired.
const MAX_HINTS: usize = 9;

/// Keys the switcher gesture reacts to, normalized from keysyms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GestureKey {
    /// Tab / Shift+Tab: one visible row, linearly, in either mode —
    /// ↓/↑'s twin since grouping rev 5 (the level-scoped step is gone).
    Tab,
    /// Explicit confirm: Return / KP_Enter / ISO_Enter while showing.
    /// Same outcome as releasing Alt.
    Commit,
    /// A directional step: Up/Left previous, Down/Right next.
    Arrow(Direction),
    /// Ctrl+1..9 (or keypad 1..9): select and activate row `n` (0-based,
    /// top row = 0) in one motion — the keyboard twin of a pointer
    /// click. Produced by the pure [`key_input`]; the wayland boundary
    /// rewrites it to [`GestureKey::SelectHandle`] — the row resolved
    /// against the event-time model — before queueing, so no bare row
    /// ever crosses the queue as an activation target.
    Select(usize),
    /// Grouped mode's Ctrl+1..9: address the Nth *level-1* entry
    /// (0-based) — an app entry or a leaf — committing its implied
    /// window. The boundary resolves it against the event-time model
    /// exactly as [`GestureKey::Select`] resolves a flat row.
    SelectApp(usize),
    /// Tab in the grouped-horizontal card: step the next/previous app
    /// (band 1) from an app tile — the spec's "Tab steps apps". Inside
    /// an open run the daemon joins the linear walk instead, so the
    /// run stays open while Tab walks its windows (the vertical Tab's
    /// behavior). Distinct from [`GestureKey::Arrow`], which steps
    /// *within* the focused band in that mode. The daemon resolves it
    /// against the frozen tree.
    StepApp(Direction),
    /// The key above Tab (keysym `grave`, `` ` `` on US layouts) in
    /// either grouped card: the focused app's key. From the app's
    /// entry or tile it toggles the group — open on the MRU-front
    /// window (Shift on the last) when collapsed, collapsed in place
    /// when open; from a window row or tile it cycles the app's
    /// windows with wrap, never crossing into another app. The daemon
    /// resolves it against the session state — the selection's level
    /// and the app's expanded flag are invisible to this pure map, and
    /// a standing query reroutes it to the type-into path there.
    ToggleOrCycle(Direction),
    /// The handle-carrying twin of the select family, produced only
    /// by the wayland boundary at event receipt: the window of the row
    /// the user addressed. `None` — a continuation row, or a row past
    /// the model's end — flows through and no-ops at the daemon,
    /// leaving the session intact.
    SelectHandle(Option<Handle>),
    /// → in grouped mode: expand the selected parent and enter its
    /// children (the W3C expand axis). The daemon resolves the target
    /// against the frozen tree.
    EnterChild,
    /// ← in grouped mode: exit to the selected child's parent row, or
    /// collapse the selected expanded parent (the W3C collapse axis).
    /// The daemon resolves the target against the frozen tree.
    ExitParent,
    /// A printable keypress (`type-filter`): the character types into
    /// the session's query, whatever modifiers report — Alt is held
    /// the whole gesture, Shift is how uppercase types. Chord arms win
    /// ahead of it in `key_input`.
    Type(char),
    /// Backspace: erase the query's last character. Never dismisses —
    /// on an empty query it is a no-op.
    Backspace,
    /// Alt+minus (top row or keypad, `close-window`): close the
    /// selected row's window — the row's resolved handle, a group
    /// header's front window included. The daemon advances the
    /// selection before the request and keeps the session open.
    Close,
    Escape,
    Other,
}

/// One normalized input into the overlay gesture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GestureInput {
    /// A stub invocation: the user pressed the keybind (`alt-tab` or
    /// `shift-alt-tab`) outside the overlay.
    Invoke(Direction),
    /// A key press on the grabbed overlay keyboard; `shift` carries the
    /// seat's last reported shift state (or an ISO_Left_Tab keysym).
    Press { key: GestureKey, shift: bool },
    /// The seat's modifier state changed; only Alt matters here.
    Modifiers { alt: bool },
    /// Keyboard focus left the overlay surface.
    FocusLost,
}

/// Pointer inputs the switcher gesture reacts to, normalized from
/// wl_pointer events on the overlay surface. The selection target is
/// captured at event-receipt time — the handle of the row the user saw —
/// so nothing draining earlier in the same batch can retarget it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerInput {
    /// Cursor over a row (enter or motion) or gone (leave / gap /
    /// off-card). `row` drives the pure crossing policy only; `handle`
    /// is the event-time selection target — `None` on continuation
    /// rows, gaps, and leaves, and those never select.
    Hover {
        row: Option<usize>,
        handle: Option<Handle>,
    },
    /// Left-button press on a content row: select and commit in one
    /// motion, addressing the handle captured at the press. Presses on
    /// continuation rows queue nothing.
    Click { handle: Handle },
    /// Left-button press inside the hovered row's × zone
    /// (`close-window`): close that row's window — the handle captured
    /// at the press, the same event-time resolution
    /// [`PointerInput::Click`] carries. The boundary's click split
    /// produces this only on the vertical card's window rows; every
    /// other press activates.
    Close { handle: Handle },
    /// One discrete wheel notch, riding the existing [`Direction`]
    /// vocabulary.
    Wheel(Direction),
}

/// Pure hover-selection policy: which [`PointerInput::Hover`]
/// observations may move the overlay's selection.
///
/// Only row *crossings* select — presence alone never steals the
/// MRU-first selection, because the overlay maps center-screen, often
/// directly under a resting cursor. The first observation after a
/// [`reset`](Self::reset) seeds the baseline and selects nothing: a
/// surface's first pointer event is always the Enter that map-time
/// contact (or first arrival) produces, and neither expresses crossing
/// intent. A `None` observation — a gap, or leaving the card — never
/// selects but forgets the row, so returning to it re-asserts pointer
/// intent.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PointerHover {
    /// The row the pointer was last observed over; forgotten by a `None`
    /// observation.
    last_row: Option<usize>,
    /// Whether any observation has arrived since the last reset. The
    /// first is map-time contact: it seeds `last_row` and selects
    /// nothing — even when it lands in a gap.
    entered: bool,
}

impl PointerHover {
    /// Applies one observation, returning the row to select — only when
    /// it is concrete and differs from the last observed row.
    pub fn observe(&mut self, row: Option<usize>) -> Option<usize> {
        let crossing = self.entered && row.is_some() && row != self.last_row;
        self.entered = true;
        self.last_row = row;
        if crossing { row } else { None }
    }

    /// Clears all history so the next overlay session seeds its first
    /// contact instead of selecting.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// What the daemon should do in response to a [`GestureInput`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GestureOutput {
    /// Move the selection one window in this direction and refresh the
    /// overlay.
    Step(Direction),
    /// → in grouped mode: expand the selected parent and move the
    /// selection to its enter-target child. The daemon resolves against
    /// the frozen tree; a leaf's enter is the W3C no-op.
    EnterChild,
    /// ← in grouped mode: move the selection to the selected child's
    /// parent row, or collapse the selected expanded parent. The daemon
    /// resolves against the frozen tree; a collapsed top-level row's
    /// exit is the W3C no-op.
    ExitParent,
    /// Tab in the grouped-horizontal card: step the next/previous app
    /// tile (band 1) — from inside an open run the daemon joins the
    /// linear walk instead, keeping the run open. The daemon resolves
    /// it against the frozen tree.
    StepApp(Direction),
    /// The key above Tab in either grouped card: toggle the selected
    /// app's group from a closed entry — opening on its MRU-front
    /// window — or collapse it when open, and cycle the app's windows
    /// from inside the group. The daemon holds the selection's level
    /// and the app's expanded flag; a standing query reroutes the press
    /// to the type-into path, keeping this map pure.
    ToggleOrCycle(Direction),
    /// A character types into the session's query (`type-filter`): the
    /// daemon narrows the card's frozen window set by it and reselects
    /// the first match.
    Type(char),
    /// Backspace erases the query's last character (a no-op on an empty
    /// query); the daemon rebuilds the card.
    Erase,
    /// Close the selected row's window (`close-window`): the daemon
    /// resolves the row's handle, advances the selection off it, sends
    /// the compositor request, and keeps the session open for repeated
    /// closes — dismissing when nothing eligible remains.
    CloseSelected,
    /// Activate the selection and unmap the overlay.
    Commit,
    /// Select and activate the captured window in one motion — the
    /// handle was fixed at event receipt against the model the user
    /// saw. The gesture session survives the output: the daemon closes
    /// it only when the activation actually happens, so a `None`
    /// handle (a continuation or out-of-range row) leaves the gesture
    /// fully intact.
    Activate {
        handle: Option<Handle>,
    },
    /// Escape was pressed. Since `type-filter`, the machine cannot
    /// decide alone: with a query standing, the press clears the query
    /// and keeps the session; only an empty query dismisses. The daemon
    /// — which holds the session state — decides and closes the
    /// gesture itself when it dismisses.
    Escape,
    /// Unmap the overlay without switching.
    Dismiss,
    Nothing,
}

/// The hold-Alt gesture as a pure state machine.
///
/// The daemon feeds it normalized inputs and applies the outputs, so the
/// full gesture — invoke, cycle, release-commit, quick-tap commit, escape,
/// focus loss — is unit-testable without a compositor.
///
/// Release-capture rides on [`GestureInput::Modifiers`]: with the overlay's
/// keyboard grabbed, the compositor reports modifier changes, so Alt going
/// away is the commit signal. The first report without Alt commits too — a
/// quick Alt+Tab tap releases before the overlay maps, and must still
/// switch.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Gesture {
    showing: bool,
    alt: bool,
}

impl Gesture {
    /// Applies one input, returning what the daemon should do.
    pub fn feed(&mut self, input: GestureInput) -> GestureOutput {
        match input {
            GestureInput::Invoke(direction) => {
                self.showing = true;
                GestureOutput::Step(direction)
            }
            GestureInput::Press { key, shift } if self.showing => match key {
                GestureKey::Tab => GestureOutput::Step(if shift {
                    Direction::Previous
                } else {
                    Direction::Next
                }),
                GestureKey::Commit => {
                    self.showing = false;
                    GestureOutput::Commit
                }
                GestureKey::Arrow(direction) => GestureOutput::Step(direction),
                // Grouped mode's tree keys. The machine cannot resolve
                // them — the targets live in the daemon's frozen tree
                // and expansion state — so it forwards them as outputs
                // exactly as it forwards arrows.
                GestureKey::EnterChild => GestureOutput::EnterChild,
                GestureKey::ExitParent => GestureOutput::ExitParent,
                // Tab in the combined card steps apps, not the focused
                // band's tiles — a distinct output so the daemon can
                // route it.
                GestureKey::StepApp(direction) => GestureOutput::StepApp(direction),
                // The key above Tab works the focused app's group —
                // toggle from its entry, cycle from inside — forwarded
                // the same way; the daemon holds the level, the window
                // set, and the expanded flag.
                GestureKey::ToggleOrCycle(direction) => GestureOutput::ToggleOrCycle(direction),
                // The query edits (`type-filter`): session state, not a
                // row address — forwarded for the daemon to apply.
                GestureKey::Type(ch) => GestureOutput::Type(ch),
                GestureKey::Backspace => GestureOutput::Erase,
                // The close key (`close-window`): the daemon resolves
                // the selection's window and owns the advance-then-close
                // ordering.
                GestureKey::Close => GestureOutput::CloseSelected,
                // Select-and-activate in one motion, addressing the
                // event-time handle; the session stays open — the daemon
                // closes it via `close()` only when the activation
                // actually happens, so an unaddressable row (`None`)
                // leaves the gesture fully intact.
                GestureKey::SelectHandle(handle) => GestureOutput::Activate { handle },
                // Bare addresses are the pure translation's shape, never
                // the queue's: the boundary rewrites each to its
                // [`GestureKey::SelectHandle`] twin before queueing, so
                // the machine never activates off a row or level index.
                GestureKey::Select(_) | GestureKey::SelectApp(_) => GestureOutput::Nothing,
                // The daemon decides: a standing query clears first, an
                // empty one dismisses (and the daemon closes the
                // gesture then — the machine's own `showing` stays true
                // until that close, so a cleared query keeps typing).
                GestureKey::Escape => GestureOutput::Escape,
                GestureKey::Other => GestureOutput::Nothing,
            },
            GestureInput::Modifiers { alt } => {
                self.alt = alt;
                if self.showing && !alt {
                    self.showing = false;
                    GestureOutput::Commit
                } else {
                    GestureOutput::Nothing
                }
            }
            GestureInput::FocusLost => {
                if self.showing {
                    self.showing = false;
                    GestureOutput::Dismiss
                } else {
                    GestureOutput::Nothing
                }
            }
            // Key presses mean nothing between sessions: the overlay is
            // unmapped, so none can arrive from our surface anyway.
            GestureInput::Press { .. } => GestureOutput::Nothing,
        }
    }

    /// Whether a gesture session is open (the overlay should be mapped).
    /// Tests assert visibility directly; the daemon mirrors the session
    /// with its ring instead of asking (dead in the binary, public API in
    /// the dev lib target — an `allow`, not an `expect`, for exactly that
    /// reason).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn showing(&self) -> bool {
        self.showing
    }

    /// Whether the last modifier report had Alt held. While true, the idle
    /// close deadline is suspended: the user is mid-gesture and quiet.
    pub fn alt_held(&self) -> bool {
        self.alt
    }

    /// Forces the session closed without further output, for daemon-side
    /// closes the input stream cannot express (the idle and hard timeouts).
    pub fn close(&mut self) {
        self.showing = false;
    }
}

/// Normalizes a grabbed key press into gesture input, grouping-aware.
///
/// Shift+Tab arrives as ISO_Left_Tab on most layouts and as Tab plus the
/// shift modifier on others: both step backwards. `shift_held`,
/// `ctrl_held`, and `alt_held` are the seat's last reported modifier
/// states — Ctrl classifies digit presses, exactly as shift classifies
/// Tab.
///
/// Grouped mode (the owner's 2026-11-08 rev-5 simplification): Tab
/// steps one visible row like ↓/↑ — headers included, collapsed
/// children never; → expands and enters ([`GestureKey::EnterChild`]), ←
/// exits or collapses ([`GestureKey::ExitParent`]); Ctrl+N addresses
/// level-1 entries ([`GestureKey::SelectApp`]). Flat mode
/// (`grouped = false`) keeps the flat mapping byte for byte — Tab and
/// ←/→ step rows — pinned by the flat regression tests. The boundary
/// passes the live model's own mode flag, so the keysym the user
/// pressed maps against the list they see.
///
/// Type-to-filter (`type-filter`, 2026-11-08): a printable keysym types
/// into the session's query through every modifier; Backspace erases.
/// Chord arms win ahead of the printable arm — only Ctrl+N survives.
/// The overlay's key-map mode — which card shape the grabbed keys map
/// against: the flat list (vertical or horizontal), the vertical grouped
/// tree, or the grouped-horizontal combination (the axis-swapped card).
/// The boundary passes the live overlay's mode, so a keysym maps against
/// the list the user sees.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyMode {
    /// The flat list: Tab/↑/↓/←/→ all step rows, Ctrl+N addresses rows.
    Flat,
    /// The vertical grouped tree (grouping rev 5): Tab/↑/↓ step the
    /// linear walk, →/← expand/exit, the key above Tab steps the
    /// focused app's windows, Ctrl+N addresses level-1 entries.
    Grouped,
    /// The grouped-horizontal combination: ←/→ step within the focused
    /// band, ↓/↑ enter/exit band 2, Tab steps apps, Ctrl+N addresses
    /// the visible tiles in reading order.
    GroupedHorizontal,
}

/// Normalizes a grabbed key press into gesture input, mode-aware.
///
/// Shift+Tab arrives as ISO_Left_Tab on most layouts and as Tab plus the
/// shift modifier on others: both step backwards. `shift_held`,
/// `ctrl_held`, and `alt_held` are the seat's last reported modifier
/// states — Ctrl classifies digit presses, exactly as shift classifies
/// Tab.
///
/// Grouped mode (the owner's 2026-11-08 rev-5 simplification): Tab
/// steps one visible row like ↓/↑ — headers included, collapsed
/// children never; → expands and enters ([`GestureKey::EnterChild`]), ←
/// exits or collapses ([`GestureKey::ExitParent`]); the key above Tab
/// works the focused app's group ([`GestureKey::ToggleOrCycle`]); Ctrl+N
/// addresses level-1 entries ([`GestureKey::SelectApp`]). Flat mode
/// keeps the flat mapping byte for byte — Tab and ←/→ step rows —
/// pinned by the flat regression tests. The boundary passes the live
/// model's own mode, so the keysym the user pressed maps against the
/// list they see.
///
/// Grouped-horizontal (the combination): ←/→ step the focused band's
/// tiles, ↓ enters band 2 from an app tile ([`GestureKey::EnterChild`]),
/// ↑ exits band 2 to its app tile ([`GestureKey::ExitParent`]), Tab
/// steps apps from an app tile and joins the linear walk inside an
/// open run ([`GestureKey::StepApp`]), the key above
/// Tab works the focused app's group ([`GestureKey::ToggleOrCycle`],
/// grave alone — every other keysym still types), and Ctrl+N
/// addresses the visible tiles in reading order ([`GestureKey::Select`]
/// — band 1 left→right then band 2 left→right, exactly as the card
/// concatenates them).
///
/// Type-to-filter (`type-filter`, 2026-11-08): a printable keysym types
/// into the session's query through every modifier; Backspace erases.
/// Chord arms win ahead of the printable arm — only Ctrl+N survives.
pub fn key_input(
    keysym: Keysym,
    shift_held: bool,
    ctrl_held: bool,
    // Alt classifies the close key (minus); typing deliberately ignores
    // it — Alt is held the whole gesture.
    alt_held: bool,
    mode: KeyMode,
) -> GestureInput {
    use KeyMode::{Flat, Grouped, GroupedHorizontal};
    let (key, shift) = match keysym {
        Keysym::ISO_Left_Tab => match mode {
            GroupedHorizontal => (GestureKey::StepApp(Direction::Previous), true),
            _ => (GestureKey::Tab, true),
        },
        Keysym::Tab => match mode {
            GroupedHorizontal => (
                GestureKey::StepApp(if shift_held {
                    Direction::Previous
                } else {
                    Direction::Next
                }),
                shift_held,
            ),
            _ => (GestureKey::Tab, shift_held),
        },
        Keysym::Return | Keysym::KP_Enter | Keysym::ISO_Enter => (GestureKey::Commit, shift_held),
        Keysym::Up => match mode {
            // Combined: ↑ exits band 2 to its app tile; vertical grouped
            // and flat step the linear walk.
            GroupedHorizontal => (GestureKey::ExitParent, shift_held),
            _ => (GestureKey::Arrow(Direction::Previous), false),
        },
        Keysym::Down => match mode {
            GroupedHorizontal => (GestureKey::EnterChild, shift_held),
            _ => (GestureKey::Arrow(Direction::Next), false),
        },
        Keysym::Left => match mode {
            Grouped => (GestureKey::ExitParent, shift_held),
            _ => (GestureKey::Arrow(Direction::Previous), false),
        },
        Keysym::Right => match mode {
            Grouped => (GestureKey::EnterChild, shift_held),
            _ => (GestureKey::Arrow(Direction::Next), false),
        },
        Keysym::Escape => (GestureKey::Escape, shift_held),
        // Backspace edits the query; never dismisses (the daemon treats
        // it as a no-op on an empty query).
        Keysym::BackSpace => (GestureKey::Backspace, shift_held),
        // Alt+minus closes the selected row's window (`close-window`),
        // top row or keypad. Shift+minus — the underscore keysym — is a
        // printable, not the key.
        Keysym::minus | Keysym::KP_Subtract if alt_held => (GestureKey::Close, shift_held),
        // The key above Tab (`` `` ``) works the focused app's group in
        // every grouped card — the strip and the vertical tree alike —
        // and only there: flat sessions keep typing it into the query
        // (the type-filter contract). Both of the key's keysyms map: the
        // compositor reports the level-resolved symbol, so Shift+` on a
        // US layout arrives as `asciitilde`, never `grave` — matching
        // `grave` alone left Alt+Shift+` unmatchable at runtime (the
        // shifted-printable twin of the underscore arm below). The
        // daemon resolves the level and the flag — a closed entry opens
        // on the MRU-front window (Shift on the last), an expanded entry
        // collapses in place, and a window row or tile cycles the app's
        // windows — and reroutes to the type-into path while a query
        // stands.
        Keysym::grave | Keysym::asciitilde if matches!(mode, GroupedHorizontal | Grouped) => (
            GestureKey::ToggleOrCycle(if shift_held {
                Direction::Previous
            } else {
                Direction::Next
            }),
            shift_held,
        ),
        // Ctrl+digits map with or without shift held; without Ctrl the
        // number row stays Other — it must never surprise-activate. In
        // vertical grouped mode the chord addresses level-1 entries; in
        // flat and combined it addresses rows (the strip's rows are
        // already the reading order).
        _ if ctrl_held && let Some(row) = digit_row(keysym) => (
            match mode {
                Grouped => GestureKey::SelectApp(row),
                Flat | GroupedHorizontal => GestureKey::Select(row),
            },
            shift_held,
        ),
        // The query (`type-filter`): a printable keysym types through
        // every modifier — Alt is held the whole gesture, Shift is how
        // uppercase types — and the chord arms above have already had
        // their pick. Control characters (Tab, Return, Escape, the
        // keypad's enter/tab) never type: their dedicated arms caught
        // them, and the filter drops the rest.
        _ if let Some(ch) = printable(keysym) => (GestureKey::Type(ch), shift_held),
        _ => (GestureKey::Other, shift_held),
    };
    GestureInput::Press { key, shift }
}

/// The character a keysym types into the query, if it types at all:
/// `key_char`'s mapping (the xkb_keysym_to_utf32 rule) minus control
/// characters. Layout-correct by construction — the keysym is what the
/// layout produced.
fn printable(keysym: Keysym) -> Option<char> {
    keysym.key_char().filter(|ch| !ch.is_control())
}

/// The 0-based overlay row a digit keysym addresses: top-row `1..=9`
/// (0x31..=0x39) and keypad `KP_1..KP_9` (0xffb1..=0xffb9) are row
/// `digit - 1`. Everything else — `0` included (no Ctrl+0-as-10) — is
/// not number-addressable. Matching rides the raw codes because a
/// range pattern over the named consts is not something xkeysym's
/// associated constants promise to compile.
fn digit_row(keysym: Keysym) -> Option<usize> {
    let code = keysym.raw();
    match code {
        0x31..=0x39 => Some((code - 0x31) as usize),
        0xffb1..=0xffb9 => Some((code - 0xffb1) as usize),
        _ => None,
    }
}

/// Saturating accumulate, bounded per event: no compositor value —
/// however buggy or hostile — can panic (debug) or enqueue an unbounded
/// step burst (release). ±32 notches in one event is far past any
/// physical device (a full-spin flick delivers a handful) and far below
/// the freeze regime; rotation outside the clamp is dropped, not
/// carried.
pub const WHEEL_MAX_NOTCHES_PER_EVENT: i32 = 32;

/// One wheel event's whole notches plus the sub-notch `value120` residual
/// to carry into the next event: the returned notches' sign is the
/// direction (positive = one [`Direction::Next`] step), its magnitude the
/// notch count, and the returned residual is what did not reach a whole
/// ±120. `value120` counts 1/120ths of a notch, so a physical notch may
/// arrive split across events (60+60, or a touchpad's stream of small
/// deltas) and only the carried residual keeps the step count linear in
/// the total rotation — the protocol pins 60+60 = 1 notch and 60−60 = 0
/// notches, so the residual keeps its sign. The deprecated `discrete`
/// step is the pre-v120 equivalent, is already whole notches, applies
/// only when the compositor sends no `value120`, and never touches the
/// residual; an event reporting neither field steps nothing.
///
/// Both branches are bounded per event at
/// [`WHEEL_MAX_NOTCHES_PER_EVENT`]: the v120 branch saturates before
/// the add and clamps before the divide, the legacy discrete count
/// clamps symmetrically — an axis value at the i32 rails steps at most
/// ±32 notches instead of panicking or freezing the enqueue loop, and
/// rotation beyond the clamp is dropped, not carried.
pub fn wheel_notches(value120: i32, discrete: i32, residual: i32) -> (i32, i32) {
    if value120 != 0 {
        let total = residual.saturating_add(value120).clamp(
            -WHEEL_MAX_NOTCHES_PER_EVENT * 120,
            WHEEL_MAX_NOTCHES_PER_EVENT * 120,
        );
        let notches = total / 120;
        (notches, total - notches * 120)
    } else {
        (
            discrete.clamp(-WHEEL_MAX_NOTCHES_PER_EVENT, WHEEL_MAX_NOTCHES_PER_EVENT),
            residual,
        )
    }
}

/// How close together two same-direction wheel notches may arrive and
/// still be one physical detent's machine-made echo: virtualized input
/// relays (a QEMU/spice mouse, seen in the COSMIC VM) have been observed
/// delivering a detent's click twice, 1 ms apart. No mechanical wheel
/// produces two detents in under ~18 ms even at full spin, so 8 ms
/// separates every echo from every real repeat with margin both ways.
pub const ECHO_WINDOW_MS: u32 = 8;

/// Whether this whole-notch wheel step is an echo of the last accepted
/// one — same direction, arriving within [`ECHO_WINDOW_MS`] of it — and
/// so should be dropped instead of stepping again.
///
/// `time_ms` is the axis event's own millisecond timestamp; a timestamp
/// that jumps backwards (clock skew in a relay) is never an echo, so a
/// glitch can only ever cost the suppression, never a real notch.
pub fn notch_is_echo(direction: Direction, time_ms: u32, last: Option<(Direction, u32)>) -> bool {
    match last {
        Some((prior_direction, prior_time)) => {
            prior_direction == direction
                && time_ms >= prior_time
                && time_ms - prior_time <= ECHO_WINDOW_MS
        }
        None => false,
    }
}

/// 8x8 bitmap glyphs for printable ASCII U+0020..=U+007E, one `[u8; 8]` per
/// glyph (a row per byte); bit `(1 << col)` is the pixel at column `col`, so
/// the least-significant bit is the leftmost pixel.
///
/// Font data from the font8x8 project (Daniel Hepper,
/// https://github.com/dhepper/font8x8), which dedicates the font tables to
/// the public domain; table transcribed from the `font8x8` Rust crate
/// v0.3.1 (MIT).
#[rustfmt::skip]
const FONT: [[u8; 8]; 95] = [
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], // (space)
    [0x18, 0x3C, 0x3C, 0x18, 0x18, 0x00, 0x18, 0x00], // !
    [0x36, 0x36, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], // "
    [0x36, 0x36, 0x7F, 0x36, 0x7F, 0x36, 0x36, 0x00], // #
    [0x0C, 0x3E, 0x03, 0x1E, 0x30, 0x1F, 0x0C, 0x00], // $
    [0x00, 0x63, 0x33, 0x18, 0x0C, 0x66, 0x63, 0x00], // %
    [0x1C, 0x36, 0x1C, 0x6E, 0x3B, 0x33, 0x6E, 0x00], // &
    [0x06, 0x06, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00], // '
    [0x18, 0x0C, 0x06, 0x06, 0x06, 0x0C, 0x18, 0x00], // (
    [0x06, 0x0C, 0x18, 0x18, 0x18, 0x0C, 0x06, 0x00], // )
    [0x00, 0x66, 0x3C, 0xFF, 0x3C, 0x66, 0x00, 0x00], // *
    [0x00, 0x0C, 0x0C, 0x3F, 0x0C, 0x0C, 0x00, 0x00], // +
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x0C, 0x0C, 0x06], // ,
    [0x00, 0x00, 0x00, 0x3F, 0x00, 0x00, 0x00, 0x00], // -
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x0C, 0x0C, 0x00], // .
    [0x60, 0x30, 0x18, 0x0C, 0x06, 0x03, 0x01, 0x00], // /
    [0x3E, 0x63, 0x73, 0x7B, 0x6F, 0x67, 0x3E, 0x00], // 0
    [0x0C, 0x0E, 0x0C, 0x0C, 0x0C, 0x0C, 0x3F, 0x00], // 1
    [0x1E, 0x33, 0x30, 0x1C, 0x06, 0x33, 0x3F, 0x00], // 2
    [0x1E, 0x33, 0x30, 0x1C, 0x30, 0x33, 0x1E, 0x00], // 3
    [0x38, 0x3C, 0x36, 0x33, 0x7F, 0x30, 0x78, 0x00], // 4
    [0x3F, 0x03, 0x1F, 0x30, 0x30, 0x33, 0x1E, 0x00], // 5
    [0x1C, 0x06, 0x03, 0x1F, 0x33, 0x33, 0x1E, 0x00], // 6
    [0x3F, 0x33, 0x30, 0x18, 0x0C, 0x0C, 0x0C, 0x00], // 7
    [0x1E, 0x33, 0x33, 0x1E, 0x33, 0x33, 0x1E, 0x00], // 8
    [0x1E, 0x33, 0x33, 0x3E, 0x30, 0x18, 0x0E, 0x00], // 9
    [0x00, 0x0C, 0x0C, 0x00, 0x00, 0x0C, 0x0C, 0x00], // :
    [0x00, 0x0C, 0x0C, 0x00, 0x00, 0x0C, 0x0C, 0x06], // ;
    [0x18, 0x0C, 0x06, 0x03, 0x06, 0x0C, 0x18, 0x00], // <
    [0x00, 0x00, 0x3F, 0x00, 0x00, 0x3F, 0x00, 0x00], // =
    [0x06, 0x0C, 0x18, 0x30, 0x18, 0x0C, 0x06, 0x00], // >
    [0x1E, 0x33, 0x30, 0x18, 0x0C, 0x00, 0x0C, 0x00], // ?
    [0x3E, 0x63, 0x7B, 0x7B, 0x7B, 0x03, 0x1E, 0x00], // @
    [0x0C, 0x1E, 0x33, 0x33, 0x3F, 0x33, 0x33, 0x00], // A
    [0x3F, 0x66, 0x66, 0x3E, 0x66, 0x66, 0x3F, 0x00], // B
    [0x3C, 0x66, 0x03, 0x03, 0x03, 0x66, 0x3C, 0x00], // C
    [0x1F, 0x36, 0x66, 0x66, 0x66, 0x36, 0x1F, 0x00], // D
    [0x7F, 0x46, 0x16, 0x1E, 0x16, 0x46, 0x7F, 0x00], // E
    [0x7F, 0x46, 0x16, 0x1E, 0x16, 0x06, 0x0F, 0x00], // F
    [0x3C, 0x66, 0x03, 0x03, 0x73, 0x66, 0x7C, 0x00], // G
    [0x33, 0x33, 0x33, 0x3F, 0x33, 0x33, 0x33, 0x00], // H
    [0x1E, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x1E, 0x00], // I
    [0x78, 0x30, 0x30, 0x30, 0x33, 0x33, 0x1E, 0x00], // J
    [0x67, 0x66, 0x36, 0x1E, 0x36, 0x66, 0x67, 0x00], // K
    [0x0F, 0x06, 0x06, 0x06, 0x46, 0x66, 0x7F, 0x00], // L
    [0x63, 0x77, 0x7F, 0x7F, 0x6B, 0x63, 0x63, 0x00], // M
    [0x63, 0x67, 0x6F, 0x7B, 0x73, 0x63, 0x63, 0x00], // N
    [0x1C, 0x36, 0x63, 0x63, 0x63, 0x36, 0x1C, 0x00], // O
    [0x3F, 0x66, 0x66, 0x3E, 0x06, 0x06, 0x0F, 0x00], // P
    [0x1E, 0x33, 0x33, 0x33, 0x3B, 0x1E, 0x38, 0x00], // Q
    [0x3F, 0x66, 0x66, 0x3E, 0x36, 0x66, 0x67, 0x00], // R
    [0x1E, 0x33, 0x07, 0x0E, 0x38, 0x33, 0x1E, 0x00], // S
    [0x3F, 0x2D, 0x0C, 0x0C, 0x0C, 0x0C, 0x1E, 0x00], // T
    [0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x3F, 0x00], // U
    [0x33, 0x33, 0x33, 0x33, 0x33, 0x1E, 0x0C, 0x00], // V
    [0x63, 0x63, 0x63, 0x6B, 0x7F, 0x77, 0x63, 0x00], // W
    [0x63, 0x63, 0x36, 0x1C, 0x1C, 0x36, 0x63, 0x00], // X
    [0x33, 0x33, 0x33, 0x1E, 0x0C, 0x0C, 0x1E, 0x00], // Y
    [0x7F, 0x63, 0x31, 0x18, 0x4C, 0x66, 0x7F, 0x00], // Z
    [0x1E, 0x06, 0x06, 0x06, 0x06, 0x06, 0x1E, 0x00], // [
    [0x03, 0x06, 0x0C, 0x18, 0x30, 0x60, 0x40, 0x00], // (backslash)
    [0x1E, 0x18, 0x18, 0x18, 0x18, 0x18, 0x1E, 0x00], // ]
    [0x08, 0x1C, 0x36, 0x63, 0x00, 0x00, 0x00, 0x00], // ^
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF], // _
    [0x0C, 0x0C, 0x18, 0x00, 0x00, 0x00, 0x00, 0x00], // `
    [0x00, 0x00, 0x1E, 0x30, 0x3E, 0x33, 0x6E, 0x00], // a
    [0x07, 0x06, 0x06, 0x3E, 0x66, 0x66, 0x3B, 0x00], // b
    [0x00, 0x00, 0x1E, 0x33, 0x03, 0x33, 0x1E, 0x00], // c
    [0x38, 0x30, 0x30, 0x3E, 0x33, 0x33, 0x6E, 0x00], // d
    [0x00, 0x00, 0x1E, 0x33, 0x3F, 0x03, 0x1E, 0x00], // e
    [0x1C, 0x36, 0x06, 0x0F, 0x06, 0x06, 0x0F, 0x00], // f
    [0x00, 0x00, 0x6E, 0x33, 0x33, 0x3E, 0x30, 0x1F], // g
    [0x07, 0x06, 0x36, 0x6E, 0x66, 0x66, 0x67, 0x00], // h
    [0x0C, 0x00, 0x0E, 0x0C, 0x0C, 0x0C, 0x1E, 0x00], // i
    [0x30, 0x00, 0x30, 0x30, 0x30, 0x33, 0x33, 0x1E], // j
    [0x07, 0x06, 0x66, 0x36, 0x1E, 0x36, 0x67, 0x00], // k
    [0x0E, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x1E, 0x00], // l
    [0x00, 0x00, 0x33, 0x7F, 0x7F, 0x6B, 0x63, 0x00], // m
    [0x00, 0x00, 0x1F, 0x33, 0x33, 0x33, 0x33, 0x00], // n
    [0x00, 0x00, 0x1E, 0x33, 0x33, 0x33, 0x1E, 0x00], // o
    [0x00, 0x00, 0x3B, 0x66, 0x66, 0x3E, 0x06, 0x0F], // p
    [0x00, 0x00, 0x6E, 0x33, 0x33, 0x3E, 0x30, 0x78], // q
    [0x00, 0x00, 0x3B, 0x6E, 0x66, 0x06, 0x0F, 0x00], // r
    [0x00, 0x00, 0x3E, 0x03, 0x1E, 0x30, 0x1F, 0x00], // s
    [0x08, 0x0C, 0x3E, 0x0C, 0x0C, 0x2C, 0x18, 0x00], // t
    [0x00, 0x00, 0x33, 0x33, 0x33, 0x33, 0x6E, 0x00], // u
    [0x00, 0x00, 0x33, 0x33, 0x33, 0x1E, 0x0C, 0x00], // v
    [0x00, 0x00, 0x63, 0x6B, 0x7F, 0x7F, 0x36, 0x00], // w
    [0x00, 0x00, 0x63, 0x36, 0x1C, 0x36, 0x63, 0x00], // x
    [0x00, 0x00, 0x33, 0x33, 0x33, 0x3E, 0x30, 0x1F], // y
    [0x00, 0x00, 0x3F, 0x19, 0x0C, 0x26, 0x3F, 0x00], // z
    [0x38, 0x0C, 0x0C, 0x07, 0x0C, 0x0C, 0x38, 0x00], // {
    [0x18, 0x18, 0x18, 0x00, 0x18, 0x18, 0x18, 0x00], // |
    [0x07, 0x0C, 0x0C, 0x38, 0x0C, 0x0C, 0x07, 0x00], // }
    [0x6E, 0x3B, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], // ~
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::switcher;
    use std::time::Instant;

    /// A snapshot builder; toplevels get consecutive MRU order by their
    /// `last_activated`.
    fn toplevel(
        handle: u64,
        title: &str,
        app_id: &str,
        workspaces: &[u64],
        state: switcher::State,
        last_activated: u64,
    ) -> switcher::Toplevel {
        switcher::Toplevel {
            handle: Handle(handle),
            title: title.to_string(),
            app_id: app_id.to_string(),
            state,
            workspaces: workspaces
                .iter()
                .copied()
                .map(switcher::WorkspaceId)
                .collect(),
            last_activated,
        }
    }

    fn snapshot(toplevels: Vec<switcher::Toplevel>, active: Option<u64>) -> Snapshot {
        Snapshot {
            toplevels,
            workspaces: Vec::new(),
            active: active.map(switcher::WorkspaceId),
        }
    }

    /// An icon cache that resolves nothing: rows fall back to the window
    /// glyph. Icon-carrying rows are built directly (see the render
    /// tests) and by the integration in `crate::icons`.
    fn no_icons() -> crate::icons::Cache {
        crate::icons::Cache::with_entries(Vec::new(), Box::new(|_name, _size| None))
    }

    fn build(snap: &Snapshot, selected: Handle, scope: Scope) -> Option<OverlayModel> {
        overlay_model(
            snap,
            selected,
            scope,
            &no_icons(),
            2,
            &Palette::dark(),
            StyleProfile::default(),
            Windowing {
                rows: DEFAULT_ROWS,
                width_floor: None,
                prev: None,
            },
            TextBackend::Bitmap,
        )
    }

    /// [`build`] with the session's width floor carried (`width` /
    /// `min-width` resolved; `None` in `build` is the content card).
    fn build_floored(
        snap: &Snapshot,
        selected: Handle,
        scope: Scope,
        width_floor: Option<u32>,
    ) -> Option<OverlayModel> {
        overlay_model(
            snap,
            selected,
            scope,
            &no_icons(),
            2,
            &Palette::dark(),
            StyleProfile::default(),
            Windowing {
                rows: DEFAULT_ROWS,
                width_floor,
                prev: None,
            },
            TextBackend::Bitmap,
        )
    }

    /// Renders `model` at `scale` into a fresh canvas; returns it with
    /// its stride. The dark palette is the behavior TH1 pinned; light
    /// lands in TH2.
    fn rendered(model: &OverlayModel, scale: u32) -> (Vec<u8>, u32) {
        let (w, h) = (model.width * scale, model.height * scale);
        let stride = w * 4;
        let mut canvas = vec![0u8; (stride * h) as usize];
        render(
            model,
            &mut canvas,
            w,
            h,
            stride,
            scale,
            &Palette::dark(),
            TextBackend::Bitmap,
        );
        (canvas, stride)
    }

    fn pixel(canvas: &[u8], stride: u32, x: u32, y: u32) -> u32 {
        let index = (y * stride + x * 4) as usize;
        u32::from_le_bytes(
            canvas[index..index + 4]
                .try_into()
                .expect("pixel-sized chunk"),
        )
    }

    /// The hint band's device pixels for `row`'s band at the rendered()
    /// scale — where `render` paints a row's digit.
    fn hint_band_points(row: usize, width: u32) -> impl Iterator<Item = (u32, u32)> {
        let top = (PAD_Y + row as u32 * (ROW_H + ROW_GAP)) * 2;
        let x0 = (width - PAD_X - HINT_COLS * TEXT_W) * 2;
        let x1 = (width - PAD_X) * 2;
        (x0..x1).flat_map(move |x| (top..top + ROW_H * 2).map(move |y| (x, y)))
    }

    // --- the geometry ladders ----------------------------------------------

    #[test]
    fn the_style_ladder_rounds_at_12_8_2_and_the_band_caps_at_8() {
        // Judged 2026-11-07, not transcribed: Square keeps a hair of
        // rounding on a hairline-border card where 0 would read as a
        // bug; SlightlyRound takes the band's own 8 so card and band
        // round together.
        assert_eq!(style_radius(Style::Round), 12);
        assert_eq!(style_radius(Style::SlightlyRound), 8);
        assert_eq!(style_radius(Style::Square), 2);
        // The band rounds with the card, never past its own pin.
        assert_eq!(band_radius(12), 8);
        assert_eq!(band_radius(8), 8);
        assert_eq!(band_radius(2), 2);
    }

    #[test]
    fn the_rhythm_table_is_the_ratified_ladder() {
        // COSMIC's own spacing ratios (compact ≈ ×2/3, spacious ≈ ×4/3)
        // on Airlock's 8-grid anchors; Compact's 4-px steps are the one
        // deliberate half-grid exception (OQ 2), row_pad_y/row_gap only.
        assert_eq!(
            rhythm(Density::Compact),
            Rhythm {
                row_pad_y: 4,
                row_gap: 4,
                pad_y: 8,
                row_h: 40
            }
        );
        assert_eq!(
            rhythm(Density::Standard),
            Rhythm {
                row_pad_y: 8,
                row_gap: 8,
                pad_y: 16,
                row_h: 48
            }
        );
        assert_eq!(
            rhythm(Density::Spacious),
            Rhythm {
                row_pad_y: 12,
                row_gap: 12,
                pad_y: 24,
                row_h: 56
            }
        );
    }

    #[test]
    fn row_h_stays_icon_driven_at_every_density() {
        for density in [Density::Compact, Density::Standard, Density::Spacious] {
            assert_eq!(
                rhythm(density).row_h,
                ICON + 2 * rhythm(density).row_pad_y,
                "{density:?}: the icon cell plus its breathing, nothing else"
            );
        }
    }

    #[test]
    fn text_profiles_derive_their_band_and_caret_from_the_cell() {
        assert_eq!(TextProfile::default(), TextProfile::new(12));
        assert_eq!(TextProfile::new(12).cell(), TEXT_W);
        assert_eq!(TextProfile::new(12).title_cols(), MAX_COLS);
        assert_eq!(TextProfile::new(12).title_band(), TITLE_BAND_PX);
        // Every accepted size stays inside the title band's physical
        // cap and reserves a caret that fits its ink plus the clearing.
        for size in FONT_SIZE_MIN..=FONT_SIZE_MAX {
            let profile = TextProfile::new(size);
            assert_eq!(profile.cell(), size);
            assert!(profile.title_cols() >= 1, "{size}: at least one column");
            assert!(
                profile.title_band() <= TITLE_BAND_PX,
                "{size}: the band never grows past the default cap"
            );
            let caret = CHEVRON_GAP + GLYPH_W * CHEVRON_SCALE;
            let reserved = profile.chevron_cols() * size;
            assert!(profile.chevron_cols() >= 1, "{size}: a caret is reserved");
            assert!(reserved >= caret, "{size}: the caret's ink fits");
            assert!(reserved < caret + size, "{size}: the reservation is tight");
        }
    }

    #[test]
    fn every_font_size_fits_inside_the_row_with_its_pips_clear() {
        // The row grows with the line while the icon, gaps, and padding
        // stay fixed. At every accepted size the cell fits with the
        // density's own breathing, the row still holds the 32-px icon,
        // and a parent's pips keep their clearing below the text line.
        for density in [Density::Compact, Density::Standard, Density::Spacious] {
            for size in FONT_SIZE_MIN..=FONT_SIZE_MAX {
                let resolved = rhythm(density).with_text(size);
                assert_eq!(
                    resolved.row_pad_y,
                    rhythm(density).row_pad_y,
                    "{density:?}: padding stays fixed"
                );
                assert!(
                    ICON + 2 * resolved.row_pad_y <= resolved.row_h,
                    "{density:?} × {size} px: the icon still fits"
                );
                assert!(
                    size + 2 * resolved.row_pad_y <= resolved.row_h,
                    "{density:?} × {size} px: the line fits the row"
                );
                let text_bottom = (resolved.row_h + size) / 2;
                let pip_top = resolved.row_h - resolved.row_pad_y - PIP_SIDE;
                assert!(
                    text_bottom + PIP_TEXT_CLEAR <= pip_top,
                    "{density:?} × {size} px: the pips clear the text line"
                );
            }
        }
    }

    #[test]
    fn a_larger_font_size_clamps_the_rows_to_fit_the_panel() {
        // A taller line means fewer windows fit the 720-px panel. The
        // clamp follows the resolved rhythm, never drops below the
        // setting's floor, and every clamped card fits.
        for density in [Density::Compact, Density::Standard, Density::Spacious] {
            for size in FONT_SIZE_MIN..=FONT_SIZE_MAX {
                let rhythm = rhythm(density).with_text(size);
                let effective = effective_rows_rhythm(rhythm, ROWS_MAX);
                assert!(
                    effective >= ROWS_MIN,
                    "{density:?} × {size} px: at least the floor fits"
                );
                assert!(
                    card_height_rhythm(rhythm, (effective + ROWS_SLACK) as u32) <= CARD_FIT_PX,
                    "{density:?} × {size} px: the clamped card fits"
                );
            }
        }
        // The largest text at Spacious is the binding case: even the
        // default budget clamps there, while Standard still fits it.
        let standard = rhythm(Density::Standard).with_text(FONT_SIZE_MAX);
        assert_eq!(effective_rows_rhythm(standard, DEFAULT_ROWS), DEFAULT_ROWS);
        let spacious = rhythm(Density::Spacious).with_text(FONT_SIZE_MAX);
        assert!(effective_rows_rhythm(spacious, DEFAULT_ROWS) < DEFAULT_ROWS);
        assert!(effective_rows_rhythm(spacious, ROWS_MAX) < ROWS_MAX);
    }

    #[test]
    fn the_default_text_keeps_every_densitys_pinned_row() {
        for density in [Density::Compact, Density::Standard, Density::Spacious] {
            assert_eq!(
                rhythm(density).with_text(DEFAULT_FONT_SIZE),
                rhythm(density),
                "{density:?}: 12 px is the pinned row"
            );
        }
    }

    #[test]
    fn a_larger_font_size_fits_fewer_columns_in_the_same_band() {
        let title = "W".repeat(200);
        let mut text = TextBackend::Bitmap;
        let small = compose_title(
            &title,
            "",
            RowKind::Leaf,
            &mut text,
            2,
            TextProfile::new(FONT_SIZE_MIN),
        );
        let large = compose_title(
            &title,
            "",
            RowKind::Leaf,
            &mut text,
            2,
            TextProfile::new(FONT_SIZE_MAX),
        );
        assert_eq!(
            small.0.chars().count(),
            TextProfile::new(FONT_SIZE_MIN).title_cols() as usize
        );
        assert_eq!(
            large.0.chars().count(),
            TextProfile::new(FONT_SIZE_MAX).title_cols() as usize
        );
        assert!(small.1 <= TITLE_BAND_PX, "the small band stays capped");
        assert!(large.1 <= TITLE_BAND_PX, "the large band stays capped");
    }

    #[test]
    fn the_band_geometry_follows_the_text_size() {
        assert_eq!(band_card_height_at(12), band_card_height());
        assert_eq!(tile_cell_h_at(12), TILE_CELL_H);
        for size in [FONT_SIZE_MIN, 12, 16, FONT_SIZE_MAX] {
            let cell = TextProfile::new(size).cell();
            assert_eq!(
                band_icon_top_at(cell),
                band_strip_top_at(cell) + cell + TILE_PARENT_GAP
            );
            assert_eq!(
                band_card_height_at(cell),
                2 * TILE_PAD_Y + cell + CAPTION_GAP + tile_cell_h_at(cell)
            );
        }
        assert!(
            band_card_height_at(FONT_SIZE_MAX) > band_card_height(),
            "a larger caption line grows the band"
        );
    }

    #[test]
    fn a_larger_text_profile_addresses_the_rows_it_paints() {
        let profile = StyleProfile::default().with_text(TextProfile::new(FONT_SIZE_MAX));
        let mut text = TextBackend::Bitmap;
        let (title, band) = compose_title("Airlock", "", RowKind::Leaf, &mut text, 2, profile.text);
        let row = RowModel {
            title,
            ..RowModel::default()
        };
        let model = finish_model(
            vec![row],
            0,
            Windowing {
                rows: DEFAULT_ROWS,
                width_floor: None,
                prev: None,
            },
            profile,
            vec![band],
        )
        .expect("a model");
        assert_eq!(model.profile.text, profile.text);
        let rhythm = model.profile.rhythm;
        let queried = with_query(model, "ab");
        let strip = strip_height_at(&queried.query, rhythm, profile.text.cell());
        // The strip's own zone holds no row; the first row starts after
        // it, exactly as the render paints it.
        assert_eq!(queried.row_at(4.0, (rhythm.pad_y + 1) as f64, 1), None);
        assert_eq!(
            queried.row_at(4.0, (rhythm.pad_y + strip + 1) as f64, 1),
            Some(0)
        );
    }

    #[test]
    fn render_paints_a_larger_text_card() {
        let profile = StyleProfile::default().with_text(TextProfile::new(FONT_SIZE_MAX));
        let mut text = TextBackend::Bitmap;
        let (title, band) = compose_title("Airlock", "", RowKind::Leaf, &mut text, 2, profile.text);
        let row = RowModel {
            title,
            handle: Some(Handle(1)),
            selected: true,
            ..RowModel::default()
        };
        let model = finish_model(
            vec![row],
            0,
            Windowing {
                rows: DEFAULT_ROWS,
                width_floor: None,
                prev: None,
            },
            profile,
            vec![band],
        )
        .expect("a model");
        let palette = Palette::dark();
        let stride = model.width * 4;
        let mut canvas = vec![0u8; (model.height * stride) as usize];
        render(
            &model,
            &mut canvas,
            model.width,
            model.height,
            stride,
            2,
            &palette,
            TextBackend::Bitmap,
        );
        let ink = palette.text.to_le_bytes();
        assert!(
            canvas.as_chunks::<4>().0.contains(&ink),
            "the larger cell paints full-coverage glyph ink"
        );
    }

    #[test]
    fn the_default_profile_is_exactly_todays_constants() {
        // The Round + Standard row — COSMIC's factory default and the
        // miss default — equals the constants block verbatim, so the
        // default desktop renders byte-identically.
        assert_eq!(
            StyleProfile::default(),
            StyleProfile {
                radius: RADIUS,
                band_radius: BAND_RADIUS,
                rhythm: Rhythm {
                    row_pad_y: ROW_PAD_Y,
                    row_gap: ROW_GAP,
                    pad_y: PAD_Y,
                    row_h: ROW_H,
                },
                text: TextProfile::default(),
            }
        );
    }

    /// The same pin at compile time: the default profile's every value
    /// is the constants block, by construction.
    const _: () = {
        let profile = StyleProfile::resolve(Style::Round, Density::Standard);
        assert!(profile.radius == RADIUS);
        assert!(profile.band_radius == BAND_RADIUS);
        assert!(profile.rhythm.row_pad_y == ROW_PAD_Y);
        assert!(profile.rhythm.row_gap == ROW_GAP);
        assert!(profile.rhythm.pad_y == PAD_Y);
        assert!(profile.rhythm.row_h == ROW_H);
    };

    #[test]
    fn every_profile_keeps_the_band_clear_of_the_card_corner() {
        // The true invariant behind the pinned era's `PAD_Y ≥ RADIUS`:
        // no part of the selection band falls in the card corner's
        // exclusion zone — inside the corner square yet outside the
        // corner circle. That assert was sufficient while pad_y was a
        // pinned 16; as a derived value Compact's pad_y is 8 and the
        // band's SELECT_INSET buys the clearance instead (the compile
        // time pin is `pad_y + SELECT_INSET ≥ radius`; this test samples
        // the geometry itself). Sampled over the band's own rounded
        // corner arc, the closest the band ever comes to the card edge.
        for style in [Style::Round, Style::SlightlyRound, Style::Square] {
            for density in [Density::Compact, Density::Standard, Density::Spacious] {
                let profile = StyleProfile::resolve(style, density);
                let (r, b) = (profile.radius as f32, profile.band_radius as f32);
                let (s, p) = (SELECT_INSET as f32, profile.rhythm.pad_y as f32);
                let mut excluded = 0;
                for step in 0..=90 {
                    let theta = std::f32::consts::FRAC_PI_2 * (step as f32) / 90.0;
                    // The band's top-left corner arc: center (s + b, p + b),
                    // from the left edge's top end to the top edge's left end.
                    let (x, y) = (s + b - b * theta.cos(), p + b - b * theta.sin());
                    let in_corner_square = x < r && y < r;
                    let outside_corner_circle = (x - r).powi(2) + (y - r).powi(2) > r * r;
                    if in_corner_square && outside_corner_circle {
                        excluded += 1;
                    }
                }
                assert_eq!(
                    excluded, 0,
                    "{style:?} + {density:?}: the band's corner stays inside the card"
                );
            }
        }
    }

    // --- the density follow: clamp, model geometry, frozen profile (GE3) ---

    #[test]
    fn every_density_and_rows_budget_fits_the_panel() {
        // The fit is guaranteed by construction (SC 4): the effective
        // budget is the largest rows whose worst-case card —
        // rows + ROWS_SLACK slots — fits the panel rule.
        for density in [Density::Compact, Density::Standard, Density::Spacious] {
            for rows in ROWS_MIN..=ROWS_MAX {
                let effective = effective_rows(density, rows);
                assert!(effective <= rows, "the clamp only lowers");
                assert!(
                    card_height_rhythm(rhythm(density), (effective + ROWS_SLACK) as u32)
                        <= CARD_FIT_PX,
                    "{density:?} × rows {rows}: the clamped card fits"
                );
            }
        }
        // Spacious is the binding density: its 68-px slot pitch clamps
        // the ceiling to 8 (9 + slack = 11 slots = 784 px > 720; 8 +
        // slack = 10 = 716 ≤ 720 — the plan's "rows ≤ 9 / 11 slots =
        // 720" line mixed Standard's gap and pad into the Spacious
        // card). Standard and Compact never clamp.
        assert_eq!(effective_rows(Density::Spacious, ROWS_MAX), 8);
        assert_eq!(effective_rows(Density::Spacious, 9), 8);
        assert_eq!(effective_rows(Density::Spacious, 8), 8);
        assert_eq!(effective_rows(Density::Standard, ROWS_MAX), ROWS_MAX);
        assert_eq!(effective_rows(Density::Compact, ROWS_MAX), ROWS_MAX);
        assert_eq!(
            effective_rows(Density::Spacious, 5),
            5,
            "within the fit, the budget is the setting's own"
        );
    }

    /// The golden-render scenario: fifteen varied windows at the default
    /// budget — long enough to window (dots, hints, bands, border), the
    /// digest pin's fixed inputs.
    fn digest_snapshot() -> Snapshot {
        fn entry(handle: u64, n: u64) -> switcher::Toplevel {
            switcher::Toplevel {
                handle: Handle(handle),
                title: format!("Window {n}"),
                app_id: format!("app.{n}"),
                state: switcher::State::default(),
                workspaces: [switcher::WorkspaceId(1)].into(),
                last_activated: 100 - n,
            }
        }
        Snapshot {
            toplevels: (1..=15).map(|n| entry(n, n)).collect(),
            workspaces: Default::default(),
            active: Some(switcher::WorkspaceId(1)),
        }
    }

    #[test]
    fn the_default_profile_renders_the_pinned_canvas_bytes() {
        // SC 1's pin, with teeth: the identical scenario rendered
        // through the profile path at Round + Standard digests to a
        // fixed canvas, so the factory-default desktop's bytes cannot
        // drift unnoticed. The digest was re-pinned 2026-11-09 with the
        // hint/chevron pass (bare digits, the inline caret); it is
        // re-captured whenever a judged design change moves the card,
        // never to make an accidental change pass.
        let model = overlay_model(
            &digest_snapshot(),
            Handle(1),
            Scope::CurrentWorkspace,
            &no_icons(),
            2,
            &Palette::dark(),
            StyleProfile::default(),
            Windowing {
                rows: DEFAULT_ROWS,
                width_floor: None,
                prev: None,
            },
            TextBackend::Bitmap,
        )
        .expect("eligible windows exist");
        let (canvas, _) = rendered(&model, 2);
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for &b in &canvas {
            hash ^= u64::from(b);
            hash = hash.wrapping_mul(0x1000_0000_01b3);
        }
        assert_eq!(
            hash, 12_296_646_189_867_322_197,
            "the default-profile render must be the pinned bytes"
        );
    }

    #[test]
    fn model_height_follows_the_density_and_width_never_moves() {
        // The same five windows at every density: the card breathes
        // vertically per the rhythm table and never horizontally —
        // density has no width analogue (the spec's pinned-X
        // invariant).
        let snap = long_snapshot(5);
        let mut width = None;
        for density in [Density::Compact, Density::Standard, Density::Spacious] {
            let profile = StyleProfile::resolve(Style::Round, density);
            let model = overlay_model(
                &snap,
                Handle(1),
                Scope::CurrentWorkspace,
                &no_icons(),
                2,
                &Palette::dark(),
                profile,
                Windowing {
                    rows: DEFAULT_ROWS,
                    width_floor: None,
                    prev: None,
                },
                TextBackend::Bitmap,
            )
            .expect("eligible windows exist");
            assert_eq!(model.profile, profile, "the model carries what it renders");
            assert_eq!(model.height, card_height_rhythm(rhythm(density), 5));
            match width {
                None => width = Some(model.width),
                Some(w) => assert_eq!(model.width, w, "{density:?}: width never moves"),
            }
        }
    }

    #[test]
    fn row_at_reads_the_profile_rhythm() {
        // Pointer rows address the geometry the model actually
        // rendered: at Spacious the second row's band starts one
        // spacious pitch below the first, and the gap seam between
        // them selects nothing.
        let snap = long_snapshot(5);
        let profile = StyleProfile::resolve(Style::Round, Density::Spacious);
        let model = overlay_model(
            &snap,
            Handle(1),
            Scope::CurrentWorkspace,
            &no_icons(),
            2,
            &Palette::dark(),
            profile,
            Windowing {
                rows: DEFAULT_ROWS,
                width_floor: None,
                prev: None,
            },
            TextBackend::Bitmap,
        )
        .expect("eligible windows exist");
        let rhythm = rhythm(Density::Spacious);
        assert_eq!(model.row_at(4.0, (rhythm.pad_y + 1) as f64, 1), Some(0));
        let second = (rhythm.pad_y + rhythm.row_h + rhythm.row_gap + 1) as f64;
        assert_eq!(model.row_at(4.0, second, 1), Some(1));
        let seam = (rhythm.pad_y + rhythm.row_h + rhythm.row_gap / 2) as f64;
        assert_eq!(
            model.row_at(4.0, seam, 1),
            None,
            "the gap seam selects nothing at any rhythm"
        );
    }

    #[test]
    fn a_square_desktop_profile_rounds_at_two() {
        // SC 2's today-value: this machine's Square desktop resolves
        // the followed 2-px radius, card and band both.
        let profile = StyleProfile::resolve(Style::Square, Density::Standard);
        assert_eq!(profile.radius, 2);
        assert_eq!(profile.band_radius, 2);
    }

    #[test]
    fn overlay_model_lists_eligible_windows_in_mru_order_only() {
        let snap = snapshot(
            vec![
                toplevel(1, "Focused", "app.a", &[1], switcher::State::ACTIVATED, 30),
                // Parked on workspace 2: never listed while workspace 1 is active.
                toplevel(
                    2,
                    "Elsewhere",
                    "app.b",
                    &[2],
                    switcher::State::default(),
                    29,
                ),
                // Minimized on this workspace: excluded.
                toplevel(
                    3,
                    "Minimized",
                    "app.c",
                    &[1],
                    switcher::State::MINIMIZED,
                    28,
                ),
                toplevel(4, "Middle", "app.d", &[1], switcher::State::default(), 20),
                toplevel(5, "Oldest", "app.e", &[1], switcher::State::default(), 10),
                // Sticky windows are visible on every workspace.
                toplevel(6, "Sticky", "app.f", &[2], switcher::State::STICKY, 5),
            ],
            Some(1),
        );
        let model =
            build(&snap, Handle(4), Scope::CurrentWorkspace).expect("eligible windows exist");
        let titles: Vec<&str> = model.rows.iter().map(|row| row.title.as_str()).collect();
        assert_eq!(titles, vec!["Focused", "Middle", "Oldest", "Sticky"]);
    }

    #[test]
    fn overlay_model_marks_only_the_selected_row() {
        let snap = snapshot(
            vec![
                toplevel(1, "First", "app.a", &[1], switcher::State::ACTIVATED, 30),
                toplevel(2, "Second", "app.b", &[1], switcher::State::default(), 20),
            ],
            Some(1),
        );
        let model =
            build(&snap, Handle(2), Scope::CurrentWorkspace).expect("eligible windows exist");
        assert!(!model.rows[0].selected);
        assert!(model.rows[1].selected);
    }

    #[test]
    fn overlay_model_is_none_without_eligible_windows() {
        let snap = snapshot(
            vec![toplevel(
                1,
                "Elsewhere",
                "app.a",
                &[2],
                switcher::State::default(),
                1,
            )],
            Some(1),
        );
        assert_eq!(build(&snap, Handle(1), Scope::CurrentWorkspace), None);
    }

    #[test]
    fn overlay_model_is_none_when_the_selection_is_not_eligible() {
        let snap = snapshot(
            vec![
                toplevel(1, "Focused", "app.a", &[1], switcher::State::ACTIVATED, 30),
                toplevel(
                    2,
                    "Elsewhere",
                    "app.b",
                    &[2],
                    switcher::State::default(),
                    20,
                ),
            ],
            Some(1),
        );
        // The selection must be one of the rows; a stale or foreign handle
        // means there is nothing coherent to highlight.
        assert_eq!(build(&snap, Handle(2), Scope::CurrentWorkspace), None);
    }

    #[test]
    fn overlay_model_is_none_without_an_active_workspace() {
        let snap = snapshot(
            vec![toplevel(
                1,
                "Window",
                "app.a",
                &[1],
                switcher::State::default(),
                1,
            )],
            None,
        );
        assert_eq!(build(&snap, Handle(1), Scope::CurrentWorkspace), None);
    }

    #[test]
    fn overlay_model_falls_back_to_app_id_then_placeholder_for_blank_titles() {
        let snap = snapshot(
            vec![
                toplevel(1, "", "app.fallback", &[1], switcher::State::ACTIVATED, 30),
                toplevel(2, "   ", "", &[1], switcher::State::default(), 20),
            ],
            Some(1),
        );
        let model =
            build(&snap, Handle(1), Scope::CurrentWorkspace).expect("eligible windows exist");
        assert_eq!(model.rows[0].title, "app.fallback");
        assert_eq!(model.rows[1].title, UNTITLED);
    }

    #[test]
    fn overlay_model_truncates_long_titles_with_an_ellipsis() {
        let long = "W".repeat(MAX_COLS as usize + 10);
        let snap = snapshot(
            vec![toplevel(
                1,
                &long,
                "app.a",
                &[1],
                switcher::State::ACTIVATED,
                1,
            )],
            Some(1),
        );
        let model =
            build(&snap, Handle(1), Scope::CurrentWorkspace).expect("eligible windows exist");
        let title = &model.rows[0].title;
        assert_eq!(title.chars().count(), MAX_COLS as usize);
        assert!(title.ends_with('…'));
        assert!(title.starts_with(&"W".repeat(MAX_COLS as usize - 1)));
    }

    #[test]
    fn overlay_model_sizes_from_rows_and_columns() {
        let short = snapshot(
            vec![toplevel(
                1,
                "A",
                "app.a",
                &[1],
                switcher::State::ACTIVATED,
                2,
            )],
            Some(1),
        );
        let model =
            build(&short, Handle(1), Scope::CurrentWorkspace).expect("eligible windows exist");
        assert_eq!(
            model.width,
            2 * PAD_X + ICON + ICON_GAP + TEXT_W + HINT_GAP + HINT_COLS * TEXT_W
        );
        assert_eq!(model.height, 2 * PAD_Y + ROW_H);

        let tall = snapshot(
            vec![
                toplevel(1, "WWWW", "app.a", &[1], switcher::State::ACTIVATED, 3),
                toplevel(2, "WWWWWWW", "app.b", &[1], switcher::State::default(), 2),
                toplevel(3, "W", "app.c", &[1], switcher::State::default(), 1),
            ],
            Some(1),
        );
        let model =
            build(&tall, Handle(1), Scope::CurrentWorkspace).expect("eligible windows exist");
        assert_eq!(
            model.width,
            2 * PAD_X + ICON + ICON_GAP + 7 * TEXT_W + HINT_GAP + HINT_COLS * TEXT_W
        );
        assert_eq!(model.height, 2 * PAD_Y + 3 * ROW_H + 2 * ROW_GAP);
    }

    #[test]
    fn a_width_floor_widens_the_vertical_card_never_narrows_it() {
        // The `width`/`min-width` contract on the vertical card: a
        // floor below the content width sizes nothing (nothing ever
        // clips), a floor above widens — rows stay left-aligned, the
        // hint band re-anchors to the new right edge.
        let snap = snapshot(
            vec![toplevel(
                1,
                "WWWW",
                "app.a",
                &[1],
                switcher::State::ACTIVATED,
                2,
            )],
            Some(1),
        );
        let content = 2 * PAD_X + ICON + ICON_GAP + 4 * TEXT_W + HINT_GAP + HINT_COLS * TEXT_W;
        // A floor under the content width (which moved down with the
        // 2026-11-09 one-column hint band) sizes nothing.
        let under = build_floored(
            &snap,
            Handle(1),
            Scope::CurrentWorkspace,
            Some(content - 40),
        )
        .expect("eligible windows exist");
        assert_eq!(under.width, content, "a floor below content sizes nothing");
        let over = build_floored(
            &snap,
            Handle(1),
            Scope::CurrentWorkspace,
            Some(content + 96),
        )
        .expect("eligible windows exist");
        assert_eq!(over.width, content + 96, "the floor above widens");
    }

    #[test]
    fn overlay_model_caps_width_at_max_columns() {
        let long = "M".repeat(MAX_COLS as usize + 5);
        let snap = snapshot(
            vec![toplevel(
                1,
                &long,
                "app.a",
                &[1],
                switcher::State::ACTIVATED,
                1,
            )],
            Some(1),
        );
        let model =
            build(&snap, Handle(1), Scope::CurrentWorkspace).expect("eligible windows exist");
        assert_eq!(
            model.width,
            2 * PAD_X + ICON + ICON_GAP + MAX_COLS * TEXT_W + HINT_GAP + HINT_COLS * TEXT_W
        );
        // The research-informed ~600 cap governs the title band; rev 2's
        // recorded decision widens the card by the structural hint band
        // on top of it rather than taxing the column budget. The 11-07
        // 12-px re-pin keeps the band under the cap (48 × 12 = 576); the
        // 2026-11-09 digit re-pin (the full `Ctrl+N` chord → one column)
        // lands the whole card at 688 logical px, down from 748.
        // (The band cap itself is a compile-time pin beside the other
        // geometry pins.)
        assert_eq!(model.width, 688, "the measured card width pin");
    }

    #[test]
    fn the_glyph_block_is_whole_device_pixels_at_the_render_scale_floor() {
        // 12-px text at the 2× floor: each font pixel is exactly three
        // device pixels, so nothing letter-spaces on any output up to
        // 200 % (the 3× case floors to 4 in a 4.5 cell — documented on
        // FONT_SCALE, not pinned here).
        assert_eq!((TEXT_W, TEXT_H), (12, 12));
        assert_eq!(TEXT_W * 2 / GLYPH_W, 3);
        assert_eq!((TEXT_W * 2) % GLYPH_W, 0);
    }

    #[test]
    fn alt_minus_closes_the_selected_window() {
        // The close key (`close-window`): Alt+minus, top row or keypad,
        // both modes alike — Alt is the gesture's own modifier.
        for mode in [KeyMode::Flat, KeyMode::Grouped] {
            for keysym in [Keysym::minus, Keysym::KP_Subtract] {
                assert_eq!(
                    key_input(keysym, false, false, true, mode),
                    GestureInput::Press {
                        key: GestureKey::Close,
                        shift: false
                    },
                    "Alt+{keysym:?} {mode:?} closes"
                );
            }
        }
        // Shift+minus is the underscore keysym — a printable, not the
        // key; a plain minus types a hyphen (the gesture always holds
        // Alt, but the pure map answers honestly without it).
        assert_eq!(
            key_input(Keysym::underscore, false, false, true, KeyMode::Grouped),
            GestureInput::Press {
                key: GestureKey::Type('_'),
                shift: false
            }
        );
        assert_eq!(
            key_input(Keysym::minus, false, false, false, KeyMode::Grouped),
            GestureInput::Press {
                key: GestureKey::Type('-'),
                shift: false
            }
        );
    }

    #[test]
    fn the_close_key_forwards_to_the_daemon() {
        let mut gesture = Gesture::default();
        gesture.feed(GestureInput::Invoke(Direction::Next));
        assert_eq!(
            gesture.feed(GestureInput::Press {
                key: GestureKey::Close,
                shift: false
            }),
            GestureOutput::CloseSelected
        );
        assert!(gesture.showing(), "the session survives the close");
        // Between sessions nothing forwards.
        let mut gesture = Gesture::default();
        assert_eq!(
            gesture.feed(GestureInput::Press {
                key: GestureKey::Close,
                shift: false
            }),
            GestureOutput::Nothing
        );
    }

    #[test]
    fn strip_height_exists_only_with_a_query() {
        let rhythm = StyleProfile::default().rhythm;
        assert_eq!(strip_height(&None, rhythm), 0, "no query, no strip");
        assert_eq!(
            strip_height(&Some("q".into()), rhythm),
            STRIP_PAD_Y + TEXT_H + rhythm.row_gap
        );
        // The const pins: the tokens the formula rides.
        assert_eq!((STRIP_PAD_Y, STRIP_CARET_GAP, STRIP_CARET_W), (8, 4, 2));
    }

    #[test]
    fn with_query_grows_the_card_and_keeps_the_tail() {
        let plain = hit_model(2);
        let height = plain.height;
        let queried = with_query(plain, "fir");
        let rhythm = queried.profile.rhythm;
        assert_eq!(
            queried.height,
            height + STRIP_PAD_Y + TEXT_H + rhythm.row_gap
        );
        assert_eq!(queried.query.as_deref(), Some("fir"));
        // The overflow shows its tail under one leading ellipsis, inside
        // the shared column budget.
        let tail = with_query(hit_model(2), &"q".repeat(MAX_COLS as usize + 5));
        let text = tail.query.expect("strip text");
        let count = text.chars().count();
        assert_eq!(count, MAX_COLS as usize, "the budget holds");
        assert!(text.starts_with('\u{2026}'), "the head spends the ellipsis");
        assert!(text.ends_with('q'), "the tail survives");
    }

    #[test]
    fn row_at_shifts_rows_below_the_strip() {
        // The strip is the top padding's twin: no row lives in it, and
        // every row's zone moves down by exactly its height.
        let plain = hit_model(2);
        let rhythm = plain.profile.rhythm;
        let strip = strip_height(&Some("ab".into()), rhythm);
        let queried = with_query(plain, "ab");
        // The old row-0 zone is now strip territory: no row.
        assert_eq!(queried.row_at(4.0, (rhythm.pad_y + 1) as f64, 1), None);
        // Row 0 moved down by the strip.
        assert_eq!(
            queried.row_at(4.0, (rhythm.pad_y + strip + 1) as f64, 1),
            Some(0)
        );
        // And the card grew to hold it — row 1 answers at its shifted
        // zone, not the plain one.
        let row_one = (rhythm.pad_y + strip + rhythm.row_h + rhythm.row_gap + 1) as f64;
        assert_eq!(queried.row_at(4.0, row_one, 1), Some(1));
        // The shift is sub-row (28 < one pitch): the plain row-1 zone
        // lands inside queried row 0 — the point of the shifted mapping.
        assert_eq!(
            queried.row_at(4.0, (rhythm.pad_y + rhythm.row_h + 1) as f64, 1),
            Some(0)
        );
    }

    // --- The workspace label's seam (workspace-label W2) ------------------

    #[test]
    fn with_workspace_off_returns_the_model_unchanged() {
        // The empty name is the identity — the default-off card is
        // byte-for-byte today's, the no-config pin.
        let plain = hit_model(2);
        let mut text = TextBackend::Bitmap;
        let sealed = with_workspace(plain.clone(), "", &mut text, 2);
        assert_eq!(sealed, plain);
    }

    #[test]
    fn with_workspace_reserves_the_vertical_top_line() {
        let plain = hit_model(2);
        let height = plain.height;
        let mut text = TextBackend::Bitmap;
        let labeled = with_workspace(plain, "code", &mut text, 2);
        let rhythm = labeled.profile.rhythm;
        assert_eq!(
            labeled.height,
            height + ws_label_cell_at(TEXT_H) + rhythm.row_gap,
            "the label's line is the strip's twin: its own cell plus the row gap"
        );
        assert_eq!(labeled.workspace, "code");
    }

    #[test]
    fn with_workspace_shifts_row_at_past_the_line() {
        // The label's line is the top padding's twin too: no row lives
        // in it, and every row's zone moves down by exactly its height.
        let plain = hit_model(2);
        let rhythm = plain.profile.rhythm;
        let line = ws_label_cell_at(TEXT_H) + rhythm.row_gap;
        let mut text = TextBackend::Bitmap;
        let labeled = with_workspace(plain, "code", &mut text, 2);
        // The old row-0 zone is now label territory: no row.
        assert_eq!(labeled.row_at(4.0, (rhythm.pad_y + 1) as f64, 1), None);
        // Row 0 moved down by the line.
        assert_eq!(
            labeled.row_at(4.0, (rhythm.pad_y + line + 1) as f64, 1),
            Some(0)
        );
    }

    #[test]
    fn with_workspace_truncates_a_long_name_at_its_cap() {
        let mut text = TextBackend::Bitmap;
        let labeled = with_workspace(hit_model(2), &"n".repeat(200), &mut text, 2);
        assert_eq!(
            labeled.workspace.chars().count() as u32,
            WS_LABEL_MAX_COLS,
            "the cap holds, the ellipsis inside it"
        );
        assert!(labeled.workspace.ends_with('\u{2026}'));
    }

    #[test]
    fn with_workspace_grows_the_horizontal_card_at_the_bottom() {
        // The label's own line (W7): the horizontal card grows by the
        // clearing and the label's cell below the tiles — and the
        // caption above is untouched, its budget whole again.
        let snap = band_snapshot(4);
        let model = build_band(&snap, Handle(1), 4, None).expect("builds");
        let height = model.height;
        let caption = model.caption.clone();
        let mut text = TextBackend::Bitmap;
        let labeled = with_workspace(model, "code", &mut text, 2);
        assert_eq!(
            labeled.height,
            height + WS_LABEL_GAP + ws_label_cell_at(TEXT_H),
            "the bottom line costs its clearing plus the smaller cell"
        );
        assert_eq!(
            labeled.caption, caption,
            "the caption keeps its whole budget"
        );
    }

    #[test]
    fn row_hints_address_exactly_one_through_nine() {
        // Entries 1..9 are addressable and hinted; entry 10 onward
        // paints nothing — no dead keys on the card (cosmic's own
        // row-10 hint is an unwired upstream gap Airlock does not
        // copy). Rev 5: one chord — Ctrl+N at level 1; the Alt+N
        // children chords left with the scoped vocabulary.
        for (index, n) in (1..=9).enumerate() {
            assert_eq!(RowHint::ctrl(index), Some(RowHint::Ctrl(n)));
        }
        assert_eq!(RowHint::ctrl(9), None, "the tenth entry is not addressable");
    }

    #[test]
    fn overlay_width_carries_the_structural_hint_band() {
        // The band is furniture every card owns: the width formula
        // reserves HINT_GAP + HINT_COLS whatever the row count — a
        // one-row card and a fifteen-row card of equal titles are
        // equally wide, whether or not any row paints a hint.
        let single = snapshot(
            vec![toplevel(
                1,
                "WWWWWWW",
                "app.a",
                &[1],
                switcher::State::ACTIVATED,
                2,
            )],
            Some(1),
        );
        let model =
            build(&single, Handle(1), Scope::CurrentWorkspace).expect("eligible windows exist");
        assert_eq!(
            model.width,
            2 * PAD_X + ICON + ICON_GAP + 7 * TEXT_W + HINT_GAP + HINT_COLS * TEXT_W
        );

        let many: Vec<switcher::Toplevel> = (0..15)
            .map(|index| {
                toplevel(
                    index + 1,
                    "WWWWWWW",
                    "app.a",
                    &[1],
                    if index == 0 {
                        switcher::State::ACTIVATED
                    } else {
                        switcher::State::default()
                    },
                    15 - index,
                )
            })
            .collect();
        let tall = build(&snapshot(many, Some(1)), Handle(1), Scope::CurrentWorkspace)
            .expect("eligible windows exist");
        assert_eq!(
            tall.width, model.width,
            "the band never varies with row count"
        );
    }

    #[test]
    fn overlay_model_pads_top_and_bottom_equally() {
        // The row gap belongs *between* rows: the last row ends exactly
        // one PAD_Y above the bottom edge, mirroring the top.
        let snap = snapshot(
            vec![
                toplevel(1, "One", "app.a", &[1], switcher::State::ACTIVATED, 3),
                toplevel(2, "Two", "app.b", &[1], switcher::State::default(), 2),
                toplevel(3, "Three", "app.c", &[1], switcher::State::default(), 1),
            ],
            Some(1),
        );
        let model =
            build(&snap, Handle(1), Scope::CurrentWorkspace).expect("eligible windows exist");
        assert_eq!(model.height, 2 * PAD_Y + 3 * ROW_H + 2 * ROW_GAP);
    }

    #[test]
    fn overlay_model_rows_carry_their_windows_handles() {
        // The on-screen model is the truth a pointer click selects: every
        // row must know which window it renders.
        let snap = snapshot(
            vec![
                toplevel(1, "First", "app.a", &[1], switcher::State::ACTIVATED, 30),
                toplevel(2, "Second", "app.b", &[1], switcher::State::default(), 20),
            ],
            Some(1),
        );
        let model =
            build(&snap, Handle(1), Scope::CurrentWorkspace).expect("eligible windows exist");
        let handles: Vec<Option<Handle>> = model.rows.iter().map(|row| row.handle).collect();
        assert_eq!(handles, vec![Some(Handle(1)), Some(Handle(2))]);
    }

    /// The slot count the rule reserves for a `len`-long list under a
    /// `rows` budget — the contract, not the implementation: whole up to
    /// `rows + ROWS_SLACK`, the budget plus one slot per truncatable edge
    /// above it.
    fn expected_slots(len: usize, rows: usize) -> usize {
        if len > rows {
            (rows + ROWS_SLACK).min(len)
        } else {
            len
        }
    }

    /// Every `rows` the setting accepts: the property suite runs over all
    /// of them — the enumerations stay small.
    fn every_rows() -> std::ops::RangeInclusive<usize> {
        ROWS_MIN..=ROWS_MAX
    }

    /// Every window `visible_window` can produce for a `len`-long list
    /// under `rows`, deduplicated: one per selection position.
    fn reachable_windows(len: usize, rows: usize) -> Vec<Viewport> {
        let mut out = Vec::new();
        for sel in 0..len {
            let vp = visible_window(len, sel, rows, None);
            if !out.contains(&vp) {
                out.push(vp);
            }
        }
        out
    }

    /// Prior windows a session could carry into a `len`-long rebuild:
    /// every window reachable at `len` plus every window reachable at a
    /// longer length — the mid-gesture shrink case.
    fn prior_windows(len: usize, rows: usize) -> Vec<Viewport> {
        let mut priors = reachable_windows(len, rows);
        for longer in len + 1..=3 * rows + 2 {
            priors.extend(reachable_windows(longer, rows));
        }
        priors
    }

    #[test]
    fn visible_window_keeps_every_selection_on_a_content_row() {
        // The fixed point, exhaustive over every (rows, len, sel, prev)
        // the gesture can produce: the window covers exactly the
        // reserved slots, the flags honestly report what is hidden, and
        // the selection always lands in the inner content range — never
        // an affordance slot, never off-window. Any implementation
        // satisfying this is the contract; the clamping inside is not.
        for rows in every_rows() {
            for len in 0..=3 * rows + 2 {
                for sel in 0..len {
                    for prev in
                        std::iter::once(None).chain(prior_windows(len, rows).into_iter().map(Some))
                    {
                        let vp = visible_window(len, sel, rows, prev);
                        let visible = expected_slots(len, rows);
                        assert_eq!(
                            vp.top as usize + vp.content + vp.bottom as usize,
                            visible,
                            "rows {rows} len {len} sel {sel} prev {prev:?}: slots must equal the \
                             reserved count"
                        );
                        assert_eq!(
                            vp.top,
                            vp.start > 0,
                            "rows {rows} len {len} sel {sel}: top flag"
                        );
                        assert_eq!(
                            vp.bottom,
                            vp.start + visible < len,
                            "rows {rows} len {len} sel {sel}: bottom flag"
                        );
                        assert!(
                            vp.start + visible <= len,
                            "rows {rows} len {len} sel {sel}: window runs past the list"
                        );
                        let first = vp.start + vp.top as usize;
                        assert!(
                            first <= sel && sel < first + vp.content,
                            "rows {rows} len {len} sel {sel} prev {prev:?}: selection {sel} not on \
                             a content row of {vp:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_long_list_keeps_at_least_rows_windows_in_view() {
        // The 11-07 budget: the `…` rows ride on top of `rows`, never
        // inside it — exactly `rows` content between two dots, one more
        // when only one edge is truncated.
        for rows in every_rows() {
            for len in rows + ROWS_SLACK + 1..=3 * rows + 2 {
                for sel in 0..len {
                    let vp = visible_window(len, sel, rows, None);
                    let edges = vp.top as usize + vp.bottom as usize;
                    assert_eq!(
                        vp.content + edges,
                        rows + ROWS_SLACK,
                        "rows {rows} len {len} sel {sel}: a long list reserves rows + slack slots"
                    );
                    assert_eq!(
                        vp.content,
                        rows + ROWS_SLACK - edges,
                        "rows {rows} len {len} sel {sel}: content is the budget plus the untruncated \
                         edges"
                    );
                    assert!(
                        vp.content >= rows,
                        "rows {rows} len {len} sel {sel}: fewer windows in view than the budget"
                    );
                }
            }
        }
    }

    #[test]
    fn a_list_within_the_budget_plus_slack_shows_whole() {
        // Up to `rows + ROWS_SLACK` windows fit the reserved slots with
        // nothing hidden, so no affordance is drawn and every row is
        // content — the no-truncation boundary moved from `rows` to
        // `rows + 2` with the 11-07 rule.
        for rows in every_rows() {
            for len in 1..=rows + ROWS_SLACK {
                for sel in 0..len {
                    assert_eq!(
                        visible_window(len, sel, rows, None),
                        Viewport {
                            start: 0,
                            content: len,
                            top: false,
                            bottom: false
                        },
                        "rows {rows} len {len} sel {sel}: a list within budget + slack shows whole"
                    );
                }
            }
        }
    }

    #[test]
    fn visible_window_is_sticky_while_the_selection_stays_visible() {
        // A selection already on a content row of the carried window
        // must not move it: interior steps keep the walking fill
        // walking, nothing else.
        for rows in every_rows() {
            for len in rows..=3 * rows + 2 {
                for prev in reachable_windows(len, rows) {
                    let first = prev.start + prev.top as usize;
                    for sel in first..first + prev.content {
                        assert_eq!(
                            visible_window(len, sel, rows, Some(prev)),
                            prev,
                            "rows {rows} len {len} sel {sel}: the window moved under a visible \
                             selection"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn visible_window_moves_one_row_at_most_and_only_when_it_must() {
        // One step of the selection moves the window by at most one row,
        // by zero while the new selection stays content-visible, and by
        // exactly one when it stepped past an edge.
        for rows in every_rows() {
            for len in rows + 1..=3 * rows + 2 {
                for prev_sel in 0..len {
                    let prev = visible_window(len, prev_sel, rows, None);
                    for next_sel in [prev_sel + 1, prev_sel.saturating_sub(1)] {
                        if next_sel >= len {
                            continue;
                        }
                        let next = visible_window(len, next_sel, rows, Some(prev));
                        let delta = next.start.abs_diff(prev.start);
                        let still_visible = (prev.start + prev.top as usize
                            ..prev.start + prev.top as usize + prev.content)
                            .contains(&next_sel);
                        let expected = if still_visible { 0 } else { 1 };
                        assert_eq!(
                            delta, expected,
                            "rows {rows} len {len} {prev_sel}→{next_sel}: window moved {delta} \
                             rows, expected {expected}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_fresh_session_opens_top_aligned() {
        // The selection at open is the MRU front (row 0), so a fresh
        // window shows the head of the list — and the rule stays total:
        // any fresh selection the head window can still show as content
        // keeps it.
        for rows in every_rows() {
            assert_eq!(visible_window(rows + 5, 0, rows, None).start, 0);
            let head = visible_window(rows + 5, rows - 2, rows, None);
            assert_eq!(head.start, 0, "rows {rows}");
            assert!(
                head.bottom,
                "rows {rows}: a longer list truncates at the bottom only"
            );
        }
    }

    #[test]
    fn the_truncation_edges_appear_past_the_slack() {
        // Past `rows + ROWS_SLACK` the windows reach both truncations:
        // the head hides rows below, the pulled-down window rows above,
        // and an interior window — four past the budget — both at once.
        for rows in every_rows() {
            let past = rows + ROWS_SLACK + 1;
            let head = visible_window(past, 0, rows, None);
            assert!(
                !head.top && head.bottom,
                "rows {rows}: head hides below only"
            );
            assert_eq!(
                head.content,
                rows + 1,
                "rows {rows}: one edge, one extra window"
            );
            let pulled = visible_window(past, past - 1, rows, None);
            assert_eq!(pulled.start, 1, "rows {rows}");
            assert!(
                pulled.top && !pulled.bottom,
                "rows {rows}: pulled hides above only"
            );
            assert_eq!(pulled.content, rows + 1, "rows {rows}");
            let interior = visible_window(rows + 4, rows + 1, rows, None);
            assert!(interior.top && interior.bottom, "rows {rows}: both dots");
            assert_eq!(
                interior.content, rows,
                "rows {rows}: exactly the budget between dots"
            );
        }
    }

    #[test]
    fn a_full_sweep_shifts_the_window_in_single_row_steps() {
        // Carrying the window through a whole Tab sweep (and back) moves
        // it from the top window to the bottom window one row at a time,
        // never jumping — the shift reads as one clean frame per step.
        for rows in every_rows() {
            for len in [rows + ROWS_SLACK + 1, 2 * rows + 2, 3 * rows + 2] {
                let visible = expected_slots(len, rows);
                let mut carried = visible_window(len, 0, rows, None);
                assert_eq!(
                    carried.start, 0,
                    "rows {rows} len {len}: the sweep opens top-aligned"
                );
                let mut starts = vec![carried.start];
                for sel in 1..len {
                    carried = visible_window(len, sel, rows, Some(carried));
                    starts.push(carried.start);
                }
                assert_eq!(
                    carried.start,
                    len - visible,
                    "rows {rows} len {len}: the sweep must reach the bottom window"
                );
                for pair in starts.windows(2) {
                    assert!(
                        pair[1] - pair[0] <= 1,
                        "rows {rows} len {len}: the window jumped: {starts:?}"
                    );
                }
                // Shift+Tab back up: the same vocabulary in reverse.
                for sel in (0..len - 1).rev() {
                    let now = visible_window(len, sel, rows, Some(carried));
                    assert!(
                        carried.start - now.start <= 1,
                        "rows {rows} len {len} sel {sel}: the window jumped back up"
                    );
                    carried = now;
                }
                assert_eq!(
                    carried.start, 0,
                    "rows {rows} len {len}: the return sweep re-tops"
                );
            }
        }
    }

    // --- card-layout: the horizontal windowing rule (CL2) ---------------

    /// Every tile budget the `columns` setting can produce, plus one
    /// past the ceiling: the property suite runs over all of them —
    /// the enumerations stay small.
    fn every_tiles() -> std::ops::RangeInclusive<usize> {
        COLUMNS_MIN..=COLUMNS_MAX + 1
    }

    /// The tile count the band reserves for a `len`-long list under a
    /// `tiles` budget — the contract, not the implementation: the whole
    /// list up to the budget, the budget above it. The trailing `…`
    /// lives inside these tiles, never beyond them.
    fn expected_tiles(len: usize, tiles: usize) -> usize {
        len.min(tiles)
    }

    /// Every window `band_window` can produce for a `len`-long list
    /// under `tiles`, deduplicated: one per selection position.
    fn reachable_bands(len: usize, tiles: usize) -> Vec<Viewport> {
        let mut out = Vec::new();
        for sel in 0..len {
            let vp = band_window(len, sel, tiles, None);
            if !out.contains(&vp) {
                out.push(vp);
            }
        }
        out
    }

    /// Prior windows a horizontal session could carry into a `len`-long
    /// rebuild: every band reachable at `len` plus every band reachable
    /// at a longer length — the mid-gesture shrink case.
    fn prior_bands(len: usize, tiles: usize) -> Vec<Viewport> {
        let mut priors = reachable_bands(len, tiles);
        for longer in len + 1..=3 * tiles + 2 {
            priors.extend(reachable_bands(longer, tiles));
        }
        priors
    }

    #[test]
    fn band_window_keeps_every_selection_on_a_content_tile() {
        // The fixed point, exhaustive over every (tiles, len, sel, prev)
        // the rule can meet: the band is exactly the reserved tiles
        // wide, the affordance replaces the trailing slot and only
        // there, the window never runs past the list, and the selection
        // always lands on a content tile.
        for tiles in every_tiles() {
            for len in 0..=3 * tiles + 2 {
                for sel in 0..len {
                    for prev in
                        std::iter::once(None).chain(prior_bands(len, tiles).into_iter().map(Some))
                    {
                        let vp = band_window(len, sel, tiles, prev);
                        let slots = expected_tiles(len, tiles);
                        assert_eq!(
                            vp.content + vp.bottom as usize,
                            slots,
                            "tiles {tiles} len {len} sel {sel} prev {prev:?}: slots must equal the \
                             reserved count"
                        );
                        assert!(
                            !vp.top,
                            "tiles {tiles} len {len} sel {sel}: a band never leads with an \
                             affordance"
                        );
                        assert_eq!(
                            vp.bottom,
                            vp.start + tiles < len,
                            "tiles {tiles} len {len} sel {sel}: bottom flag"
                        );
                        assert!(
                            vp.start + vp.content <= len,
                            "tiles {tiles} len {len} sel {sel}: window runs past the list"
                        );
                        assert!(
                            vp.start <= sel && sel < vp.start + vp.content,
                            "tiles {tiles} len {len} sel {sel} prev {prev:?}: selection {sel} not \
                             on a content tile of {vp:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_long_band_spends_one_tile_on_the_affordance() {
        // The budget contract, one axis over: a list longer than the
        // budget keeps at least `tiles − 1` windows in view — the
        // affordance spends the last tile mid-list, the tail window
        // hides nothing and shows all `tiles`.
        for tiles in every_tiles() {
            for len in tiles + 1..=3 * tiles + 2 {
                for sel in 0..len {
                    let vp = band_window(len, sel, tiles, None);
                    assert!(
                        vp.content >= tiles - 1,
                        "tiles {tiles} len {len} sel {sel}: fewer windows in view than the \
                         budget minus the affordance"
                    );
                }
            }
        }
    }

    #[test]
    fn a_list_within_the_tile_budget_shows_whole() {
        // Up to `tiles` windows fit the band with nothing hidden: no
        // affordance, every tile content, left-aligned.
        for tiles in every_tiles() {
            for len in 1..=tiles {
                for sel in 0..len {
                    assert_eq!(
                        band_window(len, sel, tiles, None),
                        Viewport {
                            start: 0,
                            content: len,
                            top: false,
                            bottom: false
                        },
                        "tiles {tiles} len {len} sel {sel}: a list within budget shows whole"
                    );
                }
            }
        }
    }

    #[test]
    fn band_window_is_sticky_while_the_selection_stays_visible() {
        // A selection already on a content tile of the carried window
        // must not move it: interior steps change nothing.
        for tiles in every_tiles() {
            for len in tiles..=3 * tiles + 2 {
                for prev in reachable_bands(len, tiles) {
                    for sel in prev.start..prev.start + prev.content {
                        assert_eq!(
                            band_window(len, sel, tiles, Some(prev)),
                            prev,
                            "tiles {tiles} len {len} sel {sel}: the band moved under a visible \
                             selection"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn band_window_moves_one_tile_at_most_and_only_when_it_must() {
        // One step of the selection moves the window by at most one
        // tile, by zero while the new selection stays content-visible,
        // and by exactly one when it stepped past the trailing edge.
        for tiles in every_tiles() {
            for len in tiles + 1..=3 * tiles + 2 {
                for prev_sel in 0..len {
                    let prev = band_window(len, prev_sel, tiles, None);
                    for next_sel in [prev_sel + 1, prev_sel.saturating_sub(1)] {
                        if next_sel >= len {
                            continue;
                        }
                        let next = band_window(len, next_sel, tiles, Some(prev));
                        let delta = next.start.abs_diff(prev.start);
                        let still_visible =
                            (prev.start..prev.start + prev.content).contains(&next_sel);
                        let expected = if still_visible { 0 } else { 1 };
                        assert_eq!(
                            delta, expected,
                            "tiles {tiles} len {len} {prev_sel}→{next_sel}: window moved {delta} \
                             tiles, expected {expected}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_fresh_session_opens_left_aligned() {
        // The selection at open is the MRU front (tile 0), so a fresh
        // band shows the head of the list — and the rule stays total:
        // any fresh selection the head window can still show as
        // content keeps it.
        for tiles in every_tiles() {
            assert_eq!(band_window(tiles + 5, 0, tiles, None).start, 0);
            let head = band_window(tiles + 5, tiles - 2, tiles, None);
            assert_eq!(head.start, 0, "tiles {tiles}");
            assert!(
                head.bottom,
                "tiles {tiles}: a longer list truncates at the trailing edge only"
            );
        }
    }

    #[test]
    fn the_affordance_appears_at_the_trailing_edge_only() {
        // Past the budget the windows truncate at one edge only: the
        // head window hides windows to its right and spends its last
        // tile on the affordance; the tail window hides nothing and
        // shows the whole budget; nothing ever hides to the left.
        for tiles in every_tiles() {
            let past = tiles + 1;
            let head = band_window(past, 0, tiles, None);
            assert!(
                !head.top && head.bottom,
                "tiles {tiles}: head hides at the trailing edge only"
            );
            assert_eq!(
                head.content,
                tiles - 1,
                "tiles {tiles}: the affordance spends exactly one tile"
            );
            let tail = band_window(past, past - 1, tiles, None);
            assert_eq!(tail.start, 1, "tiles {tiles}");
            assert!(
                !tail.top && !tail.bottom,
                "tiles {tiles}: the tail window hides nothing"
            );
            assert_eq!(tail.content, tiles, "tiles {tiles}: the whole budget");
        }
    }

    #[test]
    fn a_full_sweep_shifts_the_band_in_single_tile_steps() {
        // Carrying the band through a whole Tab sweep (and back) moves
        // it from the head window to the tail window one tile at a
        // time, never jumping — the shift reads as one clean frame per
        // step.
        for tiles in every_tiles() {
            for len in [tiles + 1, 2 * tiles + 2, 3 * tiles + 2] {
                let mut carried = band_window(len, 0, tiles, None);
                assert_eq!(
                    carried.start, 0,
                    "tiles {tiles} len {len}: the sweep opens left-aligned"
                );
                let mut starts = vec![carried.start];
                for sel in 1..len {
                    carried = band_window(len, sel, tiles, Some(carried));
                    starts.push(carried.start);
                }
                assert_eq!(
                    carried.start,
                    len - tiles,
                    "tiles {tiles} len {len}: the sweep must reach the tail window"
                );
                for pair in starts.windows(2) {
                    assert!(
                        pair[1] - pair[0] <= 1,
                        "tiles {tiles} len {len}: the band jumped: {starts:?}"
                    );
                }
                // Shift+Tab back: the same vocabulary in reverse.
                for sel in (0..len - 1).rev() {
                    let now = band_window(len, sel, tiles, Some(carried));
                    assert!(
                        carried.start - now.start <= 1,
                        "tiles {tiles} len {len} sel {sel}: the band jumped back"
                    );
                    carried = now;
                }
                assert_eq!(
                    carried.start, 0,
                    "tiles {tiles} len {len}: the return sweep re-lefts"
                );
            }
        }
    }

    // --- card-layout: the horizontal card (CL3) --------------------------

    /// A `count`-window snapshot with distinct apps, MRU-ordered so
    /// handle `n` sits at list index `n − 1`.
    fn band_snapshot(count: u64) -> Snapshot {
        let toplevels = (1..=count)
            .map(|handle| {
                toplevel(
                    handle,
                    &format!("W{handle:02}"),
                    &format!("app.{handle}"),
                    &[1],
                    if handle == 1 {
                        switcher::State::ACTIVATED
                    } else {
                        switcher::State::default()
                    },
                    count + 1 - handle,
                )
            })
            .collect();
        snapshot(toplevels, Some(1))
    }

    fn build_band(
        snap: &Snapshot,
        selected: Handle,
        tiles: usize,
        prev: Option<Viewport>,
    ) -> Option<OverlayModel> {
        overlay_model_horizontal(
            snap,
            selected,
            Scope::CurrentWorkspace,
            &no_icons(),
            2,
            &Palette::dark(),
            StyleProfile::default(),
            Banding {
                tiles,
                width_floor: None,
                prev,
            },
            TextBackend::Bitmap,
        )
    }

    #[test]
    fn a_horizontal_model_lays_tiles_in_mru_order() {
        // The flat band: one tile per window, MRU-first from the left,
        // the ring's selection marked, the model flagged horizontal so
        // the renderer and the pointer dispatch on the axis.
        let snap = band_snapshot(4);
        let model = build_band(&snap, Handle(2), 4, None).expect("eligible windows exist");
        assert!(model.horizontal);
        let handles: Vec<Option<Handle>> = model.rows.iter().map(|row| row.handle).collect();
        assert_eq!(
            handles,
            vec![
                Some(Handle(1)),
                Some(Handle(2)),
                Some(Handle(3)),
                Some(Handle(4))
            ]
        );
        let selected: Vec<bool> = model.rows.iter().map(|row| row.selected).collect();
        assert_eq!(selected, vec![false, true, false, false]);
        // A list within the budget shows whole: no continuation tile.
        assert_eq!(model.viewport.content, 4);
        assert!(!model.viewport.bottom);
    }

    #[test]
    fn a_truncated_band_appends_one_continuation_tile() {
        // Six windows, four tiles: four content tiles then one wordless
        // `…` tile — handle `None`, inert to every input, never hinted,
        // never banded.
        let snap = band_snapshot(6);
        let model = build_band(&snap, Handle(1), 4, None).expect("eligible windows exist");
        // The dots spend a content tile: three windows plus the dots is
        // four slots — the card stays four tiles wide.
        assert_eq!(model.rows.len(), 4, "three content tiles plus the dots");
        let dots = model.rows.last().expect("the trailing tile");
        assert_eq!(dots.handle, None, "the continuation tile carries no window");
        assert_eq!(dots.hint, None, "the continuation tile hints no chord");
        assert!(!dots.selected);
        assert_eq!(model.viewport.content, 3);
        assert!(model.viewport.bottom);
        // The content tiles keep their reading-order chords.
        let hints: Vec<Option<RowHint>> = model.rows.iter().map(|row| row.hint).collect();
        assert_eq!(
            hints,
            vec![
                Some(RowHint::Ctrl(1)),
                Some(RowHint::Ctrl(2)),
                Some(RowHint::Ctrl(3)),
                None
            ]
        );
    }

    #[test]
    fn tile_hints_stop_at_nine_in_reading_order() {
        // Reading order is left to right over the *content* tiles; the
        // tenth and beyond hint nothing, the dots tile nothing — the
        // flat card's every-row rule one axis over.
        let snap = band_snapshot(14);
        let model = build_band(&snap, Handle(1), 12, None).expect("eligible windows exist");
        let content = &model.rows[..model.viewport.content];
        for (index, tile) in content.iter().enumerate() {
            let expected = (index < 9).then_some(RowHint::Ctrl(index + 1));
            assert_eq!(tile.hint, expected, "content tile {index}");
        }
        assert_eq!(model.rows.last().expect("dots").hint, None);
    }

    #[test]
    fn the_band_sizes_wide_and_short() {
        // The wide-short aspect: width is the tile grid floored at the
        // caption (glance-tiles G1), height the caption line, the tile
        // cell, and the card padding — wider than tall at every budget
        // the setting accepts, and the `…` tile never widens it.
        let whole = build_band(&band_snapshot(3), Handle(1), 3, None).expect("builds");
        assert_eq!(
            whole.width, CAPTION_FLOOR_W,
            "a three-tile shape floored at the caption's guarantee"
        );
        assert_eq!(whole.height, band_card_height());
        let truncated = build_band(&band_snapshot(9), Handle(1), 3, None).expect("builds");
        assert_eq!(
            truncated.width, whole.width,
            "the dots tile spends a content tile, never the card's width"
        );
        for tiles in COLUMNS_MIN..=COLUMNS_MAX {
            let model = build_band(&band_snapshot(tiles as u64 + 2), Handle(1), tiles, None)
                .expect("builds");
            assert!(
                model.width > model.height,
                "tiles {tiles}: a band reads wide, not tall"
            );
        }
    }

    #[test]
    fn a_width_floor_widens_the_band_and_recomposes_the_caption() {
        // The floor only widens, on the band too: below the caption's
        // own guarantee it sizes nothing; above it the card widens with
        // the tiles left-aligned in the air, and the caption recomposes
        // against the wider budget — a long title un-truncates, never
        // overflows.
        let snap = snapshot(
            vec![toplevel(
                1,
                &"W".repeat(200),
                "app.a",
                &[1],
                switcher::State::ACTIVATED,
                2,
            )],
            Some(1),
        );
        let band = |floor: Option<u32>| {
            overlay_model_horizontal(
                &snap,
                Handle(1),
                Scope::CurrentWorkspace,
                &no_icons(),
                2,
                &Palette::dark(),
                StyleProfile::default(),
                Banding {
                    tiles: 3,
                    width_floor: floor,
                    prev: None,
                },
                TextBackend::Bitmap,
            )
            .expect("builds")
        };
        let content = band(None);
        assert_eq!(content.width, CAPTION_FLOOR_W, "the shape's own floor");
        assert_eq!(
            band(Some(200)).width,
            CAPTION_FLOOR_W,
            "a floor below content sizes nothing"
        );
        let over = band(Some(800));
        assert_eq!(over.width, 800, "the floor above widens");
        assert!(
            over.caption.chars().count() > content.caption.chars().count(),
            "the caption recomposes into the wider budget"
        );
    }

    #[test]
    fn auto_columns_fit_a_fraction_of_the_output_min_three() {
        // Auto: floor of two thirds of the invoking output's logical
        // width over the tile pitch, floored at three — pinned against
        // synthetic outputs, uncapped above (only pinned values cap at
        // twelve, and the parser enforces that). The pins re-derived at
        // the compact pitch (glance-tiles G1, 2026-11-09: 72 px against
        // CL4's 160 — the same outputs fit 2.2× the tiles).
        assert_eq!(resolved_tiles(ColumnsSetting::Auto, 1920), 17);
        assert_eq!(resolved_tiles(ColumnsSetting::Auto, 1366), 12);
        assert_eq!(resolved_tiles(ColumnsSetting::Auto, 2560), 23);
        assert_eq!(resolved_tiles(ColumnsSetting::Auto, 3840), 35);
        // Tiny and unknown-width outputs clamp to the floor.
        for width in [0, 1, 200, 300] {
            assert_eq!(
                resolved_tiles(ColumnsSetting::Auto, width),
                COLUMNS_MIN,
                "width {width}: never fewer than three tiles"
            );
        }
        // Pinned is the budget verbatim.
        assert_eq!(resolved_tiles(ColumnsSetting::Pinned(3), 1920), 3);
        assert_eq!(resolved_tiles(ColumnsSetting::Pinned(12), 640), 12);
    }

    #[test]
    fn width_settings_resolve_against_the_output() {
        // The three forms beside the tiles twin: `auto` stays
        // content-sized (`None`), a percentage is the output's share in
        // whole logical px, a pixel count is the floor verbatim — the
        // daemon owns the one clamp left, a pixel count past the
        // output.
        assert_eq!(resolved_width(WidthSetting::Auto, 1920), None);
        assert_eq!(resolved_width(WidthSetting::Percent(100), 1920), Some(1920));
        assert_eq!(resolved_width(WidthSetting::Percent(66), 1920), Some(1267));
        assert_eq!(resolved_width(WidthSetting::Percent(50), 1366), Some(683));
        assert_eq!(resolved_width(WidthSetting::Px(640), 1920), Some(640));
        assert_eq!(resolved_width(WidthSetting::Px(9999), 1920), Some(9999));
        // An implausible reported output saturates rather than panicking
        // the gesture path.
        assert_eq!(
            resolved_width(WidthSetting::Percent(100), u32::MAX),
            Some(u32::MAX / 100)
        );
    }

    #[test]
    fn cell_at_maps_each_tile_cell_to_its_index() {
        // The pointer's horizontal contract: a point in tile N's cell
        // addresses tile N — device pixels in, model logical out, at
        // both render scales. Gap seams, card padding, and off-card
        // points are `None`.
        let snap = band_snapshot(5);
        let model = build_band(&snap, Handle(1), 4, None).expect("builds");
        for scale in [1, 2] {
            let center_y = model.height as f64 * scale as f64 / 2.0;
            for index in 0..4u32 {
                let left = TILE_PAD_X + index * (TILE_W + TILE_GAP);
                let center_x = (left + TILE_W / 2) as f64 * scale as f64;
                assert_eq!(
                    model.cell_at(center_x, center_y, scale),
                    Some(index as usize),
                    "scale {scale}: tile {index}'s cell center"
                );
            }
            // The gap seam between tiles 0 and 1 addresses nothing.
            let seam_x = (TILE_PAD_X + TILE_W + TILE_GAP / 2) as f64 * scale as f64;
            assert_eq!(model.cell_at(seam_x, center_y, scale), None, "gap seam");
            // Card padding before the first tile and past the last.
            let before = (TILE_PAD_X / 2) as f64 * scale as f64;
            assert_eq!(model.cell_at(before, center_y, scale), None, "left padding");
            let past = model.width as f64 * scale as f64 - 1.0;
            assert_eq!(model.cell_at(past, center_y, scale), None, "right padding");
            // Off the card entirely, both axes.
            assert_eq!(model.cell_at(-1.0, center_y, scale), None, "off left");
            assert_eq!(
                model.cell_at(1.0, model.height as f64 * scale as f64 + 1.0, scale),
                None,
                "off bottom"
            );
        }
    }

    #[test]
    fn a_cell_click_resolves_the_window_the_tile_showed() {
        // The event-time twin (II1's shape): the same screen point —
        // tile 1's cell center — resolves the window *that model* was
        // showing, so a click captured against one card never activates
        // another card's window at the shifted list underneath. Run red
        // against a row-addressed resolution: tile 1 of B is W9, not
        // the W2 the user clicked on A.
        let a = build_band(&band_snapshot(4), Handle(1), 4, None).expect("model A");
        let point_x = (TILE_PAD_X + (TILE_W + TILE_GAP) + TILE_W / 2) as f64; // tile 1's center, 1×
        let point_y = a.height as f64 / 2.0;
        let cell = a.cell_at(point_x, point_y, 1).expect("a tile");
        assert_eq!(a.rows[cell].handle, Some(Handle(2)), "A's tile 1 is W2");

        // B: W9 activated, the same windows shifted one right.
        let mut shifted = band_snapshot(4).toplevels;
        shifted.insert(
            0,
            toplevel(9, "W9", "app.9", &[1], switcher::State::ACTIVATED, 50),
        );
        let b = build_band(&snapshot(shifted, Some(1)), Handle(9), 4, None).expect("model B");
        // B's own tile 0 is its new front window W9; the same point —
        // tile 1 — now addresses W1. A click captured on A resolves
        // W2 whatever B shows by drain time; a click on B resolves W1.
        assert_eq!(b.rows[0].handle, Some(Handle(9)), "B leads with W9");
        let cell_b = b.cell_at(point_x, point_y, 1).expect("a tile");
        assert_eq!(
            b.rows[cell_b].handle,
            Some(Handle(1)),
            "B's tile 1 is its own window at that point, never A's"
        );
    }

    #[test]
    fn horizontal_render_paints_fill_icon_title_and_dots() {
        // The paint contract on known pixels: the selected tile's cell
        // carries the band fill at its center, an unselected tile's
        // center stays the card background, and the `…` tile never
        // paints a band.
        let snap = band_snapshot(6);
        let model = build_band(&snap, Handle(2), 4, None).expect("builds");
        let scale = 2u32;
        let lw = model.width;
        let lh = model.height;
        let mut canvas = vec![0u8; (lw * scale * lh * scale * 4) as usize];
        render(
            &model,
            &mut canvas,
            lw * scale,
            lh * scale,
            lw * scale * 4,
            scale,
            &Palette::dark(),
            TextBackend::Bitmap,
        );
        let px = |lx: u32, ly: u32| {
            let x = lx * scale;
            let y = ly * scale;
            let index = (y * lw * scale + x) as usize * 4;
            u32::from_le_bytes(canvas[index..index + 4].try_into().expect("pixel"))
        };
        let tile_center_x = |index: u32| TILE_PAD_X + index * (TILE_W + TILE_GAP) + TILE_W / 2;
        // Interior of the selected tile (tile 1): the band fill — the
        // exact blended byte, like the vertical band pin.
        let band = Palette::dark().selected_bg;
        assert_eq!(px(tile_center_x(1), lh / 2), band, "selected tile fill");
        // An unselected tile's open center: the card background.
        let background = Palette::dark().background;
        assert_eq!(
            px(tile_center_x(2), lh / 2),
            background,
            "unselected tile stays the card"
        );
        // The dots tile (index 4): card background, never a band.
        assert_eq!(
            px(tile_center_x(4), lh / 2),
            background,
            "the continuation tile never paints a band"
        );
    }

    // --- glance-tiles: the compact pitch (G1) -----------------------------

    #[test]
    fn the_compact_pitch_pins_the_tile_cell() {
        // The owner-frozen decomposition (2026-11-09): the tile sheds
        // its title line and shrinks to a 64-px slot — the icon keeps
        // its 48-px weight — while the cell gains the pip territory
        // under the icon. Badge row + icon + pips, nothing else; the
        // slot pitch is the spec's ~72 px; every padding and gap stays
        // on the 8-grid.
        assert_eq!(TILE_W, 64, "the compact slot");
        assert_eq!(TILE_ICON, 48, "the icon keeps its weight");
        assert_eq!(TILE_W + TILE_GAP, 72, "the ~72-px slot pitch");
        assert_eq!(
            TILE_CELL_H,
            TEXT_H + TILE_PARENT_GAP + TILE_ICON + PIP_GAP + PIP_SIDE,
            "the cell is badge row + icon + pip territory, no title line"
        );
        for value in [
            TILE_W,
            TILE_GAP,
            TILE_PAD_X,
            TILE_PAD_Y,
            CAPTION_GAP,
            CAPTION_FLOOR_W,
        ] {
            assert_eq!(value % 8, 0, "{value} stays on the 8-grid");
        }
    }

    #[test]
    fn the_card_keeps_the_caption_line_and_the_working_height() {
        // One caption line above the strip, one tile cell below it, the
        // paddings around both — the working target keeps today's 168
        // px exactly (the caption takes the per-tile title lines'
        // place).
        assert_eq!(
            band_card_height(),
            2 * TILE_PAD_Y + TEXT_H + CAPTION_GAP + TILE_CELL_H
        );
        assert_eq!(band_card_height(), 168);
    }

    #[test]
    fn band_width_floors_short_strips_at_the_caption() {
        // The caption floor: a short strip renders inside a card wide
        // enough that the caption reads at a glance — the shape's width
        // when it exceeds the floor, the floor when it does not. Tiles
        // stay left-aligned in the floored width.
        let shape = |slots: u32| 2 * TILE_PAD_X + slots * TILE_W + (slots - 1) * TILE_GAP;
        assert_eq!(band_width(3), CAPTION_FLOOR_W, "a three-tile shape floored");
        assert!(shape(3) < CAPTION_FLOOR_W);
        assert_eq!(band_width(6), CAPTION_FLOOR_W, "six tiles still floored");
        assert_eq!(
            band_width(7),
            shape(7),
            "seven tiles already outspan the floor"
        );
        assert!(shape(7) > CAPTION_FLOOR_W);
        // The caption's ink budget at the floor: the card's width minus
        // its horizontal padding.
        assert_eq!(CAPTION_FLOOR_W - 2 * TILE_PAD_X, 464);
    }

    // --- glance-tiles: the tile paints (G2) -------------------------------

    /// Renders `model` with the bitmap backend at scale 2 — the
    /// tile-paint probes' shared footing.
    fn render_bitmap(model: &OverlayModel) -> Vec<u8> {
        let scale = 2u32;
        let (w, h) = (model.width * scale, model.height * scale);
        let stride = w * 4;
        let mut canvas = vec![0u8; (stride * h) as usize];
        render(
            model,
            &mut canvas,
            w,
            h,
            stride,
            scale,
            &Palette::dark(),
            TextBackend::Bitmap,
        );
        canvas
    }

    /// The logical-pixel probe over a scale-2 bitmap canvas.
    fn probe(canvas: &[u8], lw: u32, lx: u32, ly: u32) -> u32 {
        let (x, y) = (lx * 2, ly * 2);
        let index = (y * lw * 2 + x) as usize * 4;
        u32::from_le_bytes(canvas[index..index + 4].try_into().expect("pixel"))
    }

    /// A hand-built horizontal card with one tile per `counts` entry —
    /// kind Parent with that count (Leaf for zero), unselected,
    /// unhinted, iconless — the pip and badge probes' fixture.
    fn pip_card(counts: &[usize]) -> OverlayModel {
        let rows = counts
            .iter()
            .map(|&count| RowModel {
                title: format!("tile {count}"),
                icon: None,
                selected: false,
                handle: Some(Handle(count as u64 + 1)),
                kind: if count == 0 {
                    RowKind::Leaf
                } else {
                    RowKind::Parent
                },
                count,
                hint: None,
                expanded: false,
            })
            .collect();
        OverlayModel {
            workspace: String::new(),
            rows,
            width: band_width(counts.len() as u32),
            height: band_card_height(),
            viewport: Viewport::default(),
            profile: StyleProfile::default(),
            horizontal: true,
            query: None,
            hovered: None,
            caption: String::new(),
        }
    }

    // --- glance-tiles: the one caption (G3) -------------------------------

    #[test]
    fn the_caption_names_the_flat_bands_selected_window() {
        // The flat band's caption is the selected window's own title —
        // the window Alt-release would activate.
        let snap = band_snapshot(4);
        let model = build_band(&snap, Handle(2), 4, None).expect("builds");
        assert_eq!(model.caption, "W02");
        let stepped = build_band(&snap, Handle(3), 4, None).expect("builds");
        assert_eq!(
            stepped.caption, "W03",
            "the caption follows the selection's target"
        );
    }

    #[test]
    fn the_caption_names_the_app_tiles_mru_front_window() {
        // An app tile commits its MRU-front window, so the caption
        // names that window's title — the icon already names the app;
        // the tile's own label (`app.a`) is not the commit target's
        // name.
        let (snap, closed, _open) = combined_fixture();
        let model = build_strip(&snap, &closed, 0, 4, 8).expect("builds");
        assert_eq!(
            model.rows[0].title, "app.a",
            "the tile still labels the app"
        );
        assert_eq!(
            model.caption, "a-front",
            "the caption names the front window"
        );
    }

    #[test]
    fn the_caption_names_a_window_tiles_own_title() {
        // A window tile's commit target is itself: the caption is its
        // own title — here the open run's back window.
        let (snap, _closed, open) = combined_fixture();
        let model = build_strip(&snap, &open, 2, 4, 8).expect("builds");
        assert_eq!(model.rows[0].title, "a-front");
        assert_eq!(
            model.caption, "a-back",
            "the selected window tile's own title"
        );
    }

    #[test]
    fn the_caption_falls_back_through_the_display_title_rule() {
        // The caption composes through `display_title`: a blank title
        // falls to the app_id, a blank both to `(untitled)` — the same
        // rule every title on the card follows.
        let mut snap = band_snapshot(2);
        let target = snap
            .toplevels
            .iter_mut()
            .find(|t| t.handle == Handle(2))
            .expect("the fixture window");
        target.title = String::new();
        target.app_id = String::new();
        let model = build_band(&snap, Handle(2), 2, None).expect("builds");
        assert_eq!(model.caption, UNTITLED);
    }

    #[test]
    fn the_caption_truncates_with_a_real_ellipsis_within_the_budget() {
        // Budget = the card's width minus its horizontal padding; a
        // longer title ends in the real `…` (both backends), never
        // past the budget's columns.
        let mut snap = band_snapshot(1);
        snap.toplevels[0].title = "W".repeat(120);
        let model = build_band(&snap, Handle(1), 2, None).expect("builds");
        let budget_cols = (model.width - 2 * TILE_PAD_X) / TEXT_W;
        assert!(model.caption.ends_with('…'), "the bitmap floor ellipsizes");
        assert!(model.caption.chars().count() <= budget_cols as usize);
        assert!(model.caption.starts_with("WWWW"), "it kept the head");
        // The System face composes the same budget with its own
        // metrics: real `…`, within the pixel budget.
        let mut snap = band_snapshot(1);
        snap.toplevels[0].title = "W".repeat(120);
        let mut composer = fixture_text();
        let model = overlay_model_horizontal(
            &snap,
            Handle(1),
            Scope::CurrentWorkspace,
            &no_icons(),
            2,
            &Palette::dark(),
            StyleProfile::default(),
            Banding {
                tiles: 2,
                width_floor: None,
                prev: None,
            },
            TextBackend::System {
                system: &mut composer,
                query: crate::text::FontQuery::SansSerif,
            },
        )
        .expect("builds");
        assert!(model.caption.ends_with('…'), "the system face ellipsizes");
        assert!(model.caption.starts_with("WWWW"));
    }

    #[test]
    fn the_caption_stays_centered_whatever_the_selection() {
        // Fixed-center (GNOME, KDE, Gala — the record's stability
        // finding): the caption's ink never moves with the selection.
        // Two cards differing only in which tile holds the selection
        // paint byte-identical caption lines.
        let card = |selected: usize| {
            let mut model = pip_card(&[0, 1]);
            model.rows[selected].selected = true;
            model.caption = "Shared title".to_string();
            model
        };
        let caption_line = |canvas: &[u8]| -> Vec<u8> {
            let stride = (band_width(2) * 2 * 4) as usize;
            let y = ((TILE_PAD_Y + TEXT_H / 2) * 2) as usize;
            canvas[y * stride..(y + 1) * stride].to_vec()
        };
        let first = render_bitmap(&card(0));
        let second = render_bitmap(&card(1));
        assert_eq!(
            caption_line(&first),
            caption_line(&second),
            "the caption line never moves"
        );
    }

    #[test]
    fn the_caption_paints_top_center_in_both_backends() {
        // The paint: full text color, centered on the reserved line,
        // its first ink exactly where centering puts it — and the
        // caption is the only ink on its line. Proven on the bitmap
        // floor; the System face paints the same line by the
        // shape_budgeted twin above.
        let snap = band_snapshot(4);
        let model = build_band(&snap, Handle(1), 4, None).expect("builds");
        let canvas = render_bitmap(&model);
        let text = Palette::dark().text;
        let background = Palette::dark().background;
        let cols = model.caption.chars().count() as u32;
        let start = (model.width - cols * TEXT_W) / 2;
        let line_y = TILE_PAD_Y + TEXT_H / 2;
        let mut ink = 0;
        for lx in start..start + cols * TEXT_W {
            ink += u32::from(probe(&canvas, model.width, lx, line_y) == text);
        }
        assert!(ink > 0, "the caption's ink paints on its line");
        // Nothing paints left of the centered start.
        let mut stray = 0;
        for lx in TILE_PAD_X..start {
            stray += u32::from(probe(&canvas, model.width, lx, line_y) != background);
        }
        assert_eq!(stray, 0, "the caption is the only ink on its line");
    }

    // --- The workspace label's paint (workspace-label W3) ------------------

    #[test]
    fn the_workspace_label_paints_bottom_center_on_the_horizontal_card() {
        // Centered on the card below the tiles, at its smaller cell —
        // the caption's quiet counterpart at the other end. Proven on
        // the bitmap floor; the System face paints the same line by
        // the shaped twin.
        let snap = band_snapshot(4);
        let model = build_band(&snap, Handle(1), 4, None).expect("builds");
        let mut text = TextBackend::Bitmap;
        let labeled = with_workspace(model, "code", &mut text, 2);
        let canvas = render_bitmap(&labeled);
        let text_color = Palette::dark().text;
        let label_cell = ws_label_cell_at(TEXT_H);
        let ink = 4 * label_cell;
        let y = band_card_height() + WS_LABEL_GAP + label_cell / 2;
        let left = (labeled.width - ink) / 2;
        let mut found = 0;
        for lx in left..left + ink {
            found += u32::from(probe(&canvas, labeled.width, lx, y) == text_color);
        }
        assert!(found > 0, "the label's ink paints centered on its line");
        // The clearing between the tiles and the line holds nothing.
        let mut stray = 0;
        for lx in (labeled.width / 4)..(labeled.width * 3 / 4) {
            stray += u32::from(
                probe(
                    &canvas,
                    labeled.width,
                    lx,
                    band_card_height() + WS_LABEL_GAP / 2,
                ) != Palette::dark().background,
            );
        }
        assert_eq!(stray, 0, "the gap between tiles and label line is clean");
        // And the line addresses no tile: cell_at over it picks nothing.
        assert_eq!(labeled.cell_at((left + 2) as f64, (y + 1) as f64, 1), None);
    }

    #[test]
    fn the_vertical_card_paints_the_label_above_its_rows() {
        // The reserved line above the first row carries the name,
        // left-aligned at the text column's spine — the query strip's
        // own x; without the label the same line is bare — the
        // default-off identity.
        let plain = hit_model(2);
        let mut text = TextBackend::Bitmap;
        let labeled = with_workspace(plain.clone(), "ab", &mut text, 2);
        let canvas = render_bitmap(&labeled);
        let text_color = Palette::dark().text;
        let label_cell = ws_label_cell_at(TEXT_H);
        let ink = 2 * label_cell;
        let left = PAD_X + ICON + ICON_GAP;
        let y = labeled.profile.rhythm.pad_y + label_cell / 2;
        let mut found = 0;
        for lx in left..left + ink {
            found += u32::from(probe(&canvas, labeled.width, lx, y) == text_color);
        }
        assert!(found > 0, "the label paints on its reserved line");
        let bare = render_bitmap(&plain);
        let mut none = 0;
        for lx in left..left + ink {
            none += u32::from(probe(&bare, plain.width, lx, y) == text_color);
        }
        assert_eq!(none, 0, "the default-off card paints nothing there");
    }

    #[test]
    fn cell_at_skips_the_reserved_caption_line() {
        // The caption line and its gap are the top padding's twin (the
        // query strip's rule, one axis over): no tile paints there, so
        // no click or hover addresses a tile from there. The strip
        // below answers as always.
        let snap = band_snapshot(4);
        let model = build_band(&snap, Handle(1), 4, None).expect("builds");
        let center_x = (TILE_PAD_X + TILE_W / 2) as f64;
        assert_eq!(
            model.cell_at(center_x, (TILE_PAD_Y + TEXT_H / 2) as f64, 1),
            None,
            "the caption line addresses nothing"
        );
        assert_eq!(
            model.cell_at(center_x, (TILE_PAD_Y + TEXT_H) as f64, 1),
            None,
            "the caption gap addresses nothing"
        );
        assert_eq!(
            model.cell_at(center_x, (band_strip_top() + 1) as f64, 1),
            Some(0),
            "the strip answers from its own top"
        );
    }

    #[test]
    fn tile_badges_paint_the_bare_digit_not_the_chord() {
        // The compact tile's badge is the digit alone: "Ctrl+9" is six
        // bitmap columns against the four a 64-px tile can fund, so the
        // corner badge paints `1`..`9` and the chord's words stay on
        // the vertical cards' hint bands. One column of muted ink in
        // the badge row, nothing past it.
        let snap = band_snapshot(4);
        let model = build_band(&snap, Handle(1), 4, None).expect("builds");
        let canvas = render_bitmap(&model);
        let muted = Palette::dark().muted;
        let badge_top = band_strip_top();
        for index in 0..3u32 {
            let left = TILE_PAD_X + index * (TILE_W + TILE_GAP);
            let first_col = left + TILE_TEXT_PAD;
            let mut digit_ink = 0;
            let mut tail_ink = 0;
            for ly in badge_top..badge_top + TEXT_H {
                for lx in first_col..first_col + TEXT_W {
                    digit_ink += u32::from(probe(&canvas, model.width, lx, ly) == muted);
                }
                for lx in first_col + TEXT_W..left + TILE_W - TILE_TEXT_PAD {
                    tail_ink += u32::from(probe(&canvas, model.width, lx, ly) == muted);
                }
            }
            assert!(digit_ink > 0, "tile {index}: the digit paints");
            assert_eq!(
                tail_ink, 0,
                "tile {index}: no chord tail beyond the digit's column"
            );
        }
    }

    #[test]
    fn count_pips_paint_only_on_multi_window_app_tiles() {
        // R4's marker: pips under the icon, only while the app holds
        // ≥ 2 live windows, saturating at three (Unity, GNOME's arrow,
        // dash-to-dock). Exact pip counts by pixel census in the pip
        // zone — zero below two, exact to three, saturated past three.
        let model = pip_card(&[0, 1, 2, 3, 5]);
        let canvas = render_bitmap(&model);
        let muted = Palette::dark().muted;
        let background = Palette::dark().background;
        let pip_top = band_strip_top() + TEXT_H + TILE_PARENT_GAP + TILE_ICON + PIP_GAP;
        let ink_per_pip = PIP_SIDE * PIP_SIDE;
        for (index, &count) in [0, 1, 2, 3, 5].iter().enumerate() {
            let left = TILE_PAD_X + index as u32 * (TILE_W + TILE_GAP);
            let mut pips = 0;
            for ly in pip_top..pip_top + PIP_SIDE {
                for lx in left..left + TILE_W {
                    pips += u32::from(probe(&canvas, model.width, lx, ly) == muted);
                }
            }
            let expected = match count {
                0 | 1 => 0,
                2 => 2 * ink_per_pip,
                _ => 3 * ink_per_pip,
            };
            assert_eq!(pips, expected, "count {count}: the pip census is exact");
            // The zone below a pipless tile's icon is the plain card —
            // the per-tile title line is gone, nothing else paints
            // there.
            if count < 2 {
                assert_eq!(
                    probe(&canvas, model.width, left + TILE_W / 2, pip_top),
                    background,
                    "count {count}: no pip, no title, just the card"
                );
            }
        }
    }

    #[test]
    fn the_strip_sits_below_the_reserved_caption_line() {
        // The card reserves the caption line above the strip (G1's
        // height formula): the strip's cells start one caption line
        // and its gap below the top padding, and the selection fill
        // never enters the reserved line.
        let snap = band_snapshot(4);
        let model = build_band(&snap, Handle(1), 4, None).expect("builds");
        let canvas = render_bitmap(&model);
        let background = Palette::dark().background;
        let band = Palette::dark().selected_bg;
        // The reserved caption line at the selected tile's center
        // column: card, never fill.
        let caption_y = TILE_PAD_Y + TEXT_H / 2;
        assert_eq!(
            probe(&canvas, model.width, TILE_PAD_X + TILE_W / 2, caption_y),
            background,
            "the caption line never carries the selection fill"
        );
        // The fill follows the strip's own top.
        let mid_cell = band_strip_top() + TILE_CELL_H / 2;
        let inset_x = TILE_PAD_X + TILE_TEXT_PAD / 2;
        assert_eq!(
            probe(&canvas, model.width, inset_x, mid_cell),
            band,
            "the selected tile fills from the strip's top"
        );
    }

    #[test]
    fn system_tiles_paint_the_digit_and_pips_too() {
        // Both backends are honest faces of one budget (the module
        // boundary): the System face paints the same digit-only badge
        // — no chord tail within two columns of bearings — and the
        // same solid pip squares under a multi-window app tile.
        let (snap, closed, _open) = combined_fixture();
        let model = build_strip(&snap, &closed, 0, 4, 8).expect("builds");
        let scale = 2u32;
        let (w, h) = (model.width * scale, model.height * scale);
        let stride = w * 4;
        let mut canvas = vec![0u8; (stride * h) as usize];
        let mut system = fixture_text();
        render(
            &model,
            &mut canvas,
            w,
            h,
            stride,
            scale,
            &Palette::dark(),
            TextBackend::System {
                system: &mut system,
                query: crate::text::FontQuery::SansSerif,
            },
        );
        let px = |lx: u32, ly: u32| {
            let (x, y) = (lx * scale, ly * scale);
            let index = (y * w + x) as usize * 4;
            u32::from_le_bytes(canvas[index..index + 4].try_into().expect("pixel"))
        };
        let muted = Palette::dark().muted;
        // Tile 0 (the parent, count 2): the badge row holds digit ink
        // within the first two columns and no chord tail beyond them.
        let left = TILE_PAD_X;
        let badge_top = band_strip_top();
        let mut digit_ink = 0;
        let mut tail_ink = 0;
        for ly in badge_top..badge_top + TEXT_H {
            for lx in left + TILE_TEXT_PAD..left + TILE_TEXT_PAD + 2 * TEXT_W {
                digit_ink += u32::from(px(lx, ly) == muted);
            }
            for lx in left + TILE_TEXT_PAD + 2 * TEXT_W..left + TILE_W - TILE_TEXT_PAD {
                tail_ink += u32::from(px(lx, ly) == muted);
            }
        }
        assert!(digit_ink > 0, "the digit paints through the system face");
        assert_eq!(tail_ink, 0, "no chord tail beyond the digit");
        // The parent's two pips: solid muted squares at their centers.
        let pip_top = band_strip_top() + TEXT_H + TILE_PARENT_GAP + TILE_ICON + PIP_GAP;
        let center = left + TILE_W / 2;
        let span = PIP_STEP; // two pips: centers one step apart
        for side in [center - span / 2, center + span / 2] {
            let pip_x = side - PIP_SIDE / 2;
            assert_eq!(px(pip_x + PIP_SIDE / 2, pip_top + PIP_SIDE / 2), muted);
        }
    }

    // --- list-viewport: the windowed card (LV2) --------------------------

    /// A `count`-window snapshot on one workspace, MRU-ordered so handle
    /// `n` sits at list index `n − 1`; fixed-width titles keep width
    /// assertions readable.
    fn long_snapshot(count: u64) -> Snapshot {
        let toplevels = (1..=count)
            .map(|handle| {
                toplevel(
                    handle,
                    &format!("W{handle:02}"),
                    "app.a",
                    &[1],
                    if handle == 1 {
                        switcher::State::ACTIVATED
                    } else {
                        switcher::State::default()
                    },
                    count + 1 - handle,
                )
            })
            .collect();
        snapshot(toplevels, Some(1))
    }

    fn build_windowed(
        snap: &Snapshot,
        selected: Handle,
        rows: usize,
        prev: Option<Viewport>,
    ) -> Option<OverlayModel> {
        overlay_model(
            snap,
            selected,
            Scope::CurrentWorkspace,
            &no_icons(),
            2,
            &Palette::dark(),
            StyleProfile::default(),
            Windowing {
                rows,
                width_floor: None,
                prev,
            },
            TextBackend::Bitmap,
        )
    }

    /// Like [`build`], grouped: the app tree over the same snapshot,
    /// collapsed by default — `expanded` names the apps standing open,
    /// `selected` the selected *tree row* (the daemon selects rows, not
    /// handles, in this mode).
    fn build_grouped(snap: &Snapshot, selected: usize, expanded: &[&str]) -> Option<OverlayModel> {
        let tree = crate::switcher::Grouping::new(
            &snap.toplevels,
            snap.active.expect("fixture has an active workspace"),
            Scope::CurrentWorkspace,
        );
        let expanded: std::collections::HashSet<String> =
            expanded.iter().map(|app| app.to_string()).collect();
        let visible = tree.visible(&expanded);
        overlay_model_grouped(
            &tree,
            &visible,
            selected,
            snap,
            &no_icons(),
            2,
            &Palette::dark(),
            StyleProfile::default(),
            Windowing {
                rows: DEFAULT_ROWS,
                width_floor: None,
                prev: None,
            },
            TextBackend::Bitmap,
        )
    }

    // --- Grouped model ----------------------------------------------------

    /// The grouped fixture the model tests read: MRU a1, b1, a2, u —
    /// two windows of app a, one of b, one un-attributable.
    fn grouped_snap() -> Snapshot {
        snapshot(
            vec![
                toplevel(1, "a-front", "app.a", &[1], switcher::State::ACTIVATED, 40),
                toplevel(2, "b-only", "app.b", &[1], switcher::State::default(), 30),
                toplevel(3, "a-back", "app.a", &[1], switcher::State::default(), 20),
                toplevel(4, "nowhere", "", &[1], switcher::State::default(), 10),
            ],
            Some(1),
        )
    }

    #[test]
    fn grouped_model_collapses_by_default_and_lists_all_windows_expanded() {
        // Collapsed (the default): only level-1 rows — the parent entry
        // carrying its count pips, and the leaves under their window
        // titles.
        // Expanded: the parent's whole window set follows as children,
        // front window first, unhinted (rev 5: children carry no
        // chord).
        let snap = grouped_snap();
        let collapsed = build_grouped(&snap, 0, &[]).expect("eligible windows exist");
        let rows: Vec<(&str, crate::switcher::RowKind, usize, Option<RowHint>, bool)> = collapsed
            .rows
            .iter()
            .map(|row| {
                (
                    row.title.as_str(),
                    row.kind,
                    row.count,
                    row.hint,
                    row.expanded,
                )
            })
            .collect();
        assert_eq!(
            rows,
            vec![
                (
                    "app.a",
                    crate::switcher::RowKind::Parent,
                    2,
                    Some(RowHint::Ctrl(1)),
                    false
                ),
                (
                    "b-only",
                    crate::switcher::RowKind::Leaf,
                    0,
                    Some(RowHint::Ctrl(2)),
                    false
                ),
                (
                    "nowhere",
                    crate::switcher::RowKind::Leaf,
                    0,
                    Some(RowHint::Ctrl(3)),
                    false
                ),
            ]
        );
        let expanded = build_grouped(&snap, 0, &["app.a"]).expect("eligible windows exist");
        let rows: Vec<(&str, crate::switcher::RowKind, Option<RowHint>, bool)> = expanded
            .rows
            .iter()
            .map(|row| (row.title.as_str(), row.kind, row.hint, row.expanded))
            .collect();
        assert_eq!(
            rows,
            vec![
                (
                    "app.a",
                    crate::switcher::RowKind::Parent,
                    Some(RowHint::Ctrl(1)),
                    true
                ),
                ("a-front", crate::switcher::RowKind::Child, None, false),
                ("a-back", crate::switcher::RowKind::Child, None, false),
                (
                    "b-only",
                    crate::switcher::RowKind::Leaf,
                    Some(RowHint::Ctrl(2)),
                    false
                ),
                (
                    "nowhere",
                    crate::switcher::RowKind::Leaf,
                    Some(RowHint::Ctrl(3)),
                    false
                ),
            ]
        );
    }

    #[test]
    fn grouped_model_selects_by_tree_row_not_handle() {
        // A parent and its first child carry the same window — the
        // group's front — so handle matching cannot tell them apart.
        // The model marks the selected *tree row*: selecting the parent
        // highlights the entry, selecting the front child highlights
        // the child.
        let snap = grouped_snap();
        let model = build_grouped(&snap, 0, &["app.a"]).expect("eligible windows exist");
        assert_eq!(model.rows[0].handle, Some(Handle(1)));
        assert_eq!(model.rows[1].handle, Some(Handle(1)));
        assert!(model.rows[0].selected, "the parent row is the selection");
        assert!(!model.rows[1].selected);

        let model = build_grouped(&snap, 1, &["app.a"]).expect("eligible windows exist");
        assert!(!model.rows[0].selected);
        assert!(
            model.rows[1].selected,
            "the front child row is the selection"
        );
    }

    #[test]
    fn grouped_model_hints_every_header_and_no_child() {
        // Rev 5: headers are normal rows — an expanded entry hints its
        // Ctrl+N chord like any entry — and children hint nothing (the
        // Alt+N chords are gone).
        let snap = grouped_snap();
        let model = build_grouped(&snap, 1, &["app.a"]).expect("eligible windows exist");
        let rows: Vec<(crate::switcher::RowKind, Option<RowHint>)> =
            model.rows.iter().map(|row| (row.kind, row.hint)).collect();
        assert_eq!(
            rows,
            vec![
                (crate::switcher::RowKind::Parent, Some(RowHint::Ctrl(1))),
                (crate::switcher::RowKind::Child, None),
                (crate::switcher::RowKind::Child, None),
                (crate::switcher::RowKind::Leaf, Some(RowHint::Ctrl(2))),
                (crate::switcher::RowKind::Leaf, Some(RowHint::Ctrl(3))),
            ]
        );
    }

    #[test]
    fn grouped_model_sizes_for_indent_and_extras() {
        // The card widens for the tree: a child's indent and a parent's
        // caret terrace join the flat title budget. The bands are pixel
        // measures — the 16-px child indent is not a whole text column.
        let snap = grouped_snap();
        let flat =
            build(&snap, Handle(1), Scope::CurrentWorkspace).expect("eligible windows exist");
        let grouped = build_grouped(&snap, 0, &["app.a"]).expect("eligible windows exist");
        let fixed = 2 * PAD_X + ICON + ICON_GAP + HINT_GAP + HINT_COLS * TEXT_W;
        // Longest flat title is 7 cols ("a-front", "nowhere"); the tree
        // budget is the parent's 5 title cols + CHEVRON_COLS, against the
        // child's 7 title cols + TREE_INDENT.
        let flat_band = 7 * TEXT_W;
        let parent_band = 5 * TEXT_W + CHEVRON_COLS * TEXT_W;
        let child_band = 7 * TEXT_W + TREE_INDENT;
        assert_eq!(flat.width, fixed + flat_band);
        assert_eq!(grouped.width, fixed + parent_band.max(child_band));
        assert!(
            grouped.width > flat.width,
            "the tree pays for its indent and caret"
        );
    }

    #[test]
    fn grouped_model_windows_the_visible_sequence() {
        // One list, one budget: `rows`/`…` apply over the visible rows
        // — parents and expanded children together — through the same
        // windowing rule. Here a 9-row expanded tree (a×3 grouped, b×2
        // grouped, two leaves) at budget 3 keeps 5 slots with both
        // edges truncated, the selection mid-list.
        let snap = snapshot(
            vec![
                toplevel(1, "a1", "app.a", &[1], switcher::State::ACTIVATED, 70),
                toplevel(2, "a2", "app.a", &[1], switcher::State::default(), 60),
                toplevel(3, "a3", "app.a", &[1], switcher::State::default(), 50),
                toplevel(4, "b1", "app.b", &[1], switcher::State::default(), 40),
                toplevel(5, "b2", "app.b", &[1], switcher::State::default(), 30),
                toplevel(6, "c1", "app.c", &[1], switcher::State::default(), 20),
                toplevel(7, "u1", "", &[1], switcher::State::default(), 10),
            ],
            Some(1),
        );
        // Expanded tree rows: P-a1, C-a1, C-a2, C-a3, P-b1, C-b1,
        // C-b2, L-c1, L-u1 — handles 1,1,2,3,4,4,5,6,7. Selecting the
        // b parent (tree row 4) at budget 3: 5 slots from 2, both
        // edges truncated.
        let tree = crate::switcher::Grouping::new(
            &snap.toplevels,
            snap.active.expect("fixture has an active workspace"),
            Scope::CurrentWorkspace,
        );
        let expanded: std::collections::HashSet<String> = ["app.a", "app.b"]
            .iter()
            .map(|app| app.to_string())
            .collect();
        let visible = tree.visible(&expanded);
        let model = overlay_model_grouped(
            &tree,
            &visible,
            4,
            &snap,
            &no_icons(),
            2,
            &Palette::dark(),
            StyleProfile::default(),
            Windowing {
                rows: 3,
                width_floor: None,
                prev: None,
            },
            TextBackend::Bitmap,
        )
        .expect("eligible windows exist");
        let handles: Vec<Option<Handle>> = model.rows.iter().map(|row| row.handle).collect();
        assert_eq!(
            handles,
            vec![
                None, // the `…` continuation slot above
                Some(Handle(2)),
                Some(Handle(3)),
                Some(Handle(4)),
                None, // the `…` continuation slot below
            ]
        );
        assert!(model.viewport.top && model.viewport.bottom);
        assert_eq!(model.viewport.content, 3);
    }

    #[test]
    fn grouped_render_indents_the_child_row() {
        // The child's whole cell — generic window glyph included — sits
        // TREE_INDENT right of the top-level column, so the pixel where
        // a parent's glyph starts is pure card on the child's row, and
        // the child's ink appears one indent over.
        let model = build_grouped(&grouped_snap(), 0, &["app.a"]).expect("eligible windows exist");
        let (canvas, stride) = rendered(&model, 2);
        let side = GLYPH_W * GLYPH_SCALE;
        let glyph_x = PAD_X + (ICON - side) / 2;
        let glyph_y = PAD_Y + (ROW_H - ICON) / 2 + (ICON - side) / 2;
        let child_y = glyph_y + (ROW_H + ROW_GAP);
        // Parent row: glyph ink at the top-level origin.
        assert_eq!(pixel(&canvas, stride, glyph_x * 2, glyph_y * 2), COLOR_TEXT);
        // Child row: that origin is card background…
        assert_eq!(
            pixel(&canvas, stride, glyph_x * 2, child_y * 2),
            COLOR_BACKGROUND
        );
        // …and the ink sits one TREE_INDENT to the right.
        assert_eq!(
            pixel(&canvas, stride, (glyph_x + TREE_INDENT) * 2, child_y * 2),
            COLOR_TEXT
        );
    }

    #[test]
    fn render_draws_the_inline_disclosure_caret() {
        // Re-pinned 2026-11-09 (owner, in-thread): the disclosure mark
        // is an 8-px, two-pixel-stroke caret in the text color, one
        // CHEVRON_GAP after the parent's title — inline tree structure,
        // not the 16-px solid wedge that floated in the
        // hint-gap column and read as a play button. Collapsed points
        // right, expanded points down; a leaf paints nothing.
        let collapsed = build_grouped(&grouped_snap(), 3, &[]).expect("eligible windows exist");
        let (canvas, stride) = rendered(&collapsed, 2);
        let side = GLYPH_W * CHEVRON_SCALE;
        // The parent's "app.a" (5 cols) ends on the text line; the
        // caret follows one CHEVRON_GAP later. The pips live below the
        // title and never enter this terrace.
        let title_end = PAD_X + ICON + ICON_GAP + 5 * TEXT_W;
        let chev_x = title_end + CHEVRON_GAP;
        let chev_y = PAD_Y + (ROW_H - side) / 2;
        // Font pixel (col, row) of the caret cell, in device px.
        let mark = |col: u32, row: u32| {
            (
                (chev_x + col * CHEVRON_SCALE) * 2,
                (chev_y + row * CHEVRON_SCALE) * 2,
            )
        };
        // The right-pointing caret's two-pixel stroke.
        let (x, y) = mark(2, 1);
        assert_eq!(
            pixel(&canvas, stride, x, y),
            COLOR_TEXT,
            "collapsed: the upper stroke is text-strong"
        );
        let (x, y) = mark(4, 3);
        assert_eq!(
            pixel(&canvas, stride, x, y),
            COLOR_TEXT,
            "collapsed: the point is text-strong"
        );
        let (x, y) = mark(0, 0);
        assert_eq!(
            pixel(&canvas, stride, x, y),
            COLOR_BACKGROUND,
            "collapsed: the cell's corner stays card"
        );
        // A leaf earns no caret: the whole cell is card.
        let (x, y) = (mark(2, 1).0, mark(2, 1).1 + 2 * (ROW_H + ROW_GAP) * 2);
        assert_eq!(
            pixel(&canvas, stride, x, y),
            COLOR_BACKGROUND,
            "a leaf paints nothing in the caret cell"
        );

        // Expanded: the caret points down — ink where the right-pointing
        // stroke was card and vice versa.
        let expanded =
            build_grouped(&grouped_snap(), 3, &["app.a"]).expect("eligible windows exist");
        let (canvas, stride) = rendered(&expanded, 2);
        let (x, y) = mark(3, 5);
        assert_eq!(
            pixel(&canvas, stride, x, y),
            COLOR_TEXT,
            "expanded: ink at the down point"
        );
        let (x, y) = mark(2, 3);
        assert_eq!(
            pixel(&canvas, stride, x, y),
            COLOR_TEXT,
            "expanded: ink along the top arms"
        );
        let (x, y) = mark(4, 3);
        assert_eq!(
            pixel(&canvas, stride, x, y),
            COLOR_BACKGROUND,
            "expanded: the right point is card where the collapsed caret was solid"
        );
    }

    #[test]
    fn render_paints_the_count_pips_below_the_title() {
        // Re-pinned 2026-11-09 (owner, in-thread): the parent's count is
        // the horizontal tiles' pip vocabulary, adapted below the app
        // name — two muted squares for app.a's two windows, in the row's
        // own bottom padding. The title line keeps the caret and the
        // hint only; the count spends no title width. No fill shape, no
        // digit.
        let model = build_grouped(&grouped_snap(), 3, &[]).expect("eligible windows exist");
        let (canvas, stride) = rendered(&model, 2);
        let title_x = PAD_X + ICON + ICON_GAP; // the parent is un-indented
        let pip_y = PAD_Y + ROW_H - ROW_PAD_Y - PIP_SIDE;
        let muted = Palette::dark().muted;
        let pips = 2; // the fixture's app.a holds two windows
        for n in 0..pips {
            assert_eq!(
                pixel(
                    &canvas,
                    stride,
                    (title_x + n * PIP_STEP + PIP_SIDE / 2) * 2,
                    (pip_y + PIP_SIDE / 2) * 2,
                ),
                muted,
                "pip {n} paints muted"
            );
        }
        // The pips sit below the title's text line, not beside it.
        let text_bottom = PAD_Y + (ROW_H - TEXT_H) / 2 + TEXT_H;
        assert!(
            pip_y >= text_bottom,
            "the pips sit below the title's text line"
        );
        // The caret's two-column terrace is all the title line reserves;
        // the pips never enter it.
        let title_end = title_x + "app.a".chars().count() as u32 * TEXT_W;
        let edge = model.width - PAD_X - HINT_GAP - HINT_COLS * TEXT_W;
        assert!(title_end + CHEVRON_COLS * TEXT_W <= edge);
    }

    #[test]
    fn count_pips_saturate_at_three() {
        // Five windows still paint three pips — the band's saturation
        // rule, now the parent row's too. The fourth pip's slot stays
        // empty.
        let snap = snapshot(
            vec![
                toplevel(1, "w1", "app.a", &[1], switcher::State::ACTIVATED, 6),
                toplevel(2, "w2", "app.a", &[1], switcher::State::default(), 5),
                toplevel(3, "w3", "app.a", &[1], switcher::State::default(), 4),
                toplevel(4, "w4", "app.a", &[1], switcher::State::default(), 3),
                toplevel(5, "w5", "app.a", &[1], switcher::State::default(), 2),
                toplevel(6, "solo", "app.b", &[1], switcher::State::default(), 1),
            ],
            Some(1),
        );
        let model = build_grouped(&snap, 0, &[]).expect("eligible windows exist");
        let (canvas, stride) = rendered(&model, 2);
        let title_x = PAD_X + ICON + ICON_GAP;
        let pip_y = PAD_Y + ROW_H - ROW_PAD_Y - PIP_SIDE;
        let muted = Palette::dark().muted;
        for n in 0..3 {
            assert_eq!(
                pixel(
                    &canvas,
                    stride,
                    (title_x + n * PIP_STEP + PIP_SIDE / 2) * 2,
                    (pip_y + PIP_SIDE / 2) * 2,
                ),
                muted,
                "saturating pip {n} paints muted"
            );
        }
        assert_eq!(
            pixel(
                &canvas,
                stride,
                (title_x + 3 * PIP_STEP + PIP_SIDE / 2) * 2,
                (pip_y + PIP_SIDE / 2) * 2,
            ),
            COLOR_SELECTED_BG,
            "a fourth pip never paints"
        );
    }

    #[test]
    fn a_max_length_parent_title_keeps_its_pips_and_caret() {
        // The CHEVRON_COLS budget (2 columns) funds the caret and its
        // clearing even at the parent cap (MAX_COLS - CHEVRON_COLS);
        // the pips below the title spend no width. The card stays at
        // the widest flat width; nothing clips at the title band's
        // edge.
        let app = "a".repeat(MAX_COLS as usize + 10);
        let snap = snapshot(
            vec![
                toplevel(1, "front", &app, &[1], switcher::State::ACTIVATED, 2),
                toplevel(2, "back", &app, &[1], switcher::State::default(), 1),
            ],
            Some(1),
        );
        let model = build_grouped(&snap, 0, &[]).expect("eligible windows exist");
        assert_eq!(
            model.width,
            2 * PAD_X + ICON + ICON_GAP + MAX_COLS * TEXT_W + HINT_GAP + HINT_COLS * TEXT_W,
            "the parent's extras ride inside the flat title budget"
        );
        let (canvas, stride) = rendered(&model, 2);
        let title_end = PAD_X + ICON + ICON_GAP + (MAX_COLS - CHEVRON_COLS) * TEXT_W;
        // Count 2 paints two pips below the title; the caret follows
        // one CHEVRON_GAP after the title's ink.
        let pip_y = PAD_Y + ROW_H - ROW_PAD_Y - PIP_SIDE;
        assert_eq!(
            pixel(
                &canvas,
                stride,
                (PAD_X + ICON + ICON_GAP + PIP_SIDE / 2) * 2,
                (pip_y + PIP_SIDE / 2) * 2,
            ),
            Palette::dark().muted,
            "the pips paint below a max-length parent title"
        );
        let chev_x = title_end + CHEVRON_GAP;
        let chev_y = PAD_Y + (ROW_H - GLYPH_W * CHEVRON_SCALE) / 2;
        assert_eq!(
            pixel(
                &canvas,
                stride,
                (chev_x + 2 * CHEVRON_SCALE) * 2,
                (chev_y + CHEVRON_SCALE) * 2,
            ),
            COLOR_TEXT,
            "the caret paints at the end of a max-length parent row"
        );
        assert!(
            chev_x + GLYPH_W * CHEVRON_SCALE <= model.width - PAD_X - HINT_GAP - HINT_COLS * TEXT_W,
            "the caret ends inside the title band"
        );
    }

    #[test]
    fn a_list_within_the_budget_renders_whole_and_unchanged() {
        // The no-regression boundary, exactly at the default budget: no
        // affordance rows, every handle present, and the pre-module
        // geometry formulas verbatim — a ≤ 8-window gesture renders
        // byte-identical to the card before `rows` existed.
        let snap = long_snapshot(DEFAULT_ROWS as u64);
        let model =
            build_windowed(&snap, Handle(1), DEFAULT_ROWS, None).expect("eligible windows exist");
        assert_eq!(model.rows.len(), DEFAULT_ROWS);
        assert!(
            model.rows.iter().all(|row| row.handle.is_some()),
            "no affordance rows within the budget"
        );
        assert_eq!(
            model.viewport,
            Viewport {
                start: 0,
                content: DEFAULT_ROWS,
                top: false,
                bottom: false
            }
        );
        assert_eq!(
            model.width,
            2 * PAD_X + ICON + ICON_GAP + 3 * TEXT_W + HINT_GAP + HINT_COLS * TEXT_W
        );
        assert_eq!(
            model.height,
            2 * PAD_Y + DEFAULT_ROWS as u32 * ROW_H + (DEFAULT_ROWS as u32 - 1) * ROW_GAP
        );
    }

    #[test]
    fn a_long_lists_head_window_truncates_only_below() {
        // Fifteen windows under the default budget: ten slots — nine
        // windows in view, dots below.
        let snap = long_snapshot(15);
        let model =
            build_windowed(&snap, Handle(1), DEFAULT_ROWS, None).expect("eligible windows exist");
        assert_eq!(model.rows.len(), DEFAULT_ROWS + ROWS_SLACK);
        assert_eq!(
            model.viewport,
            Viewport {
                start: 0,
                content: DEFAULT_ROWS + 1,
                top: false,
                bottom: true
            }
        );
        let handles: Vec<Option<Handle>> = model.rows.iter().map(|row| row.handle).collect();
        let expected: Vec<Option<Handle>> = (1..=9)
            .map(|handle| Some(Handle(handle)))
            .chain([None])
            .collect();
        assert_eq!(handles, expected, "nine content rows, dots at the bottom");
    }

    #[test]
    fn an_interior_selection_windows_both_edges() {
        let snap = long_snapshot(15);
        // Carrying the head window while selecting list index 9 (handle
        // 10) shifts it by one row: dots above and below, exactly the
        // budget between them, the selection on the last content row.
        let model = build_windowed(
            &snap,
            Handle(10),
            DEFAULT_ROWS,
            Some(Viewport {
                start: 0,
                content: DEFAULT_ROWS + 1,
                top: false,
                bottom: true,
            }),
        )
        .expect("eligible windows exist");
        assert_eq!(
            model.viewport,
            Viewport {
                start: 1,
                content: DEFAULT_ROWS,
                top: true,
                bottom: true
            }
        );
        let handles: Vec<Option<Handle>> = model.rows.iter().map(|row| row.handle).collect();
        let expected: Vec<Option<Handle>> = [None]
            .into_iter()
            .chain((3..=10).map(|handle| Some(Handle(handle))))
            .chain([None])
            .collect();
        assert_eq!(handles, expected);
        assert_eq!(
            model.height,
            card_height((DEFAULT_ROWS + ROWS_SLACK) as u32),
            "the card at capacity stays bounded"
        );
    }

    #[test]
    fn the_card_never_breathes_while_the_window_moves() {
        // Width derives from the FULL eligible set: a max-cols title deep
        // in the list widens the head window that hides it, and every
        // step of a full sweep renders the same width.
        let toplevels: Vec<switcher::Toplevel> = (1..=15)
            .map(|handle| {
                let title = if handle == 13 {
                    "M".repeat(MAX_COLS as usize)
                } else {
                    format!("W{handle:02}")
                };
                toplevel(
                    handle,
                    &title,
                    "app.a",
                    &[1],
                    if handle == 1 {
                        switcher::State::ACTIVATED
                    } else {
                        switcher::State::default()
                    },
                    16 - handle,
                )
            })
            .collect();
        let snap = snapshot(toplevels, Some(1));

        let mut prev = None;
        let mut widths = Vec::new();
        let mut heights = Vec::new();
        for handle in 1..=15 {
            let model = build_windowed(&snap, Handle(handle), DEFAULT_ROWS, prev)
                .expect("eligible windows exist");
            widths.push(model.width);
            heights.push(model.height);
            prev = Some(model.viewport);
        }
        widths.dedup();
        assert_eq!(widths.len(), 1, "the card breathed across the sweep");
        heights.dedup();
        assert_eq!(
            heights.len(),
            1,
            "the card's height breathed across the sweep"
        );
        assert_eq!(
            widths[0],
            2 * PAD_X + ICON + ICON_GAP + MAX_COLS * TEXT_W + HINT_GAP + HINT_COLS * TEXT_W,
            "the hidden max-cols title must still fund the width"
        );
    }

    #[test]
    fn a_full_tab_sweep_keeps_the_selection_on_a_content_row() {
        let snap = long_snapshot(30);
        let mut prev = None;
        for handle in 1..=30 {
            let model = build_windowed(&snap, Handle(handle), DEFAULT_ROWS, prev)
                .expect("eligible windows exist");
            assert!(
                model.rows.len() <= DEFAULT_ROWS + ROWS_SLACK,
                "handle {handle}: card over cap"
            );
            let selected: Vec<&RowModel> = model.rows.iter().filter(|row| row.selected).collect();
            assert_eq!(selected.len(), 1, "handle {handle}: one selection");
            assert_eq!(
                selected[0].handle,
                Some(Handle(handle)),
                "handle {handle}: the selection must be a content row"
            );
            prev = Some(model.viewport);
        }
    }

    /// The device-pixel points of `slot`'s row interior — the band a
    /// selection would fill, clear of the border's edge reach.
    fn row_band_points(slot: usize, width: u32) -> impl Iterator<Item = (u32, u32)> {
        let top = (PAD_Y + slot as u32 * (ROW_H + ROW_GAP)) * 2;
        let x0 = SELECT_INSET * 2;
        let x1 = (width - SELECT_INSET) * 2;
        (x0..x1).flat_map(move |x| (top..top + ROW_H * 2).map(move |y| (x, y)))
    }

    #[test]
    fn render_draws_the_continuation_dots_and_never_a_band_on_them() {
        let snap = long_snapshot(15);
        let model = build_windowed(
            &snap,
            Handle(10),
            DEFAULT_ROWS,
            Some(Viewport {
                start: 0,
                content: DEFAULT_ROWS + 1,
                top: false,
                bottom: true,
            }),
        )
        .expect("eligible windows exist");
        for palette in [Palette::dark(), Palette::light()] {
            let (w, h) = (model.width * 2, model.height * 2);
            let stride = w * 4;
            let mut canvas = vec![0u8; (stride * h) as usize];
            render(
                &model,
                &mut canvas,
                w,
                h,
                stride,
                2,
                &palette,
                TextBackend::Bitmap,
            );
            for slot in [0, 9] {
                let mut paints = 0;
                let mut muted = 0;
                for (x, y) in row_band_points(slot, model.width) {
                    let argb = pixel(&canvas, stride, x, y);
                    assert_ne!(
                        argb, palette.selected_bg,
                        "a selection band intersected the dots row at slot {slot}"
                    );
                    if argb == palette.muted {
                        muted += 1;
                    }
                    if argb != palette.background {
                        paints += 1;
                    }
                }
                assert!(muted > 0, "slot {slot}: the dots paint in the muted color");
                assert!(
                    paints < row_band_points(slot, model.width).count(),
                    "slot {slot}: the dots row must stay card fill, not a band"
                );
            }
        }
    }

    #[test]
    fn affordance_rows_paint_no_hint() {
        // A dots row is not number-addressable, so it must not advertise
        // a chord — the content row beside it still does.
        let snap = long_snapshot(15);
        let model = build_windowed(
            &snap,
            Handle(10),
            DEFAULT_ROWS,
            Some(Viewport {
                start: 0,
                content: DEFAULT_ROWS + 1,
                top: false,
                bottom: true,
            }),
        )
        .expect("eligible windows exist");
        let (canvas, stride) = rendered(&model, 2);
        let palette = Palette::dark();
        for slot in [0, 9] {
            for (x, y) in hint_band_points(slot, model.width) {
                assert_eq!(
                    pixel(&canvas, stride, x, y),
                    palette.background,
                    "slot {slot} painted hint pixels"
                );
            }
        }
        let mut muted = 0;
        for (x, y) in hint_band_points(1, model.width) {
            if pixel(&canvas, stride, x, y) == palette.muted {
                muted += 1;
            }
        }
        assert!(muted > 0, "the content row beside the dots keeps its hint");
    }

    #[test]
    fn selection_band_rounds_its_ends_like_the_card_corners() {
        // The band is a row button: its own corners take the coverage
        // ramp — card fill at the exact corner, a blend along the arc,
        // exact fill from the first straight-edge pixel — composited
        // over the card, so the ramp never thins the near-opaque fill.
        let mut model = hit_model(3);
        model.rows[1].selected = true;
        let (canvas, stride) = rendered(&model, 2);
        let palette = Palette::dark();
        let band_top = (PAD_Y + ROW_H + ROW_GAP) * 2;
        let band_x = SELECT_INSET * 2;
        assert_eq!(
            pixel(&canvas, stride, band_x, band_top),
            palette.background,
            "the band's own corner pixel stays card fill"
        );
        let ramp = pixel(&canvas, stride, band_x + 10, band_top); // logical x = 5, mid-arc
        assert_ne!(ramp, palette.background, "the arc blends selection in");
        assert_ne!(ramp, palette.selected_bg, "the arc is not full fill yet");
        assert!(
            ramp >> 24 >= palette.background >> 24,
            "the coverage ramp never thins the card's own opacity"
        );
        assert_eq!(
            pixel(&canvas, stride, band_x + BAND_RADIUS * 2, band_top),
            palette.selected_bg,
            "the first straight-edge pixel is exact fill"
        );
        assert_eq!(
            pixel(&canvas, stride, band_x, band_top + ROW_H * 2 - 1),
            palette.background,
            "the band's bottom corner mirrors the top"
        );
    }

    #[test]
    fn continuation_dots_center_their_painted_glyph() {
        // The `…` glyph paints TEXT_W wide, so its box centers on
        // TEXT_W — centering on the font cell biased the ink 2 px right
        // of the card's center line. The expected ink derives from the
        // glyph's own set columns, not hardcoded positions.
        let mut model = hit_model(3);
        model.rows[0].handle = None; // the top continuation row
        model.rows[0].title = String::new(); // wordless, as continuation rows are
        let (canvas, stride) = rendered(&model, 2);
        let palette = Palette::dark();
        let dots_x = (model.width - TEXT_W) / 2;
        let block = TEXT_W * 2 / GLYPH_W; // device px per font pixel at scale 2
        let mut ink: Vec<u32> = Vec::new();
        for y in (PAD_Y * 2)..(PAD_Y + ROW_H) * 2 {
            for x in 0..model.width * 2 {
                if pixel(&canvas, stride, x, y) == palette.muted {
                    ink.push(x);
                }
            }
        }
        assert!(!ink.is_empty(), "the dots row paints muted ink");
        ink.sort_unstable();
        ink.dedup();
        // Set font columns of the ellipsis, mapped to device spans.
        let cols: Vec<u32> = (0..8)
            .filter(|col| ELLIPSIS_GLYPH[5] & (1 << col) != 0)
            .collect();
        assert_eq!(
            ink[0],
            dots_x * 2 + cols[0] * block,
            "the leftmost dot opens the glyph box at its centered origin"
        );
        assert_eq!(
            ink[ink.len() - 1],
            dots_x * 2 + (cols[cols.len() - 1] + 1) * block - 1,
            "the rightmost dot closes the glyph box at its centered origin"
        );
    }

    /// A plain `rows`-row model with the documented card rhythm, for
    /// hit-testing. Bands run PAD_Y + i·(ROW_H + ROW_GAP) … + ROW_H.
    fn hit_model(rows: usize) -> OverlayModel {
        OverlayModel {
            workspace: String::new(),
            rows: (0..rows)
                .map(|index| RowModel {
                    title: "W".into(),
                    icon: None,
                    selected: false,
                    handle: Some(Handle(index as u64 + 1)),
                    ..Default::default()
                })
                .collect(),
            width: 2 * PAD_X + ICON + ICON_GAP + TEXT_W + HINT_GAP + HINT_COLS * TEXT_W,
            height: 2 * PAD_Y + rows as u32 * ROW_H + (rows as u32 - 1) * ROW_GAP,
            viewport: visible_window(rows, 0, DEFAULT_ROWS, None),
            profile: StyleProfile::default(),
            horizontal: false,
            query: None,
            hovered: None,
            caption: String::new(),
        }
    }

    #[test]
    fn row_at_maps_each_row_band_to_its_index() {
        let model = hit_model(3);
        let x = model.width as f64 / 2.0;
        // Mid-band of every row: PAD_Y top inset, ROW_H bands, ROW_GAP
        // seams — 40/96/152 at the documented rhythm (PAD_Y 16 doubles
        // the row gap; the 11-07 layout pass).
        for (row, mid) in [(0usize, 40.0f64), (1, 96.0), (2, 152.0)] {
            assert_eq!(model.row_at(x, mid, 1), Some(row), "mid of row {row}");
        }
        // The band's own boundaries belong to it: first pixel and last.
        assert_eq!(model.row_at(x, PAD_Y as f64, 1), Some(0));
        assert_eq!(
            model.row_at(x, (PAD_Y + ROW_H - 1) as f64, 1),
            Some(0),
            "the last pixel of a band is still that row"
        );
    }

    #[test]
    fn row_at_returns_none_across_gap_seams() {
        let model = hit_model(3);
        let x = model.width as f64 / 2.0;
        // The seam between rows 0 and 1 is [64, 72); between 1 and 2
        // [120, 128). Selection bands never merge across a seam; neither
        // does a click land.
        for y in [64.0f64, 68.0, 71.0, 120.0] {
            assert_eq!(model.row_at(x, y, 1), None, "seam at y={y}");
        }
    }

    #[test]
    fn row_at_returns_none_in_card_padding_and_off_the_card() {
        let model = hit_model(3);
        let x = model.width as f64 / 2.0;
        // Card padding: above the first row, and below the last band's
        // end (the bottom mirrors the top).
        let last_end = (PAD_Y + 3 * ROW_H + 2 * ROW_GAP) as f64;
        assert_eq!(model.row_at(x, 5.0, 1), None);
        assert_eq!(model.row_at(x, last_end + 1.0, 1), None);
        assert_eq!(model.row_at(x, model.height as f64 - 1.0, 1), None);
        // Off the card entirely.
        assert_eq!(model.row_at(x, model.height as f64, 1), None);
        assert_eq!(model.row_at(x, -1.0, 1), None);
        assert_eq!(model.row_at(-0.5, 40.0, 1), None);
        assert_eq!(model.row_at(model.width as f64, 40.0, 1), None);
        // The last pixel columns are still the card.
        assert_eq!(
            model.row_at(model.width as f64 - 0.5, 40.0, 1),
            Some(0),
            "just inside the right edge still hits"
        );
    }

    #[test]
    fn row_at_agrees_between_scale_1_and_scale_2_device_points() {
        // Pointer events are surface-local logical; row_at takes device
        // pixels (logical × render scale). The same on-screen point must
        // name the same row at both scales — the HiDPI contract, pinned
        // here at the pure seam the hardware walk re-proves at 200%.
        let model = hit_model(3);
        let x = model.width as f64 / 2.0;
        let probes = [
            (x, 36.0),  // mid of row 0
            (x, 92.0),  // mid of row 1
            (x, 148.0), // mid of row 2
            (x, 64.0),  // a gap seam
            (x, 5.0),   // card padding
            (x, 190.0), // off the card
            (-1.0, 36.0),
        ];
        for (x, y) in probes {
            assert_eq!(
                model.row_at(x, y, 1),
                model.row_at(x * 2.0, y * 2.0, 2),
                "logical ({x}, {y}) must name one row at both scales"
            );
        }
    }

    #[test]
    fn selection_and_border_colors_are_the_documented_blends() {
        // GNOME's :selected formula — text at 20% over the card — pinned
        // exactly so the token cannot drift by accident.
        assert_eq!(COLOR_SELECTED_BG, 0xFA_48_4B_50);
        assert_eq!(COLOR_BORDER, 0xFA44464B);
    }

    #[test]
    fn the_close_zone_is_the_window_rows_icon_badge() {
        // `close-window` on the vertical card: the × sits on the row's
        // icon, top-right corner — the same badge the horizontal tiles
        // paint. The badge's own boundaries belong to it; the icon land
        // below and left of it activates, and the hint band is the
        // hint's alone.
        let model = hit_model(3);
        let center = |row: usize| {
            let (left, top, right, bottom) = close_row_zone(&model, row).expect("row");
            ((left + right) as f64 / 2.0, (top + bottom) as f64 / 2.0)
        };
        for row in 0..3 {
            let (x, y) = center(row);
            assert_eq!(
                model.close_zone_at(x, y, 1),
                Some(row),
                "row {row}'s × badge"
            );
        }
        // The badge's own edges: first pixel in, last pixel in.
        let (left, top, right, bottom) = close_row_zone(&model, 1).expect("row");
        assert_eq!(model.close_zone_at(left as f64, top as f64, 1), Some(1));
        assert_eq!(
            model.close_zone_at(right as f64 - 0.5, (bottom - 1) as f64, 1),
            Some(1)
        );
        // One pixel left of the badge and one below it: icon land.
        assert_eq!(model.close_zone_at(left as f64 - 0.5, top as f64, 1), None);
        assert_eq!(
            model.close_zone_at((left + right) as f64 / 2.0, bottom as f64, 1),
            None
        );
        // The hint band no longer closes: the pointer there activates.
        let hint_x = (model.width - PAD_X - HINT_COLS * TEXT_W) as f64 + 1.0;
        assert_eq!(
            model.close_zone_at(hint_x, top as f64 + 1.0, 1),
            None,
            "the hint band keeps its keyboard-only territory"
        );
    }

    #[test]
    fn the_close_zone_agrees_between_scale_1_and_scale_2_device_points() {
        // The HiDPI twin of row_at's contract: the same on-screen point
        // names one close zone at both scales.
        let model = hit_model(3);
        let (left, top, right, bottom) = close_row_zone(&model, 0).expect("row");
        let probes = [
            ((left + right) as f64 / 2.0, (top + bottom) as f64 / 2.0),
            (left as f64, top as f64),
            (right as f64 - 0.5, bottom as f64 - 0.5),
            (left as f64 - 0.5, top as f64),
            (left as f64 + 1.0, bottom as f64),
            (model.width as f64 / 2.0, 190.0), // off the card
        ];
        for (x, y) in probes {
            assert_eq!(
                model.close_zone_at(x, y, 1),
                model.close_zone_at(x * 2.0, y * 2.0, 2),
                "logical ({x}, {y}) must name one zone at both scales"
            );
        }
    }

    #[test]
    fn the_close_zone_refuses_headers_and_continuations() {
        // A group header closes nothing from its icon — "close an app"
        // is not this module — and a continuation affordance has no
        // window to close. Children are window rows like leaves.
        let mut model = hit_model(3);
        model.rows[0].kind = RowKind::Parent;
        model.rows[2].handle = None;
        let center = |model: &OverlayModel, row: usize| {
            let (left, top, right, bottom) = close_row_zone(model, row).expect("row");
            ((left + right) as f64 / 2.0, (top + bottom) as f64 / 2.0)
        };
        let (x, y) = center(&model, 0);
        assert_eq!(
            model.close_zone_at(x, y, 1),
            None,
            "a header never offers its icon's badge"
        );
        let (x, y) = center(&model, 1);
        assert_eq!(
            model.close_zone_at(x, y, 1),
            Some(1),
            "a child is a window row"
        );
        let (x, y) = center(&model, 2);
        assert_eq!(
            model.close_zone_at(x, y, 1),
            None,
            "a continuation row has no window"
        );
    }

    #[test]
    fn the_close_zone_is_the_horizontal_tiles_icon_badge() {
        // `close-window` on the band: the ×'s territory is the square
        // on the tile's icon at its top-right corner — the same badge
        // the vertical rows paint. The rest of the icon activates as
        // before, and the zone's own edges belong to it.
        let model = build_band(&band_snapshot(4), Handle(2), 4, None).expect("builds");
        assert!(model.horizontal);
        let zone = |tile: u32| close_tile_zone(TILE_PAD_X + tile * (TILE_W + TILE_GAP));
        let center = |tile: u32| {
            let (left, top, right, bottom) = zone(tile);
            ((left + right) / 2, (top + bottom) / 2)
        };
        for tile in 0..4u32 {
            let (x, y) = center(tile);
            assert_eq!(
                model.close_zone_at(x as f64, y as f64, 1),
                Some(tile as usize),
                "tile {tile}'s × zone"
            );
        }
        // The zone's own boundaries: first pixel in, last pixel in.
        let (left, top, right, bottom) = zone(1);
        assert_eq!(model.close_zone_at(left as f64, top as f64, 1), Some(1));
        assert_eq!(
            model.close_zone_at(right as f64 - 0.5, (bottom - 1) as f64, 1),
            Some(1)
        );
        // The icon land left of the zone, the tile's right margin, and
        // the caption gap above it all leave the tile to activate.
        assert_eq!(model.close_zone_at(left as f64 - 0.5, top as f64, 1), None);
        assert_eq!(model.close_zone_at(right as f64, top as f64, 1), None);
        assert_eq!(
            model.close_zone_at((left + right) as f64 / 2.0, top as f64 - 1.0, 1),
            None
        );
        // The tile's icon center is activation land.
        let icon_center = (
            (TILE_PAD_X + (TILE_W + TILE_GAP) + TILE_W / 2) as f64,
            (band_strip_top() + TILE_CELL_H / 2) as f64,
        );
        assert_eq!(model.close_zone_at(icon_center.0, icon_center.1, 1), None);
    }

    #[test]
    fn the_horizontal_close_zone_agrees_between_scale_1_and_scale_2_device_points() {
        // The HiDPI twin of the band's contract: the same on-screen
        // point names one close zone at both scales.
        let model = build_band(&band_snapshot(4), Handle(2), 4, None).expect("builds");
        let left = TILE_PAD_X + TILE_W + TILE_GAP;
        let (zone_left, zone_top, zone_right, zone_bottom) = close_tile_zone(left);
        let probes = [
            (
                ((zone_left + zone_right) / 2) as f64,
                ((zone_top + zone_bottom) / 2) as f64,
            ),
            (zone_left as f64, zone_top as f64),
            (zone_right as f64 - 0.5, zone_bottom as f64 - 0.5),
            (zone_left as f64 - 0.5, zone_top as f64),
            (TILE_PAD_X as f64, (band_strip_top() + TILE_CELL_H) as f64),
        ];
        for (x, y) in probes {
            assert_eq!(
                model.close_zone_at(x, y, 1),
                model.close_zone_at(x * 2.0, y * 2.0, 2),
                "logical ({x}, {y}) must name one zone at both scales"
            );
        }
    }

    #[test]
    fn the_horizontal_close_zone_refuses_app_tiles_and_continuations() {
        // An app tile closes its front window through Alt+minus but is
        // not a pointer target ("close an app" is not this module), and
        // the `…` tile has no window to close. Window tiles — the flat
        // band's leaves and the open run's children — are the zone.
        let mut model = build_band(&band_snapshot(4), Handle(2), 4, None).expect("builds");
        model.rows[0].kind = RowKind::Parent;
        model.rows[3].handle = None;
        let center = |tile: u32| {
            let (left, top, right, bottom) =
                close_tile_zone(TILE_PAD_X + tile * (TILE_W + TILE_GAP));
            ((left + right) / 2, (top + bottom) / 2)
        };
        let (x, y) = center(0);
        assert_eq!(
            model.close_zone_at(x as f64, y as f64, 1),
            None,
            "an app tile never offers its ×"
        );
        let (x, y) = center(1);
        assert_eq!(
            model.close_zone_at(x as f64, y as f64, 1),
            Some(1),
            "a window tile offers its ×"
        );
        let (x, y) = center(3);
        assert_eq!(
            model.close_zone_at(x as f64, y as f64, 1),
            None,
            "a continuation tile has no window"
        );
    }

    #[test]
    fn hover_presence_paints_the_close_badge_on_window_rows_only() {
        // Pointer presence is what reveals the ×: a hovered leaf's icon
        // gains the badge it did not carry without the hover, while a
        // header hovered the same way paints byte-identically to the
        // pointerless card — the reveal never leaks onto rows that must
        // not offer a close. The hint band stays put: the × lives in
        // the icon's corner now, never in the chord's territory.
        let palette = Palette::dark();
        let mut model = hit_model(3);
        let base = rendered(&model, 2);
        // The leaf reveal: row 1's icon gains the badge's ink.
        model.hovered = Some(1);
        let revealed = rendered(&model, 2);
        assert_ne!(revealed.0, base.0, "the hovered leaf paints the ×");
        let (left, top, right, bottom) = close_row_zone(&model, 1).expect("row");
        let mut inked = false;
        for x in left * 2..right * 2 {
            for y in top * 2..bottom * 2 {
                if pixel(&revealed.0, revealed.1, x, y) == palette.text {
                    inked = true;
                    break;
                }
            }
        }
        assert!(inked, "the × paints in the full text color");
        // The hovered row's chord is untouched — the hint no longer
        // yields to the close affordance.
        for (x, y) in hint_band_points(1, model.width) {
            assert_eq!(
                pixel(&revealed.0, revealed.1, x, y),
                pixel(&base.0, base.1, x, y),
                "row 1's hint band changed under its own hover"
            );
        }
        // Other rows' badges are untouched by row 1's hover.
        for row in [0usize, 2] {
            let (left, top, right, bottom) = close_row_zone(&model, row).expect("row");
            for x in left * 2..right * 2 {
                for y in top * 2..bottom * 2 {
                    assert_eq!(
                        pixel(&revealed.0, revealed.1, x, y),
                        pixel(&base.0, base.1, x, y),
                        "row {row}'s badge changed under row 1's hover"
                    );
                }
            }
        }
        // The refusal: a hovered header paints nothing over the
        // pointerless card, and neither does a hovered continuation.
        let mut header_model = hit_model(3);
        header_model.rows[0].kind = RowKind::Parent;
        let header_base = rendered(&header_model, 2);
        header_model.hovered = Some(0);
        assert_eq!(
            rendered(&header_model, 2).0,
            header_base.0,
            "a hovered header is byte-identical to no hover"
        );
        let mut dots_model = hit_model(3);
        dots_model.rows[2].handle = None;
        dots_model.rows[2].title = String::new();
        let dots_base = rendered(&dots_model, 2);
        dots_model.hovered = Some(2);
        assert_eq!(
            rendered(&dots_model, 2).0,
            dots_base.0,
            "a hovered continuation is byte-identical to no hover"
        );
    }

    #[test]
    fn hover_presence_paints_the_close_glyph_on_horizontal_window_tiles_only() {
        // The band's reveal twin: a hovered window tile gains the ×'s
        // ink in its top-right corner, every other tile is untouched,
        // and a hovered app tile or `…` tile paints byte-identically
        // to the pointerless card.
        let palette = Palette::dark();
        let mut model = build_band(&band_snapshot(4), Handle(2), 4, None).expect("builds");
        let base = rendered(&model, 2);
        model.hovered = Some(1);
        let revealed = rendered(&model, 2);
        assert_ne!(revealed.0, base.0, "the hovered tile paints the ×");
        let (left, top, right, bottom) = close_tile_zone(TILE_PAD_X + TILE_W + TILE_GAP);
        let mut inked = false;
        for x in left * 2..right * 2 {
            for y in top * 2..bottom * 2 {
                if pixel(&revealed.0, revealed.1, x, y) == palette.text {
                    inked = true;
                    break;
                }
            }
        }
        assert!(inked, "the × paints in the full text color");
        // Every other tile's cell is byte-identical under row 1's hover.
        for tile in [0u32, 2, 3] {
            let tile_left = TILE_PAD_X + tile * (TILE_W + TILE_GAP);
            for x in tile_left * 2..(tile_left + TILE_W) * 2 {
                for y in band_strip_top() * 2..(band_strip_top() + TILE_CELL_H) * 2 {
                    assert_eq!(
                        pixel(&revealed.0, revealed.1, x, y),
                        pixel(&base.0, base.1, x, y),
                        "tile {tile} changed under tile 1's hover"
                    );
                }
            }
        }
        // The refusal: a hovered app tile and a hovered continuation
        // paint nothing over the pointerless card.
        let mut app_model = build_band(&band_snapshot(4), Handle(2), 4, None).expect("builds");
        app_model.rows[0].kind = RowKind::Parent;
        let app_base = rendered(&app_model, 2);
        app_model.hovered = Some(0);
        assert_eq!(
            rendered(&app_model, 2).0,
            app_base.0,
            "a hovered app tile is byte-identical to no hover"
        );
        let mut dots_model = build_band(&band_snapshot(4), Handle(2), 4, None).expect("builds");
        dots_model.rows[3].handle = None;
        let dots_base = rendered(&dots_model, 2);
        dots_model.hovered = Some(3);
        assert_eq!(
            rendered(&dots_model, 2).0,
            dots_base.0,
            "a hovered continuation tile is byte-identical to no hover"
        );
    }

    #[test]
    fn light_selection_and_border_derive_by_the_blend_formula() {
        // The light card's band and hairline, hand-computed from the
        // documented 20%/22% formula over the warm-white card and
        // near-black text — the light twin of the pin above, so neither
        // variant can drift from the blend system.
        let light = Palette::light();
        assert_eq!(light.background, 0xFA_F7_F4_F0);
        assert_eq!(light.text, 0xFF_1E_21_26);
        assert_eq!(light.selected_bg, 0xFA_CB_C9_C7);
        assert_eq!(light.border, 0xFA_C7_C5_C3);
    }

    #[test]
    fn muted_derives_by_the_blend_formula_on_both_polarities() {
        // Hints: text at 55% over the card, hand-computed for both
        // polarities so the derivation cannot drift — the same pin the
        // selection and border carry. Dark: R 30→146, G 33→149, B 38→153
        // over (0x1E2126, 0xF2F4F8); light: R 247→127, G 244→127,
        // B 240→128 over (0xF7F4F0, 0x1E2126).
        assert_eq!(Palette::dark().muted, 0xFA_92_95_99);
        assert_eq!(Palette::light().muted, 0xFA_7F_7F_80);
        for palette in [Palette::dark(), Palette::light()] {
            assert_eq!(
                palette.muted,
                blend_over(palette.background, palette.text, 55),
                "muted must be the palette's own two chosen colors blended"
            );
        }
    }

    #[test]
    fn both_cards_stay_near_opaque() {
        // The compile-time assert, widened to every constructor: the
        // bleed-through must stay imperceptible whichever card paints.
        for palette in [Palette::dark(), Palette::light()] {
            assert!(palette.background >> 24 >= 0xFA);
            assert!(palette.selected_bg >> 24 >= 0xFA);
        }
    }

    #[test]
    fn resolve_honors_pinned_values_and_auto_follows_the_system() {
        // Pinned values ignore the system answer entirely; Auto follows
        // it, and an unreadable answer (None) paints dark — today's look.
        // No pair (the PA3 pin: no keys, byte-identical paint).
        let dark = Palette::dark();
        let light = Palette::light();
        for system in [Some(true), Some(false), None] {
            assert_eq!(Palette::resolve(ThemeSetting::Dark, system, None), dark);
            assert_eq!(Palette::resolve(ThemeSetting::Light, system, None), light);
        }
        assert_eq!(Palette::resolve(ThemeSetting::Auto, Some(true), None), dark);
        assert_eq!(
            Palette::resolve(ThemeSetting::Auto, Some(false), None),
            light
        );
        assert_eq!(
            Palette::resolve(ThemeSetting::Auto, None, None),
            dark,
            "auto + unreadable system paints today's card"
        );
    }

    #[test]
    fn custom_reproduces_the_builtins_from_their_own_colors() {
        // The built-ins are customs with the shipped colors: the same
        // two chosen colors in, byte-identical card out — proof the
        // custom path adds no derivation of its own.
        assert_eq!(
            Palette::custom(0x00_1E_21_26, 0x00_F2_F4_F8),
            Palette::dark()
        );
        assert_eq!(
            Palette::custom(0x00_F7_F4_F0, 0x00_1E_21_26),
            Palette::light()
        );
    }

    #[test]
    fn custom_derives_by_the_blend_system_per_polarity() {
        // 20/55 everywhere; the border rides one step off the band per
        // polarity — 18 before it on a dark card, 22 past it on a light
        // one (the built-ins' rule, mirrored).
        let dark = Palette::custom(0x00_2A_1E_30, 0x00_E8_E6_E0);
        assert_eq!(
            dark.background, 0xFA_2A_1E_30,
            "the card as given, alpha owned"
        );
        assert_eq!(dark.text, 0xFF_E8_E6_E0);
        assert!(dark.is_dark, "this card's luminance sits under 0.5");
        assert_eq!(dark.selected_bg, blend_over(dark.background, dark.text, 20));
        assert_eq!(dark.border, blend_over(dark.background, dark.text, 18));
        assert_eq!(dark.muted, blend_over(dark.background, dark.text, 55));

        let light = Palette::custom(0x00_E8_E6_E0, 0x00_2A_1E_30);
        assert!(!light.is_dark);
        assert_eq!(
            light.selected_bg,
            blend_over(light.background, light.text, 20)
        );
        assert_eq!(light.border, blend_over(light.background, light.text, 22));
        assert_eq!(light.muted, blend_over(light.background, light.text, 55));
    }

    #[test]
    fn custom_polarity_flips_at_the_luminance_threshold() {
        // The ratified 0.5 threshold: #BBBBBB sits just under (≈ 0.497),
        // #BCBCBC just over (≈ 0.503) — the icon tint follows the card
        // the user chose, not the system's.
        assert!(Palette::custom(0x00_BB_BB_BB, 0x00_00_00_00).is_dark);
        assert!(!Palette::custom(0x00_BC_BC_BC, 0x00_00_00_00).is_dark);
    }

    #[test]
    fn custom_stays_near_opaque_for_any_pair() {
        // The constructor owns the alpha bytes — background 0xFA, text
        // 0xFF — and blend_over keeps the card's, so every derived
        // color clears the near-opaque bar whatever the pair.
        for (card, text) in [
            (0x00_1E_21_26u32, 0x00_F2_F4_F8u32),
            (0x00_F7_F4_F0, 0x00_1E_21_26),
            (0x00_BB_BB_BB, 0x00_00_00_00),
            (0x00_FF_FF_FF, 0x00_76_76_76),
        ] {
            let palette = Palette::custom(card, text);
            assert_eq!(palette.background >> 24, 0xFA, "card {card:06x}");
            assert_eq!(palette.text >> 24, 0xFF);
            assert!(palette.selected_bg >> 24 >= 0xFA);
            assert!(palette.border >> 24 >= 0xFA);
            assert!(palette.muted >> 24 >= 0xFA);
        }
    }

    #[test]
    fn resolve_lets_a_present_pair_win_over_every_theme_setting() {
        // A present pair pins the card: the setting and the system
        // answer — present or not — change nothing.
        let pair = Some((0x00_2A_1E_30, 0x00_E8_E6_E0));
        let expected = Palette::custom(0x00_2A_1E_30, 0x00_E8_E6_E0);
        for setting in [ThemeSetting::Auto, ThemeSetting::Dark, ThemeSetting::Light] {
            for system in [Some(true), Some(false), None] {
                assert_eq!(Palette::resolve(setting, system, pair), expected);
            }
        }
    }

    #[test]
    fn a_custom_palette_renders_its_own_band_and_card() {
        // The render pin: the selected row's band is exactly this
        // card's own text@20 blend, the gaps are this card as given —
        // nothing in the renderer reaches for a built-in color.
        let mut model = hit_model(3);
        model.rows[1].selected = true;
        let palette = Palette::custom(0x00_2A_1E_30, 0x00_E8_E6_E0);
        let (w, h) = (model.width * 2, model.height * 2);
        let stride = w * 4;
        let mut canvas = vec![0u8; (stride * h) as usize];
        render(
            &model,
            &mut canvas,
            w,
            h,
            stride,
            2,
            &palette,
            TextBackend::Bitmap,
        );
        assert_eq!(
            palette.selected_bg,
            blend_over(palette.background, palette.text, 20)
        );
        let band_top = (PAD_Y + ROW_H + ROW_GAP) * 2;
        let band_x = SELECT_INSET * 2;
        assert_eq!(
            pixel(&canvas, stride, band_x + BAND_RADIUS * 2, band_top),
            palette.selected_bg,
            "the band is text@20 over this card, byte for byte"
        );
        let gap_y = (PAD_Y + ROW_H + ROW_GAP / 2) * 2;
        assert_eq!(
            pixel(&canvas, stride, w / 2, gap_y),
            palette.background,
            "the card fill is the card as given"
        );
    }

    #[test]
    fn neutral_selection_reproduces_the_shipped_band_on_every_card() {
        // The escape hatch, byte-exact: `selection = "neutral"` paints
        // the band every card shipped before the follow — even when an
        // accent was available to borrow.
        let accent = Some(0x63_D0_DF);
        for card in [
            Palette::dark(),
            Palette::light(),
            Palette::custom(0x00_2A_1E_30, 0x00_E8_E6_E0),
        ] {
            let band = card
                .with_selection(SelectionSetting::Neutral, accent)
                .selected_bg;
            assert_eq!(
                band, card.selected_bg,
                "neutral keeps the constructor's own band on every card"
            );
        }
    }

    #[test]
    fn the_band_blends_the_followed_accent_over_every_card() {
        // The record's fixture teal at 20 % over each card, pinned as
        // u32s: dark 0xFA2B444B, light 0xFAD9ECEC, custom 0xFA354153.
        let teal = Some(0x63_D0_DF);
        let dark = Palette::dark().with_selection(SelectionSetting::Accent, teal);
        assert_eq!(dark.selected_bg, 0xFA_2B_44_4B);
        let light = Palette::light().with_selection(SelectionSetting::Accent, teal);
        assert_eq!(light.selected_bg, 0xFA_D9_EC_EC);
        let custom = Palette::custom(0x00_2A_1E_30, 0x00_E8_E6_E0)
            .with_selection(SelectionSetting::Accent, teal);
        assert_eq!(custom.selected_bg, 0xFA_35_41_53);
        for palette in [dark, light, custom] {
            assert_eq!(
                palette.selected_bg,
                blend_over(palette.background, 0x00_63_D0_DF, 20),
                "the band is the accent's base at 20 % over this card"
            );
        }
    }

    #[test]
    fn a_ladder_miss_is_the_neutral_lift() {
        // The band is never absent: a store that misses everywhere
        // leaves the constructor's text-at-20 % band in place.
        for card in [
            Palette::dark(),
            Palette::custom(0x00_2A_1E_30, 0x00_E8_E6_E0),
        ] {
            let miss = card.with_selection(SelectionSetting::Accent, None);
            assert_eq!(miss.selected_bg, card.selected_bg);
        }
    }

    #[test]
    fn followed_alpha_never_leaks_into_the_band() {
        // SC 10: even a source color carrying its own alpha byte blends
        // opaquely — blend_over keeps the card's alpha, and the accent
        // arrives straight anyway.
        let card = Palette::dark();
        let opaque = card.with_selection(SelectionSetting::Accent, Some(0x63_D0_DF));
        let leaky = card.with_selection(SelectionSetting::Accent, Some(0x63_D0_DF | 0x1100_0000));
        assert_eq!(opaque.selected_bg, leaky.selected_bg);
        assert_eq!(
            leaky.selected_bg >> 24,
            0xFA,
            "the card's near-opaque byte survives"
        );
    }

    #[test]
    fn the_bands_accent_side_follows_the_cards_polarity() {
        // The daemon's composition pinned: the accent read for the band
        // is `accent_base(card.is_dark)` — the side matching the card
        // the keys just chose. Auto theme + system flip flips the card
        // and with it the accent side (the store record's live pair:
        // purple on dark, dark-teal on light).
        for (system, side) in [(Some(true), 0xE7_9C_FE), (Some(false), 0x00_52_5A)] {
            let card = Palette::resolve(ThemeSetting::Auto, system, None);
            let band = card.with_selection(SelectionSetting::Accent, Some(side));
            assert_eq!(
                band.selected_bg,
                blend_over(card.background, side, 20),
                "the followed side rides the flipped card"
            );
        }
    }

    #[test]
    fn contrast_formulas_reproduce_the_records_numbers() {
        // The WCAG 2.x formulas, checked against appearance-settings'
        // shipped-ratio numbers: dark title 14.66:1, light title
        // 14.73:1 exactly. The record's light muted 3.60:1 was measured
        // on the blend before integer flooring (0x808081 → 3.599); the
        // shipped floored muted (0x7F7F80) measures 3.648 — the same
        // derivation, one rounding apart, and below AA either way (the
        // note that keeps the muted derivation out of the floor's
        // jurisdiction).
        let dark = Palette::dark();
        let light = Palette::light();
        assert!((contrast_ratio(dark.text, dark.background) - 14.66).abs() < 0.01);
        assert!((contrast_ratio(light.text, light.background) - 14.73).abs() < 0.01);
        assert!(
            (contrast_ratio(light.muted, light.background) - 3.65).abs() < 0.01,
            "the record's 3.60 is the pre-flooring blend; the shipped bytes are 3.648"
        );
    }

    #[test]
    fn relative_luminance_hits_the_wcag_anchors() {
        // White is 1, black is 0, the ratio's full span is 21:1 — the
        // formula's own anchor points.
        assert!((relative_luminance(0xFFFF_FFFF) - 1.0).abs() < 1e-6);
        assert!(relative_luminance(0xFF00_0000) < 1e-6);
        assert!(
            (relative_luminance(0xFF_FF_00_00) - 0.2126).abs() < 1e-3,
            "pure red is its own weight, 0.2126"
        );
        assert!((contrast_ratio(0xFFFF_FFFF, 0xFF00_0000) - 21.0).abs() < 1e-3);
    }

    #[test]
    fn coverage_argb_is_identity_at_full_and_scaled_below() {
        assert_eq!(coverage_argb(COLOR_BACKGROUND, 255), COLOR_BACKGROUND);
        let half = coverage_argb(COLOR_TEXT, 128);
        assert_eq!(half >> 24, 128);
        // Each channel scales by the coverage, rounding half-up.
        assert_eq!((half >> 16) & 0xFF, (0xF2 * 128 + 127) / 255);
    }

    #[test]
    fn shape_cov_is_exact_on_straight_edges() {
        // A 100×60 shape: pixels along the middle of each edge are fully
        // inside; a pixel just past the right edge is fully outside.
        assert_eq!(shape_cov(50, 0, 100, 60, RADIUS, 0), 255);
        assert_eq!(shape_cov(0, 30, 100, 60, RADIUS, 0), 255);
        assert_eq!(shape_cov(100, 30, 100, 60, RADIUS, 0), 0);
        assert_eq!(shape_cov(50, 60, 100, 60, RADIUS, 0), 0);
        // The eroded shape starts one pixel in.
        assert_eq!(shape_cov(0, 30, 100, 60, RADIUS, BORDER), 0);
        assert_eq!(shape_cov(BORDER, 30, 100, 60, RADIUS, BORDER), 255);
    }

    #[test]
    fn shape_cov_rounds_corners_by_area() {
        // Corner center at (12, 12), radius 12: the exact corner pixel
        // center is 16.3 px away — no subsample inside, zero coverage.
        assert_eq!(shape_cov(0, 0, 100, 60, RADIUS, 0), 0);
        assert_eq!(shape_cov(1, 1, 100, 60, RADIUS, 0), 0);
        // A pixel straddling the arc carries partial coverage, and the
        // interior beyond the arc is full.
        let straddle = shape_cov(3, 3, 100, 60, RADIUS, 0);
        assert!(
            straddle > 0 && straddle < 255,
            "expected partial, got {straddle}"
        );
        assert_eq!(shape_cov(RADIUS + 2, RADIUS + 2, 100, 60, RADIUS, 0), 255);
    }

    #[test]
    fn render_draws_the_fallback_glyph_left_of_the_title() {
        // Two rows so the inspected row is not the selected one: the
        // selected row's full-width highlight would defeat pixel probing.
        let snap = snapshot(
            vec![
                toplevel(1, "A", "app.a", &[1], switcher::State::ACTIVATED, 2),
                toplevel(2, "B", "app.b", &[1], switcher::State::default(), 1),
            ],
            Some(1),
        );
        let model =
            build(&snap, Handle(2), Scope::CurrentWorkspace).expect("eligible windows exist");
        let (canvas, stride) = rendered(&model, 2);
        let scale = 2u32;

        let drawn = |x: u32, y: u32| pixel(&canvas, stride, x, y) != COLOR_BACKGROUND;
        // The unselected first row's band, in device pixels (row two is
        // the selection in this model; its band would defeat probing).
        let band = PAD_Y * scale..(PAD_Y + ROW_H) * scale;
        let glyph_x = (PAD_X + (ICON - GLYPH_W * GLYPH_SCALE) / 2) * scale;
        let title_x = (PAD_X + ICON + ICON_GAP) * scale;

        // The fallback glyph owns the leftmost drawn pixels of the row.
        let leftmost = band
            .clone()
            .flat_map(|y| {
                // Skip the border columns: the hairline is drawn left of
                // any row content by design.
                ((BORDER + 1) * scale..model.width * scale).filter(move |&x| drawn(x, y))
            })
            .min();
        assert_eq!(leftmost, Some(glyph_x));

        // Nothing is drawn between the glyph and the title.
        for y in band {
            for x in glyph_x + GLYPH_W * GLYPH_SCALE * scale..title_x {
                assert!(!drawn(x, y), "gap pixel drawn at ({x}, {y})");
            }
        }
    }

    #[test]
    fn render_blits_the_icon_bitmap_into_the_cell() {
        let solid = 0xFF_11_22_33;
        let icon = IconBitmap {
            side: ICON * 2,
            argb: vec![solid; (ICON * 2 * ICON * 2) as usize],
        };
        let model = OverlayModel {
            workspace: String::new(),
            rows: vec![RowModel {
                title: "Titled".into(),
                icon: Some(Rc::new(icon)),
                selected: false,
                handle: Some(Handle(1)),
                ..Default::default()
            }],
            width: 2 * PAD_X + ICON + ICON_GAP + 6 * TEXT_W + HINT_GAP + HINT_COLS * TEXT_W,
            height: 2 * PAD_Y + ROW_H,
            viewport: visible_window(1, 0, DEFAULT_ROWS, None),
            profile: StyleProfile::default(),
            horizontal: false,
            query: None,
            hovered: None,
            caption: String::new(),
        };
        let (canvas, stride) = rendered(&model, 2);

        // The icon cell is the solid bitmap, centered in the row.
        let cell_top = PAD_Y + ROW_PAD_Y;
        assert_eq!(
            pixel(&canvas, stride, PAD_X * 2 + 4, (cell_top + 4) * 2),
            solid
        );
        // A pixel just left of the cell is card background.
        assert_eq!(
            pixel(&canvas, stride, (PAD_X - 1) * 2, (cell_top + 4) * 2),
            COLOR_BACKGROUND
        );
    }

    #[test]
    fn render_rejects_a_mismatched_icon_side_and_draws_the_glyph() {
        // A bitmap prepared at the wrong scale must not blit misaligned:
        // the row falls back to the generic glyph.
        let bogus = IconBitmap {
            side: 8,
            argb: vec![0xFF_FF_00_FF; 64],
        };
        let model = OverlayModel {
            workspace: String::new(),
            rows: vec![RowModel {
                title: String::new(),
                icon: Some(Rc::new(bogus)),
                selected: false,
                handle: Some(Handle(1)),
                ..Default::default()
            }],
            width: 2 * PAD_X + ICON + ICON_GAP + TEXT_W + HINT_GAP + HINT_COLS * TEXT_W,
            height: 2 * PAD_Y + ROW_H,
            viewport: visible_window(1, 0, DEFAULT_ROWS, None),
            profile: StyleProfile::default(),
            horizontal: false,
            query: None,
            hovered: None,
            caption: String::new(),
        };
        let (canvas, stride) = rendered(&model, 2);
        let glyph_x = (PAD_X + (ICON - GLYPH_W * GLYPH_SCALE) / 2) * 2;
        let glyph_y = (PAD_Y + (ROW_H - ICON) / 2 + (ICON - GLYPH_W * GLYPH_SCALE) / 2) * 2;
        // The window glyph's first scanline is fully lit.
        assert_eq!(pixel(&canvas, stride, glyph_x, glyph_y), COLOR_TEXT);
    }

    #[test]
    fn render_draws_glyph_pixels_at_the_text_origin() {
        // 'A' (row 0 bits 2 and 3) proves the text origin — the column
        // after the icon — at the doubled font scale. The selection is on
        // the other row so this row shows plain background around glyphs.
        let snap = snapshot(
            vec![
                toplevel(1, "A", "app.a", &[1], switcher::State::ACTIVATED, 2),
                toplevel(2, "B", "app.b", &[1], switcher::State::default(), 1),
            ],
            Some(1),
        );
        let model =
            build(&snap, Handle(2), Scope::CurrentWorkspace).expect("eligible windows exist");
        let (canvas, stride) = rendered(&model, 2);

        let text_x = (PAD_X + ICON + ICON_GAP) * 2;
        let text_y = (PAD_Y + (ROW_H - TEXT_H) / 2) * 2;
        // Empty cell, then 'A' pixels at font columns 2 and 3 (each a
        // 3×3 device block: text cell × scale ÷ glyph cell at the
        // scale-2 floor). Font column c lands c blocks in; 'A' row 0 is
        // bits 2 and 3.
        let block = TEXT_W * 2 / GLYPH_W;
        assert_eq!(pixel(&canvas, stride, text_x, text_y), COLOR_BACKGROUND);
        assert_eq!(
            pixel(&canvas, stride, text_x + 2 * block, text_y),
            COLOR_TEXT
        );
        assert_eq!(
            pixel(&canvas, stride, text_x + 3 * block, text_y),
            COLOR_TEXT
        );
        assert_eq!(
            pixel(&canvas, stride, text_x + 7 * block, text_y),
            COLOR_BACKGROUND
        );
    }

    #[test]
    fn render_highlights_the_selected_row_band() {
        let snap = snapshot(
            vec![
                toplevel(1, "Top", "app.a", &[1], switcher::State::ACTIVATED, 2),
                toplevel(2, "Bottom", "app.b", &[1], switcher::State::default(), 1),
            ],
            Some(1),
        );
        let model =
            build(&snap, Handle(2), Scope::CurrentWorkspace).expect("eligible windows exist");
        let (canvas, stride) = rendered(&model, 2);

        let row0_y = (PAD_Y + ROW_H / 2) * 2;
        let row1_y = (PAD_Y + ROW_H + ROW_GAP + ROW_H / 2) * 2;
        // Mid-card x (clear of the border columns): plain background for
        // row 0, selection background for row 1.
        // Probe left of the icon cell — clear of titles at any width.
        let inside_x = 8 * 2;
        assert_eq!(pixel(&canvas, stride, inside_x, row0_y), COLOR_BACKGROUND);
        assert_eq!(pixel(&canvas, stride, inside_x, row1_y), COLOR_SELECTED_BG);
        // Top padding is plain background above both rows.
        assert_eq!(
            pixel(&canvas, stride, model.width, (PAD_Y / 2) * 2),
            COLOR_BACKGROUND
        );
    }

    #[test]
    fn render_selection_band_spans_the_row_and_never_merges() {
        // Three rows, the middle selected: the band covers the whole row
        // rect (icon cell and all), stops at the row gap, and clears the
        // neighboring rows entirely.
        let snap = snapshot(
            vec![
                toplevel(1, "Top", "app.a", &[1], switcher::State::ACTIVATED, 3),
                toplevel(2, "Middle", "app.b", &[1], switcher::State::default(), 2),
                toplevel(3, "Bottom", "app.c", &[1], switcher::State::default(), 1),
            ],
            Some(1),
        );
        let model =
            build(&snap, Handle(2), Scope::CurrentWorkspace).expect("eligible windows exist");
        let (canvas, stride) = rendered(&model, 2);

        let band_top = (PAD_Y + ROW_H + ROW_GAP) * 2;
        let band_bottom = band_top + ROW_H * 2;
        let mid_x = model.width;
        assert_eq!(pixel(&canvas, stride, mid_x, band_top), COLOR_SELECTED_BG);
        assert_eq!(
            pixel(&canvas, stride, mid_x, band_bottom - 1),
            COLOR_SELECTED_BG
        );
        // The gap rows between bands are plain background.
        assert_eq!(
            pixel(&canvas, stride, mid_x, band_top - 2),
            COLOR_BACKGROUND
        );
        assert_eq!(
            pixel(&canvas, stride, mid_x, band_bottom + 1),
            COLOR_BACKGROUND
        );
    }

    #[test]
    fn render_draws_the_hairline_border_above_everything() {
        let snap = snapshot(
            vec![toplevel(
                1,
                "Solo",
                "app.a",
                &[1],
                switcher::State::ACTIVATED,
                1,
            )],
            Some(1),
        );
        let model =
            build(&snap, Handle(1), Scope::CurrentWorkspace).expect("eligible windows exist");
        let (canvas, stride) = rendered(&model, 2);

        let (w, h) = (model.width * 2, model.height * 2);
        // Mid-edge pixels are the border, even over the selected row's
        // band; one logical pixel in, the fill owns the pixel again.
        assert_eq!(pixel(&canvas, stride, w / 2, 0), COLOR_BORDER);
        assert_eq!(pixel(&canvas, stride, w / 2, 2), COLOR_BACKGROUND);
        assert_eq!(pixel(&canvas, stride, 0, h / 2), COLOR_BORDER);
        // The band starts one SELECT_INSET in; the strip between border
        // and band stays card background.
        assert_eq!(
            pixel(&canvas, stride, (SELECT_INSET - 1) * 2, h / 2),
            COLOR_BACKGROUND
        );
        assert_eq!(
            pixel(&canvas, stride, SELECT_INSET * 2, h / 2),
            COLOR_SELECTED_BG
        );
        assert_eq!(pixel(&canvas, stride, w - 1, h / 2), COLOR_BORDER);
        // The exact corner pixel is partially covered — mostly outside
        // the rounded shape.
        let corner = pixel(&canvas, stride, w - 1, h - 1);
        assert!(
            (corner >> 24) < (COLOR_BORDER >> 24),
            "corner pixel should be partially covered, got {corner:#010x}"
        );
    }

    #[test]
    fn render_rounds_the_corners_with_partial_coverage() {
        let snap = snapshot(
            vec![toplevel(
                1,
                "Solo",
                "app.a",
                &[1],
                switcher::State::ACTIVATED,
                1,
            )],
            Some(1),
        );
        let model =
            build(&snap, Handle(1), Scope::CurrentWorkspace).expect("eligible windows exist");
        let (canvas, stride) = rendered(&model, 2);

        // The exact corner is partially covered: its alpha sits below
        // the card's, above nothing.
        let corner = pixel(&canvas, stride, 0, 0);
        assert!(
            (corner >> 24) < (COLOR_BACKGROUND >> 24),
            "corner pixel should be partially covered, got {corner:#010x}"
        );
        // Down the left edge past the arc, the pixel is fully the border.
        assert_eq!(
            pixel(&canvas, stride, 0, (RADIUS * 2 + 2) * 2),
            COLOR_BORDER
        );
    }

    #[test]
    fn render_paints_interior_pixels_with_known_colors_only() {
        let snap = snapshot(
            vec![
                toplevel(1, "Alpha", "app.a", &[1], switcher::State::ACTIVATED, 2),
                toplevel(2, "Beta", "app.b", &[1], switcher::State::default(), 1),
            ],
            Some(1),
        );
        let model =
            build(&snap, Handle(2), Scope::CurrentWorkspace).expect("eligible windows exist");
        let (canvas, stride) = rendered(&model, 2);
        let (w, h) = (model.width * 2, model.height * 2);
        // Interior: clear of the border ring and the corner arcs.
        for y in 2 * RADIUS..h - 2 * RADIUS {
            for x in 2 * RADIUS..w - 2 * RADIUS {
                let p = pixel(&canvas, stride, x, y);
                assert!(
                    p == COLOR_BACKGROUND
                        || p == COLOR_SELECTED_BG
                        || p == COLOR_TEXT
                        || p == Palette::dark().muted,
                    "interior pixel {p:#010x} at ({x}, {y}) is not a known color"
                );
            }
        }
    }

    #[test]
    fn render_respects_a_larger_stride() {
        let snap = snapshot(
            vec![toplevel(
                1,
                "A",
                "app.a",
                &[1],
                switcher::State::ACTIVATED,
                1,
            )],
            Some(1),
        );
        let model =
            build(&snap, Handle(1), Scope::CurrentWorkspace).expect("eligible windows exist");
        let (w, h) = (model.width * 2, model.height * 2);
        let stride = w * 4 + 64; // padding between rows, like a shared pool
        let mut canvas = vec![0x11; (stride * h) as usize];
        render(
            &model,
            &mut canvas,
            w,
            h,
            stride,
            2,
            &Palette::dark(),
            TextBackend::Bitmap,
        );
        // The 'A' pixels land at the text origin — font column 2 — not
        // at the row start.
        let block = TEXT_W * 2 / GLYPH_W;
        let text_x = (PAD_X + ICON + ICON_GAP) * 2 + 2 * block;
        let text_y = (PAD_Y + (ROW_H - TEXT_H) / 2) * 2;
        assert_eq!(pixel(&canvas, stride, text_x, text_y), COLOR_TEXT);
    }

    #[test]
    fn render_selection_band_keeps_uniform_row_rhythm() {
        // A polish review read the sheet as "the list stretches around the
        // selection". The renderer must prove otherwise: the band is
        // exactly one row tall, and the background corridors between
        // bands are exactly one row gap — measured on rendered pixels,
        // at every x inside the card interior, for selections at the
        // top, middle, and bottom of a five-row list.
        for selected in [0usize, 2, 4] {
            let model = OverlayModel {
                workspace: String::new(),
                rows: (0..5)
                    .map(|index| RowModel {
                        title: "W".repeat(8),
                        icon: None,
                        selected: index == selected,
                        handle: Some(Handle(index as u64 + 1)),
                        ..Default::default()
                    })
                    .collect(),
                width: 2 * PAD_X + ICON + ICON_GAP + 8 * TEXT_W + HINT_GAP + HINT_COLS * TEXT_W,
                height: 2 * PAD_Y + 5 * ROW_H + 4 * ROW_GAP,
                viewport: visible_window(5, 0, DEFAULT_ROWS, None),
                profile: StyleProfile::default(),
                horizontal: false,
                query: None,
                hovered: None,
                caption: String::new(),
            };
            let (canvas, stride) = rendered(&model, 2);
            // A probe column inside the band's horizontal extent, in the
            // empty corridor between the icon cell and the title text —
            // clear of glyphs at every row.
            let probe_x = (PAD_X + ICON + ICON_GAP / 2) * 2;
            let first_row_top = PAD_Y * 2;
            for row in 0..5 {
                let top = first_row_top + row as u32 * (ROW_H + ROW_GAP) * 2;
                let band = (top..top + ROW_H * 2)
                    .filter(|&y| pixel(&canvas, stride, probe_x, y) == COLOR_SELECTED_BG)
                    .count();
                let expected = if row == selected {
                    (ROW_H * 2) as usize
                } else {
                    0
                };
                assert_eq!(
                    band, expected,
                    "row {row} with selection at {selected}: {band} band rows, expected {expected}"
                );
            }
            // The corridors between rows are exactly ROW_GAP device rows
            // of plain background at the probe column.
            for row in 0..4 {
                let corridor_top =
                    first_row_top + (row as u32 + 1) * (ROW_H + ROW_GAP) * 2 - ROW_GAP * 2;
                let corridor = (corridor_top..corridor_top + ROW_GAP * 2)
                    .map(|y| pixel(&canvas, stride, probe_x, y))
                    .collect::<Vec<u32>>();
                assert!(
                    corridor.iter().all(|p| *p == COLOR_BACKGROUND),
                    "row gap after {row} is not uniform background: {corridor:?}"
                );
            }
        }
    }

    #[test]
    fn render_paints_hints_on_rows_one_through_nine_only() {
        // Twelve rows: the first nine carry their chord in the band;
        // rows ten and deeper stay silent, and the silence is accurate —
        // nine rows are addressable, nine are hinted.
        let model = OverlayModel {
            workspace: String::new(),
            rows: (0..12)
                .map(|index| RowModel {
                    title: format!("Window {index}"),
                    icon: None,
                    selected: false,
                    handle: Some(Handle(index as u64 + 1)),
                    hint: RowHint::ctrl(index),
                    ..Default::default()
                })
                .collect(),
            width: 2 * PAD_X + ICON + ICON_GAP + 10 * TEXT_W + HINT_GAP + HINT_COLS * TEXT_W,
            height: 2 * PAD_Y + 12 * ROW_H + 11 * ROW_GAP,
            // Hand-built past the cap on purpose (hint-silence rows past
            // nine); render is row-count agnostic and never reads the viewport.
            viewport: visible_window(12, 0, DEFAULT_ROWS, None),
            profile: StyleProfile::default(),
            horizontal: false,
            query: None,
            hovered: None,
            caption: String::new(),
        };
        let (canvas, stride) = rendered(&model, 2);
        let muted = Palette::dark().muted;
        for row in 0..9 {
            let hinted = hint_band_points(row, model.width)
                .any(|(x, y)| pixel(&canvas, stride, x, y) == muted);
            assert!(hinted, "row {row} shows no hint");
        }
        for row in 9..12 {
            let silent = hint_band_points(row, model.width)
                .all(|(x, y)| pixel(&canvas, stride, x, y) == COLOR_BACKGROUND);
            assert!(silent, "row {row} paints in the band beyond the wired nine");
        }
    }

    #[test]
    fn hints_share_the_titles_vertical_centering() {
        // The chord rides the row's own text line: the same centering
        // math as the title, never lower or higher.
        let model = OverlayModel {
            workspace: String::new(),
            rows: vec![RowModel {
                title: "Solo".into(),
                icon: None,
                selected: false,
                handle: Some(Handle(1)),
                ..Default::default()
            }],
            width: 2 * PAD_X + ICON + ICON_GAP + 4 * TEXT_W + HINT_GAP + HINT_COLS * TEXT_W,
            height: 2 * PAD_Y + ROW_H,
            viewport: visible_window(1, 0, DEFAULT_ROWS, None),
            profile: StyleProfile::default(),
            horizontal: false,
            query: None,
            hovered: None,
            caption: String::new(),
        };
        let (canvas, stride) = rendered(&model, 2);
        let muted = Palette::dark().muted;
        let text_top = (PAD_Y + (ROW_H - TEXT_H) / 2) * 2;
        for (x, y) in hint_band_points(0, model.width) {
            let p = pixel(&canvas, stride, x, y);
            if y < text_top || y >= text_top + TEXT_H * 2 {
                assert_eq!(
                    p, COLOR_BACKGROUND,
                    "hint pixel outside the text line at ({x}, {y})"
                );
            } else if p != COLOR_BACKGROUND {
                assert_eq!(p, muted, "a non-fill band pixel must be the hint");
            }
        }
    }

    #[test]
    fn the_selected_rows_hint_stays_muted() {
        // The accessory rule: the selection colors the row's fill, never
        // its hint — over the band the chord reads in the same muted
        // color as every unselected row's.
        let model = OverlayModel {
            workspace: String::new(),
            rows: vec![
                RowModel {
                    title: "Top".into(),
                    icon: None,
                    selected: true,
                    handle: Some(Handle(1)),
                    hint: Some(RowHint::Ctrl(1)),
                    ..Default::default()
                },
                RowModel {
                    title: "Bottom".into(),
                    icon: None,
                    selected: false,
                    handle: Some(Handle(2)),
                    hint: Some(RowHint::Ctrl(2)),
                    ..Default::default()
                },
            ],
            width: 2 * PAD_X + ICON + ICON_GAP + 6 * TEXT_W + HINT_GAP + HINT_COLS * TEXT_W,
            height: 2 * PAD_Y + 2 * ROW_H + ROW_GAP,
            viewport: visible_window(2, 0, DEFAULT_ROWS, None),
            profile: StyleProfile::default(),
            horizontal: false,
            query: None,
            hovered: None,
            caption: String::new(),
        };
        let (canvas, stride) = rendered(&model, 2);
        let muted = Palette::dark().muted;
        let selected_band: Vec<u32> = hint_band_points(0, model.width)
            .map(|(x, y)| pixel(&canvas, stride, x, y))
            .collect();
        assert!(
            selected_band.contains(&muted),
            "the selected row's hint is missing"
        );
        assert!(
            selected_band
                .iter()
                .all(|&p| p == muted || p == COLOR_SELECTED_BG),
            "the selected row's band paints something besides its hint and fill"
        );
    }

    #[test]
    fn render_keeps_a_max_cols_title_a_full_gap_clear_of_its_hint() {
        // Defense in depth, pixel-proved: the widest title a card can
        // carry still ends the full HINT_GAP before its hint's first
        // glyph cell — the strip between them is pure card fill, and the
        // moved clip guard cost the title nothing.
        let model = OverlayModel {
            workspace: String::new(),
            rows: vec![RowModel {
                title: "W".repeat(MAX_COLS as usize),
                icon: None,
                selected: false,
                handle: Some(Handle(1)),
                hint: Some(RowHint::Ctrl(1)),
                ..Default::default()
            }],
            width: 2 * PAD_X + ICON + ICON_GAP + MAX_COLS * TEXT_W + HINT_GAP + HINT_COLS * TEXT_W,
            height: 2 * PAD_Y + ROW_H,
            viewport: visible_window(1, 0, DEFAULT_ROWS, None),
            profile: StyleProfile::default(),
            horizontal: false,
            query: None,
            hovered: None,
            caption: String::new(),
        };
        let (canvas, stride) = rendered(&model, 2);

        let title_end = (PAD_X + ICON + ICON_GAP + MAX_COLS * TEXT_W) * 2;
        let hint_start = (model.width - PAD_X - HINT_COLS * TEXT_W) * 2;
        assert_eq!(hint_start - title_end, HINT_GAP * 2);
        for y in PAD_Y * 2..(PAD_Y + ROW_H) * 2 {
            for x in title_end..hint_start {
                assert_eq!(
                    pixel(&canvas, stride, x, y),
                    COLOR_BACKGROUND,
                    "the title/hint gap is not pure fill at ({x}, {y})"
                );
            }
        }
        // The last title glyph still paints (font column 6 of 'W' is
        // lit) — the guard clips at the title band's edge, not earlier.
        let last_cell_x = (PAD_X + ICON + ICON_GAP + (MAX_COLS - 1) * TEXT_W) * 2;
        let text_top = (PAD_Y + (ROW_H - TEXT_H) / 2) * 2;
        let block = TEXT_W * 2 / GLYPH_W;
        assert_eq!(
            pixel(&canvas, stride, last_cell_x + 6 * block, text_top),
            COLOR_TEXT
        );
        // And the hint paints beside it.
        let muted = Palette::dark().muted;
        assert!(
            hint_band_points(0, model.width).any(|(x, y)| pixel(&canvas, stride, x, y) == muted),
            "no hint beside the max-cols title"
        );
    }

    #[test]
    fn render_ten_rows_completes_quickly() {
        // Generous-bound guard for the "imperceptible render" requirement:
        // catches pathological regressions, not millisecond drift. At
        // scale 2 the canvas is 4× the pixels of the pre-refresh card.
        let model = OverlayModel {
            workspace: String::new(),
            rows: (0..10)
                .map(|i| RowModel {
                    title: "W".repeat(MAX_COLS as usize),
                    icon: None,
                    selected: i == 0,
                    handle: Some(Handle(i as u64 + 1)),
                    ..Default::default()
                })
                .collect(),
            width: 2 * PAD_X + ICON + ICON_GAP + MAX_COLS * TEXT_W + HINT_GAP + HINT_COLS * TEXT_W,
            height: 2 * PAD_Y + 10 * ROW_H + 9 * ROW_GAP,
            // Hand-built past the cap on purpose (render work, not a real
            // card shape); render never reads the viewport.
            viewport: visible_window(10, 0, DEFAULT_ROWS, None),
            profile: StyleProfile::default(),
            horizontal: false,
            query: None,
            hovered: None,
            caption: String::new(),
        };
        let (w, h) = (model.width * 2, model.height * 2);
        let stride = w * 4;
        let mut canvas = vec![0; (stride * h) as usize];
        let start = Instant::now();
        // 30 iterations at 4× the pre-refresh pixel count ≈ twice the
        // old guard's total work (200 renders of the 8-px card).
        for _ in 0..30 {
            render(
                &model,
                &mut canvas,
                w,
                h,
                stride,
                2,
                &Palette::dark(),
                TextBackend::Bitmap,
            );
        }
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_secs(2),
            "30 renders of a 10-row overlay took {elapsed:?}"
        );
    }

    #[test]
    fn glyph_maps_non_ascii_to_the_question_mark_except_the_ellipsis() {
        assert_eq!(glyph('é'), glyph('?'));
        assert_eq!(glyph('Ω'), glyph('?'));
        assert_eq!(glyph('\n'), glyph('?'));
        assert_eq!(glyph('A'), FONT[('A' as usize) - 0x20]);
        assert_eq!(glyph(' '), FONT[0]);
        assert_eq!(glyph('…'), ELLIPSIS_GLYPH);
        assert_ne!(glyph('…'), glyph('?'));
    }

    #[test]
    fn typographic_punctuation_folds_to_its_ascii_lookalike() {
        // The separator a title carries must not degrade to `?`.
        assert_eq!(glyph('—'), glyph('-'));
        assert_eq!(glyph('–'), glyph('-'));
        assert_eq!(glyph('−'), glyph('-'));
        assert_eq!(glyph('’'), glyph('\''));
        assert_eq!(glyph('“'), glyph('"'));
        assert_eq!(glyph('”'), glyph('"'));
        assert_eq!(glyph('\u{00A0}'), glyph(' '));
        assert_eq!(glyph('\u{2003}'), glyph(' '));
        // Dots draw as dots, on the middle-dot's own baseline.
        assert_eq!(glyph('·'), MIDDLE_DOT_GLYPH);
        assert_eq!(glyph('•'), MIDDLE_DOT_GLYPH);
        assert_ne!(glyph('·'), glyph('?'));
        assert_ne!(glyph('·'), glyph('.'));
        // Every fold is one char to one char, so column math never
        // shifts — asserted over the whole table, not a sample, so a
        // dropped match arm cannot silently regress a separator to `?`.
        for (input, folded) in [
            // Hyphen, figure dash, en dash, em dash, horizontal bar, minus.
            ('‐', '-'),
            ('‑', '-'),
            ('‒', '-'),
            ('–', '-'),
            ('—', '-'),
            ('―', '-'),
            ('−', '-'),
            // Curly single and double quotes, opening and closing.
            ('‘', '\''),
            ('’', '\''),
            ('‛', '\''),
            ('“', '"'),
            ('”', '"'),
            ('‟', '"'),
            // Every Unicode space.
            ('\u{00A0}', ' '),
            ('\u{1680}', ' '),
            ('\u{2000}', ' '),
            ('\u{2001}', ' '),
            ('\u{2002}', ' '),
            ('\u{2003}', ' '),
            ('\u{2004}', ' '),
            ('\u{2005}', ' '),
            ('\u{2006}', ' '),
            ('\u{2007}', ' '),
            ('\u{2008}', ' '),
            ('\u{2009}', ' '),
            ('\u{200A}', ' '),
            ('\u{202F}', ' '),
            ('\u{205F}', ' '),
            ('\u{3000}', ' '),
        ] {
            assert_eq!(
                fold_punct(input),
                folded,
                "U+{:04X} must fold to '{folded}'",
                input as u32
            );
        }
        // Only the table folds: everything else passes through — the
        // ellipsis keeps its own glyph path, letters stay themselves
        // (the diacritic fold lives at the string level, `fold_title`).
        assert_eq!(fold_punct('…'), '…');
        assert_eq!(fold_punct('é'), 'é');
    }

    #[test]
    fn latin_diacritics_fold_to_their_ascii_base_in_titles() {
        // Precomposed and decomposed forms land on the same ASCII
        // title; punctuation still rides the per-glyph table.
        assert_eq!(fold_title("Résumé — naïve.txt"), "Resume — naive.txt");
        assert_eq!(fold_title("Ångström.log"), "Angstrom.log");
        // NFD marks are dropped, so a macOS-built title folds too —
        // and the marks never spend columns for truncate_cols.
        assert_eq!(fold_title("cafe\u{0301}.txt"), "cafe.txt");
        assert_eq!(fold_title("cafe\u{0301}").chars().count(), 4);
        // Letterforms with no ASCII base stay put; `glyph`'s `?` is
        // still their honest floor. `Łódź` shows the mix: the stroke L
        // survives, its neighbors fold.
        assert_eq!(fold_title("Łódź"), "Łodz");
        assert_eq!(fold_title("您好.txt"), "您好.txt");
        for baseless in [
            '×', '÷', 'ß', 'æ', 'ð', 'ø', 'þ', 'Đ', 'Ħ', 'Ł', 'ł', 'ı', 'ŉ', 'Ŋ', 'Œ', 'œ', 'Ŧ',
            'ŧ', 'ſ',
        ] {
            assert_eq!(
                fold_diacritic(baseless),
                None,
                "{baseless} has no ASCII base"
            );
        }
    }

    #[test]
    fn the_latin_extended_table_is_exhaustive_and_canonical() {
        // Asserted over the whole table, not a sample: each entry
        // alternates upper/lower from its first codepoint, and every
        // fold lands in printable ASCII, so a mangled entry cannot
        // silently invent a letter or widen a column.
        for &(first, last, upper, lower) in LATIN_EXTENDED_A {
            for cp in first..=last {
                let folded = fold_diacritic(char::from_u32(cp).expect("table codepoint"));
                let expected = if (cp - first).is_multiple_of(2) {
                    upper
                } else {
                    lower
                };
                assert_eq!(folded, Some(expected), "U+{cp:04X} must fold to {expected}");
                assert!(
                    folded.expect("checked above").is_ascii_alphabetic(),
                    "U+{cp:04X} folds outside ASCII letters"
                );
            }
        }
    }

    #[test]
    fn default_gesture_is_hidden_with_alt_released() {
        let gesture = Gesture::default();
        assert!(!gesture.showing());
        assert!(!gesture.alt_held());
    }

    #[test]
    fn invoke_while_hidden_steps_once_and_opens_the_session() {
        let mut gesture = Gesture::default();
        assert_eq!(
            gesture.feed(GestureInput::Invoke(Direction::Next)),
            GestureOutput::Step(Direction::Next)
        );
        assert!(gesture.showing());
    }

    #[test]
    fn invoke_while_showing_continues_the_session() {
        let mut gesture = Gesture::default();
        gesture.feed(GestureInput::Invoke(Direction::Next));
        assert_eq!(
            gesture.feed(GestureInput::Invoke(Direction::Previous)),
            GestureOutput::Step(Direction::Previous)
        );
        assert!(gesture.showing());
    }

    #[test]
    fn tab_press_steps_next_and_shift_tab_steps_previous() {
        let mut gesture = Gesture::default();
        gesture.feed(GestureInput::Invoke(Direction::Next));
        assert_eq!(
            gesture.feed(GestureInput::Press {
                key: GestureKey::Tab,
                shift: false
            }),
            GestureOutput::Step(Direction::Next)
        );
        assert_eq!(
            gesture.feed(GestureInput::Press {
                key: GestureKey::Tab,
                shift: true
            }),
            GestureOutput::Step(Direction::Previous)
        );
    }

    #[test]
    fn escape_hands_the_decision_to_the_daemon() {
        // Since `type-filter`, Escape cannot be decided in the machine:
        // a standing query clears first and the session keeps going, an
        // empty one dismisses. The machine forwards and stays showing;
        // the daemon (which holds the query) closes the gesture itself
        // when it dismisses.
        let mut gesture = Gesture::default();
        gesture.feed(GestureInput::Invoke(Direction::Next));
        gesture.feed(GestureInput::Modifiers { alt: true });
        assert_eq!(
            gesture.feed(GestureInput::Press {
                key: GestureKey::Escape,
                shift: false
            }),
            GestureOutput::Escape
        );
        assert!(gesture.showing(), "a cleared query keeps the session");
    }

    #[test]
    fn other_keys_leave_the_session_alone() {
        let mut gesture = Gesture::default();
        gesture.feed(GestureInput::Invoke(Direction::Next));
        assert_eq!(
            gesture.feed(GestureInput::Press {
                key: GestureKey::Other,
                shift: false
            }),
            GestureOutput::Nothing
        );
        assert!(gesture.showing());
    }

    #[test]
    fn alt_report_held_keeps_the_session_and_tracks_state() {
        let mut gesture = Gesture::default();
        gesture.feed(GestureInput::Invoke(Direction::Next));
        assert_eq!(
            gesture.feed(GestureInput::Modifiers { alt: true }),
            GestureOutput::Nothing
        );
        assert!(gesture.showing());
        assert!(gesture.alt_held());
    }

    #[test]
    fn alt_release_commits_the_selection() {
        let mut gesture = Gesture::default();
        gesture.feed(GestureInput::Invoke(Direction::Next));
        gesture.feed(GestureInput::Modifiers { alt: true });
        assert_eq!(
            gesture.feed(GestureInput::Modifiers { alt: false }),
            GestureOutput::Commit
        );
        assert!(!gesture.showing());
    }

    #[test]
    fn first_modifiers_report_without_alt_commits_immediately() {
        // Quick tap: Alt is already released when the overlay finally maps
        // and receives keyboard focus, so the first modifier report says
        // alt=false — that tap must still switch.
        let mut gesture = Gesture::default();
        gesture.feed(GestureInput::Invoke(Direction::Next));
        assert_eq!(
            gesture.feed(GestureInput::Modifiers { alt: false }),
            GestureOutput::Commit
        );
    }

    #[test]
    fn enter_commits_the_selection_even_while_alt_is_held() {
        let mut gesture = Gesture::default();
        gesture.feed(GestureInput::Invoke(Direction::Next));
        gesture.feed(GestureInput::Modifiers { alt: true });
        assert_eq!(
            gesture.feed(GestureInput::Press {
                key: GestureKey::Commit,
                shift: false
            }),
            GestureOutput::Commit
        );
        assert!(!gesture.showing());
    }

    #[test]
    fn arrows_step_the_selection_while_showing() {
        let mut gesture = Gesture::default();
        gesture.feed(GestureInput::Invoke(Direction::Next));
        gesture.feed(GestureInput::Modifiers { alt: true });
        assert_eq!(
            gesture.feed(GestureInput::Press {
                key: GestureKey::Arrow(Direction::Previous),
                shift: false
            }),
            GestureOutput::Step(Direction::Previous)
        );
        assert_eq!(
            gesture.feed(GestureInput::Press {
                key: GestureKey::Arrow(Direction::Next),
                shift: false
            }),
            GestureOutput::Step(Direction::Next)
        );
        // Stepping never closes the session; wrap is the ring's contract.
        assert!(gesture.showing());
    }

    #[test]
    fn focus_lost_dismisses() {
        let mut gesture = Gesture::default();
        gesture.feed(GestureInput::Invoke(Direction::Next));
        assert_eq!(
            gesture.feed(GestureInput::FocusLost),
            GestureOutput::Dismiss
        );
        assert!(!gesture.showing());
    }

    #[test]
    fn inputs_after_the_session_closes_do_nothing() {
        let mut gesture = Gesture::default();
        gesture.feed(GestureInput::Invoke(Direction::Next));
        gesture.close(); // the daemon's own dismiss path
        assert_eq!(
            gesture.feed(GestureInput::Press {
                key: GestureKey::Tab,
                shift: false
            }),
            GestureOutput::Nothing
        );
        assert_eq!(
            gesture.feed(GestureInput::Modifiers { alt: false }),
            GestureOutput::Nothing
        );
        assert_eq!(
            gesture.feed(GestureInput::FocusLost),
            GestureOutput::Nothing
        );
    }

    #[test]
    fn commit_between_sessions_is_nothing() {
        let mut gesture = Gesture::default();
        assert_eq!(
            gesture.feed(GestureInput::Press {
                key: GestureKey::Commit,
                shift: false
            }),
            GestureOutput::Nothing
        );
    }

    #[test]
    fn invoke_restarts_a_closed_session() {
        let mut gesture = Gesture::default();
        gesture.feed(GestureInput::Invoke(Direction::Next));
        gesture.feed(GestureInput::FocusLost);
        assert_eq!(
            gesture.feed(GestureInput::Invoke(Direction::Next)),
            GestureOutput::Step(Direction::Next)
        );
        assert!(gesture.showing());
    }

    #[test]
    fn close_forces_the_session_hidden_and_silences_late_inputs() {
        // The daemon closes on timeouts, which the input stream cannot
        // express; a stray late event must not resurrect the session.
        let mut gesture = Gesture::default();
        gesture.feed(GestureInput::Invoke(Direction::Next));
        gesture.feed(GestureInput::Modifiers { alt: true });
        gesture.close();
        assert!(!gesture.showing());
        assert_eq!(
            gesture.feed(GestureInput::Press {
                key: GestureKey::Tab,
                shift: false
            }),
            GestureOutput::Nothing
        );
        assert_eq!(
            gesture.feed(GestureInput::Modifiers { alt: false }),
            GestureOutput::Nothing
        );
    }

    #[test]
    fn a_number_selection_activates_its_captured_handle_and_keeps_the_session_open() {
        // Ctrl+N is select-and-activate in one motion, but the session
        // survives the output: the daemon closes it via `close()` only
        // when the activation actually happens, so an unaddressable
        // Ctrl+N leaves the session fully intact.
        let mut gesture = Gesture::default();
        gesture.feed(GestureInput::Invoke(Direction::Next));
        assert_eq!(
            gesture.feed(GestureInput::Press {
                key: GestureKey::SelectHandle(Some(Handle(2))),
                shift: false
            }),
            GestureOutput::Activate {
                handle: Some(Handle(2))
            }
        );
        assert!(
            gesture.showing(),
            "the session stays open for the daemon to close"
        );
    }

    #[test]
    fn a_number_selection_between_sessions_is_nothing() {
        // The dead-arm contract every press shares: no overlay, no
        // output — for the queueable twin and for the pure translation's
        // bare row alike.
        let mut gesture = Gesture::default();
        assert_eq!(
            gesture.feed(GestureInput::Press {
                key: GestureKey::SelectHandle(Some(Handle(0))),
                shift: false
            }),
            GestureOutput::Nothing
        );
        assert_eq!(
            gesture.feed(GestureInput::Press {
                key: GestureKey::Select(0),
                shift: false
            }),
            GestureOutput::Nothing
        );
    }

    #[test]
    fn a_bare_select_row_means_nothing_to_the_machine() {
        // The invariant's machine side: a row index is not an activation
        // target. Only the boundary-produced SelectHandle twin activates;
        // the pure translation's Select never reaches the queue.
        let mut gesture = Gesture::default();
        gesture.feed(GestureInput::Invoke(Direction::Next));
        assert_eq!(
            gesture.feed(GestureInput::Press {
                key: GestureKey::Select(2),
                shift: false
            }),
            GestureOutput::Nothing,
            "the machine consumes handles, never rows"
        );
        assert!(
            gesture.showing(),
            "a bare row spends nothing — the session stands"
        );
    }

    #[test]
    fn a_repeated_number_press_after_the_daemon_closes_is_inert() {
        // Key auto-repeat after a successful activation hits the dead
        // arm: the daemon closed the spent session, so the repeat cannot
        // double-activate.
        let mut gesture = Gesture::default();
        gesture.feed(GestureInput::Invoke(Direction::Next));
        assert_eq!(
            gesture.feed(GestureInput::Press {
                key: GestureKey::SelectHandle(Some(Handle(1))),
                shift: false
            }),
            GestureOutput::Activate {
                handle: Some(Handle(1))
            }
        );
        gesture.close();
        assert_eq!(
            gesture.feed(GestureInput::Press {
                key: GestureKey::SelectHandle(Some(Handle(1))),
                shift: false
            }),
            GestureOutput::Nothing
        );
        assert_eq!(
            gesture.feed(GestureInput::Modifiers { alt: false }),
            GestureOutput::Nothing
        );
    }

    #[test]
    fn key_input_maps_tab_escape_and_backtab() {
        assert_eq!(
            key_input(Keysym::Tab, false, false, false, KeyMode::Flat),
            GestureInput::Press {
                key: GestureKey::Tab,
                shift: false
            }
        );
        // Shift held at the seat steers a plain Tab backwards.
        assert_eq!(
            key_input(Keysym::Tab, true, false, false, KeyMode::Flat),
            GestureInput::Press {
                key: GestureKey::Tab,
                shift: true
            }
        );
        // ISO_Left_Tab is Shift+Tab by another name.
        assert_eq!(
            key_input(Keysym::ISO_Left_Tab, false, false, false, KeyMode::Flat),
            GestureInput::Press {
                key: GestureKey::Tab,
                shift: true
            }
        );
        assert_eq!(
            key_input(Keysym::Escape, false, false, false, KeyMode::Flat),
            GestureInput::Press {
                key: GestureKey::Escape,
                shift: false
            }
        );
        assert_eq!(
            key_input(Keysym::Return, false, false, false, KeyMode::Flat),
            GestureInput::Press {
                key: GestureKey::Commit,
                shift: false
            }
        );
    }

    #[test]
    fn key_input_maps_the_enter_family_to_commit() {
        // Return, KP_Enter, and ISO_Enter are one key by three names.
        for keysym in [Keysym::Return, Keysym::KP_Enter, Keysym::ISO_Enter] {
            assert_eq!(
                key_input(keysym, false, false, false, KeyMode::Flat),
                GestureInput::Press {
                    key: GestureKey::Commit,
                    shift: false
                }
            );
        }
    }

    #[test]
    fn key_input_maps_arrows_to_directional_steps() {
        // Direction rides the key, not the seat's shift state: arrows carry
        // their direction inherently, so shift is reported false either way.
        for (keysym, direction) in [
            (Keysym::Up, Direction::Previous),
            (Keysym::Left, Direction::Previous),
            (Keysym::Down, Direction::Next),
            (Keysym::Right, Direction::Next),
        ] {
            assert_eq!(
                key_input(keysym, true, false, false, KeyMode::Flat),
                GestureInput::Press {
                    key: GestureKey::Arrow(direction),
                    shift: false
                }
            );
        }
    }

    #[test]
    fn key_input_leaves_unknown_keysyms_alone() {
        assert_eq!(
            key_input(Keysym::Delete, true, false, false, KeyMode::Flat),
            GestureInput::Press {
                key: GestureKey::Other,
                shift: true
            }
        );
    }

    #[test]
    fn key_input_maps_ctrl_top_row_digits_to_row_selects() {
        // Ctrl+1..9 addresses rows 0..=8 in one motion; the digit's
        // value picks the row.
        let digits = [
            (Keysym::_1, 0),
            (Keysym::_2, 1),
            (Keysym::_3, 2),
            (Keysym::_4, 3),
            (Keysym::_5, 4),
            (Keysym::_6, 5),
            (Keysym::_7, 6),
            (Keysym::_8, 7),
            (Keysym::_9, 8),
        ];
        for (keysym, row) in digits {
            assert_eq!(
                key_input(keysym, false, true, false, KeyMode::Flat),
                GestureInput::Press {
                    key: GestureKey::Select(row),
                    shift: false
                }
            );
        }
    }

    #[test]
    fn key_input_maps_ctrl_keypad_digits_to_row_selects() {
        // A keypad digit is the same intent as a top-row digit — the
        // Enter family already accepts the keypad's Enter.
        let digits = [
            (Keysym::KP_1, 0),
            (Keysym::KP_2, 1),
            (Keysym::KP_3, 2),
            (Keysym::KP_4, 3),
            (Keysym::KP_5, 4),
            (Keysym::KP_6, 5),
            (Keysym::KP_7, 6),
            (Keysym::KP_8, 7),
            (Keysym::KP_9, 8),
        ];
        for (keysym, row) in digits {
            assert_eq!(
                key_input(keysym, false, true, false, KeyMode::Flat),
                GestureInput::Press {
                    key: GestureKey::Select(row),
                    shift: false
                }
            );
        }
    }

    #[test]
    fn key_input_types_digits_without_ctrl() {
        // `type-filter` claims the number row: without Ctrl a digit
        // types into the query, top row and keypad alike — it can no
        // longer surprise-activate because it no longer activates at
        // all.
        assert_eq!(
            key_input(Keysym::_3, false, false, false, KeyMode::Flat),
            GestureInput::Press {
                key: GestureKey::Type('3'),
                shift: false
            }
        );
        assert_eq!(
            key_input(Keysym::KP_5, false, false, false, KeyMode::Flat),
            GestureInput::Press {
                key: GestureKey::Type('5'),
                shift: false
            }
        );
    }

    #[test]
    fn key_input_maps_shift_ctrl_digits_too() {
        // Shift is ignored for digits: the chord still selects, and the
        // seat's shift state is carried as-is.
        assert_eq!(
            key_input(Keysym::_4, true, true, false, KeyMode::Flat),
            GestureInput::Press {
                key: GestureKey::Select(3),
                shift: true
            }
        );
    }

    #[test]
    fn key_input_leaves_ctrl_zero_and_ctrl_non_digits_alone() {
        // No Ctrl+0-as-10 (OQ 2): zero never selects. Since
        // `type-filter` it also never surprises — a Ctrl+0 types a 0
        // like any printable (the query ignores modifier flavor), and
        // Ctrl on a non-printable (Delete) stays inert.
        assert_eq!(
            key_input(Keysym::_0, false, true, false, KeyMode::Flat),
            GestureInput::Press {
                key: GestureKey::Type('0'),
                shift: false
            }
        );
        assert_eq!(
            key_input(Keysym::KP_0, false, true, false, KeyMode::Flat),
            GestureInput::Press {
                key: GestureKey::Type('0'),
                shift: false
            }
        );
        assert_eq!(
            key_input(Keysym::Delete, false, true, false, KeyMode::Flat),
            GestureInput::Press {
                key: GestureKey::Other,
                shift: false
            }
        );
    }

    #[test]
    fn key_input_leaves_gesture_keys_unchanged_under_ctrl() {
        // Existing key semantics are byte-identical whatever Ctrl does:
        // the digit arm never swallows another gesture key.
        assert_eq!(
            key_input(Keysym::Tab, false, true, false, KeyMode::Flat),
            GestureInput::Press {
                key: GestureKey::Tab,
                shift: false
            }
        );
        assert_eq!(
            key_input(Keysym::Escape, false, true, false, KeyMode::Flat),
            GestureInput::Press {
                key: GestureKey::Escape,
                shift: false
            }
        );
        assert_eq!(
            key_input(Keysym::Return, false, true, false, KeyMode::Flat),
            GestureInput::Press {
                key: GestureKey::Commit,
                shift: false
            }
        );
    }

    #[test]
    fn grouped_key_input_binds_tab_to_the_linear_step() {
        // Rev 5: Tab is ↓'s twin in grouped mode too — one visible row,
        // wrapping, headers included. The level-scoped step is gone.
        assert_eq!(
            key_input(Keysym::Tab, false, false, false, KeyMode::Grouped),
            GestureInput::Press {
                key: GestureKey::Tab,
                shift: false
            }
        );
        assert_eq!(
            key_input(Keysym::Tab, true, false, false, KeyMode::Grouped),
            GestureInput::Press {
                key: GestureKey::Tab,
                shift: true
            }
        );
        // ISO_Left_Tab is Shift+Tab by another name, grouped or flat.
        assert_eq!(
            key_input(Keysym::ISO_Left_Tab, false, false, false, KeyMode::Grouped),
            GestureInput::Press {
                key: GestureKey::Tab,
                shift: true
            }
        );
    }

    #[test]
    fn the_level_toggle_key_is_gone() {
        // Rev 5 removed the level toggle; `type-filter` claims the
        // letters — Alt+W types a w into the query in either mode, and
        // W without Alt types it too (printables ride every modifier).
        for mode in [KeyMode::Flat, KeyMode::Grouped] {
            for (keysym, ch) in [(Keysym::w, 'w'), (Keysym::W, 'W')] {
                for alt_held in [true, false] {
                    assert_eq!(
                        key_input(keysym, false, false, alt_held, mode),
                        GestureInput::Press {
                            key: GestureKey::Type(ch),
                            shift: false
                        },
                        "Alt+{keysym:?} {mode:?} types"
                    );
                }
            }
        }
    }

    #[test]
    fn grouped_digits_with_alt_type_into_the_query() {
        // Rev 5 removed Alt+N; `type-filter` claims the number row —
        // digits type, through Alt, in grouped mode and out of it.
        // (Ctrl+N keeps its chord arm.)
        for mode in [KeyMode::Flat, KeyMode::Grouped] {
            for keysym in [Keysym::KP_1, Keysym::KP_9] {
                assert_eq!(
                    key_input(keysym, false, false, true, mode),
                    GestureInput::Press {
                        key: GestureKey::Type(keysym.key_char().expect("keypad digits map")),
                        shift: false
                    },
                    "Alt+{keysym:?} {mode:?} types, never chords"
                );
            }
        }
    }

    #[test]
    fn grouped_key_input_maps_left_right_to_the_tree_walk() {
        // The W3C vertical tree axes: → enters a multi-window app's
        // children, ← exits to the parent. The flat step aliases are
        // gone in grouped mode — the vertical step axis is ↑/↓ (and
        // Alt+Tab / Alt+Shift+Tab) alone.
        assert_eq!(
            key_input(Keysym::Right, false, false, false, KeyMode::Grouped),
            GestureInput::Press {
                key: GestureKey::EnterChild,
                shift: false
            }
        );
        assert_eq!(
            key_input(Keysym::Left, false, false, false, KeyMode::Grouped),
            GestureInput::Press {
                key: GestureKey::ExitParent,
                shift: false
            }
        );
        // Up/Down keep stepping visible rows whatever the mode.
        assert_eq!(
            key_input(Keysym::Up, false, false, false, KeyMode::Grouped),
            GestureInput::Press {
                key: GestureKey::Arrow(Direction::Previous),
                shift: false
            }
        );
        assert_eq!(
            key_input(Keysym::Down, false, false, false, KeyMode::Grouped),
            GestureInput::Press {
                key: GestureKey::Arrow(Direction::Next),
                shift: false
            }
        );
    }

    #[test]
    fn flat_key_input_keeps_the_left_right_step_aliases() {
        // The regression pin: grouping off reproduces today's mapping
        // byte for byte — Left/Right step like Up/Down, Tab steps rows.
        assert_eq!(
            key_input(Keysym::Left, false, false, false, KeyMode::Flat),
            GestureInput::Press {
                key: GestureKey::Arrow(Direction::Previous),
                shift: false
            }
        );
        assert_eq!(
            key_input(Keysym::Right, false, false, false, KeyMode::Flat),
            GestureInput::Press {
                key: GestureKey::Arrow(Direction::Next),
                shift: false
            }
        );
        assert_eq!(
            key_input(Keysym::Tab, false, false, false, KeyMode::Flat),
            GestureInput::Press {
                key: GestureKey::Tab,
                shift: false
            }
        );
    }

    #[test]
    fn grouped_key_input_keeps_the_ctrl_chord() {
        // Rev 5: Ctrl+N addresses the Nth level-1 entry — the one
        // chord left; Alt+N is gone with the scoped vocabulary. The
        // Enter family still commits and Escape still dismisses,
        // whatever the mode.
        assert_eq!(
            key_input(Keysym::_3, false, true, false, KeyMode::Grouped),
            GestureInput::Press {
                key: GestureKey::SelectApp(2),
                shift: false
            }
        );
        // Without Ctrl the digit types into the query — Ctrl is the
        // one modifier that wins over typing.
        assert_eq!(
            key_input(Keysym::_3, false, false, false, KeyMode::Grouped),
            GestureInput::Press {
                key: GestureKey::Type('3'),
                shift: false
            }
        );
        assert_eq!(
            key_input(Keysym::Return, false, false, false, KeyMode::Grouped),
            GestureInput::Press {
                key: GestureKey::Commit,
                shift: false
            }
        );
        assert_eq!(
            key_input(Keysym::Escape, false, false, false, KeyMode::Grouped),
            GestureInput::Press {
                key: GestureKey::Escape,
                shift: false
            }
        );
    }

    #[test]
    fn the_grouped_keys_forward_to_the_daemon() {
        // The machine cannot resolve grouped walks — the daemon holds
        // the frozen tree and the expansion state — so it forwards
        // them while showing, exactly as it forwards arrow steps.
        // Rev 5: Tab forwards as ↓'s twin, the level vocabulary is
        // gone, and only →/← remain as tree keys.
        let mut gesture = Gesture::default();
        gesture.feed(GestureInput::Invoke(Direction::Next));
        assert_eq!(
            gesture.feed(GestureInput::Press {
                key: GestureKey::EnterChild,
                shift: false
            }),
            GestureOutput::EnterChild
        );
        assert_eq!(
            gesture.feed(GestureInput::Press {
                key: GestureKey::ExitParent,
                shift: false
            }),
            GestureOutput::ExitParent
        );
        assert_eq!(
            gesture.feed(GestureInput::Press {
                key: GestureKey::Tab,
                shift: false
            }),
            GestureOutput::Step(Direction::Next)
        );
        assert_eq!(
            gesture.feed(GestureInput::Press {
                key: GestureKey::Tab,
                shift: true
            }),
            GestureOutput::Step(Direction::Previous)
        );
        // Between sessions nothing forwards — the overlay is unmapped.
        let mut gesture = Gesture::default();
        assert_eq!(
            gesture.feed(GestureInput::Press {
                key: GestureKey::EnterChild,
                shift: false
            }),
            GestureOutput::Nothing
        );
    }

    #[test]
    fn the_grouped_keys_mean_nothing_between_sessions() {
        let mut gesture = Gesture::default();
        for key in [
            GestureKey::Tab,
            GestureKey::EnterChild,
            GestureKey::ExitParent,
        ] {
            assert_eq!(
                gesture.feed(GestureInput::Press { key, shift: false }),
                GestureOutput::Nothing
            );
        }
    }

    #[test]
    fn every_displayed_hint_is_a_wired_key() {
        // The render twin of the input mapping: the label on row N names
        // the chord that activates row N. The pairing is pinned end to
        // end so the two tables cannot drift — a hinted-but-unwired row
        // would be a dead key advertised on the card.
        let digits = [
            (Keysym::_1, 0),
            (Keysym::_2, 1),
            (Keysym::_3, 2),
            (Keysym::_4, 3),
            (Keysym::_5, 4),
            (Keysym::_6, 5),
            (Keysym::_7, 6),
            (Keysym::_8, 7),
            (Keysym::_9, 8),
        ];
        for (keysym, row) in digits {
            assert_eq!(
                key_input(keysym, false, true, false, KeyMode::Flat),
                GestureInput::Press {
                    key: GestureKey::Select(row),
                    shift: false
                },
                "the chord hinted on row {row} must wire that row"
            );
            let hint = RowHint::ctrl(row).expect("rows 1..9 are hinted");
            assert_eq!(
                hint.digit(),
                format!("{}", row + 1),
                "row {row}'s digit must name its own chord"
            );
        }
        // The reserved band fits the longest label exactly — nothing
        // spare to misalign, nothing clipped.
        assert_eq!(RowHint::Ctrl(9).digit().len(), HINT_COLS as usize);
        assert_eq!(MAX_HINTS, 9, "nine wired entries, nine digits");
    }

    #[test]
    fn wheel_notches_counts_value120_steps_with_sign() {
        // Each multiple of ±120 is one notch; positive is one Next step.
        // Exact multiples leave nothing in the residual.
        assert_eq!(wheel_notches(120, 0, 0), (1, 0));
        assert_eq!(wheel_notches(-120, 0, 0), (-1, 0));
        assert_eq!(wheel_notches(240, 0, 0), (2, 0));
        assert_eq!(wheel_notches(-240, 0, 0), (-2, 0));
    }

    #[test]
    fn wheel_notches_accumulates_sub_notch_deltas_linearly() {
        // v120 counts 1/120ths of a notch, so a physical notch may arrive
        // split across deltas. The residual carries the sub-notch part
        // into the next event: the protocol's own examples are 60+60 = 1
        // notch and 60-60 = 0 notches. Truncating each event on its own
        // instead dropped split notches entirely — the wheel stepped
        // non-linearly.
        assert_eq!(wheel_notches(60, 0, 0), (0, 60));
        assert_eq!(wheel_notches(60, 0, 60), (1, 0));
        // Reversing an uncommitted half notch cancels it.
        assert_eq!(wheel_notches(-60, 0, 60), (0, 0));
        // A full sequence of split deltas steps exactly once per notch
        // whatever the split: four 60s are two notches (this sequence
        // stepped zero times before the residual existed).
        let mut residual = 0;
        let mut steps = 0;
        for _ in 0..4 {
            let (notches, next) = wheel_notches(60, 0, residual);
            steps += notches;
            residual = next;
        }
        assert_eq!((steps, residual), (2, 0));
        // High-resolution deltas (a notch as 40+40+40, a touchpad's
        // stream of 30s) step per crossed 120, remainder carried: 510
        // total is four whole notches.
        let mut residual = 0;
        let mut steps = 0;
        for delta in [40, 40, 40, 30, 30, 30, 30, 90, 90, 90] {
            let (notches, next) = wheel_notches(delta, 0, residual);
            steps += notches;
            residual = next;
        }
        assert_eq!((steps, residual), (4, 30));
    }

    #[test]
    fn wheel_notches_falls_back_to_the_legacy_discrete_step() {
        // Pre-v120 compositors report whole steps in `discrete` only;
        // whole steps never touch the v120 residual.
        assert_eq!(wheel_notches(0, 1, 0), (1, 0));
        assert_eq!(wheel_notches(0, -2, 60), (-2, 60));
    }

    #[test]
    fn wheel_notches_on_continuous_scroll_steps_nothing() {
        // An event carrying neither field disturbs nothing: the residual
        // waits for the next v120 delta.
        assert_eq!(wheel_notches(0, 0, 60), (0, 60));
    }

    #[test]
    fn wheel_notches_survives_a_hostile_value120_bounded_per_event() {
        // A glitching or hostile compositor can deliver an axis value at
        // the i32 rails — plain `residual + value120` panics a debug
        // daemon there, and a release daemon freezes enqueueing
        // millions of steps. The contract: no panic, at most ±32 notches
        // per event, residual always sub-notch, and rotation beyond the
        // clamp dropped (not carried into the residual).
        for value120 in [i32::MAX, i32::MIN] {
            for residual in [0, 60, -60, 119, -119] {
                let (notches, carried) = wheel_notches(value120, 0, residual);
                assert!(
                    notches.abs() <= WHEEL_MAX_NOTCHES_PER_EVENT,
                    "notches bounded per event: {notches} (v120 {value120}, residual {residual})"
                );
                assert!(
                    carried.abs() < 120,
                    "residual stays sub-notch: {carried} (v120 {value120}, residual {residual})"
                );
            }
        }
        // The bound itself: exactly 32 notches passes through; one notch
        // past it clamps, and the excess is dropped, not carried.
        assert_eq!(
            wheel_notches(WHEEL_MAX_NOTCHES_PER_EVENT * 120, 0, 0),
            (32, 0)
        );
        assert_eq!(
            wheel_notches((WHEEL_MAX_NOTCHES_PER_EVENT + 1) * 120, 0, 0),
            (32, 0)
        );
    }

    #[test]
    fn wheel_notches_prefers_value120_over_the_legacy_discrete_step() {
        // An event reporting both fields is a v120 event: the discrete
        // step is ignored, and the residual is untouched by it.
        assert_eq!(wheel_notches(120, -1, 0), (1, 0));
    }

    #[test]
    fn wheel_notches_clamps_the_legacy_discrete_step_to_the_same_bound() {
        // Uniform contract: a garbage discrete count is as bounded as a
        // garbage v120 — the enqueue loop's `unsigned_abs()` never
        // meets a freeze-regime count.
        assert_eq!(wheel_notches(0, i32::MIN, 60), (-32, 60));
        assert_eq!(wheel_notches(0, i32::MAX, -60), (32, -60));
    }

    #[test]
    fn notch_is_echo_collapses_the_vm_relay_double_click() {
        // A virtualized mouse relay was observed delivering one detent
        // twice, 1 ms apart (probe capture, COSMIC VM): the second notch
        // is machine-made and must not step. Before echo suppression
        // existed, every such pair skipped an item.
        assert!(!notch_is_echo(Direction::Next, 3_820_000, None));
        assert!(notch_is_echo(
            Direction::Next,
            3_820_001,
            Some((Direction::Next, 3_820_000))
        ));
    }

    #[test]
    fn notch_is_echo_keeps_real_fast_repeats() {
        // Detents a fast real flick produces (tens of ms apart) always
        // step, both directions, and a direction reversal is never an
        // echo even back-to-back.
        assert!(!notch_is_echo(
            Direction::Next,
            3_820_056,
            Some((Direction::Next, 3_820_000))
        ));
        assert!(!notch_is_echo(
            Direction::Previous,
            3_820_001,
            Some((Direction::Next, 3_820_000))
        ));
        // The window edge itself is still an echo; one past it is not.
        assert!(notch_is_echo(
            Direction::Next,
            3_820_008,
            Some((Direction::Next, 3_820_000))
        ));
        assert!(!notch_is_echo(
            Direction::Next,
            3_820_009,
            Some((Direction::Next, 3_820_000))
        ));
    }

    #[test]
    fn notch_is_echo_ignores_clock_skew_between_notches() {
        // A relay that delivers timestamps out of order must not eat a
        // real notch as a "negative gap" echo; only same-direction
        // forward gaps within the window count.
        assert!(!notch_is_echo(
            Direction::Next,
            3_819_000,
            Some((Direction::Next, 3_820_000))
        ));
    }

    #[test]
    fn notch_echo_pipeline_steps_three_for_the_captured_sequence() {
        // The probe capture end to end: four -120 events whose middle
        // pair is 1 ms apart are three steps, not four. Each whole-notch
        // event consults the last accepted notch.
        let events = [
            (3_819_760i32, -1),
            (3_820_000, -1),
            (3_820_001, -1),
            (3_820_056, -1),
        ];
        let mut last = None;
        let mut steps = 0;
        for (time, sign) in events {
            let (notches, _) = wheel_notches(sign * 120, 0, 0);
            let direction = if notches > 0 {
                Direction::Next
            } else {
                Direction::Previous
            };
            if notches != 0 && !notch_is_echo(direction, time as u32, last) {
                steps += notches;
                last = Some((direction, time as u32));
            }
        }
        assert_eq!(steps, -3);
    }

    #[test]
    fn hover_first_contact_seeds_the_row_without_selecting() {
        // The overlay maps center-screen, often under a resting cursor,
        // and a surface's first pointer event is the Enter that contact
        // produces. It seeds the baseline and selects nothing, so the
        // MRU step target keeps the selection; jitter inside the row is
        // no crossing either.
        let mut hover = PointerHover::default();
        assert_eq!(
            hover.observe(Some(3)),
            None,
            "map-time Enter selects nothing"
        );
        assert_eq!(hover.observe(Some(3)), None, "in-row jitter is no crossing");
    }

    #[test]
    fn hover_row_crossings_select_each_time() {
        let mut hover = PointerHover::default();
        assert_eq!(hover.observe(Some(1)), None, "first contact seeds");
        assert_eq!(hover.observe(Some(2)), Some(2));
        assert_eq!(hover.observe(Some(3)), Some(3));
        assert_eq!(
            hover.observe(Some(3)),
            None,
            "same row again is no crossing"
        );
    }

    #[test]
    fn hover_none_observation_never_selects_but_forgets_the_row() {
        // A gap motion and a card leave arrive the same way — a None
        // observation. Neither selects, but both forget the row, so
        // returning to it re-asserts pointer intent.
        let mut hover = PointerHover::default();
        assert_eq!(hover.observe(Some(2)), None, "first contact seeds");
        assert_eq!(hover.observe(None), None, "gap or leave selects nothing");
        assert_eq!(hover.observe(Some(2)), Some(2), "returning re-selects");
    }

    #[test]
    fn hover_first_contact_in_a_gap_still_counts_as_contact() {
        // First contact can land in a gap seam. Counting it as contact
        // keeps the seed where it belongs — otherwise moving into a row
        // would read as the map-time Enter and never select.
        let mut hover = PointerHover::default();
        assert_eq!(hover.observe(None), None);
        assert_eq!(hover.observe(Some(3)), Some(3));
    }

    #[test]
    fn hover_reset_seeds_the_next_session_from_scratch() {
        // The daemon resets on every overlay end: the next session's
        // first contact — the cursor resting right where the new card
        // maps — seeds again instead of selecting.
        let mut hover = PointerHover::default();
        assert_eq!(hover.observe(Some(2)), None, "first contact seeds");
        assert_eq!(hover.observe(Some(3)), Some(3));
        hover.reset();
        assert_eq!(hover.observe(Some(3)), None, "fresh session seeds again");
    }

    // --- grouped-horizontal: the derived strip (`inplace-strip`) ---------

    /// The strip card over [`grouped_snap`]'s tree: `sequence` the
    /// derived strip (tree indices, as the daemon derived it),
    /// `selected` the tree row the session holds, `width_slots` the
    /// frozen width — the daemon's max of the closed and open shapes.
    fn build_strip(
        snap: &Snapshot,
        sequence: &[usize],
        selected: usize,
        width_slots: usize,
        tiles: usize,
    ) -> Option<OverlayModel> {
        build_strip_floored(snap, sequence, selected, width_slots, tiles, None)
    }

    /// [`build_strip`] with the session's width floor carried
    /// (`width` / `min-width` resolved; `None` is the frozen grid).
    fn build_strip_floored(
        snap: &Snapshot,
        sequence: &[usize],
        selected: usize,
        width_slots: usize,
        tiles: usize,
        width_floor: Option<u32>,
    ) -> Option<OverlayModel> {
        let tree = crate::switcher::Grouping::new(
            &snap.toplevels,
            snap.active.expect("fixture has an active workspace"),
            Scope::CurrentWorkspace,
        );
        overlay_model_grouped_horizontal(
            &tree,
            sequence,
            selected,
            width_slots,
            snap,
            &no_icons(),
            2,
            &Palette::dark(),
            StyleProfile::default(),
            Banding {
                tiles,
                width_floor,
                prev: None,
            },
            TextBackend::Bitmap,
        )
    }

    /// The grouped fixture's tree indices: [P-a1, C-a1, C-a2, L-b, L-u]
    /// over [`grouped_snap`] — the closed strip [0, 3, 4] (the level-1
    /// rows), the open strip [1, 2, 3, 4] (a's run at a's slot).
    fn combined_fixture() -> (Snapshot, Vec<usize>, Vec<usize>) {
        let snap = grouped_snap();
        (
            snap.clone(),
            vec![0, 3, 4],
            vec![1, 2, 3, 4], // a's windows in a's slot, then the other apps
        )
    }

    #[test]
    fn combined_key_input_swaps_the_tree_axes() {
        // The W3C horizontal-tree axis swap: ←/→ are the step axis
        // (within the focused band), ↓/↑ the level axis (enter band 2 /
        // exit to the app tile), Tab steps apps (joining the linear
        // walk inside an open run at the daemon), and
        // Ctrl+N addresses rows — the card's rows are already band 1
        // then band 2 in reading order.
        let combined = key_input(Keysym::Tab, false, false, false, KeyMode::GroupedHorizontal);
        assert_eq!(
            combined,
            GestureInput::Press {
                key: GestureKey::StepApp(Direction::Next),
                shift: false
            }
        );
        assert_eq!(
            key_input(Keysym::Tab, true, false, false, KeyMode::GroupedHorizontal),
            GestureInput::Press {
                key: GestureKey::StepApp(Direction::Previous),
                shift: true
            }
        );
        assert_eq!(
            key_input(
                Keysym::ISO_Left_Tab,
                false,
                false,
                false,
                KeyMode::GroupedHorizontal
            ),
            GestureInput::Press {
                key: GestureKey::StepApp(Direction::Previous),
                shift: true
            }
        );
        assert_eq!(
            key_input(
                Keysym::Left,
                false,
                false,
                false,
                KeyMode::GroupedHorizontal
            ),
            GestureInput::Press {
                key: GestureKey::Arrow(Direction::Previous),
                shift: false
            }
        );
        assert_eq!(
            key_input(
                Keysym::Right,
                false,
                false,
                false,
                KeyMode::GroupedHorizontal
            ),
            GestureInput::Press {
                key: GestureKey::Arrow(Direction::Next),
                shift: false
            }
        );
        assert_eq!(
            key_input(
                Keysym::Down,
                false,
                false,
                false,
                KeyMode::GroupedHorizontal
            ),
            GestureInput::Press {
                key: GestureKey::EnterChild,
                shift: false
            }
        );
        assert_eq!(
            key_input(Keysym::Up, false, false, false, KeyMode::GroupedHorizontal),
            GestureInput::Press {
                key: GestureKey::ExitParent,
                shift: false
            }
        );
        assert_eq!(
            key_input(Keysym::_3, false, true, false, KeyMode::GroupedHorizontal),
            GestureInput::Press {
                key: GestureKey::Select(2),
                shift: false
            },
            "Ctrl+N addresses reading-order rows, not level-1 entries"
        );
        // The gesture machine forwards the app step like the tree keys.
        let mut gesture = Gesture::default();
        gesture.feed(GestureInput::Invoke(Direction::Next));
        assert_eq!(
            gesture.feed(GestureInput::Press {
                key: GestureKey::StepApp(Direction::Next),
                shift: false
            }),
            GestureOutput::StepApp(Direction::Next)
        );
    }

    #[test]
    fn grave_works_the_group_in_the_grouped_cards() {
        // The key above Tab (keysym `grave`, ` on US layouts) is the
        // focused app's key in every grouped card — the strip and the
        // vertical tree: toggle from the entry, cycle from inside — and
        // only there: flat sessions keep typing it into the query, the
        // type-filter contract untouched. The direction still picks the
        // landing window when opening and the cycle's way when inside.
        for mode in [KeyMode::GroupedHorizontal, KeyMode::Grouped] {
            assert_eq!(
                key_input(Keysym::grave, false, false, false, mode),
                GestureInput::Press {
                    key: GestureKey::ToggleOrCycle(Direction::Next),
                    shift: false
                },
                "{mode:?} runs forward on the bare key"
            );
            assert_eq!(
                key_input(Keysym::grave, true, false, false, mode),
                GestureInput::Press {
                    key: GestureKey::ToggleOrCycle(Direction::Previous),
                    shift: true
                },
                "{mode:?} runs backwards on Shift"
            );
            // The runtime form of Alt+Shift+`: the compositor reports
            // the level-resolved keysym, so the shifted press arrives
            // as `asciitilde` on US layouts — the arm must match it
            // too, or the backwards press is unreachable (2026-11-09).
            assert_eq!(
                key_input(Keysym::asciitilde, true, false, false, mode),
                GestureInput::Press {
                    key: GestureKey::ToggleOrCycle(Direction::Previous),
                    shift: true
                },
                "{mode:?} runs backwards on the shifted keysym"
            );
            assert_eq!(
                key_input(Keysym::asciitilde, false, false, false, mode),
                GestureInput::Press {
                    key: GestureKey::ToggleOrCycle(Direction::Next),
                    shift: false
                },
                "{mode:?} runs forward on the bare shifted keysym"
            );
        }
        assert_eq!(
            key_input(Keysym::grave, false, false, false, KeyMode::Flat),
            GestureInput::Press {
                key: GestureKey::Type('\u{60}'),
                shift: false
            },
            "Flat keeps typing the backtick"
        );
        assert_eq!(
            key_input(Keysym::asciitilde, true, false, false, KeyMode::Flat),
            GestureInput::Press {
                key: GestureKey::Type('~'),
                shift: true
            },
            "Flat keeps typing the tilde"
        );
        // The gesture machine forwards the key like the app step.
        let mut gesture = Gesture::default();
        gesture.feed(GestureInput::Invoke(Direction::Next));
        assert_eq!(
            gesture.feed(GestureInput::Press {
                key: GestureKey::ToggleOrCycle(Direction::Next),
                shift: false
            }),
            GestureOutput::ToggleOrCycle(Direction::Next)
        );
    }

    #[test]
    fn grave_leaves_the_printable_arm_byte_for_byte() {
        // The carve-out is the key above Tab — `grave` and its shifted
        // `asciitilde` twin, and in the grouped cards alone: every other
        // printable keysym maps to Type(char) through every mode, exactly
        // as before — the table pins the neighbors beside grave on the US
        // layout and the printable mainstays alike.
        let table = [
            (Keysym::apostrophe, '\''),
            (Keysym::minus, '-'),
            (Keysym::equal, '='),
            (Keysym::a, 'a'),
            (Keysym::_1, '1'),
            (Keysym::space, ' '),
            (Keysym::underscore, '_'),
        ];
        for (keysym, ch) in table {
            for mode in [KeyMode::Flat, KeyMode::Grouped, KeyMode::GroupedHorizontal] {
                assert_eq!(
                    key_input(keysym, false, false, false, mode),
                    GestureInput::Press {
                        key: GestureKey::Type(ch),
                        shift: false
                    },
                    "{keysym:?} {mode:?} still types"
                );
            }
        }
    }

    #[test]
    fn strip_model_lays_the_open_run_in_the_app_slot() {
        // The open shape: the selected app's tile is replaced in place
        // by its window run — window tiles at the app's slot, the other
        // apps' tiles around it, one selection on the window tile. The
        // strip's height is the single band's.
        let (snap, _closed, open) = combined_fixture();
        let model = build_strip(&snap, &open, 1, 4, 8).expect("eligible windows");
        assert!(model.horizontal);
        let rows: Vec<(crate::switcher::RowKind, Option<Handle>, bool)> = model
            .rows
            .iter()
            .map(|row| (row.kind, row.handle, row.selected))
            .collect();
        assert_eq!(
            rows,
            vec![
                // a's run — C-a1 selected, C-a2 — then the other apps.
                (crate::switcher::RowKind::Child, Some(Handle(1)), true),
                (crate::switcher::RowKind::Child, Some(Handle(3)), false),
                (crate::switcher::RowKind::Leaf, Some(Handle(2)), false),
                (crate::switcher::RowKind::Leaf, Some(Handle(4)), false),
            ]
        );
        // A window tile carries its own title; the strip never grows a
        // second band.
        assert_eq!(model.rows[0].title, "a-front");
        assert_eq!(model.height, band_card_height());
    }

    #[test]
    fn a_width_floor_widens_the_grouped_strip() {
        // The same contract on the combined strip: the frozen grid
        // keeps its slots, the floor buys the air around them, and a
        // floor below the grid sizes nothing.
        let (snap, closed, _open) = combined_fixture();
        let grid = build_strip(&snap, &closed, 0, 4, 8).expect("eligible windows");
        let under =
            build_strip_floored(&snap, &closed, 0, 4, 8, Some(200)).expect("eligible windows");
        assert_eq!(
            under.width, grid.width,
            "a floor below the frozen grid sizes nothing"
        );
        assert_eq!(grid.width, band_width(4), "the grid's own shape");
        let over =
            build_strip_floored(&snap, &closed, 0, 4, 8, Some(900)).expect("eligible windows");
        assert_eq!(over.width, 900, "the floor above widens");
    }

    #[test]
    fn strip_model_keeps_plain_app_tiles_on_a_level1_selection() {
        // The closed shape: plain app tiles — the parent labelled by its
        // app_id with its count, the leaves by their titles — and the
        // width already frozen at the open shape's: ↓ opens the run
        // inside the same grid, so the card never resizes.
        let (snap, closed, _open) = combined_fixture();
        let model = build_strip(&snap, &closed, 0, 4, 8).expect("eligible windows");
        let rows: Vec<(crate::switcher::RowKind, Option<Handle>, bool, usize)> = model
            .rows
            .iter()
            .map(|row| (row.kind, row.handle, row.selected, row.count))
            .collect();
        assert_eq!(
            rows,
            vec![
                (crate::switcher::RowKind::Parent, Some(Handle(1)), true, 2),
                (crate::switcher::RowKind::Leaf, Some(Handle(2)), false, 0),
                (crate::switcher::RowKind::Leaf, Some(Handle(4)), false, 0),
            ]
        );
        assert_eq!(model.rows[0].title, "app.a");
        assert_eq!(model.rows.len(), 3, "three tiles render");
        assert_eq!(
            model.width,
            band_width(4),
            "frozen at the open shape's four slots, not the shown three"
        );
    }

    #[test]
    fn strip_hints_number_the_reading_order_under_nine() {
        // Ctrl+N addresses the strip as shown: hints number the visible
        // content tiles left→right, the `…` tile carries none, and
        // `content_tile_handle` resolves the same order — the truncated
        // edge under a two-tile budget.
        let (snap, _closed, open) = combined_fixture();
        let model = build_strip(&snap, &open, 1, 2, 2).expect("builds");
        assert_eq!(model.rows.len(), 2, "one content tile plus the dots");
        assert_eq!(model.rows[0].hint, Some(RowHint::Ctrl(1)));
        assert_eq!(model.rows[1].hint, None, "the affordance tile");
        assert_eq!(model.content_tile_handle(0), Some(Handle(1)));
        assert_eq!(
            model.content_tile_handle(1),
            None,
            "only one content tile shows"
        );
        assert_eq!(model.rows[1].handle, None, "the dots never hold a window");
    }

    #[test]
    fn strip_cell_at_addresses_every_tile_and_nothing_else() {
        // The pointer addresses the strip's geometry: every tile cell
        // maps to its row at any y inside the card (the cell is the
        // unit), while tile gaps, the card's side padding, and the
        // space past the last tile address nothing.
        let (snap, closed, _open) = combined_fixture();
        let model = build_strip(&snap, &closed, 0, 4, 8).expect("builds");
        let scale = 2u32;
        let tile_center_x = |index: u32| {
            (TILE_PAD_X + index * (TILE_W + TILE_GAP) + TILE_W / 2) as f64 * scale as f64
        };
        let mid_y = band_card_height() as f64 / 2.0 * scale as f64;
        assert_eq!(model.cell_at(tile_center_x(0), mid_y, scale), Some(0));
        assert_eq!(model.cell_at(tile_center_x(1), mid_y, scale), Some(1));
        assert_eq!(model.cell_at(tile_center_x(2), mid_y, scale), Some(2));
        assert_eq!(
            model.cell_at(tile_center_x(3), mid_y, scale),
            None,
            "past the last tile"
        );
        let gap_x = (TILE_PAD_X + TILE_W + TILE_GAP / 2) as f64 * scale as f64;
        assert_eq!(model.cell_at(gap_x, mid_y, scale), None, "the gap seam");
        assert_eq!(
            model.cell_at((TILE_PAD_X / 2) as f64 * scale as f64, mid_y, scale),
            None,
            "the card's side padding"
        );
    }

    #[test]
    fn strip_model_shows_every_window_once() {
        // The duplicate-tile bug's model-level regression: the strip's
        // tiles never hold the same window twice — a single-window app
        // is one leaf tile, no run below it.
        let (snap, closed, _open) = combined_fixture();
        let model = build_strip(&snap, &closed, 3, 3, 8).expect("builds");
        let mut handles: Vec<Handle> = model.rows.iter().filter_map(|row| row.handle).collect();
        handles.sort();
        handles.dedup();
        assert_eq!(
            handles.len(),
            model.rows.len(),
            "every tile a distinct window"
        );
    }

    #[test]
    fn strip_render_paints_the_selection_in_place() {
        // The paint contract, one band over: the selected window tile
        // fills at its slot in the strip; the unselected tiles stay the
        // card.
        let (snap, _closed, open) = combined_fixture();
        let model = build_strip(&snap, &open, 1, 4, 8).expect("builds");
        let scale = 2u32;
        let lw = model.width;
        let lh = model.height;
        let mut canvas = vec![0u8; (lw * scale * lh * scale * 4) as usize];
        render(
            &model,
            &mut canvas,
            lw * scale,
            lh * scale,
            lw * scale * 4,
            scale,
            &Palette::dark(),
            TextBackend::Bitmap,
        );
        let px = |lx: u32, ly: u32| {
            let x = lx * scale;
            let y = ly * scale;
            let index = (y * lw * scale + x) as usize * 4;
            u32::from_le_bytes(canvas[index..index + 4].try_into().expect("pixel"))
        };
        let band = Palette::dark().selected_bg;
        let background = Palette::dark().background;
        // The fill probe reads the cell's mid-height at the icon's
        // inset — ink-free ground on both tiles (the icon's own center
        // carries the fallback glyph's strokes since the compact
        // re-pin moved the glyph under the old probe).
        let mid_cell = TILE_PAD_Y + TILE_CELL_H / 2;
        let inset_x = |index: u32| TILE_PAD_X + index * (TILE_W + TILE_GAP) + TILE_TEXT_PAD / 2;
        assert_eq!(
            px(inset_x(0), mid_cell),
            band,
            "the selected window tile fills at its slot"
        );
        assert_eq!(
            px(inset_x(1), mid_cell),
            background,
            "the unselected window tile stays the card"
        );
    }

    // --- the System text backend (typography TY3) -------------------------

    /// The committed fixture's text system — a fresh database holding
    /// only `res/test-fonts`, never the host's fonts.
    fn fixture_text() -> crate::text::TextSystem {
        crate::text::TextSystem::from_dir(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("res/test-fonts"),
        )
    }

    /// A one-window snapshot titled `title`, active on workspace 1.
    fn titled_snapshot(title: &str) -> Snapshot {
        let toplevels = vec![toplevel(
            1,
            title,
            "app.a",
            &[1],
            switcher::State::ACTIVATED,
            1,
        )];
        snapshot(toplevels, Some(1))
    }

    /// The card-width formula's ceiling — the title band can never
    /// exceed `MAX_COLS × TEXT_W`, so the card can never either.
    fn card_width_ceiling() -> u32 {
        2 * PAD_X + ICON + ICON_GAP + MAX_COLS * TEXT_W + HINT_GAP + HINT_COLS * TEXT_W
    }

    #[test]
    fn system_titles_stay_unfolded_and_budget_inside_the_band() {
        let snap = titled_snapshot(&"W".repeat(64));
        let mut text = fixture_text();
        let model = overlay_model(
            &snap,
            Handle(1),
            Scope::CurrentWorkspace,
            &no_icons(),
            2,
            &Palette::dark(),
            StyleProfile::default(),
            Windowing {
                rows: DEFAULT_ROWS,
                width_floor: None,
                prev: None,
            },
            TextBackend::System {
                system: &mut text,
                query: crate::text::FontQuery::SansSerif,
            },
        )
        .expect("eligible windows exist");
        let title = &model.rows[0].title;
        assert!(
            title.ends_with('\u{2026}'),
            "the cut marker is the font's own ellipsis"
        );
        assert!(title.chars().count() < 64, "the budget cut something");
        assert!(
            model.width <= card_width_ceiling(),
            "the System card never exceeds the formula's ceiling"
        );
        assert!(
            model.width > 2 * PAD_X + ICON + ICON_GAP,
            "the title band still holds ink"
        );
    }

    #[test]
    fn system_titles_render_unicode_verbatim() {
        // The fold is the Bitmap floor's degradation, not the System
        // path's: accents and CJK ride the title string un-folded (the
        // fixture db carries Latin; the CJK run shapes through the
        // empty fallback to nothing — the string itself is the pin).
        let snap = titled_snapshot("café — naïve.txt");
        let mut text = fixture_text();
        let model = overlay_model(
            &snap,
            Handle(1),
            Scope::CurrentWorkspace,
            &no_icons(),
            2,
            &Palette::dark(),
            StyleProfile::default(),
            Windowing {
                rows: DEFAULT_ROWS,
                width_floor: None,
                prev: None,
            },
            TextBackend::System {
                system: &mut text,
                query: crate::text::FontQuery::SansSerif,
            },
        )
        .expect("eligible windows exist");
        assert_eq!(model.rows[0].title, "café — naïve.txt");
    }

    #[test]
    fn system_render_paints_real_glyphs_deterministically() {
        let snap = titled_snapshot("Airlock — the fixture face");
        let mut composer = fixture_text();
        let model = overlay_model(
            &snap,
            Handle(1),
            Scope::CurrentWorkspace,
            &no_icons(),
            2,
            &Palette::dark(),
            StyleProfile::default(),
            Windowing {
                rows: DEFAULT_ROWS,
                width_floor: None,
                prev: None,
            },
            TextBackend::System {
                system: &mut composer,
                query: crate::text::FontQuery::SansSerif,
            },
        )
        .expect("eligible windows exist");

        let render_system = |system: &mut crate::text::TextSystem| -> Vec<u8> {
            let (w, h) = (model.width * 2, model.height * 2);
            let stride = w * 4;
            let mut canvas = vec![0u8; (stride * h) as usize];
            render(
                &model,
                &mut canvas,
                w,
                h,
                stride,
                2,
                &Palette::dark(),
                TextBackend::System {
                    system,
                    query: crate::text::FontQuery::SansSerif,
                },
            );
            canvas
        };
        let first = render_system(&mut fixture_text());
        let second = render_system(&mut fixture_text());
        assert_eq!(first, second, "same model, same bytes, fresh systems");

        // Real ink: full-coverage glyph cores paint the exact text
        // color inside the title band. (A Bitmap render of this
        // un-folded title would fold the em dash and mark — the
        // backends differ by design.)
        let stride = model.width * 2 * 4;
        let title_x = (PAD_X + ICON + ICON_GAP) * 2;
        let title_edge = (model.width - (PAD_X + HINT_GAP + HINT_COLS * TEXT_W)) * 2;
        let (band_top, band_bottom) = (PAD_Y * 2, (PAD_Y + ROW_H) * 2);
        let mut ink = 0;
        for y in band_top..band_bottom {
            for x in title_x..title_edge {
                if pixel(&first, stride, x, y) == COLOR_TEXT {
                    ink += 1;
                }
            }
        }
        assert!(
            ink > 20,
            "the fixture face paints real glyph cores ({ink} px)"
        );
    }

    #[test]
    fn system_ink_never_crosses_into_the_hint_gap() {
        // A budget-length title is the worst case: its ink must stay
        // inside the title band so the hint's territory is its own —
        // the guard paint_run carries, proven on the strip that has
        // nothing else painting into it.
        let snap = titled_snapshot(&"W".repeat(64));
        let mut composer = fixture_text();
        let model = overlay_model(
            &snap,
            Handle(1),
            Scope::CurrentWorkspace,
            &no_icons(),
            2,
            &Palette::dark(),
            StyleProfile::default(),
            Windowing {
                rows: DEFAULT_ROWS,
                width_floor: None,
                prev: None,
            },
            TextBackend::System {
                system: &mut composer,
                query: crate::text::FontQuery::SansSerif,
            },
        )
        .expect("eligible windows exist");
        let mut system = fixture_text();
        let (w, h) = (model.width * 2, model.height * 2);
        let stride = w * 4;
        let mut canvas = vec![0u8; (stride * h) as usize];
        render(
            &model,
            &mut canvas,
            w,
            h,
            stride,
            2,
            &Palette::dark(),
            TextBackend::System {
                system: &mut system,
                query: crate::text::FontQuery::SansSerif,
            },
        );
        let gap_x0 = (model.width - (PAD_X + HINT_GAP + HINT_COLS * TEXT_W)) * 2;
        let gap_x1 = (model.width - (PAD_X + HINT_COLS * TEXT_W)) * 2;
        for y in (PAD_Y * 2)..((PAD_Y + ROW_H) * 2) {
            for x in gap_x0..gap_x1 {
                let argb = pixel(&canvas, stride, x, y);
                assert_ne!(
                    argb, COLOR_TEXT,
                    "title ink crossed into the hint gap at ({x}, {y})"
                );
            }
        }
    }

    #[test]
    fn paint_run_skips_every_pixel_past_its_edge() {
        // The band guard on its own, at the helper level: a run painted
        // against a tight edge leaves every pixel beyond it untouched.
        let mut system = fixture_text();
        let run = system.shape(
            "WWWWWWWWWW",
            crate::text::FontQuery::SansSerif,
            2,
            crate::text::TEXT_PX,
        );
        assert!(run.width_px > 8.0, "the run genuinely overflows the edge");
        let (w, h) = (64u32, 24u32);
        let stride = w * 4;
        let sentinel = 0xFA_11_22_33u32;
        let mut canvas = vec![0u8; (stride * h) as usize];
        for slot in canvas.as_chunks_mut::<4>().0 {
            slot.copy_from_slice(&sentinel.to_le_bytes());
        }
        paint_run(
            &mut system,
            &run,
            &mut canvas,
            stride,
            2,
            COLOR_TEXT,
            0,
            12,
            8, // the edge: 8 logical px = 16 device
        );
        for y in 0..h {
            for x in 16..w {
                assert_eq!(
                    pixel(&canvas, stride, x, y),
                    sentinel,
                    "a pixel past the edge changed at ({x}, {y})"
                );
            }
        }
        let mut inked = 0;
        for y in 0..h {
            for x in 0..16 {
                if pixel(&canvas, stride, x, y) != sentinel {
                    inked += 1;
                }
            }
        }
        assert!(inked > 10, "ink landed inside the edge ({inked} px)");
    }
}
