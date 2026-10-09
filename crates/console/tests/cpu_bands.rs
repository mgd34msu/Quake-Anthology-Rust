use qa_console::{
    cvars::Cvars,
    views::{Context, RuleSetId},
};

#[test]
fn renderer_latch_holds_every_native_view_until_load_boundary() {
    let mut cvars = Cvars::new();
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
        let handle = view.canonical();
        assert_eq!(cvars.integer_in(handle, source), 0);
        cvars.write(view, "4").unwrap();
        assert_eq!(cvars.integer_in(handle, source), 4);
        cvars.set_latch_active(handle, true);
        cvars.write(view, "8").unwrap();
        for dialect in RuleSetId::ALL {
            assert_eq!(cvars.integer_in(handle, dialect), 4);
        }
        cvars.set_latch_active(handle, false);
        cvars.apply_latches().unwrap();
        for dialect in RuleSetId::ALL {
            assert_eq!(cvars.integer_in(handle, dialect), 8);
        }
        cvars.reset(handle);
    }
}
