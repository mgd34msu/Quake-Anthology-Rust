//! Quake II service writers, client messages, MVD/GTV, and checksums.
//!
//! Donor provenance: `src/network/q2/{checksum,server-write,client-messages,
//! mvd-encoding,mvd-recording,mvd-playback,mvd-broadcast,gtv,gtv-transport}.ts`.
//!
//! Promise-based donor servers become synchronous poll pumps over [`std::net`]
//! sockets; every state transition keeps the donor's order and limits. Encoding
//! reuses [`Q2Wire`] dispatch so the selected protocol owns every extension
//! tail.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

use flate2::{Compress, Compression, Decompress, FlushCompress, FlushDecompress, Status};

use crate::msg::{MsgError, MsgReader, MsgWriter};
use crate::protocol::q2 as protocol;
use crate::protocol::ProtocolIdentity;
use crate::q2::{angle_to_short, write_dir, EntityState, PlayerState, Usercmd};
use crate::q2_net::{
    Q2EntityBits, Q2EntityHeader, Q2NetError, Q2ServerData, Q2ServerEvent, Q2ServerMessageOptions,
    Q2ServerMessageReader, Q2ServerRecord, Q2SoundMessage, Q2TempField, Q2TempInt, Q2TempType, Q2TempVec, Q2Wire,
    Q2WireFrame,
};
use crate::q2_variants::{
    fog_bits, mvd_profile, q2pro_fog_bits, read_mvd_cmd, read_mvd_header, read_mvd_player, read_q2pro_entity,
    read_q2pro_entity_bits, write_delta_mvd_playerstate, write_mvd_cmd, write_q2pro_fog, write_q2pro_int23,
    write_q2pro_var64, BatchMove, BatchMoveFrame, FogData, GtvClientOp, GtvServerOp, MvdHeader, MvdOp, MvdPlayer,
    MvdProfile, MvdProtocol, Q2ProFeatures, RereleaseCodec, WideEntityBits, CLIENTNUM_NONE, GTF_DEFLATE,
    GTF_STRINGCMDS, GTV_PROTOCOL_VERSION, MAX_GTC_MSGLEN, MVD_MAGIC, PPS_BLEND, PPS_FOV, PPS_GUNANGLES, PPS_GUNOFFSET,
    PPS_KICKANGLES, PPS_MOREBITS, PPS_M_ORIGIN, PPS_M_ORIGIN2, PPS_M_TYPE, PPS_RDFLAGS, PPS_STATS, PPS_VIEWANGLE2,
    PPS_VIEWANGLES, PPS_VIEWOFFSET, PPS_WEAPONFRAME, PPS_WEAPONINDEX,
};
use crate::services::downloads::DownloadSource;

/// Quake II `chktbl` (`checksum.ts`).
const Q2_CHKTBL: [u8; 1024] = [
    0x84, 0x47, 0x51, 0xc1, 0x93, 0x22, 0x21, 0x24, 0x2f, 0x66, 0x60, 0x4d, 0xb0, 0x7c, 0xda, 0x88, 0x54, 0x15, 0x2b,
    0xc6, 0x6c, 0x89, 0xc5, 0x9d, 0x48, 0xee, 0xe6, 0x8a, 0xb5, 0xf4, 0xcb, 0xfb, 0xf1, 0x0c, 0x2e, 0xa0, 0xd7, 0xc9,
    0x1f, 0xd6, 0x06, 0x9a, 0x09, 0x41, 0x54, 0x67, 0x46, 0xc7, 0x74, 0xe3, 0xc8, 0xb6, 0x5d, 0xa6, 0x36, 0xc4, 0xab,
    0x2c, 0x7e, 0x85, 0xa8, 0xa4, 0xa6, 0x4d, 0x96, 0x19, 0x19, 0x9a, 0xcc, 0xd8, 0xac, 0x39, 0x5e, 0x3c, 0xf2, 0xf5,
    0x5a, 0x72, 0xe5, 0xa9, 0xd1, 0xb3, 0x23, 0x82, 0x6f, 0x29, 0xcb, 0xd1, 0xcc, 0x71, 0xfb, 0xea, 0x92, 0xeb, 0x1c,
    0xca, 0x4c, 0x70, 0xfe, 0x4d, 0xc9, 0x67, 0x43, 0x47, 0x94, 0xb9, 0x47, 0xbc, 0x3f, 0x01, 0xab, 0x7b, 0xa6, 0xe2,
    0x76, 0xef, 0x5a, 0x7a, 0x29, 0x0b, 0x51, 0x54, 0x67, 0xd8, 0x1c, 0x14, 0x3e, 0x29, 0xec, 0xe9, 0x2d, 0x48, 0x67,
    0xff, 0xed, 0x54, 0x4f, 0x48, 0xc0, 0xaa, 0x61, 0xf7, 0x78, 0x12, 0x03, 0x7a, 0x9e, 0x8b, 0xcf, 0x83, 0x7b, 0xae,
    0xca, 0x7b, 0xd9, 0xe9, 0x53, 0x2a, 0xeb, 0xd2, 0xd8, 0xcd, 0xa3, 0x10, 0x25, 0x78, 0x5a, 0xb5, 0x23, 0x06, 0x93,
    0xb7, 0x84, 0xd2, 0xbd, 0x96, 0x75, 0xa5, 0x5e, 0xcf, 0x4e, 0xe9, 0x50, 0xa1, 0xe6, 0x9d, 0xb1, 0xe3, 0x85, 0x66,
    0x28, 0x4e, 0x43, 0xdc, 0x6e, 0xbb, 0x33, 0x9e, 0xf3, 0x0d, 0x00, 0xc1, 0xcf, 0x67, 0x34, 0x06, 0x7c, 0x71, 0xe3,
    0x63, 0xb7, 0xb7, 0xdf, 0x92, 0xc4, 0xc2, 0x25, 0x5c, 0xff, 0xc3, 0x6e, 0xfc, 0xaa, 0x1e, 0x2a, 0x48, 0x11, 0x1c,
    0x36, 0x68, 0x78, 0x86, 0x79, 0x30, 0xc3, 0xd6, 0xde, 0xbc, 0x3a, 0x2a, 0x6d, 0x1e, 0x46, 0xdd, 0xe0, 0x80, 0x1e,
    0x44, 0x3b, 0x6f, 0xaf, 0x31, 0xda, 0xa2, 0xbd, 0x77, 0x06, 0x56, 0xc0, 0xb7, 0x92, 0x4b, 0x37, 0xc0, 0xfc, 0xc2,
    0xd5, 0xfb, 0xa8, 0xda, 0xf5, 0x57, 0xa8, 0x18, 0xc0, 0xdf, 0xe7, 0xaa, 0x2a, 0xe0, 0x7c, 0x6f, 0x77, 0xb1, 0x26,
    0xba, 0xf9, 0x2e, 0x1d, 0x16, 0xcb, 0xb8, 0xa2, 0x44, 0xd5, 0x2f, 0x1a, 0x79, 0x74, 0x87, 0x4b, 0x00, 0xc9, 0x4a,
    0x3a, 0x65, 0x8f, 0xe6, 0x5d, 0xe5, 0x0a, 0x77, 0xd8, 0x1a, 0x14, 0x41, 0x75, 0xb1, 0xe2, 0x50, 0x2c, 0x93, 0x38,
    0x2b, 0x6d, 0xf3, 0xf6, 0xdb, 0x1f, 0xcd, 0xff, 0x14, 0x70, 0xe7, 0x16, 0xe8, 0x3d, 0xf0, 0xe3, 0xbc, 0x5e, 0xb6,
    0x3f, 0xcc, 0x81, 0x24, 0x67, 0xf3, 0x97, 0x3b, 0xfe, 0x3a, 0x96, 0x85, 0xdf, 0xe4, 0x6e, 0x3c, 0x85, 0x05, 0x0e,
    0xa3, 0x2b, 0x07, 0xc8, 0xbf, 0xe5, 0x13, 0x82, 0x62, 0x08, 0x61, 0x69, 0x4b, 0x47, 0x62, 0x73, 0x44, 0x64, 0x8e,
    0xe2, 0x91, 0xa6, 0x9a, 0xb7, 0xe9, 0x04, 0xb6, 0x54, 0x0c, 0xc5, 0xa9, 0x47, 0xa6, 0xc9, 0x08, 0xfe, 0x4e, 0xa6,
    0xcc, 0x8a, 0x5b, 0x90, 0x6f, 0x2b, 0x3f, 0xb6, 0x0a, 0x96, 0xc0, 0x78, 0x58, 0x3c, 0x76, 0x6d, 0x94, 0x1a, 0xe4,
    0x4e, 0xb8, 0x38, 0xbb, 0xf5, 0xeb, 0x29, 0xd8, 0xb0, 0xf3, 0x15, 0x1e, 0x99, 0x96, 0x3c, 0x5d, 0x63, 0xd5, 0xb1,
    0xad, 0x52, 0xb8, 0x55, 0x70, 0x75, 0x3e, 0x1a, 0xd5, 0xda, 0xf6, 0x7a, 0x48, 0x7d, 0x44, 0x41, 0xf9, 0x11, 0xce,
    0xd7, 0xca, 0xa5, 0x3d, 0x7a, 0x79, 0x7e, 0x7d, 0x25, 0x1b, 0x77, 0xbc, 0xf7, 0xc7, 0x0f, 0x84, 0x95, 0x10, 0x92,
    0x67, 0x15, 0x11, 0x5a, 0x5e, 0x41, 0x66, 0x0f, 0x38, 0x03, 0xb2, 0xf1, 0x5d, 0xf8, 0xab, 0xc0, 0x02, 0x76, 0x84,
    0x28, 0xf4, 0x9d, 0x56, 0x46, 0x60, 0x20, 0xdb, 0x68, 0xa7, 0xbb, 0xee, 0xac, 0x15, 0x01, 0x2f, 0x20, 0x09, 0xdb,
    0xc0, 0x16, 0xa1, 0x89, 0xf9, 0x94, 0x59, 0x00, 0xc1, 0x76, 0xbf, 0xc1, 0x4d, 0x5d, 0x2d, 0xa9, 0x85, 0x2c, 0xd6,
    0xd3, 0x14, 0xcc, 0x02, 0xc3, 0xc2, 0xfa, 0x6b, 0xb7, 0xa6, 0xef, 0xdd, 0x12, 0x26, 0xa4, 0x63, 0xe3, 0x62, 0xbd,
    0x56, 0x8a, 0x52, 0x2b, 0xb9, 0xdf, 0x09, 0xbc, 0x0e, 0x97, 0xa9, 0xb0, 0x82, 0x46, 0x08, 0xd5, 0x1a, 0x8e, 0x1b,
    0xa7, 0x90, 0x98, 0xb9, 0xbb, 0x3c, 0x17, 0x9a, 0xf2, 0x82, 0xba, 0x64, 0x0a, 0x7f, 0xca, 0x5a, 0x8c, 0x7c, 0xd3,
    0x79, 0x09, 0x5b, 0x26, 0xbb, 0xbd, 0x25, 0xdf, 0x3d, 0x6f, 0x9a, 0x8f, 0xee, 0x21, 0x66, 0xb0, 0x8d, 0x84, 0x4c,
    0x91, 0x45, 0xd4, 0x77, 0x4f, 0xb3, 0x8c, 0xbc, 0xa8, 0x99, 0xaa, 0x19, 0x53, 0x7c, 0x02, 0x87, 0xbb, 0x0b, 0x7c,
    0x1a, 0x2d, 0xdf, 0x48, 0x44, 0x06, 0xd6, 0x7d, 0x0c, 0x2d, 0x35, 0x76, 0xae, 0xc4, 0x5f, 0x71, 0x85, 0x97, 0xc4,
    0x3d, 0xef, 0x52, 0xbe, 0x00, 0xe4, 0xcd, 0x49, 0xd1, 0xd1, 0x1c, 0x3c, 0xd0, 0x1c, 0x42, 0xaf, 0xd4, 0xbd, 0x58,
    0x34, 0x07, 0x32, 0xee, 0xb9, 0xb5, 0xea, 0xff, 0xd7, 0x8c, 0x0d, 0x2e, 0x2f, 0xaf, 0x87, 0xbb, 0xe6, 0x52, 0x71,
    0x22, 0xf5, 0x25, 0x17, 0xa1, 0x82, 0x04, 0xc2, 0x4a, 0xbd, 0x57, 0xc6, 0xab, 0xc8, 0x35, 0x0c, 0x3c, 0xd9, 0xc2,
    0x43, 0xdb, 0x27, 0x92, 0xcf, 0xb8, 0x25, 0x60, 0xfa, 0x21, 0x3b, 0x04, 0x52, 0xc8, 0x96, 0xba, 0x74, 0xe3, 0x67,
    0x3e, 0x8e, 0x8d, 0x61, 0x90, 0x92, 0x59, 0xb6, 0x1a, 0x1c, 0x5e, 0x21, 0xc1, 0x65, 0xe5, 0xa6, 0x34, 0x05, 0x6f,
    0xc5, 0x60, 0xb1, 0x83, 0xc1, 0xd5, 0xd5, 0xed, 0xd9, 0xc7, 0x11, 0x7b, 0x49, 0x7a, 0xf9, 0xf9, 0x84, 0x47, 0x9b,
    0xe2, 0xa5, 0x82, 0xe0, 0xc2, 0x88, 0xd0, 0xb2, 0x58, 0x88, 0x7f, 0x45, 0x09, 0x67, 0x74, 0x61, 0xbf, 0xe6, 0x40,
    0xe2, 0x9d, 0xc2, 0x47, 0x05, 0x89, 0xed, 0xcb, 0xbb, 0xb7, 0x27, 0xe7, 0xdc, 0x7a, 0xfd, 0xbf, 0xa8, 0xd0, 0xaa,
    0x10, 0x39, 0x3c, 0x20, 0xf0, 0xd3, 0x6e, 0xb1, 0x72, 0xf8, 0xe6, 0x0f, 0xef, 0x37, 0xe5, 0x09, 0x33, 0x5a, 0x83,
    0x43, 0x80, 0x4f, 0x65, 0x2f, 0x7c, 0x8c, 0x6a, 0xa0, 0x82, 0x0c, 0xd4, 0xd4, 0xfa, 0x81, 0x60, 0x3d, 0xdf, 0x06,
    0xf1, 0x5f, 0x08, 0x0d, 0x6d, 0x43, 0xf2, 0xe3, 0x11, 0x7d, 0x80, 0x32, 0xc5, 0xfb, 0xc5, 0xd9, 0x27, 0xec, 0xc6,
    0x4e, 0x65, 0x27, 0x76, 0x87, 0xa6, 0xee, 0xee, 0xd7, 0x8b, 0xd1, 0xa0, 0x5c, 0xb0, 0x42, 0x13, 0x0e, 0x95, 0x4a,
    0xf2, 0x06, 0xc6, 0x43, 0x33, 0xf4, 0xc7, 0xf8, 0xe7, 0x1f, 0xdd, 0xe4, 0x46, 0x4a, 0x70, 0x39, 0x6c, 0xd0, 0xed,
    0xca, 0xbe, 0x60, 0x3b, 0xd1, 0x7b, 0x57, 0x48, 0xe5, 0x3a, 0x79, 0xc1, 0x69, 0x33, 0x53, 0x1b, 0x80, 0xb8, 0x91,
    0x7d, 0xb4, 0xf6, 0x17, 0x1a, 0x1d, 0x5a, 0x32, 0xd6, 0xcc, 0x71, 0x29, 0x3f, 0x28, 0xbb, 0xf3, 0x5e, 0x71, 0xb8,
    0x43, 0xaf, 0xf8, 0xb9, 0x64, 0xef, 0xc4, 0xa5, 0x6c, 0x08, 0x53, 0xc7, 0x00, 0x10, 0x39, 0x4f, 0xdd, 0xe4, 0xb6,
    0x19, 0x27, 0xfb, 0xb8, 0xf5, 0x32, 0x73, 0xe5, 0xcb, 0x32, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

/// Checksum a client-move block against its sequence (`blockSequenceChecksum`).
pub fn block_sequence_checksum(bytes: &[u8], sequence: u32) -> Result<u8, Q2NetError> {
    let length = bytes.len().min(60);
    let mut block = Vec::with_capacity(length + 4);
    block.extend_from_slice(&bytes[..length]);
    let offset = (sequence % 1020) as usize;
    for i in 0..4 {
        block.push(Q2_CHKTBL[offset + i]);
    }
    let mut crc: u32 = 0xffff;
    let mut sum: u32 = 0;
    for byte in &block {
        sum += u32::from(*byte);
        crc ^= u32::from(*byte) << 8;
        for _ in 0..8 {
            crc = ((crc << 1) ^ (if (crc & 0x8000) != 0 { 0x1021 } else { 0 })) & 0xffff;
        }
    }
    Ok(((crc ^ sum) & 255) as u8)
}

/// Write a fixed-point position (`MSG_WritePos`).
pub fn write_q2_pos(writer: &mut MsgWriter, pos: [f64; 3]) -> Result<(), MsgError> {
    for component in pos {
        writer.write_short((component * 8.0).trunc() as i16)?;
    }
    Ok(())
}

/// Truncate a float to a wire byte, matching `MSG_WriteByte(Math.trunc(v))`.
fn trunc_byte(value: f64) -> u8 {
    value.trunc() as i32 as u8
}

/// Whether an identity is a KEX wire.
fn is_kex(protocol: ProtocolIdentity) -> bool {
    matches!(protocol, ProtocolIdentity::Q2Kex | ProtocolIdentity::Q2KexDemo)
}

/// Whether an identity is a rerelease wire.
fn is_rerelease(protocol: ProtocolIdentity) -> bool {
    matches!(
        protocol,
        ProtocolIdentity::Q2Rerelease | ProtocolIdentity::Q2PrivateClassic
    )
}

/// Write rerelease fog data (`writeQ2Fog`).
pub fn write_q2_fog(writer: &mut MsgWriter, fog: &FogData) -> Result<(), MsgError> {
    let bits = fog.bits
        | (if (fog.bits & 0xff00) != 0 {
            fog_bits::MORE_BITS
        } else {
            0
        });
    writer.write_byte(bits as u8)?;
    if (bits & fog_bits::MORE_BITS) != 0 {
        writer.write_byte((bits >> 8) as u8)?;
    }
    if (bits & fog_bits::DENSITY) != 0 {
        writer.write_float(fog.density)?;
        writer.write_byte(fog.skyfactor)?;
    }
    if (bits & fog_bits::R) != 0 {
        writer.write_byte(fog.red)?;
    }
    if (bits & fog_bits::G) != 0 {
        writer.write_byte(fog.green)?;
    }
    if (bits & fog_bits::B) != 0 {
        writer.write_byte(fog.blue)?;
    }
    if (bits & fog_bits::TIME) != 0 {
        writer.write_short(fog.time as i16)?;
    }
    if (bits & fog_bits::HEIGHTFOG_FALLOFF) != 0 {
        writer.write_float(fog.hf_falloff)?;
    }
    if (bits & fog_bits::HEIGHTFOG_DENSITY) != 0 {
        writer.write_float(fog.hf_density)?;
    }
    if (bits & fog_bits::HEIGHTFOG_START_R) != 0 {
        writer.write_byte(fog.hf_start[0])?;
    }
    if (bits & fog_bits::HEIGHTFOG_START_G) != 0 {
        writer.write_byte(fog.hf_start[1])?;
    }
    if (bits & fog_bits::HEIGHTFOG_START_B) != 0 {
        writer.write_byte(fog.hf_start[2])?;
    }
    if (bits & fog_bits::HEIGHTFOG_START_DIST) != 0 {
        writer.write_long(fog.hf_start_dist)?;
    }
    if (bits & fog_bits::HEIGHTFOG_END_R) != 0 {
        writer.write_byte(fog.hf_end[0])?;
    }
    if (bits & fog_bits::HEIGHTFOG_END_G) != 0 {
        writer.write_byte(fog.hf_end[1])?;
    }
    if (bits & fog_bits::HEIGHTFOG_END_B) != 0 {
        writer.write_byte(fog.hf_end[2])?;
    }
    if (bits & fog_bits::HEIGHTFOG_END_DIST) != 0 {
        writer.write_long(fog.hf_end_dist)?;
    }
    Ok(())
}

/// Encode one server event for the wire's protocol (`encodeQ2ServerEvent`).
///
/// Frames and mod-private messages have no encoder, matching the donor's
/// `Exclude<Q2ServerEvent, { kind: 'frame' | 'private' }>` input.
pub fn encode_q2_server_event(wire: &mut Q2Wire, event: &Q2ServerEvent) -> Result<Vec<u8>, Q2NetError> {
    let protocol = wire.protocol();
    let kex = is_kex(protocol);
    let rerelease = is_rerelease(protocol);
    match event {
        Q2ServerEvent::Achievement { .. }
        | Q2ServerEvent::Fog { .. }
        | Q2ServerEvent::Damage { .. }
        | Q2ServerEvent::Poi { .. }
        | Q2ServerEvent::HelpPath { .. }
        | Q2ServerEvent::Locprint { .. }
            if !kex && !rerelease =>
        {
            return Err(Q2NetError::Protocol(
                "Q2 rerelease service message cannot fit selected protocol",
            ));
        }
        Q2ServerEvent::Setting { .. }
            if !matches!(
                protocol,
                ProtocolIdentity::Q2R1q2 { .. }
                    | ProtocolIdentity::Q2Q2pro { .. }
                    | ProtocolIdentity::Q2Rerelease
                    | ProtocolIdentity::Q2PrivateClassic
            ) =>
        {
            return Err(Q2NetError::Protocol(
                "Selected Q2 protocol has no server setting message",
            ));
        }
        Q2ServerEvent::Frame { .. } | Q2ServerEvent::Private { .. } => {
            return Err(Q2NetError::Protocol("Q2 server event has no wire encoder"));
        }
        _ => {}
    }
    let mut writer = MsgWriter::new(65536, false);
    match event {
        Q2ServerEvent::Nop => writer.write_byte(6)?,
        Q2ServerEvent::Disconnect => writer.write_byte(7)?,
        Q2ServerEvent::Reconnect => writer.write_byte(8)?,
        Q2ServerEvent::LevelRestart => {
            if !kex {
                return Err(Q2NetError::Protocol("Level-restart opcode requires KEX wire"));
            }
            writer.write_byte(24)?;
        }
        Q2ServerEvent::ServerData { data } => wire.write_server_data(&mut writer, data)?,
        Q2ServerEvent::Print { level, text } => {
            writer.write_byte(10)?;
            writer.write_byte(*level)?;
            writer.write_string(text)?;
        }
        Q2ServerEvent::CenterPrint { text } => {
            writer.write_byte(15)?;
            writer.write_string(text)?;
        }
        Q2ServerEvent::CommandText { text } => {
            writer.write_byte(11)?;
            writer.write_string(text)?;
        }
        Q2ServerEvent::Layout { text } => {
            writer.write_byte(4)?;
            writer.write_string(text)?;
        }
        Q2ServerEvent::Achievement { text } => {
            writer.write_byte(33)?;
            writer.write_string(text)?;
        }
        Q2ServerEvent::ConfigString { index, value } => {
            writer.write_byte(13)?;
            writer.write_short(*index as i16)?;
            writer.write_string(value)?;
        }
        Q2ServerEvent::Baseline { entity } => {
            wire.write_spawn_baseline(&mut writer, entity)?;
        }
        Q2ServerEvent::TempEntity { value: entity } => {
            writer.write_byte(3)?;
            writer.write_byte(entity.temp_type)?;
            for field in &entity.fields {
                match field {
                    Q2TempField::Integer { name, value } => {
                        if *name == Q2TempInt::Time {
                            writer.write_long(*value)?;
                        } else if *name == Q2TempInt::Entity1
                            || *name == Q2TempInt::Entity2
                            || entity.temp_type == Q2TempType::Q2proDamageDealt as u8
                        {
                            writer.write_short(*value as i16)?;
                        } else {
                            writer.write_byte(*value as u8)?;
                        }
                    }
                    Q2TempField::Vector { name, value } => {
                        if *name == Q2TempVec::Direction {
                            write_dir(&mut writer, Some(*value))?;
                        } else if wire.q2pro_extended_v2() {
                            for component in value {
                                write_q2pro_int23(&mut writer, (component * 8.0).trunc() as i32, 0)?;
                            }
                        } else if wire.floating_coordinates() {
                            for component in value {
                                writer.write_float(*component as f32)?;
                            }
                        } else {
                            write_q2_pos(&mut writer, *value)?;
                        }
                    }
                }
            }
        }
        Q2ServerEvent::Inventory { counts } => {
            writer.write_byte(5)?;
            for count in counts {
                writer.write_short(*count)?;
            }
        }
        Q2ServerEvent::Download { percent, bytes } => {
            writer.write_byte(16)?;
            writer.write_short(bytes.as_ref().map_or(-1, |bytes| bytes.len() as i16))?;
            writer.write_byte(*percent)?;
            if let Some(bytes) = bytes {
                writer.write_bytes(bytes)?;
            }
        }
        Q2ServerEvent::Setting { index, value } => {
            writer.write_byte(if rerelease { 37 } else { 24 })?;
            writer.write_long(*index)?;
            writer.write_long(*value)?;
        }
        Q2ServerEvent::Seat { seat } => {
            if !kex {
                return Err(Q2NetError::Protocol("Source seat marker requires KEX wire"));
            }
            writer.write_byte(21)?;
            writer.write_byte(*seat)?;
        }
        Q2ServerEvent::MuzzleFlash {
            entity,
            flash,
            monster,
            silenced,
        } => {
            let mut entity = *entity;
            let mut flash = *flash;
            if *monster && flash > 255 {
                if kex || rerelease {
                    writer.write_byte(32)?;
                    writer.write_short(entity as i16)?;
                    writer.write_short(flash as i16)?;
                    return Ok(writer.bytes().to_vec());
                }
                if wire.q2pro_extended() {
                    entity |= (flash & 0x700) << 5;
                    flash &= 255;
                } else {
                    return Err(Q2NetError::Protocol(
                        "Monster muzzleflash cannot fit selected Q2 protocol",
                    ));
                }
            }
            writer.write_byte(if *monster { 2 } else { 1 })?;
            writer.write_short(entity as i16)?;
            writer.write_byte((flash | (if *silenced { 128 } else { 0 })) as u8)?;
        }
        Q2ServerEvent::Sound { sound } => {
            let mut flags = sound.flags;
            if !kex && sound.index > 255 {
                if !rerelease && !wire.q2pro_extended() {
                    return Err(Q2NetError::Protocol("Q2 sound index needs extended game layout"));
                }
                flags |= 32;
            }
            if sound.position.is_some() {
                flags |= 4;
            }
            if sound.entity != 0 || sound.channel != 0 {
                flags |= 8;
            }
            if sound.volume != 1.0 {
                flags |= 1;
            }
            if sound.attenuation != 1.0 {
                flags |= 2;
            }
            if sound.delay_seconds != 0.0 {
                flags |= 16;
            }
            let channel = (sound.entity << 3) | u32::from(sound.channel);
            if kex && channel > 65535 {
                flags |= 64;
            }
            writer.write_byte(9)?;
            writer.write_byte(flags)?;
            if kex || (flags & 32) != 0 {
                writer.write_short(sound.index as i16)?;
            } else {
                writer.write_byte(sound.index as u8)?;
            }
            if (flags & 1) != 0 {
                writer.write_byte(trunc_byte(sound.volume * 255.0))?;
            }
            if (flags & 2) != 0 {
                writer.write_byte(trunc_byte(sound.attenuation * 64.0))?;
            }
            if (flags & 16) != 0 {
                writer.write_byte(trunc_byte(sound.delay_seconds * 1000.0))?;
            }
            if (flags & 8) != 0 {
                if kex && (flags & 64) != 0 {
                    writer.write_long(channel as i32)?;
                } else {
                    writer.write_short(channel as i16)?;
                }
            }
            if (flags & 4) != 0 {
                let Some(position) = sound.position else {
                    return Err(Q2NetError::Protocol("Positioned Q2 sound has no position"));
                };
                if wire.q2pro_extended_v2() {
                    for component in position {
                        write_q2pro_int23(&mut writer, (component * 8.0).trunc() as i32, 0)?;
                    }
                } else if wire.floating_coordinates() {
                    for component in position {
                        writer.write_float(component as f32)?;
                    }
                } else {
                    write_q2_pos(&mut writer, position)?;
                }
            }
        }
        Q2ServerEvent::Fog { value } => {
            writer.write_byte(27)?;
            write_q2_fog(&mut writer, value)?;
        }
        Q2ServerEvent::Damage { indicators } => {
            writer.write_byte(25)?;
            writer.write_byte(indicators.len() as u8)?;
            for damage in indicators {
                writer.write_byte(
                    (damage.damage & 31)
                        | (if damage.health { 32 } else { 0 })
                        | (if damage.armor { 64 } else { 0 })
                        | (if damage.shield { 128 } else { 0 }),
                )?;
                write_dir(&mut writer, Some(damage.direction))?;
            }
        }
        Q2ServerEvent::Poi { value } => {
            writer.write_byte(30)?;
            writer.write_short(value.key as i16)?;
            writer.write_short(value.time as i16)?;
            for component in value.pos {
                writer.write_float(component)?;
            }
            writer.write_short(value.image as i16)?;
            writer.write_byte(value.color)?;
            writer.write_byte(value.flags)?;
        }
        Q2ServerEvent::HelpPath { value } => {
            writer.write_byte(31)?;
            writer.write_byte(if value.start { 1 } else { 0 })?;
            for component in value.pos {
                writer.write_float(component)?;
            }
            write_dir(&mut writer, Some(value.dir))?;
        }
        Q2ServerEvent::Locprint { value } => {
            writer.write_byte(26)?;
            writer.write_byte(value.flags)?;
            writer.write_string(&value.base)?;
            writer.write_byte(value.args.len() as u8)?;
            for arg in &value.args {
                writer.write_string(arg)?;
            }
        }
        Q2ServerEvent::Frame { .. } | Q2ServerEvent::Private { .. } => {
            return Err(Q2NetError::Protocol("Q2 server event has no wire encoder"));
        }
    }
    Ok(writer.bytes().to_vec())
}

/// Source-owned download advanced by `nextdl` requests (`Q2DownloadSender`).
pub struct Q2DownloadSender {
    source: Box<dyn DownloadSource>,
    offset: u64,
    block_bytes: usize,
    ended: bool,
}

impl Q2DownloadSender {
    /// Build a sender over a source at an offset.
    pub fn new(source: Box<dyn DownloadSource>, offset: u64, block_bytes: usize) -> Result<Self, Q2NetError> {
        if offset > source.byte_length() || block_bytes < 1 || block_bytes > 32767 {
            return Err(Q2NetError::Range("Invalid Q2 download range"));
        }
        Ok(Self {
            source,
            offset,
            block_bytes,
            ended: false,
        })
    }

    /// Next download chunk, or `None` once the terminal marker was emitted.
    pub fn next(&mut self) -> Result<Option<Q2ServerEvent>, Q2NetError> {
        if self.ended {
            return Ok(None);
        }
        let total = self.source.byte_length();
        let bytes = self
            .source
            .read(self.offset, (total - self.offset).min(self.block_bytes as u64) as usize)?;
        self.offset += bytes.len() as u64;
        let percent = (self.offset * 100 / total.max(1)) as u8;
        if self.offset == total {
            self.ended = true;
            self.source.close();
        }
        Ok(Some(Q2ServerEvent::Download {
            percent,
            bytes: Some(bytes),
        }))
    }

    /// Close the source without emitting further chunks.
    pub fn close(&mut self) {
        if !self.ended {
            self.ended = true;
            self.source.close();
        }
    }
}

/// Client event (`Q2ClientEvent`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Q2ClientEvent {
    /// No operation.
    Nop,
    /// Userinfo string.
    Userinfo(String),
    /// Console command.
    Command(String),
    /// Userinfo key/value delta.
    UserinfoDelta {
        /// Key.
        name: String,
        /// Value.
        value: String,
    },
    /// Client setting.
    Setting {
        /// Index.
        index: i16,
        /// Value.
        value: i16,
    },
    /// Move command triple.
    Move {
        /// Last acknowledged server frame.
        last_frame: i32,
        /// Oldest, old, and current commands.
        commands: [Usercmd; 3],
    },
    /// Batched moves.
    BatchMove {
        /// Batch.
        batch: BatchMove,
    },
}

