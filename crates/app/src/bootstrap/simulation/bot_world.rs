//! Shared bot observation world.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/simulation/bot-world.ts`
//! (`createSharedBotWorld`).
//!
//! A sensory projection and ID lookup over existing shared actors, with no
//! entity or physics ownership. The `qa-bots` [`SourceBotGame`] contract is a
//! refactored (detached) form of the donor game object: entity/player cells
//! project onto [`BotObservedEntity`]/[`BotObservedPlayer`] instead of
//! exposing `EntityState`/`PlayerState`, so view angles, velocity, armor,
//! and stat-schema cells from the donor `entity()` builder have no wire
//! target and are intentionally not carried. Engine, configstring, and memory
//! surfaces that the trait does not expose stay as inherent methods for the
//! `bots.ts` consumer.
//!
//! # Missing siblings
//!
//! - `simulation/runtime.ts` (`SharedSimulation`): [`BotWorldSimulation`],
//!   [`BotWorldScene`].
//! - Q1/Q2 source games (`simulation.q1Source()/q2Source()`,
//!   `q2WeaponSource()`): [`BotWorldQ1Source`], [`BotWorldQ2Source`],
//!   [`BotWorldSimulation::q2_arsenal_entity`]. The concrete
//!   `observeQ1Supply`/`previewQ1Supply` calls stay with the sibling that
//!   owns the game object; the seam surfaces their contract results.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use qa_bots::behavior::library::character::BotCharacterLibrary;
use qa_bots::behavior::library::genetic::BotRandom;
use qa_bots::behavior::library::weapons::WeaponAi;
use qa_bots::behavior::q3::ai_definitions::BotInventory;
use qa_bots::behavior::q3::ai_state::BotState;
use qa_bots::behavior::q3::game_host::{
    BotArsenalKnowledge, BotGameClock, BotObservedEntity, BotObservedPickup, BotObservedPlayer, BotProduct,
    BotTraceQuery, BotTraceResult, BotWeaponTactics, PickupAvailability, SourceBotGame,
};
use qa_content::contract::PickupSupplyObservation;
use qa_content::contract::{PickupAvailability as SupplyAvailability, PickupSupplyOffer, PickupSupplyPreview};
use qa_content::q3::base::game::memory::{GameMemory, GameMemorySave};
use qa_content::q3::base::shared::definitions::EntityType;
use qa_core::cvar::CvarRegistry;
use qa_core::identity::{ActorId, SavedActorId};
use qa_core::math::{Bounds, Vec3};
use qa_world::save::records::{read_saved_actor, write_saved_actor};
use qa_world::save::value::{arr, int, obj, str, SaveJson, SaveReader};
use qa_world::WorldError;
use thiserror::Error;

use super::bot_arsenal::{create_bot_arsenal_binding, BotArsenalBinding};
use super::bot_q1_knowledge::Q1BotKnowledgeSimulation;
use super::bot_q2_knowledge::Q2BotKnowledgeSimulation;
use super::bot_q3_knowledge::Q3BotKnowledgeSimulation;

/// First dynamic observation entity id.
const BOT_WORLD_FIRST_DYNAMIC: i32 = 64;
/// World observation entity id.
const BOT_WORLD_WORLD_ENTITY: i32 = 1022;
/// Absent-entity observation id.
const BOT_WORLD_ABSENT_ENTITY: i32 = 1023;
/// Player configstring base.
const BOT_WORLD_PLAYER_INFO_BASE: i32 = 544;
/// Player configstring count.
const BOT_WORLD_PLAYER_INFO_COUNT: i32 = 64;
/// Client info spectator team.
const BOT_WORLD_SPECTATOR_TEAM: i32 = 3;
/// Player contents mask.
const BOT_WORLD_PLAYER_CONTENTS: i32 = 0x2000000;
/// Brush contents mask.
const BOT_WORLD_BRUSH_CONTENTS: i32 = 1;

/// Shared world failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BotWorldError {
    /// No admitted Q1 or Q2 world is bound.
    #[error("Shared bot observations require an admitted Q1 or Q2 world")]
    MissingSource,
    /// No source numeric policy is bound.
    #[error("Bot world has no source numeric policy")]
    MissingNumericPolicy,
    /// Arsenal binding is unavailable.
    #[error("Shared bot arsenal binding is unavailable")]
    MissingArsenalBinding,
    /// Live observations exceed entity capacity.
    #[error("Live bot observations exceed the current simultaneous entity capacity")]
    EntityCapacity,
    /// Scene query did not use the Q3 decision representation.
    #[error("Bot {0} did not use the Q3 decision representation")]
    Representation(&'static str),
    /// Arsenal failure.
    #[error("Shared bot arsenal: {0}")]
    Arsenal(String),
    /// Cvar failure.
    #[error("Shared bot cvar: {0}")]
    Cvar(String),
    /// Memory restore failure.
    #[error("Shared bot memory: {0}")]
    Memory(String),
    /// Save failure.
    #[error("Shared bot save: {0}")]
    Save(String),
}

impl From<WorldError> for BotWorldError {
    fn from(value: WorldError) -> Self {
        Self::Save(format!("{value:?}"))
    }
}

/// Q1 client view (donor `source.composition.clients.get(actor)` surface).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BotWorldQ1Client {
    /// Client name.
    pub name: String,
    /// Observer flag.
    pub observer: bool,
    /// Frag count.
    pub frags: i32,
}

/// Q1 entity view (donor `source.game.entity(actor)` surface).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BotWorldQ1Entity {
    /// Model path.
    pub model: String,
    /// Model frame.
    pub frame: i32,
    /// Classname.
    pub classname: String,
    /// Entity max health.
    pub max_health: i32,
}

/// Q1 powerup channel (donor `powerupExpires(actor, ...)` argument).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BotWorldQ1Powerup {
    /// Quad damage.
    Quad,
    /// Environment suit.
    Suit,
}

/// Q1 intermission view (donor `source.game.intermission` surface).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BotWorldQ1Intermission {
    /// Exit time in source seconds.
    pub exit_after: f64,
}

/// Q1 source seam (donor `simulation.q1Source()` used surface).
pub trait BotWorldQ1Source: std::fmt::Debug {
    /// Donor `source.game.mapName`.
    fn map_name(&self) -> String;
    /// Donor `source.composition.clients.get(actor)`.
    fn client(&self, actor: &ActorId) -> Option<BotWorldQ1Client>;
    /// Donor `source.game.entity(actor)`.
    fn entity(&self, actor: &ActorId) -> Option<BotWorldQ1Entity>;
    /// Donor `source.game.player(actor)?.maxHealth`.
    fn player_max_health(&self, actor: &ActorId) -> Option<i32>;
    /// Donor `source.game.powerupExpires(actor, channel)`.
    fn powerup_expires(&self, actor: &ActorId, channel: BotWorldQ1Powerup) -> f64;
    /// Donor `source.game.time`.
    fn time(&self) -> f64;
    /// Donor `source.game.intermission`.
    fn intermission(&self) -> Option<BotWorldQ1Intermission>;
    /// Donor `source.game.world?.actor.id`.
    fn world_actor(&self) -> Option<ActorId>;
    /// Donor `source.game.host.random()`.
    fn random(&self) -> f64;
    /// Donor `source.game.requestIntermissionExit(now, silent)`.
    fn request_intermission_exit(&self, now_seconds: f64, silent: bool);
    /// Donor `source.composition.clients.update(actor, info)`.
    fn update_client(&self, actor: &ActorId, info: &HashMap<String, String>);
    /// Donor `observeQ1Supply(source.game, actor, recipient)`.
    fn observe_supply(&self, actor: &ActorId, recipient: &ActorId) -> Option<PickupSupplyObservation>;
    /// Donor `previewQ1Supply(source.game, actor, recipient)`.
    fn preview_supply(&self, actor: &ActorId, recipient: &ActorId) -> Option<PickupSupplyPreview>;
}

/// Q2 player view (donor `source.players.states.get(actor)` surface).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BotWorldQ2Player {
    /// Player name.
    pub name: String,
    /// Spectator flag.
    pub spectator: bool,
    /// Score.
    pub score: i32,
    /// Skin path.
    pub skin: String,
}

/// Q2 entity view (donor `source.game.entity(actor)` surface).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BotWorldQ2Entity {
    /// Model path.
    pub model: String,
    /// Model frame.
    pub frame: i32,
    /// Classname.
    pub classname: String,
    /// Server flags.
    pub server_flags: i32,
    /// Entity max health.
    pub max_health: i32,
}

/// Q2 powerup expiry view (donor `source.items.playerPowerups(actor)` surface).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BotWorldQ2Powerups {
    /// Quad expiry in source seconds.
    pub quad_until: f64,
    /// Breather expiry in source seconds.
    pub breather_until: f64,
    /// Enviro-suit expiry in source seconds.
    pub enviro_until: f64,
}

/// Q2 intermission view (donor `source.players.intermission` surface).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BotWorldQ2Intermission {
    /// Whether the match is still playing.
    pub playing: bool,
    /// Intermission start in source seconds.
    pub started: f64,
}

