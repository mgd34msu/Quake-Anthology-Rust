use super::*;
use qa_core::primitives::PlayerViewFlags;
use qa_network::{
    message::{Encoding, Reader, Writer},
    projection::{PlayerContext, PlayerProjection},
    states,
};

#[test]
fn native_camera_layouts_feed_the_existing_player_packet_projection() {
    for layout in [&CLASSIC, &RERELEASE] {
        let at = layout.presentation;
        let mut memory = ModuleMemory::load(0, 512, &[]).unwrap();
        let raw = Vec3([1.25, -2.5, 1.125]);
        memory.write_vec3(at.offset as u64, raw).unwrap();
        if let Some(height) = at.view_height {
            memory.write(height as u64, &[22]).unwrap();
        }
        for (index, value) in [1.0f32, 0.5, 0.25, 0.75].into_iter().enumerate() {
            memory
                .write_word((at.screen_blend + index * 4) as u64, value.to_bits() as i32)
                .unwrap();
        }
        if let Some(damage) = at.damage_blend {
            for (index, value) in [0.125f32, 0.375, 0.625, 0.875].into_iter().enumerate() {
                memory
                    .write_word((damage + index * 4) as u64, value.to_bits() as i32)
                    .unwrap();
            }
        }
        memory
            .write_word(at.fov as u64, 67.0f32.to_bits() as i32)
            .unwrap();
        memory
            .write_word(at.flags as u64, 0x8000_0095u32 as i32)
            .unwrap();
        let mut player = PlayerState::default();
        player.body.position = Vec3([128.0, -16.0, 32.0]);
        player.movement_rules = RuleSetId::Quake3;
        player.trace_rules = RuleSetId::QuakeWorld;
        layout
            .import_presentation(&memory, 0, ModuleId(7), &mut player)
            .unwrap();
        assert_eq!(
            player.view.raw_offset(),
            Some((raw, at.view_height.map(|_| 22.0)))
        );
        assert_eq!(player.view.fov_x, Some(67.0));
        assert_eq!(player.view.screen_blend, [1.0, 0.5, 0.25, 0.75]);
        assert!(player.view.flags.contains(PlayerViewFlags::INFRARED));
        assert_eq!(
            player.view.flags.contains(PlayerViewFlags::NO_WEAPON_LERP),
            at.view_height.is_some()
        );
        assert!(!player.view.flags.contains(PlayerViewFlags::TELEPORT));
        let protocol = layout.stats.protocol;
        let projection = PlayerProjection::load(protocol, &[]);
        let context = PlayerContext {
            namespace: Some(ResourceNamespace::Module(ModuleId(7))),
            ..Default::default()
        };
        let mut words = vec![0; projection.words()];
        assert!(projection.reduce(&player, &context, &mut words, |_| None, |_| None));
        let mut bytes = [0; 1400];
        let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
        if at.view_height.is_some() {
            assert_eq!(player.native.unwrap().view_flags.unwrap().raw, 0x95);
            assert_eq!(words[35], 5);
            assert_eq!(words[41], 22);
            let mut native = states::Q2ReproPlayer::default();
            native.words.copy_from_slice(&words[..43]);
            native.stats.copy_from_slice(&words[43..]);
            let flags =
                states::write_q2_repro_player(&mut writer, &Default::default(), &native).unwrap();
            let decoded = states::read_q2_repro_player(
                &mut Reader::new(writer.bytes(), Encoding::Bytes),
                &Default::default(),
                flags,
            )
            .unwrap();
            words[..43].copy_from_slice(&decoded.words);
            words[43..].copy_from_slice(&decoded.stats);
        } else {
            assert_eq!(player.native.unwrap().view_flags.unwrap().raw, 0x8000_0095);
            assert_eq!(words[35], 0x95);
            let native = words.as_slice().try_into().unwrap();
            states::write_q2_player(&mut writer, &[0; states::Q2_PLAYER_WORDS], native).unwrap();
            words.copy_from_slice(
                &states::read_q2_player(
                    &mut Reader::new(writer.bytes(), Encoding::Bytes),
                    &[0; states::Q2_PLAYER_WORDS],
                )
                .unwrap(),
            );
        }
        let mut client = PlayerState::default();
        assert!(projection.apply(
            &words,
            &mut client,
            &mut Default::default(),
            ResourceNamespace::Client(qa_core::primitives::ClientId(1)),
            |_| None
        ));
        assert_eq!(client.view.fov_x, Some(67.0));
        assert_eq!(
            client.view.screen_blend,
            [1.0, 127.0 / 255.0, 63.0 / 255.0, 191.0 / 255.0]
        );
        assert_eq!(
            client.view.raw_offset().unwrap().1,
            at.view_height.map(|_| 22.0)
        );
        assert_eq!(
            client.view.damage_blend,
            if at.view_height.is_some() {
                [31.0 / 255.0, 95.0 / 255.0, 159.0 / 255.0, 223.0 / 255.0]
            } else {
                [0.0; 4]
            }
        );
        assert_eq!(
            (player.movement_rules, player.trace_rules),
            (RuleSetId::Quake3, RuleSetId::QuakeWorld)
        );
        memory
            .write_word(at.fov as u64, f32::INFINITY.to_bits() as i32)
            .unwrap();
        assert_eq!(
            layout.import_presentation(&memory, 0, ModuleId(7), &mut player),
            Err(CallError::Memory)
        );
    }
}
