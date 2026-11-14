use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

/// Faithful test replica of cosmic's `custom` shortcuts schema
/// (cosmic-settings-daemon `config/src/shortcuts/{binding.rs,sym.rs,
/// action.rs}`): `Binding` denies unknown fields and its `key` field uses a
/// custom keysym deserializer that accepts only a **bare string** —
/// `key: Some("grave")` is a hard deserialization error, and because a
/// Binding is the map key, one bad entry poisons the whole file. That is
/// the 0.1.0 incident: cosmic-comp watched the file, failed to load it,
/// and fell back to factory defaults for the live session while the user's
/// bytes sat untouched on disk. Every file install emits must parse here.
mod cosmic_schema {
    use serde::{Deserialize, Deserializer, Serialize};
    use std::collections::HashMap;

    #[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
    pub enum Modifier {
        Ctrl,
        Alt,
        Shift,
        Super,
    }

    /// Bare-string keysym, mirroring cosmic's sym.rs: `Some(..)` rejected.
    fn deserialize_keysym<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer).map(Some)
    }

    #[derive(Debug, Deserialize, PartialEq, Eq, Hash)]
    #[serde(deny_unknown_fields)]
    pub struct Binding {
        pub modifiers: Vec<Modifier>,
        #[serde(default, deserialize_with = "deserialize_keysym")]
        pub key: Option<String>,
        #[serde(default)]
        pub description: Option<String>,
    }

    #[derive(Debug, Deserialize, PartialEq, Eq, Hash)]
    pub enum Action {
        Spawn(String),
        Disable,
    }

    pub type Shortcuts = HashMap<Binding, Action>;
}

/// Trimmed copy of the shipped system default (same shape: doc comments,
/// both switcher keys, unrelated entries).
const SYSTEM_DEFAULT: &str = concat!(
    "{\n",
    "    /// Opens the launcher\n",
    "    Launcher: \"cosmic-launcher\",\n",
    "    /// Opens the (alt+tab) window switcher\n",
    "    WindowSwitcher: \"cosmic-launcher alt-tab\",\n",
    "    /// Opens the (alt+shift+tab) window switcher\n",
    "    WindowSwitcherPrevious: \"cosmic-launcher shift-alt-tab\",\n",
    "    /// Locks the screen\n",
    "    LockScreen: \"loginctl lock-session\",\n",
    "}\n",
);

/// A user file with hostile formatting: double space after a colon,
/// trailing spaces, an unrelated key between the switcher keys.
const USER_EXISTING: &str = concat!(
    "{\n",
    "    WindowSwitcher:  \"cosmic-launcher alt-tab\",  \n",
    "    Mute: \"wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle\",\n",
    "    WindowSwitcherPrevious: \"cosmic-launcher shift-alt-tab\",\n",
    "}\n",
);

/// User file that trimmed one switcher key: install must append it.
const USER_MISSING_PREV: &str = concat!(
    "{\n",
    "    WindowSwitcher: \"cosmic-launcher alt-tab\",\n",
    "    Mute: \"wpctl\",\n",
    "}\n",
);

/// A takeover written by hand, without airlock's markers.
const USER_HAND_EDITED: &str = concat!(
    "{\n",
    "    WindowSwitcher: \"airlock alt-tab\",\n",
    "    WindowSwitcherPrevious: \"airlock shift-alt-tab\",\n",
    "}\n",
);

/// A markerless file where only one switcher key runs airlock and the
/// sibling runs a third-party tool: "using something else → ignore"
/// applies per key, never per file.
const USER_ONE_KEY_TAKEOVER: &str = concat!(
    "{\n",
    "    WindowSwitcher: \"airlock alt-tab\",\n",
    "    WindowSwitcherPrevious: \"my-tool\",\n",
    "}\n",
);

/// Round-trip escape torture: backslashes in the command, a CR at line
/// end.
const USER_BACKSLASH_CR: &str = concat!(
    "{\n",
    "    WindowSwitcher: \"cmd\\\\with\\\\backslashes\",\r\n",
    "    WindowSwitcherPrevious: \"prev\",\n",
    "}\n",
);

/// A `custom` file as cosmic-settings writes it: one user Spawn shortcut,
/// multi-line binding keys, bare modifier names, a **bare-string keysym**
/// (`key: "A"` — cosmic's custom deserializer rejects `Some(..)`), trailing
/// commas, no final newline.
const CUSTOM_USER_FILE: &str = concat!(
    "{\n",
    "    (\n",
    "        modifiers: [\n",
    "            Super,\n",
    "        ],\n",
    "        key: \"A\",\n",
    "    ): Spawn(\"xeyes\"),\n",
    "}",
);

/// A user-held Alt+grave among an unrelated entry: install must skip the
/// grave binding and leave both user entries byte-exact.
const CUSTOM_HELD_GRAVE: &str = concat!(
    "{\n",
    "    (\n",
    "        modifiers: [\n",
    "            Alt,\n",
    "        ],\n",
    "        key: \"grave\",\n",
    "    ): Spawn(\"my own thing\"),\n",
    "    (\n",
    "        modifiers: [\n",
    "            Super,\n",
    "        ],\n",
    "        key: \"A\",\n",
    "    ): Spawn(\"xeyes\"),\n",
    "}",
);

/// cosmic-settings rewrote the file after an airlock install: our entry
/// re-serialized in cosmic's multi-line shape (the in-file marker comment
/// did not survive the rewrite). Uninstall must still find and remove it —
/// and leave the user entry exactly as cosmic wrote it.
const CUSTOM_REWRITTEN: &str = concat!(
    "{\n",
    "    (\n",
    "        modifiers: [\n",
    "            Alt,\n",
    "        ],\n",
    "        key: \"grave\",\n",
    "    ): Spawn(\"airlock same-app\"),\n",
    "    (\n",
    "        modifiers: [\n",
    "            Super,\n",
    "        ],\n",
    "        key: \"A\",\n",
    "    ): Spawn(\"xeyes\"),\n",
    "}",
);

