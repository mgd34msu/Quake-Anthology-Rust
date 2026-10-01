//! Q2 product runtime (`/home/buzzkill/Projects/quake-typescript/src/content/composition/q2/index.ts`).
//!
//! The donor builds the game, weapons, and session host inside the
//! composition root. The port registers into a session-owned
//! [`Q2GameServices`](crate::q2::foundation::host::Q2GameServices) arena
//! instead: [`create_q2_product_runtime`] installs every module, pushes the
//! spawn modules in donor order, and stores the assembled
//! [`Q2ProductRuntime`] back into the arena so the composed hook functions
//! below can read configuration state without capturing closures.

use qa_core::identity::{ActorId, OwnedActor};
use qa_core::math::Vec3;

use super::match_::Q2ProductMatch;
use super::types::{
    Q2CompositionCommon, Q2CompositionEvent, Q2CompositionOptions, Q2DeathmatchFlagsHooks, Q2ForeignPowerups,
    Q2MatchSelection,
};
use super::{composition_emit, composition_services};
use crate::contract::ItemId;
use crate::q2::base::entities::types::Q2BaseEntityHooks;
use crate::q2::base::entities::{create_q2_base_entity_module, Q2BaseEntityModule, Q2PlatformPhase};
use crate::q2::base::monsters::registry::register_q2_classic_base_monsters;
use crate::q2::base::player::index::{
    create_q2_players, player_hooks, player_items, Q2Intermission, Q2PlayerAdmission, Q2Players,
};
use crate::q2::base::player::spawns::{spawn_callbacks, spawn_player_spawn};
use crate::q2::base::player::types::{Q2PlayerHooks, Q2SpawnPlacement};
use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::host::{DeathmatchFlags, Q2Edition, Q2GameServices, Q2LandmarkCarry, Q2Mode, SpawnModule};
use crate::q2::foundation::items::{create_q2_item_module, Q2ItemHooks, Q2ItemModule, Q2PickupPolicy};
use crate::q2::foundation::monsters::types::{MonsterContext, PlatformPhase};
use crate::q2::foundation::monsters::{
    monster_spawn, resume_monster, spawn_infantry_driver, touch_combat_point, touch_path_corner,
};
use crate::q2::foundation::movers::{create_q2_mover_module, Q2MoverHooks, Q2MoverModule};
use crate::q2::foundation::scenery::create_q2_scenery_module;
use crate::q2::foundation::targets::create_q2_target_module;
use crate::q2::foundation::weapons::ballistics::q2_ballistics_callbacks;
use crate::q2::foundation::weapons::types::Q2WeaponInput;
use crate::q2::missionpacks::doppleganger::doppleganger_callbacks;
use crate::q2::missionpacks::entities::movers::{Q2Plat2Phase, Q2RogueMovers};
use crate::q2::missionpacks::entities::types::{Q2MissionPackEntityEvent, Q2MissionPackEntityHooks};
use crate::q2::missionpacks::entities::Q2MissionPackEntities;
use crate::q2::missionpacks::items::mission_item_spawn;
use crate::q2::missionpacks::modes::deathball::q2_deathball_rules;
use crate::q2::missionpacks::monsters::hints::finalize_hint_paths;
use crate::q2::missionpacks::monsters::types::Q2MonsterMissionPack;
use crate::q2::missionpacks::monsters::{mission_pack_target_anger, register_q2_mission_pack_monsters};
use crate::q2::missionpacks::players::{rogue_player_spawn_module, Q2RoguePlayerSpawns};
use crate::q2::missionpacks::projectiles::mission_projectile_callbacks;
use crate::q2::missionpacks::spheres::{sphere_callbacks, Q2SphereHooks};
use crate::q2::missionpacks::types::{Q2MissionPack, Q2MissionPackPlayerEffect, Q2MissionPackProjectileHooks};
use crate::q2::missionpacks::{register_q2_mission_pack_armory, Q2MissionPackArmory, Q2MissionPackArmoryHooks};
use crate::q2::multiplayer::ctf::ctf_hooks;
use crate::q2::multiplayer::ctf::index::Q2Ctf;
use crate::q2::rerelease::index::{
    create_q2_rerelease_module, rerelease_after_pickup, rerelease_before_pickup, rerelease_before_targets,
    rerelease_can_pickup, rerelease_instanced_coop, rerelease_keep_after_pickup, rerelease_module_callbacks,
    rerelease_module_spawn, Q2RereleaseModule,
};
use crate::q2::rerelease::monsters::index::register_q2_rerelease_monsters;
use crate::q2::rerelease::players::Q2RereleasePlayers;
use crate::q2::rerelease::types::Q2RereleaseHooks;
use crate::q2::support::contracts::{AttackProvenance, DamageDecision};

/// Mission-pack expansion (`Q2ProductExpansion`).
#[derive(Debug, Clone)]
pub struct Q2ProductExpansion {
    /// Mission pack.
    pub pack: Q2MissionPack,
    /// Pack entities.
    pub entities: Q2MissionPackEntities,
    /// Original-source fallback classnames.
    pub fallbacks: Vec<String>,
}

/// Rerelease product slice (`Q2ProductRerelease`).
#[derive(Debug, Clone, Copy)]
pub struct Q2ProductRerelease {
    /// Rerelease players.
    pub players: Q2RereleasePlayers,
    /// Rerelease entities.
    pub entities: Q2RereleaseModule,
}

