use qa_core::primitives::{
    EntityId, MovementMode, MovementTimer, NumericValue, PlayerState, RuleSetId, ValueBinding,
    ValueId, ValueReset, ValueWidth, Vec3,
};
use qa_network::{
    commands::{QwCmd, packet::Protocol},
    message::{Encoding, Reader, Writer},
    projection::{PlayerContext, PlayerProjection},
    states,
};

#[test]
fn nq_clientdata_sparse_values_keep_native_wire_and_raw_items()
-> Result<(), qa_network::message::Error> {
    use qa_core::primitives::{ModelRef, ModuleId, ResourceNamespace};
    let stats: Vec<_> = (0..32)
        .map(|index| ValueBinding {
            id: ValueId(index),
            width: ValueWidth::Signed32,
            reset: ValueReset::Life,
        })
        .collect();
    let items = ValueBinding {
        id: ValueId(32),
        width: ValueWidth::Signed32,
        reset: ValueReset::Life,
    };
    let projection = PlayerProjection::load_values(Protocol::NetQuake15, &stats, &[], Some(items));
    let mut source = PlayerState::with_capacity(0, 0, 33);
    source.health = -12;
    source.armor = 55;
    source.view_offset.0[2] = 22.0;
    source.view_weapon.frame = 6;
    source.view_weapon.model = ModelRef::native(ResourceNamespace::Module(ModuleId(7)), 9);
    for (index, value) in [
        (0, 777),
        (2, 999),
        (3, 17),
        (4, 888),
        (5, 999),
        (6, 42),
        (7, 0),
        (8, 255),
        (9, 1),
        (10, 999),
        (15, 0xffff_ffff),
    ] {
        assert!(stats[index].import(&mut source.values, value));
    }
    assert!(items.import(&mut source.values, 0xa801_2345));
    let context = PlayerContext {
        weapon_number: Some(32),
        ..Default::default()
    };
    let mut words = [0; states::NQ_PLAYER_WORDS];
    assert!(projection.reduce(&source, &context, &mut words, |model| Some(model.ordinal)));
    let mut bytes = [0; 64];
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    states::write_nq_player(&mut writer, &words)?;
    let length = writer.size();
    // Native svc_clientdata, flags, items, optional frame/armor/model, then
    // signed health, current ammo, four ammo counts and active weapon.
    assert_eq!(
        &bytes[..length],
        &[
            15, 0, 0x72, 0x45, 0x23, 1, 0xa8, 6, 55, 9, 0xf4, 0xff, 17, 42, 0, 255, 1, 32
        ]
    );
    let decoded =
        states::read_nq_player(&mut Reader::new(&bytes[..length], Encoding::Bytes), false)?;
    let mut destination = PlayerState::with_capacity(0, 0, 33);
    destination.movement_rules = RuleSetId::Quake3;
    destination.trace_rules = RuleSetId::Quake2;
    destination.body.position = Vec3([123.0; 3]);
    destination.view_angles = Vec3([45.0; 3]);
    stats[1].import(&mut destination.values, 19);
    stats[15].import(&mut destination.values, 77);
    assert!(projection.apply(
        &decoded,
        &mut destination,
        &mut PlayerContext::default(),
        ResourceNamespace::Client(qa_core::primitives::ClientId(3)),
        |_| None
    ));
    assert_eq!(items.export(&destination.values), Some(0xa801_2345));
    for (index, value) in [
        (0, (-12i32) as u32),
        (2, 9),
        (3, 17),
        (4, 55),
        (5, 6),
        (6, 42),
        (7, 0),
        (8, 255),
        (9, 1),
        (10, 32),
    ] {
        assert_eq!(stats[index].export(&destination.values), Some(value));
    }
    assert_eq!(stats[1].export(&destination.values), Some(19));
    assert_eq!(stats[15].export(&destination.values), Some(77));
    assert_eq!(
        (destination.movement_rules, destination.trace_rules),
        (RuleSetId::Quake3, RuleSetId::Quake2)
    );
    assert_eq!(destination.body.position, Vec3([123.0; 3]));
    assert_eq!(destination.view_angles, Vec3([45.0; 3]));
    stats[10].import(&mut source.values, 2);
    assert!(projection.reduce(
        &source,
        &PlayerContext::default(),
        &mut words,
        |model| Some(model.ordinal)
    ));
    assert_eq!(words[18], 2);
    Ok(())
}

#[test]
fn nq_clientdata_binding_capacity_rejects_before_common_fields_change() {
    let stats: Vec<_> = (0..32)
        .map(|index| ValueBinding {
            id: ValueId(index),
            width: ValueWidth::Signed32,
            reset: ValueReset::Life,
        })
        .collect();
    let items = ValueBinding {
        id: ValueId(32),
        width: ValueWidth::Signed32,
        reset: ValueReset::Life,
    };
    let projection = PlayerProjection::load_values(Protocol::NetQuake15, &stats, &[], Some(items));
    let mut target = PlayerState::with_capacity(0, 0, 32);
    target.health = 81;
    target.armor = 53;
    target.body.velocity = Vec3([12.0; 3]);
    assert!(!projection.apply(
        &[0; states::NQ_PLAYER_WORDS],
        &mut target,
        &mut PlayerContext::default(),
        qa_core::primitives::ResourceNamespace::Client(qa_core::primitives::ClientId(0)),
        |_| None
    ));
    assert_eq!((target.health, target.armor), (81, 53));
    assert_eq!(target.body.velocity, Vec3([12.0; 3]));
}

