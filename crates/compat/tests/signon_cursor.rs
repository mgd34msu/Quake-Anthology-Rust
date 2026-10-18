use qa_compat::services::{
    AmbientSound, CallContext, CallError, ENGINE_CALLS, EngineServices, ServiceStorage,
    SignonCursor,
};
use qa_console::{command_buffer::CommandBuffer, cvars::Cvars, views::Context};
use qa_core::{
    configstrings::ConfigLayout,
    events::{EventRing, FrameEvent, OutputRecord, OutputSubmission, OutputTarget},
    names::NameTable,
    primitives::*,
};
use qa_session::clients::{Connection, Server};
use qa_world::{area::LinkOrder, collision::CollisionStore, entities::AllocationPolicy};
use std::{num::NonZeroU32, sync::Arc};

#[path = "support/userinfo.rs"]
mod userinfo;

fn bank() -> Arc<qa_audio::Bank> {
    let rate = NonZeroU32::new(44100).unwrap();
    Arc::new(
        qa_audio::Bank::load(
            rate,
            vec![(
                "sound/loop.wav".into(),
                Pcm {
                    rate,
                    channels: PcmChannels::Mono,
                    samples: vec![1000, -1000],
                    loop_start: Some(0),
                    loop_end: None,
                    loop_invalid: false,
                },
            )],
        )
        .unwrap(),
    )
}

