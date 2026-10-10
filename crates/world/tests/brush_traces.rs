use qa_core::primitives::{Bounds, GeometryId, Plane, SurfaceFlags, Vec3};
use qa_world::area::AreaGrid;
use qa_world::collision::brushes::{Brush, BrushTree};
use qa_world::collision::{
    CollisionStore, Contents, EntityTracePolicy, Trace, TraceQuery, TraceRules, WorldTrace,
};
use qa_world::entities::EntityTable;

struct Case {
    store: CollisionStore,
    geometry: GeometryId,
}
fn load(planes: Vec<Plane>, brushes: Vec<Brush>) -> Result<Case, &'static str> {
    let tree = BrushTree::direct(brushes.len()).map_err(|_| "tree")?;
    let surfaces = vec![SurfaceFlags::default(); planes.len()];
    let mut store = CollisionStore::new();
    let geometry = store
        .load_brushes(
            planes,
            brushes,
            qa_world::collision::surfaces::SurfaceTable::flags(surfaces),
            tree,
            vec![Bounds {
                mins: Vec3([-32768.0; 3]),
                maxs: Vec3([32768.0; 3]),
            }],
        )
        .map_err(|_| "store")?;
    Ok(Case { store, geometry })
}

fn entity_rules(rules: TraceRules) -> EntityTracePolicy {
    if rules == TraceRules::ARENA {
        qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake3).1
    } else {
        qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake2).1
    }
}

fn trace_world(geometry: &Case, query: TraceQuery<'_>) -> Trace {
    let entities = EntityTable::new(2, 1).unwrap();
    let area = AreaGrid::load(
        2,
        Bounds {
            mins: Vec3([-4096.0; 3]),
            maxs: Vec3([4096.0; 3]),
        },
    )
    .unwrap();
    let mut scratch = geometry.store.scratch();
    WorldTrace::new(
        &geometry.store,
        geometry.geometry,
        0,
        &entities,
        &area,
        &mut scratch,
        None,
    )
    .trace(query)
}

fn point_contents(geometry: &Case, point: Vec3, rules: EntityTracePolicy) -> Contents {
    let entities = EntityTable::new(2, 1).unwrap();
    let area = AreaGrid::load(
        2,
        Bounds {
            mins: Vec3([-4096.0; 3]),
            maxs: Vec3([4096.0; 3]),
        },
    )
    .unwrap();
    let mut scratch = geometry.store.scratch();
    WorldTrace::new(
        &geometry.store,
        geometry.geometry,
        0,
        &entities,
        &area,
        &mut scratch,
        None,
    )
    .point_contents(point, rules, &[])
}

fn box_map() -> Case {
    let planes = (0..6)
        .map(|index| {
            let mut normal = [0.0; 3];
            normal[index / 2] = if index % 2 == 0 { 1.0 } else { -1.0 };
            Plane {
                normal: Vec3(normal),
                distance: 10.0,
                axis: None,
            }
        })
        .collect();
    load(
        planes,
        vec![Brush {
            first_plane: 0,
            plane_count: 6,
            contents: Contents::SOLID,
        }],
    )
    .unwrap()
}

#[test]
fn caller_rules_choose_contact_on_the_same_brush_geometry() {
    let world = box_map();
    for rules in [TraceRules::LEGACY, TraceRules::ARENA, TraceRules::LEGACY] {
        let epsilon = rules.contact_epsilon as f32;
        for half_width in [16.0, 16.0, 15.0] {
            let trace = trace_world(
                &world,
                TraceQuery {
                    start: Vec3([100.0, 0.0, 0.0]),
                    end: Vec3::default(),
                    mins: Vec3([-half_width, -half_width, -24.0]),
                    maxs: Vec3([half_width, half_width, 32.0]),
                    mask: Contents::SOLID,
                    rules,
                    entity_rules: entity_rules(rules),
                    pass: None,
                    excluded: &[],
                },
            );
            assert_eq!(
                trace.fraction,
                (100.0 - 10.0 - half_width - epsilon) / 100.0
            );
            assert_eq!(trace.plane.normal, Vec3([1.0, 0.0, 0.0]));
            assert_eq!(trace.contents, Contents::SOLID);
            assert!(!trace.start_solid);
        }
        assert_eq!(
            point_contents(&world, Vec3::default(), entity_rules(rules)),
            Contents::SOLID
        );
        assert_eq!(
            point_contents(&world, Vec3([20.0, 0.0, 0.0]), entity_rules(rules)),
            Contents::EMPTY
        );
        assert_eq!(
            trace_world(
                &world,
                TraceQuery {
                    start: Vec3([100.0, 0.0, 0.0]),
                    end: Vec3::default(),
                    mins: Vec3::default(),
                    maxs: Vec3::default(),
                    mask: Contents::WATER,
                    rules,
                    entity_rules: entity_rules(rules),
                    pass: None,
                    excluded: &[],
                }
            )
            .fraction,
            1.0
        );
    }
}

