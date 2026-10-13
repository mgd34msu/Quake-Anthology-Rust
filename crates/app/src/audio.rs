//! The client presentation consumer owns one mixer and one platform stream.
use crate::Runtime;
use qa_audio::{Bank, Mixer};
use qa_console::{
    commands::{CommandError, Console},
    views::Context,
};
use qa_core::{
    events::FrameEvent,
    primitives::{RuleSetId, SoundAction, SoundEvent, Vec3},
};
use std::{num::NonZeroU32, sync::Arc};

/// Client cue recipes hold numeric bindings into the one PCM bank.
pub(crate) struct MuzzleSounds {
    bindings: qa_audio::SoundBindings,
    random: qa_core::random::Random,
}
impl Default for MuzzleSounds {
    fn default() -> Self {
        Self {
            bindings: qa_audio::SoundBindings::load(
                qa_gameplay::rules::q2::sounds::PATHS.len() + 1,
            ),
            random: qa_core::random::Random::new(5489),
        }
    }
}
impl MuzzleSounds {
    pub(crate) fn bind(&mut self, bank: &Bank) -> Result<(), qa_audio::SoundBindingError> {
        self.bindings.attach(bank);
        for (index, path) in qa_gameplay::rules::q2::sounds::PATHS.iter().enumerate() {
            self.bindings
                .bind_path(Some(bank), index as u32 + 1, path.as_bytes())?;
        }
        Ok(())
    }
    pub(crate) fn choose(
        &mut self,
        recipe: qa_gameplay::rules::q2::sounds::MuzzleSound,
    ) -> Option<qa_core::primitives::SoundId> {
        let offset = if recipe.choices.count > 1 {
            self.random.next_u32() % recipe.choices.count
        } else {
            0
        };
        self.bindings.sound(recipe.choices.first + offset + 1)
    }
}

/// Shared CLIENT poses are sampled once before bounded PCM mixahead.
/// A sound's native namespace selects its owning connection, not a seat slot.
pub fn respatialize(
    mixer: &mut Mixer,
    entities: &[Option<crate::client_scene::ClientEntities<'_>>;
         qa_core::sys_events::SeatId::COUNT],
) {
    mixer.respatialize(|client, number, origin| {
        entities
            .iter()
            .flatten()
            .find(|entities| entities.client == client)?
            .sound_origin(number, origin)
    });
}

pub struct Output {
    pub mixer: Mixer,
    stream: qa_platform::AudioStream,
    pcm: Box<[i16]>,
    pending: bool,
    target_bytes: usize,
    pub submissions: u64,
    pub blocked: u64,
}
impl Output {
    pub fn open(bank: Arc<Bank>) -> Result<Self, String> {
        let rate = bank.rate;
        let stream = qa_platform::AudioStream::open(rate, 2)?;
        Ok(Self {
            mixer: Mixer::load(bank, 128, qa_core::sys_events::SeatId::COUNT),
            stream,
            pcm: vec![0; 2048].into_boxed_slice(),
            pending: false,
            target_bytes: rate.get() as usize / 20 * 4,
            submissions: 0,
            blocked: 0,
        })
    }
    /// Bounded mixahead; a failed write retains the exact PCM for retry.
    pub fn submit(
        &mut self,
        entities: &[Option<crate::client_scene::ClientEntities<'_>>;
             qa_core::sys_events::SeatId::COUNT],
    ) {
        respatialize(&mut self.mixer, entities);
        for _ in 0..8 {
            if !self.pending {
                let Some(queued) = self.stream.queued_bytes() else {
                    self.blocked += 1;
                    return;
                };
                if queued >= self.target_bytes {
                    return;
                }
                self.mixer.paint(&mut self.pcm);
                self.pending = true;
            }
            if !self.stream.write(&self.pcm) {
                self.blocked += 1;
                return;
            }
            self.pending = false;
            self.submissions += 1;
        }
    }
}

