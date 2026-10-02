//! Unified admitted-player prediction projection codec.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/network/unified-prediction.ts`
//! (`encodeUnifiedPrediction`, `decodeUnifiedPrediction`,
//! `projectUnifiedPrediction`).
//!
//! Admitted-player movement only: no save image, weapon VM state, or other
//! player's inventory. The projection carries the movement snapshot plus the
//! linked collision actors the replica needs to replay pending input.
//! Envelope framing reuses the [`qa_world::save`] checkpoint value codec;
//! movement, arsenal, animation, profile, and collision shapes are local
//! mirrors carrying exactly the fields the codec reads and writes (the
//! shared `qa-world` movement states keep their fields private to their
//! crate). [`UnifiedPredictionSource`] abstracts the authoritative
//! simulation the projection reads from.

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3};
use qa_core::time::{ClockProfile, SourceTime};
use qa_world::inventory::InventoryEntry;
use qa_world::save::records::{read_inventory_entry, write_inventory_entry};
use qa_world::save::shared::{
    read_bounds, read_clock, read_numeric, read_time, write_bounds, write_clock, write_numeric, write_time,
    SavedNumericProfile,
};
use qa_world::save::value::{
    arr, boolean, decode_checkpoint_value, encode_checkpoint_value, int, namespaced, num, obj, str as json_str,
    SaveJson, SaveReader,
};
use qa_world::WorldError;

use super::unified_frame_values::{read_actor, read_vector, wire_actor, write_vector};
use super::unified_types::UnifiedIdentityDecoder;

/// Prediction codec failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UnifiedPredictionError {
    /// Checkpoint value failure.
    #[error(transparent)]
    World(#[from] WorldError),
    /// Projection source failure.
    #[error("{0}")]
    Source(String),
}

/// Trace hit (donor `TraceHit`).
#[derive(Debug, Clone, PartialEq)]
pub enum UnifiedTraceHit {
    /// No hit.
    None,
    /// World geometry hit.
    World {
        /// Model index.
        model: i64,
    },
    /// Actor hit.
    Actor {
        /// Hit actor.
        actor: ActorId,
    },
}

/// NetQuake movement state (donor `MovementState` `q1-netquake`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1NetquakeMovement {
    /// Origin.
    pub origin: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Angles.
    pub angles: Vec3,
    /// Previous origin.
    pub old_origin: Vec3,
    /// Angular velocity.
    pub angular_velocity: Vec3,
    /// View angles.
    pub view_angles: Vec3,
    /// Punch angles.
    pub punch_angles: Vec3,
    /// Move type.
    pub move_type: f64,
    /// Flags.
    pub flags: f64,
    /// Ground hit.
    pub ground: UnifiedTraceHit,
    /// Water level.
    pub water_level: f64,
    /// Water type.
    pub water_type: f64,
    /// Teleport time in seconds.
    pub teleport_time_seconds: f64,
    /// Water-jump direction.
    pub water_jump_direction: Vec3,
    /// Ideal pitch.
    pub ideal_pitch: f64,
    /// Fix-angle flag.
    pub fix_angle: bool,
    /// Health.
    pub health: f64,
}

/// QuakeWorld movement state (donor `MovementState` `q1-quakeworld`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1QuakeworldMovement {
    /// Origin.
    pub origin: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Angles.
    pub angles: Vec3,
    /// Previous buttons.
    pub old_buttons: f64,
    /// Water-jump time in seconds.
    pub water_jump_time_seconds: f64,
    /// Dead flag.
    pub dead: bool,
    /// Spectator flag.
    pub spectator: f64,
    /// Ground hit.
    pub ground: UnifiedTraceHit,
}

/// Classic Q2 movement state (donor `MovementState` `q2-classic`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2ClassicMovement {
    /// Move type.
    pub move_type: f64,
    /// Origin in eighths.
    pub origin_eighths: [f64; 3],
    /// Velocity in eighths.
    pub velocity_eighths: [f64; 3],
    /// Flags.
    pub flags: f64,
    /// Time in eight-millisecond units.
    pub time_eight_milliseconds: f64,
    /// Gravity.
    pub gravity: f64,
    /// Delta angle shorts.
    pub delta_angle_shorts: [f64; 3],
}

/// Rerelease Q2 movement state (donor `MovementState` `q2-rerelease`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2RereleaseMovement {
    /// Move type.
    pub move_type: f64,
    /// Origin.
    pub origin: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Flags.
    pub flags: f64,
    /// Time in milliseconds.
    pub time_milliseconds: f64,
    /// Gravity.
    pub gravity: f64,
    /// Delta angles.
    pub delta_angles: Vec3,
    /// View height.
    pub view_height: f64,
}

/// Q3 movement state (donor `MovementState` `q3`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3Movement {
    /// Command time in milliseconds.
    pub command_time_milliseconds: f64,
    /// Movement type.
    pub movement_type: f64,
    /// Bob cycle.
    pub bob_cycle: f64,
    /// Movement flags.
    pub movement_flags: f64,
    /// Movement time in milliseconds.
    pub movement_time_milliseconds: f64,
    /// Origin.
    pub origin: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Gravity.
    pub gravity: f64,
    /// Speed.
    pub speed: f64,
    /// Delta angle words.
    pub delta_angle_words: [f64; 3],
    /// Movement direction.
    pub movement_direction: f64,
    /// Grapple point.
    pub grapple_point: Vec3,
    /// Flags.
    pub flags: f64,
    /// View angles.
    pub view_angles: Vec3,
    /// View height.
    pub view_height: f64,
    /// Ground hit.
    pub ground: UnifiedTraceHit,
    /// Predictable event sequence.
    pub predictable_event_sequence: f64,
    /// Jump-pad actor.
    pub jump_pad: Option<ActorId>,
    /// Movement frame.
    pub movement_frame: f64,
    /// Jump-pad frame.
    pub jump_pad_frame: f64,
}

/// Movement state (donor `MovementState`).
#[derive(Debug, Clone, PartialEq)]
pub enum UnifiedMovementState {
    /// NetQuake state.
    Q1Netquake(Q1NetquakeMovement),
    /// QuakeWorld state.
    Q1Quakeworld(Q1QuakeworldMovement),
    /// Classic Q2 state.
    Q2Classic(Q2ClassicMovement),
    /// Rerelease Q2 state.
    Q2Rerelease(Q2RereleaseMovement),
    /// Q3 state.
    Q3(Q3Movement),
}

impl UnifiedMovementState {
    fn kind(&self) -> &'static str {
        match self {
            Self::Q1Netquake(_) => "q1-netquake",
            Self::Q1Quakeworld(_) => "q1-quakeworld",
            Self::Q2Classic(_) => "q2-classic",
            Self::Q2Rerelease(_) => "q2-rerelease",
            Self::Q3(_) => "q3",
        }
    }
}

/// Q1 movement parameters (donor profile `parameters`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1MovementParameters {
    /// Gravity.
    pub gravity: f64,
    /// Stop speed.
    pub stop_speed: f64,
    /// Maximum speed.
    pub max_speed: f64,
    /// Spectator maximum speed.
    pub spectator_max_speed: f64,
    /// Acceleration.
    pub accelerate: f64,
    /// Air acceleration.
    pub air_accelerate: f64,
    /// Water acceleration.
    pub water_accelerate: f64,
    /// Friction.
    pub friction: f64,
    /// Water friction.
    pub water_friction: f64,
    /// Entity gravity.
    pub entity_gravity: f64,
}

/// NetQuake edition (donor profile `edition`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1NetquakeEdition {
    /// Classic.
    Classic,
    /// Rerelease.
    Rerelease,
    /// Quake 64.
    Quake64,
}

