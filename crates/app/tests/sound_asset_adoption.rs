#[path = "model_assets/support.rs"]
#[allow(dead_code)]
mod files;
#[path = "support/client_asset_frames.rs"]
mod frames;
#[path = "support/sound_registration.rs"]
#[allow(dead_code)]
mod samples;

use qa_app::{
    Runtime,
    client_policy::ClientPolicy,
    host::{FrameHost, FrameSource},
    map::SpawnAnchor,
};
use qa_audio::{Bank, Listener, Mixer};
use qa_console::{commands::Console, views::Context};
use qa_core::{primitives::*, sys_events::*};
use qa_network::{commands::packet::Protocol, outputs};
use qa_session::timing::TickRate;
use std::{num::NonZeroU32, time::Duration};

struct Audio {
    mixer: Mixer,
    events: Vec<(SoundId, Option<u32>)>,
}
impl FrameSource for Audio {
    fn begin_frame(&mut self) -> Duration {
        Duration::from_millis(100)
    }
    fn poll_events(&mut self, queue: &mut SysEventQueue) {
        queue
            .push(SysEvent {
                time: EventTime::from_milliseconds(100),
                kind: EventKind::Time,
            })
            .unwrap();
    }
    fn wait_time(&mut self, _: Duration) -> Duration {
        panic!("uncapped fixture")
    }
    fn elapsed(&self) -> Duration {
        Duration::ZERO
    }
    fn present(&mut self) {}
    fn sound(&mut self, bank: Option<&Bank>, event: SoundEvent) -> bool {
        self.events.push((event.sound, event.source_ordinal));
        self.mixer.sound(bank.unwrap(), event)
    }
}

fn setup(protocol: Protocol) -> (files::Fixture, FrameHost, Audio) {
    let mut fixture = files::Fixture::new(&[
        ("sound/kept.wav", samples::wav()),
        ("sound/late.wav", samples::wav()),
    ]);
    let mut runtime = Runtime::load(1, []).unwrap();
    runtime.vfs = std::mem::take(&mut fixture.vfs);
    let client = runtime
        .connect_local(
            SeatId::FIRST,
            SpawnAnchor {
                position: Vec3::default(),
                angles: Vec3::default(),
                entity: 0,
                fixture_fallback: false,
            },
            ClientPolicy {
                input: protocol.rule_set(),
                client: protocol.rule_set(),
                movement: RuleSetId::Quake3,
                trace: RuleSetId::Quake,
            },
            protocol,
            0,
        )
        .unwrap();
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        vec![],
    )
    .unwrap();
    host.local_clients[0] = Some(client);
    qa_app::audio::begin_registration(&mut host, &["sound/kept.wav".into()], RuleSetId::Quake3)
        .unwrap();
    qa_app::audio::finish_registration(&mut host).unwrap();
    let rate = host.runtime.sound_bank().unwrap().rate;
    let mut mixer = Mixer::load(rate, 8, 1);
    mixer.listen(
        &[Some(Listener {
            client,
            entity: None,
            origin: Vec3::default(),
            right: Vec3([1., 0., 0.]),
            negate_dot: false,
        })],
        1.,
    );
    (
        fixture,
        host,
        Audio {
            mixer,
            events: Vec::new(),
        },
    )
}
fn sound(protocol: Protocol, ordinal: u32) -> Vec<u8> {
    let mut bytes = [0; 64];
    let count = outputs::sound(
        protocol,
        SoundEvent {
            volume: 1.,
            attenuation: 0.,
            action: SoundAction::Play,
            ..Default::default()
        },
        ordinal,
        None,
        true,
        &mut bytes,
    )
    .unwrap();
    bytes[..count].to_vec()
}

#[test]
fn changed_client_sound_is_admitted_before_a_following_cue_in_the_same_packet() {
    for protocol in [Protocol::Quake2_34, Protocol::Quake2Repro1038] {
        let (_fixture, mut host, mut audio) = setup(protocol);
        let owner = host.runtime.sound_assets.as_ref().unwrap();
        assert!(!owner.initial_registration());
        let (reads, decodes, initial_names) = (owner.reads, owner.decodes, owner.bank().len());
        let range = protocol.resource_layout().resources[1];
        let mut packet = frames::config(protocol, range.get(1).unwrap(), b"missing.wav");
        packet.extend(sound(protocol, 1));
        packet.extend(frames::config(protocol, range.get(2).unwrap(), b"late.wav"));
        packet.extend(sound(protocol, 2));
        frames::queue(&mut host, [packet.clone()]);
        let frame = host.frame(&mut audio, true);
        assert_eq!((frame.output.sounds, frame.output.unhandled_sounds), (1, 0));
        let owner = host.runtime.sound_assets.as_ref().unwrap();
        let late = owner.bank().find("sound/late.wav").unwrap();
        assert!(late.0 as usize >= initial_names);
        assert_eq!((owner.reads, owner.decodes), (reads + 2, decodes + 1));
        assert_eq!(audio.events.len(), 1);
        assert_eq!(audio.events[0].0, late);
        assert_eq!(audio.events[0].1, Some(2));
        let resources = &host.runtime.client_resources[0].as_ref().unwrap().state;
        assert_eq!(resources.sound(1), None);
        assert_eq!(resources.sound(2), Some(late));
        let mut pcm = [0; 8];
        assert!(audio.mixer.paint(owner.bank(), &mut pcm));
        assert!(pcm.iter().any(|&sample| sample != 0));
        // The same accepted rows and known missing cue do no new I/O.
        frames::queue(&mut host, [packet]);
        let frame = host.frame(&mut audio, true);
        assert_eq!((frame.output.sounds, frame.output.unhandled_sounds), (1, 0));
        let owner = host.runtime.sound_assets.as_ref().unwrap();
        assert_eq!((owner.reads, owner.decodes), (reads + 2, decodes + 1));
        assert_eq!(audio.events.len(), 2);
        assert_eq!(audio.events[1].0, late);
    }
}

