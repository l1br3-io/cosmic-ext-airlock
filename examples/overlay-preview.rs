//! Renders the switcher overlay to a PNG without a compositor — the
//! design-review loop for `overlay-visuals`. Each scenario is drawn twice,
//! over a dark and a light backdrop, so the card, border, and corners can
//! be judged on both wallpaper polarities exactly as the design skill's
//! review step demands. `--light` judges the light palette's card. Every
//! row carries the structural `Ctrl+N` hint band (number-select rev 2);
//! long lists keep `rows` windows in view with `…` continuation rows at the
//! truncated edges (list-viewport), built here exactly as `overlay_model`
//! builds them. The closing scenarios judge the accent-following
//! selection band (palette) over both polarities and a custom card — the
//! plain scenarios are the `selection = "neutral"` side of the comparison.
//!
//! `--geometry` renders the geometry sheet instead (the `geometry`
//! module's judged artifact): a 3 × 3 grid of style (Round,
//! SlightlyRound, Square, left to right) × density (Compact, Standard,
//! Spacious, top to bottom); each cell stacks the dark card's and
//! the light card's long list (a windowed 15-row gesture, dots at both
//! edges) over their short list (two rows, one selected — the band and
//! the corners at their tightest), every card over both backdrops.
//! Square's 2-px rung, Compact's 4-px half-grid, and the band's 8-px
//! cap are judged on this image at native resolution.
//!
//! `--system-font <dir>` renders text through the System backend
//! (`typography` TY3) instead of the font8x8 floor: a fresh database
//! holding only `<dir>`'s faces, so the sheet is deterministic —
//! `res/test-fonts` is the committed fixture (Open Sans, the COSMIC
//! default family itself). Titles keep this sheet's own composition;
//! the flag judges the face and its coverage, live-applying the real
//! daemon's shaping/rasterization path.
//!
//! ```sh
//! `--horizontal` renders the horizontal band family instead
//! (the `card-layout` module's CL4 sheet input): the band at CL4's
//! acceptance counts (one, three, the 1920-auto eight, the pinned
//! twelve), a swept mid-band selection, the trailing `…` tile, and
//! the 48-column title every tile clamps — tokens (pitch, icon, card
//! height, badge placement) frozen on this sheet 2026-11-08.
//!
//! cargo run --example overlay-preview -- preview.png [--scale 2] [--light] [--geometry] [--horizontal] [--system-font res/test-fonts]
//! ```
//!
//! Icons come from the real pipeline (`src/icons.rs`) against the host's
//! themes when the sample app ids resolve; unresolved ids draw synthetic
//! tiles so the layout still reads. Dev-only: examples never enter the
//! shipped binary.

use std::rc::Rc;

use cosmic_ext_airlock::switcher::Handle;
use cosmic_ext_airlock::text::{FontQuery, TextSystem};
use cosmic_ext_airlock::ui::{self, IconBitmap, OverlayModel, RowModel};

/// Sample app ids spanning themed SVG icons, Flatpak ids, and misses.
const APP_IDS: &[&str] = &[
    "firefox",
    "org.gnome.Files",
    "org.gnome.TextEditor",
    "com.google.Chrome",
    "kitty",
    "code",
    "org.gnome.Calculator",
    "utilities-terminal",
    "org.gnome.Software",
    "steam",
    "spotify",
    "no.such.app.Exists",
];

