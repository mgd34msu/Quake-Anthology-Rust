//! SERVER userinfo revisions publish once; the ring owns delivery and retries.
use crate::Runtime;
use qa_core::{
    loopback::Endpoint,
    primitives::{ClientId, EntityId, NATIVE_PEER_ROUTE_BYTES, NativePeerRoute},
    text::{FixedText, InfoBytes, InfoRules, InfoUpdate, info_changes, info_value},
};
use qa_network::{commands::packet::Protocol, outputs::PeerUpdate};

impl Runtime {
    pub(crate) fn retain_peer_numbers(&mut self, lifetime: EntityId) {
        for numbers in &mut self.native_numbers {
            numbers.retain_player(lifetime);
        }
    }

    pub(crate) fn publish_userinfos(&mut self) {
        // The same connection-owned map serves snapshots, native cues and peers.
        for slot in 0..self.server.clients.len() {
            let id = ClientId(slot as u32);
            let Some(protocol) = self
                .network
                .get(id, Endpoint::Server)
                .and_then(|connection| connection.commands.as_ref())
                .map(|commands| commands.protocol)
            else {
                continue;
            };
            let native_module = self.server.entities.columns.native_entity
                [self.server.clients[slot].entity.slot as usize]
                .map(|native| native.module);
            self.native_numbers[slot].refresh(
                &self.server.entities,
                native_module,
                protocol,
                self.server
                    .clients
                    .iter()
                    .filter(|client| client.is_server_active())
                    .map(|client| client.entity),
            );
        }
        // Do not cancel this pass's own freshly captured peers on tiny rings.
        // Give a native reliable flight its resend interval before invoking
        // the ring's bounded slow-client policy; SERVER ticks never wait here.
        let mut quota = self
            .server
            .events
            .capacity()
            .saturating_sub(self.server.events.len());
        if quota == 0
            && self.input_time.since(self.userinfo_queued_at)
                >= qa_core::sys_events::NANOS_PER_SECOND
        {
            quota = 1;
        }
        let count = self.server.clients.len();
        let start = self.userinfo_cursor;
        let mut released_player = false;
        for offset in 0..count {
            let slot = (start + offset) % count;
            let id = ClientId(slot as u32);
            let Some(record) = self
                .server
                .userinfo_record(id)
                .filter(|record| record.dirty())
            else {
                continue;
            };
            if record.connected && !self.server.clients[slot].is_server_active() {
                continue;
            }
            let stamp = record.stamp;
            let connected = record.connected;
            let length = record.text.len();
            self.output_native_bytes[..length].copy_from_slice(record.text);
            let mut used = 0;
            let mut fits = true;
            for (receiver, client) in self.server.clients.iter().enumerate() {
                let receiver = ClientId(receiver as u32);
                if !client.is_server_active()
                    || !self
                        .network
                        .get(receiver, Endpoint::Server)
                        .and_then(|connection| connection.commands.as_ref())
                        .is_some_and(|commands| {
                            matches!(
                                commands.protocol,
                                Protocol::NetQuake15 | Protocol::QuakeWorld28
                            )
                        })
                {
                    continue;
                }
                let Some(reference) = self.native_numbers[receiver.0 as usize].peer_reference(
                    &self.server.entities,
                    stamp.lifetime,
                    receiver,
                ) else {
                    continue;
                };
                let Some(output) = self
                    .output_bytes
                    .get_mut(used..used + NATIVE_PEER_ROUTE_BYTES)
                else {
                    fits = false;
                    break;
                };
                let route = NativePeerRoute {
                    receiver: client.entity,
                    ordinal: reference.ordinal,
                    reset_count: self.native_numbers[receiver.0 as usize].peer_reset_count(),
                };
                if route.write_capture(output).is_none() {
                    fits = false;
                    break;
                }
                used += NATIVE_PEER_ROUTE_BYTES;
            }
            if !fits {
                self.server.events.reject_publication();
                continue;
            }
            if used != 0 && quota == 0 {
                continue;
            }
            if used != 0
                && self
                    .server
                    .events
                    .peer_info(
                        stamp.user_id,
                        &self.output_native_bytes[..length],
                        &self.output_bytes[..used],
                    )
                    .is_err()
            {
                continue;
            }
            self.server.mark_userinfo_queued(id, stamp);
            if used != 0 {
                quota -= 1;
                self.userinfo_cursor = (slot + 1) % count;
                self.userinfo_queued_at = self.input_time;
                // The captured reset and row now share one ring-owned record.
                for bytes in self.output_bytes[..used].chunks_exact(NATIVE_PEER_ROUTE_BYTES) {
                    if let Some(route) = NativePeerRoute::read_capture(bytes)
                        && let Some(receiver) = self.server.entities.client_index(route.receiver)
                    {
                        self.native_numbers[receiver].peers_seeded();
                    }
                }
            }
            if !connected {
                for numbers in &mut self.native_numbers {
                    numbers.release_retiring_player(stamp.lifetime);
                }
                released_player = true;
            }
        }
        if released_player {
            // A previously unrepresentable player may now acquire a native
            // hole. Its accepted bytes need publication even without a change.
            self.server.invalidate_userinfo_publications();
        }
    }
}

