//! Q2 movers (`src/content/q2/foundation/movers.ts`).
//!
//! Doors, buttons, trains and rotating brushes from Quake II game/g_func.c.

use std::collections::{HashMap, HashSet};

use qa_core::identity::{ActorId, SavedActorId};
use qa_core::math::{add3, dot3, length3, normalize3, scale3, sub3, vec3, Vec3};

use super::angular_motion::{
    angular_motion_callbacks, angular_move_to, capture_angular_motion, restore_angular_motion,
    Q2AngularMotionCheckpoint,
};
use super::callbacks::Q2CallbackDefinitions;
use super::checkpoint::restore_q2_actor;
use super::fields::{integer_field, movedir, number_field};
use super::host::{
    Q2Die, Q2Edition, Q2EffectEvent, Q2GameServices, Q2ItemNameFn, Q2Mode, Q2MotionKind, Q2PresentationEvent, Q2Solid,
    Q2SoundEvent, Q2SoundLoop, Q2SpawnFn, SpawnModule,
};
use super::motion::{
    capture_linear_motion, linear_motion_callbacks, linear_move_destination, linear_move_to, restore_linear_motion,
    LinearMotionScope, Q2LinearMotionCheckpoint,
};
use crate::q2::support::contracts::{CombatTraitChanges, DeathReaction, TouchContact};

/// Mover path hook (`Q2MoverHooks` entry).
pub type Q2MoverPathHook = fn(ActorId, &mut Q2GameServices, ActorId);

/// Mover hooks (`Q2MoverHooks`).
#[derive(Debug, Clone, Copy)]
pub struct Q2MoverHooks {
    /// Path corner touch.
    pub path_corner: Q2MoverPathHook,
    /// Combat point touch.
    pub combat_point: Q2MoverPathHook,
}

/// Door phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoorPhase {
    /// At the bottom.
    Bottom,
    /// Moving up.
    Up,
    /// At the top.
    Top,
    /// Moving down.
    Down,
}

/// Door state (`DoorState`).
#[derive(Debug, Clone)]
pub struct DoorState {
    /// Start position.
    pub start: Vec3,
    /// End position.
    pub end: Vec3,
    /// Travel distance.
    pub distance: f64,
    /// Whether a button.
    pub button: bool,
    /// Whether angular.
    pub angular: bool,
    /// Whether water.
    pub water: bool,
    /// Safe-open direction.
    pub safe_direction: Vec3,
    /// Smart-water divisor.
    pub water_divisor: f64,
    /// Whether reversed.
    pub reversed: bool,
    /// Whether activated.
    pub activated: bool,
    /// Current phase.
    pub phase: DoorPhase,
    /// Team master.
    pub master: ActorId,
    /// Team members.
    pub team: Vec<ActorId>,
    /// Debounce time.
    pub debounce: f64,
}

/// Train state (`TrainState`).
#[derive(Debug, Clone)]
pub struct TrainState {
    /// Destination corner.
    pub destination: Option<ActorId>,
    /// Debounce time.
    pub debounce: f64,
    /// Whether a ship.
    pub ship: bool,
}

/// Train route stop (`Q2TrainRoute` stop).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2TrainStop {
    /// Corner actor.
    pub actor: ActorId,
    /// Destination origin.
    pub origin: Vec3,
    /// Next corner.
    pub next: Option<ActorId>,
    /// Wait seconds.
    pub wait: f64,
    /// Whether a teleport corner.
    pub teleport: bool,
}

/// Train route (`Q2TrainRoute`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2TrainRoute {
    /// Whether running.
    pub running: bool,
    /// Destination corner.
    pub destination: Option<ActorId>,
    /// Route stops.
    pub stops: Vec<Q2TrainStop>,
}

/// Mover traversal (`traversal` result).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2MoverTraversal {
    /// Whether locked.
    pub locked: bool,
    /// Linear destination.
    pub destination: Option<Vec3>,
}

/// Door checkpoint state (door state without master and team).
#[derive(Debug, Clone, PartialEq)]
pub struct DoorCheckpointState {
    /// Start position.
    pub start: Vec3,
    /// End position.
    pub end: Vec3,
    /// Travel distance.
    pub distance: f64,
    /// Whether a button.
    pub button: bool,
    /// Whether angular.
    pub angular: bool,
    /// Whether water.
    pub water: bool,
    /// Safe-open direction.
    pub safe_direction: Vec3,
    /// Smart-water divisor.
    pub water_divisor: f64,
    /// Whether reversed.
    pub reversed: bool,
    /// Whether activated.
    pub activated: bool,
    /// Current phase.
    pub phase: DoorPhase,
    /// Debounce time.
    pub debounce: f64,
}

/// Door checkpoint entry.
#[derive(Debug, Clone, PartialEq)]
pub struct DoorCheckpoint {
    /// Door actor.
    pub actor: SavedActorId,
    /// Door state.
    pub state: DoorCheckpointState,
    /// Team master.
    pub master: SavedActorId,
    /// Team members.
    pub team: Vec<SavedActorId>,
}

/// Train checkpoint entry.
#[derive(Debug, Clone, PartialEq)]
pub struct TrainCheckpoint {
    /// Train actor.
    pub actor: SavedActorId,
    /// Destination corner.
    pub destination: Option<SavedActorId>,
    /// Debounce time.
    pub debounce: f64,
    /// Whether a ship.
    pub ship: bool,
}

/// Movers checkpoint (`Q2MoversCheckpoint`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Q2MoversCheckpoint {
    /// Door entries.
    pub doors: Vec<DoorCheckpoint>,
    /// Train entries.
    pub trains: Vec<TrainCheckpoint>,
    /// Linear motion.
    pub linear: Q2LinearMotionCheckpoint,
    /// Angular motion.
    pub angular: Q2AngularMotionCheckpoint,
}

/// Arena runtime state for this module.
#[derive(Debug, Default)]
pub struct MoverRuntime {
    /// Active angular moves by actor.
    pub angular_moves: HashMap<ActorId, super::angular_motion::AngularMoveState>,
    /// Active linear moves by actor.
    pub linear_moves: HashMap<ActorId, super::motion::LinearMoveState>,
    /// Door states by actor.
    pub doors: HashMap<ActorId, DoorState>,
    /// Train states by actor.
    pub trains: HashMap<ActorId, TrainState>,
    /// Registered mover hooks.
    pub hooks: Option<Q2MoverHooks>,
}

impl MoverRuntime {
    /// Clean arena state after any actor release.
    pub fn on_actor_released(&mut self, actor: &ActorId) {
        self.doors.remove(actor);
        self.trains.remove(actor);
        self.angular_moves.remove(actor);
        self.linear_moves.remove(actor);
    }
}

/// Q2 mover module (`Q2MoverModule`).
#[derive(Debug, Clone, Copy)]
pub struct Q2MoverModule {
    /// Path hooks.
    hooks: Q2MoverHooks,
}

/// Create the Q2 mover module (`createQ2MoverModule`).
pub fn create_q2_mover_module(hooks: Q2MoverHooks) -> Q2MoverModule {
    Q2MoverModule { hooks }
}

/// Mover item-name handler (unused).
fn mover_item_name(_classname: &str) -> Option<String> {
    None
}

