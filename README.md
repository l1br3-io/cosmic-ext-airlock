# Airlock

Workspace-isolated Alt+Tab for the COSMIC™ desktop: the switcher lists and
cycles only the windows on the **current workspace**, so nothing you parked
elsewhere leaks into your focus.

![Airlock's Alt+Tab overlay: a dark rounded card listing the current
workspace's windows — app icon and title per row, one highlighted, ellipsis
rows above and below](https://raw.githubusercontent.com/l1br3-io/cosmic-ext-airlock/main/docs/img/overlay-card.png)

> Airlock is third-party software for the COSMIC™ desktop. It is not official
> COSMIC software and is not affiliated with or endorsed by System76.

**Status:** [v0.2.0](https://github.com/l1br3-io/cosmic-ext-airlock/releases/tag/v0.2.0) ·
released 2026-11-07 · early, actively developed — the core Alt+Tab gesture is
solid; same-app cycling and pointer input are newer.

## Why

- The built-in COSMIC switcher lists every window on every workspace, most
  recently used first. There is no workspace filter, and the request for one
  ([pop-os/cosmic-epoch#1777](https://github.com/pop-os/cosmic-epoch/issues/1777))
  is still open.
- Airlock is that filter, as a standalone app. No compositor patch;
  `airlock install --uninstall` restores the built-in switcher.
- The switcher path it replaces (cosmic-launcher's cosmic-switch-less)
  measured ≈ 170 MB RSS. Airlock's daemon: 8.0 MB ([Performance](#performance)).

## How it works

Hold **Alt**, tap **Tab**: an overlay lists the current workspace's windows,
most recently used first. Rows are the application's themed icon (the same
pipeline the COSMIC launcher uses) beside its title.

- **Tab** / **Shift+Tab** step forward / back; arrow keys work too
- Release **Alt** to switch and dismiss; **Enter** confirms without
  releasing; **Escape** dismisses without switching
- A quick tap, released before the overlay appears, still switches
- Pointer: hover moves the selection, click switches, the wheel steps the
  list
- **Ctrl+N** (1–9, keypad too) switches to the Nth window at once, like a
  click; each row shows its key
- Titles render in a built-in ASCII pixel font: typographic punctuation
  folds to its ASCII lookalike (`—` → `-`), Latin accents fold to their
  base letter (`é` → `e`), and anything with no ASCII form — CJK,
  Cyrillic, ligatures like `æ` — shows as `?`. Stated, not papered over.

Preview the card without a COSMIC session:

```sh
cargo run --example overlay-preview -- preview.png
```

Scope and extra keys:

- Sticky windows appear on every workspace; minimized windows nowhere; a
  window on another workspace can never be reached through Airlock. A
  workspace can opt into the global list — see
  [Settings](https://github.com/l1br3-io/cosmic-ext-airlock/blob/main/docs/settings.md).
- **Alt+grave** (the key above Tab) cycles the current application's
  windows;
  **Alt+Shift+grave** starts it stepping backwards; **Alt+Esc** jumps to the
  second-most-recent window with no overlay.
- A small daemon keeps a live compositor snapshot; the keybind stubs
  auto-start it on first press. The overlay is software-rendered.

Every key, pointer rule, the frozen-list rule, and the card's measurements:
[how the switcher behaves](https://github.com/l1br3-io/cosmic-ext-airlock/blob/main/docs/how-it-works.md).

## Install

Requires a COSMIC desktop session (epoch) and Rust 1.93+. On another
compositor the daemon exits with a diagnostic naming the missing protocol.

```sh
cargo install cosmic-ext-airlock   # from crates.io → ~/.cargo/bin/airlock
airlock install                    # the complete setup: keybind takeover
                                    # + app icon & desktop entry (user level)
```

From a checkout, `just install` checks the build prerequisites (C toolchain,
pkg-config, Wayland/xkbcommon headers, cargo) and installs missing ones
through the distro's package manager and rustup. Assets ride the binary:
`airlock install` writes the icon and desktop entry into `~/.local/share`,
never `/usr/share`.

The takeover touches only user-level files in
`~/.config/cosmic/com.system76.CosmicSettings.Shortcuts/v1`:

- `WindowSwitcher` / `WindowSwitcherPrevious` point at `airlock alt-tab` /
  `airlock shift-alt-tab`
- Three marked custom bindings are added for the extra keys
- A key already bound to one of your own shortcuts is skipped and named in
  the output
- Everything survives re-login; the first key press starts the daemon

## Uninstall

```sh
airlock install --uninstall   # restore the previous switcher and remove
                              # the app icon + desktop entry
cargo uninstall cosmic-ext-airlock   # optional: remove the binary
```

- Backups live inside the config files; the pre-install state returns
  byte-for-byte
- An asset file you edited by hand is kept and named, never silently
  deleted
- The ignore rule is per key: a slot running some other tool is never
  rewritten
- One degradation: if cosmic-settings has stripped the backup markers,
  `--uninstall` restores the built-in switcher commands best-effort and says
  so. A pre-install third-party switcher cannot be recovered in that one
  case; every other path returns your exact bytes

## Troubleshooting

- **Alt+Tab does nothing.** First press may be starting the daemon (~340 ms
  cold) — press again. If it persists, `airlock list` names any missing
  protocol; check `~/.cargo/bin` is on your session `PATH`.
- **Another tool fights over WindowSwitcher.** The user-level override is
  one slot; last writer wins. Re-run `airlock install`.

More, including dead extra keys and daemon restarts after logout:
[troubleshooting](https://github.com/l1br3-io/cosmic-ext-airlock/blob/main/docs/troubleshooting.md).

## Performance

Measured on hardware, 2026-11-02 (see the
[checklist](https://github.com/l1br3-io/cosmic-ext-airlock/blob/main/docs/hardware-checklist.md)
for method):

| Measurement | Target | Airlock |
|---|---|---|
| Daemon RSS / threads | < 30 MB | **8.0 MB / 1 thread** |
| Warm press round-trip | < 50 ms | **< 10 ms** |
| Cold press (respawn + settle) | — | ~340 ms |

## Packaging

On [crates.io](https://crates.io/crates/cosmic-ext-airlock) as
`cosmic-ext-airlock`;
[releases](https://github.com/l1br3-io/cosmic-ext-airlock/releases) carry
the tags and notes. App ID `io.github.l1br3-io.Airlock` — the hyphen matches
the GitHub org. A Launchpad PPA and Flathub are on the roadmap without dates
(see the [research index](https://github.com/l1br3-io/cosmic-ext-airlock/blob/main/docs/research/README.md)).

## Development

- `just install` — self-contained binary in `~/.cargo/bin`; the installed
  keybind keeps working regardless of repo state
- `just run` — daemon from the dev tree; `just stop` first if the installed
  one is serving (single-instance socket)
- Gates: `just test`, `just check` (clippy, warnings denied), `just
  fmt-check`. Runtime needs COSMIC hardware — the
  [hardware checklist](https://github.com/l1br3-io/cosmic-ext-airlock/blob/main/docs/hardware-checklist.md)
  or the [VM guide](https://github.com/l1br3-io/cosmic-ext-airlock/blob/main/docs/vm-development.md)

## Documentation

- [How the switcher behaves](https://github.com/l1br3-io/cosmic-ext-airlock/blob/main/docs/how-it-works.md)
- [Per-workspace settings](https://github.com/l1br3-io/cosmic-ext-airlock/blob/main/docs/settings.md)
- [Troubleshooting](https://github.com/l1br3-io/cosmic-ext-airlock/blob/main/docs/troubleshooting.md)
- [Hardware verification checklist](https://github.com/l1br3-io/cosmic-ext-airlock/blob/main/docs/hardware-checklist.md)
- [Developing and testing in the COSMIC VM](https://github.com/l1br3-io/cosmic-ext-airlock/blob/main/docs/vm-development.md)
- [Releasing](https://github.com/l1br3-io/cosmic-ext-airlock/blob/main/docs/releasing.md)
- [Research index](https://github.com/l1br3-io/cosmic-ext-airlock/blob/main/docs/research/README.md)

## License

GPL-3.0-only — see [LICENSE](LICENSE).
