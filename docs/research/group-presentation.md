# Presenting grouped windows (one app, N windows): research findings

**Question:** How do well-regarded window switchers present windows grouped by application, and what do research and practice say is the best way to present "one app, N windows" mid-gesture? Specifically: what should Airlock's grouped-horizontal card do, given a hand-composited near-opaque card, no animation, translucency, or shadows, no window thumbnails, and a 200 ms glance?

**Date:** November 9, 2026

**Scope:** GNOME Shell `main` (altTab.js, switcher theme, GNOME Help), Unity 7 (git.launchpad.net/unity), KDE kdeplasma-addons window-switcher layouts and the Plasma Task Manager schema (`master`), elementary Gala (`main`), Cinnamon (`master`), Compiz staticswitcher (compiz-reloaded), AltTab for macOS (`master`), dash-to-dock (`master`), swayr, i3ipc-python, the Raycast manual, Microsoft Learn and Support, Nielsen Norman Group, and Material Components for Android. All were fetched 2026-11-09 and read against Airlock `main` (src/ui.rs, src/daemon.rs, docs/settings.md).

**Method:** Primary sources only. gitlab.gnome.org answered HTTP 406 to every request this pass, so GNOME code was read from its official GitHub mirror (`github.com/GNOME/gnome-shell`). Apple Support answered HTTP 429 to every request, so Apple facts are carried from [switcher-behavior-adaptation.md](switcher-behavior-adaptation.md) (verified 2026-10-31) and marked as carried. No academic paper could be read: the fetch tool does not read PDFs, ACM and Taylor & Francis answered 403, and Semantic Scholar and Internet Archive Scholar answered 429. §7 lists those papers; no conclusion here rests on them. Remote code is cited by repo/file and identifier; Airlock claims carry file:line refs as of this date.

---

## 1. Recommendation

### 1.1 Where today's card stands

The grouped-horizontal card stacks two bands of one tile type (src/ui.rs:1616-1750, src/ui.rs:2022-2048):

- **Band 1** holds the app tiles: icon, label, count chip. A parent's label is its raw `app_id` string, because the tile title falls back to the app_id when the title is empty (src/ui.rs:1687, src/ui.rs:3151-3159).
- **Band 2** holds the focused app's window tiles: icon and title. It is left-aligned in the card (src/ui.rs:1734-1737), always visible, and refilled on every Tab, because the card has "no collapse state anywhere" (src/ui.rs:1621-1626).
- **One tile everywhere:** 152 px wide, a 48-px icon, and an 88-px cell of badge row, icon, and title line (src/ui.rs:271-287). The card is 272 px tall: 2 × 40 padding, 2 × 88 cells, and a 16-px separator (src/ui.rs:300-305).
- **A tile title gets 136 px** (152 minus the 8-px inset on each side, src/ui.rs:313, src/ui.rs:1411), which is 11 columns of the 12-px bitmap floor (src/ui.rs:58).

The owner's proposal is smaller icon-only tiles, no names on tiles, and the selected window's name shown once, top-center. Most of the precedent does exactly that. The evidence argues for three adjustments: the second level should not be an always-on band, the icon should stay at 48 px while the tile shrinks, and the caption should name the commit target and span the card. Its top position is the one part with no precedent.

### 1.2 Recommendations

**R1. One strip: an app's windows replace its tile in place, and only after the user asks.**

- **Every grouped switcher in the survey reveals level 2 lazily.** GNOME waits 500 ms (`THUMBNAIL_POPUP_TIME`) or for ↓. Unity waits 1500 ms after opening and 500 ms after each step, and only for apps with at least two windows (`SetDetail(true, 2)`). macOS never shows an app's windows inside Cmd+Tab (§2.1, §2.2, §2.6). Airlock's band 2 is the only eager one: it shows level 2 the whole time and swaps its content at Tab speed. That gives the eye a second, changing list to ignore while it tracks the first. This is the likeliest source of the reported confusion, though it is a hypothesis with no user test behind it.
- **Wherever both levels share the screen, the children are tied to their parent.** GNOME centers its thumbnail row under the selected app icon (`thumbnailCenter = posX + icon.width / 2`). Unity moves the window spread onto the selected icon's slot and reflows the row around it. Windows and Plasma open a group from the group's own button: a hover flyout of previews, or a popup (§2.1-§2.3, §2.7). macOS sidesteps the question with separate gestures (§2.6). Airlock's band 2 is left-aligned, so nothing on screen ties it to its app.
- **In place beats anchored-below for Airlock.** It removes the below placement the owner found confusing and keeps one row to scan. Without thumbnails, an app's windows all draw the app's icon, so the run of identical icons is itself the group marker. Unity is the precedent (§2.2).
- **Explicit entry, no dwell timer.** Airlock's only motion is the selection fill and the overlay is drawn whole in its first frame (the `airlock-design` skill), so a timed reveal is out. A key-caused reflow is the same kind of change the vertical tree's → already makes.
- **At most one open group,** the focused app's. GNOME and Unity only ever show the selected app's windows (§2.1, §2.2). Tab away and the group closes, so the reflow is bounded to one tile becoming N and back.
- **No reflow animation.** Unity animates detail mode over 250 ms (`ANIMATION_LENGTH`); Airlock should not.

