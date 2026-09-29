//! Unified audio engine: seats, stream/music buses, and device output.
//!
//! Donor provenance: `src/audio/engine.ts` (`UnifiedAudio`). Each
//! listener owns a mixer, reverb, underwater filter, and loop map; raw
//! streams and music players mix on top; one output device (or a
//! detached queue) consumes the sum. The SDL device is hidden behind
//! [`AudioOutputDevice`] so headless tests inject fakes.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use qa_core::identity::{ActorId, ProviderId, SeatId};
use qa_core::math::Vec3;
use qa_platform::audio::{AudioChannels, AudioSampleBits, AudioState, SdlAudioDevice, SdlAudioOptions};

use super::environments::{AudioTraceQuery, EnvironmentReverb, ReverbEnvironment};
use super::error::AudioError;
use super::mixer::{AudioMixer, FrameLoopingSoundOptions, MixRequest, MixerVoiceEvent, MixerVoiceOrigin, Q2SoundOptions, RealLoopingSoundOptions, SourceSoundOptions};
use super::music::MusicPlayer;
use super::output::{audio_output_format, encode_output_pcm, resample_queued_pcm, to_int16, AudioOutputFormat, EncodedPcm, DEFAULT_AUDIO_OUTPUT_FORMAT};
use super::reverb::{StereoReverb, UnderwaterFilter};
use super::streams::{RawAudioStream, RawCheckpoint};
use super::types::{AudioAudience, AudioListener, AudioStreamTarget, AudioVoiceClock, AudioVoiceEvent, LoopLifetime, LoopSound, PlaySound, SoundAsset, SoundOrigin, StreamPcm, StreamSamples};
use crate::audio::{source_sound_channel, SoundChannel, SoundFamily};

/// Default seat mixer capacity (96 voices).
const SEAT_MIXER_CAPACITY: usize = 96;
/// Default actor/entity capacity.
const DEFAULT_MAX_ACTORS: usize = 65536;
/// Underwater high-frequency gain used by the engine mix.
const UNDERWATER_GAIN: f64 = 0.25;

/// Geometry transmission: occlusion gain for a listener and position.
pub type GeometryTransmission = Box<dyn FnMut(&AudioListener, Vec3) -> f64>;

/// Accepted one-shot notification.
pub type OnSoundCallback = Box<dyn FnMut(&PlaySound)>;

/// Output device open failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceOpenError {
    /// No usable native device; `select_output` may retry after releasing.
    Unavailable(String),
    /// Any other failure.
    Failed(String),
}

impl DeviceOpenError {
    fn audio_error(&self) -> AudioError {
        match self {
            DeviceOpenError::Unavailable(detail) | DeviceOpenError::Failed(detail) => AudioError::Device(detail.clone()),
        }
    }
}

/// Output device behind the engine (real SDL or a headless fake).
pub trait AudioOutputDevice {
    /// Device name, or `None` for the default.
    fn device_name(&self) -> Option<String>;
    /// Sample rate in Hz.
    fn sample_rate(&self) -> u32;
    /// Channel count (1 or 2).
    fn channels(&self) -> u8;
    /// Sample bits (8 or 16).
    fn sample_bits(&self) -> u8;
    /// Buffer duration in input frames.
    fn buffer_frames(&self) -> u64;
    /// Maximum queue depth in frames.
    fn max_queued_frames(&self) -> u64;
    /// Queued input frames.
    fn queued_frames(&self) -> Result<u64, AudioError>;
    /// Logical playback position in frames.
    fn playback_frames(&self) -> Result<u64, AudioError>;
    /// Queue stereo 16-bit PCM, converting to the device format.
    fn queue(&mut self, samples: &[i16], format: &AudioOutputFormat) -> Result<(), AudioError>;
    /// Drop queued audio.
    fn clear(&mut self) -> Result<(), AudioError>;
    /// Pause playback.
    fn pause(&mut self) -> Result<(), AudioError>;
    /// Resume playback.
    fn resume(&mut self) -> Result<(), AudioError>;
    /// Whether the device is playing.
    fn playing(&self) -> bool;
    /// Close the device.
    fn close_box(self: Box<Self>);
}

/// Opens output devices and lists their names.
pub trait AudioDeviceFactory {
    /// Open an output device.
    fn open(&self, device_name: Option<&str>, format: &AudioOutputFormat, buffer_frames: Option<u32>) -> Result<Box<dyn AudioOutputDevice>, DeviceOpenError>;
    /// List output device names.
    fn output_names(&self) -> Result<Vec<String>, AudioError>;
}

fn platform_error(error: qa_platform::error::Error) -> AudioError {
    AudioError::Device(error.to_string())
}

/// SDL-backed output device.
pub struct SdlOutputDevice {
    device: SdlAudioDevice,
}

impl AudioOutputDevice for SdlOutputDevice {
    fn device_name(&self) -> Option<String> {
        self.device.device_name().map(str::to_string)
    }

    fn sample_rate(&self) -> u32 {
        self.device.sample_rate()
    }

    fn channels(&self) -> u8 {
        match self.device.channels() {
            AudioChannels::Mono => 1,
            AudioChannels::Stereo => 2,
        }
    }

    fn sample_bits(&self) -> u8 {
        match self.device.sample_bits() {
            AudioSampleBits::B8 => 8,
            AudioSampleBits::B16 => 16,
        }
    }

    fn buffer_frames(&self) -> u64 {
        self.device.buffer_frames()
    }

    fn max_queued_frames(&self) -> u64 {
        self.device.max_queued_frames()
    }

    fn queued_frames(&self) -> Result<u64, AudioError> {
        self.device.queued_frames().map_err(platform_error)
    }

    fn playback_frames(&self) -> Result<u64, AudioError> {
        self.device.playback_frames().map_err(platform_error)
    }

    fn queue(&mut self, samples: &[i16], format: &AudioOutputFormat) -> Result<(), AudioError> {
        match encode_output_pcm(samples, format)? {
            EncodedPcm::S16(samples) => self.device.queue_i16(&samples).map_err(platform_error),
            EncodedPcm::U8(samples) => self.device.queue_u8(&samples).map_err(platform_error),
        }
    }

    fn clear(&mut self) -> Result<(), AudioError> {
        self.device.clear().map_err(platform_error)
    }

    fn pause(&mut self) -> Result<(), AudioError> {
        self.device.pause().map_err(platform_error)
    }

    fn resume(&mut self) -> Result<(), AudioError> {
        self.device.resume().map_err(platform_error)
    }

    fn playing(&self) -> bool {
        self.device.state() == AudioState::Playing
    }

    fn close_box(self: Box<Self>) {
        let mut owned = *self;
        owned.device.close();
    }
}

/// SDL-backed device factory.
pub struct SdlDeviceFactory;

impl AudioDeviceFactory for SdlDeviceFactory {
    fn open(&self, device_name: Option<&str>, format: &AudioOutputFormat, buffer_frames: Option<u32>) -> Result<Box<dyn AudioOutputDevice>, DeviceOpenError> {
        let options = SdlAudioOptions {
            sample_rate: format.sample_rate,
            channels: if format.channels == 1 {
                AudioChannels::Mono
            } else {
                AudioChannels::Stereo
            },
            sample_bits: if format.sample_bits == 8 {
                AudioSampleBits::B8
            } else {
                AudioSampleBits::B16
            },
            device_name: device_name.map(str::to_string),
            buffer_frames: buffer_frames.unwrap_or(1024),
        };
        match SdlAudioDevice::open(&options) {
            Ok(device) => Ok(Box::new(SdlOutputDevice { device })),
            Err(error) => Err(if matches!(error, qa_platform::error::Error::Unavailable { .. }) {
                DeviceOpenError::Unavailable(error.to_string())
            } else {
                DeviceOpenError::Failed(error.to_string())
            }),
        }
    }

    fn output_names(&self) -> Result<Vec<String>, AudioError> {
        SdlAudioDevice::output_device_names().map_err(platform_error)
    }
}

/// Unified audio options.
pub struct UnifiedAudioOptions {
    /// Mix sample rate in Hz (default 44100).
    pub sample_rate: Option<u32>,
    /// Output format (defaults to the mix rate, stereo, 16-bit).
    pub output_format: Option<AudioOutputFormat>,
    /// Source allocation clock in whole milliseconds.
    pub milliseconds: Box<dyn Fn() -> i64>,
    /// Session-owned random source for Q1 same-frame phase offsets.
    pub random: Box<dyn FnMut() -> i64>,
    /// Maximum actors (default 65536).
    pub max_actors: Option<usize>,
    /// Called with each accepted one-shot request.
    pub on_sound: Option<OnSoundCallback>,
    /// Device factory (defaults to SDL; tests inject fakes).
    pub device_factory: Option<Rc<dyn AudioDeviceFactory>>,
}

/// Output state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputState {
    /// No device.
    Detached,
    /// Device open but paused.
    Paused,
    /// Device playing.
    Playing,
    /// Engine closed.
    Closed,
}

/// Live output configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputConfiguration {
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Channel count.
    pub channels: u8,
    /// Sample bits.
    pub sample_bits: u8,
    /// Device name, or `None` for the default.
    pub device_name: Option<String>,
    /// Buffer duration in input frames.
    pub buffer_frames: u64,
    /// Maximum queue depth in frames.
    pub maximum_queued_frames: u64,
}

struct SeatAudio {
    listener: Rc<RefCell<AudioListener>>,
    mixer: AudioMixer,
    reverb: StereoReverb,
    underwater: UnderwaterFilter,
    environment: Option<EnvironmentReverb>,
    loops: HashMap<String, LoopSound>,
}

struct StreamBus {
    target: AudioStreamTarget,
    stream: RawAudioStream,
}

struct MusicBus {
    target: AudioStreamTarget,
    player: MusicPlayer,
}

struct RoundMixer {
    seat: SeatId,
    mixer: AudioMixer,
}

struct ActorEntry {
    actor: ActorId,
    owner: Option<ProviderId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DetachedOutput {
    playing: bool,
    buffer_frames: u64,
    device_name: Option<String>,
}

struct VoiceObserver {
    id: u64,
    callback: Box<dyn FnMut(AudioVoiceEvent)>,
}

/// Mix-domain state, split so output resampling can borrow it for refills.
struct EngineMix {
    seats: Vec<SeatAudio>,
    streams: HashMap<String, StreamBus>,
    music: HashMap<String, MusicBus>,
    frame: i64,
}

fn selected(audience: &AudioAudience, seat: &SeatId) -> bool {
    match audience {
        AudioAudience::World => true,
        AudioAudience::Seat { seat: target } => target == seat,
    }
}

fn family_tag(family: SoundFamily) -> &'static str {
    match family {
        SoundFamily::Q1 => "q1",
        SoundFamily::Q2 => "q2",
        SoundFamily::Q3 => "q3",
    }
}

