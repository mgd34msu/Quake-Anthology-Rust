//! Q1 mission-pack world root (`src/content/q1/missionpacks/world`, barrel `index.ts`).
//!
//! Renames against the donor barrel: `registerMissionpackWorld` is
//! [`register_missionpack_world`], `Q1MissionpackWorld` is
//! [`Q1MissionpackWorld`], and `MissionpackWorldHooks` is
//! [`MissionpackWorldHooks`]. The donor passes the hooks and game
//! handle through the constructor; this port installs them per game
//! (keyed like the base registry) and exposes the session as
//! associated functions, since the sibling runtime reaches the world
//! without holding the handle. `select_spawn` reports the spawn
//! entity id instead of the entity, and the value-returning probes
//! (`saved_team`, `tag_score`, the rune scalers) fall back to the
//! donor defaults when the Rogue systems are absent. `charmer` takes
//! the game handle, matching the sibling game-passing hooks.

pub mod campaign;
pub mod common;
pub mod finale_text;
pub mod hipnotic_hazards;
pub mod hipnotic_misc;
pub mod hipnotic_particles;
pub mod hipnotic_rotate;
pub mod hipnotic_spawn;
pub mod hipnotic_train;
pub mod hipnotic_triggers;
pub mod rogue_ending;
pub mod rogue_hazards;
pub mod rogue_misc;
pub mod rogue_pendulum;
pub mod rogue_plats;
pub mod rogue_runes;
pub mod rogue_tag;
pub mod rogue_teams;
pub mod rogue_time;
pub mod rotate_targets;
pub mod shooters;

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use qa_core::identity::ActorId;
use qa_core::math::Vec3;

use crate::q1::base::rules::Q1SourceFinale;
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::missionpacks::types::Q1MissionPack;
use crate::q1::Q1Error;

use self::campaign::register_mission_campaign;
use self::hipnotic_hazards::register_hipnotic_hazards;
use self::hipnotic_misc::{earthquake_after_physics, register_hipnotic_misc};
use self::hipnotic_particles::register_hipnotic_particles;
use self::hipnotic_rotate::register_hipnotic_rotation;
use self::hipnotic_spawn::register_hipnotic_spawn;
use self::hipnotic_train::register_hipnotic_train;
use self::hipnotic_triggers::register_hipnotic_triggers;
use self::rogue_ending::{register_rogue_ending, start_rogue_ending};
use self::rogue_hazards::{register_rogue_hazards, rogue_earthquake};
use self::rogue_misc::register_rogue_misc;
use self::rogue_pendulum::register_rogue_pendulum;
use self::rogue_plats::register_rogue_plats;
use self::rogue_runes::RogueRunes;
use self::rogue_tag::RogueTag;
use self::rogue_teams::RogueTeams;
use self::rogue_time::{crash_time_machine, register_rogue_time};
use self::shooters::register_mission_shooters;

/// Host callbacks the mission-pack world needs (`MissionpackWorldHooks`).
#[derive(Default)]
#[allow(clippy::type_complexity)]
pub struct MissionpackWorldHooks {
    /// Resolve the current horn charmer.
    pub charmer: Option<Box<dyn Fn(&Q1EntityServices) -> Option<ActorId> + Send>>,
    /// Charm a spawned monster for the charmer.
    pub charm: Option<Box<dyn Fn(&mut Q1EntityServices, &ActorId, &ActorId) -> Result<(), Q1Error> + Send>>,
    /// Spawn a decoy at an origin.
    pub become_decoy: Option<Box<dyn Fn(&mut Q1EntityServices, &str, Vec3) -> Result<ActorId, Q1Error> + Send>>,
    /// Present a finale result.
    pub present_finale: Option<Box<dyn Fn(&Q1SourceFinale) + Send>>,
    /// Read the Rogue game configuration bits.
    pub gamecfg: Option<Box<dyn Fn() -> i32 + Send>>,
    /// Read a player's team color.
    pub team_color: Option<Box<dyn Fn(&ActorId) -> i32 + Send>>,
    /// Write a player's team color.
    pub set_team_color: Option<Box<dyn Fn(&ActorId, i32) + Send>>,
    /// Adjust a player's frags.
    pub add_frags: Option<Box<dyn Fn(&ActorId, i32) + Send>>,
    /// Read a player's frags.
    pub frags: Option<Box<dyn Fn(&ActorId) -> i32 + Send>>,
    /// Disconnect a player.
    pub disconnect: Option<Box<dyn Fn(&ActorId) + Send>>,
    /// Read a player's character frame.
    pub player_frame: Option<Box<dyn Fn(&ActorId) -> i32 + Send>>,
    /// Read a player's name.
    pub player_name: Option<Box<dyn Fn(&ActorId) -> String + Send>>,
}

