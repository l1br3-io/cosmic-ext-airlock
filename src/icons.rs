//! App-icon pipeline: app_id → desktop entry → icon name → themed file →
//! a ready [`ui::IconBitmap`] at the overlay's render scale.
//!
//! This module owns every byte of icon I/O so the renderer stays pure
//! data in, bytes out. The stack is pop-launcher's, per
//! `docs/research/switcher-ux-and-icons.md` §3.4: a desktop-entry corpus
//! enumerated once at daemon start (the `find_app_by_id` ladder is
//! case-insensitive and understands `StartupWMClass`, file stems, and
//! `code-url-handler`-style partial ids), then [`freedesktop_icons`]
//! for name → themed path (rooted at the `Cosmic` theme chain —
//! libcosmic's default; SVG-first because the Pop/Cosmic themes ship
//! SVG-only app icons), then SVG rasterization via `resvg` or PNG
//! decode via `png`. Benchmarked locally at 60 µs–1.2 ms per icon,
//! always at daemon start or when a window first appears — never on
//! the press path.
//!
//! Degradation is honest and one-shot per app: an app_id that resolves
//! to nothing is cached as `None` and the overlay draws the generic
//! window glyph for it (one diagnostic per app in the daemon log).

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use freedesktop_desktop_entry as fde;

use crate::ui::{self, IconBitmap};

/// Icon name tried when the desktop-entry corpus has no match or the
/// entry carries no `Icon=` — the freedesktop "unknown executable"
/// glyph, present in hicolor on normal installs. If even this misses,
/// the row falls back to the generic window glyph.
pub const FALLBACK_ICON_NAME: &str = "application-x-executable";

/// Resolves an icon *name* to a themed file. A boxed closure so tests
/// can stub the theme without touching the host's icon directories.
pub type NameLookup = Box<dyn Fn(&str, u16) -> Option<PathBuf>>;

/// The daemon's icon state: the desktop-entry corpus plus rasterized
/// icons keyed by (app_id, render scale).
///
/// Single-threaded by design — the daemon is one thread, and so is
/// every API here.
pub struct Cache {
    /// Every parseable desktop entry under the applications dirs,
    /// enumerated once at [`Cache::new`].
    entries: Vec<fde::DesktopEntry>,
    /// Name → themed path; stubbed in tests.
    lookup: NameLookup,
    /// (app_id, scale) → rasterized icon. `None` = resolved to nothing;
    /// the row draws the generic window glyph.
    rasters: HashMap<(String, u32), Option<Rc<IconBitmap>>>,
}

/// What a desktop entry's `Icon=` value asked for: a theme name, or a
/// filesystem path used as-is (the icon-theme spec's `Icon` key:
/// absolute paths bypass the theme).
#[derive(Debug, Clone, PartialEq, Eq)]
enum IconRef {
    Name(String),
    Path(PathBuf),
}

impl Default for Cache {
    /// [`Cache::new`]: enumerates the desktop-entry corpus — not a cheap
    /// default, but the only meaningful one for this type.
    fn default() -> Self {
        Self::new()
    }
}

impl Cache {
    /// Enumerates the desktop-entry corpus from the default application
    /// dirs plus the two Flatpak export locations (not always on
    /// `XDG_DATA_DIRS`). Runs once per daemon, at session start — it
    /// reads every `.desktop` on the machine, which is far too slow for
    /// any press path and irrelevant after the first gesture.
    pub fn new() -> Self {
        let mut dirs: Vec<PathBuf> = fde::default_paths().collect();
        if let Some(home) = std::env::var_os("HOME") {
            dirs.push(Path::new(&home).join(".local/share/flatpak/exports/share/applications"));
        }
        dirs.push(PathBuf::from("/var/lib/flatpak/exports/share/applications"));
        let entries: Vec<fde::DesktopEntry> = fde::Iter::new(dirs.into_iter())
            .filter_map(|path| fde::DesktopEntry::from_path(path, None::<&[String]>).ok())
            .collect();
        Self::with_entries(entries, Box::new(theme_lookup))
    }

    /// Builds a cache over an explicit corpus and lookup function — the
    /// test door. Does no filesystem access of its own.
    pub fn with_entries(entries: Vec<fde::DesktopEntry>, lookup: NameLookup) -> Self {
        Self {
            entries,
            lookup,
            rasters: HashMap::new(),
        }
    }

