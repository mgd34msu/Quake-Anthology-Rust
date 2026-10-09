use qa_core::{names::NameTable, primitives::*};
use qa_gameplay::registry::Registry;
use qa_ui::hud::{HudBindings, expire_messages, print};

#[test]
fn every_game_and_mixed_weapon_uses_the_same_snapshot_and_numeric_ammo_binding() {
    let names = NameTable::load(Registry::names_needed()).unwrap();
    let registry = Registry::load(&names).unwrap();
    let bindings = HudBindings::load(&registry, &[]).unwrap();
    let mut player =
        PlayerState::with_capacity(registry.items.len() + 1, 16, bindings.values.capacity());
    let mut state = bindings.state(16, NameId(10));
    player.health = 75;
    player.armor = 30;
    player.frags = 2;
    player.score = 9;
    player.collectibles = 5;
    player.powerup_until[3] = 123.5;
    for weapon in &registry.weapons {
        player.inventory[weapon.item.0 as usize] = 1;
        player.item_acquired_at[weapon.item.0 as usize] = 25.0;
        player.weapon = weapon.id;
        if let Some(ammo) = weapon.ammo {
            player.inventory[ammo.0 as usize] = 17;
        }
        bindings.update(&player, &mut state);
        assert_eq!(
            (state.health, state.armor, state.frags, state.score),
            (75, 30, 2, 9)
        );
        assert_eq!(state.ammo, if weapon.ammo.is_some() { 17 } else { 0 });
        assert_eq!(state.weapon, weapon.id);
        assert_ne!(
            state.owned_weapons[weapon.id.0 as usize / 64] & (1 << (weapon.id.0 % 64)),
            0
        );
        assert_eq!(state.item_acquired_at[weapon.item.0 as usize], 25.0);
        assert_eq!(
            (state.collectibles, state.powerup_until[3], state.layout),
            (5, 123.5, NameId(10))
        );
    }
    let pointer = state.item_counts.as_ptr();
    state.reset(&mut qa_core::events::TextStore::load(1, 1).unwrap());
    assert_eq!(state.item_counts.as_ptr(), pointer);
    assert!(state.owned_weapons.iter().all(|&word| word == 0));
}

#[test]
fn messages_keep_handles_per_seat_and_expire_at_their_deadline() {
    let mut first = HudState::default();
    let second = HudState::default();
    let mut texts = qa_core::events::TextStore::load(8, 32).unwrap();
    let leases: Vec<_> = (0..5).map(|i| texts.insert(&[i]).unwrap()).collect();
    let id = leases[1].id();
    for lease in &leases {
        print(
            &mut first,
            &mut texts,
            PrintEvent {
                client: Some(ClientId(0)),
                kind: PrintKind::Notify,
                text: lease.id(),
            },
            10.0,
            3.0,
            2.0,
        );
    }
    assert_eq!(first.notify[0].as_ref().unwrap().text.id().slot, 1);
    assert_eq!(first.notify[3].as_ref().unwrap().text.id().slot, 4);
    print(
        &mut first,
        &mut texts,
        PrintEvent {
            client: None,
            kind: PrintKind::Center,
            text: id,
        },
        10.0,
        3.0,
        2.0,
    );
    assert!(second.centerprint.is_none());
    expire_messages(&mut first, &mut texts, 12.0);
    assert!(first.centerprint.is_none());
    assert!(first.notify.iter().all(Option::is_some));
    expire_messages(&mut first, &mut texts, 13.0);
    assert!(first.notify.iter().all(Option::is_none));
}

#[test]
fn display_leases_release_on_replacement_expiry_and_reset_without_invalidating_other_seats() {
    let mut texts = qa_core::events::TextStore::load(3, 32).unwrap();
    let mut first = HudState::default();
    let mut second = HudState::default();
    let producer = texts.insert(b"persistent\n").unwrap();
    let id = producer.id();
    for state in [&mut first, &mut second] {
        assert!(print(
            state,
            &mut texts,
            PrintEvent {
                client: None,
                kind: PrintKind::Center,
                text: id
            },
            0.0,
            3.0,
            2.0
        ));
    }
    texts.release(producer);
    for _ in 0..100 {
        let producer = texts.insert(b"new").unwrap();
        let replacement = producer.id();
        assert!(print(
            &mut first,
            &mut texts,
            PrintEvent {
                client: None,
                kind: PrintKind::Layout,
                text: replacement
            },
            0.0,
            3.0,
            2.0
        ));
        texts.release(producer);
        assert_eq!(texts.get(id), Some(b"persistent\n".as_slice()));
    }
    first.reset(&mut texts);
    assert_eq!(texts.get(id), Some(b"persistent\n".as_slice()));
    expire_messages(&mut second, &mut texts, 2.0);
    assert!(texts.get(id).is_none());
    // All rows can now be acquired simultaneously; no lease leaked.
    let pages: Vec<_> = (0..3).map(|_| texts.insert(b"free").unwrap()).collect();
    assert!(texts.insert(b"full").is_none());
    for page in pages {
        texts.release(page);
    }
}
