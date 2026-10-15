use super::*;
use crate::{
    Runtime, WorldCollision,
    host::FrameHost,
    modules::{
        Export, ModuleResult,
        contacts::tests::{FIELDS, request},
        edit_quakec,
    },
};
use qa_compat::quakec::{Layout, Vm};
use qa_console::{commands::Console, views::Context};
use qa_core::primitives::{CallbackId, IndexSpan, Plane, RuleSetId, SurfaceFlags, ThinkTime, Vec3};
use qa_formats::program::quakec::{Image, Opcode::*};
use qa_session::{physics::PushNumber, timing::TickRate};
use qa_world::collision::{
    TraceQuery,
    brushes::{Brush, BrushTree, CollisionLeaf, ModelRoot},
    surfaces::SurfaceTable,
    trace_policy,
};

#[path = "../../../compat/tests/support/quakec_program.rs"]
mod quakec_program;

const A: ModuleId = ModuleId(1);
const B: ModuleId = ModuleId(2);

fn program() -> Vm {
    let bytes = quakec_program::program_with_definitions(
        &[
            (Return, 0, 0, 0),
            (StoreV, 100, 4, 0),
            (StoreV, 103, 7, 0),
            (StoreF, 62, 10, 0),
            (StoreEnt, 63, 13, 0),
            (Call4, 60, 0, 0),
            (Return, 80, 0, 0),
            (Call0, 61, 0, 0),
            (Return, 1, 0, 0),
            (Address, 34, 110, 114),
            (StorePS, 111, 114, 0),
            (Return, 0, 0, 0),
            (Address, 34, 113, 114),
            (StorePV, 100, 114, 0),
            (Call0, 112, 0, 0),
            (Return, 0, 0, 0),
        ],
        &[0, 1, 2, 8, -16, -14, 10, 13],
        &[
            (4, 34, b"self"),
            (2, 35, b"time"),
            (4, 36, b"other"),
            (2, 80, b"trace_fraction"),
            (4, 81, b"trace_ent"),
            (2, 82, b"trace_startsolid"),
            (2, 83, b"trace_allsolid"),
            (2, 84, b"trace_inwater"),
            (2, 85, b"trace_inopen"),
            (2, 86, b"trace_plane_dist"),
            (3, 87, b"trace_endpos"),
            (3, 90, b"trace_plane_normal"),
        ],
        FIELDS,
        40,
        128,
    );
    let mut vm = Vm::load(
        Image::parse(&bytes, Some(5927)).unwrap(),
        Layout {
            entities: 8,
            header_bytes: 16,
            extra_string_bytes: 256,
            state_step: 0.1,
            random_seed: 1,
        },
    )
    .unwrap();
    vm.image.globals[60] = 4;
    vm.image.globals[61] = 5;
    vm.image.globals[63] = 2 * vm.stride as u32;
    vm.image.globals[103] = 40.0f32.to_bits();
    vm.image.globals[110] = 27;
    vm
}