/// One sheet entry: the card and the palette it paints with. The band
/// scenarios carry their own palettes, so the sheet judges cards of
/// both polarities side by side whatever `--light` chose for the plain
/// family.
struct Scenario {
    model: OverlayModel,
    palette: ui::Palette,
}

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next().map(std::path::PathBuf::from) else {
        eprintln!(
            "usage: overlay-preview <out.png> [--scale N] [--light] [--geometry] [--system-font <dir>]"
        );
        std::process::exit(2);
    };
    let mut scale = 2;
    let mut light = false;
    let mut geometry = false;
    let mut horizontal = false;
    let mut system_font: Option<std::path::PathBuf> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--scale" => {
                scale = args
                    .next()
                    .and_then(|n| n.parse().ok())
                    .filter(|n| (1..=4).contains(n))
                    .unwrap_or(scale);
            }
            "--light" => light = true,
            "--geometry" => geometry = true,
            "--horizontal" => horizontal = true,
            "--system-font" => {
                system_font = args.next().map(std::path::PathBuf::from);
                if system_font.is_none() {
                    eprintln!("--system-font needs a font directory argument");
                    std::process::exit(2);
                }
            }
            other => {
                eprintln!(
                    "unknown argument {other}; expected --scale N, --light, --geometry, --horizontal, or --system-font <dir>"
                );
                std::process::exit(2);
            }
        }
    }
    // The System text backend when asked for: a fresh database holding
    // only the flag's directory, never the host's fonts — deterministic
    // sheets (the `typography` fixture rule).
    let mut text_system = system_font.as_ref().map(|dir| TextSystem::from_dir(dir));

    // The card the plain scenarios judge: the dark card the daemon
    // paints today, or the light palette's card under review. Icons
    // tint per polarity and the band scenarios show both, so both tint
    // sets are prepared.
    let base = if light {
        ui::Palette::light()
    } else {
        ui::Palette::dark()
    };
    let dark_icons = ui_icons(scale, &ui::Palette::dark());
    let light_icons = ui_icons(scale, &ui::Palette::light());
    if horizontal {
        let icons = if base.is_dark {
            &dark_icons
        } else {
            &light_icons
        };
        let band_dark = band_icons(scale, &ui::Palette::dark());
        let band_light = band_icons(scale, &ui::Palette::light());
        let mut sheet = horizontal_scenarios(base, icons);
        // Both polarities judge the band: the same tiles in the other
        // palette's tint, the band scenarios' arrangement.
        let other = if base.is_dark {
            ui::Palette::light()
        } else {
            ui::Palette::dark()
        };
        let other_icons = if other.is_dark {
            &band_dark
        } else {
            &band_light
        };
        sheet.extend(horizontal_scenarios(other, other_icons));
        write_sheet(&path, &sheet, scale, &mut text_system);
        return;
    }
    if geometry {
        let cells = geometry_cells(&dark_icons, &light_icons);
        write_grid(&path, &cells, scale, &mut text_system);
        return;
    }
    let scenarios = scenarios(base, &dark_icons, &light_icons);
    write_sheet(&path, &scenarios, scale, &mut text_system);
}

/// The plain sheet writer, shared by every family: every scenario twice
/// (dark backdrop, light backdrop), stacked vertically with gutters.
fn write_sheet(
    path: &std::path::Path,
    scenarios: &[Scenario],
    scale: u32,
    text_system: &mut Option<TextSystem>,
) {
    let gutter = 32 * scale;
    let backdrop_w = scenarios
        .iter()
        .map(|scenario| scenario.model.width * scale)
        .max()
        .unwrap_or(0)
        + 2 * gutter;
    let backdrop_h: u32 = scenarios
        .iter()
        .map(|scenario| scenario.model.height * scale + gutter)
        .sum();
    let (width, height) = (backdrop_w * 2 + gutter, backdrop_h + gutter);

    // Paint the sheet first: an unpainted seam between the backdrops
    // would render as transparent black in viewers and read as a defect
    // in the cards it touches.
    let mut sheet = vec![0u8; (width * height * 4) as usize];
    for pixel in sheet.as_chunks_mut::<4>().0 {
        pixel.copy_from_slice(&[26, 26, 28, 255]);
    }
    let mut y = gutter;
    for scenario in scenarios {
        let model = &scenario.model;
        let mut card = vec![0u8; (model.width * model.height * 4 * scale * scale) as usize];
        let (cw, ch) = (model.width * scale, model.height * scale);
        ui::render(
            model,
            &mut card,
            cw,
            ch,
            cw * 4,
            scale,
            &scenario.palette,
            text_backend(text_system),
        );
        for (dx, backdrop) in [(0, [46u8, 46, 50]), (backdrop_w as i64, [235, 235, 235])] {
            composite(
                &mut sheet,
                width,
                &card,
                (cw, ch),
                (gutter as i64 + dx, y as i64),
                backdrop,
            );
        }
        y += model.height * scale + gutter;
    }

    write_png(path, width, height, &sheet).expect("write preview png");
    println!("wrote {} ({width}x{height}, scale {scale})", path.display());
}

/// The backend every card render paints through: the System text stack
/// over the flag's fixture fonts when `--system-font` was given, the
/// font8x8 floor otherwise.
fn text_backend(system: &mut Option<TextSystem>) -> ui::TextBackend<'_> {
    match system {
        Some(ts) => ui::TextBackend::System {
            system: ts,
            query: FontQuery::SansSerif,
        },
        None => ui::TextBackend::Bitmap,
    }
}