If two bands stay: anchor band 2 under its app tile, show it only after ↓, and make the bands differ in kind. GNOME's levels differ in kind for free (96-px icons with names above 256-px thumbnails with titles). Airlock has no thumbnails to provide that contrast, which is the main reason to prefer in place.

**R2. Icon-only tiles: shrink the tile and keep the 48-px icon.**

- **Icon-only switcher tiles use 64-128-px icons:** Gala 64, GNOME's window switcher in icon-only mode 96, Unity 128 em in 150-em tiles, KDE Large Icons 128, AltTab's App Icons style 70/110/150 (§2). The smallest icon-only app row in the survey is a persistent dock, Unity's launcher at 44 px. GNOME drops to 32 or 22 px only when the monitor is too narrow for 48, and keeps a name under every app icon when it does.
- **A candidate geometry:** a 48-px icon (`TILE_ICON` unchanged) in a 64-px cell (8-px inset for the selection fill) with an 8-px gap. That is a 72-px pitch against 160 today, 2.2 times denser. The card can stay at today's flat-band height of 168 px: badge row 12 + 4 + icon 48 + 12 + caption 12 = 88, with the one caption line taking the place of the per-tile title lines.
- **Chord hints don't fit.** `Ctrl+9` in the bitmap floor is 6 × 12 = 72 px, wider than a 64-px cell. Compact tiles need a digit-only corner badge or no per-tile hints (§1.3).

**R3. One caption: the commit target's title, at a fixed spot, across the card, with its line always reserved.**

- **Precedent:** all five single-caption switchers read in source (GNOME's window switcher, KDE Large Icons, Unity, AltTab's App Icons style, Gala) label only the selection (§2). Material Components does the same by default once a bar holds four or more items. NN/g argues the opposite for icons in general; §3.1 explains why app icons are the favorable case for icon-only and same-app windows the unfavorable one.
- **Content:** the title of the window that Alt-release or Enter would activate. On a window tile that is its own title. On an app tile it is the app's most-recently-used window title, because committing an app entry activates that window (docs/settings.md, "Grouping"). The icon already names the app, and siblings share one icon, so the title is the only thing that tells them apart. GNOME (a name under each app icon) and Unity (the app name as caption) show the app name at the app level instead; that is the documented alternative.
- **Width:** give the caption the card's width. A 9-tile compact strip is 688 px wide (`band_width` with 64-px tiles, src/ui.rs:309-311), so the caption gets about 640 px: 53 bitmap columns against today's 11. Floor the card width so a short strip doesn't starve the caption. Gala caps its caption at the icon row's width ("Ellipsize the label if it's longer than the icons"), which is the failure mode to avoid; KDE Large Icons floors its dialog at 30% of the screen width.
- **Truncation:** end ellipsis. Gala ellipsizes at the end, and Microsoft's guideline is that window titles put "the distinguishing information first". KDE Large Icons elides the middle (§5).
- **Stability:** reserve the caption line on every frame so the card never changes height.
- **Position:** every verified precedent puts the caption below the icons and none puts it on top. Top-center has no precedent in this survey, and nothing found says it hurts. Fixed (GNOME, KDE, Gala) and selection-following (Unity, AltTab) captions both ship; fixed fits Airlock's rule that the selection fill is the only thing that moves.

**R4. A count marker on multi-window apps only.**

