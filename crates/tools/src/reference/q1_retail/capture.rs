//! Retail Q1 observation under Wine (donor `tools/reference/q1-retail/capture.ts`).
//!
//! Stages the Steam WinQuake executable with corpus PAKs into an isolated
//! directory, drives a scripted demo/save/screenshot schedule under a
//! private Xvfb display, and records the console plus output identities. The
//! live orchestration needs the retail installation and Proton runtime; the
//! schedule text, command runner, and check evaluation are pure and tested.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::error::ToolsError;
use crate::fsutil;
use crate::json::Json;
use crate::reference::environment::{corpus_root, identify_file, quake_typescript_root};
use crate::reference::q2_native::process::{start_observed, ObservedProcess};
use crate::reference::schema::FileIdentity;
use crate::reference::steam::steam_common_path;
use crate::sys::{kill_process, SIGKILL};
use crate::time::now_iso;

/// A finished file-logged command run.
#[derive(Debug, Clone)]
pub struct CommandRun {
    /// Argument vector.
    pub command: Vec<String>,
    /// Working directory.
    pub cwd: String,
    /// Replaced environment in caller order.
    pub environment: Vec<(String, String)>,
    /// ISO-8601 start time.
    pub started_at: String,
    /// Elapsed milliseconds.
    pub duration_ms: f64,
    /// Child pid.
    pub pid: u32,
    /// Exit code (`None` when killed by a signal).
    pub exit_code: Option<i32>,
    /// Whether the watchdog fired.
    pub timed_out: bool,
    /// Captured standard output.
    pub stdout: String,
    /// Captured standard error.
    pub stderr: String,
}

impl CommandRun {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("command".to_owned(), Json::array(self.command.iter().map(Json::string).collect())),
            ("cwd".to_owned(), Json::string(&self.cwd)),
            (
                "environment".to_owned(),
                Json::object(self.environment.iter().map(|(key, value)| (key.clone(), Json::string(value))).collect()),
            ),
            ("startedAt".to_owned(), Json::string(&self.started_at)),
            ("durationMs".to_owned(), Json::float(self.duration_ms)),
            ("pid".to_owned(), Json::uint(u64::from(self.pid))),
            ("exitCode".to_owned(), self.exit_code.map_or(Json::Null, |code| Json::int(i64::from(code)))),
            ("timedOut".to_owned(), Json::boolean(self.timed_out)),
            ("stdout".to_owned(), Json::string(&self.stdout)),
            ("stderr".to_owned(), Json::string(&self.stderr)),
        ])
    }
}

/// Run a command with stdout/stderr redirected to per-run log files.
pub fn run_command(
    argv: &[String],
    cwd: &str,
    environment: &[(String, String)],
    timeout_ms: u64,
) -> Result<CommandRun, ToolsError> {
    let started_at = now_iso();
    let start = Instant::now();
    let log = format!("{cwd}/process-{}", started_at.replace(':', "-"));
    let (program, args) = argv.split_first().ok_or_else(|| ToolsError::invalid("Cannot run an empty command"))?;
    let mut child_command = std::process::Command::new(program);
    child_command.args(args).current_dir(cwd).env_clear().envs(environment.iter().cloned());
    let stdout_log = std::fs::File::create(format!("{log}.stdout.txt"))
        .map_err(|error| ToolsError::io(format!("creating {log}.stdout.txt"), error))?;
    let stderr_log = std::fs::File::create(format!("{log}.stderr.txt"))
        .map_err(|error| ToolsError::io(format!("creating {log}.stderr.txt"), error))?;
    child_command.stdin(Stdio::null()).stdout(Stdio::from(stdout_log)).stderr(Stdio::from(stderr_log));
    let mut child =
        child_command.spawn().map_err(|error| ToolsError::io(format!("spawning {}", argv.join(" ")), error))?;
    let pid = child.id();
    let done = Arc::new(AtomicBool::new(false));
    let timed_out = Arc::new(AtomicBool::new(false));
    let watchdog_done = Arc::clone(&done);
    let watchdog_flag = Arc::clone(&timed_out);
    let watchdog = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        while Instant::now() < deadline {
            if watchdog_done.load(Ordering::SeqCst) {
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        if watchdog_done.load(Ordering::SeqCst) {
            return;
        }
        watchdog_flag.store(true, Ordering::SeqCst);
        kill_process(pid, SIGKILL);
    });
    let status = child.wait().map_err(|error| ToolsError::io("waiting for a child process", error))?;
    done.store(true, Ordering::SeqCst);
    let _ = watchdog.join();
    let timed_out = timed_out.load(Ordering::SeqCst);
    Ok(CommandRun {
        command: argv.to_vec(),
        cwd: cwd.to_owned(),
        environment: environment.to_vec(),
        started_at,
        duration_ms: start.elapsed().as_secs_f64() * 1000.0,
        pid,
        exit_code: status.code(),
        timed_out,
        stdout: fsutil::read_text(Path::new(&format!("{log}.stdout.txt")))?,
        stderr: fsutil::read_text(Path::new(&format!("{log}.stderr.txt")))?,
    })
}

