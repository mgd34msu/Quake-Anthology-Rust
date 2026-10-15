use qa_core::primitives::{
    ClientId, CollisionTags, EntityAppearance, EntityClientData, EntityModelData, EntityParameter,
    EntityPresentation, ModelRef, ModelUse, ModuleId, NativeAppearanceTail, NativeClientRef,
    NativeEntityRef, NativeReference, ResourceNamespace, RuleSetId, SoundRef, ThinkTime,
    TrajectoryCurve, Vec3,
};
use qa_network::{
    commands::packet::Protocol,
    message::{Encoding, Error, Reader, Writer},
    projection::{EntityContext, EntityProjection, EntityRecord},
    states,
};

const MODULE: ResourceNamespace = ResourceNamespace::Module(ModuleId(7));
const CLIENT: ResourceNamespace = ResourceNamespace::Client(ClientId(3));

#[test]
fn identical_model_references_resolve_with_their_actual_entity_cell() -> Result<(), Error> {
    for protocol in [Protocol::Quake2_34, Protocol::Quake2Repro1038] {
        let projection = EntityProjection::load(protocol);
        let mut record = EntityRecord::default();
        record.appearance.source = RuleSetId::Quake2;
        record.appearance.models = [ModelRef::native(MODULE, 0x87); 4];
        let mut words = vec![0; projection.words()];
        let mut uses = Vec::new();
        assert!(projection.reduce(
            &record,
            &EntityContext::default(),
            &mut words,
            |reference, usage| {
                assert_eq!(reference, ModelRef::native(MODULE, 0x87).unwrap());
                let ModelUse::Entity(cell) = usage else {
                    panic!("entity model cell");
                };
                uses.push(cell);
                Some([17, 0x91, 19, 20][cell])
            },
            |_| None,
            |_| None,
        ));
        assert_eq!(uses, [0, 1, 2, 3]);
        let mut accepted = EntityRecord::default();
        assert!(projection.apply(&wire(protocol, &words)?, &mut accepted, CLIENT));
        assert_eq!(
            accepted.appearance.models,
            [17, 0x91, 19, 20].map(|ordinal| ModelRef::native(CLIENT, ordinal)),
        );
    }
    Ok(())
}

