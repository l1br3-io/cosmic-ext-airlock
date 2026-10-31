//! Command-line parsing for the `airlock` binary.

/// A subcommand parsed from the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Run the switcher daemon in the foreground.
    Daemon,
    /// Spawn stub: ask the daemon to cycle to the next window.
    AltTab,
    /// Spawn stub: ask the daemon to cycle to the previous window.
    ShiftAltTab,
    /// Spawn stub: ask the daemon to switch to the previous window with no
    /// overlay at all.
    QuickToggle,
    /// Take over — or with `--uninstall`, restore — the Alt+Tab keybind.
    Install { uninstall: bool },
    /// Debug: print toplevels, workspaces, and the active workspace.
    List,
}

/// Why argument parsing failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// No subcommand was given.
    MissingSubcommand,
    /// The first argument is not a known subcommand.
    Unknown(String),
    /// A subcommand was followed by an argument it does not take.
    Unexpected(String),
}

/// Parses subcommand arguments, excluding the program name.
///
/// # Errors
///
/// Returns [`ParseError::MissingSubcommand`] for an empty argument list,
/// [`ParseError::Unknown`] when the first argument is not a subcommand, and
/// [`ParseError::Unexpected`] for any argument a subcommand does not take.
pub fn parse(args: &[String]) -> Result<Command, ParseError> {
    let first = args.first().ok_or(ParseError::MissingSubcommand)?;
    let rest = &args[1..];
    match first.as_str() {
        "daemon" => no_extra_args(rest).map(|()| Command::Daemon),
        "alt-tab" => no_extra_args(rest).map(|()| Command::AltTab),
        "shift-alt-tab" => no_extra_args(rest).map(|()| Command::ShiftAltTab),
        "quick-toggle" => no_extra_args(rest).map(|()| Command::QuickToggle),
        "install" => {
            let (uninstall, rest) = match rest.first().map(String::as_str) {
                Some("--uninstall") => (true, &rest[1..]),
                _ => (false, rest),
            };
            if let [extra, ..] = rest {
                return Err(ParseError::Unexpected(extra.clone()));
            }
            Ok(Command::Install { uninstall })
        }
        "list" => no_extra_args(rest).map(|()| Command::List),
        unknown => Err(ParseError::Unknown(unknown.to_string())),
    }
}

fn no_extra_args(rest: &[String]) -> Result<(), ParseError> {
    match rest {
        [] => Ok(()),
        [extra, ..] => Err(ParseError::Unexpected(extra.clone())),
    }
}

/// Usage text, printed to stderr when parsing fails.
pub fn usage() -> &'static str {
    "usage: airlock <command>

commands:
  daemon                    run the switcher daemon (foreground)
  alt-tab                   switch to the next window on this workspace
  shift-alt-tab             switch to the previous window on this workspace
  quick-toggle              switch to the previous window without the overlay
  install [--uninstall]     take over (or restore) the Alt+Tab keybind
  list                      print toplevels, workspaces, and the active workspace
"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_daemon() {
        assert_eq!(parse(&args(&["daemon"])), Ok(Command::Daemon));
    }

    #[test]
    fn parses_alt_tab() {
        assert_eq!(parse(&args(&["alt-tab"])), Ok(Command::AltTab));
    }

    #[test]
    fn parses_shift_alt_tab() {
        assert_eq!(parse(&args(&["shift-alt-tab"])), Ok(Command::ShiftAltTab));
    }

    #[test]
    fn parses_list() {
        assert_eq!(parse(&args(&["list"])), Ok(Command::List));
    }

    #[test]
    fn parses_quick_toggle() {
        assert_eq!(parse(&args(&["quick-toggle"])), Ok(Command::QuickToggle));
    }

    #[test]
    fn parses_install_without_flags() {
        assert_eq!(
            parse(&args(&["install"])),
            Ok(Command::Install { uninstall: false })
        );
    }

    #[test]
    fn parses_install_with_uninstall_flag() {
        assert_eq!(
            parse(&args(&["install", "--uninstall"])),
            Ok(Command::Install { uninstall: true })
        );
    }

    #[test]
    fn empty_args_report_missing_subcommand() {
        assert_eq!(parse(&args(&[])), Err(ParseError::MissingSubcommand));
    }

    #[test]
    fn unknown_first_token_reports_unknown() {
        assert_eq!(
            parse(&args(&["frobnicate"])),
            Err(ParseError::Unknown("frobnicate".to_string()))
        );
    }

    #[test]
    fn flag_like_first_token_reports_unknown() {
        assert_eq!(
            parse(&args(&["--help"])),
            Err(ParseError::Unknown("--help".to_string()))
        );
    }

    #[test]
    fn extra_arg_after_daemon_reports_unexpected() {
        assert_eq!(
            parse(&args(&["daemon", "now"])),
            Err(ParseError::Unexpected("now".to_string()))
        );
    }

    #[test]
    fn extra_arg_after_quick_toggle_reports_unexpected() {
        assert_eq!(
            parse(&args(&["quick-toggle", "now"])),
            Err(ParseError::Unexpected("now".to_string()))
        );
    }

    #[test]
    fn uninstall_flag_after_alt_tab_reports_unexpected() {
        assert_eq!(
            parse(&args(&["alt-tab", "--uninstall"])),
            Err(ParseError::Unexpected("--uninstall".to_string()))
        );
    }

    #[test]
    fn unknown_flag_after_install_reports_unexpected() {
        assert_eq!(
            parse(&args(&["install", "--remove"])),
            Err(ParseError::Unexpected("--remove".to_string()))
        );
    }

    #[test]
    fn second_extra_arg_after_install_reports_unexpected() {
        assert_eq!(
            parse(&args(&["install", "--uninstall", "extra"])),
            Err(ParseError::Unexpected("extra".to_string()))
        );
    }

    #[test]
    fn usage_lists_every_subcommand() {
        let usage = usage();
        for name in [
            "daemon",
            "alt-tab",
            "shift-alt-tab",
            "quick-toggle",
            "install",
            "list",
        ] {
            assert!(usage.contains(name), "usage is missing `{name}`");
        }
    }
}
