//! Command discovery: `find` and `help` over commands, aliases, cvars.
//!
//! Donor provenance: `src/console/discovery.ts`
//! (`queryConsoleEntries`, `findConsoleEntries`, `consoleEntryHelp`,
//! `registerDiscoveryCommands`). Entries merge registered commands,
//! aliases, and cvar snapshots with the donor's ASCII-folded precedence
//! and help text.

use qa_core::cmd::ascii_fold;
use qa_core::cvar::CvarRegistry;

use super::commands::{CommandDocumentation, ConsoleCommands};

/// What a discovery entry describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscoveryKind {
    /// Registered console command.
    Command,
    /// Console alias.
    Alias,
    /// Console variable.
    Cvar,
}

impl DiscoveryKind {
    #[must_use]
    const fn name(self) -> &'static str {
        match self {
            Self::Command => "command",
            Self::Alias => "alias",
            Self::Cvar => "cvar",
        }
    }
}

/// One searchable console entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsoleDiscoveryEntry {
    /// Entry name in declaration case.
    pub name: String,
    /// Entry kind.
    pub kind: DiscoveryKind,
    /// One-line summary.
    pub summary: Option<String>,
    /// Usage line.
    pub usage: Option<String>,
    /// Example invocations.
    pub examples: Vec<String>,
    /// Allowed values, when the entry takes an enum.
    pub allowed_values: Option<Vec<String>>,
    /// Alias expansion text.
    pub alias_value: Option<String>,
    /// Current cvar value.
    pub value: Option<String>,
    /// Default cvar value.
    pub reset_value: Option<String>,
    /// Latched cvar value, when one is staged.
    pub latched_value: Option<String>,
}

fn documented(name: &str, documentation: Option<&CommandDocumentation>) -> ConsoleDiscoveryEntry {
    ConsoleDiscoveryEntry {
        name: name.to_string(),
        kind: DiscoveryKind::Command,
        summary: documentation.and_then(|documentation| documentation.summary.clone()),
        usage: documentation.and_then(|documentation| documentation.usage.clone()),
        examples: documentation.map_or_else(Vec::new, |documentation| documentation.examples.clone()),
        allowed_values: documentation.and_then(|documentation| documentation.allowed_values.clone()),
        alias_value: None,
        value: None,
        reset_value: None,
        latched_value: None,
    }
}

/// All console entries: commands, then aliases, then cvars, sorted by name.
#[must_use]
pub fn query_console_entries(commands: &ConsoleCommands, cvars: &CvarRegistry) -> Vec<ConsoleDiscoveryEntry> {
    let mut entries: Vec<ConsoleDiscoveryEntry> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for name in commands.registered_names() {
        seen.push(ascii_fold(&name));
        entries.push(documented(&name, commands.command_documentation(&name)));
    }
    for name in commands.alias_names() {
        if seen.contains(&ascii_fold(&name)) {
            continue;
        }
        seen.push(ascii_fold(&name));
        entries.push(ConsoleDiscoveryEntry {
            name: name.clone(),
            kind: DiscoveryKind::Alias,
            summary: None,
            usage: Some(name.clone()),
            examples: Vec::new(),
            allowed_values: None,
            alias_value: commands.alias_value(&name).map(str::to_string),
            value: None,
            reset_value: None,
            latched_value: None,
        });
    }
    for snapshot in cvars.snapshots(0) {
        if seen.contains(&format!("cvar:{}", snapshot.name)) {
            continue;
        }
        seen.push(format!("cvar:{}", snapshot.name));
        entries.push(ConsoleDiscoveryEntry {
            usage: Some(format!("{} [value]", snapshot.name)),
            value: Some(snapshot.value.clone()),
            reset_value: Some(snapshot.reset_value.clone()),
            latched_value: snapshot.latched_value.clone(),
            name: snapshot.name.clone(),
            kind: DiscoveryKind::Cvar,
            summary: None,
            examples: Vec::new(),
            allowed_values: None,
            alias_value: None,
        });
    }
    entries.sort_by(|left, right| left.name.cmp(&right.name));
    entries
}

