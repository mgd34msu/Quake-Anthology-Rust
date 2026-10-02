//! Frontend audio over the shared gameplay mixer and music decoder.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/startup-audio.ts`
//! (`StartupAudio`). The frontend uses the same mixer and music decoder as
//! a gameplay session. Output cvars, music settings, menu paths, track
//! lists, cues, and preferences reuse the ported audio modules; the engine,
//! music controller, sound banks, and mounts (unported `UnifiedAudio`,
//! `ApplicationMusic`, `SoundBank`, `MountedContent` behind a shared-mixer
//! lifetime) arrive as the [`StartupAudioEngine`], [`StartupMusic`],
//! [`StartupSoundBank`], and [`StartupMusicMounts`] seams. Banks are shared
//! through `Rc<RefCell>` like the donor's shared references. Sync port:
//! the donor's async open/track selection become sync calls with the same
//! guards, texts, and ordering.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use qa_client::audio::output::AudioOutputFormat;
use qa_client::ui::common::controller::UiSound;
use qa_content::contract::GameFamily;
use qa_core::cvar::CvarRegistry;
use qa_core::identity::SeatId;
use thiserror::Error;

use super::audio::menu::menu_sound_path;
use super::audio::music::MusicSource;
use super::audio::playlist::mounted_music_tracks;
use super::audio::playlist::music_file_cue;
use super::audio::playlist::MusicMounts;
use super::audio::playlist_settings::read_music_settings;
use super::audio::playlist_settings::MusicPreferences;
use super::audio_settings::AudioPreferences;
use crate::bootstrap::audio::output_settings::read_audio_output_cvars;
use crate::bootstrap::audio::output_settings::write_audio_output_cvars;

/// Startup audio failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum StartupAudioError {
    /// Engine failure.
    #[error("{0}")]
    Engine(String),
    /// Mount failure.
    #[error("{0}")]
    Mount(String),
}

/// Shared-mixer engine (absorbed `UnifiedAudio` surface).
pub trait StartupAudioEngine<Asset> {
    /// Play a menu sound for a seat.
    fn play_menu_sound(&mut self, sound: &Asset, family: GameFamily, seat: &SeatId);
    /// Current output format.
    fn output_format(&self) -> AudioOutputFormat;
    /// Select an output format, restarting when requested.
    fn select_output(&mut self, format: &AudioOutputFormat, restart: bool);
    /// Open an output device.
    fn open_device(&mut self, device: Option<&str>) -> Result<(), String>;
    /// Set the effects volume.
    fn set_effects_volume(&mut self, gain: f64);
    /// Advance music decoding.
    fn update_music(&mut self);
    /// Pump decoded audio.
    fn pump(&mut self);
    /// Close the engine.
    fn close_engine(&mut self);
}

/// Music controller (absorbed `ApplicationMusic` surface).
pub trait StartupMusic<Bank> {
    /// Select the music source.
    fn select_source(&mut self, source: &MusicSource);
    /// Stop source playback.
    fn stop_playback(&mut self);
    /// Play a file cue through a bank.
    fn play_file(&mut self, source: &MusicSource, bank: &Rc<RefCell<Bank>>, cue: &str);
    /// Run a `cd` command.
    fn cd_command(&mut self, args: &[String], print: &mut dyn FnMut(&str));
    /// Run a `music` command.
    fn music_command(&mut self, args: &[String], print: &mut dyn FnMut(&str));
    /// Set the music volume.
    fn set_volume(&mut self, volume: f64);
    /// Stop the controller.
    fn stop(&mut self);
}

/// Sound bank (absorbed `SoundBank` surface).
pub trait StartupSoundBank {
    /// Registered asset.
    type Asset: Clone;
    /// Register a sound, if present.
    fn register(&mut self, path: &str, family: GameFamily) -> Option<Self::Asset>;
    /// Clear registrations.
    fn clear_bank(&mut self);
}

/// Music mounts (track lists plus file probes).
pub trait StartupMusicMounts: MusicMounts {
    /// Whether a file exists.
    fn has_file(&self, path: &str) -> bool;
}

