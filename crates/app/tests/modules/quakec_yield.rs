use super::*;
use crate::{
    Runtime, WorldCollision,
    modules::{Export, ModuleRequest},
};
use qa_compat::quakec::{Layout, Vm};
use qa_console::{commands::Console, views::Context};
use qa_core::primitives::{Bounds, CallbackId, IndexSpan, NativeEntity, ThinkTime, Vec3};
use qa_formats::program::quakec::{Image, Opcode, Opcode::*};
use qa_session::timing::TickRate;
use qa_world::{
    area::LinkFlags,
    collision::{
        TraceQuery, WorldTrace,
        brushes::{BrushTree, CollisionLeaf, ModelRoot},
        surfaces::SurfaceTable,
        trace_policy,
    },
    entities::AllocationPolicy,
};

#[path = "../../../compat/tests/support/quakec_program.rs"]
mod quakec_program;

const MODULE: ModuleId = ModuleId(1);
const ACTOR: i32 = 2;
const TRIGGER: i32 = 3;
const PRODUCER: i32 = 4;
const SENTINEL: i32 = 5;
const PUBLISHED: Vec3 = Vec3([33.0, 0.0, 8.0]);
const FIELDS: &[(u16, u16, &[u8])] = &[
    (3, 0, b"origin"),
    (3, 3, b"mins"),
    (3, 6, b"maxs"),
    (3, 9, b"size"),
    (3, 12, b"absmin"),
    (3, 15, b"absmax"),
    (2, 18, b"solid"),
    (2, 19, b"flags"),
    (4, 20, b"owner"),
    (1, 21, b"model"),
    (2, 22, b"modelindex"),
    (2, 23, b"nextthink"),
    (6, 24, b"think"),
    (2, 25, b"movetype"),
    (3, 26, b"angles"),
    (3, 29, b"velocity"),
    (2, 32, b"frame"),
    (1, 33, b"classname"),
    (6, 34, b"touch"),
];

struct Fixture {
    host: FrameHost,
    ids: [EntityId; 4],
    publish: CallbackId,
    malformed: CallbackId,
    repair: CallbackId,
    stride: u32,
}

fn function(
    operations: &mut Vec<(Opcode, i16, i16, i16)>,
    entries: &mut Vec<i32>,
    body: &[(Opcode, i16, i16, i16)],
) -> u32 {
    let entry = entries.len() as u32;
    entries.push(operations.len() as i32 + 1);
    operations.extend_from_slice(body);
    entry
}

