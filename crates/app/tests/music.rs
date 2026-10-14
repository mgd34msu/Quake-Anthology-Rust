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
        let mut vfs = Vfs::default();
        vfs.mount_directory(&root, 1).unwrap();
        Self { root, vfs }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        qa_platform::files::remove_dir_all(&self.root).unwrap();
    }
}
pub fn mixer() -> Mixer {
    Mixer::load(
        Arc::new(Bank::load(NonZeroU32::new(22050).unwrap(), vec![]).unwrap()),
        4,
        1,
    )
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
            smooth_volume: smooth,
            preserve_queued: preserve,
            signed_stereo_bytes: true,
        },
        intro: text,
        loop_track: text,
    }
}

#[test]
fn dynamic_tracks_bypass_effect_registration_and_switch_intro_to_loop() {
    let fixture = Fixture::new();
    let mut mix = mixer();
    assert_eq!(mix.bank.find("music/intro.wav"), None);
    let mut music = Music::default();
    assert!(music.control(
        &fixture.vfs,
        &mut mix,
        start(owner(1), false, false),
        Some(b"music/intro.wav"),
        Some(b"music/repeat.wav")
    ));
    music.update(&fixture.vfs, &mut mix, 1.);
    let mut out = [0; 12];
    mix.paint(&mut out);
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
    mix.paint(&mut out);
    assert_eq!(out, [0; 12]);
}

#[test]
fn native_volume_updates_once_and_raw_queue_survives_failed_replacement() {
    let fixture = Fixture::new();
    let mut mix = mixer();
    let mut music = Music::default();
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
    mix.paint(&mut out);
    assert_eq!(out, [375, -375, 750, -750, 0, 0]);
    assert_eq!(music.counts.invalid, 1);
}

#[test]
fn retirement_does_not_stop_a_reused_client_or_a_new_module_owner() {
    let fixture = Fixture::new();
    let mut mix = mixer();
    let mut music = Music::default();
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
    mix.paint(&mut out);
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
fn console_music_paths_pause_resume_and_once_use_the_shared_ring() {
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
    let mut music = Music::default();
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
    mix.paint(&mut pcm);
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
    mix.paint(&mut pcm);
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
    mix.paint(&mut pcm);
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
    mix.paint(&mut pcm);
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
    mix.paint(&mut output);
    assert_eq!(output, [1000, -1000, 2000, -2000, 0, 0]);
    assert!(!music.playing());
}
