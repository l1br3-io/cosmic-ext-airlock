# Appearance settings — what knobs are worth adding — research findings

**Date:** November 7, 2026 · **Author:** research agent for jpt · **Status:** Complete — ready to feed the appearance spec (verdict table in §1; the owner decides via the spec gate, per the theming/typography precedent)

**Scope:** what an "Appearance" settings surface could hold for Airlock's overlay: card opacity/translucency, compositor blur behind the card, custom card/text colors, the COSMIC accent, contrast guardrails for user colors, and the smaller geometry knobs peers expose. Companion records: [switcher-ux-and-icons.md](switcher-ux-and-icons.md) (settled the near-opaque dark card, 1 px hairline, fg@20 % selection, ≤600 px territory, no fade — §1 TL;DR #3/#13; everything that record pinned is cited here, not re-researched) and the `typography` spec's font knob (drafted, unapproved — this record touches no typography question).

**Method:** primary sources only — upstream source code read over the GitHub API (`gh api`, `master`/`main` blobs), the wayland-protocols XML on gitlab.freedesktop.org, the W3C WCAG 2.2 recommendation, and **the local COSMIC install's own config stores** (`/usr/share/cosmic` defaults and `~/.config/cosmic`, read 2026-11-07 — the switcher-ux record's precedent for local evidence). Remote code is cited by repo/file + constant or function name (line numbers were not stable across fetches); Airlock claims carry `file:line` refs into this repository. Fetched 2026-11-07. Contrast ratios in §6 were computed with the WCAG relative-luminance formula against the palette values as they exist in `src/ui.rs` today.

---

## 1. TL;DR — verdicts per knob

| # | Knob | Verdict | One-line cause | Source |
|---|------|---------|----------------|--------|
| 1 | **Card opacity** (a free alpha/transparency knob) | **Reject** | Every peer exposes one (panel `opacity` 0.8 default, fuzzel per-color alpha, dunst `transparency`) but the failure mode — the card dissolving against arbitrary wallpapers, with no compositor guarantee — is the exact failure the near-opaque assert exists to prevent; overturning it is a design-contract re-decision, not a settings addition | §3.1–3.3 |
| 2 | **Compositor blur behind the card** (frosted glass) | **Reject, record the path** | It is actually reachable — cosmic-comp implements `ext-background-effect-v1` blur — but the protocol is testing-phase ("subject to compositor policies"), cctk doesn't ship it, blur is `airlock-design` tell #1, and every pixel of it is compositor cost the zero-mapped-time rule doesn't need | §3.4 |
| 3 | **Custom card + text colors** (two free colors, derived blends kept) | **Incorporate with guardrails** | The `Palette` contract is already "two chosen colors, three computed ones" (src/ui.rs:231-249); free `RRGGBBAA` colors are the universal peer surface (fuzzel, rofi, dunst); and COSMIC itself derives component/divider colors from base+on — the two-color derivation pattern is a shipped, platform-native contract | §4 |
| 4 | **Accent color** in the selection band (or as a derived tint) | **Reject; trivially cheap to offer if re-decided** | The read path is proven by Airlock's own `theme.rs` pattern (one RON bool there; the accent is one RON file there — `Theme.Dark/v1/accent`, base/hover/pressed/selected RGBA); but the "no accent anywhere" hard line is fresh (2026-11-04) and the neutral-lift selection was the research-judged choice — re-opening is the owner's call, not a settings gap | §5 |
| 5 | **Theme presets beyond dark/light** (named palettes, "Nord card" etc.) | **Reject** | No surveyed peer ships presets for a switcher; COSMIC's own "presets" are exactly dark/light + one accent; built-in preset palettes are the furnishing the skill forbids — two free colors already cover the same want | §5.3 |
| 6 | **Contrast guardrail on user colors** | **Incorporate with #3** (floor value is an owner decision) | WCAG 2.2's AA floor for 12-px text is 4.5:1; no peer validates anything (rofi silently ignores bad values); Airlock's derived-blend system makes the guardrail almost free — the honest data: title contrast is 14.6:1 on both polarities today, and the *shipped light muted hint is already 3.60:1*, below AA | §6 |
| 7 | **Border width / corner radius / shadow / per-row hint colors** | **Reject** | Geometry is pinned by the 8-grid + preview-frozen constants (`airlock-design` tokens-before-values); peers expose radius (fuzzel `-r` 10, rofi `border-radius`) but Airlock's 12 px and 1 px border were judged on the sheet twice; shadow is impossible without soft edges on this renderer | §7 |
| 8 | **Wallpaper sampling / backdrop dimming** | **Reject — nothing changed** | The switcher-ux verdict stands (thumbnails out of reach for external clients); the one new protocol in this area (`ext-background-effect-v1`) carries no dim capability at all — its only capability is blur | §3.4 |

