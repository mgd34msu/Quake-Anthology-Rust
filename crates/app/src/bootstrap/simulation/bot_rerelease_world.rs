//! Native rerelease bot world projection.
//!
//! Port of Quake-Anthology-TS `src/app/bootstrap/simulation/bot-rerelease-world.ts`.
//!
//! Sibling homes: [`SharedSimulation`](super::runtime::SharedSimulation)
//! (`simulation/runtime.ts` port) is the live shared simulation, and
//! [`ApplicationBotNavigation`](super::navigation::ApplicationBotNavigation)
//! (`navigation.ts` port) is the live bot navigation. The
//! [`RereleaseBotWorldSimulation`] and [`RereleaseBotNavigation`] seams
//! stay as the narrow interfaces this module needs (simulation and
//! navigation surfaces); only test doubles implement them.
//!
//! The donor throws when the bot loses its player/body or when the scene
//! stops answering in the Q3 decision representation. [`BotWorldT`] methods
//! return plain values, so the port panics with the donor messages on those
//! invariant violations; construction failures are [`RereleaseBotWorldError`].

use std::rc::Rc;

use qa_bots::behavior::rerelease::data::knowledge::{BotGameModeT, BotKnowledge};
use qa_bots::behavior::rerelease::nav::RereleaseNavigation;
use qa_bots::behavior::rerelease::world::{
    BotContents, BotEntityKind, BotEntityT, BotSelfT, BotSoundT, BotTraceT, BotWorldT,
};
use qa_content::contract::ItemId;
use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3};
use qa_core::numeric::NumericProfile;
use thiserror::Error;

/// Bot objectives over the shared match (donor `RereleaseBotObjectives`).
pub trait RereleaseBotObjectives {
    /// Admit an actor to a team when the match needs balancing.
    fn admit(&self, actor: &ActorId);
    /// Current game mode.
    fn mode(&self) -> BotGameModeT;
    /// Team number for an actor (0 = none).
    fn team(&self, actor: &ActorId) -> i32;
    /// Whether an actor carries an objective.
    fn carrying(&self, actor: &ActorId) -> bool;
    /// Objective goal origin for an actor.
    fn goal(&self, actor: &ActorId) -> Option<Vec3>;
}

impl<O: RereleaseBotObjectives + ?Sized> RereleaseBotObjectives for Rc<O> {
    fn admit(&self, actor: &ActorId) {
        (**self).admit(actor);
    }
    fn mode(&self) -> BotGameModeT {
        (**self).mode()
    }
    fn team(&self, actor: &ActorId) -> i32 {
        (**self).team(actor)
    }
    fn carrying(&self, actor: &ActorId) -> bool {
        (**self).carrying(actor)
    }
    fn goal(&self, actor: &ActorId) -> Option<Vec3> {
        (**self).goal(actor)
    }
}

/// Per-service host closures (donor `isBot`/`identify`/`elapsed`/`sounds`).
pub trait RereleaseBotWorldHost {
    /// Whether an actor is bot-controlled.
    fn is_bot(&self, actor: &ActorId) -> bool;
    /// Stable observation number for an actor.
    fn identify(&self, actor: &ActorId) -> i32;
    /// Elapsed frame milliseconds.
    fn elapsed_ms(&self) -> u64;
    /// Noises heard since the last frame.
    fn sounds(&self) -> Vec<BotSoundT>;
}

impl<H: RereleaseBotWorldHost + ?Sized> RereleaseBotWorldHost for Rc<H> {
    fn is_bot(&self, actor: &ActorId) -> bool {
        (**self).is_bot(actor)
    }
    fn identify(&self, actor: &ActorId) -> i32 {
        (**self).identify(actor)
    }
    fn elapsed_ms(&self) -> u64 {
        (**self).elapsed_ms()
    }
    fn sounds(&self) -> Vec<BotSoundT> {
        (**self).sounds()
    }
}

/// Navigation surface for rerelease bots (donor `ApplicationBotNavigation`).
pub trait RereleaseBotNavigation {
    /// Navigation runtime for one client slot.
    fn for_client(&self, slot: u32) -> Box<dyn RereleaseNavigation>;
}

