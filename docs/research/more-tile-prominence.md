# The `…` tile: how prominent can the "more" affordance be, and what would it cost?

**Question:** Airlock's cards show a `…` tile when the list is longer than the tile budget. It is currently not prominent. How can the affordance be improved, what did we lock it to, and what does peer practice and HCI guidance say the affordance should carry (ellipsis, count, chevron, interactivity)?

**Date:** November 14, 2026

**Scope:** Airlock `main` (src/ui.rs, src/switcher.rs, and the committed research records) read on this date; gnome-shell `main` `js/ui/altTab.js` (GitHub mirror), pop-os/cosmic-launcher `master` `src/app.rs`, Microsoft Learn Policy CSP Multitasking (ms.date 2026-10-14), Nielsen Norman Group (two articles), W3C WCAG 2.2 Understanding for SC 2.5.8, and Wikipedia on Fitts's law (tertiary, labeled as such). All remote fetches 2026-11-14. Apple's HIG pages were again unreadable this pass (the "Menus" page fetched as an empty JavaScript shell; support.apple.com answered 429 on 2026-11-09 per the earlier record), so Apple facts are carried and marked.

**Method:** primary sources for every external claim, with the supporting quote captured while reading. Remote code is cited by repo/file and identifier. Airlock claims carry `file:line` refs as of 2026-11-14; they will drift. This record changes no code.

---

## 1. What the `…` tile is today

### 1.1 The model: one handleless row

The affordance is a `RowModel` whose only content is a missing handle. `continuation_row()` (src/ui.rs:2392-2404) builds exactly that, and its doc states the contract: "Not hoverable, not clickable, not number-addressable, never selected — its missing handle is what every input guard already reads."

- `handle: None` — the pointer's click path addresses the handle captured at event time; "Presses on continuation rows queue nothing" (src/ui.rs:3925-3927, the `PointerInput::Click` doc).
- `hint: None` — no Ctrl+N chord is ever painted or wired for it (src/ui.rs:1317-1321).
- `count: 0`, `kind: Leaf`, empty title, `icon: None` — so the pip painter, which fires only `if row.kind == RowKind::Parent` with `row.count >= 2` (src/ui.rs:3093-3097), never marks it.
- The × reveal (`close-window`) excludes it: `closeable()` requires a handle and a non-parent kind (src/ui.rs:1515-1522), and the render guard repeats it: "App tiles (group headers) and the `…` tile never reveal, whatever the pointer does" (src/ui.rs:3399-3405).

Four headless tests pin this shape: the truncated band's tile "carries no window" and "hints no chord" (src/ui.rs:6465-6476), it "never paints a band" (src/ui.rs:6746), the render test `render_draws_the_continuation_dots_and_never_a_band_on_them` (src/ui.rs:8024), and `continuation_dots_center_their_painted_glyph` (src/ui.rs:8151).

### 1.2 The paint: one muted glyph

`render_band`'s content loop dispatches on `tile.handle.is_none()` first (src/ui.rs:3255-3293): it draws the `…` glyph centered on the icon line in `palette.muted` — a hand-drawn `ELLIPSIS_GLYPH` on the bitmap floor (src/ui.rs:1207-1211), the shaped real `\u{2026}` on the system stack — then `continue`s before any of the content-tile passes. The function doc carries the design sentence: "The `…` tile paints its muted glyph centered on the icon line and nothing else — no band, no icon, no title, no badge (a tile beyond addressability earns nothing but its dots)" (src/ui.rs:3227-3231).

The vertical cards paint the same vocabulary in full-row slots: the `…` glyph centered on the titles' line, muted, no fill (src/ui.rs:2858-2950).

`palette.muted` is a derived color, "the text color at 55% over the card," documented as the hint color and "frozen on the preview sheet 2026-11-04" as the mid-band of a judged 50-70 range (src/ui.rs:830-835). The measured contrast of muted ink is 5.39:1 on the dark card and 3.60:1 on the light card, below WCAG AA for text (appearance-settings.md §6.2, table row "Muted hints (text @ 55 % over card)").

### 1.3 The budget: where the affordance rides

