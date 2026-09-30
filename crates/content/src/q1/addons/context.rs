//! Q1 addon context (`src/content/q1/addons/context.ts`).
//!
//! Source-private addon words share the base campaign flags and all
//! gameplay authorities. The donor keys context state off the game
//! through a class instance; Rust callbacks are bare function pointers,
//! so per-game addon state lives in the module registry below, keyed by
//! game address. Access runs through [`update_addons`], whose closure
//! receives only the state: game calls cannot run while the registry is
//! locked, which rules out reentrant deadlocks by construction. Flows
//! that need both state and the game clone the state out, operate, then
//! store it back.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Mutex, OnceLock};

use qa_core::identity::{ActorId, SavedActorId};
use qa_core::math::Vec3;

use crate::q1::addons::campaign::mg3_rune_count;
use crate::q1::foundation::callbacks::Q1StateExtension;
use crate::q1::foundation::checkpoint::{decode_checkpoint_value, encode_checkpoint_value};
use crate::q1::foundation::entity::{move_direction, Q1Actor};
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::gameplay::BodyPatch;
use crate::q1::foundation::host::Q1ReleaseHook;
use crate::q1::foundation::types::{Q1MoveType, Q1Solid, ZERO};
use crate::q1::{q1_error, Q1Error};
use crate::value::{arr, int, num, obj, str as save_str, SaveJson, SaveReader};

/// Round to binary32 storage (donor `Math.fround`).
#[must_use]
pub(crate) fn fround(value: f64) -> f64 {
    f64::from(qa_core::numeric::store_f32(value))
}

/// Addon program id (`Q1AddonProgram`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q1AddonProgram {
    /// Dimension of the Past.
    Dopa,
    /// Dimension of the Machine.
    Mg1,
    /// Honey (Slipgate Ironworks expansion).
    Mg3,
    /// Capture the flag.
    Ctf,
}

impl Q1AddonProgram {
    /// Donor program text.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Q1AddonProgram::Dopa => "dopa",
            Q1AddonProgram::Mg1 => "mg1",
            Q1AddonProgram::Mg3 => "mg3",
            Q1AddonProgram::Ctf => "ctf",
        }
    }

    /// Parse donor program text.
    pub fn parse(text: &str) -> Result<Self, Q1Error> {
        match text {
            "dopa" => Ok(Q1AddonProgram::Dopa),
            "mg1" => Ok(Q1AddonProgram::Mg1),
            "mg3" => Ok(Q1AddonProgram::Mg3),
            "ctf" => Ok(Q1AddonProgram::Ctf),
            _ => Err(q1_error(format!("Unknown Q1 addon program: {text}"))),
        }
    }
}

/// Addon lightning style (`lightning` event `style`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q1AddonLightningStyle {
    /// Style 1.
    Style1,
    /// Style 2.
    Style2,
    /// Style 3.
    Style3,
}

impl Q1AddonLightningStyle {
    /// Donor style number.
    #[must_use]
    pub fn as_i32(self) -> i32 {
        match self {
            Q1AddonLightningStyle::Style1 => 1,
            Q1AddonLightningStyle::Style2 => 2,
            Q1AddonLightningStyle::Style3 => 3,
        }
    }

    /// Parse a donor style number.
    pub fn parse(style: i32) -> Result<Self, Q1Error> {
        match style {
            1 => Ok(Q1AddonLightningStyle::Style1),
            2 => Ok(Q1AddonLightningStyle::Style2),
            3 => Ok(Q1AddonLightningStyle::Style3),
            _ => Err(q1_error(format!("Unknown Q1 addon lightning style: {style}"))),
        }
    }
}

