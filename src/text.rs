//! The card's system-text backend (`typography`): cosmic-text shaping,
//! rasterization, and the glyph cache behind the switcher's titles.
//!
//! The [`TextSystem`] owns one [`FontSystem`] for the daemon's whole
//! life — created at daemon start (OQ 1: the font scan is tens-of-
//! milliseconds-class and is paid at login, never on the first gesture;
//! fonts installed after startup stay unseen until the daemon restarts,
//! a documented limitation). Shaping runs per title at the pinned 12 px
//! through cosmic-text's fallback chains (`Shaping::Advanced`), so
//! CJK, emoji, and accented runs resolve their real glyphs — folding is
//! the bitmap floor's behavior, not this backend's.
//!
//! Every glyph rasterizes into an 8-bit coverage mask cached by
//! `CacheKey` — cosmic-text's own (face, glyph, device size) identity —
//! whole-pixel by construction: positions are binned to
//! [`SubpixelBin::Zero`] so a glyph's raster never depends on where the
//! pen sits (grayscale AA on the outline edges, never subpixel RGB).
//! The cache is bounded ([`GLYPH_CACHE_CAP`]) and cleared when the
//! shaped family changes (OQ 3).

// TY1 lands the backend's ownership, cache, and deterministic fixture
// path; the System render path (TY3) and the `font` key (TY4) consume
// the rest of this surface. Until then only the daemon's startup scan
// and the tests touch it, so the not-yet-wired items carry this one
// allowance — removed when TY3 lands (the GE1→GE3 precedent).
#![allow(dead_code)]

use std::collections::HashMap;
use std::path::Path;

use cosmic_text::{
    Attrs, Buffer, CacheKey, Family, FontSystem, Hinting, Metrics, Shaping, SubpixelBin, Weight,
    Wrap,
};

/// The one face size this backend paints, logical px — the 11-07 12-px
/// pin, unchanged by `typography` (the COSMIC store carries no size to
/// follow; family-only is the honest reading). The bitmap side pins the
/// same height through `ui::FONT_SCALE_NUM/DEN`; the assert keeps the
/// two pins one value.
pub const TEXT_PX: f32 = 12.0;

const _: () = {
    assert!(TEXT_PX as u32 == crate::ui::TEXT_W);
};

/// Upper bound on cached glyph masks (OQ 3's pinned number). A full
/// card — ten rows of titles plus hint bands, fallback faces included —
/// paints a few hundred distinct glyphs at one size; 1024 holds a
/// session's working set with slack. A cache at the bound clears whole
/// and refills on demand: the cost is one re-rasterization pass over
/// the visible card, never a stall.
pub const GLYPH_CACHE_CAP: usize = 1024;

/// Which family a shape asks for: the generic sans query (the fixture
/// db's Open Sans, the system db's real default) or a configured name.
/// Resolution is paint-time by design — an unresolvable name never
/// fails here, it shapes through cosmic-text's fallback and the caller
/// logs the diagnostic (the `font` key's never-a-broken-switch rule).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FontQuery<'a> {
    SansSerif,
    Named(&'a str),
}

/// The family a [`TextSystem`] is currently shaping — the cache's
/// warm-for marker: a change clears the mask cache (OQ 3).
#[derive(Debug, Clone, PartialEq, Eq)]
enum ResolvedFamily {
    SansSerif,
    Named(String),
}

/// One whole-pixel-positioned glyph: the cache key that rasterizes it
/// and its device-pixel offset from the run's pen origin at the
/// baseline. `y` is negative above the baseline, as shaped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PositionedGlyph {
    pub cache_key: CacheKey,
    pub x: i32,
    pub y: i32,
}

/// One shaped title: its positioned glyphs (device px, relative to a
/// pen at the origin on the baseline — the renderer adds the row's
/// title origin and baseline) plus the run's metrics in logical px the
/// card's centering math reads.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GlyphRun {
    pub glyphs: Vec<PositionedGlyph>,
    /// Advance width of the whole run, logical px.
    pub width_px: f32,
    /// Font ascent above the baseline, logical px (positive up).
    pub ascent_px: f32,
    /// Font descent below the baseline, logical px (positive down).
    pub descent_px: f32,
}

/// One rasterized glyph: 8-bit coverage over a `width × height` block
/// whose top-left sits `(left, top)` device px from the glyph's pen
/// position at the baseline (`top` is ≤ 0 for ink above the baseline).
/// Grayscale AA — the coverage value is the pixel's blend factor, the
/// same ramp the renderer's corners use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlyphMask {
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
    pub coverage: Vec<u8>,
}

