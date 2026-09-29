//! Rerelease bot brain from `src/bots/behavior/rerelease/brain.ts`.
//!
//! One instance per bot, one `think()` per server frame, one usercmd
//! out. Each frame reads self, decays awareness, picks a target,
//! picks a goal (explicit, target, item, roam), plans a path, runs
//! the path controller, aims the tracker, and fires the selected
//! weapon. Stuck escalation, wedge detection, hazard guards, CTF
//! objectives, coop regrouping, and interactable errands all live here.
//!
//! Determinism: every random decision draws from the injected RNG; two
//! brains with the same seed, fed the same worlds, emit the same cmds.

use std::collections::{HashMap, HashSet};

use qa_core::math::Vec3;

use crate::behavior::rerelease::aim::{aim_error, aim_lead_point, aim_step, new_aim_state, BotAimStateT};
use crate::behavior::rerelease::data::botdata::{BotSkillSettings, CharacterEntry};
use crate::behavior::rerelease::data::knowledge::{
    choose_weapon, item_value, BotGameModeT, BotGameType, BotItemContextT, BotKnowledge, BotWeaponContextT,
    Interaction, ItemFlag,
};
use crate::behavior::rerelease::math::{angle_between, angle_mod, angle_vectors, bvec_distance, bvec_ma, bvec_sub};
use crate::behavior::rerelease::nav::{
    default_traverse_caps, BotTransportStep, NavGraphLinkT, NavLinkType, NavPathT, NavPlanOptions, NavTraverseCapsT,
    PLAN_START_ABOVE,
};
use crate::behavior::rerelease::path_follow::{
    clear_path, follow_path, new_path_state, roll_combat_jump, set_path, steer_direct, BotFollowInputT, BotPathStateT,
    BotPathStatus,
};
use crate::behavior::rerelease::rng::{random_chance, random_index, random_range, Xorshift32};
use crate::behavior::rerelease::senses::{
    can_fire, evaluate_sight_geometry, is_aware, new_awareness, sense_step, should_forget, sound_audible,
    BotAwarenessT, BotContactT,
};
use crate::behavior::rerelease::world::{
    empty_usercmd, BotContents, BotEntityKind, BotEntityT, BotSelfT, BotUsercmdT, BotWorldT, BOT_BUTTON_ATTACK,
    BOT_BUTTON_JUMP, BOT_BUTTON_USE,
};
use crate::error::BotsError;

/// Goal status codes in the brain's vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BotGoalStatus;

impl BotGoalStatus {
    /// Error.
    pub const ERROR: i32 = 0;
    /// Success.
    pub const SUCCESS: i32 = 1;
    /// In progress.
    pub const IN_PROGRESS: i32 = 2;
}

/// One chat line the brain wants said.
#[derive(Debug, Clone, PartialEq)]
pub struct BotChatEventT {
    /// chats.txt locstring.
    pub locstring: String,
    /// chats.txt type.
    pub chat_type: String,
    /// Delay milliseconds.
    pub delay_ms: f32,
    /// Team only.
    pub team_only: bool,
}

/// Movement geometry projected from the selected provider.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BotBrainMovementGeom {
    /// Gravity.
    pub gravity: f32,
    /// Jump velocity.
    pub jump_velocity: f32,
    /// Jump air seconds.
    pub jump_air_seconds: f32,
    /// Maximum landing rise.
    pub maximum_landing_rise: f32,
    /// Path start height.
    pub start_above: f32,
    /// Body mins.
    pub body_mins: Vec3,
    /// Body maxs.
    pub body_maxs: Vec3,
}

impl Default for BotBrainMovementGeom {
    fn default() -> Self {
        Self {
            gravity: 800.0,
            jump_velocity: 270.0,
            jump_air_seconds: 0.7,
            maximum_landing_rise: 40.0,
            start_above: PLAN_START_ABOVE,
            body_mins: Vec3 {
                x: -16.0,
                y: -16.0,
                z: -6.0,
            },
            body_maxs: Vec3 {
                x: 16.0,
                y: 16.0,
                z: 32.0,
            },
        }
    }
}

/// Brain configuration.
pub struct BotBrainConfig {
    /// Shared knowledge.
    pub knowledge: BotKnowledge,
    /// Skill name.
    pub skill: String,
    /// Game mode.
    pub game_mode: BotGameModeT,
    /// Character entry.
    pub character: Option<CharacterEntry>,
    /// Maximum health.
    pub max_health: f32,
    /// Run speed.
    pub run_speed: f32,
    /// Walk speed.
    pub walk_speed: f32,
    /// Movement geometry.
    pub movement: BotBrainMovementGeom,
    /// Chat sink.
    pub on_chat: Option<Box<dyn FnMut(BotChatEventT)>>,
    /// Weapon number to impulse.
    pub weapon_impulse: Option<Box<dyn Fn(i32) -> i32>>,
    /// Direct weapon select.
    pub on_weapon_select: Option<Box<dyn FnMut(i32)>>,
    /// Human teammate nearby.
    pub human_teammate_near: Box<dyn Fn() -> bool>,
}

/// Explicit goal owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExplicitGoalOwner {
    /// External (QuakeC).
    External,
    /// Objective.
    Objective,
}

/// Explicit goal kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExplicitGoalKind {
    /// Point.
    Point,
    /// Entity.
    Entity,
}

/// The goal the caller set explicitly.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExplicitGoalT {
    /// Owner.
    pub owner: ExplicitGoalOwner,
    /// Kind.
    pub kind: ExplicitGoalKind,
    /// Point.
    pub point: Vec3,
    /// Entity id.
    pub entity_id: i32,
}

// Tuning constants (see the donor header for the measured rationale).
const STUCK_SECONDS: f32 = 0.6;
const REPLAN_SECONDS: f32 = 2.0;
const STUCK_GIVE_UP: i32 = 3;
const ROAM_RADIUS: f32 = 4096.0;
const EDGE_LOOKAHEAD: f32 = 32.0;
const EDGE_STOP_SECONDS: f32 = 0.15;
const EDGE_PROBE_STEP: f32 = 16.0;
const EDGE_DROP_CHECK: f32 = 1024.0;
const GAP_LANDING_SEARCH: f32 = 320.0;
const GAP_JUMP_TRIGGER: f32 = 32.0;
const GAP_REACH_MARGIN: f32 = 0.85;
const LAVA_EXIT_SAMPLES: usize = 12;
const LAVA_EXIT_REACH: f32 = 96.0;
const POINT_REST_SECONDS: f32 = 8.0;
const ESCORT_DISTANCE: f32 = 160.0;
const UNREACHABLE_SECONDS: f32 = 20.0;
const UNREACHABLE_LIVE_SECONDS: f32 = 4.0;
const UNSTICK_SECONDS: f32 = 0.5;
const INTERACT_RADIUS: f32 = 1024.0;
const INTERACT_SECONDS: f32 = 8.0;
const INTERACT_REACHED: f32 = 40.0;
const WEDGED_SECONDS: f32 = 1.2;
const WEDGED_GIVE_UP_SECONDS: f32 = 2.5;
const WEDGED_DISPLACEMENT: f32 = 96.0;
const BLIND_ROAM_RADIUS: f32 = 640.0;
const BLIND_ROAM_SECONDS: f32 = 4.0;
const COOP_REGROUP_SECONDS: f32 = 15.0;
const COOP_REGROUP_NEAR: f32 = 250.0;
const COOP_REGROUP_GIVE_UP: f32 = 12.0;
const COOP_HUNT_RADIUS: f32 = 2000.0;
const OBJECTIVE_GUARD_RADIUS: f32 = 384.0;
const CARRIER_HOLD_RADIUS: f32 = 192.0;
const UNSTICK_STEP: f32 = 40.0;
const UNSTICK_MAX_DROP: f32 = 96.0;
const SURFACE_REACH: f32 = 96.0;
const TOUCH_RADIUS: f32 = 12.0;
const GATE_SHOOT_RANGE: f32 = 768.0;
const GATE_AIM_DEGREES: f32 = 5.0;
const GATE_SHOT_SECONDS: f32 = 0.7;
const OBJECTIVE_AWAY: f32 = 96.0;

/// Whether lava or slime lies under a point.
fn hazard_below(world: &dyn BotWorldT, x: f32, y: f32, z: f32) -> bool {
    let trace = world.trace_line(
        Vec3 { x, y, z },
        Vec3 {
            x,
            y,
            z: z - EDGE_DROP_CHECK,
        },
    );
    let contents = world.point_contents(Vec3 {
        x,
        y,
        z: trace.endpos.z + 2.0,
    });
    contents == BotContents::LAVA || contents == BotContents::SLIME
}

