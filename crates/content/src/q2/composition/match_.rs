//! Q2 product match (`src/content/composition/q2/match.ts`).
//!
//! The donor holds the selected mode object and closes over the players
//! module. The port dispatches on the [`Q2MatchSelection`] plus the
//! mode arena slots, and implements every mode hook as a plain `fn`
//! reading the arena.

use qa_core::identity::ActorId;
use qa_core::math::Vec3;

use super::types::{Q2CompositionEvent, Q2MatchSelection, set_q2_info_value};
use super::{composition_emit, composition_services};
use crate::contract::InventoryEntry;
use crate::q2::base::player::index::{create_q2_players, player_hooks};
use crate::q2::base::player::spawns::{q2_entities_named, q2_players_range, q2_spawn_origin, select_q2_spawn};
use crate::q2::base::player::types::{Q2PlayerMovementChange, Q2PlayerSpawnChange, Q2PlayerState};
use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::host::{Q2Edition, Q2GameServices, SpawnModule};
use crate::q2::foundation::items::Q2ItemModule;
use crate::q2::missionpacks::modes::deathball::{Q2DeathBall, Q2DeathBallHooks, Q2DeathBallSettings, deathball_callbacks};
use crate::q2::missionpacks::modes::tag::{Q2Tag, Q2TagHooks, tag_callbacks, tag_hooks};
use crate::q2::multiplayer::ctf::index::{Q2Ctf, ctf_callbacks};
use crate::q2::multiplayer::ctf::types::{Q2CtfEvent, Q2CtfHooks, create_q2_ctf_rules, ctf_team_name, item_id};
use crate::q2::multiplayer::lmctf::runtime::Q2Lmctf;
use crate::q2::multiplayer::lmctf::types::{LmctfEvent, LmctfHooks, create_lmctf_rules};
use crate::q2::support::contracts::{CombatState, CombatTraitChanges, SharedGrappleControl};

/// Rebuild the players handle from the arena.
fn match_players(game: &Q2GameServices) -> crate::q2::base::player::index::Q2Players {
    create_q2_players(game.players.items.expect("Q2 match requires registered players"), player_hooks(game))
}

/// Admitted player state (`players().states.get`).
fn match_player(actor: ActorId, game: &mut Q2GameServices) -> Option<&mut Q2PlayerState> {
    game.players.states.get_mut(&actor)
}

/// Set a match player skin (`setSkin`).
fn match_set_skin(actor: ActorId, game: &mut Q2GameServices, skin: String) {
    let userinfo = match (game.players.states.get(&actor), game.entity(&actor)) {
        (Some(player), Some(_)) => player.userinfo.clone(),
        _ => panic!("Match skin requires an admitted player"),
    };
    let userinfo = set_q2_info_value(game, &userinfo, "skin", &skin, 512);
    match_players(game).userinfo_changed(actor, game, &userinfo);
}

/// Respawn a match player (`spawnPlayer`).
fn match_spawn_player(actor: ActorId, game: &mut Q2GameServices) {
    match_players(game).put_in_server(actor, game, true, None);
}

/// Observe as a match player (`observer`).
fn match_observer(actor: ActorId, game: &mut Q2GameServices) {
    (player_hooks(game).set_movement)(actor, Q2PlayerMovementChange::Noclip { enabled: true });
}

/// Teleport a match player (`teleport`).
fn match_teleport(actor: ActorId, game: &mut Q2GameServices, origin: Vec3, angles: Vec3, velocity: Vec3) {
    match_players(game).teleport_player(actor.clone(), game, origin, angles);
    let mut body = game.body_of(actor.clone());
    body.velocity = velocity;
    game.write_body(actor.clone(), &body, true);
    let command_angles = (player_hooks(game).movement)(actor.clone()).command_angles;
    (player_hooks(game).set_movement)(
        actor,
        Q2PlayerMovementChange::Teleport(Q2PlayerSpawnChange {
            origin,
            velocity,
            angles,
            command_angles,
            hold_milliseconds: 160,
            spectator: false,
        }),
    );
}

