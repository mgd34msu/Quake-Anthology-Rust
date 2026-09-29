//! PCM mixing: voices, loops, raw stream, and paint blocks.
//!
//! Donor provenance: `src/audio/mixer.ts` (`AudioMixer`,
//! `SOUND_TIME_EPOCH`, from `snd_mix.c`, `snd_dma.c`, `snd_mem.c`).
//! The spatialization and headless pool already in
//! [`crate::audio`] stay untouched; this module ports the remaining
//! mixer: scheduling, painting, raw queueing, and clocks.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use qa_core::cvar::CvarRegistry;
use qa_core::math::{add3, dot3, length3, sub3, vec3, Axis, Vec3};

use super::error::AudioError;
use super::paint::{int32, write_linear_blast_stereo16_float};
use super::types::{SharedPcm, SoundAsset, VoiceStopReason};
use super::wav::PcmSound;
use crate::audio::{source_sound_channel, spatialize_sound_origin, ChannelCommand, OutputChannels, SoundChannel, SoundFamily};

/// SDL paint epoch (`SOUND_TIME_EPOCH`).
pub const SOUND_TIME_EPOCH: i64 = 0x4000_0000;
/// Raw ring capacity in frames.
pub const RAW_SAMPLE_CAPACITY: usize = 16384;
/// Paint block size in frames.
pub const PAINTBUFFER_SIZE: usize = 4096;
/// Doppler chunk size in frames.
pub const SND_CHUNK_SIZE: usize = 1024;

/// Voice origin.
#[derive(Debug, Clone, PartialEq)]
pub enum MixerVoiceOrigin {
    /// At the listener.
    Local,
    /// At a fixed position.
    Fixed {
        /// Position.
        position: Vec3,
    },
    /// Following an entity.
    Entity {
        /// Entity number.
        entity: i64,
    },
}

/// Stereo volume pair.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct MixerStereoVolume {
    /// Left volume.
    pub left: f64,
    /// Right volume.
    pub right: f64,
}

/// `S_StartSound` options.
#[derive(Debug, Clone, PartialEq)]
pub struct StartSoundOptions {
    /// Entity number.
    pub entity: i64,
    /// Game channel number.
    pub channel: i32,
    /// Voice origin.
    pub origin: MixerVoiceOrigin,
    /// Quake channel volume units (127 in `S_StartSound`).
    pub volume: i32,
}

/// Frame loop options (`S_AddLoopingSound`).
#[derive(Debug, Clone, PartialEq)]
pub struct FrameLoopingSoundOptions {
    /// Entity number.
    pub entity: i64,
    /// Loop origin.
    pub origin: Vec3,
    /// Frame velocity.
    pub velocity: Vec3,
    /// Frame number.
    pub frame_number: i32,
    /// Channel volume.
    pub volume: Option<i32>,
}

/// Persistent loop options (`S_AddRealLoopingSound`).
#[derive(Debug, Clone, PartialEq)]
pub struct RealLoopingSoundOptions {
    /// Entity number.
    pub entity: i64,
    /// Loop origin.
    pub origin: Vec3,
    /// Frame velocity.
    pub velocity: Vec3,
    /// Channel volume.
    pub volume: Option<i32>,
}

/// Source sound options (Q1/Q2/static/ambient/entity-loop).
#[derive(Debug, Clone, PartialEq)]
pub struct SourceSoundOptions {
    /// Entity number (-1 for static/ambient).
    pub entity: i64,
    /// Voice origin.
    pub origin: MixerVoiceOrigin,
    /// Normalized gain.
    pub volume: f64,
    /// Attenuation.
    pub attenuation: f64,
}

/// Q2 sound options with synchronized start.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2SoundOptions {
    /// Entity number.
    pub entity: i64,
    /// Voice origin.
    pub origin: MixerVoiceOrigin,
    /// Normalized gain.
    pub volume: f64,
    /// Attenuation.
    pub attenuation: f64,
    /// Synchronized start delay in seconds.
    pub delay_seconds: Option<f64>,
    /// Server clock in milliseconds.
    pub server_milliseconds: Option<f64>,
}

/// Source loop entry.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceLoopEntry {
    /// Game family.
    pub family: SoundFamily,
    /// Entity number.
    pub entity: i64,
    /// Loop sound.
    pub sound: SharedPcm,
    /// Loop origin.
    pub origin: Vec3,
    /// Normalized gain.
    pub volume: f64,
    /// Attenuation.
    pub attenuation: Option<f64>,
}

/// Shared sound memory: prepared sounds read the live frame count.
pub type SharedMixerMemory = Rc<RefCell<Box<dyn MixerSoundMemory>>>;

/// Engine sound handles borrow the bank's resampled allocation.
pub trait MixerSoundMemory {
    /// Resampled frame count.
    fn frame_count(&self, sound: &PcmSound) -> usize;
    /// Whether resampled data is resident.
    fn has_data(&self, sound: &PcmSound) -> bool;
    /// Sample one resampled frame.
    fn sample(&self, sound: &PcmSound, frame: usize) -> i32;
    /// Touch the allocation clock.
    fn touch(&mut self, sound: &PcmSound, milliseconds: i64);
}

/// Voice policy role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceRole {
    /// One-shot effect.
    Effect,
    /// Static loop.
    Static,
    /// Ambient bed.
    Ambient,
    /// Entity loop.
    EntityLoop,
}

/// Source voice policy.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VoicePolicy {
    /// Attenuation slope.
    pub attenuation: f64,
    /// Full-volume radius.
    pub distance_offset: f64,
    /// Stereo scale.
    pub stereo_scale: f64,
    /// Unattenuated mono output.
    pub unattenuated_mono: bool,
    /// Loop start in output frames.
    pub loop_start: Option<usize>,
    /// Synchronized gain ceiling.
    pub synchronized_gain_limit: Option<f64>,
    /// Role.
    pub role: VoiceRole,
    /// Role key.
    pub key: i64,
}

/// Mixer voice event (the seat arrives from the engine).
#[derive(Debug, Clone, PartialEq)]
pub enum MixerVoiceEvent {
    /// A voice started painting.
    Start {
        /// Voice id.
        voice_id: u64,
        /// Sound asset.
        sound: SoundAsset,
        /// Output sample of the event.
        output_sample: i64,
        /// Output rate in Hz.
        sample_rate: u32,
        /// Source offset in seconds.
        source_offset_seconds: f64,
    },
    /// A voice stopped.
    Stop {
        /// Voice id.
        voice_id: u64,
        /// Output sample of the event.
        output_sample: i64,
        /// Stop reason.
        reason: VoiceStopReason,
    },
}

/// Voice clock position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VoiceStart {
    /// Scheduled at a sample with a tie-break order.
    Scheduled {
        /// Start sample.
        sample: i64,
        /// Schedule order.
        order: i64,
    },
    /// Not yet scanned.
    Pending,
    /// Painting since a sample.
    Started {
        /// Start sample.
        sample: i64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Notification {
    Pending,
    Started,
    Stopped,
}

#[derive(Clone)]
struct PreparedSound {
    doppler_sums: Option<Vec<f64>>,
    sound: SharedPcm,
    step256: f64,
    memory: Option<SharedMixerMemory>,
    output_frames: usize,
}

impl std::fmt::Debug for PreparedSound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedSound")
            .field("output_frames", &self.output_frames)
            .field("step256", &self.step256)
            .field("memory", &self.memory.is_some())
            .finish()
    }
}

#[derive(Debug, Clone)]
struct OneShotVoice {
    voice_id: u64,
    asset: Option<SoundAsset>,
    notification: Notification,
    prepared: PreparedSound,
    entity: i64,
    channel: Option<SoundChannel>,
    policy: Option<VoicePolicy>,
    origin: MixerVoiceOrigin,
    volume: f64,
    stereo_volume: MixerStereoVolume,
    start: VoiceStart,
    allocated_at: i64,
}

#[derive(Debug, Clone)]
struct LoopVoice {
    prepared: PreparedSound,
    entity: i64,
    /// Frame velocity at update time (donor state; Doppler uses the scales).
    #[allow(dead_code)]
    velocity: Vec3,
    volume: f64,
    lifetime: LoopLifetime,
    active: bool,
    doppler: bool,
    doppler_scale: f64,
    old_doppler_scale: f64,
    frame_number: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoopLifetime {
    Frame,
    Persistent,
}

#[derive(Debug, Clone)]
struct LoopMix {
    prepared: PreparedSound,
    left_volume: f64,
    right_volume: f64,
    doppler: bool,
    doppler_scale: f64,
    old_doppler_scale: f64,
}

/// Paint range request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SoundPaintRange {
    /// First frame.
    pub start_frame: i64,
    /// One past the last frame.
    pub end_frame: i64,
}

/// Mix request: consume frames or paint an explicit range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MixRequest {
    /// Consume frames at the paint clock.
    Consume(i64),
    /// Paint a range without advancing device sound time.
    Range(SoundPaintRange),
}

/// Console output for mixer diagnostics.
pub type MixerConsole = Box<dyn FnMut(&str)>;

fn require_positive_integer(value: i64, name: &str) -> Result<(), AudioError> {
    if value <= 0 {
        return Err(AudioError::NotPositiveInteger { name: name.to_string() });
    }
    Ok(())
}

fn require_entity(entity: i64, limit: usize, inclusive_limit: Option<usize>) -> Result<(), AudioError> {
    let ok = match inclusive_limit {
        None => entity >= 0 && (entity as u64) < limit as u64,
        Some(top) => entity >= 0 && (entity as u64) <= top as u64,
    };
    if ok {
        Ok(())
    } else {
        Err(AudioError::BadEntity {
            limit: inclusive_limit.unwrap_or_else(|| limit - 1).to_string(),
        })
    }
}

fn require_channel(channel: i32) -> Result<(), AudioError> {
    if int32(f64::from(channel)) != channel {
        return Err(AudioError::BadChannel);
    }
    Ok(())
}

fn require_channel_volume(volume: i32) -> Result<(), AudioError> {
    if !(0..=255).contains(&volume) {
        return Err(AudioError::BadChannelVolume);
    }
    Ok(())
}

fn require_gain(gain: f64, name: &str) -> Result<(), AudioError> {
    if !gain.is_finite() || gain < 0.0 {
        return Err(AudioError::BadGain { name: name.to_string() });
    }
    Ok(())
}

fn checked_sample(samples: &[i16], index: usize) -> Result<i32, AudioError> {
    samples.get(index).copied().map(i32::from).ok_or_else(|| AudioError::BadSampleIndex {
        index: index.to_string(),
        length: samples.len().to_string(),
    })
}

fn validate_sound(sound: &PcmSound, allow_stereo: bool) -> Result<(), AudioError> {
    require_positive_integer(i64::from(sound.sample_rate), "PCM sample rate")?;
    if allow_stereo {
        if sound.frame_count as u64 > i64::MAX as u64 {
            return Err(AudioError::BadFrameCount);
        }
    } else {
        require_positive_integer(sound.frame_count as i64, "PCM frame count")?;
    }
    if sound.channels != 1 && sound.channels != 2 {
        return Err(AudioError::BadPcmChannels);
    }
    if !allow_stereo && sound.channels != 1 {
        return Err(AudioError::EffectNotMono);
    }
    let expected = sound.frame_count * usize::from(sound.channels);
    if sound.samples.len() != expected {
        return Err(AudioError::BadSampleCount {
            have: sound.samples.len().to_string(),
            want: expected.to_string(),
        });
    }
    if sound.loop_start.is_some_and(|marker| marker >= sound.frame_count) {
        return Err(AudioError::BadLoopStart);
    }
    Ok(())
}

