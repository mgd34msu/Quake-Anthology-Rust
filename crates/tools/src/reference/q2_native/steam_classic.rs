//! Retail classic observation under Wine (donor `tools/reference/q2-native/steam-classic.ts`).
//!
//! Stages the Steam classic installation into an isolated directory with a
//! fresh Wine prefix, boots the prefix, runs the retail dedicated server on
//! a loopback port, issues connectionless protocol requests, and records
//! the console plus process observations. The live orchestration needs the
//! retail installation and Proton runtime; the check evaluation is pure and
//! tested.

use std::path::Path;

use crate::error::ToolsError;
use crate::fsutil;
use crate::json::Json;
use crate::reference::environment::{identify_file, quake_typescript_root};
use crate::reference::q2_native::capture::private_display;
use crate::reference::q2_native::content::stage_content;
use crate::reference::q2_native::process::{start_observed, ProcessObservation};
use crate::reference::q2_native::udp::{query, unused_port, UdpQuery};
use crate::reference::schema::FileIdentity;
use crate::time::now_iso;

/// Steam `steamapps/common` directory (donor verbatim machine-local path).
const STEAM_COMMON: &str = "/home/buzzkill/.local/share/Steam/steamapps/common";

/// Retail classic dedicated-server checks.
#[derive(Debug, Clone)]
pub struct SteamClassicChecks {
    /// Console shows the q2dm1 spawn.
    pub loaded_q2dm1: bool,
    /// Console shows the base1 spawn.
    pub loaded_base1: bool,
    /// A status reply advertises protocol 34.
    pub protocol34: bool,
    /// A challenge reply was observed.
    pub classic_challenge: bool,
    /// Every non-quit query drew a loopback reply.
    pub loopback_replies: bool,
}

impl SteamClassicChecks {
    /// Whether every check passed.
    #[must_use]
    pub fn all(&self) -> bool {
        self.loaded_q2dm1 && self.loaded_base1 && self.protocol34 && self.classic_challenge && self.loopback_replies
    }

    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("loadedQ2dm1".to_owned(), Json::boolean(self.loaded_q2dm1)),
            ("loadedBase1".to_owned(), Json::boolean(self.loaded_base1)),
            ("protocol34".to_owned(), Json::boolean(self.protocol34)),
            ("classicChallenge".to_owned(), Json::boolean(self.classic_challenge)),
            ("loopbackReplies".to_owned(), Json::boolean(self.loopback_replies)),
        ])
    }
}

/// Evaluate the retail checks over console text and query replies.
#[must_use]
pub fn steam_classic_checks(console_text: &str, queries: &[UdpQuery], port: u16) -> SteamClassicChecks {
    SteamClassicChecks {
        loaded_q2dm1: console_text.contains("SpawnServer: q2dm1"),
        loaded_base1: console_text.contains("SpawnServer: base1"),
        protocol34: queries.iter().any(|item| {
            item.request == "status" && item.response.iter().any(|packet| packet.text.contains("\\protocol\\34"))
        }),
        classic_challenge: queries.iter().any(|item| {
            item.request == "getchallenge" && item.response.iter().any(|packet| packet.text.contains("challenge "))
        }),
        loopback_replies: queries.len() >= 7
            && queries
                .iter()
                .filter(|item| !item.request.ends_with("quit"))
                .all(|item| {
                    !item.response.is_empty()
                        && item.response.iter().all(|packet| packet.from == "127.0.0.1" && packet.port == port)
                }),
    }
}

fn staged_binary(original: &FileIdentity, staged: &FileIdentity) -> Json {
    Json::object(vec![("original".to_owned(), original.to_json()), ("staged".to_owned(), staged.to_json())])
}

