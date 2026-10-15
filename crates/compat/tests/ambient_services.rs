use qa_compat::services::{
    AmbientSound, CallContext, CallError, ENGINE_CALLS, EngineServices, SIGNON_BYTES,
    ServiceStorage, SignonCursor,
};
use qa_console::{command_buffer::CommandBuffer, cvars::Cvars, views::Context};
use qa_core::{
    events::{FrameEvent, OutputConsumerId, OutputRecord, OutputSubmission},
    names::NameTable,
    primitives::*,
};
use qa_session::clients::{Connection, Server};
use qa_world::{area::LinkOrder, collision::CollisionStore, entities::AllocationPolicy};
use std::{num::NonZeroU32, sync::Arc};

fn context(module: ModuleId, native_rules: RuleSetId) -> CallContext {
    CallContext {
        module,
        clock: ThinkTime::Seconds(1.0),
        server_frame: 17,
        native_rules,
        trace_rules: RuleSetId::Quake2,
        sound_policy: qa_gameplay::rules::sound_policy(RuleSetId::Quake3),
        console: Context {
            source: RuleSetId::Quake2Rerelease,
            ..Context::default()
        },
        allocation: AllocationPolicy::EDICT,
        link_order: LinkOrder::Tail,
    }
}

fn with_services(run: impl FnOnce(&mut EngineServices<'_>, [CallContext; 2])) {
    let mut server = Server::load(2, 16, 1, 1, 1, 1).unwrap();
    let mut cvars = Cvars::new().unwrap();
    let mut input = qa_input::Input::load();
    let mut commands = CommandBuffer::new();
    let vfs = qa_content::vfs::Vfs::default();
    let localization = qa_content::localization::Catalog::default();
    let mut names = NameTable::load(qa_gameplay::registry::Registry::names_needed()).unwrap();
    let registry = qa_gameplay::registry::Registry::load(&names).unwrap();
    let contexts = [
        context(ModuleId(1), RuleSetId::Quake),
        context(ModuleId(2), RuleSetId::QuakeWorld),
    ];
    let configs = contexts.map(|context| {
        (
            context.module,
            qa_core::configstrings::ConfigLayout::for_rules(context.native_rules).count,
            context.native_rules,
        )
    });
    let mut storage = ServiceStorage::load_native(&configs, 0, &cvars).unwrap();
    for context in contexts {
        let lifetime = server
            .entities
            .allocate(context.clock, context.module, context.allocation)
            .unwrap()
            .id;
        storage.bind_lifetime(context.module, lifetime).unwrap();
    }
    let geometry = CollisionStore::new();
    let mut scratch = geometry.scratch();
    run(
        &mut EngineServices {
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
            vfs: &vfs,
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
            visibility: None,
            portals: None,
            scratch: &mut scratch,
        },
        contexts,
    );
}

fn bank() -> Arc<qa_audio::Bank> {
    let rate = NonZeroU32::new(44100).unwrap();
    Arc::new(
        qa_audio::Bank::load(
            rate,
            [
                ("sound/misc/loop.wav", Some(0)),
                ("sound/misc/late.wav", Some(1)),
                ("sound/misc/nonloop.wav", None),
            ]
            .into_iter()
            .map(|(name, loop_start)| {
                (
                    name.into(),
                    Pcm {
                        rate,
                        channels: PcmChannels::Mono,
                        samples: vec![1000, 2000, 1000],
                        loop_start,
                        loop_end: None,
                        loop_invalid: false,
                    },
                )
            })
            .collect(),
        )
        .unwrap(),
    )
}

fn precache(services: &mut EngineServices<'_>, context: CallContext, name: &[u8]) -> u32 {
    let sounds = services.storage.resources(context.module).unwrap()[1];
    (ENGINE_CALLS.resource_index)(services, context.module, sounds, name).unwrap()
}
fn attach(services: &mut EngineServices<'_>, context: CallContext, bank: &Arc<qa_audio::Bank>) {
    let sounds = services.storage.resources(context.module).unwrap()[1];
    services
        .storage
        .bind_sound_bank(context.module, sounds, Arc::clone(bank))
        .unwrap();
}
fn descriptor(source_ordinal: u32) -> AmbientSound {
    AmbientSound {
        source_ordinal,
        origin: [-1, 2, 27],
        volume: 129,
        attenuation: 65,
    }
}
fn recipient(services: &mut EngineServices<'_>, id: ClientId) -> OutputConsumerId {
    while services.server.clients[id.0 as usize].connection.is_none() {
        services
            .server
            .connect(Connection::Remote, ModuleId::default(), None)
            .unwrap();
    }
    services.server.clients[id.0 as usize].output.unwrap()
}
fn drain(services: &mut EngineServices<'_>, consumer: OutputConsumerId) -> Vec<OutputRecord> {
    let mut records = Vec::new();
    let mut batch = services.server.events.batch(consumer).unwrap();
    while let Some(record) = services.server.events.next(&mut batch) {
        records.push(record);
        assert!(services.server.events.submit(
            consumer,
            record.sequence,
            OutputSubmission::BestEffort
        ));
    }
    records
}
fn sound(record: OutputRecord) -> SoundEvent {
    let FrameEvent::Sound(sound) = record.event else {
        panic!("sound record")
    };
    sound
}

#[test]
fn cold_registration_waits_for_real_loop_binding_and_keeps_independent_source_roles() {
    with_services(|services, [mut context, _]| {
        let mut initial: [SignonCursor; 2] = std::array::from_fn(|_| SignonCursor::default());
        let consumer = recipient(services, ClientId(0));
        let ordinal = precache(services, context, b"misc/loop.wav");
        assert_eq!(
            (ENGINE_CALLS.ambient)(services, context, descriptor(ordinal)),
            Ok(0)
        );
        assert_eq!(
            services.storage.publish_signon(
                services.server,
                context,
                ClientId(0),
                &mut initial[context.module.0 as usize - 1]
            ),
            Ok(0)
        );
        assert!(drain(services, consumer).is_empty());
        let bank = bank();
        attach(services, context, &bank);
        context.sound_policy = qa_gameplay::rules::sound_policy(RuleSetId::Quake2);
        context.console.source = RuleSetId::QuakeWorld;
        assert_eq!(
            services.storage.publish_signon(
                services.server,
                context,
                ClientId(0),
                &mut initial[context.module.0 as usize - 1]
            ),
            Ok(1)
        );
        assert_eq!(
            services.storage.publish_signon(
                services.server,
                context,
                ClientId(0),
                &mut initial[context.module.0 as usize - 1]
            ),
            Ok(0)
        );
        let records = drain(services, consumer);
        assert_eq!(records.len(), 1);
        let record = records[0];
        let event = sound(record);
        assert_eq!(event.sound, bank.find("sound/misc/loop.wav").unwrap());
        assert_eq!(event.source_ordinal, Some(ordinal));
        assert_eq!(
            event.spatial,
            qa_gameplay::rules::sound_policy(RuleSetId::Quake3)
        );
        assert_eq!(event.action, SoundAction::StartLoop);
        assert_eq!(
            event.position.0.map(f32::to_bits),
            [-0.125f32, 0.25, 3.375].map(f32::to_bits)
        );
        assert_eq!(event.volume.to_bits(), (129.0f32 / 255.0).to_bits());
        assert_eq!(event.attenuation.to_bits(), (65.0f32 / 64.0).to_bits());
        let delivery = record.delivery.unwrap();
        let lifetime = delivery.emission.unwrap().module_lifetime;
        assert!(services.server.entities.resolve(lifetime).is_some());
        assert_eq!(delivery.source, RuleSetId::Quake);
        assert_eq!(delivery.target, NativeTarget::Client(ClientId(0)));
        assert_eq!(delivery.emission.unwrap().provider_frame, 17);
        assert_eq!(
            event.entity,
            Some(SoundEmitter::Static {
                namespace: ResourceNamespace::Module(context.module),
                lifetime,
                slot: 0,
            })
        );
        let presentation = services.server.presentation;
        assert!(drain(services, presentation).is_empty());
    });
}

#[test]
fn post_attachment_registrations_are_only_for_explicit_late_client_signon() {
    with_services(|services, [context, _]| {
        let mut initial: [SignonCursor; 2] = std::array::from_fn(|_| SignonCursor::default());
        let first = recipient(services, ClientId(0));
        let ordinal = precache(services, context, b"misc/loop.wav");
        (ENGINE_CALLS.ambient)(services, context, descriptor(ordinal)).unwrap();
        let bank = bank();
        attach(services, context, &bank);
        assert_eq!(
            services.storage.publish_signon(
                services.server,
                context,
                ClientId(0),
                &mut initial[context.module.0 as usize - 1]
            ),
            Ok(1)
        );
        assert_eq!(drain(services, first).len(), 1);
        let late = precache(services, context, b"misc/late.wav");
        assert_eq!(
            (ENGINE_CALLS.ambient)(services, context, descriptor(late)),
            Ok(1)
        );
        // Reattaching the bank cannot extend an already bound recipient cursor.
        attach(services, context, &bank);
        assert_eq!(
            services.storage.publish_signon(
                services.server,
                context,
                ClientId(0),
                &mut initial[context.module.0 as usize - 1]
            ),
            Ok(0)
        );
        assert!(drain(services, first).is_empty());
        let second = recipient(services, ClientId(1));
        let mut cursor = SignonCursor::default();
        assert_eq!(
            services
                .storage
                .publish_signon(services.server, context, ClientId(1), &mut cursor),
            Ok(2)
        );
        assert_eq!(
            services
                .storage
                .publish_signon(services.server, context, ClientId(1), &mut cursor),
            Ok(0)
        );
        assert!(drain(services, first).is_empty());
        let records = drain(services, second);
        assert_eq!(records.len(), 2);
        assert_eq!(
            sound(records[1]).sound,
            bank.find("sound/misc/late.wav").unwrap()
        );
        assert!(
            records
                .iter()
                .all(|r| r.delivery.unwrap().target == NativeTarget::Client(ClientId(1)))
        );
        assert_eq!(
            services
                .storage
                .publish_signon(services.server, context, ClientId(0), &mut cursor),
            Err(CallError::Entity)
        );
        assert!(services.server.disconnect(ClientId(0)));
        let first = recipient(services, ClientId(0));
        let mut other = SignonCursor::default();
        assert_eq!(
            services
                .storage
                .publish_signon(services.server, context, ClientId(0), &mut other),
            Ok(2)
        );
        assert_eq!(drain(services, first).len(), 2);
        assert!(drain(services, second).is_empty());
    });
}

#[test]
fn unresolved_or_nonloop_samples_never_gain_fake_handles_and_module_slots_do_not_alias() {
    with_services(|services, contexts| {
        let mut initial: [SignonCursor; 2] = std::array::from_fn(|_| SignonCursor::default());
        let consumer = recipient(services, ClientId(0));
        for context in contexts {
            for name in [
                b"misc/missing.wav".as_slice(),
                b"misc/nonloop.wav",
                b"misc/loop.wav",
            ] {
                let ordinal = precache(services, context, name);
                (ENGINE_CALLS.ambient)(services, context, descriptor(ordinal)).unwrap();
            }
        }
        let bank = bank();
        for context in contexts {
            attach(services, context, &bank);
            assert_eq!(
                services.storage.publish_signon(
                    services.server,
                    context,
                    ClientId(0),
                    &mut initial[context.module.0 as usize - 1]
                ),
                Ok(1)
            );
            assert_eq!(
                services.storage.publish_signon(
                    services.server,
                    context,
                    ClientId(0),
                    &mut initial[context.module.0 as usize - 1]
                ),
                Ok(0)
            );
        }
        let records = drain(services, consumer);
        assert_eq!(records.len(), 2);
        let events = records.iter().copied().map(sound).collect::<Vec<_>>();
        assert_eq!(events[0].sound, events[1].sound);
        assert_ne!(events[0].entity, events[1].entity);
        for (record, context) in records.into_iter().zip(contexts) {
            assert_eq!(record.delivery.unwrap().source, context.native_rules);
            assert!(matches!(
                sound(record).entity,
                Some(SoundEmitter::Static { slot: 2, .. })
            ));
        }
    });
}

#[test]
fn resource_changes_do_not_republish_and_rebind_discards_old_map() {
    with_services(|services, [context, _]| {
        let mut initial: [SignonCursor; 2] = std::array::from_fn(|_| SignonCursor::default());
        let consumer = recipient(services, ClientId(0));
        let ordinal = precache(services, context, b"misc/loop.wav");
        (ENGINE_CALLS.ambient)(services, context, descriptor(ordinal)).unwrap();
        let bank = bank();
        attach(services, context, &bank);
        let mut cursor = SignonCursor::default();
        services
            .storage
            .publish_signon(services.server, context, ClientId(0), &mut cursor)
            .unwrap();
        let started = sound(drain(services, consumer)[0]);
        let sounds = services.storage.resources(context.module).unwrap()[1];
        (ENGINE_CALLS.configstring)(
            services,
            context.module,
            sounds.get(ordinal as usize).unwrap(),
            b"",
        )
        .unwrap();
        assert_eq!(
            services
                .storage
                .publish_signon(services.server, context, ClientId(0), &mut cursor),
            Ok(0)
        );
        assert!(drain(services, consumer).is_empty());
        assert!(cursor.has_publication_for(
            context.module,
            ClientId(0),
            services.server.clients[0].entity
        ));
        let SoundEmitter::Static { lifetime, .. } = started.entity.unwrap() else {
            panic!()
        };
        assert!(
            services
                .server
                .entities
                .release(lifetime, ThinkTime::Seconds(1.0))
        );
        assert_eq!(
            services.storage.publish_signon(
                services.server,
                context,
                ClientId(0),
                &mut initial[context.module.0 as usize - 1]
            ),
            Err(CallError::Entity)
        );
        let replacement = services
            .server
            .entities
            .allocate(ThinkTime::Seconds(5.0), context.module, context.allocation)
            .unwrap()
            .id;
        assert_ne!(replacement, lifetime);
        services
            .storage
            .bind_lifetime(context.module, replacement)
            .unwrap();
        assert_eq!(
            services
                .storage
                .publish_signon(services.server, context, ClientId(0), &mut cursor),
            Ok(0)
        );
        assert_eq!(
            services.storage.publish_signon(
                services.server,
                context,
                ClientId(0),
                &mut SignonCursor::default()
            ),
            Ok(0)
        );
    });
}

#[test]
fn reservation_is_bounded_and_does_not_publish_during_registration() {
    with_services(|services, [context, _]| {
        let consumer = recipient(services, ClientId(0));
        let ordinal = precache(services, context, b"misc/loop.wav");
        assert_eq!(
            (ENGINE_CALLS.ambient)(services, context, descriptor(0)),
            Err(CallError::ConfigString)
        );
        let sounds = services.storage.resources(context.module).unwrap()[1];
        assert_eq!(
            (ENGINE_CALLS.ambient)(services, context, descriptor(sounds.count)),
            Err(CallError::ConfigString)
        );
        for slot in 0..SIGNON_BYTES / 10 {
            assert_eq!(
                (ENGINE_CALLS.ambient)(services, context, descriptor(ordinal)),
                Ok(slot as u32)
            );
        }
        assert_eq!(
            (ENGINE_CALLS.ambient)(services, context, descriptor(ordinal)),
            Err(CallError::Capacity)
        );
        assert!(drain(services, consumer).is_empty());
    });
}

#[test]
fn mixed_static_and_sound_signon_keeps_one_order_and_independent_late_cursors() {
    with_services(|services, [context, _]| {
        let mut initial: [SignonCursor; 2] = std::array::from_fn(|_| SignonCursor::default());
        let consumer = recipient(services, ClientId(0));
        let ordinal = precache(services, context, b"misc/loop.wav");
        let models = services.storage.resources(context.module).unwrap()[0];
        let model =
            (ENGINE_CALLS.resource_index)(services, context.module, models, b"progs/flame.mdl")
                .unwrap();
        let entity = EntityRecord {
            position: Vec3([1.0, 2.0, 3.0]),
            appearance: EntityAppearance {
                source: context.native_rules,
                models: [
                    ModelRef::native(ResourceNamespace::Module(context.module), model),
                    None,
                    None,
                    None,
                ],
                frame: 7,
                ..EntityAppearance::default()
            },
            ..EntityRecord::default()
        };
        let lives = services.server.entities.len();
        assert_eq!(
            (ENGINE_CALLS.ambient)(services, context, descriptor(ordinal)),
            Ok(0)
        );
        assert_eq!(
            (ENGINE_CALLS.static_entity)(services, context, entity),
            Ok(1)
        );
        assert_eq!(services.server.entities.len(), lives);
        assert!(drain(services, consumer).is_empty());
        attach(services, context, &bank());
        assert_eq!(
            services.storage.publish_signon(
                services.server,
                context,
                ClientId(0),
                &mut initial[context.module.0 as usize - 1]
            ),
            Ok(2)
        );
        let initial_records = drain(services, consumer);
        assert!(matches!(initial_records[0].event, FrameEvent::Sound(_)));
        assert!(
            matches!(initial_records[1].event, FrameEvent::StaticEntity(value) if value == entity)
        );
        assert_eq!(
            (ENGINE_CALLS.static_entity)(services, context, EntityRecord::default()),
            Ok(2)
        );
        assert_eq!(
            services.storage.publish_signon(
                services.server,
                context,
                ClientId(0),
                &mut initial[context.module.0 as usize - 1]
            ),
            Ok(0)
        );
        assert!(drain(services, consumer).is_empty());
        assert!(services.server.disconnect(ClientId(0)));
        let consumer = recipient(services, ClientId(0));
        let mut cursor = SignonCursor::default();
        assert_eq!(
            services
                .storage
                .publish_signon(services.server, context, ClientId(0), &mut cursor),
            Ok(3)
        );
        let late = drain(services, consumer);
        assert!(matches!(late[1].event, FrameEvent::StaticEntity(value) if value == entity));
        assert!(
            matches!(late[2].event, FrameEvent::StaticEntity(value) if value == EntityRecord::default())
        );
        assert_eq!(
            services
                .storage
                .publish_signon(services.server, context, ClientId(0), &mut cursor),
            Ok(0)
        );
        assert!(cursor.has_publication_for(
            context.module,
            ClientId(0),
            services.server.clients[0].entity
        ));
    });
}

#[test]
fn static_models_and_sounds_share_the_same_native_byte_budget_and_rejection_counter() {
    with_services(|services, [context, _]| {
        let ordinal = precache(services, context, b"misc/loop.wav");
        for slot in 0..SIGNON_BYTES / 14 {
            assert_eq!(
                (ENGINE_CALLS.static_entity)(services, context, EntityRecord::default()),
                Ok(slot as u32)
            );
        }
        assert_eq!(
            (ENGINE_CALLS.ambient)(services, context, descriptor(ordinal)),
            Err(CallError::Capacity)
        );
        assert_eq!(services.server.events.rejected_publications(), 1);
        assert_eq!(
            (ENGINE_CALLS.static_entity)(services, context, EntityRecord::default()),
            Err(CallError::Capacity)
        );
        assert_eq!(services.server.events.rejected_publications(), 2);
    });
}
