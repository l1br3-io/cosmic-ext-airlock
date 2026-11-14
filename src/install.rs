//! User-level shortcut takeover: `airlock install` points COSMIC's
//! `WindowSwitcher`/`WindowSwitcherPrevious` system actions at this binary
//! and claims four bindings in the sibling `custom` shortcuts file —
//! `Alt+grave`, `Alt+Shift+grave`, and the all-windows family under the
//! configured `all_windows_key` (`Alt+Escape` / `Alt+Shift+Escape` by
//! default) — spawning the same-app and all-windows stubs;
//! `airlock install --uninstall` puts the pre-install state back
//! byte-exactly. The 0.4-era `Alt+Esc` quick-toggle binding is retired:
//! a re-run of install migrates it to the new command (all-windows
//! A3.1, 2026-11-14); quick-toggle stays available as a CLI stub, bound
//! nowhere by default.
//!
//! Boundary (spec): the only files this module may ever write are the
//! user's `system_actions` and `custom` shortcuts files, exactly
//! two asset files under the user's data dir — the embedded app icon
//! and desktop entry — and the starter config under the user's config
//! dir, written only when no config exists yet. The pre-install
//! state is therefore recorded *inside* those files as leading
//! `// airlock-v1` comment markers — one per switcher key, plus one
//! `custom=absent-file` marker when install creates the `custom` file —
//! so no second file is needed and the backup can never be orphaned.
//! `/usr/share` is only ever read, as the source for first installs.
//! Unmarked user entries are never modified: a binding a user already
//! holds is skipped with a diagnostic naming the key and the way out.
//! Assets are removed on uninstall only while their bytes still match
//! what install wrote — a hand-edited file is the user's now, kept and
//! named in the report. The config is stricter still: install never
//! rewrites it (a file that exists is the user's, however it got
//! there), and uninstall removes it under the same bytes-still-match
//! rule as the assets.

use std::fs;
use std::path::{Path, PathBuf};

/// COSMIC's action key for Alt+Tab.
const SWITCHER_KEY: &str = "WindowSwitcher";
/// COSMIC's action key for Shift+Alt+Tab.
const SWITCHER_PREV_KEY: &str = "WindowSwitcherPrevious";
/// Command install binds to [`SWITCHER_KEY`].
const NEXT_CMD: &str = "airlock alt-tab";
/// Command install binds to [`SWITCHER_PREV_KEY`].
const PREV_CMD: &str = "airlock shift-alt-tab";
/// Command the Alt+grave custom binding spawns.
const SAME_APP_CMD: &str = "airlock same-app";
/// Command the Alt+Shift+grave custom binding spawns.
const SAME_APP_PREV_CMD: &str = "airlock same-app-previous";
/// Command the Alt+Esc custom binding spawns. Retired from the default
/// bindings (all-windows, 2026-11-14): the CLI stub remains, bound
/// nowhere by default — README documents binding it by hand.
const QUICK_TOGGLE_CMD: &str = "airlock quick-toggle";
/// Command the Alt+`all_windows_key` custom binding spawns.
const SHOW_ALL_CMD: &str = "airlock show-all-windows";
/// Command the Alt+Shift+`all_windows_key` custom binding spawns.
const SHOW_ALL_PREV_CMD: &str = "airlock show-all-windows-previous";
/// Prefix of every backup marker line install writes.
const MARKER_PREFIX: &str = "// airlock-v1 ";
/// Marker value when no user file existed before install.
const MARKER_ABSENT_FILE: &str = "absent-file";
/// Marker value when the file existed but had no entry for the key.
const MARKER_ABSENT_KEY: &str = "absent-key";
/// The app icon, embedded so a crates.io install needs no checkout —
/// `airlock install` is the complete setup.
const ICON_SVG: &[u8] =
    include_bytes!("../res/icons/hicolor/scalable/apps/io.github.l1br3-io.Airlock.svg");
/// The desktop entry, embedded alongside the icon.
const DESKTOP_ENTRY: &[u8] = include_bytes!("../res/io.github.l1br3-io.Airlock.desktop");
/// The commented starter config. Every setting line ships commented out,
/// so the file as written parses to the same defaults as no file at all;
/// it exists so a fresh install surfaces where settings live.
const DEFAULT_CONFIG: &[u8] = include_bytes!("../res/config.toml");

/// The files install works with.
pub struct Locations {
    /// User `system_actions` override file.
    pub system_actions: PathBuf,
    /// System default; read-only source for first installs and best-effort
    /// restores.
    pub system_default: PathBuf,
    /// User `custom` shortcuts file, sibling of [`Locations::system_actions`].
    pub custom: PathBuf,
    /// User-level app icon: [`ICON_SVG`]'s destination.
    pub icon: PathBuf,
    /// User-level desktop entry: [`DESKTOP_ENTRY`]'s destination.
    pub desktop: PathBuf,
    /// The starter config's path: [`DEFAULT_CONFIG`]'s destination, written
    /// only while absent.
    pub config: PathBuf,
}

/// What uninstall did to the user file.
#[derive(Debug)]
enum UninstallOutcome {
    /// The file now holds these bytes. `best_effort` marks a restore that had
    /// no backup markers and fell back to the system default commands.
    Restored { content: String, best_effort: bool },
    /// No user file existed before install; deleting it restores that state.
    RemoveFile,
    /// Nothing airlock-owned was found.
    NotInstalled,
}

/// The pre-install state of one switcher key, as recorded in a marker.
enum MarkerValue {
    /// No user file existed before install.
    AbsentFile,
    /// The file existed but had no entry for the key.
    AbsentKey,
    /// The entry's original line, byte-exact, newline excluded.
    Original(String),
}

/// Runs install/uninstall against the real user and system paths.
///
/// # Errors
///
/// Returns a human-readable diagnostic when paths can't be resolved, files
/// can't be read or written, or the user file has an unexpected shape.
pub fn run(uninstall: bool) -> Result<String, String> {
    run_at(uninstall, &default_locations()?)
}

/// Runs install/uninstall at explicit locations (tests point this at
/// tempdirs).
///
/// # Errors
///
/// Returns a human-readable diagnostic when files can't be read or written
/// or the user file has an unexpected shape.
pub fn run_at(uninstall: bool, locations: &Locations) -> Result<String, String> {
    let actions = manage_system_actions(uninstall, locations)?;
    let custom = manage_custom(uninstall, locations)?;
    let assets = manage_assets(uninstall, locations)?;
    let config = manage_config(uninstall, locations)?;
    Ok(format!("{actions}\n{custom}\n{assets}\n{config}"))
}

