//! Startup: resolve CLI options into a runtime configuration, load a stub
//! map, and construct the headless server.
//!
//! Donor provenance: `src/app/bootstrap/startup.ts` (application assembly),
//! `src/app/bootstrap/frame-time.ts` (`sourceFrameMilliseconds` Q1 clamp
//! window), and the per-family server cadences in
//! `src/contracts/time.ts`. Real BSP loading and game-module binding stay
//! out; the stub map spawns a world entity plus player starts so the host
//! loop and the headless E2E test exercise real server ticks.

use qa_core::identity::ProviderId;
use qa_core::math::{vec3, Bounds};
use qa_core::time::{ClockProfile, SourceTime};
use qa_guest::core::contracts::{ContentDigest, ModuleIdentity};
use qa_guest::server::GuestServerLogic;
use qa_world::client::ClientFamily;
use qa_world::server::{plan_for_profile, Server, TickPlan};
use qa_world::session::Simulation;
use qa_world::spawn::{SpawnFields, SpawnRegistry, SpawnRequest};

use crate::error::AppError;
use crate::options::{ApplicationOptions, GameFamily};

/// Actor registry capacity for a hosted session.
pub const SESSION_CAPACITY: usize = 1024;
/// Host step for clamped (Q1-family) plans: fixed 60Hz server frames.
pub const CLAMPED_STEP_SECONDS: f32 = 1.0 / 60.0;
/// Q3 server frame in milliseconds (`sv_fps` 20).
pub const Q3_FRAME_MILLISECONDS: f64 = 50.0;
/// QuakeWorld maximum command length in milliseconds.
pub const QW_COMMAND_MILLISECONDS: f64 = 50.0;

/// Resolved runtime configuration for one application run.
#[derive(Debug, Clone)]
pub struct StartupConfig {
    /// Session name for the identity authority.
    pub session_name: String,
    /// Source clock profile.
    pub profile: ClockProfile,
    /// Server tick plan derived from the profile.
    pub plan: TickPlan,
    /// Fixed host step fed to the server each frame.
    pub step: SourceTime,
    /// Actor registry capacity.
    pub capacity: usize,
    /// Local seats (0 when dedicated).
    pub seats: u32,
    /// Dedicated server (no window or seats).
    pub dedicated: bool,
    /// Open a native window and run frames on it.
    pub windowed: bool,
    /// Close after N host frames.
    pub frame_limit: Option<u64>,
    /// Gameplay random seed.
    pub seed: u32,
    /// Map resource path.
    pub map: String,
    /// Installed catalog product.
    pub product: String,
    /// Game data root for catalog discovery.
    pub corpus_root: String,
    /// Stock skill level for Q1 spawnflags inhibition.
    pub skill: u8,
    /// Game mode for Q1 spawnflags inhibition.
    pub mode: crate::options::GameMode,
    /// Window width.
    pub width: u32,
    /// Window height.
    pub height: u32,
    /// Client command family for local seats.
    pub client_family: ClientFamily,
    /// Primary scheduler provider.
    pub primary: ProviderId,
    /// Game provider for spawned entities.
    pub game_provider: ProviderId,
    /// Default bounds for spawned bodies.
    pub default_bounds: Bounds,
    /// Spatial bounds for the server trigger sweep.
    pub spatial_bounds: Bounds,
    /// `+command` lines from the command line, without the `+`.
    pub startup_commands: Vec<String>,
}

impl StartupConfig {
    /// Resolve CLI options into a runtime configuration.
    pub fn from_options(options: &ApplicationOptions) -> Result<Self, AppError> {
        let profile = clock_profile_for(options);
        let plan = plan_for_profile(&profile)?;
        let step = match plan {
            TickPlan::Fixed { step } => step,
            TickPlan::Clamped { .. } => SourceTime::Seconds(CLAMPED_STEP_SECONDS),
        };
        let client_family = client_family_for(options);
        Ok(Self {
            session_name: format!("quake-anthology:{}:{}", options.product, options.map),
            profile,
            plan,
            step,
            capacity: SESSION_CAPACITY,
            seats: if options.dedicated { 0 } else { options.seats },
            dedicated: options.dedicated,
            windowed: options.windowed,
            frame_limit: options.frame_limit,
            seed: options.seed,
            map: options.map.clone(),
            product: options.product.clone(),
            corpus_root: options.corpus_root.clone(),
            skill: options.skill,
            mode: options.mode,
            width: options.width,
            height: options.height,
            client_family,
            primary: ProviderId::new("session", "primary"),
            game_provider: ProviderId::new("game", options.product.as_str()),
            default_bounds: Bounds {
                min: vec3(-16.0, -16.0, -24.0),
                max: vec3(16.0, 16.0, 32.0),
            },
            spatial_bounds: Bounds {
                min: vec3(-4096.0, -4096.0, -4096.0),
                max: vec3(4096.0, 4096.0, 4096.0),
            },
            startup_commands: options.startup_commands.clone(),
        })
    }
}

