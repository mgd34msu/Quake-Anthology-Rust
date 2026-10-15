use super::collision;
use qa_core::primitives::{Axis, Bounds, Plane, SurfaceFlags, Vec3};
use qa_formats::bsp::{Brush, BrushSide, IndexSpan, Map, Node, Shader};
use qa_world::collision::{CollisionStore, Contents, TraceQuery, TraceRules};

fn put(bytes: &mut [u8], at: usize, value: i32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn empty_lumps(version: u32) -> Vec<Vec<u8>> {
    let q2 = version == 38;
    let early = version == 44;
    let mut lumps = vec![
        Vec::new();
        if q2 {
            19
        } else if early {
            15
        } else if version == 47 {
            18
        } else {
            17
        }
    ];
    let (plane, node, leaf, model) = if q2 {
        (1, 4, 8, 13)
    } else if early {
        (1, 2, 3, 6)
    } else {
        (2, 3, 4, 7)
    };
    lumps[plane] = vec![0; if q2 || early { 20 } else { 16 }];
    put(&mut lumps[plane], 0, 1.0f32.to_bits() as i32);
    lumps[node] = vec![0; if q2 { 28 } else { 36 }];
    put(&mut lumps[node], 4, -1);
    put(&mut lumps[node], 8, -1);
    lumps[leaf] = vec![0; if q2 { 28 } else { 48 }];
    if q2 {
        put(&mut lumps[leaf], 0, 1);
        lumps[leaf][4..6].copy_from_slice(&u16::MAX.to_le_bytes());
        lumps[17] = vec![0; 8];
    } else {
        put(&mut lumps[leaf], 0, -1);
        put(&mut lumps[leaf], 4, -1);
    }
    lumps[model] = vec![0; if q2 || early { 48 } else { 40 }];
    lumps
}

fn map_bytes(version: u32, lumps: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = vec![0; 8 + lumps.len() * 8];
    bytes[..4].copy_from_slice(b"IBSP");
    bytes[4..8].copy_from_slice(&version.to_le_bytes());
    for (id, lump) in lumps.iter().enumerate() {
        let offset = bytes.len() as u32;
        bytes[8 + id * 8..12 + id * 8].copy_from_slice(&offset.to_le_bytes());
        bytes[12 + id * 8..16 + id * 8].copy_from_slice(&(lump.len() as u32).to_le_bytes());
        bytes.extend(lump);
    }
    bytes
}

pub(crate) fn empty_map_bytes(version: u32) -> Vec<u8> {
    map_bytes(version, &empty_lumps(version))
}

fn add_box(map: &mut Map<'_>, center: f32) {
    let first = map.brush_sides.len() as u32;
    for axis in 0..3 {
        for positive in [false, true] {
            let mut normal = Vec3::default();
            normal.0[axis] = if positive { 1.0 } else { -1.0 };
            let plane = map.planes.len() as u32;
            map.planes.push(Plane {
                encoding: None,
                normal,
                distance: 10.0 + if axis == 0 { center * normal.0[0] } else { 0.0 },
                axis: if positive {
                    Axis::from_index(axis)
                } else {
                    None
                },
            });
            map.brush_sides.push(BrushSide {
                plane,
                texture_info: None,
                shader: None,
                flags: 0,
            });
        }
    }
    map.brushes.push(Brush {
        sides: IndexSpan { first, count: 6 },
        contents: 1,
        shader: None,
    });
}

pub(crate) fn two_models(map: &mut Map<'_>) {
    add_box(map, 0.0);
    add_box(map, 100.0);
    map.leaf_brushes = vec![0, 1];
    map.leaves[0].brushes = IndexSpan { first: 0, count: 1 };
    map.leaves[0].contents = 1;
    let mut inline_leaf = map.leaves[0];
    inline_leaf.brushes.first = 1;
    map.leaves.push(inline_leaf);
    map.nodes = vec![
        Node {
            plane: 0,
            children: [-1, -1],
            bounds: Bounds::default(),
            faces: IndexSpan::default(),
        },
        Node {
            plane: 0,
            children: [-2, -2],
            bounds: Bounds::default(),
            faces: IndexSpan::default(),
        },
    ];
    map.models[0].brushes = IndexSpan { first: 0, count: 1 };
    let mut inline = map.models[0];
    if map.bsp.format == qa_formats::bsp::BspFormat::Quake3 {
        // IBSP46 has no inline headnode. Only its brush range identifies it;
        // an incorrect root-based conversion would trace the world instead.
        map.nodes.truncate(1);
        map.leaves.truncate(1);
        map.leaf_brushes.truncate(1);
    } else {
        inline.headnodes[0] = 1;
    }
    inline.brushes.first = 1;
    map.models.push(inline);
}

#[test]
fn cold_conversion_keeps_models_separate_and_native_brush_numbers() {
    for version in [38, 44, 46] {
        let bytes = empty_map_bytes(version);
        let mut map = Map::parse(&bytes).unwrap();
        two_models(&mut map);
        let mut store = CollisionStore::new();
        let (geometry, count) = collision(&map, &mut store).unwrap();
        assert_eq!(count, 2);
        let mut scratch = store.scratch();
        for (rules, entity_rules) in [
            (
                TraceRules::LEGACY,
                qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake2).1,
            ),
            (
                TraceRules::ARENA,
                qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake3).1,
            ),
        ] {
            let query = TraceQuery::point(
                Vec3([140.0, 0.0, 0.0]),
                Vec3([100.0, 0.0, 0.0]),
                rules,
                entity_rules,
            );
            let unplaced = store.trace_model(geometry, 0, query, &mut scratch).unwrap();
            assert_eq!(unplaced.fraction, 1.0);
            let inline = store.trace_model(geometry, 1, query, &mut scratch).unwrap();
            assert!(inline.fraction < 1.0);
            assert_eq!(inline.plane.normal, Vec3([1.0, 0.0, 0.0]));
            let world_query =
                TraceQuery::point(Vec3([40.0, 0.0, 0.0]), Vec3::default(), rules, entity_rules);
            assert!(
                store
                    .trace_model(geometry, 0, world_query, &mut scratch)
                    .unwrap()
                    .fraction
                    < 1.0
            );
            assert_eq!(
                store
                    .trace_model(geometry, 1, world_query, &mut scratch)
                    .unwrap()
                    .fraction,
                1.0
            );
        }
    }
}