/// Decoded client record (`Q2ClientRecord`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2ClientRecord {
    /// Event.
    pub event: Q2ClientEvent,
    /// Raw bytes.
    pub raw: Vec<u8>,
    /// Split-screen seat.
    pub seat: u8,
}

/// Read a KEX control string with fatal UTF-8 decoding.
fn read_kex_control_string(wire: &mut Q2Wire) -> Result<String, Q2NetError> {
    let mut bytes = Vec::new();
    loop {
        let byte = wire.read_raw_byte()?;
        if byte == 0 {
            break;
        }
        bytes.push(byte);
    }
    String::from_utf8(bytes).map_err(|_| Q2NetError::Protocol("Truncated KEX control string"))
}

/// Read client messages from one packet (`readQ2ClientMessages`).
pub fn read_q2_client_messages(
    wire: &mut Q2Wire,
    bytes: &[u8],
    sequence: u32,
    seats: u8,
) -> Result<Vec<Q2ClientRecord>, Q2NetError> {
    wire.begin(bytes);
    let mut records = Vec::new();
    let mut moved = false;
    let kex = is_kex(wire.protocol());
    let q2pro = matches!(wire.protocol(), ProtocolIdentity::Q2Q2pro { .. });
    while wire.remaining() > 0 {
        let start = wire.position();
        let raw = wire.read_raw_byte()?;
        let op = if q2pro { raw & 31 } else { raw };
        let mut selected_seat = 0u8;
        let event = match op {
            1 => Q2ClientEvent::Nop,
            3 => Q2ClientEvent::Userinfo(if kex {
                read_kex_control_string(wire)?
            } else {
                wire.read_raw_string(2047)?
            }),
            4 => {
                if kex {
                    let seat = wire.read_raw_byte()?;
                    if seat < 1 || seat > seats {
                        return Err(Q2NetError::Protocol(
                            "Q2 connection does not own the selected split player",
                        ));
                    }
                    selected_seat = seat - 1;
                }
                Q2ClientEvent::Command(if kex {
                    read_kex_control_string(wire)?
                } else {
                    wire.read_raw_string(2047)?
                })
            }
            5 => {
                if !matches!(
                    wire.protocol(),
                    ProtocolIdentity::Q2R1q2 { .. }
                        | ProtocolIdentity::Q2Q2pro { .. }
                        | ProtocolIdentity::Q2Rerelease
                        | ProtocolIdentity::Q2PrivateClassic
                ) {
                    return Err(Q2NetError::Protocol(
                        "Client setting is not supported by selected Q2 wire",
                    ));
                }
                Q2ClientEvent::Setting {
                    index: wire.read_raw_short()?,
                    value: wire.read_raw_short()?,
                }
            }
            12 => {
                if !matches!(
                    wire.protocol(),
                    ProtocolIdentity::Q2Q2pro { .. }
                        | ProtocolIdentity::Q2Rerelease
                        | ProtocolIdentity::Q2PrivateClassic
                ) {
                    return Err(Q2NetError::Protocol(
                        "Userinfo delta is not supported by selected Q2 wire",
                    ));
                }
                Q2ClientEvent::UserinfoDelta {
                    name: wire.read_raw_string(2047)?,
                    value: wire.read_raw_string(2047)?,
                }
            }
            2 => {
                if moved {
                    return Err(Q2NetError::Protocol("Multiple Q2 move commands in one packet"));
                }
                moved = true;
                let checksum = if wire.protocol() == ProtocolIdentity::Q2Classic {
                    Some(wire.read_raw_byte()?)
                } else {
                    None
                };
                let checksum_start = wire.position();
                let last_frame = wire.read_raw_long()?;
                let seat_count = if kex { seats } else { 1 };
                for seat in 0..seat_count {
                    let lightlevel = if kex { Some(wire.read_raw_byte()?) } else { None };
                    let base = Usercmd::default();
                    let oldest = wire.read_delta_usercmd(&base)?;
                    let old = wire.read_delta_usercmd(&oldest)?;
                    let mut current = wire.read_delta_usercmd(&old)?;
                    let mut oldest = oldest;
                    let mut old = old;
                    if let Some(lightlevel) = lightlevel {
                        oldest.lightlevel = lightlevel;
                        old.lightlevel = lightlevel;
                        current.lightlevel = lightlevel;
                    }
                    if let Some(expected) = checksum {
                        let block = wire.raw_slice(checksum_start).to_vec();
                        if block_sequence_checksum(&block, sequence)? != expected {
                            return Err(Q2NetError::Protocol("Q2 command sequence checksum mismatch"));
                        }
                    }
                    records.push(Q2ClientRecord {
                        event: Q2ClientEvent::Move {
                            last_frame,
                            commands: [oldest, old, current],
                        },
                        raw: wire.raw_slice(start).to_vec(),
                        seat,
                    });
                }
                continue;
            }
            10 | 11 => {
                if moved {
                    return Err(Q2NetError::Protocol("Multiple Q2 move commands in one packet"));
                }
                moved = true;
                let batch = wire.read_batch_move_wire(op == 10, raw >> 5)?;
                Q2ClientEvent::BatchMove { batch }
            }
            _ => return Err(Q2NetError::Protocol("Unknown Q2 client opcode")),
        };
        records.push(Q2ClientRecord {
            event,
            raw: wire.raw_slice(start).to_vec(),
            seat: selected_seat,
        });
    }
    wire.finish()?;
    Ok(records)
}

/// Encode a move triple (`encodeQ2Move`).
pub fn encode_q2_move(
    wire: &mut Q2Wire,
    sequence: u32,
    last_frame: i32,
    commands: &[Usercmd; 3],
) -> Result<Vec<u8>, Q2NetError> {
    let mut writer = MsgWriter::new(65536, false);
    writer.write_byte(2)?;
    let checksum = wire.protocol() == ProtocolIdentity::Q2Classic;
    if checksum {
        writer.write_byte(0)?;
    }
    let checksum_start = writer.cursize();
    writer.write_long(last_frame)?;
    let kex = is_kex(wire.protocol());
    if kex {
        writer.write_byte(commands[2].lightlevel)?;
    }
    let base = Usercmd::default();
    wire.write_delta_usercmd(&mut writer, &base, &commands[0])?;
    wire.write_delta_usercmd(&mut writer, &commands[0], &commands[1])?;
    wire.write_delta_usercmd(&mut writer, &commands[1], &commands[2])?;
    let mut bytes = writer.bytes().to_vec();
    if checksum {
        bytes[1] = block_sequence_checksum(&bytes[checksum_start..], sequence)?;
    }
    Ok(bytes)
}

/// Encode a batched move (`encodeQ2BatchMove`).
pub fn encode_q2_batch_move(
    wire: &mut Q2Wire,
    last_frame: Option<i32>,
    frames: &[BatchMoveFrame],
) -> Result<Vec<u8>, Q2NetError> {
    let extra = if matches!(wire.protocol(), ProtocolIdentity::Q2Q2pro { .. }) {
        ((frames.len().wrapping_sub(1)) << 5) as u8
    } else {
        0
    };
    let mut writer = MsgWriter::new(65536, false);
    writer.write_byte((if last_frame.is_none() { 10 } else { 11 }) | extra)?;
    wire.write_batch_move_wire(&mut writer, last_frame, frames)?;
    Ok(writer.bytes().to_vec())
}

/// Encode a non-move client event (`encodeQ2ClientControl`).
pub fn encode_q2_client_control(event: &Q2ClientEvent, kex: bool) -> Result<Vec<u8>, Q2NetError> {
    let mut writer = MsgWriter::new(65536, false);
    match event {
        Q2ClientEvent::Nop => writer.write_byte(1)?,
        Q2ClientEvent::Userinfo(text) => {
            writer.write_byte(3)?;
            if kex {
                writer.write_bytes(text.as_bytes())?;
                writer.write_byte(0)?;
            } else {
                writer.write_string(text)?;
            }
        }
        Q2ClientEvent::Command(text) => {
            writer.write_byte(4)?;
            if kex {
                writer.write_byte(1)?;
                writer.write_bytes(text.as_bytes())?;
                writer.write_byte(0)?;
            } else {
                writer.write_string(text)?;
            }
        }
        Q2ClientEvent::Setting { index, value } => {
            writer.write_byte(5)?;
            writer.write_short(*index)?;
            writer.write_short(*value)?;
        }
        Q2ClientEvent::UserinfoDelta { name, value } => {
            writer.write_byte(12)?;
            writer.write_string(name)?;
            writer.write_string(value)?;
        }
        Q2ClientEvent::Move { .. } | Q2ClientEvent::BatchMove { .. } => {
            return Err(Q2NetError::Protocol("Q2 move has its own encoder"));
        }
    }
    Ok(writer.bytes().to_vec())
}

