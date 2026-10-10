pub mod boxes;
pub mod brushes;
pub mod contents;
pub mod hulls;
pub mod scene;
pub mod store;
pub mod surfaces;
pub mod tree;

pub use contents::Contents;
use qa_core::primitives::{EntityId, Plane, RuleSetId, SurfaceFlags, SurfaceId, Vec3};
pub use scene::WorldTrace;
pub use store::{CollisionStore, StoreError, TraceScratch};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuakeTraceKind {
    Normal,
    IgnoreBoxes,
    Missile,
}

/// Caller-owned entity behaviour, resolved once alongside clipping policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntityTracePolicy {
    pub world_entity: WorldEntityRule,
    pub link_role: crate::area::LinkFlags,
    pub filtering: EntityTraceFlags,
}

impl EntityTracePolicy {
    pub const fn stop_at_zero(self) -> bool {
        self.filtering.contains(EntityTraceFlags::STOP_AT_ZERO)
    }
    pub const fn quake_kind(self) -> Option<QuakeTraceKind> {
        match (self.filtering.0 & EntityTraceFlags::KIND_MASK) >> EntityTraceFlags::KIND_SHIFT {
            1 => Some(QuakeTraceKind::Normal),
            2 => Some(QuakeTraceKind::IgnoreBoxes),
            3 => Some(QuakeTraceKind::Missile),
            _ => None,
        }
    }
    pub const fn with_quake_kind(mut self, kind: Option<QuakeTraceKind>) -> Self {
        let value = match kind {
            Some(kind) => kind as u16 + 1,
            None => 0,
        };
        self.filtering.0 = (self.filtering.0 & !EntityTraceFlags::KIND_MASK)
            | value << EntityTraceFlags::KIND_SHIFT;
        self
    }
    pub const fn contents_leaf_gate(self) -> LeafGate {
        if self.filtering.contains(EntityTraceFlags::BRUSH_CONTENTS) {
            LeafGate::Brushes
        } else {
            LeafGate::StoredContents
        }
    }
    pub const fn merge(self) -> LinkedMerge {
        if self.filtering.contains(EntityTraceFlags::MERGE_NEARER) {
            LinkedMerge::Nearer
        } else {
            LinkedMerge::SolidOrNearer
        }
    }
}

/// Independent query behaviours packed into the hot policy word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntityTraceFlags(pub u16);
impl EntityTraceFlags {
    pub const SKIP_POINT_ENTITIES: u16 = 1;
    pub const DEAD_MONSTER_MASK: u16 = 2;
    pub const CONTENTS_MASK: u16 = 4;
    pub const SHARED_OWNER: u16 = 8;
    pub const CENTER_BOX: u16 = 16;
    pub const LINKED_CONTENTS: u16 = 32;
    pub const EXCLUDE_CONTENTS_PASS: u16 = 64;
    pub const INCLUSIVE_CONTENTS_MAX: u16 = 128;
    pub const STOP_AT_ZERO: u16 = 256;
    const KIND_SHIFT: u32 = 9;
    const KIND_MASK: u16 = 3 << Self::KIND_SHIFT;
    pub const HULL_BOX: u16 = 1 << Self::KIND_SHIFT;
    pub const BRUSH_CONTENTS: u16 = 1 << 11;
    pub const MERGE_NEARER: u16 = 1 << 12;

