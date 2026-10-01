//! Collision contents and media ported from `src/world/collision/contents.ts`
//! and `src/world/collision/media.ts`. Source flags stay in their geometry;
//! only the selected gameplay boundary maps them.

use qa_core::math::{vec3, Bounds, Vec3};

use crate::spatial::{CollisionFamily, CollisionShape};

/// Quake III collision runtime ported from `src/world/collision/q3/*`.
pub mod q3;

/// Quake I move policy for traces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1Move {
    /// Normal movement.
    Normal,
    /// Monsters do not block.
    NoMonsters,
    /// Missile with expanded monster bounds.
    Missile,
}

/// Quake II leaf-contents selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeafContents {
    /// Stored leaf contents.
    Stored,
    /// Merged leaf contents.
    Merged,
}

/// Trace policy selecting the gameplay namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TracePolicy {
    /// Quake I negative-contents policy.
    Q1 {
        /// Movement rule.
        movement: Q1Move,
    },
    /// Quake II bitmask policy.
    Q2 {
        /// Contents mask.
        contents_mask: i32,
        /// Leaf-contents selection.
        leaf_contents: LeafContents,
    },
    /// Quake III bitmask policy.
    Q3 {
        /// Contents mask.
        contents_mask: i32,
    },
}

/// Quake I contents values.
pub mod q1 {
    /// Empty leaf.
    pub const CONTENTS_EMPTY: i32 = -1;
    /// Solid leaf.
    pub const CONTENTS_SOLID: i32 = -2;
    /// Water leaf.
    pub const CONTENTS_WATER: i32 = -3;
    /// Slime leaf.
    pub const CONTENTS_SLIME: i32 = -4;
    /// Lava leaf.
    pub const CONTENTS_LAVA: i32 = -5;
    /// Sky leaf.
    pub const CONTENTS_SKY: i32 = -6;
    /// Hull plane distance epsilon (`1/32`).
    pub const DISTANCE_EPSILON: f64 = 1.0 / 32.0;
    /// Player step height.
    pub const STEP_HEIGHT: f32 = 18.0;
}

/// Quake II contents flags and masks.
pub mod q2 {
    /// Solid brush.
    pub const CONTENTS_SOLID: i32 = 1;
    /// Window brush.
    pub const CONTENTS_WINDOW: i32 = 2;
    /// Lava.
    pub const CONTENTS_LAVA: i32 = 8;
    /// Slime.
    pub const CONTENTS_SLIME: i32 = 16;
    /// Water.
    pub const CONTENTS_WATER: i32 = 32;
    /// Ladder.
    pub const CONTENTS_LADDER: i32 = 1 << 29;
    /// Player clip.
    pub const CONTENTS_PLAYERCLIP: i32 = 1 << 16;
    /// Monster clip.
    pub const CONTENTS_MONSTER: i32 = 1 << 25;
    /// Player body.
    pub const CONTENTS_PLAYER: i32 = 1 << 30;
    /// Solid or window.
    pub const MASK_SOLID: i32 = CONTENTS_SOLID | CONTENTS_WINDOW;
    /// Dead-player solid.
    pub const MASK_DEADSOLID: i32 = MASK_SOLID | CONTENTS_PLAYERCLIP;
    /// Player solid, including bodies.
    pub const MASK_PLAYERSOLID: i32 = MASK_DEADSOLID | CONTENTS_MONSTER | CONTENTS_PLAYER;
    /// Classic player solid, without bodies.
    pub const MASK_CLASSIC_PLAYERSOLID: i32 = MASK_DEADSOLID | CONTENTS_MONSTER;
    /// Any liquid.
    pub const MASK_WATER: i32 = CONTENTS_WATER | CONTENTS_LAVA | CONTENTS_SLIME;
    /// Step height.
    pub const STEPSIZE: f32 = 18.0;
    /// Maximum touch list entries.
    pub const MAXTOUCH: usize = 32;
    /// Minimum step plane normal.
    pub const MIN_STEP_NORMAL: f64 = 0.7;
    /// Stuck-move dead zone.
    pub const STOP_EPSILON: f64 = 0.1;
    /// Slick surface flag.
    pub const SURF_SLICK: i32 = 2;
}