/// Replays dropped commands before the new command (`Q2CommandReplay`).
#[derive(Debug, Default)]
pub struct Q2CommandReplay {
    previous: Usercmd,
    /// Last frame from the most recent move.
    pub last_frame: i32,
}

impl Q2CommandReplay {
    /// Fresh replay state.
    #[must_use]
    pub fn new() -> Self {
        Self {
            previous: Usercmd::default(),
            last_frame: -1,
        }
    }

    /// Execute a move or batch-move event with drop replay.
    pub fn execute(
        &mut self,
        event: &Q2ClientEvent,
        mut dropped: usize,
        mut think: impl FnMut(&Usercmd),
    ) -> Result<(), Q2NetError> {
        match event {
            Q2ClientEvent::Move { last_frame, commands } => {
                self.last_frame = *last_frame;
                if dropped < 20 {
                    while dropped > 2 {
                        think(&self.previous);
                        dropped -= 1;
                    }
                    if dropped > 1 {
                        think(&commands[0]);
                    }
                    if dropped > 0 {
                        think(&commands[1]);
                    }
                }
                think(&commands[2]);
                self.previous = commands[2].clone();
                Ok(())
            }
            Q2ClientEvent::BatchMove { batch } => {
                self.last_frame = batch.lastframe;
                let Some(last) = batch.frames.iter().flat_map(|frame| &frame.cmds).last() else {
                    return Ok(());
                };
                let last = last.clone();
                if dropped < 20 {
                    while dropped > batch.num_dups {
                        think(&self.previous);
                        dropped -= 1;
                    }
                    while dropped > 0 {
                        let Some(frame) = batch.frames.get(batch.num_dups - dropped) else {
                            return Err(Q2NetError::Protocol("Missing Q2 batch backup"));
                        };
                        for command in &frame.cmds {
                            think(command);
                        }
                        dropped -= 1;
                    }
                }
                let Some(newest) = batch.frames.get(batch.num_dups) else {
                    return Err(Q2NetError::Protocol("Missing Q2 newest command batch"));
                };
                for command in &newest.cmds {
                    think(command);
                }
                self.previous = last;
                Ok(())
            }
            _ => Err(Q2NetError::Protocol("Q2 replay needs a move event")),
        }
    }
}

/// Ten-frame rate window (`Q2RateWindow`).
#[derive(Debug, Default)]
pub struct Q2RateWindow {
    sizes: [u32; 10],
    suppressed: u32,
}

impl Q2RateWindow {
    /// Fresh window.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether to drop this frame for rate control.
    pub fn drop_frame(&mut self, server_frame: i32, bytes_per_second: u32, loopback: bool) -> bool {
        if loopback {
            return false;
        }
        let total: u32 = self.sizes.iter().sum();
        if total <= bytes_per_second {
            return false;
        }
        self.suppressed += 1;
        self.sizes[(server_frame.rem_euclid(10)) as usize] = 0;
        true
    }

    /// Record a sent frame's size.
    pub fn sent(&mut self, server_frame: i32, bytes: u32) {
        self.sizes[(server_frame.rem_euclid(10)) as usize] = bytes;
    }

    /// Take and clear the suppressed count.
    pub fn take_suppressed(&mut self) -> u32 {
        let value = self.suppressed;
        self.suppressed = 0;
        value
    }
}

/// MVD message recipient (`MvdRecipient`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MvdRecipient {
    /// Every viewer.
    All,
    /// One player slot.
    Player(u8),
    /// Potentially-visible set of a leaf.
    Pvs(u16),
    /// Potentially-hearable set of a leaf.
    Phs(u16),
}

/// One routed MVD payload (`MvdEmission`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MvdEmission {
    /// Recipient.
    pub recipient: MvdRecipient,
    /// Reliable delivery.
    pub reliable: bool,
    /// Payload bytes.
    pub bytes: Vec<u8>,
}

/// Authoritative snapshot captured for MVD (`MvdCapture`).
#[derive(Debug, Clone, PartialEq)]
pub struct MvdCapture {
    /// Stream revision.
    pub revision: u16,
    /// Stream flags.
    pub flags: u16,
    /// Server count.
    pub servercount: i32,
    /// Game directory.
    pub gamedir: String,
    /// Dummy client.
    pub dummy: i16,
    /// Configstrings by index.
    pub config_strings: BTreeMap<u16, String>,
    /// Portal bits.
    pub portal_bits: Vec<u8>,
    /// Players by slot.
    pub players: BTreeMap<u8, PlayerState>,
    /// Sorted entities.
    pub entities: Vec<EntityState>,
    /// Routed messages.
    pub messages: Vec<MvdEmission>,
}

/// Maximum MVD message bytes (`MVD_MAX_MESSAGE`).
pub const MVD_MAX_MESSAGE: usize = 0x8000;

/// Encode one routed emission (`encodeMvdEmission`).
pub fn encode_mvd_emission(event: &MvdEmission) -> Result<Vec<u8>, Q2NetError> {
    if event.bytes.is_empty() || event.bytes.len() >= 2048 {
        return Err(Q2NetError::Range("MVD routed payload must contain 1..2047 bytes"));
    }
    let mut writer = MsgWriter::new(event.bytes.len() + 5, false);
    let high = (event.bytes.len() >> 8) as u8;
    let opcode = match event.recipient {
        MvdRecipient::Player(_) => {
            if event.reliable {
                MvdOp::UnicastR
            } else {
                MvdOp::Unicast
            }
        }
        MvdRecipient::All => {
            if event.reliable {
                MvdOp::MulticastAllR
            } else {
                MvdOp::MulticastAll
            }
        }
        MvdRecipient::Phs(_) => {
            if event.reliable {
                MvdOp::MulticastPhsR
            } else {
                MvdOp::MulticastPhs
            }
        }
        MvdRecipient::Pvs(_) => {
            if event.reliable {
                MvdOp::MulticastPvsR
            } else {
                MvdOp::MulticastPvs
            }
        }
    };
    write_mvd_cmd(&mut writer, opcode as u8, high)?;
    writer.write_byte(event.bytes.len() as u8)?;
    match event.recipient {
        MvdRecipient::Player(number) => {
            if number == CLIENTNUM_NONE {
                return Err(Q2NetError::Range("Invalid MVD recipient"));
            }
            writer.write_byte(number)?;
        }
        MvdRecipient::All => {}
        MvdRecipient::Pvs(leaf) | MvdRecipient::Phs(leaf) => {
            if leaf >= 65535 {
                return Err(Q2NetError::Range("Invalid MVD leaf"));
            }
            writer.write_short(leaf as i16)?;
        }
    }
    writer.write_bytes(&event.bytes)?;
    Ok(writer.bytes().to_vec())
}

/// Whether two float slices differ.
fn differs(a: &[f64], b: &[f64]) -> bool {
    a.iter().zip(b.iter()).any(|(x, y)| x != y)
}

/// Extended MVD player delta (`playerDelta` for extended/v2/rerelease profiles).
#[allow(clippy::too_many_lines)]
fn write_extended_mvd_player_delta(
    writer: &mut MsgWriter,
    from: Option<&PlayerState>,
    to: Option<&PlayerState>,
    number: u8,
    profile: &MvdProfile,
) -> Result<(), Q2NetError> {
    if number == CLIENTNUM_NONE {
        return Err(Q2NetError::Range("Invalid MVD player number"));
    }
    let Some(to) = to else {
        writer.write_byte(number)?;
        writer.write_short(PPS_MOREBITS as i16)?;
        if profile.fog {
            writer.write_byte(1)?;
        }
        return Ok(());
    };
    let from_was_some = from.is_some();
    let previous;
    let from = match from {
        Some(from) => from,
        None => {
            previous = PlayerState::default();
            &previous
        }
    };
    let old_origin = if profile.rerelease {
        [
            f64::from(from.pmove.origin_f[0]),
            f64::from(from.pmove.origin_f[1]),
            f64::from(from.pmove.origin_f[2]),
        ]
    } else {
        [
            from.pmove.origin[0] as f64,
            from.pmove.origin[1] as f64,
            from.pmove.origin[2] as f64,
        ]
    };
    let origin = if profile.rerelease {
        [
            f64::from(to.pmove.origin_f[0]),
            f64::from(to.pmove.origin_f[1]),
            f64::from(to.pmove.origin_f[2]),
        ]
    } else {
        [
            to.pmove.origin[0] as f64,
            to.pmove.origin[1] as f64,
            to.pmove.origin[2] as f64,
        ]
    };
    let mut bits: u32 = 0;
    if to.pmove.pm_type != from.pmove.pm_type {
        bits |= u32::from(PPS_M_TYPE);
    }
    if origin[0] != old_origin[0] || origin[1] != old_origin[1] {
        bits |= u32::from(PPS_M_ORIGIN);
    }
    if origin[2] != old_origin[2] {
        bits |= u32::from(PPS_M_ORIGIN2);
    }
    if differs(&to.viewoffset, &from.viewoffset) {
        bits |= u32::from(PPS_VIEWOFFSET);
    }
    if angle_to_short(to.viewangles[0]) != angle_to_short(from.viewangles[0])
        || angle_to_short(to.viewangles[1]) != angle_to_short(from.viewangles[1])
    {
        bits |= u32::from(PPS_VIEWANGLES);
    }
    if angle_to_short(to.viewangles[2]) != angle_to_short(from.viewangles[2]) {
        bits |= u32::from(PPS_VIEWANGLE2);
    }
    if differs(&to.kick_angles, &from.kick_angles) {
        bits |= u32::from(PPS_KICKANGLES);
    }
    if to.gunindex != from.gunindex || to.gunskin != from.gunskin {
        bits |= u32::from(PPS_WEAPONINDEX);
    }
    if to.gunframe != from.gunframe {
        bits |= u32::from(PPS_WEAPONFRAME);
    }
    if differs(&to.gunoffset, &from.gunoffset) {
        bits |= u32::from(PPS_GUNOFFSET);
    }
    if differs(&to.gunangles, &from.gunangles) {
        bits |= u32::from(PPS_GUNANGLES);
    }
    if differs(&to.blend, &from.blend)
        || ((profile.v2 || profile.rerelease) && differs(&to.damage_blend, &from.damage_blend))
    {
        bits |= u32::from(PPS_BLEND);
    }
    let fog = if profile.fog {
        q2pro_fog_bits(&from.fog, &to.fog)
    } else {
        0
    };
    if fog != 0 {
        bits |= u32::from(PPS_MOREBITS) | (1 << 17);
    }
    if to.fov != from.fov {
        bits |= u32::from(PPS_FOV);
    }
    if to.rdflags != from.rdflags {
        bits |= u32::from(PPS_RDFLAGS);
    }
    let mut stats: u64 = 0;
    for i in 0..(if profile.v2 || profile.rerelease { 64 } else { 32 }) {
        if to.stats[i] != from.stats[i] {
            stats |= 1 << i;
        }
    }
    if stats != 0 {
        bits |= u32::from(PPS_STATS);
    }
    if bits == 0 && from_was_some {
        return Ok(());
    }
    writer.write_byte(number)?;
    writer.write_short(bits as i16)?;
    if (bits & u32::from(PPS_MOREBITS)) != 0 {
        writer.write_byte((bits >> 16) as u8)?;
    }
    if (bits & u32::from(PPS_M_TYPE)) != 0 {
        writer.write_byte(to.pmove.pm_type)?;
    }
    for axis in 0..3 {
        if (bits & u32::from(if axis == 2 { PPS_M_ORIGIN2 } else { PPS_M_ORIGIN })) != 0 {
            let value = origin[axis];
            if profile.rerelease {
                writer.write_float(value as f32)?;
            } else if profile.v2 {
                write_q2pro_int23(&mut *writer, value as i32, old_origin[axis] as i32)?;
            } else {
                writer.write_short(value as i16)?;
            }
        }
    }
    if (bits & u32::from(PPS_VIEWOFFSET)) != 0 {
        let scale = if profile.rerelease { 16.0 } else { 4.0 };
        for component in to.viewoffset {
            let packed = (component * scale).trunc() as i32;
            if profile.rerelease {
                writer.write_short(packed as i16)?;
            } else {
                writer.write_char(packed as i8)?;
            }
        }
    }
    if (bits & u32::from(PPS_VIEWANGLES)) != 0 {
        writer.write_short(angle_to_short(to.viewangles[0]) as i16)?;
        writer.write_short(angle_to_short(to.viewangles[1]) as i16)?;
    }
    if (bits & u32::from(PPS_VIEWANGLE2)) != 0 {
        writer.write_short(angle_to_short(to.viewangles[2]) as i16)?;
    }
    if (bits & u32::from(PPS_KICKANGLES)) != 0 {
        let scale = if profile.rerelease { 1024.0 } else { 4.0 };
        for component in to.kick_angles {
            let packed = (component * scale).trunc() as i32;
            if profile.rerelease {
                writer.write_short(packed as i16)?;
            } else {
                writer.write_char(packed as i8)?;
            }
        }
    }
    if (bits & u32::from(PPS_WEAPONINDEX)) != 0 {
        writer.write_short((to.gunindex | (to.gunskin << 13)) as i16)?;
    }
    if (bits & u32::from(PPS_WEAPONFRAME)) != 0 {
        if profile.rerelease {
            writer.write_short(to.gunframe as i16)?;
        } else {
            writer.write_byte(to.gunframe as u8)?;
        }
    }
    if (bits & u32::from(PPS_GUNOFFSET)) != 0 {
        let scale = if profile.rerelease { 512.0 } else { 8.0 };
        for component in to.gunoffset {
            writer.write_short((component * scale).trunc() as i16)?;
        }
    }
    if (bits & u32::from(PPS_GUNANGLES)) != 0 {
        let scale = if profile.rerelease { 4096.0 } else { 65536.0 / 360.0 };
        for component in to.gunangles {
            writer.write_short((component * scale).trunc() as i16)?;
        }
    }
    if (bits & u32::from(PPS_BLEND)) != 0 {
        if profile.v2 || profile.rerelease {
            let mut blend = 0u8;
            for i in 0..4 {
                if to.blend[i] != from.blend[i] {
                    blend |= 1 << i;
                }
                if to.damage_blend[i] != from.damage_blend[i] {
                    blend |= 16 << i;
                }
            }
            writer.write_byte(blend)?;
            for i in 0..4 {
                if (blend & (1 << i)) != 0 {
                    writer.write_byte(trunc_byte(to.blend[i] * 255.0))?;
                }
            }
            for i in 0..4 {
                if (blend & (16 << i)) != 0 {
                    writer.write_byte(trunc_byte(to.damage_blend[i] * 255.0))?;
                }
            }
        } else {
            for component in to.blend {
                writer.write_byte(trunc_byte(component * 255.0))?;
            }
        }
    }
    if fog != 0 {
        write_q2pro_fog(&mut *writer, fog, &to.fog)?;
    }
    if (bits & u32::from(PPS_FOV)) != 0 {
        writer.write_byte(to.fov)?;
    }
    if (bits & u32::from(PPS_RDFLAGS)) != 0 {
        writer.write_byte(to.rdflags)?;
    }
    if (bits & u32::from(PPS_STATS)) != 0 {
        if profile.rerelease {
            writer.write_long64(stats as i64)?;
        } else if profile.v2 {
            write_q2pro_var64(&mut *writer, stats)?;
        } else {
            #[allow(clippy::cast_possible_wrap)]
            writer.write_long(stats as i32)?;
        }
        for i in 0..64 {
            if (stats & (1 << i)) != 0 {
                writer.write_short(to.stats[i])?;
            }
        }
    }
    Ok(())
}

/// Map an MVD profile to a wire protocol.
fn mvd_wire_protocol(profile: &MvdProfile) -> ProtocolIdentity {
    match profile.protocol {
        MvdProtocol::Classic => ProtocolIdentity::Q2Classic,
        MvdProtocol::Q2Pro { revision } => ProtocolIdentity::Q2Q2pro {
            revision: u32::from(revision),
        },
        MvdProtocol::Rerelease => ProtocolIdentity::Q2Rerelease,
    }
}

/// Encode authoritative captures into MVD packets (`MvdEncoder`).
#[derive(Debug, Default)]
pub struct MvdEncoder {
    previous: Option<MvdCapture>,
}

impl MvdEncoder {
    /// Fresh encoder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Encode one capture; the first packet is the frame, the rest emissions.
    pub fn capture(&mut self, capture: &MvdCapture) -> Result<Vec<Vec<u8>>, Q2NetError> {
        let profile = mvd_profile(capture.revision, capture.flags)?;
        let old = self.previous.as_ref();
        let changed_world = old.is_none_or(|old| {
            old.servercount != capture.servercount || old.revision != capture.revision || old.flags != capture.flags
        });
        let mut wire = Q2Wire::new(mvd_wire_protocol(&profile))?;
        if let MvdProtocol::Q2Pro { revision } = profile.protocol {
            wire.accept_q2pro_features(revision, if profile.v2 { 24 } else { 8 })?;
        }
        let mut writer = MsgWriter::new(MVD_MAX_MESSAGE, false);
        if changed_world {
            write_mvd_cmd(
                &mut writer,
                MvdOp::Serverdata as u8,
                if profile.revision < 2012 || profile.rerelease {
                    capture.flags as u8
                } else {
                    0
                },
            )?;
            writer.write_long(37)?;
            writer.write_short(capture.revision as i16)?;
            if profile.revision >= 2012 && !profile.rerelease {
                writer.write_short(capture.flags as i16)?;
            }
            writer.write_long(capture.servercount)?;
            writer.write_string(&capture.gamedir)?;
            writer.write_short(capture.dummy)?;
            for (index, value) in &capture.config_strings {
                if usize::from(*index) >= profile.max_config_strings {
                    return Err(Q2NetError::Range("Invalid MVD configstring"));
                }
                writer.write_short(*index as i16)?;
                writer.write_string(value)?;
            }
            writer.write_short(profile.max_config_strings as i16)?;
        } else if let Some(old) = old {
            for (index, value) in &capture.config_strings {
                if old.config_strings.get(index) != Some(value) {
                    writer.write_byte(MvdOp::Configstring as u8)?;
                    writer.write_short(*index as i16)?;
                    writer.write_string(value)?;
                }
            }
            for index in old.config_strings.keys() {
                if !capture.config_strings.contains_key(index) {
                    writer.write_byte(MvdOp::Configstring as u8)?;
                    writer.write_short(*index as i16)?;
                    writer.write_string("")?;
                }
            }
            writer.write_byte(MvdOp::Frame as u8)?;
        }
        if capture.portal_bits.len() > 255 {
            return Err(Q2NetError::Range("MVD portal state too large"));
        }
        writer.write_byte(capture.portal_bits.len() as u8)?;
        writer.write_bytes(&capture.portal_bits)?;
        let empty_players: BTreeMap<u8, PlayerState> = BTreeMap::new();
        let previous_players = if changed_world {
            &empty_players
        } else {
            old.map_or(&empty_players, |old| &old.players)
        };
        for (number, player) in &capture.players {
            if profile.extended {
                write_extended_mvd_player_delta(
                    &mut writer,
                    previous_players.get(number),
                    Some(player),
                    *number,
                    &profile,
                )?;
            } else {
                write_delta_mvd_playerstate(
                    &mut writer,
                    previous_players.get(number),
                    Some(player),
                    *number,
                    previous_players.get(number).is_none(),
                )?;
            }
        }
        for number in previous_players.keys() {
            if !capture.players.contains_key(number) {
                if profile.extended {
                    write_extended_mvd_player_delta(
                        &mut writer,
                        previous_players.get(number),
                        None,
                        *number,
                        &profile,
                    )?;
                } else {
                    write_delta_mvd_playerstate(
                        &mut writer,
                        previous_players.get(number),
                        None,
                        *number,
                        previous_players.get(number).is_none(),
                    )?;
                }
            }
        }
        writer.write_byte(255)?;
        let mut previous_entities: HashMap<u16, &EntityState> = HashMap::new();
        if !changed_world {
            if let Some(old) = old {
                for entity in &old.entities {
                    previous_entities.insert(entity.number, entity);
                }
            }
        }
        let mut current = std::collections::HashSet::new();
        let fresh = EntityState::default();
        for entity in &capture.entities {
            if entity.number < 1 || usize::from(entity.number) >= profile.max_entities || !current.insert(entity.number)
            {
                return Err(Q2NetError::Range("Invalid MVD entity identity"));
            }
            let from = previous_entities.get(&entity.number).copied().unwrap_or(&fresh);
            let is_new = !previous_entities.contains_key(&entity.number);
            wire.write_delta_entity(&mut writer, from, entity, is_new, true)?;
        }
        if !changed_world {
            if let Some(old) = old {
                for entity in &old.entities {
                    if !current.contains(&entity.number) {
                        wire.write_entity_remove(&mut writer, entity.number)?;
                    }
                }
            }
        }
        wire.write_packet_entities_end(&mut writer)?;
        let mut result = vec![writer.bytes().to_vec()];
        for message in &capture.messages {
            result.push(encode_mvd_emission(message)?);
        }
        self.previous = Some(capture.clone());
        Ok(result)
    }