/// `count` console wait lines.
fn waits(count: usize) -> String {
    "wait\n".repeat(count)
}

/// The scripted retail schedule (donor `captureConfig`).
#[must_use]
pub fn capture_config() -> String {
    [
        "echo Q1_RETAIL_BEGIN",
        "version",
        "developer 1",
        "skill 1",
        "deathmatch 0",
        "coop 0",
        "host_framerate 0.05",
        "sys_ticrate 0.05",
        "cl_forwardspeed 200",
        "cl_backspeed 200",
        "cl_sidespeed 350",
        "cl_upspeed 200",
        "cl_movespeedkey 2",
        "vid_mode 2",
        "viewsize 100",
        "fov 90",
        "gamma 1",
        "record retail-reference start",
        &waits(60),
        "echo Q1_RETAIL_MAP_READY",
        "status",
        "host_framerate",
        "sys_ticrate",
        "cl_forwardspeed",
        "save retail-before",
        "screenshot",
        "+forward",
        &waits(20),
        "-forward",
        &waits(2),
        "save retail-moved",
        "screenshot",
        "stop",
        "load retail-before",
        &waits(30),
        "save retail-restored",
        "screenshot",
        "echo Q1_RETAIL_DONE",
        "toggleconsole",
        "quit",
        "",
    ]
    .join("\n")
}

/// Retail capture checks.
#[derive(Debug, Clone)]
pub struct RetailChecks {
    /// The game process exited zero without timing out.
    pub process_exited_successfully: bool,
    /// The map checkpoint appears in the console.
    pub map_checkpoint_observed: bool,
    /// The done checkpoint appears in the console.
    pub done_checkpoint_observed: bool,
}

impl RetailChecks {
    /// Whether every check passed.
    #[must_use]
    pub fn all(&self) -> bool {
        self.process_exited_successfully && self.map_checkpoint_observed && self.done_checkpoint_observed
    }

    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("processExitedSuccessfully".to_owned(), Json::boolean(self.process_exited_successfully)),
            ("mapCheckpointObserved".to_owned(), Json::boolean(self.map_checkpoint_observed)),
            ("doneCheckpointObserved".to_owned(), Json::boolean(self.done_checkpoint_observed)),
        ])
    }
}

/// Evaluate the retail checks over the game run and console text.
#[must_use]
pub fn retail_checks(observed: Option<&CommandRun>, console_text: &str) -> RetailChecks {
    RetailChecks {
        process_exited_successfully: observed.is_some_and(|run| run.exit_code == Some(0) && !run.timed_out),
        map_checkpoint_observed: console_text.contains("Q1_RETAIL_MAP_READY"),
        done_checkpoint_observed: console_text.contains("Q1_RETAIL_DONE"),
    }
}

fn retail_corpus() -> String {
    format!("{}/q1/id1", corpus_root())
}

