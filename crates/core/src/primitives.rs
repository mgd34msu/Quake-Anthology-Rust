//! Engine-owned values. Game formats and module ABIs convert at their boundaries.
pub use crate::pcm::{PcmFormat, PcmWidth};

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

/// Copied lightstyle values shared by gameplay providers and render commands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightStyle {
    pub rgb: [f32; 3],
    pub indexed_scale: u16,
}
impl Default for LightStyle {
    fn default() -> Self {
        Self {
            rgb: [1.0; 3],
            indexed_scale: 256,
        }
    }
}

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

/// An attachment's affine pose; axes retain the file's native orientation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Orientation {
    pub origin: Vec3,
    pub axes: [Vec3; 3],
}
impl Default for Orientation {
    fn default() -> Self {
        Self {
            origin: Vec3::default(),
            axes: std::array::from_fn(|axis| Vec3::axis(axis, 1.0)),
        }
    }
}
impl Orientation {
    /// Native tag interpolation uses weighted sums, followed by independent
    /// axis normalization; subtract-then-lerp rounds differently.
    pub fn interpolate(self, end: Self, fraction: f32) -> Self {
        let back = 1.0 - fraction;
        Self {
            origin: self.origin * back + end.origin * fraction,
            axes: std::array::from_fn(|axis| {
                crate::math::normalized(self.axes[axis] * back + end.axes[axis] * fraction)
            }),
        }
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

/// A caller's point-leaf policy; render and gameplay preserve their distinct
/// plane arithmetic and equality ownership over the same immutable BSP.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PointLeafRules {
    pub back_on_tie: bool,
    pub axial_shortcut: bool,
}

impl PointLeafRules {
    pub const RENDER: Self = Self {
        back_on_tie: true,
        axial_shortcut: false,
    };
    pub const CLIP: Self = Self {
        back_on_tie: false,
        axial_shortcut: true,
    };
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
    pub fn from_points(a: Vec3, b: Vec3, c: Vec3) -> Option<Self> {
        let mut normal = crate::math::cross(c - a, b - a);
        let length = crate::math::normalize(&mut normal);
        (length > 0. && length.is_finite()).then(|| Self::oriented(normal, a.dot(normal)))
    }

    /// Express a world plane in coordinates whose basis columns are `axes`.
    pub fn to_local(self, origin: Vec3, axes: [Vec3; 3]) -> Self {
        Self::oriented(
            Vec3(axes.map(|axis| self.normal.dot(axis))),
            self.distance - self.normal.dot(origin),
        )
    }

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
        match self.axis {
            Some(axis) => point.0[axis.index()] - self.distance,
            None => self.geometric_distance(point),
        }
    }

    pub fn geometric_distance(self, point: Vec3) -> f32 {
        self.normal.dot(point) - self.distance
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
/// Exact publication owner; a reused CLIENT slot is a different lifetime.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MusicOwner {
    /// None is an engine-owned request, distinct from every native module id.
    pub module: Option<ModuleId>,
    pub client: Option<(ClientId, EntityId)>,
}
/// Transition and sample rules selected at the source boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MusicPlayback {
    pub wave_first: bool,
    pub smooth_volume: bool,
    pub preserve_queued: bool,
    pub signed_stereo_bytes: bool,
}
pub const MUSIC_REMAP_TRACKS: usize = 256;
/// One decoded view of an atomic music remap payload.
pub struct MusicRemap<'a>(&'a [[u8; 4]]);
impl<'a> MusicRemap<'a> {
    pub fn parse(bytes: &'a [u8]) -> Option<Self> {
        let (values, remainder) = bytes.as_chunks::<4>();
        if values.is_empty() || values.len() >= MUSIC_REMAP_TRACKS || !remainder.is_empty() {
            None
        } else {
            Some(Self(values))
        }
    }
    pub fn tracks(self) -> impl ExactSizeIterator<Item = u32> + 'a {
        self.0.iter().map(|bytes| u32::from_le_bytes(*bytes))
    }
}
#[derive(Clone, Copy, Debug)]
pub enum MusicQuery {
    Info { volume: f32 },
    Remaps,
}
#[derive(Clone, Copy, Debug)]
pub enum MusicControl {
    Enabled(bool),
    Reset,
    /// Little-endian u32 destinations for source tracks 1 onward.
    Remap {
        entries: TextId,
    },
    Report(MusicQuery),
    Track {
        owner: MusicOwner,
        playback: MusicPlayback,
        number: u32,
        looping: bool,
    },
    Start {
        owner: MusicOwner,
        playback: MusicPlayback,
        intro: TextId,
        loop_track: TextId,
    },
    Stop {
        only_active: bool,
    },
    Pause(bool),
    Retire(MusicOwner),
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModelId(pub u32);
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
    /// Exclusive native cue/LIST extent, independent of the full waveform.
    pub loop_end: Option<usize>,
    /// Optional metadata was malformed. The waveform remains available to
    /// capabilities that ignore cues; cue-aware admission rejects it.
    pub loop_invalid: bool,
}
impl Pcm {
    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels as usize
    }
    pub fn cue_frames(&self) -> usize {
        self.loop_end.unwrap_or_else(|| self.frames())
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EffectId(pub u32);
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NameId(pub u32);
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModuleId(pub u16);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandProducer {
    pub module: ModuleId,
    pub anchor: EntityId,
}

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
    Capsule,
    Model {
        geometry: GeometryId,
        index: u32,
    },
}