/// One menu-track candidate (donor `candidates` row).
pub struct MusicCandidate<Mounts, Bank> {
    /// Candidate mounts.
    pub mounts: Mounts,
    /// Candidate source.
    pub source: MusicSource,
    /// Fallback track names.
    pub names: Vec<String>,
    /// Candidate bank.
    pub bank: Rc<RefCell<Bank>>,
}

/// Queued console command (donor `commands` row).
pub struct QueuedAudioCommand {
    /// Whether the command is `cd` (`music` otherwise).
    pub cd: bool,
    /// Command arguments.
    pub args: Vec<String>,
    /// Command printer.
    pub print: Box<dyn FnMut(&str)>,
}

/// Startup audio owners (donor `StartupAudio.open` options).
pub struct StartupAudioOptions<Mounts, Bank> {
    /// Content mounts.
    pub mounts: Mounts,
    /// Music source.
    pub source: MusicSource,
    /// Theme mounts, source, and bank, if any.
    pub theme: Option<(Mounts, MusicSource, Bank)>,
    /// Seat identity.
    pub seat: SeatId,
    /// Resolved preferences.
    pub preferences: AudioPreferences,
    /// Menu track selection (`None` means `"auto"`).
    pub menu_track: Option<String>,
}

fn fallback_tracks(family: GameFamily) -> Vec<String> {
    match family {
        GameFamily::Q1 => vec!["music/track02".to_string(), "music/02".to_string()],
        GameFamily::Q2 => vec!["music/02".to_string(), "music/track02".to_string()],
        GameFamily::Q3 => vec!["music/sonic5".to_string()],
    }
}

fn track_names(value: &str) -> Vec<String> {
    if !value.is_empty() && value.chars().all(|char| char.is_ascii_digit()) {
        let padded = format!("{:0>2}", value.parse::<u64>().unwrap_or(0));
        return vec![format!("music/{padded}"), format!("music/track{padded}")];
    }
    vec![if value.starts_with("music/") {
        value.to_string()
    } else {
        format!("music/{value}")
    }]
}

fn with_extensions(name: &str) -> Vec<String> {
    let lower = name.to_lowercase();
    if lower.ends_with(".ogg") || lower.ends_with(".wav") {
        vec![name.to_string()]
    } else {
        vec![format!("{name}.ogg"), format!("{name}.wav")]
    }
}

/// Frontend audio over the shared mixer.
pub struct StartupAudio<Engine, Music, Bank: StartupSoundBank, Mounts> {
    engine: Engine,
    music: Music,
    bank: Rc<RefCell<Bank>>,
    family: GameFamily,
    seat: SeatId,
    output_cvars: Option<CvarRegistry>,
    candidates: Vec<MusicCandidate<Mounts, Bank>>,
    menu_track: Option<String>,
    tracks: Vec<String>,
    commands: Vec<QueuedAudioCommand>,
    sounds: HashMap<UiSound, Bank::Asset>,
    print: Box<dyn FnMut(&str)>,
    closed: bool,
}