impl Q2MoverModule {
    /// Register hooks and build the spawn module.
    pub fn register(&self, game: &mut Q2GameServices) -> SpawnModule {
        game.movers.hooks = Some(self.hooks);
        let mut callbacks = Q2CallbackDefinitions::default();
        let linear = linear_motion_callbacks();
        callbacks.think.extend(linear.think);
        callbacks.use_.extend(linear.use_);
        callbacks.touch.extend(linear.touch);
        callbacks.pain.extend(linear.pain);
        callbacks.die.extend(linear.die);
        callbacks.blocked.extend(linear.blocked);
        callbacks.trajectory.extend(linear.trajectory);
        let angular = angular_motion_callbacks();
        callbacks.think.extend(angular.think);
        callbacks.use_.extend(angular.use_);
        callbacks.touch.extend(angular.touch);
        callbacks.pain.extend(angular.pain);
        callbacks.die.extend(angular.die);
        callbacks.blocked.extend(angular.blocked);
        callbacks.trajectory.extend(angular.trajectory);
        callbacks.think.insert("door_hit_bottom", door_hit_bottom);
        callbacks.think.insert("door_go_down", door_go_down);
        callbacks.think.insert("door_hit_top", door_hit_top);
        callbacks.think.insert("Think_SpawnDoorTrigger", prepare_door);
        callbacks.think.insert("smart_water_go_up", smart_water);
        callbacks.think.insert("train_wait", train_wait);
        callbacks.think.insert("train_next", train_next);
        callbacks.think.insert("train_piece_wait", train_piece_wait);
        callbacks.think.insert("func_train_find", train_find);
        callbacks.use_.insert("door_use", door_use);
        callbacks.use_.insert("Door_Activate", door_activate);
        callbacks.use_.insert("train_use", train_use);
        callbacks.use_.insert("rotating_use", rotating_use);
        callbacks.touch.insert("button_touch", button_touch);
        callbacks.touch.insert("door_touch", door_touch);
        callbacks.touch.insert("Touch_DoorTrigger", door_trigger_touch);
        callbacks.touch.insert("rotating_touch", rotating_touch);
        callbacks.touch.insert("q2_path_touch", path_touch);
        let die: Q2Die = door_killed;
        callbacks.die.insert("door_killed", die);
        callbacks.blocked.insert("door_blocked", door_blocked);
        callbacks.blocked.insert("smart_water_blocked", smart_water_blocked);
        callbacks.blocked.insert("train_blocked", train_blocked);
        callbacks.blocked.insert("rotating_blocked", rotating_damage);
        let spawn: Q2SpawnFn = spawn_mover;
        let item_name: Q2ItemNameFn = mover_item_name;
        SpawnModule {
            spawn,
            item_name,
            callbacks,
        }
    }

    /// Observe the deterministic source route without advancing callbacks or
    /// consuming target-selection randomness (`trainRoute`).
    pub fn train_route(&self, this: ActorId, game: &mut Q2GameServices) -> Option<Q2TrainRoute> {
        let state = game.movers.trains.get(&this)?.clone();
        if state.ship {
            return None;
        }
        let mut targets: HashMap<String, Option<ActorId>> = HashMap::new();
        for (actor, candidate) in &game.entities {
            if candidate.targetname.is_empty() {
                continue;
            }
            let ambiguous = targets.contains_key(&candidate.targetname) || candidate.classname != "path_corner";
            targets.insert(
                candidate.targetname.clone(),
                if ambiguous { None } else { Some(actor.clone()) },
            );
        }
        let entity = game.require_entity(&this).clone();
        let start = entity
            .spawn
            .values
            .get("target")
            .cloned()
            .unwrap_or(entity.target.clone());
        let mut corner = targets.get(&start).cloned().flatten();
        let mut stops = Vec::new();
        let mut visited = HashSet::new();
        while let Some(corner_id) = corner.clone() {
            if visited.contains(&corner_id) {
                break;
            }
            visited.insert(corner_id.clone());
            let corner_entity = game.require_entity(&corner_id).clone();
            let next = targets.get(&corner_entity.target).cloned().flatten();
            if !corner_entity.target.is_empty() && next.is_none() {
                return None;
            }
            stops.push(Q2TrainStop {
                actor: corner_id.clone(),
                origin: train_destination(this.clone(), corner_id.clone(), game),
                next: next.clone(),
                wait: corner_entity.wait,
                teleport: corner_entity.spawnflags & 1 != 0,
            });
            corner = next;
        }
        if let Some(destination) = state.destination.clone() {
            if !visited.contains(&destination) {
                return None;
            }
        }
        Some(Q2TrainRoute {
            running: game.require_entity(&this).spawnflags & 1 != 0,
            destination: state.destination,
            stops,
        })
    }

    /// Door traversal state (`traversal`).
    #[allow(unpredictable_function_pointer_comparisons)]
    pub fn traversal(&self, this: ActorId, game: &mut Q2GameServices) -> Q2MoverTraversal {
        let master = game
            .movers
            .doors
            .get(&this)
            .map(|state| state.master.clone())
            .unwrap_or_else(|| this.clone());
        let door = game.movers.doors.get(&master).cloned();
        let locked = match door {
            Some(door) if door.phase == DoorPhase::Bottom || door.phase == DoorPhase::Down => {
                let master_entity = game.require_entity(&master).clone();
                master_entity.use_ == Some(door_activate)
                    || !door.button
                        && (master_entity.max_health > 0.0 || !master_entity.targetname.is_empty() && !door.activated)
            }
            _ => false,
        };
        Q2MoverTraversal {
            locked,
            destination: linear_move_destination(game, LinearMotionScope::Foundation, &this),
        }
    }

    /// Capture mover state (`capture`).
    pub fn capture(&self, game: &mut Q2GameServices) -> Q2MoversCheckpoint {
        let mut doors = Vec::new();
        let mut trains = Vec::new();
        let actors: Vec<ActorId> = game.entities.keys().cloned().collect();
        for actor in actors {
            if let Some(door) = game.movers.doors.get(&actor).cloned() {
                doors.push(DoorCheckpoint {
                    actor: SavedActorId::from(&actor),
                    state: DoorCheckpointState {
                        start: door.start,
                        end: door.end,
                        distance: door.distance,
                        button: door.button,
                        angular: door.angular,
                        water: door.water,
                        safe_direction: door.safe_direction,
                        water_divisor: door.water_divisor,
                        reversed: door.reversed,
                        activated: door.activated,
                        phase: door.phase,
                        debounce: door.debounce,
                    },
                    master: SavedActorId::from(&door.master),
                    team: door.team.iter().map(SavedActorId::from).collect(),
                });
            }
            if let Some(train) = game.movers.trains.get(&actor).cloned() {
                trains.push(TrainCheckpoint {
                    actor: SavedActorId::from(&actor),
                    destination: train.destination.as_ref().map(SavedActorId::from),
                    debounce: train.debounce,
                    ship: train.ship,
                });
            }
        }
        Q2MoversCheckpoint {
            doors,
            trains,
            linear: capture_linear_motion(game, LinearMotionScope::Foundation),
            angular: capture_angular_motion(game),
        }
    }

    /// Restore mover state (`restore`).
    pub fn restore(&self, game: &mut Q2GameServices, checkpoint: &Q2MoversCheckpoint) {
        game.movers.doors = HashMap::new();
        game.movers.trains = HashMap::new();
        for saved in &checkpoint.doors {
            let actor = restore_mover_actor(game, saved.actor.clone());
            let master = restore_mover_actor(game, saved.master.clone());
            let team = saved
                .team
                .iter()
                .map(|member| restore_mover_actor(game, member.clone()))
                .collect();
            game.movers.doors.insert(
                actor,
                DoorState {
                    start: saved.state.start,
                    end: saved.state.end,
                    distance: saved.state.distance,
                    button: saved.state.button,
                    angular: saved.state.angular,
                    water: saved.state.water,
                    safe_direction: saved.state.safe_direction,
                    water_divisor: saved.state.water_divisor,
                    reversed: saved.state.reversed,
                    activated: saved.state.activated,
                    phase: saved.state.phase,
                    master,
                    team,
                    debounce: saved.state.debounce,
                },
            );
        }
        for saved in &checkpoint.trains {
            let actor = restore_mover_actor(game, saved.actor.clone());
            let destination = saved
                .destination
                .as_ref()
                .map(|destination| restore_mover_actor(game, destination.clone()));
            game.movers.trains.insert(
                actor,
                TrainState {
                    destination,
                    debounce: saved.debounce,
                    ship: saved.ship,
                },
            );
        }
        restore_linear_motion(game, LinearMotionScope::Foundation, &checkpoint.linear);
        restore_angular_motion(game, &checkpoint.angular);
    }