#[test]
fn stored_q2_leaf_contents_survive_and_the_caller_selects_their_use() {
    let bytes = empty_map_bytes(38);
    let mut map = Map::parse(&bytes).unwrap();
    two_models(&mut map);
    map.leaves[0].contents = 32;
    let mut store = CollisionStore::new();
    let (geometry, _) = collision(&map, &mut store).unwrap();
    assert_eq!(
        store
            .point_contents_model(
                geometry,
                0,
                Vec3([40.0, 0.0, 0.0]),
                qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake2).1
            )
            .unwrap(),
        Contents::WATER
    );
    assert_eq!(
        store
            .point_contents_model(
                geometry,
                0,
                Vec3([40.0, 0.0, 0.0]),
                qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake3).1
            )
            .unwrap(),
        Contents::EMPTY
    );
    assert_eq!(
        store
            .point_contents_model(
                geometry,
                0,
                Vec3::default(),
                qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake3).1
            )
            .unwrap(),
        Contents::SOLID
    );
    let mut scratch = store.scratch();
    let q2 = TraceQuery::point(
        Vec3([40.0, 0.0, 0.0]),
        Vec3::default(),
        TraceRules::LEGACY,
        qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake2).1,
    );
    assert_eq!(
        store
            .trace_model(geometry, 0, q2, &mut scratch)
            .unwrap()
            .fraction,
        1.0
    );
    let q3 = TraceQuery {
        rules: TraceRules::ARENA,
        entity_rules: qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake3).1,
        ..q2
    };
    assert!(
        store
            .trace_model(geometry, 0, q3, &mut scratch)
            .unwrap()
            .fraction
            < 1.0
    );
}

