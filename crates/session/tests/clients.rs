use qa_core::primitives::{ClientId, ModuleId, NativeEntity, PlayerTail, WeaponId};
use qa_session::clients::{Connection, Server};

#[test]
fn two_local_players_keep_independent_inventory_and_reuse_preallocated_state() {
    let mut server = Server::load(2, 512, 256, 16, 8, 0).unwrap();
    let first = server
        .connect(Connection::Local, ModuleId(1), PlayerTail::default(), None)
        .unwrap();
    let second = server
        .connect(Connection::Local, ModuleId(2), PlayerTail::default(), None)
        .unwrap();
    assert_ne!(first, second);
    assert_ne!(server.clients[0].entity, server.clients[1].entity);
    server.clients[0].player.health = 100;
    server.clients[1].player.health = 75;
    server.clients[0].player.inventory[4] = 25;
    server.clients[0].player.item_acquired_at[4] = 2.5;
    server.clients[0].player.powerup_until[15] = 100.0;
    server.clients[0].hud.item_counts[4] = 25;
    server.clients[0].hud.powerup_until[15] = 100.0;
    server.clients[0].hud.owned_weapons[0] = 1 << 7;
    assert_eq!(server.clients[0].hud.owned_weapons.len(), 1);
    server.clients[1].player.weapon = WeaponId(3);
    assert_eq!(server.clients[1].player.inventory[4], 0);
    assert_eq!(server.clients[0].player.weapon, WeaponId(0));
    assert!(
        server
            .connect(Connection::Bot, ModuleId(3), PlayerTail::default(), None)
            .is_none()
    );
    let memory = server.clients[0].player.inventory.as_ptr();
    let timers = server.clients[0].player.powerup_until.as_ptr();
    let hud_weapons = server.clients[0].hud.owned_weapons.as_ptr();
    let old_entity = server.clients[0].entity;
    assert!(server.disconnect(first));
    assert_eq!(
        server.connect(Connection::Local, ModuleId(3), PlayerTail::default(), None),
        Some(first)
    );
    assert_eq!(server.clients[0].player.inventory.as_ptr(), memory);
    assert_eq!(server.clients[0].player.inventory[4], 0);
    assert_eq!(server.clients[0].player.powerup_until.as_ptr(), timers);
    assert_eq!(server.clients[0].player.item_acquired_at[4], 0.0);
    assert_eq!(server.clients[0].player.powerup_until[15], 0.0);
    assert_eq!(server.clients[0].hud.owned_weapons.as_ptr(), hud_weapons);
    assert_eq!(server.clients[0].hud.owned_weapons[0], 0);
    assert_eq!(server.clients[0].hud.item_counts[4], 0);
    assert_eq!(server.clients[0].hud.powerup_until[15], 0.0);
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
                .connect(Connection::Remote, ModuleId(7), PlayerTail::default(), None)
                .unwrap();
            assert_eq!(id, ClientId(slot as u32));
            server.clients[slot].player.inventory[1] = slot as i32;
        }
        assert!(
            server
                .connect(Connection::Remote, ModuleId(7), PlayerTail::default(), None)
                .is_none()
        );
        assert_eq!(
            server.clients[count - 1].player.inventory[1],
            count as i32 - 1
        );
        assert_eq!(server.clients[0].player.inventory[1], 0);
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
        .connect(
            Connection::Local,
            ModuleId(10),
            PlayerTail::None,
            Some(arena),
        )
        .unwrap();
    let second = server
        .connect(
            Connection::Remote,
            ModuleId(11),
            PlayerTail::None,
            Some(classic),
        )
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
    let memory = server.clients[first.0 as usize].player.inventory.as_ptr();
    assert!(server.disconnect(first));
    assert_eq!(server.entities.columns.native_entity[slot], None);
    assert_eq!(
        server.connect(Connection::Bot, ModuleId(12), PlayerTail::None, None),
        Some(first)
    );
    assert_eq!(server.entities.columns.native_entity[slot], None);
    assert_eq!(
        server.clients[first.0 as usize].player.inventory.as_ptr(),
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
    server.submit_command(ClientId(u32::MAX), command);
    server.submit_command(ClientId(0), command);
    assert_eq!(server.clients[0].command.duration_ms, 0);
    let id = server
        .connect(Connection::Remote, ModuleId(1), PlayerTail::None, None)
        .unwrap();
    server.submit_command(id, command);
    assert_eq!(server.clients[0].command.duration_ms, 123);
}
