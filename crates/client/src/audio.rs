//! Audio hooks: spatialization data and channel management.
//!
//! Donor provenance: `src/audio/mixer.ts` (`spatializeSoundOrigin`,
//! `SOUND_FULLVOLUME`, `SOUND_ATTENUATE`, `AudioMixer` channel walking,
//! `SOUND_TIME_EPOCH`) and `src/audio/types.ts` (`sourceSoundChannel`,
//! `SoundFamily`, `SharedSoundChannel`).
//!
//! Voice allocation, respatialization, PCM decoding, software mixing,
//! music, reverb, and the unified output engine live here; only the raw
//! SDL/file backends stay in `qa-platform`.
//!
//! Donor provenance: `src/audio/types.ts`, `src/audio/wav.ts`,
//! `src/audio/adpcm.ts`, `src/audio/wavelet.ts`,
//! `src/audio/source-paint.ts`, `src/audio/mixer.ts`,
//! `src/audio/streams.ts`, `src/audio/music.ts`,
//! `src/audio/output.ts`, `src/audio/reverb.ts`,
//! `src/audio/reverb-presets.ts`, `src/audio/environments.ts`,
//! `src/audio/geometry.ts`, `src/audio/bank.ts`, `src/audio/engine.ts`.

pub mod adpcm;
pub mod bank;
pub mod engine;
pub mod environments;
pub mod error;
pub mod geometry;
pub mod mixer;
pub mod music;
pub mod output;
pub mod paint;
pub mod reverb;
pub mod reverb_presets;
pub mod streams;
pub mod types;
pub mod wav;
pub mod wavelet;

use qa_core::math::{dot3, length3, normalize3, sub3, vec3, Axis, Vec3};

use crate::ClientError;

/// Sound content family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SoundFamily {
    /// Quake I.
    Q1,
    /// Quake II.
    Q2,
    /// Quake III.
    Q3,
}

/// Shared sound channel (`SharedSoundChannel`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SoundChannel {
    /// Weapon channel.
    Weapon,
    /// Voice channel.
    Voice,
    /// Item channel.
    Item,
    /// Body channel.
    Body,
    /// Local channel (Q3).
    Local,
    /// Local-sound channel (Q3).
    LocalSound,
    /// Announcer channel (Q3).
    Announcer,
    /// Family extension channel.
    Extension {
        /// Family.
        family: SoundFamily,
        /// Source channel number.
        channel: i32,
    },
}

/// Channel replacement command (`SoundChannelCommand`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChannelCommand {
    /// Allocate a free channel, never replacing.
    Auto,
    /// Stop the entity's voices, then allocate.
    ReplaceActor,
    /// Replace the entity's voice on this channel, else allocate.
    Channel(SoundChannel),
}

/// Map a source channel number to a replacement command.
///
/// Channel 0 allocates; Q1 channel -1 replaces the actor; Q3 names
/// seven channels, Q1/Q2 four, and higher numbers become extensions.
pub fn source_sound_channel(family: SoundFamily, channel: i32) -> Result<ChannelCommand, ClientError> {
    if channel < 0 && !(family == SoundFamily::Q1 && channel == -1) {
        return Err(ClientError::BadSoundChannel {
            family: match family {
                SoundFamily::Q1 => "q1",
                SoundFamily::Q2 => "q2",
                SoundFamily::Q3 => "q3",
            },
            channel,
        });
    }
    if channel == -1 {
        return Ok(ChannelCommand::ReplaceActor);
    }
    if channel == 0 {
        return Ok(ChannelCommand::Auto);
    }
    const Q3: [SoundChannel; 7] = [
        SoundChannel::Local,
        SoundChannel::Weapon,
        SoundChannel::Voice,
        SoundChannel::Item,
        SoundChannel::Body,
        SoundChannel::LocalSound,
        SoundChannel::Announcer,
    ];
    const Q12: [SoundChannel; 4] = [
        SoundChannel::Weapon,
        SoundChannel::Voice,
        SoundChannel::Item,
        SoundChannel::Body,
    ];
    let named = if family == SoundFamily::Q3 {
        Q3.get(channel as usize - 1).copied()
    } else {
        Q12.get(channel as usize - 1).copied()
    };
    Ok(ChannelCommand::Channel(
        named.unwrap_or(SoundChannel::Extension { family, channel }),
    ))
}

