//! The existing native delta encoder consumes these projected field-table words.
//! This value borrows no engine ownership; namespaces remain caller-selected.
use super::{Convert, EntityContext, Field};
use crate::{commands::packet::Protocol, states};
use qa_core::primitives::{
    EntityAppearance, EntityClientData, EntityLight, EntityModelData, EntityParameter,
    EntityPresentation, EntityRecord, ModelRef, NativeClientRef, NativeEntityRef, NativeReference,
    ResourceNamespace, RuleSetId, SoundRef, ThinkTime, Trajectory as PublishedTrajectory,
    TrajectoryCurve,
};
use qa_core::solids::{PackedSolid, SolidCodec};
use qa_gameplay::rules::entities::{
    EntityRoles, ModelCellRole, ParameterRole, TrajectoryRules, entity_roles,
};

#[derive(Clone, Copy)]
enum TrajectoryField {
    Start,
    Duration,
    Curve,
    Base(usize),
    Delta(usize),
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
    EntityType,
    OtherEntity,
    Sound,
    Solid,
    Alpha,
    Scale,
    LoopVolume,
    LoopAttenuation,
    Trajectory(bool, TrajectoryField),
    PublishedPosition(usize),
    PublishedAngle(usize),
    SecondaryPosition(usize),
    SecondaryAngle(usize),
    TorsoAnimation,
    LegsAnimation,
    EventParameter,
    Ground,
    OtherEntity2,
    Client,
    Weapon,
    Powerups,
    Generic,
    Time(usize),
    Light,
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
    f(12, Velocity(0), Bits),
    f(13, Velocity(1), Bits),
    f(14, Velocity(2), Bits),
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
    f(0, Trajectory(false, TrajectoryField::Start), Bits),
    f(1, Trajectory(false, TrajectoryField::Base(0)), Bits),
    f(2, Trajectory(false, TrajectoryField::Base(1)), Bits),
    f(3, Trajectory(false, TrajectoryField::Delta(0)), Bits),
    f(4, Trajectory(false, TrajectoryField::Delta(1)), Bits),
    f(5, Trajectory(false, TrajectoryField::Base(2)), Bits),
    f(6, Trajectory(true, TrajectoryField::Base(1)), Bits),
    f(7, Trajectory(false, TrajectoryField::Delta(2)), Bits),
    f(8, Trajectory(true, TrajectoryField::Base(0)), Bits),
    f(9, Event, Bits),
    f(10, SecondaryAngle(1), Bits),
    f(11, EntityType, Bits),
    f(12, TorsoAnimation, Bits),
    f(13, EventParameter, Bits),
    f(14, LegsAnimation, Bits),
    f(15, Ground, Bits),
    f(16, Trajectory(false, TrajectoryField::Curve), Bits),
    f(17, RenderFlags, Bits),
    f(18, OtherEntity, Bits),
    f(19, Weapon, Bits),
    f(20, Client, Bits),
    f(21, PublishedAngle(1), Bits),
    f(22, Trajectory(false, TrajectoryField::Duration), Bits),
    f(23, Trajectory(true, TrajectoryField::Curve), Bits),
    f(24, PublishedPosition(0), Bits),
    f(25, PublishedPosition(1), Bits),
    f(26, PublishedPosition(2), Bits),
    f(27, Solid, Bits),
    f(28, Powerups, Bits),
    f(29, Model(0), Bits),
    f(30, OtherEntity2, Bits),
    f(31, Sound, Bits),
    f(32, Generic, Bits),
    f(33, SecondaryPosition(2), Bits),
    f(34, SecondaryPosition(0), Bits),
    f(35, SecondaryPosition(1), Bits),
    f(36, Model(1), Bits),
    f(37, PublishedAngle(0), Bits),
    f(38, Time(0), Bits),
    f(39, Trajectory(true, TrajectoryField::Start), Bits),
    f(40, Trajectory(true, TrajectoryField::Duration), Bits),
    f(41, Trajectory(true, TrajectoryField::Base(2)), Bits),
    f(42, Trajectory(true, TrajectoryField::Delta(0)), Bits),
    f(43, Trajectory(true, TrajectoryField::Delta(1)), Bits),
    f(44, Trajectory(true, TrajectoryField::Delta(2)), Bits),
    f(45, Time(1), Bits),
    f(46, PublishedAngle(2), Bits),
    f(47, SecondaryAngle(0), Bits),
    f(48, SecondaryAngle(2), Bits),
    f(49, Light, Bits),
    f(50, Frame, Bits),
];

