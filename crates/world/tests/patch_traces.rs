use qa_core::primitives::{
    Bounds, CollisionOwner, GeometryId, IndexSpan, RuleSetId, SurfaceFlags, Vec3,
};
use qa_world::{
    area::AreaGrid,
    collision::{
        CollisionStore, Contents, PatchClip, StoreError, TraceError, TraceQuery, TraceRules,
        WorldTrace,
        brushes::{Brush, BrushTree, CollisionLeaf, GeometryError, ModelRoot},
        patches::PatchInput,
        surfaces::{SurfaceInput, SurfaceTable},
    },
    entities::EntityTable,
};

fn patch(surface: u32, width: u32) -> PatchInput {
    PatchInput {
        dimensions: [width, width],
        control_points: (0..width)
            .flat_map(|y| {
                (0..width).map(move |x| {
                    Vec3([
                        x as f32 - (width - 1) as f32 * 0.5,
                        y as f32 - (width - 1) as f32 * 0.5,
                        0.0,
                    ])
                })
            })
            .collect(),
        contents: Contents::SOLID,
        surface,
    }
}
fn load(
    store: &mut CollisionStore,
    patches: Vec<PatchInput>,
    order: Vec<u32>,
    inline: bool,
) -> Result<GeometryId, StoreError> {
    let records = (0..patches.len())
        .map(|index| SurfaceInput {
            name: b"curve",
            material: b"",
            flags: SurfaceFlags(index as u64 + 7),
            value: 0,
            source_index: index as u32 + 11,
        })
        .collect();
    let mut tree = BrushTree::direct(0).unwrap();
    let membership = IndexSpan {
        first: 0,
        count: order.len() as u32,
    };
    tree.leaf_patches = order;
    if inline {
        tree.leaves.push(CollisionLeaf {
            stored_contents: None,
            brushes: IndexSpan::default(),
            patches: membership,
        });
        tree.models.push(ModelRoot::Leaf(1));
    } else {
        tree.leaves[0].patches = membership;
    }
    let bounds = vec![
        Bounds {
            mins: Vec3([-1024.0; 3]),
            maxs: Vec3([1024.0; 3])
        };
        tree.models.len()
    ];
    store.load_brushes(
        Vec::new(),
        Vec::new(),
        patches,
        SurfaceTable {
            records,
            sides: Vec::new(),
        },
        tree,
        bounds,
    )
}
fn query() -> TraceQuery<'static> {
    TraceQuery::point(
        Vec3([0.0, 0.0, 5.0]),
        Vec3([0.0, 0.0, -5.0]),
        TraceRules::ARENA,
        qa_world::collision::trace_policy(RuleSetId::Quake3).1,
    )
}

#[test]
fn patch_only_resource_has_native_front_face_box_and_capsule_contacts() {
    let mut store = CollisionStore::new();
    let geometry = load(&mut store, vec![patch(0, 17)], vec![0], false).unwrap();
    let mut scratch = store.scratch();
    let point = store
        .trace_model(geometry, 0, query(), &mut scratch)
        .unwrap();
    assert_eq!(point.fraction, ((5.0 - 0.125) / 10.0) as f32);
    assert_eq!(point.plane.normal, Vec3([0.0, 0.0, 1.0]));
    assert_eq!(point.plane.distance, 0.0);
    assert_eq!(point.plane.type_sign(), [0, 0]);
    assert_eq!(point.end, Vec3([0.0, 0.0, 0.125]));
    assert_eq!(point.contents, Contents::SOLID);
    assert_eq!(point.surface, SurfaceFlags(7));
    assert_eq!(point.surface_id.unwrap().index, 0);
    assert_eq!(
        store
            .surface(point.surface_id.unwrap())
            .unwrap()
            .source_index,
        11
    );
    assert!(!point.start_solid && !point.all_solid);

    let mut asymmetric = query();
    asymmetric.mins = Vec3([-1.0, -1.0, -2.0]);
    asymmetric.maxs = Vec3([1.0, 1.0, 4.0]);
    let trace = store
        .trace_model(geometry, 0, asymmetric, &mut scratch)
        .unwrap();
    assert_eq!(trace.fraction, ((3.0 - 0.125) / 10.0) as f32);
    assert_eq!(trace.end, Vec3([0.0, 0.0, 2.125]));
    assert_eq!(trace.plane.distance, 3.0);

    let mut capsule = query();
    capsule.mins = Vec3([-1.0, -1.0, -2.0]);
    capsule.maxs = Vec3([1.0, 1.0, 2.0]);
    let trace = store
        .trace_model(geometry, 0, capsule.with_capsule(), &mut scratch)
        .unwrap();
    assert_eq!(trace.fraction, ((3.0 - 0.125) / 10.0) as f32);
    assert_eq!(trace.end, Vec3([0.0, 0.0, 2.125]));
    // Segment offset affects distances, while the returned plane expands by
    // radius alone (original cm_patch.c CM_TraceThroughPatchCollide).
    assert_eq!(trace.plane.distance, 1.0);

    let mut backward = query();
    std::mem::swap(&mut backward.start, &mut backward.end);
    assert_eq!(
        store
            .trace_model(geometry, 0, backward, &mut scratch)
            .unwrap()
            .fraction,
        1.0
    );
}