fn fixture(rules: RuleSetId) -> Fixture {
    let mut operations = Vec::new();
    let mut entries = vec![0, -32, -4];
    let idle = function(&mut operations, &mut entries, &[(Return, 0, 0, 0)]);
    let mut link = Vec::new();
    for native_global in 80..84 {
        link.extend_from_slice(&[
            (StoreEnt, native_global, 4, 0),
            (StoreV, 100, 7, 0),
            (StoreV, 103, 10, 0),
            (Call3, 66, 0, 0),
        ]);
    }
    link.push((Return, 0, 0, 0));
    let link = function(&mut operations, &mut entries, &link);
    let touch = function(
        &mut operations,
        &mut entries,
        &[(AddF, 70, 48, 70), (Return, 0, 0, 0)],
    );
    let walk = [
        (StoreF, 0, 4, 0),
        (StoreF, 0, 7, 0),
        (Call2, 65, 0, 0),
        (StoreF, 1, 90, 0),
        (StoreF, 70, 91, 0),
        (StoreEnt, 34, 92, 0),
        (Return, 90, 0, 0),
    ];
    let mut publish = vec![(Address, 82, 60, 94), (StorePV, 110, 94, 0)];
    publish.extend_from_slice(&walk);
    let publish = function(&mut operations, &mut entries, &publish);
    let mut malformed = vec![
        (Address, 82, 60, 94),
        (StorePV, 110, 94, 0),
        (Address, 83, 61, 94),
        (StorePS, 67, 94, 0),
    ];
    malformed.extend_from_slice(&walk);
    let malformed = function(&mut operations, &mut entries, &malformed);
    let mut repair = vec![(Address, 83, 61, 94), (StorePS, 0, 94, 0)];
    repair.extend_from_slice(&walk);
    let repair = function(&mut operations, &mut entries, &repair);
    let bytes = quakec_program::program_with_definitions(
        &operations,
        &entries,
        &[(4, 34, b"self"), (2, 35, b"time"), (4, 36, b"other")],
        FIELDS,
        35,
        128,
    );
    let mut vm = Vm::load(
        Image::parse(&bytes, Some(5927)).unwrap(),
        Layout {
            entities: 64,
            header_bytes: 16,
            extra_string_bytes: 128,
            state_step: 0.1,
            random_seed: 1,
        },
    )
    .unwrap();
    let stride = vm.stride as u32;
    for (global, word) in [
        (48, 1.0f32.to_bits()),
        (60, 0),
        (61, 33),
        (65, 1),
        (66, 2),
        (67, u32::MAX),
    ] {
        vm.image.globals[global] = word;
    }
    vm.image.globals[100..103].fill((-1.0f32).to_bits());
    vm.image.globals[103..106].fill(1.0f32.to_bits());
    vm.image.globals[110..113].copy_from_slice(&PUBLISHED.0.map(f32::to_bits));
    let mut runtime = Runtime::load(1, []).unwrap();
    let geometry = runtime
        .geometry
        .load_brushes(
            vec![],
            vec![],
            Vec::new(),
            SurfaceTable::flags(vec![]),
            BrushTree {
                planes: vec![],
                nodes: vec![],
                leaves: vec![CollisionLeaf {
                    stored_contents: None,
                    brushes: IndexSpan::default(),
                    patches: Default::default(),
                }],
                leaf_brushes: vec![],
                models: vec![ModelRoot::Leaf(0)],
                leaf_patches: Vec::new(),
            },
            vec![Bounds {
                mins: Vec3([-512.0; 3]),
                maxs: Vec3([512.0; 3]),
            }],
        )
        .unwrap();
    runtime.collision = Some(WorldCollision::new(&runtime.geometry, geometry, 0));
    let anchor = runtime
        .server
        .entities
        .allocate(ThinkTime::Seconds(0.0), MODULE, AllocationPolicy::EDICT)
        .unwrap()
        .id;
    runtime.server.entities.columns.native_entity[anchor.slot as usize] = Some(NativeEntity {
        module: MODULE,
        slot: 0,
    });
    let ids = std::array::from_fn(|index| {
        let native = index as i32 + ACTOR;
        let entity = runtime
            .server
            .entities
            .allocate(ThinkTime::Seconds(0.0), MODULE, AllocationPolicy::EDICT)
            .unwrap()
            .id;
        runtime.server.entities.columns.native_entity[entity.slot as usize] = Some(NativeEntity {
            module: MODULE,
            slot: native,
        });
        vm.image.globals[80 + index] = native as u32 * stride;
        let origin = match native {
            PRODUCER => Vec3([32.0, 0.0, 8.0]),
            SENTINEL => Vec3([36.0, 0.0, 8.0]),
            _ => Vec3([0.0, 0.0, 8.0]),
        };
        let [origin_at, solid_at, flags_at, touch_at] = [0, 18, 19, 34]
            .map(|field| vm.field_address(native as u32 * stride, field, 1).unwrap());
        vm.entities.write_vec3(origin_at, origin).unwrap();
        vm.entities
            .write_word(
                solid_at,
                (if native == TRIGGER { 1.0f32 } else { 2.0f32 }).to_bits() as i32,
            )
            .unwrap();
        vm.entities
            .write_word(flags_at, 1536.0f32.to_bits() as i32)
            .unwrap();
        if native == TRIGGER {
            vm.entities.write_word(touch_at, touch as i32).unwrap();
        }
        entity
    });
    let request = ModuleRequest {
        context: CallContext {
            native_rules: rules,
            trace_rules: RuleSetId::Quake3,
            sound_policy: qa_gameplay::rules::sound_policy(rules),
            server_frame: 0,
            module: MODULE,
            clock: ThinkTime::Seconds(0.0),
            console: Context::default(),
            allocation: AllocationPolicy::for_rules(rules, 0),
            link_order: qa_gameplay::rules::link_order(rules),
        },
        phase: super::super::Phase::Server,
        timing_rules: rules,
        rate: TickRate::fixed(100).unwrap(),
        anchor,
        program: Program::quakec(vm, None),
        entries: (0..entries.len() as u32).collect(),
        frame: Export::clocked(CallbackId(idle)),
        prepare: vec![],
        initialize: vec![],
        api: None,
        shutdown: vec![],
        instruction_budget: 1000,
        configstrings: 0,
        files: 0,
        entity_source: None,
    };
    let mut console = Console::new(Context::default()).unwrap();
    console
        .append("sv_maxclients 1\nsv_maxtic 1\n", Context::default())
        .unwrap();
    console.execute_frame(&mut runtime);
    let mut host =
        FrameHost::load_modules(console, runtime, TickRate::FrameDriven, vec![request]).unwrap();
    host.call_module_export(
        MODULE,
        Export::clocked(CallbackId(link)),
        ThinkTime::Seconds(5.0),
    )
    .unwrap();
    Fixture {
        host,
        ids,
        publish: CallbackId(publish),
        malformed: CallbackId(malformed),
        repair: CallbackId(repair),
        stride,
    }
}

