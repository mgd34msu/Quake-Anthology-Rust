use qa_core::{
    bindings::{BindingError, OrdinalBindings},
    primitives::{NameId, SoundId},
};

#[test]
fn primary_preference_and_rewrite_repair_the_numeric_inverse() {
    let mut rows = OrdinalBindings::load(8, 5, 0, false);
    rows.set(6, Some(NameId(2))).unwrap();
    rows.set(3, Some(NameId(2))).unwrap();
    assert_eq!(rows.ordinal(NameId(2)), Some(3));
    rows.prefer(6);
    assert_eq!(rows.ordinal(NameId(2)), Some(6));
    rows.set(6, Some(NameId(4))).unwrap();
    assert_eq!(rows.ordinal(NameId(2)), Some(3));
    assert_eq!(rows.ordinal(NameId(4)), Some(6));
    assert_eq!(rows.get(3), Some(NameId(2)));
    rows.set(3, None).unwrap();
    assert_eq!(rows.ordinal(NameId(2)), None);
    assert_eq!(rows.get(6), Some(NameId(4)));
}

#[test]
fn invalid_registration_preserves_existing_rows_and_zero_is_domain_data() {
    let mut wire = OrdinalBindings::load(3, 2, 2, false);
    wire.set(0, Some(SoundId(1))).unwrap();
    assert_eq!(wire.get(0), None);
    assert_eq!(wire.register(Some(NameId(0)), SoundId(0)), Ok(1));
    assert_eq!(wire.named(NameId(0)), Some(1));
    assert_eq!(wire.set(1, Some(SoundId(2))), Err(BindingError::Identity));
    assert_eq!(wire.set(3, None), Err(BindingError::Ordinal));
    assert_eq!(wire.get(1), Some(SoundId(0)));
    assert_eq!(wire.ordinal(SoundId(0)), Some(1));
    assert_eq!(wire.named(NameId(0)), Some(1));
    let mut engine = OrdinalBindings::load(3, 2, 2, true);
    assert_eq!(engine.register(Some(NameId(0)), SoundId(0)), Ok(0));
    assert_eq!(engine.get(0), Some(SoundId(0)));
}

#[test]
fn named_aliases_are_invalidated_by_replacement_and_cold_reattachment() {
    let mut rows = OrdinalBindings::load(4, 3, 4, true);
    assert_eq!(rows.register(Some(NameId(1)), SoundId(1)), Ok(0));
    assert_eq!(rows.register(Some(NameId(2)), SoundId(1)), Ok(1));
    assert_eq!(rows.ordinal(SoundId(1)), Some(0));
    rows.replace_named(NameId(1), SoundId(2)).unwrap();
    assert_eq!(rows.named(NameId(1)), Some(0));
    assert_eq!(rows.named(NameId(2)), Some(1));
    assert_eq!(rows.ordinal(SoundId(1)), Some(1));
    rows.set(0, None).unwrap();
    assert_eq!(rows.named(NameId(1)), None);
    rows.configure(5, 2);
    assert_eq!(rows.get(1), None);
    assert_eq!(rows.ordinal(SoundId(1)), None);
    assert_eq!(rows.named(NameId(2)), None);
    assert_eq!(rows.register(Some(NameId(1)), SoundId(4)), Ok(0));
}
