use qa_core::primitives::Vec3;
use qa_network::{
    commands::{Q2Cmd, Q3Cmd, QwCmd, delta},
    message::{Encoding, Reader, Writer},
};

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