/// Q2 client connection outcome (donor `players.connect(...)` surface).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BotWorldQ2Connect {
    /// Whether the connection is allowed.
    pub allowed: bool,
    /// Deny reason.
    pub reason: String,
    /// Canonical userinfo.
    pub userinfo: String,
}

/// Q2 source seam (donor `simulation.q2Source()` used surface).
pub trait BotWorldQ2Source: std::fmt::Debug {
    /// Donor `source.game.options.mapName`.
    fn map_name(&self) -> String;
    /// Donor `source.players.states.get(actor)`.
    fn player(&self, actor: &ActorId) -> Option<BotWorldQ2Player>;
    /// Donor `source.game.entity(actor)`.
    fn entity(&self, actor: &ActorId) -> Option<BotWorldQ2Entity>;
    /// Donor `source.players.intermission`.
    fn intermission(&self) -> BotWorldQ2Intermission;
    /// Donor `source.game.host.random()`.
    fn host_random(&self) -> f64;
    /// Donor `source.game.host.now()`.
    fn host_now(&self) -> f64;
    /// Donor `source.game.host.worldActor()`.
    fn world_actor(&self) -> Option<ActorId>;
    /// Donor `source.items.playerPowerups(actor)`.
    fn player_powerups(&self, actor: &ActorId) -> Option<BotWorldQ2Powerups>;
    /// Donor `source.items.observeSupply(source.game, actor, recipient)`.
    fn observe_supply(&self, actor: &ActorId, recipient: &ActorId) -> Option<PickupSupplyObservation>;
    /// Donor `source.items.previewSupply(source.game, actor, recipient)`.
    fn preview_supply(&self, actor: &ActorId, recipient: &ActorId) -> Option<PickupSupplyPreview>;
    /// Donor `source.product.rerelease !== null`.
    fn use_rerelease_players(&self) -> bool;
    /// Donor classic `source.players.connect(game, userinfo)`.
    fn connect_classic(&self, userinfo: &str) -> BotWorldQ2Connect;
    /// Donor rerelease `players.connect(game, userinfo, bot)`.
    fn connect_rerelease(&self, userinfo: &str, bot: bool) -> BotWorldQ2Connect;
    /// Donor `source.players.userinfoChanged(entity, game, userinfo)`.
    fn userinfo_changed(&self, actor: &ActorId, userinfo: &str);
    /// Donor `source.players.endDeathmatchLevel(game)`.
    fn end_deathmatch_level(&self);
    /// Donor `entity.serverFlags |= flag`.
    fn set_server_flag(&self, actor: &ActorId, flag: i32);
}

/// Body view (donor `simulation.bodies.read(actor)` used surface).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BotWorldBody {
    /// Body origin.
    pub origin: Vec3,
    /// Body angles.
    pub angles: Vec3,
    /// Body bounds.
    pub bounds: Bounds,
}

/// Scene contents answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BotWorldContents {
    /// Q3 decision representation with contents.
    Q3(i32),
    /// Any other representation.
    Other,
}

/// Scene trace hit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BotWorldTraceHit {
    /// Hit an actor.
    Actor(ActorId),
    /// Hit the world.
    World,
    /// Hit nothing.
    None,
}

/// Scene trace answer.
#[derive(Debug, Clone, PartialEq)]
pub enum BotWorldTrace {
    /// Q3 decision representation.
    Q3 {
        /// Fraction reached.
        fraction: f32,
        /// End position.
        end: Vec3,
        /// Hit entity.
        hit: BotWorldTraceHit,
    },
    /// Any other representation.
    Other,
}

/// Collision scene seam (donor `simulation.scene` used surface).
pub trait BotWorldScene: std::fmt::Debug {
    /// Donor `scene.pointContents({ point, target: world, ... })`.
    fn point_contents(&self, point: Vec3) -> BotWorldContents;
    /// Donor `scene.trace({ start, end, shape, mask, passActor, ... })`.
    fn trace(
        &self,
        start: Vec3,
        end: Vec3,
        bounds: Option<Bounds>,
        mask: i32,
        pass_actor: Option<ActorId>,
    ) -> BotWorldTrace;
}

/// Shared simulation seam (donor `SharedSimulation` used surface).
pub trait BotWorldSimulation:
    Q1BotKnowledgeSimulation + Q2BotKnowledgeSimulation + Q3BotKnowledgeSimulation + Clone + 'static
{
    /// Donor `simulation.q1Source()`.
    fn q1_bot_source(&self) -> Option<Rc<dyn BotWorldQ1Source>>;
    /// Donor `simulation.q2Source()`.
    fn q2_bot_source(&self) -> Option<Rc<dyn BotWorldQ2Source>>;
    /// Donor `simulation.q2WeaponSource()?.game.entity(actor)`.
    fn q2_arsenal_entity(&self, actor: &ActorId) -> Option<BotWorldQ2Entity>;
    /// Donor `simulation.options.maxClients`.
    fn max_clients(&self) -> i32;
    /// Donor `simulation.options.world.models.length`.
    fn world_model_count(&self) -> usize;
    /// Donor `simulation.recipe.character.appearance.provider`.
    fn character_appearance_provider(&self) -> String;
    /// Donor `simulation.physics.gravity`.
    fn physics_gravity(&self) -> f64;
    /// Donor `simulation.players()`.
    fn players(&self) -> Vec<ActorId>;
    /// Donor `simulation.movementPlayer(actor)?.client.slot`.
    fn movement_client_slot(&self, actor: &ActorId) -> Option<i32>;
    /// Donor `simulation.actors.isLive(actor)`.
    fn is_live(&self, actor: &ActorId) -> bool;
    /// Donor `simulation.actors.observations()` actor ids.
    fn observations(&self) -> Vec<ActorId>;
    /// Donor `simulation.bodies.read(actor)`.
    fn body(&self, actor: &ActorId) -> Option<BotWorldBody>;
    /// Donor `simulation.bodies.linked(actor) !== null`.
    fn body_linked(&self, actor: &ActorId) -> bool;
    /// Donor `simulation.combat.read(actor)?.health`.
    fn combat_health(&self, actor: &ActorId) -> Option<i32>;
    /// Donor `simulation.physics.solidOf(actor)?.solid === "brush"`.
    fn is_brush_solid(&self, actor: &ActorId) -> bool;
    /// Donor `simulation.timeSeconds`.
    fn time_seconds(&self) -> f64;
    /// Donor `simulation.scene`; `None` maps to the missing numeric policy.
    fn bot_scene(&self) -> Option<Rc<dyn BotWorldScene>>;
}

/// Selected source kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BotWorldSourceKind {
    /// Quake II source.
    Q2,
    /// Quake source.
    Q1,
}

/// Client info projection (donor `clientInfo` surface).
#[derive(Debug, Clone, PartialEq, Eq)]
struct BotWorldClientInfo {
    name: String,
    spectator: bool,
    score: i32,
    skin: String,
}

/// Entity metadata projection (donor `metadata` surface).
#[derive(Debug, Clone, PartialEq, Eq)]
struct BotWorldMetadata {
    model: String,
    frame: i32,
    classname: String,
    hidden: bool,
    max_health: i32,
}

/// Bot actor lookup (donor `actor(client)`).
pub type BotWorldActorFn = Rc<dyn Fn(i32) -> Option<ActorId>>;
/// Client admission (donor `connect(client, restart)`).
pub type BotWorldConnectFn = Rc<dyn Fn(i32, bool) -> bool>;
/// Client drop (donor `drop(client)`).
pub type BotWorldDropFn = Rc<dyn Fn(i32)>;
/// Client begin (donor `begin(client)`).
pub type BotWorldBeginFn = Rc<dyn Fn(i32)>;
/// Print sink (donor `print(text)`).
pub type BotWorldPrintFn = Rc<dyn Fn(&str)>;
/// Console sink (donor `console(text)`).
pub type BotWorldConsoleFn = Rc<dyn Fn(&str)>;
/// Client message sink (donor `message(client, text)`).
pub type BotWorldMessageFn = Rc<dyn Fn(i32, &str)>;

/// Shared world options (donor `Options`).
#[derive(Clone)]
pub struct SharedBotWorldOptions<S> {
    /// Restore mode; skips cvar registration and refresh.
    pub restoring: bool,
    /// Shared simulation.
    pub simulation: S,
    /// Shared cvar registry.
    pub cvars: Rc<RefCell<CvarRegistry>>,
    /// Bot actor for a client number (donor `actor(client)`).
    pub actor: BotWorldActorFn,
    /// Admit a client (donor `connect(client, restart)`).
    pub connect: BotWorldConnectFn,
    /// Drop a client (donor `drop(client)`).
    pub drop_client: BotWorldDropFn,
    /// Begin a client (donor `begin(client)`).
    pub begin: BotWorldBeginFn,
    /// Print sink (donor `print(text)`).
    pub print: BotWorldPrintFn,
    /// Console sink (donor `console(text)`).
    pub console: BotWorldConsoleFn,
    /// Client message sink (donor `message(client, text)`).
    pub message: BotWorldMessageFn,
}

