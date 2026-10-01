//! Q2 players (`src/content/q2/base/player/index.ts`).

use qa_core::identity::{ActorId, OwnedActor, SavedActorId};
use qa_core::math::{add3, dot3, scale3, vec3, Vec3};

use crate::contract::{ArmorState, InventoryEntry, ItemId, PoweredProtectionState};
use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::checkpoint::{restore_q2_actor, save_q2_actor};
use crate::q2::foundation::entity_services::js_round;
use crate::q2::foundation::host::{
    Q2Die, Q2Edition, Q2EffectEvent, Q2Entity, Q2GameServices, Q2ItemNameFn, Q2LandmarkCarry, Q2Mode, Q2MotionKind,
    Q2Pain, Q2PresentationEvent, Q2Solid, Q2SpawnFn, Q2TraceRequest, SpawnModule,
};
use crate::q2::foundation::items::{Q2DropOptions, Q2ItemModule, Q2PlayerPowerups};
use crate::q2::foundation::monsters::gibs::{throw_gib, Q2GibOptions};
use crate::q2::foundation::weapons::player::{bind_player_weapon, tick_player_weapon, weapon_definition};
use crate::q2::foundation::weapons::presentation::q2_weapon_recoil;
use crate::q2::foundation::weapons::turn::{
    begin_q2_weapon_turn, early_q2_weapon_turn, latch_q2_weapon_buttons, Q2WeaponTurnState,
};
use crate::q2::foundation::weapons::types::{PlayerAnimationPriority, Q2WeaponEvent, Q2WeaponInput, Q2WeaponState};
use crate::q2::foundation::weapons::vectors::angle_vectors;
use crate::q2::support::contracts::{
    AttackCause, BodyState, CombatState, CombatTraitChanges, DamageDecision, DamageDelivery, DamageFeedback,
    DamageReactionKind, DamageRequest, DeathReaction, TransitionIntent,
};

use super::checkpoint::{
    Q2LandmarkCarryCheckpoint, Q2PlayerCheckpointEntry, Q2PlayerIntermissionCheckpoint, Q2PlayerStateCheckpoint,
    Q2PlayersCheckpoint,
};
use super::commands::{parse_command_int, q2_chat_allowed, run_q2_client_command, score_rows, userinfo_value};
use super::environment::{q2_falling_damage, q2_world_effects};
use super::landmarks::place_q2_landmark;
use super::obituary::{q2_obituary, Q2ObituaryRecipient};
use super::spawns::{
    q2_entities_named, q2_kill_box, q2_spawn_origin, select_q2_spawn, spawn_callbacks, spawn_player_spawn,
};
use super::types::{
    Q2BodyChanges, Q2CharacterContext, Q2CharacterWeapon, Q2PlayerCarry, Q2PlayerContext, Q2PlayerEvent,
    Q2PlayerGender, Q2PlayerHand, Q2PlayerHooks, Q2PlayerMovement, Q2PlayerMovementChange, Q2PlayerRules,
    Q2PlayerSpawnChange, Q2PlayerState, Q2PlayerView, Q2PrintLevel,
};
use super::view::{
    q2_build_view, q2_client_animation, q2_client_effects, q2_damage_feedback, q2_death_animation_frames,
};

/// Player intermission (`Q2Players[intermission]`).
#[derive(Debug, Clone, PartialEq)]
pub enum Q2Intermission {
    /// Playing.
    Playing,
    /// Intermission.
    Intermission {
        /// Map.
        map: String,
        /// Started time.
        started: f64,
        /// Whether exiting.
        exit: bool,
        /// Landmark.
        landmark: Option<Q2LandmarkCarry>,
    },
}

impl Default for Q2Intermission {
    fn default() -> Self {
        Q2Intermission::Playing
    }
}

/// Spawn placement solution (`spawnPlacement` result).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2SpawnSolution {
    /// Origin.
    pub origin: Vec3,
    /// Angles.
    pub angles: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Whether placed from a landmark.
    pub from_landmark: bool,
}

/// Player admission (`Q2PlayerAdmission`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2PlayerAdmission {
    /// Slot.
    pub slot: i32,
    /// Userinfo.
    pub userinfo: String,
    /// Whether to initialize inventory.
    pub initialize_inventory: bool,
    /// Whether Q2 weapons drive this player.
    pub use_q2_weapons: Option<bool>,
    /// Whether Q2 inventory drives this player.
    pub use_q2_inventory: Option<bool>,
    /// Carry.
    pub carry: Option<Q2PlayerCarry>,
}

/// Connection result (`Q2ConnectionResult`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2ConnectionResult {
    /// Whether allowed.
    pub allowed: bool,
    /// Userinfo.
    pub userinfo: String,
    /// Denial reason.
    pub reason: Option<String>,
}

/// Read registered player hooks.
pub fn player_hooks(game: &Q2GameServices) -> Q2PlayerHooks {
    game.players.hooks.expect("Q2 player hooks are not registered")
}

/// Read the registered item module.
pub fn player_items(game: &Q2GameServices) -> Q2ItemModule {
    game.players.items.expect("Q2 player items are not registered")
}

/// Read an admitted player state.
fn player_state(game: &Q2GameServices, actor: &ActorId) -> Q2PlayerState {
    game.players
        .states
        .get(actor)
        .cloned()
        .expect("Q2 player has not been admitted")
}

impl Q2CharacterContext for Q2PlayerContext<'_> {
    fn actor_id(&self) -> ActorId {
        self.actor.clone()
    }

    fn owned_actor(&self) -> OwnedActor {
        self.game.owned_of(self.actor.clone())
    }

    fn now(&mut self) -> f64 {
        self.game.host.now()
    }

    fn random(&mut self) -> f64 {
        self.game.host.random()
    }

    fn movement(&mut self) -> Q2PlayerMovement {
        (player_hooks(self.game).movement)(self.actor.clone())
    }

    fn rules(&self) -> Q2PlayerRules {
        self.game.players.rules.clone()
    }

    fn powerups(&mut self) -> Q2PlayerPowerups {
        player_items(self.game).player_powerups(self.game, &self.actor)
    }

    fn weapon_state(&mut self) -> Option<Q2CharacterWeapon> {
        weapon_state_for(self.actor.clone(), self.game)
    }

    fn environment_damage(&mut self, amount: f64, means: i32, flags: i32) {
        environment_damage(self.actor.clone(), self.game, amount, means, flags);
    }

    fn noise(&mut self, origin: Vec3) {
        (player_hooks(self.game).noise)(self.actor.clone(), origin);
    }

    fn body(&mut self) -> BodyState {
        self.game.body_of(self.actor.clone())
    }

    fn move_body(&mut self, changes: Q2BodyChanges, link: bool) {
        let mut body = self.game.body_of(self.actor.clone());
        if let Some(origin) = changes.origin {
            body.origin = origin;
        }
        if let Some(angles) = changes.angles {
            body.angles = angles;
        }
        if let Some(velocity) = changes.velocity {
            body.velocity = velocity;
        }
        if let Some(bounds) = changes.bounds {
            body.bounds = bounds;
        }
        if let Some(ground) = changes.ground {
            body.ground = ground;
        }
        self.game.write_body(self.actor.clone(), &body, link);
    }

    fn sound(&mut self, path: &str, channel: i32, volume: f64, attenuation: f64) {
        let actor = self.actor.clone();
        self.game.sound(&actor, path, channel, volume, attenuation);
    }

    fn emit(&mut self, event: Q2PresentationEvent) {
        self.game.host.emit(event);
    }

    fn point_contents(&mut self, point: Vec3) -> i32 {
        self.game.host.point_contents(point)
    }

    fn combat(&mut self) -> Option<CombatState> {
        self.game.host.combat().read(&self.actor)
    }

    fn inventory_count(&mut self, item: &ItemId) -> f64 {
        self.game.host.inventory().count(&self.actor, item)
    }

    fn mode(&self) -> Q2Mode {
        self.game.options.mode
    }

    fn deathmatch_flags(&self) -> i32 {
        self.game.options.deathmatch_flags
    }

    fn edition(&self) -> Q2Edition {
        self.game.options.edition
    }

    fn state_snapshot(&mut self) -> Q2PlayerState {
        player_state(self.game, &self.actor)
    }

    fn with_state<R>(&mut self, f: impl FnOnce(&mut Q2PlayerState) -> R) -> R {
        let state = self
            .game
            .players
            .states
            .get_mut(&self.actor)
            .expect("Q2 player has not been admitted");
        f(state)
    }

    fn entity_snapshot(&mut self) -> Q2Entity {
        self.game.require_entity(&self.actor).clone()
    }

    fn with_entity<R>(&mut self, f: impl FnOnce(&mut Q2Entity) -> R) -> R {
        let entity = self.game.require_entity_mut(&self.actor);
        f(entity)
    }
}

/// Read powerups for a player (`powerups`).
pub fn powerups_for(actor: ActorId, game: &Q2GameServices) -> Q2PlayerPowerups {
    player_items(game).player_powerups(game, &actor)
}

/// Read weapon state for a player (`weaponState`).
pub fn weapon_state_for(actor: ActorId, game: &mut Q2GameServices) -> Option<Q2CharacterWeapon> {
    if let Some(observe) = player_hooks(game).weapon_state {
        return observe(actor);
    }
    let weapon = game.weapons.states.get(&actor)?.clone();
    let ammo = match weapon.weapon.clone() {
        None => None,
        Some(name) => weapon_definition(game, &name).ammo.clone(),
    };
    let now = game.host.now();
    let edition = game.options.edition;
    let (kick_origin, kick_angles) = q2_weapon_recoil(&weapon, edition, now);
    Some(Q2CharacterWeapon {
        q2_name: weapon.weapon.clone(),
        ammo,
        kick_angles,
        kick_origin,
        loop_sound: weapon.loop_sound.clone(),
    })
}

/// Apply environment damage (`environmentDamage`).
pub fn environment_damage(actor: ActorId, game: &mut Q2GameServices, amount: f64, means: i32, flags: i32) {
    let world = game.host.world_actor();
    let mut attack = game.attack(actor.clone(), Some(world.clone()), means, flags, None);
    attack.inflictor = Some(world);
    let body = game.body_of(actor.clone());
    let zero = vec3(0.0, 0.0, 0.0);
    game.host.combat().apply(&DamageRequest {
        attack,
        target: actor,
        amount,
        knockback: 0.0,
        direction: if means == 22 { vec3(0.0, 0.0, 1.0) } else { zero },
        point: body.origin,
        normal: zero,
        delivery: DamageDelivery::Direct,
    });
}

/// Player pain (no-op).
fn player_pain(_actor: ActorId, _game: &mut Q2GameServices, _reaction: crate::q2::support::contracts::PainReaction) {}

/// Player die.
fn player_die(actor: ActorId, game: &mut Q2GameServices, reaction: DeathReaction) {
    death(actor, game, reaction, true);
}

/// Body die.
fn body_die(actor: ActorId, game: &mut Q2GameServices, reaction: DeathReaction) {
    if game.host.combat().read(&actor).map_or(0.0, |combat| combat.health) < -40.0 {
        game.sound(&actor, "misc/udeath.wav", 4, 1.0, 1.0);
        for _ in 0..4 {
            throw_gib(
                actor.clone(),
                game,
                "models/objects/gibs/sm_meat/tris.md2",
                reaction.pain.damage,
                Q2GibOptions::default(),
            );
        }
        let mut moved = game.body_of(actor.clone());
        moved.origin = vec3(moved.origin.x, moved.origin.y, moved.origin.z - 48.0);
        game.write_body(actor.clone(), &moved, true);
        bound_module(game).throw_client_head(actor.clone(), game, reaction.pain.damage);
        let owned = game.owned_of(actor);
        game.host.combat().set_traits(
            &owned,
            &CombatTraitChanges {
                can_take_damage: Some(false),
                mass: None,
                invulnerable: None,
                team: None,
                no_knockback: None,
            },
        );
    }
}