fn loop_key(family: SoundFamily, entity: i64, owner: Option<&ProviderId>) -> String {
    match owner {
        None => format!("{}:{entity}", family_tag(family)),
        Some(owner) => format!("{}:{entity}:{}:{}", family_tag(family), owner.namespace, owner.name),
    }
}

fn with_seat(event: MixerVoiceEvent, seat: &SeatId) -> AudioVoiceEvent {
    match event {
        MixerVoiceEvent::Start {
            voice_id,
            sound,
            output_sample,
            sample_rate,
            source_offset_seconds,
        } => AudioVoiceEvent::Start {
            seat: seat.clone(),
            voice_id,
            sound,
            output_sample,
            sample_rate,
            source_offset_seconds,
        },
        MixerVoiceEvent::Stop {
            voice_id,
            output_sample,
            reason,
        } => AudioVoiceEvent::Stop {
            seat: seat.clone(),
            voice_id,
            output_sample,
            reason,
        },
    }
}

fn add_f64(output: &mut [f64], input: &[f64], gain: f64) -> Result<(), AudioError> {
    if output.len() != input.len() {
        return Err(AudioError::BusLength);
    }
    for (slot, sample) in output.iter_mut().zip(input.iter()) {
        *slot += *sample * gain;
    }
    Ok(())
}

fn add_i16(output: &mut [f64], input: &[i16], gain: f64) -> Result<(), AudioError> {
    if output.len() != input.len() {
        return Err(AudioError::BusLength);
    }
    for (slot, sample) in output.iter_mut().zip(input.iter()) {
        *slot += f64::from(*sample) * gain;
    }
    Ok(())
}

/// One output device; each listener owns a voice core and acoustic state.
pub struct UnifiedAudio {
    sample_rate: u32,
    mix: EngineMix,
    voice_observers: Rc<RefCell<Vec<VoiceObserver>>>,
    next_observer: u64,
    voice_id_source: Rc<RefCell<u64>>,
    geometry: Rc<RefCell<Option<GeometryTransmission>>>,
    round_mixers: Vec<RoundMixer>,
    actors: Vec<ActorEntry>,
    actor_aliases: HashMap<i64, Vec<i64>>,
    doppler_enabled: bool,
    positions: HashMap<i64, Vec3>,
    device: Option<Box<dyn AudioOutputDevice>>,
    detached_output: Option<DetachedOutput>,
    queued_pcm: Vec<i16>,
    format: AudioOutputFormat,
    output_stream: RawAudioStream,
    output_source_frame: i64,
    closed: bool,
    paused: bool,
    effects_gain: f64,
    output_started: bool,
    output_handoff_pending: bool,
    previous_pump_frame: Option<u64>,
    pump_intervals: Vec<i64>,
    milliseconds: Rc<dyn Fn() -> i64>,
    random: Box<dyn FnMut() -> i64>,
    max_actors: usize,
    on_sound: Option<OnSoundCallback>,
    factory: Rc<dyn AudioDeviceFactory>,
}

impl EngineMix {
    fn audience_gain(&self, audience: &AudioAudience) -> f64 {
        match audience {
            AudioAudience::World => 1.0,
            AudioAudience::Seat { seat } => self
                .seats
                .iter()
                .find(|state| state.listener.borrow().seat == *seat)
                .map_or(0.0, |state| state.listener.borrow().gain),
        }
    }

    fn mix(&mut self, frames: usize, sample_rate: u32) -> Result<Vec<i16>, AudioError> {
        if frames > sample_rate as usize * 2 {
            return Err(AudioError::MixTooLarge);
        }
        let mut output = vec![0.0f64; frames * 2];
        for state in &mut self.seats {
            let mut seat = vec![0.0f64; frames * 2];
            let mixed = state.mixer.mix(MixRequest::Consume(frames as i64))?;
            add_i16(&mut seat, &mixed, 1.0)?;
            if let Some(params) = state.environment.as_ref().and_then(|environment| environment.params()) {
                state.reverb.process(&mut seat, &params)?;
            }
            let listener = state.listener.borrow().clone();
            if listener.underwater {
                state.underwater.process(&mut seat, UNDERWATER_GAIN);
            } else {
                state.underwater.reset();
            }
            add_f64(&mut output, &seat, listener.gain)?;
        }
        let mut streams: Vec<(AudioStreamTarget, Vec<f64>)> = Vec::new();
        for bus in self.streams.values_mut() {
            let mixed = bus.stream.mix(frames, bus.target.gain, None)?;
            streams.push((bus.target.clone(), mixed));
        }
        for (target, mixed) in &streams {
            add_f64(&mut output, mixed, self.audience_gain(&target.audience))?;
        }
        let mut music: Vec<(f64, AudioAudience, Vec<f64>)> = Vec::new();
        for bus in self.music.values_mut() {
            let mixed = bus.player.mix(frames)?;
            music.push((bus.target.gain, bus.target.audience.clone(), mixed));
        }
        for (gain, audience, mixed) in &music {
            add_f64(&mut output, mixed, *gain * self.audience_gain(audience))?;
        }
        self.frame += frames as i64;
        Ok(output.iter().map(|value| (value.trunc() as i64).clamp(-32768, 32767) as i16).collect())
    }
}

impl UnifiedAudio {
    /// Engine with borrowed clocks and an optional device factory.
    pub fn new(options: UnifiedAudioOptions) -> Result<Self, AudioError> {
        let sample_rate = options.sample_rate.unwrap_or(44100);
        if !(8000..=192000).contains(&sample_rate) {
            return Err(AudioError::BadOutputRate);
        }
        let format = match options.output_format {
            None => AudioOutputFormat {
                sample_rate,
                ..DEFAULT_AUDIO_OUTPUT_FORMAT
            },
            Some(format) => audio_output_format(i64::from(format.sample_rate), i64::from(format.channels), i64::from(format.sample_bits))?,
        };
        Ok(Self {
            sample_rate,
            mix: EngineMix {
                seats: Vec::new(),
                streams: HashMap::new(),
                music: HashMap::new(),
                frame: 0,
            },
            voice_observers: Rc::new(RefCell::new(Vec::new())),
            next_observer: 0,
            voice_id_source: Rc::new(RefCell::new(0)),
            geometry: Rc::new(RefCell::new(None)),
            round_mixers: Vec::new(),
            actors: Vec::new(),
            actor_aliases: HashMap::new(),
            doppler_enabled: true,
            positions: HashMap::new(),
            device: None,
            detached_output: None,
            queued_pcm: Vec::new(),
            output_stream: RawAudioStream::new(format.sample_rate),
            output_source_frame: 0,
            format,
            closed: false,
            paused: false,
            effects_gain: 0.7,
            output_started: false,
            output_handoff_pending: false,
            previous_pump_frame: None,
            pump_intervals: Vec::new(),
            milliseconds: {
                let clock = options.milliseconds;
                // The closure re-wraps the box; `Rc::new(clock)` would keep the box.
                #[allow(clippy::redundant_closure)]
                Rc::new(move || clock())
            },
            random: options.random,
            max_actors: options.max_actors.unwrap_or(DEFAULT_MAX_ACTORS),
            on_sound: options.on_sound,
            factory: options.device_factory.unwrap_or_else(|| Rc::new(SdlDeviceFactory)),
        })
    }

    /// Mix sample rate in Hz.
    #[must_use]
    pub const fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Output format.
    #[must_use]
    pub const fn output_format(&self) -> AudioOutputFormat {
        self.format
    }

    /// Mix clock in frames.
    #[must_use]
    pub const fn sample_clock(&self) -> i64 {
        self.mix.frame
    }

    /// Queued device frames (zero when detached).
    pub fn queued_frames(&self) -> Result<u64, AudioError> {
        self.device.as_ref().map_or(Ok(0), |device| device.queued_frames())
    }

    /// Selected output device name.
    #[must_use]
    pub fn selected_output(&self) -> Option<String> {
        match self.device.as_ref() {
            Some(device) => device.device_name(),
            None => self.detached_output.as_ref().and_then(|detached| detached.device_name.clone()),
        }
    }

    /// Retained output PCM (device queue depth, or everything when detached).
    pub fn pending_output(&self) -> Result<Vec<i16>, AudioError> {
        let samples = match self.device.as_ref() {
            Some(device) => device.queued_frames()? * 2,
            None => self.queued_pcm.len() as u64,
        };
        let start = self.queued_pcm.len().saturating_sub(usize::try_from(samples).unwrap_or(usize::MAX));
        Ok(self.queued_pcm[start..].to_vec())
    }

    /// List output device names.
    pub fn output_device_names(&self) -> Result<Vec<String>, AudioError> {
        self.ensure_open()?;
        self.factory.output_names()
    }

    /// Output state.
    #[must_use]
    pub fn output_state(&self) -> OutputState {
        if self.closed {
            return OutputState::Closed;
        }
        match self.device.as_ref() {
            None => OutputState::Detached,
            Some(device) if device.playing() => OutputState::Playing,
            Some(_) => OutputState::Paused,
        }
    }

    /// Live output configuration, if a device is open.
    #[must_use]
    pub fn output_configuration(&self) -> Option<OutputConfiguration> {
        self.device.as_ref().map(|device| OutputConfiguration {
            sample_rate: device.sample_rate(),
            channels: device.channels(),
            sample_bits: device.sample_bits(),
            device_name: device.device_name(),
            buffer_frames: device.buffer_frames(),
            maximum_queued_frames: device.max_queued_frames(),
        })
    }

    /// Observe voice starts and stops; returns an observer token.
    pub fn observe_voices(&mut self, observer: Box<dyn FnMut(AudioVoiceEvent)>) -> Result<u64, AudioError> {
        self.ensure_open()?;
        self.next_observer += 1;
        let id = self.next_observer;
        self.voice_observers.borrow_mut().push(VoiceObserver { id, callback: observer });
        Ok(id)
    }

    /// Remove a voice observer.
    pub fn unobserve_voices(&mut self, id: u64) {
        self.voice_observers.borrow_mut().retain(|entry| entry.id != id);
    }

    /// Voice clock snapshot.
    pub fn voice_clock(&self) -> Result<AudioVoiceClock, AudioError> {
        let queued = match self.device.as_ref() {
            Some(device) => device.queued_frames()?,
            None if self.detached_output.is_none() => 0,
            None => self.queued_pcm.len() as u64 / 2,
        };
        let lag = (queued as f64 * f64::from(self.sample_rate) / f64::from(self.format.sample_rate)).ceil() as i64;
        Ok(AudioVoiceClock {
            output_sample: (self.mix.frame - lag).max(0),
            sample_rate: self.sample_rate,
            paused: self.paused,
        })
    }

