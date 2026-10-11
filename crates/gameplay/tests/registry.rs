use qa_core::names::NameTable;
use qa_core::primitives::RuleSetId;
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
    assert_eq!(registry.items.len(), 116);
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
        if let Some(ammo) = weapon.ammo {
            assert!(matches!(
                registry.item(ammo).unwrap().kind,
                ItemKind::Ammo | ItemKind::WeaponAmmo
            ));
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
