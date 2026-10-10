use super::{
    Frame, MergeRules, Ring, SLOTS, Slot, delta_frame, native_entities, read_areas, read_entities,
    write_entities,
};
use crate::{
    commands::packet,
    message::{Reader, Writer},
    states,
};
use qa_core::primitives::ThinkTime;

const KEX_PLAYER: usize = 42 + states::Q2_RR_STATS;
const REPRO_PLAYER: usize = 43 + states::Q2_RR_STATS;
pub type Q2KexRing = Ring<KEX_PLAYER, { states::Q2_RERELEASE_ENTITY_WORDS }>;
pub type Q2KexFrame<'a> = Frame<'a, KEX_PLAYER, { states::Q2_RERELEASE_ENTITY_WORDS }>;
pub type Q2ReproRing = Ring<REPRO_PLAYER, { states::Q2_RERELEASE_ENTITY_WORDS }>;
pub type Q2ReproFrame<'a> = Frame<'a, REPRO_PLAYER, { states::Q2_RERELEASE_ENTITY_WORDS }>;

/// Endpoint-owned KEX wire metadata. Snapshot bases remain in Ring; these
/// small native columns select wire widths and reset to registered baselines.
pub struct Q2KexContext {
    demo: bool,
    entities: Box<[states::Q2KexWire]>,
}
impl Q2KexContext {
    pub fn load(ring: &Q2KexRing, demo: bool) -> Result<Self, packet::Error> {
        if ring.baselines.len() > 8192 {
            return Err(packet::Error::Count);
        }
        Ok(Self {
            demo,
            entities: ring
                .baselines
                .iter()
                .map(|words| states::Q2KexWire::from_baseline(words[19]))
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        })
    }
    pub fn set_baseline(
        &mut self,
        ring: &mut Q2KexRing,
        number: u32,
        words: &[u32; states::Q2_RERELEASE_ENTITY_WORDS],
    ) -> bool {
        let Some(wire) = self.entities.get_mut(number as usize) else {
            return false;
        };
        if !ring.set_baseline(number, words) {
            return false;
        }
        *wire = states::Q2KexWire::from_baseline(words[19]);
        true
    }
}

/// Native svc_frame prefix. Protocol 34 and KEX retain full frame numbers;
/// protocol 1038 packs a 27-bit frame and a five-bit delta offset. These are
/// wire columns, independent of the engine's timeline and player storage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Q2Header {
    pub sequence: u32,
    pub delta: i32,
    pub flags: u8,
    pub player_flags: u8,
}

pub(super) struct ReadRules {
    pub area_limit: usize,
    pub entity_limit: usize,
    pub entity_opcode: bool,
    pub extended_header: bool,
    pub valid_base: bool,
}

/// Native per-connection entity policy, independent of common client slots.
#[derive(Clone, Copy, Default)]
pub struct Q2EntityPolicy {
    pub native_clients: u32,
    pub first_person: Option<u32>,
    pub beam_old_origin_fix: bool,
}

pub(super) struct WriteRules {
    pub area_limit: usize,
    pub entity_limit: u32,
    pub entity_opcode: bool,
}

pub fn write_q2_kex(
    writer: &mut Writer<'_>,
    ring: &mut Q2KexRing,
    context: &mut Q2KexContext,
    sequence: u32,
    delta_request: Option<u32>,
    policy: Q2EntityPolicy,
) -> Result<(), packet::Error> {
    prepare_first_person(ring, sequence, delta_request, policy.first_person)?;
    write_records::<false, KEX_PLAYER, { states::Q2_RERELEASE_ENTITY_WORDS }>(
        writer,
        ring,
        sequence,
        delta_request,
        WriteRules {
            area_limit: 255,
            entity_limit: 8192,
            entity_opcode: true,
        },
        |writer, from, to| {
            states::write_q2_kex_player(
                writer,
                &player_from_words::<42, KEX_PLAYER>(from),
                &player_from_words::<42, KEX_PLAYER>(to),
            )
            .map(|()| 0)
        },
        |writer, number, from, to, force| {
            let wire = context
                .entities
                .get_mut(number as usize)
                .ok_or(crate::message::Error {
                    byte: writer.size(),
                    kind: crate::message::ErrorKind::Width,
                })?;
            states::write_q2_extended_entity::<true>(
                writer,
                number as u16,
                from,
                to,
                states::Q2EntityEncoding {
                    old_origin: write_old_origin(number, from, to, force, policy),
                    demo: context.demo,
                    force,
                },
                wire,
            )
        },
    )
}