    /// Forget the delta base.
    pub fn reset(&mut self) {
        self.previous = None;
    }
}

/// MVD stream magic (`mvdMagic`).
#[must_use]
pub fn mvd_magic() -> [u8; 4] {
    MVD_MAGIC.to_le_bytes()
}

/// Frame one MVD message with its length prefix (`frameMvdMessage`).
pub fn frame_mvd_message(bytes: &[u8]) -> Result<Vec<u8>, Q2NetError> {
    if bytes.is_empty() || bytes.len() > MVD_MAX_MESSAGE {
        return Err(Q2NetError::Range("MVD message length outside 1..32768"));
    }
    let mut result = Vec::with_capacity(bytes.len() + 2);
    result.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
    result.extend_from_slice(bytes);
    Ok(result)
}

/// Append-only MVD recording (`MvdRecording`).
pub struct MvdRecording {
    write: Box<dyn FnMut(&[u8])>,
    started: bool,
    closed: bool,
}

impl MvdRecording {
    /// Build a recording over a byte sink.
    pub fn new(write: impl FnMut(&[u8]) + 'static) -> Self {
        Self {
            write: Box::new(write),
            started: false,
            closed: false,
        }
    }

    /// Append one message, writing the magic first.
    pub fn append(&mut self, message: &[u8]) -> Result<(), Q2NetError> {
        if self.closed {
            return Err(Q2NetError::Protocol("MVD recording is closed"));
        }
        let framed = frame_mvd_message(message)?;
        if !self.started {
            (self.write)(&mvd_magic());
            self.started = true;
        }
        (self.write)(&framed);
        Ok(())
    }

    /// Write the terminator.
    pub fn close(&mut self) {
        if self.closed {
            return;
        }
        if !self.started {
            (self.write)(&mvd_magic());
            self.started = true;
        }
        (self.write)(&[0, 0]);
        self.closed = true;
    }
}

/// Incremental MVD/GTV framing (`MvdMessageFramer`).
#[derive(Debug)]
pub struct MvdMessageFramer {
    pending: Vec<u8>,
    magic_read: bool,
    ended: bool,
    limit: usize,
}

impl MvdMessageFramer {
    /// Build a framer; `magic` requires the leading magic.
    pub fn new(magic: bool, limit: usize) -> Result<Self, Q2NetError> {
        if limit < 1 || limit > MVD_MAX_MESSAGE {
            return Err(Q2NetError::Range("Invalid MVD/GTV frame limit"));
        }
        Ok(Self {
            pending: Vec::new(),
            magic_read: !magic,
            ended: false,
            limit,
        })
    }

    /// Whether the magic was consumed.
    #[must_use]
    pub fn identified(&self) -> bool {
        self.magic_read
    }

    /// Whether the terminator was consumed.
    #[must_use]
    pub fn finished(&self) -> bool {
        self.ended
    }

    /// Take buffered bytes that arrived after a halted message.
    #[must_use]
    pub fn take_pending(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.pending)
    }

    /// Push bytes; `message` returns `false` to halt after the current message.
    pub fn push(&mut self, bytes: &[u8], mut message: impl FnMut(&[u8]) -> bool) -> Result<(), Q2NetError> {
        if self.ended && !bytes.is_empty() {
            return Err(Q2NetError::Protocol("Data after MVD terminator"));
        }
        let mut offset = 0;
        while offset < bytes.len() {
            let need = if !self.magic_read {
                4
            } else if self.pending.len() < 2 {
                2
            } else {
                usize::from(u16::from_le_bytes([self.pending[0], self.pending[1]])) + 2
            };
            let count = (need - self.pending.len()).min(bytes.len() - offset);
            self.pending.extend_from_slice(&bytes[offset..offset + count]);
            offset += count;
            if self.pending.len() < need {
                break;
            }
            if !self.magic_read {
                if u32::from_le_bytes(self.pending[..4].try_into().unwrap_or([0; 4])) != MVD_MAGIC {
                    return Err(Q2NetError::Protocol("Not an MVD/GTV stream"));
                }
                self.magic_read = true;
                self.pending.clear();
                continue;
            }
            let length = usize::from(u16::from_le_bytes([self.pending[0], self.pending[1]]));
            if length > self.limit {
                return Err(Q2NetError::Range("Oversize MVD/GTV message"));
            }
            if length == 0 {
                self.ended = true;
                self.pending.clear();
                if offset != bytes.len() {
                    return Err(Q2NetError::Protocol("Data after MVD terminator"));
                }
                break;
            }
            if self.pending.len() < length + 2 {
                continue;
            }
            let payload = self.pending[2..].to_vec();
            self.pending.clear();
            if !message(&payload) {
                self.pending = bytes[offset..].to_vec();
                return Ok(());
            }
        }
        Ok(())
    }

    /// Require a complete terminated stream.
    pub fn finish(&self, require_terminator: bool) -> Result<(), Q2NetError> {
        if !self.magic_read || !self.pending.is_empty() || (require_terminator && !self.ended) {
            return Err(Q2NetError::Protocol("Truncated MVD/GTV stream"));
        }
        Ok(())
    }
}

/// Read every message of a recording (`readMvdRecording`).
pub fn read_mvd_recording(bytes: &[u8]) -> Result<Vec<Vec<u8>>, Q2NetError> {
    let mut framer = MvdMessageFramer::new(true, MVD_MAX_MESSAGE)?;
    let mut messages = Vec::new();
    framer.push(bytes, |message| {
        messages.push(message.to_vec());
        true
    })?;
    framer.finish(true)?;
    Ok(messages)
}

/// PVS/PHS channel for visibility queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MvdChannel {
    /// Potentially visible.
    Pvs,
    /// Potentially hearable.
    Phs,
}

/// Source-BSP visibility for MVD projection (`MvdVisibility`).
pub trait MvdVisibility {
    /// Filter entities for the selected player.
    fn entities(&self, entities: &[EntityState], player: &PlayerState, portal_bits: &[u8]) -> Vec<EntityState>;
    /// Whether a leaf is visible/audible to the player.
    fn visible(&self, leaf: u16, channel: MvdChannel, player: &PlayerState, portal_bits: &[u8]) -> bool;
    /// Area bits for the player.
    fn area_bits(&self, player: &PlayerState, portal_bits: &[u8]) -> Vec<u8>;
    /// Whether a world sound is audible to the player.
    fn sound_audible(&self, origin: [f64; 3], player: &PlayerState, portal_bits: &[u8]) -> bool;
    /// World origin of a sound entity.
    fn sound_origin(&self, entity: &EntityState) -> [f64; 3];
}

/// MVD playback into server records (`MvdPlayback`).
pub struct MvdPlayback {
    profile: MvdProfile,
    /// Configstrings by index.
    pub config_strings: HashMap<u16, String>,
    /// Players by slot.
    pub players: HashMap<u8, PlayerState>,
    entities: HashMap<u16, EntityState>,
    wire: Q2Wire,
    embedded: Q2ServerMessageReader,
    data: Option<Q2ServerData>,
    frame_number: i32,
    max_clients: usize,
    portal_bits: Vec<u8>,
    selected: u8,
    dummy: i16,
    visibility: Box<dyn MvdVisibility>,
}

impl MvdPlayback {
    /// Build playback over a visibility provider.
    pub fn new(visibility: impl MvdVisibility + 'static) -> Result<Self, Q2NetError> {
        let profile = mvd_profile(2010, 0)?;
        let protocol = mvd_wire_protocol(&profile);
        Ok(Self {
            profile,
            config_strings: HashMap::new(),
            players: HashMap::new(),
            entities: HashMap::new(),
            wire: Q2Wire::new(protocol)?,
            embedded: Q2ServerMessageReader::new(
                protocol,
                Q2ServerMessageOptions {
                    max_config_strings: 2080,
                    inventory_slots: 256,
                    ..Q2ServerMessageOptions::default()
                },
                HashSet::new(),
                None,
            )?,
            data: None,
            frame_number: 0,
            max_clients: 0,
            portal_bits: Vec::new(),
            selected: 0,
            dummy: -1,
            visibility: Box::new(visibility),
        })
    }

    /// Active wire protocol.
    #[must_use]
    pub fn protocol(&self) -> ProtocolIdentity {
        mvd_wire_protocol(&self.profile)
    }

    /// Current server data, if a header was read.
    #[must_use]
    pub fn header(&self) -> Option<&Q2ServerData> {
        self.data.as_ref()
    }

    /// Selected player slot.
    #[must_use]
    pub fn selected_player(&self) -> u8 {
        self.selected
    }

    /// Select the viewed player.
    pub fn select_player(&mut self, number: u8) -> Result<(), Q2NetError> {
        if usize::from(number) >= self.max_clients || !self.players.contains_key(&number) {
            return Err(Q2NetError::Range("MVD player is not active"));
        }
        self.selected = number;
        Ok(())
    }

    /// Current projection: selected client plus frame.
    #[must_use]
    pub fn projection(&self) -> (u8, Q2WireFrame) {
        (self.selected, self.frame())
    }

    fn player(&self) -> PlayerState {
        self.players.get(&self.selected).cloned().unwrap_or_default()
    }

    fn frame(&self) -> Q2WireFrame {
        let player = self.player();
        let mut entities: Vec<EntityState> = self.entities.values().cloned().collect();
        entities.sort_by_key(|entity| entity.number);
        Q2WireFrame {
            valid: true,
            server_frame: self.frame_number,
            delta_frame: -1,
            suppressed_count: 0,
            area_bits: self.visibility.area_bits(&player, &self.portal_bits),
            player: player.clone(),
            split_players: Vec::new(),
            entities: self.visibility.entities(&entities, &player, &self.portal_bits),
        }
    }

    /// Decode one MVD message into server records.
    pub fn read(&mut self, bytes: &[u8]) -> Result<Vec<Q2ServerRecord>, Q2NetError> {
        if bytes.is_empty() || bytes.len() > MVD_MAX_MESSAGE {
            return Err(Q2NetError::Range("Invalid MVD message length"));
        }
        self.wire.begin(bytes);
        let mut records = Vec::new();
        while self.wire.remaining() > 0 {
            let start = self.wire.position();
            let command = self.wire.with_reader(|reader| Ok(read_mvd_cmd(reader)?))?;
            if command.op == MvdOp::Nop as u8 {
                continue;
            }
            if command.op == MvdOp::Serverdata as u8 {
                self.wire.seek(start);
                let header: MvdHeader = self.wire.with_reader(|reader| Ok(read_mvd_header(reader)?))?;
                let protocol = mvd_wire_protocol(&header.profile);
                let header_end = self.wire.position();
                self.wire = Q2Wire::new(protocol)?;
                self.wire.begin(bytes);
                self.wire.seek(header_end);
                self.embedded = Q2ServerMessageReader::new(
                    protocol,
                    Q2ServerMessageOptions {
                        max_config_strings: header.profile.max_config_strings.min(65535) as u16,
                        inventory_slots: 256,
                        ..Q2ServerMessageOptions::default()
                    },
                    HashSet::new(),
                    None,
                )?;
                if let MvdProtocol::Q2Pro { revision } = header.profile.protocol {
                    self.embedded
                        .accept_q2pro_features(revision, if header.profile.v2 { 24 } else { 8 })?;
                }
                self.profile = header.profile.clone();
                self.config_strings.clear();
                self.players.clear();
                self.entities.clear();
                self.frame_number = 0;
                for (index, value) in &header.config_strings {
                    self.config_strings.insert(*index, value.clone());
                }
                self.max_clients = header.max_clients;
                self.dummy = header.dummy;
                // Reposition past the header before reading portal bits.
                let mut tail = MvdPlaybackTail {
                    wire: &mut self.wire,
                    profile: &self.profile,
                    players: &mut self.players,
                    entities: &mut self.entities,
                    portal_bits: &mut self.portal_bits,
                    frame_number: &mut self.frame_number,
                    selected: &mut self.selected,
                    max_clients: self.max_clients,
                    dummy: self.dummy,
                };
                tail.read_frame()?;
                if !self.players.contains_key(&self.selected) {
                    self.selected = self.players.keys().next().copied().unwrap_or(0);
                }
                self.data = Some(mvd_server_data(&header, self.selected));
                let raw = bytes[start..self.wire.position().min(bytes.len())].to_vec();
                let data = self
                    .data
                    .clone()
                    .unwrap_or_else(|| mvd_server_data(&header, self.selected));
                records.push(Q2ServerRecord {
                    seat: 0,
                    opcode: 12,
                    raw: raw.clone(),
                    event: Q2ServerEvent::ServerData { data },
                });
                for (index, value) in &header.config_strings {
                    records.push(Q2ServerRecord {
                        seat: 0,
                        opcode: 13,
                        raw: raw.clone(),
                        event: Q2ServerEvent::ConfigString {
                            index: *index,
                            value: value.clone(),
                        },
                    });
                }
                records.push(Q2ServerRecord {
                    seat: 0,
                    opcode: 11,
                    raw: raw.clone(),
                    event: Q2ServerEvent::CommandText {
                        text: format!("precache {}\n", header.servercount),
                    },
                });
                records.push(Q2ServerRecord {
                    seat: 0,
                    opcode: 20,
                    raw,
                    event: Q2ServerEvent::Frame { frame: self.frame() },
                });
                continue;
            }
            if command.op == MvdOp::Frame as u8 {
                if self.data.is_none() {
                    return Err(Q2NetError::Protocol("MVD frame before serverdata"));
                }
                let mut tail = MvdPlaybackTail {
                    wire: &mut self.wire,
                    profile: &self.profile,
                    players: &mut self.players,
                    entities: &mut self.entities,
                    portal_bits: &mut self.portal_bits,
                    frame_number: &mut self.frame_number,
                    selected: &mut self.selected,
                    max_clients: self.max_clients,
                    dummy: self.dummy,
                };
                tail.read_frame()?;
                records.push(Q2ServerRecord {
                    seat: 0,
                    opcode: 20,
                    raw: self.wire.raw_slice(start).to_vec(),
                    event: Q2ServerEvent::Frame { frame: self.frame() },
                });
                continue;
            }
            if command.op == MvdOp::Configstring as u8 {
                let (index, value) = self
                    .wire
                    .with_reader(|reader| Ok((reader.word()?, reader.string(2047))))?;
                if usize::from(index) >= self.profile.max_config_strings {
                    return Err(Q2NetError::Protocol("Invalid MVD configstring index"));
                }
                self.config_strings.insert(index, value.clone());
                records.push(Q2ServerRecord {
                    seat: 0,
                    opcode: 13,
                    raw: self.wire.raw_slice(start).to_vec(),
                    event: Q2ServerEvent::ConfigString { index, value },
                });
                continue;
            }
            if command.op == MvdOp::Print as u8 {
                let (level, text) = self
                    .wire
                    .with_reader(|reader| Ok((reader.byte()?, reader.string(2047))))?;
                records.push(Q2ServerRecord {
                    seat: 0,
                    opcode: 10,
                    raw: self.wire.raw_slice(start).to_vec(),
                    event: Q2ServerEvent::Print { level, text },
                });
                continue;
            }
            if command.op == MvdOp::Sound as u8 {
                self.read_mvd_sound(&mut records, start, command.extrabits)?;
                continue;
            }
            if matches!(
                command.op,
                op if op == MvdOp::Unicast as u8
                    || op == MvdOp::UnicastR as u8
                    || op == MvdOp::MulticastAll as u8
                    || op == MvdOp::MulticastAllR as u8
                    || op == MvdOp::MulticastPvs as u8
                    || op == MvdOp::MulticastPvsR as u8
                    || op == MvdOp::MulticastPhs as u8
                    || op == MvdOp::MulticastPhsR as u8
            ) {
                let length = usize::from(self.wire.read_raw_byte()?) | (usize::from(command.extrabits) << 8);
                let mut visible = true;
                if command.op == MvdOp::Unicast as u8 || command.op == MvdOp::UnicastR as u8 {
                    let number = self.wire.read_raw_byte()?;
                    if usize::from(number) >= self.max_clients {
                        return Err(Q2NetError::Protocol("Invalid MVD unicast player"));
                    }
                    visible = number == self.selected;
                } else if command.op != MvdOp::MulticastAll as u8 && command.op != MvdOp::MulticastAllR as u8 {
                    let leaf = self.wire.with_reader(|reader| Ok(reader.word()?))?;
                    let channel = if command.op == MvdOp::MulticastPvs as u8 || command.op == MvdOp::MulticastPvsR as u8
                    {
                        MvdChannel::Pvs
                    } else {
                        MvdChannel::Phs
                    };
                    visible = self
                        .visibility
                        .visible(leaf, channel, &self.player(), &self.portal_bits);
                }
                if length > self.wire.remaining() {
                    return Err(Q2NetError::Protocol("Truncated MVD embedded message"));
                }
                let payload = self.wire.read_raw_data(length)?;
                if visible {
                    records.extend(self.embedded.read(&payload)?);
                }
                continue;
            }
            return Err(Q2NetError::Protocol("Unsupported MVD command"));
        }
        Ok(records)
    }

    /// Decode one routed MVD sound.
    fn read_mvd_sound(
        &mut self,
        records: &mut Vec<Q2ServerRecord>,
        start: usize,
        extrabits: u8,
    ) -> Result<(), Q2NetError> {
        let (flags, index, volume, attenuation, delay_seconds, channel) = self.wire.with_reader(|reader| {
            let flags = reader.byte()?;
            let index = if (flags & 32) != 0 {
                u16::from(reader.word()?)
            } else {
                u16::from(reader.byte()?)
            };
            let volume = if (flags & 1) != 0 {
                f64::from(reader.byte()?) / 255.0
            } else {
                1.0
            };
            let attenuation = if (flags & 2) != 0 {
                f64::from(reader.byte()?) / 64.0
            } else {
                1.0
            };
            let delay_seconds = if (flags & 16) != 0 {
                f64::from(reader.byte()?) / 1000.0
            } else {
                0.0
            };
            Ok((flags, index, volume, attenuation, delay_seconds, reader.word()?))
        })?;
        let entity = channel >> 3;
        if usize::from(entity) >= self.profile.max_entities {
            return Err(Q2NetError::Protocol("Invalid MVD sound entity"));
        }
        if let Some(state) = self.entities.get(&entity) {
            let position = self.visibility.sound_origin(state);
            if (extrabits & 1) != 0
                || self
                    .visibility
                    .sound_audible(position, &self.player(), &self.portal_bits)
            {
                records.push(Q2ServerRecord {
                    seat: 0,
                    opcode: 9,
                    raw: self.wire.raw_slice(start).to_vec(),
                    event: Q2ServerEvent::Sound {
                        sound: Q2SoundMessage {
                            flags: flags | 12,
                            index,
                            entity: u32::from(entity),
                            channel: (channel & 7) as u8,
                            position: Some(position),
                            volume,
                            attenuation,
                            delay_seconds,
                        },
                    },
                });
            }
        }
        Ok(())
    }
}