/// Brain memory: everything the brain remembers across frames.
#[derive(Debug, Clone)]
pub struct BotBrainMemory {
    /// Trigger weapon number.
    pub trigger_weapon: i32,
    /// Trigger held since.
    pub trigger_held_since: f32,
    /// Trigger ready at.
    pub trigger_ready_at: f32,
    /// Aim state.
    pub aim: BotAimStateT,
    /// Path state.
    pub path_state: BotPathStateT,
    /// Awareness by entity.
    pub awareness: HashMap<i32, BotAwarenessT>,
    /// Target id.
    pub target_id: i32,
    /// Goal point.
    pub goal_point: Option<Vec3>,
    /// Goal entity id.
    pub goal_entity_id: i32,
    /// Unreachable-until by entity.
    pub unreachable_until: HashMap<i32, f32>,
    /// Consecutive stuck trips.
    pub stuck_trips: i32,
    /// Goal is live.
    pub goal_is_live: bool,
    /// Unstick window end.
    pub unstick_until: f32,
    /// Interact errand end.
    pub press_until: f32,
    /// Unstick side.
    pub unstick_side: f32,
    /// Explicit goal.
    pub explicit_goal: Option<ExplicitGoalT>,
    /// Explicit goal done.
    pub explicit_goal_done: bool,
    /// Explicit goal failed.
    pub explicit_goal_failed: bool,
    /// Wedge origin.
    pub wedge_origin: Option<Vec3>,
    /// Wedge start.
    pub wedge_since: f32,
    /// Last safe origin.
    pub last_safe_origin: Option<Vec3>,
    /// Rest point.
    pub rest_point: Option<Vec3>,
    /// Rest until.
    pub rest_until: f32,
    /// Guard refusals.
    pub guard_refusals: i32,
    /// Last guard refused.
    pub last_guard_refused: bool,
    /// Gap jumps.
    pub gap_jumps: i32,
    /// Hazard frames.
    pub hazard_frames: i32,
    /// Objective home by entity.
    pub objective_home: HashMap<i32, Vec3>,
    /// Own objective home.
    pub own_objective_home: Option<Vec3>,
    /// Enemy objective home.
    pub enemy_objective_home: Option<Vec3>,
    /// Objective role.
    pub objective_role: String,
    /// Touch goal.
    pub touch_goal: bool,
    /// Hold position.
    pub hold_position: bool,
    /// Gate shoot target.
    pub gate_shoot_at: Option<Vec3>,
    /// Gate fired at.
    pub gate_fired_at: f32,
    /// Coop regrouping.
    pub coop_regrouping: bool,
    /// Coop regroup at.
    pub coop_regroup_at: f32,
    /// Coop regroup until.
    pub coop_regroup_until: f32,
    /// Lines said this level.
    pub said_this_level: HashSet<String>,
    /// Level started.
    pub level_started: bool,
    /// Check-six until.
    pub check_six_until: f32,
    /// Check-six next at.
    pub check_six_next_at: f32,
    /// Roam point.
    pub roam_point: Option<Vec3>,
    /// Roam until.
    pub roam_until: f32,
    /// Last command.
    pub last_cmd: BotUsercmdT,
    /// Spawned once.
    pub spawned_once: bool,
    /// Last weapon number.
    pub last_weapon_number: i32,
    /// Dead since.
    pub dead_since: f32,
    /// Respawn wait.
    pub respawn_wait: f32,
    /// Respawn press toggle.
    pub respawn_press: bool,
}

impl Default for BotBrainMemory {
    fn default() -> Self {
        Self {
            trigger_weapon: 0,
            trigger_held_since: -1.0,
            trigger_ready_at: 0.0,
            aim: new_aim_state(0.0, 0.0),
            path_state: new_path_state(),
            awareness: HashMap::new(),
            target_id: -1,
            goal_point: None,
            goal_entity_id: -1,
            unreachable_until: HashMap::new(),
            stuck_trips: 0,
            goal_is_live: false,
            unstick_until: 0.0,
            press_until: 0.0,
            unstick_side: 0.0,
            explicit_goal: None,
            explicit_goal_done: false,
            explicit_goal_failed: false,
            wedge_origin: None,
            wedge_since: -1.0,
            last_safe_origin: None,
            rest_point: None,
            rest_until: 0.0,
            guard_refusals: 0,
            last_guard_refused: false,
            gap_jumps: 0,
            hazard_frames: 0,
            objective_home: HashMap::new(),
            own_objective_home: None,
            enemy_objective_home: None,
            objective_role: String::new(),
            touch_goal: false,
            hold_position: false,
            gate_shoot_at: None,
            gate_fired_at: -1.0,
            coop_regrouping: false,
            coop_regroup_at: 0.0,
            coop_regroup_until: 0.0,
            said_this_level: HashSet::new(),
            level_started: false,
            check_six_until: 0.0,
            check_six_next_at: 0.0,
            roam_point: None,
            roam_until: 0.0,
            last_cmd: empty_usercmd(),
            spawned_once: false,
            last_weapon_number: 0,
            dead_since: -1.0,
            respawn_wait: 0.0,
            respawn_press: false,
        }
    }
}

/// Brain checkpoint.
#[derive(Debug, Clone)]
pub struct BotBrainCheckpoint {
    /// Version.
    pub version: u32,
    /// Skill.
    pub skill: String,
    /// Game mode.
    pub game_mode: BotGameModeT,
    /// Memory.
    pub memory: BotBrainMemory,
}

/// The bot brain.
pub struct BotBrain {
    state: BotBrainMemory,
    /// Configuration.
    pub config: BotBrainConfig,
    /// Skill settings.
    pub settings: BotSkillSettings,
    rng: Xorshift32,
}

impl BotBrain {
    /// New brain. Fails when no skill settings exist.
    pub fn new(config: BotBrainConfig, seed: i32) -> Result<Self, BotsError> {
        let settings = config
            .knowledge
            .skill(&config.skill)
            .or_else(|| config.knowledge.skills.first())
            .cloned()
            .ok_or_else(|| {
                BotsError::BotScript(format!(
                    "bot_brain: no skill settings available (asked for \"{}\")",
                    config.skill
                ))
            })?;
        Ok(Self {
            state: BotBrainMemory::default(),
            config,
            settings,
            rng: Xorshift32::new(seed),
        })
    }

    /// Guard refusal count (tests).
    #[must_use]
    pub fn guard_refusals(&self) -> i32 {
        self.state.guard_refusals
    }

    /// Gap jump count (tests).
    #[must_use]
    pub fn gap_jumps(&self) -> i32 {
        self.state.gap_jumps
    }

    /// Hazard frame count (tests).
    #[must_use]
    pub fn hazard_frames(&self) -> i32 {
        self.state.hazard_frames
    }

    /// RNG state word.
    #[must_use]
    pub fn rng_state(&self) -> i32 {
        self.rng.peek()
    }

    /// Restore the RNG state word.
    pub fn restore_rng(&mut self, state: i32) -> Result<(), BotsError> {
        self.rng.restore(state)
    }

    /// Checkpoint behavior memory.
    #[must_use]
    pub fn checkpoint(&self) -> BotBrainCheckpoint {
        BotBrainCheckpoint {
            version: 1,
            skill: self.settings.skill.clone(),
            game_mode: self.config.game_mode.clone(),
            memory: self.state.clone(),
        }
    }

    /// Restore behavior memory.
    pub fn restore(&mut self, checkpoint: &BotBrainCheckpoint) -> Result<(), BotsError> {
        if checkpoint.version != 1 || checkpoint.skill != self.settings.skill {
            return Err(BotsError::BotCheckpoint(
                "bot behavior checkpoint has another source skill/profile version".to_owned(),
            ));
        }
        self.state = checkpoint.memory.clone();
        self.config.game_mode = checkpoint.game_mode.clone();
        Ok(())
    }

    /// `bot_movetopoint`: path to a world point.
    pub fn request_move_to_point(&mut self, point: Vec3) {
        self.set_point_goal(point, ExplicitGoalOwner::External);
    }

    /// Objective point goal (external goals win).
    pub fn set_objective_goal(&mut self, point: Option<Vec3>) {
        if self
            .state
            .explicit_goal
            .is_some_and(|goal| goal.owner == ExplicitGoalOwner::External)
        {
            return;
        }
        match point {
            None => {
                if self
                    .state
                    .explicit_goal
                    .is_some_and(|goal| goal.owner == ExplicitGoalOwner::Objective)
                {
                    self.clear_explicit_goal();
                }
            }
            Some(point) => self.set_point_goal(point, ExplicitGoalOwner::Objective),
        }
    }

    fn set_point_goal(&mut self, point: Vec3, owner: ExplicitGoalOwner) {
        let same = self.state.explicit_goal.is_some_and(|goal| {
            goal.owner == owner && goal.kind == ExplicitGoalKind::Point && bvec_distance(goal.point, point) < 8.0
        });
        if same {
            return;
        }
        self.state.explicit_goal = Some(ExplicitGoalT {
            owner,
            kind: ExplicitGoalKind::Point,
            point,
            entity_id: -1,
        });
        self.state.explicit_goal_done = false;
        self.state.explicit_goal_failed = false;
        clear_path(&mut self.state.path_state);
    }

    /// `bot_followentity`: path to an entity.
    pub fn request_follow_entity(&mut self, entity_id: i32, origin: Vec3) {
        if let Some(goal) = self.state.explicit_goal.as_mut() {
            if goal.kind == ExplicitGoalKind::Entity && goal.entity_id == entity_id {
                goal.point = origin;
                return;
            }
        }
        self.state.explicit_goal = Some(ExplicitGoalT {
            owner: ExplicitGoalOwner::External,
            kind: ExplicitGoalKind::Entity,
            point: origin,
            entity_id,
        });
        self.state.explicit_goal_done = false;
        self.state.explicit_goal_failed = false;
        clear_path(&mut self.state.path_state);
    }

    /// Explicit goal status.
    #[must_use]
    pub fn goal_status(&self) -> i32 {
        if self.state.explicit_goal.is_none() || self.state.explicit_goal_failed {
            return BotGoalStatus::ERROR;
        }
        if self.state.explicit_goal_done {
            return BotGoalStatus::SUCCESS;
        }
        BotGoalStatus::IN_PROGRESS
    }

    /// Clear the explicit goal.
    pub fn clear_explicit_goal(&mut self) {
        if self.state.explicit_goal.is_some() {
            clear_path(&mut self.state.path_state);
            self.state.goal_point = None;
            self.state.goal_entity_id = -1;
        }
        self.state.explicit_goal = None;
        self.state.explicit_goal_done = false;
        self.state.explicit_goal_failed = false;
    }

    /// Forget everything from the last level.
    pub fn reset_for_level(&mut self) {
        let spawned = self.state.spawned_once;
        self.state = BotBrainMemory::default();
        self.state.spawned_once = spawned;
    }