/// Installs or uninstalls the `system_actions` takeover at
/// [`Locations::system_actions`].
///
/// # Errors
///
/// Returns a human-readable diagnostic when the file can't be read or
/// written or has an unexpected shape.
fn manage_system_actions(uninstall: bool, locations: &Locations) -> Result<String, String> {
    if uninstall {
        let Some(user) = read_optional(&locations.system_actions)? else {
            return Ok(format!(
                "nothing to uninstall: {} does not exist",
                locations.system_actions.display()
            ));
        };
        let system_default = read_system_default(locations)?;
        return match uninstall_content(&user, &system_default)? {
            UninstallOutcome::Restored {
                content,
                best_effort,
            } => {
                write_atomic(&locations.system_actions, content.as_bytes())?;
                if best_effort {
                    Ok(format!(
                        "restored {} to the system default commands (no airlock backup \
                         was recorded in the file)",
                        locations.system_actions.display()
                    ))
                } else {
                    Ok(format!(
                        "restored {} to its pre-install state",
                        locations.system_actions.display()
                    ))
                }
            }
            UninstallOutcome::RemoveFile => {
                fs::remove_file(&locations.system_actions).map_err(|error| {
                    format!(
                        "cannot remove {}: {error}",
                        locations.system_actions.display()
                    )
                })?;
                Ok(format!(
                    "removed {}; the system default switcher is back",
                    locations.system_actions.display()
                ))
            }
            UninstallOutcome::NotInstalled => Ok(format!(
                "nothing to uninstall: no airlock takeover in {}",
                locations.system_actions.display()
            )),
        };
    }

    let system_default = read_system_default(locations)?;
    let user = read_optional(&locations.system_actions)?;
    let fresh = user.is_none();
    let content = install_content(user.as_deref(), &system_default)?;
    if let Some(dir) = locations.system_actions.parent() {
        fs::create_dir_all(dir)
            .map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
    }
    write_atomic(&locations.system_actions, content.as_bytes())?;
    let origin = if fresh {
        format!(
            "created {} from the system default",
            locations.system_actions.display()
        )
    } else {
        format!("updated {}", locations.system_actions.display())
    };
    Ok(format!(
        "installed: {SWITCHER_KEY} and {SWITCHER_PREV_KEY} now run airlock \
         ({origin}; undo with `airlock install --uninstall`)"
    ))
}

/// The `all_windows_key` the Esc-family bindings hold (all-windows A3.1):
/// the user's config value via [`crate::settings::Settings::parse`]. A
/// missing or malformed config never fails the install — the default key
/// stands — and a malformed one is flagged so the report says so:
/// settings can never break switching, and cannot break installing
/// either.
fn all_windows_key_at(locations: &Locations) -> (String, bool) {
    let default = || crate::settings::Settings::default().all_windows_key;
    match fs::read(&locations.config) {
        Ok(bytes) => match crate::settings::Settings::parse(&bytes) {
            Ok(settings) => (settings.all_windows_key, false),
            Err(_) => (default(), true),
        },
        Err(_) => (default(), false),
    }
}

/// Installs or uninstalls the custom bindings at [`Locations::custom`].
///
/// # Errors
///
/// Returns a human-readable diagnostic when the file can't be read or
/// written or has an unexpected shape.
fn manage_custom(uninstall: bool, locations: &Locations) -> Result<String, String> {
    let (all_windows_key, config_unparsed) = all_windows_key_at(locations);
    let current = read_optional(&locations.custom)?;
    if uninstall {
        let Some(current) = current else {
            return Ok(format!(
                "nothing to uninstall: {} does not exist",
                locations.custom.display()
            ));
        };
        return match custom_uninstall_content(&current, &all_windows_key)? {
            CustomUninstall::Restored { content } => {
                write_atomic(&locations.custom, content.as_bytes())?;
                Ok(format!(
                    "removed airlock's bindings from {}",
                    locations.custom.display()
                ))
            }
            CustomUninstall::RemoveFile => {
                fs::remove_file(&locations.custom).map_err(|error| {
                    format!("cannot remove {}: {error}", locations.custom.display())
                })?;
                Ok(format!(
                    "removed {}; install had created it",
                    locations.custom.display()
                ))
            }
            CustomUninstall::NotInstalled => Ok(format!(
                "nothing to uninstall: no airlock bindings in {}",
                locations.custom.display()
            )),
        };
    }

    let plan = custom_install_content(current.as_deref(), &all_windows_key)?;
    if let Some(dir) = locations.custom.parent() {
        fs::create_dir_all(dir)
            .map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
    }
    write_atomic(&locations.custom, plan.content.as_bytes())?;
    let mut report = if plan.in_place.is_empty() {
        format!(
            "custom shortcuts: none installed ({})",
            locations.custom.display()
        )
    } else {
        format!(
            "custom shortcuts: {} in place ({})",
            plan.in_place.join(", "),
            locations.custom.display()
        )
    };
    for key in &plan.skipped {
        report.push_str(&format!(
            "; skipped {key} — already bound to another command, free it in Settings → \
             Input → Keyboard → Shortcuts → Custom and re-run install"
        ));
    }
    for retired in &plan.migrated {
        report.push_str(&format!(
            "; migrated {retired} — quick-toggle retired, rebound to {SHOW_ALL_CMD}"
        ));
    }
    for stale in &plan.orphaned {
        report.push_str(&format!(
            "; removed stale {stale} — installed under a previous all_windows_key"
        ));
    }
    if config_unparsed {
        report.push_str(&format!(
            "; all_windows_key fell back to {all_windows_key} — the config did not parse"
        ));
    }
    if plan.skipped.is_empty() {
        report.push_str("; undo with `airlock install --uninstall`");
    }
    Ok(report)
}