    /// Resume a train at a corner (`resumeTrainAt`).
    pub fn resume_train_at(&self, this: ActorId, game: &mut Q2GameServices, corner: ActorId) {
        train_state_mut(game, &this).destination = Some(corner.clone());
        game.require_entity_mut(&this).spawnflags |= 1;
        let destination = train_destination(this.clone(), corner, game);
        linear_move_to(game, LinearMotionScope::Foundation, this, destination, train_wait);
    }

    /// Train travel direction (`getTrainDirection`).
    pub fn train_direction(&self, this: ActorId, game: &mut Q2GameServices) -> Vec3 {
        let Some(destination) = train_state(game, &this).destination.clone() else {
            return vec3(0.0, 0.0, 0.0);
        };
        let delta = sub3(
            train_destination(this.clone(), destination, game),
            game.body_of(this).origin,
        );
        let distance = length3(delta);
        if distance == 0.0 {
            vec3(0.0, 0.0, 0.0)
        } else {
            scale3(delta, 1.0 / distance)
        }
    }

    /// Spawn a train (`spawnTrain`).
    pub fn spawn_train(&self, this: ActorId, game: &mut Q2GameServices) {
        spawn_train(this, game);
    }
}

/// Resolve a checkpoint actor to a live mover entity.
fn restore_mover_actor(game: &mut Q2GameServices, saved: SavedActorId) -> ActorId {
    let owned = restore_q2_actor(game, saved);
    if game.entity(owned.id()).is_none() {
        panic!("Q2 mover checkpoint has no source actor");
    }
    owned.id().clone()
}

/// Read door state, panicking when absent.
fn door_state(game: &Q2GameServices, actor: &ActorId) -> DoorState {
    game.movers
        .doors
        .get(actor)
        .cloned()
        .unwrap_or_else(|| panic!("Missing Q2 door state"))
}

/// Read train state, panicking when absent.
fn train_state(game: &Q2GameServices, actor: &ActorId) -> TrainState {
    game.movers
        .trains
        .get(actor)
        .cloned()
        .unwrap_or_else(|| panic!("Missing Q2 train state"))
}

/// Mutably read train state, panicking when absent.
fn train_state_mut<'game>(game: &'game mut Q2GameServices, actor: &ActorId) -> &'game mut TrainState {
    game.movers
        .trains
        .get_mut(actor)
        .unwrap_or_else(|| panic!("Missing Q2 train state"))
}

/// Mutably read door state, panicking when absent.
fn door_state_mut<'game>(game: &'game mut Q2GameServices, actor: &ActorId) -> &'game mut DoorState {
    game.movers
        .doors
        .get_mut(actor)
        .unwrap_or_else(|| panic!("Missing Q2 door state"))
}

/// Mover loop sound (`loop`).
fn mover_loop(this: ActorId, game: &mut Q2GameServices, path: &str, start: bool) {
    if path.is_empty() {
        return;
    }
    let origin = game.body_of(this.clone()).origin;
    game.host_emit(Q2PresentationEvent::Sound(Q2SoundEvent {
        actor: Some(this),
        origin,
        path: path.to_string(),
        channel: 2,
        volume: 1.0,
        attenuation: 3.0,
        reliable: false,
        loop_: if start { Q2SoundLoop::Start } else { Q2SoundLoop::Stop },
        loop_owner: None,
    }));
}

/// Door sound (`doorSound`).
fn door_sound(this: ActorId, game: &mut Q2GameServices, start: bool) {
    let state = door_state(game, &this);
    let entity = game.require_entity(&this).clone();
    let sounds = integer_field(&entity.spawn, "sounds", 0);
    let enabled = if state.water {
        sounds == 1 || sounds == 2
    } else {
        sounds != 1
    };
    let defaults: [&str; 3] = if state.water {
        ["world/mov_watr.wav", "", "world/stp_watr.wav"]
    } else if state.button {
        ["switches/butn2.wav", "", ""]
    } else {
        ["doors/dr1_strt.wav", "doors/dr1_mid.wav", "doors/dr1_end.wav"]
    };
    let select = |key: &str, index: usize| -> String {
        let value = if game.options.edition == Q2Edition::Rerelease {
            entity.spawn.values.get(key).cloned()
        } else {
            None
        };
        match value {
            None => {
                if enabled {
                    defaults[index].to_string()
                } else {
                    String::new()
                }
            }
            Some(value) if value == "0" || value == " " => String::new(),
            Some(value) => value,
        }
    };
    let sound = select(
        if start { "noise_start" } else { "noise_end" },
        if start { 0 } else { 2 },
    );
    let middle = select("noise_middle", 1);
    let attenuation = if game.options.edition == Q2Edition::Rerelease && !state.water {
        number_field(&entity.spawn, "attenuation", 3.0)
    } else {
        3.0
    };
    if state.master == this && !sound.is_empty() {
        let mut origin = game.body_of(this.clone()).origin;
        if game.options.edition == Q2Edition::Rerelease && state.team.len() > 1 {
            let sum = state.team.iter().fold(vec3(0.0, 0.0, 0.0), |sum, member| {
                let body = game.body_of(member.clone());
                add3(
                    sum,
                    add3(body.origin, scale3(add3(body.bounds.min, body.bounds.max), 0.5)),
                )
            });
            let center = scale3(sum, 1.0 / state.team.len() as f32);
            if game.host.point_contents(center) & 1 == 0 {
                origin = center;
            }
        }
        game.host_emit(Q2PresentationEvent::Sound(Q2SoundEvent {
            actor: Some(this.clone()),
            origin,
            path: sound,
            channel: 2,
            volume: 1.0,
            attenuation: if attenuation == -1.0 { 0.0 } else { attenuation },
            reliable: false,
            loop_: Q2SoundLoop::Once,
            loop_owner: None,
        }));
    }
    if !middle.is_empty() {
        let origin = game.body_of(this.clone()).origin;
        game.host_emit(Q2PresentationEvent::Sound(Q2SoundEvent {
            actor: Some(this),
            origin,
            path: middle,
            channel: 2,
            volume: 1.0,
            attenuation: if attenuation == -1.0 { 0.0 } else { attenuation },
            reliable: false,
            loop_: if start { Q2SoundLoop::Start } else { Q2SoundLoop::Stop },
            loop_owner: None,
        }));
    }
}

/// Door area portals (`portals`).
fn door_portals(this: ActorId, game: &mut Q2GameServices, open: bool) {
    let target = game.require_entity(&this).target.clone();
    for portal in game.targets(&target) {
        let entity = game.require_entity(&portal).clone();
        if entity.classname == "func_areaportal" {
            let style = integer_field(&entity.spawn, "style", 0);
            game.host.set_area_portal(style, open);
        }
    }
}

/// Door hit bottom (`bottom`).
fn door_hit_bottom(this: ActorId, game: &mut Q2GameServices) {
    door_state_mut(game, &this).phase = DoorPhase::Bottom;
    let state = door_state(game, &this);
    if state.button {
        let entity = game.require_entity_mut(&this);
        entity.effects = entity.effects & !0x800 | 0x400;
        game.show(this);
    } else {
        door_sound(this.clone(), game, false);
        if game.options.edition == Q2Edition::Classic || game.require_entity(&this).spawnflags & 1 == 0 {
            door_portals(this, game, false);
        }
    }
}