fn source(protocol: Protocol) -> EntityRecord {
    EntityRecord {
        position: Vec3([12.375, -42.625, 48.125]),
        velocity: Vec3([1., -2., 3.]),
        angles: Vec3([45., 90., -90.]),
        appearance: EntityAppearance {
            source: protocol.rule_set(),
            snapshot_excluded: false,
            models: [
                ModelRef::native(MODULE, 8),
                ModelRef::native(MODULE, 9),
                ModelRef::native(MODULE, 10),
                ModelRef::native(MODULE, 11),
            ],
            loop_sound: SoundRef::native(MODULE, 23),
            presentation: None,
            frame: 31,
            skin: 5,
            old_origin: Vec3([-3.375, 20.125, -9.625]),
            alpha: 0.5,
            scale: 2.,
            native: NativeAppearanceTail {
                effects: 0x9876_5432_abcd_1234,
                render_flags: match protocol {
                    Protocol::NetQuake15 => 1,
                    Protocol::QuakeWorld28 => 64,
                    Protocol::Quake3_68 => 0x76543,
                    _ => 0x1234_5678,
                },
                event: 7,
                entity_type: 7,
                other_entity: Some(NativeEntityRef::entity(MODULE, 17)),
                solid: match protocol {
                    Protocol::Quake2_34 => 0x2062,
                    Protocol::Quake2Repro1038 => 0x4018_1810,
                    Protocol::Quake3_68 => 0x0040_1810,
                    _ => 0,
                },
                colormap: 4,
                loop_volume: 0.5,
                loop_attenuation: -1.,
                ..Default::default()
            },
        },
    }
}
fn wire(protocol: Protocol, words: &[u32]) -> Result<Vec<u32>, Error> {
    let encoding = if protocol == Protocol::Quake3_68 {
        Encoding::Q3
    } else {
        Encoding::Bytes
    };
    let mut bytes = [0; 1400];
    let mut writer = Writer::new(&mut bytes, encoding);
    match protocol {
        Protocol::NetQuake15 => {
            states::write_nq_entity(
                &mut writer,
                17,
                &[0; states::NQ_ENTITY_WORDS],
                words.try_into().unwrap(),
                true,
            )?;
        }
        Protocol::QuakeWorld28 => {
            states::write_qw_entity(
                &mut writer,
                17,
                &[0; states::QW_ENTITY_WORDS],
                Some(words.try_into().unwrap()),
                true,
            )?;
        }
        Protocol::Quake2_34 => {
            states::write_q2_entity(
                &mut writer,
                17,
                &[0; states::Q2_ENTITY_WORDS],
                Some(words.try_into().unwrap()),
                true,
                true,
            )?;
        }
        Protocol::Quake2Repro1038 => {
            states::write_q2_repro_entity(
                &mut writer,
                17,
                &[0; states::Q2_RERELEASE_ENTITY_WORDS],
                Some(words.try_into().unwrap()),
                true,
            )?;
        }
        Protocol::Quake3_68 => {
            states::write_q3_entity(
                &mut writer,
                17,
                &[0; states::ENTITY_WORDS],
                Some(words.try_into().unwrap()),
                true,
            )?;
        }
    }
    let mut reader = Reader::new(writer.bytes(), encoding);
    Ok(match protocol {
        Protocol::NetQuake15 => states::read_nq_entity(&mut reader, &[0; states::NQ_ENTITY_WORDS])?
            .words
            .unwrap()
            .to_vec(),
        Protocol::QuakeWorld28 => {
            states::read_qw_entity(&mut reader, &[0; states::QW_ENTITY_WORDS])?
                .words
                .unwrap()
                .to_vec()
        }
        Protocol::Quake2_34 => states::read_q2_entity(&mut reader, &[0; states::Q2_ENTITY_WORDS])?
            .words
            .unwrap()
            .to_vec(),
        Protocol::Quake2Repro1038 => {
            states::read_q2_repro_entity(&mut reader, &[0; states::Q2_RERELEASE_ENTITY_WORDS])?
                .words
                .unwrap()
                .to_vec()
        }
        Protocol::Quake3_68 => states::read_q3_entity(&mut reader, &[0; states::ENTITY_WORDS])?
            .words
            .unwrap()
            .to_vec(),
    })
}
#[test]
fn all_native_entity_tables_retain_represented_fields_and_explicit_namespace() -> Result<(), Error>
{
    for protocol in [
        Protocol::NetQuake15,
        Protocol::QuakeWorld28,
        Protocol::Quake2_34,
        Protocol::Quake2Repro1038,
        Protocol::Quake3_68,
    ] {
        let projection = EntityProjection::load(protocol);
        let src = source(protocol);
        let mut words = vec![0; projection.words()];
        assert!(projection.reduce(
            &src,
            &EntityContext::default(),
            &mut words,
            |model, _| {
                assert_eq!(model.namespace, MODULE);
                Some(model.ordinal)
            },
            |sound| {
                assert_eq!(sound.namespace, MODULE);
                Some(sound.ordinal)
            },
            |reference| Some(reference.ordinal()),
        ));
        let decoded = wire(protocol, &words)?;
        let mut imported = EntityRecord::default();
        assert!(projection.apply(&decoded, &mut imported, CLIENT));
        assert_eq!(imported.appearance.source, protocol.rule_set());
        assert_eq!(imported.appearance.models[0], ModelRef::native(CLIENT, 8));
        assert_eq!(imported.appearance.frame, 31);
        assert_eq!(imported.position, src.position);
        if protocol == Protocol::Quake3_68 {
            assert_eq!(imported.appearance.native.entity_type, 7);
            assert_eq!(
                imported.appearance.native.other_entity,
                Some(NativeEntityRef::entity(CLIENT, 17))
            );
        }
        assert_eq!(
            imported.appearance.loop_sound,
            if matches!(
                protocol,
                Protocol::Quake2_34 | Protocol::Quake2Repro1038 | Protocol::Quake3_68
            ) {
                SoundRef::native(CLIENT, 23)
            } else {
                None
            }
        );
        if matches!(protocol, Protocol::Quake2_34 | Protocol::Quake2Repro1038) {
            assert_eq!(imported.appearance.old_origin, src.appearance.old_origin);
            assert_eq!(
                imported.appearance.native.render_flags,
                src.appearance.native.render_flags
            );
        }
        if protocol == Protocol::Quake2Repro1038 {
            assert_eq!(
                imported.appearance.native.effects,
                src.appearance.native.effects
            );
            assert_eq!(imported.appearance.alpha, 127. / 255.);
            assert_eq!(imported.appearance.scale, 2.);
            assert_eq!(imported.appearance.native.loop_attenuation, -1.);
        }
        assert!(projection.reduce(
            &imported,
            &EntityContext::default(),
            &mut words,
            |model, _| {
                assert_eq!(model.namespace, CLIENT);
                Some(model.ordinal)
            },
            |sound| {
                assert_eq!(sound.namespace, CLIENT);
                Some(sound.ordinal)
            },
            |reference| Some(reference.ordinal()),
        ));
        assert_eq!(wire(protocol, &words)?, decoded, "{protocol:?}");
    }
    Ok(())
}
#[test]
fn models_are_resolved_and_missing_or_unrepresentable_models_become_zero() -> Result<(), Error> {
    for protocol in [
        Protocol::NetQuake15,
        Protocol::QuakeWorld28,
        Protocol::Quake2_34,
        Protocol::Quake2Repro1038,
        Protocol::Quake3_68,
    ] {
        let projection = EntityProjection::load(protocol);
        let mut src = source(protocol);
        src.appearance.models = [ModelRef::native(MODULE, 1); 4];
        let mut words = vec![0; projection.words()];
        let wide = if protocol == Protocol::Quake2Repro1038 {
            65535
        } else {
            255
        };
        projection.reduce(
            &src,
            &EntityContext::default(),
            &mut words,
            |_, _| Some(wide),
            |sound| Some(sound.ordinal),
            |reference| Some(reference.ordinal()),
        );
        let mut dst = EntityRecord::default();
        projection.apply(&wire(protocol, &words)?, &mut dst, CLIENT);
        assert_eq!(dst.appearance.models[0], ModelRef::native(CLIENT, wide));
        for ordinal in [None, Some(wide + 1)] {
            projection.reduce(
                &src,
                &EntityContext::default(),
                &mut words,
                |_, _| ordinal,
                |sound| Some(sound.ordinal),
                |reference| Some(reference.ordinal()),
            );
            projection.apply(&wire(protocol, &words)?, &mut dst, CLIENT);
            assert_eq!(dst.appearance.models[0], None);
        }
    }
    Ok(())
}
#[test]
fn foreign_native_flags_are_not_reinterpreted_as_other_game_flags() {
    let src = source(Protocol::Quake2Repro1038);
    for protocol in [
        Protocol::NetQuake15,
        Protocol::QuakeWorld28,
        Protocol::Quake3_68,
    ] {
        let projection = EntityProjection::load(protocol);
        let mut words = vec![0; projection.words()];
        projection.reduce(
            &src,
            &EntityContext::default(),
            &mut words,
            |model, _| Some(model.ordinal),
            |sound| Some(sound.ordinal),
            |reference| Some(reference.ordinal()),
        );
        let mut imported = EntityRecord::default();
        projection.apply(&words, &mut imported, CLIENT);
        assert_eq!(imported.appearance.native.effects, 0);
        assert_eq!(imported.appearance.native.render_flags, 0);
        assert_eq!(imported.appearance.native.event, 0);
        assert_eq!(imported.appearance.native.entity_type, 0);
        assert_eq!(
            imported.appearance.native.other_entity,
            if protocol == Protocol::Quake3_68 {
                Some(NativeEntityRef::entity(CLIENT, 0))
            } else {
                None
            }
        );
        assert_eq!(
            imported.appearance.loop_sound,
            if protocol == Protocol::Quake3_68 {
                SoundRef::native(CLIENT, 23)
            } else {
                None
            }
        );
    }
}
#[test]
fn short_records_do_not_change_output_or_imported_pose() {
    let projection = EntityProjection::load(Protocol::Quake2Repro1038);
    let mut src = source(Protocol::Quake2Repro1038);
    let saved = src;
    let mut short = [99; 3];
    assert!(!projection.reduce(
        &src,
        &EntityContext::default(),
        &mut short,
        |_, _| panic!("no model resolver on short buffer"),
        |_| panic!("no sound resolver on short buffer"),
        |reference| Some(reference.ordinal()),
    ));
    assert_eq!(short, [99; 3]);
    assert!(!projection.apply(&short, &mut src, CLIENT));
    assert_eq!(src, saved);
}

