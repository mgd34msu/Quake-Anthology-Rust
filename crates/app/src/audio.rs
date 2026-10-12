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
    ids: [Option<qa_core::primitives::SoundId>; qa_gameplay::rules::q2::sounds::PATHS.len()],
    random: qa_core::random::Random,
}
impl Default for MuzzleSounds {
    fn default() -> Self {
        Self {
            ids: [None; qa_gameplay::rules::q2::sounds::PATHS.len()],
            random: qa_core::random::Random::new(5489),
        }
    }
}
impl MuzzleSounds {
    pub(crate) fn bind(&mut self, bank: &Bank) {
        for (id, path) in self
            .ids
            .iter_mut()
            .zip(qa_gameplay::rules::q2::sounds::PATHS)
        {
            *id = bank.find(path);
        }
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
        self.ids
            .get((recipe.choices.first + offset) as usize)
            .copied()
            .flatten()
    }
}

/// Shared CLIENT poses are sampled once before bounded PCM mixahead.
/// A sound's native namespace selects its owning connection, not a seat slot.
pub fn respatialize(
    mixer: &mut Mixer,
    entities: &[Option<crate::client_scene::ClientEntities<'_>>;
         qa_core::sys_events::SeatId::COUNT],
) {
    mixer.respatialize(|client, number| {
        entities
            .iter()
            .flatten()
            .find(|entities| entities.client == client)?
            .sound_origin(number)
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

pub fn load_bank(
    host: &mut crate::host::FrameHost,
    paths: &[String],
    rules: RuleSetId,
) -> Result<Arc<Bank>, String> {
    let mut registered = paths
        .iter()
        .map(|path| (qa_core::names::canonical_path(path), rules, true))
        .collect::<Vec<_>>();
    host.for_each_native_sound(|name, rules| {
        if let Some(path) =
            qa_audio::sound_path(name, rules).map_err(|e| format!("sound path: {e:?}"))?
        {
            let path = std::str::from_utf8(path.as_bytes()).map_err(|e| e.to_string())?;
            registered.push((qa_core::names::canonical_path(path), rules, false));
        }
        Ok(())
    })?;
    if host.runtime.local_snapshots.iter().flatten().any(|client| {
        matches!(
            client.protocol,
            qa_network::commands::packet::Protocol::Quake2_34
                | qa_network::commands::packet::Protocol::Quake2Repro1038
        )
    }) {
        registered.extend(qa_gameplay::rules::q2::sounds::PATHS.iter().map(|path| {
            (
                qa_core::names::canonical_path(path),
                RuleSetId::Quake2,
                false,
            )
        }));
    }
    let names =
        qa_core::names::NameTable::load(registered.iter().map(|(name, _, _)| name.as_bytes()))
            .map_err(|e| e.to_string())?;
    let mut loaded = qa_core::stamps::StampSet::new(names.len());
    let mut reader = qa_formats::archive::ArchiveReader::default();
    let mut sources = Vec::with_capacity(names.len());
    for (path, rules, required) in registered {
        let id = names
            .find(path.as_bytes())
            .ok_or("missing sound registration")?;
        if loaded.test_and_set(id.0 as usize) {
            continue;
        }
        let bytes =
            match host
                .runtime
                .vfs
                .read_file(path.as_bytes(), 256 * 1024 * 1024, &mut reader)
            {
                Ok(bytes) => bytes,
                Err(error) if !required => {
                    host.runtime.print_event(
                        None,
                        qa_core::primitives::PrintKind::Console,
                        format_args!("sound {path}: {error}\n"),
                    );
                    continue;
                }
                Err(error) => return Err(format!("sound {path}: {error}")),
            };
        let policy = if rules == RuleSetId::Quake3 {
            qa_formats::sound::WavPolicy::Quake3
        } else {
            qa_formats::sound::WavPolicy::Quake
        };
        let pcm = qa_formats::sound::decode(&bytes, policy)
            .map_err(|e| format!("sound decode: {e:?}"))?;
        sources.push((path, pcm));
    }
    let rate = NonZeroU32::new(44100).ok_or("invalid device rate")?;
    let bank = Arc::new(Bank::load(rate, sources)?);
    host.bind_sounds(&bank)?;
    Ok(bank)
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
                    spatial_rules: context.source,
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
