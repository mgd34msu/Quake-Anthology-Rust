//! Engine-owned values. Game formats and module ABIs convert at their boundaries.

/// Fixed guest mapping granule shared by native images and owned child memory.
pub const NATIVE_PAGE_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IndexSpan {
    pub first: u32,
    pub count: u32,
}
impl IndexSpan {
    pub fn contains(self, ordinal: usize) -> bool {
        ordinal
            .checked_sub(self.first as usize)
            .is_some_and(|index| index < self.count as usize)
    }
    pub fn get(self, index: usize) -> Option<usize> {
        (index < self.count as usize).then_some(())?;
        (self.first as usize).checked_add(index)
    }
    pub fn range(self) -> std::ops::Range<usize> {
        self.first as usize..self.first as usize + self.count as usize
    }
    pub fn fits(self, length: usize) -> bool {
        (self.first as usize)
            .checked_add(self.count as usize)
            .is_some_and(|end| end <= length)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(transparent)]
pub struct Vec3(pub [f32; 3]);

impl Vec3 {
    pub const fn axis(index: usize, sign: f32) -> Self {
        Self([
            if index == 0 { sign } else { 0.0 },
            if index == 1 { sign } else { 0.0 },
            if index == 2 { sign } else { 0.0 },
        ])
    }

    pub fn is_finite(self) -> bool {
        self.0.iter().all(|value| value.is_finite())
    }

    pub fn dot(self, other: Self) -> f32 {
        self.0[0] * other.0[0] + self.0[1] * other.0[1] + self.0[2] * other.0[2]
    }

    pub fn lerp(self, end: Self, fraction: f32) -> Self {
        Self(crate::math::lerp_array(self.0, end.0, fraction))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    pub const fn index(self) -> usize {
        match self {
            Self::X => 0,
            Self::Y => 1,
            Self::Z => 2,
        }
    }

    pub const fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::X),
            1 => Some(Self::Y),
            2 => Some(Self::Z),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Plane {
    pub normal: Vec3,
    pub distance: f32,
    pub axis: Option<Axis>,
    /// Original cplane type/sign bits may deliberately differ from the normal,
    /// including temporary boxes and transformed native model contacts.
    pub encoding: Option<[u8; 2]>,
}

#[derive(Clone, Copy, Debug)]
pub struct ClipNode {
    pub plane: u32,
    pub children: [i32; 2],
}

/// Admitted BSP callers supply a negative leaf child.
pub const fn leaf_from_child(child: i32) -> u32 {
    (-1i64 - child as i64) as u32
}
pub const fn child_from_leaf(leaf: u32) -> i32 {
    (-1i64 - leaf as i64) as i32
}

impl Plane {
    pub fn is_valid(self) -> bool {
        self.distance.is_finite() && self.normal.is_finite() && self.normal != Vec3::default()
    }

    pub fn oriented(normal: Vec3, distance: f32) -> Self {
        let axis = (0..3)
            .find(|&a| normal.0[a] == 1.0 && (0..3).all(|b| b == a || normal.0[b] == 0.0))
            .and_then(Axis::from_index);
        Self {
            normal,
            distance,
            axis,
            encoding: None,
        }
    }

    pub fn type_sign(self) -> [u8; 2] {
        self.encoding.unwrap_or_else(|| {
            let kind = self.axis.map_or(3, |axis| axis.index() as u8);
            let signs = self
                .normal
                .0
                .iter()
                .enumerate()
                .fold(0, |bits, (axis, value)| {
                    bits | (u8::from(*value < 0.0) << axis)
                });
            [kind, signs]
        })
    }