/// PCM mixing with a borrowed allocation clock and no device ownership.
pub struct AudioMixer {
    output_rate: u32,
    capacity: usize,
    effects_volume: f32,
    next_voice: u64,
    voice_observer: Option<Box<dyn FnMut(MixerVoiceEvent)>>,
    allocate_voice_id: Option<Box<dyn FnMut() -> u64>>,
    transmission: Option<Box<dyn FnMut(Vec3) -> f64>>,
    transmission_cache: HashMap<String, f64>,
    music_volume: f64,
    doppler_enabled: bool,
    listener_entity: i64,
    listener_origin: Vec3,
    listener_axis: Axis,
    entity_positions: Vec<Vec3>,
    voices: Vec<Option<OneShotVoice>>,
    free_channels: Vec<usize>,
    loops: HashMap<i64, LoopVoice>,
    loop_channels: Vec<LoopMix>,
    raw_samples: Vec<i32>,
    painted_time: i64,
    source_begin_offset: i64,
    source_schedule_order: i64,
    sound_time: i64,
    raw_end_time: i64,
    enabled: bool,
    sound_cvars: Option<CvarRegistry>,
    console_output: Option<MixerConsole>,
    sound_memory: Option<SharedMixerMemory>,
    milliseconds: Box<dyn Fn() -> i64>,
    output_channels: u8,
    entity_capacity: usize,
}

