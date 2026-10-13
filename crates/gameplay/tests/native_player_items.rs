use qa_core::{names::NameTable, primitives::*, sys_events::EventTime};
use qa_gameplay::registry::{NativePlayerBindings, Registry};

#[test]
fn clientdata_masks_use_the_shared_inventory_import_and_keep_raw_flags() {
    let names = NameTable::load(Registry::names_needed()).unwrap();
    let registry = Registry::load(&names).unwrap();
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let native = NativePlayerBindings::load(rules, &registry).unwrap();
        let mut player = PlayerState::with_capacity(registry.items.len() + 1, 0, 1);
        player.health = 83;
        player.movement_rules = RuleSetId::Quake3;
        player.trace_rules = RuleSetId::Quake2;
        let ammo = registry.native_item(rules, 11).unwrap();
        player.inventory.item_counts[ammo.0 as usize] = 0;
        let binding = ValueBinding {
            id: ValueId(0),
            width: ValueWidth::Signed32,
            reset: ValueReset::Life,
        };
        // The raw ammo/extension bits remain independent of zero ammo counts.
        let bits = 2 | 256 | 16384 | 131072 | 0x0800_0000 | (5 << 28);
        assert!(native.import_items(
            Some(binding),
            bits,
            &mut player,
            EventTime::from_milliseconds(2500)
        ));
        assert_eq!(binding.export(&player.values), Some(bits));
        assert_eq!(player.inventory.item_counts[ammo.0 as usize], 0);
        let weapon = registry.native_weapon(rules, 2).unwrap();
        let item = registry.weapon(weapon).unwrap().item;
        assert_eq!(player.inventory.item_counts[item.0 as usize], 1);
        assert_eq!(player.inventory.item_acquired_at[item.0 as usize], 2.5);
        assert!(native.import_items(
            Some(binding),
            bits,
            &mut player,
            EventTime::from_milliseconds(4000)
        ));
        assert_eq!(player.inventory.item_acquired_at[item.0 as usize], 2.5);
        assert_eq!(player.inventory.collectibles, 5);
        assert_eq!(player.health, 83);
        assert_eq!(
            (player.movement_rules, player.trace_rules),
            (RuleSetId::Quake3, RuleSetId::Quake2)
        );
        assert!(native.import_items(
            Some(binding),
            0,
            &mut player,
            EventTime::from_milliseconds(5000)
        ));
        assert_eq!(player.inventory.item_counts[item.0 as usize], 0);
        assert_eq!(player.inventory.collectibles, 0);
    }
}

#[test]
fn item_mask_rejection_does_not_partially_commit_raw_or_inventory_state() {
    let names = NameTable::load(Registry::names_needed()).unwrap();
    let registry = Registry::load(&names).unwrap();
    let native = NativePlayerBindings::load(RuleSetId::Quake, &registry).unwrap();
    let binding = ValueBinding {
        id: ValueId(0),
        width: ValueWidth::Signed32,
        reset: ValueReset::Life,
    };
    let mut too_few_items = PlayerState::with_capacity(1, 0, 1);
    binding.import(&mut too_few_items.values, 77);
    assert!(!native.import_items(
        Some(binding),
        u32::MAX,
        &mut too_few_items,
        EventTime::default()
    ));
    assert_eq!(binding.export(&too_few_items.values), Some(77));
    let mut no_values = PlayerState::with_capacity(registry.items.len() + 1, 0, 0);
    no_values.inventory.item_counts[1] = 7;
    no_values.inventory.collectibles = 9;
    assert!(!native.import_items(
        Some(binding),
        u32::MAX,
        &mut no_values,
        EventTime::default()
    ));
    assert_eq!(no_values.inventory.item_counts[1], 7);
    assert_eq!(no_values.inventory.collectibles, 9);
}

fn q3_array_bindings() -> [[ValueBinding; 16]; 4] {
    std::array::from_fn(|array| {
        std::array::from_fn(|ordinal| ValueBinding {
            id: ValueId((array * 16 + ordinal) as u32),
            width: ValueWidth::Signed32,
            reset: if array == 1 {
                ValueReset::Session
            } else {
                ValueReset::Life
            },
        })
    })
}
fn q3_values(arrays: &[[ValueBinding; 16]; 4]) -> PlayerValueBindings<'_> {
    PlayerValueBindings {
        arrays: arrays.each_ref().map(|array| array.as_slice()),
        items: None,
    }
}