/// Movement player projection (donor `movementPlayer` shape).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RereleaseBotPlayer {
    /// Client slot.
    pub client_slot: u32,
    /// View angles.
    pub view_angles: Vec3,
    /// View height.
    pub view_height: f32,
    /// Water level 0-3.
    pub water_level: i32,
}

/// Body projection (donor `bodies.read` shape).
#[derive(Debug, Clone, PartialEq)]
pub struct RereleaseBotBody {
    /// Origin.
    pub origin: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Bounds.
    pub bounds: Bounds,
    /// Ground actor.
    pub ground: Option<ActorId>,
}

/// Player UI projection (donor `playerUi` shape).
#[derive(Debug, Clone, PartialEq)]
pub struct RereleaseBotUiItem {
    /// Item kind (`weapon` for weapons).
    pub kind: String,
    /// Item id.
    pub id: ItemId,
}

/// Player UI projection (donor `playerUi` shape).
#[derive(Debug, Clone, PartialEq)]
pub struct RereleaseBotUi {
    /// Visible items.
    pub items: Vec<RereleaseBotUiItem>,
    /// Active weapon item.
    pub active_weapon: Option<ItemId>,
    /// Inventory counts.
    pub inventory: Vec<(ItemId, i32)>,
    /// Health.
    pub health: f32,
    /// Regular armor points.
    pub armor_points: f32,
}

/// Bot entity projection (donor `botEntity` shape).
#[derive(Debug, Clone, PartialEq)]
pub struct RereleaseBotEntity {
    /// Classname.
    pub classname: String,
    /// Hidden from bots.
    pub hidden: bool,
    /// Health.
    pub health: f32,
    /// Spawnflags.
    pub spawnflags: i32,
    /// Targetname.
    pub targetname: String,
}

/// Combat projection (donor `combat.read` shape).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RereleaseBotCombat {
    /// Health.
    pub health: f32,
    /// Invulnerable.
    pub invulnerable: bool,
}

/// Scene trace projection (donor `scene.trace` shape).
#[derive(Debug, Clone, PartialEq)]
pub struct RereleaseBotTrace {
    /// Fraction reached.
    pub fraction: f32,
    /// End position.
    pub end: Vec3,
    /// Started inside solid.
    pub start_solid: bool,
    /// Hit actor.
    pub hit_actor: Option<ActorId>,
}

/// Simulation surface for the rerelease bot world.
pub trait RereleaseBotWorldSimulation {
    /// Donor source numeric policy.
    fn bot_numeric(&self) -> Option<NumericProfile>;
    /// Donor `simulation.timeSeconds`.
    fn time_seconds(&self) -> f32;
    /// Donor `simulation.movementPlayer(actor)`.
    fn movement_player(&self, actor: &ActorId) -> Option<RereleaseBotPlayer>;
    /// Donor `simulation.bodies.read(actor)`.
    fn body_read(&self, actor: &ActorId) -> Option<RereleaseBotBody>;
    /// Donor `simulation.bodies.linked(actor)`.
    fn bodies_linked(&self, actor: &ActorId) -> bool;
    /// Donor `simulation.playerUi(actor)`.
    fn player_ui(&self, actor: &ActorId) -> RereleaseBotUi;
    /// Donor `simulation.botEntity(actor)`.
    fn bot_entity(&self, actor: &ActorId) -> Option<RereleaseBotEntity>;
    /// Donor `simulation.combat.read(actor)`.
    fn combat_read(&self, actor: &ActorId) -> Option<RereleaseBotCombat>;
    /// Donor `simulation.inventory.count(actor, item)`.
    fn inventory_count(&self, actor: &ActorId, item: &str) -> i32;
    /// Donor `simulation.actors.observations()` ids.
    fn actor_observations(&self) -> Vec<ActorId>;
    /// Donor `simulation.scene.trace` with the Q3 decision policy.
    fn bot_trace(&self, start: Vec3, end: Vec3, bounds: Option<Bounds>, pass_actor: &ActorId) -> RereleaseBotTrace;
    /// Donor `simulation.scene.pointContents` Q3 contents, or `None` when the
    /// scene answers in a foreign representation.
    fn bot_point_contents(&self, point: Vec3, pass_actor: &ActorId) -> Option<i32>;
}