    fn ensure_open(&self) -> Result<(), AudioError> {
        if self.closed {
            return Err(AudioError::Closed);
        }
        Ok(())
    }

    /// Set geometry transmission for every seat mixer.
    pub fn set_geometry_transmission(&mut self, geometry: Option<GeometryTransmission>) -> Result<(), AudioError> {
        self.ensure_open()?;
        *self.geometry.borrow_mut() = geometry;
        for state in &mut self.mix.seats {
            Self::bind_geometry(&self.geometry, state);
        }
        Ok(())
    }

    fn bind_geometry(geometry: &Rc<RefCell<Option<GeometryTransmission>>>, state: &mut SeatAudio) {
        if geometry.borrow().is_none() {
            state.mixer.set_geometry_transmission(None);
            return;
        }
        let shared = Rc::clone(geometry);
        let listener = Rc::clone(&state.listener);
        state.mixer.set_geometry_transmission(Some(Box::new(move |position| {
            let listener = listener.borrow();
            shared.borrow_mut().as_mut().map_or(1.0, |run| run(&listener, position))
        })));
    }

    /// Map an actor (and optional Q3 owner) to a 1-based entity number.
    fn entity(&mut self, actor: &ActorId, owner: Option<&ProviderId>) -> Result<i64, AudioError> {
        if let Some(index) = self.actors.iter().position(|entry| entry.actor == *actor && entry.owner.as_ref() == owner) {
            return Ok(index as i64 + 1);
        }
        let original = if owner.is_none() {
            None
        } else {
            Some(self.entity(actor, None)?)
        };
        if self.actors.len() + 1 >= self.max_actors {
            return Err(AudioError::ActorCapacity);
        }
        self.actors.push(ActorEntry {
            actor: actor.clone(),
            owner: owner.cloned(),
        });
        let entity = self.actors.len() as i64;
        match original {
            None => {
                self.actor_aliases.insert(entity, vec![entity]);
            }
            Some(original) => {
                self.actor_aliases.entry(original).or_default().push(entity);
                if let Some(position) = self.positions.get(&original).copied() {
                    self.positions.insert(entity, position);
                    for state in &mut self.mix.seats {
                        state.mixer.update_entity_position(entity, position)?;
                    }
                }
            }
        }
        Ok(entity)
    }

    fn seat_index(&self, id: &SeatId) -> Result<usize, AudioError> {
        self.mix
            .seats
            .iter()
            .position(|state| state.listener.borrow().seat == *id)
            .ok_or(AudioError::UnknownSeat)
    }

    /// Replace the listener set, retaining round mixers by seat.
    pub fn set_listeners(&mut self, listeners: &[AudioListener]) -> Result<(), AudioError> {
        self.ensure_open()?;
        for (index, listener) in listeners.iter().enumerate() {
            if listeners[..index].iter().any(|prior| prior.seat == listener.seat) {
                return Err(AudioError::DuplicateSeat);
            }
            if !listener.gain.is_finite() || listener.gain < 0.0 {
                return Err(AudioError::BadListenerGain);
            }
        }
        let mut removed = self.mix.seats.len();
        while removed > 0 {
            removed -= 1;
            let seat = self.mix.seats[removed].listener.borrow().seat.clone();
            if !listeners.iter().any(|listener| listener.seat == seat) {
                self.mix.seats[removed].mixer.stop_all()?;
                self.mix.seats.remove(removed);
            }
        }
        for listener in listeners {
            let milliseconds = (self.milliseconds)();
            let index = match self.mix.seats.iter().position(|state| state.listener.borrow().seat == listener.seat) {
                Some(index) => index,
                None => {
                    let retained = self.round_mixers.iter().position(|entry| entry.seat == listener.seat);
                    let mixer = match retained {
                        None => {
                            let clock = Rc::clone(&self.milliseconds);
                            AudioMixer::new(self.sample_rate, Box::new(move || clock()), SEAT_MIXER_CAPACITY, 2, self.max_actors)?
                        }
                        Some(retained) => self.round_mixers.remove(retained).mixer,
                    };
                    let mut state = SeatAudio {
                        listener: Rc::new(RefCell::new(listener.clone())),
                        mixer,
                        reverb: StereoReverb::new(self.sample_rate)?,
                        underwater: UnderwaterFilter::new(self.sample_rate),
                        environment: None,
                        loops: HashMap::new(),
                    };
                    let effects = self.effects_gain;
                    let doppler = self.doppler_enabled;
                    let frame = self.mix.frame;
                    state.mixer.set_effects_volume(effects)?;
                    state.mixer.set_doppler_enabled(doppler);
                    state.mixer.select_time(frame, frame)?;
                    let positions: Vec<(i64, Vec3)> = self.positions.iter().map(|(entity, position)| (*entity, *position)).collect();
                    for (entity, position) in positions {
                        state.mixer.update_entity_position(entity, position)?;
                    }
                    let observers = Rc::clone(&self.voice_observers);
                    let seat = listener.seat.clone();
                    state.mixer.set_voice_observer(
                        Some(Box::new(move |event: MixerVoiceEvent| {
                            let event = with_seat(event, &seat);
                            for entry in observers.borrow_mut().iter_mut() {
                                (entry.callback)(event.clone());
                            }
                        })),
                        Some({
                            let counter = Rc::clone(&self.voice_id_source);
                            Box::new(move || {
                                let mut next = counter.borrow_mut();
                                *next += 1;
                                *next
                            })
                        }),
                    );
                    Self::bind_geometry(&self.geometry, &mut state);
                    self.mix.seats.push(state);
                    self.mix.seats.len() - 1
                }
            };
            *self.mix.seats[index].listener.borrow_mut() = listener.clone();
            let entity = match listener.actor.as_ref() {
                None => 0,
                Some(actor) => self.entity(actor, None)?,
            };
            let state = &mut self.mix.seats[index];
            state.mixer.set_listener(i32::try_from(entity).map_err(|_| AudioError::BadListener)?, listener.origin, listener.axis)?;
            if let Some(environment) = state.environment.as_mut() {
                environment.update(listener.origin, milliseconds as f64)?;
            }
        }
        self.round_mixers.clear();
        Ok(())
    }

    /// Set the effects gain on every seat mixer.
    pub fn set_effects_volume(&mut self, gain: f64) -> Result<(), AudioError> {
        if !gain.is_finite() || gain < 0.0 {
            return Err(AudioError::BadEffectsGain);
        }
        self.effects_gain = gain;
        for state in &mut self.mix.seats {
            state.mixer.set_effects_volume(gain)?;
        }
        Ok(())
    }

    /// Enable or disable Doppler on every seat mixer.
    pub fn set_doppler_enabled(&mut self, enabled: bool) -> Result<(), AudioError> {
        self.ensure_open()?;
        self.doppler_enabled = enabled;
        for state in &mut self.mix.seats {
            state.mixer.set_doppler_enabled(enabled);
        }
        Ok(())
    }

    /// Move an actor and all of its Q3 owner aliases.
    pub fn update_actor(&mut self, actor: &ActorId, origin: Vec3) -> Result<(), AudioError> {
        self.ensure_open()?;
        let entity = self.entity(actor, None)?;
        let aliases = self.actor_aliases.get(&entity).cloned().unwrap_or_default();
        for number in aliases {
            self.positions.insert(number, origin);
            for state in &mut self.mix.seats {
                state.mixer.update_entity_position(number, origin)?;
            }
        }
        Ok(())
    }

    /// Move a Q3 seat actor owned by a guest provider.
    pub fn update_q3_seat_actor(&mut self, seat: &SeatId, actor: &ActorId, origin: Vec3, owner: Option<&ProviderId>) -> Result<(), AudioError> {
        self.ensure_open()?;
        let index = self.seat_index(seat)?;
        let entity = self.entity(actor, owner)?;
        self.mix.seats[index].mixer.update_entity_position(entity, origin)
    }

    fn request_origin(&mut self, family: SoundFamily, origin: &SoundOrigin, owner: Option<&ProviderId>) -> Result<MixerVoiceOrigin, AudioError> {
        match origin {
            SoundOrigin::Local => Ok(MixerVoiceOrigin::Local),
            SoundOrigin::Fixed { position } => Ok(MixerVoiceOrigin::Fixed { position: *position }),
            SoundOrigin::Actor { actor } => {
                let scoped = if family == SoundFamily::Q3 { owner } else { None };
                Ok(MixerVoiceOrigin::Entity {
                    entity: self.entity(actor, scoped)?,
                })
            }
        }
    }

    /// Play a one-shot on every selected seat; returns the seat count.
    pub fn play(&mut self, request: &PlaySound) -> Result<usize, AudioError> {
        self.ensure_open()?;
        if !request.volume.is_finite() || request.volume < 0.0 || request.volume > 1.0 {
            return Err(AudioError::BadVolume);
        }
        let command = source_sound_channel(request.family, request.channel).map_err(|_| AudioError::BadSourceChannel)?;
        let owner = if request.family == SoundFamily::Q3 { request.owner.as_ref() } else { None };
        let origin = self.request_origin(request.family, &request.origin, owner)?;
        let entity = match request.actor.as_ref() {
            None => -1,
            Some(actor) => self.entity(actor, owner)?,
        };
        let listeners: Vec<AudioListener> = self.mix.seats.iter().map(|state| state.listener.borrow().clone()).collect();
        let mut planned: Vec<(bool, i64, bool)> = Vec::with_capacity(listeners.len());
        for listener in &listeners {
            if !selected(&request.audience, &listener.seat) {
                planned.push((false, 0, false));
                continue;
            }
            let local = match listener.actor.as_ref() {
                None => 0,
                Some(actor) => self.entity(actor, owner)?,
            };
            let personal = owner.is_some() && request.actor.is_some() && listener.actor.is_some() && request.actor == listener.actor;
            planned.push((true, local, personal));
        }
        let mut playing = 0usize;
        let random = &mut *self.random;
        for (state, (select, local, personal)) in self.mix.seats.iter_mut().zip(planned.iter()) {
            if !select {
                continue;
            }
            let voice_origin = if *personal { MixerVoiceOrigin::Local } else { origin.clone() };
            let voice_entity = if matches!(origin, MixerVoiceOrigin::Local) { *local } else { entity };
            let accepted = match request.family {
                SoundFamily::Q3 => state.mixer.start_shared_sound(
                    &request.sound.pcm,
                    voice_entity,
                    voice_origin,
                    (request.volume * 127.0).trunc() as i32,
                    &command,
                    Some(&request.sound.name),
                    Some(request.sound.clone()),
                )?,
                SoundFamily::Q1 => state.mixer.start_q1_sound(
                    &request.sound.pcm,
                    &SourceSoundOptions {
                        entity: voice_entity,
                        origin: voice_origin,
                        volume: request.volume,
                        attenuation: request.attenuation,
                    },
                    &command,
                    Some(&mut *random),
                    Some(request.sound.clone()),
                )?,
                SoundFamily::Q2 => state.mixer.start_q2_sound(
                    &request.sound.pcm,
                    &Q2SoundOptions {
                        entity: voice_entity,
                        origin: voice_origin,
                        volume: request.volume,
                        attenuation: request.attenuation,
                        delay_seconds: request.delay_seconds,
                        server_milliseconds: request.server_milliseconds,
                    },
                    &command,
                    Some(request.sound.clone()),
                )?,
            };
            if accepted {
                playing += 1;
            }
        }
        if playing > 0 {
            if let Some(on_sound) = self.on_sound.as_mut() {
                on_sound(request);
            }
        }
        Ok(playing)
    }