---

## 2. What each verdict re-opens (dated re-decisions owed)

Per the project's product-truth precedent (dated, explicit, never silent — the `typography` spec's re-decision section is the shape), an appearance spec built on this record must state:

- **The near-opaque invariant.** `assert!(COLOR_BACKGROUND >> 24 >= 0xFA)` at src/ui.rs:221-224, plus the `airlock-design` line "Near-opaque is a contract, not a taste" (`.agents/skills/airlock-design/SKILL.md`, "Tokens before values") and the theming spec's "alpha/opacity knobs" hard line (SPEC-theming.md, out of scope). Verdicts 1–2 reject re-opening it **today**; if the owner re-opens it anyway, the re-decision lands in the design skill, README, and PRODUCT.md with the date and the cause, like the 11-07 font re-pin did.
- **The "no accent color anywhere" hard line.** Written 2026-11-04 (SPEC-theming.md hard lines; ui.rs:90 "no accent", ui.rs:191 "borrowing the user's accent color"). Verdict 4 keeps it; an owner note is the only thing that re-opens it.
- **The one-font, 12-px austerity** is *not* touched by this record (the `typography` spec owns it); the font knob is appearance-adjacent and its re-decisions are already drafted there.
- **The theming spec's shape.** A custom-colors module would extend `theming` (the `Palette` struct, `ThemeSetting`, the last-good fallback contract) — it does not replace it; `theme = "auto" | "dark" | "light"` stays the polarity story and any custom color rides beside it (§9 has the interaction questions).

---

## 3. Card opacity / translucency

### 3.1 How COSMIC's own panel does it

