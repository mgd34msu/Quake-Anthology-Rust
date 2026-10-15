//! Load-owned PCM and voices; event admission never creates a second queue.
use qa_core::{
    math,
    names::NameTable,
    primitives::{
        ClientId, EntityId, LoopDoppler, LoopGain, LoopLifetime, LoopOrigin, LoopPhase,
        LoopPlayback, ModuleId, Pcm, ResourceNamespace, SoundAction, SoundBus, SoundEmitter,
        SoundEvent, SoundId, SoundStartOrder, Vec3,
    },
    stamps::StampSet,
};
use std::{num::NonZeroU32, sync::Arc};

pub struct Bank {
    names: NameTable,
    samples: Box<[Option<Pcm>]>,
    doppler: Box<[Option<crate::doppler::Prepared>]>,
    attempts: StampSet,
    handles: crate::SoundBindings,
    pub rate: NonZeroU32,
}
impl Bank {
    pub const DEFAULT_SOUND: &'static [u8] = b"sound/feedback/hit.wav";

    /// Reserve one staged bank before module registration. The default engine
    /// handle is zero; additional paths include both ready and failed attempts.
    pub fn with_capacity(
        rate: NonZeroU32,
        additional_paths: usize,
        additional_path_bytes: usize,
    ) -> Result<Self, String> {
        let additional_names = additional_paths
            .checked_mul(2)
            .ok_or("sound registration capacity")?;
        let additional_bytes = additional_path_bytes
            .checked_mul(2)
            .ok_or("sound registration capacity")?;
        Self::check_capacity(
            additional_names
                .checked_add(2)
                .ok_or("sound registration capacity")?,
        )?;
        let names =
            NameTable::load_reserved([Self::DEFAULT_SOUND], additional_names, additional_bytes)
                .map_err(|e| e.to_string())?;
        Self::from_names(rate, names)
    }

    fn check_capacity(capacity: usize) -> Result<(), String> {
        if capacity > i32::MAX as usize + 1 {
            Err("sound registration capacity".into())
        } else {
            Ok(())
        }
    }

    fn from_names(rate: NonZeroU32, names: NameTable) -> Result<Self, String> {
        let capacity = names.capacity();
        Self::check_capacity(capacity)?;
        let mut bank = Self {
            names,
            samples: std::iter::repeat_with(|| None).take(capacity).collect(),
            doppler: std::iter::repeat_with(|| None).take(capacity).collect(),
            attempts: StampSet::new(capacity),
            handles: crate::SoundBindings::load_reserved(capacity, capacity, true),
            rate,
        };
        // qsrc Q3 S_BeginRegistration registers this filename before any module.
        // A missing default retains handle zero but does not invent PCM.
        bank.attempt_registration(Self::DEFAULT_SOUND, Some(Self::DEFAULT_SOUND))?;
        Ok(bank)
    }

    /// Native sound ordinals are mapped to these handles at the module boundary.
    pub fn load(rate: NonZeroU32, sources: Vec<(String, Pcm)>) -> Result<Self, String> {
        let paths: Vec<_> = sources
            .iter()
            .map(|(name, _)| qa_core::names::canonical_path(name))
            .collect();
        let mut names =
            NameTable::load(paths.iter().map(|p| p.as_bytes())).map_err(|e| e.to_string())?;
        // Bulk loading keeps its existing lexical SoundIds. Native handles use
        // the same registration rows as staged loading, with zero reserved first.
        let native_bytes = sources
            .iter()
            .try_fold(Self::DEFAULT_SOUND.len(), |sum, (name, _)| {
                sum.checked_add(name.len())
            })
            .ok_or("sound registration capacity")?;
        names
            .reserve(
                sources
                    .len()
                    .checked_add(1)
                    .ok_or("sound registration capacity")?,
                native_bytes,
            )
            .map_err(|e| e.to_string())?;
        let mut bank = Self::from_names(rate, names)?;
        for ((name, pcm), path) in sources.into_iter().zip(paths) {
            let (id, _) = bank.attempt_registration(name.as_bytes(), Some(path.as_bytes()))?;
            bank.admit(id, pcm)?;
        }
        Ok(bank)
    }