#[test]
fn rerelease_zero_sentinels_and_explicit_alpha_follow_native_pack_rules() {
    let projection = EntityProjection::load(Protocol::Quake2Repro1038);
    let mut record = source(Protocol::Quake2Repro1038);
    let mut words = [0; states::Q2_RERELEASE_ENTITY_WORDS];
    record.appearance.alpha = 0.001;
    record.appearance.scale = 1.;
    record.appearance.native.loop_volume = 1.;
    record.appearance.native.loop_attenuation = 3.;
    projection.reduce(
        &record,
        &EntityContext::default(),
        &mut words,
        |model, _| Some(model.ordinal),
        |sound| Some(sound.ordinal),
        |reference| Some(reference.ordinal()),
    );
    assert_eq!((words[21], words[22], words[23], words[24]), (1, 0, 0, 0));
    for alpha in [0.0, 1.0] {
        record.appearance.alpha = alpha;
        projection.reduce(
            &record,
            &EntityContext::default(),
            &mut words,
            |model, _| Some(model.ordinal),
            |sound| Some(sound.ordinal),
            |reference| Some(reference.ordinal()),
        );
        assert_eq!(words[21], (alpha * 255.) as u32);
    }
    let mut imported = EntityRecord::default();
    words[21] = 0;
    projection.apply(&words, &mut imported, CLIENT);
    assert_eq!(imported.appearance.alpha, 0.);
    assert_eq!(imported.appearance.scale, 0.);
    assert_eq!(imported.appearance.native.loop_volume, 0.);
    assert_eq!(imported.appearance.native.loop_attenuation, 0.);
}

#[test]
fn qw_internal_peer_velocity_never_changes_native_packetentity_bytes() -> Result<(), Error> {
    assert_eq!(states::QW_NATIVE_ENTITY_WORDS, 12);
    assert_eq!(states::QW_ENTITY_WORDS, 15);
    let projection = EntityProjection::load(Protocol::QuakeWorld28);
    let mut record = source(Protocol::QuakeWorld28);
    let mut from = [0; states::QW_ENTITY_WORDS];
    assert!(projection.reduce(
        &record,
        &EntityContext::default(),
        &mut from,
        |model, _| Some(model.ordinal),
        |sound| Some(sound.ordinal),
        |reference| Some(reference.ordinal()),
    ));
    record.velocity = Vec3([300., -400., 500.]);
    let mut to = from;
    assert!(projection.reduce(
        &record,
        &EntityContext::default(),
        &mut to,
        |model, _| Some(model.ordinal),
        |sound| Some(sound.ordinal),
        |reference| Some(reference.ordinal()),
    ));
    assert_eq!(
        &from[..states::QW_NATIVE_ENTITY_WORDS],
        &to[..states::QW_NATIVE_ENTITY_WORDS]
    );
    assert_ne!(
        &from[states::QW_NATIVE_ENTITY_WORDS..],
        &to[states::QW_NATIVE_ENTITY_WORDS..]
    );
    let mut first = [0; 1400];
    let mut second = [0; 1400];
    let baseline = [0; states::QW_ENTITY_WORDS];
    let mut a = Writer::new(&mut first, Encoding::Bytes);
    let mut b = Writer::new(&mut second, Encoding::Bytes);
    assert!(states::write_qw_entity(
        &mut a,
        17,
        &baseline,
        Some(&from),
        true
    )?);
    assert!(states::write_qw_entity(
        &mut b,
        17,
        &baseline,
        Some(&to),
        true
    )?);
    assert_eq!(a.bit_position(), b.bit_position());
    assert_eq!(a.bytes(), b.bytes());
    let mut unchanged = [0; 1400];
    let mut expected = Writer::new(&mut unchanged, Encoding::Bytes);
    let mut delta = Writer::new(&mut second, Encoding::Bytes);
    let emitted = states::write_qw_entity(&mut delta, 17, &from, Some(&to), false)?;
    assert_eq!(
        emitted,
        states::write_qw_entity(&mut expected, 17, &from, Some(&from), false)?
    );
    // SOLID can itself force the native header even without changed fields.
    assert_eq!(delta.bit_position(), expected.bit_position());
    assert_eq!(delta.bytes(), expected.bytes());
    // Playerinfo velocities are retained only in accepted CLIENT rows, never
    // manufactured from a packetentity payload that has no such fields.
    let decoded = states::read_qw_entity(&mut Reader::new(a.bytes(), Encoding::Bytes), &baseline)?
        .words
        .unwrap();
    assert_eq!(&decoded[states::QW_NATIVE_ENTITY_WORDS..], &[0; 3]);
    let mut imported = EntityRecord::default();
    assert!(projection.apply(&to, &mut imported, CLIENT));
    assert_eq!(imported.velocity, record.velocity);
    Ok(())
}

