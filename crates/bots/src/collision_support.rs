//! Shared collision-query support: contents adaptation, gameplay masks, and
//! numeric-profile validation for the Quake scene adapters.
//!
//! Donor provenance:
//! - `/home/buzzkill/Projects/quake-typescript/src/world/collision/contents.ts`
//!   (`contentsBlock`, `blocksQ1Contents`, `geometryMask`, `adaptTraceResult`,
//!   `adaptPointContents`)
//! - `/home/buzzkill/Projects/quake-typescript/src/core/numeric.ts`
//!   (`createNumericOperations` backend validation)

use qa_core::math::{Plane, Vec3};
use qa_core::numeric::{Arithmetic, NumericOps, NumericProfile, Rounding};
use qa_world::collision::convert_contents;
use qa_world::spatial::CollisionFamily;
use qa_world::WorldError;

use crate::scene::{BspPlane, PointContentsResult, TraceContact, TraceDetail, TraceHit, TracePolicy, TraceResult};

/// Validate a numeric profile and select its operations, with the donor's
/// backend error messages.
pub fn select_numeric(profile: &NumericProfile) -> Result<NumericOps, WorldError> {
    match &profile.arithmetic {
        Arithmetic::Binary32EachOp
        | Arithmetic::DonorBinary64(_)
        | Arithmetic::Sse {
            flush_to_zero: false,
            denormals_are_zero: false,
            rounding: Rounding::NearestEven,
        } => {}
        Arithmetic::Sse { .. } => {
            return Err(WorldError::BadCollisionRecord(format!(
                "Numeric profile {} requires an SSE arithmetic backend",
                profile.id
            )));
        }
        Arithmetic::X87 { .. } => {
            return Err(WorldError::BadCollisionRecord(format!(
                "Numeric profile {} requires an x87 arithmetic backend",
                profile.id
            )));
        }
    }
    NumericOps::select(*profile).map_err(|error| WorldError::BadCollisionRecord(error.to_string()))
}

/// Whether contents block a trace under a gameplay policy.
#[must_use]
pub fn contents_block(contents: i32, family: CollisionFamily, policy: &TracePolicy) -> bool {
    if matches!(policy, TracePolicy::Q1 { .. }) {
        return convert_contents(contents, family, CollisionFamily::Q1) == -2;
    }
    let (kind, mask) = match policy {
        TracePolicy::Q2 { contents_mask, .. } => (CollisionFamily::Q2, contents_mask),
        TracePolicy::Q3 { contents_mask, .. } => (CollisionFamily::Q3, contents_mask),
        TracePolicy::Q1 { .. } => unreachable!("q1 policies return above"),
    };
    convert_contents(contents, family, kind) & mask != 0
}

/// Whether Quake I contents block under a gameplay policy.
#[must_use]
pub fn blocks_q1_contents(contents: i32, policy: &TracePolicy) -> bool {
    contents_block(contents, CollisionFamily::Q1, policy)
}

/// Convert a gameplay mask to the geometry's native flag namespace.
#[must_use]
pub fn geometry_mask(policy: &TracePolicy, family: CollisionFamily) -> i32 {
    let mut mask = 0;
    for bit in 0..32 {
        let flag = 1i32 << bit;
        if contents_block(flag, family, policy) {
            mask |= flag;
        }
    }
    mask
}

/// Source plane for adapted results: the contact plane normal over the
/// stored distance. The scene subset keeps a tagless plane, so the donor's
/// type and sign bits do not cross.
fn adapted_source_plane(result: &TraceResult) -> Plane {
    let normal = match &result.contact {
        TraceContact::Plane { plane } => plane.normal,
        TraceContact::None => match &result.detail {
            TraceDetail::Q1 { source_plane, .. } => source_plane.normal,
            TraceDetail::Q2 { source_plane, .. } | TraceDetail::Q3 { source_plane, .. } => source_plane.normal,
        },
    };
    let distance = match &result.detail {
        TraceDetail::Q1 { source_plane, .. } => source_plane.distance,
        TraceDetail::Q2 { source_plane, .. } | TraceDetail::Q3 { source_plane, .. } => source_plane.distance,
    };
    Plane { normal, distance }
}

/// Native contents behind a trace result, per family.
fn native_contents(result: &TraceResult, q1_contents: Option<i32>) -> i32 {
    match &result.detail {
        TraceDetail::Q1 { .. } => q1_contents.unwrap_or(0),
        TraceDetail::Q2 { contents, .. } => *contents,
        TraceDetail::Q3 { contents, .. } => match &result.hit {
            TraceHit::World { .. } => *contents,
            _ => 0,
        },
    }
}