#[test]
fn numeric_stat_projection_preserves_independent_body_view_and_movement_state() {
    for protocol in [Protocol::Quake2_34, Protocol::Quake2Repro1038] {
        let mut source = player();
        source.health = 90;
        source.armor = 41;
        source.frags = -7;
        let original = PlayerProjection::load(protocol, &[]);
        let mut words = vec![0; original.words()];
        original.reduce(&source, &context(), &mut words, |model| Some(model.ordinal));
        let projection = PlayerProjection::load_stat_values(protocol, &[], &[]);
        let mut target = player();
        target.body.position = Vec3([500.0, 600.0, 700.0]);
        target.view_angles = Vec3([10.0, 20.0, 30.0]);
        target.movement.mode = MovementMode::Noclip;
        let body = target.body;
        let movement = target.movement;
        projection.apply(
            &words,
            &mut target,
            &mut context(),
            qa_core::primitives::ResourceNamespace::Client(qa_core::primitives::ClientId(0)),
            |_| None,
        );
        assert_eq!((target.health, target.armor, target.frags), (90, 41, -7));
        assert_eq!(target.body.position, body.position);
        assert_eq!(target.body.velocity, body.velocity);
        assert_eq!(target.body.mins, body.mins);
        assert_eq!(target.body.maxs, body.maxs);
        assert_eq!(target.view_angles, Vec3([10.0, 20.0, 30.0]));
        assert_eq!(target.movement, movement);
        assert_eq!(target.movement_rules, RuleSetId::Quake2);
        assert_eq!(target.trace_rules, RuleSetId::QuakeWorld);
    }
}

#[test]
fn trick_timer_uses_only_the_native_rerelease_flag_and_clears_on_receive() {
    for (protocol, expected) in [
        (Protocol::Quake2_34, 0),
        (Protocol::Quake2Repro1038, 1024),
        (Protocol::Quake3_68, 0),
    ] {
        let projection = PlayerProjection::load(protocol, &[]);
        let mut source = PlayerState::default();
        source.movement.timer = MovementTimer::TRICK;
        source.movement.remaining_ms = 64;
        let mut words = vec![0; projection.words()];
        assert!(projection.reduce(
            &source,
            &PlayerContext::default(),
            &mut words,
            |model| Some(model.ordinal)
        ));
        let flags = if protocol == Protocol::Quake3_68 {
            19
        } else {
            qa_network::projection::PM_FLAGS_WORD
        };
        assert_eq!(words[flags], expected);
        assert_eq!(projection.movement_flag_mask() & 1024, expected);
        let mut target = PlayerState::default();
        assert!(projection.apply(
            &words,
            &mut target,
            &mut PlayerContext::default(),
            qa_core::primitives::ResourceNamespace::Client(qa_core::primitives::ClientId(0)),
            |_| None
        ));
        assert_eq!(
            target.movement.timer.contains(MovementTimer::TRICK),
            expected != 0
        );
        source.movement.timer = MovementTimer::NONE;
        source.movement.remaining_ms = 0;
        assert!(projection.reduce(
            &source,
            &PlayerContext::default(),
            &mut words,
            |model| Some(model.ordinal)
        ));
        assert!(projection.apply(
            &words,
            &mut target,
            &mut PlayerContext::default(),
            qa_core::primitives::ResourceNamespace::Client(qa_core::primitives::ClientId(0)),
            |_| None
        ));
        assert_eq!(target.movement.timer, MovementTimer::NONE);
        assert_eq!(target.movement.remaining_ms, 0);
    }
}

#[test]
fn native_value_arrays_keep_hot_fields_authoritative_and_import_received_values() {
    for protocol in [
        Protocol::Quake2_34,
        Protocol::Quake2Repro1038,
        Protocol::Quake3_68,
    ] {
        let (_, spans) = states::player_value_layout(protocol);
        let mut first = 0;
        let bindings: [Vec<ValueBinding>; 2] = spans.map(|span| {
            (0..span.count)
                .map(|_| {
                    let binding = ValueBinding {
                        id: ValueId(first),
                        width: ValueWidth::Signed16,
                        reset: ValueReset::Life,
                    };
                    first += 1;
                    binding
                })
                .collect()
        });
        let projection = PlayerProjection::load_values(protocol, &bindings[0], &bindings[1], None);
        let mut source = player();
        source.values = qa_core::primitives::ValueBank::load(first as usize);
        source.health = 123;
        source.armor = 45;
        source.frags = -7;
        source.score = -9;
        for binding in bindings.iter().flatten() {
            assert!(binding.import(&mut source.values, (binding.id.0 as i32 - 300) as u32));
        }
        let mut words = vec![0; projection.words()];
        assert!(projection.reduce(&source, &context(), &mut words, |model| Some(model.ordinal)));
        let ordinary = PlayerProjection::load(protocol, &[]);
        let mut scalar = vec![0; ordinary.words()];
        ordinary.reduce(&source, &context(), &mut scalar, |model| {
            Some(model.ordinal)
        });
        let mut target = player();
        target.values = qa_core::primitives::ValueBank::load(first as usize);
        assert!(projection.apply(
            &words,
            &mut target,
            &mut context(),
            qa_core::primitives::ResourceNamespace::Client(qa_core::primitives::ClientId(0)),
            |_| None
        ));
        assert_eq!((target.health, target.armor), (123, 45));
        if protocol == Protocol::Quake3_68 {
            assert_eq!(target.score, -9);
        } else {
            assert_eq!(target.frags, -7);
        }
        for (span, bindings) in spans.into_iter().zip(&bindings) {
            for (word, binding) in span.range().zip(bindings) {
                let expected = if scalar[word] != 0 {
                    scalar[word]
                } else {
                    binding.export(&source.values).unwrap() as i16 as i32 as u32
                };
                assert_eq!(words[word], expected);
                assert_eq!(
                    binding.export(&target.values),
                    Some(u32::from(expected as u16))
                );
            }
        }
        source.health = 0;
        source.armor = 0;
        source.score = 0;
        source.frags = 0;
        assert!(projection.reduce(&source, &context(), &mut words, |model| Some(model.ordinal)));
        assert!(projection.apply(
            &words,
            &mut target,
            &mut context(),
            qa_core::primitives::ResourceNamespace::Client(qa_core::primitives::ClientId(0)),
            |_| None
        ));
        assert_eq!((target.health, target.armor), (0, 0));
        if protocol == Protocol::Quake3_68 {
            assert_eq!(target.score, 0);
        } else {
            assert_eq!(target.frags, 0);
        }
    }
}

