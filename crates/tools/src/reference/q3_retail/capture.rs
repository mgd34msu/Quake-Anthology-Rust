//! Retail Q3 observation under Wine (donor `tools/reference/q3-retail/capture.ts`).
//!
//! Stages the Steam Quake 3 executable with corpus pk3s into an isolated
//! bubblewrap root, runs the dedicated baseq3 case under a private Xvfb
//! display, issues connectionless protocol requests, and records the engine
//! log plus output identities. The live orchestration needs the retail
//! installation, Proton runtime, and bubblewrap; the sandbox/window helpers,
//! pk3 selection, and qualification are pure and tested.

use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::error::ToolsError;
use crate::fsutil;
use crate::json::Json;
use crate::reference::environment::{corpus_root, identify_file, quake_typescript_root, source_root};
use crate::reference::q2_native::capture::settings;
use crate::reference::q2_native::udp::{query, unused_port, UdpQuery};
use crate::reference::schema::FileIdentity;
use crate::reference::steam::steam_common_path;
use crate::sys::{kill_process, SIGKILL, SIGTERM};
use crate::time::now_iso;

/// Retail edition under test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edition {
    /// Base game.
    Baseq3,
    /// Mission pack.
    Missionpack,
}

impl Edition {
    fn as_str(self) -> &'static str {
        match self {
            Self::Baseq3 => "baseq3",
            Self::Missionpack => "missionpack",
        }
    }
}

/// Capture mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetailMode {
    /// Dedicated server.
    Dedicated,
    /// Rendering client.
    Render,
}

impl RetailMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Dedicated => "dedicated",
            Self::Render => "render",
        }
    }
}

/// A finished sandboxed observation.
#[derive(Debug, Clone)]
pub struct SandboxObservation {
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
    pub timeout: bool,
    /// Captured standard output.
    pub stdout: String,
    /// Captured standard error.
    pub stderr: String,
}

impl SandboxObservation {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            (
                "command".to_owned(),
                Json::array(self.command.iter().map(Json::string).collect()),
            ),
            ("cwd".to_owned(), Json::string(&self.cwd)),
            (
                "environment".to_owned(),
                Json::object(
                    self.environment
                        .iter()
                        .map(|(key, value)| (key.clone(), Json::string(value)))
                        .collect(),
                ),
            ),
            ("startedAt".to_owned(), Json::string(&self.started_at)),
            ("durationMs".to_owned(), Json::float(self.duration_ms)),
            ("pid".to_owned(), Json::uint(u64::from(self.pid))),
            (
                "exitCode".to_owned(),
                self.exit_code.map_or(Json::Null, |code| Json::int(i64::from(code))),
            ),
            ("timeout".to_owned(), Json::boolean(self.timeout)),
            ("stdout".to_owned(), Json::string(&self.stdout)),
            ("stderr".to_owned(), Json::string(&self.stderr)),
        ])
    }
}

/// A spawned, observed sandboxed process.
pub struct ObservedSandbox {
    child: std::process::Child,
    stdout: crate::process::SharedBytes,
    stderr: crate::process::SharedBytes,
    readers: Vec<std::thread::JoinHandle<()>>,
    command: Vec<String>,
    cwd: String,
    environment: Vec<(String, String)>,
    started_at: String,
    start: Instant,
    done: Arc<AtomicBool>,
    timed_out: Arc<AtomicBool>,
    watchdog: Option<std::thread::JoinHandle<()>>,
    finished: Option<SandboxObservation>,
}

impl ObservedSandbox {
    /// Child pid.
    #[must_use]
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Whether the child is still running.
    pub fn alive(&mut self) -> Result<bool, ToolsError> {
        self.child
            .try_wait()
            .map(|status| status.is_none())
            .map_err(|error| ToolsError::io("polling a child process", error))
    }

    /// SIGTERM a running child.
    pub fn stop(&mut self) -> Result<(), ToolsError> {
        if self.alive()? {
            kill_process(self.child.id(), SIGTERM);
        }
        Ok(())
    }