/// A `custom` file exactly as 0.1.0 wrote it: airlock's three entries with
/// the schema-invalid `key: Some(..)` form — the bytes that made cosmic-comp
/// fall back to factory defaults. Reinstall must heal the file to the valid
/// shape and uninstall must still recognize and remove the entries.
const CUSTOM_LEGACY_0_1_0: &str = concat!(
    "// airlock-v1 custom=absent-file\n",
    "{\n",
    "    (modifiers: [Alt], key: Some(\"grave\"), description: Some(\"Airlock: same app windows\")): Spawn(\"airlock same-app\"),\n",
    "    (modifiers: [Alt, Shift], key: Some(\"grave\"), description: Some(\"Airlock: same app windows, backwards\")): Spawn(\"airlock same-app-previous\"),\n",
    "    (modifiers: [Alt], key: Some(\"Esc\"), description: Some(\"Airlock: quick toggle\")): Spawn(\"airlock quick-toggle\"),\n",
    "}\n",
);

#[test]
fn fresh_install_overrides_system_default_and_marks_absent_file() {
    let installed = install_content(None, SYSTEM_DEFAULT).expect("install");

    assert!(
        installed.contains(&format!("WindowSwitcher: \"{NEXT_CMD}\"")),
        "WindowSwitcher not overridden:\n{installed}"
    );
    assert!(
        installed.contains(&format!("WindowSwitcherPrevious: \"{PREV_CMD}\"")),
        "WindowSwitcherPrevious not overridden:\n{installed}"
    );
    // Unrelated entries and comments survive the copy.
    assert!(installed.contains("Launcher: \"cosmic-launcher\""));
    assert!(installed.contains("/// Locks the screen"));
    // Both markers record that no user file existed.
    let marker_lines: Vec<&str> = installed
        .lines()
        .take_while(|l| l.starts_with(MARKER_PREFIX))
        .collect();
    assert_eq!(marker_lines.len(), 2, "expected two markers:\n{installed}");
    assert!(
        installed.contains(&format!(
            "{MARKER_PREFIX}{SWITCHER_KEY}={MARKER_ABSENT_FILE}"
        )),
        "missing absent-file marker:\n{installed}"
    );
}

#[test]
fn fresh_install_uninstall_removes_file() {
    let installed = install_content(None, SYSTEM_DEFAULT).expect("install");
    match uninstall_content(&installed, SYSTEM_DEFAULT).expect("uninstall") {
        UninstallOutcome::RemoveFile => {}
        other => panic!("expected RemoveFile, got {other:?}"),
    }
}

#[test]
fn install_preserves_unrelated_lines_and_formatting() {
    let installed = install_content(Some(USER_EXISTING), SYSTEM_DEFAULT).expect("install");

    // The unrelated entry survives byte-for-byte, comments and all.
    assert!(
        installed.contains("    Mute: \"wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle\",\n"),
        "unrelated entry changed:\n{installed}"
    );
    // The marker records the exact original line, weird spacing included.
    assert!(
        installed.contains(&format!(
            "{MARKER_PREFIX}{SWITCHER_KEY}=    WindowSwitcher:  \"cosmic-launcher alt-tab\",  "
        )),
        "original line not recorded verbatim:\n{installed}"
    );
    // Markers lead the file; body starts with the original brace.
    assert!(installed.starts_with(MARKER_PREFIX));
}

#[test]
fn install_then_uninstall_restores_exact_bytes() {
    let installed = install_content(Some(USER_EXISTING), SYSTEM_DEFAULT).expect("install");
    match uninstall_content(&installed, SYSTEM_DEFAULT).expect("uninstall") {
        UninstallOutcome::Restored {
            content,
            best_effort,
        } => {
            assert!(!best_effort, "marked restore should be exact");
            assert_eq!(content, USER_EXISTING);
        }
        other => panic!("expected Restored, got {other:?}"),
    }
}

#[test]
fn second_install_is_byte_identical_and_keeps_original_markers() {
    let once = install_content(Some(USER_EXISTING), SYSTEM_DEFAULT).expect("install");
    // The markers in `once` still name cosmic-launcher; a second run must
    // not re-embed the now-installed airlock state as the "original".
    assert!(once.contains("cosmic-launcher alt-tab"));

    let twice = install_content(Some(&once), SYSTEM_DEFAULT).expect("reinstall");
    assert_eq!(once, twice, "second install changed the file");
}

#[test]
fn install_appends_missing_key_and_uninstall_removes_it() {
    let installed = install_content(Some(USER_MISSING_PREV), SYSTEM_DEFAULT).expect("install");
    assert!(
        installed.contains(&format!("WindowSwitcherPrevious: \"{PREV_CMD}\"")),
        "missing key not appended:\n{installed}"
    );

    match uninstall_content(&installed, SYSTEM_DEFAULT).expect("uninstall") {
        UninstallOutcome::Restored {
            content,
            best_effort,
        } => {
            assert!(!best_effort);
            assert_eq!(
                content, USER_MISSING_PREV,
                "appended line removal not exact"
            );
        }
        other => panic!("expected Restored, got {other:?}"),
    }
}