impl<Engine, Music, Bank, Mounts> StartupAudio<Engine, Music, Bank, Mounts>
where
    Engine: StartupAudioEngine<Bank::Asset>,
    Music: StartupMusic<Bank>,
    Bank: StartupSoundBank,
    Mounts: StartupMusicMounts,
{
    /// Open frontend audio, registering menu sounds and the menu track.
    pub fn open(
        engine: Engine,
        music: Music,
        bank: Bank,
        options: StartupAudioOptions<Mounts, Bank>,
        print: impl FnMut(&str) + 'static,
    ) -> Result<Self, StartupAudioError> {
        let bank = Rc::new(RefCell::new(bank));
        let mut audio = Self {
            engine,
            music,
            bank: Rc::clone(&bank),
            family: options.source.family,
            seat: options.seat.clone(),
            output_cvars: None,
            candidates: Vec::new(),
            menu_track: None,
            tracks: Vec::new(),
            commands: Vec::new(),
            sounds: HashMap::new(),
            print: Box::new(print),
            closed: false,
        };
        audio.engine.set_effects_volume(options.preferences.effects_volume);
        audio.music.set_volume(options.preferences.music_volume);
        for event in [
            UiSound::Open,
            UiSound::Close,
            UiSound::Move,
            UiSound::Change,
            UiSound::Reject,
        ] {
            let path = menu_sound_path(options.source.family, event);
            if let Some(sound) = audio.bank.borrow_mut().register(&path, options.source.family) {
                audio.sounds.insert(event, sound);
            }
        }
        audio.music.select_source(&options.source);
        audio.candidates.push(MusicCandidate {
            mounts: options.mounts,
            source: options.source,
            names: fallback_tracks(audio.family),
            bank: Rc::clone(&bank),
        });
        if let Some((mounts, source, theme_bank)) = options.theme {
            audio.candidates.insert(
                0,
                MusicCandidate {
                    mounts,
                    source,
                    names: vec!["music/track77".to_string()],
                    bank: Rc::new(RefCell::new(theme_bank)),
                },
            );
        }
        let mut tracks = Vec::new();
        for candidate in &audio.candidates {
            match mounted_music_tracks(&candidate.mounts) {
                Ok(list) => tracks.extend(list),
                Err(error) => return Err(StartupAudioError::Mount(error.to_string())),
            }
        }
        tracks.sort();
        tracks.dedup();
        audio.tracks = tracks;
        let menu_track = options.menu_track.clone().unwrap_or_else(|| "auto".to_string());
        audio.select_menu_track(&menu_track);
        Ok(audio)
    }

    /// Current output format.
    #[must_use]
    pub fn output_format(&self) -> AudioOutputFormat {
        self.engine.output_format()
    }

    /// Bind output cvars.
    pub fn bind_output_cvars(&mut self, cvars: CvarRegistry) {
        self.output_cvars = Some(cvars);
    }

    /// Select an output format, publishing to bound cvars.
    pub fn select_output_format(&mut self, format: &AudioOutputFormat) {
        self.engine.select_output(format, false);
        if let Some(cvars) = self.output_cvars.as_mut() {
            let _ = write_audio_output_cvars(cvars, self.engine.output_format());
        }
    }

    /// Restart output from bound cvars.
    pub fn restart_output(&mut self) {
        let format = match &self.output_cvars {
            Some(cvars) => read_audio_output_cvars(cvars).unwrap_or_else(|_| self.engine.output_format()),
            None => self.engine.output_format(),
        };
        self.engine.select_output(&format, true);
    }

    /// Mounted music tracks.
    #[must_use]
    pub fn music_tracks(&self) -> &[String] {
        &self.tracks
    }

    /// Music preferences (bound cvars or local selection).
    #[must_use]
    pub fn music_preferences(&self) -> MusicPreferences {
        match &self.output_cvars {
            Some(cvars) => read_music_settings(Some(cvars)),
            None => MusicPreferences {
                music_shuffle: false,
                menu_track: self.menu_track.clone().unwrap_or_else(|| "auto".to_string()),
            },
        }
    }

    /// Whether the audio is closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// Queued command count.
    #[must_use]
    pub fn queued_commands(&self) -> usize {
        self.commands.len()
    }

    /// Open an output device, falling back to the system default.
    pub fn open_output(
        &mut self,
        device_name: Option<&str>,
        print: &mut dyn FnMut(&str),
    ) -> Result<(), StartupAudioError> {
        match self.engine.open_device(device_name) {
            Ok(()) => Ok(()),
            Err(error) => {
                let Some(device) = device_name else {
                    return Err(StartupAudioError::Engine(error));
                };
                print(&format!(
                    "Audio output {device} unavailable: {error}. Using system default.\n"
                ));
                self.engine.open_device(None).map_err(StartupAudioError::Engine)
            }
        }
    }

    /// Run a `cd` command immediately.
    pub fn cd_command(&mut self, args: &[String], print: &mut dyn FnMut(&str)) {
        if !self.closed {
            self.music.cd_command(args, print);
        }
    }

    /// Queue a `cd` command.
    pub fn queue_cd_command(&mut self, args: Vec<String>, print: impl FnMut(&str) + 'static) {
        if !self.closed {
            self.commands.push(QueuedAudioCommand {
                cd: true,
                args,
                print: Box::new(print),
            });
        }
    }

    /// Queue a `music` command.
    pub fn queue_music_command(&mut self, args: Vec<String>, print: impl FnMut(&str) + 'static) {
        if !self.closed {
            self.commands.push(QueuedAudioCommand {
                cd: false,
                args,
                print: Box::new(print),
            });
        }
    }

    /// Flush queued commands, reselecting the menu track first.
    pub fn flush_commands(&mut self) {
        if let Some(cvars) = &self.output_cvars {
            let track = read_music_settings(Some(cvars)).menu_track;
            self.select_menu_track(&track);
        }
        while !self.closed {
            if self.commands.is_empty() {
                return;
            }
            let mut command = self.commands.remove(0);
            if command.cd {
                self.music.cd_command(&command.args, &mut command.print);
            } else {
                self.music.music_command(&command.args, &mut command.print);
            }
        }
    }

    /// Set effects and music volumes.
    pub fn set_volumes(&mut self, effects: f64, music: f64) {
        if self.closed {
            return;
        }
        self.engine.set_effects_volume(effects);
        self.music.set_volume(music);
    }

    /// Play a menu sound.
    pub fn sound(&mut self, event: UiSound) {
        if self.closed {
            return;
        }
        if let Some(sound) = self.sounds.get(&event).cloned() {
            self.engine.play_menu_sound(&sound, self.family, &self.seat);
        }
    }

    /// Pump decoded audio.
    pub fn pump(&mut self) {
        if !self.closed {
            self.engine.update_music();
            self.engine.pump();
        }
    }

    /// Close audio and clear banks.
    pub fn close(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        self.commands.clear();
        self.music.stop();
        self.engine.close_engine();
        self.bank.borrow_mut().clear_bank();
        for candidate in &self.candidates {
            // The main candidate shares the main bank; clearing twice is harmless.
            candidate.bank.borrow_mut().clear_bank();
        }
        self.sounds.clear();
    }

    fn select_menu_track(&mut self, value: &str) {
        if self.closed || self.menu_track.as_deref() == Some(value) {
            return;
        }
        self.menu_track = Some(value.to_string());
        self.music.stop_playback();
        if value == "0" {
            return;
        }
        for index in 0..self.candidates.len() {
            let names = if value == "auto" {
                self.candidates[index].names.clone()
            } else {
                track_names(value)
            };
            for name in &names {
                for path in with_extensions(name) {
                    if self.candidates[index].mounts.has_file(&path) {
                        if self.closed || self.menu_track.as_deref() != Some(value) {
                            return;
                        }
                        let candidate = &self.candidates[index];
                        let cue = music_file_cue(&path);
                        self.music.play_file(&candidate.source, &candidate.bank, &cue);
                        return;
                    }
                }
            }
        }
        if value != "auto" {
            (self.print)(&format!("Menu music unavailable: {value}\n"));
        }
    }
}