/// Addon presentation event (`Q1AddonEvent`).
#[derive(Debug, Clone, PartialEq)]
pub enum Q1AddonEvent {
    /// Switch the CD track.
    Music {
        /// New track.
        track: i32,
        /// Loop track.
        loop_track: i32,
    },
    /// Show the shareware sell screen.
    SellScreen,
    /// Set an entity fade alpha.
    Alpha {
        /// Fading actor.
        actor: ActorId,
        /// Fade alpha.
        alpha: f64,
    },
    /// A rune was collected.
    RuneCollected {
        /// Collecting player.
        player: ActorId,
        /// Collected rune bits.
        bits: i32,
        /// Addon program.
        program: Q1AddonProgram,
    },
    /// Move a cutscene camera.
    Cutscene {
        /// Camera origin.
        camera: Vec3,
        /// Camera angles.
        angles: Vec3,
    },
    /// Set fog parameters.
    Fog {
        /// Viewing player, if any.
        player: Option<ActorId>,
        /// Fog density.
        density: f64,
        /// Fog color.
        color: Vec3,
        /// Sky factor.
        sky_factor: f64,
        /// Blend duration in seconds.
        duration: f64,
    },
    /// Punch a player view.
    PunchAngle {
        /// Punched player.
        player: ActorId,
        /// Punch angles.
        angles: Vec3,
    },
    /// Roll a player view.
    ViewRoll {
        /// Rolled player.
        player: ActorId,
        /// Roll angle.
        roll: f64,
    },
    /// Draw a lightning beam.
    Lightning {
        /// Owning actor.
        actor: ActorId,
        /// Beam style.
        style: Q1AddonLightningStyle,
        /// Beam start.
        start: Vec3,
        /// Beam end.
        end: Vec3,
    },
    /// Broadcast a colored explosion.
    ColoredExplosion {
        /// Explosion origin.
        origin: Vec3,
        /// First particle color.
        color_start: i32,
        /// Particle color run length.
        color_length: i32,
    },
    /// Report the monster total.
    MonsterCount {
        /// Total monsters.
        count: i32,
    },
    /// Print a developer message.
    DeveloperMessage {
        /// Message text.
        text: String,
    },
    /// Set entity effect flags.
    ActorEffects {
        /// Target actor.
        actor: ActorId,
        /// Effect flags.
        effects: i32,
    },
    /// Draw debug bounds.
    DebugBounds {
        /// Bounds minimum.
        min: Vec3,
        /// Bounds maximum.
        max: Vec3,
        /// Line color.
        color: i32,
        /// Display lifetime in seconds.
        lifetime: f64,
        /// Whether depth testing applies.
        depth_test: bool,
    },
}

/// Cheat arsenal category (`cheatArsenal` category).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q1AddonCheatCategory {
    /// Grant weapons.
    Weapons,
    /// Grant ammunition.
    Ammo,
}

impl Q1AddonCheatCategory {
    /// Donor category text.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Q1AddonCheatCategory::Weapons => "weapons",
            Q1AddonCheatCategory::Ammo => "ammo",
        }
    }
}

/// Engine services for addons (`Q1AddonServices`).
pub trait Q1AddonServices: Send {
    /// Grant a cheat arsenal; the donor hook is optional and defaults
    /// to refusing.
    fn cheat_arsenal(&mut self, _actor: &ActorId, _category: Q1AddonCheatCategory) -> bool {
        false
    }

    /// Present an addon event.
    fn emit(&mut self, event: Q1AddonEvent);

    /// Whether an actor is a monster.
    fn is_monster(&mut self, actor: &ActorId) -> bool;

    /// Read an engine variable.
    fn cvar(&mut self, name: &str) -> f64;

    /// Write an engine variable.
    fn set_cvar(&mut self, name: &str, value: &str);

    /// Recorded events for tests.
    #[cfg(test)]
    fn test_events(&self) -> Vec<Q1AddonEvent> {
        Vec::new()
    }
}

/// Per-game addon state (`Q1AddonContext` fields).
pub struct Q1AddonState {
    /// Addon program.
    pub program: Q1AddonProgram,
    /// Engine services.
    pub services: Box<dyn Q1AddonServices>,
    /// Last addon frame time in seconds.
    pub frame_time: f64,
    player_words: HashMap<ActorId, HashMap<String, f64>>,
    player_references: HashMap<ActorId, HashMap<String, Option<ActorId>>>,
    frame_ticks: Vec<ActorId>,
}

/// Addon context handle (`Q1AddonContext`). Registration entry points
/// take this handle plus the game; the live words live in the per-game
/// registry so bare callback pointers can reach them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q1AddonContext {
    program: Q1AddonProgram,
}

impl Q1AddonContext {
    /// Fresh handle for a program.
    #[must_use]
    pub fn new(program: Q1AddonProgram) -> Self {
        Self { program }
    }

    /// Addon program.
    #[must_use]
    pub fn program(&self) -> Q1AddonProgram {
        self.program
    }
}

fn registry() -> &'static Mutex<HashMap<usize, Q1AddonState>> {
    static STATES: OnceLock<Mutex<HashMap<usize, Q1AddonState>>> = OnceLock::new();
    STATES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn addon_key(game: &Q1EntityServices) -> usize {
    std::ptr::from_ref(game) as usize
}

fn lock_registry() -> std::sync::MutexGuard<'static, HashMap<usize, Q1AddonState>> {
    registry().lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Run a pure state operation. The closure receives only the state so
/// game calls (which may reenter addon callbacks) cannot run under the
/// registry lock.
pub(crate) fn update_addons<T>(game: &Q1EntityServices, op: impl FnOnce(&mut Q1AddonState) -> T) -> Result<T, Q1Error> {
    let mut states = lock_registry();
    let state = states
        .get_mut(&addon_key(game))
        .ok_or_else(|| q1_error("Q1 addons were not registered"))?;
    Ok(op(state))
}

/// Whether addon content is registered on a game.
#[must_use]
pub fn addons_registered(game: &Q1EntityServices) -> bool {
    lock_registry().contains_key(&addon_key(game))
}