    /// A name-present, PCM-absent row is a cached failed attempt after cold load.
    /// Repeated ready or failed lookups do not allocate or consume another row.
    pub fn attempted(&self, path: &[u8]) -> Option<SoundId> {
        let id = self.names.find_path_bytes(path)?;
        self.attempts
            .contains(id.0 as usize)
            .then_some(SoundId(id.0))
    }

    pub fn attempt(&mut self, path: &[u8]) -> Result<SoundId, String> {
        if path.is_empty() {
            return Err("empty sound name".into());
        }
        let id = self
            .names
            .intern_path_bytes(path)
            .map_err(|e| e.to_string())?;
        self.attempts.mark(id.0 as usize);
        Ok(SoundId(id.0))
    }

    /// S_FindName uses ASCII-folded raw filenames, so slash and backslash
    /// names retain different native handles while sharing canonical PCM.
    /// The boolean reports a new PCM attempt, not a new native alias.
    pub fn attempt_registration(
        &mut self,
        native: &[u8],
        pcm_path: Option<&[u8]>,
    ) -> Result<(SoundId, bool), String> {
        if native.is_empty() {
            return Err("empty sound name".into());
        }
        let existing = pcm_path.and_then(|path| self.attempted(path));
        let name = match self.names.find_folded(native) {
            Some(name) => name,
            None => self.names.intern(native).map_err(|e| e.to_string())?,
        };
        let (sound, first) = match pcm_path {
            Some(path) => match existing {
                Some(sound) => (sound, false),
                None => (self.attempt(path)?, true),
            },
            // The existing empty name row is the shared no-PCM sentinel; each
            // failed native key still owns its distinct forward handle row.
            None => (SoundId(0), false),
        };
        self.handles
            .register_named(name, sound)
            .map_err(|e| format!("sound handle: {e:?}"))?;
        Ok((sound, first))
    }

    /// A native format constraint can reject its handle without destroying
    /// canonical PCM that remains valid for other shared audio consumers.
    pub fn reject_registration(&mut self, native: &[u8]) -> Result<(), String> {
        let name = self
            .names
            .find_folded(native)
            .ok_or("sound registration missing")?;
        self.handles
            .replace_named(name, SoundId(0))
            .map_err(|e| format!("sound handle: {e:?}"))
    }

