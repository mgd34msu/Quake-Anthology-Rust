pub mod boxes;
pub mod brushes;
pub mod contents;
pub mod hulls;
pub mod scene;

use brushes::BrushMap;
pub use contents::Contents;
use qa_core::primitives::{EntityId, Plane, SurfaceFlags, Vec3};
pub use scene::WorldTrace;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuakeTraceKind {
    Normal,
    IgnoreBoxes,
    Missile,
}

/// Native filtering, temporary-body kernel and result merging belong to the caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityTraceRules {
    Quake { kind: QuakeTraceKind },
    Quake2,
    Quake3,
}

impl EntityTraceRules {
    pub const QUAKE: Self = Self::Quake {
        kind: QuakeTraceKind::Normal,
    };
    pub const QUAKE2: Self = Self::Quake2;
    pub const ARENA: Self = Self::Quake3;
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

/// Caller-owned clipping choices. Geometry never selects a game or movement.
/// These are engine query values, not fields of any legacy wire protocol.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TraceRules {
    /// Native epsilon macros are unsuffixed doubles. Only the biased fraction
    /// expression is promoted; plane distances and stored fractions stay f32.
    pub contact_epsilon: f64,
    pub outside_brush: OutsideBrush,
    pub fraction_clamp: FractionClamp,
    pub bounds_origin: BoundsOrigin,
    pub all_solid: AllSolid,
}

impl TraceRules {
    /// Q1/QW hull contact and Q2 convex clipping use the same contact epsilon.
    /// Convex brushes are a shared extension for Q1 callers, not Q1 topology.
    pub const LEGACY: Self = Self {
        contact_epsilon: 0.03125,
        outside_brush: OutsideBrush::MovingAway,
        fraction_clamp: FractionClamp::AfterSelection,
        bounds_origin: BoundsOrigin::Supplied,
        all_solid: AllSolid::PreserveFraction,
    };
    pub const ARENA: Self = Self {
        contact_epsilon: 0.125,
        outside_brush: OutsideBrush::EndBeyondEpsilon,
        fraction_clamp: FractionClamp::PerPlane,
        bounds_origin: BoundsOrigin::Centered,
        all_solid: AllSolid::BlockAtStart,
    };
}

/// A point or axis-aligned box sweep over the scene. Exclusions are borrowed;
/// caller rules are independent of geometry, module format and wire protocol.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TraceQuery<'a> {
    pub start: Vec3,
    pub end: Vec3,
    pub mins: Vec3,
    pub maxs: Vec3,
    pub mask: Contents,
    pub rules: TraceRules,
    pub entity_rules: EntityTraceRules,
    pub pass: Option<EntityId>,
    pub excluded: &'a [EntityId],
}

impl TraceQuery<'_> {
    pub fn point(
        start: Vec3,
        end: Vec3,
        rules: TraceRules,
        entity_rules: EntityTraceRules,
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
    /// Brush-solid contacts preserve NetQuake's SOLID_BSP grounding rule.
    pub brush_solid: bool,
}

impl Trace {
    /// qsrc Q1/Q2 replace on solid flags or nearer; Q3 replaces only nearer.
    /// Equal-fraction Q3 solid flags affect the retained result, not its owner.
    pub fn merge_linked(&mut self, incoming: Self, rules: EntityTraceRules) {
        match rules {
            EntityTraceRules::Quake { .. } | EntityTraceRules::Quake2 => {
                if incoming.all_solid || incoming.start_solid || incoming.fraction < self.fraction {
                    let start_solid = self.start_solid;
                    *self = incoming;
                    self.start_solid |= start_solid;
                }
            }
            EntityTraceRules::Quake3 => {
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
            plane: Plane::default(),
            start_solid: false,
            all_solid: false,
            in_open: false,
            in_water: false,
            contents: Contents::EMPTY,
            entity: None,
            surface: SurfaceFlags::default(),
            brush_solid: true,
        }
    }
}

pub enum CollisionWorld {
    Hulls(hulls::Q1Hulls),
    Brushes(BrushMap),
}

/// Cold-sized caller storage. Each concurrent trace caller owns its scratch.
pub struct TraceScratch {
    hull: Option<hulls::HullScratch>,
}

impl CollisionWorld {
    pub fn scratch(&self) -> TraceScratch {
        TraceScratch {
            hull: match self {
                Self::Hulls(map) => Some(map.scratch()),
                Self::Brushes(_) => None,
            },
        }
    }

    pub(crate) fn trace_geometry(&self, query: TraceQuery, scratch: &mut TraceScratch) -> Trace {
        match self {
            Self::Hulls(map) => match &mut scratch.hull {
                Some(hull) => map.trace(query, hull),
                None => Trace::clear(query.end),
            },
            Self::Brushes(map) => map.trace(query),
        }
    }

    pub(crate) fn point_contents(&self, point: Vec3) -> Contents {
        match self {
            Self::Hulls(map) => map.point_contents(point),
            Self::Brushes(map) => map.point_contents(point),
        }
    }
}