/// Movement profile kind (donor `MovementProfile` by `kind`).
#[derive(Debug, Clone, PartialEq)]
pub enum UnifiedProfileKind {
    /// NetQuake profile.
    Q1Netquake {
        /// Parameters.
        parameters: Q1MovementParameters,
        /// Edition.
        edition: Q1NetquakeEdition,
        /// Edge friction.
        edge_friction: f64,
        /// Noclip angle hack.
        no_clip_angle_hack: bool,
    },
    /// QuakeWorld profile.
    Q1Quakeworld {
        /// Parameters.
        parameters: Q1MovementParameters,
    },
    /// Classic Q2 profile.
    Q2Classic {
        /// Air acceleration.
        air_accelerate: f64,
        /// Snap-initial flag.
        snap_initial: bool,
        /// Strafe-jump hack.
        strafejump_hack: Option<bool>,
    },
    /// Rerelease Q2 profile.
    Q2Rerelease {
        /// Air acceleration.
        air_accelerate: f64,
        /// N64 physics flag.
        n64_physics: bool,
    },
    /// Q3 profile.
    Q3 {
        /// Product.
        product: Q3ProfileProduct,
        /// Fixed step in milliseconds.
        fixed_milliseconds: Option<f64>,
        /// No-footsteps flag.
        no_footsteps: bool,
    },
}

/// Q3 profile product.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3ProfileProduct {
    /// Base Q3.
    BaseQ3,
    /// Mission pack.
    MissionPack,
}

/// Movement profile (donor `MovementProfile`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedMovementProfile {
    /// Profile id.
    pub id: String,
    /// Frame clock.
    pub clock: ClockProfile,
    /// Numeric profile.
    pub numeric: SavedNumericProfile,
    /// Profile kind.
    pub kind: UnifiedProfileKind,
}

impl UnifiedMovementProfile {
    fn kind_text(&self) -> &'static str {
        match &self.kind {
            UnifiedProfileKind::Q1Netquake { .. } => "q1-netquake",
            UnifiedProfileKind::Q1Quakeworld { .. } => "q1-quakeworld",
            UnifiedProfileKind::Q2Classic { .. } => "q2-classic",
            UnifiedProfileKind::Q2Rerelease { .. } => "q2-rerelease",
            UnifiedProfileKind::Q3 { .. } => "q3",
        }
    }
}

/// Weapon state (donor `WeaponState`).
#[derive(Debug, Clone, PartialEq)]
pub enum UnifiedWeaponState {
    /// Q1 weapon frame state.
    Q1 {
        /// Current frame.
        frame: f64,
        /// Attack-finished time in seconds.
        attack_finished_seconds: f64,
        /// Source weapon number.
        source_weapon: f64,
    },
    /// Q2 weapon frame state.
    Q2 {
        /// Gun frame.
        gun_frame: f64,
        /// Weapon state word.
        state: f64,
        /// Pending weapon.
        pending_weapon: Option<String>,
        /// Machinegun shot count.
        machinegun_shots: f64,
        /// Grenade timer.
        grenade_time: SourceTime,
        /// Grenade blew up flag.
        grenade_blew_up: bool,
    },
    /// Q3 weapon state.
    Q3 {
        /// Source weapon number.
        source_weapon: f64,
        /// Weapon state word.
        state: f64,
        /// Weapon timer in milliseconds.
        time_milliseconds: f64,
    },
}

/// Arsenal snapshot (donor `ArsenalState`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedArsenalState {
    /// Owning provider.
    pub provider: String,
    /// Active weapon.
    pub active_weapon: Option<String>,
    /// Weapon state.
    pub state: UnifiedWeaponState,
    /// Ammo counters.
    pub ammo: Vec<InventoryEntry>,
}

/// Animation state (donor `AnimationState`).
#[derive(Debug, Clone, PartialEq)]
pub enum UnifiedAnimationState {
    /// Q1 animation frame state.
    Q1 {
        /// Current frame.
        frame: f64,
        /// Next frame time in seconds.
        next_frame_seconds: f64,
    },
    /// Q2 animation frame state.
    Q2 {
        /// Current frame.
        frame: f64,
        /// End frame.
        end_frame: f64,
        /// Animation priority.
        priority: f64,
        /// Duck flag.
        duck: bool,
        /// Run flag.
        run: bool,
    },
    /// Q3 animation state.
    Q3 {
        /// Legs animation.
        legs: f64,
        /// Torso animation.
        torso: f64,
        /// Legs timer in milliseconds.
        legs_timer_milliseconds: f64,
        /// Torso timer in milliseconds.
        torso_timer_milliseconds: f64,
    },
}

/// Actor animation snapshot (donor `ActorAnimationState`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedActorAnimation {
    /// Owning provider.
    pub provider: String,
    /// Animation state.
    pub state: UnifiedAnimationState,
}

/// Client movement mode (donor `ModClientMovementOutputs['mode']`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnifiedClientMovementMode {
    /// Normal movement.
    Normal,
    /// Noclip movement.
    Noclip,
    /// Frozen.
    Freeze,
}

/// Client movement outputs (donor `ModClientMovementOutputs`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedClientOutputs {
    /// Requested view offset.
    pub view_offset: Option<Vec3>,
    /// Requested movement mode.
    pub mode: Option<UnifiedClientMovementMode>,
    /// Requested crouch stance.
    pub stance: Option<bool>,
    /// Requested body bounds.
    pub body_bounds: Option<Bounds>,
}

/// Prediction environment (donor `MovementPredictionSnapshot['environment']`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedPredictionEnvironment {
    /// Health.
    pub health: f64,
    /// Flight flag.
    pub flight: bool,
    /// Haste flag.
    pub haste: bool,
    /// Invulnerability flag.
    pub invulnerable: bool,
    /// Gravity multiplier.
    pub gravity_multiplier: f64,
    /// Client outputs.
    pub client_outputs: Option<UnifiedClientOutputs>,
}

/// Prediction contact (donor snapshot `contact`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedPredictionContact {
    /// Ground hit.
    pub ground: UnifiedTraceHit,
    /// Water level.
    pub water_level: i64,
    /// Water type.
    pub water_type: i64,
}

/// Linked collision body (donor `SpatialActor['body']`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedLinkedBody {
    /// Actor handle.
    pub actor: ActorId,
    /// Body state.
    pub state: UnifiedLinkedBodyState,
    /// Link count.
    pub link_count: i64,
    /// Absolute bounds.
    pub absolute_bounds: Bounds,
}

/// Linked body state.
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedLinkedBodyState {
    /// Origin.
    pub origin: Vec3,
    /// Angles.
    pub angles: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Local bounds.
    pub bounds: Bounds,
    /// Ground actor.
    pub ground: Option<ActorId>,
}

/// Collision shape (donor collision `shape`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnifiedCollisionShape {
    /// Box shape.
    Box,
    /// Capsule shape.
    Capsule,
    /// Brush model shape.
    Model {
        /// Model number.
        model: i64,
    },
}

/// Collision role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnifiedCollisionRole {
    /// Solid.
    Solid,
    /// Trigger.
    Trigger,
}

/// Q3 owner pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnifiedQ3Owner {
    /// Entity number.
    pub entity_number: i64,
    /// Owner number.
    pub owner_number: i64,
}

/// Linked collision record (donor `SpatialActor['collision']`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedCollisionRecord {
    /// Source family.
    pub family: UnifiedCollisionFamily,
    /// Shape.
    pub shape: UnifiedCollisionShape,
    /// Contents mask.
    pub contents: i64,
    /// Owner actor.
    pub owner: Option<ActorId>,
    /// Collision role.
    pub role: UnifiedCollisionRole,
    /// Monster flag.
    pub monster: bool,
    /// Dead-monster flag.
    pub dead_monster: bool,
    /// Q1 corpse marker.
    pub q1_corpse: Option<bool>,
    /// Q3 owner pair.
    pub q3_owner: Option<UnifiedQ3Owner>,
}