fn context() -> PlayerContext {
    PlayerContext {
        client_number: Some(17),
        ground_number: Some(1022),
        appearance: None,
        weapon_number: Some(5),

        gravity: 800.,
        speed: 300.,
        player_info_flags: 28,
        command_age_ms: 999,
        body_yaw: 75.,
    }
}
fn player() -> PlayerState {
    let mut p = PlayerState::with_capacity(2, 2, 4);
    p.view_weapon.model = qa_core::primitives::ModelRef::native(
        qa_core::primitives::ResourceNamespace::Module(qa_core::primitives::ModuleId(1)),
        8,
    );
    p.movement_rules = RuleSetId::Quake2;
    p.trace_rules = RuleSetId::QuakeWorld;
    p.body.position = Vec3([12.3, -47.17, -0.]);
    p.body.velocity = Vec3([16.12, -32.7, 1.01]);
    p.view_angles = Vec3([15., 300., -30.]);
    p.movement.delta_angles = Vec3([90., 180., -90.]).into();
    p.view_offset = Vec3([0., 0., 22.]);
    p.health = -12;
    p.armor = 55;
    p.score = 123;
    p.frags = 7;
    p.movement.command_time_ms = 1200;
    p.movement.remaining_ms = 250;
    p.movement.ducked = true;
    p.movement.jump_held = true;
    p.movement.grounded = true;
    p.movement.timer = MovementTimer(MovementTimer::LAND.0 | MovementTimer::KNOCKBACK.0);
    p.movement.water_level = 2;
    p
}

fn destination() -> PlayerState {
    let mut p = PlayerState::with_capacity(2, 2, 4);
    p.view_weapon.model = qa_core::primitives::ModelRef::native(
        qa_core::primitives::ResourceNamespace::Module(qa_core::primitives::ModuleId(1)),
        8,
    );
    p.movement_rules = RuleSetId::Quake3;
    p.trace_rules = RuleSetId::Quake;
    p.body.position = Vec3([999.; 3]);
    p.view_angles = Vec3([-111.; 3]);
    p.health = 9876;
    p.inventory.item_counts[1] = 19;
    p.movement.tuning.max_speed = Some(17.);
    p.movement.tuning.speed_multiplier = 2.;
    p.weapon_state = Some(qa_core::primitives::WeaponState {
        source: qa_core::primitives::RuleSetId::Quake,
        namespace: qa_core::primitives::ResourceNamespace::Module(qa_core::primitives::ModuleId(1)),
        readiness: qa_core::primitives::WeaponReadiness::DeadlineSeconds(5.),
    });
    p
}

fn assert_independent_state(
    p: &PlayerState,
    weapon_state: Option<qa_core::primitives::WeaponState>,
) {
    assert_eq!(p.movement_rules, RuleSetId::Quake3);
    assert_eq!(p.trace_rules, RuleSetId::Quake);
    assert_eq!(p.inventory.item_counts[1], 19);
    assert_eq!(p.movement.tuning.max_speed, Some(17.));
    assert_eq!(p.movement.tuning.speed_multiplier, 2.);
    assert_eq!(p.weapon_state, weapon_state);
}

#[test]
fn repro_projection_preserves_float_motion_and_native_packed_fields()
-> Result<(), qa_network::message::Error> {
    let mut p = player();
    p.movement.mode = MovementMode::Noclip;
    p.movement.remaining_ms = 70_000;
    p.view_offset = Vec3([4096., -4096., 22.0625]);
    p.punch_angles = Vec3([0.03125, -0.0625, 32.]);
    p.weapon_state = None;
    p.view_weapon.frame = 65535;
    let mut native = context();
    native.client_number = Some(255);
    let projection = PlayerProjection::load(Protocol::Quake2Repro1038, &[]);
    let mut words = [0; states::Q2_REPRO_PLAYER_WORDS];
    assert!(projection.reduce(&p, &native, &mut words, |model| Some(model.ordinal)));
    assert_eq!(words[0], 2);
    assert_eq!(&words[1..4], &p.body.position.0.map(f32::to_bits));
    assert_eq!(&words[4..7], &p.body.velocity.0.map(f32::to_bits));
    assert_eq!(words[7], 65535);
    assert_eq!(
        &words[10..13],
        &p.movement.delta_angles.degrees().0.map(f32::to_bits)
    );
    assert_eq!(&words[13..16], &[32767, (-32768i32) as u32, 353]);
    assert_eq!(&words[19..22], &[32, (-64i32) as u32, 32767]);
    assert_eq!(words[23], 65535);
    assert_eq!(words[42], 255);
    assert_eq!((words[44] as i32, words[48]), (-12, 55));
    let mut source = states::Q2ReproPlayer::default();
    source.words.copy_from_slice(&words[..43]);
    source.stats.copy_from_slice(&words[43..]);
    let mut bytes = [0; 1400];
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    let flags = states::write_q2_repro_player(&mut writer, &Default::default(), &source)?;
    let decoded = states::read_q2_repro_player(
        &mut Reader::new(writer.bytes(), Encoding::Bytes),
        &Default::default(),
        flags,
    )?;
    words[..43].copy_from_slice(&decoded.words);
    words[43..].copy_from_slice(&decoded.stats);
    let mut imported = destination();
    imported.weapon_state = None;
    assert!(projection.apply(
        &words,
        &mut imported,
        &mut native,
        qa_core::primitives::ResourceNamespace::Client(qa_core::primitives::ClientId(0)),
        |_| None
    ));
    assert_eq!(
        imported.body.position.0.map(f32::to_bits),
        [
            p.body.position.0[0].to_bits(),
            p.body.position.0[1].to_bits(),
            0
        ]
    );
    assert_eq!(imported.body.velocity, p.body.velocity);
    assert_eq!(imported.movement.remaining_ms, 65535);
    assert_eq!(imported.movement.mode, MovementMode::Noclip);
    assert_eq!(native.client_number, Some(255));
    assert_eq!(imported.weapon_state, None);
    assert_eq!(imported.view_weapon.frame, 65535);
    assert_eq!(
        (imported.movement_rules, imported.trace_rules),
        (RuleSetId::Quake3, RuleSetId::Quake)
    );
    assert_eq!(imported.view_offset, Vec3([2047.9375, -2048., 22.0625]));
    assert_eq!(
        imported.punch_angles,
        Vec3([0.03125, -0.0625, 32767. / 1024.])
    );
    assert_eq!((imported.health, imported.armor), (-12, 55));
    Ok(())
}