/// Assembled Q2 product runtime (`Q2ProductRuntime`).
///
/// Handles are cheap copies; the arena owns the authoritative state. The
/// runtime is also stored in
/// [`CompositionRuntime`](super::CompositionRuntime) so composed hooks can
/// read captured configuration.
#[derive(Debug, Clone)]
pub struct Q2ProductRuntime {
    /// Composition program name.
    pub program: String,
    /// Resolved match selection.
    pub selection: Q2MatchSelection,
    /// Product match.
    pub product_match: Q2ProductMatch,
    /// Item module.
    pub items: Q2ItemModule,
    /// Mover module.
    pub movers: Q2MoverModule,
    /// Base entity module.
    pub base_entities: Q2BaseEntityModule,
    /// Players module.
    pub players: Q2Players,
    /// Mission-pack armory.
    pub armory: Option<Q2MissionPackArmory>,
    /// Mission-pack expansions.
    pub expansions: Vec<Q2ProductExpansion>,
    /// Rerelease slice.
    pub rerelease: Option<Q2ProductRerelease>,
    /// Rogue player spawns.
    pub rogue_spawns: Option<Q2RoguePlayerSpawns>,
    /// Deathball movement stop speed.
    pub movement_stop_speed: Option<f64>,
    /// Active packs (`packs`).
    pub packs: Vec<Q2MissionPack>,
    /// Session item hooks (read by composed item hooks).
    pub item_hooks: Q2ItemHooks,
    /// Session player hooks (read by composed player hooks).
    pub player_hooks: Q2PlayerHooks,
    /// Session expansion-powerup clear hook.
    pub clear_expansion_powerups: Option<fn(ActorId, &mut Q2GameServices)>,
}

/// Read the assembled product runtime from the arena.
fn product(game: &Q2GameServices) -> &Q2ProductRuntime {
    super::product_runtime(game)
}

/// Rebuild the players handle from the arena.
fn product_players(game: &Q2GameServices) -> Q2Players {
    create_q2_players(player_items(game), player_hooks(game))
}

/// Active packs for an edition/program pair (`packs`).
fn q2_product_packs(edition: Q2Edition, program: &str) -> Vec<Q2MissionPack> {
    if edition == Q2Edition::Rerelease {
        return vec![Q2MissionPack::Xatrix, Q2MissionPack::Rogue];
    }
    match program {
        "xatrix" => vec![Q2MissionPack::Xatrix],
        "rogue" => vec![Q2MissionPack::Rogue],
        _ => Vec::new(),
    }
}

/// Armory pack for a program name (`registerQ2MissionPackArmory` argument).
fn q2_armory_pack(program: &str) -> Q2MissionPack {
    if program == "xatrix" {
        Q2MissionPack::Xatrix
    } else {
        Q2MissionPack::Rogue
    }
}

/// Mission-pack monster selector for an expansion pack.
fn q2_monster_pack(pack: Q2MissionPack) -> Q2MonsterMissionPack {
    match pack {
        Q2MissionPack::Xatrix => Q2MonsterMissionPack::Xatrix,
        Q2MissionPack::Rogue => Q2MonsterMissionPack::Rogue,
    }
}

/// Filter expansion fallbacks to original sources (`originalSourceFallbacks`).
fn filter_source_fallbacks(has_rerelease: bool, expansions: &[Q2ProductExpansion]) -> Vec<String> {
    if !has_rerelease {
        return Vec::new();
    }
    expansions
        .iter()
        .flat_map(|expansion| expansion.fallbacks.iter().cloned())
        .filter(|classname| classname != "monster_gladb" && classname != "monster_boss5")
        .collect()
}

/// Adapter from session flag hooks to the arena flag authority.
#[derive(Debug, Clone, Copy)]
struct ProductDeathmatchFlags {
    /// Session hooks.
    hooks: Q2DeathmatchFlagsHooks,
}

impl DeathmatchFlags for ProductDeathmatchFlags {
    fn read(&self) -> i32 {
        (self.hooks.read)()
    }

    fn write(&mut self, flags: i32) {
        (self.hooks.write)(flags)
    }
}

/// Drop a monster item (`dropItem`).
fn product_drop_item(actor: OwnedActor, game: &mut Q2GameServices, classname: &str) {
    player_items(game).drop_monster(&actor, game, classname);
}

/// Base platform phase as a monster platform phase.
fn product_platform_phase(phase: Q2PlatformPhase) -> PlatformPhase {
    match phase {
        Q2PlatformPhase::Top => PlatformPhase::Top,
        Q2PlatformPhase::Bottom => PlatformPhase::Bottom,
        Q2PlatformPhase::Up => PlatformPhase::Up,
        Q2PlatformPhase::Down => PlatformPhase::Down,
    }
}

/// Rogue platform phase as a monster platform phase.
fn product_rogue_phase(phase: Q2Plat2Phase) -> PlatformPhase {
    match phase {
        Q2Plat2Phase::Top => PlatformPhase::Top,
        Q2Plat2Phase::Bottom => PlatformPhase::Bottom,
        Q2Plat2Phase::Up => PlatformPhase::Up,
        Q2Plat2Phase::Down => PlatformPhase::Down,
    }
}

