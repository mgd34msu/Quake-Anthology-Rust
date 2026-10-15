use qa_core::primitives::{
    ClientId, ClockObservation, ModuleId, NativeEntity, ThinkTime, WeaponId,
};
use qa_session::clients::{Connection, Server};

#[test]
fn post_load_native_binding_rejects_duplicate_lifetimes_without_changing_roles() {
    use qa_core::primitives::RuleSetId;
    let mut server = Server::load(2, 8, 2, 2, 2, 0).unwrap();
    let first = server
        .connect(Connection::Local, ModuleId(0), None)
        .unwrap();
    let second = server.connect(Connection::Bot, ModuleId(0), None).unwrap();
    let native = NativeEntity {
        module: ModuleId(10),
        slot: 7,
    };
    server.clients[first.0 as usize].player.movement_rules = RuleSetId::Quake3;
    server.clients[first.0 as usize].player.trace_rules = RuleSetId::Quake;
    assert!(server.bind_client_native(first, native));
    assert!(!server.bind_client_native(second, native));
    assert_eq!(server.clients[second.0 as usize].module, ModuleId(0));
    assert_eq!(
        server.clients[first.0 as usize].player.movement_rules,
        RuleSetId::Quake3
    );
    assert_eq!(
        server.clients[first.0 as usize].player.trace_rules,
        RuleSetId::Quake
    );
    assert!(server.disconnect(first));
    assert!(!server.bind_client_native(first, native));
    assert!(server.bind_client_native(second, native));
    assert_eq!(server.client_for_native(native), Some(second));
}

#[test]
fn native_weapon_owner_handoff_clears_readiness_and_identical_binding_preserves_it() {
    use qa_core::primitives::{ResourceNamespace, RuleSetId, WeaponReadiness, WeaponState};
    let mut server = Server::load(2, 8, 2, 2, 2, 0).unwrap();
    let original = NativeEntity {
        module: ModuleId(3),
        slot: 1,
    };
    let id = server
        .connect(Connection::Local, original.module, Some(original))
        .unwrap();
    let other_native = NativeEntity {
        module: ModuleId(8),
        slot: 0,
    };
    server
        .connect(Connection::Remote, other_native.module, Some(other_native))
        .unwrap();
    let player = &mut server.clients[id.0 as usize].player;
    player.movement_rules = RuleSetId::Quake3;
    player.trace_rules = RuleSetId::Quake2;
    let seconds = Some(WeaponState {
        source: RuleSetId::QuakeWorld,
        namespace: ResourceNamespace::Module(original.module),
        readiness: WeaponReadiness::DeadlineSeconds(4.25),
    });
    player.weapon_state = seconds;
    let clock = Some(ClockObservation {
        native: ThinkTime::Milliseconds(5000),
        queued: qa_core::sys_events::EventTime::from_milliseconds(137),
    });
    player.native_clock = clock;
    let sound =
        qa_core::primitives::SoundRef::native(ResourceNamespace::Module(original.module), 37);
    player.loop_sound = sound;
    assert!(server.bind_client_native(id, original));
    assert_eq!(server.clients[id.0 as usize].player.native_clock, clock);
    assert_eq!(server.clients[id.0 as usize].player.loop_sound, sound);
    assert_eq!(server.clients[id.0 as usize].player.weapon_state, seconds);
    assert!(!server.bind_client_native(id, other_native));
    assert_eq!(server.clients[id.0 as usize].player.loop_sound, sound);
    assert_eq!(server.clients[id.0 as usize].player.native_clock, clock);
    assert_eq!(server.clients[id.0 as usize].player.weapon_state, seconds);

    let same_owner_new_row = NativeEntity {
        slot: 2,
        ..original
    };
    assert!(server.bind_client_native(id, same_owner_new_row));
    assert_eq!(server.clients[id.0 as usize].player.native_clock, None);
    assert_eq!(server.clients[id.0 as usize].player.loop_sound, None);
    assert_eq!(server.clients[id.0 as usize].player.weapon_state, None);
    let milliseconds = Some(WeaponState {
        source: RuleSetId::Quake3,
        namespace: ResourceNamespace::Module(original.module),
        readiness: WeaponReadiness::RemainingMilliseconds(-197_000_071),
    });
    server.clients[id.0 as usize].player.weapon_state = milliseconds;
    assert!(server.bind_client_native(id, same_owner_new_row));
    assert_eq!(
        server.clients[id.0 as usize].player.weapon_state,
        milliseconds
    );
    // Q2's public player ABI has no readiness field to replace the old timer.
    let q2_owner = NativeEntity {
        module: ModuleId(9),
        slot: 1,
    };
    assert!(server.bind_client_native(id, q2_owner));
    let client = &server.clients[id.0 as usize];
    assert_eq!(client.player.weapon_state, None);
    assert_eq!(
        (client.player.movement_rules, client.player.trace_rules),
        (RuleSetId::Quake3, RuleSetId::Quake2)
    );
    assert_eq!(client.module, q2_owner.module);
    assert_eq!(server.client_for_native(q2_owner), Some(id));
}

