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

/// Logical pixels per font pixel. Text is 16 logical px tall — twice the
/// pre-refresh 8 — which is the single biggest glance improvement: the
/// 200-ms glance hypothesis is *icon shape + first words*, and 8-px text
/// cannot carry words. Doubled with the font's own spacing intact, so
/// the bitmap-font identity survives the size change.
pub const FONT_SCALE: u32 = 2;

/// Title text cell width in logical pixels (glyph cell × [`FONT_SCALE`]).
pub const TEXT_W: u32 = GLYPH_W * FONT_SCALE;

/// Title text height in logical pixels.
pub const TEXT_H: u32 = GLYPH_H * FONT_SCALE;

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

/// Maximum title columns shown before elision. 32 columns of 16-px text
/// keeps the card at ≈600 logical px wide — cosmic-launcher's fixed
/// switcher width and the cross-platform consensus cap (research record
/// §2.6, §2.10).
pub const MAX_COLS: u32 = 32;

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

// Geometry pins. The 8-grid holds for every stepped metric; the row is
// exactly the icon cell plus its breathing; rows stay clear of the
// corner arcs (PAD_Y ≥ RADIUS), so a selection band can never clip a
// corner; and the border is a hairline, not a frame.
const _: () = {
    assert!(ROW_H == ICON + 2 * ROW_PAD_Y);
    assert!(TEXT_H == GLYPH_H * FONT_SCALE);
    assert!(ROW_H >= TEXT_H);
    assert!(PAD_Y >= RADIUS);
    assert!(RADIUS.is_multiple_of(4));
    assert!(ICON.is_multiple_of(8));
    assert!(ICON_GAP.is_multiple_of(4));
    assert!(ROW_PAD_Y.is_multiple_of(4));
    assert!(ROW_GAP.is_multiple_of(4));
    assert!(PAD_X.is_multiple_of(4));
    assert!(PAD_Y.is_multiple_of(4));
    assert!(BORDER == 1);
    assert!(SELECT_INSET.is_multiple_of(4));
    assert!(SELECT_INSET > BORDER);
    assert!(SELECT_INSET < PAD_X);
};

/// Placeholder shown when a window has neither title nor app_id.
const UNTITLED: &str = "(untitled)";

/// Scale of the fallback glyph, in font cells: 24 logical px, close
/// enough to the 32 px icon cell that an iconless row holds its weight
/// next to themed icons (16 px read as underweight in the preview review).
pub const GLYPH_SCALE: u32 = 3;

/// Generic window glyph, the icon-cell fallback: an 8×8 frame with a
/// title-bar divider, drawn in [`COLOR_TEXT`] scaled to `GLYPH_SCALE × 8`
/// logical px and centered in the cell. One glyph vocabulary for every
/// fallback row — the icon column belongs to the apps, the glyph to "no
/// icon".
const WINDOW_GLYPH: [u8; 8] = [0xFF, 0x81, 0x81, 0xFF, 0x81, 0x81, 0x81, 0xFF];

/// Hand-drawn ellipsis for the truncation marker — the one intentional
/// non-ASCII glyph (research record §5.2.3 says elide with `…`; font8x8
/// has none, so this is it). Dots on the period's baseline, columns
/// 1/3/5.
const ELLIPSIS_GLYPH: [u8; 8] = [0x00, 0x00, 0x00, 0x00, 0x00, 0x2A, 0x2A, 0x00];

/// One rendered row: a title, an optional app icon, and whether it is
/// the selection. `icon: None` draws the generic window glyph.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RowModel {
    pub title: String,
    pub icon: Option<Rc<IconBitmap>>,
    pub selected: bool,
}

/// What the overlay should draw: rows in MRU order plus the logical
/// pixel size the surface should have (the renderer multiplies by the
/// render scale for device pixels).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OverlayModel {
    pub rows: Vec<RowModel>,
    pub width: u32,
    pub height: u32,
}

