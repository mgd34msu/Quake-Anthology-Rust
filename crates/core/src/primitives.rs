//! Engine-owned values. Game formats and module ABIs convert at their boundaries.

#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(transparent)]
pub struct Vec3(pub [f32; 3]);

impl Vec3 {
    pub fn dot(self, other: Self) -> f32 {
        self.0[0] * other.0[0] + self.0[1] * other.0[1] + self.0[2] * other.0[2]
    }

    pub fn lerp(self, end: Self, fraction: f32) -> Self {
        Self(std::array::from_fn(|axis| {
            self.0[axis] + fraction * (end.0[axis] - self.0[axis])
        }))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
    Z,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Plane {
    pub normal: Vec3,
    pub distance: f32,
    pub axis: Option<Axis>,
}

#[derive(Clone, Copy, Debug)]
pub struct ClipNode {
    pub plane: u32,
    pub children: [i32; 2],
}

impl Plane {
    pub fn signed_distance(self, point: Vec3) -> f32 {
        let coordinate = match self.axis {
            Some(Axis::X) => point.0[0],
            Some(Axis::Y) => point.0[1],
            Some(Axis::Z) => point.0[2],
            None => self.normal.dot(point),
        };
        coordinate - self.distance
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntityId {
    pub slot: u32,
    pub generation: u32,
}

/// Engine geometry lifetime. Native inline-model ordinals remain independent
/// values at file, module and protocol boundaries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GeometryId {
    pub slot: u32,
    pub generation: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ItemId(pub u32);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProductId(pub u16);
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WeaponId(pub u32);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CvarHandle(pub u32);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SoundId(pub u32);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum PcmChannels {
    Mono = 1,
    Stereo = 2,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pcm {
    pub rate: std::num::NonZeroU32,
    pub channels: PcmChannels,
    pub samples: Vec<i16>,
    pub loop_start: Option<usize>,
}
impl Pcm {
    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels as usize
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EffectId(pub u32);
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NameId(pub u32);
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModuleId(pub u16);

/// Weak slot identity from a module's native entity namespace. Raw world/none
/// sentinels stay signed and unchanged; this is not an engine lifetime handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeEntity {
    pub module: ModuleId,
    pub slot: i32,
}

/// Collision ownership is separate from the module which runs an entity.
/// Native references name a slot across reuse; lifetime references name one
/// generation of that slot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CollisionOwner {
    #[default]
    None,
    Lifetime(EntityId),
    Native(NativeEntity),
}

/// Only shapes implemented by the shared entity narrow phase are admitted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CollisionShape {
    #[default]
    None,
    Box,
    Model {
        geometry: GeometryId,
        index: u32,
    },
}

/// A target role selects its model transform independently of geometry and
/// the caller's trace rules. Temporary boxes never rotate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ModelRotation {
    #[default]
    TranslationOnly,
    /// Restore a hit normal using the basis of the negated Euler angles.
    NegativeEuler,
    /// Center the query before translation and restore with the basis transpose.
    TransposeBasis,
}

/// Conservative bounds selected by a linked model's role, before the ordinary
/// area-link expansion. Model load bounds already include their native margin.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RotatedLinkBounds {
    #[default]
    Unrotated,
    MaxAbsCube,
    RadiusCube,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModelRules {
    pub rotation: ModelRotation,
    pub link_bounds: RotatedLinkBounds,
}

/// A module may publish a contents pose independently of its physical pose.
/// These engine coordinates are not native protocol fields.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EntityPose {
    pub position: Vec3,
    pub angles: Vec3,
}

/// Canonical collision classifications, converted from native flags at load.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(transparent)]
pub struct CollisionTags(pub u8);

impl CollisionTags {
    pub const MONSTER: Self = Self(1);
    pub const DEAD_MONSTER: Self = Self(2);
}

impl std::ops::BitOr for CollisionTags {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CallbackId(pub u32);

/// Native callback identity plus its load/spawn-resolved engine table entry.
/// The path is internal binding metadata and never a protocol or save field.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ThinkBinding {
    pub callback: Option<CallbackId>,
    pub path: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Engine client identity. Native client limits and wire widths are applied at
/// each protocol boundary, independently of this load-sized namespace.
pub struct ClientId(pub u32);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PowerupId(pub u16);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextId {
    pub slot: u16,
    pub generation: u32,
}

/// Module clocks retain their native units. Numeric width conversion belongs
/// to the module timing rule or the protocol/ABI boundary, not entity identity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ThinkTime {
    Seconds(f64),
    Milliseconds(i64),
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Think {
    pub at: Option<ThinkTime>,
    pub callback: Option<CallbackId>,
}

#[derive(Clone, Copy, Debug)]
pub enum CallbackCall {
    Think {
        entity: EntityId,
        time: ThinkTime,
    },
    Touch {
        entity: EntityId,
        other: EntityId,
    },
    Use {
        entity: EntityId,
        activator: EntityId,
    },
    Blocked {
        entity: EntityId,
        other: EntityId,
    },
    Pain {
        event: DamageEvent,
        taken: i32,
        knockback: i32,
    },
    Die {
        event: DamageEvent,
        taken: i32,
    },
}

impl CallbackCall {
    pub fn entity(self) -> EntityId {
        match self {
            Self::Think { entity, .. }
            | Self::Touch { entity, .. }
            | Self::Use { entity, .. }
            | Self::Blocked { entity, .. } => entity,
            Self::Pain { event, .. } | Self::Die { event, .. } => event.target,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Body {
    pub position: Vec3,
    pub velocity: Vec3,
    pub mins: Vec3,
    pub maxs: Vec3,
}

/// Translation-only attachment capabilities; independently chosen by a module.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyFollow {
    Translation,
    /// The local bounds center replaces the supplied offset.
    Center,
    BoundsMin,
}

/// Engine lifetimes stay internal. Native adapters publish the resulting pose,
/// without adding attachment handles or generations to a legacy protocol.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BodyAttachment {
    pub anchor: EntityId,
    pub follow: BodyFollow,
    pub offset: Vec3,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Bounds {
    pub mins: Vec3,
    pub maxs: Vec3,
}

impl Bounds {
    pub fn overlaps(self, other: Self) -> bool {
        (0..3).all(|axis| {
            self.mins.0[axis] <= other.maxs.0[axis] && self.maxs.0[axis] >= other.mins.0[axis]
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Entity {
    pub id: EntityId,
    pub body: Body,
    pub next_think: Option<Think>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum PlayerTail {
    #[default]
    None,
    Q1 {
        attack_finished: f64,
    },
    Q2 {
        weapon_frame: i32,
    },
    Q3 {
        weapon_time: i32,
    },
}

#[derive(Debug, Default)]
pub struct PlayerState {
    pub values: ValueBank,
    pub movement_rules: RuleSetId,
    /// Caller clipping/filtering policy, independent of movement and geometry.
    pub trace_rules: RuleSetId,
    pub movement: MovementState,
    pub body: Body,
    pub view_angles: Vec3,
    pub health: i32,
    pub armor: i32,
    pub armor_absorption: f32,
    pub armor_energy_absorption: f32,
    pub armor_type: ItemId,
    pub weapon: WeaponId,
    pub pending_weapon: Option<WeaponId>,
    pub inventory: Box<[i32]>,
    pub item_acquired_at: Box<[f64]>,
    pub powerup_until: Box<[f64]>,
    pub collectibles: u64,
    pub view_offset: Vec3,
    pub punch_angles: Vec3,
    /// Native view target supplied by the player/module boundary, not geometry.
    pub ideal_pitch: f32,
    pub flags: u32,
    pub score: i32,
    pub frags: i32,
    pub tail: PlayerTail,
}

/// One rule identity type. Each capability role selects its own value;
/// geometry provenance, module policy and movement need not select the same id.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum RuleSetId {
    Quake = 0,
    QuakeWorld = 1,
    Quake2 = 2,
    Quake2Rerelease = 3,
    #[default]
    Quake3 = 4,
}

impl RuleSetId {
    pub const ALL: [Self; 5] = [
        Self::Quake,
        Self::QuakeWorld,
        Self::Quake2,
        Self::Quake2Rerelease,
        Self::Quake3,
    ];

    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "q1" => Some(Self::Quake),
            "qw" => Some(Self::QuakeWorld),
            "q2" => Some(Self::Quake2),
            "q2rr" => Some(Self::Quake2Rerelease),
            "q3" => Some(Self::Quake3),
            _ => None,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Quake => "q1",
            Self::QuakeWorld => "qw",
            Self::Quake2 => "q2",
            Self::Quake2Rerelease => "q2rr",
            Self::Quake3 => "q3",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MovementMode {
    #[default]
    Walk,
    Fly,
    Noclip,
    Spectator,
    Dead,
    Gib,
    Frozen,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MovementTimer(pub u8);
impl MovementTimer {
    pub const NONE: Self = Self(0);
    pub const WATER_JUMP: Self = Self(1);
    pub const LAND: Self = Self(2);
    pub const TELEPORT: Self = Self(4);
    pub const KNOCKBACK: Self = Self(8);
    pub fn contains(self, flag: Self) -> bool {
        self.0 & flag.0 != 0
    }
    pub fn insert(&mut self, flag: Self) {
        self.0 |= flag.0;
    }
}

/// Hot physics state belongs to the player, independently of module format.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MovementState {
    pub mode: MovementMode,
    pub grounded: bool,
    pub ground: Option<EntityId>,
    pub ground_normal: Vec3,
    pub ground_surface: SurfaceFlags,
    pub water_level: u8,
    pub water_contents: u64,
    pub ladder: bool,
    pub ducked: bool,
    pub jump_held: bool,
    pub timer: MovementTimer,
    pub remaining_ms: u32,
    pub command_time_ms: i32,
    pub delta_angles: Vec3,
    pub previous_position: Vec3,
    pub water_jump_until: f64,
    pub water_jump_seconds: f32,
    pub teleport_hold_until: f64,
    pub tuning: MovementTuning,
}

/// Cached cvar/module overrides; absence retains the selected native default.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MovementTuning {
    pub max_speed: Option<f32>,
    pub gravity: Option<f32>,
    pub friction: Option<f32>,
    pub accelerate: Option<f32>,
    pub air_accelerate: Option<f32>,
    pub water_accelerate: Option<f32>,
    pub water_friction: Option<f32>,
    pub stop_speed: Option<f32>,
    pub gravity_multiplier: f32,
    pub speed_multiplier: f32,
    pub fixed_step_ms: u16,
    pub no_step: bool,
}
impl Default for MovementTuning {
    fn default() -> Self {
        Self {
            max_speed: None,
            gravity: None,
            friction: None,
            accelerate: None,
            air_accelerate: None,
            water_accelerate: None,
            water_friction: None,
            stop_speed: None,
            gravity_multiplier: 1.0,
            speed_multiplier: 1.0,
            fixed_step_ms: 0,
            no_step: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SurfaceFlags(pub u32);
impl SurfaceFlags {
    pub const SLICK: Self = Self(1);
    pub const LADDER: Self = Self(2);
    pub const NO_DAMAGE: Self = Self(4);
    pub const NO_FOOTSTEPS: Self = Self(8);
    pub const METAL_STEPS: Self = Self(16);
    pub fn contains(self, flag: Self) -> bool {
        self.0 & flag.0 != 0
    }
    pub fn from_q2(raw: u32) -> Self {
        Self(u32::from(raw & 2 != 0))
    }
    pub fn from_q3(raw: u32) -> Self {
        Self(
            u32::from(raw & 2 != 0)
                | (u32::from(raw & 8 != 0) << 1)
                | (u32::from(raw & 1 != 0) << 2)
                | (u32::from(raw & 0x2000 != 0) << 3)
                | (u32::from(raw & 0x1000 != 0) << 4),
        )
    }
}

/// Input/AI intent before rule-selected scaling, time and duration.
#[derive(Clone, Copy, Debug, Default)]
pub struct CommandIntent {
    /// Positive/negative fractions for forward, side and up, kept separate so
    /// native integer accumulation can truncate each contribution in order.
    pub movement: [[f32; 2]; 3],
    pub vertical_actions: [f32; 2],
    /// Right/left contributions from turning while strafe is held.
    pub strafe: [f32; 2],
    /// Calibrated mouse deltas routed to side/forward rather than view angles.
    pub mouse_movement: [f32; 2],
    /// Normalized forward/side axes, in device order, after keyboard and mouse.
    pub axes: [[f32; 2]; 16],
    pub axis_count: u8,
    pub speed_modifier: bool,
    pub view_angles: Vec3,
    pub buttons: u32,
    pub impulse: u8,
    pub light_level: u8,
    pub weapon: Option<WeaponId>,
}

impl CommandIntent {
    pub fn moving(axes: [f32; 3]) -> Self {
        Self {
            movement: axes.map(|value| [value.max(0.0), (-value).max(0.0)]),
            ..Self::default()
        }
    }
}

impl PlayerState {
    pub fn with_capacity(items: usize, powerups: usize, values: usize) -> Self {
        Self {
            values: ValueBank::load(values),
            inventory: vec![0; items].into_boxed_slice(),
            item_acquired_at: vec![0.0; items].into_boxed_slice(),
            powerup_until: vec![0.0; powerups].into_boxed_slice(),
            ..Self::default()
        }
    }

    pub fn reset(&mut self) {
        self.values.clear();
        self.inventory.fill(0);
        self.item_acquired_at.fill(0.0);
        self.powerup_until.fill(0.0);
        let inventory = std::mem::take(&mut self.inventory);
        let item_acquired_at = std::mem::take(&mut self.item_acquired_at);
        let powerup_until = std::mem::take(&mut self.powerup_until);
        *self = Self {
            values: std::mem::take(&mut self.values),
            inventory,
            item_acquired_at,
            powerup_until,
            ..Self::default()
        };
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct UserCmd {
    pub duration_ms: u16,
    /// Precise engine duration for NQ; integer-ms ABIs retain duration_ms.
    pub duration_ns: u64,
    pub server_time_ms: i32,
    pub view_angles: Vec3,
    /// Native float commands remain exact; wire codecs narrow at their boundary.
    pub movement: [f32; 3],
    pub buttons: u32,
    pub impulse: u8,
    pub light_level: u8,
    pub weapon: Option<WeaponId>,
}

pub mod buttons {
    pub const ATTACK: u32 = 1;
    pub const JUMP: u32 = 2;
    pub const USE: u32 = 4;
    pub const CROUCH: u32 = 8;
    pub const WALK: u32 = 16;
    pub const ANY: u32 = 32;
    pub const TALK: u32 = 64;
    pub const GESTURE: u32 = 128;
    pub const AFFIRMATIVE: u32 = 256;
    pub const NEGATIVE: u32 = 512;
    pub const GETFLAG: u32 = 1024;
    pub const GUARDBASE: u32 = 2048;
    pub const PATROL: u32 = 4096;
    pub const FOLLOWME: u32 = 8192;
    pub const EXTRA12: u32 = 1 << 14;
    pub const EXTRA13: u32 = 1 << 15;
    pub const EXTRA14: u32 = 1 << 16;
    pub const HOLSTER: u32 = 1 << 17;
}

#[derive(Clone, Copy, Debug)]
pub struct Item {
    pub id: ItemId,
    pub quantity: i32,
}
#[derive(Clone, Copy, Debug)]
pub struct Weapon {
    pub id: WeaponId,
    pub ammo: Option<ItemId>,
}
#[derive(Clone, Copy, Debug)]
pub struct DamageEvent {
    pub target: EntityId,
    pub attacker: Option<EntityId>,
    pub inflictor: Option<EntityId>,
    pub amount: f32,
    pub knockback: i32,
    pub direction: Option<Vec3>,
    pub point: Vec3,
    pub flags: DamageFlags,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct DamageFlags(pub u32);
impl DamageFlags {
    pub const RADIUS: u32 = 1;
    pub const NO_ARMOR: u32 = 2;
    pub const NO_KNOCKBACK: u32 = 4;
    pub const NO_PROTECTION: u32 = 8;
    pub const ENERGY: u32 = 16;
    pub const FALLING: u32 = 32;
    pub fn contains(self, flag: u32) -> bool {
        self.0 & flag != 0
    }
}
#[derive(Clone, Copy, Debug)]
pub struct SoundEvent {
    pub sound: SoundId,
    pub entity: Option<EntityId>,
    pub channel: u16,
    pub position: Vec3,
    pub volume: f32,
    pub attenuation: f32,
    pub action: SoundAction,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SoundAction {
    #[default]
    Play,
    StartLoop,
    Stop,
}
#[derive(Clone, Copy, Debug)]
pub struct EffectEvent {
    pub effect: EffectId,
    pub position: Vec3,
    pub direction: Vec3,
    pub count: u16,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrintKind {
    Console,
    Notify,
    Center,
    Chat,
    Layout,
}
#[derive(Clone, Copy, Debug)]
pub struct PrintEvent {
    pub client: Option<ClientId>,
    pub kind: PrintKind,
    pub text: TextId,
}
#[derive(Debug, PartialEq, Eq)]
pub struct TextLease {
    pub(crate) id: TextId,
}
impl TextLease {
    pub fn id(&self) -> TextId {
        self.id
    }
}
#[derive(Debug)]
pub struct HudLine {
    pub text: TextLease,
    pub started_at: f64,
    pub until: f64,
}
#[derive(Debug, Default)]
pub struct HudState {
    pub values: ValueBank,
    pub clipped_values: u64,
    pub health: i32,
    pub armor: i32,
    pub ammo: i32,
    pub weapon: WeaponId,
    pub armor_type: ItemId,
    pub frags: i32,
    pub score: i32,
    pub item_counts: Box<[i32]>,
    pub item_acquired_at: Box<[f64]>,
    pub owned_items: Box<[u64]>,
    pub owned_weapons: Box<[u64]>,
    pub powerup_until: Box<[f64]>,
    pub collectibles: u64,
    pub layout: NameId,
    pub layout_text: Option<TextLease>,
    pub centerprint: Option<HudLine>,
    pub notify: [Option<HudLine>; 4],
}

impl HudState {
    pub fn with_capacity(items: usize, powerups: usize, weapons: usize, values: usize) -> Self {
        Self {
            values: ValueBank::load(values),
            item_counts: vec![0; items].into_boxed_slice(),
            item_acquired_at: vec![0.0; items].into_boxed_slice(),
            owned_items: vec![0; items.div_ceil(64)].into_boxed_slice(),
            owned_weapons: vec![0; weapons.div_ceil(64)].into_boxed_slice(),
            powerup_until: vec![0.0; powerups].into_boxed_slice(),
            ..Self::default()
        }
    }

    pub fn clear_messages(&mut self, texts: &mut crate::events::TextStore) {
        if let Some(text) = self.layout_text.take() {
            texts.release(text);
        }
        if let Some(line) = self.centerprint.take() {
            texts.release(line.text);
        }
        for line in &mut self.notify {
            if let Some(line) = line.take() {
                texts.release(line.text);
            }
        }
    }

    pub fn reset(&mut self, texts: &mut crate::events::TextStore) {
        self.clear_messages(texts);
        self.values.clear();
        self.item_counts.fill(0);
        self.item_acquired_at.fill(0.0);
        self.owned_items.fill(0);
        self.owned_weapons.fill(0);
        self.powerup_until.fill(0.0);
        *self = Self {
            values: std::mem::take(&mut self.values),
            item_counts: std::mem::take(&mut self.item_counts),
            item_acquired_at: std::mem::take(&mut self.item_acquired_at),
            owned_items: std::mem::take(&mut self.owned_items),
            owned_weapons: std::mem::take(&mut self.owned_weapons),
            powerup_until: std::mem::take(&mut self.powerup_until),
            ..Self::default()
        };
    }
}

/// Internal numeric handles never replace native stat ordinals on the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ValueId(pub u32);
/// Preserve integer and negotiated float payloads without arithmetic or tagging.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NumericValue(pub u32);
#[derive(Debug, Default)]
pub struct ValueBank {
    pub(crate) bits: Box<[u32]>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueWidth {
    Signed16,
    Signed32,
    Float32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueReset {
    Life,
    Session,
}
/// Shared numeric boundary data for module, HUD and protocol consumers.
#[derive(Clone, Copy, Debug)]
pub struct ValueBinding {
    pub id: ValueId,
    pub width: ValueWidth,
    pub reset: ValueReset,
}