/// Player callbacks (`Q2Players[callbacks]`).
pub fn player_callbacks() -> Q2CallbackDefinitions {
    let mut callbacks = spawn_callbacks();
    let pain: Q2Pain = player_pain;
    let die: Q2Die = player_die;
    callbacks.pain.insert("player_pain", pain);
    callbacks.die.insert("player_die", die);
    callbacks.die.insert("body_die", body_die as _);
    callbacks
}

/// Spawn a player entity (`Q2Players[spawn]`).
pub fn spawn_player(actor: ActorId, game: &mut Q2GameServices) -> bool {
    spawn_player_spawn(actor, game)
}

/// Player item-name handler (unused).
fn player_item_name(_classname: &str) -> Option<String> {
    None
}

/// Q2 players module (`Q2Players`).
#[derive(Debug, Clone, Copy)]
pub struct Q2Players {
    /// Item module.
    items: Q2ItemModule,
    /// Provider hooks.
    hooks: Q2PlayerHooks,
}

/// Player behavior overrides (`Q2Players` subclass hooks).
///
/// The donor `Q2Players` dispatches these methods virtually, so edition
/// subclasses observe every internal call. Base methods keep direct
/// behavior; every internal call site routes through the matching
/// `dispatched_*` twin, which consults this table first. Override
/// functions call the direct base method for `super` behavior, so the
/// dispatch never recurses.
#[derive(Debug, Clone, Copy, Default)]
pub struct Q2PlayerOverrides {
    /// Connect override.
    pub connect: Option<fn(&mut Q2GameServices, String) -> Q2ConnectionResult>,
    /// Userinfo override.
    pub userinfo_changed: Option<fn(ActorId, &mut Q2GameServices, String)>,
    /// Restore-carry override.
    pub restore_carry: Option<fn(ActorId, &mut Q2GameServices, Q2PlayerCarry)>,
    /// Obituary override; `None` skips the base plain-text print.
    pub obituary: Option<fn(ActorId, &mut Q2GameServices, Option<ActorId>) -> Option<String>>,
    /// Clear-death-inventory override.
    pub clear_death_inventory: Option<fn(ActorId, &mut Q2GameServices)>,
    /// Record-death override.
    pub record_death: Option<fn(ActorId, &mut Q2GameServices, DeathReaction) -> bool>,
    /// Dead-frame override.
    pub dead_frame: Option<fn(ActorId, &mut Q2GameServices)>,
    /// World-effects override.
    pub world_effects: Option<fn(ActorId, &mut Q2GameServices)>,
    /// Falling-damage override.
    pub falling_damage: Option<fn(ActorId, &mut Q2GameServices)>,
    /// Build-view override.
    pub build_view: Option<fn(ActorId, &mut Q2GameServices, i32, bool) -> Q2PlayerView>,
    /// Damage-feedback override.
    pub damage_feedback: Option<fn(ActorId, &mut Q2GameServices, i32) -> (i32, i32)>,
    /// Client-animation override.
    pub client_animation: Option<fn(ActorId, &mut Q2GameServices)>,
    /// Update-bob override.
    pub update_bob: Option<fn(ActorId, &mut Q2GameServices)>,
    /// Spawn-placement override.
    pub spawn_placement: Option<fn(ActorId, &mut Q2GameServices, Option<Q2LandmarkCarry>) -> Q2SpawnSolution>,
    /// Kill-box override.
    pub kill_box: Option<fn(ActorId, &mut Q2GameServices) -> bool>,
    /// Put-in-server override.
    pub put_in_server: Option<fn(ActorId, &mut Q2GameServices, bool, Option<Q2LandmarkCarry>)>,
    /// Begin-intermission override.
    pub begin_intermission: Option<fn(&mut Q2GameServices, String, Option<Q2LandmarkCarry>)>,
    /// Before-exit-level override.
    pub before_exit_level: Option<fn(&mut Q2GameServices, String)>,
    /// End-frame override.
    pub end_frame: Option<fn(ActorId, &mut Q2GameServices)>,
    /// Coop-stay drop override.
    pub can_drop_coop_stay_items: Option<fn(&Q2GameServices) -> bool>,
    /// Death override.
    pub death: Option<fn(ActorId, &mut Q2GameServices, DeathReaction, bool)>,
    /// Save-carry override.
    pub save_carry: Option<fn(ActorId, &mut Q2GameServices) -> Q2PlayerCarry>,
    /// Respawn override.
    pub respawn: Option<fn(ActorId, &mut Q2GameServices)>,
}

/// Create the Q2 players module (`createQ2Players`).
pub fn create_q2_players(items: Q2ItemModule, hooks: Q2PlayerHooks) -> Q2Players {
    Q2Players { items, hooks }
}

impl Q2Players {
    /// Dispatched connect for internal call sites.
    pub fn dispatched_connect(&self, game: &mut Q2GameServices, userinfo: &str) -> Q2ConnectionResult {
        if let Some(connect) = game.players.overrides.connect {
            return connect(game, userinfo.to_string());
        }
        self.connect(game, userinfo)
    }

    /// Dispatched userinfo for internal call sites.
    pub fn dispatched_userinfo_changed(&self, actor: ActorId, game: &mut Q2GameServices, userinfo: &str) {
        if let Some(userinfo_changed) = game.players.overrides.userinfo_changed {
            return userinfo_changed(actor, game, userinfo.to_string());
        }
        self.userinfo_changed(actor, game, userinfo)
    }

    /// Dispatched restore-carry for internal call sites.
    pub fn dispatched_restore_carry(&self, actor: ActorId, game: &mut Q2GameServices, carry: Q2PlayerCarry) {
        if let Some(restore_carry) = game.players.overrides.restore_carry {
            return restore_carry(actor, game, carry);
        }
        self.restore_carry(actor, game, carry)
    }

    /// Dispatched obituary for internal call sites.
    pub fn dispatched_obituary(
        &self,
        actor: ActorId,
        game: &mut Q2GameServices,
        attacker: Option<ActorId>,
    ) -> Option<String> {
        if let Some(obituary) = game.players.overrides.obituary {
            return obituary(actor, game, attacker);
        }
        Some(self.obituary(actor, game, attacker))
    }

    /// Dispatched clear-death-inventory for internal call sites.
    pub fn dispatched_clear_death_inventory(&self, actor: ActorId, game: &mut Q2GameServices) {
        if let Some(clear) = game.players.overrides.clear_death_inventory {
            return clear(actor, game);
        }
        self.clear_death_inventory(actor, game)
    }

    /// Dispatched record-death for internal call sites.
    pub fn dispatched_record_death(&self, actor: ActorId, game: &mut Q2GameServices, reaction: DeathReaction) -> bool {
        if let Some(record_death) = game.players.overrides.record_death {
            return record_death(actor, game, reaction);
        }
        self.record_death(actor, game, reaction)
    }

    /// Dispatched dead-frame for internal call sites.
    pub fn dispatched_dead_frame(&self, actor: ActorId, game: &mut Q2GameServices) {
        if let Some(dead_frame) = game.players.overrides.dead_frame {
            return dead_frame(actor, game);
        }
        self.dead_frame(actor, game)
    }

    /// Dispatched world-effects for internal call sites.
    pub fn dispatched_world_effects(&self, actor: ActorId, game: &mut Q2GameServices) {
        if let Some(world_effects) = game.players.overrides.world_effects {
            return world_effects(actor, game);
        }
        self.world_effects(actor, game)
    }

    /// Dispatched falling-damage for internal call sites.
    pub fn dispatched_falling_damage(&self, actor: ActorId, game: &mut Q2GameServices) {
        if let Some(falling_damage) = game.players.overrides.falling_damage {
            return falling_damage(actor, game);
        }
        self.falling_damage(actor, game)
    }

    /// Dispatched build-view for internal call sites.
    pub fn dispatched_build_view(
        &self,
        actor: ActorId,
        game: &mut Q2GameServices,
        flashes: i32,
        intermission: bool,
    ) -> Q2PlayerView {
        if let Some(build_view) = game.players.overrides.build_view {
            return build_view(actor, game, flashes, intermission);
        }
        self.build_view(actor, game, flashes, intermission)
    }

    /// Dispatched damage-feedback for internal call sites.
    pub fn dispatched_damage_feedback(&self, actor: ActorId, game: &mut Q2GameServices, pain_index: i32) -> (i32, i32) {
        if let Some(damage_feedback) = game.players.overrides.damage_feedback {
            return damage_feedback(actor, game, pain_index);
        }
        self.damage_feedback(actor, game, pain_index)
    }

    /// Dispatched client-animation for internal call sites.
    pub fn dispatched_client_animation(&self, actor: ActorId, game: &mut Q2GameServices) {
        if let Some(client_animation) = game.players.overrides.client_animation {
            return client_animation(actor, game);
        }
        self.client_animation(actor, game)
    }

    /// Dispatched update-bob for internal call sites.
    pub fn dispatched_update_bob(&self, actor: ActorId, game: &mut Q2GameServices) {
        if let Some(update_bob) = game.players.overrides.update_bob {
            return update_bob(actor, game);
        }
        self.update_bob(actor, game)
    }

    /// Dispatched spawn-placement for internal call sites.
    pub fn dispatched_spawn_placement(
        &self,
        actor: ActorId,
        game: &mut Q2GameServices,
        landmark: Option<&Q2LandmarkCarry>,
    ) -> Q2SpawnSolution {
        if let Some(spawn_placement) = game.players.overrides.spawn_placement {
            return spawn_placement(actor, game, landmark.cloned());
        }
        self.spawn_placement(actor, game, landmark)
    }

    /// Dispatched kill-box for internal call sites.
    pub fn dispatched_kill_box(&self, actor: ActorId, game: &mut Q2GameServices) -> bool {
        if let Some(kill_box) = game.players.overrides.kill_box {
            return kill_box(actor, game);
        }
        self.kill_box(actor, game)
    }

    /// Dispatched put-in-server for internal call sites.
    pub fn dispatched_put_in_server(
        &self,
        actor: ActorId,
        game: &mut Q2GameServices,
        restore_loadout: bool,
        landmark: Option<&Q2LandmarkCarry>,
    ) {
        if let Some(put_in_server) = game.players.overrides.put_in_server {
            return put_in_server(actor, game, restore_loadout, landmark.cloned());
        }
        self.put_in_server(actor, game, restore_loadout, landmark)
    }

    /// Dispatched begin-intermission for internal call sites.
    pub fn dispatched_begin_intermission(
        &self,
        game: &mut Q2GameServices,
        map: String,
        landmark: Option<Q2LandmarkCarry>,
    ) {
        if let Some(begin_intermission) = game.players.overrides.begin_intermission {
            return begin_intermission(game, map, landmark);
        }
        self.begin_intermission(game, map, landmark)
    }

    /// Dispatched before-exit-level for internal call sites.
    pub fn dispatched_before_exit_level(&self, game: &mut Q2GameServices, map: &str) {
        if let Some(before_exit_level) = game.players.overrides.before_exit_level {
            return before_exit_level(game, map.to_string());
        }
        self.before_exit_level(game, map)
    }

    /// Dispatched end-frame for internal call sites.
    pub fn dispatched_end_frame(&self, actor: ActorId, game: &mut Q2GameServices) {
        if let Some(end_frame) = game.players.overrides.end_frame {
            return end_frame(actor, game);
        }
        self.end_frame(actor, game)
    }

    /// Dispatched save-carry for internal call sites.
    pub fn dispatched_save_carry(&self, actor: ActorId, game: &mut Q2GameServices) -> Q2PlayerCarry {
        if let Some(save_carry) = game.players.overrides.save_carry {
            return save_carry(actor, game);
        }
        self.save_carry(actor, game)
    }

    /// Dispatched respawn for internal call sites.
    pub fn dispatched_respawn(&self, actor: ActorId, game: &mut Q2GameServices) {
        if let Some(respawn) = game.players.overrides.respawn {
            return respawn(actor, game);
        }
        self.respawn(actor, game)
    }
}

impl Q2Players {
    /// Register hooks and build the spawn module.
    pub fn register(&self, game: &mut Q2GameServices, rules: Q2PlayerRules) -> SpawnModule {
        game.players.hooks = Some(self.hooks);
        game.players.items = Some(self.items);
        game.players.rules = rules;
        let spawn: Q2SpawnFn = spawn_player;
        let item_name: Q2ItemNameFn = player_item_name;
        SpawnModule {
            spawn,
            item_name,
            callbacks: player_callbacks(),
        }
    }

