# Troubleshooting

- **Every shortcut reset to defaults right after installing (0.1.0).**
  Airlock 0.1.0 wrote its extra bindings with a keysym form COSMIC
  rejects (`key: Some(..)` instead of `key: "grave"`). cosmic-comp then
  discarded the whole `custom` shortcuts file and ran factory defaults
  for the live session — that file holds *all* your customizations, so
  everything looked reset. **Nothing was deleted**: your entries are
  still on disk, and either fix restores them immediately, no re-login:
  - `airlock install --uninstall` (any version) puts your file back
    byte-exactly, or
  - upgrade to 0.1.1 and re-run `airlock install` — it recognizes 0.1.0's
    malformed entries, replaces them with valid ones, and leaves your
    entries untouched.
  Only exception: if you saved a change in Settings → Shortcuts while the
  session showed defaults, that one save overwrote the file with what the
  screen showed.
- **Alt+Tab does nothing.** First press may be starting the daemon (~340 ms
  cold) — press again. If it persists, `airlock list` names any missing
  protocol. Since 0.4.6 the keybinds hold the binary's absolute path, so
  the switcher itself never depends on `PATH`; the installer adds the
  binary's directory to `~/.bashrc` and `~/.profile` only so typing
  `airlock` works.
- **The one-command installer stops before finishing.** It names the
  reason. A checksum mismatch means a corrupt download: re-run it, and if
  it repeats, do not install and report it. A missing prebuilt binary
  means your architecture has none: build from source instead. A `PATH`
  warning means you ran with `AIRLOCK_NO_MODIFY_PATH=1`: the takeover
  still completed; add the printed `export PATH=...` line to your shell
  startup to type `airlock` yourself.
- **Another tool fights over WindowSwitcher.** The user-level override is
  one slot; last writer wins. Re-run `airlock install` to take it back;
  `--uninstall` then restores whatever Airlock itself backed up.
- **Alt+` (the key above Tab) or Alt+Esc dead after install.** The key
  was already bound;
  install skipped it and said so in its output. Free it in Settings →
  Input → Keyboard → Shortcuts → Custom, then re-run `airlock install`.
- **After logout or a compositor crash.** The daemon exits by design; the
  next press spawns a fresh one, and a stale socket self-heals.
- **Overlay sticks.** A 30 s hard deadline always releases the grab, so
  this should not happen — if it does: `pkill -x airlock`, then press
  Alt+Tab to restart clean.

Every release build goes through the
[hardware verification checklist](hardware-checklist.md); if something
behaves differently on your machine, that checklist tells you what to
report.
