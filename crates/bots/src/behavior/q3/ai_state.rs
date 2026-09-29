//! Bot AI state from `src/bots/behavior/q3/ai-state.ts`
//! (`game/ai_main.h`, `ai_main.c` state operations).
//!
//! `BotState` is the per-client brain: player snapshot, view angles,
//! enemy tracking, team goal bookkeeping, goal/activate stacks,
//! waypoints, and library handles (character, move, goal, chat,
//! weapon). This port keeps every donor field with plain Rust storage
//! plus a structural checkpoint.

use qa_core::math::Vec3;

use crate::behavior::library::goals::BotGoal;
use crate::behavior::orders::BotOrderState;
use crate::behavior::q3::ai_definitions::{BotInventory, MAX_ACTIVATEAREAS, MAX_ACTIVATESTACK, MAX_PROXMINES};

/// Maximum clients.
pub const MAX_BOT_CLIENTS: usize = 64;
/// Maximum entities for event-time tracking.
pub const MAX_BOT_ENTITIES: usize = 1024;

/// Bot settings (`bot_settings_t`).
#[derive(Debug, Clone, PartialEq)]
pub struct BotSettings {
    /// Character file.
    pub characterfile: String,
    /// Skill level.
    pub skill: f32,
    /// Team name.
    pub team: String,
}

impl Default for BotSettings {
    fn default() -> Self {
        Self {
            characterfile: String::new(),
            skill: 2.0,
            team: String::new(),
        }
    }
}

/// AI node: the eleven implemented `AINode` functions. `None` remains a
/// distinct source state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AiNode {
    /// Intermission.
    Intermission,
    /// Observer.
    Observer,
    /// Respawn.
    Respawn,
    /// Stand.
    Stand,
    /// Seek activate entity.
    SeekActivateEntity,
    /// Seek nearby goal.
    SeekNbg,
    /// Seek long-term goal.
    SeekLtg,
    /// Battle fight.
    BattleFight,
    /// Battle chase.
    BattleChase,
    /// Battle retreat.
    BattleRetreat,
    /// Battle nearby goal.
    BattleNbg,
}

/// Bot setup stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BotSetupStage {
    /// Allocated.
    Allocated,
    /// Character.
    Character,
    /// Settings.
    Settings,
    /// Goal state.
    GoalState,
    /// Item weights.
    ItemWeights,
    /// Weapon state.
    WeaponState,
    /// Weapon weights.
    WeaponWeights,
    /// Chat state.
    ChatState,
    /// Chat file.
    ChatFile,
    /// Chat gender.
    ChatGender,
    /// Published.
    Published,
    /// Move state.
    MoveState,
    /// Walker.
    Walker,
    /// Counted.
    Counted,
    /// Scheduled.
    Scheduled,
    /// Interbred.
    Interbred,
    /// Session.
    Session,
}

/// Bot setup progress. Progress does not own handles or imply inuse.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BotSetupProgress {
    /// Empty.
    Empty,
    /// Setting up at a stage.
    SettingUp {
        /// Current stage.
        stage: BotSetupStage,
    },
    /// Failed at a stage.
    Failed {
        /// Failing stage.
        stage: BotSetupStage,
        /// Error code.
        error_code: i32,
    },
    /// Complete.
    Complete,
}

/// Player snapshot owned by the brain (detached decision input).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BotPlayerSnapshot {
    /// Health.
    pub health: i32,
    /// Armor.
    pub armor: i32,
    /// Current weapon.
    pub weapon: i32,
    /// Weapon state.
    pub weapon_state: i32,
    /// Move type.
    pub move_type: i32,
    /// Entity flags.
    pub eflags: i32,
    /// Event sequence.
    pub event_sequence: i32,
    /// Stats.
    pub stats: [i32; 16],
    /// Persistents.
    pub persistents: [i32; 16],
    /// Powerups.
    pub powerups: [i32; 16],
    /// View height.
    pub viewheight: i32,
    /// Delta angles.
    pub delta_angles: Vec3,
    /// Ground entity number.
    pub ground_entity_num: i32,
    /// Client number.
    pub client_num: i32,
}

