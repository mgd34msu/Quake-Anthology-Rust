//! Source cvar registry and rule resolution for startup.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/startup-source.ts`
//! (`createStartupSource`, `resolveStartupRules`). The source/match selection
//! pair arrives as [`StartupSourceSelection`]; the Q3 product and server
//! profile (donor `options.q3Product`/`options.serverProfile`, absent from
//! [`ApplicationOptions`](crate::options::ApplicationOptions) on this lane)
//! arrive as explicit caller inputs. Cvar registration reuses `qa_core`,
//! Q2 server cvars and profile application reuse `crate::settings::server`,
//! Q3 game definitions reuse `qa_content`, and source administration reuses
//! [`super::server_administration`]. Frame-time registration delegates to
//! the canonical [`frame_time`](super::frame_time) module. The QuakeWorld
//! engine ([`quakeworld_cvars`](super::simulation::quakeworld_cvars)), Q3
//! server ([`q3::server_state`](super::simulation::q3::server_state), via
//! [`q3_common_cvars`](super::q3_common_cvars)), and Q3 product-policy
//! ([`Q3ApplicationProduct`](super::content::Q3ApplicationProduct))
//! registrations are ported inline with donor provenance; the shared
//! `CvarRegistry` takes no context/print callback, so those donor
//! parameters are dropped.

use qa_content::q3::base::settings::q3_game_cvar_definitions;
use qa_content::q3::base::shared::definitions::Product;
use qa_core::cmd::Dialect;
use qa_core::cvar::flags;
use qa_core::cvar::CvarError;
use qa_core::cvar::CvarRegistry;
use thiserror::Error;

use super::server_administration::register_source_administration_cvars;
use super::server_administration::ServerAdministrationError;
use crate::options::ApplicationOptions;
use crate::options::GameMode;
use crate::options::Network;
use crate::settings::server::apply_server_profile;
use crate::settings::server::register_q2_server_cvars;
use crate::settings::server::ServerBinding;
use crate::settings::server::ServerProfile;
use crate::settings::SettingsError;

/// Startup source failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum StartupSourceError {
    /// Cvar registration or assignment failed.
    #[error("Startup source cvar failed: {0}")]
    Cvar(String),
    /// Q2 server cvar registration failed.
    #[error("Startup source server cvars failed: {0}")]
    Server(String),
    /// Source administration cvars failed.
    #[error("Startup source administration failed: {0}")]
    Administration(String),
    /// Server profile application failed.
    #[error("Startup source profile failed: {0}")]
    Profile(String),
    /// Latched difficulty is not a number.
    #[error("Invalid source startup difficulty")]
    InvalidDifficulty,
    /// Resolved client capacity is outside 1..=64.
    #[error("Source startup client capacity must be between 1 and 64")]
    InvalidCapacity,
}

impl From<CvarError> for StartupSourceError {
    fn from(error: CvarError) -> Self {
        Self::Cvar(error.to_string())
    }
}

impl From<SettingsError> for StartupSourceError {
    fn from(error: SettingsError) -> Self {
        Self::Server(error.to_string())
    }
}

impl From<ServerAdministrationError> for StartupSourceError {
    fn from(error: ServerAdministrationError) -> Self {
        Self::Administration(error.to_string())
    }
}

/// Source/match selection pair (donor `Pick<ApplicationSourceSelection, "source" | "match">`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupSourceSelection {
    /// Source content identity (mission-pack detection reads this).
    pub source_content: String,
    /// Match provider (Q2 server cvar selection reads this).
    pub match_provider: String,
}

/// Q3 product policy (donor `Q3ProductPolicy` in
/// `../../core/q3-product-policy.ts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3ProductPolicy {
    /// Retail release.
    Retail,
    /// Prerelease demo (`team_arena_ui_demo` selects the demo UI).
    PrereleaseDemo {
        /// Whether the Team Arena UI is the demo UI.
        team_arena_ui_demo: bool,
    },
    /// Prerelease Team Arena demo.
    PrereleaseTaDemo,
}

/// Q3 application product (donor `options.q3Product`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartupQ3Product {
    /// Release policy.
    pub policy: Q3ProductPolicy,
    /// Whether mounts are demo-restricted (`fs_restrict`).
    pub demo_restricted: bool,
}

impl StartupQ3Product {
    /// Donor `q3PrereleaseDemo`.
    #[must_use]
    pub fn prerelease_demo(self) -> bool {
        matches!(self.policy, Q3ProductPolicy::PrereleaseDemo { .. })
    }

