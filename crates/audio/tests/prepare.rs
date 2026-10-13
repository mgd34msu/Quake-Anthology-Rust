use qa_audio::prepare;
use qa_core::primitives::{Pcm, PcmChannels};
use std::num::NonZeroU32;
fn pcm(rate: u32) -> Pcm {
    Pcm {
        rate: NonZeroU32::new(rate).unwrap(),
        channels: PcmChannels::Stereo,
        samples: vec![1000, -1000, 2000, -2000, 3000, -3000, 4000, -4000],
        loop_start: Some(1),
        loop_end: None,
        loop_invalid: false,
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

#[test]
fn preparation_resamples_full_pcm_and_native_loop_interval_independently() {
    let mut source = pcm(11025);
    source.loop_end = Some(3);
    let prepared = prepare(source, NonZeroU32::new(22050).unwrap(), false).unwrap();
    assert_eq!(prepared.frames(), 8);
    assert_eq!(prepared.cue_frames(), 6);
    assert_eq!(
        (
            prepared.loop_start,
            prepared.loop_end,
            prepared.loop_invalid
        ),
        (Some(2), Some(6), false)
    );
    assert_eq!(&prepared.samples[12..], &[4000, -4000, 4000, -4000]);
}

#[test]
fn a_loop_collapsed_by_resampling_does_not_destroy_usable_full_waveform() {
    let mut source = pcm(96000);
    source.loop_end = Some(2);
    let prepared = prepare(source, NonZeroU32::new(44100).unwrap(), false).unwrap();
    assert_eq!(prepared.frames(), 1);
    assert_eq!(prepared.samples, [1000, -1000]);
    assert_eq!(
        (
            prepared.loop_start,
            prepared.loop_end,
            prepared.loop_invalid
        ),
        (None, None, true)
    );
}
