# What's next after v0.1.0 — research findings

**Date:** November 3, 2026 · **Scope:** candidate roadmap immediately following the v0.1.0 packaging milestone — tag `v0.1.0` at fe0c775 (2026-11-02 03:42Z), GitHub Release published 03:59Z the same day, repository HEAD at 79b0948 (2026-11-03) · **Method:** primary sources only — this repository's committed files and git history, live checks against the crates.io and GitHub APIs (2026-11-03), and the upstream pop-os / wayland-protocols trackers. Inference is marked; every other claim cites a file, commit, or URL.

---

## 1. TL;DR — the next-step ledger

| # | Candidate | Priority | Basis |
|---|-----------|----------|-------|
| 1 | Finish the release: hardware/VM pass on HEAD, re-cut the `v0.1.0` tag + GitHub Release, `cargo publish`, verify the README install path | **Now** | Tag is 10 commits behind HEAD; the Release body is stale; crates.io returns 404 while the README says `cargo install cosmic-ext-airlock` (§2) |
| 2 | Act on the 2026-11-02 security-audit sweep: pin `unsafe_code = "forbid"`, bound the untrusted-input surfaces | **Now** | Findings verified directly in `src/icons.rs` and `src/daemon.rs`; no commit after the audit addresses them (§2.4) |
| 3 | Add CI (GitHub Actions) running the headless gates | High | No `.github/` exists; `just test` / `just check` / `just fmt-check` are local-only (justfile:21–35) (§2.5) |
| 4 | Real-hardware screenshots in the README | High | The README describes the overlay in prose only; the product context (PRODUCT.md, "Evidence on Hand") lists "no screenshots of the new overlay on real hardware yet" as an absence future work must not fabricate (§3.1) |
| 5 | Cap visible rows / page long lists in the overlay | Medium | Research verdict 12 of `switcher-ux-and-icons.md` (≤ ~70% output height, page beyond ~8 rows); `src/ui.rs` caps width only — no row cap exists (§3.2) |
| 6 | Engage upstream: link Airlock in cosmic-epoch#1777 and track it | Medium | The feature request is still open (31 comments, updated 2026-08-17); the dock's app list gained workspace filtering 2026-04-09 but the switcher has not (§4.1) |
| 7 | Feature candidate: close a window from the switcher | Medium | Open upstream demand (cosmic-epoch#3598) plus the GNOME precedent (q/w keys inside the popup) already documented in `switcher-behavior-adaptation.md` §2.1 (§4.3) |
| 8 | Deferred polish from the research verdicts: scroll-wheel stepping; a real font decision for non-ASCII titles | Low | Verdict 4 of `switcher-behavior-adaptation.md` ("Defer"); PRODUCT.md pins both as deliberate future decisions (§3.3) |
| 9 | Workspace-management expansion (per-workspace app panel, dynamic/static workspaces, pinning) | Low / speculative | Changes what the tool is; docs/settings.md disclaims workspace mutation today; no research record covers feasibility (§3.4, inference) |

Dead ends the evidence rules out: Flathub, thumbnails, type-to-filter, pointer interaction, and dependency-version chasing (§5).

---

## 2. Where the release actually stands

### 2.1 The tag and the tree disagree

`git rev-list v0.1.0..HEAD` counts **10 commits** landed after the tag, and they are
not noise — they are the release's headline content:

- `0007bf5` (2026-11-02 13:28 +0800) — `airlock install` now places the app icon
  and desktop entry itself; the README's one-command install story (78809c5)
  documents this feature.
- `53c8e91` / `fd43256` / `1cafd4e` (15:18–15:22) — the markerless
  uninstall-restore fix (per key, not per file) plus the contract-matrix tests
  and README contract.
- `7a82b5e` (15:52) — typographic punctuation renders instead of degrading to `?`.
- `f5dadbe` (20:29) — switcher text re-pinned from 12 px to 8 px.

The published GitHub Release body
(https://github.com/l1br3-io/cosmic-ext-airlock/releases/tag/v0.1.0, fetched
2026-11-03) still says "Text renders at **12 px**", does not mention the
icon/desktop-entry self-install, and predates the uninstall-restore fix. Anyone
building the tagged tarball gets a materially older product than `main` — one
whose `airlock install` does not place assets and whose uninstall restore is
known-buggy per 53c8e91's own message.

The remedy: re-cut `v0.1.0` — move the tag and rewrite the Release to HEAD —
rather than ship a 0.1.1 hours after 0.1.0. The release is hours old, has no
other consumers, and the crate has not been published at all (§2.2), so
nothing can break.

### 2.2 crates.io: the documented install path is currently dead

`https://crates.io/api/v1/crates/cosmic-ext-airlock` returns **404 Not Found**
(fetched 2026-11-03): the crate does not exist on crates.io. Meanwhile:

- README.md:69 — "`cargo install cosmic-ext-airlock   # from crates.io`"
- README.md:156 — "Published to [crates.io](https://crates.io/crates/cosmic-ext-airlock)"
- The v0.1.0 Release body — "Install: `cargo install cosmic-ext-airlock`, then
  `airlock install`."

`cargo publish` is an owner-only manual step that has not been run. Until it
is, the only honest install path is building from a checkout (`just install`),
and the README overclaims. Publish, then re-verify the README instructions
against the published crate (the crate's `exclude` list in Cargo.toml:15–26
means the published source must still build without the repo-only files —
worth one clean `cargo install cosmic-ext-airlock --locked` from a scratch
machine or VM).

### 2.3 No hardware pass covers the post-tag code

The hardware checklist's latest recorded pass is "2026-11-02 — full pass
(sections 1–7) … on the live session with the **overlay-visuals build**"
(docs/hardware-checklist.md, "Latest pass"). That build predates the packaging
commit c92c670, so every user-visible change since — the self-install asset
placement, the per-key uninstall restore, the punctuation fix, the 8-px
re-pin — has **no recorded verification on a real COSMIC session**. The
checklist itself says it is run "always before calling a build release-ready".
One VM session (docs/vm-development.md; snapshot first per AGENTS.md) walking
sections 3–7 against HEAD closes this and feeds the re-cut decision.

### 2.4 The security-audit findings are open

A Trail of Bits rust-review sweep ran 2026-11-02 (the audit skill was vendored
the same day, f76b628). The two commits after it (a79886c docs, 79b0948 test)
address none of the findings. Verified directly in committed source, the
highest-confidence items:

- **`unsafe_code` is enforced by nothing.** The crate is unsafe-free, but no
  `[lints]` table exists in Cargo.toml and no crate root carries
  `#![forbid(unsafe_code)]` — and `cargo clippy -- -D warnings` (justfile:23)
  does not escalate allow-by-default lints. Fix is two lines
  (`[lints.rust] unsafe_code = "forbid"`). Hardening matters because the
  daemon parses untrusted local input on every surface (IPC bytes, hand-edited
  TOML, keybind configs, `.desktop` content).
- **Untrusted SVG parsed without a nesting-depth limit** —
  `src/icons.rs:193`: `resvg::usvg::Tree::from_data(&data, &Options::default())`;
  default options carry no depth cap, and a theme-supplied (or attacker-planted)
  icon with a deep `<g>` chain can stack-overflow the single-threaded daemon.
- **PNG-header-driven allocation is uncapped** — `src/icons.rs:222`:
  `vec![0; reader.output_buffer_size()]` sizes the buffer from the file's own
  declared dimensions; a crafted PNG in an icon path can OOM-kill a 8 MB-RSS
  daemon.
- **IPC connection accumulation is unbounded and one Result is discarded** —
  `src/daemon.rs:427`: `let _ = stream.set_nonblocking(true);` with every
  accepted stream pushed to `pending` forever; a same-user process can exhaust
  fds, and a failed `set_nonblocking` silently turns the poll loop into a
  blocking read that freezes Alt+Tab.
- Softer findings (medium confidence): discarded `conn.flush()` results in
  `src/wayland.rs`, and O(n²) scans per press in `src/ui.rs` / `src/switcher.rs`
  — the latter only matters at toplevel counts no real session reaches.

None of these are remotely exploitable (the threat model is same-user/local),
but they are exactly the class of bug a v0.1.x hardening pass should clear,
and the first one is free.

### 2.5 No CI

The repository has no `.github/` directory — there are no workflows at all.
The verification gates (`cargo test`, `cargo clippy -D warnings`,
`cargo fmt --check`; justfile:21–35) run only on a developer machine. A minimal
GitHub Actions workflow running the three headless gates on push/PR is small,
keeps the crates.io-published source honest, and needs no COSMIC hardware
(AGENTS.md: headless commands are safe; runtime testing stays in the VM).

---

## 3. Unfinished local threads

### 3.1 README has no image of the product

The README describes the overlay in prose (README.md:20–26) and points at the
`overlay-preview` example for a preview PNG. The committed product context
lists "no screenshots of the new overlay on real hardware yet" among the
absences future work must not fabricate (PRODUCT.md, "Evidence on Hand"). A
CP-OV-quality hardware pass already happened (2026-11-02, both wallpapers,
200% scale — docs/hardware-checklist.md), so real screenshots are one capture
session away; for a freshly released community tool, the README screenshot is
the highest-leverage marketing pixel available.

### 3.2 Long-list row cap from the research is not implemented

`switcher-ux-and-icons.md` verdict 12: cap visible rows at ~70% of output
height and page or shrink beyond ~8 rows. `src/ui.rs` implements the width cap
(≤ 600 logical px, `overlay_model_caps_width_at_max_columns`,
src/ui.rs:1105) and per-title truncation (`truncate_cols`, src/ui.rs:600), but
no row-count or height cap exists — a workspace with many windows grows the
card without bound. Low urgency (real workspaces rarely exceed a dozen
toplevels) but it is a documented, pre-scoped backlog item.

### 3.3 Deliberately deferred product decisions

- **Scroll-wheel stepping** — verdict 4 of `switcher-behavior-adaptation.md`:
  "Defer … legitimate future polish, not v1 work"; needs `wl_pointer` plumbing
  that does not exist in `src/wayland.rs`.
- **Non-ASCII titles beyond typographic punctuation** — PRODUCT.md: non-ASCII
  rendering as `?` is "accepted, documented degradation (a real font would be
  a deliberate future decision)". Commit 7a82b5e already bought curly quotes
  and dashes into the hand font; the remaining gap is broader scripts, and the
  cost is the embedded font stack the no-dependency design deliberately
  avoids.