#[test]
fn uninstall_of_hand_edited_takeover_restores_system_default_values() {
    match uninstall_content(USER_HAND_EDITED, SYSTEM_DEFAULT).expect("uninstall") {
        UninstallOutcome::Restored {
            content,
            best_effort,
        } => {
            assert!(
                best_effort,
                "markerless restore should be flagged best-effort"
            );
            assert_eq!(
                content,
                concat!(
                    "{\n",
                    "    WindowSwitcher: \"cosmic-launcher alt-tab\",\n",
                    "    WindowSwitcherPrevious: \"cosmic-launcher shift-alt-tab\",\n",
                    "}\n",
                ),
                "values not restored from the system default:\n{content}"
            );
        }
        other => panic!("expected Restored, got {other:?}"),
    }
}

#[test]
fn markerless_one_key_takeover_leaves_third_party_sibling_untouched() {
    match uninstall_content(USER_ONE_KEY_TAKEOVER, SYSTEM_DEFAULT).expect("uninstall") {
        UninstallOutcome::Restored {
            content,
            best_effort,
        } => {
            assert!(
                best_effort,
                "markerless restore should be flagged best-effort"
            );
            assert_eq!(
                content,
                concat!(
                    "{\n",
                    "    WindowSwitcher: \"cosmic-launcher alt-tab\",\n",
                    "    WindowSwitcherPrevious: \"my-tool\",\n",
                    "}\n",
                ),
                "airlock key not restored / sibling not byte-identical:\n{content}"
            );
        }
        other => panic!("expected Restored, got {other:?}"),
    }
}

#[test]
fn uninstall_after_marker_strip_restores_builtin_switcher() {
    let installed = install_content(Some(USER_EXISTING), SYSTEM_DEFAULT).expect("install");
    // A cosmic-settings rewrite keeps the body but drops the leading
    // marker comments — the realistic degradation while installed.
    let stripped: String = installed.split_inclusive('\n').skip(2).collect();

    match uninstall_content(&stripped, SYSTEM_DEFAULT).expect("uninstall") {
        UninstallOutcome::Restored {
            content,
            best_effort,
        } => {
            assert!(
                best_effort,
                "markerless restore should be flagged best-effort"
            );
            assert_eq!(
                content,
                concat!(
                    "{\n",
                    "    WindowSwitcher: \"cosmic-launcher alt-tab\",\n",
                    "    Mute: \"wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle\",\n",
                    "    WindowSwitcherPrevious: \"cosmic-launcher shift-alt-tab\",\n",
                    "}\n",
                ),
                "built-in not restored / unrelated lines changed:\n{content}"
            );
        }
        other => panic!("expected Restored, got {other:?}"),
    }
}

#[test]
fn uninstall_without_airlock_values_reports_not_installed() {
    match uninstall_content(USER_EXISTING, SYSTEM_DEFAULT).expect("uninstall") {
        UninstallOutcome::NotInstalled => {}
        other => panic!("expected NotInstalled, got {other:?}"),
    }
}

#[test]
fn marker_escaping_survives_backslashes_and_carriage_returns() {
    let installed = install_content(Some(USER_BACKSLASH_CR), SYSTEM_DEFAULT).expect("install");
    match uninstall_content(&installed, SYSTEM_DEFAULT).expect("uninstall") {
        UninstallOutcome::Restored {
            content,
            best_effort,
        } => {
            assert!(!best_effort);
            assert_eq!(content, USER_BACKSLASH_CR);
        }
        other => panic!("expected Restored, got {other:?}"),
    }
}

#[test]
fn empty_user_file_is_rejected() {
    let result = install_content(Some(""), SYSTEM_DEFAULT);
    assert!(
        result.is_err(),
        "empty file must be rejected, got {result:?}"
    );
}

// --- `custom` shortcuts file ---

#[test]
fn custom_fresh_install_creates_marked_file_and_uninstall_removes_it() {
    let install = custom_install_content(None, "Escape").expect("install");
    assert_eq!(
        install.in_place,
        [
            "Alt+grave",
            "Alt+Shift+grave",
            "Alt+Escape",
            "Alt+Shift+Escape"
        ]
    );
    assert!(install.skipped.is_empty(), "{:?}", install.skipped);
    assert!(
        install
            .content
            .starts_with(&format!("{CUSTOM_ABSENT_MARKER}\n{{\n")),
        "missing absent-file marker:\n{}",
        install.content
    );
    for command in [
        SAME_APP_CMD,
        SAME_APP_PREV_CMD,
        SHOW_ALL_CMD,
        SHOW_ALL_PREV_CMD,
    ] {
        assert!(
            install.content.contains(&format!("Spawn(\"{command}\")")),
            "{command} not bound:\n{}",
            install.content
        );
    }
    // Quick-toggle is retired: no binding spawns it anymore.
    assert!(
        !install.content.contains(QUICK_TOGGLE_CMD),
        "quick-toggle must be bound nowhere:\n{}",
        install.content
    );

    // Re-install over the fresh file is a byte-identical no-op.
    let again = custom_install_content(Some(&install.content), "Escape").expect("reinstall");
    assert_eq!(again.content, install.content);

    match custom_uninstall_content(&install.content, "Escape").expect("uninstall") {
        CustomUninstall::RemoveFile => {}
        other => panic!("expected RemoveFile, got {other:?}"),
    }
}

#[test]
fn custom_install_preserves_user_entries_and_uninstall_restores_bytes() {
    let install = custom_install_content(Some(CUSTOM_USER_FILE), "Escape").expect("install");
    assert_eq!(install.in_place.len(), 4);
    assert!(
        install.content.contains("): Spawn(\"xeyes\"),\n"),
        "user entry lost:\n{}",
        install.content
    );

    match custom_uninstall_content(&install.content, "Escape").expect("uninstall") {
        CustomUninstall::Restored { content } => assert_eq!(content, CUSTOM_USER_FILE),
        other => panic!("expected Restored, got {other:?}"),
    }
}

