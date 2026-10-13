use qa_core::primitives::{
    ClientId, ModuleId, NativePlayerTail, PlayerDamageSample, PlayerPresentation, PlayerSignal,
    PlayerSignals, PlayerState, PlayerValueBindings, Q3StatLayout, ResourceNamespace, RuleSetId,
    Vec3,
};
use qa_network::{
    commands::packet::Protocol,
    message::{Encoding, Reader, Writer},
    projection::{PlayerContext, PlayerFeedbackSample, PlayerProjection},
    states,
};

fn native_words() -> [u32; states::PLAYER_WORDS] {
    let mut words = [0; states::PLAYER_WORDS];
    for (word, value) in [
        (13, 0xabcd_ffff),
        (16, 0xa5a5_0177),
        (18, u32::MAX),
        (23, 0xbeef_0346),
        (27, 0x1234_56ea),
        (29, 0x1234_01ab),
        (30, 0xffff_fffe),
        (31, 0xf123_00ff),
        (32, 0x8000_0087),
        (38, 0x7654_00a1),
        (39, 0x1234_56b2),
    ] {
        words[word] = value;
    }
    words
}

fn native_sample() -> PlayerSignals {
    PlayerSignals {
        sequence: 0xabcd_ffff,
        events: [
            PlayerSignal {
                code: 0xa5a5_0177,
                parameter: 0x7654_00a1,
            },
            PlayerSignal {
                code: u32::MAX,
                parameter: 0x1234_56b2,
            },
        ],
        external: PlayerSignal {
            code: 0xbeef_0346,
            parameter: 0x1234_56ea,
        },
        damage: PlayerDamageSample {
            sequence: 0x1234_01ab,
            yaw: 0xffff_fffe,
            pitch: 0xf123_00ff,
            count: 0x8000_0087,
        },
    }
}

#[test]
fn native_player_signals_preserve_every_public_cell_without_changing_roles() {
    let projection =
        PlayerProjection::load_native_q3(PlayerValueBindings::EMPTY, Q3StatLayout::BASE);
    let namespace = ResourceNamespace::Module(ModuleId(7));
    let mut player = PlayerState {
        movement_rules: RuleSetId::QuakeWorld,
        trace_rules: RuleSetId::Quake2,
        ..Default::default()
    };
    let mut context = PlayerContext::default();
    let words = native_words();
    assert!(projection.apply(&words, &mut player, &mut context, namespace, |_| None));
    assert_eq!(player.native.unwrap().signals, Some(native_sample()));
    assert_eq!(
        (player.movement_rules, player.trace_rules),
        (RuleSetId::QuakeWorld, RuleSetId::Quake2)
    );
    let mut reduced = [0; states::PLAYER_WORDS];
    assert!(projection.reduce(&player, &context, &mut reduced, |_| None, |_| None));
    for word in [13, 16, 18, 23, 27, 29, 30, 31, 32, 38, 39] {
        assert_eq!(reduced[word], words[word]);
    }
    assert!(projection.apply(
        &[0; states::PLAYER_WORDS],
        &mut player,
        &mut context,
        namespace,
        |_| None
    ));
    assert_eq!(
        player.native.unwrap().signals,
        Some(PlayerSignals::default())
    );
}

