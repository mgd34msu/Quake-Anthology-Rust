use qa_core::names::NameTable;
use qa_core::primitives::{ItemId, NameId, RuleSetId, WeaponId};
use qa_gameplay::registry::{ItemKind, Registry, SOURCE_COUNTS};

#[test]
fn source_tables_share_ids_without_losing_original_numbers_or_ammo_links() {
    let names = NameTable::load(
        Registry::names_needed().chain([b"info_player_start".as_slice(), b"ITEM_SHELLS"]),
    )
    .unwrap();
    let registry = Registry::load(&names).unwrap();
    let counts: [usize; 5] = std::array::from_fn(|index| {
        registry
            .items
            .iter()
            .filter(|item| item.source == RuleSetId::ALL[index] && item.native < 65534)
            .count()
    });
    assert_eq!(counts, SOURCE_COUNTS);
    assert_eq!(counts, [22, 22, 41, 83, 51]);
    assert_eq!(registry.items.len(), 223);
    assert!(registry.items[..116].iter().all(|item| !matches!(
        item.source,
        RuleSetId::QuakeWorld | RuleSetId::Quake2Rerelease
    )));
    // Appending the new sources preserves all original common item IDs.
    assert_eq!(registry.native_item(RuleSetId::Quake, 1), Some(ItemId(1)));
    assert_eq!(registry.native_item(RuleSetId::Quake2, 1), Some(ItemId(25)));
    assert_eq!(registry.native_item(RuleSetId::Quake3, 1), Some(ItemId(66)));
    let q1_shells = registry
        .classname(RuleSetId::Quake, names.find(b"item_shells").unwrap())
        .unwrap();
    assert_eq!(
        registry.classname(RuleSetId::Quake, names.find(b"ITEM_SHELLS").unwrap()),
        None
    );
    let q3_shells = registry
        .classname(RuleSetId::Quake3, names.find(b"ammo_shells").unwrap())
        .unwrap();
    let conversion = registry.ammo_conversion(q1_shells, q3_shells).unwrap();
    assert_eq!((conversion.numerator, conversion.denominator), (1, 1));
    let q1_rocket = registry.native_weapon(RuleSetId::Quake, 32).unwrap();
    assert_eq!(
        names.get(
            registry
                .item(registry.weapon(q1_rocket).unwrap().item)
                .unwrap()
                .classname
        ),
        Some(b"weapon_rocketlauncher".as_slice())
    );
    for weapon in &registry.weapons {
        assert_eq!(
            registry.native_weapon(weapon.source, weapon.native),
            Some(weapon.id)
        );
        assert_eq!(
            registry.native_weapon_number(weapon.source, weapon.id),
            Some(weapon.native)
        );
        if let Some(ammo) = weapon.ammo {
            assert!(matches!(
                registry.item(ammo).unwrap().kind,
                ItemKind::Ammo | ItemKind::WeaponAmmo
            ));
        }
    }
    for item in &registry.items {
        assert_eq!(
            registry.native_item(item.source, item.native),
            Some(item.id)
        );
        if item.classname != NameId(0) {
            assert_eq!(
                registry.classname(item.source, item.classname),
                Some(item.id)
            );
        }
    }
    // Original Q2 itemlist's Health row has NULL classname; preserve its index.
    let health = registry
        .items
        .iter()
        .find(|item| {
            item.source == RuleSetId::Quake2 && names.get(item.label) == Some(b"Health".as_slice())
        })
        .unwrap();
    assert_eq!(
        registry.native_item(RuleSetId::Quake2, health.native),
        Some(health.id)
    );
}

#[test]
fn quakeworld_weapon_bits_and_source_identity_stay_native() {
    let names =
        NameTable::load(Registry::names_needed().chain([b"ITEM_SHELLS".as_slice()])).unwrap();
    let registry = Registry::load(&names).unwrap();
    for (number, classname) in [
        (1, "weapon_shotgun"),
        (2, "weapon_supershotgun"),
        (4, "weapon_nailgun"),
        (8, "weapon_supernailgun"),
        (16, "weapon_grenadelauncher"),
        (32, "weapon_rocketlauncher"),
        (64, "weapon_lightning"),
        (4096, "weapon_axe"),
    ] {
        let qw = registry
            .native_weapon(RuleSetId::QuakeWorld, number)
            .unwrap();
        let nq = registry.native_weapon(RuleSetId::Quake, number).unwrap();
        assert_ne!(qw, nq);
        let weapon = registry.weapon(qw).unwrap();
        assert_eq!(weapon.source, RuleSetId::QuakeWorld);
        assert_eq!(
            names.get(registry.item(weapon.item).unwrap().classname),
            Some(classname.as_bytes())
        );
        assert_eq!(
            registry.native_weapon_number(RuleSetId::QuakeWorld, qw),
            Some(number)
        );
        assert_eq!(registry.native_weapon_number(RuleSetId::Quake, qw), None);
    }
    assert_eq!(registry.native_weapon(RuleSetId::QuakeWorld, 0), None);
    assert_eq!(registry.native_weapon(RuleSetId::QuakeWorld, 3), None);
    assert_eq!(
        registry.classname(RuleSetId::QuakeWorld, names.find(b"ITEM_SHELLS").unwrap()),
        None
    );
}