#[test]
fn custom_install_output_parses_as_cosmic_shortcut_map() {
    // THE 0.1.0 regression guard: whatever install emits must deserialize
    // as cosmic's Binding→Action map. A malformed map key (0.1.0's
    // `key: Some(..)`) makes cosmic-comp discard the whole file and run
    // factory defaults for the live session — every user shortcut appears
    // reset. Fails loudly with the file content if this ever regresses.
    for (label, expected, content) in [
        (
            "fresh install",
            4,
            custom_install_content(None, "Escape")
                .expect("fresh install")
                .content,
        ),
        (
            "over a cosmic-written user file",
            5,
            custom_install_content(Some(CUSTOM_USER_FILE), "Escape")
                .expect("install over user file")
                .content,
        ),
    ] {
        let parsed: cosmic_schema::Shortcuts = ron::from_str(&content).unwrap_or_else(
            |error| {
                panic!("cosmic cannot load this file ({label}); live session would reset to defaults: {error}\n{content}")
            },
        );
        assert_eq!(
            parsed.len(),
            expected,
            "wrong entry count ({label}):\n{content}"
        );
    }
}

#[test]
fn emitted_bindings_use_bare_keysyms_and_canonical_names() {
    let install = custom_install_content(None, "Escape").expect("install");
    assert!(
        install.content.contains("key: \"grave\""),
        "keysym must be a bare string:\n{}",
        install.content
    );
    assert!(
        install.content.contains("key: \"Escape\""),
        "the all-windows family must bind the canonical Escape keysym:\n{}",
        install.content
    );
    assert!(
        !install.content.contains("key: Some("),
        "0.1.0's Some-form must never be emitted again:\n{}",
        install.content
    );
}

#[test]
fn legacy_0_1_0_file_is_healed_by_reinstall() {
    // Re-running install over 0.1.0's malformed file replaces the
    // Some-form entries with valid ones — a repair path for bitten users.
    // The 0.1.0 quick-toggle entry on "Esc" is retired-ours: dropped and
    // counted as a migration, not left as a duplicate or a skip.
    let healed = custom_install_content(Some(CUSTOM_LEGACY_0_1_0), "Escape").expect("reinstall");
    assert_eq!(healed.in_place.len(), 4, "{:?}", healed.in_place);
    assert_eq!(healed.migrated, ["Alt+Esc"], "{:?}", healed.migrated);
    let parsed: cosmic_schema::Shortcuts =
        ron::from_str(&healed.content).expect("healed file must parse for cosmic");
    assert_eq!(
        parsed.len(),
        4,
        "only airlock's entries:\n{}",
        healed.content
    );
}

#[test]
fn legacy_0_1_0_file_uninstalls_and_restores_user_bytes() {
    // 0.1.0 entries sitting next to user entries: recognized semantically,
    // removed, user bytes preserved exactly.
    let legacy_with_user = concat!(
        "{\n",
        "    (modifiers: [Alt], key: Some(\"grave\"), description: Some(\"Airlock: same app windows\")): Spawn(\"airlock same-app\"),\n",
        "    (\n        modifiers: [\n            Super,\n        ],\n        key: \"A\",\n    ): Spawn(\"xeyes\"),\n",
        "}",
    );
    match custom_uninstall_content(legacy_with_user, "Escape").expect("uninstall") {
        CustomUninstall::Restored { content } => assert_eq!(
            content,
            concat!(
                "{\n",
                "    (\n        modifiers: [\n            Super,\n        ],\n        key: \"A\",\n    ): Spawn(\"xeyes\"),\n",
                "}",
            ),
            "user entry not byte-preserved:\n{content}"
        ),
        other => panic!("expected Restored, got {other:?}"),
    }

    // The pure 0.1.0 fresh-install file: only airlock owned it (grave
    // pair plus the retired quick-toggle), so the marker path deletes it.
    match custom_uninstall_content(CUSTOM_LEGACY_0_1_0, "Escape").expect("uninstall") {
        CustomUninstall::RemoveFile => {}
        other => panic!("expected RemoveFile, got {other:?}"),
    }
}

#[test]
fn custom_install_skips_user_held_binding_and_names_it() {
    let install = custom_install_content(Some(CUSTOM_HELD_GRAVE), "Escape").expect("install");
    assert_eq!(install.skipped, ["Alt+grave"]);
    assert_eq!(
        install.in_place,
        ["Alt+Shift+grave", "Alt+Escape", "Alt+Shift+Escape"]
    );
    assert!(
        install.content.contains("Spawn(\"my own thing\")"),
        "held entry modified:\n{}",
        install.content
    );

    // The held entry survives the round-trip untouched.
    match custom_uninstall_content(&install.content, "Escape").expect("uninstall") {
        CustomUninstall::Restored { content } => assert_eq!(content, CUSTOM_HELD_GRAVE),
        other => panic!("expected Restored, got {other:?}"),
    }
}

#[test]
fn custom_reinstall_is_byte_identical() {
    let once = custom_install_content(Some(CUSTOM_USER_FILE), "Escape").expect("install");
    let twice = custom_install_content(Some(&once.content), "Escape").expect("reinstall");
    assert_eq!(
        once.content, twice.content,
        "second install changed the file"
    );
}

#[test]
fn custom_install_after_cosmic_rewrite_refreshes_without_duplicates() {
    let install = custom_install_content(Some(CUSTOM_REWRITTEN), "Escape").expect("install");
    assert_eq!(install.in_place.len(), 4);
    assert_eq!(
        install
            .content
            .matches(&format!("Spawn(\"{SAME_APP_CMD}\")"))
            .count(),
        1,
        "duplicate same-app binding:\n{}",
        install.content
    );

    // Uninstall still recognizes the re-serialized entry and restores the
    // user-only file byte-exactly.
    match custom_uninstall_content(&install.content, "Escape").expect("uninstall") {
        CustomUninstall::Restored { content } => assert_eq!(content, CUSTOM_USER_FILE),
        other => panic!("expected Restored, got {other:?}"),
    }
}