    /// Stop an actor's channel on an audience.
    pub fn stop_sound(&mut self, actor: &ActorId, channel: Option<SoundChannel>, audience: &AudioAudience) -> Result<(), AudioError> {
        let entity = self.entity(actor, None)?;
        for state in &mut self.mix.seats {
            if selected(audience, &state.listener.borrow().seat) {
                state.mixer.stop_shared_channel(entity, channel);
            }
        }
        Ok(())
    }

    /// Add a loop (`loop` in the donor) on every selected seat.
    pub fn start_loop(&mut self, request: &LoopSound) -> Result<(), AudioError> {
        self.ensure_open()?;
        let entity = self.entity(&request.actor, None)?;
        let q3_entity = if request.family == SoundFamily::Q3 {
            Some(self.entity(&request.actor, request.owner.as_ref())?)
        } else {
            None
        };
        for state in &mut self.mix.seats {
            if !selected(&request.audience, &state.listener.borrow().seat) {
                continue;
            }
            let origin = match &request.origin {
                SoundOrigin::Local => state.listener.borrow().origin,
                SoundOrigin::Fixed { position } => *position,
                SoundOrigin::Actor { .. } => self.positions.get(&entity).copied().ok_or(AudioError::LoopPosition)?,
            };
            state.loops.insert(loop_key(request.family, entity, request.owner.as_ref()), request.clone());
            if request.family == SoundFamily::Q3 {
                let owned = q3_entity.expect("q3 entity");
                let volume = (request.volume * (if request.lifetime == LoopLifetime::Frame { 127.0 } else { 90.0 })).trunc() as i32;
                if request.lifetime == LoopLifetime::Frame {
                    state.mixer.update_looping_sound(
                        &request.sound.pcm,
                        &FrameLoopingSoundOptions {
                            entity: owned,
                            origin,
                            velocity: request.velocity,
                            frame_number: request.frame_number,
                            volume: Some(volume),
                        },
                    )?;
                } else {
                    state.mixer.update_real_looping_sound(
                        &request.sound.pcm,
                        &RealLoopingSoundOptions {
                            entity: owned,
                            origin,
                            velocity: request.velocity,
                            volume: Some(volume),
                        },
                    )?;
                }
            }
        }
        Ok(())
    }

    /// Clear frame loops before a loop frame.
    pub fn begin_loop_frame(&mut self) {
        for state in &mut self.mix.seats {
            state.mixer.clear_looping_sounds(false);
            state.loops.retain(|_, existing| existing.lifetime != LoopLifetime::Frame);
        }
    }

    /// Push accumulated loops into every seat mixer.
    pub fn end_loop_frame(&mut self) -> Result<(), AudioError> {
        let seats = self.mix.seats.len();
        for index in 0..seats {
            let listener = self.mix.seats[index].listener.borrow().clone();
            let loops: Vec<LoopSound> = self.mix.seats[index].loops.values().cloned().collect();
            let mut entries = Vec::new();
            for existing in &loops {
                let entity = self.entity(&existing.actor, None)?;
                let origin = match &existing.origin {
                    SoundOrigin::Local => listener.origin,
                    SoundOrigin::Fixed { position } => *position,
                    SoundOrigin::Actor { .. } => self.positions.get(&entity).copied().ok_or(AudioError::LoopPosition)?,
                };
                if existing.family == SoundFamily::Q1 || existing.family == SoundFamily::Q2 {
                    entries.push(super::mixer::SourceLoopEntry {
                        family: existing.family,
                        entity,
                        sound: existing.sound.pcm.clone(),
                        origin,
                        volume: existing.volume,
                        attenuation: Some(existing.attenuation),
                    });
                }
            }
            let state = &mut self.mix.seats[index];
            state.mixer.set_source_loop_sounds(&entries)?;
            let entity = match listener.actor.as_ref() {
                None => 0,
                Some(actor) => self.entity(actor, None)?,
            };
            self.mix.seats[index]
                .mixer
                .set_listener(i32::try_from(entity).map_err(|_| AudioError::BadListener)?, listener.origin, listener.axis)?;
        }
        Ok(())
    }

    /// Stop an actor's loops on an audience.
    pub fn stop_loop(&mut self, actor: &ActorId, audience: &AudioAudience, owner: Option<&ProviderId>) -> Result<(), AudioError> {
        let entity = self.entity(actor, None)?;
        let owned = self.entity(actor, owner)?;
        for state in &mut self.mix.seats {
            if !selected(audience, &state.listener.borrow().seat) {
                continue;
            }
            let mut removed_q3 = false;
            state.loops.retain(|_, existing| {
                let hit = existing.actor == *actor && existing.owner.as_ref() == owner;
                if hit && existing.family == SoundFamily::Q3 {
                    removed_q3 = true;
                }
                !hit
            });
            if removed_q3 {
                state.mixer.stop_looping_sound(owned)?;
            }
            if owner.is_none() {
                state.mixer.stop_looping_sound(entity)?;
            }
        }
        self.end_loop_frame()
    }

    /// Clear a seat's Q3 loops for an owner.
    pub fn clear_q3_seat_loops(&mut self, seat: &SeatId, kill_all: bool, owner: Option<&ProviderId>) -> Result<(), AudioError> {
        let index = self.seat_index(seat)?;
        let mut removed: Vec<(String, ActorId)> = Vec::new();
        for (key, existing) in &self.mix.seats[index].loops {
            if existing.family == SoundFamily::Q3
                && existing.owner.as_ref() == owner
                && matches!(&existing.audience, AudioAudience::Seat { seat: target } if target == seat)
                && (kill_all || existing.lifetime == LoopLifetime::Frame)
            {
                removed.push((key.clone(), existing.actor.clone()));
            }
        }
        let mut entities = Vec::with_capacity(removed.len());
        for (_, actor) in &removed {
            entities.push(self.entity(actor, owner)?);
        }
        let state = &mut self.mix.seats[index];
        for ((key, _), entity) in removed.into_iter().zip(entities) {
            state.loops.remove(&key);
            state.mixer.stop_looping_sound(entity)?;
        }
        Ok(())
    }

    /// Stop one Q3 seat loop.
    pub fn stop_q3_seat_loop(&mut self, seat: &SeatId, actor: &ActorId, owner: Option<&ProviderId>) -> Result<(), AudioError> {
        let index = self.seat_index(seat)?;
        let owned = self.entity(actor, owner)?;
        let base = self.entity(actor, None)?;
        let key = loop_key(SoundFamily::Q3, base, owner);
        let remove = match self.mix.seats[index].loops.get(&key) {
            Some(existing) if matches!(&existing.audience, AudioAudience::World) => return Ok(()),
            Some(existing) => matches!(&existing.audience, AudioAudience::Seat { seat: target } if target == seat),
            None => false,
        };
        let state = &mut self.mix.seats[index];
        if remove {
            state.loops.remove(&key);
        }
        state.mixer.stop_looping_sound(owned)
    }

    /// Release every Q3 seat voice owned by a guest provider.
    pub fn release_q3_seat_owner(&mut self, seat: &SeatId, owner: &ProviderId) -> Result<(), AudioError> {
        self.clear_q3_seat_loops(seat, true, Some(owner))?;
        let index = self.seat_index(seat)?;
        let mut entities = Vec::new();
        for (slot, entry) in self.actors.iter().enumerate() {
            if entry.owner.as_ref() == Some(owner) {
                entities.push(slot as i64 + 1);
            }
        }
        for entity in entities {
            self.mix.seats[index].mixer.stop_entity(entity)?;
        }
        Ok(())
    }

    /// Add a static loop to a seat mixer.
    pub fn add_static_sound(&mut self, seat: &SeatId, sound: &SoundAsset, origin: Vec3, volume: f64, attenuation: f64, key: i64) -> Result<bool, AudioError> {
        let index = self.seat_index(seat)?;
        self.mix.seats[index].mixer.add_static_sound(&sound.pcm, origin, volume, attenuation, key)
    }

    /// Remove a static loop from a seat mixer.
    pub fn remove_static_sound(&mut self, seat: &SeatId, key: i64) -> Result<(), AudioError> {
        let index = self.seat_index(seat)?;
        self.mix.seats[index].mixer.remove_static_sound(key);
        Ok(())
    }

    /// Update a seat's ambient bed.
    pub fn update_ambient(&mut self, seat: &SeatId, sounds: &[SoundAsset], levels: &[f64], elapsed_seconds: f64, level: f64, fade: f64) -> Result<(), AudioError> {
        let index = self.seat_index(seat)?;
        let pcm: Vec<_> = sounds.iter().map(|sound| sound.pcm.clone()).collect();
        self.mix.seats[index].mixer.update_ambient(&pcm, levels, elapsed_seconds, level, fade)
    }

    /// Bind a reverb environment selector to a seat.
    pub fn set_environment(&mut self, seat: &SeatId, environments: Vec<ReverbEnvironment>, trace: AudioTraceQuery) -> Result<(), AudioError> {
        let index = self.seat_index(seat)?;
        let milliseconds = (self.milliseconds)();
        let origin = self.mix.seats[index].listener.borrow().origin;
        let state = &mut self.mix.seats[index];
        state.environment = Some(EnvironmentReverb::new(environments, trace));
        state.reverb.reset();
        if let Some(environment) = state.environment.as_mut() {
            environment.update(origin, milliseconds as f64)?;
        }
        Ok(())
    }

