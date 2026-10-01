//! Q2 hand grenades (`src/content/q2/equipment/hand-grenades.ts`).

use std::collections::HashMap;

use qa_core::identity::{ActorId, OwnedActor, SavedActorId};
use qa_core::math::Vec3;

use crate::contract::InventoryEntry;
use crate::q2::foundation::checkpoint::{restore_q2_actor, save_q2_actor};
use crate::q2::foundation::host::{Q2Edition, Q2GameServices, Q2PresentationEvent, Q2SoundEvent, Q2SoundLoop};
use crate::q2::foundation::weapons::ballistics::{fire_hand_grenade, Q2HandGrenadeLaunch};
use crate::q2::foundation::weapons::damage::q2_weapon_damage_multiplier;
use crate::q2::foundation::weapons::hand_action::{
    step_hand_action, HandAction, HandActionHost, HandActionInput, HandLifecycle, HandSound,
};
use crate::q2::foundation::weapons::hand_grenade::{HandGrenadeTempo, HandProjectileSpec};
use crate::q2::foundation::weapons::player::{weapon_ammo_changed, weapon_firing_interval};
use crate::q2::foundation::weapons::types::{Q2WeaponInput, WeaponHand};

/// Hand grenade ammo item.
pub const HAND_GRENADE_AMMO: &str = "q2:ammo_grenades";

/// Hand grenade checkpoint version.
const HAND_GRENADE_CHECKPOINT_VERSION: i32 = 1;

/// Hand grenade loadout (`HandGrenadeLoadout`).
#[derive(Debug, Clone, PartialEq)]
pub struct HandGrenadeLoadout {
    /// Whether enabled.
    pub enabled: bool,
    /// Initial ammo.
    pub initial_ammo: f64,
    /// Capacity.
    pub capacity: f64,
    /// Whether ammo is infinite.
    pub infinite_ammo: bool,
}

/// Default hand grenade loadout.
pub fn default_hand_grenade_loadout() -> HandGrenadeLoadout {
    HandGrenadeLoadout {
        enabled: true,
        initial_ammo: 0.0,
        capacity: 50.0,
        infinite_ammo: false,
    }
}

/// Hand grenade equipment input (`HandGrenadeEquipmentInput`).
///
/// The throw projector is passed separately to `step` because it
/// borrows the session mutably.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HandGrenadeEquipmentInput {
    /// Attack pressed.
    pub pressed: bool,
    /// Attack held.
    pub held: bool,
    /// Attack released.
    pub released: bool,
    /// Lifecycle.
    pub lifecycle: HandLifecycle,
    /// Aim angles.
    pub angles: Vec3,
    /// Gravity.
    pub gravity: f64,
    /// Quad until.
    pub quad_until: f64,
    /// Double until.
    pub double_until: f64,
    /// Quad-fire until.
    pub quad_fire_until: f64,
    /// Haste.
    pub haste: bool,
    /// No stacked double.
    pub no_stack_double: bool,
    /// Whether players collide.
    pub players_collide: bool,
}

/// Hand grenade equipment state (`HandGrenadeEquipmentState`).
#[derive(Debug, Clone, PartialEq)]
pub struct HandGrenadeEquipmentState {
    /// Loadout.
    pub config: HandGrenadeLoadout,
    /// Action.
    pub action: HandAction,
}

/// Hand grenade checkpoint (`HandGrenadeCheckpoint`).
#[derive(Debug, Clone, PartialEq)]
pub struct HandGrenadeCheckpoint {
    /// Version.
    pub version: i32,
    /// Edition.
    pub edition: Q2Edition,
    /// Actors.
    pub actors: HashMap<String, HandGrenadeActorCheckpoint>,
}

/// Hand grenade actor checkpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct HandGrenadeActorCheckpoint {
    /// Actor.
    pub actor: SavedActorId,
    /// Loadout.
    pub config: HandGrenadeLoadout,
    /// Action.
    pub action: HandAction,
}