#[test]
fn custom_uninstall_without_airlock_entries_reports_not_installed() {
    match custom_uninstall_content(CUSTOM_USER_FILE, "Escape").expect("uninstall") {
        CustomUninstall::NotInstalled => {}
        other => panic!("expected NotInstalled, got {other:?}"),
    }
}

#[test]
fn custom_disable_action_on_our_key_is_held_not_clobbered() {
    // Hand-written compact file: the user disabled Alt+Escape themselves.
    // Bare keysym in cosmic's real form (canonical "Escape" name).
    let held = "{\n    (modifiers: [Alt], key: \"Escape\"): Disable,\n}";
    let install = custom_install_content(Some(held), "Escape").expect("install");
    assert_eq!(install.skipped, ["Alt+Escape"]);
    // The Shift twin is free and installs; only the held key is skipped.
    assert_eq!(
        install.in_place,
        ["Alt+grave", "Alt+Shift+grave", "Alt+Shift+Escape"]
    );
    assert!(
        install.content.contains("Disable,"),
        "user's Disable entry modified:\n{}",
        install.content
    );
}

#[test]
fn custom_modifier_order_and_compact_forms_are_recognized() {
    // Reordered modifiers on compact single-line entries: still held
    // bindings — in cosmic's bare-keysym form and in 0.1.0's Some-form.
    let bare = concat!(
        "{\n",
        "    (modifiers: [Shift, Alt], key: \"grave\"): Spawn(\"other\"),\n",
        "}\n",
    );
    assert_eq!(
        custom_install_content(Some(bare), "Escape")
            .expect("install")
            .skipped,
        ["Alt+Shift+grave"]
    );

    let legacy_some = concat!(
        "{\n",
        "    (modifiers: [Shift, Alt], key: Some(\"grave\")): Spawn(\"other\"),\n",
        "}\n",
    );
    assert_eq!(
        custom_install_content(Some(legacy_some), "Escape")
            .expect("install")
            .skipped,
        ["Alt+Shift+grave"]
    );
}

#[test]
fn custom_description_with_paren_colon_does_not_break_parsing() {
    // cosmic-settings copies the user's description verbatim; a `):` inside
    // the string must not look like the end of the binding key.
    let nasty = concat!(
        "{\n",
        "    (\n",
        "        modifiers: [\n",
        "            Super,\n",
        "        ],\n",
        "        key: Some(\"B\"),\n",
        "        description: Some(\"launch thing ): carefully\"),\n",
        "    ): Spawn(\"xeyes\"),\n",
        "}\n",
    );
    let install = custom_install_content(Some(nasty), "Escape").expect("install");
    assert_eq!(install.in_place.len(), 4);
    assert!(
        install.content.contains("launch thing ): carefully"),
        "nasty description lost:\n{}",
        install.content
    );
    assert_eq!(
        install.content.matches("): Spawn(\"xeyes\")").count(),
        1,
        "user entry mangled:\n{}",
        install.content
    );
}

#[test]
fn custom_empty_map_file_gains_entries_and_uninstalls_to_empty_map() {
    let install = custom_install_content(Some("{}"), "Escape").expect("install");
    assert_eq!(install.in_place.len(), 4);
    match custom_uninstall_content(&install.content, "Escape").expect("uninstall") {
        CustomUninstall::Restored { content } => assert_eq!(content, "{}"),
        other => panic!("expected Restored, got {other:?}"),
    }
}

// --- The Esc family, retirement, migration (all-windows A3.1) ----------

#[test]
fn custom_install_binds_the_esc_family_with_the_configured_key() {
    // The default and a configured key alike: both twins written, the
    // configured name verbatim.
    let default = custom_install_content(None, "Escape").expect("default key");
    assert_eq!(
        default.in_place,
        [
            "Alt+grave",
            "Alt+Shift+grave",
            "Alt+Escape",
            "Alt+Shift+Escape"
        ]
    );
    assert_eq!(
        default.content.matches("key: \"Escape\"").count(),
        2,
        "both twins bind Escape:\n{}",
        default.content
    );

    let tab = custom_install_content(None, "Tab").expect("configured key");
    assert_eq!(
        tab.in_place,
        ["Alt+grave", "Alt+Shift+grave", "Alt+Tab", "Alt+Shift+Tab"]
    );
    assert_eq!(
        tab.content.matches("key: \"Tab\"").count(),
        2,
        "both twins bind the configured key verbatim:\n{}",
        tab.content
    );
    // A non-Escape configured key carries no Esc legacy: 0.1.0's
    // quick-toggle entry on "Esc" must not hold the Tab binding.
    let legacy = concat!(
        "{\n",
        "    (modifiers: [Alt], key: Some(\"Esc\")): Spawn(\"airlock quick-toggle\"),\n",
        "}\n",
    );
    let over_legacy = custom_install_content(Some(legacy), "Tab").expect("install");
    assert!(over_legacy.skipped.is_empty(), "{:?}", over_legacy.skipped);
    assert_eq!(over_legacy.migrated, ["Alt+Esc"]);
}

