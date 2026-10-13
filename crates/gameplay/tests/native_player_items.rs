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
fn clock(milliseconds: u64) -> Option<ClockObservation> {
    Some(ClockObservation {
        native: ThinkTime::Milliseconds(milliseconds as i64),
        queued: EventTime::from_milliseconds(milliseconds),
    })
}

#[test]
fn q3_weapon_mask_and_signed_ammo_use_registry_items_without_changing_other_roles() {
    let names = NameTable::load(Registry::names_needed()).unwrap();
    let registry = Registry::load(&names).unwrap();
    let native = NativePlayerBindings::load(RuleSetId::Quake3, &registry).unwrap();
    let arrays = q3_array_bindings();
    let mut player =
        PlayerState::with_capacity(registry.items.len() + 1, registry.powerup_capacity(), 64);
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
    assert!(native.packet_fits(q3_values(&arrays), Q3StatLayout::BASE, &player, clock(2500)));
    assert!(native.import_packet(
        q3_values(&arrays),
        Q3StatLayout::BASE,
        ResourceNamespace::Module(ModuleId(7)),
        &mut player,
        EventTime::from_milliseconds(2500),
        clock(2500),
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
    let mut player =
        PlayerState::with_capacity(registry.items.len() + 1, registry.powerup_capacity(), 64);
    arrays[0][2].import(&mut player.values, 1 << 2);
    arrays[0][3].import(&mut player.values, 1 << 5);
    arrays[2][2].import(&mut player.values, 7);
    let import = |player: &mut PlayerState, layout, milliseconds| {
        assert!(native.import_packet(
            q3_values(&arrays),
            layout,
            ResourceNamespace::Client(ClientId(3)),
            player,
            EventTime::from_milliseconds(milliseconds),
            clock(milliseconds),
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
        let mut player = PlayerState::with_capacity(counts, registry.powerup_capacity(), bank);
        player.inventory.item_acquired_at = vec![0.0; times].into_boxed_slice();
        player.inventory.item_counts[0] = 23;
        player.inventory.item_acquired_at[0] = 4.0;
        player.inventory.collectibles = 7;
        player.health = 81;
        arrays[0][2].import(&mut player.values, u32::MAX);
        arrays[2][2].import(&mut player.values, 99);
        let before_counts = player.inventory.item_counts.to_vec();
        let before_times = player.inventory.item_acquired_at.to_vec();
        assert!(!native.packet_fits(q3_values(&arrays), Q3StatLayout::BASE, &player, clock(2500)));
        assert!(!native.import_packet(
            q3_values(&arrays),
            Q3StatLayout::BASE,
            ResourceNamespace::Module(ModuleId(1)),
            &mut player,
            EventTime::from_milliseconds(2500),
            clock(2500),
        ));
        assert_eq!(player.inventory.item_counts.as_ref(), before_counts);
        assert_eq!(player.inventory.item_acquired_at.as_ref(), before_times);
        assert_eq!((player.health, player.inventory.collectibles), (81, 7));
        assert_eq!(arrays[0][2].export(&player.values), Some(u32::MAX));
        assert_eq!(arrays[2][2].export(&player.values), Some(99));
    }
    let mut player = PlayerState::with_capacity(item_capacity, registry.powerup_capacity(), 64);
    let short_ammo = PlayerValueBindings {
        arrays: [&arrays[0], &arrays[1], &arrays[2][..15], &arrays[3]],
        items: None,
    };
    assert!(!native.import_packet(
        short_ammo,
        Q3StatLayout::BASE,
        ResourceNamespace::Client(ClientId(3)),
        &mut player,
        EventTime::default(),
        clock(0),
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
        EventTime::default(),
        clock(0),
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
        EventTime::default(),
        None,
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

#[test]
fn q3_powerup_bindings_keep_native_ordinals_and_existing_item_indexed_timers() {
    let names = NameTable::load(Registry::names_needed()).unwrap();
    let registry = Registry::load(&names).unwrap();
    let classnames = [
        "item_quad",
        "item_enviro",
        "item_haste",
        "item_invis",
        "item_regen",
        "item_flight",
        "team_CTF_redflag",
        "team_CTF_blueflag",
        "team_CTF_neutralflag",
        "item_scout",
        "item_guard",
        "item_doubler",
        "item_ammoregen",
        "holdable_invulnerability",
    ];
    assert_eq!(registry.powerups.len(), classnames.len());
    let mut required = 0;
    for (index, classname) in classnames.into_iter().enumerate() {
        let entry = registry
            .native_powerup(RuleSetId::Quake3, index as u16 + 1)
            .unwrap();
        assert_eq!(entry.source, RuleSetId::Quake3);
        assert_eq!(u32::from(entry.id.0), entry.item.0);
        assert_eq!(
            names.get(registry.item(entry.item).unwrap().classname),
            Some(classname.as_bytes())
        );
        assert_eq!(entry.owns_item, index != 13);
        required = required.max(usize::from(entry.id.0) + 1);
    }
    assert_eq!(registry.powerup_capacity(), required);
    for rules in RuleSetId::ALL {
        assert_eq!(registry.native_powerup(rules, 0), None);
        assert_eq!(registry.native_powerup(rules, 15), None);
        if rules != RuleSetId::Quake3 {
            assert_eq!(registry.native_powerup(rules, 1), None);
        }
    }
}

#[test]
fn q3_powerup_deadlines_use_the_native_observation_and_keep_raw_and_foreign_state() {
    let names = NameTable::load(Registry::names_needed()).unwrap();
    let registry = Registry::load(&names).unwrap();
    let native = NativePlayerBindings::load(RuleSetId::Quake3, &registry).unwrap();
    let arrays = q3_array_bindings();
    let mut player =
        PlayerState::with_capacity(registry.items.len() + 1, registry.powerup_capacity(), 64);
    player.movement_rules = RuleSetId::Quake2Rerelease;
    player.trace_rules = RuleSetId::QuakeWorld;
    player.health = 81;
    player.armor = 53;
    player.inventory.collectibles = 7;
    let foreign = registry.native_item(RuleSetId::Quake, 16).unwrap();
    player.inventory.item_counts[foreign.0 as usize] = 3;
    player.inventory.powerup_until[foreign.0 as usize] = 90.0;
    let invulnerability = *registry.native_powerup(RuleSetId::Quake3, 14).unwrap();
    player.inventory.item_counts[invulnerability.item.0 as usize] = 2;
    player.inventory.item_acquired_at[invulnerability.item.0 as usize] = 1.25;
    for (ordinal, deadline) in [
        (0, -11),
        (1, 102_000),
        (2, 99_999),
        (3, -1500),
        (4, 100_000),
        (6, 101_000),
        (7, i32::MAX),
        (10, 100_000),
        (11, 100_000),
        (12, 100_000),
        (13, 100_000),
        (14, 100_000),
        (15, i32::MIN),
    ] {
        assert!(arrays[3][ordinal].import(&mut player.values, deadline as u32));
    }
    let observation = ClockObservation {
        native: ThinkTime::Milliseconds(100_000),
        queued: EventTime::from_milliseconds(5000),
    };
    assert!(native.import_packet(
        q3_values(&arrays),
        Q3StatLayout::BASE,
        ResourceNamespace::Module(ModuleId(8)),
        &mut player,
        observation.queued,
        Some(observation),
    ));
    for (ordinal, until, active) in [
        (1, 7.0, 1),
        (2, 4.999, 0),
        (3, -96.5, 0),
        (4, 5.0, 1),
        (5, 0.0, 0),
        (6, 6.0, 1),
        (7, 2_147_388.647, 1),
        (10, 5.0, 1),
        (11, 5.0, 1),
        (12, 5.0, 1),
        (13, 5.0, 1),
    ] {
        let entry = registry.native_powerup(RuleSetId::Quake3, ordinal).unwrap();
        assert_eq!(
            player.inventory.powerup_until[usize::from(entry.id.0)],
            until
        );
        assert_eq!(player.inventory.item_counts[entry.item.0 as usize], active);
        assert_eq!(
            player.inventory.item_acquired_at[entry.item.0 as usize],
            if active != 0 { 5.0 } else { 0.0 }
        );
    }
    assert_eq!(
        player.inventory.powerup_until[usize::from(invulnerability.id.0)],
        5.0
    );
    assert_eq!(
        player.inventory.item_counts[invulnerability.item.0 as usize],
        2
    );
    assert_eq!(
        player.inventory.item_acquired_at[invulnerability.item.0 as usize],
        1.25
    );
    assert_eq!(arrays[3][0].export(&player.values), Some((-11i32) as u32));
    assert_eq!(arrays[3][15].export(&player.values), Some(i32::MIN as u32));
    assert_eq!(
        (player.health, player.armor, player.inventory.collectibles),
        (81, 53, 7)
    );
    assert_eq!(
        (player.movement_rules, player.trace_rules),
        (RuleSetId::Quake2Rerelease, RuleSetId::QuakeWorld)
    );
    assert_eq!(player.inventory.item_counts[foreign.0 as usize], 3);
    assert_eq!(player.inventory.powerup_until[foreign.0 as usize], 90.0);

    let later = ClockObservation {
        native: ThinkTime::Milliseconds(100_500),
        queued: EventTime::from_milliseconds(5500),
    };
    assert!(native.import_packet(
        q3_values(&arrays),
        Q3StatLayout::TEAM_ARENA,
        ResourceNamespace::Client(ClientId(3)),
        &mut player,
        later.queued,
        Some(later)
    ));
    let quad = *registry.native_powerup(RuleSetId::Quake3, 1).unwrap();
    let invisibility = *registry.native_powerup(RuleSetId::Quake3, 4).unwrap();
    assert_eq!(player.inventory.powerup_until[usize::from(quad.id.0)], 7.0);
    assert_eq!(player.inventory.item_acquired_at[quad.item.0 as usize], 5.0);
    assert_eq!(
        player.inventory.item_counts[invisibility.item.0 as usize],
        0
    );
    assert_eq!(
        player.inventory.powerup_until[usize::from(invisibility.id.0)],
        5.0
    );
    for ordinal in [1, 14] {
        assert!(arrays[3][ordinal].import(&mut player.values, 0));
    }
    assert!(native.import_packet(
        q3_values(&arrays),
        Q3StatLayout::BASE,
        ResourceNamespace::Client(ClientId(3)),
        &mut player,
        later.queued,
        Some(later)
    ));
    assert_eq!(player.inventory.powerup_until[usize::from(quad.id.0)], 0.0);
    assert_eq!(player.inventory.item_counts[quad.item.0 as usize], 0);
    assert_eq!(
        player.inventory.powerup_until[usize::from(invulnerability.id.0)],
        0.0
    );
    assert_eq!(
        player.inventory.item_counts[invulnerability.item.0 as usize],
        2
    );
    assert!(arrays[3][1].import(&mut player.values, 110_000));
    let acquired_again = ClockObservation {
        native: ThinkTime::Milliseconds(105_000),
        queued: EventTime::from_milliseconds(10_000),
    };
    assert!(native.import_packet(
        q3_values(&arrays),
        Q3StatLayout::BASE,
        ResourceNamespace::Client(ClientId(3)),
        &mut player,
        acquired_again.queued,
        Some(acquired_again)
    ));
    assert_eq!(player.inventory.powerup_until[usize::from(quad.id.0)], 15.0);
    assert_eq!(
        player.inventory.item_acquired_at[quad.item.0 as usize],
        10.0
    );
}

#[test]
fn q3_powerup_and_clock_admission_precedes_every_inventory_change() {
    let names = NameTable::load(Registry::names_needed()).unwrap();
    let registry = Registry::load(&names).unwrap();
    let native = NativePlayerBindings::load(RuleSetId::Quake3, &registry).unwrap();
    let arrays = q3_array_bindings();
    let capacity = registry.powerup_capacity();
    let mut player = PlayerState::with_capacity(registry.items.len() + 1, capacity, 64);
    player.inventory.item_counts.fill(7);
    player.inventory.item_acquired_at.fill(1.25);
    player.inventory.powerup_until.fill(60.0);
    assert!(arrays[0][2].import(&mut player.values, u32::MAX));
    assert!(arrays[2][2].import(&mut player.values, 91));
    assert!(arrays[3][1].import(&mut player.values, 12_000));
    let counts = player.inventory.item_counts.to_vec();
    let acquired = player.inventory.item_acquired_at.to_vec();
    for invalid in [
        None,
        Some(ClockObservation {
            native: ThinkTime::Seconds(1.0),
            queued: EventTime::default(),
        }),
        Some(ClockObservation {
            native: ThinkTime::Seconds(f64::NAN),
            queued: EventTime::default(),
        }),
        Some(ClockObservation {
            native: ThinkTime::Seconds(f64::INFINITY),
            queued: EventTime::default(),
        }),
    ] {
        assert!(!native.packet_fits(q3_values(&arrays), Q3StatLayout::BASE, &player, invalid));
        assert!(!native.import_packet(
            q3_values(&arrays),
            Q3StatLayout::BASE,
            ResourceNamespace::Client(ClientId(3)),
            &mut player,
            EventTime::default(),
            invalid
        ));
        assert_eq!(player.inventory.item_counts.as_ref(), counts);
        assert_eq!(player.inventory.item_acquired_at.as_ref(), acquired);
        assert!(
            player
                .inventory
                .powerup_until
                .iter()
                .all(|until| *until == 60.0)
        );
        assert_eq!(arrays[3][1].export(&player.values), Some(12_000));
    }
    let short = PlayerValueBindings {
        arrays: [&arrays[0], &arrays[1], &arrays[2], &arrays[3][..15]],
        items: None,
    };
    assert!(!native.packet_fits(short, Q3StatLayout::BASE, &player, clock(5000)));
    assert!(!native.import_packet(
        short,
        Q3StatLayout::BASE,
        ResourceNamespace::Client(ClientId(3)),
        &mut player,
        EventTime::default(),
        clock(5000)
    ));
    assert_eq!(player.inventory.item_counts.as_ref(), counts);
    player.inventory.powerup_until = vec![60.0; capacity - 1].into_boxed_slice();
    assert!(!native.packet_fits(q3_values(&arrays), Q3StatLayout::BASE, &player, clock(5000)));
    assert!(!native.import_packet(
        q3_values(&arrays),
        Q3StatLayout::BASE,
        ResourceNamespace::Client(ClientId(3)),
        &mut player,
        EventTime::default(),
        clock(5000)
    ));
    assert_eq!(player.inventory.item_counts.as_ref(), counts);
    assert_eq!(player.inventory.item_acquired_at.as_ref(), acquired);
    assert!(
        player
            .inventory
            .powerup_until
            .iter()
            .all(|until| *until == 60.0)
    );
}
