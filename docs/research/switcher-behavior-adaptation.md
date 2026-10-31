# Switcher behavior adaptation — research findings

**Date:** October 31, 2026 · **Scope:** alt-tab / window-switcher behaviors on GNOME Shell (`main`), KDE Plasma KWin (`master`), macOS, Windows 10/11, and the sway ecosystem, plus COSMIC keybind-takeover feasibility (cosmic-comp / cosmic-settings / cosmic-settings-daemon `master`, fetched 2026-10-31) · **Method:** primary sources only — upstream source code read over raw git (gitlab.gnome.org, invent.kde.org, github.com/pop-os), official Apple Support / Microsoft Support & Learn pages, and project READMEs. Remote code is cited by repo/file + function name (line numbers were not stable across fetches); Airlock claims carry file:line refs into this repository.

---

## 1. TL;DR — verdicts

| # | Candidate behavior | Verdict | Basis |
|---|---|---|---|
| 1 | Press-press commit (Enter/Space confirms selection) | **Adopt now** | GNOME ships exactly this alongside release-commit, deliberately ([switcherPopup.js](https://gitlab.gnome.org/GNOME/gnome-shell/-/raw/main/js/ui/switcherPopup.js), `_onKeyPress`); Airlock needs two enum arms and one `feed` branch |
| 2 | Arrow-key stepping while Alt held | **Adopt now** | macOS documents ←/→ in the app switcher (Apple mchlp2469); GNOME window switcher ←/→ wraps (altTab.js); same `GestureKey` extension as #1 |
| 3 | Same-application window cycling (grave family) | **Adopt with work** | KWin, GNOME, and macOS all converge on the key above Tab; COSMIC has **no system action** to take over and no default grave binding — needs the custom-shortcut path (§8) |
| 4 | Scroll-wheel stepping | **Defer** | Real precedent (KWin `TabBoxHandler::eventFilter`, GNOME `SwitcherPopup`), but Airlock has zero pointer plumbing and a keyboard-first gesture |
| 5 | Wrap-around at list ends (status quo) | **Keep** | GNOME and KWin both wrap; KWin only clamps during key auto-repeat; Airlock's `Ring` matches (src/switcher.rs:164-180) |
| 6 | Alt+Esc-style no-overlay quick toggle | **Defer** | Windows and GNOME both ship it; blocked on the same custom-shortcut plumbing as #3 |
| 7 | Type-to-filter in the switcher | **Reject** | No platform ships it *inside* the switcher (verified for GNOME and KWin); first-party implementations are launchers; it breaks Airlock's transient-gesture design (first-frame contract, 2 s idle close) |
| 8 | COSMIC feasibility for new keybinds | findings (§8) | grave and Alt+Esc are unbound by default; `Action::Spawn` custom shortcuts are the only takeover path for actions COSMIC does not define |

---

## 2. Commit semantics: release-commit vs press-press (behavior 1)

Windows-style release-to-commit is near-universal, but it is **not** the only shipped model: GNOME also commits on an explicit key press.

| Platform | Modifier release commits | Enter/Space commits while held | Escape aborts | Primary source |
|---|---|---|---|---|
| GNOME Shell | Yes — `_onModifierChange` calls `_finish()` when the binding's modifier state reaches 0; a guard in `show()` handles "the user released Alt before we got the grab" | **Yes** — `_onKeyPress` handles `KEY_space`, `KEY_Return`, `KEY_KP_Enter`, `KEY_ISO_Enter` under the comment *"Allow to explicitly select the current item"*, each calling `_finish()` | Yes (destroy) | [gnome-shell `js/ui/switcherPopup.js`](https://gitlab.gnome.org/GNOME/gnome-shell/-/raw/main/js/ui/switcherPopup.js) |
| KWin (Plasma) | Yes — `TabBox::modifiersReleased()` → `accept()`; the comment notes the switcher closes even if a non-modifier key (Tab) is still held or keys are released out of order | **Only in modifier-less mode** — `TabBox::grabbedKeyEvent` maps `Qt::Key_Enter`/`Key_Return`/`Key_Space` → `accept()` only under `if (m_noModifierGrab)`; that flag is set by `preview()` (KCM preview) and `toggleMode()` (screen-edge/touch toggles) and is false for shortcut-triggered walkthroughs | Yes — `TabBox::keyPress`: `Qt::Key_Escape` → `close(true)` (abort, restores previous window) | [kwin `src/tabbox/tabbox.cpp`](https://invent.kde.org/plasma/kwin/-/raw/master/src/tabbox/tabbox.cpp) |
| Windows | Yes — "holding Alt and pressing Tab repeatedly cycles through" the list | Not documented | (overlay cancels on Esc) | [Microsoft — Keyboard shortcuts in Windows](https://support.microsoft.com/en-us/windows/keyboard-shortcuts-in-windows-dcc61a57-8ff0-cffe-9796-cb9706c75eec) |
| macOS | Yes — "*Release the Command key.*"; "*press Esc (Escape) or the Period key*" cancels | Not documented | Yes | [Apple — Move and arrange app windows on Mac](https://support.apple.com/guide/mac-help/work-with-app-windows-mchlp2469/mac) |
| Unity7/Compiz | **Not verified from primary sources** — no claim made | — | — | — |

**Airlock today** matches the Windows model: commit rides `GestureInput::Modifiers` (src/ui.rs:315-323), Escape dismisses (src/ui.rs:309-312), and every other key is `GestureKey::Other` → `GestureOutput::Nothing` (src/ui.rs:243-248, src/ui.rs:313). Enter and arrows are currently no-ops.

**Fit — Adopt now.** The overlay already takes exclusive keyboard interactivity plus `zwp_keyboard_shortcuts_inhibit` (src/ui.rs:446-454), so Return reaches the gesture mid-session today and is simply discarded. The change is additive: extend `GestureKey` with `Commit`-style arms, map `Keysym::Return`/`KP_Enter` (optionally `Space`, matching GNOME exactly) in `key_input` (src/ui.rs:368-376), and add a `feed` branch that clears `showing` and returns `GestureOutput::Commit` — the output the daemon already knows how to apply (src/daemon.rs:318-327). No render change, no new dependency, first-frame contract untouched, quick-tap path unaffected (the overlay is not mapped yet, so no key can arrive before the first `Modifiers` report). This is also the accessibility-friendly variant: users who cannot hold a modifier reliably get an explicit confirm.

---

## 3. Arrow-key stepping while the modifier is held (behavior 2)

- **macOS (documented):** "*Press and hold the Command key, press the Tab key, then press the Left or Right arrow key until you get to the app you want. Release the Command key.*" — left/right arrows step the selection inside the open switcher; Up/Down are not documented for the switcher. Source: [Apple — Move and arrange app windows on Mac](https://support.apple.com/guide/mac-help/work-with-app-windows-mchlp2469/mac).
- **GNOME (source):** in [gnome-shell `js/ui/altTab.js`](https://gitlab.gnome.org/GNOME/gnome-shell/-/raw/main/js/ui/altTab.js):
  - App switcher (`_keyPressHandler`): `KEY_Left`/`KEY_Right` move to previous/next **app** (reversed under RTL), wrapping via the base class `_next()`/`_previous()`; `KEY_Down` moves focus into the selected app's **window thumbnails** (selecting window 0); `KEY_Up` returns focus to the app row. Within thumbnails, ←/→ move between that app's windows.
  - Window switcher: `KEY_Left`/`KEY_Right` move between windows (RTL-reversed), wrapping; no Up/Down handling.
  - ←/→ and ↑/↓ therefore **differ** only in the app switcher, where the list is two-dimensional (apps × their windows); in the flat window switcher a single axis suffices.
- **KWin:** not verifiable from the C++ I read — `TabBox::keyPress` handles the Tab-family shortcuts and Escape and forwards everything else to the QML view (`grabbedKeyEvent`), where arrow handling is layout-defined. No claim made. Source: [kwin `src/tabbox/tabbox.cpp`](https://invent.kde.org/plasma/kwin/-/raw/master/src/tabbox/tabbox.cpp).

**Fit — Adopt now.** Airlock's list is a single vertical column (rows in MRU order, src/ui.rs:126-154), so the flat-switcher case applies: ↑/↓ map naturally to `Step(Direction::Previous/Next)`, with ←/→ as optional aliases (harmless, and match the GNOME window-switcher muscle memory). Same `GestureKey`/`key_input` extension as §2; wrapping follows the existing `Ring` semantics. Adding ↓/↑ as *separate* semantics (e.g., ↑ = dismiss) would contradict every precedent — arrows should step, not confirm.

---

## 4. Same-application window cycling — the grave family (behavior 3)

All three Unix-adjacent platforms ship this, and all three bind it to **the key above Tab**:

| Platform | Key (default) | What it does | Ordering | Separate mode or filter? | Primary source |
|---|---|---|---|---|---|
| macOS | `Command-Grave` / `Shift-Command-Grave` | "*Switch between the windows of the app you're using… It's generally the key above the Tab key and to the left of the number 1*" / "*Activate the previous open window in the front app*" | Apple documents no order for this combo (next/previous window) | **No overlay described** — direct activation | [Apple — Mac keyboard shortcuts](https://support.apple.com/en-us/102650) |
| GNOME | `switch-group` = `['<Super>Above_Tab','<Alt>Above_Tab']` — "Switch windows of an application"; `cycle-group` = `['<Alt>F6']` — "Switch windows of an app directly" | Opens the **same `AppSwitcherPopup`** as Alt+Tab (dispatch in `windowManager.js` `_startSwitcher`), but `_initialSelection` for `switch-group` selects app 0 (the current app) and *window index 1* (the second-most-recent window) if the app has >1 windows, else window 0; backward selects the last window | MRU, inherited from the app switcher's list | **Not a filter** — same popup, different initial selection; the user can still navigate away to other apps | [gsettings-desktop-schemas `org.gnome.desktop.wm.keybindings`](https://gitlab.gnome.org/GNOME/gsettings-desktop-schemas/-/raw/main/schemas/org.gnome.desktop.wm.keybindings.gschema.xml.in); [gnome-shell `js/ui/altTab.js`](https://gitlab.gnome.org/GNOME/gnome-shell/-/raw/main/js/ui/altTab.js), [`js/ui/windowManager.js`](https://gitlab.gnome.org/GNOME/gnome-shell/-/raw/main/js/ui/windowManager.js) |
| KWin | "Walk Through Windows of Current Application" = `{Meta+QuoteLeft, Alt+QuoteLeft}` (grave); reverse = `{Meta+AsciiTilde, Alt+AsciiTilde}` (Shift+grave) | A **distinct TabBox mode** (`slotWalkThroughCurrentAppWindows`), overlay shown, same Tab/Escape/release handling as the main switcher | MRU — the mode uses the same client model, whose default sort is `FocusChainSwitching` ("Sort by recently used. Most recently used Window is the first", `tabboxconfig.h` `defaultSwitchingMode()`) | **Separate mode** | [kwin `src/tabbox/tabbox.cpp`](https://invent.kde.org/plasma/kwin/-/raw/master/src/tabbox/tabbox.cpp) `initShortcuts()`; [kwin `src/tabbox/tabboxconfig.h`](https://invent.kde.org/plasma/kwin/-/raw/master/src/tabbox/tabboxconfig.h) |
| Windows | No direct combo; adjacent: `Win+Ctrl+number` — "*Switch to the last active window of the app pinned to the taskbar in the position indicated by the number*" | Taskbar-slot addressed, not a cycle | "last active" per app | — | [Microsoft — Keyboard shortcuts in Windows](https://support.microsoft.com/en-us/windows/keyboard-shortcuts-in-windows-dcc61a57-8ff0-cffe-9796-cb9706c75eec) |

**Fit — Adopt with work.** The data model already carries what is needed: `Toplevel.app_id` (src/switcher.rs:43) and MRU ticks (src/switcher.rs:49). A same-app mode is an `eligible()` variant that keeps only toplevels whose `app_id` equals the focused window's (falling back to no-op when the app has one window — every platform's initial selection already behaves that way), plus:

1. a stub subcommand (`airlock same-app` / `airlock same-app-previous`) alongside `Command::AltTab`/`ShiftAltTab` (src/cli.rs:5-16, src/main.rs:41-42) and one new IPC command byte — the wire protocol is one command byte per frame (`n`/`p` today, src/ipc.rs:7-34), so a new byte is backward-compatible;
2. a **keybind takeover path** — and here is the friction: COSMIC defines no system action for it and no default grave binding exists, so this cannot ride the `system_actions` file `airlock install` manages (§8). It needs either a documented manual custom shortcut or an explicit widening of install's pinned boundary ("the only file this module may ever write is the user's `system_actions`", src/install.rs:4-9) to also manage the sibling `custom` shortcuts file.

MRU-within-app ordering and the "key above Tab" convention are both directly supported by precedent; the macOS no-overlay form (§7) is the cheap first cut if the overlay work is deferred.

---

## 5. Scroll-wheel navigation on the switcher (behavior 4)

- **KWin: yes, verified in source.** `TabBoxHandler::eventFilter` (in [kwin `src/tabbox/tabboxhandler.cpp`](https://invent.kde.org/plasma/kwin/-/raw/master/src/tabbox/tabboxhandler.cpp)) intercepts `QEvent::Wheel` on the switcher window, accumulates `angleDelta` (horizontal wins over vertical when larger), and past ±120 calls `nextPrev(true/false)` + `setCurrentIndex(...)`, consuming the event. There is **no kcfg toggle** for it — it is unconditional (the TabBox config enums in [`tabboxconfig.h`](https://invent.kde.org/plasma/kwin/-/raw/master/src/tabbox/tabboxconfig.h) contain no wheel or navigation option).
- **GNOME: yes.** `SwitcherPopup` handles scroll among its non-keyboard inputs (the popup's scroll handler drives the same `_next()`/`_previous()` helpers) — [gnome-shell `js/ui/switcherPopup.js`](https://gitlab.gnome.org/GNOME/gnome-shell/-/raw/main/js/ui/switcherPopup.js).
- **Windows / macOS:** no wheel behavior documented for the switcher by either vendor.

**Fit — Defer.** Airlock has no pointer plumbing at all: nothing in `src/` consumes `wl_pointer` frame or axis events (verified by grep over `src/*.rs`, 2026-10-31), and the gesture is keyboard-first by design — during a hold-Alt session the user's hands are on the keyboard. Supporting wheel means adding seat pointer capabilities to `src/wayland.rs` and a `GestureInput` variant, all for a pointer that only happens to hover the center-screen card. Legitimate future polish (two-desktop precedent), not v1 work.

---

## 6. Wrap-around vs clamp at list ends (behavior 5)

- **GNOME wraps**, at both levels: `_next()` = `mod(this._selectedIndex + 1, this._items.length)` (and `_previous()` symmetrically) in [`switcherPopup.js`](https://gitlab.gnome.org/GNOME/gnome-shell/-/raw/main/js/ui/switcherPopup.js); window-level cycling inside an app uses the same `SwitcherPopup.mod` pattern in [`altTab.js`](https://gitlab.gnome.org/GNOME/gnome-shell/-/raw/main/js/ui/altTab.js).
- **KWin wraps, except on key auto-repeat.** `TabBoxHandler::nextPrev` wraps rows both directions (row → `0` forward, → `rowCount()-1` backward, in [`tabboxhandler.cpp`](https://invent.kde.org/plasma/kwin/-/raw/master/src/tabbox/tabboxhandler.cpp)), but `TabBox::keyPress` carries a deliberate guard, verbatim:

  > `// Do not wrap around list on key auto-repeat`
  > `if (keyEvent.state == KeyboardKeyState::Repeated) {` … forward at the last row / backward at row 0 return early

  (source: [kwin `src/tabbox/tabbox.cpp`](https://invent.kde.org/plasma/kwin/-/raw/master/src/tabbox/tabbox.cpp)). There is no user-facing wrap option in `tabboxconfig.h`.
- **macOS:** wrap behavior is not documented by Apple; no claim.

**Fit — Keep as-is.** Airlock's `Ring` wraps both directions (src/switcher.rs:159-181, unit-tested both ways), which matches GNOME exactly and KWin's non-repeat behavior. KWin's auto-repeat clamp is a defensible anti-runaway refinement for a *held* Tab, but Airlock's wrap-on-hold is the intended fly-through semantic and is covered by existing tests; changing it is not warranted. Verdict: no change; this section exists to record that the status quo is the mainstream position.

---

## 7. MRU ordering nuances; the Alt+Esc no-overlay precedent (behavior 6)

- **MRU as the documented order:** Apple states it outright — `Command-Tab`: "*Switch to the next most recently used app among your open apps*" ([Apple](https://support.apple.com/en-us/102650)). KWin defaults to it — `defaultSwitchingMode()` returns `FocusChainSwitching`, documented as "*Sort by recently used. Most recently used Window is the first*" ([kwin `tabboxconfig.h`](https://invent.kde.org/plasma/kwin/-/raw/master/src/tabbox/tabboxconfig.h)). Microsoft documents the cycling but **not** the order of Windows' Alt+Tab list.
- **Promote-on-commit:** on every platform that documents it, the committed window becomes the MRU head for the next invocation (raising focus reorders the focus chain / z-order). **Airlock already does this**: the daemon stamps `last_activated` from observed `ACTIVATED` transitions (src/daemon.rs:48-72; a newly activated window moves to the front, tested at src/daemon.rs:491-513), while the ring stays frozen within one session. No change needed.
- **Alt+Esc — switching without any UI.** Two platforms ship a no-overlay cycler bound to Escape:
  - Windows: `Alt+Esc` — "*Cycle through items in the order in which they were opened*" (Win 11 phrasing: "*Cycle through windows in the order in which they were opened*") ([Microsoft](https://support.microsoft.com/en-us/windows/keyboard-shortcuts-in-windows-dcc61a57-8ff0-cffe-9796-cb9706c75eec)).
  - GNOME: `cycle-windows` = `['<Alt>Escape']` — "Switch windows directly", dispatched to `WindowCyclerPopup` ([gsettings-desktop-schemas](https://gitlab.gnome.org/GNOME/gsettings-desktop-schemas/-/raw/main/schemas/org.gnome.desktop.wm.keybindings.gschema.xml.in); [gnome-shell `windowManager.js`](https://gitlab.gnome.org/GNOME/gnome-shell/-/raw/main/js/ui/windowManager.js)).

  This is the precedent for a "quick toggle" — one press, switch to the previous window, nothing rendered.
- **KWin-style "previously used window" single-shot actions:** no such default exists in what I verified of KWin's shortcut set; its equivalent is Alt+Tab then release (or the grave walk). No claim beyond that.

**Fit — Defer.** An `airlock quick-toggle` stub (commit to the MRU-second window immediately, never map the overlay) is a small daemon change — today every `Invoke` steps and maps (src/daemon.rs:369-374); a toggle-style invoke would return `GestureOutput::Commit` right away. But like §4 it needs a keybind COSMIC does not define (`Alt+Esc` is free by default, §8), so it shares the custom-shortcut plumbing and the install-boundary decision with same-app mode. Bundle the two.

---

## 8. COSMIC feasibility for new keybinds (behavior 8 — feeds §4 and §7)

How the current takeover works (local): `airlock install` rewrites exactly two keys — `WindowSwitcher` → `airlock alt-tab` and `WindowSwitcherPrevious` → `airlock shift-alt-tab` — inside the user's `system_actions` file, recording the pre-install state as in-file markers (src/install.rs:14-21, src/install.rs:166-169, src/install.rs:214-237). cosmic-comp executes a system action by spawning the mapped command: `Action::System(system) => { if let Some(command) = …system_actions.get(&system) { self.spawn_command(…) } }` ([cosmic-comp `src/input/actions.rs`](https://github.com/pop-os/cosmic-comp/blob/master/src/input/actions.rs), `handle_shortcut_action`; verified on master 2026-10-31 — consistent with the earlier workspace-isolation record).

Verified answers to the feasibility questions:

1. **Are there other window-switching system actions?** No. The `System` enum in [cosmic-settings-daemon `config/src/shortcuts/action.rs`](https://github.com/pop-os/cosmic-settings-daemon/blob/master/config/src/shortcuts/action.rs) (master, fetched 2026-10-31) lists ~28 variants (`AppLibrary`, `Launcher`, `LockScreen`, `Screenshot`, volume/brightness/media, `Terminal`, `WebBrowser`, `WorkspaceOverview`, …) of which the only window-switching ones are `WindowSwitcher` ("Opens the (alt+tab) window switcher") and `WindowSwitcherPrevious`. **There is no same-app variant.**
2. **Is any action bound by default to the grave key?** No. [cosmic-comp `data/keybindings.ron`](https://github.com/pop-os/cosmic-comp/blob/master/data/keybindings.ron) (master) contains **zero** bindings for `grave`/`AccentGrave`/`above_tab`. The Tab bindings are exactly: `Alt+Tab`/`Super+Tab` → `System(WindowSwitcher)` and `Alt+Shift+Tab`/`Super+Shift+Tab` → `System(WindowSwitcherPrevious)`. The only plain-`Alt` binding on any other key is `Alt+F4` → `Close`. Every Escape binding includes Super (`Super+Escape` LockScreen, `Super+Alt+Escape` LogOut, plus `Terminate`/`Debug` variants) — so **Alt+grave and Alt+Esc are free by default**.
3. **Can users bind arbitrary commands?** Yes — Custom Shortcuts in cosmic-settings create `Action::Spawn(task)` from a free-text command field ([cosmic-settings `cosmic-settings/src/pages/input/keyboard/shortcuts/custom.rs`](https://github.com/pop-os/cosmic-settings/blob/master/cosmic-settings/src/pages/input/keyboard/shortcuts/custom.rs): `let new_action = Action::Spawn(self.add_shortcut.task.clone());`), with modifier+key capture (Tab specially handled; bare keys rejected via `is_forbidden_unmodified_keysym`; multiple keybindings per shortcut; conflicts prompt replacement), and cosmic-comp executes `Action::Spawn(command) => self.spawn_command(command)` via `/bin/sh -c` ([cosmic-comp `src/input/actions.rs`](https://github.com/pop-os/cosmic-comp/blob/master/src/input/actions.rs)). Custom shortcuts persist in the sibling `custom` file of the same shortcuts v1 directory — the shape `install.rs`'s own test fixture mirrors at src/install.rs:782-788.

**Consequence for Airlock:** the `system_actions` takeover can only ever serve actions COSMIC's enum names — and installing non-enum keys there would not parse. Same-app mode (§4) and the quick toggle (§7) must reach the keyboard through `Action::Spawn` custom shortcuts: either documented as a one-time user step (`Settings → Input → Keyboard → Shortcuts → Custom`, `Alt+grave` → `airlock same-app`) or by extending `airlock install` beyond its pinned single-file boundary (src/install.rs:4-9) to also manage `custom` — with the same marker/restore discipline. That boundary change is a spec decision, not an implementation detail.

---

## 9. Type-to-filter in the switcher (behavior 7) — brief, as scoped

There is **no platform-native type-to-filter inside the alt-tab switcher** on any major OS:

- GNOME: no text entry or filtering in the switcher popups (verified across [`altTab.js`](https://gitlab.gnome.org/GNOME/gnome-shell/-/raw/main/js/ui/altTab.js) and [`switcherPopup.js`](https://gitlab.gnome.org/GNOME/gnome-shell/-/raw/main/js/ui/switcherPopup.js)).
- KWin: nothing in the TabBox key/wheel handling read suggests text input; the switcher is selection-only.
- Windows: the closest first-party artifact is **PowerToys Run's Window Walker plugin** — a launcher (Alt+Space) whose `<` command finds open windows by title/process and can switch to or close them ([Microsoft Learn — PowerToys Run](https://learn.microsoft.com/en-us/windows/powertoys/run): "*With the Window Walker plugin, you can switch to other windows, close them, or kill the window process*"). It is shipped by Microsoft but lives outside the OS shell's switcher.
- sway ecosystem: type-to-filter window switching is launcher territory there too (fzf/dmenu-style tools; e.g. [sway-yasm](https://github.com/pancsta/sway-yasm) manages windows via FZF), while the gesture-style switchers (e.g. [SwayFloatingSwitcher](https://github.com/ErikReider/SwayFloatingSwitcher), whose README notes "*releasing the left Alt key hides the switcher*") stay selection-only.

**Fit — Reject.** Not because a pinned constraint names it, but because every layer of the approved design opposes it: the transient hold-Alt gesture (press-release contract, src/ui.rs:285-288), the first-frame-at-full-fidelity contract (no progressive UI), the 2 s idle close / 30 s hard close budgets (src/ui.rs:63-67), and the fact that COSMIC already owns window search in the launcher (Super+/, per the workspace-isolation record §5.4). Window search-by-typing belongs to launchers; Airlock's job is glance-legible MRU selection.

---

## 10. Constraint cross-check

| Pinned constraint | §1 Enter commit | §2 Arrows | §3 Same-app | §4 Wheel | §6 Quick toggle | §7 Type-filter |
|---|---|---|---|---|---|---|
| First frame at full fidelity, no fades | OK (no render change) | OK | OK (same renderer) | OK | OK (no overlay at all) | **Violated** (progressive UI) |
| Titles-only, no per-app icons / thumbnails | OK | OK | OK (app_id used for *filtering* only, not iconography) | OK | OK | n/a |
| No window management beyond focus | OK (commit = focus) | OK | OK | OK | OK | n/a |
| Minimal dependency tree | OK | OK | OK | OK (no deps, but new wl_pointer plumbing) | OK | n/a |
| COSMIC-only, install boundary (`install.rs:4-9`) | OK (no keybind change) | OK (no keybind change) | **Requires new takeover path** (custom-shortcut file or docs) | OK (no keybind change) | **Requires new takeover path** | n/a |

---

## 11. Sources

**GNOME (primary source code):**

1. gnome-shell `js/ui/switcherPopup.js` — base switcher: `_onKeyPress` (Space/Return/KP_Enter/ISO_Enter → `_finish`, "Allow to explicitly select the current item"), `_onModifierChange` (release-commit + pre-grab guard), `_next`/`_previous` (`mod` wrap) — https://gitlab.gnome.org/GNOME/gnome-shell/-/raw/main/js/ui/switcherPopup.js
2. gnome-shell `js/ui/altTab.js` — app/window switcher key handlers, arrow semantics, `switch-group` initial selection, `SwitcherPopup.mod` window wrap — https://gitlab.gnome.org/GNOME/gnome-shell/-/raw/main/js/ui/altTab.js
3. gnome-shell `js/ui/windowManager.js` — `_startSwitcher` popup dispatch (`switch-group` → `AppSwitcherPopup`; `switch-windows` → `WindowSwitcherPopup`; `cycle-windows` → `WindowCyclerPopup`) — https://gitlab.gnome.org/GNOME/gnome-shell/-/raw/main/js/ui/windowManager.js
4. gsettings-desktop-schemas `org.gnome.desktop.wm.keybindings` — `switch-applications` `['<Super>Tab','<Alt>Tab']`; `switch-group` `['<Super>Above_Tab','<Alt>Above_Tab']` "Switch windows of an application"; `cycle-group` `['<Alt>F6']`; `cycle-windows` `['<Alt>Escape']` — https://gitlab.gnome.org/GNOME/gsettings-desktop-schemas/-/raw/main/schemas/org.gnome.desktop.wm.keybindings.gschema.xml.in

**KDE / KWin (primary source code):**

5. kwin `src/tabbox/tabbox.cpp` — `initShortcuts()` (grave/tilde defaults for Current Application walks), `keyPress` (Escape abort; auto-repeat wrap clamp "// Do not wrap around list on key auto-repeat"), `grabbedKeyEvent` (Enter/Return/Space → `accept()` under `m_noModifierGrab`), `modifiersReleased`, `accept`, `close` — https://invent.kde.org/plasma/kwin/-/raw/master/src/tabbox/tabbox.cpp
6. kwin `src/tabbox/tabboxhandler.cpp` — `grabbedKeyEvent` (forwarding to the QML view), `eventFilter` (wheel: ±120 angleDelta accumulation → `nextPrev` + `setCurrentIndex`), `nextPrev` (row wrap) — https://invent.kde.org/plasma/kwin/-/raw/master/src/tabbox/tabboxhandler.cpp
7. kwin `src/tabbox/tabboxconfig.h` — `ClientSwitchingMode` (`FocusChainSwitching` "Sort by recently used…", `StackingOrderSwitching`), `defaultSwitchingMode()`; no wrap/wheel options — https://invent.kde.org/plasma/kwin/-/raw/master/src/tabbox/tabboxconfig.h
8. KDE khelpcenter common shortcuts list (does not list the grave walks; source is authoritative) — https://docs.kde.org/trunk_kf6/en/khelpcenter/fundamentals/kbd.html

**COSMIC (primary source code):**

9. cosmic-comp `data/keybindings.ron` — no grave binding; Tab → `System(WindowSwitcher|WindowSwitcherPrevious)` only; `Alt+F4` → `Close`; all Escape bindings include Super — https://github.com/pop-os/cosmic-comp/blob/master/data/keybindings.ron
10. cosmic-comp `src/input/actions.rs` — `Action::System` → `system_actions.get` → `spawn_command`; `Action::Spawn(command)` → `spawn_command` (`/bin/sh -c`) — https://github.com/pop-os/cosmic-comp/blob/master/src/input/actions.rs
11. cosmic-settings-daemon `config/src/shortcuts/action.rs` — full `System` enum (~28 variants; only `WindowSwitcher`/`WindowSwitcherPrevious` switch windows; no same-app action); `Action::Spawn(String)` "Execute a command with any given arguments" — https://github.com/pop-os/cosmic-settings-daemon/blob/master/config/src/shortcuts/action.rs
12. cosmic-settings `cosmic-settings/src/pages/input/keyboard/shortcuts/custom.rs` — custom shortcuts as `Action::Spawn(task)` from free text; key capture rules (Tab special-case, bare keys forbidden, Super keycode workaround); conflict replacement — https://github.com/pop-os/cosmic-settings/blob/master/cosmic-settings/src/pages/input/keyboard/shortcuts/custom.rs

**Apple (official documentation):**

13. Mac keyboard shortcuts — `Command-Tab` "next most recently used app"; `Command-Grave` "Switch between the windows of the app you're using… generally the key above the Tab key"; `Shift-Command-Grave`; `Control-Down Arrow` App Exposé — https://support.apple.com/en-us/102650
14. Move and arrange app windows on Mac — hold Command, Tab, then "press the Left or Right arrow key until you get to the app you want. Release the Command key."; Esc/Period cancels — https://support.apple.com/guide/mac-help/work-with-app-windows-mchlp2469/mac

**Microsoft (official documentation):**

15. Keyboard shortcuts in Windows — Alt+Tab (hold + repeat cycling), Alt+Esc "Cycle through items in the order in which they were opened", Win+Ctrl+number "switch to the last active window of the app…" — https://support.microsoft.com/en-us/windows/keyboard-shortcuts-in-windows-dcc61a57-8ff0-cffe-9796-cb9706c75eec
16. PowerToys Run (Window Walker plugin, `<` direct activation; window switch/close/kill by title search) — https://learn.microsoft.com/en-us/windows/powertoys/run

**sway ecosystem (project READMEs):**

17. SwayFloatingSwitcher — Alt+Tab/Alt+Shift+Tab bindsyms (`--next`/`--previous`); "releasing the left Alt key hides the switcher" — https://github.com/ErikReider/SwayFloatingSwitcher
18. sway-yasm (FZF-based window management — type-to-filter lives in launcher-land) — https://github.com/pancsta/sway-yasm

**Airlock (this repository):**

19. Gesture state machine and key normalization — `src/ui.rs:243-248` (`GestureKey`), `src/ui.rs:297-336` (`Gesture::feed`), `src/ui.rs:368-376` (`key_input`), `src/ui.rs:446-454` (exclusive grab + shortcuts inhibit), `src/ui.rs:63-67` (CLOSE_AFTER/HARD_CLOSE)
20. Ordering, scope, ring — `src/switcher.rs:43` (`app_id`), `src/switcher.rs:68-77` (`Scope`), `src/switcher.rs:86-97` (`eligible` MRU), `src/switcher.rs:159-181` (`Ring` wrap)
21. Daemon commit/MRU — `src/daemon.rs:48-72` (MRU observe), `src/daemon.rs:103-107` (commit handle), `src/daemon.rs:318-327` (Commit application), `src/daemon.rs:369-374` (Invoke)
22. Install boundary and mechanism — `src/install.rs:4-9` (single-file boundary), `src/install.rs:14-21` (keys/commands), `src/install.rs:166-169` (paths), `src/install.rs:782-788` (sibling `custom` file fixture)
23. CLI/IPC surface — `src/cli.rs:5-16` (`Command`), `src/main.rs:41-42` (stub dispatch), `src/ipc.rs:7-34` (one-byte wire protocol)