/// The bounded glyph-mask cache (OQ 3): one mask per `CacheKey`, whole
/// clear when full, whole clear on family change. Masks are pure data —
/// rasterization populates them, the renderer composites them, and
/// neither can grow the cache past its bound.
#[derive(Debug, Default)]
pub struct GlyphCache {
    masks: HashMap<CacheKey, GlyphMask>,
}

impl GlyphCache {
    /// The cached mask for `key`, if rasterized.
    pub fn get(&self, key: &CacheKey) -> Option<&GlyphMask> {
        self.masks.get(key)
    }

    /// Caches `mask` under `key`. A full cache clears whole first —
    /// the bound is absolute, the refill is on demand.
    pub fn insert(&mut self, key: CacheKey, mask: GlyphMask) {
        if self.masks.len() >= GLYPH_CACHE_CAP && !self.masks.contains_key(&key) {
            self.masks.clear();
        }
        self.masks.insert(key, mask);
    }

    /// Distinct masks held — the bound's observable.
    pub fn len(&self) -> usize {
        self.masks.len()
    }

    /// Whether the cache holds nothing.
    pub fn is_empty(&self) -> bool {
        self.masks.is_empty()
    }

    /// Drops every mask (family change, OQ 3).
    pub fn clear(&mut self) {
        self.masks.clear();
    }
}

/// The system-text backend: the daemon's one [`FontSystem`], the mask
/// cache it rasterizes through, and the family that cache is warm for.
/// Created once at daemon start and borrowed by every gesture — shaping
/// and rasterization are µs-class warm, and nothing here ever fails a
/// switch (an empty font database paints the bitmap card instead, the
/// zero-fonts floor).
#[derive(Debug)]
pub struct TextSystem {
    fonts: FontSystem,
    cache: GlyphCache,
    family: ResolvedFamily,
}

impl TextSystem {
    /// The daemon's system: one font scan at startup (OQ 1) — the
    /// scan's cost lands at login, never on the first gesture.
    pub fn scan_system() -> Self {
        Self::with_fonts(FontSystem::new())
    }

    /// A deterministic system for tests and sheets: a fresh database
    /// holding only the font files under `dir` — never the system scan
    /// (`FontSystem::new_with_fonts` would scan anyway; the empty db
    /// goes in through `new_with_locale_and_db`, and the unnameable
    /// `fontdb::Database` value comes from `Default` by inference). The
    /// generic families keep cosmic-text's own defaults (sans =
    /// Open Sans), which the fixture face is; the locale is pinned so
    /// fallback order never rides the host's environment.
    pub fn from_dir(dir: &Path) -> Self {
        let mut fonts = FontSystem::new_with_locale_and_db("en".into(), Default::default());
        {
            let db = fonts.db_mut();
            db.set_monospace_family("Noto Sans Mono");
            db.set_sans_serif_family("Open Sans");
            db.set_serif_family("DejaVu Serif");
            db.load_fonts_dir(dir);
        }
        Self::with_fonts(fonts)
    }

    /// Faces the database holds. Zero is the zero-fonts floor's marker:
    /// the daemon paints the bitmap card (SC 5).
    pub fn face_count(&self) -> usize {
        self.fonts.db().faces().count()
    }

    /// The glyph-mask cache (tests and the rasterizer share it).
    pub fn cache(&mut self) -> &mut GlyphCache {
        &mut self.cache
    }