    /// All decoded PCM enters through the one device-rate preparation path.
    pub fn admit(&mut self, sound: SoundId, pcm: Pcm) -> Result<(), String> {
        if sound.0 == 0
            || sound.0 as usize >= self.names.len()
            || !self.attempts.contains(sound.0 as usize)
        {
            return Err("sound registration missing".into());
        }
        let sample = &mut self.samples[sound.0 as usize];
        if sample.is_some() {
            return Err("duplicate sound registration".into());
        }
        let pcm =
            crate::prepare(pcm, self.rate, false).map_err(|e| format!("sound prepare: {e:?}"))?;
        if pcm.frames() == 0 {
            return Err("empty sound".into());
        }
        self.doppler[sound.0 as usize] = Some(crate::doppler::Prepared::load(&pcm));
        *sample = Some(pcm);
        Ok(())
    }
    pub fn find(&self, path: &str) -> Option<SoundId> {
        self.find_bytes(path.as_bytes())
    }
    /// Frozen lookup returns only prepared PCM, never a failed attempted name.
    pub fn find_bytes(&self, path: &[u8]) -> Option<SoundId> {
        let sound = self.attempted(path)?;
        self.sample(sound).map(|_| sound)
    }
    pub fn sample(&self, sound: SoundId) -> Option<&Pcm> {
        self.samples.get(sound.0 as usize).and_then(Option::as_ref)
    }
    /// Failed registrations return the native default handle without changing
    /// their retained ordinal. Unattempted frozen names remain distinguishable.
    pub fn registered_handle(&self, path: &[u8]) -> Option<i32> {
        let name = self.names.find_folded(path)?;
        let ordinal = self.handles.named(name)?;
        let sound = self.handles.sound(ordinal)?;
        if self.sample(sound).is_some() {
            i32::try_from(ordinal).ok()
        } else {
            Some(0)
        }
    }
    pub fn registered_sound(&self, handle: i32) -> Option<SoundId> {
        let sound = self.handles.sound(u32::try_from(handle).ok()?)?;
        self.sample(sound).map(|_| sound)
    }
    /// Numeric registered-name access; no path reconstruction or hashing.
    pub fn name(&self, sound: SoundId) -> Option<&[u8]> {
        self.names.get(qa_core::primitives::NameId(sound.0))
    }
    pub fn has_loop_marker(&self, sound: SoundId) -> bool {
        self.sample(sound)
            .is_some_and(|pcm| pcm.loop_start.is_some())
    }
    pub fn len(&self) -> usize {
        self.names.len()
    }
    pub fn capacity(&self) -> usize {
        self.samples.len()
    }
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

pub use qa_core::primitives::Listener;
#[derive(Clone, Copy, PartialEq, Eq)]
struct ListenerOwner {
    module: ModuleId,
    client: ClientId,
    lifetime: EntityId,
}
impl ListenerOwner {
    fn from_event(event: SoundEvent) -> Option<Self> {
        Some(Self {
            module: event.listener_owner?,
            client: event.client?,
            lifetime: event.client_lifetime?,
        })
    }
}
#[derive(Clone, Copy, Default)]
struct ListenerSlot {
    listener: Option<Listener>,
    owner: Option<ListenerOwner>,
}
#[derive(Clone, Copy)]
struct LoopMix {
    sound: SoundId,
    source_ordinal: Option<u32>,
    playback: LoopPlayback,
    doppler: Option<LoopDoppler>,
    gain: Gain,
}
struct Voice {
    event: SoundEvent,
    frame: usize,
    gain: Gain,
    loop_leader: bool,
    loop_gain: Option<Gain>,
    begin: Option<u64>,
    order: u64,
    loop_registered: bool,
    loop_mix: Option<LoopMix>,
    doppler_cursor: crate::doppler::Cursor,
    paint_scale: Option<i64>,
}
impl Voice {
    fn latched(&self) -> bool {
        self.event.looping.is_some_and(|p| p.latched)
            || self.event.playback.is_some_and(|p| p.latched)
    }
    /// Only audible numeric state is latched. The current event remains the
    /// sole registration and position authority for the next Respatialize.
    fn paint_event(&self) -> Option<SoundEvent> {
        if self.event.looping.is_some_and(|p| p.latched) {
            let mix = self.loop_mix?;
            Some(SoundEvent {
                sound: mix.sound,
                source_ordinal: mix.source_ordinal,
                looping: Some(mix.playback),
                doppler: mix.doppler,
                ..self.event
            })
        } else {
            Some(self.event)
        }
    }
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
fn same_client(a: &SoundEvent, b: &SoundEvent) -> bool {
    a.client == b.client && a.client_lifetime == b.client_lifetime
}
enum ChannelChoice {
    Slot(usize),
    Suppressed,
    Full,
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
fn loop_group(
    event: SoundEvent,
    index: usize,
) -> (u32, u32, u32, (u32, Option<u32>, bool), u16, usize) {
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
    let handle = event
        .looping
        .filter(|p| p.latched)
        .and(event.source_ordinal);
    let cue_extent = event.looping.is_some_and(|playback| playback.cue_extent);
    (
        client,
        slot,
        generation,
        (event.sound.0, handle, cue_extent),
        scale,
        separate,
    )
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
    pub suppressed: u64,
}
pub struct Mixer {
    pub bank: Arc<Bank>,
    voices: Box<[Option<Voice>]>,
    listeners: Box<[ListenerSlot]>,
    loop_order: Box<[usize]>,
    pub counts: MixCounts,
    volume: f32,
    next_start: Option<u64>,
    raw: crate::raw::Raw,
}
impl Mixer {
    pub fn load(bank: Arc<Bank>, voices: usize, listeners: usize) -> Self {
        Self {
            bank,
            voices: std::iter::repeat_with(|| None).take(voices).collect(),
            listeners: vec![ListenerSlot::default(); listeners].into_boxed_slice(),
            loop_order: vec![0; voices].into_boxed_slice(),
            counts: MixCounts::default(),
            volume: 1.0,
            next_start: None,
            raw: crate::raw::Raw::load(),
        }
    }
    /// A streaming producer retains source bytes and its own decoder cursor.
    /// Only complete blocks that fit the fixed raw paint storage are admitted.
    pub fn queue_raw(
        &mut self,
        format: qa_core::primitives::PcmFormat,
        bytes: &[u8],
        volume: f32,
        signed_stereo_bytes: bool,
    ) -> Result<usize, crate::RawError> {
        self.raw.queue(
            self.counts.frames,
            self.bank.rate,
            format,
            bytes,
            volume,
            signed_stereo_bytes,
        )
    }
    pub fn raw_remaining(&self) -> usize {
        self.raw.remaining(self.counts.frames)
    }
    pub fn clear_raw(&mut self) {
        self.raw.clear(self.counts.frames);
    }
    pub fn pause_raw(&mut self, paused: bool) {
        self.raw.pause(paused);
    }
    pub fn listen(&mut self, listeners: &[Option<Listener>], volume: f32) {
        for index in 0..self.listeners.len() {
            if self.listeners[index].owner.is_some() {
                continue;
            }
            let mut listener = listeners.get(index).copied().flatten();
            // The attached CLIENT owns its listener until an exact release,
            // even when the host has no active view for that connection.
            if listener.is_some_and(|candidate| {
                self.listeners.iter().any(|slot| {
                    slot.owner
                        .is_some_and(|owner| owner.client == candidate.client)
                })
            }) {
                listener = None;
            }
            // Repeated views of one client are one audible listener.
            if listener.is_some_and(|l| {
                listeners[..index.min(listeners.len())]
                    .iter()
                    .flatten()
                    .any(|earlier| earlier.client == l.client)
            }) {
                listener = None;
            }
            self.listeners[index].listener = listener;
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
            if voice.event.positioned || voice.latched() {
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
            if event.action == SoundAction::RetireClient {
                for slot in &mut self.listeners {
                    if slot
                        .owner
                        .is_some_and(|owner| owner.client == client && owner.lifetime == lifetime)
                    {
                        *slot = ListenerSlot::default();
                    }
                }
            }
            return true;
        }
        if !self.owner_allows(event) {
            self.counts.invalid += 1;
            return false;
        }
        if matches!(
            event.action,
            SoundAction::ClearTemporaryLoops
                | SoundAction::ClearClientLoops
                | SoundAction::UpdateEmitterPosition
                | SoundAction::Respatialize
                | SoundAction::ReleaseListener
        ) {
            return self.control(event);
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
                    if event.action == SoundAction::StopLoop
                        && slot.as_ref().is_some_and(Voice::latched)
                    {
                        if let Some(voice) = slot {
                            voice.loop_registered = false;
                            if voice.loop_mix.is_none() {
                                *slot = None;
                            }
                        }
                    } else {
                        *slot = None;
                    }
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
            || self.bank.sample(event.sound).is_some_and(|sample| {
                sample.loop_invalid
                    && event
                        .looping
                        .map_or(event.playback.is_none(), |playback| playback.cue_extent)
            })
            || !event.volume.is_finite()
            || !(0.0..=1.0).contains(&event.volume)
            || !event.attenuation.is_finite()
            || event.attenuation < 0.0
            || !event.time_offset.is_finite()
            || event.time_offset < 0.0
            || !event.position.is_finite()
            || !event.velocity.is_finite()
            || event.doppler.is_some_and(|rate| {
                event.looping.is_none()
                    || !rate.scale.is_finite()
                    || rate.scale <= 1.
                    || !rate.previous_scale.is_finite()
                    || rate.previous_scale <= 0.
            })
            || event.playback.is_some_and(|playback| {
                event.looping.is_some()
                    || event.action != SoundAction::Play
                    || playback.volume_scale == 0
                    || event.time_offset != 0.0
            })
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
        if event.looping.is_some_and(|p| p.latched) {
            self.update_emitter_position(event);
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
                && (voice.event.sound == event.sound || voice.latched())
                && voice.begin.is_none()
            {
                voice.event = event;
                voice.loop_registered = true;
                return true;
            }
        }
        let delay = (event.time_offset * self.bank.rate.get() as f32) as u64;
        let queued = event.action == SoundAction::Play
            && event.spatial.start_order == SoundStartOrder::QueuedNewestFirst;
        let begin = (delay != 0 || queued).then(|| self.counts.frames.saturating_add(delay));
        let replacement = if event.looping.is_some() {
            self.voices.iter().position(|voice| {
                voice
                    .as_ref()
                    .is_some_and(|voice| same_loop(&voice.event, &event))
            })
        } else if begin.is_none() {
            if event.playback.is_some_and(|p| p.channels.is_some()) {
                match self.native_channel(event) {
                    ChannelChoice::Slot(index) => Some(index),
                    ChannelChoice::Suppressed => {
                        self.counts.suppressed += 1;
                        return true;
                    }
                    ChannelChoice::Full => {
                        self.counts.full += 1;
                        return false;
                    }
                }
            } else {
                self.channel_voice(event)
            }
        } else {
            None
        };
        let Some(index) = replacement.or_else(|| self.voices.iter().position(Option::is_none))
        else {
            self.counts.full += 1;
            return false;
        };
        self.counts.replaced += u64::from(self.voices[index].is_some());
        self.voices[index] = Some(Voice {
            event,
            frame: 0,
            gain: if event.playback.is_some_and(|p| p.latched) {
                full_gain(event)
            } else {
                Gain::zero(event)
            },
            loop_leader: false,
            loop_gain: None,
            begin,
            order: self.counts.started,
            loop_registered: true,
            loop_mix: None,
            doppler_cursor: crate::doppler::Cursor::default(),
            paint_scale: None,
        });
        if let Some(begin) = begin {
            self.next_start = Some(self.next_start.map_or(begin, |next| next.min(begin)));
        }
        self.counts.started += 1;
        true
    }
    fn owner_allows(&self, event: SoundEvent) -> bool {
        let Some(owner) = ListenerOwner::from_event(event) else {
            // Unbound UI Local audio has no CLIENT listener ownership.
            return event.listener_owner.is_none() || event.client.is_none();
        };
        self.listeners.iter().all(|slot| {
            slot.owner
                .is_none_or(|current| current.client != owner.client || current == owner)
        })
    }
    fn control(&mut self, event: SoundEvent) -> bool {
        let Some(owner) = ListenerOwner::from_event(event) else {
            self.counts.invalid += 1;
            return false;
        };
        if event.action == SoundAction::ReleaseListener {
            if let Some(slot) = self
                .listeners
                .iter_mut()
                .find(|slot| slot.owner == Some(owner))
            {
                *slot = ListenerSlot::default();
            }
            // An attached module may fail before its first Respatialize.
            // Exact release remains an idempotent cleanup boundary.
            return true;
        }
        if event.action == SoundAction::Respatialize {
            let Some(listener) = event.listener.filter(|listener| {
                listener.client == owner.client
                    && listener.origin.is_finite()
                    && listener.right.is_finite()
            }) else {
                self.counts.invalid += 1;
                return false;
            };
            let slot = self
                .listeners
                .iter()
                .position(|slot| slot.owner == Some(owner))
                .or_else(|| {
                    self.listeners.iter().position(|slot| {
                        slot.owner.is_none()
                            && slot.listener.is_some_and(|l| l.client == owner.client)
                    })
                })
                .or_else(|| {
                    self.listeners
                        .iter()
                        .position(|slot| slot.owner.is_none() && slot.listener.is_none())
                });
            let Some(index) = slot else {
                self.counts.full += 1;
                return false;
            };
            self.listeners[index] = ListenerSlot {
                listener: Some(listener),
                owner: Some(owner),
            };
            for slot in &mut self.voices {
                let Some(voice) = slot else { continue };
                if !same_client(&voice.event, &event) || !voice.latched() {
                    continue;
                }
                if let Some(playback) = voice.event.looping {
                    if voice.loop_registered {
                        voice.loop_mix = Some(LoopMix {
                            sound: voice.event.sound,
                            source_ordinal: voice.event.source_ordinal,
                            playback,
                            doppler: voice.event.doppler,
                            gain: gains(voice.event, &listener),
                        });
                    } else {
                        *slot = None;
                    }
                } else {
                    voice.gain = gains(voice.event, &listener);
                }
            }
            return true;
        }
        if event.action == SoundAction::UpdateEmitterPosition {
            if !event.position.is_finite()
                || !matches!(event.entity, Some(SoundEmitter::Native {
                namespace: ResourceNamespace::Client(client), slot
            }) if client == owner.client && slot >= 0)
            {
                self.counts.invalid += 1;
                return false;
            }
            self.update_emitter_position(event);
            return true;
        }
        for slot in &mut self.voices {
            let Some(voice) = slot else { continue };
            let Some(playback) = voice.event.looping.filter(|p| p.latched) else {
                continue;
            };
            if !same_client(&voice.event, &event) {
                continue;
            }
            // Both native clear modes immediately mute the audible loop mix.
            voice.loop_mix = None;
            if event.action == SoundAction::ClearClientLoops
                || playback.lifetime == LoopLifetime::Temporary
            {
                *slot = None;
                self.counts.stopped += 1;
            }
        }
        true
    }
    fn update_emitter_position(&mut self, event: SoundEvent) {
        for voice in self.voices.iter_mut().flatten() {
            if voice.latched()
                && same_client(&voice.event, &event)
                && voice.event.entity == event.entity
                && (voice.event.looping.is_some() || !voice.event.positioned)
            {
                voice.event.position = event.position;
            }
        }
    }
    fn is_listener(&self, event: SoundEvent) -> bool {
        event.bus == SoundBus::Local
            || self.listeners.iter().any(|slot| {
                slot.listener.is_some_and(|listener| {
                    event.applies_to_client(listener.client)
                        && event
                            .entity
                            .is_some_and(|entity| listener.entity == Some(entity))
                })
            })
    }
    /// Native per-emitter limits and strict-oldest selection operate on the
    /// existing bounded channel table. No out-of-bounds reference is copied.
    fn native_channel(&self, event: SoundEvent) -> ChannelChoice {
        let Some(channels) = event.playback.and_then(|p| p.channels) else {
            return ChannelChoice::Full;
        };
        let now = event.queued_at.milliseconds();
        let mut in_play = 0usize;
        for voice in self.voices.iter().flatten().filter(|voice| {
            voice.event.looping.is_none()
                && same_client(&voice.event, &event)
                && voice.event.entity == event.entity
                && voice.event.sound == event.sound
                && voice.event.source_ordinal == event.source_ordinal
        }) {
            if now.saturating_sub(voice.event.queued_at.milliseconds())
                < u64::from(channels.suppress_ms)
            {
                return ChannelChoice::Suppressed;
            }
            in_play += 1;
        }
        let allowed = if self.is_listener(event) {
            channels.listener_limit
        } else {
            channels.emitter_limit
        };
        if in_play > usize::from(allowed) {
            return ChannelChoice::Suppressed;
        }
        if let Some(index) = self.voices.iter().position(Option::is_none) {
            return ChannelChoice::Slot(index);
        }
        for pass in 0..3 {
            if pass == 2 && !self.is_listener(event) {
                break;
            }
            let mut oldest = now;
            let mut chosen = None;
            for (index, voice) in self.voices.iter().enumerate() {
                let Some(voice) = voice else { continue };
                if voice.event.looping.is_some() || !same_client(&voice.event, &event) {
                    continue;
                }
                if pass < 2
                    && (self.is_listener(voice.event)
                        || voice.event.channel == channels.protected_channel)
                {
                    continue;
                }
                if pass == 0 && voice.event.entity != event.entity {
                    continue;
                }
                let allocated = voice.event.queued_at.milliseconds();
                if allocated < oldest {
                    oldest = allocated;
                    chosen = Some(index);
                }
            }
            if let Some(index) = chosen {
                return ChannelChoice::Slot(index);
            }
        }
        ChannelChoice::Full
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
                    let order = voice.event.spatial.start_order;
                    // Distinct scheduling capabilities have a fixed tie order;
                    // each retains its own publication rule within that group.
                    Some((
                        voice.begin?,
                        order,
                        order.publication_key(voice.order),
                        index,
                    ))
                })
                .min();
            let Some((begin, _, _, index)) = next else {
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
            voice.loop_leader = false;
            voice.loop_gain = None;
            voice.doppler_cursor.reset();
            voice.paint_scale = voice
                .event
                .playback
                .map(|playback| (self.volume * f32::from(playback.volume_scale)) as i64);
            if voice.event.looping.is_some_and(|p| p.latched) {
                voice.gain = voice
                    .loop_mix
                    .map_or_else(|| Gain::zero(voice.event), |mix| mix.gain);
                continue;
            }
            if voice.event.playback.is_some_and(|p| p.latched) {
                continue;
            }
            voice.gain = Gain::zero(voice.event);
            if voice.event.bus == SoundBus::Local {
                voice.gain = full_gain(voice.event);
                continue;
            }
            let mut listeners = 0;
            for listener in self
                .listeners
                .iter()
                .filter_map(|slot| slot.listener.as_ref())
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
            let mut native = [0i64; 2];
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
                    let totals = if voice.paint_scale.is_some() {
                        &mut native
                    } else {
                        &mut sum
                    };
                    for (side, total) in totals.iter_mut().enumerate() {
                        let value = sample.samples[offset + side.min(channels - 1)];
                        *total +=
                            (i64::from(value) * (gain[side] * voice.paint_scale.unwrap_or(1))) >> 8;
                    }
                }
                voice.frame += 1;
                let frames = if voice.event.playback.is_some() {
                    sample.frames()
                } else {
                    sample.cue_frames()
                };
                if voice.frame == frames {
                    let start = if voice.event.playback.is_some() {
                        None
                    } else {
                        sample
                            .loop_start
                            .or_else(|| (voice.event.action == SoundAction::StartLoop).then_some(0))
                    };
                    if let Some(start) = start {
                        voice.frame = start;
                    } else {
                        *slot = None;
                    }
                }
            }
            // Native integer loops multiply their grouped volume before the
            // sample product. Ordinary channels retain their existing order.
            let raw = self.raw.sample(time);
            let mut integer = std::array::from_fn::<_, 2, _>(|side| {
                i64::from(raw[side]) + sum[side] * volume + native[side]
            });
            let mut floating = [0.0f32; 2];
            let mut has_float = false;
            for &index in &self.loop_order[..loops] {
                let Some(voice) = self.voices[index].as_mut() else {
                    continue;
                };
                let Some(event) = voice.paint_event() else {
                    continue;
                };
                let Some(playback) = event.looping else {
                    continue;
                };
                let Some(sample) = self.bank.samples[event.sound.0 as usize].as_ref() else {
                    continue;
                };
                let frames = if playback.cue_extent {
                    sample.cue_frames()
                } else {
                    sample.frames()
                };
                let sample_frame = match playback.phase {
                    LoopPhase::Painted => (time % frames as u64) as usize,
                    LoopPhase::Cursor => voice.frame % frames,
                };
                let channels = sample.channels as usize;
                let offset = sample_frame * channels;
                let doppler = event.doppler.and_then(|rate| {
                    self.bank.doppler[event.sound.0 as usize]
                        .as_ref()
                        .map(|prepared| {
                            voice
                                .doppler_cursor
                                .sample(sample, prepared, sample_frame, rate)
                        })
                });
                match voice.loop_gain.unwrap_or(voice.gain) {
                    Gain::Integer(gain) => {
                        for (side, total) in integer.iter_mut().enumerate() {
                            if let Some(sample) = &doppler {
                                *total = sample.add_integer(*total, side, gain[side]);
                            } else {
                                let value = sample.samples[offset + side.min(channels - 1)];
                                *total += (i64::from(value) * gain[side]) >> 8;
                            }
                        }
                    }
                    Gain::Float(gain) => {
                        has_float = true;
                        for (side, total) in floating.iter_mut().enumerate() {
                            *total += if let Some(sample) = &doppler {
                                sample.floating(side, gain[side])
                            } else {
                                let value = sample.samples[offset + side.min(channels - 1)];
                                f32::from(value) * gain[side]
                            };
                        }
                    }
                }
                if playback.phase == LoopPhase::Cursor {
                    voice.frame = (sample_frame + 1) % frames;
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
            if voice.as_ref().is_some_and(|voice| {
                voice
                    .paint_event()
                    .is_some_and(|event| event.looping.is_some())
            }) {
                self.loop_order[count] = index;
                count += 1;
            }
        }
        let voices = &self.voices;
        self.loop_order[..count].sort_unstable_by_key(|&index| {
            let event = voices[index]
                .as_ref()
                .and_then(Voice::paint_event)
                .unwrap_or_default();
            (loop_group(event, index), loop_order(event), index)
        });
        let mut first = 0;
        while first < count {
            let first_index = self.loop_order[first];
            let first_event = self.voices[first_index]
                .as_ref()
                .and_then(Voice::paint_event)
                .unwrap_or_default();
            let key = loop_group(first_event, first_index);
            let mut end = first + 1;
            while end < count {
                let index = self.loop_order[end];
                let event = self.voices[index]
                    .as_ref()
                    .and_then(Voice::paint_event)
                    .unwrap_or_default();
                if loop_group(event, index) != key {
                    break;
                }
                end += 1;
            }
            // S_AddLoopSounds excludes a Doppler candidate, not a Doppler
            // leader. Each leader may therefore fold the same later plain
            // contribution; original per-voice gains must remain available.
            for position in first..end {
                let index = self.loop_order[position];
                let Some(voice) = self.voices[index].as_ref() else {
                    continue;
                };
                let Some(event) = voice.paint_event() else {
                    continue;
                };
                if position != first && event.doppler.is_none() {
                    continue;
                }
                let mut gain = voice.gain;
                for &candidate in &self.loop_order[position + 1..end] {
                    if let Some(voice) = self.voices[candidate].as_ref()
                        && voice
                            .paint_event()
                            .is_some_and(|event| event.doppler.is_none())
                    {
                        gain.add(voice.gain);
                    }
                }
                if let Some(playback) = event.looping {
                    let scale = match playback.master {
                        LoopGain::Integer { volume_scale, .. } => volume_scale,
                        LoopGain::Float(_) => 0,
                    };
                    gain.prepare_loop(self.volume, scale);
                }
                if let Some(voice) = self.voices[index].as_mut() {
                    voice.loop_leader = true;
                    voice.loop_gain = Some(gain);
                }
            }
            first = end;
        }
        // Paint group leaders in native entity order, not sound registration
        // order or the voice-slot allocation order (float addition observes it).
        let voices = &self.voices;
        self.loop_order[..count].sort_unstable_by_key(|&index| {
            let event = voices[index]
                .as_ref()
                .and_then(Voice::paint_event)
                .unwrap_or_default();
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
fn full_gain(event: SoundEvent) -> Gain {
    let master = event
        .playback
        .map_or(255.0, |playback| f32::from(playback.master));
    Gain::Integer([i64::from((event.volume * master) as i32); 2])
}
fn gains(event: SoundEvent, listener: &Listener) -> Gain {
    let master = (event.volume * event.playback.map_or(255.0, |p| f32::from(p.master))) as i32;
    if event.looping.is_none()
        && (event.bus == SoundBus::Local
            || event.attenuation == 0.0
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
    let dot = if listener.negate_dot { -dot } else { dot };
    let (distance, no_distance_panning) = event.spatial.attenuation.apply(
        (distance - event.spatial.distance_offset).max(0.0),
        event.attenuation,
    );
    let pan = [1.0 - dot, 1.0 + dot];
    if let Some(playback) = event.looping {
        let pan = pan.map(|pan| {
            let pan = if !playback.panning || no_distance_panning {
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
            let attenuated = if event.playback.is_some_and(|p| p.clamp_pan) {
                (1.0 - distance) * (event.spatial.separation * pan).max(0.0)
            } else {
                (1.0 - distance) * pan * event.spatial.separation
            };
            i64::from((master as f32 * attenuated).max(0.0) as i32)
        }))
    }
}