    /// Queue streamed PCM on a lane.
    pub fn queue_stream(&mut self, target: AudioStreamTarget, chunk: &StreamPcm) -> Result<(), AudioError> {
        self.ensure_open()?;
        let sample_rate = self.sample_rate;
        let bus = self.mix.streams.entry(target.id.clone()).or_insert_with(|| StreamBus {
            target: target.clone(),
            stream: RawAudioStream::new(sample_rate),
        });
        bus.target = target;
        bus.stream.queue(chunk)
    }

    /// Capture a stream checkpoint, if the lane exists.
    pub fn capture_stream_checkpoint(&mut self, id: &str) -> Result<Option<RawCheckpoint>, AudioError> {
        self.ensure_open()?;
        Ok(self.mix.streams.get(id).map(|bus| bus.stream.capture_checkpoint()))
    }

    /// Restore a stream checkpoint onto an unused lane.
    pub fn restore_stream_checkpoint(&mut self, target: AudioStreamTarget, value: Option<RawCheckpoint>) -> Result<(), AudioError> {
        self.ensure_open()?;
        if self.mix.streams.contains_key(&target.id) {
            return Err(AudioError::StreamLaneUsed);
        }
        if let Some(checkpoint) = value {
            let stream = RawAudioStream::restore_checkpoint(&checkpoint, self.sample_rate)?;
            self.mix.streams.insert(target.id.clone(), StreamBus { target, stream });
        }
        Ok(())
    }

    /// Pause or resume a stream lane.
    pub fn pause_stream(&mut self, id: &str, paused: bool) {
        if let Some(bus) = self.mix.streams.get_mut(id) {
            bus.stream.paused = paused;
        }
    }

    /// Drop a stream lane.
    pub fn stop_stream(&mut self, id: &str) {
        self.mix.streams.remove(id);
    }

    /// Attach a music player to a lane, closing any prior player.
    pub fn attach_music(&mut self, target: AudioStreamTarget, player: MusicPlayer) -> Result<(), AudioError> {
        if player.output_rate() != self.sample_rate {
            return Err(AudioError::MusicRateMismatch);
        }
        if let Some(mut prior) = self.mix.music.insert(target.id.clone(), MusicBus { target, player }) {
            prior.player.close();
        }
        Ok(())
    }

    /// Stop and close a music lane.
    pub fn stop_music(&mut self, id: &str) {
        if let Some(mut bus) = self.mix.music.remove(id) {
            bus.player.close();
        }
    }

    /// Advance every music player once per presentation frame.
    pub fn update_music(&mut self) {
        for bus in self.mix.music.values_mut() {
            bus.player.update();
        }
    }

    /// Mix frames of device PCM, advancing the mix clock.
    pub fn mix(&mut self, frames: usize) -> Result<Vec<i16>, AudioError> {
        self.ensure_open()?;
        if self.paused {
            return Ok(vec![0i16; frames * 2]);
        }
        let sample_rate = self.sample_rate;
        self.mix.mix(frames, sample_rate)
    }

    /// Open the output device.
    pub fn open_device(&mut self, device_name: Option<&str>, buffer_frames: Option<u32>) -> Result<(), AudioError> {
        self.ensure_open()?;
        if self.device.is_some() {
            return Err(AudioError::AlreadyOpen);
        }
        let format = self.format;
        let device = self.factory.open(device_name, &format, buffer_frames).map_err(|error| error.audio_error())?;
        self.device = Some(device);
        self.detached_output = None;
        Ok(())
    }

    /// Validate an output handoff to a replacement engine, returning the commit.
    ///
    /// The donor returns a thunk so the caller can swap engine references
    /// first; the commit closure borrows both engines and performs the move.
    pub fn prepare_output_transfer<'a>(&'a mut self, next: &'a mut UnifiedAudio) -> Result<Box<dyn FnOnce() + 'a>, AudioError> {
        self.ensure_open()?;
        next.ensure_open()?;
        if next.device.is_some() || next.detached_output.is_some() {
            return Err(AudioError::ReplacementOwnsOutput);
        }
        if next.sample_rate != self.sample_rate {
            return Err(AudioError::ReplacementRateMismatch);
        }
        Ok(Box::new(move || {
            next.device = self.device.take();
            next.detached_output = self.detached_output.take();
            next.queued_pcm = std::mem::take(&mut self.queued_pcm);
            next.format = self.format;
            let fresh_rate = next.format.sample_rate;
            next.output_stream = std::mem::replace(&mut self.output_stream, RawAudioStream::new(fresh_rate));
            next.output_source_frame = self.output_source_frame;
            next.paused = self.paused;
            next.output_started = self.output_started;
            next.previous_pump_frame = None;
            next.pump_intervals.clear();
            next.output_handoff_pending = true;
            self.previous_pump_frame = None;
            self.pump_intervals.clear();
        }))
    }

    /// Detach the output device, retaining its queue.
    pub fn detach_output(&mut self) -> Result<(), AudioError> {
        self.ensure_open()?;
        if self.device.is_none() {
            return Ok(());
        }
        let device = self.device.as_mut().expect("checked");
        let detached = DetachedOutput {
            playing: device.playing(),
            buffer_frames: device.buffer_frames(),
            device_name: device.device_name(),
        };
        device.pause()?;
        let queued = device.queued_frames()? * 2;
        let start = self.queued_pcm.len().saturating_sub(usize::try_from(queued).unwrap_or(usize::MAX));
        self.queued_pcm = self.queued_pcm[start..].to_vec();
        self.detached_output = Some(detached);
        if let Some(device) = self.device.take() {
            device.close_box();
        }
        self.previous_pump_frame = None;
        self.pump_intervals.clear();
        Ok(())
    }

    fn open_output(
        factory: &Rc<dyn AudioDeviceFactory>,
        device_name: Option<&str>,
        format: &AudioOutputFormat,
        pcm: &[i16],
        buffer_frames: Option<u32>,
    ) -> Result<Box<dyn AudioOutputDevice>, OpenOutputError> {
        let mut device = factory.open(device_name, format, buffer_frames).map_err(|error| OpenOutputError {
            unavailable: matches!(error, DeviceOpenError::Unavailable(_)),
            error: error.audio_error(),
        })?;
        if let Err(error) = device.queue(pcm, format) {
            device.close_box();
            return Err(OpenOutputError {
                unavailable: false,
                error,
            });
        }
        Ok(device)
    }

    /// Select an output device and format, recovering the old one on failure.
    pub fn select_output(&mut self, device_name: Option<&str>, requested: &AudioOutputFormat, restart: bool) -> Result<(), AudioError> {
        self.ensure_open()?;
        let format = audio_output_format(i64::from(requested.sample_rate), i64::from(requested.channels), i64::from(requested.sample_bits))?;
        let old_format = self.format;
        if !restart {
            if let Some(previous) = self.device.as_ref() {
                if previous.device_name().as_deref() == device_name && format == old_format {
                    return Ok(());
                }
            }
        }
        let detached = self.detached_output.clone();
        let playing = match self.device.as_mut() {
            None => detached.as_ref().is_some_and(|detached| detached.playing),
            Some(previous) => previous.playing(),
        };
        if let Some(previous) = self.device.as_mut() {
            previous.pause()?;
        }
        self.queued_pcm = self.pending_output()?;
        let retained = self.queued_pcm.clone();
        let converted = resample_queued_pcm(&retained, old_format.sample_rate, format.sample_rate)?;
        let old_buffer = match self.device.as_ref() {
            Some(previous) => Some(previous.buffer_frames()),
            None => detached.as_ref().map(|detached| detached.buffer_frames),
        };
        let old_buffer = old_buffer.map(|frames| frames.clamp(1, 32768) as u32);
        let paused = self.paused;
        let replacement = match Self::open_output(&self.factory.clone(), device_name, &format, &converted, old_buffer) {
            Ok(device) => device,
            Err(error) => {
                if !error.unavailable || self.device.is_none() {
                    if playing {
                        if let Some(previous) = self.device.as_mut() {
                            previous.resume()?;
                        }
                    }
                    return Err(error.error);
                }
                let previous = self.device.take().expect("checked");
                let previous_name = previous.device_name();
                let previous_buffer = previous.buffer_frames();
                previous.close_box();
                match Self::open_output(&self.factory.clone(), device_name, &format, &converted, old_buffer) {
                    Ok(device) => device,
                    Err(selection) => {
                        let restore = Self::open_output(&self.factory.clone(), previous_name.as_deref(), &old_format, &retained, Some(previous_buffer.clamp(1, 32768) as u32));
                        match restore {
                            Ok(restored) => {
                                self.device = Some(restored);
                                if playing && !paused {
                                    if let Some(device) = self.device.as_mut() {
                                        device.resume()?;
                                    }
                                }
                            }
                            Err(restore_error) => {
                                if let Some(device) = self.device.take() {
                                    device.close_box();
                                }
                                self.detached_output = Some(DetachedOutput {
                                    playing,
                                    buffer_frames: previous_buffer,
                                    device_name: previous_name,
                                });
                                self.previous_pump_frame = None;
                                self.pump_intervals.clear();
                                return Err(AudioError::OutputSelectionFailed(format!("{}; {}", selection.error, restore_error.error)));
                            }
                        }
                        self.previous_pump_frame = None;
                        self.pump_intervals.clear();
                        return Err(selection.error);
                    }
                }
            }
        };
        if let Some(previous) = self.device.take() {
            previous.close_box();
        }
        self.device = Some(replacement);
        self.detached_output = None;
        self.format = format;
        self.queued_pcm = converted;
        if format.sample_rate != old_format.sample_rate {
            self.output_stream = self.output_stream.with_output_rate(format.sample_rate);
        }
        self.previous_pump_frame = None;
        self.pump_intervals.clear();
        if playing && !paused {
            if let Some(device) = self.device.as_mut() {
                device.resume()?;
            }
        }
        Ok(())
    }

    fn mix_output(&mut self, frames: u64) -> Result<Vec<i16>, AudioError> {
        let frames_usize = usize::try_from(frames).map_err(|_| AudioError::MixTooLarge)?;
        if self.format.sample_rate == self.sample_rate && !self.output_stream.initialized() {
            self.output_source_frame += frames as i64;
            let sample_rate = self.sample_rate;
            return self.mix.mix(frames_usize, sample_rate);
        }
        let sample_rate = self.sample_rate;
        let format_rate = self.format.sample_rate;
        let stream = &mut self.output_stream;
        let source_frame = &mut self.output_source_frame;
        let mix = &mut self.mix;
        let mixed = stream.mix(
            frames_usize,
            1.0,
            Some(&mut || {
                let source_sample = *source_frame;
                let source_frames = ((frames as f64 * f64::from(sample_rate) / f64::from(format_rate)).ceil() as usize).max(1);
                let samples = mix.mix(source_frames, sample_rate)?;
                *source_frame += source_frames as i64;
                Ok(Some(StreamPcm {
                    samples: StreamSamples::S16(samples),
                    sample_rate,
                    channels: 2,
                    source_sample,
                    reset_stream: false,
                }))
            }),
        )?;
        Ok(mixed.iter().map(|value| to_int16(*value)).collect())
    }

