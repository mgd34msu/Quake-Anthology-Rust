use super::*;
use crate::{
    Runtime,
    modules::{Export, ModuleRequest, Phase},
};
use qa_compat::{
    abi::Q3_SERVER,
    quakec::{Layout, Vm},
    services::CallContext,
};
use qa_console::{commands::Console, views::Context};
use qa_core::primitives::{
    Body, CallbackId, CollisionShape, LinkOrder, NativeEntity, RuleSetId, Vec3,
};
use qa_core::sys_events::EventTime;
use qa_formats::program::quakec::{Image, Opcode::*};
use qa_session::timing::TickRate;
use qa_world::{area::LinkIntent, entities::AllocationPolicy};

#[path = "../../../compat/tests/support/quakec_program.rs"]
mod quakec_program;
#[path = "../support/service_program.rs"]
mod service_program;

const A: ModuleId = ModuleId(1);
const B: ModuleId = ModuleId(2);
const NATIVE: ModuleId = ModuleId(3);

fn admit(
    host: &mut FrameHost,
    recipient: ModuleId,
    reference: NativeEntityRef,
) -> Result<AdmittedEntity, ClientCommandError> {
    super::super::edit_quakec(
        host,
        recipient,
        crate::modules::execution::TraceAccess::empty(),
        |vm, services, context| vm.admit_reference(&services.server.entities, context, reference),
    )
}

pub(in crate::modules) const FIELDS: &[(u16, u16, &[u8])] = &[
    (3, 0, b"origin"),
    (3, 3, b"velocity"),
    (3, 6, b"angles"),
    (3, 9, b"mins"),
    (3, 12, b"maxs"),
    (3, 15, b"size"),
    (3, 18, b"absmin"),
    (3, 21, b"absmax"),
    (2, 24, b"solid"),
    (2, 25, b"flags"),
    (4, 26, b"owner"),
    (1, 27, b"model"),
    (2, 28, b"modelindex"),
    (1, 29, b"classname"),
    (6, 30, b"touch"),
    (2, 31, b"nextthink"),
    (6, 32, b"think"),
    (2, 33, b"movetype"),
    (2, 34, b"frame"),
    (4, 35, b"chain"),
    (2, 36, b"invincible_sound"),
    (2, 37, b"takedamage"),
    (2, 38, b"colormap"),
    (2, 39, b"skin"),
];

fn program(position: Vec3) -> Vm {
    let operations = [
        (Return, 0, 0, 0),     // 1 idle
        (StoreEnt, 34, 70, 0), // 2 touch: capture recipient self, other and time
        (StoreEnt, 36, 71, 0),
        (StoreF, 35, 72, 0),
        (LoadV, 36, 60, 80),
        (LoadF, 36, 61, 83),
        (Address, 36, 61, 90),
        (StorePF, 62, 90, 0),
        (Return, 70, 0, 0),
        (Return, 80, 0, 0), // 3 inspect position
        (Return, 83, 0, 0), // 4 inspect previous private word
        (StoreEnt, 34, 4, 0),
        (Call1, 65, 0, 0),
        (Return, 0, 0, 0), // 5 remove self
        (StoreEnt, 34, 70, 0),
        (StoreEnt, 36, 71, 0),
        (StoreF, 35, 72, 0),
        (Return, 70, 0, 0), // 6 weak touch without dereference
        (LoadV, 36, 60, 80),
        (Call0, 63, 0, 0),
        (Return, 0, 0, 0), // 7 trap
        (Address, 34, 60, 90),
        (StorePV, 100, 90, 0),
        (Return, 0, 0, 0), // 8 move self
        (StoreEnt, 36, 4, 0),
        (Call1, 65, 0, 0),
        (Return, 0, 0, 0), // 10 remove other
        (Call0, 66, 0, 0),
        (Return, 1, 0, 0), // 12 spawn an ordinary native row
        (StoreEnt, 34, 4, 0),
        (Call1, 65, 0, 0),
        (Call0, 66, 0, 0),
        (StoreEnt, 1, 84, 0),
        (Address, 84, 60, 90),
        (StorePV, 100, 90, 0),
        (Return, 84, 0, 0), // 13 retire/reuse native self, then write new pose
    ];
    let bytes = quakec_program::program_with_definitions(
        &operations,
        &[0, 1, 2, 10, 11, 12, 15, 19, 22, -15, 25, -14, 28, 30],
        &[(4, 34, b"self"), (2, 35, b"time"), (4, 36, b"other")],
        FIELDS,
        40,
        128,
    );
    let mut vm = Vm::load(
        Image::parse(&bytes, Some(5927)).unwrap(),
        Layout {
            entities: 4,
            header_bytes: 16,
            extra_string_bytes: 64,
            state_step: 0.1,
            random_seed: 1,
        },
    )
    .unwrap();
    for (at, word) in [
        (60, 0),
        (61, 35),
        (62, 123.0f32.to_bits()),
        (63, u32::MAX),
        (65, 9),
        (66, 11),
    ] {
        vm.image.globals[at] = word;
    }
    vm.image.globals[100..103].copy_from_slice(&[81.0f32, 82.0, 83.0].map(f32::to_bits));
    vm.image.globals[110..113].copy_from_slice(&position.0.map(f32::to_bits));
    let offset = 2 * vm.stride as u32;
    let origin = vm.field_address(offset, 0, 3).unwrap();
    vm.entities.write_vec3(origin, position).unwrap();
    for (field, word) in [(24, 2.0f32.to_bits()), (25, 1536.0f32.to_bits()), (30, 2)] {
        let at = vm.field_address(offset, field, 1).unwrap();
        vm.entities.write_word(at, word as i32).unwrap();
    }
    vm
}

