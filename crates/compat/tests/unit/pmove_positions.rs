use super::*;
use qa_core::primitives::Vec3;

#[test]
fn attached_position_write_preserves_native_widths_and_all_other_player_bytes() {
    for (layout, client, expected) in [
        (&CLASSIC, &crate::q2_abi::CLASSIC, [0x8001u32, 0x7fff, 8]),
        (
            &RERELEASE,
            &crate::q2_abi::RERELEASE,
            [
                4096.125f32.to_bits(),
                (-4096.125f32).to_bits(),
                1.0625f32.to_bits(),
            ],
        ),
    ] {
        let mover = Pmove::load(layout, client);
        let mut memory = ModuleMemory::load(0, 1024, &[0x5a; 1024]).unwrap();
        mover
            .write_position(&mut memory, 16, Vec3([4096.125, -4096.125, 1.0625]))
            .unwrap();
        let width = if layout.wide { 4 } else { 2 };
        for (axis, value) in expected.into_iter().enumerate() {
            let at = 16 + 4 + axis * width;
            assert_eq!(
                memory.read(at as u64, width).unwrap(),
                &value.to_le_bytes()[..width]
            );
        }
        for index in 0..1024 {
            if !(20..20 + width * 3).contains(&index) {
                assert_eq!(memory.read(index as u64, 1).unwrap(), &[0x5a]);
            }
        }
    }
}
