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
use qa_world::collision::{convert_contents, convert_surface_flags};
use qa_world::spatial::CollisionFamily;
use qa_world::WorldError;

use crate::scene::{
    BspPlane, PointContentsResult, Q2SurfaceInfo, TraceContact, TraceDetail, TraceHit, TracePolicy, TraceResult,
};

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

/// Whether a result already speaks the policy's dialect.
fn same_family(detail: &TraceDetail, policy: &TracePolicy) -> bool {
    matches!(
        (detail, policy),
        (TraceDetail::Q1 { .. }, TracePolicy::Q1 { .. })
            | (TraceDetail::Q2 { .. }, TracePolicy::Q2 { .. })
            | (TraceDetail::Q3 { .. }, TracePolicy::Q3 { .. })
    )
}

/// Native contents behind a trace result. Native Quake I traces carry
/// contents; adapted ones fall back to the hit record (`adaptTraceResult`).
fn native_contents(result: &TraceResult) -> i32 {
    match &result.detail {
        TraceDetail::Q1 { contents, .. } => contents.unwrap_or({
            if result.start_solid || !matches!(result.hit, TraceHit::None) {
                -2
            } else {
                -1
            }
        }),
        TraceDetail::Q2 { contents, .. } | TraceDetail::Q3 { contents, .. } => *contents,
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

/// Source plane for adapted results: native planes pass through; Quake I
/// planes take the contact normal over the stored distance and gain type
/// and sign tags (`planeWithType`).
fn adapted_source_plane(result: &TraceResult) -> BspPlane {
    match &result.detail {
        TraceDetail::Q2 { source_plane, .. } | TraceDetail::Q3 { source_plane, .. } => *source_plane,
        TraceDetail::Q1 { source_plane, .. } => {
            let normal = match &result.contact {
                TraceContact::Plane { plane } => plane.normal,
                TraceContact::None => source_plane.normal,
            };
            BspPlane {
                normal,
                distance: source_plane.distance,
                plane_type: plane_type_of(normal),
                signbits: plane_signbits_of(normal),
            }
        }
    }
}

/// Whether a Quake I result hit sky: sky contents or the sky surface bit.
fn q1_sky(result: &TraceResult, native: i32) -> bool {
    match &result.detail {
        TraceDetail::Q1 { surface_flags, .. } => native == -6 || surface_flags.unwrap_or(0) & 4 != 0,
        _ => false,
    }
}

/// Adapt a native trace result to a gameplay policy (`adaptTraceResult`).
/// Same-dialect results pass through untouched.
#[must_use]
pub fn adapt_trace_result(result: &TraceResult, policy: &TracePolicy) -> TraceResult {
    if same_family(&result.detail, policy) {
        return result.clone();
    }
    let native = native_contents(result);
    let from = family_of_detail(&result.detail);
    let source = adapted_source_plane(result);
    let contact = result.contact;
    let hit = result.hit.clone();
    let detail = match policy {
        TracePolicy::Q1 { .. } => {
            let surface_flags = match &result.detail {
                TraceDetail::Q2 { surface, .. } => surface.as_ref().map(|info| info.flags).unwrap_or(0),
                TraceDetail::Q3 { surface_flags, .. } => *surface_flags,
                TraceDetail::Q1 { .. } => unreachable!("q1 results return above"),
            };
            TraceDetail::Q1 {
                source_plane: Plane {
                    normal: source.normal,
                    distance: source.distance,
                },
                surface_flags: Some(surface_flags),
                in_open: !result.all_solid,
                in_water: native & 56 != 0,
                contents: None,
            }
        }
        TracePolicy::Q2 { .. } => {
            let contents = convert_contents(native, from, CollisionFamily::Q2);
            let surface = match &result.detail {
                TraceDetail::Q3 { surface_flags, .. } => Some(Q2SurfaceInfo {
                    name: String::new(),
                    flags: convert_surface_flags(*surface_flags, CollisionFamily::Q3, CollisionFamily::Q2),
                }),
                _ if q1_sky(result, native) => Some(Q2SurfaceInfo {
                    name: "sky".to_string(),
                    flags: 4,
                }),
                _ if !matches!(result.hit, TraceHit::None) => Some(Q2SurfaceInfo {
                    name: String::new(),
                    flags: 0,
                }),
                _ => None,
            };
            TraceDetail::Q2 {
                contents,
                surface,
                source_plane: source,
                secondary: None,
            }
        }
        TracePolicy::Q3 { .. } => {
            let contents = convert_contents(native, from, CollisionFamily::Q3);
            let surface_flags = match &result.detail {
                TraceDetail::Q2 {
                    surface: Some(info), ..
                } => convert_surface_flags(info.flags, CollisionFamily::Q2, CollisionFamily::Q3),
                TraceDetail::Q2 { .. } => 0,
                _ if q1_sky(result, native) => 4 | 16,
                _ => 0,
            };
            TraceDetail::Q3 {
                contents,
                surface_flags,
                source_plane: source,
            }
        }
    };
    TraceResult {
        fraction: result.fraction,
        end: result.end,
        start_solid: result.start_solid,
        all_solid: result.all_solid,
        contact,
        hit,
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

/// Adapt native point contents to a gameplay policy
/// (`adaptPointContents`). Same-dialect samples pass through untouched;
/// Quake II samples convert from merged contents.
#[must_use]
pub fn adapt_point_contents(result: &PointContentsResult, policy: &TracePolicy) -> PointContentsResult {
    let same = matches!(
        (result, policy),
        (PointContentsResult::Q1 { .. }, TracePolicy::Q1 { .. })
            | (PointContentsResult::Q2 { .. }, TracePolicy::Q2 { .. })
            | (PointContentsResult::Q3 { .. }, TracePolicy::Q3 { .. })
    );
    if same {
        return *result;
    }
    let (native, from) = match result {
        PointContentsResult::Q1 { contents } => (*contents, CollisionFamily::Q1),
        PointContentsResult::Q2 { merged, .. } => (*merged, CollisionFamily::Q2),
        PointContentsResult::Q3 { contents } => (*contents, CollisionFamily::Q3),
    };
    match policy {
        TracePolicy::Q1 { .. } => PointContentsResult::Q1 {
            contents: convert_contents(native, from, CollisionFamily::Q1),
        },
        TracePolicy::Q2 { .. } => {
            let contents = convert_contents(native, from, CollisionFamily::Q2);
            PointContentsResult::Q2 {
                stored: contents,
                merged: contents,
            }
        }
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
        );
        assert!(matches!(adapted.detail, TraceDetail::Q1 { .. }));
        match &adapted.detail {
            TraceDetail::Q1 {
                in_open,
                in_water,
                surface_flags,
                contents,
                ..
            } => {
                assert!(in_open);
                assert!(!in_water);
                assert_eq!(*surface_flags, Some(7));
                assert_eq!(*contents, None);
            }
            _ => panic!("q1 detail"),
        }
        let back = adapt_trace_result(&brush, &q3_policy());
        assert_eq!(back, brush);
        let wet = TraceResult {
            detail: TraceDetail::Q3 {
                contents: 33,
                surface_flags: 0,
                source_plane: BspPlane {
                    normal: vec3(0.0, 0.0, 1.0),
                    distance: 4.0,
                    plane_type: 2,
                    signbits: 0,
                },
            },
            ..brush.clone()
        };
        match adapt_trace_result(
            &wet,
            &TracePolicy::Q1 {
                move_rule: Q1MoveRule::Normal,
                hull: None,
            },
        )
        .detail
        {
            TraceDetail::Q1 { in_water, .. } => assert!(in_water),
            _ => panic!("q1 detail"),
        }
        let solid = TraceResult {
            start_solid: false,
            all_solid: false,
            contact: TraceContact::Plane {
                plane: Plane {
                    normal: vec3(1.0, 0.0, 0.0),
                    distance: 8.0,
                },
            },
            hit: TraceHit::World { model: 0 },
            detail: TraceDetail::Q1 {
                in_open: false,
                in_water: false,
                source_plane: Plane {
                    normal: vec3(0.0, 1.0, 0.0),
                    distance: 8.0,
                },
                surface_flags: None,
                contents: Some(-2),
            },
            ..brush.clone()
        };
        let q2_policy = TracePolicy::Q2 {
            contents_mask: 1,
            leaf_contents: LeafContents::Stored,
        };
        match adapt_trace_result(&solid, &q2_policy).detail {
            TraceDetail::Q2 {
                contents,
                surface,
                source_plane,
                secondary,
            } => {
                assert_eq!(contents, 1);
                assert_eq!(
                    surface,
                    Some(Q2SurfaceInfo {
                        name: String::new(),
                        flags: 0,
                    })
                );
                assert_eq!(source_plane.normal, vec3(1.0, 0.0, 0.0));
                assert_eq!(source_plane.plane_type, 0);
                assert_eq!(secondary, None);
            }
            _ => panic!("q2 detail"),
        }
        let sky = TraceResult {
            detail: TraceDetail::Q1 {
                in_open: false,
                in_water: false,
                source_plane: Plane {
                    normal: vec3(0.0, 0.0, 1.0),
                    distance: 8.0,
                },
                surface_flags: Some(4),
                contents: None,
            },
            ..brush.clone()
        };
        match adapt_trace_result(&sky, &q2_policy).detail {
            TraceDetail::Q2 { surface, .. } => assert_eq!(
                surface,
                Some(Q2SurfaceInfo {
                    name: "sky".to_string(),
                    flags: 4,
                })
            ),
            _ => panic!("q2 detail"),
        }
        match adapt_trace_result(&sky, &q3_policy()).detail {
            TraceDetail::Q3 { surface_flags, .. } => assert_eq!(surface_flags, 20),
            _ => panic!("q3 detail"),
        }
        let miss = TraceResult {
            contact: TraceContact::None,
            hit: TraceHit::None,
            detail: TraceDetail::Q1 {
                in_open: true,
                in_water: false,
                source_plane: Plane {
                    normal: vec3(0.0, 0.0, 1.0),
                    distance: 0.0,
                },
                surface_flags: None,
                contents: None,
            },
            ..brush.clone()
        };
        match adapt_trace_result(&miss, &q2_policy).detail {
            TraceDetail::Q2 { contents, surface, .. } => {
                assert_eq!(contents, 0);
                assert_eq!(surface, None);
            }
            _ => panic!("q2 detail"),
        }
        let contents = adapt_point_contents(&PointContentsResult::Q3 { contents: 1 }, &q3_policy());
        assert_eq!(contents, PointContentsResult::Q3 { contents: 1 });
        let q2_stored = TracePolicy::Q2 {
            contents_mask: 1,
            leaf_contents: LeafContents::Stored,
        };
        let same = adapt_point_contents(&PointContentsResult::Q2 { stored: 3, merged: 1 }, &q2_stored);
        assert_eq!(same, PointContentsResult::Q2 { stored: 3, merged: 1 });
        let converted = adapt_point_contents(&PointContentsResult::Q2 { stored: 3, merged: 1 }, &q3_policy());
        assert_eq!(converted, PointContentsResult::Q3 { contents: 1 });
    }
}