    /// Build a context for an admitted player.
    pub fn context<'game>(&self, actor: ActorId, game: &'game mut Q2GameServices) -> Q2PlayerContext<'game> {
        if !game.players.states.contains_key(&actor) {
            panic!("Q2 player has not been admitted");
        }
        Q2PlayerContext { actor, game }
    }

    /// Capture players.
    pub fn capture(&self, game: &mut Q2GameServices) -> Q2PlayersCheckpoint {
        let mut players = Vec::new();
        let actors: Vec<ActorId> = game.players.states.keys().cloned().collect();
        for actor in actors {
            let mut state = player_state(game, &actor);
            let chase = state.chase_target.clone();
            state.chase_target = None;
            players.push(Q2PlayerCheckpointEntry {
                actor: SavedActorId::from(&actor),
                state: Q2PlayerStateCheckpoint {
                    state,
                    chase_target: save_q2_actor(chase.as_ref()),
                },
            });
        }
        let intermission = match game.players.intermission.clone() {
            Q2Intermission::Playing => Q2PlayerIntermissionCheckpoint::Playing,
            Q2Intermission::Intermission {
                map,
                started,
                exit,
                landmark,
            } => Q2PlayerIntermissionCheckpoint::Intermission {
                map,
                started,
                exit,
                landmark: landmark.as_ref().map(|landmark| Q2LandmarkCarryCheckpoint {
                    name: landmark.name.clone(),
                    relative_origin: landmark.relative_origin,
                    relative_velocity: landmark.relative_velocity,
                    relative_view_angles: landmark.relative_view_angles,
                    player: SavedActorId::from(&landmark.player),
                }),
            },
        };
        Q2PlayersCheckpoint {
            version: 1,
            corpse_index: game.players.corpse_index,
            death_animation: game.players.death_animation,
            pain_animation: game.players.pain_animation,
            rules: game.players.rules.clone(),
            intermission,
            players,
        }
    }

    /// Restore players.
    pub fn restore(&self, game: &mut Q2GameServices, checkpoint: &Q2PlayersCheckpoint) {
        game.players.corpse_index = checkpoint.corpse_index;
        game.players.death_animation = checkpoint.death_animation;
        game.players.pain_animation = checkpoint.pain_animation;
        game.players.rules = checkpoint.rules.clone();
        game.players.intermission = match checkpoint.intermission.clone() {
            Q2PlayerIntermissionCheckpoint::Playing => Q2Intermission::Playing,
            Q2PlayerIntermissionCheckpoint::Intermission {
                map,
                started,
                exit,
                landmark,
            } => Q2Intermission::Intermission {
                map,
                started,
                exit,
                landmark: landmark.map(|landmark| Q2LandmarkCarry {
                    player: restore_q2_actor(game, landmark.player).id().clone(),
                    name: landmark.name,
                    relative_origin: landmark.relative_origin,
                    relative_velocity: landmark.relative_velocity,
                    relative_view_angles: landmark.relative_view_angles,
                }),
            },
        };
        game.players.states = std::collections::HashMap::new();
        for entry in &checkpoint.players {
            let actor = restore_q2_actor(game, entry.actor.clone()).id().clone();
            if game.entity(&actor).is_none() {
                panic!("Q2 player checkpoint references missing source wrapper");
            }
            let mut state = entry.state.state.clone();
            state.chase_target = entry
                .state
                .chase_target
                .clone()
                .map(|saved| game.host.actors().reference_saved(saved));
            game.players.states.insert(actor, state);
        }
    }

    /// Connect a player.
    pub fn connect(&self, game: &mut Q2GameServices, userinfo: &str) -> Q2ConnectionResult {
        let spectator = game.options.mode == Q2Mode::Deathmatch
            && userinfo_value(userinfo, "spectator").is_some_and(|value| !value.is_empty() && value != "0");
        let pass = if spectator {
            game.players.rules.spectator_password.clone()
        } else {
            game.players.rules.password.clone()
        };
        let mut reason = String::new();
        if (player_hooks(game).banned)(&userinfo_value(userinfo, "ip").unwrap_or_default()) {
            reason = "Banned.".to_string();
        } else if !pass.is_empty()
            && pass != "none"
            && pass != userinfo_value(userinfo, if spectator { "spectator" } else { "password" }).unwrap_or_default()
        {
            reason = if spectator {
                "Spectator password required or incorrect.".to_string()
            } else {
                "Password required or incorrect.".to_string()
            };
        } else if spectator
            && game
                .players
                .states
                .values()
                .filter(|state| state.connected && state.requested_spectator)
                .count() as i32
                >= game.players.rules.max_spectators
        {
            reason = "Server spectator limit is full.".to_string();
        }
        if reason.is_empty() {
            return Q2ConnectionResult {
                allowed: true,
                userinfo: userinfo.to_string(),
                reason: None,
            };
        }
        Q2ConnectionResult {
            allowed: false,
            userinfo: format_userinfo_with_rejmsg(userinfo, &reason),
            reason: Some(reason),
        }
    }
    /// Attach a player.
    pub fn attach(&self, actor: ActorId, game: &mut Q2GameServices, admission: Q2PlayerAdmission) -> Q2PlayerState {
        let owned = game.owned_of(actor.clone());
        game.host.actors().assert_owned(&owned);
        if game.players.states.contains_key(&actor) {
            panic!("Q2 player lifecycle already bound");
        }
        if admission.slot < 0 || admission.slot >= game.options.max_clients as i32 {
            panic!("Q2 player slot is outside maxclients");
        }
        let now = game.host.now();
        let mut state = Q2PlayerState::new(admission.slot, now);
        state.use_q2_weapons = admission.use_q2_weapons.unwrap_or(true);
        state.use_q2_inventory = admission
            .use_q2_inventory
            .unwrap_or(game.options.inventory_provider.namespace == "q2");
        state.air_finished = now + 12.0;
        game.players.states.insert(actor.clone(), state);
        // Release cleanup runs through `on_actor_released`, which drops the
        // player state alongside the arena record.
        self.dispatched_userinfo_changed(actor.clone(), game, &admission.userinfo.clone());
        let owned = game.owned_of(actor.clone());
        if !game.host.inventory().has(&actor) {
            game.host.inventory().create(&owned, &[]);
        }
        if game.host.combat().read(&actor).is_none() {
            game.create_combat(&owned, 100.0, 200.0, true);
        }
        if admission.initialize_inventory {
            player_items(game).configure_player(&owned, game, true);
        }
        if player_state(game, &actor).use_q2_weapons && !game.weapons.states.contains_key(&actor) {
            bind_player_weapon(game, &owned, Q2WeaponState::new(Some("blaster".to_string())));
        }
        {
            let entity = game.require_entity_mut(&actor);
            entity.pain = Some(player_pain as _);
            entity.die = Some(player_die as _);
            if entity.max_health == 0.0 || entity.max_health.is_nan() {
                entity.max_health = 100.0;
            }
            if entity.view_height == 0 {
                entity.view_height = 22;
            }
        }
        if let Some(carry) = admission.carry {
            self.dispatched_restore_carry(actor.clone(), game, carry);
        }
        let spawn_inventory = game.host.inventory().entries(&actor);
        let coop_respawn = self.dispatched_save_carry(actor.clone(), game);
        let entry = game
            .players
            .states
            .get_mut(&actor)
            .expect("Q2 player has not been admitted");
        entry.spawn_inventory = spawn_inventory;
        entry.coop_respawn = Some(coop_respawn);
        let requested = player_state(game, &actor).requested_spectator;
        game.players
            .states
            .get_mut(&actor)
            .expect("Q2 player has not been admitted")
            .spectator = requested;
        player_state(game, &actor)
    }

    /// Apply userinfo.
    pub fn userinfo_changed(&self, actor: ActorId, game: &mut Q2GameServices, userinfo: &str) {
        if !game.players.states.contains_key(&actor) {
            panic!("Q2 userinfo requires an admitted player");
        }
        let valid = !userinfo.contains('"') && !userinfo.contains(';');
        let source = if valid {
            userinfo
        } else {
            "\\name\\badinfo\\skin\\male/grunt"
        };
        let clipped: String = source.chars().take(511).collect();
        game.players
            .states
            .get_mut(&actor)
            .expect("Q2 userinfo requires an admitted player")
            .userinfo = clipped.clone();
        let name: String = userinfo_value(&clipped, "name")
            .unwrap_or_default()
            .chars()
            .take(15)
            .collect();
        let skin = userinfo_value(&clipped, "skin").unwrap_or_default();
        let gender = match userinfo_value(&clipped, "gender")
            .unwrap_or_default()
            .chars()
            .next()
            .map(|first| first.to_lowercase().next())
        {
            Some(Some('f')) => Q2PlayerGender::Female,
            Some(Some('m')) => Q2PlayerGender::Male,
            _ => Q2PlayerGender::Neutral,
        };
        let spectator = game.options.mode == Q2Mode::Deathmatch
            && userinfo_value(&clipped, "spectator").is_some_and(|value| !value.is_empty() && value != "0");
        let fov_value = parse_command_int(userinfo_value(&clipped, "fov").as_deref());
        let fov = if game.options.mode == Q2Mode::Deathmatch && game.options.deathmatch_flags & 32768 != 0 {
            90
        } else if fov_value < 1 {
            90
        } else {
            fov_value.min(160)
        };
        let hand_value = parse_command_int(userinfo_value(&clipped, "hand").as_deref());
        let hand = if hand_value == 1 {
            Q2PlayerHand::Left
        } else if hand_value == 2 {
            Q2PlayerHand::Center
        } else {
            Q2PlayerHand::Right
        };
        let entry = game
            .players
            .states
            .get_mut(&actor)
            .expect("Q2 userinfo requires an admitted player");
        entry.name = name;
        entry.skin = skin;
        entry.gender = gender;
        entry.requested_spectator = spectator;
        entry.fov = fov;
        entry.hand = hand;
        let (slot, name, skin) = {
            let state = player_state(game, &actor);
            (state.slot, state.name.clone(), state.skin.clone())
        };
        (player_hooks(game).emit)(Q2PlayerEvent::Userinfo {
            actor,
            slot,
            name,
            skin,
        });
    }

    /// Snapshot the carry for an actor.
    fn save_carry_snapshot(&self, actor: ActorId, game: &mut Q2GameServices) -> Q2PlayerCarry {
        let entity = game.require_entity(&actor).clone();
        self.carry_of(entity, game)
    }

    /// Build the carry for an entity.
    fn carry_of(&self, entity: Q2Entity, game: &mut Q2GameServices) -> Q2PlayerCarry {
        let actor = entity.actor.id().clone();
        let combat = game
            .host
            .combat()
            .read(&actor)
            .expect("Q2 carry requires shared combat state");
        Q2PlayerCarry {
            health: combat.health,
            maximum_health: entity.max_health,
            armor: combat.armor.clone(),
            inventory: game.host.inventory().entries(&actor),
            weapon: game.weapons.states.get(&actor).and_then(|state| state.weapon.clone()),
            selected_item: player_state(game, &actor).selected_item.clone(),
            score: player_state(game, &actor).score,
            flags: entity.flags & (16 | 32 | 4096),
            power_cubes: entity.power_cubes,
        }
    }

    /// Save the carry (`saveCarry`).
    pub fn save_carry(&self, actor: ActorId, game: &mut Q2GameServices) -> Q2PlayerCarry {
        self.save_carry_snapshot(actor, game)
    }

    /// Restore a carry.
    pub fn restore_carry(&self, actor: ActorId, game: &mut Q2GameServices, carry: Q2PlayerCarry) {
        let owned = game.owned_of(actor.clone());
        game.host.combat().set_health(&owned, carry.health);
        game.host.combat().set_armor(&owned, &carry.armor);
        self.set_inventory(actor.clone(), game, carry.inventory.clone());
        {
            let entity = game.require_entity_mut(&actor);
            entity.max_health = carry.maximum_health;
            entity.flags |= carry.flags;
            entity.power_cubes = carry.power_cubes;
        }
        let god = carry.flags & 16 != 0;
        let notarget = carry.flags & 32 != 0;
        {
            let entry = game
                .players
                .states
                .get_mut(&actor)
                .expect("Q2 player has not been admitted");
            entry.god = god;
            entry.notarget = notarget;
            entry.score = carry.score;
            entry.selected_item = carry.selected_item.clone();
        }
        game.host.combat().set_traits(
            &owned,
            &CombatTraitChanges {
                can_take_damage: None,
                mass: None,
                invulnerable: Some(god),
                team: None,
                no_knockback: None,
            },
        );
        if let Some(weapon) = game.weapons.states.get_mut(&actor) {
            weapon.weapon = carry.weapon;
            weapon.pending = None;
        }
    }

    /// Replace the inventory.
    pub fn set_inventory(&self, actor: ActorId, game: &mut Q2GameServices, entries: Vec<InventoryEntry>) {
        let owned = game.owned_of(actor.clone());
        for mut entry in game.host.inventory().entries(&actor) {
            entry.count = 0.0;
            game.host.inventory().configure(&owned, &entry);
        }
        for entry in &entries {
            game.host.inventory().configure(&owned, entry);
        }
    }

    /// Consume a key.
    pub fn consumed_key(&self, actor: ActorId, game: &mut Q2GameServices) {
        if player_state(game, &actor).coop_respawn.is_none() {
            return;
        }
        let current = game.host.inventory().entries(&actor);
        let power_cubes = game.require_entity(&actor).power_cubes;
        let entry = game
            .players
            .states
            .get_mut(&actor)
            .expect("Q2 player has not been admitted");
        if let Some(respawn) = entry.coop_respawn.as_mut() {
            respawn.power_cubes = power_cubes;
            for slot in respawn.inventory.iter_mut() {
                if slot.item.starts_with("q2:key_") {
                    if let Some(found) = current.iter().find(|item| item.item == slot.item) {
                        *slot = found.clone();
                    } else {
                        slot.count = 0.0;
                    }
                }
            }
        }
    }
}