#[test]
fn rerelease_ordinals_health_variants_and_ammo_links_use_its_active_table() {
    let names = NameTable::load(Registry::names_needed()).unwrap();
    let registry = Registry::load(&names).unwrap();
    for (number, classname, ammo) in [
        (8, "weapon_blaster", None),
        (10, "weapon_shotgun", Some((28, "ammo_shells"))),
        (20, "weapon_rocketlauncher", Some((31, "ammo_rockets"))),
        (27, "weapon_disintegrator", Some((37, "ammo_disruptor"))),
    ] {
        let id = registry
            .native_weapon(RuleSetId::Quake2Rerelease, number)
            .unwrap();
        let weapon = registry.weapon(id).unwrap();
        assert_eq!(
            names.get(registry.item(weapon.item).unwrap().classname),
            Some(classname.as_bytes())
        );
        assert_eq!(
            registry.native_weapon_number(RuleSetId::Quake2Rerelease, id),
            Some(number)
        );
        assert_eq!(registry.native_weapon_number(RuleSetId::Quake2, id), None);
        match ammo {
            Some((ordinal, classname)) => {
                let item = registry.item(weapon.ammo.unwrap()).unwrap();
                assert_eq!(item.source, RuleSetId::Quake2Rerelease);
                assert_eq!(item.native, ordinal);
                assert_eq!(names.get(item.classname), Some(classname.as_bytes()));
            }
            None => assert_eq!(weapon.ammo, None),
        }
    }
    // The same number identifies different native weapons in the two modules.
    for (source, classname) in [
        (RuleSetId::Quake2, "weapon_blaster"),
        (RuleSetId::Quake2Rerelease, "weapon_grapple"),
    ] {
        let weapon = registry
            .weapon(registry.native_weapon(source, 7).unwrap())
            .unwrap();
        assert_eq!(
            names.get(registry.item(weapon.item).unwrap().classname),
            Some(classname.as_bytes())
        );
    }
    for (ordinal, classname, amount) in [
        (72, "item_health_small", 2),
        (73, "item_health", 10),
        (74, "item_health_large", 25),
        (75, "item_health_mega", 100),
    ] {
        let item = registry
            .item(
                registry
                    .native_item(RuleSetId::Quake2Rerelease, ordinal)
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(item.kind, ItemKind::Health);
        assert_eq!(item.amount, amount);
        assert_eq!(names.get(item.classname), Some(classname.as_bytes()));
    }
    assert_eq!(registry.native_item(RuleSetId::Quake2Rerelease, 0), None);
    assert_eq!(registry.native_item(RuleSetId::Quake2Rerelease, 84), None);
    assert_eq!(names.find(b"weapon_beta_disintegrator"), None);
    let shells = registry
        .item(
            registry
                .native_item(RuleSetId::Quake2Rerelease, 28)
                .unwrap(),
        )
        .unwrap();
    assert_eq!(shells.maximum, 100);
    let disruptor = registry
        .item(
            registry
                .native_item(RuleSetId::Quake2Rerelease, 37)
                .unwrap(),
        )
        .unwrap();
    assert_eq!(disruptor.maximum, 12);
}

#[test]
fn additional_native_sources_use_authored_common_ammo_conversions() {
    let names = NameTable::load(Registry::names_needed()).unwrap();
    let registry = Registry::load(&names).unwrap();
    for (a, b) in [
        (
            (RuleSetId::QuakeWorld, "item_shells"),
            (RuleSetId::Quake2Rerelease, "ammo_shells"),
        ),
        (
            (RuleSetId::QuakeWorld, "item_spikes"),
            (RuleSetId::Quake2Rerelease, "ammo_bullets"),
        ),
        (
            (RuleSetId::QuakeWorld, "item_rockets"),
            (RuleSetId::Quake2Rerelease, "ammo_grenades"),
        ),
        (
            (RuleSetId::Quake2Rerelease, "ammo_cells"),
            (RuleSetId::Quake3, "ammo_bfg"),
        ),
        (
            (RuleSetId::Quake2Rerelease, "ammo_slugs"),
            (RuleSetId::Quake3, "ammo_slugs"),
        ),
    ] {
        let item = |(source, name): (RuleSetId, &str)| {
            registry
                .classname(source, names.find(name.as_bytes()).unwrap())
                .unwrap()
        };
        let a = item(a);
        let b = item(b);
        for (source, target) in [(a, b), (b, a)] {
            let link = registry.ammo_conversion(source, target).unwrap();
            assert_eq!((link.numerator, link.denominator), (1, 1));
        }
    }
}

#[test]
fn q3_command_weapon_mapping_drops_foreign_and_unrepresentable_values() {
    let names = NameTable::load(Registry::names_needed()).unwrap();
    let mut registry = Registry::load(&names).unwrap();
    assert_eq!(registry.command_weapon_number(None), 0);
    assert_eq!(registry.command_weapon_number(Some(WeaponId(0))), 0);
    assert_eq!(registry.command_weapon_number(Some(WeaponId(u32::MAX))), 0);
    for weapon in &registry.weapons {
        assert_eq!(
            registry.command_weapon_number(Some(weapon.id)),
            if weapon.source == RuleSetId::Quake3 {
                weapon.native as u8
            } else {
                0
            }
        );
    }
    let id = registry.native_weapon(RuleSetId::Quake3, 7).unwrap();
    registry.weapons[id.0 as usize - 1].native = 256;
    assert_eq!(registry.command_weapon_number(Some(id)), 0);
}