/// The geometry sheet's nine cells: density rows (Compact, Standard,
/// Spacious top to bottom) × style columns (Round, SlightlyRound,
/// Square left to right). Each cell stacks four cards — each palette's
/// long list (a windowed 15-row gesture) over its short list — so
/// radius, rhythm, the band's cap, and the density's height story all
/// judge per combination, both palettes.
fn geometry_cells(
    dark: &[Option<Rc<IconBitmap>>],
    light: &[Option<Rc<IconBitmap>>],
) -> Vec<Vec<Scenario>> {
    let styles = [
        cosmic_ext_airlock::theme::Style::Round,
        cosmic_ext_airlock::theme::Style::SlightlyRound,
        cosmic_ext_airlock::theme::Style::Square,
    ];
    let densities = [
        cosmic_ext_airlock::theme::Density::Compact,
        cosmic_ext_airlock::theme::Density::Standard,
        cosmic_ext_airlock::theme::Density::Spacious,
    ];
    let mut cells = Vec::new();
    for density in densities {
        let mut cell = Vec::new();
        for style in styles {
            let profile = ui::StyleProfile::resolve(style, density);
            // Long: a windowed interior selection — dots at both edges,
            // the full rhythm repeating, the density's height at the
            // default budget. Short: two rows with the first selected —
            // the band's own rounding and the card corner at their
            // tightest interaction (Compact + Round).
            for (palette, icons) in [(ui::Palette::dark(), dark), (ui::Palette::light(), light)] {
                cell.push(Scenario {
                    model: windowed(15, 7, ui::DEFAULT_ROWS, None, icons, profile),
                    palette,
                });
                cell.push(Scenario {
                    model: sized_profile(
                        OverlayModel {
                            rows: vec![
                                row("Selected on top", icons[1].clone(), true),
                                row("Unselected", icons[2].clone(), false),
                            ],
                            ..OverlayModel::default()
                        },
                        profile,
                    ),
                    palette,
                });
            }
        }
        cells.push(cell);
    }
    cells
}

/// The geometry sheet's grid composition: one backdrop pair per cell,
/// cells in a 3 × 3 grid with gutters. Cards paint centered on their
/// backdrop columns so the corners judge away from the seams.
fn write_grid(
    path: &std::path::Path,
    cells: &[Vec<Scenario>],
    scale: u32,
    text_system: &mut Option<TextSystem>,
) {
    let gutter = 32 * scale;
    let backdrop_w = cells
        .iter()
        .flatten()
        .map(|scenario| scenario.model.width * scale)
        .max()
        .unwrap_or(0)
        + 2 * gutter;
    // Each density row holds three style cells of four cards; the row's
    // height is one cell's stack (every style's is the same — height is
    // the density's, width the titles').
    let row_heights: Vec<u32> = cells
        .iter()
        .map(|row| {
            row.chunks(4)
                .next()
                .map(|cell| {
                    cell.iter()
                        .map(|scenario| scenario.model.height * scale + gutter)
                        .sum::<u32>()
                        + gutter
                })
                .unwrap_or(gutter)
        })
        .collect();
    let width = backdrop_w * 2 * 3 + gutter * 2;
    let height: u32 = row_heights.iter().sum::<u32>() + gutter;
    let mut sheet = vec![0u8; (width * height * 4) as usize];
    for pixel in sheet.as_chunks_mut::<4>().0 {
        pixel.copy_from_slice(&[26, 26, 28, 255]);
    }
    let mut y = gutter;
    for (row, row_h) in cells.iter().zip(&row_heights) {
        for (column, cell) in row.chunks(4).enumerate() {
            let mut card_y = y;
            for scenario in cell {
                let model = &scenario.model;
                let mut card = vec![0u8; (model.width * model.height * 4 * scale * scale) as usize];
                let (cw, ch) = (model.width * scale, model.height * scale);
                ui::render(
                    model,
                    &mut card,
                    cw,
                    ch,
                    cw * 4,
                    scale,
                    &scenario.palette,
                    text_backend(text_system),
                );
                let cell_x = column as u64 * (backdrop_w as u64 * 2);
                for (dx, backdrop) in [(0, [46u8, 46, 50]), (backdrop_w as i64, [235, 235, 235])] {
                    let x = gutter as i64
                        + cell_x as i64
                        + dx
                        + ((backdrop_w - 2 * gutter - cw) / 2) as i64; // centered on its backdrop
                    composite(
                        &mut sheet,
                        width,
                        &card,
                        (cw, ch),
                        (x, card_y as i64),
                        backdrop,
                    );
                }
                card_y += model.height * scale + gutter;
            }
        }
        y += row_h;
    }
    write_png(path, width, height, &sheet).expect("write preview png");
    println!(
        "wrote {} ({width}x{height}, scale {scale}, 3x3 geometry sheet)",
        path.display()
    );
}