/// Capture the retail classic dedicated server (donor `captureSteamClassic`).
pub fn capture_steam_classic() -> Result<i32, ToolsError> {
    let project = quake_typescript_root();
    let retail = Path::new(STEAM_COMMON).join("Quake 2");
    let runtime = Path::new(STEAM_COMMON).join("Proton - Experimental/files/bin");
    let run_id = now_iso().replace([':', '.'], "-");
    let directory = project.join(".artifacts/q2-retail").join(run_id);
    let stage = directory.join("game");
    let prefix = directory.join("prefix");
    std::fs::create_dir_all(&prefix).map_err(|error| ToolsError::io(format!("creating {}", prefix.display()), error))?;
    let archives = ["baseq2/pak0.pak", "baseq2/pak1.pak", "baseq2/pak2.pak"]
        .into_iter()
        .map(|name| retail.join(name).to_string_lossy().into_owned())
        .collect::<Vec<String>>();
    let content = stage_content(
        &archives,
        &stage.to_string_lossy(),
        &["maps/q2dm1.bsp".to_owned(), "maps/base1.bsp".to_owned()],
    )?;
    let mut binaries = Vec::new();
    for name in ["quake2.exe", "baseq2/gamex86.dll"] {
        let original = identify_file(&retail.join(name).to_string_lossy())?;
        let staged_path = stage.join(name);
        if let Some(parent) = staged_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| ToolsError::io(format!("creating {}", parent.display()), error))?;
        }
        std::fs::copy(retail.join(name), &staged_path)
            .map_err(|error| ToolsError::io(format!("staging {name}"), error))?;
        let staged = identify_file(&staged_path.to_string_lossy())?;
        if staged.sha256 != original.sha256 {
            return Err(ToolsError::invalid(format!("Retail staging changed bytes: {name}")));
        }
        binaries.push((original, staged));
    }
    for name in ["autoexec.cfg", "config.cfg"] {
        fsutil::write_text(&stage.join("baseq2").join(name), "// isolated retail reference\n")?;
    }
    let mut environment = vec![
        ("PATH".to_owned(), "/usr/bin:/bin".to_owned()),
        ("LC_ALL".to_owned(), "C".to_owned()),
        ("TZ".to_owned(), "UTC".to_owned()),
        ("WINEPREFIX".to_owned(), prefix.to_string_lossy().into_owned()),
        ("WINEDEBUG".to_owned(), "-all,err+all,warn+all".to_owned()),
        ("WINEDLLOVERRIDES".to_owned(), "winemenubuilder.exe=d;winegstreamer=d;mscoree=d;mshtml=d".to_owned()),
        ("LIBGL_ALWAYS_SOFTWARE".to_owned(), "1".to_owned()),
        ("MESA_LOADER_DRIVER_OVERRIDE".to_owned(), "llvmpipe".to_owned()),
        ("SDL_AUDIODRIVER".to_owned(), "dummy".to_owned()),
    ];
    let directory_text = directory.to_string_lossy().into_owned();
    let display = private_display(&directory_text, &environment)?;
    environment.push(("DISPLAY".to_owned(), display.display.clone()));
    let wine = runtime.join("wine").to_string_lossy().into_owned();
    let mut initialization = start_observed(
        &[wine.clone(), "wineboot".to_owned(), "--init".to_owned()],
        &directory_text,
        &environment,
        None,
    )?;
    initialization.wait_for_exit()?;
    let initialization_observation = initialization.finish(&format!("{directory_text}/wineboot"))?;
    let port = unused_port()?;
    let stage_text = stage.to_string_lossy().into_owned();
    let mut command = vec![wine.clone(), format!("{stage_text}/quake2.exe")];
    let port_text = port.to_string();
    command.extend(
        [
            "+set",
            "basedir",
            ".",
            "+set",
            "dedicated",
            "1",
            "+set",
            "public",
            "0",
            "+set",
            "ip",
            "127.0.0.1",
            "+set",
            "port",
            port_text.as_str(),
            "+set",
            "noipx",
            "1",
            "+set",
            "logfile",
            "2",
            "+set",
            "rcon_password",
            "native-reference",
            "+set",
            "deathmatch",
            "1",
            "+set",
            "maxclients",
            "4",
            "+set",
            "hostname",
            "steam-q2-native-reference",
            "+map",
            "q2dm1",
            "+status",
            "+echo",
            "RETAIL_READY",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    let mut child = start_observed(&command, &stage_text, &environment, None)?;
    let mut display_process = display.process;
    let mut queries: Vec<UdpQuery> = Vec::new();
    let mut console_text = String::new();
    let mut failure: Option<String> = None;
    let run = (|| -> Result<(), ToolsError> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(40_000);
        let log = stage.join("baseq2/qconsole.log");
        while !console_text.contains("RETAIL_READY") {
            console_text = fsutil::read_text(&log).unwrap_or_default();
            if std::time::Instant::now() > deadline {
                let hole: Vec<char> = console_text.chars().collect();
                let console_tail: String = hole[hole.len().saturating_sub(2500)..].iter().collect();
                let output = child.output();
                let out: Vec<char> = output.chars().collect();
                let output_tail: String = out[out.len().saturating_sub(2500)..].iter().collect();
                return Err(ToolsError::invalid(format!("Retail map checkpoint timed out: {console_tail} {output_tail}")));
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        for request in ["status", "info 34", "getchallenge", "rcon native-reference status", "rcon native-reference serverinfo"] {
            queries.push(query(port, request)?);
        }
        queries.push(query(port, "rcon native-reference map base1")?);
        std::thread::sleep(std::time::Duration::from_millis(500));
        queries.push(query(port, "rcon native-reference status")?);
        queries.push(query(port, "rcon native-reference quit")?);
        std::thread::sleep(std::time::Duration::from_millis(300));
        console_text = fsutil::read_text(&log)?;
        Ok(())
    })();
    if let Err(error) = run {
        failure = Some(error.to_string());
    }
    let process_observation = child.finish(&format!("{directory_text}/wine"))?;
    let wineserver = runtime.join("wineserver").to_string_lossy().into_owned();
    let mut cleanup = start_observed(&[wineserver, "-k".to_owned()], &directory_text, &environment, None)?;
    std::thread::sleep(std::time::Duration::from_millis(100));
    let server_cleanup = cleanup.finish(&format!("{directory_text}/wineserver-cleanup"))?;
    let display_observation = display_process.finish(&format!("{directory_text}/xvfb"))?;
    let checks = steam_classic_checks(&console_text, &queries, port);
    let mut inputs_after = Vec::new();
    for (original, _) in &binaries {
        inputs_after.push(identify_file(&original.path)?);
    }
    let observed = failure.is_none() && checks.all();
    let capture_program =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/reference/q2_native/steam_classic.rs").to_string_lossy().into_owned();
    let exe = std::env::current_exe().map_err(|error| ToolsError::io("resolving current executable", error))?;
    let result = Json::object(vec![
        ("schemaVersion".to_owned(), Json::int(1)),
        ("capturedAt".to_owned(), Json::string(now_iso())),
        ("classification".to_owned(), Json::string("retail-executable-observation-under-wine")),
        ("command".to_owned(), Json::array(std::env::args().map(Json::string).collect())),
        ("captureProgram".to_owned(), identify_file(&capture_program)?.to_json()),
        ("runtime".to_owned(), identify_file(&exe.to_string_lossy())?.to_json()),
        ("wine".to_owned(), identify_file(&wine)?.to_json()),
        (
            "wineserver".to_owned(),
            identify_file(&runtime.join("wineserver").to_string_lossy())?.to_json(),
        ),
        ("binaries".to_owned(), Json::array(binaries.iter().map(|(original, staged)| staged_binary(original, staged)).collect())),
        ("inputsAfter".to_owned(), Json::array(inputs_after.iter().map(FileIdentity::to_json).collect())),
        ("content".to_owned(), content.to_json()),
        ("port".to_owned(), Json::uint(u64::from(port))),
        ("failure".to_owned(), failure.as_ref().map_or(Json::Null, Json::string)),
        ("checks".to_owned(), checks.to_json()),
        ("observed".to_owned(), Json::boolean(observed)),
        (
            "processes".to_owned(),
            Json::object(vec![
                ("initialization".to_owned(), initialization_observation.to_json()),
                ("game".to_owned(), process_observation.to_json()),
                ("cleanup".to_owned(), server_cleanup.to_json()),
                ("display".to_owned(), display_observation.to_json()),
            ]),
        ),
        ("queries".to_owned(), Json::array(queries.iter().map(UdpQuery::to_json).collect())),
        ("consoleText".to_owned(), Json::string(&console_text)),
        ("artifactDirectory".to_owned(), Json::string(&directory_text)),
        (
            "limits".to_owned(),
            Json::array(
                [
                    "The supplied Steam classic executable ran through the existing Proton Wine runtime in a fresh prefix; this is not a native Windows platform observation.",
                    "Only dedicated q2dm1/base1 loading and connectionless protocol 34 requests are covered. Retail rendering, movement, saves, and complete client signon are not covered.",
                    "The rerelease Steam executable is not executed by this case. Existing q2repro rerelease-module captures remain a separate observation.",
                    "Wine may initialize Windows components inside this fresh prefix. Steam app files, the Proton installation, and real Steam compatibility profiles are never write targets.",
                ]
                .into_iter()
                .map(Json::string)
                .collect(),
            ),
        ),
    ]);
    fsutil::write_text(&directory.join("capture.json"), &format!("{}\n", result.render_pretty()))?;
    let destination = project.join("verification/reference-cases/q2-native/steam-classic.json");
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| ToolsError::io(format!("creating {}", parent.display()), error))?;
    }
    fsutil::write_text(&destination, &format!("{}\n", result.render_pretty()))?;
    let summary = Json::object(vec![
        ("manifest".to_owned(), Json::string(destination.to_string_lossy())),
        ("observed".to_owned(), Json::boolean(observed)),
        ("checks".to_owned(), checks.to_json()),
        ("failure".to_owned(), failure.as_ref().map_or(Json::Null, Json::string)),
    ]);
    println!("{}", summary.render());
    Ok(i32::from(!observed))
}

