#!/bin/sh
# Airlock's one-command install: download the prebuilt release binary,
# verify its sha256, install it into a directory on PATH, and run
# `airlock install` (the Alt+Tab takeover). User-level only: no sudo, no
# shell rc edits; the writes are the binary and whatever `airlock install`
# reports.
#
# Inspect before running: fetch this file, read it, then run it:
#   curl -fsSL .../install.sh -o install.sh && less install.sh && sh install.sh
#
# Environment:
#   AIRLOCK_VERSION=0.4.4      release to install (default: latest)
#   AIRLOCK_INSTALL_DIR=DIR    binary destination (default: a PATH dir)
#   AIRLOCK_NO_INTEGRATION=1   binary only; run `airlock install` yourself
#
# Uninstall: `airlock install --uninstall`, then remove the binary.
# The rationale and the surveyed precedent: docs/research/one-command-install.md.
set -eu

# --- Bake points ------------------------------------------------------------
# `just release-artifacts` rewrites these three lines in the copy uploaded
# to the release, so a pinned release script carries its own checksum.
# Keep each assignment on one line.
airlock_version_default="latest"
airlock_baked_version="0.4.4"
airlock_baked_sha256="4d1027fb800df2919828de32c88fe08e6381fe989a41606d1b9fbc9444affce5"

repo="l1br3-io/cosmic-ext-airlock"

die() { printf 'airlock: error: %s\n' "$*" >&2; exit 1; }
note() { printf 'airlock: %s\n' "$*"; }

have() { command -v "$1" >/dev/null 2>&1; }

fetch() { # url -> stdout
    if have curl; then
        curl -fsSL --proto '=https' --tlsv1.2 "$1"
    elif have wget; then
        wget -qO- "$1"
    else
        die "curl or wget is required"
    fi
}

download() { # url destination
    if have curl; then
        curl -fsSL --proto '=https' --tlsv1.2 -o "$2" "$1"
    elif have wget; then
        wget -qO "$2" "$1"
    else
        die "curl or wget is required"
    fi
}

latest_version() {
    api="https://api.github.com/repos/$repo/releases/latest"
    payload="$(fetch "$api")" || die "cannot query the latest release"
    tag="$(printf '%s\n' "$payload" |
        sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n 1)"
    [ -n "$tag" ] || die "cannot read the latest release tag"
    printf '%s' "${tag#v}"
}

resolve_target() {
    os="$(uname -s)"
    machine="$(uname -m)"
    [ "$os" = "Linux" ] ||
        die "Airlock runs on Linux only (found $os); build from source for another system"
    case "$machine" in
        x86_64 | amd64) printf '%s' "x86_64-unknown-linux-gnu" ;;
        *) die "no prebuilt binary for $machine; build from source: cargo install cosmic-ext-airlock" ;;
    esac
}

sha256_of() {
    if have sha256sum; then
        sha256sum "$1" | cut -d' ' -f1
    elif have shasum; then
        shasum -a 256 "$1" | cut -d' ' -f1
    else
        die "sha256sum (or shasum) is required to verify the download"
    fi
}

on_path() {
    case ":$PATH:" in
        *":$1:"*) return 0 ;;
        *) return 1 ;;
    esac
}

choose_dir() {
    if [ -n "${AIRLOCK_INSTALL_DIR:-}" ]; then
        printf '%s' "$AIRLOCK_INSTALL_DIR"
        return 0
    fi
    if on_path "$HOME/.local/bin"; then
        printf '%s' "$HOME/.local/bin"
        return 0
    fi
    if on_path "$HOME/.cargo/bin"; then
        printf '%s' "$HOME/.cargo/bin"
        return 0
    fi
    printf '%s' "$HOME/.local/bin"
}

# --- Main -------------------------------------------------------------------

[ "$(id -u)" -eq 0 ] &&
    die "don't run this as root; Airlock installs into the invoking user's home"

have tar || die "tar is required"

version="${AIRLOCK_VERSION:-$airlock_version_default}"
if [ "$version" = "latest" ]; then
    version="$(latest_version)"
fi
version="${version#v}"
[ -n "$version" ] || die "empty version"

target="$(resolve_target)"
base="https://github.com/$repo/releases/download/v$version"
archive="airlock-$target-v$version.tgz"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

note "downloading $archive"
download "$base/$archive" "$tmp/$archive" ||
    die "cannot download $base/$archive (is v$version released?)"

if [ "$version" = "$airlock_baked_version" ] && [ -n "$airlock_baked_sha256" ]; then
    expected="$airlock_baked_sha256"
else
    download "$base/$archive.sha256" "$tmp/$archive.sha256" ||
        die "no published checksum for v$version; install with: cargo binstall cosmic-ext-airlock"
    expected="$(cut -d' ' -f1 "$tmp/$archive.sha256")"
fi
case "$expected" in
    *[!0-9a-f]*) die "malformed checksum for $archive" ;;
esac
[ "${#expected}" -eq 64 ] || die "malformed checksum for $archive"
actual="$(sha256_of "$tmp/$archive")"
[ "$actual" = "$expected" ] ||
    die "checksum mismatch for $archive: expected $expected, got $actual"

tar -xzf "$tmp/$archive" -C "$tmp"
[ -f "$tmp/airlock" ] || die "the archive does not contain the airlock binary"

dir="$(choose_dir)"
dir="${dir%/}"
mkdir -p "$dir" 2>/dev/null || die "cannot create $dir"
[ -w "$dir" ] ||
    die "$dir is not writable; Airlock never uses sudo, so set AIRLOCK_INSTALL_DIR to a writable directory"
staged="$dir/.airlock.new.$$"
cp "$tmp/airlock" "$staged"
chmod 755 "$staged"
mv -f "$staged" "$dir/airlock"
note "installed airlock v$version to $dir/airlock"

if ! on_path "$dir"; then
    note "warning: $dir is not on PATH; add it to your shell startup:"
    note "  export PATH=\"$dir:\$PATH\""
    if [ "${AIRLOCK_NO_INTEGRATION:-0}" != "1" ]; then
        note "the takeover's keybinds call 'airlock' from PATH; add the line above,"
        note "open a new terminal, then finish with: airlock install"
        exit 0
    fi
fi

if [ "${AIRLOCK_NO_INTEGRATION:-0}" = "1" ]; then
    note "binary only (AIRLOCK_NO_INTEGRATION=1); finish with: airlock install"
    exit 0
fi

note "running: $dir/airlock install"
if "$dir/airlock" install; then
    note "done; press Alt+Tab inside COSMIC"
    note "undo: airlock install --uninstall && rm $dir/airlock"
    exit 0
fi
note "the binary is installed, but the takeover did not complete (see the report above)"
note "finish it inside a COSMIC session with: airlock install"
exit 1