/// Read platform state (`platformState`).
fn product_platform_state(game: &Q2GameServices, actor: &ActorId) -> Option<PlatformPhase> {
    game.entity(actor)?;
    let runtime = product(game);
    for expansion in &runtime.expansions {
        if expansion.pack != Q2MissionPack::Rogue {
            continue;
        }
        let movers = Q2RogueMovers {
            hooks: expansion.entities.hooks,
        };
        if let Some((_, _, phase)) = movers.platform_state(actor, game) {
            return Some(product_rogue_phase(phase));
        }
    }
    runtime
        .base_entities
        .platform_state(actor, game)
        .map(|state| product_platform_phase(state.phase))
}

/// Teleport a player (`teleportPlayer`).
fn product_teleport_player(actor: ActorId, game: &mut Q2GameServices, origin: Vec3, angles: Vec3) {
    if game.entity(&actor).is_some() {
        product_players(game).teleport_player(actor, game, origin, angles);
    }
}

/// Admit a turret driver (`turretDriver`).
fn product_turret_driver<'a>(actor: ActorId, game: &'a mut Q2GameServices) -> MonsterContext<'a> {
    spawn_infantry_driver(game, actor.clone());
    MonsterContext::new(actor, game)
}

/// Whether an actor is an admitted monster (`monsterContext`).
fn product_monster_admitted(game: &Q2GameServices, actor: &ActorId) -> bool {
    game.monsters.states.contains_key(actor)
}

/// Resume a monster (`resumeMonster`).
fn product_resume_monster(actor: ActorId, game: &mut Q2GameServices) {
    resume_monster(game, actor);
}

/// Resolve a monster context (`monster`).
fn product_monster_context<'a>(actor: ActorId, game: &'a mut Q2GameServices) -> Option<MonsterContext<'a>> {
    if game.monsters.states.contains_key(&actor) {
        Some(MonsterContext::new(actor, game))
    } else {
        None
    }
}

/// Anger a monster at a target (`targetAnger`).
fn product_target_anger(entity: ActorId, target: ActorId, game: &mut Q2GameServices) {
    mission_pack_target_anger(game, &entity, &target);
}

/// Emit a mission-pack entity event (`emit`).
fn product_entity_emit(game: &Q2GameServices, event: Q2MissionPackEntityEvent) {
    composition_emit(game, Q2CompositionEvent::MissionpackEntity(event));
}

/// Emit a mission-pack player effect (`playerEffect`).
fn product_player_effect(game: &Q2GameServices, event: Q2MissionPackPlayerEffect) {
    composition_emit(game, Q2CompositionEvent::MissionpackPlayer(event));
}

/// Read the gravity scale (`gravity`).
fn product_gravity(game: &Q2GameServices) -> f64 {
    (composition_services(game).gravity)()
}

/// Whether the match is in intermission (`intermission`).
fn product_intermission(game: &Q2GameServices) -> bool {
    matches!(game.players.intermission, Q2Intermission::Intermission { .. })
}

/// React to a picked-up weapon (`weaponPicked`).
fn product_weapon_picked(actor: ActorId, game: &mut Q2GameServices, item: ItemId, first: bool) {
    let rerelease = product(game).rerelease.is_some();
    if rerelease && game.entity(&actor).is_some() {
        let players = Q2RereleasePlayers {
            players: product_players(game),
        };
        players.weapon_picked(actor, game, item, first);
        return;
    }
    let picked = product(game).item_hooks.weapon_picked;
    picked(actor, game, item, first);
}

/// Grant pack ammo on pickup (`ammoPack`).
fn product_ammo_pack(player: OwnedActor, game: &mut Q2GameServices, full: bool) {
    let hooks = product(game).item_hooks;
    let armory = product(game).armory;
    let packs = product(game).packs.clone();
    if let Some(ammo_pack) = hooks.ammo_pack {
        ammo_pack(player.clone(), game, full);
    }
    if let Some(armory) = armory {
        for pack in &packs {
            armory.items.ammo_pack(&player, game, full, *pack);
        }
    }
}

/// Replace a random respawn (`randomRespawn`).
fn product_random_respawn(entity: ActorId, game: &mut Q2GameServices) -> Option<ActorId> {
    let random_items = composition_services(game).random_items?;
    let settings = random_items();
    let armory = product(game).armory?;
    armory.items.random_respawn(entity, game, &settings)
}

/// Read the quad-fire drop expiry (`quadFireDropUntil`).
fn product_quad_fire_drop_until(actor: ActorId, game: &Q2GameServices) -> f64 {
    if game.options.edition != Q2Edition::Rerelease {
        return 0.0;
    }
    let services = composition_services(game);
    if !services.drop_quad_fire.map(|drops| drops()).unwrap_or(true) {
        return 0.0;
    }
    product(game)
        .armory
        .map(|armory| armory.items.powerups(&actor, game).quad_fire_until)
        .unwrap_or(0.0)
}

/// Fold armory and match adjustments into weapon input (`weaponInput`).
fn adjust_q2_weapon_input(game: &mut Q2GameServices, actor: ActorId, input: Q2WeaponInput) -> Q2WeaponInput {
    let armory = product(game).armory;
    let adjusted = match armory {
        Some(armory) => armory.items.input(&actor, game, input),
        None => input,
    };
    if product(game).selection == Q2MatchSelection::Ctf && game.entity(&actor).is_some() {
        let ctf = Q2Ctf { hooks: ctf_hooks(game) };
        return ctf.weapon_input(actor, game, &adjusted);
    }
    adjusted
}

