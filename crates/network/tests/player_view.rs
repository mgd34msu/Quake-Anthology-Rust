use qa_core::primitives::{
    ClientId, ModuleId, NativePlayerTail, NativeViewFlags, PlayerState, PlayerViewFlags,
    ResourceNamespace, RuleSetId, Vec3, ViewFlagLayout,
};
use qa_network::{
    commands::packet::Protocol,
    message::{Encoding, Reader, Writer},
    projection::{PlayerContext, PlayerProjection},
    states,
};

fn wire(protocol: Protocol, words: &[u32]) -> Vec<u32> {
    let mut bytes = [0; 1400];
    let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
    if protocol == Protocol::Quake2_34 {
        let words = words.try_into().unwrap();
        states::write_q2_player(&mut writer, &[0; states::Q2_PLAYER_WORDS], words).unwrap();
        states::read_q2_player(
            &mut Reader::new(writer.bytes(), Encoding::Bytes),
            &[0; states::Q2_PLAYER_WORDS],
        )
        .unwrap()
        .to_vec()
    } else {
        let mut native = states::Q2ReproPlayer::default();
        native.words.copy_from_slice(&words[..43]);
        native.stats.copy_from_slice(&words[43..]);
        let extra =
            states::write_q2_repro_player(&mut writer, &Default::default(), &native).unwrap();
        let native = states::read_q2_repro_player(
            &mut Reader::new(writer.bytes(), Encoding::Bytes),
            &Default::default(),
            extra,
        )
        .unwrap();
        native.words.into_iter().chain(native.stats).collect()
    }
}

#[test]
fn classic_and_repro_packets_adopt_common_camera_fields_and_keep_split_height() {
    let namespace = ResourceNamespace::Module(ModuleId(7));
    for protocol in [Protocol::Quake2_34, Protocol::Quake2Repro1038] {
        let projection = PlayerProjection::load(protocol, &[]);
        let mut source = PlayerState::default();
        source.body.position = Vec3([128.0, -16.0, 32.0]);
        source.movement.view_height = 12.0;
        source
            .view
            .accept_offset(Vec3([1.25, -2.5, 1.125]), Some(22.0));
        source.view.fov_x = Some(67.0);
        source.view.screen_blend = [1.0, 0.5, 0.25, 0.75];
        source.view.damage_blend = [0.125, 0.375, 0.625, 0.875];
        source.view.flags = PlayerViewFlags::UNDERWATER
            | PlayerViewFlags::INFRARED
            | PlayerViewFlags::NO_WEAPON_LERP
            | PlayerViewFlags::TELEPORT;
        let mut native = NativePlayerTail::new(protocol.rule_set(), namespace);
        native.view_flags = Some(NativeViewFlags {
            layout: if protocol == Protocol::Quake2_34 {
                ViewFlagLayout::CLASSIC
            } else {
                ViewFlagLayout::KEX
            },
            raw: 0x80,
        });
        source.native = Some(native);
        let context = PlayerContext {
            namespace: Some(namespace),
            ..Default::default()
        };
        let mut words = vec![0; projection.words()];
        assert!(projection.reduce(&source, &context, &mut words, |_, _| None, |_| None));
        let words = wire(protocol, &words);
        let mut accepted = PlayerState::default();
        accepted.movement_rules = RuleSetId::Quake3;
        accepted.trace_rules = RuleSetId::QuakeWorld;
        assert!(projection.apply(
            &words,
            &mut accepted,
            &mut PlayerContext::default(),
            ResourceNamespace::Client(ClientId(3)),
            |_| None,
        ));
        assert_eq!(accepted.view.fov_x, Some(67.0));
        assert_eq!(
            accepted.view.screen_blend,
            [1.0, 127.0 / 255.0, 63.0 / 255.0, 191.0 / 255.0]
        );
        assert!(accepted.view.flags.contains(PlayerViewFlags::UNDERWATER));
        assert!(accepted.view.flags.contains(PlayerViewFlags::INFRARED));
        assert!(
            !accepted
                .view
                .flags
                .contains(PlayerViewFlags::NO_WEAPON_LERP)
        );
        if protocol == Protocol::Quake2_34 {
            // Classic's offset quarter-units include the separate source height.
            assert_eq!(
                accepted.view.raw_offset(),
                Some((Vec3([1.25, -2.5, 23.0]), None))
            );
            assert_eq!(accepted.view.damage_blend, [0.0; 4]);
            assert_eq!(words[35], 0x85);
            assert!(!accepted.view.flags.contains(PlayerViewFlags::TELEPORT));
        } else {
            assert_eq!(
                accepted.view.raw_offset(),
                Some((Vec3([1.25, -2.5, 1.125]), Some(22.0)))
            );
            assert_eq!(words[41], 22);
            assert_eq!(
                accepted.view.damage_blend,
                [31.0 / 255.0, 95.0 / 255.0, 159.0 / 255.0, 223.0 / 255.0]
            );
            assert_eq!(words[35], 0x15);
            assert!(accepted.view.flags.contains(PlayerViewFlags::TELEPORT));
        }
        let raw = accepted.view.raw_offset();
        let eye = accepted.eye();
        accepted.movement.view_height = -16.0;
        assert_eq!(accepted.view.raw_offset(), raw);
        assert_eq!(accepted.eye(), eye);
        assert_eq!(
            (accepted.movement_rules, accepted.trace_rules),
            (RuleSetId::Quake3, RuleSetId::QuakeWorld)
        );
    }
}