#[cfg(test)]
mod tests {
    type TestAudio = (
        StartupAudio<FakeEngine, FakeMusic, FakeBank, FakeMounts>,
        IdentityOwner,
        Rc<RefCell<Vec<String>>>,
    );

    use std::collections::HashSet;

    use qa_client::audio::output::DEFAULT_AUDIO_OUTPUT_FORMAT;
    use qa_content::contract::ContentId;
    use qa_content::mounts::MountError;
    use qa_core::cmd::Dialect;
    use qa_core::identity::IdentityOwner;

    use super::super::audio::output_settings::register_audio_output_cvars;
    use super::*;

    #[derive(Debug, Clone)]
    struct FakeMounts {
        files: HashSet<String>,
    }

    impl MusicMounts for FakeMounts {
        fn list_files(&self, directory: &str, extension: &str) -> Result<Vec<String>, MountError> {
            let _ = (directory, extension);
            Ok(Vec::new())
        }
    }

    impl StartupMusicMounts for FakeMounts {
        fn has_file(&self, path: &str) -> bool {
            self.files.contains(path)
        }
    }

    #[derive(Debug, Default)]
    struct FakeBank {
        registered: Vec<String>,
        cleared: usize,
    }

    impl StartupSoundBank for FakeBank {
        type Asset = String;