#[test]
fn signals_narrow_at_the_original_q3_codec_then_enter_the_client_namespace()
-> Result<(), qa_network::message::Error> {
    let native = PlayerProjection::load_native_q3(PlayerValueBindings::EMPTY, Q3StatLayout::BASE);
    let wire = PlayerProjection::load(Protocol::Quake3_68, &[]);
    let module = ResourceNamespace::Module(ModuleId(7));
    let client = ResourceNamespace::Client(ClientId(3));
    let mut server = PlayerState::default();
    let mut context = PlayerContext::default();
    assert!(native.apply(&native_words(), &mut server, &mut context, module, |_| None));
    let retained = server.native;
    let mut words = [0; states::PLAYER_WORDS];
    assert!(wire.reduce(&server, &context, &mut words, |_| None, |_| None));
    let mut bytes = [0; 1400];
    let mut writer = Writer::new(&mut bytes, Encoding::Q3);
    states::write_q3_player(&mut writer, &[0; states::PLAYER_WORDS], &words)?;
    let received = states::read_q3_player(
        &mut Reader::new(writer.bytes(), Encoding::Q3),
        &[0; states::PLAYER_WORDS],
    )?;
    let mut accepted = PlayerState::default();
    assert!(wire.apply(
        &received,
        &mut accepted,
        &mut PlayerContext::default(),
        client,
        |_| None
    ));
    assert_eq!(accepted.native.unwrap().namespace, client);
    assert_eq!(
        accepted.native.unwrap().signals,
        Some(PlayerSignals {
            sequence: 65_535,
            events: [
                PlayerSignal {
                    code: 119,
                    parameter: 161
                },
                PlayerSignal {
                    code: 255,
                    parameter: 178
                }
            ],
            external: PlayerSignal {
                code: 838,
                parameter: 234
            },
            damage: PlayerDamageSample {
                sequence: 171,
                yaw: 254,
                pitch: 255,
                count: 135
            },
        })
    );
    assert_eq!(server.native, retained);
    Ok(())
}

#[test]
fn signal_export_requires_the_exact_native_source_and_namespace() {
    let projection =
        PlayerProjection::load_native_q3(PlayerValueBindings::EMPTY, Q3StatLayout::BASE);
    let namespace = ResourceNamespace::Module(ModuleId(7));
    let mut player = PlayerState {
        native: Some(NativePlayerTail {
            signals: Some(native_sample()),
            ..NativePlayerTail::new(RuleSetId::Quake3, namespace)
        }),
        ..Default::default()
    };
    for (source, selected) in [
        (RuleSetId::Quake3, None),
        (
            RuleSetId::Quake3,
            Some(ResourceNamespace::Module(ModuleId(8))),
        ),
        (
            RuleSetId::Quake3,
            Some(ResourceNamespace::Client(ClientId(7))),
        ),
        (RuleSetId::Quake2, Some(namespace)),
    ] {
        player.native.as_mut().unwrap().source = source;
        let mut words = [u32::MAX; states::PLAYER_WORDS];
        assert!(projection.reduce(
            &player,
            &PlayerContext {
                namespace: selected,
                ..Default::default()
            },
            &mut words,
            |_| None,
            |_| None
        ));
        for word in [13, 16, 18, 23, 27, 29, 30, 31, 32, 38, 39] {
            assert_eq!(words[word], 0);
        }
    }
}

#[test]
fn stats_only_movement_only_and_rejected_import_preserve_accepted_signal_ownership() {
    let native = PlayerProjection::load_native_q3(PlayerValueBindings::EMPTY, Q3StatLayout::BASE);
    let stats = PlayerProjection::load_stat_values(
        Protocol::Quake3_68,
        PlayerValueBindings::EMPTY,
        Q3StatLayout::BASE,
    );
    let movement = PlayerProjection::load_movement(Protocol::Quake3_68);
    let client = ResourceNamespace::Client(ClientId(3));
    let module = ResourceNamespace::Module(ModuleId(7));
    let mut player = PlayerState::default();
    assert!(native.apply(
        &native_words(),
        &mut player,
        &mut PlayerContext::default(),
        client,
        |_| None
    ));
    let accepted = player.native;
    let mut words = [0; states::PLAYER_WORDS];
    words[29] = 99;
    assert!(native.apply_stats(&words, &mut player, module));
    assert_eq!(player.native, accepted);
    assert!(stats.apply(
        &words,
        &mut player,
        &mut PlayerContext::default(),
        module,
        |_| None
    ));
    assert_eq!(player.native, accepted);
    assert!(movement.apply(
        &words,
        &mut player,
        &mut PlayerContext::default(),
        module,
        |_| None
    ));
    assert_eq!(player.native, accepted);
    assert!(!native.apply(
        &words[..47],
        &mut player,
        &mut PlayerContext::default(),
        module,
        |_| None
    ));
    assert_eq!(player.native, accepted);
}

