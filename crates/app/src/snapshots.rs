//! Native local snapshot submission and CLIENT field application.
use qa_core::{
    events::OutputConsumerId,
    primitives::{PlayerState, ThinkTime},
    sys_events::SeatId,
};
use qa_network::{
    channel::Channel,
    commands::packet::{Error, Protocol, ZERO_QW},
    ingress::Incoming,
    message::{Encoding, Writer},
    projection::{PlayerContext, PlayerProjection},
    snapshots::{Frame, ReceivedFrame},
    states,
};

/// Connection policy and projection metadata; player storage remains in the
/// existing SERVER array and independently owned CLIENT prediction state.
pub struct LocalSnapshot {
    pub seat: SeatId,
    pub output: OutputConsumerId,
    pub protocol: Protocol,
    pub applied: u64,
    pub rejected: u64,
    native_client: u32,
    projection: PlayerProjection,
    context: PlayerContext,
    last_sent: Option<u32>,
    pub pending: Option<u32>,
}

impl LocalSnapshot {
    pub fn load(
        seat: SeatId,
        output: OutputConsumerId,
        protocol: Protocol,
        native_client: u32,
    ) -> Self {
        Self {
            seat,
            output,
            protocol,
            applied: 0,
            rejected: 0,
            native_client,
            projection: PlayerProjection::load(protocol, &[]),
            context: PlayerContext {
                client_number: Some(native_client),
                ..Default::default()
            },
            last_sent: None,
            pending: None,
        }
    }

    pub fn sequence(&self, channel: &Channel, time: qa_core::sys_events::EventTime) -> u32 {
        match self.protocol {
            Protocol::NetQuake15 => channel.send_state().datagram_sequence,
            // Protocol 34 derives time from serverframe * 100; publish only at
            // representable boundaries, independently of the world tick rate.
            Protocol::Quake2_34 => (time.milliseconds() / 100) as u32,
            Protocol::Quake2Repro1038 => channel.q2_repro_frame_ms().map_or(0, |duration| {
                (time.milliseconds() / u64::from(duration)) as u32
            }),
            _ => channel.send_state().sequence,
        }
    }

    pub fn encode(
        &self,
        channel: &mut Channel,
        player: &PlayerState,
        sequence: u32,
        time: qa_core::sys_events::EventTime,
        delta: Option<u32>,
        bytes: &mut [u8],
    ) -> Result<usize, Error> {
        let time = if self.protocol == Protocol::NetQuake15 {
            ThinkTime::Seconds(time.seconds())
        } else {
            ThinkTime::Milliseconds(time.milliseconds() as i64)
        };
        let mut words = [0; states::PLAYER_WORDS];
        // The provider has no native module/entity namespace yet. Do not turn
        // a common ground/weapon handle into an invented wire ordinal.
        let mut context = PlayerContext {
            client_number: Some(self.native_client),
            body_yaw: player.view_angles.0[1],
            ..Default::default()
        };
        // Native self playerinfo omits PF_COMMAND/PF_MSEC and includes only
        // nonzero velocity components (qsrc QW server/sv_ents.c).
        for (axis, value) in player.body.velocity.0.iter().enumerate() {
            if *value != 0. {
                context.player_info_flags |= 1 << (axis + 2);
            }
        }
        if !self.projection.reduce(player, &context, &mut words) {
            return Err(Error::Context);
        }
        macro_rules! frame {
            ($variant:ident, $n:expr) => {
                ReceivedFrame::$variant(Frame {
                    sequence,
                    time,
                    command: 0,
                    flags: 0,
                    areas: &[],
                    player: words[..$n].try_into().map_err(|_| Error::Count)?,
                    entities: &[],
                })
            };
        }
        channel.publish_snapshot(match self.protocol {
            Protocol::NetQuake15 => frame!(NetQuake, states::NQ_PLAYER_WORDS),
            Protocol::QuakeWorld28 => frame!(QuakeWorld, 0),
            Protocol::Quake2_34 => frame!(Quake2, states::Q2_PLAYER_WORDS),
            Protocol::Quake2Repro1038 => frame!(Quake2Repro, states::Q2_REPRO_PLAYER_WORDS),
            Protocol::Quake3_68 => frame!(Quake3, states::PLAYER_WORDS),
        })?;
        let body = |writer: &mut Writer<'_>, channel: &mut Channel| {
            if self.protocol == Protocol::QuakeWorld28 {
                states::write_qw_player(
                    writer,
                    self.native_client,
                    words[..states::QW_PLAYER_WORDS]
                        .try_into()
                        .map_err(|_| Error::Count)?,
                    ZERO_QW,
                )?;
            }
            channel.write_snapshot(writer, sequence, delta, 0)
        };
        if self.protocol == Protocol::Quake3_68 {
            channel.encode_server_output(bytes, body)
        } else {
            let mut writer = Writer::new(bytes, Encoding::Bytes);
            body(&mut writer, channel)?;
            Ok(writer.size())
        }
    }

    pub fn needs_send(&self, channel: &Channel, sequence: u32) -> bool {
        // Original QW replies follow an admitted client sequence, never an
        // unsolicited frame that gets ahead of the client's first command.
        (self.protocol != Protocol::QuakeWorld28 || channel.state().sequence != 0)
            && self.last_sent != Some(sequence)
    }

    pub fn submitted(&mut self, sequence: u32) {
        self.last_sent = Some(sequence);
    }

    pub fn apply(&mut self, incoming: Incoming<'_>, player: &mut PlayerState) {
        let words: Option<&[u32]> = match incoming {
            Incoming::Snapshot(frame) => match (self.protocol, frame) {
                (Protocol::NetQuake15, ReceivedFrame::NetQuake(frame)) => Some(frame.player),
                (Protocol::Quake2_34, ReceivedFrame::Quake2(frame)) => Some(frame.player),
                (Protocol::Quake2Repro1038, ReceivedFrame::Quake2Repro(frame)) => {
                    Some(frame.player)
                }
                (Protocol::Quake3_68, ReceivedFrame::Quake3(frame)) => Some(frame.player),
                _ => None,
            },
            Incoming::PlayerInfo(ref info)
                if self.protocol == Protocol::QuakeWorld28
                    && u32::from(info.number) == self.native_client =>
            {
                Some(&info.words)
            }
            _ => None,
        };
        if let Some(words) = words {
            // Native entity binding is deferred; an ordinal cannot resolve by
            // casting it to a common EntityId or selecting the executing module.
            if self
                .projection
                .apply(words, player, &mut self.context, |_| None)
            {
                self.applied += 1;
            } else {
                self.rejected += 1;
            }
        }
    }
}
