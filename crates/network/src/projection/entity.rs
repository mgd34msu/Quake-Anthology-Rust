//! The existing native delta encoder consumes these projected field-table words.
//! This value borrows no engine ownership; namespaces remain caller-selected.
use super::{Convert, Field};
use crate::{commands::packet::Protocol, states};
use qa_core::primitives::{EntityAppearance, ModelRef, ResourceNamespace, RuleSetId, Vec3};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EntityRecord {
    pub position: Vec3,
    pub velocity: Vec3,
    pub angles: Vec3,
    pub appearance: EntityAppearance,
}
#[derive(Clone, Copy)]
enum Source {
    Position(usize),
    Velocity(usize),
    Angle(usize),
    OldOrigin(usize),
    Model(usize),
    Frame,
    Skin,
    Colormap,
    EffectsLow,
    EffectsHigh,
    RenderFlags,
    Event,
    Sound,
    Solid,
    Alpha,
    Scale,
    LoopVolume,
    LoopAttenuation,
}
use Convert::*;
use Source::*;
const fn f(word: usize, source: Source, convert: Convert) -> Field<Source> {
    Field {
        word,
        source,
        convert,
    }
}
const NQ: &[Field<Source>] = &[
    f(0, Model(0), IntegerFloat),
    f(1, Frame, IntegerFloat),
    f(2, Colormap, IntegerFloat),
    f(3, Skin, IntegerFloat),
    f(4, EffectsLow, IntegerFloat),
    f(5, Position(0), Bits),
    f(6, Angle(0), Bits),
    f(7, Position(1), Bits),
    f(8, Angle(1), Bits),
    f(9, Position(2), Bits),
    f(10, Angle(2), Bits),
    f(11, RenderFlags, Bits),
];
const QW: &[Field<Source>] = &[
    f(0, Model(0), Bits),
    f(1, Frame, Bits),
    f(2, Colormap, Bits),
    f(3, Skin, Bits),
    f(4, EffectsLow, Bits),
    f(5, Position(0), Bits),
    f(6, Angle(0), Bits),
    f(7, Position(1), Bits),
    f(8, Angle(1), Bits),
    f(9, Position(2), Bits),
    f(10, Angle(2), Bits),
    f(11, RenderFlags, Bits),
];
const Q2: &[Field<Source>] = &[
    f(0, Model(0), Bits),
    f(1, Model(1), Bits),
    f(2, Model(2), Bits),
    f(3, Model(3), Bits),
    f(4, Frame, Bits),
    f(5, Skin, Bits),
    f(6, EffectsLow, Bits),
    f(7, RenderFlags, Bits),
    f(8, Position(0), Bits),
    f(9, Position(1), Bits),
    f(10, Position(2), Bits),
    f(11, Angle(0), Bits),
    f(12, Angle(1), Bits),
    f(13, Angle(2), Bits),
    f(14, OldOrigin(0), Bits),
    f(15, OldOrigin(1), Bits),
    f(16, OldOrigin(2), Bits),
    f(17, Sound, Bits),
    f(18, Event, Bits),
    f(19, Solid, Bits),
];
const fn rr() -> [Field<Source>; 25] {
    let mut fields = [f(0, Frame, Bits); 25];
    let mut i = 0;
    while i < Q2.len() {
        fields[i] = Q2[i];
        if i >= 11 && i <= 13 {
            fields[i].convert = ShortAngle;
        }
        i += 1;
    }
    fields[20] = f(20, EffectsHigh, Bits);
    fields[21] = f(
        21,
        Alpha,
        ScaledByte {
            scale: 255,
            nonzero: true,
            implicit: 0,
        },
    );
    fields[22] = f(
        22,
        Scale,
        ScaledByte {
            scale: 16,
            nonzero: true,
            implicit: 16,
        },
    );
    fields[23] = f(
        23,
        LoopVolume,
        ScaledByte {
            scale: 255,
            nonzero: false,
            implicit: 255,
        },
    );
    fields[24] = f(24, LoopAttenuation, Bits);
    fields
}
const RR: &[Field<Source>] = &rr();
const Q3: &[Field<Source>] = &[
    f(1, Position(0), Bits),
    f(2, Position(1), Bits),
    f(5, Position(2), Bits),
    f(3, Velocity(0), Bits),
    f(4, Velocity(1), Bits),
    f(7, Velocity(2), Bits),
    f(8, Angle(0), Bits),
    f(6, Angle(1), Bits),
    f(41, Angle(2), Bits),
    f(9, Event, Bits),
    f(17, RenderFlags, Bits),
    f(27, Solid, Bits),
    f(29, Model(0), Bits),
    f(36, Model(1), Bits),
    f(31, Sound, Bits),
    f(50, Frame, Bits),
];