fn call(entity: EntityId) -> CallbackCall<'static> {
    CallbackCall::Export {
        entity,
        time: ThinkTime::Seconds(5.0),
        arguments: [0; 9],
        client: None,
        command: CommandArgs::default(),
    }
}

fn activation(host: &mut FrameHost, callback: CallbackId, entity: EntityId) -> Activation {
    let functions = Arc::clone(&host.modules.as_ref().unwrap().functions);
    let ModuleDispatch::Guest(start) = functions.dispatch(host, callback, call(entity)).unwrap()
    else {
        panic!("bytecode fixture must enter its owned activation");
    };
    Activation::new(start, false)
}

fn other_links(host: &FrameHost, actor: EntityId) -> Vec<EntityId> {
    host.runtime
        .server
        .area
        .query(
            &host.runtime.server.entities,
            Bounds {
                mins: Vec3([-512.0; 3]),
                maxs: Vec3([512.0; 3]),
            },
            LinkFlags::SOLID,
        )
        .filter_map(|linked| (linked.id != actor).then_some(linked.id))
        .collect()
}

#[test]
fn qc_host_yield_publishes_unrelated_dirty_body_without_relinking_it() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let Fixture {
            mut host,
            ids,
            publish,
            stride,
            ..
        } = fixture(rules);
        let bounds = host.runtime.server.area.bounds(ids[2]).unwrap();
        let order = other_links(&host, ids[0]);
        let relinks = host.runtime.server.area.relinks;
        let mut frame = activation(&mut host, publish, ids[0]);
        let GuestStep::Host(request, point) = advance(
            &mut host,
            &mut frame,
            Arguments::default(),
            TraceAccess::empty(),
            &mut crate::modules::Driver::cache_only(),
        )
        .unwrap() else {
            panic!("walkmove must yield before its trigger callback");
        };
        assert_eq!(
            host.runtime.server.entities.columns.position[ids[2].slot as usize],
            PUBLISHED
        );
        assert_eq!(host.runtime.server.area.bounds(ids[2]), Some(bounds));
        assert_eq!(other_links(&host, ids[0]), order);
        assert_eq!(host.runtime.server.area.relinks, relinks + 1);
        let Program::QuakeC(qc) = &host.module_row(MODULE).unwrap().request.program else {
            unreachable!()
        };
        assert!(qc.vm.is_running());
        assert_eq!(qc.vm.image.globals[70], 0);
        // This uses common columns, with no stopped-VM trace view. The point is
        // inside the newly published box and outside its previous body pose.
        let world = host.runtime.collision.as_mut().unwrap();
        let (trace_rules, entity_rules) = trace_policy(RuleSetId::Quake3);
        let trace_point = Vec3([33.5, 0.0, 8.0]);
        let mut query = TraceQuery::point(trace_point, trace_point, trace_rules, entity_rules);
        query.mask |= qa_world::collision::Contents::BODY;
        let trace = WorldTrace::new(
            &host.runtime.geometry,
            world.geometry,
            world.index,
            &host.runtime.server.entities,
            &host.runtime.server.area,
            &mut world.scratch,
            qa_core::primitives::CollisionOwner::None,
        )
        .trace(query)
        .unwrap();
        assert!(trace.start_solid);
        let HostRequest::TouchTriggers { mover, time } = request else {
            unreachable!()
        };
        frame.pending = Some(Pending {
            point: Some(point),
            operation: Operation::Triggers {
                run: contacts::TriggerRun::begin(&host, mover, time).unwrap(),
                touch: None,
            },
        });
        assert!(
            drive(
                &mut host,
                Frame::Guest(frame),
                CommandArgs::default(),
                &mut crate::modules::Driver::cache_only()
            )
            .is_ok()
        );
        assert_eq!(
            host.module_counts(MODULE).unwrap().last_result,
            Some(ModuleResult::QuakeC([
                1.0f32.to_bits(),
                1.0f32.to_bits(),
                ACTOR as u32 * stride
            ]))
        );
        assert_eq!(host.runtime.server.area.bounds(ids[2]), Some(bounds));
        assert_eq!(other_links(&host, ids[0]), order);
        assert_eq!(host.runtime.server.area.relinks, relinks + 1);
        assert_eq!(host.module_counts(MODULE).unwrap().traps, 0);
    }
}

