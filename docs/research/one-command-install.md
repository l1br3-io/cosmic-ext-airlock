# One-command installation: what established CLI/desktop tools actually do

**Question:** How do established CLI/desktop tools structure their "one command" installation — build
dependencies + binary + post-install integration in one go — and what should Airlock adopt or avoid?
**Date:** 2026-11-15
**Status:** Complete — eight curl|sh installers read at source plus first-party docs for each tool's
post-install story; verdicts for Airlock in the closing section. This record is closed; corrections
go in a new record, not edits here.

**Method:** primary sources only. Each installer script was fetched in full (raw GitHub or the
canonical install URL) and interrogated for the same six facts: system-dependency handling, sudo
policy, PATH/rc behavior, download verification, version pinning, dry-run/uninstall. Post-install
integration claims come from each project's own docs/README, and the cargo post-install story from
the rust-lang/cargo issues themselves. Companion local facts: `justfile` (the `deps` gate), the
`[package.metadata.binstall]` block in `Cargo.toml`, and the repo's AGENTS.md host-forbidden rule.
Two fetches failed and are recorded under Unverified: komorebi's docs site and zoxide's zoxide.xyz
(replaced by its README).

---

## Findings

### 1. The curl|sh mechanics: nobody installs build dependencies; they ship binaries instead

The single most consistent fact across all eight surveyed installers — rustup, uv, deno, starship,
zoxide, pnpm, nvm, Homebrew — is that **none of them installs distro packages or build
dependencies**. They check for a downloader (and at most an unzipper) and either fail loudly or
print instructions:

- **rustup** (`rustup-init.sh`): checks only for `uname`/`mktemp`/`chmod`/`mkdir`/`rm`/`rmdir` plus
  curl or wget; the actual install work is done by the downloaded `rustup-init` binary
  (https://raw.githubusercontent.com/rust-lang/rustup/master/rustup-init.sh).
- **uv** (`uv-installer.sh`): `need_cmd` checks (curl/wget, tar, coreutils) only — "It never
  installs distro packages or compiles anything"
  (https://releases.astral.sh/installers/uv/latest/uv-installer.sh).
- **deno**: checks for unzip or 7z and errors out — "Error: either unzip or 7z is required to
  install Deno" (https://raw.githubusercontent.com/denoland/deno_install/master/install.sh).
- **starship**: errors on a missing HTTP download tool, and specifically warns about snap-installed
  curl (https://raw.githubusercontent.com/starship/starship/master/install/install.sh).
- **zoxide**: `need_cmd` checks for curl/wget, grep, tar/unzip — errors, never installs
  (https://raw.githubusercontent.com/ajeetdsouza/zoxide/main/install.sh).
- **pnpm**: needs only curl or wget (https://get.pnpm.io/install.sh).
- **nvm**: needs git, curl, or wget — nothing more
  (https://raw.githubusercontent.com/nvm-sh/nvm/master/install.sh).
- **Homebrew** is the one tool that faces the same problem Airlock's `deps` gate does (a Linux box
  without build tools), and its answer is instructive: it **checks** versions (Ruby 3.4, Git ≥2.7.0,
  curl ≥7.41.0, glibc ≥2.13) but **only prints** the distro commands — `sudo apt-get install
  build-essential`, `sudo dnf group install development-tools`, `sudo pacman -S base-devel` — and
  "never packages them" (https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh).

The reason is structural: every one of these tools distributes a **prebuilt binary**, so the script's
job is download-and-place, not build. An installer that compiles from source (which is what Airlock's
`just install` does, via cargo) is a different category that none of the famous curl|sh scripts
attempt. This is the core design fact for the one-command question: **the famous one-command installs
are one-command because the build step was eliminated, not because the script absorbed it.**

### 2. Sudo policy: user-level by default; sudo only as a last-resort fallback; Homebrew refuses root outright

- **Never touch sudo:** rustup-init.sh, uv-installer.sh, deno's install.sh, pnpm's script, and nvm's
  install.sh contain no sudo and no root checks at all; everything installs into the user's home
  (respective script URLs above).
- **Conditional fallback only:** **starship** uses sudo exclusively when the target bin dir is not
  writable — it touches a test file, warns "Escalated permissions are required to install to
  ${BIN_DIR}", then runs `sudo -v` and aborts if not granted
  (starship install/install.sh). **zoxide** has a `try_sudo` wrapper that tries the command plain
  first and falls back to sudo (overridable via `--sudo`), needed because it may target man dirs
  under `/usr/local` (zoxide install.sh).
- **Refuse root, allow sudo narrowly:** **Homebrew** aborts with "Don't run this as root!" (with
  documented container/CI exceptions), and its `execute_sudo` runs `/usr/bin/sudo` only when a
  non-root user has sudo access, for the few steps that need it
  (Homebrew install.sh). Its FAQ states the reasoning verbatim: "Running any brew command with sudo
  trusts the Homebrew prefix owner and anyone else who can modify Homebrew's code to run arbitrary
  code as root" and "if the owner adds rm -rf /path/to/data to bin/brew, the next sudo brew ... will
  run that deletion as root" (https://docs.brew.sh/FAQ).

Airlock's `deps` gate already encodes the same policy — it installs Rust "never as root, so `sudo
just install` never writes a root-owned toolchain into /root" (justfile:93–103) — and `airlock
install` is strictly user-level by design. The surveyed norm confirms that contract; no major
installer escalates beyond a writable-dir fallback.

### 3. PATH/rc handling: the ecosystem is split roughly in half — auto-edit vs. print instructions

**Auto-edits shell rc files:**

- **uv** creates an env script (`export PATH=...`) and appends a source line to `.profile`,
  `.bashrc`, `.bash_profile`, `.bash_login`, `.zshrc`, `.zshenv`, and the fish config; opt-out via
  `UV_NO_MODIFY_PATH=1` (uv-installer.sh). Its docs acknowledge the side effect under Upgrading:
  "Updating uv will re-run the installer and can modify your shell profiles"
  (https://docs.astral.sh/uv/getting-started/installation/).
- **deno** delegates rc editing to a bundled Deno script (`run_shell_setup`), and its docs state
  plainly: "the install script updates your shell rc file" and the uninstall section names the
  appended line `. "$HOME/.deno/env"` (deno install.sh;
  https://docs.deno.com/runtime/getting_started/installation/).
- **pnpm** runs `pnpm setup --force` at the end; the docs say setup "adds the pnpm home directory to
  the PATH by updating the shell configuration file" and confirm "The standalone installation scripts
  of pnpm run this command" — with a `--no-modify-path` escape and a print-the-lines fallback when it
  cannot identify the file (https://get.pnpm.io/install.sh; https://pnpm.io/cli/setup).
- **nvm** appends its source lines directly: `command printf '%b' "${SOURCE_STR}" >>
  "$NVM_PROFILE"`, with a dedupe check and a `PROFILE=/dev/null` escape hatch
  (nvm install.sh).
- **rustup** (the downloaded `rustup-init` binary, not the .sh) configures PATH unless
  `--no-modify-path` is passed — the flag is listed in rustup-init.sh's own usage text
  (rustup-init.sh).

**Prints instructions, never edits:**

- **starship**: the script only installs the binary and prints `eval "$(starship init bash)"`-style
  lines; the guide makes shell integration "explicitly Step 2 … users manually edit their shell's
  config file" (starship install.sh; https://starship.rs/guide/).
- **zoxide**: the script prints a PATH warning only; the README instructs "Add this to the **end** of
  your config file" with per-shell eval lines
  (zoxide install.sh; https://raw.githubusercontent.com/ajeetdsouza/zoxide/main/README.md).
- **Homebrew**: prints the exact `eval "$(brew shellenv)"` append line for the user to run, and only
  writes `/etc/paths.d/homebrew` on macOS under specific conditions (Homebrew install.sh).

The split maps to tool category: shell-adjacent prompt/chdir tools (starship, zoxide) print, because
their integration line is code the user should understand; toolchain/PATH tools (uv, deno, nvm,
pnpm, rustup) edit, because PATH is boilerplate. Airlock's integration surface is neither shell rc —
it is COSMIC's shortcuts config — so the relevant question is §5, not this one.

### 4. Download verification: only uv checks a checksum in-script; rustup and deno publish checksums their scripts never use

- **uv** is the only surveyed script that verifies: each platform target has an embedded sha256 and
  the script calls `verify_checksum`, failing on "checksum mismatch"
  (uv-installer.sh). Notably its docs page documents no checksum downloads at all — the verification
  is invisible to the user (https://docs.astral.sh/uv/getting-started/installation/).
- **rustup** publishes the artifacts but its own script ignores them: SHA-256 files exist "by
  appending .sha256 to the link" (https://rust-lang.github.io/rustup/installation/other.html), and
  standalone installers carry GPG signatures — "Each of these binaries is signed with the Rust
  signing key … with GPG" — but Rust Forge admits the gap: "Although rustup performs its downloads
  over HTTPS, the only way to verify the signatures of Rust installers today is to do so manually
  with the standalone installers"
  (https://forge.rust-lang.org/infra/other-installation-methods.html). The one-line install performs
  no verification beyond TLS; the script does add TLS hardening (`--tlsv1.2`, strong ciphers)
  (rustup-init.sh).
- **deno**'s docs state "Each asset has a matching .sha256sum file for verifying the download" — for
  *manual* installs; the script itself does no checksum
  (https://docs.deno.com/runtime/getting_started/installation/).
- **starship, zoxide, pnpm, nvm**: no checksum or signature handling anywhere in the scripts (their
  URLs above). **Homebrew** avoids the problem by `git clone`/`git fetch` of a tag from
  github.com/Homebrew/brew rather than downloading a tarball (Homebrew install.sh).

**Version pinning:** supported by rustup (`RUSTUP_VERSION`), uv (the script URL itself embeds the
version — `curl -LsSf https://astral.sh/uv/0.13.8/install.sh | sh` is the documented pinned form),
starship (`--version`), pnpm (`PNPM_VERSION`), nvm (`NVM_INSTALL_VERSION`), deno (positional arg);
zoxide and Homebrew always take latest. **`--dry-run`: none of the eight offers it.** **Uninstall:**
no script uninstalls; removal is always a separate documented path — `rustup self uninstall`
(https://rust-lang.github.io/rustup/installation/index.html), uv's `uv cache clean` + `rm` steps,
deno's `rm -rf ~/.deno` + rc-line removal, nvm's `nvm unload` + `rm -rf "$nvm_dir"` + rc-line
removal (https://raw.githubusercontent.com/nvm-sh/nvm/master/README.md), Homebrew's separate
uninstall script ("To uninstall Homebrew, run the uninstall script from the Homebrew/install
repository", https://docs.brew.sh/FAQ).

### 5. Post-install integration: printed-or-prompted, never silent — and cargo has no hook mechanism at all

**Shell-init tools** (starship, zoxide) ship the integration as a documented manual step — the
eval line per shell is in the docs, the installers print it, nobody types it for you (§3). **Desktop
integration** has one canonical cautionary tale: AppImageLauncher "extracts and patch[es] the desktop
entry as well as the related icons into the relevant locations" — but only after a first-run dialog
"prompting the user whether to run the AppImage once, or move it to a predefined location and adding
it to the application menus." Its README explicitly contrasts this with the older `appimaged`
daemon, which silently scanned directories and auto-integrated everything — criticized as
I/O-heavy, "controlling", and a security hazard after a discovered vulnerability
(https://raw.githubusercontent.com/TheAssassin/AppImageLauncher/master/README.md).

**cargo itself has no post-install mechanism.** The requests are long-standing and never accepted:
rust-lang/cargo #11539 ("Post cargo install hook", Jan 2023) was told by maintainer ehuss "I don't
believe there is currently a mechanism to do that", recommended "an outer general-purpose build
system" or "your own scripts using the cargo-xtask pattern", and was closed as not-planned in favor
of the older #2386 (https://github.com/rust-lang/cargo/issues/11539). #2386 — Manishearth's 2016
proposal for a project-supplied `install.rs` — sat labeled `S-needs-design` and was closed as
not-planned in May 2023 in favor of #2729, again without an accepted design
(https://github.com/rust-lang/cargo/issues/2386). So Airlock's `airlock install` subcommand is not a
quirk — it is the standard shape for a Rust tool that needs integration, exactly the xtask-adjacent
pattern cargo's maintainers point to. (Note: community summaries often say cargo rejected install
scripts "for security reasons"; the primary threads record closure-as-duplicate/needs-design with no
security rationale — see Unverified.)

**cargo-binstall** fits the same philosophy: its metadata format (`[package.metadata.binstall]`:
`pkg-url`, `bin-dir`, `pkg-fmt`, per-target `overrides`) describes only *where the artifact is and
what to extract* — no hook or script mechanism is documented at all, and install-time behavior is
extraction, placement into `~/.cargo/bin`, and symlinks
(https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/SUPPORT.md). Its README's
"Is this secure?" FAQ contrasts itself with shell-pipe installers: "We always pull the metadata from
crates.io over HTTPS, and verify the checksum of the crate tar", "We also enforce using HTTPS with
TLS >= 1.2 for the actual download of the package files", and — the tell — "we're not running
arbitrary code, but of course the crate you're downloading a package for might itself be malicious!"
Its resolution chain ends "finally to `cargo install` as a last resort"
(https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/README.md). Airlock already rides
this: `Cargo.toml:40–43` names the release tgz binstall downloads.

**Shortcut-rebinding / daemon tools** — Airlock's true peer group — all leave the risky integration
as a documented user action, and none ships a script that performs it silently:

- **espanso** (text expander daemon, the closest architecture match: resident daemon + per-keystroke
  integration): on Linux the deb/AppImage install is one command but service registration is a
  separate documented step — "Register espanso as a systemd service (required only once)" followed by
  `espanso start`, with an `--unmanaged` mode that explicitly does not autostart; the Wayland variant
  additionally needs `sudo setcap "cap_dac_override+p" $(which espanso)`
  (https://espanso.org/docs/install/linux/). On macOS the docs' install step is "drag the app into
  your /Applications folder" (https://espanso.org/docs/install/mac/).
- **yabai** (macOS tiling WM): brew install from a tap is the easy part; everything that makes it
  work is manual wiki steps — Accessibility "will request access upon launch" with a restart after
  granting, Screen Recording for animations, and the elevated-privilege scripting addition requires
  that SIP "can be (partially) disabled" because yabai injects into Dock.app — optional overall but
  required for many advanced features
  (https://github.com/koekeishiya/yabai/wiki — Installing-yabai/Quickstart pages).
- **Rectangle**: `brew install --cask rectangle`; the macOS Accessibility prompt is handled by the OS
  at first launch, and the README's troubleshooting documents *resetting* it (`tccutil reset All
  com.knollsoft.Rectangle`) plus an Uninstallation section (`defaults delete
  com.knollsoft.Rectangle`, `brew uninstall --zap rectangle`)
  (https://raw.githubusercontent.com/rxhanson/Rectangle/master/README.md).
- **alt-tab-macos** ships no installer at all beyond the app bundle/website download (its README is
  a two-image banner — local record
  [readme-famous-switchers.md](readme-famous-switchers.md)); permission grants are the user's
  System Settings work.
- **komorebi**'s README delegates install to its docs site with a video (local record, fetched
  2026-11-14); the docs site itself was unreachable this pass (Unverified).

The pattern across the category: **install = one command; activation = a visible, user-owned step;
undo = documented alongside.** Even the tools that could chain activation mostly print instead.

### 6. The trust model: what first-party pages actually claim about curl|sh

First-party statements about trusting a piped shell script are rarer than the pattern's prevalence
suggests:

- **uv** is the only one with explicit audit guidance: "The installation script may be inspected
  before use:" followed by pipe-to-`less` examples — plus the pinned-version URL form so the bytes
  you audit are the bytes a later run fetches (https://docs.astral.sh/uv/getting-started/installation/).
- **nvm**'s README notes you can "download and run the script manually" instead of piping, and its
  install.sh prints a tamper warning when redirected at a non-default repo: "IT IS POSSIBLE THAT
  SOMEONE IS DOING SOMETHING NASTY!" (nvm README; nvm install.sh).
- **rustup**'s docs pages state no review advice at all; the checksum/signature artifacts exist
  (§4) but the default flow never touches them, and Rust Forge says so out loud
  (forge.rust-lang.org, quoted above).
- **Homebrew**'s FAQ carries the strongest first-party trust reasoning in the set, aimed at the
  escalation question: sudo would turn any prefix-owner mistake or compromise into root code
  execution (https://docs.brew.sh/FAQ, quoted in §2).
- **cargo-binstall**'s FAQ makes the anti-curl|sh pitch explicitly: HTTPS + crates.io checksum
  verification + "we're not running arbitrary code" (cargo-binstall README, quoted in §5).

Where guidance conflicts: uv auto-edits shell profiles by default while its own docs surface the
behavior only as an upgrade-time tip (§3); cargo's no-hooks stance is widely summarized as
security-motivated but the primary threads record no such rationale (§5, Unverified); and the
checksum story is inconsistent — rustup and deno publish verification artifacts their installers
ignore, while uv embeds verification its docs never mention. There is no consensus contract to
inherit; there is a clear best-in-class (uv's embedded, per-version checksums) and a clear
worst-case (silent integration with no undo — appimaged, §5).

### What Airlock should adopt / avoid — verdicts for the owner to decide

1. **A curl|sh one-command installer is justified only as a prebuilt-binary fetcher — and for
   Airlock that works.** Every surveyed installer one-commands by *eliminating* the build, not by
   absorbing it (§1). Airlock already ships the exact artifact such a script needs: the release tgz
   that `cargo binstall` downloads (`Cargo.toml:40–43`, built by `just release-artifacts`). A
   ~100-line script — detect target triple, download the release tgz, verify sha256, extract to
   `~/.local/bin` or `~/.cargo/bin`, run `airlock install` — would give non-Rust users the one
   command without any build-deps story at all. The `just install`/`deps` gate keeps serving
   checkout users, exactly as Homebrew's printed apt/dnf commands serve Linux source-adjacent users
   rather than being automated (§1).
2. **Adopt uv's verification contract: embed per-target sha256 in the script and pin the script URL
   per release version.** uv is the only surveyed installer that verifies in-script (§4), and the
   pinned URL (`…/uv/0.13.8/install.sh`) is what makes auditing meaningful (§6). Airlock's release
   flow can publish `airlock-<target>-v<ver>.tgz.sha256` alongside the tgz and generate the script
   with the hashes baked in. Rustup's own precedent shows publishing checksums nobody's script uses
   is the norm — Airlock can be better than rustup here for ~20 lines of generator.
3. **Never sudo, never root — the surveyed norm matches Airlock's existing invariant.** User-level
   install to a writable prefix with no escalation (§2); Homebrew's FAQ quote is the reasoning to
   cite if anyone asks why the script won't `sudo mkdir /usr/local/bin`.
4. **Don't edit shell rc files; install to a dir already on PATH and print guidance when it isn't.**
   The auto-editers (uv, deno, nvm, pnpm) are tools that own shell startup; Airlock's integration
   surface is COSMIC's config, and a script that silently rewrites rc files would be its own trust
   problem (§3, §6). Print the PATH line Homebrew-style if the chosen dir is missing.
5. **Chain `airlock install` at the end of the script — that is the one-command payoff — but do NOT
   chain it in `just install`.** The pnpm precedent (script runs `pnpm setup --force`, §3) says the
   activation step belongs inside the one command; AppImageLauncher's ask-first dialog and espanso's
   documented service step (§5) say the user must be told what changed and how to undo — Airlock's
   install output plus the byte-for-byte `--uninstall` guarantee already provide both. But this
   repo's own AGENTS.md rule forbids executing the `airlock` binary on the development host because
   it hijacks the live session — `just install` chaining `airlock install` would run it on every
   dev machine invocation. The chain belongs in the end-user curl|sh script, or as an explicit
   opt-in recipe, not the default `just install` behavior.
6. **Skip `--dry-run`.** Not one of the eight offers it (§4); the trust mechanisms with actual
   first-party adoption are inspect-before-run + pinned URLs (uv) and a documented manual-download
   alternative (nvm). A README line showing the non-piped equivalent (`curl -O …; less; sh …`) buys
   more trust for less code.
7. **Uninstall stays documented, not scripted.** Every surveyed tool documents removal as a separate
   manual path (§4), and the famous-tier window managers ship Uninstall sections precisely because
   they rebind system behavior (Rectangle/pop-os/shell — local record
   [readme-famous-switchers.md](readme-famous-switchers.md)). Airlock's `airlock install
   --uninstall` is already stronger than anything surveyed; the script's job is to point at it and
   remove the binary itself (nvm's `rm -rf` + rc-cleanup pattern).
8. **Avoid the appimaged shape for any future integration work:** no silent daemons, no background
   auto-integration, no writes the user wasn't told about (§5). Airlock's install is already
   prompted-by-invocation and reversible — that is the property to preserve as the install story
   grows a script.
9. **Positioning note:** for Pop!_OS/Debian/Ubuntu specifically, the Launchpad PPA deb (local record
   [deb-packaging-launchpad.md](deb-packaging-launchpad.md)) *is* the one-command install — `sudo
   apt install airlock` beats a curl|sh on the target distro, and apt owns updates. The curl|sh
   script's audience is COSMIC users on distros without a PPA line, and `cargo binstall` already
   covers Rust users. Whether that third audience justifies the script's maintenance is the owner's
   call; the surveyed evidence says the script is cheap and conventional if built to contract 2–7.

## Sources

- https://raw.githubusercontent.com/rust-lang/rustup/master/rustup-init.sh — rustup installer source: command checks, no sudo, no checksum, TLS hardening, `--no-modify-path`/`-y`/profile flags, `RUSTUP_VERSION` pinning, noexec-/tmp error
- https://releases.astral.sh/installers/uv/latest/uv-installer.sh — uv installer source: `need_cmd` checks, no sudo, rc env-script editing, embedded per-platform sha256 verification, hardcoded version + `UV_DOWNLOAD_URL`/`UV_NO_MODIFY_PATH`/`UV_UNMANAGED_INSTALL`
- https://docs.astral.sh/uv/getting-started/installation/ — uv docs: pinned-version installer URLs, "The installation script may be inspected before use", upgrade re-runs can modify shell profiles, uninstall steps
- https://raw.githubusercontent.com/denoland/deno_install/master/install.sh — deno installer source: unzip/7z check only, no sudo, rc editing delegated to a bundled Deno script, no checksums, version arg
- https://docs.deno.com/runtime/getting_started/installation/ — deno docs: "the install script updates your shell rc file", `. "$HOME/.deno/env"` line, `.sha256sum` files for manual installs, uninstall
- https://raw.githubusercontent.com/starship/starship/master/install/install.sh — starship installer source: no deps installed, sudo only on unwritable bin dir, prints eval instructions, no checksums, `--version`/`--platform`/`--bin-dir` flags
- https://starship.rs/guide/ — starship guide: manual per-shell init lines as a documented step; `cargo install starship --locked`
- https://raw.githubusercontent.com/ajeetdsouza/zoxide/main/install.sh — zoxide installer source: `need_cmd` checks, `try_sudo` fallback, no rc edits, no checksums, always-latest via GitHub API
- https://raw.githubusercontent.com/ajeetdsouza/zoxide/main/README.md — zoxide README: manual eval lines per shell ("Add this to the end of your config file"), package-manager matrix, `cargo install zoxide --locked`
- https://get.pnpm.io/install.sh — pnpm installer source: curl/wget only, no sudo, runs `pnpm setup --force`, no checksums, `PNPM_VERSION` pin, Homebrew-styled helpers credited to Homebrew/install
- https://pnpm.io/cli/setup — pnpm setup docs: "adds the pnpm home directory to the PATH by updating the shell configuration file", standalone scripts run it, `--no-modify-path`, print fallback
- https://raw.githubusercontent.com/nvm-sh/nvm/master/install.sh — nvm installer source: rc appending with dedupe + `PROFILE=/dev/null` escape, no sudo, no checksums, "SOMEONE IS DOING SOMETHING NASTY" warning, env-var pinning
- https://raw.githubusercontent.com/nvm-sh/nvm/master/README.md — nvm README: pinned-tag one-liners, manual uninstall (`nvm unload`, rm, rc-line removal), manual download-inspect alternative
- https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh — Homebrew installer source: version checks + printed per-distro build-tools commands, "Don't run this as root!", narrow `execute_sudo`, printed shellenv line, git-clone install, NONINTERACTIVE
- https://docs.brew.sh/FAQ — Homebrew FAQ: verbatim sudo reasoning, uninstall-script pointer
- https://forge.rust-lang.org/infra/other-installation-methods.html — Rust Forge: GPG-signed standalone installers, "the only way to verify the signatures … is to do so manually"
- https://rust-lang.github.io/rustup/installation/other.html + https://rust-lang.github.io/rustup/installation/index.html — rustup docs: ".sha256" links, `rustup self uninstall`, no review advice
- https://espanso.org/docs/install/linux/ — espanso Linux: deb/AppImage/sudo patterns, "Register espanso as a systemd service (required only once)", `--unmanaged`, Wayland `setcap`
- https://espanso.org/docs/install/mac/ — espanso macOS: dmg drag-to-Applications, brew install
- https://github.com/koekeishiya/yabai/wiki — yabai wiki: accessibility requested on launch + restart, SIP partial-disable for the Dock.app scripting addition, manual quickstart steps
- https://raw.githubusercontent.com/rxhanson/Rectangle/master/README.md — Rectangle README: brew cask, accessibility reset via `tccutil`, Uninstallation section (`defaults delete`, `brew uninstall --zap`)
- https://raw.githubusercontent.com/TheAssassin/AppImageLauncher/master/README.md — AppImageLauncher: desktop-entry integration via first-run dialog, appimaged daemon contrast, remove-undoes-integration, Lite edition without root
- https://github.com/rust-lang/cargo/issues/11539 — post-install-hook request: ehuss "no mechanism", xtask recommendation, closed as duplicate of #2386
- https://github.com/rust-lang/cargo/issues/2386 — `install.rs` proposal: closed not-planned in favor of #2729, `S-needs-design`, no security rationale recorded
- https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/SUPPORT.md — binstall metadata format: pkg-url/bin-dir/pkg-fmt/overrides only; no hooks, no checksum fields documented
- https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/README.md — binstall FAQ: crates.io-over-HTTPS + crate checksum verification, TLS ≥1.2 enforcement, "we're not running arbitrary code", cargo-install fallback
- Local: justfile:93–103,177–182 (deps gate: distro packages only when missing, rustup never as root, `cargo install --path .`); Cargo.toml:40–43 (binstall release-tgz metadata); AGENTS.md (host-forbidden rule for executing the airlock binary); README.md:110–145 (current install story); docs/research/readme-famous-switchers.md (alt-tab-macos banner README, komorebi docs-site delegation, famous-tier Uninstall/Troubleshooting norms); docs/research/deb-packaging-launchpad.md (PPA verdict reused in verdict 9)

## Unverified

- **komorebi's post-install sequence** — komorebi.com and lgug2z.github.io both failed to load this
  pass (certificate error; reader 500), so its Windows autostart/whkd/AHK steps are not confirmed
  from a primary source; the README-level facts (install delegated to the docs site + video) come
  from the repo's 2026-11-14 famous-switchers record.
- **espanso's macOS Accessibility grant flow** — the macOS install page omits it; search-level
  results point at espanso.org/docs/troubleshooting/secure-input, which was not fetched. The
  Linux service-registration claims are fetched and cited.
- **alt-tab-macos's brew cask one-liner** — the fetched alt-tab.app page shows only a Download
  button and the README is a banner; no first-party `brew install --cask alt-tab` line was
  confirmed this pass, so no claim is made about it.
- **rustup-init's rc-modification behavior** — inferred from the `--no-modify-path` flag in
  rustup-init.sh's usage text (the .sh itself never edits rc); the binary's actual rc-writing logic
  was not read.
- **"cargo rejected install scripts for security reasons"** — the common community framing; the
  primary threads (#11539, #2386) record closure-as-duplicate and needs-design with no security
  rationale stated by maintainers. Recorded as a discrepancy between summaries and sources; if a
  first-party security statement exists elsewhere (e.g. #2729), it was not fetched.
- **pnpm `pnpm setup` rc-file specifics** — the docs say "the shell configuration file" without
  naming per-shell files; only the fallback-print behavior is verbatim.
- **Whether `~/.local/bin` is on PATH in a default Pop!_OS/COSMIC session** — asserted by verdict 4
  as the usual user-bin convention (uv installs there) but not verified against Pop!_OS defaults;
  the script should check and print rather than assume.