    /// Rasterizes every not-yet-seen (app_id, scale). Idempotent; misses
    /// are cached too, so each unknown app logs exactly one diagnostic.
    pub fn prepare(&mut self, app_ids: impl IntoIterator<Item = impl Into<String>>, scale: u32) {
        for app_id in app_ids {
            let app_id = app_id.into();
            if app_id.trim().is_empty() {
                continue;
            }
            let key = (app_id, scale);
            if self.rasters.contains_key(&key) {
                continue;
            }
            let raster = self.rasterize(&key.0, scale);
            if raster.is_none() {
                eprintln!(
                    "airlock daemon: no icon for app_id '{}' (row shows the window glyph)",
                    key.0
                );
            }
            self.rasters.insert(key, raster);
        }
    }

    /// The prepared icon for an (app_id, scale), if any. Unknown keys
    /// are a plain miss (the glyph) — resolution happens only in
    /// [`Cache::prepare`], never on the render path.
    pub fn bitmap(&self, app_id: &str, scale: u32) -> Option<Rc<IconBitmap>> {
        self.rasters
            .get(&(app_id.to_string(), scale))
            .cloned()
            .flatten()
    }

    /// app_id → what to load: the matched entry's `Icon=`, else the
    /// freedesktop unknown-app name.
    fn icon_ref(&self, app_id: &str) -> IconRef {
        let id = fde::unicase::Ascii::new(app_id);
        let icon = fde::find_app_by_id(&self.entries, id)
            .and_then(|entry| entry.icon())
            .unwrap_or(FALLBACK_ICON_NAME);
        if icon.contains('/') {
            IconRef::Path(PathBuf::from(icon))
        } else {
            IconRef::Name(icon.to_string())
        }
    }

    /// Resolves and rasterizes one app_id at `scale`; `None` means the
    /// glyph fallback.
    fn rasterize(&self, app_id: &str, scale: u32) -> Option<Rc<IconBitmap>> {
        let side = ui::ICON * scale;
        let (path, symbolic) = match self.icon_ref(app_id) {
            IconRef::Path(path) => (Some(path), false),
            IconRef::Name(name) => {
                let symbolic = name.ends_with("-symbolic");
                let size = side.min(u16::MAX as u32) as u16;
                ((self.lookup)(&name, size), symbolic)
            }
        };
        let path = path.as_deref()?; // an unresolvable name is the glyph
        let mut bitmap = match path.extension().and_then(|ext| ext.to_str()) {
            Some("svg") => rasterize_svg(path, side),
            Some("png") => rasterize_png(path, side),
            _ => None,
        }?;
        if symbolic {
            // Symbolic icons are monochrome shapes that carry their
            // color as `currentColor` or no fill at all; rendered raw
            // they come out black and vanish on the card. Recolor to
            // the text color, keeping coverage.
            tint_text_color(&mut bitmap);
        }
        Some(Rc::new(bitmap))
    }
}

/// The production name → path lookup: rooted at the `Cosmic` theme chain
/// (libcosmic's default; its `Inherits=` walks Pop → hicolor), hicolor
/// fallback and `/usr/share/pixmaps` built in. `with_cache` keeps the
/// theme scan to once per name.
fn theme_lookup(name: &str, size: u16) -> Option<PathBuf> {
    freedesktop_icons::lookup(name)
        .with_theme("Cosmic")
        .with_size(size)
        .with_cache()
        .find()
}

/// Renders an SVG icon into a `side × side` bitmap, fitted and centered
/// with the aspect ratio kept. Benchmarked 60 µs–1.2 ms at 16–64 px.
fn rasterize_svg(path: &Path, side: u32) -> Option<IconBitmap> {
    let data = fs::read(path).ok()?;
    let tree = resvg::usvg::Tree::from_data(&data, &resvg::usvg::Options::default()).ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(side, side)?;
    let size = tree.size();
    let scale = (side as f32 / size.width()).min(side as f32 / size.height());
    let dx = (side as f32 - size.width() * scale) / 2.0;
    let dy = (side as f32 - size.height() * scale) / 2.0;
    let transform = resvg::tiny_skia::Transform::from_scale(scale, scale).post_translate(dx, dy);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    let argb = pixmap
        .pixels()
        .iter()
        .map(|p| {
            // tiny-skia pixels are already premultiplied; pack them in
            // the renderer's 0xAARRGGBB order.
            (p.alpha() as u32) << 24
                | (p.red() as u32) << 16
                | (p.green() as u32) << 8
                | (p.blue() as u32)
        })
        .collect();
    Some(IconBitmap { side, argb })
}