/// Convert contents flags between geometry families.
#[must_use]
pub fn convert_contents(contents: i32, from: CollisionFamily, to: CollisionFamily) -> i32 {
    if from == to {
        return contents;
    }
    if from == CollisionFamily::Q1 {
        return match contents {
            -2 => 1,
            -3 => 32,
            -4 => 16,
            -5 => 8,
            -6 => 1,
            -9 => 32 | if to == CollisionFamily::Q2 { 0x40000 } else { 0 },
            -10 => 32 | if to == CollisionFamily::Q2 { 0x80000 } else { 0 },
            -11 => 32 | if to == CollisionFamily::Q2 { 0x100000 } else { 0 },
            -12 => 32 | if to == CollisionFamily::Q2 { 0x200000 } else { 0 },
            -13 => 32 | if to == CollisionFamily::Q2 { 0x400000 } else { 0 },
            -14 => 32 | if to == CollisionFamily::Q2 { 0x800000 } else { 0 },
            _ => 0,
        };
    }
    if to == CollisionFamily::Q1 {
        let solid: u32 = if from == CollisionFamily::Q2 {
            0xc600_0003
        } else {
            0x0600_0001
        };
        let bits = contents as u32;
        if bits & solid != 0 {
            return -2;
        }
        if bits & 8 != 0 {
            return -5;
        }
        if bits & 16 != 0 {
            return -4;
        }
        if bits & 32 != 0 {
            return -3;
        }
        return -1;
    }
    let bits = contents as u32;
    let mut result = bits & 0x0f03_8079;
    if from == CollisionFamily::Q2 {
        if bits & 2 != 0 {
            result |= 1;
        }
        if bits & 0xc000_0000 != 0 {
            result |= 0x0200_0000;
        }
        if bits & 0x1000_0000 != 0 {
            result |= 0x2000_0000;
        }
    } else {
        if bits & 0x2000_0000 != 0 {
            result |= 0x1000_0000;
        }
        if bits & 0x0200_0000 != 0 {
            result |= 0x4000_0000;
        }
    }
    result as i32
}

/// Map an actor's contents into a gameplay namespace. Q1 box actors map to
/// the shared body bit; dead monsters map to the corpse bit.
#[must_use]
pub fn actor_contents(
    family: CollisionFamily,
    shape: CollisionShape,
    contents: i32,
    dead_monster: bool,
    to: CollisionFamily,
) -> i32 {
    if family == CollisionFamily::Q1 && !matches!(shape, CollisionShape::Model(_)) && to != CollisionFamily::Q1 {
        return if dead_monster { 0x0400_0000 } else { 0x0200_0000 };
    }
    convert_contents(contents, family, to)
}

/// Whether contents block a trace under a gameplay policy.
#[must_use]
pub fn contents_block(contents: i32, family: CollisionFamily, policy: TracePolicy) -> bool {
    match policy {
        TracePolicy::Q1 { .. } => convert_contents(contents, family, CollisionFamily::Q1) == -2,
        TracePolicy::Q2 { contents_mask, .. } | TracePolicy::Q3 { contents_mask } => {
            let kind = match policy {
                TracePolicy::Q2 { .. } => CollisionFamily::Q2,
                _ => CollisionFamily::Q3,
            };
            convert_contents(contents, family, kind) & contents_mask != 0
        }
    }
}

/// Convert a gameplay mask to a geometry's native flag namespace.
#[must_use]
pub fn geometry_mask(policy: TracePolicy, family: CollisionFamily) -> i32 {
    debug_assert!(family != CollisionFamily::Q1);
    let mut mask: u32 = 0;
    for bit in 0..32 {
        let flag = 1u32 << bit;
        if contents_block(flag as i32, family, policy) {
            mask |= flag;
        }
    }
    mask as i32
}

