# App grouping as treeview — research findings

**Date:** November 7, 2026 · **Scope:** the W3C WAI-ARIA APG tree view pattern, and per-app window grouping in GNOME Shell (`main`), KWin (`master`), Windows 11, macOS, Dash to Dock, and Waybar's wlr-taskbar — all fetched 2026-11-07 — against Airlock `main` (src/settings.rs, src/switcher.rs, src/ui.rs, src/daemon.rs, docs/settings.md). · **Method:** primary sources only — the APG pattern page read directly; upstream code read at gitlab.gnome.org / invent.kde.org raw URLs and the GitHub API raw endpoints; first-party Microsoft and Apple support pages; GSettings schemas read in-repo. Remote code is cited by repo/file + function name (line numbers were not stable across fetches); Airlock claims carry file:line refs into this repository. One re-fetch of Apple's keyboard-shortcuts page was rate-limited (HTTP 429); its quote is carried from [switcher-behavior-adaptation.md](switcher-behavior-adaptation.md) §4, verified against the same primary source on 2026-10-31.

---

## 1. TL;DR — verdicts

| # | Question | Verdict | Basis |
|---|---|---|---|
| 1 | Per-app grouping, configurable, **off by default** | **Adopt with work** — the default-off choice has direct precedent: KWin's only in-tree grouping control and both of Waybar's grouping options ship default-off; GNOME (grouped by default) is the outlier, not the norm | [tabboxconfig.h](https://invent.kde.org/plasma/kwin/-/raw/master/src/tabbox/tabboxconfig.h) `defaultApplicationsMode()`; [Waybar man](https://github.com/Alexays/Waybar/blob/master/man/waybar-wlr-taskbar.5.scd) (§4) |
| 2 | Treeview navigation per W3C | **Adopt** — the APG keyboard model maps cleanly onto the shipped `GestureKey`/`Gesture` machinery, with one deliberate rebinding: →/← must stop aliasing step and become expand/collapse-enter/exit when grouping is on | [APG tree view](https://www.w3.org/WAI/ARIA/apg/patterns/treeview/); src/ui.rs:1373-1380 (§2, §7) |
| 3 | Parent prominence | **Adopt as a design decision** — the APG prescribes **no** visuals; platform precedent fills the gap (GNOME marks multi-window parents `EXPANDABLE`/`EXPANDED` in Atk and hides the arrow on single-window apps; children get the badge/indent role) | altTab.js `_addIcon`/`highlight`; APG "visual design" note (§5) |
| 4 | Does the vertical list need to change to horizontal? | **Keep vertical** — every grouped-horizontal shape in the survey leans on window thumbnails or icon-only cells; Airlock's established ceiling (no thumbnails for an external Wayland client, ≤600 px card, per-row `Ctrl+N` hints) rules both out. Vertical tree = the existing flat list + indent + parent rows | whats-next-post-v0.1.0.md; switcher-ux-and-icons.md (§6) |
| 5 | Reuse the shipped same-app machinery | **Partial** — `app_anchor`/`app_view` narrow to *one* app (the grave-family session); grouped Alt+Tab needs a new partition of `eligible()` by `app_id`, reusing the "empty app_id never groups" rule those functions already encode | src/switcher.rs:129-173 (§7) |
| 6 | Key model in grouped mode | **Recommend the GNOME mirror** — Tab steps apps, ↓ enters, ↑ exits, ←/→ collapse/expand (W3C vertical-tree assignment), Enter commits; the already-shipped same-app session covers within-app cycling | altTab.js `_keyPressHandler` vs APG; §2, §7 |

---

## 2. The W3C tree view pattern — behavior, not visuals

Source: [WAI-ARIA Authoring Practices Guide, Tree View Pattern](https://www.w3.org/WAI/ARIA/apg/patterns/treeview/), fetched 2026-11-07. The pattern describes a vertically oriented tree; the keyboard model is stated generally, not per-depth, so it applies as-is to a two-level (app → windows) tree.

### 2.1 Keyboard interaction (the load-bearing table)

| Key | Prescribed behavior (condensed from the pattern's own wording) |
|---|---|
| **↓** | "Moves focus to the next node that is focusable **without opening or closing a node**" |
| **↑** | "Moves focus to the previous node that is focusable without opening or closing a node" |
| **→** | On a **closed** node: "opens the node; focus does not move." On an **open** node: "moves focus to the first child node." On an **end node**: does nothing. |
| **←** | On an **open** node: "closes the node." On a **child** node that is closed or an end node: "moves focus to its parent node." On a **root** node that is closed or an end node: does nothing. |
| **Home / End** | First / last focusable node, without opening anything |
| **Enter** | "activates a node, i.e., performs its default action" — for a selectable tree, selects; for a parent it may be the toggle |
| **Typeahead** | Recommended for all trees, "especially those with more than 7 root nodes" |
| **\*** | *Optional:* expands all siblings at the current level |

Two consequences worth spelling out for a two-level switcher:

- **↓ walking the whole tree needs no special rule.** The definition of ↓ ("next node that is focusable without opening or closing") means that with every parent expanded, ↓ walks app → its first window → its second window → next app. For a flat-vertical switcher that gains grouping, the step semantics *do not change* — only what is visible does.
- **The pattern is expand-state-first.** →/← are *not* navigation aliases on a vertical tree: they are open/close and enter/exit. This is the one direct conflict with Airlock today: `key_input` maps `Up | Left → Arrow(Previous)` and `Down | Right → Arrow(Next)` (src/ui.rs:1378-1379), so → and ← currently *step*. Under grouping those two arms must be re-bound to the expand/collapse-enter/exit behaviors; ↑/↓ keep stepping.

### 2.2 Roles and states

- All nodes live in an element with role **tree**; each node is a **treeitem**; each parent "contains or owns an element with role **group**" holding its children.
- **aria-expanded** (true = open, false = closed) goes on parent nodes only — "end nodes must not have aria-expanded, otherwise assistive technologies would incorrectly identify them as parents." A single-window app is an end node: it renders like a parent but is not expandable and → does nothing on it.
- Selection is **aria-selected** (or aria-checked — never both); **aria-level/setsize/posinset** are required only for dynamically loaded trees; `aria-orientation` defaults to vertical.
- Focus model: the pattern describes behavior, not DOM structure, and offers `aria-activedescendant` as an alternative to moving DOM focus among treeitems; the arrow rules imply collapsed parents and their hidden children behave as single focus stops, since ↓/↑ move between focusable nodes without opening or closing.

### 2.3 Visual design — deliberately silent

The pattern's only visual guidance is that "it is important that the visual design distinguish between items that are selected and the item that has focus," plus a visible focus indicator. **It says nothing about parent prominence, indentation, or sizing.** "A parent should be prominent" is therefore a design decision to be grounded in platform precedent (§5), not a W3C requirement — and the *behavioral* half of "prominent parent" (parents are always focusable, carry the expanded state, are reachable with ← from any child) is fully specified above.

---

## 3. Platform precedent: who groups, how, and laid out which way

| Platform | Grouping model | Default | Navigation inside the grouped UI | Layout | Parent rendering | Source |
|---|---|---|---|---|---|---|
| **GNOME Shell** (app switcher) | True two-level: apps, each with its window list | **Grouped** — Alt+Tab is bound to `switch-applications` by default; `switch-windows` (per-window) defaults to **empty**, so ungrouped Alt+Tab exists only by rebinding | ←/→ step apps; **↓ enters the app's window row** (selecting window 0); **↑ exits back**; Tab always steps apps even from the window row; `q` quits the app, `w`/`W`/`F4` close the focused window; initial selection is app 1 (the MRU-second app) | **Horizontal** app-icon row + a second **horizontal** thumbnail row below (appears after a 500 ms delay) | App icon (96→32 px shrink ladder `baseIconSizes = [96, 64, 48, 32, 22]`), label under it, arrow drawn only for multi-window apps; multi-window apps carry Atk `EXPANDABLE`, and `EXPANDED` while thumbnails are shown — the aria-expanded analogue | [altTab.js](https://gitlab.gnome.org/GNOME/gnome-shell/-/raw/main/js/ui/altTab.js) `_keyPressHandler`, `_select` doc comment, `_initialSelection`, `_addIcon`, `_setIconSize`, `THUMBNAIL_POPUP_TIME`; [keybindings schema](https://gitlab.gnome.org/GNOME/gsettings-desktop-schemas/-/raw/main/schemas/org.gnome.desktop.wm.keybindings.gschema.xml.in) |
| **KWin / Plasma 6** | **Not a tree** — `OneWindowPerApplication` *filters* the list to one representative window per app; no per-app children exist | **Off** — `defaultApplicationsMode()` returns `AllWindowsAllApplications`; opt-in via a checkbox in the tabbox KCM and the `ApplicationsMode` config key | Arrow keys drive the QML grid (`moveCurrentIndex*`), wrapping, instant highlight (`highlightMoveDuration: 0`) | **Horizontal-first grid**: `GridView` default `LeftToRight` flow with a greedy column count — but **one window per cell, no grouping** | Single-level: thumbnail tile with a 64 px app-icon badge at its bottom-center; no parent concept | [tabboxconfig.h](https://invent.kde.org/plasma/kwin/-/raw/master/src/tabbox/tabboxconfig.h); [kwintabboxsettings.kcfg](https://invent.kde.org/plasma/kwin/-/raw/master/src/kcms/tabbox/kwintabboxsettings.kcfg) (`ApplicationsMode`, type Int); [kwintabboxconfigform.cpp](https://invent.kde.org/plasma/kwin/-/raw/master/src/kcms/tabbox/kwintabboxconfigform.cpp) (`setApplicationMode` → `ui->oneAppWindow`); [thumbnail_grid main.qml](https://invent.kde.org/plasma/kwin/-/raw/master/src/tabbox/switchers/thumbnail_grid/contents/ui/main.qml) |
| **Windows 11** | Alt+Tab is per-window; app grouping lives on the **taskbar** (grouped icon with a flyout; "Ctrl+select a grouped app icon on the taskbar" to "shift focus through the windows in the app group") | Taskbar groups (grouped icons are the documented norm); **Alt+Tab grouping: no toggle documented** | Plain Alt+Tab is Tab-cycle only; arrow keys are documented for **Ctrl+Alt+Tab** ("View a thumbnail of all open apps. Use the arrow keys…"); no grouped-overlay navigation documented | Horizontal thumbnail strip (Alt+Tab) | Not specified on first-party pages | [Microsoft — Keyboard shortcuts in Windows](https://support.microsoft.com/en-us/windows/keyboard-shortcuts-in-windows-dcc61a57-8ff0-cffe-9796-cb9706c75eec) |
| **macOS** | Cmd+Tab is **per-app by design** (apps, not windows); windows of the front app are a separate gesture (Cmd+Grave — "Switch between the windows of the app you're using…" per [switcher-behavior-adaptation.md](switcher-behavior-adaptation.md) §4, verified 2026-10-31; re-fetch rate-limited 2026-11-07) | Grouped (app-unit) — and **no grouped window overlay exists at all** | Cmd+Tab strip: hold Cmd, Tab steps, ←/→ scroll apps, Esc or period cancels; windows reached via Cmd+Grave, never inside the strip | **Horizontal** icon strip, keyboard-only documented interface | Icon + name cell; Apple publishes no px values | [Apple — Move and arrange app windows](https://support.apple.com/guide/mac-help/work-with-app-windows-mchlp2469/mac) |
| **Dash to Dock** (GNOME dock) | Per-app grouped icon; clicking it **cycles windows by default** (`click-action` default `'cycle-windows'`), previews replace the open-windows list when enabled (`show-windows-preview` default true) but stay **hidden behind a drop-down by default** (`default-windows-preview-to-open` default false) | Grouped icon; preview flyout opt-in per interaction | Mouse-first (click/scroll actions configurable: `on-click`-equivalent enums, `scroll-action` default do-nothing); no keyboard nav documented | Vertical popup above the dock | App icon is the unit; windows are list/preview entries | [dash-to-dock gschema.xml](https://github.com/micheleg/dash-to-dock/blob/master/schemas/org.gnome.shell.extensions.dash-to-dock.gschema.xml) |
| **Waybar wlr-taskbar** | Two independent opt-ins: `sort-by-app-id` ("If set to true, group tasks by their app_id" — adjacency, still one button per window) and `squash-list` (multiple instances of a listed app collapse into one button) | **Both off** (`sort-by-app-id` default false; `squash-list` default unset) | None documented — mouse module (`on-click` / `on-click-middle` / `on-click-right`) | Horizontal bar | One button per window (or per squashed app) | [Waybar man](https://github.com/Alexays/Waybar/blob/master/man/waybar-wlr-taskbar.5.scd) |

---

## 4. The default: off-by-default is the minority position for switchers, the majority for Linux taskbars

- **Switchers:** GNOME is grouped by default and offers no toggle — only a rebinding path (`switch-windows` defaults to `[]`). Windows' Alt+Tab is per-window with no documented grouping toggle. macOS has no grouped overlay. KWin's only grouping-shaped control defaults off and is a filter, not a tree. **No switcher in the survey ships a true grouped tree off by default** — there is no direct precedent for the exact shape being proposed; there *is* precedent for the default: KWin's default (`AllWindowsAllApplications`) is exactly "grouping off."
- **Taskbars:** Waybar ships two grouping options, both default-off; Dash to Dock defaults to *cycling* rather than showing a grouped flyout. Linux panel software prefers opt-in grouping.
- Windows taskbar combining is default-on (grouped icons are the documented norm on the shortcuts page); the "Combine taskbar buttons" setting itself was not re-verified from a primary page this pass — no claim made on its current default position.

Airlock's ask — a `grouping` setting, default false — is therefore the conservative corner of the design space: it changes nothing for existing users, matches KWin's shipped default, and mirrors Waybar's config philosophy, while the grouped mode itself has GNOME as its only true two-level precedent.

---

## 5. Parent prominence — what the W3C leaves to design, and what platforms do

- **W3C:** nothing prescriptive (§2.3). The *behavioral* parent model is specified: parents are focusable, carry `aria-expanded`, are the ←-exit target from any child, and end nodes (single-window apps) must not claim expandability.
- **GNOME** is the closest precedent for a two-level switcher and does three things worth copying:
  1. The parent is the **app icon row** (up to 96 px, shrinking on a `[96, 64, 48, 32, 22]` ladder to fit the screen) with the app name as its label; children are entries *under* it.
  2. An **arrow affordance appears only on multi-window apps** — `_addIcon` hides the arrow when `cachedWindows.length === 1` — exactly the APG's "end nodes must not claim expandability," rendered as a chevron policy.
  3. The accessibility tree carries the state: `EXPANDABLE` on multi-window app icons, `EXPANDED` while the window list is shown (altTab.js `_addIcon`, `_createThumbnails`, `_destroyThumbnails`).
- **KWin** has no parent concept; its badge (64 px app icon on each thumbnail) marks *ownership*, not structure.
- **For Airlock**, the transferable design is: parent rows carry the app icon at the existing 32 px row-icon size (shipped in overlay-visuals) plus the app name and a count/chevron affordance; child rows are indented under the parent. Prominence = *un-indented + icon-led + expanded-state marker*, not a bigger font — the card's 12-px title pin (recent OD3 work) and ≤600 px width both argue against scale-based prominence. Final visuals belong to the `airlock-design` skill, not this record.

---

## 6. Does the list view need to change shape? — No. Keep vertical.

Every grouped-horizontal layout in the survey leans on one of two things Airlock cannot or should not adopt:

1. **Window thumbnails.** GNOME's horizontal second row is a row of live window clones; Windows' Alt+Tab strip is live previews; KWin's grid is thumbnails per window. This repo has already established, with citations, that **window thumbnails are out of reach for an external Wayland client** — Airlock reads `zcosmic_toplevel_info_v1` (src/wayland.rs) and the honest ceiling is app icon + title + state tags (whats-next-post-v0.1.0.md, "Window thumbnails"; switcher-ux-and-icons.md §2.10). A horizontal grouped card without thumbnails collapses to **icon-only cells**, which discards the title text that is Airlock's entire information surface.
2. **Icon-only parent rows.** GNOME's and macOS's first axis works because each cell is just icon + name. Airlock's rows are title-led; converting the parent axis to icon cells would also break the `Ctrl+N` hint scheme — hints are right-aligned per-row bands (src/ui.rs:339-341, `hint_band_points`), and a horizontal grid has no row band to hang them on. The hint count also caps at 9 (`hint_label`, src/ui.rs:1138), which matches a vertical list's row budget, not an unbounded horizontal wrap.

What grouping actually needs from the list view is:

- a **partition of the existing MRU list** (not a new layout),
- an **indent on child rows** (a per-row offset — `RowModel` already carries per-row state, src/ui.rs:400-408, and `row_at` hit-testing is row-band based and layout-agnostic, src/ui.rs:431-446),
- **re-bound →/←** when grouping is on (§2.1),
- optional **collapse**, which changes the visible sequence but not the rendering machinery (`visible_window` operates on any list length, src/ui.rs:492+).

The KWin grid is sometimes cited as a "horizontal grouped switcher," but it is a per-window grid with no grouping at all — it is not a precedent for grouped-horizontal. **Verdict: keep vertical.** Horizontal remains available if Airlock ever gains thumbnails, which two prior records rule out for an external client.

---

## 7. Fit for Airlock — grounded in the current code

### 7.1 Settings (`grouping = false`)

The `Settings` struct (src/settings.rs:50-57) and `parse_root` (src/settings.rs:144-199) already carry the pattern: `theme` (enum), `rows` (range-checked integer), `[[workspace]]` rules. `grouping` slots in as a fourth top-level key with the same shape as `rows`: `match root.get("grouping")` → `as_bool()`, else `SettingsError::InvalidFile { problem: "`grouping` must be a boolean" }`; absent key → `false`. docs/settings.md gains a section mirroring [Rows](../settings.md#rows) (read at gesture start, last-good fallback, diagnostic on bad value — the "Live apply" contract at docs/settings.md:136-139 already covers it). An empty file must keep reproducing v1 behavior (src/settings.rs:48 "An empty one reproduces v1 behavior everywhere") — with `grouping` defaulting false, it does.

### 7.2 The grouped list — partition, not a new view

`eligible()` (src/switcher.rs:86-97) returns MRU-ordered handles; `is_switchable` (src/switcher.rs:104-112) already excludes minimized windows. The grouped view is a pure partition of that same list:

- **Parents in order of first appearance** in the MRU list (equivalently: ordered by their frontmost window's MRU position) — this preserves Airlock's MRU-head contract ("one step lands on the MRU-second", src/daemon.rs:146-147): the first parent is the frontmost window's app, and the initial selection steps to the second *visible* row.
- **Children MRU within their parent** — GNOME does the same (`cachedWindows` filters the MRU `getWindows` list per app, altTab.js `AppSwitcher._init`).
- **Windows with an empty `app_id` never group** — they stay top-level rows. This rule already exists in the codebase's own words: "an empty app_id never anchors — windows the compositor cannot attribute never group" (src/switcher.rs:129-141, `app_anchor`) and "an empty `app` … groups nothing" (`app_view`, src/switcher.rs:150-173). Reuse the rule; the partition helper is new, because `app_view` narrows to *one* app for the grave-family session and is the wrong tool for an all-apps view.

### 7.3 Selection — a visible-sequence walk beside the flat Ring

`Ring` (src/switcher.rs:177-217) steps flat indexes with wrap-around. Under grouping with collapse, the visible sequence differs from the flat sequence, and the APG requires stepping over *focusable* (visible) nodes only (§2.1). The clean shape is a small pure tree-index type next to `Ring` — `next_visible`/`previous_visible`/`enter_child`/`exit_to_parent` over the partition — not a `Ring` replacement: the flat default keeps `Ring` untouched, and the grouped session uses the tree walk. If collapse is deferred (recommendation: **all parents expanded on open**), the grouped sequence *is* the flat grouped order and even `Ring` would work; collapse is the only feature that forces the walk. Expanding everything up front also fits the first-frame contract: GNOME delays its second row 500 ms (`THUMBNAIL_POPUP_TIME`) because thumbnails are expensive to build — Airlock's children are text rows, and the 2-second idle close leaves no room for a staged reveal.

### 7.4 Input — one rebinding, no new key classes

- **↑/↓ keep meaning step** (`Arrow(Previous/Next)`) — under grouping they step the visible sequence (§2.1's ↓ walks children naturally once expanded).
- **→/← must stop aliasing step** in grouped mode. Today `key_input` maps `Keysym::Up | Keysym::Left → Arrow(Previous)` and `Down | Right → Arrow(Next)` (src/ui.rs:1378-1379); `feed` turns those into `GestureOutput::Step` (src/ui.rs:1303). Grouped mode needs two new `GestureKey` arms (expand/enter, collapse/exit) selected on the *state* of the current row — the boundary produces them from the same keysyms when the session is grouped. `feed` gains matching outputs; the daemon applies them by rebuilding the model, exactly as `Step` does today.
- **Tab stays the gesture's spine.** Recommendation (verdict #6): in grouped mode, Tab steps **apps** — GNOME's `SWITCH_APPLICATIONS` action steps apps from anywhere in the popup, including from the focused window row (altTab.js `_keyPressHandler`), and that is what "Alt+Tab cycles applications" means to a GNOME user. The conservative alternative (Tab keeps stepping flat windows) is defensible; it is a spec-phase decision. Either way the shipped same-app session (`same-app`, switcher-behavior-adaptation.md §4) already covers within-app cycling for users who want it.
- **Enter commits** — already shipped (`GestureKey::Commit`, src/ui.rs:1377), and identical to the APG's Enter-activates and GNOME's explicit-confirm behavior (switcher-behavior-adaptation.md §2).
- **Ctrl+N unchanged** — row-index addressing works on whatever rows are visible; grouped lists with >9 rows simply exceed the hint budget the way long flat lists already do (`hint_label` returns `None` past nine, src/ui.rs:1138; rows past nine "paint nothing" — every displayed hint is a wired key). Collapsed parents, if added later, *reduce* rows in view, which helps this budget.
- **First-frame contract untouched** — no new key can arrive before the first `Modifiers` report; the grouped model is built at session open like the flat one (src/ui.rs overlays are built from `overlay_model` with a frozen `Windowing` budget, src/ui.rs:465-473).

### 7.5 Rendering — per-row state, no layout change

`RowModel` (src/ui.rs:400-408) gains a depth/indent marker and, for parents, an expanded-state affordance (chevron/count). The renderer's row rhythm (ROW_H/ROW_GAP bands) and `row_at` hit-testing are unchanged. GNOME's chevron policy is the precedent to copy: the affordance renders only when the app has ≥2 eligible windows — single-window apps are leaf rows (§2.2, §5).

---

## 8. Residual unknowns

- **GNOME app-grouping toggle:** no setting gates the `AppSwitcherPopup`'s per-app grouping in the code read this pass (`AppSwitcherPopup._init` builds apps directly; the only `org.gnome.shell.app-switcher` key it reads is `current-workspace-only`). Whether some other GNOME surface offers a toggle was not verified — no claim made either way.
- **Windows taskbar "Combine taskbar buttons" default:** not re-verified from a live primary page this pass; the shortcuts page shows grouped taskbar icons as the norm without stating the setting's default.
- **Apple 102650 verbatim quote:** re-fetch returned HTTP 429; the quote is carried from switcher-behavior-adaptation.md §4 (verified against the same primary source, 2026-10-31).
- **KWin KCM location:** the first probe looked in plasma-desktop and missed; the tabbox KCM lives in the kwin repository itself (src/kcms/tabbox/) and was verified there.
