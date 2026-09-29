//! Bot movement states from `src/bots/behavior/q3/movement-state.ts`
//! (`be_ai_move.c` `bot_movestate_t`, `BotAllocMoveState`,
//! `BotInitMoveState`, `BotInitAvoidReach`).
//!
//! Each bot owns a movement state: origin/velocity/angles, travel
//! bookkeeping, and avoid-reach/avoid-spot lists that keep the bot from
//! retrying blocked reachabilities.

use qa_core::math::Vec3;

/// Maximum movement states.
pub const MAX_MOVE_STATES: usize = 64;
/// Maximum avoided reachabilities.
pub const MAX_AVOID_REACH: usize = 1;
/// Maximum avoid spots.
pub const MAX_AVOID_SPOTS: usize = 32;

/// Movement type bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BotMoveType;

impl BotMoveType {
    /// Walk.
    pub const WALK: i32 = 1;
    /// Crouch.
    pub const CROUCH: i32 = 2;
    /// Jump.
    pub const JUMP: i32 = 4;
    /// Grapple.
    pub const GRAPPLE: i32 = 8;
    /// Rocket jump.
    pub const ROCKETJUMP: i32 = 16;
    /// BFG jump.
    pub const BFGJUMP: i32 = 32;
}

/// Movement flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BotMoveFlag;

impl BotMoveFlag {
    /// Barrier jump.
    pub const BARRIERJUMP: i32 = 1;
    /// On ground.
    pub const ONGROUND: i32 = 2;
    /// Swimming.
    pub const SWIMMING: i32 = 4;
    /// Against ladder.
    pub const AGAINSTLADDER: i32 = 8;
    /// Water jump.
    pub const WATERJUMP: i32 = 16;
    /// Teleported.
    pub const TELEPORTED: i32 = 32;
    /// Grapple pull.
    pub const GRAPPLEPULL: i32 = 64;
    /// Active grapple.
    pub const ACTIVEGRAPPLE: i32 = 128;
    /// Grapple reset.
    pub const GRAPPLERESET: i32 = 256;
    /// Walk.
    pub const WALK: i32 = 512;
}

/// Movement result flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BotMoveResultFlag;

impl BotMoveResultFlag {
    /// Movement view.
    pub const MOVEMENTVIEW: i32 = 1;
    /// Swim view.
    pub const SWIMVIEW: i32 = 2;
    /// Waiting.
    pub const WAITING: i32 = 4;
    /// Movement view set.
    pub const MOVEMENTVIEWSET: i32 = 8;
    /// Movement weapon.
    pub const MOVEMENTWEAPON: i32 = 16;
    /// On top of obstacle.
    pub const ONTOPOFOBSTACLE: i32 = 32;
    /// On top of func_bob.
    pub const ONTOPOF_FUNCBOB: i32 = 64;
    /// On top of elevator.
    pub const ONTOPOF_ELEVATOR: i32 = 128;
    /// Blocked by avoid spot.
    pub const BLOCKEDBYAVOIDSPOT: i32 = 256;
}

/// Movement result types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BotMoveResultType;

impl BotMoveResultType {
    /// Elevator up.
    pub const ELEVATORUP: i32 = 1;
    /// Wait for func bobbing.
    pub const WAITFORFUNCBOBBING: i32 = 2;
    /// Bad grapple path.
    pub const BADGRAPPLEPATH: i32 = 4;
    /// In solid area.
    pub const INSOLIDAREA: i32 = 8;
}

/// Avoid spot types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BotAvoidSpotType;

impl BotAvoidSpotType {
    /// Clear.
    pub const CLEAR: i32 = 0;
    /// Always avoid.
    pub const ALWAYS: i32 = 1;
    /// Don't block.
    pub const DONTBLOCK: i32 = 2;
}

/// Movement result (`bot_moveresult_t`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BotMoveResult {
    /// Movement failed.
    pub failure: bool,
    /// Result type bits.
    pub result_type: i32,
    /// Blocked.
    pub blocked: bool,
    /// Blocking entity.
    pub block_entity: i32,
    /// Travel type used.
    pub travel_type: i32,
    /// Result flags.
    pub flags: i32,
    /// Movement weapon.
    pub weapon: i32,
    /// Move direction.
    pub move_direction: Vec3,
    /// Ideal view angles.
    pub ideal_view_angles: Vec3,
}

/// Avoid spot: origin, radius, and type.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BotAvoidSpot {
    /// Spot origin.
    pub origin: Vec3,
    /// Spot radius.
    pub radius: f32,
    /// Spot type.
    pub spot_type: i32,
}

/// Movement state init (`bot_initmove_t`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BotInitMove {
    /// Start origin.
    pub origin: Vec3,
    /// Start velocity.
    pub velocity: Vec3,
    /// View offset.
    pub view_offset: Vec3,
    /// Entity number.
    pub entity_num: i32,
    /// Client number.
    pub client: i32,
    /// Think time.
    pub think_time: f32,
    /// Presence type.
    pub presence_type: i32,
    /// View angles.
    pub view_angles: Vec3,
    /// OR-ed move flags.
    pub or_move_flags: i32,
}

