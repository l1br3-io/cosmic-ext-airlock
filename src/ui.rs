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
pub const ROW_H: u32 = ICON + 2 * ROW_PAD_Y;

/// Vertical padding inside one row.
pub const ROW_PAD_Y: u32 = 8;

/// Gap between rows; selection bands never merge across it.
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
/// arcs. 2026-11-07 layout pass, preview-frozen beside [`PAD_Y`].
pub const BAND_RADIUS: u32 = 8;

/// Horizontal padding inside the card.
pub const PAD_X: u32 = 20;

/// Vertical padding inside the card; rows start and end inside it.
/// Double the row gap: the card edge breathes one deliberate step above
/// the row rhythm, so an edge selection reads as held by the container
/// rather than parked beside its neighbor. Also keeps `PAD_Y ≥ RADIUS`
/// with real slack — 12 sat at exact equality, one padding trim away
/// from clipping a corner arc. 2026-11-07 layout pass (16 = 2 × the
/// 8-px row gap).
pub const PAD_Y: u32 = 16;

/// Card corner radius in logical pixels. COSMIC's house radius for cards
/// is `radius_s + 4` ≈ 12 (research record §2.6). Corners are coverage
/// anti-aliased by area sampling — see [`shape_cov`] — not aliased steps;
/// nothing is blurred.
pub const RADIUS: u32 = 12;

/// Hairline card border thickness in logical pixels.
pub const BORDER: u32 = 1;

/// Maximum title columns shown before elision. 48 columns of 12-px text
/// keeps the title territory at 660 logical px — within reach of
/// cosmic-launcher's 596 fixed switcher width and the cross-platform
/// consensus cap of ~600 (research record §2.6, §2.10); the full card,
/// hint band included, is 748. Re-pinned 2026-11-07 with the 12-px text
/// from the 8-px era's 64 columns (596 territory, 660 card): the band
/// consensus holds, the column budget pays for the larger glyphs. The
/// hint band rides on top of the cap rather than under it (rev 2's
/// recorded decision: the card widens by the band, the title budget does
/// not fund it).
pub const MAX_COLS: u32 = 48;

/// Gap between the title band's right edge and the hint band, logical
/// px. The hint's territory is its own, never the title's — a
/// max-columns title still clears it by the full gap. Two 8-grid
/// steps; judged beside a 64-column title on the preview sheet
/// (2026-11-04) before freezing.
pub const HINT_GAP: u32 = 16;

/// Reserved hint columns (the longest label, `Ctrl+9`). Structural:
/// every card carries the band regardless of row count; only rows 1..9
/// paint in it.
pub const HINT_COLS: u32 = 6; // len("Ctrl+9")

/// Indent of a grouped child row's whole cell — icon and title — from
/// the top-level column, logical px: two 8-grid steps, deep enough to
/// read as nesting at a glance without re-parenting the row rhythm (the
/// band still spans the card; only the content indents). Frozen
/// 2026-11-08 on the GR4 sheet, both palettes over both backdrops (the
/// LV3 pattern).
pub const TREE_INDENT: u32 = 16;

/// Title-band columns reserved after a parent row's title for the
/// count chip: a two-digit chip (2 × `TEXT_W` + 2 × [`BADGE_PAD_X`])
/// plus its [`BADGE_GAP`] clearing fits with columns to spare.
/// Re-pinned 2026-11-08 (owner, in-thread) when the muted `×N` text
/// became the chip — the budget widened from 3 columns so the chip
/// always shows beside even a max-length title.
pub const BADGE_COLS: u32 = 4;

/// The count chip's geometry, logical px: height on the 8-grid (two
/// steps), horizontal padding around the digit, the clearing between
/// title end and chip, and the corner radius — half the height, the
/// pill the selection band's [`BAND_RADIUS`] rounding already
/// established. The chip fills with the text color and cuts its digit
/// in the card's own background: the palette's pair, so it reads on
/// both polarities without a new color. Frozen 2026-11-08 on the GR4
/// sheet: the pill reads at metadata weight beside the 32-px icons,
/// and the one-step [`BADGE_GAP`] clearing holds — widening it would
/// spend a title column on every parent ([`BADGE_COLS`] grows with
/// it), a trade the sheet judged not worth the nit.
pub const BADGE_H: u32 = 16;
/// See [`BADGE_H`]: horizontal padding around the digit, logical px.
pub const BADGE_PAD_X: u32 = 4;
/// See [`BADGE_H`]: clearing between the title's last column and the
/// chip, logical px — one 8-grid step.
pub const BADGE_GAP: u32 = 8;
/// See [`BADGE_H`]: corner radius — half the height, a full pill.
pub const BADGE_RADIUS: u32 = 8;

/// The disclosure chevron's draw scale: the 8×8 solid-triangle glyphs
/// paint at 2× = 16 logical px — exactly the [`HINT_GAP`] column the
/// chevron owns — centered on the row's midline, in the full text
/// color. Re-pinned 2026-11-08 (owner, in-thread): the muted `>`/`v`
/// text glyphs were not noticeable; a solid triangle is the disclosure
/// vocabulary the icon rows already speak (the [`WINDOW_GLYPH`]
/// family), and 16 px holds its weight beside 12-px text without
/// competing with the 32-px icons.
pub const CHEVRON_SCALE: u32 = 2;

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