#[test]
fn custom_install_migrates_a_retired_quick_toggle_entry() {
    // The 0.4-era default binding, in cosmic's real serialization and in
    // 0.1.0's legacy `Some("Esc")` form: both are airlock-owned — removed
    // and rebound to the new command, one migration line, no duplicate.
    for (label, entry) in [
        (
            "canonical keysym",
            concat!(
                "{\n",
                "    (modifiers: [Alt], key: \"Escape\", description: Some(\"Airlock: quick toggle\")): Spawn(\"airlock quick-toggle\"),\n",
                "}\n",
            ),
        ),
        (
            "legacy Some(Esc)",
            concat!(
                "{\n",
                "    (modifiers: [Alt], key: Some(\"Esc\"), description: Some(\"Airlock: quick toggle\")): Spawn(\"airlock quick-toggle\"),\n",
                "}\n",
            ),
        ),
    ] {
        let install = custom_install_content(Some(entry), "Escape").expect("install migrates");
        assert_eq!(
            install.migrated,
            ["Alt+Esc"],
            "{label}: {:?}",
            install.migrated
        );
        assert!(
            install.skipped.is_empty(),
            "{label}: a retired entry is never a skip: {:?}",
            install.skipped
        );
        assert_eq!(
            install.content.matches("Escape").count(),
            2,
            "{label}: exactly one Escape-keyed binding pair survives:\n{}",
            install.content
        );
        assert!(
            !install.content.contains(QUICK_TOGGLE_CMD),
            "{label}: the retired command must be gone:\n{}",
            install.content
        );
        assert!(
            install
                .content
                .contains(&format!("Spawn(\"{SHOW_ALL_CMD}\")")),
            "{label}: the new command must be bound:\n{}",
            install.content
        );
    }
}

#[test]
fn custom_install_skips_a_foreign_entry_on_the_esc_key() {
    // A genuinely user-held Escape binding — different command — is
    // never taken: skipped with the diagnostic, entry untouched. The
    // Shift twin still installs.
    let held = concat!(
        "{\n",
        "    (modifiers: [Alt], key: \"Escape\"): Spawn(\"xeyes\"),\n",
        "}\n",
    );
    let install = custom_install_content(Some(held), "Escape").expect("install");
    assert_eq!(install.skipped, ["Alt+Escape"]);
    assert!(
        install.migrated.is_empty(),
        "a foreign entry is never a migration: {:?}",
        install.migrated
    );
    assert!(
        install.content.contains("Spawn(\"xeyes\")"),
        "foreign entry modified:\n{}",
        install.content
    );
    assert!(
        !install
            .content
            .contains(&format!("Spawn(\"{SHOW_ALL_CMD}\")")),
        "a held key must not gain the new binding:\n{}",
        install.content
    );
}

/// Unique disposable directory (no tempfile dependency).
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "airlock-install-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        TempDir(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write_file(path: &Path, content: &str) {
    fs::create_dir_all(path.parent().expect("fixture parent")).expect("create fixture dir");
    fs::write(path, content).expect("write fixture");
}

fn read_file(path: &Path) -> String {
    String::from_utf8(fs::read(path).expect("read file")).expect("utf-8")
}

fn locations_for(dir: &Path) -> Locations {
    let shortcuts = dir.join("config/cosmic/com.system76.CosmicSettings.Shortcuts/v1");
    let data = dir.join("share");
    Locations {
        system_actions: shortcuts.join("system_actions"),
        system_default: dir.join("usr-share/system_actions"),
        custom: shortcuts.join("custom"),
        icon: data.join("icons/hicolor/scalable/apps/io.github.l1br3-io.Airlock.svg"),
        desktop: data.join("applications/io.github.l1br3-io.Airlock.desktop"),
        config: dir.join("config/airlock/config.toml"),
    }
}

