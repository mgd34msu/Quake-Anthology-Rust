use qa_console::{
    cvars::Cvars,
    views::{Context, Source},
};
#[cfg(any(debug_assertions, feature = "lookup-tracking"))]
use std::hint::black_box;

#[cfg(any(debug_assertions, feature = "lookup-tracking"))]
#[test]
fn canonical_and_converted_consumers_read_cached_fields_without_name_lookup() {
    let mut cvars = Cvars::new();
    let gamma = cvars.bind("gamma", cvars.context()).unwrap();
    let canonical = gamma.canonical();
    cvars.set_text(canonical, "1.25");
    let integer = cvars.find("developer").unwrap();
    cvars.set_text(integer, "2.75tail");
    cvars.reset_lookup_count();
    assert!(cvars.find("unknown-positive-control").is_none());
    assert_eq!(cvars.lookup_count(), 1);
    cvars.reset_lookup_count();
    for _ in 0..10000 {
        assert_eq!(black_box(cvars.value(canonical)), 1.25);
        assert_eq!(black_box(cvars.numeric(gamma).unwrap()), 0.8);
        assert_eq!(black_box(cvars.integer(integer)), 2);
        black_box(cvars.generation(canonical));
        black_box(cvars.view_generation(gamma));
    }
    assert_eq!(cvars.lookup_count(), 0);
}

#[test]
fn generations_follow_values_details_deferred_updates_and_source_defaults() {
    let mut cvars = Cvars::new();
    let developer = cvars.find("developer").unwrap();
    let initial = cvars.generation(developer);
    cvars.set_text(developer, "1");
    let changed = cvars.generation(developer);
    assert!(changed > initial);
    cvars.set_text(developer, "1");
    assert_eq!(cvars.generation(developer), changed);
    let q2 = Context {
        source: Source::Quake2,
        ..Context::default()
    };
    let gun = cvars.bind("cl_gun", q2).unwrap();
    cvars.write(gun, "2").unwrap();
    let first = cvars.generation(gun.canonical());
    cvars.write(gun, "3").unwrap();
    assert_eq!(cvars.value(gun.canonical()), 1.0);
    assert!(cvars.generation(gun.canonical()) > first);
    let colours = cvars
        .bind(
            "_cl_color",
            Context {
                source: Source::Quake,
                ..q2
            },
        )
        .unwrap();
    cvars.write(colours, "18").unwrap();
    let colours_generation = cvars.view_generation(colours);
    let second = cvars.find("color2").unwrap();
    cvars.set_text(second, "1");
    assert!(cvars.view_generation(colours) > colours_generation);
    let mode = cvars.bind("teamplay", q2).unwrap();
    let generation = cvars.generation(mode.canonical());
    cvars.server_active = true;
    cvars.write(mode, "2").unwrap();
    assert_eq!(cvars.generation(mode.canonical()), generation);
    cvars.apply_latches();
    assert!(cvars.generation(mode.canonical()) > generation);
    let sensitivity = cvars.find("sensitivity").unwrap();
    let generation = cvars.generation(sensitivity);
    cvars.select_context(Context {
        source: Source::Quake,
        ..q2
    });
    assert_eq!(cvars.value(sensitivity), 3.0);
    assert!(cvars.generation(sensitivity) > generation);
    cvars.set_text(sensitivity, "4");
    let explicit_generation = cvars.generation(sensitivity);
    cvars.select_context(Context::default());
    assert_eq!(cvars.value(sensitivity), 4.0);
    assert_eq!(cvars.generation(sensitivity), explicit_generation);
    cvars.reset(sensitivity);
    assert_eq!(cvars.value(sensitivity), 5.0);
    assert!(cvars.generation(sensitivity) > explicit_generation);
}