#[test]
fn authoritative_attachment_commit_updates_local_remote_and_bot_hot_state() {
    use qa_core::primitives::{BodyAttachment, BodyFollow, Vec3};
    use qa_world::area::{LinkFlags, LinkIntent, LinkOrder};
    let mut server = Server::load(3, 8, 2, 2, 2, 0).unwrap();
    for connection in [Connection::Local, Connection::Remote, Connection::Bot] {
        server.connect(connection, ModuleId(1), None).unwrap();
    }
    let root = server.clients[0].entity;
    let parent = server.clients[1].entity;
    let child = server.clients[2].entity;
    server.entities.columns.position[root.slot as usize] = Vec3([10.0, 20.0, 30.0]);
    for (id, anchor) in [(child, parent), (parent, root)] {
        server
            .entities
            .attach(
                id,
                BodyAttachment {
                    anchor,
                    follow: BodyFollow::Translation,
                    offset: Vec3([1.0; 3]),
                },
            )
            .unwrap();
        assert!(server.area.link(
            &server.entities,
            id,
            LinkFlags::SOLID,
            LinkOrder::Head,
            LinkIntent::Explicit
        ));
    }
    let result = server.commit_attachments();
    assert_eq!((result.moved, result.relinked), (2, 2));
    assert_eq!(
        server.clients[1].player.body.position,
        Vec3([11.0, 21.0, 31.0])
    );
    assert_eq!(
        server.clients[2].player.body.position,
        Vec3([12.0, 22.0, 32.0])
    );
    assert_eq!(
        server.clients[2].player.body.position,
        server.entities.columns.position[child.slot as usize]
    );
    assert!(server.disconnect(ClientId(1)));
    assert!(server.entities.attachment(child).is_none());
    assert_eq!(server.commit_attachments().moved, 0);
}

