# Switcher overlay UX and app-icon acquisition — research findings

**Date:** November 1, 2026 · **Author:** research agent for jpt · **Status:** Complete — ready to feed the overlay/icon work items

**Scope:** visual design of app-switcher overlays (sizes, highlights, timings, mouse policy) and app-icon acquisition on COSMIC/freedesktop. Companion record to [switcher-behavior-adaptation.md](switcher-behavior-adaptation.md), which settled the *interaction semantics* (commit keys, arrows, wrap, same-app cycling); this one settles *what it should look like* and *where the pixels come from*.

**Method:** primary sources only — upstream source code read over raw git (gitlab.gnome.org, invent.kde.org, github.com mirror APIs), protocol XML from wlroots/wayland, freedesktop.org specifications, docs.rs for crate semantics, Apple/Microsoft support pages, and the local COSMIC install's packaged defaults. Remote code is cited by repo/file + constant or function name (line numbers were not stable across fetches); Airlock claims carry `file:line` refs into this repository. Fetched 2026-10-31/11-01.

---

## 1. TL;DR — the adaptations worth stealing

| # | Adaptation | Source |
|---|---|---|
| 1 | Selection = full-row **accent-tinted fill** (accent at ~20% alpha), not an outline; the fill is the one thing the eye tracks at Tab speed | GNOME `:selected` (fg @ 20%), alt-tab-macos (accent @ 0.2), cosmic-launcher (hovered Text-button style) |
| 2 | **32 px app icon per row** is the converged size (cosmic-launcher, GNOME `APP_ICON_SIZE_SMALL=48` on thumbnails, alt-tab-macos titles style 24–30) | §2.6, §2.1, §2.4 |
| 3 | Card: **≤ 600 px wide**, corner radius ~12 (COSMIC `radius_s + 4`), 1 px border at ~2–8% white, near-opaque dark background | cosmic-launcher `view_window`, GNOME `%osd_panel` |
| 4 | Row rhythm: **8 px vertical / 24 px horizontal padding**, one line of title, elide right | cosmic-launcher row buttons |
| 5 | **No fade-in; fade out ~100 ms ease-out.** Show-delay 100–150 ms so quick taps never flash | GNOME `POPUP_DELAY_TIMEOUT=150`, `POPUP_FADE_OUT_TIME=100`; alt-tab-macos `windowDisplayDelay=100`, `fadeOutAnimation=false` default |
| 6 | Selection moves **instantly** (KDE `highlightMoveDuration: 0`); never animate the selection itself | KWin thumbnail_grid QML |
| 7 | **Keyboard owns the overlay; pointer is pass-through** — GNOME explicitly kills hover on key press ("override %tile style so mouse doesn't steal focus"), alt-tab-macos ships mouse off by default, macOS documents no mouse at all | §2.1, §2.4, §2.3 |
| 8 | Thumbnails are **off the table** on Wayland for an external client — foreign-toplevel protocols are metadata-only by design; substitute **icon + title + workspace tag** (rofi's default window row) | §2.9, §2.7 |
| 9 | Icon pipeline = pop-launcher's: **`freedesktop-desktop-entry` for app_id → icon-name (case-insensitive `find_app_by_id`), `freedesktop-icons` for name → file path (size 48, hicolor fallback), then rasterize PNG/SVG** | §3.3, §3.4 |
| 10 | Fallback chain when app_id misses: `StartupWMClass` → desktop-file id → basename → last dot-segment → lowercased name → `application-x-executable` glyph | `find_app_by_id` in fde; rofi lowercases app_id |
| 11 | Own branding: ship `res/icons/hicolor/<sizes>/apps/<APPID>.svg` + `.desktop` with `Icon=<APPID>`, installed by a justfile loop — the COSMIC house convention (cosmic-edit, cosmic-ext-tasks) | §4 |
| 12 | Long lists: cap at ~70% screen height; go scrollable/paged beyond ~8 rows (cosmic-launcher `SCROLL_MIN = 8`) | KWin (0.7 height), cosmic-launcher |
| 13 | Keep the near-opaque card. GNOME's OSD panel is `lighten(#222226, 5%)` — Airlock's `0x1E_21_26` is already within a hair of it | GNOME `_default-colors.scss` |
| 14 | Multi-row layouts wrap and **center each row**; the grid prefers square-ish overall shape (greedy column count) | alt-tab-macos `TileGridLayout`, KWin `columnCountRecursion` |

---

## 2. Switcher overlay visual design, platform by platform

Airlock today (src/ui.rs:31-63): 8×8 `font8x8` glyphs, an 8×8 generic window glyph, `PAD_X=20`, `PAD_Y=16`, `ROW_GAP=12`, `SELECT_BLEED=4`, full-row selection fill `0xFA_1F_3A_5F` on card `0xFA_1E_21_26`. Everything below is measured against that baseline.

### 2.1 GNOME Shell (altTab.js + switcherPopup.js + theme SCSS)

In current `main`, window switching lives in [`js/ui/altTab.js`](https://gitlab.gnome.org/GNOME/gnome-shell/-/blob/main/js/ui/altTab.js) (`windowSwitcher.js` was folded into it — verified against the `js/ui` tree listing; there is no `windowSwitcher.js` anymore). Constants, read from source:

- `APP_ICON_SIZE = 96`, `APP_ICON_SIZE_SMALL = 48` (badge overlaid on thumbnails in "both" mode), `WINDOW_PREVIEW_SIZE = 128`, `THUMBNAIL_DEFAULT_SIZE = 256`.
- `baseIconSizes = [96, 64, 48, 32, 22]` — `_setIconSize()` picks the largest of these that fits the primary monitor width. A discrete ladder, not free scaling.
- Per-app mode: one `AppIcon` = vertical box (icon bin + centered label below); multi-window apps get small arrows underneath; ↓ drops into a `ThumbnailSwitcher` row of window clones. Per-window mode honors the `app-icon-mode` setting: `THUMBNAIL_ONLY`, `APP_ICON_ONLY`, or `BOTH`.
- Timings: `THUMBNAIL_POPUP_TIME = 500` ms before the per-app thumbnail row appears, fading over `THUMBNAIL_FADE_TIME = 100` ms (`EASE_OUT_QUAD`); `APP_ICON_HOVER_TIMEOUT = 200` ms debounce before hover re-targets a different item.
- `current-workspace-only` GSettings flag filters the window list per workspace — the same isolation Airlock hard-codes. Initial selection is index 1 forward (the previously-focused app is index 0) — Airlock's MRU ring already matches this.
- Extra keys inside the popup: `q`/`Q` quits the app, `w`/`W`/`F4` closes the window (with thumbnails focused). Mouse wheel steps windows-then-apps (`_scrollHandler`).

From [`js/ui/switcherPopup.js`](https://gitlab.gnome.org/GNOME/gnome-shell/-/blob/main/js/ui/switcherPopup.js):

- `POPUP_DELAY_TIMEOUT = 150` ms with the comment: *"We delay showing the popup so that fast Alt+Tab users aren't disturbed by the popup briefly flashing."* The popup is allocated at `opacity 0`, then flips straight to `opacity 255` after the delay — no fade-in.
- `POPUP_FADE_OUT_TIME = 100` ms, `EASE_OUT_QUAD`, on every destroy path (`fadeAndDestroy`).
- `POPUP_SCROLL_TIME = 100` ms; `NO_MODS_TIMEOUT = 1500` ms auto-finish when no modifier is bound.
- **Mouse policy, precisely:** every key-press path calls `_disableHover()`, which sets `mouseActive = false` and arms `DISABLE_HOVER_TIMEOUT = 500` ms before hover can select again; `_itemEntered` selects only `if (this.mouseActive)`. The keyboard always wins, and pointer jitter after a key press is ignored for half a second.

From the theme ([`widgets/_switcher-popup.scss`](https://gitlab.gnome.org/GNOME/gnome-shell/-/blob/main/data/theme/gnome-shell-sass/widgets/_switcher-popup.scss) + [`_common.scss`](https://gitlab.gnome.org/GNOME/gnome-shell/-/blob/main/data/theme/gnome-shell-sass/_common.scss) + [`_colors.scss`](https://gitlab.gnome.org/GNOME/gnome-shell/-/blob/main/data/theme/gnome-shell-sass/_colors.scss) + [`_default-colors.scss`](https://gitlab.gnome.org/GNOME/gnome-shell/-/blob/main/data/theme/gnome-shell-sass/_default-colors.scss)):

- `$base_padding: 6px`, `$base_border_radius: 8px`, `$modal_radius: 16px` → switcher panel padding **12 px**, radius **28 px** (`$modal_radius + $switcher_padding`), drop shadow `0 8px 8px 0`.
- Panel (`%osd_panel`): background `lighten($_base_color_dark, 5%)` where `$_base_color_dark: #222226` — i.e. near-black ≈ `#2f2f33`-family; 1 px border of foreground at 2% alpha.
- Selected item: `background-color: transparentize($osd_fg_color, 0.8)` — **white at 20% alpha**, a background fill, not a ring. High-contrast mode tightens to 0.7.
- Hover is deliberately neutered, with the comment *"override %tile style so mouse doesn't steal focus"* (`&:hover {background: none;}`).
- Thumbnail width pinned to 256 px in CSS, cross-referenced to `THUMBNAIL_DEFAULT_SIZE` in the comment; caption label is one shared centered label under the row for the window switcher, per-item centered labels under each app icon for the app switcher.

**What Airlock should take:** the 150 ms no-flash show delay rationale (Airlock's quick-tap already commits before mapping — this record's numbers justify keeping it), 100 ms ease-out fade-out, white@20% / accent fill for selection, and GNOME's key-press-kills-hover discipline if pointer support is ever added.

### 2.2 KDE Plasma (KWin TabBox)

Verified from source: KWin `master` ships exactly **one** built-in QML switcher layout, [`src/tabbox/switchers/thumbnail_grid/contents/ui/main.qml`](https://invent.kde.org/plasma/kwin/-/blob/master/src/tabbox/switchers/thumbnail_grid/contents/ui/main.qml) (`src/tabbox/switchers/CMakeLists.txt` installs only `thumbnail_grid`; the `src/tabbox/` tree contains no other layout directories). The classic additional layouts (large_icons, compact, text, …) are **not in-tree anywhere I could locate in Plasma 6** from primary sources — plasma-desktop `master` and its `Plasma/5.27` branch have no `tabbox/` directory at all, and kwin tags back to v5.27.0 carry none either. Treat any claim about Plasma 6 "Large Icons" layout internals as unverifiable; the QML layout *API* (install to `~/.local/share/kwin/tabbox/`, `KPackageStructure: "KWin/WindowSwitcher"`) is documented by the package metadata in-tree and the [KWin API docs](https://develop.kde.org/docs/plasma/kwin/api/).

The one shipped layout, in numbers:

- Dialog centered on `screenGeometry`; `maxWidth = 0.9 × screen width`, `maxHeight = 0.7 × screen height`; column count chosen by a greedy `columnCountRecursion` that minimizes empty cells while keeping the grid no taller than wide.
- `thumbnailWidth = Kirigami.Units.gridUnit * 16` (gridUnit is `18` in Kirigami's platform [`units.cpp`](https://invent.kde.org/frameworks/kirigami/-/blob/master/src/platform/units.cpp) — ~288 px at that default, larger if the platform theme overrides); `captionRowHeight = gridUnit * 2`; `columnSpacing = gridUnit`.
- App icon overlaid at the thumbnail's bottom-center, raised by `iconSize / 4`, at `Kirigami.Units.iconSizes.huge` = **64 px** (units.cpp ladder: 16 / 22 / 32 / 48 / 64 / 128).
- **Selection: `highlight: KSvg.FrameSvgItem { imagePath: "widgets/viewitem"; prefix: "hover" }`** — a theme-drawn rounded frame, and `highlightMoveDuration: 0`: the highlight teleports, it does not slide. `keyNavigationWraps: true`.
- Rows are a `MouseArea` per tile: click activates, hover reveals a close button (`visible: model.closeable && (containsMouse || closeButton.hovered || focus …)`). KDE is the mouse-friendliest of the set.
- Label: `PlasmaComponents3.Label`, centered, `elide: Text.ElideRight`, `textFormat: Text.PlainText`.
- Thumbnails themselves are `KWin.WindowThumbnail` — compositor-internal rendering (see §2.9).

### 2.3 macOS native Cmd+Tab

Documented behavior, quoted from [Apple — Move and arrange app windows on Mac](https://support.apple.com/guide/mac-help/work-with-app-windows-mchlp2469/mac):

- "Scroll through all open apps: Press and hold the Command key, press the Tab key, then press the Left or Right arrow key until you get to the app you want. Release the Command key."
- "If you change your mind … press Esc (Escape) or the Period key, then release the Command key."

Silences, stated explicitly: Apple publishes **no** px sizes, spacing, animation timings, or threshold values for the switcher strip, and the word "mouse" (or click/pointer) appears nowhere in the documentation of the app switcher — the documented interface is keyboard-only. (Third-party observation says clicks pass through; that is not verifiable from Apple's own docs and is not claimed here.) The strip is a horizontal icon row with no thumbnails — the closest mainstream analogue to what Wayland allows an external client to build.

### 2.4 AltTab for macOS (lwouis/alt-tab-macos)

The repo has been rewritten (v2 tree under `src/switcher/`); all numbers below are read from current `master`. Three appearance styles ship, of which **"titles"** is the direct analogue to Airlock's list, from [`src/switcher/Appearance.swift`](https://github.com/lwouis/alt-tab-macos/blob/master/src/switcher/Appearance.swift):

| Metric | titles style (S/M/L) | appIcons style | thumbnails style |
|---|---|---|---|
| Rows | 1 | 1 | 3–8 (6 on portrait) |
| App icon size | 18 / 24 / 30 px | 70 / 110 / 150 px | 16 / 26 / 28 px (badge) |
| Font size | 13 / 14 / 16 px | 13 / 14 / 16 px | 13 / 14 / 16 px |
| Panel padding | 18 | 25 | 18 |
| Panel corner radius | 23 | 23 | 23 |
| Cell corner radius | 10 | 10 | 10 |
| Tile width | 60–90% of screen | 4–30% of screen | computed from screen ratio |
| Max height | — | — | 80% of screen (`maxHeightOnScreen = 0.8`) |

- **Selection treatment** (the most citable detail): focused item = **3 px accent-colored border** (2 px in titles style) + **accent fill at 0.2 alpha**; hovered = accent fill at 0.1 alpha + accent border at 0.7 alpha. Border + tint, never a checkmark or scale change.
- Panel material is a translucent blur (`NSVisualEffectView.Material` ultraDark default, dark/light variants); font color white@0.85 on dark, black@0.8 on light; icons/thumbnails get gray@0.8 drop shadows. Spacing constants: `interCellPadding 1`, `intraCellPadding 5`, `appIconLabelSpacing 2`.
- Grid math ([`src/switcher/main-window/TileGridLayout.swift`](https://github.com/lwouis/alt-tab-macos/blob/master/src/switcher/main-window/TileGridLayout.swift)): fixed-height tiles of per-tile width wrap at `widthMax`, each row centered (`centeringOffsets`), RTL-aware. Auto-size = first candidate height that fits ([`firstSizeThatFits`](https://github.com/lwouis/alt-tab-macos/blob/master/src/switcher/main-window/TileGridLayout.swift)).
- Behavior defaults from [`src/preferences/Preferences.swift`](https://github.com/lwouis/alt-tab-macos/blob/master/src/preferences/Preferences.swift): `windowDisplayDelay: "100"` ms; `mouseHoverEnabled: "false"` — **mouse off by default**; `fadeOutAnimation: "false"` — no fade by default; cancel = `⎋`; `shortcutStyle: focusOnRelease`; `showOnScreen: active`; `spacesToShow`/`screensToShow`: all; `windowOrder: recentlyFocused`. [`TilesPanel.swift`](https://github.com/lwouis/alt-tab-macos/blob/master/src/switcher/main-window/TilesPanel.swift) keeps an ordered-out panel alive for `keyHoldLimit = 100` ms after focus to cover release races, and panel level is `.popUpMenu`.
- Labels in icon mode show only for the focused/hovered tile (`TileView.updateAppIconsLabel`) — one label at a time, centered under the icon.

### 2.5 Windows 10/11 Alt+Tab / Task View

What Microsoft actually documents ([Keyboard shortcuts in Windows](https://support.microsoft.com/en-us/windows/keyboard-shortcuts-in-windows-dcc61a57-8ff0-cffe-9796-cb9706c75eec)): Alt+Tab "Switch between open windows. To cycle through multiple windows, continue to hold the Alt key and press Tab multiple times"; **Ctrl+Alt+Tab** "View a thumbnail of all open apps. Use the arrow keys to switch between open apps"; Win+Tab "Open Task View."

Silences, stated explicitly: Microsoft publishes no px sizes, ring widths, dimming percentages, or the "graphite fade" animation values for the Alt+Tab overlay in any Learn page I could reach; the old DWM Thumbnail Overview page (`learn.microsoft.com/.../dwm/dwm-thumbnail-overview`) returns 404 as of 2026-11-01, so even the developer-side statement that Alt+Tab previews are DWM live thumbnails currently has no live primary citation. Visual specifics circulating in community posts are not cited here. The one transferable documented fact: **arrow-key navigation is bound to the modifier-held variant (Ctrl+Alt+Tab), not plain Alt+Tab** — plain Alt+Tab is Tab-cycle only on Windows.

### 2.6 COSMIC's own switcher (what Airlock replaces)

COSMIC's Alt+Tab is not drawn by the compositor: the default keybind spawns a separate binary. Primary evidence, both local and upstream: `/usr/share/cosmic/com.system76.CosmicSettings.Shortcuts/v1/system_actions` (shipped by `cosmic-settings-daemon`) maps `WindowSwitcher: "cosmic-launcher alt-tab"` / `WindowSwitcherPrevious: "cosmic-launcher shift-alt-tab"`, identical to upstream [`cosmic-settings-daemon/data/system_actions.ron`](https://github.com/pop-os/cosmic-settings-daemon/blob/master/data/system_actions.ron); `cosmic-launcher --help` lists `alt-tab` / `shift-alt-tab` subcommands.

The alt-tab view is built in [`cosmic-launcher/src/app.rs`](https://github.com/pop-os/cosmic-launcher/blob/master/src/app.rs) (`view_window`, `alt_tab` branch) — read from `master`:

- Content column: **`width(Length::Fixed(600.))`, `max_width(600)`, `spacing(16)`**, height shrink — the whole switcher is a 600 px card. The alt-tab branch omits the launcher's search entry entirely.
- Each row is a full-width button, **`padding([8, 24])`**, corner radius `theme.cosmic().corner_radii.radius_s` (bumped `+4` where ≥ 4 on the outer card), `outline_width: 0.0`; the focused row is styled as the **hovered `Button::Text`** style — a filled rounded rectangle, not an outline.
- Row content: **app icon at `Length::Fixed(32.0)`**, then a fill-portion column of `text::body` name + `text::caption` description, each `ellipsize(End, Lines(1))`; rows are separated by `divider::horizontal::light()`. In launcher mode a 16 px category icon leads, and the first ten rows carry right-aligned `Ctrl + N` badges.
- **`SCROLL_MIN = 8`**: at most 8 buttons render flat; more wraps in a `scrollable` capped at `max_height(504)`.
- Card: `padding([24, 32])`, background `theme.cosmic().background(transparent)` base, **1 px `bg_divider()` border**, `Shadow::default()`, radius `radius_s + 4`. No blur claim — the layer-shell surface is an opaque themed card.
- Commit: `alt_tab_released` (modifier release detected by `alt_tab_modifier_is_released`) sends `Activate(None)`; Tab/Shift-Tab step. This is the 170 MB RSS process Airlock's README measures against.

### 2.7 rofi window mode (the Wayland-native UX reference)

From [`doc/rofi.1.markdown`](https://github.com/davatorium/rofi/blob/next/doc/rofi.1.markdown) (read from `next`):

- `-window-format` default: **`{w}  {c}   {t}`** — desktop/workspace name, window class, title, as columns. Fields: `w` desktop name, `t` title, `n` name, `r` role, `c` class; `{field:len}` truncates *or pads* to a fixed width — fixed-width columns keep scan-ability. This is the canonical "workspace tag + app + title" row that substitutes for thumbnails.
- `-window-thumbnail`: "Show window thumbnail (if available) as icon in the window switcher" — availability-dependent, X11/EWMH only in practice.
- `-window-command` default `wmctrl -i -R {window}`; under Wayland `{window}` is the ext-foreign-toplevel [stable identifier](https://wayland.app/protocols/ext-foreign-toplevel-list-v1#ext_foreign_toplevel_handle_v1:event:identifier), cited in the manpage itself. `shift-delete` closes the selected window; `shift-enter` runs the alt command.
- Icon resolution, from source: X11 mode tries the `_NET_WM_ICON` EWMH property first, then `rofi_icon_fetcher_query(class_lower, size)` — the WM class **lowercased** into the icon theme ([`source/modes/window.c`](https://github.com/davatorium/rofi/blob/next/source/modes/window.c)); Wayland mode does the same with app_id, under the comment *"some apps don't have app_id (WM_CLASS). this is fine"* — `g_utf8_strdown(toplevel->app_id)` then theme lookup ([`source/modes/wayland-window.c`](https://github.com/davatorium/rofi/blob/next/source/modes/wayland-window.c)).

### 2.8 Coverflow Alt-Tab (animated 3D switcher, pitfalls)

From the [README](https://github.com/dsheeler/CoverflowAltTab/blob/master/README.md): Esc cancels, Return picks, `d` hides all windows, `q` closes the current window ("mostly works, might have some edge case bugs" — the only acknowledged bug note), arrows / mouse wheel / trackpad cycle, Shift reverses; three modes (windows grouped by application / flat windows / one application's windows across workspaces); D-Bus `launch/next/previous/select` for hot-corner triggers. The README is silent on sizes, easing curves, and timings — those live in its settings UI, not a citable doc. The relevant design datapoint is the contrast: every mainstream switcher in §2.1–2.6 moves the selection instantly with at most a 100 ms fade-out, while the animated 3D covers are the extension's entire reason to exist — and it needs compositor-internal window clones to do it (not portable, see §2.9).

### 2.9 What Wayland takes off the table

- The standardized toplevel protocol is metadata-only by design: [`ext-foreign-toplevel-list-v1`](https://gitlab.freedesktop.org/wayland/wayland-protocols/-/blob/main/staging/ext-foreign-toplevel-list/ext-foreign-toplevel-list-v1.xml) describes itself as *"intentionally minimalistic"* and defers *"additional functionality (e.g. creating a screencopy source from a toplevel handle …) to extension protocols"*, and notes *"the compositor may choose to restrict this protocol"*. It carries title, app_id, identifier — no pixels. Same for [`wlr-foreign-toplevel-management-unstable-v1`](https://gitlab.freedesktop.org/wlroots/wlr-protocols/-/blob/master/unstable/wlr-foreign-toplevel-management-unstable-v1.xml), whose `app_id` event is documented only as "app-id change."
- Getting pixels requires a capture protocol: [`wlr-screencopy-unstable-v1`](https://gitlab.freedesktop.org/wlroots/wlr-protocols/-/blob/master/unstable/wlr-screencopy-unstable-v1.xml) is now marked deprecated, and COSMIC's compositor implements the modern [`ext-image-copy-capture`](https://github.com/pop-os/cosmic-comp/tree/master/src/wayland/handlers/image_copy_capture) instead ([`src/wayland/handlers/image_copy_capture/`](https://github.com/pop-os/cosmic-comp/tree/master/src/wayland/handlers/image_copy_capture)). That path implies per-frame capture permission and cost — the wrong tool for a switcher that must map in one frame.
- GNOME and KDE show thumbnails only because the switcher runs **inside** the compositor (gnome-shell clones live window actors; KWin's QML uses the internal `KWin.WindowThumbnail` type). Airlock is an external client on `zcosmic_toplevel_info_v1` (src/wayland.rs:107-143), so the honest ceiling is: **app icon + title + state/workspace tags**, exactly what rofi's window mode and macOS's Cmd+Tab strip do.
- cosmic-comp itself ships no switcher UI to imitate — the system default delegates to cosmic-launcher (§2.6). No cosmic-ext overlay app with richer visuals was found to cite; the launcher's alt-tab view **is** the platform's switcher design.

### 2.10 Cross-platform synthesis

| Property | GNOME | KDE (thumbnail_grid) | macOS native | AltTab (titles) | cosmic-launcher | Airlock today |
|---|---|---|---|---|---|---|
| Layout | single row, wraps | centered grid | horizontal strip | single row / grid | vertical list, 600 px | vertical list |
| Item size | icon 96 (ladder 22–96) | thumb 16 gu, icon 64 | undocumented | icon 24–30, font 14–16 | icon 32, font body/caption | glyph 8×8 |
| Selection | fg @20% fill | viewitem hover frame | undocumented (ring) | accent @0.2 fill + 2–3 px border | hovered Text-button fill | fixed blue fill, full row |
| Selection motion | instant | instant (`highlightMoveDuration: 0`) | n/a | instant | instant | instant |
| Fade out | 100 ms ease-out | none | undocumented | off by default (option) | none | none |
| Show delay | 150 ms | none | undocumented | 100 ms | none | quick-tap path |
| Mouse | hover suppressed 500 ms after key press | click + hover close | not documented | off by default | click rows (launcher heritage) | pass-through |
| Wrap | yes | yes | n/a | yes | no (scroll) | yes |

Consensus is remarkably tight: **icon-or-thumbnail + single-line label, tinted-fill selection, instant selection movement, keyboard ownership, ≤ 600–700 px card, ~100–150 ms of delay somewhere in show/hide.**

---

## 3. App icon acquisition on COSMIC / freedesktop

### 3.1 The specs

**Icon Theme spec** ([specifications.freedesktop.org/icon-theme](http://specifications.freedesktop.org/icon-theme/latest/)):

- Base dirs, in order: `$HOME/.icons`, `$XDG_DATA_DIRS/icons`, `/usr/share/pixmaps`. A theme may be spread across base dirs.
- Lookup: exact-size directories first, then all directories scanned with the smallest `DirectorySizeDistance` (per-directory Type Fixed/Scalable/Threshold — closest match wins, not strictly closest-below).
- **hicolor is the mandatory final fallback**: implementations "are required to look in the 'hicolor' theme if an icon was not found in the current theme"; themes with no `Inherits` get hicolor appended; the search through the inheritance chain stops at the first theme containing the name at any size.
- The spec itself carries **no canonical fixed-size ladder** (its example shows only 32/48 Fixed dirs and a Scalable dir with `MinSize 1 / MaxSize 256`); the widely-cited 16→512 ladder is convention, not spec text. Scalable/SVG directories serve any size within Min/Max; SVG support is optional ("implementations without it should ignore .svg").
- Cache note: caching implementations must stat icon directory mtimes on lookup "unless it already did so less than 5 seconds ago."

**Desktop Entry spec** ([specifications.freedesktop.org/desktop-entry](http://specifications.freedesktop.org/desktop-entry/latest/)):

- `Icon` key: *"If the name is an absolute path, the given file will be used. If the name is not an absolute path, the algorithm described in the Icon Theme Specification will be used to locate the icon."* (type `iconstring`.)
- Desktop-file ID: *"make its full path relative to the $XDG_DATA_DIRS component in which the desktop file is installed, remove the `applications/` prefix, and turn `/` into `-`"*; first match in `$XDG_DATA_DIRS` precedence wins; files outside an `applications/` dir have no ID.
- Basename convention: reverse-DNS (`org.example.FooViewer.desktop`), characters limited to `[A-Za-z0-9-_]` per dot-segment.

### 3.2 app_id semantics on Wayland, and the matching problem

- The root definition is xdg-shell's `set_app_id` ([wayland.app protocol page](https://wayland.app/protocols/xdg-shell#xdg_toplevel:request:set_app_id)): an app ID identifies the class of application, is used as the D-Bus service name for activatable apps, and — key sentence — it is only *"suggested to select app ID's that match the basename of the application's .desktop file"*. Suggestion, not requirement.
- Both toplevel-metadata protocols define `app_id` as a bare string with no format constraint: [`wlr-foreign-toplevel-management`](https://gitlab.freedesktop.org/wlroots/wlr-protocols/-/blob/master/unstable/wlr-foreign-toplevel-management-unstable-v1.xml) ("app-id change") and [`ext-foreign-toplevel-list`](https://gitlab.freedesktop.org/wayland/wayland-protocols/-/blob/main/staging/ext-foreign-toplevel-list/ext-foreign-toplevel-list-v1.xml) (same, plus a separate stable `identifier` event for identity). **The protocols are silent on casing, `.desktop` suffixes, and prefixes** — normalization is client convention, which is why every consumer below implements its own fallback chain.
- Consequences in the wild (from the consumers' own code, §3.4–§3.5): Electron sets per-role IDs (`code-url-handler`, `code`), some toolkits emit capitalized or suffixed IDs, and some windows have no app_id at all (rofi: "some apps don't have app_id (WM_CLASS). this is fine").

### 3.3 libcosmic's icon infrastructure

Read from [`pop-os/libcosmic`](https://github.com/pop-os/libcosmic) `master`:

- [`src/widget/icon/named.rs`](https://github.com/pop-os/libcosmic/blob/master/src/widget/icon/named.rs): `icon::from_name(name)` returns a `Named` builder with `.size(u16)`, `.scale(u16)`, `.prefer_svg(bool)`, `.fallback(IconFallback)` (`IconFallback::Default` strips trailing `-segments` progressively, `IconFallback::Names([...])` tries explicit names), and `.with_extra_paths`. `Named::path()` performs the lookup via `freedesktop_icons::lookup(name).with_theme(theme).with_cache()` — the theme chain is the configured theme, then `"Cosmic"` (`src/icon_theme.rs`: `pub const COSMIC: &str = "Cosmic"`, `set_default()`); a miss falls through to bundled icons (`bundle::get`) and finally an empty SVG handle.
- [`src/widget/icon/handle.rs`](https://github.com/pop-os/libcosmic/blob/master/src/widget/icon/handle.rs): `Handle` data is `Data::{Image, Svg}`; constructors `from_path`, `from_svg_bytes`, `from_raster_bytes`, `from_raster_pixels`. `from_path` treats a file as SVG by extension **or** by sniffing the XML root element via `roxmltree` (≤ 16 MiB), else raster.
- Cargo ([`Cargo.toml`](https://github.com/pop-os/libcosmic/blob/master/Cargo.toml)): unix deps are `freedesktop-icons = { package = "cosmic-freedesktop-icons", git = "https://github.com/pop-os/freedesktop-icons" }` and optional `freedesktop-desktop-entry = "0.8.4"`. SVG rendering goes through iced's `svg` feature → `iced_tiny_skia`'s `svg` feature → **`resvg`** (verified in the vendored [iced tiny_skia Cargo.toml](https://github.com/pop-os/iced/blob/master/tiny_skia/Cargo.toml)).
- The pop-os fork (`cosmic-freedesktop-icons`) adds `with_extra_paths` over the crates.io crate; otherwise the API matches (below).

### 3.4 The COSMIC switcher pipeline (the pattern to copy)

pop-launcher's COSMIC toplevel plugin, [`plugins/src/cosmic_toplevel/mod.rs`](https://github.com/pop-os/launcher/blob/master/plugins/src/cosmic_toplevel/mod.rs):

1. At startup, enumerate desktop entries once: `fde::Iter::new(fde::default_paths())` → `Vec<DesktopEntry>` (`freedesktop_desktop_entry` crate).
2. Per toplevel: `let appid = fde::unicase::Ascii::new(info.app_id.as_str())` — **case-insensitive** — then `fde::find_app_by_id(&self.desktop_entries, appid)` with fallback `DesktopEntry::from_appid(appid)`.
3. Icon name: `entry.icon()`, else `"application-x-executable"`; shipped to the UI as `IconSource::Name(icon_name)`. Row text: window title as description, app name as name; search matches app_id, title, and desktop-entry keywords.

`find_app_by_id`'s exact matching ladder, from the crate source (docs.rs [freedesktop-desktop-entry 0.8.4](https://docs.rs/freedesktop-desktop-entry/latest/freedesktop_desktop_entry/), *"Case-insensitive search of desktop entries for the given app ID"*):

1. `matches_wm_class` — `StartupWMClass` equals the app_id (exact, case-insensitive via `Ascii`).
2. `matches_id` — desktop-file id equals, **or** path file-stem equals, **or** the id's last dot-segment equals the stem (this is what catches `code-url-handler` vs `code`-style partial matches).
3. `matches_name` — the localized `Name` equals.
4. `from_appid` synthesizes a nameless entry whose `Name` is the app_id's last dot-segment (used for the label when nothing matched; it has no `Icon=`).

`default_paths()` = XDG data home + data dirs, each joined with `applications` (the spec's search order, §3.1).

Then cosmic-launcher resolves the icon name for display ([`src/app.rs`](https://github.com/pop-os/cosmic-launcher/blob/master/src/app.rs)):

```rust
icon::from_name(&**name)
    .prefer_svg(true)
    .size(64)
    .fallback(Some(IconFallback::Names(vec![
        "application-default".into(),
        "application-x-executable".into(),
    ])))
    .handle()
```

with the wrinkle that an `IconSource::Name` containing `/` is first checked as a filesystem path (`icon::from_path`). The row then displays it at 32×32 (§2.6) even though it was looked up at size 64 — lookup size and display size are decoupled.

### 3.5 rofi's fallback (the minimalist version)

rofi never touches .desktop files in window mode: EWMH `_NET_WM_ICON` → theme lookup of the **lowercased** WM class (X11) or lowercased app_id (Wayland), cached per (size, scale) ([`window.c`](https://github.com/davatorium/rofi/blob/next/source/modes/window.c), [`wayland-window.c`](https://github.com/davatorium/rofi/blob/next/source/modes/wayland-window.c)). Cheap, works for most apps, misses everything the desktop entry's `Icon=` knows but the toolkit doesn't.

### 3.6 Crate choices for Airlock (no libcosmic dependency)

Airlock depends only on `cosmic-client-toolkit` + `toml` (Cargo.toml), renders software, and holds a 4 MB RSS budget (README) — so the icon stack must be picked deliberately:

| Crate | Role | Status from primary sources |
|---|---|---|
| [`freedesktop-desktop-entry`](https://docs.rs/freedesktop-desktop-entry/latest/freedesktop_desktop_entry/) (0.8.4) | app_id → `Icon=` name | exactly what pop-launcher uses; `find_app_by_id` ladder in §3.4; MPL-2.0 |
| [`freedesktop-icons`](https://docs.rs/freedesktop-icons/latest/freedesktop_icons/) (0.4.0, MIT) | name → file path | builder `lookup(name).with_theme/with_size/with_scale/with_cache/force_svg().find()`; default theme hicolor, default size 24, scale 1; **PNG preferred over SVG by default**, `force_svg` inverts; falls back to hicolor then `/usr/share/pixmaps`; internal cache. The pop-os fork adds `with_extra_paths` but is git-only |
| [`xdg`](https://docs.rs/xdg/latest/xdg/) | base dirs | pulled in transitively by both crates above; not needed directly |
| `linicon` | — | not surveyed from primary sources in this pass; no claim made |
| `image` | PNG/XPM decode | needed to rasterize PNG results into ARGB for the software renderer |
| `resvg` (usvg + tiny-skia) | SVG rasterization | the stack libcosmic itself reaches through iced; heaviest option — needed only if SVG icons must render |

### 3.7 SVG rendering

libcosmic/iced rasterize SVG with **resvg** (§3.3); `Handle::from_path` sniffs SVG vs raster by extension plus XML root. For Airlock, the pragmatic ladder is: prefer PNG at the row size from the icon-theme lookup (crates.io `freedesktop-icons` already prefers PNG), rasterize with `image` into the overlay's ARGB canvas, and only pull `resvg` in if the PNG-miss rate on real desktops justifies it — note the spec marks SVG support optional for implementations (§3.1). `application-x-executable` and `application-default` both exist as PNGs in hicolor on every normal install, so the terminal fallback is raster-cheap.

---

## 4. Shipping Airlock's own icon (COSMIC app conventions)

The house pattern, identical across first- and third-party COSMIC apps:

- **cosmic-edit** ([pop-os/cosmic-edit](https://github.com/pop-os/cosmic-edit)): desktop file `res/com.system76.CosmicEdit.desktop` carries **`Icon=com.system76.CosmicEdit`** — the icon name is the app ID. The icon ships as `res/icons/hicolor/{16x16,24x24,32x32,48x48,64x64,128x128,256x256}/apps/com.system76.CosmicEdit.svg` — the same SVG placed in every fixed size directory, no `scalable/`. The [justfile](https://github.com/pop-os/cosmic-edit/blob/master/justfile) installs it:

  ```just
  icons-src := 'res' / 'icons' / 'hicolor'
  icons-dst := clean(rootdir / prefix) / 'share' / 'icons' / 'hicolor'
  # in `install`:
  for size in `ls {{icons-src}}`; do \
      install -Dm0644 "{{icons-src}}/$size/apps/{{APPID}}.svg" "{{icons-dst}}/$size/apps/{{APPID}}.svg"; \
  done
  ```

  The `.desktop`/metainfo are generated into `target/xdgen` by `build.rs` (`xdgen`), and the About dialog references the icon as `icon::from_name(Self::APP_ID)`.
- **cosmic-ext-tasks** ([cosmic-utils/cosmic-ext-tasks](https://github.com/cosmic-utils/cosmic-ext-tasks/blob/main/justfile)): byte-identical justfile recipe (plus a flatpak destination), but the icon ships as `res/icons/hicolor/scalable/apps/dev.edfloeshz.Tasks.svg` — the `scalable/` single-SVG variant. The loop works unchanged because it iterates whatever size directories exist. The wider cosmic-utils org (tweaks, toot, now-playing, chronos, …) all share this template.
- Two valid shapes, one rule: **icon name == desktop-file basename == reverse-DNS app ID, installed into `icons/hicolor/…/apps/`**. Airlock's README already flags the app-ID decision (`io.github.<user>.Airlock`) as the packaging blocker; that decision is also the icon name.

---

## 5. Recommendations for Airlock

### 5.1 Icon pipeline (priority order)

1. **Adopt pop-launcher's mapping verbatim**: add `freedesktop-desktop-entry` (0.8.4) and at daemon start enumerate `fde::Iter::new(fde::default_paths())` once into a `Vec<DesktopEntry>`; per toplevel resolve `fde::find_app_by_id(entries, unicase::Ascii(app_id))`, fallback `from_appid`, take `entry.icon()` else `"application-x-executable"`. This inherits StartupWMClass → id → stem → last-segment matching and case-insensitivity for free (§3.4).
2. **Resolve name → file with `freedesktop-icons`** (crates.io): `lookup(name).with_theme("Cosmic").with_cache().with_size(row_icon_px).find()` — hicolor fallback and pixmaps are built in; PNG-first matches the software renderer. Cache resolved paths in the daemon's snapshot alongside titles, keyed by app_id.
3. **Rasterize to ARGB once per (app_id, scale)**: `image` for PNG; box/scale into the row's icon cell in `render()` (src/ui.rs:161-190 already owns every pixel). Keep the raster cache bounded (it is one small bitmap per distinct application).
4. **Only add `resvg` if PNG misses hurt** — one more dep and ~MBs of code; every distribution ships hicolor PNGs for the common case, and the fallback names are guaranteed PNG (§3.7).
5. **Grow the row metrics to carry a real icon**: icon cell 32 px (cosmic-launcher §2.6; KDE's is 64 inside a thumbnail tile — too big for a text list), vertically centered, 12 px gap to text. This replaces `ICON_W=8`/`WINDOW_GLYPH` (src/ui.rs:37-42, 99-101) — keep the 8×8 window glyph as the `application-x-executable` fallback shape.
6. **Airlock's own icon**: pick the app ID (already the packaging blocker), ship `res/icons/hicolor/scalable/apps/<APPID>.svg` + `<APPID>.desktop` with `Icon=<APPID>` (§4). Install user-level into `~/.local/share/{applications,icons/hicolor}` to honor the "never touch /usr/share" rule in the README.

### 5.2 Overlay visual design

Concretely, against the constants at src/ui.rs:31-101:

1. **Card**: cap width at 600 px (cosmic-launcher; GNOME's row wraps instead — a vertical list wants the cap); keep the near-opaque `0xFA_1E_21_26` background — it sits within a few RGB points of GNOME's `lighten(#222226,5%)` OSD panel (§2.1); add a 1 px border at white ~4–8% alpha (`bg_divider` role) and an 8–12 px corner radius (`radius_s + 4` ≈ 12 on COSMIC). Rounded corners in the software renderer = corner-cut alpha — the layer already uses ARGB (src/ui.rs:69-84), so it composites for free.
2. **Selection**: full-row fill in the **theme accent at ~20% alpha** over the card (GNOME uses foreground@20%; alt-tab-macos uses accent@0.2 — accent reads better on COSMIC and matches cosmic-launcher's hovered-fill). Optionally add the alt-tab-macos 2 px accent border on the selected row for light wallpapers. Keep `SELECT_BLEED`-style full-row extent; do not shrink to an icon ring.
3. **Rhythm**: row height ≈ 48 px (32 px icon + 8 px breathing), text one line, **elide right with `…`** (KDE `ElideRight`, cosmic-launcher 1-line ellipsis) rather than the current hard 48-column cut + `..` (src/ui.rs:233-240); 8 px row padding vertical, 24 px horizontal (cosmic-launcher `padding([8,24])`).
4. **Long lists**: hard-cap visible rows by ~70% of output height (KDE `maxHeight 0.7`; alt-tab 0.8); beyond ~8 rows (cosmic-launcher `SCROLL_MIN = 8`), either page the list (Selection window) or shrink row density — do not silently overflow the layer.
5. **Motion budget**: selection moves instantly (every platform; KDE pins `highlightMoveDuration: 0`). Add fade-out ~100 ms ease-out on commit/cancel (GNOME `POPUP_FADE_OUT_TIME`); a software-rendered alpha fade is N cheap full-card blends, or can be skipped with no UX penalty (alt-tab-macos ships it off by default).
6. **Substitute-for-thumbnails affordances**: keep title + icon as primary; if windows need disambiguation, rofi's pattern is a right-aligned workspace tag column (fixed-width), which Airlock's workspace isolation makes mostly unnecessary.

### 5.3 Interaction feel

1. **Keep the keyboard-grab ownership model** (src/ui.rs:446-454) and keep pointer events pass-through — GNOME's own comment is the justification ("so mouse doesn't steal focus"), macOS documents no mouse at all, and alt-tab-macos defaults mouse off (§2.1, §2.3, §2.4). If pointer interaction is ever added, copy GNOME's discipline exactly: key press disables hover for 500 ms (`DISABLE_HOVER_TIMEOUT`), and clicks inside the card activate only the hovered row.
2. **Show delay**: keep the current quick-tap-first design; if the overlay ever flashes on fast taps, the precedent number is 100–150 ms (alt-tab-macos 100, GNOME 150) with the popup mapped but `opacity 0` — commit still rides the first modifier release, so a sub-delay release never paints.
3. **Timing constants to adopt wholesale**: fade-out 100 ms ease-out; idle auto-close can stay at the current 2 s (src/ui.rs:63) — GNOME's `NO_MODS_TIMEOUT` is 1500 ms in the same role; a 100 ms grace after focus before unmapping (alt-tab `keyHoldLimit`) matches the existing release-race handling.
4. **Type-to-filter stays rejected** (previous record's verdict) — no platform surveyed here ships it inside the switcher either; it is launcher territory (cosmic-launcher's alt-tab view omits the search entry too).

---

## 6. Gaps and honest silences

- **Windows**: no official px/ring/dimming/fade values; Microsoft's shortcut page documents only interaction; the DWM thumbnail overview page is 404 as of 2026-11-01 (§2.5).
- **macOS**: Apple documents no visual metrics or timing thresholds for Cmd+Tab; mouse support is absent from the documentation rather than documented-as-absent (§2.3).
- **KDE Plasma 6**: only `thumbnail_grid` is in-tree; the "Large Icons"/"compact"/"text" layouts could not be located in any current primary-source tree (plasma-desktop `master`, `Plasma/5.27`, kwin `master`, kwin `v5.27.0` all checked negative) — claims about their current internals are unverified (§2.2).
- **Ubuntu Dock switcher**: not covered; Ubuntu's alt-tab is GNOME Shell's altTab.js in origin, but Ubuntu-specific patching was not verified from primary sources in this pass.
- **cosmic-ext overlays**: no third-party COSMIC overlay with richer visuals was found to cite; cosmic-launcher's alt-tab view is the only in-ecosystem reference (§2.9).
- **Coverflow Alt-Tab**: behavior verified from its README; visual/animation metrics live in its settings UI, not in citable docs (§2.8).
- **linicon / beeflib**: not surveyed from primary sources; no recommendation made (§3.6).
