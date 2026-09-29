//! Quake II native server-demo footage.
//!
//! Donor provenance: `Q2ServerDemoState`, `Q2ServerDemoRecord`,
//! `encodeQ2ServerDemoSignon`, `encodeQ2ServerDemoFrame`, and
//! `readQ2ServerDemo` in `src/network/q2/server-demo.ts` (Quake II
//! `sv_ccmds.c` `SV_ServerRecord_f` / `sv_ents.c` `SV_RecordDemoMessage`).
//! Framing reuses [`read_q2_demo`](crate::demo::read_q2_demo), entity
//! codecs reuse [`Q2Wire`](crate::q2_net::Q2Wire), and multicast records
//! reuse [`Q2ServerMessageReader`](crate::q2_net::Q2ServerMessageReader).
//!
//! Native server demos are entity footage, not playerstate DM2: no
//! camera or player is invented.

use std::collections::{BTreeMap, HashSet};

use thiserror::Error;

use crate::demo::{read_q2_demo, DemoError};
use crate::msg::{MsgError, MsgWriter};
use crate::protocol::q2 as protocol;
use crate::protocol::ProtocolIdentity;
use crate::q2::{write_packet_entities_begin, EntityState, MAX_EDICTS};
use crate::q2_net::{Q2EntityBits, Q2NetError, Q2ServerMessageOptions, Q2ServerMessageReader, Q2ServerRecord, Q2Wire};

/// Server-demo message capacity (`createMessage(32768)`).
pub const Q2_SERVER_DEMO_MESSAGE_BYTES: usize = 32768;
/// Classic configstring space for server demos.
pub const Q2_SERVER_DEMO_CONFIG_STRINGS: u16 = 2080;
/// Player-number marker for footage without a player view.
const NO_PLAYER_VIEW: i16 = -1;
/// Serverdata marker byte identifying native server footage.
const SERVER_FOOTAGE_MARKER: u8 = 2;