/// One cold registration owner. Names, failed attempts and native handles all
/// remain in the Bank that is moved unchanged into the frozen presentation.
pub struct SoundLoader {
    bank: Bank,
    reader: qa_formats::archive::ArchiveReader,
    failures: Box<[Option<SoundFailure>]>,
    pub reads: u64,
    pub decodes: u64,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum FailureKind {
    File,
    Decode,
}
struct SoundFailure {
    kind: FailureKind,
    message: String,
}
struct Attempt {
    sound: qa_core::primitives::SoundId,
    first: bool,
}
impl SoundLoader {
    /// Extra paths and bytes are reserved once before any module Init. The
    /// native default owns handle zero even when its actual PCM is absent.
    pub fn load(additional_paths: usize, additional_path_bytes: usize) -> Result<Self, String> {
        let rate = NonZeroU32::new(44100).ok_or("invalid device rate")?;
        let bank = Bank::with_capacity(rate, additional_paths, additional_path_bytes)?;
        let failures = std::iter::repeat_with(|| None)
            .take(bank.capacity())
            .collect();
        Ok(Self {
            bank,
            reader: qa_formats::archive::ArchiveReader::default(),
            failures,
            reads: 0,
            decodes: 0,
        })
    }
    pub fn bank(&self) -> &Bank {
        &self.bank
    }
    pub fn finish(mut self) -> Arc<Bank> {
        // Other source roles may already have loaded the default PCM. Native
        // handles keep Q3's mono restriction even without a cold cgame call.
        if let Some(sound) = self.bank.find_bytes(Bank::DEFAULT_SOUND) {
            let _ = self.validate_registration(Bank::DEFAULT_SOUND, sound, RuleSetId::Quake3);
        }
        Arc::new(self.bank)
    }
    fn read(&mut self, vfs: &qa_content::vfs::Vfs, sound: qa_core::primitives::SoundId) {
        // The path is borrowed from the sole canonical name row. No second
        // normalized path or PCM catalog is built by this loader.
        let Some(path) = self.bank.name(sound) else {
            return;
        };
        self.reads += 1;
        let bytes = match vfs.read_file(path, 256 * 1024 * 1024, &mut self.reader) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.failures[sound.0 as usize] = Some(SoundFailure {
                    kind: FailureKind::File,
                    message: format!("sound {}: {error}", String::from_utf8_lossy(path)),
                });
                return;
            }
        };
        self.decodes += 1;
        let result = qa_formats::sound::decode(&bytes, qa_formats::sound::WavPolicy::Shared)
            .map_err(|error| format!("sound decode: {error:?}"))
            .and_then(|pcm| self.bank.admit(sound, pcm));
        if let Err(message) = result {
            self.failures[sound.0 as usize] = Some(SoundFailure {
                kind: FailureKind::Decode,
                message,
            });
        }
    }
    fn attempt(
        &mut self,
        vfs: &qa_content::vfs::Vfs,
        path: &[u8],
    ) -> Result<Attempt, qa_compat::services::CallError> {
        if let Some(sound) = self.bank.attempted(path) {
            // The default name is reserved before its first load. Completed
            // attempts always contain either PCM or a retained failure.
            let first =
                self.bank.sample(sound).is_none() && self.failures[sound.0 as usize].is_none();
            if first {
                self.read(vfs, sound);
            }
            return Ok(Attempt { sound, first });
        }
        let sound = self
            .bank
            .attempt(path)
            .map_err(|_| qa_compat::services::CallError::Capacity)?;
        self.read(vfs, sound);
        Ok(Attempt { sound, first: true })
    }
    /// Explicit precaches are full VFS paths. Optional missing files warn once;
    /// malformed PCM and required missing files retain the previous fatal rule.
    pub fn precache_path(
        &mut self,
        vfs: &qa_content::vfs::Vfs,
        path: &[u8],
        source: RuleSetId,
        required: bool,
    ) -> Result<Option<String>, String> {
        let attempt = self
            .attempt(vfs, path)
            .map_err(|error| format!("sound registration: {error:?}"))?;
        if self.invalid_loops(attempt.sound, source) {
            return Err("sound decode: invalid native loop metadata".into());
        }
        match self.failures[attempt.sound.0 as usize].as_ref() {
            Some(failure) if required || failure.kind == FailureKind::Decode => {
                Err(failure.message.clone())
            }
            Some(failure) if attempt.first => Ok(Some(failure.message.clone())),
            _ => Ok(None),
        }
    }
    fn default_warning(&self) -> Option<&str> {
        let sound = self.bank.attempted(Bank::DEFAULT_SOUND)?;
        if self.bank.sample(sound).is_some() && self.bank.registered_sound(0).is_none() {
            return Some("native Q3 default sound rejected");
        }
        self.failures[sound.0 as usize]
            .as_ref()
            .map(|failure| failure.message.as_str())
    }
    fn validate_registration(
        &mut self,
        native: &[u8],
        sound: qa_core::primitives::SoundId,
        source: RuleSetId,
    ) -> Result<(), qa_compat::services::CallError> {
        use qa_compat::services::CallError;
        let pcm = self.bank.sample(sound).ok_or(CallError::File)?;
        if self.invalid_loops(sound, source) {
            return Err(CallError::File);
        }
        if source == RuleSetId::Quake3 && pcm.channels != qa_core::primitives::PcmChannels::Mono {
            // Native Q3 S_LoadSound rejects stereo effects. Its failed handle
            // stays occupied while common stereo PCM remains available.
            self.bank
                .reject_registration(native)
                .map_err(|_| CallError::Capacity)?;
            return Err(CallError::File);
        }
        Ok(())
    }
    fn invalid_loops(&self, sound: qa_core::primitives::SoundId, source: RuleSetId) -> bool {
        source != RuleSetId::Quake3 && self.bank.sample(sound).is_some_and(|pcm| pcm.loop_invalid)
    }
    fn precache_native(
        &mut self,
        vfs: &qa_content::vfs::Vfs,
        sample: &[u8],
        source: RuleSetId,
    ) -> Result<Option<String>, String> {
        let Some(path) = qa_audio::sound_path(sample, source)
            .map_err(|error| format!("sound path: {error:?}"))?
        else {
            return Ok(None);
        };
        self.precache_path(vfs, path.as_bytes(), source, false)
    }
}
impl qa_compat::services::SoundRegistrar for SoundLoader {
    fn register(
        &mut self,
        vfs: &qa_content::vfs::Vfs,
        sample: &[u8],
        source: RuleSetId,
    ) -> Result<i32, qa_compat::services::CallError> {
        use qa_compat::services::CallError;
        if source == RuleSetId::Quake3 {
            let default = self.attempt(vfs, Bank::DEFAULT_SOUND)?;
            // A missing/rejected default does not prevent another native sample.
            let _ = self.validate_registration(Bank::DEFAULT_SOUND, default.sound, source);
        }
        if source == RuleSetId::Quake3 && sample.starts_with(b"*") {
            // S_FindName admits a handle before S_LoadSound rejects player-
            // specific names, without reading a literal '*' file.
            self.bank
                .attempt_registration(sample, None)
                .map_err(|_| CallError::Capacity)?;
            return Err(CallError::File);
        }
        let Some(path) = qa_audio::sound_path(sample, source).map_err(|_| CallError::Text)? else {
            return Ok(0);
        };
        self.bank
            .attempt_registration(sample, Some(path.as_bytes()))
            .map_err(|_| CallError::Capacity)?;
        let attempt = self.attempt(vfs, path.as_bytes())?;
        self.validate_registration(sample, attempt.sound, source)?;
        self.bank.registered_handle(sample).ok_or(CallError::File)
    }
    fn precache(
        &mut self,
        vfs: &qa_content::vfs::Vfs,
        sample: &[u8],
        source: RuleSetId,
    ) -> Result<Option<qa_core::primitives::SoundId>, qa_compat::services::CallError> {
        use qa_compat::services::CallError;
        let Some(path) = qa_audio::sound_path(sample, source).map_err(|_| CallError::Text)? else {
            return Ok(None);
        };
        let attempt = self.attempt(vfs, path.as_bytes())?;
        if self.invalid_loops(attempt.sound, source) {
            return Err(CallError::File);
        }
        self.bank
            .sample(attempt.sound)
            .map(|_| Some(attempt.sound))
            .ok_or(CallError::File)
    }
    fn bank(&self) -> &Bank {
        &self.bank
    }
}

