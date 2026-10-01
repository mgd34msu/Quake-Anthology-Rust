//! Binary entry dispatch shared by `qa-muse` and `qa-dedicated`.
//!
//! Donor provenance: `src/main.ts` (command branches, dedicated vs
//! windowed assembly, quit propagation). Signal handling stays with the
//! host: the loop checks [`Application::is_finished`](crate::application::Application::is_finished)
//! between frames, and the default SIGINT/SIGTERM disposition terminates a
//! headless run.

use std::io::Write;

use qa_client::render::NullRenderer;

use qa_content::catalog::{discover_installed_content, DiscoverContentOptions, ProductAvailability};

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

/// List installed content discovered beneath the corpus root.
fn list_content(corpus_root: &str, stdout: &mut dyn Write) {
    let _ = writeln!(stdout, "corpus root: {corpus_root}");
    match discover_installed_content(&DiscoverContentOptions::new(corpus_root.into())) {
        Ok(catalog) => {
            for product in &catalog.products {
                let status = match &product.availability {
                    ProductAvailability::Installed => "installed".to_string(),
                    ProductAvailability::Missing { requirements } => {
                        format!("missing: {}", requirements.join(", "))
                    }
                    ProductAvailability::Unresolved { reason } => format!("unresolved: {reason}"),
                };
                let _ = writeln!(stdout, "{} [{}]", product.expectation.id, status);
                for archive in &product.archives {
                    let _ = writeln!(stdout, "  {}", archive.path);
                }
            }
        }
        Err(error) => {
            let _ = writeln!(stdout, "(catalog discovery failed: {error})");
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
    fn list_content_reports_catalog_products() {
        let root = std::env::temp_dir().join("qa-muse-list-content");
        let _ = std::fs::create_dir_all(&root);
        let root = root.to_string_lossy().into_owned();
        let (code, stdout, _) = run_text(&["--list-content", "--content-root", root.as_str()]);
        assert_eq!(code, 0);
        assert!(stdout.contains(format!("corpus root: {root}").as_str()), "{stdout}");
        assert!(stdout.contains("q1-classic-id1"), "{stdout}");
        assert!(!stdout.contains("not ported yet"), "{stdout}");
    }
}