/// Decodes a PNG icon and box-resamples it to `side × side` in the
/// premultiplied domain (linear filtering is only correct there).
fn rasterize_png(path: &Path, side: u32) -> Option<IconBitmap> {
    let file = fs::File::open(path).ok()?;
    let decoder = png::Decoder::new(file);
    let mut reader = decoder.read_info().ok()?;
    let (width, height) = reader.info().size();
    let mut bytes = vec![0; reader.output_buffer_size()];
    reader.next_frame(&mut bytes).ok()?;
    let color = reader.info().color_type;
    let argb = rgba_argb(&bytes, width, height, color)?;
    Some(IconBitmap {
        side,
        argb: resample(argb, width, height, side),
    })
}

/// Assembles straight-alpha PNG bytes into premultiplied
/// `0xAARRGGBB` pixels. RGB and RGBA pass through (alpha 255 for RGB);
/// grayscale and palette images are rejected honestly — themed app
/// icons are RGBA in practice, and the rest of the `png` crate's format
/// surface is not worth carrying for the remainder.
fn rgba_argb(bytes: &[u8], width: u32, height: u32, color: png::ColorType) -> Option<Vec<u32>> {
    let channels = match color {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        _ => return None,
    };
    let expected = width as usize * height as usize * channels;
    if bytes.len() < expected {
        return None;
    }
    let mut argb = Vec::with_capacity(width as usize * height as usize);
    for px in bytes[..expected].chunks_exact(channels) {
        let (r, g, b, a) = match channels {
            4 => (px[0], px[1], px[2], px[3]),
            _ => (px[0], px[1], px[2], 255),
        };
        argb.push(premultiply(r, g, b, a));
    }
    Some(argb)
}

/// One straight-alpha pixel to premultiplied `0xAARRGGBB`, rounding
/// half-up.
fn premultiply(r: u8, g: u8, b: u8, a: u8) -> u32 {
    let a = a as u32;
    (a << 24)
        | ((((r as u32) * a + 127) / 255) << 16)
        | ((((g as u32) * a + 127) / 255) << 8)
        | (((b as u32) * a + 127) / 255)
}

/// Area-averaged box resample of a premultiplied image to a square
/// `side × side`. Downscaling averages exact source-area coverage
/// (fractional window edges included — 96→64 averages honestly);
/// upscaling is nearest-neighbor, which matches the renderer's
/// crisp-pixel aesthetic instead of pretending at softness it cannot
/// draw.
fn resample(src: Vec<u32>, src_w: u32, src_h: u32, side: u32) -> Vec<u32> {
    if src_w == side && src_h == side {
        return src;
    }
    let mut dst = Vec::with_capacity(side as usize * side as usize);
    let scale_x = src_w as f32 / side as f32;
    let scale_y = src_h as f32 / side as f32;
    if src_w <= side || src_h <= side {
        // Upscale (or one axis already exact): nearest source pixel.
        for y in 0..side {
            for x in 0..side {
                let sx = ((x as f32 + 0.5) * scale_x) as u32;
                let sy = ((y as f32 + 0.5) * scale_y) as u32;
                let index =
                    sy.min(src_h - 1) as usize * src_w as usize + sx.min(src_w - 1) as usize;
                dst.push(src[index]);
            }
        }
        return dst;
    }
    // Downscale: every destination pixel is the coverage-weighted
    // average of the source cells its window overlaps. Weights are
    // overlap lengths in 1/256 fixed point; the final division makes
    // the actual average, so their absolute scale cancels.
    for y in 0..side {
        let (y0, y1) = (y as f32 * scale_y, (y + 1) as f32 * scale_y);
        for x in 0..side {
            let (x0, x1) = (x as f32 * scale_x, (x + 1) as f32 * scale_x);
            let (mut sr, mut sg, mut sb, mut sa, mut area) = (0u32, 0u32, 0u32, 0u32, 0u32);
            for py in y0.floor() as u32..y1.ceil() as u32 {
                let cy = overlap(py as f32, y0, y1);
                for px in x0.floor() as u32..x1.ceil() as u32 {
                    let weight = overlap(px as f32, x0, x1) * cy;
                    let p = src
                        [py.min(src_h - 1) as usize * src_w as usize + px.min(src_w - 1) as usize];
                    sr += ((p >> 16) & 0xFF) * weight;
                    sg += ((p >> 8) & 0xFF) * weight;
                    sb += (p & 0xFF) * weight;
                    sa += (p >> 24) * weight;
                    area += weight;
                }
            }
            if area == 0 {
                dst.push(0);
                continue;
            }
            dst.push(((sa / area) << 24) | ((sr / area) << 16) | ((sg / area) << 8) | (sb / area));
        }
    }
    dst
}