/// Full-volume radius (`SOUND_FULLVOLUME`).
pub const SOUND_FULL_VOLUME: f32 = 80.0;
/// Distance attenuation slope (`SOUND_ATTENUATE`).
pub const SOUND_ATTENUATE: f32 = 0.0008;
/// Default channel capacity (mixer slots).
pub const DEFAULT_CHANNEL_CAPACITY: usize = 96;
/// Default entity-position slots (`MAX_GENTITIES`).
pub const MAX_GENTITIES: usize = 1024;
/// Quake channel volume units for `S_StartSound`.
pub const START_SOUND_VOLUME: i32 = 127;

/// Mixer output channel count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputChannels {
    /// Mono: both scales are 1.
    Mono,
    /// Stereo: constant-power pan.
    Stereo,
}

/// Spatialized stereo volume in Quake units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StereoVolume {
    /// Left channel.
    pub left: i32,
    /// Right channel.
    pub right: i32,
}

/// `S_SpatializeOrigin`, including the mono-output branch.
///
/// Distance past [`SOUND_FULL_VOLUME`] attenuates linearly; pan is the
/// negated projection onto the listener's left axis.
#[must_use]
pub fn spatialize_sound_origin(
    position: Vec3,
    listener_origin: Vec3,
    listener_axis: Axis,
    volume: f32,
    channels: OutputChannels,
) -> StereoVolume {
    let source = sub3(position, listener_origin);
    let distance = length3(source);
    let direction = normalize3(source);
    let pan = -dot3(direction, listener_axis[1]);
    let beyond = (distance - SOUND_FULL_VOLUME).max(0.0);
    let loss = beyond * SOUND_ATTENUATE;
    let (right_scale, left_scale) = match channels {
        OutputChannels::Mono => (1.0, 1.0),
        OutputChannels::Stereo => ((0.5 * (1.0 + pan)).max(0.0), (0.5 * (1.0 - pan)).max(0.0)),
    };
    StereoVolume {
        left: (volume * ((1.0 - loss) * left_scale)) as i32,
        right: (volume * ((1.0 - loss) * right_scale)) as i32,
    }
    .clamped()
}

impl StereoVolume {
    fn clamped(self) -> Self {
        Self {
            left: self.left.max(0),
            right: self.right.max(0),
        }
    }
}

/// Voice origin (`VoiceOrigin`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VoiceOrigin {
    /// At the listener.
    Local,
    /// Fixed world position.
    Fixed(Vec3),
    /// Follows an entity.
    Entity(u32),
}

/// Allocated mixer voice (headless: no PCM state).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Voice {
    /// Unique voice ID.
    pub id: u64,
    /// Sound asset handle (content-owned).
    pub asset: u32,
    /// Emitting entity.
    pub entity: u32,
    /// Replacement channel, if any.
    pub channel: Option<SoundChannel>,
    /// Voice origin.
    pub origin: VoiceOrigin,
    /// Quake volume units.
    pub volume: i32,
    /// Spatialized stereo volume.
    pub stereo: StereoVolume,
}

/// Headless channel pool: allocation, replacement, and `S_Update` volumes.
#[derive(Debug, Clone)]
pub struct ChannelPool {
    voices: Vec<Option<Voice>>,
    entity_positions: Vec<Vec3>,
    listener_entity: u32,
    listener_origin: Vec3,
    listener_axis: Axis,
    output_channels: OutputChannels,
    next_id: u64,
}

