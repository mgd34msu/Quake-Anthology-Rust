//! Load-owned PCM and voices; event admission never creates a second queue.
use qa_core::{
    math,
    names::NameTable,
    primitives::{
        ClientId, Pcm, ResourceNamespace, RuleSetId, SoundAction, SoundEmitter, SoundEvent,
        SoundId, Vec3,
    },
};
use std::{num::NonZeroU32, sync::Arc};

#[derive(Clone, Copy)]
struct SpatialPolicy {
    distance_offset: f32,
    distance_scale: f32,
    separation: f32,
}
impl SpatialPolicy {
    fn load(rules: RuleSetId) -> Self {
        match rules {
            RuleSetId::Quake | RuleSetId::QuakeWorld => Self {
                distance_offset: 0.0,
                distance_scale: 0.001,
                separation: 1.0,
            },
            RuleSetId::Quake2 | RuleSetId::Quake2Rerelease => Self {
                distance_offset: 80.0,
                distance_scale: 0.001,
                separation: 0.5,
            },
            RuleSetId::Quake3 => Self {
                distance_offset: 80.0,
                distance_scale: 1.0 / (1250.0 - 80.0),
                separation: 0.5,
            },
        }
    }
}
pub struct Bank {
    names: NameTable,
    samples: Box<[Option<Pcm>]>,
    pub rate: NonZeroU32,
}
impl Bank {
    /// Native sound ordinals are mapped to these handles at the module boundary.
    pub fn load(rate: NonZeroU32, sources: Vec<(String, Pcm)>) -> Result<Self, String> {
        let paths: Vec<_> = sources
            .iter()
            .map(|(name, _)| qa_core::names::canonical_path(name))
            .collect();
        let names =
            NameTable::load(paths.iter().map(|p| p.as_bytes())).map_err(|e| e.to_string())?;
        let mut samples: Box<[Option<Pcm>]> =
            std::iter::repeat_with(|| None).take(names.len()).collect();
        for ((_, pcm), path) in sources.into_iter().zip(paths) {
            let id = names.find(path.as_bytes()).ok_or("sound name missing")?;
            if samples[id.0 as usize].is_some() {
                return Err("duplicate sound registration".into());
            }
            let pcm =
                crate::prepare(pcm, rate, false).map_err(|e| format!("sound prepare: {e:?}"))?;
            if pcm.frames() == 0 {
                return Err("empty sound".into());
            }
            samples[id.0 as usize] = Some(pcm);
        }
        Ok(Self {
            names,
            samples,
            rate,
        })
    }
    pub fn find(&self, path: &str) -> Option<SoundId> {
        self.names.find_path(path).map(|id| SoundId(id.0))
    }
    pub fn len(&self) -> usize {
        self.samples.len()
    }
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }
}

