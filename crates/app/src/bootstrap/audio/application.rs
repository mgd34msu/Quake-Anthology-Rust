//! Application audio data (port of donor
//! `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/audio.ts`,
//! data exports only).
//!
//! This port covers the donor's data exports: [`ApplicationAudioCommand`],
//! [`ApplicationEffectSound`], [`ApplicationAudioSeatEvents`], and
//! [`ApplicationAudioOptions`].
//!
//! Scope note: the `ApplicationAudio` class is NOT ported here. The class
//! co-owns a `UnifiedAudio` engine and an `ApplicationMusic` driver, but the
//! Rust [`ApplicationMusic`](super::music::ApplicationMusic) borrows its
//! engine (`&mut UnifiedAudio`) instead of owning or sharing it, so no safe
//! Rust struct — in this shard or the integration lane — can hold both the
//! way the donor does, and per-call music construction would reset track
//! selection, resume, and shuffle state. The class belongs with the audio
//! lane once `ApplicationMusic` ownership is restructured (owned engine or
//! shared handle). `SceneQueries` and `SimulationPresentationEvent` are
//! likewise unported, so [`ApplicationAudioSeatEvents`] stays generic over
//! its scene and event inputs.

use qa_client::audio::music::MusicControls;
use qa_client::audio::output::AudioOutputFormat;
use qa_core::identity::SeatId;
use qa_world::session::WorldSnapshot;
use std::cell::RefCell;
use std::rc::Rc;

/// Shared audio print callback (donor `print?: (text: string) => void`).
pub type SharedAudioPrint = Rc<RefCell<dyn FnMut(&str)>>;

/// Shared team-game query (donor `q3TeamGame?: () => boolean`).
pub type SharedTeamGameQuery = Rc<dyn Fn() -> bool>;

/// Sound registration pick for listings (donor `Pick<SoundRegistration,
/// "path" | "sound">`).
#[derive(Debug, Clone)]
pub struct AudioCommandRegistration {
    /// Requested sound path.
    pub path: String,
    /// Decoded asset, when the registration resolved.
    pub sound: Option<qa_content::q3::presentation::audio::SoundAsset>,
}

/// Audio console command (donor `ApplicationAudioCommand`).
#[derive(Clone)]
pub struct ApplicationAudioCommand {
    /// Command name.
    pub name: String,
    /// Command arguments.
    pub args: Vec<String>,
    /// Originating seat, when local.
    pub seat: Option<SeatId>,
    /// Extra registrations for listings.
    pub registrations: Vec<AudioCommandRegistration>,
    /// Print sink override.
    pub print: Option<SharedAudioPrint>,
}

impl ApplicationAudioCommand {
    /// Borrow a command over a name and arguments.
    pub fn new(name: &str, args: Vec<String>, seat: Option<SeatId>) -> Self {
        Self {
            name: name.to_string(),
            args,
            seat,
            registrations: Vec::new(),
            print: None,
        }
    }
}

/// World effect sound (donor `ApplicationEffectSound`).
pub type ApplicationEffectSound = super::super::media::q3::SourceEffectSound;

/// Per-seat audio frame inputs (donor `ApplicationAudioSeatEvents`).
#[derive(Debug, Clone)]
pub struct ApplicationAudioSeatEvents<E, S> {
    /// Owning seat.
    pub seat: SeatId,
    /// Seat scene queries, when the seat has a scene.
    pub scene: Option<S>,
    /// Seat world snapshot.
    pub snapshot: WorldSnapshot,
    /// Seat presentation events.
    pub events: Vec<E>,
    /// Whether the seat accepts music.
    pub music: bool,
    /// Seat effect sounds.
    pub effect_sounds: Vec<ApplicationEffectSound>,
}

/// Application audio options (donor `ApplicationAudioOptions`).
#[derive(Clone)]
pub struct ApplicationAudioOptions {
    /// Whether the Q3 game is team-based.
    pub q3_team_game: SharedTeamGameQuery,
    /// Shared music controls.
    pub music_controls: Option<MusicControls>,
    /// Output format override.
    pub output_format: Option<AudioOutputFormat>,
    /// Whether device opening is deferred.
    pub defer_output: bool,
    /// Output device name override.
    pub device_name: Option<String>,
    /// Effects volume override.
    pub effects_volume: Option<f64>,
    /// Music volume override.
    pub music_volume: Option<f64>,
}

impl Default for ApplicationAudioOptions {
    fn default() -> Self {
        Self {
            q3_team_game: Rc::new(|| false),
            music_controls: None,
            output_format: None,
            defer_output: false,
            device_name: None,
            effects_volume: None,
            music_volume: None,
        }
    }
}

impl ApplicationAudioOptions {
    /// Whether the Q3 game is team-based (donor default `false`).
    pub fn is_team_game(&self) -> bool {
        (self.q3_team_game)()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seat() -> SeatId {
        qa_core::identity::IdentityOwner::create("audio-application-test")
            .unwrap()
            .seat(0)
    }

    fn snapshot() -> WorldSnapshot {
        WorldSnapshot {
            frame: qa_core::time::FrameContext {
                frame: 0,
                time: qa_core::time::SourceTime::Milliseconds(0),
                elapsed: qa_core::time::SourceTime::Milliseconds(16),
                phase: qa_core::time::FramePhase::FrameEntry,
            },
            actors: Vec::new(),
            bodies: Vec::new(),
            inventories: Vec::new(),
        }
    }

    #[test]
    fn audio_command_defaults_to_no_print_or_registrations() {
        let command = ApplicationAudioCommand::new("s_info", Vec::new(), Some(seat()));
        assert_eq!(command.name, "s_info");
        assert!(command.args.is_empty());
        assert!(command.registrations.is_empty());
        assert!(command.print.is_none());
    }

    #[test]
    fn audio_command_print_override_receives_text() {
        let printed = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&printed);
        let mut command = ApplicationAudioCommand::new("music", vec!["2".to_string()], None);
        command.print = Some(Rc::new(RefCell::new(move |text: &str| {
            sink.borrow_mut().push(text.to_string());
        })));
        (command.print.as_ref().unwrap().borrow_mut())("cue 2\n");
        assert_eq!(printed.borrow().as_slice(), ["cue 2\n"]);
    }

    #[test]
    fn audio_options_default_to_solo_automatic_output() {
        let options = ApplicationAudioOptions::default();
        assert!(!options.is_team_game());
        assert!(options.music_controls.is_none());
        assert!(options.output_format.is_none());
        assert!(!options.defer_output);
        assert!(options.device_name.is_none());
        assert!(options.effects_volume.is_none());
        assert!(options.music_volume.is_none());
    }

    #[test]
    fn audio_options_honor_team_game_query() {
        let options = ApplicationAudioOptions {
            q3_team_game: Rc::new(|| true),
            ..ApplicationAudioOptions::default()
        };
        assert!(options.is_team_game());
    }

    #[test]
    fn seat_events_assemble_frame_inputs() {
        let events: ApplicationAudioSeatEvents<String, String> = ApplicationAudioSeatEvents {
            seat: seat(),
            scene: Some("q2-scene".to_string()),
            snapshot: snapshot(),
            events: vec!["sound".to_string()],
            music: true,
            effect_sounds: Vec::new(),
        };
        assert_eq!(events.events.len(), 1);
        assert!(events.music);
        assert!(events.scene.is_some());
    }
}
