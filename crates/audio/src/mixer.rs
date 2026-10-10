//! Load-owned PCM and voices; event admission never creates a second queue.
use qa_core::{
    math,
    names::NameTable,
    primitives::{ClientId, EntityId, Pcm, RuleSetId, SoundAction, SoundEvent, SoundId, Vec3},
};
use std::{num::NonZeroU32, sync::Arc};

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
struct Sample {
    pcm: Pcm,
    policy: SpatialPolicy,
}
pub struct Bank {
    names: NameTable,
    samples: Box<[Option<Sample>]>,
    pub rate: NonZeroU32,
}
impl Bank {
    /// Native sound ordinals are mapped to these handles at the module boundary.
    pub fn load(rate: NonZeroU32, sources: Vec<(String, Pcm, RuleSetId)>) -> Result<Self, String> {
        let paths: Vec<_> = sources
            .iter()
            .map(|(name, _, _)| qa_core::names::canonical_path(name))
            .collect();
        let names =
            NameTable::load(paths.iter().map(|p| p.as_bytes())).map_err(|e| e.to_string())?;
        let mut samples: Box<[Option<Sample>]> =
            std::iter::repeat_with(|| None).take(names.len()).collect();
        for ((_, pcm, rules), path) in sources.into_iter().zip(paths) {
            let id = names.find(path.as_bytes()).ok_or("sound name missing")?;
            if samples[id.0 as usize].is_some() {
                return Err("duplicate sound registration".into());
            }
            let pcm =
                crate::prepare(pcm, rate, false).map_err(|e| format!("sound prepare: {e:?}"))?;
            if pcm.frames() == 0 {
                return Err("empty sound".into());
            }
            samples[id.0 as usize] = Some(Sample {
                pcm,
                policy: SpatialPolicy::load(rules),
            });
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
}

#[derive(Clone, Copy)]
pub struct Listener {
    pub client: ClientId,
    pub entity: Option<EntityId>,
    pub origin: Vec3,
    pub right: Vec3,
}
struct Voice {
    event: SoundEvent,
    frame: usize,
    gain: [i64; 2],
}
#[derive(Default, Debug)]
pub struct MixCounts {
    pub started: u64,
    pub stopped: u64,
    pub replaced: u64,
    pub full: u64,
    pub invalid: u64,
    pub frames: u64,
}
pub struct Mixer {
    pub bank: Arc<Bank>,
    voices: Box<[Option<Voice>]>,
    listeners: Box<[Option<Listener>]>,
    pub counts: MixCounts,
    volume: f32,
}
impl Mixer {
    pub fn load(bank: Arc<Bank>, voices: usize, listeners: usize) -> Self {
        Self {
            bank,
            voices: std::iter::repeat_with(|| None).take(voices).collect(),
            listeners: vec![None; listeners].into_boxed_slice(),
            counts: MixCounts::default(),
            volume: 1.0,
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
    pub fn sound(&mut self, event: SoundEvent) -> bool {
        if event.action == SoundAction::Stop {
            for slot in &mut self.voices {
                if slot.as_ref().is_some_and(|v| {
                    v.event.entity == event.entity && v.event.channel == event.channel
                }) {
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
            || event.position.0.iter().any(|v| !v.is_finite())
        {
            self.counts.invalid += 1;
            return false;
        }
        if event.action == SoundAction::StartLoop
            && let Some(voice) = self.voices.iter_mut().flatten().find(|voice| {
                voice.event.action == SoundAction::StartLoop
                    && voice.event.entity == event.entity
                    && voice.event.channel == event.channel
                    && voice.event.sound == event.sound
            })
        {
            voice.event = event;
            return true;
        }
        let replacement = if event.entity.is_some() && event.channel != 0 {
            self.voices.iter().position(|v| {
                v.as_ref().is_some_and(|v| {
                    v.event.entity == event.entity && v.event.channel == event.channel
                })
            })
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
            frame: 0,
            gain: [0; 2],
        });
        self.counts.started += 1;
        true
    }
    /// Stereo device-rate PCM, accumulated in voice order and saturated once.
    pub fn paint(&mut self, output: &mut [i16]) -> bool {
        if !output.len().is_multiple_of(2) {
            return false;
        }
        let listeners = self.listeners.iter().flatten().count();
        for voice in self.voices.iter_mut().flatten() {
            let Some(sample) = self.bank.samples[voice.event.sound.0 as usize].as_ref() else {
                continue;
            };
            voice.gain = [0; 2];
            for listener in self.listeners.iter().flatten() {
                let gain = gains(voice.event, listener, &sample.policy);
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
        for frame in output.as_chunks_mut::<2>().0 {
            let mut sum = [0i64; 2];
            for slot in &mut self.voices {
                let Some(voice) = slot.as_mut() else {
                    continue;
                };
                let Some(sample) = self.bank.samples[voice.event.sound.0 as usize].as_ref() else {
                    continue;
                };
                let channels = sample.pcm.channels as usize;
                let offset = voice.frame * channels;
                for (side, total) in sum.iter_mut().enumerate() {
                    let value = sample.pcm.samples[offset + side.min(channels - 1)];
                    *total += (i64::from(value) * voice.gain[side]) >> 8;
                }
                voice.frame += 1;
                if voice.frame == sample.pcm.frames() {
                    let start = sample
                        .pcm
                        .loop_start
                        .or_else(|| (voice.event.action == SoundAction::StartLoop).then_some(0));
                    if let Some(start) = start {
                        voice.frame = start;
                    } else {
                        *slot = None;
                    }
                }
            }
            let volume = (self.volume * 256.0) as i64;
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
    let distance = math::length(delta);
    let direction = math::normalized(delta);
    let dot = listener.right.dot(direction);
    let distance =
        (distance - policy.distance_offset).max(0.0) * event.attenuation * policy.distance_scale;
    [1.0 - dot, 1.0 + dot]
        .map(|pan| (master as f32 * ((1.0 - distance) * pan * policy.separation)).max(0.0) as i32)
}