/// Hand grenade equipment (`Q2HandGrenadeEquipment`).
///
/// Stateless handle; live state lives in the arena runtime and the
/// session passes the game on every call.
#[derive(Debug, Clone, Copy, Default)]
pub struct Q2HandGrenadeEquipment;

impl Q2HandGrenadeEquipment {
    /// Register ballistics callbacks (`constructor`).
    pub fn new(game: &mut Q2GameServices) -> Self {
        crate::q2::foundation::weapons::ballistics::register_ballistics_callbacks(game);
        Q2HandGrenadeEquipment
    }

    /// Read owner state (`state`).
    pub fn state_snapshot(&self, game: &Q2GameServices, actor: ActorId) -> Option<HandGrenadeEquipmentState> {
        game.equipment.grenades.get(&actor).cloned()
    }

    /// Configure an owner (`configure`).
    pub fn configure(&self, owner: &OwnedActor, game: &mut Q2GameServices, loadout: HandGrenadeLoadout) {
        check_loadout(&loadout);
        let actor = owner.id().clone();
        game.host.inventory().configure(
            owner,
            &InventoryEntry {
                item: HAND_GRENADE_AMMO.to_string(),
                count: loadout.initial_ammo,
                capacity: loadout.capacity,
                count_policy: None,
            },
        );
        game.equipment.grenades.insert(
            actor,
            HandGrenadeEquipmentState {
                config: loadout,
                action: HandAction::Idle,
            },
        );
    }

    /// Remove an owner (`remove`).
    pub fn remove(&self, actor: ActorId, game: &mut Q2GameServices) {
        game.equipment.grenades.remove(&actor);
    }

    /// Step an owner (`step`).
    pub fn step(
        &self,
        actor: ActorId,
        game: &mut Q2GameServices,
        input: &HandGrenadeEquipmentInput,
        project: &mut dyn FnMut(Vec3, Vec3) -> (Vec3, Vec3),
    ) {
        let Some(current) = game.equipment.grenades.get(&actor).cloned() else {
            return;
        };
        let owned = game.host.actors().resolve_owned(&actor);
        if owned.is_none() || input.lifecycle == HandLifecycle::Removed {
            game.equipment.grenades.remove(&actor);
            return;
        }
        let owned = owned.expect("Q2 hand grenade owner is missing");
        game.equipment.grenade_steps += 1;
        self.step_inner(actor, game, input, project, owned, &current);
        game.equipment.grenade_steps -= 1;
    }

