# Airlock build recipes. Verification gate: `just test`; quality gates:
# `just check` (clippy -D warnings) and `just fmt-check`.

# A rustup bootstrapped by `deps` must be usable by `install` (and any
# cargo recipe) in the same invocation — each recipe runs its own
# shell, so CARGO_HOME/bin is prepended for every recipe. A nonexistent
# PATH entry is harmless.
export PATH := env('CARGO_HOME', home_directory() / '.cargo') / 'bin' + ':' + env('PATH', '')

default:
	@just --list

# Development build.
build:
    cargo build

# Release build (COSMIC justfile convention).
release:
    cargo build --release

# Quality gate: clippy with warnings denied.
check:
    cargo clippy --all-targets --all-features --locked -- -D warnings

# Write formatting.
fmt:
    cargo fmt --all

# Quality gate: formatting check.
fmt-check:
    cargo fmt --all -- --check

# Verification gate: full test suite.
test:
    cargo test --all-targets --all-features

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
	#!/usr/bin/env sh
	set -eu

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
	cargo install --path .

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