/// Server-demo failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Q2ServerDemoError {
    /// Underlying demo framing failure.
    #[error("{0}")]
    Demo(#[from] DemoError),
    /// Underlying Q2 netcode failure.
    #[error("{0}")]
    Net(#[from] Q2NetError),
    /// Underlying message failure.
    #[error("{0}")]
    Msg(#[from] MsgError),
    /// Invalid encode input, with the donor's message text.
    #[error("{0}")]
    Range(&'static str),
    /// Malformed footage, with the donor's message text.
    #[error("{0}")]
    Protocol(&'static str),
}

/// Server-demo signon state (`Q2ServerDemoState`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2ServerDemoState {
    /// Server count.
    pub servercount: i32,
    /// Game directory.
    pub gamedir: String,
    /// Configstrings by index.
    pub config_strings: BTreeMap<u16, String>,
}

/// Decoded server-demo record (`Q2ServerDemoRecord`).
#[derive(Debug, Clone, PartialEq)]
pub enum Q2ServerDemoRecord {
    /// Signon preamble.
    Signon {
        /// Server state.
        state: Q2ServerDemoState,
    },
    /// Entity frame with multicast records.
    Frame {
        /// Server frame number.
        server_frame: i32,
        /// Frame entities in number order.
        entities: Vec<EntityState>,
        /// Multicast records after the entity block.
        multicasts: Vec<Q2ServerRecord>,
    },
}

/// Encode a server-demo signon (`encodeQ2ServerDemoSignon`).
///
/// Configstring 0 travels both as the bare level-name string and as an
/// indexed configstring, exactly like the donor.
pub fn encode_q2_server_demo_signon(state: &Q2ServerDemoState) -> Result<Vec<u8>, Q2ServerDemoError> {
    let mut writer = MsgWriter::new(Q2_SERVER_DEMO_MESSAGE_BYTES, false);
    writer.write_byte(protocol::Svc::Serverdata as u8)?;
    writer.write_long(protocol::PROTOCOL_VERSION as i32)?;
    writer.write_long(state.servercount)?;
    writer.write_byte(SERVER_FOOTAGE_MARKER)?;
    writer.write_string(&state.gamedir)?;
    writer.write_short(NO_PLAYER_VIEW)?;
    writer.write_string(state.config_strings.get(&0).map_or("", String::as_str))?;
    for (index, value) in &state.config_strings {
        if *index >= Q2_SERVER_DEMO_CONFIG_STRINGS {
            return Err(Q2ServerDemoError::Range(
                "Classic server demo configstring outside range",
            ));
        }
        if value.is_empty() {
            continue;
        }
        writer.write_byte(protocol::Svc::Configstring as u8)?;
        writer.write_short(*index as i16)?;
        writer.write_string(value)?;
    }
    Ok(writer.bytes().to_vec())
}

/// Encode a server-demo frame (`encodeQ2ServerDemoFrame`).
pub fn encode_q2_server_demo_frame(
    server_frame: i32,
    entities: &[EntityState],
    multicasts: &[impl AsRef<[u8]>],
) -> Result<Vec<u8>, Q2ServerDemoError> {
    if server_frame < 0 {
        return Err(Q2ServerDemoError::Range("Invalid server demo frame"));
    }
    let mut wire = Q2Wire::new(ProtocolIdentity::Q2Classic)?;
    let mut writer = MsgWriter::new(Q2_SERVER_DEMO_MESSAGE_BYTES, false);
    let zero = EntityState::default();
    writer.write_byte(protocol::Svc::Frame as u8)?;
    writer.write_long(server_frame)?;
    write_packet_entities_begin(&mut writer)?;
    let mut previous = 0u16;
    for entity in entities {
        if entity.number <= previous || entity.number >= MAX_EDICTS {
            return Err(Q2ServerDemoError::Range(
                "Classic server demo entities must be ordered and unique",
            ));
        }
        previous = entity.number;
        if entity.modelindex != 0 || entity.effects != 0 || entity.sound != 0 || entity.event != 0 {
            wire.write_delta_entity(&mut writer, &zero, entity, false, true)?;
        }
    }
    wire.write_packet_entities_end(&mut writer)?;
    for bytes in multicasts {
        writer.write_bytes(bytes.as_ref())?;
    }
    Ok(writer.bytes().to_vec())
}

/// Read server-demo footage (`readQ2ServerDemo`).
pub fn read_q2_server_demo(bytes: &[u8]) -> Result<Vec<Q2ServerDemoRecord>, Q2ServerDemoError> {
    let mut wire = Q2Wire::new(ProtocolIdentity::Q2Classic)?;
    let mut multicast = Q2ServerMessageReader::new(
        ProtocolIdentity::Q2Classic,
        Q2ServerMessageOptions {
            max_config_strings: Q2_SERVER_DEMO_CONFIG_STRINGS,
            inventory_slots: 256,
            ..Default::default()
        },
        HashSet::new(),
        None,
    )?;
    let mut signon = false;
    let mut records = Vec::new();
    for record in read_q2_demo(bytes)? {
        wire.begin(&record.bytes);
        let opcode = wire.read_raw_byte()?;
        if opcode == protocol::Svc::Serverdata as u8 {
            if wire.read_raw_long()? != protocol::PROTOCOL_VERSION as i32 {
                return Err(Q2ServerDemoError::Protocol("Server demo requires classic protocol34"));
            }
            let servercount = wire.read_raw_long()?;
            if wire.read_raw_byte()? != SERVER_FOOTAGE_MARKER {
                return Err(Q2ServerDemoError::Protocol("Not a native server demo"));
            }
            let gamedir = wire.read_raw_string(2047)?;
            if wire.read_raw_short()? != NO_PLAYER_VIEW {
                return Err(Q2ServerDemoError::Protocol("Server demo cannot contain a player view"));
            }
            wire.read_raw_string(2047)?;
            let mut config_strings = BTreeMap::new();
            while wire.remaining() > 0 {
                if wire.read_raw_byte()? != protocol::Svc::Configstring as u8 {
                    return Err(Q2ServerDemoError::Protocol("Invalid server demo signon opcode"));
                }
                let index = wire.read_raw_short()?;
                if index < 0 || index >= Q2_SERVER_DEMO_CONFIG_STRINGS as i16 {
                    return Err(Q2ServerDemoError::Protocol("Invalid server demo configstring"));
                }
                config_strings.insert(index as u16, wire.read_raw_string(2047)?);
            }
            wire.finish()?;
            signon = true;
            records.push(Q2ServerDemoRecord::Signon {
                state: Q2ServerDemoState {
                    servercount,
                    gamedir,
                    config_strings,
                },
            });
        } else if opcode == protocol::Svc::Frame as u8 && signon {
            let server_frame = wire.read_raw_long()?;
            if server_frame < 0 {
                return Err(Q2ServerDemoError::Protocol("Invalid server demo frame"));
            }
            wire.read_packet_entities_begin()?;
            let mut entities = Vec::new();
            let mut previous = 0u16;
            loop {
                let header = wire.read_entity_bits()?;
                if header.number == 0 {
                    break;
                }
                let bits = match &header.bits {
                    Q2EntityBits::Classic(bits) => *bits,
                    _ => return Err(Q2NetError::Protocol("Q2 entity bits disagree with protocol").into()),
                };
                if header.number <= previous || header.number >= MAX_EDICTS || (bits & protocol::U_REMOVE) != 0 {
                    return Err(Q2ServerDemoError::Protocol("Invalid full server demo entity"));
                }
                previous = header.number;
                entities.push(wire.read_delta_entity(&EntityState::default(), header.number, header)?);
            }
            let rest = record.bytes[wire.position()..].to_vec();
            let multicasts = multicast.read(&rest)?;
            records.push(Q2ServerDemoRecord::Frame {
                server_frame,
                entities,
                multicasts,
            });
        } else {
            return Err(Q2ServerDemoError::Protocol(
                "Server demo requires signon followed by entity frames",
            ));
        }
    }
    if !signon {
        return Err(Q2ServerDemoError::Protocol("Server demo has no signon"));
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo::{finish_q2_demo, write_q2_demo_record};

    fn demo_state() -> Q2ServerDemoState {
        Q2ServerDemoState {
            servercount: 5,
            gamedir: "baseq2".to_owned(),
            config_strings: BTreeMap::from([
                (0, "q2dm1".to_owned()),
                (1, "maxclients\\8".to_owned()),
                (5, String::new()),
            ]),
        }
    }

    fn demo_entity(number: u16) -> EntityState {
        EntityState {
            number,
            modelindex: 1,
            origin: [100.0, 200.0, 300.0],
            ..EntityState::default()
        }
    }

    fn demo_bytes() -> Vec<u8> {
        let mut bytes = write_q2_demo_record(&encode_q2_server_demo_signon(&demo_state()).unwrap());
        bytes.extend_from_slice(&write_q2_demo_record(
            &encode_q2_server_demo_frame(7, &[demo_entity(1), demo_entity(3)], &[Vec::<u8>::new()]).unwrap(),
        ));
        bytes.extend_from_slice(&finish_q2_demo());
        bytes
    }

    #[test]
    fn signon_prefix_is_byte_exact() {
        let bytes = encode_q2_server_demo_signon(&demo_state()).unwrap();
        assert_eq!(
            &bytes[..15],
            &[12, 34, 0, 0, 0, 5, 0, 0, 0, 2, b'b', b'a', b's', b'e', b'q']
        );
        // Player number -1 follows the NUL-terminated gamedir.
        let gamedir_start = bytes.windows(6).position(|window| window == b"baseq2").unwrap();
        assert_eq!(bytes[gamedir_start + 6], 0);
        assert_eq!(&bytes[gamedir_start + 7..gamedir_start + 9], &[0xFF, 0xFF]);
    }

    #[test]
    fn footage_round_trip() {
        let records = read_q2_server_demo(&demo_bytes()).unwrap();
        assert_eq!(records.len(), 2);
        let Q2ServerDemoRecord::Signon { state } = &records[0] else {
            panic!("expected a signon first");
        };
        assert_eq!(state.servercount, 5);
        assert_eq!(state.gamedir, "baseq2");
        assert_eq!(state.config_strings.get(&0).map(String::as_str), Some("q2dm1"));
        assert_eq!(state.config_strings.get(&1).map(String::as_str), Some("maxclients\\8"));
        // Empty configstrings are skipped on encode.
        assert!(!state.config_strings.contains_key(&5));
        let Q2ServerDemoRecord::Frame {
            server_frame,
            entities,
            multicasts,
        } = &records[1]
        else {
            panic!("expected a frame second");
        };
        assert_eq!(*server_frame, 7);
        assert_eq!(entities.len(), 2);
        assert_eq!(entities[0].number, 1);
        assert_eq!(entities[0].origin, [100.0, 200.0, 300.0]);
        assert_eq!(entities[1].number, 3);
        assert!(multicasts.is_empty());
    }

    #[test]
    fn unordered_entities_fail_encode() {
        assert_eq!(
            encode_q2_server_demo_frame(1, &[demo_entity(3), demo_entity(1)], &[Vec::<u8>::new()]),
            Err(Q2ServerDemoError::Range(
                "Classic server demo entities must be ordered and unique"
            ))
        );
        assert_eq!(
            encode_q2_server_demo_frame(1, &[demo_entity(0)], &[Vec::<u8>::new()]),
            Err(Q2ServerDemoError::Range(
                "Classic server demo entities must be ordered and unique"
            ))
        );
        assert_eq!(
            encode_q2_server_demo_frame(-1, &[], &[Vec::<u8>::new()]),
            Err(Q2ServerDemoError::Range("Invalid server demo frame"))
        );
    }

    #[test]
    fn oversized_configstring_fails_encode() {
        let mut state = demo_state();
        state.config_strings.insert(2080, "nope".to_owned());
        assert_eq!(
            encode_q2_server_demo_signon(&state),
            Err(Q2ServerDemoError::Range(
                "Classic server demo configstring outside range"
            ))
        );
    }

    #[test]
    fn frame_before_signon_fails() {
        let frame = encode_q2_server_demo_frame(7, &[demo_entity(1)], &[Vec::<u8>::new()]).unwrap();
        let mut bytes = write_q2_demo_record(&frame);
        bytes.extend_from_slice(&finish_q2_demo());
        assert_eq!(
            read_q2_server_demo(&bytes),
            Err(Q2ServerDemoError::Protocol(
                "Server demo requires signon followed by entity frames"
            ))
        );
        assert_eq!(
            read_q2_server_demo(&finish_q2_demo()),
            Err(Q2ServerDemoError::Protocol("Server demo has no signon"))
        );
    }

    #[test]
    fn corrupt_signon_fails() {
        let mut signon = encode_q2_server_demo_signon(&demo_state()).unwrap();
        // Wrong protocol version.
        signon[1] = 35;
        let mut bytes = write_q2_demo_record(&signon);
        bytes.extend_from_slice(&finish_q2_demo());
        assert_eq!(
            read_q2_server_demo(&bytes),
            Err(Q2ServerDemoError::Protocol("Server demo requires classic protocol34"))
        );
    }
}