    /// Reap the child and observe it (memoized like the donor).
    pub fn finish(&mut self) -> Result<SandboxObservation, ToolsError> {
        if let Some(observation) = &self.finished {
            return Ok(observation.clone());
        }
        let status = self
            .child
            .wait()
            .map_err(|error| ToolsError::io("waiting for a child process", error))?;
        self.done.store(true, Ordering::SeqCst);
        if let Some(watchdog) = self.watchdog.take() {
            let _ = watchdog.join();
        }
        for reader in self.readers.drain(..) {
            let _ = reader.join();
        }
        let observation = SandboxObservation {
            command: self.command.clone(),
            cwd: self.cwd.clone(),
            environment: self.environment.clone(),
            started_at: self.started_at.clone(),
            duration_ms: self.start.elapsed().as_secs_f64() * 1000.0,
            pid: self.child.id(),
            exit_code: status.code(),
            timeout: self.timed_out.load(Ordering::SeqCst),
            stdout: crate::process::snapshot(&self.stdout),
            stderr: crate::process::snapshot(&self.stderr),
        };
        self.finished = Some(observation.clone());
        Ok(observation)
    }
}

/// Spawn an observed process; the watchdog SIGKILLs past `timeout_ms`.
pub fn spawn_observed(
    command: &[String],
    cwd: &str,
    environment: &[(String, String)],
    timeout_ms: u64,
) -> Result<ObservedSandbox, ToolsError> {
    let (program, args) = command
        .split_first()
        .ok_or_else(|| ToolsError::invalid("Cannot observe an empty command"))?;
    let mut child_command = std::process::Command::new(program);
    child_command
        .args(args)
        .current_dir(cwd)
        .env_clear()
        .envs(environment.iter().cloned());
    child_command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = child_command
        .spawn()
        .map_err(|error| ToolsError::io(format!("spawning {}", command.join(" ")), error))?;
    let (stdout, out_handle) = crate::process::spawn_reader(child.stdout.take().expect("piped stdout"));
    let (stderr, err_handle) = crate::process::spawn_reader(child.stderr.take().expect("piped stderr"));
    let done = Arc::new(AtomicBool::new(false));
    let timed_out = Arc::new(AtomicBool::new(false));
    let pid = child.id();
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
    Ok(ObservedSandbox {
        child,
        stdout,
        stderr,
        readers: vec![out_handle, err_handle],
        command: command.to_vec(),
        cwd: cwd.to_owned(),
        environment: environment.to_vec(),
        started_at: now_iso(),
        start: Instant::now(),
        done,
        timed_out,
        watchdog: Some(watchdog),
        finished: None,
    })
}

/// Wrap a command in the bubblewrap sandbox (donor `sandbox`).
#[must_use]
pub fn sandbox(root: &str, command: &[String]) -> Vec<String> {
    let mut wrapped = vec![
        "/usr/bin/bwrap".to_owned(),
        "--ro-bind".to_owned(),
        "/".to_owned(),
        "/".to_owned(),
        "--bind".to_owned(),
        root.to_owned(),
        root.to_owned(),
        "--bind".to_owned(),
        format!("{root}/tmp"),
        "/tmp".to_owned(),
        "--dev".to_owned(),
        "/dev".to_owned(),
        "--proc".to_owned(),
        "/proc".to_owned(),
        "--unshare-pid".to_owned(),
        "--die-with-parent".to_owned(),
        "--".to_owned(),
    ];
    wrapped.extend(command.iter().cloned());
    wrapped
}

/// Map a host path into Wine's `Z:` drive (donor `windows`).
#[must_use]
pub fn windows(path: &str) -> String {
    format!("Z:{}", path.replace('/', "\\"))
}

/// Whether a corpus file is a selected pak (`/^pak[0-9]+\.pk3$/`).
#[must_use]
pub fn selected_pak(name: &str) -> bool {
    name.strip_prefix("pak")
        .and_then(|rest| rest.strip_suffix(".pk3"))
        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()))
}

