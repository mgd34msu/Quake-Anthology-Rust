//! SERVER-owned native file sources; bytes enter the existing output ring.
use crate::Runtime;
use qa_console::{
    command_text::Arguments,
    cvars::{Cvars, View},
    views::Context,
};
use qa_content::vfs::{FileRef, MountKind, Vfs};
use qa_core::{
    events::{EventRing, OutputConsumerId},
    names::path_byte,
    payloads::PayloadQueue,
    primitives::{ClientId, EntityId},
    sys_events::EventTime,
    text::FixedText,
};
use qa_formats::archive::ArchiveReader;
use qa_network::{
    commands::packet::Protocol,
    downloads::{self, AssetCategory},
};
use std::fmt::Write;

struct Permissions {
    protocol: Protocol,
    allow: View,
    categories: [Option<View>; 7],
    pure: Option<View>,
    max_rate: Option<View>,
}
impl Permissions {
    fn bind(cvars: &Cvars, protocol: Protocol) -> Option<Self> {
        let context = Context {
            source: protocol.rule_set(),
            side: qa_console::catalog::Scope::Server,
            ..Default::default()
        };
        Some(Self {
            protocol,
            allow: cvars.bind("sv_allowDownload", context)?,
            pure: cvars.bind("sv_pure", context),
            max_rate: cvars.bind("sv_maxRate", context),
            categories: [
                "allow_download_maps",
                "allow_download_models",
                "allow_download_sounds",
                if protocol == Protocol::QuakeWorld28 {
                    "allow_download_skins"
                } else {
                    "allow_download_players"
                },
                "allow_download_textures",
                "allow_download_pics",
                "allow_download_others",
            ]
            .map(|name| cvars.bind(name, context)),
        })
    }
    fn allows(&self, name: &[u8], cvars: &Cvars) -> bool {
        let enabled = |view| {
            if matches!(
                self.protocol,
                Protocol::Quake2Repro1038 | Protocol::Quake3_68
            ) {
                cvars.integer_view(view).unwrap_or(0) != 0
            } else {
                cvars.numeric(view).unwrap_or(0.0) != 0.0
            }
        };
        if !enabled(self.allow) {
            return false;
        }
        let allowed = |index: usize| self.categories[index].is_some_and(enabled);
        match downloads::content_category(self.protocol, name) {
            AssetCategory::Map => allowed(0),
            AssetCategory::Model
                if self.protocol == Protocol::QuakeWorld28 && name.starts_with(b"progs/")
                    || self.protocol != Protocol::QuakeWorld28 && name.starts_with(b"models/")
                    || self.protocol == Protocol::Quake2Repro1038
                        && name.starts_with(b"sprites/") =>
            {
                allowed(1)
            }
            AssetCategory::Sound => allowed(2),
            AssetCategory::Skin if self.protocol == Protocol::QuakeWorld28 => allowed(3),
            AssetCategory::Player if self.protocol != Protocol::QuakeWorld28 => allowed(3),
            AssetCategory::Texture if self.protocol == Protocol::Quake2Repro1038 => allowed(4),
            AssetCategory::Picture if self.protocol == Protocol::Quake2Repro1038 => allowed(5),
            _ if self.protocol == Protocol::Quake2Repro1038 => allowed(6),
            _ => true,
        }
    }
}

#[derive(Clone, Copy)]
struct Receiver {
    lifetime: EntityId,
    output: OutputConsumerId,
    overflow: u64,
    resyncs: u64,
    protocol: Protocol,
    policy: Option<downloads::Sequential>,
}
impl Receiver {
    fn current(self, events: &EventRing) -> bool {
        !events.needs_resync(self.output)
            && events.counters(self.output).is_some_and(|counts| {
                counts.overflow == self.overflow && counts.resyncs == self.resyncs
            })
    }
}

struct Source {
    file: FileRef,
    length: u64,
    offset: u64,
}
#[derive(Clone, Copy, Default)]
enum Pending {
    #[default]
    None,
    Chunk,
    Unavailable,
}

const WINDOW: usize = 8;
const BLOCK_BYTES: usize = 2048;

