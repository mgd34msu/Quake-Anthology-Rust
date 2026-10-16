use qa_app::audio::music::Music;
use qa_audio::{Bank, Mixer};
use qa_content::vfs::Vfs;
use qa_core::primitives::*;
use std::{
    num::NonZeroU32,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
#[global_allocator]
static ALLOCATOR: qa_platform::allocations::CountingAllocator =
    qa_platform::allocations::CountingAllocator;
const MONO: &[u8] = include_bytes!("../../formats/tests/fixtures/sounds/mono.ogg");
const STEREO: &[u8] = include_bytes!("../../formats/tests/fixtures/sounds/stereo.ogg");
const PAGES: &[u8] = include_bytes!("../../formats/tests/fixtures/sounds/pages.ogg");
pub struct Fixture {
    root: std::path::PathBuf,
    pub vfs: Vfs,
}
impl Fixture {
    pub fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "qa-rust-music-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        qa_platform::files::create_dir_all(root.join("music")).unwrap();
        for (name, samples) in [
            ("intro.wav", [1000, -1000, 2000, -2000]),
            ("repeat.wav", [3000, -3000, 4000, -4000]),
        ] {
            let mut bytes = b"RIFF\0\0\0\0WAVEfmt \x10\0\0\0".to_vec();
            bytes.extend(1_u16.to_le_bytes());
            bytes.extend(2_u16.to_le_bytes());
            bytes.extend(22050_u32.to_le_bytes());
            bytes.extend(88200_u32.to_le_bytes());
            bytes.extend(4_u16.to_le_bytes());
            bytes.extend(16_u16.to_le_bytes());
            bytes.extend(b"data");
            bytes.extend(8_u32.to_le_bytes());
            for value in samples {
                bytes.extend((value as i16).to_le_bytes());
            }
            let length = bytes.len() as u32 - 8;
            bytes[4..8].copy_from_slice(&length.to_le_bytes());
            qa_platform::files::write(root.join("music").join(name), bytes).unwrap();
        }
        for (name, bytes) in [
            ("mono.ogg", MONO),
            ("stereo.ogg", STEREO),
            ("pages.ogg", PAGES),
        ] {
            qa_platform::files::write(root.join("music").join(name), bytes).unwrap();
        }
        let mut vfs = Vfs::default();
        vfs.mount_directory(&root, 1).unwrap();
        Self { root, vfs }
    }
    pub fn archive(&mut self) {
        let path = self.root.join("tracks.pk3");
        qa_platform::files::write(&path, include_bytes!("fixtures/music/tracks.pk3")).unwrap();
        let file = Arc::new(qa_platform::files::ReadFile::open(&path).unwrap());
        let archive = Arc::new(qa_formats::archive::Archive::parse(file).unwrap());
        self.vfs.mount_archive(&path, 10, archive).unwrap();
    }
    pub fn numbered(&mut self) {
        for (name, bytes) in [
            ("02.ogg", MONO),
            ("track02.ogg", STEREO),
            ("04.ogg", &b"broken"[..]),
            ("track04.ogg", MONO),
            ("choice.ogg", MONO),
            ("fallback.ogg", MONO),
            ("fallback.wav", &b"broken"[..]),
            ("12-theme.ogg", MONO),
        ] {
            qa_platform::files::write(self.root.join("music").join(name), bytes).unwrap();
        }
        let wave = qa_platform::files::read(self.root.join("music/intro.wav")).unwrap();
        qa_platform::files::write(self.root.join("music/track03.wav"), wave).unwrap();
        let wave = qa_platform::files::read(self.root.join("music/intro.wav")).unwrap();
        qa_platform::files::write(self.root.join("music/choice.wav"), wave).unwrap();
        let intro = qa_platform::files::read(self.root.join("music/intro.wav")).unwrap();
        let loop_track = qa_platform::files::read(self.root.join("music/repeat.wav")).unwrap();
        qa_platform::files::write(self.root.join("music/intro theme.wav"), intro).unwrap();
        qa_platform::files::write(self.root.join("music/loop theme.wav"), loop_track).unwrap();
        self.vfs.mount_directory(&self.root, 2).unwrap();
    }
}