/// A swept capsule is a sphere around a centered line segment. Its native
/// vertical size comes from the supplied X and Z extents, independently of
/// the map or movement role. Model transforms may rotate its segment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Capsule {
    pub radius: f32,
    pub offset: Vec3,
}
impl Capsule {
    pub fn contains(self, center: Vec3, point: Vec3) -> bool {
        let delta = point - center;
        let length_squared = self.offset.dot(self.offset);
        let fraction = if length_squared == 0.0 {
            0.0
        } else {
            (delta.dot(self.offset) / length_squared).clamp(-1.0, 1.0)
        };
        let nearest = delta - self.offset * fraction;
        nearest.dot(nearest) <= self.radius * self.radius
    }

    pub fn from_bounds(bounds: Bounds) -> Self {
        let half = bounds.maxs - crate::math::midpoint(bounds.mins, bounds.maxs);
        let radius = if half.0[0] > half.0[2] {
            half.0[2]
        } else {
            half.0[0]
        };
        Self {
            radius,
            offset: Vec3([0.0, 0.0, half.0[2] - radius]),
        }
    }
    pub fn swept_bounds(self, start: Vec3, end: Vec3, margin: f32) -> Bounds {
        let endpoints = std::array::from_fn::<_, 3, _>(|axis| {
            if start.0[axis] < end.0[axis] {
                (start.0[axis], end.0[axis])
            } else {
                (end.0[axis], start.0[axis])
            }
        });
        Bounds {
            mins: Vec3(std::array::from_fn(|axis| {
                // Native fabs promotes the offset and both following size
                // operations to double before storing the bound as float.
                (f64::from(endpoints[axis].0)
                    - f64::from(self.offset.0[axis]).abs()
                    - f64::from(self.radius)) as f32
                    - margin
            })),
            maxs: Vec3(std::array::from_fn(|axis| {
                (f64::from(endpoints[axis].1)
                    + f64::from(self.offset.0[axis]).abs()
                    + f64::from(self.radius)) as f32
                    + margin
            })),
        }
    }
    pub fn closest_segment(self, start: Vec3, end: Vec3, normal: Vec3) -> (Vec3, Vec3) {
        if normal.dot(self.offset) > 0.0 {
            (start - self.offset, end - self.offset)
        } else {
            (start + self.offset, end + self.offset)
        }
    }
    pub fn to_local(mut self, axes: [Vec3; 3]) -> Self {
        self.offset = if self.offset.0[0] == 0.0 && self.offset.0[1] == 0.0 {
            // Native transformed capsules form this matrix column directly.
            // Adding zero products would change signed-zero rounding.
            Vec3(axes.map(|axis| axis.0[2] * self.offset.0[2]))
        } else {
            crate::math::inverse_transform_point(Vec3::default(), axes, self.offset)
        };
        self
    }
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

/// A supplied native clock sample tied to the queued engine timeline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClockObservation {
    pub native: ThinkTime,
    pub queued: crate::sys_events::EventTime,
}
impl ClockObservation {
    pub fn is_valid(self) -> bool {
        match self.native {
            ThinkTime::Seconds(value) => value.is_finite(),
            ThinkTime::Milliseconds(_) => true,
        }
    }

    /// Preserve past deadlines. Unit mismatch and nonfinite samples reject.
    pub fn deadline(self, deadline: ThinkTime) -> Option<f64> {
        let mapped = match (self.native, deadline) {
            (ThinkTime::Seconds(observed), ThinkTime::Seconds(deadline)) => {
                if !observed.is_finite() || !deadline.is_finite() {
                    return None;
                }
                self.queued.seconds() + (deadline - observed)
            }
            (ThinkTime::Milliseconds(observed), ThinkTime::Milliseconds(deadline)) => {
                let nanos = i128::from(self.queued.0)
                    + (i128::from(deadline) - i128::from(observed))
                        * i128::from(crate::sys_events::NANOS_PER_MILLISECOND);
                nanos as f64 / crate::sys_events::NANOS_PER_SECOND as f64
            }
            _ => return None,
        };
        mapped.is_finite().then_some(mapped)
    }
}

