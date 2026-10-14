//! Movement capability data; the one movement executor consumes this table.
use qa_core::primitives::{Bounds, MovementMode, MovementTimer, RuleSetId, Vec3};

#[derive(Clone, Copy)]
pub struct ViewAnglePolicy {
    pub short_angles: bool,
    pub signed_pitch_limit: bool,
    pub circular_pitch: bool,
    pub lock_dead_non_spectator: bool,
}
#[derive(Clone, Copy)]
pub struct ViewAnglePolicies {
    pub movement: ViewAnglePolicy,
    pub prediction: ViewAnglePolicy,
}
const RAW_ANGLES: ViewAnglePolicy = ViewAnglePolicy {
    short_angles: false,
    signed_pitch_limit: false,
    circular_pitch: false,
    lock_dead_non_spectator: false,
};
const CLASSIC_ANGLES: ViewAnglePolicy = ViewAnglePolicy {
    short_angles: true,
    circular_pitch: true,
    ..RAW_ANGLES
};
const RERELEASE_ANGLES: ViewAnglePolicy = ViewAnglePolicy {
    circular_pitch: true,
    ..RAW_ANGLES
};
const ARENA_ANGLES: ViewAnglePolicy = ViewAnglePolicy {
    short_angles: true,
    signed_pitch_limit: true,
    lock_dead_non_spectator: true,
    ..RAW_ANGLES
};
const VIEW_ANGLES: [ViewAnglePolicies; 5] = [
    ViewAnglePolicies {
        movement: RAW_ANGLES,
        prediction: RAW_ANGLES,
    },
    ViewAnglePolicies {
        movement: RAW_ANGLES,
        prediction: RAW_ANGLES,
    },
    ViewAnglePolicies {
        movement: CLASSIC_ANGLES,
        // Native cl_pred's no-position branch does not apply Pmove clamps.
        prediction: RAW_ANGLES,
    },
    ViewAnglePolicies {
        movement: RERELEASE_ANGLES,
        // q2repro CL_PredictAngles adds command and float delta directly.
        prediction: RAW_ANGLES,
    },
    ViewAnglePolicies {
        movement: ARENA_ANGLES,
        prediction: ARENA_ANGLES,
    },
];

