use qa_core::{primitives::ModuleId, sys_events::EventTime};
use qa_session::timing::{Tick, TickRate, TickTarget, Timeline, TimingError};

fn mixed() -> Timeline {
    Timeline::load(
        TickRate::fixed(50).unwrap(),
        [(ModuleId(3), 50), (ModuleId(1), 100), (ModuleId(2), 25)]
            .map(|(module, ms)| (module, TickRate::fixed(ms).unwrap())),
    )
    .unwrap()
}

fn collect(times: &[u64]) -> Vec<Tick> {
    let mut timeline = mixed();
    timeline.seed(EventTime(10_000_000_000));
    let mut ticks = Vec::new();
    for &ms in times {
        timeline.advance(EventTime(10_000_000_000 + ms * 1_000_000), |tick| {
            ticks.push(tick)
        });
    }
    ticks
}

#[test]
fn native_rates_residuals_and_order_survive_client_frame_partitioning() {
    // Q2 G_RunFrame advances framenum*0.1; rerelease uses 0.025;
    // Q3 SV_Frame consumes 1000/sv_fps milliseconds from timeResidual.
    let ticks = collect(&[1, 24, 26, 49, 51, 99, 123, 250, 1000]);
    assert_eq!(ticks, collect(&[1000]));
    for (target, ms, count) in [
        (TickTarget::World, 50, 20),
        (TickTarget::Provider(ModuleId(1)), 100, 10),
        (TickTarget::Provider(ModuleId(2)), 25, 40),
        (TickTarget::Provider(ModuleId(3)), 50, 20),
    ] {
        let native: Vec<_> = ticks.iter().filter(|tick| tick.target == target).collect();
        assert_eq!(native.len(), count);
        for (index, tick) in native.into_iter().enumerate() {
            assert_eq!(tick.end.since(tick.start), ms * 1_000_000);
            assert_eq!(tick.index, index as u64 + 1);
        }
    }
    let simultaneous: Vec<_> = ticks
        .iter()
        .filter(|tick| tick.end.0 == 10_100_000_000)
        .map(|tick| tick.target)
        .collect();
    assert_eq!(
        simultaneous,
        [
            TickTarget::World,
            TickTarget::Provider(ModuleId(1)),
            TickTarget::Provider(ModuleId(2)),
            TickTarget::Provider(ModuleId(3))
        ]
    );
}

#[test]
fn frame_driven_q1_coexists_with_fixed_providers_without_startup_debt() {
    let mut timeline = Timeline::load(
        TickRate::FrameDriven,
        [(ModuleId(9), TickRate::fixed(100).unwrap())],
    )
    .unwrap();
    let origin = EventTime(50_000_000_000);
    assert_eq!(timeline.advance(origin, |_| unreachable!()), 0);
    let mut ticks = Vec::new();
    for ms in [16, 33, 70, 120] {
        timeline.advance(EventTime(origin.0 + ms * 1_000_000), |tick| {
            ticks.push(tick)
        });
    }
    assert_eq!(
        ticks
            .iter()
            .filter(|tick| tick.target == TickTarget::World)
            .count(),
        4
    );
    let provider = ticks
        .iter()
        .find(|tick| matches!(tick.target, TickTarget::Provider(_)))
        .unwrap();
    assert_eq!(provider.end.since(provider.start), 100_000_000);
    assert_eq!(
        timeline.advance(EventTime(origin.0 + 120_000_000), |_| unreachable!()),
        0
    );
    timeline.seed(EventTime(80_000_000_000));
    assert_eq!(
        timeline.advance(EventTime(80_000_000_000), |_| unreachable!()),
        0
    );
}

#[test]
fn invalid_rate_and_duplicate_provider_are_load_boundary_errors() {
    assert_eq!(TickRate::fixed(0), None);
    assert_eq!(
        Timeline::load(
            TickRate::FrameDriven,
            [(ModuleId(4), TickRate::FrameDriven); 2]
        )
        .err(),
        Some(TimingError::DuplicateProvider)
    );
}