    pub const fn contains(self, behaviour: u16) -> bool {
        self.0 & behaviour != 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorldEntityRule {
    HitOrStartSolid,
    Always,
    FractionChanged,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkedMerge {
    SolidOrNearer,
    Nearer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutsideBrush {
    /// Q2 keeps approaching planes even when both endpoints are outside.
    MovingAway,
    /// Q3 also rejects endpoints at or beyond the contact epsilon.
    EndBeyondEpsilon,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FractionClamp {
    AfterSelection,
    PerPlane,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundsOrigin {
    Supplied,
    Centered,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AllSolid {
    PreserveFraction,
    BlockAtStart,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NonAxialOffset {
    ProjectedExtents,
    ProjectedExtentsBinary32,
    Fixed(f32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeafGate {
    StoredContents,
    Brushes,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PositionRules {
    AllSides,
    AxialPrefix,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PositionEndpoint {
    Start,
    ByFraction,
}

/// Caller-owned clipping choices. Geometry never selects a game or movement.
/// These are engine query values, not fields of any legacy wire protocol.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TraceRules {
    /// Plane distances and stored fractions stay f32; the native epsilon
    /// expression's precision is selected independently below.
    pub contact_epsilon: f64,
    pub fraction_binary32: bool,
    pub outside_brush: OutsideBrush,
    pub fraction_clamp: FractionClamp,
    pub bounds_origin: BoundsOrigin,
    pub all_solid: AllSolid,
    pub tree_margin: f32,
    pub nonaxial_offset: NonAxialOffset,
    pub leaf_gate: LeafGate,
    pub position: PositionRules,
    pub position_endpoint: PositionEndpoint,
    /// Original stationary collection truncation is caller compatibility data.
    pub position_leaf_limit: u32,
    /// Rerelease keeps the native second entering plane, in source space.
    pub secondary_contact: bool,
}

impl TraceRules {
    /// Q1/QW hull contact and Q2 convex clipping use the same contact epsilon.
    /// Convex brushes are a shared extension for Q1 callers, not Q1 topology.
    pub const LEGACY: Self = Self {
        contact_epsilon: 0.03125,
        fraction_binary32: false,
        outside_brush: OutsideBrush::MovingAway,
        fraction_clamp: FractionClamp::AfterSelection,
        bounds_origin: BoundsOrigin::Supplied,
        all_solid: AllSolid::PreserveFraction,
        tree_margin: 0.0,
        nonaxial_offset: NonAxialOffset::ProjectedExtents,
        leaf_gate: LeafGate::StoredContents,
        position: PositionRules::AllSides,
        position_endpoint: PositionEndpoint::Start,
        position_leaf_limit: 1024,
        secondary_contact: false,
    };
    pub const ARENA: Self = Self {
        contact_epsilon: 0.125,
        fraction_binary32: false,
        outside_brush: OutsideBrush::EndBeyondEpsilon,
        fraction_clamp: FractionClamp::PerPlane,
        bounds_origin: BoundsOrigin::Centered,
        all_solid: AllSolid::BlockAtStart,
        tree_margin: 1.0,
        nonaxial_offset: NonAxialOffset::Fixed(2048.0),
        leaf_gate: LeafGate::Brushes,
        position: PositionRules::AxialPrefix,
        position_endpoint: PositionEndpoint::ByFraction,
        position_leaf_limit: 1024,
        secondary_contact: false,
    };
    pub const RERELEASE: Self = Self {
        fraction_binary32: true,
        outside_brush: OutsideBrush::EndBeyondEpsilon,
        fraction_clamp: FractionClamp::PerPlane,
        all_solid: AllSolid::BlockAtStart,
        nonaxial_offset: NonAxialOffset::ProjectedExtentsBinary32,
        leaf_gate: LeafGate::Brushes,
        secondary_contact: true,
        ..Self::LEGACY
    };

    pub(crate) fn contact_fraction(self, first: f32, last: f32, entering: bool) -> f32 {
        if self.fraction_binary32 {
            let epsilon = self.contact_epsilon as f32;
            let numerator = if entering {
                first - epsilon
            } else {
                first + epsilon
            };
            numerator / (first - last)
        } else {
            let numerator = if entering {
                f64::from(first) - self.contact_epsilon
            } else {
                f64::from(first) + self.contact_epsilon
            };
            (numerator / f64::from(first - last)) as f32
        }
    }

    pub(crate) fn split_fraction(self, distance: f32, inverse: f32, add: bool) -> f32 {
        if self.fraction_binary32 {
            let epsilon = self.contact_epsilon as f32;
            let biased = if add {
                distance + epsilon
            } else {
                distance - epsilon
            };
            biased * inverse
        } else {
            let biased = if add {
                f64::from(distance) + self.contact_epsilon
            } else {
                f64::from(distance) - self.contact_epsilon
            };
            (biased * f64::from(inverse)) as f32
        }
    }

    pub(crate) fn inverse_span(self, span: f32) -> f32 {
        if self.fraction_binary32 {
            1.0f32 / span
        } else {
            (1.0f64 / f64::from(span)) as f32
        }
    }
}

/// Resolve the caller's explicit role into the shared query values. Native
/// presets are data; neither geometry nor movement selects this role here.
pub const fn trace_policy(rules: RuleSetId) -> (TraceRules, EntityTracePolicy) {
    use crate::area::LinkFlags;
    match rules {
        RuleSetId::Quake | RuleSetId::QuakeWorld => (
            TraceRules::LEGACY,
            EntityTracePolicy {
                world_entity: WorldEntityRule::HitOrStartSolid,
                link_role: LinkFlags::SOLID,
                filtering: EntityTraceFlags(
                    EntityTraceFlags::SKIP_POINT_ENTITIES
                        | EntityTraceFlags::HULL_BOX
                        | EntityTraceFlags::INCLUSIVE_CONTENTS_MAX,
                ),
            },
        ),
        RuleSetId::Quake2 | RuleSetId::Quake2Rerelease => (
            if matches!(rules, RuleSetId::Quake2Rerelease) {
                TraceRules::RERELEASE
            } else {
                TraceRules::LEGACY
            },
            EntityTracePolicy {
                world_entity: WorldEntityRule::Always,
                link_role: LinkFlags::SOLID,
                filtering: EntityTraceFlags(
                    EntityTraceFlags::DEAD_MONSTER_MASK
                        | EntityTraceFlags::LINKED_CONTENTS
                        | EntityTraceFlags::STOP_AT_ZERO
                        | if matches!(rules, RuleSetId::Quake2Rerelease) {
                            EntityTraceFlags::BRUSH_CONTENTS
                        } else {
                            0
                        },
                ),
            },
        ),
        RuleSetId::Quake3 => (
            TraceRules::ARENA,
            EntityTracePolicy {
                world_entity: WorldEntityRule::FractionChanged,
                link_role: LinkFlags::LINKED,
                filtering: EntityTraceFlags(
                    EntityTraceFlags::CONTENTS_MASK
                        | EntityTraceFlags::STOP_AT_ZERO
                        | EntityTraceFlags::BRUSH_CONTENTS
                        | EntityTraceFlags::MERGE_NEARER
                        | EntityTraceFlags::SHARED_OWNER
                        | EntityTraceFlags::CENTER_BOX
                        | EntityTraceFlags::LINKED_CONTENTS
                        | EntityTraceFlags::EXCLUDE_CONTENTS_PASS
                        | EntityTraceFlags::INCLUSIVE_CONTENTS_MAX,
                ),
            },
        ),
    }
}

/// A point or axis-aligned box sweep over the scene. Exclusions are borrowed;
/// caller rules are independent of geometry, module format and wire protocol.
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct TraceQuery<'a> {
    pub rules: TraceRules,
    pub excluded: &'a [EntityId],
    pub mask: Contents,
    pub pass: Option<EntityId>,
    // Keep the float block's measured offsets; small policy data occupies its tail.
    pub start: Vec3,
    pub end: Vec3,
    pub mins: Vec3,
    pub maxs: Vec3,
    pub entity_rules: EntityTracePolicy,
}

impl TraceQuery<'_> {
    pub fn point(
        start: Vec3,
        end: Vec3,
        rules: TraceRules,
        entity_rules: EntityTracePolicy,
    ) -> Self {
        Self {
            start,
            end,
            mins: Vec3::default(),
            maxs: Vec3::default(),
            mask: Contents::SOLID,
            rules,
            entity_rules,
            pass: None,
            excluded: &[],
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Trace {
    pub fraction: f32,
    pub end: Vec3,
    pub plane: Plane,
    pub start_solid: bool,
    pub all_solid: bool,
    pub in_open: bool,
    pub in_water: bool,
    pub contents: Contents,
    pub entity: Option<EntityId>,
    pub surface: SurfaceFlags,
    pub surface_id: Option<SurfaceId>,
    pub secondary_plane: Option<Plane>,
    pub secondary_surface_id: Option<SurfaceId>,
    /// Brush-solid contacts preserve NetQuake's SOLID_BSP grounding rule.
    pub brush_solid: bool,
}

impl Trace {
    /// qsrc Q1/Q2 replace on solid flags or nearer; Q3 replaces only nearer.
    /// Equal-fraction Q3 solid flags affect the retained result, not its owner.
    pub fn merge_linked(&mut self, incoming: Self, rules: EntityTracePolicy) {
        match rules.merge() {
            LinkedMerge::SolidOrNearer => {
                if incoming.all_solid || incoming.start_solid || incoming.fraction < self.fraction {
                    let start_solid = self.start_solid;
                    *self = incoming;
                    self.start_solid |= start_solid;
                }
            }
            LinkedMerge::Nearer => {
                if incoming.all_solid {
                    self.all_solid = true;
                } else if incoming.start_solid {
                    self.start_solid = true;
                }
                if incoming.fraction < self.fraction {
                    let start_solid = self.start_solid;
                    *self = incoming;
                    self.start_solid |= start_solid;
                }
            }
        }
    }
    pub fn clear(end: Vec3) -> Self {
        Self {
            fraction: 1.0,
            end,
            plane: Plane {
                encoding: Some([0, 0]),
                ..Plane::default()
            },
            start_solid: false,
            all_solid: false,
            in_open: false,
            in_water: false,
            contents: Contents::EMPTY,
            entity: None,
            surface: SurfaceFlags::default(),
            surface_id: None,
            secondary_plane: None,
            secondary_surface_id: None,
            brush_solid: true,
        }
    }
}