/// Builds the overlay contents for `snapshot` with `selected` highlighted,
/// listing windows under `scope`, with app icons from `icons` prepared at
/// `scale`.
///
/// Rows follow [`switcher::eligible`] order (most recently activated
/// first). Returns `None` when there is nothing to show: no active
/// workspace, no eligible windows, or a selection that is not in the
/// eligible set.
pub fn overlay_model(
    snapshot: &Snapshot,
    selected: Handle,
    scope: Scope,
    icons: &crate::icons::Cache,
    scale: u32,
) -> Option<OverlayModel> {
    let active = snapshot.active?;
    let handles = switcher::eligible(&snapshot.toplevels, active, scope);
    if handles.is_empty() || !handles.contains(&selected) {
        return None;
    }
    let mut rows = Vec::with_capacity(handles.len());
    for handle in handles {
        // Unreachable in practice: eligible() draws from this same snapshot.
        let toplevel = snapshot.toplevels.iter().find(|t| t.handle == handle)?;
        let title = truncate_cols(display_title(&toplevel.title, &toplevel.app_id));
        rows.push(RowModel {
            title,
            icon: icons.bitmap(&toplevel.app_id, scale),
            selected: handle == selected,
        });
    }
    let cols = rows
        .iter()
        .map(|row| row.title.chars().count() as u32)
        .max()
        .unwrap_or(0);
    let count = rows.len() as u32;
    Some(OverlayModel {
        rows,
        width: 2 * PAD_X + ICON + ICON_GAP + cols * TEXT_W,
        // The row gap lives between rows only; the bottom edge mirrors
        // the top.
        height: 2 * PAD_Y + count * ROW_H + (count - 1) * ROW_GAP,
    })
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
/// `scale` device pixels per logical pixel.
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
) {
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
                COLOR_BACKGROUND
            } else {
                coverage_argb(COLOR_BACKGROUND, *cov)
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
                        let argb = coverage_argb(COLOR_SELECTED_BG, *cov);
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
                );
            }
        }
        // Title: vertically centered against the row.
        let text_y = top + (ROW_H - TEXT_H) / 2;
        let mut lx = PAD_X + ICON + ICON_GAP;
        for ch in row.title.chars() {
            if lx + TEXT_W > lw.saturating_sub(PAD_X) {
                break;
            }
            draw_glyph(
                canvas,
                stride,
                glyph(ch),
                lx * scale,
                text_y * scale,
                FONT_SCALE * scale,
            );
            lx += TEXT_W;
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
                let argb = coverage_argb(COLOR_BORDER, outer[index]);
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

/// Draws one 8×8 glyph with `block` device pixels per font pixel at
/// device coordinates (`x`, `y`).
fn draw_glyph(canvas: &mut [u8], stride: u32, glyph: [u8; 8], x: u32, y: u32, block: u32) {
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
                            COLOR_TEXT,
                        );
                    }
                }
            }
        }
    }
}

