use qa_audio::prepare;
use qa_core::primitives::{Pcm, PcmChannels};
use std::num::NonZeroU32;
fn pcm(rate: u32) -> Pcm {
    Pcm {
        rate: NonZeroU32::new(rate).unwrap(),
        channels: PcmChannels::Stereo,
        samples: vec![1000, -1000, 2000, -2000, 3000, -3000, 4000, -4000],
        loop_start: Some(1),
    }
}
#[test]
fn preparation_moves_native_rate_pcm_and_resamples_interleaved_frames_once() {
    let original = pcm(11025);
    let pointer = original.samples.as_ptr();
    let moved = prepare(original, NonZeroU32::new(11025).unwrap(), false).unwrap();
    assert_eq!(pointer, moved.samples.as_ptr());
    let up = prepare(moved, NonZeroU32::new(22050).unwrap(), false).unwrap();
    assert_eq!(up.loop_start, Some(2));
    assert_eq!(
        up.samples,
        vec![
            1000, -1000, 1000, -1000, 2000, -2000, 2000, -2000, 3000, -3000, 3000, -3000, 4000,
            -4000, 4000, -4000
        ]
    );
    let quantised = prepare(pcm(11025), NonZeroU32::new(11025).unwrap(), true).unwrap();
    assert_eq!(&quantised.samples[..2], &[768, -1024]);
}
