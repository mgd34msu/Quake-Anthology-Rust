use qa_app::client_resources::{ConfigPublication, ResourceReadiness};
use qa_core::{
    configstrings::ConfigLayout,
    primitives::{ClientId, ModuleId, ResourceNamespace, RuleSetId, SoundId, SoundRef},
};
use std::num::NonZeroU32;

fn bank() -> qa_audio::Bank {
    let rate = NonZeroU32::new(44100).unwrap();
    qa_audio::Bank::load(
        rate,
        ["sound/a.wav", "sound/b.wav", "sound/c.wav"]
            .map(|path| {
                (
                    path.into(),
                    qa_core::primitives::Pcm {
                        rate,
                        channels: qa_core::primitives::PcmChannels::Mono,
                        samples: vec![1; 2],
                        loop_start: None,
                        loop_end: None,
                        loop_invalid: false,
                    },
                )
            })
            .into(),
    )
    .unwrap()
}
fn publication(bank: &qa_audio::Bank) -> (ConfigPublication, ConfigLayout) {
    let layout = ConfigLayout::for_rules(RuleSetId::Quake2);
    let mut publication = ConfigPublication::load();
    publication.configure_sources(4);
    publication.bind_sounds(bank);
    publication.connection(ModuleId(1), layout);
    (publication, layout)
}
#[test]
fn current_destination_sequence_is_the_only_ack_that_readies_a_row() {
    let bank = bank();
    let (mut p, layout) = publication(&bank);
    let row = layout.resources[1].first as usize + 1;
    assert!(p.begin(ModuleId(1), 1));
    assert_eq!(p.readiness(row), ResourceReadiness::Pending);
    p.published(row, Some(7));
    p.acknowledged(row, 6);
    assert_eq!(p.readiness(row), ResourceReadiness::Pending);
    p.acknowledged(row, 7);
    assert_eq!(p.readiness(row), ResourceReadiness::Ready);
    p.published(row, Some(8));
    p.acknowledged(row, 7);
    assert_eq!(p.readiness(row), ResourceReadiness::Pending);
    p.published(row, None);
    p.acknowledged(row, 8);
    assert_eq!(p.readiness(row), ResourceReadiness::Unsupported);
    p.connection(ModuleId(2), layout);
    p.published(row, Some(9));
    p.acknowledged(row, 8);
    assert_eq!(p.readiness(row), ResourceReadiness::Pending);
    p.finish(ModuleId(2), 1);
    assert!(!p.begin(ModuleId(2), 1));
    assert!(p.begin(ModuleId(1), 1));
}
#[test]
fn secondary_samples_do_not_alias_primary_ordinals_and_growth_relocates_them() {
    let bank = bank();
    let (mut p, layout) = publication(&bank);
    let a = bank.find("sound/a.wav").unwrap();
    let b = bank.find("sound/b.wav").unwrap();
    let c = bank.find("sound/c.wav").unwrap();
    p.primary_sound(1, true, Some(a));
    let row = layout.resources[1].first as usize + 1;
    p.published(row, Some(3));
    p.acknowledged(row, 3);
    assert_eq!(p.sound(a), Some((1, ResourceReadiness::Ready)));
    let foreign = p.reserve_sound(b).unwrap();
    assert_eq!(foreign, 255);
    let old = layout.resources[1].first as usize + foreign as usize;
    p.published(old, Some(4));
    p.acknowledged(old, 4);
    // A newly registered primary native ordinal takes precedence. Retained
    // records retain `old`; the displaced numeric b can register elsewhere.
    p.primary_sound(foreign, true, Some(c));
    p.published(old, Some(5));
    p.acknowledged(old, 4);
    assert_eq!(p.sound(c), Some((foreign, ResourceReadiness::Pending)));
    let relocated = p.reserve_sound(b).unwrap();
    assert_eq!(relocated, 254);
    let new = layout.resources[1].first as usize + relocated as usize;
    p.published(new, Some(6));
    p.acknowledged(new, 4);
    assert_eq!(p.sound(b), Some((relocated, ResourceReadiness::Pending)));
    p.acknowledged(new, 6);
    assert_eq!(p.sound(b), Some((relocated, ResourceReadiness::Ready)));
    p.acknowledged(old, 5);
    assert_eq!(p.sound(c), Some((foreign, ResourceReadiness::Ready)));
    assert_eq!(p.reserve_sound(a), Some(1));
}
#[test]
fn full_capacity_is_unsupported_and_connection_reset_rebinds_without_old_ack() {
    let bank = bank();
    let (mut p, layout) = publication(&bank);
    for ordinal in 1..layout.resources[1].count {
        p.primary_sound(ordinal, true, None);
    }
    let a = bank.find("sound/a.wav").unwrap();
    assert_eq!(p.reserve_sound(a), None);
    assert_eq!(p.sound(SoundId(bank.len() as u32)), None);
    p.reset();
    p.connection(ModuleId(1), layout);
    let ordinal = p.reserve_sound(a).unwrap();
    let row = layout.resources[1].first as usize + ordinal as usize;
    p.published(row, Some(100));
    p.acknowledged(row, 99);
    assert_eq!(p.sound(a), Some((ordinal, ResourceReadiness::Pending)));
}