#[test]
fn console_play_admits_native_names_after_startup_and_reuses_the_same_sample() {
    for source in [
        RuleSetId::Quake,
        RuleSetId::QuakeWorld,
        RuleSetId::Quake2,
        RuleSetId::Quake2Rerelease,
        RuleSetId::Quake3,
    ] {
        let mut fixture = files::Fixture::new(&[("sound/misc/late.wav", samples::wav())]);
        let mut runtime = Runtime::load(1, []).unwrap();
        runtime.vfs = std::mem::take(&mut fixture.vfs);
        let context = Context {
            source,
            ..Default::default()
        };
        let mut host = FrameHost::load(
            Console::new(context).unwrap(),
            runtime,
            TickRate::FrameDriven,
            vec![],
        )
        .unwrap();
        qa_app::audio::begin_registration(&mut host, &[], source).unwrap();
        qa_app::audio::finish_registration(&mut host).unwrap();
        let mut audio = Audio {
            mixer: Mixer::load(NonZeroU32::new(44100).unwrap(), 8, 1),
            events: Vec::new(),
        };
        let command = if source == RuleSetId::Quake3 {
            "play sound/misc/late"
        } else {
            "play misc/late"
        };
        let mut reads_after_first = None;
        for repeat in [false, true] {
            host.console.append_line(command, context).unwrap();
            let frame = host.frame(&mut audio, true);
            assert_eq!((frame.output.sounds, frame.output.unhandled_sounds), (1, 0));
            let owner = host.runtime.sound_assets.as_ref().unwrap();
            if let Some(reads) = reads_after_first {
                assert_eq!(owner.reads, reads);
            } else {
                assert!(owner.reads > 0);
                reads_after_first = Some(owner.reads);
            }
            assert_eq!(
                audio.events.last().unwrap().0,
                owner.bank().find("sound/misc/late.wav").unwrap()
            );
            let mut pcm = [0; 8];
            assert!(audio.mixer.paint(owner.bank(), &mut pcm));
            assert!(pcm.iter().any(|&sample| sample != 0));
            if !repeat {
                fixture.remove("sound/misc/late.wav");
            }
        }
        assert_eq!(audio.events.len(), 2);
        assert_eq!(audio.events[0].0, audio.events[1].0);
    }
}

#[test]
fn q3_console_registration_rejects_stereo_without_silencing_a_later_valid_name() {
    let mut fixture = files::Fixture::new(&[
        ("sound/stereo.wav", samples::wav_channels(2)),
        ("sound/mono.wav", samples::wav()),
    ]);
    let mut runtime = Runtime::load(1, []).unwrap();
    runtime.vfs = std::mem::take(&mut fixture.vfs);
    let context = Context {
        source: RuleSetId::Quake3,
        ..Default::default()
    };
    let mut host = FrameHost::load(
        Console::new(context).unwrap(),
        runtime,
        TickRate::FrameDriven,
        vec![],
    )
    .unwrap();
    qa_app::audio::begin_registration(&mut host, &[], RuleSetId::Quake3).unwrap();
    qa_app::audio::finish_registration(&mut host).unwrap();
    let mut audio = Audio {
        mixer: Mixer::load(NonZeroU32::new(44100).unwrap(), 8, 1),
        events: Vec::new(),
    };
    host.console
        .append_line("play sound/stereo sound/mono", context)
        .unwrap();
    let frame = host.frame(&mut audio, true);
    assert_eq!((frame.output.sounds, frame.output.unhandled_sounds), (1, 0));
    let bank = host.runtime.sound_bank().unwrap();
    assert!(bank.find("sound/stereo.wav").is_some());
    assert_eq!(audio.events.len(), 1);
    assert_eq!(audio.events[0].0, bank.find("sound/mono.wav").unwrap());
    let mut pcm = [0; 8];
    assert!(audio.mixer.paint(bank, &mut pcm));
    assert!(pcm.iter().any(|&sample| sample != 0));
}
