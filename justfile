# Airlock build recipes. Verification gate: `just test`; quality gates:
# `just check` (clippy -D warnings) and `just fmt-check`.
#
# Recipes are silent on success and announce only on failure (AGENTS.md):
# `set quiet` stops just from echoing each recipe line, and agent-facing
# gates route through `quietly` below so a green run costs zero output
# tokens while a red one replays the full log with its exit code.
set quiet

# Run a command via the shell, printing its combined output only when it
# fails.
[private]
quietly command:
	#!/usr/bin/env bash
	set -euo pipefail
	out="$(mktemp)"
	set +e
	bash -c '{{command}}' >"$out" 2>&1
	status=$?
	set -e
	if [ "$status" -ne 0 ]; then
		cat "$out"
		rm -f "$out"
		exit "$status"
	fi
	rm -f "$out"

# A rustup bootstrapped by `deps` must be usable by `install` (and any
# cargo recipe) in the same invocation — each recipe runs its own
# shell, so CARGO_HOME/bin is prepended for every recipe. A nonexistent
# PATH entry is harmless.
export PATH := env('CARGO_HOME', home_directory() / '.cargo') / 'bin' + ':' + env('PATH', '')

default:
	@just --list

# Development build.
build: (quietly "cargo build")

# Release build (COSMIC justfile convention).
release: (quietly "cargo build --release")

# Quality gate: clippy with warnings denied.
check: (quietly "cargo clippy --all-targets --all-features --locked -- -D warnings")

# Write formatting.
fmt:
    cargo fmt --all

# Quality gate: formatting check.
fmt-check:
    cargo fmt --all -- --check

# Verification gate: full test suite.
test: (quietly "cargo test --all-targets --all-features")

# Dev loop: build and run the daemon from the dev tree (foreground). Only
# one daemon can run at a time — if the installed daemon is serving, stop
# it first with `just stop` or this exits "already running".
run:
    cargo run -- daemon

# Stop the running daemon, whichever build it came from. The next key
# press auto-spawns the daemon from the installed binary: use this to
# hand control back after a dev session, or to activate a fresh
# `just install` (a running daemon keeps the old code until stopped).
stop:
	-@pkill -x airlock

# Regenerate docs/img/overlay-card.png (the README hero) and its
# byte-identical site/assets/overlay-card.png copy: the overlay-preview
# contact sheet's grouped family's windowed 12-row tree (selected group
# expanded to its windows, `…` row below) cropped from the dark-backdrop
# column at --scale 2 through the real font pipeline (--system-font pins the
# committed Open Sans fixture; the flagless run draws the font8x8
# floor). Card 316x584 logical px = 632x1168 device; offsets
# x = gutter = 64, y = 64 + singles(2128) + query(952) + close(880)
# + workspace-label(1656) + long-list(7056) + grouped(2128) = 14864
# — the grouped family's windowed 12-row tree — re-derive from
# examples/overlay-preview.rs if
# the scenario list changes. The card's rounded-corner cutouts reveal
# the dark backdrop (46,46,50); -transparent turns them back into the
# alpha the sheet flattened, so the hero sits clean on any page
# background. Themed icons resolve from the host's icon theme; a
# theme-less host draws the synthetic tiles. Needs ImageMagick
# (convert).
readme-image:
	{{just_executable()}} quietly 'cargo run --example overlay-preview -- /tmp/airlock-preview-sheet.png --system-font res/test-fonts'
	convert /tmp/airlock-preview-sheet.png -crop 632x1168+64+14864 +repage -transparent 'rgb(46,46,50)' docs/img/overlay-card.png
	cp docs/img/overlay-card.png site/assets/overlay-card.png


