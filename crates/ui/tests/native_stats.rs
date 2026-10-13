use qa_core::{
    events::{
        EventRing, FrameEvent, OutputRecord, OutputSubmission, OutputTarget, dispatch_consumer,
    },
    names::NameTable,
    primitives::*,
    sys_events::EventTime,
};
use qa_gameplay::registry::Registry;
use qa_ui::hud::HudBindings;

#[test]
fn native_stats_update_client_player_then_one_hud_inventory_consumer() {
    let names = NameTable::load(Registry::names_needed()).unwrap();
    let registry = Registry::load(&names).unwrap();
    let hud = HudBindings::load(&registry, &[]).unwrap();
    for source in [RuleSetId::QuakeWorld] {
        let id = ClientId(3);
        let mut player =
            PlayerState::with_capacity(registry.items.len() + 1, 32, hud.values.capacity());
        player.movement_rules = RuleSetId::Quake3;
        player.trace_rules = RuleSetId::Quake2;
        player.weapon_state = Some(WeaponState {
            source: RuleSetId::Quake3,
            namespace: ResourceNamespace::Module(ModuleId(7)),
            readiness: WeaponReadiness::RemainingMilliseconds(-17),
        });
        let readiness = player.weapon_state;
        let time = EventTime::from_milliseconds(2500);
        for (index, bits) in [
            (0, 91),
            (1, 4),
            (2, 19),
            (4, 75),
            (5, 6),
            (6, 13),
            (7, 21),
            (8, 4),
            (9, 9),
            (10, 2),
            (11, 3),
            (12, 8),
            (13, 1),
            (14, 2),
            (15, 2 | 16384 | 131072 | 4194304 | (5 << 28)),
            (31, 0xfffffeff),
        ] {
            assert!(hud.import_native_stat(source, id, index, bits, &mut player, time));
        }
        assert_eq!(
            (player.health, player.armor, player.frags, player.score),
            (91, 75, 4, 4)
        );
        assert_eq!(
            player.view_weapon.model,
            ModelRef::native(ResourceNamespace::Client(id), 19)
        );
        assert_eq!(player.view_weapon.frame, 6);
        assert_eq!(player.inventory.collectibles, 5);
        assert_eq!(player.weapon_state, readiness);
        assert_eq!(player.movement_rules, RuleSetId::Quake3);
        assert_eq!(player.trace_rules, RuleSetId::Quake2);
        let values = hud.values.native(source);
        assert_eq!(values.stats[31].export(&player.values), Some(0xfffffeff));
        assert_eq!(values.stats[11].export(&player.values), Some(3));
        let weapon = registry.native_weapon(source, 2).unwrap();
        assert_eq!(player.inventory.weapon, weapon);
        let item = registry.weapon(weapon).unwrap().item;
        assert_eq!(player.inventory.item_acquired_at[item.0 as usize], 2.5);
        // An unchanged native stat never restarts the HUD acquisition lease.
        assert!(hud.import_native_stat(
            source,
            id,
            15,
            2 | 16384 | 131072 | 4194304 | (5 << 28),
            &mut player,
            EventTime::from_milliseconds(4000)
        ));
        assert_eq!(player.inventory.item_acquired_at[item.0 as usize], 2.5);
        let mut state = hud.state(32, NameId(0));
        let mut publication = qa_session::clients::HudPublication::with_capacity(
            player.inventory.item_counts.len(),
            32,
        );
        let mut ring = EventRing::load(4096, 1, 1, 32).unwrap();
        let consumer = ring.bind(OutputTarget::Client(id)).unwrap();
        hud.project_stats(&player, &mut state);
        publication.publish_changes(&player, &mut ring, id);
        let retained = ring.len();
        publication.publish_changes(&player, &mut ring, id);
        assert_eq!(ring.len(), retained);
        dispatch_consumer(
            &mut ring,
            consumer,
            &mut |record: OutputRecord, _: &mut qa_core::events::TextStore| {
                let FrameEvent::Hud(event) = record.event else {
                    panic!("HUD inventory record");
                };
                assert!(hud.apply(&mut state, event.change));
                OutputSubmission::BestEffort
            },
        );
        assert_eq!((state.health, state.armor, state.frags), (91, 75, 4));
        assert_eq!(state.inventory.weapon, weapon);
        assert_eq!(state.ammo, 13);
        assert!(state.owned_weapons.contains(weapon.0 as usize));
        assert_eq!(state.inventory.collectibles, 5);
        assert!(hud.import_native_stat(source, id, 2, 0, &mut player, time));
        assert_eq!(player.view_weapon.model, None);
    }
}