    /// Set the game mode.
    pub fn set_game_mode(&mut self, mode: BotGameModeT) {
        self.config.game_mode = mode;
    }

    /// Awareness record for an entity.
    #[must_use]
    pub fn awareness_of(&self, id: i32) -> Option<&BotAwarenessT> {
        self.state.awareness.get(&id)
    }

    /// Current target, or -1.
    #[must_use]
    pub fn current_target(&self) -> i32 {
        self.state.target_id
    }

    /// Current path.
    #[must_use]
    pub fn current_path(&self) -> Option<&NavPathT> {
        self.state.path_state.path.as_ref()
    }

    /// Current path index.
    #[must_use]
    pub fn current_path_index(&self) -> usize {
        self.state.path_state.index
    }

    /// Last usercmd produced.
    #[must_use]
    pub fn last_usercmd(&self) -> BotUsercmdT {
        self.state.last_cmd
    }

    fn emit_chat_once(&mut self, chat_type: &str) {
        if self.state.said_this_level.contains(chat_type) {
            return;
        }
        self.state.said_this_level.insert(chat_type.to_owned());
        self.emit_chat(chat_type);
    }

    /// Emit one chats.txt line of a type when its chance rolls true.
    pub fn emit_chat(&mut self, chat_type: &str) {
        let candidates: Vec<(String, String, f32, bool, f32)> = self
            .config
            .knowledge
            .chats_of_type(chat_type)
            .iter()
            .map(|chat| {
                (
                    chat.locstring.clone(),
                    chat.chat_type.clone(),
                    chat.time as f32,
                    chat.team,
                    chat.chance as f32,
                )
            })
            .collect();
        if candidates.is_empty() {
            return;
        }
        let pick = random_index(&mut self.rng, candidates.len());
        let (locstring, chat_type, time, team, chance) = candidates[pick].clone();
        if !random_chance(&mut self.rng, chance) {
            return;
        }
        if let Some(on_chat) = self.config.on_chat.as_mut() {
            on_chat(BotChatEventT {
                locstring,
                chat_type,
                delay_ms: time,
                team_only: team,
            });
        }
    }