#[test]
fn one_player_projects_into_all_native_layouts_without_changing_role_choices()
-> Result<(), qa_network::message::Error> {
    let p = player();
    let c = context();
    let mut bytes = [0; 1400];
    let nq = PlayerProjection::load(Protocol::NetQuake15, &[]);
    let mut words = [0; states::NQ_PLAYER_WORDS];
    assert!(nq.reduce(&p, &c, &mut words, |model| Some(model.ordinal)));
    assert_eq!(words[12], (-12.0f32).to_bits());
    assert_eq!(words[10], 55.0f32.to_bits());
    assert_eq!((words[19], words[20]), (1, 1));
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    states::write_nq_player(&mut writer, &words)?;
    let n = writer.size();
    let decoded = states::read_nq_player(&mut Reader::new(&bytes[..n], Encoding::Bytes), false)?;
    assert_eq!(decoded[12] as i32, -12);
    assert_eq!(decoded[10], 55);
    assert_eq!(f32::from_bits(decoded[3]), 16.);
    assert_eq!(f32::from_bits(decoded[5]), -32.);
    let mut imported = destination();
    let mut native = context();
    assert!(nq.apply(
        &decoded,
        &mut imported,
        &mut native,
        qa_core::primitives::ResourceNamespace::Client(qa_core::primitives::ClientId(0)),
        |_| { panic!("NQ clientdata has no ground ordinal") }
    ));
    assert_eq!((imported.health, imported.armor), (-12, 55));
    assert_eq!(imported.body.velocity.0[0..2], [16., -32.]);
    assert_eq!(imported.body.position, Vec3([999.; 3]));
    assert_eq!(imported.view_angles, Vec3([-111.; 3]));
    assert!(imported.movement.grounded);
    assert_eq!(imported.movement.water_level, 2);
    assert_independent_state(&imported, destination().weapon_state);

    let qw = PlayerProjection::load(Protocol::QuakeWorld28, &[]);
    let mut words = [0; states::QW_PLAYER_WORDS];
    assert!(qw.reduce(&p, &c, &mut words, |model| Some(model.ordinal)));
    let command = QwCmd {
        msec: 0,
        view_angles: Vec3::default(),
        movement: [0; 3],
        buttons: 0,
        impulse: 0,
    };
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    assert!(states::write_qw_player(&mut writer, 17, &words, command)?);
    let n = writer.size();
    let decoded =
        states::read_qw_player(&mut Reader::new(&bytes[..n], Encoding::Bytes), 0, |_| {
            command
        })?;
    assert_eq!(decoded.number, 17);
    assert_eq!(f32::from_bits(decoded.words[0]), 12.25);
    assert_eq!(f32::from_bits(decoded.words[6]), -32.);
    let mut imported = destination();
    assert!(qw.apply(
        &decoded.words,
        &mut imported,
        &mut native,
        qa_core::primitives::ResourceNamespace::Client(qa_core::primitives::ClientId(0)),
        |_| { panic!("QW playerinfo has no ground ordinal") }
    ));
    assert_eq!(imported.body.position.0[0], 12.25);
    assert_eq!(imported.body.velocity.0[1], -32.);
    assert_eq!(imported.health, 9876);
    assert_eq!(imported.view_angles, Vec3([-111.; 3]));
    assert_eq!(native.command_age_ms, 0); // PF_MSEC is absent in flags 28
    assert_eq!(native.player_info_flags, 28);
    assert_eq!(native.body_yaw, 75.); // writer-only command input
    assert_independent_state(&imported, destination().weapon_state);

    let q2 = PlayerProjection::load(Protocol::Quake2_34, &[]);
    let mut words = [0; states::Q2_PLAYER_WORDS];
    assert!(q2.reduce(&p, &c, &mut words, |model| Some(model.ordinal)));
    assert_eq!(words[1] as i32, 98);
    assert_eq!(words[2] as i32, -377);
    assert_eq!(words[7], 31);
    assert_eq!(words[8], 23);
    assert_eq!(words[11] as i32, -32768);
    assert_eq!(words[12] as i32, -16384);
    assert_eq!(words[37] as i32, -12);
    assert_eq!(words[41], 55);
    assert_eq!(words[50], 7);
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    states::write_q2_player(&mut writer, &[0; states::Q2_PLAYER_WORDS], &words)?;
    let n = writer.size();
    let decoded = states::read_q2_player(
        &mut Reader::new(&bytes[..n], Encoding::Bytes),
        &[0; states::Q2_PLAYER_WORDS],
    )?;
    assert_eq!(decoded[1..13], words[1..13]);
    assert_eq!(decoded[37], words[37]);
    assert_eq!(decoded[41], words[41]);
    assert_eq!(decoded[50], words[50]);
    let mut imported = destination();
    assert!(q2.apply(
        &decoded,
        &mut imported,
        &mut native,
        qa_core::primitives::ResourceNamespace::Client(qa_core::primitives::ClientId(0)),
        |_| { panic!("Q2 transmits ground presence, not its ordinal") }
    ));
    assert_eq!(imported.body.position.0[0..2], [12.25, -47.125]);
    assert_eq!(
        imported.movement.delta_angles.degrees(),
        Vec3([90., -180., -90.])
    );
    assert_eq!(imported.movement.remaining_ms, 248);
    assert_eq!(imported.movement.timer, MovementTimer::LAND);
    assert!(imported.movement.ducked && imported.movement.jump_held);
    assert!(imported.movement.grounded);
    assert_eq!(imported.movement.ground, None);
    assert_eq!(
        (imported.health, imported.armor, imported.frags),
        (-12, 55, 7)
    );
    assert_eq!(native.gravity, 800.);
    assert_independent_state(&imported, destination().weapon_state);

    let q3 = PlayerProjection::load(Protocol::Quake3_68, &[]);
    let mut words = [0; states::PLAYER_WORDS];
    assert!(q3.reduce(&p, &c, &mut words, |model| Some(model.ordinal)));
    assert_eq!(words[1], p.body.position.0[0].to_bits());
    assert_eq!(words[19], 99);
    assert_eq!(words[20], 1022);
    assert_eq!(words[40], 17);
    assert_eq!(words[26], 32768);
    assert_eq!(words[36], 49152);
    assert_eq!(words[48] as i32, -12);
    let mut writer = Writer::new(&mut bytes, Encoding::Q3);
    states::write_q3_player(&mut writer, &[0; states::PLAYER_WORDS], &words)?;
    let n = writer.size();
    let decoded = states::read_q3_player(
        &mut Reader::new(&bytes[..n], Encoding::Q3),
        &[0; states::PLAYER_WORDS],
    )?;
    assert_eq!(decoded[19], 99);
    assert_eq!(decoded[1], words[1]);
    assert_eq!(decoded[48] as i32, -12);
    assert_eq!(decoded[26], words[26]);
    assert_eq!(decoded[36], words[36]);
    let mut imported = destination();
    let mut ground_calls = 0;
    assert!(q3.apply(
        &decoded,
        &mut imported,
        &mut native,
        qa_core::primitives::ResourceNamespace::Client(qa_core::primitives::ClientId(0)),
        |number| {
            ground_calls += 1;
            assert_eq!(number, 1022); // native world, not a common lifetime handle
            None
        }
    ));
    assert_eq!(ground_calls, 1);
    assert_eq!(imported.body.position, p.body.position);
    assert_eq!(imported.body.velocity, p.body.velocity);
    assert_eq!(imported.view_angles, p.view_angles);
    assert_eq!(
        imported.movement.delta_angles.degrees(),
        Vec3([90., 180., 270.])
    );
    assert_eq!(imported.movement.remaining_ms, 250);
    assert_eq!(imported.movement.command_time_ms, 1200);
    assert_eq!(imported.movement.timer, p.movement.timer);
    assert!(imported.movement.grounded);
    assert_eq!(imported.movement.ground, None);
    assert_eq!(native.ground_number, Some(1022));
    assert_eq!(native.client_number, Some(17));
    assert_eq!(native.weapon_number, Some(5));
    assert_eq!(native.speed, 300.);
    assert_independent_state(
        &imported,
        Some(qa_core::primitives::WeaponState {
            source: RuleSetId::Quake3,
            namespace: qa_core::primitives::ResourceNamespace::Client(
                qa_core::primitives::ClientId(0),
            ),
            readiness: qa_core::primitives::WeaponReadiness::RemainingMilliseconds(0),
        }),
    );
    assert_eq!(p.movement_rules, RuleSetId::Quake2);
    assert_eq!(p.trace_rules, RuleSetId::QuakeWorld);
    Ok(())
}