fn geometry(runtime: &mut Runtime, sizes: [f32; 2]) -> GeometryId {
    let mut planes = vec![];
    let mut brushes = vec![];
    let mut leaves = vec![CollisionLeaf {
        stored_contents: None,
        brushes: IndexSpan::default(),
        patches: Default::default(),
    }];
    let mut models = vec![ModelRoot::Leaf(0)];
    let mut bounds = vec![Bounds {
        mins: Vec3([-512.0; 3]),
        maxs: Vec3([512.0; 3]),
    }];
    for size in sizes {
        let first = planes.len() as u32;
        for normal in [
            Vec3([1.0, 0.0, 0.0]),
            Vec3([-1.0, 0.0, 0.0]),
            Vec3([0.0, 1.0, 0.0]),
            Vec3([0.0, -1.0, 0.0]),
            Vec3([0.0, 0.0, 1.0]),
            Vec3([0.0, 0.0, -1.0]),
        ] {
            planes.push(Plane::oriented(normal, size));
        }
        let brush = brushes.len() as u32;
        brushes.push(Brush {
            sides: IndexSpan { first, count: 6 },
            contents: qa_world::collision::Contents::SOLID,
        });
        leaves.push(CollisionLeaf {
            stored_contents: None,
            brushes: IndexSpan {
                first: brush,
                count: 1,
            },
            patches: Default::default(),
        });
        models.push(ModelRoot::Leaf(leaves.len() as u32 - 1));
        bounds.push(Bounds {
            mins: Vec3([-size; 3]),
            maxs: Vec3([size; 3]),
        });
    }
    let surfaces = SurfaceTable::flags(vec![SurfaceFlags::default(); planes.len()]);
    runtime
        .geometry
        .load_brushes(
            planes,
            brushes,
            Vec::new(),
            surfaces,
            BrushTree {
                planes: vec![],
                nodes: vec![],
                leaves,
                leaf_brushes: vec![0, 1],
                models,
                leaf_patches: Vec::new(),
            },
            bounds,
        )
        .unwrap()
}

fn fixture() -> (
    FrameHost,
    GeometryId,
    GeometryId,
    qa_core::primitives::EntityId,
) {
    let mut runtime = Runtime::load(2, []).unwrap();
    let owner_geometry = geometry(&mut runtime, [2.0, 6.0]);
    let query_geometry = geometry(&mut runtime, [8.0, 10.0]);
    runtime.collision = Some(WorldCollision::new(&runtime.geometry, owner_geometry, 0));
    let requests = [A, B].map(|module| {
        request(
            &mut runtime,
            module,
            Program::quakec(program(), None),
            (0..8).collect(),
            RuleSetId::Quake,
        )
        .0
    });
    let mut console = Console::new(Context::default()).unwrap();
    console
        .append("sv_maxclients 1\n", Context::default())
        .unwrap();
    console.execute_frame(&mut runtime);
    let mut host =
        FrameHost::load_modules(console, runtime, TickRate::FrameDriven, requests.into()).unwrap();
    for module in [A, B] {
        assert_eq!(
            call(&mut host, module, 3)[0],
            2 * match &host.module_row(module).unwrap().request.program {
                Program::QuakeC(qc) => qc.vm.stride as u32,
                _ => unreachable!(),
            }
        );
        edit_quakec(
            &mut host,
            module,
            crate::modules::execution::TraceAccess::empty(),
            |vm, services, context| {
                let native = 2 * vm.stride as u32;
                let model = vm.insert_string(b"*1").unwrap();
                for (field, word) in [
                    (24, if module == A { 4.0f32 } else { 2.0f32 }.to_bits()),
                    (27, model as u32),
                ] {
                    let address = vm.field_address(native, field, 1).unwrap();
                    vm.entities.write_word(address, word as i32).unwrap();
                }
                for (field, vector) in [
                    (0, Vec3([if module == A { 20.0 } else { -100.0 }, 0.0, 0.0])),
                    (9, Vec3([-12.0; 3])),
                    (12, Vec3([12.0; 3])),
                ] {
                    let address = vm.field_address(native, field, 3).unwrap();
                    vm.entities.write_vec3(address, vector).unwrap();
                }
                vm.relink_entity(services, context, native)
            },
        )
        .unwrap();
    }
    let actor = host
        .runtime
        .server
        .entities
        .find_native(NativeEntity { module: A, slot: 2 }, None)
        .unwrap();
    assert_eq!(host.module_row(A).unwrap().geometry, Some(owner_geometry));
    host.runtime.collision = Some(WorldCollision::new(
        &host.runtime.geometry,
        query_geometry,
        0,
    ));
    (host, owner_geometry, query_geometry, actor)
}

