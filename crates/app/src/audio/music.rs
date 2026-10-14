//! One presentation-owned soundtrack streams into the existing raw paint lane.
use qa_audio::Mixer;
use qa_content::vfs::{FileRef, Vfs};
use qa_core::primitives::{MusicControl, MusicOwner, MusicPlayback, SoundAction, SoundEvent};
use qa_formats::{FormatError, archive::ArchiveReader, sound::WavInfo};

#[derive(Default, Debug)]
pub struct MusicCounts {
    pub started: u64,
    pub stopped: u64,
    pub loops: u64,
    pub invalid: u64,
    pub source_bytes: u64,
    pub queued_frames: u64,
}
#[derive(Clone, Copy)]
struct Source {
    file: FileRef,
    info: WavInfo,
    position: usize,
}
impl Source {
    fn open(vfs: &Vfs, file: FileRef, reader: &mut ArchiveReader) -> Result<Self, FormatError> {
        let length = vfs.length(file).map_err(|_| FormatError::InvalidRange)?;
        let info = WavInfo::read_source(length, |at, out| read(vfs, file, at, out, reader))?;
        if info.frames == 0 {
            return Err(FormatError::InvalidRange);
        }
        Ok(Self {
            file,
            info,
            position: 0,
        })
    }
}
fn read(
    vfs: &Vfs,
    file: FileRef,
    at: u64,
    out: &mut [u8],
    reader: &mut ArchiveReader,
) -> Result<(), FormatError> {
    match vfs.read_stream_reusing(file, at, out, reader) {
        Ok(count) if count == out.len() => Ok(()),
        Ok(_) => Err(FormatError::Truncated),
        Err(qa_content::vfs::VfsError::Format(error)) => Err(error),
        Err(_) => Err(FormatError::Io(std::io::ErrorKind::Other)),
    }
}