        fn register(&mut self, path: &str, _family: GameFamily) -> Option<String> {
            self.registered.push(path.to_string());
            Some(path.to_string())
        }

        fn clear_bank(&mut self) {
            self.cleared += 1;
        }
    }

    struct FakeEngine {
        format: AudioOutputFormat,
        selections: Vec<(AudioOutputFormat, bool)>,
        volumes: Vec<f64>,
        played: Vec<String>,
        devices: Vec<Option<String>>,
        fail_device: bool,
        pumped: usize,
        closed: bool,
    }

    impl StartupAudioEngine<String> for FakeEngine {
        fn play_menu_sound(&mut self, sound: &String, _family: GameFamily, _seat: &SeatId) {
            self.played.push(sound.clone());
        }

        fn output_format(&self) -> AudioOutputFormat {
            self.format
        }

        fn select_output(&mut self, format: &AudioOutputFormat, restart: bool) {
            self.format = *format;
            self.selections.push((*format, restart));
        }

        fn open_device(&mut self, device: Option<&str>) -> Result<(), String> {
            self.devices.push(device.map(str::to_string));
            if self.fail_device && device.is_some() {
                return Err("no such device".to_string());
            }
            Ok(())
        }

        fn set_effects_volume(&mut self, gain: f64) {
            self.volumes.push(gain);
        }

        fn update_music(&mut self) {}

        fn pump(&mut self) {
            self.pumped += 1;
        }

        fn close_engine(&mut self) {
            self.closed = true;
        }
    }

    struct FakeMusic {
        cues: Vec<String>,
        cd: Vec<Vec<String>>,
        music: Vec<Vec<String>>,
        volumes: Vec<f64>,
        stopped: usize,
    }

    impl StartupMusic<FakeBank> for FakeMusic {
        fn select_source(&mut self, _source: &MusicSource) {}

        fn stop_playback(&mut self) {}

        fn play_file(&mut self, _source: &MusicSource, _bank: &Rc<RefCell<FakeBank>>, cue: &str) {
            self.cues.push(cue.to_string());
        }

        fn cd_command(&mut self, args: &[String], _print: &mut dyn FnMut(&str)) {
            self.cd.push(args.to_vec());
        }

        fn music_command(&mut self, args: &[String], _print: &mut dyn FnMut(&str)) {
            self.music.push(args.to_vec());
        }

        fn set_volume(&mut self, volume: f64) {
            self.volumes.push(volume);
        }

        fn stop(&mut self) {
            self.stopped += 1;
        }
    }

    fn source() -> MusicSource {
        MusicSource {
            content: ContentId("q1".to_string()),
            family: GameFamily::Q1,
            edition: String::new(),
            campaign: String::new(),
        }
    }

    fn preferences() -> AudioPreferences {
        AudioPreferences {
            music_shuffle: None,
            menu_track: None,
            device_name: None,
            output_format: DEFAULT_AUDIO_OUTPUT_FORMAT,
            effects_volume: 0.7,
            music_volume: 0.25,
        }
    }

