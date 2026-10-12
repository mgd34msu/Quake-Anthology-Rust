//! A native dupe key is scoped to one physical presentation, not an IP address.
use qa_core::{
    events::{EventRing, NativeReceipt, OutputConsumerId, OutputRecord, OutputSubmission},
    primitives::{ClientId, EntityId, ModuleId},
};
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhysicalGroup {
    Engine,
    Peer(ClientId),
    Negotiated(u32),
}
#[derive(Clone, Copy, PartialEq, Eq)]
struct Key {
    module: ModuleId,
    lifetime: EntityId,
    frame: u64,
    dupe: u32,
    group: PhysicalGroup,
}
struct Entry {
    client: ClientId,
    owner: OutputConsumerId,
    sequence: u64,
    state: State,
    superseded: bool,
    retain_until: u64,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Claimed,
    Packet,
    Reliable(NativeReceipt),
    Sent,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admission {
    Send,
    Pending,
    Suppressed,
    Full,
}

pub struct Coordinator {
    entries: Vec<(Key, Entry)>,
    limit: usize,
    negotiated: Box<[Option<u32>]>,
}
impl Coordinator {
    pub fn load(capacity: usize, clients: usize) -> Self {
        Self {
            entries: Vec::with_capacity(capacity),
            limit: capacity,
            negotiated: vec![None; clients].into_boxed_slice(),
        }
    }
    /// Set only from explicit physical-client negotiation, never peer address.
    pub fn negotiate(&mut self, client: ClientId, group: Option<u32>) -> bool {
        let Some(slot) = self.negotiated.get_mut(client.0 as usize) else {
            return false;
        };
        *slot = group;
        true
    }
    /// A reusable ClientId is not the lifetime of its former remote peer.
    pub fn reset_client(&mut self, client: ClientId) {
        self.negotiate(client, None);
        self.entries.retain(|(key, entry)| {
            key.group != PhysicalGroup::Peer(client)
                && (entry.client != client || entry.state == State::Sent)
        });
    }
    pub fn group(&self, client: ClientId, local: bool) -> PhysicalGroup {
        if local {
            PhysicalGroup::Engine
        } else {
            self.negotiated
                .get(client.0 as usize)
                .copied()
                .flatten()
                .map_or(PhysicalGroup::Peer(client), PhysicalGroup::Negotiated)
        }
    }
    /// A ring cancellation cannot cancel bytes already owned by a channel.
    /// None names its exact pending output packet; Some names an admitted
    /// reliable receipt, which lives until native ACK or channel teardown.
    pub fn sweep(
        &mut self,
        events: &EventRing,
        mut retained: impl FnMut(ClientId, OutputConsumerId, u64, Option<NativeReceipt>) -> bool,
    ) {
        let head = events.sequence_range().start;
        self.entries.retain(|(_, entry)| match entry.state {
            State::Sent => !entry.superseded || head < entry.retain_until,
            State::Claimed => events.retained_for(entry.owner, entry.sequence),
            State::Packet | State::Reliable(_) => {
                let receipt = match entry.state {
                    State::Reliable(receipt) => Some(receipt),
                    _ => None,
                };
                events.retained_for(entry.owner, entry.sequence)
                    || retained(entry.client, entry.owner, entry.sequence, receipt)
            }
        });
    }
    pub fn admit(
        &mut self,
        record: OutputRecord,
        client: ClientId,
        owner: OutputConsumerId,
        group: PhysicalGroup,
        retained: Range<u64>,
    ) -> Admission {
        let Some(delivery) = record.delivery else {
            return Admission::Send;
        };
        if delivery.dupe_key == 0 {
            return Admission::Send;
        }
        let Some(emission) = delivery.emission else {
            return Admission::Send;
        };
        let key = Key {
            module: delivery.module,
            lifetime: emission.module_lifetime,
            frame: emission.provider_frame,
            dupe: delivery.dupe_key,
            group,
        };
        let mut found = None;
        let mut index = 0;
        self.entries.retain_mut(|(previous, entry)| {
            if previous.module == key.module
                && (previous.lifetime != key.lifetime || previous.frame < key.frame)
            {
                entry.superseded = true;
            }
            let keep = entry.state != State::Sent
                || !entry.superseded
                || retained.start < entry.retain_until;
            if keep {
                if *previous == key {
                    found = Some(index);
                }
                index += 1;
            }
            keep
        });
        if let Some(index) = found {
            let entry = &mut self.entries[index].1;
            entry.retain_until = entry.retain_until.max(retained.end);
            return if entry.state == State::Sent {
                Admission::Suppressed
            } else if entry.owner == owner && entry.sequence == record.sequence {
                Admission::Send
            } else {
                Admission::Pending
            };
        }
        // The load-sized Vec never grows: retain compacts in place and every
        // insertion is guarded by the exact reserved entry limit.
        if self.entries.len() == self.limit {
            return Admission::Full;
        }
        self.entries.push((
            key,
            Entry {
                client,
                owner,
                sequence: record.sequence,
                state: State::Claimed,
                superseded: false,
                retain_until: retained.end,
            },
        ));
        Admission::Send
    }
    /// Reliable queue admission retains its claim until a validated native ACK.
    /// Best effort completes only when all native fragments were submitted.
    pub fn submitted(
        &mut self,
        owner: OutputConsumerId,
        sequence: u64,
        submission: OutputSubmission,
        completed: bool,
        retained_packet: bool,
    ) {
        self.entries.retain_mut(|(_, entry)| {
            if entry.owner != owner || entry.sequence != sequence || entry.state == State::Sent {
                return true;
            }
            match submission {
                OutputSubmission::Reliable(receipt) => {
                    entry.state = State::Reliable(receipt);
                    true
                }
                OutputSubmission::BestEffort if completed => {
                    entry.state = State::Sent;
                    true
                }
                OutputSubmission::Unsent if retained_packet => {
                    entry.state = State::Packet;
                    true
                }
                _ => false,
            }
        });
    }
    /// Called only for a validated native channel receipt, independently of
    /// whether bounded ring resync already cancelled its publication slot.
    pub fn acknowledged(&mut self, owner: OutputConsumerId, receipt: NativeReceipt) {
        for (_, entry) in &mut self.entries {
            if entry.owner == owner && entry.state == State::Reliable(receipt) {
                entry.state = State::Sent;
            }
        }
    }
}
