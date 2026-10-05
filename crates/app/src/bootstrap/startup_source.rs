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
use qa_core::cvar::q2_flags;
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

/// Frame-time cvars, delegated to the canonical [`frame_time`](super::frame_time)
/// registration so all three former copies stay consistent.
fn register_frame_time_cvars(cvars: &mut CvarRegistry) -> Result<(), CvarError> {
    super::frame_time::register_frame_time_cvars(cvars).map_err(|error| match error {
        super::frame_time::FrameTimeError::Cvar(source) => source,
        super::frame_time::FrameTimeError::Invalid(message) => CvarError::Domain(message),
    })
}

/// QuakeWorld engine cvars, delegated to the canonical
/// [`quakeworld_cvars`](super::simulation::quakeworld_cvars) registration.
fn register_quake_world_engine_cvars(cvars: &mut CvarRegistry) -> Result<(), CvarError> {
    super::simulation::quakeworld_cvars::register_quake_world_engine_cvars(cvars)
}

/// Q3 server cvars, delegated to the canonical
/// [`q3_common_cvars`](super::q3_common_cvars) engine table.
fn register_q3_server_cvars(cvars: &mut CvarRegistry, max_clients: u32, map_name: &str) -> Result<(), CvarError> {
    super::q3_common_cvars::register_q3_server_cvars(cvars, max_clients, map_name)
}

/// Q3 product-policy cvars, delegated to the canonical content
/// registration; the resolved policy is returned to the caller by
/// [`qa_content::q3::product_restriction::register_q3_product_policy`].
fn register_q3_product_policy(cvars: &mut CvarRegistry) -> Result<(), CvarError> {
    qa_content::q3::product_restriction::register_q3_product_policy(cvars)
        .map(|_| ())
        .map_err(|error| CvarError::Domain(error.to_string()))
}

/// Register the source cvars for a startup selection into a live
/// registry (donor `createStartupSource`). `q3_product` carries the
/// donor's `options.q3Product`; the donor's context/print parameters have
/// no Rust `CvarRegistry` counterpart and are dropped. Engine tables run
/// before game tables so re-registration merges flags exactly like the
/// engine-then-VM order; capacity/mode rows carry the sources' flags.
pub fn register_startup_source_cvars(
    cvars: &mut CvarRegistry,
    options: &ApplicationOptions,
    selection: &StartupSourceSelection,
    max_clients: u32,
    q3_product: Option<StartupQ3Product>,
) -> Result<(), StartupSourceError> {
    let dialect = cvars.dialect();
    if dialect == Dialect::Q2Classic || dialect == Dialect::Q2Rerelease {
        register_q2_server_cvars(cvars, &selection.match_provider)?;
    } else if dialect == Dialect::Q3 {
        register_q3_server_cvars(cvars, max_clients, &options.map)?;
        register_q3_product_policy(cvars)?;
        cvars.register("fs_restrict", "0", flags::INIT)?;
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
    let q2_latched_info = q2_flags::SERVER_INFO | q2_flags::LATCH;
    let latched_info = if dialect.is_q2() {
        q2_latched_info
    } else if dialect == Dialect::Q3 {
        flags::SERVER_INFO | flags::LATCH
    } else if dialect == Dialect::Q1Quakeworld {
        flags::SERVER_INFO
    } else {
        0
    };
    let mut rules: Vec<(&str, String, u32)> = Vec::new();
    if dialect != Dialect::Q3 {
        rules.push(("skill", options.skill.to_string(), 0));
        rules.push((
            "deathmatch",
            if options.mode == GameMode::Deathmatch { "1" } else { "0" }.to_string(),
            if dialect.is_q2() { q2_latched_info } else { 0 },
        ));
        rules.push((
            "coop",
            if options.mode == GameMode::Coop { "1" } else { "0" }.to_string(),
            if dialect.is_q2() { q2_latched_info } else { 0 },
        ));
    } else {
        rules.push((
            "g_gametype",
            if options.mode == GameMode::Singleplayer {
                "2"
            } else {
                "0"
            }
            .to_string(),
            flags::SERVER_INFO | flags::LATCH,
        ));
    }
    rules.push((
        if dialect == Dialect::Q3 {
            "sv_maxclients"
        } else {
            "maxclients"
        },
        max_clients.to_string(),
        latched_info,
    ));
    for (name, value, bits) in &rules {
        if cvars.get(name).is_none() {
            cvars.register(name, value, *bits)?;
        } else {
            cvars.set(name, value, true)?;
        }
    }
    if dialect == Dialect::Q1Quakeworld {
        register_quake_world_engine_cvars(cvars)?;
    }
    register_frame_time_cvars(cvars)?;
    register_source_administration_cvars(cvars)?;
    if !options.dedicated && options.network == Network::Offline && dialect != Dialect::Q3 {
        cvars.register("sv_autosave", "1", flags::ARCHIVE)?;
    }
    cvars.register("qts_weaponBehavior", "", flags::ARCHIVE)?;
    Ok(())
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

    fn source(
        options: &ApplicationOptions,
        selection: &StartupSourceSelection,
        dialect: Dialect,
        max_clients: u32,
        product: Option<StartupQ3Product>,
    ) -> CvarRegistry {
        let mut cvars = CvarRegistry::new(dialect);
        register_startup_source_cvars(&mut cvars, options, selection, max_clients, product).unwrap();
        cvars
    }

    #[test]
    fn builds_q1_source_with_rules() {
        let cvars = source(&options(), &selection(), Dialect::Q1Netquake, 4, None);
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
        let cvars = source(&options(), &selection(), Dialect::Q1Quakeworld, 8, None);
        assert_eq!(cvars.variable_string("sv_aim"), "2");
        assert_eq!(cvars.variable_string("sv_phs"), "1");
        assert_eq!(cvars.variable_string("maxspectators"), "8");
        assert!(cvars.get("timescale").is_none());
        assert_eq!(cvars.variable_string("cl_maxfps"), "0");
    }

    #[test]
    fn builds_q2_source_with_match_provider() {
        let cvars = source(&options(), &selection(), Dialect::Q2Classic, 8, None);
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
        let cvars = source(&options(), &selection, Dialect::Q3, 12, Some(product));
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
        let mut cvars = source(&options, &selection(), Dialect::Q1Netquake, 4, None);
        let resolved = resolve_startup_rules(&options, &mut cvars, 6, &[], None, true).unwrap();
        assert_eq!(resolved.options.skill, 2);
        assert_eq!(resolved.options.mode, GameMode::Deathmatch);
        assert_eq!(resolved.max_clients, 6);
        assert_eq!(cvars.variable_string("maxclients"), "6");
    }

    #[test]
    fn resolves_latched_mode_and_q2_capacity_floors() {
        let options = options();
        let mut cvars = source(&options, &selection(), Dialect::Q2Classic, 1, None);
        cvars.set("deathmatch", "1", true).unwrap();
        let resolved = resolve_startup_rules(&options, &mut cvars, 1, &[], None, false).unwrap();
        assert_eq!(resolved.options.mode, GameMode::Deathmatch);
        assert_eq!(resolved.max_clients, 8);
    }

    #[test]
    fn rejects_invalid_capacity() {
        let options = options();
        let mut cvars = source(&options, &selection(), Dialect::Q1Netquake, 4, None);
        cvars.set("maxclients", "99", true).unwrap();
        assert_eq!(
            resolve_startup_rules(&options, &mut cvars, 4, &[], None, false),
            Err(StartupSourceError::InvalidCapacity)
        );
    }
}