    /// One think: read, sense, target, goal, move, aim, fire.
    pub fn think(&mut self, world: &mut dyn BotWorldT) -> BotUsercmdT {
        let bot = world.bot_self();
        let now = world.time();
        let dt = world.frame_time();
        let mut cmd = empty_usercmd();
        if !self.state.spawned_once {
            self.state.spawned_once = true;
            self.emit_chat("connected");
        }
        if !self.state.level_started {
            self.state.level_started = true;
            self.emit_chat("match_start");
        }
        self.state.aim.pitch = bot.view_angles.x;
        self.state.aim.yaw = bot.view_angles.y;
        if bot.dead {
            self.state.trigger_weapon = 0;
            self.state.trigger_held_since = -1.0;
            self.state.trigger_ready_at = 0.0;
            cmd.view_angles = Vec3 {
                x: self.state.aim.pitch,
                y: self.state.aim.yaw,
                z: 0.0,
            };
            if self.state.dead_since < 0.0 {
                self.state.dead_since = now;
                self.state.respawn_wait = random_range(
                    &mut self.rng,
                    self.settings.behaviors.min_respawn_time,
                    self.settings.behaviors.max_respawn_time,
                );
                self.state.respawn_press = false;
            }
            if now - self.state.dead_since >= self.state.respawn_wait {
                self.state.respawn_press = !self.state.respawn_press;
                if self.state.respawn_press {
                    cmd.buttons |= BOT_BUTTON_ATTACK;
                }
            }
            clear_path(&mut self.state.path_state);
            self.state.target_id = -1;
            self.state.awareness.clear();
            self.state.last_cmd = cmd;
            return cmd;
        }
        self.state.dead_since = -1.0;
        let entities = world.entities();
        let sounds = world.hearing();
        self.update_senses(world, &bot, &entities, &sounds, dt, now);
        self.update_objectives(&entities, &bot);
        let target = self.select_target(&entities, bot.team, bot.origin);
        self.state.target_id = target.as_ref().map_or(-1, |target| target.id);
        let wedged = self.update_wedge(bot.origin, now, target.is_some());
        let goal = self.select_goal(world, &entities, target.as_ref(), now);
        let mut move_target: Option<Vec3> = None;
        let mut riding_lift = false;
        let mut hazard_face: Option<Vec3> = None;
        let mut in_hazard_now = false;
        if goal.is_some() && self.state.hold_position {
            clear_path(&mut self.state.path_state);
            self.state.wedge_origin = Some(bot.origin);
            self.state.wedge_since = now;
        } else if let Some(goal) = goal {
            self.ensure_path(world, goal, now);
            let has_nav = world.nav().is_some();
            if self.state.path_state.path.is_none() && has_nav && self.state.goal_entity_id >= 0 {
                let rest = self.unreachable_rest();
                self.state
                    .unreachable_until
                    .insert(self.state.goal_entity_id, now + rest);
                self.abandon_goal();
                self.state.press_until = now + INTERACT_SECONDS;
            }
            let air_above = if bot.water_level >= 3 {
                Some(self.air_above(world, &bot))
            } else {
                None
            };
            let input = BotFollowInputT {
                origin: bot.origin,
                yaw: self.state.aim.yaw,
                on_ground: bot.on_ground,
                water_level: bot.water_level,
                air_seconds: bot.air_seconds,
                air_above,
                velocity: Some(bot.velocity),
                now,
                stuck_time: STUCK_SECONDS,
                run_speed: self.config.run_speed,
                walk_speed: self.config.walk_speed,
            };
            let mut transport =
                |link: &NavGraphLinkT, origin: Vec3| world.nav().and_then(|nav| nav.transport(link, origin));
            let follow = follow_path(
                &mut self.state.path_state,
                &input,
                &self.settings.movement,
                &mut transport,
            );
            if follow.status == BotPathStatus::STUCK {
                self.state.stuck_trips += 1;
                self.state.unstick_until = now + UNSTICK_SECONDS;
                self.state.unstick_side = self.pick_unstick_side(world, &bot);
                if self.state.stuck_trips >= STUCK_GIVE_UP {
                    if self.state.goal_entity_id >= 0 {
                        let rest = self.unreachable_rest();
                        self.state
                            .unreachable_until
                            .insert(self.state.goal_entity_id, now + rest);
                    }
                    self.abandon_goal();
                    self.state.press_until = now + INTERACT_SECONDS;
                    self.state.stuck_trips = 0;
                } else {
                    clear_path(&mut self.state.path_state);
                }
            } else if follow.status == BotPathStatus::ARRIVED {
                if self.state.touch_goal && bvec_distance(bot.origin, goal) >= TOUCH_RADIUS {
                    let (forwardmove, sidemove) = steer_direct(
                        bot.origin,
                        self.state.aim.yaw,
                        goal,
                        self.settings.movement.walk_only,
                        self.config.run_speed,
                        self.config.walk_speed,
                    );
                    cmd.forwardmove = forwardmove;
                    cmd.sidemove = sidemove;
                    move_target = Some(goal);
                } else {
                    self.rest_static_goal(goal, now);
                    self.reach_goal();
                }
            } else if follow.status == BotPathStatus::MOVING {
                self.state.stuck_trips = 0;
                cmd.forwardmove = follow.forwardmove;
                cmd.sidemove = follow.sidemove;
                cmd.upmove = follow.upmove;
                if follow.jump {
                    cmd.buttons |= BOT_BUTTON_JUMP;
                }
                riding_lift = follow.riding;
                move_target = follow.target;
                let bounds = follow
                    .link
                    .and_then(|link| link.entity_bounds)
                    .map(|bounds| (bounds.mins, bounds.maxs));
                self.state.gate_shoot_at = self.gate_to_shoot(world, &entities, &bot, bounds);
            } else if follow.status == BotPathStatus::NO_PATH {
                let reach = if self.state.touch_goal { TOUCH_RADIUS } else { 48.0 };
                if bvec_distance(bot.origin, goal) < reach {
                    self.rest_static_goal(goal, now);
                    self.reach_goal();
                } else {
                    let (forwardmove, sidemove) = steer_direct(
                        bot.origin,
                        self.state.aim.yaw,
                        goal,
                        self.settings.movement.walk_only,
                        self.config.run_speed,
                        self.config.walk_speed,
                    );
                    cmd.forwardmove = forwardmove;
                    cmd.sidemove = sidemove;
                    move_target = Some(goal);
                }
            }
        }
        if self.in_hazard(world, &bot) {
            self.state.hazard_frames += 1;
            in_hazard_now = true;
            clear_path(&mut self.state.path_state);
            let exit = self.state.last_safe_origin.or_else(|| self.hazard_exit(world, &bot));
            if let Some(exit) = exit {
                self.state.aim.yaw = (exit.y - bot.origin.y).atan2(exit.x - bot.origin.x).to_degrees();
                self.state.aim.pitch = 0.0;
                hazard_face = Some(exit);
                let (forwardmove, sidemove) = steer_direct(
                    bot.origin,
                    self.state.aim.yaw,
                    exit,
                    false,
                    self.config.run_speed,
                    self.config.walk_speed,
                );
                cmd.forwardmove = forwardmove;
                cmd.sidemove = sidemove;
                move_target = Some(exit);
            }
            cmd.upmove = if bot.water_level >= 3 {
                self.config.run_speed
            } else {
                0.0
            };
            cmd.buttons &= !BOT_BUTTON_JUMP;
            self.state.wedge_origin = Some(bot.origin);
            self.state.wedge_since = now;
        } else if bot.on_ground && bot.water_level == 0 {
            self.state.last_safe_origin = Some(bot.origin);
        }
        if target.is_some()
            && roll_combat_jump(
                &mut self.state.path_state,
                &self.settings.movement,
                &mut self.rng,
                now,
                bot.on_ground,
            )
        {
            cmd.buttons |= BOT_BUTTON_JUMP;
        }
        if wedged {
            if self.state.unstick_until <= now {
                self.state.unstick_until = now + UNSTICK_SECONDS;
                self.state.unstick_side = self.pick_unstick_side(world, &bot);
                clear_path(&mut self.state.path_state);
            }
            if now - self.state.wedge_since >= WEDGED_GIVE_UP_SECONDS {
                if self.state.goal_entity_id >= 0 {
                    let rest = self.unreachable_rest();
                    self.state
                        .unreachable_until
                        .insert(self.state.goal_entity_id, now + rest);
                }
                self.abandon_goal();
                self.state.press_until = now + INTERACT_SECONDS;
                self.state.wedge_origin = Some(bot.origin);
                self.state.wedge_since = now;
            }
        }
        if now < self.state.unstick_until {
            cmd.sidemove = self.state.unstick_side * self.config.run_speed;
            cmd.forwardmove *= 0.5;
            if bot.on_ground {
                cmd.buttons |= BOT_BUTTON_JUMP;
            }
        }
        self.state.last_guard_refused = false;
        if !in_hazard_now && bot.on_ground && (cmd.forwardmove != 0.0 || cmd.sidemove != 0.0) {
            let link = self
                .state
                .path_state
                .path
                .as_ref()
                .and_then(|path| path.links.get(self.state.path_state.index).and_then(|link| *link));
            let planned_jump = link.is_some_and(|link| {
                link.link_type == NavLinkType::LongJump || link.link_type == NavLinkType::ManualLongJump
            });
            let mut gap_jump_pressed = false;
            if !planned_jump {
                let mut gap = self.gap_ahead(world, &bot, &cmd, move_target);
                if gap.is_some_and(|gap| gap.1) && now < self.state.unstick_until {
                    gap = gap.map(|(hazard_at, _)| (hazard_at, false));
                }
                if gap.is_some_and(|gap| !gap.1) && now < self.state.unstick_until {
                    cmd.sidemove = -cmd.sidemove;
                    self.state.unstick_side = -self.state.unstick_side;
                    gap = self.gap_ahead(world, &bot, &cmd, move_target);
                    if gap.is_some_and(|gap| !gap.1) {
                        cmd.sidemove = 0.0;
                        cmd.forwardmove = -self.config.run_speed;
                        cmd.buttons &= !BOT_BUTTON_JUMP;
                        gap = self.gap_ahead(world, &bot, &cmd, move_target);
                    }
                }
                if let Some((hazard_at, crossable)) = gap {
                    if crossable {
                        if hazard_at <= GAP_JUMP_TRIGGER {
                            cmd.buttons |= BOT_BUTTON_JUMP;
                            self.state.gap_jumps += 1;
                            gap_jump_pressed = true;
                        }
                    } else {
                        self.state.guard_refusals += 1;
                        self.state.last_guard_refused = true;
                        cmd.forwardmove = 0.0;
                        cmd.sidemove = 0.0;
                        cmd.buttons &= !BOT_BUTTON_JUMP;
                        if self.state.path_state.path.is_none()
                            && self.state.roam_point.is_some()
                            && self.state.goal_entity_id < 0
                            && !self.state.touch_goal
                            && !self.state.hold_position
                        {
                            self.abandon_goal();
                        }
                    }
                }
            }
            if cmd.buttons & BOT_BUTTON_JUMP != 0
                && !planned_jump
                && !gap_jump_pressed
                && self.jump_lands_in_hazard(world, &bot, &cmd)
            {
                self.state.guard_refusals += 1;
                cmd.buttons &= !BOT_BUTTON_JUMP;
            }
        }
        if self.state.hold_position {
            cmd.forwardmove = 0.0;
            cmd.sidemove = 0.0;
            cmd.upmove = 0.0;
        }
        if bot.on_lift == Some(true) && !riding_lift && cmd.forwardmove == 0.0 && cmd.sidemove == 0.0 {
            cmd.forwardmove = self.config.walk_speed;
        }
        let mut aim_at: Option<Vec3> = None;
        if hazard_face.is_some() {
            // Yaw already held on the exit.
        } else if let Some(target) = target.as_ref() {
            let awareness = self.state.awareness.get(&target.id).copied();
            let point = self.aim_point_for(target, bot.origin, bot.current_weapon);
            let mut lead = aim_lead_point(point, target.velocity, &self.settings.aiming);
            let weapon = self.config.knowledge.weapon_by_number(bot.current_weapon);
            if self.settings.aiming.lead_targets {
                if let Some(weapon) = weapon {
                    if weapon.entry.speed > 0.0 {
                        lead = bvec_ma(
                            lead,
                            bvec_distance(bot.eye, point) / weapon.entry.speed as f32,
                            target.velocity,
                        );
                    }
                }
            }
            aim_at = Some(bvec_sub(lead, bot.eye));
            if let Some(awareness) = awareness {
                if awareness.last_seen < now {
                    aim_at = Some(bvec_sub(awareness.last_known_origin, bot.eye));
                }
            }
        } else if let Some(gate) = self.state.gate_shoot_at {
            aim_at = Some(bvec_sub(gate, bot.eye));
        } else if self.should_check_six(now) {
            let yaw = (self.state.aim.yaw + 180.0).to_radians();
            aim_at = Some(Vec3 {
                x: yaw.cos(),
                y: yaw.sin(),
                z: 0.0,
            });
        } else if let Some(move_target) = move_target {
            let mut dir = bvec_sub(move_target, bot.origin);
            dir.z = 0.0;
            aim_at = Some(dir);
        }
        if let Some(aim_at) = aim_at {
            aim_step(&mut self.state.aim, aim_at, &self.settings.aiming, dt, now);
        }
        cmd.view_angles = Vec3 {
            x: self.state.aim.pitch,
            y: angle_mod(self.state.aim.yaw),
            z: 0.0,
        };
        if self.state.gate_shoot_at.is_some() && target.is_none() && now - self.state.gate_fired_at >= GATE_SHOT_SECONDS
        {
            let gate = self.state.gate_shoot_at.expect("gate checked");
            if angle_between(self.state.aim.pitch, self.state.aim.yaw, bvec_sub(gate, bot.eye)) <= GATE_AIM_DEGREES {
                cmd.buttons |= BOT_BUTTON_ATTACK;
                self.state.gate_fired_at = now;
            }
        }
        if let Some(target) = target.as_ref() {
            let pick = self.select_weapon(&bot, target);
            if let Some(pick) = pick {
                if pick != bot.current_weapon && pick != self.state.last_weapon_number {
                    let impulse = self.config.weapon_impulse.as_ref().map(|map| map(pick)).unwrap_or(0);
                    if impulse > 0 {
                        cmd.impulse = impulse;
                        self.state.last_weapon_number = pick;
                    } else if self.config.on_weapon_select.is_some() {
                        if let Some(select) = self.config.on_weapon_select.as_mut() {
                            select(pick);
                        }
                        self.state.last_weapon_number = pick;
                    }
                } else if pick == bot.current_weapon {
                    self.state.last_weapon_number = 0;
                }
            }
            let awareness = self.state.awareness.get(&target.id).copied();
            let aimed = awareness.is_some_and(|awareness| {
                can_fire(&awareness)
                    && aim_at.is_some_and(|aim_at| {
                        aim_error(&self.state.aim, aim_at) < self.weapon_cone(bot.current_weapon) / 2.0
                    })
            });
            if self.trigger(bot.current_weapon, aimed, now) {
                cmd.buttons |= BOT_BUTTON_ATTACK;
            }
        } else {
            self.state.trigger_held_since = -1.0;
        }
        if self.wants_use(&entities, bot.origin) {
            cmd.buttons |= BOT_BUTTON_USE;
        }
        self.state.last_cmd = cmd;
        cmd
    }

    fn update_senses(
        &mut self,
        world: &dyn BotWorldT,
        bot: &BotSelfT,
        entities: &[BotEntityT],
        sounds: &[crate::behavior::rerelease::world::BotSoundT],
        dt: f32,
        now: f32,
    ) {
        let senses = self.settings.senses;
        let mut weapons = self.settings.weapons;
        weapons.fov_angle = self.weapon_cone(bot.current_weapon);
        for entity in entities {
            if entity.id == bot.id {
                continue;
            }
            if entity.kind != BotEntityKind::PLAYER && entity.kind != BotEntityKind::MONSTER {
                continue;
            }
            if entity.dead {
                self.state.awareness.remove(&entity.id);
                continue;
            }
            let geom = evaluate_sight_geometry(
                bot.eye,
                self.state.aim.pitch,
                self.state.aim.yaw,
                entity.center,
                entity.invisible,
                &senses,
                &weapons,
            );
            let clear = geom.within_invis_range && world.trace_line(bot.eye, entity.center).fraction >= 1.0;
            let mut audible = false;
            for sound in sounds {
                if sound.source_id != entity.id {
                    continue;
                }
                if sound_audible(sound.origin, sound.time, sound.loudness, bot.origin, &senses, now) {
                    audible = true;
                    break;
                }
            }
            let contact = BotContactT {
                line_of_sight: clear,
                in_sight_fov: geom.in_sight_fov,
                in_weapon_fov: geom.in_weapon_fov,
                audible,
                invisible: entity.invisible,
                distance: geom.distance,
                origin: entity.center,
            };
            let awareness = self
                .state
                .awareness
                .entry(entity.id)
                .or_insert_with(|| new_awareness(entity.id, now, entity.center));
            sense_step(awareness, &contact, &senses, &weapons, dt, now);
        }
        let stale: Vec<i32> = self
            .state
            .awareness
            .iter()
            .filter(|(_, awareness)| should_forget(awareness, &senses, now))
            .map(|(id, _)| *id)
            .collect();
        for id in stale {
            self.state.awareness.remove(&id);
        }
    }