/// Chase as a match player (`chase`).
fn match_chase(actor: ActorId, game: &mut Q2GameServices) {
    if game.players.states.get(&actor).is_none() || game.entity(&actor).is_none() {
        panic!("Match chase requires an admitted player");
    }
    match_players(game).chase(actor, game, 1, true);
}

/// Toggle grapple prediction (`setGrapplePrediction`).
fn match_set_grapple_prediction(actor: ActorId, game: &mut Q2GameServices, suppress: bool) {
    composition_emit(game, Q2CompositionEvent::GrapplePrediction { actor, suppress });
}

/// Read the gravity scale (`gravity`).
fn match_gravity(game: &Q2GameServices) -> f64 {
    (composition_services(game).gravity)()
}

/// Emit a CTF event (`emit`).
fn match_ctf_emit(game: &mut Q2GameServices, event: Q2CtfEvent) {
    composition_emit(game, Q2CompositionEvent::Ctf(event));
}

/// Emit an LMCTF event (`emit`).
fn match_lmctf_emit(game: &mut Q2GameServices, event: LmctfEvent) {
    composition_emit(game, Q2CompositionEvent::Lmctf(event));
}

/// End the match level (`endLevel`).
fn match_end_level(game: &mut Q2GameServices, map: Option<String>) {
    match map {
        None => match_players(game).end_deathmatch_level(game),
        Some(map) => match_players(game).begin_intermission(game, map, None),
    }
}

/// Kick a match player (`kick`).
fn match_kick(actor: ActorId, game: &mut Q2GameServices) {
    composition_emit(game, Q2CompositionEvent::Kick { actor });
}

/// Write the deathmatch flags (`setDeathmatchFlags`).
fn match_set_deathmatch_flags(game: &mut Q2GameServices, flags: i32) {
    match composition_services(game).deathmatch_flags {
        Some(hooks) => (hooks.write)(flags),
        None => game.composition.active_rules = flags,
    }
}

/// Whether chat is allowed (`chatAllowed`).
fn match_chat_allowed(actor: ActorId, game: &mut Q2GameServices) -> bool {
    match_players(game).chat_allowed(actor, game)
}

/// Add match score (`addScore`).
fn match_add_score(actor: ActorId, game: &mut Q2GameServices, amount: f64) {
    match game.players.states.get_mut(&actor) {
        Some(player) => player.score += amount as i32,
        None => panic!("Q2 match score requires an admitted player"),
    }
}

/// Select a match spawn placement (`selectSpawn`).
fn match_select_spawn_placement(actor: ActorId, game: &mut Q2GameServices) -> (Vec3, Vec3) {
    let state = game.players.states.get(&actor).cloned().unwrap_or_else(|| Q2PlayerState::new(0, game.now()));
    let spawn_point = game.players.rules.spawn_point.clone();
    let spot = select_q2_spawn(game, &state, &spawn_point);
    let origin = q2_spawn_origin(game, spot.clone());
    let angles = game.body_of(spot).angles;
    (origin, angles)
}

/// Find the farthest deathmatch spawn (`farthestSpawn`).
fn match_farthest_spawn(game: &mut Q2GameServices) -> Option<ActorId> {
    let mut best: Option<ActorId> = None;
    let mut best_range = 0.0;
    for spot in q2_entities_named(game, "info_player_deathmatch") {
        let range = q2_players_range(game, spot.clone());
        if best.is_none() || range > best_range {
            best_range = range;
            best = Some(spot);
        }
    }
    best
}

/// Read the deathball settings (`settings`).
fn match_deathball_settings(game: &Q2GameServices) -> Q2DeathBallSettings {
    game.deathball.settings.clone().expect("Q2 DeathBall settings require a deathball match")
}

