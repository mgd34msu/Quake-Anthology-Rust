use qa_app::client_resources::{ConfigPublication, ModelName, ResourceReadiness};
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
    publication.configure_sources(4, 64);
    publication.bind_sounds(bank);
    publication.connection(ModuleId(1), layout, layout);
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
    p.connection(ModuleId(2), layout, layout);
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
    p.connection(ModuleId(1), layout, layout);
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
    publication.configure_sources(4, 64);
    publication.bind_sounds(&bank);
    publication.connection(ModuleId(1), layout, layout);
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
            |_, _usage| None,
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
    publication.configure_sources(4, 64);
    publication.connection(ModuleId(1), layout, layout);
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

#[test]
fn models_use_exact_name_bindings_and_only_the_current_destination_ack() {
    use qa_app::client_resources::ModelProjection::{Pending, Ready};
    use qa_core::primitives::{ModelRef, NameId};
    let mut p = ConfigPublication::load();
    p.configure_sources(4, 1024);
    let layout = ConfigLayout::QUAKE2;
    p.connection(ModuleId(1), layout, layout);
    let source = ModelRef::native(ResourceNamespace::Module(ModuleId(1)), 7).unwrap();
    p.primary_model(7, true, Some(NameId(1)));
    let primary = layout.resources[0].first as usize + 7;
    p.published(primary, Some(10));
    assert_eq!(
        p.project_model_ref(
            source,
            qa_core::primitives::ModelUse::Entity(0),
            ClientId(0),
            |_, _, _| Some(ModelName {
                name: Some(NameId(1)),
                revision: 1,
                source: RuleSetId::Quake3
            })
        ),
        Pending
    );
    p.acknowledged(primary, 10);
    assert_eq!(
        p.project_model_ref(
            source,
            qa_core::primitives::ModelUse::Entity(0),
            ClientId(0),
            |_, _, _| Some(ModelName {
                name: Some(NameId(1)),
                revision: 1,
                source: RuleSetId::Quake3
            })
        ),
        Ready(7)
    );
    let foreign = ModelRef::native(ResourceNamespace::Module(ModuleId(2)), 3).unwrap();
    assert_eq!(
        p.project_model_ref(
            foreign,
            qa_core::primitives::ModelUse::Entity(0),
            ClientId(0),
            |_, _, _| Some(ModelName {
                name: Some(NameId(2)),
                revision: 1,
                source: RuleSetId::Quake3
            })
        ),
        Pending
    );
    let first = p.reserve_model(NameId(2)).unwrap();
    assert_eq!(first, 126);
    let first_row = layout.resources[0].first as usize + first as usize;
    p.published(first_row, Some(11));
    p.acknowledged(first_row, 10);
    assert_eq!(
        p.project_model_ref(
            foreign,
            qa_core::primitives::ModelUse::Entity(0),
            ClientId(0),
            |_, _, _| Some(ModelName {
                name: Some(NameId(2)),
                revision: 1,
                source: RuleSetId::Quake3
            })
        ),
        Pending
    );
    p.acknowledged(first_row, 11);
    assert_eq!(
        p.project_model_ref(
            foreign,
            qa_core::primitives::ModelUse::Entity(0),
            ClientId(0),
            |_, _, _| Some(ModelName {
                name: Some(NameId(2)),
                revision: 1,
                source: RuleSetId::Quake3
            })
        ),
        Ready(first)
    );
    // A source rewrite changes the exact NameId. Its old receipt cannot ready
    // the new destination row, even though the native source ordinal is stable.
    assert_eq!(
        p.project_model_ref(
            foreign,
            qa_core::primitives::ModelUse::Entity(0),
            ClientId(0),
            |_, _, _| Some(ModelName {
                name: Some(NameId(3)),
                revision: 2,
                source: RuleSetId::Quake3
            })
        ),
        Pending
    );
    let rewritten = p.reserve_model(NameId(3)).unwrap();
    assert_eq!(rewritten, 125);
    let row = layout.resources[0].first as usize + rewritten as usize;
    p.published(row, Some(12));
    p.acknowledged(row, 11);
    assert_eq!(
        p.project_model_ref(
            foreign,
            qa_core::primitives::ModelUse::Entity(0),
            ClientId(0),
            |_, _, _| Some(ModelName {
                name: Some(NameId(3)),
                revision: 2,
                source: RuleSetId::Quake3
            })
        ),
        Pending
    );
    p.acknowledged(row, 12);
    assert_eq!(
        p.project_model_ref(
            foreign,
            qa_core::primitives::ModelUse::Entity(0),
            ClientId(0),
            |_, _, _| Some(ModelName {
                name: Some(NameId(3)),
                revision: 2,
                source: RuleSetId::Quake3
            })
        ),
        Ready(rewritten)
    );
    assert_eq!(
        p.project_model_ref(
            foreign,
            qa_core::primitives::ModelUse::Entity(0),
            ClientId(0),
            |_, _, _| Some(ModelName {
                name: Some(NameId(1)),
                revision: 2,
                source: RuleSetId::Quake3
            })
        ),
        Ready(7)
    );
    // Primary growth takes its native ordinal back; the displaced foreign file
    // acquires one other bounded row and waits for that row's own ACK.
    p.primary_model(first, true, Some(NameId(4)));
    p.published(first_row, Some(13));
    let relocated = p.reserve_model(NameId(2)).unwrap();
    assert_eq!(relocated, 124);
    let row = layout.resources[0].first as usize + relocated as usize;
    p.published(row, Some(14));
    p.acknowledged(row, 11);
    assert_eq!(
        p.project_model_ref(
            foreign,
            qa_core::primitives::ModelUse::Entity(0),
            ClientId(0),
            |_, _, _| Some(ModelName {
                name: Some(NameId(2)),
                revision: 2,
                source: RuleSetId::Quake3
            })
        ),
        Pending
    );
    p.acknowledged(row, 14);
    assert_eq!(
        p.project_model_ref(
            foreign,
            qa_core::primitives::ModelUse::Entity(0),
            ClientId(0),
            |_, _, _| Some(ModelName {
                name: Some(NameId(2)),
                revision: 2,
                source: RuleSetId::Quake3
            })
        ),
        Ready(relocated)
    );
}

