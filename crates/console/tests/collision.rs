use qa_console::{collision::CollisionHandles, cvars::Cvars, views::Context};
use qa_core::primitives::{PatchClip, RuleSetId};

#[test]
fn curve_controls_share_values_across_roles_without_runtime_name_lookups() {
    let mut vars = Cvars::new().unwrap();
    let handles = CollisionHandles::bind(&vars).unwrap();
    assert_eq!(handles.resolve(&vars), PatchClip::All);
    for source in RuleSetId::ALL {
        let context = Context {
            source,
            ..Context::default()
        };
        let points = vars.bind("cm_playerCurveClip", context).unwrap();
        let disabled = vars.bind("cm_noCurves", context).unwrap();
        vars.force_write(points, "0").unwrap();
        vars.force_write(disabled, "0").unwrap();
        vars.reset_lookup_count();
        assert_eq!(handles.resolve(&vars), PatchClip::Volumes);
        assert_eq!(vars.lookup_count(), 0);
        vars.force_write(disabled, "1").unwrap();
        assert_eq!(handles.resolve(&vars), PatchClip::Disabled);
        vars.force_write(points, "1").unwrap();
        assert_eq!(handles.resolve(&vars), PatchClip::Disabled);
        vars.force_write(disabled, "0").unwrap();
        assert_eq!(handles.resolve(&vars), PatchClip::All);
    }
}