    /// Inner step with the active-step counter held.
    fn step_inner(
        &self,
        actor: ActorId,
        game: &mut Q2GameServices,
        input: &HandGrenadeEquipmentInput,
        project: &mut dyn FnMut(Vec3, Vec3) -> (Vec3, Vec3),
        owned: OwnedActor,
        current: &HandGrenadeEquipmentState,
    ) {
        let edition = game.options.edition;
        let now = game.host.now();
        let mut host = GrenadeStepHost {
            game,
            actor: actor.clone(),
            owned,
            config: current.config.clone(),
            effects: Vec::new(),
        };
        let mut lifecycle = input.lifecycle;
        if lifecycle == HandLifecycle::Alive
            && host
                .game
                .host
                .combat()
                .read(&actor)
                .is_some_and(|combat| combat.health <= 0.0)
        {
            lifecycle = HandLifecycle::Dead;
        }
        let mut action_input = HandActionInput {
            tempo: HandGrenadeTempo {
                edition,
                haste: input.haste,
                quad_fire: input.quad_fire_until > now,
            },
            now,
            pressed: input.pressed,
            held: input.held,
            released: input.released,
            lifecycle,
            enabled: current.config.enabled,
            angles: input.angles,
            damage_multiplier: 1.0,
            gravity: input.gravity,
            project,
        };
        let action = step_hand_action(&current.action, &mut action_input, &mut host);
        let game = host.game;
        if game.equipment.grenades.get(&actor) != Some(current) || !game.host.actors().is_live(&actor) {
            return;
        }
        let next = HandGrenadeEquipmentState {
            config: current.config.clone(),
            action,
        };
        game.equipment.grenades.insert(actor.clone(), next.clone());
        let effects = host.effects;
        for effect in effects {
            if game.equipment.grenades.get(&actor) != Some(&next) || !game.host.actors().is_live(&actor) {
                break;
            }
            match effect {
                GrenadeSideEffect::Launch { spec } => {
                    let weapon_input = Q2WeaponInput {
                        attack: false,
                        latched_attack: false,
                        holster: false,
                        angles: input.angles,
                        ducked: false,
                        spectator: false,
                        notarget: false,
                        hand: WeaponHand::Right,
                        animate_player: false,
                        quad_until: input.quad_until,
                        double_until: input.double_until,
                        quad_fire_until: input.quad_fire_until,
                        haste: input.haste,
                        no_stack_double: input.no_stack_double,
                        instant_switch: false,
                        quick_switch: false,
                        infinite_ammo: false,
                        players_collide: input.players_collide,
                        gravity: input.gravity,
                        weapon_thunk: false,
                        view_height: 0.0,
                    };
                    let damage = spec.damage * q2_weapon_damage_multiplier(&actor, &weapon_input, now, game);
                    fire_hand_grenade(
                        actor.clone(),
                        game,
                        &Q2HandGrenadeLaunch {
                            start: spec.start,
                            direction: spec.direction,
                            damage,
                            speed: spec.speed,
                            timer: spec.fuse,
                            radius: spec.radius,
                            held: spec.held,
                            gravity: spec.gravity,
                            players_collide: input.players_collide,
                        },
                    );
                }
                GrenadeSideEffect::Sound { event } => {
                    let body = game
                        .host
                        .bodies()
                        .read(&actor)
                        .unwrap_or_else(|| panic!("Hand grenade owner lost its shared body before removal"));
                    game.host.emit(Q2PresentationEvent::Sound(Q2SoundEvent {
                        actor: Some(actor.clone()),
                        origin: body.origin,
                        path: if event == HandSound::Cock {
                            "weapons/hgrena1b.wav".to_string()
                        } else {
                            "weapons/hgrenc1b.wav".to_string()
                        },
                        channel: 1,
                        volume: 1.0,
                        attenuation: 1.0,
                        reliable: false,
                        loop_: match event {
                            HandSound::Cock => Q2SoundLoop::Once,
                            HandSound::CookStart => Q2SoundLoop::Start,
                            HandSound::CookStop => Q2SoundLoop::Stop,
                        },
                        loop_owner: None,
                    }));
                }
            }
        }
    }

    /// Capture equipment (`capture`).
    pub fn capture(&self, game: &Q2GameServices) -> HandGrenadeCheckpoint {
        if game.equipment.grenade_steps != 0 {
            panic!("Hand grenade checkpoint requires no active steps");
        }
        let mut actors = HashMap::new();
        for (actor, state) in game.equipment.grenades.iter() {
            let saved = save_q2_actor(Some(actor)).expect("Q2 hand grenade actor is missing");
            actors.insert(
                checkpoint_key(actor),
                HandGrenadeActorCheckpoint {
                    actor: saved,
                    config: state.config.clone(),
                    action: state.action.clone(),
                },
            );
        }
        HandGrenadeCheckpoint {
            version: HAND_GRENADE_CHECKPOINT_VERSION,
            edition: game.options.edition,
            actors,
        }
    }