/// Installs or uninstalls the embedded app assets at
/// [`Locations::icon`] and [`Locations::desktop`]. Install rewrites
/// them with the embedded bytes (the app-ID namespaced filenames are
/// airlock's own, like any package's owned files); uninstall removes
/// each only while its bytes still match — a hand-edited file is kept
/// and named in the report instead of deleted.
///
/// # Errors
///
/// Returns a human-readable diagnostic when the files can't be read,
/// written, or removed.
fn manage_assets(uninstall: bool, locations: &Locations) -> Result<String, String> {
    let assets = [
        (locations.icon.clone(), ICON_SVG, "icon"),
        (locations.desktop.clone(), DESKTOP_ENTRY, "desktop entry"),
    ];
    if uninstall {
        let mut report = Vec::new();
        for (path, embedded, what) in assets {
            match fs::read(&path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(format!("cannot read {}: {error}", path.display())),
                Ok(bytes) if bytes == embedded => {
                    fs::remove_file(&path)
                        .map_err(|error| format!("cannot remove {}: {error}", path.display()))?;
                    report.push(format!("removed {what} {}", path.display()));
                }
                Ok(_) => report.push(format!(
                    "kept {what} {} — it differs from what airlock installs \
                     (edited by hand?); remove it yourself if you want",
                    path.display()
                )),
            }
        }
        if report.is_empty() {
            return Ok("nothing to uninstall: no airlock assets present".to_string());
        }
        return Ok(report.join("\n"));
    }

    for (path, embedded, _) in assets {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)
                .map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
        }
        write_atomic(&path, embedded)?;
    }
    Ok(format!(
        "app assets: icon {} and desktop entry {} (user level)",
        locations.icon.display(),
        locations.desktop.display()
    ))
}

/// Places or removes the starter config at [`Locations::config`].
///
/// Unlike the assets, install never rewrites it: a config that exists is
/// the user's, however it got there, and is only named in the report.
/// Uninstall mirrors the asset rule — removed only while the bytes still
/// match [`DEFAULT_CONFIG`]; a hand-edited file is kept and named.
///
/// # Errors
///
/// Returns a human-readable diagnostic when the file can't be read,
/// written, or removed.
fn manage_config(uninstall: bool, locations: &Locations) -> Result<String, String> {
    if uninstall {
        return match fs::read(&locations.config) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok("nothing to uninstall: no starter config present".to_string())
            }
            Err(error) => Err(format!(
                "cannot read {}: {error}",
                locations.config.display()
            )),
            Ok(bytes) if bytes == DEFAULT_CONFIG => {
                fs::remove_file(&locations.config).map_err(|error| {
                    format!("cannot remove {}: {error}", locations.config.display())
                })?;
                Ok(format!(
                    "removed starter config {}",
                    locations.config.display()
                ))
            }
            Ok(_) => Ok(format!(
                "kept config {} — it differs from the starter airlock writes \
                 (edited by hand?); remove it yourself if you want",
                locations.config.display()
            )),
        };
    }
    match fs::metadata(&locations.config) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "cannot read {}: {error}",
                locations.config.display()
            ));
        }
        Ok(_) => {
            return Ok(format!(
                "starter config: {} left untouched (already present)",
                locations.config.display()
            ));
        }
    }
    if let Some(dir) = locations.config.parent() {
        fs::create_dir_all(dir)
            .map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
    }
    write_atomic(&locations.config, DEFAULT_CONFIG)?;
    Ok(format!(
        "starter config: wrote a commented {} — uncomment to customize; \
         deleting it entirely also means defaults",
        locations.config.display()
    ))
}

/// Resolves the real [`Locations`]: the XDG config and data base
/// directories, plus the fixed system default path.
///
/// # Errors
///
/// Returns a diagnostic when neither an absolute
/// `XDG_CONFIG_HOME`/`XDG_DATA_HOME` nor a usable `HOME` is set.
fn default_locations() -> Result<Locations, String> {
    let config = xdg_dir("XDG_CONFIG_HOME", ".config")?;
    let data = xdg_dir("XDG_DATA_HOME", ".local/share")?;
    Ok(Locations {
        system_actions: config
            .join("cosmic/com.system76.CosmicSettings.Shortcuts/v1/system_actions"),
        system_default: PathBuf::from(
            "/usr/share/cosmic/com.system76.CosmicSettings.Shortcuts/v1/system_actions",
        ),
        custom: config.join("cosmic/com.system76.CosmicSettings.Shortcuts/v1/custom"),
        icon: data.join("icons/hicolor/scalable/apps/io.github.l1br3-io.Airlock.svg"),
        desktop: data.join("applications/io.github.l1br3-io.Airlock.desktop"),
        config: config.join("airlock/config.toml"),
    })
}

/// Resolves an XDG base directory: `$XDG_<VAR>` when absolute, else
/// `$HOME/<fallback>`. An empty or relative value is ignored per the
/// XDG spec.
///
/// # Errors
///
/// Returns a diagnostic when neither an absolute `$XDG_<VAR>` nor a
/// usable `HOME` is set.
fn xdg_dir(var: &str, fallback: &str) -> Result<PathBuf, String> {
    Ok(match std::env::var_os(var) {
        Some(dir) if !dir.is_empty() && Path::new(&dir).is_absolute() => PathBuf::from(dir),
        _ => {
            let home = std::env::var_os("HOME")
                .filter(|home| !home.is_empty())
                .ok_or_else(|| format!("cannot resolve {var}: no absolute ${var} and no HOME"))?;
            PathBuf::from(home).join(fallback)
        }
    })
}

/// Reads a file, mapping "not found" to `None`.
///
/// # Errors
///
/// Propagates any other I/O error, including non-UTF-8 content — only files
/// that round-trip as text are ever rewritten.
fn read_optional(path: &Path) -> Result<Option<String>, String> {
    match fs::read_to_string(path) {
        Ok(content) => Ok(Some(content)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("cannot read {}: {error}", path.display())),
    }
}

/// Reads the system default; `/usr/share` is only ever read, never written.
///
/// # Errors
///
/// Returns a diagnostic when the file is missing or unreadable.
fn read_system_default(locations: &Locations) -> Result<String, String> {
    fs::read_to_string(&locations.system_default).map_err(|error| {
        format!(
            "cannot read the system default at {}: {error} — is this a COSMIC system?",
            locations.system_default.display()
        )
    })
}

