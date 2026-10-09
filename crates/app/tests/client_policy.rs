use qa_app::{Runtime, client_policy::ClientPolicy, map::SpawnAnchor};
use qa_console::{cvars::Cvars, views::Context};
use qa_core::{
    primitives::{Bounds, RuleSetId, Vec3},
    sys_events::SeatId,
};
use qa_session::timing::TickRate;
use qa_world::area::{LinkFlags, LinkOrder};

fn fixed(milliseconds: u32) -> Result<TickRate, String> {
    TickRate::fixed(milliseconds).ok_or("zero expected native tick period".to_owned())
}

#[test]
fn startup_seat_policy_preserves_independent_roles_and_rejects_invalid_fields() {
    let (seat, policy) = ClientPolicy::parse_seat("1:q2:q3:q1").unwrap();
    assert_eq!(seat.index(), 1);
    assert_eq!(policy.client, RuleSetId::Quake2);
    assert_eq!(policy.movement, RuleSetId::Quake3);
    assert_eq!(policy.trace, RuleSetId::Quake);
    for value in [
        "4:q1:q1:q1",
        "-1:q1:q1:q1",
        "1:q1:q1",
        "1:q1:q1:q1:extra",
        "1:q1:q1:bad",
    ] {
        assert!(ClientPolicy::parse_seat(value).is_err(), "{value}");
    }
}

#[test]
fn client_selection_requires_identity_and_defaults_each_role_independently() -> Result<(), String> {
    for (movement, trace) in [
        (None, None),
        (Some(RuleSetId::Quake3), None),
        (None, Some(RuleSetId::Quake2)),
        (Some(RuleSetId::Quake3), Some(RuleSetId::Quake2)),
    ] {
        assert!(ClientPolicy::select(None, None, movement, trace).is_err());
    }
    for client in RuleSetId::ALL {
        for (explicit, stock) in [(Some(client), None), (None, Some(client))] {
            let policy = ClientPolicy::select(explicit, stock, None, None)?;
            assert_eq!(
                (policy.client, policy.movement, policy.trace),
                (client, client, client)
            );
        }
        for stock in RuleSetId::ALL {
            let policy = ClientPolicy::select(Some(client), Some(stock), None, None)?;
            assert_eq!(
                (policy.client, policy.movement, policy.trace),
                (client, client, client)
            );
        }
    }
    Ok(())
}

#[test]
fn foreign_movement_and_trace_overrides_do_not_select_native_clock_or_link_order()
-> Result<(), String> {
    let mut cvars = Cvars::with_context(Context {
        source: RuleSetId::QuakeWorld,
        ..Context::default()
    })
    .unwrap();
    for client in RuleSetId::ALL {
        let (rate, order) = match client {
            RuleSetId::Quake | RuleSetId::QuakeWorld => (TickRate::FrameDriven, LinkOrder::Tail),
            RuleSetId::Quake2 => (fixed(100)?, LinkOrder::Tail),
            RuleSetId::Quake2Rerelease => (fixed(25)?, LinkOrder::Tail),
            // Q3 sv_init.c registers sv_fps=20; SV_Frame uses integer 1000/fps.
            RuleSetId::Quake3 => (fixed(50)?, LinkOrder::Head),
        };
        for foreign in RuleSetId::ALL {
            let movement = ClientPolicy::select(None, Some(client), Some(foreign), None)?;
            assert_eq!(
                (movement.client, movement.movement, movement.trace),
                (client, foreign, client)
            );
            assert_eq!(movement.tick_rate(&mut cvars)?, rate);
            assert_eq!(movement.link_order(), order);
            let trace = ClientPolicy::select(None, Some(client), None, Some(foreign))?;
            assert_eq!(
                (trace.client, trace.movement, trace.trace),
                (client, client, foreign)
            );
            assert_eq!(trace.tick_rate(&mut cvars)?, rate);
            assert_eq!(trace.link_order(), order);
        }
    }
    Ok(())
}

#[test]
fn q3_tick_period_and_low_rate_repair_use_the_client_view_in_every_console_dialect()
-> Result<(), String> {
    // Original Q3 sv_main.c:772-775 writes 10 below one, then integer-divides
    // 1000 by sv_fps. Movement and the selected console dialect are independent.
    for dialect in RuleSetId::ALL {
        for (input, expected_fps, milliseconds) in [
            (None, 20, 50),
            (Some("40"), 40, 25),
            (Some("0"), 10, 100),
            (Some("-8"), 10, 100),
            (Some("33"), 33, 30),
        ] {
            let context = Context {
                source: dialect,
                ..Context::default()
            };
            let mut cvars = Cvars::with_context(context).unwrap();
            let fps = cvars.find("sv_fps").ok_or("missing sv_fps")?;
            if let Some(input) = input {
                cvars
                    .set_text(fps, input)
                    .map_err(|error| format!("{error:?}"))?;
            }
            let generation = cvars.generation(fps);
            let policy = ClientPolicy::select(
                Some(RuleSetId::Quake3),
                None,
                Some(RuleSetId::Quake),
                Some(RuleSetId::Quake2),
            )?;
            assert_eq!(policy.tick_rate(&mut cvars)?, fixed(milliseconds)?);
            assert_eq!(cvars.integer_in(fps, RuleSetId::Quake3), expected_fps);
            assert_eq!(cvars.context(), context);
            if matches!(input, Some("0" | "-8")) {
                assert!(cvars.generation(fps) > generation);
                let alias = cvars
                    .bind(
                        "sys_ticrate",
                        Context {
                            source: RuleSetId::Quake,
                            ..context
                        },
                    )
                    .ok_or("missing sys_ticrate")?;
                assert!(
                    (cvars.numeric(alias).map_err(|error| format!("{error:?}"))? - 0.1).abs()
                        < 0.000001
                );
            } else {
                assert_eq!(cvars.generation(fps), generation);
            }
        }
    }
    Ok(())
}