/// Read composed weapon input (`weaponInput` hook).
fn product_weapon_input(actor: ActorId, game: &mut Q2GameServices) -> Q2WeaponInput {
    let read = product(game).player_hooks.weapon_input;
    let input = read(actor.clone(), game);
    adjust_q2_weapon_input(game, actor, input)
}

/// Score a death (`score` hook).
fn product_score(
    victim: ActorId,
    attacker: Option<ActorId>,
    game: &mut Q2GameServices,
    change: i32,
    means: i32,
    recipient: ActorId,
) {
    let selection = product(game).selection.clone();
    if selection == Q2MatchSelection::Standard {
        if let Some(score) = product(game).player_hooks.score {
            score(victim, attacker, game, change, means, recipient);
            return;
        }
    }
    Q2ProductMatch { selection }.score(victim, game, change, means, recipient, attacker);
}

/// React to a spawn (`playerSpawned` hook).
fn product_player_spawned(entity: ActorId, game: &mut Q2GameServices) {
    let selection = product(game).selection.clone();
    Q2ProductMatch { selection }.player_spawned(entity.clone(), game);
    if let Some(spawned) = product(game).player_hooks.player_spawned {
        spawned(entity, game);
    }
}

/// Select a spawn (`selectSpawn` hook).
fn product_select_spawn(entity: ActorId, game: &mut Q2GameServices) -> Option<Q2SpawnPlacement> {
    let runtime = product(game);
    let selection = runtime.selection.clone();
    let rogue = runtime.rogue_spawns.is_some();
    let configured = runtime.player_hooks.select_spawn;
    let product_match = Q2ProductMatch { selection };
    if let Some((origin, angles)) = product_match.select_spawn(entity.clone(), game) {
        return Some(Q2SpawnPlacement { origin, angles });
    }
    if rogue {
        if let Some((origin, angles)) = Q2RoguePlayerSpawns.select_spawn(game) {
            return Some(Q2SpawnPlacement { origin, angles });
        }
    }
    configured.and_then(|select| select(entity, game))
}

/// Handle a client command (`command` hook).
fn product_command(entity: ActorId, game: &mut Q2GameServices, name: &str, args: &[String]) -> bool {
    let selection = product(game).selection.clone();
    let product_match = Q2ProductMatch { selection };
    if product_match.command(entity.clone(), game, name, args) {
        return true;
    }
    product(game)
        .player_hooks
        .command
        .map(|command| command(entity, game, name, args))
        .unwrap_or(false)
}

/// Drop match inventory before death inventory clears (`beforeDeathInventory` hook).
fn product_before_death_inventory(entity: ActorId, game: &mut Q2GameServices, attack: Option<AttackProvenance>) {
    let selection = product(game).selection.clone();
    Q2ProductMatch { selection }.drop_inventory(entity.clone(), game);
    if let Some(before) = product(game).player_hooks.before_death_inventory {
        before(entity, game, attack);
    }
}

/// React to a death (`death` hook).
fn product_death(entity: ActorId, game: &mut Q2GameServices, attack: Option<AttackProvenance>) {
    let selection = product(game).selection.clone();
    Q2ProductMatch { selection }.death(entity.clone(), game);
    if let Some(armory) = product(game).armory {
        armory.spheres.owner_died(&entity, game, attack.as_ref());
        armory.items.reset(&entity, game);
    }
    if let Some(death) = product(game).player_hooks.death {
        death(entity, game, attack);
    }
}

/// React to a disconnect (`disconnect` hook).
fn product_disconnect(entity: ActorId, game: &mut Q2GameServices) {
    let selection = product(game).selection.clone();
    Q2ProductMatch { selection }.disconnect(entity.clone(), game);
    if let Some(armory) = product(game).armory {
        armory.spheres.disconnect(&entity, game);
        armory.items.reset(&entity, game);
    }
    if let Some(disconnect) = product(game).player_hooks.disconnect {
        disconnect(entity, game);
    }
}

/// Whether a monster holds a health bar (rerelease default).
fn product_monster_holds_health_bar(game: &Q2GameServices, actor: ActorId) -> bool {
    game.entity(&actor)
        .is_some_and(|entity| entity.classname == "monster_jorg")
}

/// Clear expansion powerup timers (`clearExpansionPowerups`).
fn product_clear_expansion_powerups(actor: ActorId, game: &mut Q2GameServices) {
    if let Some(armory) = product(game).armory {
        armory.items.reset(&actor, game);
    }
    if let Some(clear) = product(game).clear_expansion_powerups {
        clear(actor, game);
    }
}

/// Retarget a health bar (`transferHealthbarTarget`).
fn product_transfer_healthbar(old_actor: ActorId, new_actor: ActorId, game: &mut Q2GameServices) {
    let rerelease = product(game)
        .rerelease
        .expect("rerelease healthbar requires rerelease composition");
    rerelease
        .entities
        .entities
        .transfer_healthbar_target(old_actor, new_actor, game);
}

