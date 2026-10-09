use qa_console::{
    cvars::Cvars,
    cvars_generated::{BINDINGS, DEFINITIONS, ENGINE_DEFINITION_COUNT, OWNER_DEFINITION_COUNT},
};

#[test]
fn every_canonical_alias_and_seat_has_a_slot_and_seats_remain_independent() {
    assert_eq!(OWNER_DEFINITION_COUNT, 1260);
    assert_eq!(
        DEFINITIONS.len(),
        OWNER_DEFINITION_COUNT + ENGINE_DEFINITION_COUNT
    );
    let mut cvars = Cvars::new().unwrap();
    assert_eq!(cvars.entries().count(), 1293 + ENGINE_DEFINITION_COUNT);
    for binding in BINDINGS {
        if binding.scope == qa_console::catalog::Scope::Server {
            continue;
        }
        assert!(cvars.find(binding.name).is_some(), "{}", binding.name);
    }
    let first = cvars.find("ui_seat1_language").unwrap();
    let second = cvars.find("ui_seat2_language").unwrap();
    assert_ne!(first, second);
    cvars.set_text(first, "Alice").unwrap();
    cvars.set_text(second, "Bob").unwrap();
    assert_eq!(cvars.text(first), "Alice");
    assert_eq!(cvars.text(second), "Bob");
    let developer = cvars.find("DEVELOPER").unwrap();
    assert_eq!(cvars.value(developer), 0.0);
    cvars.set(developer, 1.0).unwrap();
    assert_eq!(cvars.value(developer), 1.0);
    assert_eq!(cvars.find("FOV"), cvars.find("cg_fov"));
}

#[test]
fn cpu_bands_is_an_archived_latched_extension_in_every_source_view() {
    use qa_console::views::{Context, RuleSetId};

    let cvars = Cvars::new().unwrap();
    let handle = cvars.find("r_cpuBands").unwrap();
    let row = DEFINITIONS
        .iter()
        .position(|row| row.name == "r_cpuBands")
        .unwrap();
    assert!(row >= OWNER_DEFINITION_COUNT);
    assert!(DEFINITIONS[row].sources.starts_with("engine-extension:"));
    assert_eq!(DEFINITIONS[row].home, None);
    let binding = BINDINGS
        .iter()
        .find(|binding| binding.row as usize == row)
        .unwrap();
    assert_eq!(binding.native_sources, 0);
    for source in RuleSetId::ALL {
        let view = cvars
            .bind(
                "r_cpuBands",
                Context {
                    source,
                    ..Context::default()
                },
            )
            .unwrap();
        assert_eq!(view.canonical(), handle);
        assert_eq!(cvars.integer_in(handle, source), 0);
        assert_eq!(cvars.flags(view), 1 | 32);
    }
}