/// User command produced by the brain.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BotUserCommand {
    /// Server time.
    pub server_time: i32,
    /// Angles.
    pub angles: Vec3,
    /// Buttons.
    pub buttons: i32,
    /// Weapon.
    pub weapon: i32,
    /// Forward move.
    pub forwardmove: i32,
    /// Right move.
    pub rightmove: i32,
    /// Up move.
    pub upmove: i32,
}

/// Q3 button bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CommandButtons;

impl CommandButtons {
    /// Attack.
    pub const ATTACK: i32 = 1;
    /// Talk.
    pub const TALK: i32 = 2;
    /// Use holdable.
    pub const USE_HOLDABLE: i32 = 4;
    /// Gesture.
    pub const GESTURE: i32 = 8;
    /// Walking.
    pub const WALKING: i32 = 16;
    /// Affirmative.
    pub const AFFIRMATIVE: i32 = 32;
    /// Negative.
    pub const NEGATIVE: i32 = 64;
    /// Get flag.
    pub const GETFLAG: i32 = 128;
    /// Guard base.
    pub const GUARDBASE: i32 = 256;
    /// Patrol.
    pub const PATROL: i32 = 512;
    /// Follow me.
    pub const FOLLOWME: i32 = 1024;
}

/// Waypoint in a checkpoint/patrol chain.
#[derive(Debug, Clone, PartialEq)]
pub struct BotWaypoint {
    /// In use.
    pub inuse: bool,
    /// Name.
    pub name: String,
    /// Goal.
    pub goal: BotGoal,
    /// Next index in the heap.
    pub next: Option<usize>,
    /// Previous index in the heap.
    pub prev: Option<usize>,
}

impl Default for BotWaypoint {
    fn default() -> Self {
        Self {
            inuse: false,
            name: String::new(),
            goal: BotGoal::default(),
            next: None,
            prev: None,
        }
    }
}

/// Activate goal: an entity the bot must use or shoot.
#[derive(Debug, Clone, PartialEq)]
pub struct BotActivateGoal {
    /// In use.
    pub inuse: bool,
    /// Goal.
    pub goal: BotGoal,
    /// Expiry time.
    pub time: f32,
    /// Start time.
    pub start_time: f32,
    /// Just-used time.
    pub just_used_time: f32,
    /// Shoot instead of touch.
    pub shoot: bool,
    /// Weapon to shoot with.
    pub weapon: i32,
    /// Shoot target.
    pub target: Vec3,
    /// Goal origin.
    pub origin: Vec3,
    /// Activation areas.
    pub areas: [i32; MAX_ACTIVATEAREAS],
    /// Area count.
    pub num_areas: i32,
    /// Areas disabled.
    pub areas_disabled: bool,
}

impl Default for BotActivateGoal {
    fn default() -> Self {
        Self {
            inuse: false,
            goal: BotGoal::default(),
            time: 0.0,
            start_time: 0.0,
            just_used_time: 0.0,
            shoot: false,
            weapon: 0,
            target: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            origin: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            areas: [0; MAX_ACTIVATEAREAS],
            num_areas: 0,
            areas_disabled: false,
        }
    }
}

fn zero3() -> Vec3 {
    Vec3 { x: 0.0, y: 0.0, z: 0.0 }
}