#[test]
fn q3_weapon_mask_and_signed_ammo_use_registry_items_without_changing_other_roles() {
    let names = NameTable::load(Registry::names_needed()).unwrap();
    let registry = Registry::load(&names).unwrap();
    let native = NativePlayerBindings::load(RuleSetId::Quake3, &registry).unwrap();
    let arrays = q3_array_bindings();
    let mut player = PlayerState::with_capacity(registry.items.len() + 1, 16, 64);
    player.health = 81;
    player.armor = 53;
    player.movement_rules = RuleSetId::Quake2;
    player.trace_rules = RuleSetId::Quake;
    player.inventory.collectibles = 7;
    player.inventory.powerup_until[3] = 12.5;
    player.view_weapon.frame = 17;
    player.weapon_state = Some(WeaponState {
        source: RuleSetId::Quake2,
        namespace: ResourceNamespace::Module(ModuleId(7)),
        readiness: WeaponReadiness::RemainingMilliseconds(-17),
    });
    let readiness = player.weapon_state;
    let rocket = registry.native_weapon(RuleSetId::Quake3, 5).unwrap();
    player.inventory.weapon = rocket;
    let foreign = registry.native_item(RuleSetId::Quake, 11).unwrap();
    player.inventory.item_counts[foreign.0 as usize] = 23;
    let mask = (1 << 1) | (1 << 2) | (1 << 3) | (1 << 5) | (1 << 10) | (1 << 11) | (1 << 15);
    arrays[0][2].import(&mut player.values, mask);
    for (ordinal, count) in [(2, 31), (3, 16_777_219), (5, -1), (11, -17), (15, -345)] {
        arrays[2][ordinal].import(&mut player.values, count as u32);
    }
    arrays[3][3].import(&mut player.values, 12345);
    assert!(native.packet_fits(q3_values(&arrays), Q3StatLayout::BASE, &player));
    assert!(native.import_packet(
        q3_values(&arrays),
        Q3StatLayout::BASE,
        ResourceNamespace::Module(ModuleId(7)),
        &mut player,
        EventTime::from_milliseconds(2500),
    ));
    for (ordinal, expected) in [(2, 31), (3, 16_777_219), (5, -1), (11, -17)] {
        let weapon = registry.native_weapon(RuleSetId::Quake3, ordinal).unwrap();
        let entry = registry.weapon(weapon).unwrap();
        let ammo = entry.ammo.unwrap();
        assert_eq!(player.inventory.item_counts[ammo.0 as usize], expected);
        assert_eq!(player.inventory.item_counts[entry.item.0 as usize], 1);
        assert_eq!(
            player.inventory.item_acquired_at[entry.item.0 as usize],
            2.5
        );
    }
    for ordinal in [1, 10] {
        let entry = registry
            .weapon(registry.native_weapon(RuleSetId::Quake3, ordinal).unwrap())
            .unwrap();
        assert_eq!(entry.ammo, None);
        assert_eq!(player.inventory.item_counts[entry.item.0 as usize], 1);
    }
    assert_eq!(arrays[2][15].export(&player.values), Some((-345i32) as u32));
    assert_eq!(arrays[0][2].export(&player.values), Some(mask));
    assert_eq!(arrays[3][3].export(&player.values), Some(12345));
    assert_eq!(player.inventory.item_counts[foreign.0 as usize], 23);
    assert_eq!(
        (player.health, player.armor, player.view_weapon.frame),
        (81, 53, 17)
    );
    assert_eq!(
        (player.movement_rules, player.trace_rules),
        (RuleSetId::Quake2, RuleSetId::Quake)
    );
    assert_eq!(player.inventory.weapon, rocket);
    assert_eq!(player.inventory.collectibles, 7);
    assert_eq!(player.inventory.powerup_until[3], 12.5);
    assert_eq!(player.weapon_state, readiness);
}

#[test]
fn q3_stat_layout_and_acquisition_time_are_explicit_and_do_not_republish_unchanged_items() {
    let names = NameTable::load(Registry::names_needed()).unwrap();
    let registry = Registry::load(&names).unwrap();
    let native = NativePlayerBindings::load(RuleSetId::Quake3, &registry).unwrap();
    let arrays = q3_array_bindings();
    let machinegun = registry
        .weapon(registry.native_weapon(RuleSetId::Quake3, 2).unwrap())
        .unwrap();
    let rocket = registry
        .weapon(registry.native_weapon(RuleSetId::Quake3, 5).unwrap())
        .unwrap();
    let mut player = PlayerState::with_capacity(registry.items.len() + 1, 0, 64);
    arrays[0][2].import(&mut player.values, 1 << 2);
    arrays[0][3].import(&mut player.values, 1 << 5);
    arrays[2][2].import(&mut player.values, 7);
    let import = |player: &mut PlayerState, layout, milliseconds| {
        assert!(native.import_packet(
            q3_values(&arrays),
            layout,
            ResourceNamespace::Client(ClientId(3)),
            player,
            EventTime::from_milliseconds(milliseconds)
        ));
    };
    import(&mut player, Q3StatLayout::BASE, 2500);
    assert_eq!(player.inventory.item_counts[machinegun.item.0 as usize], 1);
    assert_eq!(player.inventory.item_counts[rocket.item.0 as usize], 0);
    import(&mut player, Q3StatLayout::BASE, 4000);
    assert_eq!(
        player.inventory.item_acquired_at[machinegun.item.0 as usize],
        2.5
    );
    assert_eq!(
        player.inventory.item_acquired_at[machinegun.ammo.unwrap().0 as usize],
        2.5
    );
    import(&mut player, Q3StatLayout::TEAM_ARENA, 5000);
    assert_eq!(player.inventory.item_counts[machinegun.item.0 as usize], 0);
    assert_eq!(player.inventory.item_counts[rocket.item.0 as usize], 1);
    assert_eq!(
        player.inventory.item_acquired_at[rocket.item.0 as usize],
        5.0
    );
    import(&mut player, Q3StatLayout::BASE, 6000);
    assert_eq!(
        player.inventory.item_acquired_at[machinegun.item.0 as usize],
        6.0
    );
}