/// Produces the installed user-file content.
///
/// `user` is `None` when no user file exists yet; the file is then seeded
/// from `system_default`. Existing unrelated lines — including their exact
/// spacing and comments — are preserved byte-for-byte; only the two
/// switcher lines change (or get appended before the closing brace). Any
/// existing airlock markers are kept as-is, which makes a second install a
/// byte-identical no-op.
///
/// # Errors
///
/// Returns a diagnostic when a switcher key is missing and the file has no
/// closing `}` to append it to (empty or malformed content is left alone).
fn install_content(user: Option<&str>, system_default: &str) -> Result<String, String> {
    let (body, markers) = match user {
        Some(content) => {
            let lines = split_lines(content);
            match parse_markers(&lines)? {
                // Markers from a previous install stay authoritative, so a
                // second install can never overwrite the first backup.
                Some(found) => (lines[2..].concat(), found),
                None => (content.to_string(), original_markers(&lines)),
            }
        }
        None => (
            system_default.to_string(),
            (MarkerValue::AbsentFile, MarkerValue::AbsentFile),
        ),
    };
    let installed = apply_override(&body)?;
    Ok(format!(
        "{}{}{}",
        marker_line(SWITCHER_KEY, &markers.0),
        marker_line(SWITCHER_PREV_KEY, &markers.1),
        installed
    ))
}

/// Fresh markers for a file airlock has never touched: the current line per
/// key, or absent-key when the file carries no entry for it.
fn original_markers(lines: &[&str]) -> (MarkerValue, MarkerValue) {
    (
        original_marker_value(lines, SWITCHER_KEY),
        original_marker_value(lines, SWITCHER_PREV_KEY),
    )
}

fn original_marker_value(lines: &[&str], key: &str) -> MarkerValue {
    match key_line_index(lines, key) {
        Some(index) => MarkerValue::Original(lines[index].trim_end_matches('\n').to_string()),
        None => MarkerValue::AbsentKey,
    }
}

/// Rewrites the two switcher entries in `body`, preserving every other byte.
/// Missing entries are appended before the closing brace.
///
/// # Errors
///
/// Returns a diagnostic when an entry is missing and the file has no closing
/// `}` to append it to.
fn apply_override(body: &str) -> Result<String, String> {
    let mut lines: Vec<String> = body.split_inclusive('\n').map(str::to_string).collect();
    for (key, command) in [(SWITCHER_KEY, NEXT_CMD), (SWITCHER_PREV_KEY, PREV_CMD)] {
        if let Some(index) = key_line_index(&lines, key) {
            let newline = if lines[index].ends_with('\n') {
                "\n"
            } else {
                ""
            };
            let indent = indent_of(&lines[index]).to_string();
            lines[index] = format!("{indent}{}{newline}", override_line(key, command));
        } else {
            let close = closing_brace_index(&lines).ok_or_else(|| {
                format!(
                    "{key} is missing and the file has no closing brace to append it to; \
                     the file was left untouched"
                )
            })?;
            let indent = format!("{}    ", indent_of(&lines[close]));
            lines.insert(close, format!("{indent}{}\n", override_line(key, command)));
        }
    }
    Ok(lines.concat())
}

/// Reverts an installed user file.
///
/// # Errors
///
/// Returns a diagnostic when a best-effort restore is needed but the system
/// default lacks the entry, or a marker line is corrupt.
fn uninstall_content(user: &str, system_default: &str) -> Result<UninstallOutcome, String> {
    let lines = split_lines(user);
    let Some((switcher, previous)) = parse_markers(&lines)? else {
        return restore_without_markers(&lines, system_default);
    };
    if matches!(switcher, MarkerValue::AbsentFile) || matches!(previous, MarkerValue::AbsentFile) {
        return Ok(UninstallOutcome::RemoveFile);
    }
    let mut body: Vec<String> = lines[2..].iter().map(|line| line.to_string()).collect();
    for (key, marker) in [(SWITCHER_KEY, switcher), (SWITCHER_PREV_KEY, previous)] {
        let Some(index) = key_line_index(&body, key) else {
            // The entry was deleted while airlock was installed; a missing
            // entry stays missing.
            continue;
        };
        match marker {
            MarkerValue::Original(line) => {
                let newline = if body[index].ends_with('\n') {
                    "\n"
                } else {
                    ""
                };
                body[index] = format!("{line}{newline}");
            }
            MarkerValue::AbsentKey => {
                body.remove(index);
            }
            MarkerValue::AbsentFile => unreachable!("handled above"),
        }
    }
    Ok(UninstallOutcome::Restored {
        content: body.concat(),
        best_effort: false,
    })
}

/// Markerless file: either airlock owns nothing here, or the takeover was
/// written by hand (or a cosmic-settings rewrite stripped the markers) and
/// only the system default can put the built-in switcher back. Only entries
/// actually running an airlock command are rewritten; any other entry — a
/// third-party switcher on the sibling key — survives byte-exact.
fn restore_without_markers(
    lines: &[&str],
    system_default: &str,
) -> Result<UninstallOutcome, String> {
    let defaults = split_lines(system_default);
    let mut restored: Vec<String> = lines.iter().map(|line| line.to_string()).collect();
    let mut airlock_owned = false;
    for (key, command) in [(SWITCHER_KEY, NEXT_CMD), (SWITCHER_PREV_KEY, PREV_CMD)] {
        let Some(index) = key_line_index(lines, key) else {
            continue;
        };
        if !lines[index].contains(command) {
            // Someone else's entry: never a byte of it moves.
            continue;
        }
        airlock_owned = true;
        let Some(default) = key_line_index(&defaults, key) else {
            return Err(format!(
                "the system default has no {key} entry to restore; edit the file manually"
            ));
        };
        let newline = if restored[index].ends_with('\n') {
            "\n"
        } else {
            ""
        };
        restored[index] = format!("{}{newline}", defaults[default].trim_end_matches('\n'));
    }
    if !airlock_owned {
        return Ok(UninstallOutcome::NotInstalled);
    }
    Ok(UninstallOutcome::Restored {
        content: restored.concat(),
        best_effort: true,
    })
}

