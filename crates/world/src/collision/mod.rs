pub mod brushes;
pub mod contents;
pub mod hulls;

use brushes::{BrushMap, BrushRules};
pub use contents::Contents;
use qa_core::primitives::{EntityId, Plane, Vec3};

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
}

impl Trace {
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
        }
    }
}

pub enum CollisionWorld {
    Q1Hulls(hulls::Q1Hulls),
    Q2Brushes(BrushMap),
    Q3Brushes(BrushMap),
}

impl CollisionWorld {
    pub fn trace(
        &mut self,
        start: Vec3,
        end: Vec3,
        mins: Vec3,
        maxs: Vec3,
        mask: Contents,
    ) -> Trace {
        match self {
            Self::Q1Hulls(map) => map.trace(start, end, mins, maxs, mask),
            Self::Q2Brushes(map) => map.trace(start, end, mins, maxs, mask, BrushRules::Classic),
            Self::Q3Brushes(map) => map.trace(start, end, mins, maxs, mask, BrushRules::Arena),
        }
    }

    pub fn point_contents(&self, point: Vec3) -> Contents {
        match self {
            Self::Q1Hulls(map) => map.point_contents(point),
            Self::Q2Brushes(map) | Self::Q3Brushes(map) => map.point_contents(point),
        }
    }
}