impl ChannelPool {
    /// Pool with default capacities and a stereo forward listener.
    #[must_use]
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_CHANNEL_CAPACITY, MAX_GENTITIES, OutputChannels::Stereo)
    }

    /// Pool with explicit capacities.
    #[must_use]
    pub fn with_capacity(channel_capacity: usize, entity_capacity: usize, output_channels: OutputChannels) -> Self {
        Self {
            voices: vec![None; channel_capacity],
            entity_positions: vec![vec3(0.0, 0.0, 0.0); entity_capacity],
            listener_entity: 0,
            listener_origin: vec3(0.0, 0.0, 0.0),
            listener_axis: [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)],
            output_channels,
            next_id: 1,
        }
    }

    /// Set the listener transform.
    pub fn set_listener(&mut self, entity: u32, origin: Vec3, axis: Axis) {
        self.listener_entity = entity;
        self.listener_origin = origin;
        self.listener_axis = axis;
    }

    /// Set an entity's world position.
    pub fn set_entity_position(&mut self, entity: u32, position: Vec3) -> Result<(), ClientError> {
        let Some(slot) = self.entity_positions.get_mut(entity as usize) else {
            return Err(ClientError::BadAudioEntity { index: entity as usize });
        };
        *slot = position;
        Ok(())
    }

    /// Live voices.
    pub fn voices(&self) -> impl Iterator<Item = &Voice> {
        self.voices.iter().flatten()
    }

    /// Resolve a voice origin to a world position.
    fn resolve(&self, voice: &Voice) -> Result<Vec3, ClientError> {
        match voice.origin {
            VoiceOrigin::Local => Ok(self.listener_origin),
            VoiceOrigin::Fixed(position) => Ok(position),
            VoiceOrigin::Entity(entity) => self
                .entity_positions
                .get(entity as usize)
                .copied()
                .ok_or(ClientError::BadAudioEntity { index: entity as usize }),
        }
    }

    fn spatialize(&self, voice: &Voice) -> Result<StereoVolume, ClientError> {
        Ok(spatialize_sound_origin(
            self.resolve(voice)?,
            self.listener_origin,
            self.listener_axis,
            voice.volume as f32,
            self.output_channels,
        ))
    }

    /// Start a voice with `S_StartSound` replacement semantics.
    pub fn start_sound(
        &mut self,
        asset: u32,
        entity: u32,
        command: ChannelCommand,
        origin: VoiceOrigin,
        volume: i32,
    ) -> Result<u64, ClientError> {
        let channel = match command {
            ChannelCommand::Auto => None,
            ChannelCommand::ReplaceActor => {
                self.stop_entity(entity);
                None
            }
            ChannelCommand::Channel(channel) => Some(channel),
        };
        if let Some(channel) = channel {
            let replace = self
                .voices
                .iter()
                .position(|slot| slot.is_some_and(|voice| voice.entity == entity && voice.channel == Some(channel)));
            if let Some(index) = replace {
                let id = self.next_id;
                self.next_id += 1;
                let mut voice = Voice {
                    id,
                    asset,
                    entity,
                    channel: Some(channel),
                    origin,
                    volume,
                    stereo: StereoVolume::default(),
                };
                voice.stereo = self.spatialize(&voice)?;
                self.voices[index] = Some(voice);
                return Ok(id);
            }
        }
        let Some(index) = self.voices.iter().position(|slot| slot.is_none()) else {
            return Err(ClientError::NoFreeChannel);
        };
        let id = self.next_id;
        self.next_id += 1;
        let mut voice = Voice {
            id,
            asset,
            entity,
            channel,
            origin,
            volume,
            stereo: StereoVolume::default(),
        };
        voice.stereo = self.spatialize(&voice)?;
        self.voices[index] = Some(voice);
        Ok(id)
    }

    /// Re-spatialize every voice (`S_Update` channel walk).
    pub fn update_volumes(&mut self) {
        let listener_origin = self.listener_origin;
        let listener_axis = self.listener_axis;
        let output_channels = self.output_channels;
        let positions = &self.entity_positions;
        for voice in self.voices.iter_mut().flatten() {
            let position = match voice.origin {
                VoiceOrigin::Local => Some(listener_origin),
                VoiceOrigin::Fixed(position) => Some(position),
                VoiceOrigin::Entity(entity) => positions.get(entity as usize).copied(),
            };
            voice.stereo = position.map_or(StereoVolume::default(), |position| {
                spatialize_sound_origin(
                    position,
                    listener_origin,
                    listener_axis,
                    voice.volume as f32,
                    output_channels,
                )
            });
        }
    }

    /// Stop every voice from an entity.
    pub fn stop_entity(&mut self, entity: u32) {
        for slot in self.voices.iter_mut() {
            if slot.is_some_and(|voice| voice.entity == entity) {
                *slot = None;
            }
        }
    }

    /// Stop every voice.
    pub fn stop_all(&mut self) {
        for slot in self.voices.iter_mut() {
            *slot = None;
        }
    }
}

