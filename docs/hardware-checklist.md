# Hardware verification checklist

Airlock is a COSMIC Wayland app: runtime behavior can't be exercised
headless. This is the checklist a human runs on a real COSMIC session —
after meaningful changes, and always before calling a build release-ready.

Setup: a COSMIC session with windows on **at least two workspaces**, one of
them sticky, at least one minimized, and at least one application with two
windows on the current workspace (a second terminal is enough). Build and
install first (`just install`), so the presses below exercise the installed
binary.

## 1. State layer

- [ ] `airlock list` prints every toplevel with title, app id, state flags,
      and workspace set, and names the active workspace
- [ ] Output matches what the dock/overview shows for the same windows

## 2. Terminal cycling and isolation

- [ ] `airlock alt-tab` focuses the next window of the **current** workspace
      in most-recently-used order; `airlock shift-alt-tab` reverses
- [ ] A window parked on another workspace is never focused, no matter how
      many presses
- [ ] The sticky window appears in the list on every workspace; minimized
      windows never do

## 3. Gesture (with the takeover installed)

- [ ] Hold Alt, tap Tab repeatedly — selection walks the list in MRU order;
      Shift+Tab reverses
- [ ] Releasing Alt focuses the selected window and dismisses the overlay
- [ ] Escape dismisses without switching
- [ ] A quick Alt+Tab tap (released before the overlay appears) still
      switches
- [ ] Two rapid invokes in a row behave as one continuous gesture session

### Gesture keys (Enter and arrows)

- [ ] While the overlay is up, Enter commits mid-hold — the selection
      focuses and the overlay unmaps, exactly as if Alt had been released
- [ ] Arrows step the selection both directions: Up/Left back, Down/Right
      forward, wrapping at the list ends like Tab
- [ ] Regression: Escape still dismisses without switching
- [ ] Regression: Alt release still commits the selection
- [ ] Regression: a quick Alt+Tab tap still switches, no lingering overlay
- [ ] Regression: Shift+Tab still walks back

### Pointer select (hover, click, wheel)

- [ ] Rest the cursor where the card will appear, then Alt+Tab — the
      selection does not jump to the row under it, and jitter inside
      that row moves nothing either
- [ ] The cursor turns to the pointing hand the moment it enters the
      card, both layouts and both themes, and returns to the arrow the
      moment it leaves
- [ ] Crossing rows walks the highlight one row per crossing; parking on
      a row and releasing Alt focuses it
- [ ] Hover a row, then press Tab — the step continues from the hovered
      selection, not from wherever the keyboard left it
- [ ] Leaving the card and returning to the same row re-selects it
- [ ] Click each row of a 5+-window list mid-hold — the clicked window
      focuses, the overlay unmaps, and a later Alt release is inert
- [ ] Repeat the click pass at 200% scale — clicks still land on the
      clicked row
- [ ] A click with the cursor off the card changes nothing; Alt release
      still commits the selection
- [ ] The wheel steps the list one row per notch, both directions,
      wrapping at the ends; a same-app overlay wheels within its own
      filtered list
- [ ] Rapid multi-notch flicks land exactly one row per detent — a
      detent's machine-made echo (virtualized mice in the COSMIC VM were
      observed delivering one click twice, 1 ms apart) steps nothing,
      while real repeats tens of ms apart all step
- [ ] A high-resolution wheel or trackpad scroll (deltas split below one
      notch) advances the selection linearly — total rotation, not per
      frame speed, decides the steps; slow and flicks agree
- [ ] A slow mouse scan holds the overlay open (no 2 s idle close); the
      30 s hard close still applies
- [ ] The cursor stays visible over the card
- [ ] Regression: Tab / Shift+Tab / arrows / Enter / Escape / Alt
      release / quick tap behave exactly as before

### Number select (Ctrl+1..9 and the row hints)

- [ ] Hold Alt, tap Tab, press Ctrl+3 — the third row's window focuses,
      the overlay unmaps, and a later Alt release is inert
- [ ] The keypad digits do the same: Ctrl with keypad 3 reaches the third
      row
- [ ] Ctrl+9 with fewer than nine windows listed changes nothing — the
      overlay stays up and Tab still steps