/// Collision source family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnifiedCollisionFamily {
    /// Quake.
    Q1,
    /// Quake II.
    Q2,
    /// Quake III.
    Q3,
}

/// Linked collision actor (donor `SpatialActor`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedSpatialActor {
    /// Linked body.
    pub body: UnifiedLinkedBody,
    /// Collision record.
    pub collision: UnifiedCollisionRecord,
}

/// Admitted-player prediction projection (donor `UnifiedPredictionProjection`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedPredictionProjection {
    /// Admitted actor.
    pub actor: ActorId,
    /// Acknowledged input sequence.
    pub sequence: i64,
    /// Command time in milliseconds.
    pub command_time_milliseconds: f64,
    /// Movement state.
    pub state: UnifiedMovementState,
    /// Movement profile.
    pub profile: UnifiedMovementProfile,
    /// Arsenal snapshot.
    pub arsenal: UnifiedArsenalState,
    /// Animation snapshot.
    pub animation: UnifiedActorAnimation,
    /// Standing bounds.
    pub standing_bounds: Bounds,
    /// Standing view height.
    pub standing_view_height: f64,
    /// Body bounds.
    pub bounds: Bounds,
    /// View angles.
    pub view_angles: Vec3,
    /// View height.
    pub view_height: f64,
    /// View offset.
    pub view_offset: Vec3,
    /// Environment.
    pub environment: UnifiedPredictionEnvironment,
    /// Contact.
    pub contact: Option<UnifiedPredictionContact>,
    /// Linked collision actors.
    pub collisions: Vec<UnifiedSpatialActor>,
}

fn write_hit(hit: &UnifiedTraceHit) -> SaveJson {
    match hit {
        UnifiedTraceHit::None => obj(vec![("kind", json_str("none"))]),
        UnifiedTraceHit::World { model } => obj(vec![("kind", json_str("world")), ("model", int(*model))]),
        UnifiedTraceHit::Actor { actor } => obj(vec![("kind", json_str("actor")), ("actor", wire_actor(actor))]),
    }
}

fn read_hit(reader: SaveReader, identity: &dyn UnifiedIdentityDecoder) -> Result<UnifiedTraceHit, WorldError> {
    match reader.field("kind").choice_str(&["none", "world", "actor"])?.as_str() {
        "world" => Ok(UnifiedTraceHit::World {
            model: reader.field("model").integer(0)?,
        }),
        "actor" => Ok(UnifiedTraceHit::Actor {
            actor: read_actor(reader.field("actor"), identity)?,
        }),
        _ => Ok(UnifiedTraceHit::None),
    }
}

fn read_triple(reader: SaveReader) -> Result<[f64; 3], WorldError> {
    let values = reader.list(|value| value.finite())?;
    if values.len() != 3 {
        return Err(reader.fail("expected three numbers"));
    }
    Ok([values[0], values[1], values[2]])
}

fn write_triple(values: [f64; 3]) -> SaveJson {
    arr(vec![num(values[0]), num(values[1]), num(values[2])])
}

fn read_movement(
    reader: SaveReader,
    identity: &dyn UnifiedIdentityDecoder,
) -> Result<UnifiedMovementState, WorldError> {
    let kind =
        reader
            .field("kind")
            .choice_str(&["q1-netquake", "q1-quakeworld", "q2-classic", "q2-rerelease", "q3"])?;
    match kind.as_str() {
        "q1-netquake" => Ok(UnifiedMovementState::Q1Netquake(Q1NetquakeMovement {
            origin: read_vector(reader.field("origin"))?,
            velocity: read_vector(reader.field("velocity"))?,
            angles: read_vector(reader.field("angles"))?,
            old_origin: read_vector(reader.field("oldOrigin"))?,
            angular_velocity: read_vector(reader.field("angularVelocity"))?,
            view_angles: read_vector(reader.field("viewAngles"))?,
            punch_angles: read_vector(reader.field("punchAngles"))?,
            move_type: reader.field("moveType").finite()?,
            flags: reader.field("flags").finite()?,
            ground: read_hit(reader.field("ground"), identity)?,
            water_level: reader.field("waterLevel").finite()?,
            water_type: reader.field("waterType").finite()?,
            teleport_time_seconds: reader.field("teleportTimeSeconds").finite()?,
            water_jump_direction: read_vector(reader.field("waterJumpDirection"))?,
            ideal_pitch: reader.field("idealPitch").finite()?,
            fix_angle: reader.field("fixAngle").boolean()?,
            health: reader.field("health").finite()?,
        })),
        "q1-quakeworld" => Ok(UnifiedMovementState::Q1Quakeworld(Q1QuakeworldMovement {
            origin: read_vector(reader.field("origin"))?,
            velocity: read_vector(reader.field("velocity"))?,
            angles: read_vector(reader.field("angles"))?,
            old_buttons: reader.field("oldButtons").finite()?,
            water_jump_time_seconds: reader.field("waterJumpTimeSeconds").finite()?,
            dead: reader.field("dead").boolean()?,
            spectator: reader.field("spectator").finite()?,
            ground: read_hit(reader.field("ground"), identity)?,
        })),
        "q2-classic" => Ok(UnifiedMovementState::Q2Classic(Q2ClassicMovement {
            move_type: reader.field("type").finite()?,
            origin_eighths: read_triple(reader.field("originEighths"))?,
            velocity_eighths: read_triple(reader.field("velocityEighths"))?,
            flags: reader.field("flags").finite()?,
            time_eight_milliseconds: reader.field("timeEightMilliseconds").finite()?,
            gravity: reader.field("gravity").finite()?,
            delta_angle_shorts: read_triple(reader.field("deltaAngleShorts"))?,
        })),
        "q2-rerelease" => Ok(UnifiedMovementState::Q2Rerelease(Q2RereleaseMovement {
            move_type: reader.field("type").finite()?,
            origin: read_vector(reader.field("origin"))?,
            velocity: read_vector(reader.field("velocity"))?,
            flags: reader.field("flags").finite()?,
            time_milliseconds: reader.field("timeMilliseconds").finite()?,
            gravity: reader.field("gravity").finite()?,
            delta_angles: read_vector(reader.field("deltaAngles"))?,
            view_height: reader.field("viewHeight").finite()?,
        })),
        _ => Ok(UnifiedMovementState::Q3(Q3Movement {
            command_time_milliseconds: reader.field("commandTimeMilliseconds").finite()?,
            movement_type: reader.field("movementType").finite()?,
            bob_cycle: reader.field("bobCycle").finite()?,
            movement_flags: reader.field("movementFlags").finite()?,
            movement_time_milliseconds: reader.field("movementTimeMilliseconds").finite()?,
            origin: read_vector(reader.field("origin"))?,
            velocity: read_vector(reader.field("velocity"))?,
            gravity: reader.field("gravity").finite()?,
            speed: reader.field("speed").finite()?,
            delta_angle_words: read_triple(reader.field("deltaAngleWords"))?,
            movement_direction: reader.field("movementDirection").finite()?,
            grapple_point: read_vector(reader.field("grapplePoint"))?,
            flags: reader.field("flags").finite()?,
            view_angles: read_vector(reader.field("viewAngles"))?,
            view_height: reader.field("viewHeight").finite()?,
            ground: read_hit(reader.field("ground"), identity)?,
            predictable_event_sequence: reader.field("predictableEventSequence").finite()?,
            jump_pad: reader.field("jumpPad").nullable(|pad| read_actor(pad, identity))?,
            movement_frame: reader.field("movementFrame").finite()?,
            jump_pad_frame: reader.field("jumpPadFrame").finite()?,
        })),
    }
}

