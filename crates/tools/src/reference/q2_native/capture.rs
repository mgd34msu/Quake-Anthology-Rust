//! Native q2repro observation (donor `tools/reference/q2-native/capture.ts`).
//!
//! Records provenance for the existing q2repro binaries, stages corpus
//! content, and runs four live cases (classic and rerelease modules crossed
//! with dedicated-network and singleplayer-save modes) under a private Xvfb
//! display: console checkpoints, screenshots, demos, UDP replies, and
//! save/load evidence. The live orchestration needs the q2repro build tree,
//! corpus archives, and Xvfb; the parsing, settings, output-inspection, and
//! check helpers are pure and tested.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::error::ToolsError;
use crate::fsutil;
use crate::json::{parse_json, Json};
use crate::reference::environment::{corpus_root, identify_file, observe_command, quake_typescript_root, source_root};
use crate::reference::q2_native::content::{stage_content, StagedContent};
use crate::reference::q2_native::process::{start_observed, ObservedProcess, ProcessObservation};
use crate::reference::q2_native::udp::{query, unused_port, UdpQuery};
use crate::reference::schema::{CommandObservation, FileIdentity};
use crate::time::now_iso;

/// q2repro source tree (donor `source`).
fn q2repro_source() -> String {
    format!("{}/q2repro", source_root())
}

/// q2repro build tree (donor `build`).
fn q2repro_build() -> String {
    format!("{}/build", q2repro_source())
}

/// q2 corpus directory (donor `corpus`).
fn q2_corpus() -> String {
    format!("{}/q2", corpus_root())
}

/// Live case mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaseMode {
    /// Dedicated server plus networked client.
    DedicatedNetwork,
    /// Local client with save and reload.
    SingleplayerSave,
}

impl CaseMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::DedicatedNetwork => "dedicated-network",
            Self::SingleplayerSave => "singleplayer-save",
        }
    }
}

/// A game-module definition: id, module path, and corpus archives.
#[derive(Debug, Clone)]
pub struct CaseDefinition {
    /// Stable definition id.
    pub id: &'static str,
    /// Game module path.
    pub module: String,
    /// Corpus archives, low to high priority.
    pub archives: Vec<String>,
}

/// Classic and rerelease module definitions.
#[must_use]
pub fn case_definitions() -> Vec<CaseDefinition> {
    let build = q2repro_build();
    let corpus = q2_corpus();
    vec![
        CaseDefinition {
            id: "classic",
            module: format!("{build}/baseq2/gamex86_64.so"),
            archives: ["baseq2/pak0.pak", "baseq2/pak1.pak", "baseq2/pak2.pak"]
                .into_iter()
                .map(|name| format!("{corpus}/{name}"))
                .collect(),
        },
        CaseDefinition {
            id: "rerelease",
            module: format!("{build}/baseq2/game_x86_64.so"),
            archives: vec![format!("{corpus}/rerelease/baseq2/pak0.pak")],
        },
    ]
}

/// A named process observation (donor `{ name, ...finish }` order).
#[derive(Debug, Clone)]
pub struct NamedObservation {
    /// Process name.
    pub name: String,
    /// Finish observation.
    pub observation: ProcessObservation,
}

impl NamedObservation {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        let mut pairs = vec![("name".to_owned(), Json::string(&self.name))];
        if let Json::Object(rest) = self.observation.to_json() {
            pairs.extend(rest);
        }
        Json::object(pairs)
    }
}

/// A captured output file.
#[derive(Debug, Clone)]
pub enum CaseOutput {
    /// PNG screenshot with decoded dimensions.
    Png {
        /// File identity.
        identity: FileIdentity,
        /// Image width.
        width: u32,
        /// Image height.
        height: u32,
    },
    /// Any other output file.
    File {
        /// File identity.
        identity: FileIdentity,
    },
}

impl CaseOutput {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        match self {
            Self::Png { identity, width, height } => Json::object(vec![
                ("kind".to_owned(), Json::string("png")),
                ("identity".to_owned(), identity.to_json()),
                ("width".to_owned(), Json::uint(u64::from(*width))),
                ("height".to_owned(), Json::uint(u64::from(*height))),
            ]),
            Self::File { identity } => Json::object(vec![
                ("kind".to_owned(), Json::string("file")),
                ("identity".to_owned(), identity.to_json()),
            ]),
        }
    }
}

/// Live case checks.
#[derive(Debug, Clone)]
pub struct CaseChecks {
    /// The checkpoint sequence completed without failure.
    pub checkpoint_sequence_completed: bool,
    /// At least two 640x480 screenshots were captured.
    pub screenshots_640x480: bool,
    /// A demo larger than 1024 bytes was written.
    pub demo_written: bool,
    /// Every query drew a loopback reply (network mode only).
    pub loopback_reply: bool,
    /// The client console shows a connection.
    pub native_client_connected: bool,
    /// The client console shows the software renderer.
    pub software_renderer_observed: bool,
    /// Save and reload evidence is present (save mode only).
    pub save_and_reload_observed: bool,
}

