//! Q2 rerelease goals (`src/content/q2/rerelease/goals.ts`).
//!
//! Gameplay logic adapted from id Software's Quake II game and the
//! rerelease game DLL (GPL-2.0-or-later).

use qa_core::identity::ActorId;

use crate::q2::base::player::index::Q2Intermission;
use crate::q2::base::player::types::Q2PlayerEvent;
use crate::q2::foundation::callbacks::Q2CallbackDefinitions;
use crate::q2::foundation::fields::number_field;
use crate::q2::foundation::host::{Q2GameServices, Q2Mode, Q2PresentationEvent, Q2Think, Q2Use};

use super::types::{Q2RereleaseEvent, Q2RereleaseHooks};

/// Rerelease goals checkpoint (`Q2RereleaseGoalsCheckpoint`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Q2RereleaseGoalsCheckpoint {
    /// Goal list.
    pub goals: Option<String>,
    /// Goal number.
    pub goal_number: i32,
}

/// Rerelease goals (`Q2RereleaseGoals`).
#[derive(Debug, Clone, Copy)]
pub struct Q2RereleaseGoals {
    /// Session hooks.
    pub hooks: Q2RereleaseHooks,
}

/// Rerelease goal callbacks (`Q2RereleaseGoals::callbacks`).
pub fn rerelease_goal_callbacks() -> Q2CallbackDefinitions {
    let mut callbacks = Q2CallbackDefinitions::default();
    callbacks.think.insert("rr.G_VerifyTargetted", rerelease_verify_target as Q2Think);
    callbacks.use_.insert("rr.Use_Target_Help", rerelease_help_use as Q2Use);
    callbacks.use_.insert("rr.use_target_goal_or_secret", rerelease_goal_use as Q2Use);
    callbacks
}

/// Spawn rerelease goals (`Q2RereleaseGoals::spawn`).
pub fn rerelease_goals_spawn(entity: ActorId, game: &mut Q2GameServices) -> bool {
    Q2RereleaseGoals { hooks: super::rerelease_hooks(game) }.spawn(entity, game)
}

/// Target help use (`helpUse`).
fn rerelease_help_use(entity: ActorId, game: &mut Q2GameServices, other: Option<ActorId>, activator: Option<ActorId>) {
    Q2RereleaseGoals { hooks: super::rerelease_hooks(game) }.help_use(entity, game, other, activator);
}

/// Target goal or secret use (`goalUse`).
fn rerelease_goal_use(entity: ActorId, game: &mut Q2GameServices, _other: Option<ActorId>, activator: Option<ActorId>) {
    Q2RereleaseGoals { hooks: super::rerelease_hooks(game) }.goal_use(entity, game, activator);
}

/// Verify a goal target (`verifyTarget`).
fn rerelease_verify_target(entity: ActorId, game: &mut Q2GameServices) {
    let record = game.require_entity(&entity).clone();
    if record.targetname.is_empty() {
        game.host.diagnostic(&format!("WARNING: missing targetname on {}", record.classname));
    } else if !game.entities.values().any(|other| other.target == record.targetname) {
        game.host.diagnostic(&format!("WARNING: nothing targets {} {}", record.classname, record.targetname));
    }
}

/// Truncate a message to 511 characters.
fn truncate_message(message: &str) -> String {
    message.chars().take(511).collect()
}

impl Q2RereleaseGoals {
    /// Spawn goals and secrets.
    fn spawn(&self, entity: ActorId, game: &mut Q2GameServices) -> bool {
        let record = game.require_entity(&entity).clone();
        if record.classname == "worldspawn" {
            if number_field(&record.spawn, "hub_map", 0.0) != 0.0 {
                game.rerelease.campaign.mission = super::campaign::Q2RereleaseMission::default();
                for extra in game.rerelease.states.values_mut() {
                    extra.game_help1_changed = 0;
                    extra.game_help2_changed = 0;
                }
            }
            game.rerelease.goals = record.spawn.values.get("goals").cloned();
            if game.rerelease.goals.is_some() {
                game.rerelease.campaign.mission.primary_changes += 1;
            }
            return false;
        }
        if record.classname != "target_help" && record.classname != "target_secret" && record.classname != "target_goal" {
            return false;
        }
        if game.options.mode == Q2Mode::Deathmatch {
            game.remove_actor(entity);
            return true;
        }
        if record.classname == "target_help" {
            if record.message.is_empty() {
                game.host.diagnostic("target_help has no message");
                game.remove_actor(entity);
                return true;
            }
            game.require_entity_mut(&entity).use_ = Some(rerelease_help_use as Q2Use);
            return true;
        }
        game.require_entity_mut(&entity).visible = false;
        game.require_entity_mut(&entity).server_flags |= 1;
        game.require_entity_mut(&entity).use_ = Some(rerelease_goal_use as Q2Use);
        if record.classname == "target_secret" {
            game.counters.total_secrets += 1;
            game.schedule(entity, 0.01, rerelease_verify_target as Q2Think);
        } else {
            game.counters.total_goals += 1;
        }
        true
    }