#[test]
fn stored_q2_aux_leaf_contents_survive_cold_conversion() {
    let bytes = empty_map_bytes(38);
    let mut map = Map::parse(&bytes).unwrap();
    two_models(&mut map);
    map.leaves[0].contents = 4 | 32;
    let mut store = CollisionStore::new();
    let (geometry, _) = collision(&map, &mut store).unwrap();
    assert_eq!(
        store
            .point_contents_model(
                geometry,
                0,
                Vec3::default(),
                qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake2).1
            )
            .unwrap(),
        Contents::AUX | Contents::WATER
    );
    assert_eq!(
        store
            .point_contents_model(
                geometry,
                0,
                Vec3::default(),
                qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake3).1
            )
            .unwrap(),
        Contents::SOLID
    );
}

#[test]
fn q3_trace_contacts_convert_early_side_flags_and_modern_shader_flags() {
    for version in [44, 46] {
        let bytes = empty_map_bytes(version);
        let mut map = Map::parse(&bytes).unwrap();
        two_models(&mut map);
        map.shaders.push(Shader {
            name: b"test_slick",
            surface_flags: 2,
            content_flags: 1,
        });
        // Native SURF_SLICK is 2. The unused representation is deliberately
        // different so selecting flags from the wrong layout is observable.
        for side in &mut map.brush_sides {
            side.flags = if version == 44 { 2 } else { 0 };
            side.shader = if version == 44 { None } else { Some(0) };
        }
        let mut store = CollisionStore::new();
        let (geometry, _) = collision(&map, &mut store).unwrap();
        let mut scratch = store.scratch();
        let query = TraceQuery::point(
            Vec3([40.0, 0.0, 0.0]),
            Vec3::default(),
            TraceRules::ARENA,
            qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake3).1,
        );
        assert_eq!(
            store
                .trace_model(geometry, 0, query, &mut scratch)
                .unwrap()
                .surface,
            SurfaceFlags::SLICK
        );
        let id = store
            .trace_model(geometry, 0, query, &mut scratch)
            .unwrap()
            .surface_id
            .unwrap();
        let row = store.surface(id).unwrap();
        assert_eq!(row.flags.to_q3(), 2);
        assert_eq!(
            row.name,
            if version == 44 {
                &b""[..]
            } else {
                &b"test_slick"[..]
            }
        );
    }
}

#[test]
fn q2_texinfo_contacts_keep_full_native_flags_name_value_and_index() {
    let bytes = empty_map_bytes(38);
    let mut map = Map::parse(&bytes).unwrap();
    two_models(&mut map);
    map.texture_info.push(qa_formats::bsp::TextureInfo {
        projection: [[0.0; 4]; 2],
        flags: 0x80000083u32 as i32,
        texture: 0,
        value: 96,
        next: -1,
        name: b"e1u1/Light_A",
    });
    for side in &mut map.brush_sides {
        side.texture_info = Some(0);
    }
    let mut store = CollisionStore::new();
    let (geometry, _) = collision(&map, &mut store).unwrap();
    let mut scratch = store.scratch();
    let (rules, entities) =
        qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake2);
    let hit = store
        .trace_model(
            geometry,
            0,
            TraceQuery::point(Vec3([40.0, 0.0, 0.0]), Vec3::default(), rules, entities),
            &mut scratch,
        )
        .unwrap();
    let id = hit.surface_id.unwrap();
    assert_eq!(id.index, 1);
    let row = store.surface(id).unwrap();
    assert_eq!(row.name, b"e1u1/Light_A");
    assert_eq!(row.value, 96);
    assert_eq!(row.source_index, 1);
    assert_eq!(row.flags.to_q2(), 0x80000083);
    assert_eq!(store.surface_count(geometry), Some(2));
}