/// Additional failed native names are bounded cold registration capacity, not
/// a restriction on configstring ordinals or the combined native namespaces.
const FAILED_NATIVE_NAMES: usize = 4096;

pub fn begin_registration(
    host: &mut crate::host::FrameHost,
    paths: &[String],
    rules: RuleSetId,
) -> Result<(), String> {
    if host.runtime.sound_loader.is_some() || host.runtime.sound_bank.is_some() {
        return Err("sound registration already started".into());
    }
    let mut count = FAILED_NATIVE_NAMES;
    let mut bytes = FAILED_NATIVE_NAMES * 70;
    for path in host
        .runtime
        .vfs
        .files()
        .map(|(_, path)| path)
        .chain(paths.iter().map(|path| path.as_bytes()))
        .chain(
            qa_gameplay::rules::q2::sounds::PATHS
                .iter()
                .map(|path| path.as_bytes()),
        )
    {
        count = count.checked_add(1).ok_or("sound registration capacity")?;
        bytes = bytes.checked_add(path.len()).ok_or("sound path capacity")?;
    }
    let mut loader = SoundLoader::load(count, bytes)?;
    for path in paths {
        loader.precache_path(&host.runtime.vfs, path.as_bytes(), rules, true)?;
    }
    host.runtime.sound_loader = Some(Box::new(loader));
    Ok(())
}

