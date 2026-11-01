use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

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

/// Round-trip escape torture: backslashes in the command, a CR at line
/// end.
const USER_BACKSLASH_CR: &str = concat!(
    "{\n",
    "    WindowSwitcher: \"cmd\\\\with\\\\backslashes\",\r\n",
    "    WindowSwitcherPrevious: \"prev\",\n",
    "}\n",
);

/// A `custom` file as cosmic-settings writes it: one user Spawn shortcut,
/// multi-line binding keys, bare modifier names, trailing commas, no final
/// newline.
const CUSTOM_USER_FILE: &str = concat!(
    "{\n",
    "    (\n",
    "        modifiers: [\n",
    "            Super,\n",
    "        ],\n",
    "        key: Some(\"A\"),\n",
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
    "        key: Some(\"grave\"),\n",
    "    ): Spawn(\"my own thing\"),\n",
    "    (\n",
    "        modifiers: [\n",
    "            Super,\n",
    "        ],\n",
    "        key: Some(\"A\"),\n",
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
    "        key: Some(\"grave\"),\n",
    "    ): Spawn(\"airlock same-app\"),\n",
    "    (\n",
    "        modifiers: [\n",
    "            Super,\n",
    "        ],\n",
    "        key: Some(\"A\"),\n",
    "    ): Spawn(\"xeyes\"),\n",
    "}",
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
    let install = custom_install_content(None).expect("install");
    assert_eq!(
        install.in_place,
        ["Alt+grave", "Alt+Shift+grave", "Alt+Esc"]
    );
    assert!(install.skipped.is_empty(), "{:?}", install.skipped);
    assert!(
        install
            .content
            .starts_with(&format!("{CUSTOM_ABSENT_MARKER}\n{{\n")),
        "missing absent-file marker:\n{}",
        install.content
    );
    for command in [SAME_APP_CMD, SAME_APP_PREV_CMD, QUICK_TOGGLE_CMD] {
        assert!(
            install.content.contains(&format!("Spawn(\"{command}\")")),
            "{command} not bound:\n{}",
            install.content
        );
    }

    // Re-install over the fresh file is a byte-identical no-op.
    let again = custom_install_content(Some(&install.content)).expect("reinstall");
    assert_eq!(again.content, install.content);

    match custom_uninstall_content(&install.content).expect("uninstall") {
        CustomUninstall::RemoveFile => {}
        other => panic!("expected RemoveFile, got {other:?}"),
    }
}

#[test]
fn custom_install_preserves_user_entries_and_uninstall_restores_bytes() {
    let install = custom_install_content(Some(CUSTOM_USER_FILE)).expect("install");
    assert_eq!(install.in_place.len(), 3);
    assert!(
        install.content.contains("): Spawn(\"xeyes\"),\n"),
        "user entry lost:\n{}",
        install.content
    );

    match custom_uninstall_content(&install.content).expect("uninstall") {
        CustomUninstall::Restored { content } => assert_eq!(content, CUSTOM_USER_FILE),
        other => panic!("expected Restored, got {other:?}"),
    }
}

#[test]
fn custom_install_skips_user_held_binding_and_names_it() {
    let install = custom_install_content(Some(CUSTOM_HELD_GRAVE)).expect("install");
    assert_eq!(install.skipped, ["Alt+grave"]);
    assert_eq!(install.in_place, ["Alt+Shift+grave", "Alt+Esc"]);
    assert!(
        install.content.contains("Spawn(\"my own thing\")"),
        "held entry modified:\n{}",
        install.content
    );

    // The held entry survives the round-trip untouched.
    match custom_uninstall_content(&install.content).expect("uninstall") {
        CustomUninstall::Restored { content } => assert_eq!(content, CUSTOM_HELD_GRAVE),
        other => panic!("expected Restored, got {other:?}"),
    }
}

#[test]
fn custom_reinstall_is_byte_identical() {
    let once = custom_install_content(Some(CUSTOM_USER_FILE)).expect("install");
    let twice = custom_install_content(Some(&once.content)).expect("reinstall");
    assert_eq!(
        once.content, twice.content,
        "second install changed the file"
    );
}

#[test]
fn custom_install_after_cosmic_rewrite_refreshes_without_duplicates() {
    let install = custom_install_content(Some(CUSTOM_REWRITTEN)).expect("install");
    assert_eq!(install.in_place.len(), 3);
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
    match custom_uninstall_content(&install.content).expect("uninstall") {
        CustomUninstall::Restored { content } => assert_eq!(content, CUSTOM_USER_FILE),
        other => panic!("expected Restored, got {other:?}"),
    }
}

#[test]
fn custom_uninstall_without_airlock_entries_reports_not_installed() {
    match custom_uninstall_content(CUSTOM_USER_FILE).expect("uninstall") {
        CustomUninstall::NotInstalled => {}
        other => panic!("expected NotInstalled, got {other:?}"),
    }
}

#[test]
fn custom_disable_action_on_our_key_is_held_not_clobbered() {
    // Hand-written compact file: the user disabled Alt+Esc themselves.
    let held = "{\n    (modifiers: [Alt], key: Some(\"Esc\")): Disable,\n}";
    let install = custom_install_content(Some(held)).expect("install");
    assert_eq!(install.skipped, ["Alt+Esc"]);
    assert_eq!(install.in_place, ["Alt+grave", "Alt+Shift+grave"]);
    assert!(
        install.content.contains("Disable,"),
        "user's Disable entry modified:\n{}",
        install.content
    );
}

#[test]
fn custom_modifier_order_and_compact_forms_are_recognized() {
    // Reordered modifiers on a compact single-line entry: still a held
    // Alt+Shift+grave.
    let held = concat!(
        "{\n",
        "    (modifiers: [Shift, Alt], key: Some(\"grave\")): Spawn(\"other\"),\n",
        "}\n",
    );
    let install = custom_install_content(Some(held)).expect("install");
    assert_eq!(install.skipped, ["Alt+Shift+grave"]);
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
    let install = custom_install_content(Some(nasty)).expect("install");
    assert_eq!(install.in_place.len(), 3);
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
    let install = custom_install_content(Some("{}")).expect("install");
    assert_eq!(install.in_place.len(), 3);
    match custom_uninstall_content(&install.content).expect("uninstall") {
        CustomUninstall::Restored { content } => assert_eq!(content, "{}"),
        other => panic!("expected Restored, got {other:?}"),
    }
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
    Locations {
        system_actions: shortcuts.join("system_actions"),
        system_default: dir.join("usr-share/system_actions"),
        custom: shortcuts.join("custom"),
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
        installed_custom.contains(&format!("Spawn(\"{QUICK_TOGGLE_CMD}\")")),
        "quick-toggle not installed:\n{installed_custom}"
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

    run_at(true, &locations).expect("uninstall");
    assert!(
        !locations.system_actions.exists(),
        "absent-file restore must delete the file"
    );
    assert!(
        !locations.custom.exists(),
        "absent-file restore must delete the custom file"
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
