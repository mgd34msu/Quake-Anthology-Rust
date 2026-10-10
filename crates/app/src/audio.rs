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
    pub fn submit(&mut self) {
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
    vfs: &qa_content::vfs::Vfs,
    paths: &[String],
    rules: RuleSetId,
) -> Result<Arc<Bank>, String> {
    let mut reader = qa_formats::archive::ArchiveReader::default();
    let mut sources = Vec::with_capacity(paths.len());
    for path in paths {
        let file = vfs
            .open(path.as_bytes())
            .ok_or_else(|| format!("sound not found: {path}"))?;
        let length = usize::try_from(vfs.length(file).map_err(|e| format!("sound: {e:?}"))?)
            .map_err(|_| "sound too large")?;
        if length > 256 * 1024 * 1024 {
            return Err("sound too large".into());
        }
        let mut bytes = vec![0; length];
        let read = vfs
            .read_into_reusing(file, &mut bytes, &mut reader)
            .map_err(|e| format!("sound: {e:?}"))?;
        if read != length {
            return Err("short sound read".into());
        }
        let policy = if rules == RuleSetId::Quake3 {
            qa_formats::sound::WavPolicy::Quake3
        } else {
            qa_formats::sound::WavPolicy::Quake
        };
        let pcm = qa_formats::sound::decode(&bytes, policy)
            .map_err(|e| format!("sound decode: {e:?}"))?;
        sources.push((path.clone(), pcm, rules));
    }
    let rate = NonZeroU32::new(44100).ok_or("invalid device rate")?;
    Ok(Arc::new(Bank::load(rate, sources)?))
}

pub fn play(
    _: &mut Console<Runtime>,
    runtime: &mut Runtime,
    args: &qa_console::command_text::Arguments<'_>,
    _: Context,
) -> Result<(), CommandError> {
    if args.len() < 2 {
        return Err(CommandError::Usage);
    }
    for path in args.iter().skip(1) {
        let sound = runtime.sound_bank.as_ref().and_then(|bank| bank.find(path));
        if let Some(sound) = sound {
            let _ = runtime.server.events.push(FrameEvent::Sound(SoundEvent {
                sound,
                entity: None,
                channel: 0,
                position: Vec3::default(),
                volume: 1.0,
                attenuation: 0.0,
                action: SoundAction::Play,
            }));
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