/// Interior shared state behind the game and its knowledge wrapper.
struct SharedBotState<S> {
    simulation: S,
    q1: Option<Rc<dyn BotWorldQ1Source>>,
    q2: Option<Rc<dyn BotWorldQ2Source>>,
    kind: BotWorldSourceKind,
    scene: Rc<dyn BotWorldScene>,
    binding: Option<Box<dyn BotArsenalBinding>>,
    actor_for_client: BotWorldActorFn,
    connect: BotWorldConnectFn,
    drop_client: BotWorldDropFn,
    begin: BotWorldBeginFn,
    print: BotWorldPrintFn,
    console: BotWorldConsoleFn,
    message: BotWorldMessageFn,
    ids: HashMap<ActorId, i32>,
    actors: HashMap<i32, ActorId>,
    userinfos: HashMap<i32, String>,
    strings: HashMap<i32, String>,
    models: HashMap<String, i32>,
    next_entity: i32,
    next_generation: i32,
    free_ids: Vec<i32>,
    generations: HashMap<i32, i32>,
    begun: HashSet<i32>,
}

impl<S: BotWorldSimulation> SharedBotState<S> {
    fn is_q2(&self) -> bool {
        self.kind == BotWorldSourceKind::Q2
    }

    fn client_info(&self, actor: Option<&ActorId>) -> Option<BotWorldClientInfo> {
        let actor = actor?;
        if self.is_q2() {
            let player = self.q2.as_ref()?.player(actor)?;
            return Some(BotWorldClientInfo {
                name: player.name,
                spectator: player.spectator,
                score: player.score,
                skin: player.skin,
            });
        }
        let player = self.q1.as_ref()?.client(actor)?;
        Some(BotWorldClientInfo {
            name: player.name,
            spectator: player.observer,
            score: player.frags,
            skin: self.simulation.character_appearance_provider(),
        })
    }

    fn metadata(&self, actor: Option<&ActorId>) -> Option<BotWorldMetadata> {
        let actor = actor?;
        let q2 = if self.is_q2() {
            self.q2.as_ref()?.entity(actor)
        } else {
            self.simulation.q2_arsenal_entity(actor)
        };
        if let Some(q2) = q2 {
            return Some(BotWorldMetadata {
                model: q2.model,
                frame: q2.frame,
                classname: q2.classname,
                hidden: q2.server_flags & 1 != 0,
                max_health: q2.max_health,
            });
        }
        let q1 = if self.is_q2() {
            None
        } else {
            self.q1.as_ref()?.entity(actor)
        };
        let q1 = q1?;
        let max_health = if self.is_q2() {
            q1.max_health
        } else {
            self.q1.as_ref()?.player_max_health(actor).unwrap_or(q1.max_health)
        };
        Some(BotWorldMetadata {
            model: q1.model,
            frame: q1.frame,
            classname: q1.classname,
            hidden: false,
            max_health,
        })
    }

    fn retire(&mut self) {
        let stale: Vec<(ActorId, i32)> = self
            .ids
            .iter()
            .filter(|(actor, _)| !self.simulation.is_live(actor))
            .map(|(actor, id)| (actor.clone(), *id))
            .collect();
        for (actor, id) in stale {
            self.ids.remove(&actor);
            self.actors.remove(&id);
            self.free_ids.push(id);
        }
    }

    fn entity_id(&mut self, actor: &ActorId) -> Result<i32, BotWorldError> {
        if let Some(slot) = self.simulation.movement_client_slot(actor) {
            return Ok(slot);
        }
        if self
            .metadata(Some(actor))
            .is_some_and(|meta| meta.classname == "worldspawn")
        {
            return Ok(BOT_WORLD_WORLD_ENTITY);
        }
        if let Some(existing) = self.ids.get(actor) {
            return Ok(*existing);
        }
        self.retire();
        let id = self.free_ids.pop().unwrap_or_else(|| {
            let id = self.next_entity;
            self.next_entity += 1;
            id
        });
        if id >= BOT_WORLD_WORLD_ENTITY {
            return Err(BotWorldError::EntityCapacity);
        }
        self.ids.insert(actor.clone(), id);
        self.actors.insert(id, actor.clone());
        let generation = self.next_generation;
        self.next_generation += 1;
        self.generations.insert(id, generation);
        Ok(id)
    }

    fn refresh(&mut self) -> Result<(), BotWorldError> {
        self.retire();
        let observed = self.simulation.observations();
        for actor in observed {
            if self.simulation.body(&actor).is_some() {
                self.entity_id(&actor)?;
            }
        }
        Ok(())
    }

    fn actor_for_id(&self, number: i32) -> Option<ActorId> {
        let actor = if number < BOT_WORLD_FIRST_DYNAMIC {
            self.simulation
                .players()
                .into_iter()
                .find(|actor| self.simulation.movement_client_slot(actor) == Some(number))
        } else if number == BOT_WORLD_WORLD_ENTITY {
            if self.is_q2() {
                self.q2.as_ref()?.world_actor()
            } else {
                self.q1.as_ref()?.world_actor()
            }
        } else {
            self.actors.get(&number).cloned()
        };
        actor.filter(|actor| self.simulation.is_live(actor))
    }

    fn model_index(&mut self, name: &str) -> i32 {
        if name.is_empty() {
            return 0;
        }
        if let Some(inline) = name.strip_prefix('*') {
            return inline.parse().unwrap_or(0);
        }
        if let Some(existing) = self.models.get(name) {
            return *existing;
        }
        let index = self.simulation.world_model_count() as i32 + self.models.len() as i32;
        self.models.insert(name.to_string(), index);
        index
    }

    fn player_info(&self, client: i32) -> String {
        let actor = self.actor_for_id(client);
        let Some(player) = self.client_info(actor.as_ref()) else {
            return String::new();
        };
        format!(
            "\\\\n\\\\{name}\\\\t\\\\{spectator}\\\\model\\\\{skin}",
            name = player.name,
            spectator = if player.spectator { 3 } else { 0 },
            skin = player.skin,
        )
    }

    fn source_random(&self) -> f64 {
        if self.is_q2() {
            self.q2.as_ref().map_or(0.0, |q2| q2.host_random())
        } else {
            self.q1.as_ref().map_or(0.0, |q1| q1.random())
        }
    }

    fn observe_supply(&self, actor: &ActorId, recipient: &ActorId) -> Option<PickupSupplyObservation> {
        if self.is_q2() {
            self.q2.as_ref()?.observe_supply(actor, recipient)
        } else {
            self.q1.as_ref()?.observe_supply(actor, recipient)
        }
    }

    fn preview_supply(&self, actor: &ActorId, recipient: &ActorId) -> Option<PickupSupplyPreview> {
        if self.is_q2() {
            self.q2.as_ref()?.preview_supply(actor, recipient)
        } else {
            self.q1.as_ref()?.preview_supply(actor, recipient)
        }
    }

    fn supply_now(&self) -> f64 {
        if self.is_q2() {
            self.q2.as_ref().map_or(0.0, |q2| q2.host_now())
        } else {
            self.q1.as_ref().map_or(0.0, |q1| q1.time())
        }
    }

    fn inspect_pickup(&mut self, client: i32, actor: &ActorId) -> Option<BotObservedPickup> {
        let recipient = self.actor_for_id(client)?;
        if !self.simulation.is_live(actor) {
            return None;
        }
        let body = self.simulation.body(actor)?;
        let observation = self.observe_supply(actor, &recipient)?;
        let preview = self.preview_supply(actor, &recipient)?;
        let entity_meta = self.metadata(Some(actor));
        let (availability, respawn_delay) = match &observation.availability {
            SupplyAvailability::Ready { .. } => (PickupAvailability::Ready, 0.0),
            SupplyAvailability::Respawning { at_seconds } => (
                PickupAvailability::Waiting,
                (at_seconds - self.supply_now()).max(0.0) as f32,
            ),
            SupplyAvailability::Inactive => (PickupAvailability::Disabled, 0.0),
        };
        let name = entity_meta
            .as_ref()
            .map(|meta| meta.classname.clone())
            .unwrap_or_else(|| match &observation.offer {
                PickupSupplyOffer::Ammo(_) => "ammo".to_string(),
                PickupSupplyOffer::AmmoWeapon(_) => "ammoWeapon".to_string(),
                PickupSupplyOffer::Weapon(_) => "weapon".to_string(),
            });
        let entity = self
            .entity_id(actor)
            .expect("Live bot observations exceed the current simultaneous entity capacity");
        Some(BotObservedPickup {
            availability,
            eligible: preview.accepted,
            entity,
            origin: body.origin,
            bounds: body.bounds,
            name,
            item_index: 0,
            respawn_delay,
        })
    }
}

/// Knowledge wrapper with the shared quad/suit inventory projection.
struct BotWorldKnowledge<S> {
    state: Rc<RefCell<SharedBotState<S>>>,
}