fn patch_vertices(points: &[[f32; 3]]) -> Vec<u8> {
    let mut output = vec![0; points.len() * 44];
    for (index, point) in points.iter().enumerate() {
        for (axis, value) in point.iter().enumerate() {
            put(&mut output, index * 44 + axis * 4, value.to_bits() as i32);
        }
        put(&mut output, index * 44 + 36, 1.0f32.to_bits() as i32);
        output[index * 44 + 40..index * 44 + 44].fill(255);
    }
    output
}

fn patch_surface(shader: i32, first: i32, kind: i32) -> Vec<u8> {
    let mut output = vec![0; 104];
    for (offset, value) in [(0, shader), (4, -1), (8, kind), (12, first), (28, -1)] {
        put(&mut output, offset, value);
    }
    if kind == 2 {
        for (offset, value) in [(16, 9), (96, 3), (100, 3)] {
            put(&mut output, offset, value);
        }
    }
    output
}

fn parsed_patch_bytes(version: u32) -> Vec<u8> {
    let mut lumps = empty_lumps(version);
    let mut shaders = vec![0; 72 * 3];
    for (index, (name, flags, contents)) in [
        (&b"test/curve_first"[..], 2, 1),
        (&b"test/curve_leaf_first"[..], 0x80004002u32 as i32, 1),
        (&b"test/curve_inline"[..], 8, 0x02000000),
    ]
    .into_iter()
    .enumerate()
    {
        shaders[index * 72..index * 72 + name.len()].copy_from_slice(name);
        put(&mut shaders, index * 72 + 64, flags);
        put(&mut shaders, index * 72 + 68, contents);
    }
    lumps[1] = shaders;
    for (offset, value) in [
        (12, -512),
        (16, -512),
        (20, -512),
        (24, 512),
        (28, 512),
        (32, 512),
    ] {
        put(&mut lumps[3], offset, value);
    }
    // A sweep starts in leaf0; equal hits must retain its authored surface2,
    // ignoring flare1 and duplicate refs rather than sorting by patch ID.
    put(&mut lumps[3], 8, -2);
    lumps[4].resize(96, 0);
    for leaf in 0..2 {
        for (offset, value) in [
            (0, -1),
            (4, -1),
            (8, -512),
            (12, -512),
            (16, -512),
            (20, 512),
            (24, 512),
            (28, 512),
        ] {
            put(&mut lumps[4], leaf * 48 + offset, value);
        }
        put(&mut lumps[4], leaf * 48 + 32, if leaf == 0 { 0 } else { 4 });
        put(&mut lumps[4], leaf * 48 + 36, if leaf == 0 { 4 } else { 1 });
    }
    lumps[5] = [2i32, 1, 0, 2, 0]
        .into_iter()
        .flat_map(i32::to_le_bytes)
        .collect();
    lumps[7].resize(80, 0);
    for (index, (mins, maxs, first, count)) in [
        ([-512.0f32; 3], [512.0f32; 3], 0, 3),
        ([136.0, -64.0, 0.0], [264.0, 64.0, 0.0], 3, 1),
    ]
    .into_iter()
    .enumerate()
    {
        for (axis, value) in mins.into_iter().chain(maxs).enumerate() {
            put(&mut lumps[7], index * 40 + axis * 4, value.to_bits() as i32);
        }
        put(&mut lumps[7], index * 40 + 24, first);
        put(&mut lumps[7], index * 40 + 28, count);
    }
    let mut points = Vec::new();
    for y in [64.0, 0.0, -64.0] {
        for (x, z) in [(-64.0, 0.0), (0.0, 64.0), (64.0, 0.0)] {
            points.push([x, y, z]);
        }
    }
    for y in [64.0, 0.0, -64.0] {
        for x in [136.0, 200.0, 264.0] {
            points.push([x, y, 0.0]);
        }
    }
    lumps[10] = patch_vertices(&points);
    lumps[13] = [
        patch_surface(0, 0, 2),
        patch_surface(0, 0, 4),
        patch_surface(1, 0, 2),
        patch_surface(2, 9, 2),
    ]
    .concat();
    map_bytes(version, &lumps)
}