/// Bot AI state (`bot_state_t`).
#[derive(Debug, Clone, PartialEq)]
pub struct BotState {
    /// Scripted order from the director.
    pub scripted_order: Option<BotOrderState>,
    /// Slot in use.
    pub inuse: bool,
    /// Think residual milliseconds.
    pub bot_think_residual: i32,
    /// Client number.
    pub client: i32,
    /// Entity number.
    pub entity_num: i32,
    /// Current player snapshot.
    pub cur_ps: BotPlayerSnapshot,
    /// Last entity flags.
    pub last_eflags: i32,
    /// Last user command.
    pub last_ucmd: BotUserCommand,
    /// Entity event times.
    pub entity_event_time: Vec<i32>,
    /// Bot settings.
    pub settings: BotSettings,
    /// Current AI node.
    pub ai_node: Option<AiNode>,
    /// Think time.
    pub think_time: f32,
    /// Origin.
    pub origin: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Presence type.
    pub presence_type: i32,
    /// Eye position.
    pub eye: Vec3,
    /// Current area.
    pub area_num: i32,
    /// Inventory.
    pub inventory: Vec<i32>,
    /// Travel flags.
    pub tfl: i32,
    /// Bot flags.
    pub flags: i32,
    /// Waiting to respawn.
    pub respawn_wait: bool,
    /// Last health.
    pub last_health: i32,
    /// Last killed player.
    pub last_killed_player: i32,
    /// Last killed by.
    pub last_killed_by: i32,
    /// Bot death type.
    pub bot_death_type: i32,
    /// Enemy death type.
    pub enemy_death_type: i32,
    /// Bot suicide.
    pub bot_suicide: bool,
    /// Enemy suicide.
    pub enemy_suicide: bool,
    /// Setup count.
    pub setup_count: i32,
    /// Map restart.
    pub map_restart: bool,
    /// Enter-game chat done.
    pub enter_game_chat: bool,
    /// Deaths.
    pub num_deaths: i32,
    /// Kills.
    pub num_kills: i32,
    /// Revenge enemy.
    pub revenge_enemy: i32,
    /// Revenge kills.
    pub revenge_kills: i32,
    /// Last frame health.
    pub last_frame_health: i32,
    /// Last hit count.
    pub last_hit_count: i32,
    /// Chat target.
    pub chat_to: i32,
    /// Walker value.
    pub walker: f32,
    /// Last think time.
    pub ltime: f32,
    /// Enter game time.
    pub enter_game_time: f32,
    /// Long-term goal time.
    pub ltg_time: f32,
    /// Nearby goal time.
    pub nbg_time: f32,
    /// Respawn time.
    pub respawn_time: f32,
    /// Respawn chat time.
    pub respawn_chat_time: f32,
    /// Chase time.
    pub chase_time: f32,
    /// Enemy visible time.
    pub enemy_visible_time: f32,
    /// Check time.
    pub check_time: f32,
    /// Stand time.
    pub stand_time: f32,
    /// Last chat time.
    pub last_chat_time: f32,
    /// Kamikaze time.
    pub kamikaze_time: f32,
    /// Invulnerability time.
    pub invulnerability_time: f32,
    /// Stand find enemy time.
    pub stand_find_enemy_time: f32,
    /// Attack strafe time.
    pub attack_strafe_time: f32,
    /// Attack crouch time.
    pub attack_crouch_time: f32,
    /// Attack chase time.
    pub attack_chase_time: f32,
    /// Attack jump time.
    pub attack_jump_time: f32,
    /// Enemy sight time.
    pub enemy_sight_time: f32,
    /// Enemy death time.
    pub enemy_death_time: f32,
    /// Enemy position time.
    pub enemy_position_time: f32,
    /// Defend away time.
    pub defend_away_time: f32,
    /// Defend away range.
    pub defend_away_range: f32,
    /// Rush base away time.
    pub rush_base_away_time: f32,
    /// Attack away time.
    pub attack_away_time: f32,
    /// Harvest away time.
    pub harvest_away_time: f32,
    /// CTF roam time.
    pub ctf_roam_time: f32,
    /// Killed enemy time.
    pub killed_enemy_time: f32,
    /// Arrive time.
    pub arrive_time: f32,
    /// Last air time.
    pub last_air_time: f32,
    /// Teleport time.
    pub teleport_time: f32,
    /// Camp time.
    pub camp_time: f32,
    /// Camp range.
    pub camp_range: f32,
    /// Weapon change time.
    pub weapon_change_time: f32,
    /// Fire throttle wait time.
    pub fire_throttle_wait_time: f32,
    /// Fire throttle shoot time.
    pub fire_throttle_shoot_time: f32,
    /// Not blocked time.
    pub not_blocked_time: f32,
    /// Blocked by avoid spot time.
    pub blocked_by_avoid_spot_time: f32,
    /// Predict obstacles time.
    pub predict_obstacles_time: f32,
    /// Predict obstacles goal area.
    pub predict_obstacles_goal_area_num: i32,
    /// Aim target.
    pub aim_target: Vec3,
    /// Enemy velocity.
    pub enemy_velocity: Vec3,
    /// Enemy origin.
    pub enemy_origin: Vec3,
    /// Kamikaze body.
    pub kamikaze_body: i32,
    /// Proximity mines.
    pub prox_mines: Vec<i32>,
    /// Proximity mine count.
    pub num_prox_mines: i32,
    /// Character handle.
    pub character: i32,
    /// Move state handle.
    pub ms: i32,
    /// Goal state handle.
    pub gs: i32,
    /// Chat state handle.
    pub cs: i32,
    /// Weapon state handle.
    pub ws: i32,
    /// Enemy client, or -1.
    pub enemy: i32,
    /// Last enemy area.
    pub last_enemy_area_num: i32,
    /// Last enemy origin.
    pub last_enemy_origin: Vec3,
    /// Weapon number.
    pub weapon_num: i32,
    /// View angles.
    pub viewangles: Vec3,
    /// Ideal view angles.
    pub ideal_viewangles: Vec3,
    /// View angle speed.
    pub viewangle_speed: Vec3,
    /// Long-term goal type.
    pub ltg_type: i32,
    /// Teammate client.
    pub teammate: i32,
    /// Decision maker.
    pub decisionmaker: i32,
    /// Ordered flag.
    pub ordered: bool,
    /// Order time.
    pub order_time: f32,
    /// Own decision time.
    pub own_decision_time: i32,
    /// Team goal.
    pub team_goal: BotGoal,
    /// Alternate route goal.
    pub alt_route_goal: BotGoal,
    /// Reached alternate route goal time.
    pub reached_alt_route_goal_time: f32,
    /// Team message time.
    pub team_message_time: f32,
    /// Team goal time.
    pub team_goal_time: f32,
    /// Teammate visible time.
    pub teammate_visible_time: f32,
    /// Team task preference.
    pub team_task_preference: i32,
    /// Last goal decision maker.
    pub last_goal_decisionmaker: i32,
    /// Last goal long-term goal type.
    pub last_goal_ltg_type: i32,
    /// Last goal teammate.
    pub last_goal_teammate: i32,
    /// Last goal team goal.
    pub last_goal_team_goal: BotGoal,
    /// Lead teammate.
    pub lead_teammate: i32,
    /// Lead team goal.
    pub lead_team_goal: BotGoal,
    /// Lead time.
    pub lead_time: f32,
    /// Lead visible time.
    pub lead_visible_time: f32,
    /// Lead message time.
    pub lead_message_time: f32,
    /// Lead backup time.
    pub lead_backup_time: f32,
    /// Team leader name.
    pub team_leader: String,
    /// Ask team leader time.
    pub ask_team_leader_time: f32,
    /// Become team leader time.
    pub become_team_leader_time: f32,
    /// Team give orders time.
    pub team_give_orders_time: f32,
    /// Last flag capture time.
    pub last_flag_capture_time: f32,
    /// Teammate count.
    pub num_teammates: i32,
    /// Red flag status.
    pub red_flag_status: i32,
    /// Blue flag status.
    pub blue_flag_status: i32,
    /// Neutral flag status.
    pub neutral_flag_status: i32,
    /// Flag status changed.
    pub flag_status_changed: bool,
    /// Force orders.
    pub force_orders: bool,
    /// Flag carrier.
    pub flag_carrier: i32,
    /// CTF strategy.
    pub ctf_strategy: i32,
    /// Subteam name.
    pub subteam: String,
    /// Formation distance.
    pub formation_dist: f32,
    /// Formation teammate.
    pub formation_teammate: String,
    /// Formation angle.
    pub formation_angle: f32,
    /// Formation direction.
    pub formation_dir: Vec3,
    /// Formation origin.
    pub formation_origin: Vec3,
    /// Formation goal.
    pub formation_goal: BotGoal,
    /// Activate goal stack (heap indexes, top last).
    pub activate_stack: Vec<usize>,
    /// Activate goal heap.
    pub activate_goal_heap: Vec<BotActivateGoal>,
    /// Waypoint heap.
    pub waypoints: Vec<BotWaypoint>,
    /// Checkpoint chain head.
    pub checkpoints: Option<usize>,
    /// Patrol chain head.
    pub patrol_points: Option<usize>,
    /// Current patrol point.
    pub current_patrol_point: Option<usize>,
    /// Patrol flags.
    pub patrol_flags: i32,
    /// Setup progress.
    pub setup: BotSetupProgress,
}

