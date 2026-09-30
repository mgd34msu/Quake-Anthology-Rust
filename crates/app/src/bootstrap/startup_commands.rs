//! Startup `+command` parsing and phasing.
//!
//! Donor: `src/app/bootstrap/startup-commands.ts`.
//! `CommandDialect` reuses `qa_core::cmd::Dialect`; text helpers reuse
//! `qa_core::cmd`.

use qa_core::cmd::{
    ascii_fold, command_separator_offset, source_command_text, tokenize_command, CmdError, Dialect,
    TextMode,
};
use thiserror::Error;

/// Startup command failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum StartupCommandsError {
    /// An operand needs quoting or a cfg file.
    #[error("Startup argument cannot contain quotes or line breaks; use a quoted +command batch or exec a cfg file.")]
    BadOperand,
    /// Expected `+command`.
    #[error("Expected +command")]
    ExpectedCommand,
    /// Commands must be split across `+command` arguments.
    #[error("Use a separate +command for each startup command.")]
    NeedsSeparate,
    /// Ordered `+connect` is unsupported.
    #[error("Ordered +connect startup is unsupported; use --connect-q1, --connect-qw, --connect-q2 or --connect-q3.")]
    OrderedConnect,
    /// Filesystem `+set` selection is unsupported.
    #[error("Filesystem +set selection is unsupported; use --game with an installed catalog product and --content-root or --user-content-root.")]
    FilesystemSet,
    /// Command text failure.
    #[error("{0}")]
    Cmd(#[from] CmdError),
}

/// Parsed `+command` with the last consumed argv index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupCommand {
    /// Command text.
    pub text: String,
    /// Last consumed argv index.
    pub end: usize,
}

fn operand(value: &str) -> Result<String, StartupCommandsError> {
    source_command_text(value)?;
    if value.contains(['"', '\r', '\n', '\0']) {
        return Err(StartupCommandsError::BadOperand);
    }
    Ok(format!("\"{value}\""))
}

/// Read one `+command` batch starting at `index`.
pub fn read_startup_command(
    argv: &[String],
    index: usize,
) -> Result<StartupCommand, StartupCommandsError> {
    let Some(first) = argv.get(index) else {
        return Err(StartupCommandsError::ExpectedCommand);
    };
    let Some(rest) = first.strip_prefix('+') else {
        return Err(StartupCommandsError::ExpectedCommand);
    };
    if rest.is_empty() {
        return Err(StartupCommandsError::ExpectedCommand);
    }
    let mut text = rest.to_owned();
    let mut end = index;
    while end + 1 < argv.len() {
        let next = &argv[end + 1];
        if next.starts_with('+') || next.starts_with("--") {
            break;
        }
        text.push(' ');
        text.push_str(&operand(next)?);
        end += 1;
    }
    source_command_text(&text)?;
    // The Rust separator offset is in bytes and the donor offset is in
    // UTF-16 units, but both equal the text length exactly when no
    // separator is present, so the comparison is equivalent.
    if text.contains(['\r', '\n', '\0']) || command_separator_offset(&text, Dialect::Q3) < text.len() {
        return Err(StartupCommandsError::NeedsSeparate);
    }
    let tokens = tokenize_command(&text, Dialect::Q3, TextMode::Source)?.argv;
    if tokens.is_empty() {
        return Err(StartupCommandsError::ExpectedCommand);
    }
    if ascii_fold(tokens.first().map_or("", String::as_str)) == "connect" {
        return Err(StartupCommandsError::OrderedConnect);
    }
    if ascii_fold(tokens.first().map_or("", String::as_str)) == "set"
        && ["game", "fs_game", "basedir", "cddir", "fs_basepath", "fs_homepath", "fs_cdpath"]
            .contains(&ascii_fold(tokens.get(1).map_or("", String::as_str)).as_str())
    {
        return Err(StartupCommandsError::FilesystemSet);
    }
    Ok(StartupCommand { text, end })
}

/// Whether any startup line loads a world.
pub fn startup_requests_world(lines: &[String]) -> Result<bool, StartupCommandsError> {
    for line in lines {
        let argv = tokenize_command(line, Dialect::Q3, TextMode::Source)?.argv;
        let head = argv.first().map_or("", String::as_str).to_lowercase();
        if ["map", "devmap", "spmap", "spdevmap"].contains(&head.as_str()) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// A captured `set` variable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupCommandVariable {
    /// Variable name.
    pub name: String,
    /// Variable value.
    pub value: String,
}

/// Phased startup commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupCommandPhases {
    /// Early (Q2 `set`) commands.
    pub early: String,
    /// Late commands.
    pub late: String,
    /// All stuffed commands, including the safe line.
    pub stuffed: String,
    /// Whether a safe line was present.
    pub safe: bool,
    /// Captured Q3 `set` variables.
    pub variables: Vec<StartupCommandVariable>,
}

