//! User-level `system_actions` takeover: `airlock install` points COSMIC's
//! `WindowSwitcher`/`WindowSwitcherPrevious` system actions at this binary,
//! `airlock install --uninstall` puts the pre-install state back byte-exactly.
//!
//! Boundary (spec): the only file this module may ever write is the user's
//! `system_actions`. The pre-install state is therefore recorded *inside*
//! that file as leading `// airlock-v1` comment markers — one per switcher
//! key — so no second file is needed and the backup can never be orphaned.
//! `/usr/share` is only ever read, as the source for first installs.

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
/// Prefix of every backup marker line install writes.
const MARKER_PREFIX: &str = "// airlock-v1 ";
/// Marker value when no user file existed before install.
const MARKER_ABSENT_FILE: &str = "absent-file";
/// Marker value when the file existed but had no entry for the key.
const MARKER_ABSENT_KEY: &str = "absent-key";

/// The two files install works with.
pub struct Locations {
    /// User override file; the only file ever written.
    pub user: PathBuf,
    /// System default; read-only source for first installs and best-effort
    /// restores.
    pub system_default: PathBuf,
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
    if uninstall {
        let Some(user) = read_optional(&locations.user)? else {
            return Ok(format!(
                "nothing to uninstall: {} does not exist",
                locations.user.display()
            ));
        };
        let system_default = read_system_default(locations)?;
        return match uninstall_content(&user, &system_default)? {
            UninstallOutcome::Restored {
                content,
                best_effort,
            } => {
                write_atomic(&locations.user, &content)?;
                if best_effort {
                    Ok(format!(
                        "restored {} to the system default commands (no airlock backup \
                         was recorded in the file)",
                        locations.user.display()
                    ))
                } else {
                    Ok(format!(
                        "restored {} to its pre-install state",
                        locations.user.display()
                    ))
                }
            }
            UninstallOutcome::RemoveFile => {
                fs::remove_file(&locations.user).map_err(|error| {
                    format!("cannot remove {}: {error}", locations.user.display())
                })?;
                Ok(format!(
                    "removed {}; the system default switcher is back",
                    locations.user.display()
                ))
            }
            UninstallOutcome::NotInstalled => Ok(format!(
                "nothing to uninstall: no airlock takeover in {}",
                locations.user.display()
            )),
        };
    }

    let system_default = read_system_default(locations)?;
    let user = read_optional(&locations.user)?;
    let fresh = user.is_none();
    let content = install_content(user.as_deref(), &system_default)?;
    if let Some(dir) = locations.user.parent() {
        fs::create_dir_all(dir)
            .map_err(|error| format!("cannot create {}: {error}", dir.display()))?;
    }
    write_atomic(&locations.user, &content)?;
    let origin = if fresh {
        format!(
            "created {} from the system default",
            locations.user.display()
        )
    } else {
        format!("updated {}", locations.user.display())
    };
    Ok(format!(
        "installed: {SWITCHER_KEY} and {SWITCHER_PREV_KEY} now run airlock \
         ({origin}; undo with `airlock install --uninstall`)"
    ))
}

/// Resolves the real [`Locations`]: `$XDG_CONFIG_HOME` (absolute) or
/// `$HOME/.config` per the XDG spec, plus the fixed system default path.
///
/// # Errors
///
/// Returns a diagnostic when neither an absolute `XDG_CONFIG_HOME` nor a
/// usable `HOME` is set.
fn default_locations() -> Result<Locations, String> {
    let config = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(dir) if !dir.is_empty() && Path::new(&dir).is_absolute() => PathBuf::from(dir),
        // An empty or relative XDG_CONFIG_HOME is ignored per the XDG spec.
        _ => {
            let home = std::env::var_os("HOME")
                .filter(|home| !home.is_empty())
                .ok_or_else(|| {
                    "cannot find the config directory: no absolute XDG_CONFIG_HOME and no HOME"
                        .to_string()
                })?;
            PathBuf::from(home).join(".config")
        }
    };
    Ok(Locations {
        user: config.join("cosmic/com.system76.CosmicSettings.Shortcuts/v1/system_actions"),
        system_default: PathBuf::from(
            "/usr/share/cosmic/com.system76.CosmicSettings.Shortcuts/v1/system_actions",
        ),
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
/// written by hand and only the system default can put it back.
fn restore_without_markers(
    lines: &[&str],
    system_default: &str,
) -> Result<UninstallOutcome, String> {
    let airlock_owned = [(SWITCHER_KEY, NEXT_CMD), (SWITCHER_PREV_KEY, PREV_CMD)]
        .into_iter()
        .any(|(key, command)| {
            key_line_index(lines, key).is_some_and(|index| lines[index].contains(command))
        });
    if !airlock_owned {
        return Ok(UninstallOutcome::NotInstalled);
    }
    let defaults = split_lines(system_default);
    let mut restored: Vec<String> = lines.iter().map(|line| line.to_string()).collect();
    for key in [SWITCHER_KEY, SWITCHER_PREV_KEY] {
        let Some(index) = key_line_index(lines, key) else {
            continue;
        };
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
fn write_atomic(path: &Path, content: &str) -> Result<(), String> {
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
        file.write_all(content.as_bytes())?;
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

#[cfg(test)]
mod tests;