    /// Use a help target.
    fn help_use(&self, entity: ActorId, game: &mut Q2GameServices, other: Option<ActorId>, activator: Option<ActorId>) {
        let record = game.require_entity(&entity).clone();
        let mut changed = false;
        if record.spawnflags & 1 != 0 {
            if game.rerelease.campaign.mission.primary != record.message {
                game.rerelease.campaign.mission.primary = truncate_message(&record.message);
                game.rerelease.campaign.mission.primary_changes += 1;
                changed = true;
            }
        } else if game.rerelease.campaign.mission.secondary != record.message {
            game.rerelease.campaign.mission.secondary = truncate_message(&record.message);
            game.rerelease.campaign.mission.secondary_changes += 1;
            changed = true;
        }
        if changed {
            game.host_emit(Q2PresentationEvent::Help {
                slot: if record.spawnflags & 1 != 0 { 1 } else { 2 },
                text: record.message.clone(),
            });
        }
        if record.spawnflags & 2 != 0 {
            let set_poi = game.rerelease.set_poi.expect("Q2 rerelease POI use is not registered");
            set_poi(entity, game, other, activator);
        }
    }

    /// Use a goal or secret target.
    fn goal_use(&self, entity: ActorId, game: &mut Q2GameServices, activator: Option<ActorId>) {
        let record = game.require_entity(&entity).clone();
        let noise = record.spawn.values.get("noise").cloned().unwrap_or_else(|| "misc/secret.wav".to_string());
        game.sound(&entity, &noise, 2, 1.0, 1.0);
        if record.classname == "target_secret" {
            game.counters.found_secrets += 1;
        } else {
            game.counters.found_goals += 1;
            if game.counters.found_goals == game.counters.total_goals && record.spawnflags & 1 == 0 {
                game.host_emit(Q2PresentationEvent::Music {
                    track: format!("{}", number_field(&record.spawn, "sounds", 0.0)),
                });
            }
            if game.rerelease.goals.is_some() {
                game.rerelease.goal_number += 1;
                game.rerelease.campaign.mission.primary_changes += 1;
                for actor in game.host.players() {
                    if game.entity(&actor).is_some() {
                        self.notify(actor, game);
                    }
                }
            }
        }
        let authored = game.require_entity(&entity).authored_target();
        game.use_targets(&authored, activator.as_ref(), false);
        game.remove_actor(entity);
    }

    /// Notify a player of mission objectives (`notify`).
    pub fn notify(&self, player: ActorId, game: &mut Q2GameServices) {
        if game.options.mode == Q2Mode::Deathmatch {
            return;
        }
        let entered_at = game.players.states.get(&player).map(|state| state.entered_at);
        let spawned = game.rerelease.states.get(&player).map(|extra| extra.spawned);
        let (Some(entered_at), Some(spawned)) = (entered_at, spawned) else {
            return;
        };
        if !spawned || game.now() - entered_at < 0.3 {
            return;
        }
        if game.rerelease.goals.is_some() {
            let primary_changes = game.rerelease.campaign.mission.primary_changes;
            let secondary_changes = game.rerelease.campaign.mission.secondary_changes;
            if primary_changes != secondary_changes {
                let goals = game.rerelease.goals.clone().unwrap_or_default();
                let goal_number = game.rerelease.goal_number;
                let goal = if goal_number < 0 {
                    None
                } else {
                    goals.split('\t').nth(goal_number as usize)
                };
                let Some(goal) = goal else {
                    panic!("Invalid Quake 64 goals: completed goal index exceeds the authored goal list");
                };
                game.rerelease.campaign.mission.primary = truncate_message(goal);
                game.rerelease.campaign.mission.secondary_changes = primary_changes;
            }
            let primary_changes = game.rerelease.campaign.mission.primary_changes;
            let primary = game.rerelease.campaign.mission.primary.clone();
            let changed = game
                .rerelease
                .states
                .get(&player)
                .expect("Q2 rerelease player is not admitted")
                .game_help1_changed
                != primary_changes;
            if changed {
                (self.hooks.emit)(
                    game,
                    Q2RereleaseEvent::MissionObjective {
                        actor: player.clone(),
                        text: primary,
                        args: Vec::new(),
                        talk_sound: true,
                    },
                );
                game.rerelease
                    .states
                    .get_mut(&player)
                    .expect("Q2 rerelease player is not admitted")
                    .game_help1_changed = primary_changes;
            }
            return;
        }
        let primary_changes = game.rerelease.campaign.mission.primary_changes;
        let secondary_changes = game.rerelease.campaign.mission.secondary_changes;
        let primary = game.rerelease.campaign.mission.primary.clone();
        let secondary = game.rerelease.campaign.mission.secondary.clone();
        let now = game.now();
        let notify_primary = {
            let extra = game
                .rerelease
                .states
                .get_mut(&player)
                .expect("Q2 rerelease player is not admitted");
            if extra.game_help1_changed == primary_changes {
                false
            } else {
                extra.game_help1_changed = primary_changes;
                extra.help_changed = 1;
                extra.help_time = now + 5.0;
                !primary.is_empty()
            }
        };
        if notify_primary {
            (self.hooks.emit)(
                game,
                Q2RereleaseEvent::MissionObjective {
                    actor: player.clone(),
                    text: "$g_primary_mission_objective".to_string(),
                    args: vec![primary],
                    talk_sound: false,
                },
            );
        }
        let notify_secondary = {
            let extra = game
                .rerelease
                .states
                .get_mut(&player)
                .expect("Q2 rerelease player is not admitted");
            if extra.game_help2_changed == secondary_changes {
                false
            } else {
                extra.game_help2_changed = secondary_changes;
                extra.help_changed = 1;
                extra.help_time = now + 5.0;
                !secondary.is_empty()
            }
        };
        if notify_secondary {
            (self.hooks.emit)(
                game,
                Q2RereleaseEvent::MissionObjective {
                    actor: player,
                    text: "$g_secondary_mission_objective".to_string(),
                    args: vec![secondary],
                    talk_sound: false,
                },
            );
        }
    }