impl Default for ChannelPool {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn axis() -> Axis {
        [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)]
    }

    #[test]
    fn channels_map_per_family() {
        assert_eq!(
            source_sound_channel(SoundFamily::Q1, 1).unwrap(),
            ChannelCommand::Channel(SoundChannel::Weapon)
        );
        assert_eq!(
            source_sound_channel(SoundFamily::Q3, 1).unwrap(),
            ChannelCommand::Channel(SoundChannel::Local)
        );
        assert_eq!(
            source_sound_channel(SoundFamily::Q3, 7).unwrap(),
            ChannelCommand::Channel(SoundChannel::Announcer)
        );
        assert!(matches!(
            source_sound_channel(SoundFamily::Q1, 9).unwrap(),
            ChannelCommand::Channel(SoundChannel::Extension { .. })
        ));
        assert_eq!(
            source_sound_channel(SoundFamily::Q1, -1).unwrap(),
            ChannelCommand::ReplaceActor
        );
        assert_eq!(source_sound_channel(SoundFamily::Q3, 0).unwrap(), ChannelCommand::Auto);
        assert!(source_sound_channel(SoundFamily::Q3, -1).is_err());
    }

    #[test]
    fn spatialization_splits_pans_and_attenuates() {
        let origin = vec3(0.0, 0.0, 0.0);
        let center = spatialize_sound_origin(origin, origin, axis(), 127.0, OutputChannels::Stereo);
        assert_eq!(center, StereoVolume { left: 63, right: 63 });
        let full = spatialize_sound_origin(vec3(80.0, 0.0, 0.0), origin, axis(), 127.0, OutputChannels::Stereo);
        assert_eq!(full, StereoVolume { left: 63, right: 63 });
        let far = spatialize_sound_origin(vec3(1080.0, 0.0, 0.0), origin, axis(), 127.0, OutputChannels::Stereo);
        assert_eq!(far, StereoVolume { left: 12, right: 12 });
        let left = spatialize_sound_origin(vec3(0.0, 100.0, 0.0), origin, axis(), 127.0, OutputChannels::Stereo);
        assert_eq!(left.right, 0);
        assert_eq!(left.left, 124);
        let mono = spatialize_sound_origin(vec3(0.0, 100.0, 0.0), origin, axis(), 127.0, OutputChannels::Mono);
        assert_eq!(mono, StereoVolume { left: 124, right: 124 });
    }

    #[test]
    fn pool_replaces_stops_and_updates() {
        let mut pool = ChannelPool::with_capacity(2, 8, OutputChannels::Stereo);
        pool.set_entity_position(5, vec3(100.0, 0.0, 0.0)).unwrap();
        assert!(pool.set_entity_position(9, origin()).is_err());
        let auto = pool
            .start_sound(1, 1, ChannelCommand::Auto, VoiceOrigin::Local, 127)
            .unwrap();
        let first = pool
            .start_sound(
                2,
                5,
                ChannelCommand::Channel(SoundChannel::Weapon),
                VoiceOrigin::Entity(5),
                127,
            )
            .unwrap();
        let second = pool
            .start_sound(
                3,
                5,
                ChannelCommand::Channel(SoundChannel::Weapon),
                VoiceOrigin::Entity(5),
                127,
            )
            .unwrap();
        assert_ne!(first, second);
        assert_eq!(pool.voices().count(), 2);
        assert!(pool
            .start_sound(4, 2, ChannelCommand::Auto, VoiceOrigin::Local, 127)
            .is_err());
        pool.stop_entity(1);
        assert!(pool.voices().all(|voice| voice.id != auto));
        pool.set_entity_position(5, vec3(1080.0, 0.0, 0.0)).unwrap();
        pool.update_volumes();
        let voice = pool.voices().next().unwrap();
        assert_eq!(voice.stereo, StereoVolume { left: 12, right: 12 });
        pool.stop_all();
        assert_eq!(pool.voices().count(), 0);
    }

    fn origin() -> Vec3 {
        vec3(0.0, 0.0, 0.0)
    }
}