fn clock_profile_for(options: &ApplicationOptions) -> ClockProfile {
    if options.movement_product.as_deref() == Some("q1-quakeworld") {
        return ClockProfile::Q1Quakeworld {
            maximum_command_milliseconds: QW_COMMAND_MILLISECONDS,
        };
    }
    match options.movement {
        GameFamily::Q1 => ClockProfile::Q1Netquake {
            minimum_frame_seconds: 0.001,
            maximum_frame_seconds: 0.1,
            fixed_frame_seconds: None,
        },
        GameFamily::Q2 => ClockProfile::Q2Classic,
        GameFamily::Q3 => ClockProfile::Q3 {
            server_frame_milliseconds: Q3_FRAME_MILLISECONDS,
            fixed_movement_milliseconds: None,
        },
    }
}

fn client_family_for(options: &ApplicationOptions) -> ClientFamily {
    if options.movement_product.as_deref() == Some("q1-quakeworld") {
        return ClientFamily::Q1Quakeworld;
    }
    match options.movement {
        GameFamily::Q1 => ClientFamily::Q1Netquake,
        GameFamily::Q2 => ClientFamily::Q2Classic,
        GameFamily::Q3 => ClientFamily::Q3,
    }
}

/// Stub map: a world entity plus player starts, standing in for BSP entity
/// strings until format loading is wired into startup.
#[derive(Debug, Clone)]
pub struct StubMap {
    /// Map resource path.
    pub name: String,
    /// Entities to spawn in order.
    pub entities: Vec<SpawnFields>,
}

/// Load the stub map for a resource path.
#[must_use]
pub fn load_stub_map(name: &str) -> StubMap {
    let mut entities = vec![SpawnFields {
        classname: "worldspawn".to_string(),
        origin: vec3(0.0, 0.0, 0.0),
        angles: vec3(0.0, 0.0, 0.0),
        ..SpawnFields::default()
    }];
    for (index, origin) in [
        vec3(0.0, 0.0, 32.0),
        vec3(128.0, 0.0, 32.0),
        vec3(0.0, 128.0, 32.0),
        vec3(128.0, 128.0, 32.0),
    ]
    .into_iter()
    .enumerate()
    {
        entities.push(SpawnFields {
            classname: if index < 1 {
                "info_player_start"
            } else {
                "info_player_deathmatch"
            }
            .to_string(),
            origin,
            angles: vec3(0.0, 0.0, 0.0),
            ..SpawnFields::default()
        });
    }
    StubMap {
        name: name.to_string(),
        entities,
    }
}

/// Register the stub-map spawn functions on a spawn registry.
pub fn register_stub_spawns(registry: &mut SpawnRegistry) {
    for classname in [
        "worldspawn",
        "info_player_start",
        "info_player_deathmatch",
        "info_player_coop",
    ] {
        let definition = format!("stub:{classname}");
        registry.register(
            classname,
            Box::new(move |fields| {
                Ok(SpawnRequest {
                    definition: definition.clone(),
                    origin: Some(fields.origin),
                    combat: None,
                    grants: Vec::new(),
                })
            }),
        );
    }
}

/// Build the headless server for a configuration, with stub spawns
/// registered. Game logic runs in the guest VM; no game module is bound
/// here, so logic hooks are no-ops until the host loads one.
pub fn open_server(config: &StartupConfig) -> Result<Server<GuestServerLogic>, AppError> {
    let initial = match config.step {
        SourceTime::Seconds(_) => SourceTime::Seconds(0.0),
        SourceTime::Milliseconds(_) => SourceTime::Milliseconds(0),
    };
    let simulation = Simulation::new(
        &config.session_name,
        config.primary.clone(),
        config.profile,
        initial,
        config.capacity,
    )?;
    let logic = GuestServerLogic::new(ModuleIdentity::new(
        config.game_provider.clone(),
        "",
        ContentDigest::new("none", ""),
        "0",
    ))?;
    let mut server = Server::new(
        simulation,
        logic,
        config.plan,
        config.game_provider.clone(),
        config.default_bounds,
        config.spatial_bounds,
    );
    register_stub_spawns(server.spawns_mut());
    Ok(server)
}

