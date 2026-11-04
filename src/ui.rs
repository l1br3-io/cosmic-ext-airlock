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
/// 8 logical px tall, re-pinned 2026-11-02 by the owner after the
/// hardware pass read even the 12-px step as oversized next to the 32-px
/// icons — the icons carry the row's identity, the title only has to
/// confirm it, and an 8-px bitmap stays crisp where a fractional 10-px
/// scale would letter-space every glyph. Whole-pixel exact at every
/// render scale (each font pixel is `scale` device px); the studied
/// switchers' 13–16 px consensus (research record §2.10) spends its
/// height on richer fonts, not on proportion.
pub const FONT_SCALE_NUM: u32 = 1;
pub const FONT_SCALE_DEN: u32 = 1;

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

/// Horizontal padding inside the card.
pub const PAD_X: u32 = 20;

/// Vertical padding inside the card; rows start and end inside it.
pub const PAD_Y: u32 = 12;

/// Card corner radius in logical pixels. COSMIC's house radius for cards
/// is `radius_s + 4` ≈ 12 (research record §2.6). Corners are coverage
/// anti-aliased by area sampling — see [`shape_cov`] — not aliased steps;
/// nothing is blurred.
pub const RADIUS: u32 = 12;

/// Hairline card border thickness in logical pixels.
pub const BORDER: u32 = 1;

/// Maximum title columns shown before elision. 64 columns of 8-px text
/// keeps the title territory at 596 logical px — cosmic-launcher's fixed
/// switcher width and the cross-platform consensus cap of ~600 (research
/// record §2.6, §2.10). The 8-px re-pin spends its savings on more first
/// words per title, not on a narrower card; the hint band rides on top
/// of the cap rather than under it (rev 2's recorded decision: the card
/// widens by the band, the title budget does not fund it).
pub const MAX_COLS: u32 = 64;

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

