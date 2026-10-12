//! Load-owned PCM and voices; event admission never creates a second queue.
use qa_core::{
    math,
    names::NameTable,
    primitives::{
        ClientId, LoopGain, LoopOrigin, LoopPhase, Pcm, ResourceNamespace, SoundAction,
        SoundEmitter, SoundEvent, SoundId, Vec3,
    },
};
use std::{num::NonZeroU32, sync::Arc};

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
    /// Numeric registered-name access; no path reconstruction or hashing.
    pub fn name(&self, sound: SoundId) -> Option<&[u8]> {
        self.names.get(qa_core::primitives::NameId(sound.0))
    }
    pub fn has_loop_marker(&self, sound: SoundId) -> bool {
        self.samples
            .get(sound.0 as usize)
            .and_then(Option::as_ref)
            .is_some_and(|pcm| pcm.loop_start.is_some())
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
    frame: usize,
    gain: Gain,
    loop_leader: bool,
    begin: Option<u64>,
    order: u64,
}
#[derive(Clone, Copy)]
enum Gain {
    Integer([i64; 2]),
    Float([f32; 2]),
}
impl Gain {
    fn zero(event: SoundEvent) -> Self {
        if event
            .looping
            .is_some_and(|playback| matches!(playback.master, LoopGain::Float(_)))
        {
            Self::Float([0.0; 2])
        } else {
            Self::Integer([0; 2])
        }
    }
    fn add(&mut self, value: Self) -> bool {
        match (self, value) {
            (Self::Integer(to), Self::Integer(from)) => {
                for (to, from) in to.iter_mut().zip(from) {
                    *to += from;
                }
            }
            (Self::Float(to), Self::Float(from)) => {
                for (to, from) in to.iter_mut().zip(from) {
                    *to += from;
                }
            }
            _ => return false,
        }
        true
    }
    fn average(&mut self, listeners: usize) {
        if listeners == 0 {
            return;
        }
        match self {
            Self::Integer(gain) => {
                for gain in gain {
                    *gain /= listeners as i64;
                }
            }
            Self::Float(gain) => {
                for gain in gain {
                    *gain /= listeners as f32;
                }
            }
        }
    }
    /// Native same-PCM groups saturate before applying the device volume.
    fn prepare_loop(&mut self, volume: f32, volume_scale: u16) {
        match self {
            Self::Integer(gain) => {
                let volume = (volume * f32::from(volume_scale)) as i64;
                for gain in gain {
                    *gain = (*gain).min(255) * volume;
                }
            }
            Self::Float(gain) => {
                for gain in gain {
                    *gain = (*gain).min(1.0) * volume;
                }
            }
        }
    }
}
fn same_channel(a: &SoundEvent, b: &SoundEvent) -> bool {
    a.entity == b.entity
        && a.channel == b.channel
        && a.client == b.client
        && a.client_lifetime == b.client_lifetime
}
fn same_loop(a: &SoundEvent, b: &SoundEvent) -> bool {
    a.looping.is_some()
        && a.entity == b.entity
        && a.client == b.client
        && a.client_lifetime == b.client_lifetime
}
fn loop_order(event: SoundEvent) -> (u32, u32, u32, u32) {
    let client = event.client.map_or(0, |client| client.0);
    let (slot, generation) = event
        .client_lifetime
        .map_or((0, 0), |id| (id.slot, id.generation));
    let entity = match event.entity {
        Some(SoundEmitter::Native { slot, .. }) => slot as u32,
        _ => 0,
    };
    (client, slot, generation, entity)
}
fn loop_group(event: SoundEvent, index: usize) -> (u32, u32, u32, u32, u16, usize) {
    let (client, slot, generation, _) = loop_order(event);
    let (scale, separate) = event.looping.map_or((0, index + 1), |playback| {
        let scale = match playback.master {
            LoopGain::Integer { volume_scale, .. } => volume_scale,
            LoopGain::Float(_) => 0,
        };
        let separate = if playback.phase == LoopPhase::Painted {
            0
        } else {
            index + 1
        };
        (scale, separate)
    });
    (client, slot, generation, event.sound.0, scale, separate)
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
    loop_order: Box<[usize]>,
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
            loop_order: vec![0; voices].into_boxed_slice(),
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
    pub fn respatialize(
        &mut self,
        mut resolve: impl FnMut(ClientId, u32, Option<LoopOrigin>) -> Option<Vec3>,
    ) {
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
                .and_then(|slot| resolve(client, slot, voice.event.looping.map(|p| p.origin)));
            if let Some(origin) = origin.filter(|origin| origin.is_finite()) {
                voice.event.position = origin;
                self.counts.respatialized += 1;
            } else {
                self.counts.missing_origins += 1;
            }
        }
    }

    pub fn sound(&mut self, event: SoundEvent) -> bool {
        if matches!(
            event.action,
            SoundAction::RetireClient | SoundAction::ResetClientLoops
        ) {
            let (Some(client), Some(lifetime)) = (event.client, event.client_lifetime) else {
                self.counts.invalid += 1;
                return false;
            };
            for slot in &mut self.voices {
                if slot.as_ref().is_some_and(|voice| {
                    voice.event.client == Some(client)
                        && voice.event.client_lifetime == Some(lifetime)
                        && (event.action == SoundAction::RetireClient
                            || voice.event.looping.is_some())
                }) {
                    *slot = None;
                    self.counts.stopped += 1;
                }
            }
            return true;
        }
        if matches!(event.action, SoundAction::Stop | SoundAction::StopLoop) {
            for slot in &mut self.voices {
                if slot.as_ref().is_some_and(|v| {
                    if event.action == SoundAction::StopLoop {
                        same_loop(&v.event, &event)
                    } else {
                        v.event.looping.is_none() && same_channel(&v.event, &event)
                    }
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
            || !event.time_offset.is_finite()
            || event.time_offset < 0.0
            || !event.position.is_finite()
            || event.looping.is_some_and(|playback| {
                event.action != SoundAction::StartLoop
                    || event.time_offset != 0.0
                    || event.client_lifetime.is_none()
                    || !matches!(event.entity, Some(SoundEmitter::Native {
                        namespace: ResourceNamespace::Client(client), slot
                    }) if Some(client) == event.client && slot >= 0)
                    || match playback.master {
                        LoopGain::Float(master) => {
                            !master.is_finite() || !(0.0..=1.0).contains(&master)
                        }
                        LoopGain::Integer { volume_scale, .. } => volume_scale == 0,
                    }
            })
        {
            self.counts.invalid += 1;
            return false;
        }
        for voice in &mut self.voices {
            let Some(voice) = voice else { continue };
            let matches = if event.looping.is_some() {
                same_loop(&voice.event, &event)
            } else {
                voice.event.looping.is_none() && same_channel(&voice.event, &event)
            };
            if !matches {
                continue;
            }
            if event.action == SoundAction::StartLoop
                && voice.event.action == SoundAction::StartLoop
                && voice.event.sound == event.sound
                && voice.begin.is_none()
            {
                voice.event = event;
                return true;
            }
        }
        let delay = (event.time_offset * self.bank.rate.get() as f32) as u64;
        let begin = (delay != 0).then(|| self.counts.frames.saturating_add(delay));
        let replacement = if event.looping.is_some() {
            self.voices.iter().position(|voice| {
                voice
                    .as_ref()
                    .is_some_and(|voice| same_loop(&voice.event, &event))
            })
        } else if begin.is_none() {
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
            frame: 0,
            gain: Gain::zero(event),
            loop_leader: false,
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
            voice.as_ref().is_some_and(|voice| {
                voice.event.looping.is_none()
                    && voice.begin.is_none()
                    && same_channel(&voice.event, &event)
            })
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
            voice.gain = Gain::zero(voice.event);
            voice.loop_leader = false;
            let mut listeners = 0;
            for listener in self
                .listeners
                .iter()
                .flatten()
                .filter(|l| voice.event.applies_to_client(l.client))
            {
                listeners += 1;
                let gain = gains(voice.event, listener);
                voice.gain.add(gain);
            }
            voice.gain.average(listeners);
        }
        let loops = self.prepare_loops();
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
                if voice.event.looping.is_some() {
                    continue;
                }
                let Some(sample) = self.bank.samples[voice.event.sound.0 as usize].as_ref() else {
                    continue;
                };
                let channels = sample.channels as usize;
                let offset = voice.frame * channels;
                if let Gain::Integer(gain) = voice.gain {
                    for (side, total) in sum.iter_mut().enumerate() {
                        let value = sample.samples[offset + side.min(channels - 1)];
                        *total += (i64::from(value) * gain[side]) >> 8;
                    }
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
            // Native integer loops multiply their grouped volume before the
            // sample product. Ordinary channels retain their existing order.
            let mut integer = sum.map(|sum| sum * volume);
            let mut floating = [0.0f32; 2];
            let mut has_float = false;
            for &index in &self.loop_order[..loops] {
                let Some(voice) = self.voices[index].as_mut() else {
                    continue;
                };
                let Some(playback) = voice.event.looping else {
                    continue;
                };
                let Some(sample) = self.bank.samples[voice.event.sound.0 as usize].as_ref() else {
                    continue;
                };
                let sample_frame = match playback.phase {
                    LoopPhase::Painted => (time % sample.frames() as u64) as usize,
                    LoopPhase::Cursor => voice.frame,
                };
                let channels = sample.channels as usize;
                let offset = sample_frame * channels;
                match voice.gain {
                    Gain::Integer(gain) => {
                        for (side, total) in integer.iter_mut().enumerate() {
                            let value = sample.samples[offset + side.min(channels - 1)];
                            *total += (i64::from(value) * gain[side]) >> 8;
                        }
                    }
                    Gain::Float(gain) => {
                        has_float = true;
                        for (side, total) in floating.iter_mut().enumerate() {
                            let value = sample.samples[offset + side.min(channels - 1)];
                            *total += f32::from(value) * gain[side];
                        }
                    }
                }
                if playback.phase == LoopPhase::Cursor {
                    voice.frame = (voice.frame + 1) % sample.frames();
                }
            }
            for side in 0..2 {
                frame[side] = if has_float {
                    // Float-native paint buffers truncate at device transfer.
                    // The integer roles finish their original arithmetic shift
                    // before combination, so an inaudible float loop cannot
                    // change the existing negative one-shot sample rounding.
                    let value = floating[side] + (integer[side] >> 8) as f32;
                    value.clamp(f32::from(i16::MIN), f32::from(i16::MAX)) as i16
                } else {
                    (integer[side] >> 8).clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i16
                };
            }
        }
        self.counts.frames += (output.len() / 2) as u64;
        true
    }
    /// One bounded numeric scratch orders same-PCM groups and native emitter
    /// folds. It stores indices into the one voice array, never audio records.
    fn prepare_loops(&mut self) -> usize {
        let mut count = 0;
        for (index, voice) in self.voices.iter().enumerate() {
            if voice
                .as_ref()
                .is_some_and(|voice| voice.event.looping.is_some())
            {
                self.loop_order[count] = index;
                count += 1;
            }
        }
        let voices = &self.voices;
        self.loop_order[..count].sort_unstable_by_key(|&index| {
            let event = voices[index]
                .as_ref()
                .map_or_else(SoundEvent::default, |v| v.event);
            (loop_group(event, index), loop_order(event), index)
        });
        let mut leader: Option<usize> = None;
        for &index in &self.loop_order[..count] {
            let Some((event, gain)) = self.voices[index].as_ref().map(|v| (v.event, v.gain)) else {
                continue;
            };
            let grouped = leader.is_some_and(|first| {
                let Some(first_voice) = self.voices[first].as_mut() else {
                    return false;
                };
                loop_group(first_voice.event, first) == loop_group(event, index)
                    && first_voice.gain.add(gain)
            });
            if !grouped {
                leader = Some(index);
                if let Some(voice) = &mut self.voices[index] {
                    voice.loop_leader = true;
                }
            }
        }
        for voice in self.voices.iter_mut().flatten().filter(|v| v.loop_leader) {
            if let Some(playback) = voice.event.looping {
                let scale = match playback.master {
                    LoopGain::Integer { volume_scale, .. } => volume_scale,
                    LoopGain::Float(_) => 0,
                };
                voice.gain.prepare_loop(self.volume, scale);
            }
        }
        // Paint group leaders in native entity order, not sound registration
        // order or the voice-slot allocation order (float addition observes it).
        let voices = &self.voices;
        self.loop_order[..count].sort_unstable_by_key(|&index| {
            let event = voices[index]
                .as_ref()
                .map_or_else(SoundEvent::default, |v| v.event);
            (loop_order(event), index)
        });
        let mut leaders = 0;
        for read in 0..count {
            let index = self.loop_order[read];
            if self.voices[index]
                .as_ref()
                .is_some_and(|voice| voice.loop_leader)
            {
                self.loop_order[leaders] = index;
                leaders += 1;
            }
        }
        leaders
    }
}
fn gains(event: SoundEvent, listener: &Listener) -> Gain {
    let master = (event.volume * 255.0) as i32;
    if event.looping.is_none()
        && (event.attenuation == 0.0
            || event
                .entity
                .is_some_and(|entity| listener.entity == Some(entity)))
    {
        return Gain::Integer([i64::from(master); 2]);
    }
    let delta = event.position - listener.origin;
    let mut direction = delta;
    let distance = math::normalize(&mut direction);
    let dot = listener.right.dot(direction);
    let distance = (distance - event.spatial.distance_offset).max(0.0)
        * event.attenuation
        * event.spatial.distance_scale;
    let pan = [1.0 - dot, 1.0 + dot];
    if let Some(playback) = event.looping {
        let pan = pan.map(|pan| {
            let pan = if !playback.panning || event.spatial.distance_scale == 0.0 {
                1.0
            } else {
                event.spatial.separation * pan
            };
            if playback.clamp_pan {
                pan.max(0.0)
            } else {
                pan
            }
        });
        match playback.master {
            LoopGain::Integer { master, .. } => Gain::Integer(pan.map(|pan| {
                i64::from((f32::from(master) * ((1.0 - distance) * pan)).max(0.0) as i32)
            })),
            LoopGain::Float(master) => {
                Gain::Float(pan.map(|pan| (master * ((1.0 - distance) * pan)).max(0.0)))
            }
        }
    } else {
        Gain::Integer(pan.map(|pan| {
            i64::from(
                (master as f32 * ((1.0 - distance) * pan * event.spatial.separation)).max(0.0)
                    as i32,
            )
        }))
    }
}