#[test]
fn position_point_contents_masks_and_curve_controls_keep_distinct_native_semantics() {
    let mut store = CollisionStore::new();
    let geometry = load(&mut store, vec![patch(0, 17)], vec![0], false).unwrap();
    let mut scratch = store.scratch();
    let mut stationary = query();
    stationary.start = Vec3::default();
    stationary.end = stationary.start;
    let hit = store
        .trace_model(geometry, 0, stationary, &mut scratch)
        .unwrap();
    assert!(hit.start_solid && hit.all_solid);
    assert_eq!(hit.fraction, 0.0);
    assert_eq!(
        store
            .point_contents_model(geometry, 0, Vec3::default(), stationary.entity_rules)
            .unwrap(),
        Contents::EMPTY
    );
    let mut points_disabled = query();
    points_disabled.rules.patches = PatchClip::Volumes;
    assert_eq!(
        store
            .trace_model(geometry, 0, points_disabled, &mut scratch)
            .unwrap()
            .fraction,
        1.0
    );
    points_disabled.mins = Vec3([-1.0; 3]);
    points_disabled.maxs = Vec3([1.0; 3]);
    assert!(
        store
            .trace_model(geometry, 0, points_disabled, &mut scratch)
            .unwrap()
            .fraction
            < 1.0
    );
    stationary.rules.patches = PatchClip::Volumes;
    assert!(
        store
            .trace_model(geometry, 0, stationary, &mut scratch)
            .unwrap()
            .all_solid
    );
    stationary.rules.patches = PatchClip::Disabled;
    assert!(
        !store
            .trace_model(geometry, 0, stationary, &mut scratch)
            .unwrap()
            .all_solid
    );
    points_disabled.rules.patches = PatchClip::Disabled;
    assert_eq!(
        store
            .trace_model(geometry, 0, points_disabled, &mut scratch)
            .unwrap()
            .fraction,
        1.0
    );
    let mut masked = query();
    masked.mask = Contents::WATER;
    assert_eq!(
        store
            .trace_model(geometry, 0, masked, &mut scratch)
            .unwrap()
            .fraction,
        1.0
    );
}