fn call(host: &mut FrameHost, module: ModuleId, callback: u32) -> [u32; 3] {
    host.call_module_export(
        module,
        Export::clocked(CallbackId(callback)),
        ThinkTime::Seconds(3.0),
    )
    .unwrap();
    let Some(ModuleResult::QuakeC(words)) = host.module_counts(module).unwrap().last_result else {
        unreachable!()
    };
    words
}

fn solid(host: &mut FrameHost, value: f32) {
    edit_quakec(
        host,
        A,
        crate::modules::execution::TraceAccess::empty(),
        |vm, services, context| {
            vm.push_store(
                &mut services.server.entities,
                context,
                2 * vm.stride as u32,
                PushNumber::Solid,
                value,
            )
        },
    )
    .unwrap();
}

fn model(host: &mut FrameHost, text: &[u8]) {
    let Program::QuakeC(qc) = &mut host.module_row_mut(A).unwrap().request.program else {
        unreachable!()
    };
    qc.vm.image.globals[111] = qc.vm.insert_string(text).unwrap() as u32;
    host.call_module(
        CallbackId(6),
        qa_core::primitives::CallbackCall::Export {
            entity: host
                .runtime
                .server
                .entities
                .find_native(NativeEntity { module: A, slot: 2 }, None)
                .unwrap(),
            time: ThinkTime::Seconds(3.0),
            arguments: [0; 9],
            client: None,
            command: Default::default(),
        },
    )
    .unwrap();
}

fn expected(host: &FrameHost, geometry: GeometryId, model: u32) -> u32 {
    let (rules, entities) = trace_policy(RuleSetId::Quake3);
    host.runtime
        .geometry
        .trace_transformed(
            geometry,
            model,
            TraceQuery::point(Vec3::default(), Vec3([40.0, 0.0, 0.0]), rules, entities),
            Vec3([20.0, 0.0, 0.0]),
            Vec3::default(),
            qa_gameplay::rules::model_rules(RuleSetId::Quake),
            &mut host.runtime.geometry.scratch(),
        )
        .unwrap()
        .fraction
        .to_bits()
}

#[test]
fn foreign_qc_trace_reads_temporary_solid_and_current_model_from_its_owner_geometry() {
    let (mut host, owner, query, actor) = fixture();
    let bounds = host.runtime.server.area.bounds(actor);
    let relinks = host.runtime.server.area.relinks;
    let Program::QuakeC(qc) = &mut host.module_row_mut(A).unwrap().request.program else {
        unreachable!()
    };
    qc.vm.image.globals[63] = 0;
    assert_eq!(call(&mut host, A, 2)[0], expected(&host, owner, 1));
    let first = call(&mut host, B, 2);
    assert_eq!(first[0], expected(&host, owner, 1));
    assert_ne!(first[0], expected(&host, query, 1));
    assert_ne!(first[1], 0);
    solid(&mut host, 0.0);
    assert_ne!(
        host.runtime.server.entities.columns.collision_shape[actor.slot as usize],
        qa_core::primitives::CollisionShape::None
    );
    assert_eq!(call(&mut host, B, 2)[0], 1.0f32.to_bits());
    model(&mut host, b"*2");
    solid(&mut host, 4.0);
    let changed = call(&mut host, B, 2);
    assert_eq!(changed[0], expected(&host, owner, 2));
    assert_ne!(changed[0], first[0]);
    assert_eq!(changed[1], first[1]);
    assert_eq!(host.runtime.server.area.bounds(actor), bounds);
    assert_eq!(host.runtime.server.area.relinks, relinks);
}

