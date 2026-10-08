use qa_core::primitives::{ModuleId, PlayerTail, WeaponId};
use qa_session::clients::{Connection, Server};

#[test]
fn two_local_players_keep_independent_inventory_and_reuse_preallocated_state() {
    let mut server = Server::load(2, 512, 256, 16, 8).unwrap();
    let first = server
        .connect(Connection::Local, ModuleId(1), PlayerTail::default())
        .unwrap();
    let second = server
        .connect(Connection::Local, ModuleId(2), PlayerTail::default())
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
            .connect(Connection::Bot, ModuleId(3), PlayerTail::default())
            .is_none()
    );
    let memory = server.clients[0].player.inventory.as_ptr();
    let timers = server.clients[0].player.powerup_until.as_ptr();
    let hud_weapons = server.clients[0].hud.owned_weapons.as_ptr();
    let old_entity = server.clients[0].entity;
    assert!(server.disconnect(first));
    assert_eq!(
        server.connect(Connection::Local, ModuleId(3), PlayerTail::default()),
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