/// Rebuild a module handle from registered runtime state.
fn bound_module(game: &Q2GameServices) -> Q2Players {
    Q2Players {
        items: player_items(game),
        hooks: player_hooks(game),
    }
}

/// Run death (shared by the die callback and direct kills).
fn death(actor: ActorId, game: &mut Q2GameServices, reaction: DeathReaction, callback: bool) {
    if let Some(death) = game.players.overrides.death {
        return death(actor, game, reaction, callback);
    }
    bound_module(game).death(actor, game, reaction, callback);
}

impl Q2Players {
    /// Place a spawn (`spawnPlacement`).
    pub fn spawn_placement(
        &self,
        actor: ActorId,
        game: &mut Q2GameServices,
        landmark: Option<&Q2LandmarkCarry>,
    ) -> Q2SpawnSolution {
        if let Some(select) = player_hooks(game).select_spawn {
            if let Some(selected) = select(actor.clone(), game) {
                return Q2SpawnSolution {
                    origin: selected.origin,
                    angles: selected.angles,
                    velocity: Vec3::default(),
                    from_landmark: false,
                };
            }
        }
        let state = player_state(game, &actor);
        let movement = (player_hooks(game).movement)(actor.clone());
        // Source selects spawn while the prior life still contributes its old health/distance.
        let spawn_point = game.players.rules.spawn_point.clone();
        let spot = select_q2_spawn(game, &state, &spawn_point);
        let placement = match landmark {
            None => None,
            Some(carry) => place_q2_landmark(actor.clone(), game, carry, spot.clone(), movement.standing_bounds),
        };
        match placement {
            None => {
                let body = game.body_of(spot.clone());
                Q2SpawnSolution {
                    origin: q2_spawn_origin(game, spot),
                    angles: vec3(0.0, body.angles.y, 0.0),
                    velocity: Vec3::default(),
                    from_landmark: false,
                }
            }
            Some(found) => Q2SpawnSolution {
                origin: vec3(found.origin.x, found.origin.y, found.origin.z + 1.0),
                angles: vec3(found.angles.x / 3.0, found.angles.y, found.angles.z),
                velocity: found.velocity,
                from_landmark: true,
            },
        }
    }

    /// Run the spawn kill box (`killBox`).
    pub fn kill_box(&self, actor: ActorId, game: &mut Q2GameServices) -> bool {
        q2_kill_box(actor, game)
    }

    /// Select a teleport spawn (`selectTeleportSpawn`).
    pub fn select_teleport_spawn(&self, actor: ActorId, game: &mut Q2GameServices) -> Q2SpawnSolution {
        self.dispatched_spawn_placement(actor, game, None)
    }

    /// Put a player in the server (`putInServer`).
    pub fn put_in_server(
        &self,
        actor: ActorId,
        game: &mut Q2GameServices,
        restore_loadout: bool,
        landmark: Option<&Q2LandmarkCarry>,
    ) {
        let old_movement = (player_hooks(game).movement)(actor.clone());
        let placement = self.dispatched_spawn_placement(actor.clone(), game, landmark);
        if restore_loadout {
            let state = player_state(game, &actor);
            let mut coop_restored = false;
            if game.options.mode == Q2Mode::Coop {
                if let Some(respawn) = state.coop_respawn.clone() {
                    let score = state.score.max(respawn.score);
                    self.dispatched_restore_carry(actor.clone(), game, Q2PlayerCarry { score, ..respawn });
                    coop_restored = true;
                }
            }
            if !coop_restored
                && (game.options.mode == Q2Mode::Deathmatch
                    || game.host.combat().read(&actor).map_or(0.0, |combat| combat.health) <= 0.0)
            {
                if state.use_q2_inventory {
                    self.set_inventory(actor.clone(), game, Vec::new());
                    let owned = game.owned_of(actor.clone());
                    player_items(game).configure_player(&owned, game, true);
                } else {
                    self.set_inventory(actor.clone(), game, state.spawn_inventory.clone());
                }
                let owned = game.owned_of(actor.clone());
                game.host.combat().set_health(&owned, 100.0);
                game.host.combat().set_armor(
                    &owned,
                    &ArmorState {
                        regular: crate::contract::RegularArmorState::None,
                        powered: PoweredProtectionState::None,
                    },
                );
                game.require_entity_mut(&actor).max_health = 100.0;
                game.players
                    .states
                    .get_mut(&actor)
                    .expect("Q2 player has not been admitted")
                    .selected_item = Some("q2:weapon_blaster".to_string());
                if player_state(game, &actor).use_q2_inventory {
                    if let Some(initialized) = player_hooks(game).persistent_inventory_initialized {
                        initialized(actor.clone(), game);
                    }
                }
            }
        }
        let now = game.host.now();
        {
            let entry = game
                .players
                .states
                .get_mut(&actor)
                .expect("Q2 player has not been admitted");
            entry.dead = false;
            entry.gibbed = false;
            entry.old_water_level = 0;
            entry.air_finished = now + 12.0;
            entry.drown_damage = 2.0;
            entry.damage_alpha = 0.0;
            entry.bonus_alpha = 0.0;
            entry.damage_blood = 0.0;
            entry.damage_armor = 0.0;
            entry.damage_power_armor = 0.0;
            entry.damage_knockback = 0.0;
            entry.fall_time = 0.0;
            entry.bob_time = 0.0;
            entry.bob_move = 0.0;
            entry.animation_priority = 0;
            entry.animation_end = 39;
            entry.spectator = entry.requested_spectator;
            entry.noclip = entry.spectator;
            entry.chase_target = None;
            entry.old_velocity = Vec3::default();
            entry.landmark_free_fall = placement.from_landmark;
        }
        self.clear_powerups(actor.clone(), game);
        let spectator = player_state(game, &actor).spectator;
        let animate = old_movement.animate_q2;
        let skin = player_state(game, &actor).skin.clone();
        let god = player_state(game, &actor).god;
        let edition = game.options.edition;
        {
            let entity = game.require_entity_mut(&actor);
            entity.view_height = 22;
            entity.server_flags &= !(1 | 2);
            entity.flags &= !(1024 | 0x20000);
            entity.angular_velocity = Vec3::default();
            entity.frame = 0;
            entity.old_frame = -1;
            entity.effects = 0;
            entity.render_flags = if edition == Q2Edition::Rerelease { 32768 } else { 0 };
            entity.visible = !spectator;
            if animate {
                let model = skin.split('/').next().filter(|part| !part.is_empty()).unwrap_or("male");
                entity.model = format!("players/{model}/tris.md2");
            }
        }
        let owned = game.owned_of(actor.clone());
        game.host.combat().set_traits(
            &owned,
            &CombatTraitChanges {
                can_take_damage: Some(!spectator),
                mass: Some(200.0),
                invulnerable: Some(god),
                team: None,
                no_knockback: None,
            },
        );
        let mut moved = game.body_of(actor.clone());
        moved.origin = placement.origin;
        moved.velocity = placement.velocity;
        moved.angles = placement.angles;
        moved.bounds = old_movement.standing_bounds;
        moved.ground = None;
        game.write_body(actor.clone(), &moved, false);
        game.set_solid(actor.clone(), if spectator { Q2Solid::None } else { Q2Solid::Box });
        game.set_motion_kind(actor.clone(), Q2MotionKind::Stationary);
        (player_hooks(game).set_movement)(
            actor.clone(),
            Q2PlayerMovementChange::Spawn(Q2PlayerSpawnChange {
                origin: placement.origin,
                velocity: placement.velocity,
                angles: placement.angles,
                command_angles: old_movement.command_angles,
                hold_milliseconds: 0,
                spectator,
            }),
        );
        if !spectator {
            self.dispatched_kill_box(actor.clone(), game);
        }
        if player_state(game, &actor).use_q2_weapons && game.weapons.states.contains_key(&actor) {
            let state = player_state(game, &actor);
            let selected = if game.options.mode == Q2Mode::Deathmatch {
                "blaster".to_string()
            } else {
                state
                    .coop_respawn
                    .as_ref()
                    .and_then(|respawn| respawn.weapon.clone())
                    .or_else(|| game.weapons.states.get(&actor).and_then(|weapon| weapon.weapon.clone()))
                    .unwrap_or_else(|| "blaster".to_string())
            };
            crate::q2::foundation::weapons::ballistics::reset_silencer(game, &actor);
            game.weapons
                .states
                .insert(actor.clone(), Q2WeaponState::new(Some(selected)));
        }
        if animate {
            game.show(actor.clone());
        }
        game.link_actor(actor.clone());
        if let Some(spawned) = player_hooks(game).player_spawned {
            spawned(actor, game);
        }
    }

    /// Respawn a player (`respawn`).
    pub fn respawn(&self, actor: ActorId, game: &mut Q2GameServices) {
        if game.options.mode == Q2Mode::Singleplayer {
            (player_hooks(game).emit)(Q2PlayerEvent::LoadMenu { actor: actor.clone() });
            return;
        }
        if !player_state(game, &actor).noclip {
            self.copy_to_body_queue(actor.clone(), game);
        }
        self.dispatched_put_in_server(actor.clone(), game, true, None);
        let body = game.body_of(actor.clone());
        let movement = (player_hooks(game).movement)(actor.clone());
        let spectator = player_state(game, &actor).spectator;
        (player_hooks(game).set_movement)(
            actor.clone(),
            Q2PlayerMovementChange::Spawn(Q2PlayerSpawnChange {
                origin: body.origin,
                velocity: body.velocity,
                angles: movement.view_angles,
                command_angles: movement.command_angles,
                hold_milliseconds: 112,
                spectator,
            }),
        );
        let now = game.host.now();
        let entry = game
            .players
            .states
            .get_mut(&actor)
            .expect("Q2 player has not been admitted");
        entry.respawn_time = now;
        entry.event = "q2:player-teleport".to_string();
    }