#[test]
fn cancelled_qc_export_publishes_its_admitted_prefix_in_owner_geometry() {
    use qa_core::primitives::{CallbackCall, CollisionShape};
    let (mut host, owner, query, actor) = fixture();
    let bounds = host.runtime.server.area.bounds(actor);
    let relinks = host.runtime.server.area.relinks;
    let position = Vec3([24.0, 0.0, 0.0]);
    let raw_model = {
        let Program::QuakeC(qc) = &mut host.module_row_mut(A).unwrap().request.program else {
            unreachable!()
        };
        for (word, value) in qc.vm.image.globals[100..103].iter_mut().zip(position.0) {
            *word = value.to_bits();
        }
        qc.vm.image.globals[112] = u32::MAX;
        qc.vm.image.globals[113] = 0;
        let field = qc.vm.field_address(2 * qc.vm.stride as u32, 27, 1).unwrap();
        qc.vm.entities.read_word(field).unwrap()
    };
    assert!(
        host.call_module(
            CallbackId(7),
            CallbackCall::Export {
                entity: actor,
                time: ThinkTime::Seconds(3.0),
                arguments: [0; 9],
                client: None,
                command: Default::default(),
            },
        )
        .is_err()
    );
    assert_eq!(host.module_counts(A).unwrap().traps, 1);
    let slot = actor.slot as usize;
    assert_eq!(
        host.runtime.server.entities.columns.position[slot],
        position
    );
    assert_eq!(
        host.runtime.server.entities.columns.collision_shape[slot],
        CollisionShape::Model {
            geometry: owner,
            index: 1,
        }
    );
    assert_ne!(owner, query);
    assert_eq!(host.runtime.server.area.bounds(actor), bounds);
    assert_eq!(host.runtime.server.area.relinks, relinks);
    let Program::QuakeC(qc) = &mut host.module_row_mut(A).unwrap().request.program else {
        unreachable!()
    };
    let native = 2 * qc.vm.stride as u32;
    let origin = qc.vm.field_address(native, 0, 3).unwrap();
    assert_eq!(qc.vm.entities.read_vec3(origin).unwrap(), position);
    let model = qc.vm.field_address(native, 27, 1).unwrap();
    assert_eq!(qc.vm.entities.read_word(model).unwrap(), raw_model);
    call(&mut host, A, 1);
    assert_eq!(host.module_counts(A).unwrap().traps, 1);
}

#[test]
fn foreign_qc_box_does_not_require_an_owner_geometry_binding() {
    let (mut host, _, _, _) = fixture();
    host.module_row_mut(A).unwrap().geometry = None;
    solid(&mut host, 2.0);
    let hit = call(&mut host, B, 2);
    assert!(f32::from_bits(hit[0]) < 1.0);
    assert_ne!(hit[1], 0);
}

#[test]
fn foreign_qc_invalid_target_rejects_only_when_reached_and_later_trace_recovers() {
    let (mut host, _, _, actor) = fixture();
    solid(&mut host, 0.0);
    model(&mut host, b"*999");
    assert_eq!(call(&mut host, B, 2)[0], 1.0f32.to_bits());
    solid(&mut host, 4.0);
    let Program::QuakeC(qc) = &mut host.module_row_mut(B).unwrap().request.program else {
        unreachable!()
    };
    qc.vm.image.globals[101] = 100.0f32.to_bits();
    qc.vm.image.globals[104] = 100.0f32.to_bits();
    assert_eq!(call(&mut host, B, 2)[0], 1.0f32.to_bits());
    let Program::QuakeC(qc) = &mut host.module_row_mut(B).unwrap().request.program else {
        unreachable!()
    };
    qc.vm.image.globals[101] = 0;
    qc.vm.image.globals[104] = 0;
    assert!(
        host.call_module_export(B, Export::clocked(CallbackId(2)), ThinkTime::Seconds(3.0))
            .is_err()
    );
    assert_eq!(host.module_counts(B).unwrap().traps, 1);
    solid(&mut host, 0.0);
    model(&mut host, b"*1");
    solid(&mut host, 4.0);
    assert_ne!(call(&mut host, B, 2)[0], 1.0f32.to_bits());
    assert_eq!(host.module_counts(B).unwrap().traps, 1);
    assert!(host.runtime.server.area.bounds(actor).is_some());
}