/// Door go down (`down`).
fn door_go_down(this: ActorId, game: &mut Q2GameServices) {
    door_state_mut(game, &this).phase = DoorPhase::Down;
    let state = door_state(game, &this);
    let entity = game.require_entity(&this).clone();
    if entity.max_health > 0.0 && !state.water {
        let owned = game.owned_of(this.clone());
        game.host.combat().set_health(&owned, entity.max_health);
        game.host.combat().set_traits(
            &owned,
            &CombatTraitChanges {
                can_take_damage: Some(true),
                ..CombatTraitChanges::default()
            },
        );
    }
    if state.button {
        game.require_entity_mut(&this).frame = 0;
        game.show(this.clone());
    } else {
        door_sound(this.clone(), game, true);
    }
    if state.angular {
        angular_move_to(game, this.clone(), state.start, door_hit_bottom);
    } else {
        linear_move_to(
            game,
            LinearMotionScope::Foundation,
            this.clone(),
            state.start,
            door_hit_bottom,
        );
    }
    if game.options.edition == Q2Edition::Rerelease && !state.button && game.require_entity(&this).spawnflags & 1 != 0 {
        door_portals(this, game, true);
    }
}

/// Door hit top (`top`).
fn door_hit_top(this: ActorId, game: &mut Q2GameServices) {
    door_state_mut(game, &this).phase = DoorPhase::Top;
    let state = door_state(game, &this);
    if state.button {
        let entity = game.require_entity_mut(&this);
        entity.effects = entity.effects & !0x400 | 0x800;
        entity.frame = 1;
        game.show(this.clone());
        let authored = game.require_entity(&this).authored_target();
        let activator = game.require_entity(&this).activator.clone();
        game.use_targets(&authored, activator.as_ref(), false);
        if !game.host.actors().is_live(&this) {
            return;
        }
    } else {
        door_sound(this.clone(), game, false);
        if game.require_entity(&this).spawnflags & 32 != 0 {
            return;
        }
    }
    if game.require_entity(&this).wait >= 0.0 {
        let wait = game.require_entity(&this).wait;
        game.schedule(this.clone(), wait, door_go_down);
    }
    if game.options.edition == Q2Edition::Rerelease && !state.button && game.require_entity(&this).spawnflags & 1 != 0 {
        door_portals(this, game, false);
    }
}

/// Door go up (`up`).
fn door_up(this: ActorId, game: &mut Q2GameServices, activator: Option<ActorId>) {
    let state = door_state(game, &this);
    if state.phase == DoorPhase::Up {
        return;
    }
    if state.phase == DoorPhase::Top {
        let entity = game.require_entity(&this).clone();
        if !state.button && entity.wait >= 0.0 && entity.spawnflags & 32 == 0 {
            game.schedule(this, entity.wait, door_go_down);
        }
        return;
    }
    game.require_entity_mut(&this).activator = activator.clone();
    door_state_mut(game, &this).phase = DoorPhase::Up;
    door_sound(this.clone(), game, true);
    let destination = if state.reversed {
        scale3(state.end, -1.0)
    } else {
        state.end
    };
    if state.angular {
        angular_move_to(game, this.clone(), destination, door_hit_top);
    } else {
        linear_move_to(
            game,
            LinearMotionScope::Foundation,
            this.clone(),
            destination,
            door_hit_top,
        );
    }
    if !state.button {
        let authored = game.require_entity(&this).authored_target();
        game.use_targets(&authored, activator.as_ref(), false);
        if game.options.edition == Q2Edition::Classic || game.require_entity(&this).spawnflags & 1 == 0 {
            door_portals(this, game, true);
        }
    }
}

/// Door use (`use`).
fn mover_use(this: ActorId, game: &mut Q2GameServices, activator: Option<ActorId>) {
    let state = door_state(game, &this);
    if state.master != this {
        return;
    }
    let entity = game.require_entity(&this).clone();
    if game.options.edition == Q2Edition::Rerelease
        && state.angular
        && entity.spawnflags & 0x20000 != 0
        && (state.phase == DoorPhase::Bottom || state.phase == DoorPhase::Down)
    {
        if let Some(activator_id) = activator.clone() {
            if let Some(body) = game.host.bodies().read(&activator_id) {
                let origin = game.body_of(this.clone()).origin;
                let reversed = dot3(normalize3(sub3(body.origin, origin)), state.safe_direction) > 0.0;
                door_state_mut(game, &this).reversed = reversed;
            }
        }
    }
    let state = door_state(game, &this);
    let entity = game.require_entity(&this).clone();
    let close =
        !state.button && entity.spawnflags & 32 != 0 && (state.phase == DoorPhase::Up || state.phase == DoorPhase::Top);
    if !close && game.options.edition == Q2Edition::Rerelease && state.water && entity.spawnflags & 2 != 0 {
        let body = game.body_of(this.clone());
        let center = scale3(add3(body.bounds.min, body.bounds.max), 0.5);
        if game.host.point_contents(center) & 56 != 0 {
            let entity = game.require_entity_mut(&this);
            entity.message = String::new();
            entity.touch = None;
            entity.enemy = activator;
            smart_water(this, game);
            return;
        }
    }
    for member in state.team.clone() {
        let entity = game.require_entity_mut(&member);
        entity.message = String::new();
        entity.touch = None;
        if close {
            door_go_down(member, game);
        } else {
            door_up(member, game, activator.clone());
        }
    }
}