impl<S: BotWorldSimulation> BotWorldKnowledge<S> {
    fn with_binding<T>(&self, read: impl FnOnce(&dyn BotArsenalBinding) -> T) -> T {
        let shared = self.state.borrow();
        let binding = shared
            .binding
            .as_ref()
            .expect("Shared bot arsenal binding is unavailable");
        read(binding.as_ref())
    }

    fn update_powerups(&mut self, state: &mut BotState) {
        let shared = self.state.borrow();
        let actor = shared.actor_for_id(state.client);
        if shared.is_q2() {
            let powers = actor
                .as_ref()
                .and_then(|actor| shared.q2.as_ref()?.player_powerups(actor));
            let now = shared.q2.as_ref().map_or(0.0, |q2| q2.host_now());
            let quad = i32::from(powers.is_some_and(|powers| powers.quad_until > now));
            let suit = i32::from(powers.is_some_and(|powers| powers.breather_until > now || powers.enviro_until > now));
            drop(shared);
            write_inventory(state, BotInventory::QUAD, quad);
            write_inventory(state, BotInventory::ENVIRONMENTSUIT, suit);
        } else {
            let (quad, suit) = match actor.as_ref() {
                Some(actor) => {
                    let q1 = shared.q1.as_ref();
                    let time = q1.map_or(0.0, |q1| q1.time());
                    (
                        i32::from(q1.is_some_and(|q1| q1.powerup_expires(actor, BotWorldQ1Powerup::Quad) > time)),
                        i32::from(q1.is_some_and(|q1| q1.powerup_expires(actor, BotWorldQ1Powerup::Suit) > time)),
                    )
                }
                None => (0, 0),
            };
            drop(shared);
            write_inventory(state, BotInventory::QUAD, quad);
            write_inventory(state, BotInventory::ENVIRONMENTSUIT, suit);
        }
    }
}

fn write_inventory(state: &mut BotState, index: usize, value: i32) {
    if state.inventory.len() <= index {
        state.inventory.resize(index + 1, 0);
    }
    state.inventory[index] = value;
}

impl<S: BotWorldSimulation> BotArsenalKnowledge for BotWorldKnowledge<S> {
    fn pickup_utility(&self, characters: &BotCharacterLibrary, state: &BotState, pickup: &BotObservedPickup) -> f32 {
        self.state
            .borrow()
            .binding
            .as_ref()
            .expect("Shared bot arsenal binding is unavailable")
            .pickup_utility(characters, state, pickup)
    }

    fn choose_weapon(&self, characters: &BotCharacterLibrary, state: &BotState) -> i32 {
        self.with_binding(|binding| binding.choose_weapon(characters, state))
    }

    fn activation_weapon(&self, characters: &BotCharacterLibrary, state: &BotState) -> i32 {
        self.with_binding(|binding| binding.activation_weapon(characters, state))
    }

    fn tactics(&self, weapon: i32) -> BotWeaponTactics {
        self.with_binding(|binding| binding.tactics(weapon))
    }

    fn aggression(&self, state: &BotState) -> f32 {
        self.with_binding(|binding| binding.aggression(state))
    }

    fn update_inventory(&mut self, state: &mut BotState) {
        // Take/restore: the binding calls back into `actor_for_id`, so no
        // borrow may be held across the call.
        let mut binding = self
            .state
            .borrow_mut()
            .binding
            .take()
            .expect("Shared bot arsenal binding is unavailable");
        binding.update_inventory(state);
        self.state.borrow_mut().binding = Some(binding);
        self.update_powerups(state);
    }
}

/// Host random adapter (donor `random` surface).
struct BotWorldRandom {
    draw: Rc<dyn Fn() -> f64>,
}

impl BotRandom for BotWorldRandom {
    fn next_int(&mut self) -> i32 {
        ((self.draw)() * f64::from(i32::MAX)) as i32
    }

    fn next_unit(&mut self) -> f32 {
        (self.draw)() as f32
    }
}

/// Shared bot observation world (donor `createSharedBotWorld` result).
pub struct SharedBotGame<S> {
    state: Rc<RefCell<SharedBotState<S>>>,
    cvars: Rc<RefCell<CvarRegistry>>,
    memory: GameMemory,
    random: BotWorldRandom,
    knowledge: BotWorldKnowledge<S>,
}

/// Create the shared bot observation world (donor `createSharedBotWorld`).
pub fn create_shared_bot_world<S: BotWorldSimulation>(
    options: SharedBotWorldOptions<S>,
    weapons: &WeaponAi<'_>,
) -> Result<SharedBotGame<S>, BotWorldError> {
    let q2 = options.simulation.q2_bot_source();
    let q1 = options.simulation.q1_bot_source();
    let kind = if q2.is_some() {
        BotWorldSourceKind::Q2
    } else if q1.is_some() {
        BotWorldSourceKind::Q1
    } else {
        return Err(BotWorldError::MissingSource);
    };
    let scene = options
        .simulation
        .bot_scene()
        .ok_or(BotWorldError::MissingNumericPolicy)?;
    let map_name = if kind == BotWorldSourceKind::Q2 {
        q2.as_ref().map_or_else(String::new, |q2| q2.map_name())
    } else {
        q1.as_ref().map_or_else(String::new, |q1| q1.map_name())
    };
    if !options.restoring {
        let mut cvars = options.cvars.borrow_mut();
        let entries = [
            ("sv_maxclients", options.simulation.max_clients().to_string()),
            ("g_gametype", "0".to_string()),
            ("mapname", map_name.clone()),
            ("sv_mapname", map_name.clone()),
            ("g_spSkill", "2".to_string()),
            ("bot_enable", "1".to_string()),
            ("bot_minplayers", "0".to_string()),
            ("dedicated", "1".to_string()),
            ("g_gravity", options.simulation.physics_gravity().to_string()),
        ];
        for (name, value) in entries {
            cvars
                .register(name, &value, 0)
                .map_err(|error| BotWorldError::Cvar(format!("{error:?}")))?;
        }
        cvars
            .set("mapname", &map_name, true)
            .map_err(|error| BotWorldError::Cvar(format!("{error:?}")))?;
        cvars
            .set("sv_mapname", &map_name, true)
            .map_err(|error| BotWorldError::Cvar(format!("{error:?}")))?;
    }
    let state = Rc::new(RefCell::new(SharedBotState {
        simulation: options.simulation.clone(),
        q1,
        q2,
        kind,
        scene,
        binding: None,
        actor_for_client: Rc::clone(&options.actor),
        connect: Rc::clone(&options.connect),
        drop_client: Rc::clone(&options.drop_client),
        begin: Rc::clone(&options.begin),
        print: Rc::clone(&options.print),
        console: Rc::clone(&options.console),
        message: Rc::clone(&options.message),
        ids: HashMap::new(),
        actors: HashMap::new(),
        userinfos: HashMap::new(),
        strings: HashMap::new(),
        models: HashMap::new(),
        next_entity: BOT_WORLD_FIRST_DYNAMIC,
        next_generation: 1,
        free_ids: Vec::new(),
        generations: HashMap::new(),
        begun: HashSet::new(),
    }));
    let actor_state = Rc::clone(&state);
    let binding = create_bot_arsenal_binding(
        options.simulation.clone(),
        move |client| actor_state.borrow().actor_for_id(client),
        weapons,
    )
    .ok_or(BotWorldError::MissingArsenalBinding)?;
    state.borrow_mut().binding = Some(binding);
    if !options.restoring {
        state.borrow_mut().refresh()?;
    }
    let random_state = Rc::clone(&state);
    let print = Rc::clone(&options.print);
    Ok(SharedBotGame {
        state: Rc::clone(&state),
        cvars: options.cvars,
        memory: GameMemory::new(Box::new(|| 0), Box::new(move |text| print(text.as_str()))),
        random: BotWorldRandom {
            draw: Rc::new(move || random_state.borrow().source_random()),
        },
        knowledge: BotWorldKnowledge { state },
    })
}

fn zero_vec() -> Vec3 {
    Vec3 { x: 0.0, y: 0.0, z: 0.0 }
}

fn zero_bounds() -> Bounds {
    Bounds {
        min: zero_vec(),
        max: zero_vec(),
    }
}