#[test]
fn world_override_is_explicit_and_inline_membership_is_not_world_membership() {
    let mut store = CollisionStore::new();
    let geometry = load(&mut store, vec![patch(0, 17)], vec![0], true).unwrap();
    let mut scratch = store.scratch();
    assert_eq!(
        store
            .trace_model(geometry, 0, query(), &mut scratch)
            .unwrap()
            .fraction,
        1.0
    );
    assert!(
        store
            .trace_model(geometry, 1, query(), &mut scratch)
            .unwrap()
            .fraction
            < 1.0
    );
    let entities = EntityTable::new(2, 1).unwrap();
    let area = AreaGrid::load(
        2,
        Bounds {
            mins: Vec3([-1024.0; 3]),
            maxs: Vec3([1024.0; 3]),
        },
    )
    .unwrap();
    let mut disabled = query();
    disabled.rules.patches = PatchClip::Disabled;
    let mut trace = WorldTrace::new(
        &store,
        geometry,
        1,
        &entities,
        &area,
        &mut scratch,
        CollisionOwner::None,
    );
    assert_eq!(trace.trace(disabled).unwrap().fraction, 1.0);
    assert!(
        trace
            .with_patch_clip(PatchClip::All)
            .trace(disabled)
            .unwrap()
            .fraction
            < 1.0
    );
}

#[test]
fn duplicate_leaf_patch_references_keep_native_first_surface_contact() {
    let mut store = CollisionStore::new();
    let geometry = load(
        &mut store,
        vec![patch(0, 17), patch(1, 17)],
        vec![1, 0, 1],
        false,
    )
    .unwrap();
    let mut scratch = store.scratch();
    for _ in 0..2 {
        let hit = store
            .trace_model(geometry, 0, query(), &mut scratch)
            .unwrap();
        assert_eq!(hit.fraction, ((5.0 - 0.125) / 10.0) as f32);
        assert_eq!(hit.surface, SurfaceFlags(8));
        assert_eq!(hit.surface_id.unwrap().index, 1);
    }
}

#[test]
fn raised_control_counts_degenerate_grids_and_scratch_admission_are_bounded() {
    let mut store = CollisionStore::new();
    let mut stale = store.scratch();
    // 4225 authored controls exceed original CMod_LoadPatches' 1024 limit.
    let geometry = load(&mut store, vec![patch(0, 65)], vec![0], false).unwrap();
    assert!(matches!(
        store.trace_model(geometry, 0, query(), &mut stale),
        Err(TraceError::Scratch)
    ));
    let mut scratch = store.scratch();
    assert!(
        store
            .trace_model(geometry, 0, query(), &mut scratch)
            .unwrap()
            .fraction
            < 1.0
    );
    let mut degenerate = patch(0, 3);
    degenerate.control_points.fill(Vec3::default());
    let geometry = load(&mut store, vec![degenerate], vec![0], false).unwrap();
    let mut scratch = store.scratch();
    assert_eq!(
        store
            .trace_model(geometry, 0, query(), &mut scratch)
            .unwrap()
            .fraction,
        1.0
    );
}

#[test]
fn invalid_patch_grids_surfaces_and_membership_reject_before_resource_publication() {
    for kind in 0..3 {
        let mut input = patch(0, 3);
        match kind {
            0 => input.dimensions[0] = 4,
            1 => {
                input.control_points.pop();
            }
            _ => input.control_points[0].0[0] = f32::NAN,
        }
        let mut store = CollisionStore::new();
        assert_eq!(
            load(&mut store, vec![input], vec![0], false),
            Err(StoreError::Brush(GeometryError::PatchGrid))
        );
    }
    let mut store = CollisionStore::new();
    assert_eq!(
        load(&mut store, vec![patch(9, 3)], vec![0], false),
        Err(StoreError::Brush(GeometryError::Surface))
    );
    assert_eq!(
        load(&mut store, vec![patch(0, 3)], vec![1], false),
        Err(StoreError::Brush(GeometryError::TreeRange))
    );
}

