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
        assert!(projection.reduce(&source, &context, &mut words, |_| None, |_| None));
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
            |_| None,
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
        assert!(projection.reduce(&player, &Default::default(), &mut words, |_| None, |_| None));
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
    assert!(projection.reduce(&player, &Default::default(), &mut words, |_| None, |_| None));
    assert_eq!(words[41], 0);
    assert_eq!(&words[13..16], &[16, 32, 386]);
    assert_eq!(player.view.fov_x, None);
    assert_eq!(words[34], 90);
}