pub fn write_q2_repro(
    writer: &mut Writer<'_>,
    ring: &mut Q2ReproRing,
    sequence: u32,
    delta_request: Option<u32>,
    policy: Q2EntityPolicy,
) -> Result<(), packet::Error> {
    prepare_first_person(ring, sequence, delta_request, policy.first_person)?;
    write_records::<true, REPRO_PLAYER, { states::Q2_RERELEASE_ENTITY_WORDS }>(
        writer,
        ring,
        sequence,
        delta_request,
        WriteRules {
            area_limit: 255,
            entity_limit: 8192,
            entity_opcode: false,
        },
        |writer, from, to| {
            states::write_q2_repro_player(
                writer,
                &player_from_words::<43, REPRO_PLAYER>(from),
                &player_from_words::<43, REPRO_PLAYER>(to),
            )
        },
        |writer, number, from, to, force| {
            states::write_q2_extended_entity::<false>(
                writer,
                number as u16,
                from,
                to,
                states::Q2EntityEncoding {
                    old_origin: write_old_origin(number, from, to, force, policy),
                    demo: false,
                    force,
                },
                &mut states::Q2KexWire::default(),
            )
        },
    )
}

fn prepare_first_person<const P: usize>(
    ring: &mut Ring<P, { states::Q2_RERELEASE_ENTITY_WORDS }>,
    sequence: u32,
    request: Option<u32>,
    number: Option<u32>,
) -> Result<(), packet::Error> {
    ring.frame(sequence).ok_or(packet::Error::Context)?;
    if let Some(number) = number
        && let Some(old) = delta_frame(ring, sequence, request)
        && let Ok(index) = old.entities.binary_search_by_key(&number, |e| e.number)
    {
        let previous = old.entities[index].words;
        let slot = sequence as usize & (SLOTS - 1);
        let start = slot * ring.capacity;
        let current = &mut ring.entities[start..start + ring.slots[slot].count];
        if let Ok(index) = current.binary_search_by_key(&number, |e| e.number) {
            // Native emit_packet_entities updates the retained client frame,
            // not only the temporary delta, when suppressing its own pose.
            current[index].words[8..14].copy_from_slice(&previous[8..14]);
        }
    }
    Ok(())
}

fn write_old_origin(
    number: u32,
    from: &[u32; states::Q2_RERELEASE_ENTITY_WORDS],
    to: Option<&[u32; states::Q2_RERELEASE_ENTITY_WORDS]>,
    force: bool,
    policy: Q2EntityPolicy,
) -> bool {
    let Some(to) = to else {
        return false;
    };
    if !force && policy.first_person == Some(number) {
        return false;
    }
    ((force || number <= policy.native_clients || to[7] & 64 != 0) && to[14..17] != from[8..11])
        || (to[7] & 128 != 0 && (!policy.beam_old_origin_fix || to[14..17] != from[14..17]))
}