#[test]
fn loop_sounds_use_the_resolved_namespace_independently_of_source_family() -> Result<(), Error> {
    let publishing = ResourceNamespace::Module(ModuleId(19));
    for native_source in [
        Protocol::NetQuake15,
        Protocol::QuakeWorld28,
        Protocol::Quake2_34,
        Protocol::Quake2Repro1038,
        Protocol::Quake3_68,
    ] {
        for protocol in [
            Protocol::Quake2_34,
            Protocol::Quake2Repro1038,
            Protocol::Quake3_68,
        ] {
            let projection = EntityProjection::load(protocol);
            let mut record = source(native_source);
            record.appearance.loop_sound = SoundRef::native(publishing, 900);
            let mut words = vec![0; projection.words()];
            let mut resolutions = 0;
            assert!(projection.reduce(
                &record,
                &EntityContext::default(),
                &mut words,
                |model, _| Some(model.ordinal),
                |sound| {
                    assert_eq!(sound, SoundRef::native(publishing, 900).unwrap());
                    resolutions += 1;
                    Some(37)
                },
                |reference| Some(reference.ordinal()),
            ));
            assert_eq!(resolutions, 1);
            let mut imported = EntityRecord::default();
            assert!(projection.apply(&wire(protocol, &words)?, &mut imported, CLIENT));
            assert_eq!(imported.appearance.loop_sound, SoundRef::native(CLIENT, 37));
            assert_eq!(imported.appearance.models[0], ModelRef::native(CLIENT, 8));
        }
    }
    Ok(())
}

#[test]
fn loop_sound_zero_missing_and_native_width_boundaries_do_not_alias_resources() -> Result<(), Error>
{
    for protocol in [
        Protocol::Quake2_34,
        Protocol::Quake2Repro1038,
        Protocol::Quake3_68,
    ] {
        let projection = EntityProjection::load(protocol);
        let record = source(Protocol::QuakeWorld28);
        let maximum = if protocol == Protocol::Quake2Repro1038 {
            0x3fff
        } else {
            255
        };
        for resolved in [Some(maximum), Some(maximum + 1), Some(0), None] {
            let mut words = vec![0; projection.words()];
            assert!(projection.reduce(
                &record,
                &EntityContext::default(),
                &mut words,
                |model, _| Some(model.ordinal),
                |_| resolved,
                |reference| Some(reference.ordinal()),
            ));
            let mut imported = EntityRecord::default();
            imported.appearance.loop_sound = SoundRef::native(MODULE, 77);
            assert!(projection.apply(&wire(protocol, &words)?, &mut imported, CLIENT));
            assert_eq!(
                imported.appearance.loop_sound,
                if resolved == Some(maximum) {
                    SoundRef::native(CLIENT, maximum)
                } else {
                    None
                }
            );
            assert_eq!(imported.appearance.models[0], ModelRef::native(CLIENT, 8));
        }
        let mut empty = record;
        empty.appearance.loop_sound = None;
        let mut words = vec![0; projection.words()];
        assert!(projection.reduce(
            &empty,
            &EntityContext::default(),
            &mut words,
            |model, _| Some(model.ordinal),
            |_| panic!("absent sound has no ordinal to resolve"),
            |reference| Some(reference.ordinal()),
        ));
        let mut imported = record;
        assert!(projection.apply(&wire(protocol, &words)?, &mut imported, CLIENT));
        assert_eq!(imported.appearance.loop_sound, None);
    }
    Ok(())
}

#[test]
fn q3_speaker_entity_type_has_the_original_literal_delta_encoding() -> Result<(), Error> {
    // Original MSG_WriteDeltaEntity fields: entity17, remove0, delta1, lc12,
    // eleven unchanged fields, then nonzero eType7. ET_SPEAKER is7 in
    // qsrc bg_public.h, eType is an8-bit field in original msg.c.
    const SPEAKER: [u8; 5] = [0x85, 0x3a, 0x01, 0xc0, 0x3f];
    assert_eq!(states::ENTITY_LAYOUT[11], ("eType", 8));
    assert_eq!(states::ENTITY_LAYOUT[18], ("otherEntityNum", 10));
    let projection = EntityProjection::load(Protocol::Quake3_68);
    let mut record = EntityRecord::default();
    record.appearance.source = Protocol::Quake3_68.rule_set();
    record.appearance.native.entity_type = 7;
    record.appearance.presentation = Some(EntityPresentation::new(MODULE));
    let mut words = [0; states::ENTITY_WORDS];
    assert!(projection.reduce(
        &record,
        &EntityContext::default(),
        &mut words,
        |_, _| panic!("no model reference"),
        |_| panic!("no sound reference"),
        |reference| Some(reference.ordinal()),
    ));
    let mut bytes = [0; 32];
    let mut writer = Writer::new(&mut bytes, Encoding::Q3);
    assert!(states::write_q3_entity(
        &mut writer,
        17,
        &[0; states::ENTITY_WORDS],
        Some(&words),
        true
    )?);
    assert_eq!(writer.bit_position(), 38);
    assert_eq!(writer.bytes(), SPEAKER);
    let decoded = states::read_q3_entity(
        &mut Reader::new(&SPEAKER, Encoding::Q3),
        &[0; states::ENTITY_WORDS],
    )?;
    assert_eq!(decoded.number, 17);
    assert_eq!(decoded.words.unwrap(), words);
    let mut imported = EntityRecord::default();
    assert!(projection.apply(&words, &mut imported, CLIENT));
    assert_eq!(imported.appearance.native.entity_type, 7);
    assert_eq!(
        imported.appearance.native.other_entity,
        Some(NativeEntityRef::entity(CLIENT, 0))
    );
    assert_eq!(imported.appearance.loop_sound, None);
    Ok(())
}

#[test]
fn q3_event_target_is_retained_only_at_its_own_native_boundary() -> Result<(), Error> {
    let mut record = source(Protocol::Quake3_68);
    record.appearance.native.entity_type = 17;
    record.appearance.native.other_entity = Some(NativeEntityRef::entity(MODULE, 999));
    for protocol in [
        Protocol::Quake2_34,
        Protocol::Quake2Repro1038,
        Protocol::Quake3_68,
    ] {
        let projection = EntityProjection::load(protocol);
        let mut words = vec![0; projection.words()];
        assert!(projection.reduce(
            &record,
            &EntityContext::default(),
            &mut words,
            |model, _| Some(model.ordinal),
            |sound| Some(sound.ordinal),
            |reference| Some(reference.ordinal()),
        ));
        let mut imported = EntityRecord::default();
        assert!(projection.apply(&wire(protocol, &words)?, &mut imported, CLIENT));
        assert_eq!(
            (
                imported.appearance.native.entity_type,
                imported.appearance.native.other_entity
            ),
            if protocol == Protocol::Quake3_68 {
                (17, Some(NativeEntityRef::entity(CLIENT, 999)))
            } else {
                (0, None)
            }
        );
        assert_eq!(imported.appearance.loop_sound, SoundRef::native(CLIENT, 23));
    }
    Ok(())
}