fn write_movement(state: &UnifiedMovementState) -> SaveJson {
    match state {
        UnifiedMovementState::Q1Netquake(state) => obj(vec![
            ("kind", json_str("q1-netquake")),
            ("origin", write_vector(state.origin)),
            ("velocity", write_vector(state.velocity)),
            ("angles", write_vector(state.angles)),
            ("oldOrigin", write_vector(state.old_origin)),
            ("angularVelocity", write_vector(state.angular_velocity)),
            ("viewAngles", write_vector(state.view_angles)),
            ("punchAngles", write_vector(state.punch_angles)),
            ("moveType", num(state.move_type)),
            ("flags", num(state.flags)),
            ("ground", write_hit(&state.ground)),
            ("waterLevel", num(state.water_level)),
            ("waterType", num(state.water_type)),
            ("teleportTimeSeconds", num(state.teleport_time_seconds)),
            ("waterJumpDirection", write_vector(state.water_jump_direction)),
            ("idealPitch", num(state.ideal_pitch)),
            ("fixAngle", boolean(state.fix_angle)),
            ("health", num(state.health)),
        ]),
        UnifiedMovementState::Q1Quakeworld(state) => obj(vec![
            ("kind", json_str("q1-quakeworld")),
            ("origin", write_vector(state.origin)),
            ("velocity", write_vector(state.velocity)),
            ("angles", write_vector(state.angles)),
            ("oldButtons", num(state.old_buttons)),
            ("waterJumpTimeSeconds", num(state.water_jump_time_seconds)),
            ("dead", boolean(state.dead)),
            ("spectator", num(state.spectator)),
            ("ground", write_hit(&state.ground)),
        ]),
        UnifiedMovementState::Q2Classic(state) => obj(vec![
            ("kind", json_str("q2-classic")),
            ("type", num(state.move_type)),
            ("originEighths", write_triple(state.origin_eighths)),
            ("velocityEighths", write_triple(state.velocity_eighths)),
            ("flags", num(state.flags)),
            ("timeEightMilliseconds", num(state.time_eight_milliseconds)),
            ("gravity", num(state.gravity)),
            ("deltaAngleShorts", write_triple(state.delta_angle_shorts)),
        ]),
        UnifiedMovementState::Q2Rerelease(state) => obj(vec![
            ("kind", json_str("q2-rerelease")),
            ("type", num(state.move_type)),
            ("origin", write_vector(state.origin)),
            ("velocity", write_vector(state.velocity)),
            ("flags", num(state.flags)),
            ("timeMilliseconds", num(state.time_milliseconds)),
            ("gravity", num(state.gravity)),
            ("deltaAngles", write_vector(state.delta_angles)),
            ("viewHeight", num(state.view_height)),
        ]),
        UnifiedMovementState::Q3(state) => obj(vec![
            ("kind", json_str("q3")),
            ("commandTimeMilliseconds", num(state.command_time_milliseconds)),
            ("movementType", num(state.movement_type)),
            ("bobCycle", num(state.bob_cycle)),
            ("movementFlags", num(state.movement_flags)),
            ("movementTimeMilliseconds", num(state.movement_time_milliseconds)),
            ("origin", write_vector(state.origin)),
            ("velocity", write_vector(state.velocity)),
            ("gravity", num(state.gravity)),
            ("speed", num(state.speed)),
            ("deltaAngleWords", write_triple(state.delta_angle_words)),
            ("movementDirection", num(state.movement_direction)),
            ("grapplePoint", write_vector(state.grapple_point)),
            ("flags", num(state.flags)),
            ("viewAngles", write_vector(state.view_angles)),
            ("viewHeight", num(state.view_height)),
            ("ground", write_hit(&state.ground)),
            ("predictableEventSequence", num(state.predictable_event_sequence)),
            ("jumpPad", state.jump_pad.as_ref().map_or(SaveJson::Null, wire_actor)),
            ("movementFrame", num(state.movement_frame)),
            ("jumpPadFrame", num(state.jump_pad_frame)),
        ]),
    }
}

fn read_weapon(reader: SaveReader) -> Result<UnifiedWeaponState, WorldError> {
    match reader.field("kind").choice_str(&["q1", "q2", "q3"])?.as_str() {
        "q1" => Ok(UnifiedWeaponState::Q1 {
            frame: reader.field("frame").finite()?,
            attack_finished_seconds: reader.field("attackFinishedSeconds").finite()?,
            source_weapon: reader.field("sourceWeapon").finite()?,
        }),
        "q2" => Ok(UnifiedWeaponState::Q2 {
            gun_frame: reader.field("gunFrame").finite()?,
            state: reader.field("state").finite()?,
            pending_weapon: reader.field("pendingWeapon").nullable(namespaced)?,
            machinegun_shots: reader.field("machinegunShots").finite()?,
            grenade_time: read_time(reader.field("grenadeTime"))?,
            grenade_blew_up: reader.field("grenadeBlewUp").boolean()?,
        }),
        _ => Ok(UnifiedWeaponState::Q3 {
            source_weapon: reader.field("sourceWeapon").finite()?,
            state: reader.field("state").finite()?,
            time_milliseconds: reader.field("timeMilliseconds").finite()?,
        }),
    }
}

fn write_weapon(state: &UnifiedWeaponState) -> SaveJson {
    match state {
        UnifiedWeaponState::Q1 {
            frame,
            attack_finished_seconds,
            source_weapon,
        } => obj(vec![
            ("kind", json_str("q1")),
            ("frame", num(*frame)),
            ("attackFinishedSeconds", num(*attack_finished_seconds)),
            ("sourceWeapon", num(*source_weapon)),
        ]),
        UnifiedWeaponState::Q2 {
            gun_frame,
            state,
            pending_weapon,
            machinegun_shots,
            grenade_time,
            grenade_blew_up,
        } => obj(vec![
            ("kind", json_str("q2")),
            ("gunFrame", num(*gun_frame)),
            ("state", num(*state)),
            (
                "pendingWeapon",
                pending_weapon
                    .as_ref()
                    .map_or(SaveJson::Null, |weapon| json_str(weapon)),
            ),
            ("machinegunShots", num(*machinegun_shots)),
            ("grenadeTime", write_time(*grenade_time)),
            ("grenadeBlewUp", boolean(*grenade_blew_up)),
        ]),
        UnifiedWeaponState::Q3 {
            source_weapon,
            state,
            time_milliseconds,
        } => obj(vec![
            ("kind", json_str("q3")),
            ("sourceWeapon", num(*source_weapon)),
            ("state", num(*state)),
            ("timeMilliseconds", num(*time_milliseconds)),
        ]),
    }
}

fn read_arsenal(reader: SaveReader) -> Result<UnifiedArsenalState, WorldError> {
    Ok(UnifiedArsenalState {
        provider: namespaced(reader.field("provider"))?,
        active_weapon: reader.field("activeWeapon").nullable(namespaced)?,
        state: read_weapon(reader.field("state"))?,
        ammo: reader.field("ammo").list(read_inventory_entry)?,
    })
}

fn write_arsenal(arsenal: &UnifiedArsenalState) -> SaveJson {
    obj(vec![
        ("provider", json_str(&arsenal.provider)),
        (
            "activeWeapon",
            arsenal
                .active_weapon
                .as_ref()
                .map_or(SaveJson::Null, |weapon| json_str(weapon)),
        ),
        ("state", write_weapon(&arsenal.state)),
        ("ammo", arr(arsenal.ammo.iter().map(write_inventory_entry).collect())),
    ])
}