    /// Queue device audio ahead of playback; returns the frames queued.
    pub fn pump(&mut self, ahead_frames: Option<u64>, measured_work_milliseconds: f64) -> Result<u64, AudioError> {
        self.ensure_open()?;
        if self.device.is_none() {
            return Err(AudioError::NotOpen);
        }
        if self.paused {
            return Ok(0);
        }
        if !measured_work_milliseconds.is_finite() || measured_work_milliseconds < 0.0 {
            return Err(AudioError::BadMeasuredWork);
        }
        let device = self.device.as_ref().expect("checked");
        let device_rate = device.sample_rate();
        let max_queued = device.max_queued_frames();
        let buffer_frames = device.buffer_frames();
        let state_paused = !device.playing();
        let queued_now = device.queued_frames()?;
        let work_frames = (measured_work_milliseconds * f64::from(device_rate) / 1000.0).ceil() as i64;
        let initial_fill = self.output_handoff_pending || !self.output_started && state_paused && queued_now == 0;
        let playback_frame = self.device.as_ref().expect("checked").playback_frames()?;
        let interval = self.previous_pump_frame.map_or(0, |previous| playback_frame as i64 - previous as i64);
        if !initial_fill {
            self.pump_intervals.push(interval.max(work_frames));
            if self.pump_intervals.len() > 8 {
                self.pump_intervals.remove(0);
            }
        }
        let target = ahead_frames.unwrap_or_else(|| {
            max_queued.min(if initial_fill {
                ((f64::from(device_rate) * 0.2).ceil() as u64).max(buffer_frames * 2)
            } else {
                ((f64::from(device_rate) * 0.08).ceil() as u64).max(self.pump_intervals.iter().copied().max().unwrap_or(0).max(0) as u64 + buffer_frames * 2)
            })
        });
        if target > max_queued {
            return Err(AudioError::BadLookahead);
        }
        self.previous_pump_frame = Some(playback_frame);
        let queued = self.device.as_ref().expect("checked").queued_frames()?;
        let keep = usize::try_from(queued * 2).unwrap_or(usize::MAX);
        let start = self.queued_pcm.len().saturating_sub(keep.min(self.queued_pcm.len()));
        self.queued_pcm.drain(..start);
        let frames = target.saturating_sub(queued);
        if frames > 0 {
            let samples = self.mix_output(frames)?;
            self.queued_pcm.extend_from_slice(&samples);
            let format = self.format;
            self.device.as_mut().expect("checked").queue(&samples, &format)?;
            let queued = self.device.as_ref().expect("checked").queued_frames()?;
            let keep = usize::try_from(queued * 2).unwrap_or(usize::MAX);
            let start = self.queued_pcm.len().saturating_sub(keep.min(self.queued_pcm.len()));
            self.queued_pcm.drain(..start);
        }
        self.device.as_mut().expect("checked").resume()?;
        self.output_started = true;
        self.output_handoff_pending = false;
        Ok(frames)
    }

    /// Pause or resume the engine.
    pub fn pause(&mut self, paused: bool) -> Result<(), AudioError> {
        self.ensure_open()?;
        self.paused = paused;
        self.previous_pump_frame = None;
        self.pump_intervals.clear();
        if paused {
            if let Some(device) = self.device.as_mut() {
                device.pause()?;
            }
        } else if let Some(device) = self.device.as_mut() {
            device.resume()?;
            self.output_started = true;
        }
        Ok(())
    }

    /// Stop every voice, loop, stream, and queued device frame.
    pub fn stop_all(&mut self) -> Result<(), AudioError> {
        for state in &mut self.mix.seats {
            state.mixer.stop_all()?;
            state.loops.clear();
            state.reverb.reset();
            state.underwater.reset();
        }
        self.mix.streams.clear();
        for bus in self.mix.music.values_mut() {
            bus.player.close();
        }
        self.mix.music.clear();
        if let Some(device) = self.device.as_mut() {
            device.clear()?;
        }
        self.queued_pcm.clear();
        self.output_stream = RawAudioStream::new(self.format.sample_rate);
        self.output_source_frame = 0;
        Ok(())
    }

    /// Stop everything and retain seat mixers for the next round.
    pub fn reset_round(&mut self) -> Result<(), AudioError> {
        self.ensure_open()?;
        self.stop_all()?;
        for state in self.mix.seats.drain(..) {
            let seat = state.listener.borrow().seat.clone();
            let entry = RoundMixer { seat, mixer: state.mixer };
            match self.round_mixers.iter_mut().find(|retained| retained.seat == entry.seat) {
                Some(slot) => *slot = entry,
                None => self.round_mixers.push(entry),
            }
        }
        self.actors.clear();
        self.actor_aliases.clear();
        self.positions.clear();
        self.previous_pump_frame = None;
        self.pump_intervals.clear();
        Ok(())
    }

    /// Close the engine, releasing the device.
    pub fn close(&mut self) -> Result<(), AudioError> {
        if self.closed {
            return Ok(());
        }
        let result = self.stop_all();
        self.closed = true;
        self.geometry.borrow_mut().take();
        self.voice_observers.borrow_mut().clear();
        for state in &mut self.mix.seats {
            state.mixer.set_geometry_transmission(None);
        }
        for retained in &mut self.round_mixers {
            retained.mixer.set_geometry_transmission(None);
        }
        self.round_mixers.clear();
        if let Some(device) = self.device.take() {
            device.close_box();
        }
        self.detached_output = None;
        result
    }
}

impl Drop for UnifiedAudio {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

struct OpenOutputError {
    unavailable: bool,
    error: AudioError,
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::collections::VecDeque;

    use qa_core::identity::IdentityOwner;
    use qa_core::math::{vec3, Axis};

    use super::super::music::{MusicControls, MusicVolumeMode};
    use super::super::wav::PcmSound;
    use super::*;

    fn axis() -> Axis {
        [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)]
    }

    fn blip(frames: usize, loop_start: Option<usize>) -> std::rc::Rc<PcmSound> {
        std::rc::Rc::new(PcmSound {
            sample_rate: 44100,
            channels: 1,
            samples: vec![1000i16; frames],
            frame_count: frames,
            loop_start,
        })
    }

    fn asset(pcm: std::rc::Rc<PcmSound>) -> SoundAsset {
        SoundAsset {
            resource: "test".to_string(),
            name: "blip".to_string(),
            pcm,
        }
    }

    struct FakeDevice {
        name: Option<String>,
        format: AudioOutputFormat,
        buffer_frames: u64,
        queued: u64,
        playback: u64,
        playing: bool,
        queue_fail: bool,
    }

    impl AudioOutputDevice for FakeDevice {
        fn device_name(&self) -> Option<String> {
            self.name.clone()
        }
        fn sample_rate(&self) -> u32 {
            self.format.sample_rate
        }
        fn channels(&self) -> u8 {
            self.format.channels
        }
        fn sample_bits(&self) -> u8 {
            self.format.sample_bits
        }
        fn buffer_frames(&self) -> u64 {
            self.buffer_frames
        }
        fn max_queued_frames(&self) -> u64 {
            u64::from(self.format.sample_rate) * 2
        }
        fn queued_frames(&self) -> Result<u64, AudioError> {
            Ok(self.queued)
        }
        fn playback_frames(&self) -> Result<u64, AudioError> {
            Ok(self.playback)
        }
        fn queue(&mut self, samples: &[i16], format: &AudioOutputFormat) -> Result<(), AudioError> {
            if self.queue_fail {
                return Err(AudioError::Device("fake queue failed".to_string()));
            }
            let encoded = encode_output_pcm(samples, format)?;
            let frames = match &encoded {
                EncodedPcm::S16(samples) => samples.len() as u64 / u64::from(format.channels),
                EncodedPcm::U8(samples) => samples.len() as u64 / u64::from(format.channels),
            };
            self.queued += frames;
            Ok(())
        }
        fn clear(&mut self) -> Result<(), AudioError> {
            self.queued = 0;
            Ok(())
        }
        fn pause(&mut self) -> Result<(), AudioError> {
            self.playing = false;
            Ok(())
        }
        fn resume(&mut self) -> Result<(), AudioError> {
            self.playing = true;
            Ok(())
        }
        fn playing(&self) -> bool {
            self.playing
        }
        fn close_box(self: Box<Self>) {}
    }

    #[derive(Clone)]
    struct FakeSpec {
        buffer_frames: u64,
        queue_fail: bool,
    }

    enum ScriptedOpen {
        Device(FakeSpec),
        Unavailable,
        Failed,
    }

    type OpenRecord = (Option<String>, AudioOutputFormat, Option<u32>);

    struct FakeFactory {
        script: RefCell<VecDeque<ScriptedOpen>>,
        names: Vec<String>,
        opens: RefCell<Vec<OpenRecord>>,
        spec: FakeSpec,
    }

    impl FakeFactory {
        fn named(names: &[&str]) -> Self {
            Self {
                script: RefCell::new(VecDeque::new()),
                names: names.iter().map(|name| name.to_string()).collect(),
                opens: RefCell::new(Vec::new()),
                spec: FakeSpec {
                    buffer_frames: 512,
                    queue_fail: false,
                },
            }
        }

        fn queue_fail() -> Self {
            let mut factory = Self::named(&["fake"]);
            factory.spec.queue_fail = true;
            factory
        }

        fn push(&self, open: ScriptedOpen) {
            self.script.borrow_mut().push_back(open);
        }
    }

    impl AudioDeviceFactory for FakeFactory {
        fn open(&self, device_name: Option<&str>, format: &AudioOutputFormat, buffer_frames: Option<u32>) -> Result<Box<dyn AudioOutputDevice>, DeviceOpenError> {
            self.opens.borrow_mut().push((device_name.map(str::to_string), *format, buffer_frames));
            let spec = match self.script.borrow_mut().pop_front() {
                Some(ScriptedOpen::Device(spec)) => spec,
                None => self.spec.clone(),
                Some(ScriptedOpen::Unavailable) => return Err(DeviceOpenError::Unavailable("fake unavailable".to_string())),
                Some(ScriptedOpen::Failed) => return Err(DeviceOpenError::Failed("fake failed".to_string())),
            };
            Ok(Box::new(FakeDevice {
                name: device_name.map(str::to_string),
                format: *format,
                buffer_frames: spec.buffer_frames,
                queued: 0,
                playback: 0,
                playing: false,
                queue_fail: spec.queue_fail,
            }))
        }