impl BotState {
    /// New idle state for a client.
    #[must_use]
    pub fn new(client: i32) -> Self {
        Self {
            scripted_order: None,
            inuse: false,
            bot_think_residual: 0,
            client,
            entity_num: client,
            cur_ps: BotPlayerSnapshot::default(),
            last_eflags: 0,
            last_ucmd: BotUserCommand::default(),
            entity_event_time: vec![0; MAX_BOT_ENTITIES],
            settings: BotSettings::default(),
            ai_node: None,
            think_time: 0.0,
            origin: zero3(),
            velocity: zero3(),
            presence_type: 0,
            eye: zero3(),
            area_num: 0,
            inventory: vec![0; BotInventory::SIZE],
            tfl: 0,
            flags: 0,
            respawn_wait: false,
            last_health: 0,
            last_killed_player: 0,
            last_killed_by: 0,
            bot_death_type: 0,
            enemy_death_type: 0,
            bot_suicide: false,
            enemy_suicide: false,
            setup_count: 0,
            map_restart: false,
            enter_game_chat: false,
            num_deaths: 0,
            num_kills: 0,
            revenge_enemy: 0,
            revenge_kills: 0,
            last_frame_health: 0,
            last_hit_count: 0,
            chat_to: 0,
            walker: 0.0,
            ltime: 0.0,
            enter_game_time: 0.0,
            ltg_time: 0.0,
            nbg_time: 0.0,
            respawn_time: 0.0,
            respawn_chat_time: 0.0,
            chase_time: 0.0,
            enemy_visible_time: 0.0,
            check_time: 0.0,
            stand_time: 0.0,
            last_chat_time: 0.0,
            kamikaze_time: 0.0,
            invulnerability_time: 0.0,
            stand_find_enemy_time: 0.0,
            attack_strafe_time: 0.0,
            attack_crouch_time: 0.0,
            attack_chase_time: 0.0,
            attack_jump_time: 0.0,
            enemy_sight_time: 0.0,
            enemy_death_time: 0.0,
            enemy_position_time: 0.0,
            defend_away_time: 0.0,
            defend_away_range: 0.0,
            rush_base_away_time: 0.0,
            attack_away_time: 0.0,
            harvest_away_time: 0.0,
            ctf_roam_time: 0.0,
            killed_enemy_time: 0.0,
            arrive_time: 0.0,
            last_air_time: 0.0,
            teleport_time: 0.0,
            camp_time: 0.0,
            camp_range: 0.0,
            weapon_change_time: 0.0,
            fire_throttle_wait_time: 0.0,
            fire_throttle_shoot_time: 0.0,
            not_blocked_time: 0.0,
            blocked_by_avoid_spot_time: 0.0,
            predict_obstacles_time: 0.0,
            predict_obstacles_goal_area_num: 0,
            aim_target: zero3(),
            enemy_velocity: zero3(),
            enemy_origin: zero3(),
            kamikaze_body: 0,
            prox_mines: vec![0; MAX_PROXMINES],
            num_prox_mines: 0,
            character: 0,
            ms: 0,
            gs: 0,
            cs: 0,
            ws: 0,
            enemy: -1,
            last_enemy_area_num: 0,
            last_enemy_origin: zero3(),
            weapon_num: 0,
            viewangles: zero3(),
            ideal_viewangles: zero3(),
            viewangle_speed: zero3(),
            ltg_type: 0,
            teammate: 0,
            decisionmaker: 0,
            ordered: false,
            order_time: 0.0,
            own_decision_time: 0,
            team_goal: BotGoal::default(),
            alt_route_goal: BotGoal::default(),
            reached_alt_route_goal_time: 0.0,
            team_message_time: 0.0,
            team_goal_time: 0.0,
            teammate_visible_time: 0.0,
            team_task_preference: 0,
            last_goal_decisionmaker: 0,
            last_goal_ltg_type: 0,
            last_goal_teammate: 0,
            last_goal_team_goal: BotGoal::default(),
            lead_teammate: 0,
            lead_team_goal: BotGoal::default(),
            lead_time: 0.0,
            lead_visible_time: 0.0,
            lead_message_time: 0.0,
            lead_backup_time: 0.0,
            team_leader: String::new(),
            ask_team_leader_time: 0.0,
            become_team_leader_time: 0.0,
            team_give_orders_time: 0.0,
            last_flag_capture_time: 0.0,
            num_teammates: 0,
            red_flag_status: 0,
            blue_flag_status: 0,
            neutral_flag_status: 0,
            flag_status_changed: false,
            force_orders: false,
            flag_carrier: 0,
            ctf_strategy: 0,
            subteam: String::new(),
            formation_dist: 0.0,
            formation_teammate: String::new(),
            formation_angle: 0.0,
            formation_dir: zero3(),
            formation_origin: zero3(),
            formation_goal: BotGoal::default(),
            activate_stack: Vec::new(),
            activate_goal_heap: vec![BotActivateGoal::default(); MAX_ACTIVATESTACK],
            waypoints: Vec::new(),
            checkpoints: None,
            patrol_points: None,
            current_patrol_point: None,
            patrol_flags: 0,
            setup: BotSetupProgress::Empty,
        }
    }