impl<S: BotWorldSimulation> SharedBotGame<S> {
    /// Raw arsenal binding (donor top-level `knowledge`).
    pub fn binding(&self) -> std::cell::Ref<'_, dyn BotArsenalBinding> {
        std::cell::Ref::map(self.state.borrow(), |state| {
            state
                .binding
                .as_ref()
                .expect("Shared bot arsenal binding is unavailable")
                .as_ref()
        })
    }

    /// Observation entity id for an actor (donor `entityId`).
    pub fn entity_id(&self, actor: &ActorId) -> Result<i32, BotWorldError> {
        self.state.borrow_mut().entity_id(actor)
    }

    /// Actor for an observation entity id (donor `actorForId`).
    #[must_use]
    pub fn actor_for_id(&self, number: i32) -> Option<ActorId> {
        self.state.borrow().actor_for_id(number)
    }

    /// Refresh observation ids (donor `refresh`).
    pub fn refresh(&self) -> Result<(), BotWorldError> {
        self.state.borrow_mut().refresh()
    }

    /// Shared cvar registry (donor `options.cvars`).
    #[must_use]
    pub fn cvars(&self) -> Rc<RefCell<CvarRegistry>> {
        Rc::clone(&self.cvars)
    }

    /// Game memory (donor `game.memory`).
    #[must_use]
    pub fn memory(&self) -> &GameMemory {
        &self.memory
    }

    /// Game memory mutably (donor `game.memory`).
    pub fn memory_mut(&mut self) -> &mut GameMemory {
        &mut self.memory
    }

    /// Configstring value (donor `options.configstrings.get`).
    #[must_use]
    pub fn configstring_get(&self, index: i32) -> String {
        let shared = self.state.borrow();
        if (BOT_WORLD_PLAYER_INFO_BASE..BOT_WORLD_PLAYER_INFO_BASE + BOT_WORLD_PLAYER_INFO_COUNT).contains(&index) {
            return shared.player_info(index - BOT_WORLD_PLAYER_INFO_BASE);
        }
        shared.strings.get(&index).cloned().unwrap_or_default()
    }

    /// Set a configstring (donor `options.configstrings.set`).
    pub fn configstring_set(&self, index: i32, value: &str) {
        self.state.borrow_mut().strings.insert(index, value.to_string());
    }

    /// Client userinfo (donor `options.engine.getUserinfo`).
    #[must_use]
    pub fn engine_userinfo(&self, client: i32) -> String {
        self.state.borrow().userinfos.get(&client).cloned().unwrap_or_default()
    }

    /// Set client userinfo (donor `options.engine.setUserinfo`).
    pub fn set_engine_userinfo(&self, client: i32, userinfo: &str) {
        self.state.borrow_mut().userinfos.insert(client, userinfo.to_string());
    }

    /// Send a server command (donor `options.engine.sendServerCommand`).
    pub fn send_server_command(&self, client: i32, command: &str) {
        (self.state.borrow().message)(client, command);
    }

    /// Drop a client (donor `options.engine.dropClient`).
    pub fn drop_client(&self, client: i32, reason: &str) {
        let shared = self.state.borrow();
        (shared.print)(reason);
        (shared.drop_client)(client);
    }

    /// Insert a console command (donor `insertConsoleCommand`).
    pub fn insert_console_command(&self, command: &str) {
        (self.state.borrow().console)(command);
    }

    /// Append a console command (donor `appendConsoleCommand`).
    pub fn append_console_command(&self, command: &str) {
        (self.state.borrow().console)(command);
    }

    /// Podium reset no-op (donor `resetPodiumPlayers`).
    pub fn reset_podium_players(&self) {}

    /// Whether a client number has begun.
    #[must_use]
    pub fn has_begun(&self, client: i32) -> bool {
        self.state.borrow().begun.contains(&client)
    }

    fn observe_entity(&self, number: i32) -> BotObservedEntity {
        // Scoped borrows: the arsenal binding calls back into `actor_for_id`,
        // so no borrow may be held across `source_weapon`/`model_index`.
        let (actor, meta, body, mover, common) = {
            let shared = self.state.borrow();
            (
                shared.actor_for_id(number),
                shared.metadata(shared.actor_for_id(number).as_ref()),
                shared
                    .actor_for_id(number)
                    .as_ref()
                    .and_then(|actor| shared.simulation.body(actor)),
                shared
                    .actor_for_id(number)
                    .as_ref()
                    .and_then(|actor| shared.simulation.movement_client_slot(actor)),
                shared.client_info(shared.actor_for_id(number).as_ref()),
            )
        };
        let mut player = None;
        if let (Some(actor), Some(_slot), Some(common), Some(_body)) =
            (actor.clone(), mover, common.as_ref(), body.as_ref())
        {
            let health = self.state.borrow().simulation.combat_health(&actor).unwrap_or(0);
            let weapon = self
                .state
                .borrow()
                .binding
                .as_ref()
                .expect("Shared bot arsenal binding is unavailable")
                .source_weapon(number)
                .expect("Shared bot arsenal lost its source weapon");
            let connected = {
                let shared = self.state.borrow();
                (shared.actor_for_client)(number).is_none() || shared.begun.contains(&number)
            };
            player = Some(BotObservedPlayer {
                health,
                connected,
                team: if common.spectator { BOT_WORLD_SPECTATOR_TEAM } else { 0 },
                name: common.name.clone(),
                last_hurt_client: 0,
                last_hurt_mod: 0,
                weapon,
                powerups: [0; 16],
            });
        }
        let (is_brush, live, linked, bot, generation) = {
            let shared = self.state.borrow();
            let is_brush = actor
                .as_ref()
                .is_some_and(|actor| shared.simulation.is_brush_solid(actor));
            let live = actor.as_ref().is_some_and(|actor| shared.simulation.is_live(actor));
            let linked = actor.as_ref().is_some_and(|actor| shared.simulation.body_linked(actor));
            let bot = match (shared.actor_for_client)(number) {
                Some(bot) => actor.as_ref() == Some(&bot),
                None => false,
            };
            let generation = if number < BOT_WORLD_FIRST_DYNAMIC {
                actor.as_ref().map_or(0, |actor| actor.generation() as i32)
            } else {
                shared.generations.get(&number).copied().unwrap_or(0)
            };
            (is_brush, live, linked, bot, generation)
        };
        let entity_type = if player.is_some() {
            EntityType::EtPlayer as i32
        } else if is_brush {
            EntityType::EtMover as i32
        } else {
            EntityType::EtGeneral as i32
        };
        let model_index = meta
            .as_ref()
            .map_or(0, |meta| self.state.borrow_mut().model_index(&meta.model));
        let frame = meta.as_ref().map_or(0, |meta| meta.frame);
        let inline_model = meta
            .as_ref()
            .and_then(|meta| meta.model.strip_prefix('*'))
            .and_then(|inline| inline.parse().ok());
        let contents = if is_brush {
            BOT_WORLD_BRUSH_CONTENTS
        } else if player.is_none() {
            0
        } else {
            BOT_WORLD_PLAYER_CONTENTS
        };
        BotObservedEntity {
            generation,
            present: live && body.is_some(),
            linked,
            hidden: meta.as_ref().is_some_and(|meta| meta.hidden),
            bot,
            entity_type,
            model_index,
            weapon: player.as_ref().map_or(0, |player| player.weapon),
            event: 0,
            frame,
            origin: body.as_ref().map_or_else(zero_vec, |body| body.origin),
            angles: body.as_ref().map_or_else(zero_vec, |body| body.angles),
            bounds: body.as_ref().map_or_else(zero_bounds, |body| body.bounds),
            contents,
            inline_model,
            classname: meta.map(|meta| meta.classname),
            event_time: 0,
            proximity_trigger: false,
            player,
        }
    }
}

impl<S: BotWorldSimulation> SourceBotGame for SharedBotGame<S> {
    fn product(&self) -> BotProduct {
        BotProduct::BaseQ3
    }

    fn game_type(&self) -> i32 {
        0
    }

    fn max_clients(&self) -> i32 {
        self.state.borrow().simulation.max_clients()
    }

    fn entity_count(&self) -> i32 {
        self.state.borrow().next_entity
    }

    fn clock(&self) -> BotGameClock {
        let shared = self.state.borrow();
        let time = (shared.simulation.time_seconds() * 1000.0).trunc() as i32;
        let intermission_time = if shared.is_q2() {
            let intermission = shared.q2.as_ref().map(|q2| q2.intermission());
            match intermission {
                Some(view) if !view.playing => (view.started * 1000.0).trunc() as i32,
                _ => 0,
            }
        } else {
            shared
                .q1
                .as_ref()
                .and_then(|q1| q1.intermission())
                .map_or(0, |view| ((view.exit_after - 5.0) * 1000.0).trunc() as i32)
        };
        BotGameClock {
            time,
            start_time: 0,
            intermission_time,
        }
    }

    fn entity(&self, number: i32) -> BotObservedEntity {
        self.observe_entity(number)
    }

    fn model_index(&self, name: &str) -> i32 {
        self.state.borrow_mut().model_index(name)
    }

    fn trace(&self, query: &BotTraceQuery) -> BotTraceResult {
        let pass_actor = self.state.borrow().actor_for_id(query.pass_entity);
        let result = self
            .state
            .borrow()
            .scene
            .trace(query.start, query.end, query.bounds, query.mask, pass_actor);
        let BotWorldTrace::Q3 { fraction, end, hit } = result else {
            panic!("Bot trace did not use the Q3 decision representation");
        };
        let entity_num = match hit {
            BotWorldTraceHit::Actor(actor) => self
                .state
                .borrow_mut()
                .entity_id(&actor)
                .expect("Live bot observations exceed the current simultaneous entity capacity"),
            BotWorldTraceHit::World => BOT_WORLD_WORLD_ENTITY,
            BotWorldTraceHit::None => BOT_WORLD_ABSENT_ENTITY,
        };
        BotTraceResult {
            fraction,
            entity_num,
            end,
        }
    }