    /// Run the end-of-frame mission status (`endPlayerFrame`).
    pub fn end_player_frame(&self, player: ActorId, game: &mut Q2GameServices) {
        let now = game.now();
        let (help_changed, help_time) = {
            let extra = game
                .rerelease
                .states
                .get(&player)
                .expect("Q2 rerelease player is not admitted");
            (extra.help_changed, extra.help_time)
        };
        if help_changed != 0 && help_changed <= 3 && help_time < now {
            if help_changed == 1 {
                game.sound(&player, "misc/pc_up.wav", 0, 1.0, 3.0);
            }
            let extra = game
                .rerelease
                .states
                .get_mut(&player)
                .expect("Q2 rerelease player is not admitted");
            extra.help_changed += 1;
            extra.help_time = now + 5.0;
        }
        let help_changed = game
            .rerelease
            .states
            .get(&player)
            .expect("Q2 rerelease player is not admitted")
            .help_changed;
        (self.hooks.emit)(
            game,
            Q2RereleaseEvent::MissionStatus {
                actor: player,
                icon_visible: help_changed >= 1 && help_changed <= 2 && (now * 1000.0).trunc() as i64 % 1000 < 500,
            },
        );
    }

    /// Toggle the help computer (`help`).
    pub fn help(&self, player: ActorId, game: &mut Q2GameServices) {
        if game.players.intermission != Q2Intermission::Playing {
            return;
        }
        let primary_changes = game.rerelease.campaign.mission.primary_changes;
        let secondary_changes = game.rerelease.campaign.mission.secondary_changes;
        let primary = game.rerelease.campaign.mission.primary.clone();
        let secondary = game.rerelease.campaign.mission.secondary.clone();
        let extra_help1 = game
            .rerelease
            .states
            .get(&player)
            .expect("Q2 rerelease player is not admitted")
            .game_help1_changed;
        let extra_help2 = game
            .rerelease
            .states
            .get(&player)
            .expect("Q2 rerelease player is not admitted")
            .game_help2_changed;
        let state = game
            .players
            .states
            .get_mut(&player)
            .expect("Q2 player has not been admitted");
        state.show_inventory = false;
        state.show_scores = false;
        if state.show_help && (extra_help1 == primary_changes || extra_help2 == secondary_changes) {
            state.show_help = false;
        } else {
            state.show_help = true;
            game.rerelease
                .states
                .get_mut(&player)
                .expect("Q2 rerelease player is not admitted")
                .help_changed = 0;
        }
        let visible = game
            .players
            .states
            .get(&player)
            .expect("Q2 player has not been admitted")
            .show_help;
        let player_hooks = game.players.hooks.expect("Q2 player hooks are not registered");
        (player_hooks.emit)(Q2PlayerEvent::Help { actor: player.clone(), visible });
        (self.hooks.emit)(
            game,
            Q2RereleaseEvent::HelpComputer {
                actor: player,
                visible,
                primary,
                secondary,
                slow_time: visible,
            },
        );
    }

    /// Capture goals (`capture`).
    pub fn capture(&self, game: &Q2GameServices) -> Q2RereleaseGoalsCheckpoint {
        Q2RereleaseGoalsCheckpoint {
            goals: game.rerelease.goals.clone(),
            goal_number: game.rerelease.goal_number,
        }
    }

    /// Restore goals (`restore`).
    pub fn restore(&self, game: &mut Q2GameServices, checkpoint: &Q2RereleaseGoalsCheckpoint) {
        game.rerelease.goals = checkpoint.goals.clone();
        game.rerelease.goal_number = checkpoint.goal_number;
    }
}