#[derive(Clone, Copy, Default)]
enum Failure {
    #[default]
    Missing,
    Disabled,
    Stock(&'static str),
    Changed,
}

#[derive(Clone, Copy, Default)]
struct DownloadRate {
    blocks: i32,
    interval: u64,
}

struct DownloadFrame<'a> {
    events: &'a mut EventRing,
    vfs: &'a Vfs,
    scratch: &'a mut [u8],
    time: EventTime,
    rate: DownloadRate,
    cvars: &'a Cvars,
}

struct Numbered {
    blocks: PayloadQueue<i32>,
    current: i32,
    transmit: i32,
    sent: i32,
    eof: bool,
    last_send: EventTime,
    next_send: EventTime,
}
impl Numbered {
    fn reset(&mut self) {
        self.blocks.clear();
        self.current = 0;
        self.transmit = 0;
        self.sent = 0;
        self.eof = false;
        self.last_send = EventTime::default();
        self.next_send = EventTime::default();
    }
}

pub(crate) struct ServerDownload {
    receiver: Option<Receiver>,
    source: Option<Source>,
    pending: Pending,
    permissions: Option<Permissions>,
    reader: ArchiveReader,
    numbered: Numbered,
    name: FixedText<64>,
    failure: Failure,
    retire_before: u64,
}
impl ServerDownload {
    pub(crate) fn load() -> Result<Self, String> {
        Ok(Self {
            receiver: None,
            source: None,
            pending: Pending::None,
            permissions: None,
            reader: ArchiveReader::default(),
            numbered: Numbered {
                blocks: PayloadQueue::load(WINDOW, WINDOW * BLOCK_BYTES)
                    .map_err(|_| "SERVER download window capacity")?,
                current: 0,
                transmit: 0,
                sent: 0,
                eof: false,
                last_send: EventTime::default(),
                next_send: EventTime::default(),
            },
            name: FixedText::default(),
            failure: Failure::Missing,
            retire_before: 0,
        })
    }
    pub(crate) fn accepts_record(&self, sequence: u64) -> bool {
        sequence >= self.retire_before
    }
    fn cancel(&mut self, boundary: u64) {
        self.reset();
        self.retire_before = boundary;
    }

    pub(crate) fn reset(&mut self) {
        self.receiver = None;
        self.source = None;
        self.pending = Pending::None;
        self.reader.close_stream();
        self.numbered.reset();
        self.name.clear();
        self.failure = Failure::Missing;
    }

    fn begin(&mut self, receiver: Receiver, args: Arguments<'_>, cvars: &Cvars, vfs: &Vfs) {
        self.receiver = Some(receiver);
        self.pending = Pending::Unavailable;
        let raw = args.get_bytes(1);
        if downloads::validate_path(raw, downloads::PathRole::Request).is_err() {
            return;
        }
        let mut name = [0; 64];
        for (to, from) in name.iter_mut().zip(raw) {
            *to = path_byte(*from);
        }
        let name = &name[..raw.len()];
        if self.name.append_bytes(name).is_err() {
            return;
        }
        if receiver.protocol == Protocol::Quake3_68 {
            if let Some(product) = downloads::q3_stock_package(name) {
                self.failure = Failure::Stock(product);
                return;
            }
            if !name.ends_with(b".pk3") {
                return;
            }
        }
        if self
            .permissions
            .as_ref()
            .is_none_or(|views| views.protocol != receiver.protocol)
        {
            self.permissions = Permissions::bind(cvars, receiver.protocol);
        }
        if self
            .permissions
            .as_ref()
            .is_none_or(|views| !views.allows(name, cvars))
        {
            self.failure = Failure::Disabled;
            return;
        }
        let offset = if matches!(
            receiver.protocol,
            Protocol::QuakeWorld28 | Protocol::Quake3_68
        ) {
            0
        } else {
            let value = qa_core::text::atoi_prefix(args.get_bytes(2));
            if value < 0 {
                return;
            }
            value as u64
        };
        let Some(file) = (if receiver.protocol == Protocol::Quake3_68 {
            vfs.open_product_file(name)
        } else {
            vfs.open(name)
        }) else {
            return;
        };
        if name.starts_with(b"maps/")
            && vfs
                .origin(&file)
                .is_none_or(|origin| origin.kind != MountKind::Directory)
        {
            return;
        }
        let Ok(length) = vfs.length(&file) else {
            return;
        };
        if length > i32::MAX as u64 || receiver.protocol == Protocol::Quake3_68 && length == 0 {
            return;
        }
        self.source = Some(Source {
            file,
            length,
            offset: offset.min(length),
        });
        self.pending = Pending::Chunk;
    }