#[test]
fn run_at_installs_and_restores_both_files_touching_nothing_else() {
    let dir = TempDir::new("e2e");
    let locations = locations_for(dir.path());
    let shortcuts_dir = locations.system_actions.parent().unwrap().to_path_buf();
    fs::create_dir_all(&shortcuts_dir).expect("create shortcuts dir");
    write_file(&locations.system_default, SYSTEM_DEFAULT);
    write_file(&locations.system_actions, USER_EXISTING);
    write_file(&locations.custom, CUSTOM_USER_FILE);

    let report = run_at(false, &locations).expect("install");
    assert!(report.contains("custom"), "report skips custom: {report}");
    let installed_actions = read_file(&locations.system_actions);
    assert!(installed_actions.contains(&format!("WindowSwitcher: \"{NEXT_CMD}\"")));
    let installed_custom = read_file(&locations.custom);
    assert!(
        installed_custom.contains(&format!("Spawn(\"{SHOW_ALL_CMD}\")")),
        "show-all-windows not installed:\n{installed_custom}"
    );
    assert!(
        !installed_custom.contains(QUICK_TOGGLE_CMD),
        "quick-toggle must be bound nowhere:\n{installed_custom}"
    );
    assert!(
        installed_custom.contains("): Spawn(\"xeyes\"),\n"),
        "user custom entry lost:\n{installed_custom}"
    );

    run_at(true, &locations).expect("uninstall");
    assert_eq!(read_file(&locations.system_actions), USER_EXISTING);
    assert_eq!(read_file(&locations.custom), CUSTOM_USER_FILE);

    // Only the two known files exist: no backups, no leftover temporaries.
    let mut names: Vec<String> = fs::read_dir(&shortcuts_dir)
        .expect("list shortcuts dir")
        .map(|e| {
            e.expect("dir entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec!["custom".to_string(), "system_actions".to_string()]
    );
}

#[test]
fn run_at_creates_missing_config_dirs_on_fresh_install() {
    let dir = TempDir::new("fresh");
    let locations = locations_for(dir.path());
    write_file(&locations.system_default, SYSTEM_DEFAULT);

    run_at(false, &locations).expect("fresh install");
    assert!(locations.system_actions.is_file(), "user file not created");
    assert!(
        read_file(&locations.system_actions).contains(&format!("WindowSwitcher: \"{NEXT_CMD}\""))
    );
    assert!(locations.custom.is_file(), "custom file not created");
    assert!(
        read_file(&locations.custom).contains(&format!("Spawn(\"{SAME_APP_CMD}\")")),
        "same-app not installed"
    );
    assert!(
        locations.config.is_file(),
        "the starter config must appear on a fresh install"
    );

    run_at(true, &locations).expect("uninstall");
    assert!(
        !locations.system_actions.exists(),
        "absent-file restore must delete the file"
    );
    assert!(
        !locations.custom.exists(),
        "absent-file restore must delete the custom file"
    );
    assert!(
        !locations.config.exists(),
        "the unedited starter must leave with the rest"
    );
}

// --- Install reads all_windows_key from the config (all-windows A3.1) ---

#[test]
fn install_binds_the_esc_family_from_a_configured_all_windows_key() {
    let dir = TempDir::new("configured-key");
    let locations = locations_for(dir.path());
    write_file(&locations.system_default, SYSTEM_DEFAULT);
    write_file(&locations.config, "all_windows_key = \"Tab\"\n");

    let report = run_at(false, &locations).expect("install");
    let installed_custom = read_file(&locations.custom);
    assert_eq!(
        installed_custom.matches("key: \"Tab\"").count(),
        2,
        "both twins must bind the configured key:\n{installed_custom}"
    );
    assert!(
        !installed_custom.contains("key: \"Escape\""),
        "the default must not leak over a configured key:\n{installed_custom}"
    );
    // A present, parseable config is not a fallback: the report stays
    // quiet about one.
    assert!(
        !report.contains("fell back"),
        "a valid config must not report a fallback: {report}"
    );
}

#[test]
fn install_falls_back_to_the_default_key_over_a_malformed_config() {
    let dir = TempDir::new("malformed-key");
    let locations = locations_for(dir.path());
    write_file(&locations.system_default, SYSTEM_DEFAULT);
    write_file(&locations.config, "all_windows_key = \n");

    let report = run_at(false, &locations).expect("install never fails over config");
    assert!(
        report.contains("all_windows_key fell back to Escape"),
        "the report must name the fallback: {report}"
    );
    let installed_custom = read_file(&locations.custom);
    assert_eq!(
        installed_custom.matches("key: \"Escape\"").count(),
        2,
        "the default key stands:\n{installed_custom}"
    );
}

// --- starter config ---

#[test]
fn the_starter_config_parses_to_the_defaults() {
    // The starter may never change behavior: as shipped (all lines
    // commented) it must parse to the same settings as no file at all.
    let settings =
        crate::settings::Settings::parse(DEFAULT_CONFIG).expect("the starter must be valid TOML");
    assert_eq!(
        settings,
        crate::settings::Settings::default(),
        "the starter changes behavior as shipped:\n{settings:?}"
    );
}

#[test]
fn the_starter_documents_the_real_defaults_and_bounds() {
    // The starter's commented literals must track the constants: a
    // re-pin of DEFAULT_ROWS or either ROWS bound touches this test,
    // the same tripwire settings.rs keeps on its diagnostic text.
    let text = std::str::from_utf8(DEFAULT_CONFIG).expect("starter is UTF-8");
    assert!(
        text.contains(&format!("# rows = {}", crate::ui::DEFAULT_ROWS)),
        "starter must document the default rows:\n{text}"
    );
    assert!(
        text.contains(&format!("{}–{}", crate::ui::ROWS_MIN, crate::ui::ROWS_MAX)),
        "starter must document the accepted rows range:\n{text}"
    );
}

#[test]
fn install_writes_the_starter_config_when_absent() {
    let dir = TempDir::new("config-fresh");
    let locations = locations_for(dir.path());
    write_file(&locations.system_default, SYSTEM_DEFAULT);

    let report = run_at(false, &locations).expect("install");
    assert_eq!(
        fs::read(&locations.config).expect("starter written"),
        DEFAULT_CONFIG
    );
    assert!(
        report.contains("starter config: wrote"),
        "report must say the starter was written: {report}"
    );
}

#[test]
fn install_never_touches_an_existing_config() {
    let dir = TempDir::new("config-user");
    let locations = locations_for(dir.path());
    write_file(&locations.system_default, SYSTEM_DEFAULT);
    let user_config = "rows = 5\n";
    write_file(&locations.config, user_config);

    let report = run_at(false, &locations).expect("install");
    assert_eq!(
        read_file(&locations.config),
        user_config,
        "an existing config is the user's, however it got there"
    );
    assert!(
        report.contains("left untouched"),
        "report must say the config was left alone: {report}"
    );
}

#[test]
fn uninstall_removes_the_starter_config() {
    let dir = TempDir::new("config-undo");
    let locations = locations_for(dir.path());
    write_file(&locations.system_default, SYSTEM_DEFAULT);

    run_at(false, &locations).expect("install");
    let report = run_at(true, &locations).expect("uninstall");
    assert!(
        !locations.config.exists(),
        "a byte-identical starter is airlock's to remove"
    );
    assert!(
        report.contains("removed starter config") && !report.contains("kept config"),
        "report must say removed, not kept: {report}"
    );
}

#[test]
fn uninstall_absent_config_reports_nothing_to_uninstall() {
    let dir = TempDir::new("config-absent");
    let locations = locations_for(dir.path());
    write_file(&locations.system_default, SYSTEM_DEFAULT);

    let report = run_at(true, &locations).expect("uninstall report");
    assert!(
        report.contains("nothing to uninstall: no starter config present"),
        "the absent-config line must be pinned verbatim: {report}"
    );
}

#[test]
fn uninstall_keeps_a_user_edited_config_and_names_it() {
    let dir = TempDir::new("config-edited");
    let locations = locations_for(dir.path());
    write_file(&locations.system_default, SYSTEM_DEFAULT);

    run_at(false, &locations).expect("install");
    let edited = "theme = \"dark\"\n";
    write_file(&locations.config, edited);

    let report = run_at(true, &locations).expect("uninstall");
    assert_eq!(
        read_file(&locations.config),
        edited,
        "a hand-edited config is the user's now"
    );
    assert!(
        report.contains(&locations.config.display().to_string()),
        "report must name the kept file: {report}"
    );
}

#[test]
fn run_at_uninstall_without_files_reports_not_installed() {
    let dir = TempDir::new("absent");
    let locations = locations_for(dir.path());
    write_file(&locations.system_default, SYSTEM_DEFAULT);

    let report = run_at(true, &locations).expect("uninstall report");
    assert!(report.contains("nothing to uninstall"), "report: {report}");
    assert!(
        !locations.system_actions.exists(),
        "uninstall must not create a file"
    );
    assert!(
        !locations.custom.exists(),
        "uninstall must not create a file"
    );
}

#[test]
fn best_effort_without_needed_default_key_fails_loudly() {
    let dir = TempDir::new("missing-default");
    let locations = locations_for(dir.path());
    // A system default lacking WindowSwitcherPrevious: the restore plan
    // for an airlock-owned key cannot be built, so nothing may be written.
    let default_lacking_prev = concat!(
        "{\n",
        "    WindowSwitcher: \"cosmic-launcher alt-tab\",\n",
        "}\n",
    );
    write_file(&locations.system_default, default_lacking_prev);
    write_file(&locations.system_actions, USER_HAND_EDITED);

    match run_at(true, &locations) {
        Err(report) => assert!(
            report.contains(SWITCHER_PREV_KEY),
            "error must name the missing key: {report}"
        ),
        Ok(report) => panic!("expected Err, got Ok: {report}"),
    }
    assert!(
        !locations.custom.exists(),
        "no file may appear on a failed uninstall"
    );
    assert_eq!(
        read_file(&locations.system_actions),
        USER_HAND_EDITED,
        "the user file must be untouched when the plan fails"
    );
}

#[test]
fn install_places_both_assets_from_the_binary() {
    let dir = TempDir::new("assets");
    let locations = locations_for(dir.path());
    write_file(&locations.system_default, SYSTEM_DEFAULT);

    let report = run_at(false, &locations).expect("install");
    assert_eq!(fs::read(&locations.icon).expect("icon placed"), ICON_SVG);
    assert_eq!(
        fs::read(&locations.desktop).expect("desktop entry placed"),
        DESKTOP_ENTRY
    );
    assert!(
        report.contains("app assets"),
        "report must mention the assets: {report}"
    );
}

#[test]
fn reinstall_rewrites_the_same_asset_bytes() {
    let dir = TempDir::new("assets-idempotent");
    let locations = locations_for(dir.path());
    write_file(&locations.system_default, SYSTEM_DEFAULT);

    run_at(false, &locations).expect("first install");
    // Simulate a stray external edit: re-install must restore our bytes.
    write_file(&locations.desktop, "[Desktop Entry]\nName=stale\n");
    run_at(false, &locations).expect("re-install");
    assert_eq!(
        fs::read(&locations.desktop).expect("desktop entry placed"),
        DESKTOP_ENTRY,
        "re-install must rewrite the embedded bytes"
    );
}

#[test]
fn uninstall_removes_unmodified_assets() {
    let dir = TempDir::new("assets-undo");
    let locations = locations_for(dir.path());
    write_file(&locations.system_default, SYSTEM_DEFAULT);

    run_at(false, &locations).expect("install");
    run_at(true, &locations).expect("uninstall");
    assert!(!locations.icon.exists(), "icon must be removed");
    assert!(!locations.desktop.exists(), "desktop entry must be removed");
}

#[test]
fn uninstall_keeps_a_user_edited_asset_and_names_it() {
    let dir = TempDir::new("assets-edited");
    let locations = locations_for(dir.path());
    write_file(&locations.system_default, SYSTEM_DEFAULT);

    run_at(false, &locations).expect("install");
    let edited = "[Desktop Entry]\nType=Application\nName=Airlock (mine)\n";
    write_file(&locations.desktop, edited);

    let report = run_at(true, &locations).expect("uninstall");
    assert!(
        !locations.icon.exists(),
        "unmodified icon must still be removed"
    );
    assert_eq!(
        fs::read(&locations.desktop).expect("edited entry kept"),
        edited.as_bytes(),
        "user-edited asset must never be deleted"
    );
    assert!(
        report.contains(&locations.desktop.display().to_string()),
        "report must name the kept file: {report}"
    );
}

#[test]
fn uninstall_without_assets_still_restores_the_shortcut_files() {
    let dir = TempDir::new("assets-absent");
    let locations = locations_for(dir.path());
    write_file(&locations.system_default, SYSTEM_DEFAULT);
    write_file(&locations.system_actions, USER_EXISTING);

    run_at(false, &locations).expect("install");
    // The old-binary scenario: shortcuts installed, assets never
    // placed (or already removed by hand).
    fs::remove_file(&locations.icon).expect("icon absent");
    fs::remove_file(&locations.desktop).expect("desktop absent");

    let report = run_at(true, &locations).expect("uninstall report");
    assert_eq!(
        read_file(&locations.system_actions),
        USER_EXISTING,
        "shortcuts must still restore byte-exact with the assets absent"
    );
    assert!(
        report.contains("no airlock assets present"),
        "the assets line must name itself, not ride the shortcut files' \
         wording: {report}"
    );
    assert!(!locations.icon.exists(), "uninstall must not create a file");
    assert!(
        !locations.desktop.exists(),
        "uninstall must not create a file"
    );
}
