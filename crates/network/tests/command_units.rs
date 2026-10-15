use qa_core::primitives::{RuleSetId, UserCmd, Vec3, WeaponId, buttons};
use qa_gameplay::rules::command_units;
use qa_network::commands::{self, packet::Protocol};

#[test]
fn speed_protocols_keep_raw_speeds_and_q3_bridges_the_producing_scale() {
    let command = UserCmd {
        units: command_units(RuleSetId::Quake),
        movement: [200., -200., 320.],
        ..Default::default()
    };
    assert_eq!(commands::to_q1_move(&command).movement, [200, -200, 320]);
    assert_eq!(commands::to_qw_usercmd(&command).movement, [200, -200, 320]);
    assert_eq!(commands::to_q2_usercmd(&command).movement, [200, -200, 320]);
    assert_eq!(
        commands::to_q2_rr_usercmd(&command, 0).movement,
        [200., -200.]
    );
    assert_eq!(
        commands::to_q3_usercmd(&command, 0).movement,
        [79, -79, 127]
    );

    let normalized = UserCmd {
        units: command_units(RuleSetId::Quake3),
        movement: [127., -127., 0.],
        ..command
    };
    assert_eq!(
        commands::to_q3_usercmd(&normalized, 0).movement,
        [127, -127, 0]
    );
    assert_eq!(commands::to_q1_move(&normalized).movement, [320, -320, 0]);
    assert_eq!(
        commands::to_qw_usercmd(&normalized).movement,
        [320, -320, 0]
    );
    assert_eq!(
        commands::to_q2_usercmd(&normalized).movement,
        [200, -200, 0]
    );
    assert_eq!(
        commands::to_q2_rr_usercmd(&normalized, 0).movement,
        [200., -200.]
    );

    let q2_speed = UserCmd {
        units: command_units(RuleSetId::Quake2),
        movement: [200., -200., 0.],
        ..command
    };
    assert_eq!(
        commands::to_q3_usercmd(&q2_speed, 0).movement,
        [127, -127, 0]
    );
}

#[test]
fn native_decoders_tag_their_actual_units_and_preserve_q3_negative_128() {
    let q1 = commands::Q1Move {
        view_angles: Vec3::default(),
        movement: [200, -200, 0],
        buttons: 0,
        impulse: 0,
    };
    assert_eq!(
        commands::from_q1_move(q1, 16, 16).units,
        command_units(RuleSetId::Quake)
    );
    let qw = commands::QwCmd {
        msec: 16,
        view_angles: q1.view_angles,
        movement: q1.movement,
        buttons: 0,
        impulse: 0,
    };
    assert_eq!(
        commands::from_qw_usercmd(qw, 16).units,
        command_units(RuleSetId::QuakeWorld)
    );
    let q2 = commands::Q2Cmd {
        msec: 16,
        angles: [0; 3],
        movement: q1.movement,
        buttons: 0,
        impulse: 0,
        light_level: 0,
    };
    assert_eq!(
        commands::from_q2_usercmd(q2, 16).units,
        command_units(RuleSetId::Quake2)
    );
    let rr = commands::Q2RrCmd {
        msec: 16,
        buttons: 0,
        angles: q1.view_angles,
        movement: [200., -200.],
        server_frame: 9,
    };
    assert_eq!(
        commands::from_q2_rr_usercmd(rr, 16).units,
        command_units(RuleSetId::Quake2Rerelease)
    );
    let q3 = commands::Q3Cmd {
        server_time: 16,
        angles: [0; 3],
        movement: [-128, 127, -128],
        buttons: 0,
        weapon: 0,
    };
    let common = commands::from_q3_usercmd(q3, 0, None);
    assert_eq!(common.units, command_units(RuleSetId::Quake3));
    assert_eq!(commands::to_q3_usercmd(&common, 0), q3);
    let admitted = commands::canonical_command(Protocol::Quake3_68, common).unwrap();
    assert_eq!(admitted.movement, [-128., 127., -128.]);
}

