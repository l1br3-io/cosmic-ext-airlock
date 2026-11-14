//! User settings: config parsing and workspace-reference resolution.
//!
//! Pure — config bytes in, scope decisions out. No compositor, no daemon,
//! no file watching: the daemon owns *when* to read, this module owns
//! *how* it parses and resolves.

use std::fmt;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::switcher::{Scope, Workspace, WorkspaceId};
use crate::ui::{
    self, ColumnsSetting, LayoutSetting, SelectionSetting, ThemeSetting, WidthSetting,
};

/// Cap on config size: reloads run at session open, on the gesture path,
/// so a pathological file must never stall switching.
const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

/// The `columns` union's one diagnostic: any bad value — wrong type, a
/// string that isn't `"auto"`, out of range — names the whole accepted
/// shape (the key is a union; a split message would say the same thing
/// twice). The range words must track [`ui::COLUMNS_MIN`]/
/// [`ui::COLUMNS_MAX`]; a test pins the pair.
const COLUMNS_PROBLEM: &str = "`columns` must be \"auto\" or a whole number between 3 and 12";

/// The `all_windows_key` diagnostic (all-windows, 2026-11-14): the value
/// names a keysym under the binding's fixed Alt modifier, and the
/// compositor is the authority airlock cannot check offline — so any
/// non-empty name parses and only the shape rejects here.
const ALL_WINDOWS_KEY_PROBLEM: &str = "`all_windows_key` must be a non-empty key name";

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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// Which card the overlay paints; missing key = [`ThemeSetting::Auto`].
    pub theme: ThemeSetting,
    /// Windows the card keeps in view on a long list; missing key =
    /// [`ui::DEFAULT_ROWS`], accepted `ui::ROWS_MIN..=ui::ROWS_MAX`.
    pub rows: usize,
    /// Whether the switcher groups its list by app — the tree view with
    /// parents, children, and linear stepping over the visible rows;
    /// missing key = `false`, the flat list, so the no-config card is
    /// unchanged.
    pub grouping: bool,
    /// Whether the card labels itself with the invoking workspace's
    /// name (workspace-label, 2026-11-09) — a small line on every card,
    /// top-left on the vertical ones, bottom-center on the horizontal
    /// ones. Missing key = `false`, no label, so the no-config card is
    /// unchanged.
    pub show_workspace: bool,
    /// Which way the card lays its list out; missing key =
    /// [`LayoutSetting::Vertical`], today's card, so the no-config
    /// card is unchanged.
    pub layout: LayoutSetting,
    /// The horizontal card's tile budget; missing key =
    /// [`ColumnsSetting::Auto`] — fit to the invoking output. Binds
    /// `layout = "horizontal"` only; the vertical card ignores it.
    pub columns: ColumnsSetting,
    /// The card's width floor; missing key = [`WidthSetting::Auto`] —
    /// the content-sized card. Both layouts read it; a floor below the
    /// card's content width does nothing (nothing ever clips) and the
    /// daemon clamps one past the invoking output.
    pub width: WidthSetting,
    /// The card's never-below width; same syntax as [`Self::width`],
    /// and the pair collapses to the wider floor at open.
    pub min_width: WidthSetting,
    /// The custom card/text pair from the paired `card`/`text` keys,
    /// each `0x00RRGGBB` (alpha unreachable by construction) and
    /// contrast-checked against [`ui::CONTRAST_FLOOR`] at parse.
    /// `None` when neither key is set; a present pair pins the card
    /// over any `theme` value.
    pub pair: Option<(u32, u32)>,
    /// What the selection band is made of; missing key =
    /// [`SelectionSetting::Accent`] — the follow, the desktop's accent
    /// at 20 % over the effective card. `Neutral` is the shipped band.
    pub selection: SelectionSetting,
    /// Which family the card's titles paint in; missing key =
    /// [`text::FontSetting::Default`] — the COSMIC interface font's
    /// family. Any string is valid syntax; resolution (and its
    /// diagnostics) happen at open, never at parse.
    pub font: crate::text::FontSetting,
    /// The card's text size in logical pixels; missing key =
    /// [`ui::DEFAULT_FONT_SIZE`], accepted
    /// `ui::FONT_SIZE_MIN..=ui::FONT_SIZE_MAX`. Every text-derived
    /// metric — title band, query strip, caption line, band height —
    /// resolves from it at open.
    pub font_size: u32,
    /// The key the all-windows binding holds under its fixed Alt
    /// modifier (all-windows, 2026-11-14): a keysym name — canonical X
    /// names like "Escape", "grave", "Tab" — the compositor resolves, so
    /// any non-empty name parses and only the shape rejects. The
    /// `-previous` twin adds Shift, and a key whose keysym changes under
    /// Shift (letters, digits) makes that twin unreliable. Missing key =
    /// "Escape".
    pub all_windows_key: String,
    pub workspaces: Vec<WorkspaceRule>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: ThemeSetting::Auto,
            rows: ui::DEFAULT_ROWS,
            grouping: false,
            show_workspace: false,
            layout: LayoutSetting::Vertical,
            columns: ColumnsSetting::Auto,
            width: WidthSetting::Auto,
            min_width: WidthSetting::Auto,
            pair: None,
            selection: SelectionSetting::Accent,
            font: crate::text::FontSetting::Default,
            font_size: ui::DEFAULT_FONT_SIZE,
            all_windows_key: "Escape".to_owned(),
            workspaces: Vec::new(),
        }
    }
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