#[test]
fn feedback_reader_preserves_native_cells_and_observes_the_accepted_wire_widths()
-> Result<(), qa_network::message::Error> {
    let native = PlayerProjection::load_native_q3(PlayerValueBindings::EMPTY, Q3StatLayout::BASE);
    let wire = PlayerProjection::load(Protocol::Quake3_68, &[]);
    let mut words = native_words();
    words[40] = 63;
    words[states::player_stat_word(Protocol::Quake3_68, 0).unwrap()] = (-73i32) as u32;
    assert_eq!(
        native.damage_feedback(&words),
        Some(PlayerFeedbackSample {
            client_number: 63,
            damage: native_sample().damage,
            health: -73,
        })
    );
    let mut bytes = [0; 1400];
    let mut writer = Writer::new(&mut bytes, Encoding::Q3);
    states::write_q3_player(&mut writer, &[0; states::PLAYER_WORDS], &words)?;
    let received = states::read_q3_player(
        &mut Reader::new(writer.bytes(), Encoding::Q3),
        &[0; states::PLAYER_WORDS],
    )?;
    assert_eq!(
        wire.damage_feedback(&received),
        Some(PlayerFeedbackSample {
            client_number: 63,
            damage: PlayerDamageSample {
                sequence: 171,
                yaw: 254,
                pitch: 255,
                count: 135,
            },
            health: -73,
        })
    );
    Ok(())
}

#[test]
fn feedback_reader_uses_the_selected_health_stat_without_importing_a_player() {
    let layout = Q3StatLayout {
        health: 7,
        ..Q3StatLayout::BASE
    };
    let mut words = native_words();
    words[40] = 0;
    words[states::player_stat_word(Protocol::Quake3_68, 0).unwrap()] = 90;
    words[states::player_stat_word(Protocol::Quake3_68, 7).unwrap()] = i32::MIN as u32 + 5;
    for projection in [
        PlayerProjection::load_native_q3(PlayerValueBindings::EMPTY, layout),
        PlayerProjection::load_values(Protocol::Quake3_68, PlayerValueBindings::EMPTY, layout),
    ] {
        assert_eq!(
            projection.damage_feedback(&words),
            Some(PlayerFeedbackSample {
                client_number: 0,
                damage: native_sample().damage,
                health: i32::MIN + 5,
            })
        );
    }
}

#[test]
fn feedback_reader_rejects_truncation_and_invalid_native_view_targets() {
    let projection = PlayerProjection::load(Protocol::Quake3_68, &[]);
    let mut words = native_words();
    words[40] = 63;
    assert!(projection.damage_feedback(&words).is_some());
    assert_eq!(
        projection.damage_feedback(&words[..projection.words() - 1]),
        None
    );
    for client in [64, u32::MAX] {
        words[40] = client;
        assert_eq!(projection.damage_feedback(&words), None);
    }
}

#[test]
fn feedback_reader_requires_an_admitted_full_damage_projection() {
    let words = native_words();
    for projection in [
        PlayerProjection::load_movement(Protocol::Quake3_68),
        PlayerProjection::load_stat_values(
            Protocol::Quake3_68,
            PlayerValueBindings::EMPTY,
            Q3StatLayout::BASE,
        ),
        PlayerProjection::load_native_q3(
            PlayerValueBindings::EMPTY,
            Q3StatLayout {
                health: u8::MAX,
                ..Q3StatLayout::BASE
            },
        ),
        PlayerProjection::load(Protocol::Quake2_34, &[]),
    ] {
        assert_eq!(projection.damage_feedback(&words), None);
    }
}

const PRESENTATION_WORDS: [usize; 13] = [3, 11, 14, 15, 17, 21, 22, 33, 37, 43, 44, 45, 46];