#[test]
fn history_admission_uses_native_movement_and_buttons_without_narrowing_other_fields() {
    let command = UserCmd {
        duration_ms: 999,
        duration_ns: 999_123_456,
        server_time_ms: i32::MIN,
        view_angles: Vec3([-0., 719.125, f32::from_bits(0x7fc1_2345)]),
        movement: [200.25, -17.75, 7.75],
        units: command_units(RuleSetId::Quake),
        buttons: buttons::ATTACK
            | buttons::ANY
            | buttons::HOLSTER
            | buttons::EXTRA12
            | buttons::TALK,
        impulse: 201,
        light_level: 247,
        weapon: Some(WeaponId(999_999)),
    };
    for (protocol, movement, native_buttons) in [
        (Protocol::NetQuake15, [200., -17., 7.], buttons::ATTACK),
        (Protocol::QuakeWorld28, [200., -17., 7.], buttons::ATTACK),
        (
            Protocol::Quake2_34,
            [200., -17., 7.],
            buttons::ATTACK | buttons::ANY,
        ),
        (
            Protocol::Quake2Repro1038,
            [200., -17., 0.],
            buttons::ATTACK | buttons::ANY | buttons::HOLSTER | buttons::JUMP,
        ),
        (
            Protocol::Quake3_68,
            [79., -7., 3.],
            buttons::ATTACK | buttons::ANY | buttons::EXTRA12 | buttons::TALK,
        ),
    ] {
        let admitted = commands::canonical_command(protocol, command).unwrap();
        assert_eq!(admitted.movement, movement, "{protocol:?}");
        assert_eq!(admitted.units, command_units(protocol.rule_set()));
        assert_eq!(admitted.buttons, native_buttons, "{protocol:?}");
        assert_eq!(admitted.duration_ms, command.duration_ms);
        assert_eq!(admitted.duration_ns, command.duration_ns);
        assert_eq!(admitted.server_time_ms, command.server_time_ms);
        assert_eq!(
            admitted.view_angles.0.map(f32::to_bits),
            command.view_angles.0.map(f32::to_bits)
        );
        assert_eq!(admitted.weapon, command.weapon);
        assert_eq!(admitted.impulse, command.impulse);
        assert_eq!(admitted.light_level, command.light_level);
        let again = commands::canonical_command(protocol, admitted).unwrap();
        assert_eq!(
            again.movement.map(f32::to_bits),
            admitted.movement.map(f32::to_bits)
        );
        assert_eq!(again.units, admitted.units);
        assert_eq!(again.buttons, admitted.buttons);
    }
}

#[test]
fn classic_q2_vertical_flags_match_native_jump_threshold_and_keep_other_buttons() {
    let retained = buttons::ATTACK | buttons::USE | buttons::ANY;
    for (up, vertical) in [(1, 0), (9, 0), (10, buttons::JUMP), (-1, buttons::CROUCH)] {
        let native = commands::Q2Cmd {
            msec: 16,
            angles: [0; 3],
            movement: [0, 0, up],
            buttons: 131,
            impulse: 9,
            light_level: 37,
        };
        let decoded = commands::from_q2_usercmd(native, 100);
        assert_eq!(decoded.buttons, retained | vertical, "upmove={up}");
        assert_eq!(commands::to_q2_usercmd(&decoded), native);
        let admitted = commands::canonical_command(
            Protocol::Quake2_34,
            UserCmd {
                units: command_units(RuleSetId::Quake2),
                movement: [0., 0., f32::from(up)],
                buttons: retained | buttons::JUMP,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(admitted.movement, [0., 0., f32::from(up)]);
        assert_eq!(admitted.buttons, retained | vertical, "upmove={up}");
    }
}

#[test]
fn admitted_mixed_command_and_decoded_packet_have_identical_movement_meaning() {
    use qa_core::loopback::Endpoint;
    use qa_network::{channel::Channel, commands::connection::Commands};
    let mut channel =
        Channel::load(Protocol::Quake3_68.channel(), Endpoint::Server, 8192, 8).unwrap();
    let client = Channel::load(Protocol::Quake3_68.channel(), Endpoint::Client, 8192, 8).unwrap();
    let mut outgoing = Commands::load(Protocol::Quake3_68, 8192);
    outgoing.configure_history(4096).unwrap();
    let mut incoming = Commands::load(Protocol::Quake3_68, 8192);
    for (index, units, expected) in [
        (1, command_units(RuleSetId::Quake), [79., -79., 0.]),
        (2, command_units(RuleSetId::Quake3), [127., -127., 0.]),
    ] {
        let admitted = commands::canonical_command(
            Protocol::Quake3_68,
            UserCmd {
                server_time_ms: index * 16,
                duration_ms: 16,
                movement: if index == 1 {
                    [200., -200., 0.]
                } else {
                    [127., -127., 0.]
                },
                units,
                ..Default::default()
            },
        )
        .unwrap();
        let number = outgoing.record(admitted).unwrap();
        let mut bytes = [0; 1400];
        let length = outgoing
            .encode(&admitted, |_| 0, &client, &mut bytes)
            .unwrap();
        let length = incoming.stage(&bytes[..length]).unwrap();
        let mut received = None;
        incoming
            .decode(length, 0, 0, &mut channel, None, |record| {
                if let qa_network::ingress::Incoming::Command { command, .. } = record {
                    received = Some(command);
                }
                std::ops::ControlFlow::Continue(())
            })
            .unwrap();
        let decoded = received.unwrap();
        assert_eq!(decoded.movement, expected);
        let retained = outgoing.history().unwrap().read(number).unwrap().unwrap();
        assert_eq!(decoded.movement, retained.movement);
        assert_eq!(decoded.units, retained.units);
        assert_eq!(decoded.buttons, retained.buttons);
    }
}