#[test]
fn other_protocol_stat_arrays_are_not_reinterpreted_as_q1_fields() {
    let names = NameTable::load(Registry::names_needed()).unwrap();
    let registry = Registry::load(&names).unwrap();
    let hud = HudBindings::load(&registry, &[]).unwrap();
    let mut player =
        PlayerState::with_capacity(registry.items.len() + 1, 32, hud.values.capacity());
    player.health = 71;
    player.armor = 33;
    player.view_weapon.frame = 7;
    for source in [
        RuleSetId::Quake2,
        RuleSetId::Quake2Rerelease,
        RuleSetId::Quake3,
    ] {
        assert!(hud.import_native_stat(
            source,
            ClientId(0),
            0,
            1234,
            &mut player,
            EventTime::default()
        ));
        assert_eq!(
            (player.health, player.armor, player.view_weapon.frame),
            (71, 33, 7)
        );
        assert_eq!(
            hud.values.native(source).stats[0].export(&player.values),
            Some(1234)
        );
    }
}

#[test]
fn netquake_unknown_stat_fifteen_does_not_replace_clientdata_inventory() {
    let names = NameTable::load(Registry::names_needed()).unwrap();
    let registry = Registry::load(&names).unwrap();
    let hud = HudBindings::load(&registry, &[]).unwrap();
    let mut player =
        PlayerState::with_capacity(registry.items.len() + 1, 32, hud.values.capacity());
    player.inventory.weapon = WeaponId(3);
    player.inventory.collectibles = 5;
    player.inventory.item_counts[1] = 7;
    let time = EventTime::from_milliseconds(1000);
    assert!(hud.import_native_stat(RuleSetId::Quake, ClientId(0), 0, 91, &mut player, time));
    assert!(hud.import_native_stat(RuleSetId::Quake, ClientId(0), 4, 75, &mut player, time));
    assert!(hud.import_native_stat(
        RuleSetId::Quake,
        ClientId(0),
        15,
        0xffffffff,
        &mut player,
        time
    ));
    assert_eq!((player.health, player.armor), (91, 75));
    assert_eq!(player.inventory.weapon, WeaponId(3));
    assert_eq!(player.inventory.collectibles, 5);
    assert_eq!(player.inventory.item_counts[1], 7);
    assert_eq!(
        hud.values.native(RuleSetId::Quake).stats[15].export(&player.values),
        Some(0xffffffff)
    );
}