#[test]
fn subdivided_ridge_retains_native_facet_plane_and_independent_point_epsilon() {
    let mut ridge = patch(0, 3);
    ridge.control_points = [-64.0, 0.0, 64.0]
        .into_iter()
        .flat_map(|y| {
            [-64.0, 0.0, 64.0]
                .into_iter()
                .zip([0.0, 64.0, 0.0])
                .map(move |(x, z)| Vec3([x, y, z]))
        })
        .collect();
    let mut store = CollisionStore::new();
    let geometry = load(&mut store, vec![ridge], vec![0], false).unwrap();
    let mut scratch = store.scratch();
    // Native subdivision creates anchors (-64,0), (0,32), (64,0).
    // The right facet is x+2z=64; this is an analytic source-space oracle.
    let inverse = 1.0 / 5.0f32.sqrt();
    let plane =
        qa_core::primitives::Plane::oriented(Vec3([inverse, 0.0, 2.0 * inverse]), 64.0 * inverse);
    let mut moving = query();
    moving.start = Vec3([16.0, 0.0, 100.0]);
    moving.end = Vec3([16.0, 0.0, -100.0]);
    for (rules, epsilon) in [(TraceRules::ARENA, 0.125), (TraceRules::LEGACY, 0.03125)] {
        moving.rules = rules;
        let first = plane.geometric_distance(moving.start);
        let last = plane.geometric_distance(moving.end);
        let expected = ((f64::from(first) - epsilon) / f64::from(first - last)) as f32;
        let hit = store
            .trace_model(geometry, 0, moving, &mut scratch)
            .unwrap();
        assert_eq!(hit.plane.normal, plane.normal);
        assert_eq!(hit.plane.distance, plane.distance);
        assert_eq!(hit.fraction, expected);
        assert_eq!(hit.end, moving.start.lerp(moving.end, expected));
    }
    moving.rules = TraceRules::ARENA;
    moving.mins = Vec3([-1.0, -1.0, -2.0]);
    moving.maxs = Vec3([1.0, 1.0, 2.0]);
    let capsule = moving.with_capsule();
    let hit = store
        .trace_model(geometry, 0, capsule, &mut scratch)
        .unwrap();
    assert_eq!(hit.plane.normal, plane.normal);
    assert_eq!(hit.plane.distance, plane.distance + 1.0);
    // Expanded plane distance is rounded before subtraction, as cm_patch does.
    let expanded = qa_core::primitives::Plane::oriented(plane.normal, plane.distance + 1.0);
    let first = expanded.geometric_distance(moving.start - Vec3([0.0, 0.0, 1.0]));
    let last = expanded.geometric_distance(moving.end - Vec3([0.0, 0.0, 1.0]));
    assert_eq!(
        hit.fraction,
        ((f64::from(first) - 0.125) / f64::from(first - last)) as f32
    );
}

#[test]
fn brush_and_patch_zero_ids_have_independent_stamps_and_native_tie_order() {
    use qa_core::primitives::Plane;
    for center in [100.0, 0.0] {
        let mut store = CollisionStore::new();
        let planes = vec![
            Plane::oriented(Vec3([1.0, 0.0, 0.0]), center + 8.0),
            Plane::oriented(Vec3([-1.0, 0.0, 0.0]), -center + 8.0),
            Plane::oriented(Vec3([0.0, 1.0, 0.0]), 8.0),
            Plane::oriented(Vec3([0.0, -1.0, 0.0]), 8.0),
            Plane::oriented(Vec3([0.0, 0.0, 1.0]), 0.0),
            Plane::oriented(Vec3([0.0, 0.0, -1.0]), 2.0),
        ];
        let mut tree = BrushTree::direct(1).unwrap();
        tree.leaves[0].patches = IndexSpan { first: 0, count: 1 };
        tree.leaf_patches = vec![0];
        let surfaces = SurfaceTable {
            records: vec![
                SurfaceInput::unnamed(SurfaceFlags(19)),
                SurfaceInput::unnamed(SurfaceFlags(29)),
            ],
            sides: vec![0; 6],
        };
        let geometry = store
            .load_brushes(
                planes,
                vec![Brush {
                    sides: IndexSpan { first: 0, count: 6 },
                    contents: Contents::SOLID,
                }],
                vec![patch(1, 17)],
                surfaces,
                tree,
                vec![Bounds {
                    mins: Vec3([-1024.0; 3]),
                    maxs: Vec3([1024.0; 3]),
                }],
            )
            .unwrap();
        let mut scratch = store.scratch();
        let hit = store
            .trace_model(geometry, 0, query(), &mut scratch)
            .unwrap();
        assert_eq!(hit.fraction, ((5.0 - 0.125) / 10.0) as f32);
        assert_eq!(
            hit.surface,
            SurfaceFlags(if center == 0.0 { 19 } else { 29 })
        );
        assert_eq!(
            hit.plane.type_sign(),
            if center == 0.0 { [2, 0] } else { [0, 0] }
        );
    }
}

