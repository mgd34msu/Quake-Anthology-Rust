//! Cold retail admission and supplied-box queries through the production loader.
//! This is neither a gameplay run nor a timing/allocation qualification.
use qa_content::vfs::Vfs;
use qa_core::primitives::{GeometryId, Plane, RuleSetId, Vec3};
use qa_formats::{
    archive::ArchiveReader,
    bsp::Map,
    entities::{EntityLump, EntitySyntax},
};
use qa_world::collision::{
    CollisionStore, HullPositionLimits, HullShape, Trace, TraceQuery, TraceRules, trace_policy,
};
use std::path::PathBuf;

fn player_start(map: &Map<'_>) -> Vec3 {
    let entities =
        EntityLump::parse(map.entity_text(), EntitySyntax::Quake).expect("native entities");
    let classname = entities.names.find(b"classname").expect("classname key");
    let origin = entities.names.find(b"origin").expect("origin key");
    let record = entities
        .records
        .iter()
        .find(|record| {
            entities.fields[(*record).clone()]
                .iter()
                .any(|field| field.key == classname && field.value == b"info_player_start")
        })
        .expect("retail player start");
    let text = entities.fields[record.clone()]
        .iter()
        .find(|field| field.key == origin)
        .expect("player start origin")
        .value;
    let mut components = std::str::from_utf8(text)
        .expect("spawn vector text")
        .split_ascii_whitespace();
    let result = Vec3(std::array::from_fn(|_| {
        components
            .next()
            .expect("spawn vector component")
            .parse::<f32>()
            .expect("spawn vector number")
    }));
    assert!(components.next().is_none() && result.is_finite());
    // Native PutClientInServer raises the authored Quake spawn by one unit.
    result + Vec3([0.0, 0.0, 1.0])
}

fn plane_bits(
    plane: Plane,
) -> (
    [u32; 3],
    u32,
    Option<qa_core::primitives::Axis>,
    Option<[u8; 2]>,
) {
    (
        plane.normal.0.map(f32::to_bits),
        plane.distance.to_bits(),
        plane.axis,
        plane.encoding,
    )
}

fn assert_native_bits(before: Trace, after: Trace) {
    assert_eq!(before.fraction.to_bits(), after.fraction.to_bits());
    assert_eq!(
        before.end.0.map(f32::to_bits),
        after.end.0.map(f32::to_bits)
    );
    assert_eq!(plane_bits(before.plane), plane_bits(after.plane));
    assert_eq!(
        before.secondary_plane.map(plane_bits),
        after.secondary_plane.map(plane_bits)
    );
    assert_eq!(
        (
            before.start_solid,
            before.all_solid,
            before.in_open,
            before.in_water,
            before.brush_solid
        ),
        (
            after.start_solid,
            after.all_solid,
            after.in_open,
            after.in_water,
            after.brush_solid
        )
    );
    assert_eq!(before.contents, after.contents);
    assert_eq!(before.entity, after.entity);
    assert_eq!(before.surface, after.surface);
    assert_eq!(before.surface_id, after.surface_id);
    assert_eq!(before.secondary_surface_id, after.secondary_surface_id);
}

fn exact_query(start: Vec3, end: Vec3, mins: Vec3, maxs: Vec3) -> TraceQuery<'static> {
    TraceQuery {
        mins,
        maxs,
        rules: TraceRules {
            hull_shape: HullShape::ExactBox,
            ..TraceRules::ARENA
        },
        ..TraceQuery::point(
            start,
            end,
            TraceRules::ARENA,
            trace_policy(RuleSetId::Quake3).1,
        )
    }
}

fn run_exact(
    store: &CollisionStore,
    geometry: GeometryId,
    phase: &str,
    query: TraceQuery,
    limits: HullPositionLimits,
) -> Trace {
    println!(
        "e1m1 exact phase={phase} begin start={:?} end={:?} mins={:?} maxs={:?}",
        query.start, query.end, query.mins, query.maxs
    );
    match store.trace_model(geometry, 0, query, &mut store.scratch()) {
        Ok(trace) => {
            println!(
                "e1m1 exact {phase}: fraction={} start_solid={} all_solid={} end={:?}",
                trace.fraction, trace.start_solid, trace.all_solid, trace.end
            );
            trace
        }
        Err(error) => panic!(
            "e1m1 exact phase={phase} error={error:?} query_work={} contact_rows={}; \
             public query errors do not distinguish work/contact/arithmetic/input rejection",
            limits.query_work, limits.contact_rows
        ),
    }
}

