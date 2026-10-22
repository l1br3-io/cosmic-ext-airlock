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

# Run the daemon in the foreground for manual testing.
run:
    cargo run -- daemon

# Install the airlock binary into ~/.cargo/bin.
install:
    cargo install --path .