/// Card height in logical px for `slots` rows: the padding top and
/// bottom, every row, and a gap between rows only. The one place the
/// formula lives — the compile-time pins and [`overlay_model`] agree by
/// construction.
pub const fn card_height(slots: u32) -> u32 {
    2 * PAD_Y + slots * ROW_H + (slots - 1) * ROW_GAP
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

    /// The card `setting` paints: a pinned value wins regardless of the
    /// system; `Auto` follows the system's dark/light answer, and an
    /// unreadable one (`None`) paints dark — today's look, never a
    /// broken switch.
    pub fn resolve(setting: ThemeSetting, system_prefers_dark: Option<bool>) -> Palette {
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

// Geometry pins. The 8-grid holds for every stepped metric; the row is
// exactly the icon cell plus its breathing; rows stay clear of the
// corner arcs (PAD_Y ≥ RADIUS), so a selection band can never clip a
// corner, and the band's own ends round within its height; and the
// border is a hairline, not a frame.
const _: () = {
    assert!(ROW_H == ICON + 2 * ROW_PAD_Y);
    assert!(TEXT_H == GLYPH_H * FONT_SCALE_NUM / FONT_SCALE_DEN);
    assert!(TEXT_W == TEXT_H);
    // The glyph block is whole device pixels at the render-scale floor.
    assert!((TEXT_W * 2).is_multiple_of(GLYPH_W));
    // The title band stays under the ~600-px switcher-width consensus
    // (research record §2.6, §2.10); the hint band rides on top of it.
    assert!(MAX_COLS * TEXT_W <= 600);
    assert!(ROW_H >= TEXT_H);
    assert!(PAD_Y >= RADIUS);
    assert!(RADIUS.is_multiple_of(4));
    assert!(ICON.is_multiple_of(8));
    assert!(ICON_GAP.is_multiple_of(4));
    assert!(HINT_GAP.is_multiple_of(4));
    assert!(ROW_PAD_Y.is_multiple_of(4));
    assert!(ROW_GAP.is_multiple_of(4));
    assert!(PAD_X.is_multiple_of(4));
    assert!(PAD_Y.is_multiple_of(4));
    assert!(BORDER == 1);
    assert!(SELECT_INSET.is_multiple_of(4));
    assert!(SELECT_INSET > BORDER);
    assert!(SELECT_INSET < PAD_X);
    assert!(BAND_RADIUS.is_multiple_of(4));
    assert!(2 * BAND_RADIUS <= ROW_H);
    // The card at the `rows` setting's worst case — `rows + ROWS_SLACK`
    // slots — fits a 720-logical screen (1440p@2x, any 1× panel): 584 at
    // the default, 696 at the ceiling. The 11-04 540 pin moved with the
    // 11-07 re-pin (spec `overlay-density`, OQ 2).
    assert!(ROWS_MIN >= 3);
    assert!(ROWS_MIN <= DEFAULT_ROWS && DEFAULT_ROWS <= ROWS_MAX);
    assert!(card_height((DEFAULT_ROWS + ROWS_SLACK) as u32) <= 720);
    assert!(card_height((ROWS_MAX + ROWS_SLACK) as u32) <= 720);
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

/// Solid disclosure triangle, collapsed (points right) — the grouped
/// parent's expansion mark, drawn at [`CHEVRON_SCALE`] in the text
/// color. Solid, not a stroked `>`: the mark must read as structure at
/// a glance, which is weight, not line art.
const CHEVRON_RIGHT_GLYPH: [u8; 8] = [0x00, 0x06, 0x1E, 0x7E, 0x7E, 0x1E, 0x06, 0x00];

/// Solid disclosure triangle, expanded (points down) — the right
/// glyph rotated a quarter turn.
const CHEVRON_DOWN_GLYPH: [u8; 8] = [0x00, 0x7E, 0x7E, 0x3C, 0x3C, 0x18, 0x18, 0x00];

/// The number chord a row advertises in its hint band — the single
/// source of "which N selects this row" for both the renderer (the
/// painted label) and the input map (the wired key): Ctrl+N addresses
/// level-1 entries, Alt+N a group's children, and a row with no hint
/// (`None`, rows past nine, continuation rows) is not number-addressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowHint {
    /// `Ctrl+N` — the Nth level-1 entry (1-based), flat rows included
    /// (the flat card's every-row numbering is the same rule over a
    /// one-level tree).
    Ctrl(usize),
    /// `Alt+N` — the Nth child (1-based) of the group this row's card
    /// block shows.
    Alt(usize),
}

impl RowHint {
    /// The Ctrl+N chord for the level-1 entry at `index` (0-based):
    /// `None` past the ninth — the tenth entry is not number-addressed,
    /// and no chord is ever shown for it.
    fn ctrl(index: usize) -> Option<Self> {
        (index < MAX_HINTS).then_some(RowHint::Ctrl(index + 1))
    }

    /// The Alt+N chord for the group's child at `index` (0-based):
    /// `None` past the ninth.
    fn alt(index: usize) -> Option<Self> {
        (index < MAX_HINTS).then_some(RowHint::Alt(index + 1))
    }

    /// The label the hint band paints, right-aligned in the reserved
    /// `HINT_COLS` territory.
    fn label(self) -> String {
        match self {
            RowHint::Ctrl(n) => format!("Ctrl+{n}"),
            RowHint::Alt(n) => format!("Alt+{n}"),
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
    /// flat row. Drives the child indent, the parent's badge and
    /// chevron, and nothing else — input addressing reads `handle`.
    pub kind: RowKind,
    /// On a parent row, the app's window count the chip shows; zero
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

/// Whether a row renders as a pure divider — an expanded group's
/// parent while cycling is open (`scoped = false`, the
/// `scoped_cycling` default): it holds no selection, hints no chord,
/// and resolves no input. The model builder's hint pass and the
/// wayland boundary's row→handle pass share this one rule.
pub(crate) fn divider(kind: RowKind, expanded: bool, scoped: bool) -> bool {
    !scoped && kind == RowKind::Parent && expanded
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
        let y_px = y / scale as f64 - PAD_Y as f64;
        let pitch = (ROW_H + ROW_GAP) as f64;
        if y_px < 0.0 || y_px >= self.rows.len() as f64 * pitch {
            return None; // card padding above the first row / below the last
        }
        let row = (y_px / pitch) as usize;
        (y_px - row as f64 * pitch < ROW_H as f64).then_some(row) // gap seam
    }
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
/// the setting) and the window the previous rebuild computed, `None` at
/// open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Windowing {
    pub rows: usize,
    pub prev: Option<Viewport>,
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

/// Builds the flat overlay contents for `snapshot` with `selected`
/// highlighted, listing windows under `scope`, with app icons from
/// `icons` prepared at `scale` for the card `palette` paints, windowed
/// by `windowing` — the session's `rows` budget and the sticky viewport
/// it carries (`None` at open). Rows follow [`switcher::eligible`]
/// order (most recently activated first); every row hints its Ctrl+N
/// chord ([`RowHint::Ctrl`]).
///
/// One list, one budget: window per [`visible_window`], a continuation
/// row replacing the outermost slot at each truncated edge, and the
/// model reports the window it computed for the session to carry next.
/// Returns `None` when there is nothing to show: no active workspace,
/// no eligible windows, or a selection that is not in the eligible set.
pub fn overlay_model(
    snapshot: &Snapshot,
    selected: Handle,
    scope: Scope,
    icons: &crate::icons::Cache,
    scale: u32,
    palette: &Palette,
    windowing: Windowing,
) -> Option<OverlayModel> {
    let active = snapshot.active?;
    let entries = switcher::eligible(&snapshot.toplevels, active, scope);
    if entries.is_empty() {
        return None;
    }
    let sel = entries.iter().position(|&handle| handle == selected)?;
    let mut rows = Vec::with_capacity(entries.len());
    for (index, &handle) in entries.iter().enumerate() {
        // Unreachable in practice: the sequence draws from this same
        // snapshot.
        let toplevel = snapshot.toplevels.iter().find(|t| t.handle == handle)?;
        rows.push(RowModel {
            title: truncate_cols(
                fold_title(&display_title(&toplevel.title, &toplevel.app_id)),
                MAX_COLS,
            ),
            icon: icons.bitmap(&toplevel.app_id, scale, palette),
            selected: index == sel,
            handle: Some(handle),
            kind: RowKind::Leaf,
            count: 0,
            hint: RowHint::ctrl(index),
            expanded: false,
        });
    }
    finish_model(rows, sel, windowing)
}

/// Builds the grouped overlay contents: the session's frozen tree cut to
/// `visible` (its expansion state, liveness-filtered by the caller),
/// with the tree row `selected` highlighted. Titles and icons draw from
/// `snapshot`; the window set does not — a window that appeared after
/// the session froze never joins the card, and one that died was
/// filtered out of `visible` by the caller.
///
/// Level-1 rows (app entries and leaves) hint their Ctrl+N chord by
/// position among level 1; a child hints its Alt+N chord by position
/// within its group. A parent's badge counts its app's frozen windows
/// and its chevron states the expansion the caller passed — expanded
/// iff its children follow it in `visible`. Under open cycling
/// (`scoped = false`, the `scoped_cycling` default) an expanded entry
/// is a pure divider — [`divider`]: it hints nothing. Width derives
/// from the visible rows' widest title plus the child indent and
/// parent badge, so the card is stable under stepping and breathes
/// only when the user expands or collapses. Returns `None` when
/// nothing is visible or the selection is not on a visible row.
// Nine render inputs, each a distinct concern (tree, visible window,
// selection, cycle scope, snapshot, icons, scale, colors, windowing) —
// the flat builder's seam, one level deeper.
#[expect(clippy::too_many_arguments)]
pub fn overlay_model_grouped(
    tree: &switcher::Grouping,
    visible: &[usize],
    selected: usize,
    scoped: bool,
    snapshot: &Snapshot,
    icons: &crate::icons::Cache,
    scale: u32,
    palette: &Palette,
    windowing: Windowing,
) -> Option<OverlayModel> {
    let sel = visible.iter().position(|&index| index == selected)?;
    let mut rows = Vec::with_capacity(visible.len());
    // The hint ordinals: level-1 position (Ctrl) and position within
    // the open group (Alt) — the parent row resets the child count.
    let mut level1 = 0;
    let mut child = 0;
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
                rows.push(RowModel {
                    title: truncate_cols(
                        fold_title(&display_title(&toplevel.title, &toplevel.app_id)),
                        MAX_COLS - TREE_INDENT / TEXT_W,
                    ),
                    icon: icons.bitmap(&toplevel.app_id, scale, palette),
                    selected: index == selected,
                    handle: Some(grow.handle),
                    kind: RowKind::Child,
                    count: 0,
                    hint: RowHint::alt(child),
                    expanded: false,
                });
                child += 1;
            }
            kind => {
                let ordinal = level1;
                level1 += 1;
                child = 0;
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
                    // follow-up); the badge counts the app's frozen
                    // windows, expanded state included in the card
                    // only through the chevron. Under open cycling an
                    // expanded entry is a divider — [`divider`]: it
                    // still counts toward the Ctrl+N ordinal (the
                    // chord works), it just says nothing.
                    let hint = if divider(RowKind::Parent, expanded, scoped) {
                        None
                    } else {
                        RowHint::ctrl(ordinal)
                    };
                    rows.push(RowModel {
                        title: truncate_cols(fold_title(&grow.app_id), MAX_COLS - BADGE_COLS),
                        icon: icons.bitmap(&grow.app_id, scale, palette),
                        selected: index == selected,
                        handle: Some(grow.handle),
                        kind: RowKind::Parent,
                        count: tree.children_of(index).len(),
                        hint,
                        expanded,
                    });
                } else {
                    rows.push(RowModel {
                        title: truncate_cols(
                            fold_title(&display_title(&toplevel.title, &toplevel.app_id)),
                            MAX_COLS,
                        ),
                        icon: icons.bitmap(&toplevel.app_id, scale, palette),
                        selected: index == selected,
                        handle: Some(grow.handle),
                        kind: RowKind::Leaf,
                        count: 0,
                        hint: RowHint::ctrl(ordinal),
                        expanded: false,
                    });
                }
            }
        }
    }
    finish_model(rows, sel, windowing)
}

/// The builders' shared tail: widest-row sizing, the windowing rule, and
/// the continuation rows at truncated edges. `sel` indexes `rows`.
fn finish_model(rows: Vec<RowModel>, sel: usize, windowing: Windowing) -> Option<OverlayModel> {
    let cols = rows
        .iter()
        .map(|row| {
            row.title.chars().count() as u32
                + (row.kind == RowKind::Child) as u32 * (TREE_INDENT / TEXT_W)
                + (row.kind == RowKind::Parent) as u32 * BADGE_COLS
        })
        .max()
        .unwrap_or(0);
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
    Some(OverlayModel {
        rows: windowed,
        width: 2 * PAD_X + ICON + ICON_GAP + cols * TEXT_W + HINT_GAP + HINT_COLS * TEXT_W,
        // The row gap lives between rows only; the bottom edge mirrors
        // the top.
        height: card_height(count),
        viewport,
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
/// `palette`.
///
/// Paint order, back to front: card fill through the rounded-corner
/// coverage mask, selection band per selected row, title glyphs, app
/// icons, and the border hairline last so the stroke reads above the
/// selection. Every device pixel inside `width`/`height` is written;
/// coordinates outside are skipped, so an oversized canvas is harmless.
pub fn render(
    model: &OverlayModel,
    canvas: &mut [u8],
    width: u32,
    height: u32,
    stride: u32,
    scale: u32,
    palette: &Palette,
) {
    // Unreachable in production (render_scale clamps with max(2, ·)),
    // but render is pub: fail loudly in debug, clamp in release, never
    // divide by zero.
    debug_assert!(scale > 0, "render scale must be positive");
    let scale = scale.max(1);
    let lw = width / scale;
    let lh = height / scale;
    // Coverage masks on the logical grid, computed once per render:
    // the card shape and the same shape eroded by the border width
    // (their difference is the border ring).
    let mut outer = vec![0u8; (lw * lh) as usize];
    for ly in 0..lh {
        for lx in 0..lw {
            outer[(ly * lw + lx) as usize] = shape_cov(lx, ly, lw, lh, RADIUS, 0);
        }
    }
    // The eroded mask is only read near the card edges (the border
    // pass), so computing the interior is wasted work at Tab speed.
    let mut inner = vec![0u8; (lw * lh) as usize];
    let reach = RADIUS + BORDER;
    for ly in 0..lh {
        let spans = edge_spans(ly, lh, lw, reach);
        for (x0, x1) in spans {
            for lx in x0..x1 {
                inner[(ly * lw + lx) as usize] = shape_cov(lx, ly, lw, lh, RADIUS, BORDER);
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
    for (index, row) in model.rows.iter().enumerate() {
        let top = PAD_Y + index as u32 * (ROW_H + ROW_GAP);
        // Continuation row: the `…` glyph card-centered on the titles'
        // own line, in the muted derivation — the hint color, quieter
        // than any title and legible on both card fills and both
        // wallpaper polarities. Nothing else paints on this row: no
        // band, no icon, no title, no hint — a row beyond addressability
        // earns nothing but its dots. The glyph paints `TEXT_W` wide, so
        // its box centers on `TEXT_W` exactly as `dots_y` centers on
        // `TEXT_H`; centering on the font cell biased the ink 2 px right
        // of the card's center line. Card-centered is the candidate
        // placement; the preview sheet freezes it (LV3).
        if row.handle.is_none() {
            let dots_x = lw.saturating_sub(TEXT_W) / 2;
            let dots_y = top + (ROW_H - TEXT_H) / 2;
            draw_glyph(
                canvas,
                stride,
                ELLIPSIS_GLYPH,
                dots_x * scale,
                dots_y * scale,
                TEXT_W * scale / GLYPH_W,
                palette.muted,
            );
            continue;
        }
        if row.selected {
            // The band is its own rounded rect: `BAND_RADIUS` ends
            // through the same coverage sampler as the card corners.
            // The ramp interpolates toward the band fill between two
            // painted fills — full coverage is the exact band color, zero
            // coverage leaves the card untouched, and because both are
            // near-opaque the arc never thins the card. `PAD_Y ≥ RADIUS`
            // holds the band clear of the card's corner arcs, so the
            // card mask doesn't ride along.
            let band_w = lw.saturating_sub(2 * SELECT_INSET);
            for ly in top..top + ROW_H {
                for dy in 0..scale {
                    let y = ly * scale + dy;
                    for lx in SELECT_INSET..lw.saturating_sub(SELECT_INSET) {
                        let cov =
                            shape_cov(lx - SELECT_INSET, ly - top, band_w, ROW_H, BAND_RADIUS, 0);
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
        let cell_y = top + (ROW_H - ICON) / 2;
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
        // Title: vertically centered against the row, stopping at the
        // title band's right edge — the hint's territory is its own, so
        // even a max-cols title can never touch a hint (the model sizes
        // for `cols`; this guard is the defense in depth).
        let text_y = top + (ROW_H - TEXT_H) / 2;
        let title_edge = lw.saturating_sub(PAD_X + HINT_GAP + HINT_COLS * TEXT_W);
        let mut lx = cell_x + ICON + ICON_GAP;
        for ch in row.title.chars() {
            if lx + TEXT_W > title_edge {
                break;
            }
            draw_glyph(
                canvas,
                stride,
                glyph(ch),
                lx * scale,
                text_y * scale,
                TEXT_W * scale / GLYPH_W,
                palette.text,
            );
            lx += TEXT_W;
        }
        // Parent count chip and disclosure chevron — re-pinned
        // 2026-11-08 (owner, in-thread) for prominence after the muted
        // `x3` text and `>`/`v` glyphs proved unnoticeable. The chip is
        // a pill filled with the text color, the bare window count cut
        // out in the card's own background — the palette's pair, so it
        // reads on both polarities with no new color — one BADGE_GAP
        // clear of the title, inside the BADGE_COLS budget the model
        // reserves for it (the fit guard is defense in depth, the
        // title guard's precedent). The chevron is a solid triangle in
        // the same full text color — right when collapsed, down when
        // expanded — CHEVRON_SCALE-sized on the row's midline, in the
        // hint-gap column every card already reserves. Leaves and
        // children earn neither.
        if row.kind == RowKind::Parent {
            let digits: Vec<char> = row.count.to_string().chars().take(2).collect();
            let chip_w = digits.len() as u32 * TEXT_W + 2 * BADGE_PAD_X;
            let chip_x = lx + BADGE_GAP;
            let chip_y = top + (ROW_H - BADGE_H) / 2;
            if chip_x + chip_w <= title_edge {
                // The pill: exact rounded-rect coverage, the same
                // sampler as the selection band and the card corners —
                // full coverage is the exact text color over whatever
                // sits beneath (band or card).
                for ly in chip_y..chip_y + BADGE_H {
                    for dy in 0..scale {
                        let y = ly * scale + dy;
                        for cx in chip_x..chip_x + chip_w {
                            let cov = shape_cov(
                                cx - chip_x,
                                ly - chip_y,
                                chip_w,
                                BADGE_H,
                                BADGE_RADIUS,
                                0,
                            );
                            if cov == 0 {
                                continue;
                            }
                            let x0 = cx * scale;
                            for x in x0..x0 + scale {
                                let index = y as usize * stride as usize + x as usize * 4;
                                if let Some(slot) = canvas.get_mut(index..index + 4) {
                                    let dst = u32::from_le_bytes(
                                        slot.try_into().expect("pixel is four bytes"),
                                    );
                                    let out = mix_pixel(dst, palette.text, cov);
                                    slot.copy_from_slice(&out.to_le_bytes());
                                }
                            }
                        }
                    }
                }
                // The count digit, centered in the chip and cut out in
                // the card's own background.
                let digit_x = chip_x + (chip_w - digits.len() as u32 * TEXT_W) / 2;
                let digit_y = chip_y + (BADGE_H - TEXT_H) / 2;
                let mut dx = digit_x;
                for ch in digits {
                    draw_glyph(
                        canvas,
                        stride,
                        glyph(ch),
                        dx * scale,
                        digit_y * scale,
                        TEXT_W * scale / GLYPH_W,
                        palette.background,
                    );
                    dx += TEXT_W;
                }
            }
            let chevron = if row.expanded {
                CHEVRON_DOWN_GLYPH
            } else {
                CHEVRON_RIGHT_GLYPH
            };
            let side = GLYPH_W * CHEVRON_SCALE;
            let chevron_x = title_edge + (HINT_GAP - side) / 2;
            let chevron_y = top + (ROW_H - side) / 2;
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
        // Hint: the row's own number chord, right-aligned in the band
        // the card always reserves and vertically centered on the
        // titles' own line. Rows past nine paint nothing — every
        // displayed hint is a wired key (the model's [`RowHint`] is
        // that truth's single source, paired with `key_input`).
        if let Some(label) = row.hint.map(RowHint::label) {
            let band_right = lw.saturating_sub(PAD_X);
            let mut hx = band_right.saturating_sub(label.chars().count() as u32 * TEXT_W);
            for ch in label.chars() {
                if hx + TEXT_W > band_right {
                    break; // a degenerate canvas: never paint past the card
                }
                draw_glyph(
                    canvas,
                    stride,
                    glyph(ch),
                    hx * scale,
                    text_y * scale,
                    TEXT_W * scale / GLYPH_W,
                    palette.muted,
                );
                hx += TEXT_W;
            }
        }
    }
    // Border ring last: inside the card but not fully inside the eroded
    // shape, so the stroke follows the rounded corners with the same
    // anti-aliasing. The difference-of-masks formulation could open gaps
    // where both masks quantize alike on an arc; this rule cannot.
    // The ring lives only within RADIUS + BORDER of the card boundary —
    // scanning the interior is wasted work at Tab speed.
    for ly in 0..lh {
        for (x0, x1) in edge_spans(ly, lh, lw, RADIUS + BORDER) {
            for lx in x0..x1 {
                let index = (ly * lw + lx) as usize;
                if outer[index] == 0 || inner[index] == 255 {
                    continue;
                }
                let argb = coverage_argb(palette.border, outer[index]);
                for dy in 0..scale {
                    for dx in 0..scale {
                        put_pixel(canvas, stride, lx * scale + dx, ly * scale + dy, argb);
                    }
                }
            }
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
/// app_id, else a placeholder.
fn display_title(title: &str, app_id: &str) -> String {
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
/// the parent badge out of the same budget, so the card's width ceiling
/// holds in the tree too.
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
    /// Grouped mode's Alt+1..9: address the Nth child (0-based) of the
    /// group the selection is on — only while that group is expanded
    /// (an unshown child is never hinted, so never wired). The boundary
    /// resolves it against the event-time model.
    SelectAlt(usize),
    /// The handle-carrying twin of the select family, produced only
    /// by the wayland boundary at event receipt: the window of the row
    /// the user addressed. `None` — a continuation row, or a row past
    /// the model's end — flows through and no-ops at the daemon,
    /// leaving the session intact.
    SelectHandle(Option<Handle>),
    /// Grouped mode's Tab / Shift+Tab (Alt+Tab / Alt+Shift+Tab): the
    /// scoped step — level 1 cycles app entries, level 2 cycles the
    /// group's children. Never descends from level 1 by itself except
    /// the single-group workspace, where Tab enters and cycles inside
    /// the only group there is.
    TabScope,
    /// Grouped mode's Alt+W: the level toggle — enters the selected
    /// group's children at level 1, exits to the app entry at level 2.
    /// → is [`GestureKey::EnterChild`]'s enter-only twin; W works in
    /// both directions.
    ToggleLevel,
    /// → in grouped mode: expand the selected parent and enter its
    /// children (the W3C expand axis). The daemon resolves the target
    /// against the frozen tree.
    EnterChild,
    /// ← in grouped mode: exit to the selected child's parent row, or
    /// collapse the selected expanded parent (the W3C collapse axis).
    /// The daemon resolves the target against the frozen tree.
    ExitParent,
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
    /// Grouped mode's Alt+Tab: step to the next/previous *peer* — the
    /// next app entry at level 1, the next window of the same group at
    /// level 2 — and refresh. The daemon resolves the walk against the
    /// session's frozen tree; flat sessions never see this output.
    StepScope(Direction),
    /// Grouped mode's Alt+W: flip one level — enter the selected
    /// group's children at level 1, exit to its app entry at level 2.
    /// The daemon resolves against the frozen tree.
    ToggleLevel,
    /// → in grouped mode: expand the selected parent and move the
    /// selection to its enter-target child. The daemon resolves against
    /// the frozen tree; a leaf's enter is the W3C no-op.
    EnterChild,
    /// ← in grouped mode: move the selection to the selected child's
    /// parent row, or collapse the selected expanded parent. The daemon
    /// resolves against the frozen tree; a collapsed top-level row's
    /// exit is the W3C no-op.
    ExitParent,
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
                // Grouped mode's key vocabulary. The machine cannot
                // resolve any of them — the targets live in the daemon's
                // frozen tree and expansion state — so it forwards them
                // as outputs exactly as it forwards arrows.
                GestureKey::TabScope => GestureOutput::StepScope(if shift {
                    Direction::Previous
                } else {
                    Direction::Next
                }),
                GestureKey::ToggleLevel => GestureOutput::ToggleLevel,
                GestureKey::EnterChild => GestureOutput::EnterChild,
                GestureKey::ExitParent => GestureOutput::ExitParent,
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
                GestureKey::Select(_) | GestureKey::SelectApp(_) | GestureKey::SelectAlt(_) => {
                    GestureOutput::Nothing
                }
                GestureKey::Escape => {
                    self.showing = false;
                    GestureOutput::Dismiss
                }
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
/// states — Ctrl classifies digit presses, Alt classifies W and the
/// grouped digit row, exactly as shift classifies Tab.
///
/// Grouped mode (the owner's 2026-11-07 collapse re-decision) re-binds
/// the axes per the W3C vertical treeview: Alt+Tab / Alt+Shift+Tab
/// ([`GestureKey::TabScope`]) cycle the current level — app entries on
/// level 1, one group's windows on level 2 — never crossing levels
/// except the single-group workspace; → expands and enters
/// ([`GestureKey::EnterChild`]), ← exits or collapses
/// ([`GestureKey::ExitParent`]); Alt+W
/// ([`GestureKey::ToggleLevel`]) flips one level either way; Ctrl+N
/// addresses level-1 entries ([`GestureKey::SelectApp`]) and Alt+N the
/// selected group's children ([`GestureKey::SelectAlt`]). Flat mode
/// (`grouped = false`) keeps the flat mapping byte for byte — Tab and
/// ←/→ step rows — pinned by the flat regression tests. The boundary
/// passes the live model's own mode flag, so the keysym the user
/// pressed maps against the list they see.
pub fn key_input(
    keysym: Keysym,
    shift_held: bool,
    ctrl_held: bool,
    alt_held: bool,
    grouped: bool,
) -> GestureInput {
    let (key, shift) = match keysym {
        Keysym::ISO_Left_Tab => (
            if grouped {
                GestureKey::TabScope
            } else {
                GestureKey::Tab
            },
            true,
        ),
        Keysym::Tab => (
            if grouped {
                GestureKey::TabScope
            } else {
                GestureKey::Tab
            },
            shift_held,
        ),
        Keysym::Return | Keysym::KP_Enter | Keysym::ISO_Enter => (GestureKey::Commit, shift_held),
        Keysym::Up => (GestureKey::Arrow(Direction::Previous), false),
        Keysym::Down => (GestureKey::Arrow(Direction::Next), false),
        Keysym::Left => {
            if grouped {
                (GestureKey::ExitParent, shift_held)
            } else {
                (GestureKey::Arrow(Direction::Previous), false)
            }
        }
        Keysym::Right => {
            if grouped {
                (GestureKey::EnterChild, shift_held)
            } else {
                (GestureKey::Arrow(Direction::Next), false)
            }
        }
        // Alt+W flips one level — down into the children at level 1,
        // back out at level 2 — for hands that never leave the Alt row.
        // Both cases ride one arm: Shift+Alt+W reports the W keysym.
        Keysym::w | Keysym::W if alt_held && grouped => (GestureKey::ToggleLevel, shift_held),
        Keysym::Escape => (GestureKey::Escape, shift_held),
        // Ctrl+digits map with or without shift held; without Ctrl the
        // number row stays Other — it must never surprise-activate. In
        // grouped mode the chord addresses level-1 entries, not rows.
        _ if ctrl_held && let Some(row) = digit_row(keysym) => (
            if grouped {
                GestureKey::SelectApp(row)
            } else {
                GestureKey::Select(row)
            },
            shift_held,
        ),
        // Alt+digits address the selected group's children — grouped
        // only; the flat card never re-binds the number row.
        _ if alt_held
            && grouped
            && let Some(row) = digit_row(keysym) =>
        {
            (GestureKey::SelectAlt(row), shift_held)
        }
        _ => (GestureKey::Other, shift_held),
    };
    GestureInput::Press { key, shift }
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
            Windowing {
                rows: DEFAULT_ROWS,
                prev: None,
            },
        )
    }

    /// Renders `model` at `scale` into a fresh canvas; returns it with
    /// its stride. The dark palette is the behavior TH1 pinned; light
    /// lands in TH2.
    fn rendered(model: &OverlayModel, scale: u32) -> (Vec<u8>, u32) {
        let (w, h) = (model.width * scale, model.height * scale);
        let stride = w * 4;
        let mut canvas = vec![0u8; (stride * h) as usize];
        render(model, &mut canvas, w, h, stride, scale, &Palette::dark());
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
    /// scale — where `render` paints a row's `Ctrl+N` chord.
    fn hint_band_points(row: usize, width: u32) -> impl Iterator<Item = (u32, u32)> {
        let top = (PAD_Y + row as u32 * (ROW_H + ROW_GAP)) * 2;
        let x0 = (width - PAD_X - HINT_COLS * TEXT_W) * 2;
        let x1 = (width - PAD_X) * 2;
        (x0..x1).flat_map(move |x| (top..top + ROW_H * 2).map(move |y| (x, y)))
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
        // 12-px re-pin keeps the band under the cap (48 × 12 = 576) and
        // lands the whole card at 748 logical px — the measured pin
        // PRODUCT.md re-dates from 660.
        // (The band cap itself is a compile-time pin beside the other
        // geometry pins.)
        assert_eq!(model.width, 748, "the measured card width pin");
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
    fn row_hints_address_exactly_one_through_nine() {
        // Rows 1..9 are addressable and hinted; row 10 onward paints
        // nothing — no dead keys on the card (cosmic's own row-10 hint
        // is an unwired upstream gap Airlock does not copy). Both
        // chords ride the same nine cap: Ctrl at level 1, Alt within
        // a group.
        for (index, n) in (1..=9).enumerate() {
            assert_eq!(RowHint::ctrl(index), Some(RowHint::Ctrl(n)));
            assert_eq!(RowHint::alt(index), Some(RowHint::Alt(n)));
        }
        assert_eq!(RowHint::ctrl(9), None, "the tenth entry is not addressable");
        assert_eq!(RowHint::alt(9), None, "the tenth child is not addressable");
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
            Windowing { rows, prev },
        )
    }

    /// Like [`build`], grouped: the app tree over the same snapshot,
    /// collapsed by default — `expanded` names the apps standing open,
    /// `selected` the selected *tree row* (the daemon selects rows, not
    /// handles, in this mode), `scoped` the session's cycle scope.
    fn build_grouped(
        snap: &Snapshot,
        selected: usize,
        expanded: &[&str],
        scoped: bool,
    ) -> Option<OverlayModel> {
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
            scoped,
            snap,
            &no_icons(),
            2,
            &Palette::dark(),
            Windowing {
                rows: DEFAULT_ROWS,
                prev: None,
            },
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
        // carrying its badge, and the leaves under their window titles.
        // Expanded: the parent's whole window set follows as children,
        // front window first, each Alt-numbered within the group.
        let snap = grouped_snap();
        let collapsed = build_grouped(&snap, 0, &[], true).expect("eligible windows exist");
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
        let expanded = build_grouped(&snap, 0, &["app.a"], true).expect("eligible windows exist");
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
                (
                    "a-front",
                    crate::switcher::RowKind::Child,
                    Some(RowHint::Alt(1)),
                    false
                ),
                (
                    "a-back",
                    crate::switcher::RowKind::Child,
                    Some(RowHint::Alt(2)),
                    false
                ),
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
        let model = build_grouped(&snap, 0, &["app.a"], true).expect("eligible windows exist");
        assert_eq!(model.rows[0].handle, Some(Handle(1)));
        assert_eq!(model.rows[1].handle, Some(Handle(1)));
        assert!(model.rows[0].selected, "the parent row is the selection");
        assert!(!model.rows[1].selected);

        let model = build_grouped(&snap, 1, &["app.a"], true).expect("eligible windows exist");
        assert!(!model.rows[0].selected);
        assert!(
            model.rows[1].selected,
            "the front child row is the selection"
        );
    }

    #[test]
    fn grouped_model_open_cycling_hints_nothing_on_a_divider() {
        // Open cycling (the `scoped_cycling = false` default): an
        // expanded group's entry is a pure divider — no highlight ever
        // rests on it, so no chord is advertised on it. Everything else
        // keeps its hint: collapsed entries their Ctrl+N, children
        // their Alt+N, under either scope.
        let snap = grouped_snap();
        let model = build_grouped(&snap, 1, &["app.a"], false).expect("eligible windows exist");
        let rows: Vec<(crate::switcher::RowKind, Option<RowHint>)> =
            model.rows.iter().map(|row| (row.kind, row.hint)).collect();
        assert_eq!(
            rows,
            vec![
                (crate::switcher::RowKind::Parent, None), // the divider
                (crate::switcher::RowKind::Child, Some(RowHint::Alt(1))),
                (crate::switcher::RowKind::Child, Some(RowHint::Alt(2))),
                (crate::switcher::RowKind::Leaf, Some(RowHint::Ctrl(2))),
                (crate::switcher::RowKind::Leaf, Some(RowHint::Ctrl(3))),
            ]
        );
        // The same card under scoped cycling: the entry is a real Tab
        // target and says so.
        let model = build_grouped(&snap, 0, &["app.a"], true).expect("eligible windows exist");
        assert_eq!(model.rows[0].hint, Some(RowHint::Ctrl(1)));
        // Collapsed entries hint under either scope — they are how an
        // open-cycling Tab reaches a collapsed app.
        let model = build_grouped(&snap, 0, &[], false).expect("eligible windows exist");
        assert_eq!(model.rows[0].hint, Some(RowHint::Ctrl(1)));
    }

    #[test]
    fn divider_rule_marks_expanded_parents_only_under_open_cycling() {
        // The one rule the model builder's hint pass and the wayland
        // boundary's row→handle pass share: an expanded parent while
        // cycling is open. Scoped cycling has no dividers; neither does
        // a collapsed entry, a child, or a leaf.
        use crate::switcher::RowKind;
        assert!(divider(RowKind::Parent, true, false));
        assert!(!divider(RowKind::Parent, true, true));
        assert!(!divider(RowKind::Parent, false, false));
        assert!(!divider(RowKind::Child, true, false));
        assert!(!divider(RowKind::Leaf, true, false));
    }

    #[test]
    fn grouped_model_sizes_for_indent_and_badge() {
        // The card widens for the tree: a child's indent columns and a
        // parent's count-badge columns join the flat title budget. Same
        // title lengths everywhere isolate the tree columns.
        let snap = grouped_snap();
        let flat =
            build(&snap, Handle(1), Scope::CurrentWorkspace).expect("eligible windows exist");
        let grouped = build_grouped(&snap, 0, &["app.a"], true).expect("eligible windows exist");
        // Longest flat title is 7 cols; the tree budget is the parent's
        // 5 title cols + BADGE_COLS, the child's 7 + TREE_INDENT cols.
        let tree_cols = (5 + BADGE_COLS).max(7 + TREE_INDENT / TEXT_W).max(7);
        assert_eq!(
            grouped.width,
            2 * PAD_X + ICON + ICON_GAP + tree_cols * TEXT_W + HINT_GAP + HINT_COLS * TEXT_W
        );
        assert_eq!(flat.width, grouped.width - (tree_cols - 7) * TEXT_W);
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
            true,
            &snap,
            &no_icons(),
            2,
            &Palette::dark(),
            Windowing {
                rows: 3,
                prev: None,
            },
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
        let model =
            build_grouped(&grouped_snap(), 0, &["app.a"], true).expect("eligible windows exist");
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
    fn render_draws_a_solid_chevron_at_full_strength() {
        // The owner's 2026-11-08 prominence re-pin: the disclosure mark
        // is a solid triangle in the text color — full strength, not
        // the hints' muted derivation — CHEVRON_SCALE-sized on the
        // row's midline in the hint-gap column it already owned.
        // Collapsed points right, expanded points down; a leaf paints
        // nothing there.
        let collapsed =
            build_grouped(&grouped_snap(), 3, &[], true).expect("eligible windows exist");
        let (canvas, stride) = rendered(&collapsed, 2);
        let side = GLYPH_W * CHEVRON_SCALE;
        let chev_x = collapsed.width - PAD_X - HINT_GAP - HINT_COLS * TEXT_W;
        let chev_y = PAD_Y + (ROW_H - side) / 2;
        // Font pixel (col, row) of the chevron cell, in device px.
        let mark = |col: u32, row: u32| {
            (
                (chev_x + col * CHEVRON_SCALE) * 2,
                (chev_y + row * CHEVRON_SCALE) * 2,
            )
        };
        // The right-pointing wedge: solid along the widest rows from
        // column 1 (full strength reads as COLOR_TEXT exactly, never
        // the hints' muted blend), card at the top-right corner.
        let (x, y) = mark(1, 3);
        assert_eq!(
            pixel(&canvas, stride, x, y),
            COLOR_TEXT,
            "collapsed: solid at the widest row's left edge"
        );
        let (x, y) = mark(6, 3);
        assert_eq!(
            pixel(&canvas, stride, x, y),
            COLOR_TEXT,
            "collapsed: solid at the widest row's point"
        );
        let (x, y) = mark(6, 1);
        assert_eq!(
            pixel(&canvas, stride, x, y),
            COLOR_BACKGROUND,
            "collapsed: the top-right corner of the cell stays card"
        );
        // A leaf earns no chevron: the whole cell is card.
        let (x, y) = (mark(1, 3).0, mark(1, 3).1 + 2 * (ROW_H + ROW_GAP) * 2);
        assert_eq!(
            pixel(&canvas, stride, x, y),
            COLOR_BACKGROUND,
            "a leaf paints nothing in the chevron column"
        );

        // Expanded: the wedge points down — solid at the bottom point
        // and along the full top edge, card where the right-pointing
        // shape had its own ink and vice versa.
        let expanded =
            build_grouped(&grouped_snap(), 3, &["app.a"], true).expect("eligible windows exist");
        let (canvas, stride) = rendered(&expanded, 2);
        let (x, y) = mark(3, 6);
        assert_eq!(
            pixel(&canvas, stride, x, y),
            COLOR_TEXT,
            "expanded: solid at the down-point's row"
        );
        let (x, y) = mark(1, 1);
        assert_eq!(
            pixel(&canvas, stride, x, y),
            COLOR_TEXT,
            "expanded: solid along the full top edge"
        );
        let (x, y) = mark(1, 3);
        assert_eq!(
            pixel(&canvas, stride, x, y),
            COLOR_BACKGROUND,
            "expanded: the mid-height taper keeps the card where the right wedge was solid"
        );
    }

    #[test]
    fn render_paints_the_count_chip_after_the_title() {
        // The badge is a chip: a pill filled with the text color, the
        // bare window count digit cut out in the card's own background
        // — the palette's pair, so it reads on both polarities — a
        // BADGE_GAP clear of the title, inside the BADGE_COLS budget
        // the model reserves. No `x` prefix: the shape carries
        // "count" (the accessory this re-pin removes).
        let model = build_grouped(&grouped_snap(), 3, &[], true).expect("eligible windows exist");
        let (canvas, stride) = rendered(&model, 2);
        let title_end = PAD_X + ICON + ICON_GAP + "app.a".chars().count() as u32 * TEXT_W;
        let chip_x = title_end + BADGE_GAP;
        let chip_y = PAD_Y + (ROW_H - BADGE_H) / 2;
        let digits = 1; // the fixture's app.a holds two windows
        let chip_w = digits * TEXT_W + 2 * BADGE_PAD_X;
        let mut fill = 0;
        let mut cut = 0;
        for y in chip_y * 2..(chip_y + BADGE_H) * 2 {
            for x in chip_x * 2..(chip_x + chip_w) * 2 {
                match pixel(&canvas, stride, x, y) {
                    COLOR_TEXT => fill += 1,
                    COLOR_BACKGROUND => cut += 1,
                    _ => {}
                }
            }
        }
        assert!(
            fill > 8 * BADGE_H,
            "the chip paints a solid pill ({fill} px)"
        );
        assert!(
            cut > 2 * TEXT_W,
            "the digit cuts through the fill in the card color ({cut} px)"
        );
        // The chip never crosses the title band's edge: the pixel just
        // past the reserved budget stays card.
        let edge = model.width - PAD_X - HINT_GAP - HINT_COLS * TEXT_W;
        let budget_end = title_end + BADGE_COLS * TEXT_W;
        assert!(budget_end <= edge);
        assert_eq!(
            pixel(
                &canvas,
                stride,
                (chip_x + chip_w) * 2,
                (chip_y + BADGE_H / 2) * 2
            ),
            COLOR_BACKGROUND,
            "the chip ends inside its budget"
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
            render(&model, &mut canvas, w, h, stride, 2, &palette);
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
        let dark = Palette::dark();
        let light = Palette::light();
        for system in [Some(true), Some(false), None] {
            assert_eq!(Palette::resolve(ThemeSetting::Dark, system), dark);
            assert_eq!(Palette::resolve(ThemeSetting::Light, system), light);
        }
        assert_eq!(Palette::resolve(ThemeSetting::Auto, Some(true)), dark);
        assert_eq!(Palette::resolve(ThemeSetting::Auto, Some(false)), light);
        assert_eq!(
            Palette::resolve(ThemeSetting::Auto, None),
            dark,
            "auto + unreadable system paints today's card"
        );
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
        render(&model, &mut canvas, w, h, stride, 2, &Palette::dark());
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
        };
        let (w, h) = (model.width * 2, model.height * 2);
        let stride = w * 4;
        let mut canvas = vec![0; (stride * h) as usize];
        let start = Instant::now();
        // 30 iterations at 4× the pre-refresh pixel count ≈ twice the
        // old guard's total work (200 renders of the 8-px card).
        for _ in 0..30 {
            render(&model, &mut canvas, w, h, stride, 2, &Palette::dark());
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
    fn escape_dismisses_even_while_alt_is_held() {
        let mut gesture = Gesture::default();
        gesture.feed(GestureInput::Invoke(Direction::Next));
        gesture.feed(GestureInput::Modifiers { alt: true });
        assert_eq!(
            gesture.feed(GestureInput::Press {
                key: GestureKey::Escape,
                shift: false
            }),
            GestureOutput::Dismiss
        );
        assert!(!gesture.showing());
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
        gesture.feed(GestureInput::Press {
            key: GestureKey::Escape,
            shift: false,
        });
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
            key_input(Keysym::Tab, false, false, false, false),
            GestureInput::Press {
                key: GestureKey::Tab,
                shift: false
            }
        );
        // Shift held at the seat steers a plain Tab backwards.
        assert_eq!(
            key_input(Keysym::Tab, true, false, false, false),
            GestureInput::Press {
                key: GestureKey::Tab,
                shift: true
            }
        );
        // ISO_Left_Tab is Shift+Tab by another name.
        assert_eq!(
            key_input(Keysym::ISO_Left_Tab, false, false, false, false),
            GestureInput::Press {
                key: GestureKey::Tab,
                shift: true
            }
        );
        assert_eq!(
            key_input(Keysym::Escape, false, false, false, false),
            GestureInput::Press {
                key: GestureKey::Escape,
                shift: false
            }
        );
        assert_eq!(
            key_input(Keysym::Return, false, false, false, false),
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
                key_input(keysym, false, false, false, false),
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
                key_input(keysym, true, false, false, false),
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
            key_input(Keysym::Delete, true, false, false, false),
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
                key_input(keysym, false, true, false, false),
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
                key_input(keysym, false, true, false, false),
                GestureInput::Press {
                    key: GestureKey::Select(row),
                    shift: false
                }
            );
        }
    }

    #[test]
    fn key_input_leaves_digits_without_ctrl_inert() {
        // The number row must never surprise-activate: without Ctrl held
        // a digit is just an unknown key, top row and keypad alike.
        assert_eq!(
            key_input(Keysym::_3, false, false, false, false),
            GestureInput::Press {
                key: GestureKey::Other,
                shift: false
            }
        );
        assert_eq!(
            key_input(Keysym::KP_5, false, false, false, false),
            GestureInput::Press {
                key: GestureKey::Other,
                shift: false
            }
        );
    }

    #[test]
    fn key_input_maps_shift_ctrl_digits_too() {
        // Shift is ignored for digits: the chord still selects, and the
        // seat's shift state is carried as-is.
        assert_eq!(
            key_input(Keysym::_4, true, true, false, false),
            GestureInput::Press {
                key: GestureKey::Select(3),
                shift: true
            }
        );
    }

    #[test]
    fn key_input_leaves_ctrl_zero_and_ctrl_non_digits_alone() {
        // No Ctrl+0-as-10 (OQ 2), and Ctrl on a non-digit key is not a
        // selection.
        assert_eq!(
            key_input(Keysym::_0, false, true, false, false),
            GestureInput::Press {
                key: GestureKey::Other,
                shift: false
            }
        );
        assert_eq!(
            key_input(Keysym::KP_0, false, true, false, false),
            GestureInput::Press {
                key: GestureKey::Other,
                shift: false
            }
        );
        assert_eq!(
            key_input(Keysym::Delete, false, true, false, false),
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
            key_input(Keysym::Tab, false, true, false, false),
            GestureInput::Press {
                key: GestureKey::Tab,
                shift: false
            }
        );
        assert_eq!(
            key_input(Keysym::Escape, false, true, false, false),
            GestureInput::Press {
                key: GestureKey::Escape,
                shift: false
            }
        );
        assert_eq!(
            key_input(Keysym::Return, false, true, false, false),
            GestureInput::Press {
                key: GestureKey::Commit,
                shift: false
            }
        );
    }

    #[test]
    fn grouped_key_input_binds_tab_to_the_scoped_step() {
        // The owner's 2026-11-08 collapse re-decision: Alt+Tab cycles
        // the current level — app entries on level 1, the group's
        // windows on level 2 — never crossing levels by itself. Flat
        // mode keeps Tab stepping rows, byte for byte.
        assert_eq!(
            key_input(Keysym::Tab, false, false, false, true),
            GestureInput::Press {
                key: GestureKey::TabScope,
                shift: false
            }
        );
        assert_eq!(
            key_input(Keysym::Tab, true, false, false, true),
            GestureInput::Press {
                key: GestureKey::TabScope,
                shift: true
            }
        );
        // ISO_Left_Tab is Shift+Tab by another name, grouped or flat.
        assert_eq!(
            key_input(Keysym::ISO_Left_Tab, false, false, false, true),
            GestureInput::Press {
                key: GestureKey::TabScope,
                shift: true
            }
        );
    }

    #[test]
    fn grouped_alt_w_flips_the_level() {
        // Alt+W is the level toggle — down into the children at level
        // 1, back out at level 2 — for hands that never leave the Alt
        // row. → enters only; W works both ways. Shift may ride along
        // (W); without Alt the letter means nothing, and flat mode
        // never re-binds it.
        for keysym in [Keysym::w, Keysym::W] {
            assert_eq!(
                key_input(keysym, false, false, true, true),
                GestureInput::Press {
                    key: GestureKey::ToggleLevel,
                    shift: false
                },
                "Alt+{keysym:?} flips the level"
            );
        }
        assert_eq!(
            key_input(Keysym::w, false, false, false, true),
            GestureInput::Press {
                key: GestureKey::Other,
                shift: false
            },
            "W without Alt is no gesture key"
        );
        assert_eq!(
            key_input(Keysym::w, false, false, true, false),
            GestureInput::Press {
                key: GestureKey::Other,
                shift: false
            },
            "flat mode never re-binds the letter"
        );
    }

    #[test]
    fn grouped_key_input_maps_left_right_to_the_tree_walk() {
        // The W3C vertical tree axes: → enters a multi-window app's
        // children, ← exits to the parent. The flat step aliases are
        // gone in grouped mode — the vertical step axis is ↑/↓ (and
        // Alt+Tab / Alt+Shift+Tab) alone.
        assert_eq!(
            key_input(Keysym::Right, false, false, false, true),
            GestureInput::Press {
                key: GestureKey::EnterChild,
                shift: false
            }
        );
        assert_eq!(
            key_input(Keysym::Left, false, false, false, true),
            GestureInput::Press {
                key: GestureKey::ExitParent,
                shift: false
            }
        );
        // Up/Down keep stepping visible rows whatever the mode.
        assert_eq!(
            key_input(Keysym::Up, false, false, false, true),
            GestureInput::Press {
                key: GestureKey::Arrow(Direction::Previous),
                shift: false
            }
        );
        assert_eq!(
            key_input(Keysym::Down, false, false, false, true),
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
            key_input(Keysym::Left, false, false, false, false),
            GestureInput::Press {
                key: GestureKey::Arrow(Direction::Previous),
                shift: false
            }
        );
        assert_eq!(
            key_input(Keysym::Right, false, false, false, false),
            GestureInput::Press {
                key: GestureKey::Arrow(Direction::Next),
                shift: false
            }
        );
        assert_eq!(
            key_input(Keysym::Tab, false, false, false, false),
            GestureInput::Press {
                key: GestureKey::Tab,
                shift: false
            }
        );
    }

    #[test]
    fn grouped_key_input_rebinds_both_number_chords() {
        // Ctrl+N addresses the Nth level-1 entry; Alt+N the selected
        // group's Nth child (the boundary resolves both against the
        // event-time model). The Enter family still commits and Escape
        // still dismisses, whatever the mode.
        assert_eq!(
            key_input(Keysym::_3, false, true, false, true),
            GestureInput::Press {
                key: GestureKey::SelectApp(2),
                shift: false
            }
        );
        assert_eq!(
            key_input(Keysym::_3, false, false, true, true),
            GestureInput::Press {
                key: GestureKey::SelectAlt(2),
                shift: false
            },
            "Alt+3 addresses the group's third child"
        );
        assert_eq!(
            key_input(Keysym::KP_5, false, false, true, true),
            GestureInput::Press {
                key: GestureKey::SelectAlt(4),
                shift: false
            },
            "the keypad rides the same chord"
        );
        // Without Alt the number row never surprise-activates.
        assert_eq!(
            key_input(Keysym::_3, false, false, false, true),
            GestureInput::Press {
                key: GestureKey::Other,
                shift: false
            }
        );
        assert_eq!(
            key_input(Keysym::Return, false, false, false, true),
            GestureInput::Press {
                key: GestureKey::Commit,
                shift: false
            }
        );
        assert_eq!(
            key_input(Keysym::Escape, false, false, false, true),
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
        let mut gesture = Gesture::default();
        gesture.feed(GestureInput::Invoke(Direction::Next));
        assert_eq!(
            gesture.feed(GestureInput::Press {
                key: GestureKey::TabScope,
                shift: false
            }),
            GestureOutput::StepScope(Direction::Next)
        );
        assert_eq!(
            gesture.feed(GestureInput::Press {
                key: GestureKey::TabScope,
                shift: true
            }),
            GestureOutput::StepScope(Direction::Previous)
        );
        assert_eq!(
            gesture.feed(GestureInput::Press {
                key: GestureKey::ToggleLevel,
                shift: false
            }),
            GestureOutput::ToggleLevel
        );
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
    }

    #[test]
    fn the_grouped_keys_mean_nothing_between_sessions() {
        let mut gesture = Gesture::default();
        for key in [
            GestureKey::TabScope,
            GestureKey::ToggleLevel,
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
                key_input(keysym, false, true, false, false),
                GestureInput::Press {
                    key: GestureKey::Select(row),
                    shift: false
                },
                "the chord hinted on row {row} must wire that row"
            );
            let hint = RowHint::ctrl(row).expect("rows 1..9 are hinted");
            assert_eq!(
                hint.label(),
                format!("Ctrl+{}", row + 1),
                "row {row}'s label must name its own chord"
            );
        }
        // The reserved band fits the longest label exactly — nothing
        // spare to misalign, nothing clipped.
        assert_eq!(RowHint::Ctrl(9).label().len(), HINT_COLS as usize);
        assert_eq!(MAX_HINTS, 9, "nine wired entries, nine labels");
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
}