/// Addon program registered on a game.
pub fn addon_program(game: &Q1EntityServices) -> Result<Q1AddonProgram, Q1Error> {
    update_addons(game, |state| state.program)
}

/// Last addon frame time in seconds.
pub fn addon_frame_time(game: &Q1EntityServices) -> Result<f64, Q1Error> {
    update_addons(game, |state| state.frame_time)
}

/// Read an addon player word (`playerNumber`).
pub fn addon_player_number(game: &Q1EntityServices, actor: &ActorId, name: &str) -> Result<f64, Q1Error> {
    Ok(addon_player_word(game, actor, name)?.unwrap_or(0.0))
}

/// Read an addon player word when present (`playerWord`).
pub fn addon_player_word(game: &Q1EntityServices, actor: &ActorId, name: &str) -> Result<Option<f64>, Q1Error> {
    let owned = match game.host.actors.resolve_owned(actor) {
        Some(owned) => owned,
        None => return Ok(None),
    };
    let id = owned.id().clone();
    update_addons(game, |state| {
        state.player_words.get(&id).and_then(|words| words.get(name).copied())
    })
}

/// Write an addon player word (`setPlayerNumber`).
pub fn set_addon_player_number(
    game: &mut Q1EntityServices,
    actor: &ActorId,
    name: &str,
    value: f64,
) -> Result<(), Q1Error> {
    let owned = game
        .host
        .actors
        .resolve_owned(actor)
        .ok_or_else(|| q1_error("Addon player is no longer admitted"))?;
    let id = owned.id().clone();
    let value = fround(value);
    update_addons(game, |state| {
        state
            .player_words
            .entry(id)
            .or_default()
            .insert(name.to_string(), value);
    })
}

/// Read an addon player reference (`playerReference`).
pub fn addon_player_reference(
    game: &Q1EntityServices,
    actor: &ActorId,
    name: &str,
) -> Result<Option<ActorId>, Q1Error> {
    let owned = match game.host.actors.resolve_owned(actor) {
        Some(owned) => owned,
        None => return Ok(None),
    };
    let id = owned.id().clone();
    update_addons(game, |state| {
        state
            .player_references
            .get(&id)
            .and_then(|references| references.get(name).cloned())
            .flatten()
    })
}

/// Write an addon player reference (`setPlayerReference`).
pub fn set_addon_player_reference(
    game: &mut Q1EntityServices,
    actor: &ActorId,
    name: &str,
    target: Option<&ActorId>,
) -> Result<(), Q1Error> {
    let owned = game
        .host
        .actors
        .resolve_owned(actor)
        .ok_or_else(|| q1_error("Addon player is no longer admitted"))?;
    let id = owned.id().clone();
    let target = target.cloned();
    update_addons(game, |state| {
        state
            .player_references
            .entry(id)
            .or_default()
            .insert(name.to_string(), target);
    })
}

/// Write a binary32 entity number (`setNumber`).
pub fn set_addon_number(game: &mut Q1EntityServices, id: &ActorId, name: &str, value: f64) -> Result<(), Q1Error> {
    let text = fround(value).to_string();
    game.update_entity(id, |entity| {
        entity.fields.insert(name.to_string(), text);
    })
}

/// Write a binary32 entity vector (`setVector`).
pub fn set_addon_vector(game: &mut Q1EntityServices, id: &ActorId, name: &str, value: Vec3) -> Result<(), Q1Error> {
    let text = format!(
        "{} {} {}",
        fround(f64::from(value.x)),
        fround(f64::from(value.y)),
        fround(f64::from(value.z))
    );
    game.update_entity(id, |entity| {
        entity.fields.insert(name.to_string(), text);
    })
}

/// Present an addon event.
pub fn addon_emit(game: &Q1EntityServices, event: Q1AddonEvent) -> Result<(), Q1Error> {
    update_addons(game, |state| state.services.emit(event))
}

/// Read an engine variable.
pub fn addon_cvar(game: &Q1EntityServices, name: &str) -> Result<f64, Q1Error> {
    update_addons(game, |state| state.services.cvar(name))
}

/// Write an engine variable.
pub fn addon_set_cvar(game: &Q1EntityServices, name: &str, value: &str) -> Result<(), Q1Error> {
    update_addons(game, |state| state.services.set_cvar(name, value))
}

/// Whether an actor is a monster.
pub fn addon_is_monster(game: &Q1EntityServices, actor: &ActorId) -> Result<bool, Q1Error> {
    update_addons(game, |state| state.services.is_monster(actor))
}

/// Grant a cheat arsenal.
pub fn addon_cheat_arsenal(
    game: &Q1EntityServices,
    actor: &ActorId,
    category: Q1AddonCheatCategory,
) -> Result<bool, Q1Error> {
    update_addons(game, |state| state.services.cheat_arsenal(actor, category))
}

