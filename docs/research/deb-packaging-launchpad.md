# Airlock — deb packaging: Launchpad PPA vs self-hosted apt repo

**Date:** November 7, 2026 · **Method:** primary-source web pass (Launchpad,
Ubuntu archive pages and dev docs, the noble `dh-cargo` package itself, Debian
wiki, upstream READMEs) plus a local measurement of the vendored dependency
tree in this repository. This record is closed; corrections go in a new
record, not edits here.

## 1. The question

Airlock is Rust edition 2024 with `rust-version = "1.93"`, targeting
Pop!_OS 24.04 (Ubuntu noble base) on amd64 and arm64, installable by a
stranger with `apt` — no Rust toolchain on their machine. Can a Launchpad
PPA building from source carry it, or does the channel have to be a
self-hosted apt repo of prebuilt debs?

## 2. The collision: noble's rustc vs the crate's floor

- Noble's base `rustc` is **1.75.0+dfsg0ubuntu1-0ubuntu7.4** (main;
  noble-updates delivers only fixes of 1.75, no major bumps)
  (https://packages.ubuntu.com/noble/rustc, https://packages.ubuntu.com/noble-updates/arm64/rustc).
- Noble does carry *versioned* toolchain packages in universe: 1.74–1.85,
  1.89, and 1.91 — the newest noble rustc in any pocket is
  **1.91.1+dfsg~24.04-0ubuntu0.24.04.3** (noble-updates, uploaded
  2026-04-27) (https://ubuntu.com/developers/docs/reference/availability/rust,
  https://launchpad.net/ubuntu/noble/+source/rustc-1.91).
- `rustc-1.93` exists in the primary archive only for 26.04
  (https://launchpad.net/ubuntu/+source/rustc-1.93).
- Canonical's backport policy: keep the three newest toolchains per series,
  and backports land in 22.04/24.04 "unless something else requires them" —
  Rust 1.90 was built and tested but not uploaded
  (https://ubuntu.com/developers/docs/reference/availability/rust).

**The gap:** the floor (1.93) sits 18 minors above noble's base (1.75) and 2
above its newest (1.91). The in-archive toolchain cannot build this crate.

## 3. Toolchain paths examined

| Path | Evidence | Verdict |
|------|----------|---------|
| In-archive only (1.75 / 1.91) | §2 | **Rejected** — 1.91 < 1.93 |
| Wait for an in-archive backport | Policy quote in §2; 1.90 built but not uploaded | **Rejected** — indefinite, policy says no |
| Chain `ppa:rust-toolchain/staging` as a PPA build dependency | See below | **Viable — the path** |
| Ship a prebuilt toolchain inside the source package | rust-1.93.1 x86_64 dist ≈184 MiB, aarch64 ≈162 MiB (≈346 MiB both, ~4.3% of the 8 GiB PPA quota); but a source package is shared across arches, so per-arch contents or duplicated source packages are needed, and policy-shape says source (https://static.rust-lang.org/dist/rust-1.93.1-x86_64-unknown-linux-gnu.tar.xz, https://static.rust-lang.org/dist/rust-1.93.1-aarch64-unknown-linux-gnu.tar.xz, https://ubuntu.com/docs/launchpad/user/reference/packaging/ppas/ppa) | **Rejected** — fits the quota, fights the format |
| Build rustc from source on the builder | Rust's bootstrap downloads prebuilt stage0 tarballs — a network dependency builders don't have; Debian/Ubuntu's own source packages instead chain the previous in-archive rustc (`rustc 1.75` dsc: `rustc:native >= 1.74`, `llvm-17-dev`, with `<!pkg.rustc.dlstage0>` guards); building takes 10–15 GB (up to ~100 GB) and the matching LLVM (https://raw.githubusercontent.com/rust-lang/rust/master/src/bootstrap/README.md, https://archive.ubuntu.com/ubuntu/pool/main/r/rustc/rustc_1.75.0+dfsg0ubuntu1-0ubuntu7.4.dsc, https://rustc-dev-guide.rust-lang.org/building/how-to-build-and-run.html) | **Rejected** — 1.93 would need a noble 1.92 that doesn't exist |

**The chosen path in detail.** `ppa:rust-toolchain/staging` (Canonical Rust
engineers; "Rust toolchain components that are staged for upload") publishes
versioned rustc for noble well past 1.91: rustc-1.93
`1.93.1+dfsg~24.04-0ubuntu4~24.04.1` (2026-04-10) through rustc-1.97
(2026-09-25) (https://launchpad.net/~rust-toolchain/+archive/ubuntu/staging).
PPA-to-PPA build dependencies are an official Launchpad feature: "A PPA can
depend on other archives, so that packages published in those archives are
available when building packages in your PPA" (API
`addArchiveDependency`, pin −100…2000)
(https://ubuntu.com/docs/launchpad/user/how-to/packaging/create-ppa).

## 4. Launchpad mechanics that shape the build

- **Builders have no general internet**: "Builders do not have direct access
  to the Internet, but rather need to acquire an authentication token to be
  able to access a restricted set of URLs"
  (https://ubuntu.com/docs/launchpad/developer/reference/services/build-farm).
  All crates must ride in the source package.
- **Source packages only**; prebuilt binary uploads are rejected — which
  matches our model, since the binary is built onsite
  (https://ubuntu.com/docs/launchpad/user/reference/packaging/ppas/ppa).
- **The vendored-crates flow is first-class in noble**: `dh-cargo`
  31ubuntu2~24.04.1 ships a debhelper cargo buildsystem that, with
  `CARGO_VENDOR_DIR` set, runs `cargo prepare-debian` and verifies a flat
  one-crate-per-subdir tree "as is generated by `cargo vendor`"; without it,
  builds use the system registry (the tarball's `cargo.pm` lines 90–228,
  https://archive.ubuntu.com/ubuntu/pool/main/d/dh-cargo/dh-cargo_31ubuntu2~24.04.1.tar.xz).
- **Quota:** 8 GiB disk per PPA, larger on request
  (https://ubuntu.com/docs/launchpad/user/reference/packaging/ppas/ppa).
- **Mechanics:** `ppa:<owner>/<name>` naming; PPA Terms of Use on first
  activation; `debuild -S` → `dput ppa:owner/name foo_source.changes`;
  uploads must be signed; versions unique per series; the build farm has
  amd64 and arm64 builders, per-PPA arch selection via `setProcessors`
  (https://ubuntu.com/docs/launchpad/user/how-to/packaging/ppa-package-upload,
  https://launchpad.net/builders).

## 5. The vendored tree, measured locally

Measured on this repository, 2026-11-07: `cargo vendor` produces **128
crates, 180 MB raw, 14 MB xz-compressed**. Against the 8 GiB quota the
compressed payload is negligible; the source package carries it once,
arch-independently (Rust sources). The tree is generated by the deb recipes
and never committed (`.gitignore` carries `/vendor/`).

## 6. The fallback vehicle's real shape

If the toolchain chain or Launchpad builds prove unmaintainable, the
documented fallback is a self-hosted apt repo of prebuilt debs:

- **Repo tool:** `reprepro` — the Debian wiki's recommendation for small
  personal archives (strict checking, no database); `aptly` an alternative;
  `apt-ftparchive` listed as deprecated
  (https://wiki.debian.org/DebianRepository/Setup).
- **Client trust:** keyring in `/etc/apt/keyrings` (binary, dearmored),
  `signed-by` pointing to a file, deb822 `.sources` — "Every sources entry
  SHOULD have the signed-by option set", and it "MUST point to a file"
  (https://wiki.debian.org/DebianRepository/UseThirdParty).
- **Real precedents:** GitHub CLI ships `cli.github.com/packages` with the
  keyrings pattern (https://github.com/cli/cli/blob/trunk/docs/install_linux.md);
  `deckerego/ndiclient` hosts a deb822 apt repo on GitHub Pages for a
  cargo-deb-built Rust project
  (https://github.com/deckerego/ndiclient).
- **Producer:** `cargo-deb` builds `.deb`s from Cargo.toml metadata
  (`[package.metadata.deb]`, including `systemd-units` for daemon packages);
  requires Rust 1.76+; cross-compilation works but the README calls its
  requirements "onerous" and warns against cross-building for older
  releases than the host — arm64 debs would realistically mean a build
  container/VM (https://github.com/kornelski/cargo-deb).

The fallback's real cost is exactly the arch matrix: every architecture is
built and hosted owner-side, instead of Launchpad's farm building both.

## 7. Ecosystem precedents

- Comparable cosmic-ext projects ship via Flatpak (COSMIC Store / Pop's
  cosmic flatpak remote on apt.pop-os.org) and GitHub-Releases debs —
  none of the sampled projects publish a Launchpad PPA
  (https://github.com/cosmic-utils/cosmic-ext-applet-minimon,
  https://github.com/nwxnw/cosmic-ext-connected, https://github.com/franz-net/cosmic-ext-flux).
- A third-party cosmic-ext PPA on noble exists as a counter-example:
  `ppa:wanleungwong/cosmic-ext-ime` (cosmic-ext-ime 0.2.1, noble, README
  targets Pop!_OS 24.04)
  (https://launchpad.net/~wanleungwong/+archive/ubuntu/cosmic-ext-ime).
- System76 does **not** build COSMIC debs on Launchpad: internal CI
  publishes to apt.pop-os.org staging suites that promote to release;
  `cosmic-comp`'s `debian/control` pins only `rustc (>=1.57)` and the dev
  side pins 1.95 via rustup
  (https://github.com/pop-os/pop/blob/master/scripts/apt,
  https://raw.githubusercontent.com/pop-os/cosmic-comp/master/debian/control).
  Their absence from Launchpad is a CI choice, not evidence against it.

## 8. Verdicts

1. **Vehicle: Launchpad PPA, source builds — the default stands.** The
   toolchain path exists (below), the vendored flow is first-class in
   noble's dh-cargo, the payload is trivial (14 MB xz), both target arches
   are on the build farm, and an in-niche cosmic-ext PPA on noble proves
   the vehicle. The fallback repo (§6) stays documented as the designed
   remedy, not the plan.
2. **Toolchain: chain `ppa:rust-toolchain/staging` as a PPA build
   dependency** and Build-Depend on its versioned rustc (1.93) — the exact
   Build-Depends lines are pinned in PP2, where they are testable. This is
   the one external dependency the channel takes on; its risk and remedy:
   the PPA is described as "staged for upload", so a version can retire —
   if the chain ever breaks, the remedy is the §6 fallback vehicle or
   re-pinning the crate floor, decided at a checkpoint, never silently.
3. **Arch matrix: amd64 + arm64 — the default stands.** Both arches are on
   the Launchpad build farm and the vendored tree is arch-independent.
   Residual check before enabling the arm64 builder (PP2/PP3): confirm
   rustc-1.93 arm64 binaries are published in the toolchain PPA.