#[derive(Clone, Copy)]
pub enum FreeWish {
    Existing,
    NativeControl,
    Full,
    Normalized,
    ScaledBasis,
}
#[derive(Clone, Copy)]
pub enum FreeFriction {
    Native,
    StopControlled,
    Linear(f32),
    Flight(f32),
}
#[derive(Clone, Copy)]
pub enum FreeMotion {
    Direct,
    StepSlide,
}
#[derive(Clone, Copy)]
pub enum FreeAcceleration {
    Immediate,
    Normal,
    Flight,
}
#[derive(Clone, Copy)]
pub enum FreeFinish {
    Snap(bool),
    Timers,
    Environment,
}
#[derive(Clone, Copy)]
pub struct FreeMovePolicy {
    pub wish: FreeWish,
    pub friction: FreeFriction,
    pub motion: FreeMotion,
    pub acceleration: FreeAcceleration,
    pub finish: FreeFinish,
    pub abort_without_acceleration: bool,
    pub stance: bool,
    pub spectator_speed: bool,
    pub speed_multiplier: f32,
    pub button_vertical: bool,
    pub world_only: bool,
    pub bounds: Option<Bounds>,
    pub view_height: Option<f32>,
    pub reset_flags: bool,
    pub clear_water_outputs: bool,
    pub clear_ground_reference: bool,
    pub exclude_body_contents: bool,
}
#[derive(Clone, Copy)]
pub struct FreeModes {
    pub flight: FreeMovePolicy,
    pub noclip: FreeMovePolicy,
    pub spectator: FreeMovePolicy,
}
impl FreeModes {
    pub fn get(self, mode: MovementMode) -> Option<FreeMovePolicy> {
        match mode {
            MovementMode::Fly => Some(self.flight),
            MovementMode::Noclip => Some(self.noclip),
            MovementMode::Spectator => Some(self.spectator),
            _ => None,
        }
    }
}
const QUAKE_FREE: FreeMovePolicy = FreeMovePolicy {
    wish: FreeWish::Existing,
    friction: FreeFriction::Native,
    motion: FreeMotion::Direct,
    acceleration: FreeAcceleration::Immediate,
    finish: FreeFinish::Snap(true),
    abort_without_acceleration: false,
    stance: false,
    spectator_speed: false,
    speed_multiplier: 1.0,
    button_vertical: false,
    world_only: false,
    bounds: None,
    view_height: None,
    reset_flags: false,
    clear_water_outputs: false,
    clear_ground_reference: false,
    exclude_body_contents: false,
};
const CLASSIC_FREE: FreeMovePolicy = FreeMovePolicy {
    wish: FreeWish::Normalized,
    friction: FreeFriction::StopControlled,
    motion: FreeMotion::Direct,
    acceleration: FreeAcceleration::Normal,
    finish: FreeFinish::Snap(true),
    abort_without_acceleration: true,
    stance: false,
    spectator_speed: false,
    speed_multiplier: 1.0,
    button_vertical: false,
    world_only: false,
    bounds: None,
    view_height: None,
    reset_flags: false,
    clear_water_outputs: false,
    clear_ground_reference: false,
    exclude_body_contents: false,
};
const ARENA_NOCLIP: FreeMovePolicy = FreeMovePolicy {
    wish: FreeWish::Full,
    friction: FreeFriction::StopControlled,
    motion: FreeMotion::Direct,
    acceleration: FreeAcceleration::Normal,
    finish: FreeFinish::Timers,
    abort_without_acceleration: false,
    stance: false,
    spectator_speed: false,
    speed_multiplier: 1.0,
    button_vertical: false,
    world_only: false,
    bounds: None,
    view_height: None,
    reset_flags: false,
    clear_water_outputs: true,
    clear_ground_reference: false,
    exclude_body_contents: false,
};
const ARENA_SPECTATOR: FreeMovePolicy = FreeMovePolicy {
    wish: FreeWish::ScaledBasis,
    friction: FreeFriction::Linear(5.0),
    motion: FreeMotion::StepSlide,
    acceleration: FreeAcceleration::Flight,
    finish: FreeFinish::Timers,
    abort_without_acceleration: false,
    stance: true,
    spectator_speed: false,
    speed_multiplier: 1.0,
    button_vertical: false,
    world_only: false,
    bounds: None,
    view_height: None,
    reset_flags: false,
    clear_water_outputs: true,
    clear_ground_reference: false,
    exclude_body_contents: true,
};
const FREE_MODES: [FreeModes; 5] = [
    FreeModes {
        flight: FreeMovePolicy {
            motion: FreeMotion::StepSlide,
            ..QUAKE_FREE
        },
        noclip: QUAKE_FREE,
        spectator: QUAKE_FREE,
    },
    FreeModes {
        flight: FreeMovePolicy {
            motion: FreeMotion::StepSlide,
            ..CLASSIC_FREE
        },
        noclip: CLASSIC_FREE,
        spectator: FreeMovePolicy {
            finish: FreeFinish::Snap(false),
            spectator_speed: true,
            ..CLASSIC_FREE
        },
    },
    FreeModes {
        flight: FreeMovePolicy {
            motion: FreeMotion::StepSlide,
            ..CLASSIC_FREE
        },
        noclip: CLASSIC_FREE,
        spectator: FreeMovePolicy {
            finish: FreeFinish::Snap(false),
            clear_water_outputs: true,
            clear_ground_reference: true,
            ..CLASSIC_FREE
        },
    },
    FreeModes {
        flight: FreeMovePolicy {
            motion: FreeMotion::StepSlide,
            abort_without_acceleration: false,
            ..CLASSIC_FREE
        },
        noclip: FreeMovePolicy {
            abort_without_acceleration: false,
            speed_multiplier: 2.0,
            button_vertical: true,
            reset_flags: true,
            clear_water_outputs: true,
            view_height: Some(22.0),
            ..CLASSIC_FREE
        },
        spectator: FreeMovePolicy {
            motion: FreeMotion::StepSlide,
            abort_without_acceleration: false,
            speed_multiplier: 2.0,
            button_vertical: true,
            reset_flags: true,
            clear_water_outputs: true,
            world_only: true,
            bounds: Some(Bounds {
                mins: Vec3([-8.0; 3]),
                maxs: Vec3([8.0; 3]),
            }),
            view_height: Some(0.0),
            ..CLASSIC_FREE
        },
    },
    FreeModes {
        flight: FreeMovePolicy {
            friction: FreeFriction::Flight(3.0),
            finish: FreeFinish::Environment,
            clear_water_outputs: false,
            exclude_body_contents: false,
            ..ARENA_SPECTATOR
        },
        noclip: ARENA_NOCLIP,
        spectator: ARENA_SPECTATOR,
    },
];
#[derive(Clone, Copy)]
pub enum LadderControl {
    PitchVertical,
    ProjectedStrafe,
}
#[derive(Clone, Copy)]
pub struct LadderPolicy {
    pub transient: bool,
    pub ladder_only_mask: bool,
    pub max_water_level: u8,
    pub control: LadderControl,
}
#[derive(Clone, Copy)]
pub enum ProbePhase {
    BeforeTimers,
    InWaterMovement,
}
#[derive(Clone, Copy)]
pub enum WaterJumpProbe {
    Points {
        ahead: f32,
        lower: f32,
        upper: f32,
        down_limit: Option<f32>,
    },
    Ballistic,
}
#[derive(Clone, Copy)]
pub enum JumpTime {
    Milliseconds(u32),
    Seconds(f32),
}
#[derive(Clone, Copy)]
pub enum WaterJumpMotion {
    Existing,
    PreGravity,
    StepGravityThenGravity,
}
#[derive(Clone, Copy)]
pub struct WaterJumpPolicy {
    pub phase: ProbePhase,
    pub probe: WaterJumpProbe,
    pub horizontal: f32,
    pub vertical: f32,
    pub full_forward: bool,
    pub duration: JumpTime,
    pub motion: WaterJumpMotion,
    pub mark_jump_held: bool,
    pub ground_clear: MovementTimer,
}
#[derive(Clone, Copy)]
pub struct SpecialMovement {
    pub ladder: Option<LadderPolicy>,
    pub water_jump: Option<WaterJumpPolicy>,
    pub current_speed: Option<f32>,
    pub swim_jump_impulse: bool,
    pub swim_buttons: bool,
    pub sink_on_ground: bool,
    pub water_standing_cap: bool,
    pub water_duck_cap_after: Option<f32>,
    pub release_jump_below: Option<f32>,
    pub lower_step_probe: bool,
    pub settle_ground_step: bool,
    pub recover_stuck: bool,
    pub wall_ground: bool,
}
const POINT_JUMP: WaterJumpPolicy = WaterJumpPolicy {
    phase: ProbePhase::BeforeTimers,
    probe: WaterJumpProbe::Points {
        ahead: 30.0,
        lower: 4.0,
        upper: 16.0,
        down_limit: None,
    },
    horizontal: 50.0,
    vertical: 350.0,
    full_forward: false,
    duration: JumpTime::Milliseconds(2040),
    motion: WaterJumpMotion::PreGravity,
    mark_jump_held: false,
    ground_clear: MovementTimer(
        MovementTimer::WATER_JUMP.0
            | MovementTimer::LAND.0
            | MovementTimer::TELEPORT.0
            | MovementTimer::KNOCKBACK.0,
    ),
};
const NO_SPECIAL: SpecialMovement = SpecialMovement {
    ladder: None,
    water_jump: None,
    current_speed: None,
    swim_jump_impulse: true,
    swim_buttons: false,
    sink_on_ground: true,
    water_standing_cap: false,
    water_duck_cap_after: None,
    release_jump_below: None,
    lower_step_probe: false,
    settle_ground_step: false,
    recover_stuck: false,
    wall_ground: false,
};
const SPECIAL: [SpecialMovement; 5] = [
    NO_SPECIAL,
    SpecialMovement {
        water_jump: Some(WaterJumpPolicy {
            probe: WaterJumpProbe::Points {
                ahead: 24.0,
                lower: 8.0,
                upper: 24.0,
                down_limit: Some(-180.0),
            },
            vertical: 310.0,
            duration: JumpTime::Seconds(2.0),
            motion: WaterJumpMotion::Existing,
            mark_jump_held: true,
            ..POINT_JUMP
        }),
        ..NO_SPECIAL
    },
    SpecialMovement {
        ladder: Some(LadderPolicy {
            transient: true,
            ladder_only_mask: false,
            max_water_level: 3,
            control: LadderControl::PitchVertical,
        }),
        water_jump: Some(POINT_JUMP),
        current_speed: Some(400.0),
        water_standing_cap: true,
        ..NO_SPECIAL
    },
    SpecialMovement {
        ladder: Some(LadderPolicy {
            transient: false,
            ladder_only_mask: true,
            max_water_level: 1,
            control: LadderControl::ProjectedStrafe,
        }),
        water_jump: Some(WaterJumpPolicy {
            probe: WaterJumpProbe::Ballistic,
            duration: JumpTime::Milliseconds(2048),
            ground_clear: MovementTimer(
                MovementTimer::WATER_JUMP.0
                    | MovementTimer::LAND.0
                    | MovementTimer::TELEPORT.0
                    | MovementTimer::TRICK.0,
            ),
            ..POINT_JUMP
        }),
        current_speed: Some(400.0),
        swim_jump_impulse: false,
        swim_buttons: true,
        sink_on_ground: false,
        water_standing_cap: true,
        water_duck_cap_after: Some(100.0),
        release_jump_below: None,
        lower_step_probe: true,
        settle_ground_step: true,
        recover_stuck: true,
        wall_ground: true,
    },
    SpecialMovement {
        release_jump_below: Some(10.0),
        water_jump: Some(WaterJumpPolicy {
            phase: ProbePhase::InWaterMovement,
            horizontal: 200.0,
            full_forward: true,
            duration: JumpTime::Milliseconds(2000),
            motion: WaterJumpMotion::StepGravityThenGravity,
            ground_clear: MovementTimer(MovementTimer::WATER_JUMP.0 | MovementTimer::LAND.0),
            ..POINT_JUMP
        }),
        ..NO_SPECIAL
    },
];
#[derive(Clone, Copy)]
pub struct FallLanding {
    pub previous_velocity: bool,
    pub threshold: f32,
    pub duration: u32,
    pub fast: Option<(f32, u32)>,
}
#[derive(Clone, Copy)]
pub struct TrickLanding {
    pub threshold: f32,
    pub normal: f32,
    pub duration: u32,
}
#[derive(Clone, Copy)]
pub struct LandingPolicy {
    pub fall: Option<FallLanding>,
    pub trick: Option<TrickLanding>,
    pub duck_duration: Option<u32>,
    pub n64_duration: Option<u32>,
    pub impact: bool,
    pub publish_jump: bool,
    pub recheck_jump: bool,
    pub blocks_jump: bool,
}
const NO_LANDING: LandingPolicy = LandingPolicy {
    fall: None,
    trick: None,
    duck_duration: None,
    n64_duration: None,
    impact: false,
    publish_jump: false,
    recheck_jump: false,
    blocks_jump: false,
};
const LANDING: [LandingPolicy; 5] = [
    NO_LANDING,
    NO_LANDING,
    LandingPolicy {
        blocks_jump: true,
        fall: Some(FallLanding {
            previous_velocity: false,
            threshold: -200.0,
            duration: 144,
            fast: Some((-400.0, 200)),
        }),
        ..NO_LANDING
    },
    LandingPolicy {
        trick: Some(TrickLanding {
            threshold: 100.0,
            normal: 0.9,
            duration: 64,
        }),
        duck_duration: Some(128),
        n64_duration: Some(128),
        impact: true,
        publish_jump: true,
        recheck_jump: true,
        blocks_jump: true,
        ..NO_LANDING
    },
    LandingPolicy {
        fall: Some(FallLanding {
            previous_velocity: true,
            threshold: -200.0,
            duration: 250,
            fast: None,
        }),
        ..NO_LANDING
    },
];
#[derive(Clone, Copy)]
pub struct StancePolicy {
    pub gib_bounds: bool,
    pub dead_duck: bool,
    pub grounded_duck: bool,
    pub after_ground: bool,
    pub button_only: bool,
    pub above_water: bool,
    pub forbid_ladder: bool,
    pub forbid_n64: bool,
    pub check_duck_hull: bool,
    pub initial_snap: Option<f32>,
}
const NATIVE_STANCE: StancePolicy = StancePolicy {
    gib_bounds: false,
    dead_duck: false,
    grounded_duck: false,
    after_ground: false,
    button_only: false,
    above_water: false,
    forbid_ladder: false,
    forbid_n64: false,
    check_duck_hull: false,
    initial_snap: None,
};
const STANCE: [StancePolicy; 5] = [
    NATIVE_STANCE,
    NATIVE_STANCE,
    StancePolicy {
        gib_bounds: true,
        dead_duck: true,
        grounded_duck: true,
        initial_snap: Some(0.125),
        ..NATIVE_STANCE
    },
    StancePolicy {
        gib_bounds: true,
        dead_duck: true,
        grounded_duck: true,
        after_ground: true,
        button_only: true,
        above_water: true,
        forbid_ladder: true,
        forbid_n64: true,
        check_duck_hull: true,
        initial_snap: Some(1.0),
    },
    NATIVE_STANCE,
];
#[derive(Clone, Copy)]
pub struct Capabilities {
    pub free: FreeModes,
    pub control: Option<FreeMovePolicy>,
    pub relative_command_clock: bool,
    pub special: SpecialMovement,
    pub landing: LandingPolicy,
    pub stance: StancePolicy,
    pub view_angles: ViewAnglePolicies,
    pub dead_friction: Option<f32>,
}
pub fn capabilities(rules: RuleSetId) -> Capabilities {
    Capabilities {
        free: FREE_MODES[rules as usize],
        control: [
            Some(FreeMovePolicy {
                wish: FreeWish::NativeControl,
                ..QUAKE_FREE
            }),
            None,
            None,
            None,
            None,
        ][rules as usize],
        relative_command_clock: [false, false, false, false, true][rules as usize],
        special: SPECIAL[rules as usize],
        landing: LANDING[rules as usize],
        stance: STANCE[rules as usize],
        view_angles: VIEW_ANGLES[rules as usize],
        dead_friction: [None, None, Some(20.0), Some(20.0), Some(20.0)][rules as usize],
    }
}