/// Wait until the engine log contains `marker`, failing when the process exits first.
fn wait_for_log(
    path: &str,
    marker: &str,
    process: &mut ObservedSandbox,
    timeout_ms: u64,
) -> Result<String, ToolsError> {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    while Instant::now() < deadline {
        let text = fsutil::read_text(Path::new(path)).unwrap_or_default();
        if text.contains(marker) {
            return Ok(text);
        }
        if !process.alive()? {
            let tail: String = text
                .chars()
                .rev()
                .take(2500)
                .collect::<Vec<char>>()
                .into_iter()
                .rev()
                .collect();
            return Err(ToolsError::invalid(format!("Exited before {marker}: {tail}")));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let text = fsutil::read_text(Path::new(path)).unwrap_or_default();
    let tail: String = text
        .chars()
        .rev()
        .take(2500)
        .collect::<Vec<char>>()
        .into_iter()
        .rev()
        .collect();
    Err(ToolsError::invalid(format!("Missing {marker}: {tail}")))
}

/// Case qualification over the engine log, queries, and outputs.
#[derive(Debug, Clone)]
pub struct CaseQualification {
    /// The log shows server initialization for the expected map.
    pub map_initialized: bool,
    /// The log shows game VM execution.
    pub vm_executed: bool,
    /// Queries that drew replies.
    pub protocol_replies: usize,
    /// A demo larger than 16 bytes was recorded.
    pub demo_recorded: bool,
    /// Rendered frames captured.
    pub render_frames: usize,
}

impl CaseQualification {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("mapInitialized".to_owned(), Json::boolean(self.map_initialized)),
            ("vmExecuted".to_owned(), Json::boolean(self.vm_executed)),
            ("protocolReplies".to_owned(), Json::uint(self.protocol_replies as u64)),
            ("demoRecorded".to_owned(), Json::boolean(self.demo_recorded)),
            ("renderFrames".to_owned(), Json::uint(self.render_frames as u64)),
        ])
    }
}

/// Evaluate case qualification.
#[must_use]
pub fn qualify_case(log: &str, map: &str, queries: &[UdpQuery], outputs: &[FileIdentity]) -> CaseQualification {
    CaseQualification {
        map_initialized: log.contains("------ Server Initialization ------") && log.contains(&format!("map: {map}")),
        vm_executed: log.contains("Game Initialization") || log.contains("InitGame:"),
        protocol_replies: queries.iter().filter(|item| !item.response.is_empty()).count(),
        demo_recorded: outputs
            .iter()
            .any(|item| item.path.ends_with(".dm_68") && item.size > 16),
        render_frames: outputs.iter().filter(|item| item.path.ends_with(".tga")).count(),
    }
}

fn all_files_below(root: &str) -> Result<Vec<String>, ToolsError> {
    let mut result = Vec::new();
    let mut stack = vec![root.to_owned()];
    while let Some(current) = stack.pop() {
        let listing =
            std::fs::read_dir(&current).map_err(|error| ToolsError::io(format!("listing {current}"), error))?;
        for entry in listing {
            let entry = entry.map_err(|error| ToolsError::io(format!("listing {current}"), error))?;
            let file_type = entry
                .file_type()
                .map_err(|error| ToolsError::io(format!("stating {}", entry.path().display()), error))?;
            let path = format!("{}/{}", current, entry.file_name().to_string_lossy());
            if file_type.is_dir() {
                stack.push(path);
            } else if file_type.is_file() {
                result.push(path);
            }
        }
    }
    result.sort();
    Ok(result)
}

fn case_config(mode: RetailMode, map: &str) -> String {
    if mode == RetailMode::Dedicated {
        format!("map {map}\nstatus\nserverinfo\nfs_homepath\nfs_basepath\nprotocol\nsv_fps\necho Q3_RETAIL_READY\n")
    } else {
        format!(
            "map {map}\nwait 180\nviewpos\nrecord retail-reference\nwait 30\nscreenshot before\n+forward\nwait 60\n-forward\nviewpos\nscreenshot after\nwait 30\nstoprecord\nstatus\ngfxinfo\nfs_homepath\nfs_basepath\nprotocol\necho Q3_RETAIL_CAPTURE_DONE\nquit\n"
        )
    }
}

