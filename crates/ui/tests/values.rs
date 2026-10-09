//! Original native field widths/counts and common immutable HUD snapshots.
use qa_core::{names::NameTable, primitives::*};
use qa_gameplay::registry::Registry;
use qa_ui::{hud::HudBindings, values::*};

#[test]
fn native_highest_slots_and_signed_values_survive_common_snapshots() {
    let names = NameTable::load(Registry::names_needed()).unwrap();
    let registry = Registry::load(&names).unwrap();
    let bindings = HudBindings::load(&registry, &[]).unwrap();
    assert_eq!(bindings.values.capacity(), 192);
    let mut player =
        PlayerState::with_capacity(registry.items.len() + 1, 16, bindings.values.capacity());
    let mut state = bindings.state(16, NameId(1));
    let mut texts = qa_core::events::TextStore::load(1, 32).unwrap();
    let lease = texts.insert(b"layout").unwrap();
    let text = lease.id();
    state.layout_text = Some(lease);
    for (rules, slots, persistent) in [
        (RuleSetId::Quake, 32, 0),
        (RuleSetId::QuakeWorld, 32, 0),
        (RuleSetId::Quake2, 32, 0),
        (RuleSetId::Quake2Rerelease, 64, 0),
        (RuleSetId::Quake3, 16, 16),
    ] {
        let table = bindings.values.native(rules);
        assert_eq!(table.stats.len(), slots);
        assert_eq!(table.persistent.len(), persistent);
        for field in table.stats.iter().chain(&table.persistent) {
            for value in [i32::MIN, -32768, -1, 0, 1234, 32767, i32::MAX] {
                assert!(field.import(&mut player.values, value as u32));
                bindings.update(&player, &mut state);
                let expected = if field.width == ValueWidth::Signed16 {
                    i32::from(value as i16)
                } else {
                    value
                };
                assert_eq!(state.values.get(field.id).unwrap().as_integer(), expected);
                assert_eq!(
                    field.export(&state.values).unwrap(),
                    if field.width == ValueWidth::Signed16 {
                        u32::from(value as u16)
                    } else {
                        value as u32
                    }
                );
            }
        }
    }
    let q2 = bindings.values.native(RuleSetId::Quake2).stats[18];
    q2.import(&mut player.values, 1234);
    bindings.update(&player, &mut state);
    assert_eq!(state.values.get(q2.id).unwrap().as_integer(), 1234);
    assert_eq!(state.layout_text.as_ref().map(TextLease::id), Some(text));
    assert_eq!(state.clipped_values, 0);
    assert_ne!(q2.id, bindings.values.native(RuleSetId::Quake3).stats[2].id);
    let held = bindings.values.native(RuleSetId::Quake3).persistent[15];
    assert!(held.import(&mut player.values, 0x8123_4567));
    bindings
        .values
        .native(RuleSetId::Quake3)
        .reset_life(&mut player.values);
    assert_eq!(held.export(&player.values), Some(0x8123_4567));
    assert_eq!(q2.export(&player.values), Some(1234));
    player.reset();
    state.reset(&mut texts);
    assert!(texts.get(text).is_none());
    assert_eq!(player.values.capacity(), 192);
    assert_eq!(state.values.capacity(), 192);
    assert_eq!(held.export(&player.values), Some(0));
}

#[test]
fn extension_float_bits_and_capacity_failures_are_independent_of_native_rules() {
    let layout = ValueLayout::load(&[ExtensionValue {
        name: NameId(42),
        width: ValueWidth::Float32,
        reset: ValueReset::Session,
    }])
    .unwrap();
    let field = layout.extensions[0].1;
    let mut source = ValueBank::load(layout.capacity());
    let mut snapshot = ValueBank::load(layout.capacity());
    for bits in [0, 0x8000_0000, 0x3f80_0001, 0x7fc0_1234, 0x7f80_0000] {
        assert!(field.import(&mut source, bits));
        assert_eq!(snapshot.copy_from(&source), 0);
        assert_eq!(field.export(&snapshot), Some(bits));
        layout.reset_life(&mut source);
        assert_eq!(field.export(&source), Some(bits));
    }
    let mut small = ValueBank::load(192);
    assert_eq!(small.copy_from(&source), 1);
    assert_eq!(field.export(&small), None);
    assert!(!field.import(&mut small, 1));
    assert!(NativeValues::load(RuleSetId::Quake3, ValueId(u32::MAX)).is_none());
    let module = NativeValues::load(RuleSetId::Quake2, ValueId(0)).unwrap();
    assert_eq!(module.stats[18].id, ValueId(18));
}