fn patch_query(x: f32) -> TraceQuery<'static> {
    let (rules, entities) =
        qa_world::collision::trace_policy(qa_core::primitives::RuleSetId::Quake3);
    TraceQuery::point(Vec3([x, 0.0, 96.0]), Vec3([x, 0.0, -32.0]), rules, entities)
}

fn near(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 0.0001, "{actual} != {expected}");
}

#[test]
fn parsed_q3_patch_only_world_retains_authored_membership_metadata_and_exact_shape() {
    for version in [46, 47] {
        let bytes = parsed_patch_bytes(version);
        let map = Map::parse(&bytes).unwrap();
        assert!(map.brushes.is_empty());
        assert_eq!(map.leaf_faces, [2, 1, 0, 2, 0]);
        let mut store = CollisionStore::new();
        let (geometry, brushes) = collision(&map, &mut store).unwrap();
        assert_eq!(brushes, 0);
        assert_eq!(store.model_count(geometry), Some(2));
        let mut scratch = store.scratch();
        let normal = Vec3([1.0 / 5.0f32.sqrt(), 0.0, 2.0 / 5.0f32.sqrt()]);
        let plane = 64.0 / 5.0f32.sqrt();
        for (mins, maxs, capsule, support, reported_support) in [
            (Vec3::default(), Vec3::default(), false, 0.0, 0.0),
            (
                Vec3([-2.0; 3]),
                Vec3([2.0; 3]),
                false,
                6.0 / 5.0f32.sqrt(),
                6.0 / 5.0f32.sqrt(),
            ),
            (
                Vec3([-1.0, -2.0, -3.0]),
                Vec3([5.0, 2.0, 7.0]),
                false,
                7.0 / 5.0f32.sqrt(),
                13.0 / 5.0f32.sqrt(),
            ),
            (
                Vec3([-2.0, -2.0, -6.0]),
                Vec3([2.0, 2.0, 6.0]),
                true,
                2.0 + 4.0 * normal.0[2],
                2.0,
            ),
        ] {
            let mut query = TraceQuery {
                mins,
                maxs,
                ..patch_query(16.0)
            };
            if capsule {
                query = query.with_capsule();
            }
            let hit = store.trace_model(geometry, 0, query, &mut scratch).unwrap();
            let expected_z = 24.0 + (support + 0.125) / normal.0[2];
            near(hit.fraction, (96.0 - expected_z) / 128.0);
            near(hit.end.0[2], expected_z);
            near(hit.end.0[0], 16.0);
            for axis in 0..3 {
                near(hit.plane.normal.0[axis], normal.0[axis]);
            }
            near(hit.plane.distance, plane + reported_support);
            assert!(!hit.start_solid && !hit.all_solid);
            assert_eq!(hit.contents, Contents::SOLID);
            let id = hit.surface_id.unwrap();
            assert_eq!(id.geometry, geometry);
            assert_eq!(id.index, 2);
            let row = store.surface(id).unwrap();
            assert_eq!(row.name, b"test/curve_leaf_first");
            assert_eq!(row.flags.to_q3(), 0x80004002);
            assert_eq!(row.source_index, 2);
        }
        let point = Vec3([16.0, 0.0, 24.0]);
        let stationary = TraceQuery {
            start: point,
            end: point,
            mins: Vec3([-1.0; 3]),
            maxs: Vec3([1.0; 3]),
            ..patch_query(16.0)
        };
        let overlap = store
            .trace_model(geometry, 0, stationary, &mut scratch)
            .unwrap();
        assert!(overlap.start_solid && overlap.all_solid);
        assert_eq!(overlap.fraction, 0.0);
        assert_eq!(
            store
                .point_contents_model(geometry, 0, point, stationary.entity_rules)
                .unwrap(),
            Contents::EMPTY
        );
    }
}