/// Spawn a door (`spawnDoor`).
fn spawn_door(this: ActorId, game: &mut Q2GameServices) {
    let entity = game.require_entity(&this).clone();
    let button = entity.classname == "func_button";
    let angular = entity.classname == "func_door_rotating";
    let water = entity.classname == "func_water";
    let safe_direction = if angular && entity.spawnflags & 0x20000 != 0 {
        movedir(game.body_of(this.clone()).angles)
    } else {
        vec3(0.0, 0.0, 0.0)
    };
    let water_divisor = if entity.accel == 0.0 { 20.0 } else { entity.accel };
    let mut direction = if angular {
        if entity.spawnflags & 64 != 0 {
            vec3(0.0, 0.0, 1.0)
        } else if entity.spawnflags & 128 != 0 {
            vec3(1.0, 0.0, 0.0)
        } else {
            vec3(0.0, 1.0, 0.0)
        }
    } else {
        movedir(game.body_of(this.clone()).angles)
    };
    if angular && entity.spawnflags & 2 != 0 {
        direction = scale3(direction, -1.0);
    }
    game.require_entity_mut(&this).movedir = direction;
    let mut moved = game.body_of(this.clone());
    moved.angles = vec3(0.0, 0.0, 0.0);
    game.write_body(this.clone(), &moved, false);
    game.set_solid(this.clone(), Q2Solid::Brush);
    game.set_motion_kind(
        this.clone(),
        if button { Q2MotionKind::Stop } else { Q2MotionKind::Push },
    );
    let entity = game.require_entity_mut(&this);
    if entity.speed == 0.0 {
        entity.speed = if button {
            40.0
        } else if water {
            25.0
        } else {
            100.0
        };
    }
    if !button && !angular && !water && game.options.mode == Q2Mode::Deathmatch {
        let entity = game.require_entity_mut(&this);
        entity.speed *= 2.0;
    }
    let entity = game.require_entity_mut(&this);
    if entity.accel == 0.0 {
        entity.accel = entity.speed;
    }
    if entity.decel == 0.0 {
        entity.decel = entity.speed;
    }
    if entity.wait == 0.0 {
        entity.wait = if water { -1.0 } else { 3.0 };
    }
    if entity.damage == 0.0 {
        entity.damage = 2.0;
    }
    let entity = game.require_entity(&this).clone();
    let body = game.body_of(this.clone());
    let size = sub3(body.bounds.max, body.bounds.min);
    let direction = vec3(entity.movedir.x.abs(), entity.movedir.y.abs(), entity.movedir.z.abs());
    let lip = number_field(&entity.spawn, "lip", 0.0);
    let distance = if angular {
        let authored = number_field(&entity.spawn, "distance", 0.0);
        if authored == 0.0 {
            90.0
        } else {
            authored
        }
    } else {
        f64::from(dot3(direction, size))
            - if lip == 0.0 {
                if button {
                    4.0
                } else if water {
                    0.0
                } else {
                    8.0
                }
            } else {
                lip
            }
    };
    let mut start = if angular { vec3(0.0, 0.0, 0.0) } else { body.origin };
    let mut end = add3(start, scale3(entity.movedir, distance as f32));
    if !button && entity.spawnflags & 1 != 0 {
        if game.options.edition == Q2Edition::Rerelease && angular && entity.spawnflags & 0x20000 != 0 {
            game.require_entity_mut(&this).spawnflags &= !0x20000;
            game.host
                .diagnostic("Q2 rotating door SAFE_OPEN is incompatible with START_OPEN");
        }
        std::mem::swap(&mut start, &mut end);
        let mut moved = game.body_of(this.clone());
        if angular {
            moved.angles = start;
        } else {
            moved.origin = start;
        }
        game.write_body(this.clone(), &moved, true);
        if angular {
            let movedir = game.require_entity(&this).movedir;
            game.require_entity_mut(&this).movedir = scale3(movedir, -1.0);
        }
    }
    game.movers.doors.insert(
        this.clone(),
        DoorState {
            start,
            end,
            distance,
            button,
            angular,
            water,
            safe_direction,
            water_divisor,
            reversed: false,
            activated: false,
            phase: DoorPhase::Bottom,
            master: this.clone(),
            team: vec![this.clone()],
            debounce: 0.0,
        },
    );
    if game.require_entity(&this).spawn.values.get("team").is_none() {
        game.require_entity_mut(&this).team_master = Some(this.clone());
    }
    if water {
        let speed = game.require_entity(&this).speed;
        let entity = game.require_entity_mut(&this);
        entity.accel = speed;
        entity.decel = speed;
        if entity.wait == -1.0 {
            entity.spawnflags |= 32;
        }
        if game.options.edition == Q2Edition::Classic {
            game.require_entity_mut(&this).classname = "func_door".to_string();
        }
    }
    if button {
        game.require_entity_mut(&this).effects |= 0x400;
    } else if !water {
        if game.require_entity(&this).spawnflags & 16 != 0 {
            game.require_entity_mut(&this).effects |= 0x1000;
        }
        if !angular && game.require_entity(&this).spawnflags & 64 != 0 {
            game.require_entity_mut(&this).effects |= 0x2000;
        }
    }
    game.require_entity_mut(&this).use_ = Some(door_use);
    let entity = game.require_entity(&this).clone();
    if entity.max_health > 0.0 && !water {
        let owned = game.owned_of(this.clone());
        game.create_combat(&owned, entity.max_health, 0.0, true);
        game.require_entity_mut(&this).die = Some(door_killed);
    } else if button && entity.targetname.is_empty() {
        game.require_entity_mut(&this).touch = Some(button_touch);
    } else if !button && !entity.targetname.is_empty() && !entity.message.is_empty() {
        game.require_entity_mut(&this).touch = Some(door_touch);
    }
    game.require_entity_mut(&this).blocked = Some(door_blocked);
    game.show(this.clone());
    if !button && !water {
        let frame_seconds = game.host.frame_seconds();
        game.schedule(this.clone(), frame_seconds, prepare_door);
    }
    if water {
        game.require_entity_mut(&this).blocked = None;
        if game.options.edition == Q2Edition::Rerelease && game.require_entity(&this).spawnflags & 2 != 0 {
            game.require_entity_mut(&this).blocked = Some(smart_water_blocked);
        }
    }
    if game.options.edition == Q2Edition::Rerelease && angular && game.require_entity(&this).spawnflags & 0x10000 != 0 {
        if game.require_entity(&this).max_health > 0.0 {
            let owned = game.owned_of(this.clone());
            game.host.combat().set_traits(
                &owned,
                &CombatTraitChanges {
                    can_take_damage: Some(false),
                    ..CombatTraitChanges::default()
                },
            );
        }
        game.require_entity_mut(&this).die = None;
        game.cancel_actor(this.clone());
        game.require_entity_mut(&this).use_ = Some(door_activate);
    }
}