/// Read a deathball skin (`skin`).
fn match_deathball_skin(actor: ActorId, game: &Q2GameServices) -> String {
    game.players.states.get(&actor).map(|player| player.skin.clone()).unwrap_or_default()
}

/// Set a deathball skin (`setSkin`).
fn match_deathball_set_skin(actor: ActorId, game: &mut Q2GameServices, skin: String) {
    let userinfo = match (game.players.states.get(&actor), game.entity(&actor)) {
        (Some(player), Some(_)) => player.userinfo.clone(),
        _ => panic!("Q2 DeathBall skin requires an admitted source player"),
    };
    let maximum = if game.options.edition == Q2Edition::Rerelease { 2048 } else { 512 };
    let userinfo = set_q2_info_value(game, &userinfo, "skin", &skin, maximum);
    match_players(game).userinfo_changed(actor, game, &userinfo);
}

/// End the deathmatch level (`endLevel`).
fn match_end_deathmatch_level(game: &mut Q2GameServices) {
    match_players(game).end_deathmatch_level(game);
}

/// Measure the closest-player range to a spot (`spawnDistance`).
fn match_spawn_distance(actor: ActorId, game: &mut Q2GameServices) -> f64 {
    q2_players_range(game, actor)
}

/// Spawn dispatch for a tag match.
fn match_spawn_tag(entity: ActorId, game: &mut Q2GameServices) -> bool {
    match game.require_entity(&entity).classname.as_str() {
        "dm_dball_team1_start" | "dm_dball_team2_start" | "dm_dball_ball_start" | "dm_dball_goal_touch" => false,
        _ => Q2Tag { hooks: tag_hooks(game) }.spawn(entity, game),
    }
}

/// Spawn dispatch for a deathball match.
fn match_spawn_deathball(entity: ActorId, game: &mut Q2GameServices) -> bool {
    if game.require_entity(&entity).classname == "dm_tag_token" {
        return false;
    }
    Q2DeathBall { hooks: crate::q2::missionpacks::modes::deathball::deathball_hooks(game) }.spawn(entity, game)
}

/// Spawn dispatch for a CTF match.
fn match_spawn_ctf(entity: ActorId, game: &mut Q2GameServices) -> bool {
    if game.require_entity(&entity).classname == "dm_tag_token" {
        return false;
    }
    Q2Ctf { hooks: crate::q2::multiplayer::ctf::ctf_hooks(game) }.spawn(entity, game)
}

/// Spawn dispatch for an LMCTF match.
fn match_spawn_lmctf(entity: ActorId, game: &mut Q2GameServices) -> bool {
    if game.require_entity(&entity).classname == "dm_tag_token" {
        return false;
    }
    Q2Lmctf { hooks: crate::q2::multiplayer::lmctf::lmctf_hooks(game) }.spawn(entity, game)
}

/// Spawn dispatch for a standard match.
fn match_spawn_standard(_entity: ActorId, _game: &mut Q2GameServices) -> bool {
    false
}

/// Selected Q2 match (`Q2ProductMatch`).
#[derive(Debug, Clone)]
pub struct Q2ProductMatch {
    /// Match selection.
    pub selection: Q2MatchSelection,
}

impl Q2ProductMatch {
    /// Session CTF handle.
    fn ctf(&self, game: &Q2GameServices) -> Q2Ctf {
        let _ = self;
        Q2Ctf { hooks: crate::q2::multiplayer::ctf::ctf_hooks(game) }
    }

    /// Session LMCTF handle.
    fn lmctf(&self, game: &Q2GameServices) -> Q2Lmctf {
        let _ = self;
        Q2Lmctf { hooks: crate::q2::multiplayer::lmctf::lmctf_hooks(game) }
    }

    /// Session tag handle.
    fn tag(&self, game: &Q2GameServices) -> Q2Tag {
        let _ = self;
        Q2Tag { hooks: tag_hooks(game) }
    }