#[test]
fn module_bindings_use_common_values_and_drop_only_unrepresentable_destinations() {
    let mut p = player();
    let binding = ValueBinding {
        id: ValueId(0),
        width: ValueWidth::Signed16,
        reset: ValueReset::Life,
    };
    assert!(binding.import(&mut p.values, 65534));
    let absent = ValueBinding {
        id: ValueId(999),
        ..binding
    };
    let projection = PlayerProjection::load(
        Protocol::Quake3_68,
        &[(48, binding), (49, absent), (1000, binding)],
    );
    assert_eq!(projection.dropped_bindings, 1);
    let mut words = [123; states::PLAYER_WORDS];
    assert!(projection.reduce(&p, &context(), &mut words, |model| Some(model.ordinal)));
    assert_eq!(words[48] as i32, -2);
    assert_eq!(words[49], 0);
    assert_eq!(p.values.get(ValueId(0)), Some(NumericValue::integer(-2)));
    let mut short = [123; 8];
    assert!(!projection.reduce(&p, &context(), &mut short, |model| Some(model.ordinal)));
    assert_eq!(short, [123; 8]);
}

#[test]
fn native_ordinals_and_modes_remain_boundary_data() {
    let mut p = player();
    let mut c = context();
    c.client_number = Some(9999);
    c.ground_number = Some(9999);
    c.weapon_number = None;
    let projection = PlayerProjection::load(Protocol::Quake3_68, &[]);
    let mut words = [0; states::PLAYER_WORDS];
    for (mode, native) in [
        (MovementMode::Walk, 0),
        (MovementMode::Fly, 1),
        (MovementMode::Noclip, 1),
        (MovementMode::Spectator, 2),
        (MovementMode::Dead, 3),
        (MovementMode::Gib, 3),
        (MovementMode::Frozen, 4),
    ] {
        p.movement.mode = mode;
        assert!(projection.reduce(&p, &c, &mut words, |model| Some(model.ordinal)));
        assert_eq!(words[34], native);
        assert_eq!(words[40], 0);
        assert_eq!(words[20], 1023);
        assert_eq!(words[41], 0);
    }
}

#[test]
fn delta_angles_keep_the_native_macro_order_and_signedness() {
    let mut p = player();
    let q2 = PlayerProjection::load(Protocol::Quake2_34, &[]);
    let q3 = PlayerProjection::load(Protocol::Quake3_68, &[]);
    let mut q2_words = [0; states::Q2_PLAYER_WORDS];
    let mut q3_words = [0; states::PLAYER_WORDS];
    let mut seed = 860u32;
    for _ in 0..4096 {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        let angle = (seed as i32 as f32) / 4096.;
        p.movement.delta_angles = Vec3([angle; 3]).into();
        assert!(q2.reduce(&p, &context(), &mut q2_words, |model| Some(model.ordinal)));
        assert!(q3.reduce(&p, &context(), &mut q3_words, |model| Some(model.ordinal)));
        let native = (angle * 65536. / 360.) as i32;
        for word in &q2_words[10..13] {
            assert_eq!(*word, native as i16 as i32 as u32);
        }
        for index in [26, 35, 36] {
            assert_eq!(q3_words[index], (native & 65535) as u32);
        }
    }
}

