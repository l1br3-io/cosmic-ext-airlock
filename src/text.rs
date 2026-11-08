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

// The binary consumes the System path from TY4, when the daemon
// resolves the backend per open (`FontQuery::Named` is the configured
// key's arm, `from_dir`/`face_count` the floor's); until then only the
// lib's render path and the tests touch this surface. Removed in TY4
// (the GE1→GE3 precedent).
#![allow(dead_code)]

use std::collections::HashMap;
use std::path::Path;

use cosmic_text::{
    Attrs, Buffer, CacheKey, Family, FontSystem, Hinting, Metrics, Shaping, SubpixelBin,
    SwashCache, SwashContent, SwashImage, Weight, Wrap,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// How many leading units of `advances` (per-cluster measured widths,
/// logical px) fit `budget_px` once the ellipsis's own width rides
/// along: the largest `k` with `advances[..k] + ellipsis_px ≤ budget`.
/// Pure over measured widths — no font present, testable with
/// synthetic data. `0` when even the ellipsis alone cannot fit; the
/// whole input when everything fits.
pub fn truncate_to_budget(advances: &[f32], ellipsis_px: f32, budget_px: f32) -> usize {
    let mut kept = 0.0f32;
    let mut count = 0usize;
    for &advance in advances {
        if kept + advance + ellipsis_px > budget_px {
            break;
        }
        kept += advance;
        count += 1;
    }
    count
}

/// The baseline's y position inside a row `row_h` logical px tall, for
/// a run with these font metrics: the ink box (ascent + descent)
/// centers in the row, the baseline sits `ascent` below the ink top,
/// and the answer rounds to a whole logical pixel — the System path's
/// whole-pixel pin (the Bitmap path centers on its `TEXT_H` cells, its
/// own rule). Clamped inside the row so degenerate metrics can never
/// place ink outside it.
pub fn centering(ascent_px: f32, descent_px: f32, row_h: u32) -> u32 {
    let ink = (ascent_px + descent_px).max(0.0);
    let baseline = (row_h as f32 - ink) / 2.0 + ascent_px;
    baseline.clamp(0.0, row_h as f32).round() as u32
}

/// One rasterized glyph: 8-bit coverage over a `width × height` block
/// whose top-left sits `(left, top)` device px from the glyph's pen
/// position at the baseline (`top` is ≤ 0 for ink above the baseline).
/// Grayscale AA — the coverage value is the pixel's blend factor toward
/// the paint color, the same ramp the renderer's corners use. Color
/// bitmaps (CBDT/COLR emoji, the fallback chains' pick) carry their own
/// `colors` per pixel; their coverage is the pixel's alpha, so the
/// glyph paints as the small image it is — the same color-glyph
/// honesty the card's themed icons already practice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlyphMask {
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
    pub coverage: Vec<u8>,
    /// Straight RGB per pixel, present only for color-bitmap glyphs.
    pub colors: Option<Vec<[u8; 3]>>,
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
    swash: SwashCache,
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

    /// The rasterized mask for `key`, rasterizing through swash on the
    /// first ask and recalling it from the bounded cache after. `None`
    /// when swash cannot rasterize the glyph at all (no outline, no
    /// bitmap) — the caller skips it, exactly the glyph cosmic-text's
    /// own renderer would not paint.
    pub fn mask(&mut self, key: &CacheKey) -> Option<&GlyphMask> {
        if self.cache.get(key).is_none() {
            let image = self.swash.get_image_uncached(&mut self.fonts, *key)?;
            let mask = mask_from_image(&image)?;
            self.cache.insert(*key, mask);
        }
        self.cache.get(key)
    }

    /// Shapes `text` at [`TEXT_PX`] × `scale` device px in `query`'s
    /// family, with fallback chains. Positions are whole device pixels
    /// relative to a pen at the origin on the baseline; a family change
    /// clears the mask cache first. An empty or unshapable text yields
    /// an empty run — the caller's `(untitled)` path already owns that
    /// case.
    pub fn shape(&mut self, text: &str, query: FontQuery<'_>, scale: u32) -> GlyphRun {
        self.shape_with_clusters(text, query, scale).0
    }

    /// [`shape`] plus the per-cluster facts truncation needs: each
    /// cluster's end byte in `text` and the advance it consumed. A
    /// cluster is the byte range one user-perceived character shaped
    /// into (a base char, or a base with combining marks, or a
    /// fallback-script run member) — the unit a budget cut keeps whole.
    fn shape_with_clusters(
        &mut self,
        text: &str,
        query: FontQuery<'_>,
        scale: u32,
    ) -> (GlyphRun, Vec<Cluster>) {
        let resolved = match query {
            FontQuery::SansSerif => ResolvedFamily::SansSerif,
            FontQuery::Named(name) => ResolvedFamily::Named(name.to_owned()),
        };
        if self.family != resolved {
            self.cache.clear();
            self.family = resolved;
        }
        if text.is_empty() {
            return (GlyphRun::default(), Vec::new());
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
            return (GlyphRun::default(), Vec::new());
        };
        // Copied out before the shaping read below mutates the buffer
        // (the run borrows it).
        let (line_i, line_w) = (run.line_i, run.line_w);
        let mut glyphs = Vec::with_capacity(run.glyphs.len());
        // Clusters accumulate over the glyphs their range covers; the
        // advance is the glyph hitbox width (`w`), the width cosmic's
        // own hit-testing uses — for a budget cut, exact to well under
        // a px where it differs from the pen advance at all.
        let mut clusters: Vec<Cluster> = Vec::new();
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
            match clusters.last_mut() {
                Some(cluster) if cluster.end_byte == glyph.end => {
                    cluster.advance_px += glyph.w;
                }
                _ => clusters.push(Cluster {
                    end_byte: glyph.end,
                    advance_px: glyph.w,
                }),
            }
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
                line.shape(&mut self.fonts, 8).layout(
                    TEXT_PX,
                    None,
                    Wrap::None,
                    None,
                    None,
                    Hinting::Disabled,
                )
            })
            .and_then(|lines| {
                lines
                    .first()
                    .map(|layout| (layout.max_ascent, layout.max_descent))
            })
            .unwrap_or((0.0, 0.0));
        (
            GlyphRun {
                glyphs,
                width_px: line_w,
                ascent_px,
                descent_px,
            },
            clusters,
        )
    }

    /// Shapes `title` under a pixel budget: a title wider than
    /// `budget_px` keeps the longest cluster prefix that fits beside a
    /// real `…` (the font's own ellipsis glyph, re-shaped in place),
    /// and the answer carries both the display string the card stores
    /// and the run its width came from. A title inside the budget
    /// passes through untouched. The budget is the title band's own
    /// width — `MAX_COLS × TEXT_W` — never the full card (the hint
    /// band's territory is not the title's to spend).
    pub fn shape_budgeted(
        &mut self,
        title: &str,
        query: FontQuery<'_>,
        scale: u32,
        budget_px: f32,
    ) -> (String, GlyphRun) {
        let (run, clusters) = self.shape_with_clusters(title, query, scale);
        if run.width_px <= budget_px || clusters.is_empty() {
            return (title.to_owned(), run);
        }
        let ellipsis_px = self.shape_with_clusters("…", query, scale).0.width_px;
        let advances: Vec<f32> = clusters.iter().map(|c| c.advance_px).collect();
        let kept = truncate_to_budget(&advances, ellipsis_px, budget_px);
        // The cut at zero clusters is the degenerate budget (an
        // ellipsis that cannot fit): show the ellipsis alone rather
        // than nothing — a title row never paints empty.
        let cut = if kept == 0 {
            0
        } else {
            clusters[kept - 1].end_byte
        };
        let display = format!("{}…", &title[..cut]);
        let run = self.shape_with_clusters(&display, query, scale).0;
        (display, run)
    }

    /// Assembles a system around `fonts` with a cold cache.
    fn with_fonts(fonts: FontSystem) -> Self {
        TextSystem {
            fonts,
            swash: SwashCache::new(),
            cache: GlyphCache::default(),
            family: ResolvedFamily::SansSerif,
        }
    }
}