#[test]
fn native_view_residual_requires_exact_owner_and_flag_layout() {
    let namespace = ResourceNamespace::Module(ModuleId(7));
    let projection = PlayerProjection::load(Protocol::Quake2_34, &[]);
    let mut player = PlayerState::default();
    player.view.flags = PlayerViewFlags::INFRARED;
    let mut tail = NativePlayerTail::new(RuleSetId::Quake2, namespace);
    tail.view_flags = Some(NativeViewFlags {
        layout: ViewFlagLayout::CLASSIC,
        raw: 0x80,
    });
    player.native = Some(tail);
    for (selected, layout, expected) in [
        (Some(namespace), ViewFlagLayout::CLASSIC, 0x84),
        (None, ViewFlagLayout::CLASSIC, 4),
        (
            Some(ResourceNamespace::Module(ModuleId(8))),
            ViewFlagLayout::CLASSIC,
            4,
        ),
        (Some(namespace), ViewFlagLayout::KEX, 4),
    ] {
        player
            .native
            .as_mut()
            .unwrap()
            .view_flags
            .as_mut()
            .unwrap()
            .layout = layout;
        let mut words = vec![0; projection.words()];
        assert!(projection.reduce(
            &player,
            &PlayerContext {
                namespace: selected,
                ..Default::default()
            },
            &mut words,
            |_, _| None,
            |_| None
        ));
        assert_eq!(words[35], expected);
    }
}

#[test]
fn repro_height_signed_width_and_unified_foreign_offsets_are_explicit() {
    let projection = PlayerProjection::load(Protocol::Quake2Repro1038, &[]);
    for height in [-128.0, -2.0, 0.0, 127.0] {
        let mut player = PlayerState::default();
        player
            .view
            .accept_offset(Vec3([-0.0, 1.25, -0.0]), Some(height));
        let mut words = vec![0; projection.words()];
        assert!(projection.reduce(
            &player,
            &Default::default(),
            &mut words,
            |_, _| None,
            |_| None
        ));
        let words = wire(Protocol::Quake2Repro1038, &words);
        assert_eq!(words[41] as i32, height as i32);
        let mut accepted = PlayerState::default();
        assert!(projection.apply(
            &words,
            &mut accepted,
            &mut Default::default(),
            ResourceNamespace::Client(ClientId(0)),
            |_| None
        ));
        assert_eq!(accepted.view.raw_offset().unwrap().1, Some(height));
    }
    let mut player = PlayerState::default();
    player.view.accept_offset(Vec3([1.0, 2.0, 24.125]), None);
    let mut words = vec![0; projection.words()];
    assert!(projection.reduce(
        &player,
        &Default::default(),
        &mut words,
        |_, _| None,
        |_| None
    ));
    assert_eq!(words[41], 0);
    assert_eq!(&words[13..16], &[16, 32, 386]);
    assert_eq!(player.view.fov_x, None);
    assert_eq!(words[34], 90);
}

