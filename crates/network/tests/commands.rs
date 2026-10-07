use qa_core::primitives::{UserCmd, Vec3, WeaponId, buttons as b};
use qa_network::commands::*;

#[test]
fn each_boundary_roundtrips_its_original_shape() {
    let q1 = Q1Move {
        view_angles: Vec3([-90.0, 180.0, 0.0]),
        movement: [200.0, -200.0, 0.0],
        buttons: 3,
        impulse: 10,
    };
    assert_eq!(to_q1_move(&from_q1_move(q1, 13, 100)), q1);
    let qw = QwCmd {
        msec: 13,
        view_angles: q1.view_angles,
        movement: [200, -200, 0],
        buttons: 3,
        impulse: 10,
    };
    assert_eq!(to_qw_usercmd(&from_qw_usercmd(qw, 100)), qw);
    let q2 = Q2Cmd {
        msec: 13,
        angles: [-16384, -32768, 1],
        movement: [200, -200, -200],
        buttons: 131,
        impulse: 10,
        light_level: 255,
    };
    let decoded = from_q2_usercmd(q2, 100);
    assert_eq!(decoded.buttons, b::ATTACK | b::USE | b::ANY | b::CROUCH);
    assert_eq!(to_q2_usercmd(&decoded), q2);
    let q3 = Q3Cmd {
        server_time: 100,
        angles: [49152, 32768, 1],
        movement: [127, -128, 127],
        buttons: 4095,
        weapon: 9,
    };
    let decoded = from_q3_usercmd(q3, 87, WeaponId(700));
    assert_eq!(decoded.duration_ms, 13);
    assert_eq!(decoded.weapon, Some(WeaponId(700)));
    assert_eq!(to_q3_usercmd(&decoded, 9), q3);
}

#[test]
fn engine_jump_crouch_and_any_bits_map_to_each_protocol() {
    let command = UserCmd {
        duration_ms: 300,
        view_angles: Vec3([-90.0, 180.0, 360.0]),
        buttons: b::JUMP | b::ANY,
        ..UserCmd::default()
    };
    assert_eq!(to_q1_move(&command).buttons, 2);
    assert_eq!(to_qw_usercmd(&command).msec, 255);
    assert_eq!(to_q2_usercmd(&command).movement[2], 200);
    assert_eq!(to_q2_usercmd(&command).buttons, 128);
    assert_eq!(to_q3_usercmd(&command, 0).movement[2], 127);
    assert_eq!(to_q3_usercmd(&command, 0).buttons, 2048);
    assert_eq!(to_q3_usercmd(&command, 0).angles, [49152, 32768, 0]);
}

#[test]
fn every_original_short_angle_survives_the_boundary_projection() {
    for angle in 0..=u16::MAX {
        let q2 = Q2Cmd {
            msec: 13,
            angles: [angle as i16; 3],
            movement: [0; 3],
            buttons: 0,
            impulse: 0,
            light_level: 0,
        };
        assert_eq!(to_q2_usercmd(&from_q2_usercmd(q2, 0)).angles, q2.angles);
        let q3 = Q3Cmd {
            server_time: 13,
            angles: [i32::from(angle); 3],
            movement: [0; 3],
            buttons: 0,
            weapon: 0,
        };
        assert_eq!(
            to_q3_usercmd(&from_q3_usercmd(q3, 0, WeaponId(0)), 0).angles,
            q3.angles
        );
    }
}
