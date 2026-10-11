use qa_compat::{memory::ModuleMemory, q2_abi::client_layout};
use qa_core::primitives::{RuleSetId, UserCmd, Vec3, buttons};

#[test]
fn game_command_layouts_preserve_native_offsets_widths_and_float_bits() {
    let command = UserCmd {
        duration_ms: 300,
        view_angles: Vec3([90.0, -90.0, 180.0]),
        movement: [123.75, -456.5, 200.0],
        buttons: buttons::ATTACK | buttons::USE | buttons::ANY,
        impulse: 17,
        light_level: 42,
        ..Default::default()
    };
    for rules in [RuleSetId::Quake2, RuleSetId::Quake2Rerelease] {
        let layout = client_layout(rules).unwrap();
        let mut memory = ModuleMemory::load(0x1000, 32, &[0xcc; 32]).unwrap();
        layout
            .write_command(&mut memory, 0x1004, &command, 0xfedcba98)
            .unwrap();
        let expected = if rules == RuleSetId::Quake2 {
            vec![
                255, 131, 0, 64, 0, 192, 0, 128, 123, 0, 56, 254, 200, 0, 17, 42,
            ]
        } else {
            [
                vec![255, 139, 0, 0],
                [90.0f32, -90.0, 180.0, 123.75, -456.5]
                    .into_iter()
                    .flat_map(f32::to_le_bytes)
                    .collect(),
                0xfedcba98u32.to_le_bytes().to_vec(),
            ]
            .concat()
        };
        assert_eq!(memory.read(0x1004, layout.command_bytes).unwrap(), expected);
        let decoded = layout.read_command(&memory, 0x1004, -1234).unwrap();
        let expected_command = if rules == RuleSetId::Quake2 {
            qa_network::commands::from_q2_usercmd(
                qa_network::commands::to_q2_usercmd(&command),
                -1234,
            )
        } else {
            qa_network::commands::from_q2_rr_usercmd(
                qa_network::commands::to_q2_rr_usercmd(&command, 0xfedcba98),
                -1234,
            )
        };
        assert_eq!(decoded.server_time_ms, expected_command.server_time_ms);
        assert_eq!(decoded.duration_ms, expected_command.duration_ms);
        assert_eq!(
            decoded.view_angles.0.map(f32::to_bits),
            expected_command.view_angles.0.map(f32::to_bits)
        );
        assert_eq!(
            decoded.movement.map(f32::to_bits),
            expected_command.movement.map(f32::to_bits)
        );
        assert_eq!(decoded.buttons, expected_command.buttons);
        assert_eq!(decoded.impulse, expected_command.impulse);
        assert_eq!(decoded.light_level, expected_command.light_level);
        assert!(layout.read_command(&memory, 0x1018, 0).is_err());

        assert_eq!(memory.read(0x1000, 4).unwrap(), [0xcc; 4]);
        assert!(
            memory
                .read(
                    0x1004 + layout.command_bytes as u64,
                    28 - layout.command_bytes
                )
                .unwrap()
                .iter()
                .all(|&byte| byte == 0xcc)
        );
        layout
            .write_command(&mut memory, 0x1004, &UserCmd::default(), 0)
            .unwrap();
        assert!(
            memory
                .read(0x1004, layout.command_bytes)
                .unwrap()
                .iter()
                .all(|&byte| byte == 0)
        );
        assert!(
            layout
                .write_command(&mut memory, 0x1018, &command, 0)
                .is_err()
        );
    }
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld, RuleSetId::Quake3] {
        assert!(client_layout(rules).is_none());
    }
    let mut command = UserCmd::default();
    command.view_angles = Vec3([-0.0, f32::from_bits(0x7fc12345), f32::INFINITY]);
    command.movement = [-0.0, f32::from_bits(1), 0.0];
    let layout = client_layout(RuleSetId::Quake2Rerelease).unwrap();
    let mut memory = ModuleMemory::load(0, 28, &[]).unwrap();
    layout
        .write_command(&mut memory, 0, &command, u32::MAX)
        .unwrap();
    for (index, bits) in [0x80000000u32, 0x7fc12345, 0x7f800000, 0x80000000, 1]
        .into_iter()
        .enumerate()
    {
        assert_eq!(
            memory.read(4 + index as u64 * 4, 4).unwrap(),
            bits.to_le_bytes()
        );
    }
    assert_eq!(memory.read(24, 4).unwrap(), u32::MAX.to_le_bytes());
}