/// Parses the two leading marker lines, if the file starts with them.
///
/// # Errors
///
/// Returns a diagnostic for a malformed, unknown, duplicated, or incomplete
/// marker block.
fn parse_markers(lines: &[&str]) -> Result<Option<(MarkerValue, MarkerValue)>, String> {
    let mut switcher = None;
    let mut previous = None;
    let mut count = 0;
    for line in lines {
        let Some(rest) = line.strip_prefix(MARKER_PREFIX) else {
            break;
        };
        // Strip the line terminator, never value content: original lines may
        // end in spaces that a restore must reproduce byte-exactly.
        let rest = rest.strip_suffix('\n').unwrap_or(rest);
        let rest = rest.strip_suffix('\r').unwrap_or(rest);
        let (key, value) = rest
            .split_once('=')
            .ok_or_else(|| format!("corrupt airlock marker: {rest}"))?;
        let parsed = match value {
            MARKER_ABSENT_FILE => MarkerValue::AbsentFile,
            MARKER_ABSENT_KEY => MarkerValue::AbsentKey,
            "" => return Err(format!("empty airlock marker value: {rest}")),
            raw => MarkerValue::Original(decode_marker_value(raw)?),
        };
        match key {
            SWITCHER_KEY if switcher.is_none() => switcher = Some(parsed),
            SWITCHER_PREV_KEY if previous.is_none() => previous = Some(parsed),
            _ => return Err(format!("corrupt airlock marker: {rest}")),
        }
        count += 1;
    }
    if count == 0 {
        return Ok(None);
    }
    match (switcher, previous) {
        (Some(switcher), Some(previous)) => Ok(Some((switcher, previous))),
        _ => Err("incomplete airlock marker block: expected two marker lines".to_string()),
    }
}

/// One rendered marker line, newline included.
fn marker_line(key: &str, value: &MarkerValue) -> String {
    let value = match value {
        MarkerValue::AbsentFile => MARKER_ABSENT_FILE.to_string(),
        MarkerValue::AbsentKey => MARKER_ABSENT_KEY.to_string(),
        MarkerValue::Original(line) => encode_marker_value(line),
    };
    format!("{MARKER_PREFIX}{key}={value}\n")
}

/// Splits `content` into lines that keep their trailing newline, so a join
/// reproduces the original bytes exactly.
fn split_lines(content: &str) -> Vec<&str> {
    content.split_inclusive('\n').collect()
}

/// Index of the last line whose only content is the closing `}`.
fn closing_brace_index(lines: &[String]) -> Option<usize> {
    lines.iter().rposition(|line| line.trim() == "}")
}

/// Index of the line assigning `key` (`Key: "value",`), if present. A line
/// for `WindowSwitcher` never matches `WindowSwitcherPrevious`: after the
/// key text only whitespace may precede the colon.
fn key_line_index<L: AsRef<str>>(lines: &[L], key: &str) -> Option<usize> {
    lines.iter().position(|line| {
        let trimmed = line.as_ref().trim_start();
        trimmed
            .strip_prefix(key)
            .is_some_and(|rest| rest.trim_start_matches([' ', '\t']).starts_with(':'))
    })
}

/// Airlock's replacement entry line for `key` (no indent, no newline).
fn override_line(key: &str, command: &str) -> String {
    format!("{key}: \"{command}\",")
}

/// Leading whitespace of `line`.
fn indent_of(line: &str) -> &str {
    &line[..line.len() - line.trim_start().len()]
}

/// Encodes a raw original line for a marker value: backslashes double, CR
/// becomes `\r` (newlines cannot occur inside a line).
fn encode_marker_value(raw_line: &str) -> String {
    raw_line.replace('\\', "\\\\").replace('\r', "\\r")
}

/// Inverse of [`encode_marker_value`].
fn decode_marker_value(value: &str) -> Result<String, String> {
    let mut decoded = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            decoded.push(c);
            continue;
        }
        match chars.next() {
            Some('\\') => decoded.push('\\'),
            Some('r') => decoded.push('\r'),
            other => {
                return Err(format!("corrupt escape in airlock marker: \\{other:?}"));
            }
        }
    }
    Ok(decoded)
}

/// Writes `content` to `path` atomically: a dot-prefixed temporary in the
/// same directory is written, synced, then renamed over the target, so a
/// crash mid-write never leaves a truncated config. No temporary file
/// survives a completed call.
///
/// # Errors
///
/// Propages I/O errors, cleaning up the temporary best-effort.
fn write_atomic(path: &Path, content: &[u8]) -> Result<(), String> {
    let Some(dir) = path.parent() else {
        return Err(format!("no directory to write {} in", path.display()));
    };
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "airlock".to_string());
    let temp = dir.join(format!(".{name}.tmp"));
    let written = (|| -> Result<(), std::io::Error> {
        use std::io::Write as _;
        let mut file = fs::File::create(&temp)?;
        file.write_all(content)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)
    })();
    if let Err(error) = written {
        let _ = fs::remove_file(&temp);
        return Err(format!("cannot write {}: {error}", path.display()));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// `custom` shortcuts file: the same-app / quick-toggle bindings.
//
// COSMIC binds actions it does not define only through custom shortcuts —
// a RON map from `Binding` keys to `Action` values in the sibling `custom`
// file of the shortcuts v1 directory. cosmic-config writes it with RON's
// default pretty printer: multi-line binding keys (`modifiers: [\n Alt,\n]`,
// bare variant names), inline values (`Spawn("...")`), trailing commas, no
// final newline, entries in HashMap order. Two schema facts are load-
// bearing (0.1.0 got the first wrong and reset users' live shortcuts):
//
// * `key` is a **bare string** — `key: "grave"`. cosmic's keysym
//   deserializer (cosmic-settings-daemon `config/src/shortcuts/sym.rs`)
//   rejects `key: Some("grave")`, and because a Binding is the map key,
//   one bad entry fails the whole file's load: cosmic-comp then falls
//   back to factory defaults *in memory* — every user shortcut appears
//   reset while their bytes sit untouched on disk. Recognition below
//   still understands 0.1.0's `Some(..)` form so those installs can be
//   uninstalled or healed by a re-run.
// * cosmic-comp matches a press against the level-0 keysym of the
//   physical key (layout- and shift-level-agnostic), so `Alt+Shift+grave`
//   binds `key: "grave"` — and keysyms carry their canonical X names:
//   Escape, never "Esc".
//
// cosmic's re-serialization strips comment lines — so airlock's marker can
// vanish while its entries live on; recognition below is therefore semantic
// (binding + spawned command), never byte- or marker-only.

/// One custom binding airlock claims in the `custom` shortcuts file.
struct CustomBinding {
    /// Human-readable binding name, for diagnostics.
    display: String,
    /// Modifier names in the order COSMIC serializes them.
    modifiers: &'static [&'static str],
    /// Keysym name of the key (level-0, per cosmic-comp matching), as a
    /// bare string in the emitted file — canonical X names ("Escape",
    /// "grave"), never abbreviations.
    key: String,
    /// Keysym names earlier releases emitted for this binding (empty when
    /// none). 0.1.0 bound the quick toggle as `"Esc"` — not a keysym —
    /// so its entries must be matched by the legacy name to uninstall or
    /// heal a 0.1.0 machine.
    legacy_keys: Vec<&'static str>,
    /// Command the binding spawns.
    command: &'static str,
    /// Description cosmic-settings shows for the entry.
    description: String,
}

