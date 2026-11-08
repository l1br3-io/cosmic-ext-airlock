# Airlock — Flathub publishing: submission path, COSMIC precedents, sandbox feasibility

**Date:** November 8, 2026 · **Method:** primary-source web pass (docs.flathub.org author docs, Flathub
manifest repos fetched raw, pop-os/xdg-desktop-portal-cosmic source listing, flatpak docs, xdg-desktop-portal
portal spec) plus a read of this repository's committed sources (`src/install.rs`, `src/main.rs`,
`src/wayland.rs`, `Cargo.toml`, `README.md`). Quotes are verbatim; every claim cites its URL or file.
This record is closed; corrections go in a new record, not edits here.

## 1. The question and the prior verdict

Can Airlock — a workspace-isolated Alt+Tab daemon that rewrites COSMIC's shortcuts config and paints
layer-shell overlays — be published to Flathub, and what would it cost? A prior record declared Flathub
a dead end ("a sandboxed global-keybind daemon fights the Flatpak model",
[whats-next-post-v0.1.0.md](whats-next-post-v0.1.0.md) §5, citing the then-README). The README has since
moved Flathub onto the roadmap: "A Launchpad PPA and Flathub are on the roadmap without dates"
(README.md:148–150). This record re-opens the question with the primary sources the earlier verdict
lacked, and finds a materially more nuanced answer: **the Flathub submission itself is unremarkable —
the keybind-capture architecture is the hard part, and it has a plausible-but-unproven sandbox shape.**

## 2. Verdicts

| # | Question | Verdict |
|---|----------|---------|
| 1 | Is there a COSMIC runtime/SDK on Flathub? | **No.** COSMIC apps pin `org.freedesktop.Platform` (or `org.gnome.Platform`) + `org.freedesktop.Sdk.Extension.rust-stable`, build libcosmic from source in the app module, and share the icon theme via `com.system76.Cosmic.BaseApp` (§5) |
| 2 | Will Flathub accept `--filesystem=xdg-config/cosmic:rw` + `--talk-name=com.system76.CosmicSettingsDaemon`? | **Yes, with precedent** — CosmicTweaks and cosmic-ext-color-picker ship exactly that (§4). Airlock's config rewrite is not the blocker |
| 3 | Is the keybind-spawn-per-press architecture sandboxable? | **Unproven; the real risk.** COSMIC must spawn `flatpak run --command=airlock …` per keypress (latency never measured); no portal fallback exists because xdg-desktop-portal-cosmic does not implement GlobalShortcuts (§6) |
| 4 | Is there precedent for a sandboxed exclusive-keyboard layer-shell overlay on Flathub? | **Not found.** Every Flathub COSMIC app renders a normal toplevel; no maintained Flathub app was found doing Airlock's layer-shell + keyboard-shortcuts-inhibit dance (§6.3) |
| 5 | Best distribution path today | **crates.io + GitHub Releases + Launchpad PPA (already researched), Flathub as a measured prototype** — build the manifest, test the `flatpak run` keybind latency in the VM, submit only if the numbers hold (§9–§10) |

## 3. The first-submission path

### 3.1 App ID — and a problem with ours