    /// Copy to the body queue (`copyToBodyQueue`).
    pub fn copy_to_body_queue(&self, actor: ActorId, game: &mut Q2GameServices) {
        let corpses = q2_entities_named(game, "bodyque");
        let slot = game.players.corpse_index;
        let corpse = corpses
            .get(slot as usize)
            .cloned()
            .expect("Q2 corpse reuse requires worldspawn's eight reserved body slots");
        game.players.corpse_index = (slot + 1) % 8;
        let owned = game.owned_of(actor.clone());
        game.host.bodies().unlink(&owned);
        let corpse_owned = game.owned_of(corpse.clone());
        game.host.bodies().unlink(&corpse_owned);
        let entity = game.require_entity(&actor).clone();
        {
            let target = game.require_entity_mut(&corpse);
            target.model = entity.model.clone();
            target.frame = entity.frame;
            target.skin = entity.skin;
            target.effects = entity.effects;
            target.render_flags = entity.render_flags;
            target.server_flags = entity.server_flags;
            target.clip_mask = entity.clip_mask;
            target.owner = entity.owner.clone();
            target.visible = entity.visible;
        }
        let body = game.body_of(actor.clone());
        game.write_body(corpse.clone(), &body, false);
        // Original CopyToBodyQue leaves the reserved edict's health value in place.
        if game.host.combat().read(&corpse).is_none() {
            let owned = game.owned_of(corpse.clone());
            game.create_combat(&owned, 0.0, 0.0, true);
        } else {
            let owned = game.owned_of(corpse.clone());
            game.set_combat_traits(
                &owned,
                &CombatTraitChanges {
                    can_take_damage: Some(true),
                    mass: None,
                    invulnerable: Some(false),
                    team: None,
                    no_knockback: None,
                },
            );
        }
        game.require_entity_mut(&corpse).die = Some(body_die as Q2Die);
        game.set_solid(corpse.clone(), entity.solid);
        game.set_motion_kind(corpse.clone(), entity.motion);
        game.show(corpse);
    }

    /// Throw a client head (`throwClientHead`).
    pub fn throw_client_head(&self, actor: ActorId, game: &mut Q2GameServices, damage: f64) {
        let head = game.host.random() < 0.5;
        {
            let entity = game.require_entity_mut(&actor);
            entity.model = if head {
                "models/objects/gibs/head2/tris.md2"
            } else {
                "models/objects/gibs/skull/tris.md2"
            }
            .to_string();
            entity.skin = if head { 1 } else { 0 };
            entity.frame = 0;
            entity.effects = 2;
            entity.angular_velocity = Vec3::default();
        }
        let magnitude = if damage < 50.0 { 0.7 } else { 1.2 };
        let push_x = (game.host.random() * 2.0 - 1.0) * 100.0 * magnitude;
        let push_y = (game.host.random() * 2.0 - 1.0) * 100.0 * magnitude;
        let push_z = (200.0 + game.host.random() * 100.0) * magnitude;
        let body = game.body_of(actor.clone());
        let mut moved = body.clone();
        moved.origin = vec3(body.origin.x, body.origin.y, body.origin.z + 32.0);
        moved.velocity = vec3(
            body.velocity.x + push_x as f32,
            body.velocity.y + push_y as f32,
            body.velocity.z + push_z as f32,
        );
        moved.bounds.min = vec3(-16.0, -16.0, 0.0);
        moved.bounds.max = vec3(16.0, 16.0, 16.0);
        game.write_body(actor.clone(), &moved, true);
        game.set_solid(actor.clone(), Q2Solid::None);
        game.set_motion_kind(actor.clone(), Q2MotionKind::Bounce);
        game.show(actor);
    }

    /// Run death (`death`).
    pub fn death(&self, actor: ActorId, game: &mut Q2GameServices, reaction: DeathReaction, callback: bool) {
        let _ = callback;
        let movement = (player_hooks(game).movement)(actor.clone());
        if !movement.animate_q2 {
            self.dispatched_record_death(actor, game, reaction);
            return;
        }
        {
            let entity = game.require_entity_mut(&actor);
            entity.angular_velocity = Vec3::default();
            entity.server_flags |= 2;
        }
        let body = game.body_of(actor.clone());
        let mut moved = body.clone();
        moved.angles = vec3(0.0, body.angles.y, 0.0);
        moved.bounds.max = vec3(body.bounds.max.x, body.bounds.max.y, -8.0);
        game.write_body(actor.clone(), &moved, false);
        game.set_motion_kind(actor.clone(), Q2MotionKind::Toss);
        let owned = game.owned_of(actor.clone());
        game.host.combat().set_traits(
            &owned,
            &CombatTraitChanges {
                can_take_damage: Some(true),
                mass: None,
                invulnerable: None,
                team: None,
                no_knockback: None,
            },
        );
        let first = self.dispatched_record_death(actor.clone(), game, reaction.clone());
        let health = game.host.combat().read(&actor).map_or(0.0, |combat| combat.health);
        if health < -40.0 && !player_state(game, &actor).gibbed {
            if game.require_entity(&actor).flags & 0x10000 == 0 {
                game.sound(&actor, "misc/udeath.wav", 4, 1.0, 1.0);
                for _ in 0..4 {
                    throw_gib(
                        actor.clone(),
                        game,
                        "models/objects/gibs/sm_meat/tris.md2",
                        reaction.pain.damage,
                        Q2GibOptions::default(),
                    );
                }
            }
            game.require_entity_mut(&actor).flags &= !0x10000;
            self.throw_client_head(actor.clone(), game, reaction.pain.damage);
            game.players
                .states
                .get_mut(&actor)
                .expect("Q2 player has not been admitted")
                .gibbed = true;
            let owned = game.owned_of(actor.clone());
            game.host.combat().set_traits(
                &owned,
                &CombatTraitChanges {
                    can_take_damage: Some(false),
                    mass: None,
                    invulnerable: None,
                    team: None,
                    no_knockback: None,
                },
            );
        } else if first {
            game.players.death_animation = (game.players.death_animation + 1) % 3;
            let death_animation = game.players.death_animation;
            game.players
                .states
                .get_mut(&actor)
                .expect("Q2 player has not been admitted")
                .animation_priority = 5;
            let frames = q2_death_animation_frames(movement.ducked, death_animation);
            game.require_entity_mut(&actor).frame = frames.0;
            game.players
                .states
                .get_mut(&actor)
                .expect("Q2 player has not been admitted")
                .animation_end = frames.1;
            let variant = (game.host.random() * 4.0).floor() as i32 + 1;
            game.sound(&actor, &format!("*death{variant}.wav"), 2, 1.0, 1.0);
        }
        game.link_actor(actor.clone());
        game.show(actor);
    }

    /// Format an obituary (`obituary`).
    pub fn obituary(&self, actor: ActorId, game: &mut Q2GameServices, attacker: Option<ActorId>) -> String {
        let victim = player_state(game, &actor);
        let attacker_state = attacker
            .as_ref()
            .and_then(|attacker| game.players.states.get(attacker).cloned());
        let means = game
            .require_entity(&actor)
            .last_attack
            .as_ref()
            .map(|attack| match &attack.cause {
                AttackCause::Q2 { means_of_death, .. } => *means_of_death,
                _ => 0,
            })
            .unwrap_or(0);
        let deathmatch = game.options.mode == Q2Mode::Deathmatch;
        let coop = game.options.mode == Q2Mode::Coop;
        let suicide = attacker.as_ref() == Some(&actor);
        let victim_actor = actor;
        let attacker_actor = attacker;
        let mut apply = |recipient: Q2ObituaryRecipient, change: i32| {
            let recipient = match recipient {
                Q2ObituaryRecipient::Victim => victim_actor.clone(),
                Q2ObituaryRecipient::Attacker => attacker_actor.clone().unwrap_or(victim_actor.clone()),
            };
            self.apply_score(
                victim_actor.clone(),
                attacker_actor.clone(),
                game,
                change,
                means,
                recipient,
            );
        };
        q2_obituary(
            &victim,
            attacker_state.as_ref(),
            suicide,
            means,
            deathmatch,
            coop,
            &mut apply,
        )
    }

    /// Apply a score change (`applyScore`).
    pub fn apply_score(
        &self,
        victim: ActorId,
        attacker: Option<ActorId>,
        game: &mut Q2GameServices,
        change: i32,
        means: i32,
        recipient: ActorId,
    ) {
        let Some(score) = player_hooks(game).score else {
            if let Some(entry) = game.players.states.get_mut(&recipient) {
                entry.score += change;
            }
            return;
        };
        let scorer = if recipient == victim {
            victim.clone()
        } else {
            match attacker.clone() {
                Some(scorer) if game.entity(&scorer).is_some() => scorer,
                _ => panic!("Q2 obituary score recipient has no live source actor"),
            }
        };
        score(victim, attacker, game, change, means, scorer);
    }

    /// Clear the death inventory (`clearDeathInventory`).
    pub fn clear_death_inventory(&self, actor: ActorId, game: &mut Q2GameServices) {
        self.set_inventory(actor, game, Vec::new());
    }

    /// Record a death (`recordDeath`).
    pub fn record_death(&self, actor: ActorId, game: &mut Q2GameServices, reaction: DeathReaction) -> bool {
        let first = !player_state(game, &actor).dead;
        if first {
            let now = game.host.now();
            game.players
                .states
                .get_mut(&actor)
                .expect("Q2 player has not been admitted")
                .respawn_time = now + 1.0;
            let attacker = reaction.pain.attacker.clone();
            let killer = if attacker.as_ref().is_some_and(|attacker| *attacker != actor) {
                attacker.as_ref().and_then(|attacker| game.host.bodies().read(attacker))
            } else {
                match reaction.inflictor.clone() {
                    Some(inflictor) if inflictor != actor => game.host.bodies().read(&inflictor),
                    _ => None,
                }
            };
            let origin = game.body_of(actor.clone()).origin;
            let yaw = match killer {
                Some(killer) => {
                    (f64::from(killer.origin.y - origin.y).atan2(f64::from(killer.origin.x - origin.x)) * 180.0
                        / std::f64::consts::PI
                        + 360.0)
                        % 360.0
                }
                None => f64::from(game.body_of(actor.clone()).angles.y),
            };
            game.players
                .states
                .get_mut(&actor)
                .expect("Q2 player has not been admitted")
                .killer_yaw = yaw;
            if let Some(message) = self.dispatched_obituary(actor.clone(), game, attacker) {
                (player_hooks(game).emit)(Q2PlayerEvent::Print {
                    target: None,
                    level: Q2PrintLevel::Medium,
                    text: message,
                });
            }
            self.toss_weapon(actor.clone(), game);
            let state = player_state(game, &actor);
            let last_attack = game.require_entity(&actor).last_attack.clone();
            if !state.use_q2_weapons {
                if let Some(drop) = player_hooks(game).drop_inventory {
                    drop(actor.clone(), game, last_attack.clone());
                }
            }
            if let Some(before) = player_hooks(game).before_death_inventory {
                before(actor.clone(), game, last_attack);
            }
            if game.options.mode == Q2Mode::Coop && player_state(game, &actor).coop_respawn.is_some() {
                let current = game.host.inventory().entries(&actor);
                let entry = game
                    .players
                    .states
                    .get_mut(&actor)
                    .expect("Q2 player has not been admitted");
                if let Some(respawn) = entry.coop_respawn.as_mut() {
                    for slot in respawn.inventory.iter_mut() {
                        if slot.item.starts_with("q2:key_") {
                            if let Some(found) = current.iter().find(|item| item.item == slot.item) {
                                *slot = found.clone();
                            }
                        }
                    }
                }
            }
            self.dispatched_clear_death_inventory(actor.clone(), game);
            let deathmatch = game.options.mode == Q2Mode::Deathmatch;
            game.players
                .states
                .get_mut(&actor)
                .expect("Q2 player has not been admitted")
                .show_scores = deathmatch;
        }
        game.players
            .states
            .get_mut(&actor)
            .expect("Q2 player has not been admitted")
            .dead = true;
        self.clear_powerups(actor.clone(), game);
        let trackers = q2_entities_named(game, "pain daemon");
        for tracker in trackers {
            if game.require_entity(&tracker).enemy.as_ref() == Some(&actor) {
                game.remove_actor(tracker);
            }
        }
        if let Some(died) = player_hooks(game).death {
            died(actor.clone(), game, game.require_entity(&actor).last_attack.clone());
        }
        first
    }