/// Fade an entity (`alpha`).
pub fn addon_alpha(game: &mut Q1EntityServices, id: &ActorId, alpha: f64) -> Result<(), Q1Error> {
    set_addon_number(game, id, "alpha", alpha)?;
    let actor = id.clone();
    addon_emit(game, Q1AddonEvent::Alpha { actor, alpha })
}

/// Retarget an actor's combat team, preserving the remaining traits
/// (`setTraits` with `team`).
pub fn set_combat_team(
    game: &mut Q1EntityServices,
    actor: &qa_core::identity::OwnedActor,
    team: Option<&str>,
) -> Result<(), Q1Error> {
    use crate::q1::foundation::gameplay::CombatTraits;

    let Some(combat) = game.host.combat.read(actor.id()) else {
        return Ok(());
    };
    game.host.combat.set_traits(
        actor,
        CombatTraits {
            can_take_damage: combat.can_take_damage,
            mass: combat.mass,
            invulnerable: combat.invulnerable,
            team: team.map(str::to_string),
            no_knockback: combat.no_knockback,
        },
    )
}

/// Print a message to every player (`broadcast`).
pub fn addon_broadcast(game: &mut Q1EntityServices, text: &str) {
    for player in (game.host.players)() {
        game.message_simple(Some(&player), text);
    }
}

/// Remove an entity outside cooperative mode (`removedOutsideCoop`).
pub fn removed_outside_coop(game: &mut Q1EntityServices, id: &ActorId, inhibit_coop: bool) -> Result<bool, Q1Error> {
    let coop = game.options().coop;
    let spawnflags = game
        .entity_ref(id)
        .map(|entity| entity.spawnflags)
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    let removed = if coop {
        inhibit_coop && (spawnflags & 131072) != 0
    } else {
        (spawnflags & 32768) != 0
    };
    if removed {
        game.remove(id)?;
    }
    Ok(removed)
}

/// Remove a rune-gated Honey entity (`removedForRunes`).
pub fn removed_for_runes(game: &mut Q1EntityServices, id: &ActorId) -> Result<bool, Q1Error> {
    if addon_program(game)? != Q1AddonProgram::Mg3 {
        return Ok(false);
    }
    let flags = crate::q1::base::provider::update_base(game, |state| state.campaign.read_flags())?;
    let flag = 262144 * 2_i32.pow(mg3_rune_count(flags) as u32);
    let spawnflags = game
        .entity_ref(id)
        .map(|entity| entity.spawnflags)
        .ok_or_else(|| q1_error("Missing Q1 entity"))?;
    if (spawnflags & flag) == 0 {
        return Ok(false);
    }
    game.remove(id)?;
    Ok(true)
}

/// Initialize a trigger volume (`initTrigger`).
pub fn init_trigger(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    if removed_outside_coop(game, id, false)? || removed_for_runes(game, id)? {
        return Ok(());
    }
    let angles = game.body(id)?.angles;
    let entity = game.entity_ref(id).ok_or_else(|| q1_error("Missing Q1 entity"))?;
    let direction = if entity.fields.contains_key("movedir") {
        entity.vector("movedir")
    } else {
        entity.movedir
    };
    let movedir = if (angles.x == 0.0 && angles.y == 0.0 && angles.z == 0.0)
        || direction.x != 0.0
        || direction.y != 0.0
        || direction.z != 0.0
    {
        direction
    } else {
        move_direction(angles, Some(game))
    };
    game.update_entity(id, |entity| {
        entity.movedir = movedir;
        entity.solid = Q1Solid::Trigger;
        entity.movement = Q1MoveType::None;
        entity.model.clear();
    })?;
    game.set_body(
        id,
        &BodyPatch {
            angles: Some(ZERO),
            ..Default::default()
        },
    )
}

/// Register an entity for the addon frame phase (`addFrameTick`).
pub fn add_frame_tick(game: &mut Q1EntityServices, id: &ActorId, name: &str) -> Result<(), Q1Error> {
    let name = name.to_string();
    game.update_entity(id, |entity| {
        entity.fields.insert("addon.frameTick".to_string(), name);
    })?;
    update_addons(game, |state| {
        state.frame_ticks.insert(0, id.clone());
    })
}

/// Unregister an entity from the addon frame phase (`removeFrameTick`).
pub fn remove_frame_tick(game: &Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    update_addons(game, |state| {
        if let Some(index) = state.frame_ticks.iter().position(|tick| tick == id) {
            state.frame_ticks.remove(index);
        }
    })
}

/// Run the addon frame phase; this does not advance simulation time
/// (`frame`).
pub fn frame_addons(game: &mut Q1EntityServices, seconds: f64) -> Result<(), Q1Error> {
    let ticks = update_addons(game, |state| {
        state.frame_time = seconds;
        state.frame_ticks.clone()
    })?;
    for id in &ticks {
        let tick = game
            .entity_ref(id)
            .map(|entity| entity.text("addon.frameTick"))
            .unwrap_or_default();
        if !tick.is_empty() && game.is_live(id) {
            game.invoke_action(id, &tick)?;
        }
    }
    Ok(())
}