    /// Reset decision state on respawn (`BotResetState` decision half).
    pub fn reset_decision_state(&mut self) {
        self.enemy = -1;
        self.ai_node = None;
        self.ltg_type = 0;
        self.teammate = 0;
        self.ordered = false;
        self.team_goal = BotGoal::default();
        self.alt_route_goal = BotGoal::default();
        self.activate_stack.clear();
        for goal in &mut self.activate_goal_heap {
            *goal = BotActivateGoal::default();
        }
        self.checkpoints = None;
        self.patrol_points = None;
        self.current_patrol_point = None;
        self.scripted_order = None;
    }

    /// Allocate a waypoint; returns the heap index.
    pub fn alloc_waypoint(&mut self) -> Option<usize> {
        if let Some(index) = self.waypoints.iter().position(|point| !point.inuse) {
            self.waypoints[index] = BotWaypoint {
                inuse: true,
                ..BotWaypoint::default()
            };
            return Some(index);
        }
        if self.waypoints.len() >= super::ai_definitions::MAX_WAYPOINTS {
            return None;
        }
        self.waypoints.push(BotWaypoint {
            inuse: true,
            ..BotWaypoint::default()
        });
        Some(self.waypoints.len() - 1)
    }

    /// Free a waypoint chain.
    pub fn free_waypoints(&mut self, head: Option<usize>) {
        let mut next = head;
        while let Some(index) = next {
            if let Some(point) = self.waypoints.get_mut(index) {
                next = point.next;
                *point = BotWaypoint::default();
            } else {
                break;
            }
        }
    }