    fn acknowledge(
        &mut self,
        number: i32,
        time: EventTime,
        boundary: u64,
    ) -> Result<(), &'static str> {
        let Some((expected, bytes)) = self.numbered.blocks.front() else {
            return Err("broken download");
        };
        if number != expected || number >= self.numbered.sent {
            return Err("broken download");
        }
        if bytes.is_empty() {
            self.cancel(boundary);
        } else {
            self.numbered.blocks.pop();
            self.numbered.transmit = self.numbered.transmit.max(number + 1);
            self.numbered.last_send = time;
        }
        Ok(())
    }
    fn fill_window(&mut self, vfs: &Vfs, scratch: &mut [u8]) -> Result<(), ()> {
        let Some(source) = &mut self.source else {
            return Err(());
        };
        while self.numbered.blocks.has_room(0) && source.offset != source.length {
            let length = BLOCK_BYTES.min((source.length - source.offset) as usize);
            let bytes = scratch.get_mut(..length).ok_or(())?;
            if vfs
                .read_stream_reusing(&source.file, source.offset, bytes, &mut self.reader)
                .map_err(|_| ())?
                != length
            {
                return Err(());
            }
            self.numbered
                .blocks
                .push(self.numbered.current, bytes, 0)
                .map_err(|_| ())?;
            self.numbered.current += 1;
            source.offset += length as u64;
        }
        if source.offset == source.length && !self.numbered.eof && self.numbered.blocks.has_room(0)
        {
            self.numbered
                .blocks
                .push(self.numbered.current, &[], 0)
                .map_err(|_| ())?;
            self.numbered.current += 1;
            self.numbered.eof = true;
        }
        Ok(())
    }
    fn failure_text(&self, cvars: &Cvars) -> FixedText<1024> {
        let mut text = FixedText::default();
        let name = self.name.as_str();
        let _ = match self.failure {
            Failure::Stock("missionpack") => write!(
                text,
                "Cannot autodownload Team Arena file \"{name}\"\nThe Team Arena mission pack can be found in your local game store."
            ),
            Failure::Stock(_) => write!(text, "Cannot autodownload id pk3 file \"{name}\""),
            Failure::Disabled => {
                let pure = self
                    .permissions
                    .as_ref()
                    .and_then(|views| views.pure)
                    .and_then(|view| cvars.integer_view(view).ok())
                    .unwrap_or(0)
                    != 0;
                let advice = if pure {
                    "You will need to get this file elsewhere before you can connect to this pure server.\n"
                } else {
                    "The server you are connecting to is not a pure server, set autodownload to No in your settings and you might be able to join the game anyway.\n"
                };
                write!(
                    text,
                    "Could not download \"{name}\" because autodownloading is disabled on the server.\n\n{advice}"
                )
            }
            Failure::Changed => write!(text, "File \"{name}\" changed while downloading.\n"),
            Failure::Missing => write!(
                text,
                "File \"{name}\" not found on server for autodownloading.\n"
            ),
        };
        text
    }
    fn rate(&self, info: &[u8], cvars: &mut Cvars) -> DownloadRate {
        if self
            .receiver
            .is_none_or(|receiver| receiver.protocol != Protocol::Quake3_68)
        {
            return DownloadRate::default();
        }
        let rate = qa_core::text::info_value(info, b"rate", false)
            .map(|span| &info[span])
            .filter(|text| !text.is_empty())
            .map_or(3000, |text| {
                qa_core::text::atoi_prefix(text).clamp(1000, 90000)
            });
        let max_rate = self
            .permissions
            .as_ref()
            .and_then(|views| views.max_rate)
            .and_then(|view| cvars.integer_view(view).ok())
            .unwrap_or(0);
        if max_rate != 0
            && max_rate < 1000
            && let Some(view) = self.permissions.as_ref().and_then(|views| views.max_rate)
        {
            let _ = cvars.force_write(view, "1000");
        }
        let rate = if max_rate == 0 {
            rate
        } else {
            rate.min(max_rate.max(1000))
        };
        let interval = qa_core::text::info_value(info, b"snaps", false)
            .map(|span| &info[span])
            .filter(|text| !text.is_empty())
            .map_or(50, |text| {
                1000 / qa_core::text::atoi_prefix(text).clamp(1, 30)
            });
        DownloadRate {
            blocks: ((rate * interval / 1000 + BLOCK_BYTES as i32) / BLOCK_BYTES as i32)
                .clamp(1, WINDOW as i32),
            interval: interval as u64,
        }
    }
    fn flush_numbered(&mut self, id: ClientId, receiver: Receiver, frame: DownloadFrame<'_>) {
        let DownloadFrame {
            events,
            vfs,
            scratch,
            time,
            rate,
            cvars,
        } = frame;
        if matches!(self.pending, Pending::None) {
            return;
        }
        if matches!(self.pending, Pending::Chunk) && self.fill_window(vfs, scratch).is_err() {
            self.source = None;
            self.reader.close_stream();
            self.pending = Pending::Unavailable;
            self.failure = Failure::Changed;
        }
        if matches!(self.pending, Pending::Unavailable) {
            let message = self.failure_text(cvars);
            if events
                .file_chunk(
                    id,
                    receiver.lifetime,
                    downloads::Chunk::Rejected {
                        size: -1,
                        message: message.as_bytes(),
                    },
                )
                .is_ok()
            {
                self.reset();
            }
            return;
        }
        if time < self.numbered.next_send {
            return;
        }
        self.numbered.next_send =
            EventTime::from_milliseconds(time.milliseconds().saturating_add(rate.interval));
        for _ in 0..rate.blocks {
            if self.numbered.transmit == self.numbered.current {
                if time.since(self.numbered.last_send) <= EventTime::from_milliseconds(1000).0 {
                    return;
                }
                let Some((number, _)) = self.numbered.blocks.front() else {
                    return;
                };
                self.numbered.transmit = number;
            }
            let Some((first, _)) = self.numbered.blocks.front() else {
                return;
            };
            let Some((number, bytes)) = self
                .numbered
                .blocks
                .get((self.numbered.transmit - first) as usize)
            else {
                return;
            };
            let total = (number == 0).then(|| {
                self.source
                    .as_ref()
                    .map_or(0, |source| source.length as u32)
            });
            if events
                .file_chunk(
                    id,
                    receiver.lifetime,
                    downloads::Chunk::Block {
                        number: number as i16,
                        total,
                        bytes,
                    },
                )
                .is_err()
            {
                return;
            }
            if !receiver.current(events) {
                self.reset();
                return;
            }
            self.numbered.transmit += 1;
            self.numbered.sent = self.numbered.sent.max(self.numbered.transmit);
            self.numbered.last_send = time;
        }
    }

    fn flush(&mut self, id: ClientId, frame: DownloadFrame<'_>) {
        let Some(receiver) = self.receiver else {
            return;
        };
        if !receiver.current(frame.events) {
            self.reset();
            return;
        }
        if receiver.protocol == Protocol::Quake3_68 {
            self.flush_numbered(id, receiver, frame);
            return;
        }
        let DownloadFrame {
            events,
            vfs,
            scratch,
            ..
        } = frame;
        let Some(policy) = receiver.policy else {
            self.reset();
            return;
        };
        let (length, percent, available) = match self.pending {
            Pending::None => return,
            Pending::Unavailable => (0, 0, false),
            Pending::Chunk => {
                let Some(source) = self.source.as_ref() else {
                    self.reset();
                    return;
                };
                let maximum = policy
                    .block_bytes()
                    .min((source.length - source.offset) as usize);
                let read = scratch.get_mut(..maximum).and_then(|destination| {
                    vfs.read_stream_reusing(
                        &source.file,
                        source.offset,
                        destination,
                        &mut self.reader,
                    )
                    .ok()
                });
                match read {
                    Some(length) if length == maximum => {
                        let next = source.offset + length as u64;
                        let percent = (next * 100 / source.length.max(1)) as u8;
                        (length, percent, true)
                    }
                    _ => {
                        self.source = None;
                        self.reader.close_stream();
                        self.pending = Pending::Unavailable;
                        (0, 0, false)
                    }
                }
            }
        };
        let bytes = available.then_some(&scratch[..length]);
        if events
            .file_chunk(
                id,
                receiver.lifetime,
                downloads::Chunk::Sequential {
                    percent,
                    bytes,
                    compressed: false,
                },
            )
            .is_err()
        {
            return;
        }
        // A publication may itself retire an overflowing consumer. Never skip
        // that lost block by retaining a continuation under its old epoch.
        if !receiver.current(events) {
            self.reset();
            return;
        }
        self.pending = Pending::None;
        if let Some(source) = self.source.as_mut().filter(|_| available) {
            source.offset += length as u64;
            if source.offset < source.length {
                return;
            }
        }
        self.source = None;
        self.reader.close_stream();
    }
}

