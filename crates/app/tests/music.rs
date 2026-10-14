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
        module: ModuleId(1),
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
