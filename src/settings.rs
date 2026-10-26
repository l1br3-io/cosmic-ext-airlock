//! User settings: config parsing and workspace-reference resolution.
//!
//! Pure — config bytes in, scope decisions out. No compositor, no daemon,
//! no file watching: the daemon owns *when* to read, this module owns
//! *how* it parses and resolves.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::switcher::{Scope, Workspace, WorkspaceId};

/// Cap on config size: reloads run at session open, on the gesture path,
/// so a pathological file must never stall switching.
const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

/// One `[[workspace]]` rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRule {
    pub reference: WorkspaceRef,
    pub isolated: bool,
}

impl WorkspaceRule {
    /// The scope this rule selects: [`Scope::All`] when not isolated, the
    /// v1 behavior otherwise.
    pub fn scope(&self) -> Scope {
        if self.isolated {
            Scope::CurrentWorkspace
        } else {
            Scope::All
        }
    }
}

/// How a rule points at a workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceRef {
    /// Matches a workspace COSMIC reports with exactly this name.
    Name(String),
    /// Matches the workspace at this 1-based position in
    /// coordinates-sorted order.
    Position(u32),
}

/// Parsed settings. An empty one reproduces v1 behavior everywhere.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    pub workspaces: Vec<WorkspaceRule>,
}

/// The workspace a gesture was invoked on, as resolution sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActiveWorkspace<'a> {
    /// Compositor-reported name; `None` when the workspace is unnamed.
    pub name: Option<&'a str>,
    /// 1-based position in coordinates-sorted workspace order.
    pub position: u32,
}

/// Scope for the workspace a gesture was invoked on: a `name` rule wins
/// for a named workspace, `index` rules apply otherwise, duplicate
/// references resolve last-wins, and no match keeps the v1 default.
pub fn scope_for(active: ActiveWorkspace<'_>, settings: &Settings) -> Scope {
    let by_name = active.name.and_then(|name| {
        last_matching(
            settings,
            |rule| matches!(&rule.reference, WorkspaceRef::Name(n) if n == name),
        )
    });
    let by_position = last_matching(
        settings,
        |rule| matches!(rule.reference, WorkspaceRef::Position(p) if p == active.position),
    );
    by_name
        .or(by_position)
        .map_or(Scope::CurrentWorkspace, WorkspaceRule::scope)
}

/// Last rule satisfying `matches` — the duplicate-resolution rule.
fn last_matching(
    settings: &Settings,
    matches: impl Fn(&WorkspaceRule) -> bool,
) -> Option<&WorkspaceRule> {
    settings.workspaces.iter().rev().find(|rule| matches(rule))
}

/// Name and 1-based position of `active` within `workspaces`, the position
/// being the index in coordinates-sorted (lexicographic) order. `None`
/// when the active workspace is absent from the list.
pub fn active_workspace(
    workspaces: &[Workspace],
    active: WorkspaceId,
) -> Option<ActiveWorkspace<'_>> {
    let mut sorted: Vec<&Workspace> = workspaces.iter().collect();
    sorted.sort_by(|a, b| a.coordinates.cmp(&b.coordinates));
    let (index, workspace) = sorted
        .into_iter()
        .enumerate()
        .find(|(_, workspace)| workspace.id == active)?;
    Some(ActiveWorkspace {
        name: (!workspace.name.is_empty()).then_some(workspace.name.as_str()),
        position: index as u32 + 1,
    })
}

impl Settings {
    /// Parses settings from raw config bytes. An empty file parses to the
    /// same defaults as a missing one.
    ///
    /// # Errors
    ///
    /// A [`SettingsError`] naming the offending line or entry; never a
    /// panic, whatever the bytes contain.
    pub fn parse(bytes: &[u8]) -> Result<Self, SettingsError> {
        let text = std::str::from_utf8(bytes).map_err(|_| SettingsError::InvalidUtf8)?;
        // `from_str`, not `Value::from_str`: the latter mis-reports valid
        // TOML as malformed in toml 0.9.12.
        let root = toml::from_str::<toml::Value>(text).map_err(|error| malformed(text, &error))?;
        parse_root(&root)
    }
}

/// Walks a parsed document: `[[workspace]]` entries become rules.
fn parse_root(root: &toml::Value) -> Result<Settings, SettingsError> {
    let Some(entries) = root.get("workspace") else {
        return Ok(Settings::default());
    };
    let Some(entries) = entries.as_array() else {
        return Err(SettingsError::InvalidFile {
            problem: "`workspace` must be an array of tables (`[[workspace]]`)",
        });
    };
    let workspaces = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| parse_rule(index + 1, entry))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Settings { workspaces })
}