/// Convert surface flags between families. Slick, sky, and nodraw share
/// their values; Q1 surfaces never convert.
#[must_use]
pub fn convert_surface_flags(flags: i32, from: CollisionFamily, to: CollisionFamily) -> i32 {
    if from == to {
        return flags;
    }
    if from == CollisionFamily::Q1 || to == CollisionFamily::Q1 {
        return 0;
    }
    (flags & 0x86)
        | (if from == CollisionFamily::Q2 && to == CollisionFamily::Q3 && flags & 4 != 0 {
            16
        } else {
            0
        })
}

/// Swept bounds of a trace, with the donor's per-family padding.
#[must_use]
pub fn swept_bounds(start: Vec3, end: Vec3, shape: Option<&Bounds>, policy: TracePolicy) -> Bounds {
    let empty = Bounds {
        min: vec3(0.0, 0.0, 0.0),
        max: vec3(0.0, 0.0, 0.0),
    };
    let bounds = shape.unwrap_or(&empty);
    if matches!(policy, TracePolicy::Q3 { .. }) {
        let axis = |start: f32, end: f32, offset: f32, minimum: bool| {
            let edge = if minimum {
                start.min(end) + offset
            } else {
                start.max(end) + offset
            };
            edge + if minimum { -1.0 } else { 1.0 }
        };
        return Bounds {
            min: vec3(
                axis(start.x, end.x, bounds.min.x, true),
                axis(start.y, end.y, bounds.min.y, true),
                axis(start.z, end.z, bounds.min.z, true),
            ),
            max: vec3(
                axis(start.x, end.x, bounds.max.x, false),
                axis(start.y, end.y, bounds.max.y, false),
                axis(start.z, end.z, bounds.max.z, false),
            ),
        };
    }
    Bounds {
        min: vec3(
            start.x.min(end.x) + bounds.min.x - 1.0,
            start.y.min(end.y) + bounds.min.y - 1.0,
            start.z.min(end.z) + bounds.min.z - 1.0,
        ),
        max: vec3(
            start.x.max(end.x) + bounds.max.x + 1.0,
            start.y.max(end.y) + bounds.max.y + 1.0,
            start.z.max(end.z) + bounds.max.z + 1.0,
        ),
    }
}

/// Medium classification of a reached trace segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceMedia {
    /// Some open segment was reached.
    pub in_open: bool,
    /// Some liquid segment was reached.
    pub in_water: bool,
}

/// One brush side as start/end plane distances.
#[derive(Debug, Clone, PartialEq)]
pub struct MediumBrush {
    /// Native contents.
    pub contents: i32,
    /// Per-plane `(start, end)` distances.
    pub distances: Vec<(f64, f64)>,
}

#[derive(Debug, Clone, Copy)]
struct Interval {
    start: f64,
    end: f64,
}

/// Classify the reached segment without changing brush visitation or impact
/// selection. Distances are precomputed by the calling provider.
#[must_use]
pub fn trace_brush_media(brushes: &[MediumBrush], family: CollisionFamily, fraction: f64) -> TraceMedia {
    let mut solids: Vec<Interval> = Vec::new();
    let mut liquids: Vec<Interval> = Vec::new();
    for brush in brushes {
        let contents = convert_contents(brush.contents, family, CollisionFamily::Q1);
        if contents == q1::CONTENTS_EMPTY {
            continue;
        }
        let mut start = 0.0_f64;
        let mut end = fraction;
        let mut outside = false;
        for (first, last) in &brush.distances {
            if *first > 0.0 && *last > 0.0 {
                outside = true;
                break;
            }
            if *first <= 0.0 && *last <= 0.0 {
                continue;
            }
            let crossing = first / (first - last);
            if first > last {
                start = start.max(crossing);
            } else {
                end = end.min(crossing);
            }
            if start > end {
                outside = true;
                break;
            }
        }
        if !outside && !brush.distances.is_empty() {
            let interval = Interval { start, end };
            if contents == q1::CONTENTS_SOLID {
                solids.push(interval);
            } else {
                liquids.push(interval);
            }
        }
    }
    solids.sort_by(|a, b| a.start.total_cmp(&b.start));
    let mut occupied = solids.clone();
    occupied.extend_from_slice(&liquids);
    occupied.sort_by(|a, b| a.start.total_cmp(&b.start));
    TraceMedia {
        in_open: uncovered(
            Interval {
                start: 0.0,
                end: fraction,
            },
            &occupied,
        ),
        in_water: liquids.iter().any(|interval| uncovered(*interval, &solids)),
    }
}

