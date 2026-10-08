use qa_app::{
    Runtime,
    host::{FrameHost, FrameSource},
};
use qa_console::{commands::Console, views::Context};
use qa_core::{
    primitives::{HudLine, ItemId, ModuleId, MovementRules, PlayerTail, WeaponId},
    sys_events::{EventKind, EventTime, SysEvent, SysEventQueue},
};
use qa_gameplay::registry::ItemKind;
use qa_session::{clients::Connection, timing::TickRate};
use std::time::Duration;

struct Source;
impl FrameSource for Source {
    fn begin_frame(&mut self, queue: &mut SysEventQueue) {
        self.poll_events(queue);
    }
    fn poll_events(&mut self, queue: &mut SysEventQueue) {
        queue
            .push(SysEvent {
                time: EventTime(1_000_000),
                kind: EventKind::Time,
            })
            .unwrap();
    }
    fn wait_events(&mut self, _: &mut SysEventQueue, _: Duration) {
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
    let mut runtime = Runtime::load([name.as_slice(), b"models/custom.mdl".as_slice()]).unwrap();
    let target = runtime.catalog.names.find(b"MIXED_DOOR").unwrap();
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
        .connect(Connection::Local, ModuleId(1), PlayerTail::None)
        .unwrap();
    let second = runtime
        .server
        .connect(Connection::Remote, ModuleId(2), PlayerTail::None)
        .unwrap();
    let entities = [first, second].map(|id| runtime.server.clients[id.0 as usize].entity);
    for entity in entities {
        assert!(runtime.server.entities.set_targetname(entity, target));
    }
    assert!(runtime.targets.refresh(&runtime.server.entities));
    assert_eq!(runtime.targets.find(target).collect::<Vec<_>>(), entities);
    assert!(!runtime.targets.refresh(&runtime.server.entities));
    runtime.server.clients[first.0 as usize].player.health = 42;
    assert!(!runtime.targets.refresh(&runtime.server.entities));
    assert!(runtime.server.disconnect(first));
    assert!(runtime.targets.refresh(&runtime.server.entities));
    assert_eq!(
        runtime.targets.find(target).collect::<Vec<_>>(),
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
fn all_clients_project_mixed_inventory_and_item_timers_without_losing_messages() {
    let mut runtime = Runtime::load(std::iter::empty()).unwrap();
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
    for slot in 0..64 {
        let id = runtime
            .server
            .connect(
                [Connection::Local, Connection::Remote, Connection::Bot][slot % 3],
                ModuleId((slot % 3 + 1) as u16),
                PlayerTail::None,
            )
            .unwrap();
        let player = &mut runtime.server.clients[id.0 as usize].player;
        let loadout = loadouts[slot % 3];
        player.movement_rules = MovementRules::Quake3;
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
    // The last registry weapon and item are reachable, including the zero slot.
    runtime.server.clients[63].player.weapon = highest_weapon;
    let line = runtime.texts.insert(b"keep\nthis").unwrap();
    let layout = runtime.catalog.names.find(b"item_health").unwrap();
    let hud = &mut runtime.server.clients[0].hud;
    hud.layout = layout;
    hud.layout_text = Some(line);
    hud.centerprint = Some(HudLine {
        text: line,
        started_at: 0.0,
        until: 10.0,
    });
    hud.notify[3] = hud.centerprint;
    let mut host = FrameHost::load(
        Console::new(Context::default()),
        runtime,
        TickRate::FrameDriven,
        vec![],
    )
    .unwrap();
    host.frame(&mut Source, true);
    for (slot, client) in host.runtime.server.clients.iter().enumerate() {
        let hud = &client.hud;
        let player = &client.player;
        assert_eq!(
            (hud.health, hud.armor, hud.score, hud.frags),
            (
                100 - slot as i32,
                slot as i32,
                slot as i32 * 3,
                -(slot as i32)
            )
        );
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
    assert_eq!(hud.layout_text, Some(line));
    assert_eq!(hud.centerprint.unwrap().text, line);
    assert_eq!(hud.notify[3].unwrap().text, line);
    assert_eq!(host.runtime.texts.get(line), Some(b"keep\nthis".as_slice()));
}
