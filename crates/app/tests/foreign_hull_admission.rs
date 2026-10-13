//! Cold retail admission and supplied-box queries through the production loader.
//! This is neither a gameplay run nor a timing/allocation qualification.
use qa_content::vfs::Vfs;
use qa_core::primitives::{Bounds, ClipNode, GeometryId, ModelRules, Plane, RuleSetId, Vec3};
use qa_formats::{
    archive::ArchiveReader,
    bsp::Map,
    entities::{EntityLump, EntitySyntax},
};
use qa_world::collision::{
    CollisionStore, HullPositionLimits, HullShape, Trace, TraceQuery, TraceRules, hulls::HullModel,
    trace_policy,
};
use std::path::PathBuf;

#[path = "support/service_program.rs"]
#[allow(dead_code)]
mod service_program;

fn half_space_runtime() -> qa_app::Runtime {
    let mut runtime = qa_app::Runtime::load(2, []).unwrap();
    let node = ClipNode {
        plane: 0,
        children: [-1, -2],
    };
    let geometry = runtime
        .geometry
        .load_hulls(
            vec![Plane::oriented(Vec3::axis(0, 1.0), 0.0)],
            vec![node],
            vec![node],
            vec![HullModel { roots: [0; 3] }],
            vec![Bounds {
                mins: Vec3([-1024.0; 3]),
                maxs: Vec3([1024.0; 3]),
            }],
        )
        .unwrap();
    runtime.collision = Some(qa_app::WorldCollision::new(&runtime.geometry, geometry, 0));
    runtime
}

fn box_crossing(rules: RuleSetId) -> TraceQuery<'static> {
    let (rules, entity_rules) = trace_policy(rules);
    TraceQuery {
        mins: Vec3([-2.0; 3]),
        maxs: Vec3([2.0; 3]),
        ..TraceQuery::point(
            Vec3([4.0, 0.0, 0.0]),
            Vec3([-4.0, 0.0, 0.0]),
            rules,
            entity_rules,
        )
    }
}

#[test]
fn client_trace_role_admits_supplied_boxes_independently_of_movement_and_protocol() {
    use qa_app::{client_policy::ClientPolicy, host::FrameHost, map::SpawnAnchor};
    use qa_console::{commands::Console, views::Context};
    use qa_core::sys_events::SeatId;
    use qa_network::commands::packet::Protocol;
    use qa_session::timing::TickRate;
    for rules in [
        RuleSetId::Quake,
        RuleSetId::QuakeWorld,
        RuleSetId::Quake2,
        RuleSetId::Quake2Rerelease,
        RuleSetId::Quake3,
    ] {
        let mut runtime = half_space_runtime();
        runtime
            .connect_local(
                SeatId::FIRST,
                SpawnAnchor {
                    position: Vec3([64.0, 0.0, 64.0]),
                    angles: Vec3::default(),
                    entity: 0,
                    fixture_fallback: false,
                },
                ClientPolicy::select(
                    Some(RuleSetId::Quake),
                    None,
                    Some(RuleSetId::Quake),
                    Some(rules),
                )
                .unwrap(),
                Protocol::QuakeWorld28,
                0,
            )
            .unwrap();
        let mut host = FrameHost::load(
            Console::new(Context::default()).unwrap(),
            runtime,
            TickRate::FrameDriven,
            vec![],
        )
        .unwrap();
        let collision = host.runtime.collision.as_mut().unwrap();
        let trace = host
            .runtime
            .geometry
            .trace_transformed(
                collision.geometry,
                collision.index,
                box_crossing(rules),
                Vec3::default(),
                Vec3::default(),
                ModelRules::default(),
                &mut collision.scratch,
            )
            .unwrap();
        if matches!(rules, RuleSetId::Quake | RuleSetId::QuakeWorld) {
            assert_eq!(trace.fraction, 1.0, "native compiled box remains selected");
        } else {
            let epsilon = if rules == RuleSetId::Quake3 {
                0.125f32
            } else {
                0.03125
            };
            assert_eq!(trace.fraction.to_bits(), ((2.0 - epsilon) / 8.0).to_bits());
            assert_eq!(trace.plane.normal, Vec3::axis(0, 1.0));
        }
    }
}

