use qa_core::primitives::{PcmChannels, PcmFormat, PcmWidth};
use std::num::NonZeroU32;

#[test]
fn source_pcm_widths_and_conversion_have_one_definition() {
    assert_eq!(PcmWidth::from_bytes(0), None);
    assert_eq!(PcmWidth::from_bytes(4), None);
    for (bytes, width) in [
        (1, PcmWidth::Byte),
        (2, PcmWidth::Short),
        (3, PcmWidth::ThreeBytes),
    ] {
        assert_eq!(PcmWidth::from_bytes(bytes), Some(width));
        let format = PcmFormat {
            rate: NonZeroU32::new(44100).unwrap(),
            channels: PcmChannels::Stereo,
            width,
        };
        assert_eq!(format.frame_bytes(), 2 * bytes as usize);
    }
    assert_eq!(PcmWidth::Byte.sample(&[0], false), -32768);
    assert_eq!(PcmWidth::Byte.sample(&[128], false), 0);
    assert_eq!(PcmWidth::Byte.sample(&[255], false), 32512);
    assert_eq!(PcmWidth::Byte.sample(&[128], true), -32768);
    assert_eq!(PcmWidth::Byte.sample(&[255], true), -256);
    assert_eq!(PcmWidth::Short.sample(&[1, 128], false), -32767);
    assert_eq!(PcmWidth::ThreeBytes.sample(&[255, 1, 128], false), -32767);
}