    fn select_target(&mut self, entities: &[BotEntityT], team: i32, self_origin: Vec3) -> Option<BotEntityT> {
        if !self.settings.behaviors.allow_combat {
            for entity in entities {
                if let Some(awareness) = self.state.awareness.get(&entity.id) {
                    if is_aware(awareness) && !self.friendly(entity, team) {
                        return Some(entity.clone());
                    }
                }
            }
            return None;
        }
        let mut best: Option<BotEntityT> = None;
        let mut best_score = f32::NEG_INFINITY;
        for entity in entities {
            let awareness = match self.state.awareness.get(&entity.id) {
                Some(awareness) if is_aware(awareness) => *awareness,
                _ => continue,
            };
            if self.friendly(entity, team) {
                continue;
            }
            let mut score = 4096.0 - bvec_distance(self_origin, awareness.last_known_origin);
            if entity.kind == BotEntityKind::PLAYER {
                score += 512.0;
            }
            if entity.carrying_objective {
                score += 1536.0;
            }
            score += awareness.weapon * 256.0;
            if entity.id == self.state.target_id {
                score += 128.0;
            }
            if score > best_score {
                best_score = score;
                best = Some(entity.clone());
            }
        }
        best
    }

    fn team_game(&self) -> bool {
        let game_type = self.config.game_mode.game_type.as_str();
        self.config.game_mode.has_teams.unwrap_or_else(|| {
            game_type == BotGameType::TEAM_DEATHMATCH || game_type == BotGameType::CTF || self.coop_game()
        })
    }

    fn coop_game(&self) -> bool {
        let game_type = self.config.game_mode.game_type.as_str();
        game_type == BotGameType::COOP || game_type == BotGameType::HORDE
    }

    fn friendly(&self, entity: &BotEntityT, team: i32) -> bool {
        if entity.kind == BotEntityKind::MONSTER {
            return self.config.knowledge.monster(&entity.classname).is_none();
        }
        if entity.kind != BotEntityKind::PLAYER {
            return true;
        }
        if !self.team_game() {
            return false;
        }
        if team <= 0 || entity.team <= 0 {
            return false;
        }
        entity.team == team
    }

    fn update_wedge(&mut self, origin: Vec3, now: f32, in_combat: bool) -> bool {
        let cmd = self.state.last_cmd;
        let pressing = cmd.forwardmove != 0.0 || cmd.sidemove != 0.0 || self.state.last_guard_refused;
        if in_combat
            || self.state.wedge_origin.is_none()
            || !pressing
            || self
                .state
                .wedge_origin
                .is_some_and(|wedge| bvec_distance(origin, wedge) > WEDGED_DISPLACEMENT)
        {
            self.state.wedge_origin = Some(origin);
            self.state.wedge_since = now;
            return false;
        }
        now - self.state.wedge_since >= WEDGED_SECONDS
    }

    fn update_objectives(&mut self, entities: &[BotEntityT], bot: &BotSelfT) {
        for entity in entities {
            if entity.kind != BotEntityKind::ITEM {
                continue;
            }
            let item = match self.config.knowledge.item(&entity.classname) {
                Some(item) if item.entry.flags.iter().any(|flag| flag == ItemFlag::OBJECTIVE) => item.clone(),
                _ => continue,
            };
            self.state.objective_home.entry(entity.id).or_insert(entity.origin);
            let team = item.entry.team.map(|team| team as i32).unwrap_or(entity.team);
            if team <= 0 || bot.team <= 0 {
                continue;
            }
            let home = self.state.objective_home.get(&entity.id).copied();
            match home {
                Some(home) if team == bot.team => self.state.own_objective_home = Some(home),
                Some(home) => self.state.enemy_objective_home = Some(home),
                None => {}
            }
        }
        if self.state.objective_role.is_empty()
            && self.state.own_objective_home.is_some()
            && self.state.enemy_objective_home.is_some()
        {
            self.state.objective_role = if random_chance(&mut self.rng, 25.0) {
                "defend".to_owned()
            } else {
                "attack".to_owned()
            };
            let chat = if self.state.objective_role == "defend" {
                "ctf_on_defense"
            } else {
                "ctf_on_offense"
            };
            self.emit_chat_once(chat);
        }
    }

    fn objective_goal(
        &mut self,
        entities: &[BotEntityT],
        bot: &BotSelfT,
        target: Option<&BotEntityT>,
        now: f32,
    ) -> Option<Vec3> {
        if self.state.own_objective_home.is_none() && self.state.enemy_objective_home.is_none() {
            return None;
        }
        if bot.carrying_objective {
            if let Some(home) = self.state.own_objective_home {
                self.emit_chat_once("ctf_delivering_flag");
                self.state.goal_entity_id = -1;
                for entity in entities {
                    if entity.kind != BotEntityKind::ITEM {
                        continue;
                    }
                    let item = match self.config.knowledge.item(&entity.classname) {
                        Some(item) if item.entry.flags.iter().any(|flag| flag == ItemFlag::OBJECTIVE) => item.clone(),
                        _ => continue,
                    };
                    if item.entry.team.map(|team| team as i32).unwrap_or(entity.team) != bot.team {
                        continue;
                    }
                    if bvec_distance(entity.origin, home) > OBJECTIVE_AWAY {
                        continue;
                    }
                    self.state.touch_goal = true;
                    return Some(entity.origin);
                }
                if bvec_distance(bot.origin, home) < CARRIER_HOLD_RADIUS {
                    self.state.hold_position = true;
                }
                return Some(home);
            }
        }
        let mut enemy_flag: Option<BotEntityT> = None;
        for entity in entities {
            if entity.kind != BotEntityKind::ITEM {
                continue;
            }
            let item = match self.config.knowledge.item(&entity.classname) {
                Some(item) if item.entry.flags.iter().any(|flag| flag == ItemFlag::OBJECTIVE) => item.clone(),
                _ => continue,
            };
            let team = item.entry.team.map(|team| team as i32).unwrap_or(entity.team);
            if team <= 0 || bot.team <= 0 {
                continue;
            }
            if team == bot.team {
                if let Some(home) = self.state.objective_home.get(&entity.id) {
                    if bvec_distance(entity.origin, *home) > OBJECTIVE_AWAY {
                        self.emit_chat_once("ctf_returning_dropped_flag");
                        self.state.goal_entity_id = entity.id;
                        return Some(entity.origin);
                    }
                }
                continue;
            }
            enemy_flag = Some(entity.clone());
        }
        let mut our_carrier: Option<BotEntityT> = None;
        let mut enemy_carrier: Option<BotEntityT> = None;
        for entity in entities {
            if entity.kind != BotEntityKind::PLAYER || !entity.carrying_objective || entity.id == bot.id || entity.dead
            {
                continue;
            }
            if entity.team != bot.team {
                enemy_carrier = Some(entity.clone());
            } else {
                our_carrier = Some(entity.clone());
            }
        }
        if self.state.objective_role == "defend" {
            if target.is_some() {
                return None;
            }
            if let Some(carrier) = enemy_carrier {
                self.emit_chat_once("ctf_attacking_enemy_carrier");
                self.state.goal_entity_id = carrier.id;
                self.state.goal_is_live = true;
                return Some(carrier.origin);
            }
            let home = self.state.own_objective_home?;
            if bvec_distance(bot.origin, home) < OBJECTIVE_GUARD_RADIUS {
                return None;
            }
            self.state.goal_entity_id = -1;
            return Some(home);
        }
        if let Some(target) = target {
            if target.kind == BotEntityKind::PLAYER && target.team > 0 && target.team != bot.team {
                self.emit_chat_once("ctf_attacking_enemy_carrier");
            }
        }
        if let Some(flag) = enemy_flag {
            self.state.goal_entity_id = flag.id;
            return Some(flag.origin);
        }
        if let Some(carrier) = our_carrier {
            let carrier_home = self
                .state
                .own_objective_home
                .is_some_and(|home| bvec_distance(carrier.origin, home) < CARRIER_HOLD_RADIUS);
            if carrier_home {
                if let Some(enemy) = enemy_carrier {
                    self.emit_chat_once("ctf_attacking_enemy_carrier");
                    self.state.goal_entity_id = enemy.id;
                    self.state.goal_is_live = true;
                    return Some(enemy.origin);
                }
                return None;
            }
            self.state.goal_entity_id = carrier.id;
            self.state.goal_is_live = true;
            if bvec_distance(bot.origin, carrier.origin) < ESCORT_DISTANCE {
                return None;
            }
            return Some(carrier.origin);
        }
        let home = self.state.enemy_objective_home?;
        if self
            .state
            .rest_point
            .is_some_and(|rest| bvec_distance(rest, home) < 1.0)
            && now < self.state.rest_until
        {
            return None;
        }
        self.state.goal_entity_id = -1;
        Some(home)
    }

    fn gate_to_shoot(
        &self,
        world: &dyn BotWorldT,
        entities: &[BotEntityT],
        bot: &BotSelfT,
        bounds: Option<(Vec3, Vec3)>,
    ) -> Option<Vec3> {
        let (mins, maxs) = bounds?;
        for entity in entities {
            if entity.kind != BotEntityKind::INTERACTABLE {
                continue;
            }
            let center = entity.center;
            if center.x < mins.x - 8.0
                || center.x > maxs.x + 8.0
                || center.y < mins.y - 8.0
                || center.y > maxs.y + 8.0
                || center.z < mins.z - 8.0
                || center.z > maxs.z + 8.0
            {
                continue;
            }
            if self.config.knowledge.interaction_for(
                &entity.classname,
                entity.spawnflags,
                entity.has_health,
                entity.has_targetname,
            ) != Some(Interaction::SHOOT)
            {
                return None;
            }
            if bvec_distance(bot.eye, center) > GATE_SHOOT_RANGE {
                return None;
            }
            let trace = world.trace_line(bot.eye, center);
            if trace.fraction < 1.0 && trace.hit_id != entity.id {
                continue;
            }
            return Some(center);
        }
        None
    }

