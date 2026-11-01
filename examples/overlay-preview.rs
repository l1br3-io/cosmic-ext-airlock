//! Renders the switcher overlay to a PNG without a compositor — the
//! design-review loop for `overlay-visuals`. Each scenario is drawn twice,
//! over a dark and a light backdrop, so the card, border, and corners can
//! be judged on both wallpaper polarities exactly as the design skill's
//! review step demands.
//!
//! ```sh
//! cargo run --example overlay-preview -- preview.png [--scale 2]
//! ```
//!
//! Icons come from the real pipeline (`src/icons.rs`) against the host's
//! themes when the sample app ids resolve; unresolved ids draw synthetic
//! tiles so the layout still reads. Dev-only: examples never enter the
//! shipped binary.

use std::rc::Rc;

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

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next().map(std::path::PathBuf::from) else {
        eprintln!("usage: overlay-preview <out.png> [--scale N]");
        std::process::exit(2);
    };
    let scale = match args.next().as_deref() {
        None => 2,
        Some("--scale") => args
            .next()
            .and_then(|n| n.parse().ok())
            .filter(|n| (1..=4).contains(n))
            .unwrap_or(2),
        Some(other) => {
            eprintln!("unknown argument {other}; expected --scale N");
            std::process::exit(2);
        }
    };

    let icons = ui_icons(scale);
    let scenarios = scenarios(&icons);

    // Sheet: every scenario twice (dark backdrop, light backdrop), stacked
    // vertically with gutters.
    let gutter = 32 * scale;
    let backdrop_w = scenarios
        .iter()
        .map(|model| model.width * scale)
        .max()
        .unwrap_or(0)
        + 2 * gutter;
    let backdrop_h: u32 = scenarios
        .iter()
        .map(|model| model.height * scale + gutter)
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
    for model in &scenarios {
        let mut card = vec![0u8; (model.width * model.height * 4 * scale * scale) as usize];
        let (cw, ch) = (model.width * scale, model.height * scale);
        ui::render(model, &mut card, cw, ch, cw * 4, scale);
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

    write_png(&path, width, height, &sheet).expect("write preview png");
    println!("wrote {} ({width}x{height}, scale {scale})", path.display());
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
fn ui_icons(scale: u32) -> Vec<Option<Rc<IconBitmap>>> {
    let mut cache = cosmic_ext_airlock::icons::Cache::new();
    cache.prepare(APP_IDS.iter().copied(), scale);
    APP_IDS
        .iter()
        .enumerate()
        .map(|(index, id)| {
            cache
                .bitmap(id, scale)
                .or_else(|| cache.bitmap("application-x-executable", scale))
                .or_else(|| cache.bitmap("firefox", scale))
                .map_or_else(|| Some(synthetic_icon(index, ui::ICON * scale)), Some)
        })
        .collect()
}

fn row(title: &str, icon: Option<Rc<IconBitmap>>, selected: bool) -> RowModel {
    RowModel {
        title: title.to_string(),
        icon,
        selected,
    }
}

fn scenarios(icons: &[Option<Rc<IconBitmap>>]) -> Vec<OverlayModel> {
    // A long title that elides at MAX_COLS.
    let elided = format!("{}…", "W".repeat(ui::MAX_COLS as usize - 1));
    vec![
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
                // Non-ASCII titles degrade to '?' — the documented state.
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
        // A long list: what a busy workspace looks like at full height.
        OverlayModel {
            rows: (0..12)
                .map(|index| {
                    row(
                        &format!("Window {index} - the long-list case"),
                        icons[index % icons.len()].clone(),
                        index == 7,
                    )
                })
                .collect(),
            ..single()
        },
    ]
    .into_iter()
    .map(|mut model| {
        // Width/height from the widest title, mirroring overlay_model.
        let cols = model
            .rows
            .iter()
            .map(|row| row.title.chars().count() as u32)
            .max()
            .unwrap_or(1);
        let count = model.rows.len() as u32;
        model.width = 2 * ui::PAD_X + ui::ICON + ui::ICON_GAP + cols * ui::TEXT_W;
        model.height = 2 * ui::PAD_Y + count * ui::ROW_H + (count - 1) * ui::ROW_GAP;
        model
    })
    .collect()
}

fn single() -> OverlayModel {
    OverlayModel::default()
}

/// Straight-alpha composite of one rendered card (its own pixels are
/// coverage-premultiplied where the corners fade) onto a solid backdrop —
/// the same arithmetic the compositor performs.
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
            let dst_index = (sy as usize * sheet_width as usize + sx as usize) * 4;
            let dst = &mut sheet[dst_index..dst_index + 4];
            // Card pixels are coverage-premultiplied: src over backdrop.
            for channel in 0..3 {
                dst[channel] =
                    (src[channel] as u32 + backdrop[channel] as u32 * (255 - a) / 255) as u8;
            }
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