/// The bindings an install claims: the static grave pair plus the
/// all-windows family under `all_windows_key` (all-windows A3.1,
/// 2026-11-14). Built per run because the key is the user's config
/// value; the legacy `"Esc"` name counts only while the configured key
/// is the canonical `"Escape"` it once mis-serialized as.
fn custom_bindings(all_windows_key: &str) -> Vec<CustomBinding> {
    let esc_legacy = if all_windows_key == "Escape" {
        vec!["Esc"]
    } else {
        Vec::new()
    };
    vec![
        CustomBinding {
            display: "Alt+grave".to_owned(),
            modifiers: &["Alt"],
            key: "grave".to_owned(),
            legacy_keys: Vec::new(),
            command: SAME_APP_CMD,
            description: "Airlock: same app windows".to_owned(),
        },
        CustomBinding {
            display: "Alt+Shift+grave".to_owned(),
            modifiers: &["Alt", "Shift"],
            key: "grave".to_owned(),
            legacy_keys: Vec::new(),
            command: SAME_APP_PREV_CMD,
            description: "Airlock: same app windows, backwards".to_owned(),
        },
        CustomBinding {
            display: format!("Alt+{all_windows_key}"),
            modifiers: &["Alt"],
            key: all_windows_key.to_owned(),
            legacy_keys: esc_legacy,
            command: SHOW_ALL_CMD,
            description: "Airlock: every workspace's windows".to_owned(),
        },
        CustomBinding {
            display: format!("Alt+Shift+{all_windows_key}"),
            modifiers: &["Alt", "Shift"],
            key: all_windows_key.to_owned(),
            legacy_keys: Vec::new(),
            command: SHOW_ALL_PREV_CMD,
            description: "Airlock: every workspace's windows, backwards".to_owned(),
        },
    ]
}

/// A binding an earlier release installed that this one no longer does
/// (all-windows A3.1, 2026-11-14): the 0.4-era `Alt+Esc` quick-toggle,
/// retired when the Esc family moved to `show-all-windows`. Under
/// today's ownership rules a leftover entry would look user-held — it
/// holds the new binding's key while spawning a different command — so
/// it is recognized here instead: an entry matching a retired binding's
/// key+modifiers *and* spawning its command is airlock-owned, removed
/// by the rebuild and reported as a migration, never a skip.
struct RetiredBinding {
    /// Human-readable binding name, for the migration report.
    display: &'static str,
    /// Modifier names of the retired binding.
    modifiers: &'static [&'static str],
    /// Keysym names the retired entries carry — canonical and legacy
    /// alike, matching the existing legacy-key mechanism.
    keys: &'static [&'static str],
    /// Command the retired binding spawned.
    command: &'static str,
}

const RETIRED_BINDINGS: &[RetiredBinding] = &[RetiredBinding {
    display: "Alt+Esc",
    modifiers: &["Alt"],
    keys: &["Escape", "Esc"],
    command: QUICK_TOGGLE_CMD,
}];

/// Marker line install records when it creates the `custom` file, so
/// uninstall can delete it again. Stripped by any cosmic-settings rewrite;
/// that only degrades uninstall to restoring an empty map.
const CUSTOM_ABSENT_MARKER: &str = "// airlock-v1 custom=absent-file";

/// A planned `custom`-file install.
struct CustomInstall {
    /// The file content to write.
    content: String,
    /// Bindings in place after the install (added or refreshed), by display
    /// name.
    in_place: Vec<String>,
    /// Bindings skipped because a user entry already holds the key.
    skipped: Vec<String>,
    /// Retired bindings whose stale entries this install removed and
    /// rebound (all-windows A3.1): quick-toggle entries on the Esc key.
    migrated: Vec<&'static str>,
    /// Stale entries a previous `all_windows_key` installed, removed by
    /// this install and named in the report (the ship-review follow-up):
    /// the family an earlier run wrote under a different key.
    orphaned: Vec<String>,
}

/// What uninstall did to the `custom` file.
#[derive(Debug)]
enum CustomUninstall {
    /// The file now holds these bytes.
    Restored { content: String },
    /// Install created the file; deleting it restores that state.
    RemoveFile,
    /// Nothing airlock-owned was found.
    NotInstalled,
}

/// Airlock's canonical one-line serialization of one binding. The keysym
/// is a bare string — cosmic's `Binding.key` deserializer rejects
/// `Some(..)`, and a malformed map key makes cosmic-comp discard the whole
/// `custom` file and run factory defaults (the 0.1.0 incident).
fn custom_entry_line(binding: &CustomBinding) -> String {
    format!(
        "    (modifiers: [{}], key: \"{}\", description: Some(\"{}\")): \
         Spawn(\"{}\"),\n",
        binding.modifiers.join(", "),
        binding.key,
        binding.description,
        binding.command,
    )
}

/// One map entry parsed from a `custom` shortcuts file.
struct CustomEntry {
    /// Lines the entry occupies, as `[start, end)` indices into the file's
    /// lines.
    span: (usize, usize),
    /// Lowercased modifier names from the binding's `modifiers` list.
    modifiers: Vec<String>,
    /// Keysym name from `key: "…"` (or 0.1.0's `key: Some("…")`); `None`
    /// when absent or `None`.
    key: Option<String>,
    /// Action text after the binding's closing `): `, trailing comma
    /// stripped (e.g. `Spawn("xeyes")`).
    action: String,
}