#[test]
fn two_local_players_keep_independent_inventory_and_reuse_preallocated_state() {
    let mut server = Server::load(2, 512, 256, 16, 8, 0).unwrap();
    let first = server
        .connect(Connection::Local, ModuleId(1), None)
        .unwrap();
    let second = server
        .connect(Connection::Local, ModuleId(2), None)
        .unwrap();
    assert_ne!(first, second);
    assert_ne!(server.clients[0].entity, server.clients[1].entity);
    server.clients[0].player.health = 100;
    server.clients[1].player.health = 75;
    server.clients[0].player.inventory.item_counts[4] = 25;
    server.clients[0].player.inventory.item_acquired_at[4] = 2.5;
    server.clients[0].player.inventory.powerup_until[15] = 100.0;
    server.clients[0].hud.inventory.item_counts[4] = 25;
    server.clients[0].hud.inventory.powerup_until[15] = 100.0;
    server.clients[0].hud.owned_weapons.insert(7);
    assert_eq!(server.clients[0].hud.owned_weapons.words().len(), 1);
    server.clients[1].player.inventory.weapon = WeaponId(3);
    assert_eq!(server.clients[1].player.inventory.item_counts[4], 0);
    assert_eq!(server.clients[0].player.inventory.weapon, WeaponId(0));
    assert!(server.connect(Connection::Bot, ModuleId(3), None).is_none());
    let memory = server.clients[0].player.inventory.item_counts.as_ptr();
    let timers = server.clients[0].player.inventory.powerup_until.as_ptr();
    let hud_weapons = server.clients[0].hud.owned_weapons.words().as_ptr();
    let old_entity = server.clients[0].entity;
    assert!(server.disconnect(first));
    assert_eq!(
        server.connect(Connection::Local, ModuleId(3), None),
        Some(first)
    );
    assert_eq!(
        server.clients[0].player.inventory.item_counts.as_ptr(),
        memory
    );
    assert_eq!(server.clients[0].player.inventory.item_counts[4], 0);
    assert_eq!(
        server.clients[0].player.inventory.powerup_until.as_ptr(),
        timers
    );
    assert_eq!(server.clients[0].player.inventory.item_acquired_at[4], 0.0);
    assert_eq!(server.clients[0].player.inventory.powerup_until[15], 0.0);
    assert_eq!(
        server.clients[0].hud.owned_weapons.words().as_ptr(),
        hud_weapons
    );
    assert!(server.clients[0].hud.owned_weapons.next_set(0).is_none());
    assert_eq!(server.clients[0].hud.inventory.item_counts[4], 0);
    assert_eq!(server.clients[0].hud.inventory.powerup_until[15], 0.0);
    assert_ne!(server.clients[0].entity, old_entity);
    assert!(server.entities.resolve(old_entity).is_none());
    assert_eq!(server.clients[1].player.health, 75);
}

#[test]
fn native_q2_capacity_and_larger_common_namespaces_do_not_truncate_client_ids() {
    for count in [256, 512] {
        let mut server = Server::load(count, count + 1, 2, 2, 2, 0).unwrap();
        assert_eq!(server.clients.len(), count);
        for slot in 0..count {
            let id = server
                .connect(Connection::Remote, ModuleId(7), None)
                .unwrap();
            assert_eq!(id, ClientId(slot as u32));
            server.clients[slot].player.inventory.item_counts[1] = slot as i32;
        }
        assert!(
            server
                .connect(Connection::Remote, ModuleId(7), None)
                .is_none()
        );
        assert_eq!(
            server.clients[count - 1].player.inventory.item_counts[1],
            count as i32 - 1
        );
        assert_eq!(server.clients[0].player.inventory.item_counts[1], 0);
        assert!(server.disconnect(ClientId(count as u32 - 1)));
        assert!(!server.disconnect(ClientId(count as u32)));
    }
}

#[test]
fn native_slot_and_namespace_are_explicit_and_reset_on_client_reuse() {
    let mut server = Server::load(2, 16, 2, 2, 2, 0).unwrap();
    let arena = NativeEntity {
        module: ModuleId(20),
        slot: 0,
    };
    let classic = NativeEntity {
        module: ModuleId(21),
        slot: 1,
    };
    let first = server
        .connect(Connection::Local, ModuleId(10), Some(arena))
        .unwrap();
    let second = server
        .connect(Connection::Remote, ModuleId(11), Some(classic))
        .unwrap();
    let entity = server.clients[first.0 as usize].entity;
    let slot = entity.slot as usize;
    assert_eq!(slot, 1);
    assert_eq!(server.entities.columns.owner[slot], ModuleId(10));
    assert_eq!(server.entities.columns.native_entity[slot], Some(arena));
    assert_eq!(
        server.entities.columns.native_entity
            [server.clients[second.0 as usize].entity.slot as usize],
        Some(classic)
    );
    assert_eq!(server.entities.columns.native_entity[0], None);
    let memory = server.clients[first.0 as usize]
        .player
        .inventory
        .item_counts
        .as_ptr();
    assert!(server.disconnect(first));
    assert_eq!(server.entities.columns.native_entity[slot], None);
    assert_eq!(
        server.connect(Connection::Bot, ModuleId(12), None),
        Some(first)
    );
    assert_eq!(server.entities.columns.native_entity[slot], None);
    assert_eq!(
        server.clients[first.0 as usize]
            .player
            .inventory
            .item_counts
            .as_ptr(),
        memory
    );
    assert!(server.entities.resolve(entity).is_none());
}