#[allow(clippy::too_many_lines)]
fn capture_case(
    root: &str,
    display: &str,
    content: &str,
    edition: Edition,
    mode: RetailMode,
) -> Result<Json, ToolsError> {
    let directory = format!("{root}/{}-{}", edition.as_str(), mode.as_str());
    let home = format!("{directory}/profile");
    let map = if edition == Edition::Baseq3 { "q3dm1" } else { "mpteam1" };
    std::fs::create_dir_all(format!("{home}/{}", edition.as_str()))
        .map_err(|error| ToolsError::io(format!("creating {home}"), error))?;
    std::fs::create_dir_all(format!("{home}/baseq3"))
        .map_err(|error| ToolsError::io(format!("creating {home}"), error))?;
    let config = case_config(mode, map);
    fsutil::write_text(
        Path::new(&format!("{home}/{}/reference.cfg", edition.as_str())),
        &config,
    )?;
    fsutil::write_text(
        Path::new(&format!("{home}/{}/q3config.cfg", edition.as_str())),
        "// owned empty reference profile\n",
    )?;
    let port = unused_port()?;
    let port_text = port.to_string();
    let home_dir = format!("{directory}/wine-home");
    let prefix = format!("{directory}/wine-prefix");
    let cache = format!("{directory}/cache");
    let environment = vec![
        ("PATH".to_owned(), "/usr/bin:/bin".to_owned()),
        ("HOME".to_owned(), home_dir.clone()),
        ("WINEPREFIX".to_owned(), prefix.clone()),
        ("WINEDEBUG".to_owned(), "-all".to_owned()),
        ("WINEDLLOVERRIDES".to_owned(), "mscoree,mshtml=".to_owned()),
        ("DISPLAY".to_owned(), display.to_owned()),
        ("LC_ALL".to_owned(), "C".to_owned()),
        ("TZ".to_owned(), "UTC".to_owned()),
        ("LIBGL_ALWAYS_SOFTWARE".to_owned(), "1".to_owned()),
        ("MESA_LOADER_DRIVER_OVERRIDE".to_owned(), "llvmpipe".to_owned()),
        ("XDG_CACHE_HOME".to_owned(), cache),
    ];
    std::fs::create_dir_all(&home_dir).map_err(|error| ToolsError::io("creating the wine home", error))?;
    let wine = steam_common_path()
        .join("Proton - Experimental/files/bin/wine")
        .to_string_lossy()
        .into_owned();
    let fs_basepath = windows(content);
    let fs_homepath = windows(&home);
    let fs_game = if edition == Edition::Baseq3 {
        ""
    } else {
        edition.as_str()
    };
    let dedicated = if mode == RetailMode::Dedicated { "1" } else { "0" };
    let hostname = format!("q3-retail-{}", edition.as_str());
    let gametype = if edition == Edition::Baseq3 { "0" } else { "4" };
    let values = [
        ("fs_basepath", fs_basepath.as_str()),
        ("fs_homepath", fs_homepath.as_str()),
        ("fs_cdpath", ""),
        ("fs_copyfiles", "0"),
        ("fs_game", fs_game),
        ("dedicated", dedicated),
        ("net_ip", "127.0.0.1"),
        ("net_port", port_text.as_str()),
        ("sv_master1", ""),
        ("sv_pure", "0"),
        ("sv_maxclients", "4"),
        ("sv_hostname", hostname.as_str()),
        ("cl_motd", "0"),
        ("cl_allowDownload", "0"),
        ("sv_allowDownload", "0"),
        ("rconPassword", "reference-private"),
        ("logfile", "2"),
        ("developer", "1"),
        ("bot_enable", "0"),
        ("com_hunkMegs", "128"),
        ("com_zoneMegs", "32"),
        ("com_maxfps", "60"),
        ("r_mode", "3"),
        ("r_fullscreen", "0"),
        ("r_allowSoftwareGL", "1"),
        ("r_colorbits", "24"),
        ("r_depthbits", "24"),
        ("r_stencilbits", "8"),
        ("r_swapInterval", "0"),
        ("s_initsound", "0"),
        ("in_mouse", "0"),
        ("g_gametype", gametype),
        ("g_doWarmup", "0"),
        ("g_log", "games.log"),
        ("g_logSync", "1"),
        ("vm_game", "1"),
        ("vm_cgame", "1"),
        ("vm_ui", "1"),
        ("com_introplayed", "1"),
    ];
    let mut argv = vec![wine, windows(&format!("{root}/bin/quake3.exe"))];
    argv.extend(settings(&values));
    argv.extend(["+exec".to_owned(), "reference.cfg".to_owned()]);
    let command = sandbox(root, &argv);
    let mut process = spawn_observed(&command, &directory, &environment, 90_000)?;
    fsutil::write_text(
        Path::new(&format!("{root}/live.json")),
        &Json::object(vec![
            ("display".to_owned(), Json::string(display)),
            ("pid".to_owned(), Json::uint(u64::from(process.pid()))),
            ("directory".to_owned(), Json::string(&directory)),
            ("prefix".to_owned(), Json::string(&prefix)),
        ])
        .render(),
    )?;
    println!("{} {} started PID {}", edition.as_str(), mode.as_str(), process.pid());
    let mut queries: Vec<UdpQuery> = Vec::new();
    let mut failure: Option<String> = None;
    let log_path = format!("{home}/{}/qconsole.log", edition.as_str());
    let run = (|| -> Result<(), ToolsError> {
        wait_for_log(
            &log_path,
            if mode == RetailMode::Dedicated {
                "Q3_RETAIL_READY"
            } else {
                "Q3_RETAIL_CAPTURE_DONE"
            },
            &mut process,
            80_000,
        )?;
        if mode == RetailMode::Dedicated {
            for request in ["getstatus q3-retail", "getinfo q3-retail", "getchallenge"] {
                queries.push(query(port, request)?);
            }
            queries.push(query(port, "rcon reference-private status")?);
            queries.push(query(port, "rcon reference-private quit")?);
            process.stop()?;
        }
        Ok(())
    })();
    if let Err(error) = run {
        failure = Some(error.to_string());
        process.stop()?;
    }
    let observation = process.finish()?;
    let mut outputs = Vec::new();
    for path in all_files_below(&home)? {
        outputs.push(identify_file(&path)?);
    }
    let log = fsutil::read_text(Path::new(&log_path)).unwrap_or_default();
    let qualification = qualify_case(&log, map, &queries, &outputs);
    fsutil::write_text(
        Path::new(&format!("{directory}/process.json")),
        &format!("{}\n", observation.to_json().render_pretty()),
    )?;
    println!(
        "{} {}: {}{}",
        edition.as_str(),
        mode.as_str(),
        qualification.to_json().render(),
        failure
            .as_ref()
            .map_or(String::new(), |reason| format!(" failure {reason}"))
    );
    Ok(Json::object(vec![
        ("edition".to_owned(), Json::string(edition.as_str())),
        ("mode".to_owned(), Json::string(mode.as_str())),
        ("map".to_owned(), Json::string(map)),
        ("directory".to_owned(), Json::string(&directory)),
        (
            "requestedSettings".to_owned(),
            Json::object(
                values
                    .iter()
                    .map(|(name, value)| ((*name).to_owned(), Json::string(*value)))
                    .collect(),
            ),
        ),
        ("input".to_owned(), Json::string(&config)),
        ("observation".to_owned(), observation.to_json()),
        ("qualification".to_owned(), qualification.to_json()),
        ("failure".to_owned(), failure.as_ref().map_or(Json::Null, Json::string)),
        (
            "queries".to_owned(),
            Json::array(queries.iter().map(UdpQuery::to_json).collect()),
        ),
        (
            "outputs".to_owned(),
            Json::array(outputs.iter().map(FileIdentity::to_json).collect()),
        ),
        ("log".to_owned(), Json::string(&log)),
        ("home".to_owned(), Json::string(&home)),
        (
            "environment".to_owned(),
            Json::object(
                environment
                    .iter()
                    .map(|(key, value)| (key.clone(), Json::string(value)))
                    .collect(),
            ),
        ),
    ]))
}