    /// Restore equipment (`restore`).
    pub fn restore(&self, game: &mut Q2GameServices, checkpoint: HandGrenadeCheckpoint) {
        if game.equipment.grenade_steps != 0 {
            panic!("Hand grenade checkpoint requires no active steps");
        }
        if checkpoint.edition != game.options.edition {
            panic!("Hand grenade checkpoint edition mismatch");
        }
        let mut next: HashMap<ActorId, HandGrenadeEquipmentState> = HashMap::new();
        for saved in checkpoint.actors.values() {
            check_loadout(&saved.config);
            let actor = restore_q2_actor(game, saved.actor.clone()).id().clone();
            if game.host.bodies().read(&actor).is_none() || game.host.inventory().entries(&actor).is_empty() {
                panic!("Hand grenade checkpoint owner is missing shared state");
            }
            if next.contains_key(&actor) {
                panic!("Hand grenade checkpoint owner is duplicated");
            }
            if !game
                .host
                .inventory()
                .entries(&actor)
                .iter()
                .any(|entry| entry.item == HAND_GRENADE_AMMO)
            {
                panic!("Hand grenade checkpoint owner lost its ammo entry");
            }
            next.insert(
                actor,
                HandGrenadeEquipmentState {
                    config: saved.config.clone(),
                    action: saved.action.clone(),
                },
            );
        }
        game.equipment.grenades = next;
    }
}

/// Step host over the throwing owner.
struct GrenadeStepHost<'a> {
    /// Game services.
    game: &'a mut Q2GameServices,
    /// Owning actor.
    actor: ActorId,
    /// Owned actor.
    owned: OwnedActor,
    /// Loadout.
    config: HandGrenadeLoadout,
    /// Queued side effects.
    effects: Vec<GrenadeSideEffect>,
}

/// Queued grenade side effect.
#[derive(Debug, Clone, Copy, PartialEq)]
enum GrenadeSideEffect {
    /// Launch a projectile.
    Launch {
        /// Projectile spec.
        spec: HandProjectileSpec,
    },
    /// Play a sound.
    Sound {
        /// Sound event.
        event: HandSound,
    },
}

impl HandActionHost for GrenadeStepHost<'_> {
    fn firing_interval(&mut self, seconds: f64) -> f64 {
        weapon_firing_interval(self.game, &self.actor, seconds)
    }

    fn reserve(&mut self) -> bool {
        if self.config.infinite_ammo {
            return true;
        }
        if self
            .game
            .host
            .inventory()
            .consume(&self.owned, &HAND_GRENADE_AMMO.to_string(), 1.0)
        {
            weapon_ammo_changed(self.game, &self.actor, &HAND_GRENADE_AMMO.to_string());
            return true;
        }
        self.game
            .host
            .inventory()
            .count(&self.actor, &HAND_GRENADE_AMMO.to_string())
            != 0.0
    }

    fn consume(&mut self) {}

    fn refund(&mut self) {
        let Some(entry) = self
            .game
            .host
            .inventory()
            .entries(&self.actor)
            .into_iter()
            .find(|entry| entry.item == HAND_GRENADE_AMMO)
        else {
            panic!("Reserved hand grenade lost its canonical inventory entry");
        };
        let mut refunded = entry.clone();
        refunded.count += 1.0;
        self.game.host.inventory().configure(&self.owned, &refunded);
        weapon_ammo_changed(self.game, &self.actor, &HAND_GRENADE_AMMO.to_string());
    }

    fn emit(&mut self, spec: &HandProjectileSpec) {
        self.effects.push(GrenadeSideEffect::Launch { spec: *spec });
    }

    fn sound(&mut self, event: HandSound) {
        self.effects.push(GrenadeSideEffect::Sound { event });
    }
}

/// Validate a loadout (`checkLoadout`).
fn check_loadout(loadout: &HandGrenadeLoadout) {
    for value in [loadout.capacity, loadout.initial_ammo] {
        if !value.is_finite() || value.fract() != 0.0 || !(0.0..=9_007_199_254_740_992.0).contains(&value) {
            panic!("Hand grenade loadout violates the engine ammo contract");
        }
    }
    if loadout.initial_ammo > loadout.capacity {
        panic!("Hand grenade loadout violates the engine ammo contract");
    }
}

/// Checkpoint map key for an actor.
fn checkpoint_key(actor: &ActorId) -> String {
    let saved = save_q2_actor(Some(actor)).expect("Q2 hand grenade actor is missing");
    format!("{}:{}", saved.slot, saved.generation)
}
