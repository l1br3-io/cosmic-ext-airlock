//! The system dark/light preference: COSMIC's ThemeMode store, read.
//!
//! COSMIC Settings persists its dark/light switch as one bare RON bool
//! per key (`ThemeMode` v1); this module answers the one bool the
//! switcher cares about. Deliberately a hand read — `cosmic-config` is
//! a git-only tree pulling `notify` + `atomicwrites` (a second git
//! repo) + serde/ron/tracing, far too much for one boolean (the TH3
//! decision, ratified 2026-11-04). The read happens once per session
//! open, on the gesture path, so this is two small file reads at most
//! and never fatal: any miss is `None`, and `Palette::resolve` paints
//! dark (today's look).

use std::path::{Path, PathBuf};

/// The system-default ThemeMode `is_dark`, cosmic-config's fallback when
/// no user value exists (`/usr/share/cosmic`, xdg prefix `cosmic`).
const SYSTEM_IS_DARK_PATH: &str = "/usr/share/cosmic/com.system76.CosmicTheme.Mode/v1/is_dark";

/// Whether the COSMIC desktop prefers a dark theme: the user's
/// `ThemeMode` `is_dark` when it exists and parses, else the system
/// default, else `None` (with one diagnostic — the caller paints dark).
pub fn system_prefers_dark() -> Option<bool> {
    let preference = preference_at(
        user_is_dark_path().as_deref(),
        Path::new(SYSTEM_IS_DARK_PATH),
    );
    if preference.is_none() {
        eprintln!("airlock daemon: no readable COSMIC theme preference; auto paints dark");
    }
    preference
}

/// The user's `ThemeMode` `is_dark` under COSMIC's config rule
/// (`$XDG_CONFIG_HOME` absolute else `$HOME/.config` — the same rule the
/// installer and `settings` use). `None` when the environment names
/// neither; the system default still serves.
fn user_is_dark_path() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".config")));
    config.map(|dir| {
        dir.join("cosmic")
            .join("com.system76.CosmicTheme.Mode")
            .join("v1")
            .join("is_dark")
    })
}

/// The preference from `user` when it exists and parses, else `system`,
/// else `None`.
fn preference_at(user: Option<&Path>, system: &Path) -> Option<bool> {
    user.and_then(read_preference)
        .or_else(|| read_preference(system))
}

/// The parseable preference in one file, `None` for any miss (absent,
/// unreadable, garbage): COSMIC's store is one bare RON bool per file,
/// and the fallback ladder lives in [`preference_at`].
fn read_preference(path: &Path) -> Option<bool> {
    parse_preference(&std::fs::read(path).ok()?)
}

/// A bare RON bool: `true` or `false`, whitespace-padded as files are.
/// Anything else — garbage, empty, non-UTF-8 — is `None`, never a panic.
fn parse_preference(bytes: &[u8]) -> Option<bool> {
    match std::str::from_utf8(bytes).ok()?.trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- parse -----------------------------------------------------------

    #[test]
    fn parse_preference_accepts_trimmed_true_and_false() {
        assert_eq!(parse_preference(b"true"), Some(true));
        assert_eq!(parse_preference(b"false"), Some(false));
        // Files carry padding: a trailing newline, surrounding spaces.
        assert_eq!(parse_preference(b"true\n"), Some(true));
        assert_eq!(parse_preference(b"  false\r\n"), Some(false));
        assert_eq!(parse_preference(b"\n\ttrue \n"), Some(true));
    }

    #[test]
    fn parse_preference_rejects_garbage_and_empty() {
        assert_eq!(parse_preference(b""), None);
        assert_eq!(parse_preference(b"yes"), None);
        // RON bools are lowercase; case variants are not answers.
        assert_eq!(parse_preference(b"TRUE"), None);
        assert_eq!(parse_preference(b"true false"), None);
        assert_eq!(parse_preference(b"1"), None);
        // Non-UTF-8 bytes, like every other miss, are None — no panic.
        assert_eq!(parse_preference(&[0xFF, 0xFE]), None);
    }

    // --- path priority ---------------------------------------------------

    /// Unique disposable directory (no tempfile dependency).
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "airlock-theme-{tag}-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            ));
            std::fs::create_dir_all(&path).expect("create temp dir");
            TempDir(path)
        }

        /// Writes `body` at `<dir>/is_dark` and returns that path.
        fn is_dark(&self, body: &[u8]) -> PathBuf {
            let path = self.0.join("is_dark");
            std::fs::write(&path, body).expect("write fixture");
            path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_user_preference_wins_over_the_system_default() {
        let user_dir = TempDir::new("user-wins");
        let system_dir = TempDir::new("user-wins-system");
        let user = user_dir.is_dark(b"false\n");
        let system = system_dir.is_dark(b"true\n");
        assert_eq!(
            preference_at(Some(&user), &system),
            Some(false),
            "the user's answer is the answer"
        );
    }

    #[test]
    fn an_absent_user_file_falls_back_to_the_system_default() {
        let user_dir = TempDir::new("user-absent");
        let system_dir = TempDir::new("user-absent-system");
        let user = user_dir.0.join("is_dark"); // never written
        let system = system_dir.is_dark(b"true\n");
        assert_eq!(preference_at(Some(&user), &system), Some(true));
    }

    #[test]
    fn a_garbage_user_file_does_not_poison_the_system_default() {
        let user_dir = TempDir::new("user-garbage");
        let system_dir = TempDir::new("user-garbage-system");
        let user = user_dir.is_dark(b"banana");
        let system = system_dir.is_dark(b"false");
        assert_eq!(preference_at(Some(&user), &system), Some(false));
    }

    #[test]
    fn a_total_miss_is_none() {
        let user_dir = TempDir::new("total-miss");
        let system_dir = TempDir::new("total-miss-system");
        let user = user_dir.0.join("is_dark"); // never written
        let system = system_dir.0.join("is_dark"); // never written
        assert_eq!(preference_at(Some(&user), &system), None);
        assert_eq!(preference_at(None, &system), None, "no user path at all");
    }
}