    /// Donor `q3TeamArenaDemo`.
    #[must_use]
    pub fn team_arena_demo(self) -> bool {
        match self.policy {
            Q3ProductPolicy::PrereleaseTaDemo => true,
            Q3ProductPolicy::PrereleaseDemo { team_arena_ui_demo } => team_arena_ui_demo,
            Q3ProductPolicy::Retail => false,
        }
    }
}

/// Resolved startup rules (donor `resolveStartupRules` return).
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedStartupRules {
    /// Options with resolved skill and mode.
    pub options: ApplicationOptions,
    /// Resolved client capacity.
    pub max_clients: u32,
}

/// Quake III network protocol version (donor `Q3_PROTOCOL` in
/// `../../network/q3/adapters.ts`).
const Q3_PROTOCOL_VERSION: u32 = 68;

/// Register when absent (donor `cvars.find(name) === undefined` guard).
fn register_when_absent(cvars: &mut CvarRegistry, name: &str, value: &str, bits: u32) -> Result<(), CvarError> {
    if cvars.get(name).is_none() {
        cvars.register(name, value, bits)?;
    }
    Ok(())
}

/// Frame-time cvars, delegated to the canonical [`frame_time`](super::frame_time)
/// registration so all three former copies stay consistent.
fn register_frame_time_cvars(cvars: &mut CvarRegistry) -> Result<(), CvarError> {
    super::frame_time::register_frame_time_cvars(cvars).map_err(|error| match error {
        super::frame_time::FrameTimeError::Cvar(source) => source,
        super::frame_time::FrameTimeError::Invalid(message) => CvarError::Domain(message),
    })
}

/// QuakeWorld engine cvars, ported inline from
/// `./simulation/quakeworld-cvars.ts` (`registerQuakeWorldEngineCvars`).
fn register_quake_world_engine_cvars(cvars: &mut CvarRegistry) -> Result<(), CvarError> {
    for (name, value) in [
        ("sv_phs", "1"),
        ("sv_stopspeed", "100"),
        ("sv_spectatormaxspeed", "500"),
        ("sv_accelerate", "10"),
        ("sv_airaccelerate", "0.7"),
        ("sv_wateraccelerate", "10"),
        ("sv_friction", "4"),
        ("sv_waterfriction", "4"),
        ("password", ""),
        ("spectator_password", ""),
        ("sv_highchars", "1"),
    ] {
        register_when_absent(cvars, name, value, 0)?;
    }
    cvars.register("maxspectators", "8", flags::SERVER_INFO)?;
    Ok(())
}

/// Q3 server cvars, ported inline from `../q3-common-cvars.ts`
/// (`q3ServerCvarDefinitions`/`registerQ3ServerCvars`) plus the collision map
/// definitions from `../../world/collision/q3/settings.ts`.
fn register_q3_server_cvars(cvars: &mut CvarRegistry, max_clients: u32, map_name: &str) -> Result<(), CvarError> {
    let server_info = flags::SERVER_INFO;
    let read_only = flags::READ_ONLY;
    let system_info = flags::SYSTEM_INFO;
    let definitions = [
        ("protocol", Q3_PROTOCOL_VERSION.to_string(), server_info | read_only),
        ("sv_pure", "1".to_string(), system_info),
        ("sv_allowDownload", "0".to_string(), server_info),
        ("sv_maxRate", "0".to_string(), server_info),
        ("sv_fps", "20".to_string(), flags::NONE),
        ("sv_serverid", "0".to_string(), system_info | read_only),
        ("sv_paks", String::new(), system_info | read_only),
        ("sv_pakNames", String::new(), system_info | read_only),
        ("sv_referencedPaks", String::new(), system_info | read_only),
        ("sv_referencedPakNames", String::new(), system_info | read_only),
        ("sv_maxclients", max_clients.to_string(), server_info | flags::LATCH),
        ("mapname", map_name.to_string(), server_info | read_only),
        ("sv_mapname", String::new(), server_info | read_only),
        ("sv_privateClients", "0".to_string(), server_info),
        ("sv_privatePassword", String::new(), flags::TEMPORARY),
        ("sv_reconnectlimit", "3".to_string(), flags::NONE),
        ("sv_minPing", "0".to_string(), flags::ARCHIVE | server_info),
        ("sv_maxPing", "0".to_string(), flags::ARCHIVE | server_info),
        ("sv_floodProtect", "1".to_string(), flags::ARCHIVE | server_info),
        ("sv_strictAuth", "1".to_string(), flags::ARCHIVE),
        ("bot_enable", "1".to_string(), flags::NONE),
        ("cm_noAreas", "0".to_string(), flags::CHEAT),
        ("cm_noCurves", "0".to_string(), flags::CHEAT),
        ("cm_playerCurveClip", "1".to_string(), flags::ARCHIVE | flags::CHEAT),
    ];
    for (name, value, bits) in definitions {
        cvars.register(name, &value, bits)?;
    }
    Ok(())
}