/// A camera-only response. Its clock is the accepted native frame sample;
/// player movement and command angles remain independently owned.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewFeedback {
    pub kick: Vec3,
    pub screen: [f32; 2],
    pub value: f32,
    pub clock: ClockObservation,
    pub deflect_ms: f32,
    pub return_ms: f64,
}
impl ViewFeedback {
    /// Native integer presentation clocks subtract after conversion to float.
    /// The return division is float; its outer subtraction is double.
    pub fn kick_at(self, now: ThinkTime) -> Option<Vec3> {
        let elapsed = match (self.clock.native, now) {
            (ThinkTime::Milliseconds(start), ThinkTime::Milliseconds(now)) => {
                now as f32 - start as f32
            }
            (ThinkTime::Seconds(start), ThinkTime::Seconds(now)) => ((now - start) * 1000.0) as f32,
            _ => return None,
        };
        if !elapsed.is_finite()
            || !self.deflect_ms.is_finite()
            || self.deflect_ms <= 0.0
            || !self.return_ms.is_finite()
            || self.return_ms <= 0.0
        {
            return None;
        }
        let ratio = if elapsed < self.deflect_ms {
            elapsed / self.deflect_ms
        } else {
            let remaining = (elapsed - self.deflect_ms) / self.return_ms as f32;
            let ratio = (1.0 - f64::from(remaining)) as f32;
            if ratio <= 0.0 {
                return None;
            }
            ratio
        };
        Some(self.kick * ratio)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewFeedbackEvent {
    pub client: ClientId,
    pub client_lifetime: EntityId,
    pub source: RuleSetId,
    pub namespace: ResourceNamespace,
    pub native_client: u32,
    pub feedback: ViewFeedback,
}

/// Borrowed command tokens survive only the synchronous module invocation.
/// The console owns tokenization; ABI adapters only project these bytes.
#[derive(Clone, Copy, Debug, Default)]
pub struct CommandArgs<'a> {
    pub argv: &'a [&'a [u8]],
    pub raw: &'a [u8],
}

/// Lifecycle phases use the owning module's loaded ABI policy. They do not
/// select movement, trace, presentation or the connection's wire protocol.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientCallback {
    NewParameters,
    Connect,
    Begin,
    BeforeMove,
    AfterMove,
    Disconnect,
    Command,
    UserinfoChanged,
}

#[derive(Clone, Copy, Debug)]
pub enum CallbackCall<'a> {
    Client {
        entity: EntityId,
        client: ClientId,
        event: ClientCallback,
        time: ThinkTime,
        arguments: [u64; 9],
        command: CommandArgs<'a>,
    },
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
        /// The first impact callback may remove this native row before the
        /// reciprocal callback runs. Only callback-self requires liveness.
        other: NativeEntityRef,
        /// Some selects a native trigger clock; contacts preserve globals.
        time: Option<ThinkTime>,
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
            Self::Client { entity, .. }
            | Self::Export { entity, .. }
            | Self::Think { entity, .. }
            | Self::Spawn { entity, .. }
            | Self::Touch { entity, .. }
            | Self::Use { entity, .. }
            | Self::Blocked { entity, .. } => entity,
            Self::Pain { event, .. } | Self::Die { event, .. } => event.target,
        }
    }
}

/// Entity motion capabilities, independent of the module and wire identities.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Motion {
    #[default]
    None,
    Walk,
    Step,
    Fly,
    FlyMissile,
    Toss,
    Bounce,
    Push,
    Stop,
    Noclip,
    Unsupported,
}

/// Scalar ABI synchronization after common motion arithmetic. A native store
/// may overlap the inputs of a later component.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MotionWrite {
    Angles(usize),
    AngularVelocity(usize),
    Origin(usize),
    Velocity(usize),
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

/// Native readiness quantities retain their units and original numeric width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WeaponReadiness {
    DeadlineSeconds(f32),
    RemainingMilliseconds(i32),
}

/// Weapon ownership is independent of movement, presentation and wire format.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeaponState {
    pub source: RuleSetId,
    pub namespace: ResourceNamespace,
    pub readiness: WeaponReadiness,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlayerSignal {
    pub code: u32,
    pub parameter: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlayerDamageSample {
    pub sequence: u32,
    pub yaw: u32,
    pub pitch: u32,
    pub count: u32,
}

/// Raw native signals share the enclosing player tail's source and namespace.
/// These two native event cells are ABI state, not command recording/history.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlayerSignals {
    pub sequence: u32,
    pub events: [PlayerSignal; 2],
    pub external: PlayerSignal,
    pub damage: PlayerDamageSample,
}

/// Player animation and native presentation state retain the enclosing tail's
/// source and resource namespace. Animation toggle bits and native ordinals
/// remain intact until a presentation or protocol boundary interprets them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayerPresentation {
    pub bob_cycle: u32,
    pub legs_timer: i32,
    pub legs_animation: u32,
    pub torso_timer: i32,
    pub torso_animation: u32,
    pub movement_direction: i32,
    pub grapple_point: Vec3,
    pub entity_flags: u32,
    pub weapon_phase: i32,
    pub generic: i32,
    pub jump_pad_entity: u32,
}

/// Native player metadata retains source widths independently of movement and
/// connection identity. Hot movement fields overlay only their mapped flags.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NativePlayerTail {
    pub source: RuleSetId,
    pub namespace: ResourceNamespace,
    pub flags: u32,
    pub client_number: Option<u32>,
    pub ground_number: Option<u32>,
    pub weapon_number: Option<u32>,
    pub gravity: Option<i32>,
    pub speed: Option<i32>,
    pub signals: Option<PlayerSignals>,
    pub presentation: Option<PlayerPresentation>,
}
impl NativePlayerTail {
    pub const fn new(source: RuleSetId, namespace: ResourceNamespace) -> Self {
        Self {
            source,
            namespace,
            flags: 0,
            client_number: None,
            ground_number: None,
            weapon_number: None,
            gravity: None,
            speed: None,
            signals: None,
            presentation: None,
        }
    }
}