pub struct EntityProjection {
    protocol: Protocol,
    fields: &'static [Field<Source>],
    words: usize,
}
impl EntityProjection {
    pub fn load(protocol: Protocol) -> Self {
        let (fields, words) = match protocol {
            Protocol::NetQuake15 => (NQ, states::NQ_ENTITY_WORDS),
            Protocol::QuakeWorld28 => (QW, states::QW_ENTITY_WORDS),
            Protocol::Quake2_34 => (Q2, states::Q2_ENTITY_WORDS),
            Protocol::Quake2Repro1038 => (RR, states::Q2_RERELEASE_ENTITY_WORDS),
            Protocol::Quake3_68 => (Q3, states::ENTITY_WORDS),
        };
        Self {
            protocol,
            fields,
            words,
        }
    }
    pub fn rule_set(&self) -> RuleSetId {
        self.protocol.rule_set()
    }
    pub fn words(&self) -> usize {
        self.words
    }
    /// The resolver maps a publishing namespace's ordinal into this connection's
    /// precache. Missing/unrepresentable resources become native model zero.
    pub fn reduce(
        &self,
        record: &EntityRecord,
        out: &mut [u32],
        mut resolve_model: impl FnMut(ModelRef) -> Option<u32>,
    ) -> bool {
        let Some(out) = out.get_mut(..self.words) else {
            return false;
        };
        out.fill(0);
        let a = &record.appearance;
        let same = a.source == self.protocol.rule_set();
        let q2_pair = matches!(a.source, RuleSetId::Quake2 | RuleSetId::Quake2Rerelease)
            && matches!(
                self.protocol,
                Protocol::Quake2_34 | Protocol::Quake2Repro1038
            );
        let q1_pair = matches!(a.source, RuleSetId::Quake | RuleSetId::QuakeWorld)
            && matches!(self.protocol, Protocol::NetQuake15 | Protocol::QuakeWorld28);
        for field in self.fields {
            let value = match field.source {
                Position(i) => record.position.0[i].to_bits(),
                Velocity(i) => record.velocity.0[i].to_bits(),
                Angle(i) => record.angles.0[i].to_bits(),
                OldOrigin(i) => a.old_origin.0[i].to_bits(),
                Model(i) => a.models[i]
                    .and_then(&mut resolve_model)
                    .filter(|&n| {
                        n <= if self.protocol == Protocol::Quake2Repro1038 {
                            65535
                        } else {
                            255
                        }
                    })
                    .unwrap_or(0),
                Frame => a.frame as u32,
                Skin => a.skin as u32,
                Colormap => {
                    if q1_pair {
                        a.native.colormap
                    } else {
                        0
                    }
                }
                EffectsLow => {
                    if same || q2_pair || q1_pair {
                        a.native.effects as u32 & if q1_pair { 255 } else { u32::MAX }
                    } else {
                        0
                    }
                }
                EffectsHigh => {
                    if same {
                        (a.native.effects >> 32) as u32
                    } else {
                        0
                    }
                }
                RenderFlags => {
                    if same || q2_pair {
                        a.native.render_flags
                            & match self.protocol {
                                Protocol::NetQuake15 => 1,
                                Protocol::QuakeWorld28 => 64,
                                Protocol::Quake3_68 => 0x7ffff,
                                _ => u32::MAX,
                            }
                    } else {
                        0
                    }
                }
                Event => {
                    if same || q2_pair {
                        a.native.event
                    } else {
                        0
                    }
                }
                Sound => {
                    if same || q2_pair {
                        a.native.loop_sound
                    } else {
                        0
                    }
                }
                Solid => {
                    if same || q2_pair {
                        a.native.solid
                    } else {
                        0
                    }
                }
                Alpha => a.alpha.to_bits(),
                Scale => a.scale.to_bits(),
                LoopVolume => a.native.loop_volume.to_bits(),
                LoopAttenuation => encode_attenuation(a.native.loop_attenuation),
            };
            out[field.word] = field.convert.export(value);
        }
        // Q3 trajectories use stationary/interpolate bases for these common
        // poses. Source-only unrepresented trajectory/animation words stay zero.
        true
    }
    /// Consume decoder words; QC integer floats have already become integers.
    /// No common handle is manufactured from a native ordinal.
    pub fn apply(
        &self,
        words: &[u32],
        record: &mut EntityRecord,
        namespace: ResourceNamespace,
    ) -> bool {
        let Some(words) = words.get(..self.words) else {
            return false;
        };
        record.appearance = EntityAppearance {
            source: self.protocol.rule_set(),
            ..Default::default()
        };
        for field in self.fields {
            let value = field.convert.import(words[field.word]);
            let a = &mut record.appearance;
            match field.source {
                Position(i) => record.position.0[i] = f32::from_bits(value),
                Velocity(i) => record.velocity.0[i] = f32::from_bits(value),
                Angle(i) => record.angles.0[i] = f32::from_bits(value),
                OldOrigin(i) => a.old_origin.0[i] = f32::from_bits(value),
                Model(i) => a.models[i] = ModelRef::native(namespace, value),
                Frame => a.frame = value as i32,
                Skin => a.skin = value as i32,
                Colormap => a.native.colormap = value,
                EffectsLow => {
                    a.native.effects = (a.native.effects & 0xffff_ffff_0000_0000) | u64::from(value)
                }
                EffectsHigh => {
                    a.native.effects = (a.native.effects & 0xffff_ffff) | (u64::from(value) << 32)
                }
                RenderFlags => a.native.render_flags = value,
                Event => a.native.event = value,
                Sound => a.native.loop_sound = value,
                Solid => a.native.solid = value,
                Alpha => a.alpha = f32::from_bits(value),
                Scale => a.scale = f32::from_bits(value),
                LoopVolume => a.native.loop_volume = f32::from_bits(value),
                LoopAttenuation => a.native.loop_attenuation = decode_attenuation(value),
            }
        }
        true
    }
}
fn encode_attenuation(value: f32) -> u32 {
    if value == -1.0 {
        192
    } else {
        let packed = (value * 64.0).clamp(0.0, 255.0) as u32;
        if packed == 192 { 0 } else { packed }
    }
}
fn decode_attenuation(value: u32) -> f32 {
    if value == 192 {
        -1.0
    } else {
        value as u8 as f32 / 64.0
    }
}