/// Bot movement state (`bot_movestate_t`).
#[derive(Debug, Clone, PartialEq)]
pub struct BotMoveState {
    /// Current origin.
    pub origin: Vec3,
    /// Current velocity.
    pub velocity: Vec3,
    /// View offset.
    pub view_offset: Vec3,
    /// Entity number.
    pub entity_num: i32,
    /// Client number.
    pub client: i32,
    /// Think time.
    pub think_time: f32,
    /// Presence type.
    pub presence_type: i32,
    /// View angles.
    pub view_angles: Vec3,
    /// Move flags.
    pub move_flags: i32,
    /// Avoided reachabilities.
    pub avoid_reach: [i32; MAX_AVOID_REACH],
    /// Avoid reach expiry times.
    pub avoid_reach_times: [f32; MAX_AVOID_REACH],
    /// Avoid reach retry counts.
    pub avoid_reach_tries: [i32; MAX_AVOID_REACH],
    /// Avoid spots.
    pub avoid_spots: Vec<BotAvoidSpot>,
    /// Last origin.
    pub last_origin: Vec3,
    /// Last area.
    pub last_area: i32,
    /// Last time on ground.
    pub last_time_on_ground: f32,
    /// Jump frame time.
    pub jump_time: f32,
}

impl BotMoveState {
    /// Initialize from an init move (`BotInitMoveState`).
    #[must_use]
    pub fn from_init(init: &BotInitMove) -> Self {
        Self {
            origin: init.origin,
            velocity: init.velocity,
            view_offset: init.view_offset,
            entity_num: init.entity_num,
            client: init.client,
            think_time: init.think_time,
            presence_type: init.presence_type,
            view_angles: init.view_angles,
            move_flags: init.or_move_flags,
            avoid_reach: [0; MAX_AVOID_REACH],
            avoid_reach_times: [0.0; MAX_AVOID_REACH],
            avoid_reach_tries: [0; MAX_AVOID_REACH],
            avoid_spots: vec![BotAvoidSpot::default(); MAX_AVOID_SPOTS],
            last_origin: init.origin,
            last_area: 0,
            last_time_on_ground: 0.0,
            jump_time: 0.0,
        }
    }

    /// Reset avoid reach (`BotInitAvoidReach`).
    pub fn reset_avoid_reach(&mut self) {
        self.avoid_reach = [0; MAX_AVOID_REACH];
        self.avoid_reach_times = [0.0; MAX_AVOID_REACH];
        self.avoid_reach_tries = [0; MAX_AVOID_REACH];
    }

    /// Add an avoid spot (`BotAddAvoidSpot`); returns the slot.
    pub fn add_avoid_spot(&mut self, origin: Vec3, radius: f32, spot_type: i32) -> usize {
        if let Some(slot) = self
            .avoid_spots
            .iter_mut()
            .find(|spot| spot.spot_type == BotAvoidSpotType::CLEAR)
        {
            *slot = BotAvoidSpot {
                origin,
                radius,
                spot_type,
            };
            self.avoid_spots
                .iter()
                .position(|spot| spot.spot_type != BotAvoidSpotType::CLEAR && spot.origin == origin)
                .unwrap_or(0)
        } else {
            self.avoid_spots[0] = BotAvoidSpot {
                origin,
                radius,
                spot_type,
            };
            0
        }
    }
}

/// Movement state store (`BotAllocMoveState` and friends).
#[derive(Debug, Clone, Default)]
pub struct BotMoveStateStore {
    states: Vec<Option<BotMoveState>>,
}

impl BotMoveStateStore {
    /// Empty store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            states: vec![None; MAX_MOVE_STATES],
        }
    }

    /// Allocate a state; returns the 1-based handle.
    pub fn alloc(&mut self) -> Option<i32> {
        self.states.iter().position(Option::is_none).map(|index| {
            self.states[index] = Some(BotMoveState::from_init(&BotInitMove {
                origin: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                velocity: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                view_offset: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                entity_num: 0,
                client: 0,
                think_time: 0.0,
                presence_type: 0,
                view_angles: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                or_move_flags: 0,
            }));
            index as i32 + 1
        })
    }

    /// Free a state.
    pub fn free(&mut self, handle: i32) {
        if let Some(slot) = handle
            .checked_sub(1)
            .and_then(|index| self.states.get_mut(index as usize))
        {
            *slot = None;
        }
    }

    /// Initialize a state from an init move.
    pub fn init(&mut self, handle: i32, init: &BotInitMove) {
        if let Some(slot) = handle
            .checked_sub(1)
            .and_then(|index| self.states.get_mut(index as usize))
        {
            *slot = Some(BotMoveState::from_init(init));
        }
    }

    /// Reset a state to a fresh init move.
    pub fn reset(&mut self, handle: i32) {
        if let Some(Some(state)) = handle
            .checked_sub(1)
            .and_then(|index| self.states.get_mut(index as usize))
        {
            let init = BotInitMove {
                origin: state.origin,
                velocity: state.velocity,
                view_offset: state.view_offset,
                entity_num: state.entity_num,
                client: state.client,
                think_time: state.think_time,
                presence_type: state.presence_type,
                view_angles: state.view_angles,
                or_move_flags: 0,
            };
            *state = BotMoveState::from_init(&init);
        }
    }

    /// Reset avoid reach for a state.
    pub fn reset_avoid_reach(&mut self, handle: i32) {
        if let Some(Some(state)) = handle
            .checked_sub(1)
            .and_then(|index| self.states.get_mut(index as usize))
        {
            state.reset_avoid_reach();
        }
    }

    /// Borrow a state.
    #[must_use]
    pub fn get(&self, handle: i32) -> Option<&BotMoveState> {
        handle
            .checked_sub(1)
            .and_then(|index| self.states.get(index as usize)?.as_ref())
    }

    /// Mutably borrow a state.
    pub fn get_mut(&mut self, handle: i32) -> Option<&mut BotMoveState> {
        handle
            .checked_sub(1)
            .and_then(|index| self.states.get_mut(index as usize)?.as_mut())
    }
}