fn saved_actor_id(reader: &SaveReader) -> Result<SavedActorId, Q1Error> {
    Ok(SavedActorId {
        slot: u32::try_from(reader.field("slot").integer(0)?).unwrap_or(u32::MAX),
        generation: u32::try_from(reader.field("generation").integer(0)?).unwrap_or(u32::MAX),
    })
}

/// Actor-release cleanup for addon words, references, and frame ticks.
struct Q1AddonRelease;

impl Q1ReleaseHook for Q1AddonRelease {
    fn on_release(&mut self, game: &mut Q1EntityServices, actor: &qa_core::identity::OwnedActor) {
        let id = actor.id().clone();
        let _ = update_addons(game, |state| {
            state.player_words.remove(&id);
            state.player_references.remove(&id);
            state.frame_ticks.retain(|tick| tick != &id);
        });
    }
}

/// Checkpoint extension for addon words, references, and frame ticks.
struct Q1AddonStateExtension {
    id: String,
}

impl Q1StateExtension for Q1AddonStateExtension {
    fn id(&self) -> &str {
        &self.id
    }

    fn capture(&self, game: &Q1EntityServices) -> Vec<u8> {
        let (players, references, ticks) = update_addons(game, |state| {
            let players = state
                .player_words
                .iter()
                .map(|(actor, words)| {
                    obj(vec![
                        ("slot", int(i64::from(actor.slot()))),
                        ("generation", int(i64::from(actor.generation()))),
                        (
                            "words",
                            arr(words
                                .iter()
                                .map(|(name, value)| arr(vec![save_str(name), num(*value)]))
                                .collect()),
                        ),
                    ])
                })
                .collect();
            let references = state
                .player_references
                .iter()
                .map(|(actor, values)| {
                    obj(vec![
                        ("slot", int(i64::from(actor.slot()))),
                        ("generation", int(i64::from(actor.generation()))),
                        (
                            "values",
                            arr(values
                                .iter()
                                .map(|(name, target)| {
                                    obj(vec![
                                        ("name", save_str(name)),
                                        (
                                            "target",
                                            match target {
                                                Some(target) => obj(vec![
                                                    ("slot", int(i64::from(target.slot()))),
                                                    ("generation", int(i64::from(target.generation()))),
                                                ]),
                                                None => SaveJson::Null,
                                            },
                                        ),
                                    ])
                                })
                                .collect()),
                        ),
                    ])
                })
                .collect();
            let ticks = state
                .frame_ticks
                .iter()
                .map(|actor| {
                    obj(vec![
                        ("slot", int(i64::from(actor.slot()))),
                        ("generation", int(i64::from(actor.generation()))),
                    ])
                })
                .collect();
            (players, references, ticks)
        })
        .unwrap_or_default();
        encode_checkpoint_value(&obj(vec![
            ("players", arr(players)),
            ("references", arr(references)),
            ("ticks", arr(ticks)),
        ]))
    }

    fn restore(&mut self, game: &mut Q1EntityServices, bytes: &[u8]) -> Result<(), Q1Error> {
        let saved = decode_checkpoint_value(bytes)?;
        let root = SaveReader::new(&saved);
        let mut players: HashMap<ActorId, HashMap<String, f64>> = HashMap::new();
        for (actor, words) in root.field("players").list(|reader| {
            let saved = saved_actor_id(&reader)?;
            let actor = game
                .host
                .actors
                .resolve_saved(&saved)
                .ok_or_else(|| Q1Error::from(reader.fail("missing addon player")))?;
            let pairs: Vec<SaveReader> = reader.field("words").list(Ok::<_, Q1Error>)?;
            let mut words = HashMap::new();
            for pair in pairs {
                let values: Vec<SaveReader> = pair.list(Ok::<_, Q1Error>)?;
                if values.len() != 2 {
                    return Err(Q1Error::from(pair.fail("invalid addon player word")));
                }
                words.insert(values[0].string()?, values[1].number()?);
            }
            Ok::<_, Q1Error>((actor.id().clone(), words))
        })? {
            players.insert(actor, words);
        }
        let mut references: HashMap<ActorId, HashMap<String, Option<ActorId>>> = HashMap::new();
        for (actor, values) in root.field("references").list(|reader| {
            let saved = saved_actor_id(&reader)?;
            let actor = game
                .host
                .actors
                .resolve_saved(&saved)
                .ok_or_else(|| Q1Error::from(reader.fail("missing addon player references")))?;
            let values: Vec<(String, Option<ActorId>)> = reader.field("values").list(|value| {
                let name = value.field("name").string()?;
                let target = value
                    .field("target")
                    .nullable(|target| Ok::<_, Q1Error>(game.host.actors.reference_saved(&saved_actor_id(&target)?)))?;
                Ok::<_, Q1Error>((name, target))
            })?;
            Ok::<_, Q1Error>((actor.id().clone(), values.into_iter().collect::<HashMap<_, _>>()))
        })? {
            references.insert(actor, values);
        }
        let mut ticks = Vec::new();
        for id in root
            .field("ticks")
            .list(|reader| {
                let saved = saved_actor_id(&reader)?;
                let actor = game.host.actors.resolve_saved(&saved);
                let entity = actor.as_ref().and_then(|actor| game.entity(actor.id()));
                if entity.is_none() {
                    return Err(Q1Error::from(reader.fail("missing addon frame tick actor")));
                }
                Ok::<_, Q1Error>(actor.map(|actor| actor.id().clone()))
            })?
            .into_iter()
            .flatten()
        {
            ticks.push(id);
        }
        update_addons(game, |state| {
            state.player_words = players;
            state.player_references = references;
            state.frame_ticks = ticks;
        })
    }
}