fn q3_public_cells() -> [u32; states::ENTITY_WORDS] {
    // MSG field order from original qcommon/msg.c; native memory order is
    // exercised independently by compat's original-header fixture.
    let mut words = std::array::from_fn(|i| 0x8123_4500 + i as u32);
    for (i, value) in [
        (1, 1.25),
        (2, -2.5),
        (3, 3.75),
        (4, -4.125),
        (5, 5.5),
        (6, -6.75),
        (7, 7.125),
        (8, -8.5),
        (10, 10.75),
        (21, -21.25),
        (24, 24.5),
        (25, -25.75),
        (26, 26.125),
        (33, -33.5),
        (34, 34.75),
        (35, -35.125),
        (37, 37.5),
        (41, -41.75),
        (42, 42.125),
        (43, -43.5),
        (44, 44.75),
        (46, -46.125),
        (47, 47.5),
        (48, -48.75),
    ] {
        words[i] = f32::to_bits(value);
    }
    words[11] = 0; // ET_GENERAL: model/client/event cells keep their own roles.
    words[16] = 5; // TR_GRAVITY
    words[23] = 4; // TR_SINE
    words
}

#[test]
fn q3_native_all_51_cells_survive_named_projection_without_physical_pose_aliasing() {
    let projection = EntityProjection::load_native_q3();
    let words = q3_public_cells();
    let mut record = EntityRecord::default();
    assert!(projection.apply(&words, &mut record, MODULE));
    let published = record.appearance.presentation.unwrap();
    assert_eq!(published.namespace, MODULE);
    assert_eq!(
        published.trajectories.position.curve,
        TrajectoryCurve::Gravity
    );
    assert_eq!(published.trajectories.angles.curve, TrajectoryCurve::Sine);
    assert_eq!(published.pose.position, Vec3([24.5, -25.75, 26.125]));
    assert_eq!(published.pose.angles, Vec3([37.5, -21.25, -46.125]));
    assert_eq!(
        published.secondary_pose.position,
        Vec3([34.75, -35.125, -33.5])
    );
    assert_eq!(
        published.times[0],
        ThinkTime::Milliseconds(i64::from(words[38] as i32))
    );
    assert_eq!(published.light.packed(), words[49]);
    // Physical link/currentOrigin can move without changing the module's
    // published trajectory, s.origin or s.angles observation.
    record.position = Vec3([1001., 1002., 1003.]);
    record.velocity = Vec3([-11., -12., -13.]);
    record.angles = Vec3([271., 272., 273.]);
    let mut reduced = [0; states::ENTITY_WORDS];
    assert!(projection.reduce(
        &record,
        &EntityContext::default(),
        &mut reduced,
        |model, _| {
            assert_eq!(model.namespace, MODULE);
            Some(model.ordinal)
        },
        |sound| {
            assert_eq!(sound.namespace, MODULE);
            Some(sound.ordinal)
        },
        |reference| {
            assert_eq!(reference.namespace(), MODULE);
            Some(reference.ordinal())
        },
    ));
    assert_eq!(reduced, words);
    let before = record;
    assert!(!projection.apply(&words[..50], &mut record, CLIENT));
    assert_eq!(record, before);
}

#[test]
fn q3_all_named_fields_cross_the_existing_native_delta_table() -> Result<(), Error> {
    let projection = EntityProjection::load(Protocol::Quake3_68);
    let mut words = q3_public_cells();
    for (i, value) in [
        (9, 7),
        (12, 31),
        (13, 64),
        (14, 79),
        (15, 1022),
        (17, 0x71234),
        (18, 57),
        (19, 9),
        (20, 32),
        (27, 0x123456),
        (28, 0xabcd),
        (29, 7),
        (30, 1023),
        (31, 23),
        (32, 131),
        (36, 8),
        (50, 32769),
    ] {
        words[i] = value;
    }
    let mut record = EntityRecord::default();
    assert!(projection.apply(&words, &mut record, MODULE));
    let mut reduced = [0; states::ENTITY_WORDS];
    assert!(projection.reduce(
        &record,
        &EntityContext::default(),
        &mut reduced,
        |m, _| Some(m.ordinal),
        |s| Some(s.ordinal),
        |r| Some(r.ordinal())
    ));
    assert_eq!(reduced, words);
    assert_eq!(wire(Protocol::Quake3_68, &reduced)?, words);
    Ok(())
}

#[test]
fn q3_item_and_team_model_cells_keep_semantic_payload_instead_of_model_handles() {
    let projection = EntityProjection::load_native_q3();
    for (kind, expected) in [
        (
            2,
            EntityModelData::Item {
                index: 97,
                secondary: -4,
            },
        ),
        (
            12,
            EntityModelData::Team {
                team: 97,
                health: -4,
            },
        ),
    ] {
        let mut words = [0; states::ENTITY_WORDS];
        words[11] = kind;
        words[29] = 97;
        words[36] = (-4i32) as u32;
        let mut record = EntityRecord::default();
        assert!(projection.apply(&words, &mut record, MODULE));
        assert_eq!(record.appearance.models, [None; 4]);
        assert_eq!(record.appearance.presentation.unwrap().model_data, expected);
        let mut reduced = [0; states::ENTITY_WORDS];
        assert!(projection.reduce(
            &record,
            &EntityContext::default(),
            &mut reduced,
            |_, _| panic!("item/team payload is not a registered model"),
            |_| panic!("no sound"),
            |r| Some(r.ordinal())
        ));
        assert_eq!(reduced, words);
    }
}