pub(in crate::modules) fn request(
    runtime: &mut Runtime,
    module: ModuleId,
    program: Program,
    entries: Vec<u32>,
    rules: RuleSetId,
) -> (ModuleRequest, Option<EntityId>) {
    let mut allocate = |native| {
        let id = runtime
            .server
            .entities
            .allocate(ThinkTime::Seconds(0.0), module, AllocationPolicy::EDICT)
            .unwrap()
            .id;
        runtime.server.entities.columns.native_entity[id.slot as usize] = Some(NativeEntity {
            module,
            slot: native,
        });
        id
    };
    let anchor = allocate(0);
    let body = (!matches!(program, Program::QuakeC(_))).then(|| allocate(2));
    (
        ModuleRequest {
            q3_image: None,
            context: CallContext {
                native_rules: rules,
                trace_rules: RuleSetId::Quake3,
                sound_policy: qa_gameplay::rules::sound_policy(rules),
                server_frame: 0,
                module,
                clock: ThinkTime::module_clock(rules),
                console: Context::default(),
                allocation: AllocationPolicy::for_rules(rules, 0),
                link_order: LinkOrder::Tail,
            },
            phase: Phase::Server,
            timing_rules: rules,
            rate: TickRate::fixed(100).unwrap(),
            anchor,
            program,
            entries,
            frame: Export::clocked(CallbackId(if rules == RuleSetId::Quake3 { 0 } else { 1 })),
            prepare: vec![],
            initialize: vec![],
            api: None,
            shutdown: vec![],
            instruction_budget: 1000,
            configstrings: 0,
            files: 0,
            entity_source: None,
        },
        body,
    )
}

pub(in crate::modules) fn fixture() -> (FrameHost, [EntityId; 3], u32) {
    let mut runtime = Runtime::load(1, []).unwrap();
    let vm_a = program(Vec3([10.0, 20.0, 30.0]));
    let stride = vm_a.stride as u32;
    let (a, _) = request(
        &mut runtime,
        A,
        Program::quakec(vm_a, None),
        (0..14).collect(),
        RuleSetId::Quake,
    );
    let (b, _) = request(
        &mut runtime,
        B,
        Program::quakec(program(Vec3([40.0, 50.0, 60.0])), None),
        (0..14).collect(),
        RuleSetId::Quake,
    );
    use qa_formats::program::qvm::Opcode as Q;
    let native = Program::Qvm {
        vm: Box::new(service_program::program(
            &[(Q::Enter, 0), (Q::Const, 0), (Q::Leave, 0)],
            &[],
        )),
        imports: &Q3_SERVER,
    };
    let (native, entity_native) = request(&mut runtime, NATIVE, native, vec![0], RuleSetId::Quake3);
    let entity_native = entity_native.unwrap();
    runtime.server.entities.columns.set_body(
        entity_native.slot as usize,
        Body {
            position: Vec3([70.0, 80.0, 90.0]),
            mins: Vec3([-1.0; 3]),
            maxs: Vec3([1.0; 3]),
            ..Body::default()
        },
    );
    runtime.server.entities.columns.collision_shape[entity_native.slot as usize] =
        CollisionShape::Box;
    let mut console = Console::new(Context::default()).unwrap();
    console
        .append("sv_maxclients 1\n", Context::default())
        .unwrap();
    console.execute_frame(&mut runtime);
    let mut host =
        FrameHost::load_modules(console, runtime, TickRate::FrameDriven, vec![a, b, native])
            .unwrap();
    for module in [A, B] {
        host.call_module_export(
            module,
            Export::clocked(CallbackId(12)),
            ThinkTime::Seconds(0.0),
        )
        .unwrap();
        super::super::edit_quakec(
            &mut host,
            module,
            crate::modules::execution::TraceAccess::empty(),
            |vm, services, context| {
                let offset = 2 * vm.stride as u32;
                let position = Vec3(std::array::from_fn(|axis| {
                    f32::from_bits(vm.image.globals[110 + axis])
                }));
                let origin = vm.field_address(offset, 0, 3).unwrap();
                vm.entities.write_vec3(origin, position).unwrap();
                for (field, word) in [(24, 2.0f32.to_bits()), (25, 1536.0f32.to_bits()), (30, 2)] {
                    let address = vm.field_address(offset, field, 1).unwrap();
                    vm.entities.write_word(address, word as i32).unwrap();
                }
                vm.project_entities(services, context)
            },
        )
        .unwrap();
    }
    host.time = EventTime::from_milliseconds(3000);
    let entity_a = host
        .runtime
        .server
        .entities
        .find_native(NativeEntity { module: A, slot: 2 }, None)
        .unwrap();
    let entity_b = host
        .runtime
        .server
        .entities
        .find_native(NativeEntity { module: B, slot: 2 }, None)
        .unwrap();
    (host, [entity_a, entity_b, entity_native], stride)
}

