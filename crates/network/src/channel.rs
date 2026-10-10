//! One connected receive channel. Policy is negotiated at the connection
//! boundary; geometry, movement and module identities never select framing.
use crate::headers::{self, Direction, Format, Header, QPort, datagram};
use qa_core::{
    loopback::Endpoint,
    payloads::{PayloadQueue, QueueError},
    sys_events::EventTime,
};

pub mod commands;
mod transmit;
pub use transmit::{Prepared, SendState, TransmitError, Unreliable};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Policy {
    format: Format,
    datagram: bool,
    toggle_ack: bool,
    early_ack: bool,
    reliable_bias: u32,
    fragment_payload: usize,
    fragment_inclusive: bool,
    command_ack: bool,
}
pub const NETQUAKE: Policy = Policy {
    format: headers::NETQUAKE,
    datagram: true,
    toggle_ack: false,
    early_ack: false,
    reliable_bias: 0,
    fragment_payload: 0,
    fragment_inclusive: false,
    command_ack: false,
};
pub const QUAKEWORLD: Policy = Policy {
    reliable_bias: 1,
    ..q2_old(QPort::Short)
};
pub const QUAKE2: Policy = QUAKEWORLD;
pub const QUAKE3: Policy = Policy {
    format: headers::QUAKE3,
    datagram: false,
    toggle_ack: false,
    early_ack: false,
    reliable_bias: 0,
    fragment_payload: 1300,
    fragment_inclusive: true,
    command_ack: true,
};
pub const fn q2_old(qport: QPort) -> Policy {
    Policy {
        format: headers::q2_old(qport),
        datagram: false,
        toggle_ack: true,
        early_ack: false,
        reliable_bias: 0,
        fragment_payload: 0,
        fragment_inclusive: false,
        command_ack: false,
    }
}
pub const fn q2_new(qport_present: bool) -> Policy {
    Policy {
        format: headers::q2_new(qport_present),
        datagram: false,
        toggle_ack: true,
        early_ack: true,
        reliable_bias: 0,
        fragment_payload: 1300,
        fragment_inclusive: false,
        command_ack: false,
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct State {
    /// NetQuake's next reliable sequence; other channels' last accepted one.
    pub sequence: u32,
    pub next_datagram: u32,
    pub acknowledged: u32,
    pub reliable_acknowledged: bool,
    pub reliable_sequence: bool,
    pub fragment_sequence: u32,
    pub fragment_bytes: usize,
    pub dropped: u32,
    pub last_received: EventTime,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub delivered: u64,
    pub pending: u64,
    pub stale: u64,
    pub fragment_order: u64,
    pub malformed: u64,
    pub control_full: u64,
    pub dropped: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Header(headers::Error),
    Capacity,
    MessageTooLarge,
    ControlFull,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "channel receive {self:?}")
    }
}
impl std::error::Error for Error {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delivery<'a> {
    Ignored,
    Pending,
    Payload(&'a [u8]),
    /// A native control, not a validated transmit ACK or an output receipt.
    Control,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Received<'a> {
    pub header: Header,
    pub delivery: Delivery<'a>,
}

pub struct Channel {
    policy: Policy,
    direction: Direction,
    state: State,
    counts: Counts,
    assembly: Box<[u8]>,
    controls: PayloadQueue<Header>,
    transmit: transmit::Transmit,
    commands: Option<commands::CommandMessages>,
    snapshots: Option<Box<crate::snapshots::Q3Ring>>,
}

impl Channel {
    pub(crate) fn policy(&self) -> Policy {
        self.policy
    }
    /// The negotiated module/protocol boundary supplies its maximum message.
    /// Native QW receive limits may exceed our future 1400-byte transmit limit.
    pub fn load(
        policy: Policy,
        endpoint: Endpoint,
        maximum_message: usize,
        controls: usize,
    ) -> Result<Self, Error> {
        if !(1..=64 * 1024 * 1024).contains(&maximum_message) {
            return Err(Error::Capacity);
        }
        Ok(Self {
            policy,
            direction: match endpoint {
                Endpoint::Client => Direction::ToClient,
                Endpoint::Server => Direction::ToServer,
            },
            state: State::default(),
            counts: Counts::default(),
            assembly: vec![0; maximum_message].into_boxed_slice(),
            controls: PayloadQueue::load(controls, 1).map_err(|_| Error::Capacity)?,
            transmit: transmit::Transmit::load(policy, maximum_message, endpoint)?,
            commands: policy.command_ack.then(commands::CommandMessages::load),
            snapshots: if policy.command_ack && endpoint == Endpoint::Client {
                Some(Box::new(
                    crate::snapshots::Q3Ring::load(1023, 1024, 32, Some(2048 - 128))
                        .map_err(|_| Error::Capacity)?,
                ))
            } else {
                None
            },
        })
    }