/// Spawn every stub-map entity, returning the live actors in spawn order.
pub fn spawn_stub_map(
    server: &mut Server<GuestServerLogic>,
    stub: &StubMap,
) -> Result<Vec<qa_core::identity::OwnedActor>, AppError> {
    let mut actors = Vec::with_capacity(stub.entities.len());
    for fields in &stub.entities {
        actors.push(server.spawn_entity(fields)?);
    }
    Ok(actors)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::parse_application_command;

    fn options(words: &[&str]) -> ApplicationOptions {
        let argv: Vec<String> = words.iter().map(|word| (*word).to_string()).collect();
        match parse_application_command(&argv).unwrap() {
            crate::options::ApplicationCommand::Run { options }
            | crate::options::ApplicationCommand::Menu { options } => options,
            other => panic!("expected run or menu, got {other:?}"),
        }
    }

    #[test]
    fn dedicated_has_no_seats() {
        let config = StartupConfig::from_options(&options(&["--dedicated", "--seats", "4"])).unwrap();
        assert!(config.dedicated);
        assert_eq!(config.seats, 0);
        let config = StartupConfig::from_options(&options(&["--seats", "2"])).unwrap();
        assert!(!config.dedicated);
        assert_eq!(config.seats, 2);
    }

    #[test]
    fn windowed_flows_from_options() {
        let config = StartupConfig::from_options(&options(&["--windowed", "--frames", "3"])).unwrap();
        assert!(config.windowed);
        assert!(!config.dedicated);
        assert_eq!(config.frame_limit, Some(3));
        let config = StartupConfig::from_options(&options(&["--menu"])).unwrap();
        assert!(!config.windowed);
    }

    #[test]
    fn clock_profiles_follow_movement() {
        let config = StartupConfig::from_options(&options(&["--movement", "q1"])).unwrap();
        assert!(matches!(config.profile, ClockProfile::Q1Netquake { .. }));
        assert!(matches!(config.plan, TickPlan::Clamped { .. }));
        assert_eq!(config.client_family, ClientFamily::Q1Netquake);

        let config = StartupConfig::from_options(&options(&["--movement", "qw"])).unwrap();
        assert!(matches!(config.profile, ClockProfile::Q1Quakeworld { .. }));
        assert_eq!(config.client_family, ClientFamily::Q1Quakeworld);

        let config = StartupConfig::from_options(&options(&["--movement", "q2"])).unwrap();
        assert!(matches!(config.profile, ClockProfile::Q2Classic));
        assert_eq!(config.step, SourceTime::Milliseconds(100));
        assert_eq!(config.client_family, ClientFamily::Q2Classic);

        let config = StartupConfig::from_options(&options(&["--movement", "q3"])).unwrap();
        assert!(matches!(config.profile, ClockProfile::Q3 { .. }));
        assert_eq!(config.step, SourceTime::Milliseconds(50));
        assert_eq!(config.client_family, ClientFamily::Q3);
    }

    #[test]
    fn stub_map_spawns_world_and_starts() {
        let config = StartupConfig::from_options(&options(&["--movement", "q1"])).unwrap();
        let mut server = open_server(&config).unwrap();
        let stub = load_stub_map(&config.map);
        assert_eq!(stub.entities.len(), 5);
        let actors = spawn_stub_map(&mut server, &stub).unwrap();
        assert_eq!(actors.len(), 5);
        assert_eq!(server.simulation().actor_count(), 5);
        for actor in &actors {
            assert!(server.simulation().body_state(actor.id()).is_some());
        }
    }

    #[test]
    fn stub_spawns_reject_unknown_classnames() {
        let config = StartupConfig::from_options(&options(&["--menu"])).unwrap();
        let mut server = open_server(&config).unwrap();
        let fields = SpawnFields {
            classname: "monster_ogre".to_string(),
            ..SpawnFields::default()
        };
        assert!(server.spawn_entity(&fields).is_err());
    }
}
