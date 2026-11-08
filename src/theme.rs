//! COSMIC appearance stores, read — the shared "follow the desktop"
//! seam.
//!
//! COSMIC Settings persists its appearance values as one RON file per
//! key under a user and a system store root; this module hand-reads
//! the ones the switcher follows. Deliberately a hand read —
//! `cosmic-config` is a git-only tree pulling `notify` +
//! `atomicwrites` (a second git repo) + serde/ron/tracing, far too
//! much for a handful of booleans and colors (the TH3 decision,
//! ratified 2026-11-04). Reads happen once per session open, on the
//! gesture path, so this is a few small file reads at most and never
//! fatal: any miss is `None`, and the caller paints its documented
//! default (`Palette::resolve` dark, the accent's caller the neutral
//! lift).
//!
//! Two answers live here today: the ThemeMode dark/light bool (one
//! bare RON bool per key, v1) and the per-polarity theme accent (v2
//! hex strings with a v1 f32 fallback, read through the version ladder
//! `cosmic-config` itself chains). The ladder is the shared seam:
//! every future follow reads through it too.

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

/// The user's `ThemeMode` `is_dark` under COSMIC's config rule — the
/// user store root plus this key's path. `None` when the environment
/// names neither root; the system default still serves.
fn user_is_dark_path() -> Option<PathBuf> {
    user_cosmic_root().map(|root| {
        root.join("com.system76.CosmicTheme.Mode")
            .join("v1")
            .join("is_dark")
    })
}

/// The user's COSMIC store root: `$XDG_CONFIG_HOME` (absolute) else
/// `$HOME/.config`, plus `cosmic` — cosmic-config's `Config::new_inner`
/// rule, the same one the installer and `settings` use. `None` when
/// the environment names neither.
fn user_cosmic_root() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".config")));
    config.map(|dir| dir.join("cosmic"))
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

// --- The accent follow -----------------------------------------------------
//
// The selection band's follow target: the COSMIC accent's `base` color,
// per polarity, read through the same version ladder cosmic-config
// chains. Everything here is a read — Airlock observes the stores
// cosmic-settings owns; it never writes them and never creates the
// directories cosmic-config's `Config::new` does as a side effect.

/// The system-wide COSMIC store root (xdg data prefix `cosmic`).
#[allow(dead_code)] // wired by the accent follow's daemon read (PA4)
const SYSTEM_COSMIC_ROOT: &str = "/usr/share/cosmic";

/// The per-polarity theme store ids (`cosmic-theme`'s `DARK_THEME_ID` /
/// `LIGHT_THEME_ID`) — accents are real per-polarity values, not one
/// color the theme recolors.
#[cfg_attr(not(test), allow(dead_code))]
const THEME_DARK_ID: &str = "com.system76.CosmicTheme.Dark";
#[cfg_attr(not(test), allow(dead_code))]
const THEME_LIGHT_ID: &str = "com.system76.CosmicTheme.Light";

/// The accent key and its store version: `Theme` is a v2 store, with
/// the v1 fallback below it — where this machine's live accent
/// actually sits (the store record's §2 rule).
#[cfg_attr(not(test), allow(dead_code))]
const ACCENT_KEY: &str = "accent";
#[cfg_attr(not(test), allow(dead_code))]
const ACCENT_VERSION: u32 = 2;

/// The COSMIC accent's `base` color for one polarity (`true` = dark),
/// as straight `0xRRGGBB` — the selection band's follow target, the
/// desktop's personality in the one place a glance tracks it. Read
/// through the full version ladder over the per-polarity store, both
/// serializations decoded, alpha discarded (followed colors are forced
/// opaque). `None` on any miss — never a broken switch — with one
/// aggregated diagnostic naming every rung that missed; the caller
/// falls back to the neutral lift.
#[allow(dead_code)] // the daemon calls it from the accent follow (PA4)
pub fn accent_base(dark: bool) -> Option<u32> {
    let (value, missed) = accent_base_at(
        user_cosmic_root().as_deref(),
        Path::new(SYSTEM_COSMIC_ROOT),
        dark,
    );
    if value.is_none() {
        eprintln!(
            "airlock daemon: no readable COSMIC accent ({}); the selection band uses the neutral lift",
            missed.join(", ")
        );
    }
    value
}

/// The accent answer over explicit store roots — the testable core of
/// [`accent_base`]: `(value, missed)`. `missed` is empty on a hit; on a
/// total miss it labels every rung tried, `"{origin} {id}/v{v}/{key}"`,
/// for the caller's one aggregated diagnostic line.
#[cfg_attr(not(test), allow(dead_code))]
fn accent_base_at(
    user_root: Option<&Path>,
    system_root: &Path,
    dark: bool,
) -> (Option<u32>, Vec<String>) {
    ladder_at(
        user_root,
        system_root,
        if dark { THEME_DARK_ID } else { THEME_LIGHT_ID },
        ACCENT_VERSION,
        ACCENT_KEY,
        parse_accent_base,
    )
}