fn presentation_words() -> [u32; states::PLAYER_WORDS] {
    let mut words = [0; states::PLAYER_WORDS];
    for (word, value) in [
        (3, 0x1234_01fe),
        (11, 0xff00_0181),
        (14, 0xabcd_81a5),
        (15, 0x1234_abcd),
        (17, 0xffff_0083),
        (21, 0x8000_0019),
        (22, 0x1234_a581),
        (33, 0x8000_01a7),
        (37, 0xffff_1abc),
        (43, 12.5f32.to_bits()),
        (44, (-33.125f32).to_bits()),
        (45, 512.0f32.to_bits()),
        (46, 0x1234_067b),
    ] {
        words[word] = value;
    }
    words
}

#[test]
fn native_presentation_preserves_full_scalar_cells_and_ieee_bits() {
    let projection =
        PlayerProjection::load_native_q3(PlayerValueBindings::EMPTY, Q3StatLayout::BASE);
    let namespace = ResourceNamespace::Module(ModuleId(7));
    let mut words = presentation_words();
    for (word, bits) in [43, 44, 45]
        .into_iter()
        .zip([0x8000_0000, 0x7fc0_1234, 0xff80_0000])
    {
        words[word] = bits;
    }
    let mut player = PlayerState {
        movement_rules: RuleSetId::QuakeWorld,
        trace_rules: RuleSetId::Quake2,
        ..Default::default()
    };
    let mut context = PlayerContext::default();
    assert!(projection.apply(&words, &mut player, &mut context, namespace, |_| None));
    let presentation = player.native.unwrap().presentation.unwrap();
    assert_eq!(presentation.bob_cycle, 0x1234_01fe);
    assert_eq!(presentation.legs_timer as u32, 0xff00_0181);
    assert_eq!(presentation.legs_animation, 0xffff_0083);
    assert_eq!(presentation.torso_timer as u32, 0xffff_1abc);
    assert_eq!(presentation.torso_animation, 0xabcd_81a5);
    assert_eq!(presentation.movement_direction as u32, 0x1234_abcd);
    assert_eq!(presentation.entity_flags, 0x1234_a581);
    assert_eq!(presentation.weapon_phase as u32, 0x8000_0019);
    assert_eq!(presentation.generic as u32, 0x8000_01a7);
    assert_eq!(presentation.jump_pad_entity, 0x1234_067b);
    assert_eq!(
        presentation.grapple_point.0.map(f32::to_bits),
        [0x8000_0000, 0x7fc0_1234, 0xff80_0000]
    );
    let mut reduced = [0; states::PLAYER_WORDS];
    assert!(projection.reduce(&player, &context, &mut reduced, |_| None, |_| None));
    for word in PRESENTATION_WORDS {
        assert_eq!(reduced[word], words[word]);
    }
    assert_eq!(
        (player.movement_rules, player.trace_rules),
        (RuleSetId::QuakeWorld, RuleSetId::Quake2)
    );
}

#[test]
fn presentation_narrows_only_in_the_original_q3_codec_and_enters_the_client_namespace()
-> Result<(), qa_network::message::Error> {
    let native = PlayerProjection::load_native_q3(PlayerValueBindings::EMPTY, Q3StatLayout::BASE);
    let wire = PlayerProjection::load(Protocol::Quake3_68, &[]);
    let module = ResourceNamespace::Module(ModuleId(7));
    let client = ResourceNamespace::Client(ClientId(3));
    let mut server = PlayerState::default();
    let mut context = PlayerContext::default();
    assert!(native.apply(
        &presentation_words(),
        &mut server,
        &mut context,
        module,
        |_| None
    ));
    let retained = server.native;
    let mut words = [0; states::PLAYER_WORDS];
    assert!(wire.reduce(&server, &context, &mut words, |_| None, |_| None));
    let mut bytes = [0; 1400];
    let mut writer = Writer::new(&mut bytes, Encoding::Q3);
    states::write_q3_player(&mut writer, &[0; states::PLAYER_WORDS], &words)?;
    let received = states::read_q3_player(
        &mut Reader::new(writer.bytes(), Encoding::Q3),
        &[0; states::PLAYER_WORDS],
    )?;
    let mut accepted = PlayerState::default();
    let mut accepted_context = PlayerContext::default();
    assert!(wire.apply(
        &received,
        &mut accepted,
        &mut accepted_context,
        client,
        |_| None
    ));
    let tail = accepted.native.unwrap();
    assert_eq!(tail.namespace, client);
    assert_eq!(
        tail.presentation,
        Some(PlayerPresentation {
            bob_cycle: 254,
            legs_timer: 129,
            legs_animation: 131,
            torso_timer: 0xabc,
            torso_animation: 165,
            movement_direction: 13,
            grapple_point: Vec3([12.5, -33.125, 512.0]),
            entity_flags: 0xa581,
            weapon_phase: 9,
            generic: 167,
            jump_pad_entity: 0x27b,
        })
    );
    let mut reduced = [0; states::PLAYER_WORDS];
    assert!(wire.reduce(
        &accepted,
        &accepted_context,
        &mut reduced,
        |_| None,
        |_| None
    ));
    for word in PRESENTATION_WORDS {
        assert_eq!(reduced[word], received[word]);
    }
    assert_eq!(server.native, retained);
    Ok(())
}

