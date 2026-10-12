//! A native dupe key is scoped to one physical presentation, not an IP address.
use qa_core::{
    events::{EventRing, OutputConsumerId, OutputRecord, OutputSubmission},
    primitives::ClientId,
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
    module: u16,
    lifetime: (u32, u32),
    frame: u64,
    dupe: u32,
    group: PhysicalGroup,
}
struct Entry {
    owner: OutputConsumerId,
    sequence: u64,
    sent: bool,
    superseded: bool,
    retain_until: u64,
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
        self.entries
            .retain(|(key, _)| key.group != PhysicalGroup::Peer(client));
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
    pub fn sweep(&mut self, events: &EventRing) {
        let head = events.sequence_range().start;
        self.entries.retain(|(_, entry)| {
            if entry.sent {
                !entry.superseded || head < entry.retain_until
            } else {
                events.retained_for(entry.owner, entry.sequence)
            }
        });
    }
    pub fn admit(
        &mut self,
        record: OutputRecord,
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
            module: delivery.module.0,
            lifetime: (
                emission.module_lifetime.slot,
                emission.module_lifetime.generation,
            ),
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
            let keep = !entry.sent || !entry.superseded || retained.start < entry.retain_until;
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
            return if entry.sent {
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
                owner,
                sequence: record.sequence,
                sent: false,
                superseded: false,
                retain_until: retained.end,
            },
        ));
        Admission::Send
    }
    /// Reliable queue admission retains its claim until the ring observes ACK.
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
            if entry.owner != owner || entry.sequence != sequence || entry.sent {
                return true;
            }
            match submission {
                OutputSubmission::Reliable(_) => true,
                OutputSubmission::BestEffort if completed => {
                    entry.sent = true;
                    true
                }
                OutputSubmission::Unsent if retained_packet => true,
                _ => false,
            }
        });
    }
    pub fn acknowledged(&mut self, owner: OutputConsumerId, sequence: u64) {
        for (_, entry) in &mut self.entries {
            if entry.owner == owner && entry.sequence == sequence {
                entry.sent = true;
            }
        }
    }
}