#[test]
fn netquake_clientdata_bank_enters_one_inventory_and_hud_at_queued_receipt_time() {
    let names = NameTable::load(Registry::names_needed()).unwrap();
    let registry = Registry::load(&names).unwrap();
    let hud = HudBindings::load(&registry, &[]).unwrap();
    let source = RuleSetId::Quake;
    let client = ClientId(3);
    let values = hud.values.native(source);
    let mut player =
        PlayerState::with_capacity(registry.items.len() + 1, 32, hud.values.capacity());
    player.movement_rules = RuleSetId::Quake3;
    player.trace_rules = RuleSetId::Quake2;
    player.frags = 19;
    player.weapon_state = Some(WeaponState {
        source: RuleSetId::Quake3,
        namespace: ResourceNamespace::Module(ModuleId(7)),
        readiness: WeaponReadiness::RemainingMilliseconds(-17),
    });
    let readiness = player.weapon_state;
    let foreign = registry.native_item(RuleSetId::Quake3, 1).unwrap();
    player.inventory.item_counts[foreign.0 as usize] = 9;
    values.stats[1].import(&mut player.values, 23);
    values.stats[15].import(&mut player.values, u32::MAX);
    for (index, bits) in [
        (0, 91),
        (2, 19),
        (3, 17),
        (4, 75),
        (5, 6),
        (6, 42),
        (7, 0),
        (8, 255),
        (9, 1),
        (10, 2),
    ] {
        values.stats[index].import(&mut player.values, bits);
    }
    let bits = 2 | 16384 | 131072 | 0x0800_0000 | (5 << 28);
    values.items.unwrap().import(&mut player.values, bits);
    let time = EventTime::from_milliseconds(2500);
    assert!(hud.import_native_packet(source, client, &mut player, time));
    let weapon = registry.native_weapon(source, 2).unwrap();
    let weapon_item = registry.weapon(weapon).unwrap().item;
    let shells = registry.native_item(source, 11).unwrap();
    assert_eq!((player.health, player.armor, player.frags), (91, 75, 19));
    assert_eq!(
        player.view_weapon.model,
        ModelRef::native(ResourceNamespace::Client(client), 19)
    );
    assert_eq!(player.view_weapon.frame, 6);
    assert_eq!(player.inventory.weapon, weapon);
    assert_eq!(player.inventory.item_counts[shells.0 as usize], 42);
    assert_eq!(
        player.inventory.item_acquired_at[weapon_item.0 as usize],
        2.5
    );
    assert_eq!(player.inventory.collectibles, 5);
    assert_eq!(player.inventory.item_counts[foreign.0 as usize], 9);
    assert_eq!(player.weapon_state, readiness);
    assert_eq!(
        (player.movement_rules, player.trace_rules),
        (RuleSetId::Quake3, RuleSetId::Quake2)
    );
    assert!(hud.import_native_packet(
        source,
        client,
        &mut player,
        EventTime::from_milliseconds(4000)
    ));
    assert_eq!(
        player.inventory.item_acquired_at[weapon_item.0 as usize],
        2.5
    );

    let mut state = hud.state(32, NameId(0));
    let mut publication =
        qa_session::clients::HudPublication::with_capacity(player.inventory.item_counts.len(), 32);
    let mut ring = EventRing::load(4096, 1, 1, 32).unwrap();
    let consumer = ring.bind(OutputTarget::Client(client)).unwrap();
    hud.project_stats(&player, &mut state);
    publication.publish_changes(&player, &mut ring, client);
    dispatch_consumer(
        &mut ring,
        consumer,
        &mut |record: OutputRecord, _: &mut qa_core::events::TextStore| {
            let FrameEvent::Hud(event) = record.event else {
                panic!("HUD inventory record");
            };
            assert!(hud.apply(&mut state, event.change));
            OutputSubmission::BestEffort
        },
    );
    assert_eq!(state.inventory.weapon, weapon);
    assert_eq!(state.inventory.collectibles, 5);
    assert_eq!(
        state.inventory.item_acquired_at[weapon_item.0 as usize],
        2.5
    );
    assert_eq!(values.stats[3].export(&state.values), Some(17));
    assert_eq!(values.stats[6].export(&state.values), Some(42));
    assert_eq!(values.stats[1].export(&state.values), Some(23));
    assert_eq!(values.stats[15].export(&state.values), Some(u32::MAX));
    assert_eq!(values.items.unwrap().export(&state.values), Some(bits));
    assert_eq!(state.clipped_values, 0);

    values.items.unwrap().import(&mut player.values, 0);
    values.stats[6].import(&mut player.values, 0);
    assert!(hud.import_native_packet(
        source,
        client,
        &mut player,
        EventTime::from_milliseconds(5000)
    ));
    assert_eq!(player.inventory.item_counts[weapon_item.0 as usize], 0);
    assert_eq!(player.inventory.item_counts[shells.0 as usize], 0);
    assert_eq!(player.inventory.collectibles, 0);
    assert_eq!(player.inventory.item_counts[foreign.0 as usize], 9);
    assert_eq!(values.stats[15].export(&player.values), Some(u32::MAX));
}

#[test]
fn clientdata_capacity_admission_precedes_all_common_inventory_and_scalar_writes() {
    let names = NameTable::load(Registry::names_needed()).unwrap();
    let registry = Registry::load(&names).unwrap();
    let hud = HudBindings::load(&registry, &[]).unwrap();
    for (items, bank) in [(1, hud.values.capacity()), (registry.items.len() + 1, 32)] {
        let mut player = PlayerState::with_capacity(items, 32, bank);
        player.health = 81;
        player.armor = 53;
        player.inventory.collectibles = 9;
        player.inventory.item_counts[0] = 7;
        let values = hud.values.native(RuleSetId::Quake);
        values.stats[0].import(&mut player.values, 91);
        values.stats[4].import(&mut player.values, 75);
        values.items.unwrap().import(&mut player.values, u32::MAX);
        assert!(!hud.import_native_packet(
            RuleSetId::Quake,
            ClientId(0),
            &mut player,
            EventTime::default()
        ));
        assert_eq!((player.health, player.armor), (81, 53));
        assert_eq!(player.inventory.collectibles, 9);
        assert_eq!(player.inventory.item_counts[0], 7);
    }
}
