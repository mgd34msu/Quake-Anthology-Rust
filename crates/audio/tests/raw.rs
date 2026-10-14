use qa_audio::{Bank, Mixer, RAW_FRAMES, RawError};
use qa_core::primitives::*;
use std::{num::NonZeroU32, sync::Arc};

fn rate(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).unwrap()
}
fn mixer(output: u32) -> Mixer {
    Mixer::load(Arc::new(Bank::load(rate(output), vec![]).unwrap()), 4, 1)
}
fn format(source: u32, channels: PcmChannels, width: PcmWidth) -> PcmFormat {
    PcmFormat {
        rate: rate(source),
        channels,
        width,
    }
}
fn shorts(values: &[i16]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

#[test]
fn raw_stereo_short_gain_and_effect_volume_are_independent() {
    let mut mix = mixer(22050);
    mix.listen(&[], 0.);
    assert_eq!(
        mix.queue_raw(
            format(22050, PcmChannels::Stereo, PcmWidth::Short),
            &shorts(&[1000, -1000, 32767, -32768]),
            0.5,
            true
        ),
        Ok(2)
    );
    let mut output = [0; 6];
    assert!(mix.paint(&mut output));
    assert_eq!(output, [500, -500, 16383, -16384, 0, 0]);
}

#[test]
fn raw_bytes_preserve_unsigned_mono_and_signed_native_stereo() {
    let mut mix = mixer(22050);
    mix.queue_raw(
        format(22050, PcmChannels::Stereo, PcmWidth::Byte),
        &[0, 255, 128, 127],
        0.5,
        true,
    )
    .unwrap();
    let mut output = [0; 4];
    mix.paint(&mut output);
    assert_eq!(output, [0, -128, -16384, 16256]);
    mix.queue_raw(
        format(22050, PcmChannels::Mono, PcmWidth::Byte),
        &[0, 128, 255],
        0.5,
        true,
    )
    .unwrap();
    let mut output = [0; 6];
    mix.paint(&mut output);
    assert_eq!(output, [-16384, -16384, 0, 0, 16256, 16256]);
}

#[test]
fn source_clock_is_native_f32_and_restarts_at_each_submitted_block() {
    let mut mix = mixer(2000);
    let fmt = format(3000, PcmChannels::Mono, PcmWidth::Short);
    let block = shorts(&[100, 200, 300, 400]);
    assert_eq!(mix.queue_raw(fmt, &block, 1., false), Ok(3));
    assert_eq!(mix.queue_raw(fmt, &block, 1., false), Ok(3));
    let mut output = [0; 12];
    mix.paint(&mut output);
    assert_eq!(
        output,
        [100, 100, 200, 200, 400, 400, 100, 100, 200, 200, 400, 400]
    );
    let fmt = format(1000, PcmChannels::Mono, PcmWidth::Short);
    assert_eq!(mix.queue_raw(fmt, &shorts(&[100, 200]), 1., false), Ok(4));
    let mut output = [0; 8];
    mix.paint(&mut output);
    assert_eq!(output, [100, 100, 100, 100, 200, 200, 200, 200]);
}

#[test]
fn raw_capacity_failure_is_atomic_wrap_and_clear_do_not_touch_effects() {
    let mut mix = mixer(22050);
    let fmt = format(22050, PcmChannels::Mono, PcmWidth::Short);
    let input = shorts(&vec![123; RAW_FRAMES]);
    assert_eq!(mix.queue_raw(fmt, &input, 1., false), Ok(RAW_FRAMES));
    assert_eq!(mix.raw_remaining(), 0);
    assert_eq!(
        mix.queue_raw(fmt, &[0, 0], 1., false),
        Err(RawError::Capacity)
    );
    let mut output = vec![0; RAW_FRAMES * 2];
    mix.paint(&mut output);
    assert!(output.iter().all(|v| *v == 123));
    assert_eq!(mix.raw_remaining(), RAW_FRAMES);
    mix.queue_raw(fmt, &shorts(&[-123]), 1., false).unwrap();
    let mut output = [0; 4];
    mix.paint(&mut output);
    assert_eq!(output, [-123, -123, 0, 0]);
    mix.queue_raw(fmt, &input, 1., false).unwrap();
    mix.clear_raw();
    mix.paint(&mut output);
    assert_eq!(output, [0; 4]);
}

#[test]
fn raw_rejects_bad_blocks_before_mutation_and_supports_24_bit_sources() {
    let mut mix = mixer(22050);
    let fmt = format(22050, PcmChannels::Stereo, PcmWidth::ThreeBytes);
    assert_eq!(
        mix.queue_raw(fmt, &[0], 1., false),
        Err(RawError::Alignment)
    );
    assert_eq!(
        mix.queue_raw(fmt, &[0; 6], f32::NAN, false),
        Err(RawError::Volume)
    );
    assert_eq!(mix.raw_remaining(), RAW_FRAMES);
    mix.queue_raw(fmt, &[9, 0, 1, 9, 0, 255], 1., false)
        .unwrap();
    let mut output = [0; 2];
    mix.paint(&mut output);
    assert_eq!(output, [256, -256]);
}

#[test]
fn raw_and_effects_saturate_only_after_common_accumulation() {
    let output_rate = rate(22050);
    let bank = Bank::load(
        output_rate,
        vec![(
            "tone".into(),
            Pcm {
                rate: output_rate,
                channels: PcmChannels::Mono,
                samples: vec![30000],
                loop_start: None,
                loop_end: None,
                loop_invalid: false,
            },
        )],
    )
    .unwrap();
    let sound = bank.find("tone").unwrap();
    let mut mix = Mixer::load(Arc::new(bank), 4, 1);
    assert!(mix.sound(SoundEvent {
        sound,
        bus: SoundBus::Local,
        volume: 1.,
        action: SoundAction::Play,
        ..Default::default()
    }));
    mix.queue_raw(
        format(22050, PcmChannels::Mono, PcmWidth::Short),
        &shorts(&[-30000]),
        1.,
        false,
    )
    .unwrap();
    let mut output = [0; 2];
    mix.paint(&mut output);
    assert_eq!(output, [-118, -118]);
}

#[test]
fn pausing_raw_retains_its_read_position_while_effects_keep_playing() {
    let output_rate = rate(22050);
    let bank = Bank::load(
        output_rate,
        vec![(
            "tone".into(),
            Pcm {
                rate: output_rate,
                channels: PcmChannels::Mono,
                samples: vec![1000; 5],
                loop_start: None,
                loop_end: None,
                loop_invalid: false,
            },
        )],
    )
    .unwrap();
    let sound = bank.find("tone").unwrap();
    let mut mix = Mixer::load(Arc::new(bank), 4, 1);
    let fmt = format(22050, PcmChannels::Mono, PcmWidth::Short);
    mix.queue_raw(fmt, &shorts(&[100, 200, 300]), 1., false)
        .unwrap();
    let mut first = [0; 2];
    mix.paint(&mut first);
    assert_eq!(first, [100; 2]);
    let remaining = mix.raw_remaining();
    mix.pause_raw(true);
    assert!(mix.sound(SoundEvent {
        sound,
        bus: SoundBus::Local,
        volume: 1.,
        action: SoundAction::Play,
        ..Default::default()
    }));
    let mut paused = [0; 8];
    mix.paint(&mut paused);
    assert_eq!(paused, [996; 8]);
    assert_eq!(mix.raw_remaining(), remaining);
    mix.queue_raw(fmt, &shorts(&[400]), 1., false).unwrap();
    mix.pause_raw(false);
    let mut resumed = [0; 8];
    mix.paint(&mut resumed);
    assert_eq!(resumed, [1196, 1196, 300, 300, 400, 400, 0, 0]);
    mix.pause_raw(true);
    mix.paint(&mut paused);
    mix.clear_raw();
    mix.queue_raw(fmt, &shorts(&[500]), 1., false).unwrap();
    mix.pause_raw(false);
    mix.paint(&mut first);
    assert_eq!(first, [500; 2]);
}