#[test]
fn model_exhaustion_is_terminal_only_after_publication_and_rr255_stays_native() {
    use qa_app::client_resources::ModelProjection::{Pending, Ready, Unsupported};
    use qa_core::primitives::{ModelRef, NameId};
    let mut p = ConfigPublication::load();
    p.configure_sources(4, 1024);
    let layout = ConfigLayout::QUAKE2_RERELEASE;
    p.connection(ModuleId(1), layout, layout);
    for ordinal in 1..layout.resources[0].count {
        if layout.resource_policies[0].permits(ordinal) {
            p.primary_model(ordinal, true, Some(NameId(1)));
        }
    }
    assert_eq!(p.reserve_model(NameId(900)), None);
    let foreign = ModelRef::native(ResourceNamespace::Module(ModuleId(2)), 9).unwrap();
    assert_eq!(
        p.project_model_ref(
            foreign,
            qa_core::primitives::ModelUse::Entity(0),
            ClientId(0),
            |_, _, _| Some(ModelName {
                name: Some(NameId(900)),
                revision: 3,
                source: RuleSetId::Quake3
            })
        ),
        Pending
    );
    p.finish(ModuleId(2), 3);
    assert_eq!(
        p.project_model_ref(
            foreign,
            qa_core::primitives::ModelUse::Entity(0),
            ClientId(0),
            |_, _, _| Some(ModelName {
                name: Some(NameId(900)),
                revision: 3,
                source: RuleSetId::Quake3
            })
        ),
        Unsupported
    );
    assert_eq!(
        p.project_model_ref(
            foreign,
            qa_core::primitives::ModelUse::Entity(0),
            ClientId(0),
            |_, _, _| Some(ModelName {
                name: Some(NameId(900)),
                revision: 4,
                source: RuleSetId::Quake3
            })
        ),
        Pending
    );
    let primary = ModelRef::native(ResourceNamespace::Module(ModuleId(1)), 255).unwrap();
    assert_eq!(
        p.project_model_ref(
            primary,
            qa_core::primitives::ModelUse::Entity(0),
            ClientId(0),
            |_, _, _| Some(ModelName {
                name: None,
                revision: 0,
                source: RuleSetId::Quake2
            })
        ),
        Ready(255)
    );
    assert_eq!(
        p.project_model_ref(
            ModelRef::native(ResourceNamespace::Client(ClientId(1)), 255).unwrap(),
            qa_core::primitives::ModelUse::Entity(0),
            ClientId(0),
            |_, _, _| None
        ),
        Unsupported
    );
    p.reset();
    p.connection(ModuleId(1), layout, layout);
    assert_eq!(p.reserve_model(NameId(900)), Some(8191));
    for ordinal in (256..8191).rev() {
        p.primary_model(ordinal, true, Some(NameId(1)));
    }
    p.primary_model(8191, true, Some(NameId(1)));
    assert_eq!(p.reserve_model(NameId(901)), Some(254));
}

