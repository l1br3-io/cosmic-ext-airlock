//! Switcher overlay: a pure render model, a software renderer, and the
//! layer-shell surface that presents it.
//!
//! The model and renderer are plain data in, bytes out — unit-tested without
//! a compositor. The [`Overlay`] driver owns the protocol objects and is only
//! provable on real COSMIC hardware.

use std::fmt;
use std::time::{Duration, Instant};

use cctk::sctk::compositor::CompositorState;
use cctk::sctk::shell::WaylandSurface;
use cctk::sctk::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerShell, LayerSurface,
};
use cctk::sctk::shm::Shm;
use cctk::sctk::shm::slot::SlotPool;
use cctk::wayland_client::QueueHandle;
use cctk::wayland_client::protocol::{wl_output::WlOutput, wl_shm, wl_surface::WlSurface};
use cosmic_client_toolkit as cctk;

use crate::switcher::{self, Handle, Snapshot};
use crate::wayland::App;

/// Glyph cell width in logical pixels.
pub const GLYPH_W: u32 = 8;

/// Glyph cell height in logical pixels.
pub const GLYPH_H: u32 = 8;

/// Horizontal padding inside the overlay.
pub const PAD_X: u32 = 16;

/// Vertical padding inside the overlay.
pub const PAD_Y: u32 = 12;

/// Vertical gap between rows.
pub const ROW_GAP: u32 = 10;

/// Maximum title columns shown before truncation.
pub const MAX_COLS: u32 = 48;

/// How long the overlay stays up before closing itself when idle.
pub const CLOSE_AFTER: Duration = Duration::from_secs(2);

/// Overlay background (ARGB, little-endian byte order when written).
pub const COLOR_BACKGROUND: u32 = 0xF0_1E_21_26;

/// Row background for the selected row.
pub const COLOR_SELECTED_BG: u32 = 0xF0_1F_3A_5F;

/// Text color.
pub const COLOR_TEXT: u32 = 0xFF_F2_F4_F8;

/// Placeholder shown when a window has neither title nor app_id.
const UNTITLED: &str = "(untitled)";

/// One rendered row: a title and whether it is the selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowModel {
    pub title: String,
    pub selected: bool,
}

/// What the overlay should draw: rows in MRU order plus the pixel size the
/// surface should have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayModel {
    pub rows: Vec<RowModel>,
    pub width: u32,
    pub height: u32,
}

/// Builds the overlay contents for `snapshot` with `selected` highlighted.
///
/// Rows follow [`switcher::eligible`] order (most recently activated first).
/// Returns `None` when there is nothing to show: no active workspace, no
/// eligible windows, or a selection that is not in the eligible set.
pub fn overlay_model(snapshot: &Snapshot, selected: Handle) -> Option<OverlayModel> {
    let active = snapshot.active?;
    let handles = switcher::eligible(&snapshot.toplevels, active);
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
            selected: handle == selected,
        });
    }
    let cols = rows
        .iter()
        .map(|row| row.title.chars().count() as u32)
        .max()
        .unwrap_or(0);
    let height = rows.len() as u32 * (GLYPH_H + ROW_GAP) + 2 * PAD_Y;
    Some(OverlayModel {
        rows,
        width: cols * GLYPH_W + 2 * PAD_X,
        height,
    })
}