/// Glyph bitmap for one character: ASCII printables come from [`FONT`],
/// the ellipsis is [`ELLIPSIS_GLYPH`], everything else renders as `?`.
fn glyph(ch: char) -> [u8; 8] {
    match ch {
        '…' => ELLIPSIS_GLYPH,
        ch if ch.is_ascii() && !ch.is_ascii_control() => FONT[(ch as usize) - 0x20],
        _ => FONT[('?' as usize) - 0x20],
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

/// Keys the switcher gesture reacts to, normalized from keysyms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GestureKey {
    Tab,
    /// Explicit confirm: Return / KP_Enter / ISO_Enter while showing.
    /// Same outcome as releasing Alt.
    Commit,
    /// A directional step: Up/Left previous, Down/Right next.
    Arrow(Direction),
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

/// What the daemon should do in response to a [`GestureInput`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GestureOutput {
    /// Move the selection one window in this direction and refresh the
    /// overlay.
    Step(Direction),
    /// Activate the selection and unmap the overlay.
    Commit,
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
/// shift modifier on others: both step backwards. `shift_held` is the
/// seat's last reported shift state.
pub fn key_input(keysym: Keysym, shift_held: bool) -> GestureInput {
    let (key, shift) = match keysym {
        Keysym::ISO_Left_Tab => (GestureKey::Tab, true),
        Keysym::Tab => (GestureKey::Tab, shift_held),
        Keysym::Return | Keysym::KP_Enter | Keysym::ISO_Enter => (GestureKey::Commit, shift_held),
        Keysym::Up | Keysym::Left => (GestureKey::Arrow(Direction::Previous), false),
        Keysym::Down | Keysym::Right => (GestureKey::Arrow(Direction::Next), false),
        Keysym::Escape => (GestureKey::Escape, shift_held),
        _ => (GestureKey::Other, shift_held),
    };
    GestureInput::Press { key, shift }
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
        overlay_model(snap, selected, scope, &no_icons(), 2)
    }

    /// Renders `model` at `scale` into a fresh canvas; returns it with
    /// its stride.
    fn rendered(model: &OverlayModel, scale: u32) -> (Vec<u8>, u32) {
        let (w, h) = (model.width * scale, model.height * scale);
        let stride = w * 4;
        let mut canvas = vec![0u8; (stride * h) as usize];
        render(model, &mut canvas, w, h, stride, scale);
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
        assert_eq!(model.width, 2 * PAD_X + ICON + ICON_GAP + TEXT_W);
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
        assert_eq!(model.width, 2 * PAD_X + ICON + ICON_GAP + 7 * TEXT_W);
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
        assert_eq!(model.width, 2 * PAD_X + ICON + ICON_GAP + MAX_COLS * TEXT_W);
        // The research-informed cap: the widest card stays ≤ 600 logical px.
        assert!(model.width <= 600, "card wider than the 600 px cap");
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
    fn selection_and_border_colors_are_the_documented_blends() {
        // GNOME's :selected formula — text at 20% over the card — pinned
        // exactly so the token cannot drift by accident.
        assert_eq!(COLOR_SELECTED_BG, 0xFA_48_4B_50);
        assert_eq!(COLOR_BORDER, 0xFA44464B);
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
            }],
            width: 2 * PAD_X + ICON + ICON_GAP + 6 * TEXT_W,
            height: 2 * PAD_Y + ROW_H,
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
            }],
            width: 2 * PAD_X + ICON + ICON_GAP + TEXT_W,
            height: 2 * PAD_Y + ROW_H,
        };
        let (canvas, stride) = rendered(&model, 2);
        let glyph_x = (PAD_X + (ICON - GLYPH_W * GLYPH_SCALE) / 2) * 2;
        let glyph_y = (PAD_Y + (ROW_H - GLYPH_H * FONT_SCALE) / 2) * 2;
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
        // 4×4 device block at scale 2 × FONT_SCALE 2).
        // Font column c lands FONT_SCALE logical px in; 'A' row 0 is bits
        // 2 and 3.
        assert_eq!(pixel(&canvas, stride, text_x, text_y), COLOR_BACKGROUND);
        assert_eq!(
            pixel(&canvas, stride, text_x + 2 * FONT_SCALE * 2, text_y),
            COLOR_TEXT
        );
        assert_eq!(
            pixel(&canvas, stride, text_x + 3 * FONT_SCALE * 2, text_y),
            COLOR_TEXT
        );
        assert_eq!(
            pixel(&canvas, stride, text_x + 7 * FONT_SCALE * 2, text_y),
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
                    p == COLOR_BACKGROUND || p == COLOR_SELECTED_BG || p == COLOR_TEXT,
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
        render(&model, &mut canvas, w, h, stride, 2);
        // The 'A' pixels land at the text origin — font column 2 — not
        // at the row start.
        let text_x = (PAD_X + ICON + ICON_GAP + 2 * FONT_SCALE) * 2;
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
                    })
                    .collect(),
                width: 2 * PAD_X + ICON + ICON_GAP + 8 * TEXT_W,
                height: 2 * PAD_Y + 5 * ROW_H + 4 * ROW_GAP,
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
                })
                .collect(),
            width: 2 * PAD_X + ICON + ICON_GAP + MAX_COLS * TEXT_W,
            height: 2 * PAD_Y + 10 * ROW_H + 9 * ROW_GAP,
        };
        let (w, h) = (model.width * 2, model.height * 2);
        let stride = w * 4;
        let mut canvas = vec![0; (stride * h) as usize];
        let start = Instant::now();
        // 30 iterations at 4× the pre-refresh pixel count ≈ twice the
        // old guard's total work (200 renders of the 8-px card).
        for _ in 0..30 {
            render(&model, &mut canvas, w, h, stride, 2);
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
    fn key_input_maps_tab_escape_and_backtab() {
        assert_eq!(
            key_input(Keysym::Tab, false),
            GestureInput::Press {
                key: GestureKey::Tab,
                shift: false
            }
        );
        // Shift held at the seat steers a plain Tab backwards.
        assert_eq!(
            key_input(Keysym::Tab, true),
            GestureInput::Press {
                key: GestureKey::Tab,
                shift: true
            }
        );
        // ISO_Left_Tab is Shift+Tab by another name.
        assert_eq!(
            key_input(Keysym::ISO_Left_Tab, false),
            GestureInput::Press {
                key: GestureKey::Tab,
                shift: true
            }
        );
        assert_eq!(
            key_input(Keysym::Escape, false),
            GestureInput::Press {
                key: GestureKey::Escape,
                shift: false
            }
        );
        assert_eq!(
            key_input(Keysym::Return, false),
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
                key_input(keysym, false),
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
                key_input(keysym, true),
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
            key_input(Keysym::Delete, true),
            GestureInput::Press {
                key: GestureKey::Other,
                shift: true
            }
        );
    }
}
