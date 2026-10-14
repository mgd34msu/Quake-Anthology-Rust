use qa_compat::{
    renderer::{Q3_REFDEF_BYTES, read_q3_view},
    services::CallError,
};
use qa_render::stage::DrawInputs;

#[test]
fn signed_clock_and_complete_text_rows_survive_native_refdef_decode() {
    for time in [i32::MIN, -12345, 0, 16777217, i32::MAX] {
        let mut bytes = [0; Q3_REFDEF_BYTES];
        bytes[8..12].copy_from_slice(&320i32.to_le_bytes());
        bytes[12..16].copy_from_slice(&200i32.to_le_bytes());
        bytes[72..76].copy_from_slice(&time.to_le_bytes());
        bytes[76..80].copy_from_slice(&5i32.to_le_bytes());
        bytes[80..112].fill(0xf1);
        bytes[113] = 0xff;
        let view = read_q3_view(&bytes).unwrap();
        assert_eq!(view.refdef.time_ms, i64::from(time));
        assert_eq!(
            view.refdef.time_seconds().to_bits(),
            ((time as f32) * 0.001f32).to_bits()
        );
        assert_eq!(
            DrawInputs::for_view(&view.refdef).time_seconds().to_bits(),
            view.refdef.time_seconds().to_bits()
        );
        assert_eq!(view.hidden_areas, [0xf1; 32]);
        assert_eq!(view.refdef.text[0][0], 0);
        assert_eq!(view.refdef.text[0][1], 0xff);
        assert!(view.refdef.hyperspace);
        assert!(!view.world);
        bytes[112..144].fill(1);
        assert_eq!(read_q3_view(&bytes).unwrap_err(), CallError::Text);
        assert_eq!(
            read_q3_view(&bytes[..Q3_REFDEF_BYTES - 1]).unwrap_err(),
            CallError::Memory
        );
    }
}
