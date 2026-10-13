//! Native effect properties, independent of file, movement and presentation rules.
use qa_core::{
    effects::{
        EffectCatalog, NativeParticleBindings, ParticleColor, ParticleDefinition, ParticleEmission,
        ParticleLifetime, ParticleMotion, ParticleRamp, ParticleVelocity, RandomOffset,
        RandomSample, VelocityChange,
    },
    primitives::{EffectId, RuleSetId},
};
use std::num::NonZeroU32;

const DIRECTIONAL_MOTION: [ParticleMotion; 1] = [ParticleMotion {
    ramp: None,
    velocity: VelocityChange::None,
    gravity_scale: 0.05,
    alpha_rate: 0.0,
}];
const EXPLOSION_MOTION: [ParticleMotion; 2] = [
    ParticleMotion {
        ramp: Some(ParticleRamp {
            rate: 15.0,
            colors: &[0x6f, 0x6e, 0x6d, 0x6c, 0x6b, 0x6a, 0x68, 0x66],
        }),
        velocity: VelocityChange::SubtractScaled(1.0),
        gravity_scale: 0.05,
        alpha_rate: 0.0,
    },
    ParticleMotion {
        ramp: Some(ParticleRamp {
            rate: 10.0,
            colors: &[0x6f, 0x6d, 0x6b, 0x69, 0x67, 0x65, 0x63, 0x61],
        }),
        velocity: VelocityChange::AddScaled(4.0),
        gravity_scale: 0.05,
        alpha_rate: 0.0,
    },
];

pub fn native_catalog() -> EffectCatalog {
    let definitions = Box::new([
        ParticleDefinition {
            source: RuleSetId::Quake,
            emission: ParticleEmission {
                lifetime: ParticleLifetime {
                    seconds: 0.1,
                    random: Some(RandomSample::Modulo(NonZeroU32::MIN.saturating_add(4))),
                },
                color: ParticleColor {
                    event_mask: Some(!7),
                    base: 0,
                    random: Some(RandomSample::Mask(7)),
                },
                ramp: None,
                position: RandomOffset {
                    sample: RandomSample::Mask(15),
                    bias: -8,
                },
                velocity: ParticleVelocity {
                    direction_scale: 15.0,
                    random: None,
                },
            },
            variants: &DIRECTIONAL_MOTION,
        },
        ParticleDefinition {
            source: RuleSetId::Quake,
            emission: ParticleEmission {
                lifetime: ParticleLifetime {
                    seconds: 5.0,
                    random: None,
                },
                color: ParticleColor {
                    event_mask: None,
                    base: 0x6f,
                    random: None,
                },
                ramp: Some(RandomSample::Mask(3)),
                position: RandomOffset {
                    sample: RandomSample::Modulo(NonZeroU32::MIN.saturating_add(31)),
                    bias: -16,
                },
                velocity: ParticleVelocity {
                    direction_scale: 0.0,
                    random: Some(RandomOffset {
                        sample: RandomSample::Modulo(NonZeroU32::MIN.saturating_add(511)),
                        bias: -256,
                    }),
                },
            },
            variants: &EXPLOSION_MOTION,
        },
    ]);
    let mut native = [None; RuleSetId::ALL.len()];
    native[RuleSetId::Quake as usize] = Some(NativeParticleBindings {
        directional: EffectId(1),
        explosion: EffectId(2),
    });
    EffectCatalog::new(definitions, native)
}