/// Q3 product-policy cvars, ported inline from
/// `../../core/q3-product-policy.ts` (`registerQ3ProductPolicy`).
fn register_q3_product_policy(cvars: &mut CvarRegistry) -> Result<(), CvarError> {
    cvars.register("com_prereleaseDemo", "0", flags::INIT)?;
    cvars.register("com_prereleaseTeamArenaDemo", "0", flags::INIT)?;
    Ok(())
}

/// Build the source cvar registry for a startup selection (donor
/// `createStartupSource`). `q3_product` carries the donor's
/// `options.q3Product`; the donor's context/print parameters have no Rust
/// `CvarRegistry` counterpart and are dropped.
pub fn create_startup_source(
    options: &ApplicationOptions,
    selection: &StartupSourceSelection,
    dialect: Dialect,
    max_clients: u32,
    q3_product: Option<StartupQ3Product>,
) -> Result<CvarRegistry, StartupSourceError> {
    let mut cvars = CvarRegistry::new(dialect);
    if dialect == Dialect::Q2Classic || dialect == Dialect::Q2Rerelease {
        register_q2_server_cvars(&mut cvars, &selection.match_provider)?;
    } else if dialect == Dialect::Q3 {
        if let Some(product) = q3_product {
            cvars.set(
                "com_prereleaseDemo",
                if product.prerelease_demo() { "1" } else { "0" },
                true,
            )?;
            cvars.set(
                "com_prereleaseTeamArenaDemo",
                if product.team_arena_demo() { "1" } else { "0" },
                true,
            )?;
            cvars.set("fs_restrict", if product.demo_restricted { "1" } else { "0" }, true)?;
        }
        register_q3_product_policy(&mut cvars)?;
        cvars.register("fs_restrict", "0", flags::INIT)?;
        let product = if selection.source_content.contains("missionpack") {
            Product::Missionpack
        } else {
            Product::Baseq3
        };
        for definition in q3_game_cvar_definitions(product) {
            cvars.register(&definition.name, &definition.value, definition.flags)?;
        }
    } else {
        for (name, value) in [
            ("skill", "1"),
            ("deathmatch", "0"),
            ("coop", "0"),
            ("teamplay", "0"),
            ("sv_cheats", "0"),
            ("sv_aim", if dialect == Dialect::Q1Quakeworld { "2" } else { "0.93" }),
            ("developer", "0"),
            ("pausable", "1"),
            ("sv_gravity", "800"),
            ("sv_maxspeed", "320"),
            ("samelevel", "0"),
            ("timelimit", "0"),
            ("fraglimit", "0"),
            ("gamecfg", "0"),
            ("registered", "1"),
            ("footsteps", "1"),
        ] {
            cvars.register(name, value, 0)?;
        }
    }
    let capacity_name = if dialect == Dialect::Q3 {
        "sv_maxclients"
    } else {
        "maxclients"
    };
    for (name, value) in [
        ("skill", options.skill.to_string()),
        (
            "deathmatch",
            if options.mode == GameMode::Deathmatch { "1" } else { "0" }.to_string(),
        ),
        (
            "coop",
            if options.mode == GameMode::Coop { "1" } else { "0" }.to_string(),
        ),
        (
            "g_gametype",
            if options.mode == GameMode::Singleplayer {
                "2"
            } else {
                "0"
            }
            .to_string(),
        ),
        (capacity_name, max_clients.to_string()),
    ] {
        if cvars.get(name).is_none() {
            cvars.register(name, &value, 0)?;
        } else {
            cvars.set(name, &value, true)?;
        }
    }
    if dialect == Dialect::Q1Quakeworld {
        register_quake_world_engine_cvars(&mut cvars)?;
    }
    if dialect == Dialect::Q3 {
        register_q3_server_cvars(&mut cvars, max_clients, &options.map)?;
    }
    register_frame_time_cvars(&mut cvars)?;
    register_source_administration_cvars(&mut cvars)?;
    if !options.dedicated && options.network == Network::Offline && dialect != Dialect::Q3 {
        cvars.register("sv_autosave", "1", flags::ARCHIVE)?;
        cvars.register("sv_autosave_interval", "0", flags::ARCHIVE)?;
    }
    cvars.register("qts_weaponBehavior", "", flags::ARCHIVE)?;
    Ok(cvars)
}