/// Split borrows for the frame reader.
struct MvdPlaybackTail<'a> {
    wire: &'a mut Q2Wire,
    profile: &'a MvdProfile,
    players: &'a mut HashMap<u8, PlayerState>,
    entities: &'a mut HashMap<u16, EntityState>,
    portal_bits: &'a mut Vec<u8>,
    frame_number: &'a mut i32,
    selected: &'a mut u8,
    max_clients: usize,
    dummy: i16,
}

impl MvdPlaybackTail<'_> {
    /// Read portal bits, players, and entities of one MVD frame.
    fn read_frame(&mut self) -> Result<(), Q2NetError> {
        let portal_length = usize::from(self.wire.read_raw_byte()?);
        if portal_length > self.wire.remaining() {
            return Err(Q2NetError::Protocol("Truncated MVD portal bits"));
        }
        *self.portal_bits = self.wire.read_raw_data(portal_length)?;
        loop {
            let number = self.wire.read_raw_byte()?;
            if number == CLIENTNUM_NONE {
                break;
            }
            if usize::from(number) >= self.max_clients {
                return Err(Q2NetError::Protocol("Invalid MVD player number"));
            }
            let result: MvdPlayer = self.wire.with_reader(|reader| {
                Ok(read_mvd_player(
                    reader,
                    self.players.get(&number),
                    number,
                    self.profile,
                )?)
            })?;
            if result.removed {
                self.players.remove(&number);
            } else {
                let mut ps = result.ps;
                ps.clientnum = i32::from(number);
                self.players.insert(number, ps);
            }
        }
        let mut next: HashMap<u16, EntityState> = HashMap::new();
        let carried: Vec<(u16, EntityState)> = self
            .entities
            .iter()
            .map(|(number, previous)| (*number, previous.clone()))
            .collect();
        for (number, previous) in &carried {
            let mut entity = self.read_entity(previous, *number, 0, None)?;
            entity.old_origin = if (previous.renderfx & 128) != 0 {
                previous.old_origin
            } else {
                previous.origin
            };
            next.insert(*number, entity);
        }
        loop {
            let (number, classic_bits, extended_bits, wide) = if self.profile.extended {
                let (number, bits) = self.wire.with_reader(|reader| Ok(read_q2pro_entity_bits(reader)?))?;
                (number, 0u32, bits, None)
            } else {
                let header = self.wire.read_entity_bits()?;
                match header.bits {
                    Q2EntityBits::Classic(bits) => (header.number, bits, 0, None),
                    Q2EntityBits::Wide(wide) => (header.number, wide.lo, 0, Some(wide)),
                    Q2EntityBits::Q2Pro(bits) => (header.number, 0, bits, None),
                }
            };
            if number == 0 {
                break;
            }
            if usize::from(number) >= self.profile.max_entities {
                return Err(Q2NetError::Protocol("Invalid MVD entity number"));
            }
            let remove = if self.profile.extended || classic_bits == 0 && extended_bits != 0 {
                (extended_bits & u64::from(protocol::U_REMOVE)) != 0
            } else {
                (classic_bits & protocol::U_REMOVE) != 0
            };
            if remove {
                next.remove(&number);
                continue;
            }
            let previous = self.entities.get(&number).cloned().unwrap_or_default();
            let mut entity = if self.profile.rerelease {
                let wide = wide.unwrap_or(WideEntityBits {
                    number,
                    lo: classic_bits,
                    hi: 0,
                });
                self.wire
                    .with_reader(|reader| Ok(RereleaseCodec::read_delta_entity(reader, &previous, number, wide)?))?
            } else if self.profile.extended {
                let features = Q2ProFeatures {
                    revision: if self.profile.fog {
                        1026
                    } else if self.profile.v2 {
                        1025
                    } else {
                        1024
                    },
                    flags: if self.profile.v2 { 24 } else { 8 },
                };
                self.wire
                    .with_reader(|reader| Ok(read_q2pro_entity(reader, features, &previous, number, extended_bits)?))?
            } else {
                self.wire.read_delta_entity(
                    &previous,
                    number,
                    Q2EntityHeader {
                        number,
                        bits: Q2EntityBits::Classic(classic_bits),
                    },
                )?
            };
            let mask_bits: u64 = if self.profile.extended || extended_bits != 0 {
                extended_bits
            } else {
                u64::from(classic_bits)
            };
            if (mask_bits & u64::from(protocol::U_OLDORIGIN)) == 0 {
                entity.old_origin = if (previous.renderfx & 128) != 0 {
                    previous.old_origin
                } else {
                    previous.origin
                };
            }
            if (mask_bits & u64::from(protocol::U_FRAME16)) != 0 {
                entity.frame &= 65535;
            }
            if (mask_bits & u64::from(protocol::U_SKIN8 | protocol::U_SKIN16)) == u64::from(protocol::U_SKIN16) {
                entity.skinnum &= 65535;
            }
            if (mask_bits & u64::from(protocol::U_EFFECTS8 | protocol::U_EFFECTS16)) == u64::from(protocol::U_EFFECTS16)
            {
                entity.effects &= 65535;
            }
            if (mask_bits & u64::from(protocol::U_RENDERFX8 | protocol::U_RENDERFX16))
                == u64::from(protocol::U_RENDERFX16)
            {
                entity.renderfx &= 65535;
            }
            if !self.profile.extended && (mask_bits & u64::from(protocol::U_SOLID)) != 0 {
                entity.solid &= 65535;
            }
            next.insert(number, entity);
        }
        for (number, player) in self.players.iter() {
            let Some(entity) = next.get_mut(&u16::from(*number).wrapping_add(1)) else {
                continue;
            };
            if i32::from(*number) == i32::from(self.dummy) || player.pmove.pm_type != 0 {
                continue;
            }
            for axis in 0..3 {
                entity.origin[axis] = if self.profile.rerelease {
                    f64::from(player.pmove.origin_f[axis])
                } else {
                    f64::from(player.pmove.origin[axis]) / 8.0
                };
            }
            let pitch = player.viewangles[0];
            entity.angles[0] = (if pitch > 180.0 { pitch - 360.0 } else { pitch }) / 3.0;
            entity.angles[1] = player.viewangles[1];
            entity.angles[2] = 0.0;
        }
        *self.entities = next;
        if !self.players.contains_key(self.selected) {
            *self.selected = self.players.keys().next().copied().unwrap_or(0);
        }
        *self.frame_number += 1;
        Ok(())
    }

    /// Read one entity delta under the active profile.
    fn read_entity(
        &mut self,
        from: &EntityState,
        number: u16,
        bits: u32,
        extended_bits: Option<u64>,
    ) -> Result<EntityState, Q2NetError> {
        if self.profile.rerelease {
            let wide = WideEntityBits {
                number,
                lo: bits,
                hi: 0,
            };
            Ok(self
                .wire
                .with_reader(|reader| Ok(RereleaseCodec::read_delta_entity(reader, from, number, wide)?))?)
        } else if self.profile.extended {
            let features = Q2ProFeatures {
                revision: if self.profile.fog {
                    1026
                } else if self.profile.v2 {
                    1025
                } else {
                    1024
                },
                flags: if self.profile.v2 { 24 } else { 8 },
            };
            Ok(self.wire.with_reader(|reader| {
                Ok(read_q2pro_entity(
                    reader,
                    features,
                    from,
                    number,
                    extended_bits.unwrap_or(0),
                )?)
            })?)
        } else {
            self.wire.read_delta_entity(
                from,
                number,
                Q2EntityHeader {
                    number,
                    bits: Q2EntityBits::Classic(bits),
                },
            )
        }
    }
}

/// One persistent zlib stream across every GTV packet and sync flush.
enum GtvCompression {
    Encode(Compress),
    Decode(Decompress),
}

impl GtvCompression {
    /// Push bytes through the stream, collecting output.
    fn push(&mut self, bytes: &[u8]) -> Result<Vec<u8>, Q2NetError> {
        let mut out = Vec::new();
        let mut buf = [0u8; 4096];
        match self {
            GtvCompression::Encode(compressor) => {
                let mut input = bytes;
                loop {
                    let before_in = compressor.total_in();
                    let before_out = compressor.total_out();
                    let status = compressor
                        .compress(input, &mut buf, FlushCompress::Sync)
                        .map_err(|_| Q2NetError::Protocol("GTV compression failed"))?;
                    let consumed = (compressor.total_in() - before_in) as usize;
                    let produced = (compressor.total_out() - before_out) as usize;
                    out.extend_from_slice(&buf[..produced]);
                    if out.len() > MVD_MAX_MESSAGE * 64 {
                        return Err(Q2NetError::Protocol("GTV decompression output limit exceeded"));
                    }
                    input = &input[consumed.min(input.len())..];
                    if input.is_empty() && (produced < buf.len() || status != Status::Ok) {
                        break;
                    }
                    if status == Status::BufError && input.is_empty() {
                        break;
                    }
                }
            }
            GtvCompression::Decode(decompressor) => {
                let mut input = bytes;
                loop {
                    if input.is_empty() {
                        break;
                    }
                    let before_in = decompressor.total_in();
                    let before_out = decompressor.total_out();
                    let status = decompressor
                        .decompress(input, &mut buf, FlushDecompress::None)
                        .map_err(|_| Q2NetError::Protocol("GTV decompression failed"))?;
                    let consumed = (decompressor.total_in() - before_in) as usize;
                    let produced = (decompressor.total_out() - before_out) as usize;
                    out.extend_from_slice(&buf[..produced]);
                    if out.len() > MVD_MAX_MESSAGE * 64 {
                        return Err(Q2NetError::Protocol("GTV decompression output limit exceeded"));
                    }
                    input = &input[consumed.min(input.len())..];
                    if status == Status::BufError || (consumed == 0 && produced == 0) {
                        break;
                    }
                }
            }
        }
        Ok(out)
    }
}

/// Frame one GTV packet (`packet`).
fn gtv_packet(op: u8, body: &[u8]) -> Result<Vec<u8>, Q2NetError> {
    let mut message = Vec::with_capacity(body.len() + 1);
    message.push(op);
    message.extend_from_slice(body);
    frame_mvd_message(&message)
}

/// Validate GTV identity/command text.
fn checked_gtv_text(text: &str) -> Result<(), Q2NetError> {
    if text.contains('\0') || text.chars().any(|c| c as u32 > 255) {
        return Err(Q2NetError::Protocol("GTV strings must be non-NUL single-byte text"));
    }
    Ok(())
}

/// GTV identity (`GtvIdentity`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GtvIdentity {
    /// Username.
    pub username: String,
    /// Password.
    pub password: String,
    /// Client version.
    pub version: String,
}

/// GTV client event (`GtvEvent`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GtvEvent {
    /// Negotiated hello.
    Hello {
        /// Negotiated flags.
        flags: u32,
    },
    /// Stream payload.
    Data {
        /// Bytes.
        bytes: Vec<u8>,
    },
    /// Pong.
    Pong,
    /// Stream started.
    Started,
    /// Stream stopped.
    Stopped,
    /// Stream suspended (empty data).
    Suspended,
    /// Stream resumed.
    Resumed,
    /// Connection closed.
    Closed {
        /// Reason.
        reason: String,
    },
}

/// GTV client phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GtvPhase {
    Magic,
    Hello,
    Connected,
    Starting,
    Reading,
    Waiting,
    Stopping,
    Closed,
}

/// GTV client (`GtvClient`).
pub struct GtvClient {
    send: Box<dyn FnMut(&[u8]) -> Result<(), Q2NetError>>,
    identity: GtvIdentity,
    requested_flags: u32,
    phase: GtvPhase,
    magic: Vec<u8>,
    frames: MvdMessageFramer,
    compression: Option<GtvCompression>,
    flags: u32,
    started: bool,
}

impl GtvClient {
    /// Build a client over a send sink.
    pub fn new(
        send: impl FnMut(&[u8]) -> Result<(), Q2NetError> + 'static,
        identity: GtvIdentity,
        requested_flags: u32,
    ) -> Result<Self, Q2NetError> {
        checked_gtv_text(&identity.username)?;
        checked_gtv_text(&identity.password)?;
        checked_gtv_text(&identity.version)?;
        Ok(Self {
            send: Box::new(send),
            identity,
            requested_flags,
            phase: GtvPhase::Magic,
            magic: Vec::new(),
            frames: MvdMessageFramer::new(false, MVD_MAX_MESSAGE)?,
            compression: None,
            flags: 0,
            started: false,
        })
    }

    /// Send the opening magic.
    pub fn start(&mut self) -> Result<(), Q2NetError> {
        if self.started || self.phase != GtvPhase::Magic {
            return Err(Q2NetError::Protocol("GTV already started"));
        }
        self.started = true;
        (self.send)(&mvd_magic())
    }

    /// Request the stream.
    pub fn request_start(&mut self, max_buffered_packets: u16) -> Result<(), Q2NetError> {
        if self.phase != GtvPhase::Connected {
            return Err(Q2NetError::Protocol("GTV is not ready to start"));
        }
        let packet = gtv_packet(GtvClientOp::StreamStart as u8, &max_buffered_packets.to_le_bytes())?;
        (self.send)(&packet)?;
        self.phase = GtvPhase::Starting;
        Ok(())
    }

    /// Stop the stream.
    pub fn request_stop(&mut self) -> Result<(), Q2NetError> {
        if self.phase != GtvPhase::Reading && self.phase != GtvPhase::Waiting {
            return Err(Q2NetError::Protocol("GTV is not streaming"));
        }
        (self.send)(&gtv_packet(GtvClientOp::StreamStop as u8, &[])?)?;
        self.phase = GtvPhase::Stopping;
        Ok(())
    }

    /// Send a ping.
    pub fn ping(&mut self) -> Result<(), Q2NetError> {
        self.connected()?;
        (self.send)(&gtv_packet(GtvClientOp::Ping as u8, &[])?)?;
        Ok(())
    }

    /// Forward a console command.
    pub fn command(&mut self, text: &str) -> Result<(), Q2NetError> {
        self.connected()?;
        checked_gtv_text(text)?;
        if (self.flags & u32::from(GTF_STRINGCMDS)) == 0 {
            return Err(Q2NetError::Protocol("GTV command forwarding was not negotiated"));
        }
        let clipped: String = text.chars().take(150).collect();
        let mut writer = MsgWriter::new(MAX_GTC_MSGLEN, false);
        writer.write_string(&clipped)?;
        (self.send)(&gtv_packet(GtvClientOp::Stringcmd as u8, writer.bytes())?)?;
        Ok(())
    }

    /// Negotiated flags.
    #[must_use]
    pub fn flags(&self) -> u32 {
        self.flags
    }

    fn connected(&self) -> Result<(), Q2NetError> {
        if matches!(self.phase, GtvPhase::Magic | GtvPhase::Hello | GtvPhase::Closed) {
            return Err(Q2NetError::Protocol("GTV is not connected"));
        }
        Ok(())
    }

    /// Feed received bytes, returning new events.
    pub fn receive(&mut self, input: &[u8]) -> Result<Vec<GtvEvent>, Q2NetError> {
        if !self.started {
            return Err(Q2NetError::Protocol("GTV was not started"));
        }
        if self.phase == GtvPhase::Closed {
            return Err(Q2NetError::Protocol("GTV connection is closed"));
        }
        let result = self.receive_inner(input);
        if result.is_err() {
            self.close();
        }
        result
    }

    fn receive_inner(&mut self, input: &[u8]) -> Result<Vec<GtvEvent>, Q2NetError> {
        let mut bytes = input;
        let events = Vec::new();
        if self.phase == GtvPhase::Magic {
            let count = (4 - self.magic.len()).min(bytes.len());
            self.magic.extend_from_slice(&bytes[..count]);
            bytes = &bytes[count..];
            if self.magic.len() != 4 {
                return Ok(events);
            }
            if self.magic != mvd_magic() {
                return Err(Q2NetError::Protocol("Not a GTV server"));
            }
            let mut writer = MsgWriter::new(MAX_GTC_MSGLEN, false);
            writer.write_short(GTV_PROTOCOL_VERSION as i16)?;
            writer.write_long(self.requested_flags as i32)?;
            writer.write_long(0)?;
            writer.write_string(&self.identity.username)?;
            writer.write_string(&self.identity.password)?;
            writer.write_string(&self.identity.version)?;
            let hello = gtv_packet(GtvClientOp::Hello as u8, writer.bytes())?;
            if hello.len() - 2 > MAX_GTC_MSGLEN {
                return Err(Q2NetError::Protocol("GTV identity exceeds message limit"));
            }
            self.phase = GtvPhase::Hello;
            (self.send)(&hello)?;
        }
        if self.compression.is_some() {
            let decompressed = self.compression.as_mut().expect("checked").push(bytes)?;
            return self.receive_framed(&decompressed, events);
        }
        self.receive_framed(bytes, events)
    }

    fn receive_framed(&mut self, bytes: &[u8], mut events: Vec<GtvEvent>) -> Result<Vec<GtvEvent>, Q2NetError> {
        let had_compression = self.compression.is_some();
        let mut switched = false;
        let mut failure: Option<Q2NetError> = None;
        {
            let frames = &mut self.frames;
            let compression = &mut self.compression;
            let phase = &mut self.phase;
            let flags = &mut self.flags;
            let requested = self.requested_flags;
            frames.push(bytes, |message| {
                if failure.is_some() {
                    return false;
                }
                match consume_gtv_message(message, &mut events, compression, phase, flags, requested) {
                    Ok(()) => {
                        if !had_compression && compression.is_some() {
                            switched = true;
                            false
                        } else {
                            true
                        }
                    }
                    Err(error) => {
                        failure = Some(error);
                        false
                    }
                }
            })?;
        }
        if let Some(error) = failure {
            return Err(error);
        }
        // Hello itself is plain; even a shared TCP read may switch
        // immediately to zlib.
        if switched && self.compression.is_some() {
            let pending = self.frames.take_pending();
            if !pending.is_empty() {
                let decompressed = self.compression.as_mut().expect("checked").push(&pending)?;
                let mut failure: Option<Q2NetError> = None;
                {
                    let frames = &mut self.frames;
                    let compression = &mut self.compression;
                    let phase = &mut self.phase;
                    let flags = &mut self.flags;
                    let requested = self.requested_flags;
                    frames.push(&decompressed, |message| {
                        if failure.is_some() {
                            return false;
                        }
                        match consume_gtv_message(message, &mut events, compression, phase, flags, requested) {
                            Ok(()) => true,
                            Err(error) => {
                                failure = Some(error);
                                false
                            }
                        }
                    })?;
                }
                if let Some(error) = failure {
                    return Err(error);
                }
            }
        }
        if self.frames.finished() {
            self.close();
            events.push(GtvEvent::Closed {
                reason: "end of stream".to_string(),
            });
        }
        Ok(events)
    }

    /// Close the client.
    pub fn close(&mut self) {
        self.phase = GtvPhase::Closed;
        self.compression = None;
    }
}