/// Native family behind a trace result, from its detail variant.
fn family_of_detail(detail: &TraceDetail) -> CollisionFamily {
    match detail {
        TraceDetail::Q1 { .. } => CollisionFamily::Q1,
        TraceDetail::Q2 { .. } => CollisionFamily::Q2,
        TraceDetail::Q3 { .. } => CollisionFamily::Q3,
    }
}

/// Adapt a native trace result to a gameplay policy.
#[must_use]
pub fn adapt_trace_result(result: &TraceResult, policy: &TracePolicy, q1_contents: Option<i32>) -> TraceResult {
    let source_plane = adapted_source_plane(result);
    let native = native_contents(result, q1_contents);
    let detail = match policy {
        TracePolicy::Q1 { .. } => {
            let surface_flags = match &result.hit {
                TraceHit::World { .. } => match &result.detail {
                    TraceDetail::Q1 { surface_flags, .. } => *surface_flags,
                    TraceDetail::Q2 { surface, .. } => surface.as_ref().map(|info| info.flags),
                    TraceDetail::Q3 { surface_flags, .. } => Some(*surface_flags),
                },
                _ => None,
            };
            let in_open = !contents_block(native, family_of_detail(&result.detail), policy);
            TraceDetail::Q1 {
                source_plane,
                surface_flags,
                in_open,
                in_water: !in_open
                    && convert_contents(native, family_of_detail(&result.detail), CollisionFamily::Q1) == -3,
            }
        }
        TracePolicy::Q2 { .. } => {
            let (contents, surface, plane) = (
                convert_contents(native, family_of_detail(&result.detail), CollisionFamily::Q2),
                match &result.hit {
                    TraceHit::World { .. } => match &result.detail {
                        TraceDetail::Q2 { surface, .. } => surface.clone(),
                        _ => None,
                    },
                    _ => None,
                },
                BspPlane {
                    normal: source_plane.normal,
                    distance: source_plane.distance,
                    plane_type: plane_type_of(source_plane.normal),
                    signbits: plane_signbits_of(source_plane.normal),
                },
            );
            TraceDetail::Q2 {
                contents,
                surface,
                source_plane: plane,
            }
        }
        TracePolicy::Q3 { .. } => {
            let contents = convert_contents(native, family_of_detail(&result.detail), CollisionFamily::Q3);
            let surface_flags = match &result.hit {
                TraceHit::World { .. } => match &result.detail {
                    TraceDetail::Q2 { surface, .. } => surface.as_ref().map(|info| info.flags).unwrap_or(0),
                    TraceDetail::Q3 { surface_flags, .. } => *surface_flags,
                    TraceDetail::Q1 { surface_flags, .. } => surface_flags.unwrap_or(0),
                },
                _ => 0,
            };
            TraceDetail::Q3 {
                contents,
                surface_flags,
                source_plane: BspPlane {
                    normal: source_plane.normal,
                    distance: source_plane.distance,
                    plane_type: plane_type_of(source_plane.normal),
                    signbits: plane_signbits_of(source_plane.normal),
                },
            }
        }
    };
    TraceResult {
        fraction: result.fraction,
        end: result.end,
        start_solid: result.start_solid,
        all_solid: result.all_solid,
        contact: result.contact,
        hit: result.hit.clone(),
        detail,
    }
}

fn plane_type_of(normal: Vec3) -> i32 {
    if normal.x == 1.0 {
        0
    } else if normal.y == 1.0 {
        1
    } else if normal.z == 1.0 {
        2
    } else {
        3
    }
}

fn plane_signbits_of(normal: Vec3) -> i32 {
    i32::from(normal.x < 0.0) | (i32::from(normal.y < 0.0) << 1) | (i32::from(normal.z < 0.0) << 2)
}