#[test]
fn wrapped_patch_keeps_sweeps_on_both_sides_of_its_seam() {
    let ring = [
        (0.0, 4.0),
        (4.0, 4.0),
        (4.0, 0.0),
        (4.0, -4.0),
        (0.0, -4.0),
        (-4.0, -4.0),
        (-4.0, 0.0),
        (-4.0, 4.0),
        (0.0, 4.0),
    ];
    let input = PatchInput {
        dimensions: [9, 3],
        control_points: [-8.0, 0.0, 8.0]
            .into_iter()
            .flat_map(|y| ring.into_iter().map(move |(x, z)| Vec3([x, y, z])))
            .collect(),
        contents: Contents::SOLID,
        surface: 0,
    };
    let mut store = CollisionStore::new();
    let geometry = load(&mut store, vec![input], vec![0], false).unwrap();
    let mut scratch = store.scratch();
    for x in [-0.01, 0.0, 0.01] {
        let mut moving = query();
        moving.start = Vec3([x, 0.0, 10.0]);
        moving.end = Vec3([x, 0.0, 0.0]);
        moving.mins = Vec3([-0.25; 3]);
        moving.maxs = Vec3([0.25; 3]);
        let hit = store
            .trace_model(geometry, 0, moving, &mut scratch)
            .unwrap();
        assert!(hit.fraction > 0.0 && hit.fraction < 1.0);
        assert!(hit.plane.normal.0[2] > 0.0);
        assert_eq!(hit.contents, Contents::SOLID);
    }
}

#[test]
fn supplied_asymmetric_borders_keep_exact_shape_and_native_center_rounding_stays_selected() {
    let mut store = CollisionStore::new();
    let geometry = load(&mut store, vec![patch(0, 17)], vec![0], false).unwrap();
    let mut scratch = store.scratch();
    let mut moving = query();
    moving.rules = TraceRules::LEGACY;
    moving.start.0[0] = 7.5;
    moving.end.0[0] = 7.5;
    moving.mins = Vec3([1.0, -0.5, -0.5]);
    moving.maxs = Vec3([2.0, 0.5, 0.5]);
    // Actual x extent [8.5,9.5] is wholly beyond the patch's x=8 border.
    assert_eq!(
        store
            .trace_model(geometry, 0, moving, &mut scratch)
            .unwrap()
            .fraction,
        1.0
    );
    moving.start.0[0] = 6.5;
    moving.end.0[0] = 6.5;
    let hit = store
        .trace_model(geometry, 0, moving, &mut scratch)
        .unwrap();
    assert_eq!(hit.fraction, ((4.5 - 0.03125) / 10.0) as f32);
    assert_eq!(hit.plane.distance, 0.5);

    let mut position = query();
    position.start = Vec3([f32::from_bits(7.0f32.to_bits() + 1), 0.0, 0.0]);
    position.end = position.start;
    position.mins = Vec3([1.0, -0.5, -0.5]);
    position.maxs = Vec3([f32::from_bits(1.0f32.to_bits() + 1), 0.5, 0.5]);
    // Native sum-before-half centering rounds the x center to exactly 8,
    // retaining unequal centered x bounds [0,one ulp]. Supplied rules do not.
    assert!(
        store
            .trace_model(geometry, 0, position, &mut scratch)
            .unwrap()
            .all_solid
    );
    position.rules = TraceRules::LEGACY;
    assert!(
        !store
            .trace_model(geometry, 0, position, &mut scratch)
            .unwrap()
            .all_solid
    );
}