#[test]
fn non_q3_clients_do_not_repair_q3_sv_fps() -> Result<(), String> {
    for (input, expected_fps) in [("0", 0), ("-8", -8)] {
        let mut cvars = Cvars::new().unwrap();
        let fps = cvars.find("sv_fps").ok_or("missing sv_fps")?;
        cvars
            .set_text(fps, input)
            .map_err(|error| format!("{error:?}"))?;
        let generation = cvars.generation(fps);
        for (client, expected) in [
            (RuleSetId::Quake, TickRate::FrameDriven),
            (RuleSetId::QuakeWorld, TickRate::FrameDriven),
            (RuleSetId::Quake2, fixed(100)?),
            (RuleSetId::Quake2Rerelease, fixed(25)?),
        ] {
            let policy = ClientPolicy::select(
                Some(client),
                None,
                Some(RuleSetId::Quake3),
                Some(RuleSetId::Quake3),
            )?;
            assert_eq!(policy.tick_rate(&mut cvars)?, expected);
            assert_eq!(cvars.integer_in(fps, RuleSetId::Quake3), expected_fps);
            assert_eq!(cvars.generation(fps), generation);
        }
    }
    Ok(())
}

#[test]
fn the_first_local_link_uses_client_order_and_snapshot_keeps_role_ids() -> Result<(), String> {
    let mut runtime = Runtime::load(std::iter::empty())?;
    let spawn = SpawnAnchor {
        position: Vec3([0.0, 0.0, 24.0]),
        angles: Vec3([5.0, 90.0, 0.0]),
        entity: 7,
        fixture_fallback: false,
    };
    let choices = [
        (
            RuleSetId::Quake2,
            RuleSetId::QuakeWorld,
            RuleSetId::Quake3,
            LinkOrder::Tail,
        ),
        (
            RuleSetId::Quake3,
            RuleSetId::QuakeWorld,
            RuleSetId::Quake2,
            LinkOrder::Head,
        ),
        (
            RuleSetId::Quake2,
            RuleSetId::QuakeWorld,
            RuleSetId::Quake,
            LinkOrder::Tail,
        ),
        (
            RuleSetId::Quake3,
            RuleSetId::QuakeWorld,
            RuleSetId::QuakeWorld,
            LinkOrder::Head,
        ),
    ];
    let mut entities = Vec::new();
    for (seat, (client, movement, trace, order)) in SeatId::ALL.into_iter().zip(choices) {
        let policy = ClientPolicy::select(Some(client), None, Some(movement), Some(trace))?;
        let id = runtime.connect_local(seat, spawn, policy)?;
        let connected = &runtime.server.clients[id.0 as usize];
        assert_eq!(connected.client_rules, client);
        assert_eq!(connected.player.movement_rules, movement);
        assert_eq!(connected.player.trace_rules, trace);
        assert_eq!(connected.link_order, order);
        let prediction = &runtime.prediction[seat.index()].player;
        assert_eq!(
            (prediction.movement_rules, prediction.trace_rules),
            (movement, trace)
        );
        assert_eq!(connected.player.body.mins, Vec3([-16.0, -16.0, -24.0]));
        assert_eq!(connected.player.body.maxs, Vec3([16.0, 16.0, 32.0]));
        assert_eq!(
            (
                prediction.body.position,
                prediction.body.velocity,
                prediction.body.mins,
                prediction.body.maxs
            ),
            (
                connected.player.body.position,
                connected.player.body.velocity,
                connected.player.body.mins,
                connected.player.body.maxs
            ),
        );
        assert_eq!(prediction.view_angles, spawn.angles);
        entities.push(connected.entity);
    }
    // Q3 sv_world.c:351-353 prepends; Q2 sv_world.c:341-343 appends.
    // All bounds cross the same area split, so this observes the first links
    // directly. A corrective relink would change the link count and ordering.
    assert_eq!(runtime.server.area.relinks, 4);
    let actual = runtime
        .server
        .area
        .query(
            &runtime.server.entities,
            Bounds {
                mins: Vec3([-128.0; 3]),
                maxs: Vec3([128.0; 3]),
            },
            LinkFlags::SOLID,
        )
        .map(|row| row.id)
        .collect::<Vec<_>>();
    assert_eq!(actual, [entities[3], entities[1], entities[0], entities[2]]);
    Ok(())
}