#[test]
fn embedded_motion_keeps_original_brush_trace_semantics() {
    let world = box_map();
    for (rules, fraction, contents) in [
        (TraceRules::LEGACY, 1.0, Contents::EMPTY),
        (TraceRules::ARENA, 0.0, Contents::SOLID),
    ] {
        let query = TraceQuery {
            start: Vec3::default(),
            end: Vec3([1.0, 0.0, 0.0]),
            mins: Vec3::default(),
            maxs: Vec3::default(),
            mask: Contents::SOLID,
            rules,
            entity_rules: entity_rules(rules),
            pass: None,
            excluded: &[],
        };
        let trace = trace_world(&world, query);
        assert!(trace.start_solid && trace.all_solid);
        assert_eq!(trace.fraction, fraction);
        assert_eq!(trace.contents, contents);
        let still = trace_world(
            &world,
            TraceQuery {
                end: query.start,
                ..query
            },
        );
        assert_eq!(still.fraction, 0.0);
        assert_eq!(still.contents, Contents::SOLID);
    }
}

fn half_spaces(normals: &[Vec3]) -> Case {
    load(
        normals
            .iter()
            .map(|&normal| Plane {
                normal,
                distance: 0.0,
                axis: None,
            })
            .collect(),
        vec![Brush {
            first_plane: 0,
            plane_count: normals.len() as u32,
            contents: Contents::SOLID,
        }],
    )
    .unwrap()
}

fn point(start: Vec3, end: Vec3, rules: TraceRules) -> TraceQuery<'static> {
    TraceQuery::point(start, end, rules, entity_rules(rules))
}

#[test]
fn q2_approaching_positive_plane_can_contact_before_reaching_its_interior() {
    // CM_ClipBoxToBrush: d1=1/16, d2=1/64. Both are positive;
    // the epsilon boundary at1/32 is reached at2/3 of the move.
    let world = half_spaces(&[Vec3([1.0, 0.0, 0.0])]);
    let trace = trace_world(
        &world,
        point(
            Vec3([0.0625, 0.0, 0.0]),
            Vec3([0.015625, 0.0, 0.0]),
            TraceRules::LEGACY,
        ),
    );
    assert_eq!(trace.fraction.to_bits(), (2.0f32 / 3.0).to_bits());
    assert_eq!(trace.end.0[0], 0.03125);
    assert_eq!(trace.plane.normal, Vec3([1.0, 0.0, 0.0]));
    assert!(!trace.start_solid && !trace.all_solid);
    let retreat = trace_world(
        &world,
        point(
            Vec3([0.015625, 0.0, 0.0]),
            Vec3([0.0625, 0.0, 0.0]),
            TraceRules::LEGACY,
        ),
    );
    assert_eq!(retreat.fraction, 1.0);
}

#[test]
fn q3_clamps_each_enter_fraction_before_selecting_the_contact_plane() {
    // Original Q3 gives -1/2 and -1/4, clamps both to0, and keeps
    // the first plane. Selecting first and clamping later picks Y instead.
    let world = half_spaces(&[Vec3([1.0, 0.0, 0.0]), Vec3([0.0, 1.0, 0.0])]);
    let query = point(
        Vec3([0.0625, 0.09375, 0.0]),
        Vec3([-0.0625, -0.03125, 0.0]),
        TraceRules::ARENA,
    );
    let arena = trace_world(&world, query);
    assert_eq!(arena.fraction, 0.0);
    assert_eq!(arena.plane.normal, Vec3([1.0, 0.0, 0.0]));
    let legacy = trace_world(
        &world,
        TraceQuery {
            rules: TraceRules::LEGACY,
            entity_rules: qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake2)
                .1,
            ..query
        },
    );
    assert_eq!(legacy.fraction, 0.5);
    assert_eq!(legacy.plane.normal, Vec3([0.0, 1.0, 0.0]));
}

#[test]
fn q3_endpoint_epsilon_rejection_is_inclusive_and_caller_selected() {
    let world = half_spaces(&[Vec3([1.0, 0.0, 0.0])]);
    let query = point(
        Vec3([0.25, 0.0, 0.0]),
        Vec3([0.125, 0.0, 0.0]),
        TraceRules::ARENA,
    );
    assert_eq!(trace_world(&world, query).fraction, 1.0);
    let below = f32::from_bits(0.125f32.to_bits() - 1);
    // One ULP below rounds the f32 denominator back to1/8; the native
    // fraction remains1. Two ULPs below survive that subtraction.
    assert_eq!(
        trace_world(
            &world,
            TraceQuery {
                end: Vec3([below, 0.0, 0.0]),
                ..query
            }
        )
        .fraction,
        1.0
    );
    let below = f32::from_bits(0.125f32.to_bits() - 2);
    let hit = trace_world(
        &world,
        TraceQuery {
            end: Vec3([below, 0.0, 0.0]),
            ..query
        },
    );
    assert_eq!(hit.fraction.to_bits(), 0x3f7ffffe);
    assert_eq!(hit.plane.normal, Vec3([1.0, 0.0, 0.0]));
    assert_eq!(
        trace_world(
            &world,
            TraceQuery {
                rules: TraceRules::LEGACY,
                entity_rules: qa_world::collision::trace_policy(
                    qa_core::primitives::RuleSetId::Quake2
                )
                .1,
                end: Vec3([below, 0.0, 0.0]),
                ..query
            }
        )
        .fraction,
        1.0
    );
}

