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
# self-contained — the spawn stub re-executes the daemon from its own
# path, so daily use never touches the dev tree.
install:
    cargo install --path .