    /// Toss the weapon on death (`tossWeapon`).
    pub fn toss_weapon(&self, actor: ActorId, game: &mut Q2GameServices) {
        if game.options.mode != Q2Mode::Deathmatch {
            return;
        }
        if !player_state(game, &actor).use_q2_weapons {
            return;
        }
        let weapon = game.weapons.states.get(&actor).and_then(|state| state.weapon.clone());
        let item = match weapon {
            None => None,
            Some(name) => {
                let definition = weapon_definition(game, &name);
                if definition.name != "blaster"
                    && definition
                        .ammo
                        .as_ref()
                        .is_none_or(|ammo| game.host.inventory().count(&actor, ammo) != 0.0)
                {
                    Some(definition.item.clone())
                } else {
                    None
                }
            }
        };
        let now = game.host.now();
        let quad = player_items(game).player_powerups(game, &actor).quad_until;
        let quad_fire = player_hooks(game)
            .quad_fire_drop_until
            .map(|until| until(actor.clone()))
            .unwrap_or(0.0);
        let drop_quad = game.options.deathmatch_flags & 16384 != 0 && quad > now + 1.0;
        let drop_quad_fire = quad_fire > now + 1.0;
        let spread = if item.is_none() {
            0.0
        } else if drop_quad {
            22.5
        } else if drop_quad_fire {
            12.5
        } else {
            0.0
        };
        let rerelease = game.options.edition == Q2Edition::Rerelease;
        if let Some(item) = item {
            player_items(game).drop(
                actor.clone(),
                game,
                &item,
                &Q2DropOptions {
                    immediate_touch: false,
                    player_death: true,
                    yaw_offset: Some(-spread),
                    expires_at: None,
                },
            );
        }
        if drop_quad {
            player_items(game).drop(
                actor.clone(),
                game,
                "q2:item_quad",
                &Q2DropOptions {
                    immediate_touch: rerelease,
                    player_death: true,
                    yaw_offset: Some(spread),
                    expires_at: Some(quad),
                },
            );
        }
        if drop_quad_fire {
            player_items(game).drop(
                actor.clone(),
                game,
                "q2:item_quadfire",
                &Q2DropOptions {
                    immediate_touch: true,
                    player_death: true,
                    yaw_offset: Some(spread),
                    expires_at: Some(quad_fire),
                },
            );
        }
    }

    /// Clear powerups (`clearPowerups`).
    pub fn clear_powerups(&self, actor: ActorId, game: &mut Q2GameServices) {
        player_items(game).clear_powerups(game, &actor);
        game.require_entity_mut(&actor).flags &= !4096;
        let owned = game.owned_of(actor.clone());
        game.host
            .combat()
            .set_powered_protection(&owned, &PoweredProtectionState::None);
        let god = player_state(game, &actor).god;
        game.host.combat().set_traits(
            &owned,
            &CombatTraitChanges {
                can_take_damage: None,
                mass: None,
                invulnerable: Some(god),
                team: None,
                no_knockback: None,
            },
        );
    }

    /// Run after client think (`afterClientThink`).
    pub fn after_client_think(&self, actor: ActorId, game: &mut Q2GameServices) {
        let movement = (player_hooks(game).movement)(actor.clone());
        self.latch_buttons(actor.clone(), game, movement.buttons);
        if let Q2Intermission::Intermission { started, .. } = game.players.intermission.clone() {
            if game.host.now() > started + 5.0 && player_state(game, &actor).buttons != 0 {
                if let Q2Intermission::Intermission { ref mut exit, .. } = game.players.intermission {
                    *exit = true;
                }
            }
            return;
        }
        let state = player_state(game, &actor);
        if state.spectator {
            if state.latched_buttons & 1 != 0 {
                game.players
                    .states
                    .get_mut(&actor)
                    .expect("Q2 player has not been admitted")
                    .latched_buttons &= !1;
                self.chase(actor.clone(), game, 1, true);
            }
        } else if state.use_q2_weapons {
            self.early_weapon_turn(actor.clone(), game);
        }
        let watchers: Vec<ActorId> = game
            .players
            .states
            .iter()
            .filter(|(_, watcher)| watcher.chase_target.as_ref() == Some(&actor))
            .map(|(watcher, _)| watcher.clone())
            .collect();
        for watcher in watchers {
            if game.entity(&watcher).is_some() {
                self.update_chase(watcher, game);
            }
        }
    }

    /// Latch weapon buttons into the player state.
    fn latch_buttons(&self, actor: ActorId, game: &mut Q2GameServices, buttons: i32) {
        let state = player_state(game, &actor);
        let mut turn = Q2WeaponTurnState {
            buttons: state.buttons,
            latched_buttons: state.latched_buttons,
            weapon_thunk: state.weapon_thunk,
        };
        latch_q2_weapon_buttons(&mut turn, buttons);
        let entry = game
            .players
            .states
            .get_mut(&actor)
            .expect("Q2 player has not been admitted");
        entry.buttons = turn.buttons;
        entry.latched_buttons = turn.latched_buttons;
        entry.weapon_thunk = turn.weapon_thunk;
    }

    /// Run the early weapon turn (`earlyQ2WeaponTurn`).
    fn early_weapon_turn(&self, actor: ActorId, game: &mut Q2GameServices) {
        let state = player_state(game, &actor);
        let mut turn = Q2WeaponTurnState {
            buttons: state.buttons,
            latched_buttons: state.latched_buttons,
            weapon_thunk: state.weapon_thunk,
        };
        let input = (player_hooks(game).weapon_input)(actor.clone());
        let owned = game.owned_of(actor.clone());
        early_q2_weapon_turn(&mut turn, &mut |latched| {
            tick_player_weapon(
                game,
                &owned,
                Q2WeaponInput {
                    latched_attack: latched,
                    ..input.clone()
                },
            );
        });
        let entry = game
            .players
            .states
            .get_mut(&actor)
            .expect("Q2 player has not been admitted");
        entry.buttons = turn.buttons;
        entry.latched_buttons = turn.latched_buttons;
        entry.weapon_thunk = turn.weapon_thunk;
    }

    /// Run begin frame (`beginFrame`).
    pub fn begin_frame(&self, actor: ActorId, game: &mut Q2GameServices) {
        if !matches!(game.players.intermission, Q2Intermission::Playing) {
            return;
        }
        let state = player_state(game, &actor);
        let now = game.host.now();
        if game.options.mode == Q2Mode::Deathmatch
            && state.requested_spectator != state.spectator
            && now - state.respawn_time >= 5.0
        {
            self.spectator_respawn(actor, game);
            return;
        }
        let allowed = state.use_q2_weapons && !state.spectator;
        {
            let mut turn = Q2WeaponTurnState {
                buttons: state.buttons,
                latched_buttons: state.latched_buttons,
                weapon_thunk: state.weapon_thunk,
            };
            let input = (player_hooks(game).weapon_input)(actor.clone());
            let owned = game.owned_of(actor.clone());
            begin_q2_weapon_turn(&mut turn, allowed, &mut |latched| {
                tick_player_weapon(
                    game,
                    &owned,
                    Q2WeaponInput {
                        latched_attack: latched,
                        ..input.clone()
                    },
                );
            });
            let entry = game
                .players
                .states
                .get_mut(&actor)
                .expect("Q2 player has not been admitted");
            entry.buttons = turn.buttons;
            entry.latched_buttons = turn.latched_buttons;
            entry.weapon_thunk = turn.weapon_thunk;
        }
        if player_state(game, &actor).dead {
            self.dispatched_dead_frame(actor, game);
            return;
        }
        if game.options.mode != Q2Mode::Deathmatch {
            let origin = game.body_of(actor.clone()).origin;
            (player_hooks(game).emit)(Q2PlayerEvent::Trail {
                actor: actor.clone(),
                origin,
                time: now,
            });
        }
        game.players
            .states
            .get_mut(&actor)
            .expect("Q2 player has not been admitted")
            .latched_buttons = 0;
    }

    /// Run the dead frame (`deadFrame`).
    pub fn dead_frame(&self, actor: ActorId, game: &mut Q2GameServices) {
        let state = player_state(game, &actor);
        let deathmatch = game.options.mode == Q2Mode::Deathmatch;
        let mask = if deathmatch { 1 } else { -1 };
        if game.host.now() > state.respawn_time
            && (state.latched_buttons & mask != 0 || deathmatch && game.options.deathmatch_flags & 1024 != 0)
        {
            self.dispatched_respawn(actor.clone(), game);
            game.players
                .states
                .get_mut(&actor)
                .expect("Q2 player has not been admitted")
                .latched_buttons = 0;
        }
    }

    /// Run world effects (`worldEffects`).
    pub fn world_effects(&self, actor: ActorId, game: &mut Q2GameServices) {
        q2_world_effects(&mut self.context(actor, game));
    }

    /// Run falling damage (`fallingDamage`).
    pub fn falling_damage(&self, actor: ActorId, game: &mut Q2GameServices) {
        q2_falling_damage(&mut self.context(actor, game));
    }

    /// Build the player view (`buildView`).
    pub fn build_view(
        &self,
        actor: ActorId,
        game: &mut Q2GameServices,
        flashes: i32,
        intermission: bool,
    ) -> Q2PlayerView {
        q2_build_view(&mut self.context(actor, game), flashes, intermission)
    }

    /// Run damage feedback (`damageFeedback`).
    pub fn damage_feedback(&self, actor: ActorId, game: &mut Q2GameServices, pain_index: i32) -> (i32, i32) {
        q2_damage_feedback(&mut self.context(actor, game), pain_index)
    }

    /// Run client animation (`clientAnimation`).
    pub fn client_animation(&self, actor: ActorId, game: &mut Q2GameServices) {
        q2_client_animation(&mut self.context(actor, game));
    }

    /// Update view bob (`updateBob`).
    pub fn update_bob(&self, actor: ActorId, game: &mut Q2GameServices) {
        let movement = (player_hooks(game).movement)(actor.clone());
        let body = game.body_of(actor.clone());
        let speed = f64::from(body.velocity.x).hypot(f64::from(body.velocity.y));
        let entry = game
            .players
            .states
            .get_mut(&actor)
            .expect("Q2 player has not been admitted");
        if speed < 5.0 {
            entry.bob_move = 0.0;
            entry.bob_time = 0.0;
        } else if movement.grounded {
            entry.bob_move = if speed > 210.0 {
                0.25
            } else if speed > 100.0 {
                0.125
            } else {
                0.0625
            };
        }
        entry.bob_time += entry.bob_move;
    }