/// Run the retail capture (donor `main` takes no arguments).
pub fn run(args: &[String]) -> Result<i32, ToolsError> {
    if !args.is_empty() {
        return Err(ToolsError::invalid(
            "Usage: qa-tools reference::q3-retail::capture::run",
        ));
    }
    let project = quake_typescript_root();
    let common = steam_common_path();
    let retail = common.join("Quake 3 Arena/quake3.exe").to_string_lossy().into_owned();
    let runtime = common
        .join("Proton - Experimental/files")
        .to_string_lossy()
        .into_owned();
    let wine = format!("{runtime}/bin/wine");
    let corpus = format!("{}/q3a", corpus_root());
    let original_source = format!("{}/quake-iii-arena", source_root());
    let artifacts = project.join(".artifacts/q3-retail");
    std::fs::create_dir_all(&artifacts)
        .map_err(|error| ToolsError::io(format!("creating {}", artifacts.display()), error))?;
    let root = fsutil::make_temp_dir(&artifacts, "capture-")?
        .to_string_lossy()
        .into_owned();
    for path in ["tmp/.X11-unix", "bin", "content/baseq3", "content/missionpack"] {
        std::fs::create_dir_all(format!("{root}/{path}"))
            .map_err(|error| ToolsError::io(format!("creating {root}/{path}"), error))?;
    }
    std::fs::copy(&retail, format!("{root}/bin/quake3.exe"))
        .map_err(|error| ToolsError::io("staging quake3.exe", error))?;
    let mut archives = Vec::new();
    for edition in ["baseq3", "missionpack"] {
        let mut names: Vec<String> = Vec::new();
        let listing = std::fs::read_dir(format!("{corpus}/{edition}"))
            .map_err(|error| ToolsError::io(format!("listing {corpus}/{edition}"), error))?;
        for entry in listing {
            let entry = entry.map_err(|error| ToolsError::io("listing corpus", error))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if selected_pak(&name) {
                names.push(name);
            }
        }
        names.sort();
        for name in names {
            let source = format!("{corpus}/{edition}/{name}");
            archives.push(identify_file(&source)?.to_json());
            fsutil::create_symlink(
                Path::new(&source),
                Path::new(&format!("{root}/content/{edition}/{name}")),
            )?;
        }
    }
    let display = ":23173";
    let display_environment = vec![
        ("PATH".to_owned(), "/usr/bin:/bin".to_owned()),
        ("LC_ALL".to_owned(), "C".to_owned()),
        ("HOME".to_owned(), root.clone()),
        ("LIBGL_ALWAYS_SOFTWARE".to_owned(), "1".to_owned()),
        (
            "__EGL_VENDOR_LIBRARY_FILENAMES".to_owned(),
            "/usr/share/glvnd/egl_vendor.d/50_mesa.json".to_owned(),
        ),
    ];
    let xvfb_command = sandbox(
        &root,
        &[
            "/usr/bin/Xvfb",
            display,
            "-screen",
            "0",
            "640x480x24",
            "-nolisten",
            "tcp",
            "-noreset",
            "-extension",
            "GLX",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<String>>(),
    );
    let mut server = spawn_observed(&xvfb_command, &root, &display_environment, 120_000)?;
    let mut cases = Vec::new();
    let mut failure: Option<String> = None;
    let run = (|| -> Result<(), ToolsError> {
        std::thread::sleep(Duration::from_millis(500));
        if !server.alive()? {
            return Err(ToolsError::invalid("Private Xvfb exited"));
        }
        cases.push(capture_case(
            &root,
            display,
            &format!("{root}/content"),
            Edition::Baseq3,
            RetailMode::Dedicated,
        )?);
        Ok(())
    })();
    if let Err(error) = run {
        failure = Some(error.to_string());
    }
    server.stop()?;
    let display_observation = server.finish()?;
    let exe = std::env::current_exe().map_err(|error| ToolsError::io("resolving current executable", error))?;
    let capture_program = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/reference/q3_retail/capture.rs")
        .to_string_lossy()
        .into_owned();
    let mut runtime_files = Vec::new();
    for path in [
        wine.as_str(),
        &format!("{runtime}/bin/wineserver"),
        &format!("{runtime}/lib/wine/i386-windows/ntdll.dll"),
        &format!("{runtime}/lib/wine/x86_64-unix/ntdll.so"),
        &exe.to_string_lossy(),
        "/usr/bin/bwrap",
        "/usr/bin/Xvfb",
    ] {
        runtime_files.push(identify_file(path)?.to_json());
    }
    let mut source_files = Vec::new();
    for path in ["code/qcommon/files.c", "code/win32/win_main.c", "code/client/cl_main.c"] {
        source_files.push(identify_file(&format!("{original_source}/{path}"))?.to_json());
    }
    let manifest = Json::object(vec![
        ("schemaVersion".to_owned(), Json::int(1)),
        ("classification".to_owned(), Json::string("independent-retail-executable-observation")),
        ("capturedAt".to_owned(), Json::string(now_iso())),
        ("root".to_owned(), Json::string(&root)),
        (
            "command".to_owned(),
            Json::array(vec![Json::string(exe.to_string_lossy()), Json::string("reference::q3-retail::capture::run")]),
        ),
        ("executable".to_owned(), identify_file(&retail)?.to_json()),
        ("copiedExecutable".to_owned(), identify_file(&format!("{root}/bin/quake3.exe"))?.to_json()),
        ("captureProgram".to_owned(), identify_file(&capture_program)?.to_json()),
        ("runtime".to_owned(), Json::array(runtime_files)),
        (
            "sourceContext".to_owned(),
            Json::object(vec![
                ("classification".to_owned(), Json::string("filesystem-review-only-not-retail-build-provenance")),
                ("files".to_owned(), Json::array(source_files)),
                (
                    "review".to_owned(),
                    Json::string(
                        "FS_FOpenFileWrite/Append and FS_SV_FOpenFileWrite use fs_homepath; fs_copyfiles is disabled. Retail writes are independently restricted by a read-only host mount and owned writable root.",
                    ),
                ),
            ]),
        ),
        (
            "content".to_owned(),
            Json::object(vec![
                ("archives".to_owned(), Json::array(archives)),
                (
                    "order".to_owned(),
                    Json::string(
                        "For each game directory pk3 names mount in reverse lexical order, and missionpack precedes baseq3. Actual search paths are retained in each engine log.",
                    ),
                ),
                (
                    "selection".to_owned(),
                    Json::string(
                        "Only selected pakN.pk3 symlinks; existing external configs, q3key, logs and donor executables are excluded.",
                    ),
                ),
            ]),
        ),
        (
            "isolation".to_owned(),
            Json::object(vec![
                ("writableRoot".to_owned(), Json::string(&root)),
                ("display".to_owned(), Json::string(display)),
                ("tmp".to_owned(), Json::string(format!("{root}/tmp"))),
                ("policy".to_owned(), Json::string("bubblewrap read-only root; only artifact root and owned /tmp writable; private PID namespace dies with each process; no Proton launcher; no host display; loopback engine binding")),
            ]),
        ),
        ("displayObservation".to_owned(), display_observation.to_json()),
        ("cases".to_owned(), Json::array(cases.clone())),
        ("failure".to_owned(), failure.as_ref().map_or(Json::Null, Json::string)),
        (
            "limits".to_owned(),
            Json::array(
                [
                    "No retail build source provenance is asserted. Source hashes describe only the filesystem safety review.",
                    "This bounded independent capture does not establish a TypeScript implementation comparison, campaign completion, multi-seat behavior, audio, or performance acceptance.",
                    "Requested cvars and input schedules are retained alongside actual logs and output identities; wall-clock timing is environment-specific.",
                ]
                .into_iter()
                .map(Json::string)
                .collect(),
            ),
        ),
    ]);
    let output = project.join("verification/reference-cases/q3-retail/latest.json");
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| ToolsError::io(format!("creating {}", parent.display()), error))?;
    }
    fsutil::write_text(&output, &format!("{}\n", manifest.render_pretty()))?;
    fsutil::write_text(
        Path::new(&format!("{root}/manifest.json")),
        &format!("{}\n", manifest.render_pretty()),
    )?;
    fsutil::write_text(
        Path::new(&format!("{root}/live.json")),
        &Json::object(vec![
            ("state".to_owned(), Json::string("reaped")),
            ("displayPid".to_owned(), Json::uint(u64::from(server.pid()))),
            (
                "casePids".to_owned(),
                Json::array(
                    cases
                        .iter()
                        .filter_map(|case| {
                            case.get("observation")
                                .and_then(|observation| observation.get("pid"))
                                .cloned()
                        })
                        .collect(),
                ),
            ),
        ])
        .render(),
    )?;
    println!("{}", output.display());
    let failed = failure.is_some()
        || cases
            .iter()
            .any(|case| case.get("failure").is_some_and(|value| !value.is_null()));
    Ok(i32::from(failed))
}