#[test]
fn native_q3_delta_units_keep_full_signed_words_until_the_target_boundary() {
    use qa_core::primitives::{AngleOffset, ClientId, ModuleId, ResourceNamespace};
    let native = PlayerProjection::load_native_q3(&[], &[]);
    let wire = PlayerProjection::load(Protocol::Quake3_68, &[]);
    let raw = [i32::MIN + 1, i32::MAX - 1, 65537];
    let mut source = vec![0; native.words()];
    for (word, value) in [35, 26, 36].into_iter().zip(raw) {
        source[word] = value as u32;
    }
    source[34] = 1;
    let mut target = PlayerState::default();
    target.movement_rules = RuleSetId::QuakeWorld;
    target.trace_rules = RuleSetId::Quake2;
    target.movement.mode = MovementMode::Fly;
    assert!(native.apply(
        &source,
        &mut target,
        &mut PlayerContext::default(),
        ResourceNamespace::Module(ModuleId(7)),
        |_| None
    ));
    assert_eq!(target.movement.delta_angles.0, raw.map(AngleOffset::Units));
    assert_eq!(target.movement.mode, MovementMode::Noclip);
    assert_eq!(
        (target.movement_rules, target.trace_rules),
        (RuleSetId::QuakeWorld, RuleSetId::Quake2)
    );
    let mut encoded = vec![0; native.words()];
    assert!(native.reduce(&target, &PlayerContext::default(), &mut encoded, |_| None));
    for (word, value) in [35, 26, 36].into_iter().zip(raw) {
        assert_eq!(encoded[word], value as u32);
    }
    assert!(wire.reduce(&target, &PlayerContext::default(), &mut encoded, |_| None));
    for (word, value) in [35, 26, 36].into_iter().zip(raw) {
        assert_eq!(encoded[word], value as u16 as u32);
    }
    let repro = PlayerProjection::load(Protocol::Quake2Repro1038, &[]);
    let mut floats = vec![0; repro.words()];
    for (word, bits) in
        [10, 11, 12]
            .into_iter()
            .zip([(-0.0f32).to_bits(), 0x7fc0_0123, 17.25f32.to_bits()])
    {
        floats[word] = bits;
    }
    assert!(repro.apply(
        &floats,
        &mut target,
        &mut PlayerContext::default(),
        ResourceNamespace::Client(ClientId(0)),
        |_| None
    ));
    for (angle, bits) in target.movement.delta_angles.0.into_iter().zip([
        (-0.0f32).to_bits(),
        0x7fc0_0123,
        17.25f32.to_bits(),
    ]) {
        assert!(matches!(angle, AngleOffset::Degrees(_)));
        assert_eq!(angle.degrees().to_bits(), bits);
    }
}

#[test]
fn imports_keep_lossy_modes_and_resolve_native_lifetimes_at_the_caller() {
    let q2 = PlayerProjection::load(Protocol::Quake2_34, &[]);
    let q3 = PlayerProjection::load(Protocol::Quake3_68, &[]);
    let mut p = destination();
    let mut c = context();
    let mut q2_words = [0; states::Q2_PLAYER_WORDS];
    q2_words[0] = 1;
    p.movement.mode = MovementMode::Noclip;
    p.movement.timer = MovementTimer::KNOCKBACK;
    assert!(q2.apply(
        &q2_words,
        &mut p,
        &mut c,
        qa_core::primitives::ResourceNamespace::Client(qa_core::primitives::ClientId(0)),
        |_| None
    ));
    assert_eq!(p.movement.mode, MovementMode::Noclip);
    assert_eq!(p.movement.timer, MovementTimer::KNOCKBACK); // not in Q2's flags
    p.movement.mode = MovementMode::Walk;
    assert!(q2.apply(
        &q2_words,
        &mut p,
        &mut c,
        qa_core::primitives::ResourceNamespace::Client(qa_core::primitives::ClientId(0)),
        |_| None
    ));
    assert_eq!(p.movement.mode, MovementMode::Spectator);
    p.weapon_state = None;
    q2_words[23] = 42;
    assert!(q2.apply(
        &q2_words,
        &mut p,
        &mut c,
        qa_core::primitives::ResourceNamespace::Client(qa_core::primitives::ClientId(0)),
        |_| None
    ));
    assert_eq!(p.weapon_state, None);
    assert_eq!(p.view_weapon.frame, 42);

    let mut words = [0; states::PLAYER_WORDS];
    words[34] = 3;
    words[20] = 27;
    words[19] = 32 | 64; // simultaneous landing and knockback
    let lifetime = EntityId {
        slot: 91,
        generation: 44,
    };
    p.movement.mode = MovementMode::Gib;
    assert!(q3.apply(
        &words,
        &mut p,
        &mut c,
        qa_core::primitives::ResourceNamespace::Client(qa_core::primitives::ClientId(0)),
        |number| {
            assert_eq!(number, 27);
            Some(lifetime)
        }
    ));
    assert_eq!(p.movement.mode, MovementMode::Gib);
    assert_eq!(p.movement.ground, Some(lifetime));
    assert_eq!(
        p.movement.timer.0,
        MovementTimer::LAND.0 | MovementTimer::KNOCKBACK.0
    );
    words[20] = 1023;
    words[19] = 0;
    words[8] = 77;
    p.weapon_state = None;
    for (native, mode) in [
        (3, MovementMode::Dead),
        (5, MovementMode::Frozen),
        (6, MovementMode::Frozen),
        (255, MovementMode::Frozen),
    ] {
        p.movement.mode = MovementMode::Walk;
        words[34] = native;
        assert!(q3.apply(
            &words,
            &mut p,
            &mut c,
            qa_core::primitives::ResourceNamespace::Client(qa_core::primitives::ClientId(0)),
            |_| { panic!("native none must not resolve") }
        ));
        assert_eq!(p.movement.mode, mode);
        assert!(!p.movement.grounded);
        assert_eq!(p.movement.ground, None);
        assert_eq!(c.ground_number, None);
        assert_eq!(p.movement.timer, MovementTimer::NONE);
        assert_eq!(
            p.weapon_state,
            Some(qa_core::primitives::WeaponState {
                source: RuleSetId::Quake3,
                namespace: qa_core::primitives::ResourceNamespace::Client(
                    qa_core::primitives::ClientId(0)
                ),
                readiness: qa_core::primitives::WeaponReadiness::RemainingMilliseconds(77),
            })
        );
    }
}

