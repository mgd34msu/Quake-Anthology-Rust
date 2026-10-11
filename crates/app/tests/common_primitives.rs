use qa_app::{
    Runtime,
    host::{FrameHost, FrameSource},
};
use qa_console::{commands::Console, views::Context};
use qa_core::{
    primitives::{ClientId, HudLine, ItemId, ModuleId, PlayerTail, RuleSetId, WeaponId},
    sys_events::{EventKind, EventTime, SysEvent, SysEventQueue},
};
use qa_gameplay::registry::ItemKind;
use qa_session::{clients::Connection, timing::TickRate};
use std::time::Duration;

struct Source;
impl FrameSource for Source {
    fn begin_frame(&mut self) -> Duration {
        Duration::from_nanos(1_000_000)
    }
    fn poll_events(&mut self, queue: &mut SysEventQueue) {
        queue
            .push(SysEvent {
                time: EventTime(1_000_000),
                kind: EventKind::Time,
            })
            .unwrap();
    }
    fn wait_time(&mut self, _: Duration) -> Duration {
        panic!("uncapped fixture");
    }
    fn elapsed(&self) -> Duration {
        Duration::ZERO
    }
    fn present(&mut self) {}
}

#[test]
fn map_and_module_names_share_owned_registry_ids_and_one_target_index() {
    let mut name = b"mixed_door".to_vec();
    let mut runtime =
        Runtime::load(64, [name.as_slice(), b"models/custom.mdl".as_slice()]).unwrap();
    let target = runtime.catalog.names.find_folded(b"MIXED_DOOR").unwrap();
    name.fill(b'x');
    assert_eq!(
        runtime.catalog.names.get(target),
        Some(b"mixed_door".as_slice())
    );
    assert!(runtime.catalog.names.find(b"models/custom.mdl").is_some());
    for item in &runtime.catalog.registry.items {
        assert!(runtime.catalog.names.get(item.classname).is_some());
        assert!(runtime.catalog.names.get(item.pickup_sound).is_some());
        for &model in &item.models {
            assert!(runtime.catalog.names.get(model).is_some());
        }
    }
    let first = runtime
        .server
        .connect(Connection::Local, ModuleId(1), PlayerTail::None, None)
        .unwrap();
    let second = runtime
        .server
        .connect(Connection::Remote, ModuleId(2), PlayerTail::None, None)
        .unwrap();
    let entities = [first, second].map(|id| runtime.server.clients[id.0 as usize].entity);
    for entity in entities {
        assert!(runtime.server.entities.set_targetname(entity, Some(target)));
    }
    assert!(
        runtime
            .targets
            .refresh(&mut runtime.server.entities, &runtime.catalog.names)
    );
    assert_eq!(
        runtime
            .targets
            .find(
                target,
                qa_core::names::NameMatch::Exact,
                &runtime.catalog.names
            )
            .collect::<Vec<_>>(),
        entities
    );
    assert!(
        !runtime
            .targets
            .refresh(&mut runtime.server.entities, &runtime.catalog.names)
    );
    runtime.server.clients[first.0 as usize].player.health = 42;
    assert!(
        !runtime
            .targets
            .refresh(&mut runtime.server.entities, &runtime.catalog.names)
    );
    assert!(runtime.server.disconnect(first));
    assert!(
        runtime
            .targets
            .refresh(&mut runtime.server.entities, &runtime.catalog.names)
    );
    assert_eq!(
        runtime
            .targets
            .find(
                target,
                qa_core::names::NameMatch::Exact,
                &runtime.catalog.names
            )
            .collect::<Vec<_>>(),
        [entities[1]]
    );
}

#[derive(Clone, Copy)]
struct Loadout {
    weapon: WeaponId,
    item: ItemId,
    ammo: ItemId,
    timer: ItemId,
}

