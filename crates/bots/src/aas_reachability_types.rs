//! Source AAS settings and the reachability construction contract,
//! ported from `src/bots/navigation/aas-reachability-types.ts`.

use qa_core::math::Vec3;

use crate::aas::{aas_point_area, AasAreaSettings, AasAsset};
use crate::error::{indexed, BotsError};

/// AAS client-movement stop events.
pub struct AasStopEvent;

impl AasStopEvent {
    /// No event.
    pub const NONE: i32 = 0;
    /// Hit ground.
    pub const HIT_GROUND: i32 = 1;
    /// Left ground.
    pub const LEAVE_GROUND: i32 = 2;
    /// Entered water.
    pub const ENTER_WATER: i32 = 4;
    /// Entered slime.
    pub const ENTER_SLIME: i32 = 8;
    /// Entered lava.
    pub const ENTER_LAVA: i32 = 16;
    /// Hit ground with damage.
    pub const HIT_GROUND_DAMAGE: i32 = 32;
    /// Gap.
    pub const GAP: i32 = 64;
    /// Touched a jump pad.
    pub const TOUCH_JUMP_PAD: i32 = 128;
    /// Touched a teleporter.
    pub const TOUCH_TELEPORTER: i32 = 256;
    /// Entered an area.
    pub const ENTER_AREA: i32 = 512;
    /// Hit a ground area.
    pub const HIT_GROUND_AREA: i32 = 1024;
    /// Hit the bounding box.
    pub const HIT_BOUNDING_BOX: i32 = 2048;
    /// Touched a cluster portal.
    pub const TOUCH_CLUSTER_PORTAL: i32 = 4096;
}

/// Source AAS movement settings. The source static `aassettings` record
/// survives AAS world replacement and shutdown.
#[derive(Debug, Clone, PartialEq)]
pub struct AasMovementSettings {
    /// Gravity direction.
    pub gravity_direction: Vec3,
    /// Friction.
    pub friction: f64,
    /// Stop speed.
    pub stop_speed: f64,
    /// Gravity.
    pub gravity: f64,
    /// Water friction.
    pub water_friction: f64,
    /// Water gravity.
    pub water_gravity: f64,
    /// Maximum velocity.
    pub max_velocity: f64,
    /// Maximum walk velocity.
    pub max_walk_velocity: f64,
    /// Maximum crouch velocity.
    pub max_crouch_velocity: f64,
    /// Maximum swim velocity.
    pub max_swim_velocity: f64,
    /// Walk acceleration.
    pub walk_accelerate: f64,
    /// Air acceleration.
    pub air_accelerate: f64,
    /// Swim acceleration.
    pub swim_accelerate: f64,
    /// Maximum step.
    pub max_step: f64,
    /// Maximum steepness.
    pub max_steepness: f64,
    /// Maximum water jump.
    pub max_water_jump: f64,
    /// Maximum barrier.
    pub max_barrier: f64,
    /// Jump velocity.
    pub jump_velocity: f64,
    /// Fall delta for 5 damage.
    pub fall_delta5: f64,
    /// Fall delta for 10 damage.
    pub fall_delta10: f64,
    /// Water-jump time.
    pub water_jump_time: f64,
    /// Teleport time.
    pub teleport_time: f64,
    /// Barrier-jump time.
    pub barrier_jump_time: f64,
    /// Start-crouch time.
    pub start_crouch_time: f64,
    /// Start-grapple time.
    pub start_grapple_time: f64,
    /// Start walk-off-ledge time.
    pub start_walk_off_ledge_time: f64,
    /// Start-jump time.
    pub start_jump_time: f64,
    /// Rocket-jump time.
    pub rocket_jump_time: f64,
    /// BFG-jump time.
    pub bfg_jump_time: f64,
    /// Jump-pad time.
    pub jump_pad_time: f64,
    /// Air-controlled jump-pad time.
    pub air_controlled_jump_pad_time: f64,
    /// Func-bob time.
    pub func_bob_time: f64,
    /// Start-elevator time.
    pub start_elevator_time: f64,
    /// Fall-damage-5 time.
    pub fall_damage5_time: f64,
    /// Fall-damage-10 time.
    pub fall_damage10_time: f64,
    /// Maximum fall height.
    pub max_fall_height: f64,
    /// Maximum jump-fall height.
    pub max_jump_fall_height: f64,
}

impl Default for AasMovementSettings {
    fn default() -> Self {
        Self {
            gravity_direction: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            friction: 0.0,
            stop_speed: 0.0,
            gravity: 0.0,
            water_friction: 0.0,
            water_gravity: 0.0,
            max_velocity: 0.0,
            max_walk_velocity: 0.0,
            max_crouch_velocity: 0.0,
            max_swim_velocity: 0.0,
            walk_accelerate: 0.0,
            air_accelerate: 0.0,
            swim_accelerate: 0.0,
            max_step: 0.0,
            max_steepness: 0.0,
            max_water_jump: 0.0,
            max_barrier: 0.0,
            jump_velocity: 0.0,
            fall_delta5: 0.0,
            fall_delta10: 0.0,
            water_jump_time: 0.0,
            teleport_time: 0.0,
            barrier_jump_time: 0.0,
            start_crouch_time: 0.0,
            start_grapple_time: 0.0,
            start_walk_off_ledge_time: 0.0,
            start_jump_time: 0.0,
            rocket_jump_time: 0.0,
            bfg_jump_time: 0.0,
            jump_pad_time: 0.0,
            air_controlled_jump_pad_time: 0.0,
            func_bob_time: 0.0,
            start_elevator_time: 0.0,
            fall_damage5_time: 0.0,
            fall_damage10_time: 0.0,
            max_fall_height: 0.0,
            max_jump_fall_height: 0.0,
        }
    }
}