Flathub requires reverse-DNS IDs of 3–5 components: "the reverse-DNS format {tld}.{vendor}.{product}";
"at least 3 components. Applications must not exceed 5 components"; "A dash - is only allowed in the last
component"; "The domain portion must be in lowercase and must convert dash - to underscore _"; "The ID
must exactly match the ID tag in Metainfo file"; GitHub-hosted apps use the `io.github.` prefix
(https://docs.flathub.org/docs/for-app-authors/requirements).

**Flag:** the README commits to "App ID `io.github.l1br3-io.Airlock` — the hyphen matches the GitHub org"
(README.md:148–149). That puts a dash in the *third* component, which the rule above disallows ("only
allowed in the last component"). The conservative reading is `io.github.l1br3_io.Airlock` (dash →
underscore) — but whether GitHub *verification* then still matches the `l1br3-io` account name is not
stated in the verification docs (https://docs.flathub.org/docs/for-app-authors/verification) and must be
checked against a real verified app with a hyphenated GitHub account before anything is committed
(Unverified §11.1). Also note `com.system76.` is a protected prefix ("pop-os org on GitHub" only) —
Airlock cannot and should not want it.

### 3.2 Required metadata

Per the MetaInfo guidelines (https://docs.flathub.org/docs/for-app-authors/metainfo-guidelines):

- **id** "must be exactly the same as the Application-ID"; file at
  `/app/share/metainfo/%{id}.metainfo.xml` (the `.appdata.xml` path is deprecated).
- **name/summary/description** — "A short and informative description must be present"; only `p`, `ol`,
  `ul`/`li`, `em`, `code` children.
- **Licenses** — "All MetaInfo must have a metadata license tag that is the license of the MetaInfo file
  itself" (CC0-1.0 or FSFAP typical), "Followed by a mandatory project_license tag" with SPDX
  identifiers. Airlock: GPL-3.0-only (Cargo.toml:6).
- **Developer tag** "with an id attribute and a name child tag must be present."
- **Launchable** — "All graphical applications having a desktop file must have this tag"; one only.
- **URLs** — "At minimum url type="homepage" tag must be present"; `vcs-browser` "highly recommended".
- **Releases** — "Applications must supply a releases tag in their MetaInfo to pass validation"; dates
  "must not be in the future and versions must be properly ordered."
- **OARS** — "properly tagged by OARS data based on their content" using `type="oars-1.1"` (Airlock is
  all-nil on the OARS questionnaire).
- **Screenshots** — "All graphical applications must have one or more screenshots"; direct web links,
  git-hosted images "from a tag or a commit and not a branch."

Plus the general requirements (https://docs.flathub.org/docs/for-app-authors/requirements): "All
graphical applications must include a desktop file"; icons must be "preferably a SVG icon or at least a
256x256 PNG icon, properly named and installed" (Airlock already has both, placed by `airlock install` —
the flatpak packaging places them via the manifest instead); license files install to
`$FLATPAK_DEST/share/licenses/$FLATPAK_ID`.

Validation: "All MetaInfo files included in build must pass validation" via `flatpak-builder-lint`, and
"Both errors and warnings are considered to be fatal by appstreamcli and needs to be resolved"
(metainfo-guidelines, above). The linter runs "four checks … the appstream check, the manifest check, the
builddir check and the repo check" (https://github.com/flathub/flatpak-builder-lint).

### 3.3 Permissions policy

"Static permissions must be kept to an absolute minimum. Applications should rely on XDG Portals and
follow established XDG standards wherever possible." When a portal covers the use case, "using the portal
becomes mandatory instead of static permissions"; exceptions are case-by-case and "using niche or
unmaintained setups or dependencies may not count as grounds for allowing an exception." Submissions must
"explain why the app needs its permissions in the pull request"
(https://docs.flathub.org/docs/for-app-authors/requirements,
https://docs.flathub.org/docs/for-app-authors/submission).

Airlock's defensible position: the portal that *would* cover its use case (GlobalShortcuts) does not
exist on COSMIC (§6.2), so static permissions (`--socket=wayland`, `--filesystem=xdg-config/cosmic:rw`)
are the only mechanism — and the COSMIC precedents in §4 show reviewers accept that pattern.

### 3.4 PR workflow and tooling

From the submission doc (https://docs.flathub.org/docs/for-app-authors/submission):

- Install the builder: `flatpak install -y flathub org.flatpak.Builder`; build locally with
  `flatpak run --command=flathub-build org.flatpak.Builder --install <manifest>`; test with
  `flatpak run <app id>`; lint with `flatpak run --command=flatpak-builder-lint org.flatpak.Builder
  manifest <manifest>`.
- Submission is a GitHub PR: fork `flathub/flathub` (with "Copy the master branch only" unchecked),
  clone the `new-pr` branch, PR title "Add org.example.MyAwesomeApp" — and "do **not** open the PR
  against master — target the **`new-pr`** base branch."
- CI test build is triggered by commenting "`bot, build`" on the PR.
- "Never close the PR to address review comments, including ID changes"; "Pull requests that remove the
  template, does not follow the submission guidelines or contain excessive AI-generated" content may be
  closed without review (the Generative AI policy is explicitly linked).
- On approval the app "is merged into a new repo under the Flathub GitHub org" with a write-access
  invite (2FA required "within one week"); builds "publish in usually 1-2 hour". "Reviewers are
  volunteers and the response time may vary."

On the task's premise of `flatpak run org.flathub.Publisher`: **no such app could be found** — the Flathub
docs never mention it and web searches return nothing for the ID (Unverified §11.2). The actual
machinery is the pieces cited above: `org.flatpak.Builder` + `flatpak-builder-lint`, the `bot, build`
PR command, and [flatpak-external-data-checker](https://github.com/flathub/flatpak-external-data-checker)
for update automation. Post-merge identity uses the verification system: website
(`.well-known/org.flathub.VerifiedApps.txt`), DNS TXT at `_flathub.<domain>`, or source-hosting login —
for GitHub, "the authenticated account must be the owner of the repository. If the repository is owned by
an organization, the account must be an admin of the organization"
(https://docs.flathub.org/docs/for-app-authors/verification).

## 4. Precedents: COSMIC apps on Flathub

The flathub GitHub org holds ~11 repos matching "cosmic"
(https://api.github.com/search/repositories?q=cosmic+user:flathub —
`com.system76.Cosmic.BaseApp`, `dev.edfloreshz.CosmicTweaks`, `com.francescogaglione.cosmicmoney`,
`fr.nytuo.cosmiccomics`, `com.vkhitrin.cosmicding`, `org.cosmic_utils.enroll`,
`io.github.cosmic_utils.camera`, `io.github.cosmic_utils.Examine`, `io.github.chimi70.cosmic-expansion`,
`io.github.pixeldoted.cosmic-ext-color-picker`, plus one archived EOL app). System76's own epoch apps are
**not** on Flathub; the population is community apps, including one `cosmic-ext-*`.

Three manifests read raw:

**dev.edfloreshz.CosmicTweaks** (https://raw.githubusercontent.com/flathub/dev.edfloreshz.CosmicTweaks/master/dev.edfloreshz.CosmicTweaks.json)
— edits COSMIC's config, the closest analog to Airlock's host-config writes:

```json
"runtime": "org.freedesktop.Platform", "runtime-version": "26.08",
"sdk": "org.freedesktop.Sdk",
"sdk-extensions": ["org.freedesktop.Sdk.Extension.rust-stable"],
"base": "com.system76.Cosmic.BaseApp", "base-version": "stable",
"finish-args": [
  "--share=ipc", "--socket=fallback-x11", "--socket=wayland", "--device=dri",
  "--share=network",
  "--talk-name=com.system76.CosmicSettingsDaemon",
  "--filesystem=xdg-config/cosmic:rw"
],
"build-options": { "append-path": "/usr/lib/sdk/rust-stable/bin",
  "env": { "CARGO_HOME": "/run/build/tweaks/cargo" } }
```
Module: git cosmic-utils/tweaks pinned to a commit + `cargo-sources.json`, built `cargo --offline build
--release --verbose`.

**io.github.pixeldoted.cosmic-ext-color-picker**
(https://raw.githubusercontent.com/flathub/io.github.pixeldoted.cosmic-ext-color-picker/master/io.github.pixeldoted.cosmic-ext-color-picker.json)
— same skeleton (`org.freedesktop.Platform` 24.08, rust-stable, BaseApp, `--talk-name=
com.system76.CosmicSettingsDaemon`, `--filesystem=xdg-config/cosmic:rw`), upstream pins libcosmic from
git (`pop-os/libcosmic` rev 2aa9133, built inside the app module). It depends on `ashpd` (portals).

**cosmic-nightlight** (upstream's own packaging,
https://raw.githubusercontent.com/cosmic-nightlight/cosmic-nightlight/main/flatpak/io.github.cosmic_nightlight.json)
— `org.freedesktop.Platform` 25.08, BaseApp, and the most aggressive permission set of the three:
`--filesystem=xdg-config/cosmic:rw`, `--filesystem=xdg-config/autostart:create`,
`--talk-name=com.system76.CosmicSettingsDaemon`, `--talk-name=com.system76.CosmicSettingsDaemon.Config.*`,
`--talk-name=org.freedesktop.Flatpak`.

**The pattern is uniform:** freedesktop or GNOME runtime, rust via the Sdk extension, app built from a
git checkout with vendored crates (`cargo-sources.json`, generated by
[flatpak-builder-tools' cargo generator](https://github.com/flatpak/flatpak-builder-tools)), libcosmic
compiled per-app, BaseApp for the shared icon theme, and — where the app touches COSMIC state —
`xdg-config/cosmic:rw` + the SettingsDaemon bus name. Airlock's *config* story fits this pattern exactly.

## 5. Runtime and SDK: what COSMIC apps actually pin

There is **no COSMIC SDK runtime and no cosmic Sdk-extension on Flathub.** The thing that carries the
COSMIC name is a *base app* — `com.system76.Cosmic.BaseApp`
(https://github.com/flathub/com.system76.Cosmic.BaseApp) — whose manifest
(https://raw.githubusercontent.com/flathub/com.system76.Cosmic.BaseApp/master/com.system76.Cosmic.BaseApp.json)
is:

```json
"id": "com.system76.Cosmic.BaseApp",
"runtime": "org.freedesktop.Platform", "sdk": "org.freedesktop.Sdk",
"runtime-version": "25.08", "separate-locales": false
```
with three content modules: a prebuilt musl `just` binary (casey/just 1.49.0), `pop-icon-theme` v3.5.1
(pop-os tag), and `cosmic-icons` epoch-1.0.9 (`just prefix=/app install`), plus a cleanup script.

So the BaseApp ships **no libcosmic, no smithay, no cosmic-protocols** — only the icon theme and just.
Each app compiles libcosmic (or, in Airlock's case, cosmic-client-toolkit) itself against rust-stable.
Note the BaseApp manifest pins `runtime-version: "25.08"` while CosmicTweaks runs on `"26.08"` — bases
are version-flexible ("base-version": "stable" tracks the current one), but this is worth re-checking at
submission time. Runtimes in current use, verbatim from §4's manifests: `org.freedesktop.Platform` 24.08,
25.08, 26.08; `org.gnome.Platform` 49 (dev.heppen.webapps,
https://raw.githubusercontent.com/flathub/dev.heppen.webapps/master/dev.heppen.webapps.json). End-of-life
runtimes are grounds for rejection (https://docs.flathub.org/docs/for-app-authors/requirements).

For Airlock this is simpler than the precedents: **no libcosmic dependency at all** (PRODUCT.md,
Constraints; Cargo.toml:33 pins only `cosmic-client-toolkit = "0.2.0"`, rustc floor 1.93 — well within
the rust-stable extension's rolling toolchain).

## 6. The hard problems for Airlock specifically

### 6.1 How Airlock captures Alt+Tab today (the architecture in play)

`airlock install` rewrites "the user's `system_actions` and `custom` shortcuts files" at
`~/.config/cosmic/com.system76.CosmicSettings.Shortcuts/v1/{system_actions,custom}` and drops the starter
config + desktop entry + icon (src/install.rs:1–12, :401–404). COSMIC's keybind daemon then *spawns the
stub binary per keypress* (`airlock alt-tab` etc., src/main.rs:43–47), which unix-socket-IPC's the
gesture to the resident daemon and auto-spawns it if dead (src/main.rs:71–104). The daemon talks raw
Wayland to cosmic-comp: wlr-layer-shell surfaces (`KeyboardInteractivity`, src/wayland.rs:25–28),
`zwp_keyboard_shortcuts_inhibit_manager_v1` to hold the keyboard while Alt is held (src/wayland.rs:41–46),
and the cosmic/ext toplevel+workspace protocols for listing and activation (src/wayland.rs:31–32;
full list at src/wayland.rs:112–144 per whats-next §4.2). Measured UX: "Warm press round-trip < 50 ms" —
"< 10 ms" (README.md:140–141); "Cold press (respawn + settle) ~340 ms".

### 6.2 No portal fallback exists on COSMIC

The GlobalShortcuts portal (`org.freedesktop.portal.GlobalShortcuts`, "version 2", shortcuts that fire
"regardless of the focused state of the application window",
https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.GlobalShortcuts.html) is
implemented by KDE and Hyprland's portals, not by xdg-desktop-portal-wlr, and GNOME's support landed
late (secondary sources; see Sources). **The decisive primary fact: pop-os/xdg-desktop-portal-cosmic's
src/ contains no GlobalShortcuts implementation** — the module listing is access, app, documents,
file_chooser, remote_desktop (+dialog, +ei), screencast (+dialog, +thread), screenshot, subscription
(https://api.github.com/repos/pop-os/xdg-desktop-portal-cosmic/contents/src), and main.rs registers only
Settings besides those (https://raw.githubusercontent.com/pop-os/xdg-desktop-portal-cosmic/master/src/main.rs).
So a sandboxed Airlock **cannot** receive its keybinds through a portal on COSMIC today — there is no
portal to ask. Until cosmic implements GlobalShortcuts, the only capture mechanism is COSMIC's own
keybind config spawning a command (§6.4) or nothing.

### 6.3 The overlay itself: layer-shell through the sandboxed socket

`--socket=wayland` is documented simply as "Show windows with Wayland"
(https://docs.flatpak.org/en/latest/sandbox-permissions.html); the docs do not claim protocol filtering,
and in practice the sandbox gets the compositor's socket, on which wlr-layer-shell and
keyboard-shortcuts-inhibit are ordinary protocols the compositor offers to any client (compositor-side
filtering would require the security-context protocol, which cosmic-comp has not been verified to
implement — Unverified §11.3). Functionally, a sandboxed client should be able to bind the same
protocols unsandboxed Airlock binds — **but no maintained Flathub precedent was found for a sandboxed
exclusive-keyboard layer-shell overlay.** The COSMIC Flathub population renders normal toplevels
(color-picker uses libcosmic's winit toplevels, no layer-shell features — upstream Cargo.toml,
https://raw.githubusercontent.com/PixelDoted/cosmic-ext-color-picker/master/Cargo.toml); the nearest
widget-ish app, Monitorets (https://flathub.org/en/apps/io.github.jorchube.monitorets), pins
`org.gnome.Platform` with `--socket=wayland` + `--filesystem=host:ro`
(https://raw.githubusercontent.com/flathub/io.github.jorchube.monitorets/master/io.github.jorchube.monitorets.json)
but shows no layer-shell usage in its repo either. Honest verdict: **probably feasible, entirely
unproven** — the compositor may treat sandboxed clients differently in future, and reviewers have no
reference app to compare against.

### 6.4 The real blocker: the keybind command must run from outside

Inside a flatpak, COSMIC's shortcuts config can still be rewritten (`--filesystem=xdg-config/cosmic:rw`,
precedent §4), but the command it must name has to be spawnable by the *host* keybind daemon. That means
the config entry becomes something like
`flatpak run --command=airlock io.github.<id>.Airlock alt-tab` — i.e. **every Alt+Tab press pays
`flatpak run` process/mount-namespace setup before the stub even execs**, on the path that today measures
"< 10 ms". The magnitude is unmeasured (Unverified §11.4); `flatpak run` is a heavyweight launcher, and
Alt+Tab latency is the product's headline number. The daemon+IPC design keeps this to one stub spawn per
press (the daemon does not restart), which is the architecture's saving grace — but the stub itself
becomes the risk.

Mitigations, in decreasing honesty:

1. **Measure first.** `flatpak run --command=true <id>` timing in the COSMIC VM is a one-hour
   experiment that decides the whole question (§10).
2. **GlobalShortcuts portal when cosmic ships it** — clean, sandbox-native, and then the sandboxed app
   binds shortcuts itself; requires xdg-desktop-portal-cosmic work upstream (§6.2) and a cctk/portal
   client in Airlock. This is the only design that removes the per-press host spawn.
3. **`--talk-name=org.freedesktop.Flatpak`** (used by cosmic-nightlight, §4) lets the app talk to the
   host flatpak D-Bus — nominally enabling `flatpak spawn --host` from inside. But spawning the gesture
   handler on the host puts the binary outside the sandbox on every press anyway; it satisfies reviewers
   less than plain static permissions would. Not recommended.
4. **Keep the daemon's overlay and activation path sandboxed, accept a tiny host-side helper** —
   that is no longer a flatpak of the whole app and fails the point.

Also inside-sandbox (mechanical, not blockers): the single-instance IPC socket lives in the sandbox's own
`XDG_RUNTIME_DIR`, shared by stub and daemon — fine; icon lookup would read the BaseApp's
pop-icon-theme/cosmic-icons in `/app` rather than the host user's theme (`~/.local/share/icons` needs an
extra `--filesystem` grant — a visible divergence worth documenting); `airlock list` and
`airlock install`/`uninstall` become `flatpak run --command=…` invocations.

## 7. Alternatives if the sandbox fights back

- **Launchpad PPA (debs)** — already fully researched with verdicts in
  [deb-packaging-launchpad.md](deb-packaging-launchpad.md): vehicle = source builds chained on
  `ppa:rust-toolchain/staging` (in-archive rustc tops at 1.91 < the crate's 1.93 floor), amd64+arm64,
  vendored tree 128 crates / 14 MB xz against the 8 GiB quota; fallback = reprepro + GitHub Pages +
  cargo-deb. A PPA deb also appears in COSMIC Store on Pop, since the Store "lists apps from configured
  software repositories — by default Flathub plus the Ubuntu/Pop!_OS repos"
  (https://system76.com/support/managing-apps).
- **GitHub Releases** (tarball + prebuilt debs) — already shipping tags/notes
  (https://github.com/l1br3-io/cosmic-ext-airlock/releases); zero review, zero latency guarantees for
  users, but no auto-updates.
- **OBS** — multi-distro builds "for a wide range of operating systems and hardware architectures",
  "over 140.000 packages for 21 base distributions on 6 architectures"
  (https://openbuildservice.org/about/) — the wiki page detailing deb targets was not reachable
  (Unverified §11.5); the deb record's PPA/fallback paths dominate for Airlock's Pop-first audience.
- **Flathub with the permission set of §4** — accepted pattern, and the only path into COSMIC Store's
  Flathub listing for users who install via Store.

What the COSMIC community does: System76 enables Flathub out of the box in the Store
(https://system76.com/support/managing-apps), desktop components ship as native packages, and third-party
COSMIC apps have settled on Flathub (§4) or cargo-install (whats-next §4.4: "GitHub-and-build-from-source
is the niche's norm"). No COSMIC-epoch-era official app is on Flathub to imitate.

## 8. Local authoring, testing, CI

- **Local loop** (submission doc, §3.4): `flatpak install -y flathub org.flatpak.Builder` →
  `flatpak run --command=flathub-build org.flatpak.Builder --install <manifest>` →
  `flatpak run <app id>` → `flatpak run --command=flatpak-builder-lint org.flatpak.Builder manifest
  <manifest>` (plus a repo variant). Manifest inputs: git source pinned to tag+commit, plus
  `cargo-sources.json` generated by the cargo tool of
  [flatpak-builder-tools](https://github.com/flatpak/flatpak-builder-tools) ("refer to the individual
  tool subdirectories for documentation"). Note "There is no network access during the build process"
  (requirements) — the vendored-crates file is mandatory for cargo, exactly like the PPA's vendor story
  (deb record §5).
- **CI** (https://docs.flathub.org/docs/for-app-authors/github-actions): own-repo apps use
  [flatpak/flatpak-github-actions](https://github.com/flatpak/flatpak-github-actions) —
  `flatpak-builder` (inputs: `manifest-path`, `bundle`, `arch`, cache keys, `stop-at-module`, …) and
  `flat-manager` for repo uploads; example container image `ghcr.io/flathub-infra/flatpak-github-actions:gnome-48`
  with `--privileged`; aarch64 needs ARM64 runners or QEMU. Apps *inside* the flathub org may only use
  "Any action from the flathub organization" plus `peter-evans/create-pull-request`. Same-repo CI can run
  the manifest on every push before a Flathub PR is ever opened.

## 9. Effort and risk ranking

| Path | Effort | Risk | Notes |
|------|--------|------|-------|
| crates.io | done | — | 404 → published since whats-next §2.2 |
| GitHub Releases + PPA deb | medium, spec'd in deb record | low-mechanical | auto-updates via apt; no Store friction |
| Flathub, manifest + metainfo only | 1–2 days | low | pattern is copy-paste from §4 precedents |
| Flathub, keybind path proven in VM | +1 day VM experiment | **unproven** | decides `flatpak run` per-press latency (§6.4) |
| Flathub, portal-based keybinds | +weeks, gated upstream | blocked on cosmic | xdg-desktop-portal-cosmic has no GlobalShortcuts (§6.2) |
| AUR / OBS | small / medium | low | community-bait per whats-next §4.4; OBS deb-target docs unreachable |

## 10. Recommendation

Flathub is no longer a dead end on evidence — the config-rewrite precedent (§4) dissolves the old
verdict's stated reason — but the keybind-capture architecture keeps it a **measured experiment first**:

1. **Resolve the app ID now** (it is already in the README): verify against Flathub validation and a real
   verified hyphenated-GitHub-account app whether `io.github.l1br3-io.Airlock` passes or must become
   `io.github.l1br3_io.Airlock` (§3.1). Every later step keys off this.
2. **Run the one-hour latency experiment in the COSMIC VM**: install any flatpak, set a COSMIC custom
   shortcut to `flatpak run --command=true <id>`, measure warm-press delta vs the current <10 ms stub
   (§6.4). This single number decides whether to proceed.
3. **If the number holds**: author the manifest on the §4 skeleton (freedesktop runtime + rust-stable +
   BaseApp + `--socket=wayland` + `--filesystem=xdg-config/cosmic:rw` +
   `--talk-name=com.system76.CosmicSettingsDaemon`), generate `cargo-sources.json`, write the metainfo
   (releases, OARS, screenshots — the screenshots already flagged as missing in whats-next §3.1 become
   mandatory), lint, build locally, and open the PR against `flathub/flathub`'s `new-pr` branch with the
   permissions justification up front (§3.3–3.4).
4. **If it doesn't**: ship the PPA path (already spec'd) and file/track GlobalShortcuts support in
   xdg-desktop-portal-cosmic; revisit Flathub when a portal-based Airlock client is plausible — that is
   the only design that removes the per-press host spawn.
5. **Either way, do not let Flathub gate the release**: it is one channel among four, and the only one
   with volunteer-reviewer latency measured in months.

## Sources

- https://docs.flathub.org/docs/for-app-authors/requirements — app-id rules, desktop file, icons, licenses, permission policy, EOL runtimes
- https://docs.flathub.org/docs/for-app-authors/metainfo-guidelines — metainfo required tags, OARS, screenshots, validation
- https://docs.flathub.org/docs/for-app-authors/submission — org.flatpak.Builder build, lint, `new-pr` PR workflow, `bot, build`, post-merge flow
- https://docs.flathub.org/docs/for-app-authors/verification — website/DNS/GitHub verification mechanics
- https://docs.flathub.org/docs/for-app-authors/github-actions + https://github.com/flatpak/flatpak-github-actions — CI action, inputs, flathub-org action restrictions
- https://github.com/flathub/flatpak-builder-lint — the four lint checks
- https://github.com/flatpak/flatpak-builder-tools — cargo-sources.json generator
- https://github.com/flathub/com.system76.Cosmic.BaseApp + its manifest — the COSMIC BaseApp: icons only, no libcosmic
- https://raw.githubusercontent.com/flathub/dev.edfloreshz.CosmicTweaks/master/dev.edfloreshz.CosmicTweaks.json — CosmicTweaks manifest (xdg-config/cosmic:rw precedent)
- https://raw.githubusercontent.com/flathub/io.github.pixeldoted.cosmic-ext-color-picker/master/io.github.pixeldoted.cosmic-ext-color-picker.json + upstream Cargo.toml — cosmic-ext-* precedent, libcosmic-from-source
- https://raw.githubusercontent.com/cosmic-nightlight/cosmic-nightlight/main/flatpak/io.github.cosmic_nightlight.json — most aggressive COSMIC permission set on Flathub
- https://raw.githubusercontent.com/flathub/dev.heppen.webapps/master/dev.heppen.webapps.json — GNOME-runtime variant, CEF-enabled COSMIC app
- https://api.github.com/search/repositories?q=cosmic+user:flathub — the Flathub COSMIC population
- https://api.github.com/repos/pop-os/xdg-desktop-portal-cosmic/contents/src + .../src/main.rs — no GlobalShortcuts backend on COSMIC
- https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.GlobalShortcuts.html — portal spec, version 2
- https://docs.flatpak.org/en/latest/sandbox-permissions.html — --socket=wayland, --talk-name, --filesystem=xdg-config semantics
- https://flathub.org/en/apps/io.github.jorchube.monitorets + its manifest — nearest non-toplevel candidate (no layer-shell found)
- https://system76.com/support/managing-apps — COSMIC Store sources: Flathub + system repos
- https://openbuildservice.org/about/ — OBS multi-distro positioning
- This repository: src/install.rs:1–12, :401–404; src/main.rs:43–104; src/wayland.rs:25–46; Cargo.toml:6,33; README.md:140–150; docs/research/whats-next-post-v0.1.0.md §4.4, §5; docs/research/deb-packaging-launchpad.md

**Secondary (used only where marked):** web-search summaries of GlobalShortcuts backend support
(KDE/Hyprland implement; xdg-desktop-portal-wlr does not; GNOME landed late —
https://gitlab.gnome.org/GNOME/xdg-desktop-portal-gnome/-/merge_requests/208), and the Reddit/rustutils
community threads in §7's context.

## Unverified

- **§11.1 App-ID dash normalization** — that `io.github.l1br3-io.Airlock` fails validation is the
  conservative reading of the requirements quote ("A dash - is only allowed in the last component"); no
  verified Flathub app with a hyphenated *middle* component was found either way, and the docs do not
  state how GitHub verification maps account dashes. Must be settled by experiment or a Flathub issue
  before the README's app-id line is treated as final.
- **§3.4 `org.flathub.Publisher`** — the task premise; no trace found in Flathub docs, the Flathub site,
  or search. Recorded as likely nonexistent; the real tooling is documented above.
- **§6.3 security-context** — whether cosmic-comp implements Wayland security-context (and could
  therefore restrict sandboxed clients' protocol access) was not checked against cosmic-comp source.
- **§6.4 `flatpak run` per-press latency** — magnitude never measured; only the experiment in §10.2
  answers it.
- **§7 OBS deb targets** — the Open Build Service intro page confirms multi-distro/arch builds but does
  not name deb/Ubuntu support on the fetched page; the wiki page was unreachable (403). The deb record's
  PPA path does not depend on this.
- **"cosmic-iteration" package lists** (task premise) — no such repo/org was located; the nearest real
  anchors used instead are System76's Store docs and the flathub org search (§4, §7).