/// Register the addon context on a game. Registration overwrites any
/// previous entry so a reused game address never observes stale words.
pub fn register_addon_context(
    game: &mut Q1EntityServices,
    program: Q1AddonProgram,
    services: Box<dyn Q1AddonServices>,
) -> Result<Q1AddonContext, Q1Error> {
    lock_registry().insert(
        addon_key(game),
        Q1AddonState {
            program,
            services,
            frame_time: 0.0,
            player_words: HashMap::new(),
            player_references: HashMap::new(),
            frame_ticks: Vec::new(),
        },
    );
    game.register_state_extension(Box::new(Q1AddonStateExtension {
        id: format!("q1:{}:addons", program.as_str()),
    }))?;
    game.register_release_hook(Rc::new(RefCell::new(Q1AddonRelease)));
    Ok(Q1AddonContext::new(program))
}

/// Read a live entity record or fail with the donor message.
pub(crate) fn require_entity<'g>(game: &'g Q1EntityServices, id: &ActorId) -> Result<&'g Q1Actor, Q1Error> {
    game.entity_ref(id).ok_or_else(|| q1_error("Missing Q1 entity"))
}

/// Test engine services recording addon events and variables.
#[cfg(test)]
#[derive(Debug, Default)]
pub(crate) struct TestAddonServices {
    /// Presented events.
    pub events: Vec<Q1AddonEvent>,
    /// Engine variables.
    pub cvars: HashMap<String, f64>,
    /// Actors reported as monsters.
    pub monsters: Vec<ActorId>,
    /// Cheat arsenal grants.
    pub arsenal: bool,
}

#[cfg(test)]
impl Q1AddonServices for TestAddonServices {
    fn cheat_arsenal(&mut self, _actor: &ActorId, _category: Q1AddonCheatCategory) -> bool {
        self.arsenal
    }

    fn emit(&mut self, event: Q1AddonEvent) {
        self.events.push(event);
    }

    fn is_monster(&mut self, actor: &ActorId) -> bool {
        self.monsters.contains(actor)
    }

    fn cvar(&mut self, name: &str) -> f64 {
        self.cvars.get(name).copied().unwrap_or(0.0)
    }

    fn set_cvar(&mut self, name: &str, value: &str) {
        self.cvars.insert(name.to_string(), value.parse::<f64>().unwrap_or(0.0));
    }

    fn test_events(&self) -> Vec<Q1AddonEvent> {
        self.events.clone()
    }
}

/// Recorded addon events for tests.
#[cfg(test)]
pub(crate) fn test_addon_events(game: &Q1EntityServices) -> Result<Vec<Q1AddonEvent>, Q1Error> {
    update_addons(game, |state| state.services.test_events())
}

/// Register addon context with recording test services.
#[cfg(test)]
pub(crate) fn register_test_addons(game: &mut Q1EntityServices, program: Q1AddonProgram) -> Q1AddonContext {
    register_addon_context(game, program, Box::new(TestAddonServices::default())).expect("register test addons")
}

/// Admit a healthy, damageable test player. Callers register base
/// content first.
#[cfg(test)]
pub(crate) fn attach_test_player(game: &mut Q1EntityServices) -> ActorId {
    use crate::q1::foundation::entity_services::Q1AttachOptions;

    let player = game.create("player", None, None).expect("test player");
    let owned = game
        .entity_ref(&player)
        .map(|entity| entity.actor.clone())
        .expect("owned");
    game.attach_player(&owned, &Q1AttachOptions::default()).expect("attach");
    game.set_health(&player, 100.0).expect("health");
    game.set_damageable(&player, true).expect("damageable");
    player
}

#[cfg(test)]
mod tests {
    use qa_core::math::Vec3;

    use super::*;
    use crate::q1::base::provider::{Q1BaseGuard, Q1BaseOptions};
    use crate::q1::foundation::callbacks::Q1CallbackHandlers;
    use crate::q1::missionpacks::types::test_game;

