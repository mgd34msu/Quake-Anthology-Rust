use qa_core::primitives::Vec3;
use qa_network::commands::packet::{self, Error, ZERO_Q2_RR};

#[test]
fn repro_moves_have_no_crc_and_keep_three_command_baselines() -> Result<(), String> {
    let mut commands = [ZERO_Q2_RR; 3];
    commands[0].angles = Vec3([0., 90., -90.]);
    commands[0].movement = [12.25, -31.75];
    commands[0].buttons = 255;
    commands[0].msec = 17;
    commands[0].server_frame = 99;
    commands[1] = commands[0];
    commands[1].msec = 7;
    commands[2] = commands[1];
    commands[2].angles.0[0] = 45.;
    commands[2].movement[0] = -12.25;
    commands[2].msec = 255;
    let mut bytes = [0; 1400];
    let length =
        packet::write_q2_repro_move(&mut bytes, -123, &commands).map_err(|e| e.to_string())?;
    assert_eq!(&bytes[..5], &[2, 133, 255, 255, 255]);
    let (frame, decoded) =
        packet::read_q2_repro_move(&bytes[..length]).map_err(|e| e.to_string())?;
    assert_eq!(frame, -123);
    for (index, mut expected) in commands.into_iter().enumerate() {
        expected.movement = [if index == 2 { -12. } else { 12. }, -31.];
        expected.server_frame = 0;
        assert_eq!(decoded[index], expected);
    }
    for prefix in 0..length {
        assert!(packet::read_q2_repro_move(&bytes[..prefix]).is_err());
        assert!(packet::write_q2_repro_move(&mut bytes[..prefix], -123, &commands).is_err());
    }
    let length =
        packet::write_q2_repro_move(&mut bytes, -1, &[ZERO_Q2_RR; 3]).map_err(|e| e.to_string())?;
    assert_eq!(
        &bytes[..length],
        &[2, 255, 255, 255, 255, 0, 0, 0, 0, 0, 0, 0, 0, 0]
    );
    bytes[length] = 0;
    assert_eq!(
        packet::read_q2_repro_move(&bytes[..length + 1]),
        Err(Error::Trailing)
    );
    bytes[0] = 3;
    assert_eq!(
        packet::read_q2_repro_move(&bytes[..length]),
        Err(Error::Opcode)
    );
    Ok(())
}

#[test]
fn repro_moves_consume_legacy_bytes_and_reject_native_upmove() -> Result<(), String> {
    let bytes = [2, 7, 0, 0, 0, 128, 239, 23, 214, 0, 1, 0, 0, 2, 0];
    let (frame, commands) = packet::read_q2_repro_move(&bytes).map_err(|e| e.to_string())?;
    assert_eq!(frame, 7);
    assert_eq!(commands.map(|command| command.msec), [23, 1, 2]);
    for command in commands {
        assert_eq!(command.buttons, 0);
        assert_eq!(command.server_frame, 0);
    }
    let mut invalid = bytes;
    invalid[5] = 32;
    assert!(packet::read_q2_repro_move(&invalid).is_err());
    Ok(())
}
