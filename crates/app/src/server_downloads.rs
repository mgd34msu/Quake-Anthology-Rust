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
    primitives::{ClientId, EntityId},
};
use qa_formats::archive::ArchiveReader;
use qa_network::{
    commands::packet::Protocol,
    downloads::{self, AssetCategory},
};

struct Permissions {
    protocol: Protocol,
    allow: View,
    categories: [Option<View>; 7],
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
            if self.protocol == Protocol::Quake2Repro1038 {
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
    policy: downloads::Sequential,
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

pub(crate) struct ServerDownload {
    receiver: Option<Receiver>,
    source: Option<Source>,
    pending: Pending,
    permissions: Option<Permissions>,
    reader: ArchiveReader,
}
impl Default for ServerDownload {
    fn default() -> Self {
        Self {
            receiver: None,
            source: None,
            pending: Pending::None,
            permissions: None,
            reader: ArchiveReader::default(),
        }
    }
}
impl ServerDownload {
    pub(crate) fn reset(&mut self) {
        self.receiver = None;
        self.source = None;
        self.pending = Pending::None;
        self.reader.close_stream();
    }

    fn begin(&mut self, receiver: Receiver, args: Arguments<'_>, cvars: &Cvars, vfs: &Vfs) {
        self.reset();
        self.receiver = Some(receiver);
        self.pending = Pending::Unavailable;
        let raw = args.get_bytes(1);
        if downloads::validate_path(raw).is_err() {
            return;
        }
        let mut name = [0; 64];
        for (to, from) in name.iter_mut().zip(raw) {
            *to = path_byte(*from);
        }
        let name = &name[..raw.len()];
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
            return;
        }
        let offset = if receiver.protocol == Protocol::QuakeWorld28 {
            0
        } else {
            let value = qa_core::text::atoi_prefix(args.get_bytes(2));
            if value < 0 {
                return;
            }
            value as u64
        };
        let Some(file) = vfs.open(name) else {
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
        if length > i32::MAX as u64 {
            return;
        }
        self.source = Some(Source {
            file,
            length,
            offset: offset.min(length),
        });
        self.pending = Pending::Chunk;
    }

    fn flush(&mut self, id: ClientId, events: &mut EventRing, vfs: &Vfs, scratch: &mut [u8]) {
        let Some(receiver) = self.receiver else {
            return;
        };
        if !receiver.current(events) {
            self.reset();
            return;
        }
        let (length, percent, available) = match self.pending {
            Pending::None => return,
            Pending::Unavailable => (0, 0, false),
            Pending::Chunk => {
                let Some(source) = self.source.as_ref() else {
                    self.reset();
                    return;
                };
                let maximum = receiver
                    .policy
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
            .file_chunk(id, receiver.lifetime, percent, bytes)
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
        cvars: &Cvars,
    ) -> bool {
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
                row.reset();
            }
            return false;
        }
        if !matches!(args.get_bytes(0), b"download" | b"nextdl") {
            return false;
        }
        let slot = id.0 as usize;
        let Some(client) = self
            .server
            .clients
            .get(slot)
            .filter(|client| client.connection.is_some())
        else {
            return false;
        };
        let Some(binding) = self.snapshots.get(slot).and_then(Option::as_ref) else {
            return false;
        };
        let protocol = binding.protocol;
        let Some(policy) = downloads::sequential(protocol) else {
            return false;
        };
        let Some(output) = client.output.filter(|output| binding.output == *output) else {
            return true;
        };
        let Some(counts) = self.server.events.counters(output) else {
            return true;
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
            row.begin(receiver, args, cvars, &self.vfs);
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
        row.flush(
            id,
            &mut self.server.events,
            &self.vfs,
            &mut self.output_bytes,
        );
        true
    }

    pub(crate) fn flush_server_downloads(&mut self) {
        for (slot, row) in self.server_downloads.iter_mut().enumerate() {
            if row.receiver.is_some_and(|receiver| {
                self.server.clients[slot].connection.is_none()
                    || self.server.clients[slot].entity != receiver.lifetime
                    || self.server.clients[slot].output != Some(receiver.output)
            }) {
                row.reset();
            }
            row.flush(
                ClientId(slot as u32),
                &mut self.server.events,
                &self.vfs,
                &mut self.output_bytes,
            );
        }
    }
}