/// CTF instanced-coop policy (`instancedCoop`).
fn product_ctf_instanced_coop(game: &mut Q2GameServices) -> bool {
    if product(game).rerelease.is_some() {
        rerelease_instanced_coop(game)
    } else {
        false
    }
}

/// CTF pickup policy (`canPickup`).
fn product_ctf_can_pickup(entity: ActorId, game: &mut Q2GameServices, player: ActorId) -> bool {
    let ctf = Q2Ctf { hooks: ctf_hooks(game) };
    if !ctf.pickups_allowed(game) {
        return false;
    }
    if product(game).rerelease.is_some() {
        rerelease_can_pickup(entity, game, player)
    } else {
        true
    }
}

/// CTF pre-pickup policy (`beforePickup`).
fn product_ctf_before_pickup(entity: ActorId, game: &mut Q2GameServices, player: ActorId) -> bool {
    let ctf = Q2Ctf { hooks: ctf_hooks(game) };
    if !ctf.pickups_allowed(game) {
        return false;
    }
    if product(game).rerelease.is_some() {
        rerelease_before_pickup(entity, game, player)
    } else {
        true
    }
}

/// CTF pre-target policy (`beforeTargets`).
fn product_ctf_before_targets(entity: ActorId, game: &mut Q2GameServices, player: ActorId, taken: bool) {
    if product(game).rerelease.is_some() {
        rerelease_before_targets(entity, game, player, taken);
    }
}

/// CTF post-pickup policy (`afterPickup`).
fn product_ctf_after_pickup(entity: ActorId, game: &mut Q2GameServices, player: ActorId, taken: bool) {
    if product(game).rerelease.is_some() {
        rerelease_after_pickup(entity, game, player, taken);
    }
}

/// CTF keep-after-pickup policy (`keepAfterPickup`).
fn product_ctf_keep_after_pickup(entity: ActorId, game: &mut Q2GameServices, player: ActorId) -> bool {
    if product(game).rerelease.is_some() {
        rerelease_keep_after_pickup(entity, game, player)
    } else {
        false
    }
}

impl Q2ProductRuntime {
    /// Write the deathmatch flags (`setDeathmatchFlags`).
    pub fn set_deathmatch_flags(&self, game: &mut Q2GameServices, flags: i32) {
        let _ = self;
        match composition_services(game).deathmatch_flags {
            Some(hooks) => (hooks.write)(flags),
            None => {
                game.composition.active_rules = flags;
                game.options.deathmatch_flags = flags;
            }
        }
    }

    /// Original-source fallback classnames (`originalSourceFallbacks`).
    pub fn original_source_fallbacks(&self) -> Vec<String> {
        filter_source_fallbacks(self.rerelease.is_some(), &self.expansions)
    }

    /// Register module and armory callbacks (`registerCallbacks`).
    pub fn register_callbacks(&self, game: &mut Q2GameServices) {
        let modules: Vec<Q2CallbackDefinitions> = game.modules.iter().map(|module| module.callbacks.clone()).collect();
        for callbacks in &modules {
            game.source_callbacks.register(callbacks);
        }
        game.source_callbacks.register(&q2_ballistics_callbacks());
        if self.armory.is_some() {
            game.source_callbacks.register(&mission_projectile_callbacks());
            game.source_callbacks.register(&sphere_callbacks());
            game.source_callbacks.register(&doppleganger_callbacks());
        }
    }

    /// Admit a player (`admit`).
    pub fn admit(
        &self,
        actor: OwnedActor,
        game: &mut Q2GameServices,
        admission: Q2PlayerAdmission,
        landmark: Option<Q2LandmarkCarry>,
    ) -> ActorId {
        let entity = game.attach_player(actor);
        self.players.attach(entity.clone(), game, admission);
        self.product_match.admitted(entity.clone(), game);
        if self.selection != Q2MatchSelection::Ctf {
            self.players
                .put_in_server(entity.clone(), game, false, landmark.as_ref());
        }
        entity
    }

    /// Fold armory and match adjustments into weapon input (`weaponInput`).
    pub fn weapon_input(&self, actor: ActorId, game: &mut Q2GameServices, input: Q2WeaponInput) -> Q2WeaponInput {
        let _ = self;
        adjust_q2_weapon_input(game, actor, input)
    }

    /// Read powerup expiries (`powerups`).
    pub fn powerups(&self, actor: ActorId, game: &Q2GameServices) -> Q2ForeignPowerups {
        if game
            .players
            .states
            .get(&actor)
            .is_some_and(|state| !state.use_q2_inventory)
        {
            return (composition_services(game).foreign_powerups)(actor);
        }
        let base = self.items.player_powerups(game, &actor);
        Q2ForeignPowerups {
            quad_until: base.quad_until,
            double_until: self
                .armory
                .map(|armory| armory.items.powerups(&actor, game).double_until)
                .unwrap_or(0.0),
            invulnerability_until: base.invulnerability_until,
        }
    }

    /// Record dealt damage (`recordDamage`).
    pub fn record_damage(&self, actor: ActorId, game: &mut Q2GameServices, decision: &DamageDecision) {
        self.players.record_damage(actor, game, decision);
    }

    /// React before a damage reaction (`beforeReaction`).
    pub fn before_reaction(&self, actor: ActorId, game: &mut Q2GameServices, attack: &AttackProvenance) {
        if let Some(armory) = self.armory {
            armory.spheres.owner_damaged(&actor, attack, game);
        }
    }