    pub fn state(&self) -> State {
        self.state
    }
    pub fn snapshot(&self, sequence: u32) -> Option<crate::snapshots::Q3Frame<'_>> {
        self.snapshots.as_ref()?.frame(sequence)
    }
    pub fn snapshots_mut(&mut self) -> Option<&mut crate::snapshots::Q3Ring> {
        self.snapshots.as_deref_mut()
    }
    pub fn endpoint(&self) -> Endpoint {
        match self.direction {
            Direction::ToClient => Endpoint::Client,
            Direction::ToServer => Endpoint::Server,
        }
    }
    pub fn counts(&self) -> Counts {
        self.counts
    }
    pub fn next_control(&mut self) -> Option<Header> {
        self.controls.pop().map(|(header, _)| header)
    }
    pub fn pending_controls(&self) -> usize {
        self.controls.len()
    }

    /// Only the host's system-event dispatch calls this; it does not poll an OS
    /// source. Delivered slices borrow the packet or this channel's assembly.
    pub fn receive<'a>(
        &'a mut self,
        packet: &'a [u8],
        time: EventTime,
    ) -> Result<Received<'a>, Error> {
        self.transmit.receipt_count = 0;
        let (header, payload) = match headers::decode(self.policy.format, self.direction, packet) {
            Ok(decoded) => decoded,
            Err(error) => {
                self.counts.malformed = self.counts.malformed.saturating_add(1);
                return Err(Error::Header(error));
            }
        };
        if self.policy.datagram {
            return self.receive_datagram(header, payload, time);
        }
        if header.sequence <= self.state.sequence {
            self.counts.stale = self.counts.stale.saturating_add(1);
            return Ok(Received {
                header,
                delivery: Delivery::Ignored,
            });
        }
        self.state.dropped = header.sequence - self.state.sequence - 1;
        // Q2pro observes the toggle before even a rejected/pending fragment.
        if self.policy.early_ack {
            self.state.reliable_acknowledged = header.reliable_ack;
            self.transmit.ack_toggle(header.reliable_ack);
        }
        let assembled = if let Some(fragment) = header.fragment {
            if header.sequence != self.state.fragment_sequence {
                self.state.fragment_sequence = header.sequence;
                self.state.fragment_bytes = 0;
            }
            if usize::from(fragment.offset) != self.state.fragment_bytes {
                self.counts.fragment_order = self.counts.fragment_order.saturating_add(1);
                return Ok(Received {
                    header,
                    delivery: Delivery::Ignored,
                });
            }
            self.append(payload)?;
            if fragment.more {
                self.counts.pending = self.counts.pending.saturating_add(1);
                return Ok(Received {
                    header,
                    delivery: Delivery::Pending,
                });
            }
            Some(self.state.fragment_bytes)
        } else {
            if payload.len() > self.assembly.len() {
                return self.too_large();
            }
            None
        };
        self.state.sequence = header.sequence;
        if self.policy.toggle_ack {
            self.state.acknowledged = header.acknowledgement;
            self.state.reliable_acknowledged = header.reliable_ack;
            self.state.reliable_sequence ^= header.reliable;
            if !self.policy.early_ack {
                self.transmit.ack_toggle(header.reliable_ack);
            }
        }
        self.state.last_received = time;
        self.counts.delivered = self.counts.delivered.saturating_add(1);
        self.counts.dropped = self
            .counts
            .dropped
            .saturating_add(u64::from(self.state.dropped));
        let payload = if let Some(length) = assembled {
            self.state.fragment_bytes = 0;
            &self.assembly[..length]
        } else {
            payload
        };
        Ok(Received {
            header,
            delivery: Delivery::Payload(payload),
        })
    }

    fn receive_datagram<'a>(
        &'a mut self,
        header: Header,
        payload: &'a [u8],
        time: EventTime,
    ) -> Result<Received<'a>, Error> {
        // Preserve native UNRELIABLE -> ACK -> DATA flag precedence.
        if header.datagram_flags & datagram::UNRELIABLE != 0 {
            if header.sequence < self.state.next_datagram {
                self.counts.stale = self.counts.stale.saturating_add(1);
                return Ok(Received {
                    header,
                    delivery: Delivery::Ignored,
                });
            }
            if payload.len() > self.assembly.len() {
                return self.too_large();
            }
            self.state.dropped = header.sequence - self.state.next_datagram;
            self.counts.dropped = self
                .counts
                .dropped
                .saturating_add(u64::from(self.state.dropped));
            self.state.next_datagram = header.sequence.wrapping_add(1);
            self.state.last_received = time;
            self.counts.delivered = self.counts.delivered.saturating_add(1);
            return Ok(Received {
                header,
                delivery: Delivery::Payload(payload),
            });
        }
        if header.datagram_flags & datagram::ACK != 0 {
            self.transmit.ack_datagram(header.sequence);
            return Ok(Received {
                header,
                delivery: Delivery::Control,
            });
        }
        if header.datagram_flags & datagram::DATA == 0 {
            return Ok(Received {
                header,
                delivery: Delivery::Ignored,
            });
        }
        let control = Header {
            sequence: header.sequence,
            datagram_flags: datagram::ACK,
            ..Header::default()
        };
        if let Err(QueueError::Full | QueueError::PayloadFull) = self.controls.push(control, &[], 0)
        {
            self.counts.control_full = self.counts.control_full.saturating_add(1);
            return Err(Error::ControlFull);
        }
        if header.sequence != self.state.sequence {
            self.counts.stale = self.counts.stale.saturating_add(1);
            return Ok(Received {
                header,
                delivery: Delivery::Ignored,
            });
        }
        self.append(payload)?;
        self.state.sequence = self.state.sequence.wrapping_add(1);
        if header.datagram_flags & datagram::EOM == 0 {
            self.counts.pending = self.counts.pending.saturating_add(1);
            return Ok(Received {
                header,
                delivery: Delivery::Pending,
            });
        }
        let length = self.state.fragment_bytes;
        self.state.fragment_bytes = 0;
        self.state.last_received = time;
        self.counts.delivered = self.counts.delivered.saturating_add(1);
        Ok(Received {
            header,
            delivery: Delivery::Payload(&self.assembly[..length]),
        })
    }

    fn append(&mut self, payload: &[u8]) -> Result<(), Error> {
        let start = self.state.fragment_bytes;
        if payload.len() > self.assembly.len() - start {
            return self.too_large();
        }
        self.assembly[start..start + payload.len()].copy_from_slice(payload);
        self.state.fragment_bytes += payload.len();
        Ok(())
    }
    fn too_large<T>(&mut self) -> Result<T, Error> {
        self.counts.malformed = self.counts.malformed.saturating_add(1);
        Err(Error::MessageTooLarge)
    }
}
