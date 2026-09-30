//! Application soundtrack selection over the shared audio engine.
//!
//! Port of donor `src/app/bootstrap/audio/music.ts` (`MusicSource`,
//! `worldMusicTrack`, `q1MusicFallback`, `ApplicationMusic`). The donor's
//! async track opens run synchronously. Two ownership seams differ from
//! the donor: the shared `SoundBank` is reference-counted so the track
//! opener can borrow it while the selection owns it, and the live player
//! stays owned by `CdMusic` because the merged engine takes attached
//! players by value without a lane accessor, so selection does not attach
//! a mix lane (teardown still stops the `world` lane); selection,
//! fallback, shuffle, and command behavior are unchanged.
//!
//! The donor shares one [`MusicControls`] object between the application
//! and the player; the merged player clones it at construction, so every
//! command that mutates controls applies the change to both copies.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use qa_client::audio::bank::{SoundBank, SoundContent};
use qa_client::audio::engine::UnifiedAudio;
use qa_client::audio::error::AudioError;
use qa_client::audio::music::{
    remap_q2_music_track, CdMusic, LoopSource, MusicControls, MusicPlayer, MusicVolumeMode, OpenMusicTrack,
    Q2SoundtrackProfile,
};
use qa_client::audio::streams::PcmStream;
use qa_client::audio::SoundFamily;
use qa_content::catalog::{CatalogError, InstalledCatalog, ProductAvailability};
use qa_content::contract::{ContentId, GameFamily};

use super::output_settings::{is_js_trim, js_number_string};
use super::playlist::{is_js_space, music_file_cue, shuffled_tracks};

/// Soundtrack source: content identity plus family and edition selectors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MusicSource {
    /// Content identity.
    pub content: ContentId,
    /// Game family.
    pub family: GameFamily,
    /// Edition selector (`rerelease` selects the remastered Q2 order).
    pub edition: String,
    /// Campaign selector for the remastered Q2 order.
    pub campaign: String,
}

/// Shared track opener (donor `OpenMusicTrack` object identity).
pub type SharedOpener = Rc<RefCell<OpenMusicTrack>>;

/// Authored playlist for automatic playback.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MusicPlaylist {
    /// Shuffle at track end (Quake II only).
    pub shuffle: bool,
    /// Playlist tracks.
    pub tracks: Vec<String>,
}

/// Why playback stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopPlaybackReason {
    /// Explicit stop; clears the automatic cue.
    Manual,
    /// Source retired; also clears the authored cue.
    Source,
}

/// Options for [`ApplicationMusic::new`].
pub struct ApplicationMusicOptions {
    /// Player volume mode.
    pub volume_mode: MusicVolumeMode,
    /// Shared controls.
    pub controls: MusicControls,
    /// Shuffle draw in `[0, 1)`; defaults to a time-seeded draw.
    pub random: Option<Box<dyn FnMut() -> f64>>,
}

impl Default for ApplicationMusicOptions {
    fn default() -> Self {
        Self {
            volume_mode: MusicVolumeMode::Source,
            controls: MusicControls::new(),
            random: None,
        }
    }
}

fn sound_family(family: GameFamily) -> SoundFamily {
    match family {
        GameFamily::Q1 => SoundFamily::Q1,
        GameFamily::Q2 => SoundFamily::Q2,
        GameFamily::Q3 => SoundFamily::Q3,
    }
}

fn default_random() -> Box<dyn FnMut() -> f64> {
    #[allow(clippy::cast_possible_truncation)]
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0x1234_5678_9abc_def0, |time| time.as_nanos() as u64)
        | 1;
    let mut state = seed;
    Box::new(move || {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        #[allow(clippy::cast_precision_loss)]
        {
            (state.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 11) as f64 / 9_007_199_254_740_992.0
        }
    })
}

/// Worldspawn music key for a source.
#[must_use]
pub fn world_music_track(world: Option<&HashMap<String, String>>, source: &MusicSource) -> String {
    if source.family == GameFamily::Q3 {
        return world.and_then(|world| world.get("music")).cloned().unwrap_or_default();
    }
    let music = if source.family == GameFamily::Q2 && source.edition == "rerelease" {
        world.and_then(|world| world.get("music")).cloned().unwrap_or_default()
    } else {
        String::new()
    };
    if !music.is_empty() {
        return music;
    }
    world.and_then(|world| world.get("sounds")).cloned().unwrap_or_default()
}

/// Alternate Q1 content sharing the selected numbered soundtrack, if installed.
pub fn q1_music_fallback(content: &ContentId, catalog: &InstalledCatalog) -> Result<Option<ContentId>, CatalogError> {
    const PAIRS: [[&str; 2]; 3] = [
        ["q1-classic-id1", "q1-rerelease-id1"],
        ["q1-classic-hipnotic", "q1-rerelease-hipnotic"],
        ["q1-classic-rogue", "q1-rerelease-rogue"],
    ];
    let selected = catalog.product(content.as_str())?;
    let Some(pair) = PAIRS.iter().find(|ids| ids.contains(&selected.expectation.id.as_str())) else {
        return Ok(None);
    };
    let Some(alternate) = pair.iter().find(|id| **id != selected.expectation.id) else {
        return Ok(None);
    };
    Ok(catalog
        .products
        .iter()
        .find(|candidate| {
            candidate.expectation.id == **alternate && candidate.availability == ProductAvailability::Installed
        })
        .map(|product| product.id.clone()))
}

