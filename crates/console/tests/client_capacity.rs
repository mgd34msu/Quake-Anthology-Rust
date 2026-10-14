use qa_console::{
    cvars::Cvars,
    views::{Context, Role},
};
use qa_core::primitives::RuleSetId;

#[test]
fn native_module_client_limits_have_numeric_defaults() {
    let mut cvars = Cvars::new().unwrap();
    let handle = cvars.find("sv_maxclients").unwrap();
    for (rules, expected) in [
        (RuleSetId::Quake2, 1),
        (RuleSetId::Quake2Rerelease, 8),
        (RuleSetId::Quake3, 8),
    ] {
        assert_eq!(cvars.integer_in(handle, rules), expected);
        for role in [Role::Engine, Role::Game] {
            let view = cvars
                .bind(
                    "maxclients",
                    Context {
                        source: rules,
                        role,
                        ..Default::default()
                    },
                )
                .unwrap();
            assert_eq!(cvars.integer_view(view).unwrap(), expected);
            assert_eq!(cvars.read(view).unwrap().as_str(), expected.to_string());
        }
    }
    let context = Context {
        source: RuleSetId::Quake2Rerelease,
        role: Role::Game,
        ..Default::default()
    };
    let view = cvars.bind("maxclients", context).unwrap();
    cvars.force_write(view, "12").unwrap();
    cvars
        .register("maxclients", Some("8"), 4 | 32, context)
        .unwrap();
    assert_eq!(cvars.integer_in(handle, RuleSetId::Quake2Rerelease), 12);
    assert_eq!(cvars.integer_view(view).unwrap(), 12);
}