pub struct EntityProjection {
    protocol: Protocol,
    fields: &'static [Field<Source>],
    words: usize,
    resource_limit: u32,
    sound_limit: u32,
    roles: EntityRoles,
    native32: bool,
    models: usize,
    solid: Option<SolidCodec>,
    trajectory: Option<TrajectoryRules>,
}
impl EntityProjection {
    pub const fn load(protocol: Protocol) -> Self {
        let (fields, words) = match protocol {
            Protocol::NetQuake15 => (NQ, states::NQ_ENTITY_WORDS),
            Protocol::QuakeWorld28 => (QW, states::QW_ENTITY_WORDS),
            Protocol::Quake2_34 => (Q2, states::Q2_ENTITY_WORDS),
            Protocol::Quake2Repro1038 => (RR, states::Q2_RERELEASE_ENTITY_WORDS),
            Protocol::Quake3_68 => (Q3, states::ENTITY_WORDS),
        };
        let mut models = 0;
        let mut i = 0;
        while i < fields.len() {
            if let Model(index) = fields[i].source {
                if index + 1 > models {
                    models = index + 1;
                }
            }
            i += 1;
        }
        Self {
            protocol,
            fields,
            words,
            resource_limit: match protocol {
                Protocol::Quake2Repro1038 => 65535,
                _ => 255,
            },
            // Rerelease loop sound occupies the lower fourteen packed bits;
            // the upper two select its volume and attenuation fields.
            sound_limit: match protocol {
                Protocol::Quake2Repro1038 => 0x3fff,
                _ => 255,
            },
            roles: entity_roles(protocol.rule_set()),
            native32: false,
            models,
            solid: SolidCodec::load(protocol.rule_set()),
            trajectory: match protocol {
                Protocol::Quake3_68 => Some(TrajectoryRules::Q3_NATIVE),
                _ => None,
            },
        }
    }
    /// Public game-memory values retain all 32 bits; the wire instance applies
    /// its own resource/reference limits and the existing MSG field widths.
    pub const fn load_native_q3() -> Self {
        let mut value = Self::load(Protocol::Quake3_68);
        value.resource_limit = u32::MAX;
        value.sound_limit = u32::MAX;
        value.native32 = true;
        value
    }
    pub fn rule_set(&self) -> RuleSetId {
        self.protocol.rule_set()
    }
    pub fn words(&self) -> usize {
        self.words
    }
    pub fn trajectory_rules(&self) -> Option<TrajectoryRules> {
        self.trajectory
    }
    /// The resolvers map publishing resource namespaces into this connection's
    /// precaches. Missing/unrepresentable resources become native ordinal zero.
    pub fn reduce(
        &self,
        record: &EntityRecord,
        context: &EntityContext,
        out: &mut [u32],
        mut resolve_model: impl FnMut(ModelRef) -> Option<u32>,
        mut resolve_sound: impl FnMut(SoundRef) -> Option<u32>,
        mut resolve_entity: impl FnMut(NativeReference) -> Option<u32>,
    ) -> bool {
        let Some(out) = out.get_mut(..self.words) else {
            return false;
        };
        out.fill(0);
        let a = &record.appearance;
        let presentation = a.presentation.as_ref();
        let roles = self
            .roles
            .cells(a.native.entity_type, a.native.event, a.native.render_flags);
        let same = a.source == self.protocol.rule_set();
        let q2_pair = matches!(a.source, RuleSetId::Quake2 | RuleSetId::Quake2Rerelease)
            && matches!(
                self.protocol,
                Protocol::Quake2_34 | Protocol::Quake2Repro1038
            );
        let q1_pair = matches!(a.source, RuleSetId::Quake | RuleSetId::QuakeWorld)
            && matches!(self.protocol, Protocol::NetQuake15 | Protocol::QuakeWorld28);
        let model_data = presentation.filter(|_| same).map(|p| p.model_data);
        let mut models = [0; 4];
        for (i, model) in models.iter_mut().enumerate().take(self.models) {
            *model = match model_data {
                Some(EntityModelData::Item { index, secondary }) => {
                    [index, secondary].get(i).copied().unwrap_or(0) as u32
                }
                Some(EntityModelData::Team { team, health }) => {
                    [team, health].get(i).copied().unwrap_or(0) as u32
                }
                _ => a.models[i]
                    .and_then(&mut resolve_model)
                    .filter(|&n| n <= self.resource_limit)
                    .unwrap_or(0),
            };
        }
        for field in self.fields {
            let value = match field.source {
                Position(i) => record.position.0[i].to_bits(),
                Velocity(i) => record.velocity.0[i].to_bits(),
                Angle(i) => record.angles.0[i].to_bits(),
                OldOrigin(i) => a.old_origin.0[i].to_bits(),
                Model(i) => models[i],
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
                                Protocol::Quake3_68 if !self.native32 => 0x7ffff,
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
                EntityType => self
                    .roles
                    .project_type(same.then_some(a.native.entity_type), context.tags),
                OtherEntity => {
                    if same {
                        match a.native.other_entity {
                            Some(reference) if roles.other_reference => {
                                let Some(value) =
                                    self.reference(reference, false, &mut resolve_entity)
                                else {
                                    return false;
                                };
                                value
                            }
                            Some(reference) => reference.ordinal,
                            None if roles.other_reference => return false,
                            None => 0,
                        }
                    } else {
                        0
                    }
                }
                Sound => a
                    .loop_sound
                    .and_then(&mut resolve_sound)
                    .filter(|&n| n <= self.sound_limit)
                    .unwrap_or(0),
                Solid => {
                    if self.native32 && same {
                        a.native.solid
                    } else {
                        match (SolidCodec::load(a.source), self.solid) {
                            (Some(source), Some(target)) => {
                                if matches!(source.decode(a.native.solid), PackedSolid::Brush)
                                    && (models[0] == 0 || a.models[0].is_none())
                                {
                                    return false;
                                }
                                let Some(word) = target.transcode(source, a.native.solid) else {
                                    return false;
                                };
                                word
                            }
                            _ => 0,
                        }
                    }
                }
                Alpha => a.alpha.to_bits(),
                Scale => a.scale.to_bits(),
                LoopVolume => a.native.loop_volume.to_bits(),
                LoopAttenuation => encode_attenuation(a.native.loop_attenuation),
                Trajectory(angular, part) => {
                    let Some(value) =
                        trajectory_value(record, presentation.filter(|_| same), angular, part)
                    else {
                        return false;
                    };
                    value
                }
                PublishedPosition(axis) => presentation
                    .filter(|_| same)
                    .map_or(0, |p| p.pose.position.0[axis].to_bits()),
                PublishedAngle(axis) => presentation
                    .filter(|_| same)
                    .map_or(0, |p| p.pose.angles.0[axis].to_bits()),
                SecondaryPosition(axis) => presentation
                    .filter(|_| same)
                    .map_or(0, |p| p.secondary_pose.position.0[axis].to_bits()),
                SecondaryAngle(axis) => presentation
                    .filter(|_| same)
                    .map_or(0, |p| p.secondary_pose.angles.0[axis].to_bits()),
                TorsoAnimation => presentation
                    .filter(|_| same)
                    .map_or(0, |p| p.animation.torso),
                LegsAnimation => presentation
                    .filter(|_| same)
                    .map_or(0, |p| p.animation.legs),
                EventParameter => match presentation.filter(|_| same).map(|p| p.event_parameter) {
                    Some(EntityParameter::Scalar(value)) => value,
                    Some(EntityParameter::Sound(None)) | None => 0,
                    Some(EntityParameter::Sound(Some(reference))) => {
                        let Some(value) =
                            resolve_sound(reference).filter(|&value| value <= self.sound_limit)
                        else {
                            return false;
                        };
                        value
                    }
                    Some(EntityParameter::Entity(reference)) => {
                        let Some(value) = self
                            .reference(reference, false, &mut resolve_entity)
                            .filter(|&value| self.native32 || value <= 255)
                        else {
                            return false;
                        };
                        value
                    }
                },
                Ground => match presentation.filter(|_| same).and_then(|p| p.ground) {
                    Some(reference) => {
                        let Some(value) = self.reference(reference, true, &mut resolve_entity)
                        else {
                            return false;
                        };
                        value
                    }
                    None => 0,
                },
                OtherEntity2 => match presentation.filter(|_| same).and_then(|p| p.other_entity2) {
                    Some(reference) if roles.other2_reference => {
                        let Some(value) = self.reference(reference, false, &mut resolve_entity)
                        else {
                            return false;
                        };
                        value
                    }
                    Some(reference) => reference.ordinal,
                    None if same && roles.other2_reference => return false,
                    None => 0,
                },
                Client => match if same {
                    presentation.map(|p| p.client)
                } else {
                    context.client.map(EntityClientData::Client)
                } {
                    Some(EntityClientData::Scalar(value)) => value as u32,
                    Some(EntityClientData::Client(reference)) => {
                        let Some(value) = resolve_entity(NativeReference::Client(reference))
                            .filter(|&value| {
                                self.native32
                                    || value
                                        < qa_core::configstrings::ConfigLayout::for_rules(
                                            RuleSetId::Quake3,
                                        )
                                        .players
                                        .count
                            })
                        else {
                            return false;
                        };
                        value
                    }
                    None => 0,
                },
                Weapon => presentation.filter(|_| same).map_or(0, |p| p.weapon),
                Powerups => presentation.filter(|_| same).map_or(0, |p| p.powerups),
                Generic => presentation.filter(|_| same).map_or(0, |p| p.generic),
                Time(index) => match presentation.filter(|_| same).map(|p| p.times[index]) {
                    Some(ThinkTime::Milliseconds(time)) => time as u32,
                    Some(ThinkTime::Seconds(_)) => return false,
                    None => 0,
                },
                Light => presentation
                    .filter(|_| same)
                    .map_or(0, |p| p.light.packed()),
            };
            out[field.word] = field.convert.export(value);
        }
        true
    }
    fn reference(
        &self,
        reference: NativeEntityRef,
        ground: bool,
        resolve: &mut impl FnMut(NativeReference) -> Option<u32>,
    ) -> Option<u32> {
        if self.native32 {
            return resolve(NativeReference::Entity(reference));
        }
        // Native sentinels have meaning without a live entity-table row.
        if reference.ordinal == 1022 || reference.ordinal == 1023 {
            return Some(reference.ordinal);
        }
        if ground && reference.ordinal == u32::MAX {
            return Some(1023);
        }
        resolve(NativeReference::Entity(reference)).filter(|&value| value < 1023)
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
        let roles = if self.protocol == Protocol::Quake3_68 {
            let roles = self.roles.cells(words[11], words[9], words[17]);
            let mut presentation = EntityPresentation::new(namespace);
            presentation.model_data = match roles.models {
                ModelCellRole::Models => EntityModelData::Models,
                ModelCellRole::Item => EntityModelData::Item {
                    index: words[29] as i32,
                    secondary: words[36] as i32,
                },
                ModelCellRole::Team => EntityModelData::Team {
                    team: words[29] as i32,
                    health: words[36] as i32,
                },
            };
            record.appearance.presentation = Some(presentation);
            Some(roles)
        } else {
            None
        };
        for field in self.fields {
            let value = field.convert.import(words[field.word]);
            let a = &mut record.appearance;
            match field.source {
                Position(i) => record.position.0[i] = f32::from_bits(value),
                Velocity(i) => record.velocity.0[i] = f32::from_bits(value),
                Angle(i) => record.angles.0[i] = f32::from_bits(value),
                OldOrigin(i) => a.old_origin.0[i] = f32::from_bits(value),
                Model(i) => {
                    if roles.is_none_or(|roles| roles.models == ModelCellRole::Models) {
                        a.models[i] = ModelRef::native(namespace, value);
                    }
                }
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
                EntityType => a.native.entity_type = value,
                OtherEntity => {
                    a.native.other_entity = Some(NativeEntityRef::entity(namespace, value))
                }
                Sound => a.loop_sound = SoundRef::native(namespace, value),
                Solid => a.native.solid = value,
                Alpha => a.alpha = f32::from_bits(value),
                Scale => a.scale = f32::from_bits(value),
                LoopVolume => a.native.loop_volume = f32::from_bits(value),
                LoopAttenuation => a.native.loop_attenuation = decode_attenuation(value),
                Trajectory(_, _)
                | PublishedPosition(_)
                | PublishedAngle(_)
                | SecondaryPosition(_)
                | SecondaryAngle(_)
                | TorsoAnimation
                | LegsAnimation
                | EventParameter
                | Ground
                | OtherEntity2
                | Client
                | Weapon
                | Powerups
                | Generic
                | Time(_)
                | Light => {
                    let Some(p) = a.presentation.as_mut() else {
                        return false;
                    };
                    match field.source {
                        Trajectory(angular, part) => {
                            let trajectory = trajectory_mut(p, angular);
                            match part {
                                TrajectoryField::Start => {
                                    trajectory.start =
                                        ThinkTime::Milliseconds(i64::from(value as i32))
                                }
                                TrajectoryField::Duration => {
                                    trajectory.duration_ms = i64::from(value as i32)
                                }
                                TrajectoryField::Curve => {
                                    trajectory.curve = curve_from_native(value)
                                }
                                TrajectoryField::Base(axis) => {
                                    trajectory.base.0[axis] = f32::from_bits(value)
                                }
                                TrajectoryField::Delta(axis) => {
                                    trajectory.delta.0[axis] = f32::from_bits(value)
                                }
                            }
                        }
                        PublishedPosition(axis) => p.pose.position.0[axis] = f32::from_bits(value),
                        PublishedAngle(axis) => p.pose.angles.0[axis] = f32::from_bits(value),
                        SecondaryPosition(axis) => {
                            p.secondary_pose.position.0[axis] = f32::from_bits(value)
                        }
                        SecondaryAngle(axis) => {
                            p.secondary_pose.angles.0[axis] = f32::from_bits(value)
                        }
                        TorsoAnimation => p.animation.torso = value,
                        LegsAnimation => p.animation.legs = value,
                        EventParameter => {
                            let Some(roles) = roles else {
                                return false;
                            };
                            p.event_parameter = match roles.parameter {
                                ParameterRole::Scalar => EntityParameter::Scalar(value),
                                ParameterRole::Sound => {
                                    EntityParameter::Sound(SoundRef::native(namespace, value))
                                }
                                ParameterRole::Entity => EntityParameter::Entity(
                                    NativeEntityRef::entity(namespace, value),
                                ),
                            }
                        }
                        Ground => p.ground = Some(NativeEntityRef::entity(namespace, value)),
                        OtherEntity2 => {
                            p.other_entity2 = Some(NativeEntityRef::entity(namespace, value))
                        }
                        Client => {
                            let Some(roles) = roles else {
                                return false;
                            };
                            p.client = if roles.client_reference {
                                EntityClientData::Client(NativeClientRef::client(namespace, value))
                            } else {
                                EntityClientData::Scalar(value as i32)
                            }
                        }
                        Weapon => p.weapon = value,
                        Powerups => p.powerups = value,
                        Generic => p.generic = value,
                        Time(index) => {
                            p.times[index] = ThinkTime::Milliseconds(i64::from(value as i32))
                        }
                        Light => p.light = EntityLight::from_packed(value),
                        _ => return false,
                    }
                }
            }
        }
        if let Some(p) = record.appearance.presentation {
            record.position = p.trajectories.position.base;
            record.velocity = p.trajectories.position.delta;
            record.angles = p.trajectories.angles.base;
        }
        true
    }
}