    fn setup(files: &[&str], menu_track: Option<&str>) -> TestAudio {
        let owner = IdentityOwner::create("startup-audio-test").unwrap();
        let printed = Rc::new(RefCell::new(Vec::new()));
        let output = Rc::clone(&printed);
        let audio = StartupAudio::open(
            FakeEngine {
                format: DEFAULT_AUDIO_OUTPUT_FORMAT,
                selections: Vec::new(),
                volumes: Vec::new(),
                played: Vec::new(),
                devices: Vec::new(),
                fail_device: false,
                pumped: 0,
                closed: false,
            },
            FakeMusic {
                cues: Vec::new(),
                cd: Vec::new(),
                music: Vec::new(),
                volumes: Vec::new(),
                stopped: 0,
            },
            FakeBank::default(),
            StartupAudioOptions {
                mounts: FakeMounts {
                    files: files.iter().map(|file| file.to_string()).collect(),
                },
                source: source(),
                theme: None,
                seat: owner.seat(0),
                preferences: preferences(),
                menu_track: menu_track.map(str::to_string),
            },
            move |text| output.borrow_mut().push(text.to_string()),
        )
        .unwrap();
        (audio, owner, printed)
    }

    #[test]
    fn open_registers_sounds_and_auto_track() {
        let (mut audio, _, printed) = setup(&["music/track02.ogg"], None);
        assert!(audio.music_tracks().is_empty());
        assert_eq!(audio.music_preferences().menu_track, "auto");
        assert_eq!(audio.music.cues.len(), 1);
        assert!(printed.borrow().is_empty());
        audio.sound(UiSound::Open);
        assert_eq!(audio.engine.played.len(), 1);
        audio.pump();
        assert_eq!(audio.engine.pumped, 1);
        audio.close();
        assert!(audio.is_closed());
        audio.sound(UiSound::Open);
        assert_eq!(audio.engine.played.len(), 1);
        audio.close();
    }

    #[test]
    fn numeric_and_named_tracks_resolve() {
        let (audio, _, _) = setup(&["music/03.wav"], Some("3"));
        assert_eq!(audio.music.cues.len(), 1);
        let (audio, _, printed) = setup(&[], Some("missing"));
        assert!(audio.music.cues.is_empty());
        assert_eq!(*printed.borrow(), vec!["Menu music unavailable: missing\n".to_string()]);
        let (audio, _, _) = setup(&["music/track02.ogg"], Some("0"));
        assert!(audio.music.cues.is_empty());
    }

    #[test]
    fn output_selection_publishes_and_restarts() {
        let (mut audio, _, _) = setup(&[], None);
        let mut cvars = CvarRegistry::new(Dialect::Q3);
        register_audio_output_cvars(&mut cvars, DEFAULT_AUDIO_OUTPUT_FORMAT).unwrap();
        audio.bind_output_cvars(cvars);
        let mut format = DEFAULT_AUDIO_OUTPUT_FORMAT;
        format.sample_rate = 22050;
        audio.select_output_format(&format);
        assert_eq!(audio.output_format().sample_rate, 22050);
        audio.restart_output();
        assert!(audio.engine.selections.iter().any(|(_, restart)| *restart));
    }

    #[test]
    fn commands_queue_and_flush() {
        let (mut audio, _, _) = setup(&[], None);
        audio.queue_cd_command(vec!["play".to_string()], |_| {});
        audio.queue_music_command(vec!["stop".to_string()], |_| {});
        assert_eq!(audio.queued_commands(), 2);
        audio.flush_commands();
        assert_eq!(audio.queued_commands(), 0);
        assert_eq!(audio.music.cd.len(), 1);
        assert_eq!(audio.music.music.len(), 1);
        audio.set_volumes(0.5, 0.1);
        assert_eq!(audio.engine.volumes.last(), Some(&0.5));
    }

    #[test]
    fn open_output_falls_back_to_default() {
        let (mut audio, _, _) = setup(&[], None);
        audio.engine.fail_device = true;
        let mut printed = Vec::new();
        audio
            .open_output(Some("nope"), &mut |text| printed.push(text.to_string()))
            .unwrap();
        assert_eq!(audio.engine.devices, vec![Some("nope".to_string()), None]);
        assert!(printed.concat().contains("Audio output nope unavailable"));
        assert!(audio.open_output(None, &mut |_| {}).is_ok());
    }
}
