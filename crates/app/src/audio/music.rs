//! One presentation-owned soundtrack streams into the existing raw paint lane.
use qa_audio::Mixer;
use qa_content::vfs::{FileRef, Vfs};
use qa_core::primitives::{
    MusicControl, MusicOwner, MusicPlayback, PcmFormat, SoundAction, SoundEvent,
};
use qa_formats::{
    FormatError,
    archive::ArchiveReader,
    sound::{Vorbis, WavInfo},
};
use std::fmt::Write;

#[derive(Default, Debug)]
pub struct MusicCounts {
    pub started: u64,
    pub stopped: u64,
    pub loops: u64,
    pub invalid: u64,
    pub source_bytes: u64,
    pub queued_frames: u64,
}
enum Decoder {
    Wave(WavInfo),
    Vorbis(Vorbis),
}
struct Track {
    file: FileRef,
    length: u64,
    decoder: Decoder,
}
impl Track {
    fn load(vfs: &Vfs, file: FileRef, reader: &mut ArchiveReader) -> Result<Self, FormatError> {
        let length = vfs.length(file).map_err(|_| FormatError::InvalidRange)?;
        let mut magic = [0; 4];
        read(vfs, file, 0, &mut magic, reader)?;
        let decoder = match &magic {
            b"RIFF" => {
                let info =
                    WavInfo::read_source(length, |at, out| read(vfs, file, at, out, reader))?;
                if info.frames == 0 {
                    return Err(FormatError::InvalidRange);
                }
                Decoder::Wave(info)
            }
            b"OggS" => {
                let mut vorbis = Vorbis::load(length, |at, out| read(vfs, file, at, out, reader))?;
                if vorbis
                    .read_packet(|at, out| read(vfs, file, at, out, reader))?
                    .is_none()
                {
                    return Err(FormatError::InvalidRange);
                }
                vorbis.rewind();
                Decoder::Vorbis(vorbis)
            }
            _ => return Err(FormatError::Unsupported),
        };
        Ok(Self {
            file,
            length,
            decoder,
        })
    }
    fn format(&self) -> PcmFormat {
        match &self.decoder {
            Decoder::Wave(info) => info.format(),
            Decoder::Vorbis(reader) => reader.format(),
        }
    }
    fn reset(&mut self) {
        if let Decoder::Vorbis(reader) = &mut self.decoder {
            reader.rewind();
        }
    }
    fn fill(
        &mut self,
        vfs: &Vfs,
        source: &mut Source,
        frames: usize,
        bytes: &mut [u8],
        archive: &mut ArchiveReader,
    ) -> Result<(usize, bool), FormatError> {
        match &mut self.decoder {
            Decoder::Wave(info) => {
                let frames = frames.min(info.frames - source.position);
                let size = info.format().frame_bytes();
                let length = frames * size;
                let offset = info.data_offset + source.position * size;
                read(vfs, self.file, offset as u64, &mut bytes[..length], archive)?;
                let ended = source.position + frames == info.frames;
                if ended {
                    read(vfs, self.file, self.length, &mut [], archive)?;
                }
                Ok((frames, ended))
            }
            Decoder::Vorbis(reader) => {
                let channels = reader.format().channels as usize;
                let mut written = 0;
                while written < frames * channels {
                    if source.packet_at == reader.samples().len() {
                        if reader
                            .read_packet(|at, out| read(vfs, self.file, at, out, archive))?
                            .is_none()
                        {
                            read(vfs, self.file, self.length, &mut [], archive)?;
                            return Ok((written / channels, true));
                        }
                        source.packet_at = 0;
                    }
                    let remaining = &reader.samples()[source.packet_at..];
                    let count = remaining.len().min(frames * channels - written);
                    for (sample, output) in remaining[..count]
                        .iter()
                        .zip(bytes[written * 2..(written + count) * 2].chunks_exact_mut(2))
                    {
                        output.copy_from_slice(&sample.to_le_bytes());
                    }
                    source.packet_at += count;
                    written += count;
                }
                Ok((written / channels, false))
            }
        }
    }
}
#[derive(Clone, Copy)]
struct Source {
    track: usize,
    position: usize,
    packet_at: usize,
    ended: bool,
}
impl Source {
    fn new(track: usize) -> Self {
        Self {
            track,
            position: 0,
            packet_at: 0,
            ended: false,
        }
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
    tracks: Box<[Track]>,
    playback: MusicPlayback,
    volume: f32,
    paused: bool,
    reader: ArchiveReader,
    bytes: Box<[u8]>,
    pub counts: MusicCounts,
}
impl Music {
    /// File names remain in the VFS. Only compiled source state is retained,
    /// keyed by its existing FileRef, independently of effect registration.
    pub fn load(vfs: &Vfs) -> Self {
        let mut reader = ArchiveReader::default();
        let mut tracks = Vec::new();
        for (file, _) in vfs.files() {
            if let Ok(track) = Track::load(vfs, file, &mut reader) {
                tracks.push(track);
            }
        }
        tracks.sort_unstable_by_key(|track| track.file);
        reader.close_stream();
        Self {
            owner: None,
            current: None,
            loop_track: None,
            tracks: tracks.into_boxed_slice(),
            playback: MusicPlayback {
                wave_first: false,
                smooth_volume: false,
                preserve_queued: false,
                signed_stereo_bytes: false,
            },
            volume: 0.5,
            paused: false,
            reader,
            bytes: vec![0; 30000].into_boxed_slice(),
            counts: MusicCounts::default(),
        }
    }
    fn track(&self, file: FileRef) -> Option<usize> {
        self.tracks
            .binary_search_by_key(&file, |track| track.file)
            .ok()
    }
    fn begin(&mut self, track: usize) -> Source {
        self.reader.close_stream();
        self.tracks[track].reset();
        Source::new(track)
    }
    fn numbered_track(&self, vfs: &Vfs, number: u32) -> Option<usize> {
        for extension in ["ogg", "wav"] {
            for prefix in ["", "track"] {
                let mut path = qa_core::text::FixedText::<40>::default();
                write!(path, "music/{prefix}{number:02}.{extension}").ok()?;
                if let Some(track) = vfs.open(path.as_bytes()).and_then(|file| self.track(file)) {
                    return Some(track);
                }
            }
        }
        None
    }
    fn named_file(&self, vfs: &Vfs, name: &[u8], wave_first: bool) -> Option<FileRef> {
        // Explicit names keep their native meaning, including mounted files
        // outside music/ and extensions unrelated to their PCM encoding.
        if let Some(file) = vfs.open(name) {
            return Some(file);
        }
        let extensions: [&[u8]; 2] = if wave_first {
            [b".wav", b".ogg"]
        } else {
            [b".ogg", b".wav"]
        };
        let mut invalid = None;
        for extension in extensions {
            let mut path = qa_core::text::FixedText::<8192>::default();
            path.append_bytes(name).ok()?;
            qa_core::names::default_extension(&mut path, extension, true).ok()?;
            if let Some(file) = vfs.open(path.as_bytes()) {
                if self.track(file).is_some() {
                    return Some(file);
                }
                invalid.get_or_insert(file);
            }
        }
        // An existing malformed loop retains its old failure/queued-PCM
        // semantics instead of becoming an absent loop.
        invalid
    }
    fn start(
        &mut self,
        mixer: &mut Mixer,
        owner: MusicOwner,
        playback: MusicPlayback,
        intro: Option<usize>,
        loop_track: Option<FileRef>,
    ) -> bool {
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
        let Some(track) = intro else {
            self.counts.invalid += 1;
            return false;
        };
        self.loop_track = loop_track;
        self.current = Some(self.begin(track));
        self.counts.started += 1;
        true
    }
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
            MusicControl::Track {
                owner,
                playback,
                number,
                looping,
            } => {
                if number == 0 {
                    self.stop(mixer);
                    return true;
                }
                let intro = self.numbered_track(vfs, number);
                let loop_track = intro
                    .filter(|_| looping)
                    .map(|track| self.tracks[track].file);
                if self.owner == Some(owner)
                    && self.playback == playback
                    && self
                        .current
                        .is_some_and(|source| Some(source.track) == intro)
                    && self.loop_track == loop_track
                {
                    return true;
                }
                self.start(mixer, owner, playback, intro, loop_track)
            }
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
                let opened = intro
                    .and_then(|name| self.named_file(vfs, name, playback.wave_first))
                    .and_then(|file| self.track(file));
                let loop_track = loop_track
                    .filter(|name| !name.is_empty())
                    .and_then(|name| self.named_file(vfs, name, playback.wave_first));
                self.start(mixer, owner, playback, opened, loop_track)
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
            if source.ended {
                let Some(next) = self.loop_track else {
                    self.current = None;
                    self.reader.close_stream();
                    return;
                };
                let Some(track) = self.track(next) else {
                    self.current = None;
                    self.reader.close_stream();
                    self.counts.invalid += 1;
                    return;
                };
                self.current = Some(self.begin(track));
                self.counts.loops += 1;
                continue;
            }
            let format = self.tracks[source.track].format();
            let wanted = (mixer.raw_remaining() as u64 * u64::from(format.rate.get())
                / u64::from(mixer.bank.rate.get())) as usize;
            let frames = wanted.min(self.bytes.len() / format.frame_bytes());
            if frames == 0 {
                return;
            }
            let filled = self.tracks[source.track].fill(
                vfs,
                &mut source,
                frames,
                &mut self.bytes,
                &mut self.reader,
            );
            let Ok((frames, ended)) = filled else {
                self.counts.invalid += 1;
                self.stop(mixer);
                return;
            };
            source.ended = ended;
            if frames == 0 {
                self.current = Some(source);
                continue;
            }
            let length = frames * format.frame_bytes();
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