#[test]
fn module_trace_admission_precedes_initialize_and_allocates_shared_trace_scratch() {
    use qa_app::{
        host::FrameHost,
        modules::{Export, ModuleRequest, ModuleResult, Phase, Program},
    };
    use qa_compat::{abi::Q3_SERVER, services::CallContext};
    use qa_console::{commands::Console, views::Context};
    use qa_core::primitives::{CallbackId, ModuleId, ThinkTime};
    use qa_formats::program::qvm::Opcode::*;
    use qa_session::timing::TickRate;
    use qa_world::{area::LinkOrder, entities::AllocationPolicy};
    let mut operations = vec![(Enter, 64)];
    for (argument, value) in [
        64,
        128,
        144,
        160,
        176,
        1023, // Q3 ENTITYNUM_NONE in the guest ABI.
        1,
    ]
    .into_iter()
    .enumerate()
    {
        operations.extend([(Const, value), (Arg, 8 + argument as i32 * 4)]);
    }
    operations.extend([
        (Const, -25),
        (Call, 0),
        (Pop, 0),
        (Const, 72),
        (Load4, 0),
        (Leave, 64),
    ]);
    let mut data = [0u8; 192];
    for (offset, value) in [
        (128, Vec3([4.0, 0.0, 0.0])),
        (144, Vec3([-2.0; 3])),
        (160, Vec3([2.0; 3])),
        (176, Vec3([-4.0, 0.0, 0.0])),
    ] {
        for (axis, component) in value.0.into_iter().enumerate() {
            data[offset + axis * 4..offset + axis * 4 + 4]
                .copy_from_slice(&component.to_le_bytes());
        }
    }
    for rules in [
        RuleSetId::Quake2,
        RuleSetId::Quake2Rerelease,
        RuleSetId::Quake3,
    ] {
        let mut runtime = half_space_runtime();
        let module = ModuleId(1);
        let anchor = runtime
            .server
            .entities
            .allocate(ThinkTime::Milliseconds(0), module, AllocationPolicy::q3(0))
            .unwrap()
            .id;
        let mut host = FrameHost::load_modules(
            Console::new(Context::default()).unwrap(),
            runtime,
            TickRate::FrameDriven,
            vec![ModuleRequest {
                context: CallContext {
                    native_rules: RuleSetId::Quake3,
                    trace_rules: rules,
                    sound_policy: qa_gameplay::rules::sound_policy(RuleSetId::Quake3),
                    server_frame: 0,
                    module,
                    clock: ThinkTime::Milliseconds(0),
                    console: Context::default(),
                    allocation: AllocationPolicy::q3(0),
                    link_order: LinkOrder::Head,
                },
                timing_rules: RuleSetId::Quake3,
                phase: Phase::Server,
                rate: TickRate::FrameDriven,
                anchor,
                program: Program::Qvm {
                    vm: Box::new(service_program::program(&operations, &data)),
                    imports: &Q3_SERVER,
                },
                entries: vec![0],
                frame: Export::clocked(CallbackId(0)),
                prepare: vec![],
                initialize: vec![Export::clocked(CallbackId(0))],
                api: None,
                shutdown: vec![],
                instruction_budget: 100,
                configstrings: 0,
                files: 0,
                entity_source: None,
            }],
        )
        .unwrap();
        host.initialize_modules(Phase::Server);
        assert_eq!(
            host.module_counts(module).unwrap().traps,
            0,
            "{:?}",
            host.module_counts(module)
        );
        let epsilon = if rules == RuleSetId::Quake3 {
            0.125f32
        } else {
            0.03125
        };
        assert_eq!(
            host.module_counts(module).unwrap().last_result,
            Some(ModuleResult::Qvm(((2.0 - epsilon) / 8.0).to_bits() as i32))
        );
        assert_eq!(host.module_counts(module).unwrap().traps, 0);
    }
}

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
        rules: trace_policy(RuleSetId::Quake3).0,
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
    let admission = store.prepare_traces(trace_policy(RuleSetId::Quake3).0, limits);
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