#[test]
fn parsed_q3_patch_only_inline_uses_native_model_membership_and_shared_transform() {
    use qa_core::primitives::{Body, CollisionShape, ModuleId, RuleSetId, ThinkTime};
    use qa_world::{
        area::{AreaGrid, LinkFlags, LinkIntent, LinkOrder},
        collision::{WorldTrace, store::BodyTraceMode},
        entities::{AllocationPolicy, EntityTable},
    };
    for version in [46, 47] {
        let bytes = parsed_patch_bytes(version);
        let map = Map::parse(&bytes).unwrap();
        let mut store = CollisionStore::new();
        let (geometry, _) = collision(&map, &mut store).unwrap();
        let mut scratch = store.scratch();
        let mut query = patch_query(200.0);
        query.mask = Contents::BODY;
        assert_eq!(
            store
                .trace_model(geometry, 0, query, &mut scratch)
                .unwrap()
                .fraction,
            1.0
        );
        let inline = store.trace_model(geometry, 1, query, &mut scratch).unwrap();
        near(inline.end.0[2], 0.125);
        assert_eq!(inline.plane.normal, Vec3([0.0, 0.0, 1.0]));
        assert_eq!(inline.contents, Contents::BODY);
        assert_eq!(
            store.surface(inline.surface_id.unwrap()).unwrap().name,
            b"test/curve_inline"
        );
        assert_eq!(
            store.model_bounds(geometry, 1),
            Some(Bounds {
                mins: Vec3([135.0, -65.0, -1.0]),
                maxs: Vec3([265.0, 65.0, 1.0])
            })
        );

        // A separate geometry with its own inline1 must never select this owner's patch.
        let other_bytes = empty_map_bytes(version);
        let mut other = Map::parse(&other_bytes).unwrap();
        other.models.push(other.models[0]);
        let (query_geometry, _) = collision(&other, &mut store).unwrap();
        let shape = CollisionShape::Model { geometry, index: 1 };
        let rules = qa_gameplay::rules::model_rules(RuleSetId::Quake3);
        let bounds = store.model_bounds(geometry, 1).unwrap();
        let body = Body {
            position: Vec3([10.0, 20.0, 30.0]),
            mins: bounds.mins,
            maxs: bounds.maxs,
            ..Body::default()
        };
        let angles = Vec3([0.0, 90.0, 0.0]);
        query.start = Vec3([10.0, 220.0, 126.0]);
        query.end = Vec3([10.0, 220.0, -2.0]);
        let transformed = store
            .trace_body(
                query,
                shape,
                &body,
                angles,
                rules,
                BodyTraceMode::Transformed,
                None,
                &mut store.scratch(),
            )
            .unwrap();
        near(transformed.end.0[2], 30.125);
        assert_eq!(transformed.surface_id.unwrap().geometry, geometry);

        let mut entities = EntityTable::new(8, 0).unwrap();
        let entity = entities
            .allocate(
                ThinkTime::Milliseconds(0),
                ModuleId(1),
                AllocationPolicy::q3(0),
            )
            .unwrap()
            .id;
        entities.columns.set_body(entity.slot as usize, body);
        entities.columns.angles[entity.slot as usize] = angles;
        entities.columns.collision_shape[entity.slot as usize] = shape;
        entities.columns.model_rules[entity.slot as usize] = rules;
        entities.columns.collision_contents[entity.slot as usize] = Contents::BODY.0;
        let mut area = AreaGrid::load(
            8,
            Bounds {
                mins: Vec3([-512.0; 3]),
                maxs: Vec3([512.0; 3]),
            },
        )
        .unwrap();
        assert!(area.link(
            &entities,
            entity,
            LinkFlags::SOLID,
            LinkOrder::Head,
            LinkIntent::Explicit,
        ));
        let mut scratch = store.scratch();
        let linked = WorldTrace::new(
            &store,
            query_geometry,
            0,
            &entities,
            &area,
            &mut scratch,
            Default::default(),
        )
        .trace(query)
        .unwrap();
        near(linked.end.0[2], 30.125);
        assert_eq!(linked.entity, Some(entity));
        assert_eq!(linked.surface_id.unwrap().geometry, geometry);
        assert_eq!(
            entities.columns.collision_shape[entity.slot as usize],
            shape
        );
    }
}