#[test]
fn pending_foreign_qc_remains_readable_but_retired_owner_anchor_uses_cached_body() {
    let (mut host, owner, _, _) = fixture();
    host.module_row_mut(A).unwrap().state = super::super::State::Pending;
    solid(&mut host, 0.0);
    assert_eq!(call(&mut host, B, 2)[0], 1.0f32.to_bits());
    let anchor = host.module_row(A).unwrap().request.anchor;
    assert!(
        host.runtime
            .server
            .free_entity(anchor, ThinkTime::Seconds(3.0))
    );
    assert_eq!(call(&mut host, B, 2)[0], expected(&host, owner, 1));
}

#[test]
fn engine_client_rejects_live_foreign_target_before_commit_and_next_client_still_moves() {
    use crate::{client_policy::ClientPolicy, map::SpawnAnchor};
    use qa_core::{
        primitives::UserCmd,
        sys_events::{EventTime, SeatId},
    };
    use qa_network::commands::packet::{CommandExecution, Protocol};
    let (mut host, _, _, _) = fixture();
    solid(&mut host, 0.0);
    model(&mut host, b"*999");
    solid(&mut host, 4.0);
    let mut clients = Vec::new();
    for (seat, y) in [(SeatId::ALL[0], 0.0), (SeatId::ALL[1], 100.0)] {
        clients.push(
            host.runtime
                .connect_local(
                    seat,
                    SpawnAnchor {
                        position: Vec3([0.0, y, 0.0]),
                        angles: Vec3::default(),
                        entity: 0,
                        fixture_fallback: false,
                    },
                    ClientPolicy {
                        input: RuleSetId::Quake3,
                        client: RuleSetId::Quake3,
                        movement: RuleSetId::Quake3,
                        trace: RuleSetId::Quake3,
                    },
                    Protocol::Quake3_68,
                    seat.index() as u32,
                )
                .unwrap(),
        );
    }
    let bad = clients[0];
    let entity = host.runtime.server.clients[bad.0 as usize].entity;
    let body = host.runtime.server.clients[bad.0 as usize].player.body;
    let tuning = host.runtime.server.clients[bad.0 as usize]
        .player
        .movement
        .tuning;
    let common = host
        .runtime
        .server
        .entities
        .columns
        .body(entity.slot as usize);
    let bounds = host.runtime.server.area.bounds(entity);
    let links = host.runtime.server.area.relinks;
    for id in &clients {
        host.received_command(
            *id,
            UserCmd {
                movement: [127.0, 0.0, 0.0],
                units: qa_gameplay::rules::command_units(RuleSetId::Quake3),
                ..Default::default()
            },
            CommandExecution::FrameLatest,
            None,
        );
    }
    host.time = EventTime::from_milliseconds(50);
    host.run_pending_commands(EventTime::from_milliseconds(50));
    let client = &host.runtime.server.clients[bad.0 as usize];
    assert!(client.is_active());
    assert_eq!(client.player.body.position, body.position);
    assert_eq!(client.player.body.velocity, body.velocity);
    assert_eq!(client.player.movement.command_time_ms, 0);
    assert_eq!(client.player.movement.tuning, tuning);
    assert!(host.runtime.server.take_pending_command(bad).is_none());
    let current = host
        .runtime
        .server
        .entities
        .columns
        .body(entity.slot as usize);
    assert_eq!(current.position, common.position);
    assert_eq!(current.velocity, common.velocity);
    assert_eq!(host.runtime.server.area.bounds(entity), bounds);
    let healthy = &host.runtime.server.clients[clients[1].0 as usize];
    assert!(healthy.is_active());
    assert!(healthy.player.body.position.0[0] > 0.0);
    assert_eq!(healthy.player.movement.command_time_ms, 50);
    assert_eq!(host.runtime.server.area.relinks, links + 1);
    assert_eq!(call(&mut host, B, 1), [0; 3]);
}