    /// Session deathball handle.
    fn deathball(&self, game: &Q2GameServices) -> Q2DeathBall {
        let _ = self;
        Q2DeathBall { hooks: crate::q2::missionpacks::modes::deathball::deathball_hooks(game) }
    }

    /// Build the shared CTF hooks.
    fn ctf_hooks(&self, items: Q2ItemModule) -> Q2CtfHooks {
        let _ = self;
        Q2CtfHooks {
            items,
            player: match_player,
            set_skin: match_set_skin,
            spawn_player: match_spawn_player,
            observer: match_observer,
            teleport: match_teleport,
            chase: match_chase,
            set_grapple_prediction: match_set_grapple_prediction,
            gravity: match_gravity,
            emit: match_ctf_emit,
            end_level: match_end_level,
            kick: match_kick,
            set_deathmatch_flags: match_set_deathmatch_flags,
            chat_allowed: match_chat_allowed,
        }
    }

    /// Build the shared LMCTF hooks.
    fn lmctf_hooks(&self, items: Q2ItemModule) -> LmctfHooks {
        let _ = self;
        LmctfHooks {
            items,
            player: match_player,
            set_skin: match_set_skin,
            spawn_player: match_spawn_player,
            observer: match_observer,
            teleport: match_teleport,
            chase: match_chase,
            set_grapple_prediction: match_set_grapple_prediction,
            gravity: match_gravity,
            emit: match_lmctf_emit,
            end_level: match_end_level,
            kick: match_kick,
            set_deathmatch_flags: match_set_deathmatch_flags,
            chat_allowed: match_chat_allowed,
        }
    }

    /// Register the selected match and build its spawn module.
    pub fn register(
        &self,
        game: &mut Q2GameServices,
        items: Q2ItemModule,
        shared_grapple: Option<Box<dyn SharedGrappleControl>>,
    ) -> SpawnModule {
        match &self.selection {
            Q2MatchSelection::Tag => Q2Tag {
                hooks: Q2TagHooks {
                    items,
                    select_spawn: match_select_spawn_placement,
                    farthest_spawn: match_farthest_spawn,
                    add_score: match_add_score,
                },
            }
            .register(game),
            Q2MatchSelection::Deathball { team1_skin, team2_skin, goal_limit } => {
                game.deathball.settings = Some(Q2DeathBallSettings {
                    team1_skin: team1_skin.clone(),
                    team2_skin: team2_skin.clone(),
                    goal_limit: *goal_limit as i32,
                });
                Q2DeathBall {
                    hooks: Q2DeathBallHooks {
                        settings: match_deathball_settings,
                        skin: match_deathball_skin,
                        set_skin: match_deathball_set_skin,
                        add_score: match_add_score,
                        end_level: match_end_deathmatch_level,
                        select_spawn: match_select_spawn_placement,
                        spawn_distance: match_spawn_distance,
                    },
                }
                .register(game)
            }
            Q2MatchSelection::Ctf => {
                Q2Ctf { hooks: self.ctf_hooks(items) }.register(game, create_q2_ctf_rules(), shared_grapple)
            }
            Q2MatchSelection::Lmctf { travel } => {
                let rules = travel.clone().map(|travel| travel.rules).unwrap_or_else(create_lmctf_rules);
                Q2Lmctf { hooks: self.lmctf_hooks(items) }.register(game, rules, travel.clone(), shared_grapple)
            }
            Q2MatchSelection::Standard => SpawnModule {
                spawn: match_spawn_standard,
                item_name: |_| None,
                callbacks: Q2CallbackDefinitions::default(),
            },
        }
    }