/// Entries whose name, summary, or usage contains `text` (ASCII-folded).
#[must_use]
pub fn find_console_entries(
    commands: &ConsoleCommands,
    cvars: &CvarRegistry,
    text: &str,
) -> Vec<ConsoleDiscoveryEntry> {
    let query = ascii_fold(text);
    query_console_entries(commands, cvars)
        .into_iter()
        .filter(|entry| {
            ascii_fold(&format!(
                "{} {} {}",
                entry.name,
                entry.summary.as_deref().unwrap_or(""),
                entry.usage.as_deref().unwrap_or("")
            ))
            .contains(&query)
        })
        .collect()
}

/// Multi-line help for one entry.
#[must_use]
pub fn console_entry_help(entry: &ConsoleDiscoveryEntry) -> Vec<String> {
    let mut lines = vec![
        format!("{} ({})", entry.name, entry.kind.name()),
        entry
            .summary
            .clone()
            .unwrap_or_else(|| "No description registered.".to_string()),
        format!("Usage: {}", entry.usage.as_deref().unwrap_or("not documented")),
    ];
    match entry.kind {
        DiscoveryKind::Cvar => {
            lines.push(format!("Current: {:?}", entry.value.as_deref().unwrap_or("")));
            lines.push(format!("Default: {:?}", entry.reset_value.as_deref().unwrap_or("")));
            if let Some(latched) = &entry.latched_value {
                lines.push(format!("Pending: {latched:?}"));
            }
        }
        DiscoveryKind::Alias => {
            lines.push(format!(
                "Expands to: {}",
                entry.alias_value.as_deref().unwrap_or("").trim_end()
            ));
        }
        DiscoveryKind::Command => {}
    }
    if let Some(allowed) = &entry.allowed_values {
        lines.push(format!("Allowed values: {}", allowed.join(", ")));
    }
    for example in &entry.examples {
        lines.push(format!("Example: {example}"));
    }
    lines
}

/// Register `find` and `help`; returns names for later removal.
///
/// Handlers read [`super::commands::ConsoleCommandServices::discovery_entries`],
/// which the host refreshes before each submitted line (the registry cannot
/// lend itself to its own handlers).
pub fn register_discovery_commands(commands: &mut ConsoleCommands) -> Vec<String> {
    let mut names = Vec::new();
    if commands.register(
        "find",
        move |invocation, services| {
            let query = invocation.argv.get(1).cloned();
            let page_text = invocation.argv.get(2).cloned().unwrap_or_else(|| "1".to_string());
            let page = validate_page(invocation.argv.len(), query.is_some(), &page_text);
            let Some((query, page)) = query.zip(page) else {
                services.print("Usage: find <text> [page]\nExample: find mouse\n");
                return Ok(());
            };
            let needle = ascii_fold(&query);
            let entries: Vec<ConsoleDiscoveryEntry> = services
                .discovery_entries()
                .into_iter()
                .filter(|entry| {
                    ascii_fold(&format!(
                        "{} {} {}",
                        entry.name,
                        entry.summary.as_deref().unwrap_or(""),
                        entry.usage.as_deref().unwrap_or("")
                    ))
                    .contains(&needle)
                })
                .collect();
            let pages = entries.len().div_ceil(30);
            let pages = pages.max(1);
            if page > pages {
                services.print(&format!("Page {page} is out of range; {pages} page(s).\n"));
                return Ok(());
            }
            for entry in entries.iter().skip((page - 1) * 30).take(30) {
                match &entry.summary {
                    Some(summary) => services.print(&format!("{} ({}) - {summary}\n", entry.name, entry.kind.name())),
                    None => services.print(&format!("{} ({})\n", entry.name, entry.kind.name())),
                }
            }
            services.print(&format!(
                "{} match(es), page {page}/{pages}. Use help <name> for details.\n",
                entries.len()
            ));
            if page < pages {
                services.print(&format!("Next page: find {query:?} {}\n", page + 1));
            }
            Ok(())
        },
        Some(CommandDocumentation {
            summary: Some("Search command and setting names and descriptions.".to_string()),
            usage: Some("find <text> [page]".to_string()),
            examples: vec!["find r_".to_string(), "find mouse".to_string()],
            allowed_values: None,
        }),
    ) {
        names.push("find".to_string());
    }
    if commands.register(
        "help",
        move |invocation, services| {
            let name = invocation.argv.get(1).cloned();
            if name.is_none() || invocation.argv.len() != 2 {
                services.print("Usage: help <name>\nUse find <text> to search commands and settings.\n");
                return Ok(());
            }
            let name = name.unwrap_or_default();
            let entries = services.discovery_entries();
            let entry = entries.iter().find(|entry| entry.name == name).or_else(|| {
                entries
                    .iter()
                    .find(|entry| ascii_fold(&entry.name) == ascii_fold(&name))
            });
            match entry {
                None => services.print(&format!(
                    "No command, setting, or alias named {name:?}. Use find <text>.\n"
                )),
                Some(entry) => services.print(&format!("{}\n", console_entry_help(entry).join("\n"))),
            }
            Ok(())
        },
        Some(CommandDocumentation {
            summary: Some("Show usage and available documentation for a command, setting, or alias.".to_string()),
            usage: Some("help <name>".to_string()),
            examples: vec!["help echo".to_string()],
            allowed_values: None,
        }),
    ) {
        names.push("help".to_string());
    }
    names
}