#[test]
fn import_bindings_use_the_one_value_bank_and_short_records_do_not_mutate() {
    let binding = ValueBinding {
        id: ValueId(2),
        width: ValueWidth::Signed16,
        reset: ValueReset::Life,
    };
    let absent = ValueBinding {
        id: ValueId(999),
        ..binding
    };
    let projection = PlayerProjection::load(
        Protocol::Quake3_68,
        &[(48, binding), (49, absent), (1000, binding)],
    );
    let mut p = destination();
    let arena = p.inventory.item_counts.as_ptr();
    let mut c = context();
    assert!(!projection.apply(
        &[7; 8],
        &mut p,
        &mut c,
        qa_core::primitives::ResourceNamespace::Client(qa_core::primitives::ClientId(0)),
        |_| panic!("short record")
    ));
    assert_eq!(p.weapon_state, destination().weapon_state);
    assert_eq!(p.health, 9876);
    assert_eq!(p.body.position, Vec3([999.; 3]));
    assert_eq!(c.ground_number, Some(1022));
    let mut words = [0; states::PLAYER_WORDS];
    words[48] = 65534;
    words[8] = (-17i32) as u32;
    words[20] = 1023;
    assert!(!projection.apply(
        &words,
        &mut p,
        &mut c,
        qa_core::primitives::ResourceNamespace::Client(qa_core::primitives::ClientId(0)),
        |_| None
    ));
    assert_eq!(p.health, 9876);
    assert_eq!(p.body.position, Vec3([999.; 3]));
    assert_eq!(p.values.get(ValueId(2)), Some(NumericValue::integer(0)));
    let projection = PlayerProjection::load(Protocol::Quake3_68, &[(48, binding), (1000, binding)]);
    assert!(projection.apply(
        &words,
        &mut p,
        &mut c,
        qa_core::primitives::ResourceNamespace::Client(qa_core::primitives::ClientId(0)),
        |_| None
    ));
    assert_eq!(p.values.get(ValueId(2)), Some(NumericValue::integer(-2)));
    assert_eq!(p.inventory.item_counts.as_ptr(), arena);
    assert_eq!(projection.dropped_bindings, 1);
    assert_independent_state(
        &p,
        Some(qa_core::primitives::WeaponState {
            source: RuleSetId::Quake3,
            namespace: qa_core::primitives::ResourceNamespace::Client(
                qa_core::primitives::ClientId(0),
            ),
            readiness: qa_core::primitives::WeaponReadiness::RemainingMilliseconds(-17),
        }),
    );
}

#[test]
fn view_weapon_uses_one_primitive_with_connection_model_resolution_and_native_packing()
-> Result<(), qa_network::message::Error> {
    use qa_core::primitives::{ClientId, ModelRef, ModuleId, ResourceNamespace, ViewWeapon};
    let source_namespace = ResourceNamespace::Module(ModuleId(9));
    let destination_namespace = ResourceNamespace::Client(ClientId(3));
    for protocol in [Protocol::Quake2_34, Protocol::Quake2Repro1038] {
        let projection = PlayerProjection::load(protocol, &[]);
        let mut p = player();
        p.view_weapon = ViewWeapon {
            model: ModelRef::native(source_namespace, 900),
            frame: 173,
            offset: Vec3([1.25, -2.5, 3.75]),
            angles: Vec3([0.5, -0.75, 1.25]),
            skin: 5,
            rate: 12,
        };
        p.weapon_state = Some(qa_core::primitives::WeaponState {
            source: qa_core::primitives::RuleSetId::Quake,
            namespace: qa_core::primitives::ResourceNamespace::Module(
                qa_core::primitives::ModuleId(1),
            ),
            readiness: qa_core::primitives::WeaponReadiness::DeadlineSeconds(99.),
        });
        let mut words = vec![0; projection.words()];
        assert!(projection.reduce(&p, &context(), &mut words, |model| {
            assert_eq!(model.namespace, source_namespace);
            assert_eq!(model.ordinal, 900);
            Some(17)
        }));
        assert_eq!(
            words[22],
            if protocol == Protocol::Quake2Repro1038 {
                17 | (5 << 13)
            } else {
                17
            }
        );
        let mut bytes = [0; 1400];
        let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
        if protocol == Protocol::Quake2_34 {
            states::write_q2_player(
                &mut writer,
                &[0; states::Q2_PLAYER_WORDS],
                words.as_slice().try_into().unwrap(),
            )?;
            words = states::read_q2_player(
                &mut Reader::new(writer.bytes(), Encoding::Bytes),
                &[0; states::Q2_PLAYER_WORDS],
            )?
            .to_vec();
        } else {
            let mut src = states::Q2ReproPlayer::default();
            src.words.copy_from_slice(&words[..43]);
            src.stats.copy_from_slice(&words[43..]);
            let flags = states::write_q2_repro_player(&mut writer, &Default::default(), &src)?;
            let read = states::read_q2_repro_player(
                &mut Reader::new(writer.bytes(), Encoding::Bytes),
                &Default::default(),
                flags,
            )?;
            words[..43].copy_from_slice(&read.words);
            words[43..].copy_from_slice(&read.stats);
        }
        let mut imported = destination();
        let previous_tail = imported.weapon_state;
        assert!(projection.apply(
            &words,
            &mut imported,
            &mut context(),
            destination_namespace,
            |_| None
        ));
        assert_eq!(
            imported.view_weapon.model,
            ModelRef::native(destination_namespace, 17)
        );
        assert_eq!(imported.view_weapon.frame, 173);
        assert_eq!(imported.view_weapon.offset, p.view_weapon.offset);
        assert_eq!(imported.view_weapon.angles, p.view_weapon.angles);
        assert_eq!(imported.weapon_state, previous_tail);
        if protocol == Protocol::Quake2Repro1038 {
            assert_eq!(imported.view_weapon.skin, 5);
            assert_eq!(imported.view_weapon.rate, 12);
        }
        p.view_weapon.model = None;
        projection.reduce(&p, &context(), &mut words, |_| {
            panic!("no native model to resolve")
        });
        projection.apply(
            &words,
            &mut imported,
            &mut context(),
            destination_namespace,
            |_| None,
        );
        assert_eq!(imported.view_weapon.model, None);
    }
    Ok(())
}