impl CaseChecks {
    /// Whether every check passed.
    #[must_use]
    pub fn all(&self) -> bool {
        self.checkpoint_sequence_completed
            && self.screenshots_640x480
            && self.demo_written
            && self.loopback_reply
            && self.native_client_connected
            && self.software_renderer_observed
            && self.save_and_reload_observed
    }

    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("checkpointSequenceCompleted".to_owned(), Json::boolean(self.checkpoint_sequence_completed)),
            ("screenshots640x480".to_owned(), Json::boolean(self.screenshots_640x480)),
            ("demoWritten".to_owned(), Json::boolean(self.demo_written)),
            ("loopbackReply".to_owned(), Json::boolean(self.loopback_reply)),
            ("nativeClientConnected".to_owned(), Json::boolean(self.native_client_connected)),
            ("softwareRendererObserved".to_owned(), Json::boolean(self.software_renderer_observed)),
            ("saveAndReloadObserved".to_owned(), Json::boolean(self.save_and_reload_observed)),
        ])
    }
}

/// Evaluate the live case checks over captured evidence.
#[must_use]
pub fn case_checks(
    mode: CaseMode,
    failure: Option<&str>,
    outputs: &[CaseOutput],
    queries: &[UdpQuery],
    observations: &[NamedObservation],
    port: u16,
) -> CaseChecks {
    let screenshots: Vec<&CaseOutput> =
        outputs.iter().filter(|output| matches!(output, CaseOutput::Png { .. })).collect();
    let demo = outputs.iter().find(|output| output.identity().path.ends_with(".dm2"));
    let client_output =
        observations.iter().find(|item| item.name == "client").map_or("", |item| item.observation.stdout.as_str());
    CaseChecks {
        checkpoint_sequence_completed: failure.is_none(),
        screenshots_640x480: screenshots.len() >= 2
            && screenshots
                .iter()
                .all(|output| matches!(output, CaseOutput::Png { width: 640, height: 480, .. })),
        demo_written: demo.is_some_and(|output| output.identity().size > 1024),
        loopback_reply: mode == CaseMode::SingleplayerSave
            || queries.iter().all(|item| {
                item.response.iter().any(|packet| packet.from == "127.0.0.1" && packet.port == port)
            }),
        native_client_connected: observations
            .iter()
            .any(|item| item.name == "client" && item.observation.stdout.contains("Connected to ")),
        software_renderer_observed: client_output.contains("llvmpipe"),
        save_and_reload_observed: mode == CaseMode::DedicatedNetwork
            || client_output.contains("Game saved.")
                && client_output.contains("Current map: base1")
                && client_output.matches("Connected to loopback").count() >= 2
                && outputs.iter().any(|output| {
                    output.identity().path.ends_with("/save/native-reference/base1.sav") && output.identity().size > 0
                })
                && outputs.iter().any(|output| {
                    output.identity().path.ends_with("/save/native-reference/game.ssv") && output.identity().size > 0
                }),
    }
}

impl CaseOutput {
    fn identity(&self) -> &FileIdentity {
        match self {
            Self::Png { identity, .. } | Self::File { identity } => identity,
        }
    }
}

/// Expand settings into `+set` argument triples, preserving order.
#[must_use]
pub fn settings(values: &[(&str, &str)]) -> Vec<String> {
    let mut args = Vec::with_capacity(values.len() * 3);
    for (key, value) in values {
        args.push("+set".to_owned());
        args.push((*key).to_owned());
        args.push((*value).to_owned());
    }
    args
}

/// Dependency path from one `ldd` line (donor `/(?:=>\s*)?(\/\S+)\s+\(/`).
#[must_use]
pub fn ldd_dependency(line: &str) -> Option<String> {
    let open = line.rfind('(')?;
    let head = line[..open].trim_end();
    let token = head.split_whitespace().next_back()?;
    if token.starts_with('/') {
        Some(token.to_owned())
    } else {
        None
    }
}