impl CustomEntry {
    /// Whether this entry binds exactly `binding`'s modifier set and key —
    /// order and quoting differences aside. A legacy keysym name counts as
    /// the same key, so 0.1.0-era entries are still recognized as airlock's.
    fn holds(&self, binding: &CustomBinding) -> bool {
        self.key
            .as_deref()
            .is_some_and(|key| key == binding.key || binding.legacy_keys.contains(&key))
            && self.modifiers.len() == binding.modifiers.len()
            && binding.modifiers.iter().all(|modifier| {
                self.modifiers
                    .iter()
                    .any(|owned| owned.eq_ignore_ascii_case(modifier))
            })
    }

    /// Whether this entry binds a retired binding's modifier set and key
    /// — the twin of [`CustomEntry::holds`] over [`RetiredBinding`]'s
    /// key list, legacy names included.
    fn holds_retired(&self, retired: &RetiredBinding) -> bool {
        self.key
            .as_deref()
            .is_some_and(|key| retired.keys.contains(&key))
            && self.modifiers.len() == retired.modifiers.len()
            && retired.modifiers.iter().all(|modifier| {
                self.modifiers
                    .iter()
                    .any(|owned| owned.eq_ignore_ascii_case(modifier))
            })
    }

    /// Whether this entry is a stale retired binding airlock once wrote —
    /// the key held and the retired command spawned.
    fn is_retired_ours(&self) -> bool {
        RETIRED_BINDINGS
            .iter()
            .any(|retired| self.holds_retired(retired) && self.spawns(retired.command))
    }

    /// Whether this entry is an airlock-written orphan of a previous
    /// `all_windows_key` (the ship-review follow-up to all-windows A3.1):
    /// it spawns one of the two commands whose key the setting names but
    /// holds none of today's bindings — the fingerprint of an install
    /// made under a different key. Claimed as airlock's own: dropped by
    /// the rebuild, named in the report, removed by uninstall. Scoped to
    /// the show-all-windows commands, the only ones whose key the setting
    /// moves — a hand-bound `airlock quick-toggle` on some other key
    /// (README documents the recipe) survives untouched.
    fn is_orphaned_ours(&self, bindings: &[CustomBinding]) -> bool {
        (self.spawns(SHOW_ALL_CMD) || self.spawns(SHOW_ALL_PREV_CMD))
            && !bindings.iter().any(|binding| self.holds(binding))
    }

    /// The entry's own display name — "Alt+Shift+Escape", modifiers
    /// capitalized, key verbatim — for the orphan report.
    fn display(&self) -> String {
        let mut name = self
            .modifiers
            .iter()
            .map(|modifier| capitalized(modifier))
            .collect::<Vec<_>>()
            .join("+");
        if let Some(key) = &self.key {
            if !name.is_empty() {
                name.push('+');
            }
            name.push_str(key);
        }
        name
    }

    /// Whether this entry spawns `command` — airlock's own entry, however
    /// it got serialized.
    fn spawns(&self, command: &str) -> bool {
        self.action
            .find("Spawn")
            .and_then(|start| quoted_string(&self.action[start..]))
            .is_some_and(|spawned| spawned == command)
    }
}

/// Parses the map entries of a `custom` shortcuts file. Both shapes that
/// occur in practice are recognized — cosmic-config's multi-line blocks and
/// airlock's canonical one-liners — by tracking paren depth: an entry starts
/// at a line whose first non-space character is `(` and ends where the key's
/// depth returns to zero right before a `:` on the same line (a `):` inside
/// a string sits at inner depth and cannot end the key). Entries share no
/// lines with anything else, so anything unrecognized stays unowned and is
/// byte-preserved by the rebuilds.
fn parse_custom_entries(lines: &[&str]) -> Vec<CustomEntry> {
    let mut entries = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        if !lines[index].trim_start().starts_with('(') {
            index += 1;
            continue;
        }
        let start = index;
        let mut depth: i64 = 0;
        let mut complete: Option<String> = None;
        while complete.is_none() && index < lines.len() {
            let line = lines[index];
            index += 1;
            for (offset, character) in line.char_indices() {
                match character {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            if let Some(value) = line[offset + 1..].trim_start().strip_prefix(':') {
                                complete =
                                    Some(value.trim().trim_end_matches(',').trim().to_string());
                            }
                            break;
                        }
                    }
                    _ => {}
                }
            }
            if complete.is_none() && depth <= 0 {
                // The block closed without a `):` — not a binding entry we
                // understand; leave it unowned.
                break;
            }
        }
        if let Some(action) = complete {
            let text: String = lines[start..index].concat();
            entries.push(CustomEntry {
                span: (start, index),
                modifiers: modifiers_of(&text),
                key: key_of(&text),
                action,
            });
        }
    }
    entries
}

/// Lowercased modifier names inside the first `modifiers: [ … ]` list.
fn modifiers_of(text: &str) -> Vec<String> {
    let open = text
        .find("modifiers")
        .and_then(|m| text[m..].find('[').map(|relative| m + relative));
    let Some(open) = open else {
        return Vec::new();
    };
    let Some(close) = text[open..].find(']').map(|relative| open + relative) else {
        return Vec::new();
    };
    text[open + 1..close]
        .split([',', ' ', '\t', '\n', '\r'])
        .filter(|token| !token.is_empty())
        .map(|token| token.trim_matches('"').to_ascii_lowercase())
        .collect()
}

/// The keysym string inside `key: "…"` — or `key: Some("…")`, the form
/// 0.1.0 emitted; both must be recognized so legacy installs uninstall
/// cleanly. `keycode:` never matches because its `key` is followed by `c`,
/// not `:`.
fn key_of(text: &str) -> Option<String> {
    let colon = text.find("key:")?;
    let rest = text[colon + 4..].trim_start();
    let rest = rest.strip_prefix("Some(").unwrap_or(rest);
    quoted_string(rest)
}

/// The first `"`-quoted string in `text`, if any.
fn quoted_string(text: &str) -> Option<String> {
    let start = text.find('"')? + 1;
    let end = text[start..].find('"')? + start;
    Some(text[start..end].to_string())
}

/// ` "alt"` → `"Alt"`, for report display names built from parsed
/// entries (modifiers are stored lowercased).
fn capitalized(word: &str) -> String {
    let mut characters = word.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().collect::<String>() + characters.as_str(),
        None => word.to_owned(),
    }
}

/// Index of the first line opening the map.
fn opening_brace_index(lines: &[&str]) -> Option<usize> {
    lines
        .iter()
        .position(|line| line.trim_start().starts_with('{'))
}