    #[test]
    fn program_ids_match_donor() {
        assert_eq!(Q1AddonProgram::Dopa.as_str(), "dopa");
        assert_eq!(Q1AddonProgram::Mg1.as_str(), "mg1");
        assert_eq!(Q1AddonProgram::Mg3.as_str(), "mg3");
        assert_eq!(Q1AddonProgram::Ctf.as_str(), "ctf");
        assert_eq!(Q1AddonProgram::parse("mg3"), Ok(Q1AddonProgram::Mg3));
        assert!(Q1AddonProgram::parse("rogue").is_err());
        assert_eq!(Q1AddonLightningStyle::parse(3), Ok(Q1AddonLightningStyle::Style3));
        assert_eq!(Q1AddonLightningStyle::Style1.as_i32(), 1);
        assert!(Q1AddonLightningStyle::parse(4).is_err());
        assert_eq!(Q1AddonCheatCategory::Ammo.as_str(), "ammo");
    }

    #[test]
    fn player_words_default_and_round_trip() {
        let mut game = test_game();
        let context = register_test_addons(&mut game, Q1AddonProgram::Mg1);
        assert_eq!(context.program(), Q1AddonProgram::Mg1);
        assert!(addons_registered(&game));
        let player = game.create("player", None, None).expect("player");
        assert_eq!(addon_player_number(&game, &player, "hunger_time"), Ok(0.0));
        assert_eq!(addon_player_word(&game, &player, "hunger_time"), Ok(None));
        set_addon_player_number(&mut game, &player, "hunger_time", 10.5).expect("set");
        assert_eq!(addon_player_number(&game, &player, "hunger_time"), Ok(10.5));
        let missing = game.create("info_null", None, None).expect("missing");
        let owned = game.host.actors.resolve_owned(&missing).expect("owned");
        game.release_actor(&owned).expect("release");
        assert_eq!(addon_player_word(&game, &missing, "hunger_time"), Ok(None));
        assert!(set_addon_player_number(&mut game, &missing, "hunger_time", 1.0).is_err());
    }

    #[test]
    fn player_references_round_trip() {
        let mut game = test_game();
        register_test_addons(&mut game, Q1AddonProgram::Mg3);
        let player = game.create("player", None, None).expect("player");
        let target = game.create("info_null", None, None).expect("target");
        assert_eq!(addon_player_reference(&game, &player, "rope").expect("read"), None);
        set_addon_player_reference(&mut game, &player, "rope", Some(&target)).expect("set");
        assert_eq!(
            addon_player_reference(&game, &player, "rope").expect("read"),
            Some(target)
        );
        set_addon_player_reference(&mut game, &player, "rope", None).expect("clear");
        assert_eq!(addon_player_reference(&game, &player, "rope").expect("read"), None);
    }

    #[test]
    fn entity_numbers_vectors_and_alpha_match_donor() {
        let mut game = test_game();
        register_test_addons(&mut game, Q1AddonProgram::Dopa);
        let id = game.create("info_null", None, None).expect("entity");
        set_addon_number(&mut game, &id, "style", 1.0).expect("number");
        assert_eq!(require_entity(&game, &id).expect("entity").number("style"), 1.0);
        set_addon_vector(&mut game, &id, "oldorigin", Vec3 { x: 1.0, y: 2.0, z: 3.0 }).expect("vector");
        assert_eq!(
            require_entity(&game, &id).expect("entity").vector("oldorigin"),
            Vec3 { x: 1.0, y: 2.0, z: 3.0 }
        );
        addon_alpha(&mut game, &id, 0.2).expect("alpha");
        assert_eq!(require_entity(&game, &id).expect("entity").number("alpha"), fround(0.2));
        assert_eq!(
            test_addon_events(&game).expect("events"),
            vec![Q1AddonEvent::Alpha {
                actor: id.clone(),
                alpha: 0.2
            }]
        );
    }

    #[test]
    fn services_round_trip_through_registry() {
        let mut game = test_game();
        register_addon_context(
            &mut game,
            Q1AddonProgram::Mg1,
            Box::new(TestAddonServices {
                cvars: HashMap::from([(String::from("horde"), 1.0)]),
                ..TestAddonServices::default()
            }),
        )
        .expect("register");
        assert_eq!(addon_cvar(&game, "horde"), Ok(1.0));
        addon_set_cvar(&game, "horde", "0").expect("set");
        assert_eq!(addon_cvar(&game, "horde"), Ok(0.0));
        let player = game.create("player", None, None).expect("player");
        assert_eq!(addon_is_monster(&game, &player), Ok(false));
        assert_eq!(
            addon_cheat_arsenal(&game, &player, Q1AddonCheatCategory::Weapons),
            Ok(false)
        );
        addon_emit(
            &game,
            Q1AddonEvent::DeveloperMessage {
                text: String::from("hello"),
            },
        )
        .expect("emit");
        update_addons(&game, |state| {
            assert_eq!(state.frame_time, 0.0);
        })
        .expect("read");
    }