/// One synthetic icon tile: a filled rounded-ish square in a per-app
/// color, so unresolved ids still show an icon-shaped object.
fn synthetic_icon(index: usize, side: u32) -> Rc<IconBitmap> {
    let palette = [
        (0xE6, 0x6A, 0x33),
        (0x35, 0x84, 0xE4),
        (0x2E, 0xC2, 0x7E),
        (0xC0, 0x1C, 0xBE),
        (0xF6, 0xD3, 0x2D),
        (0x26, 0xA2, 0x69),
    ];
    let (r, g, b) = palette[index % palette.len()];
    let mut argb = Vec::with_capacity((side * side) as usize);
    let edge = side / 8; // a clear margin keeps the tile from touching the cell
    for py in 0..side {
        for px in 0..side {
            let inset = edge..side - edge;
            argb.push(if inset.contains(&px) && inset.contains(&py) {
                0xFF00_0000 | ((r as u32) << 16) | ((g as u32) << 8) | b as u32
            } else {
                0
            });
        }
    }
    Rc::new(IconBitmap { side, argb })
}

/// Real themed icons when the host can resolve them; the app ids that
/// miss fall to synthetic tiles. On the dev host this shows the actual
/// COSMIC/Pop icons the daemon will show.
fn ui_icons(scale: u32, palette: &ui::Palette) -> Vec<Option<Rc<IconBitmap>>> {
    let mut cache = cosmic_ext_airlock::icons::Cache::new();
    cache.prepare(APP_IDS.iter().copied(), ui::ICON, scale, palette);
    APP_IDS
        .iter()
        .enumerate()
        .map(|(index, id)| {
            cache
                .bitmap(id, ui::ICON, scale, palette)
                .or_else(|| cache.bitmap("application-x-executable", ui::ICON, scale, palette))
                .or_else(|| cache.bitmap("firefox", ui::ICON, scale, palette))
                .map_or_else(|| Some(synthetic_icon(index, ui::ICON * scale)), Some)
        })
        .collect()
}

/// [`ui_icons`], at the tile's own side — the band's 48-px icons, the
/// row family's 32-px rule one axis over.
fn band_icons(scale: u32, palette: &ui::Palette) -> Vec<Option<Rc<IconBitmap>>> {
    let mut cache = cosmic_ext_airlock::icons::Cache::new();
    cache.prepare(APP_IDS.iter().copied(), ui::TILE_ICON, scale, palette);
    APP_IDS
        .iter()
        .enumerate()
        .map(|(index, id)| {
            cache
                .bitmap(id, ui::TILE_ICON, scale, palette)
                .or_else(|| cache.bitmap("application-x-executable", ui::TILE_ICON, scale, palette))
                .or_else(|| cache.bitmap("firefox", ui::TILE_ICON, scale, palette))
                .map_or_else(|| Some(synthetic_icon(index, ui::TILE_ICON * scale)), Some)
        })
        .collect()
}

/// The horizontal band family (card-layout): the flat band as
/// `overlay_model_horizontal` composes it — `band_window` over the
/// selection, the trailing `…` tile, the tile grid's width — at CL4's
/// acceptance counts (one, three, the 1920-auto eight, the pinned
/// twelve), a swept mid-band selection, and the 48-column title every
/// tile clamps to its own budget. The tokens these render froze on
/// this sheet, 2026-11-08, every value kept as built.
fn horizontal_scenarios(base: ui::Palette, icons: &[Option<Rc<IconBitmap>>]) -> Vec<Scenario> {
    let titles: Vec<String> = (0..16)
        .map(|index| format!("Window {index} — the band case"))
        .collect();
    let band_row =
        |title: &str, icon: Option<Rc<IconBitmap>>, selected: bool, position: usize| -> RowModel {
            RowModel {
                // The tile's own clamp: the band's width is the tile grid,
                // never the title's — the daemon composes the same budget.
                title: ui::clamp_tile_title(title),
                icon,
                selected,
                handle: Some(Handle::default()),
                hint: (position < 9).then_some(ui::RowHint::Ctrl(position + 1)),
                ..RowModel::default()
            }
        };
    let band = |list: &[String], sel: usize, tiles: usize| -> OverlayModel {
        let count = list.len();
        let viewport = ui::band_window(count, sel, tiles, None);
        let mut rows = Vec::with_capacity(viewport.content + viewport.bottom as usize);
        for position in 0..viewport.content {
            let index = viewport.start + position;
            rows.push(band_row(
                &list[index],
                icons[index % icons.len()].clone(),
                index == sel,
                position,
            ));
        }
        if viewport.bottom {
            rows.push(RowModel {
                title: String::new(),
                icon: None,
                selected: false,
                handle: None,
                ..RowModel::default()
            });
        }
        let slots = rows.len() as u32;
        OverlayModel {
            rows,
            width: ui::band_width(slots),
            height: ui::band_card_height(),
            viewport,
            profile: ui::StyleProfile::default(),
            horizontal: true,
        }
    };
    // The 48-column clamp input: the tile's own budget (eleven columns
    // at the frozen pitch) ellipsizes every tile that carries it.
    let long = "This title is exactly forty-eight columns long —";
    let long_titles: Vec<String> = std::iter::repeat_n(long.to_string(), 5).collect();
    [
        // A lone window: one tile, selected, its badge the only hint.
        band(&titles[..1], 0, 8),
        // Three whole tiles, the middle selected — the hint badge over
        // the selection fill at its tightest.
        band(&titles[..3], 1, 3),
        // The working band (the 1920 auto budget): hints to nine,
        // dots past it.
        band(&titles, 1, 8),
        // A mid-band selection after a sweep: the follow window.
        band(&titles, 9, 8),
        // The pinned extremes: three tiles and twelve.
        band(&titles, 0, ui::COLUMNS_MIN),
        band(&titles, 0, ui::COLUMNS_MAX),
        // The 48-column title: every tile clamps it, selection mid-band.
        band(&long_titles, 2, 5),
    ]
    .into_iter()
    .map(|model| Scenario {
        model,
        palette: base,
    })
    .collect()
}