/// Adapt native point contents to a gameplay policy.
#[must_use]
pub fn adapt_point_contents(result: &PointContentsResult, policy: &TracePolicy) -> PointContentsResult {
    let native = match result {
        PointContentsResult::Q1 { contents } => *contents,
        PointContentsResult::Q2 { stored, merged } => {
            if matches!(policy, TracePolicy::Q2 { .. }) {
                *stored
            } else {
                *merged
            }
        }
        PointContentsResult::Q3 { contents } => *contents,
    };
    let from = match result {
        PointContentsResult::Q1 { .. } => CollisionFamily::Q1,
        PointContentsResult::Q2 { .. } => CollisionFamily::Q2,
        PointContentsResult::Q3 { .. } => CollisionFamily::Q3,
    };
    match policy {
        TracePolicy::Q1 { .. } => PointContentsResult::Q1 {
            contents: convert_contents(native, from, CollisionFamily::Q1),
        },
        TracePolicy::Q2 { .. } => PointContentsResult::Q2 {
            stored: native,
            merged: convert_contents(native, from, CollisionFamily::Q2),
        },
        TracePolicy::Q3 { .. } => PointContentsResult::Q3 {
            contents: convert_contents(native, from, CollisionFamily::Q3),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::math::vec3;
    use qa_core::numeric::{FloatToInt, NumericProfile, Q3_BINARY32_PROFILE};

    use crate::scene::{LeafContents, Q1MoveRule};

    fn q3_policy() -> TracePolicy {
        TracePolicy::Q3 {
            contents_mask: 1,
            curves: true,
            player_curve_clip: true,
        }
    }

    #[test]
    fn numeric_selection_matches_donor() {
        assert!(select_numeric(&Q3_BINARY32_PROFILE).is_ok());
        let error = select_numeric(&NumericProfile {
            id: "q2:x87",
            arithmetic: Arithmetic::X87 {
                precision_bits: qa_core::numeric::X87Precision::Bits53,
                rounding: Rounding::NearestEven,
            },
            float_to_int: FloatToInt::CheckedTruncation,
        })
        .expect_err("x87 must fail");
        assert_eq!(
            error.to_string(),
            "Numeric profile q2:x87 requires an x87 arithmetic backend"
        );
        let error = select_numeric(&NumericProfile {
            id: "q2:sse-sloppy",
            arithmetic: Arithmetic::Sse {
                flush_to_zero: true,
                denormals_are_zero: false,
                rounding: Rounding::NearestEven,
            },
            float_to_int: FloatToInt::CheckedTruncation,
        })
        .expect_err("sloppy SSE must fail");
        assert_eq!(
            error.to_string(),
            "Numeric profile q2:sse-sloppy requires an SSE arithmetic backend"
        );
    }

    #[test]
    fn masks_and_blocks_match_donor() {
        let policy = q3_policy();
        assert!(contents_block(1, CollisionFamily::Q3, &policy));
        assert!(!contents_block(2, CollisionFamily::Q3, &policy));
        assert!(blocks_q1_contents(-2, &policy));
        assert!(!blocks_q1_contents(-1, &policy));
        assert_eq!(geometry_mask(&policy, CollisionFamily::Q3), 1);
        let swim = TracePolicy::Q1 {
            move_rule: Q1MoveRule::Normal,
            hull: None,
        };
        assert!(contents_block(-2, CollisionFamily::Q1, &swim));
        assert!(!contents_block(-3, CollisionFamily::Q1, &swim));
    }

    #[test]
    fn adaptation_matches_donor() {
        let brush = TraceResult {
            fraction: 0.5,
            end: vec3(0.0, 0.0, 0.0),
            start_solid: false,
            all_solid: false,
            contact: TraceContact::Plane {
                plane: Plane {
                    normal: vec3(0.0, 0.0, 1.0),
                    distance: 4.0,
                },
            },
            hit: TraceHit::World { model: 0 },
            detail: TraceDetail::Q3 {
                contents: 1,
                surface_flags: 7,
                source_plane: BspPlane {
                    normal: vec3(0.0, 0.0, 1.0),
                    distance: 4.0,
                    plane_type: 2,
                    signbits: 0,
                },
            },
        };
        let adapted = adapt_trace_result(
            &brush,
            &TracePolicy::Q1 {
                move_rule: Q1MoveRule::Normal,
                hull: None,
            },
            None,
        );
        assert!(matches!(adapted.detail, TraceDetail::Q1 { .. }));
        match &adapted.detail {
            TraceDetail::Q1 {
                in_open,
                in_water,
                surface_flags,
                ..
            } => {
                assert!(!in_open);
                assert!(!in_water);
                assert_eq!(*surface_flags, Some(7));
            }
            _ => panic!("q1 detail"),
        }
        let back = adapt_trace_result(&brush, &q3_policy(), None);
        match &back.detail {
            TraceDetail::Q3 {
                contents,
                surface_flags,
                ..
            } => {
                assert_eq!(*contents, 1);
                assert_eq!(*surface_flags, 7);
            }
            _ => panic!("q3 detail"),
        }
        let contents = adapt_point_contents(&PointContentsResult::Q3 { contents: 1 }, &q3_policy());
        assert_eq!(contents, PointContentsResult::Q3 { contents: 1 });
        let merged = adapt_point_contents(
            &PointContentsResult::Q2 { stored: 3, merged: 1 },
            &TracePolicy::Q2 {
                contents_mask: 1,
                leaf_contents: LeafContents::Stored,
            },
        );
        assert_eq!(merged, PointContentsResult::Q2 { stored: 3, merged: 3 });
    }
}
