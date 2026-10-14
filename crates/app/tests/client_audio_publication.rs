#[path = "support/service_program.rs"]
mod service_program;

use qa_app::{
    Runtime,
    client_policy::ClientPolicy,
    host::{FrameHost, FrameSource},
    map::SpawnAnchor,
    modules::{Export, ModuleRequest, Phase, Program},
};
use qa_compat::{
    abi::Q3_CLIENT,
    services::{CallContext, CallError, ClientAudio},
};
use qa_console::{commands::Console, views::Context};
use qa_core::{
    events::{FrameEvent, OutputTarget},
    primitives::*,
    sys_events::*,
};
use qa_formats::program::qvm::Opcode::*;
use qa_network::commands::packet::Protocol;
use qa_session::timing::TickRate;
use qa_world::entities::AllocationPolicy;
use std::time::Duration;

#[derive(Default)]
struct Source {
    last: Option<SoundEvent>,
    resets: usize,
    plays: usize,
    expected: Vec<(ClientId, Vec3, ModuleId)>,
}
impl FrameSource for Source {
    fn begin_frame(&mut self) -> Duration {
        Duration::ZERO
    }
    fn poll_events(&mut self, _: &mut SysEventQueue) {}
    fn wait_time(&mut self, _: Duration) -> Duration {
        Duration::ZERO
    }
    fn elapsed(&self) -> Duration {
        Duration::ZERO
    }
    fn present(&mut self) {}
    fn sound(&mut self, event: SoundEvent) -> bool {
        self.resets += usize::from(event.action == SoundAction::ResetClientLoops);
        if event.action == SoundAction::Play {
            self.plays += 1;
            if !self.expected.is_empty() {
                let &(client, position, module) = self
                    .expected
                    .iter()
                    .find(|row| Some(row.0) == event.client)
                    .unwrap();
                assert_eq!(
                    event.entity,
                    Some(SoundEmitter::Native {
                        namespace: ResourceNamespace::Client(client),
                        slot: 0,
                    })
                );
                assert_eq!(event.position, position);
                assert_eq!(event.listener_owner, Some(module));
                assert_eq!(event.source_ordinal, Some(23));
                assert_eq!(
                    event.spatial,
                    qa_gameplay::rules::native_sound_spatial(RuleSetId::QuakeWorld)
                );
                assert_eq!(event.queued_at, EventTime::from_milliseconds(123));
            }
        }
        self.last = Some(event);
        true
    }
}
fn fixture(count: usize) -> (Box<FrameHost>, Vec<ClientId>) {
    let mut runtime = Runtime::load(count, []).unwrap();
    let mut ids = Vec::new();
    let mut requests = Vec::new();
    for seat in 0..count {
        let id = runtime
            .connect_local(
                SeatId::new(seat as u8).unwrap(),
                SpawnAnchor {
                    position: Vec3::default(),
                    angles: Vec3::default(),
                    entity: 0,
                    fixture_fallback: false,
                },
                ClientPolicy {
                    client: RuleSetId::Quake2Rerelease,
                    movement: RuleSetId::Quake,
                    trace: RuleSetId::QuakeWorld,
                },
                if seat == 0 {
                    Protocol::NetQuake15
                } else {
                    Protocol::Quake2Repro1038
                },
                0,
            )
            .unwrap();
        ids.push(id);
        let module = ModuleId(seat as u16 + 1);
        let anchor = runtime
            .server
            .entities
            .allocate(ThinkTime::Milliseconds(0), module, AllocationPolicy::q3(0))
            .unwrap()
            .id;
        requests.push(ModuleRequest {
            context: CallContext {
                module,
                clock: ThinkTime::Milliseconds(0),
                server_frame: 0,
                native_rules: RuleSetId::Quake3,
                trace_rules: RuleSetId::Quake,
                sound_policy: qa_gameplay::rules::native_sound_spatial(RuleSetId::QuakeWorld),
                console: Context::default(),
                allocation: AllocationPolicy::q3(0),
                link_order: LinkOrder::Head,
            },
            phase: Phase::Client,
            timing_rules: RuleSetId::Quake3,
            rate: TickRate::FrameDriven,
            anchor,
            program: Program::Qvm {
                vm: Box::new(service_program::program(
                    &[(Enter, 64), (Const, 0), (Leave, 64)],
                    &[0; 128],
                )),
                imports: &Q3_CLIENT,
            },
            entries: vec![0],
            frame: Export {
                client: Some(id),
                ..Export::words(CallbackId(0), &[])
            },
            prepare: Vec::new(),
            initialize: Vec::new(),
            api: None,
            shutdown: Vec::new(),
            instruction_budget: 100,
            configstrings: 0,
            files: 0,
            entity_source: None,
        });
    }
    let mut host = FrameHost::load_modules(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        requests,
    )
    .unwrap();
    for (seat, &id) in ids.iter().enumerate() {
        host.local_clients[seat] = Some(id);
    }
    (Box::new(host), ids)
}
fn publish(
    host: &mut FrameHost,
    id: ClientId,
    event: SoundEvent,
    entity: Option<i32>,
) -> Result<(), CallError> {
    host.runtime.snapshots[id.0 as usize]
        .as_mut()
        .unwrap()
        .publish(&mut host.runtime.server.events, event, entity, true)
}
fn event(module: ModuleId, action: SoundAction) -> SoundEvent {
    SoundEvent {
        listener_owner: Some(module),
        action,
        sound: SoundId(9),
        source_ordinal: Some(23),
        volume: 1.,
        attenuation: 1.,
        queued_at: EventTime::from_milliseconds(123),
        spatial: qa_gameplay::rules::native_sound_spatial(RuleSetId::QuakeWorld),
        ..SoundEvent::default()
    }
}
fn dispatch(host: &mut FrameHost, source: &mut Source) {
    qa_app::output::dispatch(
        &mut host.runtime,
        source,
        &mut host.console.scrollback,
        &host.local_clients,
        EventTime::default(),
        3.,
        2.,
    );
}
#[test]
fn attach_receipt_preserves_new_native_origins_and_independent_client_scope() {
    let (mut host, ids) = fixture(2);
    let mut source = Source::default();
    for (seat, &id) in ids.iter().enumerate() {
        let mut sound = event(ModuleId(seat as u16 + 1), SoundAction::StartLoop);
        sound.position = Vec3([100. * (seat as f32 + 1.), 2., 3.]);
        sound.looping = Some(
            qa_gameplay::rules::loop_rules(RuleSetId::Quake3)
                .unwrap()
                .native(false),
        );
        publish(&mut host, id, sound, Some(0)).unwrap();
    }
    // The initial reset and new Add are drained together. A reset completion
    // must clear its old rows only, never these later successful publications.
    dispatch(&mut host, &mut source);
    assert_eq!(source.resets, 2);
    for (seat, &id) in ids.iter().enumerate() {
        let module = ModuleId(seat as u16 + 1);
        publish(&mut host, id, event(module, SoundAction::Play), Some(0)).unwrap();
        assert_eq!(
            publish(
                &mut host,
                id,
                event(ModuleId(99), SoundAction::Play),
                Some(0)
            ),
            Err(CallError::Entity)
        );
    }
    source.expected = ids
        .iter()
        .enumerate()
        .map(|(seat, &id)| {
            (
                id,
                Vec3([100. * (seat as f32 + 1.), 2., 3.]),
                ModuleId(seat as u16 + 1),
            )
        })
        .collect();
    dispatch(&mut host, &mut source);
    assert_eq!(source.plays, 2);
}
#[test]
fn positioned_signed_emitter_and_stale_metadata_do_not_read_origin_rows() {
    let (mut host, ids) = fixture(1);
    let id = ids[0];
    let mut source = Source::default();
    let mut sound = event(ModuleId(1), SoundAction::Play);
    sound.positioned = true;
    sound.position = Vec3([11., 22., 33.]);
    publish(&mut host, id, sound, Some(-7)).unwrap();
    let mut stale = sound;
    let lifetime = host.runtime.server.clients[id.0 as usize].entity;
    stale.client_lifetime = Some(EntityId {
        generation: lifetime.generation.wrapping_add(1),
        ..lifetime
    });
    assert_eq!(
        publish(&mut host, id, stale, Some(-7)),
        Err(CallError::Entity)
    );
    stale = sound;
    stale.client = Some(ClientId(99));
    assert_eq!(
        publish(&mut host, id, stale, Some(-7)),
        Err(CallError::Entity)
    );
    assert_eq!(
        publish(
            &mut host,
            id,
            event(ModuleId(1), SoundAction::Play),
            Some(-7)
        ),
        Err(CallError::Entity)
    );
    dispatch(&mut host, &mut source);
    let record = source.last.unwrap();
    assert_eq!(record.action, SoundAction::Play);
    assert_eq!(record.position, Vec3([11., 22., 33.]));
    assert_eq!(
        record.entity,
        Some(SoundEmitter::Native {
            namespace: ResourceNamespace::Client(id),
            slot: -7
        })
    );
    assert_eq!(source.plays, 1);
}
#[test]
fn failed_publication_is_counted_and_never_settles_native_origin() {
    let (mut host, ids) = fixture(1);
    let id = ids[0];
    assert!(
        host.runtime.client_resources[id.0 as usize].is_none(),
        "NQ cleanup cannot depend on a native configstring adapter"
    );
    let mut source = Source::default();
    dispatch(&mut host, &mut source);
    for _ in 0..=host.runtime.server.events.capacity() {
        host.runtime
            .server
            .events
            .push_to(
                OutputTarget::Presentation,
                FrameEvent::Sound(event(ModuleId(1), SoundAction::Play)),
            )
            .unwrap();
    }
    assert!(
        host.runtime
            .server
            .events
            .needs_resync(host.runtime.server.presentation)
    );
    let rejected = host.runtime.server.events.rejected_publications();
    let mut update = event(ModuleId(1), SoundAction::UpdateEmitterPosition);
    update.position = Vec3([777.; 3]);
    assert_eq!(
        publish(&mut host, id, update, Some(0)),
        Err(CallError::Capacity)
    );
    assert!(host.runtime.server.events.rejected_publications() > rejected);
    dispatch(&mut host, &mut source);
    let resets = source.resets;
    host.publish_client_loops();
    dispatch(&mut host, &mut source);
    assert_eq!(
        source.resets,
        resets + 1,
        "the resumed NQ owner retries its reset without configstrings"
    );
    publish(
        &mut host,
        id,
        event(ModuleId(1), SoundAction::Play),
        Some(0),
    )
    .unwrap();
    dispatch(&mut host, &mut source);
    assert_eq!(source.last.unwrap().position, Vec3::default());
}