    pub fn signed_distance(self, point: Vec3) -> f32 {
        let coordinate = match self.axis {
            Some(axis) => point.0[axis.index()],
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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
    pub const PLAYER: Self = Self(4);
    pub const PROJECTILE: Self = Self(8);
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
impl ThinkTime {
    pub const fn module_clock(rules: RuleSetId) -> Self {
        match rules {
            RuleSetId::Quake | RuleSetId::QuakeWorld | RuleSetId::Quake2 => Self::Seconds(0.0),
            RuleSetId::Quake2Rerelease | RuleSetId::Quake3 => Self::Milliseconds(0),
        }
    }

    pub fn at(self, time: crate::sys_events::EventTime) -> Self {
        match self {
            Self::Seconds(_) => Self::Seconds(time.seconds()),
            Self::Milliseconds(_) => Self::Milliseconds(time.milliseconds() as i64),
        }
    }
    pub fn seconds(self) -> f64 {
        match self {
            Self::Seconds(value) => value,
            Self::Milliseconds(value) => value as f64 / 1000.0,
        }
    }
    pub fn milliseconds(self) -> i64 {
        match self {
            Self::Seconds(value) => (value * 1000.0) as i64,
            Self::Milliseconds(value) => value,
        }
    }
}

/// Borrowed command tokens survive only the synchronous module invocation.
/// The console owns tokenization; ABI adapters only project these bytes.
#[derive(Clone, Copy, Debug, Default)]
pub struct CommandArgs<'a> {
    pub argv: &'a [&'a [u8]],
    pub raw: &'a [u8],
}

#[derive(Clone, Copy, Debug)]
pub enum CallbackCall<'a> {
    /// A module export keeps its native words at the ABI boundary. The entity
    /// supplies the module owner and lifetime to the same dispatch table.
    Export {
        entity: EntityId,
        time: ThinkTime,
        arguments: [u64; 9],
        client: Option<ClientId>,
        command: CommandArgs<'a>,
    },
    Think {
        entity: EntityId,
        time: ThinkTime,
    },
    /// Ordered load-time entity initialization uses the same lifetime-checked
    /// function table. The module ABI decides which call globals it changes.
    Spawn {
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

impl CallbackCall<'_> {
    pub fn entity(self) -> EntityId {
        match self {
            Self::Export { entity, .. }
            | Self::Think { entity, .. }
            | Self::Spawn { entity, .. }
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
impl Body {
    pub fn center(self) -> Vec3 {
        self.position + crate::math::midpoint(self.mins, self.maxs)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LinkOrder {
    Head,
    #[default]
    Tail,
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
    pub fn swept(start: Vec3, end: Vec3, mins: Vec3, maxs: Vec3, margin: f32) -> Self {
        Self {
            mins: Vec3(std::array::from_fn(|axis| {
                (if start.0[axis] < end.0[axis] {
                    start.0[axis]
                } else {
                    end.0[axis]
                }) + mins.0[axis]
                    - margin
            })),
            maxs: Vec3(std::array::from_fn(|axis| {
                (if start.0[axis] < end.0[axis] {
                    end.0[axis]
                } else {
                    start.0[axis]
                }) + maxs.0[axis]
                    + margin
            })),
        }
    }

    pub fn is_valid(self) -> bool {
        (0..3).all(|axis| {
            self.mins.0[axis].is_finite()
                && self.maxs.0[axis].is_finite()
                && self.mins.0[axis] <= self.maxs.0[axis]
        })
    }

    pub fn corner(self, normal: Vec3, far: bool) -> Vec3 {
        Vec3(std::array::from_fn(|axis| {
            if (normal.0[axis] < 0.0) == far {
                self.mins.0[axis]
            } else {
                self.maxs.0[axis]
            }
        }))
    }

    pub fn empty() -> Self {
        Self {
            mins: Vec3([f32::INFINITY; 3]),
            maxs: Vec3([f32::NEG_INFINITY; 3]),
        }
    }

    pub fn add_point(&mut self, point: Vec3) {
        for axis in 0..3 {
            self.mins.0[axis] = self.mins.0[axis].min(point.0[axis]);
            self.maxs.0[axis] = self.maxs.0[axis].max(point.0[axis]);
        }
    }

    pub fn add_bounds(&mut self, other: Self) {
        self.add_point(other.mins);
        self.add_point(other.maxs);
    }

    pub fn overlaps(self, other: Self) -> bool {
        (0..3).all(|axis| {
            self.mins.0[axis] <= other.maxs.0[axis] && self.maxs.0[axis] >= other.mins.0[axis]
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum PlayerTail {
    #[default]
    None,
    Q1 {
        attack_finished: f64,
    },
    Q2,
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
    pub pending_weapon: Option<WeaponId>,
    pub inventory: Inventory,
    pub view_offset: Vec3,
    pub punch_angles: Vec3,
    pub view_weapon: ViewWeapon,
    /// Native view target supplied by the player/module boundary, not geometry.
    pub ideal_pitch: f32,
    pub flags: u32,
    pub score: i32,
    pub frags: i32,
    pub tail: PlayerTail,
}

/// Native resource ordinals are relative to their publishing module/client;
/// they are never renderer asset handles or common entity reservations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceNamespace {
    Module(ModuleId),
    Client(ClientId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelRef {
    pub namespace: ResourceNamespace,
    pub ordinal: u32,
}
impl ModelRef {
    /// Native model ordinal zero means no model in module and wire ABIs.
    pub const fn native(namespace: ResourceNamespace, ordinal: u32) -> Option<Self> {
        if ordinal == 0 {
            None
        } else {
            Some(Self { namespace, ordinal })
        }
    }
}

/// Residual source-native appearance bits are interpreted using the enclosing
/// EntityAppearance's source, independently of movement or trace rules.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NativeAppearanceTail {
    pub colormap: u32,
    pub effects: u64,
    pub render_flags: u32,
    pub event: u32,
    pub loop_sound: u32,
    pub solid: u32,
    pub instance_bits: u8,
    pub loop_volume: f32,
    pub loop_attenuation: f32,
    pub owner: i32,
    pub old_frame: i32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EntityAppearance {
    pub source: RuleSetId,
    pub models: [Option<ModelRef>; 4],
    pub frame: i32,
    pub skin: i32,
    pub old_origin: Vec3,
    pub alpha: f32,
    pub scale: f32,
    pub native: NativeAppearanceTail,
}
impl Default for EntityAppearance {
    fn default() -> Self {
        Self {
            source: RuleSetId::default(),
            models: [None; 4],
            frame: 0,
            skin: 0,
            old_origin: Vec3::default(),
            alpha: 1.0,
            scale: 1.0,
            native: NativeAppearanceTail::default(),
        }
    }
}

/// Retained entity presentation at a file, module or wire boundary.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EntityRecord {
    pub position: Vec3,
    pub velocity: Vec3,
    pub angles: Vec3,
    pub appearance: EntityAppearance,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViewWeapon {
    pub model: Option<ModelRef>,
    pub frame: i32,
    pub offset: Vec3,
    pub angles: Vec3,
    pub skin: i32,
    pub rate: i32,
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

/// Derived file-decoding lineage; capability roles still select RuleSetId.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lineage {
    Quake,
    Quake2,
    Quake3,
}

impl RuleSetId {
    pub const fn lineage(self) -> Lineage {
        match self {
            Self::Quake | Self::QuakeWorld => Lineage::Quake,
            Self::Quake2 | Self::Quake2Rerelease => Lineage::Quake2,
            Self::Quake3 => Lineage::Quake3,
        }
    }

    pub const ALL: [Self; 5] = [
        Self::Quake,
        Self::QuakeWorld,
        Self::Quake2,
        Self::Quake2Rerelease,
        Self::Quake3,
    ];

    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "q1" | "q1-netquake" => Some(Self::Quake),
            "qw" | "q1-quakeworld" => Some(Self::QuakeWorld),
            "q2" | "q2-classic" => Some(Self::Quake2),
            "q2rr" | "q2-rerelease" => Some(Self::Quake2Rerelease),
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
    pub const TRICK: Self = Self(16);
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
    pub ground_contents: u64,
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
    pub water_jump_seconds: f32,
    pub teleport_hold_until: f64,
    pub tuning: MovementTuning,
}

/// Cached cvar/module overrides; absence retains the selected native default.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MovementTuning {
    pub max_speed: Option<f32>,
    pub spectator_max_speed: Option<f32>,
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
    pub n64_physics: bool,
}
impl Default for MovementTuning {
    fn default() -> Self {
        Self {
            max_speed: None,
            spectator_max_speed: None,
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
            n64_physics: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SurfaceFlags(pub u64);
impl SurfaceFlags {
    pub const SLICK: Self = Self(1);
    pub const LADDER: Self = Self(2);
    pub const NO_DAMAGE: Self = Self(4);
    pub const NO_FOOTSTEPS: Self = Self(8);
    pub const METAL_STEPS: Self = Self(16);
    pub const SKY: Self = Self(1 << 36);
    const Q2_BITS: [u8; 32] = [
        5, 0, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27,
        28, 29, 30, 31, 32, 33, 34, 35,
    ];
    const Q3_BITS: [u8; 32] = [
        2, 0, 36, 1, 37, 38, 39, 11, 40, 41, 42, 43, 4, 3, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53,
        54, 55, 56, 57, 58, 59, 60, 61,
    ];
    pub fn contains(self, flag: Self) -> bool {
        self.0 & flag.0 != 0
    }
    pub fn from_q2(raw: u32) -> Self {
        Self::decode(raw, &Self::Q2_BITS)
    }
    pub fn from_q3(raw: u32) -> Self {
        Self::decode(raw, &Self::Q3_BITS)
    }
    pub fn to_q2(self) -> u32 {
        self.encode(&Self::Q2_BITS) | if self.contains(Self::SKY) { 4 } else { 0 }
    }
    pub fn to_q3(self) -> u32 {
        self.encode(&Self::Q3_BITS) | if self.0 & (1 << 6) != 0 { 4 | 16 } else { 0 }
    }
    fn decode(raw: u32, bits: &[u8; 32]) -> Self {
        Self(bits.iter().enumerate().fold(0, |flags, (bit, &target)| {
            flags | (u64::from((raw >> bit) & 1) << target)
        }))
    }
    fn encode(self, bits: &[u8; 32]) -> u32 {
        bits.iter().enumerate().fold(0, |flags, (bit, &source)| {
            flags | (((self.0 >> source) as u32 & 1) << bit)
        })
    }
}

/// A collision contact retains its resource lifetime and original surface row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SurfaceId {
    pub geometry: GeometryId,
    pub index: u32,
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
    pub fn eye(&self) -> Vec3 {
        self.body.position + self.view_offset
    }

    pub fn with_capacity(items: usize, powerups: usize, values: usize) -> Self {
        Self {
            values: ValueBank::load(values),
            inventory: Inventory::with_capacity(items, powerups),
            ..Self::default()
        }
    }

    pub fn reset(&mut self) {
        self.values.clear();
        self.inventory.reset();
        let inventory = std::mem::take(&mut self.inventory);
        *self = Self {
            values: std::mem::take(&mut self.values),
            inventory,
            ..Self::default()
        };
    }
}

/// Transport-selected command consumption, independent of movement rules.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CommandExecution {
    /// Keep the latest command and resolve its duration at the SERVER frame.
    FrameLatest,
    /// Consume each admitted command before admitting the next one.
    #[default]
    PerCommand,
}

/// SERVER provenance of a submitted snapshot, observed through a packet's
/// native last-frame field. It is not a sampling time for each backup command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotObservation {
    pub module: ModuleId,
    pub module_lifetime: EntityId,
    pub native_frame: u32,
    pub time: crate::sys_events::EventTime,
}
impl SnapshotObservation {
    pub fn frame_for(self, module: ModuleId, lifetime: EntityId) -> Option<u32> {
        (self.module == module && self.module_lifetime == lifetime).then_some(self.native_frame)
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
/// Voice identity retains a common lifetime, native slot or static registration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoundEmitter {
    Entity(EntityId),
    Native {
        namespace: ResourceNamespace,
        slot: i32,
    },
    /// Persistent map registrations have no native edict or moving pose.
    Static {
        namespace: ResourceNamespace,
        lifetime: EntityId,
        slot: u32,
    },
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SoundEvent {
    pub sound: SoundId,
    /// The source boundary's ordinal, retained without replacing PCM identity.
    pub source_ordinal: Option<u32>,
    /// Spatial behavior belongs to this emission, independently of shared PCM.
    pub spatial_rules: RuleSetId,
    pub entity: Option<SoundEmitter>,
    pub channel: u16,
    pub position: Vec3,
    pub volume: f32,
    pub attenuation: f32,
    pub action: SoundAction,
    pub client: Option<ClientId>,
    pub time_offset: f32,
    /// The emission needs its supplied position rather than an entity origin.
    pub positioned: bool,
}
impl SoundEvent {
    pub fn applies_to_client(self, client: ClientId) -> bool {
        self.client.is_none_or(|recipient| recipient == client)
    }
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
impl PrintKind {
    pub const fn from_localized_level(level: u8) -> Self {
        match level {
            4 | 5 => Self::Center,
            other => Self::from_level(other),
        }
    }
    pub const fn from_level(level: u8) -> Self {
        match level {
            0 | 1 => Self::Notify,
            3 => Self::Chat,
            _ => Self::Console,
        }
    }
    pub const fn default_level(self) -> u8 {
        match self {
            Self::Chat => 3,
            Self::Notify => 1,
            _ => 2,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct PrintEvent {
    pub client: Option<ClientId>,
    pub kind: PrintKind,
    /// Native byte priority is independent of display kind. Protocols without
    /// this field omit it at their boundary; 0 and 1 must remain distinct.
    pub level: u8,
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
/// Recipient filtering uses compiled gameplay visibility, never render PVS.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeTarget {
    All,
    Sight,
    Hearing,
    Client(ClientId),
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NativeDelivery {
    pub module: ModuleId,
    pub source: RuleSetId,
    pub target: NativeTarget,
    pub origin: Vec3,
    pub reliable: bool,
    pub dupe_key: u32,
    /// Captured at publication, never reconstructed from a later host frame.
    pub emission: Option<NativeEmission>,
    pub check_portals: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeEmission {
    pub module_lifetime: EntityId,
    pub provider_frame: u64,
}
/// Immutable native-cue boundary identities, stored with the leased bytes.
pub const NATIVE_WIRE_BYTES: usize = 32 * 1024;
pub const NATIVE_CUE_CAPTURE_BYTES: usize = 16;
/// An opcode, flags byte and 8-bit sound ordinal are the smallest native cue.
pub const NATIVE_CUE_MIN_WIRE_BYTES: usize = 3;
pub const NATIVE_CUE_CAPACITY: usize = NATIVE_WIRE_BYTES / NATIVE_CUE_MIN_WIRE_BYTES;
pub const NATIVE_PAYLOAD_BYTES: usize =
    NATIVE_WIRE_BYTES + NATIVE_CUE_CAPACITY * NATIVE_CUE_CAPTURE_BYTES;
pub const OUTPUT_PAGE_BYTES: usize = 8192;
pub const NATIVE_WIRE_PAGES: usize = NATIVE_WIRE_BYTES / OUTPUT_PAGE_BYTES;
pub const NATIVE_PAYLOAD_PAGES: usize = NATIVE_PAYLOAD_BYTES.div_ceil(OUTPUT_PAGE_BYTES);

#[derive(Clone, Copy, Debug)]
pub struct NativeCueBinding {
    pub sound: Option<SoundId>,
    pub entity: Option<EntityId>,
}
impl NativeCueBinding {
    pub fn write_capture(self, offset: u32, output: &mut [u8]) -> Option<()> {
        let row = output.get_mut(..NATIVE_CUE_CAPTURE_BYTES)?;
        row[..4].copy_from_slice(&offset.to_le_bytes());
        row[4..8].copy_from_slice(&self.sound.map_or(u32::MAX, |sound| sound.0).to_le_bytes());
        row[8..12].copy_from_slice(&self.entity.map_or(u32::MAX, |id| id.slot).to_le_bytes());
        row[12..16].copy_from_slice(&self.entity.map_or(0, |id| id.generation).to_le_bytes());
        Some(())
    }
}
/// Wire bytes and immutable cue captures use the existing leased payload pages.
#[derive(Clone, Copy, Debug)]
pub struct NativePayload {
    pub(crate) pages: [Option<TextId>; NATIVE_PAYLOAD_PAGES],
    pub(crate) length: usize,
    pub(crate) stored_length: usize,
}
impl NativePayload {
    pub fn len(self) -> usize {
        self.length
    }
    pub fn is_empty(self) -> bool {
        self.length == 0
    }
    /// Sorted (native-service byte offset, captured PCM identity) entries live
    /// in the same leased pages as the original bytes, never module storage.
    pub fn cue(self, stored: &[u8], offset: usize) -> Option<NativeCueBinding> {
        let rows = stored.get(self.length..self.stored_length)?;
        let mut low = 0;
        let mut high = rows.len() / NATIVE_CUE_CAPTURE_BYTES;
        while low < high {
            let index = low + (high - low) / 2;
            let first = index * NATIVE_CUE_CAPTURE_BYTES;
            let row = rows.get(first..first + NATIVE_CUE_CAPTURE_BYTES)?;
            let at = u32::from_le_bytes(row[..4].try_into().ok()?) as usize;
            match at.cmp(&offset) {
                std::cmp::Ordering::Less => low = index + 1,
                std::cmp::Ordering::Greater => high = index,
                std::cmp::Ordering::Equal => {
                    let sound = u32::from_le_bytes(row[4..8].try_into().ok()?);
                    let slot = u32::from_le_bytes(row[8..12].try_into().ok()?);
                    let generation = u32::from_le_bytes(row[12..16].try_into().ok()?);
                    return Some(NativeCueBinding {
                        sound: (sound != u32::MAX).then_some(SoundId(sound)),
                        entity: (slot != u32::MAX).then_some(EntityId { slot, generation }),
                    });
                }
            }
        }
        None
    }
}
#[derive(Clone, Copy, Debug)]
pub struct HudEvent {
    pub client: ClientId,
    pub change: HudChange,
}
#[derive(Clone, Copy, Debug)]
pub enum HudChange {
    Item {
        item: ItemId,
        count: i32,
        acquired_at: f64,
    },
    Weapon(WeaponId),
    Powerup {
        slot: u32,
        until: f64,
    },
    Collectibles(u64),
}
#[derive(Debug, Default)]
pub struct Inventory {
    pub item_counts: Box<[i32]>,
    pub item_acquired_at: Box<[f64]>,
    pub powerup_until: Box<[f64]>,
    pub weapon: WeaponId,
    pub collectibles: u64,
}
impl Inventory {
    pub fn with_capacity(items: usize, powerups: usize) -> Self {
        Self {
            item_counts: vec![0; items].into_boxed_slice(),
            item_acquired_at: vec![0.0; items].into_boxed_slice(),
            powerup_until: vec![0.0; powerups].into_boxed_slice(),
            ..Self::default()
        }
    }
    pub fn reset(&mut self) {
        self.item_counts.fill(0);
        self.item_acquired_at.fill(0.0);
        self.powerup_until.fill(0.0);
        self.weapon = WeaponId::default();
        self.collectibles = 0;
    }
    pub fn apply(&mut self, change: HudChange) -> bool {
        match change {
            HudChange::Item {
                item,
                count,
                acquired_at,
            } => {
                let index = item.0 as usize;
                let (Some(value), Some(time)) = (
                    self.item_counts.get_mut(index),
                    self.item_acquired_at.get_mut(index),
                ) else {
                    return false;
                };
                *value = count;
                *time = acquired_at;
            }
            HudChange::Weapon(weapon) => self.weapon = weapon,
            HudChange::Powerup { slot, until } => {
                let Some(value) = self.powerup_until.get_mut(slot as usize) else {
                    return false;
                };
                *value = until;
            }
            HudChange::Collectibles(value) => self.collectibles = value,
        }
        true
    }
}
#[derive(Debug, Default)]
pub struct HudState {
    pub values: ValueBank,
    pub clipped_values: u64,
    pub health: i32,
    pub armor: i32,
    pub ammo: i32,
    pub inventory: Inventory,
    pub armor_type: ItemId,
    pub frags: i32,
    pub score: i32,
    pub owned_items: crate::bits::BitSet,
    pub owned_weapons: crate::bits::BitSet,
    pub layout: NameId,
    pub layout_text: Option<TextLease>,
    pub centerprint: Option<HudLine>,
    pub notify: [Option<HudLine>; 4],
}

impl HudState {
    pub fn with_capacity(items: usize, powerups: usize, weapons: usize, values: usize) -> Self {
        Self {
            values: ValueBank::load(values),
            inventory: Inventory::with_capacity(items, powerups),
            owned_items: crate::bits::BitSet::empty(items),
            owned_weapons: crate::bits::BitSet::empty(weapons),
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

    pub fn reset_inventory(&mut self) {
        self.inventory.reset();
        self.owned_items.clear();
        self.owned_weapons.clear();
        self.ammo = 0;
    }

    pub fn reset(&mut self, texts: &mut crate::events::TextStore) {
        self.clear_messages(texts);
        self.values.clear();
        self.reset_inventory();
        *self = Self {
            values: std::mem::take(&mut self.values),
            inventory: std::mem::take(&mut self.inventory),
            owned_items: std::mem::take(&mut self.owned_items),
            owned_weapons: std::mem::take(&mut self.owned_weapons),
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
/// Native stat/persistent counts and module value widths, in RuleSetId order.
pub const NATIVE_VALUE_LAYOUT: [(u32, u32, ValueWidth); 5] = [
    (32, 0, ValueWidth::Signed32),
    (32, 0, ValueWidth::Signed32),
    (32, 0, ValueWidth::Signed16),
    (64, 0, ValueWidth::Signed16),
    (16, 16, ValueWidth::Signed32),
];
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
