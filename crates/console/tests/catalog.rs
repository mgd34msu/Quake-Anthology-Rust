use qa_console::{
    cvars::Cvars,
    cvars_generated::{BINDINGS, DEFINITIONS},
};

#[test]
fn every_canonical_alias_and_seat_has_a_slot_and_seats_remain_independent() {
    assert_eq!(DEFINITIONS.len(), 1260);
    let mut cvars = Cvars::new();
    assert_eq!(cvars.entries().count(), 1293);
    for binding in BINDINGS {
        if binding.scope == qa_console::catalog::Scope::Server {
            continue;
        }
        assert!(cvars.find(binding.name).is_some(), "{}", binding.name);
    }
    let first = cvars.find("ui_seat1_language").unwrap();
    let second = cvars.find("ui_seat2_language").unwrap();
    assert_ne!(first, second);
    cvars.set_text(first, "Alice");
    cvars.set_text(second, "Bob");
    assert_eq!(cvars.text(first), "Alice");
    assert_eq!(cvars.text(second), "Bob");
    let developer = cvars.find("DEVELOPER").unwrap();
    assert_eq!(cvars.value(developer), 0.0);
    cvars.set(developer, 1.0);
    assert_eq!(cvars.value(developer), 1.0);
    assert_eq!(cvars.find("FOV"), cvars.find("cg_fov"));
}