/// Converts one swash image into a [`GlyphMask`]: mask content is the
/// coverage bytes as rasterized; color bitmaps split into per-pixel
/// straight RGB plus alpha-as-coverage (the byte order — R, G, B, A —
/// is cosmic-text's own `with_pixels` read of the same image);
/// subpixel-mask content (never requested from this design, grayscale
/// AA only) degrades to its alpha byte so a face that answers with it
/// still paints, never tofu. `None` for a zero-sized image.
fn mask_from_image(image: &SwashImage) -> Option<GlyphMask> {
    let placement = image.placement;
    if placement.width == 0 || placement.height == 0 {
        return None;
    }
    let expected = (placement.width as usize) * (placement.height as usize);
    let (coverage, colors) = match image.content {
        SwashContent::Mask => {
            if image.data.len() < expected {
                return None;
            }
            (image.data[..expected].to_vec(), None)
        }
        SwashContent::SubpixelMask | SwashContent::Color => {
            if image.data.len() < expected * 4 {
                return None;
            }
            let coverage = image.data[..expected * 4]
                .as_chunks::<4>()
                .0
                .iter()
                .map(|px| px[3])
                .collect();
            let colors = (image.content == SwashContent::Color).then(|| {
                image.data[..expected * 4]
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|px| [px[0], px[1], px[2]])
                    .collect()
            });
            (coverage, colors)
        }
    };
    Some(GlyphMask {
        left: placement.left,
        // Swash measures `top` upward from the baseline; masks speak
        // the same axis as their pixels (down positive).
        top: -placement.top,
        width: placement.width,
        height: placement.height,
        coverage,
        colors,
    })
}

