//! Command-line options ported from `src/app/bootstrap/options.ts`,
//! `src/app/bootstrap/startup-commands.ts`,
//! `src/app/bootstrap/weapon-behavior-tool-options.ts`,
//! `src/contracts/mods.ts` (`readModSelection`), and
//! `src/network/q1/profile.ts` (`defaultNetQuakeProfile`).
//!
//! Wire identities reuse [`qa_net::protocol::ProtocolIdentity`]; `+command`
//! lexing reuses the [`qa_core::cmd`] text layer.

use std::path::{Path, PathBuf};

use qa_content::catalog::RemoteContentSelection;
use qa_core::cmd::{ascii_fold, command_separator_offset, source_command_text, tokenize_command, Dialect, TextMode};
use qa_net::protocol::{q1, ProtocolIdentity};

use crate::error::AppError;
use crate::persistence::mods;

/// Application help, adapted from the donor's `applicationHelp`.
pub const HELP: &str = "Quake

Usage: qa-muse [options]

  +command [arguments]       Run source startup command (use '+bind x \"+attack\"' as one shell argument)
  --menu                     Open the startup menu (default without launch selections)
  --preset q2-q1-q3|q1-q2     Select an initial mixed-game profile
  --content-root PATH        Game data root (default: beside the executable)
  --user-content-root PATH   Writable user content root
  --game PRODUCT             Installed catalog product, e.g. q2-classic-baseq2
  --map-game PRODUCT         Select map content independently from the game module
  --map NAME                 Map name or maps/path.bsp
  --progs MOUNTED_PATH       Validated mounted QuakeC .dat artifact
  --q2-game MOUNTED_PATH     Explicit Quake II game DLL (classic i386 / rerelease x64)
  --weapon-behavior PRODUCT/ID  Overlay a declared projectile trajectory
  --mod PRODUCT/ID           Enable a declared mod component (repeat to combine)
  --movement q1|q2|q3|qw|PRODUCT  Movement family or exact installed product
  --character q1|q2|q3       Player character provider
  --model NAME               Character model (e.g. sarge or male)
  --renderer cpu|gl          Renderer (default gl)
  --render-worker 0|1       Execute rendering on a worker (default 0)
  --gamma N                 Display gamma, 0.5 through 3 (default 1; higher is brighter)
  --width N --height N       Window dimensions (default 960 by 600)
  --seats N                  Local seats, 1 through 4
  --mode singleplayer|coop|deathmatch
  --server-profile PATH      Load validated shared server settings from JSON
  --rules standard|ctf|lmctf|tag|deathball|horde Source match rules
  --skill 0|1|2|3            Quake I/II gameplay difficulty
  --bot-skill 1|2|3|4|5      Quake III bot difficulty (default 2)
  --dedicated                Run without a window or local seats
  --listen-unified PORT      Host the selected mixed-game recipe
  --connect-unified ADDRESS  Join a mixed-game server
  --listen PORT              Host the selected game's native source protocol
  --q1-protocol 15|666|999    NetQuake host protocol (default 15; RMQ flags 130)
  --listen-q2 PORT           Host the native Quake II source protocol
  --bind ADDRESS             Server IP (default 0.0.0.0)
  --connect-q1 ADDRESS       Join a native Quake server (id1, protocols 15/666/999)
  --connect-qw ADDRESS       Join a base QuakeWorld protocol 28 server
  --connect-q3 ADDRESS       Join a baseq3 protocol 68 server (sv_pure 0)
  --q2-protocol 34|35[:1904|1905]|36[:revision]|4038|1038|2023 Q2 client/server protocol (35 defaults to 1904; 36 to 1026)
  --connect-q2 ADDRESS       Join a native Quake II server
  --ipx-dosbox HOST[:PORT]   Use DOSBox IPXNET relay (default relay port 213)
  --ipx-native               Require a host AF_IPX socket capability
  --seed N                   Gameplay random seed
  --frames N                 Close after N simulation steps
  --hidden                   Start a hidden native window
  --list-content             Show installed games and expansions
  --version                  Show the application version
  weapon-behavior --help     Inspect and author mounted source behavior declarations
  --help                     Show these options
";

/// Weapon-behavior tool help, ported from `weaponBehaviorToolHelp`.
pub const WEAPON_BEHAVIOR_HELP: &str = "Usage:
  qa-muse weapon-behavior inspect PRODUCT [options]
  qa-muse weapon-behavior declare-qvm PRODUCT --profile MOUNTED_PROFILE_JSON [options]
  qa-muse weapon-behavior declare-native PRODUCT --profile MOUNTED_PROFILE_JSON [options]
  qa-muse weapon-behavior declare PRODUCT --id NAMESPACE:ID --role ROLE --fire CALLBACK [options]

  --content PATH        Installed content root (default: beside the executable)
  --user-content PATH   Writable user content root
  --artifact PATH       Mounted program (QC descriptor/progs.dat/qwprogs.dat, vm/qagame.qvm, or native game DLL)
  --profile PATH        Author-written mounted QVM or API2023 Windows x64 native profile with exact digest, entries and entity layout
  --title TEXT          Display title (defaults to declaration ID)
  --activate CALLBACK   Optional source activation callback
  --role ROLE           rocket, grenade, nail, bolt, plasma, energy, grapple
  --help                Show this help

Inspection reports actual bytecode callbacks and think assignments. It does not infer
projectile roles or activation gates. Declare only callbacks whose behavior you have
established from the source. Declarations bind the exact current program digest.
";

/// Game family (`GameFamily` contract).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GameFamily {
    /// Quake I.
    Q1,
    /// Quake II.
    Q2,
    /// Quake III.
    Q3,
}

/// Renderer selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Renderer {
    /// Software rasterizer.
    Cpu,
    /// Hardware GL.
    Gl,
}

/// How the renderer was chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RendererSelection {
    /// Default.
    Default,
    /// Explicit `--renderer` flag.
    Explicit,
}

/// Game mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GameMode {
    /// Single player.
    Singleplayer,
    /// Cooperative.
    Coop,
    /// Deathmatch.
    Deathmatch,
}

/// Q3 application product (donor `src/core/q3-product-policy.ts`).
pub use crate::bootstrap::content::Q3ApplicationProduct;
/// Match rules (canonical donor `src/app/bootstrap/match-modes.ts`).
pub use crate::bootstrap::match_modes::MatchRules;

/// Network role selected on the command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Network {
    /// Offline local play.
    Offline,
    /// Host the selected game's native source protocol.
    NativeServer {
        /// Bind address.
        host: String,
        /// Bind port.
        port: u16,
    },
    /// Host the native Quake II source protocol.
    Q2Server {
        /// Bind address.
        host: String,
        /// Bind port.
        port: u16,
    },
    /// Host the selected mixed-game recipe.
    UnifiedServer {
        /// Bind address.
        host: String,
        /// Bind port.
        port: u16,
    },
    /// Join a native Quake server.
    Q1Client {
        /// Remote address.
        remote: String,
    },
    /// Join a QuakeWorld server.
    QwClient {
        /// Remote address.
        remote: String,
    },
    /// Join a native Quake II server.
    Q2Client {
        /// Remote address.
        remote: String,
    },
    /// Join a baseq3 server.
    Q3Client {
        /// Remote address.
        remote: String,
    },
    /// Join a mixed-game server.
    UnifiedClient {
        /// Remote address.
        remote: String,
    },
}