impl<S: RereleaseBotWorldSimulation + ?Sized> RereleaseBotWorldSimulation for Rc<S> {
    fn bot_numeric(&self) -> Option<NumericProfile> {
        (**self).bot_numeric()
    }
    fn time_seconds(&self) -> f32 {
        (**self).time_seconds()
    }
    fn movement_player(&self, actor: &ActorId) -> Option<RereleaseBotPlayer> {
        (**self).movement_player(actor)
    }
    fn body_read(&self, actor: &ActorId) -> Option<RereleaseBotBody> {
        (**self).body_read(actor)
    }
    fn bodies_linked(&self, actor: &ActorId) -> bool {
        (**self).bodies_linked(actor)
    }
    fn player_ui(&self, actor: &ActorId) -> RereleaseBotUi {
        (**self).player_ui(actor)
    }
    fn bot_entity(&self, actor: &ActorId) -> Option<RereleaseBotEntity> {
        (**self).bot_entity(actor)
    }
    fn combat_read(&self, actor: &ActorId) -> Option<RereleaseBotCombat> {
        (**self).combat_read(actor)
    }
    fn inventory_count(&self, actor: &ActorId, item: &str) -> i32 {
        (**self).inventory_count(actor, item)
    }
    fn actor_observations(&self) -> Vec<ActorId> {
        (**self).actor_observations()
    }
    fn bot_trace(&self, start: Vec3, end: Vec3, bounds: Option<Bounds>, pass_actor: &ActorId) -> RereleaseBotTrace {
        (**self).bot_trace(start, end, bounds, pass_actor)
    }
    fn bot_point_contents(&self, point: Vec3, pass_actor: &ActorId) -> Option<i32> {
        (**self).bot_point_contents(point, pass_actor)
    }
}

/// Rerelease bot world construction failures.
#[derive(Debug, Error)]
pub enum RereleaseBotWorldError {
    /// Native bot world requires source numeric policy.
    #[error("Native bot world requires source numeric policy")]
    MissingNumeric,
    /// Native bot lost shared player.
    #[error("Native bot lost shared player")]
    MissingPlayer,
}

fn normalized(name: &str) -> String {
    let name = name.rsplit(':').next().unwrap_or(name);
    let slash = name.rfind("weapon/").map(|index| index + 7);
    let underscore = name.rfind("weapon_").map(|index| index + 7);
    let cut = slash.max(underscore).unwrap_or(0);
    name[cut..].replace(['_', '/'], "")
}

/// Resolve a knowledge weapon number to a live weapon item.
pub fn native_weapon_item<S: RereleaseBotWorldSimulation>(
    simulation: &S,
    actor: &ActorId,
    knowledge: &BotKnowledge,
    number: i32,
) -> Option<ItemId> {
    let weapon = knowledge.weapon_by_number(number)?;
    let ui = simulation.player_ui(actor);
    ui.items.iter().find_map(|item| {
        (item.kind == "weapon" && normalized(&item.id) == normalized(&weapon.entry.name)).then(|| item.id.clone())
    })
}

/// World construction options (donor `RereleaseWorldOptions`).
pub struct RereleaseWorldOptions<S, O, H, N> {
    /// Shared simulation.
    pub simulation: S,
    /// Bot navigation.
    pub navigation: N,
    /// Rerelease knowledge.
    pub knowledge: Rc<BotKnowledge>,
    /// Bot objectives.
    pub objectives: O,
    /// Service host closures.
    pub host: H,
}

/// Native rerelease bot world (donor `createRereleaseBotWorld` result).
pub struct RereleaseBotWorld<S, O, H> {
    simulation: S,
    knowledge: Rc<BotKnowledge>,
    objectives: O,
    host: H,
    nav: Box<dyn RereleaseNavigation>,
    actor: ActorId,
}