pub(in crate::modules) fn set_touch(host: &mut FrameHost, module: ModuleId, callback: u32) {
    let Program::QuakeC(qc) = &mut host.module_row_mut(module).unwrap().request.program else {
        unreachable!()
    };
    let at = qc.vm.field_address(2 * qc.vm.stride as u32, 30, 1).unwrap();
    qc.vm.entities.write_word(at, callback as i32).unwrap();
}
fn inspect(host: &mut FrameHost, module: ModuleId, callback: u32) -> [u32; 3] {
    host.call_module_export(
        module,
        Export::clocked(CallbackId(callback)),
        ThinkTime::Seconds(0.0),
    )
    .unwrap();
    let Some(super::super::ModuleResult::QuakeC(words)) =
        host.module_counts(module).unwrap().last_result
    else {
        unreachable!()
    };
    words
}

#[test]
fn foreign_qc_touch_uses_recipient_clock_stable_alias_and_private_words() {
    let (mut host, [a, b, _], stride) = fixture();
    let b_capture = capture(&host, b).unwrap();

    let count = host.runtime.server.entities.len();
    let other = capture(&host, a).unwrap();
    let expected = host.module_row(B).unwrap().time(host.time).seconds() as f32;
    touch(
        &mut host,
        A,
        TraceAccess::empty(),
        b_capture,
        other,
        Some(ThinkTime::Seconds(42.0)),
        false,
        false,
        &mut crate::modules::Driver::cache_only(),
    )
    .unwrap();
    assert_eq!(
        host.module_counts(B).unwrap().last_result,
        Some(super::super::ModuleResult::QuakeC([
            2 * stride,
            3 * stride,
            expected.to_bits()
        ]))
    );
    assert_eq!(
        inspect(&mut host, B, 3),
        [10.0f32, 20.0, 30.0].map(f32::to_bits)
    );
    assert_eq!(inspect(&mut host, B, 4)[0], 0);
    touch(
        &mut host,
        A,
        TraceAccess::empty(),
        b_capture,
        other,
        Some(ThinkTime::Seconds(42.0)),
        false,
        false,
        &mut crate::modules::Driver::cache_only(),
    )
    .unwrap();
    assert_eq!(inspect(&mut host, B, 4)[0], 123.0f32.to_bits());
    assert_eq!(host.runtime.server.entities.len(), count);
    assert!(host.runtime.server.area.bounds(a).is_none());
    assert!(host.runtime.server.area.bounds(b).is_none());
}

#[test]
fn foreign_contact_dedup_uses_invoking_batch_canonical_slots() {
    let (mut host, [a, b, _], _) = fixture();
    let a_capture = capture(&host, a).unwrap();
    let b_capture = capture(&host, b).unwrap();

    let other = capture(&host, a).unwrap();
    host.module_row_mut(A).unwrap().client_contacts.begin();
    touch(
        &mut host,
        A,
        TraceAccess::empty(),
        a_capture,
        other,
        None,
        true,
        false,
        &mut crate::modules::Driver::cache_only(),
    )
    .unwrap();
    touch(
        &mut host,
        A,
        TraceAccess::empty(),
        b_capture,
        other,
        None,
        true,
        false,
        &mut crate::modules::Driver::cache_only(),
    )
    .unwrap();
    let calls = host.module_counts(B).unwrap().calls;
    touch(
        &mut host,
        A,
        TraceAccess::empty(),
        b_capture,
        other,
        None,
        true,
        false,
        &mut crate::modules::Driver::cache_only(),
    )
    .unwrap();
    assert_eq!(host.module_counts(B).unwrap().calls, calls);
    let stamps = &host.module_row(A).unwrap().client_contacts;
    assert!(stamps.contains(a.slot as usize));
    assert!(stamps.contains(b.slot as usize));
    assert!(
        !host
            .module_row(B)
            .unwrap()
            .client_contacts
            .contains(b.slot as usize)
    );
}