/// Reads one key through the version ladder cosmic-config itself chains
/// (store record §2): user v`version` → user v`version − 1` → system
/// v`version` — a v2 config never consults system v1, and the middle
/// rung is load-bearing (a v1-era user tweak must outrank the system
/// default). Each rung is one read-only `read_to_string`; nothing is
/// ever created. A rung whose text does not parse is a miss and the
/// ladder continues; the answer is the first value, or `None` with
/// every rung labeled — never a panic.
#[cfg_attr(not(test), allow(dead_code))]
fn ladder_at<T>(
    user_root: Option<&Path>,
    system_root: &Path,
    id: &str,
    version: u32,
    key: &str,
    parse: impl Fn(&str) -> Option<T>,
) -> (Option<T>, Vec<String>) {
    let rung = |root: &Path, v: u32| root.join(id).join(format!("v{v}")).join(key);
    let label = |origin: &str, v: u32| format!("{origin} {id}/v{v}/{key}");
    let mut rungs: Vec<(String, PathBuf)> = Vec::new();
    if let Some(root) = user_root {
        rungs.push((label("user", version), rung(root, version)));
        if version > 1 {
            rungs.push((label("user", version - 1), rung(root, version - 1)));
        }
    }
    rungs.push((label("system", version), rung(system_root, version)));
    let mut missed = Vec::new();
    for (name, path) in rungs {
        if let Some(value) = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| parse(&text))
        {
            return (Some(value), Vec::new());
        }
        missed.push(name);
    }
    (None, missed)
}

/// The accent `base` from one `Component` file's text, whichever
/// serialization it carries: v2 quotes a `#RRGGBBAA` hex string, v1
/// writes a named f32 tuple. Alpha is parsed then discarded in both —
/// followed colors are forced opaque, the store's translucency never
/// reaches the card. Any miss (no `base` field, wrong shape, non-hex,
/// non-finite) is `None`, never a panic.
#[cfg_attr(not(test), allow(dead_code))]
fn parse_accent_base(text: &str) -> Option<u32> {
    let value = base_field(text)?;
    if let Some(hex) = value
        .strip_prefix('"')
        .and_then(|inner| inner.strip_suffix('"'))
    {
        hex_accent(hex)
    } else {
        f32_accent(value)
    }
}

/// The `base` field's value text: the remainder of its line after
/// `base:`, surrounding space and a trailing comma removed. The one
/// field both serializations share; the other eleven are ignored.
#[cfg_attr(not(test), allow(dead_code))]
fn base_field(text: &str) -> Option<&str> {
    text.lines().find_map(|line| {
        line.trim()
            .strip_prefix("base:")
            .map(|rest| rest.trim().trim_end_matches(','))
    })
}

/// Hex `#RRGGBBAA` (or `#RRGGBB`) to straight `0xRRGGBB`: the alpha
/// byte parsed then dropped.
#[cfg_attr(not(test), allow(dead_code))]
fn hex_accent(hex: &str) -> Option<u32> {
    let digits = hex.strip_prefix('#').unwrap_or(hex);
    match digits.len() {
        6 => u32::from_str_radix(digits, 16).ok(),
        8 => u32::from_str_radix(digits, 16).ok().map(|argb| argb >> 8),
        _ => None,
    }
}