impl<S, O, H> RereleaseBotWorld<S, O, H>
where
    S: RereleaseBotWorldSimulation,
    O: RereleaseBotObjectives,
    H: RereleaseBotWorldHost,
{
    /// Create the world projection for one bot actor.
    pub fn new<N: RereleaseBotNavigation>(
        options: RereleaseWorldOptions<S, O, H, N>,
        actor: ActorId,
    ) -> Result<Self, RereleaseBotWorldError> {
        if options.simulation.bot_numeric().is_none() {
            return Err(RereleaseBotWorldError::MissingNumeric);
        }
        let Some(player) = options.simulation.movement_player(&actor) else {
            return Err(RereleaseBotWorldError::MissingPlayer);
        };
        let nav = options.navigation.for_client(player.client_slot);
        Ok(Self {
            simulation: options.simulation,
            knowledge: options.knowledge,
            objectives: options.objectives,
            host: options.host,
            nav,
            actor,
        })
    }

    fn player(&self) -> RereleaseBotPlayer {
        self.simulation
            .movement_player(&self.actor)
            .expect("Native bot lost shared player")
    }

    fn trace(&self, start: Vec3, end: Vec3, bounds: Option<Bounds>) -> BotTraceT {
        let result = self.simulation.bot_trace(start, end, bounds, &self.actor);
        BotTraceT {
            fraction: result.fraction,
            endpos: result.end,
            startsolid: result.start_solid,
            hit_id: result.hit_actor.as_ref().map_or(-1, |actor| self.host.identify(actor)),
        }
    }
}