#[test]
fn local_client_snapshots_project_mixed_inventory_and_timers_without_painting_remote_huds() {
    let capacity = 257;
    let mut runtime = Runtime::load(capacity, std::iter::empty()).unwrap();
    assert_eq!(runtime.server.clients.len(), capacity);
    let registry = &runtime.catalog.registry;
    let highest_item = registry.items.last().unwrap().id;
    let highest_weapon = registry.weapons.last().unwrap().id;
    let loadouts: [Loadout; 3] = std::array::from_fn(|family| {
        let module = ModuleId(family as u16 + 1);
        let weapon = registry
            .weapons
            .iter()
            .find(|weapon| weapon.module == module && weapon.ammo.is_some())
            .unwrap();
        let timer = registry
            .items
            .iter()
            .find(|item| item.module == module && item.kind == ItemKind::Powerup)
            .unwrap()
            .id;
        Loadout {
            weapon: weapon.id,
            item: weapon.item,
            ammo: weapon.ammo.unwrap(),
            timer,
        }
    });
    assert_ne!(loadouts[0].timer, loadouts[1].timer);
    assert_ne!(loadouts[1].timer, loadouts[2].timer);
    assert!(loadouts[2].timer.0 > 16);
    for slot in 0..capacity {
        let id = runtime
            .server
            .connect(
                [Connection::Local, Connection::Remote, Connection::Bot][slot % 3],
                ModuleId((slot % 3 + 1) as u16),
                PlayerTail::None,
                None,
            )
            .unwrap();
        assert_eq!(id.0 as usize, slot);
        let player = &mut runtime.server.clients[id.0 as usize].player;
        let loadout = loadouts[slot % 3];
        player.movement_rules = RuleSetId::Quake3;
        player.health = 100 - slot as i32;
        player.armor = slot as i32;
        player.score = slot as i32 * 3;
        player.frags = -(slot as i32);
        player.weapon = loadout.weapon;
        player.inventory[loadout.item.0 as usize] = 1;
        player.inventory[loadout.ammo.0 as usize] = slot as i32 + 7;
        player.inventory[highest_item.0 as usize] = slot as i32 + 1;
        player.item_acquired_at[highest_item.0 as usize] = slot as f64 + 0.25;
        player.powerup_until[loadout.timer.0 as usize] = slot as f64 + 100.0;
    }
    assert!(
        runtime
            .server
            .connect(Connection::Bot, ModuleId(3), PlayerTail::None, None)
            .is_none()
    );
    let highest_client = ClientId((capacity - 1) as u32);
    runtime
        .loopback
        .send(
            qa_core::loopback::Endpoint::Client,
            highest_client,
            b"highest client",
        )
        .unwrap();
    assert_eq!(
        runtime.loopback.send(
            qa_core::loopback::Endpoint::Client,
            ClientId(capacity as u32),
            b"outside"
        ),
        Err(qa_core::loopback::SendError::Client)
    );
    // The last registry weapon and item are reachable, including the zero slot.
    runtime.server.clients[capacity - 1].player.weapon = highest_weapon;
    let lease = runtime.server.events.texts.insert(b"keep\nthis").unwrap();
    let line = lease.id();
    let center = runtime.server.events.texts.lease(line).unwrap();
    let notify = runtime.server.events.texts.lease(line).unwrap();
    let layout = runtime.catalog.names.find(b"item_health").unwrap();
    let hud = &mut runtime.server.clients[0].hud;
    hud.layout = layout;
    hud.layout_text = Some(lease);
    hud.centerprint = Some(HudLine {
        text: center,
        started_at: 0.0,
        until: 10.0,
    });
    hud.notify[3] = Some(HudLine {
        text: notify,
        started_at: 0.0,
        until: 10.0,
    });
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        vec![],
    )
    .unwrap();
    let local = [0, 255, 256];
    for (seat, slot) in local.into_iter().enumerate() {
        let client = &mut host.runtime.server.clients[slot];
        client.connection = Some(Connection::Local);
        let source = &client.player;
        let received = &mut host.runtime.prediction[seat].player;
        received.health = source.health - 10;
        received.armor = source.armor;
        received.armor_type = source.armor_type;
        received.score = source.score;
        received.frags = source.frags;
        received.weapon = source.weapon;
        received.inventory.copy_from_slice(&source.inventory);
        received
            .item_acquired_at
            .copy_from_slice(&source.item_acquired_at);
        received
            .powerup_until
            .copy_from_slice(&source.powerup_until);
        received.values.copy_from(&source.values);
        host.local_clients[seat] = Some(ClientId(slot as u32));
    }
    host.frame(&mut Source, true);
    assert_eq!(host.runtime.network.packets, 1);
    assert_eq!(
        host.runtime.network.last_from,
        Some(qa_core::sys_events::Peer::Loopback(highest_client))
    );
    assert_eq!(
        host.runtime.network.last_socket,
        Some(qa_core::loopback::Endpoint::Server.socket())
    );
    for (slot, client) in host.runtime.server.clients.iter().enumerate() {
        let hud = &client.hud;
        let Some(seat) = local.iter().position(|&id| id == slot) else {
            assert_eq!((hud.health, hud.armor, hud.score, hud.frags), (0, 0, 0, 0));
            assert!(hud.item_counts.iter().all(|&value| value == 0));
            assert!(hud.powerup_until.iter().all(|&value| value == 0.0));
            continue;
        };
        let player = &host.runtime.prediction[seat].player;
        assert_eq!(
            (hud.health, hud.armor, hud.score, hud.frags),
            (player.health, player.armor, player.score, player.frags)
        );
        assert_ne!(hud.health, client.player.health);
        assert_eq!(hud.item_counts[highest_item.0 as usize], slot as i32 + 1);
        assert_eq!(
            hud.item_acquired_at[highest_item.0 as usize],
            slot as f64 + 0.25
        );
        assert_eq!(hud.item_counts, player.inventory);
        assert_eq!(hud.powerup_until, player.powerup_until);
        assert_eq!(
            hud.powerup_until[loadouts[slot % 3].timer.0 as usize],
            slot as f64 + 100.0
        );
        assert_ne!(
            hud.owned_items[highest_item.0 as usize / 64] & (1 << (highest_item.0 % 64)),
            0
        );
        assert_ne!(
            hud.owned_weapons[player.weapon.0 as usize / 64] & (1 << (player.weapon.0 % 64)),
            0
        );
        let weapon = host.runtime.catalog.registry.weapon(player.weapon).unwrap();
        assert_eq!(hud.ammo, player.inventory[weapon.ammo.unwrap().0 as usize]);
    }
    let hud = &host.runtime.server.clients[0].hud;
    assert_eq!(hud.layout, layout);
    assert_eq!(hud.layout_text.as_ref().map(|text| text.id()), Some(line));
    assert_eq!(hud.centerprint.as_ref().unwrap().text.id(), line);
    assert_eq!(hud.notify[3].as_ref().unwrap().text.id(), line);
    assert_eq!(
        host.runtime.server.events.texts.get(line),
        Some(b"keep\nthis".as_slice())
    );
}