/// Parses one `[[workspace]]` entry; `entry_number` is its 1-based file
/// order, carried into errors so a bad line is findable.
fn parse_rule(entry_number: usize, entry: &toml::Value) -> Result<WorkspaceRule, SettingsError> {
    let invalid = |problem: &'static str| SettingsError::InvalidEntry {
        entry: entry_number,
        problem,
    };
    let table = entry.as_table().ok_or_else(|| invalid("must be a table"))?;
    let reference = match (table.get("name"), table.get("index")) {
        (Some(_), Some(_)) => {
            return Err(invalid("use either `name` or `index`, not both"));
        }
        (None, None) => return Err(invalid("needs `name` or `index`")),
        (Some(value), None) => WorkspaceRef::Name(
            value
                .as_str()
                .ok_or_else(|| invalid("`name` must be a string"))?
                .to_owned(),
        ),
        (None, Some(value)) => {
            let index = value
                .as_integer()
                .ok_or_else(|| invalid("`index` must be a whole number"))?;
            let index =
                u32::try_from(index).map_err(|_| invalid("`index` must be 1 or greater"))?;
            if index == 0 {
                return Err(invalid("`index` is 1-based and must be 1 or greater"));
            }
            WorkspaceRef::Position(index)
        }
    };
    let isolated = match table.get("isolated") {
        None => true,
        Some(value) => value
            .as_bool()
            .ok_or_else(|| invalid("`isolated` must be a boolean"))?,
    };
    Ok(WorkspaceRule {
        reference,
        isolated,
    })
}

/// Converts a parser error into a [`SettingsError`], keeping its span as
/// 1-based line/column (0/0 when the parser reported no span).
fn malformed(source: &str, error: &toml::de::Error) -> SettingsError {
    let (line, column) = error
        .span()
        .map(|span| line_column(source, span.start))
        .unwrap_or((0, 0));
    SettingsError::MalformedToml {
        line,
        column,
        message: error.message().to_owned(),
    }
}

/// 1-based line and column of a byte offset in `source`.
fn line_column(source: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(source.len());
    let before = &source[..offset];
    let line = before.matches('\n').count() + 1;
    let column = before
        .rsplit('\n')
        .next()
        .map_or(1, |l| l.chars().count() + 1);
    (line, column)
}

/// Why a config could not be used.
#[derive(Debug, PartialEq, Eq)]
pub enum SettingsError {
    /// The bytes are not valid UTF-8.
    InvalidUtf8,
    /// Malformed TOML; line/column are 0 when the parser reported no span.
    MalformedToml {
        line: usize,
        column: usize,
        message: String,
    },
    /// The document shape is wrong outside any single entry.
    InvalidFile { problem: &'static str },
    /// Entry `entry` (1-based, in file order) is not a valid rule.
    InvalidEntry { entry: usize, problem: &'static str },
}

impl fmt::Display for SettingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SettingsError::InvalidUtf8 => write!(f, "settings: file is not valid UTF-8"),
            SettingsError::MalformedToml {
                line,
                column,
                message,
            } if *line > 0 => write!(
                f,
                "settings: malformed TOML at line {line}, column {column}: {message}"
            ),
            SettingsError::MalformedToml { message, .. } => {
                write!(f, "settings: malformed TOML: {message}")
            }
            SettingsError::InvalidFile { problem } => {
                write!(f, "settings: {problem}")
            }
            SettingsError::InvalidEntry { entry, problem } => {
                write!(f, "settings: workspace entry {entry}: {problem}")
            }
        }
    }
}

impl std::error::Error for SettingsError {}

/// What a [`SettingsSource::check`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reload {
    /// No config file exists; defaults apply.
    NotPresent,
    /// Same mtime as the last check; the loaded settings still apply.
    Unchanged,
    /// New file or new mtime; read and parse again.
    Changed,
}