    fn point_contents(&self, point: Vec3, _pass_entity: i32) -> i32 {
        match self.state.borrow().scene.point_contents(point) {
            BotWorldContents::Q3(contents) => contents,
            BotWorldContents::Other => panic!("Bot contents did not use the Q3 decision representation"),
        }
    }

    fn random(&mut self) -> &mut dyn BotRandom {
        &mut self.random
    }

    fn choose_team(&mut self, _client: i32) -> i32 {
        0
    }

    fn activate_bot(&mut self, client: i32) {
        let shared = self.state.borrow();
        if !shared.is_q2() {
            return;
        }
        let actor = (shared.actor_for_client)(client);
        if let (Some(q2), Some(actor)) = (shared.q2.as_ref(), actor.as_ref()) {
            if q2.entity(actor).is_some() {
                q2.set_server_flag(actor, 16);
            }
        }
    }

    fn exit_level(&mut self) {
        let shared = self.state.borrow();
        if shared.is_q2() {
            if let Some(q2) = shared.q2.as_ref() {
                q2.end_deathmatch_level();
            }
        } else if let Some(q1) = shared.q1.as_ref() {
            q1.request_intermission_exit(shared.simulation.time_seconds(), true);
        }
    }

    fn client_userinfo_changed(&mut self, client: i32) {
        let shared = self.state.borrow();
        let actor = (shared.actor_for_client)(client);
        let Some(actor) = actor else { return };
        if shared.is_q2() {
            if let Some(q2) = shared.q2.as_ref() {
                if q2.entity(&actor).is_some() {
                    let userinfo = shared.userinfos.get(&client).cloned().unwrap_or_default();
                    q2.userinfo_changed(&actor, &userinfo);
                }
            }
            return;
        }
        let userinfo = shared.userinfos.get(&client).cloned().unwrap_or_default();
        let fields: Vec<&str> = userinfo.split('\\').collect();
        let mut info = HashMap::new();
        let mut index = 1;
        while index + 1 < fields.len() {
            info.insert(fields[index].to_string(), fields[index + 1].to_string());
            index += 2;
        }
        if let Some(q1) = shared.q1.as_ref() {
            q1.update_client(&actor, &info);
        }
    }

    fn client_connect(&mut self, client: i32, first_time: bool, is_bot: bool) -> Option<String> {
        let outcome = {
            let shared = self.state.borrow();
            if !shared.is_q2() {
                None
            } else {
                let userinfo = shared.userinfos.get(&client).cloned().unwrap_or_default();
                shared.q2.as_ref().map(|q2| {
                    if q2.use_rerelease_players() {
                        q2.connect_rerelease(&userinfo, is_bot)
                    } else {
                        q2.connect_classic(&userinfo)
                    }
                })
            }
        };
        if let Some(outcome) = outcome {
            if !outcome.allowed {
                return Some(outcome.reason);
            }
            self.state.borrow_mut().userinfos.insert(client, outcome.userinfo);
        }
        self.client_userinfo_changed(client);
        let connect = Rc::clone(&self.state.borrow().connect);
        if connect(client, !first_time) {
            None
        } else {
            Some("Bot setup failed".to_string())
        }
    }

    fn client_begin(&mut self, client: i32) {
        let begin = Rc::clone(&self.state.borrow().begin);
        self.state.borrow_mut().begun.insert(client);
        begin(client);
    }

    fn pickup_candidates(&self, client: i32) -> Vec<BotObservedPickup> {
        let actors: Vec<ActorId> = self.state.borrow().actors.values().cloned().collect();
        let mut pickups = Vec::new();
        for actor in actors {
            if let Some(pickup) = self.state.borrow_mut().inspect_pickup(client, &actor) {
                pickups.push(pickup);
            }
        }
        pickups
    }

    fn knowledge(&mut self) -> &mut dyn BotArsenalKnowledge {
        &mut self.knowledge
    }
}

impl<S: BotWorldSimulation> SharedBotGame<S> {
    /// Capture the world checkpoint (donor `checkpoint()`).
    #[must_use]
    pub fn checkpoint(&self) -> SaveJson {
        let shared = self.state.borrow();
        let memory = self.memory.capture_save_state();
        let mut actors: Vec<(&ActorId, &i32)> = shared.ids.iter().collect();
        actors.sort_by_key(|(_, id)| **id);
        let mut generations: Vec<(&i32, &i32)> = shared.generations.iter().collect();
        generations.sort_by_key(|(id, _)| **id);
        let mut userinfos: Vec<(&i32, &String)> = shared.userinfos.iter().collect();
        userinfos.sort_by_key(|(id, _)| **id);
        let mut strings: Vec<(&i32, &String)> = shared.strings.iter().collect();
        strings.sort_by_key(|(id, _)| **id);
        let mut models: Vec<(&String, &i32)> = shared.models.iter().collect();
        models.sort_by_key(|(name, _)| (*name).clone());
        let mut begun: Vec<i32> = shared.begun.iter().copied().collect();
        begun.sort_unstable();
        obj(vec![
            ("version", int(1)),
            ("source", str(if shared.is_q2() { "q2" } else { "q1" })),
            (
                "memory",
                obj(vec![
                    ("pool", SaveJson::Bytes(memory.pool)),
                    ("allocPoint", int(memory.alloc_point as i64)),
                ]),
            ),
            ("nextEntity", int(i64::from(shared.next_entity))),
            ("nextGeneration", int(i64::from(shared.next_generation))),
            (
                "actors",
                arr(actors
                    .into_iter()
                    .map(|(actor, id)| {
                        obj(vec![
                            ("actor", write_saved_actor(SavedActorId::from(actor))),
                            ("id", int(i64::from(*id))),
                        ])
                    })
                    .collect()),
            ),
            (
                "freeIds",
                arr(shared.free_ids.iter().map(|id| int(i64::from(*id))).collect()),
            ),
            (
                "generations",
                arr(generations
                    .into_iter()
                    .map(|(id, generation)| {
                        obj(vec![
                            ("id", int(i64::from(*id))),
                            ("generation", int(i64::from(*generation))),
                        ])
                    })
                    .collect()),
            ),
            ("begun", arr(begun.into_iter().map(|id| int(i64::from(id))).collect())),
            (
                "userinfos",
                arr(userinfos
                    .into_iter()
                    .map(|(id, value)| obj(vec![("id", int(i64::from(*id))), ("value", str(value))]))
                    .collect()),
            ),
            (
                "strings",
                arr(strings
                    .into_iter()
                    .map(|(id, value)| obj(vec![("id", int(i64::from(*id))), ("value", str(value))]))
                    .collect()),
            ),
            (
                "models",
                arr(models
                    .into_iter()
                    .map(|(name, id)| obj(vec![("name", str(name)), ("id", int(i64::from(*id)))]))
                    .collect()),
            ),
        ])
    }