    fn pick_unstick_side(&mut self, world: &dyn BotWorldT, bot: &BotSelfT) -> f32 {
        let first = if random_chance(&mut self.rng, 50.0) { 1.0 } else { -1.0 };
        let (_, right, _) = angle_vectors(0.0, self.state.aim.yaw, 0.0);
        let mut drop_sides = 0;
        for side in [first, -first] {
            let at = Vec3 {
                x: bot.origin.x + right.x * side * UNSTICK_STEP,
                y: bot.origin.y + right.y * side * UNSTICK_STEP,
                z: bot.origin.z + 8.0,
            };
            let down = world.trace_box(
                at,
                Vec3 {
                    x: -16.0,
                    y: -16.0,
                    z: -24.0,
                },
                Vec3 {
                    x: 16.0,
                    y: 16.0,
                    z: 32.0,
                },
                Vec3 {
                    x: at.x,
                    y: at.y,
                    z: at.z - UNSTICK_MAX_DROP,
                },
            );
            if down.startsolid {
                continue;
            }
            if down.fraction < 1.0 {
                return side;
            }
            drop_sides += 1;
        }
        if drop_sides == 2 {
            first
        } else {
            0.0
        }
    }

    fn rest_static_goal(&mut self, goal: Vec3, now: f32) {
        if self.state.goal_entity_id >= 0 || self.state.touch_goal || self.state.hold_position {
            return;
        }
        self.state.rest_point = Some(goal);
        self.state.rest_until = now + POINT_REST_SECONDS;
    }

    fn hazard_under_segment(&self, world: &dyn BotWorldT, a: Vec3, b: Vec3) -> bool {
        let len = bvec_distance(a, b);
        let steps = ((len / 64.0).ceil() as usize).max(1);
        for i in 1..steps {
            let f = i as f32 / steps as f32;
            if hazard_below(
                world,
                a.x + (b.x - a.x) * f,
                a.y + (b.y - a.y) * f,
                a.z + (b.z - a.z) * f,
            ) {
                return true;
            }
        }
        false
    }

    fn in_hazard(&self, world: &dyn BotWorldT, bot: &BotSelfT) -> bool {
        let bad = |contents: i32| contents == BotContents::LAVA || contents == BotContents::SLIME;
        bad(world.point_contents(bot.origin))
            || bad(world.point_contents(Vec3 {
                x: bot.origin.x,
                y: bot.origin.y,
                z: bot.origin.z - 20.0,
            }))
    }

    fn hazard_exit(&self, world: &dyn BotWorldT, bot: &BotSelfT) -> Option<Vec3> {
        for i in 0..LAVA_EXIT_SAMPLES {
            let angle = (i as f32 / LAVA_EXIT_SAMPLES as f32) * std::f32::consts::TAU;
            let point = Vec3 {
                x: bot.origin.x + angle.cos() * LAVA_EXIT_REACH,
                y: bot.origin.y + angle.sin() * LAVA_EXIT_REACH,
                z: bot.origin.z,
            };
            let contents = world.point_contents(point);
            if matches!(contents, c if c == BotContents::LAVA || c == BotContents::SLIME || c == BotContents::SOLID) {
                continue;
            }
            let below = world.point_contents(Vec3 {
                x: point.x,
                y: point.y,
                z: point.z - 24.0,
            });
            if below == BotContents::LAVA || below == BotContents::SLIME {
                continue;
            }
            return Some(point);
        }
        None
    }

    fn gap_ahead(
        &self,
        world: &dyn BotWorldT,
        bot: &BotSelfT,
        cmd: &BotUsercmdT,
        steer_at: Option<Vec3>,
    ) -> Option<(f32, bool)> {
        let (mx, my) = self.move_direction(cmd)?;
        let speed = (bot.velocity.x * bot.velocity.x + bot.velocity.y * bot.velocity.y).sqrt();
        let mut ahead = EDGE_LOOKAHEAD + speed * EDGE_STOP_SECONDS;
        if let Some(steer_at) = steer_at {
            let along = (steer_at.x - bot.origin.x) * mx + (steer_at.y - bot.origin.y) * my;
            if along > EDGE_PROBE_STEP && along < ahead {
                ahead = along;
            }
        }
        let mut hazard_at = -1.0f32;
        let mut d = EDGE_PROBE_STEP;
        while d <= ahead + 0.01 {
            if hazard_below(world, bot.origin.x + mx * d, bot.origin.y + my * d, bot.origin.z) {
                hazard_at = d;
                break;
            }
            d += EDGE_PROBE_STEP;
        }
        if hazard_at < 0.0 {
            return None;
        }
        let floor_z = bot.origin.z - 24.0;
        let cap = if self.settings.movement.walk_only {
            self.config.walk_speed
        } else {
            self.config.run_speed
        };
        let run_speed = speed.max(cap);
        let mut landing_start = -1.0f32;
        let mut landing_z = 0.0f32;
        let mut d = hazard_at + EDGE_PROBE_STEP;
        while d <= hazard_at + GAP_LANDING_SEARCH {
            let (x, y) = (bot.origin.x + mx * d, bot.origin.y + my * d);
            let trace = world.trace_line(
                Vec3 { x, y, z: bot.origin.z },
                Vec3 {
                    x,
                    y,
                    z: bot.origin.z - EDGE_DROP_CHECK,
                },
            );
            if trace.startsolid {
                return Some((hazard_at, false));
            }
            let land_z = trace.endpos.z;
            let contents = if trace.fraction < 1.0 {
                world.point_contents(Vec3 { x, y, z: land_z + 2.0 })
            } else {
                BotContents::LAVA
            };
            let floor_here = trace.fraction < 1.0 && contents != BotContents::LAVA && contents != BotContents::SLIME;
            if !floor_here {
                landing_start = -1.0;
                d += EDGE_PROBE_STEP;
                continue;
            }
            if land_z > floor_z + self.config.movement.maximum_landing_rise {
                return Some((hazard_at, false));
            }
            if landing_start < 0.0 || (land_z - landing_z).abs() > 8.0 {
                landing_start = d;
                landing_z = land_z;
                d += EDGE_PROBE_STEP;
                continue;
            }
            let drop = (floor_z - landing_z).max(0.0);
            let jump_velocity = self.config.movement.jump_velocity;
            let gravity = self.config.movement.gravity;
            let airtime = (jump_velocity + (jump_velocity * jump_velocity + 2.0 * gravity * drop).sqrt()) / gravity;
            let reach = run_speed * airtime * GAP_REACH_MARGIN;
            return Some((hazard_at, landing_start + 16.0 <= reach));
        }
        Some((hazard_at, false))
    }

    fn move_direction(&self, cmd: &BotUsercmdT) -> Option<(f32, f32)> {
        let yaw = self.state.aim.yaw.to_radians();
        let (fx, fy) = (yaw.cos(), yaw.sin());
        let (rx, ry) = (yaw.sin(), -yaw.cos());
        let mx = fx * cmd.forwardmove + rx * cmd.sidemove;
        let my = fy * cmd.forwardmove + ry * cmd.sidemove;
        let length = (mx * mx + my * my).sqrt();
        if length < 1.0 {
            return None;
        }
        Some((mx / length, my / length))
    }

    fn jump_lands_in_hazard(&self, world: &dyn BotWorldT, bot: &BotSelfT, cmd: &BotUsercmdT) -> bool {
        let (mut vx, mut vy) = (bot.velocity.x, bot.velocity.y);
        let mut speed = (vx * vx + vy * vy).sqrt();
        if speed < 1.0 {
            let Some((dx, dy)) = self.move_direction(cmd) else {
                return false;
            };
            speed = self.config.run_speed;
            vx = dx * speed;
            vy = dy * speed;
        }
        let reach = speed * self.config.movement.jump_air_seconds;
        let (ux, uy) = (vx / speed, vy / speed);
        let mut d = EDGE_PROBE_STEP;
        while d <= reach + 0.01 {
            if hazard_below(world, bot.origin.x + ux * d, bot.origin.y + uy * d, bot.origin.z) {
                return true;
            }
            d += EDGE_PROBE_STEP;
        }
        false
    }

    fn air_above(&self, world: &dyn BotWorldT, bot: &BotSelfT) -> bool {
        world.point_contents(Vec3 {
            x: bot.eye.x,
            y: bot.eye.y,
            z: bot.eye.z + SURFACE_REACH,
        }) == BotContents::EMPTY
    }

    fn interactable_goal(&mut self, entities: &[BotEntityT], bot: &BotSelfT, now: f32) -> Option<Vec3> {
        if now >= self.state.press_until {
            return None;
        }
        if self.state.own_objective_home.is_some() || self.state.enemy_objective_home.is_some() {
            return None;
        }
        let mut best: Option<BotEntityT> = None;
        let mut best_range = f32::INFINITY;
        for entity in entities {
            if entity.kind != BotEntityKind::INTERACTABLE {
                continue;
            }
            if self.config.knowledge.interaction_for(
                &entity.classname,
                entity.spawnflags,
                entity.has_health,
                entity.has_targetname,
            ) != Some("push")
            {
                continue;
            }
            if entity.has_targetname {
                continue;
            }
            if let Some(blocked) = self.state.unreachable_until.get(&entity.id).copied() {
                if blocked > now {
                    continue;
                }
                self.state.unreachable_until.remove(&entity.id);
            }
            let range = bvec_distance(bot.origin, entity.center);
            if range > INTERACT_RADIUS || range >= best_range {
                continue;
            }
            best_range = range;
            best = Some(entity.clone());
        }
        let best = best?;
        if best_range <= INTERACT_REACHED {
            self.state.press_until = 0.0;
            return None;
        }
        self.state.goal_entity_id = best.id;
        Some(best.center)
    }