#[test]
fn untrusted_command_client_ids_are_scoped_and_disconnected_slots_stay_idle() {
    let mut server = Server::load(2, 8, 1, 0, 0, 0).unwrap();
    let command = qa_core::primitives::UserCmd {
        duration_ms: 123,
        ..Default::default()
    };
    server.submit_command(
        ClientId(u32::MAX),
        command,
        qa_core::primitives::CommandExecution::PerCommand,
        None,
    );
    server.submit_command(
        ClientId(0),
        command,
        qa_core::primitives::CommandExecution::PerCommand,
        None,
    );
    assert_eq!(server.clients[0].command.duration_ms, 0);
    let id = server
        .connect(Connection::Remote, ModuleId(1), None)
        .unwrap();
    server.submit_command(
        id,
        command,
        qa_core::primitives::CommandExecution::PerCommand,
        None,
    );
    assert_eq!(server.clients[0].command.duration_ms, 123);
}

#[test]
fn module_movement_requires_admission_owner_and_current_entity_lifetime() {
    let mut server = Server::load(3, 8, 2, 2, 2, 0).unwrap();
    for (index, connection) in [Connection::Local, Connection::Remote, Connection::Bot]
        .into_iter()
        .enumerate()
    {
        let native = NativeEntity {
            module: ModuleId(3),
            slot: index as i32 + 1,
        };
        let id = server
            .connect(connection, native.module, Some(native))
            .unwrap();
        assert_eq!(server.module_moves(id), None);
        server.clients[id.0 as usize].module_movement = true;
        assert_eq!(server.module_moves(id), Some(native));
        server.clients[id.0 as usize].module = ModuleId(4);
        assert_eq!(server.module_moves(id), None);
        server.clients[id.0 as usize].module = native.module;
        let entity = server.clients[id.0 as usize].entity;
        server.entities.columns.native_entity[entity.slot as usize] = None;
        assert_eq!(server.module_moves(id), None);
        server.entities.columns.native_entity[entity.slot as usize] = Some(native);
        let replacement = server.entities.reset_client(entity).unwrap();
        server.entities.columns.native_entity[replacement.slot as usize] = Some(native);
        assert_eq!(server.module_moves(id), None);
        server.clients[id.0 as usize].entity = replacement;
        assert_eq!(server.module_moves(id), Some(native));
        assert!(server.disconnect(id));
        assert_eq!(server.module_moves(id), None);
    }
    assert_eq!(server.module_moves(ClientId(u32::MAX)), None);
}

#[test]
fn pending_command_keeps_its_snapshot_observation_once_and_disconnect_clears_it() {
    use qa_core::{
        primitives::{CommandExecution, SnapshotObservation, UserCmd},
        sys_events::EventTime,
    };
    let mut server = Server::load(1, 8, 1, 0, 0, 0).unwrap();
    let id = server
        .connect(Connection::Remote, ModuleId(7), None)
        .unwrap();
    let observation = SnapshotObservation {
        module: ModuleId(7),
        module_lifetime: server.clients[id.0 as usize].entity,
        native_frame: 4,
        time: EventTime::from_milliseconds(2300),
    };
    server.submit_command(
        id,
        UserCmd::default(),
        CommandExecution::FrameLatest,
        Some(observation),
    );
    server.resolve_frame_command(
        id,
        EventTime::from_milliseconds(20),
        EventTime::from_milliseconds(2400),
    );
    let (command, source) = server.take_pending_command(id).unwrap();
    assert_eq!(source, Some(observation));
    assert_eq!(command.duration_ms, 20);
    assert!(server.take_pending_command(id).is_none());
    assert_eq!(server.clients[id.0 as usize].command_observation, None);
    server.submit_command(
        id,
        UserCmd::default(),
        CommandExecution::PerCommand,
        Some(observation),
    );
    server.disconnect(id);
    assert_eq!(server.clients[id.0 as usize].command_observation, None);
    assert_eq!(
        server.connect(Connection::Remote, ModuleId(7), None),
        Some(id)
    );
    assert!(server.take_pending_command(id).is_none());
}