/// The v1 tuple `(red: …, green: …, blue: …, alpha: …)` to straight
/// `0xRRGGBB`: each channel f32-to-u8, clamped to `0..=1`, rounded to
/// the nearest of 255, non-finite rejected; `alpha` ignored.
#[cfg_attr(not(test), allow(dead_code))]
fn f32_accent(group: &str) -> Option<u32> {
    let group = group.strip_prefix('(')?;
    let channel = |name: &str| -> Option<u8> {
        let rest = group.split_once(&format!("{name}:"))?.1;
        let end = rest.find([',', ')']).unwrap_or(rest.len());
        let value = rest[..end].trim().parse::<f32>().ok()?;
        if !value.is_finite() {
            return None;
        }
        Some((value.clamp(0.0, 1.0) * 255.0).round() as u8)
    };
    Some(
        (u32::from(channel("red")?)) << 16
            | (u32::from(channel("green")?)) << 8
            | u32::from(channel("blue")?),
    )
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

        /// Writes one store rung at `<dir>/<id>/v<version>/<key>`.
        fn rung(&self, id: &str, version: u32, key: &str, body: &str) {
            let dir = self.0.join(id).join(format!("v{version}"));
            std::fs::create_dir_all(&dir).expect("create rung dir");
            std::fs::write(dir.join(key), body).expect("write rung");
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

    // --- accent decode ----------------------------------------------------

    /// The system Dark v2 accent file, as the store record quoted it
    /// (cosmic-appearance-stores.md §3.1): the v2 serialization, every
    /// field a `#RRGGBBAA` string. The record elides nine of the twelve
    /// `Component` fields; the decoder reads only `base`, and the quoted
    /// `disabled_border` rides along as the no-confusion guard.
    const V2_ACCENT_DARK: &str = r##"(
    base: "#63D0DFFF",
    hover: "#63BAC6FF",
    disabled_border: "#63D0DF80",
)"##;

    /// This machine's live accent: the user Dark v1 `base` line, quoted
    /// by the record (§3.1) — the v1 serialization, named f32s. The
    /// values are 231/156/254 over 255: the purple this desktop has
    /// carried since v1, reachable only through the middle rung.
    const V1_ACCENT_PURPLE: &str =
        "base: (red: 0.9058823, green: 0.6117647, blue: 0.99607843, alpha: 1.0),";

    /// The user Light v2 accent `base`, quoted by the record (§3.1):
    /// this desktop runs purple-on-dark, dark-teal-on-light.
    const V2_ACCENT_LIGHT: &str = "base: \"#00525AFF\",";

    /// A user v2 rung the fixtures control: no real store carries this
    /// value, so a ladder test reading it proves the rung, not the value.
    const V2_ACCENT_MARKER: &str = "base: \"#010203FF\",";

    #[test]
    fn accent_decode_reads_the_v2_hex_fixtures() {
        assert_eq!(parse_accent_base(V2_ACCENT_DARK), Some(0x63_D0_DF));
        assert_eq!(parse_accent_base(V2_ACCENT_LIGHT), Some(0x00_52_5A));
    }

    #[test]
    fn accent_decode_reads_the_v1_f32_fixture() {
        assert_eq!(parse_accent_base(V1_ACCENT_PURPLE), Some(0xE7_9C_FE));
    }

    #[test]
    fn accent_decode_discards_alpha_in_both_encodings() {
        // Followed colors are forced opaque: the v2 hex's alpha byte and
        // the v1 `alpha` field are parsed then dropped — the band never
        // inherits translucency from the store.
        assert_eq!(
            parse_accent_base("base: \"#63D0DF80\",\n"),
            Some(0x63_D0_DF)
        );
        assert_eq!(
            parse_accent_base(
                "base: (red: 0.9058823, green: 0.6117647, blue: 0.99607843, alpha: 0.5),"
            ),
            Some(0xE7_9C_FE)
        );
    }

    #[test]
    fn accent_decode_is_lenient_about_case_and_the_hash_only() {
        // The store writes `#RRGGBBAA` uppercase; the digits are the
        // value, so case and the sigil are not the answer's shape.
        assert_eq!(parse_accent_base("base: \"63d0df\",\n"), Some(0x63_D0_DF));
        assert_eq!(
            parse_accent_base("base: \"#63d0dfff\",\n"),
            Some(0x63_D0_DF)
        );
    }

    #[test]
    fn accent_decode_rejects_garbage_without_panicking() {
        for text in [
            "",                                // empty file
            "hover: \"#63D0DFFF\",\n",         // no base field at all
            "base: \"#63D0D\",\n",             // 5 digits
            "base: \"#63D0DFF\",\n",           // 7 digits
            "base: \"#63D0DFGG\",\n",          // not hex
            "base: \"#63D0DFFF,\n",            // unterminated quote
            "base: \"\",\n",                   // empty string
            "base: 0.5,\n",                    // neither encoding
            "base: (red: 0.5, green: 0.5),\n", // missing blue
            "base: (red: banana, green: 0.5, blue: 0.5),\n",
            "base: (red: NaN, green: 0.5, blue: 0.5),\n", // non-finite
            "base: (red: inf, green: 0.5, blue: 0.5),\n", // non-finite
        ] {
            assert_eq!(
                parse_accent_base(text),
                None,
                "garbage must be a miss, never a panic: {text:?}"
            );
        }
    }

    // --- accent ladder ----------------------------------------------------

    #[test]
    fn the_ladder_prefers_user_v2() {
        let user = TempDir::new("ladder-u2");
        let system = TempDir::new("ladder-u2-system");
        user.rung(THEME_DARK_ID, 2, ACCENT_KEY, V2_ACCENT_MARKER);
        user.rung(THEME_DARK_ID, 1, ACCENT_KEY, V1_ACCENT_PURPLE);
        system.rung(THEME_DARK_ID, 2, ACCENT_KEY, V2_ACCENT_DARK);
        let (value, _) = accent_base_at(Some(&user.0), &system.0, true);
        assert_eq!(value, Some(0x01_02_03), "user v2 is the first rung");
    }

    #[test]
    fn user_v1_beats_system_v2() {
        // The live-purple rule (record §2): this machine's user v2 store
        // has no accent, so the v1 purple is the answer — the middle
        // rung is load-bearing, never skipped for the simpler ladder.
        let user = TempDir::new("ladder-u1");
        let system = TempDir::new("ladder-u1-system");
        user.rung(THEME_DARK_ID, 1, ACCENT_KEY, V1_ACCENT_PURPLE);
        system.rung(THEME_DARK_ID, 2, ACCENT_KEY, V2_ACCENT_DARK);
        let (value, _) = accent_base_at(Some(&user.0), &system.0, true);
        assert_eq!(value, Some(0xE7_9C_FE), "user v1 outranks system v2");
    }

    #[test]
    fn a_garbage_user_v2_falls_through_to_v1() {
        // A rung that exists but does not parse is a miss, the ladder's
        // own `preference_at` rule: garbage never poisons, the next rung
        // serves.
        let user = TempDir::new("ladder-garbage");
        let system = TempDir::new("ladder-garbage-system");
        user.rung(THEME_DARK_ID, 2, ACCENT_KEY, "base: \"not-a-color\",\n");
        user.rung(THEME_DARK_ID, 1, ACCENT_KEY, V1_ACCENT_PURPLE);
        let (value, _) = accent_base_at(Some(&user.0), &system.0, true);
        assert_eq!(value, Some(0xE7_9C_FE));
    }

    #[test]
    fn an_absent_user_store_falls_to_the_system_default() {
        let user = TempDir::new("ladder-sys");
        let system = TempDir::new("ladder-sys-system");
        system.rung(THEME_DARK_ID, 2, ACCENT_KEY, V2_ACCENT_DARK);
        let (value, _) = accent_base_at(Some(&user.0), &system.0, true);
        assert_eq!(value, Some(0x63_D0_DF));
        let (value, _) = accent_base_at(None, &system.0, true);
        assert_eq!(value, Some(0x63_D0_DF), "no user root at all");
    }

    #[test]
    fn a_total_ladder_miss_names_every_rung() {
        let user = TempDir::new("ladder-miss");
        let system = TempDir::new("ladder-miss-system");
        let (value, missed) = accent_base_at(Some(&user.0), &system.0, true);
        assert_eq!(value, None);
        assert_eq!(
            missed,
            vec![
                format!("user {THEME_DARK_ID}/v2/{ACCENT_KEY}"),
                format!("user {THEME_DARK_ID}/v1/{ACCENT_KEY}"),
                format!("system {THEME_DARK_ID}/v2/{ACCENT_KEY}"),
            ],
            "the one aggregated diagnostic names every rung it tried"
        );
    }

    #[test]
    fn each_polarity_reads_its_own_store() {
        // Per-polarity accents are real (record §3.1): the dark and
        // light stores hold different values on this very machine.
        let user = TempDir::new("ladder-polarity");
        let system = TempDir::new("ladder-polarity-system");
        user.rung(THEME_DARK_ID, 2, ACCENT_KEY, V2_ACCENT_DARK);
        user.rung(THEME_LIGHT_ID, 2, ACCENT_KEY, V2_ACCENT_LIGHT);
        let (dark, _) = accent_base_at(Some(&user.0), &system.0, true);
        let (light, _) = accent_base_at(Some(&user.0), &system.0, false);
        assert_eq!(dark, Some(0x63_D0_DF));
        assert_eq!(light, Some(0x00_52_5A));
    }

    #[test]
    fn ladder_reads_never_create_directories() {
        // cosmic-config's `Config::new` create_dir_all's the user store
        // as a side effect (record §2); Airlock only observes, so a read
        // against absent roots must leave the filesystem untouched.
        let base = TempDir::new("ladder-readonly");
        let user = base.0.join("user-cosmic");
        let system = base.0.join("system-cosmic");
        let (value, missed) = accent_base_at(Some(&user), &system, true);
        assert_eq!(value, None);
        assert_eq!(missed.len(), 3, "three rungs tried, three misses");
        assert!(!user.exists(), "no user store directory was created");
        assert!(!system.exists(), "no system store directory was created");
    }
}