fn validate_page(argc: usize, has_query: bool, page_text: &str) -> Option<usize> {
    if !has_query || argc > 3 {
        return None;
    }
    if page_text.is_empty() || !page_text.bytes().all(|byte| byte.is_ascii_digit()) || page_text.starts_with('0') {
        return None;
    }
    let page: usize = page_text.parse().ok()?;
    if page < 1 {
        return None;
    }
    Some(page)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::commands::ConsoleCommandServices;
    use qa_core::cmd::{Dialect, TextMode};

    struct Fixture {
        printed: Vec<String>,
        snapshot: Vec<ConsoleDiscoveryEntry>,
    }

    impl ConsoleCommandServices for Fixture {
        fn print(&mut self, text: &str) {
            self.printed.push(text.to_string());
        }

        fn forward_to_server(&mut self, _line: &str) {}
        fn toggle_console(&mut self) {}
        fn clear_console(&mut self) {}
        fn message_mode(&mut self, _team: bool) {}
        fn can_chat(&self) -> bool {
            false
        }

        fn console_dump(&self) -> String {
            String::new()
        }

        fn write_file(&mut self, _path: &str, _contents: &str) -> Result<(), String> {
            Ok(())
        }

        fn configuration_text(&mut self, _argv: &[String]) -> String {
            String::new()
        }

        fn map_name(&self) -> String {
            String::new()
        }

        fn discovery_entries(&self) -> Vec<ConsoleDiscoveryEntry> {
            self.snapshot.clone()
        }
    }

    #[test]
    fn queries_commands_aliases_and_cvars() {
        let mut commands = ConsoleCommands::new();
        commands.register("status", |_, _| Ok(()), None);
        commands.set_alias("s", "status");
        let mut cvars = CvarRegistry::new(Dialect::Q3);
        cvars.register("sensitivity", "5", 0).unwrap();
        let entries = query_console_entries(&commands, &cvars);
        let kinds: Vec<DiscoveryKind> = entries.iter().map(|entry| entry.kind).collect();
        assert!(kinds.contains(&DiscoveryKind::Command));
        assert!(kinds.contains(&DiscoveryKind::Alias));
        assert!(kinds.contains(&DiscoveryKind::Cvar));
        let found = find_console_entries(&commands, &cvars, "sens");
        assert_eq!(found.len(), 1);
        let help = console_entry_help(&found[0]);
        assert!(help.iter().any(|line| line.contains("Current:")));
    }

    #[test]
    fn find_pages_and_help_resolves_names() {
        let mut commands = ConsoleCommands::new();
        for index in 0..35 {
            let name = format!("probe{index:02}");
            commands.register(&name.clone(), move |_, _| Ok(()), None);
        }
        let names = register_discovery_commands(&mut commands);
        assert_eq!(names, vec!["find".to_string(), "help".to_string()]);
        let mut cvars = CvarRegistry::new(Dialect::Q3);
        let snapshot = query_console_entries(&commands, &cvars);
        let mut services = Fixture {
            printed: Vec::new(),
            snapshot,
        };
        commands
            .execute("find", Dialect::Q3, TextMode::Source, &mut cvars, &mut services)
            .unwrap();
        assert!(services.printed[0].starts_with("Usage: find"));
        commands
            .execute("help probe01", Dialect::Q3, TextMode::Source, &mut cvars, &mut services)
            .unwrap();
        assert!(services.printed.last().unwrap().contains("probe01 (command)"));
        commands
            .execute("help missing", Dialect::Q3, TextMode::Source, &mut cvars, &mut services)
            .unwrap();
        assert!(services.printed.last().unwrap().contains("No command"));
    }
}
