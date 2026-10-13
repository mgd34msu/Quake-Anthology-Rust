//! Immutable effect recipes. Native boundaries cache numeric bindings at load.
use crate::primitives::{EffectEvent, EffectId, RuleSetId, Vec3};
use std::num::NonZeroU32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RandomSample {
    Mask(u32),
    Modulo(NonZeroU32),
}
impl RandomSample {
    pub fn sample(self, value: u32) -> u32 {
        match self {
            Self::Mask(mask) => value & mask,
            Self::Modulo(modulus) => value % modulus.get(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RandomOffset {
    pub sample: RandomSample,
    pub bias: i32,
}
impl RandomOffset {
    pub fn sample(self, value: u32) -> i32 {
        (self.sample.sample(value) as i32).wrapping_add(self.bias)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParticleLifetime {
    pub seconds: f64,
    /// When present, the duration is seconds * sample rather than seconds.
    pub random: Option<RandomSample>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParticleColor {
    /// None selects base; Some selects (event.color & mask) + base.
    pub event_mask: Option<u8>,
    pub base: u8,
    pub random: Option<RandomSample>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParticleVelocity {
    pub direction_scale: f32,
    pub random: Option<RandomOffset>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParticleEmission {
    /// The emitter consumes random words for lifetime, color and ramp in this
    /// order, followed by position and velocity for each component.
    pub lifetime: ParticleLifetime,
    pub color: ParticleColor,
    pub ramp: Option<RandomSample>,
    pub position: RandomOffset,
    pub velocity: ParticleVelocity,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParticleRamp {
    pub rate: f32,
    pub colors: &'static [u8],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VelocityChange {
    None,
    AddScaled(f32),
    SubtractScaled(f32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParticleMotion {
    pub ramp: Option<ParticleRamp>,
    pub velocity: VelocityChange,
    /// The float step * gravity product is promoted before the final scale.
    pub gravity_scale: f64,
    pub alpha_rate: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParticleDefinition {
    pub source: RuleSetId,
    pub emission: ParticleEmission,
    /// Emission ordinal selects a variant cyclically, before active-list order.
    pub variants: &'static [ParticleMotion],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeParticleBindings {
    pub directional: EffectId,
    pub explosion: EffectId,
}
impl NativeParticleBindings {
    pub fn event(self, position: Vec3, direction: Vec3, color: u8, wire_count: u8) -> EffectEvent {
        let (effect, count) = if wire_count == u8::MAX {
            (self.explosion, 1024)
        } else {
            (self.directional, u16::from(wire_count))
        };
        EffectEvent {
            effect,
            position,
            direction,
            count,
            color,
            ..EffectEvent::default()
        }
    }

    pub fn wire_count(self, event: EffectEvent) -> Option<u8> {
        if event.effect == self.explosion && event.count == 1024 {
            Some(u8::MAX)
        } else if event.effect == self.directional && event.count < u16::from(u8::MAX) {
            Some(event.count as u8)
        } else {
            None
        }
    }
}

#[derive(Debug)]
pub struct EffectCatalog {
    definitions: Box<[ParticleDefinition]>,
    native: [Option<NativeParticleBindings>; RuleSetId::ALL.len()],
}
impl EffectCatalog {
    pub fn new(
        definitions: Box<[ParticleDefinition]>,
        native: [Option<NativeParticleBindings>; RuleSetId::ALL.len()],
    ) -> Self {
        Self {
            definitions,
            native,
        }
    }

    pub fn definition(&self, id: EffectId) -> Option<&ParticleDefinition> {
        id.0.checked_sub(1)
            .and_then(|index| self.definitions.get(index as usize))
    }

    pub fn definitions(&self) -> &[ParticleDefinition] {
        &self.definitions
    }

    pub fn native_particles(&self, source: RuleSetId) -> Option<NativeParticleBindings> {
        self.native[source as usize]
    }

    pub fn len(&self) -> usize {
        self.definitions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.definitions.is_empty()
    }
}