#[test]
fn qw_contact_dedup_keeps_captured_slot_when_callback_reuses_native_at_another_slot() {
    use qa_formats::program::quakec::Statement;
    let image = program(Vec3::default()).image;
    let mut vm = Vm::load(
        image,
        Layout {
            entities: 64,
            header_bytes: 16,
            extra_string_bytes: 64,
            state_step: 0.1,
            random_seed: 1,
        },
    )
    .unwrap();
    let stride = vm.stride as u32;
    vm.image.globals[85] = 30;
    vm.image.globals[86] = 6;
    vm.image.globals[87] = 34 * stride;
    vm.image.globals[88] = 24;
    vm.image.globals[89] = 2.0f32.to_bits();
    vm.image.functions[13].first_statement = vm.image.statements.len() as i32;
    let mut statements = vm.image.statements.to_vec();
    statements.extend(
        [
            (StoreEnt, 34, 4, 0),
            (Call1, 65, 0, 0),
            (StoreEnt, 87, 4, 0),
            (Call1, 65, 0, 0),
            (Call0, 66, 0, 0),
            (StoreEnt, 1, 84, 0),
            (Address, 84, 85, 90),
            (StorePF, 86, 90, 0),
            (Address, 84, 88, 90),
            (StorePF, 89, 90, 0),
            (Return, 84, 0, 0),
        ]
        .map(|(opcode, a, b, c)| Statement {
            opcode,
            source_opcode: opcode as u16,
            operands: [a, b, c],
        }),
    );
    vm.image.statements = statements.into_boxed_slice();
    let mut runtime = Runtime::load(1, []).unwrap();
    // QW reserves 1..=32. Releasing this unrelated common slot between native
    // spawns gives native 34 the lower slot without borrowing either QC body.
    let filler = runtime
        .server
        .entities
        .allocate(
            ThinkTime::Seconds(0.0),
            ModuleId(9),
            AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    let (request, _) = request(
        &mut runtime,
        A,
        Program::quakec(vm, None),
        (0..14).collect(),
        RuleSetId::QuakeWorld,
    );
    let anchor = request.anchor;
    let console = Console::new(Context::default()).unwrap();
    let mut host =
        FrameHost::load_modules(console, runtime, TickRate::FrameDriven, vec![request]).unwrap();
    assert_eq!(inspect(&mut host, A, 12)[0], 33 * stride);
    let original = host
        .runtime
        .server
        .entities
        .find_native(
            NativeEntity {
                module: A,
                slot: 33,
            },
            None,
        )
        .unwrap();
    assert!(
        host.runtime
            .server
            .free_entity(filler, ThinkTime::Seconds(0.0))
    );
    assert_eq!(inspect(&mut host, A, 12)[0], 34 * stride);
    let earlier = host
        .runtime
        .server
        .entities
        .find_native(
            NativeEntity {
                module: A,
                slot: 34,
            },
            None,
        )
        .unwrap();
    assert_eq!(earlier.slot, filler.slot);
    assert!(earlier.slot < original.slot);
    super::super::edit_quakec(
        &mut host,
        A,
        crate::modules::execution::TraceAccess::empty(),
        |vm, services, context| {
            for (native, callback) in [(33, 13), (34, 0)] {
                let base = native * vm.stride as u32;
                let solid = vm.field_address(base, 24, 1).unwrap();
                vm.entities
                    .write_word(solid, 2.0f32.to_bits() as i32)
                    .unwrap();
                let touch = vm.field_address(base, 30, 1).unwrap();
                vm.entities.write_word(touch, callback).unwrap();
            }
            vm.project_entities(services, context)
        },
    )
    .unwrap();
    let target = capture(&host, original).unwrap();
    let other = capture(&host, anchor).unwrap();
    host.module_row_mut(A).unwrap().client_contacts.begin();
    touch(
        &mut host,
        A,
        TraceAccess::empty(),
        target,
        other,
        None,
        true,
        false,
        &mut crate::modules::Driver::cache_only(),
    )
    .unwrap();
    let replacement = host
        .runtime
        .server
        .entities
        .resolve_reference(target)
        .unwrap();
    assert_eq!(replacement.slot, earlier.slot);
    assert_ne!(replacement, original);
    let Program::QuakeC(qc) = &host.module_row(A).unwrap().request.program else {
        unreachable!()
    };
    assert_eq!(
        qc.vm.touch_callback(33 * stride, false).unwrap(),
        Some(CallbackId(6))
    );
    let calls = host.module_counts(A).unwrap().calls;
    assert!(
        host.module_row(A)
            .unwrap()
            .client_contacts
            .contains(original.slot as usize)
    );
    assert!(
        !host
            .module_row(A)
            .unwrap()
            .client_contacts
            .contains(replacement.slot as usize)
    );
    touch(
        &mut host,
        A,
        TraceAccess::empty(),
        target,
        other,
        None,
        true,
        false,
        &mut crate::modules::Driver::cache_only(),
    )
    .unwrap();
    assert_eq!(host.module_counts(A).unwrap().calls, calls);
    assert_eq!(host.module_counts(A).unwrap().traps, 0);
}

#[test]
fn foreign_direct_touch_trap_releases_its_only_alias_slot() {
    let (mut host, [a, b, native], stride) = fixture();
    let other = reference(&host, a).unwrap();
    let call = CallbackCall::Touch {
        entity: b,
        other,
        time: Some(ThinkTime::Seconds(42.0)),
    };
    assert!(host.call_module(CallbackId(7), call).is_err());
    let native_reference = reference(&host, native).unwrap();
    // A live resident alias remains stable even without an active caller.
    assert!(matches!(
        admit(&mut host, B, native_reference),
        Err(ClientCommandError::Usercmd(
            qa_compat::services::CallError::Capacity
        ))
    ));
    assert!(host.runtime.server.free_entity(a, ThinkTime::Seconds(3.0)));
    let next = admit(&mut host, B, native_reference).unwrap();
    assert_eq!(next.offset(), 3 * stride);
    release(&mut host, next).unwrap();
    host.call_module(
        CallbackId(2),
        CallbackCall::Touch {
            entity: b,
            other: native_reference,
            time: Some(ThinkTime::Seconds(42.0)),
        },
    )
    .unwrap();
    assert_eq!(
        inspect(&mut host, B, 3),
        [70.0f32, 80.0, 90.0].map(f32::to_bits)
    );
    assert_eq!(host.module_counts(B).unwrap().traps, 1);
}

#[test]
fn foreign_impact_pins_weak_other_before_first_callback_retires_mover() {
    let (mut host, [a, b, _], stride) = fixture();
    set_touch(&mut host, A, 5);
    set_touch(&mut host, B, 6);
    let mover = capture(&host, a).unwrap();
    let obstacle = capture(&host, b).unwrap();
    impact(
        &mut host,
        A,
        TraceAccess::empty(),
        Some(a),
        mover,
        obstacle,
        ThinkTime::Seconds(42.0),
        &mut crate::modules::Driver::cache_only(),
    )
    .unwrap();
    assert!(host.runtime.server.entities.resolve(a).is_none());
    let Some(super::super::ModuleResult::QuakeC(words)) =
        host.module_counts(B).unwrap().last_result
    else {
        unreachable!()
    };
    assert_eq!(&words[..2], &[2 * stride, 3 * stride]);
    let Program::QuakeC(qc) = &host.module_row(B).unwrap().request.program else {
        unreachable!()
    };
    assert_eq!(
        qc.vm
            .resolve_entity(&host.runtime.server.entities, B, words[1])
            .unwrap(),
        None
    );
    assert_eq!(host.module_counts(B).unwrap().traps, 0);
}

#[test]
fn foreign_impact_with_no_touch_ignores_full_recipient_alias_capacity() {
    let (mut host, [a, b, native], _) = fixture();
    set_touch(&mut host, A, 0);
    set_touch(&mut host, B, 0);
    let foreign = reference(&host, native).unwrap();
    let held_a = admit(&mut host, A, foreign).unwrap();
    let held_b = admit(&mut host, B, foreign).unwrap();
    let mover = capture(&host, a).unwrap();
    let obstacle = capture(&host, b).unwrap();
    impact(
        &mut host,
        A,
        TraceAccess::empty(),
        Some(a),
        mover,
        obstacle,
        ThinkTime::Seconds(42.0),
        &mut crate::modules::Driver::cache_only(),
    )
    .unwrap();
    release(&mut host, held_a).unwrap();
    release(&mut host, held_b).unwrap();
    assert_eq!(host.module_counts(A).unwrap().traps, 0);
    assert_eq!(host.module_counts(B).unwrap().traps, 0);
}

#[test]
fn reciprocal_touch_presence_does_not_resolve_a_function_before_the_first_callback() {
    let (mut host, _, _) = fixture();
    set_touch(&mut host, B, u32::MAX);
    let Program::QuakeC(qc) = &host.module_row(B).unwrap().request.program else {
        unreachable!()
    };
    let offset = 2 * qc.vm.stride as u32;
    assert!(qc.vm.touch_pending(offset).unwrap());
    assert!(qc.vm.touch_callback(offset, false).is_err());
}

#[test]
fn foreign_impact_runs_mover_touch_when_no_touch_recipient_has_full_alias_capacity() {
    let (mut host, [a, b, native], stride) = fixture();
    set_touch(&mut host, A, 6);
    set_touch(&mut host, B, 0);
    let foreign = reference(&host, native).unwrap();
    let held = admit(&mut host, B, foreign).unwrap();
    let mover = capture(&host, a).unwrap();
    let obstacle = capture(&host, b).unwrap();
    let calls = host.module_counts(B).unwrap().calls;
    impact(
        &mut host,
        A,
        TraceAccess::empty(),
        Some(a),
        mover,
        obstacle,
        ThinkTime::Seconds(42.0),
        &mut crate::modules::Driver::cache_only(),
    )
    .unwrap();
    assert_eq!(
        host.module_counts(A).unwrap().last_result,
        Some(super::super::ModuleResult::QuakeC([
            2 * stride,
            3 * stride,
            42.0f32.to_bits(),
        ]))
    );
    assert_eq!(host.module_counts(B).unwrap().calls, calls);
    release(&mut host, held).unwrap();
    assert_eq!(host.module_counts(A).unwrap().traps, 0);
    assert_eq!(host.module_counts(B).unwrap().traps, 0);
}

#[test]
fn foreign_reciprocal_impact_rebinds_other_after_first_callback_reuses_native_row() {
    let (mut host, [old_mover, obstacle, _], stride) = fixture();
    // Original edict allocation permits immediate reuse during startup grace.
    host.time = EventTime::from_milliseconds(0);
    set_touch(&mut host, A, 13);
    set_touch(&mut host, B, 2);
    let mover = capture(&host, old_mover).unwrap();
    let obstacle_ref = capture(&host, obstacle).unwrap();
    impact(
        &mut host,
        A,
        TraceAccess::empty(),
        Some(old_mover),
        mover,
        obstacle_ref,
        ThinkTime::Seconds(0.0),
        &mut crate::modules::Driver::cache_only(),
    )
    .unwrap();
    let replacement = host
        .runtime
        .server
        .entities
        .resolve_reference(mover)
        .unwrap();
    assert_ne!(replacement, old_mover);
    assert!(host.runtime.server.entities.resolve(old_mover).is_none());
    let Some(super::super::ModuleResult::QuakeC(words)) =
        host.module_counts(B).unwrap().last_result
    else {
        unreachable!()
    };
    assert_eq!(&words[..2], &[2 * stride, 3 * stride]);
    let Program::QuakeC(qc) = &host.module_row(B).unwrap().request.program else {
        unreachable!()
    };
    assert_eq!(
        qc.vm
            .resolve_entity(&host.runtime.server.entities, B, words[1])
            .unwrap(),
        Some(replacement)
    );
    assert_eq!(
        inspect(&mut host, B, 3),
        [81.0f32, 82.0, 83.0].map(f32::to_bits)
    );
    assert_eq!(host.module_counts(A).unwrap().traps, 0);
    assert_eq!(host.module_counts(B).unwrap().traps, 0);
}

#[test]
fn native_private_trigger_is_counted_without_hiding_later_qc_recipient() {
    let (mut host, [a, b, native], _) = fixture();
    let position = host.runtime.server.entities.columns.position[a.slot as usize];
    let Program::QuakeC(qc) = &mut host.module_row_mut(B).unwrap().request.program else {
        unreachable!()
    };
    let origin = qc.vm.field_address(2 * qc.vm.stride as u32, 0, 3).unwrap();
    let solid = qc.vm.field_address(2 * qc.vm.stride as u32, 24, 1).unwrap();
    qc.vm.entities.write_vec3(origin, position).unwrap();
    qc.vm
        .entities
        .write_word(solid, 1.0f32.to_bits() as i32)
        .unwrap();
    host.call_module_export(B, Export::clocked(CallbackId(1)), ThinkTime::Seconds(0.0))
        .unwrap();
    host.runtime
        .server
        .entities
        .columns
        .set_position(native.slot as usize, position);
    for (entity, flags) in [
        (a, LinkFlags::SOLID),
        (native, LinkFlags::TRIGGER),
        (b, LinkFlags::TRIGGER),
    ] {
        assert!(host.runtime.server.area.link(
            &host.runtime.server.entities,
            entity,
            flags,
            LinkOrder::Tail,
            LinkIntent::Explicit
        ));
    }
    let bounds = host.runtime.server.area.bounds(native).unwrap();
    let relinks = host.runtime.server.area.relinks;
    let calls = host.module_counts(B).unwrap().calls;
    triggers(
        &mut host,
        A,
        TraceAccess::empty(),
        a,
        ThinkTime::Seconds(42.0),
        &mut crate::modules::Driver::cache_only(),
    )
    .unwrap();
    assert_eq!(host.module_counts(A).unwrap().unsupported_contacts, 1);
    assert_eq!(host.module_counts(B).unwrap().calls, calls + 1);
    assert_eq!(host.module_counts(B).unwrap().traps, 0);
    assert_eq!(host.runtime.server.area.bounds(native), Some(bounds));
    assert_eq!(host.runtime.server.area.relinks, relinks);
}

#[test]
fn qc_contact_reads_published_native_body_without_importing_alias_back() {
    let (mut host, [_, b, native], _) = fixture();
    let b_capture = capture(&host, b).unwrap();

    let count = host.runtime.server.entities.len();
    let other = capture(&host, native).unwrap();
    let body = host
        .runtime
        .server
        .entities
        .columns
        .body(native.slot as usize);
    touch(
        &mut host,
        A,
        TraceAccess::empty(),
        b_capture,
        other,
        None,
        false,
        false,
        &mut crate::modules::Driver::cache_only(),
    )
    .unwrap();
    assert_eq!(
        inspect(&mut host, B, 3),
        [70.0f32, 80.0, 90.0].map(f32::to_bits)
    );
    let current = host
        .runtime
        .server
        .entities
        .columns
        .body(native.slot as usize);
    assert_eq!(current.position, body.position);
    assert_eq!(current.velocity, body.velocity);
    assert_eq!(current.mins, body.mins);
    assert_eq!(current.maxs, body.maxs);
    assert_eq!(
        host.runtime.server.entities.columns.owner[native.slot as usize],
        NATIVE
    );
    assert_eq!(host.runtime.server.entities.len(), count);
}

#[test]
fn rejected_prepared_foreign_touch_releases_its_alias_lease() {
    let (mut host, [a, b, native], stride) = fixture();
    let b_capture = capture(&host, b).unwrap();

    let other = capture(&host, a).unwrap();
    let prepared = prepare_touch(
        &mut host,
        A,
        TraceAccess::empty(),
        b_capture,
        other,
        None,
        false,
        false,
    )
    .unwrap()
    .unwrap();
    host.runtime.server.entities.columns.owner[b.slot as usize] = NATIVE;
    assert!(
        prepared
            .invoke(&mut host, &mut crate::modules::Driver::cache_only())
            .is_err()
    );
    host.runtime.server.entities.columns.owner[b.slot as usize] = B;
    let native_capture = capture(&host, native).unwrap();
    let native = native_capture.reference().unwrap();
    assert!(matches!(
        admit(&mut host, B, native),
        Err(ClientCommandError::Usercmd(
            qa_compat::services::CallError::Capacity
        ))
    ));
    assert!(host.runtime.server.free_entity(a, ThinkTime::Seconds(3.0)));
    let next = admit(&mut host, B, native).unwrap();
    assert_eq!(next.offset(), 3 * stride);
    release(&mut host, next).unwrap();
    touch(
        &mut host,
        A,
        TraceAccess::empty(),
        b_capture,
        native_capture,
        None,
        false,
        false,
        &mut crate::modules::Driver::cache_only(),
    )
    .unwrap();
    assert_eq!(
        inspect(&mut host, B, 3),
        [70.0f32, 80.0, 90.0].map(f32::to_bits)
    );
    assert_eq!(host.module_counts(B).unwrap().traps, 0);
}

#[test]
fn captured_retired_foreign_mover_keeps_weak_other_at_reciprocal_impact() {
    for reuse in [false, true] {
        let (mut host, [a, b, _], stride) = fixture();
        set_touch(&mut host, B, 6);
        let mover = capture(&host, a).unwrap();
        let obstacle = capture(&host, b).unwrap();
        assert!(host.runtime.server.free_entity(a, ThinkTime::Seconds(0.0)));
        let replacement = if reuse {
            let id = host
                .runtime
                .server
                .entities
                .allocate(ThinkTime::Seconds(3.0), A, AllocationPolicy::EDICT)
                .unwrap()
                .id;
            host.runtime.server.entities.columns.native_entity[id.slot as usize] =
                Some(NativeEntity { module: A, slot: 2 });
            Some(id)
        } else {
            None
        };
        impact(
            &mut host,
            A,
            TraceAccess::empty(),
            None,
            mover,
            obstacle,
            ThinkTime::Seconds(42.0),
            &mut crate::modules::Driver::cache_only(),
        )
        .unwrap();
        let Some(super::super::ModuleResult::QuakeC(words)) =
            host.module_counts(B).unwrap().last_result
        else {
            unreachable!()
        };
        assert_eq!(&words[..2], &[2 * stride, 3 * stride]);
        let Program::QuakeC(qc) = &host.module_row(B).unwrap().request.program else {
            unreachable!()
        };
        assert_eq!(
            qc.vm
                .resolve_entity(&host.runtime.server.entities, B, words[1])
                .unwrap(),
            replacement
        );
    }
}

fn anonymous(host: &mut FrameHost, position: Vec3) -> EntityId {
    let entity = host
        .runtime
        .server
        .entities
        .allocate(
            ThinkTime::Seconds(0.0),
            ModuleId(9),
            AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    host.runtime.server.entities.columns.set_body(
        entity.slot as usize,
        Body {
            position,
            mins: Vec3([-1.0; 3]),
            maxs: Vec3([1.0; 3]),
            ..Default::default()
        },
    );
    assert!(host.runtime.server.entities.columns.native_entity[entity.slot as usize].is_none());
    entity
}

#[test]
fn impact_admits_anonymous_other_pose_and_skips_its_absent_native_callback() {
    let (mut host, [actor, _, _], stride) = fixture();
    let pose = Vec3([15.0, 25.0, 35.0]);
    let obstacle = anonymous(&mut host, pose);
    let mover = capture(&host, actor).unwrap();
    let other = capture(&host, obstacle).unwrap();
    assert_eq!(other.reference(), None);
    let entities = host.runtime.server.entities.len();
    impact(
        &mut host,
        A,
        TraceAccess::empty(),
        Some(actor),
        mover,
        other,
        ThinkTime::Seconds(42.0),
        &mut crate::modules::Driver::cache_only(),
    )
    .unwrap();
    assert_eq!(
        host.module_counts(A).unwrap().last_result,
        Some(super::super::ModuleResult::QuakeC([
            2 * stride,
            3 * stride,
            42.0f32.to_bits()
        ]))
    );
    assert_eq!(inspect(&mut host, A, 3), pose.0.map(f32::to_bits));
    assert_eq!(
        host.runtime.server.entities.columns.position[obstacle.slot as usize],
        pose
    );
    assert_eq!(host.runtime.server.entities.len(), entities);
    assert_eq!(host.module_counts(A).unwrap().unsupported_contacts, 0);
    assert_eq!(host.module_counts(A).unwrap().traps, 0);
}

#[test]
fn retired_anonymous_other_traps_on_read_without_retargeting_reused_common_slot() {
    let (mut host, [_, recipient, _], stride) = fixture();
    let recipient_capture = capture(&host, recipient).unwrap();

    let old = anonymous(&mut host, Vec3([15.0, 25.0, 35.0]));
    let original = capture(&host, old).unwrap();
    let prepared = prepare_touch(
        &mut host,
        A,
        TraceAccess::empty(),
        recipient_capture,
        original,
        None,
        false,
        false,
    )
    .unwrap()
    .unwrap();
    assert!(
        host.runtime
            .server
            .free_entity(old, ThinkTime::Seconds(0.0))
    );
    let fresh_pose = Vec3([55.0, 65.0, 75.0]);
    let fresh = anonymous(&mut host, fresh_pose);
    assert_eq!(fresh.slot, old.slot);
    assert_ne!(fresh, old);
    assert_eq!(
        host.runtime.server.entities.resolve_reference(original),
        None
    );
    assert!(
        prepared
            .invoke(&mut host, &mut crate::modules::Driver::cache_only())
            .is_err()
    );
    assert_eq!(host.module_counts(B).unwrap().traps, 1);
    let Program::QuakeC(qc) = &host.module_row(B).unwrap().request.program else {
        unreachable!()
    };
    let other = qc.vm.image.globals[71];
    assert_eq!(other, 3 * stride);
    assert_eq!(
        qc.vm
            .resolve_entity(&host.runtime.server.entities, B, other)
            .unwrap(),
        None
    );
    let current = capture(&host, fresh).unwrap();
    touch(
        &mut host,
        A,
        TraceAccess::empty(),
        recipient_capture,
        current,
        None,
        false,
        false,
        &mut crate::modules::Driver::cache_only(),
    )
    .unwrap();
    assert_eq!(inspect(&mut host, B, 3), fresh_pose.0.map(f32::to_bits));
    assert_eq!(host.module_counts(B).unwrap().traps, 1);
}

#[test]
fn anonymous_published_callback_is_resolved_before_missing_native_self_is_counted() {
    let (mut host, [actor, _, _], _) = fixture();
    let obstacle = anonymous(&mut host, Vec3([15.0, 25.0, 35.0]));
    host.runtime.server.entities.columns.owner[obstacle.slot as usize] = B;
    host.runtime.server.entities.columns.touch[obstacle.slot as usize] = Some(CallbackId(2));
    let mover = capture(&host, actor).unwrap();
    let other = capture(&host, obstacle).unwrap();
    impact(
        &mut host,
        A,
        TraceAccess::empty(),
        Some(actor),
        mover,
        other,
        ThinkTime::Seconds(42.0),
        &mut crate::modules::Driver::cache_only(),
    )
    .unwrap();
    assert_eq!(host.module_counts(A).unwrap().unsupported_contacts, 1);
    assert_eq!(
        inspect(&mut host, A, 3),
        [15.0f32, 25.0, 35.0].map(f32::to_bits)
    );
    host.runtime.server.entities.columns.touch[obstacle.slot as usize] = Some(CallbackId(u32::MAX));
    assert!(resolve_touch(&mut host, A, obstacle, None, None, false).is_err());
}