#[test]
fn targeted_view_weapon_projection_matches_full_import_without_a_stat_bank() {
    let namespace =
        qa_core::primitives::ResourceNamespace::Client(qa_core::primitives::ClientId(5));
    for protocol in [
        Protocol::NetQuake15,
        Protocol::QuakeWorld28,
        Protocol::Quake2_34,
        Protocol::Quake2Repro1038,
    ] {
        let projection = PlayerProjection::load(protocol, &[]);
        let mut source = PlayerState::default();
        source.view_weapon = qa_core::primitives::ViewWeapon {
            model: qa_core::primitives::ModelRef::native(namespace, 17),
            frame: 13,
            offset: Vec3([0.5, -1.25, 2.0]),
            angles: Vec3([2.0, -3.0, 4.0]),
            skin: 5,
            rate: 12,
        };
        let mut words = vec![0; projection.words()];
        assert!(projection.reduce(&source, &context(), &mut words, |model| Some(model.ordinal)));
        // IntegerFloat is the writer's input representation. These two native
        // readers store the decoded byte integer in the accepted player row.
        match protocol {
            Protocol::NetQuake15 => words[9] = 13,
            Protocol::QuakeWorld28 => words[11] = 13,
            _ => {}
        }
        let mut imported = PlayerState::default();
        assert!(projection.apply(&words, &mut imported, &mut context(), namespace, |_| None));
        assert_eq!(
            projection.view_weapon(&words, namespace),
            Some(imported.view_weapon)
        );
        assert_eq!(imported.view_weapon.frame, 13);
        assert!(
            projection
                .view_weapon(&words[..words.len() - 1], namespace)
                .is_none()
        );
    }
}

#[test]
fn q2_player_interpolation_rejects_only_native_discontinuities() {
    for protocol in [Protocol::Quake2_34, Protocol::Quake2Repro1038] {
        let projection = PlayerProjection::load(protocol, &[]);
        let mut from = vec![0; projection.words()];
        let mut to = from.clone();
        let position = if protocol == Protocol::Quake2_34 {
            (256.0 * 8.0) as u32
        } else {
            256.0f32.to_bits()
        };
        to[1] = position;
        assert!(projection.can_interpolate_player(&from, &to));
        to[1] = if protocol == Protocol::Quake2_34 {
            (256.125 * 8.0) as u32
        } else {
            256.125f32.to_bits()
        };
        assert!(!projection.can_interpolate_player(&from, &to));
        to[1] = 0;
        if protocol == Protocol::Quake2Repro1038 {
            to[35] = 1 << 4;
            assert!(!projection.can_interpolate_player(&from, &to));
            from[35] = 1 << 4;
            assert!(projection.can_interpolate_player(&from, &to));
            to[42] = 1;
            assert!(!projection.can_interpolate_player(&from, &to));
        }
    }
}

#[test]
fn q3_weapon_readiness_is_projected_without_a_protocol_selected_player_tail()
-> Result<(), qa_network::message::Error> {
    use qa_core::primitives::{
        ClientId, ModuleId, ResourceNamespace, WeaponReadiness, WeaponState,
    };
    let projection = PlayerProjection::load(Protocol::Quake3_68, &[]);
    let namespace = ResourceNamespace::Client(ClientId(7));
    for timer in [i32::MIN, -197_000_071, -1, 0, i32::MAX] {
        let mut server = player();
        server.weapon_state = Some(WeaponState {
            source: RuleSetId::Quake3,
            namespace: ResourceNamespace::Module(ModuleId(13)),
            readiness: WeaponReadiness::RemainingMilliseconds(timer),
        });
        let native_state = server.weapon_state;
        let mut words = [0; states::PLAYER_WORDS];
        assert!(projection.reduce(&server, &context(), &mut words, |_| None));
        assert_eq!(words[8], timer as u32);
        let mut bytes = [0; 1400];
        let mut writer = Writer::new(&mut bytes, Encoding::Q3);
        states::write_q3_player(&mut writer, &[0; states::PLAYER_WORDS], &words)?;
        let received = states::read_q3_player(
            &mut Reader::new(writer.bytes(), Encoding::Q3),
            &[0; states::PLAYER_WORDS],
        )?;
        for previous in [None, destination().weapon_state] {
            let mut client = destination();
            client.weapon_state = previous;
            assert!(projection.apply(&received, &mut client, &mut context(), namespace, |_| None));
            assert_eq!(
                client.weapon_state,
                Some(WeaponState {
                    source: RuleSetId::Quake3,
                    namespace,
                    readiness: WeaponReadiness::RemainingMilliseconds(timer as i16 as i32),
                })
            );
            assert_eq!(
                (client.movement_rules, client.trace_rules),
                (RuleSetId::Quake3, RuleSetId::Quake)
            );
        }
        assert_eq!(server.weapon_state, native_state);
    }
    let mut server = destination();
    let deadline = server.weapon_state;
    let mut words = [0; states::PLAYER_WORDS];
    assert!(projection.reduce(&server, &context(), &mut words, |_| None));
    assert_eq!(words[8], 0); // The native wire carries a duration, never a QC seconds deadline.
    assert_eq!(server.weapon_state, deadline);
    let movement = PlayerProjection::load_movement(Protocol::Quake3_68);
    words[8] = 99;
    assert!(movement.apply(&words, &mut server, &mut context(), namespace, |_| None));
    assert_eq!(server.weapon_state, deadline);
    Ok(())
}