fn parsed_early_patch_bytes(associated: bool) -> Vec<u8> {
    let mut lumps = empty_lumps(44);
    for axis in 0..6 {
        put(
            &mut lumps[6],
            axis * 4,
            if axis < 3 { -512.0f32 } else { 512.0f32 }.to_bits() as i32,
        );
    }
    put(&mut lumps[6], 44, 1);
    put(&mut lumps[3], 36, 1);
    lumps[4] = 0i32.to_le_bytes().to_vec();
    let points: Vec<_> = [64.0, 0.0, -64.0]
        .into_iter()
        .flat_map(|y| [-64.0, 0.0, 64.0].map(|x| [x, y, 0.0]))
        .collect();
    lumps[11] = patch_vertices(&points);
    let mut surface = vec![0; 164];
    surface[..16].copy_from_slice(b"test/early_curve");
    for (at, value) in [
        (64, -1),
        (68, if associated { 0 } else { -1 }),
        (76, 9),
        (88, 3),
        (92, 3),
        (96, -1),
    ] {
        put(&mut surface, at, value);
    }
    lumps[12] = surface;
    if associated {
        // The brush supplies IBSP44's explicit contents, but is not a member
        // of the queried world leaf. Only the patch can produce this contact.
        lumps[1].resize(7 * 20, 0);
        lumps[8] = vec![0; 6 * 8];
        for side in 0..6 {
            let normal = if side % 2 == 0 { 1.0f32 } else { -1.0f32 };
            put(
                &mut lumps[1],
                (side + 1) * 20 + side / 2 * 4,
                normal.to_bits() as i32,
            );
            put(
                &mut lumps[1],
                (side + 1) * 20 + 12,
                1024.0f32.to_bits() as i32,
            );
            put(
                &mut lumps[1],
                (side + 1) * 20 + 16,
                if side % 2 == 0 { (side / 2) as i32 } else { 3 },
            );
            put(&mut lumps[8], side * 8, side as i32 + 1);
            put(&mut lumps[8], side * 8 + 4, 0x4002);
        }
        lumps[7] = [0i32, 6, 32]
            .into_iter()
            .flat_map(i32::to_le_bytes)
            .collect();
    }
    map_bytes(44, &lumps)
}

#[test]
fn parsed_q3_test_patch_uses_explicit_side_metadata_and_never_invents_solid_contents() {
    for associated in [true, false] {
        let bytes = parsed_early_patch_bytes(associated);
        let map = Map::parse(&bytes).unwrap();
        assert!(map.shaders.is_empty());
        assert_eq!(map.surfaces[0].shader, None);
        assert!(map.leaf_brushes.is_empty());
        let mut store = CollisionStore::new();
        let (geometry, brushes) = collision(&map, &mut store).unwrap();
        assert_eq!(brushes, if associated { 1 } else { 0 });
        let query = TraceQuery {
            mask: Contents::WATER | Contents::SOLID,
            ..patch_query(16.0)
        };
        let hit = store
            .trace_model(geometry, 0, query, &mut store.scratch())
            .unwrap();
        if associated {
            near(hit.end.0[2], 0.125);
            assert_eq!(hit.contents, Contents::WATER);
            assert_eq!(hit.surface.to_q3(), 0x4002);
            let row = store.surface(hit.surface_id.unwrap()).unwrap();
            assert_eq!(row.name, b"test/early_curve");
            assert_eq!(row.source_index, 0);
        } else {
            assert_eq!(hit.fraction, 1.0);
            assert_eq!(hit.surface_id, None);
        }
    }
}
