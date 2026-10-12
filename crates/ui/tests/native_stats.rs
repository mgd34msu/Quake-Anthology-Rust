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