pub fn load_registered_sounds(
    host: &mut crate::host::FrameHost,
    paths: &[String],
    rules: RuleSetId,
) -> Result<(), String> {
    let mut loader = host
        .runtime
        .sound_loader
        .take()
        .ok_or("sound registration not started")?;
    let mut warnings = Vec::new();
    let result = (|| {
        for path in paths {
            loader.precache_path(&host.runtime.vfs, path.as_bytes(), rules, true)?;
        }
        host.for_each_native_sound(|sample, source| {
            if let Some(warning) = loader.precache_native(&host.runtime.vfs, sample, source)? {
                warnings.push(warning);
            }
            Ok(())
        })?;
        // Native cue sources, not map or movement roles, select this cold
        // recipe admission. A foreign client protocol can still receive cues.
        let q2_cues = host.runtime.snapshots.iter().flatten().any(|client| {
            matches!(
                client.protocol,
                qa_network::commands::packet::Protocol::Quake2_34
                    | qa_network::commands::packet::Protocol::Quake2Repro1038
            )
        }) || host.modules.as_ref().is_some_and(|modules| {
            modules.has_sound_source(RuleSetId::Quake2)
                || modules.has_sound_source(RuleSetId::Quake2Rerelease)
        });
        if q2_cues {
            for path in qa_gameplay::rules::q2::sounds::PATHS {
                if let Some(warning) = loader.precache_path(
                    &host.runtime.vfs,
                    path.as_bytes(),
                    RuleSetId::Quake2,
                    false,
                )? {
                    warnings.push(warning);
                }
            }
        }
        Ok(())
    })();
    host.runtime.sound_loader = Some(loader);
    for warning in warnings {
        host.runtime.print_event(
            None,
            qa_core::primitives::PrintKind::Console,
            format_args!("{warning}\n"),
        );
    }
    result
}

pub fn finish_registration(host: &mut crate::host::FrameHost) -> Result<Arc<Bank>, String> {
    let loader = host
        .runtime
        .sound_loader
        .take()
        .ok_or("sound registration not started")?;
    if let Some(warning) = loader.default_warning() {
        host.runtime.print_event(
            None,
            qa_core::primitives::PrintKind::Console,
            format_args!("{warning}\n"),
        );
    }
    let bank = loader.finish();
    host.runtime.sound_bank = Some(bank.clone());
    host.bind_sounds(&bank)?;
    Ok(bank)
}

pub fn load_bank(
    host: &mut crate::host::FrameHost,
    paths: &[String],
    rules: RuleSetId,
) -> Result<Arc<Bank>, String> {
    begin_registration(host, paths, rules)?;
    load_registered_sounds(host, paths, rules)?;
    finish_registration(host)
}

pub fn play(
    _: &mut Console<Runtime>,
    runtime: &mut Runtime,
    args: &qa_console::command_text::Arguments<'_>,
    context: Context,
) -> Result<(), CommandError> {
    if args.len() < 2 {
        return Err(CommandError::Usage);
    }
    for path in args.iter().skip(1) {
        let sound = runtime.sound_bank.as_ref().and_then(|bank| bank.find(path));
        if let Some(sound) = sound {
            let _ = runtime.server.events.push_to(
                qa_core::events::OutputTarget::Presentation,
                FrameEvent::Sound(SoundEvent {
                    sound,
                    bus: qa_core::primitives::SoundBus::Local,
                    queued_at: context.event_time.unwrap_or(runtime.input_time),
                    spatial: qa_gameplay::rules::sound_policy(context.source),
                    playback: qa_gameplay::rules::native_sound_playback(context.source),
                    entity: None,
                    channel: 0,
                    position: Vec3::default(),
                    volume: 1.0,
                    attenuation: 0.0,
                    action: SoundAction::Play,
                    ..SoundEvent::default()
                }),
            );
        } else {
            runtime.print_event(
                None,
                qa_core::primitives::PrintKind::Console,
                format_args!("sound is not precached: {path}\n"),
            );
        }
    }
    Ok(())
}