    /// React to a movement impact (`movementImpact`).
    pub fn movement_impact(&self, actor: ActorId, game: &mut Q2GameServices, impact_delta: f64, on_ladder: bool) {
        if let Some(rerelease) = self.rerelease {
            rerelease.players.movement_impact(game, actor, impact_delta, on_ladder);
        }
    }

    /// Record a foreign weapon firing (`recordForeignWeaponFire`).
    pub fn record_foreign_weapon_fire(&self, actor: ActorId, game: &mut Q2GameServices) {
        if let Some(rerelease) = self.rerelease {
            let now = game.host.now();
            rerelease.players.record_weapon_fire(game, actor, now);
        }
    }

    /// Run after player frames (`afterPlayerFrames`).
    pub fn after_player_frames(&self, game: &mut Q2GameServices) {
        let actors: Vec<ActorId> = game.players.states.keys().cloned().collect();
        for actor in actors {
            if game.entity(&actor).is_some() {
                self.product_match.effects(actor, game);
            }
        }
        if let Some(rerelease) = self.rerelease {
            rerelease.entities.after_player_frames(game);
        }
    }

    /// Run after spawning (`afterSpawn`).
    pub fn after_spawn(&self, game: &mut Q2GameServices) {
        if self.packs.contains(&Q2MissionPack::Rogue) {
            finalize_hint_paths(game);
        }
        self.product_match.after_spawn(game);
    }

    /// Check the rules (`checkRules`).
    pub fn check_rules(&self, game: &mut Q2GameServices) {
        if game.players.intermission == Q2Intermission::Playing && self.product_match.check_rules(game) {
            return;
        }
        self.players.check_rules(game);
    }
}

/// Push a spawn module, registering its callbacks first.
fn push_product_module(game: &mut Q2GameServices, module: SpawnModule) {
    game.source_callbacks.register(&module.callbacks);
    game.modules.push(module);
}