#[test]
fn q3_general_sound_and_speaker_parameters_resolve_resources_without_fabricated_zero()
-> Result<(), Error> {
    let projection = EntityProjection::load(Protocol::Quake3_68);
    for kind in [7, 13 + 45, 13 + 46] {
        let mut words = [0; states::ENTITY_WORDS];
        words[11] = kind;
        words[13] = 23;
        words[20] = 137; // Speaker random delay and unused event scalar.
        let mut record = EntityRecord::default();
        assert!(projection.apply(&words, &mut record, MODULE));
        let p = record.appearance.presentation.unwrap();
        assert_eq!(
            p.event_parameter,
            EntityParameter::Sound(SoundRef::native(MODULE, 23))
        );
        assert_eq!(p.client, EntityClientData::Scalar(137));
        let mut reduced = [0; states::ENTITY_WORDS];
        assert!(projection.reduce(
            &record,
            &EntityContext::default(),
            &mut reduced,
            |_, _| None,
            |s| {
                assert_eq!(s, SoundRef::native(MODULE, 23).unwrap());
                Some(77)
            },
            |r| Some(r.ordinal())
        ));
        assert_eq!(reduced[13], 77);
        assert_eq!(reduced[20], 137);
        let decoded = wire(Protocol::Quake3_68, &reduced)?;
        assert!(projection.apply(&decoded, &mut record, CLIENT));
        assert_eq!(
            record.appearance.presentation.unwrap().event_parameter,
            EntityParameter::Sound(SoundRef::native(CLIENT, 77))
        );
        for missing in [None, Some(256)] {
            assert!(!projection.reduce(
                &record,
                &EntityContext::default(),
                &mut reduced,
                |_, _| None,
                |_| missing,
                |r| Some(r.ordinal())
            ));
        }
    }
    Ok(())
}

#[test]
fn q3_reference_resolver_retains_client_domain_and_native_zero_and_sentinels() {
    let projection = EntityProjection::load(Protocol::Quake3_68);
    let mut words = [0; states::ENTITY_WORDS];
    words[11] = 1; // ET_PLAYER clientNum is a CS_PLAYERS client reference.
    words[15] = 1022;
    let mut record = EntityRecord::default();
    assert!(projection.apply(&words, &mut record, MODULE));
    assert_eq!(
        record.appearance.presentation.unwrap().client,
        EntityClientData::Client(NativeClientRef::client(MODULE, 0))
    );
    let mut out = [0; states::ENTITY_WORDS];
    assert!(projection.reduce(
        &record,
        &EntityContext::default(),
        &mut out,
        |_, _| None,
        |_| None,
        |reference| {
            assert_eq!(
                reference,
                NativeReference::Client(NativeClientRef::client(MODULE, 0))
            );
            Some(9)
        }
    ));
    assert_eq!(out[15], 1022);
    assert_eq!(out[20], 9);
    assert!(!projection.reduce(
        &record,
        &EntityContext::default(),
        &mut out,
        |_, _| None,
        |_| None,
        |_| None
    ));
    words[11] = 13 + 48; // EV_BULLET_HIT_FLESH eventParm is victim entity.
    words[13] = 0;
    words[18] = 1023;
    assert!(projection.apply(&words, &mut record, MODULE));
    assert_eq!(
        record.appearance.presentation.unwrap().event_parameter,
        EntityParameter::Entity(NativeEntityRef::entity(MODULE, 0))
    );
    assert!(projection.reduce(
        &record,
        &EntityContext::default(),
        &mut out,
        |_, _| None,
        |_| None,
        |reference| {
            assert_eq!(
                reference,
                NativeReference::Entity(NativeEntityRef::entity(MODULE, 0))
            );
            Some(0)
        }
    ));
    assert_eq!(out[13], 0);
    assert_eq!(out[18], 1023);
    assert!(!projection.reduce(
        &record,
        &EntityContext::default(),
        &mut out,
        |_, _| None,
        |_| None,
        |_| Some(256)
    ));
}

#[test]
fn foreign_canonical_players_cross_q3_wire_with_destination_client_references() -> Result<(), Error>
{
    let projection = EntityProjection::load(Protocol::Quake3_68);
    for rules in [
        RuleSetId::Quake,
        RuleSetId::QuakeWorld,
        RuleSetId::Quake2,
        RuleSetId::Quake2Rerelease,
    ] {
        let record = EntityRecord {
            position: Vec3([12.375, -42.625, 48.125]),
            appearance: EntityAppearance {
                source: rules,
                // These source-native cells must not select the target role or
                // replace the destination connection's mapped client reference.
                native: NativeAppearanceTail {
                    entity_type: 7,
                    ..Default::default()
                },
                presentation: Some(EntityPresentation {
                    client: EntityClientData::Scalar(51),
                    ..EntityPresentation::new(MODULE)
                }),
                ..Default::default()
            },
            ..Default::default()
        };
        let before = record;
        for client in [0, 63] {
            let context = EntityContext {
                tags: CollisionTags::PLAYER | CollisionTags::MONSTER,
                client: Some(NativeClientRef::client(CLIENT, client)),
            };
            let mut words = [0; states::ENTITY_WORDS];
            let mut resolutions = 0;
            assert!(projection.reduce(
                &record,
                &context,
                &mut words,
                |_, _| None,
                |_| None,
                |reference| {
                    assert_eq!(
                        reference,
                        NativeReference::Client(NativeClientRef::client(CLIENT, client))
                    );
                    resolutions += 1;
                    Some(client)
                }
            ));
            assert_eq!(resolutions, 1);
            // qsrc bg_public.h: ET_GENERAL=0, ET_PLAYER=1; MSG writes both
            // eType and clientNum as eight-bit fields.
            assert_eq!(words[11], 1);
            assert_eq!(words[20], client);
            let decoded = wire(Protocol::Quake3_68, &words)?;
            let mut received = EntityRecord::default();
            assert!(projection.apply(&decoded, &mut received, CLIENT));
            assert_eq!(received.position, record.position);
            assert_eq!(received.appearance.source, RuleSetId::Quake3);
            assert_eq!(received.appearance.native.entity_type, 1);
            assert_eq!(
                received.appearance.presentation.unwrap().client,
                EntityClientData::Client(NativeClientRef::client(CLIENT, client))
            );
        }
        assert_eq!(record, before);
    }
    Ok(())
}

