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
- [ ] Rows one to nine show `Ctrl+1` … `Ctrl+9` at the right edge,
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
- [ ] Alt+Esc flips to the MRU-second window with nothing rendered;
      repeated presses ping-pong between the top two
- [ ] Regression: Alt+Tab still lists every window in scope, not only the
      current application's

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
      setting, clamped value — eight), the card fits the panel, and the
      setting keeps its value after the density drops back
- [ ] Move the COSMIC stores aside (rename `~/.config/cosmic`); the
      next Alt+Tab paints the factory-default geometry with one
      aggregated diagnostic per follow, and switching is unaffected

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
      rows, each entry carrying a count chip and a right-pointing
      triangle
- [ ] Tab / Shift+Tab step every app in one sequence (the default
      scope): expanded groups contribute their windows, the expanded
      entry itself never holds the highlight and shows no chord; a
      collapsed entry is selectable
- [ ] → on an entry expands it and enters — landing one past the window
      you just left when it is your own app — and the triangle turns
      down; Alt+W enters and exits too; ← on a child returns to the
      entry and collapses the group in one motion
- [ ] Set `scoped_cycling = true`: Tab steps app entries only, and one
      group's windows once entered; ← on the expanded entry collapses it
- [ ] ↑/↓ and the wheel walk the visible rows linearly across groups,
      wrapping; under the default scope the dividers are skipped
- [ ] Ctrl+N activates the Nth app entry's window — an expanded entry
      shows no chord but still answers it; Alt+N (shown on expanded
      children) activates the entered group's Nth window instantly
- [ ] Releasing Alt on an entry (or Enter) activates that app's most
      recently used window; a quick tap follows the cycle scope, not the
      flat MRU-second window
- [ ] Expansion persists across gestures: expand an app, release Alt,
      Alt+Tab again — the group is still expanded (until the daemon
      restarts)
- [ ] A workspace holding a single app's windows: Tab enters and cycles
      inside it — never a dead key
- [ ] A tree taller than `rows`: the `…` windowing rides the flattened
      sequence; Ctrl+N and the pointer are inert on the dots
- [ ] Pointer on the tree: hover and click select windows; the divider
      entry resolves no pointer input
- [ ] `grouping = false` regression: the flat card and every key behave
      exactly as before; `scoped_cycling` alone changes nothing
- [ ] Edit `grouping` (or `scoped_cycling`) while the daemon runs: the
      next Alt+Tab uses it; `grouping = 1` logs a `settings:` diagnostic
      naming the key and the last-good value holds

## 4. Takeover and restore

- [ ] `airlock install` redirects Alt+Tab / Shift+Alt+Tab to Airlock at user
      level; the override survives re-login; `/usr/share` is untouched
- [ ] `airlock install --uninstall` brings the previous switcher back
      exactly (diff `~/.config/cosmic/com.system76.CosmicSettings.Shortcuts/v1/system_actions`
      before install and after uninstall: byte-identical)
- [ ] `airlock install` also adds three marked custom bindings — Alt+grave,
      Alt+Shift+grave, Alt+Esc — to the sibling `custom` file; a key
      already bound by one of your shortcuts is skipped with a diagnostic,
      never clobbered
- [ ] **Every pre-existing shortcut still fires right after install** —
      before anything else, press a custom shortcut you already had; the
      0.1.0 regression here made cosmic-comp discard the whole `custom`
      file (factory-default session) because one emitted keysym didn't
      parse. Confirm `journalctl --user -b | grep "custom config error"`
      shows nothing new
- [ ] Alt+grave and Alt+Esc actually fire (not just appear in the file);
      Esc's keysym name is `Escape`
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