/// One cluster's shaping facts, the unit a budget cut keeps whole.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Cluster {
    /// End byte of the cluster in the shaped text (a UTF-8 boundary —
    /// shaping's own cluster indices).
    end_byte: usize,
    /// Advance the cluster consumed, logical px.
    advance_px: f32,
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
            colors: None,
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

    // --- TY2: truncation, centering, budgeted shaping -------------------

    /// The title band's budget, straight from the constants the Bitmap
    /// path sizes by — the System path spends the same territory.
    fn band_px() -> f32 {
        (crate::ui::MAX_COLS * crate::ui::TEXT_W) as f32
    }

    #[test]
    fn truncation_keeps_the_longest_prefix_that_fits_with_its_ellipsis() {
        let widths = [10.0, 10.0, 10.0, 10.0, 10.0];
        // 20 + 3 fits in 24; 30 + 3 does not — and one more unit past
        // the cut must overflow (maximality).
        let kept = truncate_to_budget(&widths, 3.0, 24.0);
        assert_eq!(kept, 2);
        let sum: f32 = widths[..kept].iter().sum();
        assert!(sum + 3.0 <= 24.0, "the kept prefix + ellipsis fits");
        if kept < widths.len() {
            assert!(
                sum + widths[kept] + 3.0 > 24.0,
                "one more unit would overflow"
            );
        }
    }

    #[test]
    fn truncation_passes_the_whole_input_through_when_it_fits() {
        let widths = [10.0, 10.0, 10.0];
        assert_eq!(truncate_to_budget(&widths, 3.0, 100.0), 3);
        // Exact fit counts as fitting — the budget is inclusive.
        assert_eq!(truncate_to_budget(&widths, 3.0, 33.0), 3);
        // Nothing at all is the empty-input answer.
        assert_eq!(truncate_to_budget(&[], 3.0, 24.0), 0);
    }

    #[test]
    fn truncation_degenerates_to_nothing_when_the_ellipsis_alone_cannot_fit() {
        assert_eq!(truncate_to_budget(&[10.0, 10.0], 5.0, 3.0), 0);
    }

    #[test]
    fn truncation_never_exceeds_the_budget_over_wide_inputs() {
        // A deterministic sweep of synthetic width patterns (no rand
        // dependency): uniform, growing, alternating wide/narrow, and
        // sub-pixel noise. The invariant is the spec's own: the kept
        // prefix + ellipsis never exceeds the budget, and the cut is
        // maximal.
        let patterns: Vec<Vec<f32>> = vec![
            vec![7.5; 80],
            (0..80).map(|i| 1.0 + i as f32 * 0.25).collect(),
            (0..80)
                .map(|i| if i % 2 == 0 { 12.0 } else { 1.5 })
                .collect(),
            (0..80).map(|i| 3.0 + (i % 7) as f32 * 0.3).collect(),
        ];
        for widths in patterns {
            for budget in [1.0f32, 10.0, 57.6, 100.0, 576.0] {
                let ellipsis = 4.2;
                let kept = truncate_to_budget(&widths, ellipsis, budget);
                let sum: f32 = widths[..kept].iter().sum();
                // `kept == 0` is the degenerate budget where even the
                // ellipsis cannot fit — the only state where the sum
                // invariant has nothing to hold.
                assert!(
                    kept == 0 || sum + ellipsis <= budget,
                    "kept prefix + ellipsis inside the budget"
                );
                if kept > 0 && kept < widths.len() {
                    assert!(sum + widths[kept] + ellipsis > budget, "the cut is maximal");
                }
                // Maximality at zero: a nonzero first unit would not
                // have fit either.
                if kept == 0 && !widths.is_empty() {
                    assert!(
                        widths[0] + ellipsis > budget,
                        "zero is honest — one unit truly cannot fit"
                    );
                }
            }
        }
    }

    #[test]
    fn centering_lands_whole_px_inside_every_rhythm_row() {
        let mut system = TextSystem::from_dir(&fixture_dir());
        let run = system.shape("Airlock", FontQuery::SansSerif, 2);
        assert!(run.ascent_px > 0.0 && run.descent_px > 0.0);
        // The geometry ladders' three row heights: 40/48/56.
        for row_h in [40u32, 48, 56] {
            let baseline = centering(run.ascent_px, run.descent_px, row_h);
            assert!(
                baseline > 0 && baseline < row_h,
                "the baseline sits inside a {row_h}-px row"
            );
        }
    }

    #[test]
    fn centering_mirrors_ink_within_the_row() {
        // Equal ink, swapped halves: the two baselines sit symmetrically
        // about the row's center (28 + 20 = 48).
        assert_eq!(centering(10.0, 2.0, 48), 28);
        assert_eq!(centering(2.0, 10.0, 48), 20);
        assert_eq!(centering(10.0, 2.0, 48) + centering(2.0, 10.0, 48), 48);
        // Degenerate metrics stay inside the row: ink dwarfing the row
        // centers to a mid-row baseline ((48 − 180)/2 + 90 = 24), and
        // zero ink parks the baseline at the center.
        assert_eq!(centering(90.0, 90.0, 48), 24);
        assert_eq!(centering(0.0, 0.0, 48), 24);
    }

    #[test]
    fn a_64_char_title_budgets_to_inside_the_band_with_a_real_ellipsis() {
        let mut system = TextSystem::from_dir(&fixture_dir());
        let title = "W".repeat(64);
        // Precondition: the full title genuinely overflows the band at
        // the pinned 12 px (W is one of Open Sans's widest advances).
        let full = system.shape(&title, FontQuery::SansSerif, 2);
        assert!(
            full.width_px > band_px(),
            "the fixture's 64 Ws overflow the band"
        );
        let (display, run) = system.shape_budgeted(&title, FontQuery::SansSerif, 2, band_px());
        assert!(
            display.ends_with('…'),
            "the truncation marker is the font's own ellipsis character"
        );
        assert!(display.chars().count() < 64, "something was cut");
        // Sub-pixel float noise tolerance: the re-shaped kept prefix
        // may differ from the accumulated cut by a hair.
        assert!(
            run.width_px <= band_px() + 1.0,
            "the budgeted run stays inside the band"
        );
    }

    #[test]
    fn budget_passes_short_titles_through_untouched() {
        let mut system = TextSystem::from_dir(&fixture_dir());
        let (display, run) = system.shape_budgeted("Airlock", FontQuery::SansSerif, 2, band_px());
        assert_eq!(display, "Airlock");
        assert!(run.width_px <= band_px());
    }

    #[test]
    fn mask_rasterizes_real_coverage_from_the_fixture_face() {
        let mut system = TextSystem::from_dir(&fixture_dir());
        let run = system.shape("A", FontQuery::SansSerif, 2);
        let key = run.glyphs[0].cache_key;
        let mask = system
            .mask(&key)
            .expect("the fixture face rasterizes")
            .clone();
        assert!(mask.width > 0 && mask.height > 0, "the glyph has extent");
        assert!(
            mask.coverage.iter().any(|&c| c > 0),
            "the glyph carries ink"
        );
        assert!(mask.colors.is_none(), "an outline glyph is coverage only");
        // Second ask recalls the identical mask from the cache.
        assert_eq!(system.mask(&key), Some(&mask));
    }
}