# Prerequisite gate, shared by `install` (and runnable alone): checks
# the C toolchain, pkg-config, the pkg-config modules behind the system
# libraries the daemon links at build time (wayland-client and xkbcommon
# — cosmic-client-toolkit's Wayland stack), and cargo, installing only
# what is missing through the distro's package manager
# (apt/dnf/pacman/zypper). Airlock targets the COSMIC desktop, so
# anything that is not Linux fails with the reason instead of a package
# guessing game. Rust comes via rustup (stable, minimal profile) when
# cargo is absent — never as root, so `sudo just install` never writes a
# root-owned toolchain into /root. With everything present the gate is a
# silent no-op that touches neither sudo nor the network.
deps:
	#!/usr/bin/env bash
	# pipefail: the rustup bootstrap is a `curl | sh` pipeline — without
	# it, a dead curl hands sh an EOF, sh exits 0, and deps reports
	# success with cargo never installed.
	set -euo pipefail

	missing=""
	if command -v pkg-config >/dev/null 2>&1; then
		for module in wayland-client xkbcommon; do
			pkg-config --exists "$module" 2>/dev/null || missing="$missing $module"
		done
	else
		missing=" pkg-config"
	fi
	command -v cc >/dev/null 2>&1 || missing="$missing cc"

	if [ -n "$missing" ]; then
		case "$(uname -s)" in
			Linux)
				echo "deps: missing:$missing — installing via the system package manager"
				if [ "$(id -u)" -eq 0 ]; then
					sudo=""
				elif command -v sudo >/dev/null 2>&1; then
					sudo="sudo"
				else
					echo "deps: not root and sudo not found — install the above by hand and re-run" >&2
					exit 1
				fi
				# $sudo is deliberately unquoted: it word-splits away
				# when empty (already root).
				if command -v apt-get >/dev/null 2>&1; then
					$sudo apt-get update -qq
					$sudo apt-get install -y build-essential pkg-config \
						libwayland-dev libxkbcommon-dev
				elif command -v dnf >/dev/null 2>&1; then
					$sudo dnf install -y gcc gcc-c++ make pkgconf-pkg-config \
						wayland-devel libxkbcommon-devel
				elif command -v pacman >/dev/null 2>&1; then
					$sudo pacman -S --needed --noconfirm base-devel pkgconf \
						wayland libxkbcommon
				elif command -v zypper >/dev/null 2>&1; then
					$sudo zypper --non-interactive install gcc gcc-c++ make pkg-config \
						wayland-devel libxkbcommon-devel
				else
					echo "deps: no supported package manager (apt-get, dnf, pacman, zypper)" >&2
					echo "deps: install the equivalents of:$missing and re-run" >&2
					exit 1
				fi
				;;
			Darwin)
				echo "deps: Airlock targets the COSMIC desktop — build on Linux; macOS has neither the compositor nor its libraries" >&2
				exit 1
				;;
			*)
				echo "deps: Airlock targets the COSMIC desktop (Linux); $(uname -s) is not supported" >&2
				exit 1
				;;
		esac
	fi

	# Rust side (never as root — see the recipe comment).
	if [ "$(id -u)" -ne 0 ] && ! command -v cargo >/dev/null 2>&1; then
		echo "deps: cargo not found — installing Rust (rustup, stable, minimal profile)"
		if command -v rustup >/dev/null 2>&1; then
			rustup toolchain install stable --profile minimal
			rustup default stable
		else
			curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
				| sh -s -- -y --profile minimal --default-toolchain stable
		fi
	fi

# Install the release binary into ~/.cargo/bin, checking the build
# prerequisites first (see deps — a no-op once they exist). The
# installed copy is self-contained — the spawn stub re-executes the
# daemon from its own path, so daily use never touches the dev tree.
install: deps
	{{just_executable()}} quietly 'cargo install --path .'

# --- Release runbook (release-pipeline module; owner-run) ---
# release-check is the gate: green means the tree is releasable. The
# other four are the public steps, invoked by the owner in order after
# CP-RP1 and the CP-PK walk — never by the agent, and nothing here
# pushes or publishes except those owner-run recipes (plus the publish
# dry-run inside the gate).