impl Runtime {
    pub(crate) fn server_download_command(
        &mut self,
        id: ClientId,
        args: Arguments<'_>,
        cvars: &mut Cvars,
        time: EventTime,
    ) -> Result<bool, &'static str> {
        if args.get_bytes(0) == b"new" {
            if let Some(client) = self
                .server
                .clients
                .get(id.0 as usize)
                .filter(|client| client.connection.is_some())
                && let Some(row) = self.server_downloads.get_mut(id.0 as usize)
                && row
                    .receiver
                    .is_some_and(|receiver| receiver.lifetime == client.entity)
            {
                row.cancel(self.server.events.sequence_range().end);
            }
            return Ok(false);
        }
        if !matches!(
            args.get_bytes(0),
            b"download" | b"nextdl" | b"stopdl" | b"donedl"
        ) {
            return Ok(false);
        }
        let slot = id.0 as usize;
        let Some(client) = self
            .server
            .clients
            .get(slot)
            .filter(|client| client.connection.is_some())
        else {
            return Ok(false);
        };
        let Some(binding) = self.snapshots.get(slot).and_then(Option::as_ref) else {
            return Ok(false);
        };
        let protocol = binding.protocol;
        let policy = downloads::sequential(protocol);
        if policy.is_none() && protocol != Protocol::Quake3_68 {
            return Ok(false);
        }
        if matches!(args.get_bytes(0), b"stopdl" | b"donedl") && protocol != Protocol::Quake3_68 {
            return Ok(false);
        }
        if args.get_bytes(0) == b"donedl" {
            self.config_publications[slot].restart_game_state();
            return Ok(true);
        }
        let Some(output) = client.output.filter(|output| binding.output == *output) else {
            return Ok(true);
        };
        let Some(counts) = self.server.events.counters(output) else {
            return Ok(true);
        };
        let receiver = Receiver {
            lifetime: client.entity,
            output,
            overflow: counts.overflow,
            resyncs: counts.resyncs,
            protocol,
            policy,
        };
        let row = &mut self.server_downloads[slot];
        if args.get_bytes(0) == b"download" {
            row.cancel(self.server.events.sequence_range().end);
            row.begin(receiver, args, cvars, &self.vfs);
        } else if args.get_bytes(0) == b"stopdl" {
            row.cancel(self.server.events.sequence_range().end);
        } else if protocol == Protocol::Quake3_68 {
            if row.receiver.is_none_or(|old| {
                old.lifetime != receiver.lifetime
                    || old.output != receiver.output
                    || !old.current(&self.server.events)
            }) {
                return Err("broken download");
            }
            row.acknowledge(
                qa_core::text::atoi_prefix(args.get_bytes(1)),
                time,
                self.server.events.sequence_range().end,
            )?;
        } else if row.receiver.is_some_and(|old| {
            old.lifetime == receiver.lifetime
                && old.output == receiver.output
                && old.protocol == protocol
                && old.current(&self.server.events)
        }) && row.source.is_some()
        {
            row.pending = Pending::Chunk;
        } else {
            row.reset();
        }
        Ok(true)
    }

    pub(crate) fn flush_server_downloads(&mut self, time: EventTime, cvars: &mut Cvars) {
        for (slot, row) in self.server_downloads.iter_mut().enumerate() {
            if row.receiver.is_some_and(|receiver| {
                self.server.clients[slot].connection.is_none()
                    || self.server.clients[slot].entity != receiver.lifetime
                    || self.server.clients[slot].output != Some(receiver.output)
            }) {
                row.reset();
            }
            let rate = row.rate(
                self.server.userinfo(ClientId(slot as u32)).unwrap_or(&[]),
                cvars,
            );
            row.flush(
                ClientId(slot as u32),
                DownloadFrame {
                    events: &mut self.server.events,
                    vfs: &self.vfs,
                    scratch: &mut self.output_bytes,
                    time,
                    rate,
                    cvars,
                },
            );
        }
    }
}