/// Per-game world session state.
struct MissionpackWorldState {
    pack: Q1MissionPack,
    hooks: MissionpackWorldHooks,
}

/// Installed per-game world sessions, keyed like the base registry.
static STATES: OnceLock<Mutex<HashMap<usize, MissionpackWorldState>>> = OnceLock::new();

/// Registry key for a game.
fn world_key(game: &Q1EntityServices) -> usize {
    std::ptr::from_ref(game) as usize
}

/// Locked world registry.
fn lock_registry() -> std::sync::MutexGuard<'static, HashMap<usize, MissionpackWorldState>> {
    STATES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Pack installed on a game (Hipnotic when unregistered, whose facade
/// is all no-ops and defaults).
fn world_pack(game: &Q1EntityServices) -> Q1MissionPack {
    lock_registry()
        .get(&world_key(game))
        .map(|state| state.pack)
        .unwrap_or(Q1MissionPack::Hipnotic)
}

/// Install the world hooks (`hooks` constructor argument). Keeps an
/// already-registered pack; use [`register_missionpack_world`] to set it.
pub fn install_missionpack_hooks(game: &mut Q1EntityServices, hooks: MissionpackWorldHooks) {
    let mut registry = lock_registry();
    let key = world_key(game);
    let pack = registry
        .get(&key)
        .map(|state| state.pack)
        .unwrap_or(Q1MissionPack::Hipnotic);
    registry.insert(key, MissionpackWorldState { pack, hooks });
}

/// Run a callback with the installed world hooks.
pub fn with_missionpack_hooks<R>(
    game: &mut Q1EntityServices,
    run: impl FnOnce(&mut Q1EntityServices, &MissionpackWorldHooks) -> Result<R, Q1Error>,
) -> Result<R, Q1Error> {
    let registry = lock_registry();
    let fallback;
    let hooks = match registry.get(&world_key(game)) {
        Some(state) => &state.hooks,
        None => {
            fallback = MissionpackWorldHooks::default();
            &fallback
        }
    };
    run(game, hooks)
}

/// Rogue teams handle (unit struct, registered by the world).
const TEAMS: RogueTeams = RogueTeams;
/// Rogue runes handle (unit struct, registered by the world).
const RUNES: RogueRunes = RogueRunes;
/// Rogue tag handle (unit struct, registered by the world).
const TAG: RogueTag = RogueTag;

/// Mission-pack world session (`Q1MissionpackWorld`).
pub struct Q1MissionpackWorld {
    /// Mission pack.
    pub pack: Q1MissionPack,
}

/// Register the mission-pack world (`registerMissionpackWorld`).
pub fn register_missionpack_world(
    game: &mut Q1EntityServices,
    pack: Q1MissionPack,
    hooks: MissionpackWorldHooks,
) -> Result<Q1MissionpackWorld, Q1Error> {
    register_mission_shooters(game, pack)?;
    register_mission_campaign(game, pack)?;
    if pack == Q1MissionPack::Hipnotic {
        register_hipnotic_triggers(game)?;
        register_hipnotic_train(game)?;
        register_hipnotic_rotation(game)?;
        register_hipnotic_misc(game)?;
        register_hipnotic_particles(game)?;
        register_hipnotic_spawn(game)?;
        register_hipnotic_hazards(game)?;
    } else {
        register_rogue_misc(game)?;
        register_rogue_time(game)?;
        register_rogue_ending(game)?;
        register_rogue_pendulum(game)?;
        register_rogue_plats(game)?;
        register_rogue_hazards(game)?;
        RogueRunes::new(game)?;
        RogueTeams::new(game)?;
        RogueTag::new(game)?;
    }
    lock_registry().insert(world_key(game), MissionpackWorldState { pack, hooks });
    Ok(Q1MissionpackWorld { pack })
}

impl Q1MissionpackWorld {
    /// Tick quakes, teams, runes, and the ending (`afterPhysics`).
    pub fn after_physics(game: &mut Q1EntityServices, actor: &ActorId, _seconds: f64) -> Result<(), Q1Error> {
        if world_pack(game) == Q1MissionPack::Hipnotic {
            return earthquake_after_physics(game, actor);
        }
        let world = game.world.clone();
        let active = world
            .as_ref()
            .and_then(|world| game.entity(world))
            .map(|world| world.number("rogue:earthquake_active"))
            .unwrap_or(0.0);
        if active == 1.0 {
            let intensity = world
                .as_ref()
                .and_then(|world| game.entity(world))
                .map(|world| world.number("rogue:earthquake_intensity"))
                .unwrap_or(0.0);
            rogue_earthquake(game, actor, intensity)?;
        }
        TEAMS.frame(game, actor)?;
        RUNES.frame(game, actor)?;
        start_rogue_ending(game, actor)
    }

    /// Admit a spawned player to a team (`playerSpawned`).
    pub fn player_spawned(game: &mut Q1EntityServices, actor: &ActorId) -> Result<(), Q1Error> {
        if world_pack(game) == Q1MissionPack::Hipnotic {
            return Ok(());
        }
        TEAMS.player_spawned(game, actor)
    }

    /// Drop a player's carried flag (`dropCarriedFlag`).
    pub fn drop_carried_flag(game: &mut Q1EntityServices, actor: &ActorId) -> Result<(), Q1Error> {
        if world_pack(game) == Q1MissionPack::Hipnotic {
            return Ok(());
        }
        TEAMS.drop_carried_flag(game, actor)
    }

    /// Handle a team impulse (`impulse`).
    pub fn impulse(game: &mut Q1EntityServices, actor: &ActorId, impulse: i32) -> Result<bool, Q1Error> {
        if world_pack(game) == Q1MissionPack::Hipnotic {
            return Ok(false);
        }
        TEAMS.impulse(game, actor, impulse)
    }

    /// Read a player's saved team (`savedTeam`).
    pub fn saved_team(game: &mut Q1EntityServices, actor: &ActorId) -> i32 {
        if world_pack(game) == Q1MissionPack::Hipnotic {
            return 0;
        }
        TEAMS.team(game, Some(actor)).unwrap_or(0)
    }

    /// Select a team spawn (`selectSpawn`).
    pub fn select_spawn(game: &mut Q1EntityServices, actor: &ActorId) -> Option<ActorId> {
        if world_pack(game) == Q1MissionPack::Hipnotic {
            return None;
        }
        TEAMS.select_spawn(game, actor).unwrap_or(None)
    }

    /// Score a tag kill (`tagScore`).
    pub fn tag_score(game: &mut Q1EntityServices, victim: &ActorId, attacker: &ActorId) -> i32 {
        if world_pack(game) == Q1MissionPack::Hipnotic {
            return 1;
        }
        TAG.score(game, victim, attacker).unwrap_or(1)
    }

    /// Record confirmed damage for assists (`confirmedDamage`).
    pub fn confirmed_damage(
        game: &mut Q1EntityServices,
        target: &ActorId,
        attacker: Option<&ActorId>,
    ) -> Result<(), Q1Error> {
        if world_pack(game) == Q1MissionPack::Hipnotic {
            return Ok(());
        }
        TEAMS.confirmed_damage(game, target, attacker)
    }

    /// Handle player death: teams plus rune drops (`playerDied`).
    pub fn player_died(
        game: &mut Q1EntityServices,
        actor: &ActorId,
        attacker: Option<&ActorId>,
    ) -> Result<(), Q1Error> {
        if world_pack(game) == Q1MissionPack::Hipnotic {
            return Ok(());
        }
        TEAMS.player_died(game, actor, attacker)?;
        RUNES.drop(game, actor)
    }

    /// Apply the haste rune to an attack delay (`runeAttackDelay`).
    pub fn rune_attack_delay(game: &mut Q1EntityServices, actor: &ActorId, delay: f64) -> f64 {
        if world_pack(game) == Q1MissionPack::Hipnotic {
            return delay;
        }
        RUNES.attack_delay(game, actor, delay).unwrap_or(delay)
    }

    /// Play the strength rune attack sound (`runeAttackSound`).
    pub fn rune_attack_sound(game: &mut Q1EntityServices, actor: &ActorId) -> Result<(), Q1Error> {
        if world_pack(game) == Q1MissionPack::Hipnotic {
            return Ok(());
        }
        RUNES.attack_sound(game, actor)
    }

    /// Apply the strength rune to damage (`runeDamage`).
    pub fn rune_damage(game: &mut Q1EntityServices, actor: &ActorId, amount: f64) -> f64 {
        if world_pack(game) == Q1MissionPack::Hipnotic {
            return amount;
        }
        RUNES.damage(game, actor, amount).unwrap_or(amount)
    }

    /// Apply the resistance rune to damage (`runeResistance`).
    pub fn rune_resistance(game: &mut Q1EntityServices, actor: &ActorId, amount: f64) -> f64 {
        if world_pack(game) == Q1MissionPack::Hipnotic {
            return amount;
        }
        RUNES.resistance(game, actor, amount).unwrap_or(amount)
    }

    /// Whether an actor carries the regeneration rune (`hasRegenerationRune`).
    pub fn has_regeneration_rune(game: &mut Q1EntityServices, actor: &ActorId) -> bool {
        if world_pack(game) == Q1MissionPack::Hipnotic {
            return false;
        }
        RUNES.has_regeneration(game, actor).unwrap_or(false)
    }

    /// Crash the time machine (`crashTimeMachine`).
    pub fn crash_time_machine(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
        crash_time_machine(game)
    }
}

#[cfg(test)]
mod tests {
    use super::{register_missionpack_world, MissionpackWorldHooks, Q1MissionpackWorld};
    use crate::q1::base::provider::{Q1BaseGuard, Q1BaseOptions};
    use crate::q1::missionpacks::types::{test_game, Q1MissionPack};

    #[test]
    fn hipnotic_world_reports_defaults() {
        let mut game = test_game();
        let _guard = Q1BaseGuard::register(&mut game, Q1BaseOptions::default()).expect("base");
        let world = register_missionpack_world(&mut game, Q1MissionPack::Hipnotic, MissionpackWorldHooks::default())
            .expect("world");
        assert_eq!(world.pack, Q1MissionPack::Hipnotic);
        let player = game.create("player", None, None).expect("player");
        Q1MissionpackWorld::after_physics(&mut game, &player, 0.1).expect("tick");
        assert!(!Q1MissionpackWorld::impulse(&mut game, &player, 23).expect("impulse"));
        assert_eq!(Q1MissionpackWorld::saved_team(&mut game, &player), 0);
        assert!(Q1MissionpackWorld::select_spawn(&mut game, &player).is_none());
        assert_eq!(Q1MissionpackWorld::tag_score(&mut game, &player, &player), 1);
        assert_eq!(Q1MissionpackWorld::rune_damage(&mut game, &player, 10.0), 10.0);
        assert!(!Q1MissionpackWorld::has_regeneration_rune(&mut game, &player));
        assert!(Q1MissionpackWorld::crash_time_machine(&mut game).is_err());
    }

    #[test]
    fn rogue_world_runs_teams_and_runes() {
        let mut game = test_game();
        let _guard = Q1BaseGuard::register(&mut game, Q1BaseOptions::default()).expect("base");
        let world = register_missionpack_world(
            &mut game,
            Q1MissionPack::Rogue,
            MissionpackWorldHooks {
                team_color: Some(Box::new(|_| 0)),
                set_team_color: Some(Box::new(|_, _| {})),
                add_frags: Some(Box::new(|_, _| {})),
                frags: Some(Box::new(|_| 0)),
                player_name: Some(Box::new(|_| "player".to_string())),
                ..Default::default()
            },
        )
        .expect("world");
        assert_eq!(world.pack, Q1MissionPack::Rogue);
        let player = game.create("player", None, None).expect("player");
        Q1MissionpackWorld::after_physics(&mut game, &player, 0.1).expect("tick");
        Q1MissionpackWorld::player_spawned(&mut game, &player).expect("spawned");
        Q1MissionpackWorld::drop_carried_flag(&mut game, &player).expect("flag");
        Q1MissionpackWorld::confirmed_damage(&mut game, &player, None).expect("damage");
        Q1MissionpackWorld::player_died(&mut game, &player, None).expect("died");
        Q1MissionpackWorld::rune_attack_sound(&mut game, &player).expect("sound");
        let world_entity = game.create("worldspawn", None, None).expect("world");
        let machine = game.create("item_time_machine", None, None).expect("machine");
        game.update_entity(&world_entity, |entity| {
            entity.references.insert("rogue:theMachine".to_string(), Some(machine));
        })
        .expect("machine ref");
        game.world = Some(world_entity);
        Q1MissionpackWorld::crash_time_machine(&mut game).expect("crash");
    }
}