# Release gate: quality gates + one-version coherence + package
# preview. Coherence: Cargo.toml version == the CHANGELOG's top
# section == the metainfo <release>, and the two dates agree; a
# v<version> tag may be absent, at HEAD, or awaiting release-tag's
# documented re-cut force-move — all pass, because the gate runs
# before the move and release-tag refuses a version the tree does not
# carry. The metainfo must validate with nothing beyond the one
# documented hyphen warning (README Packaging: the app ID matches the
# GitHub org). Ends with the crate file list and a full publish
# rehearsal (--locked, --dry-run). Green = releasable.
[group('release')]
release-check:
	#!/usr/bin/env bash
	set -euo pipefail
	justbin="{{just_executable()}}"

	fail() { echo "release-check: $*" >&2; exit 1; }

	# One version story across Cargo.toml, CHANGELOG, and metainfo.
	cargo_ver="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1)"
	[ -n "$cargo_ver" ] || fail "no version line found in Cargo.toml"

	cl_header="$(grep -m1 '^## \[' CHANGELOG.md)"
	cl_ver="$(printf '%s\n' "$cl_header" | sed -n 's/^## \[\([^]]*\)\].*/\1/p')"
	cl_date="$(printf '%s\n' "$cl_header" | sed -n 's/^## \[[^]]*\] — *\([^ ]*\).*/\1/p')"
	[ -n "$cl_ver" ] && [ -n "$cl_date" ] \
		|| fail "CHANGELOG top section not parseable: '$cl_header'"

	mi_header="$(grep -m1 '<release ' res/io.github.l1br3-io.Airlock.metainfo.xml)"
	mi_ver="$(printf '%s\n' "$mi_header" | sed -n 's/.*version="\([^"]*\)".*/\1/p')"
	mi_date="$(printf '%s\n' "$mi_header" | sed -n 's/.*date="\([^"]*\)".*/\1/p')"
	[ -n "$mi_ver" ] && [ -n "$mi_date" ] \
		|| fail "metainfo <release> line not parseable: '$mi_header'"

	[ "$cl_ver" = "$cargo_ver" ] \
		|| fail "Cargo.toml version $cargo_ver ≠ CHANGELOG top version $cl_ver"
	[ "$mi_ver" = "$cargo_ver" ] \
		|| fail "Cargo.toml version $cargo_ver ≠ metainfo release version $mi_ver"
	[ "$cl_date" = "$mi_date" ] \
		|| fail "CHANGELOG date $cl_date ≠ metainfo date $mi_date"

	# Metainfo validation: appstreamcli exits 3 on any warning, so judge
	# the findings, not the code — everything but the documented hyphen
	# cid warning is a failure.
	command -v appstreamcli >/dev/null 2>&1 \
		|| fail "appstreamcli not found (Debian package: appstream)"
	as_out="$(appstreamcli validate res/io.github.l1br3-io.Airlock.metainfo.xml 2>&1 || true)"
	unexpected="$(printf '%s\n' "$as_out" | grep '^[EWP]: ' | grep -v 'cid-rdns-contains-hyphen' || true)"
	[ -z "$unexpected" ] || fail "appstreamcli: $unexpected"

	# Gates, then the crate preview and a full publish rehearsal.
	"$justbin" test
	"$justbin" check
	"$justbin" fmt-check
	cargo package --list --locked >/dev/null
	out="$(mktemp)"
	if ! cargo publish --dry-run --locked >"$out" 2>&1; then
		tail -n 40 "$out" >&2
		rm -f "$out"
		fail "cargo publish --dry-run failed"
	fi
	rm -f "$out"

# Extract one version's CHANGELOG section (sans its header) to
# target/release-notes-<ver>.md — the text release-gh attaches to the
# GitHub Release verbatim. CP-RP1 approves this text; the extraction
# is mechanical so the approved words are the published words.
[group('release')]
release-notes ver:
	#!/usr/bin/env bash
	set -euo pipefail
	notes="target/release-notes-{{ver}}.md"
	mkdir -p target
	awk -v section='## [{{ver}}]' '
		index($0, section) == 1 { in_section = 1; next }
		in_section && /^## \[/ { exit }
		in_section { print }
	' CHANGELOG.md >"$notes"
	if [ ! -s "$notes" ]; then
		rm -f "$notes"
		echo "release-notes: no [{{ver}}] section in CHANGELOG.md" >&2
		exit 1
	fi

# Annotated tag v<ver> at HEAD, then push. Re-cut mode: an existing
# tag is force-moved (OQ3 — accepted; the version was never on
# crates.io, and the GitHub Release follows the tag ref). Owner-run,
# after release-check: the tag and the tree must carry one version.
[group('release')]
release-tag ver:
	#!/usr/bin/env bash
	set -euo pipefail
	cargo_ver="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1)"
	if [ "{{ver}}" != "$cargo_ver" ]; then
		echo "release-tag: Cargo.toml carries $cargo_ver, asked for {{ver}}" >&2
		exit 1
	fi
	if git rev-parse -q --verify "v{{ver}}^{commit}" >/dev/null; then
		echo "release-tag: v{{ver}} exists at $(git rev-parse --short "v{{ver}}^{commit}") — force-moving to HEAD (re-cut)" >&2
	fi
	git tag -fa "v{{ver}}" -m "Release {{ver}}"
	git push origin "v{{ver}}" --force

# Point the GitHub Release for v<ver> at the extracted notes. Re-cut
# mode edits the existing release object in place so its URL survives;
# after release-tag's force-push the release follows the moved tag.
# Requires release-notes <ver> to have run (the notes file is the
# CP-RP1-approved text).
[group('release')]
release-gh ver:
	#!/usr/bin/env bash
	set -euo pipefail
	notes="target/release-notes-{{ver}}.md"
	if [ ! -s "$notes" ]; then
		echo "release-gh: $notes missing or empty — run: just release-notes {{ver}}" >&2
		exit 1
	fi
	if gh release view "v{{ver}}" >/dev/null 2>&1; then
		gh release edit "v{{ver}}" --notes-file "$notes" --title "Airlock {{ver}}" --latest
	else
		gh release create "v{{ver}}" --notes-file "$notes" --title "Airlock {{ver}}" --latest
	fi

# Build the x86_64-unknown-linux-gnu release binary and upload its
# .tgz to the v<ver> GitHub Release — the archive cargo-binstall
# downloads ([package.metadata.binstall] in Cargo.toml names this
# exact file; the two change together). The triple is explicit so a
# non-x86_64 build host fails loudly instead of mislabeling the
# artifact. Owner-run, after release-gh creates the release (upload
# needs it to exist); --clobber keeps re-cut mode working by
# replacing same-named assets.
[group('release')]
release-artifacts ver:
	#!/usr/bin/env bash
	set -euo pipefail
	archive="airlock-x86_64-unknown-linux-gnu-v{{ver}}.tgz"
	{{just_executable()}} quietly 'cargo build --release --locked --target x86_64-unknown-linux-gnu'
	tar -C target/x86_64-unknown-linux-gnu/release -czf "target/$archive" airlock
	gh release upload "v{{ver}}" "target/$archive" --clobber

# Publish the crate to crates.io, built exactly as the committed
# Cargo.lock pins it. Owner-run only — the invocation is the consent,
# tokens never live in this repo, and crates.io versions are
# immutable: this is the one unretractable step of the runbook.
[group('release')]
release-publish: (quietly "cargo publish --locked")

# --- CP4 VM rehearsal plumbing (delegates to scripts/vm.sh) ---
# Guest-side actions ride the QEMU guest agent: install it in the VM once
# with `sudo apt install qemu-guest-agent`. The keypress rehearsal itself
# stays human — see docs/vm-development.md.

# Rehearsal VM name and guest username. Every VM variable (these two plus
# VM_HOME, VM_BIN, BRIDGE_IP, PORT), the override forms, and the run order:
# `just vm-help`.
vm-name := env_var_or_default("VM_NAME", "PopOs24.04")
vm-user := env_var_or_default("VM_USER", "jpt-virt")

# VM workflow reference: variables, override forms, workflow order.
[group('vm')]
vm-help:
	scripts/vm.sh help

# Guest IP on the VM's NAT bridge.
[group('vm')]
vm-ip:
	VM_NAME={{vm-name}} scripts/vm.sh ip

# Build the release binary, transfer it into the VM over the NAT bridge,
# verify its sha256 on arrival, and stage it at the guest's ~/.local/bin.
[group('vm')]
vm-push:
	VM_NAME={{vm-name}} VM_USER={{vm-user}} scripts/vm.sh push

# Run `airlock install` inside the VM (as the guest desktop user).
[group('vm')]
vm-install:
	VM_NAME={{vm-name}} VM_USER={{vm-user}} scripts/vm.sh install

# Run `airlock install --uninstall` inside the VM.
[group('vm')]
vm-uninstall:
	VM_NAME={{vm-name}} VM_USER={{vm-user}} scripts/vm.sh uninstall

# Take a named libvirt snapshot of the running VM (default "checkpoint").
[group('vm')]
vm-snapshot name="checkpoint":
	VM_NAME={{vm-name}} scripts/vm.sh snapshot {{name}}

# Revert the VM to a named snapshot; it comes back running.
[group('vm')]
vm-restore name:
	VM_NAME={{vm-name}} scripts/vm.sh restore {{name}}

# VM state, guest IP, staged binary path, and snapshots at a glance.
[group('vm')]
vm-status:
	VM_NAME={{vm-name}} VM_USER={{vm-user}} scripts/vm.sh status

# --- Site (GitHub Pages; site module) ---

# Link/asset integrity gate for site/: every local href/src resolves
# inside site/, anchors exist on their target page, imgs carry
# alt/width/height, and assets stay self-contained. The commit gate for
# anything touching site/ — silent on success, loud on breakage.
site-check: (quietly "python3 scripts/check-site-links.py site")

# Serve site/ at http://localhost:8080 for local preview (Ctrl+C stops).
site-preview:
    python3 -m http.server -d site 8080