fn uncovered(interval: Interval, occupied: &[Interval]) -> bool {
    if interval.start == interval.end {
        return !occupied
            .iter()
            .any(|part| part.start <= interval.start && part.end >= interval.end);
    }
    let mut position = interval.start;
    for part in occupied {
        if part.end < position {
            continue;
        }
        if part.start > position {
            return true;
        }
        position = position.max(part.end);
        if position >= interval.end {
            return false;
        }
    }
    position < interval.end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contents_convert_between_families() {
        use CollisionFamily::{Q1, Q2, Q3};
        assert_eq!(convert_contents(-2, Q1, Q2), 1);
        assert_eq!(convert_contents(-3, Q1, Q2), 32);
        assert_eq!(convert_contents(-4, Q1, Q3), 16);
        assert_eq!(convert_contents(-5, Q1, Q3), 8);
        assert_eq!(convert_contents(1, Q2, Q1), -2);
        assert_eq!(convert_contents(8, Q2, Q1), -5);
        assert_eq!(convert_contents(32, Q3, Q1), -3);
        assert_eq!(convert_contents(0, Q2, Q1), -1);
        assert_eq!(convert_contents(7, Q2, Q2), 7);
    }

    #[test]
    fn actor_contents_map_q1_boxes_to_body_bits() {
        use CollisionFamily::{Q1, Q2};
        assert_eq!(actor_contents(Q1, CollisionShape::Box, -2, false, Q2), 0x0200_0000);
        assert_eq!(actor_contents(Q1, CollisionShape::Box, -2, true, Q2), 0x0400_0000);
        assert_eq!(actor_contents(Q1, CollisionShape::Model(1), -2, false, Q2), 1);
    }

    #[test]
    fn blocking_and_surface_flags_follow_the_policy() {
        use CollisionFamily::{Q1, Q2, Q3};
        let q1 = TracePolicy::Q1 {
            movement: Q1Move::Normal,
        };
        assert!(contents_block(-2, Q1, q1));
        assert!(!contents_block(-3, Q1, q1));
        let q2 = TracePolicy::Q2 {
            contents_mask: q2::MASK_PLAYERSOLID,
            leaf_contents: LeafContents::Merged,
        };
        assert!(contents_block(1, Q2, q2));
        assert!(!contents_block(32, Q2, q2));
        assert_eq!(convert_surface_flags(0x86, Q2, Q3), 0x96);
        assert_eq!(convert_surface_flags(4, Q2, Q3), 4 | 16);
        assert_eq!(convert_surface_flags(7, Q1, Q2), 0);
    }

    #[test]
    fn swept_bounds_pad_per_family() {
        let start = vec3(0.0, 0.0, 0.0);
        let end = vec3(10.0, 0.0, 0.0);
        let q1 = swept_bounds(
            start,
            end,
            None,
            TracePolicy::Q1 {
                movement: Q1Move::Normal,
            },
        );
        assert_eq!(q1.min.x, -1.0);
        assert_eq!(q1.max.x, 11.0);
        let q3 = swept_bounds(start, end, None, TracePolicy::Q3 { contents_mask: 1 });
        assert_eq!(q3.min.x, -1.0);
        assert_eq!(q3.max.x, 11.0);
    }

    #[test]
    fn media_classifies_open_and_liquid_segments() {
        use CollisionFamily::Q2;
        let open = trace_brush_media(&[], Q2, 1.0);
        assert_eq!(
            open,
            TraceMedia {
                in_open: true,
                in_water: false
            }
        );
        let solid = MediumBrush {
            contents: 1,
            distances: vec![(-1.0, -1.0)],
        };
        let covered = trace_brush_media(&[solid], Q2, 1.0);
        assert!(!covered.in_open);
        let liquid = MediumBrush {
            contents: 32,
            distances: vec![(-1.0, -1.0)],
        };
        let wet = trace_brush_media(&[liquid], Q2, 1.0);
        assert!(wet.in_water);
    }
}
