# Airlock build recipes. Verification gate: `just test`; quality gates:
# `just check` (clippy -D warnings) and `just fmt-check`.

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

# Install the release binary into ~/.cargo/bin. The installed copy is
# self-contained — the spawn stub re-executes the daemon from its own path,
# so daily use never touches the dev tree.
install:
    cargo install --path .

# --- CP4 VM rehearsal plumbing (delegates to scripts/vm.sh) ---
# Guest-side actions ride the QEMU guest agent: install it in the VM once
# with `sudo apt install qemu-guest-agent`. The keypress rehearsal itself
# stays human — see docs/vm-development.md.

# Rehearsal VM name and guest username. Every VM variable (these two plus
# VM_HOME, VM_BIN, BRIDGE_IP, PORT), the override forms, and the run order:
# `just vm-help`.
vm-name := env_var_or_default("VM_NAME", "PopOs24.04")
vm-user := env_var_or_default("VM_USER", "jpt")

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