fn read_animation(reader: SaveReader) -> Result<UnifiedAnimationState, WorldError> {
    match reader.field("kind").choice_str(&["q1", "q2", "q3"])?.as_str() {
        "q1" => Ok(UnifiedAnimationState::Q1 {
            frame: reader.field("frame").finite()?,
            next_frame_seconds: reader.field("nextFrameSeconds").finite()?,
        }),
        "q2" => Ok(UnifiedAnimationState::Q2 {
            frame: reader.field("frame").finite()?,
            end_frame: reader.field("endFrame").finite()?,
            priority: reader.field("priority").finite()?,
            duck: reader.field("duck").boolean()?,
            run: reader.field("run").boolean()?,
        }),
        _ => Ok(UnifiedAnimationState::Q3 {
            legs: reader.field("legs").finite()?,
            torso: reader.field("torso").finite()?,
            legs_timer_milliseconds: reader.field("legsTimerMilliseconds").finite()?,
            torso_timer_milliseconds: reader.field("torsoTimerMilliseconds").finite()?,
        }),
    }
}

fn write_animation(state: &UnifiedAnimationState) -> SaveJson {
    match state {
        UnifiedAnimationState::Q1 {
            frame,
            next_frame_seconds,
        } => obj(vec![
            ("kind", json_str("q1")),
            ("frame", num(*frame)),
            ("nextFrameSeconds", num(*next_frame_seconds)),
        ]),
        UnifiedAnimationState::Q2 {
            frame,
            end_frame,
            priority,
            duck,
            run,
        } => obj(vec![
            ("kind", json_str("q2")),
            ("frame", num(*frame)),
            ("endFrame", num(*end_frame)),
            ("priority", num(*priority)),
            ("duck", boolean(*duck)),
            ("run", boolean(*run)),
        ]),
        UnifiedAnimationState::Q3 {
            legs,
            torso,
            legs_timer_milliseconds,
            torso_timer_milliseconds,
        } => obj(vec![
            ("kind", json_str("q3")),
            ("legs", num(*legs)),
            ("torso", num(*torso)),
            ("legsTimerMilliseconds", num(*legs_timer_milliseconds)),
            ("torsoTimerMilliseconds", num(*torso_timer_milliseconds)),
        ]),
    }
}

fn clock_kind(clock: &ClockProfile) -> &'static str {
    match clock {
        ClockProfile::Q1Netquake { .. } => "q1-netquake",
        ClockProfile::Q1Quakeworld { .. } => "q1-quakeworld",
        ClockProfile::Q2Classic => "q2-classic",
        ClockProfile::Q2Rerelease { .. } => "q2-rerelease",
        ClockProfile::Q3 { .. } => "q3",
    }
}

fn read_parameters(reader: SaveReader) -> Result<Q1MovementParameters, WorldError> {
    let parameters = reader.field("parameters");
    Ok(Q1MovementParameters {
        gravity: parameters.field("gravity").finite()?,
        stop_speed: parameters.field("stopSpeed").finite()?,
        max_speed: parameters.field("maxSpeed").finite()?,
        spectator_max_speed: parameters.field("spectatorMaxSpeed").finite()?,
        accelerate: parameters.field("accelerate").finite()?,
        air_accelerate: parameters.field("airAccelerate").finite()?,
        water_accelerate: parameters.field("waterAccelerate").finite()?,
        friction: parameters.field("friction").finite()?,
        water_friction: parameters.field("waterFriction").finite()?,
        entity_gravity: parameters.field("entityGravity").finite()?,
    })
}

fn write_parameters(parameters: &Q1MovementParameters) -> SaveJson {
    obj(vec![
        ("gravity", num(parameters.gravity)),
        ("stopSpeed", num(parameters.stop_speed)),
        ("maxSpeed", num(parameters.max_speed)),
        ("spectatorMaxSpeed", num(parameters.spectator_max_speed)),
        ("accelerate", num(parameters.accelerate)),
        ("airAccelerate", num(parameters.air_accelerate)),
        ("waterAccelerate", num(parameters.water_accelerate)),
        ("friction", num(parameters.friction)),
        ("waterFriction", num(parameters.water_friction)),
        ("entityGravity", num(parameters.entity_gravity)),
    ])
}

fn read_profile(reader: SaveReader) -> Result<UnifiedMovementProfile, WorldError> {
    let id = namespaced(reader.field("id"))?;
    let clock = read_clock(reader.field("clock"))?;
    let numeric = read_numeric(reader.field("numeric"))?;
    let kind =
        reader
            .field("kind")
            .choice_str(&["q1-netquake", "q1-quakeworld", "q2-classic", "q2-rerelease", "q3"])?;
    if clock_kind(&clock) != kind {
        return Err(reader.fail("movement clock differs from profile"));
    }
    let profile_kind = match kind.as_str() {
        "q1-netquake" => {
            let edition = reader
                .field("edition")
                .choice_str(&["classic", "rerelease", "quake64"])?;
            UnifiedProfileKind::Q1Netquake {
                parameters: read_parameters(reader.clone())?,
                edition: match edition.as_str() {
                    "classic" => Q1NetquakeEdition::Classic,
                    "rerelease" => Q1NetquakeEdition::Rerelease,
                    _ => Q1NetquakeEdition::Quake64,
                },
                edge_friction: reader.field("edgeFriction").finite()?,
                no_clip_angle_hack: reader.field("noClipAngleHack").boolean()?,
            }
        }
        "q1-quakeworld" => UnifiedProfileKind::Q1Quakeworld {
            parameters: read_parameters(reader.clone())?,
        },
        "q2-classic" => {
            let strafejump = reader.field("strafejumpHack");
            UnifiedProfileKind::Q2Classic {
                air_accelerate: reader.field("airAccelerate").finite()?,
                snap_initial: reader.field("snapInitial").boolean()?,
                strafejump_hack: if strafejump.value.is_none() {
                    None
                } else {
                    Some(strafejump.boolean()?)
                },
            }
        }
        "q2-rerelease" => UnifiedProfileKind::Q2Rerelease {
            air_accelerate: reader.field("airAccelerate").finite()?,
            n64_physics: reader.field("n64Physics").boolean()?,
        },
        _ => {
            let product = reader.field("product").choice_str(&["baseq3", "missionpack"])?;
            UnifiedProfileKind::Q3 {
                product: if product == "baseq3" {
                    Q3ProfileProduct::BaseQ3
                } else {
                    Q3ProfileProduct::MissionPack
                },
                fixed_milliseconds: reader.field("fixedMilliseconds").nullable(|value| value.finite())?,
                no_footsteps: reader.field("noFootsteps").boolean()?,
            }
        }
    };
    Ok(UnifiedMovementProfile {
        id,
        clock,
        numeric,
        kind: profile_kind,
    })
}

