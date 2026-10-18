//! Connection startup owns native phases; filenames remain in ConfigStrings.
use super::*;
mod q2;
pub(crate) use q2::{FileCursor as Q2FileCursor, Q2Signon};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResourceKind {
    Sound,
    Model,
}
impl ResourceKind {
    pub(crate) fn range(self, layout: ConfigLayout) -> qa_core::primitives::IndexSpan {
        layout.resources[usize::from(self == Self::Sound)]
    }
}

#[derive(Clone, Copy)]
pub(crate) enum StartupReply {
    Prespawn,
    Name,
    Color,
    Spawn,
    Begin,
    List {
        kind: ResourceKind,
        server: i32,
        start: u32,
    },
    QwPrespawn {
        server: i32,
        buffer: u32,
        checksum: Option<u32>,
    },
    QwSpawn {
        server: i32,
        start: u32,
    },
    QwBegin {
        server: i32,
    },
    Q2List {
        kind: qa_network::commands::startup::ResourceStream,
        server: i32,
        start: u32,
    },
    Q2Begin {
        server: i32,
    },
    New,
}

#[derive(Clone, Copy)]
pub(crate) enum ServerControl {
    Skins,
    Changing,
    Reconnect,
}

#[derive(Default)]
pub(crate) enum ClientStartup {
    #[default]
    Unmanaged,
    NetQuake(NqSignon),
    AwaitHeader {
        new_pending: bool,
    },
    Restarting,
    Quake2(Q2Signon),
    Quake3(Q3Phase),
    QuakeWorld(QwSignon),
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Q3Phase {
    AwaitGamestate,
    Downloading,
    Loading,
    Primed,
    Active,
}

#[derive(Default)]
pub(crate) struct NqSignon {
    pub(super) stage: u8,
    observed: u8,
    stage2_step: u8,
    resources_admitted: bool,
}

pub(crate) struct QwSignon {
    server: i32,
    pub(crate) game_directory:
        qa_core::text::FixedText<{ qa_network::message::LEGACY_STRING_BYTES }>,
    server_info: qa_core::text::FixedText<512>,
    phase: QwPhase,
}
#[derive(Clone, Copy)]
enum QwPhase {
    List {
        kind: ResourceKind,
        end: u32,
        pending: bool,
    },
    Files {
        kind: ResourceKind,
        ordinal: u32,
        end: u32,
    },
    Content,
    Reply(StartupReply),
    Signon,
    Skins {
        prepared: bool,
    },
    BeginQueued,
    Active,
}

impl ClientStartup {
    pub(crate) fn begin_q2(
        &mut self,
        data: &qa_network::commands::startup::Q2ServerData<'_>,
    ) -> Result<(), &'static str> {
        if data.game_directory.windows(2).any(|pair| pair == b"..")
            || data
                .game_directory
                .iter()
                .any(|byte| matches!(byte, b'/' | b'\\' | b':'))
        {
            return Err("Q2 game directory must be a single relative component");
        }
        *self = Self::Quake2(Q2Signon::new(data)?);
        Ok(())
    }
    pub(crate) fn resource_stream(
        &mut self,
        kind: qa_network::commands::startup::ResourceStream,
    ) -> Result<(), &'static str> {
        match self {
            Self::Unmanaged => Ok(()),
            Self::Quake2(state) => state.stream_complete(kind),
            _ => Err("Q2 stream before serverdata"),
        }
    }
    pub(crate) fn q2(&self) -> Option<&Q2Signon> {
        match self {
            Self::Quake2(state) => Some(state),
            _ => None,
        }
    }
    pub(crate) fn q2_mut(&mut self) -> Option<&mut Q2Signon> {
        match self {
            Self::Quake2(state) => Some(state),
            _ => None,
        }
    }
    pub(crate) fn connecting(&mut self, protocol: Protocol) {
        match protocol {
            Protocol::NetQuake15 => self.begin_nq(),
            Protocol::Quake3_68 => *self = Self::Quake3(Q3Phase::AwaitGamestate),
            Protocol::QuakeWorld28 | Protocol::Quake2_34 | Protocol::Quake2Repro1038 => {
                *self = Self::AwaitHeader { new_pending: false }
            }
        }
    }
    pub(crate) fn connected(&mut self, protocol: Protocol) {
        if protocol == Protocol::Quake3_68 {
            self.connecting(protocol);
            return;
        }
        if matches!(
            protocol,
            Protocol::QuakeWorld28 | Protocol::Quake2_34 | Protocol::Quake2Repro1038
        ) {
            *self = Self::AwaitHeader { new_pending: true };
        }
    }
    pub(crate) fn restart_connection(&mut self) {
        *self = Self::Restarting;
    }
    pub(crate) fn reconnect_in_place(&self) -> bool {
        match self {
            Self::AwaitHeader { .. } | Self::NetQuake(_) => true,
            Self::QuakeWorld(state) => !matches!(state.phase, QwPhase::Active),
            Self::Quake2(state) => state.reconnect_in_place(),
            Self::Unmanaged | Self::Restarting | Self::Quake3(_) => false,
        }
    }
    pub(crate) fn reconnect(&mut self, protocol: Protocol) {
        self.connecting(protocol);
        self.connected(protocol);
    }
    pub(crate) fn changing(&mut self, protocol: Protocol) {
        self.connecting(protocol);
    }
    pub(crate) fn begin_nq(&mut self) {
        *self = Self::NetQuake(NqSignon::default());
    }
    pub(crate) fn begin_qw(
        &mut self,
        data: &qa_network::commands::startup::QwServerData<'_>,
    ) -> Result<(), &'static str> {
        if data.game_directory.windows(2).any(|pair| pair == b"..")
            || data
                .game_directory
                .iter()
                .any(|byte| matches!(byte, b'/' | b'\\' | b':'))
        {
            return Err("QW game directory must be a single relative component");
        }
        let mut game_directory = qa_core::text::FixedText::default();
        game_directory
            .set_bytes(data.game_directory)
            .map_err(|_| "QW game directory capacity")?;
        *self = Self::QuakeWorld(QwSignon {
            server: data.server_count,
            game_directory,
            server_info: Default::default(),
            phase: QwPhase::List {
                kind: ResourceKind::Sound,
                end: 0,
                pending: true,
            },
        });
        Ok(())
    }
    pub(crate) fn game_directory(&self) -> Option<&[u8]> {
        match self {
            Self::QuakeWorld(state) => Some(state.game_directory.as_bytes()),
            Self::Quake2(state) => Some(state.game_directory.as_bytes()),
            _ => None,
        }
    }
    pub(crate) fn server_info(&self) -> Option<&[u8]> {
        match self {
            Self::QuakeWorld(state) => Some(state.server_info.as_bytes()),
            _ => None,
        }
    }
    pub(crate) fn nq_stage(&self) -> Option<u8> {
        match self {
            Self::NetQuake(state) => Some(state.stage),
            _ => None,
        }
    }
    pub(crate) fn waiting(&self) -> bool {
        match self {
            Self::Unmanaged => false,
            Self::AwaitHeader { .. } | Self::Restarting => true,
            Self::NetQuake(state) => state.stage < 4 || !state.resources_admitted,
            Self::QuakeWorld(state) => !matches!(state.phase, QwPhase::Active),
            Self::Quake2(state) => state.waiting(),
            Self::Quake3(phase) => *phase != Q3Phase::Active,
        }
    }
    pub(crate) fn resources_needed(&self) -> bool {
        match self {
            Self::NetQuake(state) => !state.resources_admitted,
            Self::QuakeWorld(state) => matches!(state.phase, QwPhase::Content),
            Self::Quake2(state) => state.resources_needed(),
            Self::Unmanaged | Self::AwaitHeader { .. } | Self::Restarting | Self::Quake3(_) => {
                false
            }
        }
    }
    pub(crate) fn allow_models(&self) -> bool {
        if let Self::Quake2(state) = self {
            return state.allows_media();
        }
        !matches!(
            self,
            Self::AwaitHeader { .. }
                | Self::Restarting
                | Self::Quake3(Q3Phase::AwaitGamestate | Q3Phase::Downloading)
                | Self::QuakeWorld(QwSignon {
                    phase: QwPhase::List { .. } | QwPhase::Files { .. },
                    ..
                })
        )
    }
    pub(crate) fn allow_sounds(&self) -> bool {
        if let Self::Quake2(state) = self {
            return state.allows_media();
        }
        !matches!(
            self,
            Self::AwaitHeader { .. }
                | Self::Restarting
                | Self::Quake3(Q3Phase::AwaitGamestate | Q3Phase::Downloading)
                | Self::QuakeWorld(QwSignon {
                    phase: QwPhase::List {
                        kind: ResourceKind::Sound,
                        ..
                    } | QwPhase::Files {
                        kind: ResourceKind::Sound,
                        ..
                    },
                    ..
                })
        )
    }
    pub(crate) fn allow_skins(&self) -> bool {
        !matches!(
            self,
            Self::AwaitHeader { .. }
                | Self::Restarting
                | Self::Quake3(Q3Phase::AwaitGamestate | Q3Phase::Downloading)
        ) && !matches!(self, Self::QuakeWorld(state) if !matches!(state.phase, QwPhase::Skins { .. } | QwPhase::BeginQueued | QwPhase::Active))
    }
    pub(crate) fn q3(&self) -> Option<Q3Phase> {
        match self {
            Self::Quake3(phase) => Some(*phase),
            _ => None,
        }
    }
    pub(crate) fn q3_wait_gamestate(&mut self) {
        if self.q3().is_some() {
            self.connecting(Protocol::Quake3_68);
        }
    }
    pub(crate) fn q3_gamestate(&mut self) {
        if matches!(self, Self::Quake3(_)) {
            *self = Self::Quake3(Q3Phase::Downloading);
        }
    }
    pub(crate) fn q3_packages_ready(&mut self) {
        if self.q3() == Some(Q3Phase::Downloading) {
            *self = Self::Quake3(Q3Phase::Loading);
        }
    }
    pub(crate) fn q3_primed(&mut self) {
        if matches!(self, Self::Quake3(Q3Phase::Loading)) {
            *self = Self::Quake3(Q3Phase::Primed);
        }
    }
    pub(crate) fn q3_snapshot(&mut self, flags: u8) {
        if flags & qa_network::snapshots::Q3_NOT_ACTIVE == 0
            && matches!(self, Self::Quake3(Q3Phase::Primed))
        {
            *self = Self::Quake3(Q3Phase::Active);
        }
    }
    pub(crate) fn allows_commands(&self) -> bool {
        !self.waiting() || matches!(self, Self::Quake3(Q3Phase::Primed))
    }
    pub(crate) fn receive_nq_stage(&mut self, stage: u8) -> bool {
        if matches!(self, Self::Unmanaged) {
            self.begin_nq();
        }
        let Self::NetQuake(state) = self else {
            return false;
        };
        if stage <= state.stage || stage > 4 {
            return false;
        }
        state.stage = stage;
        if stage < 4 {
            state.observed |= 1 << stage;
        }
        true
    }
    pub(crate) fn receive_entity(&mut self, protocol: Protocol) {
        match self {
            Self::Quake2(state)
                if matches!(protocol, Protocol::Quake2_34 | Protocol::Quake2Repro1038) =>
            {
                state.receive_entity()
            }
            Self::NetQuake(state) if protocol == Protocol::NetQuake15 && state.stage == 3 => {
                state.stage = 4
            }
            Self::QuakeWorld(state)
                if protocol == Protocol::QuakeWorld28
                    && matches!(state.phase, QwPhase::BeginQueued) =>
            {
                state.phase = QwPhase::Active
            }
            _ => {}
        }
    }
    pub(crate) fn resources_admitted(&mut self, checksum: Option<u32>) -> Result<(), &'static str> {
        match self {
            Self::NetQuake(state) => state.resources_admitted = true,
            Self::Quake2(state) => state.resources_admitted(),
            Self::QuakeWorld(state) if matches!(state.phase, QwPhase::Content) => {
                let checksum = checksum.ok_or("QW world has no native BSP checksum")?;
                state.phase = QwPhase::Reply(StartupReply::QwPrespawn {
                    server: state.server,
                    buffer: 0,
                    checksum: Some(checksum),
                });
            }
            _ => {}
        }
        Ok(())
    }
    pub(crate) fn resource_list(
        &mut self,
        progress: qa_network::outputs::ResourceListProgress,
        layout: ConfigLayout,
    ) -> Result<(), &'static str> {
        // Explicitly bound clients also receive single-row native config
        // updates through this reader. Only managed startup sequences lists.
        if matches!(
            self,
            Self::Unmanaged
                | Self::QuakeWorld(QwSignon {
                    phase: QwPhase::Active,
                    ..
                })
        ) {
            return Ok(());
        }
        let Self::QuakeWorld(state) = self else {
            return Err("QW list before serverdata");
        };
        let QwPhase::List { kind, end, .. } = state.phase else {
            return Err("unexpected QW resource list");
        };
        if progress.range != kind.range(layout)
            || progress.first != end
            || progress.end < end
            || (progress.next != 0 && progress.next != progress.end)
        {
            return Err("non-contiguous QW resource list");
        }
        state.phase = if progress.next == 0 {
            QwPhase::Files {
                kind,
                ordinal: 1,
                end: progress.end,
            }
        } else {
            QwPhase::List {
                kind,
                end: progress.end,
                pending: true,
            }
        };
        Ok(())
    }
    pub(crate) fn file_cursor(&self) -> Option<(ResourceKind, u32, u32)> {
        match self {
            Self::QuakeWorld(QwSignon {
                phase: QwPhase::Files { kind, ordinal, end },
                ..
            }) => Some((*kind, *ordinal, *end)),
            _ => None,
        }
    }
    pub(crate) fn advance_file(&mut self) {
        if let Self::QuakeWorld(QwSignon {
            phase: QwPhase::Files { ordinal, .. },
            ..
        }) = self
        {
            *ordinal += 1;
        }
    }
    pub(crate) fn files_complete(&mut self) {
        if let Self::QuakeWorld(state) = self
            && let QwPhase::Files { kind, .. } = state.phase
        {
            state.phase = match kind {
                ResourceKind::Sound => QwPhase::List {
                    kind: ResourceKind::Model,
                    end: 0,
                    pending: true,
                },
                ResourceKind::Model => QwPhase::Content,
            };
        }
    }
    pub(crate) fn skins_prepared(&mut self) {
        if let Self::QuakeWorld(QwSignon {
            phase: QwPhase::Skins { prepared },
            ..
        }) = self
        {
            *prepared = true;
        }
    }
    pub(crate) fn invalidate_skins(&mut self) {
        if let Self::QuakeWorld(QwSignon {
            phase: QwPhase::Skins { prepared },
            ..
        }) = self
        {
            *prepared = false;
        }
    }
    pub(crate) fn reply(&self, skins_done: bool) -> Option<StartupReply> {
        match self {
            Self::Unmanaged | Self::Restarting | Self::Quake3(_) => None,
            Self::AwaitHeader { new_pending, .. } => new_pending.then_some(StartupReply::New),
            Self::Quake2(state) => state.reply(),
            Self::NetQuake(state) if state.observed & 2 != 0 => Some(StartupReply::Prespawn),
            Self::NetQuake(state) if state.observed & 4 != 0 => Some(match state.stage2_step {
                0 => StartupReply::Name,
                1 => StartupReply::Color,
                _ => StartupReply::Spawn,
            }),
            Self::NetQuake(state) if state.observed & 8 != 0 => Some(StartupReply::Begin),
            Self::NetQuake(_) => None,
            Self::QuakeWorld(state) => match state.phase {
                QwPhase::List {
                    kind,
                    end,
                    pending: true,
                } => Some(StartupReply::List {
                    kind,
                    server: state.server,
                    start: end,
                }),
                QwPhase::Reply(reply) => Some(reply),
                QwPhase::Skins { prepared: true } if skins_done => Some(StartupReply::QwBegin {
                    server: state.server,
                }),
                _ => None,
            },
        }
    }
    pub(crate) fn queued(&mut self, reply: StartupReply) {
        if matches!(reply, StartupReply::New) {
            if let Self::AwaitHeader { new_pending, .. } = self {
                *new_pending = false;
            }
            return;
        }
        match self {
            Self::Quake2(state) => state.queued(reply),
            Self::NetQuake(state) => match reply {
                StartupReply::Prespawn => state.observed &= !2,
                StartupReply::Name => state.stage2_step = 1,
                StartupReply::Color => state.stage2_step = 2,
                StartupReply::Spawn => state.observed &= !4,
                StartupReply::Begin => state.observed &= !8,
                _ => {}
            },
            Self::QuakeWorld(state) => match reply {
                StartupReply::List { .. } => {
                    if let QwPhase::List { pending, .. } = &mut state.phase {
                        *pending = false;
                    }
                }
                StartupReply::QwBegin { .. } => state.phase = QwPhase::BeginQueued,
                _ => state.phase = QwPhase::Signon,
            },
            Self::Unmanaged | Self::AwaitHeader { .. } | Self::Restarting | Self::Quake3(_) => {}
        }
    }
    pub(crate) fn stufftext(
        &mut self,
        args: qa_console::command_text::Arguments<'_>,
        layout: ConfigLayout,
        entity_limit: u32,
    ) -> Result<Option<ServerControl>, &'static str> {
        if args.len() == 1 {
            match args.get_bytes(0) {
                b"changing" => return Ok(Some(ServerControl::Changing)),
                b"reconnect" => return Ok(Some(ServerControl::Reconnect)),
                _ => {}
            }
        }
        if args.is_empty() {
            return Ok(None);
        }
        if let Self::Quake2(state) = self {
            state.command(args, layout, entity_limit)?;
            return Ok(None);
        }
        let Self::QuakeWorld(state) = self else {
            return Err("native command before serverdata");
        };
        match args.get_bytes(0) {
            b"" => {}
            b"fullserverinfo" if args.len() == 2 => {
                state
                    .server_info
                    .set_bytes(args.get_bytes(1))
                    .map_err(|_| "QW server info capacity")?;
            }
            b"cmd" if args.len() == 4 && matches!(args.get_bytes(1), b"prespawn" | b"spawn") => {
                let server = args
                    .get(2)
                    .parse::<i32>()
                    .map_err(|_| "invalid QW server count")?;
                let value = args
                    .get(3)
                    .parse::<u32>()
                    .map_err(|_| "invalid QW signon ordinal")?;
                if server != state.server {
                    return Err("stale QW signon command");
                }
                if !matches!(state.phase, QwPhase::Signon) {
                    return Err("QW signon command before content admission");
                }
                state.phase = QwPhase::Reply(if args.get_bytes(1) == b"prespawn" {
                    StartupReply::QwPrespawn {
                        server,
                        buffer: value,
                        checksum: None,
                    }
                } else {
                    StartupReply::QwSpawn {
                        server,
                        start: value,
                    }
                });
            }
            b"skins" if args.len() == 1 => {
                if !matches!(state.phase, QwPhase::Signon | QwPhase::Active) {
                    return Err("QW skins before signon");
                }
                if !matches!(state.phase, QwPhase::Active) {
                    state.phase = QwPhase::Skins { prepared: false };
                }
                return Ok(Some(ServerControl::Skins));
            }
            _ => return Err("unsupported QW server command"),
        }
        Ok(None)
    }
}