impl AudioMixer {
    /// Mixer at an output rate with an allocation clock.
    pub fn new(output_rate: u32, milliseconds: Box<dyn Fn() -> i64>, capacity: usize, output_channels: u8, entity_capacity: usize) -> Result<Self, AudioError> {
        require_positive_integer(i64::from(output_rate), "output rate")?;
        require_positive_integer(capacity as i64, "channel capacity")?;
        require_positive_integer(entity_capacity as i64, "entity capacity")?;
        Ok(Self {
            output_rate,
            capacity,
            effects_volume: 0.8,
            next_voice: 0,
            voice_observer: None,
            allocate_voice_id: None,
            transmission: None,
            transmission_cache: HashMap::new(),
            music_volume: 0.25,
            doppler_enabled: true,
            listener_entity: 0,
            listener_origin: vec3(0.0, 0.0, 0.0),
            listener_axis: [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)],
            entity_positions: vec![vec3(0.0, 0.0, 0.0); entity_capacity],
            voices: vec![None; capacity],
            free_channels: (0..capacity).collect(),
            loops: HashMap::new(),
            loop_channels: Vec::new(),
            raw_samples: vec![0; RAW_SAMPLE_CAPACITY * 2],
            painted_time: 0,
            source_begin_offset: 0,
            source_schedule_order: 0,
            sound_time: 0,
            raw_end_time: 0,
            enabled: true,
            sound_cvars: None,
            console_output: None,
            sound_memory: None,
            milliseconds,
            output_channels,
            entity_capacity,
        })
    }

    /// Output rate in Hz.
    #[must_use]
    pub const fn output_rate(&self) -> u32 {
        self.output_rate
    }

    /// Initial slot allocation.
    #[must_use]
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    /// Paint clock.
    #[must_use]
    pub const fn sample_clock(&self) -> i64 {
        self.painted_time
    }

    /// Device sound clock.
    #[must_use]
    pub const fn sound_clock(&self) -> i64 {
        self.sound_time
    }

    /// Whether playback is enabled.
    #[must_use]
    pub const fn playback_enabled(&self) -> bool {
        self.enabled
    }

    /// Enable or disable playback.
    pub fn set_playback_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Bind sound cvars.
    pub fn bind_sound_cvars(&mut self, cvars: CvarRegistry) {
        self.sound_cvars = Some(cvars);
    }

    /// Bind console output.
    pub fn bind_console_output(&mut self, output: MixerConsole) {
        self.console_output = Some(output);
    }

    /// Bind sound memory.
    pub fn bind_sound_memory(&mut self, memory: Box<dyn MixerSoundMemory>) {
        self.sound_memory = Some(Rc::new(RefCell::new(memory)));
    }

    /// Observe voice starts and stops.
    pub fn set_voice_observer(&mut self, observer: Option<Box<dyn FnMut(MixerVoiceEvent)>>, allocate_id: Option<Box<dyn FnMut() -> u64>>) {
        self.voice_observer = observer;
        if allocate_id.is_some() {
            self.allocate_voice_id = allocate_id;
        }
    }

    fn alloc_id(&mut self) -> u64 {
        match self.allocate_voice_id.as_mut() {
            Some(allocate) => allocate(),
            None => {
                self.next_voice += 1;
                self.next_voice
            }
        }
    }

    /// Set geometry transmission.
    pub fn set_geometry_transmission(&mut self, transmission: Option<Box<dyn FnMut(Vec3) -> f64>>) {
        self.transmission = transmission;
        self.transmission_cache.clear();
    }

    fn voice_started(&mut self, index: usize) {
        let notify = match &self.voices[index] {
            Some(voice) if voice.notification == Notification::Pending && matches!(voice.start, VoiceStart::Started { .. }) && voice.asset.is_some() => {
                let start_sample = match voice.start {
                    VoiceStart::Started { sample } => sample,
                    _ => 0,
                };
                Some((voice.voice_id, voice.asset.clone().expect("checked"), start_sample))
            }
            _ => None,
        };
        let Some((voice_id, asset, start_sample)) = notify else {
            return;
        };
        if let Some(voice) = self.voices[index].as_mut() {
            voice.notification = Notification::Started;
        }
        let event = MixerVoiceEvent::Start {
            voice_id,
            sound: asset,
            output_sample: self.painted_time,
            sample_rate: self.output_rate,
            source_offset_seconds: 0i64.max(self.painted_time - start_sample) as f64 / f64::from(self.output_rate),
        };
        if let Some(observer) = self.voice_observer.as_mut() {
            observer(event);
        }
    }

    fn voice_stopped(&mut self, index: usize, reason: VoiceStopReason, sample: Option<i64>) {
        let notify = match &self.voices[index] {
            Some(voice) if voice.notification == Notification::Started => Some(voice.voice_id),
            _ => None,
        };
        let Some(voice_id) = notify else {
            return;
        };
        if let Some(voice) = self.voices[index].as_mut() {
            voice.notification = Notification::Stopped;
        }
        let event = MixerVoiceEvent::Stop {
            voice_id,
            output_sample: sample.unwrap_or(self.painted_time),
            reason,
        };
        if let Some(observer) = self.voice_observer.as_mut() {
            observer(event);
        }
    }

    fn transmit(&mut self, position: Vec3, volume: MixerStereoVolume) -> MixerStereoVolume {
        if self.transmission.is_none() || volume.left == 0.0 && volume.right == 0.0 {
            return volume;
        }
        let key = format!("{},{},{}", position.x, position.y, position.z);
        let gain = match self.transmission_cache.get(&key) {
            Some(gain) => *gain,
            None => {
                let gain = self.transmission.as_mut().expect("checked")(position);
                self.transmission_cache.insert(key, gain);
                gain
            }
        };
        if gain == 1.0 {
            volume
        } else {
            MixerStereoVolume {
                left: (volume.left * gain).trunc(),
                right: (volume.right * gain).trunc(),
            }
        }
    }

    /// Audible one-shot volumes, excluding scheduled and silent voices.
    pub fn channel_volumes(&self) -> Vec<(SharedPcm, f64, f64)> {
        self.voices
            .iter()
            .flatten()
            .filter(|voice| !matches!(voice.start, VoiceStart::Scheduled { .. }) && (voice.stereo_volume.left != 0.0 || voice.stereo_volume.right != 0.0))
            .map(|voice| (voice.prepared.sound.clone(), voice.stereo_volume.left, voice.stereo_volume.right))
            .collect()
    }

    /// Select paint time, advancing sound time monotonically.
    pub fn select_time(&mut self, sound_time: i64, paint_time: i64) -> Result<(), AudioError> {
        if sound_time < self.sound_time {
            return Err(AudioError::BadTimeSelect);
        }
        self.sound_time = sound_time;
        self.painted_time = paint_time;
        Ok(())
    }

    /// Rebase clocks into the same SDL epoch after a stop.
    pub fn rebase_time(&mut self, delivered_time: i64) -> Result<i64, AudioError> {
        if delivered_time <= self.sound_time || delivered_time < SOUND_TIME_EPOCH {
            return Err(AudioError::BadRebase);
        }
        let offset = (delivered_time / SOUND_TIME_EPOCH) * SOUND_TIME_EPOCH;
        self.sound_time = delivered_time - offset;
        self.painted_time -= offset;
        Ok(self.sound_time)
    }

    /// Raw stream end.
    #[must_use]
    pub const fn raw_end(&self) -> i64 {
        self.raw_end_time
    }

    /// Live raw sample allocation.
    #[must_use]
    pub fn raw_samples(&self) -> &[i32] {
        &self.raw_samples
    }

    /// Mutable raw sample allocation.
    pub fn raw_samples_mut(&mut self) -> &mut [i32] {
        &mut self.raw_samples
    }

    /// Set the listener, respatializing every started voice.
    pub fn set_listener(&mut self, entity: i32, origin: Vec3, axis: Axis) -> Result<(), AudioError> {
        self.transmission_cache.clear();
        if !self.enabled {
            return Ok(());
        }
        self.listener_entity = i64::from(entity);
        self.listener_origin = origin;
        self.listener_axis = axis;
        for index in 0..self.voices.len() {
            let Some(voice) = self.voices[index].clone() else {
                continue;
            };
            if matches!(voice.start, VoiceStart::Scheduled { .. }) {
                continue;
            }
            let stereo = if voice.policy.is_none() {
                self.spatialize(voice.entity, &voice.origin, voice.volume)?
            } else {
                self.policy_spatialize(&voice)?
            };
            if let Some(slot) = self.voices[index].as_mut() {
                slot.stereo_volume = stereo;
            }
        }
        self.loop_channels = self.collect_loop_mixes()?;
        Ok(())
    }

    /// Write an entity position cell.
    pub fn update_entity_position(&mut self, entity: i64, origin: Vec3) -> Result<(), AudioError> {
        require_entity(entity, self.entity_capacity, None)?;
        self.entity_positions[entity as usize] = origin;
        Ok(())
    }

    /// Set the effects volume.
    pub fn set_effects_volume(&mut self, volume: f64) -> Result<(), AudioError> {
        let rounded = volume as f32;
        let gain = (rounded * 255.0).trunc();
        if !gain.is_finite() || f64::from(gain) < -2_147_483_648.0 || f64::from(gain) > 2_147_483_647.0 {
            return Err(AudioError::BadEffectsConversion);
        }
        self.effects_volume = rounded;
        Ok(())
    }

    /// Set the music volume.
    pub fn set_music_volume(&mut self, volume: f64) -> Result<(), AudioError> {
        require_gain(volume, "music volume")?;
        self.music_volume = volume;
        Ok(())
    }

    /// Global Doppler permission.
    pub fn set_doppler_enabled(&mut self, enabled: bool) {
        self.doppler_enabled = enabled;
    }

    /// Start a listener-local sound.
    pub fn start_local_sound(&mut self, sound: &SharedPcm, channel: i32, source_name: Option<&str>) -> Result<bool, AudioError> {
        let listener = self.listener_entity;
        self.start_sound(
            sound,
            &StartSoundOptions {
                entity: listener,
                channel,
                origin: MixerVoiceOrigin::Entity { entity: listener },
                volume: 127,
            },
            source_name,
        )
    }

    /// Start a Q3 sound.
    pub fn start_sound(&mut self, sound: &SharedPcm, options: &StartSoundOptions, source_name: Option<&str>) -> Result<bool, AudioError> {
        if !self.enabled {
            return Ok(false);
        }
        require_channel(options.channel)?;
        let command = source_sound_channel(SoundFamily::Q3, options.channel).map_err(|_| AudioError::BadSourceChannel)?;
        self.start_shared_sound(sound, options.entity, options.origin.clone(), options.volume, &command, source_name, None)
    }

    /// Start a sound with an explicit channel command.
    #[allow(clippy::too_many_arguments)]
    pub fn start_shared_sound(
        &mut self,
        sound: &SharedPcm,
        entity: i64,
        origin: MixerVoiceOrigin,
        volume: i32,
        command: &ChannelCommand,
        source_name: Option<&str>,
        asset: Option<SoundAsset>,
    ) -> Result<bool, AudioError> {
        if !self.enabled {
            return Ok(false);
        }
        if self.sound_memory.is_none() {
            validate_sound(sound, false)?;
        }
        if matches!(origin, MixerVoiceOrigin::Fixed { .. }) {
            if !(-2_147_483_648..=2_147_483_647).contains(&entity) {
                return Err(AudioError::BadFixedEntity);
            }
        } else {
            require_entity(entity, self.entity_capacity, Some(self.entity_capacity))?;
        }
        require_channel_volume(volume)?;
        let prepared = self.prepare(sound)?;
        if self.diagnostic_setting("s_show")? == 1 {
            let (Some(source_name), Some(console)) = (source_name, self.console_output.as_mut()) else {
                return Err(AudioError::StartDiagnostics);
            };
            console(&format!("{} : {source_name}\n", self.painted_time));
            if !self.enabled {
                return Err(AudioError::PlaybackEnded);
            }
        }
        let time = self.allocation_time()?;
        let mut same_sound_count = 0;
        for voice in self.voices.iter().flatten() {
            if voice.policy.is_some() || voice.entity != entity || !std::rc::Rc::ptr_eq(&voice.prepared.sound, sound) {
                continue;
            }
            if int32((time - voice.allocated_at) as f64) < 50 {
                return Ok(false);
            }
            same_sound_count += 1;
        }
        if same_sound_count > if entity == self.listener_entity { 8 } else { 4 } {
            return Ok(false);
        }
        if let Some(memory) = self.sound_memory.as_ref() {
            memory.borrow_mut().touch(sound, time);
        }
        self.replace_channel(entity, command, false, VoiceStopReason::Replaced);
        let channel = match self.free_channels.pop() {
            Some(channel) => channel,
            None => self.voices.len(),
        };
        let allocated_at = if channel == self.voices.len() { time } else { self.allocation_time()? };
        let voice = OneShotVoice {
            voice_id: self.alloc_id(),
            asset,
            notification: Notification::Pending,
            prepared,
            entity,
            channel: match command {
                ChannelCommand::Channel(channel) => Some(*channel),
                _ => None,
            },
            policy: None,
            origin,
            volume: f64::from(volume),
            stereo_volume: MixerStereoVolume {
                left: f64::from(volume),
                right: f64::from(volume),
            },
            start: VoiceStart::Pending,
            allocated_at,
        };
        if channel == self.voices.len() {
            self.voices.push(Some(voice));
        } else {
            self.voices[channel] = Some(voice);
        }
        Ok(true)
    }

    fn policy_spatialize(&mut self, voice: &OneShotVoice) -> Result<MixerStereoVolume, AudioError> {
        if voice.policy.is_some_and(|policy| policy.role == VoiceRole::Ambient) {
            return Ok(voice.stereo_volume);
        }
        if matches!(voice.origin, MixerVoiceOrigin::Local) || voice.entity == self.listener_entity {
            return Ok(MixerStereoVolume {
                left: voice.volume,
                right: voice.volume,
            });
        }
        let position = self.resolve_origin(&voice.origin)?;
        let delta = sub3(position, self.listener_origin);
        let distance = length3(delta);
        let pan = if distance == 0.0 { 0.0 } else { f64::from(-dot3(delta, self.listener_axis[1])) / f64::from(distance) };
        let Some(policy) = voice.policy else {
            return Err(AudioError::MissingPolicy);
        };
        let gain = voice.volume * (1.0 - 0f64.max(f64::from(distance) - policy.distance_offset) * policy.attenuation);
        let mono = self.output_channels == 1 || policy.unattenuated_mono && policy.attenuation == 0.0;
        let volume = MixerStereoVolume {
            left: 0f64.max((gain * if mono { 1.0 } else { policy.stereo_scale * (1.0 - pan) }).trunc()),
            right: 0f64.max((gain * if mono { 1.0 } else { policy.stereo_scale * (1.0 + pan) }).trunc()),
        };
        Ok(if policy.attenuation == 0.0 { volume } else { self.transmit(position, volume) })
    }

    /// Start a Q1 effect.
    pub fn start_q1_sound(
        &mut self,
        sound: &SharedPcm,
        options: &SourceSoundOptions,
        command: &ChannelCommand,
        random: Option<&mut dyn FnMut() -> i64>,
        asset: Option<SoundAsset>,
    ) -> Result<bool, AudioError> {
        self.admit_source_sound(
            sound,
            options,
            command,
            VoicePolicy {
                attenuation: options.attenuation / 1000.0,
                distance_offset: 0.0,
                stereo_scale: 1.0,
                unattenuated_mono: false,
                loop_start: None,
                synchronized_gain_limit: None,
                role: VoiceRole::Effect,
                key: 0,
            },
            random,
            None,
            asset,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn admit_source_sound(
        &mut self,
        sound: &SharedPcm,
        options: &SourceSoundOptions,
        command: &ChannelCommand,
        policy: VoicePolicy,
        mut random: Option<&mut dyn FnMut() -> i64>,
        scheduled: Option<(i64, i64)>,
        asset: Option<SoundAsset>,
    ) -> Result<bool, AudioError> {
        if !self.enabled {
            return Ok(false);
        }
        validate_sound(sound, false)?;
        if sound.channels != 1 || sound.frame_count < 1 {
            return Err(AudioError::SourceEffectFormat);
        }
        if !options.volume.is_finite() || options.volume < 0.0 || !options.attenuation.is_finite() || options.attenuation < 0.0 {
            return Err(AudioError::BadSourceGain);
        }
        let ratio = f64::from(sound.sample_rate) / f64::from(self.output_rate);
        let output_frames = (sound.frame_count as f64 / ratio).trunc();
        if output_frames < 1.0 {
            return Err(AudioError::ZeroResample);
        }
        let prepared = PreparedSound {
            doppler_sums: None,
            sound: sound.clone(),
            step256: ratio * 256.0,
            memory: None,
            output_frames: output_frames as usize,
        };
        let marker = policy.loop_start.or_else(|| sound.loop_start.map(|marker| (marker as f64 / ratio).trunc() as usize));
        if marker.is_some_and(|marker| marker >= prepared.output_frames) {
            return Err(AudioError::LoopOutsidePcm);
        }
        let mut offset = 0i64;
        let jitter = self.voices.iter().flatten().any(|voice| {
            voice.policy.is_some_and(|policy| policy.role == VoiceRole::Effect)
                && std::rc::Rc::ptr_eq(&voice.prepared.sound, sound)
                && matches!(voice.start, VoiceStart::Started { sample } if sample == self.painted_time)
        });
        if jitter {
            if let Some(random) = random {
                let value = random();
            if value < 0 {
                return Err(AudioError::BadRandom);
            }
            offset = (prepared.output_frames as i64 - 1).min(value % 1.max((0.1 * f64::from(self.output_rate)).trunc() as i64));
            }
        }
        let start = match scheduled {
            None => VoiceStart::Started {
                sample: if policy.role == VoiceRole::EntityLoop { 0 } else { self.painted_time - offset },
            },
            Some((sample, order)) => VoiceStart::Scheduled { sample, order },
        };
        let voice = OneShotVoice {
            voice_id: self.alloc_id(),
            asset,
            notification: Notification::Pending,
            prepared,
            entity: options.entity,
            channel: match command {
                ChannelCommand::Channel(channel) => Some(*channel),
                _ => None,
            },
            origin: options.origin.clone(),
            volume: (options.volume * 255.0).trunc(),
            stereo_volume: MixerStereoVolume::default(),
            start,
            allocated_at: self.allocation_time()?,
            policy: Some(VoicePolicy { loop_start: marker, ..policy }),
        };
        if scheduled.is_none() {
            let stereo = self.policy_spatialize(&voice)?;
            let silent = policy.role == VoiceRole::Effect && stereo.left == 0.0 && stereo.right == 0.0;
            if silent {
                return Ok(false);
            }
            self.replace_channel(options.entity, command, false, VoiceStopReason::Replaced);
            let index = match self.free_channels.pop() {
                Some(index) => index,
                None => self.voices.len(),
            };
            let mut voice = voice;
            voice.stereo_volume = stereo;
            if index == self.voices.len() {
                self.voices.push(Some(voice));
            } else {
                self.voices[index] = Some(voice);
            }
            self.voice_started(index);
        } else {
            let index = match self.free_channels.pop() {
                Some(index) => index,
                None => self.voices.len(),
            };
            if index == self.voices.len() {
                self.voices.push(Some(voice));
            } else {
                self.voices[index] = Some(voice);
            }
        }
        Ok(true)
    }

    /// Add a static loop.
    pub fn add_static_sound(&mut self, sound: &SharedPcm, origin: Vec3, volume: f64, attenuation: f64, key: i64) -> Result<bool, AudioError> {
        if sound.loop_start.is_none() {
            return Err(AudioError::StaticLoop);
        }
        self.admit_source_sound(
            sound,
            &SourceSoundOptions {
                entity: -1,
                origin: MixerVoiceOrigin::Fixed { position: origin },
                volume: volume / 255.0,
                attenuation,
            },
            &ChannelCommand::Auto,
            VoicePolicy {
                attenuation: attenuation / 64000.0,
                distance_offset: 0.0,
                stereo_scale: 1.0,
                unattenuated_mono: false,
                loop_start: None,
                synchronized_gain_limit: None,
                role: VoiceRole::Static,
                key,
            },
            None,
            None,
            None,
        )
    }

    /// Remove static loops by key.
    pub fn remove_static_sound(&mut self, key: i64) {
        for index in 0..self.voices.len() {
            let matches = self.voices[index].as_ref().is_some_and(|voice| voice.policy.is_some_and(|policy| policy.role == VoiceRole::Static && policy.key == key));
            if matches {
                self.free_channel(index, VoiceStopReason::Stopped);
            }
        }
    }

    /// Update the two ambient beds.
    pub fn update_ambient(&mut self, sounds: &[SharedPcm], levels: &[f64], elapsed_seconds: f64, level: f64, fade: f64) -> Result<(), AudioError> {
        if !self.enabled {
            return Ok(());
        }
        for key in 0..2i64 {
            let sound = sounds.get(key as usize);
            let amount = levels.get(key as usize).copied();
            let mut index = self.voices.iter().position(|voice| voice.as_ref().is_some_and(|voice| voice.policy.is_some_and(|policy| policy.role == VoiceRole::Ambient && policy.key == key)));
            if sound.is_none() || amount.is_none() || level == 0.0 {
                if let Some(index) = index {
                    self.free_channel(index, VoiceStopReason::Stopped);
                }
                continue;
            }
            let (sound, amount) = (sound.expect("checked"), amount.expect("checked"));
            if index.is_none_or(|index| self.voices[index].as_ref().is_some_and(|voice| !std::rc::Rc::ptr_eq(&voice.prepared.sound, sound))) {
                if let Some(previous) = index {
                    self.free_channel(previous, VoiceStopReason::Stopped);
                }
                self.admit_source_sound(
                    sound,
                    &SourceSoundOptions {
                        entity: -1,
                        origin: MixerVoiceOrigin::Local,
                        volume: 0.0,
                        attenuation: 0.0,
                    },
                    &ChannelCommand::Auto,
                    VoicePolicy {
                        attenuation: 0.0,
                        distance_offset: 0.0,
                        stereo_scale: 1.0,
                        unattenuated_mono: false,
                        loop_start: Some(0),
                        synchronized_gain_limit: None,
                        role: VoiceRole::Ambient,
                        key,
                    },
                    None,
                    None,
                    None,
                )?;
                index = self.voices.iter().position(|voice| voice.as_ref().is_some_and(|voice| voice.policy.is_some_and(|policy| policy.role == VoiceRole::Ambient && policy.key == key)));
            }
            let Some(index) = index else {
                return Err(AudioError::AmbientAdmission);
            };
            let Some(voice) = self.voices[index].as_ref() else {
                return Err(AudioError::AmbientAdmission);
            };
            let target = if level * amount < 8.0 { 0.0 } else { level * amount };
            let current = voice.stereo_volume.left;
            let step = 0f64.max(elapsed_seconds * fade);
            let gain = if current < target { target.min(current + step) } else { target.max(current - step) };
            if let Some(voice) = self.voices[index].as_mut() {
                voice.stereo_volume = MixerStereoVolume { left: gain, right: gain };
            }
        }
        Ok(())
    }

    /// Replace the entity-loop voices.
    pub fn set_source_loop_sounds(&mut self, entries: &[SourceLoopEntry]) -> Result<(), AudioError> {
        for index in 0..self.voices.len() {
            let matches = self.voices[index].as_ref().is_some_and(|voice| voice.policy.is_some_and(|policy| policy.role == VoiceRole::EntityLoop));
            if matches {
                self.free_channel(index, VoiceStopReason::Stopped);
            }
        }
        for entry in entries {
            let attenuation = entry.attenuation.unwrap_or(1.0);
            let q1 = entry.family == SoundFamily::Q1;
            self.admit_source_sound(
                &entry.sound,
                &SourceSoundOptions {
                    entity: entry.entity,
                    origin: MixerVoiceOrigin::Fixed { position: entry.origin },
                    volume: entry.volume,
                    attenuation,
                },
                &ChannelCommand::Auto,
                VoicePolicy {
                    attenuation: attenuation * if q1 { 0.001 } else { 0.003 },
                    distance_offset: if q1 { 0.0 } else { 80.0 },
                    stereo_scale: if q1 { 1.0 } else { 0.5 },
                    unattenuated_mono: !q1,
                    loop_start: Some(0),
                    synchronized_gain_limit: if q1 { None } else { Some(255.0) },
                    role: VoiceRole::EntityLoop,
                    key: entry.entity,
                },
                None,
                None,
                None,
            )?;
        }
        Ok(())
    }

    /// Start a Q2 synchronized sound.
    pub fn start_q2_sound(&mut self, sound: &SharedPcm, options: &Q2SoundOptions, command: &ChannelCommand, asset: Option<SoundAsset>) -> Result<bool, AudioError> {
        if !self.enabled {
            return Ok(false);
        }
        let delay = options.delay_seconds.unwrap_or(0.0);
        let server = options.server_milliseconds.unwrap_or_else(|| self.painted_time as f64 * 1000.0 / f64::from(self.output_rate)) * 0.001 * f64::from(self.output_rate);
        if !delay.is_finite() || !server.is_finite() {
            return Err(AudioError::BadSourceTimestamp);
        }
        let mut offset = self.source_begin_offset;
        let mut begin = (server + offset as f64).trunc() as i64;
        if begin < self.painted_time {
            begin = self.painted_time;
            offset = (begin as f64 - server).trunc() as i64;
        } else if begin as f64 > self.painted_time as f64 + 0.3 * f64::from(self.output_rate) {
            begin = (self.painted_time as f64 + 0.1 * f64::from(self.output_rate)).trunc() as i64;
            offset = (begin as f64 - server).trunc() as i64;
        } else {
            offset -= 10;
        }
        let scheduled = if delay == 0.0 { self.painted_time as f64 } else { (begin as f64 + delay * f64::from(self.output_rate)).trunc() };
        if scheduled.abs() >= 9_007_199_254_740_992.0 {
            return Err(AudioError::BadDeadline);
        }
        let accepted = self.admit_source_sound(
            sound,
            &SourceSoundOptions {
                entity: options.entity,
                origin: options.origin.clone(),
                volume: options.volume,
                attenuation: options.attenuation,
            },
            command,
            VoicePolicy {
                attenuation: options.attenuation * if options.attenuation == 3.0 { 0.001 } else { 0.0005 },
                distance_offset: 80.0,
                stereo_scale: 0.5,
                unattenuated_mono: true,
                loop_start: None,
                synchronized_gain_limit: None,
                role: VoiceRole::Effect,
                key: 0,
            },
            None,
            Some((scheduled as i64, self.source_schedule_order)),
            asset,
        )?;
        if accepted {
            self.source_begin_offset = offset;
            self.source_schedule_order += 1;
        }
        Ok(accepted)
    }

    /// Add or refresh a frame loop.
    pub fn update_looping_sound(&mut self, sound: &SharedPcm, options: &FrameLoopingSoundOptions) -> Result<(), AudioError> {
        if !self.enabled {
            return Ok(());
        }
        if self.sound_memory.is_none() {
            validate_sound(sound, false)?;
        }
        require_entity(options.entity, self.entity_capacity, None)?;
        require_channel_volume(options.volume.unwrap_or(127))?;
        let prepared = self.prepare(sound)?;
        if Self::prepared_output_frames(&prepared) == 0 {
            return Err(AudioError::ZeroLoop);
        }
        let previous = self.loops.get(&options.entity).cloned();
        self.entity_positions[options.entity as usize] = options.origin;
        let mut doppler = false;
        let mut doppler_scale = 1.0;
        let mut old_doppler_scale = 1.0;
        let doppler_setting = match self.sound_cvars.as_ref() {
            None => None,
            Some(cvars) => match cvars.get("s_doppler") {
                None => return Err(AudioError::MissingCvar("s_doppler".to_string())),
                Some(setting) => Some(setting.integer_value),
            },
        };
        if self.doppler_enabled && doppler_setting.is_none_or(|value| value != 0) && dot3(options.velocity, options.velocity) > 0.0 {
            doppler = true;
            let listener_position = self.position_for_entity(self.listener_entity)?;
            let before = sub3(listener_position, options.origin);
            let after = sub3(listener_position, add3(options.origin, options.velocity));
            let distance_before = dot3(before, before);
            let distance_after = dot3(after, after);
            if previous.as_ref().is_some_and(|previous| previous.frame_number.wrapping_add(1) == options.frame_number) {
                old_doppler_scale = 1.0;
            }
            doppler_scale = f64::from(distance_after / (distance_before * 100.0));
            if !doppler_scale.is_finite() {
                doppler_scale = 1.0;
            }
            if doppler_scale <= 1.0 {
                doppler = false;
            }
        }
        self.loops.insert(
            options.entity,
            LoopVoice {
                prepared,
                entity: options.entity,
                velocity: options.velocity,
                volume: f64::from(options.volume.unwrap_or(127)),
                lifetime: LoopLifetime::Frame,
                active: true,
                doppler,
                doppler_scale,
                old_doppler_scale,
                frame_number: options.frame_number,
            },
        );
        Ok(())
    }

    /// Add or refresh a persistent loop.
    pub fn update_real_looping_sound(&mut self, sound: &SharedPcm, options: &RealLoopingSoundOptions) -> Result<(), AudioError> {
        if !self.enabled {
            return Ok(());
        }
        if self.sound_memory.is_none() {
            validate_sound(sound, false)?;
        }
        require_entity(options.entity, self.entity_capacity, None)?;
        require_channel_volume(options.volume.unwrap_or(90))?;
        let prepared = self.prepare(sound)?;
        if Self::prepared_output_frames(&prepared) == 0 {
            return Err(AudioError::ZeroLoop);
        }
        let previous = self.loops.get(&options.entity).cloned();
        self.entity_positions[options.entity as usize] = options.origin;
        self.loops.insert(
            options.entity,
            LoopVoice {
                prepared,
                entity: options.entity,
                velocity: options.velocity,
                volume: f64::from(options.volume.unwrap_or(90)),
                lifetime: LoopLifetime::Persistent,
                active: true,
                doppler: false,
                doppler_scale: previous.as_ref().map_or(1.0, |previous| previous.doppler_scale),
                old_doppler_scale: previous.as_ref().map_or(1.0, |previous| previous.old_doppler_scale),
                frame_number: previous.as_ref().map_or(0, |previous| previous.frame_number),
            },
        );
        Ok(())
    }

    /// Clear frame loops (or all loops).
    pub fn clear_looping_sounds(&mut self, kill_all: bool) {
        let stale: Vec<i64> = self
            .loops
            .iter()
            .filter(|(_, loop_voice)| kill_all || loop_voice.lifetime == LoopLifetime::Frame || Self::prepared_output_frames(&loop_voice.prepared) == 0)
            .map(|(entity, _)| *entity)
            .collect();
        for entity in stale {
            if let Some(loop_voice) = self.loops.get_mut(&entity) {
                loop_voice.active = false;
            }
        }
        self.loop_channels.clear();
    }

    /// Stop one entity's loop.
    pub fn stop_looping_sound(&mut self, entity: i64) -> Result<(), AudioError> {
        require_entity(entity, self.entity_capacity, None)?;
        if let Some(loop_voice) = self.loops.get_mut(&entity) {
            loop_voice.active = false;
        }
        Ok(())
    }

    /// Stop an entity's Q3 channel.
    pub fn stop_channel(&mut self, entity: i64, channel: i32) -> Result<(), AudioError> {
        let command = source_sound_channel(SoundFamily::Q3, channel).map_err(|_| AudioError::BadSourceChannel)?;
        self.replace_channel(entity, &command, false, VoiceStopReason::Stopped);
        Ok(())
    }

    /// Stop an entity's shared channel.
    pub fn stop_shared_channel(&mut self, entity: i64, channel: Option<SoundChannel>) {
        let Some(channel) = channel else {
            for index in 0..self.voices.len() {
                let matches = self.voices[index].as_ref().is_some_and(|voice| {
                    voice.entity == entity && voice.channel.is_none() && voice.policy.is_none_or(|policy| policy.role == VoiceRole::Effect)
                });
                if matches {
                    self.free_channel(index, VoiceStopReason::Stopped);
                    return;
                }
            }
            return;
        };
        self.replace_channel(entity, &ChannelCommand::Channel(channel), true, VoiceStopReason::Stopped);
    }

    fn replace_channel(&mut self, entity: i64, command: &ChannelCommand, cancel_scheduled: bool, reason: VoiceStopReason) {
        for index in 0..self.voices.len() {
            let matches = self.voices[index].as_ref().is_some_and(|voice| {
                (cancel_scheduled || !matches!(voice.start, VoiceStart::Scheduled { .. }))
                    && voice.entity == entity
                    && voice.policy.is_none_or(|policy| policy.role == VoiceRole::Effect)
                    && match command {
                        ChannelCommand::ReplaceActor => true,
                        ChannelCommand::Channel(want) => voice.channel == Some(*want),
                        ChannelCommand::Auto => false,
                    }
            });
            if matches {
                self.free_channel(index, reason);
                if *command == ChannelCommand::ReplaceActor {
                    return;
                }
            }
        }
    }

    /// Stop an entity's voices and loop.
    pub fn stop_entity(&mut self, entity: i64) -> Result<(), AudioError> {
        require_entity(entity, self.entity_capacity, None)?;
        for index in 0..self.voices.len() {
            if self.voices[index].as_ref().is_some_and(|voice| voice.entity == entity) {
                self.free_channel(index, VoiceStopReason::Stopped);
            }
        }
        self.stop_looping_sound(entity)
    }

    /// Stop everything.
    pub fn stop_all(&mut self) -> Result<(), AudioError> {
        self.reset_channels()?;
        self.source_begin_offset = 0;
        self.source_schedule_order = 0;
        self.loops.clear();
        self.loop_channels.clear();
        self.entity_positions.fill(vec3(0.0, 0.0, 0.0));
        self.clear_raw();
        Ok(())
    }

    /// Clear loops and channels without moving paint time.
    pub fn clear_sound_buffer(&mut self) -> Result<(), AudioError> {
        self.loops.clear();
        self.loop_channels.clear();
        self.entity_positions.fill(vec3(0.0, 0.0, 0.0));
        self.reset_channels()?;
        self.raw_end_time = 0;
        Ok(())
    }

    /// Clear the raw stream.
    pub fn clear_raw(&mut self) {
        self.raw_end_time = self.painted_time;
    }

    /// Reset raw end to sound time.
    pub fn reset_raw_to_sound_time(&mut self) {
        self.raw_end_time = self.sound_time;
    }

    /// Stop raw playback.
    pub fn stop_raw(&mut self) {
        self.raw_end_time = 0;
    }

    /// Queue decoded PCM through the raw path.
    pub fn queue_raw(&mut self, sound: &PcmSound, volume: f64) -> Result<(), AudioError> {
        if !self.enabled {
            return Ok(());
        }
        validate_sound(sound, true)?;
        require_gain(volume, "raw volume")?;
        let integer_volume = ((volume as f32) * 256.0).trunc();
        if !integer_volume.is_finite() || f64::from(integer_volume) < -2_147_483_648.0 || f64::from(integer_volume) > 2_147_483_647.0 {
            return Err(AudioError::BadRawConversion);
        }
        self.begin_raw_write()?;
        let output_frames = self.raw_output_frames(sound)?;
        self.append_raw(sound, integer_volume as i32, output_frames)?;
        if self.raw_end_time > self.sound_time + RAW_SAMPLE_CAPACITY as i64 {
            self.raw_debug_print(&format!("S_RawSamples: overflowed {} > {}\n", self.raw_end_time, self.sound_time))?;
        }
        Ok(())
    }

    /// Queue raw bytes (`S_RawSamples`).
    pub fn queue_raw_bytes(&mut self, samples: i32, rate: i32, width: i32, channels: i32, data: &[u8], volume: f64) -> Result<(), AudioError> {
        if !self.enabled {
            return Ok(());
        }
        if rate <= 0 {
            return Err(AudioError::BadRawShape);
        }
        let mut integer_volume = ((volume as f32) * 256.0).trunc();
        if !integer_volume.is_finite() || f64::from(integer_volume) < -2_147_483_648.0 || f64::from(integer_volume) > 2_147_483_647.0 {
            return Err(AudioError::BadRawConversion);
        }
        self.begin_raw_write()?;
        if (channels == 1 || channels == 2) && (width == 1 || width == 2) {
            let scale = (rate as f32) / (self.output_rate as f32);
            if width == 1 {
                integer_volume = (integer_volume as i32).wrapping_mul(256) as f32;
            }
            let integer_volume = integer_volume as i32;
            let mut index = 0i64;
            loop {
                let source = if channels == 2 && width == 2 && scale == 1.0 {
                    index
                } else {
                    ((index as f64 * f64::from(scale)) as f32).trunc() as i64
                };
                if source >= i64::from(samples) {
                    break;
                }
                let destination = (self.raw_end_time & (RAW_SAMPLE_CAPACITY as i64 - 1)) as usize;
                self.raw_end_time += 1;
                let source_index = source * i64::from(channels);
                let sample = |at: i64| -> Result<i32, AudioError> {
                    if width == 2 {
                        let offset = at * 2;
                        if offset < 0 || offset + 1 >= data.len() as i64 {
                            return Err(AudioError::RawAccess(offset as usize));
                        }
                        let offset = offset as usize;
                        Ok(i32::from(i16::from_le_bytes([data[offset], data[offset + 1]])))
                    } else if channels == 2 {
                        let byte = data.get(at as usize).copied().ok_or(AudioError::RawAccess(at as usize))?;
                        Ok(i32::from(byte as i8))
                    } else {
                        let byte = data.get(at as usize).copied().ok_or(AudioError::RawAccess(at as usize))?;
                        Ok(i32::from(byte) - 128)
                    }
                };
                self.raw_samples[destination * 2] = sample(source_index)?.wrapping_mul(integer_volume);
                self.raw_samples[destination * 2 + 1] = sample(source_index + i64::from(channels) - 1)?.wrapping_mul(integer_volume);
                index += 1;
            }
        }
        if self.raw_end_time > self.sound_time + RAW_SAMPLE_CAPACITY as i64 {
            self.raw_debug_print(&format!("S_RawSamples: overflowed {} > {}\n", self.raw_end_time, self.sound_time))?;
        }
        Ok(())
    }

    fn raw_output_frames(&self, sound: &PcmSound) -> Result<i64, AudioError> {
        let scale = (sound.sample_rate as f32) / (self.output_rate as f32);
        if sound.channels == 2 && scale == 1.0 {
            return Ok(sound.frame_count as i64);
        }
        let estimate = (sound.frame_count as f64 / f64::from(scale)).ceil();
        if !estimate.is_finite() || estimate.abs() >= 9_007_199_254_740_992.0 {
            return Err(AudioError::BadRawFrames);
        }
        let mut output_frames = estimate as i64;
        while output_frames > 0 && (((output_frames - 1) as f64 * f64::from(scale)) as f32).trunc() as i64 >= sound.frame_count as i64 {
            output_frames -= 1;
        }
        while ((((output_frames as f64) * f64::from(scale)) as f32).trunc() as i64) < sound.frame_count as i64 {
            output_frames += 1;
            if output_frames.abs() >= 9_007_199_254_740_992 {
                return Err(AudioError::BadRawFrames);
            }
        }
        Ok(output_frames)
    }

    fn begin_raw_write(&mut self) -> Result<(), AudioError> {
        if self.raw_end_time < self.sound_time {
            self.raw_debug_print(&format!("S_RawSamples: resetting minimum: {} < {}\n", self.raw_end_time, self.sound_time))?;
            self.raw_end_time = self.sound_time;
        }
        Ok(())
    }

    fn raw_debug_print(&mut self, text: &str) -> Result<(), AudioError> {
        let developer = self.sound_cvars.as_ref().and_then(|cvars| cvars.get("developer"));
        if developer.is_none_or(|setting| setting.integer_value == 0) {
            return Ok(());
        }
        let Some(console) = self.console_output.as_mut() else {
            return Err(AudioError::RawDiagnostics);
        };
        console(text);
        Ok(())
    }

    fn append_raw(&mut self, sound: &PcmSound, integer_volume: i32, count: i64) -> Result<(), AudioError> {
        if self.raw_end_time.checked_add(count).is_none() {
            return Err(AudioError::BadRawEnd);
        }
        let scale = (sound.sample_rate as f32) / (self.output_rate as f32);
        for offset in 0..count.max(0) {
            let source_frame = if sound.channels == 2 && scale == 1.0 {
                offset
            } else {
                ((offset as f64 * f64::from(scale)) as f32).trunc() as i64
            };
            let destination = (self.raw_end_time & (RAW_SAMPLE_CAPACITY as i64 - 1)) as usize;
            self.raw_end_time += 1;
            let source_index = source_frame * i64::from(sound.channels);
            let left = checked_sample(&sound.samples, source_index as usize)?;
            let right = if sound.channels == 1 {
                left
            } else {
                checked_sample(&sound.samples, (source_index + 1) as usize)?
            };
            self.raw_samples[destination * 2] = left.wrapping_mul(integer_volume);
            self.raw_samples[destination * 2 + 1] = right.wrapping_mul(integer_volume);
        }
        Ok(())
    }

    /// Queue music at the music volume.
    pub fn queue_music(&mut self, sound: &PcmSound) -> Result<(), AudioError> {
        let volume = self.music_volume;
        self.queue_raw(sound, volume)
    }

    /// Prepare a sound for painting, borrowing bank memory when bound.
    fn prepare(&mut self, sound: &SharedPcm) -> Result<PreparedSound, AudioError> {
        if let Some(memory) = self.sound_memory.as_ref() {
            let output_frames = memory.borrow().frame_count(sound);
            return Ok(PreparedSound {
                doppler_sums: None,
                sound: sound.clone(),
                step256: 256.0,
                memory: Some(memory.clone()),
                output_frames,
            });
        }
        let scale = (sound.sample_rate as f32) / (self.output_rate as f32);
        let output_frames = ((sound.frame_count as f32) / scale).trunc();
        if !output_frames.is_finite() || output_frames < 0.0 || output_frames > i64::MAX as f32 {
            return Err(AudioError::BadResampleFrames);
        }
        Ok(PreparedSound {
            doppler_sums: None,
            sound: sound.clone(),
            step256: f64::from((scale * 256.0).trunc()),
            memory: None,
            output_frames: output_frames as usize,
        })
    }

    /// Live output frame count (bank-backed sounds read the bank).
    fn prepared_output_frames(prepared: &PreparedSound) -> usize {
        prepared.memory.as_ref().map_or(prepared.output_frames, |memory| memory.borrow().frame_count(&prepared.sound))
    }

    /// Read the signed-int allocation clock.
    fn allocation_time(&self) -> Result<i64, AudioError> {
        let time = (self.milliseconds)();
        if time < i64::from(i32::MIN) || time > i64::from(i32::MAX) {
            return Err(AudioError::BadAllocationClock);
        }
        Ok(time)
    }

    /// Free every channel and rebuild the free list.
    fn reset_channels(&mut self) -> Result<(), AudioError> {
        for index in 0..self.voices.len() {
            if self.voices[index].is_some() {
                self.voice_stopped(index, VoiceStopReason::Stopped, None);
            }
        }
        self.voices.fill(None);
        self.free_channels.clear();
        self.free_channels.extend(0..self.voices.len());
        self.raw_debug_print("Channel memory manager started\n")
    }

    /// Free one channel slot.
    fn free_channel(&mut self, index: usize, reason: VoiceStopReason) {
        if self.voices[index].is_some() {
            self.voice_stopped(index, reason, None);
        }
        self.voices[index] = None;
        self.free_channels.push(index);
    }

    /// Read an integer sound diagnostic, defaulting to zero when unbound.
    fn diagnostic_setting(&self, name: &str) -> Result<i32, AudioError> {
        let Some(cvars) = self.sound_cvars.as_ref() else {
            return Ok(0);
        };
        cvars
            .get(name)
            .map(|cvar| cvar.integer_value)
            .ok_or_else(|| AudioError::MissingCvar(name.to_string()))
    }

    /// Read an entity position cell.
    fn position_for_entity(&self, entity: i64) -> Result<Vec3, AudioError> {
        if entity < 0 {
            return Err(AudioError::MissingEntityPosition(entity));
        }
        self.entity_positions
            .get(entity as usize)
            .copied()
            .ok_or(AudioError::MissingEntityPosition(entity))
    }

    /// Resolve a voice origin to a position.
    fn resolve_origin(&self, origin: &MixerVoiceOrigin) -> Result<Vec3, AudioError> {
        match origin {
            MixerVoiceOrigin::Local => Ok(self.listener_origin),
            MixerVoiceOrigin::Fixed { position } => Ok(*position),
            MixerVoiceOrigin::Entity { entity } => {
                require_entity(*entity, self.entity_capacity, None)?;
                self.position_for_entity(*entity)
            }
        }
    }

    /// Spatialize a policy-less voice.
    fn spatialize(&mut self, entity: i64, origin: &MixerVoiceOrigin, volume: f64) -> Result<MixerStereoVolume, AudioError> {
        if matches!(origin, MixerVoiceOrigin::Local) || entity == self.listener_entity {
            return Ok(MixerStereoVolume {
                left: volume,
                right: volume,
            });
        }
        let position = self.resolve_origin(origin)?;
        self.spatialize_origin(position, volume)
    }

    /// Spatialize a position with geometry transmission.
    fn spatialize_origin(&mut self, position: Vec3, volume: f64) -> Result<MixerStereoVolume, AudioError> {
        let channels = if self.output_channels == 1 {
            OutputChannels::Mono
        } else {
            OutputChannels::Stereo
        };
        let volume = spatialize_sound_origin(position, self.listener_origin, self.listener_axis, volume as f32, channels);
        Ok(self.transmit(
            position,
            MixerStereoVolume {
                left: f64::from(volume.left),
                right: f64::from(volume.right),
            },
        ))
    }

    /// Start pending voices and retire ended ones (`S_ScanChannelStarts`).
    pub fn scan_channel_starts(&mut self) -> bool {
        let mut new_samples = false;
        for index in 0..self.voices.len() {
            let action = match &self.voices[index] {
                None => None,
                Some(voice) => match voice.start {
                    VoiceStart::Pending => Some(false),
                    VoiceStart::Started { sample } if voice.policy.is_none_or(|policy| policy.loop_start.is_none()) && sample + voice.prepared.output_frames as i64 <= self.painted_time => {
                        Some(true)
                    }
                    _ => None,
                },
            };
            match action {
                Some(false) => {
                    if let Some(voice) = self.voices[index].as_mut() {
                        voice.start = VoiceStart::Started {
                            sample: self.painted_time,
                        };
                    }
                    self.voice_started(index);
                    new_samples = true;
                }
                Some(true) => {
                    let end = match &self.voices[index] {
                        Some(voice) => match voice.start {
                            VoiceStart::Started { sample } => sample + voice.prepared.output_frames as i64,
                            _ => self.painted_time,
                        },
                        None => self.painted_time,
                    };
                    self.voice_stopped(index, VoiceStopReason::Ended, Some(end));
                    self.free_channel(index, VoiceStopReason::Ended);
                }
                None => {}
            }
        }
        new_samples
    }

    /// Start scheduled voices whose deadline has passed.
    fn issue_scheduled_sounds(&mut self) -> Result<(), AudioError> {
        let mut due: Vec<(usize, i64, i64)> = Vec::new();
        for (index, voice) in self.voices.iter().enumerate() {
            if let Some(voice) = voice {
                if let VoiceStart::Scheduled { sample, order } = voice.start {
                    if sample <= self.painted_time {
                        due.push((index, sample, order));
                    }
                }
            }
        }
        due.sort_by(|left, right| left.1.cmp(&right.1).then(right.2.cmp(&left.2)));
        for (index, _, _) in due {
            let Some(voice) = self.voices[index].clone() else {
                continue;
            };
            if !matches!(voice.start, VoiceStart::Scheduled { .. }) {
                continue;
            }
            if let Some(channel) = voice.channel {
                self.replace_channel(voice.entity, &ChannelCommand::Channel(channel), false, VoiceStopReason::Replaced);
            }
            let stereo = self.policy_spatialize(&voice)?;
            if let Some(slot) = self.voices[index].as_mut() {
                slot.start = VoiceStart::Started {
                    sample: self.painted_time,
                };
                slot.stereo_volume = stereo;
            }
            self.voice_started(index);
        }
        Ok(())
    }

    /// Sample one output frame of a prepared sound.
    fn effect_sample(memory: &Option<SharedMixerMemory>, prepared: &PreparedSound, output_frame: usize) -> Result<i32, AudioError> {
        if let Some(memory) = memory {
            return Ok(memory.borrow().sample(&prepared.sound, output_frame));
        }
        let source_frame = (output_frame as f64 * prepared.step256 / 256.0).trunc() as usize;
        checked_sample(&prepared.sound.samples, source_frame)
    }

    /// Paint one effect sample into a stereo paint buffer.
    fn paint_effect(paint: &mut [f64], output_frame: usize, sample: i32, volume: MixerStereoVolume, effects_gain: f64) -> Result<(), AudioError> {
        let left_gain = volume.left * effects_gain;
        let right_gain = volume.right * effects_gain;
        let left_index = output_frame * 2;
        let right_index = left_index + 1;
        if right_index >= paint.len() {
            return Err(AudioError::BadPaintIndex {
                index: right_index.to_string(),
                length: paint.len().to_string(),
            });
        }
        paint[left_index] += (f64::from(sample) * left_gain / 256.0).floor();
        paint[right_index] += (f64::from(sample) * right_gain / 256.0).floor();
        Ok(())
    }

    /// Sample a Doppler loop chunk, stabilizing overrun tails as zero.
    fn doppler_sample(memory: &Option<SharedMixerMemory>, prepared: &PreparedSound, chunk: i64, sample_offset: i64) -> Result<i32, AudioError> {
        let output_frame = chunk * SND_CHUNK_SIZE as i64 + (sample_offset & (SND_CHUNK_SIZE as i64 - 1));
        if let Some(memory) = memory {
            let frame = usize::try_from(output_frame).map_err(|_| AudioError::NegativeLoopAccess)?;
            return Ok(memory.borrow().sample(&prepared.sound, frame));
        }
        if output_frame < 0 {
            return Err(AudioError::NegativeLoopAccess);
        }
        if output_frame >= prepared.output_frames as i64 {
            return Ok(0);
        }
        Self::effect_sample(memory, prepared, output_frame as usize)
    }

    /// Paint a loop mix across a paint block.
    fn paint_loop(
        paint: &mut [f64],
        memory: &Option<SharedMixerMemory>,
        doppler_enabled: bool,
        painted_time: i64,
        frames: i64,
        loop_mix: &mut LoopMix,
        effects_gain: f64,
    ) -> Result<(), AudioError> {
        let mut output_frame = 0i64;
        while output_frame < frames {
            let output_frames = loop_mix.prepared.output_frames as i64;
            if output_frames <= 0 {
                return Ok(());
            }
            let sample_offset = (painted_time + output_frame) % output_frames;
            let count = (frames - output_frame).min(output_frames - sample_offset);
            if !doppler_enabled || !loop_mix.doppler || loop_mix.doppler_scale == 1.0 {
                for index in 0..count {
                    if sample_offset + index < 0 {
                        return Err(AudioError::NegativeLoopAccess);
                    }
                    let sample = Self::effect_sample(memory, &loop_mix.prepared, (sample_offset + index) as usize)?;
                    Self::paint_effect(
                        paint,
                        (output_frame + index) as usize,
                        sample,
                        MixerStereoVolume {
                            left: loop_mix.left_volume,
                            right: loop_mix.right_volume,
                        },
                        effects_gain,
                    )?;
                }
            } else {
                Self::paint_doppler_loop(paint, memory, output_frame, count, sample_offset, loop_mix, effects_gain)?;
            }
            output_frame += count;
        }
        Ok(())
    }

    /// Paint a Doppler-scaled loop span.
    fn paint_doppler_loop(
        paint: &mut [f64],
        memory: &Option<SharedMixerMemory>,
        output_frame: i64,
        count: i64,
        source_offset: i64,
        loop_mix: &mut LoopMix,
        effects_gain: f64,
    ) -> Result<(), AudioError> {
        if loop_mix.doppler_scale > SND_CHUNK_SIZE as f64 {
            return Self::paint_wide_doppler_loop(paint, memory, output_frame, count, source_offset, loop_mix, effects_gain);
        }
        let output_frames = loop_mix.prepared.output_frames as i64;
        let scaled_offset = ((source_offset as f32) * loop_mix.old_doppler_scale as f32).trunc() as i64;
        let chunk_count = ((output_frames + SND_CHUNK_SIZE as i64 - 1) / SND_CHUNK_SIZE as i64).max(1);
        let mut chunk = if scaled_offset < 0 {
            0
        } else {
            (scaled_offset / SND_CHUNK_SIZE as i64) % chunk_count
        };
        let mut offset = (if scaled_offset < 0 { scaled_offset } else { scaled_offset % SND_CHUNK_SIZE as i64 }) as f32;
        let left_volume = loop_mix.left_volume as f32 * effects_gain as f32;
        let right_volume = loop_mix.right_volume as f32 * effects_gain as f32;
        for index in 0..count {
            let first = offset.trunc() as i64;
            offset += loop_mix.doppler_scale as f32;
            let last = offset.trunc() as i64;
            let mut sample_total = 0f32;
            for source in first..last {
                if source == SND_CHUNK_SIZE as i64 {
                    chunk = (chunk + 1) % chunk_count;
                    offset -= SND_CHUNK_SIZE as f32;
                }
                sample_total += Self::doppler_sample(memory, &loop_mix.prepared, chunk, source)? as f32;
            }
            let divisor = 256.0f32 * (last - first) as f32;
            let left_contribution = (sample_total * left_volume) / divisor;
            let right_contribution = (sample_total * right_volume) / divisor;
            Self::add_float_paint(paint, ((output_frame + index) * 2) as usize, f64::from(left_contribution))?;
            Self::add_float_paint(paint, ((output_frame + index) * 2 + 1) as usize, f64::from(right_contribution))?;
        }
        Ok(())
    }

    /// Paint a wide Doppler span with periodic range sums.
    fn paint_wide_doppler_loop(
        paint: &mut [f64],
        memory: &Option<SharedMixerMemory>,
        output_frame: i64,
        count: i64,
        source_offset: i64,
        loop_mix: &mut LoopMix,
        effects_gain: f64,
    ) -> Result<(), AudioError> {
        let output_frames = loop_mix.prepared.output_frames;
        let period = output_frames.div_ceil(SND_CHUNK_SIZE) * SND_CHUNK_SIZE;
        if loop_mix.prepared.doppler_sums.as_ref().is_none_or(|sums| sums.len() != period + 1) {
            let mut sums = vec![0.0f64; period + 1];
            let mut total = 0.0f64;
            for frame in 0..period {
                if frame < output_frames {
                    total += f64::from(Self::effect_sample(memory, &loop_mix.prepared, frame)?);
                }
                sums[frame + 1] = total;
            }
            loop_mix.prepared.doppler_sums = Some(sums);
        }
        let sums = loop_mix.prepared.doppler_sums.as_ref().expect("cached");
        let sum = |index: i64| -> Result<f64, AudioError> {
            if index < 0 {
                return Err(AudioError::DopplerRange);
            }
            sums.get(index as usize).copied().ok_or(AudioError::DopplerRange)
        };
        let period_float = period as f64;
        let cycles = (loop_mix.doppler_scale / period_float).floor();
        let remainder = loop_mix.doppler_scale % period_float;
        let cycle_samples = cycles * period_float;
        let cycle_total = cycles * sum(period as i64)?;
        let mut offset = (source_offset % period as i64) as f64;
        for index in 0..count {
            let end = offset + remainder;
            let first = offset.trunc() as i64;
            let last = end.trunc() as i64;
            let tail = sum(last.min(period as i64))? - sum(first)? + if last > period as i64 { sum(last - period as i64)? } else { 0.0 };
            let average = (cycle_total + tail) / (cycle_samples + (last - first) as f64);
            Self::add_float_paint(
                paint,
                ((output_frame + index) * 2) as usize,
                average * loop_mix.left_volume * effects_gain / 256.0,
            )?;
            Self::add_float_paint(
                paint,
                ((output_frame + index) * 2 + 1) as usize,
                average * loop_mix.right_volume * effects_gain / 256.0,
            )?;
            offset = end % period_float;
        }
        Ok(())
    }

    /// Add a truncated float contribution to a paint cell.
    fn add_float_paint(paint: &mut [f64], index: usize, contribution: f64) -> Result<(), AudioError> {
        let length = paint.len();
        let cell = paint.get_mut(index).ok_or_else(|| AudioError::BadPaintIndex {
            index: index.to_string(),
            length: length.to_string(),
        })?;
        *cell += contribution.trunc();
        Ok(())
    }

    /// Collect active loops into merged mixes by entity order.
    fn collect_loop_mixes(&mut self) -> Result<Vec<LoopMix>, AudioError> {
        self.loop_channels.clear();
        let time = if self.sound_memory.is_some() {
            Some(self.allocation_time()?)
        } else {
            None
        };
        let mut loops: Vec<LoopVoice> = self.loops.values().filter(|loop_voice| loop_voice.active).cloned().collect();
        loops.sort_by_key(|loop_voice| loop_voice.entity);
        let mut merged: HashSet<i64> = HashSet::new();
        let mut mixes: Vec<LoopMix> = Vec::new();
        for index in 0..loops.len() {
            if merged.contains(&loops[index].entity) {
                continue;
            }
            let position = self.position_for_entity(loops[index].entity)?;
            let volume = self.spatialize_origin(position, loops[index].volume)?;
            if let (Some(time), Some(memory)) = (time, self.sound_memory.as_ref()) {
                memory.borrow_mut().touch(&loops[index].prepared.sound, time);
            }
            let mut left_volume = volume.left;
            let mut right_volume = volume.right;
            for candidate in &loops[index + 1..] {
                if candidate.doppler || !Rc::ptr_eq(&candidate.prepared.sound, &loops[index].prepared.sound) {
                    continue;
                }
                merged.insert(candidate.entity);
                let position = self.position_for_entity(candidate.entity)?;
                let volume = self.spatialize_origin(position, candidate.volume)?;
                if let (Some(time), Some(memory)) = (time, self.sound_memory.as_ref()) {
                    memory.borrow_mut().touch(&candidate.prepared.sound, time);
                }
                left_volume += volume.left;
                right_volume += volume.right;
            }
            if left_volume == 0.0 && right_volume == 0.0 {
                continue;
            }
            mixes.push(LoopMix {
                prepared: loops[index].prepared.clone(),
                left_volume: left_volume.min(255.0),
                right_volume: right_volume.min(255.0),
                doppler: loops[index].doppler,
                doppler_scale: loops[index].doppler_scale,
                old_doppler_scale: loops[index].old_doppler_scale,
            });
        }
        Ok(mixes)
    }

    /// Paint the raw stream into a paint block (replacing covered frames).
    fn paint_raw(&self, paint: &mut [f64], frames: i64) -> Result<(), AudioError> {
        let stop = (self.painted_time + frames).min(self.raw_end_time);
        let mut absolute = self.painted_time;
        while absolute < stop {
            let output_frame = (absolute - self.painted_time) as usize;
            let raw_index = (absolute & (RAW_SAMPLE_CAPACITY as i64 - 1)) as usize;
            let left = self.raw_samples.get(raw_index * 2).copied().ok_or(AudioError::BadRawIndex {
                index: (raw_index * 2).to_string(),
                length: self.raw_samples.len().to_string(),
            })?;
            let right = self.raw_samples.get(raw_index * 2 + 1).copied().ok_or(AudioError::BadRawIndex {
                index: (raw_index * 2 + 1).to_string(),
                length: self.raw_samples.len().to_string(),
            })?;
            let Some(left_cell) = paint.get_mut(output_frame * 2) else {
                return Err(AudioError::BadPaintIndex {
                    index: (output_frame * 2).to_string(),
                    length: paint.len().to_string(),
                });
            };
            *left_cell = f64::from(left);
            let Some(right_cell) = paint.get_mut(output_frame * 2 + 1) else {
                return Err(AudioError::BadPaintIndex {
                    index: (output_frame * 2 + 1).to_string(),
                    length: paint.len().to_string(),
                });
            };
            *right_cell = f64::from(right);
            absolute += 1;
        }
        Ok(())
    }

    /// Mix frames, consuming the paint clock or painting an explicit range.
    pub fn mix(&mut self, request: MixRequest) -> Result<Vec<i16>, AudioError> {
        let (start_frame, frames) = match request {
            MixRequest::Consume(frames) => (self.painted_time, frames),
            MixRequest::Range(range) => (range.start_frame, range.end_frame.checked_sub(range.start_frame).ok_or(AudioError::BadMixFrames)?),
        };
        if frames < 0 {
            return Err(AudioError::BadMixFrames);
        }
        let sample_count = frames.checked_mul(2).ok_or(AudioError::MixOutputTooLarge)?;
        let end_frame = start_frame.checked_add(frames).ok_or(AudioError::BadPaintEnd)?;
        if matches!(request, MixRequest::Consume(_)) && end_frame < self.sound_time {
            return Err(AudioError::MixRewind);
        }
        let mut output = vec![0i16; sample_count as usize];
        self.painted_time = start_frame;
        self.scan_channel_starts();
        let effects_gain = f64::from((self.effects_volume * 255.0).trunc());
        while self.painted_time < end_frame {
            self.issue_scheduled_sounds()?;
            let mut count = (PAINTBUFFER_SIZE as i64).min(end_frame - self.painted_time);
            for voice in self.voices.iter().flatten() {
                if let VoiceStart::Scheduled { sample, .. } = voice.start {
                    count = count.min(sample - self.painted_time);
                }
            }
            let mut paint = vec![0.0f64; (count * 2) as usize];
            self.paint_raw(&mut paint, count)?;
            let voices = self.voices.clone();
            let mut merged_voices: HashSet<usize> = HashSet::new();
            for (index, voice) in voices.iter().enumerate() {
                let Some(voice) = voice else {
                    continue;
                };
                if matches!(voice.start, VoiceStart::Scheduled { .. }) || merged_voices.contains(&index) {
                    continue;
                }
                if !matches!(voice.start, VoiceStart::Started { .. }) {
                    return Err(AudioError::PendingSound);
                }
                let mut stereo = voice.stereo_volume;
                if let Some(limit) = voice.policy.and_then(|policy| policy.synchronized_gain_limit) {
                    let mut left = stereo.left;
                    let mut right = stereo.right;
                    merged_voices.insert(index);
                    for (candidate_index, candidate) in voices.iter().enumerate() {
                        let Some(candidate) = candidate else {
                            continue;
                        };
                        if merged_voices.contains(&candidate_index) || !matches!(candidate.start, VoiceStart::Started { .. }) {
                            continue;
                        }
                        if candidate.policy.and_then(|policy| policy.synchronized_gain_limit) != Some(limit) {
                            continue;
                        }
                        if !Rc::ptr_eq(&candidate.prepared.sound, &voice.prepared.sound) {
                            continue;
                        }
                        merged_voices.insert(candidate_index);
                        left += candidate.stereo_volume.left;
                        right += candidate.stereo_volume.right;
                    }
                    stereo = MixerStereoVolume {
                        left: left.min(limit),
                        right: right.min(limit),
                    };
                }
                if stereo.left == 0.0 && stereo.right == 0.0 {
                    continue;
                }
                let VoiceStart::Started { sample: start_sample } = voice.start else {
                    return Err(AudioError::PendingSound);
                };
                let first_offset = self.painted_time - start_sample;
                for output_frame in 0..count {
                    let mut sound_frame = first_offset + output_frame;
                    if let Some(loop_start) = voice.policy.and_then(|policy| policy.loop_start) {
                        let output_frames = voice.prepared.output_frames as i64;
                        if sound_frame >= output_frames {
                            sound_frame = loop_start as i64 + (sound_frame - output_frames) % (output_frames - loop_start as i64);
                        }
                    }
                    if sound_frame < 0 || sound_frame >= voice.prepared.output_frames as i64 {
                        continue;
                    }
                    let sample = Self::effect_sample(&self.sound_memory, &voice.prepared, sound_frame as usize)?;
                    Self::paint_effect(&mut paint, output_frame as usize, sample, stereo, effects_gain)?;
                }
            }
            for loop_mix in &mut self.loop_channels {
                let skip = self.sound_memory.as_ref().is_some_and(|memory| {
                    !memory.borrow().has_data(&loop_mix.prepared.sound) || memory.borrow().frame_count(&loop_mix.prepared.sound) == 0
                });
                if skip {
                    continue;
                }
                let memory = self.sound_memory.clone();
                let painted_time = self.painted_time;
                let doppler_enabled = self.doppler_enabled;
                Self::paint_loop(&mut paint, &memory, doppler_enabled, painted_time, count, loop_mix, effects_gain)?;
            }
            if self.diagnostic_setting("s_testsound")? != 0 {
                for frame in 0..count {
                    let sample = (((self.painted_time + frame) as f64 * 0.1).sin() * 20000.0 * 256.0).trunc();
                    paint[(frame * 2) as usize] = sample;
                    paint[(frame * 2 + 1) as usize] = sample;
                }
            }
            let output_offset = ((self.painted_time - start_frame) * 2) as usize;
            write_linear_blast_stereo16_float(&paint, &mut output[output_offset..], paint.len())?;
            self.painted_time += count;
            for index in 0..self.voices.len() {
                let ended = matches!(&self.voices[index], Some(voice) if matches!(voice.start, VoiceStart::Started { sample } if (voice.policy.is_none() || voice.policy.is_some_and(|policy| policy.loop_start.is_none())) && sample + voice.prepared.output_frames as i64 <= self.painted_time));
                if ended {
                    let end = match &self.voices[index] {
                        Some(voice) => match voice.start {
                            VoiceStart::Started { sample } => sample + voice.prepared.output_frames as i64,
                            _ => self.painted_time,
                        },
                        None => self.painted_time,
                    };
                    self.voice_stopped(index, VoiceStopReason::Ended, Some(end));
                }
            }
        }
        if matches!(request, MixRequest::Consume(_)) {
            self.sound_time = end_frame;
        }
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use qa_core::math::{vec3, Axis};

    use super::*;

    fn axis() -> Axis {
        [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)]
    }

    fn blip(frames: usize, loop_start: Option<usize>) -> SharedPcm {
        Rc::new(PcmSound {
            sample_rate: 44100,
            channels: 1,
            samples: vec![1000i16; frames],
            frame_count: frames,
            loop_start,
        })
    }

    fn mixer() -> AudioMixer {
        AudioMixer::new(44100, Box::new(|| 100), 96, 2, 1024).unwrap()
    }

    #[test]
    fn starts_and_paints_q3() {
        let mut mixer = mixer();
        let sound = blip(4410, None);
        assert!(mixer
            .start_sound(
                &sound,
                &StartSoundOptions {
                    entity: 0,
                    channel: 0,
                    origin: MixerVoiceOrigin::Entity { entity: 0 },
                    volume: 127,
                },
                Some("blip"),
            )
            .unwrap());
        let mixed = mixer.mix(MixRequest::Consume(8)).unwrap();
        assert_eq!(mixed.len(), 16);
        assert!(mixed.iter().any(|sample| *sample != 0));
        assert_eq!(mixer.sound_clock(), 8);
        assert_eq!(mixer.sample_clock(), 8);
        assert_eq!(mixer.channel_volumes().len(), 1);
    }

    #[test]
    fn paints_ranges_without_advancing_sound_time() {
        let mut mixer = mixer();
        let sound = blip(4410, None);
        mixer
            .start_sound(
                &sound,
                &StartSoundOptions {
                    entity: 0,
                    channel: 0,
                    origin: MixerVoiceOrigin::Entity { entity: 0 },
                    volume: 127,
                },
                None,
            )
            .unwrap();
        let ranged = mixer
            .mix(MixRequest::Range(SoundPaintRange {
                start_frame: 0,
                end_frame: 8,
            }))
            .unwrap();
        assert!(ranged.iter().any(|sample| *sample != 0));
        assert_eq!(mixer.sound_clock(), 0);
        assert_eq!(mixer.sample_clock(), 8);
        assert!(matches!(mixer.mix(MixRequest::Consume(-1)), Err(AudioError::BadMixFrames)));
    }

    #[test]
    fn jitters_q1_and_drops_silence() {
        let mut mixer = mixer();
        let sound = blip(4410, None);
        let options = SourceSoundOptions {
            entity: 0,
            origin: MixerVoiceOrigin::Local,
            volume: 1.0,
            attenuation: 0.0,
        };
        let mut random = || 3i64;
        assert!(mixer.start_q1_sound(&sound, &options, &ChannelCommand::Auto, Some(&mut random), None).unwrap());
        assert!(mixer.start_q1_sound(&sound, &options, &ChannelCommand::Auto, Some(&mut random), None).unwrap());
        let far = SourceSoundOptions {
            entity: 5,
            origin: MixerVoiceOrigin::Fixed {
                position: vec3(100000.0, 0.0, 0.0),
            },
            volume: 1.0,
            attenuation: 10.0,
        };
        assert!(!mixer.start_q1_sound(&sound, &far, &ChannelCommand::Auto, None, None).unwrap());
        assert!(mixer.mix(MixRequest::Consume(4)).unwrap().iter().any(|sample| *sample != 0));
    }

    #[test]
    fn schedules_q2_and_notifies() {
        let mut mixer = mixer();
        mixer.set_listener(0, vec3(0.0, 0.0, 0.0), axis()).unwrap();
        let events: Rc<RefCell<Vec<MixerVoiceEvent>>> = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&events);
        mixer.set_voice_observer(Some(Box::new(move |event| seen.borrow_mut().push(event))), None);
        let sound = blip(4410, None);
        let asset = SoundAsset {
            resource: "test".to_string(),
            name: "q2".to_string(),
            pcm: sound.clone(),
        };
        assert!(mixer
            .start_q2_sound(
                &sound,
                &Q2SoundOptions {
                    entity: 0,
                    origin: MixerVoiceOrigin::Local,
                    volume: 1.0,
                    attenuation: 1.0,
                    delay_seconds: Some(0.001),
                    server_milliseconds: Some(0.0),
                },
                &ChannelCommand::Auto,
                Some(asset),
            )
            .unwrap());
        mixer.mix(MixRequest::Consume(5000)).unwrap();
        assert!(events.borrow().iter().any(|event| matches!(event, MixerVoiceEvent::Start { .. })));
        assert!(events.borrow().iter().any(|event| matches!(event, MixerVoiceEvent::Stop { .. })));
    }

    #[test]
    fn paints_loops_and_raw() {
        let mut mixer = mixer();
        mixer.set_listener(0, vec3(0.0, 0.0, 0.0), axis()).unwrap();
        let sound = blip(1024, Some(0));
        mixer
            .update_looping_sound(
                &sound,
                &FrameLoopingSoundOptions {
                    entity: 1,
                    origin: vec3(0.0, 0.0, 0.0),
                    velocity: vec3(0.0, 0.0, 0.0),
                    frame_number: 1,
                    volume: Some(127),
                },
            )
            .unwrap();
        mixer.set_listener(0, vec3(0.0, 0.0, 0.0), axis()).unwrap();
        assert!(mixer.mix(MixRequest::Consume(8)).unwrap().iter().any(|sample| *sample != 0));
        mixer.clear_looping_sounds(true);
        mixer
            .update_real_looping_sound(
                &sound,
                &RealLoopingSoundOptions {
                    entity: 2,
                    origin: vec3(0.0, 0.0, 0.0),
                    velocity: vec3(0.0, 0.0, 0.0),
                    volume: None,
                },
            )
            .unwrap();
        mixer.stop_looping_sound(2).unwrap();
        mixer.clear_looping_sounds(false);
        mixer.queue_raw(&sound, 1.0).unwrap();
        assert!(mixer.raw_end() > 0);
        mixer.clear_raw();
        assert_eq!(mixer.raw_end(), mixer.sample_clock());
        mixer.stop_all().unwrap();
    }

    #[test]
    fn scans_starts_and_rebases_clocks() {
        let mut mixer = mixer();
        let sound = blip(16, None);
        mixer.start_local_sound(&sound, 0, None).unwrap();
        assert!(mixer.scan_channel_starts());
        assert!(!mixer.scan_channel_starts());
        mixer.mix(MixRequest::Consume(32)).unwrap();
        assert!(!mixer.scan_channel_starts());
        assert!(mixer.select_time(32, 40).is_ok());
        assert!(matches!(mixer.select_time(10, 40), Err(AudioError::BadTimeSelect)));
        mixer.select_time(100, 100).unwrap();
        let rebased = mixer.rebase_time(SOUND_TIME_EPOCH + 50).unwrap();
        assert_eq!(rebased, 50);
    }

    #[test]
    fn manages_static_ambient_and_entity_loops() {
        let mut mixer = mixer();
        mixer.set_listener(0, vec3(0.0, 0.0, 0.0), axis()).unwrap();
        let plain = blip(64, None);
        assert!(matches!(mixer.add_static_sound(&plain, vec3(0.0, 0.0, 0.0), 200.0, 1.0, 1), Err(AudioError::StaticLoop)));
        let looping = blip(64, Some(0));
        assert!(mixer.add_static_sound(&looping, vec3(0.0, 0.0, 0.0), 200.0, 1.0, 1).unwrap());
        mixer.remove_static_sound(1);
        mixer.update_ambient(std::slice::from_ref(&looping), &[1.0], 0.016, 0.3, 100.0).unwrap();
        mixer.update_ambient(&[], &[], 0.016, 0.0, 100.0).unwrap();
        mixer
            .set_source_loop_sounds(&[SourceLoopEntry {
                family: SoundFamily::Q2,
                entity: 3,
                sound: looping,
                origin: vec3(0.0, 0.0, 0.0),
                volume: 1.0,
                attenuation: Some(1.0),
            }])
            .unwrap();
        assert!(mixer.mix(MixRequest::Consume(8)).unwrap().iter().any(|sample| *sample != 0));
        mixer.set_source_loop_sounds(&[]).unwrap();
    }

    #[test]
    fn replaces_channels_and_stops_entities() {
        let mut mixer = mixer();
        let sound = blip(4410, None);
        let options = StartSoundOptions {
            entity: 1,
            channel: 1,
            origin: MixerVoiceOrigin::Entity { entity: 1 },
            volume: 127,
        };
        mixer.start_sound(&sound, &options, None).unwrap();
        mixer.start_sound(&sound, &options, None).unwrap();
        mixer.stop_channel(1, 1).unwrap();
        mixer.start_sound(&sound, &options, None).unwrap();
        mixer.stop_entity(1).unwrap();
        mixer.start_sound(&sound, &options, None).unwrap();
        mixer.clear_sound_buffer().unwrap();
        assert!(mixer.channel_volumes().is_empty());
        mixer.set_music_volume(0.5).unwrap();
        mixer.queue_music(&sound).unwrap();
        mixer.reset_raw_to_sound_time();
        mixer.stop_raw();
    }

    #[test]
    fn applies_geometry_and_doppler_loops() {
        let mut mixer = mixer();
        mixer.set_listener(0, vec3(0.0, 0.0, 0.0), axis()).unwrap();
        mixer.set_geometry_transmission(Some(Box::new(|_| 0.5)));
        let sound = blip(2048, Some(0));
        mixer
            .update_looping_sound(
                &sound,
                &FrameLoopingSoundOptions {
                    entity: 1,
                    origin: vec3(10.0, 0.0, 0.0),
                    velocity: vec3(100.0, 0.0, 0.0),
                    frame_number: 1,
                    volume: Some(127),
                },
            )
            .unwrap();
        mixer
            .update_looping_sound(
                &sound,
                &FrameLoopingSoundOptions {
                    entity: 2,
                    origin: vec3(10.0, 0.0, 0.0),
                    velocity: vec3(10000.0, 0.0, 0.0),
                    frame_number: 1,
                    volume: Some(127),
                },
            )
            .unwrap();
        mixer.set_listener(0, vec3(0.0, 0.0, 0.0), axis()).unwrap();
        let mixed = mixer.mix(MixRequest::Consume(2048)).unwrap();
        assert!(mixed.iter().any(|sample| *sample != 0));
        mixer.set_geometry_transmission(None);
        mixer.set_doppler_enabled(false);
        mixer.mix(MixRequest::Consume(8)).unwrap();
    }
}