#[test]
fn qc_rejected_host_publication_consumes_continuation_before_cancel_and_recovers() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let Fixture {
            mut host,
            ids,
            malformed,
            repair,
            stride,
            ..
        } = fixture(rules);
        let bounds = host.runtime.server.area.bounds(ids[2]).unwrap();
        let mut frame = activation(&mut host, malformed, ids[0]);
        assert!(
            advance(
                &mut host,
                &mut frame,
                Arguments::default(),
                TraceAccess::empty(),
                &mut crate::modules::Driver::cache_only()
            )
            .is_err()
        );
        assert!(frame.pending.is_none());
        assert!(frame.reply.is_none());
        // A successfully imported prefix survives the later malformed row.
        assert_eq!(
            host.runtime.server.entities.columns.position[ids[2].slot as usize],
            PUBLISHED
        );
        assert_eq!(host.runtime.server.area.bounds(ids[2]), Some(bounds));
        let Program::QuakeC(qc) = &mut host.module_row_mut(MODULE).unwrap().request.program else {
            unreachable!()
        };
        assert!(qc.vm.is_running());
        assert_eq!(qc.vm.image.globals[34], ACTOR as u32 * stride);
        assert_eq!(qc.vm.image.globals[70], 0);
        // The yielded import is already consumed. Cancellation must not apply
        // its saved self a second time over a later scalar value.
        qc.vm.image.globals[34] = SENTINEL as u32 * stride;
        cancel(
            &mut host,
            &mut frame,
            Arguments::default(),
            TraceAccess::empty(),
        );
        let Program::QuakeC(qc) = &host.module_row(MODULE).unwrap().request.program else {
            unreachable!()
        };
        assert!(!qc.vm.is_running());
        assert_eq!(qc.vm.image.globals[34], SENTINEL as u32 * stride);
        completing(&mut host, &mut frame, Err(()));
        assert!(
            !drive(
                &mut host,
                Frame::Guest(frame),
                CommandArgs::default(),
                &mut crate::modules::Driver::cache_only()
            )
            .is_ok()
        );
        host.call_module(repair, call(ids[0])).unwrap();
        assert_eq!(
            host.module_counts(MODULE).unwrap().last_result,
            Some(ModuleResult::QuakeC([
                1.0f32.to_bits(),
                1.0f32.to_bits(),
                ACTOR as u32 * stride
            ]))
        );
        assert_eq!(host.module_counts(MODULE).unwrap().traps, 1);
        assert_eq!(
            host.module_state(MODULE),
            Some(crate::modules::State::Running)
        );
        assert_eq!(host.runtime.server.area.bounds(ids[2]), Some(bounds));
    }
}
