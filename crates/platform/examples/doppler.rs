//! Binary correctness bridge for the extracted original Q3 paint function.
use qa_audio::{Bank, Listener, Mixer};
use qa_core::primitives::*;
use std::{
    io::{Read, Write},
    num::NonZeroU32,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut bytes = Vec::new();
    std::io::stdin().read_to_end(&mut bytes)?;
    let mut cursor = 0;
    let mut output = std::io::BufWriter::new(std::io::stdout().lock());
    while cursor < bytes.len() {
        let header = bytes.get(cursor..cursor + 32).ok_or("doppler header")?;
        let words = header.as_chunks::<4>().0;
        let word = |at: usize| u32::from_le_bytes(words[at]);
        let frames = word(0) as usize;
        let start = word(1);
        let count = word(2) as usize;
        let scale = f32::from_bits(word(3));
        let previous_scale = f32::from_bits(word(4));
        let volume = f32::from_bits(word(5));
        let voices = word(6) as usize;
        let doppler_mask = word(7);
        cursor += 32;
        let pcm = bytes
            .get(cursor..cursor + frames * 2)
            .ok_or("doppler samples")?;
        let samples = pcm
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&pair| i16::from_le_bytes(pair))
            .collect();
        cursor += frames * 2;
        let rate = NonZeroU32::new(44100).ok_or("device rate")?;
        let bank = Bank::load(
            rate,
            vec![(
                "sound/probe.wav".into(),
                Pcm {
                    rate,
                    channels: PcmChannels::Mono,
                    samples,
                    loop_start: None,
                    loop_end: None,
                    loop_invalid: false,
                },
            )],
        )?;
        let mut mixer = Mixer::load(bank.rate, voices, 1);
        let client = ClientId(1);
        mixer.listen(
            &[Some(Listener {
                client,
                entity: None,
                origin: Vec3::default(),
                right: Vec3([1., 0., 0.]),
                negate_dot: false,
            })],
            volume,
        );
        mixer.counts.frames = u64::from(start);
        let appearance = EntityAppearance {
            loop_sound: SoundRef::native(ResourceNamespace::Client(client), 1),
            ..Default::default()
        };
        let spatial = qa_gameplay::rules::loop_spatial_policy(RuleSetId::Quake3);
        let (playback, _) = qa_gameplay::rules::loop_rules(RuleSetId::Quake3)
            .ok_or("loop rules")?
            .admit(&appearance, spatial)
            .ok_or("loop playback")?;
        for voice in 0..voices {
            if !mixer.sound(
                &bank,
                SoundEvent {
                    sound: bank.find("sound/probe.wav").ok_or("PCM")?,
                    looping: Some(playback),
                    spatial,
                    doppler: ((doppler_mask & (1 << voice)) != 0).then_some(LoopDoppler {
                        scale,
                        previous_scale,
                    }),
                    volume: 1.,
                    attenuation: 1.,
                    action: SoundAction::StartLoop,
                    client: Some(client),
                    client_lifetime: Some(EntityId {
                        slot: 0,
                        generation: 1,
                    }),
                    entity: Some(SoundEmitter::Native {
                        namespace: ResourceNamespace::Client(client),
                        slot: 2 + voice as i32,
                    }),
                    ..Default::default()
                },
            ) {
                return Err("loop rejected".into());
            }
        }
        let mut pcm = vec![0i16; count * 2];
        if !mixer.paint(&bank, &mut pcm) {
            return Err("PCM paint".into());
        }
        for sample in pcm {
            output.write_all(&sample.to_le_bytes())?;
        }
    }
    output.flush()?;
    Ok(())
}