#[test]
#[ignore = "requires QA_RETAIL_ROOT; cold exact admission only"]
fn e1m1_default_exact_admission_and_supplied_boxes_preserve_native_queries() {
    let root = PathBuf::from(std::env::var_os("QA_RETAIL_ROOT").expect("QA_RETAIL_ROOT"));
    let mut vfs = Vfs::default();
    vfs.mount_product(&root.join("q1/id1"), 0)
        .expect("retail Quake product");
    let file = vfs.open(b"maps/e1m1.bsp").expect("retail e1m1");
    let bytes = vfs
        .read_to_vec(file, 64 * 1024 * 1024, &mut ArchiveReader::default())
        .expect("bounded retail bytes");
    let map = Map::parse(&bytes).expect("retail BSP");
    assert_eq!(map.bsp.format.rule_set(), RuleSetId::Quake);
    let start = player_start(&map);
    let end = start - Vec3([0.0, 0.0, 128.0]);
    let mut store = CollisionStore::new();
    let (geometry, _) =
        qa_app::map::collision(&map, &mut store).expect("production collision loader");

    let mut native_queries = Vec::new();
    for model in [0, 1, map.models.len() as u32 - 1] {
        for (mins, maxs) in [
            (Vec3::default(), Vec3::default()),
            (Vec3([-16.0, -16.0, -24.0]), Vec3([16.0, 16.0, 32.0])),
            (Vec3([-32.0; 3]), Vec3([32.0; 3])),
        ] {
            let query = TraceQuery {
                mins,
                maxs,
                ..TraceQuery::point(
                    start,
                    end,
                    TraceRules::LEGACY,
                    trace_policy(RuleSetId::Quake).1,
                )
            };
            let trace = store
                .trace_model(geometry, model, query, &mut store.scratch())
                .expect("native before optional admission");
            native_queries.push((model, query, trace));
        }
    }
    let limits = HullPositionLimits::default();
    println!("e1m1 cold exact phase=admission begin limits={limits:?}");
    let admission = store.prepare_hull_positions(geometry, limits);
    println!(
        "e1m1 cold exact phase=admission result={admission:?} limits={limits:?}; public Capacity errors do not expose the failed budget field"
    );
    // Optional admission rejection must also leave compiled native queries intact.
    let mut scratch = store.scratch();
    for (model, query, before) in native_queries {
        let after = store
            .trace_model(geometry, model, query, &mut scratch)
            .expect("native after optional admission");
        assert_native_bits(before, after);
    }
    admission.expect("default-budget e1m1 admission; no compiled-width fallback for exact queries");

    for (name, mins, maxs) in [
        (
            "full-volume",
            Vec3([-15.0, -15.0, -24.0]),
            Vec3([15.0, 15.0, 32.0]),
        ),
        (
            "asymmetric",
            Vec3([-15.0, -13.0, -23.0]),
            Vec3([14.0, 16.0, 31.0]),
        ),
        ("point", Vec3::default(), Vec3::default()),
        ("line", Vec3([0.0, 0.0, -24.0]), Vec3([0.0, 0.0, 32.0])),
        ("flat", Vec3([-15.0, -15.0, 0.0]), Vec3([15.0, 15.0, 0.0])),
    ] {
        let query = exact_query(start, start, mins, maxs);
        let stationary = run_exact(&store, geometry, name, query, limits);
        assert!(
            !stationary.start_solid && !stationary.all_solid,
            "native player start must fit supplied {name} box"
        );
        let moving = TraceQuery { end, ..query };
        let trace = run_exact(&store, geometry, name, moving, limits);
        assert!(!trace.start_solid && !trace.all_solid);
        assert!(
            trace.fraction > 0.0 && trace.fraction < 1.0,
            "downward {name} sweep must find the floor"
        );
        assert!(trace.plane.normal.0[2] > 0.7);
        assert_eq!(
            trace.end.0.map(f32::to_bits),
            start.lerp(end, trace.fraction).0.map(f32::to_bits)
        );
        let endpoint = TraceQuery {
            start: trace.end,
            end: trace.end,
            ..query
        };
        assert!(!run_exact(&store, geometry, "published-endpoint", endpoint, limits).start_solid);
    }
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        assert_eq!(trace_policy(rules).0.hull_shape, HullShape::CompiledWidth);
    }
}