    /// Shapes `text` at [`TEXT_PX`] × `scale` device px in `query`'s
    /// family, with fallback chains. Positions are whole device pixels
    /// relative to a pen at the origin on the baseline; a family change
    /// clears the mask cache first. An empty or unshapable text yields
    /// an empty run — the caller's `(untitled)` path already owns that
    /// case.
    pub fn shape(&mut self, text: &str, query: FontQuery<'_>, scale: u32) -> GlyphRun {
        let resolved = match query {
            FontQuery::SansSerif => ResolvedFamily::SansSerif,
            FontQuery::Named(name) => ResolvedFamily::Named(name.to_owned()),
        };
        if self.family != resolved {
            self.cache.clear();
            self.family = resolved;
        }
        if text.is_empty() {
            return GlyphRun::default();
        }
        let family = match &query {
            FontQuery::SansSerif => Family::SansSerif,
            FontQuery::Named(name) => Family::Name(name),
        };
        let attrs = Attrs::new().family(family).weight(Weight::NORMAL);
        // One line, no wrap, at the pinned logical size: `physical`
        // scales to `scale` device px per glyph below. The line height
        // never matters — titles are single-line and the card's math
        // reads ascent/descent, not the line box.
        let mut buffer = Buffer::new(&mut self.fonts, Metrics::new(TEXT_PX, TEXT_PX));
        buffer.set_wrap(Wrap::None);
        buffer.set_text(text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.fonts, false);
        let Some(run) = buffer.layout_runs().next() else {
            return GlyphRun::default();
        };
        // Copied out before the shaping read below mutates the buffer
        // (the run borrows it).
        let (line_i, line_w) = (run.line_i, run.line_w);
        let mut glyphs = Vec::with_capacity(run.glyphs.len());
        for glyph in run.glyphs {
            let physical = glyph.physical((0.0, 0.0), scale as f32);
            // Rebuild the key at whole-pixel bins: a glyph's raster
            // must not depend on where the pen sat, or the cache key
            // would (and subpixel-RGB coverage is not this design).
            glyphs.push(PositionedGlyph {
                cache_key: CacheKey {
                    x_bin: SubpixelBin::Zero,
                    y_bin: SubpixelBin::Zero,
                    ..physical.cache_key
                },
                x: physical.x,
                y: physical.y,
            });
        }
        // Ascent/descent live on the laid-out line, which the run
        // borrows but does not expose: read them from the line's cached
        // shape (shaping already happened above; this second layout
        // pass is arithmetic over cached glyphs, µs-class). The tab
        // width 8 is Buffer's own default (`buffer.rs` seeds it).
        let (ascent_px, descent_px) = buffer
            .lines
            .get_mut(line_i)
            .map(|line| {
                line.shape(&mut self.fonts, 8)
                    .layout(TEXT_PX, None, Wrap::None, None, None, Hinting::Disabled)
            })
            .and_then(|lines| lines.first().map(|layout| (layout.max_ascent, layout.max_descent)))
            .unwrap_or((0.0, 0.0));
        GlyphRun {
            glyphs,
            width_px: line_w,
            ascent_px,
            descent_px,
        }
    }

    /// Assembles a system around `fonts` with a cold cache.
    fn with_fonts(fonts: FontSystem) -> Self {
        TextSystem {
            fonts,
            cache: GlyphCache::default(),
            family: ResolvedFamily::SansSerif,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// The committed fixture directory (see `res/test-fonts/README.md`).
    fn fixture_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("res/test-fonts")
    }

    #[test]
    fn fixture_shapes_ascii_into_positioned_glyphs_deterministically() {
        let mut first = TextSystem::from_dir(&fixture_dir());
        let mut second = TextSystem::from_dir(&fixture_dir());
        assert!(first.face_count() >= 1, "the fixture db holds its face");

        let run_a = first.shape("Airlock", FontQuery::SansSerif, 2);
        let run_b = second.shape("Airlock", FontQuery::SansSerif, 2);

        assert!(
            !run_a.glyphs.is_empty(),
            "ASCII shapes into positioned glyphs"
        );
        assert!(run_a.width_px > 0.0, "the run carries a width");
        assert!(run_a.ascent_px > 0.0, "the run carries an ascent");
        assert!(run_a.descent_px >= 0.0, "the run carries a descent");
        // Same bytes in, same run out — across fresh systems, the
        // determinism the tests and sheets rely on.
        assert_eq!(run_a, run_b);
    }

    #[test]
    fn empty_directory_is_the_zero_fonts_floor_marker() {
        let mut system = TextSystem::from_dir(Path::new("/nonexistent-airlock-test-fonts"));
        assert_eq!(system.face_count(), 0);
        // Shaping against an empty db still answers: an empty run, not
        // a panic — switching never breaks on font I/O.
        let run = system.shape("", FontQuery::SansSerif, 2);
        assert!(run.glyphs.is_empty());
    }

    #[test]
    fn glyph_cache_respects_its_bound_under_synthetic_load() {
        let mut cache = GlyphCache::default();
        // A seed key whose font id comes from inference (`fontdb::ID`
        // is Default but not nameable through cosmic-text's exports);
        // no font is touched — the bound is pure cache behavior.
        // `CacheKey::new` also returns the binned position; the cache
        // identity is the key alone.
        let (seed, _, _) = CacheKey::new(
            Default::default(),
            0,
            TEXT_PX,
            (0.0, 0.0),
            Weight::NORMAL,
            cosmic_text::CacheKeyFlags::empty(),
        );
        let mask = GlyphMask {
            left: 0,
            top: 0,
            width: 1,
            height: 1,
            coverage: vec![0xFF],
        };
        // 8 × the cap in distinct keys: the bound must hold throughout.
        for glyph_id in 0..(GLYPH_CACHE_CAP * 8) as u16 {
            cache.insert(CacheKey { glyph_id, ..seed }, mask.clone());
            assert!(
                cache.len() <= GLYPH_CACHE_CAP,
                "the cache never grows past its bound"
            );
        }
        // A cache at the bound still serves an insert (it cleared and
        // refilled), and clear() empties it for the family change.
        cache.clear();
        assert!(cache.is_empty());
    }
}
