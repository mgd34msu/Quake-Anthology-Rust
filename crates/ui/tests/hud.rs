use qa_core::{names::NameTable, primitives::*};
use qa_gameplay::registry::Registry;
use qa_ui::hud::{HudBindings, expire_messages, print};

#[test]
fn every_game_and_mixed_weapon_uses_the_same_snapshot_and_numeric_ammo_binding() {
    let names = NameTable::load(Registry::names_needed()).unwrap();
    let registry = Registry::load(&names).unwrap();
    let bindings = HudBindings::load(&registry);
    let mut player = PlayerState::with_capacity(registry.items.len() + 1, 16);
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
    state.reset();
    assert_eq!(state.item_counts.as_ptr(), pointer);
    assert!(state.owned_weapons.iter().all(|&word| word == 0));
}

#[test]
fn messages_keep_handles_per_seat_and_expire_at_their_deadline() {
    let mut first = HudState::default();
    let second = HudState::default();
    let id = TextId {
        slot: 1,
        generation: 2,
    };
    for i in 0..5 {
        print(
            &mut first,
            PrintEvent {
                client: Some(ClientId(0)),
                kind: PrintKind::Notify,
                text: TextId { slot: i, ..id },
            },
            10.0,
            3.0,
            2.0,
        );
    }
    assert_eq!(first.notify[0].unwrap().text.slot, 1);
    assert_eq!(first.notify[3].unwrap().text.slot, 4);
    print(
        &mut first,
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
    expire_messages(&mut first, 12.0);
    assert!(first.centerprint.is_none());
    assert!(first.notify.iter().all(Option::is_some));
    expire_messages(&mut first, 13.0);
    assert!(first.notify.iter().all(Option::is_none));
}