/// Walks a parsed document: the top-level `theme` key becomes the
/// [`ThemeSetting`], `rows` the windows-in-view budget, `[[workspace]]`
/// entries become rules.
fn parse_root(root: &toml::Value) -> Result<Settings, SettingsError> {
    let theme = match root.get("theme") {
        None => ThemeSetting::Auto,
        Some(value) => match value.as_str() {
            Some("auto") => ThemeSetting::Auto,
            Some("dark") => ThemeSetting::Dark,
            Some("light") => ThemeSetting::Light,
            Some(_) => {
                return Err(SettingsError::InvalidFile {
                    problem: "`theme` must be one of \"auto\", \"dark\", or \"light\"",
                });
            }
            None => {
                return Err(SettingsError::InvalidFile {
                    problem: "`theme` must be a string",
                });
            }
        },
    };
    let rows = match root.get("rows") {
        None => ui::DEFAULT_ROWS,
        Some(value) => {
            let rows = value.as_integer().ok_or(SettingsError::InvalidFile {
                problem: "`rows` must be a whole number",
            })?;
            usize::try_from(rows)
                .ok()
                .filter(|rows| (ui::ROWS_MIN..=ui::ROWS_MAX).contains(rows))
                .ok_or(SettingsError::InvalidFile {
                    problem: "`rows` must be between 3 and 10",
                })?
        }
    };
    let grouping = match root.get("grouping") {
        None => false,
        Some(value) => value.as_bool().ok_or(SettingsError::InvalidFile {
            problem: "`grouping` must be a boolean",
        })?,
    };
    let show_workspace = match root.get("show_workspace") {
        None => false,
        Some(value) => value.as_bool().ok_or(SettingsError::InvalidFile {
            problem: "`show_workspace` must be a boolean",
        })?,
    };
    let layout = match root.get("layout") {
        None => LayoutSetting::Vertical,
        Some(value) => match value.as_str() {
            Some("vertical") => LayoutSetting::Vertical,
            Some("horizontal") => LayoutSetting::Horizontal,
            Some(_) => {
                return Err(SettingsError::InvalidFile {
                    problem: "`layout` must be \"vertical\" or \"horizontal\"",
                });
            }
            None => {
                return Err(SettingsError::InvalidFile {
                    problem: "`layout` must be a string",
                });
            }
        },
    };
    let columns = match root.get("columns") {
        None => ColumnsSetting::Auto,
        Some(value) => match value.as_str() {
            Some("auto") => ColumnsSetting::Auto,
            Some(_) => {
                return Err(SettingsError::InvalidFile {
                    problem: COLUMNS_PROBLEM,
                });
            }
            None => {
                let count = value.as_integer().ok_or(SettingsError::InvalidFile {
                    problem: COLUMNS_PROBLEM,
                })?;
                let count = usize::try_from(count)
                    .ok()
                    .filter(|count| (ui::COLUMNS_MIN..=ui::COLUMNS_MAX).contains(count))
                    .ok_or(SettingsError::InvalidFile {
                        problem: COLUMNS_PROBLEM,
                    })?;
                ColumnsSetting::Pinned(count)
            }
        },
    };
    let width = width_key(root.get("width"), WIDTH_PROBLEM)?;
    let min_width = width_key(root.get("min-width"), MIN_WIDTH_PROBLEM)?;
    let card = color_key(
        root.get("card"),
        "`card` must be a string holding a 6-digit hex color",
        "`card` must be a 6-digit hex color like \"#1E2126\"",
    )?;
    let text = color_key(
        root.get("text"),
        "`text` must be a string holding a 6-digit hex color",
        "`text` must be a 6-digit hex color like \"#F2F4F8\"",
    )?;
    let pair = match (card, text) {
        (None, None) => None,
        (Some(card), Some(text)) => {
            let ratio = ui::contrast_ratio(card, text);
            if ratio < ui::CONTRAST_FLOOR {
                return Err(SettingsError::InvalidContrast { ratio });
            }
            Some((card, text))
        }
        _ => {
            return Err(SettingsError::InvalidFile {
                problem: "`card` and `text` must be set together",
            });
        }
    };
    let selection = match root.get("selection") {
        None => SelectionSetting::Accent,
        Some(value) => match value.as_str() {
            Some("accent") => SelectionSetting::Accent,
            Some("neutral") => SelectionSetting::Neutral,
            Some(_) => {
                return Err(SettingsError::InvalidFile {
                    problem: "`selection` must be \"accent\" or \"neutral\"",
                });
            }
            None => {
                return Err(SettingsError::InvalidFile {
                    problem: "`selection` must be a string",
                });
            }
        },
    };
    let font = match root.get("font") {
        None => crate::text::FontSetting::Default,
        Some(value) => crate::text::FontSetting::Family(
            value
                .as_str()
                .ok_or(SettingsError::InvalidFile {
                    problem: "`font` must be a font family name (a string)",
                })?
                .to_owned(),
        ),
    };
    let all_windows_key = match root.get("all_windows_key") {
        None => "Escape".to_owned(),
        Some(value) => {
            // Shape only: a keysym name the compositor resolves —
            // airlock holds no table to check it against offline. A
            // padded name trims so the binding never carries the
            // padding into the compositor config.
            let key = value
                .as_str()
                .ok_or(SettingsError::InvalidFile {
                    problem: ALL_WINDOWS_KEY_PROBLEM,
                })?
                .trim();
            if key.is_empty() {
                return Err(SettingsError::InvalidFile {
                    problem: ALL_WINDOWS_KEY_PROBLEM,
                });
            }
            key.to_owned()
        }
    };
    let font_size = match root.get("font_size") {
        None => ui::DEFAULT_FONT_SIZE,
        Some(value) => {
            let size = value.as_integer().ok_or(SettingsError::InvalidFile {
                problem: "`font_size` must be a whole number",
            })?;
            u32::try_from(size)
                .ok()
                .filter(|size| (ui::FONT_SIZE_MIN..=ui::FONT_SIZE_MAX).contains(size))
                .ok_or(SettingsError::InvalidFile {
                    problem: "`font_size` must be between 8 and 24",
                })?
        }
    };
    let Some(entries) = root.get("workspace") else {
        return Ok(Settings {
            theme,
            rows,
            grouping,
            show_workspace,
            layout,
            columns,
            width,
            min_width,
            pair,
            selection,
            font,
            font_size,
            all_windows_key,
            ..Settings::default()
        });
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
    Ok(Settings {
        theme,
        rows,
        grouping,
        show_workspace,
        layout,
        columns,
        width,
        min_width,
        pair,
        selection,
        font,
        font_size,
        all_windows_key,
        workspaces,
    })
}

/// One of the paired color keys, as `0x00RRGGBB`: `None` when the key
/// is absent, a 6-digit hex color (`#` optional, case-insensitive)
/// otherwise. `not_string` and `not_hex` are the key's own problems —
/// the messages name their key; alpha never parses (the 8-digit
/// `#RRGGBBAA` form is `not_hex` by construction).
fn color_key(
    value: Option<&toml::Value>,
    not_string: &'static str,
    not_hex: &'static str,
) -> Result<Option<u32>, SettingsError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let Some(text) = value.as_str() else {
        return Err(SettingsError::InvalidFile {
            problem: not_string,
        });
    };
    Ok(Some(
        hex_rgb(text).ok_or(SettingsError::InvalidFile { problem: not_hex })?,
    ))
}

/// The `width` union's one diagnostic: any bad value — wrong type, a
/// percentage past 100, a zero — names the key and the whole domain.
const WIDTH_PROBLEM: &str =
    "`width` must be \"auto\", a percentage like \"100%\" (1–100), or a pixel count (1 at least)";
/// `min-width`'s twin of [`WIDTH_PROBLEM`].
const MIN_WIDTH_PROBLEM: &str = "`min-width` must be \"auto\", a percentage like \"100%\" (1–100), or a pixel count (1 at least)";

/// Parses one card-width key: `"auto"`, a whole number of pixels, or a
/// percentage like `"100%"` — the percentage 1..=100 (the card is
/// centered, so 100 is the whole output) and the pixel count 1 at
/// least (a floor of zero sizes nothing). A missing key is `auto`.
fn width_key(
    value: Option<&toml::Value>,
    problem: &'static str,
) -> Result<WidthSetting, SettingsError> {
    let Some(value) = value else {
        return Ok(WidthSetting::Auto);
    };
    match value.as_str() {
        Some("auto") => Ok(WidthSetting::Auto),
        Some(percent) => percent
            .strip_suffix('%')
            .and_then(|digits| digits.parse::<u32>().ok())
            .filter(|percent| (1..=100).contains(percent))
            .map(WidthSetting::Percent)
            .ok_or(SettingsError::InvalidFile { problem }),
        None => value
            .as_integer()
            .and_then(|px| u32::try_from(px).ok())
            .filter(|px| *px >= 1)
            .map(WidthSetting::Px)
            .ok_or(SettingsError::InvalidFile { problem }),
    }
}