    fn coop_regroup_goal(&mut self, entities: &[BotEntityT], bot: &BotSelfT, now: f32) -> Option<Vec3> {
        if !self.coop_game() {
            return None;
        }
        let mut nearest: Option<BotEntityT> = None;
        let mut best = f32::INFINITY;
        for entity in entities {
            if entity.kind != BotEntityKind::PLAYER || entity.is_bot || entity.dead {
                continue;
            }
            let d = bvec_distance(bot.origin, entity.origin);
            if d < best {
                best = d;
                nearest = Some(entity.clone());
            }
        }
        let nearest = match nearest {
            Some(nearest) => nearest,
            None => {
                self.state.coop_regrouping = false;
                return None;
            }
        };
        if !self.state.coop_regrouping && now >= self.state.coop_regroup_at {
            self.state.coop_regrouping = true;
            self.state.coop_regroup_until = now + COOP_REGROUP_GIVE_UP;
        }
        if self.state.coop_regrouping && (best <= COOP_REGROUP_NEAR || now >= self.state.coop_regroup_until) {
            self.state.coop_regrouping = false;
            self.state.coop_regroup_at = now + COOP_REGROUP_SECONDS;
        }
        if !self.state.coop_regrouping {
            return None;
        }
        self.state.goal_entity_id = -1;
        Some(nearest.origin)
    }

    fn coop_hunt_goal(&mut self, entities: &[BotEntityT], bot: &BotSelfT, now: f32) -> Option<Vec3> {
        if !self.coop_game() {
            return None;
        }
        let mut human: Option<BotEntityT> = None;
        let mut human_range = f32::INFINITY;
        for entity in entities {
            if entity.kind != BotEntityKind::PLAYER || entity.is_bot || entity.dead {
                continue;
            }
            let d = bvec_distance(bot.origin, entity.origin);
            if d < human_range {
                human_range = d;
                human = Some(entity.clone());
            }
        }
        let mut best: Option<BotEntityT> = None;
        let mut best_range = f32::INFINITY;
        for entity in entities {
            if entity.kind != BotEntityKind::MONSTER || entity.dead {
                continue;
            }
            if self.friendly(entity, bot.team) {
                continue;
            }
            if let Some(blocked) = self.state.unreachable_until.get(&entity.id).copied() {
                if blocked > now {
                    continue;
                }
                self.state.unreachable_until.remove(&entity.id);
            }
            if let Some(human) = human.as_ref() {
                if bvec_distance(human.origin, entity.origin) > COOP_HUNT_RADIUS {
                    continue;
                }
            }
            let d = bvec_distance(bot.origin, entity.origin);
            if d < best_range {
                best_range = d;
                best = Some(entity.clone());
            }
        }
        let best = best?;
        self.state.goal_entity_id = best.id;
        self.state.goal_is_live = true;
        Some(best.origin)
    }

    fn select_goal(
        &mut self,
        world: &mut dyn BotWorldT,
        entities: &[BotEntityT],
        target: Option<&BotEntityT>,
        now: f32,
    ) -> Option<Vec3> {
        self.state.goal_entity_id = -1;
        self.state.goal_is_live = false;
        self.state.touch_goal = false;
        self.state.hold_position = false;
        if let Some(goal) = self.state.explicit_goal {
            if !self.state.explicit_goal_done && !self.state.explicit_goal_failed {
                if goal.kind == ExplicitGoalKind::Entity {
                    if let Some(entity) = entities.iter().find(|entity| entity.id == goal.entity_id) {
                        if let Some(explicit) = self.state.explicit_goal.as_mut() {
                            explicit.point = entity.origin;
                            self.state.goal_point = Some(explicit.point);
                            return self.state.goal_point;
                        }
                    } else {
                        self.state.explicit_goal_failed = true;
                    }
                } else {
                    self.state.goal_point = Some(goal.point);
                    return self.state.goal_point;
                }
            }
        }
        let bot = world.bot_self();
        if let Some(objective) = self.objective_goal(entities, &bot, target, now) {
            self.state.goal_point = Some(objective);
            return self.state.goal_point;
        }
        if let Some(target) = target {
            let point = self
                .state
                .awareness
                .get(&target.id)
                .map(|awareness| awareness.last_known_origin)
                .unwrap_or(target.origin);
            self.state.goal_point = Some(point);
            if self.settings.behaviors.allow_grab_items_in_combat {
                if let Some(item) = self.best_item(world, entities, now, true) {
                    if bvec_distance(bot.origin, item.origin) < self.settings.behaviors.combat_max_item_dist {
                        self.state.goal_entity_id = item.id;
                        return Some(item.origin);
                    }
                }
            }
            return self.state.goal_point;
        }
        if let Some(unblock) = self.interactable_goal(entities, &bot, now) {
            self.state.goal_point = Some(unblock);
            return self.state.goal_point;
        }
        if let Some(regroup) = self.coop_regroup_goal(entities, &bot, now) {
            self.state.goal_point = Some(regroup);
            return self.state.goal_point;
        }
        if let Some(hunt) = self.coop_hunt_goal(entities, &bot, now) {
            self.state.goal_point = Some(hunt);
            return self.state.goal_point;
        }
        if self.settings.behaviors.allow_grab_items {
            if let Some(item) = self.best_item(world, entities, now, false) {
                self.state.goal_entity_id = item.id;
                self.state.goal_point = Some(item.origin);
                return self.state.goal_point;
            }
        }
        self.roam_goal(world, now)
    }

    fn best_item(
        &mut self,
        world: &mut dyn BotWorldT,
        entities: &[BotEntityT],
        now: f32,
        in_combat: bool,
    ) -> Option<BotEntityT> {
        let bot = world.bot_self();
        let defer_power = self.settings.behaviors.defer_power_items_to_humans && (self.config.human_teammate_near)();
        let mut best: Option<BotEntityT> = None;
        let mut best_score = 0.0f32;
        for entity in entities {
            if entity.kind != BotEntityKind::ITEM {
                continue;
            }
            if let Some(blocked) = self.state.unreachable_until.get(&entity.id).copied() {
                if blocked > now {
                    continue;
                }
                self.state.unreachable_until.remove(&entity.id);
            }
            let item = match self.config.knowledge.item(&entity.classname) {
                Some(item) => item.clone(),
                None => continue,
            };
            if f64::from(bvec_distance(bot.origin, entity.origin)) > item.entry.sight_dist {
                continue;
            }
            if in_combat {
                let behaviors = self.settings.behaviors;
                if item.is_weapon && !behaviors.combat_grab_weapons {
                    continue;
                }
                if item.is_health && bot.health >= self.config.max_health * behaviors.combat_min_health_pct / 100.0 {
                    continue;
                }
                if item.is_armor && bot.armor >= bot.max_armor.unwrap_or(200.0) * behaviors.combat_min_armor_pct / 100.0
                {
                    continue;
                }
                if item.is_ammo {
                    if let Some(weapon) = self.config.knowledge.weapon_by_number(bot.current_weapon) {
                        let have = bot.ammo.get(&weapon.entry.ammo_name).copied().unwrap_or(0) as f64;
                        if have >= weapon.entry.max_ammo * f64::from(behaviors.combat_min_ammo_pct) / 100.0 {
                            continue;
                        }
                    }
                }
            }
            if (item.is_powerup || item.is_mega) && defer_power {
                continue;
            }
            let home = self.state.objective_home.get(&entity.id).copied();
            let ctx = BotItemContextT {
                spawnflags: entity.spawnflags,
                health: bot.health,
                max_health: self.config.max_health,
                armor: bot.armor,
                items: bot.items,
                ammo: bot.ammo.clone(),
                weapon_stay: self.config.game_mode.weapon_stay,
                allow_power_items: self.settings.behaviors.allow_grab_power_items,
                team: bot.team,
                item_team: item.entry.team.map(|team| team as i32).unwrap_or(entity.team),
                objective_at_home: home.is_none_or(|home| bvec_distance(entity.origin, home) <= OBJECTIVE_AWAY),
            };
            let value = item_value(&item, &ctx, &self.config.knowledge.weapons);
            if value <= 0.0 {
                continue;
            }
            let dist = bvec_distance(bot.origin, entity.origin).max(64.0);
            let score = value * 1024.0 / dist;
            if score > best_score {
                best_score = score;
                best = Some(entity.clone());
            }
        }
        best
    }

    fn roam_goal(&mut self, world: &mut dyn BotWorldT, now: f32) -> Option<Vec3> {
        let bot = world.bot_self();
        let node_count = world.nav().map(|nav| nav.node_count()).unwrap_or(0);
        if node_count == 0 {
            return self.blind_roam_goal(world, now);
        }
        if self
            .state
            .roam_point
            .is_some_and(|roam| bvec_distance(bot.origin, roam) > 64.0)
            && self.state.path_state.path.is_some()
        {
            return self.state.roam_point;
        }
        for _ in 0..8 {
            let index = random_index(&mut self.rng, node_count);
            let node = world.nav().and_then(|nav| nav.nodes().get(index).copied());
            let Some(node) = node else {
                continue;
            };
            if bvec_distance(bot.origin, node.origin) > ROAM_RADIUS {
                continue;
            }
            self.state.roam_point = Some(node.origin);
            self.state.path_state.planned_at = now - REPLAN_SECONDS;
            return self.state.roam_point;
        }
        self.state.roam_point
    }