fn write_profile(profile: &UnifiedMovementProfile) -> SaveJson {
    let mut members = vec![
        ("id", json_str(&profile.id)),
        ("clock", write_clock(profile.clock)),
        ("numeric", write_numeric(&profile.numeric)),
        ("kind", json_str(profile.kind_text())),
    ];
    match &profile.kind {
        UnifiedProfileKind::Q1Netquake {
            parameters,
            edition,
            edge_friction,
            no_clip_angle_hack,
        } => {
            members.push(("parameters", write_parameters(parameters)));
            members.push((
                "edition",
                json_str(match edition {
                    Q1NetquakeEdition::Classic => "classic",
                    Q1NetquakeEdition::Rerelease => "rerelease",
                    Q1NetquakeEdition::Quake64 => "quake64",
                }),
            ));
            members.push(("edgeFriction", num(*edge_friction)));
            members.push(("noClipAngleHack", boolean(*no_clip_angle_hack)));
        }
        UnifiedProfileKind::Q1Quakeworld { parameters } => {
            members.push(("parameters", write_parameters(parameters)));
        }
        UnifiedProfileKind::Q2Classic {
            air_accelerate,
            snap_initial,
            strafejump_hack,
        } => {
            members.push(("airAccelerate", num(*air_accelerate)));
            members.push(("snapInitial", boolean(*snap_initial)));
            if let Some(hack) = strafejump_hack {
                members.push(("strafejumpHack", boolean(*hack)));
            }
        }
        UnifiedProfileKind::Q2Rerelease {
            air_accelerate,
            n64_physics,
        } => {
            members.push(("airAccelerate", num(*air_accelerate)));
            members.push(("n64Physics", boolean(*n64_physics)));
        }
        UnifiedProfileKind::Q3 {
            product,
            fixed_milliseconds,
            no_footsteps,
        } => {
            members.push((
                "product",
                json_str(match product {
                    Q3ProfileProduct::BaseQ3 => "baseq3",
                    Q3ProfileProduct::MissionPack => "missionpack",
                }),
            ));
            members.push(("fixedMilliseconds", fixed_milliseconds.map_or(SaveJson::Null, num)));
            members.push(("noFootsteps", boolean(*no_footsteps)));
        }
    }
    obj(members)
}

fn read_client_outputs(reader: SaveReader) -> Result<UnifiedClientOutputs, WorldError> {
    let view_offset = reader.field("viewOffset");
    let mode = reader.field("mode");
    let stance = reader.field("stance");
    let body_bounds = reader.field("bodyBounds");
    Ok(UnifiedClientOutputs {
        view_offset: if view_offset.value.is_none() {
            None
        } else {
            Some(read_vector(view_offset)?)
        },
        mode: if mode.value.is_none() {
            None
        } else {
            Some(match mode.choice_str(&["normal", "noclip", "freeze"])?.as_str() {
                "normal" => UnifiedClientMovementMode::Normal,
                "noclip" => UnifiedClientMovementMode::Noclip,
                _ => UnifiedClientMovementMode::Freeze,
            })
        },
        stance: if stance.value.is_none() {
            None
        } else {
            Some(stance.boolean()?)
        },
        body_bounds: if body_bounds.value.is_none() {
            None
        } else {
            Some(read_bounds(body_bounds)?)
        },
    })
}

fn write_client_outputs(outputs: &UnifiedClientOutputs) -> SaveJson {
    let mut members = Vec::new();
    if let Some(offset) = outputs.view_offset {
        members.push(("viewOffset", write_vector(offset)));
    }
    if let Some(mode) = outputs.mode {
        members.push((
            "mode",
            json_str(match mode {
                UnifiedClientMovementMode::Normal => "normal",
                UnifiedClientMovementMode::Noclip => "noclip",
                UnifiedClientMovementMode::Freeze => "freeze",
            }),
        ));
    }
    if let Some(stance) = outputs.stance {
        members.push(("stance", boolean(stance)));
    }
    if let Some(bounds) = outputs.body_bounds {
        members.push(("bodyBounds", write_bounds(bounds)));
    }
    obj(members)
}

fn read_environment(reader: SaveReader) -> Result<UnifiedPredictionEnvironment, WorldError> {
    let outputs = reader.field("clientOutputs");
    Ok(UnifiedPredictionEnvironment {
        health: reader.field("health").finite()?,
        flight: reader.field("flight").boolean()?,
        haste: reader.field("haste").boolean()?,
        invulnerable: reader.field("invulnerable").boolean()?,
        gravity_multiplier: reader.field("gravityMultiplier").finite()?,
        client_outputs: if outputs.value.is_none() {
            None
        } else {
            Some(read_client_outputs(outputs)?)
        },
    })
}

fn write_environment(environment: &UnifiedPredictionEnvironment) -> SaveJson {
    let mut members = vec![
        ("health", num(environment.health)),
        ("flight", boolean(environment.flight)),
        ("haste", boolean(environment.haste)),
        ("invulnerable", boolean(environment.invulnerable)),
        ("gravityMultiplier", num(environment.gravity_multiplier)),
    ];
    if let Some(outputs) = &environment.client_outputs {
        members.push(("clientOutputs", write_client_outputs(outputs)));
    }
    obj(members)
}

fn read_collision(
    reader: SaveReader,
    identity: &dyn UnifiedIdentityDecoder,
) -> Result<UnifiedSpatialActor, WorldError> {
    let body = reader.field("body");
    let state = body.field("state");
    let bounds = state.field("bounds");
    let collision = reader.field("collision");
    let shape = collision.field("shape");
    let shape_kind = shape.field("kind").choice_str(&["box", "capsule", "model"])?;
    let family = collision.field("family").choice_str(&["q1", "q2", "q3"])?;
    let role = collision.field("role").choice_str(&["solid", "trigger"])?;
    let q1_corpse = collision.field("q1Corpse");
    let q3_owner = collision.field("q3Owner");
    Ok(UnifiedSpatialActor {
        body: UnifiedLinkedBody {
            actor: read_actor(body.field("actor"), identity)?,
            state: UnifiedLinkedBodyState {
                origin: read_vector(state.field("origin"))?,
                angles: read_vector(state.field("angles"))?,
                velocity: read_vector(state.field("velocity"))?,
                bounds: read_bounds(bounds)?,
                ground: state.field("ground").nullable(|ground| read_actor(ground, identity))?,
            },
            link_count: body.field("linkCount").integer(0)?,
            absolute_bounds: read_bounds(body.field("absoluteBounds"))?,
        },
        collision: UnifiedCollisionRecord {
            family: match family.as_str() {
                "q1" => UnifiedCollisionFamily::Q1,
                "q2" => UnifiedCollisionFamily::Q2,
                _ => UnifiedCollisionFamily::Q3,
            },
            shape: match shape_kind.as_str() {
                "model" => UnifiedCollisionShape::Model {
                    model: shape.field("model").integer(0)?,
                },
                "capsule" => UnifiedCollisionShape::Capsule,
                _ => UnifiedCollisionShape::Box,
            },
            contents: collision.field("contents").finite()? as i64,
            owner: collision.field("owner").nullable(|owner| read_actor(owner, identity))?,
            role: if role == "solid" {
                UnifiedCollisionRole::Solid
            } else {
                UnifiedCollisionRole::Trigger
            },
            monster: collision.field("monster").boolean()?,
            dead_monster: collision.field("deadMonster").boolean()?,
            q1_corpse: if q1_corpse.value.is_none() {
                None
            } else {
                Some(q1_corpse.literal_bool(true)?)
            },
            q3_owner: if q3_owner.value.is_none() {
                None
            } else {
                Some(UnifiedQ3Owner {
                    entity_number: q3_owner.field("entityNumber").finite()? as i64,
                    owner_number: q3_owner.field("ownerNumber").finite()? as i64,
                })
            },
        },
    })
}