#[test]
fn q2_model_cells_preserve_source_tags_and_do_not_treat_viewweapon255_as_player() {
    use qa_app::client_resources::ModelProjection::{Pending, Ready, Unsupported};
    use qa_core::primitives::{ModelRef, ModelUse, NameId};
    for layout in [ConfigLayout::QUAKE2, ConfigLayout::QUAKE2_RERELEASE] {
        let mut p = ConfigPublication::load();
        p.configure_sources(4, 64);
        p.connection(ModuleId(1), layout, layout);
        let reference = ModelRef::native(ResourceNamespace::Module(ModuleId(1)), 255).unwrap();
        let player = |_, _, _| {
            Some(ModelName {
                name: None,
                revision: 1,
                source: RuleSetId::Quake2,
            })
        };
        assert_eq!(
            p.project_model_ref(reference, ModelUse::Entity(0), ClientId(0), player),
            Ready(255)
        );
        assert_eq!(
            p.project_model_ref(reference, ModelUse::Entity(1), ClientId(0), player),
            Ready(255)
        );
        assert_eq!(
            p.project_model_ref(reference, ModelUse::ViewWeapon, ClientId(0), player),
            Unsupported
        );
        let ordinal = p.reserve_model(NameId(8)).unwrap();
        assert_ne!(ordinal, 255);
        let row = layout.resources[0].first as usize + ordinal as usize;
        p.published(row, Some(31));
        let file = |_, _, _| {
            Some(ModelName {
                name: Some(NameId(8)),
                revision: 1,
                source: RuleSetId::Quake3,
            })
        };
        assert_eq!(
            p.project_model_ref(reference, ModelUse::Entity(0), ClientId(0), file),
            Pending
        );
        p.acknowledged(row, 31);
        assert_eq!(
            p.project_model_ref(reference, ModelUse::Entity(0), ClientId(0), file),
            Ready(ordinal)
        );
        assert_eq!(
            p.project_model_ref(reference, ModelUse::ViewWeapon, ClientId(0), file),
            Ready(ordinal)
        );
    }
    let mut p = ConfigPublication::load();
    p.configure_sources(4, 64);
    p.connection(ModuleId(1), ConfigLayout::QUAKE2, ConfigLayout::QUAKE2);
    p.primary_model(2, true, Some(NameId(2)));
    let row = ConfigLayout::QUAKE2.resources[0].first as usize + 2;
    p.published(row, Some(32));
    p.acknowledged(row, 32);
    let reference = ModelRef::native(ResourceNamespace::Module(ModuleId(1)), 0x82).unwrap();
    let source = |_, _, _| {
        Some(ModelName {
            name: Some(NameId(2)),
            revision: 1,
            source: RuleSetId::Quake2,
        })
    };
    assert_eq!(
        p.project_model_ref(reference, ModelUse::Entity(1), ClientId(0), source),
        Ready(0x82)
    );
    // A high ordinary primary slot cannot be reused as a foreign secondary
    // filename: bit7 has a native meaning in that cell.
    p.primary_model(200, true, Some(NameId(3)));
    let row = ConfigLayout::QUAKE2.resources[0].first as usize + 200;
    p.published(row, Some(33));
    p.acknowledged(row, 33);
    let high = ModelRef::native(ResourceNamespace::Module(ModuleId(1)), 200).unwrap();
    let ordinary = |_, _, _| {
        Some(ModelName {
            name: Some(NameId(3)),
            revision: 2,
            source: RuleSetId::Quake3,
        })
    };
    assert_eq!(
        p.project_model_ref(high, ModelUse::Entity(1), ClientId(0), ordinary),
        Pending
    );
    for usage in [ModelUse::Entity(0), ModelUse::ViewWeapon] {
        assert_eq!(
            p.project_model_ref(high, usage, ClientId(0), ordinary),
            Ready(200)
        );
    }
    let alias = p.reserve_model(NameId(3)).unwrap();
    assert_eq!(alias, 126);
    p.release_model_aliases();
    assert_eq!(p.reserve_model(NameId(3)), Some(126));
    let row = ConfigLayout::QUAKE2.resources[0].first as usize + alias as usize;
    p.published(row, Some(34));
    assert_eq!(
        p.project_model_ref(high, ModelUse::Entity(1), ClientId(0), ordinary),
        Pending
    );
    p.acknowledged(row, 34);
    assert_eq!(
        p.project_model_ref(high, ModelUse::Entity(1), ClientId(0), ordinary),
        Ready(126)
    );
    for usage in [ModelUse::Entity(0), ModelUse::ViewWeapon] {
        assert_eq!(
            p.project_model_ref(high, usage, ClientId(0), ordinary),
            Ready(200)
        );
    }
    let invalid = ModelRef::native(ResourceNamespace::Module(ModuleId(1)), 456).unwrap();
    assert_eq!(
        p.project_model_ref(invalid, ModelUse::Entity(1), ClientId(0), source),
        Unsupported
    );
}