#[test]
fn admitted_module_clients_keep_the_command_span_for_native_leaf_preparation() {
    use qa_core::{
        primitives::{CommandExecution, RuleSetId, UserCmd},
        sys_events::EventTime,
    };
    let mut server = Server::load(2, 8, 1, 0, 0, 0).unwrap();
    let native = NativeEntity {
        module: ModuleId(7),
        slot: 1,
    };
    let module = server
        .connect(Connection::Bot, native.module, Some(native))
        .unwrap();
    let engine = server.connect(Connection::Bot, ModuleId(8), None).unwrap();
    for client in &mut server.clients {
        client.input_rules = RuleSetId::Quake3;
        client.player.movement_rules = RuleSetId::Quake3;
    }
    server.clients[module.0 as usize].module_movement = true;
    let cvars = qa_console::cvars::Cvars::new().unwrap();
    let handles = qa_console::input_settings::InputHandles::load(&cvars).unwrap();
    server.build_bot_commands(EventTime(0), EventTime::from_milliseconds(250), |rules| {
        handles.policy(&cvars, rules)
    });
    assert_eq!(
        server.take_pending_command(module).unwrap().0.duration_ms,
        250
    );
    assert_eq!(
        server.take_pending_command(engine).unwrap().0.duration_ms,
        200
    );
    for id in [module, engine] {
        server.submit_command(id, UserCmd::default(), CommandExecution::FrameLatest, None);
        server.resolve_frame_command(
            id,
            EventTime::from_milliseconds(250),
            EventTime::from_milliseconds(300),
        );
    }
    assert_eq!(
        server.take_pending_command(module).unwrap().0.duration_ms,
        250
    );
    assert_eq!(
        server.take_pending_command(engine).unwrap().0.duration_ms,
        200
    );
}

#[test]
fn pending_client_audio_retirement_survives_reset_and_blocks_only_its_slot() {
    use qa_session::clients::AudioRetirement;
    let mut server = Server::load(2, 8, 2, 2, 2, 0).unwrap();
    let old = server
        .connect(Connection::Local, ModuleId(1), None)
        .unwrap();
    let lifetime = server.clients[old.0 as usize].entity;
    server.clients[old.0 as usize].audio_retirement = Some(AudioRetirement {
        lifetime,
        publication: Some((server.presentation, 42)),
    });
    assert!(server.disconnect(old));
    assert_eq!(
        server.clients[old.0 as usize]
            .audio_retirement
            .unwrap()
            .lifetime,
        lifetime
    );
    let healthy = server.connect(Connection::Bot, ModuleId(2), None).unwrap();
    assert_ne!(healthy, old);
    assert!(
        server
            .connect(Connection::Local, ModuleId(1), None)
            .is_none()
    );
    // The app's Presentation callback is the sole production completion owner.
    server.clients[old.0 as usize].audio_retirement = None;
    assert_eq!(
        server.connect(Connection::Local, ModuleId(1), None),
        Some(old)
    );
    assert_ne!(server.clients[old.0 as usize].entity, lifetime);
    assert!(server.disconnect(healthy));
    // SERVER-only clients never acquire an audio retirement and reuse at once.
    assert_eq!(
        server.connect(Connection::Bot, ModuleId(2), None),
        Some(healthy)
    );
}