#[derive(Clone, Copy)]
pub struct Listener {
    pub client: ClientId,
    pub entity: Option<SoundEmitter>,
    pub origin: Vec3,
    pub right: Vec3,
}
struct Voice {
    event: SoundEvent,
    policy: SpatialPolicy,
    frame: usize,
    gain: [i64; 2],
    begin: Option<u64>,
    order: u64,
}
fn same_channel(a: &SoundEvent, b: &SoundEvent) -> bool {
    a.entity == b.entity && a.channel == b.channel && a.client == b.client
}
#[derive(Default, Debug)]
pub struct MixCounts {
    pub started: u64,
    pub stopped: u64,
    pub replaced: u64,
    pub full: u64,
    pub invalid: u64,
    pub frames: u64,
    pub respatialized: u64,
    pub missing_origins: u64,
}
pub struct Mixer {
    pub bank: Arc<Bank>,
    voices: Box<[Option<Voice>]>,
    listeners: Box<[Option<Listener>]>,
    pub counts: MixCounts,
    volume: f32,
    next_start: Option<u64>,
}
impl Mixer {
    pub fn load(bank: Arc<Bank>, voices: usize, listeners: usize) -> Self {
        Self {
            bank,
            voices: std::iter::repeat_with(|| None).take(voices).collect(),
            listeners: vec![None; listeners].into_boxed_slice(),
            counts: MixCounts::default(),
            volume: 1.0,
            next_start: None,
        }
    }
    pub fn listen(&mut self, listeners: &[Option<Listener>], volume: f32) {
        for (index, slot) in self.listeners.iter_mut().enumerate() {
            *slot = listeners.get(index).copied().flatten();
            // Repeated views of one client are one audible listener.
            if slot.is_some_and(|l| {
                listeners[..index.min(listeners.len())]
                    .iter()
                    .flatten()
                    .any(|earlier| earlier.client == l.client)
            }) {
                *slot = None;
            }
        }
        self.volume = if volume.is_finite() {
            volume.clamp(0.0, 1.0)
        } else {
            0.0
        };
    }
    /// Resolve moving native emitters once at the CLIENT frame boundary.
    /// Fixed-position sounds and non-CLIENT identities retain their supplied
    /// origin. An absent pose retains the voice's last known origin.
    pub fn respatialize(&mut self, mut resolve: impl FnMut(ClientId, u32) -> Option<Vec3>) {
        for voice in self.voices.iter_mut().flatten() {
            if voice.event.positioned {
                continue;
            }
            let Some(SoundEmitter::Native {
                namespace: ResourceNamespace::Client(client),
                slot,
            }) = voice.event.entity
            else {
                continue;
            };
            let origin = u32::try_from(slot)
                .ok()
                .and_then(|slot| resolve(client, slot));
            if let Some(origin) = origin.filter(|origin| origin.is_finite()) {
                voice.event.position = origin;
                self.counts.respatialized += 1;
            } else {
                self.counts.missing_origins += 1;
            }
        }
    }

