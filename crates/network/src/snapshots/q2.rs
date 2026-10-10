use super::{Frame, Ring, SLOTS, Slot, read_areas, read_entities};
use crate::{
    commands::packet,
    message::{Reader, Writer},
    states,
};
use qa_core::primitives::ThinkTime;

const KEX_PLAYER: usize = 42 + states::Q2_RR_STATS;
pub type Q2KexRing = Ring<KEX_PLAYER, { states::Q2_RERELEASE_ENTITY_WORDS }>;
pub type Q2KexFrame<'a> = Frame<'a, KEX_PLAYER, { states::Q2_RERELEASE_ENTITY_WORDS }>;

/// Endpoint-owned KEX decoder metadata. Snapshot bases remain in Ring; these
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
                .map(|words| states::Q2KexWire {
                    nonzero_solid: words[19] != 0,
                    baseline_solid: words[19] != 0,
                })
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
        *wire = states::Q2KexWire {
            nonzero_solid: words[19] != 0,
            baseline_solid: words[19] != 0,
        };
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
        |reader, from, _| {
            let mut old = states::Q2KexPlayer::default();
            let fields = old.words.len();
            old.words.copy_from_slice(&from[..fields]);
            old.stats.copy_from_slice(&from[fields..]);
            let decoded = states::read_q2_kex_player(reader, &old)?;
            let mut words = [0; KEX_PLAYER];
            words[..fields].copy_from_slice(&decoded.words);
            words[fields..].copy_from_slice(&decoded.stats);
            Ok(words)
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
        0,
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