fn row(title: &str, icon: Option<Rc<IconBitmap>>, selected: bool) -> RowModel {
    RowModel {
        // The same fold the daemon applies in `build` — the preview
        // shows what the card would actually render.
        title: ui::fold_title(title),
        icon,
        selected,
        // Preview scenarios render no real window, so no handle to name —
        // but a content row must carry one: a handleless row renders as
        // the continuation affordance.
        handle: Some(Handle::default()),
        ..RowModel::default()
    }
}

/// The continuation row at a truncated edge, exactly as `overlay_model`
/// builds it: no handle, no icon, no title — just its dots.
fn dots_row() -> RowModel {
    RowModel {
        title: String::new(),
        icon: None,
        selected: false,
        handle: None,
        ..RowModel::default()
    }
}

/// A windowed long-list card composed the way the daemon composes it:
/// `visible_window` over the selection, dots rows at truncated edges,
/// width from the full list's widest title so the card never breathes
/// while the window moves.
fn windowed(
    count: usize,
    sel: usize,
    rows: usize,
    prev: Option<ui::Viewport>,
    icons: &[Option<Rc<IconBitmap>>],
    profile: ui::StyleProfile,
) -> OverlayModel {
    let titles: Vec<String> = (0..count)
        .map(|index| format!("Window {index} - the long-list case"))
        .collect();
    let viewport = ui::visible_window(count, sel, rows, prev);
    let mut rows =
        Vec::with_capacity(viewport.top as usize + viewport.content + viewport.bottom as usize);
    if viewport.top {
        rows.push(dots_row());
    }
    let first = viewport.start + viewport.top as usize;
    for index in first..first + viewport.content {
        rows.push(row(
            &titles[index],
            icons[index % icons.len()].clone(),
            index == sel,
        ));
    }
    if viewport.bottom {
        rows.push(dots_row());
    }
    let cols = titles
        .iter()
        .map(|title| title.chars().count() as u32)
        .max()
        .unwrap_or(1);
    let shown = rows.len() as u32;
    OverlayModel {
        rows,
        width: 2 * ui::PAD_X
            + ui::ICON
            + ui::ICON_GAP
            + cols * ui::TEXT_W
            + ui::HINT_GAP
            + ui::HINT_COLS * ui::TEXT_W,
        height: ui::card_height_rhythm(profile.rhythm, shown),
        viewport,
        profile,
        horizontal: false,
    }
}

/// [`sized`], under an explicit profile — the geometry sheet sizes its
/// cards at every style × density combination.
fn sized_profile(mut model: OverlayModel, profile: ui::StyleProfile) -> OverlayModel {
    let cols = model
        .rows
        .iter()
        .map(|row| row.title.chars().count() as u32)
        .max()
        .unwrap_or(1);
    let count = model.rows.len() as u32;
    model.width = 2 * ui::PAD_X
        + ui::ICON
        + ui::ICON_GAP
        + cols * ui::TEXT_W
        + ui::HINT_GAP
        + ui::HINT_COLS * ui::TEXT_W;
    model.height = ui::card_height_rhythm(profile.rhythm, count);
    model.profile = profile;
    model
}