/// Renders `model` into `canvas` as ARGB8888 (`stride` bytes per row).
///
/// Every pixel is written: rows of background (selection highlighted) with
/// title glyphs on top. Coordinates outside `width`/`height` are skipped, so
/// an oversized canvas is harmless.
pub fn render(model: &OverlayModel, canvas: &mut [u8], width: u32, height: u32, stride: u32) {
    for y in 0..height {
        for x in 0..width {
            put_pixel(canvas, stride, x, y, COLOR_BACKGROUND);
        }
    }
    for (index, row) in model.rows.iter().enumerate() {
        let top = PAD_Y + index as u32 * (GLYPH_H + ROW_GAP);
        if row.selected {
            for y in top..top + GLYPH_H {
                for x in 0..width {
                    put_pixel(canvas, stride, x, y, COLOR_SELECTED_BG);
                }
            }
        }
        let mut x = PAD_X;
        for ch in row.title.chars() {
            if x + GLYPH_W > width.saturating_sub(PAD_X) {
                break;
            }
            draw_glyph(canvas, stride, glyph(ch), x, top);
            x += GLYPH_W;
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

fn draw_glyph(canvas: &mut [u8], stride: u32, glyph: [u8; 8], x: u32, y: u32) {
    for (row, bits) in glyph.iter().enumerate() {
        for col in 0..8 {
            if bits & (1 << col) != 0 {
                put_pixel(canvas, stride, x + col, y + row as u32, COLOR_TEXT);
            }
        }
    }
}

/// Glyph bitmap for one character: ASCII printables come from [`FONT`],
/// everything else renders as `?`.
fn glyph(ch: char) -> [u8; 8] {
    if ch.is_ascii() && !ch.is_ascii_control() {
        FONT[(ch as usize) - 0x20]
    } else {
        FONT[('?' as usize) - 0x20]
    }
}

/// The title shown for a toplevel: its title if non-blank, else its app_id,
/// else a placeholder.
fn display_title(title: &str, app_id: &str) -> String {
    if !title.trim().is_empty() {
        title.trim().to_string()
    } else if !app_id.trim().is_empty() {
        app_id.trim().to_string()
    } else {
        UNTITLED.to_string()
    }
}

/// Caps a title at [`MAX_COLS`] columns, marking the cut with `..`.
fn truncate_cols(title: String) -> String {
    if title.chars().count() > MAX_COLS as usize {
        let keep: String = title.chars().take(MAX_COLS as usize - 2).collect();
        format!("{keep}..")
    } else {
        title
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
/// Why creating an overlay failed. The daemon logs this and carries on
/// without a switcher surface rather than dying mid-press.
#[derive(Debug)]
pub struct OverlayError(pub String);

impl fmt::Display for OverlayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for OverlayError {}

/// The mapped switcher overlay: one layer-shell surface backed by a shared
/// memory pool, showing one [`OverlayModel`].
///
/// Creating the surface sends an initial empty commit; the compositor
/// answers with a configure, and only then may buffers attach (attaching
/// earlier is a protocol error). Dropping the `LayerSurface` unmaps the
/// overlay.
pub struct Overlay {
    layer: LayerSurface,
    pool: SlotPool,
    model: OverlayModel,
    /// Size granted by the compositor; `None` until the first configure.
    size: Option<(u32, u32)>,
    close_at: Instant,
}

impl Overlay {
    /// Creates the overlay surface for `model` on `output` (centered, overlay
    /// layer, exclusive keyboard interactivity) and requests its first
    /// configure.
    ///
    /// # Errors
    ///
    /// Fails when the shared-memory pool cannot be created.
    pub fn new(
        model: OverlayModel,
        compositor: &CompositorState,
        layer_shell: &LayerShell,
        shm: &Shm,
        qh: &QueueHandle<App>,
        output: Option<&WlOutput>,
    ) -> Result<Self, OverlayError> {
        let surface = compositor.create_surface(qh);
        let layer =
            layer_shell.create_layer_surface(qh, surface, Layer::Overlay, Some("airlock"), output);
        // No anchor edges: the surface centers in its output.
        layer.set_anchor(Anchor::empty());
        layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
        layer.set_size(model.width, model.height);
        layer.commit();

        let len = model.width as usize * model.height as usize * 4;
        let pool = SlotPool::new(len, shm)
            .map_err(|err| OverlayError(format!("shared memory pool: {err}")))?;

        Ok(Self {
            layer,
            pool,
            model,
            size: None,
            close_at: Instant::now() + CLOSE_AFTER,
        })
    }

    /// Swaps in a fresh model. Same-sized redraws happen immediately; a size
    /// change goes through `set_size` + commit and draws on the next
    /// configure.
    pub fn update(&mut self, model: OverlayModel) {
        let unchanged = self.size == Some((model.width, model.height));
        self.close_at = Instant::now() + CLOSE_AFTER;
        let (width, height) = (model.width, model.height);
        self.model = model;
        if unchanged {
            self.draw();
        } else {
            self.layer.set_size(width, height);
            self.layer.commit();
        }
    }

    /// Applies a configure: remember the granted size and draw. A zero
    /// dimension means "pick your own", which for this surface is the model
    /// size.
    pub fn on_configure(&mut self, width: u32, height: u32) {
        let width = if width == 0 { self.model.width } else { width };
        let height = if height == 0 {
            self.model.height
        } else {
            height
        };
        self.size = Some((width, height));
        self.draw();
    }

    /// Whether the idle close deadline has passed at `now`.
    pub fn expired(&self, now: Instant) -> bool {
        now >= self.close_at
    }

    /// The model currently shown.
    pub fn model(&self) -> &OverlayModel {
        &self.model
    }

    /// The overlay's surface, for matching keyboard focus events.
    pub fn wl_surface(&self) -> &WlSurface {
        self.layer.wl_surface()
    }

    /// Renders the model into a fresh buffer and commits it. A no-op before
    /// the first configure; draw failures are logged and leave the surface
    /// as-is — the next press retries with a fresh pool slot.
    fn draw(&mut self) {
        let Some((width, height)) = self.size else {
            return;
        };
        let stride = width.saturating_mul(4);
        let Ok((buffer, canvas)) = self.pool.create_buffer(
            width as i32,
            height as i32,
            stride as i32,
            wl_shm::Format::Argb8888,
        ) else {
            eprintln!("airlock daemon: overlay buffer allocation failed");
            return;
        };
        render(&self.model, canvas, width, height, stride);
        let surface = self.layer.wl_surface();
        surface.damage_buffer(0, 0, width as i32, height as i32);
        if let Err(err) = buffer.attach_to(surface) {
            eprintln!("airlock daemon: overlay buffer attach failed: {err}");
            return;
        }
        self.layer.commit();
    }
}

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
        let model = overlay_model(&snap, Handle(4)).expect("eligible windows exist");
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
        let model = overlay_model(&snap, Handle(2)).expect("eligible windows exist");
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
        assert_eq!(overlay_model(&snap, Handle(1)), None);
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
        assert_eq!(overlay_model(&snap, Handle(2)), None);
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
        assert_eq!(overlay_model(&snap, Handle(1)), None);
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
        let model = overlay_model(&snap, Handle(1)).expect("eligible windows exist");
        assert_eq!(model.rows[0].title, "app.fallback");
        assert_eq!(model.rows[1].title, UNTITLED);
    }

    #[test]
    fn overlay_model_truncates_long_titles_with_two_dots() {
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
        let model = overlay_model(&snap, Handle(1)).expect("eligible windows exist");
        let title = &model.rows[0].title;
        assert_eq!(title.chars().count(), MAX_COLS as usize);
        assert!(title.ends_with(".."));
        assert!(title.starts_with(&"W".repeat(MAX_COLS as usize - 2)));
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
        let model = overlay_model(&short, Handle(1)).expect("eligible windows exist");
        assert_eq!(model.width, GLYPH_W + 2 * PAD_X);
        assert_eq!(model.height, (GLYPH_H + ROW_GAP) + 2 * PAD_Y);

        let tall = snapshot(
            vec![
                toplevel(1, "WWWW", "app.a", &[1], switcher::State::ACTIVATED, 3),
                toplevel(2, "WWWWWWW", "app.b", &[1], switcher::State::default(), 2),
                toplevel(3, "W", "app.c", &[1], switcher::State::default(), 1),
            ],
            Some(1),
        );
        let model = overlay_model(&tall, Handle(1)).expect("eligible windows exist");
        assert_eq!(model.width, 7 * GLYPH_W + 2 * PAD_X);
        assert_eq!(model.height, 3 * (GLYPH_H + ROW_GAP) + 2 * PAD_Y);
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
        let model = overlay_model(&snap, Handle(1)).expect("eligible windows exist");
        assert_eq!(model.width, MAX_COLS * GLYPH_W + 2 * PAD_X);
    }

    #[test]
    fn render_paints_every_pixel_with_a_known_color() {
        let snap = snapshot(
            vec![
                toplevel(1, "Alpha", "app.a", &[1], switcher::State::ACTIVATED, 2),
                toplevel(2, "Beta", "app.b", &[1], switcher::State::default(), 1),
            ],
            Some(1),
        );
        let model = overlay_model(&snap, Handle(2)).expect("eligible windows exist");
        let (w, h) = (model.width, model.height);
        let stride = w * 4;
        let mut canvas = vec![0xEE; (stride * h) as usize];
        render(&model, &mut canvas, w, h, stride);
        for chunk in canvas.as_chunks::<4>().0 {
            let pixel = u32::from_le_bytes(*chunk);
            assert!(
                pixel == COLOR_BACKGROUND || pixel == COLOR_SELECTED_BG || pixel == COLOR_TEXT,
                "unpainted pixel {pixel:#010x}"
            );
        }
    }

    #[test]
    fn render_draws_glyph_pixels_at_the_text_origin() {
        // 'A' (0x0C first row, bits 2 and 3) is the first character: its
        // set pixels prove both the text origin and the LSB-left bit order.
        // The selection is on the other row so this row shows plain
        // background around the glyphs.
        let snap = snapshot(
            vec![
                toplevel(1, "A", "app.a", &[1], switcher::State::ACTIVATED, 2),
                toplevel(2, "B", "app.b", &[1], switcher::State::default(), 1),
            ],
            Some(1),
        );
        let model = overlay_model(&snap, Handle(2)).expect("eligible windows exist");
        let (w, h) = (model.width, model.height);
        let stride = w * 4;
        let mut canvas = vec![0x00; (stride * h) as usize];
        render(&model, &mut canvas, w, h, stride);

        let pixel = |x: u32, y: u32| {
            let index = (y * stride + x * 4) as usize;
            u32::from_le_bytes(
                canvas[index..index + 4]
                    .try_into()
                    .expect("pixel-sized chunk"),
            )
        };
        assert_eq!(pixel(PAD_X, PAD_Y), COLOR_BACKGROUND);
        assert_eq!(pixel(PAD_X + 2, PAD_Y), COLOR_TEXT);
        assert_eq!(pixel(PAD_X + 3, PAD_Y), COLOR_TEXT);
        assert_eq!(pixel(PAD_X + 7, PAD_Y), COLOR_BACKGROUND);
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
        let model = overlay_model(&snap, Handle(2)).expect("eligible windows exist");
        let (w, h) = (model.width, model.height);
        let stride = w * 4;
        let mut canvas = vec![0x00; (stride * h) as usize];
        render(&model, &mut canvas, w, h, stride);

        let pixel = |x: u32, y: u32| {
            let index = (y * stride + x * 4) as usize;
            u32::from_le_bytes(
                canvas[index..index + 4]
                    .try_into()
                    .expect("pixel-sized chunk"),
            )
        };
        let row0_y = PAD_Y;
        let row1_y = PAD_Y + (GLYPH_H + ROW_GAP);
        // Left margin, inside the overlay: plain background for row 0,
        // selection background for row 1.
        assert_eq!(pixel(2, row0_y + 2), COLOR_BACKGROUND);
        assert_eq!(pixel(2, row1_y + 2), COLOR_SELECTED_BG);
        // Top padding is plain background above both rows.
        assert_eq!(pixel(2, PAD_Y / 2), COLOR_BACKGROUND);
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
        let model = overlay_model(&snap, Handle(1)).expect("eligible windows exist");
        let (w, h) = (model.width, model.height);
        let stride = w * 4 + 64; // padding between rows, like a shared pool
        let mut canvas = vec![0x11; (stride * h) as usize];
        render(&model, &mut canvas, w, h, stride);
        let index = |x: u32, y: u32| (y * stride + x * 4) as usize;
        // First glyph pixel lands at the text origin, not the row start.
        let start = index(PAD_X + 2, PAD_Y);
        let pixel = u32::from_le_bytes(
            canvas[start..start + 4]
                .try_into()
                .expect("pixel-sized chunk"),
        );
        assert_eq!(pixel, COLOR_TEXT);
    }

    #[test]
    fn glyph_maps_non_ascii_to_the_question_mark() {
        assert_eq!(glyph('é'), glyph('?'));
        assert_eq!(glyph('Ω'), glyph('?'));
        assert_eq!(glyph('\n'), glyph('?'));
        assert_eq!(glyph('A'), FONT[('A' as usize) - 0x20]);
        assert_eq!(glyph(' '), FONT[0]);
    }
}