/// Alternate network transport (native protocols only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkTransport {
    /// DOSBox IPXNET relay.
    IpxDosbox {
        /// Relay host.
        relay: String,
    },
    /// Host AF_IPX socket.
    IpxNative,
}

/// Mod component selection (`PRODUCT/COMPONENT_ID`, shared with persistence).
pub use crate::persistence::mods::ModSelection;

/// Weapon-behavior overlay selection (`PRODUCT/DECLARED_ID`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeaponBehavior {
    /// Package product.
    pub product: String,
    /// Declared behavior ID.
    pub id: String,
}

/// Explicitly-set match rules (`explicitRules`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ExplicitRules {
    /// `--skill` was passed.
    pub skill: bool,
    /// `--mode` was passed.
    pub mode: bool,
    /// `--seats` was passed.
    pub capacity: bool,
}

/// Display overrides (`displayOverrides`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct DisplayOverrides {
    /// Width override.
    pub width: Option<u32>,
    /// Height override.
    pub height: Option<u32>,
    /// Gamma override.
    pub gamma: Option<f64>,
}

/// Parsed application options (`ApplicationOptions`).
#[derive(Debug, Clone, PartialEq)]
pub struct ApplicationOptions {
    /// Alternate network transport.
    pub network_transport: Option<NetworkTransport>,
    /// Q3 application product.
    pub q3_product: Option<Q3ApplicationProduct>,
    /// `+command` startup lines.
    pub startup_commands: Vec<String>,
    /// Explicitly-set rules.
    pub explicit_rules: ExplicitRules,
    /// Remote content selection.
    pub remote_content: Option<RemoteContentSelection>,
    /// NetQuake host protocol.
    pub q1_protocol: Option<ProtocolIdentity>,
    /// Quake II client/server protocol.
    pub q2_protocol: Option<ProtocolIdentity>,
    /// Shared server settings path (`--server-profile`).
    pub server_profile_path: Option<String>,
    /// Game data root.
    pub corpus_root: String,
    /// Writable user content root.
    pub user_content_root: Option<String>,
    /// Installed catalog product.
    pub product: String,
    /// Map content product override.
    pub map_product: Option<String>,
    /// Map resource path.
    pub map: String,
    /// Validated mounted QuakeC artifact.
    pub quake_c_program: Option<String>,
    /// Explicit Quake II game DLL.
    pub q2_game_library: Option<String>,
    /// Weapon-behavior overlay.
    pub weapon_behavior: Option<WeaponBehavior>,
    /// Enabled mod components.
    pub mods: Vec<ModSelection>,
    /// Movement family.
    pub movement: GameFamily,
    /// Exact movement product override.
    pub movement_product: Option<String>,
    /// Player character provider.
    pub character: GameFamily,
    /// Character model.
    pub character_model: String,
    /// Renderer.
    pub renderer: Renderer,
    /// Execute rendering on a worker.
    pub render_worker: Option<bool>,
    /// How the renderer was chosen.
    pub renderer_selection: RendererSelection,
    /// Display gamma.
    pub gamma: f64,
    /// Display overrides.
    pub display_overrides: DisplayOverrides,
    /// Dedicated server (no window or seats).
    pub dedicated: bool,
    /// Window width.
    pub width: u32,
    /// Window height.
    pub height: u32,
    /// Local seats.
    pub seats: u32,
    /// Gameplay difficulty.
    pub skill: u8,
    /// Quake III bot difficulty.
    pub bot_skill: Option<u8>,
    /// Game mode.
    pub mode: GameMode,
    /// Match rules.
    pub rules: Option<MatchRules>,
    /// Gameplay random seed.
    pub seed: u32,
    /// Close after N simulation steps.
    pub frame_limit: Option<u64>,
    /// Start a hidden window.
    pub hidden: bool,
    /// Network role.
    pub network: Network,
}

impl Default for ApplicationOptions {
    fn default() -> Self {
        Self {
            network_transport: None,
            q3_product: None,
            startup_commands: Vec::new(),
            explicit_rules: ExplicitRules::default(),
            remote_content: None,
            q1_protocol: None,
            q2_protocol: None,
            server_profile_path: None,
            corpus_root: default_corpus_root(),
            user_content_root: None,
            product: "q2-classic-baseq2".to_string(),
            map_product: None,
            map: "maps/base1.bsp".to_string(),
            quake_c_program: None,
            q2_game_library: None,
            weapon_behavior: None,
            mods: Vec::new(),
            movement: GameFamily::Q1,
            movement_product: None,
            character: GameFamily::Q3,
            character_model: "sarge".to_string(),
            renderer: Renderer::Gl,
            render_worker: None,
            renderer_selection: RendererSelection::Default,
            gamma: 1.0,
            display_overrides: DisplayOverrides::default(),
            dedicated: false,
            width: 960,
            height: 600,
            seats: 1,
            skill: 1,
            bot_skill: None,
            mode: GameMode::Singleplayer,
            rules: None,
            seed: 1,
            frame_limit: None,
            hidden: false,
            network: Network::Offline,
        }
    }
}

/// Parsed top-level command (`ApplicationCommand`).
#[derive(Debug, Clone, PartialEq)]
pub enum ApplicationCommand {
    /// Print help.
    Help,
    /// Print the version.
    Version,
    /// Weapon-behavior tool invocation.
    WeaponBehavior {
        /// Parsed tool command.
        command: WeaponBehaviorTool,
    },
    /// List installed content.
    ListContent {
        /// Game data root.
        corpus_root: String,
    },
    /// Run the application.
    Run {
        /// Application options.
        options: ApplicationOptions,
    },
    /// Open the startup menu.
    Menu {
        /// Application options.
        options: ApplicationOptions,
    },
}

/// Parsed weapon-behavior tool command (`WeaponBehaviorToolCommand`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WeaponBehaviorTool {
    /// Print tool help.
    Help,
    /// Inspect declarations.
    Inspect {
        /// Tool content roots.
        content: ToolContent,
    },
    /// Declare from a QVM profile.
    DeclareQvm {
        /// Tool content roots.
        content: ToolContent,
        /// Mounted profile path.
        profile: String,
    },
    /// Declare from a native profile.
    DeclareNative {
        /// Tool content roots.
        content: ToolContent,
        /// Mounted profile path.
        profile: String,
    },
    /// Author a declaration.
    Declare {
        /// Tool content roots.
        content: ToolContent,
        /// Declaration ID (`NAMESPACE:ID`).
        id: String,
        /// Display title.
        title: String,
        /// Projectile role.
        role: ProjectileRole,
        /// Fire callback.
        fire: String,
        /// Activation callback.
        activate: Option<String>,
    },
}

/// Tool content roots (`ToolContent`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolContent {
    /// Installed product.
    pub product: String,
    /// Installed content root.
    pub corpus_root: String,
    /// Writable user content root.
    pub user_content_root: String,
    /// Mounted program artifact.
    pub artifact: Option<String>,
}

/// Projectile role (`ProjectileRole`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProjectileRole {
    /// Rocket.
    Rocket,
    /// Grenade.
    Grenade,
    /// Nail.
    Nail,
    /// Bolt.
    Bolt,
    /// Plasma.
    Plasma,
    /// Energy.
    Energy,
    /// Grapple.
    Grapple,
}

/// Default game data root: executable-relative discovery (the executable's
/// own directory, then its parent), falling back to the executable's own
/// directory when no game content is found nearby.
#[must_use]
pub fn default_corpus_root() -> String {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    crate::directories::resolve_corpus_root(None, exe_dir.as_deref(), &crate::directories::FsProbe)
}

