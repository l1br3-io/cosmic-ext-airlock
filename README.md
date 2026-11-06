# Airlock

Workspace-isolated Alt+Tab for the COSMIC™ desktop: the switcher lists and
cycles only the windows on the **current workspace**, so nothing you parked
elsewhere leaks into your focus.

![The Airlock overlay mid-gesture: a dark rounded card on a light wallpaper,
listing six windows of the current workspace — each row an app icon and a
title, one row highlighted, ellipsis rows marking the windows hidden above
and below](https://raw.githubusercontent.com/l1br3-io/cosmic-ext-airlock/main/docs/img/overlay-card.png)

> Airlock is third-party software for the COSMIC™ desktop. It is not official
> COSMIC software and is not affiliated with or endorsed by System76.

**Status:** [v0.1.0](https://github.com/l1br3-io/cosmic-ext-airlock/releases/tag/v0.1.0),
released 2026-11-02 — early and actively developed. The core Alt+Tab gesture
is solid; same-app cycling and pointer input are newer edges.

## Why

The built-in COSMIC switcher lists every window from every workspace, in the
order you last used them. Its pipeline has no workspace filter, and the
request to add one
([pop-os/cosmic-epoch#1777](https://github.com/pop-os/cosmic-epoch/issues/1777))
is still open. Airlock is that filter, as a standalone app: no compositor
patch, and `airlock install --uninstall` puts the built-in switcher back.
The switcher path it replaces (cosmic-launcher's cosmic-switch-less)
measured ≈ 170 MB RSS; Airlock's daemon measures 8.0 MB (see
[Performance](#performance)).

## How it works

Hold **Alt** and tap **Tab**: an overlay lists the windows of the current
workspace in most-recently-used order. **Tab** walks forward, **Shift+Tab**
walks back, and the arrow keys step too — **Up** or **Left** back,
**Down** or **Right** forward. Releasing **Alt** focuses the selection and
dismisses the overlay; **Enter** confirms the same outcome without letting
go of **Alt**. **Escape** dismisses without switching. A quick tap —
released before the overlay even appears — still switches.

The mouse drives the same gesture while the overlay is up. Crossing
between rows moves the selection with the cursor; wherever the pointer
first lands — including resting right where the card appears — claims
nothing. **Clicking** a row focuses that window and dismisses the overlay
— the same outcome as releasing **Alt**. The **scroll wheel** steps the
list one row per notch, forward or back, wrapping like Tab, and a slow
scan never times the overlay out. The list is frozen for the gesture: a
window that opened after the overlay appeared is not on it, so neither
the pointer nor the wheel can reach it mid-gesture.

Each row shows the window's application icon — resolved through the same
desktop-entry and icon-theme pipeline the COSMIC launcher uses — next to
its title; an application with no themed icon falls back to a generic
window glyph. The card is a rounded rectangle with a hairline border, at
most 660 px wide, with the selected row highlighted; text renders at
8 px and stays sharp on HiDPI displays (the overlay renders at the
output's scale). The `theme` setting picks the card — `auto` (the
default) follows the desktop's dark/light preference; see
[Settings](https://github.com/l1br3-io/cosmic-ext-airlock/blob/main/docs/settings.md#theme). To preview the look without a COSMIC
session:

```sh
cargo run --example overlay-preview -- preview.png
```

Only windows whose workspace set includes the current workspace are listed:
sticky windows appear everywhere, minimized windows nowhere, and a window
parked on another workspace can never be focused through Airlock.

The isolation is per-workspace configurable: a workspace can opt into the
global switcher through a config file — see [Settings](https://github.com/l1br3-io/cosmic-ext-airlock/blob/main/docs/settings.md).

`airlock install` also claims three more keys. Hold **Alt** and tap
**grave** (the key above Tab): the overlay lists only the current
application's windows, MRU order — the same gesture, the same isolation
rule (`isolated = false` widens the list to that application's windows
everywhere). **Alt+Shift+grave** opens it stepping backwards. An
application with fewer than two windows in scope does nothing — there is
nothing to cycle. **Alt+Esc** switches to the MRU-second window under the
same scope — the window a quick Alt+Tab tap would reach — with no overlay
at all.

A small daemon keeps a live snapshot of the compositor state; the keybind
stubs are milliseconds-thin and auto-start the daemon on first press. The
overlay is software-rendered — no GPU required.

## Requirements

- A COSMIC desktop session (epoch). Other compositors are out of scope for
  v1; on one that lacks the required protocols the daemon exits non-zero
  with a diagnostic naming what is missing.
- A Rust toolchain (1.93+) to build (`cargo install cosmic-ext-airlock` handles
  the rest; see [Install](#install)). Building from a checkout with
  `just install` checks the build prerequisites first — the C toolchain,
  pkg-config, the Wayland/xkbcommon headers, and cargo — and installs
  whatever is missing through the distro's package manager
  (apt/dnf/pacman/zypper) and rustup, so a fresh machine needs nothing
  by hand.

## Install

```sh
cargo install cosmic-ext-airlock   # from crates.io → ~/.cargo/bin/airlock
airlock install                    # the complete setup: keybind takeover
                                    # + app icon & desktop entry (user level)
```

Building from a checkout instead: `just install` (checks the build
prerequisites first). The assets ride the binary — `airlock install`
writes the icon and desktop entry into `~/.local/share` (never
`/usr/share`); no checkout or extra command is needed.

The takeover rewrites only user-level files in
`~/.config/cosmic/com.system76.CosmicSettings.Shortcuts/v1`:
`system_actions` points `WindowSwitcher` / `WindowSwitcherPrevious` at
`airlock alt-tab` / `airlock shift-alt-tab`, and the sibling `custom` file
gains three marked bindings — `Alt+grave` → `airlock same-app`,
`Alt+Shift+grave` → `airlock same-app-previous`, `Alt+Esc` → `airlock
quick-toggle`. A key already bound to one of your own shortcuts is left
alone: install names it and skips it. Everything survives re-login and
never touches `/usr/share`. The first key press starts the daemon
automatically.

## Uninstall

```sh
airlock install --uninstall   # restore the previous switcher and remove
                              # the app icon + desktop entry
cargo uninstall cosmic-ext-airlock   # optional: remove the binary
```

`--uninstall` puts the previous switcher back. Airlock keeps its backups
inside the config files themselves, so the pre-install state returns
byte-for-byte — a `custom` file install created is deleted again — and
both asset files are removed too, unless you edited one by hand: yours
is kept and named, never silently deleted.

The ignore rule is per key. A switcher slot that runs some other tool
was never Airlock's, and `--uninstall` does not rewrite it — even when
the sibling key does run Airlock.

One degradation is real: while Airlock is installed, a cosmic-settings
rewrite can strip the backup comments from the file. Without them,
`--uninstall` restores the built-in COSMIC switcher commands instead —
best effort, and its report says so — the same restore a
hand-written takeover without markers gets. In that one case a
third-party switcher from before install cannot be recovered; every
other path returns your exact bytes.

## Troubleshooting

- **Alt+Tab does nothing.** Press again — the first press may still be
  starting the daemon (~340 ms cold). If it persists, run `airlock list`:
  a missing-protocol diagnostic names what the compositor lacks. Make sure
  `~/.cargo/bin` is on your `PATH` inside the session.
- **Only one program can own WindowSwitcher.** The user-level override is a
  single slot: another tool that rebinds Alt+Tab (or a hand-edited config)
  and Airlock will clobber each other — last writer wins. Re-run
  `airlock install` to take the slot back; `--uninstall` then restores
  whatever Airlock itself backed up.
- **Alt+grave or Alt+Esc does nothing after install.** The key was already
  bound to another command; install left it alone and said so in its
  output. Free the key in Settings → Input → Keyboard → Shortcuts →
  Custom and re-run `airlock install`.
- **After a logout or compositor crash.** The daemon exits loudly by
  design; the next key press spawns a fresh one. A stale socket self-heals.
- **The overlay ever sticks** (it should not — a 30 s hard deadline always
  releases the grab): `pkill -x airlock`, then press Alt+Tab to restart
  clean.

See also the [hardware verification checklist](https://github.com/l1br3-io/cosmic-ext-airlock/blob/main/docs/hardware-checklist.md)
for the full pass that every release build goes through.

## Performance

Measured on hardware (2026-11-02, see the
[checklist](https://github.com/l1br3-io/cosmic-ext-airlock/blob/main/docs/hardware-checklist.md) for how to reproduce):

| Measurement | Target | Airlock |
|---|---|---|
| Daemon RSS / threads | < 30 MB | **8.0 MB / 1 thread** |
| Warm press round-trip | < 50 ms | **< 10 ms** |
| Cold press (respawn + settle) | — | ~340 ms |

## Packaging

Published to [crates.io](https://crates.io/crates/cosmic-ext-airlock) as
`cosmic-ext-airlock`;
[GitHub Releases](https://github.com/l1br3-io/cosmic-ext-airlock/releases)
carries the tags and release notes. App ID: `io.github.l1br3-io.Airlock`
— it names the icon, the desktop entry, and the AppStream metainfo
under `res/` (AppStream warns about the hyphen in `l1br3-io`; the ID
deliberately matches the GitHub org, and hyphenated `io.github.<org>`
IDs are widespread). A Launchpad PPA and Flathub are on the roadmap
without dates — Flathub only after re-opening the research verdict that
a sandboxed daemon cannot take over the keybind (the takeover rewrites
`~/.config/cosmic/…`, outside the sandbox); see the
[research index](https://github.com/l1br3-io/cosmic-ext-airlock/blob/main/docs/research/README.md).

## Development

Daily use and development never collide:

- **Daily use** — `just install` puts a self-contained `airlock` in
  `~/.cargo/bin`. The spawn stub re-executes the daemon from its own path,
  so the installed keybind keeps working no matter what state the repo is
  in. Reinstalling while a daemon runs is safe; the old daemon keeps
  serving until stopped.
- **Development** — `just run` builds and runs the daemon from the dev
  tree. Only one daemon can exist at a time (single-instance socket), so
  `just stop` first if the installed daemon is serving. `just stop` also
  activates a fresh `just install`: the next key press auto-spawns the
  newly installed binary.

Verification gates: `just test`, `just check` (clippy, warnings denied),
`just fmt-check`. Runtime behavior needs real COSMIC hardware — run the
[hardware checklist](https://github.com/l1br3-io/cosmic-ext-airlock/blob/main/docs/hardware-checklist.md); a COSMIC VM works too
(see the [VM guide](https://github.com/l1br3-io/cosmic-ext-airlock/blob/main/docs/vm-development.md)).

## Documentation

- [Per-workspace settings](https://github.com/l1br3-io/cosmic-ext-airlock/blob/main/docs/settings.md)
- [Hardware verification checklist](https://github.com/l1br3-io/cosmic-ext-airlock/blob/main/docs/hardware-checklist.md)
- [Developing and testing in the COSMIC VM](https://github.com/l1br3-io/cosmic-ext-airlock/blob/main/docs/vm-development.md)
- [Research index](https://github.com/l1br3-io/cosmic-ext-airlock/blob/main/docs/research/README.md)

## License

GPL-3.0-only — see [LICENSE](LICENSE).