fn trajectory_mut(
    presentation: &mut EntityPresentation,
    angular: bool,
) -> &mut PublishedTrajectory {
    if angular {
        &mut presentation.trajectories.angles
    } else {
        &mut presentation.trajectories.position
    }
}
fn trajectory_value(
    record: &EntityRecord,
    presentation: Option<&EntityPresentation>,
    angular: bool,
    part: TrajectoryField,
) -> Option<u32> {
    let fallback = PublishedTrajectory {
        curve: TrajectoryCurve::Interpolate,
        base: if angular {
            record.angles
        } else {
            record.position
        },
        delta: if angular {
            Default::default()
        } else {
            record.velocity
        },
        ..Default::default()
    };
    let trajectory = presentation.map_or(&fallback, |p| {
        if angular {
            &p.trajectories.angles
        } else {
            &p.trajectories.position
        }
    });
    Some(match part {
        TrajectoryField::Start => match trajectory.start {
            ThinkTime::Milliseconds(value) => value as u32,
            ThinkTime::Seconds(_) => return None,
        },
        TrajectoryField::Duration => trajectory.duration_ms as u32,
        TrajectoryField::Curve => curve_native(trajectory.curve),
        TrajectoryField::Base(axis) => trajectory.base.0[axis].to_bits(),
        TrajectoryField::Delta(axis) => trajectory.delta.0[axis].to_bits(),
    })
}
fn curve_from_native(value: u32) -> TrajectoryCurve {
    match value {
        0 => TrajectoryCurve::Stationary,
        1 => TrajectoryCurve::Interpolate,
        2 => TrajectoryCurve::Linear,
        3 => TrajectoryCurve::LinearStop,
        4 => TrajectoryCurve::Sine,
        5 => TrajectoryCurve::Gravity,
        value => TrajectoryCurve::Unsupported(value),
    }
}
fn curve_native(curve: TrajectoryCurve) -> u32 {
    match curve {
        TrajectoryCurve::Stationary => 0,
        TrajectoryCurve::Interpolate => 1,
        TrajectoryCurve::Linear => 2,
        TrajectoryCurve::LinearStop => 3,
        TrajectoryCurve::Sine => 4,
        TrajectoryCurve::Gravity => 5,
        TrajectoryCurve::Unsupported(value) => value,
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