#[test]
fn named_track_extension_order_is_source_data_with_fallback_to_valid_prepared_audio() {
    let bank = bank();
    let mut fixture = Fixture::new();
    fixture.numbered();
    let mono = qa_formats::sound::vorbis(MONO).unwrap();
    for rules in [
        RuleSetId::Quake,
        RuleSetId::QuakeWorld,
        RuleSetId::Quake2,
        RuleSetId::Quake2Rerelease,
        RuleSetId::Quake3,
    ] {
        for name in [
            &b"music/choice"[..],
            &b"music/fallback"[..],
            &b"music/12-theme.ogg"[..],
        ] {
            let mut music = Music::load(&fixture.vfs);
            let mut mix = mixer();
            let playback = qa_gameplay::rules::music_playback(rules);
            let mut control = start(owner(1), false, false);
            if let MusicControl::Start {
                playback: actual, ..
            } = &mut control
            {
                *actual = playback;
            }
            assert!(music.control(&fixture.vfs, &mut mix, control, Some(name), None));
            music.update(&fixture.vfs, &mut mix, 1.);
            let mut actual = [0; 8];
            mix.paint(&bank, &mut actual);
            let wave = rules == RuleSetId::Quake3 && name == b"music/choice";
            if wave {
                assert_eq!(actual, [625, -625, 1250, -1250, 0, 0, 0, 0]);
            } else {
                let mut reference = mixer();
                let bytes: Vec<_> = mono.samples.iter().flat_map(|x| x.to_le_bytes()).collect();
                reference
                    .queue_raw(
                        PcmFormat {
                            rate: mono.rate,
                            channels: mono.channels,
                            width: PcmWidth::Short,
                        },
                        &bytes,
                        if playback.smooth_volume { 0.625 } else { 1. },
                        playback.signed_stereo_bytes,
                    )
                    .unwrap();
                let mut expected = [0; 8];
                reference.paint(&bank, &mut expected);
                assert_eq!(actual, expected, "{rules:?}, {name:?}");
            }
            assert_eq!(music.counts.invalid, 0);
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        qa_platform::files::remove_dir_all(&self.root).unwrap();
    }
}
pub fn bank() -> Bank {
    Bank::load(NonZeroU32::new(22050).unwrap(), vec![]).unwrap()
}
pub fn mixer() -> Mixer {
    Mixer::load(NonZeroU32::new(22050).unwrap(), 4, 1)
}
fn owner(generation: u32) -> MusicOwner {
    MusicOwner {
        module: Some(ModuleId(1)),
        client: Some((
            ClientId(2),
            EntityId {
                slot: 8,
                generation,
            },
        )),
    }
}
fn start(owner: MusicOwner, preserve: bool, smooth: bool) -> MusicControl {
    let text = TextId {
        slot: 0,
        generation: 0,
    };
    MusicControl::Start {
        owner,
        playback: MusicPlayback {
            wave_first: preserve,
            smooth_volume: smooth,
            preserve_queued: preserve,
            signed_stereo_bytes: true,
        },
        intro: text,
        loop_track: text,
    }
}

#[test]
fn numbered_tracks_use_prepared_vfs_priority_and_do_not_restart_on_repeated_requests() {
    let bank = bank();
    let mut fixture = Fixture::new();
    fixture.numbered();
    let expected = qa_formats::sound::vorbis(MONO).unwrap();
    let mut music = Music::load(&fixture.vfs);
    let mut mix = mixer();
    let control = |number, looping| MusicControl::Track {
        owner: owner(1),
        playback: qa_gameplay::rules::music_playback(RuleSetId::Quake),
        number,
        looping,
    };
    assert!(music.control(&fixture.vfs, &mut mix, control(2, true), None, None));
    music.update(&fixture.vfs, &mut mix, 1.);
    let mut pcm = [0; 16];
    mix.paint(&bank, &mut pcm);
    assert_eq!(pcm, std::array::from_fn(|i| expected.samples[i / 2]));
    let queued = mix.raw_remaining();
    assert!(music.control(&fixture.vfs, &mut mix, control(2, true), None, None));
    assert_eq!(mix.raw_remaining(), queued);
    assert_eq!(music.counts.started, 1);
    assert!(music.control(&fixture.vfs, &mut mix, control(0, true), None, None));
    assert!(!music.playing());
    mix.paint(&bank, &mut pcm);
    assert_eq!(pcm, [0; 16]);
    // A malformed first candidate is skipped in favor of the valid next file.
    assert!(music.control(&fixture.vfs, &mut mix, control(4, false), None, None));
    music.update(&fixture.vfs, &mut mix, 1.);
    mix.paint(&bank, &mut pcm);
    assert_eq!(pcm, std::array::from_fn(|i| expected.samples[i / 2]));
    assert!(music.control(&fixture.vfs, &mut mix, control(3, false), None, None));
    music.update(&fixture.vfs, &mut mix, 1.);
    let mut wave = [0; 8];
    mix.paint(&bank, &mut wave);
    assert_eq!(wave, [1000, -1000, 2000, -2000, 0, 0, 0, 0]);
    assert!(!music.playing());
}

#[test]
fn dynamic_tracks_bypass_effect_registration_and_switch_intro_to_loop() {
    let bank = bank();
    let fixture = Fixture::new();
    let mut mix = mixer();
    assert_eq!(bank.find("music/intro.wav"), None);
    let mut music = Music::load(&fixture.vfs);
    assert!(music.control(
        &fixture.vfs,
        &mut mix,
        start(owner(1), false, false),
        Some(b"music/intro.wav"),
        Some(b"music/repeat.wav")
    ));
    music.update(&fixture.vfs, &mut mix, 1.);
    let mut out = [0; 12];
    mix.paint(&bank, &mut out);
    assert_eq!(
        out,
        [
            1000, -1000, 2000, -2000, 3000, -3000, 4000, -4000, 3000, -3000, 4000, -4000
        ]
    );
    assert_eq!(music.counts.invalid, 0);
    assert!(music.counts.loops > 0);
    assert_eq!(mix.raw_remaining(), 6);
    music.control(
        &fixture.vfs,
        &mut mix,
        MusicControl::Stop { only_active: false },
        None,
        None,
    );
    mix.paint(&bank, &mut out);
    assert_eq!(out, [0; 12]);
}

#[test]
fn native_volume_updates_once_and_raw_queue_survives_failed_replacement() {
    let bank = bank();
    let fixture = Fixture::new();
    let mut mix = mixer();
    let mut music = Music::load(&fixture.vfs);
    assert!(music.control(
        &fixture.vfs,
        &mut mix,
        start(owner(1), true, true),
        Some(b"music/intro.wav"),
        None
    ));
    music.update(&fixture.vfs, &mut mix, 0.5);
    assert!(!music.playing());
    assert!(!music.control(
        &fixture.vfs,
        &mut mix,
        start(owner(2), true, true),
        Some(b"music/missing.wav"),
        None
    ));
    music.control(
        &fixture.vfs,
        &mut mix,
        MusicControl::Stop { only_active: true },
        None,
        None,
    );
    let mut out = [0; 6];
    mix.paint(&bank, &mut out);
    assert_eq!(out, [375, -375, 750, -750, 0, 0]);
    assert_eq!(music.counts.invalid, 1);
}

#[test]
fn loose_vorbis_streams_exact_authored_samples_and_keeps_pause_position() {
    let bank = bank();
    let fixture = Fixture::new();
    let expected = qa_formats::sound::vorbis(MONO).unwrap();
    let mut mix = mixer();
    let mut music = Music::load(&fixture.vfs);
    assert!(music.control(
        &fixture.vfs,
        &mut mix,
        start(owner(1), false, false),
        Some(b"music/mono.ogg"),
        None
    ));
    music.update(&fixture.vfs, &mut mix, 1.);
    let mut pair = [0; 2];
    mix.paint(&bank, &mut pair);
    assert_eq!(pair, [expected.samples[0]; 2]);
    music.control(
        &fixture.vfs,
        &mut mix,
        MusicControl::Pause(true),
        None,
        None,
    );
    mix.paint(&bank, &mut pair);
    assert_eq!(pair, [0; 2]);
    music.control(
        &fixture.vfs,
        &mut mix,
        MusicControl::Pause(false),
        None,
        None,
    );
    let mut actual = vec![0; expected.samples.len() * 2];
    mix.paint(&bank, &mut actual);
    let mut remaining: Vec<_> = expected.samples[1..]
        .iter()
        .flat_map(|&sample| [sample; 2])
        .collect();
    remaining.extend([0; 2]);
    assert_eq!(actual, remaining);
    assert!(!music.playing());
    assert_eq!(music.counts.invalid, 0);
}

#[test]
fn deflated_vorbis_with_arbitrary_name_uses_the_same_pcm_blocks_as_wave() {
    let bank = bank();
    let mut fixture = Fixture::new();
    fixture.archive();
    let expected = qa_formats::sound::vorbis(STEREO).unwrap();
    let mut music = Music::load(&fixture.vfs);
    let mut actual_mix = mixer();
    let mut expected_mix = mixer();
    assert!(music.control(
        &fixture.vfs,
        &mut actual_mix,
        start(owner(1), false, false),
        Some(b"scores/theme.data"),
        None
    ));
    music.update(&fixture.vfs, &mut actual_mix, 1.);
    let bytes: Vec<_> = expected
        .samples
        .iter()
        .flat_map(|sample| sample.to_le_bytes())
        .collect();
    let format = PcmFormat {
        rate: expected.rate,
        channels: expected.channels,
        width: PcmWidth::Short,
    };
    for block in bytes.chunks(30000) {
        expected_mix.queue_raw(format, block, 1., false).unwrap();
    }
    let mut actual = [0; 12000];
    let mut reference = [0; 12000];
    actual_mix.paint(&bank, &mut actual);
    expected_mix.paint(&bank, &mut reference);
    assert_eq!(actual, reference);
    assert!(!music.playing());
    assert_eq!(music.counts.invalid, 0);
}

#[test]
fn vorbis_intro_to_wave_loop_preserves_the_boundary_and_voice_bank() {
    let bank = bank();
    let fixture = Fixture::new();
    let expected = qa_formats::sound::vorbis(MONO).unwrap();
    let mut music = Music::load(&fixture.vfs);
    let mut mix = mixer();
    assert!(music.control(
        &fixture.vfs,
        &mut mix,
        start(owner(1), false, false),
        Some(b"music/mono.ogg"),
        Some(b"music/repeat.wav")
    ));
    music.update(&fixture.vfs, &mut mix, 1.);
    let mut actual = vec![0; expected.samples.len() * 2 + 8];
    mix.paint(&bank, &mut actual);
    let mut reference: Vec<_> = expected
        .samples
        .iter()
        .flat_map(|&sample| [sample; 2])
        .collect();
    reference.extend([3000, -3000, 4000, -4000, 3000, -3000, 4000, -4000]);
    assert_eq!(actual, reference);
    assert_eq!(bank.find("music/mono.ogg"), None);
    assert_eq!(music.counts.invalid, 0);
}

#[test]
fn multi_page_vorbis_keeps_partial_packets_across_presentation_refills() {
    let fixture = Fixture::new();
    let expected = qa_formats::sound::vorbis(PAGES).unwrap();
    let bank = Bank::load(expected.rate, vec![]).unwrap();
    let mut mix = Mixer::load(expected.rate, 4, 1);
    let mut music = Music::load(&fixture.vfs);
    assert!(music.control(
        &fixture.vfs,
        &mut mix,
        start(owner(1), false, false),
        Some(b"music/pages.ogg"),
        None
    ));
    let mut buffer = [0; 8192];
    let mut position = 0;
    let mut iteration = 0;
    while position < expected.frames() + 20 {
        music.update(&fixture.vfs, &mut mix, 1.);
        let frames = [127, 512, 2048, 4096, 333][iteration % 5];
        let frames = frames.min(expected.frames() + 20 - position);
        mix.paint(&bank, &mut buffer[..frames * 2]);
        for frame in 0..frames {
            let sample = expected.samples.get(position + frame).copied().unwrap_or(0);
            assert_eq!(&buffer[frame * 2..frame * 2 + 2], &[sample; 2]);
        }
        position += frames;
        iteration += 1;
    }
    assert!(iteration > 10);
    assert!(!music.playing());
    assert_eq!(music.counts.invalid, 0);
}

#[test]
fn console_vorbis_path_reaches_the_mixer_through_the_shared_ring() {
    let bank = bank();
    let mut fixture = Fixture::new();
    let mut runtime = qa_app::Runtime::load(1, []).unwrap();
    runtime.vfs = std::mem::take(&mut fixture.vfs);
    let context = qa_console::views::Context {
        source: RuleSetId::Quake3,
        ..Default::default()
    };
    let mut console = qa_console::commands::Console::new(context).unwrap();
    console.register("music", qa_app::audio::music);
    let mut mix = mixer();
    let mut music = Music::load(&runtime.vfs);
    console_control(
        &mut runtime,
        &mut console,
        &mut music,
        &mut mix,
        "music once mono.ogg",
        context,
    );
    music.update(&runtime.vfs, &mut mix, 1.);
    let expected = qa_formats::sound::vorbis(MONO).unwrap();
    let mut output = [0; 8];
    mix.paint(&bank, &mut output);
    let mut reference = mixer();
    let bytes: Vec<_> = expected
        .samples
        .iter()
        .flat_map(|sample| sample.to_le_bytes())
        .collect();
    reference
        .queue_raw(
            PcmFormat {
                rate: expected.rate,
                channels: expected.channels,
                width: PcmWidth::Short,
            },
            &bytes,
            0.625,
            false,
        )
        .unwrap();
    let mut reference_output = [0; 8];
    reference.paint(&bank, &mut reference_output);
    assert_eq!(output, reference_output);
    assert_eq!(music.counts.invalid, 0);
}

#[test]
fn prepared_archive_tracks_start_refill_pause_loop_and_stop_without_heap_growth() {
    let bank = bank();
    let mut fixture = Fixture::new();
    fixture.archive();
    let mut music = Music::load(&fixture.vfs);
    let mut mix = mixer();
    let mut output = [0; 8192];
    qa_platform::allocations::begin_frame();
    assert!(music.control(
        &fixture.vfs,
        &mut mix,
        start(owner(1), false, false),
        Some(b"scores/theme.data"),
        Some(b"music/repeat.wav")
    ));
    for _ in 0..20 {
        music.update(&fixture.vfs, &mut mix, 1.);
        mix.paint(&bank, &mut output);
    }
    music.control(
        &fixture.vfs,
        &mut mix,
        MusicControl::Pause(true),
        None,
        None,
    );
    music.update(&fixture.vfs, &mut mix, 1.);
    mix.paint(&bank, &mut output);
    music.control(
        &fixture.vfs,
        &mut mix,
        MusicControl::Pause(false),
        None,
        None,
    );
    music.update(&fixture.vfs, &mut mix, 1.);
    mix.paint(&bank, &mut output);
    music.control(
        &fixture.vfs,
        &mut mix,
        MusicControl::Stop { only_active: false },
        None,
        None,
    );
    let counts = qa_platform::allocations::end_frame();
    assert_eq!(counts, qa_platform::allocations::Counts::default());
    assert_eq!(music.counts.invalid, 0);
    assert!(music.counts.loops > 0);
}

#[test]
fn retirement_does_not_stop_a_reused_client_or_a_new_module_owner() {
    let bank = bank();
    let fixture = Fixture::new();
    let mut mix = mixer();
    let mut music = Music::load(&fixture.vfs);
    music.control(
        &fixture.vfs,
        &mut mix,
        start(owner(2), false, false),
        Some(b"music/repeat.wav"),
        Some(b"music/repeat.wav"),
    );
    music.control(
        &fixture.vfs,
        &mut mix,
        MusicControl::Retire(owner(1)),
        None,
        None,
    );
    music.sound_control(
        &mut mix,
        SoundEvent::retire_client(
            ClientId(2),
            EntityId {
                slot: 8,
                generation: 1,
            },
        ),
    );
    assert!(music.playing());
    music.update(&fixture.vfs, &mut mix, 1.);
    music.control(
        &fixture.vfs,
        &mut mix,
        MusicControl::Retire(owner(2)),
        None,
        None,
    );
    let mut out = [0; 4];
    mix.paint(&bank, &mut out);
    assert_eq!(out, [0; 4]);
    assert!(!music.playing());
}

fn console_control(
    runtime: &mut qa_app::Runtime,
    console: &mut qa_console::commands::Console<qa_app::Runtime>,
    music: &mut Music,
    mix: &mut Mixer,
    line: &str,
    context: qa_console::views::Context,
) -> Option<MusicControl> {
    use qa_core::events::{FrameEvent, OutputSubmission, dispatch_consumer};
    console.append_line(line, context).unwrap();
    console.execute_frame(runtime);
    let mut last = None;
    let consumer = runtime.server.presentation;
    dispatch_consumer(
        &mut runtime.server.events,
        consumer,
        &mut |record: qa_core::events::OutputRecord, texts: &mut qa_core::events::TextStore| {
            if let FrameEvent::Music(control) = record.event {
                let paths = match control {
                    MusicControl::Start {
                        owner,
                        intro,
                        loop_track,
                        ..
                    } => {
                        assert_eq!(
                            owner,
                            MusicOwner {
                                module: None,
                                client: None
                            }
                        );
                        (texts.get(intro), texts.get(loop_track))
                    }
                    MusicControl::Remap { entries } => (texts.get(entries), None),
                    _ => (None, None),
                };
                assert!(music.control(&runtime.vfs, mix, control, paths.0, paths.1));
                last = Some(control);
            }
            OutputSubmission::BestEffort
        },
    );
    last
}

#[test]
fn console_cd_controls_keep_one_player_and_apply_atomic_remaps() {
    let bank = bank();
    let mut fixture = Fixture::new();
    fixture.numbered();
    let mut runtime = qa_app::Runtime::load(1, []).unwrap();
    runtime.vfs = std::mem::take(&mut fixture.vfs);
    let context = qa_console::views::Context {
        source: RuleSetId::Quake2,
        ..Default::default()
    };
    let mut console = qa_console::commands::Console::new(context).unwrap();
    console.register("cd", qa_app::audio::cd);
    console.register("music", qa_app::audio::music);
    let mut mix = mixer();
    let mut music = Music::load(&runtime.vfs);
    macro_rules! run {
        ($line:expr) => {
            console_control(
                &mut runtime,
                &mut console,
                &mut music,
                &mut mix,
                $line,
                context,
            )
        };
    }
    assert!(matches!(
        run!("cd remap 3 0 2"),
        Some(MusicControl::Remap { .. })
    ));
    assert_eq!(
        music.report(&runtime.vfs, MusicQuery::Remaps).to_string(),
        "  1 -> 3\n  2 -> 0\n  3 -> 2\n"
    );
    assert!(matches!(
        run!("cd loop 1"),
        Some(MusicControl::Track { looping: true, .. })
    ));
    music.update(&runtime.vfs, &mut mix, 1.);
    let mut pcm = [0; 8];
    mix.paint(&bank, &mut pcm);
    assert_eq!(pcm, [1000, -1000, 2000, -2000, 1000, -1000, 2000, -2000]);
    assert_eq!(
        music
            .report(&runtime.vfs, MusicQuery::Info { volume: 1. })
            .to_string(),
        "Currently looping track 1 (mapped to 3)\nVolume is 1\n"
    );
    run!("cd pause");
    mix.paint(&bank, &mut pcm);
    assert_eq!(pcm, [0; 8]);
    run!("cd resume");
    mix.paint(&bank, &mut pcm);
    assert_ne!(pcm, [0; 8]);
    run!("cd off");
    let started = music.counts.started;
    run!("cd play 3");
    run!("music mono.ogg");
    run!("cd resume");
    assert!(!music.playing());
    assert_eq!(music.counts.started, started);
    run!("cd on");
    assert!(!music.playing());
    run!("cd loop 1");
    assert!(music.playing());
    assert!(run!("cd remap 2 bad").is_none());
    assert!(run!("cd remap 4294967296").is_none());
    assert_eq!(
        music.report(&runtime.vfs, MusicQuery::Remaps).to_string(),
        "  1 -> 3\n  2 -> 0\n  3 -> 2\n"
    );
    run!("cd loop 2");
    assert!(!music.playing(), "mapped zero disables this track");
    run!("cd off");
    run!("cd reset");
    assert_eq!(
        music.report(&runtime.vfs, MusicQuery::Remaps).to_string(),
        "CD track remap is identity.\n"
    );
    run!("cd play 2");
    assert!(music.playing());
    assert_eq!(music.counts.invalid, 0);
}

#[test]
fn full_native_remap_range_is_available_through_every_engine_console_profile() {
    for rules in [
        RuleSetId::Quake,
        RuleSetId::QuakeWorld,
        RuleSetId::Quake2,
        RuleSetId::Quake2Rerelease,
        RuleSetId::Quake3,
    ] {
        let mut fixture = Fixture::new();
        fixture.numbered();
        let mut runtime = qa_app::Runtime::load(1, []).unwrap();
        runtime.vfs = std::mem::take(&mut fixture.vfs);
        let context = qa_console::views::Context {
            source: rules,
            ..Default::default()
        };
        let mut console = qa_console::commands::Console::new(context).unwrap();
        console.register("cd", qa_app::audio::cd);
        let mut music = Music::load(&runtime.vfs);
        let mut mix = mixer();
        let mut line = String::from("cd remap");
        for _ in 1..MUSIC_REMAP_TRACKS {
            line.push_str(" 4294967295");
        }
        assert!(line.len() > 1024);
        assert!(matches!(
            console_control(
                &mut runtime,
                &mut console,
                &mut music,
                &mut mix,
                &line,
                context
            ),
            Some(MusicControl::Remap { .. })
        ));
        let report = music.report(&runtime.vfs, MusicQuery::Remaps).to_string();
        assert_eq!(report.lines().count(), MUSIC_REMAP_TRACKS - 1);
        assert!(report.ends_with("  255 -> 4294967295\n"));
        line.push_str(" 3");
        assert!(
            console_control(
                &mut runtime,
                &mut console,
                &mut music,
                &mut mix,
                &line,
                context
            )
            .is_none()
        );
        assert_eq!(
            music.report(&runtime.vfs, MusicQuery::Remaps).to_string(),
            report
        );
    }
}
#[test]
fn console_music_paths_pause_resume_and_once_use_the_shared_ring() {
    let bank = bank();
    let mut fixture = Fixture::new();
    let mut runtime = qa_app::Runtime::load(1, []).unwrap();
    runtime.vfs = std::mem::take(&mut fixture.vfs);
    let context = qa_console::views::Context {
        source: RuleSetId::Quake2,
        ..Default::default()
    };
    let mut console = qa_console::commands::Console::new(context).unwrap();
    console.register("music", qa_app::audio::music);
    let mut mix = mixer();
    let mut music = Music::load(&runtime.vfs);
    assert!(matches!(
        console_control(
            &mut runtime,
            &mut console,
            &mut music,
            &mut mix,
            "music intro repeat",
            context
        ),
        Some(MusicControl::Start { .. })
    ));
    music.update(&runtime.vfs, &mut mix, 1.);
    let mut pcm = [0; 2];
    mix.paint(&bank, &mut pcm);
    assert_eq!(pcm, [1000, -1000]);
    assert!(matches!(
        console_control(
            &mut runtime,
            &mut console,
            &mut music,
            &mut mix,
            "music PAUSE",
            context
        ),
        Some(MusicControl::Pause(true))
    ));
    music.update(&runtime.vfs, &mut mix, 1.);
    mix.paint(&bank, &mut pcm);
    assert_eq!(pcm, [0; 2]);
    assert!(matches!(
        console_control(
            &mut runtime,
            &mut console,
            &mut music,
            &mut mix,
            "music resume",
            context
        ),
        Some(MusicControl::Pause(false))
    ));
    mix.paint(&bank, &mut pcm);
    assert_eq!(pcm, [2000, -2000]);
    music.control(
        &runtime.vfs,
        &mut mix,
        MusicControl::Retire(MusicOwner {
            module: Some(ModuleId(0)),
            client: None,
        }),
        None,
        None,
    );
    assert!(music.playing());
    assert!(matches!(
        console_control(
            &mut runtime,
            &mut console,
            &mut music,
            &mut mix,
            "music stop",
            context
        ),
        Some(MusicControl::Stop { only_active: false })
    ));
    mix.paint(&bank, &mut pcm);
    assert_eq!(pcm, [0; 2]);
    assert!(matches!(
        console_control(
            &mut runtime,
            &mut console,
            &mut music,
            &mut mix,
            "music once intro",
            context
        ),
        Some(MusicControl::Start { .. })
    ));
    music.update(&runtime.vfs, &mut mix, 1.);
    let mut output = [0; 6];
    mix.paint(&bank, &mut output);
    assert_eq!(output, [1000, -1000, 2000, -2000, 0, 0]);
    assert!(!music.playing());
}

#[test]
fn console_bare_music_names_reach_the_same_source_selected_resolver() {
    let bank = bank();
    for rules in [RuleSetId::Quake2, RuleSetId::Quake3] {
        let mut fixture = Fixture::new();
        fixture.numbered();
        let mut runtime = qa_app::Runtime::load(1, []).unwrap();
        runtime.vfs = std::mem::take(&mut fixture.vfs);
        let context = qa_console::views::Context {
            source: rules,
            ..Default::default()
        };
        let mut console = qa_console::commands::Console::new(context).unwrap();
        console.register("music", qa_app::audio::music);
        let mut music = Music::load(&runtime.vfs);
        let mut mix = mixer();
        let control = console_control(
            &mut runtime,
            &mut console,
            &mut music,
            &mut mix,
            "music choice",
            context,
        )
        .unwrap();
        let MusicControl::Start { intro, .. } = control else {
            panic!("start");
        };
        assert_eq!(
            runtime.server.events.texts.get(intro),
            None,
            "ring paths retired after consumption"
        );
        music.update(&runtime.vfs, &mut mix, 1.);
        let mut actual = [0; 8];
        mix.paint(&bank, &mut actual);
        if rules == RuleSetId::Quake3 {
            assert_eq!(actual, [625, -625, 1250, -1250, 625, -625, 1250, -1250]);
        } else {
            let mono = qa_formats::sound::vorbis(MONO).unwrap();
            assert_eq!(actual, std::array::from_fn(|i| mono.samples[i / 2]));
        }
        assert_eq!(music.counts.invalid, 0);
    }
}

#[test]
fn presentation_reset_keeps_peer_music_and_requires_the_exact_client_lifetime() {
    let bank = bank();
    let fixture = Fixture::new();
    let mut mix = mixer();
    let mut music = Music::load(&fixture.vfs);
    music.control(
        &fixture.vfs,
        &mut mix,
        start(owner(2), false, false),
        Some(b"music/repeat.wav"),
        Some(b"music/repeat.wav"),
    );
    let (client, lifetime) = owner(2).client.unwrap();
    music.update(&fixture.vfs, &mut mix, 1.0);
    music.reset_client(&mut mix, ClientId(client.0 + 1), lifetime);
    music.reset_client(&mut mix, client, owner(1).client.unwrap().1);
    assert!(music.playing());
    assert_eq!(music.counts.stopped, 0);
    let mut pcm = [0; 8];
    assert!(mix.paint(&bank, &mut pcm));
    assert!(pcm.iter().any(|&value| value != 0));
    music.reset_client(&mut mix, client, lifetime);
    assert!(!music.playing());
    assert_eq!(music.counts.stopped, 1);
    assert!(mix.paint(&bank, &mut pcm));
    assert_eq!(pcm, [0; 8]);
    music.control(
        &fixture.vfs,
        &mut mix,
        start(owner(3), false, false),
        Some(b"music/repeat.wav"),
        Some(b"music/repeat.wav"),
    );
    music.reset_client(&mut mix, client, lifetime);
    assert!(music.playing());
    let mut release = SoundEvent {
        action: SoundAction::ReleaseListener,
        client: Some(client),
        client_lifetime: owner(3).client.map(|(_, lifetime)| lifetime),
        listener_owner: Some(ModuleId(99)),
        ..Default::default()
    };
    music.sound_control(&mut mix, release);
    assert!(music.playing());
    release.listener_owner = owner(3).module;
    music.sound_control(&mut mix, release);
    assert!(!music.playing());
    assert_eq!(music.counts.stopped, 2);
}