fn scenarios(
    base: ui::Palette,
    dark: &[Option<Rc<IconBitmap>>],
    light: &[Option<Rc<IconBitmap>>],
) -> Vec<Scenario> {
    // The plain family judges one polarity — the base card; icons tint
    // to match it.
    let icons = if base.is_dark { dark } else { light };
    // A long title that elides at MAX_COLS.
    let elided = format!("{}…", "W".repeat(ui::MAX_COLS as usize - 1));
    let mut sheet = vec![
        OverlayModel {
            rows: vec![row("Solo window", icons[0].clone(), false)],
            ..single()
        },
        OverlayModel {
            rows: vec![
                row("Selected on top", icons[1].clone(), true),
                row("Unselected", icons[2].clone(), false),
            ],
            ..single()
        },
        OverlayModel {
            rows: vec![
                row("MRU first", icons[3].clone(), false),
                row("Middle - selection", icons[4].clone(), true),
                row("Fallback glyph row", None, false),
                row(&elided, icons[6].clone(), false),
                row("(untitled)", None, false),
                // Non-ASCII letters degrade to '?' — the documented
                // state; the em-dash itself folds to '-'.
                row("Résumé — naïve.txt", icons[7].clone(), false),
            ],
            ..single()
        },
        // The same-app view (Alt+grave): one application, repeated icon,
        // titles carry the disambiguation.
        OverlayModel {
            rows: vec![
                row("main.rs — airlock", icons[5].clone(), true),
                row("overlay-preview.rs — airlock", icons[5].clone(), false),
                row("Cargo.toml — airlock", icons[5].clone(), false),
            ],
            ..single()
        },
        // Rev 2's no-collision judgment: the widest title a card carries
        // beside its right-aligned hint, plus a selected hinted row.
        OverlayModel {
            rows: vec![
                row(&elided, icons[8].clone(), false),
                row("Short title", icons[9].clone(), true),
            ],
            ..single()
        },
    ]
    .into_iter()
    .map(sized)
    .map(|model| Scenario {
        model,
        palette: base,
    })
    .collect::<Vec<_>>();
    // The long-list family (list-viewport), each windowed and sized by
    // `windowed` itself — the map above must not re-derive their width
    // from the window's titles alone. A fresh 15-row gesture's head
    // window (dots below only), the same list stepped to an interior
    // selection (dots at both edges, the fill one row down), its bottom
    // window (dots above only), and a 30-row list mid-sweep — all at the
    // default `rows` budget; then the same 30-row list at the setting's
    // floor and ceiling (overlay-density).
    let rows = ui::DEFAULT_ROWS;
    let head = windowed(15, 0, rows, None, icons, ui::StyleProfile::default());
    let interior = windowed(
        15,
        7,
        rows,
        Some(head.viewport),
        icons,
        ui::StyleProfile::default(),
    );
    let bottom = windowed(
        15,
        14,
        rows,
        Some(interior.viewport),
        icons,
        ui::StyleProfile::default(),
    );
    for model in [
        head,
        interior,
        bottom,
        windowed(30, 15, rows, None, icons, ui::StyleProfile::default()),
        windowed(
            30,
            15,
            ui::ROWS_MIN,
            None,
            icons,
            ui::StyleProfile::default(),
        ),
        windowed(
            30,
            15,
            ui::ROWS_MAX,
            None,
            icons,
            ui::StyleProfile::default(),
        ),
    ] {
        sheet.push(Scenario {
            model,
            palette: base,
        });
    }
    // The grouped family (grouping): a mixed tree with the selection on
    // its parent, the same tree with the selection on a child, and the
    // tree pushed past the `rows` budget so the `…` windowing rides the
    // flattened sequence. Candidate tokens (indent, badge, chevron) —
    // the GR4 sheet freezes them.
    // Collapsed, the default: level-1 entries only, every group's
    // chevron pointing at its closed children.
    for model in [
        tree_card(
            vec![
                parent_row("firefox", 3, icons[0].clone(), true, false, Some(1)),
                parent_row(
                    "org.gnome.TextEditor",
                    2,
                    icons[2].clone(),
                    false,
                    false,
                    Some(2),
                ),
                entry_row("kitty — solo window", icons[4].clone(), false, 3),
                entry_row("un-attributable", None, false, 4),
            ],
            0,
        ),
        // Expanded: the selected group's whole window set below it, front
        // window first, Alt-numbered; the other group stays collapsed.
        tree_card(
            vec![
                parent_row("firefox", 3, icons[0].clone(), false, true, Some(1)),
                child_row("twitter.com/home", icons[0].clone(), true, 1),
                child_row("github.com — PR 42", icons[0].clone(), false, 2),
                parent_row(
                    "org.gnome.TextEditor",
                    2,
                    icons[2].clone(),
                    false,
                    false,
                    Some(2),
                ),
                entry_row("kitty — solo window", icons[4].clone(), false, 3),
                entry_row("un-attributable", None, false, 4),
            ],
            1,
        ),
        // Open cycling (`scoped_cycling = false`, the default): the same
        // expanded tree, but the expanded entry is a divider — unhinted,
        // never holding the selection — while the collapsed entry keeps
        // its Ctrl chord (it is how Tab reaches that app). The selection
        // lives among the windows; Tab crosses groups.
        tree_card(
            vec![
                parent_row("firefox", 3, icons[0].clone(), false, true, None),
                child_row("twitter.com/home", icons[0].clone(), false, 1),
                child_row("github.com — PR 42", icons[0].clone(), true, 2),
                parent_row(
                    "org.gnome.TextEditor",
                    2,
                    icons[2].clone(),
                    false,
                    false,
                    Some(2),
                ),
                entry_row("kitty — solo window", icons[4].clone(), false, 3),
                entry_row("un-attributable", None, false, 4),
            ],
            2,
        ),
        tree_card(
            {
                // The budget window over the flattened sequence, hand-rolled
                // exactly as `overlay_model` windows it: a 12-row tree at the
                // default budget keeps 10 slots, the `…` rows replacing the
                // outermost slots at each truncated edge.
                let long = vec![
                    parent_row("firefox", 5, icons[0].clone(), false, true, Some(1)),
                    child_row("window one", icons[0].clone(), false, 1),
                    child_row("window two", icons[0].clone(), false, 2),
                    child_row("window three", icons[0].clone(), false, 3),
                    child_row("window four", icons[0].clone(), false, 4),
                    parent_row("org.gnome.Files", 4, icons[1].clone(), true, true, Some(2)),
                    child_row("home dir", icons[1].clone(), false, 1),
                    child_row("downloads", icons[1].clone(), false, 2),
                    child_row("media", icons[1].clone(), false, 3),
                    entry_row("kitty — solo window", icons[4].clone(), false, 3),
                    entry_row("code — airlock", icons[5].clone(), false, 4),
                    entry_row("un-attributable", None, false, 5),
                ];
                let vp = ui::visible_window(long.len(), 5, ui::DEFAULT_ROWS, None);
                let mut windowed_rows = Vec::new();
                if vp.top {
                    windowed_rows.push(dots_row());
                }
                windowed_rows.extend(
                    long.into_iter()
                        .skip(vp.start + vp.top as usize)
                        .take(vp.content),
                );
                if vp.bottom {
                    windowed_rows.push(dots_row());
                }
                windowed_rows
            },
            5,
        ),
    ] {
        sheet.push(Scenario {
            model,
            palette: base,
        });
    }
    // The accent follow (palette rev 2): the band is the followed
    // accent's `base` at 20 % over the effective card — pinned to the
    // store record's live fixtures so the sheet is reproducible: the
    // dark side's v1 purple and the light side's v2 dark-teal, plus the
    // purple over a custom card whose own luminance picks the dark
    // side. The plain scenarios above are the `selection = "neutral"`
    // side of the comparison, byte-identical to every card shipped
    // before the follow.
    const DARK_SIDE_ACCENT: u32 = 0xE7_9C_FE;
    const LIGHT_SIDE_ACCENT: u32 = 0x00_52_5A;
    let follow = |rows: Vec<RowModel>, card: ui::Palette, accent: u32| Scenario {
        model: sized(OverlayModel { rows, ..single() }),
        palette: card.with_selection(ui::SelectionSetting::Accent, Some(accent)),
    };
    sheet.push(follow(
        vec![
            row("Accent band — dark side", dark[0].clone(), false),
            row("purple, followed from COSMIC", dark[1].clone(), true),
            row("the desktop's own accent", dark[2].clone(), false),
        ],
        ui::Palette::dark(),
        DARK_SIDE_ACCENT,
    ));
    sheet.push(follow(
        vec![
            row("Accent band — light side", light[0].clone(), false),
            row("dark-teal, followed from COSMIC", light[1].clone(), true),
            row("per-polarity accents are real", light[2].clone(), false),
        ],
        ui::Palette::light(),
        LIGHT_SIDE_ACCENT,
    ));
    sheet.push(follow(
        vec![
            row("Custom card, accent band", dark[0].clone(), false),
            row("the band blends over any card", dark[2].clone(), true),
            row("polarity from the card's luminance", dark[4].clone(), false),
        ],
        ui::Palette::custom(0x00_2A_1E_30, 0x00_E8_E6_E0),
        DARK_SIDE_ACCENT,
    ));
    sheet
}