        fn output_names(&self) -> Result<Vec<String>, AudioError> {
            Ok(self.names.clone())
        }
    }

    fn engine(factory: FakeFactory) -> UnifiedAudio {
        engine_with_options(factory, None, None)
    }

    fn engine_with_options(factory: FakeFactory, sample_rate: Option<u32>, max_actors: Option<usize>) -> UnifiedAudio {
        UnifiedAudio::new(UnifiedAudioOptions {
            sample_rate,
            output_format: None,
            milliseconds: Box::new(|| 0),
            random: Box::new(|| 7),
            max_actors,
            on_sound: None,
            device_factory: Some(Rc::new(factory)),
        })
        .unwrap()
    }

    fn listener(owner: &IdentityOwner, seat: u32) -> AudioListener {
        AudioListener {
            seat: owner.seat(seat),
            actor: None,
            origin: vec3(0.0, 0.0, 0.0),
            axis: axis(),
            gain: 1.0,
            underwater: false,
        }
    }

    #[test]
    fn validates_rate_and_reports_state() {
        assert!(matches!(
            UnifiedAudio::new(UnifiedAudioOptions {
                sample_rate: Some(4000),
                output_format: None,
                milliseconds: Box::new(|| 0),
                random: Box::new(|| 0),
                max_actors: None,
                on_sound: None,
                device_factory: Some(Rc::new(FakeFactory::named(&[]))),
            }),
            Err(AudioError::BadOutputRate)
        ));
        let audio = engine(FakeFactory::named(&["a", "b"]));
        assert_eq!(audio.sample_rate(), 44100);
        assert_eq!(audio.output_state(), OutputState::Detached);
        assert_eq!(audio.output_device_names().unwrap(), vec!["a".to_string(), "b".to_string()]);
        assert!(audio.output_configuration().is_none());
        assert_eq!(audio.selected_output(), None);
    }

    #[test]
    fn manages_listeners() {
        let owner = IdentityOwner::create("test").unwrap();
        let mut audio = engine(FakeFactory::named(&[]));
        let first = listener(&owner, 0);
        let mut second = listener(&owner, 1);
        second.gain = f64::NAN;
        assert!(matches!(audio.set_listeners(&[first.clone(), second]), Err(AudioError::BadListenerGain)));
        assert!(matches!(audio.set_listeners(&[first.clone(), first.clone()]), Err(AudioError::DuplicateSeat)));
        audio.set_listeners(std::slice::from_ref(&first)).unwrap();
        let mut second = listener(&owner, 1);
        second.gain = 0.5;
        audio.set_listeners(&[first, second]).unwrap();
        audio.set_listeners(&[]).unwrap();
    }

