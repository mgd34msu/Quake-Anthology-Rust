use super::*;
use qa_network::commands::startup::{Q2ReproServerData, Q2ServerData, ResourceStream};

pub(crate) struct Q2Signon {
    server: i32,
    attract_loop: bool,
    client_number: i16,
    rerelease: Option<Q2ReproServerData>,
    pub(super) game_directory:
        qa_core::text::FixedText<{ qa_network::message::LEGACY_STRING_BYTES }>,
    phase: Phase,
    pub(crate) world_input: Option<crate::map::MapInput>,
}

#[derive(Clone, Copy)]
enum Phase {
    List {
        kind: ResourceStream,
        start: u32,
        pending: bool,
    },
    Files(FileCursor),
    Content,
    Begin,
    BeginQueued,
    Active,
    Cinematic,
}

#[derive(Clone, Copy)]
pub(crate) enum FileCursor {
    Models { ordinal: u32, image: Option<u32> },
    Sounds(u32),
    Images(u32),
    Players { slot: u32, part: u8 },
    Sky(u8),
    Textures(u32),
}

impl Q2Signon {
    pub(crate) fn new(data: &Q2ServerData<'_>) -> Result<Self, &'static str> {
        let mut game_directory = qa_core::text::FixedText::default();
        game_directory
            .set_bytes(data.game_directory)
            .map_err(|_| "Q2 game directory capacity")?;
        Ok(Self {
            server: data.server_count,
            attract_loop: data.attract_loop,
            client_number: data.client_number,
            rerelease: data.rerelease,
            game_directory,
            phase: if data.cinematic() {
                Phase::Cinematic
            } else {
                Phase::List {
                    kind: ResourceStream::ConfigStrings,
                    start: 0,
                    pending: false,
                }
            },
            world_input: None,
        })
    }
    pub(crate) fn data<'a>(&'a self, level_name: &'a [u8]) -> Q2ServerData<'a> {
        Q2ServerData {
            server_count: self.server,
            attract_loop: self.attract_loop,
            game_directory: self.game_directory.as_bytes(),
            client_number: self.client_number,
            level_name,
            rerelease: self.rerelease,
        }
    }
    pub(crate) fn waiting(&self) -> bool {
        !matches!(self.phase, Phase::Active)
    }
    pub(crate) fn reconnect_in_place(&self) -> bool {
        matches!(
            self.phase,
            Phase::List { .. } | Phase::Files(_) | Phase::Content
        )
    }
    pub(crate) fn allows_media(&self) -> bool {
        matches!(
            self.phase,
            Phase::Content | Phase::Begin | Phase::BeginQueued | Phase::Active
        )
    }
    pub(crate) fn resources_needed(&self) -> bool {
        matches!(self.phase, Phase::Content)
    }
    pub(crate) fn resources_admitted(&mut self) {
        if matches!(self.phase, Phase::Content) {
            self.world_input = None;
            self.phase = Phase::Begin;
        }
    }
    pub(crate) fn receive_entity(&mut self) {
        if matches!(self.phase, Phase::BeginQueued) {
            self.phase = Phase::Active;
        }
    }
    pub(crate) fn file_cursor(&self) -> Option<FileCursor> {
        match self.phase {
            Phase::Files(cursor) => Some(cursor),
            _ => None,
        }
    }
    pub(crate) fn advance_file(&mut self, next: Option<FileCursor>) {
        if matches!(self.phase, Phase::Files(_)) {
            self.phase = next.map_or(Phase::Content, Phase::Files);
        }
    }
    pub(crate) fn stream_complete(&mut self, kind: ResourceStream) -> Result<(), &'static str> {
        if matches!(self.phase, Phase::Active | Phase::Cinematic) {
            return Ok(());
        }
        if self.rerelease.is_none() {
            return Err("Q2 stream on classic protocol");
        }
        let Phase::List { kind: expected, .. } = self.phase else {
            return Err("unexpected Q2 resource stream");
        };
        // Native terminators end each bounded packet chunk. Precache ends the
        // whole gamestate, including one with no streamed rows or baselines.
        if expected == ResourceStream::Baselines && kind == ResourceStream::ConfigStrings {
            return Err("out-of-order Q2 resource stream");
        }
        self.phase = Phase::List {
            kind,
            start: 0,
            pending: false,
        };
        Ok(())
    }
    pub(crate) fn command(
        &mut self,
        args: qa_console::command_text::Arguments<'_>,
        layout: ConfigLayout,
        entity_limit: u32,
    ) -> Result<(), &'static str> {
        let parse = |index| {
            args.get(index)
                .parse::<i32>()
                .map_err(|_| "invalid Q2 signon number")
        };
        if args.get_bytes(0) == b"precache" && args.len() == 2 {
            if parse(1)? != self.server {
                return Err("stale Q2 precache generation");
            }
            if !matches!(self.phase, Phase::List { kind, pending: false, .. }
                if self.rerelease.is_some() || kind == ResourceStream::Baselines)
            {
                return Err("Q2 precache before baselines");
            }
            self.phase = Phase::Files(FileCursor::Models {
                ordinal: 1,
                image: None,
            });
            return Ok(());
        }
        if args.get_bytes(0) != b"cmd" || args.len() != 4 {
            return Err("unsupported Q2 server command");
        }
        let kind = match args.get_bytes(1) {
            b"configstrings" => ResourceStream::ConfigStrings,
            b"baselines" => ResourceStream::Baselines,
            _ => return Err("unsupported Q2 signon command"),
        };
        if parse(2)? != self.server {
            return Err("stale Q2 signon generation");
        }
        let start = args
            .get(3)
            .parse::<u32>()
            .map_err(|_| "invalid Q2 signon ordinal")?;
        let limit = match kind {
            ResourceStream::ConfigStrings => layout.count as u32,
            ResourceStream::Baselines => entity_limit,
        };
        if start >= limit {
            return Err("Q2 signon ordinal outside native range");
        }
        let Phase::List {
            kind: current,
            start: previous,
            pending: false,
        } = self.phase
        else {
            return Err("unexpected Q2 signon command");
        };
        if (current == kind && start < previous)
            || (current != kind
                && !(current == ResourceStream::ConfigStrings
                    && kind == ResourceStream::Baselines
                    && start == 0))
        {
            return Err("out-of-order Q2 signon command");
        }
        self.phase = Phase::List {
            kind,
            start,
            pending: true,
        };
        Ok(())
    }
    pub(crate) fn reply(&self) -> Option<StartupReply> {
        match self.phase {
            Phase::List {
                kind,
                start,
                pending: true,
            } => Some(StartupReply::Q2List {
                kind,
                server: self.server,
                start,
            }),
            Phase::Begin => Some(StartupReply::Q2Begin {
                server: self.server,
            }),
            _ => None,
        }
    }
    pub(crate) fn queued(&mut self, reply: StartupReply) {
        match reply {
            StartupReply::Q2List { .. } => {
                if let Phase::List { pending, .. } = &mut self.phase {
                    *pending = false;
                }
            }
            StartupReply::Q2Begin { .. } => self.phase = Phase::BeginQueued,
            _ => {}
        }
    }
}
