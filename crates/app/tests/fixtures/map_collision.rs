use super::collision;
use qa_core::primitives::{Axis, Bounds, Plane, SurfaceFlags, Vec3};
use qa_formats::bsp::{Brush, BrushSide, IndexRange, Map, Node, Shader};
use qa_world::collision::{CollisionStore, Contents, EntityTraceRules, TraceQuery, TraceRules};

fn put(bytes: &mut [u8], at: usize, value: i32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn empty_map_bytes(version: u32) -> Vec<u8> {
    let q2 = version == 38;
    let early = version == 44;
    let mut lumps = vec![
        Vec::new();
        if q2 {
            19
        } else if early {
            15
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

fn add_box(map: &mut Map<'_>, center: f32) {
    let first = map.brush_sides.len() as u32;
    for axis in 0..3 {
        for positive in [false, true] {
            let mut normal = Vec3::default();
            normal.0[axis] = if positive { 1.0 } else { -1.0 };
            let plane = map.planes.len() as u32;
            map.planes.push(Plane {
                normal,
                distance: 10.0 + if axis == 0 { center * normal.0[0] } else { 0.0 },
                axis: positive.then_some([Axis::X, Axis::Y, Axis::Z][axis]),
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
        sides: IndexRange { first, count: 6 },
        contents: 1,
        shader: None,
    });
}

fn two_models(map: &mut Map<'_>) {
    add_box(map, 0.0);
    add_box(map, 100.0);
    map.leaf_brushes = vec![0, 1];
    map.leaves[0].brushes = IndexRange { first: 0, count: 1 };
    map.leaves[0].contents = 1;
    let mut inline_leaf = map.leaves[0];
    inline_leaf.brushes.first = 1;
    map.leaves.push(inline_leaf);
    map.nodes = vec![
        Node {
            plane: 0,
            children: [-1, -1],
            bounds: Bounds::default(),
            faces: IndexRange::default(),
        },
        Node {
            plane: 0,
            children: [-2, -2],
            bounds: Bounds::default(),
            faces: IndexRange::default(),
        },
    ];
    map.models[0].brushes = IndexRange { first: 0, count: 1 };
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
            (TraceRules::LEGACY, EntityTraceRules::QUAKE2),
            (TraceRules::ARENA, EntityTraceRules::ARENA),
        ] {
            let query = TraceQuery::point(
                Vec3([140.0, 0.0, 0.0]),
                Vec3([100.0, 0.0, 0.0]),
                rules,
                entity_rules,
            );
            let unplaced = store.trace_model(geometry, 0, query, &mut scratch);
            assert_eq!(unplaced.fraction, 1.0);
            let inline = store.trace_model(geometry, 1, query, &mut scratch);
            assert!(inline.fraction < 1.0);
            assert_eq!(inline.plane.normal, Vec3([1.0, 0.0, 0.0]));
            let world_query =
                TraceQuery::point(Vec3([40.0, 0.0, 0.0]), Vec3::default(), rules, entity_rules);
            assert!(
                store
                    .trace_model(geometry, 0, world_query, &mut scratch)
                    .fraction
                    < 1.0
            );
            assert_eq!(
                store
                    .trace_model(geometry, 1, world_query, &mut scratch)
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
        store.point_contents_model(
            geometry,
            0,
            Vec3([40.0, 0.0, 0.0]),
            EntityTraceRules::QUAKE2
        ),
        Contents::WATER
    );
    assert_eq!(
        store.point_contents_model(geometry, 0, Vec3([40.0, 0.0, 0.0]), EntityTraceRules::ARENA),
        Contents::EMPTY
    );
    assert_eq!(
        store.point_contents_model(geometry, 0, Vec3::default(), EntityTraceRules::ARENA),
        Contents::SOLID
    );
    let mut scratch = store.scratch();
    let q2 = TraceQuery::point(
        Vec3([40.0, 0.0, 0.0]),
        Vec3::default(),
        TraceRules::LEGACY,
        EntityTraceRules::QUAKE2,
    );
    assert_eq!(
        store.trace_model(geometry, 0, q2, &mut scratch).fraction,
        1.0
    );
    let q3 = TraceQuery {
        rules: TraceRules::ARENA,
        entity_rules: EntityTraceRules::ARENA,
        ..q2
    };
    assert!(store.trace_model(geometry, 0, q3, &mut scratch).fraction < 1.0);
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
        store.point_contents_model(geometry, 0, Vec3::default(), EntityTraceRules::QUAKE2),
        Contents::AUX | Contents::WATER
    );
    assert_eq!(
        store.point_contents_model(geometry, 0, Vec3::default(), EntityTraceRules::ARENA),
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
            EntityTraceRules::ARENA,
        );
        assert_eq!(
            store.trace_model(geometry, 0, query, &mut scratch).surface,
            SurfaceFlags::SLICK
        );
    }
}