From [pop-os/cosmic-panel](https://github.com/pop-os/cosmic-panel) `cosmic-panel-config/src/panel_config.rs` (master, fetched 2026-11-07):

- `CosmicPanelConfig` carries `/// opacity of the panel — pub opacity: f32` (field `opacity`), with a **default of `0.8`** (the `Default` impl's `opacity: 0.8`). There is no range clamp anywhere in the config crate — the f32 is free.
- Background choices are the enum `CosmicPanelBackground`: `ThemeDefault` ("theme default color with optional transparency"), `Dark`, `Light`, or `Color([f32; 3])` (RGB only — no per-color alpha). `bg_color_override()` returns `Some([r, g, b, self.opacity])` — i.e. **the opacity knob multiplies the theme color's alpha**; `ThemeDefault` + 0.8 is the platform's stock translucent panel.
- Even COSMIC itself force-opaques in a mode: `maximize()` sets `self.opacity = 1.0` before flattening the margins — the panel's "clean" look is opaque.
- Rendering (`cosmic-panel-bin/src/iced/elements/background.rs`, `Background::view`): the fill is `Color::from(color)` straight into an iced container style — the iced/wlr compositor blends it — and there is an explicit degradation comment: **"no border when panel is fully transparent"** (`if color[3] < 0.01 { PanelBorder::None }`). Transparency has a cost even in COSMIC's own renderer: the hairline that separates the panel from the wallpaper is dropped when the panel goes invisible.
- The **frosted** story (blur behind translucent COSMIC surfaces) lives in the theme, not the panel config: the local theme stores carry a bool `is_frosted` (`/usr/share/cosmic/com.system76.CosmicTheme.Dark/v1/is_frosted` → `false` on this machine, read 2026-11-07), and libcosmic's theme model ([pop-os/libcosmic](https://github.com/pop-os/libcosmic) `cosmic-theme/src/model/theme.rs`, fields `frosted`, `frosted_panel`, `frosted_windows`, `frosted_system_interface`, `frosted_applets`, `frosted_maximized_apps`) carries them with a `BlurStrength` enum — fourteen steps, default `Medium`, documented *"Actual blur radius is decided by cosmic-comp."* COSMIC's design answer to translucent surfaces is **blur provided by the compositor**, not raw alpha alone.

### 3.2 How peers expose it

- **fuzzel** ([dnkl/fuzzel](https://codeberg.org/dnkl/fuzzel), `doc/fuzzel.ini.5.scd`, fetched 2026-11-07): *"All color values are in RGBA"* — 8-digit `RRGGBBAA`, so transparency is a property of each color, background included (default `fdf6e3ff`, fully opaque alpha `ff`). The current `fuzzel.1.scd` has **no `--opacity` option**; per-color alpha is the mechanism. A user who wants translucency writes `background=1e2126aa` — free-form, unvalidated.
- **dunst** ([dunst-project/dunst](https://github.com/dunst-project/dunst), `docs/dunst.5.pod`, fetched 2026-11-07): `transparency` is documented as **"(default: 0) (X11 only)"**, 0–100 — and the doc's own Wayland guidance is: *"To make windows transparent on wayland, set the transparent part of a color"* (colors accept `#RRGGBBAA`). Same conclusion, one abstraction down.
- **rofi** ([davatorium/rofi](https://github.com/davatorium/rofi), `next`, `doc/rofi-theme.5.markdown`): full CSS-style color syntax including `rgba(r,g,b,a)` and 8-hex `rrggbbaa` on `background`/`border-color`; invalid values are silently *"ignored"* — no clamping, no warnings, no contrast checks anywhere in the doc.

**Pattern:** opacity is universally *free-form and unvalidated* in this peer set. None of them documents or guards against the failure mode; the failure mode is simply the user's wallpaper problem. (The one degradation any of them ships is cosmic-panel's dropped border above.)

### 3.3 Why Airlock's near-opaque line stands

- The failure mode is recorded in Airlock's own artifacts, with causes: the border "dissolve" episodes (ui.rs:194-199 — two preview rounds strengthened the dark border to 18 % *because anything subtler dissolves against dark wallpapers*; ui.rs:272-278 — the light border needed 22 % for the same reason, the card being only ~12 RGB points from a white backdrop); the compile-time assert's own comment (ui.rs:219-220, *"background bleed through the overlay must stay imperceptible on light and dark wallpapers alike"*); the design skill's tell #1 (*"The glass card … Translucency on a hand-composited shm surface means bleed"*); and switcher-ux TL;DR #13 ("keep the near-opaque card").
- Airlock's titles must survive a **200 ms peripheral glance against whatever wallpaper the user has** — that is the product's whole design premise (`airlock-design`, "Ground the design in the subject"). A card at 0.8 opacity fails that premise on busy wallpapers; at 0.5 it fails on most. The peers that can afford translucency are launchers/notifications that users read with attention, or (COSMIC, alt-tab-macos) pair it with compositor blur.
- **The one honest concession to peer behavior:** a knob isn't needed to get translucency — Airlock's buffer is ARGB8888 and the compositor already blends it (§8.1), so translucency would be a one-constant change, not plumbing. The rejection is entirely a design-contract decision, which is exactly why it must be re-decided with a date if it changes.

### 3.4 Compositor blur: the one feasible path, recorded so it isn't re-researched

- **The protocol exists and cosmic-comp implements it.** [`ext-background-effect-v1`](https://gitlab.freedesktop.org/wayland/wayland-protocols/-/raw/main/staging/ext-background-effect/ext-background-effect-v1.xml) (staging/testing phase, copyright 2024 Xaver Hugl): it provides *"a way to improve visuals of translucent surfaces by applying effects like blur to the background behind them"*, takes any `wl_surface`, and negotiates a `capabilities` event whose sole entry is `blur`. Two load-bearing caveats in the XML: *"The blur algorithm is subject to compositor policies"* and *"when the capability goes away, the corresponding effect is no longer applied by the compositor, even if it was set before."* cosmic-comp wires it in `src/wayland/handlers/background_effect.rs` (`ExtBackgroundEffectHandler for State`, `Capability::Blur`, `set_blur_region`), with the shader machinery in `src/backend/render/wayland/blur_effect.rs` and the down/up-sample fragments (`MAX_STEPS = 15`, noise 0.03).
- **Airlock's toolkit doesn't ship it.** The tree listing of [pop-os/cosmic-client-toolkit](https://github.com/pop-os/cosmic-client-toolkit) (`master`) contains no background/blur/effect module — Airlock pins `cosmic-client-toolkit = "0.2.0"` (Cargo.toml:33) — so support would mean hand-writing the protocol binding, as `theme.rs`-style minimalism but at protocol scale. Also unknown: whether the *installed* compositor exposes the global (this machine runs `cosmic-comp 0.1~1792508925~24.04~8cb1802`; the running compositor's globals could not be listed here — no `wayland-info` on the host).
- **The design verdict:** blur is the `airlock-design` glass-card tell verbatim, the protocol promises no particular algorithm, and a switcher that must map in one frame gains nothing from paying compositor blur on every selection step. **Reject — and record this section as the citation for a future re-open**, so the answer stays one file away.

---

## 4. Custom card and text colors

### 4.1 How peers present color config

- **fuzzel**: `[colors]` in `fuzzel.ini` — `background`, `text`, `selection`, `selection-text`, `border`, `match`, `counter`, `placeholder`, all `RRGGBBAA` (§3.2). Flat list, one file, no derived anything.
- **rofi**: rasi `theme {}` blocks; any property can be themed; again fully flat and free.
- **alt-tab-macos**: notably, its settings UI does **not** expose free color pickers. `src/preferences/Preferences.swift` (master) carries only `appearanceStyle` / `appearanceSize` / `appearanceTheme` (system/dark/light) plus toggles; the "Customize StyleSheet" sheet (`src/preferences/settings-window/tabs/appearance/CustomizeStyleSheet.swift`) ships toggles and illustrations, not color editors. The panel color itself is `NSVisualEffectView` material presets (ultraDark default — switcher-ux §2.4). The most elaborate appearance settings UI in the peer set stops at **presets + toggles**; it does not ask users to mix colors.
- **COSMIC**: colors are not per-app settings at all — they are *theme* values owned by cosmic-settings (§5.1), with apps reading the derived theme. COSMIC's philosophy: users pick one accent, the system derives the rest.

### 4.2 The two-color derivation contract — and that it's a shipped pattern

- Airlock's `Palette` (src/ui.rs:231-252) is **"two chosen colors, three computed ones, and the polarity itself"** — `selected_bg` = text @ 20 % over card, `border` = text @ 18 %/22 % by polarity, `muted` = text @ 55 %, all through the `blend_over` mix (src/ui.rs:207-217). A custom-colors feature is therefore *not* a new system: it is pointing the same two chosen colors at user input and letting the derivations run.
- **COSMIC itself derives exactly this way.** libcosmic `cosmic-theme/src/model/derivation.rs`, `Container::new`: `let divider_c = on.with_alpha(if is_high_contrast { 0.5 } else { 0.2 });` — the container's divider is the *on* color at 20 % alpha (50 % in high-contrast), and `small_widget.alpha = 0.25` is derived too. Airlock's border = text @ 18 % is the same family of derivation (and COSMIC's own house rule is 20 % — the two preview rounds that moved Airlock to 18 %/22 % are documented at ui.rs:194-199/272-278).
- **The answer to "does the derived-color contract hold up user-facing":** yes as a *mechanism* (it is COSMIC's own), with the honest caveat that peers using flat color lists (fuzzel/rofi) give users direct control of the selection and border colors instead — a user who wants a loud green selection band cannot express it through Airlock's two knobs. That limitation is the price of never letting a palette drift from the design system (the theming spec's stated rationale, SPEC-theming.md "Code Style").

### 4.3 Guardrails the evidence supports

1. **Alpha: pin or clamp, never free.** A user-chosen card color with a low alpha byte reintroduces the §3.3 failure mode through the same door. Options (spec decides): reject alpha-bearing values with a named diagnostic, clamp to ≥ 0xFA, or parse 6-digit-only hex. The theme-mode key precedent (`theme` rejects `"blue"` with a named problem, src/settings.rs:151-155) says *named rejection over silent clamping* is this project's house style.
2. **Polarity must follow the colors.** `is_dark` drives the symbolic-icon tint cache (src/ui.rs:248-251) — a custom palette has to declare or derive it (relative luminance of the chosen card color against a threshold), or the icon cache tints wrongly. Deriving is one function; declaring is one more key. (§9, Q4.)
3. **Contrast: surface it, don't police it (§6).** No peer validates; Airlock's guardrail should at minimum *diagnose*, not silently accept, a text color under the AA floor against its card.

---

## 5. Accent color and color-scheme presets

### 5.1 Where COSMIC's accent lives, and how cheap it is to read

- **On disk, verified locally (2026-11-07):** the accent is one RON file per polarity — `~/.config/cosmic/com.system76.CosmicTheme.Dark/v1/accent` (and `.../CosmicTheme.Light/...`), with the system default under `/usr/share/cosmic/com.system76.CosmicTheme.Dark/v1/accent`. Shape: a struct of four f32-RGBA fields — `base`, `hover`, `pressed`, `selected` — e.g. the system default dark accent `base: (red: 0.3882353, green: 0.8156863, blue: 0.8745098, alpha: 1.0)` (teal); this machine's user accent is a purple. The `Component` type behind it is libcosmic's `accent: Component` (`cosmic-theme/src/model/theme.rs`).
- **The read path is Airlock's own proven pattern.** `src/theme.rs:22-31` already answers "does the system prefer dark?" by hand-reading `com.system76.CosmicTheme.Mode/v1/is_dark` with user→system fallback and `None`-on-any-miss (deliberate: `cosmic-config` was rejected as a git-only tree for one boolean — TH3, ratified 2026-11-04). Reading `Theme.<polarity>/v1/accent`'s `base` field is the same one-file read with a tiny RON struct parse. **No dependency needed.**
- **What the accent would mean for Airlock:** the selection band is currently text @ 20 % over the card (src/ui.rs:192) — "a neutral lift that reads on both wallpaper polarities without borrowing the user's accent color." The studied switchers mostly *do* borrow it: GNOME fills with foreground @ 20 %, alt-tab-macos with accent @ 0.2, cosmic-launcher styles the focused row as the hovered Text-button (theme-colored) — switcher-ux §2.10 table. Airlock's neutral-lift choice was deliberate (theming spec, "no accent") and re-judged as recently as the 11-04 approval.

### 5.2 Verdict reasoning

The cost side is near zero and the platform-conformity benefit is real — but the hard line ("Never: introduce an accent color into the selection band — the neutral-lift decision stands", SPEC-theming.md; "no accent, no glow", ui.rs:90-93; `airlock-design` tell #6) is nine days old and was *the owner's* ratified decision, not a gap left by accident. The research position is therefore: **technically feasible and cheap; the rejection stands until the owner re-decides it**; §9 Q5 frames the choice precisely so the spec gate can settle it in one sentence.

### 5.3 Presets

No surveyed peer ships named color presets for a switcher surface: fuzzel/rofi users copy theme snippets from dotfiles, alt-tab-macos ships material presets + dark/light/system, COSMIC ships dark/light + one accent. A built-in preset library would be furnishing (the skill's own warning against "spending freedom on a default"); two free colors + the two polarities already express what presets would.

---

## 6. Contrast guardrails (WCAG 2.2, with Airlock's own numbers)

### 6.1 The standard

- **SC 1.4.3 Contrast (Minimum), Level AA** (w3.org/TR/WCAG22/#contrast-minimum): text needs **≥ 4.5:1**; *"large-scale text"* ≥ 3:1 (large = ≥ 24 CSS px, or ≥ 18.66 CSS px bold — Airlock's 12-px titles qualify for neither). Logotypes and incidental text are exempt.
- **SC 1.4.11 Non-text Contrast, Level AA**: visual information *required to identify UI components and states* needs **≥ 3:1** against adjacent colors. The selection band is a state indicator; the card border separates the card from the backdrop.

### 6.2 Airlock's palettes measured (WCAG formula, computed 2026-11-07 from src/ui.rs values)

| Pair | Dark card | Light card |
|------|-----------|------------|
| Title text vs card (12-px → 4.5:1 floor) | **14.66 : 1** | **14.73 : 1** |
| Title text over the selection band | 7.95 : 1 | 9.88 : 1 |
| Muted hints (text @ 55 % over card) | **5.39 : 1** | **3.60 : 1** — below AA |
| Selection band vs card (state → 3:1 floor) | 1.84 : 1 | 1.49 : 1 — below |
| Border vs card | 1.73 : 1 | 1.56 : 1 |
| Card vs extreme backdrop (black / white) | 1.30 : 1 | 1.10 : 1 |

Reading, honestly:

- **Text is never the problem on the derived palettes** — 14.6:1 is ~3× the floor; custom colors are where a floor bites.
- **The shipped light-card muted hint is already below AA** (3.60:1). The 55 % derivation was preview-frozen 2026-11-04 as the mid-band of a judged 50–70 range, "tightest read was muted-over-selection on the light card" (src/ui.rs:243-249) — a judged-value trade-off, not an oversight, but it means a contrast guardrail for *user* colors would sit right of a value Airlock itself ships. A spec that adds the floor owes a note on its own muted derivation (bump to ~62 % on light would clear 4.5:1; a re-judge on the sheet).
- **The selection band and border are below 3:1 by design** — and so is every peer's: GNOME's fg @ 20 % fill computes to the same sub-2:1 territory, alt-tab's accent @ 0.2 likewise. WCAG's 1.4.11 exception ("when the user agent determines the appearance") doesn't formally cover this, but the cross-platform consensus (switcher-ux §2.10) is tinted fills at exactly these strengths; the selection state is redundantly identified by the hint keys and the wheel/click motion. A guardrail should therefore target **text** (4.5:1) and treat band/border as judged design values, like the peers do.
- **Card vs backdrop can never be guaranteed** against an arbitrary wallpaper (1.10:1 against pure white is the light card's honest worst case) — this is the standing argument for the near-opaque card plus the hairline (ui.rs:194-199), and it is independent of any settings knob.

### 6.3 What peers do (nothing) — and what Airlock's cheap guardrail is

rofi: invalid values silently "ignored"; fuzzel and dunst: no validation of any kind documented. No peer checks contrast. Airlock's differentiator is that it *derives*: because `selected_bg`/`border`/`muted` are computed from the two chosen colors, a guardrail only has to examine one pair (card vs text) at palette-resolution time — a pure function, testable in `#[cfg(test)]` with the same machinery as the existing derivation tests. Enforcement level (reject the file / clamp with diagnostic / accept with warning) is §9 Q3.

---

## 7. Other knobs peers expose (brief, only where evidence is strong)

- **Border width and color**: fuzzel `[border] width` (default 1), rofi `border`/`border-color`/`border-radius` with AA toggles. Airlock's border is width-1 pinned by assert (ui.rs:114, ui.rs:351 `assert!(BORDER == 1)`) and its strength was *the* thing two preview rounds tuned (§4.2) — a settings knob would re-open a human-judged value for no peer-validated benefit. **Reject.**
- **Corner radius**: fuzzel `-r` (default 10), cosmic-panel `border_radius_override` / `CosmicCornerRadiusLayerV1` (cosmic-comp's `src/wayland/protocols/corner_radius.rs` — note: COSMIC offers its own layer clients a corner-radius protocol). Airlock's 12 px = COSMIC `radius_s + 4` (ui.rs:107-111) with exact-coverage AA (`shape_cov`, ui.rs:834; `coverage_argb`, ui.rs:885). **Reject** — geometry is the 8-grid skill's territory.
- **Shadow**: impossible on this renderer without soft edges (`airlock-design` tell #5); cosmic-launcher's card carries `Shadow::default()` only because iced draws it. **Reject, permanently unless the renderer changes.**
- **Hint/muted color overrides**: would fragment the derived-color contract for one string per row; covered by the text-color knob in §4. **Reject.**
- **Per-workspace palettes**: hard line since theming (SPEC-theming.md); the `[[workspace]]` rules already exist as the pattern, but nothing in the peer set does per-surface theming, and the card is a two-second overlay. **Reject.**

---

## 8. Technical feasibility in Airlock's actual code

### 8.1 What the renderer already gives for free

- **The buffer is `wl_shm::Format::Argb8888`** (src/wayland.rs:1050-1054, stride `bw * 4`), drawn by `ui::render` into an owned `SlotPool` (src/wayland.rs:967-968) and committed per frame (src/wayland.rs:1043-1075). The compositor alpha-blends the whole surface over the wallpaper — that is how the rounded corners work: partial-coverage pixels are **premultiplied by coverage** so the compositor blends them against any backdrop (`coverage_argb`, ui.rs:882-888, doc: *"Partial pixels are premultiplied by coverage so the compositor blends the corner against any backdrop"*). **Translucency is a constant change, not plumbing** — the same is true of a corner-radius value, a border alpha, or any palette edit; the constraint on every knob in this record is design, not mechanism.
- **The palette is already parameterized**: `render(model, canvas, w, h, stride, scale, palette)` (src/ui.rs:623-631) takes `&Palette`, and `Palette::resolve(setting, system_prefers_dark)` (src/ui.rs:296-307) is the one decision point the daemon calls per invocation. A custom palette plugs in at `resolve` without touching render code.
- **The blend helper is the whole design system**: `blend_over(under, over, percent)` (src/ui.rs:207-217), used for `selected_bg` (20 %), `border` (18 %/22 %), `muted` (55 %) — any new derived value is one call with a new percent, and the compile-time near-opaque asserts (src/ui.rs:221-224) already gate the card's alpha.

### 8.2 The settings surface is ready for two more top-level keys

- `Settings` (src/settings.rs:50-57) carries `theme` + `rows` + `[[workspace]]`; `parse_root` (src/settings.rs:144-199) is the template — the `theme` arm (src/settings.rs:145-162) is exactly the shape a `background`/`text` hex-string pair would follow (unknown value → named `InvalidFile` problem → last-good contract; `ReadError`/`SettingsError` vocabulary at src/settings.rs:271-353; the mtime reload contract at src/settings.rs:393-433). The `font` spec's parse sketch (SPEC-typography.md "Code Style") is the second precedent for a string-valued appearance key.
- **Freeze-at-open is the existing behavior**: the palette is resolved per session open (the theming spec's live-apply story; `docs/settings.md` "Like every setting, `theme` is read when a gesture starts"). Custom colors inherit it for free.

### 8.3 The accent read, if ever ratified

`src/theme.rs` already: hand-reads `com.system76.CosmicTheme.Mode/v1/is_dark` (src/theme.rs:17, 37-48) with user→system fallback and a silent `None`. The accent file is the same store family — `com.system76.CosmicTheme.<Dark|Light>/v1/accent`, a RON struct of four f32-RGBA fields (§5.1, read from this machine). Parse cost: one small function; failure modes: identical (`None` → keep the neutral band). No new dependency, per the TH3 rationale.

---

## 9. Questions the spec will need answered (open — the owner decides)

1. **Interaction between custom colors and `theme`.** Does `background`/`text` (a) replace both polarities regardless of `theme`, (b) define the card only when `theme = "dark"`/`"light"` is set, (c) become a third value of `theme` (e.g. `theme = "custom"` + a `[[palette]]` or two top-level keys), or (d) define one polarity and leave the other derived? Each has a different fallback story (what does a custom-colors user's card do when `theme = "auto"`?).
2. **Contrast floor value and enforcement.** Is the guardrail 4.5:1 (WCAG AA for 12-px text), or a softer custom floor? Enforced how: reject the file (house style: named error, last-good), clamp, or warn-and-accept? And does the *light muted 3.60:1* finding (§6.2) get re-judged in the same change — a guardrail stricter than a shipped value needs a sentence.
3. **Alpha semantics for user colors.** 6-digit hex only? Accept-and-clamp ≥ 0xFA with a diagnostic? (§4.3.1 — the near-opaque assert means an alpha byte can't pass through silently.)
4. **Polarity for the icon tint.** Derive `is_dark` from the chosen card color's luminance (one function, one threshold — value is plan-time) or add a third key? Threshold and derivation belong to the spec, not the research.
5. **Accent: the one-sentence re-decision.** Keep the neutral-lift hard line (2026-11-04) or offer `selection = "neutral" | "accent"` / a derived accent tint? The read path costs one `theme.rs`-style function (§5.1, §8.3); the re-decision, if made, dates in `airlock-design`, README, PRODUCT.md per product-truth.
6. **If the owner re-opens opacity or blur despite §3:** what is the honest floor — e.g. an opacity knob that clamps at 0.9, or blur behind a near-opaque card (which is a no-op)? The evidence suggests any re-open lands on "blur only, never raw alpha" (COSMIC's own frosted architecture, §3.1/§3.4) — but that is a design decision, recorded here only as the shape of the question.

---

## 10. Gaps and honest silences

- **The installed compositor's protocol surface is unverified.** cosmic-comp `master` implements `ext-background-effect-v1` (§3.4), but this machine's `cosmic-comp 0.1~1792508925~24.04~8cb1802` could not be probed for the global (no `wayland-info` on the host; running any client against the dev desktop was out of scope). Any future blur work starts by verifying the global exists on the target COSMIC release.
- **cosmic-panel's opacity is not clamped at the config layer** — whether cosmic-settings' UI itself constrains the range was not verified (the settings UI source was not read); the panel config crate admits any f32.
- **fuzzel's older `--opacity` option**: current `fuzzel.1.scd` has no such option (§3.2); when it was removed, and whether a deprecated spelling still parses, was not traced — the record claims only that *current master documents per-color alpha as the mechanism*.
- **alt-tab-macos**: the settings-tab sources read (`Preferences.swift`, `CustomizeStyleSheet.swift`) show style/size/theme + toggles and no color pickers; other tabs (e.g. `AppearanceTab.swift`) were not read in full, so "no free colors anywhere in its settings" is claimed only about the sources read.
- **WCAG large-text definitions** (the 24 px / 18.66 px-bold glossary numbers) are the standard's well-known values; the fetched page carried the 1.4.3 criterion text but the large-scale glossary entry itself was truncated in the fetch — the ratios used here need no large-text exception (12 px is small text either way).
- **GNOME accent handling**: GNOME Shell's accent work (post-2024) was not surveyed; the GNOME citations here remain the switcher-ux record's fg @ 20 % selection formula.
- **wlogout / other Wayland exit menus**: dropped from this pass as evidence-free for a two-second switcher; dunst, fuzzel, rofi, cosmic-panel and alt-tab-macos covered the opacity and color-config questions with primary text.