/// Resolve latched rules back into options and capacity (donor
/// `resolveStartupRules`). `server_profile` carries the donor's
/// `options.serverProfile`; `bindings` carries the donor's
/// recipe-definition/owner pairs in the ported [`ServerBinding`] shape.
pub fn resolve_startup_rules(
    options: &ApplicationOptions,
    cvars: &mut CvarRegistry,
    default_capacity: u32,
    bindings: &[ServerBinding],
    server_profile: Option<&ServerProfile>,
    apply_explicit: bool,
) -> Result<ResolvedStartupRules, StartupSourceError> {
    cvars.apply_latched(None)?;
    if apply_explicit && options.explicit_rules.skill {
        cvars.set("skill", &options.skill.to_string(), true)?;
    }
    if apply_explicit && options.explicit_rules.mode {
        cvars.set(
            "deathmatch",
            if options.mode == GameMode::Deathmatch { "1" } else { "0" },
            true,
        )?;
        cvars.set("coop", if options.mode == GameMode::Coop { "1" } else { "0" }, true)?;
        if cvars.dialect() == Dialect::Q3 {
            if options.mode == GameMode::Singleplayer {
                cvars.set("g_gametype", "2", true)?;
            } else if options.mode != GameMode::Deathmatch
                || cvars
                    .get("g_gametype")
                    .is_some_and(|snapshot| snapshot.integer_value == 2)
            {
                cvars.set("g_gametype", "0", true)?;
            }
        }
    }
    if apply_explicit {
        if let Some(profile) = server_profile {
            apply_server_profile(profile, bindings, cvars)
                .map_err(|error| StartupSourceError::Profile(error.to_string()))?;
        }
    }
    cvars.apply_latched(None)?;
    let floored = cvars.variable_value("skill").floor();
    if floored.is_nan() {
        return Err(StartupSourceError::InvalidDifficulty);
    }
    let skill = floored.clamp(0.0, 3.0) as u8;
    let mode = if apply_explicit && options.explicit_rules.mode {
        options.mode
    } else if cvars.dialect() == Dialect::Q3 {
        if cvars.variable_value("g_gametype") == 2.0 {
            GameMode::Singleplayer
        } else {
            GameMode::Deathmatch
        }
    } else if cvars.variable_value("deathmatch") != 0.0 {
        GameMode::Deathmatch
    } else if cvars.variable_value("coop") != 0.0 {
        GameMode::Coop
    } else {
        GameMode::Singleplayer
    };
    let capacity_name = if cvars.dialect() == Dialect::Q3 {
        "sv_maxclients"
    } else {
        "maxclients"
    };
    let requested = if apply_explicit && options.explicit_rules.capacity {
        default_capacity as f32
    } else {
        cvars.variable_value(capacity_name)
    };
    let native_capacity =
        if (cvars.dialect() == Dialect::Q2Classic || cvars.dialect() == Dialect::Q2Rerelease) && requested <= 1.0 {
            match mode {
                GameMode::Deathmatch => 8.0,
                GameMode::Coop => 4.0,
                GameMode::Singleplayer => requested,
            }
        } else {
            requested
        };
    let floor = if options.dedicated { 1.0 } else { options.seats as f32 };
    let capacity = floor.max(native_capacity.trunc());
    if !(1.0..=64.0).contains(&capacity) {
        return Err(StartupSourceError::InvalidCapacity);
    }
    let max_clients = capacity as u32;
    cvars.set(capacity_name, &max_clients.to_string(), true)?;
    Ok(ResolvedStartupRules {
        options: ApplicationOptions {
            skill,
            mode,
            ..options.clone()
        },
        max_clients,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> ApplicationOptions {
        ApplicationOptions {
            skill: 2,
            mode: GameMode::Deathmatch,
            seats: 2,
            map: "maps/base1.bsp".to_string(),
            ..ApplicationOptions::default()
        }
    }

    fn selection() -> StartupSourceSelection {
        StartupSourceSelection {
            source_content: "q2-classic-baseq2".to_string(),
            match_provider: "q2:official".to_string(),
        }
    }

    #[test]
    fn builds_q1_source_with_rules() {
        let cvars = create_startup_source(&options(), &selection(), Dialect::Q1Netquake, 4, None).unwrap();
        assert_eq!(cvars.variable_string("skill"), "2");
        assert_eq!(cvars.variable_string("deathmatch"), "1");
        assert_eq!(cvars.variable_string("coop"), "0");
        assert_eq!(cvars.variable_string("maxclients"), "4");
        assert_eq!(cvars.variable_string("sv_gravity"), "800");
        assert!(cvars.get("timescale").is_none());
        assert_eq!(cvars.variable_string("host_framerate"), "0");
        assert_eq!(cvars.variable_string("sv_autosave"), "1");
        assert!(cvars.get("qts_weaponBehavior").is_some());
    }

    #[test]
    fn builds_quakeworld_source_with_engine_cvars() {
        let cvars = create_startup_source(&options(), &selection(), Dialect::Q1Quakeworld, 8, None).unwrap();
        assert_eq!(cvars.variable_string("sv_aim"), "2");
        assert_eq!(cvars.variable_string("sv_phs"), "1");
        assert_eq!(cvars.variable_string("maxspectators"), "8");
        assert!(cvars.get("timescale").is_none());
        assert_eq!(cvars.variable_string("cl_maxfps"), "0");
    }

    #[test]
    fn builds_q2_source_with_match_provider() {
        let cvars = create_startup_source(&options(), &selection(), Dialect::Q2Classic, 8, None).unwrap();
        assert_eq!(cvars.variable_string("maxclients"), "8");
        assert_eq!(cvars.variable_string("fixedtime"), "0");
        assert!(cvars.get("timescale").is_some());
    }

    #[test]
    fn builds_q3_source_with_product_policy() {
        let product = StartupQ3Product {
            policy: Q3ProductPolicy::PrereleaseDemo {
                team_arena_ui_demo: true,
            },
            demo_restricted: true,
        };
        let selection = StartupSourceSelection {
            source_content: "q3-missionpack".to_string(),
            match_provider: "q3:official".to_string(),
        };
        let cvars = create_startup_source(&options(), &selection, Dialect::Q3, 12, Some(product)).unwrap();
        assert_eq!(cvars.variable_string("com_prereleaseDemo"), "1");
        assert_eq!(cvars.variable_string("com_prereleaseTeamArenaDemo"), "1");
        assert!(cvars.get("g_gametype").is_some());
        assert_eq!(cvars.variable_string("sv_maxclients"), "12");
        assert_eq!(cvars.variable_string("protocol"), "68");
        assert_eq!(cvars.variable_string("cm_playerCurveClip"), "1");
        assert!(cvars.get("sv_autosave").is_none());
    }

    #[test]
    fn resolves_explicit_rules_and_capacity() {
        let mut options = options();
        options.explicit_rules.skill = true;
        options.explicit_rules.mode = true;
        options.explicit_rules.capacity = true;
        let mut cvars = create_startup_source(&options, &selection(), Dialect::Q1Netquake, 4, None).unwrap();
        let resolved = resolve_startup_rules(&options, &mut cvars, 6, &[], None, true).unwrap();
        assert_eq!(resolved.options.skill, 2);
        assert_eq!(resolved.options.mode, GameMode::Deathmatch);
        assert_eq!(resolved.max_clients, 6);
        assert_eq!(cvars.variable_string("maxclients"), "6");
    }

    #[test]
    fn resolves_latched_mode_and_q2_capacity_floors() {
        let options = options();
        let mut cvars = create_startup_source(&options, &selection(), Dialect::Q2Classic, 1, None).unwrap();
        cvars.set("deathmatch", "1", true).unwrap();
        let resolved = resolve_startup_rules(&options, &mut cvars, 1, &[], None, false).unwrap();
        assert_eq!(resolved.options.mode, GameMode::Deathmatch);
        assert_eq!(resolved.max_clients, 8);
    }

    #[test]
    fn rejects_invalid_capacity() {
        let options = options();
        let mut cvars = create_startup_source(&options, &selection(), Dialect::Q1Netquake, 4, None).unwrap();
        cvars.set("maxclients", "99", true).unwrap();
        assert_eq!(
            resolve_startup_rules(&options, &mut cvars, 4, &[], None, false),
            Err(StartupSourceError::InvalidCapacity)
        );
    }
}
