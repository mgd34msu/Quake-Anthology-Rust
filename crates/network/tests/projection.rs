use qa_core::primitives::{
    EntityId, MovementMode, MovementTimer, NumericValue, PlayerState, PlayerValueBindings,
    Q3StatLayout, RuleSetId, ValueBinding, ValueId, ValueReset, ValueWidth, Vec3,
};
use qa_network::{
    commands::{QwCmd, packet::Protocol},
    message::{Encoding, Reader, Writer},
    projection::{PlayerContext, PlayerProjection},
    states,
};

#[test]
fn punch_samples_use_the_same_native_width_conversion_as_full_projection() {
    use qa_core::primitives::ResourceNamespace;
    for protocol in [
        Protocol::NetQuake15,
        Protocol::Quake2_34,
        Protocol::Quake2Repro1038,
    ] {
        let projection = PlayerProjection::load(protocol, &[]);
        let source = PlayerState {
            punch_angles: Vec3([1.25, -2.5, 0.03125]),
            ..PlayerState::default()
        };
        let mut words = vec![
            0;
            match protocol {
                Protocol::NetQuake15 => states::NQ_PLAYER_WORDS,
                Protocol::Quake2_34 => states::Q2_PLAYER_WORDS,
                Protocol::Quake2Repro1038 => states::Q2_REPRO_PLAYER_WORDS,
                _ => unreachable!(),
            }
        ];
        assert!(projection.reduce(
            &source,
            &PlayerContext::default(),
            &mut words,
            |_| None,
            |_| None
        ));
        let mut accepted = PlayerState::default();
        assert!(projection.apply(
            &words,
            &mut accepted,
            &mut PlayerContext::default(),
            ResourceNamespace::Client(qa_core::primitives::ClientId(0)),
            |_| None,
        ));
        assert_eq!(
            projection.punch_angles(&words).unwrap().0.map(f32::to_bits),
            accepted.punch_angles.0.map(f32::to_bits)
        );
        assert!(projection.punch_angles(&words[..words.len() - 1]).is_none());
        assert!(
            PlayerProjection::load_movement(protocol)
                .punch_angles(&words)
                .is_none()
        );
        assert!(
            PlayerProjection::load_stat_values(
                protocol,
                PlayerValueBindings::EMPTY,
                Q3StatLayout::BASE
            )
            .punch_angles(&words)
            .is_none()
        );
    }
    for protocol in [Protocol::QuakeWorld28, Protocol::Quake3_68] {
        assert!(
            PlayerProjection::load(protocol, &[])
                .punch_angles(&[0; states::PLAYER_WORDS])
                .is_none()
        );
    }
}

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
    let projection = PlayerProjection::load_values(
        Protocol::NetQuake15,
        PlayerValueBindings {
            arrays: [&stats, &[], &[], &[]],
            items: Some(items),
        },
        Q3StatLayout::BASE,
    );
    let mut source = PlayerState::with_capacity(0, 0, 33);
    source.health = -12;
    source.armor = 55;
    source.view.accept_offset(Vec3([0.0, 0.0, 22.0]), None);
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
    assert!(projection.reduce(
        &source,
        &context,
        &mut words,
        |model| Some(model.ordinal),
        |sound| Some(sound.ordinal),
    ));
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
        |model| Some(model.ordinal),
        |sound| Some(sound.ordinal),
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
    let projection = PlayerProjection::load_values(
        Protocol::NetQuake15,
        PlayerValueBindings {
            arrays: [&stats, &[], &[], &[]],
            items: Some(items),
        },
        Q3StatLayout::BASE,
    );
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
        original.reduce(
            &source,
            &context(),
            &mut words,
            |model| Some(model.ordinal),
            |sound| Some(sound.ordinal),
        );
        let projection = PlayerProjection::load_stat_values(
            protocol,
            PlayerValueBindings::EMPTY,
            Q3StatLayout::BASE,
        );
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
            |model| Some(model.ordinal),
            |sound| Some(sound.ordinal),
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
            |model| Some(model.ordinal),
            |sound| Some(sound.ordinal),
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
        let bindings: [Vec<ValueBinding>; 4] = spans.map(|span| {
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
        let projection = PlayerProjection::load_values(
            protocol,
            PlayerValueBindings {
                arrays: bindings.each_ref().map(|array| array.as_slice()),
                items: None,
            },
            Q3StatLayout::BASE,
        );
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
        assert!(projection.reduce(
            &source,
            &context(),
            &mut words,
            |model| Some(model.ordinal),
            |sound| Some(sound.ordinal),
        ));
        let ordinary = PlayerProjection::load(protocol, &[]);
        let mut scalar = vec![0; ordinary.words()];
        ordinary.reduce(
            &source,
            &context(),
            &mut scalar,
            |model| Some(model.ordinal),
            |sound| Some(sound.ordinal),
        );
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
        assert!(projection.reduce(
            &source,
            &context(),
            &mut words,
            |model| Some(model.ordinal),
            |sound| Some(sound.ordinal),
        ));
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

#[test]
fn q3_four_arrays_preserve_native_integers_and_narrow_only_through_the_existing_wire_codec()
-> Result<(), qa_network::message::Error> {
    use qa_core::primitives::{ClientId, ModuleId, PredictionPolicy, ResourceNamespace, ValueBank};
    let (_, spans) = states::player_value_layout(Protocol::Quake3_68);
    assert_eq!(spans.map(|span| span.count), [16; 4]);
    assert_eq!(spans[0].first as usize, states::PLAYER_LAYOUT.len());
    assert_eq!(spans[3].first + spans[3].count, states::PLAYER_WORDS as u32);
    let mut next = 0;
    let arrays: [Vec<ValueBinding>; 4] = spans.map(|span| {
        span.range()
            .map(|_| {
                let binding = ValueBinding {
                    id: ValueId(next),
                    width: ValueWidth::Signed32,
                    reset: ValueReset::Life,
                };
                next += 1;
                binding
            })
            .collect()
    });
    let values = PlayerValueBindings {
        arrays: arrays.each_ref().map(|array| array.as_slice()),
        items: None,
    };
    for layout in [Q3StatLayout::BASE, Q3StatLayout::TEAM_ARENA] {
        let native = PlayerProjection::load_native_q3(values, layout);
        let wire = PlayerProjection::load_values(Protocol::Quake3_68, values, layout);
        let mut source = PlayerState::with_capacity(0, 0, next as usize);
        source.health = 41;
        source.armor = 73;
        source.score = -7;
        for (index, binding) in arrays[0].iter().enumerate() {
            binding.import(&mut source.values, 1000 + index as u32);
        }
        let mask = 0x41;
        arrays[0][layout.weapons as usize].import(&mut source.values, mask);
        for (index, bits) in [0, u32::MAX, 0x0100_0001, i32::MIN as u32, 32768, 65535]
            .into_iter()
            .enumerate()
        {
            arrays[2][index].import(&mut source.values, bits);
        }
        for (index, bits) in [0x8123_4567, u32::MAX, 0x0100_0001].into_iter().enumerate() {
            arrays[3][index].import(&mut source.values, bits);
        }
        let mut native_words = [0; states::PLAYER_WORDS];
        assert!(native.reduce(
            &source,
            &PlayerContext::default(),
            &mut native_words,
            |_| None,
            |sound| Some(sound.ordinal),
        ));
        let stats = spans[0].first as usize;
        assert_eq!(native_words[stats + layout.health as usize], 41);
        assert_eq!(native_words[stats + layout.armor as usize], 73);
        assert_eq!(native_words[stats + layout.weapons as usize], mask);
        if layout == Q3StatLayout::TEAM_ARENA {
            assert_eq!(native_words[stats + 2], 1002); // Persistant powerup is not the weapon mask.
        }
        assert_eq!(native_words[spans[2].first as usize + 2], 0x0100_0001);
        assert_eq!(native_words[spans[2].first as usize + 3], i32::MIN as u32);
        assert_eq!(native_words[spans[3].first as usize], 0x8123_4567);

        let mut published = PlayerState::with_capacity(0, 0, next as usize);
        published.movement_rules = RuleSetId::Quake2;
        published.trace_rules = RuleSetId::QuakeWorld;
        published.prediction = PredictionPolicy::DISABLED;
        published.body.position = Vec3([123.0; 3]);
        published.view_angles = Vec3([45.0; 3]);
        assert!(native.apply_stats(
            &native_words,
            &mut published,
            ResourceNamespace::Module(ModuleId(7))
        ));
        assert_eq!(published.body.position, Vec3([123.0; 3]));
        assert_eq!(published.view_angles, Vec3([45.0; 3]));
        assert_eq!(published.prediction, PredictionPolicy::DISABLED);
        assert_eq!(published.native, None);
        assert_eq!(
            (published.movement_rules, published.trace_rules),
            (RuleSetId::Quake2, RuleSetId::QuakeWorld)
        );
        assert_eq!(arrays[2][2].export(&published.values), Some(0x0100_0001));
        assert_eq!(arrays[2][1].export(&published.values), Some(u32::MAX));

        let mut outgoing = [0; states::PLAYER_WORDS];
        assert!(wire.reduce(
            &source,
            &PlayerContext::default(),
            &mut outgoing,
            |_| None,
            |sound| Some(sound.ordinal),
        ));
        let mut bytes = [0; 1400];
        let mut writer = Writer::new(&mut bytes, Encoding::Q3);
        states::write_q3_player(&mut writer, &[0; states::PLAYER_WORDS], &outgoing)?;
        let incoming = states::read_q3_player(
            &mut Reader::new(writer.bytes(), Encoding::Q3),
            &[0; states::PLAYER_WORDS],
        )?;
        let mut client = PlayerState::with_capacity(0, 0, next as usize);
        client.values = ValueBank::load(next as usize);
        client.movement_rules = RuleSetId::Quake2;
        client.trace_rules = RuleSetId::QuakeWorld;
        assert!(wire.apply(
            &incoming,
            &mut client,
            &mut PlayerContext::default(),
            ResourceNamespace::Client(ClientId(3)),
            |_| None
        ));
        for (array, span) in spans.into_iter().enumerate() {
            for (ordinal, word) in span.range().enumerate() {
                let expected = if array == 3 {
                    outgoing[word]
                } else {
                    outgoing[word] as i16 as i32 as u32
                };
                assert_eq!(incoming[word], expected);
                assert_eq!(
                    arrays[array][ordinal].export(&client.values),
                    Some(expected)
                );
            }
        }
        assert_eq!((client.health, client.armor), (41, 73));
        assert_eq!(
            arrays[0][layout.weapons as usize].export(&client.values),
            Some(mask)
        );
        assert_eq!(arrays[2][1].export(&client.values), Some(u32::MAX));
        assert_eq!(arrays[2][2].export(&client.values), Some(1));
        assert_eq!(arrays[3][0].export(&client.values), Some(0x8123_4567));
        assert_eq!(
            (client.movement_rules, client.trace_rules),
            (RuleSetId::Quake2, RuleSetId::QuakeWorld)
        );
    }
    Ok(())
}

#[test]
fn invalid_q3_stat_layout_rejects_before_raw_values_or_common_fields_change() {
    use qa_core::primitives::{ClientId, ResourceNamespace};
    let layout = Q3StatLayout {
        health: 16,
        ..Q3StatLayout::BASE
    };
    let projection = PlayerProjection::load_native_q3(PlayerValueBindings::EMPTY, layout);
    let mut source = PlayerState::default();
    source.health = 81;
    source.armor = 53;
    let mut words = [99; states::PLAYER_WORDS];
    assert!(!projection.reduce(
        &source,
        &PlayerContext::default(),
        &mut words,
        |_| None,
        |sound| Some(sound.ordinal),
    ));
    assert_eq!(words, [99; states::PLAYER_WORDS]);
    assert!(!projection.apply(
        &words,
        &mut source,
        &mut PlayerContext::default(),
        ResourceNamespace::Client(ClientId(0)),
        |_| None
    ));
    assert_eq!((source.health, source.armor), (81, 53));
    assert!(!projection.apply_stats(&words, &mut source, ResourceNamespace::Client(ClientId(0))));
    assert_eq!((source.health, source.armor), (81, 53));
}

fn context() -> PlayerContext {
    PlayerContext {
        namespace: None,
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
    p.view.accept_offset(Vec3([0., 0., 22.]), None);
    p.movement.view_height = 22.0;
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
    p.movement.tuning.max_speed = Some(17.0.into());
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
    assert_eq!(p.movement.tuning.speed_multiplier, 2.);
    assert_eq!(p.weapon_state, weapon_state);
}

#[test]
fn repro_projection_preserves_float_motion_and_native_packed_fields()
-> Result<(), qa_network::message::Error> {
    let mut p = player();
    p.movement.mode = MovementMode::Noclip;
    p.movement.remaining_ms = 70_000;
    p.view.accept_offset(Vec3([4096., -4096., 22.0625]), None);
    p.punch_angles = Vec3([0.03125, -0.0625, 32.]);
    p.weapon_state = None;
    p.view_weapon.frame = 65535;
    let mut native = context();
    native.client_number = Some(255);
    let projection = PlayerProjection::load(Protocol::Quake2Repro1038, &[]);
    let mut words = [0; states::Q2_REPRO_PLAYER_WORDS];
    assert!(projection.reduce(
        &p,
        &native,
        &mut words,
        |model| Some(model.ordinal),
        |sound| Some(sound.ordinal),
    ));
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
    assert_eq!(imported.eye_offset(), Vec3([2047.9375, -2048., 22.0625]));
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
    assert!(nq.reduce(
        &p,
        &c,
        &mut words,
        |model| Some(model.ordinal),
        |sound| Some(sound.ordinal),
    ));
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
    assert!(qw.reduce(
        &p,
        &c,
        &mut words,
        |model| Some(model.ordinal),
        |sound| Some(sound.ordinal),
    ));
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
    assert!(q2.reduce(
        &p,
        &c,
        &mut words,
        |model| Some(model.ordinal),
        |sound| Some(sound.ordinal),
    ));
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
    assert_eq!(imported.movement.tuning.max_speed, Some(17.0.into()));
    assert_independent_state(&imported, destination().weapon_state);

    let q3 = PlayerProjection::load(Protocol::Quake3_68, &[]);
    let mut words = [0; states::PLAYER_WORDS];
    assert!(q3.reduce(
        &p,
        &c,
        &mut words,
        |model| Some(model.ordinal),
        |sound| Some(sound.ordinal),
    ));
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
    assert_eq!(imported.movement.tuning.max_speed, Some(150.0.into()));
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
    assert!(projection.reduce(
        &p,
        &context(),
        &mut words,
        |model| Some(model.ordinal),
        |sound| Some(sound.ordinal),
    ));
    assert_eq!(words[48] as i32, -2);
    assert_eq!(words[49], 0);
    assert_eq!(p.values.get(ValueId(0)), Some(NumericValue::integer(-2)));
    let mut short = [123; 8];
    assert!(!projection.reduce(
        &p,
        &context(),
        &mut short,
        |model| Some(model.ordinal),
        |sound| Some(sound.ordinal),
    ));
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
        (MovementMode::Intermission, 5),
        (MovementMode::SinglePlayerIntermission, 6),
    ] {
        p.movement.mode = mode;
        assert!(projection.reduce(
            &p,
            &c,
            &mut words,
            |model| Some(model.ordinal),
            |sound| Some(sound.ordinal),
        ));
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
        assert!(q2.reduce(
            &p,
            &context(),
            &mut q2_words,
            |model| Some(model.ordinal),
            |sound| Some(sound.ordinal),
        ));
        assert!(q3.reduce(
            &p,
            &context(),
            &mut q3_words,
            |model| Some(model.ordinal),
            |sound| Some(sound.ordinal),
        ));
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
    let native = PlayerProjection::load_native_q3(PlayerValueBindings::EMPTY, Q3StatLayout::BASE);
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
    assert!(native.reduce(
        &target,
        &PlayerContext::default(),
        &mut encoded,
        |_| None,
        |sound| Some(sound.ordinal),
    ));
    for (word, value) in [35, 26, 36].into_iter().zip(raw) {
        assert_eq!(encoded[word], value as u32);
    }
    assert!(wire.reduce(
        &target,
        &PlayerContext::default(),
        &mut encoded,
        |_| None,
        |sound| Some(sound.ordinal),
    ));
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
fn q3_freeze_and_intermission_modes_survive_native_and_wire_projection()
-> Result<(), qa_network::message::Error> {
    use qa_core::primitives::{ClientId, ModuleId, PredictionPolicy, ResourceNamespace};
    let native = PlayerProjection::load_native_q3(PlayerValueBindings::EMPTY, Q3StatLayout::BASE);
    let wire = PlayerProjection::load(Protocol::Quake3_68, &[]);
    let mut words = [0; states::PLAYER_WORDS];
    let mut output = [0; states::PLAYER_WORDS];
    for (number, mode) in [
        (4, MovementMode::Frozen),
        (5, MovementMode::Intermission),
        (6, MovementMode::SinglePlayerIntermission),
    ] {
        words[34] = number;
        let mut player = PlayerState::default();
        player.movement_rules = RuleSetId::QuakeWorld;
        player.trace_rules = RuleSetId::Quake2;
        player.prediction = PredictionPolicy::DISABLED;
        assert!(native.apply(
            &words,
            &mut player,
            &mut PlayerContext::default(),
            ResourceNamespace::Module(ModuleId(7)),
            |_| None,
        ));
        assert_eq!(player.movement.mode, mode);
        assert_eq!(player.prediction, PredictionPolicy::default());
        assert!(native.reduce(
            &player,
            &PlayerContext::default(),
            &mut output,
            |_| None,
            |_| None,
        ));
        assert_eq!(output[34], number);
        assert!(wire.reduce(
            &player,
            &PlayerContext::default(),
            &mut output,
            |_| None,
            |_| None,
        ));
        let mut bytes = [0; 1024];
        let mut writer = Writer::new(&mut bytes, Encoding::Q3);
        states::write_q3_player(&mut writer, &[0; states::PLAYER_WORDS], &output)?;
        let received = states::read_q3_player(
            &mut Reader::new(writer.bytes(), Encoding::Q3),
            &[0; states::PLAYER_WORDS],
        )?;
        assert_eq!(received[34], number);
        let mut accepted = PlayerState::default();
        accepted.movement_rules = RuleSetId::Quake2Rerelease;
        accepted.trace_rules = RuleSetId::Quake;
        assert!(wire.apply(
            &received,
            &mut accepted,
            &mut PlayerContext::default(),
            ResourceNamespace::Client(ClientId(3)),
            |_| None,
        ));
        assert_eq!(accepted.movement.mode, mode);
        assert_eq!(accepted.movement_rules, RuleSetId::Quake2Rerelease);
        assert_eq!(accepted.trace_rules, RuleSetId::Quake);
        // A stats-only publication cannot release a received control lock.
        words[34] = 0;
        assert!(native.apply_stats(
            &words,
            &mut accepted,
            ResourceNamespace::Module(ModuleId(9)),
        ));
        assert_eq!(accepted.movement.mode, mode);
        assert_eq!(player.movement_rules, RuleSetId::QuakeWorld);
        assert_eq!(player.trace_rules, RuleSetId::Quake2);
    }
    Ok(())
}

#[test]
fn q2_boundaries_map_intermission_controls_to_their_existing_freeze_value() {
    use qa_core::primitives::{ClientId, ResourceNamespace};
    for (protocol, number) in [(Protocol::Quake2_34, 4), (Protocol::Quake2Repro1038, 6)] {
        let projection = PlayerProjection::load(protocol, &[]);
        let mut words = vec![0; projection.words()];
        for mode in [
            MovementMode::Intermission,
            MovementMode::SinglePlayerIntermission,
        ] {
            let mut source = PlayerState::default();
            source.movement.mode = mode;
            assert!(projection.reduce(
                &source,
                &PlayerContext::default(),
                &mut words,
                |_| None,
                |_| None,
            ));
            assert_eq!(words[0], number);
            let mut receiver = PlayerState::default();
            assert!(projection.apply(
                &words,
                &mut receiver,
                &mut PlayerContext::default(),
                ResourceNamespace::Client(ClientId(0)),
                |_| None,
            ));
            assert_eq!(receiver.movement.mode, MovementMode::Frozen);
            assert!(receiver.movement.mode.allows_view_angles());
            // A round trip through an aliased native pmove field preserves the
            // already selected common mode, as it does for Fly/Noclip/Gib.
            assert!(projection.apply(
                &words,
                &mut source,
                &mut PlayerContext::default(),
                ResourceNamespace::Client(ClientId(0)),
                |_| None,
            ));
            assert_eq!(source.movement.mode, mode);
        }
    }
}

#[test]
fn native_metadata_keeps_raw_widths_only_in_the_explicit_source_namespace()
-> Result<(), qa_network::message::Error> {
    use qa_core::primitives::{ModuleId, MovementMagnitude, NativePlayerTail, ResourceNamespace};
    let namespace = ResourceNamespace::Module(ModuleId(7));
    let native = PlayerProjection::load_native_q3(PlayerValueBindings::EMPTY, Q3StatLayout::BASE);
    let wire = PlayerProjection::load(Protocol::Quake3_68, &[]);
    assert_eq!(native.movement_flag_mask(), 0x363);
    let mut words = [0; states::PLAYER_WORDS];
    let flags = 0x8000_0000 | 8 | 16 | 512 | 1024 | 2048 | 4096 | 8192 | 16384 | 0x163;
    words[19] = flags;
    words[24] = 16_777_217;
    words[25] = (-16_777_219i32) as u32;
    words[40] = 1;
    words[41] = 65539;
    let mut player = PlayerState::default();
    player.movement_rules = RuleSetId::QuakeWorld;
    player.trace_rules = RuleSetId::Quake2;
    player.movement.tuning.gravity_multiplier = 2.0;
    player.movement.tuning.speed_multiplier = 0.5;
    player.movement.tuning.spectator_max_speed = Some(501.0);
    let mut context = PlayerContext::default();
    assert!(native.apply(&words, &mut player, &mut context, namespace, |_| None));
    assert_eq!(
        player.native,
        Some(NativePlayerTail {
            source: RuleSetId::Quake3,
            namespace,
            flags,
            client_number: Some(1),
            ground_number: Some(0),
            weapon_number: Some(65539),
            gravity: Some(16_777_217),
            speed: Some(-16_777_219),
            signals: Some(Default::default()),
            presentation: Some(Default::default()),
            view_flags: None,
        })
    );
    assert_eq!(
        player.movement.tuning.gravity,
        Some(MovementMagnitude::Base(16_777_217i32 as f32 / 2.0))
    );
    assert_eq!(
        player.movement.tuning.max_speed,
        Some(MovementMagnitude::Base(-16_777_219i32 as f32 / 0.5))
    );
    assert_eq!(player.movement.tuning.spectator_max_speed, Some(501.0));
    assert_eq!(
        (player.movement_rules, player.trace_rules),
        (RuleSetId::QuakeWorld, RuleSetId::Quake2)
    );
    player.movement.ducked = false;
    player.movement.timer = MovementTimer::LAND;
    let expected_flags = (flags & !0x163) | 2 | 32;
    let mut projected = [0; states::PLAYER_WORDS];
    assert!(native.reduce(
        &player,
        &context,
        &mut projected,
        |_| None,
        |sound| Some(sound.ordinal),
    ));
    assert_eq!(projected[19], expected_flags);
    assert_eq!(projected[24..=25], words[24..=25]);
    assert_eq!(projected[40..=41], words[40..=41]);
    let mut bytes = [0; 1400];
    assert!(wire.reduce(
        &player,
        &context,
        &mut projected,
        |_| None,
        |sound| Some(sound.ordinal),
    ));
    let mut writer = Writer::new(&mut bytes, Encoding::Q3);
    states::write_q3_player(&mut writer, &[0; states::PLAYER_WORDS], &projected)?;
    let length = writer.size();
    let decoded = states::read_q3_player(
        &mut Reader::new(&bytes[..length], Encoding::Q3),
        &[0; states::PLAYER_WORDS],
    )?;
    assert_eq!(decoded[19], expected_flags & 65535);
    assert_eq!(decoded[24], 1);
    assert_eq!(decoded[25], (-16_777_219i32) as u16 as u32);
    assert_eq!((decoded[40], decoded[41]), (1, 3));

    // Explicitly selecting another namespace drops residual/follow metadata.
    context.namespace = Some(ResourceNamespace::Module(ModuleId(8)));
    context.client_number = Some(5);
    context.weapon_number = Some(7);
    context.gravity = 123.0;
    context.speed = 456.0;
    assert!(wire.reduce(
        &player,
        &context,
        &mut projected,
        |_| None,
        |sound| Some(sound.ordinal),
    ));
    // Respawn suppression is now common movement state, like jump/timers;
    // changing the resource namespace drops only the residual native bits.
    assert_eq!(projected[19], 2 | 32 | 512);
    assert_eq!(projected[24..=25], [123, 456]);
    assert_eq!(projected[40..=41], [5, 7]);
    // Matching namespace never makes foreign raw flag bits meaningful.
    context.namespace = Some(namespace);
    let q2 = PlayerProjection::load(Protocol::Quake2_34, &[]);
    let mut q2_words = [0; states::Q2_PLAYER_WORDS];
    assert!(q2.reduce(
        &player,
        &context,
        &mut q2_words,
        |_| None,
        |sound| Some(sound.ordinal),
    ));
    assert_eq!(q2_words[8], 2 | 4 | 16);
    assert_eq!(q2_words[9], 123);

    let unchanged_tail = player.native;
    let unchanged_tuning = player.movement.tuning;
    words[19] = 0;
    words[24] = 0;
    words[25] = 0;
    words[40] = 0;
    words[41] = 0;
    assert!(native.apply_stats(&words, &mut player, ResourceNamespace::Module(ModuleId(8))));
    assert_eq!(player.native, unchanged_tail);
    assert_eq!(player.movement.tuning, unchanged_tuning);
    Ok(())
}

#[test]
fn accepted_native_tuning_with_zero_multipliers_remains_effective() {
    use qa_core::primitives::{ClientId, MovementMagnitude, ResourceNamespace};
    let projection = PlayerProjection::load(Protocol::Quake3_68, &[]);
    let mut words = [0; states::PLAYER_WORDS];
    words[24] = 1200;
    words[25] = 250;
    let mut player = PlayerState::default();
    player.movement.tuning.gravity_multiplier = 0.0;
    player.movement.tuning.speed_multiplier = 0.0;
    let mut context = PlayerContext::default();
    assert!(projection.apply(
        &words,
        &mut player,
        &mut context,
        ResourceNamespace::Client(ClientId(0)),
        |_| None
    ));
    assert_eq!(
        player.movement.tuning.gravity,
        Some(MovementMagnitude::Effective(1200.0))
    );
    assert_eq!(
        player.movement.tuning.max_speed,
        Some(MovementMagnitude::Effective(250.0))
    );
    assert_eq!(
        player.movement.tuning.gravity.unwrap().effective(0.0),
        1200.0
    );
    assert_eq!(
        player.movement.tuning.max_speed.unwrap().effective(0.0),
        250.0
    );
    assert_eq!(player.native.unwrap().client_number, Some(0));
    assert_eq!(player.native.unwrap().weapon_number, Some(0));
}

#[test]
fn native_world_ground_survives_until_airborne_or_an_explicit_mapping_replaces_it() {
    use qa_core::primitives::{ModuleId, ResourceNamespace};
    let namespace = ResourceNamespace::Module(ModuleId(7));
    let projection =
        PlayerProjection::load_native_q3(PlayerValueBindings::EMPTY, Q3StatLayout::BASE);
    let mut words = [0; states::PLAYER_WORDS];
    words[20] = 1022;
    let mut player = PlayerState::default();
    assert!(projection.apply(
        &words,
        &mut player,
        &mut PlayerContext::default(),
        namespace,
        |_| None
    ));
    assert!(player.movement.grounded);
    assert_eq!(player.movement.ground, None);
    assert_eq!(player.native.unwrap().ground_number, Some(1022));
    let mut context = PlayerContext {
        namespace: Some(namespace),
        ..Default::default()
    };
    let mut output = [0; states::PLAYER_WORDS];
    assert!(projection.reduce(
        &player,
        &context,
        &mut output,
        |_| None,
        |sound| Some(sound.ordinal),
    ));
    assert_eq!(output[20], 1022);
    player.movement.grounded = false;
    assert!(projection.reduce(
        &player,
        &context,
        &mut output,
        |_| None,
        |sound| Some(sound.ordinal),
    ));
    assert_eq!(output[20], 1023);
    context.ground_number = Some(9);
    assert!(projection.reduce(
        &player,
        &context,
        &mut output,
        |_| None,
        |sound| Some(sound.ordinal),
    ));
    assert_eq!(output[20], 9); // An explicit boundary mapping has precedence.
    player.movement.grounded = true;
    player.movement.ground = Some(EntityId {
        slot: 91,
        generation: 4,
    });
    assert!(projection.reduce(
        &player,
        &context,
        &mut output,
        |_| None,
        |sound| Some(sound.ordinal),
    ));
    assert_eq!(output[20], 9);
    context.ground_number = None;
    assert!(projection.reduce(
        &player,
        &context,
        &mut output,
        |_| None,
        |sound| Some(sound.ordinal),
    ));
    assert_eq!(output[20], 1023); // Unrepresentable common ground cannot use stale raw ground.
    player.movement.ground = None;
    context.namespace = Some(ResourceNamespace::Module(ModuleId(8)));
    assert!(projection.reduce(
        &player,
        &context,
        &mut output,
        |_| None,
        |sound| Some(sound.ordinal),
    ));
    assert_eq!(output[20], 1023);
    context.namespace = Some(namespace);
    player.native.as_mut().unwrap().source = RuleSetId::Quake2;
    assert!(projection.reduce(
        &player,
        &context,
        &mut output,
        |_| None,
        |sound| Some(sound.ordinal),
    ));
    assert_eq!(output[20], 1023);
}

#[test]
fn accepted_follow_capability_and_client_target_use_the_existing_field_table() {
    use qa_core::primitives::{ClientId, PredictionPolicy, ResourceNamespace};
    let namespace = ResourceNamespace::Client(ClientId(0));
    let projection = PlayerProjection::load(Protocol::Quake3_68, &[]);
    let mut words = [0; states::PLAYER_WORDS];
    let mut player = PlayerState::default();
    player.movement_rules = RuleSetId::QuakeWorld;
    for number in [0, 1, 63] {
        words[40] = number;
        assert_eq!(projection.client_number(&words), Some(number));
    }
    words[40] = 64;
    assert_eq!(projection.client_number(&words), None);
    assert_eq!(projection.client_number(&words[..40]), None);
    words[19] = 4096;
    assert!(projection.apply(
        &words,
        &mut player,
        &mut PlayerContext::default(),
        namespace,
        |_| None
    ));
    assert_eq!(player.prediction, PredictionPolicy::DISABLED);
    words[19] = 0;
    assert!(projection.apply_stats(&words, &mut player, namespace));
    assert_eq!(player.prediction, PredictionPolicy::DISABLED);
    assert!(projection.apply(
        &words,
        &mut player,
        &mut PlayerContext::default(),
        namespace,
        |_| None
    ));
    assert_eq!(player.prediction, PredictionPolicy::default());
    for protocol in [
        Protocol::NetQuake15,
        Protocol::QuakeWorld28,
        Protocol::Quake2_34,
        Protocol::Quake2Repro1038,
    ] {
        let other = PlayerProjection::load(protocol, &[]);
        let mut foreign = vec![0; other.words()];
        if matches!(protocol, Protocol::Quake2_34 | Protocol::Quake2Repro1038) {
            foreign[8] = 64 | 256 | 4096;
        }
        player.prediction = PredictionPolicy::DISABLED;
        assert!(other.apply(
            &foreign,
            &mut player,
            &mut PlayerContext::default(),
            namespace,
            |_| None
        ));
        assert_eq!(
            player.prediction,
            match protocol {
                Protocol::Quake2_34 => PredictionPolicy {
                    position: false,
                    angles: true
                },
                Protocol::Quake2Repro1038 => PredictionPolicy::DISABLED,
                _ => PredictionPolicy::default(),
            }
        );
        assert_eq!(player.movement_rules, RuleSetId::QuakeWorld);
    }
    let repro = PlayerProjection::load(Protocol::Quake2Repro1038, &[]);
    let mut repro_words = vec![0; repro.words()];
    repro_words[42] = 255;
    assert_eq!(repro.client_number(&repro_words), Some(255));
    repro_words[42] = 256;
    assert_eq!(repro.client_number(&repro_words), None);
}

#[test]
fn native_prediction_flags_keep_independent_capabilities_and_exact_wire_words()
-> Result<(), qa_network::message::Error> {
    use qa_core::primitives::{ClientId, PredictionPolicy, ResourceNamespace};
    let namespace = ResourceNamespace::Client(ClientId(3));
    for (protocol, cases) in [
        (Protocol::Quake2_34, &[0, 64, 65][..]),
        (Protocol::Quake2Repro1038, &[0, 64, 256, 320, 321][..]),
        (Protocol::Quake3_68, &[0, 64, 256, 4096, 4097][..]),
    ] {
        let projection = PlayerProjection::load(protocol, &[]);
        let flag_word = if protocol == Protocol::Quake3_68 {
            19
        } else {
            8
        };
        for &flags in cases {
            let mut words = vec![0; projection.words()];
            words[flag_word] = flags;
            let mut bytes = [0; 1400];
            let received = match protocol {
                Protocol::Quake2_34 => {
                    let native: [u32; states::Q2_PLAYER_WORDS] =
                        words.as_slice().try_into().unwrap();
                    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
                    states::write_q2_player(&mut writer, &[0; states::Q2_PLAYER_WORDS], &native)?;
                    states::read_q2_player(
                        &mut Reader::new(writer.bytes(), Encoding::Bytes),
                        &[0; states::Q2_PLAYER_WORDS],
                    )?
                    .to_vec()
                }
                Protocol::Quake2Repro1038 => {
                    let mut native = states::Q2ReproPlayer::default();
                    let scalars = native.words.len();
                    native.words.copy_from_slice(&words[..scalars]);
                    native.stats.copy_from_slice(&words[scalars..]);
                    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
                    let extra =
                        states::write_q2_repro_player(&mut writer, &Default::default(), &native)?;
                    let decoded = states::read_q2_repro_player(
                        &mut Reader::new(writer.bytes(), Encoding::Bytes),
                        &Default::default(),
                        extra,
                    )?;
                    decoded.words.into_iter().chain(decoded.stats).collect()
                }
                Protocol::Quake3_68 => {
                    let native: [u32; states::PLAYER_WORDS] = words.as_slice().try_into().unwrap();
                    let mut writer = Writer::new(&mut bytes, Encoding::Q3);
                    states::write_q3_player(&mut writer, &[0; states::PLAYER_WORDS], &native)?;
                    states::read_q3_player(
                        &mut Reader::new(writer.bytes(), Encoding::Q3),
                        &[0; states::PLAYER_WORDS],
                    )?
                    .to_vec()
                }
                _ => unreachable!(),
            };
            assert_eq!(received[flag_word], flags);
            let expected = match protocol {
                Protocol::Quake2_34 => PredictionPolicy {
                    position: flags & 64 == 0,
                    angles: true,
                },
                Protocol::Quake2Repro1038 => PredictionPolicy {
                    position: flags & 64 == 0,
                    angles: flags & 256 == 0,
                },
                Protocol::Quake3_68 => PredictionPolicy {
                    position: flags & 4096 == 0,
                    angles: flags & 4096 == 0,
                },
                _ => unreachable!(),
            };
            let mut player = PlayerState::default();
            player.movement_rules = RuleSetId::QuakeWorld;
            player.trace_rules = RuleSetId::Quake2;
            player.prediction = PredictionPolicy::DISABLED;
            let mut context = PlayerContext::default();
            assert!(projection.apply(&received, &mut player, &mut context, namespace, |_| None));
            assert_eq!(player.prediction, expected);
            assert_eq!(
                (player.movement_rules, player.trace_rules),
                (RuleSetId::QuakeWorld, RuleSetId::Quake2)
            );
            assert_eq!(player.native.unwrap().flags, flags);
            let mut projected = vec![0; projection.words()];
            assert!(projection.reduce(
                &player,
                &context,
                &mut projected,
                |_| None,
                |sound| Some(sound.ordinal),
            ));
            assert_eq!(projected[flag_word], flags);
            words.copy_from_slice(&received);
            words[flag_word] = 0;
            assert!(projection.apply_stats(&words, &mut player, namespace));
            assert_eq!(player.prediction, expected);
            let stats = PlayerProjection::load_stat_values(
                protocol,
                PlayerValueBindings::EMPTY,
                Q3StatLayout::BASE,
            );
            assert!(stats.apply(&words, &mut player, &mut context, namespace, |_| None));
            assert_eq!(player.prediction, expected);
            assert!(projection.apply(&words, &mut player, &mut context, namespace, |_| None));
            assert_eq!(player.prediction, PredictionPolicy::default());
        }
    }
    Ok(())
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
        (5, MovementMode::Intermission),
        (6, MovementMode::SinglePlayerIntermission),
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
        assert!(projection.reduce(
            &p,
            &context(),
            &mut words,
            |model| {
                assert_eq!(model.namespace, source_namespace);
                assert_eq!(model.ordinal, 900);
                Some(17)
            },
            |sound| Some(sound.ordinal),
        ));
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
        projection.reduce(
            &p,
            &context(),
            &mut words,
            |_| panic!("no native model to resolve"),
            |sound| Some(sound.ordinal),
        );
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
        assert!(projection.reduce(
            &source,
            &context(),
            &mut words,
            |model| Some(model.ordinal),
            |sound| Some(sound.ordinal),
        ));
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
        assert!(projection.reduce(
            &server,
            &context(),
            &mut words,
            |_| None,
            |sound| Some(sound.ordinal),
        ));
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
    assert!(projection.reduce(
        &server,
        &context(),
        &mut words,
        |_| None,
        |sound| Some(sound.ordinal),
    ));
    assert_eq!(words[8], 0); // The native wire carries a duration, never a QC seconds deadline.
    assert_eq!(server.weapon_state, deadline);
    let movement = PlayerProjection::load_movement(Protocol::Quake3_68);
    words[8] = 99;
    assert!(movement.apply(&words, &mut server, &mut context(), namespace, |_| None));
    assert_eq!(server.weapon_state, deadline);
    Ok(())
}

#[test]
fn q3_player_loop_retains_native_words_in_the_explicit_namespace_and_zero_clears() {
    use qa_core::primitives::{ClientId, ModuleId, ResourceNamespace, SoundRef};
    let projection =
        PlayerProjection::load_native_q3(PlayerValueBindings::EMPTY, Q3StatLayout::BASE);
    for namespace in [
        ResourceNamespace::Module(ModuleId(7)),
        ResourceNamespace::Client(ClientId(3)),
    ] {
        let mut player = PlayerState::default();
        player.movement_rules = RuleSetId::QuakeWorld;
        player.trace_rules = RuleSetId::Quake2;
        let mut context = PlayerContext::default();
        for ordinal in [1, 65_535, 65_536, i32::MAX as u32, u32::MAX, 0] {
            let mut words = [0; states::PLAYER_WORDS];
            words[47] = ordinal;
            assert!(projection.apply(&words, &mut player, &mut context, namespace, |_| None));
            assert_eq!(player.loop_sound, SoundRef::native(namespace, ordinal));
            assert_eq!(context.namespace, Some(namespace));
            let mut outgoing = [0; states::PLAYER_WORDS];
            assert!(projection.reduce(
                &player,
                &context,
                &mut outgoing,
                |_| None,
                |sound| Some(sound.ordinal),
            ));
            assert_eq!(outgoing[47], ordinal);
            assert_eq!(
                (player.movement_rules, player.trace_rules),
                (RuleSetId::QuakeWorld, RuleSetId::Quake2)
            );
        }
    }
}

#[test]
fn q3_player_loop_native_words_narrow_only_at_the_queued_wire_boundary()
-> Result<(), qa_network::message::Error> {
    use qa_core::primitives::{ClientId, ModuleId, ResourceNamespace, SoundRef};
    let native = PlayerProjection::load_native_q3(PlayerValueBindings::EMPTY, Q3StatLayout::BASE);
    let wire = PlayerProjection::load(Protocol::Quake3_68, &[]);
    let module = ResourceNamespace::Module(ModuleId(7));
    let client = ResourceNamespace::Client(ClientId(3));
    for ordinal in [1, 65_535, 65_536, 65_537, i32::MAX as u32, u32::MAX, 0] {
        let mut original = [0; states::PLAYER_WORDS];
        original[47] = ordinal;
        let mut server = PlayerState::default();
        let mut context = PlayerContext::default();
        assert!(native.apply(&original, &mut server, &mut context, module, |_| None));
        let retained = server.loop_sound;
        let mut outgoing = [0; states::PLAYER_WORDS];
        assert!(wire.reduce(
            &server,
            &context,
            &mut outgoing,
            |_| None,
            |sound| Some(sound.ordinal),
        ));
        assert_eq!(outgoing[47], ordinal);
        let mut bytes = [0; 1400];
        let mut writer = Writer::new(&mut bytes, Encoding::Q3);
        states::write_q3_player(&mut writer, &[0; states::PLAYER_WORDS], &outgoing)?;
        let received = states::read_q3_player(
            &mut Reader::new(writer.bytes(), Encoding::Q3),
            &[0; states::PLAYER_WORDS],
        )?;
        assert_eq!(received[47], ordinal as u16 as u32);
        let mut destination = PlayerState::default();
        destination.loop_sound = SoundRef::native(client, 17);
        assert!(wire.apply(
            &received,
            &mut destination,
            &mut PlayerContext::default(),
            client,
            |_| None
        ));
        assert_eq!(
            destination.loop_sound,
            SoundRef::native(client, ordinal as u16 as u32)
        );
        assert_eq!(server.loop_sound, retained);
    }
    Ok(())
}

#[test]
fn q3_player_loop_export_drops_unselected_module_and_client_namespaces() {
    use qa_core::primitives::{ClientId, ModuleId, ResourceNamespace, SoundRef};
    let projection =
        PlayerProjection::load_native_q3(PlayerValueBindings::EMPTY, Q3StatLayout::BASE);
    let namespace = ResourceNamespace::Module(ModuleId(7));
    let mut source = PlayerState::default();
    source.loop_sound = SoundRef::native(namespace, u32::MAX);
    for selected in [
        None,
        Some(ResourceNamespace::Module(ModuleId(8))),
        Some(ResourceNamespace::Client(ClientId(7))),
    ] {
        let mut words = [0; states::PLAYER_WORDS];
        words[47] = 17;
        let context = PlayerContext {
            namespace: selected,
            ..Default::default()
        };
        assert!(projection.reduce(&source, &context, &mut words, |_| None, |_| None,));
        assert_eq!(words[47], 0);
        assert_eq!(source.loop_sound, SoundRef::native(namespace, u32::MAX));
    }
    source.loop_sound = SoundRef::native(ResourceNamespace::Client(ClientId(7)), 19);
    let mut words = [0; states::PLAYER_WORDS];
    assert!(projection.reduce(
        &source,
        &PlayerContext {
            namespace: Some(ResourceNamespace::Client(ClientId(8))),
            ..Default::default()
        },
        &mut words,
        |_| None,
        |_| None,
    ));
    assert_eq!(words[47], 0);
}

#[test]
fn stats_and_movement_only_projection_preserve_the_accepted_player_loop() {
    use qa_core::primitives::{ClientId, ModuleId, ResourceNamespace, SoundRef};
    let client = ResourceNamespace::Client(ClientId(3));
    let module = ResourceNamespace::Module(ModuleId(7));
    let native = PlayerProjection::load_native_q3(PlayerValueBindings::EMPTY, Q3StatLayout::BASE);
    let stats = PlayerProjection::load_stat_values(
        Protocol::Quake3_68,
        PlayerValueBindings::EMPTY,
        Q3StatLayout::BASE,
    );
    let movement = PlayerProjection::load_movement(Protocol::Quake3_68);
    let mut player = PlayerState::default();
    player.loop_sound = SoundRef::native(client, 17);
    for ordinal in [0, 9, u32::MAX] {
        let mut words = [0; states::PLAYER_WORDS];
        words[47] = ordinal;
        assert!(native.apply_stats(&words, &mut player, module));
        assert_eq!(player.loop_sound, SoundRef::native(client, 17));
        assert!(stats.apply(
            &words,
            &mut player,
            &mut PlayerContext::default(),
            module,
            |_| None
        ));
        assert_eq!(player.loop_sound, SoundRef::native(client, 17));
        assert!(movement.apply(
            &words,
            &mut player,
            &mut PlayerContext::default(),
            module,
            |_| None
        ));
        assert_eq!(player.loop_sound, SoundRef::native(client, 17));
    }
    assert!(!native.apply(
        &[0; 47],
        &mut player,
        &mut PlayerContext::default(),
        module,
        |_| None
    ));
    assert_eq!(player.loop_sound, SoundRef::native(client, 17));
}

#[test]
fn respawn_lock_is_common_state_with_protocol_exact_residual_flags()
-> Result<(), qa_network::message::Error> {
    use qa_core::primitives::{ClientId, ModuleId, ResourceNamespace};
    let namespace = ResourceNamespace::Module(ModuleId(7));
    let native = PlayerProjection::load_native_q3(PlayerValueBindings::EMPTY, Q3StatLayout::BASE);
    let wire = PlayerProjection::load(Protocol::Quake3_68, &[]);
    let mut words = [0; states::PLAYER_WORDS];
    words[19] = 0x8000_0000 | 4096 | 512 | 8;
    let mut player = PlayerState {
        movement_rules: RuleSetId::Quake2,
        trace_rules: RuleSetId::QuakeWorld,
        ..Default::default()
    };
    let mut context = PlayerContext::default();
    assert!(native.apply(&words, &mut player, &mut context, namespace, |_| None));
    assert!(player.movement.respawned);
    assert_eq!(player.native.unwrap().flags, words[19]);
    assert_eq!(
        (player.movement_rules, player.trace_rules),
        (RuleSetId::Quake2, RuleSetId::QuakeWorld)
    );

    // Stats imports and rejected memory admission cannot clear a movement lock.
    let stats = PlayerProjection::load_stat_values(
        Protocol::Quake2_34,
        PlayerValueBindings::EMPTY,
        Q3StatLayout::BASE,
    );
    assert!(stats.apply(
        &[0; states::Q2_PLAYER_WORDS],
        &mut player,
        &mut context,
        namespace,
        |_| None,
    ));
    assert!(player.movement.respawned);
    assert!(native.apply_stats(&[0; states::PLAYER_WORDS], &mut player, namespace));
    assert!(player.movement.respawned);
    assert!(!native.apply(&words[..1], &mut player, &mut context, namespace, |_| None));
    assert!(player.movement.respawned);

    // Shared movement owns changes to the mapped flag. A retained raw tail
    // must not resurrect it when projecting back to native or wire state.
    player.movement.respawned = false;
    let mut projected = [0; states::PLAYER_WORDS];
    assert!(native.reduce(&player, &context, &mut projected, |_| None, |_| None));
    assert_eq!(projected[19], words[19] & !512);
    player.movement.respawned = true;
    assert!(wire.reduce(&player, &context, &mut projected, |_| None, |_| None));
    let mut bytes = [0; 1400];
    let mut writer = Writer::new(&mut bytes, Encoding::Q3);
    states::write_q3_player(&mut writer, &[0; states::PLAYER_WORDS], &projected)?;
    let size = writer.size();
    let received = states::read_q3_player(
        &mut Reader::new(&bytes[..size], Encoding::Q3),
        &[0; states::PLAYER_WORDS],
    )?;
    assert_eq!(received[19], words[19] & 65535);
    let mut client = PlayerState::default();
    assert!(wire.apply(
        &received,
        &mut client,
        &mut PlayerContext::default(),
        ResourceNamespace::Client(ClientId(0)),
        |_| None,
    ));
    assert!(client.movement.respawned);

    // Rerelease bit512 is a different shared capability, never a Q3 respawn lock.
    let rr = PlayerProjection::load(Protocol::Quake2Repro1038, &[]);
    let mut rr_words = [0; states::Q2_REPRO_PLAYER_WORDS];
    rr_words[qa_network::projection::PM_FLAGS_WORD] = 512;
    assert!(rr.apply(&rr_words, &mut player, &mut context, namespace, |_| None));
    assert!(!player.movement.respawned);
    assert!(player.movement.ignore_player_collision);
    assert_eq!(player.native.unwrap().flags, 512);
    assert_eq!(
        (player.movement_rules, player.trace_rules),
        (RuleSetId::Quake2, RuleSetId::QuakeWorld)
    );
    Ok(())
}

#[test]
fn rerelease_ignore_player_flag_survives_wire_and_shared_movement_projection()
-> Result<(), qa_network::message::Error> {
    use qa_core::primitives::{ClientId, ModuleId, ResourceNamespace};
    let namespace = ResourceNamespace::Module(ModuleId(7));
    let wire = PlayerProjection::load(Protocol::Quake2Repro1038, &[]);
    let movement = PlayerProjection::load_movement(Protocol::Quake2Repro1038);
    let mut source = states::Q2ReproPlayer::default();
    source.words[qa_network::projection::PM_FLAGS_WORD] = 512 | 2048;
    let mut bytes = [0; 1400];
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    let extra = states::write_q2_repro_player(&mut writer, &Default::default(), &source)?;
    let decoded = states::read_q2_repro_player(
        &mut Reader::new(writer.bytes(), Encoding::Bytes),
        &Default::default(),
        extra,
    )?;
    let words: Vec<_> = decoded.words.into_iter().chain(decoded.stats).collect();
    let mut player = PlayerState {
        movement_rules: RuleSetId::Quake3,
        trace_rules: RuleSetId::Quake,
        ..Default::default()
    };
    let mut context = PlayerContext::default();
    assert!(wire.apply(
        &words,
        &mut player,
        &mut context,
        ResourceNamespace::Client(ClientId(3)),
        |_| None
    ));
    assert!(player.movement.ignore_player_collision);
    assert!(!player.movement.respawned);
    // Native Pmove borrows the module's current tail, not another CLIENT's
    // retained wire metadata. Source ownership stays explicit at the boundary.
    assert!(wire.apply(&words, &mut player, &mut context, namespace, |_| None));
    assert!(movement.apply(&words, &mut player, &mut context, namespace, |_| None));
    assert!(player.movement.ignore_player_collision);
    assert_eq!(
        (player.movement_rules, player.trace_rules),
        (RuleSetId::Quake3, RuleSetId::Quake)
    );
    let empty = vec![0; wire.words()];
    assert!(wire.apply_stats(&empty, &mut player, namespace));
    assert!(player.movement.ignore_player_collision);
    assert!(!movement.apply(&[], &mut player, &mut context, namespace, |_| None));
    assert!(player.movement.ignore_player_collision);
    let mut reduced = vec![0; movement.words()];
    assert!(movement.reduce(&player, &context, &mut reduced, |_| None, |_| None));
    assert_eq!(reduced[qa_network::projection::PM_FLAGS_WORD], 512 | 2048);
    player.movement.ignore_player_collision = false;
    assert!(movement.reduce(&player, &context, &mut reduced, |_| None, |_| None));
    assert_eq!(reduced[qa_network::projection::PM_FLAGS_WORD], 2048);
    player.movement.ignore_player_collision = true;
    let q3 = PlayerProjection::load(Protocol::Quake3_68, &[]);
    let mut q3_words = [0; states::PLAYER_WORDS];
    q3_words[19] = 512;
    assert!(q3.apply(&q3_words, &mut player, &mut context, namespace, |_| None));
    assert!(player.movement.respawned);
    assert!(!player.movement.ignore_player_collision);
    Ok(())
}
