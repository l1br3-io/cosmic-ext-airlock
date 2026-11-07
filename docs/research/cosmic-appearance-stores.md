# COSMIC appearance stores — where "follow the desktop" settings live — research findings

**Date:** November 7, 2026 · **Author:** research agent for jpt · **Status:** Complete — ready to feed the revised theming/typography/palette specs (store facts verified; every mapping decision is left to the spec gate, §9)

**Scope:** where and how COSMIC persists the four ratified "follow" settings (accent, style/radius, density, interface font) plus the two dropped-as-unmappable rows (tiling gaps, active-window-hint), so the revised specs cite stores, not assumptions. Follow-up to [appearance-settings.md](appearance-settings.md) (§5.1 verified the accent store; carried forward, not re-researched) and the hand-read precedent in `src/theme.rs`. The mappings themselves are candidates, not verdicts.

**Method:** primary sources only — the local COSMIC install's stores (`/usr/share/cosmic`, `~/.config/cosmic`, read 2026-11-07; installed vintage pinned by `dpkg`: cosmic-settings `1.11.0~1792526230~24.04~f1d00fc`, built 2026-10-20 per its vendored-path strings, cosmic-comp `0.1~1792508925~24.04~8cb1802`) and upstream source read over the GitHub API (`gh api`, pop-os/cosmic-settings, pop-os/libcosmic — cosmic-theme, cosmic-config, src/config — pop-os/cosmic-comp, `master`, fetched 2026-11-07). The installed cosmic-settings binary itself was strings-probed for vintage divergence. Remote code cited by repo/file + constant; Airlock claims carry `file:line`.

---

## 1. TL;DR — one row per store

All stores are per-key RON files under `~/.config/cosmic/<id>/v<n>/<key>` (user) and `/usr/share/cosmic/<id>/v<n>/<key>` (system default); read ladder §2.