/// All regular files below `directory`, sorted; directory symlinks are not followed.
pub fn files_below(directory: &str) -> Result<Vec<String>, ToolsError> {
    let mut result = Vec::new();
    let mut stack = vec![directory.to_owned()];
    while let Some(current) = stack.pop() {
        let mut entries: Vec<(String, std::fs::FileType)> = Vec::new();
        let listing =
            std::fs::read_dir(&current).map_err(|error| ToolsError::io(format!("listing {current}"), error))?;
        for entry in listing {
            let entry = entry.map_err(|error| ToolsError::io(format!("listing {current}"), error))?;
            let file_type = entry
                .file_type()
                .map_err(|error| ToolsError::io(format!("stating {}", entry.path().display()), error))?;
            entries.push((entry.file_name().to_string_lossy().into_owned(), file_type));
        }
        entries.sort_by(|left, right| left.0.cmp(&right.0));
        for (name, file_type) in entries {
            let path = format!("{current}/{name}");
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

/// Compilation input paths from a parsed `compile_commands.json` document.
pub fn compilation_input_paths(document: &Json) -> Result<Vec<String>, ToolsError> {
    let Some(entries) = document.as_array() else {
        return Err(ToolsError::invalid("Expected compilation database array"));
    };
    let mut paths = std::collections::BTreeSet::new();
    for entry in entries {
        let (Some(directory), Some(file)) =
            (entry.get("directory").and_then(Json::as_str), entry.get("file").and_then(Json::as_str))
        else {
            return Err(ToolsError::invalid("Invalid compilation database entry"));
        };
        paths.insert(fsutil::lexical_absolute(Path::new(directory), file).to_string_lossy().into_owned());
    }
    Ok(paths.into_iter().collect())
}

/// Identify the current compilation inputs under `build`.
pub fn source_inputs(build: &Path) -> Result<Vec<FileIdentity>, ToolsError> {
    let document = parse_json(&fsutil::read_text(&build.join("compile_commands.json"))?)?;
    let mut identities = Vec::new();
    for path in compilation_input_paths(&document)? {
        identities.push(identify_file(&path)?);
    }
    Ok(identities)
}

/// Inspect home-directory outputs, validating PNG dimensions.
pub fn inspect_outputs(home: &str) -> Result<Vec<CaseOutput>, ToolsError> {
    let mut results = Vec::new();
    for path in files_below(home)? {
        let identity = identify_file(&path)?;
        if path.ends_with(".png") {
            let data = read_prefix(&path, 24)?;
            if data.len() != 24 || data[0..8] != [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a] {
                return Err(ToolsError::invalid(format!("Invalid PNG: {path}")));
            }
            let width = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
            let height = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
            results.push(CaseOutput::Png { identity, width, height });
        } else {
            results.push(CaseOutput::File { identity });
        }
    }
    Ok(results)
}

fn read_prefix(path: &str, count: usize) -> Result<Vec<u8>, ToolsError> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|error| ToolsError::io(format!("reading {path}"), error))?;
    let mut bytes = Vec::new();
    file.take(count as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| ToolsError::io(format!("reading {path}"), error))?;
    Ok(bytes)
}

/// A private Xvfb display and its process.
pub struct PrivateDisplay {
    /// Display name (`:number`).
    pub display: String,
    /// Display server process.
    pub process: ObservedProcess,
}

/// Start a private Xvfb display on an unused number in the reserved range.
pub fn private_display(directory: &str, environment: &[(String, String)]) -> Result<PrivateDisplay, ToolsError> {
    for index in 0..100_u32 {
        let number = 13000 + (std::process::id() % 10000) + index;
        let socket = format!("/tmp/.X11-unix/X{number}");
        let lock = format!("/tmp/.X{number}-lock");
        if Path::new(&socket).exists() || Path::new(&lock).exists() {
            continue;
        }
        let display = format!(":{number}");
        let mut child = start_observed(
            &[String::from("/usr/bin/Xvfb"), display.clone(), String::from("-screen"), String::from("0"), String::from("640x480x24"), String::from("-nolisten"), String::from("tcp"), String::from("-noreset")],
            directory,
            environment,
            Some(180_000),
        )?;
        let deadline = Instant::now() + Duration::from_millis(5000);
        let mut ready = false;
        while Instant::now() < deadline {
            if Path::new(&socket).exists() {
                ready = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        if ready {
            return Ok(PrivateDisplay { display, process: child });
        }
        let _ = child.finish(&format!("{directory}/xvfb-failed"));
        return Err(ToolsError::invalid("Private Xvfb did not create its socket"));
    }
    Err(ToolsError::invalid("No unused private display number in reserved search range"))
}

fn binary_observation(identity: FileIdentity, header: CommandObservation, linked: CommandObservation) -> Json {
    Json::object(vec![
        ("identity".to_owned(), identity.to_json()),
        ("header".to_owned(), header.to_json()),
        ("linked".to_owned(), linked.to_json()),
    ])
}

/// Record provenance for the existing q2repro binaries (donor `provenance`).
fn provenance(directory: &str) -> Result<Json, ToolsError> {
    let project = quake_typescript_root();
    let project_text = project.to_string_lossy().into_owned();
    let build = q2repro_build();
    let records = [
        "config.h",
        "build.ninja",
        "compile_commands.json",
        ".ninja_log",
        ".ninja_deps",
        "meson-info/intro-projectinfo.json",
        "meson-info/intro-targets.json",
        "meson-info/intro-compilers.json",
        "meson-info/intro-buildoptions.json",
        "meson-info/intro-dependencies.json",
        "meson-logs/meson-log.txt",
    ];
    let mut binaries = vec![format!("{build}/q2repro"), format!("{build}/q2reproded")];
    binaries.extend(case_definitions().into_iter().map(|definition| definition.module));
    let mut observations = Vec::with_capacity(binaries.len());
    let mut dependencies = std::collections::BTreeSet::new();
    for path in &binaries {
        let header = observe_command(
            &[String::from("file"), String::from("--brief"), path.clone()],
            &project_text,
            None,
        )?;
        let linked =
            observe_command(&[String::from("ldd"), path.clone()], &project_text, None)?;
        for line in linked.stdout.split('\n') {
            if let Some(dependency) = ldd_dependency(line) {
                dependencies.insert(dependency);
            }
        }
        observations.push(binary_observation(identify_file(path)?, header, linked));
    }
    let mut build_records = Vec::with_capacity(records.len());
    for record in records {
        build_records.push(identify_file(&format!("{build}/{record}"))?);
    }
    let mut dependency_files = Vec::with_capacity(dependencies.len());
    for path in &dependencies {
        dependency_files.push(identify_file(path)?);
    }
    let inventory_path = project.join("verification/reference-environment.json");
    let original_inventory = identify_file(&inventory_path.to_string_lossy())?;
    let captured_inventory_path = format!("{directory}/reference-environment.json");
    std::fs::copy(&inventory_path, &captured_inventory_path)
        .map_err(|error| ToolsError::io("copying the environment inventory", error))?;
    let environment_manifest = identify_file(&captured_inventory_path)?;
    if original_inventory.sha256 != environment_manifest.sha256 {
        return Err(ToolsError::invalid("Environment inventory changed during capture"));
    }
    Ok(Json::object(vec![
        ("classification".to_owned(), Json::string("independent-executable-observation")),
        ("binaries".to_owned(), Json::array(observations)),
        ("buildRecords".to_owned(), Json::array(build_records.iter().map(FileIdentity::to_json).collect())),
        ("buildConfigText".to_owned(), Json::string(fsutil::read_text(Path::new(&format!("{build}/config.h")))?)),
        (
            "currentCompilationInputs".to_owned(),
            Json::array(source_inputs(Path::new(&build))?.iter().map(FileIdentity::to_json).collect()),
        ),
        ("dependencies".to_owned(), Json::array(dependency_files.iter().map(FileIdentity::to_json).collect())),
        ("environmentManifest".to_owned(), environment_manifest.to_json()),
        (
            "limitation".to_owned(),
            Json::string(
                "Existing binaries were not rebuilt. Build records, current compilation inputs and source inventory do not prove that current source bytes produced these binaries. Header dependencies are represented by the existing Ninja dependency record, not a reconstructed source closure. No current-HEAD build equivalence is claimed.",
            ),
        ),
    ]))
}

/// Write an isolated home directory with neutral configs.
fn make_home(path: &str) -> Result<(), ToolsError> {
    std::fs::create_dir_all(format!("{path}/baseq2"))
        .map_err(|error| ToolsError::io(format!("creating {path}/baseq2"), error))?;
    for name in ["autoexec.cfg", "q2config.cfg", "config.cfg"] {
        fsutil::write_text(Path::new(&format!("{path}/baseq2/{name}")), "// isolated native reference\n")?;
    }
    Ok(())
}

fn sleep_ms(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

#[allow(clippy::too_many_lines)]
fn run_case(
    definition: &CaseDefinition,
    content_directory: &str,
    directory: &str,
    mode: CaseMode,
    display: &str,
) -> Result<Json, ToolsError> {
    std::fs::create_dir_all(directory).map_err(|error| ToolsError::io(format!("creating {directory}"), error))?;
    let server_home = format!("{directory}/server");
    let client_home = format!("{directory}/client");
    make_home(&server_home)?;
    make_home(&client_home)?;
    let port = unused_port()?;
    let build = q2repro_build();
    let port_text = port.to_string();
    let hostname = format!("q2-native-{}", definition.id);
    let common = [
        ("basedir", content_directory),
        ("libdir", build.as_str()),
        ("sys_forcegamelib", definition.module.as_str()),
        ("public", "0"),
        ("net_ip", "127.0.0.1"),
        ("net_enable_ipv6", "0"),
        ("net_port", port_text.as_str()),
        ("hostname", hostname.as_str()),
        ("allow_download", "0"),
        ("logfile", "0"),
        ("sys_console", "1"),
    ];
    let environment = [
        ("PATH", "/usr/bin:/bin"),
        ("LC_ALL", "C"),
        ("TZ", "UTC"),
        ("DISPLAY", display),
        ("SDL_VIDEODRIVER", "x11"),
        ("SDL_AUDIODRIVER", "dummy"),
        ("LIBGL_ALWAYS_SOFTWARE", "1"),
        ("MESA_LOADER_DRIVER_OVERRIDE", "llvmpipe"),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect::<Vec<(String, String)>>();
    let mut dedicated: Option<ObservedProcess> = None;
    let mut queries: Vec<UdpQuery> = Vec::new();
    let mut failure: Option<String> = None;
    let mut client_process: Option<ObservedProcess> = None;
    let run = (|| -> Result<(), ToolsError> {
        if mode == CaseMode::DedicatedNetwork {
            let mut server_settings = common.to_vec();
            server_settings.extend([("homedir", server_home.as_str()), ("deathmatch", "1"), ("maxclients", "4")]);
            let mut argv = vec![format!("{build}/q2reproded")];
            argv.extend(settings(&server_settings));
            argv.extend(["+map", "q2dm1", "+status", "+serverinfo", "+echo", "NATIVE_SERVER_READY"].into_iter().map(str::to_owned));
            dedicated = Some(start_observed(&argv, directory, &environment, None)?);
            let server = dedicated.as_mut().expect("stored dedicated server");
            server.wait_for("NATIVE_SERVER_READY", None)?;
            if !server.output().contains("Current map: q2dm1") {
                return Err(ToolsError::invalid("Dedicated map did not load"));
            }
            for request in ["status", "info 34", "getchallenge"] {
                queries.push(query(port, request)?);
            }
            if queries.iter().any(|item| item.response.is_empty()) {
                return Err(ToolsError::invalid("Missing UDP reference response"));
            }
        }
        let mut client_settings = common.to_vec();
        client_settings.extend([
            ("homedir", client_home.as_str()),
            ("deathmatch", "0"),
            ("s_enable", "0"),
            ("cl_autopause", "0"),
            ("cl_maxfps", "60"),
            ("cl_async", "0"),
            ("vid_driver", "x11"),
            ("vid_geometry", "640x480"),
            ("vid_fullscreen", "0"),
            ("r_screenshot_async", "0"),
            ("r_screenshot_format", "png"),
            ("con_notifytime", "0"),
            ("name", "native-reference"),
            ("cl_protocol", "0"),
        ]);
        let mut argv = vec![format!("{build}/q2repro")];
        argv.extend(settings(&client_settings));
        if mode == CaseMode::DedicatedNetwork {
            argv.extend(["+connect".to_owned(), format!("127.0.0.1:{port}")]);
        } else {
            argv.extend(["+map".to_owned(), "base1".to_owned()]);
        }
        argv.extend(["+echo".to_owned(), "NATIVE_CLIENT_READY".to_owned()]);
        client_process = Some(start_observed(&argv, directory, &environment, None)?);
        let client = client_process.as_mut().expect("stored client");
        client.wait_for("NATIVE_CLIENT_READY", None)?;
        sleep_ms(2500);
        client.send("strings\nviewpos\nrecord native-reference\necho NATIVE_RECORD_REQUESTED")?;
        client.wait_for("NATIVE_RECORD_REQUESTED", None)?;
        sleep_ms(300);
        client.send("screenshot png\n+forward\necho NATIVE_FORWARD_START")?;
        client.wait_for("NATIVE_FORWARD_START", None)?;
        sleep_ms(400);
        client.send("-forward\nviewpos\nscreenshot png\necho NATIVE_FORWARD_STOP")?;
        client.wait_for("NATIVE_FORWARD_STOP", None)?;
        sleep_ms(300);
        if mode == CaseMode::SingleplayerSave {
            client.send("save native-reference\necho NATIVE_SAVE_REQUESTED")?;
            client.wait_for("NATIVE_SAVE_REQUESTED", None)?;
            sleep_ms(300);
            client.send("load native-reference\necho NATIVE_LOAD_REQUESTED")?;
            client.wait_for("NATIVE_LOAD_REQUESTED", None)?;
            sleep_ms(500);
            client.send("viewpos\nstatus\nscreenshot png\necho NATIVE_RELOAD_OBSERVED")?;
            client.wait_for("NATIVE_RELOAD_OBSERVED", None)?;
        } else {
            let server = dedicated.as_mut().ok_or_else(|| ToolsError::invalid("Dedicated process absent"))?;
            server.send("status\nstatus p\nstatus t\nsv_fps\necho NATIVE_CLIENT_STATUS")?;
            server.wait_for("NATIVE_CLIENT_STATUS", None)?;
            queries.push(query(port, "status")?);
        }
        client.send("stop\necho NATIVE_CAPTURE_DONE")?;
        client.wait_for("NATIVE_CAPTURE_DONE", None)?;
        sleep_ms(200);
        client.send("quit")?;
        sleep_ms(200);
        Ok(())
    })();
    if let Err(error) = run {
        failure = Some(error.to_string());
    }
    let mut observations = Vec::new();
    if let Some(mut client) = client_process {
        observations.push(NamedObservation {
            name: "client".to_owned(),
            observation: client.finish(&format!("{directory}/client-logs"))?,
        });
    }
    if let Some(mut server) = dedicated {
        observations.push(NamedObservation {
            name: "dedicated".to_owned(),
            observation: server.finish(&format!("{directory}/dedicated-logs"))?,
        });
    }
    let mut outputs = inspect_outputs(&server_home)?;
    outputs.extend(inspect_outputs(&client_home)?);
    let checks = case_checks(mode, failure.as_deref(), &outputs, &queries, &observations, port);
    let observed = checks.all();
    let result = Json::object(vec![
        ("id".to_owned(), Json::string(format!("{}-{}", definition.id, mode.as_str()))),
        ("classification".to_owned(), Json::string("independent-executable-observation")),
        ("mode".to_owned(), Json::string(mode.as_str())),
        ("port".to_owned(), Json::uint(u64::from(port))),
        ("failure".to_owned(), failure.as_ref().map_or(Json::Null, Json::string)),
        ("checks".to_owned(), checks.to_json()),
        ("observed".to_owned(), Json::boolean(observed)),
        ("queries".to_owned(), Json::array(queries.iter().map(UdpQuery::to_json).collect())),
        ("processes".to_owned(), Json::array(observations.iter().map(NamedObservation::to_json).collect())),
        ("outputs".to_owned(), Json::array(outputs.iter().map(CaseOutput::to_json).collect())),
        (
            "limitations".to_owned(),
            Json::array(
                [
                    "Input durations and checkpoints use wall-clock scheduling; they are not a deterministic simulation clock.",
                    "Screenshots are native llvmpipe observations; no TypeScript renderer comparison or image tolerance is established.",
                    "Demo files contain native server messages; this lane records their bytes without claiming complete protocol decoding.",
                    "A save/load request is accepted only when its console and files show success; unsupported game save formats remain unsupported.",
                ]
                .into_iter()
                .map(Json::string)
                .collect(),
            ),
        ),
    ]);
    fsutil::write_text(Path::new(&format!("{directory}/case.json")), &format!("{}\n", result.render_pretty()))?;
    Ok(result)
}

/// Capture the four native cases (donor `capture`).
pub fn capture() -> Result<i32, ToolsError> {
    let project = quake_typescript_root();
    let run_id = now_iso().replace([':', '.'], "-");
    let directory = project.join(".artifacts/q2-native").join(run_id).to_string_lossy().into_owned();
    std::fs::create_dir_all(&directory).map_err(|error| ToolsError::io(format!("creating {directory}"), error))?;
    let identity = provenance(&directory)?;
    fsutil::write_text(Path::new(&format!("{directory}/provenance.json")), &format!("{}\n", identity.render_pretty()))?;
    let environment =
        [("PATH", "/usr/bin:/bin"), ("LC_ALL", "C"), ("TZ", "UTC")].into_iter().map(|(key, value)| (key.to_owned(), value.to_owned())).collect::<Vec<(String, String)>>();
    let display = private_display(&directory, &environment)?;
    let mut display_process = display.process;
    let run = (|| -> Result<(Vec<Json>, Vec<Json>), ToolsError> {
        let mut cases = Vec::new();
        let mut content = Vec::new();
        for definition in case_definitions() {
            let mounted = stage_content(
                &definition.archives,
                &format!("{directory}/{}/data", definition.id),
                &["maps/base1.bsp".to_owned(), "maps/q2dm1.bsp".to_owned()],
            )?;
            content.push(staged_with_id(definition.id, &mounted));
            for mode in [CaseMode::DedicatedNetwork, CaseMode::SingleplayerSave] {
                let result = run_case(&definition, &mounted.mount, &format!("{}/{}/{}", directory, definition.id, mode.as_str()), mode, &display.display)?;
                println!(
                    "{}",
                    Json::object(vec![
                        ("case".to_owned(), result.get("id").unwrap_or(&Json::Null).clone()),
                        ("observed".to_owned(), result.get("observed").unwrap_or(&Json::Null).clone()),
                        ("checks".to_owned(), result.get("checks").unwrap_or(&Json::Null).clone()),
                        ("failure".to_owned(), result.get("failure").unwrap_or(&Json::Null).clone()),
                    ])
                    .render()
                );
                cases.push(result);
            }
        }
        Ok((cases, content))
    })();
    let display_observation = display_process.finish(&format!("{directory}/xvfb"))?;
    let (cases, content) = run?;
    let tools_directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/reference/q2_native");
    let mut tools = Vec::new();
    for path in files_below(&tools_directory.to_string_lossy())? {
        if path.ends_with(".rs") {
            tools.push(identify_file(&path)?);
        }
    }
    for name in ["environment.rs", "schema.rs"] {
        tools.push(identify_file(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("src/reference").join(name).to_string_lossy(),
        )?);
    }
    let mut display_pairs = vec![("binary".to_owned(), identify_file("/usr/bin/Xvfb")?.to_json())];
    if let Json::Object(rest) = display_observation.to_json() {
        display_pairs.extend(rest);
    }
    let result = Json::object(vec![
        ("schemaVersion".to_owned(), Json::int(1)),
        ("capturedAt".to_owned(), Json::string(now_iso())),
        ("command".to_owned(), Json::array(std::env::args().map(Json::string).collect())),
        ("artifactDirectory".to_owned(), Json::string(&directory)),
        (
            "runtime".to_owned(),
            identify_file(&std::env::current_exe().map_err(|error| ToolsError::io("resolving current executable", error))?.to_string_lossy())?.to_json(),
        ),
        ("tools".to_owned(), Json::array(tools.iter().map(FileIdentity::to_json).collect())),
        ("provenance".to_owned(), identify_file(&format!("{directory}/provenance.json"))?.to_json()),
        (
            "sourceAttribution".to_owned(),
            identity.get("limitation").unwrap_or(&Json::Null).clone(),
        ),
        ("content".to_owned(), Json::array(content)),
        ("display".to_owned(), Json::object(display_pairs)),
        ("cases".to_owned(), Json::array(cases.clone())),
        (
            "unsupportedReferences".to_owned(),
            Json::array(
                [
                    "Retail Windows quake2.exe and quake2ex_steam.exe were not executed.",
                    "Mission packs, multiplayer interoperability with the TypeScript engine, numerical gameplay equivalence, controlled frame timing, and renderer tolerances are not established.",
                ]
                .into_iter()
                .map(Json::string)
                .collect(),
            ),
        ),
    ]);
    fsutil::write_text(Path::new(&format!("{directory}/capture.json")), &format!("{}\n", result.render_pretty()))?;
    let destination = project.join("verification/reference-cases/q2-native/latest.json");
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| ToolsError::io(format!("creating {}", parent.display()), error))?;
    }
    fsutil::write_text(&destination, &format!("{}\n", result.render_pretty()))?;
    let observed_cases = cases.iter().filter(|case| case.get("observed").and_then(Json::as_bool) == Some(true)).count();
    println!(
        "{}",
        Json::object(vec![
            ("manifest".to_owned(), Json::string(destination.to_string_lossy())),
            ("artifacts".to_owned(), Json::string(&directory)),
            ("observedCases".to_owned(), Json::uint(observed_cases as u64)),
        ])
        .render()
    );
    Ok(if observed_cases == cases.len() { 0 } else { 1 })
}

fn staged_with_id(id: &str, staged: &StagedContent) -> Json {
    let mut pairs = vec![("id".to_owned(), Json::string(id))];
    if let Json::Object(rest) = staged.to_json() {
        pairs.extend(rest);
    }
    Json::object(pairs)
}

/// Run the native capture (donor `main` ignores arguments).
pub fn run() -> Result<i32, ToolsError> {
    capture()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::reference::q2_native::udp::UdpPacket;

    use super::*;

    fn temp_dir(prefix: &str) -> PathBuf {
        fsutil::make_temp_dir(&std::env::temp_dir(), prefix).expect("temp dir")
    }

    #[test]
    fn expands_settings_in_order() {
        assert_eq!(settings(&[("a", "1"), ("b", "2")]), vec!["+set", "a", "1", "+set", "b", "2"]);
    }

    #[test]
    fn parses_ldd_dependencies() {
        assert_eq!(
            ldd_dependency("libSDL2-2.0.so.0 => /lib/x86_64-linux-gnu/libSDL2-2.0.so.0 (0x00007f1b2c000000)").as_deref(),
            Some("/lib/x86_64-linux-gnu/libSDL2-2.0.so.0")
        );
        assert_eq!(
            ldd_dependency("/lib64/ld-linux-x86-64.so.2 (0x00007f1b2d000000)").as_deref(),
            Some("/lib64/ld-linux-x86-64.so.2")
        );
        assert_eq!(ldd_dependency("linux-vdso.so.1 (0x00007ffc12345000)"), None);
        assert_eq!(ldd_dependency("libmissing.so => not found"), None);
        assert_eq!(ldd_dependency(""), None);
    }

    #[test]
    fn lists_files_sorted_without_following_links() {
        let directory = temp_dir("quake-files-below-");
        let text = directory.to_string_lossy().into_owned();
        fsutil::write_text(&directory.join("b.txt"), "b").expect("write");
        std::fs::create_dir(directory.join("sub")).expect("mkdir");
        fsutil::write_text(&directory.join("sub/a.txt"), "a").expect("write");
        let files = files_below(&text).expect("list");
        assert_eq!(files, vec![format!("{text}/b.txt"), format!("{text}/sub/a.txt")]);
        fsutil::remove_forced(&directory);
    }

    #[test]
    fn validates_compilation_databases() {
        let document = parse_json(r#"[{"directory": "/tmp/build", "file": "../src/a.c"}, {"directory": "/tmp/build", "file": "/abs/b.c"}]"#).unwrap();
        assert_eq!(compilation_input_paths(&document).unwrap(), vec!["/abs/b.c".to_owned(), "/tmp/src/a.c".to_owned()]);
        assert!(compilation_input_paths(&parse_json(r#"{"not": "array"}"#).unwrap()).is_err());
        assert!(compilation_input_paths(&parse_json(r#"[{"directory": "/tmp"}]"#).unwrap()).is_err());
    }

    fn png_bytes(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13, b'I', b'H', b'D', b'R'];
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes
    }

    #[test]
    fn inspects_outputs_and_validates_png() {
        let directory = temp_dir("quake-outputs-");
        let text = directory.to_string_lossy().into_owned();
        fsutil::write_bytes(&directory.join("shot.png"), &png_bytes(640, 480)).expect("write");
        fsutil::write_bytes(&directory.join("demo.dm2"), &[1; 2048]).expect("write");
        let outputs = inspect_outputs(&text).expect("inspect");
        assert_eq!(outputs.len(), 2);
        assert!(outputs.iter().any(|output| matches!(output, CaseOutput::Png { width: 640, height: 480, .. })));
        fsutil::write_bytes(&directory.join("bad.png"), b"short").expect("write");
        assert!(inspect_outputs(&text).is_err());
        fsutil::remove_forced(&directory);
    }

    fn named_observation(name: &str, stdout: &str) -> NamedObservation {
        NamedObservation {
            name: name.to_owned(),
            observation: ProcessObservation {
                command: Vec::new(),
                cwd: String::new(),
                environment: Vec::new(),
                pid: 1,
                started_at: String::new(),
                duration_ms: 0.0,
                exit_code: Some(0),
                timeout: false,
                events: Vec::new(),
                stdout: stdout.to_owned(),
                stderr: String::new(),
            },
        }
    }

    fn file_output(path: &str, size: u64) -> CaseOutput {
        CaseOutput::File {
            identity: FileIdentity { path: path.to_owned(), size, sha256: String::new() },
        }
    }

    #[test]
    fn evaluates_network_case_checks() {
        let port = 27910;
        let packet = UdpPacket { hex: String::new(), text: "reply".to_owned(), from: "127.0.0.1".to_owned(), port, elapsed_ms: 1.0 };
        let queries = vec![UdpQuery {
            destination: String::new(),
            request: "status".to_owned(),
            sent_hex: String::new(),
            response: vec![packet],
        }];
        let home = "/tmp/home";
        let png = |name: &str| CaseOutput::Png {
            identity: FileIdentity { path: format!("{home}/{name}"), size: 10, sha256: String::new() },
            width: 640,
            height: 480,
        };
        let outputs = vec![png("a.png"), png("b.png"), file_output(&format!("{home}/native-reference.dm2"), 2048)];
        let observations =
            vec![named_observation("client", "Connected to 127.0.0.1\nllvmpipe\n"), named_observation("dedicated", "")];
        let checks = case_checks(CaseMode::DedicatedNetwork, None, &outputs, &queries, &observations, port);
        assert!(checks.all(), "{:?}", checks.to_json().render());
        let failed = case_checks(CaseMode::DedicatedNetwork, Some("boom"), &outputs, &queries, &observations, port);
        assert!(!failed.checkpoint_sequence_completed);
        assert!(!failed.all());
    }

    #[test]
    fn evaluates_save_case_checks() {
        let home = "/tmp/home";
        let png = |name: &str| CaseOutput::Png {
            identity: FileIdentity { path: format!("{home}/{name}"), size: 10, sha256: String::new() },
            width: 640,
            height: 480,
        };
        let outputs = vec![
            png("a.png"),
            png("b.png"),
            png("c.png"),
            file_output(&format!("{home}/native-reference.dm2"), 2048),
            file_output(&format!("{home}/save/native-reference/base1.sav"), 100),
            file_output(&format!("{home}/save/native-reference/game.ssv"), 100),
        ];
        let stdout = "Connected to loopback\nGame saved.\nCurrent map: base1\nConnected to loopback\nllvmpipe\n";
        let observations = vec![named_observation("client", stdout)];
        let checks = case_checks(CaseMode::SingleplayerSave, None, &outputs, &[], &observations, 1);
        assert!(checks.all(), "{:?}", checks.to_json().render());
        assert!(checks.loopback_reply);
        let missing = case_checks(CaseMode::SingleplayerSave, None, &outputs[..3], &[], &observations, 1);
        assert!(!missing.demo_written);
        assert!(!missing.save_and_reload_observed);
    }
}