fn write_collision(entry: &UnifiedSpatialActor) -> SaveJson {
    let shape = match &entry.collision.shape {
        UnifiedCollisionShape::Model { model } => obj(vec![("kind", json_str("model")), ("model", int(*model))]),
        UnifiedCollisionShape::Capsule => obj(vec![("kind", json_str("capsule"))]),
        UnifiedCollisionShape::Box => obj(vec![("kind", json_str("box"))]),
    };
    let mut collision_members = vec![
        (
            "family",
            json_str(match entry.collision.family {
                UnifiedCollisionFamily::Q1 => "q1",
                UnifiedCollisionFamily::Q2 => "q2",
                UnifiedCollisionFamily::Q3 => "q3",
            }),
        ),
        ("shape", shape),
        ("contents", num(entry.collision.contents as f64)),
        (
            "owner",
            entry.collision.owner.as_ref().map_or(SaveJson::Null, wire_actor),
        ),
        (
            "role",
            json_str(match entry.collision.role {
                UnifiedCollisionRole::Solid => "solid",
                UnifiedCollisionRole::Trigger => "trigger",
            }),
        ),
        ("monster", boolean(entry.collision.monster)),
        ("deadMonster", boolean(entry.collision.dead_monster)),
    ];
    if let Some(corpse) = entry.collision.q1_corpse {
        collision_members.push(("q1Corpse", boolean(corpse)));
    }
    if let Some(owner) = &entry.collision.q3_owner {
        collision_members.push((
            "q3Owner",
            obj(vec![
                ("entityNumber", num(owner.entity_number as f64)),
                ("ownerNumber", num(owner.owner_number as f64)),
            ]),
        ));
    }
    obj(vec![
        (
            "body",
            obj(vec![
                ("actor", wire_actor(&entry.body.actor)),
                (
                    "state",
                    obj(vec![
                        ("origin", write_vector(entry.body.state.origin)),
                        ("angles", write_vector(entry.body.state.angles)),
                        ("velocity", write_vector(entry.body.state.velocity)),
                        ("bounds", write_bounds(entry.body.state.bounds)),
                        (
                            "ground",
                            entry.body.state.ground.as_ref().map_or(SaveJson::Null, wire_actor),
                        ),
                    ]),
                ),
                ("linkCount", int(entry.body.link_count)),
                ("absoluteBounds", write_bounds(entry.body.absolute_bounds)),
            ]),
        ),
        ("collision", obj(collision_members)),
    ])
}

/// Encode a prediction projection (donor `encodeUnifiedPrediction`).
#[must_use]
pub fn encode_unified_prediction(value: &UnifiedPredictionProjection) -> Vec<u8> {
    encode_checkpoint_value(&obj(vec![
        ("schema", json_str("qts-unified-prediction")),
        ("version", int(1)),
        ("actor", wire_actor(&value.actor)),
        ("sequence", int(value.sequence)),
        ("commandTimeMilliseconds", num(value.command_time_milliseconds)),
        ("profile", write_profile(&value.profile)),
        ("state", write_movement(&value.state)),
        ("arsenal", write_arsenal(&value.arsenal)),
        (
            "animation",
            obj(vec![
                ("provider", json_str(&value.animation.provider)),
                ("state", write_animation(&value.animation.state)),
            ]),
        ),
        ("standingBounds", write_bounds(value.standing_bounds)),
        ("standingViewHeight", num(value.standing_view_height)),
        ("bounds", write_bounds(value.bounds)),
        ("viewAngles", write_vector(value.view_angles)),
        ("viewHeight", num(value.view_height)),
        ("viewOffset", write_vector(value.view_offset)),
        ("environment", write_environment(&value.environment)),
        (
            "contact",
            value.contact.as_ref().map_or(SaveJson::Null, |contact| {
                obj(vec![
                    ("ground", write_hit(&contact.ground)),
                    ("waterLevel", int(contact.water_level)),
                    ("waterType", int(contact.water_type)),
                ])
            }),
        ),
        (
            "collisions",
            arr(value.collisions.iter().map(write_collision).collect()),
        ),
    ]))
}

/// Decode a prediction projection (donor `decodeUnifiedPrediction`).
pub fn decode_unified_prediction(
    bytes: &[u8],
    identity: &dyn UnifiedIdentityDecoder,
) -> Result<UnifiedPredictionProjection, UnifiedPredictionError> {
    if bytes.len() > 32 * 1024 * 1024 {
        return Err(UnifiedPredictionError::Source(
            "Unified prediction exceeds byte limit".to_string(),
        ));
    }
    let value = decode_checkpoint_value(bytes)?;
    let reader = SaveReader::new(&value);
    reader.field("schema").literal_str("qts-unified-prediction")?;
    reader.field("version").literal_i64(1)?;
    let state = read_movement(reader.field("state"), identity)?;
    let profile = read_profile(reader.field("profile"))?;
    if state.kind() != profile.kind_text() {
        return Err(reader.fail("movement state differs from profile").into());
    }
    let environment = read_environment(reader.field("environment"))?;
    Ok(UnifiedPredictionProjection {
        actor: read_actor(reader.field("actor"), identity)?,
        sequence: reader.field("sequence").integer(-1)?,
        command_time_milliseconds: reader.field("commandTimeMilliseconds").finite()?,
        state,
        profile,
        arsenal: read_arsenal(reader.field("arsenal"))?,
        animation: {
            let animation = reader.field("animation");
            UnifiedActorAnimation {
                provider: namespaced(animation.field("provider"))?,
                state: read_animation(animation.field("state"))?,
            }
        },
        standing_bounds: read_bounds(reader.field("standingBounds"))?,
        standing_view_height: reader.field("standingViewHeight").finite()?,
        bounds: read_bounds(reader.field("bounds"))?,
        view_angles: read_vector(reader.field("viewAngles"))?,
        view_height: reader.field("viewHeight").finite()?,
        view_offset: read_vector(reader.field("viewOffset"))?,
        environment,
        contact: reader.field("contact").nullable(|contact| {
            Ok::<_, UnifiedPredictionError>(UnifiedPredictionContact {
                ground: read_hit(contact.field("ground"), identity)?,
                water_level: contact.field("waterLevel").finite()? as i64,
                water_type: contact.field("waterType").finite()? as i64,
            })
        })?,
        collisions: reader
            .field("collisions")
            .list(|entry| read_collision(entry, identity))?,
    })
}

/// Authoritative simulation surface the projection reads from.
///
/// Mirrors the donor `SharedSimulation` calls used by
/// `projectUnifiedPrediction`: the admitted movement player plus linked
/// collision actors.
pub trait UnifiedPredictionSource {
    /// Seconds on the authoritative clock.
    fn time_seconds(&self) -> f64;
    /// Read the admitted movement player, if any.
    fn movement_player(&self, actor: &ActorId) -> Option<UnifiedSourcePlayer>;
    /// Linked collision actor for an observed id, if any.
    fn linked_actor(&self, actor: &ActorId) -> Option<UnifiedSpatialActor>;
    /// Observed actor ids.
    fn observed_actors(&self) -> Vec<ActorId>;
}

/// Admitted movement player snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct UnifiedSourcePlayer {
    /// Movement state.
    pub state: UnifiedMovementState,
    /// Arsenal snapshot.
    pub arsenal: UnifiedArsenalState,
    /// Animation snapshot.
    pub animation: UnifiedActorAnimation,
    /// Prediction environment.
    pub environment: UnifiedPredictionEnvironment,
    /// Body bounds.
    pub bounds: Bounds,
    /// View angles.
    pub view_angles: Vec3,
    /// View height.
    pub view_height: f64,
    /// Ground hit.
    pub ground: UnifiedTraceHit,
    /// Water level.
    pub water_level: i64,
    /// Water type.
    pub water_type: i64,
    /// Prediction profile.
    pub profile: UnifiedMovementProfile,
    /// Standing bounds.
    pub standing_bounds: Bounds,
    /// Character family (`q3` selects the 26-unit standing eye).
    pub character_is_q3: bool,
}