| Setting | Store / key (installed vintage) | Shape (quoted §3) | Default when absent | On installed 1.11.0? | In scope? | One-line cause |
|---|---|---|---|---|---|---|
| **Polarity** † | `com.system76.CosmicTheme.Mode/v1/is_dark` | bare RON bool | `true` (mode.rs `Default`; system file also `true`) | yes | carry | record #1 + `src/theme.rs:22-31`; the ladder every other store borrows |
| **Accent** † | `com.system76.CosmicTheme.{Dark,Light}/v2/accent` (v1 user fallback) | 12-field `Component`; v2 = `#RRGGBBAA` hex strings, v1 = f32 structs; read `base` | teal `#63D0DFFF` | yes | **in** | per-polarity store, two serializations, `base` is the band color; the neutral-lift re-open (record #1 §5) |
| **Style → radius** | `com.system76.CosmicTheme.{Dark,Light}/v2/corner_radii` | `CornerRadii`: 6 × `[f32; 4]` | Round `0/4/8/16/32/160` | yes | **in** | three presets in source (§5); this machine's user file is Square (all 2 s) |
| **Density** | `com.system76.CosmicTk/v1/interface_density` (+ expanded `spacing` in the theme, §3.3) | RON enum variant `Compact` / `Standard` / `Spacious` | `Standard` | **yes** (binary strings: `interface_density`, `Spacious`, UI key `interface-density`) | **in** | newer feature but present on this vintage; the enum's middle name is Standard, the UI labels it "Comfortable" |
| **Interface font** | installed: `com.system76.CosmicTk/v1/interface_font`; master (since 2026-10-22): `com.system76.CosmicTheme.{Dark,Light}/v2/font` (+ `mono_font`) | `FontConfig` struct: family/weight/stretch/style | `Open Sans` (`DEFAULT_FONT_FAMILY`; mono `Noto Sans Mono`) | **no store file anywhere on this machine** | **in** | the store moved Tk→Theme two days after this build (§7); default is a named family, not an abstraction |
| **Tiling gaps** | `com.system76.CosmicTheme.{Dark,Light}/v2/gaps` | `(outer, inner)` u32 pair | `(0, 8)` | yes | **out** | tiling spacing; recorded so it is never re-researched |
| **Active-window hint** | `com.system76.CosmicComp/v1/active_hint` (bool) + theme `v2/active_hint` (u32 width) + `v2/window_hint` (color) | bare bool; bare u32; `Option<Srgb>` | width `3`, color `None`, comp bool unset | yes (user comp bool `false`) | **out** | compositor border/hint; recorded so it is never re-researched |

† Carried from record #1 §5.1 / `src/theme.rs`; re-confirmed on disk 2026-11-07 — paths unchanged, user Dark v1 accent still the purple, system v1 accent still the quoted teal.

---

## 2. The store convention (cosmic-config), verified from source

pop-os/libcosmic `cosmic-config/src/lib.rs` (master; the standalone pop-os/cosmic-config repo is gone — 404, the crate is a libcosmic workspace member):

- **Path**: user `$XDG_CONFIG_HOME` (absolute) else `$HOME/.config`, plus `cosmic/<id>/v<version>/<key>` (`Config::new_inner`); system default via `xdg::BaseDirectories::with_prefix("cosmic").find_data_file` → `/usr/share/cosmic/<id>/v<version>/<key>`. Exactly the two roots `src/theme.rs:17,37-48` hand-reads.
- **One key = one RON file** (`ConfigGet::get_local` → `fs::read_to_string` + `ron::from_str`). No bundling, no index.
- **Version ladder, the load-bearing rule**: `Config::new(id, 2)` chains a `previous: Config` for v1, and `get(key)` = **user v2 → user v1 → system v2** — `get_local` recurses into `previous` on a miss; `get_system_default` reads its own version only (system v1 is never consulted by a v2 config). Consequence, proven on this machine: the user's v2 Dark store has *no* `accent`, so the live accent is the **v1** purple — record #1's v1 read path still returns the live value. A faithful follower reads user v2, user v1, then system v2.
- **Theme ids/versions** (`cosmic-theme/src/model/theme.rs`): `DARK_THEME_ID`/`LIGHT_THEME_ID` = `com.system76.CosmicTheme.{Dark,Light}`, `#[version = 2]` on `Theme` (frosted-era); `THEME_MODE_ID` = `com.system76.CosmicTheme.Mode`, `#[version = 1]` (`mode.rs`). The installed vintage is v2-era (ships full v2 default schemas; binary strings carry the frosted keys). `CosmicTk` = `com.system76.CosmicTk`, v1 (`src/config/mod.rs`).
- **Writes go to both polarities**: cosmic-settings' `theme_manager` calls `set_spacing`/`set_corner_radii`/… on dark *and* light (`theme_manager.rs` `set_spacing`, `set_gap_size`, `set_active_hint`); a follower's polarity question is only "which side to read", never "which side was written".
- Note for the hand-read: `Config::new` **creates** the user version dir (`fs::create_dir_all`) as a side effect; Airlock's read-only `std::fs::read` pattern avoids that correctly.

---

## 3. Per-store detail (user path → system default; real quoted content)

### 3.1 Accent — `Theme.{Dark,Light}/v2/accent` (v1 fallback)

- User: `~/.config/cosmic/com.system76.CosmicTheme.Dark/v2/accent` **absent** on this machine; `…/Dark/v1/accent` present — quoted (v1 encoding):

  ```
  base: (red: 0.9058823, green: 0.6117647, blue: 0.99607843, alpha: 1.0),  … (purple)
  ```

- System: `/usr/share/cosmic/com.system76.CosmicTheme.Dark/v2/accent` — quoted (v2 encoding, the *current* serialization):

  ```
  (
      base: "#63D0DFFF",
      hover: "#63BAC6FF",
      …
      disabled_border: "#63D0DF80",
  )
  ```

- Corrections to record #1 §5.1, stated here because records are never edited: the `Component` struct has **twelve** fields (`base, hover, pressed, selected, selected_text, focus, divider, on, disabled, on_disabled, border, disabled_border` — `cosmic-theme/src/model/color.rs`), not four; the four it named do exist and `base` remains the read target. And the encoding **changed with v2**: v2 files are `#RRGGBBAA` hex strings (alpha included — note `…80` = 50 %), v1 files f32-struct tuples. A follow must parse both. Same teal in both encodings on the system side (`0.3882353 ≈ 0x63`), i.e. an encoding change, not a value change.
- **Per-polarity accents are real**: user `…/Light/v2/accent` exists with `base: "#00525AFF"` — this desktop runs purple-on-dark, dark-teal-on-light. §9 Q4.

### 3.2 Style → `Theme.{Dark,Light}/v2/corner_radii`

- User: `…/Dark/v2/corner_radii` — quoted (this machine, **Square**):

  ```
  (
      radius_0: (0.0, 0.0, 0.0, 0.0),
      radius_xs: (2.0, 2.0, 2.0, 2.0),
      radius_s: (2.0, 2.0, 2.0, 2.0),
      radius_m: (2.0, 2.0, 2.0, 2.0),
      radius_l: (2.0, 2.0, 2.0, 2.0),
      radius_xl: (2.0, 2.0, 2.0, 2.0),
  )
  ```

- System: `/usr/share/…/Dark/v2/corner_radii` — Round default `0 / 4 / 8 / 16 / 32 / 160`. (User v1 file holds the same Round values — pre-Square leftover, now shadowed.)
- The selector: cosmic-settings `pages/desktop/appearance/style.rs` — three image buttons (`fl!("style", "round" | "slightly-round" | "square")`) sending `Message::Roundness`, handled in `custom.rs` (`Message::Roundness(r) … set_corner_radii(r.into())`). Presets and the reverse classifier in `cosmic-theme/src/model/corner.rs` (§5).
- Parse: six 4-tuples of f32; the tuples are per-corner (TL/TR/BR/BL) but always uniform in every preset. Easy.

### 3.3 Density — `CosmicTk/v1/interface_density` (+ theme `spacing`)

- User: `~/.config/cosmic/com.system76.CosmicTk/v1/` is **empty** → default. System: `/usr/share/…/CosmicTk/v1/` ships only `apply_theme_global` — so the density default is code-side (`CosmicTk::default().interface_density = Density::Standard`, libcosmic `src/config/mod.rs`), reached when both files miss.
- RON shape when set: the enum variant alone, e.g. `Standard` / `Compact` / `Spacious` (an externally-tagged unit enum serializes as its variant name — same bare-word form as `active_hint: false` and the `window_hint: None` file). Enum: `cosmic-theme/src/model/spacing.rs` `Density { Compact, Standard(#[default]), Spacious }`.
- **The write fan-out** (cosmic-settings `appearance/mod.rs` `Message::Density`): (1) `theme_manager.set_spacing(density.into())` — the whole expanded `Spacing` preset into **both** polarities' theme `spacing` key; (2) `config.set("interface_density", density)` + `set("header_size", density)` into CosmicTk; (3) `update_panel_spacing` — rewrites panel spacing too. So the cheap flag is `interface_density`; the expanded truth is theme `spacing` (quoted Standard preset: `space_none…space_xxxl = 0,4,8,12,16,24,32,48,64,128`).
- The UI labels (installed binary fl keys): `interface-density`, `compact`, `comfortable` — "Comfortable" is the label for `Density::Standard`; "spacious" exists in the enum strings. The task brief's "comfortable" is the UI word, not the store word.
- Parse: bare word; three acceptable values; anything else = miss → Standard.

### 3.4 Font — the store that moved two days after this build

- **Installed vintage (built 2026-10-20)**: `com.system76.CosmicTk/v1/interface_font` (+ `monospace_font`) — binary strings prove the compiled-in field list (`icon_theme header_size show_maximize show_minimize interface_font monospace_font interface_density apply_theme_global`, `com.system76.CosmicTk`). **No such file exists on this machine** (user or system) → the default is code-side: `SANS_FAMILY_DEFAULT = "Open Sans"`, `MONO_FAMILY_DEFAULT = "Noto Sans Mono"` (removed-constant diff, libcosmic commit `4c853eba` 2026-10-22 "refactor: move font and icon from Tk to Theme").
- **Master (since 2026-10-22)**: `Theme.{Dark,Light}/v2/font` + `mono_font` — cosmic-settings `appearance/font_config.rs`: `const FONT: &str = "font"; const MONO_FONT: &str = "mono_font";`, `update_theme` → `Theme::dark_config().set(FONT, FontConfig{…})` (family + `Weight::Normal`/`Style::Normal`/`Stretch::Normal`), then syncs the theme bundle. `FontConfig` (`cosmic-theme/src/model/font.rs`): `{ family: String, weight, stretch, style }`, `#[serde(default)]`, `DEFAULT_FONT_FAMILY = "Open Sans"`.
- Shape on disk: **unquotable locally** — no file exists; the struct above is source-verified, the RON form of the iced `weight/stretch/style` values is not (§10). The pragmatic parse is the `family: "…"` line and nothing else.
- Open Sans is not just the constant: the installed binary embeds the Open Sans faces (full name tables in `strings`), so the default face renders even with no font-store file.
- Divergence cost, stated: a follow that wants both vintages reads `CosmicTk/v1/interface_font` **and** `Theme.<polarity>/v2/font` (either-or; both default to Open Sans). §9 Q5.

### 3.5 Gaps and active-window-hint — out of scope, recorded

- **Gaps**: theme `v2/gaps`, quoted `(0, 8)` on system and user v1 — `Theme` fields doc it as "cosmic-comp window gaps size (outer, inner)" (`theme.rs` field comment); the Appearance page's `Message::GapSize` writes it via `set_gap_size` (outer = the slider, inner floored at `active_hint`). cosmic-comp consumes the **theme** store (`cosmic-comp/src/theme.rs` subscribes `Theme::dark_config()/light_config()`); its own `com.system76.CosmicComp` v1 config carries no gaps key on this vintage (user `workspaces` file quoted: `workspace_mode/layout/action_on_typing/workspace_wraparound` only).
- **Active-window hint**: the enable toggle is `com.system76.CosmicComp/v1/active_hint`, a bare bool (user file quoted: `false`; cosmic-comp `src/config/mod.rs` `"active_hint" => get_config::<bool>`; set from the Window Management page — `active-window-hint` fl key). The width and color live in the theme: `v2/active_hint` u32 (default `3`, user v1 file quoted `3`) and `v2/window_hint` `Option<Srgb>` (user v1 file quoted `None`; hex-string form in v2 like accent). No honest mapping to a switcher card — dropped per the brief.

---

## 4. Read-cost model — what "follow" costs per gesture open

Faithful ladder reads for all four mappings, on top of `theme.rs`'s existing one (≤2 reads):

| Follow | Reads (worst case) |
|---|---|
| is_dark (existing) | 2 (user, system) |
| accent | 3 (user v2, user v1, system v2) ×1 polarity |
| corner_radii | 3 (user v2, user v1, system v2) |
| interface_density | 2 (user, system) |
| font | 2 per store considered (§3.4) — 2–4 |

**≈ 12–14 path touches, each a stat + ≤512-byte RON read** (largest here: corner_radii at ~200 B, accent ~450 B). Measured ceiling on this machine (shell loop, one `dd` spawn per file — process-spawn-dominated, so a hard upper bound): **15 ms per full pass cold-ish, ~11 ms warm ×100**. A daemon doing `std::fs::read` in-process pays page-cache microseconds per file — orders of magnitude inside the <50 ms warm-press budget, consistent with TH OQ 3's per-invoke ratification ("a watch/subscription would only return if per-invoke proves measurable on the gesture path"). Staleness semantics unchanged and inherited: read at session open, frozen mid-gesture, fresh next gesture (docs/settings.md's live-apply rule); also covers scheduled dark/light flips (`Mode/v1/auto_switch`, `false` here) at the next open. Misses are `None` → the Airlock default (theme.rs's never-fatal pattern); a follow owes one aggregated diagnostic per open, not per store.

---

## 5. Style → radius: source values vs Airlock's pin

`cosmic-theme/src/model/corner.rs`, `impl From<Roundness> for CornerRadii` (master; the two presets present on this machine's disk match exactly):

| Style | radius_0/xs/s/m/l/xl | Reverse classifier |
|---|---|---|
| Round (default) | 0 / 4 / **8** / 16 / 32 / 160 | `radius_m ≈ 16` |
| SlightlyRound | 0 / 2 / **8** / 8 / 8 / 8 | `radius_m ≈ 8` |
| Square | 0 / 2 / **2** / 2 / 2 / 2 | else |

Airlock's pin: `RADIUS = 12` = `radius_s + 4` in the Round default (`src/ui.rs:107-111`, cosmic-launcher's card formula, record #2 §2.6), constrained by `PAD_Y ≥ RADIUS` (`src/ui.rs:99-105` — 16 vs 12 today, "real slack"; 16 would sit at exact equality again, the condition that comment exists to avoid). Candidate mappings (spec decides):

| Strategy | Round | SlightlyRound | Square | Note |
|---|---|---|---|---|
| `radius_s + 4` (today's formula, followed) | 12 | 12 | 6 | keeps the launcher precedent; Square = 6 is off Airlock's 8-grid (BAND_RADIUS is 8) |
| `radius_s` | 8 | 8 | 2 | band already rounds at 8 |
| Judged ladder on the 8-grid | 12 | 8 | 0–2 | preview-sheet work either way |

Also in play: `BAND_RADIUS = 8` (`src/ui.rs:93`) is its own pin; a follow could move card and band together or card only. This machine today would paint the Square column — the user's desktop is Square.

## 6. Density → row rhythm: what the three densities actually change

`impl From<Density> for Spacing` (`spacing.rs`) — the relevant small tokens:

| Token | Compact | Standard | Spacious |
|---|---|---|---|
| space_xxs | 4 | **8** | 12 |
| space_xs | 8 | 12 | 16 |
| space_s | 16 | **16** | 24 |
| space_m | 16 | 24 | 32 |

Airlock's rhythm anchors (`src/ui.rs:74-105`): `ROW_PAD_Y = 8`, `ROW_GAP = 8` (= Standard's `space_xxs`), `PAD_Y = 16` (documented as 2 × the row gap, = Standard's `space_s`), `PAD_X = 20`, `ROW_H = 48` (icon + 2×8). Candidate deltas — one spacing notch per step (compact ≈ ×2/3, spacious ≈ ×4/3, matching COSMIC's ratios):

| Density | ROW_PAD_Y / ROW_GAP / PAD_Y | ROW_H | card_height(10 slots) | worst case 12 slots |
|---|---|---|---|---|
| Compact | 4 / 4 / 8 | 40 | 452 | 540 |
| Standard (today) | 8 / 8 / 16 | 48 | 584 | 696 |
| Spacious | 12 / 12 / 24 | 56 | 716 | **852** |

Flags for the spec: **Spacious + ROWS_MAX breaks the recorded 720-logical-px fit** (716 at the default budget clears it by 4 px; the 12-slot worst case doesn't) — density must couple with the rows budget or clamp; Compact's 4 is a half-grid step (`space_xxxs` in COSMIC, but Airlock's grid vocabulary is 8); `PAD_X` and the selection-band inset have no density analogue in COSMIC's model and would stay pinned. Density exists on the installed vintage (§3.3) — no comfortable-fallback needed on this machine.

## 7. Font: family, size, and the spec question

The store answers "what is the system sans" concretely: **`Open Sans`** on both the installed vintage and master (default constant; absent store file). Size is **not in the store** — `FontConfig` carries family/weight/stretch/style only; COSMIC's text sizing comes from each toolkit's type scale, so there is no desktop "interface font size" to follow. Consequences for SPEC-typography's "default flips to the system sans": following the store's `family` (with the 12-px pin kept, and `Regular` ≈ `Weight::Normal` matching cosmic-settings' own write) is the honest COSMIC-conformant reading; a size follow would have no store to read and would re-open the 11-07 12-px pin with nothing to anchor to. §9 Q5 also carries the vintage split (Tk vs Theme key) and the parse caveat.

## 8. Accent × custom palette (surface, don't decide)

If the band becomes accent-tinted (re-opening record #1 §5's rejected-for-now verdict and the 2026-11-04 hard line), the blend question lands on SPEC-palette's `Palette::custom`: today `selected_bg = blend_over(card, text, 20)` (`src/ui.rs:192,207-217`) — the band would become `blend_over(card, accent_base, ~20)`. Open semantics: accent arrives as `#RRGGBBAA` (alpha byte present — honored, rejected, or forced 100?); the blend is the same straight-sRGB mix `blend_over` already does (no linear-light correction anywhere in the system); contrast resets from the measured 1.84/1.49:1 neutrals to wallpaper- and accent-dependent values; and per-polarity accents (§3.1) mean the band color can flip with `is_dark` independently of the card (a custom card pins one card but the accent store still has two sides). These are §9 questions, not recommendations.

---

## 9. Questions for the revised specs

1. **Radius mapping row** (§5): which strategy, and does `BAND_RADIUS` follow the card or stay 8? Does a followed radius ever exceed 12 (PAD_Y slack rule)?
2. **Density coupling** (§6): which candidate ladder; do `PAD_X`/band inset stay pinned; and how is the Spacious × ROWS_MAX overflow resolved (clamp rows, clamp density, or accept per-output)?
3. **Density source**: read the cheap `interface_density` flag or the expanded theme `spacing` (authoritative if anything else ever writes spacing)?
4. **Accent polarity**: which side of the per-polarity accent store feeds the band — the side matching the *effective* card (theme/auto/custom), and what happens when a custom card's luminance disagrees with the store polarity the accent came from?
5. **Font vintage split** (§3.4): read `CosmicTk/v1/interface_font`, `Theme.<polarity>/v2/font`, or both-first-wins? And confirm family-only with the 12-px pin (the store offers no size to follow).
6. **Hex-alpha policy** for accent `#RRGGBBAA` (and any followed color): honor, reject-with-diagnostic (the 6-digit-only house style, SPEC-palette), or force opaque?
7. **Aggregate the miss diagnostic**: one line per open naming every store that missed (theme.rs precedent is one line per total miss), or silent per-store `None`?
8. **v1 fallback depth**: does the hand-read implement the full user-v2 → user-v1 → system-v2 ladder (faithful; the purple on this machine depends on it) or user-v2 → system-v2 only (simpler, loses v1-era user tweaks)?

## 10. Gaps and honest silences

- **No font store file exists on any reachable machine here** — the `FontConfig` disk shape (RON form of iced `weight/stretch/style`) is unverified; no sample could be quoted. The `family:`-line parse sidesteps it, but the shape claim is source-only.
- **The installed libcosmic commit is unknowable** (vendored into the cosmic-settings build; no version string beyond `1.11.0`). Divergence is bounded by binary strings (frosted keys, `interface_density`, CosmicTk `interface_font` compiled in) and the refactor commit date (2026-10-22) vs the build epoch (2026-10-20); exact intermediate states are not claimed.
- **`icon_theme` moved out of CosmicTk in the same 2026-10-22 refactor** — its new home on master was not chased (out of scope; no mapping ratified for icons).
- **Panel spacing rewrite on density change** (`update_panel_spacing`) was noted but not traced into `cosmic-panel-config` — irrelevant to a switcher, recorded as a silence.
- **Whether the theme-bundle machinery** (`cosmic_theme_bundle_config`, visible in master's font/style write paths) rewrites these keys behind cosmic-settings' back was not verified; on this machine the ladder explains every observed file.
- **Roundness preset history**: SlightlyRound's values are master's; no SlightlyRound file exists locally to cross-check the installed vintage against (only Round-default and Square files present).
- **Master's fl key list** for density ("spacious") was inferred from enum strings and the installed `compact`/`comfortable` keys; the master `.ftl` files were not fetched.