/// Rows the card may show at once, content and affordances together. 8
/// keeps the card at capacity at `2·12 + 8·48 + 7·8 = 464` logical px —
/// inside a 540-logical (1080p@2x) screen with headroom, and past ~8
/// rows a glance stops finding the target anyway: this is a switcher,
/// not a launcher. The ring still walks the full eligible list; only
/// presentation windows.
pub const VIEW_ROWS: usize = 8;

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
// corner; and the border is a hairline, not a frame.
const _: () = {
    assert!(ROW_H == ICON + 2 * ROW_PAD_Y);
    assert!(TEXT_H == GLYPH_H * FONT_SCALE_NUM / FONT_SCALE_DEN);
    assert!(TEXT_W == TEXT_H);
    // The glyph block is whole device pixels at the render-scale floor.
    assert!((TEXT_W * 2).is_multiple_of(GLYPH_W));
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
    // The card at capacity fits the smallest common logical screen
    // (1080p at 2× = 540 logical px).
    assert!(2 * PAD_Y + VIEW_ROWS as u32 * ROW_H + (VIEW_ROWS as u32 - 1) * ROW_GAP <= 540);
    // With rows hidden at both edges the two outermost slots become
    // affordances; a window smaller than three could show no content at
    // all.
    assert!(VIEW_ROWS >= 3);
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
    /// continuation row.
    pub handle: Option<Handle>,
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

/// The window over a `len`-long eligible list with `sel` selected,
/// continuing from `prev` — `None` is a fresh session, top-aligned.
///
/// The window moves the minimum needed to keep the selection on a
/// *content* row, never a continuation slot: nothing while the selection
/// stays interior (the sticky rule — the walking fill keeps its motion
/// vocabulary), exactly one row when stepping past an edge. The starts
/// keeping `sel` content-visible form one interval, so the rule is a
/// clamp of the carried start into it; the interval's upper end widens
/// at the last window, whose bottom edge hides nothing. A start from a
/// longer list re-clamps after a mid-gesture shrink.
pub fn visible_window(len: usize, sel: usize, prev: Option<Viewport>) -> Viewport {
    debug_assert!(sel < len, "the caller's selection indexes the list");
    let visible = len.min(VIEW_ROWS);
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

/// Builds the overlay contents for `snapshot` with `selected` highlighted,
/// listing windows under `scope`, with app icons from `icons` prepared at
/// `scale` for the card `palette` paints, continuing the sticky `viewport`
/// a session carries (`None` at open).
///
/// Rows follow [`switcher::eligible`] order (most recently activated
/// first), then window to [`VIEW_ROWS`]: a continuation row replaces the
/// outermost slot at each truncated edge, and the model reports the
/// window it computed for the session to carry next. Width still derives
/// from the *full* eligible set's widest title, so the card never
/// breathes while the window moves under stepping. Returns `None` when
/// there is nothing to show: no active workspace, no eligible windows,
/// or a selection that is not in the eligible set.
pub fn overlay_model(
    snapshot: &Snapshot,
    selected: Handle,
    scope: Scope,
    icons: &crate::icons::Cache,
    scale: u32,
    palette: &Palette,
    viewport: Option<Viewport>,
) -> Option<OverlayModel> {
    let active = snapshot.active?;
    let handles = switcher::eligible(&snapshot.toplevels, active, scope);
    if handles.is_empty() {
        return None;
    }
    let sel = handles.iter().position(|&handle| handle == selected)?;
    let mut rows = Vec::with_capacity(handles.len());
    for handle in handles {
        // Unreachable in practice: eligible() draws from this same snapshot.
        let toplevel = snapshot.toplevels.iter().find(|t| t.handle == handle)?;
        let title = truncate_cols(display_title(&toplevel.title, &toplevel.app_id));
        rows.push(RowModel {
            title,
            icon: icons.bitmap(&toplevel.app_id, scale, palette),
            selected: handle == selected,
            handle: Some(handle),
        });
    }
    let cols = rows
        .iter()
        .map(|row| row.title.chars().count() as u32)
        .max()
        .unwrap_or(0);
    let viewport = visible_window(rows.len(), sel, viewport);
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
        height: 2 * PAD_Y + count * ROW_H + (count - 1) * ROW_GAP,
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
        // earns nothing but its dots. Card-centered is the candidate
        // placement; the preview sheet freezes it (LV3).
        if row.handle.is_none() {
            let dots_x = lw.saturating_sub(GLYPH_W) / 2;
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
            let band_width = lw.saturating_sub(2 * SELECT_INSET) as usize;
            for ly in top..top + ROW_H {
                let cov_row = &outer[(ly * lw) as usize..(ly * lw + lw) as usize];
                for dy in 0..scale {
                    let y = ly * scale + dy;
                    for (lx, cov) in cov_row
                        .iter()
                        .enumerate()
                        .skip(SELECT_INSET as usize)
                        .take(band_width)
                    {
                        let argb = coverage_argb(palette.selected_bg, *cov);
                        let x0 = lx as u32 * scale;
                        for x in x0..x0 + scale {
                            put_pixel(canvas, stride, x, y, argb);
                        }
                    }
                }
            }
        }
        // Icon cell: a bitmap prepared at ICON × scale blits 1:1; a
        // mismatched size or a missing icon draws the generic glyph.
        let cell_y = top + (ROW_H - ICON) / 2;
        match row.icon.as_deref() {
            Some(bitmap) if bitmap.side == ICON * scale => {
                blit_bitmap(canvas, stride, bitmap, PAD_X * scale, cell_y * scale);
            }
            _ => {
                let side = GLYPH_W * GLYPH_SCALE;
                draw_glyph(
                    canvas,
                    stride,
                    WINDOW_GLYPH,
                    (PAD_X + (ICON - side) / 2) * scale,
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
        let mut lx = PAD_X + ICON + ICON_GAP;
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
        // Hint: the row's Ctrl+N chord, right-aligned in the band the
        // card always reserves and vertically centered on the titles'
        // own line. Rows past nine paint nothing — every displayed hint
        // is a wired key ([`hint_label`] is that truth's single source).
        if let Some(label) = hint_label(index) {
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

/// Composites a premultiplied-ARGB bitmap 1:1 over the canvas at device
/// coordinates (`src` over `dst`, both premultiplied at the boundary).
fn blit_bitmap(canvas: &mut [u8], stride: u32, bitmap: &IconBitmap, x: u32, y: u32) {
    for py in 0..bitmap.side {
        for px in 0..bitmap.side {
            let src = bitmap.argb[(py * bitmap.side + px) as usize];
            let sa = src >> 24;
            let index = ((y + py) as usize) * stride as usize + ((x + px) as usize) * 4;
            let Some(slot) = canvas.get_mut(index..index + 4) else {
                continue;
            };
            if sa == 255 {
                slot.copy_from_slice(&src.to_le_bytes());
                continue;
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
    }
}

/// Writes one ARGB8888 pixel; out-of-canvas coordinates are skipped.
fn put_pixel(canvas: &mut [u8], stride: u32, x: u32, y: u32, argb: u32) {
    let index = y as usize * stride as usize + x as usize * 4;
    if let Some(slot) = canvas.get_mut(index..index + 4) {
        slot.copy_from_slice(&argb.to_le_bytes());
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

/// Glyph bitmap for one character: ASCII printables come from [`FONT`],
/// the ellipsis is [`ELLIPSIS_GLYPH`], typographic punctuation folds to
/// its ASCII lookalike ([`fold_punct`]), everything else renders as `?`.
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
/// to one char, so column math is unchanged. Letters and other scripts
/// still fall through to `?`: the honest floor of an ASCII font, stated
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

/// Caps a title at [`MAX_COLS`] columns, marking the cut with `…`.
fn truncate_cols(title: String) -> String {
    if title.chars().count() > MAX_COLS as usize {
        let keep: String = title.chars().take(MAX_COLS as usize - 1).collect();
        format!("{keep}…")
    } else {
        title
    }
}

/// The chords addressing the first nine rows, in row order — the render
/// twin of `key_input`'s Ctrl+digit mapping. The pairing is unit-pinned:
/// a label that outlives its wiring would be a dead key advertised on
/// the card.
const HINTS: [&str; 9] = [
    "Ctrl+1", "Ctrl+2", "Ctrl+3", "Ctrl+4", "Ctrl+5", "Ctrl+6", "Ctrl+7", "Ctrl+8", "Ctrl+9",
];

/// The chord addressing `row` (0-based). Rows 1..9 are addressable and
/// hinted; deeper rows return `None` and paint nothing. Every displayed
/// hint is a wired key — this function is the single source of that
/// truth, and `key_input`'s `Select` mapping is its input twin.
fn hint_label(row: usize) -> Option<&'static str> {
    HINTS.get(row).copied()
}

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
    /// top row = 0) in one motion — the keyboard twin of a pointer click.
    Select(usize),
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
/// wl_pointer events on the overlay surface. `row` indexes the live
/// overlay model's rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerInput {
    /// Cursor is over `Some(row)` of the overlay (enter or motion);
    /// `None` is a gap or off-card movement and never selects.
    Hover(Option<usize>),
    /// Left-button press on a row: select and commit in one motion —
    /// the same outcome as pressing Enter or releasing Alt.
    Click { row: usize },
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
    /// Activate the selection and unmap the overlay.
    Commit,
    /// Select and activate overlay `row` in one motion. The gesture
    /// session survives the output: the daemon closes it only when the
    /// activation actually happens, so an out-of-range row leaves the
    /// gesture fully intact.
    Activate {
        row: usize,
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
                // Select-and-activate in one motion; the session stays
                // open — the daemon closes it via `close()` only when
                // the activation actually happens, so an out-of-range
                // row leaves the gesture fully intact.
                GestureKey::Select(row) => GestureOutput::Activate { row },
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

/// Normalizes a grabbed key press into gesture input.
///
/// Shift+Tab arrives as ISO_Left_Tab on most layouts and as Tab plus the
/// shift modifier on others: both step backwards. `shift_held` and
/// `ctrl_held` are the seat's last reported modifier states — Ctrl
/// classifies digit presses, exactly as shift classifies Tab.
pub fn key_input(keysym: Keysym, shift_held: bool, ctrl_held: bool) -> GestureInput {
    let (key, shift) = match keysym {
        Keysym::ISO_Left_Tab => (GestureKey::Tab, true),
        Keysym::Tab => (GestureKey::Tab, shift_held),
        Keysym::Return | Keysym::KP_Enter | Keysym::ISO_Enter => (GestureKey::Commit, shift_held),
        Keysym::Up | Keysym::Left => (GestureKey::Arrow(Direction::Previous), false),
        Keysym::Down | Keysym::Right => (GestureKey::Arrow(Direction::Next), false),
        Keysym::Escape => (GestureKey::Escape, shift_held),
        // Ctrl+digits map with or without shift held; without Ctrl the
        // number row stays Other — it must never surprise-activate.
        _ if ctrl_held && let Some(row) = digit_row(keysym) => {
            (GestureKey::Select(row), shift_held)
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
pub fn wheel_notches(value120: i32, discrete: i32, residual: i32) -> (i32, i32) {
    if value120 != 0 {
        let total = residual + value120;
        let notches = total / 120;
        (notches, total - notches * 120)
    } else {
        (discrete, residual)
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
            None,
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
        // The research-informed cap governs the title territory; rev 2's
        // recorded decision widens the card by the structural hint band
        // on top of it rather than taxing the 64-column title budget.
        assert!(
            model.width <= 600 + HINT_GAP + HINT_COLS * TEXT_W,
            "card wider than the 600 px title cap plus the hint band"
        );
    }

    #[test]
    fn hint_label_addresses_exactly_rows_one_through_nine() {
        // Rows 1..9 are addressable and hinted; row 10 onward paints
        // nothing — no dead keys on the card (cosmic's own row-10 hint
        // is an unwired upstream gap Airlock does not copy).
        let labels = [
            "Ctrl+1", "Ctrl+2", "Ctrl+3", "Ctrl+4", "Ctrl+5", "Ctrl+6", "Ctrl+7", "Ctrl+8",
            "Ctrl+9",
        ];
        for (row, label) in labels.iter().enumerate() {
            assert_eq!(hint_label(row), Some(*label), "row {row}");
        }
        assert_eq!(hint_label(9), None, "row 10 is not addressable");
        assert_eq!(hint_label(usize::MAX), None);
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

    /// Every window `visible_window` can produce for a `len`-long list,
    /// deduplicated: one per selection position.
    fn reachable_windows(len: usize) -> Vec<Viewport> {
        let mut out = Vec::new();
        for sel in 0..len {
            let vp = visible_window(len, sel, None);
            if !out.contains(&vp) {
                out.push(vp);
            }
        }
        out
    }

    /// Prior windows a session could carry into a `len`-long rebuild:
    /// every window reachable at `len` plus every window reachable at a
    /// longer length — the mid-gesture shrink case.
    fn prior_windows(len: usize) -> Vec<Viewport> {
        let mut priors = reachable_windows(len);
        for longer in len + 1..=3 * VIEW_ROWS + 2 {
            priors.extend(reachable_windows(longer));
        }
        priors
    }

    #[test]
    fn visible_window_keeps_every_selection_on_a_content_row() {
        // The fixed point, exhaustive over every (len, sel, prev) the
        // gesture can produce: the window covers exactly the visible
        // slots, the flags honestly report what is hidden, and the
        // selection always lands in the inner content range — never an
        // affordance slot, never off-window. Any implementation
        // satisfying this is the contract; the clamping inside is not.
        for len in 0..=3 * VIEW_ROWS + 2 {
            for sel in 0..len {
                for prev in std::iter::once(None).chain(prior_windows(len).into_iter().map(Some)) {
                    let vp = visible_window(len, sel, prev);
                    let visible = len.min(VIEW_ROWS);
                    assert_eq!(
                        vp.top as usize + vp.content + vp.bottom as usize,
                        visible,
                        "len {len} sel {sel} prev {prev:?}: slots must equal the visible cap"
                    );
                    assert_eq!(vp.top, vp.start > 0, "len {len} sel {sel}: top flag");
                    assert_eq!(
                        vp.bottom,
                        vp.start + visible < len,
                        "len {len} sel {sel}: bottom flag"
                    );
                    assert!(
                        vp.start + visible <= len,
                        "len {len} sel {sel}: window runs past the list"
                    );
                    let first = vp.start + vp.top as usize;
                    assert!(
                        first <= sel && sel < first + vp.content,
                        "len {len} sel {sel} prev {prev:?}: selection {sel} not on a content \
                         row of {vp:?}"
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
        for len in VIEW_ROWS..=3 * VIEW_ROWS + 2 {
            for prev in reachable_windows(len) {
                let first = prev.start + prev.top as usize;
                for sel in first..first + prev.content {
                    assert_eq!(
                        visible_window(len, sel, Some(prev)),
                        prev,
                        "len {len} sel {sel}: the window moved under a visible selection"
                    );
                }
            }
        }
    }

    #[test]
    fn visible_window_moves_one_row_at_most_and_only_when_it_must() {
        // One step of the selection moves the window by at most one row,
        // by zero while the new selection stays content-visible, and by
        // exactly one when it stepped past an edge.
        for len in VIEW_ROWS + 1..=3 * VIEW_ROWS + 2 {
            for prev_sel in 0..len {
                let prev = visible_window(len, prev_sel, None);
                for next_sel in [prev_sel + 1, prev_sel.saturating_sub(1)] {
                    if next_sel >= len {
                        continue;
                    }
                    let next = visible_window(len, next_sel, Some(prev));
                    let delta = next.start.abs_diff(prev.start);
                    let still_visible = (prev.start + prev.top as usize
                        ..prev.start + prev.top as usize + prev.content)
                        .contains(&next_sel);
                    let expected = if still_visible { 0 } else { 1 };
                    assert_eq!(
                        delta, expected,
                        "len {len} {prev_sel}→{next_sel}: window moved {delta} rows, expected \
                         {expected}"
                    );
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
        assert_eq!(visible_window(VIEW_ROWS + 5, 0, None).start, 0);
        let head = visible_window(VIEW_ROWS + 5, VIEW_ROWS - 2, None);
        assert_eq!(head.start, 0);
        assert!(head.bottom, "a longer list truncates at the bottom only");
    }

    #[test]
    fn a_list_at_the_cap_shows_whole() {
        // The 8-row-exact boundary: at the cap the window is the list —
        // no affordances, no truncation, today's card exactly.
        let at_cap = visible_window(VIEW_ROWS, VIEW_ROWS - 1, None);
        assert_eq!(
            at_cap,
            Viewport {
                start: 0,
                content: VIEW_ROWS,
                top: false,
                bottom: false
            }
        );
        // Below the cap the same holds for any length.
        let short = visible_window(3, 2, None);
        assert_eq!(
            short,
            Viewport {
                start: 0,
                content: 3,
                top: false,
                bottom: false
            }
        );
        // One past the cap, both truncations are reachable — the head
        // window hides a row below, the pulled-down window hides a row
        // above — but never both at once: only two windows exist.
        let head = visible_window(VIEW_ROWS + 1, 0, None);
        assert!(!head.top && head.bottom);
        let pulled = visible_window(VIEW_ROWS + 1, VIEW_ROWS, None);
        assert_eq!(pulled.start, 1);
        assert!(pulled.top && !pulled.bottom);
        assert_eq!(pulled.content, VIEW_ROWS - 1);
        // Two past the cap, the interior window carries both dots.
        let interior = visible_window(VIEW_ROWS + 2, VIEW_ROWS - 1, None);
        assert!(interior.top && interior.bottom);
        assert_eq!(interior.content, VIEW_ROWS - 2);
    }

    #[test]
    fn a_full_sweep_shifts_the_window_in_single_row_steps() {
        // Carrying the window through a whole Tab sweep (and back) moves
        // it from the top window to the bottom window one row at a time,
        // never jumping — the shift reads as one clean frame per step.
        for len in [VIEW_ROWS + 1, 2 * VIEW_ROWS, 3 * VIEW_ROWS + 2] {
            let visible = len.min(VIEW_ROWS);
            let mut carried = visible_window(len, 0, None);
            assert_eq!(carried.start, 0, "len {len}: the sweep opens top-aligned");
            let mut starts = vec![carried.start];
            for sel in 1..len {
                carried = visible_window(len, sel, Some(carried));
                starts.push(carried.start);
            }
            assert_eq!(
                carried.start,
                len - visible,
                "len {len}: the sweep must reach the bottom window"
            );
            for pair in starts.windows(2) {
                assert!(
                    pair[1] - pair[0] <= 1,
                    "len {len}: the window jumped: {starts:?}"
                );
            }
            // Shift+Tab back up: the same vocabulary in reverse.
            for sel in (0..len - 1).rev() {
                let now = visible_window(len, sel, Some(carried));
                assert!(
                    carried.start - now.start <= 1,
                    "len {len} sel {sel}: the window jumped back up"
                );
                carried = now;
            }
            assert_eq!(carried.start, 0, "len {len}: the return sweep re-tops");
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
        prev: Option<Viewport>,
    ) -> Option<OverlayModel> {
        overlay_model(
            snap,
            selected,
            Scope::CurrentWorkspace,
            &no_icons(),
            2,
            &Palette::dark(),
            prev,
        )
    }

    #[test]
    fn a_list_at_the_cap_renders_whole_and_unchanged() {
        // The no-regression boundary, exactly at the cap: no affordance
        // rows, every handle present, and today's geometry formulas
        // verbatim — a ≤ 8-window gesture is byte-identical to the
        // pre-module card.
        let snap = long_snapshot(VIEW_ROWS as u64);
        let model = build_windowed(&snap, Handle(1), None).expect("eligible windows exist");
        assert_eq!(model.rows.len(), VIEW_ROWS);
        assert!(
            model.rows.iter().all(|row| row.handle.is_some()),
            "no affordance rows at the cap"
        );
        assert_eq!(
            model.viewport,
            Viewport {
                start: 0,
                content: VIEW_ROWS,
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
            2 * PAD_Y + VIEW_ROWS as u32 * ROW_H + (VIEW_ROWS as u32 - 1) * ROW_GAP
        );
    }

    #[test]
    fn a_long_lists_head_window_truncates_only_below() {
        let snap = long_snapshot(15);
        let model = build_windowed(&snap, Handle(1), None).expect("eligible windows exist");
        assert_eq!(model.rows.len(), VIEW_ROWS);
        assert_eq!(
            model.viewport,
            Viewport {
                start: 0,
                content: VIEW_ROWS - 1,
                top: false,
                bottom: true
            }
        );
        let handles: Vec<Option<Handle>> = model.rows.iter().map(|row| row.handle).collect();
        let expected: Vec<Option<Handle>> = (1..=7)
            .map(|handle| Some(Handle(handle)))
            .chain([None])
            .collect();
        assert_eq!(handles, expected, "seven content rows, dots at the bottom");
    }

    #[test]
    fn an_interior_selection_windows_both_edges() {
        let snap = long_snapshot(15);
        // Carrying the head window while selecting list index 7 (handle
        // 8) shifts it by one row: dots above and below, the selection on
        // the last content row.
        let model = build_windowed(
            &snap,
            Handle(8),
            Some(Viewport {
                start: 0,
                content: VIEW_ROWS - 1,
                top: false,
                bottom: true,
            }),
        )
        .expect("eligible windows exist");
        assert_eq!(
            model.viewport,
            Viewport {
                start: 1,
                content: VIEW_ROWS - 2,
                top: true,
                bottom: true
            }
        );
        let handles: Vec<Option<Handle>> = model.rows.iter().map(|row| row.handle).collect();
        let expected: Vec<Option<Handle>> = [None]
            .into_iter()
            .chain((3..=8).map(|handle| Some(Handle(handle))))
            .chain([None])
            .collect();
        assert_eq!(handles, expected);
        assert_eq!(
            model.height,
            2 * PAD_Y + VIEW_ROWS as u32 * ROW_H + (VIEW_ROWS as u32 - 1) * ROW_GAP,
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
        for handle in 1..=15 {
            let model =
                build_windowed(&snap, Handle(handle), prev).expect("eligible windows exist");
            widths.push(model.width);
            prev = Some(model.viewport);
        }
        widths.dedup();
        assert_eq!(widths.len(), 1, "the card breathed across the sweep");
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
            let model =
                build_windowed(&snap, Handle(handle), prev).expect("eligible windows exist");
            assert!(
                model.rows.len() <= VIEW_ROWS,
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
            Handle(8),
            Some(Viewport {
                start: 0,
                content: VIEW_ROWS - 1,
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
            for slot in [0, 7] {
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
            Handle(8),
            Some(Viewport {
                start: 0,
                content: VIEW_ROWS - 1,
                top: false,
                bottom: true,
            }),
        )
        .expect("eligible windows exist");
        let (canvas, stride) = rendered(&model, 2);
        let palette = Palette::dark();
        for slot in [0, 7] {
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
                })
                .collect(),
            width: 2 * PAD_X + ICON + ICON_GAP + TEXT_W + HINT_GAP + HINT_COLS * TEXT_W,
            height: 2 * PAD_Y + rows as u32 * ROW_H + (rows as u32 - 1) * ROW_GAP,
            viewport: visible_window(rows, 0, None),
        }
    }

    #[test]
    fn row_at_maps_each_row_band_to_its_index() {
        let model = hit_model(3);
        let x = model.width as f64 / 2.0;
        // Mid-band of every row: PAD_Y top inset, ROW_H bands, ROW_GAP
        // seams — 36/92/148 at the documented rhythm.
        for (row, mid) in [(0usize, 36.0f64), (1, 92.0), (2, 148.0)] {
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
        // The seam between rows 0 and 1 is [60, 68); between 1 and 2
        // [116, 124). Selection bands never merge across a seam; neither
        // does a click land.
        for y in [60.0f64, 64.0, 67.0, 116.0] {
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
        assert_eq!(model.row_at(-0.5, 36.0, 1), None);
        assert_eq!(model.row_at(model.width as f64, 36.0, 1), None);
        // The last pixel columns are still the card.
        assert_eq!(
            model.row_at(model.width as f64 - 0.5, 36.0, 1),
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
            }],
            width: 2 * PAD_X + ICON + ICON_GAP + 6 * TEXT_W + HINT_GAP + HINT_COLS * TEXT_W,
            height: 2 * PAD_Y + ROW_H,
            viewport: visible_window(1, 0, None),
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
            }],
            width: 2 * PAD_X + ICON + ICON_GAP + TEXT_W + HINT_GAP + HINT_COLS * TEXT_W,
            height: 2 * PAD_Y + ROW_H,
            viewport: visible_window(1, 0, None),
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
                    })
                    .collect(),
                width: 2 * PAD_X + ICON + ICON_GAP + 8 * TEXT_W + HINT_GAP + HINT_COLS * TEXT_W,
                height: 2 * PAD_Y + 5 * ROW_H + 4 * ROW_GAP,
                viewport: visible_window(5, 0, None),
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
                })
                .collect(),
            width: 2 * PAD_X + ICON + ICON_GAP + 10 * TEXT_W + HINT_GAP + HINT_COLS * TEXT_W,
            height: 2 * PAD_Y + 12 * ROW_H + 11 * ROW_GAP,
            // Hand-built past the cap on purpose (hint-silence rows past
            // nine); render is row-count agnostic and never reads the viewport.
            viewport: visible_window(12, 0, None),
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
            }],
            width: 2 * PAD_X + ICON + ICON_GAP + 4 * TEXT_W + HINT_GAP + HINT_COLS * TEXT_W,
            height: 2 * PAD_Y + ROW_H,
            viewport: visible_window(1, 0, None),
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
                },
                RowModel {
                    title: "Bottom".into(),
                    icon: None,
                    selected: false,
                    handle: Some(Handle(2)),
                },
            ],
            width: 2 * PAD_X + ICON + ICON_GAP + 6 * TEXT_W + HINT_GAP + HINT_COLS * TEXT_W,
            height: 2 * PAD_Y + 2 * ROW_H + ROW_GAP,
            viewport: visible_window(2, 0, None),
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
            }],
            width: 2 * PAD_X + ICON + ICON_GAP + MAX_COLS * TEXT_W + HINT_GAP + HINT_COLS * TEXT_W,
            height: 2 * PAD_Y + ROW_H,
            viewport: visible_window(1, 0, None),
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
                })
                .collect(),
            width: 2 * PAD_X + ICON + ICON_GAP + MAX_COLS * TEXT_W + HINT_GAP + HINT_COLS * TEXT_W,
            height: 2 * PAD_Y + 10 * ROW_H + 9 * ROW_GAP,
            // Hand-built past the cap on purpose (render work, not a real
            // card shape); render never reads the viewport.
            viewport: visible_window(10, 0, None),
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
        // ellipsis keeps its own glyph path, letters stay themselves.
        assert_eq!(fold_punct('…'), '…');
        assert_eq!(fold_punct('é'), 'é');
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
    fn a_number_selection_activates_row_n_and_keeps_the_session_open() {
        // Ctrl+N is select-and-activate in one motion, but the session
        // survives the output: the daemon closes it via `close()` only
        // when the activation actually happens, so an out-of-range
        // Ctrl+N leaves the session fully intact.
        let mut gesture = Gesture::default();
        gesture.feed(GestureInput::Invoke(Direction::Next));
        assert_eq!(
            gesture.feed(GestureInput::Press {
                key: GestureKey::Select(2),
                shift: false
            }),
            GestureOutput::Activate { row: 2 }
        );
        assert!(
            gesture.showing(),
            "the session stays open for the daemon to close"
        );
    }

    #[test]
    fn a_number_selection_between_sessions_is_nothing() {
        // The dead-arm contract every press shares: no overlay, no output.
        let mut gesture = Gesture::default();
        assert_eq!(
            gesture.feed(GestureInput::Press {
                key: GestureKey::Select(0),
                shift: false
            }),
            GestureOutput::Nothing
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
                key: GestureKey::Select(1),
                shift: false
            }),
            GestureOutput::Activate { row: 1 }
        );
        gesture.close();
        assert_eq!(
            gesture.feed(GestureInput::Press {
                key: GestureKey::Select(1),
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
            key_input(Keysym::Tab, false, false),
            GestureInput::Press {
                key: GestureKey::Tab,
                shift: false
            }
        );
        // Shift held at the seat steers a plain Tab backwards.
        assert_eq!(
            key_input(Keysym::Tab, true, false),
            GestureInput::Press {
                key: GestureKey::Tab,
                shift: true
            }
        );
        // ISO_Left_Tab is Shift+Tab by another name.
        assert_eq!(
            key_input(Keysym::ISO_Left_Tab, false, false),
            GestureInput::Press {
                key: GestureKey::Tab,
                shift: true
            }
        );
        assert_eq!(
            key_input(Keysym::Escape, false, false),
            GestureInput::Press {
                key: GestureKey::Escape,
                shift: false
            }
        );
        assert_eq!(
            key_input(Keysym::Return, false, false),
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
                key_input(keysym, false, false),
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
                key_input(keysym, true, false),
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
            key_input(Keysym::Delete, true, false),
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
                key_input(keysym, false, true),
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
                key_input(keysym, false, true),
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
            key_input(Keysym::_3, false, false),
            GestureInput::Press {
                key: GestureKey::Other,
                shift: false
            }
        );
        assert_eq!(
            key_input(Keysym::KP_5, false, false),
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
            key_input(Keysym::_4, true, true),
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
            key_input(Keysym::_0, false, true),
            GestureInput::Press {
                key: GestureKey::Other,
                shift: false
            }
        );
        assert_eq!(
            key_input(Keysym::KP_0, false, true),
            GestureInput::Press {
                key: GestureKey::Other,
                shift: false
            }
        );
        assert_eq!(
            key_input(Keysym::Delete, false, true),
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
            key_input(Keysym::Tab, false, true),
            GestureInput::Press {
                key: GestureKey::Tab,
                shift: false
            }
        );
        assert_eq!(
            key_input(Keysym::Escape, false, true),
            GestureInput::Press {
                key: GestureKey::Escape,
                shift: false
            }
        );
        assert_eq!(
            key_input(Keysym::Return, false, true),
            GestureInput::Press {
                key: GestureKey::Commit,
                shift: false
            }
        );
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
                key_input(keysym, false, true),
                GestureInput::Press {
                    key: GestureKey::Select(row),
                    shift: false
                },
                "the chord hinted on row {row} must wire that row"
            );
            let chord = format!("Ctrl+{}", row + 1);
            assert_eq!(
                hint_label(row),
                Some(chord.as_str()),
                "row {row}'s label must name its own chord"
            );
        }
        // The reserved band fits the longest label exactly — nothing
        // spare to misalign, nothing clipped.
        let longest = HINTS.iter().map(|label| label.len()).max().unwrap();
        assert_eq!(HINT_COLS as usize, longest);
        assert_eq!(HINTS.len(), 9, "nine wired rows, nine labels");
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