/// Prepare a door team (`prepareDoor`).
fn prepare_door(this: ActorId, game: &mut Q2GameServices) {
    let mut team = Vec::new();
    for owned in game.push_team(&this) {
        if game.entity(owned.id()).is_some() && game.movers.doors.contains_key(owned.id()) {
            team.push(owned.id().clone());
        }
    }
    let master = team.first().cloned().unwrap_or_else(|| this.clone());
    for member in &team {
        if let Some(state) = game.movers.doors.get_mut(member) {
            state.master = master.clone();
            state.team.clone_from(&team);
        }
    }
    if master != this {
        return;
    }
    let entity = game.require_entity(&this).clone();
    if game.options.edition == Q2Edition::Rerelease && !door_state(game, &this).angular && entity.spawnflags & 1 != 0 {
        door_portals(this.clone(), game, true);
    }
    let shortest = team.iter().fold(f64::INFINITY, |shortest, member| {
        shortest.min(door_state(game, member).distance.abs())
    });
    let time = shortest / game.require_entity(&this).speed;
    if time > 0.0 {
        for member in &team {
            let speed = door_state(game, member).distance.abs() / time;
            let member_speed = game.require_entity(member).speed;
            let ratio = speed / member_speed;
            let entity = game.require_entity_mut(member);
            entity.accel *= ratio;
            entity.decel *= ratio;
            entity.speed = speed;
        }
    }
    let entity = game.require_entity(&this).clone();
    if entity.max_health > 0.0 || !entity.targetname.is_empty() && !door_state(game, &this).activated {
        return;
    }
    let mut min = vec3(f32::INFINITY, f32::INFINITY, f32::INFINITY);
    let mut max = vec3(f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
    for member in &team {
        let body = game.body_of(member.clone());
        let low = add3(body.origin, body.bounds.min);
        let high = add3(body.origin, body.bounds.max);
        min = vec3(min.x.min(low.x), min.y.min(low.y), min.z.min(low.z));
        max = vec3(max.x.max(high.x), max.y.max(high.y), max.z.max(high.z));
    }
    let trigger = game.create("door_trigger", std::collections::BTreeMap::new());
    game.require_entity_mut(&trigger).owner = Some(this.clone());
    game.require_entity_mut(&trigger).visible = false;
    let mut moved = game.body_of(trigger.clone());
    moved.bounds.min = vec3(min.x - 60.0, min.y - 60.0, min.z);
    moved.bounds.max = vec3(max.x + 60.0, max.y + 60.0, max.z);
    game.write_body(trigger.clone(), &moved, false);
    game.require_entity_mut(&trigger).touch = Some(door_trigger_touch);
    game.set_solid(trigger, Q2Solid::Trigger);
    if game.require_entity(&this).spawnflags & 1 != 0 {
        door_portals(this, game, true);
    }
}

/// Smart water think (`smartWater`).
fn smart_water(this: ActorId, game: &mut Q2GameServices) {
    let state = door_state(game, &this);
    let body = game.body_of(this.clone());
    let top = body.origin.z + body.bounds.max.z + 1.0;
    if state.phase == DoorPhase::Top {
        if game.require_entity(&this).wait >= 0.0 {
            let wait = game.require_entity(&this).wait;
            game.schedule(this, wait, smart_water);
        }
        return;
    }
    let entity = game.require_entity(&this).clone();
    if entity.max_health != 0.0 && f64::from(top) >= entity.max_health {
        let mut moved = game.body_of(this.clone());
        moved.velocity = vec3(0.0, 0.0, 0.0);
        game.write_body(this.clone(), &moved, false);
        let kind = game.require_entity(&this).motion;
        game.set_motion_kind(this.clone(), kind);
        game.cancel_actor(this.clone());
        door_state_mut(game, &this).phase = DoorPhase::Top;
        return;
    }
    door_sound(this.clone(), game, true);
    let mut lowest = None;
    let mut height = 999_999.0f32;
    for actor in game.host.players() {
        let health = game.host.combat().read(&actor).map(|state| state.health).unwrap_or(0.0);
        if health <= 0.0 {
            continue;
        }
        if let Some(player) = game.host.bodies().read(&actor) {
            let feet = player.origin.z + player.bounds.min.z - 1.0;
            if feet < height {
                lowest = Some(actor);
                height = feet;
            }
        }
    }
    let Some(lowest) = lowest else { return };
    let distance = f64::from(height) - f64::from(top);
    let speed = entity.speed.min(5.0f64.max(if distance < state.water_divisor {
        5.0
    } else {
        distance / state.water_divisor
    }));
    let mut moved = game.body_of(this.clone());
    moved.velocity = vec3(0.0, 0.0, speed as f32);
    game.write_body(this.clone(), &moved, false);
    let kind = game.require_entity(&this).motion;
    game.set_motion_kind(this.clone(), kind);
    if door_state(game, &this).phase != DoorPhase::Up {
        let authored = game.require_entity(&this).authored_target();
        game.use_targets(&authored, Some(&lowest), false);
        door_portals(this.clone(), game, true);
        door_state_mut(game, &this).phase = DoorPhase::Up;
    }
    let frame_seconds = game.host.frame_seconds();
    game.schedule(this, frame_seconds, smart_water);
}

/// Train destination for a corner (`trainDestination`).
fn train_destination(this: ActorId, target: ActorId, game: &mut Q2GameServices) -> Vec3 {
    let origin = game.body_of(target).origin;
    if game.options.edition == Q2Edition::Rerelease && game.require_entity(&this).spawnflags & 32 != 0 {
        return origin;
    }
    let destination = sub3(origin, game.body_of(this.clone()).bounds.min);
    if game.options.edition == Q2Edition::Rerelease && game.require_entity(&this).spawnflags & 16 != 0 {
        sub3(destination, vec3(1.0, 1.0, 1.0))
    } else {
        destination
    }
}

/// Train wait (`trainWait`).
fn train_wait(this: ActorId, game: &mut Q2GameServices) {
    let Some(target) = train_state(game, &this).destination.clone() else {
        return;
    };
    let path_target = game
        .require_entity(&target)
        .spawn
        .values
        .get("pathtarget")
        .cloned()
        .unwrap_or_default();
    if !path_target.is_empty() {
        let previous = game.require_entity(&target).target.clone();
        game.require_entity_mut(&target).target = path_target;
        let authored = game.require_entity(&target).authored_target();
        let activator = game.require_entity(&this).activator.clone();
        game.use_targets(&authored, activator.as_ref(), false);
        game.require_entity_mut(&target).target = previous;
        if !game.host.actors().is_live(&this) {
            return;
        }
    }
    let wait = game.require_entity(&target).wait;
    if wait == 0.0 {
        train_next(this, game);
        return;
    }
    if wait > 0.0 {
        game.schedule(this.clone(), wait, train_next);
    } else if game.require_entity(&this).spawnflags & 2 != 0 {
        if game.options.edition == Q2Edition::Rerelease {
            train_state_mut(game, &this).destination = None;
        } else {
            train_next(this.clone(), game);
        }
        game.require_entity_mut(&this).spawnflags &= !1;
        let mut moved = game.body_of(this.clone());
        moved.velocity = vec3(0.0, 0.0, 0.0);
        game.write_body(this.clone(), &moved, false);
        let kind = game.require_entity(&this).motion;
        game.set_motion_kind(this.clone(), kind);
        game.cancel_actor(this.clone());
    }
    let noise = game
        .require_entity(&this)
        .spawn
        .values
        .get("noise")
        .cloned()
        .unwrap_or_default();
    mover_loop(this, game, &noise, false);
}

/// Train next (`trainNext`).
fn train_next(this: ActorId, game: &mut Q2GameServices) {
    let mut teleported = false;
    loop {
        if game.require_entity(&this).target.is_empty() {
            let noise = game
                .require_entity(&this)
                .spawn
                .values
                .get("noise")
                .cloned()
                .unwrap_or_default();
            mover_loop(this, game, &noise, false);
            return;
        }
        let target_name = game.require_entity(&this).target.clone();
        let Some(target) = game.pick_target(&target_name) else {
            game.host.diagnostic(&format!("Q2 train target missing: {target_name}"));
            return;
        };
        let target_entity = game.require_entity(&target).clone();
        game.require_entity_mut(&this).target = target_entity.target.clone();
        if target_entity.spawnflags & 1 != 0 {
            if teleported {
                game.host.diagnostic("Q2 train has consecutive teleport corners");
                return;
            }
            teleported = true;
            let destination = train_destination(this.clone(), target, game);
            let mut moved = game.body_of(this.clone());
            moved.origin = destination;
            game.write_body(this.clone(), &moved, true);
            let origin = game.body_of(this.clone()).origin;
            game.host_emit(Q2PresentationEvent::Effect(Q2EffectEvent {
                effect: "q2:other-teleport".to_string(),
                origin,
                direction: vec3(0.0, 0.0, 0.0),
                count: 1,
                color: 0,
            }));
            continue;
        }
        train_state_mut(game, &this).destination = Some(target.clone());
        if game.options.edition == Q2Edition::Rerelease && target_entity.speed != 0.0 {
            let entity = game.require_entity_mut(&this);
            entity.speed = target_entity.speed;
            entity.accel = if target_entity.accel == 0.0 {
                target_entity.speed
            } else {
                target_entity.accel
            };
            entity.decel = if target_entity.decel == 0.0 {
                target_entity.speed
            } else {
                target_entity.decel
            };
        }
        let noise = game
            .require_entity(&this)
            .spawn
            .values
            .get("noise")
            .cloned()
            .unwrap_or_default();
        mover_loop(this.clone(), game, &noise, true);
        game.require_entity_mut(&this).spawnflags |= 1;
        let destination = train_destination(this.clone(), target, game);
        let delta = sub3(destination, game.body_of(this.clone()).origin);
        linear_move_to(
            game,
            LinearMotionScope::Foundation,
            this.clone(),
            destination,
            train_wait,
        );
        if game.options.edition == Q2Edition::Rerelease && game.require_entity(&this).spawnflags & 8 != 0 {
            for owned in game.push_team(&this) {
                let member = owned.id().clone();
                if game.entity(&member).is_none() || member == this {
                    continue;
                }
                let entity = game.require_entity(&this).clone();
                let member_entity = game.require_entity_mut(&member);
                member_entity.speed = entity.speed;
                member_entity.accel = entity.accel;
                member_entity.decel = entity.decel;
                game.set_motion_kind(member.clone(), Q2MotionKind::Push);
                let origin = add3(game.body_of(member.clone()).origin, delta);
                linear_move_to(game, LinearMotionScope::Foundation, member, origin, train_piece_wait);
            }
        }
        return;
    }
}

/// Train piece wait (`trainPieceWait`).
fn train_piece_wait(_this: ActorId, _game: &mut Q2GameServices) {}

/// Spawn a train (`spawnTrain`).
fn spawn_train(this: ActorId, game: &mut Q2GameServices) {
    let ship = game.require_entity(&this).classname != "func_train";
    game.movers.trains.insert(
        this.clone(),
        TrainState {
            destination: None,
            debounce: 0.0,
            ship,
        },
    );
    if ship {
        let model = if game.require_entity(&this).classname == "misc_strogg_ship" {
            "models/ships/strogg1/tris.md2"
        } else {
            "models/ships/viper/tris.md2"
        };
        game.require_entity_mut(&this).model = model.to_string();
        game.require_entity_mut(&this).visible = false;
        let mut moved = game.body_of(this.clone());
        moved.bounds.min = vec3(-16.0, -16.0, 0.0);
        moved.bounds.max = vec3(16.0, 16.0, 32.0);
        game.write_body(this.clone(), &moved, false);
    }
    game.set_solid(this.clone(), if ship { Q2Solid::None } else { Q2Solid::Brush });
    game.set_motion_kind(this.clone(), Q2MotionKind::Push);
    let mut moved = game.body_of(this.clone());
    moved.angles = vec3(0.0, 0.0, 0.0);
    game.write_body(this.clone(), &moved, true);
    let entity = game.require_entity_mut(&this);
    if entity.speed == 0.0 {
        entity.speed = if ship { 300.0 } else { 100.0 };
    }
    let speed = game.require_entity(&this).speed;
    let entity = game.require_entity_mut(&this);
    entity.accel = speed;
    entity.decel = speed;
    let entity = game.require_entity_mut(&this);
    entity.damage = if entity.spawnflags & 4 != 0 {
        0.0
    } else if entity.damage == 0.0 {
        100.0
    } else {
        entity.damage
    };
    entity.use_ = Some(train_use);
    entity.blocked = Some(train_blocked);
    game.show(this.clone());
    let frame_seconds = game.host.frame_seconds();
    game.schedule(this, frame_seconds, train_find);
}

/// Spawn a rotating brush (`spawnRotating`).
fn spawn_rotating(this: ActorId, game: &mut Q2GameServices) {
    let spawnflags = game.require_entity(&this).spawnflags;
    let mut direction = if spawnflags & 4 != 0 {
        vec3(0.0, 0.0, 1.0)
    } else if spawnflags & 8 != 0 {
        vec3(1.0, 0.0, 0.0)
    } else {
        vec3(0.0, 1.0, 0.0)
    };
    if spawnflags & 2 != 0 {
        direction = scale3(direction, -1.0);
    }
    game.require_entity_mut(&this).movedir = direction;
    let entity = game.require_entity_mut(&this);
    if entity.speed == 0.0 {
        entity.speed = 100.0;
    }
    if entity.damage == 0.0 {
        entity.damage = 2.0;
    }
    game.set_solid(this.clone(), Q2Solid::Brush);
    let stop = game.require_entity(&this).spawnflags & 32 != 0;
    game.set_motion_kind(this.clone(), if stop { Q2MotionKind::Stop } else { Q2MotionKind::Push });
    let entity = game.require_entity_mut(&this);
    entity.blocked = Some(rotating_damage);
    entity.use_ = Some(rotating_use);
    if game.require_entity(&this).spawnflags & 1 != 0 {
        rotating_use(this.clone(), game, None, None);
    }
    if game.require_entity(&this).spawnflags & 64 != 0 {
        game.require_entity_mut(&this).effects |= 0x1000;
    }
    if game.require_entity(&this).spawnflags & 128 != 0 {
        game.require_entity_mut(&this).effects |= 0x2000;
    }
    game.show(this);
}

/// Spawn a path corner or combat point (`spawnPath`).
fn spawn_path(this: ActorId, game: &mut Q2GameServices) {
    let entity = game.require_entity(&this).clone();
    if entity.targetname.is_empty() && entity.classname == "path_corner" {
        game.remove_actor(this);
        return;
    }
    let mut moved = game.body_of(this.clone());
    moved.bounds.min = vec3(-8.0, -8.0, -8.0);
    moved.bounds.max = vec3(8.0, 8.0, 8.0);
    game.write_body(this.clone(), &moved, false);
    game.require_entity_mut(&this).visible = false;
    game.require_entity_mut(&this).touch = Some(path_touch);
    game.set_solid(this, Q2Solid::Trigger);
}

/// Mover spawn dispatch (`spawn`).
fn spawn_mover(actor: ActorId, game: &mut Q2GameServices) -> bool {
    match game.require_entity(&actor).classname.as_str() {
        "func_door" | "func_door_rotating" | "func_water" | "func_button" => {
            spawn_door(actor, game);
            true
        }
        "func_train" | "misc_strogg_ship" | "misc_viper" => {
            spawn_train(actor, game);
            true
        }
        "func_rotating" => {
            spawn_rotating(actor, game);
            true
        }
        "path_corner" | "point_combat" => {
            spawn_path(actor, game);
            true
        }
        _ => false,
    }
}

/// Door use callback (`doorUse`).
fn door_use(this: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, activator: Option<ActorId>) {
    mover_use(this, game, activator);
}

/// Door killed callback (`doorKilled`).
fn door_killed(this: ActorId, game: &mut Q2GameServices, reaction: DeathReaction) {
    let state = door_state(game, &this);
    for member in state.team.clone() {
        if game.require_entity(&member).max_health <= 0.0 {
            continue;
        }
        let max_health = game.require_entity(&member).max_health;
        let owned = game.owned_of(member);
        game.host.combat().set_health(&owned, max_health);
        game.host.combat().set_traits(
            &owned,
            &CombatTraitChanges {
                can_take_damage: Some(false),
                ..CombatTraitChanges::default()
            },
        );
    }
    mover_use(state.master, game, reaction.pain.attacker);
}

/// Button touch (`buttonTouch`).
fn button_touch(this: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    if game.host.is_player(&contact.other) {
        let health = game
            .host
            .combat()
            .read(&contact.other)
            .map(|state| state.health)
            .unwrap_or(0.0);
        if health > 0.0 {
            mover_use(this, game, Some(contact.other));
        }
    }
}

/// Door touch (`doorTouch`).
fn door_touch(this: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    let state = door_state(game, &this);
    if !game.host.is_player(&contact.other) {
        return;
    }
    let now = game.host.now();
    if now < state.debounce {
        return;
    }
    door_state_mut(game, &this).debounce = now + 5.0;
    let message = game.require_entity(&this).message.clone();
    game.host_emit(Q2PresentationEvent::CenterPrint {
        actor: contact.other,
        text: message,
        instant: false,
        duration_seconds: None,
    });
    game.sound(&this, "misc/talk1.wav", 0, 1.0, 1.0);
}

/// Door blocked (`doorBlocked`).
fn door_blocked(this: ActorId, game: &mut Q2GameServices, other: ActorId) {
    let Some(body) = game.host.bodies().read(&other) else {
        return;
    };
    let zero = vec3(0.0, 0.0, 0.0);
    if !game.host.is_player(&other) && !game.host.is_monster(&other) {
        if game.host.combat().read(&other).is_some() {
            game.damage(
                other.clone(),
                this.clone(),
                Some(this),
                100_000.0,
                1.0,
                zero,
                body.origin,
                zero,
                20,
                0,
                None,
            );
        }
        if game.entity(&other).is_some() {
            game.remove_actor(other);
        }
        return;
    }
    let damage = game.require_entity(&this).damage;
    game.damage(
        other,
        this.clone(),
        Some(this.clone()),
        damage,
        1.0,
        zero,
        body.origin,
        zero,
        20,
        0,
        None,
    );
    let entity = game.require_entity(&this).clone();
    if entity.spawnflags & 4 != 0 || entity.wait < 0.0 {
        return;
    }
    let state = door_state(game, &this);
    let reverse = state.phase == DoorPhase::Down;
    for member in state.team.clone() {
        if reverse {
            let activator = game.require_entity(&member).activator.clone();
            door_up(member, game, activator);
        } else {
            door_go_down(member, game);
        }
    }
}

/// Smart water blocked (`smartWaterBlocked`).
fn smart_water_blocked(this: ActorId, game: &mut Q2GameServices, other: ActorId) {
    let Some(body) = game.host.bodies().read(&other) else {
        return;
    };
    let living = game.host.is_player(&other) || game.host.is_monster(&other);
    if game.host.combat().read(&other).is_some() {
        let zero = vec3(0.0, 0.0, 0.0);
        game.damage(
            other.clone(),
            this.clone(),
            Some(this),
            if living { 100.0 } else { 100_000.0 },
            1.0,
            zero,
            body.origin,
            zero,
            19,
            0,
            None,
        );
    }
    if !living
        && game.entity(&other).is_some()
        && game.host.actors().is_live(&other)
        && game.require_entity(&other).solid != Q2Solid::None
    {
        game.remove_actor(other);
    }
}

/// Door activate (`doorActivate`).
fn door_activate(this: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, _activator: Option<ActorId>) {
    let max_health = game.require_entity(&this).max_health;
    let entity = game.require_entity_mut(&this);
    entity.use_ = None;
    entity.die = if max_health > 0.0 { Some(door_killed) } else { None };
    door_state_mut(game, &this).activated = true;
    if max_health > 0.0 {
        let owned = game.owned_of(this.clone());
        game.host.combat().set_traits(
            &owned,
            &CombatTraitChanges {
                can_take_damage: Some(true),
                ..CombatTraitChanges::default()
            },
        );
    }
    let frame_seconds = game.host.frame_seconds();
    game.schedule(this, frame_seconds, prepare_door);
}

/// Door trigger touch (`Touch_DoorTrigger`).
fn door_trigger_touch(this: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    let Some(owner) = game.require_entity(&this).owner.clone() else {
        return;
    };
    if game.entity(&owner).is_none() {
        return;
    }
    let health = game
        .host
        .combat()
        .read(&contact.other)
        .map(|state| state.health)
        .unwrap_or(0.0);
    if health <= 0.0 {
        return;
    }
    let now = game.host.now();
    if now < game.require_entity(&this).timestamp {
        return;
    }
    let monster = game.host.is_monster(&contact.other);
    let player = game.host.is_player(&contact.other);
    let spawnflags = game.require_entity(&owner).spawnflags;
    if !monster && !player || monster && spawnflags & 8 != 0 {
        return;
    }
    game.require_entity_mut(&this).timestamp = now + 1.0;
    mover_use(owner, game, Some(contact.other));
}

/// Train use (`trainUse`).
fn train_use(this: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, activator: Option<ActorId>) {
    game.require_entity_mut(&this).activator = activator;
    let entity = game.require_entity(&this).clone();
    if train_state(game, &this).ship && !entity.visible {
        game.require_entity_mut(&this).visible = true;
        game.show(this.clone());
    }
    if game.require_entity(&this).spawnflags & 1 != 0 {
        if game.require_entity(&this).spawnflags & 2 == 0 {
            return;
        }
        game.require_entity_mut(&this).spawnflags &= !1;
        let mut moved = game.body_of(this.clone());
        moved.velocity = vec3(0.0, 0.0, 0.0);
        game.write_body(this.clone(), &moved, false);
        let kind = game.require_entity(&this).motion;
        game.set_motion_kind(this.clone(), kind);
        game.cancel_actor(this);
        return;
    }
    let destination = train_state(game, &this).destination.clone();
    match destination {
        None => train_next(this, game),
        Some(destination) => {
            game.require_entity_mut(&this).spawnflags |= 1;
            let origin = train_destination(this.clone(), destination, game);
            linear_move_to(game, LinearMotionScope::Foundation, this, origin, train_wait);
        }
    }
}

/// Train blocked (`trainBlocked`).
fn train_blocked(this: ActorId, game: &mut Q2GameServices, other: ActorId) {
    let state = train_state(game, &this);
    let Some(body) = game.host.bodies().read(&other) else {
        return;
    };
    if game.require_entity(&this).damage == 0.0 {
        return;
    }
    let now = game.host.now();
    if now < state.debounce {
        return;
    }
    train_state_mut(game, &this).debounce = now + 0.5;
    if game.host.combat().read(&other).is_some() {
        let zero = vec3(0.0, 0.0, 0.0);
        let damage = game.require_entity(&this).damage;
        game.damage(
            other,
            this.clone(),
            Some(this),
            damage,
            1.0,
            zero,
            body.origin,
            zero,
            20,
            0,
            None,
        );
    }
}

/// Train find (`trainFind`).
fn train_find(this: ActorId, game: &mut Q2GameServices) {
    let target_name = game.require_entity(&this).target.clone();
    let Some(target) = game.pick_target(&target_name) else {
        game.host
            .diagnostic(&format!("Q2 train first target missing: {target_name}"));
        return;
    };
    let next = game.require_entity(&target).target.clone();
    game.require_entity_mut(&this).target = next;
    let destination = train_destination(this.clone(), target, game);
    let mut moved = game.body_of(this.clone());
    moved.origin = destination;
    game.write_body(this.clone(), &moved, true);
    if game.require_entity(&this).targetname.is_empty() {
        game.require_entity_mut(&this).spawnflags |= 1;
    }
    if game.require_entity(&this).spawnflags & 1 != 0 {
        game.require_entity_mut(&this).activator = Some(this.clone());
        let frame_seconds = game.host.frame_seconds();
        game.schedule(this, frame_seconds, train_next);
    }
}

/// Rotating damage (`rotatingDamage`).
fn rotating_damage(this: ActorId, game: &mut Q2GameServices, other: ActorId) {
    let body = game.host.bodies().read(&other);
    if body.is_some() && game.host.combat().read(&other).is_some() {
        let zero = vec3(0.0, 0.0, 0.0);
        let damage = game.require_entity(&this).damage;
        let origin = body.map(|body| body.origin).unwrap_or(zero);
        game.damage(
            other,
            this.clone(),
            Some(this),
            damage,
            1.0,
            zero,
            origin,
            zero,
            20,
            0,
            None,
        );
    }
}

/// Rotating touch (`rotatingTouch`).
fn rotating_touch(this: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    rotating_damage(this, game, contact.other);
}

/// Rotating use (`rotatingUse`).
fn rotating_use(this: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, _activator: Option<ActorId>) {
    let entity = game.require_entity(&this).clone();
    let stop = length3(entity.angular_velocity) != 0.0;
    let entity = game.require_entity_mut(&this);
    entity.angular_velocity = if stop {
        vec3(0.0, 0.0, 0.0)
    } else {
        scale3(entity.movedir, entity.speed as f32)
    };
    let touch = !stop && game.require_entity(&this).spawnflags & 16 != 0;
    game.require_entity_mut(&this).touch = if touch { Some(rotating_touch) } else { None };
    let kind = game.require_entity(&this).motion;
    game.set_motion_kind(this, kind);
}

/// Path touch (`pathTouch`).
fn path_touch(this: ActorId, game: &mut Q2GameServices, contact: TouchContact) {
    let hooks = game.movers.hooks.expect("Q2 mover hooks are not registered");
    if game.require_entity(&this).classname == "path_corner" {
        (hooks.path_corner)(this, game, contact.other);
    } else {
        (hooks.combat_point)(this, game, contact.other);
    }
}
