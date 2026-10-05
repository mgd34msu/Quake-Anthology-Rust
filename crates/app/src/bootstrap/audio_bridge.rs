//! Windowed gameplay audio bridge: bank plus sim-event triggers over retained mounts.
//!
//! Donor provenance: `src/app/bootstrap/audio.ts` (`ApplicationAudio.receive`
//! and `frame`, adapted — synchronous, minimal audible slice only).
//!
//! The windowed run opens a [`UnifiedAudio`] engine and pumps it, but nothing
//! ever registered sounds, set listeners, or fed sim sound events to it, so
//! play stayed silent. This bridge closes those three gaps:
//!
//! 1. [`MountsSoundContent`] adapts the retained product mounts (see
//!    [`PlayWorld::audio_mounts`](super::play_world::PlayWorld::audio_mounts))
//!    for one shared [`SoundBank`] covering every family (unified engine
//!    rule: no per-family forks). Precedent: `RetainedSoundContent` in the
//!    Q3 client assets.
//! 2. [`AudioBridge::update_listeners`] sets the local seat/eye listener per
//!    frame, without which `play` mixes zero seats.
//! 3. [`AudioBridge::receive`] maps sim presentation sound events to
//!    `engine.play`/`stop_sound` (q1 `sound`/`stop-sound`, q2 `sound` once
//!    plus `start`/`stop` loops), and
//!    [`AudioBridge::receive_q3_operations`] drains collected
//!    [`Q3SeatAudioOperation`] play/stop operations into the engine.
//!
//! Missing sounds warn once per path (donor `warned`), like the donor. Every
//! engine failure logs to stderr and the run stays silent; nothing here
//! panics.

use std::collections::HashSet;
use std::rc::Rc;

use qa_client::audio::bank::{OpenedSound, SoundBank, SoundContent};
use qa_client::audio::engine::UnifiedAudio;
use qa_client::audio::types::{
    AudioAudience, AudioListener, LoopLifetime, LoopSound, PlaySound, SoundAsset, SoundOrigin,
};
use qa_client::audio::{source_sound_channel, ChannelCommand, SoundFamily};
use qa_content::mounts::MountedContent;
use qa_content::q1::foundation::types::{Q1Event, Q1SoundChannel};
use qa_content::q2::foundation::host::{Q2PresentationEvent, Q2SoundLoop};
use qa_core::identity::{ActorId, ProviderId, SeatId};
use qa_core::math::{angles_to_axis, vec3, Vec3};

use super::audio::q3::Q3SeatAudioOperation;
use super::simulation::types::{SimulationPresentationEvent, SourcePresentationEvent};

/// [`SoundContent`] over shared product mounts for the engine bank.
#[derive(Clone)]
pub struct MountsSoundContent {
    /// Retained mounts (shared with the map world).
    mounts: Rc<MountedContent>,
}

impl MountsSoundContent {
    /// Adapter over retained mounts.
    #[must_use]
    pub fn new(mounts: Rc<MountedContent>) -> Self {
        Self { mounts }
    }
}

impl SoundContent for MountsSoundContent {
    fn open(&mut self, path: &str) -> Option<OpenedSound> {
        let opened = self.mounts.open(path, |_| true).ok()??;
        Some(OpenedSound {
            id: opened.reference.requested_path.clone(),
            content: String::new(),
            bytes: opened.bytes,
        })
    }
}

/// Quake I sound channel name to engine channel number (donor `receive`).
fn q1_channel_number(channel: &Q1SoundChannel) -> i32 {
    match channel {
        Q1SoundChannel::Auto => 0,
        Q1SoundChannel::Weapon => 1,
        Q1SoundChannel::Voice => 2,
        Q1SoundChannel::Item => 3,
        Q1SoundChannel::Body => 4,
        Q1SoundChannel::Raw(number) => *number,
    }
}

/// Gameplay audio bridge: one shared sound bank plus trigger mapping.
///
/// Generic over the bank content so tests can serve synthetic bytes; the
/// windowed run uses [`MountsSoundContent`].
pub struct AudioBridge<Content: SoundContent> {
    /// Shared bank over retained content.
    bank: SoundBank<Content>,
    /// Warn-once keys (`{family:?}:{path}`) for missing sounds.
    warned: HashSet<String>,
}