/// Split startup lines into early/late phases for a dialect.
pub fn startup_command_phases(
    lines: &[String],
    dialect: Dialect,
) -> Result<StartupCommandPhases, StartupCommandsError> {
    let mut safe_index: Option<usize> = None;
    if dialect == Dialect::Q3 {
        for (index, line) in lines.iter().enumerate() {
            let argv = tokenize_command(line, dialect, TextMode::Source)?.argv;
            if ["safe", "cvar_restart"].contains(&ascii_fold(argv.first().map_or("", String::as_str)).as_str()) {
                safe_index = Some(index);
                break;
            }
        }
    }
    let mut early = Vec::new();
    let mut late = Vec::new();
    let mut variables = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if Some(index) == safe_index {
            continue;
        }
        let argv = tokenize_command(line, dialect, TextMode::Source)?.argv;
        let set = argv.first().is_some_and(|head| head == "set");
        if dialect == Dialect::Q3 && set {
            variables.push(StartupCommandVariable {
                name: argv.get(1).cloned().unwrap_or_default(),
                value: argv.get(2).cloned().unwrap_or_default(),
            });
        }
        if matches!(dialect, Dialect::Q2Classic | Dialect::Q2Rerelease) && set {
            early.push(line.clone());
        }
        if dialect == Dialect::Q3 || !set || dialect.is_q1() {
            late.push(line.clone());
        }
    }
    let text = |commands: &[String]| {
        if commands.is_empty() {
            String::new()
        } else {
            format!("{}\n", commands.join("\n"))
        }
    };
    Ok(StartupCommandPhases {
        early: text(&early),
        late: text(&late),
        stuffed: text(lines),
        variables,
        safe: safe_index.is_some(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn read_command_batches_operands() {
        let args = argv(&["+map", "q3tourney6", "+set", "name", "x"]);
        let first = read_startup_command(&args, 0).unwrap();
        assert_eq!(first.text, "map \"q3tourney6\"");
        assert_eq!(first.end, 1);
        let second = read_startup_command(&args, 2).unwrap();
        assert_eq!(second.text, "set \"name\" \"x\"");
        assert_eq!(second.end, 4);
    }

    #[test]
    fn read_command_stops_at_flags() {
        let args = argv(&["+map", "--connect-q3", "x"]);
        let command = read_startup_command(&args, 0).unwrap();
        assert_eq!(command.text, "map");
        assert_eq!(command.end, 0);
    }

    #[test]
    fn read_command_rejects_bad_inputs() {
        assert_eq!(
            read_startup_command(&argv(&["map"]), 0),
            Err(StartupCommandsError::ExpectedCommand)
        );
        assert_eq!(
            read_startup_command(&argv(&["+"]), 0),
            Err(StartupCommandsError::ExpectedCommand)
        );
        assert_eq!(
            read_startup_command(&argv(&["+map", "a\"b"]), 1 - 1),
            Err(StartupCommandsError::BadOperand)
        );
        assert_eq!(
            read_startup_command(&argv(&["+map a; quit"]), 0),
            Err(StartupCommandsError::NeedsSeparate)
        );
        assert_eq!(
            read_startup_command(&argv(&["+connect", "x"]), 0),
            Err(StartupCommandsError::OrderedConnect)
        );
        assert_eq!(
            read_startup_command(&argv(&["+set", "fs_game", "x"]), 0),
            Err(StartupCommandsError::FilesystemSet)
        );
        // Case-insensitive command, exact `set` match for capture below.
        assert!(read_startup_command(&argv(&["+SET", "name", "x"]), 0).is_ok());
    }

    #[test]
    fn requests_world_matches_map_commands() {
        let yes = argv(&["set name x", "devmap q3tourney6"]);
        assert!(startup_requests_world(&yes).unwrap());
        let no = argv(&["set name x", "vstr next"]);
        assert!(!startup_requests_world(&no).unwrap());
    }

    #[test]
    fn phases_split_q3_and_q2() {
        let lines = argv(&["set name x", "safe", "map q3tourney6"]);
        let q3 = startup_command_phases(&lines, Dialect::Q3).unwrap();
        assert!(q3.safe);
        assert_eq!(q3.late, "set name x\nmap q3tourney6\n");
        assert_eq!(q3.stuffed, "set name x\nsafe\nmap q3tourney6\n");
        assert_eq!(
            q3.variables,
            vec![StartupCommandVariable { name: "name".to_owned(), value: "x".to_owned() }]
        );
        let q2 = startup_command_phases(&lines, Dialect::Q2Classic).unwrap();
        assert!(!q2.safe);
        assert_eq!(q2.early, "set name x\n");
        assert_eq!(q2.late, "safe\nmap q3tourney6\n");
        let q1 = startup_command_phases(&lines, Dialect::Q1Netquake).unwrap();
        assert_eq!(q1.early, "");
        assert_eq!(q1.late, "set name x\nsafe\nmap q3tourney6\n");
    }
}