    /// Run end frame (`endFrame`).
    pub fn end_frame(&self, actor: ActorId, game: &mut Q2GameServices) {
        let now = game.host.now();
        if !matches!(game.players.intermission, Q2Intermission::Playing) {
            let view = self.dispatched_build_view(actor.clone(), game, 0, true);
            (player_hooks(game).emit)(Q2PlayerEvent::View { actor, view });
            return;
        }
        let powers = player_items(game).player_powerups(game, &actor);
        let god = player_state(game, &actor).god;
        let owned = game.owned_of(actor.clone());
        game.host.combat().set_traits(
            &owned,
            &CombatTraitChanges {
                can_take_damage: None,
                mass: None,
                invulnerable: Some(god || powers.invulnerability_until > now),
                team: None,
                no_knockback: None,
            },
        );
        self.dispatched_world_effects(actor.clone(), game);
        let movement = (player_hooks(game).movement)(actor.clone());
        let body = game.body_of(actor.clone());
        let vectors = angle_vectors(movement.view_angles);
        let side = dot3(body.velocity, vectors.right);
        let rules = game.players.rules.clone();
        let roll = (if side < 0.0 { -1.0 } else { 1.0 })
            * (f64::from(side.abs()) * rules.roll_angle / rules.roll_speed).min(rules.roll_angle);
        if movement.animate_q2 {
            let pitch = if movement.view_angles.x > 180.0 {
                movement.view_angles.x - 360.0
            } else {
                movement.view_angles.x
            };
            let mut moved = body.clone();
            moved.angles = vec3(pitch / 3.0, movement.view_angles.y, (roll * 4.0) as f32);
            game.write_body(actor.clone(), &moved, false);
        }
        let speed = f64::from(body.velocity.x).hypot(f64::from(body.velocity.y));
        self.dispatched_update_bob(actor.clone(), game);
        self.dispatched_falling_damage(actor.clone(), game);
        let pain_index = game.players.pain_animation;
        let feedback = self.dispatched_damage_feedback(actor.clone(), game, pain_index);
        game.players.pain_animation = feedback.1;
        let view = self.dispatched_build_view(actor.clone(), game, feedback.0, false);
        let view_angles = view.angles;
        (player_hooks(game).emit)(Q2PlayerEvent::View {
            actor: actor.clone(),
            view,
        });
        let state = player_state(game, &actor);
        let cycle = if movement.ducked {
            state.bob_time * 4.0
        } else {
            state.bob_time
        }
        .trunc();
        if state.event.is_empty()
            && movement.grounded
            && speed > 225.0
            && (state.bob_time + state.bob_move).trunc() != cycle
        {
            game.players
                .states
                .get_mut(&actor)
                .expect("Q2 player has not been admitted")
                .event = "q2:footstep".to_string();
        }
        let event = player_state(game, &actor).event.clone();
        if !event.is_empty() {
            let number = match event.as_str() {
                "q2:footstep" => Some(2),
                "q2:fall-short" => Some(3),
                "q2:fall" => Some(4),
                "q2:fall-far" => Some(5),
                "q2:player-teleport" => Some(6),
                _ => None,
            };
            match number {
                Some(number) => game.host.emit(Q2PresentationEvent::EntityEvent {
                    actor: actor.clone(),
                    event: number,
                }),
                None => game.host.emit(Q2PresentationEvent::Effect(Q2EffectEvent {
                    effect: event,
                    origin: body.origin,
                    direction: Vec3::default(),
                    count: 1,
                    color: 0,
                })),
            }
            game.players
                .states
                .get_mut(&actor)
                .expect("Q2 player has not been admitted")
                .event = String::new();
        }
        q2_client_effects(&mut self.context(actor.clone(), game));
        self.dispatched_client_animation(actor.clone(), game);
        if movement.animate_q2 {
            game.show(actor.clone());
        }
        {
            let entry = game
                .players
                .states
                .get_mut(&actor)
                .expect("Q2 player has not been admitted");
            entry.old_velocity = body.velocity;
            entry.old_view_angles = view_angles;
        }
        if player_state(game, &actor).show_scores && js_round(now * 10.0) as i32 & 31 == 0 {
            self.scoreboard(actor, game, false);
        }
    }

    /// Record damage (`recordDamage`).
    pub fn record_damage(&self, actor: ActorId, game: &mut Q2GameServices, decision: &DamageDecision) {
        if !game.players.states.contains_key(&actor) {
            return;
        }
        game.require_entity_mut(&actor).last_attack = Some(decision.request.attack.clone());
        if decision.reaction == DamageReactionKind::Death {
            return;
        }
        let (blood, armor, power_armor, knockback) = match decision.feedback {
            Some(DamageFeedback::Q2 {
                blood,
                armor,
                power_armor,
                knockback,
            }) => (blood, armor, power_armor, knockback),
            _ => (decision.applied_damage, 0.0, 0.0, decision.request.knockback),
        };
        let now = game.host.now();
        let entry = game
            .players
            .states
            .get_mut(&actor)
            .expect("Q2 player has not been admitted");
        entry.damage_blood += blood;
        entry.damage_armor += armor;
        entry.damage_power_armor += power_armor;
        if power_armor > 0.0 {
            entry.power_armor_time = now + 0.2;
        }
        entry.damage_knockback += knockback;
        entry.damage_from = decision.request.point;
    }

    /// Handle a weapon event (`weaponEvent`).
    pub fn weapon_event(&self, game: &mut Q2GameServices, event: &Q2WeaponEvent) {
        if let Q2WeaponEvent::PlayerAnimation {
            actor,
            priority,
            first,
            last,
            ..
        } = event
        {
            if !game.players.states.contains_key(actor) {
                return;
            }
            let priority = match priority {
                PlayerAnimationPriority::Attack => 4,
                PlayerAnimationPriority::Pain => 3,
                PlayerAnimationPriority::Reverse => 6,
            };
            game.players
                .states
                .get_mut(actor)
                .expect("Q2 player has not been admitted")
                .animation_priority = priority;
            game.players
                .states
                .get_mut(actor)
                .expect("Q2 player has not been admitted")
                .animation_end = *last;
            game.require_entity_mut(actor).frame = *first;
        }
    }

    /// Teleport a player (`teleportPlayer`).
    pub fn teleport_player(&self, actor: ActorId, game: &mut Q2GameServices, origin: Vec3, angles: Vec3) {
        let movement = (player_hooks(game).movement)(actor.clone());
        let spectator = player_state(game, &actor).spectator;
        game.players
            .states
            .get_mut(&actor)
            .expect("Q2 player has not been admitted")
            .event = "q2:player-teleport".to_string();
        (player_hooks(game).set_movement)(
            actor,
            Q2PlayerMovementChange::Teleport(Q2PlayerSpawnChange {
                origin,
                velocity: Vec3::default(),
                angles,
                command_angles: movement.command_angles,
                hold_milliseconds: 160,
                spectator,
            }),
        );
    }

    /// Respawn a spectator (`spectatorRespawn`).
    fn spectator_respawn(&self, actor: ActorId, game: &mut Q2GameServices) {
        let userinfo = player_state(game, &actor).userinfo.clone();
        let result = self.dispatched_connect(game, &userinfo);
        if !result.allowed {
            let spectator = player_state(game, &actor).spectator;
            game.players
                .states
                .get_mut(&actor)
                .expect("Q2 player has not been admitted")
                .requested_spectator = spectator;
            let reason = result.reason.unwrap_or_default();
            (player_hooks(game).emit)(Q2PlayerEvent::Print {
                target: Some(actor.clone()),
                level: Q2PrintLevel::High,
                text: format!("{reason}\n"),
            });
            (player_hooks(game).emit)(Q2PlayerEvent::StuffText {
                actor,
                text: format!("spectator {}\n", if spectator { 1 } else { 0 }),
            });
            return;
        }
        game.players
            .states
            .get_mut(&actor)
            .expect("Q2 player has not been admitted")
            .score = 0;
        self.dispatched_put_in_server(actor.clone(), game, true, None);
        let now = game.host.now();
        game.players
            .states
            .get_mut(&actor)
            .expect("Q2 player has not been admitted")
            .respawn_time = now;
        let state = player_state(game, &actor);
        (player_hooks(game).emit)(Q2PlayerEvent::Print {
            target: None,
            level: Q2PrintLevel::High,
            text: format!(
                "{} {}\n",
                state.name,
                if state.spectator {
                    "has moved to the sidelines"
                } else {
                    "joined the game"
                }
            ),
        });
        if !player_state(game, &actor).spectator {
            game.players
                .states
                .get_mut(&actor)
                .expect("Q2 player has not been admitted")
                .event = "q2:player-teleport".to_string();
        }
    }

    /// Disconnect a player (`disconnect`).
    pub fn disconnect(&self, actor: ActorId, game: &mut Q2GameServices) {
        let name = player_state(game, &actor).name.clone();
        (player_hooks(game).emit)(Q2PlayerEvent::Print {
            target: None,
            level: Q2PrintLevel::High,
            text: format!("{name} disconnected\n"),
        });
        let trackers = q2_entities_named(game, "pain daemon");
        for tracker in trackers {
            if game.require_entity(&tracker).enemy.as_ref() == Some(&actor) {
                game.remove_actor(tracker);
            }
        }
        if let Some(disconnect) = player_hooks(game).disconnect {
            disconnect(actor.clone(), game);
        }
        let slot = player_state(game, &actor).slot;
        game.players
            .states
            .get_mut(&actor)
            .expect("Q2 player has not been admitted")
            .connected = false;
        game.require_entity_mut(&actor).visible = false;
        game.set_solid(actor.clone(), Q2Solid::None);
        game.show(actor.clone());
        let origin = game.body_of(actor.clone()).origin;
        game.host.emit(Q2PresentationEvent::Effect(Q2EffectEvent {
            effect: "q2:logout".to_string(),
            origin,
            direction: Vec3::default(),
            count: 1,
            color: 0,
        }));
        (player_hooks(game).emit)(Q2PlayerEvent::Userinfo {
            actor: actor.clone(),
            slot,
            name: String::new(),
            skin: String::new(),
        });
        game.players.states.remove(&actor);
    }

    /// Send the scoreboard (`scoreboard`).
    pub fn scoreboard(&self, actor: ActorId, game: &mut Q2GameServices, reliable: bool) {
        let rows = score_rows(game);
        let killer = game.require_entity(&actor).enemy.clone();
        (player_hooks(game).emit)(Q2PlayerEvent::Scoreboard {
            actor,
            rows,
            killer,
            reliable,
        });
    }

    /// Chase (`chase`).
    pub fn chase(&self, actor: ActorId, game: &mut Q2GameServices, direction: i32, toggle: bool) {
        let target = player_state(game, &actor).chase_target.clone();
        if toggle && target.is_some() {
            game.players
                .states
                .get_mut(&actor)
                .expect("Q2 player has not been admitted")
                .chase_target = None;
        } else {
            let mut candidates: Vec<(ActorId, i32)> = game
                .players
                .states
                .iter()
                .filter(|(_, state)| state.connected && !state.spectator)
                .map(|(id, state)| (id.clone(), state.slot))
                .collect();
            candidates.sort_by(|left, right| left.1.cmp(&right.1));
            let index = candidates
                .iter()
                .position(|(id, _)| Some(id) == target.as_ref())
                .map_or(-1, |found| found as i32);
            let next = if candidates.is_empty() {
                None
            } else {
                let raw = (index + direction + candidates.len() as i32) % candidates.len() as i32;
                candidates.get(raw as usize).map(|(id, _)| id.clone())
            };
            game.players
                .states
                .get_mut(&actor)
                .expect("Q2 player has not been admitted")
                .chase_target = next;
        }
        let target = player_state(game, &actor).chase_target.clone();
        (player_hooks(game).emit)(Q2PlayerEvent::Chase {
            actor: actor.clone(),
            target: target.clone(),
        });
        if target.is_some() {
            self.update_chase(actor, game);
        } else {
            (player_hooks(game).set_movement)(actor, Q2PlayerMovementChange::Noclip { enabled: true });
        }
    }