Unity's switcher draws window pips only when `window_indicators > 1` and saturates at three. GNOME draws its arrow only under multi-window apps. dash-to-dock's dots cap at four (§2.1, §2.2, §2.8). Airlock's count chip already follows this rule (parents only), and under R1 it also previews how far the strip will reflow on entry. One caution, which is judgment with no source behind it: on macOS a number in an icon's corner reads as a notification count (AltTab draws the Dock's badge there, §2.6). The space under the icon, where GNOME draws its arrow, avoids that reading.

**R5. Keys: Tab steps apps, ` steps the selected app's windows, ↓/↑ enter and leave, ←/→ walk the strip.**

- **GNOME inside its popup:** the key above Tab selects window 0 of the selected app on the first press and steps its windows after that; Shift reverses; ↓ enters and ↑ leaves; Tab steps apps from anywhere (§2.1; app-grouping-treeview.md §3). AATWS documents the same model: the key above Tab will "iterate over windows of the selected application, Tab switches back to apps" (§4).
- **No COSMIC keybinding needed.** The overlay holds an exclusive keyboard grab for the whole gesture (switcher-behavior-adaptation.md §2), so Alt+` reaches Airlock directly. The standalone same-app session needs a custom shortcut (switcher-behavior-adaptation.md §8).
- **Conflict with type-to-filter.** ` is printable, and today every printable keysym types into the filter query (src/ui.rs:3632-3638). A standing query flattens the grouped card (src/daemon.rs:2031-2035). Binding ` means carving it out of the printable arm in grouped mode, the way the Ctrl+N chords already are. That is a spec decision.
- **Unity differs on Tab.** In Unity's detail mode, Tab walks the open app's windows and then moves on to the next app (§2.2). The strip already has ←/→ for the linear walk, so GNOME's split keeps two speeds: Tab for apps, ←/→ for tiles.

**R6. Keep grouping opt-in.** KWin (app-grouping-treeview.md §3), Windows Alt+Tab, and AltTab all default to per-window. AltTab's README pitches exactly that: "See every window, even within the same app." GNOME is the grouped-by-default outlier (§2, §5). Airlock's `grouping = false` default stands.

### 1.3 Decisions the evidence leaves to the owner

| Decision | Options | What the evidence says |
|---|---|---|
| Caption position | Top-center (the owner's ask); below the strip | Every verified precedent is below; no source found argues top vs bottom |
| Caption anchoring | Fixed center; follows the selected tile | Fixed: GNOME, KDE, Gala. Follows: Unity (clamped to the view), AltTab |
| Caption on an app tile | The MRU window's title; the app name | R3 argues for the title; GNOME and Unity show the app name |
| Level-2 placement | In place (Unity); anchored below on demand (GNOME) | Both are anchored and lazy; only Airlock's current band is neither |
| Chord hints on compact tiles | Digit-only corner badge; none; full `Ctrl+N` on the selected tile only | None of the switchers surveyed paints chord hints; cosmic-launcher paints `Ctrl + N` only in launcher mode (switcher-ux-and-icons.md §2.6) |
| Entry keys | ↓ and `; ↓ only | ` is the GNOME and AATWS precedent but collides with type-to-filter (R5) |

---

## 2. Prior art, per source

### 2.1 GNOME Shell: two levels, and the windows "pop down" under their app

Read from `js/ui/altTab.js` and `widgets/_switcher-popup.scss` on the GitHub mirror (`main`), plus GNOME Help.

- **Level 1 names every app.** Each `AppIcon` adds `new St.Label({text: this.app.get_name(), …})` under its icon. The icon size is the largest of `baseIconSizes = [96, 64, 48, 32, 22]` that fits the monitor (switcher-ux-and-icons.md §2.1).
- **An expand marker only where there is something to expand.** `_addIcon` draws a down arrow under each app and hides it when `appIcon.cachedWindows.length === 1`; multi-window apps get the Atk `EXPANDABLE` state.
- **Level 2 is a thumbnail row below, centered on its app.** `vfunc_allocate` sets `childBox.y1 = this._switcherList.allocation.y2 + spacing` and centers the row on `thumbnailCenter = posX + icon.width / 2`, clamped to the monitor. Every thumbnail carries its own title label (`ThumbnailSwitcher`), and thumbnails are 256 px wide (`.thumbnail { width: 256px; }`, commented "equal to THUMBNAIL_DEFAULT_SIZE in altTab.js"). The row appears after `THUMBNAIL_POPUP_TIME = 500` ms or on ↓.
- **GNOME Help says the same in user terms:** "Windows in the window switcher are grouped by application." "Previews of applications with multiple windows pop down as you click through." "Hold down Super and press ` (or the key above Tab) to step through the list."
- **The per-window switcher uses icon-only items and one caption at the bottom.** `WindowIcon` builds a title label but never adds it as a child. `WindowSwitcher` creates one centered `St.Label`, allocates it in a strip at the bottom (`childBox.y1 = childBox.y2 - labelHeight`), and `highlight()` sets it to the selected window's title. Icon-only mode draws `APP_ICON_SIZE = 96`; the thumbnail modes draw `WINDOW_PREVIEW_SIZE = 128`, with a 48-px app badge in "both" mode.
- **Keys inside the app popup** (`AppSwitcherPopup._keyPressHandler`): `SWITCH_GROUP` (the key above Tab) selects window 0 of the selected app when the thumbnails aren't focused, and the next window when they are; `SWITCH_GROUP_BACKWARD` selects the previous one. With the thumbnails focused, ←/→ step windows and ↑ returns to the app row. On the app row, ←/→ step apps and ↓ selects window 0.
- **Theme:** item spacing 12 px (`$base_padding * 2`), panel padding 12 px, and the selected item filled with the foreground at 20% (`transparentize($osd_fg_color, 0.8)`).

### 2.2 Unity 7: the selected app's tile opens in place

Read from `launcher/SwitcherView.cpp`, `launcher/SwitcherController.cpp`, `unity-shared/IconRenderer.cpp`, and `plugins/unityshell/unityshell.xml.in` on git.launchpad.net/unity.

- **Level 1:** app icons at `ICON_SIZE = 128_em` in `TILE_SIZE = 150_em` tiles, with `VERTICAL_PADDING = 45_em`. One bold caption (`text_view_`) shows the selected icon's tooltip text, which is the app name. It sits below the row, centered on the selected icon and clamped to the view (`start_x = arg.render_center.x - view_width / 2`), and is at most 3.5 tiles wide (`TEXT_TILE_MULTIPLIER`). It hides in detail mode (`SetVisible(!model->detail_selection)`).
- **Count pips inside the switcher:** `arg.window_indicators = icon->WindowsVisibleOnMonitor(monitor); if (arg.window_indicators > 1) arg.running_arrow = true; else arg.window_indicators = 0;`. Single-window apps get no marker. The shared renderer draws at most three (`int markers [3]`): one window gets an arrow, two get two pips, three or more get three.
- **Level 2 opens in place.** In detail mode the selected icon is skipped (`if (i == selection && detail_selection) arg.skip = true;`), the window spread is moved onto its slot (`OffsetRenderTargets(arg.render_center.x, arg.render_center.y)`), and the row gives up the spread's width (`max_width -= spread_padded_width - tile_size;`), so the other icons reflow. The selected window draws at alpha 1.0 and the rest at 0.9. Entry animates over `ANIMATION_LENGTH = 250` ms.
- **When it opens:** `detail_on_timeout(true)`, `initial_detail_timeout_length(1500)`, `detail_timeout_length(500)`. The timer re-arms on every selection change and only enters detail for apps with at least two windows (`SetDetail(true, 2)`, which checks `SelectionWindows().size() >= min_windows`).
- **Tab inside detail mode** (`TAB_NEXT_WINDOW`) moves to the next window until the last one, then calls `model_->Next()`, which selects the next app.
- **Bindings** (unityshell.xml.in): `alt_tab_forward` is `<Alt>Tab` and `alt_tab_forward_all` is `<Control><Alt>Tab`. `alt_tab_next_window` and `alt_tab_prev_window` exist with no default key in the metadata. The launcher's `icon_size` defaults to 44 (range 8-64).

### 2.3 KDE Plasma: no grouped switcher; grouping lives in the task manager

- **The window-switcher layouts ship in kdeplasma-addons** under `kwin/windowswitchers/`: `big_icons`, `compact`, `coverswitch`, `flipswitch`, `sidebar`. This closes a gap an earlier record left open: switcher-ux-and-icons.md §2.2 and §6 searched kwin and plasma-desktop, found only kwin's `thumbnail_grid`, and could not locate the classic layouts. They live in kdeplasma-addons.
  - **Large Icons** (`big_icons`, described as "A window switcher layout using large icons to represent the window"): `iconSize: Kirigami.Units.iconSizes.enormous` in a `GridView` that flows left to right. That is 128 px, inferred as the step above `huge` (64) on the Kirigami ladder recorded in switcher-ux-and-icons.md §2.2. Items have no labels. One shared label sits below the grid, bold, centered, and middle-elided: `text: icons.currentItem ? icons.currentItem.caption : ""`. The dialog is `min(max(0.3 × screen width, icons), 0.9 × screen width)` wide, and the highlight jumps without animation (`highlightMoveDuration: 0`).
  - **Compact:** a vertical list with a small icon and a caption on every row (`elide: Text.ElideMiddle`).
  - **Sidebar:** a vertical list 15% of the screen wide; each item is a window thumbnail with a medium icon and a caption heading of up to two lines.
- **None of the layouts groups by app.** KWin's only grouping control filters the list to one window per app (app-grouping-treeview.md §3).
- **The Task Manager groups** (plasma-desktop `applets/taskmanager/main.xml`): `groupingStrategy` defaults to 1, "By Program Name". `groupedTaskVisualization` defaults to 0, "cycle through grouped tasks"; the other values are 1 (tooltips with window previews), 2 (the Present Windows effect), and 3 ("show textual list (AKA group dialog)"). `groupPopups` defaults to true: "Whether groups are to be reduced to a single task button and expand into a popup". `onlyGroupWhenFull` defaults to true. The schema has no window-count badge option.

### 2.4 elementary Gala: icon-only tiles, one caption below

- **The tile is the icon.** `WindowSwitcherIcon` draws the app icon at `ICON_SIZE = 64` and nothing else; the window title is only its accessible name.
- **The caption sits below.** `lib/Widgets/AbstractSwitcher.vala` stacks the icon container (a horizontal `FlowLayout`) and then the caption in a vertical box. The caption is `ellipsize = END` and `line_alignment = CENTER`, and its width is capped at the icon row's ("Ellipsize the label if it's longer than the icons"). `WRAPPER_PADDING = 12`.
- **Caption text and the same-app mode** (`WindowSwitcher.vala`): the caption is the selected window's title (`caption_text = current_window != null ? current_window.title : "n/a"`). A `switch-group` binding collects only the active app's windows (`collect_current_windows`), which makes it a separate mode on the same switcher.

### 2.5 Cinnamon and Compiz: per-window, no app grouping in the presentation

- **Cinnamon** (`js/ui/appSwitcher/classicSwitcher.js`): every window is its own item with its own title label under its icon. `iconSizes = [96, 64, 48]`, thumbnails are 256 px, there is no app grouping, and the selection highlight slides (`translation_x`).
- **Compiz staticswitcher** (compiz-reloaded `metadata/staticswitcher.xml.in`): a popup of window thumbnails with a 48-px app-icon overlay (`popup_icon_size` default 48) and no title options. Its only app awareness is `next_group_key`, "Popup switcher if not visible and select next window of the current application", which has no default binding.

### 2.6 macOS and AltTab for macOS

- **macOS switches apps; one app's windows are separate gestures.** Cmd+Tab steps apps. Inside it you "press the Left or Right arrow key until you get to the app you want", and Esc or Period cancels (Apple, "Move and arrange app windows on Mac", fetched 2026-11-09; the page documents nothing else about the switcher). An app's windows are reached outside the strip: Command-Grave, "Switch between the windows of the app you're using…", and Control-Down Arrow for App Exposé (Apple 102650, carried from switcher-behavior-adaptation.md §4 and §11, verified 2026-10-31).
- **AltTab is per-window by default** (lwouis/alt-tab-macos `master`). `showAppsOrWindows` defaults to "Show all windows"; the alternative, "Show one window per app", filters the list down to each app's main window (`GroupAppsPreference.mainWindow`).
- **AltTab's styles:** "Thumbnails" (the default), "App Icons", and "Titles". `showTitles` defaults to "Window Title"; the alternatives are "Application Name" and "Application Name - Window Title".
- **AltTab's App Icons labels:** a tile's label shows only while the tile is focused or hovered (`label.isHidden = !shouldBeVisible`). It sits under the icon and may overflow the tile, up to `min(viewWidth * 2, TilesView.thumbnailsWidth)`.
- **No window counts in AltTab.** It draws no window-count badge; the badge it does draw at the icon's bottom-right is the Dock's notification label.
- **AltTab's pitch** (README): "See every window, even within the same app."

### 2.7 Windows

- **Alt+Tab is per-window.** Arrow keys are documented only for Ctrl+Alt+Tab (switcher-ux-and-icons.md §2.5). No first-party documentation of an app-grouped Alt+Tab turned up this pass.
- **Taskbar grouping is the default, and it is icon-only.** "Always, hide labels. This is the default setting. Each app appears as a single, unlabeled button, even when multiple windows for that app are open." "When taskbar is full" collapses only when crowded, and "Never" keeps "each window as an individual, labeled button" (Microsoft Support, "Customize the Taskbar in Windows", Windows 10 section).
- **Group members are opened from the group's button.** "Each member of a group is given a separate preview in the thumbnail flyout that is shown when the mouse hovers over the group's taskbar button." Vista drew a collapsed group "as a stack, but only the active window is shown". "If there are more thumbnails than can be displayed, the preview reverts to the legacy thumbnail or a standard icon." Microsoft also concedes that thumbnails struggle to tell documents apart: an app that "knows that its documents or tabs will appear similar when viewed at thumbnail size" may clip its thumbnail to the distinguishing part (Microsoft Learn, "Taskbar Extensions", ms.date 2018-05-31).
- **The Windows 7 UX guidelines for the taskbar** (Microsoft Learn, "Taskbar", ms.date 2020-10-20; Microsoft notes it was written for Windows 7 and not updated since):
  - "Each program (specifically, each program perceived as a separate program) should have a single taskbar button."
  - Icon-only commands are acceptable when icons are "recognizable and distinguishable … After initial use, users shouldn't have to rely on tooltips", with "a tooltip to label each command".
  - Window titles should be optimized "by concisely placing the distinguishing information first".
  - For status marks: "The eye is sensitive to changes in the peripheral field of vision, so status changes need to be subtle", and "Don't use animations."

### 2.8 Docks

- **dash-to-dock** (`appIconIndicators.js`) draws one running dot per window, capped at `MAX_WINDOWS_CLASSES = 4`; its `BINARY` style encodes up to 15. A click cycles windows by default, with previews behind a drop-down (app-grouping-treeview.md §3).
- **Unity's launcher** uses the shared renderer: an arrow for one window, two pips for two, three for three or more (§2.2), with the focused arrow on the other side.

### 2.9 The tiling-WM ecosystem

- **swayr:** `switch-window` is a menu (wofi by default) listing "urgent windows first, then windows in most-recently-used order, and the currently focused window last". The default `window_format` is icon, app name, quoted title, workspace, marks, and id: a text list, ungrouped. Same-app cycling goes through `next-matching-window`/`prev-matching-window`, which "take a criteria query" such as `app_id=…`; there is no dedicated same-app command.
- **i3ipc-python `focus-last.py`:** "Focus last focused window", a 15-deep MRU history toggled by a key, with no app or class filter.
- **Hyprland:** the dispatcher documentation could not be fetched (§7).

### 2.10 Raycast

- **Same-app cycling is its own command.** "Focus Next/Previous App Window: Cycle through the windows of the app you're using. Press repeatedly to reach each one." No overlay is involved (manual, Window Management).
- **"Switch Windows" is a flat, searchable launcher list:** it "lists every open window in a single searchable view, including multiple windows of the same app", with each result showing "the window's title alongside the app it belongs to" (manual, Navigation). It does not group.

### 2.11 Summary: how "one app, N windows" is presented

| Switcher | Level 1 item | One app's windows | Where they appear | When | Labels | Count marker |
|---|---|---|---|---|---|---|
| GNOME app switcher | 96-22 px icon with name | 256-px thumbnails, each titled | Row below, centered on the app | 500 ms dwell or ↓ | Every item | Arrow, multi-window apps only |
| GNOME window switcher | none (flat list) | 96-px icons or 128-px previews | Inline | Always | One caption, bottom, fixed | none |
| Unity | 128-em icon | Thumbnail spread | In the app's slot; the row reflows | 1500/500 ms dwell (2+ windows) | One caption below, follows the selection, hidden in detail | Pips if more than one window, max 3 |
| KDE Large Icons | none (flat list) | 128-px icons | Inline | Always | One caption below, fixed, middle-elided | none |
| Gala | none (flat list) | 64-px icons | Inline; `switch-group` is a separate mode | Always | One caption below, fixed, end-elided | none |
| AltTab, App Icons | none (flat by default) | 70-150-px icons | Inline | Always | Focused tile only, under it | none |
| macOS | App icon | Not in the strip | App Exposé or Cmd+` (separate) | On request | Not documented | none |
| Windows taskbar | Unlabeled app button (default) | One preview per window | Flyout from the button | Hover | On previews | A stack outline (Vista) |
| Plasma Task Manager | Task button | Cycle (default), previews, Present Windows, or list | Popup from the button | Click | Per entry | None in the schema |
| Airlock today | 48-px icon, app_id label, count chip | 48-px icon and title tiles | Separate band below, left-aligned | Always; swaps on every Tab | Every tile | Chip, parents only |

---

## 3. HCI and design-system guidance

### 3.1 A label on every item, or one for the selection

- **NN/g** (Aurora Harley, "Icon Usability", 2014-07-27): "Icon labels should be visible at all times, without any interaction from the user", and "Don't rely on hover to reveal text labels". Its argument concerns icons that stand for functions, where "most icons continue to be ambiguous to users", and it grants that icons are "fast to recognize at a glance (if well designed)".
- **Material Components for Android** (BottomNavigation docs): `labelVisibilityMode` defaults to auto. "The label behaves as 'labeled' when there are 3 items or less, or 'selected' when there are 4 items or more"; in "selected" mode only the current item shows its label.
- **Microsoft** (Windows 7 UX guidelines, §2.7): icon-only works when icons are "recognizable and distinguishable", with a tooltip label as backup.
- **What this means for Airlock.** The three sources agree on when icon-only works: familiar, distinguishable icons.
  - App icons in a switcher are that case: the user's own apps, brand icons, seen daily. Windows ships its default taskbar on exactly this (§2.7).
  - Sibling windows are the opposite case. One app's windows share one icon, so in Microsoft's terms they are not distinguishable. The caption is their only identifier, and picking one becomes a serial task: step, read, step.
  - The flat icon-only precedents accept that cost: GNOME's window switcher, KDE Large Icons, and Gala draw identical icons for same-app windows under one caption. Three things soften it: MRU order within the app puts the likeliest target first, a caption wide enough to read whole, and a fast step key (R5).

### 3.2 Telling windows apart: thumbnails or titles

- **Microsoft concedes** that documents "appear similar when viewed at thumbnail size" (§2.7).
- **No surveyed switcher relies on thumbnails alone.** GNOME titles every 256-px thumbnail, and KDE's thumbnail grid and Sidebar layouts caption every thumbnail (§2.1, §2.3; switcher-ux-and-icons.md §2.2).
- **Airlock cannot get thumbnails** as an external Wayland client (switcher-ux-and-icons.md §2.9). In every precedent the title is what tells windows apart, and with icon-only tiles it has to live in the caption.
- **The controlled study on this exact question** (Kaasten, Greenberg & Edwards 2002, on recognizing pages from titles, URLs, and thumbnails at several sizes) could not be read this pass (§7).

### 3.3 Grouping hierarchy

No controlled study was accessible, so this is shipped practice. Grouped presentations share three traits:

- **The children are tied to the parent:** centered below it, in its slot, or opened from its button.
- **They appear on demand,** not all the time.
- **The two levels look different:** GNOME's icons over thumbnails, the taskbar's button over previews.

In a vertical list the equivalent is the indent Airlock's tree already uses (app-grouping-treeview.md §5). The other family moves one app's windows into a separate mode: macOS Cmd+` and App Exposé, KWin's current-application walk, Gala's `switch-group`, and Raycast's Focus Next App Window (§4).

---

## 4. Within-app cycling: the key above Tab

| Platform | Default key | Shape | Source |
|---|---|---|---|
| GNOME | `switch-group`: Super or Alt + Above_Tab | Same popup as app switching; inside it, steps the selected app's windows, window 0 first | §2.1; switcher-behavior-adaptation.md §4 |
| KWin | Meta or Alt + ` ("Walk Through Windows of Current Application") | Separate TabBox mode | switcher-behavior-adaptation.md §4 |
| macOS | Command-Grave | Direct activation, no overlay | Apple 102650 (carried, §2.6) |
| Unity | `alt_tab_next_window` | Steps windows in the switcher; no default key in the plugin metadata | §2.2 |
| Gala | Mutter's `switch-group` binding | Same switcher, filtered to the active app | §2.4 |
| AATWS (GNOME extension) | The key above Tab | In app mode: "iterate over windows of the selected application, Tab switches back to apps" | AATWS README |
| Compiz staticswitcher | `next_group_key`, unbound | "select next window of the current application" | §2.5 |
| Raycast | User-assigned | "Focus Next/Previous App Window", no overlay | §2.10 |
| swayr | User-assigned | `next-matching-window` with an `app_id` criterion | §2.9 |
| Windows | None | Closest: Win+Ctrl+number, the last active window of a pinned app | switcher-behavior-adaptation.md §4 |
| COSMIC | None bound by default | n/a | switcher-behavior-adaptation.md §8 |

**Shipped two-level key maps:**

- **GNOME:** Tab steps apps from anywhere, including the thumbnail row; ` steps the selected app's windows; ↓ enters and ↑ leaves; ←/→ move within whichever level has focus.
- **Unity:** Tab steps apps; inside detail mode, Tab walks the windows and then moves on to the next app.
- **AATWS:** the GNOME model, stated in its README.

Airlock's grouped-horizontal key map is already the GNOME model minus the key above Tab: Tab steps apps, ←/→ step within the focused band, ↓/↑ enter and leave band 2 (src/ui.rs:3556-3562).

App grouping in alt-tab is contested. KWin, Windows, and AltTab default to per-window. GNOME's grouped default draws persistent complaints and "rebind Alt+Tab to switch windows" advice, but the pages that document them could not be read first-hand this pass (§7).

---

## 5. Where sources disagree

1. **Labels:** always visible (NN/g) or selection-only (Material's default at four or more items, and every icon-strip switcher). §3.1 resolves it by how familiar and distinguishable the icons are, without declaring a winner.
2. **Caption anchoring:** fixed center (GNOME, KDE, Gala) or following the selection (Unity, clamped; AltTab).
3. **Caption truncation:** end (Gala; KDE's thumbnail grid elides right) or middle (KDE Large Icons and Compact).
4. **Where level 2 goes:** below and centered (GNOME), in the app's slot (Unity), or a separate mode (macOS, KWin, Gala, Raycast).
5. **Tab inside level 2:** always apps (GNOME) or windows and then the next app (Unity).
6. **How a count is shown:** a binary arrow (GNOME); marks saturating at 3 (Unity) or 4 (dash-to-dock); a stack outline (the Vista taskbar); nothing (AltTab, Plasma's task manager schema); or digits (Airlock today).
7. **Caption content at app level:** the app name (Unity; GNOME's per-icon names) or the target window's title (R3).
8. **Grouped by default:** GNOME yes; KWin, Windows Alt+Tab, and AltTab no.

---

## 6. Sources

All fetched 2026-11-09 unless marked.

**GNOME**

- gnome-shell `js/ui/altTab.js` (`main`, GitHub mirror): AppIcon labels, `_addIcon` arrow and `EXPANDABLE`, thumbnail allocation below and centered, per-thumbnail titles, the WindowIcon/WindowSwitcher bottom caption, constants, `_keyPressHandler`. https://raw.githubusercontent.com/GNOME/gnome-shell/main/js/ui/altTab.js
- gnome-shell `data/theme/gnome-shell-sass/widgets/_switcher-popup.scss`: item spacing, 256-px thumbnail width, selected fill. https://raw.githubusercontent.com/GNOME/gnome-shell/main/data/theme/gnome-shell-sass/widgets/_switcher-popup.scss
- GNOME Help, "Switch between windows": grouped by application, previews "pop down", the key above Tab. https://help.gnome.org/users/gnome-help/stable/shell-windows-switching.html.en

**Unity 7**

- `launcher/SwitcherView.cpp`: sizes, caption placement, the in-place detail spread, switcher pips. https://git.launchpad.net/unity/plain/launcher/SwitcherView.cpp
- `launcher/SwitcherController.cpp`: detail timeouts, the two-window minimum, `TAB_NEXT_WINDOW`. https://git.launchpad.net/unity/plain/launcher/SwitcherController.cpp
- `unity-shared/IconRenderer.cpp`: at most three markers. https://git.launchpad.net/unity/plain/unity-shared/IconRenderer.cpp
- `plugins/unityshell/unityshell.xml.in`: switcher bindings, launcher icon size. https://git.launchpad.net/unity/plain/plugins/unityshell/unityshell.xml.in

**KDE**

- kdeplasma-addons `kwin/windowswitchers/` listing. https://api.github.com/repos/KDE/kdeplasma-addons/contents/kwin/windowswitchers
- Large Icons layout and metadata. https://raw.githubusercontent.com/KDE/kdeplasma-addons/master/kwin/windowswitchers/big_icons/contents/ui/main.qml and https://raw.githubusercontent.com/KDE/kdeplasma-addons/master/kwin/windowswitchers/big_icons/metadata.json
- Compact layout. https://raw.githubusercontent.com/KDE/kdeplasma-addons/master/kwin/windowswitchers/compact/contents/ui/main.qml
- Sidebar layout. https://raw.githubusercontent.com/KDE/kdeplasma-addons/master/kwin/windowswitchers/sidebar/contents/ui/main.qml
- plasma-desktop Task Manager schema. https://raw.githubusercontent.com/KDE/plasma-desktop/master/applets/taskmanager/main.xml

**elementary**

- gala `src/Widgets/WindowSwitcher/WindowSwitcherIcon.vala`. https://raw.githubusercontent.com/elementary/gala/main/src/Widgets/WindowSwitcher/WindowSwitcherIcon.vala
- gala `src/Widgets/WindowSwitcher/WindowSwitcher.vala`. https://raw.githubusercontent.com/elementary/gala/main/src/Widgets/WindowSwitcher/WindowSwitcher.vala
- gala `lib/Widgets/AbstractSwitcher.vala`. https://raw.githubusercontent.com/elementary/gala/main/lib/Widgets/AbstractSwitcher.vala

**Cinnamon and Compiz**

- cinnamon `js/ui/appSwitcher/classicSwitcher.js`. https://raw.githubusercontent.com/linuxmint/cinnamon/master/js/ui/appSwitcher/classicSwitcher.js
- compiz-reloaded `metadata/staticswitcher.xml.in`. https://raw.githubusercontent.com/compiz-reloaded/compiz-plugins-main/master/metadata/staticswitcher.xml.in

**macOS**

- Apple, "Move and arrange app windows on Mac" (macOS 27). https://support.apple.com/guide/mac-help/work-with-app-windows-mchlp2469/mac
- Apple, "Mac keyboard shortcuts" (102650): Command-Grave and Control-Down Arrow. Carried from switcher-behavior-adaptation.md, verified 2026-10-31; HTTP 429 on 2026-11-09. https://support.apple.com/en-us/102650
- AltTab `src/preferences/Preferences.swift`. https://raw.githubusercontent.com/lwouis/alt-tab-macos/master/src/preferences/Preferences.swift
- AltTab `src/preferences/MacroPreferences.swift`: the user-facing enum strings. https://raw.githubusercontent.com/lwouis/alt-tab-macos/master/src/preferences/MacroPreferences.swift
- AltTab `src/switcher/main-window/TileView.swift`: focused-only labels, badge placement. https://raw.githubusercontent.com/lwouis/alt-tab-macos/master/src/switcher/main-window/TileView.swift
- AltTab README. https://raw.githubusercontent.com/lwouis/alt-tab-macos/master/README.md

**Microsoft**

- "Customize the Taskbar in Windows": the default "Always, hide labels". https://support.microsoft.com/en-us/windows/customize-the-taskbar-in-windows-0657a50f-0cc7-dbfd-ae6b-05020b195b07
- "Taskbar Extensions" (ms.date 2018-05-31): per-window previews per group, the stack representation, thumbnail similarity. https://learn.microsoft.com/en-us/windows/win32/shell/taskbar-extensions
- "Taskbar", Windows 7 UX guidelines (ms.date 2020-10-20). https://learn.microsoft.com/en-us/windows/win32/uxguide/winenv-taskbar

**Docks, tiling WMs, launchers**

- dash-to-dock `appIconIndicators.js`. https://raw.githubusercontent.com/micheleg/dash-to-dock/master/appIconIndicators.js
- swayr README. https://git.sr.ht/~tsdh/swayr/blob/main/README.md
- i3ipc-python `examples/focus-last.py`. https://raw.githubusercontent.com/altdesktop/i3ipc-python/master/examples/focus-last.py
- AATWS README. https://raw.githubusercontent.com/G-dH/advanced-alttab-window-switcher/main/README.md
- Raycast manual, Window Management. https://manual.raycast.com/window-management
- Raycast manual, Navigation. https://manual.raycast.com/navigation

**HCI and design systems**

- NN/g, Aurora Harley, "Icon Usability" (2014-07-27). https://www.nngroup.com/articles/icon-usability/
- Material Components for Android, BottomNavigation docs. https://raw.githubusercontent.com/material-components/material-components-android/master/docs/components/BottomNavigation.md

**Airlock (this repository)**

- src/ui.rs:58, src/ui.rs:61 (text cell), src/ui.rs:271-318 (tile tokens, band and combined geometry), src/ui.rs:1406-1416 and src/ui.rs:3151-3159 (tile titles and the app_id fallback), src/ui.rs:1616-1750 (`overlay_model_grouped_horizontal`), src/ui.rs:2022-2048 (two-band render), src/ui.rs:3525-3536, src/ui.rs:3556-3562, and src/ui.rs:3632-3638 (`KeyMode`, `key_input`, the printable arm)
- src/daemon.rs:2031-2035 (a query flattens the grouped card)
- docs/settings.md, "Grouping" (committing an app entry activates its MRU window)
- Earlier records: [app-grouping-treeview.md](app-grouping-treeview.md), [switcher-ux-and-icons.md](switcher-ux-and-icons.md), [switcher-behavior-adaptation.md](switcher-behavior-adaptation.md)

---

## 7. Unverified

- **Academic studies, not read.** Kaasten, Greenberg & Edwards, "How People Recognise Previously Seen Web Pages from Titles, URLs and Thumbnails" (People and Computers XVI, 2002): PDF only, which the fetch tool cannot read. Secondary summaries say about 80% of pages were recognized at "appropriate" thumbnail sizes; that is not verified and not used. Tak, Cockburn et al., "Improving Window Switching Interfaces" (INTERACT 2009), and Tak's 2011 thesis "Understanding and Supporting Window Switching": no reachable abstract (Springer redirects to a login, Crossref and OpenAlex carry no abstract, Semantic Scholar answered 429). Wiedenbeck 1999 on icons and labels (Taylor & Francis 403). Everett & Byrne 2004 on icon spacing and visual search (ACM 403). The recommendations rest on shipped practice and design-system guidance only.
- **Apple.** Mission Control's "Group windows by application" option, Stage Manager's per-app window grouping, the Dock's window list, and whether ↑/↓ inside Cmd+Tab opens App Exposé: every Apple page answered 429 this pass. That Cmd+Tab shows the app name only under the selected icon is widely observed, but Apple documents no visuals (switcher-ux-and-icons.md §2.3).
- **Windows 11.** No first-party documentation of an app-grouped Alt+Tab was found. The Multitasking "Alt+Tab" setting for browser tabs appeared only in search summaries. No primary source was fetched for Windows 7/8 Alt+Tab or Flip 3D visuals.
- **GNOME's design rationale** for app-based Alt+Tab, and the complaint threads: gnome-shell issue #6905 ("Alt-Tab brings all the windows of an app to the front") and a Super User thread on disabling the grouping. GNOME's GitLab answered 406 and wiki.gnome.org 403, so only their titles are known, from search results.
- **Kirigami's `enormous` icon size** is taken as 128 px from the ladder recorded in switcher-ux-and-icons.md §2.2; Kirigami's `units.cpp` was not re-read this pass.
- **Hyprland** dispatchers (`cyclenext`, `focuswindow class:`) and hyprswitch: the wiki has moved and both hosts and the wiki repo path returned 404.
- **Witch, Contexts, HyperSwitch** (macOS): not surveyed.
- **Unity's detail-mode key:** the controller has `StartDetailMode`, but its key binding is not in the plugin metadata read.
- **Line references** into Airlock are as of 2026-11-09 and will drift.