#[test]
fn tagged_q2_foreign_alias_never_emits_the_player_sentinel() {
    use qa_app::client_resources::ModelProjection::{Pending, Ready};
    use qa_core::primitives::{ModelRef, ModelUse, NameId};
    let layout = ConfigLayout::QUAKE2;
    for primary_same_name in [false, true] {
        let mut p = ConfigPublication::load();
        p.configure_sources(4, 64);
        p.connection(ModuleId(1), layout, layout);
        let name = NameId(5);
        let ordinary = |_, _, _| {
            Some(ModelName {
                name: Some(name),
                revision: 1,
                source: RuleSetId::Quake3,
            })
        };
        let primary = ModelRef::native(ResourceNamespace::Module(ModuleId(1)), 127).unwrap();
        if primary_same_name {
            p.primary_model(127, true, Some(name));
            let row = layout.resources[0].first as usize + 127;
            p.published(row, Some(40));
            p.acknowledged(row, 40);
            for usage in [
                ModelUse::Entity(0),
                ModelUse::Entity(1),
                ModelUse::ViewWeapon,
            ] {
                assert_eq!(
                    p.project_model_ref(primary, usage, ClientId(0), ordinary),
                    Ready(127)
                );
            }
        }
        let tagged = ModelRef::native(ResourceNamespace::Module(ModuleId(2)), 200).unwrap();
        let foreign = |_, _, _| {
            Some(ModelName {
                name: Some(name),
                revision: 1,
                source: RuleSetId::Quake2,
            })
        };
        assert_eq!(
            p.project_model_ref(tagged, ModelUse::Entity(1), ClientId(0), foreign),
            Pending
        );
        let alias = p.reserve_model(name).unwrap();
        assert_eq!(alias, 126);
        p.release_model_aliases();
        assert_eq!(p.reserve_model(name), Some(alias));
        let row = layout.resources[0].first as usize + alias as usize;
        p.published(row, Some(41));
        assert_eq!(
            p.project_model_ref(tagged, ModelUse::Entity(1), ClientId(0), foreign),
            Pending
        );
        p.acknowledged(row, 41);
        assert_eq!(
            p.project_model_ref(tagged, ModelUse::Entity(1), ClientId(0), foreign),
            Ready(254)
        );
        if primary_same_name {
            for usage in [
                ModelUse::Entity(0),
                ModelUse::Entity(1),
                ModelUse::ViewWeapon,
            ] {
                assert_eq!(
                    p.project_model_ref(primary, usage, ClientId(0), ordinary),
                    Ready(127)
                );
            }
        }
    }
}