/// Why reading the config from disk did not produce [`Settings`].
#[derive(Debug)]
pub enum ReadError {
    /// No config file — defaults apply silently.
    Missing,
    /// The file exceeds [`MAX_CONFIG_BYTES`]; defaults apply.
    TooLarge,
    /// The file could not be read.
    Io(std::io::Error),
    /// The file read but did not parse.
    Malformed(SettingsError),
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReadError::Missing => write!(f, "settings: no config file"),
            ReadError::TooLarge => write!(
                f,
                "settings: config exceeds {} bytes and was ignored",
                MAX_CONFIG_BYTES
            ),
            ReadError::Io(error) => write!(f, "settings: cannot read config: {error}"),
            ReadError::Malformed(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for ReadError {}

/// Resolves the config path and answers "did it change?" with one stat,
/// so the daemon can reload cheaply on the gesture path without a watcher
/// dependency.
pub struct SettingsSource {
    path: Option<PathBuf>,
    last_mtime: Option<SystemTime>,
}

impl SettingsSource {
    /// Source at the user's config path (`$XDG_CONFIG_HOME` absolute else
    /// `$HOME/.config`, then `airlock/config.toml` — the same rule as the
    /// installer). `None` path when the environment names neither: every
    /// check reports [`Reload::NotPresent`] and defaults apply forever.
    pub fn new() -> Self {
        let dir = config_dir(
            std::env::var_os("XDG_CONFIG_HOME")
                .as_deref()
                .map(Path::new),
            std::env::var_os("HOME").as_deref().map(Path::new),
        );
        Self {
            path: dir.map(|dir| dir.join("airlock/config.toml")),
            last_mtime: None,
        }
    }

    /// Source at an explicit path (tests).
    #[cfg(test)]
    pub(crate) fn at(path: PathBuf) -> Self {
        Self {
            path: Some(path),
            last_mtime: None,
        }
    }

    /// One-stat reload check. Remembers the mtime it reports, so an
    /// unchanged file is cheap to skip and a malformed re-read keeps
    /// last-good settings until the file actually changes again.
    pub fn check(&mut self) -> Reload {
        let mtime = self
            .path
            .as_deref()
            .and_then(|path| fs::metadata(path).ok())
            .and_then(|metadata| metadata.modified().ok());
        match mtime {
            None => {
                self.last_mtime = None;
                Reload::NotPresent
            }
            Some(seen) if self.last_mtime == Some(seen) => Reload::Unchanged,
            Some(seen) => {
                self.last_mtime = Some(seen);
                Reload::Changed
            }
        }
    }

    /// Reads and parses the config now. Size-capped so a reload can never
    /// stall the gesture.
    ///
    /// # Errors
    ///
    /// [`ReadError::Missing`] for no file (expected, silent), anything
    /// else is diagnostic-worthy.
    pub fn read(&self) -> Result<Settings, ReadError> {
        let Some(path) = self.path.as_deref() else {
            return Err(ReadError::Missing);
        };
        let metadata = fs::metadata(path).map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => ReadError::Missing,
            _ => ReadError::Io(error),
        })?;
        if metadata.len() > MAX_CONFIG_BYTES {
            return Err(ReadError::TooLarge);
        }
        let bytes = fs::read(path).map_err(ReadError::Io)?;
        Settings::parse(&bytes).map_err(ReadError::Malformed)
    }
}