#[cfg(test)]
mod tests {
    use crate::reference::q2_native::udp::{UdpPacket, UdpQuery};

    use super::*;

    #[test]
    fn wraps_commands_in_the_sandbox() {
        let wrapped = sandbox("/root", &["/bin/echo".to_owned(), "hi".to_owned()]);
        assert_eq!(
            wrapped,
            vec![
                "/usr/bin/bwrap",
                "--ro-bind",
                "/",
                "/",
                "--bind",
                "/root",
                "/root",
                "--bind",
                "/root/tmp",
                "/tmp",
                "--dev",
                "/dev",
                "--proc",
                "/proc",
                "--unshare-pid",
                "--die-with-parent",
                "--",
                "/bin/echo",
                "hi",
            ]
        );
    }

    #[test]
    fn maps_windows_paths() {
        assert_eq!(windows("/root/content/baseq3"), "Z:\\root\\content\\baseq3");
    }

    #[test]
    fn selects_numbered_paks() {
        for name in ["pak0.pk3", "pak8.pk3", "pak10.pk3"] {
            assert!(selected_pak(name), "{name}");
        }
        for name in ["pak.pk3", "pak0.pk3x", "pak0a.pk3", "Pak0.pk3", "pak0zip", "q3key"] {
            assert!(!selected_pak(name), "{name}");
        }
    }