/// Accepted protocol capabilities, independent of movement and presentation.
/// A native client may predict angles while retaining its received position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PredictionPolicy {
    pub position: bool,
    pub angles: bool,
}
impl PredictionPolicy {
    pub const DISABLED: Self = Self {
        position: false,
        angles: false,
    };
}
impl Default for PredictionPolicy {
    fn default() -> Self {
        Self {
            position: true,
            angles: true,
        }
    }
}

#[derive(Debug, Default)]
pub struct PlayerState {
    pub values: ValueBank,
    pub movement_rules: RuleSetId,
    /// Caller clipping/filtering policy, independent of movement and geometry.
    pub trace_rules: RuleSetId,
    pub movement: MovementState,
    pub prediction: PredictionPolicy,
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
    /// Player presentation can carry a loop when its native entity is omitted
    /// from the snapshot. The resource remains in its explicit namespace.
    pub loop_sound: Option<SoundRef>,
    /// Native view target supplied by the player/module boundary, not geometry.
    pub ideal_pitch: f32,
    pub flags: u32,
    pub score: i32,
    pub frags: i32,
    pub weapon_state: Option<WeaponState>,
    pub native: Option<NativePlayerTail>,
    /// Accepted owning module or wire clock observation. Movement command time
    /// never supplies this sample; native deadline projection owns its update.
    pub native_clock: Option<ClockObservation>,
}