/// Shared Q2 frame prefix, scalar player records and ordered entity emission.
pub(super) fn write_records<const PACKED: bool, const P: usize, const E: usize>(
    writer: &mut Writer<'_>,
    ring: &Ring<P, E>,
    sequence: u32,
    request: Option<u32>,
    rules: WriteRules,
    player: impl FnOnce(&mut Writer<'_>, &[u32; P], &[u32; P]) -> Result<u8, crate::message::Error>,
    entity: impl FnMut(
        &mut Writer<'_>,
        u32,
        &[u32; E],
        Option<&[u32; E]>,
        bool,
    ) -> Result<bool, crate::message::Error>,
) -> Result<(), packet::Error> {
    let to = ring.frame(sequence).ok_or(packet::Error::Context)?;
    let from = delta_frame(ring, sequence, request);
    let start = writer.size();
    Q2Header {
        sequence,
        delta: from.map_or(-1, |f| f.sequence as i32),
        flags: to.flags,
        player_flags: 0,
    }
    .write::<PACKED>(writer, &to.areas[..to.areas.len().min(rules.area_limit)])?;
    let flags = player(writer, from.map_or(&[0; P], |f| f.player), to.player)?;
    if PACKED {
        writer.patch_byte(start + 6, flags)?;
    }
    if rules.entity_opcode {
        writer.write_bits(18, 8)?;
    }
    write_entities(
        writer,
        from.map_or(&[][..], |f| {
            native_entities(f.entities, 1, rules.entity_limit)
        }),
        native_entities(to.entities, 1, rules.entity_limit),
        &ring.baselines,
        entity,
    )?;
    writer.write_bits(0, 16)?;
    Ok(())
}

fn player_from_words<const F: usize, const P: usize>(
    words: &[u32; P],
) -> states::Q2RereleasePlayer<F> {
    let mut player = states::Q2RereleasePlayer::default();
    player.words.copy_from_slice(&words[..F]);
    player.stats.copy_from_slice(&words[F..]);
    player
}

/// KEX frame after svc_frame; the context selects retail 2023 or demo 2022
/// coordinates. Native clock conversion is supplied by the connection, independently
/// of map format or movement. No alternate player or entity store is created.
pub fn read_q2_kex(
    reader: &mut Reader<'_>,
    ring: &mut Q2KexRing,
    context: &mut Q2KexContext,
    time: impl FnOnce(u32) -> ThinkTime,
) -> Result<bool, packet::Error> {
    read_records::<false, KEX_PLAYER, { states::Q2_RERELEASE_ENTITY_WORDS }>(
        reader,
        ring,
        ReadRules {
            area_limit: 255,
            entity_limit: 8192,
            entity_opcode: true,
            extended_header: true,
            valid_base: true,
        },
        |reader, from, flags| {
            read_player(reader, from, flags, |reader, from, _| {
                states::read_q2_kex_player(reader, from)
            })
        },
        |reader, header, from| {
            let wire = context
                .entities
                .get_mut(usize::from(header.number))
                .ok_or(packet::Error::Count)?;
            Ok(states::read_q2_extended_entity_body::<true>(
                reader,
                header,
                from,
                context.demo,
                wire,
            )?
            .words)
        },
        |from| states::q2_unchanged_entity(from, true),
        time,
    )
}

/// Q2repro 1038 body after svc_frame. The packed prefix's extra flags belong
/// to the player table; this format has no player/packetentities opcodes.
pub fn read_q2_repro(
    reader: &mut Reader<'_>,
    ring: &mut Q2ReproRing,
    time: impl FnOnce(u32) -> ThinkTime,
) -> Result<bool, packet::Error> {
    read_records::<true, REPRO_PLAYER, { states::Q2_RERELEASE_ENTITY_WORDS }>(
        reader,
        ring,
        ReadRules {
            area_limit: 255,
            entity_limit: 8192,
            entity_opcode: false,
            extended_header: true,
            valid_base: true,
        },
        |reader, from, flags| read_player(reader, from, flags, states::read_q2_repro_player),
        |reader, header, from| {
            Ok(states::read_q2_extended_entity_body::<false>(
                reader,
                header,
                from,
                false,
                &mut states::Q2KexWire::default(),
            )?
            .words)
        },
        |from| states::q2_unchanged_entity(from, true),
        time,
    )
}

fn read_player<const F: usize, const P: usize>(
    reader: &mut Reader<'_>,
    from: &[u32; P],
    flags: u8,
    decode: impl FnOnce(
        &mut Reader<'_>,
        &states::Q2RereleasePlayer<F>,
        u8,
    ) -> Result<states::Q2RereleasePlayer<F>, crate::message::Error>,
) -> Result<[u32; P], crate::message::Error> {
    let old = player_from_words(from);
    let decoded = decode(reader, &old, flags)?;
    let mut words = [0; P];
    words[..F].copy_from_slice(&decoded.words);
    words[F..].copy_from_slice(&decoded.stats);
    Ok(words)
}

/// The one Q2 frame receive path. The scalar record tables and native boundary
/// rules vary; base validation, retention, merging and publication do not.
pub(super) fn read_records<const PACKED: bool, const P: usize, const E: usize>(
    reader: &mut Reader<'_>,
    ring: &mut Ring<P, E>,
    rules: ReadRules,
    player: impl FnOnce(&mut Reader<'_>, &[u32; P], u8) -> Result<[u32; P], crate::message::Error>,
    body: impl FnMut(
        &mut Reader<'_>,
        states::EntityHeader,
        &[u32; E],
    ) -> Result<Option<[u32; E]>, packet::Error>,
    unchanged: impl Fn(&[u32; E]) -> [u32; E],
    time: impl FnOnce(u32) -> ThinkTime,
) -> Result<bool, packet::Error> {
    let (header, area_bytes) =
        Q2Header::read::<PACKED>(reader, &mut ring.scratch_areas, rules.area_limit)?;
    if rules.valid_base && (header.sequence as i32) < 0 {
        return Err(packet::Error::Count);
    }
    let base_index = header.delta as usize & (SLOTS - 1);
    let full = header.delta <= 0;
    let old = if full {
        Slot::ZERO
    } else {
        ring.slots[base_index]
    };
    // Classic CL_ParseFrame ignores old.valid; the enhanced client also
    // rejects an invalid base and a delta from the current frame.
    let base_valid = full
        || (old.sequence == Some(header.delta as u32)
            && (!rules.valid_base || (old.valid && header.delta as u32 != header.sequence))
            && !ring
                .retained_rows
                .is_some_and(|n| ring.parsed_rows.saturating_sub(old.first_entity) > n));
    let player = player(reader, &old.player, header.player_flags)?;
    if rules.entity_opcode && reader.read_bits(8)? != 18 {
        return Err(packet::Error::Opcode);
    }
    let (count, overflow) = read_entities(
        reader,
        &ring.entities[base_index * ring.capacity..base_index * ring.capacity + old.count],
        &ring.baselines,
        &mut ring.scratch,
        MergeRules {
            terminator: 0,
            remove_advances_old: true,
        },
        |reader| {
            let header = states::read_q2_entity_prefix(reader, rules.extended_header)?;
            if usize::from(header.number) >= rules.entity_limit {
                return Err(packet::Error::Count);
            }
            Ok(header)
        },
        body,
        unchanged,
    )?;
    Ok(ring.publish_received(
        Slot {
            sequence: Some(header.sequence),
            time: time(header.sequence),
            flags: header.flags,
            area_bytes,
            count,
            player,
            ..Slot::ZERO
        },
        base_valid,
        overflow,
        true,
    ))
}

impl Q2Header {
    /// Body after svc_frame. Areas are borrowed connection-owned scratch;
    /// the supplied limit is the negotiated protocol's native area capacity.
    pub fn read<const PACKED: bool>(
        reader: &mut Reader<'_>,
        areas: &mut [u8],
        area_limit: usize,
    ) -> Result<(Self, usize), packet::Error> {
        let encoded = reader.read_bits(32)?;
        let (sequence, delta) = if PACKED {
            let offset = encoded >> 27;
            let sequence = encoded & 0x07ff_ffff;
            (
                sequence,
                if offset == 31 {
                    -1
                } else {
                    sequence.wrapping_sub(offset) as i32
                },
            )
        } else {
            (encoded, reader.read_bits(32)? as i32)
        };
        let mut flags = reader.read_bits(8)? as u8;
        let player_flags = if PACKED {
            flags &= 0x0f;
            reader.read_bits(8)? as u8
        } else {
            0
        };
        let area_bytes = read_areas(reader, areas, area_limit)?;
        Ok((
            Self {
                sequence,
                delta,
                flags,
                player_flags,
            },
            area_bytes,
        ))
    }

    /// Writes svc_frame and its area mask, leaving the native player/entity
    /// records to the existing scalar field walker.
    pub fn write<const PACKED: bool>(
        self,
        writer: &mut Writer<'_>,
        areas: &[u8],
    ) -> Result<(), packet::Error> {
        if areas.len() > 255 {
            return Err(packet::Error::Count);
        }
        writer.write_bits(20, 8)?;
        if PACKED {
            let offset = if self.delta == -1 {
                31
            } else {
                self.sequence.wrapping_sub(self.delta as u32)
            };
            writer.write_bits((self.sequence & 0x07ff_ffff) | (offset << 27), 32)?;
        } else {
            writer.write_bits(self.sequence, 32)?;
            writer.write_bits(self.delta as u32, 32)?;
        }
        writer.write_bits(u32::from(self.flags), 8)?;
        if PACKED {
            writer.write_bits(u32::from(self.player_flags), 8)?;
        }
        writer.write_bits(areas.len() as u32, 8)?;
        writer.write_data(areas)?;
        Ok(())
    }
}