    /// Read the match callbacks (`callbacks`).
    pub fn callbacks(&self, game: &Q2GameServices) -> Q2CallbackDefinitions {
        match &self.selection {
            Q2MatchSelection::Tag => tag_callbacks(),
            Q2MatchSelection::Deathball { .. } => deathball_callbacks(),
            Q2MatchSelection::Ctf => ctf_callbacks(),
            Q2MatchSelection::Lmctf { .. } => crate::q2::multiplayer::lmctf::lmctf_callbacks(game),
            Q2MatchSelection::Standard => Q2CallbackDefinitions::default(),
        }
    }

    /// Spawn match entities (`spawn`).
    pub fn spawn(&self, entity: ActorId, game: &mut Q2GameServices) -> bool {
        match &self.selection {
            Q2MatchSelection::Tag => match_spawn_tag(entity, game),
            Q2MatchSelection::Deathball { .. } => match_spawn_deathball(entity, game),
            Q2MatchSelection::Ctf => match_spawn_ctf(entity, game),
            Q2MatchSelection::Lmctf { .. } => match_spawn_lmctf(entity, game),
            Q2MatchSelection::Standard => match_spawn_standard(entity, game),
        }
    }

    /// Resolve match spawns (`afterSpawn`).
    pub fn after_spawn(&self, game: &mut Q2GameServices) {
        if matches!(self.selection, Q2MatchSelection::Ctf) {
            self.ctf(game).after_spawn(game);
        }
    }

    /// Admit a match player (`admitted`).
    pub fn admitted(&self, entity: ActorId, game: &mut Q2GameServices) {
        match &self.selection {
            Q2MatchSelection::Ctf => self.ctf(game).admitted(entity, game),
            Q2MatchSelection::Lmctf { .. } => self.lmctf(game).admitted(entity, game),
            _ => {}
        }
    }

    /// Select a match spawn (`selectSpawn`).
    pub fn select_spawn(&self, entity: ActorId, game: &mut Q2GameServices) -> Option<(Vec3, Vec3)> {
        match &self.selection {
            Q2MatchSelection::Ctf => self.ctf(game).select_spawn(entity, game),
            Q2MatchSelection::Lmctf { .. } => Some(self.lmctf(game).select_spawn(entity, game)),
            Q2MatchSelection::Deathball { .. } => Some(self.deathball(game).select_spawn(&entity, game)),
            Q2MatchSelection::Tag | Q2MatchSelection::Standard => None,
        }
    }

    /// Score a match kill (`score`).
    pub fn score(
        &self,
        victim: ActorId,
        game: &mut Q2GameServices,
        change: i32,
        means: i32,
        recipient: ActorId,
        attacker: Option<ActorId>,
    ) {
        let attacker = attacker.unwrap_or_else(|| recipient.clone());
        if matches!(self.selection, Q2MatchSelection::Tag) {
            self.tag(game).score(&attacker, &victim, game, f64::from(change), means);
            return;
        }
        match game.players.states.get_mut(&recipient) {
            Some(player) => player.score += change,
            None => panic!("Q2 match score has no admitted source recipient"),
        }
    }

    /// Handle a match death (`death`).
    pub fn death(&self, entity: ActorId, game: &mut Q2GameServices) {
        match &self.selection {
            Q2MatchSelection::Ctf => self.ctf(game).death(entity, game),
            Q2MatchSelection::Lmctf { .. } => self.lmctf(game).player_death(entity, game),
            Q2MatchSelection::Tag => self.tag(game).player_death(&entity, game),
            _ => {}
        }
    }

    /// Handle a match disconnect (`disconnect`).
    pub fn disconnect(&self, entity: ActorId, game: &mut Q2GameServices) {
        match &self.selection {
            Q2MatchSelection::Ctf => self.ctf(game).disconnect(entity, game),
            Q2MatchSelection::Lmctf { .. } => self.lmctf(game).disconnect(entity, game),
            Q2MatchSelection::Tag => self.tag(game).disconnect(&entity, game),
            _ => {}
        }
    }