impl<S, O, H> BotWorldT for RereleaseBotWorld<S, O, H>
where
    S: RereleaseBotWorldSimulation,
    O: RereleaseBotObjectives,
    H: RereleaseBotWorldHost,
{
    fn time(&self) -> f32 {
        self.simulation.time_seconds()
    }

    fn frame_time(&self) -> f32 {
        self.host.elapsed_ms() as f32 / 1000.0
    }

    fn bot_self(&self) -> BotSelfT {
        let movement = self.player();
        let body = self
            .simulation
            .body_read(&self.actor)
            .expect("Native bot lost shared body");
        let ui = self.simulation.player_ui(&self.actor);
        let ground_class = body
            .ground
            .as_ref()
            .and_then(|ground| self.simulation.bot_entity(ground))
            .map(|entity| entity.classname);
        let mut items = 0;
        let mut current_weapon = 0;
        let mut ammo = std::collections::HashMap::new();
        for weapon in &self.knowledge.weapons {
            let item = native_weapon_item(&self.simulation, &self.actor, &self.knowledge, weapon.entry.number);
            if item
                .as_ref()
                .is_some_and(|item| self.simulation.inventory_count(&self.actor, item) > 0)
            {
                items |= weapon.entry.number;
            }
            if item.as_deref() == ui.active_weapon.as_deref() {
                current_weapon = weapon.entry.number;
            }
            let ammo_name = weapon.entry.ammo_name.clone();
            if !ammo_name.is_empty() {
                let count = ui
                    .inventory
                    .iter()
                    .find(|(item, _)| normalized(item) == normalized(&ammo_name))
                    .map_or(0, |(_, count)| *count);
                ammo.insert(ammo_name, count);
            }
        }
        BotSelfT {
            id: self.host.identify(&self.actor),
            origin: body.origin,
            velocity: body.velocity,
            view_angles: movement.view_angles,
            eye: Vec3 {
                x: body.origin.x,
                y: body.origin.y,
                z: body.origin.z + movement.view_height,
            },
            health: ui.health,
            armor: ui.armor_points,
            items,
            ammo,
            current_weapon,
            on_ground: body.ground.is_some(),
            water_level: movement.water_level,
            air_seconds: None,
            on_lift: Some(matches!(
                ground_class.as_deref(),
                Some("func_plat" | "func_plat2" | "func_train")
            )),
            team: self.objectives.team(&self.actor),
            dead: ui.health <= 0.0,
            has_protection: self
                .simulation
                .combat_read(&self.actor)
                .is_some_and(|combat| combat.invulnerable),
            max_armor: None,
            carrying_objective: self.objectives.carrying(&self.actor),
        }
    }

    fn trace_line(&self, start: Vec3, end: Vec3) -> BotTraceT {
        self.trace(start, end, None)
    }

    fn trace_box(&self, start: Vec3, mins: Vec3, maxs: Vec3, end: Vec3) -> BotTraceT {
        self.trace(start, end, Some(Bounds { min: mins, max: maxs }))
    }

    fn point_contents(&self, point: Vec3) -> i32 {
        let contents = self
            .simulation
            .bot_point_contents(point, &self.actor)
            .expect("Native bot contents projection lost its selected policy");
        if contents & 8 != 0 {
            BotContents::LAVA
        } else if contents & 16 != 0 {
            BotContents::SLIME
        } else if contents & 32 != 0 {
            BotContents::WATER
        } else if contents & 1 != 0 {
            BotContents::SOLID
        } else {
            BotContents::EMPTY
        }
    }

    fn entities(&self) -> Vec<BotEntityT> {
        let mut entities = Vec::new();
        for target in self.simulation.actor_observations() {
            let Some(body) = self.simulation.body_read(&target) else {
                continue;
            };
            if !self.simulation.bodies_linked(&target) {
                continue;
            }
            let movement = self.simulation.movement_player(&target);
            let combat = self.simulation.combat_read(&target);
            let entity = self.simulation.bot_entity(&target);
            if entity.as_ref().is_some_and(|entity| entity.hidden) || (entity.is_none() && movement.is_none()) {
                continue;
            }
            let classname = if movement.is_some() {
                "player".to_string()
            } else {
                entity.as_ref().map_or(String::new(), |entity| entity.classname.clone())
            };
            let kind = if movement.is_some() {
                BotEntityKind::PLAYER
            } else if classname.starts_with("monster_") {
                BotEntityKind::MONSTER
            } else if self.knowledge.item(&classname).is_some() {
                BotEntityKind::ITEM
            } else {
                BotEntityKind::INTERACTABLE
            };
            let health = combat.map_or_else(
                || entity.as_ref().map_or(0.0, |entity| entity.health),
                |combat| combat.health,
            );
            entities.push(BotEntityT {
                id: self.host.identify(&target),
                kind,
                classname,
                origin: body.origin,
                velocity: body.velocity,
                center: Vec3 {
                    x: body.origin.x + (body.bounds.min.x + body.bounds.max.x) / 2.0,
                    y: body.origin.y + (body.bounds.min.y + body.bounds.max.y) / 2.0,
                    z: body.origin.z + (body.bounds.min.z + body.bounds.max.z) / 2.0,
                },
                head: Vec3 {
                    x: body.origin.x,
                    y: body.origin.y,
                    z: body.origin.z + body.bounds.max.z,
                },
                feet: Vec3 {
                    x: body.origin.x,
                    y: body.origin.y,
                    z: body.origin.z + body.bounds.min.z,
                },
                health,
                team: self.objectives.team(&target),
                dead: combat.is_some_and(|combat| combat.health <= 0.0),
                invisible: false,
                water_level: movement.map_or(0, |movement| movement.water_level),
                is_bot: self.host.is_bot(&target),
                spawnflags: entity.as_ref().map_or(0, |entity| entity.spawnflags),
                has_health: health > 0.0,
                has_targetname: entity.as_ref().is_some_and(|entity| !entity.targetname.is_empty()),
                carrying_objective: self.objectives.carrying(&target),
            });
        }
        entities
    }

    fn hearing(&self) -> Vec<BotSoundT> {
        self.host.sounds()
    }

    fn nav(&mut self) -> Option<&mut dyn RereleaseNavigation> {
        Some(&mut *self.nav)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_bots::behavior::rerelease::nav::BotTransportStep;
    use qa_bots::behavior::rerelease::nav::{NavGraphLinkT, NavGraphNodeT, NavPathT, NavPlanOptions};
    use qa_core::identity::IdentityOwner;
    use qa_core::numeric::Q3_BINARY32_PROFILE;
    use std::cell::RefCell;
    use std::collections::HashMap;

    struct StubNav;

    impl RereleaseNavigation for StubNav {
        fn node_count(&self) -> usize {
            0
        }
        fn nodes(&self) -> Vec<NavGraphNodeT> {
            Vec::new()
        }
        fn plan_path(&mut self, _start: Vec3, _goal: Vec3, _options: &NavPlanOptions) -> Option<NavPathT> {
            None
        }
        fn path_valid(&mut self, _path: &NavPathT) -> bool {
            false
        }
        fn transport(&mut self, _link: &NavGraphLinkT, _origin: Vec3) -> Option<BotTransportStep> {
            None
        }
    }

    struct StubNavigation;

    impl RereleaseBotNavigation for StubNavigation {
        fn for_client(&self, _slot: u32) -> Box<dyn RereleaseNavigation> {
            Box::new(StubNav)
        }
    }

    struct StubObjectives;

    impl RereleaseBotObjectives for StubObjectives {
        fn admit(&self, _actor: &ActorId) {}
        fn mode(&self) -> BotGameModeT {
            BotGameModeT {
                game_type: "dm".to_string(),
                weapon_stay: false,
                has_teams: Some(false),
                team_damage: Some(false),
            }
        }
        fn team(&self, _actor: &ActorId) -> i32 {
            0
        }
        fn carrying(&self, _actor: &ActorId) -> bool {
            false
        }
        fn goal(&self, _actor: &ActorId) -> Option<Vec3> {
            None
        }
    }

    struct StubHost {
        next: RefCell<i32>,
    }

    impl RereleaseBotWorldHost for StubHost {
        fn is_bot(&self, _actor: &ActorId) -> bool {
            true
        }
        fn identify(&self, _actor: &ActorId) -> i32 {
            let mut next = self.next.borrow_mut();
            let id = *next;
            *next += 1;
            id
        }
        fn elapsed_ms(&self) -> u64 {
            16
        }
        fn sounds(&self) -> Vec<BotSoundT> {
            Vec::new()
        }
    }

    struct StubSimulation {
        actors: Vec<ActorId>,
        players: HashMap<ActorId, RereleaseBotPlayer>,
        bodies: HashMap<ActorId, RereleaseBotBody>,
        ui: HashMap<ActorId, RereleaseBotUi>,
    }

    impl RereleaseBotWorldSimulation for StubSimulation {
        fn bot_numeric(&self) -> Option<NumericProfile> {
            Some(Q3_BINARY32_PROFILE)
        }
        fn time_seconds(&self) -> f32 {
            1.5
        }
        fn movement_player(&self, actor: &ActorId) -> Option<RereleaseBotPlayer> {
            self.players.get(actor).copied()
        }
        fn body_read(&self, actor: &ActorId) -> Option<RereleaseBotBody> {
            self.bodies.get(actor).cloned()
        }
        fn bodies_linked(&self, actor: &ActorId) -> bool {
            self.bodies.contains_key(actor)
        }
        fn player_ui(&self, actor: &ActorId) -> RereleaseBotUi {
            self.ui.get(actor).cloned().unwrap()
        }
        fn bot_entity(&self, _actor: &ActorId) -> Option<RereleaseBotEntity> {
            None
        }
        fn combat_read(&self, _actor: &ActorId) -> Option<RereleaseBotCombat> {
            None
        }
        fn inventory_count(&self, _actor: &ActorId, _item: &str) -> i32 {
            0
        }
        fn actor_observations(&self) -> Vec<ActorId> {
            self.actors.clone()
        }
        fn bot_trace(
            &self,
            _start: Vec3,
            end: Vec3,
            _bounds: Option<Bounds>,
            _pass_actor: &ActorId,
        ) -> RereleaseBotTrace {
            RereleaseBotTrace {
                fraction: 1.0,
                end,
                start_solid: false,
                hit_actor: None,
            }
        }
        fn bot_point_contents(&self, _point: Vec3, _pass_actor: &ActorId) -> Option<i32> {
            Some(0)
        }
    }

    fn fixture() -> (IdentityOwner, ActorId, StubSimulation) {
        let owner = IdentityOwner::create("test").unwrap();
        let actor = owner.actor(1, 1);
        let mut players = HashMap::new();
        players.insert(
            actor.clone(),
            RereleaseBotPlayer {
                client_slot: 0,
                view_angles: Vec3::default(),
                view_height: 22.0,
                water_level: 0,
            },
        );
        let mut bodies = HashMap::new();
        bodies.insert(
            actor.clone(),
            RereleaseBotBody {
                origin: Vec3 {
                    x: 10.0,
                    y: 20.0,
                    z: 30.0,
                },
                velocity: Vec3::default(),
                bounds: Bounds {
                    min: Vec3 {
                        x: -16.0,
                        y: -16.0,
                        z: -24.0,
                    },
                    max: Vec3 {
                        x: 16.0,
                        y: 16.0,
                        z: 32.0,
                    },
                },
                ground: None,
            },
        );
        let mut ui = HashMap::new();
        ui.insert(
            actor.clone(),
            RereleaseBotUi {
                items: Vec::new(),
                active_weapon: None,
                inventory: Vec::new(),
                health: 100.0,
                armor_points: 0.0,
            },
        );
        (
            owner,
            actor.clone(),
            StubSimulation {
                actors: vec![actor],
                players,
                bodies,
                ui,
            },
        )
    }

    fn world(
        sim: StubSimulation,
        actor: ActorId,
    ) -> RereleaseBotWorld<Rc<StubSimulation>, Rc<StubObjectives>, Rc<StubHost>> {
        RereleaseBotWorld::new(
            RereleaseWorldOptions {
                simulation: Rc::new(sim),
                navigation: StubNavigation,
                knowledge: Rc::new(BotKnowledge::default()),
                objectives: Rc::new(StubObjectives),
                host: Rc::new(StubHost { next: RefCell::new(1) }),
            },
            actor,
        )
        .unwrap()
    }

    #[test]
    fn self_projects_origin_eye_and_time() {
        let (_owner, actor, sim) = fixture();
        let world = world(sim, actor);
        assert_eq!(world.time(), 1.5);
        assert_eq!(world.frame_time(), 0.016);
        let me = world.bot_self();
        assert_eq!(
            me.origin,
            Vec3 {
                x: 10.0,
                y: 20.0,
                z: 30.0
            }
        );
        assert_eq!(
            me.eye,
            Vec3 {
                x: 10.0,
                y: 20.0,
                z: 52.0
            }
        );
        assert!(!me.on_ground);
        assert!(!me.dead);
        assert_eq!(me.on_lift, Some(false));
    }

    #[test]
    fn entities_observe_players() {
        let (_owner, actor, sim) = fixture();
        let world = world(sim, actor);
        let entities = world.entities();
        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0].kind, BotEntityKind::PLAYER);
        assert_eq!(entities[0].classname, "player");
    }

    #[test]
    fn traces_and_contents_answer() {
        let (_owner, actor, sim) = fixture();
        let world = world(sim, actor);
        let trace = world.trace_line(Vec3::default(), Vec3 { x: 1.0, y: 0.0, z: 0.0 });
        assert_eq!(trace.fraction, 1.0);
        assert_eq!(trace.hit_id, -1);
        assert_eq!(world.point_contents(Vec3::default()), BotContents::EMPTY);
    }

    #[test]
    fn normalized_strips_provider_and_weapon_prefix() {
        assert_eq!(normalized("q2:weapon/shotgun"), "shotgun");
        assert_eq!(normalized("weapon_shotgun"), "shotgun");
        assert_eq!(
            normalized("q1:ammo/shells"),
            "q1:ammo/shells".rsplit(':').next().unwrap().replace(['_', '/'], "")
        );
    }
}
