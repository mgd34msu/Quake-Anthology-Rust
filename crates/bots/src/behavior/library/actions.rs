//! Bot action buffer from `src/bots/behavior/library/actions.ts`
//! (`be_ea.c`: `EA_SetInput`, `EA_GetInput`, `EA_ResetInput`).
//!
//! Each client owns one input cell: think time, move direction/speed,
//! view angles, action flags, and weapon. The brain writes through the
//! `EA_*` entry points every frame; the game reads the cell back through
//! `get_input` and converts it to a user command.

use qa_core::math::Vec3;

/// Elementary-action flags (`actionflag_t`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BotActionFlag;

impl BotActionFlag {
    /// Attack.
    pub const ATTACK: i32 = 0x0000_0001;
    /// Use.
    pub const USE: i32 = 0x0000_0002;
    /// Respawn.
    pub const RESPAWN: i32 = 0x0000_0008;
    /// Jump.
    pub const JUMP: i32 = 0x0000_0010;
    /// Move up.
    pub const MOVE_UP: i32 = 0x0000_0020;
    /// Crouch.
    pub const CROUCH: i32 = 0x0000_0080;
    /// Move down.
    pub const MOVE_DOWN: i32 = 0x0000_0100;
    /// Move forward.
    pub const MOVE_FORWARD: i32 = 0x0000_0200;
    /// Move back.
    pub const MOVE_BACK: i32 = 0x0000_0800;
    /// Move left.
    pub const MOVE_LEFT: i32 = 0x0000_1000;
    /// Move right.
    pub const MOVE_RIGHT: i32 = 0x0000_2000;
    /// Delayed jump (converts to jump on read).
    pub const DELAYED_JUMP: i32 = 0x0000_8000;
    /// Talk.
    pub const TALK: i32 = 0x0001_0000;
    /// Gesture.
    pub const GESTURE: i32 = 0x0002_0000;
    /// Walk (vs run).
    pub const WALK: i32 = 0x0008_0000;
    /// Affirmative voice.
    pub const AFFIRMATIVE: i32 = 0x0010_0000;
    /// Negative voice.
    pub const NEGATIVE: i32 = 0x0020_0000;
    /// Get flag.
    pub const GET_FLAG: i32 = 0x0080_0000;
    /// Guard base.
    pub const GUARD_BASE: i32 = 0x0100_0000;
    /// Patrol.
    pub const PATROL: i32 = 0x0200_0000;
    /// Follow me.
    pub const FOLLOW_ME: i32 = 0x0800_0000;
}

/// One client's elementary-action input cell.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BotInput {
    /// Think time in seconds.
    pub think_time: f32,
    /// Move direction.
    pub direction: Vec3,
    /// Move speed (0-400).
    pub speed: f32,
    /// View angles.
    pub view_angles: Vec3,
    /// Action flags.
    pub action_flags: i32,
    /// Selected weapon.
    pub weapon: i32,
}

impl Default for BotInput {
    fn default() -> Self {
        Self {
            think_time: 0.0,
            direction: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            speed: 0.0,
            view_angles: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            action_flags: 0,
            weapon: 0,
        }
    }
}

/// Per-client action buffer (`BotActionBuffer`).
#[derive(Debug, Clone)]
pub struct BotActionBuffer {
    inputs: Vec<BotInput>,
}

impl BotActionBuffer {
    /// Buffer for `clients` client slots.
    #[must_use]
    pub fn new(clients: usize) -> Self {
        Self {
            inputs: vec![BotInput::default(); clients],
        }
    }