    /// Restore a world checkpoint (donor `restoreCheckpoint(value, actor)`).
    pub fn restore_checkpoint(
        &mut self,
        value: &SaveJson,
        actor: &dyn Fn(SavedActorId) -> ActorId,
    ) -> Result<(), BotWorldError> {
        let reader = SaveReader::at(value, "sharedBotWorld");
        reader.field("version").literal_i64(1)?;
        let expected_source = if self.state.borrow().is_q2() { "q2" } else { "q1" };
        reader.field("source").literal_str(expected_source)?;
        let memory_reader = reader.field("memory");
        let save = GameMemorySave {
            pool: memory_reader.field("pool").bytes()?,
            alloc_point: memory_reader.field("allocPoint").integer(0)? as usize,
        };
        let mut probed = GameMemory::new(Box::new(|| 0), Box::new(|_| {}));
        probed
            .restore_save_state(&save)
            .map_err(|error| BotWorldError::Memory(format!("{error:?}")))?;
        let entity_limit = reader.field("nextEntity").integer(i64::from(BOT_WORLD_FIRST_DYNAMIC))? as i32;
        let generation_limit = reader.field("nextGeneration").integer(1)? as i32;
        if entity_limit > BOT_WORLD_WORLD_ENTITY {
            return Err(reader
                .field("nextEntity")
                .fail("observation entity capacity exceeded")
                .into());
        }
        let read_entity = |entry: &SaveReader<'_>| -> Result<i32, WorldError> {
            let id = entry.integer(i64::from(BOT_WORLD_FIRST_DYNAMIC))? as i32;
            if id >= entity_limit {
                return Err(entry.fail("observation entity outside allocated range"));
            }
            Ok(id)
        };
        let mut restored_ids: HashMap<ActorId, i32> = HashMap::new();
        let mut restored_actors: HashMap<i32, ActorId> = HashMap::new();
        let mut seen_actors: HashSet<(u32, u32)> = HashSet::new();
        reader.field("actors").list(|entry| -> Result<(), WorldError> {
            let id = read_entity(&entry.field("id"))?;
            let saved = read_saved_actor(entry.field("actor"))?;
            let key = (saved.slot, saved.generation);
            if restored_actors.contains_key(&id) || seen_actors.contains(&key) {
                return Err(entry.fail("duplicate observation entity or actor"));
            }
            let resolved = actor(saved);
            if restored_ids.keys().any(|existing| existing == &resolved) {
                return Err(entry.fail("duplicate remapped observation actor"));
            }
            seen_actors.insert(key);
            restored_ids.insert(resolved.clone(), id);
            restored_actors.insert(id, resolved);
            Ok(())
        })?;
        let mut restored_free: Vec<i32> = Vec::new();
        let mut free: HashSet<i32> = HashSet::new();
        reader.field("freeIds").list(|entry| -> Result<(), WorldError> {
            let id = read_entity(&entry)?;
            if free.contains(&id) || restored_actors.contains_key(&id) {
                return Err(entry.fail("duplicate or active free observation entity"));
            }
            free.insert(id);
            restored_free.push(id);
            Ok(())
        })?;
        if restored_actors.len() + free.len() != (entity_limit - BOT_WORLD_FIRST_DYNAMIC) as usize {
            return Err(reader.fail("allocated observation entities are missing").into());
        }
        let mut restored_generations: HashMap<i32, i32> = HashMap::new();
        let mut used_generations: HashSet<i32> = HashSet::new();
        reader.field("generations").list(|entry| -> Result<(), WorldError> {
            let id = read_entity(&entry.field("id"))?;
            let generation = entry.field("generation").integer(1)? as i32;
            if generation >= generation_limit
                || restored_generations.contains_key(&id)
                || used_generations.contains(&generation)
            {
                return Err(entry.fail("invalid or duplicate observation generation"));
            }
            used_generations.insert(generation);
            restored_generations.insert(id, generation);
            Ok(())
        })?;
        if restored_generations.len() != (entity_limit - BOT_WORLD_FIRST_DYNAMIC) as usize {
            return Err(reader.fail("observation generations are missing").into());
        }
        let mut restored_begun: HashSet<i32> = HashSet::new();
        let max_clients = self.state.borrow().simulation.max_clients();
        reader.field("begun").list(|entry| -> Result<(), WorldError> {
            let id = entry.integer(0)? as i32;
            if id >= max_clients || restored_begun.contains(&id) {
                return Err(entry.fail("invalid or duplicate begun client"));
            }
            restored_begun.insert(id);
            Ok(())
        })?;
        let read_strings = |name: &str| -> Result<HashMap<i32, String>, WorldError> {
            let mut result = HashMap::new();
            reader.field(name).list(|entry| -> Result<(), WorldError> {
                let id = entry.field("id").integer(0)? as i32;
                if result.contains_key(&id) {
                    return Err(entry.fail("duplicate string index"));
                }
                result.insert(id, entry.field("value").string()?);
                Ok(())
            })?;
            Ok(result)
        };
        let restored_userinfos = read_strings("userinfos")?;
        let restored_strings = read_strings("strings")?;
        let mut restored_models: HashMap<String, i32> = HashMap::new();
        let mut model_ids: HashSet<i32> = HashSet::new();
        let model_base = self.state.borrow().simulation.world_model_count() as i32;
        reader.field("models").list(|entry| -> Result<(), WorldError> {
            let name = entry.field("name").string()?;
            let id = entry.field("id").integer(i64::from(model_base))? as i32;
            if name.is_empty()
                || name.starts_with('*')
                || restored_models.contains_key(&name)
                || model_ids.contains(&id)
            {
                return Err(entry.fail("invalid or duplicate model index"));
            }
            restored_models.insert(name, id);
            model_ids.insert(id);
            Ok(())
        })?;
        for index in 0..restored_models.len() as i32 {
            if !model_ids.contains(&(model_base + index)) {
                return Err(reader.field("models").fail("model indices are not contiguous").into());
            }
        }
        self.memory
            .restore_save_state(&probed.capture_save_state())
            .map_err(|error| BotWorldError::Memory(format!("{error:?}")))?;
        let mut shared = self.state.borrow_mut();
        shared.ids = restored_ids;
        shared.actors = restored_actors;
        shared.free_ids = restored_free;
        shared.generations = restored_generations;
        shared.begun = restored_begun;
        shared.userinfos = restored_userinfos;
        shared.strings = restored_strings;
        shared.models = restored_models;
        shared.next_entity = entity_limit;
        shared.next_generation = generation_limit;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_bots::behavior::assets::BotSourceFiles;
    use qa_core::cmd::Dialect;
    use qa_core::identity::IdentityOwner;

    use super::super::bot_q1_knowledge::{Q1BotBounds, Q1BotCombat, Q1BotPlayer};
    use super::super::bot_q2_knowledge::{Q2BotCombat, Q2BotWeaponState};
    use super::super::bot_q3_knowledge::{Q3BotArsenalView, Q3BotCombat};
    use qa_content::q1::foundation::types::{Q1BaseWeapon, Q1Weapon};
    use qa_content::q2::foundation::weapons::types::Q2WeaponDefinition;

    struct FakeFiles;

    impl BotSourceFiles for FakeFiles {
        fn read(&self, _path: &str) -> Option<Vec<u8>> {
            None
        }

        fn list(&self, _directory: &str, _extension: &str) -> Vec<String> {
            Vec::new()
        }
    }

    #[derive(Debug, Clone)]
    struct StubScene;

    impl BotWorldScene for StubScene {
        fn point_contents(&self, _point: Vec3) -> BotWorldContents {
            BotWorldContents::Q3(0)
        }

        fn trace(
            &self,
            _start: Vec3,
            end: Vec3,
            _bounds: Option<Bounds>,
            _mask: i32,
            _pass_actor: Option<ActorId>,
        ) -> BotWorldTrace {
            BotWorldTrace::Q3 {
                fraction: 1.0,
                end,
                hit: BotWorldTraceHit::None,
            }
        }
    }

    #[derive(Debug, Clone)]
    struct StubQ1 {
        player: ActorId,
    }

    impl BotWorldQ1Source for StubQ1 {
        fn map_name(&self) -> String {
            "e1m1".to_string()
        }

        fn client(&self, actor: &ActorId) -> Option<BotWorldQ1Client> {
            if actor == &self.player {
                Some(BotWorldQ1Client {
                    name: "Ranger".to_string(),
                    observer: false,
                    frags: 5,
                })
            } else {
                None
            }
        }

        fn entity(&self, actor: &ActorId) -> Option<BotWorldQ1Entity> {
            if actor == &self.player {
                Some(BotWorldQ1Entity {
                    model: "progs/player.mdl".to_string(),
                    frame: 0,
                    classname: "player".to_string(),
                    max_health: 100,
                })
            } else {
                None
            }
        }

        fn player_max_health(&self, _actor: &ActorId) -> Option<i32> {
            None
        }

        fn powerup_expires(&self, _actor: &ActorId, _channel: BotWorldQ1Powerup) -> f64 {
            0.0
        }

        fn time(&self) -> f64 {
            10.0
        }

        fn intermission(&self) -> Option<BotWorldQ1Intermission> {
            None
        }

        fn world_actor(&self) -> Option<ActorId> {
            None
        }

        fn random(&self) -> f64 {
            0.5
        }

        fn request_intermission_exit(&self, _now_seconds: f64, _silent: bool) {}

        fn update_client(&self, _actor: &ActorId, _info: &HashMap<String, String>) {}

        fn observe_supply(&self, _actor: &ActorId, _recipient: &ActorId) -> Option<PickupSupplyObservation> {
            None
        }

        fn preview_supply(&self, _actor: &ActorId, _recipient: &ActorId) -> Option<PickupSupplyPreview> {
            None
        }
    }

    #[derive(Debug, Clone)]
    struct StubSim {
        player: ActorId,
        dynamic: ActorId,
        q1: Option<Rc<StubQ1>>,
        scene: Option<Rc<StubScene>>,
    }

    impl Q1BotKnowledgeSimulation for StubSim {
        fn has_q1_weapon_source(&self) -> bool {
            true
        }

        fn q1_time(&self) -> f64 {
            10.0
        }

        fn q1_weapons_in_order(&self) -> Vec<Q1Weapon> {
            Vec::new()
        }

        fn q1_is_registered_weapon(&self, _weapon: Q1Weapon) -> bool {
            false
        }

        fn q1_weapon_item(&self, _weapon: Q1BaseWeapon) -> qa_content::contract::ItemId {
            String::new()
        }

        fn q1_weapon_ammo(&self, _weapon: Q1BaseWeapon) -> Option<qa_content::contract::ItemId> {
            None
        }

        fn q1_weapon_model(&self, _weapon: Q1BaseWeapon) -> String {
            String::new()
        }

        fn q1_player(&self, _actor: &ActorId) -> Option<Q1BotPlayer> {
            None
        }

        fn q1_weapon_available(&self, _actor: &ActorId, _weapon: Q1BaseWeapon) -> bool {
            false
        }