    /// Scale match damage (`changeDamage`).
    pub fn damage(&self, target: ActorId, attacker: Option<ActorId>, game: &mut Q2GameServices, amount: f64) -> f64 {
        match &self.selection {
            Q2MatchSelection::Tag => self.tag(game).change_damage(&target, attacker.as_ref(), amount, game),
            Q2MatchSelection::Deathball { .. } => {
                self.deathball(game).change_damage(&target, attacker.as_ref(), amount, game)
            }
            _ => amount,
        }
    }

    /// Scale match knockback (`changeKnockback`).
    pub fn knockback(&self, target: ActorId, game: &mut Q2GameServices, amount: f64, means: i32) -> f64 {
        match &self.selection {
            Q2MatchSelection::Deathball { .. } => self.deathball(game).change_knockback(&target, amount, means, game),
            _ => amount,
        }
    }

    /// Check the match rules (`checkRules`).
    pub fn check_rules(&self, game: &mut Q2GameServices) -> bool {
        match &self.selection {
            Q2MatchSelection::Deathball { .. } => self.deathball(game).check_rules(game),
            Q2MatchSelection::Ctf => self.ctf(game).check_rules(game),
            _ => false,
        }
    }

    /// Drop match inventory (`dropInventory`).
    pub fn drop_inventory(&self, entity: ActorId, game: &mut Q2GameServices) {
        match &self.selection {
            Q2MatchSelection::Ctf => self.ctf(game).drop_inventory(entity, game),
            Q2MatchSelection::Lmctf { .. } => self.lmctf(game).drop_inventory(entity, game),
            _ => {}
        }
    }

    /// Handle a match command (`command`).
    pub fn command(&self, entity: ActorId, game: &mut Q2GameServices, name: &str, args: &[String]) -> bool {
        match &self.selection {
            Q2MatchSelection::Ctf => self.ctf(game).command(entity, game, name, args),
            Q2MatchSelection::Lmctf { .. } => self.lmctf(game).command(entity, game, name, args),
            _ => false,
        }
    }

    /// Run before a match player frame (`beforePlayer`).
    pub fn before_player(&self, entity: ActorId, game: &mut Q2GameServices) {
        if matches!(self.selection, Q2MatchSelection::Ctf) {
            self.ctf(game).before_player(entity, game);
        }
    }

    /// Respawn a match player (`playerSpawned`).
    pub fn player_spawned(&self, entity: ActorId, game: &mut Q2GameServices) {
        match &self.selection {
            Q2MatchSelection::Lmctf { .. } => self.lmctf(game).player_spawned(entity, game),
            Q2MatchSelection::Ctf => {
                if game.ctf.states.get(&entity).map(|state| state.team).unwrap_or(0) == 0 {
                    return;
                }
                let team = game.ctf.states.get(&entity).map(|state| state.team).unwrap_or(0);
                let owned = game.owned_of(entity.clone());
                game.host.combat().set_traits(
                    &owned,
                    &CombatTraitChanges {
                        team: Some(Some(ctf_team_name(team).to_string())),
                        ..CombatTraitChanges::default()
                    },
                );
                if game.ctf.equipment.is_some()
                    && game.players.states.get(&entity).map(|player| player.use_q2_weapons).unwrap_or(false)
                {
                    game.host.inventory().configure(
                        &owned,
                        &InventoryEntry { item: item_id("q2:weapon_grapple"), count: 1.0, capacity: 1.0, count_policy: None },
                    );
                }
                self.ctf(game).assign_skin(entity, game);
            }
            _ => {}
        }
    }

    /// Read the match gravity scale (`gravityScale`).
    pub fn gravity_scale(&self, actor: &ActorId, game: &Q2GameServices) -> f64 {
        match &self.selection {
            Q2MatchSelection::Lmctf { .. } => f64::from(self.lmctf(game).gravity_scale(actor, game)),
            _ => 1.0,
        }
    }