/// Whether an entry is airlock's own for some binding — a current one, a
/// retired one, or an orphan a previous `all_windows_key` left behind —
/// so stale entries never survive a rebuild or an uninstall.
fn entry_is_ours(entry: &CustomEntry, bindings: &[CustomBinding]) -> bool {
    bindings
        .iter()
        .any(|binding| entry.holds(binding) && entry.spawns(binding.command))
        || entry.is_retired_ours()
        || entry.is_orphaned_ours(bindings)
}

/// Plans the installed `custom`-file content over `current` (`None` when no
/// file exists). Airlock's entries are written in one canonical line each,
/// inserted right after the opening brace; every other byte of the file is
/// preserved. A binding a user entry already holds is never taken: it is
/// reported skipped. A stale retired entry on a new binding's key is
/// airlock's own — dropped by the rebuild and reported as a migration,
/// never a skip (all-windows A3.1).
///
/// # Errors
///
/// Returns a diagnostic when the file has no opening brace to insert after;
/// it is then left untouched.
fn custom_install_content(
    current: Option<&str>,
    all_windows_key: &str,
) -> Result<CustomInstall, String> {
    let bindings = custom_bindings(all_windows_key);
    let Some(current) = current else {
        let mut content = format!("{CUSTOM_ABSENT_MARKER}\n{{\n");
        for binding in &bindings {
            content.push_str(&custom_entry_line(binding));
        }
        content.push('}');
        return Ok(CustomInstall {
            content,
            in_place: bindings.iter().map(|b| b.display.clone()).collect(),
            skipped: Vec::new(),
            migrated: Vec::new(),
            orphaned: Vec::new(),
        });
    };

    let lines = split_lines(current);
    let brace = opening_brace_index(&lines).ok_or_else(|| {
        "the custom shortcuts file has no opening brace to insert into; it was left \
             untouched"
            .to_string()
    })?;
    let entries = parse_custom_entries(&lines);

    let mut in_place = Vec::new();
    let mut skipped = Vec::new();
    for binding in &bindings {
        match entries.iter().find(|entry| entry.holds(binding)) {
            // A previous airlock install (possibly re-serialized by
            // cosmic-settings): refreshed at the canonical position.
            Some(entry) if entry.spawns(binding.command) => in_place.push(binding.display.clone()),
            // A stale retired entry (the 0.4 quick-toggle) holds the key:
            // the rebuild drops it and the new binding lands in its place.
            Some(entry) if entry.is_retired_ours() => in_place.push(binding.display.clone()),
            // A user entry holds the key; it is never touched.
            Some(_) => skipped.push(binding.display.clone()),
            None => in_place.push(binding.display.clone()),
        }
    }
    let migrated: Vec<&'static str> = RETIRED_BINDINGS
        .iter()
        .filter(|retired| {
            entries
                .iter()
                .any(|entry| entry.holds_retired(retired) && entry.spawns(retired.command))
        })
        .map(|retired| retired.display)
        .collect();
    // Stale families from a previous key, named by their own entries:
    // the display is the old binding's name, not today's.
    let orphaned: Vec<String> = entries
        .iter()
        .filter(|entry| entry.is_orphaned_ours(&bindings))
        .map(|entry| entry.display())
        .collect();

    // A one-line empty map (`{}`) is split so the entries have a place to
    // go; every other shape keeps its own brace line verbatim.
    let empty_map = lines[brace].trim() == "{}";
    let mut content = String::new();
    for line in &lines[..brace] {
        content.push_str(line);
    }
    content.push_str(if empty_map { "{\n" } else { lines[brace] });
    for binding in &bindings {
        if in_place.contains(&binding.display) {
            content.push_str(&custom_entry_line(binding));
        }
    }
    for (index, line) in lines.iter().enumerate().skip(brace + 1) {
        let owned = entries.iter().any(|entry| {
            entry_is_ours(entry, &bindings) && index >= entry.span.0 && index < entry.span.1
        });
        if !owned {
            content.push_str(line);
        }
    }
    if empty_map {
        content.push('}');
    }
    Ok(CustomInstall {
        content,
        in_place,
        skipped,
        migrated,
        orphaned,
    })
}

/// Plans the uninstall of airlock's `custom`-file entries: removed
/// semantically (a cosmic-settings rewrite may have re-serialized them), the
/// remaining bytes preserved. An absent-file marker plus no surviving user
/// entries means install created the file and deleting it restores the
/// pre-install state.
///
/// # Errors
///
/// Returns a diagnostic for content this module refuses to touch.
fn custom_uninstall_content(
    current: &str,
    all_windows_key: &str,
) -> Result<CustomUninstall, String> {
    let bindings = custom_bindings(all_windows_key);
    let lines = split_lines(current);
    let entries = parse_custom_entries(&lines);
    let marker_line = lines
        .iter()
        .position(|line| line.trim_end_matches(['\n', '\r']) == CUSTOM_ABSENT_MARKER);
    let ours: Vec<&CustomEntry> = entries
        .iter()
        .filter(|entry| entry_is_ours(entry, &bindings))
        .collect();

    if ours.is_empty() && marker_line.is_none() {
        return Ok(CustomUninstall::NotInstalled);
    }
    // Entries a user (or cosmic-settings on their behalf) still owns.
    let user_remains = entries.len() > ours.len();
    if marker_line.is_some() && !user_remains {
        return Ok(CustomUninstall::RemoveFile);
    }
    if !user_remains {
        // Only airlock ever wrote here; the canonical empty map is the
        // closest restore (a cosmic-settings rewrite may have reshaped the
        // file since install, so the original bytes are unrecoverable).
        let mut content = String::new();
        if let Some(brace) = opening_brace_index(&lines) {
            for (index, line) in lines[..brace].iter().enumerate() {
                if Some(index) != marker_line {
                    content.push_str(line);
                }
            }
            content.push_str("{}");
            return Ok(CustomUninstall::Restored { content });
        }
    }
    let mut content = String::new();
    for (index, line) in lines.iter().enumerate() {
        let dropped = Some(index) == marker_line
            || ours
                .iter()
                .any(|entry| index >= entry.span.0 && index < entry.span.1);
        if !dropped {
            content.push_str(line);
        }
    }
    Ok(CustomUninstall::Restored { content })
}

#[cfg(test)]
mod tests;