/// Source library-variable reader: `AAS_InitSettings` reads each variable
/// once, in source order; later writes need another init.
pub trait AasLibVarValue {
    /// Read a variable with its source default text.
    fn value(&self, name: &str, default: &str) -> f64;
}

impl<F: Fn(&str, &str) -> f64> AasLibVarValue for F {
    fn value(&self, name: &str, default: &str) -> f64 {
        self(name, default)
    }
}

/// Initialize movement settings from library variables, in source order.
pub fn init_aas_movement_settings(value: &dyn AasLibVarValue, settings: &mut AasMovementSettings) {
    let read = |name: &str, initial: &str| f64::from(value.value(name, initial) as f32);
    settings.gravity_direction = Vec3 {
        x: 0.0,
        y: 0.0,
        z: -1.0,
    };
    settings.friction = read("phys_friction", "6");
    settings.stop_speed = read("phys_stopspeed", "100");
    settings.gravity = read("phys_gravity", "800");
    settings.water_friction = read("phys_waterfriction", "1");
    settings.water_gravity = read("phys_watergravity", "400");
    settings.max_velocity = read("phys_maxvelocity", "320");
    settings.max_walk_velocity = read("phys_maxwalkvelocity", "320");
    settings.max_crouch_velocity = read("phys_maxcrouchvelocity", "100");
    settings.max_swim_velocity = read("phys_maxswimvelocity", "150");
    settings.walk_accelerate = read("phys_walkaccelerate", "10");
    settings.air_accelerate = read("phys_airaccelerate", "1");
    settings.swim_accelerate = read("phys_swimaccelerate", "4");
    settings.max_step = read("phys_maxstep", "19");
    settings.max_steepness = read("phys_maxsteepness", "0.7");
    settings.max_water_jump = read("phys_maxwaterjump", "18");
    settings.max_barrier = read("phys_maxbarrier", "33");
    settings.jump_velocity = read("phys_jumpvel", "270");
    settings.fall_delta5 = read("phys_falldelta5", "40");
    settings.fall_delta10 = read("phys_falldelta10", "60");
    settings.water_jump_time = read("rs_waterjump", "400");
    settings.teleport_time = read("rs_teleport", "50");
    settings.barrier_jump_time = read("rs_barrierjump", "100");
    settings.start_crouch_time = read("rs_startcrouch", "300");
    settings.start_grapple_time = read("rs_startgrapple", "500");
    settings.start_walk_off_ledge_time = read("rs_startwalkoffledge", "70");
    settings.start_jump_time = read("rs_startjump", "300");
    settings.rocket_jump_time = read("rs_rocketjump", "500");
    settings.bfg_jump_time = read("rs_bfgjump", "500");
    settings.jump_pad_time = read("rs_jumppad", "250");
    settings.air_controlled_jump_pad_time = read("rs_aircontrolledjumppad", "300");
    settings.func_bob_time = read("rs_funcbob", "300");
    settings.start_elevator_time = read("rs_startelevator", "50");
    settings.fall_damage5_time = read("rs_falldamage5", "300");
    settings.fall_damage10_time = read("rs_falldamage10", "500");
    settings.max_fall_height = read("rs_maxfallheight", "0");
    settings.max_jump_fall_height = read("rs_maxjumpfallheight", "450");
}

/// Source initial values for controlled fixtures; production calls init
/// with the actual variable owner.
#[must_use]
pub fn default_aas_movement_settings() -> AasMovementSettings {
    struct Defaults;
    impl AasLibVarValue for Defaults {
        fn value(&self, _name: &str, default: &str) -> f64 {
            default.parse().unwrap_or(0.0)
        }
    }
    let mut settings = AasMovementSettings::default();
    init_aas_movement_settings(&Defaults, &mut settings);
    settings
}

/// AAS world under reachability construction: immutable source geometry
/// with mutable per-area settings.
#[derive(Debug, Clone, PartialEq)]
pub struct AasReachabilityWorld {
    /// Source asset geometry.
    pub asset: AasAsset,
    /// Mutable per-area settings.
    pub settings: Vec<AasAreaSettings>,
}

impl AasReachabilityWorld {
    /// Area containing a point.
    pub fn point_area(&self, point: Vec3) -> Result<i32, BotsError> {
        aas_point_area(&self.asset, point)
    }

    /// Borrow area settings.
    pub fn setting(&self, area: i32) -> Result<&AasAreaSettings, BotsError> {
        indexed(&self.settings, i64::from(area), "AAS reachability index")
    }

    /// Mutably borrow area settings.
    pub fn setting_mut(&mut self, area: i32) -> Result<&mut AasAreaSettings, BotsError> {
        crate::error::indexed_mut(&mut self.settings, i64::from(area), "AAS reachability index")
    }
}

/// Predicted client move for reachability construction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AasClientMove {
    /// End position.
    pub end: Vec3,
    /// End velocity.
    pub velocity: Vec3,
    /// End area.
    pub end_area: i32,
    /// Simulated frames.
    pub frames: i32,
    /// Stop-event bits.
    pub stop_event: i32,
    /// Simulated seconds.
    pub time: f32,
}