- [ ] A number key without Ctrl does nothing mid-hold
- [ ] In an Alt+grave overlay, Ctrl+N addresses the Nth row of that
      application's list
- [ ] Rows one to nine show their digits `1` … `9` at the right edge,
      legible on both themes at 100% and 200% scale over a light and a
      dark wallpaper; with ten or more windows, rows beyond the ninth
      show nothing
- [ ] The selected row's hint stays legible over the highlight; a long,
      truncated title stops short of its hint
- [ ] Regression: Tab / Shift+Tab / arrows / Enter / Escape / Alt
      release / quick tap / hover / click / wheel behave exactly as before

### Same-app and quick toggle

- [ ] Alt+grave opens the overlay listing only the current application's
      windows, MRU order; Tab walks them; Alt release commits
- [ ] Alt+Shift+grave opens the same list stepping backwards
- [ ] An application with a single window in scope is a silent no-op — no
      overlay, no switch
- [ ] On a default isolated workspace the list holds that application's
      windows here; on one with `isolated = false`
      ([settings](settings.md)), that application's windows everywhere
- [ ] `airlock quick-toggle`, run by hand, flips to the MRU-second window
      with nothing rendered; repeated runs ping-pong between the top two
      (the command keeps working with no key bound to it)
- [ ] Regression: Alt+Tab still lists every window in scope, not only the
      current application's

### All windows (Alt+Esc)

- [ ] On a workspace whose rules isolate it, Alt+Esc opens the card over
      every non-minimized window on every workspace, MRU order — the
      rules cannot veto this invoke; Tab and the arrows walk it
- [ ] Alt release commits to a window parked on another workspace: the
      switch lands there
- [ ] Alt+Shift+Esc opens the same list stepping backwards
- [ ] Escape mid-gesture dismisses without switching; Alt+grave and plain
      Alt+Tab behave exactly as before on the same workspace
- [ ] With `all_windows_key = "Tab"`
      ([settings](settings.md)) and `airlock install` re-run, the family
      follows the configured key (Alt+Tab / Alt+Shift+Tab here)

### Workspace label

- [ ] With `show_workspace = true`
      ([settings](settings.md)), the card names the workspace you pressed
      on — small, top-left on the vertical cards, bottom-center on the
      horizontal band and the grouped strip