        fn q1_nail_speed(&self, _actor: &ActorId, base: f64) -> f64 {
            base
        }

        fn q1_quad_expires(&self, _actor: &ActorId) -> f64 {
            0.0
        }

        fn inventory_count(&self, _actor: &ActorId, _item: &str) -> i32 {
            0
        }

        fn combat_read(&self, _actor: &ActorId) -> Option<Q1BotCombat> {
            None
        }

        fn body_bounds(&self, _actor: &ActorId) -> Option<Q1BotBounds> {
            None
        }
    }

    impl Q2BotKnowledgeSimulation for StubSim {
        fn has_q2_weapon_source(&self) -> bool {
            false
        }

        fn q2_weapon_definitions(&self) -> Vec<Q2WeaponDefinition> {
            Vec::new()
        }

        fn q2_edition_is_rerelease(&self) -> bool {
            false
        }

        fn q2_mode_is_deathmatch(&self) -> bool {
            false
        }

        fn q2_weapon_state(&self, _actor: &ActorId) -> Option<Q2BotWeaponState> {
            None
        }

        fn inventory_count(&self, _actor: &ActorId, _item: &str) -> i32 {
            0
        }

        fn combat_read(&self, _actor: &ActorId) -> Option<Q2BotCombat> {
            None
        }
    }

    impl Q3BotKnowledgeSimulation for StubSim {
        fn has_selected_q3_weapon_source(&self) -> bool {
            false
        }

        fn q3_source_has(&self, _actor: &ActorId) -> bool {
            false
        }

        fn q3_arsenal(&self, _actor: &ActorId) -> Option<Q3BotArsenalView> {
            None
        }

        fn inventory_count(&self, _actor: &ActorId, _item: &str) -> i32 {
            0
        }

        fn combat_read(&self, _actor: &ActorId) -> Option<Q3BotCombat> {
            None
        }
    }

    impl BotWorldSimulation for StubSim {
        fn q1_bot_source(&self) -> Option<Rc<dyn BotWorldQ1Source>> {
            self.q1.clone().map(|q1| q1 as Rc<dyn BotWorldQ1Source>)
        }

        fn q2_bot_source(&self) -> Option<Rc<dyn BotWorldQ2Source>> {
            None
        }

        fn q2_arsenal_entity(&self, _actor: &ActorId) -> Option<BotWorldQ2Entity> {
            None
        }

        fn max_clients(&self) -> i32 {
            8
        }

        fn world_model_count(&self) -> usize {
            3
        }

        fn character_appearance_provider(&self) -> String {
            "q1:base".to_string()
        }

        fn physics_gravity(&self) -> f64 {
            800.0
        }

        fn players(&self) -> Vec<ActorId> {
            vec![self.player.clone()]
        }

        fn movement_client_slot(&self, actor: &ActorId) -> Option<i32> {
            if actor == &self.player {
                Some(0)
            } else {
                None
            }
        }

        fn is_live(&self, _actor: &ActorId) -> bool {
            true
        }

        fn observations(&self) -> Vec<ActorId> {
            vec![self.dynamic.clone()]
        }

        fn body(&self, _actor: &ActorId) -> Option<BotWorldBody> {
            Some(BotWorldBody {
                origin: qa_core::math::vec3(1.0, 2.0, 3.0),
                angles: qa_core::math::vec3(0.0, 0.0, 0.0),
                bounds: Bounds {
                    min: qa_core::math::vec3(-16.0, -16.0, -24.0),
                    max: qa_core::math::vec3(16.0, 16.0, 32.0),
                },
            })
        }

        fn body_linked(&self, _actor: &ActorId) -> bool {
            true
        }

        fn combat_health(&self, _actor: &ActorId) -> Option<i32> {
            Some(100)
        }

        fn is_brush_solid(&self, _actor: &ActorId) -> bool {
            false
        }

        fn time_seconds(&self) -> f64 {
            2.5
        }

        fn bot_scene(&self) -> Option<Rc<dyn BotWorldScene>> {
            self.scene.clone().map(|scene| scene as Rc<dyn BotWorldScene>)
        }
    }

    fn stub_options(simulation: StubSim) -> SharedBotWorldOptions<StubSim> {
        let player = simulation.player.clone();
        SharedBotWorldOptions {
            restoring: false,
            simulation,
            cvars: Rc::new(RefCell::new(CvarRegistry::new(Dialect::Q3))),
            actor: Rc::new(move |client| if client == 0 { Some(player.clone()) } else { None }),
            connect: Rc::new(|_, _| true),
            drop_client: Rc::new(|_| {}),
            begin: Rc::new(|_| {}),
            print: Rc::new(|_| {}),
            console: Rc::new(|_| {}),
            message: Rc::new(|_, _| {}),
        }
    }

    fn stub_sim() -> (IdentityOwner, StubSim) {
        let owner = IdentityOwner::create("bot-world-test").unwrap();
        let player = owner.actor(1, 1);
        let dynamic = owner.actor(2, 1);
        let q1 = Rc::new(StubQ1 { player: player.clone() });
        (
            owner,
            StubSim {
                player,
                dynamic,
                q1: Some(q1),
                scene: Some(Rc::new(StubScene)),
            },
        )
    }

    #[test]
    fn missing_source_errors() {
        let (_owner, mut simulation) = stub_sim();
        simulation.q1 = None;
        let files = FakeFiles;
        let weapons = WeaponAi::new(&files);
        let outcome = create_shared_bot_world(stub_options(simulation), &weapons);
        assert!(matches!(outcome, Err(BotWorldError::MissingSource)));
    }

    #[test]
    fn missing_scene_errors() {
        let (_owner, mut simulation) = stub_sim();
        simulation.scene = None;
        let files = FakeFiles;
        let weapons = WeaponAi::new(&files);
        let outcome = create_shared_bot_world(stub_options(simulation), &weapons);
        assert!(matches!(outcome, Err(BotWorldError::MissingNumericPolicy)));
    }

    #[test]
    fn q1_world_registers_and_maps_ids() {
        let (_owner, simulation) = stub_sim();
        let player = simulation.player.clone();
        let dynamic = simulation.dynamic.clone();
        let files = FakeFiles;
        let weapons = WeaponAi::new(&files);
        let options = stub_options(simulation);
        let cvars = Rc::clone(&options.cvars);
        let game = create_shared_bot_world(options, &weapons).unwrap();
        assert_eq!(cvars.borrow().get("sv_maxclients").unwrap().value, "8");
        assert_eq!(cvars.borrow().variable_string("mapname"), "e1m1");
        assert_eq!(game.product(), BotProduct::BaseQ3);
        assert_eq!(game.game_type(), 0);
        assert_eq!(game.max_clients(), 8);
        assert_eq!(game.entity_count(), 65);
        assert_eq!(game.entity_id(&player).unwrap(), 0);
        assert_eq!(game.entity_id(&dynamic).unwrap(), 64);
        assert_eq!(game.actor_for_id(0), Some(player.clone()));
        assert_eq!(game.actor_for_id(64), Some(dynamic.clone()));
        let clock = game.clock();
        assert_eq!(clock.time, 2500);
        assert_eq!(clock.start_time, 0);
        assert_eq!(clock.intermission_time, 0);
        let observed = game.entity(0);
        assert!(observed.present);
        assert!(observed.linked);
        assert!(observed.bot);
        assert_eq!(observed.entity_type, EntityType::EtPlayer as i32);
        assert_eq!(observed.player.as_ref().unwrap().name, "Ranger");
        assert_eq!(observed.player.as_ref().unwrap().health, 100);
        assert_eq!(game.point_contents(qa_core::math::vec3(0.0, 0.0, 0.0), 0), 0);
    }

    #[test]
    fn checkpoint_round_trip() {
        let (_owner, simulation) = stub_sim();
        let dynamic = simulation.dynamic.clone();
        let files = FakeFiles;
        let weapons = WeaponAi::new(&files);
        let game = create_shared_bot_world(stub_options(simulation), &weapons).unwrap();
        let image = game.checkpoint();
        let (_owner, simulation) = stub_sim();
        let mut options = stub_options(simulation);
        options.restoring = true;
        let mut restored = create_shared_bot_world(options, &weapons).unwrap();
        assert_eq!(restored.entity_count(), 64);
        let expected = dynamic.clone();
        restored
            .restore_checkpoint(&image, &|saved| {
                assert_eq!(saved, SavedActorId::from(&expected));
                expected.clone()
            })
            .unwrap();
        assert_eq!(restored.entity_count(), 65);
        assert_eq!(restored.actor_for_id(64), Some(dynamic));
    }

    #[test]
    fn q1_client_connect_succeeds() {
        let (_owner, simulation) = stub_sim();
        let files = FakeFiles;
        let weapons = WeaponAi::new(&files);
        let mut game = create_shared_bot_world(stub_options(simulation), &weapons).unwrap();
        assert_eq!(game.client_connect(0, true, true), None);
        game.client_begin(0);
        assert!(game.has_begun(0));
    }
}
