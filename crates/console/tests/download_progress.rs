use qa_console::{
    cvars::Cvars,
    views::{Context, Role, RuleSetId},
};
use qa_core::sys_events::SeatId;

#[test]
fn native_progress_aliases_keep_independent_seat_values_and_cached_vm_views() {
    let mut cvars = Cvars::new().unwrap();
    let contexts = SeatId::ALL.map(|seat| Context {
        source: RuleSetId::Quake3,
        seat,
        ..Default::default()
    });
    let views = contexts.map(|context| cvars.bind("cl_downloadCount", context).unwrap());
    let guest = contexts.map(|context| {
        cvars
            .bind(
                "cl_downloadCount",
                Context {
                    role: Role::Cgame,
                    ..context
                },
            )
            .unwrap()
    });
    let native = guest.map(|view| cvars.native_view(view).unwrap());
    for (index, view) in views.into_iter().enumerate() {
        cvars
            .force_write(view, &(41 + index * 1000).to_string())
            .unwrap();
    }
    let modifications = native.map(|handle| cvars.native_snapshot(handle).unwrap().1);
    #[cfg(any(debug_assertions, feature = "lookup-tracking"))]
    cvars.reset_lookup_count();
    for _ in 0..1000 {
        for (index, view) in guest.into_iter().enumerate() {
            assert_eq!(cvars.numeric(view).unwrap(), (41 + index * 1000) as f32);
            assert_eq!(
                cvars.integer_view(view).unwrap(),
                (41 + index * 1000) as i32
            );
            assert_eq!(cvars.name(view), "cl_downloadCount");
        }
    }
    #[cfg(any(debug_assertions, feature = "lookup-tracking"))]
    assert_eq!(cvars.lookup_count(), 0);
    let explicit = cvars.bind("cl_downloadCount2", contexts[0]).unwrap();
    assert_eq!(explicit.canonical(), views[1].canonical());
    cvars.force_write(explicit, "777").unwrap();
    for index in 0..4 {
        assert_eq!(
            cvars.native_snapshot(native[index]).unwrap().1,
            modifications[index] + i32::from(index == 1)
        );
        assert_eq!(
            cvars.integer_view(guest[index]).unwrap(),
            if index == 1 {
                777
            } else {
                (41 + index * 1000) as i32
            }
        );
    }
}