    pub fn sound(&mut self, event: SoundEvent) -> bool {
        if event.action == SoundAction::Stop {
            for slot in &mut self.voices {
                if slot
                    .as_ref()
                    .is_some_and(|v| same_channel(&v.event, &event))
                {
                    *slot = None;
                    self.counts.stopped += 1;
                }
            }
            return true;
        }
        if self
            .bank
            .samples
            .get(event.sound.0 as usize)
            .is_none_or(Option::is_none)
            || !event.volume.is_finite()
            || !(0.0..=1.0).contains(&event.volume)
            || !event.attenuation.is_finite()
            || event.attenuation < 0.0
            || !event.time_offset.is_finite()
            || event.time_offset < 0.0
            || !event.position.is_finite()
        {
            self.counts.invalid += 1;
            return false;
        }
        for voice in &mut self.voices {
            let Some(voice) = voice else { continue };
            if !same_channel(&voice.event, &event) {
                continue;
            }
            if event.action == SoundAction::StartLoop
                && voice.event.action == SoundAction::StartLoop
                && voice.event.sound == event.sound
                && voice.begin.is_none()
            {
                voice.event = event;
                voice.policy = SpatialPolicy::load(event.spatial_rules);
                return true;
            }
        }
        let delay = (event.time_offset * self.bank.rate.get() as f32) as u64;
        let begin = (delay != 0).then(|| self.counts.frames.saturating_add(delay));
        let replacement = if begin.is_none() {
            self.channel_voice(event)
        } else {
            None
        };
        let Some(index) = replacement.or_else(|| self.voices.iter().position(Option::is_none))
        else {
            self.counts.full += 1;
            return false;
        };
        self.counts.replaced += u64::from(replacement.is_some());
        self.voices[index] = Some(Voice {
            event,
            policy: SpatialPolicy::load(event.spatial_rules),
            frame: 0,
            gain: [0; 2],
            begin,
            order: self.counts.started,
        });
        if let Some(begin) = begin {
            self.next_start = Some(self.next_start.map_or(begin, |next| next.min(begin)));
        }
        self.counts.started += 1;
        true
    }
    fn channel_voice(&self, event: SoundEvent) -> Option<usize> {
        if event.entity.is_none() || event.channel == 0 {
            return None;
        }
        self.voices.iter().position(|voice| {
            voice
                .as_ref()
                .is_some_and(|voice| voice.begin.is_none() && same_channel(&voice.event, &event))
        })
    }
    /// Q2 issues scheduled sounds at their sample time, then selects a channel.
    /// Pending starts use the existing bounded voice table, not another queue.
    fn start_ready(&mut self, time: u64) {
        loop {
            let next = self
                .voices
                .iter()
                .enumerate()
                .filter_map(|(index, voice)| {
                    let voice = voice.as_ref()?;
                    Some((voice.begin?, voice.order, index))
                })
                .min();
            let Some((begin, _, index)) = next else {
                self.next_start = None;
                return;
            };
            self.next_start = Some(begin);
            if begin > time {
                return;
            }
            let Some(event) = self.voices[index].as_ref().map(|voice| voice.event) else {
                continue;
            };
            if let Some(previous) = self.channel_voice(event) {
                self.voices[previous] = None;
                self.counts.replaced += 1;
            }
            if let Some(voice) = &mut self.voices[index] {
                voice.begin = None;
            }
        }
    }
    /// Stereo device-rate PCM, accumulated in voice order and saturated once.
    pub fn paint(&mut self, output: &mut [i16]) -> bool {
        if !output.len().is_multiple_of(2) {
            return false;
        }
        for voice in self.voices.iter_mut().flatten() {
            voice.gain = [0; 2];
            let mut listeners = 0;
            for listener in self
                .listeners
                .iter()
                .flatten()
                .filter(|l| voice.event.applies_to_client(l.client))
            {
                listeners += 1;
                let gain = gains(voice.event, listener, &voice.policy);
                for (to, from) in voice.gain.iter_mut().zip(gain) {
                    *to += i64::from(from);
                }
            }
            if listeners != 0 {
                for gain in &mut voice.gain {
                    *gain /= listeners as i64;
                }
            }
        }
        let volume = (self.volume * 256.0) as i64;
        for (offset, frame) in output.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            let time = self.counts.frames.saturating_add(offset as u64);
            if self.next_start.is_some_and(|next| next <= time) {
                self.start_ready(time);
            }
            let mut sum = [0i64; 2];
            for slot in &mut self.voices {
                let Some(voice) = slot.as_mut() else {
                    continue;
                };
                if voice.begin.is_some() {
                    continue;
                }
                let Some(sample) = self.bank.samples[voice.event.sound.0 as usize].as_ref() else {
                    continue;
                };
                let channels = sample.channels as usize;
                let offset = voice.frame * channels;
                for (side, total) in sum.iter_mut().enumerate() {
                    let value = sample.samples[offset + side.min(channels - 1)];
                    *total += (i64::from(value) * voice.gain[side]) >> 8;
                }
                voice.frame += 1;
                if voice.frame == sample.frames() {
                    let start = sample
                        .loop_start
                        .or_else(|| (voice.event.action == SoundAction::StartLoop).then_some(0));
                    if let Some(start) = start {
                        voice.frame = start;
                    } else {
                        *slot = None;
                    }
                }
            }
            for side in 0..2 {
                frame[side] = ((sum[side] * volume) >> 8)
                    .clamp(i64::from(i16::MIN), i64::from(i16::MAX))
                    as i16;
            }
        }
        self.counts.frames += (output.len() / 2) as u64;
        true
    }
}
fn gains(event: SoundEvent, listener: &Listener, policy: &SpatialPolicy) -> [i32; 2] {
    let master = (event.volume * 255.0) as i32;
    if event.attenuation == 0.0
        || event
            .entity
            .is_some_and(|entity| listener.entity == Some(entity))
    {
        return [master; 2];
    }
    let delta = event.position - listener.origin;
    let mut direction = delta;
    let distance = math::normalize(&mut direction);
    let dot = listener.right.dot(direction);
    let distance =
        (distance - policy.distance_offset).max(0.0) * event.attenuation * policy.distance_scale;
    [1.0 - dot, 1.0 + dot]
        .map(|pan| (master as f32 * ((1.0 - distance) * pan * policy.separation)).max(0.0) as i32)
}
