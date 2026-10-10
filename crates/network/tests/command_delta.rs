use qa_core::primitives::Vec3;
use qa_network::{
    commands::{Q2Cmd, Q2RrCmd, Q3Cmd, QwCmd, delta},
    message::{Encoding, Reader, Writer},
};

#[test]
fn q2_repro_float_abi_uses_native_widths_and_drops_unrepresented_fields() -> Result<(), String> {
    let from = Q2RrCmd {
        angles: Vec3([-0., 0.001, 17.]),
        movement: [-0., 12.25],
        buttons: 191,
        msec: 1,
        server_frame: 41,
    };
    let to = Q2RrCmd {
        angles: Vec3([0., 0.002, 17.]),
        movement: [0., 12.75],
        buttons: 31,
        msec: 255,
        server_frame: 999,
    };
    let mut bytes = [0; 32];
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    delta::write_q2_repro(&mut writer, from, to).map_err(|e| e.to_string())?;
    assert_eq!(writer.bytes(), &[82, 0, 0, 12, 0, 31, 255, 0]);
    let mut reader = Reader::new(writer.bytes(), Encoding::Bytes);
    let decoded = delta::read_q2_repro(&mut reader, from).map_err(|e| e.to_string())?;
    assert_eq!(reader.byte_position(), writer.size());
    assert_eq!(
        decoded.angles.0.map(f32::to_bits),
        [(-0.0f32).to_bits(), 0, 17.0f32.to_bits()]
    );
    assert_eq!(
        decoded.movement.map(f32::to_bits),
        [(-0.0f32).to_bits(), 12.0f32.to_bits()]
    );
    assert_eq!(
        (decoded.buttons, decoded.msec, decoded.server_frame),
        (31, 255, 41)
    );
    for prefix in 0..writer.size() {
        assert!(
            delta::read_q2_repro(
                &mut Reader::new(&writer.bytes()[..prefix], Encoding::Bytes),
                from
            )
            .is_err()
        );
    }
    let mut reader = Reader::new(&[128, 239, 23, 214], Encoding::Bytes);
    let skipped = delta::read_q2_repro(&mut reader, from).map_err(|e| e.to_string())?;
    assert_eq!(skipped, Q2RrCmd { msec: 23, ..from });
    assert_eq!(reader.byte_position(), 4);
    assert!(
        delta::read_q2_repro(&mut Reader::new(&[32, 0, 0, 0, 0], Encoding::Bytes), from).is_err()
    );
    let engine = qa_core::primitives::UserCmd {
        buttons: qa_core::primitives::buttons::ATTACK | qa_core::primitives::buttons::HOLSTER,
        movement: [12.25, -31.75, 200.],
        duration_ms: 300,
        impulse: 129,
        light_level: 211,
        ..Default::default()
    };
    let native = qa_network::commands::to_q2_rr_usercmd(&engine, 3001);
    let zero = Q2RrCmd {
        angles: Default::default(),
        movement: [0.; 2],
        buttons: 0,
        msec: 0,
        server_frame: 0,
    };
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    delta::write_q2_repro(&mut writer, zero, native).map_err(|e| e.to_string())?;
    let native = delta::read_q2_repro(&mut Reader::new(writer.bytes(), Encoding::Bytes), zero)
        .map_err(|e| e.to_string())?;
    let received = qa_network::commands::from_q2_rr_usercmd(native, 999);
    assert_eq!(received.movement, [12., -31., 0.]);
    assert_eq!(
        received.buttons,
        engine.buttons | qa_core::primitives::buttons::JUMP
    );
    assert_eq!(
        (received.duration_ms, received.impulse, received.light_level),
        (255, 0, 0)
    );
    Ok(())
}

#[test]
fn qw_mask_compares_native_floats_before_angle_quantization() -> Result<(), String> {
    let from = QwCmd {
        view_angles: Vec3([-0.0, 0.001, 0.0]),
        movement: [0; 3],
        buttons: 0,
        impulse: 0,
        msec: 0,
    };
    let to = QwCmd {
        view_angles: Vec3([0.0, 0.002, 0.0]),
        msec: 17,
        ..from
    };
    let mut bytes = [0; 32];
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    delta::write_qw(&mut writer, from, to).map_err(|e| e.to_string())?;
    assert_eq!(writer.bytes(), &[128, 0, 0, 17]);
    let decoded = delta::read_qw(&mut Reader::new(writer.bytes(), Encoding::Bytes), from)
        .map_err(|e| e.to_string())?;
    assert_eq!(
        decoded.view_angles.0.map(f32::to_bits),
        [(-0.0f32).to_bits(), 0, 0]
    );
    assert_eq!(decoded.msec, 17);
    Ok(())
}
#[test]
fn q2_always_sends_native_duration_and_light_after_unchanged_fields() -> Result<(), String> {
    let from = Q2Cmd {
        angles: [i16::MIN, 7, i16::MAX],
        movement: [i16::MAX, -7, i16::MIN],
        buttons: 129,
        impulse: 37,
        msec: 0,
        light_level: 0,
    };
    let to = Q2Cmd {
        msec: 255,
        light_level: 127,
        ..from
    };
    let mut bytes = [0; 32];
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    delta::write_q2(&mut writer, from, to).map_err(|e| e.to_string())?;
    assert_eq!(writer.bytes(), &[0, 255, 127]);
    assert_eq!(
        delta::read_q2(&mut Reader::new(writer.bytes(), Encoding::Bytes), from)
            .map_err(|e| e.to_string())?,
        to
    );
    assert!(
        delta::read_q2(
            &mut Reader::new(&writer.bytes()[..2], Encoding::Bytes),
            from
        )
        .is_err()
    );
    Ok(())
}
#[test]
fn q3_key_mask_and_signed_byte_projection_match_native_reader() -> Result<(), String> {
    let from = Q3Cmd {
        server_time: 1000,
        angles: [0; 3],
        movement: [0; 3],
        buttons: 0,
        weapon: 0,
    };
    let to = Q3Cmd {
        server_time: 1256,
        angles: [1, 65535, 2],
        movement: [-128, 127, -1],
        buttons: 65535,
        weapon: 255,
    };
    let key = 0x10100 ^ to.server_time as u32;
    let mut bytes = [0; 128];
    let mut writer = Writer::new(&mut bytes, Encoding::Q3);
    delta::write_q3(&mut writer, from, to, key).map_err(|e| e.to_string())?;
    let result = delta::read_q3(&mut Reader::new(writer.bytes(), Encoding::Q3), from, key)
        .map_err(|e| e.to_string())?;
    // Original kbitmask[16] retains bit16; char/byte assignment narrows bit8.
    assert_eq!(result.angles, [65537, 131071, 65538]);
    assert_eq!(result.buttons, 131071);
    assert_eq!(
        (result.server_time, result.movement, result.weapon),
        (1256, [-128, 127, -1], 255)
    );
    Ok(())
}