- **Vertical cards:** the `…` rows ride *on top of* the window budget through `ROWS_SLACK = 2`, one slot per truncatable edge, so eight windows stay in view mid-list (src/ui.rs:241-267; the windowing rule at src/ui.rs:1727-1744). Both edges can carry one.
- **Horizontal band and grouped strip:** the affordance lives *inside* the budget, "replacing the trailing tile when the window's end hides windows... A truncated band shows `tiles − 1` windows... There is no leading affordance — the list reads left to right, so only a truncated *trailing* edge gets one" (src/ui.rs:1777-1784; the builder at src/ui.rs:1884: "A truncated trailing edge spends its last tile on the wordless `…`").
- The windowing rule always keeps the selection on a *content* tile, never the affordance slot (src/ui.rs:1736-1743, 1786-1790), so the fill can never rest on it and stepping past the edge is the only way the hidden windows are reached.

### 1.4 The card's existing prominence vocabulary

The levers a change could spend are all already named constants and paints:

- **Selection fill** — the strongest attention shape on the card, "the one thing the eye tracks at Tab speed" (switcher-ux-and-icons.md TL;DR #1). The `…` tile never paints it, and a test forbids that (src/ui.rs:6746).
- **Count pips** — the count vocabulary, "the horizontal tiles' count vocabulary (glance-tiles G2)": up to three muted squares, saturating, `PIP_SIDE = 6`, on multi-window parent tiles only (src/ui.rs:330-353, 3080-3097). group-presentation.md R4 locked the shape with Unity/GNOME/dash-to-dock precedent.
- **Disclosure caret** — the grouped parent's chevron, painted in the *text* color, not muted (src/ui.rs:1219-1222: "drawn at `CHEVRON_SCALE` in the text color"; the draw call passes `palette.text` at src/ui.rs:3111-3119). This is the in-card precedent for a structural mark at full ink weight.
- **Badge digits** — the Ctrl+N corner badges, also muted ("muted like every hint", src/ui.rs:3226), with a digit-only vocabulary (`RowHint::digit`, src/ui.rs:1287-1291) already shipped on tiles.
- **Tile geometry** — `TILE_W = 64`, `TILE_ICON = 48`, a structural pip row at the tile bottom that "is structural on every tile (pips paint only on multi-window app tiles), so the cell never varies per tile" (src/ui.rs:305-355). The `…` tile therefore leaves its pip row empty on every card.

## 2. Why it reads faint, and the locks an improvement would revisit

The faintness is not an accident; it is four frozen decisions stacked. Any change names which it re-opens:

1. **The wordless `…` mark.** switcher-ux-and-icons.md §5.2.3 locked the elision vocabulary: "elide right with `…`", and the glyph's own doc cites that lock (src/ui.rs:1207-1208: "research record §5.2.3 says elide with `…`"). A count label extends the lock; a chevron in the ellipsis's place abandons it.
2. **"Earns nothing but its dots."** The render doc (src/ui.rs:3227-3231) plus the four tests in §1.1 and the never-a-band test (src/ui.rs:6746) pin the minimal paint. Ink color, pips, a digit, or a fill each re-pin this block.
3. **Count means pips, on parents only, saturating at 3.** group-presentation.md R4 (Unity draws pips only for apps with more than one window and saturates at three; GNOME's arrow is binary; dash-to-dock's dots cap at four). The pips' pixel size was re-frozen on the G4 sheet, 2026-11-09 (src/ui.rs:331-343). A count on the `…` tile extends R4's scope from "multi-window apps" to the overflow slot itself.
4. **Muted is the hint color.** The dots share the 55% hint ink that the preview sheet froze on 2026-11-04 (src/ui.rs:830-835). Painting the dots at full text weight decouples them from that vocabulary; the chevron at `palette.text` (§1.4) shows the card already carries two ink weights for structural marks, so the split has an in-card precedent rather than a new color.

Two more locks are adjacent but not at stake: the handleless, inert interaction contract (§1.1) is a code and test contract from `pointer-select`, not a research-record lock, and making the tile interactive would revisit it; and the budget accounting (§1.3, both rules frozen in their windowing docs) is untouched by prominence work — nothing about ink or count changes what the budget shows.

## 3. Prior art: what peers do when the list overflows

The surveyed group splits into three behaviors, and none of them is a numeric "+N more" mark.

**Fit everything by shrinking.** GNOME Shell's app switcher renders every item and steps the icon size down a ladder until the row fits. `_setIconSize()` walks `const baseIconSizes = [96, 64, 48, 32, 22]`, computing `w = height * this._items.length + totalSpacing` until it fits the available width, and "the loop leaves `iconSize` at the smallest value (22) if nothing fits — items are never dropped or truncated"; the file contains no truncation marker of any kind (gnome-shell `js/ui/altTab.js`, fetched from the GitHub mirror 2026-11-14).

**Fit everything by scrolling.** COSMIC's own launcher keeps every result reachable in a scrollable view: `const SCROLL_MIN: usize = 8` and, in `view_window`, `if buttons.len() > SCROLL_MIN` the list is wrapped as `container(scrollable(components::list::column(buttons)).id(SCROLLABLE.clone()))` capped at `.max_height(504)`, while eight or fewer render flat (pop-os/cosmic-launcher `src/app.rs`, fetched 2026-11-14). The in-ecosystem precedent hides nothing and adds no marker.

**Truncate silently.** Windows 11 caps what Alt+Tab shows with no marker: the Multitasking policy `BrowserAltTabBlowout` defaults to "Open windows and all tabs in Microsoft Edge" and can be set to "Open windows and 5 most recent tabs", "Open windows and 3 most recent tabs", or "Open windows only" (Microsoft Learn, Policy CSP Multitasking, ms.date 2026-10-14). Nothing in the policy or the Settings-app description surfaces a count of what was dropped.

**Magnitude hints without numbers.** Where peers do mark multi-window-ness, the mark saturates: Unity's switcher pips stop at three (group-presentation.md §2.2), GNOME's under-app arrow is binary and multi-window-only (§2.1), dash-to-dock's dots cap at four (§2.8). The Plasma task manager's schema has no window-count badge at all; its overflow answers are interaction ones (a hover flyout of previews, or a "textual list (AKA group dialog)" popup) opened from the group's own button (§2.3).

**macOS never answers the question in the strip.** Cmd+Tab steps apps; an app's windows live outside the switcher entirely (Command-Grave, App Exposé), carried from switcher-behavior-adaptation.md §4/§11 and group-presentation.md §2.6, where Apple's pages were verified 2026-10-31 (429 again since).

**A browser tab bar is the closest shipped chevron-on-overflow.** Firefox shows a "List all tabs" chevron button at the end of an overflowing tab strip (secondary sources only this pass: the button became permanent around Firefox 106; ghacks, 2024-10-16). Treated as a secondary-source datapoint, not a verified primary claim (see Unverified).

Honest summary: the peer record supports *saturating pips/arrows* and *fit-everything* strategies; silent truncation (Windows) is the failure mode Airlock's tile exists to avoid; and a numeric "+N" on the overflow tile itself has no precedent in any switcher surveyed. That makes a number the strongest-scent but least-precedented choice, and pips the most precedented but least-informative one.

## 4. HCI guidance, and what of it applies at a glance

**Progressive disclosure requires a discoverable, expectation-setting affordance.** Nielsen's formulation: show "only a few of the most important options" first and "offer a larger set of specialized options upon request"; visibility itself carries meaning ("the very fact that something appears on the initial display tells users that it's important"), and the entry point must be labeled to "set clear expectations for what users will find"; with multiple paths, "at least ensure that you clearly label all choices" (NN/g, Jakob Nielsen, "Progressive Disclosure", 2006-12-03). A wordless muted `…` sets weak expectations; a count sets the strongest ("how much more is there"); pips sit between. The article's two-levels-max warning ("designs going beyond two levels typically have low usability") does not bite here: Airlock's affordance discloses a *continuation of one list*, not a second level.

**Attention is a cost.** NN/g on indicators: they are "visual cues intended to attract users' attention to a particular piece of content or UI element that is dynamic in nature" but "can introduce noise and clutter to your overall interface, and may distract users" (Kim Flaherty, "Indicators, Validations, and Notifications", 2024-01-17). On a card whose whole job is a 200 ms recognition scan (the `airlock-design` skill's own floor: "the list must survive a 200 ms glance"), a louder `…` trades scan noise against comprehension of truncation; the saturating-pip family (Unity 3, dash-to-dock 4) is the shipped compromise between attention and reading load, and it is what the card already speaks.

**Target-size rules govern a future pointer target, not today's tile.** WCAG 2.2 SC 2.5.8: "The size of the target for pointer inputs is at least 24 by 24 CSS pixels," with Spacing, Inline, Equivalent, User Agent Control, and Essential exceptions, and the Understanding doc's intent is activation "without accidentally activating an adjacent target" (W3C WCAG 2.2 Understanding, Target Size (Minimum), fetched 2026-11-14). Airlock's `…` tile is *not a target at all* — nothing is activatable on it, so the criterion is vacuous today rather than violated. If it ever became a click target it would be a 64 × 76 logical-px cell (src/ui.rs:305-357), far above the minimum, with the tile gap (8 px) as spacing. Keyboard reachability of the hidden windows already exists by stepping past the edge (§1.3).

**Fitts's law is pointer-epoch guidance; position is its transferable part.** "The time required to rapidly move to a target area is a function of the ratio between the distance to the target and the width of the target" (Fitts 1954, Shannon form ID = log2(D/W + 1); Wikipedia, tertiary source, read for the formulation only). The overlay is driven by an Alt+Tab gesture; the pointer is secondary (shipped as `pointer-select`, never the primary loop). So the law's usable lesson for this card is placement, not inflation: the affordance already sits at the trailing edge, which is where a left-to-right scan and a rightward pointer motion both end. No prominence change needs to move it, and the frozen "no leading affordance" rule matches the reading direction (src/ui.rs:1777-1784).

**What actually applies.** The transferable findings for a sub-second, gesture-driven overlay are the recognition-speed ones: a visible mark in the right place (satisfied), enough contrast to survive peripheral vision (the muted ink's 3.60:1 light measurement is the weak link on the light card), and an expectation-setting magnitude cue (absent today). Pointer-accuracy guidance (WCAG 2.5.8, Fitts) becomes load-bearing only for the optional "make it clickable" lever (§5, L4).

## 5. Candidate levers, each with the lock it re-opens

Ranked roughly by evidence weight; the sheet process (`overlay-preview`, both wallpapers, the long-list edge cases) judges what constants settle on, and every lever stays inside the card's existing vocabulary.

- **L1. Full-weight ink.** Paint the dots in `palette.text` (the caret's own precedent, §1.4) or a new derived ink. Re-opens: the muted-as-hint coupling (lock 4) and the render doc block (lock 2). Measured effect: dark 5.39 → ~14.6:1, light 3.60 → ~14.7:1. No vocabulary change; the `…` stays the §5.2.3 mark.
- **L2. Count pips on the tile.** Spend the structurally empty pip row (§1.4) with pips equal to the hidden-window count, saturating at three like every other count on the card. Re-opens: R4's "parents only" scope (lock 3) and the render contract (lock 2). Precedented (Unity family); keeps the card wordless.
- **L3. An on-tile digit.** Paint "+N" (the badge digits' vocabulary exists, `RowHint::digit`) in the pip row or beside the dots. Re-opens: the wordless lock (lock 1) plus locks 2 and 3. Strongest information scent per NN/g; no peer precedent in any switcher surveyed (§3); a number also invites a reading a gesture user has no time for.
- **L4. A pointer target.** Make the tile clickable (commit the window the next step would reach, or jump the window forward). Re-opens: the handleless contract (§1.1, the `pointer-select` semantics and its tests) — the only lever that changes interaction, and the one with no peer precedent for the overflow slot itself. WCAG 2.5.8 and Fitts apply from that point on and are trivially satisfied by the existing geometry.

The budget locks (§1.3) and the trailing-edge placement are not levers: nothing in the evidence argues against either.

## 6. Sources

All fetched 2026-11-14 unless marked.

**Airlock (this repository)**

- src/ui.rs:1515-1522 (`closeable`), 2392-2404 (`continuation_row`), 241-267 (row budget and `ROWS_SLACK`), 305-357 (tile geometry tokens), 830-835 (`muted` doc), 1207-1211 (`ELLIPSIS_GLYPH`, §5.2.3 citation), 1219-1222 (caret in text color), 1287-1291 (badge digit), 1515-1522, 1777-1790 (`band_window` doc), 1884 (wordless builder doc), 2858-2950 (vertical `…` paint), 3093-3120 (pips and caret paints), 3221-3293 (`render_band` doc and the dots branch), 3399-3405 (× reveal guard), 3911-3938 (`PointerInput` docs), 6465-6476, 6746, 8024, 8151 (the four shape tests)
- src/switcher.rs:350-403 (`RowKind`, `GroupedRow`, `Grouping`)
- docs/research/switcher-ux-and-icons.md §5.2.3-§5.2.4 (elision vocabulary, long-list budget), TL;DR #1 (the selection fill)
- docs/research/group-presentation.md §1.2 R4, §2.1-§2.3, §2.6, §2.8 (pips/arrows precedents, macOS, task manager), §6 (sources there)
- docs/research/appearance-settings.md §6.1-§6.2 (WCAG thresholds; muted measured 5.39:1 dark, 3.60:1 light)
- docs/research/switcher-behavior-adaptation.md §4, §11 (carried Apple facts, verified 2026-10-31)
- `.zcode/skills/airlock-design/SKILL.md` (the 200 ms glance floor; the austerity contract)

**GNOME**

- gnome-shell `js/ui/altTab.js` (`main`, GitHub mirror), `_setIconSize` and the icon ladder: https://raw.githubusercontent.com/GNOME/gnome-shell/main/js/ui/altTab.js

**COSMIC**

- cosmic-launcher `src/app.rs` (`master`), `SCROLL_MIN`, `view_window`: https://raw.githubusercontent.com/pop-os/cosmic-launcher/master/src/app.rs

**Microsoft**

- Policy CSP Multitasking, `BrowserAltTabBlowout` (ms.date 2026-10-14): https://learn.microsoft.com/en-us/windows/client-management/mdm/policy-csp-multitasking

**HCI**

- NN/g, Jakob Nielsen, "Progressive Disclosure" (2006-12-03): https://www.nngroup.com/articles/progressive-disclosure/
- NN/g, Kim Flaherty, "Indicators, Validations, and Notifications" (2024-01-17): https://www.nngroup.com/articles/indicators-validations-notifications
- W3C, WCAG 2.2 Understanding SC 2.5.8 Target Size (Minimum): https://www.w3.org/WAI/WCAG22/Understanding/target-size-minimum.html
- Wikipedia, "Fitts's law" (tertiary, formulation only): https://en.wikipedia.org/wiki/Fitts%27s_law

**Secondary, flagged**

- ghacks, "Firefox adds option to remove List all tabs button" (2024-10-16): https://www.ghacks.net/2024/10/16/firefox-adds-option-to-remove-list-all-tabs-button-after-user-backlash

---

## 7. Unverified

- **Apple.** The HIG "Menus" page fetched as an empty JavaScript shell this pass and support.apple.com was unreachable on the earlier pass, so nothing about the macOS Window menu (whether it lists all open windows, numbered) is verified; the carried macOS facts (§3) rest on the 2026-10-31 verification recorded in switcher-behavior-adaptation.md.
- **Firefox's overflow chevron.** No primary Mozilla source (release notes or source file) was fetched; the "appears on overflow, permanent since Firefox 106" claim rests on the ghacks article and Mozilla support threads, all secondary.
- **Windows 11 Alt+Tab's own visuals** (thumbnail sizing, whether the grid itself ever truncates windows rather than tabs): no first-party page describing it was found this pass; the policy page covers tabs only. The prior record's finding stands: no first-party documentation of an app-grouped Alt+Tab exists.
- **cosmic-comp's workspace overview overflow** (how the Super-key overview renders more workspaces than fit): the implementation was not located in pop-os/cosmic-comp's tree this pass (src/shell/{workspace.rs, grabs/} fetched; the overview lives elsewhere); not verified.
- **Fitts's original paper** (Fitts 1954, J. Exp. Psychol.) was not read; the formulation comes from the Wikipedia article and is labeled tertiary. No conclusion here rests on quantitative Fitts predictions.
- **Material Design 3's badge guidance** (count-badge conventions, digit caps): m3.material.io's badge overview returned 404 this pass; not used.
- **Line references** into Airlock are as of 2026-11-14 and will drift.
