//! Binary entry dispatch shared by `qa-muse` and `qa-dedicated`.
//!
//! Donor provenance: `src/main.ts` (command branches, dedicated vs
//! windowed assembly, quit propagation). Signal handling stays with the
//! host: the loop checks [`Application::is_finished`](crate::application::Application::is_finished)
//! between frames, and the default SIGINT/SIGTERM disposition terminates a
//! headless run.

use std::io::Write;

use qa_client::render::NullRenderer;

use crate::application::Application;
use crate::error::AppError;
use crate::options::{parse_application_command, ApplicationCommand, HELP, WEAPON_BEHAVIOR_HELP};
use crate::startup::StartupConfig;

/// Run the application command line, returning the process exit code.
pub fn run(argv: &[String], stdout: &mut dyn Write, stderr: &mut dyn Write, version: &str) -> i32 {
    match run_inner(argv, stdout, version) {
        Ok(()) => 0,
        Err(error) => {
            let _ = writeln!(stderr, "qa-muse: {error}");
            1
        }
    }
}

fn run_inner(argv: &[String], stdout: &mut dyn Write, version: &str) -> Result<(), AppError> {
    match parse_application_command(argv)? {
        ApplicationCommand::Help => {
            let _ = stdout.write_all(HELP.as_bytes());
            Ok(())
        }
        ApplicationCommand::Version => {
            let _ = writeln!(stdout, "Quake Anthology {version}");
            Ok(())
        }
        ApplicationCommand::WeaponBehavior { command } => {
            if matches!(command, crate::options::WeaponBehaviorTool::Help) {
                let _ = stdout.write_all(WEAPON_BEHAVIOR_HELP.as_bytes());
                return Ok(());
            }
            Err(AppError::ToolUnavailable)
        }
        ApplicationCommand::ListContent { corpus_root } => {
            list_content(&corpus_root, stdout);
            Ok(())
        }
        ApplicationCommand::Run { options } | ApplicationCommand::Menu { options } => {
            let config = StartupConfig::from_options(&options)?;
            let mut application = Application::open(&config, NullRenderer::new())?;
            let stats = application.run()?;
            let _ = writeln!(
                stdout,
                "Ran {} host frames, {} server ticks, {} entities ({} render frames)",
                stats.frames, stats.ticks, stats.entities, stats.render_frames
            );
            Ok(())
        }
    }
}

/// List the corpus-root directory entries (provisional: catalog discovery
/// is not ported yet, so this reports raw directory names).
fn list_content(corpus_root: &str, stdout: &mut dyn Write) {
    let _ = writeln!(stdout, "corpus root: {corpus_root}");
    let entries = std::fs::read_dir(corpus_root).map(|entries| {
        let mut names: Vec<String> = entries
            .filter_map(|entry| entry.ok())
            .map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                    format!("{name}/")
                } else {
                    name
                }
            })
            .collect();
        names.sort();
        names
    });
    match entries {
        Ok(names) => {
            for name in names {
                let _ = writeln!(stdout, "{name}");
            }
        }
        Err(_) => {
            let _ = writeln!(stdout, "(missing or unreadable; catalog discovery is not ported yet)");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_text(argv: &[&str]) -> (i32, String, String) {
        let owned: Vec<String> = argv.iter().map(|arg| (*arg).to_string()).collect();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = run(&owned, &mut stdout, &mut stderr, "0.1.0");
        (
            code,
            String::from_utf8(stdout).unwrap(),
            String::from_utf8(stderr).unwrap(),
        )
    }

    #[test]
    fn help_and_version_branches() {
        let (code, stdout, _) = run_text(&["--help"]);
        assert_eq!(code, 0);
        assert!(stdout.starts_with("Quake\n\nUsage: qa-muse"));
        let (code, stdout, _) = run_text(&["--version"]);
        assert_eq!(code, 0);
        assert_eq!(stdout, "Quake Anthology 0.1.0\n");
    }

    #[test]
    fn headless_run_reports_stats() {
        let (code, stdout, _) = run_text(&["--dedicated", "--movement", "q3", "--frames", "4"]);
        assert_eq!(code, 0);
        assert!(stdout.contains("4 host frames"), "{stdout}");
        assert!(stdout.contains("server ticks"), "{stdout}");
    }

    #[test]
    fn errors_exit_nonzero() {
        let (code, _, stderr) = run_text(&["--bogus", "x"]);
        assert_eq!(code, 1);
        assert!(stderr.contains("Unknown option"), "{stderr}");
        let (code, _, stderr) = run_text(&["--bogus"]);
        assert_eq!(code, 1);
        assert!(stderr.contains("Missing value"), "{stderr}");
        let (code, _, stderr) = run_text(&["weapon-behavior", "inspect", "pkg"]);
        assert_eq!(code, 1);
        assert!(stderr.contains("not ported yet"), "{stderr}");
        let (code, stdout, _) = run_text(&["weapon-behavior", "--help"]);
        assert_eq!(code, 0);
        assert!(stdout.contains("declare-qvm"), "{stdout}");
    }

    #[test]
    fn list_content_reports_corpus_root() {
        let (code, stdout, _) = run_text(&["--list-content", "--content-root", "/nonexistent-qa-muse"]);
        assert_eq!(code, 0);
        assert!(stdout.contains("corpus root: /nonexistent-qa-muse"), "{stdout}");
        assert!(stdout.contains("not ported yet"), "{stdout}");
    }
}