    #[test]
    fn plays_and_mixes_families() {
        let owner = IdentityOwner::create("test").unwrap();
        let heard = Rc::new(Cell::new(0usize));
        let observed = Rc::new(RefCell::new(Vec::new()));
        let mut audio = UnifiedAudio::new(UnifiedAudioOptions {
            sample_rate: None,
            output_format: None,
            milliseconds: Box::new(|| 0),
            random: Box::new(|| 7),
            max_actors: None,
            on_sound: Some(Box::new({
                let heard = Rc::clone(&heard);
                move |_| heard.set(heard.get() + 1)
            })),
            device_factory: Some(Rc::new(FakeFactory::named(&[]))),
        })
        .unwrap();
        audio.set_listeners(&[listener(&owner, 0)]).unwrap();
        let seen = Rc::clone(&observed);
        let token = audio.observe_voices(Box::new(move |event| seen.borrow_mut().push(event))).unwrap();
        let sound = asset(blip(4410, None));
        for family in [SoundFamily::Q1, SoundFamily::Q3, SoundFamily::Q2] {
            let request = PlaySound {
                family,
                sound: sound.clone(),
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
            assert_eq!(audio.play(&request).unwrap(), 1);
        }
        assert_eq!(heard.get(), 3);
        let mixed = audio.mix(64).unwrap();
        assert_eq!(mixed.len(), 128);
        assert!(mixed.iter().any(|sample| *sample != 0));
        assert_eq!(audio.sample_clock(), 64);
        assert!(observed.borrow().iter().any(|event| matches!(event, AudioVoiceEvent::Start { .. })));
        audio.unobserve_voices(token);
        let bad = PlaySound {
            family: SoundFamily::Q1,
            sound: sound.clone(),
            origin: SoundOrigin::Local,
            actor: None,
            owner: None,
            channel: 0,
            volume: 2.0,
            attenuation: 0.0,
            audience: AudioAudience::World,
            delay_seconds: None,
            server_milliseconds: None,
        };
        assert!(matches!(audio.play(&bad), Err(AudioError::BadVolume)));
        assert!(audio.mix(44100 * 2 + 1).is_err());
    }

    #[test]
    fn stops_sounds_and_loops() {
        let owner = IdentityOwner::create("test").unwrap();
        let mut audio = engine(FakeFactory::named(&[]));
        audio.set_listeners(&[listener(&owner, 0)]).unwrap();
        let actor = owner.actor(1, 0);
        audio.update_actor(&actor, vec3(10.0, 0.0, 0.0)).unwrap();
        let sound = asset(blip(4410, None));
        let request = PlaySound {
            family: SoundFamily::Q1,
            sound: sound.clone(),
            origin: SoundOrigin::Actor { actor: actor.clone() },
            actor: Some(actor.clone()),
            owner: None,
            channel: 1,
            volume: 1.0,
            attenuation: 0.0,
            audience: AudioAudience::World,
            delay_seconds: None,
            server_milliseconds: None,
        };
        assert_eq!(audio.play(&request).unwrap(), 1);
        audio.stop_sound(&actor, Some(SoundChannel::Weapon), &AudioAudience::World).unwrap();
        audio.stop_sound(&actor, None, &AudioAudience::World).unwrap();
        let persistent = LoopSound {
            family: SoundFamily::Q1,
            sound: sound.clone(),
            origin: SoundOrigin::Actor { actor: actor.clone() },
            actor: actor.clone(),
            owner: None,
            velocity: vec3(0.0, 0.0, 0.0),
            frame_number: 0,
            volume: 1.0,
            attenuation: 1.0,
            lifetime: LoopLifetime::Persistent,
            audience: AudioAudience::World,
        };
        audio.start_loop(&persistent).unwrap();
        audio.begin_loop_frame();
        audio.end_loop_frame().unwrap();
        let mixed = audio.mix(16).unwrap();
        assert!(mixed.iter().any(|sample| *sample != 0));
        audio.stop_loop(&actor, &AudioAudience::World, None).unwrap();
        assert!(audio.add_static_sound(&owner.seat(0), &asset(blip(64, Some(0))), vec3(0.0, 0.0, 0.0), 200.0, 1.0, 3).unwrap());
        audio.remove_static_sound(&owner.seat(0), 3).unwrap();
        audio.update_ambient(&owner.seat(0), &[sound], &[1.0], 0.016, 0.3, 100.0).unwrap();
    }

    #[test]
    fn manages_q3_seat_loops() {
        let owner = IdentityOwner::create("test").unwrap();
        let mut audio = engine(FakeFactory::named(&[]));
        let seat = owner.seat(0);
        audio.set_listeners(&[listener(&owner, 0)]).unwrap();
        let actor = owner.actor(2, 0);
        let provider = ProviderId::new("q3", "guest");
        audio.update_q3_seat_actor(&seat, &actor, vec3(5.0, 0.0, 0.0), Some(&provider)).unwrap();
        let sound = asset(blip(2205, None));
        let frame_loop = LoopSound {
            family: SoundFamily::Q3,
            sound: sound.clone(),
            origin: SoundOrigin::Fixed { position: vec3(5.0, 0.0, 0.0) },
            actor: actor.clone(),
            owner: Some(provider.clone()),
            velocity: vec3(0.0, 0.0, 0.0),
            frame_number: 1,
            volume: 1.0,
            attenuation: 1.0,
            lifetime: LoopLifetime::Frame,
            audience: AudioAudience::Seat { seat: seat.clone() },
        };
        audio.start_loop(&frame_loop).unwrap();
        audio.begin_loop_frame();
        audio.end_loop_frame().unwrap();
        audio.clear_q3_seat_loops(&seat, false, Some(&provider)).unwrap();
        audio.start_loop(&frame_loop).unwrap();
        audio.stop_q3_seat_loop(&seat, &actor, Some(&provider)).unwrap();
        let world_loop = LoopSound {
            audience: AudioAudience::World,
            ..frame_loop.clone()
        };
        audio.start_loop(&world_loop).unwrap();
        audio.stop_q3_seat_loop(&seat, &actor, Some(&provider)).unwrap();
        audio.release_q3_seat_owner(&seat, &provider).unwrap();
        assert!(matches!(audio.clear_q3_seat_loops(&owner.seat(9), true, None), Err(AudioError::UnknownSeat)));
    }

    #[test]
    fn queues_streams_and_music() {
        let owner = IdentityOwner::create("test").unwrap();
        let mut audio = engine(FakeFactory::named(&[]));
        audio.set_listeners(&[listener(&owner, 0)]).unwrap();
        let target = AudioStreamTarget {
            id: "movie".to_string(),
            gain: 1.0,
            audience: AudioAudience::World,
        };
        audio
            .queue_stream(
                target.clone(),
                &StreamPcm {
                    samples: StreamSamples::S16(vec![2000, -2000]),
                    sample_rate: 44100,
                    channels: 2,
                    source_sample: 0,
                    reset_stream: true,
                },
            )
            .unwrap();
        let mixed = audio.mix(1).unwrap();
        assert_eq!(mixed, vec![2000, -2000]);
        let checkpoint = audio.capture_stream_checkpoint("movie").unwrap().expect("checkpoint");
        assert!(audio.capture_stream_checkpoint("missing").unwrap().is_none());
        audio.stop_stream("movie");
        let target2 = AudioStreamTarget {
            id: "movie2".to_string(),
            gain: 1.0,
            audience: AudioAudience::World,
        };
        audio.restore_stream_checkpoint(target2.clone(), Some(checkpoint)).unwrap();
        audio.restore_stream_checkpoint(
            AudioStreamTarget {
                id: "noop".to_string(),
                gain: 1.0,
                audience: AudioAudience::World,
            },
            None,
        )
        .unwrap();
        assert!(audio.capture_stream_checkpoint("noop").unwrap().is_none());
        assert!(matches!(
            audio.restore_stream_checkpoint(target2.clone(), Some(RawCheckpoint {
                output_rate: 44100,
                input_rate: 44100,
                channels: 2,
                origin: 0.0,
                output_frames: 0,
                end: 0,
                paused: false,
                segments: Vec::new(),
            })),
            Err(AudioError::StreamLaneUsed)
        ));
        audio.pause_stream("movie2", true);
        let silent = audio.mix(1).unwrap();
        assert_eq!(silent, vec![0, 0]);
        let player = MusicPlayer::new(44100, SoundFamily::Q3, MusicVolumeMode::Immediate, MusicControls::new());
        let music_target = AudioStreamTarget {
            id: "music".to_string(),
            gain: 1.0,
            audience: AudioAudience::World,
        };
        audio.attach_music(music_target.clone(), player).unwrap();
        audio.update_music();
        let player = MusicPlayer::new(22050, SoundFamily::Q3, MusicVolumeMode::Immediate, MusicControls::new());
        assert!(matches!(audio.attach_music(music_target, player), Err(AudioError::MusicRateMismatch)));
        audio.stop_music("music");
    }

    #[test]
    fn pumps_and_selects_output() {
        let owner = IdentityOwner::create("test").unwrap();
        let mut audio = engine(FakeFactory::named(&["fake"]));
        audio.set_listeners(&[listener(&owner, 0)]).unwrap();
        assert!(matches!(audio.pump(None, 0.0), Err(AudioError::NotOpen)));
        audio.open_device(Some("fake"), None).unwrap();
        assert!(matches!(audio.open_device(None, None), Err(AudioError::AlreadyOpen)));
        assert_eq!(audio.output_state(), OutputState::Paused);
        let config = audio.output_configuration().unwrap();
        assert_eq!(config.sample_rate, 44100);
        assert_eq!(config.channels, 2);
        let queued = audio.pump(None, 0.0).unwrap();
        assert_eq!(queued, (f64::from(44100) * 0.2).ceil() as u64);
        assert_eq!(audio.output_state(), OutputState::Playing);
        assert!(audio.queued_frames().unwrap() > 0);
        let clock = audio.voice_clock().unwrap();
        assert_eq!(clock.sample_rate, 44100);
        assert!(!clock.paused);
        audio.pause(true).unwrap();
        assert_eq!(audio.pump(None, 0.0).unwrap(), 0);
        assert_eq!(audio.mix(4).unwrap(), vec![0; 8]);
        audio.pause(false).unwrap();
        let format = audio.output_format();
        audio.select_output(Some("fake"), &format, false).unwrap();
        let mono = AudioOutputFormat {
            sample_rate: 22050,
            channels: 1,
            sample_bits: 16,
        };
        audio.select_output(Some("fake"), &mono, false).unwrap();
        assert_eq!(audio.output_format(), mono);
        audio.detach_output().unwrap();
        assert_eq!(audio.output_state(), OutputState::Detached);
        assert_eq!(audio.selected_output().as_deref(), Some("fake"));
        audio.detach_output().unwrap();
    }

    #[test]
    fn recovers_output_selection() {
        let owner = IdentityOwner::create("test").unwrap();
        let factory = FakeFactory::named(&["fake", "other"]);
        let shared = Rc::new(factory);
        let mut audio = UnifiedAudio::new(UnifiedAudioOptions {
            sample_rate: None,
            output_format: None,
            milliseconds: Box::new(|| 0),
            random: Box::new(|| 0),
            max_actors: None,
            on_sound: None,
            device_factory: Some(Rc::clone(&shared) as Rc<dyn AudioDeviceFactory>),
        })
        .unwrap();
        audio.set_listeners(&[listener(&owner, 0)]).unwrap();
        audio.open_device(Some("fake"), None).unwrap();
        audio.pump(None, 0.0).unwrap();
        let mono = AudioOutputFormat {
            sample_rate: 22050,
            channels: 1,
            sample_bits: 16,
        };
        shared.push(ScriptedOpen::Unavailable);
        audio.select_output(Some("other"), &mono, false).unwrap();
        assert_eq!(audio.selected_output().as_deref(), Some("other"));
        assert_eq!(audio.output_format(), mono);
        shared.push(ScriptedOpen::Unavailable);
        shared.push(ScriptedOpen::Unavailable);
        let stereo = AudioOutputFormat {
            sample_rate: 44100,
            channels: 2,
            sample_bits: 16,
        };
        assert!(matches!(audio.select_output(Some("gone"), &stereo, false), Err(AudioError::Device(_))));
        assert_eq!(audio.selected_output().as_deref(), Some("other"));
        shared.push(ScriptedOpen::Unavailable);
        shared.push(ScriptedOpen::Unavailable);
        shared.push(ScriptedOpen::Failed);
        assert!(matches!(
            audio.select_output(Some("gone"), &stereo, false),
            Err(AudioError::OutputSelectionFailed(_))
        ));
        assert_eq!(audio.output_state(), OutputState::Detached);
        let opens = shared.opens.borrow();
        assert!(opens.iter().any(|(name, format, _)| name.as_deref() == Some("fake") && format.sample_rate == 44100));
    }

    #[test]
    fn reports_queue_failures_and_custom_buffers() {
        let owner = IdentityOwner::create("test").unwrap();
        let mut audio = engine(FakeFactory::queue_fail());
        audio.set_listeners(&[listener(&owner, 0)]).unwrap();
        audio.open_device(Some("fake"), Some(2048)).unwrap();
        assert!(matches!(audio.pump(None, 0.0), Err(AudioError::Device(_))));
        let format = audio.output_format();
        assert!(matches!(audio.select_output(Some("other"), &format, false), Err(AudioError::Device(_))));
        assert_eq!(audio.selected_output().as_deref(), Some("fake"));
        let factory = FakeFactory::named(&["fake"]);
        factory.push(ScriptedOpen::Device(FakeSpec {
            buffer_frames: 256,
            queue_fail: false,
        }));
        let mut audio = engine(factory);
        audio.open_device(Some("fake"), None).unwrap();
        assert_eq!(audio.output_configuration().unwrap().buffer_frames, 256);
    }

    #[test]
    fn transfers_detaches_and_closes() {
        let owner = IdentityOwner::create("test").unwrap();
        let mut audio = engine(FakeFactory::named(&["fake"]));
        audio.set_listeners(&[listener(&owner, 0)]).unwrap();
        audio.open_device(Some("fake"), None).unwrap();
        audio.pump(None, 0.0).unwrap();
        let mut next = engine(FakeFactory::named(&["fake"]));
        let commit = audio.prepare_output_transfer(&mut next).unwrap();
        commit();
        assert_eq!(audio.output_state(), OutputState::Detached);
        assert_eq!(next.output_state(), OutputState::Playing);
        assert_eq!(next.queued_frames().unwrap(), (f64::from(44100) * 0.2).ceil() as u64);
        assert_eq!(next.pump(None, 0.0).unwrap(), 0);
        let mut other = engine(FakeFactory::named(&["fake"]));
        other.open_device(Some("fake"), None).unwrap();
        assert!(matches!(next.prepare_output_transfer(&mut other), Err(AudioError::ReplacementOwnsOutput)));
        let mut slow = engine_with_options(FakeFactory::named(&["fake"]), Some(22050), None);
        assert!(matches!(next.prepare_output_transfer(&mut slow), Err(AudioError::ReplacementRateMismatch)));
        next.close().unwrap();
        next.close().unwrap();
        assert_eq!(next.output_state(), OutputState::Closed);
        assert!(matches!(next.mix(1), Err(AudioError::Closed)));
        assert!(matches!(next.set_listeners(&[]), Err(AudioError::Closed)));
    }

    #[test]
    fn resets_rounds_and_applies_acoustics() {
        let owner = IdentityOwner::create("test").unwrap();
        let mut audio = engine(FakeFactory::named(&[]));
        let seat = listener(&owner, 0);
        audio.set_listeners(&[seat.clone()]).unwrap();
        audio
            .set_geometry_transmission(Some(Box::new(|_, _| 0.5)))
            .unwrap();
        audio.set_environment(&seat.seat, Vec::new(), Box::new(|_, _, _, _| super::super::environments::AudioTrace {
            fraction: 1.0,
            end: [0.0, 0.0, 0.0],
            material: None,
            sky: false,
        })).unwrap();
        let sound = asset(blip(4410, None));
        audio
            .play(&PlaySound {
                family: SoundFamily::Q1,
                sound,
                origin: SoundOrigin::Fixed { position: vec3(200.0, 0.0, 0.0) },
                actor: None,
                owner: None,
                channel: 0,
                volume: 1.0,
                attenuation: 1.0,
                audience: AudioAudience::World,
                delay_seconds: None,
                server_milliseconds: None,
            })
            .unwrap();
        assert!(audio.mix(8).unwrap().iter().any(|sample| *sample != 0));
        let mut wet = seat.clone();
        wet.underwater = true;
        audio.set_listeners(&[wet]).unwrap();
        assert_eq!(audio.mix(8).unwrap().len(), 16);
        audio.set_doppler_enabled(false).unwrap();
        audio.set_effects_volume(0.3).unwrap();
        assert!(matches!(audio.set_effects_volume(-1.0), Err(AudioError::BadEffectsGain)));
        audio.reset_round().unwrap();
        audio.set_listeners(std::slice::from_ref(&seat)).unwrap();
        audio.set_geometry_transmission(None).unwrap();
        audio.stop_all().unwrap();
    }

    #[test]
    fn enforces_actor_capacity_and_loop_positions() {
        let owner = IdentityOwner::create("test").unwrap();
        let mut audio = engine_with_options(FakeFactory::named(&[]), None, Some(2));
        audio.set_listeners(&[listener(&owner, 0)]).unwrap();
        let first = owner.actor(0, 0);
        audio.update_actor(&first, vec3(0.0, 0.0, 0.0)).unwrap();
        let second = owner.actor(1, 0);
        assert!(matches!(audio.update_actor(&second, vec3(0.0, 0.0, 0.0)), Err(AudioError::ActorCapacity)));
        let mut roomy = engine(FakeFactory::named(&[]));
        roomy.set_listeners(&[listener(&owner, 0)]).unwrap();
        let ghost = owner.actor(7, 0);
        let sound = asset(blip(64, None));
        assert!(matches!(
            roomy.start_loop(&LoopSound {
                family: SoundFamily::Q2,
                sound,
                origin: SoundOrigin::Actor { actor: ghost.clone() },
                actor: ghost,
                owner: None,
                velocity: vec3(0.0, 0.0, 0.0),
                frame_number: 0,
                volume: 1.0,
                attenuation: 1.0,
                lifetime: LoopLifetime::Persistent,
                audience: AudioAudience::World,
            }),
            Err(AudioError::LoopPosition)
        ));
    }
}