/// Assemble the Q2 product runtime (`createQ2ProductRuntime`).
pub fn create_q2_product_runtime(game: &mut Q2GameServices, options: Q2CompositionOptions) -> Q2ProductRuntime {
    let (edition, program, common, rerelease_hooks, rerelease_options, campaign) = match options {
        Q2CompositionOptions::Classic { common, program } => {
            (Q2Edition::Classic, program.as_str(), common, None, None, None)
        }
        Q2CompositionOptions::Rerelease {
            common,
            program,
            rerelease_hooks,
            rerelease_options,
            campaign,
        } => (
            Q2Edition::Rerelease,
            program.as_str(),
            common,
            Some(rerelease_hooks),
            rerelease_options,
            campaign,
        ),
    };
    let Q2CompositionCommon {
        options: _,
        item_hooks: config_item_hooks,
        player_hooks: config_player_hooks,
        player_rules,
        match_selection,
        entity_hooks,
        services: composition_services_value,
    } = *common;
    let clear_expansion_powerups = rerelease_hooks
        .as_ref()
        .and_then(|hooks| hooks.clear_expansion_powerups);
    let packs = q2_product_packs(edition, program);
    let selection = match_selection.unwrap_or(Q2MatchSelection::Standard);
    if selection != Q2MatchSelection::Standard && game.options.mode != Q2Mode::Deathmatch {
        panic!("Q2 Tag and DeathBall require deathmatch admission");
    }
    game.options.edition = edition;
    game.monsters.hooks.drop_item = Some(product_drop_item);
    game.monsters.hooks.platform_state = Some(product_platform_state);
    register_q2_classic_base_monsters(game);
    let movers = create_q2_mover_module(Q2MoverHooks {
        path_corner: touch_path_corner,
        combat_point: touch_combat_point,
    });
    let base_entities = create_q2_base_entity_module(Q2BaseEntityHooks {
        movers,
        fire_blaster: game.monsters.weapons.fire_blaster,
        fire_rocket: game.monsters.weapons.fire_rocket,
        teleport_player: product_teleport_player,
        player_push: entity_hooks.player_push,
        set_actor_gravity: entity_hooks.set_actor_gravity,
        local_time: entity_hooks.local_time,
        turret_driver: product_turret_driver,
        monster_context: product_monster_admitted,
        resume_monster: product_resume_monster,
    });
    let mut services = composition_services_value;
    let items = create_q2_item_module(Q2ItemHooks {
        weapon_picked: product_weapon_picked,
        ammo_pack: Some(product_ammo_pack),
        random_respawn: if services.random_items.is_some() {
            Some(product_random_respawn)
        } else {
            None
        },
        ..config_item_hooks
    });
    let classic_base = edition == Q2Edition::Classic && program == "baseq2";
    let armory = if classic_base {
        None
    } else {
        Some(register_q2_mission_pack_armory(
            game,
            q2_armory_pack(program),
            Q2MissionPackArmoryHooks {
                projectile_hooks: Q2MissionPackProjectileHooks {
                    strong_mines: services.strong_mines,
                    gravity: Some(product_gravity),
                    monster: product_monster_context,
                    player_effect: product_player_effect,
                },
                sphere_hooks: Q2SphereHooks {
                    hunter_camera: services.hunter_camera,
                    intermission: product_intermission,
                    player_effect: product_player_effect,
                },
                items,
            },
            edition,
        ))
    };
    let mut expansions = Vec::new();
    let mut expansion_modules = Vec::new();
    for pack in &packs {
        let entities = Q2MissionPackEntities {
            pack: *pack,
            hooks: Q2MissionPackEntityHooks {
                movers,
                teleport_player: product_teleport_player,
                target_anger: product_target_anger,
                emit: product_entity_emit,
            },
        };
        expansion_modules.push(entities.register(game));
        let fallbacks = register_q2_mission_pack_monsters(game, q2_monster_pack(*pack), edition);
        expansions.push(Q2ProductExpansion {
            pack: *pack,
            entities,
            fallbacks,
        });
    }
    let product_match = Q2ProductMatch {
        selection: selection.clone(),
    };
    let shared_grapple = services.shared_grapple.take();
    let match_module = product_match.register(game, items, shared_grapple);
    let rogue_spawns = if edition == Q2Edition::Classic && program == "rogue" {
        Some(Q2RoguePlayerSpawns)
    } else {
        None
    };
    let movement_stop_speed = if matches!(selection, Q2MatchSelection::Deathball { .. }) {
        Some(0.0)
    } else {
        None
    };
    let composed_player_hooks = Q2PlayerHooks {
        quad_fire_drop_until: Some(product_quad_fire_drop_until),
        weapon_input: product_weapon_input,
        score: Some(product_score),
        player_spawned: Some(product_player_spawned),
        select_spawn: Some(product_select_spawn),
        command: Some(product_command),
        before_death_inventory: Some(product_before_death_inventory),
        death: Some(product_death),
        disconnect: Some(product_disconnect),
        ..config_player_hooks
    };
    game.players.hooks = Some(composed_player_hooks);
    game.players.items = Some(items);
    if let Some(rules) = player_rules {
        game.players.rules = rules;
    }
    let rerelease = if edition == Q2Edition::Rerelease {
        if armory.is_none() {
            panic!("Rerelease source composition requires the combined arsenal");
        }
        if !packs.contains(&Q2MissionPack::Rogue) {
            panic!("Rerelease source composition requires Rogue monster state");
        }
        let hooks = rerelease_hooks.expect("rerelease composition requires rerelease hooks");
        if let Some(options) = rerelease_options {
            game.rerelease.options = options;
        }
        if let Some(campaign) = campaign {
            game.rerelease.campaign = *campaign;
        }
        let players = Q2RereleasePlayers {
            players: create_q2_players(items, composed_player_hooks),
        };
        let entities = create_q2_rerelease_module(
            game,
            players,
            Q2RereleaseHooks {
                monster_holds_health_bar: hooks
                    .monster_holds_health_bar
                    .or(Some(product_monster_holds_health_bar)),
                clear_expansion_powerups: Some(product_clear_expansion_powerups),
                ..hooks
            },
        );
        // The merged composition options have no N64 program, so the
        // N64 selector is always false here.
        register_q2_rerelease_monsters(game, false, Some(product_transfer_healthbar));
        Some(Q2ProductRerelease { players, entities })
    } else {
        None
    };
    let players = create_q2_players(items, composed_player_hooks);
    if selection == Q2MatchSelection::Ctf {
        items.set_pickup_policy(
            game,
            Q2PickupPolicy {
                instanced_coop: Some(product_ctf_instanced_coop),
                can_pickup: product_ctf_can_pickup,
                before_pickup: product_ctf_before_pickup,
                before_targets: Some(product_ctf_before_targets),
                after_pickup: product_ctf_after_pickup,
                keep_after_pickup: product_ctf_keep_after_pickup,
            },
        );
    }
    let mut modules = Vec::new();
    if rerelease.is_some() {
        modules.push(SpawnModule {
            spawn: rerelease_module_spawn,
            item_name: |_| None,
            callbacks: rerelease_module_callbacks(),
        });
    }
    modules.push(SpawnModule {
        spawn: spawn_player_spawn,
        item_name: |_| None,
        callbacks: spawn_callbacks(),
    });
    modules.push(match_module);
    if rogue_spawns.is_some() {
        modules.push(rogue_player_spawn_module());
    }
    modules.extend(expansion_modules);
    modules.push(base_entities.register(game));
    modules.push(create_q2_target_module());
    modules.push(movers.register(game));
    modules.push(create_q2_scenery_module());
    if armory.is_some() {
        modules.push(SpawnModule {
            spawn: mission_item_spawn,
            item_name: |_| None,
            callbacks: Q2CallbackDefinitions::default(),
        });
    }
    modules.push(items.register(game));
    modules.push(SpawnModule {
        spawn: monster_spawn,
        item_name: |_| None,
        callbacks: Q2CallbackDefinitions::default(),
    });
    for module in modules {
        push_product_module(game, module);
    }
    let configured = match services.deathmatch_flags {
        Some(hooks) => (hooks.read)(),
        None => game.options.deathmatch_flags,
    };
    let deathmatch_flags = if matches!(selection, Q2MatchSelection::Deathball { .. }) {
        q2_deathball_rules(configured).deathmatch_flags
    } else {
        configured
    };
    game.options.deathmatch_flags = deathmatch_flags;
    game.composition.active_rules = deathmatch_flags;
    if let Some(hooks) = services.deathmatch_flags {
        game.deathmatch_flags = Some(Box::new(ProductDeathmatchFlags { hooks }));
    }
    game.composition.services = Some(services);
    let runtime = Q2ProductRuntime {
        program: program.to_string(),
        selection,
        product_match,
        items,
        movers,
        base_entities,
        players,
        armory,
        expansions,
        rerelease,
        rogue_spawns,
        movement_stop_speed,
        packs,
        item_hooks: config_item_hooks,
        player_hooks: config_player_hooks,
        clear_expansion_powerups,
    };
    game.composition.product = Some(runtime.clone());
    runtime.register_callbacks(game);
    runtime
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stub_path(_corner: ActorId, _game: &mut Q2GameServices, _other: ActorId) {}

    fn stub_teleport(_actor: ActorId, _game: &mut Q2GameServices, _origin: Vec3, _angles: Vec3) {}

    fn stub_anger(_entity: ActorId, _target: ActorId, _game: &mut Q2GameServices) {}

    fn stub_emit(_game: &Q2GameServices, _event: Q2MissionPackEntityEvent) {}

    fn test_entities(pack: Q2MissionPack) -> Q2MissionPackEntities {
        Q2MissionPackEntities {
            pack,
            hooks: Q2MissionPackEntityHooks {
                movers: create_q2_mover_module(Q2MoverHooks {
                    path_corner: stub_path,
                    combat_point: stub_path,
                }),
                teleport_player: stub_teleport,
                target_anger: stub_anger,
                emit: stub_emit,
            },
        }
    }

    fn test_expansion(pack: Q2MissionPack, fallbacks: &[&str]) -> Q2ProductExpansion {
        Q2ProductExpansion {
            pack,
            entities: test_entities(pack),
            fallbacks: fallbacks.iter().map(ToString::to_string).collect(),
        }
    }

    #[test]
    fn classic_base_has_no_packs() {
        assert!(q2_product_packs(Q2Edition::Classic, "baseq2").is_empty());
    }

    #[test]
    fn classic_packs_follow_the_program() {
        assert_eq!(
            q2_product_packs(Q2Edition::Classic, "xatrix"),
            vec![Q2MissionPack::Xatrix]
        );
        assert_eq!(
            q2_product_packs(Q2Edition::Classic, "rogue"),
            vec![Q2MissionPack::Rogue]
        );
    }

    #[test]
    fn rerelease_always_uses_both_packs() {
        for program in ["baseq2", "xatrix", "rogue", "mg2"] {
            assert_eq!(
                q2_product_packs(Q2Edition::Rerelease, program),
                vec![Q2MissionPack::Xatrix, Q2MissionPack::Rogue]
            );
        }
    }

    #[test]
    fn armory_pack_defaults_to_rogue() {
        assert_eq!(q2_armory_pack("xatrix"), Q2MissionPack::Xatrix);
        assert_eq!(q2_armory_pack("rogue"), Q2MissionPack::Rogue);
        assert_eq!(q2_armory_pack("baseq2"), Q2MissionPack::Rogue);
        assert_eq!(q2_armory_pack("mg2"), Q2MissionPack::Rogue);
    }

    #[test]
    fn monster_packs_match_expansions() {
        assert_eq!(
            q2_monster_pack(Q2MissionPack::Xatrix) as u8,
            Q2MonsterMissionPack::Xatrix as u8
        );
        assert_eq!(
            q2_monster_pack(Q2MissionPack::Rogue) as u8,
            Q2MonsterMissionPack::Rogue as u8
        );
    }

    #[test]
    fn platform_phases_map_verbatim() {
        assert_eq!(product_platform_phase(Q2PlatformPhase::Top), PlatformPhase::Top);
        assert_eq!(product_platform_phase(Q2PlatformPhase::Bottom), PlatformPhase::Bottom);
        assert_eq!(product_platform_phase(Q2PlatformPhase::Up), PlatformPhase::Up);
        assert_eq!(product_platform_phase(Q2PlatformPhase::Down), PlatformPhase::Down);
        assert_eq!(product_rogue_phase(Q2Plat2Phase::Top), PlatformPhase::Top);
        assert_eq!(product_rogue_phase(Q2Plat2Phase::Bottom), PlatformPhase::Bottom);
        assert_eq!(product_rogue_phase(Q2Plat2Phase::Up), PlatformPhase::Up);
        assert_eq!(product_rogue_phase(Q2Plat2Phase::Down), PlatformPhase::Down);
    }

    #[test]
    fn fallbacks_require_rerelease() {
        let expansions = vec![test_expansion(Q2MissionPack::Xatrix, &["monster_soldier"])];
        assert!(filter_source_fallbacks(false, &expansions).is_empty());
    }

    #[test]
    fn fallbacks_merge_expansions_and_skip_bosses() {
        let expansions = vec![
            test_expansion(Q2MissionPack::Xatrix, &["monster_soldier", "monster_gladb"]),
            test_expansion(Q2MissionPack::Rogue, &["monster_boss5", "monster_carrier"]),
        ];
        assert_eq!(
            filter_source_fallbacks(true, &expansions),
            vec!["monster_soldier".to_string(), "monster_carrier".to_string()]
        );
    }
}