    /// Allocate an activate goal; returns the heap index.
    pub fn alloc_activate_goal(&mut self) -> Option<usize> {
        self.activate_goal_heap
            .iter()
            .position(|goal| !goal.inuse)
            .map(|index| {
                self.activate_goal_heap[index].inuse = true;
                index
            })
    }
}

/// Bot state store: one cell per client slot.
#[derive(Debug, Clone, Default)]
pub struct BotStateStore {
    cells: Vec<Option<BotState>>,
}

impl BotStateStore {
    /// Store with `MAX_BOT_CLIENTS` slots.
    #[must_use]
    pub fn new() -> Self {
        Self {
            cells: (0..MAX_BOT_CLIENTS).map(|_| None).collect(),
        }
    }

    /// Borrow a client's state.
    #[must_use]
    pub fn get(&self, client: i32) -> Option<&BotState> {
        usize::try_from(client)
            .ok()
            .and_then(|index| self.cells.get(index)?.as_ref())
    }

    /// Mutably borrow a client's state.
    pub fn get_mut(&mut self, client: i32) -> Option<&mut BotState> {
        usize::try_from(client)
            .ok()
            .and_then(|index| self.cells.get_mut(index)?.as_mut())
    }

    /// Acquire (or create) a client's state.
    pub fn acquire(&mut self, client: i32) -> &mut BotState {
        let index = usize::try_from(client).unwrap_or(0).min(MAX_BOT_CLIENTS - 1);
        if self.cells[index].is_none() {
            self.cells[index] = Some(BotState::new(client));
        }
        self.cells[index].as_mut().expect("state just acquired")
    }

    /// Release a client's state.
    pub fn release(&mut self, client: i32) {
        if let Ok(index) = usize::try_from(client) {
            if let Some(slot) = self.cells.get_mut(index) {
                *slot = None;
            }
        }
    }

    /// In-use client count.
    #[must_use]
    pub fn in_use(&self) -> usize {
        self.cells
            .iter()
            .filter(|cell| cell.as_ref().is_some_and(|state| state.inuse))
            .count()
    }
}