/// Default writable user content root
/// (`~/.local/share/quake-typescript/content`).
#[must_use]
pub fn default_user_content_root() -> String {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() => Path::new(&home)
            .join(".local/share/quake-typescript/content")
            .to_string_lossy()
            .into_owned(),
        _ => resolve_path(".local/share/quake-typescript/content"),
    }
}

/// Resolve a CLI path against the working directory (donor `resolve`).
#[must_use]
pub fn resolve_path(value: &str) -> String {
    let path = Path::new(value);
    if path.is_absolute() {
        return value.to_string();
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    normalize_path(&cwd.join(path))
}

fn normalize_path(path: &Path) -> String {
    let mut parts: Vec<String> = Vec::new();
    for component in path.components() {
        use std::path::Component;
        match component {
            Component::ParentDir => {
                parts.pop();
            }
            Component::CurDir => {}
            other => parts.push(other.as_os_str().to_string_lossy().into_owned()),
        }
    }
    if path.is_absolute() {
        format!("/{}", parts.join("/"))
    } else {
        parts.join("/")
    }
}

/// Effective Q2 protocol (`liveQ2Protocol`): the explicit selection, else
/// Kex 2023 for rerelease content, else classic 34.
#[must_use]
pub fn live_q2_protocol(options: &ApplicationOptions, rerelease: bool) -> ProtocolIdentity {
    options.q2_protocol.unwrap_or(if rerelease {
        ProtocolIdentity::Q2Kex
    } else {
        ProtocolIdentity::Q2Classic
    })
}

/// Normalize a map name to a `maps/*.bsp` resource path (`mapResourcePath`).
pub fn map_resource_path(name: &str) -> Result<String, AppError> {
    let normalized = name.replace('\\', "/");
    let valid = normalized.len() <= "maps/".len() + name.len()
        && valid_map_characters(&normalized)
        && !normalized.split('/').any(|segment| segment == "..");
    if !valid {
        return Err(AppError::BadMapName(name.to_string()));
    }
    let with_prefix = if normalized.starts_with("maps/") {
        normalized
    } else {
        format!("maps/{normalized}")
    };
    if with_prefix.ends_with(".bsp") {
        Ok(with_prefix)
    } else {
        Ok(format!("{with_prefix}.bsp"))
    }
}

fn valid_map_characters(normalized: &str) -> bool {
    let stripped = normalized.strip_prefix("maps/").unwrap_or(normalized);
    if stripped.is_empty() {
        return false;
    }
    let stripped = stripped.strip_suffix(".bsp").unwrap_or(stripped);
    if stripped.is_empty() {
        return false;
    }
    stripped
        .chars()
        .all(|char| char.is_ascii_alphanumeric() || matches!(char, '_' | '/' | '-'))
}

/// Read one `+command` startup argument (`readStartupCommand`).
///
/// Returns the command text and the index of the last consumed argument.
pub fn read_startup_command(argv: &[String], index: usize) -> Result<(String, usize), AppError> {
    let first = argv
        .get(index)
        .ok_or_else(|| AppError::BadStartupCommand("Expected +command".to_string()))?;
    if !first.starts_with('+') || first.len() == 1 {
        return Err(AppError::BadStartupCommand("Expected +command".to_string()));
    }
    let mut text = first[1..].to_string();
    let mut end = index;
    while let Some(next) = argv.get(end + 1) {
        if next.starts_with('+') || next.starts_with("--") {
            break;
        }
        text.push(' ');
        text.push_str(&startup_operand(next)?);
        end += 1;
    }
    source_command_text(&text)?;
    if text.chars().any(|char| matches!(char, '\r' | '\n' | '\0'))
        || command_separator_offset(&text, Dialect::Q3) < text.len()
    {
        return Err(AppError::BadStartupCommand(
            "Use a separate +command for each startup command.".to_string(),
        ));
    }
    let tokens = tokenize_command(&text, Dialect::Q3, TextMode::Console)?;
    if tokens.argv.is_empty() {
        return Err(AppError::BadStartupCommand("Expected +command".to_string()));
    }
    let verb = ascii_fold(tokens.argv[0].as_str());
    if verb == "connect" {
        return Err(AppError::BadStartupCommand(
            "Ordered +connect startup is unsupported; use --connect-q1, --connect-qw, --connect-q2 or --connect-q3."
                .to_string(),
        ));
    }
    if verb == "set" {
        let variable = ascii_fold(tokens.argv.get(1).map_or("", String::as_str));
        if matches!(
            variable.as_str(),
            "game" | "fs_game" | "basedir" | "cddir" | "fs_basepath" | "fs_homepath" | "fs_cdpath"
        ) {
            return Err(AppError::BadStartupCommand(
                "Filesystem +set selection is unsupported; use --game with an installed catalog product and --content-root or --user-content-root."
                    .to_string(),
            ));
        }
    }
    Ok((text, end))
}

fn startup_operand(value: &str) -> Result<String, AppError> {
    source_command_text(value)?;
    if value.chars().any(|char| matches!(char, '"' | '\r' | '\n' | '\0')) {
        return Err(AppError::BadStartupCommand(
            "Startup argument cannot contain quotes or line breaks; use a quoted +command batch or exec a cfg file."
                .to_string(),
        ));
    }
    Ok(format!("\"{value}\""))
}

/// Whether any startup line requests a world (`startupRequestsWorld`).
#[must_use]
pub fn startup_requests_world(lines: &[String]) -> bool {
    lines.iter().any(|line| {
        tokenize_command(line, Dialect::Q3, TextMode::Console)
            .ok()
            .and_then(|tokens| tokens.argv.first().cloned())
            .is_some_and(|verb| matches!(verb.to_lowercase().as_str(), "map" | "devmap" | "spmap" | "spdevmap"))
    })
}

/// Read a mod selection (`readModSelection`): `PRODUCT/COMPONENT_ID`.
pub fn read_mod_selection(value: &str) -> Result<ModSelection, AppError> {
    mods::read_mod_selection(value).map_err(|error| AppError::BadValue(error.to_string()))
}

/// Parse the weapon-behavior tool arguments (`parseWeaponBehaviorTool`).
pub fn parse_weapon_behavior_tool(argv: &[String]) -> Result<WeaponBehaviorTool, AppError> {
    if argv.iter().any(|arg| arg == "--help" || arg == "-h") {
        return Ok(WeaponBehaviorTool::Help);
    }
    let action = argv.first().map(String::as_str);
    let product = argv.get(1).map(String::as_str);
    let valid_action = matches!(action, Some("inspect" | "declare" | "declare-qvm" | "declare-native"));
    if !valid_action || product.is_none_or(|product| product.starts_with("--")) {
        return Err(AppError::BadValue(WEAPON_BEHAVIOR_HELP.to_string()));
    }
    let action = action.unwrap_or("inspect");
    let product = product.unwrap_or("").to_string();
    let mut flags: Vec<(String, String)> = Vec::new();
    let mut index = 2;
    while index < argv.len() {
        let key = argv[index].clone();
        let value = argv.get(index + 1).cloned();
        if !matches!(
            key.as_str(),
            "--content"
                | "--user-content"
                | "--artifact"
                | "--id"
                | "--title"
                | "--role"
                | "--fire"
                | "--activate"
                | "--profile"
        ) {
            return Err(AppError::BadValue(format!("Unknown behavior option: {key}")));
        }
        let Some(value) = value else {
            return Err(AppError::BadValue(format!("Missing or invalid value for {key}")));
        };
        if value.starts_with("--") || value.is_empty() || value.contains('\0') {
            return Err(AppError::BadValue(format!("Missing or invalid value for {key}")));
        }
        if flags.iter().any(|(flag, _)| flag == &key) {
            return Err(AppError::BadValue(format!("Repeated behavior option: {key}")));
        }
        flags.push((key, value));
        index += 2;
    }
    let flag = |key: &str| {
        flags
            .iter()
            .find(|(flag, _)| flag == key)
            .map(|(_, value)| value.as_str())
    };
    let artifact = flag("--artifact")
        .map(qa_content::paths::normalize_resource_path)
        .transpose()
        .map_err(|error| AppError::Content(error.to_string()))?;
    let content = ToolContent {
        product,
        corpus_root: resolve_path(flag("--content").unwrap_or(&default_corpus_root())),
        user_content_root: resolve_path(flag("--user-content").unwrap_or(&default_user_content_root())),
        artifact,
    };
    let has_any = |keys: &[&str]| keys.iter().any(|key| flag(key).is_some());
    if action == "declare-qvm" || action == "declare-native" {
        let Some(profile) = flag("--profile") else {
            return Err(AppError::BadValue(format!(
                "{action} requires --profile and takes identity, role and callbacks from that declaration"
            )));
        };
        if has_any(&["--id", "--title", "--role", "--fire", "--activate"]) {
            return Err(AppError::BadValue(format!(
                "{action} requires --profile and takes identity, role and callbacks from that declaration"
            )));
        }
        let profile = qa_content::paths::normalize_resource_path(profile)
            .map_err(|error| AppError::Content(error.to_string()))?;
        if action == "declare-qvm" {
            return Ok(WeaponBehaviorTool::DeclareQvm { content, profile });
        }
        return Ok(WeaponBehaviorTool::DeclareNative { content, profile });
    }
    if flag("--profile").is_some() {
        return Err(AppError::BadValue(
            "--profile requires declare-qvm or declare-native".to_string(),
        ));
    }
    if action == "inspect" {
        if has_any(&["--id", "--title", "--role", "--fire", "--activate"]) {
            return Err(AppError::BadValue(
                "Declaration options require the declare action".to_string(),
            ));
        }
        return Ok(WeaponBehaviorTool::Inspect { content });
    }
    let id = flag("--id");
    let fire = flag("--fire");
    let role = flag("--role");
    if id.is_none_or(|id| !valid_behavior_id(id)) || fire.is_none() || role.is_none() {
        return Err(AppError::BadValue(
            "Declare requires --id NAMESPACE:ID, --role ROLE and --fire CALLBACK".to_string(),
        ));
    }
    Ok(WeaponBehaviorTool::Declare {
        content,
        id: id.unwrap_or("").to_string(),
        title: flag("--title").unwrap_or_else(|| id.unwrap_or("")).to_string(),
        role: parse_projectile_role(role.unwrap_or(""))?,
        fire: fire.unwrap_or("").to_string(),
        activate: flag("--activate").map(str::to_string),
    })
}

fn valid_behavior_id(id: &str) -> bool {
    let Some(colon) = id.find(':') else {
        return false;
    };
    colon > 0
        && colon + 1 < id.len()
        && !id[..colon].contains(char::is_whitespace)
        && !id[colon + 1..].chars().any(char::is_whitespace)
}

fn parse_projectile_role(value: &str) -> Result<ProjectileRole, AppError> {
    match value {
        "rocket" => Ok(ProjectileRole::Rocket),
        "grenade" => Ok(ProjectileRole::Grenade),
        "nail" => Ok(ProjectileRole::Nail),
        "bolt" => Ok(ProjectileRole::Bolt),
        "plasma" => Ok(ProjectileRole::Plasma),
        "energy" => Ok(ProjectileRole::Energy),
        "grapple" => Ok(ProjectileRole::Grapple),
        _ => Err(AppError::BadValue(format!("Invalid projectile role: {value}"))),
    }
}

fn integer_option(value: &str, label: &str, minimum: u64, maximum: u64) -> Result<u64, AppError> {
    if value.is_empty() || !value.chars().all(|char| char.is_ascii_digit()) {
        return Err(AppError::BadValue(format!("{label} must be an integer")));
    }
    let number: u64 = value
        .parse()
        .map_err(|_| AppError::BadValue(format!("{label} must be between {minimum} and {maximum}")))?;
    if number < minimum || number > maximum {
        return Err(AppError::BadValue(format!(
            "{label} must be between {minimum} and {maximum}"
        )));
    }
    Ok(number)
}

fn parse_family(value: &str) -> Result<GameFamily, AppError> {
    match value {
        "q1" => Ok(GameFamily::Q1),
        "q2" => Ok(GameFamily::Q2),
        "q3" => Ok(GameFamily::Q3),
        _ => Err(AppError::BadValue(format!("Unknown game family: {value}"))),
    }
}

fn parse_q1_protocol(value: &str) -> Result<ProtocolIdentity, AppError> {
    let version = integer_option(value, "--q1-protocol", 15, 999)?;
    match version {
        15 => Ok(ProtocolIdentity::Q1Netquake),
        666 => Ok(ProtocolIdentity::Q1Fitzquake),
        999 => Ok(ProtocolIdentity::Q1Rmq {
            flags: q1::PRFL_INT32COORD | q1::PRFL_SHORTANGLE,
        }),
        _ => Err(AppError::BadValue(format!("Unsupported NetQuake protocol {version}"))),
    }
}

fn parse_q2_protocol(value: &str) -> Result<ProtocolIdentity, AppError> {
    match value {
        "34" => return Ok(ProtocolIdentity::Q2Classic),
        "35" | "35:1904" => {
            return Ok(ProtocolIdentity::Q2R1q2 { revision: 1904 });
        }
        "35:1905" => {
            return Ok(ProtocolIdentity::Q2R1q2 { revision: 1905 });
        }
        "4038" => return Ok(ProtocolIdentity::Q2PrivateClassic),
        "2023" => return Ok(ProtocolIdentity::Q2Kex),
        "1038" => return Ok(ProtocolIdentity::Q2Rerelease),
        _ => {}
    }
    let revision = if value == "36" {
        1026
    } else if let Some(suffix) = value.strip_prefix("36:") {
        suffix.parse::<u32>().unwrap_or(0)
    } else {
        0
    };
    if matches!(revision, 1015 | 1017..=1026) {
        return Ok(ProtocolIdentity::Q2Q2pro { revision });
    }
    Err(AppError::BadValue(
        "--q2-protocol requires 34, 35:1904/1905, 36:1015/1017..1026, 4038, 1038 or 2023".to_string(),
    ))
}

/// Flags that do not count as an explicit launch selection.
const NON_LAUNCH_FLAGS: &[&str] = &[
    "--content-root",
    "--user-content-root",
    "--renderer",
    "--render-worker",
    "--gamma",
    "--width",
    "--height",
    "--hidden",
    "--list-content",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ListenKind {
    Native,
    Q2,
    Unified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RemoteKind {
    Q1,
    Qw,
    Q2,
    Q3,
    Unified,
}

/// Parse the application command line (`parseApplicationCommand`).
pub fn parse_application_command(argv: &[String]) -> Result<ApplicationCommand, AppError> {
    if argv.first().is_some_and(|first| first == "weapon-behavior") {
        return Ok(ApplicationCommand::WeaponBehavior {
            command: parse_weapon_behavior_tool(&argv[1..])?,
        });
    }
    let mut options = ApplicationOptions::default();
    let mut startup_commands: Vec<String> = Vec::new();
    let mut list = false;
    let mut menu = false;
    let mut explicit_launch = false;
    let mut listen_kind = ListenKind::Q2;
    let mut remote_kind = RemoteKind::Q2;
    let mut bind = "0.0.0.0".to_string();
    let mut listen: Option<u16> = None;
    let mut remote: Option<String> = None;

    let mut index = 0;
    while index < argv.len() {
        let flag = argv[index].clone();
        if flag.starts_with('+') {
            let (text, end) = read_startup_command(argv, index)?;
            startup_commands.push(text);
            index = end + 1;
            continue;
        }
        if flag == "--menu" {
            menu = true;
            index += 1;
            continue;
        }
        if !NON_LAUNCH_FLAGS.contains(&flag.as_str()) {
            explicit_launch = true;
        }
        if flag == "--help" || flag == "-h" {
            return Ok(ApplicationCommand::Help);
        }
        if flag == "--version" {
            return Ok(ApplicationCommand::Version);
        }
        if flag == "--dedicated" {
            options.dedicated = true;
            index += 1;
            continue;
        }
        if flag == "--hidden" {
            options.hidden = true;
            index += 1;
            continue;
        }
        if flag == "--list-content" {
            list = true;
            index += 1;
            continue;
        }
        if flag == "--ipx-native" {
            if options.network_transport.is_some() {
                return Err(AppError::ConflictingOptions("Choose one IPX transport".to_string()));
            }
            options.network_transport = Some(NetworkTransport::IpxNative);
            index += 1;
            continue;
        }
        index += 1;
        let value = argv
            .get(index)
            .ok_or_else(|| AppError::MissingValue(flag.clone()))?
            .clone();
        match flag.as_str() {
            "--ipx-dosbox" => {
                if options.network_transport.is_some() {
                    return Err(AppError::ConflictingOptions("Choose one IPX transport".to_string()));
                }
                if value.trim().is_empty() || value.starts_with("--") {
                    return Err(AppError::BadValue(
                        "--ipx-dosbox requires a relay hostname or IPv4 address".to_string(),
                    ));
                }
                options.network_transport = Some(NetworkTransport::IpxDosbox { relay: value });
            }
            "--preset" => {
                options.movement_product = None;
                match value.as_str() {
                    "q2-q1-q3" => {
                        options.product = "q2-classic-baseq2".to_string();
                        options.map = "maps/base1.bsp".to_string();
                        options.movement = GameFamily::Q1;
                        options.character = GameFamily::Q3;
                        options.character_model = "sarge".to_string();
                    }
                    "q1-q2" => {
                        options.product = "q1-rerelease-id1".to_string();
                        options.map = "maps/e1m1.bsp".to_string();
                        options.movement = GameFamily::Q2;
                        options.character = GameFamily::Q2;
                        options.character_model = "male".to_string();
                    }
                    _ => return Err(AppError::BadValue(format!("Unknown launch preset: {value}"))),
                }
            }
            "--user-content-root" => options.user_content_root = Some(resolve_path(&value)),
            "--content-root" => options.corpus_root = resolve_path(&value),
            "--map-game" => options.map_product = Some(value),
            "--game" => options.product = value,
            "--mod" => options.mods.push(read_mod_selection(&value)?),
            "--map" => options.map = map_resource_path(&value)?,
            "--weapon-behavior" => {
                let slash = value.find('/').unwrap_or(0);
                let valid = slash > 0
                    && slash + 1 < value.len()
                    && !value.chars().any(char::is_whitespace)
                    && value[slash + 1..].contains(':');
                if !valid {
                    return Err(AppError::BadValue(
                        "Weapon behavior must be PRODUCT/DECLARED_ID".to_string(),
                    ));
                }
                options.weapon_behavior = Some(WeaponBehavior {
                    product: value[..slash].to_string(),
                    id: value[slash + 1..].to_string(),
                });
            }
            "--q2-game" => {
                let path = qa_content::paths::normalize_resource_path(&value)
                    .map_err(|error| AppError::Content(error.to_string()))?;
                if !path.ends_with(".dll") {
                    return Err(AppError::BadValue(
                        "--q2-game requires a mounted .dll artifact".to_string(),
                    ));
                }
                options.q2_game_library = Some(path);
            }
            "--progs" => {
                let path = qa_content::paths::normalize_resource_path(&value)
                    .map_err(|error| AppError::Content(error.to_string()))?;
                if !path.ends_with(".dat") {
                    return Err(AppError::BadValue(
                        "--progs requires a mounted .dat artifact".to_string(),
                    ));
                }
                options.quake_c_program = Some(path);
            }
            "--movement" => {
                options.movement_product = None;
                match value.as_str() {
                    "q1" | "q2" | "q3" => options.movement = parse_family(&value)?,
                    _ => {
                        if !mods::valid_product(&value) {
                            return Err(AppError::BadValue(format!("Invalid movement product: {value}")));
                        }
                        options.movement_product = Some(if value == "qw" {
                            "q1-quakeworld".to_string()
                        } else {
                            value
                        });
                    }
                }
            }
            "--character" => {
                let selected = parse_family(&value)?;
                options.character = selected;
                options.character_model = match selected {
                    GameFamily::Q3 => "sarge".to_string(),
                    GameFamily::Q2 => "male".to_string(),
                    GameFamily::Q1 => "player".to_string(),
                };
            }
            "--model" => {
                if value.is_empty()
                    || !value
                        .chars()
                        .all(|char| char.is_ascii_alphanumeric() || matches!(char, '_' | '-'))
                {
                    return Err(AppError::BadValue(format!("Invalid character model: {value}")));
                }
                options.character_model = value;
            }
            "--render-worker" => {
                if value != "0" && value != "1" {
                    return Err(AppError::BadValue("Render worker must be 0 or 1".to_string()));
                }
                options.render_worker = Some(value == "1");
            }
            "--renderer" => match value.as_str() {
                "cpu" => {
                    options.renderer = Renderer::Cpu;
                    options.renderer_selection = RendererSelection::Explicit;
                }
                "gl" => {
                    options.renderer = Renderer::Gl;
                    options.renderer_selection = RendererSelection::Explicit;
                }
                _ => return Err(AppError::BadValue(format!("Unknown renderer: {value}"))),
            },
            "--width" => {
                let width = integer_option(&value, &flag, 64, 16384)?;
                options.width = width as u32;
                options.display_overrides.width = Some(width as u32);
            }
            "--gamma" => {
                let gamma: f64 = value.parse().unwrap_or(f64::NAN);
                if !gamma.is_finite() || gamma < 0.5 || gamma > 3.0 {
                    return Err(AppError::BadValue(
                        "Display gamma must be between 0.5 and 3".to_string(),
                    ));
                }
                options.gamma = gamma;
                options.display_overrides.gamma = Some(gamma);
            }
            "--height" => {
                let height = integer_option(&value, &flag, 64, 16384)?;
                options.height = height as u32;
                options.display_overrides.height = Some(height as u32);
            }
            "--seats" => {
                options.seats = integer_option(&value, &flag, 1, 4)? as u32;
                options.explicit_rules.capacity = true;
            }
            "--seed" => options.seed = integer_option(&value, &flag, 0, 0xffff_ffff)? as u32,
            "--frames" => options.frame_limit = Some(integer_option(&value, &flag, 1, u64::MAX)?),
            "--listen" | "--listen-q2" | "--listen-unified" => {
                let kind = if flag == "--listen-unified" {
                    ListenKind::Unified
                } else if flag == "--listen" {
                    ListenKind::Native
                } else {
                    ListenKind::Q2
                };
                if listen.is_some() && listen_kind != kind {
                    return Err(AppError::ConflictingOptions("Choose one server listener".to_string()));
                }
                listen_kind = kind;
                listen = Some(integer_option(&value, &flag, 0, 65535)? as u16);
            }
            "--q2-protocol" => options.q2_protocol = Some(parse_q2_protocol(&value)?),
            "--q1-protocol" => options.q1_protocol = Some(parse_q1_protocol(&value)?),
            "--connect-qw" | "--connect-q1" | "--connect-q2" | "--connect-q3" | "--connect-unified" => {
                if remote.is_some() {
                    return Err(AppError::ConflictingOptions("Choose one remote connection".to_string()));
                }
                remote_kind = if flag == "--connect-unified" {
                    RemoteKind::Unified
                } else if flag == "--connect-qw" {
                    RemoteKind::Qw
                } else if flag == "--connect-q1" {
                    RemoteKind::Q1
                } else if flag == "--connect-q3" {
                    RemoteKind::Q3
                } else {
                    RemoteKind::Q2
                };
                remote = Some(value);
            }
            "--bind" => bind = value,
            "--skill" => {
                let skill = integer_option(&value, &flag, 0, 3)? as u8;
                options.skill = skill;
                options.explicit_rules.skill = true;
            }
            "--bot-skill" => {
                options.bot_skill = Some(integer_option(&value, &flag, 1, 5)? as u8);
            }
            "--server-profile" => options.server_profile_path = Some(resolve_path(&value)),
            "--rules" => {
                options.rules = Some(match value.as_str() {
                    "standard" => MatchRules::Standard,
                    "ctf" => MatchRules::Ctf,
                    "lmctf" => MatchRules::Lmctf,
                    "tag" => MatchRules::Tag,
                    "deathball" => MatchRules::Deathball,
                    "horde" => MatchRules::Horde,
                    _ => return Err(AppError::BadValue(format!("Unknown match rules: {value}"))),
                });
            }
            "--mode" => {
                options.mode = match value.as_str() {
                    "singleplayer" => GameMode::Singleplayer,
                    "coop" => GameMode::Coop,
                    "deathmatch" => GameMode::Deathmatch,
                    _ => return Err(AppError::BadValue(format!("Unknown game mode: {value}"))),
                };
                options.explicit_rules.mode = true;
            }
            _ => return Err(AppError::UnknownOption(flag)),
        }
        index += 1;
    }

    if matches!(options.rules, Some(MatchRules::Ctf | MatchRules::Lmctf))
        || options.rules.is_none() && matches!(options.product.as_str(), "q2-classic-ctf" | "q2-classic-lmctf")
    {
        options.mode = GameMode::Deathmatch;
    }
    if !startup_commands.is_empty() {
        explicit_launch = explicit_launch || startup_requests_world(&startup_commands);
        options.startup_commands = startup_commands;
    }
    if list {
        return Ok(ApplicationCommand::ListContent {
            corpus_root: options.corpus_root,
        });
    }
    if listen.is_some() && remote.is_some() {
        return Err(AppError::ConflictingOptions(
            "Choose a server listener or a remote connection".to_string(),
        ));
    }
    if let Some(port) = listen {
        options.network = match listen_kind {
            ListenKind::Native => Network::NativeServer {
                host: bind.clone(),
                port,
            },
            ListenKind::Q2 => Network::Q2Server {
                host: bind.clone(),
                port,
            },
            ListenKind::Unified => Network::UnifiedServer {
                host: bind.clone(),
                port,
            },
        };
    } else if bind != "0.0.0.0" {
        return Err(AppError::ConflictingOptions(
            "--bind requires --listen, --listen-q2 or --listen-unified".to_string(),
        ));
    }
    if let Some(remote) = remote {
        if options.bot_skill.is_some() {
            return Err(AppError::ConflictingOptions(
                "--bot-skill is not a native Quake II client setting".to_string(),
            ));
        }
        options.network = match remote_kind {
            RemoteKind::Q1 => Network::Q1Client { remote },
            RemoteKind::Qw => Network::QwClient { remote },
            RemoteKind::Q2 => Network::Q2Client { remote },
            RemoteKind::Q3 => Network::Q3Client { remote },
            RemoteKind::Unified => Network::UnifiedClient { remote },
        };
    }
    if options.network_transport.is_some()
        && matches!(
            options.network,
            Network::UnifiedServer { .. } | Network::UnifiedClient { .. }
        )
    {
        return Err(AppError::ConflictingOptions(
            "Unified networking uses UDP; IPX applies to native game protocols".to_string(),
        ));
    }
    if options.network_transport.is_some() && matches!(options.network, Network::Offline) {
        return Err(AppError::ConflictingOptions(
            "IPX selection requires --listen, --listen-q2 or a native client connection".to_string(),
        ));
    }
    if matches!(options.network_transport, Some(NetworkTransport::IpxNative)) && bind != "0.0.0.0" {
        return Err(AppError::ConflictingOptions(
            "--bind selects an IP interface and cannot bind native AF_IPX".to_string(),
        ));
    }
    if options.network_transport.is_some() && matches!(options.network, Network::QwClient { .. }) {
        return Err(AppError::ConflictingOptions(
            "QuakeWorld uses UDP; IPX is not a QuakeWorld transport".to_string(),
        ));
    }
    if options.q2_protocol.is_some()
        && !matches!(
            options.network,
            Network::Q2Client { .. } | Network::Q2Server { .. } | Network::NativeServer { .. }
        )
    {
        return Err(AppError::ConflictingOptions(
            "--q2-protocol requires --connect-q2, --listen-q2 or --listen".to_string(),
        ));
    }
    if options.q1_protocol.is_some() && !matches!(options.network, Network::NativeServer { .. }) {
        return Err(AppError::ConflictingOptions(
            "--q1-protocol requires --listen for a Quake I host".to_string(),
        ));
    }
    if options.q1_protocol.is_some() && options.product == "q1-quakeworld" {
        return Err(AppError::ConflictingOptions(
            "--q1-protocol selects NetQuake; QuakeWorld uses native protocol 28".to_string(),
        ));
    }
    if !matches!(options.network, Network::Offline) && options.mode == GameMode::Singleplayer {
        let deathmatch_host = matches!(
            options.network,
            Network::NativeServer { .. } | Network::UnifiedServer { .. }
        ) && (options.product.starts_with("q3-") || options.product == "q1-quakeworld");
        options.mode = if deathmatch_host {
            GameMode::Deathmatch
        } else {
            GameMode::Coop
        };
    }
    if options.seats > 1 && options.mode == GameMode::Singleplayer {
        options.mode = GameMode::Coop;
    }
    if menu && (options.dedicated || !matches!(options.network, Network::Offline)) {
        return Err(AppError::ConflictingOptions(
            "--menu requires a local, non-dedicated application".to_string(),
        ));
    }
    if menu || !explicit_launch {
        Ok(ApplicationCommand::Menu { options })
    } else {
        Ok(ApplicationCommand::Run { options })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_string()).collect()
    }

    fn run_options(words: &[&str]) -> ApplicationOptions {
        match parse_application_command(&argv(words)).unwrap() {
            ApplicationCommand::Run { options } | ApplicationCommand::Menu { options } => options,
            other => panic!("expected run or menu, got {other:?}"),
        }
    }

    #[test]
    fn help_version_and_empty_menu() {
        assert_eq!(
            parse_application_command(&argv(&["--help"])).unwrap(),
            ApplicationCommand::Help
        );
        assert_eq!(
            parse_application_command(&argv(&["-h"])).unwrap(),
            ApplicationCommand::Help
        );
        assert_eq!(
            parse_application_command(&argv(&["--version"])).unwrap(),
            ApplicationCommand::Version
        );
        assert!(matches!(
            parse_application_command(&argv(&[])).unwrap(),
            ApplicationCommand::Menu { .. }
        ));
        assert!(matches!(
            parse_application_command(&argv(&["--menu"])).unwrap(),
            ApplicationCommand::Menu { .. }
        ));
    }

    #[test]
    fn defaults_match_donor() {
        let options = run_options(&["--menu"]);
        assert_eq!(options.product, "q2-classic-baseq2");
        assert_eq!(options.map, "maps/base1.bsp");
        assert_eq!(options.movement, GameFamily::Q1);
        assert_eq!(options.character, GameFamily::Q3);
        assert_eq!(options.character_model, "sarge");
        assert_eq!(options.renderer, Renderer::Gl);
        assert_eq!(options.renderer_selection, RendererSelection::Default);
        assert_eq!(options.gamma, 1.0);
        assert!(!options.dedicated);
        assert_eq!((options.width, options.height), (960, 600));
        assert_eq!(options.seats, 1);
        assert_eq!(options.skill, 1);
        assert_eq!(options.bot_skill, None);
        assert_eq!(options.mode, GameMode::Singleplayer);
        assert_eq!(options.seed, 1);
        assert_eq!(options.frame_limit, None);
        assert!(!options.hidden);
        assert_eq!(options.network, Network::Offline);
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf));
        match exe_dir {
            Some(dir) => assert_eq!(Path::new(&options.corpus_root), dir.as_path()),
            None => assert!(!options.corpus_root.is_empty()),
        }
    }

    #[test]
    fn launch_flags_select_run() {
        assert!(matches!(
            parse_application_command(&argv(&["--dedicated", "--map", "e1m1"])).unwrap(),
            ApplicationCommand::Run { .. }
        ));
        let options = run_options(&["--dedicated", "--map", "e1m1"]);
        assert!(options.dedicated);
        assert_eq!(options.map, "maps/e1m1.bsp");
    }

    #[test]
    fn map_names_normalize_and_reject() {
        assert_eq!(map_resource_path("e1m1").unwrap(), "maps/e1m1.bsp");
        assert_eq!(map_resource_path("maps/base1.bsp").unwrap(), "maps/base1.bsp");
        assert_eq!(map_resource_path("dm\\q3dm1").unwrap(), "maps/dm/q3dm1.bsp");
        assert!(map_resource_path("../escape").is_err());
        assert!(map_resource_path("maps/../x").is_err());
        assert!(map_resource_path("bad name!").is_err());
        assert!(map_resource_path("maps/").is_err());
    }

    #[test]
    fn presets_movement_and_character() {
        let options = run_options(&["--preset", "q1-q2"]);
        assert_eq!(options.product, "q1-rerelease-id1");
        assert_eq!(options.map, "maps/e1m1.bsp");
        assert_eq!(options.movement, GameFamily::Q2);
        assert_eq!(options.character_model, "male");
        let options = run_options(&["--preset", "q2-q1-q3"]);
        assert_eq!(options.movement, GameFamily::Q1);
        assert_eq!(options.character, GameFamily::Q3);
        assert!(parse_application_command(&argv(&["--preset", "nope"])).is_err());

        let options = run_options(&["--movement", "qw"]);
        assert_eq!(options.movement_product.as_deref(), Some("q1-quakeworld"));
        let options = run_options(&["--movement", "q3"]);
        assert_eq!(options.movement, GameFamily::Q3);
        assert_eq!(options.movement_product, None);
        assert!(parse_application_command(&argv(&["--movement", "bogus!"])).is_err());

        let options = run_options(&["--character", "q1"]);
        assert_eq!(options.character_model, "player");
        let options = run_options(&["--character", "q2", "--model", "female"]);
        assert_eq!(options.character_model, "female");
        assert!(parse_application_command(&argv(&["--model", "bad name"])).is_err());
    }

    #[test]
    fn numeric_ranges_are_enforced() {
        assert!(parse_application_command(&argv(&["--seats", "5"])).is_err());
        assert!(parse_application_command(&argv(&["--seats", "0"])).is_err());
        assert!(parse_application_command(&argv(&["--seats", "x"])).is_err());
        assert!(parse_application_command(&argv(&["--gamma", "0.4"])).is_err());
        assert!(parse_application_command(&argv(&["--gamma", "nan"])).is_err());
        assert!(parse_application_command(&argv(&["--width", "63"])).is_err());
        assert!(parse_application_command(&argv(&["--skill", "4"])).is_err());
        assert!(parse_application_command(&argv(&["--bot-skill", "6"])).is_err());
        assert!(parse_application_command(&argv(&["--frames", "0"])).is_err());
        assert!(parse_application_command(&argv(&["--render-worker", "2"])).is_err());
        assert!(parse_application_command(&argv(&["--renderer", "vulkan"])).is_err());
        assert!(parse_application_command(&argv(&["--seed"])).is_err());
        assert!(parse_application_command(&argv(&["--bogus"])).is_err());
        let options = run_options(&["--seed", "42", "--frames", "120", "--gamma", "2"]);
        assert_eq!(options.seed, 42);
        assert_eq!(options.frame_limit, Some(120));
        assert_eq!(options.gamma, 2.0);
    }

    #[test]
    fn network_selection_and_conflicts() {
        let options = run_options(&["--listen-q2", "27910"]);
        assert_eq!(
            options.network,
            Network::Q2Server {
                host: "0.0.0.0".to_string(),
                port: 27910
            }
        );
        assert_eq!(options.mode, GameMode::Coop);
        let options = run_options(&["--listen", "26000", "--bind", "127.0.0.1"]);
        assert_eq!(
            options.network,
            Network::NativeServer {
                host: "127.0.0.1".to_string(),
                port: 26000
            }
        );
        let options = run_options(&["--connect-q3", "example:27960"]);
        assert!(matches!(options.network, Network::Q3Client { .. }));
        assert!(parse_application_command(&argv(&["--listen", "1", "--listen-q2", "2"])).is_err());
        assert!(parse_application_command(&argv(&["--connect-q1", "a", "--connect-q2", "b"])).is_err());
        assert!(parse_application_command(&argv(&["--listen", "1", "--connect-q2", "b"])).is_err());
        assert!(parse_application_command(&argv(&["--bind", "127.0.0.1"])).is_err());
        assert!(parse_application_command(&argv(&["--connect-q2", "x", "--bot-skill", "3"])).is_err());
        assert!(parse_application_command(&argv(&["--menu", "--dedicated"])).is_err());
        assert!(parse_application_command(&argv(&["--menu", "--connect-q2", "x"])).is_err());
    }

    #[test]
    fn protocol_selection() {
        let options = run_options(&["--listen", "26000", "--q1-protocol", "999"]);
        assert_eq!(
            options.q1_protocol,
            Some(ProtocolIdentity::Q1Rmq {
                flags: q1::PRFL_INT32COORD | q1::PRFL_SHORTANGLE
            })
        );
        assert!(parse_application_command(&argv(&["--listen", "26000", "--q1-protocol", "16"])).is_err());
        assert!(parse_application_command(&argv(&["--q1-protocol", "15"])).is_err());
        assert!(parse_application_command(&argv(&["--listen-q2", "1", "--q1-protocol", "15"])).is_err());
        let options = run_options(&["--listen-q2", "27910", "--q2-protocol", "36:1020"]);
        assert_eq!(options.q2_protocol, Some(ProtocolIdentity::Q2Q2pro { revision: 1020 }));
        let options = run_options(&["--listen-q2", "27910", "--q2-protocol", "35"]);
        assert_eq!(options.q2_protocol, Some(ProtocolIdentity::Q2R1q2 { revision: 1904 }));
        assert!(parse_application_command(&argv(&["--q2-protocol", "34"])).is_err());
        assert!(parse_application_command(&argv(&["--listen-q2", "1", "--q2-protocol", "37"])).is_err());
        assert_eq!(
            live_q2_protocol(&run_options(&["--menu"]), false),
            ProtocolIdentity::Q2Classic
        );
        assert_eq!(
            live_q2_protocol(&run_options(&["--menu"]), true),
            ProtocolIdentity::Q2Kex
        );
    }

    #[test]
    fn ipx_rules() {
        let options = run_options(&["--listen-q2", "27910", "--ipx-dosbox", "relay"]);
        assert_eq!(
            options.network_transport,
            Some(NetworkTransport::IpxDosbox {
                relay: "relay".to_string()
            })
        );
        assert!(parse_application_command(&argv(&["--ipx-native"])).is_err());
        assert!(parse_application_command(&argv(&["--ipx-native", "--ipx-dosbox", "r"])).is_err());
        assert!(parse_application_command(&argv(&["--listen-unified", "1", "--ipx-native"])).is_err());
        assert!(parse_application_command(&argv(&["--connect-qw", "x", "--ipx-native"])).is_err());
        assert!(
            parse_application_command(&argv(&["--listen-q2", "1", "--bind", "127.0.0.1", "--ipx-native"])).is_err()
        );
    }

    #[test]
    fn mode_rules_and_seats() {
        let options = run_options(&["--rules", "ctf"]);
        assert_eq!(options.mode, GameMode::Deathmatch);
        let options = run_options(&["--seats", "2"]);
        assert_eq!(options.mode, GameMode::Coop);
        assert!(options.explicit_rules.capacity);
        let options = run_options(&["--game", "q2-classic-ctf"]);
        assert_eq!(options.mode, GameMode::Deathmatch);
        let options = run_options(&["--game", "q3-baseq3", "--listen", "27960"]);
        assert_eq!(options.mode, GameMode::Deathmatch);
        assert!(parse_application_command(&argv(&["--rules", "nope"])).is_err());
        assert!(parse_application_command(&argv(&["--mode", "nope"])).is_err());
    }

    #[test]
    fn mods_weapon_behavior_and_artifacts() {
        let options = run_options(&["--mod", "pkg/comp", "--mod", "pkg/other"]);
        assert_eq!(options.mods.len(), 2);
        assert_eq!(options.mods[0].product, "pkg");
        assert!(parse_application_command(&argv(&["--mod", "no-slash"])).is_err());
        assert!(parse_application_command(&argv(&["--mod", "!bad/id"])).is_err());
        let options = run_options(&["--weapon-behavior", "pkg/ns:traj"]);
        assert_eq!(options.weapon_behavior.as_ref().unwrap().product, "pkg");
        assert!(parse_application_command(&argv(&["--weapon-behavior", "pkg/nocolon"])).is_err());
        let options = run_options(&["--progs", "progs.dat", "--q2-game", "gamex86.dll"]);
        assert_eq!(options.quake_c_program.as_deref(), Some("progs.dat"));
        assert_eq!(options.q2_game_library.as_deref(), Some("gamex86.dll"));
        assert!(parse_application_command(&argv(&["--progs", "progs.dll"])).is_err());
        assert!(parse_application_command(&argv(&["--q2-game", "game.dat"])).is_err());
    }

    #[test]
    fn mod_selection_matches_persistence() {
        for value in ["pkg/comp", "pkg/sub/comp", "q1-quakeworld/baseqw", "a.b_c+d/e:f/g-h"] {
            let options_selection = read_mod_selection(value).expect("options accepts donor key");
            let saved = crate::persistence::mods::read_mod_selection(value).expect("persistence accepts donor key");
            assert_eq!(options_selection, saved);
        }
        for value in ["no-slash", "!bad/id", "pkg/", "/comp", "pkg/has space", ""] {
            assert!(read_mod_selection(value).is_err(), "{value} must be rejected");
            assert!(crate::persistence::mods::read_mod_selection(value).is_err());
        }
    }

    #[test]
    fn startup_commands() {
        let (text, end) = read_startup_command(&argv(&["+map", "e1m1", "--dedicated"]), 0).unwrap();
        assert_eq!(text, "map \"e1m1\"");
        assert_eq!(end, 1);
        let options = run_options(&["+map", "e1m1"]);
        assert_eq!(options.startup_commands, vec!["map \"e1m1\"".to_string()]);
        assert!(matches!(
            parse_application_command(&argv(&["+map", "e1m1"])).unwrap(),
            ApplicationCommand::Run { .. }
        ));
        assert!(matches!(
            parse_application_command(&argv(&["+bind", "x"])).unwrap(),
            ApplicationCommand::Menu { .. }
        ));
        assert!(parse_application_command(&argv(&["+connect", "x"])).is_err());
        assert!(parse_application_command(&argv(&["+set", "game", "x"])).is_err());
        assert!(startup_requests_world(&["spmap \"e1m1\"".to_string()]));
        assert!(!startup_requests_world(&["bind \"x\"".to_string()]));
    }

    #[test]
    fn list_content_and_weapon_behavior_branches() {
        match parse_application_command(&argv(&["--list-content", "--content-root", "/tmp/x"])).unwrap() {
            ApplicationCommand::ListContent { corpus_root } => assert_eq!(corpus_root, "/tmp/x"),
            other => panic!("expected list-content, got {other:?}"),
        }
        assert!(matches!(
            parse_weapon_behavior_tool(&argv(&["--help"])).unwrap(),
            WeaponBehaviorTool::Help
        ));
        match parse_weapon_behavior_tool(&argv(&["inspect", "pkg"])).unwrap() {
            WeaponBehaviorTool::Inspect { content } => assert_eq!(content.product, "pkg"),
            other => panic!("expected inspect, got {other:?}"),
        }
        assert!(parse_weapon_behavior_tool(&argv(&["bogus", "pkg"])).is_err());
        assert!(parse_weapon_behavior_tool(&argv(&["inspect", "pkg", "--id", "a:b"])).is_err());
        match parse_weapon_behavior_tool(&argv(&[
            "declare", "pkg", "--id", "ns:id", "--role", "rocket", "--fire", "f",
        ]))
        .unwrap()
        {
            WeaponBehaviorTool::Declare { role, title, .. } => {
                assert_eq!(role, ProjectileRole::Rocket);
                assert_eq!(title, "ns:id");
            }
            other => panic!("expected declare, got {other:?}"),
        }
        assert!(parse_weapon_behavior_tool(&argv(&["declare", "pkg", "--role", "nope", "--fire", "f"])).is_err());
    }
}