/// Exact native fields are selected after the semantic record has entered the
/// ring. QW permits a full userinfo replacement with the same admission id.
pub(super) fn encode(
    protocol: Protocol,
    route: NativePeerRoute,
    user_id: i32,
    info: &[u8],
    output: &mut [u8],
) -> Result<usize, ()> {
    let mut length = 0;
    for slot in 0..route.reset_count {
        length += encode_row(protocol, slot, 0, b"", &mut output[length..])?;
    }
    length += encode_row(
        protocol,
        route.ordinal,
        user_id,
        info,
        &mut output[length..],
    )?;
    Ok(length)
}

fn encode_row(
    protocol: Protocol,
    slot: u32,
    user_id: i32,
    info: &[u8],
    output: &mut [u8],
) -> Result<usize, ()> {
    let slot = u8::try_from(slot).map_err(|_| ())?;
    match protocol {
        Protocol::QuakeWorld28 => {
            let mut projected = FixedText::<{ qa_compat::quakec::QW_USERINFO_BYTES }>::default();
            let rules = InfoRules {
                filter: InfoBytes::AboveCarriageReturn,
                ..qa_compat::services::userinfo_rules(protocol.rule_set())
            };
            for (key, value) in info_changes(b"", info).filter(|(key, _)| !key.starts_with(b"_")) {
                if let Ok(update) = InfoUpdate::new(key, value, rules) {
                    // Larger module fields remain intact in SERVER. Only their
                    // native boundary projection may omit an unrepresentable row.
                    let _ = projected.update_info(update);
                }
            }
            qa_network::outputs::peer_update(
                protocol,
                PeerUpdate::UserInfo {
                    slot,
                    user_id,
                    info: projected.as_bytes(),
                },
                output,
            )
            .map_err(|_| ())
        }
        Protocol::NetQuake15 => {
            let name = info_value(info, b"name", false).map_or(&b""[..], |span| &info[span]);
            let length =
                qa_network::outputs::peer_update(protocol, PeerUpdate::Name { slot, name }, output)
                    .map_err(|_| ())?;
            let colors = qa_core::text::info_integer(info, b"topcolor") as u8;
            let colors =
                (colors & 15) << 4 | (qa_core::text::info_integer(info, b"bottomcolor") as u8 & 15);
            let rest = qa_network::outputs::peer_update(
                protocol,
                PeerUpdate::Colors { slot, colors },
                &mut output[length..],
            )
            .map_err(|_| ())?;
            Ok(length + rest)
        }
        _ => Err(()),
    }
}