/// Width/height from the widest title, mirroring `overlay_model` — hint
/// band included (number-select rev 2).
fn sized(mut model: OverlayModel) -> OverlayModel {
    let cols = model
        .rows
        .iter()
        .map(|row| row.title.chars().count() as u32)
        .max()
        .unwrap_or(1);
    let count = model.rows.len() as u32;
    model.width = 2 * ui::PAD_X
        + ui::ICON
        + ui::ICON_GAP
        + cols * ui::TEXT_W
        + ui::HINT_GAP
        + ui::HINT_COLS * ui::TEXT_W;
    model.height = 2 * ui::PAD_Y + count * ui::ROW_H + (count - 1) * ui::ROW_GAP;
    model
}

/// A parent row as `overlay_model_grouped` builds one: labeled by its
/// app_id, carrying its app's window count for the badge, its
/// expansion state for the chevron, and its Ctrl+N ordinal — `None`
/// for the one row that hints nothing, open cycling's divider (an
/// expanded entry while `scoped_cycling = false`).
fn parent_row(
    app: &str,
    count: usize,
    icon: Option<Rc<IconBitmap>>,
    selected: bool,
    expanded: bool,
    ctrl: Option<usize>,
) -> RowModel {
    RowModel {
        title: app.to_string(),
        icon,
        selected,
        handle: Some(Handle::default()),
        kind: cosmic_ext_airlock::switcher::RowKind::Parent,
        count,
        expanded,
        hint: ctrl.map(ui::RowHint::Ctrl),
    }
}