/// Native resource ordinals are relative to their publishing module/client;
/// they are never renderer asset handles or common entity reservations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceNamespace {
    Module(ModuleId),
    Client(ClientId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResourceRef<T> {
    pub namespace: ResourceNamespace,
    pub ordinal: u32,
    marker: std::marker::PhantomData<T>,
}
impl<T> ResourceRef<T> {
    /// Native ordinal zero means no resource in module and wire ABIs.
    pub const fn native(namespace: ResourceNamespace, ordinal: u32) -> Option<Self> {
        if ordinal == 0 {
            None
        } else {
            Some(Self {
                namespace,
                ordinal,
                marker: std::marker::PhantomData,
            })
        }
    }
}
pub type ModelRef = ResourceRef<ModelId>;
pub type SoundRef = ResourceRef<SoundId>;
/// Weak entity ordinal in an explicitly selected module or CLIENT namespace.
/// Unlike model/sound resources, native entity zero is an ordinary reference.
pub type NativeEntityRef = ResourceRef<EntityId>;
pub type NativeClientRef = ResourceRef<ClientId>;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeReference {
    Entity(NativeEntityRef),
    Client(NativeClientRef),
}
impl NativeReference {
    pub const fn namespace(self) -> ResourceNamespace {
        match self {
            Self::Entity(value) => value.namespace,
            Self::Client(value) => value.namespace,
        }
    }
    pub const fn ordinal(self) -> u32 {
        match self {
            Self::Entity(value) => value.ordinal,
            Self::Client(value) => value.ordinal,
        }
    }
}
impl ResourceRef<EntityId> {
    pub const fn entity(namespace: ResourceNamespace, ordinal: u32) -> Self {
        Self {
            namespace,
            ordinal,
            marker: std::marker::PhantomData,
        }
    }
}
impl ResourceRef<ClientId> {
    pub const fn client(namespace: ResourceNamespace, ordinal: u32) -> Self {
        Self {
            namespace,
            ordinal,
            marker: std::marker::PhantomData,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TrajectoryCurve {
    #[default]
    Stationary,
    Interpolate,
    Linear,
    LinearStop,
    Sine,
    Gravity,
    Unsupported(u32),
}

/// Published motion is independent of the linked physical Body. Native time
/// arithmetic and gravity are supplied by the source's trajectory capability.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Trajectory {
    pub curve: TrajectoryCurve,
    pub start: ThinkTime,
    pub duration_ms: i64,
    pub base: Vec3,
    pub delta: Vec3,
}
impl Default for Trajectory {
    fn default() -> Self {
        Self {
            curve: TrajectoryCurve::Stationary,
            start: ThinkTime::Milliseconds(0),
            duration_ms: 0,
            base: Vec3::default(),
            delta: Vec3::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EntityTrajectories {
    pub position: Trajectory,
    pub angles: Trajectory,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EntityAnimation {
    pub legs: u32,
    pub torso: u32,
}

/// The source/type role determines whether a native model cell names a model,
/// an item, a team, or a scalar health value. Actual render models stay in the
/// enclosing appearance's shared ModelRef array.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EntityModelData {
    #[default]
    Models,
    Item {
        index: i32,
        secondary: i32,
    },
    Team {
        team: i32,
        health: i32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityParameter {
    Scalar(u32),
    Sound(Option<SoundRef>),
    Entity(NativeEntityRef),
}
impl Default for EntityParameter {
    fn default() -> Self {
        Self::Scalar(0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityClientData {
    Scalar(i32),
    Client(NativeClientRef),
}
impl Default for EntityClientData {
    fn default() -> Self {
        Self::Scalar(0)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EntityLight {
    pub color: [u8; 3],
    pub intensity: u8,
}
impl EntityLight {
    pub const fn from_packed(value: u32) -> Self {
        let bytes = value.to_le_bytes();
        Self {
            color: [bytes[0], bytes[1], bytes[2]],
            intensity: bytes[3],
        }
    }
    pub const fn packed(self) -> u32 {
        u32::from_le_bytes([self.color[0], self.color[1], self.color[2], self.intensity])
    }
}

/// Named public presentation capabilities. This is not a retained native row;
/// each field is consumed by the shared projection or presentation services.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EntityPresentation {
    pub namespace: ResourceNamespace,
    pub trajectories: EntityTrajectories,
    pub pose: EntityPose,
    pub secondary_pose: EntityPose,
    pub times: [ThinkTime; 2],
    pub animation: EntityAnimation,
    pub event_parameter: EntityParameter,
    pub ground: Option<NativeEntityRef>,
    pub other_entity2: Option<NativeEntityRef>,
    pub client: EntityClientData,
    pub model_data: EntityModelData,
    pub weapon: u32,
    pub powerups: u32,
    pub generic: u32,
    pub light: EntityLight,
}
impl EntityPresentation {
    pub fn new(namespace: ResourceNamespace) -> Self {
        Self {
            namespace,
            trajectories: EntityTrajectories::default(),
            pose: EntityPose::default(),
            secondary_pose: EntityPose::default(),
            times: [ThinkTime::Milliseconds(0); 2],
            animation: EntityAnimation::default(),
            event_parameter: EntityParameter::default(),
            ground: None,
            other_entity2: None,
            client: EntityClientData::default(),
            model_data: EntityModelData::default(),
            weapon: 0,
            powerups: 0,
            generic: 0,
            light: EntityLight::default(),
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
    pub entity_type: u32,
    pub other_entity: Option<NativeEntityRef>,
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
    pub snapshot_excluded: bool,
    pub models: [Option<ModelRef>; 4],
    pub loop_sound: Option<SoundRef>,
    pub presentation: Option<EntityPresentation>,
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
            snapshot_excluded: false,
            models: [None; 4],
            loop_sound: None,
            presentation: None,
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
    Intermission,
    SinglePlayerIntermission,
}
impl MovementMode {
    pub const fn allows_movement(self) -> bool {
        !matches!(
            self,
            Self::Frozen | Self::Intermission | Self::SinglePlayerIntermission
        )
    }
    pub const fn allows_view_angles(self) -> bool {
        !matches!(self, Self::Intermission | Self::SinglePlayerIntermission)
    }
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

/// A native short-angle offset retains its complete integer word until the
/// selected movement or wire boundary narrows it. Float ABIs retain degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AngleOffset {
    Degrees(f32),
    Units(i32),
}
impl Default for AngleOffset {
    fn default() -> Self {
        Self::Degrees(0.0)
    }
}
impl AngleOffset {
    pub fn degrees(self) -> f32 {
        match self {
            Self::Degrees(value) => value,
            Self::Units(value) => crate::math::short_to_angle(value),
        }
    }
    pub fn units(self, form: crate::math::AngleShortForm) -> i32 {
        match self {
            Self::Degrees(value) => crate::math::angle_to_short(value, form),
            Self::Units(value) => value,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AngleOffsets(pub [AngleOffset; 3]);
impl From<Vec3> for AngleOffsets {
    fn from(value: Vec3) -> Self {
        Self(value.0.map(AngleOffset::Degrees))
    }
}
impl AngleOffsets {
    pub fn degrees(self) -> Vec3 {
        Vec3(self.0.map(AngleOffset::degrees))
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
    /// The active movement policy may wait for spawn-time actions to release.
    pub respawned: bool,
    /// A supported movement policy omits canonical players from default traces.
    pub ignore_player_collision: bool,
    pub timer: MovementTimer,
    pub remaining_ms: u32,
    pub command_time_ms: i32,
    pub delta_angles: AngleOffsets,
    pub previous_position: Vec3,
    pub water_jump_seconds: f32,
    pub teleport_hold_until: f64,
    pub tuning: MovementTuning,
}

/// A base value still receives the player's multiplier. Native effective
/// values use a base when division is possible and retain their value when a
/// zero multiplier has no inverse.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MovementMagnitude {
    Base(f32),
    Effective(f32),
}
impl From<f32> for MovementMagnitude {
    fn from(value: f32) -> Self {
        Self::Base(value)
    }
}
impl MovementMagnitude {
    pub fn native(effective: f32, multiplier: f32) -> Self {
        if multiplier != 0.0 {
            Self::Base(effective / multiplier)
        } else {
            Self::Effective(effective)
        }
    }
    pub fn effective(self, multiplier: f32) -> f32 {
        match self {
            Self::Base(value) => value * multiplier,
            Self::Effective(value) => value,
        }
    }
}

/// Cached cvar/module overrides; absence retains the selected native default.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MovementTuning {
    pub max_speed: Option<MovementMagnitude>,
    pub spectator_max_speed: Option<f32>,
    pub gravity: Option<MovementMagnitude>,
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
    /// A boundary may consume native metadata only in its declared namespace.
    pub fn native_for(
        &self,
        source: RuleSetId,
        namespace: Option<ResourceNamespace>,
    ) -> Option<&NativePlayerTail> {
        self.native
            .as_ref()
            .filter(|tail| tail.source == source && namespace == Some(tail.namespace))
    }

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

/// The selected listening view, supplied by the host or an owning CLIENT module.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Listener {
    pub client: ClientId,
    pub entity: Option<SoundEmitter>,
    pub origin: Vec3,
    pub right: Vec3,
    /// Some native listeners supply their left axis. Negate the completed
    /// dot product, preserving that ABI's original floating-point order.
    pub negate_dot: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SoundBus {
    #[default]
    Spatial,
    /// UI/menu audio remains audible with no connected CLIENT/listener.
    Local,
}

/// Load-selected channel behavior, independent of PCM and client protocol.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SoundChannels {
    pub suppress_ms: u32,
    pub emitter_limit: u8,
    pub listener_limit: u8,
    pub protected_channel: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SoundPlayback {
    pub master: u8,
    pub volume_scale: u16,
    pub clamp_pan: bool,
    /// Native CLIENT channels change spatial gain only on Respatialize.
    pub latched: bool,
    pub channels: Option<SoundChannels>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttenuationPrecision {
    F32,
    F64,
}

/// Distance attenuation order and coefficient rounding, selected at load.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DistanceAttenuation {
    Linear(f32),
    Product {
        ordinary: f64,
        match_value: f32,
        matched: f64,
        precision: AttenuationPrecision,
    },
}
impl DistanceAttenuation {
    /// Attenuated distance and whether distance panning is disabled. Linear
    /// preserves distance-first multiplication; Product rounds the channel's
    /// attenuation coefficient before multiplying distance.
    pub fn apply(self, distance: f32, attenuation: f32) -> (f32, bool) {
        match self {
            Self::Linear(scale) => (distance * attenuation * scale, scale == 0.0),
            Self::Product {
                ordinary,
                match_value,
                matched,
                precision,
            } => {
                let scale = if attenuation == match_value {
                    matched
                } else {
                    ordinary
                };
                let coefficient = match precision {
                    AttenuationPrecision::F32 => attenuation * scale as f32,
                    AttenuationPrecision::F64 => (f64::from(attenuation) * scale) as f32,
                };
                (distance * coefficient, coefficient == 0.0)
            }
        }
    }
}

/// One-shot issue order selected at the sound behavior boundary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum SoundStartOrder {
    #[default]
    ImmediateFifo,
    /// Queue even zero-offset starts and issue equal deadlines newest first.
    QueuedNewestFirst,
}
impl SoundStartOrder {
    pub const fn publication_key(self, order: u64) -> u64 {
        match self {
            Self::ImmediateFifo => order,
            Self::QueuedNewestFirst => u64::MAX - order,
        }
    }
}

/// Sound capability parameters selected at the emitting role's boundary.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpatialPolicy {
    pub distance_offset: f32,
    pub attenuation: DistanceAttenuation,
    pub separation: f32,
    pub start_order: SoundStartOrder,
}
impl Default for SpatialPolicy {
    fn default() -> Self {
        Self {
            distance_offset: 0.0,
            attenuation: DistanceAttenuation::Linear(0.0),
            separation: 1.0,
            start_order: SoundStartOrder::ImmediateFifo,
        }
    }
}

/// Native loop gain and global-volume arithmetic, selected at load.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LoopGain {
    Integer { master: u8, volume_scale: u16 },
    Float(f32),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoopPhase {
    Cursor,
    Painted,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoopLifetime {
    Temporary,
    Persistent,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoopOrigin {
    Accepted,
    Presented,
}
/// One admitted CLIENT loop. Static WAV-cue loops retain the ordinary path.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoopPlayback {
    pub master: LoopGain,
    pub phase: LoopPhase,
    pub lifetime: LoopLifetime,
    pub panning: bool,
    pub clamp_pan: bool,
    pub origin: LoopOrigin,
    /// Native CLIENT imports latch their audible mix only on Respatialize.
    pub latched: bool,
    /// Use the native cue/LIST sample extent rather than the full waveform.
    pub cue_extent: bool,
}

/// Captured rate for one looping emission. Source rules compute it once;
/// the mixer retains the audible value at its ordinary listener boundary.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoopDoppler {
    pub scale: f32,
    pub previous_scale: f32,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SoundEvent {
    pub sound: SoundId,
    /// The source boundary's ordinal, retained without replacing PCM identity.
    pub source_ordinal: Option<u32>,
    /// Exact quantized fields captured at a native wire boundary.
    pub native_fields: Option<NativeSoundFields>,
    /// Spatial behavior belongs to this emission, independently of shared PCM.
    pub spatial: SpatialPolicy,
    pub looping: Option<LoopPlayback>,
    pub playback: Option<SoundPlayback>,
    pub bus: SoundBus,
    pub entity: Option<SoundEmitter>,
    pub channel: i32,
    /// Captured queued event time, never a module-clock or raw-clock sample.
    pub queued_at: crate::sys_events::EventTime,
    pub position: Vec3,
    pub velocity: Vec3,
    pub doppler: Option<LoopDoppler>,
    pub volume: f32,
    pub attenuation: f32,
    pub action: SoundAction,
    pub client: Option<ClientId>,
    /// Canonical receiving CLIENT connection; SERVER emissions have no value.
    pub client_lifetime: Option<EntityId>,
    pub time_offset: f32,
    /// The emission needs its supplied position rather than an entity origin.
    pub positioned: bool,
    pub listener: Option<Listener>,
    pub listener_owner: Option<ModuleId>,
}
impl SoundEvent {
    /// Presentation-only retirement; native protocols never encode this control.
    pub fn retire_client(client: ClientId, lifetime: EntityId) -> Self {
        Self {
            client: Some(client),
            client_lifetime: Some(lifetime),
            action: SoundAction::RetireClient,
            ..Self::default()
        }
    }
    pub fn reset_client_loops(client: ClientId, lifetime: EntityId) -> Self {
        Self {
            client: Some(client),
            client_lifetime: Some(lifetime),
            action: SoundAction::ResetClientLoops,
            ..Self::default()
        }
    }
    pub fn applies_to_client(self, client: ClientId) -> bool {
        self.client.is_none_or(|recipient| recipient == client)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeSoundFields {
    pub flags: u8,
    pub volume: u8,
    pub attenuation: u8,
    pub time_offset: u8,
    /// Original native slot, independent of the captured common lifetime.
    pub entity: u16,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SoundAction {
    #[default]
    Play,
    StartLoop,
    Stop,
    /// Remove admitted loops for one emitter, preserving channel-zero one-shots.
    StopLoop,
    /// Remove admitted loops for a CLIENT lifetime, preserving static cues.
    ResetClientLoops,
    /// Native Clear(false): remove temporary registrations and mute latched
    /// output immediately. Persistent registrations resume at Respatialize.
    ClearTemporaryLoops,
    /// Native Clear(true), distinct from connection/ownership retirement.
    ClearClientLoops,
    UpdateEmitterPosition,
    Respatialize,
    ReleaseListener,
    /// Remove only voices received during one canonical CLIENT connection.
    RetireClient,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EffectEvent {
    pub effect: EffectId,
    pub position: Vec3,
    pub direction: Vec3,
    pub count: u16,
    pub color: u8,
    pub time: crate::sys_events::EventTime,
    pub client: Option<ClientId>,
    /// Canonical receiving CLIENT connection; SERVER emissions have no value.
    pub client_lifetime: Option<EntityId>,
}
impl EffectEvent {
    pub fn applies_to_client(self, client: ClientId) -> bool {
        self.client.is_none_or(|recipient| recipient == client)
    }
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
    pub text: PrintText,
}
/// Native print bytes and their semantic view share the ring's leased pages.
#[derive(Clone, Copy, Debug)]
pub enum PrintText {
    Text(TextId),
    Native {
        wire: NativePayload,
        text: IndexSpan,
        /// Parsed native-service extent; an undecodable suffix remains opaque.
        consumed: u32,
    },
}
impl PrintText {
    pub fn display(self) -> Option<TextId> {
        match self {
            Self::Text(id) => Some(id),
            Self::Native { .. } => None,
        }
    }
}
impl From<TextId> for PrintText {
    fn from(id: TextId) -> Self {
        Self::Text(id)
    }
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
    /// Stable source-service offset distinguishes siblings of one emission.
    pub component: u32,
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
pub const NATIVE_CUE_CAPTURE_BYTES: usize = 12;
/// Muzzle flashes are the remaining native cues with captured entity lifetimes.
pub const NATIVE_CUE_MIN_WIRE_BYTES: usize = 4;
pub const NATIVE_CUE_CAPACITY: usize = NATIVE_WIRE_BYTES / NATIVE_CUE_MIN_WIRE_BYTES;
pub const NATIVE_PAYLOAD_BYTES: usize =
    NATIVE_WIRE_BYTES + NATIVE_CUE_CAPACITY * NATIVE_CUE_CAPTURE_BYTES;
pub const OUTPUT_PAGE_BYTES: usize = 8192;
pub const NATIVE_WIRE_PAGES: usize = NATIVE_WIRE_BYTES / OUTPUT_PAGE_BYTES;
pub const NATIVE_PAYLOAD_PAGES: usize = NATIVE_PAYLOAD_BYTES.div_ceil(OUTPUT_PAGE_BYTES);

#[derive(Clone, Copy, Debug)]
pub struct NativeCueBinding {
    pub entity: Option<EntityId>,
}
impl NativeCueBinding {
    pub fn offset(row: &[u8]) -> Option<u32> {
        Some(u32::from_le_bytes(row.get(..4)?.try_into().ok()?))
    }
    pub fn write_capture(self, offset: u32, output: &mut [u8]) -> Option<()> {
        let row = output.get_mut(..NATIVE_CUE_CAPTURE_BYTES)?;
        row[..4].copy_from_slice(&offset.to_le_bytes());
        row[4..8].copy_from_slice(&self.entity.map_or(u32::MAX, |id| id.slot).to_le_bytes());
        row[8..12].copy_from_slice(&self.entity.map_or(0, |id| id.generation).to_le_bytes());
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
    /// Sorted (native-service byte offset, captured entity lifetime) entries live
    /// in the same leased pages as the original bytes, never module storage.
    pub fn cue(self, stored: &[u8], offset: usize) -> Option<NativeCueBinding> {
        let rows = stored.get(self.length..self.stored_length)?;
        let mut low = 0;
        let mut high = rows.len() / NATIVE_CUE_CAPTURE_BYTES;
        while low < high {
            let index = low + (high - low) / 2;
            let first = index * NATIVE_CUE_CAPTURE_BYTES;
            let row = rows.get(first..first + NATIVE_CUE_CAPTURE_BYTES)?;
            let at = NativeCueBinding::offset(row)? as usize;
            match at.cmp(&offset) {
                std::cmp::Ordering::Less => low = index + 1,
                std::cmp::Ordering::Greater => high = index,
                std::cmp::Ordering::Equal => {
                    let slot = u32::from_le_bytes(row[4..8].try_into().ok()?);
                    let generation = u32::from_le_bytes(row[8..12].try_into().ok()?);
                    return Some(NativeCueBinding {
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
    pub view_feedback: Option<ViewFeedback>,
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
        self.clear_notify(texts);
    }

    pub fn clear_notify(&mut self, texts: &mut crate::events::TextStore) {
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
/// Native arrays and extra packet fields reserve rows in the same value bank.
/// `items` is a clientdata field, independent of the native stat ordinals.
#[derive(Clone, Copy, Debug)]
pub struct NativeValueLayout {
    pub stats: u32,
    pub persistent: u32,
    pub ammo: u32,
    pub powerups: u32,
    pub width: ValueWidth,
    pub items: bool,
}
impl NativeValueLayout {
    pub const fn arrays(self) -> [u32; 4] {
        [self.stats, self.persistent, self.ammo, self.powerups]
    }
    pub fn values(self) -> u32 {
        self.arrays().into_iter().sum::<u32>() + u32::from(self.items)
    }
}
/// Stat ordinals supplied by NQ clientdata, independently of separate updates.
pub const NQ_PACKET_STATS: [usize; 10] = [0, 2, 3, 4, 5, 6, 7, 8, 9, 10];
/// Native stat/persistent counts and module value widths, in RuleSetId order.
pub const NATIVE_VALUE_LAYOUT: [NativeValueLayout; 5] = [
    NativeValueLayout {
        stats: 32,
        persistent: 0,
        ammo: 0,
        powerups: 0,
        width: ValueWidth::Signed32,
        items: true,
    },
    NativeValueLayout {
        stats: 32,
        persistent: 0,
        ammo: 0,
        powerups: 0,
        width: ValueWidth::Signed32,
        items: false,
    },
    NativeValueLayout {
        stats: 32,
        persistent: 0,
        ammo: 0,
        powerups: 0,
        width: ValueWidth::Signed16,
        items: false,
    },
    NativeValueLayout {
        stats: 64,
        persistent: 0,
        ammo: 0,
        powerups: 0,
        width: ValueWidth::Signed16,
        items: false,
    },
    NativeValueLayout {
        stats: 16,
        persistent: 16,
        ammo: 16,
        powerups: 16,
        width: ValueWidth::Signed32,
        items: false,
    },
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

/// Borrowed native array bindings in stats, persistent, ammo, powerup order.
/// These describe rows in the one value bank and own no additional values.
#[derive(Clone, Copy, Debug)]
pub struct PlayerValueBindings<'a> {
    pub arrays: [&'a [ValueBinding]; 4],
    pub items: Option<ValueBinding>,
}
impl PlayerValueBindings<'_> {
    pub const EMPTY: Self = Self {
        arrays: [&[]; 4],
        items: None,
    };
    pub fn iter(&self) -> impl Iterator<Item = ValueBinding> + '_ {
        self.arrays
            .iter()
            .flat_map(|array| array.iter().copied())
            .chain(self.items)
    }
    pub fn fits(&self, bank: &ValueBank) -> bool {
        self.iter().all(|binding| bank.get(binding.id).is_some())
    }
}

/// Public Q3 stat ordinals are module/protocol admission data. Team Arena's
/// extra stat shifts weapons and armor; movement and presentation do not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Q3StatLayout {
    pub health: u8,
    pub weapons: u8,
    pub armor: u8,
}
impl Q3StatLayout {
    pub const BASE: Self = Self {
        health: 0,
        weapons: 2,
        armor: 3,
    };
    pub const TEAM_ARENA: Self = Self {
        health: 0,
        weapons: 3,
        armor: 4,
    };
    pub fn fits(self, stats: usize) -> bool {
        [self.health, self.weapons, self.armor]
            .into_iter()
            .all(|index| usize::from(index) < stats)
    }
}
impl Default for Q3StatLayout {
    fn default() -> Self {
        Self::BASE
    }
}
