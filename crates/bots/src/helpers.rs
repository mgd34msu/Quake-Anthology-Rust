//! Shared navigation helpers ported from `src/bots/navigation/helpers.ts`:
//! checked record access, distances, profile selection, medium contents,
//! traces, and profile validation.

use qa_core::math::{Bounds, Vec3};

use crate::error::{indexed, BotsError};
use crate::scene::{PointContentsQuery, PointContentsResult, QueryTarget, TraceQuery, TraceResult};
use crate::types::{NavigationNode, NavigationProfile, NavigationSource, NavigationWorld};

/// Borrow a navigation record by index.
pub fn at<T>(values: &[T], index: i32) -> Result<&T, BotsError> {
    indexed(values, i64::from(index), "Navigation record")
}

/// Euclidean distance between two points.
#[must_use]
pub fn distance(a: Vec3, b: Vec3) -> f64 {
    let dx = f64::from(a.x) - f64::from(b.x);
    let dy = f64::from(a.y) - f64::from(b.y);
    let dz = f64::from(a.z) - f64::from(b.z);
    dx.hypot(dy).hypot(dz)
}

/// Midpoint of two points. The donor averages in binary64; narrow the
/// stored coordinates the same way.
#[must_use]
pub fn midpoint(a: Vec3, b: Vec3) -> Vec3 {
    Vec3 {
        x: ((f64::from(a.x) + f64::from(b.x)) / 2.0) as f32,
        y: ((f64::from(a.y) + f64::from(b.y)) / 2.0) as f32,
        z: ((f64::from(a.z) + f64::from(b.z)) / 2.0) as f32,
    }
}

/// Translate bounds to a point.
#[must_use]
pub fn translated(point: Vec3, bounds: Bounds) -> Bounds {
    Bounds {
        min: Vec3 {
            x: (f64::from(point.x) + f64::from(bounds.min.x)) as f32,
            y: (f64::from(point.y) + f64::from(bounds.min.y)) as f32,
            z: (f64::from(point.z) + f64::from(bounds.min.z)) as f32,
        },
        max: Vec3 {
            x: (f64::from(point.x) + f64::from(bounds.max.x)) as f32,
            y: (f64::from(point.y) + f64::from(bounds.max.y)) as f32,
            z: (f64::from(point.z) + f64::from(bounds.max.z)) as f32,
        },
    }
}

/// Medium content bits reported by [`contents`].
pub struct NavigationContents;

impl NavigationContents {
    /// Water.
    pub const WATER: i32 = 1;
    /// Slime.
    pub const SLIME: i32 = 2;
    /// Lava.
    pub const LAVA: i32 = 4;
    /// Ladder.
    pub const LADDER: i32 = 8;
}

/// Crouched profile, when the profile carries a crouched body and the
/// crouch capability.
#[must_use]
pub fn crouched_profile(profile: &NavigationProfile) -> Option<NavigationProfile> {
    use crate::types::TravelMode;
    let shape = profile.crouched_shape?;
    if !profile.capabilities.contains(&TravelMode::Crouch) {
        return None;
    }
    let mut crouched = profile.clone();
    crouched.shape = shape;
    Some(crouched)
}

/// Profile for a node: crouch-only sources select the crouched body.
#[must_use]
pub fn node_profile(profile: &NavigationProfile, node: &NavigationNode) -> Option<NavigationProfile> {
    use crate::types::KexGeneration;
    let crouched = match node.source {
        NavigationSource::Aas { .. } => (node.presence & 2) == 0 && (node.presence & 4) != 0,
        NavigationSource::Kex { generation, .. } => generation == KexGeneration::Nav3 && (node.flags & 512) != 0,
        NavigationSource::Constructed { .. } => node.presence == 4,
    };
    if crouched {
        crouched_profile(profile)
    } else {
        Some(profile.clone())
    }
}

/// Medium contents at a point, folded to [`NavigationContents`] bits.
pub fn contents(world: &dyn NavigationWorld, profile: &NavigationProfile, point: Vec3, target: &QueryTarget) -> i32 {
    let sample = world.scene().point_contents(&PointContentsQuery {
        point,
        target: *target,
        policy: profile.policy,
        numeric: profile.movement.numeric,
        pass_actor: world.pass_actor(),
    });
    match sample {
        PointContentsResult::Q1 { contents } => match contents {
            -3 => NavigationContents::WATER,
            -4 => NavigationContents::SLIME,
            -5 => NavigationContents::LAVA,
            _ => 0,
        },
        PointContentsResult::Q2 { merged, .. } => {
            (if merged & 32 != 0 { NavigationContents::WATER } else { 0 })
                | (if merged & 16 != 0 { NavigationContents::SLIME } else { 0 })
                | (if merged & 8 != 0 { NavigationContents::LAVA } else { 0 })
                | (if merged & 0x2000_0000 != 0 {
                    NavigationContents::LADDER
                } else {
                    0
                })
        }
        PointContentsResult::Q3 { contents } => {
            (if contents & 32 != 0 {
                NavigationContents::WATER
            } else {
                0
            }) | (if contents & 16 != 0 {
                NavigationContents::SLIME
            } else {
                0
            }) | (if contents & 8 != 0 { NavigationContents::LAVA } else { 0 })
        }
    }
}

/// Trace the profile body between two points.
pub fn trace(
    world: &dyn NavigationWorld,
    profile: &NavigationProfile,
    start: Vec3,
    end: Vec3,
    target: &QueryTarget,
) -> TraceResult {
    world.scene().trace(&TraceQuery {
        start,
        end,
        shape: profile.shape.into(),
        target: *target,
        policy: profile.policy,
        numeric: profile.movement.numeric,
        pass_actor: world.pass_actor(),
    })
}

/// Whether the profile body sweeps between two points without contact.
pub fn clear(
    world: &dyn NavigationWorld,
    profile: &NavigationProfile,
    start: Vec3,
    end: Vec3,
    target: &QueryTarget,
) -> bool {
    let result = trace(world, profile, start, end, target);
    !result.start_solid && !result.all_solid && result.fraction == 1.0
}

/// Validate profile scalars, body envelopes, and policy family.
pub fn validate_profile(profile: &NavigationProfile) -> Result<(), BotsError> {
    let mut shapes = vec![profile.shape];
    if let Some(shape) = profile.crouched_shape {
        shapes.push(shape);
    }
    for shape in shapes {
        let bounds = shape.bounds();
        for value in [
            f64::from(bounds.min.x),
            f64::from(bounds.min.y),
            f64::from(bounds.min.z),
            f64::from(bounds.max.x),
            f64::from(bounds.max.y),
            f64::from(bounds.max.z),
            profile.maximum_step,
            profile.maximum_drop,
            profile.minimum_floor_normal,
        ] {
            if !value.is_finite() {
                return Err(BotsError::NonFiniteProfile);
            }
        }
        if bounds.min.x >= bounds.max.x
            || bounds.min.y >= bounds.max.y
            || bounds.min.z >= bounds.max.z
            || profile.maximum_step < 0.0
            || profile.maximum_drop < 0.0
            || profile.minimum_floor_normal <= 0.0
            || profile.minimum_floor_normal > 1.0
        {
            return Err(BotsError::BadProfileEnvelope);
        }
    }
    if profile.policy.family() != profile.movement.kind.family() {
        return Err(BotsError::PolicyMismatch);
    }
    Ok(())
}