    #[test]
    fn qualifies_case_evidence() {
        let log = "------ Server Initialization ------\nmap: q3dm1\nGame Initialization\n";
        let packet = UdpPacket {
            hex: String::new(),
            text: "reply".to_owned(),
            from: "127.0.0.1".to_owned(),
            port: 27960,
            elapsed_ms: 1.0,
        };
        let queries = vec![
            UdpQuery {
                destination: String::new(),
                request: "getstatus".to_owned(),
                sent_hex: String::new(),
                response: vec![packet],
            },
            UdpQuery {
                destination: String::new(),
                request: "quit".to_owned(),
                sent_hex: String::new(),
                response: Vec::new(),
            },
        ];
        let outputs = vec![
            FileIdentity {
                path: "/home/demo.dm_68".to_owned(),
                size: 100,
                sha256: String::new(),
            },
            FileIdentity {
                path: "/home/shot.tga".to_owned(),
                size: 100,
                sha256: String::new(),
            },
        ];
        let qualification = qualify_case(log, "q3dm1", &queries, &outputs);
        assert!(qualification.map_initialized);
        assert!(qualification.vm_executed);
        assert_eq!(qualification.protocol_replies, 1);
        assert!(qualification.demo_recorded);
        assert_eq!(qualification.render_frames, 1);
        let empty = qualify_case("", "q3dm1", &[], &[]);
        assert!(!empty.map_initialized);
        assert!(!empty.vm_executed);
    }

    #[test]
    fn rejects_arguments() {
        assert!(run(&["--bogus".to_owned()]).is_err());
    }

    #[test]
    fn observes_sandboxed_processes() {
        let directory = fsutil::make_temp_dir(&std::env::temp_dir(), "quake-sandbox-").expect("temp dir");
        let text = directory.to_string_lossy().into_owned();
        let environment = vec![("PATH".to_owned(), "/usr/bin:/bin".to_owned())];
        let mut child = spawn_observed(
            &[String::from("sh"), String::from("-c"), String::from("echo READY")],
            &text,
            &environment,
            10_000,
        )
        .expect("spawn");
        let _ = child.alive().expect("poll");
        let observation = child.finish().expect("finish");
        assert_eq!(observation.exit_code, Some(0));
        assert_eq!(observation.stdout, "READY\n");
        assert!(!observation.timeout);
        let again = child.finish().expect("memoized");
        assert_eq!(again.stdout, "READY\n");
        fsutil::remove_forced(&directory);
    }
}
