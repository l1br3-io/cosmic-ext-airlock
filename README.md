# Airlock

Workspace-isolated Alt+Tab for the COSMIC™ desktop: the switcher lists and
cycles only the windows on the **current workspace**, so nothing you parked
elsewhere leaks into your focus.

> Airlock is third-party software for the COSMIC™ desktop. It is not official
> COSMIC software and is not affiliated with or endorsed by System76.

Status: early development — not yet usable. See the
[spec's research records](docs/research/README.md) for the feasibility and
prior-art background.

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

## Documentation

- [Research index](docs/research/README.md)
- [Developing and testing in the COSMIC VM](docs/vm-development.md)

## License

GPL-3.0-only — see [LICENSE](LICENSE).