/// XDG rule shared with the installer: absolute `$XDG_CONFIG_HOME`, else
/// `$HOME/.config` (empty or relative XDG values are ignored); `None`
/// when the environment names neither.
fn config_dir(xdg_config_home: Option<&Path>, home: Option<&Path>) -> Option<PathBuf> {
    if let Some(dir) = xdg_config_home
        && !dir.as_os_str().is_empty()
        && dir.is_absolute()
    {
        return Some(dir.to_path_buf());
    }
    home.filter(|home| !home.as_os_str().is_empty())
        .map(|home| home.join(".config"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Parse ---------------------------------------------------------

    #[test]
    fn parse_reads_name_and_index_rules() {
        let config = br#"
[[workspace]]
name = "code"
isolated = true

[[workspace]]
index = 2
isolated = false
"#;
        let settings = Settings::parse(config).expect("valid config");
        assert_eq!(
            settings,
            Settings {
                workspaces: vec![
                    WorkspaceRule {
                        reference: WorkspaceRef::Name("code".to_owned()),
                        isolated: true,
                    },
                    WorkspaceRule {
                        reference: WorkspaceRef::Position(2),
                        isolated: false,
                    },
                ]
            }
        );
    }

    #[test]
    fn parse_on_an_empty_file_yields_defaults() {
        let settings = Settings::parse(b"").expect("empty is valid");
        assert_eq!(settings, Settings::default());
    }

    #[test]
    fn parse_on_a_file_without_workspace_entries_yields_defaults() {
        let settings = Settings::parse(b"something_else = 1\n").expect("valid");
        assert_eq!(settings, Settings::default());
    }

    #[test]
    fn parse_defaults_isolated_to_true_when_the_key_is_absent() {
        let settings = Settings::parse(b"[[workspace]]\nname = \"a\"\n").expect("valid");
        assert_eq!(settings.workspaces.len(), 1);
        assert!(settings.workspaces[0].isolated);
    }

    #[test]
    fn parse_ignores_unknown_keys() {
        let settings =
            Settings::parse(b"[[workspace]]\nname = \"a\"\nunexpected = 1\n").expect("valid");
        assert_eq!(settings.workspaces.len(), 1);
    }

    #[test]
    fn parse_rejects_malformed_toml_with_position_info() {
        let error = Settings::parse(b"[[workspace]\nname = ").expect_err("malformed");
        match error {
            SettingsError::MalformedToml { line, column, .. } => {
                assert!(line >= 1, "line should be reported, got {line}");
                assert!(column >= 1, "column should be reported, got {column}");
            }
            other => panic!("expected MalformedToml, got {other:?}"),
        }
    }

    #[test]
    fn parse_rejects_an_entry_with_both_name_and_index() {
        let error = Settings::parse(b"[[workspace]]\nname = \"a\"\nindex = 1\nisolated = false\n")
            .expect_err("both keys");
        assert_eq!(
            error,
            SettingsError::InvalidEntry {
                entry: 1,
                problem: "use either `name` or `index`, not both"
            }
        );
    }

    #[test]
    fn parse_rejects_an_entry_with_neither_name_nor_index() {
        let error = Settings::parse(b"[[workspace]]\nisolated = false\n").expect_err("neither key");
        assert_eq!(
            error,
            SettingsError::InvalidEntry {
                entry: 1,
                problem: "needs `name` or `index`"
            }
        );
    }

    #[test]
    fn parse_rejects_index_zero() {
        let error = Settings::parse(b"[[workspace]]\nindex = 0\n").expect_err("zero index");
        assert_eq!(
            error,
            SettingsError::InvalidEntry {
                entry: 1,
                problem: "`index` is 1-based and must be 1 or greater"
            }
        );
    }

    #[test]
    fn parse_rejects_a_negative_index() {
        let error = Settings::parse(b"[[workspace]]\nindex = -1\n").expect_err("negative");
        assert_eq!(
            error,
            SettingsError::InvalidEntry {
                entry: 1,
                problem: "`index` must be 1 or greater"
            }
        );
    }

    #[test]
    fn parse_rejects_a_non_string_name() {
        let error = Settings::parse(b"[[workspace]]\nname = 3\n").expect_err("numeric name");
        assert_eq!(
            error,
            SettingsError::InvalidEntry {
                entry: 1,
                problem: "`name` must be a string"
            }
        );
    }

    #[test]
    fn parse_rejects_a_non_integer_index() {
        let error = Settings::parse(b"[[workspace]]\nindex = 1.5\n").expect_err("float index");
        assert_eq!(
            error,
            SettingsError::InvalidEntry {
                entry: 1,
                problem: "`index` must be a whole number"
            }
        );
    }

    #[test]
    fn parse_rejects_a_non_boolean_isolated() {
        let error = Settings::parse(b"[[workspace]]\nname = \"a\"\nisolated = \"yes\"\n")
            .expect_err("string isolated");
        assert_eq!(
            error,
            SettingsError::InvalidEntry {
                entry: 1,
                problem: "`isolated` must be a boolean"
            }
        );
    }

    #[test]
    fn parse_rejects_invalid_utf8() {
        assert_eq!(
            Settings::parse(b"\xff\xfe[[workspace]]").expect_err("not utf-8"),
            SettingsError::InvalidUtf8
        );
    }

    #[test]
    fn parse_rejects_workspace_as_a_non_array() {
        // `[workspace]` (table) instead of `[[workspace]]` (array of tables).
        let error = Settings::parse(b"[workspace]\nname = \"a\"\n").expect_err("not an array");
        assert_eq!(
            error,
            SettingsError::InvalidFile {
                problem: "`workspace` must be an array of tables (`[[workspace]]`)"
            }
        );
    }

    #[test]
    fn parse_reports_the_entry_number_of_the_bad_rule() {
        let config = b"[[workspace]]\nname = \"good\"\n\n[[workspace]]\nindex = 0\n";
        let error = Settings::parse(config).expect_err("second entry is bad");
        assert_eq!(
            error,
            SettingsError::InvalidEntry {
                entry: 2,
                problem: "`index` is 1-based and must be 1 or greater"
            }
        );
    }

    // --- Resolution ------------------------------------------------------

    fn rule(reference: WorkspaceRef, isolated: bool) -> WorkspaceRule {
        WorkspaceRule {
            reference,
            isolated,
        }
    }

    #[test]
    fn scope_for_matches_a_named_workspace_by_name() {
        let settings = Settings {
            workspaces: vec![rule(WorkspaceRef::Name("browse".to_owned()), false)],
        };
        let scope = scope_for(
            ActiveWorkspace {
                name: Some("browse"),
                position: 3,
            },
            &settings,
        );
        assert_eq!(scope, Scope::All);
    }

    #[test]
    fn scope_for_matches_an_unnamed_workspace_by_position() {
        let settings = Settings {
            workspaces: vec![rule(WorkspaceRef::Position(2), false)],
        };
        let scope = scope_for(
            ActiveWorkspace {
                name: None,
                position: 2,
            },
            &settings,
        );
        assert_eq!(scope, Scope::All);
    }

    #[test]
    fn scope_for_prefers_the_name_rule_for_a_named_workspace() {
        // Both rules point at the same workspace; the name rule wins.
        let settings = Settings {
            workspaces: vec![
                rule(WorkspaceRef::Position(1), true),
                rule(WorkspaceRef::Name("code".to_owned()), false),
            ],
        };
        let scope = scope_for(
            ActiveWorkspace {
                name: Some("code"),
                position: 1,
            },
            &settings,
        );
        assert_eq!(scope, Scope::All);
    }

    #[test]
    fn scope_for_falls_back_to_the_position_rule_for_a_named_workspace() {
        let settings = Settings {
            workspaces: vec![rule(WorkspaceRef::Position(1), false)],
        };
        let scope = scope_for(
            ActiveWorkspace {
                name: Some("code"),
                position: 1,
            },
            &settings,
        );
        assert_eq!(scope, Scope::All);
    }

    #[test]
    fn scope_for_on_an_out_of_range_position_matches_nothing() {
        let settings = Settings {
            workspaces: vec![rule(WorkspaceRef::Position(9), false)],
        };
        let scope = scope_for(
            ActiveWorkspace {
                name: None,
                position: 2,
            },
            &settings,
        );
        assert_eq!(scope, Scope::CurrentWorkspace);
    }

    #[test]
    fn scope_for_last_duplicate_rule_wins() {
        let settings = Settings {
            workspaces: vec![
                rule(WorkspaceRef::Name("browse".to_owned()), false),
                rule(WorkspaceRef::Name("browse".to_owned()), true),
            ],
        };
        let scope = scope_for(
            ActiveWorkspace {
                name: Some("browse"),
                position: 1,
            },
            &settings,
        );
        assert_eq!(scope, Scope::CurrentWorkspace);
    }

    #[test]
    fn scope_for_without_a_matching_rule_is_current_workspace() {
        let settings = Settings {
            workspaces: vec![rule(WorkspaceRef::Name("elsewhere".to_owned()), false)],
        };
        let scope = scope_for(
            ActiveWorkspace {
                name: Some("code"),
                position: 1,
            },
            &settings,
        );
        assert_eq!(scope, Scope::CurrentWorkspace);
    }

    #[test]
    fn active_workspace_sorts_by_coordinates_and_numbers_from_one() {
        // Snapshot order is shuffled relative to coordinates.
        let workspaces = vec![
            Workspace {
                id: WorkspaceId(30),
                name: String::new(),
                coordinates: vec![2, 0],
            },
            Workspace {
                id: WorkspaceId(10),
                name: "code".to_owned(),
                coordinates: vec![0, 0],
            },
            Workspace {
                id: WorkspaceId(20),
                name: String::new(),
                coordinates: vec![1, 0],
            },
        ];
        let first = active_workspace(&workspaces, WorkspaceId(10)).expect("known workspace");
        assert_eq!(first.name, Some("code"));
        assert_eq!(first.position, 1);

        let third = active_workspace(&workspaces, WorkspaceId(30)).expect("known workspace");
        assert_eq!(third.name, None, "empty name means unnamed");
        assert_eq!(third.position, 3);
    }

    #[test]
    fn active_workspace_on_an_unknown_workspace_is_none() {
        let workspaces = vec![Workspace {
            id: WorkspaceId(1),
            name: String::new(),
            coordinates: vec![0, 0],
        }];
        assert_eq!(active_workspace(&workspaces, WorkspaceId(99)), None);
    }

    // --- Source ----------------------------------------------------------

    #[test]
    fn config_dir_prefers_an_absolute_xdg_config_home() {
        assert_eq!(
            config_dir(Some(Path::new("/xdg/abs")), Some(Path::new("/home/u"))),
            Some(PathBuf::from("/xdg/abs"))
        );
    }

    #[test]
    fn config_dir_ignores_a_relative_xdg_config_home() {
        assert_eq!(
            config_dir(Some(Path::new("relative/xdg")), Some(Path::new("/home/u"))),
            Some(PathBuf::from("/home/u/.config"))
        );
    }

    #[test]
    fn config_dir_ignores_an_empty_xdg_config_home() {
        assert_eq!(
            config_dir(Some(Path::new("")), Some(Path::new("/home/u"))),
            Some(PathBuf::from("/home/u/.config"))
        );
    }

    #[test]
    fn config_dir_without_home_is_none() {
        assert_eq!(config_dir(Some(Path::new("relative")), None), None);
        assert_eq!(config_dir(None, None), None);
    }

    /// Unique disposable directory (no tempfile dependency).
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "airlock-settings-{tag}-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            ));
            std::fs::create_dir_all(&path).expect("create temp dir");
            TempDir(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn check_reports_not_present_without_a_file() {
        let dir = TempDir::new("missing");
        let mut source = SettingsSource::at(dir.path().join("config.toml"));
        assert_eq!(source.check(), Reload::NotPresent);
    }

    #[test]
    fn check_reports_changed_then_unchanged_for_a_stable_file() {
        let dir = TempDir::new("stable");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, b"[[workspace]]\nname = \"a\"\n").expect("write");
        let mut source = SettingsSource::at(path);
        assert_eq!(source.check(), Reload::Changed);
        assert_eq!(source.check(), Reload::Unchanged);
    }

    #[test]
    fn check_reports_changed_again_after_a_rewrite() {
        let dir = TempDir::new("rewrite");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, b"[[workspace]]\nname = \"a\"\n").expect("write");
        let mut source = SettingsSource::at(path.clone());
        assert_eq!(source.check(), Reload::Changed);

        std::fs::write(&path, b"[[workspace]]\nname = \"b\"\n").expect("rewrite");
        assert_eq!(source.check(), Reload::Changed);
    }

    #[test]
    fn check_returns_to_not_present_after_the_file_is_removed() {
        let dir = TempDir::new("removed");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, b"[[workspace]]\nname = \"a\"\n").expect("write");
        let mut source = SettingsSource::at(path.clone());
        assert_eq!(source.check(), Reload::Changed);

        std::fs::remove_file(&path).expect("remove");
        assert_eq!(source.check(), Reload::NotPresent);
    }

    #[test]
    fn read_on_a_missing_file_reports_missing() {
        let dir = TempDir::new("read-missing");
        let source = SettingsSource::at(dir.path().join("config.toml"));
        assert!(matches!(source.read(), Err(ReadError::Missing)));
    }

    #[test]
    fn read_parses_a_config_file_from_disk() {
        let dir = TempDir::new("read-ok");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, b"[[workspace]]\nname = \"code\"\nisolated = false\n")
            .expect("write");
        let source = SettingsSource::at(path);
        let settings = source.read().expect("parses");
        assert_eq!(
            settings,
            Settings {
                workspaces: vec![rule(WorkspaceRef::Name("code".to_owned()), false)]
            }
        );
    }

    #[test]
    fn read_on_a_malformed_file_reports_the_settings_error() {
        let dir = TempDir::new("read-bad");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, b"[[workspace]]\nindex = 0\n").expect("write");
        let source = SettingsSource::at(path);
        match source.read().expect_err("malformed") {
            ReadError::Malformed(SettingsError::InvalidEntry { entry: 1, .. }) => {}
            other => panic!("expected Malformed InvalidEntry, got {other:?}"),
        }
    }

    #[test]
    fn read_rejects_an_oversized_file() {
        let dir = TempDir::new("read-huge");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, vec![b'x'; 1024 * 1024 + 1]).expect("write");
        let source = SettingsSource::at(path);
        assert!(matches!(source.read(), Err(ReadError::TooLarge)));
    }
}