/// Run the retail capture (donor `main` ignores arguments).
pub fn run() -> Result<i32, ToolsError> {
    capture_steam_classic()
}

#[cfg(test)]
mod tests {
    use crate::reference::q2_native::udp::UdpPacket;

    use super::*;

    fn packet(text: &str, port: u16) -> UdpPacket {
        UdpPacket { hex: String::new(), text: text.to_owned(), from: "127.0.0.1".to_owned(), port, elapsed_ms: 1.0 }
    }

    fn query_with(request: &str, packets: Vec<UdpPacket>) -> UdpQuery {
        UdpQuery {
            destination: "127.0.0.1:27910".to_owned(),
            request: request.to_owned(),
            sent_hex: String::new(),
            response: packets,
        }
    }

    #[test]
    fn evaluates_retail_checks() {
        let port = 27910;
        let queries = vec![
            query_with("status", vec![packet("\\protocol\\34\\hostname\\x", port)]),
            query_with("info 34", vec![packet("info", port)]),
            query_with("getchallenge", vec![packet("challenge 1234", port)]),
            query_with("rcon native-reference status", vec![packet("status", port)]),
            query_with("rcon native-reference serverinfo", vec![packet("serverinfo", port)]),
            query_with("rcon native-reference map base1", vec![packet("map", port)]),
            query_with("rcon native-reference status", vec![packet("status", port)]),
            query_with("rcon native-reference quit", vec![]),
        ];
        let console = "SpawnServer: q2dm1\nSpawnServer: base1\nRETAIL_READY\n";
        let checks = steam_classic_checks(console, &queries, port);
        assert!(checks.all(), "{:?}", checks.to_json().render());
        let missing = steam_classic_checks("SpawnServer: q2dm1\n", &queries[..2].to_vec(), port);
        assert!(!missing.all());
        assert!(!missing.loaded_base1);
        assert!(!missing.classic_challenge);
        let remote = query_with("status", vec![packet("x", 1234)]);
        let foreign = steam_classic_checks(console, &[remote], port);
        assert!(!foreign.loopback_replies);
    }
}