#[test]
fn snapshot_loop_resources_follow_their_owner_and_real_destination_ack() {
    let bank = bank();
    let (mut publication, layout) = publication(&bank);
    let client = ClientId(4);
    let primary = bank.find("sound/a.wav").unwrap();
    let foreign = bank.find("sound/b.wav").unwrap();
    publication.primary_sound(1, true, Some(primary));
    let destination = publication.reserve_sound(foreign).unwrap();
    let reference = SoundRef::native(ResourceNamespace::Module(ModuleId(3)), 1).unwrap();
    let mut resolve = |module, ordinal| match (module, ordinal) {
        (ModuleId(1), 1) => Some(primary),
        (ModuleId(3), 1) => Some(foreign),
        _ => None,
    };
    assert_eq!(
        publication.project_sound_ref(reference, client, &mut resolve),
        None
    );
    let row = layout.resources[1].first as usize + destination as usize;
    publication.published(row, Some(20));
    publication.acknowledged(row, 19);
    assert_eq!(
        publication.project_sound_ref(reference, client, &mut resolve),
        None
    );
    publication.acknowledged(row, 20);
    assert_eq!(
        publication.project_sound_ref(reference, client, &mut resolve),
        Some(destination)
    );
    let client_ref = SoundRef::native(ResourceNamespace::Client(client), destination).unwrap();
    assert_eq!(
        publication.project_sound_ref(client_ref, client, &mut resolve),
        Some(destination)
    );
    let other = SoundRef::native(ResourceNamespace::Client(ClientId(5)), destination).unwrap();
    assert_eq!(
        publication.project_sound_ref(other, client, &mut resolve),
        None
    );
    let missing = SoundRef::native(ResourceNamespace::Module(ModuleId(9)), 1).unwrap();
    assert_eq!(
        publication.project_sound_ref(missing, client, &mut resolve),
        None
    );
    publication.published(row, Some(21));
    assert_eq!(
        publication.project_sound_ref(reference, client, &mut resolve),
        None
    );
    publication.acknowledged(row, 20);
    assert_eq!(
        publication.project_sound_ref(reference, client, &mut resolve),
        None
    );
    publication.acknowledged(row, 21);
    assert_eq!(
        publication.project_sound_ref(reference, client, &mut resolve),
        Some(destination)
    );
}

#[test]
fn player_loop_uses_the_connections_sound_mapping_and_ack() {
    use qa_core::primitives::{NativePlayerTail, PlayerState};
    use qa_network::{
        commands::packet::Protocol,
        projection::{PlayerContext, PlayerProjection},
        states,
    };
    let bank = bank();
    let layout = ConfigLayout::QUAKE3;
    let mut publication = ConfigPublication::load();
    publication.configure_sources(4);
    publication.bind_sounds(&bank);
    publication.connection(ModuleId(1), layout);
    let primary = bank.find("sound/a.wav").unwrap();
    let foreign = bank.find("sound/b.wav").unwrap();
    publication.primary_sound(1, true, Some(primary));
    let destination = publication.reserve_sound(foreign).unwrap();
    assert_ne!(destination, 1);
    let client = ClientId(4);
    let owner = ResourceNamespace::Module(ModuleId(3));
    let mut player = PlayerState::default();
    player.native = Some(NativePlayerTail::new(RuleSetId::Quake3, owner));
    player.loop_sound = SoundRef::native(owner, 1);
    let context = PlayerContext {
        namespace: Some(owner),
        ..Default::default()
    };
    let projection = PlayerProjection::load(Protocol::Quake3_68, &[]);
    let row = layout.resources[1].first as usize + destination as usize;
    let resolve = |module, ordinal| match (module, ordinal) {
        (ModuleId(1), 1) => Some(primary),
        (ModuleId(3), 1) => Some(foreign),
        _ => None,
    };
    let reduce = |publication: &ConfigPublication| {
        let mut words = [0; states::PLAYER_WORDS];
        assert!(projection.reduce(
            &player,
            &context,
            &mut words,
            |_| None,
            |sound| publication.project_sound_ref(sound, client, resolve)
        ));
        words[47]
    };
    assert_eq!(reduce(&publication), 0);
    publication.published(row, Some(20));
    publication.acknowledged(row, 19);
    assert_eq!(reduce(&publication), 0);
    publication.acknowledged(row, 20);
    assert_eq!(reduce(&publication), destination);
    publication.published(row, Some(21));
    publication.acknowledged(row, 20);
    assert_eq!(reduce(&publication), 0);
    publication.acknowledged(row, 21);
    assert_eq!(reduce(&publication), destination);
    assert_eq!(player.loop_sound, SoundRef::native(owner, 1));
}

#[test]
fn dedicated_primary_loops_require_a_claim_and_compatible_acknowledged_row() {
    let layout = ConfigLayout::QUAKE3;
    let mut publication = ConfigPublication::load();
    publication.configure_sources(4);
    publication.connection(ModuleId(1), layout);
    let client = ClientId(4);
    let reference = SoundRef::native(ResourceNamespace::Module(ModuleId(1)), 5).unwrap();
    let project = |publication: &ConfigPublication| {
        publication.project_sound_ref(reference, client, |_, _| None)
    };
    let row = layout.resources[1].first as usize + 5;
    publication.published(row, Some(3));
    publication.acknowledged(row, 3);
    assert_eq!(project(&publication), None); // A receipt alone is not a registration.
    publication.primary_sound(5, true, None);
    assert_eq!(project(&publication), Some(5));
    publication.published(row, Some(4));
    publication.acknowledged(row, 3);
    assert_eq!(project(&publication), None);
    publication.acknowledged(row, 4);
    assert_eq!(project(&publication), Some(5));
    assert_eq!(
        publication.project_sound_ref(
            SoundRef::native(ResourceNamespace::Module(ModuleId(3)), 5).unwrap(),
            client,
            |_, _| None
        ),
        None
    );
    publication.primary_sound(5, false, None);
    assert_eq!(project(&publication), None);
    publication.primary_sound(5, true, None);
    publication.published(row, None); // Filename conversion was unsupported.
    assert_eq!(project(&publication), None);
}