#[test]
fn foreign_type_projection_requires_canonical_player_role_and_allows_absent_client() {
    let projection = EntityProjection::load(Protocol::Quake3_68);
    let record = EntityRecord {
        appearance: EntityAppearance {
            source: RuleSetId::Quake2,
            native: NativeAppearanceTail {
                entity_type: 1,
                ..Default::default()
            },
            ..Default::default()
        },
        ..Default::default()
    };
    for (tags, expected) in [
        (CollisionTags::default(), 0),
        (CollisionTags::MONSTER, 0),
        (CollisionTags::DEAD_MONSTER, 0),
        (CollisionTags::PROJECTILE, 0),
        (CollisionTags::PLAYER, 1),
    ] {
        let mut words = [u32::MAX; states::ENTITY_WORDS];
        assert!(projection.reduce(
            &record,
            &EntityContext { tags, client: None },
            &mut words,
            |_, _| None,
            |_| None,
            |_| panic!("an absent client reference must not resolve")
        ));
        assert_eq!(words[11], expected);
        assert_eq!(words[20], 0);
    }
}

#[test]
fn canonical_context_never_replaces_same_source_q3_native_type_or_client_cells() -> Result<(), Error>
{
    let native = EntityProjection::load_native_q3();
    let projection = EntityProjection::load(Protocol::Quake3_68);
    let context = EntityContext {
        tags: CollisionTags::PLAYER,
        client: Some(NativeClientRef::client(CLIENT, 63)),
    };
    for (kind, client) in [(0, 0), (0x8000_0125, 0x8000_0107)] {
        let mut original = [0; states::ENTITY_WORDS];
        original[11] = kind;
        original[20] = client;
        let mut record = EntityRecord::default();
        assert!(native.apply(&original, &mut record, MODULE));
        let mut words = [0; states::ENTITY_WORDS];
        for target in [&native, &projection] {
            assert!(target.reduce(
                &record,
                &context,
                &mut words,
                |_, _| None,
                |_| None,
                |reference| match reference {
                    NativeReference::Entity(reference) => Some(reference.ordinal),
                    NativeReference::Client(_) => {
                        panic!("native scalar client data must not use context resolution")
                    }
                }
            ));
            assert_eq!(words[11], kind);
            assert_eq!(words[20], client);
        }
        let decoded = wire(Protocol::Quake3_68, &words)?;
        assert_eq!(decoded[11], kind & 255);
        assert_eq!(decoded[20], client & 255);
    }

    let mut original = [0; states::ENTITY_WORDS];
    original[11] = 1;
    original[20] = 2;
    let mut record = EntityRecord::default();
    assert!(native.apply(&original, &mut record, MODULE));
    let mut words = [0; states::ENTITY_WORDS];
    assert!(projection.reduce(
        &record,
        &context,
        &mut words,
        |_, _| None,
        |_| None,
        |reference| match reference {
            NativeReference::Entity(reference) => Some(reference.ordinal),
            NativeReference::Client(reference) => {
                assert_eq!(reference, NativeClientRef::client(MODULE, 2));
                Some(4)
            }
        }
    ));
    assert_eq!(words[11], 1);
    assert_eq!(words[20], 4);
    Ok(())
}

#[test]
fn foreign_client_projection_reuses_checked_resolver_and_native_wire_limits() {
    let record = EntityRecord {
        appearance: EntityAppearance {
            source: RuleSetId::Quake2,
            ..Default::default()
        },
        ..Default::default()
    };
    let context = EntityContext {
        tags: CollisionTags::PLAYER,
        client: Some(NativeClientRef::client(MODULE, 0)),
    };
    let projection = EntityProjection::load(Protocol::Quake3_68);
    let mut words = [0; states::ENTITY_WORDS];
    for resolved in [None, Some(64), Some(u32::MAX)] {
        assert!(!projection.reduce(
            &record,
            &context,
            &mut words,
            |_, _| None,
            |_| None,
            |reference| {
                assert_eq!(
                    reference,
                    NativeReference::Client(NativeClientRef::client(MODULE, 0))
                );
                resolved
            }
        ));
    }
    assert!(!projection.reduce(
        &record,
        &context,
        &mut words,
        |_, _| None,
        |_| None,
        |reference| (reference.namespace() == CLIENT).then_some(reference.ordinal())
    ));

    let native = EntityProjection::load_native_q3();
    assert!(native.reduce(
        &record,
        &context,
        &mut words,
        |_, _| None,
        |_| None,
        |_| Some(u32::MAX)
    ));
    assert_eq!(words[11], 1);
    assert_eq!(words[20], u32::MAX);
}

#[test]
fn packed_box_solids_cross_protocol_boundaries_in_native_units() -> Result<(), Error> {
    // Independent SV_LinkEdict/LinkEntity oracles for min[-16,-24,-24],
    // max[16,24,32]. Classic and Q3 intentionally make Y symmetric with X.
    for (source_protocol, source_word, target_protocol, target_word) in [
        (
            Protocol::Quake2_34,
            0x2062,
            Protocol::Quake2Repro1038,
            0x4018_1010,
        ),
        (
            Protocol::Quake2Repro1038,
            0x4018_1810,
            Protocol::Quake2_34,
            0x2062,
        ),
        (
            Protocol::Quake2Repro1038,
            0x4018_1810,
            Protocol::Quake3_68,
            0x0040_1810,
        ),
        (
            Protocol::Quake3_68,
            0x0040_1810,
            Protocol::Quake2Repro1038,
            0x4018_1010,
        ),
        (
            Protocol::Quake2_34,
            0x2062,
            Protocol::Quake3_68,
            0x0040_1810,
        ),
        (
            Protocol::Quake3_68,
            0x0040_1810,
            Protocol::Quake2_34,
            0x2062,
        ),
    ] {
        let mut record = source(source_protocol);
        record.appearance.native.solid = source_word;
        let target = EntityProjection::load(target_protocol);
        let mut words = vec![0; target.words()];
        assert!(target.reduce(
            &record,
            &EntityContext::default(),
            &mut words,
            |m, _| Some(m.ordinal),
            |s| Some(s.ordinal),
            |r| Some(r.ordinal())
        ));
        let mut imported = EntityRecord::default();
        assert!(target.apply(&wire(target_protocol, &words)?, &mut imported, CLIENT));
        assert_eq!(
            imported.appearance.native.solid, target_word,
            "{source_protocol:?} -> {target_protocol:?}"
        );
        assert_eq!(record.appearance.native.solid, source_word);
    }
    Ok(())
}