fn with_services(run: impl FnOnce(&mut EngineServices<'_>, CallContext, u32)) {
    let mut server = Server::load(2, 16, 1, 1, 1, 1).unwrap();
    server.events = EventRing::load(8, 4, 4, 64).unwrap();
    server.presentation = server.events.bind(OutputTarget::Presentation).unwrap();
    let mut cvars = Cvars::new().unwrap();
    let mut commands = CommandBuffer::new();
    let mut input = qa_input::Input::load();
    let mut vfs = qa_content::vfs::Vfs::default();
    let localization = qa_content::localization::Catalog::default();
    let mut names = NameTable::load(qa_gameplay::registry::Registry::names_needed()).unwrap();
    let registry = qa_gameplay::registry::Registry::load(&names).unwrap();
    let context = CallContext {
        module: ModuleId(7),
        clock: ThinkTime::Seconds(1.0),
        server_frame: 41,
        native_rules: RuleSetId::QuakeWorld,
        trace_rules: RuleSetId::Quake2,
        sound_policy: qa_gameplay::rules::sound_policy(RuleSetId::Quake3),
        console: Context::default(),
        allocation: AllocationPolicy::EDICT,
        link_order: LinkOrder::Tail,
    };
    let layout = ConfigLayout::for_rules(context.native_rules);
    let mut storage = ServiceStorage::load_native(
        &[(context.module, layout.count, context.native_rules)],
        0,
        &cvars,
    )
    .unwrap();
    let anchor = server
        .entities
        .allocate(context.clock, context.module, context.allocation)
        .unwrap()
        .id;
    storage.bind_lifetime(context.module, anchor).unwrap();
    let geometry = CollisionStore::new();
    let mut scratch = geometry.scratch();
    let mut services = EngineServices {
        server: &mut server,
        callbacks: None,
        client_disconnect: None,
        input: &mut input,
        editor: None,
        cvars: &mut cvars,
        commands: &mut commands,
        client_connection: None,
        client_collision: None,
        renderer: None,
        scene: None,
        client_audio: None,
        vfs: &mut vfs,
        model_loader: None,
        model_lookup: None,
        sound_loader: None,
        sound_bank: None,
        localization: &localization,
        registry: &registry,
        names: &mut names,
        storage: &mut storage,
        geometry: &geometry,
        world: None,
        native_geometry: None,
        foreign_trace: None,
        visibility: None,
        portals: None,
        scratch: &mut scratch,
    };
    let ordinal = (ENGINE_CALLS.resource_index)(
        &mut services,
        context.module,
        layout.resources[1],
        b"loop.wav",
    )
    .unwrap();
    run(&mut services, context, ordinal);
}

fn register(services: &mut EngineServices<'_>, context: CallContext, ordinal: u32, x: i16) {
    (ENGINE_CALLS.ambient)(
        services,
        context,
        AmbientSound {
            source_ordinal: ordinal,
            origin: [x, 0, 0],
            volume: 255,
            attenuation: 64,
        },
    )
    .unwrap();
}

fn connect(server: &mut Server) -> ClientId {
    server
        .connect(Connection::Remote, ModuleId(1), None)
        .unwrap()
}

fn drain(server: &mut Server, client: ClientId) -> Vec<OutputRecord> {
    let output = server.clients[client.0 as usize].output.unwrap();
    let mut records = Vec::new();
    let mut batch = server.events.batch(output).unwrap();
    while let Some(record) = server.events.next(&mut batch) {
        records.push(record);
        assert!(
            server
                .events
                .submit(output, record.sequence, OutputSubmission::BestEffort)
        );
    }
    records
}

fn register_static(services: &mut EngineServices<'_>, context: CallContext, x: f32) {
    (ENGINE_CALLS.static_entity)(
        services,
        context,
        EntityRecord {
            position: Vec3([x, 0.0, 0.0]),
            ..EntityRecord::default()
        },
    )
    .unwrap();
}

#[test]
fn static_only_signon_has_no_audio_dependency_and_fixes_each_recipient_extent() {
    with_services(|services, context, _| {
        let first = connect(services.server);
        let mut first_cursor = SignonCursor::default();
        register_static(services, context, 8.0);
        assert_eq!(
            services.storage.publish_signon(
                services.server,
                context,
                first,
                &mut first_cursor,
                None
            ),
            Ok(1)
        );
        let records = drain(services.server, first);
        assert!(matches!(records[0].event, FrameEvent::StaticEntity(value)
            if value.position == Vec3([8.0, 0.0, 0.0])));
        register_static(services, context, 16.0);
        assert_eq!(
            services.storage.publish_signon(
                services.server,
                context,
                first,
                &mut first_cursor,
                None
            ),
            Ok(0)
        );
        let second = connect(services.server);
        assert_eq!(
            services.storage.publish_signon(
                services.server,
                context,
                second,
                &mut SignonCursor::default(),
                None
            ),
            Ok(2)
        );
        assert_eq!(drain(services.server, second).len(), 2);
        assert!(drain(services.server, first).is_empty());
    });
}

#[test]
fn bank_free_static_prefix_preserves_pending_ambient_order_until_bank_attachment() {
    with_services(|services, context, ordinal| {
        let client = connect(services.server);
        let mut cursor = SignonCursor::default();
        register_static(services, context, 8.0);
        register(services, context, ordinal, 16);
        assert_eq!(
            services
                .storage
                .publish_signon(services.server, context, client, &mut cursor, None),
            Ok(1)
        );
        assert!(matches!(
            drain(services.server, client)[0].event,
            FrameEvent::StaticEntity(_)
        ));
        assert_eq!(
            services
                .storage
                .publish_signon(services.server, context, client, &mut cursor, None),
            Ok(0)
        );
        // The static prefix already fixed this recipient's extent; later rows
        // cannot leak into it when the pending ambient becomes bank-backed.
        register_static(services, context, 24.0);
        let bank = bank();
        services
            .storage
            .bind_sound_bank(
                context.module,
                ConfigLayout::for_rules(context.native_rules).resources[1],
                bank.as_ref(),
                Some(qa_audio::SoundSource::DEFAULT),
            )
            .unwrap();
        assert_eq!(
            services.storage.publish_signon(
                services.server,
                context,
                client,
                &mut cursor,
                Some(bank.as_ref())
            ),
            Ok(1)
        );
        let sound = match drain(services.server, client)[0].event {
            FrameEvent::Sound(value) => value,
            _ => panic!("ambient after static prefix"),
        };
        assert_eq!(sound.position, Vec3([2.0, 0.0, 0.0]));
        assert_eq!(sound.action, SoundAction::StartLoop);
        assert_eq!(
            services.storage.publish_signon(
                services.server,
                context,
                client,
                &mut cursor,
                Some(bank.as_ref())
            ),
            Ok(0)
        );
    });
}

#[test]
fn bank_backed_extent_is_fixed_and_new_connection_and_module_lifetimes_restart_it() {
    with_services(|services, context, ordinal| {
        let first = connect(services.server);
        let old_connection = services.server.clients[first.0 as usize].entity;
        let mut first_cursor = SignonCursor::default();
        register(services, context, ordinal, 8);
        assert_eq!(
            services.storage.publish_signon(
                services.server,
                context,
                first,
                &mut first_cursor,
                None
            ),
            Ok(0)
        );
        register(services, context, ordinal, 16);
        let bank = bank();
        services
            .storage
            .bind_sound_bank(
                context.module,
                ConfigLayout::for_rules(context.native_rules).resources[1],
                bank.as_ref(),
                Some(qa_audio::SoundSource::DEFAULT),
            )
            .unwrap();
        assert_eq!(
            services.storage.publish_signon(
                services.server,
                context,
                first,
                &mut first_cursor,
                Some(bank.as_ref())
            ),
            Ok(2)
        );
        let records = drain(services.server, first);
        assert_eq!(records.len(), 2);
        for record in records {
            let delivery = record.delivery.unwrap();
            assert_eq!(delivery.target, NativeTarget::Client(first));
            assert_eq!(delivery.emission.unwrap().provider_frame, 41);
            assert!(delivery.reliable);
        }
        assert!(services.server.events.is_empty());
        register(services, context, ordinal, 24);
        assert_eq!(
            services.storage.publish_signon(
                services.server,
                context,
                first,
                &mut first_cursor,
                Some(bank.as_ref())
            ),
            Ok(0)
        );
        let second = connect(services.server);
        let mut second_cursor = SignonCursor::default();
        assert_eq!(
            services.storage.publish_signon(
                services.server,
                context,
                second,
                &mut second_cursor,
                Some(bank.as_ref())
            ),
            Ok(3)
        );
        assert_eq!(drain(services.server, second).len(), 3);
        assert!(drain(services.server, first).is_empty());
        assert_eq!(
            services.storage.publish_signon(
                services.server,
                context,
                second,
                &mut first_cursor,
                Some(bank.as_ref())
            ),
            Err(CallError::Entity)
        );
        assert!(services.server.disconnect(first));
        userinfo::publish(services.server, first);
        let retired = drain(services.server, second);
        assert!(matches!(
            retired.as_slice(),
            [OutputRecord {
                event: FrameEvent::PeerInfo { .. },
                ..
            }]
        ));
        let reused = connect(services.server);
        assert_eq!(reused, first);
        assert_ne!(
            services.server.clients[reused.0 as usize].entity,
            old_connection
        );
        assert_eq!(
            services.storage.publish_signon(
                services.server,
                context,
                reused,
                &mut first_cursor,
                Some(bank.as_ref())
            ),
            Ok(3)
        );
        assert_eq!(drain(services.server, reused).len(), 3);
        assert_eq!(
            services.storage.publish_signon(
                services.server,
                context,
                second,
                &mut second_cursor,
                Some(bank.as_ref())
            ),
            Ok(0)
        );
        let old_records = services.server.events.sequence_range();
        let anchor = services
            .server
            .entities
            .allocate(context.clock, context.module, context.allocation)
            .unwrap()
            .id;
        services
            .storage
            .bind_lifetime(context.module, anchor)
            .unwrap();
        register(services, context, ordinal, 32);
        assert_eq!(
            services.storage.publish_signon(
                services.server,
                context,
                reused,
                &mut first_cursor,
                Some(bank.as_ref())
            ),
            Ok(1)
        );
        let record = drain(services.server, reused).pop().unwrap();
        assert_eq!(
            record.delivery.unwrap().emission.unwrap().module_lifetime,
            anchor
        );
        assert!(record.sequence >= old_records.end);
        assert_eq!(
            services.storage.publish_signon(
                services.server,
                context,
                second,
                &mut second_cursor,
                Some(bank.as_ref())
            ),
            Ok(1)
        );
        assert_eq!(drain(services.server, second).len(), 1);
    });
}

#[test]
fn one_resync_recipient_does_not_consume_its_extent_or_block_another_recipient() {
    with_services(|services, context, ordinal| {
        let stalled = connect(services.server);
        let healthy = connect(services.server);
        let output = services.server.clients[stalled.0 as usize].output.unwrap();
        for _ in 0..=services.server.events.capacity() {
            services
                .server
                .events
                .push_to(
                    OutputTarget::Client(stalled),
                    FrameEvent::Sound(SoundEvent::default()),
                )
                .unwrap();
        }
        assert!(services.server.events.needs_resync(output));
        register(services, context, ordinal, 8);
        let bank = bank();
        services
            .storage
            .bind_sound_bank(
                context.module,
                ConfigLayout::for_rules(context.native_rules).resources[1],
                bank.as_ref(),
                Some(qa_audio::SoundSource::DEFAULT),
            )
            .unwrap();
        let mut stalled_cursor = SignonCursor::default();
        let mut healthy_cursor = SignonCursor::default();
        assert_eq!(
            services.storage.publish_signon(
                services.server,
                context,
                stalled,
                &mut stalled_cursor,
                Some(bank.as_ref())
            ),
            Ok(0)
        );
        assert_eq!(
            services.storage.publish_signon(
                services.server,
                context,
                healthy,
                &mut healthy_cursor,
                Some(bank.as_ref())
            ),
            Ok(1)
        );
        assert_eq!(drain(services.server, healthy).len(), 1);
        register(services, context, ordinal, 16);
        assert_eq!(
            services.storage.publish_signon(
                services.server,
                context,
                healthy,
                &mut healthy_cursor,
                Some(bank.as_ref())
            ),
            Ok(0)
        );
        let resumed = services.server.events.resume(output).unwrap();
        services.server.clients[stalled.0 as usize].output = Some(resumed);
        assert_eq!(
            services.storage.publish_signon(
                services.server,
                context,
                stalled,
                &mut stalled_cursor,
                Some(bank.as_ref())
            ),
            Ok(2)
        );
        assert_eq!(drain(services.server, stalled).len(), 2);
        assert_eq!(
            services.storage.publish_signon(
                services.server,
                context,
                stalled,
                &mut stalled_cursor,
                Some(bank.as_ref())
            ),
            Ok(0)
        );
    });
}