/// Overlap of source cell `[cell, cell+1)` with window `[lo, hi)`, in
/// 1/256 fixed point (0..=256).
fn overlap(cell: f32, lo: f32, hi: f32) -> u32 {
    let length = (cell + 1.0).min(hi) - cell.max(lo);
    ((length.max(0.0) * 256.0) as u32).min(256)
}

/// Recolors a bitmap to [`ui::COLOR_TEXT`], keeping per-pixel coverage —
/// symbolic icons carry their shape in alpha only.
fn tint_text_color(bitmap: &mut IconBitmap) {
    // 0xAARRGGBB → big-endian bytes are [A, R, G, B].
    let [_, tr, tg, tb] = ui::COLOR_TEXT.to_be_bytes();
    for p in &mut bitmap.argb {
        let a = *p >> 24;
        *p = (a << 24)
            | ((((tr as u32) * a + 127) / 255) << 16)
            | ((((tg as u32) * a + 127) / 255) << 8)
            | (((tb as u32) * a + 127) / 255);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    /// A 10×10 SVG: the left half opaque red, the right half empty.
    const RED_HALF_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><rect x="0" y="0" width="5" height="10" fill="#ff0000"/></svg>"##;

    /// Fixtures live under an `applications/` directory — the spec-shaped
    /// path — so the crate derives the same desktop-file id from the file
    /// name that production derives under `/usr/share/applications`.
    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("airlock-icons")
            .join(std::process::id().to_string())
            .join("applications");
        std::fs::create_dir_all(&dir).expect("fixture dir");
        dir.join(name)
    }

    fn svg_fixture(name: &str) -> PathBuf {
        let path = temp_path(name);
        fs::write(&path, RED_HALF_SVG).expect("write svg fixture");
        path
    }

    fn png_fixture(name: &str, side: u8, color: [u8; 4]) -> PathBuf {
        let path = temp_path(name);
        let file = fs::File::create(&path).expect("create png fixture");
        let mut encoder = png::Encoder::new(file, side as u32, side as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("png header");
        let mut rgba = Vec::with_capacity(side as usize * side as usize * 4);
        for _ in 0..side as usize * side as usize {
            rgba.extend_from_slice(&color);
        }
        writer.write_image_data(&rgba).expect("png pixels");
        path
    }

    /// A desktop-entry corpus built from inline file bodies in a temp
    /// dir; `(file name, file body)` pairs.
    fn corpus(files: &[(&str, &str)]) -> Vec<fde::DesktopEntry> {
        files
            .iter()
            .map(|(name, body)| {
                let path = temp_path(name);
                fs::write(&path, body).expect("write desktop fixture");
                fde::DesktopEntry::from_path(path, None::<&[String]>)
                    .expect("parse desktop fixture")
            })
            .collect()
    }

    /// A stub lookup serving exactly one icon name → path, counting
    /// calls and panicking on any other name.
    fn stub(name: &'static str, path: PathBuf) -> (NameLookup, Rc<Cell<u32>>) {
        let calls = Rc::new(Cell::new(0));
        let counter = Rc::clone(&calls);
        let lookup: NameLookup = Box::new(move |asked, _size| {
            counter.set(counter.get() + 1);
            assert_eq!(asked, name, "unexpected icon name asked: {asked}");
            Some(path.clone())
        });
        (lookup, calls)
    }

    fn argb(pixel: u32) -> [u8; 4] {
        pixel.to_be_bytes() // [A, R, G, B]
    }

    #[test]
    fn entry_icon_name_wins_and_is_looked_up() {
        let svg = svg_fixture("entry-icon.svg");
        let (lookup, calls) = stub("org.test.icon", svg);
        let mut cache = Cache::with_entries(
            corpus(&[(
                "org.test.App.desktop",
                "[Desktop Entry]\nType=Application\nName=Test\nIcon=org.test.icon\n",
            )]),
            lookup,
        );
        cache.prepare(["org.test.app"], 2); // case-insensitive id match
        let bitmap = cache.bitmap("org.test.app", 2).expect("icon rasterized");
        assert_eq!(bitmap.side, ui::ICON * 2);
        assert_eq!(calls.get(), 1, "exactly one theme lookup");
        // The SVG's left half is opaque red.
        let mid = bitmap.side as usize / 2;
        assert_eq!(argb(bitmap.argb[0]), [255, 255, 0, 0]);
        assert_eq!(bitmap.argb[mid * bitmap.side as usize + mid] >> 24, 0);
    }

    #[test]
    fn no_entry_falls_back_to_the_unknown_executable_name() {
        let svg = svg_fixture("unknown-exec.svg");
        let calls = Rc::new(Cell::new(0));
        let counter = Rc::clone(&calls);
        let lookup: NameLookup = Box::new(move |asked, _size| {
            counter.set(counter.get() + 1);
            assert_eq!(asked, FALLBACK_ICON_NAME);
            Some(svg.clone())
        });
        let mut cache = Cache::with_entries(Vec::new(), lookup);
        cache.prepare(["nothing.matches"], 1);
        assert!(cache.bitmap("nothing.matches", 1).is_some());
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn an_unresolvable_name_caches_the_glyph_fallback() {
        let lookup: NameLookup = Box::new(|_name, _size| None);
        let mut cache = Cache::with_entries(Vec::new(), lookup);
        cache.prepare(["ghost.app"], 1);
        assert_eq!(cache.bitmap("ghost.app", 1), None);
    }

    #[test]
    fn entry_without_icon_uses_the_fallback_name() {
        let svg = svg_fixture("no-icon-entry.svg");
        let (lookup, _calls) = stub(FALLBACK_ICON_NAME, svg);
        let mut cache = Cache::with_entries(
            corpus(&[(
                "Bare.desktop",
                "[Desktop Entry]\nType=Application\nName=Bare\n",
            )]),
            lookup,
        );
        cache.prepare(["bare"], 1);
        assert!(cache.bitmap("bare", 1).is_some());
    }

    #[test]
    fn absolute_icon_path_bypasses_the_theme() {
        let svg = svg_fixture("absolute-icon.svg");
        let calls = Rc::new(Cell::new(0));
        let counter = Rc::clone(&calls);
        let lookup: NameLookup = Box::new(move |_name, _size| {
            counter.set(counter.get() + 1);
            None
        });
        let path_display = svg.display().to_string();
        let mut cache = Cache::with_entries(
            corpus(&[(
                "org.test.Path.desktop",
                &format!("[Desktop Entry]\nType=Application\nName=Path\nIcon={path_display}\n"),
            )]),
            lookup,
        );
        cache.prepare(["org.test.path"], 1);
        assert!(cache.bitmap("org.test.path", 1).is_some());
        assert_eq!(calls.get(), 0, "theme never consulted for a path Icon=");
    }

    #[test]
    fn symbolic_names_are_tinted_to_the_text_color() {
        let svg = svg_fixture("symbolic.svg");
        let (lookup, _calls) = stub("org.test.fancy-symbolic", svg);
        let mut cache = Cache::with_entries(
            corpus(&[(
                "org.test.Fancy.desktop",
                "[Desktop Entry]\nType=Application\nName=Fancy\nIcon=org.test.fancy-symbolic\n",
            )]),
            lookup,
        );
        cache.prepare(["org.test.fancy"], 1);
        let bitmap = cache.bitmap("org.test.fancy", 1).expect("icon rasterized");
        // Opaque source pixel keeps alpha 255 but wears the text color.
        assert_eq!(argb(bitmap.argb[0])[0], 255);
        assert_eq!(
            &argb(bitmap.argb[0])[1..],
            &ui::COLOR_TEXT.to_be_bytes()[1..],
            "symbolic shape recolored to the text color"
        );
    }

    #[test]
    fn png_icons_decode_and_downsample() {
        let png = png_fixture("solid.png", 64, [0x10, 0x20, 0x30, 0xFF]);
        let (lookup, _calls) = stub("org.test.pngicon", png);
        let mut cache = Cache::with_entries(
            corpus(&[(
                "org.test.Png.desktop",
                "[Desktop Entry]\nType=Application\nName=Png\nIcon=org.test.pngicon\n",
            )]),
            lookup,
        );
        cache.prepare(["org.test.png"], 2); // 64 → side 64: 1:1
        let bitmap = cache.bitmap("org.test.png", 2).expect("icon rasterized");
        assert_eq!(bitmap.side, 64);
        assert_eq!(argb(bitmap.argb[0]), [255, 0x10, 0x20, 0x30]);
    }

    #[test]
    fn prepare_is_idempotent_per_app_and_scale() {
        let svg = svg_fixture("idempotent.svg");
        let (lookup, calls) = stub("org.test.again", svg);
        let mut cache = Cache::with_entries(
            corpus(&[(
                "org.test.Again.desktop",
                "[Desktop Entry]\nType=Application\nName=Again\nIcon=org.test.again\n",
            )]),
            lookup,
        );
        cache.prepare(["org.test.again"], 1);
        cache.prepare(["org.test.again"], 1);
        assert_eq!(calls.get(), 1, "second prepare is a cache hit");
        cache.prepare(["org.test.again"], 2);
        assert_eq!(calls.get(), 2, "a new scale resolves again");
    }

    #[test]
    fn empty_app_ids_skip_resolution() {
        let calls = Rc::new(Cell::new(0));
        let counter = Rc::clone(&calls);
        let lookup: NameLookup = Box::new(move |_name, _size| {
            counter.set(counter.get() + 1);
            None
        });
        let mut cache = Cache::with_entries(Vec::new(), lookup);
        cache.prepare(["", "  "], 1);
        assert_eq!(calls.get(), 0);
        assert_eq!(cache.bitmap("", 1), None);
    }

    #[test]
    fn premultiply_rounds_half_up() {
        // 255 × 128 / 255 = 128 exactly; 254 × 128 / 255 ≈ 127.5 stays 127.
        assert_eq!(premultiply(255, 0, 0, 128), 0x8080_0000);
        assert_eq!((premultiply(254, 0, 0, 128) >> 16) & 0xFF, 127);
    }

    #[test]
    fn resample_divisible_downscale_averages() {
        let red = 0xFF_FF_00_00;
        let green = 0xFF_00_FF_00;
        let src = vec![red, green, green, red];
        let dst = resample(src, 2, 2, 1);
        // Equal parts of both colors: r = g = 127 after rounding down.
        assert_eq!(dst[0], 0xFF_7F_7F_00);
    }

    #[test]
    fn resample_fractional_windows_average_by_coverage() {
        // 3×3 column stripes red/green/blue → side 2: window edges fall at
        // 1.5 on both axes. Window (0,0) = full red with half coverage of
        // green on each axis → r=170, g=85; window (1,0) = half green +
        // full blue → g=85, b=170 — the 1/256 fixed-point divisions come
        // out exact.
        let row = [0xFF_FF_00_00, 0xFF_00_FF_00, 0xFF_00_00_FF];
        let src: Vec<u32> = row.iter().cycle().take(9).copied().collect();
        let dst = resample(src, 3, 3, 2);
        assert_eq!(
            dst,
            vec![0xFF_AA_55_00, 0xFF_00_55_AA, 0xFF_AA_55_00, 0xFF_00_55_AA]
        );
    }

    #[test]
    fn resample_upscale_is_nearest() {
        let src = vec![0xFF_10_20_30, 0xFF_10_20_30, 0xFF_10_20_30, 0xFF_10_20_30];
        let dst = resample(src, 2, 2, 4);
        assert_eq!(dst.len(), 16);
        assert!(dst.iter().all(|p| *p == 0xFF_10_20_30));
    }

    /// The brand hatch icon ships in-repo and must rasterize at every
    /// overlay-relevant size (it also proves the asset is valid SVG).
    #[test]
    fn brand_hatch_icon_renders_at_overlay_sizes() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("res/icons/hicolor/scalable/apps/airlock.svg");
        for side in [16, 32, 64] {
            let bitmap = rasterize_svg(&path, side).expect("hatch renders");
            assert_eq!(bitmap.side, side);
            let lit = bitmap.argb.iter().filter(|p| *p >> 24 == 0xFF).count();
            // The ring alone guarantees a few hundred opaque pixels at
            // any size; a blank render fails this immediately.
            assert!(
                lit > side as usize * 2,
                "{side}px: only {lit} opaque pixels"
            );
        }
    }

    /// Integration against the host's real theme. On a machine without
    /// the Cosmic theme this is a silent no-op; the VM pass is the gate
    /// that matters.
    #[test]
    fn theme_lookup_finds_a_cosmic_app_icon() {
        if !Path::new("/usr/share/icons/Cosmic").exists() {
            return;
        }
        let path = theme_lookup("utilities-terminal", 64);
        assert!(
            path.is_some(),
            "utilities-terminal should resolve through the Cosmic chain"
        );
    }
}