/// Capture the retail WinQuake reference (donor `captureRetail`).
pub fn capture_retail() -> Result<Json, ToolsError> {
    let project = quake_typescript_root();
    let steam = steam_common_path();
    let wine = steam.join("Proton - Experimental/files/bin/wine").to_string_lossy().into_owned();
    let wineserver = steam.join("Proton - Experimental/files/bin/wineserver").to_string_lossy().into_owned();
    let original = steam.join("Quake/Winquake.exe").to_string_lossy().into_owned();
    let corpus = retail_corpus();
    let run_id = now_iso().replace([':', '.'], "-");
    let directory = project.join(".artifacts/q1-retail").join(run_id).to_string_lossy().into_owned();
    let game = format!("{directory}/game");
    let id1 = format!("{game}/id1");
    let prefix = format!("{directory}/wineprefix");
    let private_home = format!("{directory}/home");
    std::fs::create_dir_all(&directory).map_err(|error| ToolsError::io(format!("creating {directory}"), error))?;
    std::fs::create_dir_all(&id1).map_err(|error| ToolsError::io(format!("creating {id1}"), error))?;
    std::fs::create_dir_all(&prefix).map_err(|error| ToolsError::io(format!("creating {prefix}"), error))?;
    std::fs::create_dir_all(&private_home).map_err(|error| ToolsError::io(format!("creating {private_home}"), error))?;
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/reference/q1_retail/capture.rs"),
        format!("{directory}/capture-driver.rs"),
    )
    .map_err(|error| ToolsError::io("copying the capture driver", error))?;
    let mut assets = Vec::new();
    for (source, target) in [("PAK0.PAK", "pak0.pak"), ("PAK1.PAK", "pak1.pak")] {
        let path = format!("{corpus}/{source}");
        assets.push(identify_file(&path)?);
        fsutil::create_symlink(Path::new(&path), Path::new(&format!("{id1}/{target}")))?;
    }
    let executable = identify_file(&original)?;
    std::fs::copy(&original, format!("{game}/Winquake.exe")).map_err(|error| ToolsError::io("staging Winquake.exe", error))?;
    fsutil::write_text(Path::new(&format!("{id1}/config.cfg")), "// Private retail reference profile\n")?;
    fsutil::write_text(Path::new(&format!("{id1}/autoexec.cfg")), "// Private retail reference profile\n")?;
    let config = capture_config();
    fsutil::write_text(Path::new(&format!("{id1}/reference.cfg")), &config)?;
    let mut number = 25000 + std::process::id() % 10000;
    while Path::new(&format!("/tmp/.X11-unix/X{number}")).exists() || Path::new(&format!("/tmp/.X{number}-lock")).exists() {
        number += 1;
    }
    let display = format!(":{number}");
    let environment = vec![
        ("PATH".to_owned(), "/usr/bin:/bin".to_owned()),
        ("HOME".to_owned(), private_home),
        ("LC_ALL".to_owned(), "C".to_owned()),
        ("TZ".to_owned(), "UTC".to_owned()),
        ("WINEPREFIX".to_owned(), prefix),
        ("WINEDEBUG".to_owned(), "-all".to_owned()),
        ("WINEDLLOVERRIDES".to_owned(), "winemenubuilder.exe=d".to_owned()),
        ("DISPLAY".to_owned(), display.clone()),
        ("LIBGL_ALWAYS_SOFTWARE".to_owned(), "1".to_owned()),
        ("MESA_LOADER_DRIVER_OVERRIDE".to_owned(), "llvmpipe".to_owned()),
    ];
    let display_command = ["/usr/bin/Xvfb", display.as_str(), "-screen", "0", "800x600x24", "-nolisten", "tcp", "-noreset"]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<String>>();
    let mut xvfb: ObservedProcess = start_observed(&display_command, &directory, &environment, None)?;
    let xvfb_pid = xvfb.pid();
    let mut observed: Option<CommandRun> = None;
    let mut failure: Option<String> = None;
    let run = (|| -> Result<(), ToolsError> {
        let deadline = Instant::now() + Duration::from_millis(5000);
        while !Path::new(&format!("/tmp/.X11-unix/X{number}")).exists() {
            if !xvfb.alive()? || Instant::now() > deadline {
                return Err(ToolsError::invalid("Private Xvfb failed to start"));
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        observed = Some(run_command(
            &[
                wine.clone(),
                format!("{game}/Winquake.exe"),
                "-startwindowed".to_owned(),
                "-dibonly".to_owned(),
                "-nocdaudio".to_owned(),
                "-nosound".to_owned(),
                "-nojoy".to_owned(),
                "-nomouse".to_owned(),
                "-condebug".to_owned(),
                "+exec".to_owned(),
                "reference.cfg".to_owned(),
            ],
            &game,
            &environment,
            90_000,
        )?);
        Ok(())
    })();
    if let Err(error) = run {
        failure = Some(error.to_string());
    }
    let cleanup = run_command(&[wineserver.clone(), "-k".to_owned()], &directory, &environment, 5000)?;
    let display_observation = xvfb.finish(&format!("{directory}/xvfb"))?;
    let mut outputs = Vec::new();
    let mut entries: Vec<PathBuf> = Vec::new();
    for entry in std::fs::read_dir(&id1).map_err(|error| ToolsError::io(format!("listing {id1}"), error))? {
        let entry = entry.map_err(|error| ToolsError::io(format!("listing {id1}"), error))?;
        if entry.file_type().map_err(|error| ToolsError::io(format!("stating {}", entry.path().display()), error))?.is_file() {
            entries.push(entry.path());
        }
    }
    entries.sort();
    for path in entries {
        outputs.push(identify_file(&path.to_string_lossy())?);
    }
    let console_path = format!("{id1}/qconsole.log");
    let console_text = fsutil::read_text(Path::new(&console_path)).unwrap_or_default();
    let checks = retail_checks(observed.as_ref(), &console_text);
    let result = Json::object(vec![
        ("schemaVersion".to_owned(), Json::int(1)),
        ("capturedAt".to_owned(), Json::string(now_iso())),
        ("classification".to_owned(), Json::string("independent-retail-executable-observation")),
        ("directory".to_owned(), Json::string(&directory)),
        (
            "provenance".to_owned(),
            Json::object(vec![
                ("executable".to_owned(), executable.to_json()),
                ("stagedExecutable".to_owned(), identify_file(&format!("{game}/Winquake.exe"))?.to_json()),
                ("assets".to_owned(), Json::array(assets.iter().map(FileIdentity::to_json).collect())),
                ("runtime".to_owned(), identify_file(&wine)?.to_json()),
                ("wineserver".to_owned(), identify_file(&wineserver)?.to_json()),
                ("driver".to_owned(), identify_file(&format!("{directory}/capture-driver.rs"))?.to_json()),
            ]),
        ),
        ("config".to_owned(), Json::string(&config)),
        ("observed".to_owned(), observed.as_ref().map_or(Json::Null, CommandRun::to_json)),
        ("cleanup".to_owned(), cleanup.to_json()),
        ("failure".to_owned(), failure.as_ref().map_or(Json::Null, Json::string)),
        (
            "display".to_owned(),
            Json::object(vec![
                ("command".to_owned(), Json::array(display_command.iter().map(Json::string).collect())),
                (
                    "environment".to_owned(),
                    Json::object(environment.iter().map(|(key, value)| (key.clone(), Json::string(value))).collect()),
                ),
                ("pid".to_owned(), Json::uint(u64::from(xvfb_pid))),
                (
                    "exitCode".to_owned(),
                    display_observation.exit_code.map_or(Json::Null, |code| Json::int(i64::from(code))),
                ),
                ("stdout".to_owned(), Json::string(&display_observation.stdout)),
                ("stderr".to_owned(), Json::string(&display_observation.stderr)),
            ]),
        ),
        ("consoleText".to_owned(), Json::string(&console_text)),
        ("outputs".to_owned(), Json::array(outputs.iter().map(FileIdentity::to_json).collect())),
        ("checks".to_owned(), checks.to_json()),
        (
            "limitations".to_owned(),
            Json::array(
                [
                    "Installed executable identity establishes the observed bytes, not vendor authenticity or equivalence to current source.",
                    "The reference.cfg schedule uses requested host_framerate and wait commands. Simulation and command acceptance must be checked against observed artifacts.",
                    "No TypeScript engine comparison, multiplayer interoperability, mission pack, rerelease, or image tolerance is established.",
                ]
                .into_iter()
                .map(Json::string)
                .collect(),
            ),
        ),
    ]);
    fsutil::write_text(Path::new(&format!("{directory}/capture.json")), &format!("{}\n", result.render_pretty()))?;
    fsutil::write_text(
        &project.join("verification/reference-cases/q1-retail/latest.json"),
        &format!("{}\n", result.render_pretty()),
    )?;
    println!(
        "{}",
        Json::object(vec![
            ("directory".to_owned(), Json::string(&directory)),
            ("checks".to_owned(), checks.to_json()),
            ("failure".to_owned(), failure.as_ref().map_or(Json::Null, Json::string)),
            ("outputCount".to_owned(), Json::uint(outputs.len() as u64)),
        ])
        .render()
    );
    Ok(result)
}

/// Run the retail capture (donor `main` ignores arguments).
pub fn run() -> Result<i32, ToolsError> {
    let result = capture_retail()?;
    let failed = result.get("failure").is_some_and(|failure| !failure.is_null())
        || result
            .get("checks")
            .and_then(Json::as_object)
            .is_some_and(|checks| checks.iter().any(|(_, value)| value.as_bool() != Some(true)));
    Ok(i32::from(failed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedule_matches_the_donor_script() {
        let config = capture_config();
        assert!(config.starts_with("echo Q1_RETAIL_BEGIN\nversion\n"));
        assert!(config.contains("record retail-reference start\n"));
        assert!(config.contains("echo Q1_RETAIL_MAP_READY\n"));
        assert!(config.contains("save retail-before\nscreenshot\n+forward\n"));
        assert!(config.contains("load retail-before\n"));
        assert!(config.contains("save retail-restored\nscreenshot\necho Q1_RETAIL_DONE\ntoggleconsole\nquit\n"));
        assert_eq!(config.matches("wait\n").count(), 60 + 20 + 2 + 30);
        assert!(config.ends_with("quit\n"));
    }

    #[test]
    fn file_logged_commands_capture_streams() {
        let directory = fsutil::make_temp_dir(&std::env::temp_dir(), "quake-q1-command-").expect("temp dir");
        let text = directory.to_string_lossy().into_owned();
        let environment = vec![("PATH".to_owned(), "/usr/bin:/bin".to_owned()), ("LC_ALL".to_owned(), "C".to_owned())];
        let run = run_command(
            &[String::from("sh"), String::from("-c"), String::from("echo out; echo err >&2; exit 3")],
            &text,
            &environment,
            10_000,
        )
        .expect("run");
        assert_eq!(run.exit_code, Some(3));
        assert!(!run.timed_out);
        assert_eq!(run.stdout, "out\n");
        assert_eq!(run.stderr, "err\n");
        assert!(run_command(&[String::from("sh"), String::from("-c"), String::from("exec sleep 30")], &text, &environment, 200)
            .expect("timeout run")
            .timed_out);
        fsutil::remove_forced(&directory);
    }

    #[test]
    fn evaluates_retail_checks() {
        let run = CommandRun {
            command: Vec::new(),
            cwd: String::new(),
            environment: Vec::new(),
            started_at: String::new(),
            duration_ms: 0.0,
            pid: 1,
            exit_code: Some(0),
            timed_out: false,
            stdout: String::new(),
            stderr: String::new(),
        };
        let checks = retail_checks(Some(&run), "Q1_RETAIL_MAP_READY\nQ1_RETAIL_DONE\n");
        assert!(checks.all());
        assert!(!retail_checks(None, "").process_exited_successfully);
        let timed_out = CommandRun { timed_out: true, ..run.clone() };
        assert!(!retail_checks(Some(&timed_out), "Q1_RETAIL_MAP_READY\nQ1_RETAIL_DONE\n").all());
    }
}