fn is_safe_integer(value: f64) -> bool {
    value.is_finite() && value.fract() == 0.0 && value.abs() <= 9_007_199_254_740_991.0
}

/// Split a cue into `"quoted"` and bare tokens, stripping one round of quotes.
fn tokenize_cue(selected: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut chars = selected.chars().peekable();
    while let Some(&char) = chars.peek() {
        if char == '"' {
            chars.next();
            let mut token = String::new();
            for char in chars.by_ref() {
                if char == '"' {
                    break;
                }
                token.push(char);
            }
            tokens.push(token);
        } else if is_js_space(char) {
            chars.next();
        } else {
            let mut token = String::new();
            while let Some(&char) = chars.peek() {
                if is_js_space(char) {
                    break;
                }
                token.push(char);
                chars.next();
            }
            let mut token = token.as_str();
            if let Some(rest) = token.strip_prefix('"') {
                token = rest;
            }
            if let Some(rest) = token.strip_suffix('"') {
                token = rest;
            }
            tokens.push(token.to_string());
        }
    }
    tokens
}

fn has_music_extension(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".wav") || lower.ends_with(".ogg")
}

struct Current<Content: SoundContent + 'static> {
    source: MusicSource,
    bank: Rc<RefCell<SoundBank<Content>>>,
    fallback: Option<SharedOpener>,
    fallback_active: Rc<Cell<bool>>,
    cd: CdMusic,
    track: String,
    looping: bool,
    authored_cue: String,
}

struct Automatic {
    cue: String,
    tracks: Vec<String>,
    bag: Vec<String>,
    shuffle: bool,
    completed: u64,
}

/// One world soundtrack over the shared engine's music state.
pub struct ApplicationMusic<'engine, Content: SoundContent + 'static> {
    engine: &'engine mut UnifiedAudio,
    print: Box<dyn FnMut(&str) + 'engine>,
    volume_mode: MusicVolumeMode,
    /// Shared controls, mirrored into the live player by [`ApplicationMusic::cd_command`].
    /// Prefer the command path for remaps; the enabled flag is honored when set directly.
    pub controls: MusicControls,
    random: Box<dyn FnMut() -> f64 + 'engine>,
    current: Option<Current<Content>>,
    automatic: Option<Automatic>,
    request: u64,
    gain: f64,
}