#[test]
fn classic_q2_primary_filename255_keeps_ordinary_model_cells() {
    use qa_app::client_resources::ModelProjection::{Pending, Ready};
    use qa_core::primitives::{ModelRef, ModelUse, NameId};
    let layout = ConfigLayout::QUAKE2;
    let mut p = ConfigPublication::load();
    p.configure_sources(4, 64);
    p.connection(ModuleId(1), layout, layout);
    let name = NameId(17);
    p.primary_model(255, true, Some(name));
    let row = layout.resources[0].first as usize + 255;
    p.published(row, Some(50));
    p.acknowledged(row, 50);
    let reference = ModelRef::native(ResourceNamespace::Module(ModuleId(1)), 255).unwrap();
    let native = |_, _, _| {
        Some(ModelName {
            name: Some(name),
            revision: 1,
            source: RuleSetId::Quake2,
        })
    };
    for usage in [
        ModelUse::ViewWeapon,
        ModelUse::Entity(2),
        ModelUse::Entity(3),
        ModelUse::Entity(0),
        ModelUse::Entity(1),
    ] {
        assert_eq!(
            p.project_model_ref(reference, usage, ClientId(0), native),
            Ready(255)
        );
    }
    let ordinary = |_, _, _| {
        Some(ModelName {
            name: Some(name),
            revision: 1,
            source: RuleSetId::Quake3,
        })
    };
    assert_eq!(
        p.project_model_ref(reference, ModelUse::Entity(0), ClientId(0), ordinary),
        Pending
    );
    let alias = p.reserve_model(name).unwrap();
    assert_eq!(alias, 126);
    p.release_model_aliases();
    let row = layout.resources[0].first as usize + alias as usize;
    p.published(row, Some(51));
    p.acknowledged(row, 51);
    assert_eq!(
        p.project_model_ref(reference, ModelUse::Entity(0), ClientId(0), ordinary),
        Ready(alias)
    );
    for usage in [
        ModelUse::ViewWeapon,
        ModelUse::Entity(2),
        ModelUse::Entity(3),
    ] {
        assert_eq!(
            p.project_model_ref(reference, usage, ClientId(0), native),
            Ready(255)
        );
    }
    for usage in [ModelUse::Entity(0), ModelUse::Entity(1)] {
        assert_eq!(
            p.project_model_ref(reference, usage, ClientId(0), native),
            Ready(255)
        );
    }
}

#[test]
fn unchanged_bank_attachment_preserves_sound_claims_cursors_and_native_receipts() {
    let bank = bank();
    let (mut p, layout) = publication(&bank);
    let sound = bank.find("sound/a.wav").unwrap();
    p.primary_sound(1, true, Some(sound));
    let row = layout.resources[1].first as usize + 1;
    p.seen(row, 3);
    p.published(row, Some(7));
    p.finish(ModuleId(1), 5);
    p.bind_sounds(&bank);
    assert!(!p.begin(ModuleId(1), 5));
    assert!(!p.needs(row, 3));
    assert_eq!(p.sound(sound), Some((1, ResourceReadiness::Pending)));
    p.acknowledged(row, 7);
    assert_eq!(p.sound(sound), Some((1, ResourceReadiness::Ready)));
    let reference = SoundRef::native(ResourceNamespace::Module(ModuleId(1)), 1).unwrap();
    assert_eq!(
        p.project_sound_ref(reference, ClientId(0), |_, _| None),
        Some(1)
    );
    p.bind_sounds(&bank);
    assert_eq!(p.sound(sound), Some((1, ResourceReadiness::Ready)));
    p.connection(ModuleId(2), layout, layout);
    assert_eq!(
        p.sound(sound),
        None,
        "a real connection change still clears its namespace"
    );
}