    #[test]
    fn init_trigger_sets_trigger_volume() {
        let mut game = test_game();
        register_test_addons(&mut game, Q1AddonProgram::Mg1);
        let id = game.create("trigger_multiple", None, None).expect("trigger");
        init_trigger(&mut game, &id).expect("init");
        let entity = require_entity(&game, &id).expect("entity");
        assert_eq!(entity.solid, Q1Solid::Trigger);
        assert_eq!(entity.movement, Q1MoveType::None);
        assert_eq!(entity.model, "");
        assert_eq!(game.body(&id).expect("body").angles, ZERO);
    }

    #[test]
    fn coop_and_rune_inhibition_match_donor() {
        let mut game = test_game();
        register_test_addons(&mut game, Q1AddonProgram::Mg1);
        let inhibited = game.create("trigger_multiple", None, None).expect("trigger");
        game.update_entity(&inhibited, |entity| entity.spawnflags = 32768)
            .expect("flags");
        assert_eq!(removed_outside_coop(&mut game, &inhibited, true), Ok(true));
        assert!(game.entity_ref(&inhibited).is_none());
        let kept = game.create("trigger_multiple", None, None).expect("kept");
        assert_eq!(removed_for_runes(&mut game, &kept), Ok(false));

        let mut game = test_game();
        register_test_addons(&mut game, Q1AddonProgram::Mg3);
        let _guard = Q1BaseGuard::register(&mut game, Q1BaseOptions::default()).expect("base");
        let gated = game.create("trigger_multiple", None, None).expect("gated");
        game.update_entity(&gated, |entity| entity.spawnflags = 262144)
            .expect("flags");
        assert_eq!(removed_for_runes(&mut game, &gated), Ok(true));
        assert!(game.entity_ref(&gated).is_none());
    }

    fn tick_action(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
        game.update_entity(id, |entity| {
            entity.fields.insert(String::from("ticked"), String::from("1"));
        })
    }

    #[test]
    fn frame_ticks_dispatch_named_actions() {
        let mut game = test_game();
        register_test_addons(&mut game, Q1AddonProgram::Mg3);
        game.named
            .register(
                "test:tick",
                Q1CallbackHandlers {
                    action: Some(tick_action),
                    ..Default::default()
                },
            )
            .expect("register");
        let id = game.create("misc_rope", None, None).expect("rope");
        add_frame_tick(&mut game, &id, "test:tick").expect("add");
        frame_addons(&mut game, 1.5).expect("frame");
        assert_eq!(addon_frame_time(&game), Ok(1.5));
        assert_eq!(require_entity(&game, &id).expect("entity").text("ticked"), "1");
        remove_frame_tick(&game, &id).expect("remove");
        game.update_entity(&id, |entity| {
            entity.fields.insert(String::from("ticked"), String::from("0"));
        })
        .expect("clear");
        frame_addons(&mut game, 2.5).expect("frame");
        assert_eq!(require_entity(&game, &id).expect("entity").text("ticked"), "0");
    }

    #[test]
    fn release_clears_player_words() {
        let mut game = test_game();
        register_test_addons(&mut game, Q1AddonProgram::Mg1);
        let player = game.create("player", None, None).expect("player");
        set_addon_player_number(&mut game, &player, "hunger_time", 3.0).expect("set");
        let owned = game.host.actors.resolve_owned(&player).expect("owned");
        game.release_actor(&owned).expect("release");
        assert_eq!(addon_player_word(&game, &player, "hunger_time"), Ok(None));
    }

    #[test]
    fn checkpoint_round_trips_words_references_and_ticks() {
        let mut game = test_game();
        register_test_addons(&mut game, Q1AddonProgram::Mg1);
        game.named
            .register(
                "test:tick",
                Q1CallbackHandlers {
                    action: Some(tick_action),
                    ..Default::default()
                },
            )
            .expect("register");
        let player = game.create("player", None, None).expect("player");
        let target = game.create("info_null", None, None).expect("target");
        set_addon_player_number(&mut game, &player, "hunger_time", 7.0).expect("word");
        set_addon_player_reference(&mut game, &player, "rope", Some(&target)).expect("reference");
        add_frame_tick(&mut game, &player, "test:tick").expect("tick");
        let mut extension = game.state_extensions.remove("q1:mg1:addons").expect("extension");
        let bytes = extension.capture(&game);
        set_addon_player_number(&mut game, &player, "hunger_time", 1.0).expect("overwrite");
        extension.restore(&mut game, &bytes).expect("restore");
        game.state_extensions.insert(String::from("q1:mg1:addons"), extension);
        assert_eq!(addon_player_number(&game, &player, "hunger_time"), Ok(7.0));
        assert_eq!(
            addon_player_reference(&game, &player, "rope").expect("reference"),
            Some(target)
        );
        frame_addons(&mut game, 3.0).expect("frame");
        assert_eq!(require_entity(&game, &player).expect("entity").text("ticked"), "1");
    }
}