#[test]
fn brush_solids_require_one_resolved_primary_model_before_wire_emission() -> Result<(), Error> {
    let protocols = [
        Protocol::Quake2_34,
        Protocol::Quake2Repro1038,
        Protocol::Quake3_68,
    ];
    for publishing in protocols {
        let mut record = source(publishing);
        record.appearance.native.solid = if publishing == Protocol::Quake3_68 {
            0x00ff_ffff
        } else {
            31
        };
        for receiving in protocols {
            let projection = EntityProjection::load(receiving);
            let mut words = vec![0; projection.words()];
            let mut resolutions = [0; 4];
            assert!(projection.reduce(
                &record,
                &EntityContext::default(),
                &mut words,
                |model, _| {
                    resolutions[(model.ordinal - 8) as usize] += 1;
                    Some(model.ordinal + 2)
                },
                |s| Some(s.ordinal),
                |r| Some(r.ordinal())
            ));
            assert_eq!(
                resolutions,
                if receiving == Protocol::Quake3_68 {
                    [1, 1, 0, 0]
                } else {
                    [1; 4]
                }
            );
            let mut imported = EntityRecord::default();
            assert!(projection.apply(&wire(receiving, &words)?, &mut imported, CLIENT));
            assert_eq!(imported.appearance.models[0], ModelRef::native(CLIENT, 10));
            assert_eq!(
                imported.appearance.native.solid,
                if receiving == Protocol::Quake3_68 {
                    0x00ff_ffff
                } else {
                    31
                }
            );
            for missing in [None, Some(0), Some(65536)] {
                assert!(!projection.reduce(
                    &record,
                    &EntityContext::default(),
                    &mut words,
                    |model, _| {
                        if model.ordinal == 8 {
                            missing
                        } else {
                            Some(model.ordinal)
                        }
                    },
                    |s| Some(s.ordinal),
                    |r| Some(r.ordinal())
                ));
            }
            let mut absent = record;
            absent.appearance.models[0] = None;
            assert!(!projection.reduce(
                &absent,
                &EntityContext::default(),
                &mut words,
                |m, _| Some(m.ordinal),
                |s| Some(s.ordinal),
                |r| Some(r.ordinal())
            ));
        }
    }
    Ok(())
}

#[test]
fn native_public_solid_words_remain_exact_without_a_client_model_catalog() {
    let projection = EntityProjection::load_native_q3();
    let mut words = [0; states::ENTITY_WORDS];
    words[27] = 0x00ff_ffff;
    let mut record = EntityRecord::default();
    assert!(projection.apply(&words, &mut record, MODULE));
    let mut out = [0; states::ENTITY_WORDS];
    assert!(projection.reduce(
        &record,
        &EntityContext::default(),
        &mut out,
        |_, _| None,
        |_| None,
        |r| Some(r.ordinal())
    ));
    assert_eq!(out, words);
    words[27] = 0x8123_4567;
    assert!(projection.apply(&words, &mut record, MODULE));
    assert!(projection.reduce(
        &record,
        &EntityContext::default(),
        &mut out,
        |_, _| None,
        |_| None,
        |r| Some(r.ordinal())
    ));
    assert_eq!(out, words);
}

#[test]
fn malformed_foreign_bounds_and_box_to_brush_collisions_reject_only_the_record() {
    let projection = EntityProjection::load(Protocol::Quake3_68);
    let mut record = source(Protocol::Quake2Repro1038);
    for raw in [0x0000_0101, u32::MAX] {
        record.appearance.native.solid = raw;
        let mut words = [0; states::ENTITY_WORDS];
        assert!(!projection.reduce(
            &record,
            &EntityContext::default(),
            &mut words,
            |m, _| Some(m.ordinal),
            |s| Some(s.ordinal),
            |r| Some(r.ordinal())
        ));
        assert_eq!(record.appearance.native.solid, raw);
    }
    record.appearance.native.solid = 0;
    let mut words = [0; states::ENTITY_WORDS];
    assert!(projection.reduce(
        &record,
        &EntityContext::default(),
        &mut words,
        |m, _| Some(m.ordinal),
        |s| Some(s.ordinal),
        |r| Some(r.ordinal())
    ));
    assert_eq!(words[27], 0);
}

#[test]
fn synthetic_q3_pose_uses_load_selected_interpolation_without_changing_native_trajectories() {
    for protocol in [
        Protocol::NetQuake15,
        Protocol::QuakeWorld28,
        Protocol::Quake2_34,
        Protocol::Quake2Repro1038,
    ] {
        assert!(
            EntityProjection::load(protocol)
                .trajectory_rules()
                .is_none()
        );
    }
    let projection = EntityProjection::load(Protocol::Quake3_68);
    assert!(projection.trajectory_rules().is_some());
    let mut record = EntityRecord::default();
    record.position = Vec3([11., 12., 13.]);
    record.angles = Vec3([14., 15., 16.]);
    record.appearance.source = Protocol::Quake3_68.rule_set();
    let mut words = [0; states::ENTITY_WORDS];
    assert!(projection.reduce(
        &record,
        &EntityContext::default(),
        &mut words,
        |_, _| None,
        |_| None,
        |_| None
    ));
    assert_eq!((words[16], words[23]), (1, 1));
    assert_eq!(f32::from_bits(words[1]), 11.);
    assert_eq!(f32::from_bits(words[6]), 15.);
    record.appearance.presentation = Some(EntityPresentation::new(MODULE));
    assert!(projection.reduce(
        &record,
        &EntityContext::default(),
        &mut words,
        |_, _| None,
        |_| None,
        |_| None
    ));
    assert_eq!((words[16], words[23]), (0, 0));
}