/// A 6-digit hex color (`#RRGGBB`, `#` optional, case-insensitive) as
/// `0x00RRGGBB`. `None` for anything else.
fn hex_rgb(text: &str) -> Option<u32> {
    let digits = text.trim().strip_prefix('#').unwrap_or(text.trim());
    if digits.len() != 6 || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(digits, 16).ok()
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
#[derive(Debug, PartialEq)]
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
    /// The `card`/`text` pair parses but measures below the contrast
    /// floor; `ratio` is the measured contrast (the floor itself is
    /// [`ui::CONTRAST_FLOOR`]).
    InvalidContrast { ratio: f32 },
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
            SettingsError::InvalidContrast { ratio } => write!(
                f,
                "settings: `card`/`text` contrast is {ratio:.1}:1, below the {:.1}:1 floor",
                ui::CONTRAST_FLOOR
            ),
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
    /// Same bytes as the last check; the loaded settings still apply.
    Unchanged,
    /// New file or new content; read and parse again.
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

/// Resolves the config path and answers "did it change?" with one
/// bounded read, so the daemon can reload cheaply on the gesture path
/// without a watcher dependency.
///
/// The comparison is **content, never metadata** (2026-11-09): an mtime
/// alone misses edits on filesystems with coarse or stale timestamps —
/// VM shares (9p, virtiofs) are the known offenders — and a boolean
/// flip like `grouping = true` → `false` never moves a byte count
/// either. Worse, a filesystem whose `modified()` reads as an error
/// used to count as *absence*, resetting every gesture to defaults.
/// Bytes cannot lie; the config is a few hundred of them, read at
/// session open where icon rasterization already costs more.
pub struct SettingsSource {
    path: Option<PathBuf>,
    last_content: Option<Vec<u8>>,
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
            last_content: None,
        }
    }

    /// Source at an explicit path (tests).
    #[cfg(test)]
    pub(crate) fn at(path: PathBuf) -> Self {
        Self {
            path: Some(path),
            last_content: None,
        }
    }

    /// One-read reload check. Remembers the bytes it reads, so an
    /// unchanged file is cheap to compare and a malformed re-read keeps
    /// last-good settings until the file actually changes again. A
    /// transient I/O error reads as [`Reload::Unchanged`] — the next
    /// check retries, and settings never churn on a flaky read.
    pub fn check(&mut self) -> Reload {
        let Some(path) = self.path.as_deref() else {
            return Reload::NotPresent;
        };
        match read_capped(path) {
            Ok(bytes) => {
                if self.last_content.as_ref() == Some(&bytes) {
                    Reload::Unchanged
                } else {
                    self.last_content = Some(bytes);
                    Reload::Changed
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.last_content = None;
                Reload::NotPresent
            }
            // Any other read failure keeps the baseline: absence was
            // never proven, so the loaded settings stand.
            Err(_) => Reload::Unchanged,
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

/// The file's first [`MAX_CONFIG_BYTES`] bytes — all the change
/// detection needs: every config the parse accepts is smaller, and an
/// oversize file's prefix still moves when its bytes do (the parse
/// keeps rejecting it either way).
fn read_capped(path: &Path) -> std::io::Result<Vec<u8>> {
    let file = fs::File::open(path)?;
    let mut bytes = Vec::new();
    file.take(MAX_CONFIG_BYTES).read_to_end(&mut bytes)?;
    Ok(bytes)
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
                theme: ThemeSetting::Auto,
                rows: ui::DEFAULT_ROWS,
                grouping: false,
                show_workspace: false,
                layout: LayoutSetting::Vertical,
                columns: ColumnsSetting::Auto,
                width: WidthSetting::Auto,
                min_width: WidthSetting::Auto,
                pair: None,
                selection: SelectionSetting::Accent,
                font: crate::text::FontSetting::Default,
                font_size: ui::DEFAULT_FONT_SIZE,
                all_windows_key: "Escape".to_owned(),
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

    // --- The font key ----------------------------------------------------

    #[test]
    fn parse_defaults_font_to_the_follow_when_absent() {
        let settings = Settings::parse(b"").expect("valid");
        assert_eq!(settings.font, crate::text::FontSetting::Default);
    }

    #[test]
    fn parse_reads_a_font_family_name() {
        let settings = Settings::parse(b"font = \"Fira Sans\"\n").expect("valid");
        assert_eq!(
            settings.font,
            crate::text::FontSetting::Family("Fira Sans".to_owned())
        );
        // Any string is valid syntax — resolution is paint-time by
        // design, so even a family nothing installs parses.
        let settings = Settings::parse(b"font = \"Not Installed\"\n").expect("valid");
        assert_eq!(
            settings.font,
            crate::text::FontSetting::Family("Not Installed".to_owned())
        );
    }

    #[test]
    fn parse_rejects_a_non_string_font_with_the_named_problem() {
        let error = Settings::parse(b"font = 12\n").expect_err("wrong type");
        assert_eq!(
            error,
            SettingsError::InvalidFile {
                problem: "`font` must be a font family name (a string)",
            }
        );
        let error = Settings::parse(b"[font]\nfamily = \"x\"\n").expect_err("wrong type");
        assert_eq!(
            error,
            SettingsError::InvalidFile {
                problem: "`font` must be a font family name (a string)",
            }
        );
    }

    #[test]
    fn parse_a_bad_font_beside_valid_keys_fails_the_whole_file() {
        // The `theme`/`rows` half is fine on its own; one bad `font`
        // rejects the file, same as every other key — last-good holds.
        let error = Settings::parse(b"theme = \"dark\"\nrows = 6\nfont = true\n")
            .expect_err("the file fails whole");
        assert!(matches!(error, SettingsError::InvalidFile { .. }));
    }

    // --- The font_size key ----------------------------------------------

    #[test]
    fn parse_defaults_font_size_to_the_twelve_px_pin() {
        assert_eq!(
            Settings::parse(b"").expect("empty").font_size,
            ui::DEFAULT_FONT_SIZE
        );
        let rules_only = Settings::parse(b"[[workspace]]\nname = \"a\"\n").expect("valid");
        assert_eq!(rules_only.font_size, ui::DEFAULT_FONT_SIZE);
        assert_eq!(Settings::default().font_size, 12);
    }

    #[test]
    fn parse_reads_font_size_across_the_accepted_range() {
        for (config, size) in [
            (&b"font_size = 8\n"[..], 8),
            (b"font_size = 12\n", 12),
            (b"font_size = 16\n", 16),
            (b"font_size = 24\n", 24),
        ] {
            let settings = Settings::parse(config).expect("valid font size");
            assert_eq!(
                settings.font_size,
                size,
                "{}",
                String::from_utf8_lossy(config)
            );
        }
    }

    #[test]
    fn parse_rejects_font_size_out_of_range() {
        // Below the floor, above the ceiling, zero, negative, and
        // absurd alike: named, never clamped.
        for config in [
            &b"font_size = 7\n"[..],
            b"font_size = 25\n",
            b"font_size = 0\n",
            b"font_size = -1\n",
            b"font_size = 99999999999\n",
        ] {
            let error = Settings::parse(config).expect_err("out-of-range font size");
            assert_eq!(
                error,
                SettingsError::InvalidFile {
                    problem: "`font_size` must be between 8 and 24"
                },
                "{}",
                String::from_utf8_lossy(config)
            );
        }
    }

    #[test]
    fn parse_rejects_a_non_integer_font_size() {
        for config in [
            &b"font_size = 12.5\n"[..],
            b"font_size = \"12\"\n",
            b"font_size = true\n",
        ] {
            let error = Settings::parse(config).expect_err("non-integer font size");
            assert_eq!(
                error,
                SettingsError::InvalidFile {
                    problem: "`font_size` must be a whole number"
                },
                "{}",
                String::from_utf8_lossy(config)
            );
        }
    }

    #[test]
    fn the_font_size_diagnostic_names_the_accepted_range() {
        // The literals in the parser must track the constants: a re-pin
        // of either bound touches this test.
        assert_eq!((ui::FONT_SIZE_MIN, ui::FONT_SIZE_MAX), (8, 24));
        let error = Settings::parse(b"font_size = 7\n").expect_err("below the floor");
        assert_eq!(
            error.to_string(),
            "settings: `font_size` must be between 8 and 24"
        );
    }

    // --- The all_windows_key (all-windows A2.1, 2026-11-14) --------------

    #[test]
    fn parse_defaults_all_windows_key_to_escape_when_absent() {
        assert_eq!(
            Settings::parse(b"").expect("empty").all_windows_key,
            "Escape"
        );
        let rules_only = Settings::parse(b"[[workspace]]\nname = \"a\"\n").expect("valid");
        assert_eq!(rules_only.all_windows_key, "Escape");
        assert_eq!(Settings::default().all_windows_key, "Escape");
    }

    #[test]
    fn parse_reads_any_non_empty_key_name_verbatim() {
        // The compositor is the keysym authority airlock cannot check
        // offline: canonical X names and anything else non-empty alike
        // parse, with no validation beyond the shape. Whitespace trims —
        // a padded name would bind a padded keysym.
        for (config, key) in [
            (&b"all_windows_key = \"Tab\"\n"[..], "Tab"),
            (b"all_windows_key = \"grave\"\n", "grave"),
            (b"all_windows_key = \"F13\"\n", "F13"),
            (b"all_windows_key = \"  Tab  \"\n", "Tab"),
        ] {
            let settings = Settings::parse(config).expect("valid key name");
            assert_eq!(
                settings.all_windows_key,
                key,
                "{}",
                String::from_utf8_lossy(config)
            );
        }
    }

    #[test]
    fn parse_rejects_a_non_string_all_windows_key_with_the_named_problem() {
        for config in [
            &b"all_windows_key = 1\n"[..],
            b"all_windows_key = true\n",
            b"[all_windows_key]\nname = \"x\"\n",
        ] {
            let error = Settings::parse(config).expect_err("non-string key name");
            assert_eq!(
                error,
                SettingsError::InvalidFile {
                    problem: "`all_windows_key` must be a non-empty key name"
                },
                "{}",
                String::from_utf8_lossy(config)
            );
        }
    }

    #[test]
    fn parse_rejects_an_empty_all_windows_key_with_the_named_problem() {
        // The empty string and one that trims empty: the same named
        // diagnostic, never a silent default.
        for config in [
            &b"all_windows_key = \"\"\n"[..],
            b"all_windows_key = \"   \"\n",
        ] {
            let error = Settings::parse(config).expect_err("empty key name");
            assert_eq!(
                error,
                SettingsError::InvalidFile {
                    problem: "`all_windows_key` must be a non-empty key name"
                },
                "{}",
                String::from_utf8_lossy(config)
            );
        }
    }

    #[test]
    fn a_bad_all_windows_key_fails_the_whole_file() {
        // The last-good contract, same as every key: no partial set.
        let config = b"theme = \"dark\"\nall_windows_key = \"\"\n";
        assert!(Settings::parse(config).is_err());
    }

    #[test]
    fn parse_coexists_all_windows_key_with_the_other_keys() {
        let config =
            b"theme = \"light\"\nall_windows_key = \"grave\"\n\n[[workspace]]\nname = \"code\"\n";
        let settings = Settings::parse(config).expect("valid config");
        assert_eq!(settings.all_windows_key, "grave");
        assert_eq!(settings.theme, ThemeSetting::Light);
        assert_eq!(settings.workspaces.len(), 1);
    }

    #[test]
    fn a_bad_font_size_fails_the_whole_file() {
        // The last-good contract: a bad `font_size` beside valid keys
        // yields no settings at all, never a partial set.
        let config = b"theme = \"dark\"\nfont_size = 40\n\n[[workspace]]\nname = \"code\"\n";
        assert!(Settings::parse(config).is_err());
    }

    #[test]
    fn parse_coexists_font_size_with_font_and_other_keys() {
        let config = b"font = \"Fira Sans\"\nfont_size = 18\ntheme = \"light\"\n";
        let settings = Settings::parse(config).expect("valid config");
        assert_eq!(
            settings.font,
            crate::text::FontSetting::Family("Fira Sans".to_owned())
        );
        assert_eq!(settings.font_size, 18);
        assert_eq!(settings.theme, ThemeSetting::Light);
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

    // --- Theme ------------------------------------------------------------

    #[test]
    fn parse_reads_the_theme_key() {
        for (config, theme) in [
            (&b"theme = \"auto\"\n"[..], ThemeSetting::Auto),
            (b"theme = \"dark\"\n", ThemeSetting::Dark),
            (b"theme = \"light\"\n", ThemeSetting::Light),
        ] {
            let settings = Settings::parse(config).expect("valid theme value");
            assert_eq!(settings.theme, theme);
        }
    }

    #[test]
    fn parse_without_a_theme_key_defaults_to_auto() {
        // An empty file and a rules-only file alike: no `theme`, Auto.
        assert_eq!(
            Settings::parse(b"").expect("empty").theme,
            ThemeSetting::Auto
        );
        let rules_only = Settings::parse(b"[[workspace]]\nname = \"a\"\n").expect("valid");
        assert_eq!(rules_only.theme, ThemeSetting::Auto);
    }

    #[test]
    fn parse_rejects_an_unknown_theme_value() {
        let error = Settings::parse(b"theme = \"blue\"\n").expect_err("unknown value");
        assert_eq!(
            error,
            SettingsError::InvalidFile {
                problem: "`theme` must be one of \"auto\", \"dark\", or \"light\""
            }
        );
    }

    #[test]
    fn parse_rejects_a_non_string_theme() {
        let error = Settings::parse(b"theme = 3\n").expect_err("numeric theme");
        assert_eq!(
            error,
            SettingsError::InvalidFile {
                problem: "`theme` must be a string"
            }
        );
    }

    #[test]
    fn parse_coexists_theme_with_workspace_rules() {
        let config = b"theme = \"light\"\n\n[[workspace]]\nname = \"code\"\nisolated = false\n";
        let settings = Settings::parse(config).expect("both keys");
        assert_eq!(settings.theme, ThemeSetting::Light);
        assert_eq!(settings.workspaces.len(), 1);
    }

    #[test]
    fn parse_reads_the_rows_key() {
        for (config, rows) in [
            (&b"rows = 8\n"[..], 8),
            (b"rows = 3\n", ui::ROWS_MIN),
            (b"rows = 10\n", ui::ROWS_MAX),
            (b"rows = 5\n", 5),
        ] {
            let settings = Settings::parse(config).expect("valid rows value");
            assert_eq!(settings.rows, rows);
        }
    }

    #[test]
    fn parse_without_a_rows_key_defaults_to_the_windowing_budget() {
        // An empty file and a rules-only file alike: no `rows`, the
        // default budget — the no-config card is unchanged.
        assert_eq!(Settings::parse(b"").expect("empty").rows, ui::DEFAULT_ROWS);
        let rules_only = Settings::parse(b"[[workspace]]\nindex = 1\n").expect("rules");
        assert_eq!(rules_only.rows, ui::DEFAULT_ROWS);
        assert_eq!(Settings::default().rows, ui::DEFAULT_ROWS);
    }

    #[test]
    fn parse_rejects_rows_out_of_range() {
        // Below the floor, above the ceiling, zero, negative, and absurd
        // alike: named, never clamped.
        for config in [
            &b"rows = 2\n"[..],
            b"rows = 11\n",
            b"rows = 0\n",
            b"rows = -1\n",
            b"rows = 99999999999\n",
        ] {
            let error = Settings::parse(config).expect_err("out-of-range rows");
            assert_eq!(
                error,
                SettingsError::InvalidFile {
                    problem: "`rows` must be between 3 and 10"
                },
                "{}",
                String::from_utf8_lossy(config)
            );
        }
    }

    #[test]
    fn parse_rejects_a_non_integer_rows() {
        for config in [&b"rows = 8.5\n"[..], b"rows = \"8\"\n", b"rows = true\n"] {
            let error = Settings::parse(config).expect_err("non-integer rows");
            assert_eq!(
                error,
                SettingsError::InvalidFile {
                    problem: "`rows` must be a whole number"
                },
                "{}",
                String::from_utf8_lossy(config)
            );
        }
    }

    #[test]
    fn the_rows_diagnostic_names_the_accepted_range() {
        // The literal in the parser must track the constants: a re-pin
        // of either bound touches this test.
        assert_eq!((ui::ROWS_MIN, ui::ROWS_MAX), (3, 10));
        let error = Settings::parse(b"rows = 1\n").expect_err("below the floor");
        assert_eq!(
            error.to_string(),
            "settings: `rows` must be between 3 and 10"
        );
    }

    #[test]
    fn a_bad_rows_fails_the_whole_file() {
        // The last-good contract: an invalid `rows` beside valid rules
        // yields no settings at all, never a partial set.
        let config = b"rows = 42\n\n[[workspace]]\nname = \"code\"\nisolated = false\n";
        assert!(Settings::parse(config).is_err());
    }

    #[test]
    fn parse_coexists_rows_with_theme_and_workspace_rules() {
        let config = b"theme = \"light\"\nrows = 6\n\n[[workspace]]\nname = \"code\"\n";
        let settings = Settings::parse(config).expect("valid config");
        assert_eq!(settings.theme, ThemeSetting::Light);
        assert_eq!(settings.rows, 6);
        assert_eq!(settings.workspaces.len(), 1);
    }

    // --- Grouping ---------------------------------------------------------

    #[test]
    fn parse_reads_the_grouping_key() {
        for (config, grouping) in [
            (&b"grouping = true\n"[..], true),
            (b"grouping = false\n", false),
        ] {
            let settings = Settings::parse(config).expect("valid grouping value");
            assert_eq!(settings.grouping, grouping);
        }
    }

    #[test]
    fn parse_without_a_grouping_key_defaults_to_false() {
        // An empty file and a rules-only file alike: no `grouping`, the
        // flat list — the no-config card is unchanged.
        assert!(!Settings::parse(b"").expect("empty").grouping);
        let rules_only = Settings::parse(b"[[workspace]]\nname = \"a\"\n").expect("valid");
        assert!(!rules_only.grouping);
        assert!(!Settings::default().grouping);
    }

    #[test]
    fn parse_rejects_a_non_boolean_grouping() {
        // `grouping = 1` and `grouping = "yes"` both name the key in a
        // diagnostic; the whole file fails, so last-good settings hold.
        for config in [&b"grouping = 1\n"[..], b"grouping = \"yes\"\n"] {
            let error = Settings::parse(config).expect_err("non-boolean grouping");
            assert_eq!(
                error,
                SettingsError::InvalidFile {
                    problem: "`grouping` must be a boolean"
                },
                "{}",
                String::from_utf8_lossy(config)
            );
        }
    }

    #[test]
    fn parse_coexists_grouping_with_the_other_keys() {
        let config =
            b"theme = \"light\"\nrows = 6\ngrouping = true\n\n[[workspace]]\nname = \"code\"\n";
        let settings = Settings::parse(config).expect("valid config");
        assert!(settings.grouping);
        assert_eq!(settings.theme, ThemeSetting::Light);
        assert_eq!(settings.rows, 6);
        assert_eq!(settings.workspaces.len(), 1);
    }

    // --- The show_workspace key (workspace-label W1) ----------------------

    #[test]
    fn parse_without_a_show_workspace_key_defaults_to_false() {
        // No key, the label stays off: the no-config card is unchanged.
        assert!(!Settings::parse(b"").expect("empty").show_workspace);
        let rules_only = Settings::parse(b"[[workspace]]\nname = \"a\"\n").expect("valid");
        assert!(!rules_only.show_workspace);
        assert!(!Settings::default().show_workspace);
    }

    #[test]
    fn parse_reads_the_show_workspace_key() {
        let settings = Settings::parse(b"show_workspace = true\n").expect("valid");
        assert!(settings.show_workspace);
    }

    #[test]
    fn parse_rejects_a_non_boolean_show_workspace() {
        for config in [&b"show_workspace = 1\n"[..], b"show_workspace = \"yes\"\n"] {
            let error = Settings::parse(config).expect_err("non-boolean show_workspace");
            assert_eq!(
                error,
                SettingsError::InvalidFile {
                    problem: "`show_workspace` must be a boolean"
                },
                "{}",
                String::from_utf8_lossy(config)
            );
        }
    }

    // --- Scoped cycling (removed rev 5) -----------------------------------

    #[test]
    fn scoped_cycling_is_an_ignored_unknown_key() {
        // Rev 5 removed the setting: a config still carrying it parses
        // to the defaults for every other key — unknown keys are
        // ignored, so old configs never break.
        assert_eq!(
            Settings::parse(b"scoped_cycling = true\n").expect("parses"),
            Settings::default()
        );
        // Beside live keys, too — the unknown rides along silently.
        let config = b"theme = \"light\"\ngrouping = true\nscoped_cycling = true\n";
        let settings = Settings::parse(config).expect("valid config");
        assert_eq!(settings.theme, ThemeSetting::Light);
        assert!(settings.grouping);
        assert_eq!(
            settings,
            Settings {
                theme: ThemeSetting::Light,
                grouping: true,
                ..Settings::default()
            }
        );
    }

    // --- Custom palette ---------------------------------------------------

    #[test]
    fn parse_reads_a_valid_card_text_pair() {
        let config = b"card = \"#1E2126\"\ntext = \"#F2F4F8\"\n";
        let settings = Settings::parse(config).expect("valid pair");
        assert_eq!(settings.pair, Some((0x00_1E_21_26, 0x00_F2_F4_F8)));
    }

    #[test]
    fn parse_accepts_the_pair_without_the_hash_and_case_insensitively() {
        // The digits are the value: `#` optional, case irrelevant.
        let config = b"card = \"1e2126\"\ntext = \"f2F4f8\"\n";
        let settings = Settings::parse(config).expect("valid pair");
        assert_eq!(settings.pair, Some((0x00_1E_21_26, 0x00_F2_F4_F8)));
    }

    #[test]
    fn parse_without_either_color_key_leaves_no_pair() {
        // An empty file and a rules-only file alike: no `card`, no
        // `text` — the `theme` key keeps deciding the card.
        assert_eq!(Settings::parse(b"").expect("empty").pair, None);
        let rules_only = Settings::parse(b"[[workspace]]\nname = \"a\"\n").expect("valid");
        assert_eq!(rules_only.pair, None);
        assert_eq!(Settings::default().pair, None);
    }

    #[test]
    fn parse_rejects_one_color_key_alone() {
        // The keys are a pair: either both or neither.
        for config in [&b"card = \"#1E2126\"\n"[..], b"text = \"#F2F4F8\"\n"] {
            let error = Settings::parse(config).expect_err("pairs only");
            assert_eq!(
                error,
                SettingsError::InvalidFile {
                    problem: "`card` and `text` must be set together"
                },
                "{}",
                String::from_utf8_lossy(config)
            );
        }
    }

    #[test]
    fn parse_rejects_colors_that_are_not_6_hex_digits() {
        // 3/4/8 digits, non-hex — the 8-digit rejection is the alpha
        // guard: a `#RRGGBBAA` value has no path in.
        for (config, key) in [
            (&b"card = \"#F2F\"\ntext = \"#F2F4F8\"\n"[..], "card"),
            (b"card = \"#F2F4\"\ntext = \"#F2F4F8\"\n", "card"),
            (b"card = \"#F2F4F8FF\"\ntext = \"#F2F4F8\"\n", "card"),
            (b"card = \"#GGGGGG\"\ntext = \"#F2F4F8\"\n", "card"),
            (b"card = \"#1E2126\"\ntext = \"#F2F4F8FF\"\n", "text"),
        ] {
            let error = Settings::parse(config).expect_err("not 6 hex digits");
            let SettingsError::InvalidFile { problem } = &error else {
                panic!(
                    "expected InvalidFile, got {error:?} for {}",
                    String::from_utf8_lossy(config)
                );
            };
            assert!(
                problem.contains(key),
                "the problem names its key ({key}): {problem}"
            );
        }
        // The literals, pinned: each key carries its own message.
        assert_eq!(
            Settings::parse(b"card = \"#F2F4F8FF\"\ntext = \"#F2F4F8\"\n"),
            Err(SettingsError::InvalidFile {
                problem: "`card` must be a 6-digit hex color like \"#1E2126\""
            })
        );
        assert_eq!(
            Settings::parse(b"card = \"#1E2126\"\ntext = \"#F2F4F8FF\"\n"),
            Err(SettingsError::InvalidFile {
                problem: "`text` must be a 6-digit hex color like \"#F2F4F8\""
            })
        );
    }

    #[test]
    fn parse_rejects_non_string_colors() {
        assert_eq!(
            Settings::parse(b"card = 3\ntext = \"#F2F4F8\"\n"),
            Err(SettingsError::InvalidFile {
                problem: "`card` must be a string holding a 6-digit hex color"
            })
        );
        for config in [
            &b"card = \"#1E2126\"\ntext = { hex = \"#F2F4F8\" }\n"[..],
            b"card = \"#1E2126\"\ntext = [\"#F2F4F8\"]\n",
        ] {
            assert_eq!(
                Settings::parse(config),
                Err(SettingsError::InvalidFile {
                    problem: "`text` must be a string holding a 6-digit hex color"
                }),
                "{}",
                String::from_utf8_lossy(config)
            );
        }
    }

    #[test]
    fn a_below_floor_pair_rejects_with_the_measured_ratio() {
        // #ADADAD on #FFFFFF measures ~2.24:1; the diagnostic names both
        // keys and the ratio at one decimal.
        let config = b"card = \"#FFFFFF\"\ntext = \"#ADADAD\"\n";
        let error = Settings::parse(config).expect_err("below the floor");
        match &error {
            SettingsError::InvalidContrast { ratio } => {
                assert!((ratio - 2.24).abs() < 0.01, "measured ratio, got {ratio}");
            }
            other => panic!("expected InvalidContrast, got {other:?}"),
        }
        assert_eq!(
            error.to_string(),
            "settings: `card`/`text` contrast is 2.2:1, below the 4.5:1 floor"
        );
    }

    #[test]
    fn the_contrast_floor_boundary_accepts() {
        // #767676 on #FFFFFF measures ~4.54:1 — the WCAG AA boundary
        // gray, a hair over the floor: the floor rejects below, not at.
        let config = b"card = \"#FFFFFF\"\ntext = \"#767676\"\n";
        let settings = Settings::parse(config).expect("just over the floor");
        assert_eq!(settings.pair, Some((0x00_FF_FF_FF, 0x00_76_76_76)));
    }

    #[test]
    fn a_bad_pair_fails_the_whole_file() {
        // The last-good contract: a below-floor pair beside otherwise
        // valid keys yields no settings at all, never a partial set.
        // Top-level keys ride before the `[[workspace]]` table — in
        // TOML, keys after it belong to the entry.
        let config = b"theme = \"light\"\nrows = 6\ngrouping = true\ncard = \"#FFFFFF\"\ntext = \"#ADADAD\"\n\n[[workspace]]\nname = \"code\"\n";
        assert!(Settings::parse(config).is_err());
    }

    // --- Selection band ---------------------------------------------------

    #[test]
    fn parse_reads_the_selection_key() {
        for (config, selection) in [
            (&b"selection = \"accent\"\n"[..], SelectionSetting::Accent),
            (b"selection = \"neutral\"\n", SelectionSetting::Neutral),
        ] {
            let settings = Settings::parse(config).expect("valid selection value");
            assert_eq!(settings.selection, selection);
        }
    }

    #[test]
    fn parse_without_a_selection_key_defaults_to_accent() {
        // The follow is the default: no key, the band carries the
        // desktop's accent. An empty file and a rules-only file alike.
        assert_eq!(
            Settings::parse(b"").expect("empty").selection,
            SelectionSetting::Accent
        );
        let rules_only = Settings::parse(b"[[workspace]]\nname = \"a\"\n").expect("valid");
        assert_eq!(rules_only.selection, SelectionSetting::Accent);
        assert_eq!(Settings::default().selection, SelectionSetting::Accent);
    }

    #[test]
    fn parse_rejects_an_unknown_selection_value() {
        assert_eq!(
            Settings::parse(b"selection = \"blue\"\n"),
            Err(SettingsError::InvalidFile {
                problem: "`selection` must be \"accent\" or \"neutral\""
            })
        );
        assert_eq!(
            Settings::parse(b"selection = 3\n"),
            Err(SettingsError::InvalidFile {
                problem: "`selection` must be a string"
            })
        );
    }

    #[test]
    fn parse_coexists_selection_with_the_pair_and_theme() {
        let config =
            b"theme = \"dark\"\ncard = \"#2A1E30\"\ntext = \"#E8E6E0\"\nselection = \"neutral\"\n";
        let settings = Settings::parse(config).expect("valid config");
        assert_eq!(settings.selection, SelectionSetting::Neutral);
        assert_eq!(settings.pair, Some((0x00_2A_1E_30, 0x00_E8_E6_E0)));
    }

    // --- Layout -----------------------------------------------------------

    #[test]
    fn parse_reads_the_layout_key() {
        for (config, layout) in [
            (&b"layout = \"vertical\"\n"[..], LayoutSetting::Vertical),
            (b"layout = \"horizontal\"\n", LayoutSetting::Horizontal),
        ] {
            let settings = Settings::parse(config).expect("valid layout value");
            assert_eq!(settings.layout, layout);
        }
    }

    #[test]
    fn parse_without_a_layout_key_defaults_to_vertical() {
        // An empty file and a rules-only file alike: no `layout`, the
        // vertical card — the no-config card is unchanged.
        assert_eq!(
            Settings::parse(b"").expect("empty").layout,
            LayoutSetting::Vertical
        );
        let rules_only = Settings::parse(b"[[workspace]]\nname = \"a\"\n").expect("valid");
        assert_eq!(rules_only.layout, LayoutSetting::Vertical);
        assert_eq!(Settings::default().layout, LayoutSetting::Vertical);
    }

    #[test]
    fn parse_rejects_an_unknown_layout_value() {
        assert_eq!(
            Settings::parse(b"layout = \"side\"\n"),
            Err(SettingsError::InvalidFile {
                problem: "`layout` must be \"vertical\" or \"horizontal\""
            })
        );
    }

    #[test]
    fn parse_rejects_a_non_string_layout() {
        assert_eq!(
            Settings::parse(b"layout = 1\n"),
            Err(SettingsError::InvalidFile {
                problem: "`layout` must be a string"
            })
        );
    }

    #[test]
    fn a_bad_layout_fails_the_whole_file() {
        // The last-good contract: an unknown `layout` beside otherwise
        // valid keys yields no settings at all, never a partial set.
        let config = b"theme = \"light\"\nrows = 6\nlayout = \"side\"\n";
        assert!(Settings::parse(config).is_err());
    }

    #[test]
    fn parse_coexists_layout_with_the_other_keys() {
        let config =
            b"theme = \"light\"\nrows = 6\ngrouping = true\nlayout = \"horizontal\"\n\n[[workspace]]\nname = \"code\"\n";
        let settings = Settings::parse(config).expect("valid config");
        assert_eq!(settings.layout, LayoutSetting::Horizontal);
        assert_eq!(settings.theme, ThemeSetting::Light);
        assert_eq!(settings.rows, 6);
        assert_eq!(settings.workspaces.len(), 1);
    }

    // --- Columns ----------------------------------------------------------

    #[test]
    fn parse_reads_the_columns_key() {
        for (config, columns) in [
            (&b"columns = \"auto\"\n"[..], ColumnsSetting::Auto),
            (b"columns = 3\n", ColumnsSetting::Pinned(ui::COLUMNS_MIN)),
            (b"columns = 12\n", ColumnsSetting::Pinned(ui::COLUMNS_MAX)),
            (b"columns = 7\n", ColumnsSetting::Pinned(7)),
        ] {
            let settings = Settings::parse(config).expect("valid columns value");
            assert_eq!(settings.columns, columns);
        }
    }

    #[test]
    fn parse_without_a_columns_key_defaults_to_auto() {
        // An empty file and a rules-only file alike: no `columns`, the
        // responsive band — the no-config card is unchanged.
        assert_eq!(
            Settings::parse(b"").expect("empty").columns,
            ColumnsSetting::Auto
        );
        let rules_only = Settings::parse(b"[[workspace]]\nname = \"a\"\n").expect("valid");
        assert_eq!(rules_only.columns, ColumnsSetting::Auto);
        assert_eq!(Settings::default().columns, ColumnsSetting::Auto);
    }

    #[test]
    fn parse_rejects_columns_out_of_range() {
        // Below the floor, above the ceiling, zero, negative, and
        // absurd alike: named, never clamped.
        for config in [
            &b"columns = 2\n"[..],
            b"columns = 13\n",
            b"columns = 0\n",
            b"columns = -1\n",
            b"columns = 99999999999\n",
        ] {
            assert_eq!(
                Settings::parse(config),
                Err(SettingsError::InvalidFile {
                    problem: "`columns` must be \"auto\" or a whole number between 3 and 12"
                }),
                "{}",
                String::from_utf8_lossy(config)
            );
        }
    }

    #[test]
    fn parse_rejects_columns_shapes_that_are_neither_auto_nor_a_whole_number() {
        // A string that isn't "auto", a float, a bool, an array, a
        // table — the union names its whole accepted shape.
        for config in [
            &b"columns = \"many\"\n"[..],
            b"columns = \"8\"\n",
            b"columns = 8.5\n",
            b"columns = true\n",
            b"columns = [3]\n",
            b"[columns]\ncount = 3\n",
        ] {
            assert_eq!(
                Settings::parse(config),
                Err(SettingsError::InvalidFile {
                    problem: "`columns` must be \"auto\" or a whole number between 3 and 12"
                }),
                "{}",
                String::from_utf8_lossy(config)
            );
        }
    }

    #[test]
    fn the_columns_diagnostic_names_the_accepted_range() {
        // The literal in the parser must track the constants: a re-pin
        // of either bound touches this test (the rows twin).
        assert_eq!((ui::COLUMNS_MIN, ui::COLUMNS_MAX), (3, 12));
        let error = Settings::parse(b"columns = 1\n").expect_err("below the floor");
        assert_eq!(
            error.to_string(),
            "settings: `columns` must be \"auto\" or a whole number between 3 and 12"
        );
    }

    #[test]
    fn a_bad_columns_fails_the_whole_file() {
        // The last-good contract: an out-of-range `columns` beside
        // valid rules yields no settings at all, never a partial set.
        let config = b"columns = 42\n\n[[workspace]]\nname = \"code\"\nisolated = false\n";
        assert!(Settings::parse(config).is_err());
    }

    #[test]
    fn parse_coexists_columns_with_layout() {
        let config = b"layout = \"horizontal\"\ncolumns = 5\n";
        let settings = Settings::parse(config).expect("valid config");
        assert_eq!(settings.layout, LayoutSetting::Horizontal);
        assert_eq!(settings.columns, ColumnsSetting::Pinned(5));
    }

    #[test]
    fn parse_reads_the_width_keys() {
        for (config, width, min_width) in [
            (
                &b"width = \"auto\"\n"[..],
                WidthSetting::Auto,
                WidthSetting::Auto,
            ),
            (
                b"width = \"100%\"\n",
                WidthSetting::Percent(100),
                WidthSetting::Auto,
            ),
            (
                b"width = \"66%\"\n",
                WidthSetting::Percent(66),
                WidthSetting::Auto,
            ),
            (b"width = 640\n", WidthSetting::Px(640), WidthSetting::Auto),
            (
                b"min-width = \"50%\"\n",
                WidthSetting::Auto,
                WidthSetting::Percent(50),
            ),
            (
                b"min-width = 800\n",
                WidthSetting::Auto,
                WidthSetting::Px(800),
            ),
            (
                b"width = \"100%\"\nmin-width = 640\n",
                WidthSetting::Percent(100),
                WidthSetting::Px(640),
            ),
        ] {
            let settings = Settings::parse(config).expect("valid width value");
            assert_eq!(
                (settings.width, settings.min_width),
                (width, min_width),
                "{}",
                String::from_utf8_lossy(config)
            );
        }
    }

    #[test]
    fn parse_without_width_keys_defaults_to_auto() {
        // An empty file and a rules-only file alike: no width keys, the
        // content-sized card — the no-config card is unchanged.
        assert_eq!(
            Settings::parse(b"").expect("empty").width,
            WidthSetting::Auto
        );
        let rules_only = Settings::parse(b"[[workspace]]\nname = \"a\"\n").expect("valid");
        assert_eq!(rules_only.min_width, WidthSetting::Auto);
        assert_eq!(Settings::default().width, WidthSetting::Auto);
        assert_eq!(Settings::default().min_width, WidthSetting::Auto);
    }

    #[test]
    fn parse_rejects_width_values_out_of_domain() {
        // A percentage past 100, a zero of either form, a negative, a
        // quoted pixel count, a float, a bool — named per key, never
        // clamped.
        for (config, problem) in [
            (&b"width = \"101%\"\n"[..], WIDTH_PROBLEM),
            (b"width = \"0%\"\n", WIDTH_PROBLEM),
            (b"width = 0\n", WIDTH_PROBLEM),
            (b"width = -640\n", WIDTH_PROBLEM),
            (b"width = \"640\"\n", WIDTH_PROBLEM),
            (b"width = 640.5\n", WIDTH_PROBLEM),
            (b"width = true\n", WIDTH_PROBLEM),
            (b"min-width = \"200%\"\n", MIN_WIDTH_PROBLEM),
            (b"min-width = 0\n", MIN_WIDTH_PROBLEM),
            (b"min-width = \"auto-ish\"\n", MIN_WIDTH_PROBLEM),
        ] {
            assert_eq!(
                Settings::parse(config),
                Err(SettingsError::InvalidFile { problem }),
                "{}",
                String::from_utf8_lossy(config)
            );
        }
    }

    #[test]
    fn a_bad_width_fails_the_whole_file() {
        // The last-good contract: an out-of-domain `min-width` beside
        // valid rules yields no settings at all, never a partial set.
        let config = b"min-width = \"101%\"\n\n[[workspace]]\nname = \"code\"\nisolated = false\n";
        assert!(Settings::parse(config).is_err());
    }

    #[test]
    fn parse_coexists_width_with_columns() {
        let config = b"columns = 5\nwidth = \"100%\"\n";
        let settings = Settings::parse(config).expect("valid config");
        assert_eq!(settings.columns, ColumnsSetting::Pinned(5));
        assert_eq!(settings.width, WidthSetting::Percent(100));
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
            theme: ThemeSetting::Auto,
            rows: ui::DEFAULT_ROWS,
            grouping: false,
            show_workspace: false,
            layout: LayoutSetting::Vertical,
            columns: ColumnsSetting::Auto,
            width: WidthSetting::Auto,
            min_width: WidthSetting::Auto,
            pair: None,
            selection: SelectionSetting::Accent,
            font: crate::text::FontSetting::Default,
            font_size: ui::DEFAULT_FONT_SIZE,
            all_windows_key: "Escape".to_owned(),
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
            theme: ThemeSetting::Auto,
            rows: ui::DEFAULT_ROWS,
            grouping: false,
            show_workspace: false,
            layout: LayoutSetting::Vertical,
            columns: ColumnsSetting::Auto,
            width: WidthSetting::Auto,
            min_width: WidthSetting::Auto,
            pair: None,
            selection: SelectionSetting::Accent,
            font: crate::text::FontSetting::Default,
            font_size: ui::DEFAULT_FONT_SIZE,
            all_windows_key: "Escape".to_owned(),
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
            theme: ThemeSetting::Auto,
            rows: ui::DEFAULT_ROWS,
            grouping: false,
            show_workspace: false,
            layout: LayoutSetting::Vertical,
            columns: ColumnsSetting::Auto,
            width: WidthSetting::Auto,
            min_width: WidthSetting::Auto,
            pair: None,
            selection: SelectionSetting::Accent,
            font: crate::text::FontSetting::Default,
            font_size: ui::DEFAULT_FONT_SIZE,
            all_windows_key: "Escape".to_owned(),
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
            theme: ThemeSetting::Auto,
            rows: ui::DEFAULT_ROWS,
            grouping: false,
            show_workspace: false,
            layout: LayoutSetting::Vertical,
            columns: ColumnsSetting::Auto,
            width: WidthSetting::Auto,
            min_width: WidthSetting::Auto,
            pair: None,
            selection: SelectionSetting::Accent,
            font: crate::text::FontSetting::Default,
            font_size: ui::DEFAULT_FONT_SIZE,
            all_windows_key: "Escape".to_owned(),
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
            theme: ThemeSetting::Auto,
            rows: ui::DEFAULT_ROWS,
            grouping: false,
            show_workspace: false,
            layout: LayoutSetting::Vertical,
            columns: ColumnsSetting::Auto,
            width: WidthSetting::Auto,
            min_width: WidthSetting::Auto,
            pair: None,
            selection: SelectionSetting::Accent,
            font: crate::text::FontSetting::Default,
            font_size: ui::DEFAULT_FONT_SIZE,
            all_windows_key: "Escape".to_owned(),
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
            theme: ThemeSetting::Auto,
            rows: ui::DEFAULT_ROWS,
            grouping: false,
            show_workspace: false,
            layout: LayoutSetting::Vertical,
            columns: ColumnsSetting::Auto,
            width: WidthSetting::Auto,
            min_width: WidthSetting::Auto,
            pair: None,
            selection: SelectionSetting::Accent,
            font: crate::text::FontSetting::Default,
            font_size: ui::DEFAULT_FONT_SIZE,
            all_windows_key: "Escape".to_owned(),
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
            theme: ThemeSetting::Auto,
            rows: ui::DEFAULT_ROWS,
            grouping: false,
            show_workspace: false,
            layout: LayoutSetting::Vertical,
            columns: ColumnsSetting::Auto,
            width: WidthSetting::Auto,
            min_width: WidthSetting::Auto,
            pair: None,
            selection: SelectionSetting::Accent,
            font: crate::text::FontSetting::Default,
            font_size: ui::DEFAULT_FONT_SIZE,
            all_windows_key: "Escape".to_owned(),
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
    fn check_compares_bytes_not_metadata() {
        // The reload trigger is content (2026-11-09): rewriting
        // identical bytes — a fresh mtime over the old content — stays
        // Unchanged, while a same-length edit, the boolean flip that
        // never moves a byte count or a coarse timestamp, still reports
        // Changed. An mtime-only check missed both classes on
        // coarse-granularity filesystems.
        let dir = TempDir::new("content-based");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, b"grouping = true\n").expect("write");
        let mut source = SettingsSource::at(path.clone());
        assert_eq!(source.check(), Reload::Changed);
        std::fs::write(&path, b"grouping = true\n").expect("identical rewrite");
        assert_eq!(source.check(), Reload::Unchanged);
        std::fs::write(&path, b"grouping = false\n").expect("same-length flip");
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
                theme: ThemeSetting::Auto,
                rows: ui::DEFAULT_ROWS,
                grouping: false,
                show_workspace: false,
                layout: LayoutSetting::Vertical,
                columns: ColumnsSetting::Auto,
                width: WidthSetting::Auto,
                min_width: WidthSetting::Auto,
                pair: None,
                selection: SelectionSetting::Accent,
                font: crate::text::FontSetting::Default,
                font_size: ui::DEFAULT_FONT_SIZE,
                all_windows_key: "Escape".to_owned(),
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