    fn blind_roam_goal(&mut self, world: &mut dyn BotWorldT, now: f32) -> Option<Vec3> {
        let bot = world.bot_self();
        if self
            .state
            .roam_point
            .is_some_and(|roam| bvec_distance(bot.origin, roam) > 64.0)
            && now < self.state.roam_until
        {
            return self.state.roam_point;
        }
        let angle = random_index(&mut self.rng, 360) as f32 * std::f32::consts::PI / 180.0;
        let reach = BLIND_ROAM_RADIUS / 2.0 + random_index(&mut self.rng, (BLIND_ROAM_RADIUS / 2.0) as usize) as f32;
        self.state.roam_point = Some(Vec3 {
            x: bot.origin.x + angle.cos() * reach,
            y: bot.origin.y + angle.sin() * reach,
            z: bot.origin.z,
        });
        self.state.roam_until = now + BLIND_ROAM_SECONDS;
        self.state.roam_point
    }

    fn unreachable_rest(&self) -> f32 {
        if self.state.goal_is_live {
            UNREACHABLE_LIVE_SECONDS
        } else {
            UNREACHABLE_SECONDS
        }
    }

    fn abandon_goal(&mut self) {
        clear_path(&mut self.state.path_state);
        self.state.roam_point = None;
        self.state.roam_until = 0.0;
        self.state.goal_point = None;
        self.state.goal_entity_id = -1;
        if self.state.explicit_goal.is_some() {
            self.state.explicit_goal_failed = true;
        }
    }

    fn reach_goal(&mut self) {
        clear_path(&mut self.state.path_state);
        self.state.stuck_trips = 0;
        self.state.press_until = 0.0;
        self.state.roam_point = None;
        self.state.roam_until = 0.0;
        if self.state.explicit_goal.is_some() {
            self.state.explicit_goal_done = true;
        }
    }

    fn traverse_caps(&self) -> NavTraverseCapsT {
        let mut caps = default_traverse_caps();
        caps.jump = !self.settings.movement.walk_only || self.settings.movement.allow_jumping_in_combat;
        caps
    }

    fn ensure_path(&mut self, world: &mut dyn BotWorldT, goal: Vec3, now: f32) {
        let bot = world.bot_self();
        let has_nav = world.nav().is_some();
        if !has_nav {
            clear_path(&mut self.state.path_state);
            return;
        }
        let followed = &self.state.path_state;
        let current_link = followed
            .path
            .as_ref()
            .and_then(|path| path.links.get(followed.index).and_then(|link| *link));
        let previous_link = if followed.index > 0 {
            followed
                .path
                .as_ref()
                .and_then(|path| path.links.get(followed.index - 1).and_then(|link| *link))
        } else {
            None
        };
        let train = current_link
            .filter(|link| link.link_type == NavLinkType::Train)
            .or_else(|| previous_link.filter(|link| link.link_type == NavLinkType::Train));
        if let Some(train) = train {
            let step = world.nav().and_then(|nav| nav.transport(&train, bot.origin));
            match step {
                Some(BotTransportStep::Ride) | Some(BotTransportStep::Wait) => return,
                Some(BotTransportStep::Move { approach, .. }) if !approach => return,
                _ => {}
            }
        }
        let stale = self.state.path_state.path.is_none()
            || world.nav().is_some_and(|nav| {
                self.state
                    .path_state
                    .path
                    .as_ref()
                    .is_some_and(|path| !nav.path_valid(path))
            })
            || now - self.state.path_state.planned_at > REPLAN_SECONDS;
        if !stale {
            let end = self
                .state
                .path_state
                .path
                .as_ref()
                .and_then(|path| path.points.last().copied());
            if end.is_none_or(|end| bvec_distance(end, goal) < 96.0) {
                return;
            }
        }
        let mut avoid = Vec::new();
        if let Some(nav) = world.nav() {
            for node in nav.nodes() {
                let contents = world.point_contents(Vec3 {
                    x: node.origin.x,
                    y: node.origin.y,
                    z: node.origin.z + 8.0,
                });
                if contents == BotContents::LAVA || contents == BotContents::SLIME {
                    avoid.push(node.index);
                }
            }
        }
        let caps = NavTraverseCapsT {
            avoid_nodes: avoid,
            ..self.traverse_caps()
        };
        let path = world.nav().and_then(|nav| {
            nav.plan_path(
                bot.origin,
                goal,
                &NavPlanOptions {
                    caps: Some(caps),
                    max_radius: None,
                    start_above: Some(self.config.movement.start_above),
                    ..NavPlanOptions::default()
                },
            )
        });
        // Visibility re-check: refuse corner cuts over hazards.
        let path = match path {
            Some(path) if self.path_crosses_hazard(world, &path) => None,
            other => other,
        };
        set_path(&mut self.state.path_state, path, bot.origin, now);
        if self.state.path_state.path.is_none() {
            self.state.path_state.planned_at = now;
        }
    }

    fn path_crosses_hazard(&self, world: &dyn BotWorldT, path: &NavPathT) -> bool {
        for window in path.points.windows(2) {
            if self.hazard_under_segment(world, window[0], window[1]) {
                return true;
            }
        }
        false
    }

    fn aim_point_for(&self, target: &BotEntityT, from: Vec3, weapon_number: i32) -> Vec3 {
        let weapon = self.config.knowledge.weapon_by_number(weapon_number).cloned();
        let point = weapon
            .as_ref()
            .map_or("center", |weapon| weapon.entry.aim_point.as_str());
        match point {
            "head" => target.head,
            "feet" => target.feet,
            "best" => {
                if let Some(weapon) = weapon.as_ref() {
                    if weapon.entry.flags.iter().any(|flag| flag == "explosive")
                        && (target.origin.z - from.z).abs() < 32.0
                    {
                        return target.feet;
                    }
                }
                let names = self.config.knowledge.skill_names();
                let rank = names.iter().position(|name| *name == self.settings.skill).unwrap_or(0);
                if rank + 2 >= names.len() {
                    return target.head;
                }
                target.center
            }
            _ => target.center,
        }
    }

    fn select_weapon(&self, bot: &BotSelfT, target: &BotEntityT) -> Option<i32> {
        let pick = choose_weapon(
            &self.config.knowledge.weapons,
            &BotWeaponContextT {
                items: bot.items,
                ammo: bot.ammo.clone(),
                range: bvec_distance(bot.origin, target.origin),
                height_delta: target.origin.z - bot.origin.z,
                in_water: bot.water_level >= 2,
                has_protection: bot.has_protection,
                target_in_water: target.water_level >= 2,
                allow_melee: self.settings.behaviors.allow_melee,
            },
        );
        pick.map(|pick| pick.number)
    }

    fn weapon_cone(&self, weapon_number: i32) -> f32 {
        if self.settings.weapons.fov_scalar <= 0.0 {
            return self.settings.weapons.fov_angle;
        }
        let weapon = self.config.knowledge.weapon_by_number(weapon_number);
        let ideal = weapon.map_or(self.settings.senses.fov_angle, |weapon| {
            if weapon.entry.ideal_fov > 0.0 {
                weapon.entry.ideal_fov as f32
            } else {
                self.settings.senses.fov_angle
            }
        });
        ideal * self.settings.weapons.fov_scalar
    }

    fn trigger(&mut self, weapon_number: i32, aimed: bool, now: f32) -> bool {
        let weapon = self.config.knowledge.weapon_by_number(weapon_number).cloned();
        let Some(weapon) = weapon else {
            self.state.trigger_weapon = 0;
            self.state.trigger_held_since = -1.0;
            self.state.trigger_ready_at = 0.0;
            return aimed;
        };
        if weapon.entry.trigger_type == crate::behavior::rerelease::data::botdata::TriggerType::Continuous {
            self.state.trigger_weapon = 0;
            self.state.trigger_held_since = -1.0;
            self.state.trigger_ready_at = 0.0;
            return aimed;
        }
        if self.state.trigger_weapon != weapon_number {
            self.state.trigger_weapon = weapon_number;
            self.state.trigger_held_since = -1.0;
            self.state.trigger_ready_at = 0.0;
        }
        if self.state.trigger_held_since >= 0.0 {
            if aimed && now - self.state.trigger_held_since < weapon.entry.trigger_hold as f32 {
                return true;
            }
            self.state.trigger_held_since = -1.0;
            self.state.trigger_ready_at = now + weapon.entry.trigger_cooldown as f32;
            return false;
        }
        if !aimed || now < self.state.trigger_ready_at {
            return false;
        }
        self.state.trigger_held_since = now;
        true
    }

    fn should_check_six(&mut self, now: f32) -> bool {
        if !self.settings.behaviors.allow_check_six {
            return false;
        }
        if now < self.state.check_six_until {
            return true;
        }
        if now < self.state.check_six_next_at {
            return false;
        }
        self.state.check_six_next_at = now + random_range(&mut self.rng, 3.0, 8.0);
        if !random_chance(&mut self.rng, 35.0) {
            return false;
        }
        self.state.check_six_until = now + 0.5;
        true
    }

    fn wants_use(&self, entities: &[BotEntityT], origin: Vec3) -> bool {
        for entity in entities {
            if entity.kind != BotEntityKind::INTERACTABLE {
                continue;
            }
            if bvec_distance(origin, entity.center) > 96.0 {
                continue;
            }
            let how = self.config.knowledge.interaction_for(
                &entity.classname,
                entity.spawnflags,
                entity.has_health,
                entity.has_targetname,
            );
            if how == Some("use") || how == Some("push") {
                return true;
            }
        }
        false
    }
}