#[test]
fn q3_centered_box_math_preserves_native_rounding_at_large_origins() {
    let origin = 16_777_216.0;
    let world = load(
        vec![Plane {
            normal: Vec3([1.0, 0.0, 0.0]),
            distance: origin,
            axis: None,
        }],
        vec![Brush {
            first_plane: 0,
            plane_count: 1,
            contents: Contents::SOLID,
        }],
    )
    .unwrap();
    let query = TraceQuery {
        start: Vec3([origin + 6.0, 0.0, 0.0]),
        end: Vec3([origin + 4.0, 0.0, 0.0]),
        mins: Vec3([-2.0, 0.0, 0.0]),
        maxs: Vec3([4.0, 0.0, 0.0]),
        mask: Contents::SOLID,
        rules: TraceRules::ARENA,
        entity_rules: qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake3).1,
        pass: None,
        excluded: &[],
    };
    // Native centering adds1 to both positions. Those additions round to
    // origin+8 and origin+4; the expanded plane rounds to origin+4. Thus
    // d1=4,d2=0 and (4-1/8)/4=31/32, before restoring the original endpoint.
    let centered = trace_world(&world, query);
    assert_eq!(centered.fraction, 0.96875);
    assert_eq!(centered.end.0[0], origin + 4.0);
    let supplied = trace_world(
        &world,
        TraceQuery {
            rules: TraceRules::LEGACY,
            entity_rules: qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake2)
                .1,
            ..query
        },
    );
    assert_eq!(supplied.fraction, 1.0);
}

fn overlapping_contents() -> Case {
    load(
        vec![
            Plane {
                normal: Vec3([1.0, 0.0, 0.0]),
                distance: 0.0,
                axis: None,
            },
            Plane {
                normal: Vec3([1.0, 0.0, 0.0]),
                distance: 1.0,
                axis: None,
            },
        ],
        vec![
            Brush {
                first_plane: 0,
                plane_count: 1,
                contents: Contents::SOLID,
            },
            Brush {
                first_plane: 1,
                plane_count: 1,
                contents: Contents::PLAYER_CLIP,
            },
        ],
    )
    .unwrap()
}

#[test]
fn first_enclosed_zero_fraction_preserves_native_brush_contents() {
    let world = overlapping_contents();
    for (rules, start, end) in [
        (
            TraceRules::ARENA,
            Vec3([-1.0, 0.0, 0.0]),
            Vec3([-2.0, 0.0, 0.0]),
        ),
        (
            TraceRules::LEGACY,
            Vec3([-1.0, 0.0, 0.0]),
            Vec3([-1.0, 0.0, 0.0]),
        ),
        (
            TraceRules::ARENA,
            Vec3([-1.0, 0.0, 0.0]),
            Vec3([-1.0, 0.0, 0.0]),
        ),
    ] {
        // Q2 CM_TestBoxInLeaf and Q3 CM_TraceThroughLeaf return as soon as
        // fraction is zero, before examining the overlapping PLAYER_CLIP.
        let trace = trace_world(
            &world,
            TraceQuery {
                mask: Contents::SOLID | Contents::PLAYER_CLIP,
                ..point(start, end, rules)
            },
        );
        assert_eq!(trace.fraction, 0.0);
        assert_eq!(trace.contents, Contents::SOLID);
        assert!(trace.start_solid && trace.all_solid);
    }
}

#[test]
fn selected_zero_contact_stops_before_an_overlapping_enclosed_brush() {
    let world = overlapping_contents();
    for rules in [TraceRules::LEGACY, TraceRules::ARENA] {
        let trace = trace_world(
            &world,
            TraceQuery {
                mask: Contents::SOLID | Contents::PLAYER_CLIP,
                ..point(Vec3([0.015625, 0.0, 0.0]), Vec3([-0.0625, 0.0, 0.0]), rules)
            },
        );
        assert_eq!(trace.fraction, 0.0);
        assert_eq!(trace.contents, Contents::SOLID);
        assert_eq!(trace.plane.normal, Vec3([1.0, 0.0, 0.0]));
        assert!(!trace.start_solid && !trace.all_solid);
    }
}

#[test]
fn unobstructed_trace_copies_endpoint_bits_without_lerp_cancellation() {
    let world = load(Vec::new(), Vec::new()).unwrap();
    let end = Vec3([0.0001, -0.0, 1.0e-20]);
    for rules in [TraceRules::LEGACY, TraceRules::ARENA] {
        let trace = trace_world(&world, point(Vec3([8192.0, 10000.0, 16384.0]), end, rules));
        assert_eq!(trace.fraction, 1.0);
        assert_eq!(trace.end.0.map(f32::to_bits), end.0.map(f32::to_bits));
        assert!(!trace.start_solid && !trace.all_solid);
    }
}