/// Consume one GTV server message.
fn consume_gtv_message(
    bytes: &[u8],
    events: &mut Vec<GtvEvent>,
    compression: &mut Option<GtvCompression>,
    phase: &mut GtvPhase,
    flags: &mut u32,
    requested: u32,
) -> Result<(), Q2NetError> {
    let mut reader = MsgReader::new(bytes);
    let opcode = reader.byte()?;
    match opcode {
        op if op == GtvServerOp::Hello as u8 => {
            if *phase != GtvPhase::Hello {
                return Err(Q2NetError::Protocol("Unexpected GTV hello"));
            }
            *flags = reader.long()? as u32;
            if (*flags & !requested) != 0 {
                return Err(Q2NetError::Protocol("Unrequested GTV flags"));
            }
            if (*flags & u32::from(GTF_DEFLATE)) != 0 {
                *compression = Some(GtvCompression::Decode(Decompress::new(true)));
            }
            *phase = GtvPhase::Connected;
            events.push(GtvEvent::Hello { flags: *flags });
        }
        op if op == GtvServerOp::StreamStart as u8 => {
            if *phase != GtvPhase::Starting {
                return Err(Q2NetError::Protocol("Unexpected GTV start acknowledgement"));
            }
            *phase = GtvPhase::Reading;
            events.push(GtvEvent::Started);
        }
        op if op == GtvServerOp::StreamStop as u8 => {
            if *phase != GtvPhase::Stopping {
                return Err(Q2NetError::Protocol("Unexpected GTV stop acknowledgement"));
            }
            *phase = GtvPhase::Connected;
            events.push(GtvEvent::Stopped);
        }
        op if op == GtvServerOp::StreamData as u8 => {
            if *phase == GtvPhase::Stopping {
                return Ok(());
            }
            if *phase != GtvPhase::Reading && *phase != GtvPhase::Waiting {
                return Err(Q2NetError::Protocol("Unexpected GTV stream data"));
            }
            if bytes.len() == 1 {
                *phase = GtvPhase::Waiting;
                events.push(GtvEvent::Suspended);
            } else {
                if *phase == GtvPhase::Waiting {
                    events.push(GtvEvent::Resumed);
                }
                *phase = GtvPhase::Reading;
                events.push(GtvEvent::Data {
                    bytes: bytes[1..].to_vec(),
                });
            }
            return Ok(());
        }
        op if op == GtvServerOp::Pong as u8 => {
            if matches!(*phase, GtvPhase::Magic | GtvPhase::Hello | GtvPhase::Closed) {
                return Err(Q2NetError::Protocol("GTV is not connected"));
            }
            events.push(GtvEvent::Pong);
        }
        op if op == GtvServerOp::Error as u8
            || op == GtvServerOp::BadRequest as u8
            || op == GtvServerOp::NoAccess as u8
            || op == GtvServerOp::Disconnect as u8
            || op == GtvServerOp::Reconnect as u8 =>
        {
            *phase = GtvPhase::Closed;
            *compression = None;
            events.push(GtvEvent::Closed {
                reason: gtv_close_reason(op).to_string(),
            });
            return Ok(());
        }
        _ => return Err(Q2NetError::Protocol("Unknown GTV server opcode")),
    }
    if reader.remaining() != 0 {
        return Err(Q2NetError::Protocol("Trailing GTV control bytes"));
    }
    Ok(())
}

/// Reverse opcode name for close reasons (`GtvServerOpT[opcode]`).
fn gtv_close_reason(opcode: u8) -> &'static str {
    if opcode == GtvServerOp::Error as u8 {
        "GTS_ERROR"
    } else if opcode == GtvServerOp::BadRequest as u8 {
        "GTS_BADREQUEST"
    } else if opcode == GtvServerOp::NoAccess as u8 {
        "GTS_NOACCESS"
    } else if opcode == GtvServerOp::Disconnect as u8 {
        "GTS_DISCONNECT"
    } else if opcode == GtvServerOp::Reconnect as u8 {
        "GTS_RECONNECT"
    } else {
        "GTV closed"
    }
}

/// GTV client hello (`GtvClientHello`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GtvClientHello {
    /// Username.
    pub username: String,
    /// Password.
    pub password: String,
    /// Client version.
    pub version: String,
    /// Requested flags.
    pub flags: u32,
}

/// GTV client request (`GtvRequest`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GtvRequest {
    /// Hello.
    Hello {
        /// Hello.
        hello: GtvClientHello,
    },
    /// Start streaming.
    Start {
        /// Buffering request.
        max_buffered_packets: u16,
    },
    /// Ping.
    Ping,
    /// Stop streaming.
    Stop,
    /// Console command.
    Command {
        /// Text.
        text: String,
    },
}

/// Read one client request; client traffic is never compressed.
pub fn read_gtv_request(bytes: &[u8]) -> Result<GtvRequest, Q2NetError> {
    if bytes.is_empty() || bytes.len() > MAX_GTC_MSGLEN {
        return Err(Q2NetError::Range("Invalid GTV client message length"));
    }
    let mut reader = MsgReader::new(bytes);
    let request = match reader.byte()? {
        op if op == GtvClientOp::Hello as u8 => {
            if reader.word()? != GTV_PROTOCOL_VERSION {
                return Err(Q2NetError::Protocol("Unsupported GTV protocol"));
            }
            let flags = reader.long()? as u32;
            let _reserved = reader.long()?;
            GtvRequest::Hello {
                hello: GtvClientHello {
                    username: reader.string(256),
                    password: reader.string(256),
                    version: reader.string(256),
                    flags,
                },
            }
        }
        op if op == GtvClientOp::Ping as u8 => GtvRequest::Ping,
        op if op == GtvClientOp::StreamStart as u8 => GtvRequest::Start {
            max_buffered_packets: reader.word()?,
        },
        op if op == GtvClientOp::StreamStop as u8 => GtvRequest::Stop,
        op if op == GtvClientOp::Stringcmd as u8 => GtvRequest::Command {
            text: reader.string(256),
        },
        _ => return Err(Q2NetError::Protocol("Unknown GTV client opcode")),
    };
    if reader.remaining() != 0 {
        return Err(Q2NetError::Protocol("Trailing GTV request bytes"));
    }
    Ok(request)
}

/// One server-side GTV wire stream (`GtvServerStream`).
pub struct GtvServerStream {
    compression: Option<GtvCompression>,
    greeted: bool,
    closed: bool,
}

impl GtvServerStream {
    /// Fresh stream.
    #[must_use]
    pub fn new() -> Self {
        Self {
            compression: None,
            greeted: false,
            closed: false,
        }
    }

    /// Send the hello, enabling compression when negotiated.
    pub fn hello(&mut self, flags: u8) -> Result<Vec<u8>, Q2NetError> {
        if self.greeted || self.closed {
            return Err(Q2NetError::Protocol("GTV server hello already sent"));
        }
        if (flags & !(GTF_DEFLATE | GTF_STRINGCMDS)) != 0 {
            return Err(Q2NetError::Protocol("Unknown GTV negotiated flags"));
        }
        self.greeted = true;
        if (flags & GTF_DEFLATE) != 0 {
            self.compression = Some(GtvCompression::Encode(Compress::new(Compression::default(), true)));
        }
        gtv_packet(GtvServerOp::Hello as u8, &u32::from(flags).to_le_bytes())
    }

    /// Send one framed message, compressing when negotiated.
    pub fn message(&mut self, opcode: GtvServerOp, body: &[u8]) -> Result<Vec<u8>, Q2NetError> {
        if !self.greeted || self.closed || opcode as u8 == GtvServerOp::Hello as u8 {
            return Err(Q2NetError::Protocol("Invalid GTV server stream state"));
        }
        let bytes = gtv_packet(opcode as u8, body)?;
        match &mut self.compression {
            Some(compression) => compression.push(&bytes),
            None => Ok(bytes),
        }
    }

    /// Close the stream.
    pub fn close(&mut self) {
        self.closed = true;
        self.compression = None;
    }
}

impl Default for GtvServerStream {
    fn default() -> Self {
        Self::new()
    }
}

/// Synthesize server data from an MVD header.
fn mvd_server_data(header: &MvdHeader, selected: u8) -> Q2ServerData {
    match header.profile.protocol {
        MvdProtocol::Q2Pro { revision } => Q2ServerData::Q2Pro(crate::q2_variants::Q2ProServerData {
            servercount: header.servercount,
            attractloop: true,
            gamedir: header.gamedir.clone(),
            clientnum: i16::from(selected),
            levelname: header.levelname.clone(),
            version: revision,
            server_state: 2,
            wire_flags: if header.profile.v2 { 24 } else { 8 },
        }),
        MvdProtocol::Rerelease => Q2ServerData::Rerelease(crate::q2_variants::RereleaseServerData {
            servercount: header.servercount,
            attractloop: true,
            gamedir: header.gamedir.clone(),
            clientnum: i16::from(selected),
            levelname: header.levelname.clone(),
            protocol_revision: header.profile.revision,
            server_state: 2,
            wire_flags: 0,
            server_fps: 10,
        }),
        MvdProtocol::Classic => Q2ServerData::Vanilla(crate::q2::ServerData {
            servercount: header.servercount,
            attractloop: true,
            gamedir: header.gamedir.clone(),
            clientnum: i16::from(selected),
            levelname: header.levelname.clone(),
        }),
    }
}

/// Options for [`MvdBroadcast`].
pub struct MvdBroadcastOptions {
    /// Authorize a viewer hello.
    pub authorize: Box<dyn Fn(&GtvClientHello) -> bool>,
    /// Handle a forwarded console command.
    pub command: Option<Box<dyn FnMut(&str)>>,
    /// Record every packet.
    pub record: Option<Box<dyn FnMut(&[u8])>>,
    /// Viewer limit.
    pub max_viewers: usize,
}

impl MvdBroadcastOptions {
    /// Build options with a 16-viewer limit.
    pub fn new(authorize: impl Fn(&GtvClientHello) -> bool + 'static) -> Self {
        Self {
            authorize: Box::new(authorize),
            command: None,
            record: None,
            max_viewers: 16,
        }
    }
}

/// One GTV viewer of a broadcast.
struct MvdViewer {
    stream: TcpStream,
    gtv: GtvServerStream,
    frames: MvdMessageFramer,
    identified: bool,
    authorized: bool,
    active: bool,
    needs_gamestate: bool,
    flags: u8,
    closed: bool,
    sequence: u64,
    last_activity: Instant,
}

impl MvdViewer {
    /// Send bytes, retiring on failure.
    fn send(&mut self, bytes: &[u8]) -> Result<(), Q2NetError> {
        // Nonblocking reads stay responsive; writes take a bounded blocking
        // window instead of an async queue.
        self.stream
            .set_nonblocking(false)
            .map_err(|error| Q2NetError::Io(error.to_string()))?;
        self.stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .map_err(|error| Q2NetError::Io(error.to_string()))?;
        let result = self
            .stream
            .write_all(bytes)
            .map_err(|error| Q2NetError::Io(error.to_string()));
        let _ = self.stream.set_nonblocking(true);
        result
    }
}

/// Shared captures feeding one recording and every GTV viewer (`MvdBroadcast`).
pub struct MvdBroadcast {
    encoder: MvdEncoder,
    recording: Option<MvdRecording>,
    latest: Option<MvdCapture>,
    sequence: u64,
    viewers: Vec<MvdViewer>,
    listener: Option<TcpListener>,
    closed: bool,
    options: MvdBroadcastOptions,
}

impl MvdBroadcast {
    /// Build a broadcast.
    pub fn new(mut options: MvdBroadcastOptions) -> Result<Self, Q2NetError> {
        if options.max_viewers < 1 {
            return Err(Q2NetError::Range("Invalid GTV viewer limit"));
        }
        let recording = options.record.take().map(MvdRecording::new);
        Ok(Self {
            encoder: MvdEncoder::new(),
            recording,
            latest: None,
            sequence: 0,
            viewers: Vec::new(),
            listener: None,
            closed: false,
            options,
        })
    }

    /// Publish one authoritative capture to the recording and viewers.
    pub fn observe(&mut self, capture: &MvdCapture) -> Result<(), Q2NetError> {
        if self.closed {
            return Err(Q2NetError::Protocol("MVD producer is closed"));
        }
        let packets = self.encoder.capture(capture)?;
        self.latest = Some(capture.clone());
        self.sequence += 1;
        if let Some(recording) = &mut self.recording {
            for packet in &packets {
                recording.append(packet)?;
            }
        }
        let sequence = self.sequence;
        for index in 0..self.viewers.len() {
            let viewer = &mut self.viewers[index];
            if !viewer.active || sequence <= viewer.sequence {
                continue;
            }
            let messages = if viewer.needs_gamestate || viewer.sequence + 1 != sequence {
                MvdEncoder::new().capture(&MvdCapture {
                    messages: Vec::new(),
                    ..capture.clone()
                })?
            } else {
                packets.clone()
            };
            let mut failed = false;
            for bytes in &messages {
                match viewer.gtv.message(GtvServerOp::StreamData, bytes) {
                    Ok(framed) => {
                        if viewer.send(&framed).is_err() {
                            failed = true;
                            break;
                        }
                    }
                    Err(_) => {
                        failed = true;
                        break;
                    }
                }
            }
            if failed {
                self.viewers[index].closed = true;
            } else {
                self.viewers[index].needs_gamestate = false;
                self.viewers[index].sequence = sequence;
            }
        }
        self.viewers.retain(|viewer| !viewer.closed);
        Ok(())
    }

    /// Listen for viewers, returning the bound port.
    pub fn listen(&mut self, host: &str, port: u16) -> Result<u16, Q2NetError> {
        if self.closed || self.listener.is_some() {
            return Err(Q2NetError::Protocol("GTV listener already owned or closed"));
        }
        let listener =
            TcpListener::bind(format!("{host}:{port}")).map_err(|error| Q2NetError::Io(error.to_string()))?;
        listener
            .set_nonblocking(true)
            .map_err(|error| Q2NetError::Io(error.to_string()))?;
        let bound = listener
            .local_addr()
            .map_err(|error| Q2NetError::Io(error.to_string()))?
            .port();
        self.listener = Some(listener);
        Ok(bound)
    }