/// A child row: a window title nested under its parent, Alt-numbered
/// within its group.
fn child_row(title: &str, icon: Option<Rc<IconBitmap>>, selected: bool, n: usize) -> RowModel {
    RowModel {
        kind: cosmic_ext_airlock::switcher::RowKind::Child,
        hint: Some(ui::RowHint::Alt(n)),
        ..row(title, icon, selected)
    }
}

/// Stamps a level-1 row's Ctrl+N hint (the Nth entry) on a plain row.
fn entry_row(title: &str, icon: Option<Rc<IconBitmap>>, selected: bool, n: usize) -> RowModel {
    RowModel {
        hint: Some(ui::RowHint::Ctrl(n)),
        ..row(title, icon, selected)
    }
}

/// Composes one grouped card, sized the way `overlay_model` sizes it:
/// the title budget carries the child indent and the parent badge
/// columns beside the titles themselves.
fn tree_card(rows: Vec<RowModel>, selected: usize) -> OverlayModel {
    let shown = rows.len() as u32;
    let mut model = OverlayModel {
        rows,
        ..OverlayModel::default()
    };
    model.rows[selected].selected = true;
    let cols = model
        .rows
        .iter()
        .map(|row| {
            row.title.chars().count() as u32
                + (row.kind == cosmic_ext_airlock::switcher::RowKind::Child) as u32
                    * (ui::TREE_INDENT / ui::TEXT_W)
                + (row.kind == cosmic_ext_airlock::switcher::RowKind::Parent) as u32
                    * ui::BADGE_COLS
        })
        .max()
        .unwrap_or(1);
    model.width = 2 * ui::PAD_X
        + ui::ICON
        + ui::ICON_GAP
        + cols * ui::TEXT_W
        + ui::HINT_GAP
        + ui::HINT_COLS * ui::TEXT_W;
    model.height = ui::card_height(shown);
    model
}

fn single() -> OverlayModel {
    OverlayModel::default()
}

/// Straight-alpha composite of one rendered card (its own pixels are
/// coverage-premultiplied where the corners fade) onto a solid backdrop —
/// the same arithmetic the compositor performs. The card canvas is
/// `render`'s ARGB8888 written little-endian, so a pixel's bytes arrive
/// as B, G, R, A; the sheet is RGBA.
fn composite(
    sheet: &mut [u8],
    sheet_width: u32,
    card: &[u8],
    (card_w, card_h): (u32, u32),
    (x0, y0): (i64, i64),
    backdrop: [u8; 3],
) {
    let sheet_height = (sheet.len() / (sheet_width as usize * 4)) as i64;
    for cy in 0..card_h {
        let sy = y0 + cy as i64;
        if sy < 0 || sy >= sheet_height {
            continue;
        }
        for cx in 0..card_w {
            let sx = x0 + cx as i64;
            if sx < 0 || sx >= sheet_width as i64 {
                continue;
            }
            let src = &card[(cy * card_w * 4 + cx * 4) as usize..];
            let a = src[3] as u32;
            let (b, g, r) = (src[0] as u32, src[1] as u32, src[2] as u32);
            let dst_index = (sy as usize * sheet_width as usize + sx as usize) * 4;
            let dst = &mut sheet[dst_index..dst_index + 4];
            // Card pixels are coverage-premultiplied: src over backdrop.
            dst[0] = (r + backdrop[0] as u32 * (255 - a) / 255) as u8;
            dst[1] = (g + backdrop[1] as u32 * (255 - a) / 255) as u8;
            dst[2] = (b + backdrop[2] as u32 * (255 - a) / 255) as u8;
            dst[3] = 255;
        }
    }
}

fn write_png(path: &std::path::Path, width: u32, height: u32, rgba: &[u8]) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|err| err.to_string())?;
    let mut encoder = png::Encoder::new(file, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|err| err.to_string())?;
    writer
        .write_image_data(rgba)
        .map_err(|err| err.to_string())?;
    Ok(())
}