impl<Content: SoundContent> AudioBridge<Content> {
    /// Bridge over bank content.
    #[must_use]
    pub fn new(content: Content) -> Self {
        Self {
            bank: SoundBank::new(content),
            warned: HashSet::new(),
        }
    }

    /// Set the single local listener from the seat and player eye (donor
    /// `frame` `setListeners`). Without this `play` mixes zero seats.
    pub fn update_listeners(
        &self,
        engine: &mut UnifiedAudio,
        seat: &SeatId,
        eye: Option<(Vec3, Vec3)>,
        actor: Option<&ActorId>,
    ) {
        let (origin, angles) = eye.map_or((vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0)), |(eye, angles)| {
            (eye, angles)
        });
        let listener = AudioListener {
            seat: seat.clone(),
            actor: actor.cloned(),
            origin,
            axis: angles_to_axis(angles),
            gain: 1.0,
            underwater: false,
        };
        if let Err(error) = engine.set_listeners(std::slice::from_ref(&listener)) {
            eprintln!("windowed audio: cannot set listeners ({error})");
        }
    }

    /// Drain one frame of sim presentation sound events into the engine
    /// (donor `receive`, minimal slice): q1 `sound`/`stop-sound`, q2 `sound`
    /// once plus `start`/`stop` loops. Q2 muzzle/entity tables, q3
    /// character sounds, and music/finale tracks belong to follow-up slices.
    pub fn receive(
        &mut self,
        engine: &mut UnifiedAudio,
        events: &[SimulationPresentationEvent],
        audience: &AudioAudience,
    ) {
        let mut started_loop = false;
        for event in events {
            match &event.event {
                SourcePresentationEvent::Q1(event) => self.receive_q1(engine, event, audience),
                SourcePresentationEvent::Q2(event) => {
                    started_loop |= self.receive_q2(engine, event, audience);
                }
                _ => {}
            }
        }
        if started_loop {
            // Q1/Q2 loops queue in the engine until pushed into the seat
            // mixers (`stop_loop` pushes itself; `start_loop` does not).
            if let Err(error) = engine.end_loop_frame() {
                eprintln!("windowed audio: cannot push loop frame ({error})");
            }
        }
    }

    /// Drain collected Q3 seat audio operations into the engine (donor
    /// `frame` cgame drain, play/stop slice). Accepts the seat/owner plus
    /// operations rather than either frame struct so both Q3 producers can
    /// share it. `Loop`/`Position` belong to the persistent-loop slice,
    /// which needs per-frame re-issue like the donor.
    pub fn receive_q3_operations(
        &self,
        engine: &mut UnifiedAudio,
        seat: &SeatId,
        owner: Option<&ProviderId>,
        operations: &[Q3SeatAudioOperation],
    ) {
        for operation in operations {
            match operation {
                Q3SeatAudioOperation::Play { sound } => {
                    if let Err(error) = engine.play(sound) {
                        eprintln!("windowed audio: cannot play q3 sound ({error})");
                    }
                }
                Q3SeatAudioOperation::StopLoop { actor } => {
                    if let Err(error) = engine.stop_q3_seat_loop(seat, actor, owner) {
                        eprintln!("windowed audio: cannot stop q3 loop ({error})");
                    }
                }
                Q3SeatAudioOperation::ClearLoops { kill_all } => {
                    if let Err(error) = engine.clear_q3_seat_loops(seat, *kill_all, owner) {
                        eprintln!("windowed audio: cannot clear q3 loops ({error})");
                    }
                }
                Q3SeatAudioOperation::ReleaseOwner => match owner {
                    Some(owner) => {
                        if let Err(error) = engine.release_q3_seat_owner(seat, owner) {
                            eprintln!("windowed audio: cannot release q3 owner ({error})");
                        }
                    }
                    None => eprintln!("windowed audio: q3 release-owner without a source owner"),
                },
                Q3SeatAudioOperation::Loop { .. } | Q3SeatAudioOperation::Position { .. } => {}
            }
        }
    }

    /// Resolve (and memoize, via the bank) one sound, warning once per
    /// missing path like the donor. Q2 `*` player sounds resolve through
    /// the bank sexed fallback with the default model; per-actor models
    /// from userinfo belong to a follow-up slice.
    fn sound(&mut self, path: &str, family: SoundFamily) -> Option<SoundAsset> {
        let key = format!("{family:?}:{path}");
        let registered = if path.starts_with('*') && family == SoundFamily::Q2 {
            self.bank.register_sexed_sound(path, "male")
        } else {
            self.bank.register(path, family)
        };
        match registered {
            Ok(Some(asset)) => Some(asset),
            Ok(None) => {
                if self.warned.insert(key) {
                    eprintln!("windowed audio: sound unavailable: {path}");
                }
                None
            }
            Err(error) => {
                if self.warned.insert(key) {
                    eprintln!("windowed audio: cannot decode {path} ({error})");
                }
                None
            }
        }
    }

    /// Resolve one path and play it as a one-shot.
    #[allow(clippy::too_many_arguments)]
    fn play_one(
        &mut self,
        engine: &mut UnifiedAudio,
        family: SoundFamily,
        path: &str,
        actor: Option<&ActorId>,
        origin: SoundOrigin,
        channel: i32,
        volume: f64,
        attenuation: f64,
        audience: &AudioAudience,
    ) {
        let Some(asset) = self.sound(path, family) else {
            return;
        };
        let request = PlaySound {
            family,
            sound: asset,
            origin,
            actor: actor.cloned(),
            owner: None,
            channel,
            volume,
            attenuation,
            audience: audience.clone(),
            delay_seconds: None,
            server_milliseconds: None,
        };
        if let Err(error) = engine.play(&request) {
            eprintln!("windowed audio: cannot play {path} ({error})");
        }
    }

    /// Map one Quake I event: `sound` to `play` (channel name to number),
    /// `stop-sound` to `stop_sound`.
    fn receive_q1(&mut self, engine: &mut UnifiedAudio, event: &Q1Event, audience: &AudioAudience) {
        match event {
            Q1Event::Sound {
                origin,
                actor,
                path,
                channel,
                attenuation,
                volume,
            } => {
                let sound_origin = match origin {
                    Some(position) => SoundOrigin::Fixed { position: *position },
                    None => SoundOrigin::Actor { actor: actor.clone() },
                };
                self.play_one(
                    engine,
                    SoundFamily::Q1,
                    path,
                    Some(actor),
                    sound_origin,
                    q1_channel_number(channel),
                    *volume,
                    *attenuation,
                    audience,
                );
            }
            Q1Event::StopSound { actor, channel } => match source_sound_channel(SoundFamily::Q1, *channel) {
                Ok(ChannelCommand::Auto) => {
                    if let Err(error) = engine.stop_sound(actor, None, audience) {
                        eprintln!("windowed audio: cannot stop q1 sound ({error})");
                    }
                }
                Ok(ChannelCommand::Channel(channel)) => {
                    if let Err(error) = engine.stop_sound(actor, Some(channel), audience) {
                        eprintln!("windowed audio: cannot stop q1 sound ({error})");
                    }
                }
                Ok(ChannelCommand::ReplaceActor) => {
                    eprintln!("windowed audio: netquake stop sound requires a nonnegative channel");
                }
                Err(error) => {
                    eprintln!("windowed audio: bad q1 stop channel ({error})");
                }
            },
            _ => {}
        }
    }

    /// Map one Quake II presentation event. Returns whether a loop started
    /// (the caller pushes the loop frame once per batch). `start` registers
    /// a persistent loop: the donor re-issues frame loops every frame, but
    /// this slice has no re-issue pass, so frame lifetime would go quiet.
    /// Actor-follow sounds use the actor origin directly; the donor's
    /// dead-actor fallback to a fixed origin needs a snapshot this bridge
    /// does not carry.
    fn receive_q2(&mut self, engine: &mut UnifiedAudio, event: &Q2PresentationEvent, audience: &AudioAudience) -> bool {
        let Q2PresentationEvent::Sound(sound) = event else {
            return false;
        };
        match &sound.loop_ {
            Q2SoundLoop::Once => {
                let origin = match &sound.actor {
                    Some(actor) => SoundOrigin::Actor { actor: actor.clone() },
                    None => SoundOrigin::Fixed { position: sound.origin },
                };
                self.play_one(
                    engine,
                    SoundFamily::Q2,
                    &sound.path,
                    sound.actor.as_ref(),
                    origin,
                    sound.channel,
                    sound.volume,
                    sound.attenuation,
                    audience,
                );
                false
            }
            Q2SoundLoop::Start => {
                let Some(actor) = sound.actor.as_ref() else {
                    return false;
                };
                if let Err(error) = engine.update_actor(actor, sound.origin) {
                    eprintln!("windowed audio: cannot place q2 loop actor ({error})");
                    return false;
                }
                let Some(asset) = self.sound(&sound.path, SoundFamily::Q2) else {
                    return false;
                };
                let request = LoopSound {
                    family: SoundFamily::Q2,
                    sound: asset,
                    origin: SoundOrigin::Actor { actor: actor.clone() },
                    actor: actor.clone(),
                    owner: sound.loop_owner.clone(),
                    velocity: vec3(0.0, 0.0, 0.0),
                    frame_number: 0,
                    volume: sound.volume,
                    attenuation: sound.attenuation,
                    lifetime: LoopLifetime::Persistent,
                    audience: audience.clone(),
                };
                if let Err(error) = engine.start_loop(&request) {
                    eprintln!("windowed audio: cannot start q2 loop ({error})");
                    return false;
                }
                true
            }
            Q2SoundLoop::Stop => {
                if let Some(actor) = sound.actor.as_ref() {
                    if let Err(error) = engine.stop_loop(actor, audience, sound.loop_owner.as_ref()) {
                        eprintln!("windowed audio: cannot stop q2 loop ({error})");
                    }
                }
                false
            }
        }
    }

    /// Warn-once keys recorded so far.
    #[cfg(test)]
    fn warned_count(&self) -> usize {
        self.warned.len()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use qa_client::audio::engine::{AudioDeviceFactory, DeviceOpenError, UnifiedAudio, UnifiedAudioOptions};
    use qa_client::audio::error::AudioError;
    use qa_client::audio::output::AudioOutputFormat;
    use qa_client::audio::wav::PcmSound;
    use qa_content::contract::ContentId;
    use qa_core::identity::{IdentityOwner, ProviderId};

    use super::*;

    /// In-memory content serving synthetic files to the bank.
    struct FakeContent {
        files: HashMap<String, Vec<u8>>,
    }

    impl SoundContent for FakeContent {
        fn open(&mut self, path: &str) -> Option<OpenedSound> {
            let bytes = self.files.get(path)?.clone();
            Some(OpenedSound {
                id: path.to_string(),
                content: String::new(),
                bytes,
            })
        }
    }

    /// Device factory that never opens: the tests mix PCM without a device.
    struct FakeFactory;

    impl AudioDeviceFactory for FakeFactory {
        fn open(
            &self,
            _device_name: Option<&str>,
            _format: &AudioOutputFormat,
            _buffer_frames: Option<u32>,
        ) -> Result<Box<dyn qa_client::audio::engine::AudioOutputDevice>, DeviceOpenError> {
            Err(DeviceOpenError::Unavailable("headless".to_string()))
        }

        fn output_names(&self) -> Result<Vec<String>, AudioError> {
            Ok(Vec::new())
        }
    }

    /// Minimal PCM16 mono WAV bytes around raw samples.
    fn wav_bytes(samples: &[i16]) -> Vec<u8> {
        let data_len = samples.len() * 2;
        let mut bytes = Vec::with_capacity(44 + data_len);
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data_len as u32).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&44100u32.to_le_bytes());
        bytes.extend_from_slice(&(44100u32 * 2).to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&(data_len as u32).to_le_bytes());
        for sample in samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        bytes
    }

    fn engine() -> UnifiedAudio {
        UnifiedAudio::new(UnifiedAudioOptions {
            sample_rate: None,
            output_format: None,
            milliseconds: Box::new(|| 0),
            random: Box::new(|| 7),
            max_actors: None,
            on_sound: None,
            device_factory: Some(Rc::new(FakeFactory)),
        })
        .unwrap()
    }

    /// Bridge whose bank serves one synthetic blip.
    fn bridge() -> AudioBridge<FakeContent> {
        let mut files = HashMap::new();
        files.insert("sound/test/blip.wav".to_string(), wav_bytes(&vec![6000i16; 512]));
        AudioBridge::new(FakeContent { files })
    }

    fn wrap(event: SourcePresentationEvent) -> SimulationPresentationEvent {
        SimulationPresentationEvent {
            event,
            owner: None,
            recipient: None,
            sequence: 1,
            content: ContentId("test".to_string()),
            seconds: 0.0,
            source_entity: None,
        }
    }

    fn q1_sound(actor: &ActorId, path: &str) -> SimulationPresentationEvent {
        wrap(SourcePresentationEvent::Q1(Q1Event::Sound {
            origin: Some(vec3(10.0, 0.0, 0.0)),
            actor: actor.clone(),
            path: path.to_string(),
            channel: Q1SoundChannel::Auto,
            attenuation: 1.0,
            volume: 1.0,
        }))
    }

    fn q2_sound(actor: Option<&ActorId>, path: &str, loop_: Q2SoundLoop) -> SimulationPresentationEvent {
        wrap(SourcePresentationEvent::Q2(Q2PresentationEvent::Sound(
            qa_content::q2::foundation::host::Q2SoundEvent {
                actor: actor.cloned(),
                origin: vec3(10.0, 0.0, 0.0),
                path: path.to_string(),
                channel: 1,
                volume: 1.0,
                attenuation: 1.0,
                reliable: false,
                loop_,
                loop_owner: None,
            },
        )))
    }

    fn synthetic_asset() -> SoundAsset {
        SoundAsset {
            resource: "test".to_string(),
            name: "blip".to_string(),
            pcm: Rc::new(PcmSound {
                sample_rate: 44100,
                channels: 1,
                samples: vec![6000i16; 512],
                frame_count: 512,
                loop_start: None,
            }),
        }
    }

    #[test]
    fn q1_channel_names_map_to_numbers() {
        assert_eq!(q1_channel_number(&Q1SoundChannel::Auto), 0);
        assert_eq!(q1_channel_number(&Q1SoundChannel::Weapon), 1);
        assert_eq!(q1_channel_number(&Q1SoundChannel::Voice), 2);
        assert_eq!(q1_channel_number(&Q1SoundChannel::Item), 3);
        assert_eq!(q1_channel_number(&Q1SoundChannel::Body), 4);
        assert_eq!(q1_channel_number(&Q1SoundChannel::Raw(-1)), -1);
        assert_eq!(q1_channel_number(&Q1SoundChannel::Raw(7)), 7);
    }

    #[test]
    fn q1_sound_event_mixes_audible_pcm() {
        let owner = IdentityOwner::create("audio-bridge-test").unwrap();
        let seat = owner.seat(0);
        let actor = owner.actor(1, 0);
        let mut engine = engine();
        let mut bridge = bridge();
        bridge.update_listeners(
            &mut engine,
            &seat,
            Some((vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0))),
            Some(&actor),
        );
        bridge.receive(&mut engine, &[q1_sound(&actor, "test/blip.wav")], &AudioAudience::World);
        let mixed = engine.mix(256).unwrap();
        assert_eq!(mixed.len(), 512);
        assert!(
            mixed.iter().any(|sample| *sample != 0),
            "bridged q1 sound mixed silence"
        );
    }

    #[test]
    fn play_without_listeners_mixes_silence() {
        let owner = IdentityOwner::create("audio-bridge-test").unwrap();
        let seat = owner.seat(0);
        let actor = owner.actor(1, 0);
        let mut engine = engine();
        let mut bridge = bridge();
        let event = q1_sound(&actor, "test/blip.wav");
        bridge.receive(&mut engine, &[event.clone()], &AudioAudience::World);
        let silent = engine.mix(256).unwrap();
        assert!(
            silent.iter().all(|sample| *sample == 0),
            "play without listeners should mix silence"
        );
        bridge.update_listeners(
            &mut engine,
            &seat,
            Some((vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0))),
            Some(&actor),
        );
        bridge.receive(&mut engine, &[event], &AudioAudience::World);
        let heard = engine.mix(256).unwrap();
        assert!(
            heard.iter().any(|sample| *sample != 0),
            "play with listeners should mix sound"
        );
    }

    #[test]
    fn missing_path_warns_once_and_stays_silent() {
        let owner = IdentityOwner::create("audio-bridge-test").unwrap();
        let seat = owner.seat(0);
        let actor = owner.actor(1, 0);
        let mut engine = engine();
        let mut bridge = bridge();
        bridge.update_listeners(&mut engine, &seat, None, None);
        let event = q1_sound(&actor, "missing/nope.wav");
        bridge.receive(&mut engine, &[event.clone()], &AudioAudience::World);
        bridge.receive(&mut engine, &[event], &AudioAudience::World);
        assert_eq!(bridge.warned_count(), 1);
        let mixed = engine.mix(64).unwrap();
        assert!(mixed.iter().all(|sample| *sample == 0));
    }

    #[test]
    fn q1_stop_sound_silences_its_channel() {
        let owner = IdentityOwner::create("audio-bridge-test").unwrap();
        let seat = owner.seat(0);
        let actor = owner.actor(1, 0);
        let mut engine = engine();
        let mut bridge = bridge();
        bridge.update_listeners(
            &mut engine,
            &seat,
            Some((vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0))),
            Some(&actor),
        );
        let start = wrap(SourcePresentationEvent::Q1(Q1Event::Sound {
            origin: Some(vec3(10.0, 0.0, 0.0)),
            actor: actor.clone(),
            path: "test/blip.wav".to_string(),
            channel: Q1SoundChannel::Weapon,
            attenuation: 1.0,
            volume: 1.0,
        }));
        let stop = wrap(SourcePresentationEvent::Q1(Q1Event::StopSound {
            actor: actor.clone(),
            channel: 1,
        }));
        bridge.receive(&mut engine, &[start, stop], &AudioAudience::World);
        let mixed = engine.mix(256).unwrap();
        assert!(
            mixed.iter().all(|sample| *sample == 0),
            "stopped q1 sound should mix silence"
        );
    }

    #[test]
    fn q2_once_and_loop_start_stop() {
        let owner = IdentityOwner::create("audio-bridge-test").unwrap();
        let seat = owner.seat(0);
        let actor = owner.actor(1, 0);
        let mut engine = engine();
        let mut bridge = bridge();
        bridge.update_listeners(
            &mut engine,
            &seat,
            Some((vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0))),
            Some(&actor),
        );
        bridge.receive(
            &mut engine,
            &[q2_sound(None, "test/blip.wav", Q2SoundLoop::Once)],
            &AudioAudience::World,
        );
        let once = engine.mix(256).unwrap();
        assert!(once.iter().any(|sample| *sample != 0), "q2 once sound mixed silence");
        bridge.receive(
            &mut engine,
            &[q2_sound(Some(&actor), "test/blip.wav", Q2SoundLoop::Start)],
            &AudioAudience::World,
        );
        let looping = engine.mix(256).unwrap();
        assert!(looping.iter().any(|sample| *sample != 0), "q2 loop start mixed silence");
        bridge.receive(
            &mut engine,
            &[q2_sound(Some(&actor), "test/blip.wav", Q2SoundLoop::Stop)],
            &AudioAudience::World,
        );
        // Actorless loop edges are ignored, never fatal.
        bridge.receive(
            &mut engine,
            &[q2_sound(None, "test/blip.wav", Q2SoundLoop::Start)],
            &AudioAudience::World,
        );
        bridge.receive(
            &mut engine,
            &[q2_sound(None, "test/blip.wav", Q2SoundLoop::Stop)],
            &AudioAudience::World,
        );
        assert!(engine.mix(64).is_ok());
    }

    #[test]
    fn q3_play_and_stop_operations_drain() {
        let owner = IdentityOwner::create("audio-bridge-test").unwrap();
        let seat = owner.seat(0);
        let actor = owner.actor(1, 0);
        let provider = ProviderId::new("q3", "test");
        let mut engine = engine();
        let bridge = bridge();
        bridge.update_listeners(
            &mut engine,
            &seat,
            Some((vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0))),
            Some(&actor),
        );
        let play = PlaySound {
            family: SoundFamily::Q3,
            sound: synthetic_asset(),
            origin: SoundOrigin::Local,
            actor: None,
            owner: None,
            channel: 0,
            volume: 1.0,
            attenuation: 0.0,
            audience: AudioAudience::World,
            delay_seconds: None,
            server_milliseconds: None,
        };
        bridge.receive_q3_operations(&mut engine, &seat, None, &[Q3SeatAudioOperation::Play { sound: play }]);
        let mixed = engine.mix(256).unwrap();
        assert!(
            mixed.iter().any(|sample| *sample != 0),
            "q3 play operation mixed silence"
        );
        bridge.receive_q3_operations(
            &mut engine,
            &seat,
            Some(&provider),
            &[
                Q3SeatAudioOperation::StopLoop { actor: actor.clone() },
                Q3SeatAudioOperation::ClearLoops { kill_all: true },
                Q3SeatAudioOperation::ReleaseOwner,
            ],
        );
        // Ownerless release warns instead of panicking.
        bridge.receive_q3_operations(&mut engine, &seat, None, &[Q3SeatAudioOperation::ReleaseOwner]);
        assert!(engine.mix(64).is_ok());
    }
}
