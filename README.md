# Airlock

Workspace-isolated Alt+Tab for the COSMIC™ desktop: the switcher lists and
cycles only the windows on the **current workspace**, so nothing you parked
elsewhere leaks into your focus.

![Airlock's Alt+Tab overlay: a dark rounded card listing the current
workspace's windows — app icon and title per row, one highlighted, ellipsis
rows above and below](https://raw.githubusercontent.com/l1br3-io/cosmic-ext-airlock/main/docs/img/overlay-card.png)

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

Hold **Alt** and tap **Tab**: an overlay lists the current workspace's
windows in most-recently-used order. **Tab** steps forward, **Shift+Tab**
back; releasing **Alt** switches and dismisses, **Enter** confirms without
letting go, **Escape** dismisses, and a quick tap — released before the
overlay appears — still switches. The pointer drives the same gesture:
hover moves the selection, click switches, the wheel steps the list.

Each row is the application's themed icon — the same pipeline the COSMIC
launcher uses — beside its title. Preview the card without a COSMIC
session:

```sh
cargo run --example overlay-preview -- preview.png
```

Sticky windows appear on every workspace, minimized windows nowhere, and a
window on another workspace can never be focused through Airlock; a
workspace can opt into the global list through a config file (see
[Settings](https://github.com/l1br3-io/cosmic-ext-airlock/blob/main/docs/settings.md)).
`airlock install` also claims three more keys: **Alt+grave** cycles the
current application's windows, **Alt+Shift+grave** starts it stepping
backwards, and **Alt+Esc** jumps to the MRU-second window with no overlay.
A small daemon keeps a live compositor snapshot — the keybind stubs are
milliseconds-thin and auto-start it; the overlay is software-rendered, no
GPU required.

Every key, pointer subtlety, the frozen-list rule, and the card's
measurements: [how the switcher behaves](https://github.com/l1br3-io/cosmic-ext-airlock/blob/main/docs/how-it-works.md).

## Install

Needs a COSMIC desktop session (epoch) — on another compositor the daemon
exits non-zero with a diagnostic naming the missing protocol — and Rust
1.93+.

```sh
cargo install cosmic-ext-airlock   # from crates.io → ~/.cargo/bin/airlock
airlock install                    # the complete setup: keybind takeover
                                    # + app icon & desktop entry (user level)
```

Building from a checkout instead: `just install` checks the build
prerequisites first — C toolchain, pkg-config, Wayland/xkbcommon headers,
cargo — and installs whatever is missing through the distro's package
manager and rustup. The assets ride the binary — `airlock install` writes
the icon and desktop entry into `~/.local/share`, never `/usr/share`.

The takeover rewrites only user-level files in
`~/.config/cosmic/com.system76.CosmicSettings.Shortcuts/v1`: the
`WindowSwitcher` slots point at `airlock alt-tab` / `airlock
shift-alt-tab`, and three marked custom bindings are added for the extra
keys. A key already bound to one of your own shortcuts is left alone —
install names it and skips it. Everything survives re-login; the first key
press starts the daemon automatically.

## Uninstall

```sh
airlock install --uninstall   # restore the previous switcher and remove
                              # the app icon + desktop entry
cargo uninstall cosmic-ext-airlock   # optional: remove the binary
```

Backups live inside the config files themselves, so the pre-install state
returns byte-for-byte — a `custom` file install created is deleted again,
and an asset file you edited by hand is kept and named, never silently
deleted. The ignore rule is per key: a switcher slot running some other
tool was never Airlock's, and `--uninstall` does not rewrite it. One
degradation is real — if cosmic-settings has stripped the backup markers,
`--uninstall` restores the built-in switcher commands best-effort and says
so; in that one case a pre-install third-party switcher cannot be
recovered. Every other path returns your exact bytes.

## Troubleshooting

- **Alt+Tab does nothing.** The first press may still be starting the
  daemon (~340 ms cold) — press again. If it persists, run `airlock list`:
  a missing-protocol diagnostic names what the compositor lacks. Make sure
  `~/.cargo/bin` is on your `PATH` inside the session.
- **Only one program can own WindowSwitcher.** Another tool that rebinds
  Alt+Tab (or a hand-edited config) and Airlock will clobber each other —
  last writer wins. Re-run `airlock install` to take the slot back.
- **Alt+grave or Alt+Esc does nothing after install.** The key was already
  bound; install left it alone and said so. Free it in Settings → Input →
  Keyboard → Shortcuts → Custom and re-run `airlock install`.
- **After a logout or compositor crash.** The daemon exits loudly by
  design; the next key press spawns a fresh one, and a stale socket
  self-heals.
- **The overlay sticks** (it should not — a 30 s hard deadline always
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
carries the tags and release notes. App ID `io.github.l1br3-io.Airlock` —
the hyphen deliberately matches the GitHub org (AppStream warns about it;
hyphenated `io.github.<org>` IDs are widespread). A Launchpad PPA and
Flathub are on the roadmap without dates — see the
[research index](https://github.com/l1br3-io/cosmic-ext-airlock/blob/main/docs/research/README.md)
for the Flathub sandbox verdict.

## Development

- **Daily use** — `just install` puts a self-contained `airlock` in
  `~/.cargo/bin`; the installed keybind keeps working no matter the repo
  state, and reinstalling while a daemon runs is safe.
- **Development** — `just run` runs the daemon from the dev tree. Only one
  daemon can exist (single-instance socket), so `just stop` first if the
  installed one is serving; `just stop` also activates a fresh
  `just install`.

Verification gates: `just test`, `just check` (clippy, warnings denied),
`just fmt-check`. Runtime behavior needs real COSMIC hardware — run the
[hardware checklist](https://github.com/l1br3-io/cosmic-ext-airlock/blob/main/docs/hardware-checklist.md);
a COSMIC VM works too (see the
[VM guide](https://github.com/l1br3-io/cosmic-ext-airlock/blob/main/docs/vm-development.md)).

## Documentation

- [How the switcher behaves](https://github.com/l1br3-io/cosmic-ext-airlock/blob/main/docs/how-it-works.md)
- [Per-workspace settings](https://github.com/l1br3-io/cosmic-ext-airlock/blob/main/docs/settings.md)
- [Hardware verification checklist](https://github.com/l1br3-io/cosmic-ext-airlock/blob/main/docs/hardware-checklist.md)
- [Developing and testing in the COSMIC VM](https://github.com/l1br3-io/cosmic-ext-airlock/blob/main/docs/vm-development.md)
- [Research index](https://github.com/l1br3-io/cosmic-ext-airlock/blob/main/docs/research/README.md)

## License

GPL-3.0-only — see [LICENSE](LICENSE).