impl<'engine, Content: SoundContent + 'static> ApplicationMusic<'engine, Content> {
    /// Application music over an engine with a print sink and options.
    pub fn new(
        engine: &'engine mut UnifiedAudio,
        print: Box<dyn FnMut(&str) + 'engine>,
        options: ApplicationMusicOptions,
    ) -> Self {
        Self {
            engine,
            print,
            volume_mode: options.volume_mode,
            controls: options.controls,
            random: options.random.unwrap_or_else(default_random),
            current: None,
            automatic: None,
            request: 0,
            gain: 0.25,
        }
    }

    fn emit(&mut self, text: &str) {
        (self.print)(text);
    }

    /// Current music gain.
    #[must_use]
    pub fn volume(&self) -> f64 {
        self.gain
    }

    /// Set the music gain.
    pub fn set_volume(&mut self, value: f64) -> Result<(), AudioError> {
        if !value.is_finite() || value < 0.0 {
            return Err(AudioError::BadMusicVolume);
        }
        self.gain = value;
        if let Some(current) = self.current.as_mut() {
            current.cd.player_mut().set_volume(value)?;
        }
        Ok(())
    }

    /// Select a soundtrack source, keeping the live selection when identical.
    pub fn select(
        &mut self,
        source: &MusicSource,
        bank: Rc<RefCell<SoundBank<Content>>>,
        fallback: Option<SharedOpener>,
    ) {
        if self
            .current
            .as_ref()
            .is_some_and(|current| current.source.content == source.content && Rc::ptr_eq(&current.bank, &bank))
        {
            return;
        }
        self.stop();
        let mut player = MusicPlayer::new(
            self.engine.sample_rate(),
            sound_family(source.family),
            self.volume_mode,
            self.controls.clone(),
        );
        // The gain is validated on the way in, so this cannot fail.
        player.set_volume(self.gain).expect("gain is a valid volume");
        let fallback_active = Rc::new(Cell::new(false));
        let opener_bank = Rc::clone(&bank);
        let opener_fallback = fallback.clone();
        let opener_flag = Rc::clone(&fallback_active);
        let open: OpenMusicTrack =
            Box::new(move |path| Self::dispatch_open(&opener_bank, &opener_fallback, &opener_flag, path));
        let cd = CdMusic::new(player, open);
        self.current = Some(Current {
            source: source.clone(),
            bank,
            fallback,
            fallback_active,
            cd,
            track: String::new(),
            looping: false,
            authored_cue: String::new(),
        });
    }

    /// Open through the bank, or the fallback while it is swapped in.
    fn dispatch_open(
        bank: &Rc<RefCell<SoundBank<Content>>>,
        fallback: &Option<SharedOpener>,
        fallback_active: &Rc<Cell<bool>>,
        path: &str,
    ) -> Result<Option<Box<dyn PcmStream>>, AudioError> {
        if fallback_active.get() {
            if let Some(fallback) = fallback {
                return fallback.borrow_mut()(path);
            }
        }
        bank.borrow_mut().open_music(path, None)
    }

    /// Run the `music` command.
    pub fn music_command(&mut self, args: &[String]) -> Result<(), AudioError> {
        if args.is_empty() || args.len() > 2 || args.iter().any(|value| value.trim_matches(is_js_trim).is_empty()) {
            self.emit("music <intro> [loop]\n");
            return Ok(());
        }
        if self.current.is_none() {
            self.emit("No soundtrack source selected.\n");
            return Ok(());
        }
        self.automatic = None;
        let looping = args.len() == 2;
        let selected = args.iter().map(|arg| music_file_cue(arg)).collect::<Vec<_>>().join(" ");
        self.start_track(&selected, looping, false, &|| true)
    }

    /// Run the `cd` command.
    pub fn cd_command(&mut self, args: &[String]) -> Result<(), AudioError> {
        let Some(command) = args.first().map(|arg| arg.to_lowercase()) else {
            return Ok(());
        };
        if command == "close" || command == "eject" {
            self.emit(&format!(
                "cd {command}: disc tray operations are unavailable with file-backed music.\n"
            ));
            return Ok(());
        }
        if command == "info" {
            if !self.controls.enabled {
                self.emit("CD music is disabled.\n");
            } else {
                let line = self
                    .current
                    .as_ref()
                    .map(|current| {
                        if current.cd.player().playing() {
                            let mapped = current.cd.playing_track();
                            let track = match mapped {
                                Some(mapped) if mapped.to_string() != current.track => {
                                    format!("{} (mapped to {mapped})", current.track)
                                }
                                _ => current.track.clone(),
                            };
                            format!(
                                "{} {} track {track}\n",
                                if current.cd.player().paused {
                                    "Paused"
                                } else {
                                    "Currently"
                                },
                                if current.looping { "looping" } else { "playing" },
                            )
                        } else {
                            "Not playing.\n".to_string()
                        }
                    })
                    .unwrap_or_else(|| "Not playing.\n".to_string());
                self.emit(&line);
            }
            let gain = js_number_string(self.gain);
            self.emit(&format!("Volume is {gain}\n"));
            return Ok(());
        }
        if command == "on" {
            self.controls.enabled = true;
            if let Some(current) = self.current.as_mut() {
                current.cd.set_enabled(true);
            }
            return Ok(());
        }
        if command == "off" {
            self.stop_playback(StopPlaybackReason::Manual);
            self.controls.enabled = false;
            if let Some(current) = self.current.as_mut() {
                current.cd.set_enabled(false);
            }
            return Ok(());
        }
        if command == "stop" {
            self.stop_playback(StopPlaybackReason::Manual);
            return Ok(());
        }
        if command == "reset" {
            self.stop_playback(StopPlaybackReason::Manual);
            self.controls.reset();
            if let Some(current) = self.current.as_mut() {
                current.cd.reset();
            }
            return Ok(());
        }
        if command == "remap" {
            if args.len() == 1 {
                let mut lines = String::new();
                for (index, track) in self.controls.remapped_tracks().iter().enumerate() {
                    #[allow(clippy::cast_possible_wrap)]
                    if *track != index as i32 + 1 {
                        lines.push_str(&format!("  {} -> {track}\n", index + 1));
                    }
                }
                if !lines.is_empty() {
                    self.emit(&lines);
                }
                return Ok(());
            }
            let mut tracks = Vec::new();
            for value in &args[1..] {
                if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                    tracks.push(f64::NAN);
                } else {
                    tracks.push(value.parse::<f64>().unwrap_or(f64::NAN));
                }
            }
            if tracks.len() > 99
                || tracks
                    .iter()
                    .any(|track| !is_safe_integer(*track) || *track < 0.0 || *track > 255.0)
            {
                self.emit("cd remap requires at most 99 track numbers from 0 through 255.\n");
                return Ok(());
            }
            #[allow(clippy::cast_possible_truncation)]
            let tracks: Vec<i32> = tracks.iter().map(|track| *track as i32).collect();
            self.controls.set_remap(&tracks)?;
            if let Some(current) = self.current.as_mut() {
                current.cd.set_remap(&tracks)?;
            }
            return Ok(());
        }
        if self.current.is_none() {
            if command != "pause" && command != "resume" {
                self.emit("No soundtrack source selected.\n");
            }
            return Ok(());
        }
        if command == "play" || command == "loop" {
            let argument = args.get(1).map_or("", String::as_str);
            let track = if argument.is_empty() || !argument.bytes().all(|byte| byte.is_ascii_digit()) {
                f64::NAN
            } else {
                argument.parse::<f64>().unwrap_or(f64::NAN)
            };
            if args.len() != 2 || !is_safe_integer(track) || track < 1.0 || track > 255.0 {
                self.emit(&format!("cd {command} <track 1..255>\n"));
                return Ok(());
            }
            self.automatic = None;
            #[allow(clippy::cast_possible_truncation)]
            let selected = (track as i64).to_string();
            let looping = command == "loop";
            return self.start_track(&selected, looping, true, &|| true);
        }
        if command == "pause" || command == "resume" {
            if let Some(current) = self.current.as_mut() {
                if command == "pause" {
                    current.cd.pause();
                } else if self.controls.enabled {
                    current.cd.resume();
                }
            }
            return Ok(());
        }
        self.emit(&format!("Unknown cd command: {command}.\n"));
        Ok(())
    }

    /// Stop playback, clearing the automatic cue (and the authored cue for sources).
    pub fn stop_playback(&mut self, reason: StopPlaybackReason) {
        self.automatic = None;
        if reason == StopPlaybackReason::Source {
            if let Some(current) = self.current.as_mut() {
                current.authored_cue.clear();
            }
        }
        self.clear_playback();
    }

    /// Invalidate pending opens.
    pub fn invalidate_pending(&mut self) {
        self.request += 1;
        if let Some(current) = self.current.as_mut() {
            current.cd.invalidate_pending();
        }
    }

    fn clear_playback(&mut self) {
        self.request += 1;
        if let Some(current) = self.current.as_mut() {
            current.cd.stop();
        }
        self.engine.stop_music("world");
    }

    /// Retire the selected content as well as playback when its application closes.
    pub fn stop(&mut self) {
        self.stop_playback(StopPlaybackReason::Source);
        self.current = None;
    }

    /// Play an authored cue, shuffling a playlist for Quake II sources.
    pub fn play(
        &mut self,
        source: &MusicSource,
        bank: Rc<RefCell<SoundBank<Content>>>,
        track: &str,
        fallback: Option<SharedOpener>,
        playlist: Option<&MusicPlaylist>,
    ) -> Result<(), AudioError> {
        self.select(source, bank, fallback);
        let selected = track.trim_matches(is_js_trim).to_string();
        if playlist.is_some()
            && self
                .current
                .as_ref()
                .is_some_and(|current| current.authored_cue == selected)
            && self.automatic.is_none()
        {
            return Ok(());
        }
        if let Some(current) = self.current.as_mut() {
            current.authored_cue = selected.clone();
        }
        if selected.is_empty() || selected == "0" {
            self.automatic = None;
            self.clear_playback();
            return Ok(());
        }
        if self.current.is_none() {
            return Ok(());
        }
        let shuffle = source.family == GameFamily::Q2
            && playlist.is_some_and(|playlist| playlist.shuffle && !playlist.tracks.is_empty());
        let resume = self
            .automatic
            .as_ref()
            .is_some_and(|automatic| automatic.cue == selected && automatic.shuffle == shuffle)
            && self
                .current
                .as_ref()
                .is_some_and(|current| current.cd.player().playing());
        if resume {
            return Ok(());
        }
        let completed = self
            .current
            .as_ref()
            .map_or(0, |current| current.cd.player().completed_plays());
        self.automatic = Some(Automatic {
            cue: selected.clone(),
            tracks: playlist.map_or_else(Vec::new, |playlist| playlist.tracks.clone()),
            bag: Vec::new(),
            shuffle,
            completed,
        });
        if shuffle {
            self.next_automatic_track()
        } else {
            self.start_track(&selected, true, false, &|| true)
        }
    }

    /// Play an intro/loop pair while its owner stays current.
    pub fn play_tracks(
        &mut self,
        source: &MusicSource,
        bank: Rc<RefCell<SoundBank<Content>>>,
        intro: &str,
        loop_cue: &str,
        current: &dyn Fn() -> bool,
    ) -> Result<(), AudioError> {
        if !current() {
            return Ok(());
        }
        self.select(source, bank, None);
        self.automatic = None;
        if let Some(selected) = self.current.as_mut() {
            selected.authored_cue.clear();
        }
        if intro.is_empty() {
            self.clear_playback();
            return Ok(());
        }
        let selected = if loop_cue.is_empty() {
            music_file_cue(intro)
        } else {
            format!("{} {}", music_file_cue(intro), music_file_cue(loop_cue))
        };
        self.start_track(&selected, true, false, current)
    }

    /// Advance automatic playback when the shuffle state or completion count moves.
    pub fn update_automatic(&mut self, shuffle: bool) -> Result<(), AudioError> {
        if self.automatic.is_none() || self.current.is_none() || !self.controls.enabled {
            return Ok(());
        }
        if self.current.as_ref().is_some_and(|current| current.cd.player().paused) {
            return Ok(());
        }
        let enabled = self
            .current
            .as_ref()
            .is_some_and(|current| current.source.family == GameFamily::Q2)
            && shuffle
            && self
                .automatic
                .as_ref()
                .is_some_and(|automatic| !automatic.tracks.is_empty());
        let automatic = self.automatic.as_ref().expect("automatic playback selected");
        if enabled != automatic.shuffle {
            let cue = automatic.cue.clone();
            let completed = self
                .current
                .as_ref()
                .map_or(0, |current| current.cd.player().completed_plays());
            if let Some(automatic) = self.automatic.as_mut() {
                automatic.shuffle = enabled;
                automatic.completed = completed;
            }
            if enabled {
                return self.next_automatic_track();
            }
            return self.start_track(&cue, true, false, &|| true);
        }
        let completed = self
            .current
            .as_ref()
            .map_or(0, |current| current.cd.player().completed_plays());
        let advance = self
            .automatic
            .as_ref()
            .is_some_and(|automatic| enabled && automatic.completed != completed);
        if advance {
            if let Some(automatic) = self.automatic.as_mut() {
                automatic.completed = completed;
            }
            return self.next_automatic_track();
        }
        Ok(())
    }

    fn next_automatic_track(&mut self) -> Result<(), AudioError> {
        if self
            .automatic
            .as_ref()
            .is_some_and(|automatic| automatic.bag.is_empty())
        {
            let tracks = self
                .automatic
                .as_ref()
                .map_or_else(Vec::new, |automatic| automatic.tracks.clone());
            let previous = self
                .current
                .as_ref()
                .map_or_else(String::new, |current| current.track.clone());
            // The synchronous port has no suspension points, so the automatic
            // selection cannot be replaced while a track starts.
            let bag = shuffled_tracks(&tracks, &previous, &mut *self.random);
            if let Some(automatic) = self.automatic.as_mut() {
                automatic.bag = bag;
            }
        }
        loop {
            let next = {
                let (Some(automatic), Some(_)) = (self.automatic.as_mut(), self.current.as_ref()) else {
                    return Ok(());
                };
                if automatic.bag.is_empty() || !self.controls.enabled {
                    return Ok(());
                }
                automatic.bag.remove(0)
            };
            self.start_track(&music_file_cue(&next), false, false, &|| true)?;
            if self
                .current
                .as_ref()
                .is_some_and(|current| current.cd.player().playing())
            {
                return Ok(());
            }
        }
    }

    /// Start a cue, resolving numbered tracks through the CD layer.
    fn start_track(
        &mut self,
        selected: &str,
        looping: bool,
        numbered: bool,
        active: &dyn Fn() -> bool,
    ) -> Result<(), AudioError> {
        if !self.controls.enabled || !active() {
            return Ok(());
        }
        let fresh = self.current.as_ref().is_some_and(|current| current.cd.enabled());
        if !fresh {
            return Ok(());
        }
        let (family, edition, campaign, content) = self
            .current
            .as_ref()
            .map(|current| {
                (
                    current.source.family,
                    current.source.edition.clone(),
                    current.source.campaign.clone(),
                    current.source.content.to_string(),
                )
            })
            .expect("current selection checked");
        let numeric = !selected.is_empty()
            && selected.bytes().all(|byte| byte.is_ascii_digit())
            && (family != GameFamily::Q3 || numbered);
        #[allow(clippy::cast_possible_truncation)]
        let requested = selected.parse::<f64>().unwrap_or(f64::NAN) as i32;
        let mapped = if !numeric {
            None
        } else if family == GameFamily::Q2 {
            let profile = if edition == "rerelease" {
                Q2SoundtrackProfile::Remastered { campaign }
            } else {
                Q2SoundtrackProfile::Disc
            };
            Some(remap_q2_music_track(requested, &profile)?)
        } else {
            Some(requested)
        };
        let resume = self.current.as_ref().is_some_and(|current| {
            current.track == selected
                && current.looping == looping
                && current.cd.player().playing()
                && mapped.is_none_or(|mapped| {
                    let remapped = current
                        .cd
                        .remapped_tracks()
                        .get(
                            mapped
                                .checked_sub(1)
                                .and_then(|index| usize::try_from(index).ok())
                                .unwrap_or(usize::MAX),
                        )
                        .copied()
                        .unwrap_or(mapped);
                    current.cd.playing_track() == Some(remapped)
                })
        });
        if resume {
            return Ok(());
        }
        self.clear_playback();
        let request = self.request;
        if let Some(current) = self.current.as_mut() {
            current.track = selected.to_string();
            current.looping = looping;
            current.fallback_active.set(false);
        }
        if let Some(mapped) = mapped {
            let played = self
                .current
                .as_mut()
                .expect("current selection checked")
                .cd
                .play(mapped, looping)?;
            let retry = !played
                && family == GameFamily::Q1
                && self.current.as_ref().is_some_and(|current| current.fallback.is_some())
                && request == self.request;
            let played = if retry {
                let current = self.current.as_mut().expect("current selection checked");
                current.fallback_active.set(true);
                current.cd.play(mapped, looping)?
            } else {
                played
            };
            if request != self.request || !self.controls.enabled {
                return Ok(());
            }
            if !played {
                self.emit(&format!("Music unavailable: {content}/{selected}\n"));
            }
            return Ok(());
        }
        let tokens = tokenize_cue(selected);
        let intro_name = tokens.first().cloned().unwrap_or_default();
        let loop_name = tokens.get(1).cloned();
        let (bank, fallback) = self
            .current
            .as_ref()
            .map(|current| (Rc::clone(&current.bank), current.fallback.clone()))
            .expect("current selection checked");
        let stale = || request != self.request || !self.controls.enabled || !active();
        let open = |name: &str, stale: &dyn Fn() -> bool| -> Result<Option<Box<dyn PcmStream>>, AudioError> {
            let normalized = name.replace('\\', "/");
            let path = if normalized.starts_with("music/") {
                normalized
            } else {
                format!("music/{normalized}")
            };
            let candidates: Vec<String> = if has_music_extension(&path) {
                vec![path]
            } else if family == GameFamily::Q3 {
                vec![format!("{path}.wav"), format!("{path}.ogg")]
            } else {
                vec![format!("{path}.ogg"), format!("{path}.wav")]
            };
            for candidate in &candidates {
                if let Some(stream) = bank.borrow_mut().open_music(candidate, None)? {
                    return Ok(Some(stream));
                }
                if stale() {
                    return Ok(None);
                }
            }
            if family == GameFamily::Q1 {
                if let Some(fallback) = fallback.as_ref() {
                    for candidate in &candidates {
                        if let Some(stream) = fallback.borrow_mut()(candidate)? {
                            return Ok(Some(stream));
                        }
                        if stale() {
                            return Ok(None);
                        }
                    }
                }
            }
            Ok(None)
        };
        let Some(mut intro) = open(&intro_name, &stale)? else {
            if !stale() {
                self.emit(&format!("Music unavailable: {content}/{intro_name}\n"));
            }
            return Ok(());
        };
        if stale() {
            intro.close();
            return Ok(());
        }
        let loop_choice: Option<LoopSource> = if !looping {
            None
        } else if loop_name.as_deref().is_none_or(|name| name.is_empty())
            || loop_name.as_deref() == Some(intro_name.as_str())
        {
            Some(LoopSource::SameAsIntro)
        } else {
            open(loop_name.as_deref().unwrap_or_default(), &stale)?.map(LoopSource::Stream)
        };
        if stale() {
            intro.close();
            if let Some(LoopSource::Stream(mut stream)) = loop_choice {
                stream.close();
            }
            return Ok(());
        }
        if let Some(current) = self.current.as_mut() {
            current.looping = loop_choice.is_some();
            current.cd.player_mut().start(intro, loop_choice);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_client::audio::bank::OpenedSound;
    use qa_client::audio::engine::UnifiedAudioOptions;
    use qa_client::audio::streams::MemoryPcmStream;
    use qa_client::audio::wav::PcmSound;
    use qa_content::catalog::{CatalogProduct, ProductExpectation};
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    struct FakeContent {
        files: HashMap<String, Vec<u8>>,
    }

    impl SoundContent for FakeContent {
        fn open(&mut self, path: &str) -> Option<OpenedSound> {
            self.files.get(path).map(|bytes| OpenedSound {
                id: path.to_string(),
                content: "base".to_string(),
                bytes: bytes.clone(),
            })
        }
    }

    fn wav_bytes() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&40u32.to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&11025u32.to_le_bytes());
        bytes.extend_from_slice(&11025u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&8u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&4u32.to_le_bytes());
        bytes.extend_from_slice(&[0, 128, 255, 64]);
        bytes
    }

    fn pcm(frames: usize) -> PcmSound {
        PcmSound {
            sample_rate: 44100,
            channels: 2,
            samples: vec![0; frames * 2],
            frame_count: frames,
            loop_start: None,
        }
    }

    fn engine() -> UnifiedAudio {
        UnifiedAudio::new(UnifiedAudioOptions {
            sample_rate: Some(44100),
            output_format: None,
            milliseconds: Box::new(|| 0),
            random: Box::new(|| 0),
            max_actors: None,
            on_sound: None,
            device_factory: None,
        })
        .unwrap()
    }

    fn source(family: GameFamily) -> MusicSource {
        MusicSource {
            content: ContentId("q2-classic-baseq2".to_string()),
            family,
            edition: "classic".to_string(),
            campaign: "baseq2".to_string(),
        }
    }

    fn music<'engine>(
        engine: &'engine mut UnifiedAudio,
        printed: Rc<RefCell<Vec<String>>>,
    ) -> ApplicationMusic<'engine, FakeContent> {
        ApplicationMusic::new(
            engine,
            Box::new(move |text: &str| printed.borrow_mut().push(text.to_string())),
            ApplicationMusicOptions {
                random: Some(Box::new(|| 0.0)),
                ..ApplicationMusicOptions::default()
            },
        )
    }

    fn bank_with(files: &[(&str, Vec<u8>)]) -> Rc<RefCell<SoundBank<FakeContent>>> {
        let files = files
            .iter()
            .map(|(path, bytes)| (path.to_string(), bytes.clone()))
            .collect();
        Rc::new(RefCell::new(SoundBank::new(FakeContent { files })))
    }

    #[test]
    fn resolves_world_track() {
        let world = HashMap::from([
            ("music".to_string(), "track8".to_string()),
            ("sounds".to_string(), "track2".to_string()),
        ]);
        let q3 = source(GameFamily::Q3);
        assert_eq!(world_music_track(Some(&world), &q3), "track8");
        let mut rerelease = source(GameFamily::Q2);
        rerelease.edition = "rerelease".to_string();
        assert_eq!(world_music_track(Some(&world), &rerelease), "track8");
        let classic = source(GameFamily::Q2);
        assert_eq!(world_music_track(Some(&world), &classic), "track2");
        assert_eq!(world_music_track(None, &q3), "");
    }

    #[test]
    fn falls_back_across_q1_editions() {
        let product = |id: &str, expectation: &str, installed: bool| CatalogProduct {
            id: ContentId(id.to_string()),
            expectation: ProductExpectation {
                id: expectation.to_string(),
                family: GameFamily::Q1,
                edition: "classic".to_string(),
                campaign: "id1".to_string(),
                title: String::new(),
                content_directory: String::new(),
                base_product: None,
                required_content_archives: Vec::new(),
                required_programs: Vec::new(),
                map_witness: None,
                unresolved_reason: None,
            },
            availability: if installed {
                ProductAvailability::Installed
            } else {
                ProductAvailability::Missing {
                    requirements: vec!["pak0.pak".to_string()],
                }
            },
            archives: Vec::new(),
            loose_root: None,
            user_content: None,
            maps: Vec::new(),
            diagnostics: Vec::new(),
        };
        let catalog = InstalledCatalog::new(
            String::new(),
            vec![
                product("q1-classic-id1", "q1-classic-id1", true),
                product("q1-rerelease-id1", "q1-rerelease-id1", true),
            ],
            Vec::new(),
            0,
            None,
        )
        .unwrap();
        let alternate = q1_music_fallback(&ContentId("q1-classic-id1".to_string()), &catalog).unwrap();
        assert_eq!(alternate, Some(ContentId("q1-rerelease-id1".to_string())));
        let back = q1_music_fallback(&ContentId("q1-rerelease-id1".to_string()), &catalog).unwrap();
        assert_eq!(back, Some(ContentId("q1-classic-id1".to_string())));
        let missing = q1_music_fallback(&ContentId("q2-classic-baseq2".to_string()), &catalog);
        assert!(missing.is_err());
    }

    #[test]
    fn plays_numbered_track_from_bank() {
        let mut audio = engine();
        let printed = Rc::new(RefCell::new(Vec::new()));
        let mut music = music(&mut audio, Rc::clone(&printed));
        let bank = bank_with(&[("music/02.ogg", wav_bytes())]);
        let source = source(GameFamily::Q2);
        music.select(&source, Rc::clone(&bank), None);
        music.cd_command(&["play".to_string(), "2".to_string()]).unwrap();
        assert!(printed.borrow().is_empty());
        music.cd_command(&["info".to_string()]).unwrap();
        assert_eq!(
            printed.borrow().as_slice(),
            ["Currently playing track 2\n", "Volume is 0.25\n"]
        );
    }

    #[test]
    fn cd_command_validates_and_reports() {
        let mut audio = engine();
        let printed = Rc::new(RefCell::new(Vec::new()));
        let mut music = music(&mut audio, Rc::clone(&printed));
        music.cd_command(&[]).unwrap();
        assert!(printed.borrow().is_empty());
        music.cd_command(&["info".to_string()]).unwrap();
        assert_eq!(printed.borrow().as_slice(), ["Not playing.\n", "Volume is 0.25\n"]);
        printed.borrow_mut().clear();
        music.cd_command(&["close".to_string()]).unwrap();
        assert_eq!(
            printed.borrow().as_slice(),
            ["cd close: disc tray operations are unavailable with file-backed music.\n"]
        );
        printed.borrow_mut().clear();
        music.cd_command(&["play".to_string(), "2".to_string()]).unwrap();
        assert_eq!(printed.borrow().as_slice(), ["No soundtrack source selected.\n"]);
        printed.borrow_mut().clear();
        music.cd_command(&["pause".to_string()]).unwrap();
        assert!(printed.borrow().is_empty());
        music.cd_command(&["bogus".to_string()]).unwrap();
        assert_eq!(printed.borrow().as_slice(), ["No soundtrack source selected.\n"]);
    }

    #[test]
    fn music_command_validates() {
        let mut audio = engine();
        let printed = Rc::new(RefCell::new(Vec::new()));
        let mut music = music(&mut audio, Rc::clone(&printed));
        music.music_command(&[]).unwrap();
        music
            .music_command(&["a".to_string(), "b".to_string(), "c".to_string()])
            .unwrap();
        music.music_command(&["  ".to_string()]).unwrap();
        assert_eq!(
            printed.borrow().as_slice(),
            [
                "music <intro> [loop]\n",
                "music <intro> [loop]\n",
                "music <intro> [loop]\n"
            ]
        );
        printed.borrow_mut().clear();
        music.music_command(&["music/win".to_string()]).unwrap();
        assert_eq!(printed.borrow().as_slice(), ["No soundtrack source selected.\n"]);
    }

    #[test]
    fn volume_validates_and_applies() {
        let mut audio = engine();
        let printed = Rc::new(RefCell::new(Vec::new()));
        let mut music = music(&mut audio, Rc::clone(&printed));
        assert_eq!(music.volume(), 0.25);
        assert_eq!(
            music.set_volume(f64::NAN).unwrap_err().to_string(),
            "Invalid music volume"
        );
        assert_eq!(music.set_volume(-1.0).unwrap_err().to_string(), "Invalid music volume");
        music.set_volume(0.5).unwrap();
        assert_eq!(music.volume(), 0.5);
    }

    #[test]
    fn remap_lists_and_validates() {
        let mut audio = engine();
        let printed = Rc::new(RefCell::new(Vec::new()));
        let mut music = music(&mut audio, Rc::clone(&printed));
        music.cd_command(&["remap".to_string()]).unwrap();
        assert!(printed.borrow().is_empty());
        music
            .cd_command(&["remap".to_string(), "3".to_string(), "1".to_string()])
            .unwrap();
        assert!(printed.borrow().is_empty());
        music.cd_command(&["remap".to_string()]).unwrap();
        assert_eq!(printed.borrow().as_slice(), ["  1 -> 3\n  2 -> 1\n"]);
        printed.borrow_mut().clear();
        music.cd_command(&["remap".to_string(), "999".to_string()]).unwrap();
        assert_eq!(
            printed.borrow().as_slice(),
            ["cd remap requires at most 99 track numbers from 0 through 255.\n"]
        );
    }

    #[test]
    fn stop_and_select_keep_live_selection() {
        let mut audio = engine();
        let printed = Rc::new(RefCell::new(Vec::new()));
        let mut music = music(&mut audio, Rc::clone(&printed));
        let bank = bank_with(&[]);
        let source = source(GameFamily::Q2);
        music.select(&source, Rc::clone(&bank), None);
        music.cd_command(&["stop".to_string()]).unwrap();
        music.select(&source, Rc::clone(&bank), None);
        music.stop();
        music.cd_command(&["info".to_string()]).unwrap();
        assert_eq!(printed.borrow().as_slice(), ["Not playing.\n", "Volume is 0.25\n"]);
    }

    #[test]
    fn plays_named_cue_through_fallback() {
        let mut audio = engine();
        let printed = Rc::new(RefCell::new(Vec::new()));
        let mut music = music(&mut audio, Rc::clone(&printed));
        let bank = bank_with(&[]);
        let fallback: SharedOpener = Rc::new(RefCell::new(Box::new(|path: &str| {
            if path == "music/win.ogg" {
                Ok(Some(Box::new(MemoryPcmStream::new(pcm(8))) as Box<dyn PcmStream>))
            } else {
                Ok(None)
            }
        })));
        let source = source(GameFamily::Q1);
        music
            .play(&source, bank, "music/win", Some(Rc::clone(&fallback)), None)
            .unwrap();
        assert!(printed.borrow().is_empty());
        music.cd_command(&["info".to_string()]).unwrap();
        assert_eq!(
            printed.borrow().as_slice(),
            ["Currently looping track music/win\n", "Volume is 0.25\n"]
        );
    }

    #[test]
    fn reports_missing_cue() {
        let mut audio = engine();
        let printed = Rc::new(RefCell::new(Vec::new()));
        let mut music = music(&mut audio, Rc::clone(&printed));
        let bank = bank_with(&[]);
        let source = source(GameFamily::Q2);
        music.play(&source, bank, "music/missing", None, None).unwrap();
        assert_eq!(
            printed.borrow().as_slice(),
            ["Music unavailable: q2-classic-baseq2/music/missing\n"]
        );
    }

    #[test]
    fn shuffles_playlist_without_repeat() {
        let mut audio = engine();
        let printed = Rc::new(RefCell::new(Vec::new()));
        let mut music = music(&mut audio, Rc::clone(&printed));
        let bank = bank_with(&[("music/a.ogg", wav_bytes()), ("music/b.ogg", wav_bytes())]);
        let source = source(GameFamily::Q2);
        let playlist = MusicPlaylist {
            shuffle: true,
            tracks: vec!["music/a".to_string(), "music/b".to_string()],
        };
        let replay_bank = Rc::clone(&bank);
        music.play(&source, bank, "music/a", None, Some(&playlist)).unwrap();
        assert!(printed.borrow().is_empty());
        // Replaying the authored cue while automatic playback runs is a no-op.
        music
            .play(&source, replay_bank, "music/a", None, Some(&playlist))
            .unwrap();
        assert!(printed.borrow().is_empty());
    }
}