    /// Update chase (`updateChase`).
    pub fn update_chase(&self, actor: ActorId, game: &mut Q2GameServices) {
        let target = player_state(game, &actor).chase_target.clone();
        let live = target.as_ref().and_then(|target| {
            let state = game.players.states.get(target)?;
            if !state.connected || state.spectator || game.entity(target).is_none() {
                return None;
            }
            Some(target.clone())
        });
        let Some(target) = live else {
            if player_state(game, &actor).chase_target.is_some() {
                game.players
                    .states
                    .get_mut(&actor)
                    .expect("Q2 player has not been admitted")
                    .chase_target = None;
                self.chase(actor, game, 1, false);
            }
            return;
        };
        let target_body = game.body_of(target.clone());
        let movement = (player_hooks(game).movement)(target.clone());
        let view_height = game.require_entity(&target).view_height;
        let eye = vec3(
            target_body.origin.x,
            target_body.origin.y,
            target_body.origin.z + view_height as f32,
        );
        let pitched = vec3(
            movement.view_angles.x.min(56.0),
            movement.view_angles.y,
            movement.view_angles.z,
        );
        let forward = angle_vectors(pitched).forward;
        let behind = add3(eye, scale3(forward, -30.0));
        let mut desired = behind;
        desired.z = behind.z.max(target_body.origin.z + 20.0) + if movement.grounded { 0.0 } else { 16.0 };
        let trace = |game: &mut Q2GameServices, start: Vec3, end: Vec3| {
            game.host.trace(&Q2TraceRequest {
                start,
                end,
                bounds: None,
                ignore: Some(target.clone()),
                mask: 3,
                exclude: Vec::new(),
            })
        };
        let mut goal = add3(trace(game, eye, desired).end, scale3(forward, 2.0));
        let ceiling = trace(game, goal, vec3(goal.x, goal.y, goal.z + 6.0));
        if ceiling.fraction < 1.0 {
            goal = vec3(ceiling.end.x, ceiling.end.y, ceiling.end.z - 6.0);
        }
        let floor = trace(game, goal, vec3(goal.x, goal.y, goal.z - 6.0));
        if floor.fraction < 1.0 {
            goal = vec3(floor.end.x, floor.end.y, floor.end.z + 6.0);
        }
        let target_state = player_state(game, &target);
        let angles = if target_state.dead {
            vec3(-15.0, target_state.killer_yaw as f32, 40.0)
        } else {
            movement.view_angles
        };
        game.require_entity_mut(&actor).view_height = 0;
        let mut moved = game.body_of(actor.clone());
        moved.origin = goal;
        moved.velocity = Vec3::default();
        game.write_body(actor.clone(), &moved, true);
        (player_hooks(game).set_movement)(actor, Q2PlayerMovementChange::Freeze { origin: goal, angles });
    }

    /// Begin intermission (`beginIntermission`).
    pub fn begin_intermission(&self, game: &mut Q2GameServices, map: String, landmark: Option<Q2LandmarkCarry>) {
        if !matches!(game.players.intermission, Q2Intermission::Playing) {
            return;
        }
        let actors: Vec<ActorId> = game.players.states.keys().cloned().collect();
        for actor in &actors {
            if game.entity(actor).is_some() && game.host.combat().read(actor).map_or(0.0, |combat| combat.health) <= 0.0
            {
                self.dispatched_put_in_server(actor.clone(), game, true, None);
            }
        }
        let end_unit = map.contains('*');
        if end_unit && game.options.mode == Q2Mode::Coop {
            let actors: Vec<ActorId> = game.players.states.keys().cloned().collect();
            for actor in actors {
                let Some(owned) = game.host.actors().resolve_owned(&actor) else {
                    continue;
                };
                for mut entry in game.host.inventory().entries(&actor) {
                    if entry.item.starts_with("q2:key_") {
                        entry.count = 0.0;
                        game.host.inventory().configure(&owned, &entry);
                    }
                }
            }
        }
        let exit = !end_unit && game.options.mode != Q2Mode::Deathmatch;
        let started = game.host.now();
        game.players.intermission = Q2Intermission::Intermission {
            map,
            started,
            exit,
            landmark,
        };
        if exit {
            return;
        }
        let authored = q2_entities_named(game, "info_player_intermission");
        let spot = if authored.is_empty() {
            let starts = q2_entities_named(game, "info_player_start");
            let deaths = q2_entities_named(game, "info_player_deathmatch");
            starts.first().or_else(|| deaths.first()).cloned()
        } else {
            let pick = ((game.host.random() * 4.0).floor() as usize & 3) % authored.len();
            authored.get(pick).cloned()
        };
        let Some(spot) = spot else {
            panic!("Q2 intermission has no authored camera or player spawn");
        };
        let camera = game.body_of(spot);
        let actors: Vec<ActorId> = game.players.states.keys().cloned().collect();
        for actor in actors {
            if game.entity(&actor).is_none() {
                continue;
            }
            let show_scores = game.options.mode != Q2Mode::Singleplayer;
            {
                let state = game
                    .players
                    .states
                    .get_mut(&actor)
                    .expect("Q2 player has not been admitted");
                state.show_scores = show_scores;
                state.loop_sound = String::new();
            }
            self.clear_powerups(actor.clone(), game);
            {
                let entity = game.require_entity_mut(&actor);
                entity.view_height = 0;
                entity.visible = false;
                entity.effects = 0;
            }
            game.set_solid(actor.clone(), Q2Solid::None);
            let mut moved = game.body_of(actor.clone());
            moved.origin = camera.origin;
            moved.velocity = Vec3::default();
            game.write_body(actor.clone(), &moved, true);
            game.show(actor.clone());
            (player_hooks(game).set_movement)(
                actor.clone(),
                Q2PlayerMovementChange::Freeze {
                    origin: camera.origin,
                    angles: camera.angles,
                },
            );
            if show_scores {
                self.scoreboard(actor, game, true);
            }
        }
    }

    /// Check rules (`checkRules`).
    pub fn check_rules(&self, game: &mut Q2GameServices) {
        if let Q2Intermission::Intermission {
            map, landmark, exit, ..
        } = game.players.intermission.clone()
        {
            if !exit {
                return;
            }
            if landmark.is_some() {
                let actors: Vec<ActorId> = game.players.states.keys().cloned().collect();
                for actor in actors {
                    if game.entity(&actor).is_some() {
                        self.dispatched_end_frame(actor, game);
                    }
                }
            }
            game.players.intermission = Q2Intermission::Playing;
            if landmark.is_none() {
                let actors: Vec<ActorId> = game.players.states.keys().cloned().collect();
                for actor in actors {
                    if game.entity(&actor).is_some() {
                        self.dispatched_end_frame(actor, game);
                    }
                }
            }
            self.dispatched_before_exit_level(game, &map);
            let actors: Vec<ActorId> = game.players.states.keys().cloned().collect();
            for actor in &actors {
                if game.entity(actor).is_none() {
                    continue;
                }
                let health = game.host.combat().read(actor).map_or(0.0, |combat| combat.health);
                let max_health = game.require_entity(actor).max_health;
                if health > max_health {
                    let owned = game.owned_of(actor.clone());
                    game.host.combat().set_health(&owned, max_health);
                }
            }
            let server_flags = game.counters.server_flags;
            game.host.prepare_level_change(&map, landmark.as_ref(), server_flags);
            let unit_map = map.strip_prefix('*').unwrap_or(&map);
            let parts: Vec<&str> = unit_map.split('$').collect();
            let destination = parts.first().copied().unwrap_or("");
            if destination.is_empty() {
                panic!("Q2 transition has no map destination");
            }
            let spawn = parts.get(1).copied().unwrap_or("");
            let campaign = game.options.campaign.clone();
            game.host.transition(TransitionIntent::campaign_level(
                campaign,
                format!("q2:{destination}"),
                spawn.to_string(),
                Vec::new(),
                None,
            ));
            return;
        }
        if game.options.mode != Q2Mode::Deathmatch {
            return;
        }
        let now = game.host.now();
        let rules = game.players.rules.clone();
        let mut message = String::new();
        if rules.time_limit_minutes != 0 && now >= f64::from(rules.time_limit_minutes) * 60.0 {
            message = "Timelimit hit.\n".to_string();
        } else if rules.frag_limit != 0
            && game
                .players
                .states
                .values()
                .any(|state| state.connected && state.score >= rules.frag_limit)
        {
            message = "Fraglimit hit.\n".to_string();
        }
        if !message.is_empty() {
            (player_hooks(game).emit)(Q2PlayerEvent::Print {
                target: None,
                level: Q2PrintLevel::High,
                text: message,
            });
            self.end_deathmatch_level(game);
        }
    }

    /// Run before exiting the level (`beforeExitLevel`).
    pub fn before_exit_level(&self, _game: &mut Q2GameServices, _map: &str) {}

    /// End the deathmatch level (`endDeathmatchLevel`).
    pub fn end_deathmatch_level(&self, game: &mut Q2GameServices) {
        let mut next = game.options.map_name.clone();
        if game.options.deathmatch_flags & 32 == 0 {
            let maps = game.players.rules.map_list.clone();
            let lowered = game.options.map_name.to_lowercase();
            match maps.iter().position(|map| map.to_lowercase() == lowered) {
                Some(index) => {
                    if game.options.edition == Q2Edition::Rerelease
                        && game.players.rules.map_list_shuffle
                        && maps.len() > 1
                        && index == maps.len() - 1
                    {
                        let mut shuffled = maps.clone();
                        for i in (1..shuffled.len()).rev() {
                            let bound = i as i32 + 1;
                            let j = match game.host.rerelease_random() {
                                Some(source) => source.integer_max(bound),
                                None => (game.host.random() * f64::from(bound)).floor() as i32,
                            };
                            if j < 0 || j as usize >= shuffled.len() {
                                panic!("Q2 map shuffle index outside rotation");
                            }
                            shuffled.swap(i, j as usize);
                        }
                        if shuffled.first().is_some_and(|first| *first == game.options.map_name) {
                            let last = shuffled.len() - 1;
                            shuffled.swap(0, last);
                        }
                        next = shuffled.first().cloned().unwrap_or(next);
                        game.players.rules.map_list = shuffled;
                    } else if game.options.edition != Q2Edition::Rerelease || maps.len() != 1 {
                        next = maps[(index + 1) % maps.len()].clone();
                    }
                }
                None => {
                    if !game.players.rules.next_map.is_empty() {
                        next = game.players.rules.next_map.clone();
                    } else if let Some(change) = q2_entities_named(game, "target_changelevel").first().cloned() {
                        if let Some(map) = game.require_entity(&change).spawn.values.get("map").cloned() {
                            if !map.is_empty() {
                                next = map;
                            }
                        }
                    }
                }
            }
        }
        self.dispatched_begin_intermission(game, next, None);
    }

    /// Whether a password is needed (`needPassword`).
    pub fn need_password(&self, game: &Q2GameServices) -> i32 {
        let rules = &game.players.rules;
        let password = i32::from(!rules.password.is_empty() && rules.password.to_lowercase() != "none");
        let spectator =
            i32::from(!rules.spectator_password.is_empty() && rules.spectator_password.to_lowercase() != "none");
        password | spectator << 1
    }

    /// Whether chat is allowed (`chatAllowed`).
    pub fn chat_allowed(&self, actor: ActorId, game: &mut Q2GameServices) -> bool {
        game.entity(&actor).is_some()
            && game.players.states.contains_key(&actor)
            && q2_chat_allowed(&mut self.context(actor, game))
    }

    /// Run a client command (`clientCommand`).
    pub fn client_command(&self, actor: ActorId, game: &mut Q2GameServices, command: &str, args: &[String]) -> bool {
        run_q2_client_command(&mut self.context(actor, game), command, args)
    }
}

/// Whether coop-stay items may drop (`canDropCoopStayItems`).
pub fn can_drop_coop_stay_items() -> bool {
    false
}

/// Send the scoreboard (`scoreboard`).
pub fn send_scoreboard(actor: &ActorId, game: &mut Q2GameServices, reliable: bool) {
    bound_module(game).scoreboard(actor.clone(), game, reliable);
}

/// Chase (`chase`).
pub fn chase_player(actor: ActorId, game: &mut Q2GameServices, direction: i32) {
    bound_module(game).chase(actor, game, direction, false);
}

/// Parse userinfo pairs in order (`q2Userinfo`).
fn userinfo_pairs(source: &str) -> Vec<(String, String)> {
    let stripped = source.strip_prefix('\\').unwrap_or(source);
    let fields: Vec<&str> = stripped.split('\\').collect();
    let mut pairs = Vec::new();
    let mut index = 0;
    while index + 1 < fields.len() {
        let key = fields[index].to_string();
        if !pairs.iter().any(|(seen, _): &(String, String)| *seen == key) {
            pairs.push((key, fields[index + 1].to_string()));
        }
        index += 2;
    }
    pairs
}

/// Rebuild userinfo with a rejection message.
fn format_userinfo_with_rejmsg(source: &str, reason: &str) -> String {
    let mut pairs: Vec<(String, String)> = userinfo_pairs(source)
        .into_iter()
        .filter(|(key, _)| key != "rejmsg")
        .collect();
    pairs.push(("rejmsg".to_string(), reason.to_string()));
    let mut out = String::new();
    for (key, value) in pairs {
        out.push('\\');
        out.push_str(&key);
        out.push('\\');
        out.push_str(&value);
    }
    out
}