/// Project a prediction snapshot from the authoritative simulation.
///
/// The command clock follows the Q3 state when present and the server
/// clock otherwise; every observed actor with a linked collision record
/// joins the projection.
pub fn project_unified_prediction(
    simulation: &dyn UnifiedPredictionSource,
    actor: &ActorId,
    acknowledged_input: i64,
) -> Result<UnifiedPredictionProjection, UnifiedPredictionError> {
    let player = simulation.movement_player(actor).ok_or_else(|| {
        UnifiedPredictionError::Source("Unified prediction requires an admitted movement player".to_string())
    })?;
    let command_time_milliseconds = match &player.state {
        UnifiedMovementState::Q3(state) => state.command_time_milliseconds,
        _ => simulation.time_seconds() * 1000.0,
    };
    let mut collisions = Vec::new();
    for observed in simulation.observed_actors() {
        if let Some(linked) = simulation.linked_actor(&observed) {
            collisions.push(linked);
        }
    }
    Ok(UnifiedPredictionProjection {
        actor: actor.clone(),
        sequence: acknowledged_input,
        command_time_milliseconds,
        state: player.state,
        profile: player.profile,
        arsenal: player.arsenal,
        animation: player.animation,
        environment: player.environment,
        bounds: player.bounds,
        view_angles: player.view_angles,
        view_height: player.view_height,
        view_offset: Vec3 {
            x: 0.0,
            y: 0.0,
            z: player.view_height as f32,
        },
        contact: Some(UnifiedPredictionContact {
            ground: player.ground,
            water_level: player.water_level,
            water_type: player.water_type,
        }),
        standing_bounds: player.standing_bounds,
        standing_view_height: if player.character_is_q3 { 26.0 } else { 22.0 },
        collisions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::{ClientId, IdentityOwner, SeatId, SessionId};

    use super::super::unified_types::UnifiedIdentityDecoder as Decoder;

    struct Ledger {
        owner: IdentityOwner,
    }

    impl Decoder for Ledger {
        fn session(&self) -> SessionId {
            self.owner.session().clone()
        }
        fn actor(&self, slot: u32, generation: u32) -> ActorId {
            self.owner.actor(slot, generation)
        }
        fn client(&self, slot: u32, generation: u32) -> ClientId {
            self.owner.client(slot, generation)
        }
        fn seat(&self, index: u32) -> SeatId {
            self.owner.seat(index)
        }
        fn resource_id(&self, id: &str) -> String {
            id.to_string()
        }
    }

    fn ledger() -> Ledger {
        Ledger {
            owner: IdentityOwner::create("prediction").unwrap(),
        }
    }

    fn vec_json(x: f64, y: f64, z: f64) -> SaveJson {
        obj(vec![("x", num(x)), ("y", num(y)), ("z", num(z))])
    }

    fn bounds_json() -> SaveJson {
        obj(vec![
            ("min", vec_json(-16.0, -16.0, -24.0)),
            ("max", vec_json(16.0, 16.0, 32.0)),
        ])
    }

    fn movement_json() -> SaveJson {
        obj(vec![
            ("kind", json_str("q1-quakeworld")),
            ("origin", vec_json(1.0, 2.0, 3.0)),
            ("velocity", vec_json(0.0, 0.0, 0.0)),
            ("angles", vec_json(0.0, 90.0, 0.0)),
            ("oldButtons", num(0.0)),
            ("waterJumpTimeSeconds", num(0.0)),
            ("dead", boolean(false)),
            ("spectator", num(0.0)),
            ("ground", obj(vec![("kind", json_str("none"))])),
        ])
    }

    fn profile_json() -> SaveJson {
        obj(vec![
            ("id", json_str("test:qw")),
            (
                "clock",
                obj(vec![
                    ("kind", json_str("q1-quakeworld")),
                    ("maximumCommandMilliseconds", num(100.0)),
                ]),
            ),
            (
                "numeric",
                obj(vec![
                    ("id", json_str("test:default")),
                    (
                        "arithmetic",
                        obj(vec![
                            ("kind", json_str("binary32")),
                            ("round", json_str("each-operation")),
                        ]),
                    ),
                    ("floatToInt", json_str("x86-indefinite")),
                    ("scalarStorage", json_str("binary32")),
                    ("integerOverflow", json_str("wrap32")),
                ]),
            ),
            ("kind", json_str("q1-quakeworld")),
            (
                "parameters",
                obj(vec![
                    ("gravity", num(800.0)),
                    ("stopSpeed", num(100.0)),
                    ("maxSpeed", num(320.0)),
                    ("spectatorMaxSpeed", num(500.0)),
                    ("accelerate", num(10.0)),
                    ("airAccelerate", num(1.0)),
                    ("waterAccelerate", num(10.0)),
                    ("friction", num(4.0)),
                    ("waterFriction", num(4.0)),
                    ("entityGravity", num(1.0)),
                ]),
            ),
        ])
    }

    fn projection_json(actor: &ActorId) -> SaveJson {
        obj(vec![
            ("schema", json_str("qts-unified-prediction")),
            ("version", int(1)),
            ("actor", wire_actor(actor)),
            ("sequence", int(7)),
            ("commandTimeMilliseconds", num(1234.0)),
            ("profile", profile_json()),
            ("state", movement_json()),
            (
                "arsenal",
                obj(vec![
                    ("provider", json_str("test:arsenal")),
                    ("activeWeapon", SaveJson::Null),
                    (
                        "state",
                        obj(vec![
                            ("kind", json_str("q1")),
                            ("frame", num(0.0)),
                            ("attackFinishedSeconds", num(0.0)),
                            ("sourceWeapon", num(1.0)),
                        ]),
                    ),
                    ("ammo", arr(Vec::new())),
                ]),
            ),
            (
                "animation",
                obj(vec![
                    ("provider", json_str("test:character")),
                    (
                        "state",
                        obj(vec![
                            ("kind", json_str("q1")),
                            ("frame", num(0.0)),
                            ("nextFrameSeconds", num(0.0)),
                        ]),
                    ),
                ]),
            ),
            ("standingBounds", bounds_json()),
            ("standingViewHeight", num(22.0)),
            ("bounds", bounds_json()),
            ("viewAngles", vec_json(0.0, 90.0, 0.0)),
            ("viewHeight", num(22.0)),
            ("viewOffset", vec_json(0.0, 0.0, 22.0)),
            (
                "environment",
                obj(vec![
                    ("health", num(100.0)),
                    ("flight", boolean(false)),
                    ("haste", boolean(false)),
                    ("invulnerable", boolean(false)),
                    ("gravityMultiplier", num(1.0)),
                ]),
            ),
            ("contact", SaveJson::Null),
            ("collisions", arr(Vec::new())),
        ])
    }

    #[test]
    fn projection_round_trips() {
        let ledger = ledger();
        let actor = ledger.actor(2, 0);
        let encoded = encode_checkpoint_value(&projection_json(&actor));
        let decoded = decode_unified_prediction(&encoded, &ledger).unwrap();
        assert_eq!(decoded.actor, actor);
        assert_eq!(decoded.sequence, 7);
        assert!(matches!(decoded.state, UnifiedMovementState::Q1Quakeworld(_)));
        let reencoded = encode_unified_prediction(&decoded);
        let round = decode_unified_prediction(&reencoded, &ledger).unwrap();
        assert_eq!(round, decoded);
    }

    #[test]
    fn state_profile_mismatch_fails() {
        let ledger = ledger();
        let actor = ledger.actor(2, 0);
        let mut value = projection_json(&actor);
        if let SaveJson::Object(members) = &mut value {
            for (key, member) in members.iter_mut() {
                if key == "state" {
                    if let SaveJson::Object(fields) = member {
                        for (field, data) in fields.iter_mut() {
                            if field == "kind" {
                                *data = json_str("q3");
                            }
                        }
                    }
                }
            }
        }
        // Q3 state needs Q3 fields; the kind swap alone must fail decoding.
        let encoded = encode_checkpoint_value(&value);
        assert!(decode_unified_prediction(&encoded, &ledger).is_err());
    }
}