- **MRU persistence across daemon restarts** is in-memory only — a fresh
  daemon starts with compositor-reported focus order (src/daemon.rs MRU
  observe on activation ticks). Every restart-cold press orders by the
  compositor's list until the user interacts. (Inference: acceptable; nobody
  has asked otherwise.)

### 3.4 Workspace-management ambitions

A farther-future direction, distinct from everything above because it changes
what the tool *is*: a per-workspace app panel, dynamic/static workspace
creation, and pinning an app to a workspace. Nothing in the committed research
covers feasibility, and Airlock's own docs disclaim it today ("Airlock never
creates, renames, or rearranges workspaces — names and positions come from
COSMIC itself", docs/settings.md). Treat as a possible v0.2 direction requiring
its own research record first — `cosmic_workspace_v2` does carry workspace
move/pin requests ("workspace-v2: Add pinning and moving requests",
cosmic-protocols commit 2025-04-22), so part of the protocol surface exists.

---

## 4. Upstream ecosystem

### 4.1 The reason Airlock exists is still open upstream

[pop-os/cosmic-epoch#1777](https://github.com/pop-os/cosmic-epoch/issues/1777)
("can we toggle alt-tab window switcher to only show windows on the current
desktop") is **open** — 31 comments, `enhancement` label, last updated
2026-08-17 (GitHub API, 2026-11-03). System76 has not shipped native
workspace-scoped Alt+Tab. Adjacent movement: the dock's app list gained
workspace filtering on 2026-04-09
([pop-os/cosmic-applets#1236](https://github.com/pop-os/cosmic-applets/pull/1236),
"feat(app-list): implement workspace and configured-output filtering",
merged) — a July 2026 comment on #1777 points at it — but the switcher
itself has not moved.

Two implications, both inference:

1. **Growth:** the issue is a 31-comment demand funnel for exactly what
   Airlock ships. A maintainer-appropriate comment linking the v0.1.0 release
   (after publish, §2.2) is the cheapest distribution channel available.
2. **Risk:** if cosmic-comp ever scopes its own switcher, Airlock's core
   feature is absorbed. The durable differentiators are the ones the platform
   default does not offer — per-workspace isolation config, same-app cycling,
   Alt+Esc quick toggle, and the 8 MB footprint (README.md:140–152).

### 4.2 The protocol surface is stable — nothing to adopt, nothing to chase

Airlock binds `zcosmic_toplevel_info_v1` (≥ v3 for workspace membership
events), `zcosmic_toplevel_manager_v1`, `zcosmic_workspace_v2`,
`ext_foreign_toplevel_list_v1`, and `ext_workspace_manager_v1`
(src/wayland.rs:112–144). Checked against upstream (2026-11-03):

- [pop-os/cosmic-protocols](https://github.com/pop-os/cosmic-protocols) —
  the toplevel-info / toplevel-management / workspace XMLs are unchanged
  since a 2025-09-17 DTD fix; 2026 activity added unrelated protocols
  (`cosmic-session-lock-layer-v1` 2026-09-11, `cosmic-keyboard-layout-unstable-v1`
  2026-06-24, corner-radius popup/layer-shell support 2026-06-25).
- [wayland-protocols](https://gitlab.freedesktop.org/wayland/wayland-protocols)
  — `ext-foreign-toplevel-list` and `ext-workspace` sit in `staging/`
  (confirmed via the GitLab repository tree API); **no standardized
  toplevel-activation protocol exists there**, which keeps
  activation COSMIC-specific and keeps "other compositors out of scope"
  (README.md:55) the honest position.
- `cosmic-client-toolkit` 0.2.0 is the newest **and only** crates.io release
  (published 2026-01-08) — the pinned version is current by definition.

One genuinely optional adoption: the corner-radius protocol gained
layer-shell support upstream (2026-06-25); Airlock currently hand-cuts
rounded corners with exact-AA in the software renderer. Matching the
compositor's own corner semantics could be a visual-polish follow-up.
(Inference; no research record covers it.)

### 4.3 Upstream switcher issues that pull at Airlock

All open in pop-os/cosmic-epoch (GitHub API, 2026-11-03):

- [#3598](https://github.com/pop-os/cosmic-epoch/issues/3598) "Allow closing
  a window from the Alt+Tab window switcher" (updated 2026-06-19) — a feature
  Airlock could deliver before the platform does; GNOME's precedent (q/Q
  quits, w/W/F4 closes inside the popup) is already documented in
  `switcher-behavior-adaptation.md` §2.1. Needs a new activation-adjacent
  protocol request (`close` exists on `zcosmic_toplevel_manager_v1`
  capabilities) and one gesture-key arm. (Candidate; inference.)
- [#2422](https://github.com/pop-os/cosmic-epoch/issues/2422) "…slow to
  update the order of windows" and
  [#2424](https://github.com/pop-os/cosmic-epoch/issues/2424) "Wait a moment
  before showing window switcher" — both are complaints about the
  spawn-per-press default that Airlock's resident daemon and quick-tap-first
  design already answer; useful README positioning, not work items.
- [#3258](https://github.com/pop-os/cosmic-epoch/issues/3258) /
  [#1705](https://github.com/pop-os/cosmic-epoch/issues/1705) — custom
  shortcuts for window switching don't dismiss on modifier release. Airlock
  owns its gesture end-to-end and is immune; worth linking from the README's
  positioning when convenient. (Inference.)

### 4.4 Distribution beyond crates.io

Flathub is decided out (README.md:161–163: "a sandboxed global-keybind daemon
fights the Flatpak model"). For scale, the comparable community tool
`cosmic-switch-less` is **not** on crates.io either (404, fetched 2026-11-03)
— GitHub-and-build-from-source is the niche's norm, so a published crate
already puts Airlock ahead of its comparables. Further channels (AUR, a
release tarball with the icon/metainfo) are community-bait once there is
demand, not near-term work. (Inference on priorities; the 404s and README
decision are cited above.)

---

## 5. Dead ends the evidence rules out

- **Flathub** — explicit v1 decision, README.md:154–163. Revisit only on
  demand; a sandboxed daemon cannot rewrite `~/.config/cosmic/…`.
- **Window thumbnails** — "off the table on Wayland for an external client —
  foreign-toplevel protocols are metadata-only by design"
  (`switcher-ux-and-icons.md`, verdict 8 / §2.9). No protocol added since the
  record changes this (§4.2).
- **Type-to-filter in the switcher** — rejected, verdict 7 of
  `switcher-behavior-adaptation.md`; reconfirmed by the UX record (§5.3.4).
- **Pointer interaction** — "keyboard-only is final (owner-confirmed)"
  (PRODUCT.md, Product commitments).
- **Dependency-version chasing** — `cosmic-client-toolkit` 0.2.0 is the only
  published version (crates.io, 2026-01-08); tracking libcosmic/iced releases
  is a non-item because Airlock deliberately has no libcosmic/iced dependency
  (PRODUCT.md, Constraints).
- **Watching for a newer workspace/toplevel protocol version** — unchanged
  upstream since 2025-09 (§4.2); there is nothing to migrate to.

---

## 6. Sources

**This repository (committed):**

1. README.md — install/packaging/troubleshooting claims (lines 53–77, 154–163);
   performance table (140–152)
2. CHANGELOG.md — [0.1.0] 2026-11-02 entry
3. Cargo.toml — version 0.1.0, crate `exclude` list (15–26), dependency pins
4. justfile — gates (21–35), `deps`/`install` (61–136), VM recipes (138–189)
5. docs/hardware-checklist.md — sections 1–7, "Latest pass" note (2026-11-02,
   overlay-visuals build)
6. docs/settings.md — "Airlock never creates, renames, or rearranges
   workspaces"
7. docs/vm-development.md — "What the VM proves — and what it can't"
8. docs/research/alt-tab-workspace-isolation.md — upstream #1777 history and
   the protocol map
9. docs/research/switcher-behavior-adaptation.md — verdict table (§1), GNOME
   q/w keys (§2.1), wheel deferral (§4)
10. docs/research/switcher-ux-and-icons.md — verdict table (§1, verdicts 8 and
    12), recommendations (§5), gaps (§6)
11. PRODUCT.md — commitments, constraints, "Evidence on Hand" absences
12. src/wayland.rs:112–144 — required-protocol checks and bound versions
13. src/icons.rs:193, :222 — SVG parse and PNG allocation sites
14. src/daemon.rs:425–431 — IPC accept loop
15. src/ui.rs:600, :1105 — truncation and width cap (no row cap)
16. Commits: c92c670 (v0.1.0 packaging), fe0c775 (tag target), 0007bf5,
    78809c5, 53c8e91, fd43256, 1cafd4e, 7a82b5e, f5dadbe, f76b628, a79886c,
    79b0948 (git log, 2026-11-03)

**Live checks (2026-11-03):**

17. crates.io API — `cosmic-ext-airlock`: HTTP 404 (not published);
    `cosmic-client-toolkit`: max version 0.2.0, published 2026-01-08;
    `cosmic-switch-less`: HTTP 404
18. GitHub API, l1br3-io/cosmic-ext-airlock — release v0.1.0 published
    2026-11-02T03:59:42Z, body citing 12-px text and crates.io install
19. GitHub API, pop-os/cosmic-epoch — issue #1777 open (31 comments, updated
    2026-08-17); issues #3598, #2422, #2424, #3258, #1705 open
20. GitHub API, pop-os/cosmic-applets — PR #1236 merged 2026-04-09
21. GitHub API, pop-os/cosmic-protocols — unstable/ listing and commit
    history (last workspace/toplevel change 2025-09-17; corner-radius
    layer-shell support 2026-06-25; relicensing to MIT 2026-08-14)
22. GitLab API, wayland/wayland-protocols — staging/ tree:
    `ext-foreign-toplevel-list`, `ext-workspace`; no toplevel-management
    protocol present