- [ ] An unnamed workspace shows the synthesized "# Workspace N" at
      the same position an `index = N` rule matches; a workspace whose
      name is just digits (COSMIC's numeric default) shows the same
      form carrying its digits, and a named workspace shows its name
      verbatim — the `#` mark paints on both text faces
- [ ] A workspace name long enough to hit its cap truncates with `…` and
      never widens the card; on the horizontal cards a full-length caption
      keeps clear of the label
- [ ] Typing a filter with the label showing keeps both lines (label
      above the query strip on a vertical card); the no-matches card
      keeps the label line too, so the card never jumps when the list
      empties. On a horizontal card a click on the label line addresses
      no tile
- [ ] Editing the setting mid-gesture never repaints the open card; the
      next press applies it
- [ ] With the setting off (or removed), every card renders exactly as
      before — no reserved line, no height change

### Overlay visuals (icons, card, HiDPI)

- [ ] Six or more applications show their real themed icons in the rows,
      including an SVG-only icon (Firefox) and a Flatpak application
- [ ] An application with no themed icon shows the generic window glyph,
      and the daemon log names it once (`no icon for app_id …`)
- [ ] The card reads on a **light** and a **dark** wallpaper: hairline
      border and rounded corners visible and clean on both (screenshot
      each)
- [ ] The selected row's inset highlight is legible on both wallpapers
      and walks the rows at Tab speed
- [ ] Set the display to 200% scale: text and icons stay crisp (no
      upscaled blur), the card stays centered
- [ ] At 100% scale the titles read at a glance — the 12-px re-pin
      (2026-11-07); the icons still carry the row, the title confirms
- [ ] A 48+-character title truncates with `…`; an untitled window shows
      `(untitled)`

### Rows (list density)

- [ ] A 20+-window list at the default: eight windows in view mid-list,
      nine at either end, `…` rows at the truncated edges
- [ ] Step the list end to end: the card's height never changes while
      the window moves, and the selection never lands on a `…` row
- [ ] `rows = 10`, then `rows = 3`: the next Alt+Tab keeps that many
      windows in view, no restart; a flip mid-gesture windows nothing
      until the next press
- [ ] `rows = 11` and `rows = "8"` each log a `settings:` diagnostic
      naming `rows`; switching keeps working on the last-good value
- [ ] Pointer and Ctrl+N on a `…` row are inert: no selection move, no
      switch; the next Tab still steps
- [ ] At 1× and 2×, both themes, over a light and a dark wallpaper: the
      `…` rows read at a glance (screenshot each)
- [ ] A list of ten or fewer at the default — any list within `rows` —
      matches the card before windowing: same height, no `…` rows
- [ ] Same-app cycling (Alt+grave) on one app with more than `rows`
      windows open windows the same way

### Layout (the horizontal card)

- [ ] `layout = "horizontal"` on a 20+-window list: a wide band of
      tiles at 1× and 2×, both themes, over a light and a dark
      wallpaper — each tile the app icon with its clamped title, the
      trailing `…` tile marking the truncated edge (screenshot each)
- [ ] Left/Right step the band; Tab/Shift+Tab and the wheel step the
      same sequence; Up/Down still step (aliases); stepping past the
      edge scrolls just enough — the selection never lands on the `…`
      tile
- [ ] Ctrl+N: the first nine tiles show their digit badges in reading
      order and commit instantly; a tile past nine answers no chord and
      shows none; hover moves the selection per tile, a click commits
      it, and the `…` tile is inert to pointer and chords alike
- [ ] `columns = "auto"` fits two-thirds of the invoking output's
      logical width (never fewer than three tiles) — check on two
      different output sizes; `columns = 3`, then `columns = 12`,
      applies on the next press, no restart; `rows` changes nothing on
      the band
- [ ] `layout = "vertical"` restores the tall card unchanged; the
      quick tap, Escape, and the 2-s idle close behave exactly as on
      the vertical card
- [ ] `grouping = true` with `layout = "horizontal"` renders the
      grouped card — vertical — unchanged
- [ ] `layout = "side"` and `columns = 2` each log a `settings:`
      diagnostic naming their key; switching keeps working on the
      last-good value

### Geometry (style and density follows)

- [ ] Flip COSMIC's Appearance style Round → Slightly-Round → Square;
      each next Alt+Tab follows — corners at 12 / 8 / 2 px, the
      selection band rounding with the card (capped at 8), Square's 2
      reading as a deliberate hair of rounding, not a bug
- [ ] Flip COSMIC's interface density Compact → Comfortable → Spacious;
      each next Alt+Tab follows — rows 40 / 48 / 56 px, icon-driven at
      every step, the card's width never moving
- [ ] Both follows at 1× and 2×, both themes, over a light and a dark
      wallpaper (screenshot each cell — the 3×3 sheet renders headless:
      `cargo run --example overlay-preview -- /tmp/geometry.png
      --geometry`)
- [ ] A style or density flip mid-gesture changes nothing until the
      next press — the open card keeps its shape under held Alt
- [ ] Round + Comfortable (COSMIC's factory default) renders the card
      exactly as before the follow — no visible change for the default
      desktop
- [ ] Spacious + `rows = 10`: the daemon log names the clamp (density,
      text size, setting, clamped value — eight), the card fits the
      panel, and the setting keeps its value after the density drops
      back
- [ ] Move the COSMIC stores aside (rename `~/.config/cosmic`); the
      next Alt+Tab paints the factory-default geometry with one
      aggregated diagnostic per follow, and switching is unaffected

### Font (typography)

- [ ] No `font` key: the card paints the COSMIC interface font at
      12 px — Open Sans on the default desktop — at 1× and 2×, both
      themes, over a light and a dark wallpaper
- [ ] Flip COSMIC Settings' own interface font; the next Alt+Tab
      follows (the follow is the key's default)
- [ ] `font = "<an installed family>"` paints it at the next gesture,
      live-apply without restart; an edit mid-gesture never repaints
      the open card (frozen until the next press)
- [ ] CJK, emoji, and accented title rows render their real glyphs —
      fallback chains, no folding, no `?` (screenshot each)
- [ ] A 64-char title truncates at the pixel budget with the font's
      real `…`, a full gap clear of the hint band; the hint band
      aligns on the new metrics
- [ ] `font = "Not Installed"`: the daemon log names the key, the card
      paints the interface font, switching is unaffected
- [ ] A wrong-type `font` (`font = 12`) fails the file with the named
      diagnostic and last-good holds
- [ ] No loadable fonts on the guest (move the font directories
      aside): the card paints the built-in bitmap face — the floor —
      and switching still works
- [ ] Regression: Tab / arrows / Enter / Escape / pointer input / Alt
      release behave exactly as before; no added latency on open
      (shaping and masks are cached; the scan is startup-only)

### Font size

- [ ] No `font_size` key: the card is the 12-px default, byte-for-byte
      the same as before the key (the byte-identity regression)
- [ ] `font_size = 8`, `16`, and `24` at 1× and 2×, both themes, over a
      light and a dark wallpaper: titles, query strip, hint digits, and
      the band's caption and badges all scale; rows grow above 12 px;
      hints and the parent caret never collide with titles
- [ ] At 24 px a max-width title elides sooner than at 12 px and the
      card does not widen past its ~600-px title band
- [ ] At 24 px the parent pips keep their clearing under the title in
      the vertical grouped card; the band's pips and badges stay clear
      of the icon and caption
- [ ] At 24 px + Spacious, the default `rows = 8` clamps (to six): the
      daemon log names density, text size, setting, and clamp; the card
      fits the 720-px panel and the setting keeps its value
- [ ] An edit mid-gesture never repaints the open card; the next press
      applies the new size
- [ ] `font_size = 30` (or a non-integer) fails the file with the named
      diagnostic and last-good holds
- [ ] No loadable fonts: the bitmap face scales with `font_size` too

### Theme (card color)

- [ ] `theme = "light"` on a dark-mode desktop: the next Alt+Tab paints
      the light card; `theme = "dark"` paints the dark card on a
      light-mode desktop
- [ ] `theme = "auto"` follows the desktop: flipping COSMIC's dark mode
      (Settings or a schedule) shows on the next Alt+Tab, both
      directions; a flip mid-gesture does not repaint the open card
- [ ] The light card reads on a light and a dark wallpaper: border,
      corners, and selection legible on both (screenshot each); repeat
      at 200% scale
- [ ] The dark card is unchanged from pre-theming builds (visual compare
      against an older screenshot)
- [ ] With `theme = "auto"`, rename
      `~/.config/cosmic/com.system76.CosmicTheme.Mode/v1/is_dark` — the
      next Alt+Tab paints dark and the daemon log carries one
      diagnostic; restore the file
- [ ] Edit `theme` while the daemon runs: the next press reflects it, no
      restart
- [ ] Regression: Tab / Shift+Tab / arrows / Enter / Escape / pointer
      input / Alt release behave exactly as before; no added latency on
      open
- [ ] Re-measure daemon RSS (section 7) — unchanged from the reference
      row

### Grouping (app tree)

- [ ] Set `grouping = true` on a workspace with 3+ apps — one
      multi-window, one single-window, and one un-attributable window if
      available: the card opens on collapsed app entries plus top-level
      rows, each entry carrying window-count pips below the app name and
      an inline right-pointing caret, each row's Ctrl+N key shown as a
      bare digit at the right edge (children show none)
- [ ] Tab / Shift+Tab, ↑/↓, and the wheel are one linear walk over the
      visible rows, wrapping: entries, leaves, and expanded groups'
      windows together; the walk lands on an expanded entry like any
      row; Alt+W does nothing
- [ ] → on an entry expands it and enters — landing one past the window
      you just left when it is your own app — and the caret turns
      down; ← on a child returns to the entry with the group still
      expanded; ← on the expanded entry collapses it
- [ ] ` (the key above Tab) on a collapsed app entry opens that app's
      windows — plain ` on their most recently used one, Shift+` on the
      last — and the group expands for it exactly as → does, the state
      persisting; on the expanded entry it collapses in place; from a
      window row it cycles the same app's windows, wrapping, never
      crossing into another, and the group stays open while it cycles;
      on a single-window app it holds without dead ends; the grouped
      strip obeys the same key from its tiles, and with a query
      standing the key types instead
- [ ] The strip (`grouping = true` with `layout = "horizontal"`): Tab
      from an app tile steps app tiles; from inside an open run it
      steps the run's windows like ←/→ — the run staying open until
      the walk steps off its edge, never closing on the first press
- [ ] Nothing descends into a collapsed group: a workspace holding a
      single collapsed entry keeps the selection under repeated Tab
- [ ] Ctrl+N activates the Nth app entry's window instantly; expanded
      children show no chord hint
- [ ] Releasing Alt on an entry (or Enter) activates that app's most
      recently used window; a quick tap lands one window past the one
      you left when your group is expanded, on the next app entry when
      it is collapsed
- [ ] Expansion persists across gestures: expand an app, release Alt,
      Alt+Tab again — the group is still expanded (until the daemon
      restarts)
- [ ] A tree taller than `rows`: the `…` windowing rides the flattened
      sequence; Ctrl+N and the pointer are inert on the dots
- [ ] Pointer on the tree: hover and click select rows — an expanded
      entry commits its front window like any row
- [ ] `grouping = false` regression: the flat card and every key behave
      exactly as before
- [ ] Edit `grouping` while the daemon runs: the next Alt+Tab uses it;
      `grouping = 1` logs a `settings:` diagnostic naming the key and
      the last-good value holds; a config still carrying the removed
      `scoped_cycling` key parses fine (ignored unknown)

### Type-to-filter

- [ ] Hold Alt+Tab on a workspace with mixed apps; type a word from one
      title: the card narrows to its matches, the query strip above the
      rows showing what you typed with a caret; the first match is
      selected
- [ ] Typing filters on title *and* app name (type an app's name); the
      match is case-insensitive
- [ ] With `grouping = true`: typing flattens the card to matching
      windows — collapsed groups' windows match all the same; clearing
      the query restores the tree with the kept window selected
- [ ] Alt+grave (same-app): typing filters within that app's windows
- [ ] Tab / arrows / wheel step the matches only; Enter and Alt-release
      commit the selected match
- [ ] Ctrl+N still instant-commits over the filtered rows; plain digits
      type
- [ ] Backspace edits; on an empty query it does nothing — never
      dismisses
- [ ] Escape clears a standing query first (the card restores, selection
      kept when live); the second Escape dismisses
- [ ] A query matching nothing keeps the card with the "no matches"
      verdict; Enter commits nothing; Backspace recovers
- [ ] A 65th typed character is ignored; an overlong query shows its
      tail in the strip with a leading ellipsis
- [ ] Regression: without typing, every card and key behaves exactly as
      before

### Closing windows

- [ ] Flat card: walk to a window you can spare, Alt+minus (top-row) —
      it closes, the selection lands on the next visible row (wrapping),
      the card stays up for another close, and the closed row leaves
      the card the moment the window goes — no further key or pointer
      input needed
- [ ] Keypad minus closes the same; Shift+minus (underscore) types into
      the query (starting one if none stands) — never closes
- [ ] Repeated closes walk the card down; closing the last eligible
      window dismisses the overlay (nothing left to switch to)
- [ ] `grouping = true`: Alt+minus on a collapsed entry closes that
      app's front window — the window the entry commits; the card
      recomposes (the count pips follow the count, saturating at three;
      the entry itself goes when its last window closes)
- [ ] Expanded group: the × badge reveals on a child row's icon under
      the pointer — never on the entry itself — and clicking a child's
      × closes that window
- [ ] The × reveals only while the pointer rests on a window row's
      icon; the row's key-hint band stays put (the chord no longer
      yields), and headers, `…` rows, and the pointerless card never
      show the badge
- [ ] Click split: a click on the × badge closes; a click anywhere else
      on the row (including the rest of the icon) activates as before
- [ ] Regression, `show_workspace = true` on the vertical card: the ×
      reveals below the workspace label and a click on the badge still
      closes — never activates — the row it sits on
- [ ] Horizontal band and grouped strip: the same × badge reveals on a
      hovered window tile's icon at its top-right corner, clicking the
      badge closes that tile's window, and clicking the rest of the
      icon still activates; app tiles never show the × (Alt+minus still
      closes their front window), and the `…` tile never shows it
- [ ] With a standing query: closing a match narrows the card within
      the matches; the query survives the close
- [ ] A window someone else closes mid-gesture drops off the card the
      same way (no × needed); closing never adds a window back
- [ ] Regression: without closing anything, every card, key, and click
      behaves exactly as before

## 4. Takeover and restore

- [ ] `airlock install` redirects Alt+Tab / Shift+Alt+Tab to Airlock at user
      level; the override survives re-login; `/usr/share` is untouched
- [ ] `airlock install --uninstall` brings the previous switcher back
      exactly (diff `~/.config/cosmic/com.system76.CosmicSettings.Shortcuts/v1/system_actions`
      before install and after uninstall: byte-identical)
- [ ] `airlock install` also adds four marked custom bindings — Alt+grave,
      Alt+Shift+grave, the all-windows pair Alt+Esc / Alt+Shift+Esc — to
      the sibling `custom` file; a key already bound by one of your
      shortcuts is skipped with a diagnostic, never clobbered
- [ ] A machine upgraded from 0.4: the re-run of `airlock install` that
      applies this release reports the old `Escape → airlock
      quick-toggle` entry as a **migration**, and the file holds no
      quick-toggle spawn afterward; quick-toggle is bound nowhere
- [ ] **Every pre-existing shortcut still fires right after install** —
      before anything else, press a custom shortcut you already had; the
      0.1.0 regression here made cosmic-comp discard the whole `custom`
      file (factory-default session) because one emitted keysym didn't
      parse. Confirm `journalctl --user -b | grep "custom config error"`
      shows nothing new
- [ ] Alt+grave, Alt+Esc, and Alt+Shift+Esc actually fire (not just
      appear in the file); Esc's keysym name is `Escape`
- [ ] `--uninstall` restores the `custom` file byte-exactly too (same
      diff), or deletes it when install created it; unmarked entries
      survive

## 5. Live-state edge cases

- [ ] Close the selected window while the overlay is up — commit is a no-op,
      the daemon survives
- [ ] Move a listed window to another workspace mid-gesture — the gesture
      dismisses (the list is never silently swapped under a held Alt)
- [ ] Toggle a window sticky while listed — the next invoke reflects it
- [ ] Switch workspaces mid-gesture — dismisses; the next press starts fresh
      against the new workspace
- [ ] Invoke on an empty workspace — the overlay shows nothing, commit is a
      no-op
- [ ] Restart the compositor (or log out/in) — the daemon exits loudly; the
      next press auto-spawns a fresh one and cycling works again

## 6. Soak

- [ ] 10+ rapid presses in a row: no panics, no stuck keyboard grab, the
      overlay never lingers past the gesture

## 7. Performance

Measure and compare against the targets:

| Measurement | How | Target | Reference (2026-11-02) |
|---|---|---|---|
| Daemon RSS / threads | `ps -o rss=,nlwp= -C airlock` | < 30 MB | 8.0 MB / 1 thread |
| Warm press round-trip | `time airlock alt-tab` (daemon running) | < 50 ms | < 10 ms |
| Cold press (respawn + settle + switch) | `time airlock alt-tab` (daemon stopped) | — | ~340 ms |

The 2026-11-02 values are the overlay-visuals build: the +4 MB RSS is
the icon corpus and resvg, the ~40 ms of extra cold time is the
one-time desktop-entry enumeration — both prices of real app icons,
both far inside the budgets. Warm presses were measured from outside
the session through the guest agent (timer granularity 10 ms); spawn
with the session's `XDG_CURRENT_DESKTOP`/`XDG_SESSION_TYPE` or cosmic-comp
withholds the initial toplevel burst and the daemon never settles.

For scale: the switcher path this replaces (cosmic-launcher's
cosmic-switch-less) measured ≈ 170 MB RSS.

## Latest pass

2026-11-02 — full pass (sections 1–7) by the developer on the live
session with the overlay-visuals build: icons on 6+ apps including an
SVG-only icon, both wallpaper polarities, 200% scale crisp, gesture and
same-app regressions clean, plus the reference measurements above.

2026-10-25 — full pass (sections 1–6) by the developer on the live session,
plus the reference measurements above with the overlay-polish build.