#[test]
fn presentation_export_requires_the_original_source_and_exact_resource_namespace() {
    let projection =
        PlayerProjection::load_native_q3(PlayerValueBindings::EMPTY, Q3StatLayout::BASE);
    let namespace = ResourceNamespace::Module(ModuleId(7));
    let mut player = PlayerState::default();
    assert!(projection.apply(
        &presentation_words(),
        &mut player,
        &mut PlayerContext::default(),
        namespace,
        |_| None
    ));
    for (source, selected) in [
        (RuleSetId::Quake3, None),
        (
            RuleSetId::Quake3,
            Some(ResourceNamespace::Module(ModuleId(8))),
        ),
        (
            RuleSetId::Quake3,
            Some(ResourceNamespace::Client(ClientId(7))),
        ),
        (RuleSetId::Quake2, Some(namespace)),
    ] {
        player.native.as_mut().unwrap().source = source;
        let mut words = [u32::MAX; states::PLAYER_WORDS];
        assert!(projection.reduce(
            &player,
            &PlayerContext {
                namespace: selected,
                ..Default::default()
            },
            &mut words,
            |_| None,
            |_| None
        ));
        for word in PRESENTATION_WORDS {
            assert_eq!(words[word], 0);
        }
    }
}

#[test]
fn partial_and_rejected_imports_preserve_presentation_until_a_full_clear() {
    let native = PlayerProjection::load_native_q3(PlayerValueBindings::EMPTY, Q3StatLayout::BASE);
    let stats = PlayerProjection::load_stat_values(
        Protocol::Quake3_68,
        PlayerValueBindings::EMPTY,
        Q3StatLayout::BASE,
    );
    let movement = PlayerProjection::load_movement(Protocol::Quake3_68);
    let client = ResourceNamespace::Client(ClientId(3));
    let module = ResourceNamespace::Module(ModuleId(7));
    let mut player = PlayerState::default();
    assert!(native.apply(
        &presentation_words(),
        &mut player,
        &mut PlayerContext::default(),
        client,
        |_| None
    ));
    let accepted = player.native;
    let zero = [0; states::PLAYER_WORDS];
    assert!(native.apply_stats(&zero, &mut player, module));
    assert!(stats.apply(
        &zero,
        &mut player,
        &mut PlayerContext::default(),
        module,
        |_| None
    ));
    assert!(movement.apply(
        &zero,
        &mut player,
        &mut PlayerContext::default(),
        module,
        |_| None
    ));
    assert!(!native.apply(
        &zero[..native.words() - 1],
        &mut player,
        &mut PlayerContext::default(),
        module,
        |_| None
    ));
    assert_eq!(player.native, accepted);
    assert!(native.apply(
        &zero,
        &mut player,
        &mut PlayerContext::default(),
        client,
        |_| None
    ));
    assert_eq!(
        player.native.unwrap().presentation,
        Some(PlayerPresentation::default())
    );
}