    /// Client capacity.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.inputs.len()
    }

    fn cell(&mut self, client: i32) -> &mut BotInput {
        let index = usize::try_from(client).unwrap_or(usize::MAX);
        if index >= self.inputs.len() {
            panic!("bot action client {client} exceeds capacity {}", self.inputs.len());
        }
        &mut self.inputs[index]
    }

    /// Reset a client's cell (`EA_ResetInput`).
    pub fn reset_input(&mut self, client: i32) {
        *self.cell(client) = BotInput::default();
    }

    /// Read a client's cell (`EA_GetInput`).
    #[must_use]
    pub fn get_input(&self, client: i32) -> BotInput {
        let index = usize::try_from(client).unwrap_or(usize::MAX);
        self.inputs.get(index).copied().unwrap_or_default()
    }

    /// Set the move for a client (`EA_Move`).
    pub fn set_move(&mut self, client: i32, direction: Vec3, speed: f32) {
        let cell = self.cell(client);
        cell.direction = direction;
        cell.speed = speed;
    }

    /// Set the view angles (`EA_View`).
    pub fn view(&mut self, client: i32, angles: Vec3) {
        self.cell(client).view_angles = angles;
    }

    /// Set the think time (`EA_ThinkTime`... stored on next action).
    pub fn think_time(&mut self, client: i32, time: f32) {
        self.cell(client).think_time = time;
    }

    /// OR action flags into the cell.
    pub fn action(&mut self, client: i32, flags: i32) {
        let cell = self.cell(client);
        cell.action_flags |= flags;
    }

    /// Select a weapon (`EA_SelectWeapon`).
    pub fn select_weapon(&mut self, client: i32, weapon: i32) {
        self.cell(client).weapon = weapon;
    }

    /// Attack hold (`EA_Attack`).
    pub fn attack(&mut self, client: i32) {
        self.action(client, BotActionFlag::ATTACK);
    }

    /// Use hold (`EA_Use`).
    pub fn use_holdable(&mut self, client: i32) {
        self.action(client, BotActionFlag::USE);
    }

    /// Respawn press (`EA_Respawn`).
    pub fn respawn(&mut self, client: i32) {
        self.action(client, BotActionFlag::RESPAWN);
    }

    /// Jump press (`EA_Jump`).
    pub fn jump(&mut self, client: i32) {
        self.action(client, BotActionFlag::JUMP);
    }

    /// Delayed jump (`EA_DelayedJump`).
    pub fn delayed_jump(&mut self, client: i32) {
        self.action(client, BotActionFlag::DELAYED_JUMP);
    }

    /// Crouch hold (`EA_Crouch`).
    pub fn crouch(&mut self, client: i32) {
        self.action(client, BotActionFlag::CROUCH);
    }

    /// Walk modifier (`EA_Walk`).
    pub fn walk(&mut self, client: i32) {
        self.action(client, BotActionFlag::WALK);
    }

    /// Talk press (`EA_Talk`).
    pub fn talk(&mut self, client: i32) {
        self.action(client, BotActionFlag::TALK);
    }

    /// Gesture press (`EA_Gesture`).
    pub fn gesture(&mut self, client: i32) {
        self.action(client, BotActionFlag::GESTURE);
    }

    /// Move forward hold (`EA_MoveForward`).
    pub fn move_forward(&mut self, client: i32) {
        self.action(client, BotActionFlag::MOVE_FORWARD);
    }

    /// Move back hold (`EA_MoveBack`).
    pub fn move_back(&mut self, client: i32) {
        self.action(client, BotActionFlag::MOVE_BACK);
    }

    /// Strafe left hold (`EA_MoveLeft`).
    pub fn move_left(&mut self, client: i32) {
        self.action(client, BotActionFlag::MOVE_LEFT);
    }

    /// Strafe right hold (`EA_MoveRight`).
    pub fn move_right(&mut self, client: i32) {
        self.action(client, BotActionFlag::MOVE_RIGHT);
    }

    /// Move up hold (`EA_MoveUp`).
    pub fn move_up(&mut self, client: i32) {
        self.action(client, BotActionFlag::MOVE_UP);
    }

    /// Move down hold (`EA_MoveDown`).
    pub fn move_down(&mut self, client: i32) {
        self.action(client, BotActionFlag::MOVE_DOWN);
    }

    /// Voice command (`EA_Command` maps onto the donor's voice flags).
    pub fn command(&mut self, client: i32, flags: i32) {
        self.action(client, flags);
    }

    /// Checkpoint all input cells.
    #[must_use]
    pub fn checkpoint(&self) -> Vec<BotInput> {
        self.inputs.clone()
    }

    /// Restore checkpointed input cells.
    pub fn restore(&mut self, inputs: &[BotInput]) {
        self.inputs = inputs.to_vec();
    }
}