#[test]
fn q3_wire_height_drives_predicted_view_without_importing_foreign_camera_bob() {
    let projection = PlayerProjection::load(Protocol::Quake3_68, &[]);
    let mut server = PlayerState::default();
    server.body.position = Vec3([17.0, 33.0, 70.0]);
    server.movement.view_height = 26.0;
    server
        .view
        .accept_offset(Vec3([1.0, 2.0, 101.0]), Some(22.0));
    let mut words = [0; states::PLAYER_WORDS];
    assert!(projection.reduce(
        &server,
        &Default::default(),
        &mut words,
        |_, _| None,
        |_| None
    ));
    assert_eq!(words[28], 26);
    let mut bytes = [0; 1400];
    let mut writer = Writer::new(&mut bytes, Encoding::Q3);
    states::write_q3_player(&mut writer, &[0; states::PLAYER_WORDS], &words).unwrap();
    let words = states::read_q3_player(
        &mut Reader::new(writer.bytes(), Encoding::Q3),
        &[0; states::PLAYER_WORDS],
    )
    .unwrap();
    let mut client = PlayerState::default();
    client.movement_rules = RuleSetId::QuakeWorld;
    client.trace_rules = RuleSetId::Quake2;
    client.view = server.view;
    assert!(projection.apply(
        &words,
        &mut client,
        &mut Default::default(),
        ResourceNamespace::Client(ClientId(0)),
        |_| None
    ));
    assert_eq!(client.view.raw_offset(), None);
    assert_eq!(client.movement.view_height, 26.0);
    assert_eq!(client.eye(), Vec3([17.0, 33.0, 96.0]));
    // A stance result changes the movement-driven camera before another packet.
    client.movement.view_height = 12.0;
    assert_eq!(client.eye(), Vec3([17.0, 33.0, 82.0]));
    assert_eq!(
        (client.movement_rules, client.trace_rules),
        (RuleSetId::QuakeWorld, RuleSetId::Quake2)
    );
    // The restricted pmove boundary does not acquire camera ownership.
    client.view.accept_offset(Vec3([1.0, 2.0, 3.0]), Some(22.0));
    let authored = client.view;
    let movement = PlayerProjection::load_movement(Protocol::Quake3_68);
    assert!(movement.apply(
        &words,
        &mut client,
        &mut Default::default(),
        ResourceNamespace::Module(ModuleId(7)),
        |_| None
    ));
    assert_eq!(client.view, authored);
}

fn camera_words(protocol: Protocol, source: &PlayerState) -> Vec<u32> {
    let projection = PlayerProjection::load(protocol, &[]);
    let mut words = vec![0; projection.words()];
    assert!(projection.reduce(
        source,
        &Default::default(),
        &mut words,
        |_, _| None,
        |_| None
    ));
    wire(protocol, &words)
}

#[test]
fn retained_camera_samples_use_native_fov_offset_blend_and_flag_rules() {
    for protocol in [Protocol::Quake2_34, Protocol::Quake2Repro1038] {
        let projection = PlayerProjection::load(protocol, &[]);
        let mut old = PlayerState::default();
        old.body.position = Vec3([10.0, 0.0, 0.0]);
        old.view.accept_offset(Vec3([4.0, 0.0, 0.0]), Some(22.0));
        old.view.fov_x = Some(80.0);
        old.view.screen_blend = [1.0; 4];
        old.view.damage_blend = [0.0; 4];
        let mut new = PlayerState::default();
        new.body.position = Vec3([110.0, 0.0, 0.0]);
        new.view.accept_offset(Vec3([12.0, 0.0, 0.0]), Some(22.0));
        new.view.fov_x = Some(100.0);
        new.view.screen_blend = [0.0; 4];
        new.view.damage_blend = [1.0; 4];
        new.view.flags = PlayerViewFlags::UNDERWATER;
        let old = camera_words(protocol, &old);
        let new = camera_words(protocol, &new);
        let mut predicted = PlayerState::default();
        predicted.body.position = Vec3([100.0, 0.0, 0.0]);
        predicted.movement_rules = RuleSetId::Quake3;
        predicted.trace_rules = RuleSetId::QuakeWorld;
        let sample = projection
            .sample_view(Some(&old), &new, 0.25, &predicted, None)
            .unwrap();
        assert_eq!(sample.origin, Vec3([106.0, 0.0, 22.0]));
        assert_eq!(sample.view.fov_x, Some(85.0));
        assert_eq!(sample.view.flags, PlayerViewFlags::UNDERWATER);
        if protocol == Protocol::Quake2_34 {
            assert_eq!(sample.view.screen_blend, [0.0; 4]);
            assert_eq!(sample.view.damage_blend, [0.0; 4]);
            assert_eq!(
                sample.view.raw_offset(),
                Some((Vec3([6.0, 0.0, 22.0]), None))
            );
        } else {
            assert_eq!(sample.view.screen_blend, [0.75; 4]);
            // A new damage flash starts immediately when prior alpha is clear.
            assert_eq!(sample.view.damage_blend, [1.0; 4]);
            assert_eq!(
                sample.view.raw_offset(),
                Some((Vec3([6.0, 0.0, 0.0]), Some(22.0)))
            );
            assert_eq!(projection.split_view_height(&new), Some(22.0));
            let height = projection
                .sample_view(Some(&old), &new, 0.25, &predicted, Some(12.0))
                .unwrap();
            assert_eq!(height.origin.0[2], 12.0);
        }
        predicted.prediction.position = false;
        let sample = projection
            .sample_view(Some(&old), &new, 0.25, &predicted, None)
            .unwrap();
        assert_eq!(sample.origin, Vec3([41.0, 0.0, 22.0]));
    }
}

