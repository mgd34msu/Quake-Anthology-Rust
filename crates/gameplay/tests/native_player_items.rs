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