pub struct Music {
    owner: Option<MusicOwner>,
    current: Option<Source>,
    loop_track: Option<FileRef>,
    playback: MusicPlayback,
    volume: f32,
    paused: bool,
    reader: ArchiveReader,
    bytes: Box<[u8]>,
    pub counts: MusicCounts,
}
impl Default for Music {
    fn default() -> Self {
        Self {
            owner: None,
            current: None,
            loop_track: None,
            playback: MusicPlayback {
                smooth_volume: false,
                preserve_queued: false,
                signed_stereo_bytes: false,
            },
            volume: 0.5,
            paused: false,
            reader: ArchiveReader::default(),
            bytes: vec![0; 30000].into_boxed_slice(),
            counts: MusicCounts::default(),
        }
    }
}
impl Music {
    pub fn playing(&self) -> bool {
        self.current.is_some()
    }
    pub fn control(
        &mut self,
        vfs: &Vfs,
        mixer: &mut Mixer,
        control: MusicControl,
        intro: Option<&[u8]>,
        loop_track: Option<&[u8]>,
    ) -> bool {
        match control {
            MusicControl::Pause(paused) => {
                if self.owner.is_some() {
                    self.paused = paused;
                    mixer.pause_raw(paused);
                }
                true
            }
            MusicControl::Stop { only_active } => {
                if !only_active || self.current.is_some() {
                    self.stop(mixer);
                }
                true
            }
            MusicControl::Retire(owner) => {
                if self.owner == Some(owner) {
                    self.stop(mixer);
                }
                true
            }
            MusicControl::Start {
                owner, playback, ..
            } => {
                // Replacement closes the prior source first. Native Q3 keeps
                // already queued PCM even when the new source fails to open.
                self.current = None;
                self.reader.close_stream();
                self.loop_track = None;
                self.owner = Some(owner);
                self.playback = playback;
                self.paused = false;
                mixer.pause_raw(false);
                if !playback.preserve_queued {
                    mixer.clear_raw();
                }
                let opened = intro
                    .and_then(|name| vfs.open(name))
                    .and_then(|file| Source::open(vfs, file, &mut self.reader).ok());
                let Some(source) = opened else {
                    self.counts.invalid += 1;
                    return false;
                };
                self.loop_track = loop_track
                    .filter(|name| !name.is_empty())
                    .and_then(|name| vfs.open(name));
                self.current = Some(source);
                self.counts.started += 1;
                true
            }
        }
    }
    fn stop(&mut self, mixer: &mut Mixer) {
        if self.owner.is_none() {
            return;
        }
        self.counts.stopped += u64::from(self.current.is_some() || self.owner.is_some());
        self.current = None;
        self.loop_track = None;
        self.owner = None;
        self.paused = false;
        mixer.pause_raw(false);
        self.reader.close_stream();
        mixer.clear_raw();
    }
    pub fn sound_control(&mut self, mixer: &mut Mixer, event: SoundEvent) {
        let Some(owner) = self.owner else {
            return;
        };
        let retire = match event.action {
            SoundAction::RetireClient => owner.client.is_some_and(|(client, lifetime)| {
                event.client == Some(client) && event.client_lifetime == Some(lifetime)
            }),
            SoundAction::ReleaseListener => {
                owner.module.is_some()
                    && event.listener_owner == owner.module
                    && owner.client.is_some_and(|(client, lifetime)| {
                        event.client == Some(client) && event.client_lifetime == Some(lifetime)
                    })
            }
            _ => false,
        };
        if retire {
            self.stop(mixer);
        }
    }
    /// Called once per presentation frame, independently of PCM write retries.
    pub fn update(&mut self, vfs: &Vfs, mixer: &mut Mixer, target_volume: f32) {
        if self.current.is_none() || self.paused {
            return;
        }
        let target = if target_volume.is_finite() {
            target_volume.max(0.)
        } else {
            0.
        };
        self.volume = if self.playback.smooth_volume {
            (self.volume + target * 2.) / 4.
        } else {
            target
        };
        if self.volume <= 0. {
            return;
        }
        // Every successful read advances the source and queues at least one
        // output frame, so fixed raw capacity also bounds short authored loops.
        loop {
            let Some(mut source) = self.current else {
                return;
            };
            if source.position == source.info.frames {
                // Validate the archive's unread WAV tail before committing EOF.
                let checked = vfs.length(source.file).is_ok_and(|length| {
                    read(vfs, source.file, length, &mut [], &mut self.reader).is_ok()
                });
                if !checked {
                    self.counts.invalid += 1;
                    self.stop(mixer);
                    return;
                }
                let Some(next) = self.loop_track else {
                    self.current = None;
                    self.reader.close_stream();
                    return;
                };
                self.current = if next == source.file {
                    source.position = 0;
                    Some(source)
                } else {
                    Source::open(vfs, next, &mut self.reader).ok()
                };
                if self.current.is_none() {
                    self.counts.invalid += 1;
                    return;
                }
                self.counts.loops += 1;
                continue;
            }
            let format = source.info.format();
            let wanted = (mixer.raw_remaining() as u64 * u64::from(format.rate.get())
                / u64::from(mixer.bank.rate.get())) as usize;
            let frames = wanted
                .min(source.info.frames - source.position)
                .min(self.bytes.len() / format.frame_bytes());
            if frames == 0 {
                return;
            }
            let length = frames * format.frame_bytes();
            let offset = source.info.data_offset + source.position * format.frame_bytes();
            if read(
                vfs,
                source.file,
                offset as u64,
                &mut self.bytes[..length],
                &mut self.reader,
            )
            .is_err()
            {
                self.counts.invalid += 1;
                self.stop(mixer);
                return;
            }
            match mixer.queue_raw(
                format,
                &self.bytes[..length],
                self.volume,
                self.playback.signed_stereo_bytes,
            ) {
                Ok(queued) => {
                    self.counts.source_bytes += length as u64;
                    self.counts.queued_frames += queued as u64;
                    source.position += frames;
                    self.current = Some(source);
                }
                Err(_) => {
                    self.counts.invalid += 1;
                    self.stop(mixer);
                    return;
                }
            }
        }
    }
}