    /// Accept viewers and drive their requests once.
    pub fn poll(&mut self) -> Result<(), Q2NetError> {
        if self.closed {
            return Ok(());
        }
        if self.listener.is_some() {
            loop {
                let accepted = self.listener.as_ref().expect("checked").accept();
                match accepted {
                    Ok((stream, _)) => self.attach(stream)?,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(error) => return Err(Q2NetError::Io(error.to_string())),
                }
            }
        }
        let mut index = 0;
        while index < self.viewers.len() {
            let mut buf = [0u8; 4096];
            let read = self.viewers[index].stream.read(&mut buf);
            match read {
                Ok(0) => self.viewers[index].closed = true,
                Ok(count) => {
                    self.viewers[index].last_activity = Instant::now();
                    let chunk = buf[..count].to_vec();
                    if self.drive_viewer(index, &chunk).is_err() {
                        self.viewers[index].closed = true;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(_) => self.viewers[index].closed = true,
            }
            if self.viewers[index].last_activity.elapsed() > Duration::from_secs(90) {
                self.viewers[index].closed = true;
            }
            if self.viewers[index].closed {
                self.viewers[index].gtv.close();
                let _ = self.viewers[index].stream.shutdown(std::net::Shutdown::Both);
                self.viewers.remove(index);
            } else {
                index += 1;
            }
        }
        Ok(())
    }

    /// Attach one accepted socket.
    fn attach(&mut self, stream: TcpStream) -> Result<(), Q2NetError> {
        if self.closed || self.viewers.len() >= self.options.max_viewers {
            return Ok(());
        }
        stream
            .set_nodelay(true)
            .map_err(|error| Q2NetError::Io(error.to_string()))?;
        stream
            .set_nonblocking(true)
            .map_err(|error| Q2NetError::Io(error.to_string()))?;
        self.viewers.push(MvdViewer {
            stream,
            gtv: GtvServerStream::new(),
            frames: MvdMessageFramer::new(true, 256)?,
            identified: false,
            authorized: false,
            active: false,
            needs_gamestate: true,
            flags: 0,
            closed: false,
            sequence: 0,
            last_activity: Instant::now(),
        });
        Ok(())
    }

    /// Feed one chunk into a viewer's framer and requests.
    fn drive_viewer(&mut self, index: usize, chunk: &[u8]) -> Result<(), Q2NetError> {
        let mut requests = Vec::new();
        {
            let viewer = &mut self.viewers[index];
            let mut failure: Option<Q2NetError> = None;
            viewer.frames.push(chunk, |payload| {
                if failure.is_some() {
                    return false;
                }
                match read_gtv_request(payload) {
                    Ok(request) => {
                        requests.push(request);
                        true
                    }
                    Err(error) => {
                        failure = Some(error);
                        false
                    }
                }
            })?;
            if let Some(error) = failure {
                return Err(error);
            }
            if !viewer.identified && viewer.frames.identified() {
                viewer.identified = true;
                viewer.send(&mvd_magic())?;
            }
            if viewer.frames.finished() {
                viewer.closed = true;
                return Ok(());
            }
        }
        for request in requests {
            self.handle_request(index, request)?;
        }
        Ok(())
    }

    /// Handle one viewer request.
    fn handle_request(&mut self, index: usize, request: GtvRequest) -> Result<(), Q2NetError> {
        match request {
            GtvRequest::Hello { hello } => {
                if self.viewers[index].authorized {
                    return Err(Q2NetError::Protocol("Duplicate GTV hello"));
                }
                if !(self.options.authorize)(&hello) {
                    let denied = frame_mvd_message(&[GtvServerOp::NoAccess as u8])?;
                    let _ = self.viewers[index].send(&denied);
                    self.viewers[index].closed = true;
                    return Ok(());
                }
                let mask = if self.options.command.is_some() { 3 } else { 1 };
                let flags = (hello.flags as u8) & mask;
                let greeting = self.viewers[index].gtv.hello(flags)?;
                self.viewers[index].send(&greeting)?;
                self.viewers[index].flags = flags;
                self.viewers[index].authorized = true;
            }
            request => {
                if !self.viewers[index].authorized {
                    return Err(Q2NetError::Protocol("GTV request before authorization"));
                }
                match request {
                    GtvRequest::Ping => {
                        let pong = self.viewers[index].gtv.message(GtvServerOp::Pong, &[])?;
                        self.viewers[index].send(&pong)?;
                    }
                    GtvRequest::Start {
                        max_buffered_packets: _,
                    } => {
                        if self.viewers[index].active {
                            return Err(Q2NetError::Protocol("GTV already streaming"));
                        }
                        self.viewers[index].active = true;
                        self.viewers[index].needs_gamestate = true;
                        let ack = self.viewers[index].gtv.message(GtvServerOp::StreamStart, &[])?;
                        self.viewers[index].send(&ack)?;
                        if let Some(latest) = self.latest.clone() {
                            let sequence = self.sequence;
                            let initial = MvdEncoder::new().capture(&MvdCapture {
                                messages: Vec::new(),
                                ..latest
                            })?;
                            for packet in &initial {
                                let framed = self.viewers[index].gtv.message(GtvServerOp::StreamData, packet)?;
                                self.viewers[index].send(&framed)?;
                            }
                            self.viewers[index].needs_gamestate = false;
                            self.viewers[index].sequence = sequence;
                        } else {
                            let empty = self.viewers[index].gtv.message(GtvServerOp::StreamData, &[])?;
                            self.viewers[index].send(&empty)?;
                        }
                    }
                    GtvRequest::Stop => {
                        if !self.viewers[index].active {
                            return Err(Q2NetError::Protocol("GTV is not streaming"));
                        }
                        self.viewers[index].active = false;
                        let ack = self.viewers[index].gtv.message(GtvServerOp::StreamStop, &[])?;
                        self.viewers[index].send(&ack)?;
                    }
                    GtvRequest::Command { text } => {
                        if (self.viewers[index].flags & 2) != 0 {
                            if let Some(command) = self.options.command.as_mut() {
                                command(&text);
                            }
                        }
                    }
                    GtvRequest::Hello { .. } => {}
                }
            }
        }
        Ok(())
    }

    /// Close the broadcast, recording, viewers, and listener.
    pub fn close(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        if let Some(recording) = &mut self.recording {
            recording.close();
        }
        for viewer in &mut self.viewers {
            viewer.gtv.close();
            let _ = viewer.stream.shutdown(std::net::Shutdown::Both);
        }
        self.viewers.clear();
        self.listener = None;
    }
}

/// Options for [`GtvConnection::connect`].
pub struct GtvConnectionOptions {
    /// Server host.
    pub host: String,
    /// Server port.
    pub port: u16,
    /// Client identity.
    pub identity: GtvIdentity,
    /// Connect/handshake timeout.
    pub timeout: Duration,
}

/// One TCP GTV connection with an ordered dictionary (`GtvConnection`).
pub struct GtvConnection {
    stream: TcpStream,
    client: GtvClient,
    outbox: std::rc::Rc<std::cell::RefCell<Vec<u8>>>,
    opened: bool,
    closed: bool,
    last_send: Instant,
}

impl GtvConnection {
    /// Connect and complete the hello handshake.
    pub fn connect(options: &GtvConnectionOptions) -> Result<Self, Q2NetError> {
        if options.port == 0 || options.host.is_empty() || options.timeout.is_zero() {
            return Err(Q2NetError::Range("Invalid GTV endpoint or timeout"));
        }
        let address = format!("{}:{}", options.host, options.port)
            .parse::<std::net::SocketAddr>()
            .or_else(|_| {
                use std::net::ToSocketAddrs as _;
                format!("{}:{}", options.host, options.port)
                    .to_socket_addrs()?
                    .next()
                    .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no GTV address"))
            })
            .map_err(|error| Q2NetError::Io(error.to_string()))?;
        let stream =
            TcpStream::connect_timeout(&address, options.timeout).map_err(|error| Q2NetError::Io(error.to_string()))?;
        stream
            .set_nodelay(true)
            .map_err(|error| Q2NetError::Io(error.to_string()))?;
        stream
            .set_read_timeout(Some(options.timeout))
            .map_err(|error| Q2NetError::Io(error.to_string()))?;
        let outbox = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let sink = outbox.clone();
        let client = GtvClient::new(
            move |bytes| {
                let mut outbox = sink.borrow_mut();
                if outbox.len() + bytes.len() > 65536 {
                    return Err(Q2NetError::Protocol("GTV send queue overflow"));
                }
                outbox.extend_from_slice(bytes);
                Ok(())
            },
            options.identity.clone(),
            u32::from(GTF_DEFLATE | GTF_STRINGCMDS),
        )?;
        let mut connection = Self {
            stream,
            client,
            outbox,
            opened: false,
            closed: false,
            last_send: Instant::now(),
        };
        connection.client.start()?;
        connection.flush()?;
        let deadline = Instant::now() + options.timeout;
        while !connection.opened {
            if Instant::now() > deadline {
                return Err(Q2NetError::Io("GTV handshake timed out".to_string()));
            }
            let mut buf = [0u8; 4096];
            match connection.stream.read(&mut buf) {
                Ok(0) => return Err(Q2NetError::Io("GTV peer ended stream".to_string())),
                Ok(count) => {
                    for event in connection.client.receive(&buf[..count])? {
                        if matches!(event, GtvEvent::Hello { .. }) {
                            connection.opened = true;
                        }
                        if matches!(event, GtvEvent::Closed { .. }) {
                            connection.close();
                            return Err(Q2NetError::Io("GTV handshake closed".to_string()));
                        }
                    }
                    connection.flush()?;
                }
                Err(error) => return Err(Q2NetError::Io(error.to_string())),
            }
        }
        connection
            .stream
            .set_nonblocking(true)
            .map_err(|error| Q2NetError::Io(error.to_string()))?;
        Ok(connection)
    }

    /// Whether the handshake completed.
    #[must_use]
    pub fn opened(&self) -> bool {
        self.opened
    }

    /// Request the stream.
    pub fn start(&mut self, max_buffered_packets: u16) -> Result<(), Q2NetError> {
        self.client.request_start(max_buffered_packets)?;
        self.flush()
    }

    /// Stop the stream.
    pub fn stop(&mut self) -> Result<(), Q2NetError> {
        self.client.request_stop()?;
        self.flush()
    }

    /// Forward a console command.
    pub fn command(&mut self, text: &str) -> Result<(), Q2NetError> {
        self.client.command(text)?;
        self.flush()
    }

    /// Send a ping.
    pub fn ping(&mut self) -> Result<(), Q2NetError> {
        self.client.ping()?;
        self.flush()
    }

    /// Read available events, sending a keepalive when idle.
    pub fn poll(&mut self) -> Result<Vec<GtvEvent>, Q2NetError> {
        if self.closed {
            return Ok(Vec::new());
        }
        if self.opened && self.last_send.elapsed() >= Duration::from_secs(60) {
            if self.client.ping().is_err() || self.flush().is_err() {
                self.close();
                return Ok(vec![GtvEvent::Closed {
                    reason: "GTV keepalive failed".to_string(),
                }]);
            }
        }
        let mut buf = [0u8; 4096];
        let mut events = Vec::new();
        loop {
            match self.stream.read(&mut buf) {
                Ok(0) => {
                    self.close();
                    events.push(GtvEvent::Closed {
                        reason: "GTV peer ended stream".to_string(),
                    });
                    break;
                }
                Ok(count) => match self.client.receive(&buf[..count]) {
                    Ok(mut received) => {
                        if self.flush().is_err() {
                            self.close();
                            received.push(GtvEvent::Closed {
                                reason: "GTV transport failed".to_string(),
                            });
                            events.extend(received);
                            break;
                        }
                        let closed = received.iter().any(|event| matches!(event, GtvEvent::Closed { .. }));
                        events.extend(received);
                        if closed || self.client.phase == GtvPhase::Closed {
                            self.close();
                            break;
                        }
                    }
                    Err(error) => {
                        self.close();
                        events.push(GtvEvent::Closed {
                            reason: error.to_string(),
                        });
                        break;
                    }
                },
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => {
                    self.close();
                    events.push(GtvEvent::Closed {
                        reason: "GTV transport failed".to_string(),
                    });
                    break;
                }
            }
        }
        Ok(events)
    }

    /// Flush queued bytes to the socket.
    fn flush(&mut self) -> Result<(), Q2NetError> {
        let bytes = std::mem::take(&mut *self.outbox.borrow_mut());
        if bytes.is_empty() {
            return Ok(());
        }
        // The handshake reads block; steady-state writes take a bounded
        // blocking window like the broadcast side.
        let nonblocking = self.opened;
        if nonblocking {
            let _ = self.stream.set_nonblocking(false);
        }
        let _ = self.stream.set_write_timeout(Some(Duration::from_secs(5)));
        let result = self
            .stream
            .write_all(&bytes)
            .map_err(|error| Q2NetError::Io(error.to_string()));
        if nonblocking {
            let _ = self.stream.set_nonblocking(true);
        }
        result?;
        self.last_send = Instant::now();
        Ok(())
    }

    /// Close the connection.
    pub fn close(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        self.client.close();
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q2::{Q2ProFog, ServerData};
    use crate::q2_net::Q2TempEntity;
    use crate::q2_variants::{fog_bits, KexDamageIndicator, KexHelpPath, KexLocprint, KexPoi};
    use crate::services::downloads::{DownloadError, DownloadSource};

    fn unhex(hex: &str) -> Vec<u8> {
        (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
            .collect()
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn classic_wire() -> Q2Wire {
        Q2Wire::new(ProtocolIdentity::Q2Classic).unwrap()
    }

    fn encode(wire: &mut Q2Wire, event: &Q2ServerEvent) -> String {
        hex(&encode_q2_server_event(wire, event).unwrap())
    }

    #[test]
    fn server_write_classic_byte_exact() {
        let mut wire = classic_wire();
        assert_eq!(encode(&mut wire, &Q2ServerEvent::Nop), "06");
        assert_eq!(encode(&mut wire, &Q2ServerEvent::Disconnect), "07");
        assert_eq!(encode(&mut wire, &Q2ServerEvent::Reconnect), "08");
        assert_eq!(
            encode(
                &mut wire,
                &Q2ServerEvent::Print {
                    level: 2,
                    text: "hi\n".to_string()
                }
            ),
            "0a0268690a00"
        );
        assert_eq!(
            encode(
                &mut wire,
                &Q2ServerEvent::CenterPrint {
                    text: "mid".to_string()
                }
            ),
            "0f6d696400"
        );
        assert_eq!(
            encode(
                &mut wire,
                &Q2ServerEvent::CommandText {
                    text: "precache 7\n".to_string()
                }
            ),
            "0b707265636163686520370a00"
        );
        assert_eq!(
            encode(
                &mut wire,
                &Q2ServerEvent::Layout {
                    text: "xv 1".to_string()
                }
            ),
            "047876203100"
        );
        assert_eq!(
            encode(
                &mut wire,
                &Q2ServerEvent::ConfigString {
                    index: 512,
                    value: "dm1".to_string()
                }
            ),
            "0d0002646d3100"
        );
        assert_eq!(
            encode(&mut wire, &Q2ServerEvent::Inventory { counts: vec![0, 5, -3] }),
            "0500000500fdff"
        );
        assert_eq!(
            encode(
                &mut wire,
                &Q2ServerEvent::Download {
                    percent: 33,
                    bytes: Some(vec![9, 8, 7])
                }
            ),
            "10030021090807"
        );
        assert_eq!(
            encode(
                &mut wire,
                &Q2ServerEvent::Download {
                    percent: 100,
                    bytes: Some(vec![])
                }
            ),
            "10000064"
        );
        assert_eq!(
            encode(
                &mut wire,
                &Q2ServerEvent::MuzzleFlash {
                    entity: 300,
                    flash: 5,
                    monster: false,
                    silenced: true
                }
            ),
            "012c0185"
        );
        assert_eq!(
            encode(
                &mut wire,
                &Q2ServerEvent::MuzzleFlash {
                    entity: 12,
                    flash: 200,
                    monster: true,
                    silenced: false
                }
            ),
            "020c00c8"
        );
    }

    #[test]
    fn server_write_temp_sound_baseline_byte_exact() {
        let mut wire = classic_wire();
        let temp = Q2ServerEvent::TempEntity {
            value: Q2TempEntity {
                temp_type: 3,
                fields: vec![
                    Q2TempField::Vector {
                        name: Q2TempVec::Position1,
                        value: [100.5, -8.25, 0.0],
                    },
                    Q2TempField::Integer {
                        name: Q2TempInt::Color,
                        value: 7,
                    },
                ],
                raw: Vec::new(),
            },
        };
        assert_eq!(encode(&mut wire, &temp), "03032403beff000007");
        // A railtrail (type 3) round-trips through the temp-entity reader.
        let rail = Q2TempEntity {
            temp_type: 3,
            fields: vec![
                Q2TempField::Vector {
                    name: Q2TempVec::Position1,
                    value: [100.5, -8.25, 0.0],
                },
                Q2TempField::Vector {
                    name: Q2TempVec::Position2,
                    value: [1.0, 2.0, 3.0],
                },
            ],
            raw: Vec::new(),
        };
        let rail_event = Q2ServerEvent::TempEntity { value: rail };
        assert_eq!(encode(&mut wire, &rail_event), "03032403beff0000080010001800");
        let bytes = unhex("032403beff0000080010001800");
        let mut reader = MsgReader::new(&bytes);
        let decoded = crate::q2_net::read_temp_entity(&mut reader, false, false, false).unwrap();
        assert_eq!(decoded.temp_type, 3);
        assert_eq!(decoded.fields.len(), 2);
        let plain = Q2ServerEvent::Sound {
            sound: Q2SoundMessage {
                flags: 0,
                index: 9,
                entity: 0,
                channel: 0,
                position: None,
                volume: 1.0,
                attenuation: 1.0,
                delay_seconds: 0.0,
            },
        };
        assert_eq!(encode(&mut wire, &plain), "090009");
        let rich = Q2ServerEvent::Sound {
            sound: Q2SoundMessage {
                flags: 0,
                index: 44,
                entity: 9,
                channel: 3,
                position: Some([1.0, 2.0, 3.0]),
                volume: 0.5,
                attenuation: 2.0,
                delay_seconds: 0.05,
            },
        };
        assert_eq!(encode(&mut wire, &rich), "091f2c7f80324b00080010001800");
        let mut base = EntityState::default();
        base.number = 41;
        base.origin = [10.0, 20.0, 30.0];
        assert_eq!(
            encode(&mut wire, &Q2ServerEvent::Baseline { entity: base }),
            "0e83828001295000a000f000000000000000"
        );
    }

    #[test]
    fn server_write_variants_byte_exact() {
        let mut q2pro = Q2Wire::new(ProtocolIdentity::Q2Q2pro { revision: 1026 }).unwrap();
        q2pro.accept_q2pro_features(1026, 24).unwrap();
        assert_eq!(
            encode(&mut q2pro, &Q2ServerEvent::Setting { index: 3, value: 99 }),
            "180300000063000000"
        );
        assert_eq!(
            encode(
                &mut q2pro,
                &Q2ServerEvent::MuzzleFlash {
                    entity: 12,
                    flash: 0x312,
                    monster: true,
                    silenced: false
                }
            ),
            "020c6012"
        );
        let mut re = Q2Wire::new(ProtocolIdentity::Q2Rerelease).unwrap();
        assert_eq!(
            encode(&mut re, &Q2ServerEvent::Setting { index: 3, value: 99 }),
            "250300000063000000"
        );
        assert_eq!(
            encode(
                &mut re,
                &Q2ServerEvent::Achievement {
                    text: "won".to_string()
                }
            ),
            "21776f6e00"
        );
        assert_eq!(
            encode(
                &mut re,
                &Q2ServerEvent::MuzzleFlash {
                    entity: 5000,
                    flash: 0x234,
                    monster: true,
                    silenced: false
                }
            ),
            "2088133402"
        );
        let mut kex = Q2Wire::new(ProtocolIdentity::Q2Kex).unwrap();
        assert_eq!(encode(&mut kex, &Q2ServerEvent::LevelRestart), "18");
        assert_eq!(encode(&mut kex, &Q2ServerEvent::Seat { seat: 2 }), "1502");
        assert_eq!(
            encode(
                &mut kex,
                &Q2ServerEvent::Damage {
                    indicators: vec![KexDamageIndicator {
                        damage: 40,
                        health: true,
                        armor: false,
                        shield: true,
                        direction: [0.0, 0.0, 1.0],
                    }]
                }
            ),
            "1901a805"
        );
        assert_eq!(
            encode(
                &mut kex,
                &Q2ServerEvent::Poi {
                    value: KexPoi {
                        key: 7,
                        time: 30,
                        pos: [1.0, 2.0, 3.0],
                        image: 9,
                        color: 4,
                        flags: 2
                    }
                }
            ),
            "1e07001e000000803f000000400000404009000402"
        );
        assert_eq!(
            encode(
                &mut kex,
                &Q2ServerEvent::HelpPath {
                    value: KexHelpPath {
                        start: true,
                        pos: [4.0, 5.0, 6.0],
                        dir: [0.0, 1.0, 0.0]
                    }
                }
            ),
            "1f01000080400000a0400000c04020"
        );
        assert_eq!(
            encode(
                &mut kex,
                &Q2ServerEvent::Locprint {
                    value: KexLocprint {
                        flags: 3,
                        base: "WIN".to_string(),
                        args: vec!["a".to_string(), "b".to_string()]
                    }
                }
            ),
            "1a0357494e000261006200"
        );
        assert_eq!(
            encode(
                &mut kex,
                &Q2ServerEvent::Fog {
                    value: FogData {
                        bits: fog_bits::DENSITY | fog_bits::R | fog_bits::HEIGHTFOG_START_DIST,
                        density: 0.5,
                        skyfactor: 9,
                        red: 10,
                        green: 0,
                        blue: 0,
                        time: 0,
                        hf_falloff: 0.0,
                        hf_density: 0.0,
                        hf_start: [0; 3],
                        hf_start_dist: 700,
                        hf_end: [0; 3],
                        hf_end_dist: 0,
                    }
                }
            ),
            "1b83080000003f090abc020000"
        );
    }

    #[test]
    fn fog_writer_byte_exact() {
        let mut writer = MsgWriter::new(256, false);
        write_q2_fog(
            &mut writer,
            &FogData {
                bits: fog_bits::G | fog_bits::TIME,
                density: 0.0,
                skyfactor: 0,
                red: 0,
                green: 44,
                blue: 0,
                time: 300,
                hf_falloff: 0.0,
                hf_density: 0.0,
                hf_start: [0; 3],
                hf_start_dist: 0,
                hf_end: [0; 3],
                hf_end_dist: 0,
            },
        )
        .unwrap();
        assert_eq!(hex(writer.bytes()), "142c2c01");
    }

    #[test]
    fn server_write_rejections() {
        let mut classic = classic_wire();
        let fog = Q2ServerEvent::Fog {
            value: FogData {
                bits: 0,
                density: 0.0,
                skyfactor: 0,
                red: 0,
                green: 0,
                blue: 0,
                time: 0,
                hf_falloff: 0.0,
                hf_density: 0.0,
                hf_start: [0; 3],
                hf_start_dist: 0,
                hf_end: [0; 3],
                hf_end_dist: 0,
            },
        };
        assert!(encode_q2_server_event(&mut classic, &fog).is_err());
        assert!(encode_q2_server_event(&mut classic, &Q2ServerEvent::Setting { index: 0, value: 0 }).is_err());
        assert!(encode_q2_server_event(&mut classic, &Q2ServerEvent::LevelRestart).is_err());
        assert!(encode_q2_server_event(&mut classic, &Q2ServerEvent::Seat { seat: 0 }).is_err());
        assert!(encode_q2_server_event(
            &mut classic,
            &Q2ServerEvent::MuzzleFlash {
                entity: 1,
                flash: 300,
                monster: true,
                silenced: false
            }
        )
        .is_err());
        let wide_sound = Q2ServerEvent::Sound {
            sound: Q2SoundMessage {
                flags: 0,
                index: 300,
                entity: 0,
                channel: 0,
                position: None,
                volume: 1.0,
                attenuation: 1.0,
                delay_seconds: 0.0,
            },
        };
        assert!(encode_q2_server_event(&mut classic, &wide_sound).is_err());
    }

    #[test]
    fn checksum_matches_donor() {
        assert_eq!(block_sequence_checksum(&[9, 0, 0, 0, 3, 1], 7).unwrap(), 194);
        assert_eq!(block_sequence_checksum(&[65u8; 100], 1023).unwrap(), 180);
        assert_eq!(block_sequence_checksum(&[], 0).unwrap(), 10);
    }

    #[test]
    fn client_move_byte_exact_and_reads_back() {
        let mut wire = classic_wire();
        let a = Usercmd {
            forwardmove: 100,
            buttons: 3,
            ..Usercmd::default()
        };
        let b = Usercmd {
            forwardmove: 100,
            buttons: 3,
            angles: [100, 0, 0],
            ..Usercmd::default()
        };
        let c = Usercmd {
            forwardmove: 50,
            impulse: 9,
            msec: 33,
            ..Usercmd::default()
        };
        let bytes = encode_q2_move(&mut wire, 41, 777, &[a, b, c]).unwrap();
        assert_eq!(hex(&bytes), "02a1090300004864000300000164000000c90000320000092100");
        let mut reader = classic_wire();
        let records = read_q2_client_messages(&mut reader, &bytes, 41, 1).unwrap();
        assert_eq!(records.len(), 1);
        let Q2ClientEvent::Move { last_frame, commands } = &records[0].event else {
            panic!("expected move");
        };
        assert_eq!(*last_frame, 777);
        assert_eq!((commands[0].forwardmove, commands[0].buttons), (100, 3));
        assert_eq!(commands[1].angles[0], 100);
        assert_eq!(
            (commands[2].forwardmove, commands[2].impulse, commands[2].msec),
            (50, 9, 33)
        );
        assert_eq!(records[0].raw, bytes);
        // Corrupt the checksum.
        let mut bad = bytes.clone();
        bad[1] ^= 0xff;
        let mut reader = classic_wire();
        assert!(read_q2_client_messages(&mut reader, &bad, 41, 1).is_err());
    }

    #[test]
    fn client_control_byte_exact() {
        assert_eq!(
            hex(&encode_q2_client_control(&Q2ClientEvent::Nop, false).unwrap()),
            "01"
        );
        assert_eq!(
            hex(&encode_q2_client_control(&Q2ClientEvent::Userinfo("\\name\\x".to_string()), false).unwrap()),
            "035c6e616d655c7800"
        );
        assert_eq!(
            hex(&encode_q2_client_control(&Q2ClientEvent::Command("say hi".to_string()), false).unwrap()),
            "0473617920686900"
        );
        assert_eq!(
            hex(&encode_q2_client_control(&Q2ClientEvent::Setting { index: 4, value: -2 }, false).unwrap()),
            "050400feff"
        );
        assert_eq!(
            hex(&encode_q2_client_control(
                &Q2ClientEvent::UserinfoDelta {
                    name: "skin".to_string(),
                    value: "male/grunt".to_string()
                },
                false
            )
            .unwrap()),
            "0c736b696e006d616c652f6772756e7400"
        );
        let mut q2pro = Q2Wire::new(ProtocolIdentity::Q2Q2pro { revision: 1026 }).unwrap();
        let bytes = unhex("050400feff");
        let records = read_q2_client_messages(&mut q2pro, &bytes, 0, 1).unwrap();
        assert_eq!(records[0].event, Q2ClientEvent::Setting { index: 4, value: -2 });
        let mut classic = classic_wire();
        assert!(read_q2_client_messages(&mut classic, &bytes, 0, 1).is_err());
        assert_eq!(
            hex(&encode_q2_client_control(&Q2ClientEvent::Command("use blaster".to_string()), true).unwrap()),
            "040175736520626c617374657200"
        );
    }

    #[test]
    fn kex_move_byte_exact() {
        let mut wire = Q2Wire::new(ProtocolIdentity::Q2Kex).unwrap();
        let k = Usercmd {
            server_frame: 5,
            ..Usercmd::default()
        };
        let bytes = encode_q2_move(&mut wire, 3, 99, &[Usercmd::default(), Usercmd::default(), k]).unwrap();
        assert_eq!(hex(&bytes), "02630000000000000000800500000000");
        let mut reader = Q2Wire::new(ProtocolIdentity::Q2Kex).unwrap();
        let records = read_q2_client_messages(&mut reader, &bytes, 3, 1).unwrap();
        assert_eq!(records.len(), 1);
        // A second seat triple decodes with its seat index.
        let mut two = bytes.clone();
        let mut extra = MsgWriter::new(256, false);
        extra.write_byte(9).unwrap();
        let base = Usercmd::default();
        let mut seat_wire = Q2Wire::new(ProtocolIdentity::Q2Kex).unwrap();
        seat_wire.write_delta_usercmd(&mut extra, &base, &base).unwrap();
        seat_wire.write_delta_usercmd(&mut extra, &base, &base).unwrap();
        seat_wire.write_delta_usercmd(&mut extra, &base, &base).unwrap();
        two.extend_from_slice(extra.bytes());
        let records = read_q2_client_messages(&mut reader, &two, 3, 2).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[1].seat, 1);
        let Q2ClientEvent::Move { commands, .. } = &records[1].event else {
            panic!("expected move");
        };
        assert!(commands.iter().all(|command| command.lightlevel == 9));
    }

    #[test]
    fn batch_moves_byte_exact() {
        let mut q2pro = Q2Wire::new(ProtocolIdentity::Q2Q2pro { revision: 1026 }).unwrap();
        let f1 = Usercmd {
            forwardmove: 40,
            msec: 50,
            ..Usercmd::default()
        };
        let f2 = Usercmd {
            forwardmove: 41,
            msec: 50,
            ..Usercmd::default()
        };
        let frames = vec![
            BatchMoveFrame { cmds: vec![f1] },
            BatchMoveFrame { cmds: vec![f2.clone()] },
        ];
        let bytes = encode_q2_batch_move(&mut q2pro, Some(55), &frames).unwrap();
        assert_eq!(hex(&bytes), "2b370000000021220a3221420a");
        let mut reader = Q2Wire::new(ProtocolIdentity::Q2Q2pro { revision: 1026 }).unwrap();
        let records = read_q2_client_messages(&mut reader, &bytes, 0, 1).unwrap();
        let Q2ClientEvent::BatchMove { batch } = &records[0].event else {
            panic!("expected batch");
        };
        assert_eq!((batch.lastframe, batch.num_dups), (55, 1));
        assert_eq!(batch.frames[1].cmds[0].forwardmove, 41);
        let mut re = Q2Wire::new(ProtocolIdentity::Q2Rerelease).unwrap();
        let f1 = Usercmd {
            forwardmove: 40,
            msec: 50,
            ..Usercmd::default()
        };
        let f2 = Usercmd {
            forwardmove: 41,
            msec: 50,
            ..Usercmd::default()
        };
        let bytes = encode_q2_batch_move(&mut re, None, &[BatchMoveFrame { cmds: vec![f1, f2] }]).unwrap();
        assert_eq!(hex(&bytes), "0a000022220a3211539001");
        let mut classic = classic_wire();
        assert!(read_q2_client_messages(&mut classic, &bytes, 0, 1).is_err());
    }

    #[test]
    fn client_message_rejections() {
        let mut wire = classic_wire();
        let two = [unhex("01"), unhex("01")].concat();
        let records = read_q2_client_messages(&mut wire, &two, 0, 1).unwrap();
        assert_eq!(records.len(), 2);
        // Two moves in one packet.
        let a = Usercmd::default();
        let one = encode_q2_move(&mut classic_wire(), 0, 0, &[a.clone(), a.clone(), a]).unwrap();
        let mut both = one.clone();
        both.extend_from_slice(&one);
        assert!(read_q2_client_messages(&mut classic_wire(), &both, 0, 1).is_err());
        // Unknown opcode.
        assert!(read_q2_client_messages(&mut classic_wire(), &[99], 0, 1).is_err());
        // KEX seat out of range.
        assert!(read_q2_client_messages(&mut Q2Wire::new(ProtocolIdentity::Q2Kex).unwrap(), &[4, 0, 0], 0, 2).is_err());
    }

    #[test]
    fn command_replay_order_matches_donor() {
        let mut replay = Q2CommandReplay::new();
        let mk = |fwd| Usercmd {
            forwardmove: fwd,
            ..Usercmd::default()
        };
        let event = Q2ClientEvent::Move {
            last_frame: 9,
            commands: [mk(1), mk(2), mk(3)],
        };
        let mut seen = Vec::new();
        replay.execute(&event, 3, |cmd| seen.push(cmd.forwardmove)).unwrap();
        assert_eq!(seen, vec![0, 1, 2, 3]);
        assert_eq!(replay.last_frame, 9);
        let mut seen = Vec::new();
        replay.execute(&event, 0, |cmd| seen.push(cmd.forwardmove)).unwrap();
        assert_eq!(seen, vec![3]);
        let batch = Q2ClientEvent::BatchMove {
            batch: BatchMove {
                lastframe: 4,
                num_dups: 1,
                frames: vec![
                    BatchMoveFrame { cmds: vec![mk(7)] },
                    BatchMoveFrame {
                        cmds: vec![mk(8), mk(9)],
                    },
                ],
            },
        };
        let mut seen = Vec::new();
        replay.execute(&batch, 1, |cmd| seen.push(cmd.forwardmove)).unwrap();
        assert_eq!(seen, vec![7, 8, 9]);
    }

    #[test]
    fn rate_window_matches_donor() {
        let mut window = Q2RateWindow::new();
        let mut out = Vec::new();
        for frame in 0..12 {
            window.sent(frame, 400);
            out.push(window.drop_frame(frame + 1, 3000, false));
        }
        out.push(window.take_suppressed() == 5);
        out.push(window.take_suppressed() == 0);
        assert_eq!(
            out,
            vec![false, false, false, false, false, false, false, true, true, true, true, true, true, true]
        );
        assert!(!window.drop_frame(99, 0, true));
    }

    #[test]
    fn mvd_emission_recording_framer_byte_exact() {
        assert_eq!(
            hex(&encode_mvd_emission(&MvdEmission {
                recipient: MvdRecipient::Pvs(300),
                reliable: true,
                bytes: vec![1, 2, 3]
            })
            .unwrap()),
            "0f032c01010203"
        );
        assert_eq!(
            hex(&encode_mvd_emission(&MvdEmission {
                recipient: MvdRecipient::Player(7),
                reliable: false,
                bytes: vec![9]
            })
            .unwrap()),
            "08010709"
        );
        assert_eq!(hex(&mvd_magic()), "4d564432");
        assert_eq!(hex(&frame_mvd_message(&[5, 6]).unwrap()), "02000506");
        let chunks = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let sink = chunks.clone();
        let mut recording = MvdRecording::new(move |bytes| sink.borrow_mut().extend_from_slice(bytes));
        recording.append(&[1, 2]).unwrap();
        recording.append(&[3]).unwrap();
        recording.close();
        assert_eq!(hex(&chunks.borrow()), "4d564432020001020100030000");
        assert_eq!(read_mvd_recording(&chunks.borrow()).unwrap(), vec![vec![1, 2], vec![3]]);
    }

    fn mvd_test_capture(revision: u16, flags: u16) -> MvdCapture {
        let mut player = PlayerState::default();
        player.pmove.pm_type = 0;
        player.pmove.origin = [80, 0, -8];
        player.viewangles = [90.0, 180.0, 0.0];
        player.stats[3] = 25;
        player.gunindex = 4;
        let mut entity = EntityState::default();
        entity.number = 5;
        entity.origin = [1.0, 2.0, 3.0];
        entity.modelindex = 7;
        MvdCapture {
            revision,
            flags,
            servercount: 2,
            gamedir: "baseq2".to_string(),
            dummy: -1,
            config_strings: BTreeMap::from([(0, "q2dm1".to_string()), (30, "8".to_string())]),
            portal_bits: vec![1, 2],
            players: BTreeMap::from([(0, player)]),
            entities: vec![entity],
            messages: Vec::new(),
        }
    }

    #[test]
    fn mvd_encoder_byte_exact() {
        let mut encoder = MvdEncoder::new();
        let first = mvd_test_capture(2010, 0);
        let packets = encoder.capture(&first).unwrap();
        assert_eq!(packets.len(), 1);
        assert_eq!(
            hex(&packets[0]),
            "0425000000da070200000062617365713200ffff00007132646d31001e003800200802010200164250000000f8ff0040008004080000001900ff838a800105070800100018000000000000000000"
        );
        let mut second = mvd_test_capture(2010, 0);
        second.players.get_mut(&0).unwrap().stats[3] = 26;
        second.entities[0].origin = [4.0, 5.0, 6.0];
        second.config_strings.insert(31, "x".to_string());
        let packets = encoder.capture(&second).unwrap();
        assert_eq!(
            hex(&packets[0]),
            "051f00780006020102000040080000001a00ff83828001052000280030000000000000000000"
        );
        let mut extended = mvd_test_capture(2013, 12);
        extended.players.get_mut(&0).unwrap().fog = Q2ProFog {
            color: [1, 2, 3],
            density: 4,
            sky_factor: 5,
            height_density: 6,
            height_falloff: 7,
            height_start_color: [8, 9, 10],
            height_end_color: [11, 12, 13],
            height_start_distance: 14,
            height_end_distance: 15,
        };
        let packets = MvdEncoder::new().capture(&extended).unwrap();
        assert_eq!(
            hex(&packets[0]),
            "0425000000dd070c000200000062617365713200ffff00007132646d31001e0038003e350201020016c202a0000000f0ff004000800400ff010203040005000600070008090a0b0c0d1c001e00081900ff838a800105071000200030000000000000000000"
        );
    }

    struct Passthrough;

    impl MvdVisibility for Passthrough {
        fn entities(&self, entities: &[EntityState], _player: &PlayerState, _portal: &[u8]) -> Vec<EntityState> {
            entities.to_vec()
        }
        fn visible(&self, _leaf: u16, _channel: MvdChannel, _player: &PlayerState, _portal: &[u8]) -> bool {
            true
        }
        fn area_bits(&self, _player: &PlayerState, _portal: &[u8]) -> Vec<u8> {
            Vec::new()
        }
        fn sound_audible(&self, _origin: [f64; 3], _player: &PlayerState, _portal: &[u8]) -> bool {
            true
        }
        fn sound_origin(&self, entity: &EntityState) -> [f64; 3] {
            entity.origin
        }
    }

    #[test]
    fn mvd_playback_round_trip() {
        let mut encoder = MvdEncoder::new();
        let first = mvd_test_capture(2010, 0);
        let packets = encoder.capture(&first).unwrap();
        let mut playback = MvdPlayback::new(Passthrough).unwrap();
        let records = playback.read(&packets[0]).unwrap();
        let opcodes: Vec<u8> = records.iter().map(|record| record.opcode).collect();
        assert_eq!(opcodes, vec![12, 13, 13, 11, 20]);
        let Q2ServerEvent::ServerData { data } = &records[0].event else {
            panic!("expected server-data");
        };
        let Q2ServerData::Vanilla(ServerData {
            servercount, gamedir, ..
        }) = data
        else {
            panic!("expected vanilla data");
        };
        assert_eq!((*servercount, gamedir.as_str()), (2, "baseq2"));
        assert_eq!(playback.selected_player(), 0);
        assert!(playback.select_player(0).is_ok());
        assert!(playback.select_player(7).is_err());
        let Q2ServerEvent::Frame { frame } = &records[4].event else {
            panic!("expected frame");
        };
        assert_eq!((frame.server_frame, frame.entities.len()), (1, 1));
        assert_eq!(frame.entities[0].number, 5);
        let mut second = mvd_test_capture(2010, 0);
        second.players.get_mut(&0).unwrap().stats[3] = 26;
        second.config_strings.insert(31, "x".to_string());
        let packets = encoder.capture(&second).unwrap();
        let records = playback.read(&packets[0]).unwrap();
        let opcodes: Vec<u8> = records.iter().map(|record| record.opcode).collect();
        assert_eq!(opcodes, vec![13, 20]);
        let Q2ServerEvent::Frame { frame } = &records[1].event else {
            panic!("expected frame");
        };
        assert_eq!(frame.server_frame, 2);
    }

    #[test]
    fn download_sender_blocks() {
        struct Memory {
            bytes: Vec<u8>,
            closed: bool,
        }
        impl DownloadSource for Memory {
            fn byte_length(&self) -> u64 {
                self.bytes.len() as u64
            }
            fn read(&mut self, offset: u64, max_bytes: usize) -> Result<Vec<u8>, DownloadError> {
                let start = offset as usize;
                let end = (start + max_bytes).min(self.bytes.len());
                Ok(self.bytes[start..end].to_vec())
            }
            fn close(&mut self) {
                self.closed = true;
            }
        }
        let source = Memory {
            bytes: vec![7u8; 2500],
            closed: false,
        };
        let mut sender = Q2DownloadSender::new(Box::new(source), 0, 1024).unwrap();
        let mut percents = Vec::new();
        let mut total = 0;
        while let Some(Q2ServerEvent::Download { percent, bytes }) = sender.next().unwrap() {
            percents.push(percent);
            total += bytes.unwrap().len();
        }
        assert_eq!((percents.as_slice(), total), (&[40, 81, 100][..], 2500));
        assert!(Q2DownloadSender::new(
            Box::new(Memory {
                bytes: vec![],
                closed: false
            }),
            1,
            1024
        )
        .is_err());
    }

    #[test]
    fn gtv_request_and_hello_byte_exact() {
        let mut stream = GtvServerStream::new();
        assert_eq!(hex(&stream.hello(3).unwrap()), "05000003000000");
        let bytes = unhex("0004ed0300000000000000750070007600");
        let request = read_gtv_request(&bytes).unwrap();
        let GtvRequest::Hello { hello } = request else {
            panic!("expected hello");
        };
        assert_eq!(
            (
                hello.username.as_str(),
                hello.password.as_str(),
                hello.version.as_str(),
                hello.flags
            ),
            ("u", "p", "v", 3)
        );
    }

    #[test]
    fn gtv_client_server_loopback() {
        let outbox = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let sink = outbox.clone();
        let mut client = GtvClient::new(
            move |bytes| {
                sink.borrow_mut().extend_from_slice(bytes);
                Ok(())
            },
            GtvIdentity {
                username: "u".to_string(),
                password: "p".to_string(),
                version: "v".to_string(),
            },
            3,
        )
        .unwrap();
        client.start().unwrap();
        assert_eq!(hex(&outbox.borrow()), "4d564432");
        outbox.borrow_mut().clear();
        // Server magic echo triggers the client hello.
        assert!(client.receive(&mvd_magic()).unwrap().is_empty());
        assert_eq!(hex(&outbox.borrow()), "11000004ed0300000000000000750070007600");
        outbox.borrow_mut().clear();
        let mut server = GtvServerStream::new();
        let greeting = server.hello(1).unwrap();
        let events = client.receive(&greeting).unwrap();
        assert_eq!(events, vec![GtvEvent::Hello { flags: 1 }]);
        client.request_start(10).unwrap();
        assert_eq!(hex(&outbox.borrow()), "0300020a00");
        outbox.borrow_mut().clear();
        let ack = server.message(GtvServerOp::StreamStart, &[]).unwrap();
        assert_eq!(client.receive(&ack).unwrap(), vec![GtvEvent::Started]);
        let data = server.message(GtvServerOp::StreamData, &[9, 9, 9]).unwrap();
        assert_eq!(
            client.receive(&data).unwrap(),
            vec![GtvEvent::Data { bytes: vec![9, 9, 9] }]
        );
        let suspended = server.message(GtvServerOp::StreamData, &[]).unwrap();
        assert_eq!(client.receive(&suspended).unwrap(), vec![GtvEvent::Suspended]);
        let resumed = server.message(GtvServerOp::StreamData, &[7]).unwrap();
        assert_eq!(
            client.receive(&resumed).unwrap(),
            vec![GtvEvent::Resumed, GtvEvent::Data { bytes: vec![7] }]
        );
        client.request_stop().unwrap();
        let stop = server.message(GtvServerOp::StreamStop, &[]).unwrap();
        assert_eq!(client.receive(&stop).unwrap(), vec![GtvEvent::Stopped]);
        client.ping().unwrap();
        let pong = server.message(GtvServerOp::Pong, &[]).unwrap();
        assert_eq!(client.receive(&pong).unwrap(), vec![GtvEvent::Pong]);
    }

    #[test]
    fn gtv_broadcast_loopback_over_tcp() {
        let recorded = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let sink = recorded.clone();
        let mut broadcast = MvdBroadcast::new(MvdBroadcastOptions {
            authorize: Box::new(|_| true),
            command: None,
            record: Some(Box::new(move |bytes| sink.borrow_mut().extend_from_slice(bytes))),
            max_viewers: 4,
        })
        .unwrap();
        let port = broadcast.listen("127.0.0.1", 0).unwrap();
        broadcast.observe(&mvd_test_capture(2010, 0)).unwrap();
        assert!(hex(&recorded.borrow()).starts_with("4d564432"));

        let mut socket = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
        socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        socket.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
        let framed = |op: u8, body: &[u8]| {
            let mut message = vec![op];
            message.extend_from_slice(body);
            frame_mvd_message(&message).unwrap()
        };
        let read_framed = |socket: &mut TcpStream| {
            let mut len = [0u8; 2];
            socket.read_exact(&mut len).unwrap();
            let len = usize::from(u16::from_le_bytes(len));
            let mut payload = vec![0u8; len];
            socket.read_exact(&mut payload).unwrap();
            payload
        };
        socket.write_all(&mvd_magic()).unwrap();
        for _ in 0..4 {
            broadcast.poll().unwrap();
        }
        let mut echo = [0u8; 4];
        socket.read_exact(&mut echo).unwrap();
        assert_eq!(echo, mvd_magic());
        // Hello without compression so the stream stays plain.
        let mut hello = GTV_PROTOCOL_VERSION.to_le_bytes().to_vec();
        hello.extend_from_slice(&0u32.to_le_bytes());
        hello.extend_from_slice(&0u32.to_le_bytes());
        hello.extend_from_slice(b"me\0\0v\0");
        socket.write_all(&framed(GtvClientOp::Hello as u8, &hello)).unwrap();
        for _ in 0..4 {
            broadcast.poll().unwrap();
        }
        let greeting = read_framed(&mut socket);
        assert_eq!(greeting, vec![GtvServerOp::Hello as u8, 0, 0, 0, 0]);
        socket
            .write_all(&framed(GtvClientOp::StreamStart as u8, &10u16.to_le_bytes()))
            .unwrap();
        for _ in 0..4 {
            broadcast.poll().unwrap();
        }
        assert_eq!(read_framed(&mut socket), vec![GtvServerOp::StreamStart as u8]);
        let data = read_framed(&mut socket);
        assert_eq!(data[0], GtvServerOp::StreamData as u8);
        let mut playback = MvdPlayback::new(Passthrough).unwrap();
        let records = playback.read(&data[1..]).unwrap();
        let Q2ServerEvent::ServerData { data } = &records[0].event else {
            panic!("expected gamestate server-data");
        };
        assert!(matches!(data, Q2ServerData::Vanilla(ServerData { servercount: 2, .. })));
        socket.write_all(&framed(GtvClientOp::StreamStop as u8, &[])).unwrap();
        for _ in 0..4 {
            broadcast.poll().unwrap();
        }
        assert_eq!(read_framed(&mut socket), vec![GtvServerOp::StreamStop as u8]);
        broadcast.close();
    }
}