    /// Whether armor applies to a hit (`armorAllowed`).
    fn armor_allowed(
        &self,
        game: &Q2GameServices,
        attacker: Option<&ActorId>,
        target: &CombatState,
        attacker_state: Option<&CombatState>,
    ) -> bool {
        let unteamed = attacker.is_none()
            || attacker_state.is_none()
            || std::ptr::eq(attacker_state.unwrap(), target)
            || target.team.is_none()
            || target.team != attacker_state.and_then(|state| state.team.clone());
        if matches!(self.selection, Q2MatchSelection::Lmctf { .. }) {
            return game.lmctf.rules.ctf_flags & 1024 == 0 || unteamed;
        }
        game.deathmatch_flags() & 262144 == 0 || unteamed
    }

    /// Scale damage before momentum (`beforeMomentum`).
    pub fn before_momentum(&self, game: &mut Q2GameServices, attacker: Option<ActorId>, damage: f64) -> f64 {
        match &self.selection {
            Q2MatchSelection::Ctf => {
                self.ctf(game).techs().strength(attacker, game, damage)
            }
            Q2MatchSelection::Lmctf { .. } => self.lmctf(game).runes().damage(attacker, damage, game),
            _ => damage,
        }
    }

    /// Whether power armor applies (`powerArmorAllowed`).
    pub fn power_armor_allowed(
        &self,
        game: &Q2GameServices,
        attacker: Option<&ActorId>,
        target: &CombatState,
        attacker_state: Option<&CombatState>,
    ) -> bool {
        match &self.selection {
            Q2MatchSelection::Ctf => self.armor_allowed(game, attacker, target, attacker_state),
            _ => true,
        }
    }

    /// Whether armor applies (`armorAllowed`).
    pub fn armor_allowed_effect(
        &self,
        game: &Q2GameServices,
        attacker: Option<&ActorId>,
        target: &CombatState,
        attacker_state: Option<&CombatState>,
    ) -> bool {
        match &self.selection {
            Q2MatchSelection::Ctf | Q2MatchSelection::Lmctf { .. } => {
                self.armor_allowed(game, attacker, target, attacker_state)
            }
            _ => true,
        }
    }

    /// Scale damage after armor (`afterArmor`).
    pub fn after_armor(&self, game: &mut Q2GameServices, target: ActorId, take: f64) -> f64 {
        match &self.selection {
            Q2MatchSelection::Ctf => self.ctf(game).techs().resistance(target, game, take),
            _ => take,
        }
    }

    /// Scale damage after power armor (`afterPowerArmor`).
    pub fn after_power_armor(&self, game: &mut Q2GameServices, target: ActorId, take: f64) -> f64 {
        match &self.selection {
            Q2MatchSelection::Lmctf { .. } => self.lmctf(game).runes().after_power_armor(target, take, game),
            _ => take,
        }
    }

    /// Apply damage after health (`afterHealth`).
    pub fn after_health(
        &self,
        game: &mut Q2GameServices,
        target: ActorId,
        attacker: Option<ActorId>,
        applied: f64,
    ) {
        match &self.selection {
            Q2MatchSelection::Ctf => self.ctf(game).flags().hurt_carrier(target, attacker, game),
            Q2MatchSelection::Lmctf { .. } => self.lmctf(game).runes().after_health(target, attacker, applied, game),
            _ => {}
        }
    }

    /// Update match presentation (`effects`).
    pub fn effects(&self, entity: ActorId, game: &mut Q2GameServices) {
        match &self.selection {
            Q2MatchSelection::Ctf => self.ctf(game).after_player(entity, game),
            Q2MatchSelection::Lmctf { .. } => self.lmctf(game).player_frame(entity, game),
            Q2MatchSelection::Tag => {
                let bits = self.tag(game).effects(&entity, game);
                game.require_entity_mut(&entity).effects =
                    game.require_entity(&entity).effects & !0x20000000 | bits;
                game.show(entity);
            }
            _ => {}
        }
    }
}
