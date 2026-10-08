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
    cvars.set_text(canonical, "1.25").unwrap();
    let integer = cvars.find("developer").unwrap();
    cvars.set_text(integer, "2.75tail").unwrap();
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
    cvars.set_text(developer, "1").unwrap();
    let changed = cvars.generation(developer);
    assert!(changed > initial);
    cvars.set_text(developer, "1").unwrap();
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
    cvars.set_text(second, "1").unwrap();
    assert!(cvars.view_generation(colours) > colours_generation);
    let mode = cvars.bind("teamplay", q2).unwrap();
    let generation = cvars.generation(mode.canonical());
    cvars.server_active = true;
    cvars.write(mode, "2").unwrap();
    assert_eq!(cvars.generation(mode.canonical()), generation);
    cvars.apply_latches().unwrap();
    assert!(cvars.generation(mode.canonical()) > generation);
    let sensitivity = cvars.find("sensitivity").unwrap();
    let generation = cvars.generation(sensitivity);
    cvars.select_context(Context {
        source: Source::Quake,
        ..q2
    });
    assert_eq!(cvars.value(sensitivity), 3.0);
    assert!(cvars.generation(sensitivity) > generation);
    cvars.set_text(sensitivity, "4").unwrap();
    let explicit_generation = cvars.generation(sensitivity);
    cvars.select_context(Context::default());
    assert_eq!(cvars.value(sensitivity), 4.0);
    assert_eq!(cvars.generation(sensitivity), explicit_generation);
    cvars.reset(sensitivity);
    assert_eq!(cvars.value(sensitivity), 5.0);
    assert!(cvars.generation(sensitivity) > explicit_generation);
}

#[test]
fn published_hot_views_match_native_projections_after_coupled_and_seat_changes() {
    use qa_console::{
        catalog::Scope,
        cvars_generated::BINDINGS,
        numbers::number,
        views::{Role, Source},
    };
    let mut cvars = Cvars::new();
    cvars.cheats = true;
    for (source, name, value) in [
        (Source::Quake, "gamma", "0.8"),
        (Source::Quake, "teamplay", "2"),
        (Source::Quake, "_cl_color", "18"),
        (Source::Quake2, "cl_gun", "3"),
        (Source::Quake2, "sensitivity", "3.25"),
        (Source::Quake3, "cg_fov", "110"),
        (Source::QuakeWorld, "skin", "grunt"),
        (Source::Quake3, "ui_seat2_language", "1"),
    ] {
        let context = Context {
            source,
            ..Context::default()
        };
        cvars
            .write(cvars.bind(name, context).unwrap(), value)
            .unwrap();
        for binding in BINDINGS {
            let side = if binding.scope == Scope::Any {
                Scope::Client
            } else {
                binding.scope
            };
            for source in Source::ALL {
                for role in [Role::Engine, Role::Game, Role::Cgame] {
                    let context = Context {
                        source,
                        side,
                        role,
                        ..Context::default()
                    };
                    let Some(view) = cvars.bind(binding.name, context) else {
                        continue;
                    };
                    let expected = cvars
                        .read(view)
                        .map(|text| number(text.as_str(), source).to_bits());
                    assert_eq!(
                        cvars.numeric(view).map(f32::to_bits),
                        expected,
                        "{name} -> {} {source:?} {role:?}",
                        binding.name
                    );
                }
            }
        }
    }
}