#[test]
fn classic_predicted_camera_retains_add_before_lerp_binary32_order() {
    let projection = PlayerProjection::load(Protocol::Quake2_34, &[]);
    let mut source = PlayerState::default();
    source.view.accept_offset(Vec3([1.0, 0.0, 0.0]), None);
    let old = camera_words(Protocol::Quake2_34, &source);
    source.view.accept_offset(Vec3([3.0, 0.0, 0.0]), None);
    let new = camera_words(Protocol::Quake2_34, &source);
    source.body.position = Vec3([16_777_216.0, 0.0, 0.0]);
    let sample = projection
        .sample_view(Some(&old), &new, 0.5, &source, None)
        .unwrap();
    assert_eq!(sample.origin.0[0].to_bits(), 16_777_216.0f32.to_bits());
    assert_eq!(sample.view.raw_offset().unwrap().0.0[0], 2.0);
    assert_eq!(
        (source.body.position.0[0] + 2.0).to_bits(),
        16_777_218.0f32.to_bits()
    );
}

#[test]
fn camera_sampling_rejects_missing_large_step_changed_pov_and_teleport_transitions() {
    let projection = PlayerProjection::load(Protocol::Quake2Repro1038, &[]);
    let mut source = PlayerState::default();
    source.view.accept_offset(Vec3([2.0, 0.0, 0.0]), Some(22.0));
    source.view.fov_x = Some(80.0);
    source.view.flags = PlayerViewFlags::TELEPORT;
    let old = camera_words(Protocol::Quake2Repro1038, &source);
    source
        .view
        .accept_offset(Vec3([10.0, 0.0, 0.0]), Some(22.0));
    source.view.fov_x = Some(100.0);
    let new = camera_words(Protocol::Quake2Repro1038, &source);
    // A persistent teleport bit is not itself a discontinuity.
    let sample = projection
        .sample_view(Some(&old), &new, 0.5, &source, None)
        .unwrap();
    assert_eq!(sample.view.fov_x, Some(90.0));
    assert_eq!(sample.origin.0[0], 6.0);
    let mut changed_position = new.clone();
    changed_position[1] = 257.0f32.to_bits();
    let mut changed_pov = new.clone();
    changed_pov[42] = 1;
    let mut changed_teleport = new.clone();
    changed_teleport[35] ^= 1 << 4;
    for (previous, current) in [
        (None, new.as_slice()),
        (Some(old.as_slice()), changed_position.as_slice()),
        (Some(old.as_slice()), changed_pov.as_slice()),
        (Some(old.as_slice()), changed_teleport.as_slice()),
    ] {
        let sample = projection
            .sample_view(previous, current, 0.5, &source, None)
            .unwrap();
        assert_eq!(sample.view.fov_x, Some(100.0));
        assert_eq!(sample.origin.0[0], 10.0);
    }
    assert!(
        PlayerProjection::load_movement(Protocol::Quake2Repro1038)
            .sample_view(Some(&old), &new, 0.5, &source, None)
            .is_none()
    );
}