#[test]
fn q3_packet_preflights_every_bound_bank_row_and_both_inventory_arrays() {
    let names = NameTable::load(Registry::names_needed()).unwrap();
    let registry = Registry::load(&names).unwrap();
    let native = NativePlayerBindings::load(RuleSetId::Quake3, &registry).unwrap();
    let arrays = q3_array_bindings();
    let item_capacity = registry.items.len() + 1;
    for (counts, times, bank) in [
        (1, item_capacity, 64),
        (item_capacity, 1, 64),
        (item_capacity, item_capacity, 63),
    ] {
        let mut player = PlayerState::with_capacity(counts, 0, bank);
        player.inventory.item_acquired_at = vec![0.0; times].into_boxed_slice();
        player.inventory.item_counts[0] = 23;
        player.inventory.item_acquired_at[0] = 4.0;
        player.inventory.collectibles = 7;
        player.health = 81;
        arrays[0][2].import(&mut player.values, u32::MAX);
        arrays[2][2].import(&mut player.values, 99);
        let before_counts = player.inventory.item_counts.to_vec();
        let before_times = player.inventory.item_acquired_at.to_vec();
        assert!(!native.packet_fits(q3_values(&arrays), Q3StatLayout::BASE, &player));
        assert!(!native.import_packet(
            q3_values(&arrays),
            Q3StatLayout::BASE,
            ResourceNamespace::Module(ModuleId(1)),
            &mut player,
            EventTime::from_milliseconds(2500)
        ));
        assert_eq!(player.inventory.item_counts.as_ref(), before_counts);
        assert_eq!(player.inventory.item_acquired_at.as_ref(), before_times);
        assert_eq!((player.health, player.inventory.collectibles), (81, 7));
        assert_eq!(arrays[0][2].export(&player.values), Some(u32::MAX));
        assert_eq!(arrays[2][2].export(&player.values), Some(99));
    }
    let mut player = PlayerState::with_capacity(item_capacity, 0, 64);
    let short_ammo = PlayerValueBindings {
        arrays: [&arrays[0], &arrays[1], &arrays[2][..15], &arrays[3]],
        items: None,
    };
    assert!(!native.import_packet(
        short_ammo,
        Q3StatLayout::BASE,
        ResourceNamespace::Client(ClientId(3)),
        &mut player,
        EventTime::default()
    ));
    let invalid_layout = Q3StatLayout {
        health: 0,
        weapons: 16,
        armor: 3,
    };
    assert!(!native.import_packet(
        q3_values(&arrays),
        invalid_layout,
        ResourceNamespace::Client(ClientId(3)),
        &mut player,
        EventTime::default()
    ));
}

#[test]
fn quakeworld_bulk_playerinfo_does_not_reinterpret_incremental_stats_or_inventory() {
    let names = NameTable::load(Registry::names_needed()).unwrap();
    let registry = Registry::load(&names).unwrap();
    let native = NativePlayerBindings::load(RuleSetId::QuakeWorld, &registry).unwrap();
    let stats: [ValueBinding; 32] = std::array::from_fn(|ordinal| ValueBinding {
        id: ValueId(ordinal as u32),
        width: ValueWidth::Signed32,
        reset: ValueReset::Life,
    });
    let values = PlayerValueBindings {
        arrays: [&stats, &[], &[], &[]],
        items: None,
    };
    let mut player = PlayerState::with_capacity(1, 0, 32);
    player.health = 77;
    player.view_weapon.frame = 5;
    player.inventory.item_counts[0] = 23;
    stats[0].import(&mut player.values, 99);
    stats[5].import(&mut player.values, 17);
    stats[15].import(&mut player.values, u32::MAX);
    assert!(native.import_packet(
        values,
        Q3StatLayout::BASE,
        ResourceNamespace::Client(ClientId(3)),
        &mut player,
        EventTime::default()
    ));
    assert_eq!(
        (
            player.health,
            player.view_weapon.frame,
            player.inventory.item_counts[0]
        ),
        (77, 5, 23)
    );
    assert_eq!(stats[15].export(&player.values), Some(u32::MAX));
}
