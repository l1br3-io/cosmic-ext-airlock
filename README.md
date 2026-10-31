# Airlock

Workspace-isolated Alt+Tab for the COSMIC™ desktop: the switcher lists and
cycles only the windows on the **current workspace**, so nothing you parked
elsewhere leaks into your focus.

> Airlock is third-party software for the COSMIC™ desktop. It is not official
> COSMIC software and is not affiliated with or endorsed by System76.

## How it works

Hold **Alt** and tap **Tab**: an overlay lists the windows of the current
workspace in most-recently-used order. **Tab** walks forward, **Shift+Tab**
walks back, and the arrow keys step too — **Up** or **Left** back,
**Down** or **Right** forward. Releasing **Alt** focuses the selection and
dismisses the overlay; **Enter** confirms the same outcome without letting
go of **Alt**. **Escape** dismisses without switching. A quick tap —
released before the overlay even appears — still switches.

Only windows whose workspace set includes the current workspace are listed:
sticky windows appear everywhere, minimized windows nowhere, and a window
parked on another workspace can never be focused through Airlock.

The isolation is per-workspace configurable: a workspace can opt into the
global switcher through a config file — see [Settings](docs/settings.md).

A small daemon keeps a live snapshot of the compositor state; the keybind
stubs are milliseconds-thin and auto-start the daemon on first press. The
overlay is software-rendered — no GPU required.

## Requirements

- A COSMIC desktop session (epoch). Other compositors are out of scope for
  v1; on one that lacks the required protocols the daemon exits non-zero
  with a diagnostic naming what is missing.
- A Rust toolchain to build (prebuilt packages are not yet published —
  see [Packaging](#packaging)).

## Install

```sh
just install        # or: cargo install --path .   → ~/.cargo/bin/airlock
airlock install     # take over the Alt+Tab keybind (user level)
```

The takeover rewrites only the user-level
`~/.config/cosmic/com.system76.CosmicSettings.Shortcuts/v1/system_actions`
file — `WindowSwitcher` / `WindowSwitcherPrevious` now run `airlock
alt-tab` / `airlock shift-alt-tab`. It survives re-login and never touches
`/usr/share`. The first key press starts the daemon automatically.

## Uninstall

```sh
airlock install --uninstall   # restore the pre-install switcher, byte-exactly
cargo uninstall cosmic-ext-airlock   # optional: remove the binary
```

`--uninstall` replays the backup Airlock keeps inside the config file, so
the previous state returns byte-for-byte. If a takeover was hand-written
without Airlock's markers, restore falls back to the system default
commands (best effort).

## Troubleshooting

- **Alt+Tab does nothing.** Press again — the first press may still be
  starting the daemon (~300 ms cold). If it persists, run `airlock list`:
  a missing-protocol diagnostic names what the compositor lacks. Make sure
  `~/.cargo/bin` is on your `PATH` inside the session.
- **Only one program can own WindowSwitcher.** The user-level override is a
  single slot: another tool that rebinds Alt+Tab (or a hand-edited config)
  and Airlock will clobber each other — last writer wins. Re-run
  `airlock install` to take the slot back; `--uninstall` then restores
  whatever Airlock itself backed up.
- **After a logout or compositor crash.** The daemon exits loudly by
  design; the next key press spawns a fresh one. A stale socket self-heals.
- **The overlay ever sticks** (it should not — a 30 s hard deadline always
  releases the grab): `pkill -x airlock`, then press Alt+Tab to restart
  clean.

See also the [hardware verification checklist](docs/hardware-checklist.md)
for the full pass that every release build goes through.

## Performance

Measured on hardware (2026-10-25, see the
[checklist](docs/hardware-checklist.md) for how to reproduce):

| Measurement | Target | Airlock |
|---|---|---|
| Daemon RSS / threads | < 30 MB | **4.0 MB / 1 thread** |
| Warm press round-trip | < 50 ms | **3–4 ms** |
| Cold press (respawn + settle) | — | ~300 ms |

For scale, the switcher path this replaces (cosmic-launcher's
cosmic-switch-less) measures ≈ 170 MB RSS.

## Packaging

Not yet published. The only blocker is the app-ID decision
(`io.github.<username>.Airlock` needs the owner's GitHub username, or a
different prefix); releasing to crates.io / Flathub is a deliberate,
separate step.

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
[hardware checklist](docs/hardware-checklist.md); a COSMIC VM works too
(see the [VM guide](docs/vm-development.md)).

## Documentation

- [Per-workspace settings](docs/settings.md)
- [Hardware verification checklist](docs/hardware-checklist.md)
- [Developing and testing in the COSMIC VM](docs/vm-development.md)
- [Research index](docs/research/README.md)

## License

GPL-3.0-only — see [LICENSE](LICENSE).
