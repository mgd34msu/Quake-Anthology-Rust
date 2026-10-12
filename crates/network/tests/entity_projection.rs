use qa_core::primitives::{
    ClientId, EntityAppearance, ModelRef, ModuleId, NativeAppearanceTail, ResourceNamespace, Vec3,
};
use qa_network::{
    commands::packet::Protocol,
    message::{Encoding, Error, Reader, Writer},
    projection::{EntityProjection, EntityRecord},
    states,
};

const MODULE: ResourceNamespace = ResourceNamespace::Module(ModuleId(7));
const CLIENT: ResourceNamespace = ResourceNamespace::Client(ClientId(3));
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
                loop_sound: 23,
                solid: 1234,
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
        assert!(projection.reduce(&src, &mut words, |model| {
            assert_eq!(model.namespace, MODULE);
            Some(model.ordinal)
        }));
        let decoded = wire(protocol, &words)?;
        let mut imported = EntityRecord::default();
        assert!(projection.apply(&decoded, &mut imported, CLIENT));
        assert_eq!(imported.appearance.source, protocol.rule_set());
        assert_eq!(imported.appearance.models[0], ModelRef::native(CLIENT, 8));
        assert_eq!(imported.appearance.frame, 31);
        assert_eq!(imported.position, src.position);
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
        assert!(projection.reduce(&imported, &mut words, |model| {
            assert_eq!(model.namespace, CLIENT);
            Some(model.ordinal)
        }));
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
        projection.reduce(&src, &mut words, |_| Some(wide));
        let mut dst = EntityRecord::default();
        projection.apply(&wire(protocol, &words)?, &mut dst, CLIENT);
        assert_eq!(dst.appearance.models[0], ModelRef::native(CLIENT, wide));
        for ordinal in [None, Some(wide + 1)] {
            projection.reduce(&src, &mut words, |_| ordinal);
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
        projection.reduce(&src, &mut words, |model| Some(model.ordinal));
        let mut imported = EntityRecord::default();
        projection.apply(&words, &mut imported, CLIENT);
        assert_eq!(imported.appearance.native.effects, 0);
        assert_eq!(imported.appearance.native.render_flags, 0);
        assert_eq!(imported.appearance.native.event, 0);
        assert_eq!(imported.appearance.native.loop_sound, 0);
    }
}
#[test]
fn short_records_do_not_change_output_or_imported_pose() {
    let projection = EntityProjection::load(Protocol::Quake2Repro1038);
    let mut src = source(Protocol::Quake2Repro1038);
    let saved = src;
    let mut short = [99; 3];
    assert!(!projection.reduce(&src, &mut short, |_| panic!("no resolver on short buffer")));
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
    projection.reduce(&record, &mut words, |model| Some(model.ordinal));
    assert_eq!((words[21], words[22], words[23], words[24]), (1, 0, 0, 0));
    for alpha in [0.0, 1.0] {
        record.appearance.alpha = alpha;
        projection.reduce(&record, &mut words, |model| Some(model.ordinal));
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
    assert!(projection.reduce(&record, &mut from, |model| Some(model.ordinal)));
    record.velocity = Vec3([300., -400., 500.]);
    let mut to = from;
    assert!(projection.reduce(&record, &mut to, |model| Some(model.ordinal)));
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
