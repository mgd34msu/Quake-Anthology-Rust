//! Quake II extended protocol codec variants.
//!
//! Donor provenance: `src/network/q2/codecs/{codec,r1q2,q2pro,q2pro-fields,
//! clc_batch_move,q2repro,kexdemo,kex-write,kex-usercmd,zpacket,mvd}.ts`,
//! `src/network/q2/fog.ts`, and `src/network/q2/mvd-profile.ts`. Shared
//! state shapes ([`EntityState`], [`PlayerState`]) live in [`crate::q2`].
//!
//! Each codec mirrors its donor `ProtocolCodec`: server data, delta
//! entities, player states, frames, user commands, and batch moves.
//! Unchanged fields inherit from the delta base on every read, matching
//! the donor's copy-then-overlay readers.

use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Write};

use flate2::read::{DeflateDecoder, ZlibDecoder};
use flate2::write::DeflateEncoder;
use flate2::Compression;
use thiserror::Error;

use crate::msg::{MsgError, MsgReader, MsgWriter};
use crate::protocol::q2 as protocol;
use crate::q2::{angle_to_short, read_dir, scaled_trunc, short_to_angle};
use crate::q2::{
    EntityState, FrameHeader, FrameWrite, PlayerState, Q2CodecError, Q2ProFog, Usercmd, MAX_EDICTS, MAX_STATS,
    MAX_STATS_STORAGE, RF_BEAM,
};

/// Error for extended Q2 codec variants.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum VariantError {
    /// Underlying message failure.
    #[error("{0}")]
    Msg(#[from] MsgError),
    /// Underlying classic codec failure.
    #[error("{0}")]
    Codec(#[from] Q2CodecError),
    /// Entity number outside the Q2Pro source range.
    #[error("entity number {0} outside Q2Pro range 1..=8191")]
    EntityRange(u16),
    /// Q2Pro coordinate outside the signed 23-bit range.
    #[error("Q2PRO coordinate {0} exceeds signed 23-bit range")]
    CoordinateRange(i32),
    /// Q2Pro entity fields need negotiated game extensions.
    #[error("Q2PRO fields need negotiated game extensions")]
    ExtensionsRequired,
    /// Q2Pro player fog needs revision 1026.
    #[error("Q2PRO player fog requires revision 1026 (have {0})")]
    FogRevision(u16),
    /// KEX gun frame outside nine bits.
    #[error("KEX gun frame {0} exceeds nine bits")]
    GunRange(i32),
    /// Rerelease non-batched movement met the reserved `CM_UP` bit.
    #[error("q2repro non-batched movement has reserved CM_UP bit")]
    ReservedUpMove,
    /// Rerelease non-batched move cannot encode upmove changes.
    #[error("q2repro non-batched clc_move cannot encode upmove changes")]
    NonBatchableUpMove,
    /// Batch move exceeds wire bounds.
    #[error("command batch exceeds wire bounds")]
    BatchBounds,
    /// Batch move duplicate count out of range.
    #[error("batch move num_dups {0} out of range")]
    BatchNumDups(u8),
    /// Rerelease batch decoder met the rejected `CM_UP` bit.
    #[error("q2repro batch move rejects the CM_UP bit")]
    BatchCmUp,
    /// Malformed Q2Pro variable-length integer.
    #[error("invalid Q2PRO variable uint64")]
    BadVar64,
    /// Unterminated Q2Pro variable-length integer.
    #[error("unterminated Q2PRO variable uint64")]
    UnterminatedVar64,
    /// KEX split-player count out of range.
    #[error("invalid KEX split count {0}")]
    BadSplitCount(i16),
    /// KEX split-player list too long to write.
    #[error("invalid KEX split player count {0}")]
    TooManySplitPlayers(usize),
    /// KEX localized print exceeds the argument limit.
    #[error("kex locprint has {0} args (max 8)")]
    TooManyLocArgs(u8),
    /// MVD revision outside the supported set.
    #[error("unsupported MVD revision {0}")]
    UnsupportedMvdRevision(u16),
    /// MVD extended-v2 needs extended limits.
    #[error("MVD extended-v2 requires extended limits")]
    MvdV2NeedsExtended,
    /// Bytes did not start with an MVD gamestate header.
    #[error("MVD gamestate header expected")]
    BadMvdHeader,
    /// MVD configstring index out of range.
    #[error("invalid MVD configstring index {0}")]
    BadConfigstringIndex(u16),
    /// MVD player limits failed validation.
    #[error("invalid MVD player limits")]
    BadMvdLimits,
    /// Compression round-trip failure.
    #[error("zlib failure: {0}")]
    Zlib(String),
    /// Inflated zpacket length differs from its header.
    #[error("zpacket inflated length {found} differs from header {expected}")]
    ZpacketLength {
        /// Header length.
        expected: usize,
        /// Actual length.
        found: usize,
    },
}

/// Maximum batched-move frames (`MAX_CLC_BATCH_MOVE_FRAMES`).
pub const MAX_BATCH_MOVE_FRAMES: usize = 4;
/// Maximum commands per batched-move frame (`MAX_CLC_BATCH_MOVE_CMDS`).
pub const MAX_BATCH_MOVE_CMDS: usize = 32;

/// LSB-first bit reader over a message (`BitReader`).
pub struct BatchBitReader<'a, 'b> {
    reader: &'b mut MsgReader<'a>,
    buf: u32,
    left: u32,
}

impl<'a, 'b> BatchBitReader<'a, 'b> {
    /// Borrow a message reader for bit reads.
    pub fn new(reader: &'b mut MsgReader<'a>) -> Self {
        Self {
            reader,
            buf: 0,
            left: 0,
        }
    }

    fn fill(&mut self, bits: u32) -> Result<(), MsgError> {
        while self.left < bits {
            let byte = self.reader.byte()?;
            self.buf |= u32::from(byte) << self.left;
            self.left += 8;
        }
        Ok(())
    }

    /// Read an unsigned value of `bits` bits.
    pub fn read_unsigned(&mut self, bits: u32) -> Result<u32, MsgError> {
        self.fill(bits)?;
        let value = self.buf & ((1 << bits) - 1);
        self.buf >>= bits;
        self.left -= bits;
        Ok(value)
    }

    /// Read a signed value of `bits` bits.
    pub fn read_signed(&mut self, bits: u32) -> Result<i32, MsgError> {
        let value = self.read_unsigned(bits)?;
        let sign = 1 << (bits - 1);
        Ok((value ^ sign) as i32 - sign as i32)
    }
}

/// LSB-first bit writer over a message (`BitWriter`).
pub struct BatchBitWriter<'a> {
    writer: &'a mut MsgWriter,
    buf: u32,
    left: u32,
}

impl<'a> BatchBitWriter<'a> {
    /// Borrow a message writer for bit writes.
    pub fn new(writer: &'a mut MsgWriter) -> Self {
        Self {
            writer,
            buf: 0,
            left: 0,
        }
    }

    /// Write the low `bits` bits of `value`.
    pub fn write_unsigned(&mut self, value: u32, bits: u32) -> Result<(), MsgError> {
        self.buf |= (value & ((1 << bits) - 1)) << self.left;
        self.left += bits;
        while self.left >= 8 {
            self.writer.write_byte((self.buf & 0xff) as u8)?;
            self.buf >>= 8;
            self.left -= 8;
        }
        Ok(())
    }

    /// Write a signed value in `bits` bits.
    pub fn write_signed(&mut self, value: i32, bits: u32) -> Result<(), MsgError> {
        self.write_unsigned(value as u32, bits)
    }

    /// Flush a partial trailing byte.
    pub fn flush(&mut self) -> Result<(), MsgError> {
        if self.left > 0 {
            self.writer.write_byte((self.buf & 0xff) as u8)?;
            self.buf = 0;
            self.left = 0;
        }
        Ok(())
    }
}

/// One batched-move frame (`ClcBatchMoveFrameT`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BatchMoveFrame {
    /// Commands in this frame.
    pub cmds: Vec<Usercmd>,
}

/// Parsed batch move (`ClcBatchMoveT`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchMove {
    /// Delta base frame (`-1` for none).
    pub lastframe: i32,
    /// Duplicate frame count.
    pub num_dups: usize,
    /// Decoded frames.
    pub frames: Vec<BatchMoveFrame>,
}

/// Read one delta-or-absolute batch-move angle component.
pub fn read_batch_move_angle(reader: &mut BatchBitReader<'_, '_>, prev_angle: i16) -> Result<i16, MsgError> {
    if reader.read_unsigned(1)? != 0 {
        Ok(prev_angle.wrapping_add(reader.read_signed(8)? as i16))
    } else {
        Ok(reader.read_signed(16)? as i16)
    }
}

/// Read `num_dups + 1` batched-move frames (`readBatchMoveFrames`).
pub fn read_batch_move_frames(
    reader: &mut BatchBitReader<'_, '_>,
    num_dups: usize,
    mut decode: impl for<'r, 's> FnMut(&mut BatchBitReader<'r, 's>, Option<&Usercmd>) -> Result<Usercmd, VariantError>,
) -> Result<Vec<BatchMoveFrame>, VariantError> {
    let mut frames = Vec::with_capacity(num_dups + 1);
    let mut prev: Option<Usercmd> = None;
    for _ in 0..=num_dups {
        let num_cmds = reader.read_unsigned(5)? as usize;
        let mut cmds = Vec::with_capacity(num_cmds);
        for _ in 0..num_cmds {
            let cmd = decode(&mut *reader, prev.as_ref())?;
            cmds.push(cmd.clone());
            prev = Some(cmd);
        }
        frames.push(BatchMoveFrame { cmds });
    }
    Ok(frames)
}

/// Write batched-move frames (`writeBatchMoveFrames`).
pub fn write_batch_move_frames(
    writer: &mut BatchBitWriter<'_>,
    frames: &[BatchMoveFrame],
    mut encode: impl for<'w> FnMut(&mut BatchBitWriter<'w>, &Usercmd, Option<&Usercmd>) -> Result<(), VariantError>,
) -> Result<(), VariantError> {
    let mut prev: Option<&Usercmd> = None;
    for frame in frames {
        writer.write_unsigned(frame.cmds.len() as u32, 5)?;
        for cmd in &frame.cmds {
            encode(&mut *writer, cmd, prev)?;
            prev = Some(cmd);
        }
    }
    writer.flush()?;
    Ok(())
}

/// Seed a batch command from its predecessor (`seedFromPrev`).
#[must_use]
pub fn seed_from_prev(prev: Option<&Usercmd>) -> Usercmd {
    prev.cloned().unwrap_or_default()
}

/// Read a Q2Pro/userinfo delta pair (`readUserinfoDelta`).
pub fn read_userinfo_delta(reader: &mut MsgReader<'_>) -> Result<(String, String), MsgError> {
    let name = reader.string(2047);
    let value = reader.string(2047);
    Ok((name, value))
}

/// Read a client setting pair (`readClientSetting`).
pub fn read_client_setting(reader: &mut MsgReader<'_>) -> Result<(i16, i16), MsgError> {
    let index = reader.short()?;
    let value = reader.short()?;
    Ok((index, value))
}

/// Wide entity header bits plus number (`readEntityBitsWide`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WideEntityBits {
    /// Entity number.
    pub number: u16,
    /// Low 32 header bits.
    pub lo: u32,
    /// Fifth header byte.
    pub hi: u8,
}

/// Write wide entity bits plus number (`writeEntityBitsWide`).
pub fn write_entity_bits_wide(writer: &mut MsgWriter, lo: u32, hi: u8, number: u16) -> Result<(), MsgError> {
    let mut lo = lo;
    if number >= 256 {
        lo |= protocol::U_NUMBER16;
    }
    if hi != 0 {
        lo |= protocol::U_MOREBITS4 | protocol::U_MOREBITS3 | protocol::U_MOREBITS2 | protocol::U_MOREBITS1;
    } else if (lo & 0xff00_0000) != 0 {
        lo |= protocol::U_MOREBITS3 | protocol::U_MOREBITS2 | protocol::U_MOREBITS1;
    } else if (lo & 0x00ff_0000) != 0 {
        lo |= protocol::U_MOREBITS2 | protocol::U_MOREBITS1;
    } else if (lo & 0x0000_ff00) != 0 {
        lo |= protocol::U_MOREBITS1;
    }
    writer.write_byte((lo & 0xff) as u8)?;
    if (lo & protocol::U_MOREBITS1) != 0 {
        writer.write_byte(((lo >> 8) & 0xff) as u8)?;
    }
    if (lo & protocol::U_MOREBITS2) != 0 {
        writer.write_byte(((lo >> 16) & 0xff) as u8)?;
    }
    if (lo & protocol::U_MOREBITS3) != 0 {
        writer.write_byte(((lo >> 24) & 0xff) as u8)?;
    }
    if (lo & protocol::U_MOREBITS4) != 0 {
        writer.write_byte(hi)?;
    }
    if (lo & protocol::U_NUMBER16) != 0 {
        writer.write_short(number as i16)?;
    } else {
        writer.write_byte(number as u8)?;
    }
    Ok(())
}

/// Read wide entity bits plus number (`readEntityBitsWide`).
pub fn read_entity_bits_wide(reader: &mut MsgReader<'_>) -> Result<WideEntityBits, MsgError> {
    let mut lo = u32::from(reader.byte()?);
    if (lo & protocol::U_MOREBITS1) != 0 {
        lo |= u32::from(reader.byte()?) << 8;
    }
    if (lo & protocol::U_MOREBITS2) != 0 {
        lo |= u32::from(reader.byte()?) << 16;
    }
    if (lo & protocol::U_MOREBITS3) != 0 {
        lo |= u32::from(reader.byte()?) << 24;
    }
    let mut hi = 0;
    if (lo & protocol::U_MOREBITS4) != 0 {
        hi = reader.byte()?;
    }
    let number = if (lo & protocol::U_NUMBER16) != 0 {
        reader.short()? as u16
    } else {
        u16::from(reader.byte()?)
    };
    Ok(WideEntityBits { number, lo, hi })
}

// ---------------------------------------------------------------------------
// R1Q2
// ---------------------------------------------------------------------------

/// R1Q2 server data (`ServerDataParamsT` / `ServerDataReadResultT`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct R1q2ServerData {
    /// Server count.
    pub servercount: i32,
    /// Attract loop.
    pub attractloop: bool,
    /// Game directory.
    pub gamedir: String,
    /// Client number.
    pub clientnum: i16,
    /// Level name.
    pub levelname: String,
    /// R1Q2 minor version.
    pub version: u16,
    /// Strafejump hack negotiated.
    pub strafejump_hack: bool,
}

/// Encoded R1Q2 player-state delta.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct R1q2PlayerDelta {
    /// Main flags.
    pub flags: u16,
    /// Extra flags.
    pub extraflags: u8,
    /// Changed stat bits.
    pub statbits: u32,
}

/// R1Q2 protocol codec (`createR1Q2Codec`).
#[derive(Debug, Clone)]
pub struct R1q2Codec {
    minor_version: u32,
    frame_extrabits: u8,
    frame_extraflags: u8,
}

impl R1q2Codec {
    /// Build a codec for an R1Q2 minor version.
    #[must_use]
    pub fn new(minor_version: u32) -> Self {
        Self {
            minor_version,
            frame_extrabits: 0,
            frame_extraflags: 0,
        }
    }

    /// Minor version this codec encodes for.
    #[must_use]
    pub fn minor_version(&self) -> u32 {
        self.minor_version
    }

    /// Whether solid values travel as longs (`PROTOCOL_VERSION_R1Q2_LONG_SOLID`).
    #[must_use]
    pub fn long_solid(&self) -> bool {
        self.minor_version >= protocol::PROTOCOL_VERSION_R1Q2_LONG_SOLID
    }

    /// Whether user commands use compressed movements (`PROTOCOL_VERSION_R1Q2_UCMD`).
    #[must_use]
    pub fn compressed_movements(&self) -> bool {
        self.minor_version >= protocol::PROTOCOL_VERSION_R1Q2_UCMD
    }

    /// Note frame opcode extra bits before reading a frame header.
    pub fn set_frame_extrabits(&mut self, extrabits: u8) {
        self.frame_extrabits = extrabits;
    }

    /// Write server data (`writeServerData`).
    pub fn write_server_data(&self, writer: &mut MsgWriter, params: &R1q2ServerData) -> Result<(), MsgError> {
        writer.write_byte(protocol::Svc::Serverdata as u8)?;
        writer.write_long(protocol::PROTOCOL_VERSION_R1Q2 as i32)?;
        writer.write_long(params.servercount)?;
        writer.write_byte(u8::from(params.attractloop))?;
        writer.write_string(&params.gamedir)?;
        writer.write_short(params.clientnum)?;
        writer.write_string(&params.levelname)?;
        writer.write_byte(0)?;
        writer.write_short(params.version as i16)?;
        writer.write_byte(0)?;
        writer.write_byte(u8::from(params.strafejump_hack))
    }

    /// Read server data (`readServerData`); opcode and version are consumed by the caller.
    pub fn read_server_data(reader: &mut MsgReader<'_>) -> Result<R1q2ServerData, MsgError> {
        let servercount = reader.long()?;
        let attractloop = reader.byte()? != 0;
        let gamedir = reader.string(2047);
        let clientnum = reader.short()?;
        let levelname = reader.string(2047);
        reader.byte()?;
        let version = reader.short()? as u16;
        reader.byte()?;
        let strafejump_hack = reader.byte()? != 0;
        Ok(R1q2ServerData {
            servercount,
            attractloop,
            gamedir,
            clientnum,
            levelname,
            version,
            strafejump_hack,
        })
    }

    /// Write a delta entity; `Ok(false)` writes nothing when unchanged and not forced.
    pub fn write_delta_entity(
        &self,
        writer: &mut MsgWriter,
        from: &EntityState,
        to: &EntityState,
        force: bool,
        newentity: bool,
    ) -> Result<bool, Q2CodecError> {
        if to.number == 0 {
            return Err(Q2CodecError::UnsetNumber);
        }
        if to.number >= MAX_EDICTS {
            return Err(Q2CodecError::NumberTooLarge(to.number));
        }
        let mut bits = crate::q2::entity_bits(from, to, newentity);
        // The R1Q2 skin test is unsigned while the classic one is signed.
        if to.skinnum != from.skinnum {
            bits &= !(protocol::U_SKIN8 | protocol::U_SKIN16);
            let skin = to.skinnum as u32;
            if skin < 256 {
                bits |= protocol::U_SKIN8;
            } else if skin < 0x8000 {
                bits |= protocol::U_SKIN16;
            } else {
                bits |= protocol::U_SKIN8 | protocol::U_SKIN16;
            }
        }
        if bits == 0 && !force {
            return Ok(false);
        }
        if (bits & 0xff00_0000) != 0 {
            bits |= protocol::U_MOREBITS3 | protocol::U_MOREBITS2 | protocol::U_MOREBITS1;
        } else if (bits & 0x00ff_0000) != 0 {
            bits |= protocol::U_MOREBITS2 | protocol::U_MOREBITS1;
        } else if (bits & 0x0000_ff00) != 0 {
            bits |= protocol::U_MOREBITS1;
        }
        writer.write_byte((bits & 255) as u8)?;
        if (bits & 0xff00_0000) != 0 {
            writer.write_byte(((bits >> 8) & 255) as u8)?;
            writer.write_byte(((bits >> 16) & 255) as u8)?;
            writer.write_byte(((bits >> 24) & 255) as u8)?;
        } else if (bits & 0x00ff_0000) != 0 {
            writer.write_byte(((bits >> 8) & 255) as u8)?;
            writer.write_byte(((bits >> 16) & 255) as u8)?;
        } else if (bits & 0x0000_ff00) != 0 {
            writer.write_byte(((bits >> 8) & 255) as u8)?;
        }
        if (bits & protocol::U_NUMBER16) != 0 {
            writer.write_short(to.number as i16)?;
        } else {
            writer.write_byte(to.number as u8)?;
        }
        if (bits & protocol::U_MODEL) != 0 {
            writer.write_byte(to.modelindex as u8)?;
        }
        if (bits & protocol::U_MODEL2) != 0 {
            writer.write_byte(to.modelindex2 as u8)?;
        }
        if (bits & protocol::U_MODEL3) != 0 {
            writer.write_byte(to.modelindex3 as u8)?;
        }
        if (bits & protocol::U_MODEL4) != 0 {
            writer.write_byte(to.modelindex4 as u8)?;
        }
        if (bits & protocol::U_FRAME8) != 0 {
            writer.write_byte(to.frame as u8)?;
        }
        if (bits & protocol::U_FRAME16) != 0 {
            writer.write_short(to.frame as i16)?;
        }
        if (bits & protocol::U_SKIN8) != 0 && (bits & protocol::U_SKIN16) != 0 {
            writer.write_long(to.skinnum)?;
        } else if (bits & protocol::U_SKIN8) != 0 {
            writer.write_byte(to.skinnum as u8)?;
        } else if (bits & protocol::U_SKIN16) != 0 {
            writer.write_short(to.skinnum as i16)?;
        }
        if (bits & (protocol::U_EFFECTS8 | protocol::U_EFFECTS16)) == (protocol::U_EFFECTS8 | protocol::U_EFFECTS16) {
            writer.write_long(to.effects)?;
        } else if (bits & protocol::U_EFFECTS8) != 0 {
            writer.write_byte(to.effects as u8)?;
        } else if (bits & protocol::U_EFFECTS16) != 0 {
            writer.write_short(to.effects as i16)?;
        }
        if (bits & (protocol::U_RENDERFX8 | protocol::U_RENDERFX16)) == (protocol::U_RENDERFX8 | protocol::U_RENDERFX16)
        {
            writer.write_long(to.renderfx)?;
        } else if (bits & protocol::U_RENDERFX8) != 0 {
            writer.write_byte(to.renderfx as u8)?;
        } else if (bits & protocol::U_RENDERFX16) != 0 {
            writer.write_short(to.renderfx as i16)?;
        }
        if (bits & protocol::U_ORIGIN1) != 0 {
            writer.write_q2_coord(to.origin[0])?;
        }
        if (bits & protocol::U_ORIGIN2) != 0 {
            writer.write_q2_coord(to.origin[1])?;
        }
        if (bits & protocol::U_ORIGIN3) != 0 {
            writer.write_q2_coord(to.origin[2])?;
        }
        if (bits & protocol::U_ANGLE1) != 0 {
            writer.write_q2_angle(to.angles[0])?;
        }
        if (bits & protocol::U_ANGLE2) != 0 {
            writer.write_q2_angle(to.angles[1])?;
        }
        if (bits & protocol::U_ANGLE3) != 0 {
            writer.write_q2_angle(to.angles[2])?;
        }
        if (bits & protocol::U_OLDORIGIN) != 0 {
            writer.write_q2_pos(to.old_origin)?;
        }
        if (bits & protocol::U_SOUND) != 0 {
            writer.write_byte(to.sound as u8)?;
        }
        if (bits & protocol::U_EVENT) != 0 {
            writer.write_byte(to.event)?;
        }
        if (bits & protocol::U_SOLID) != 0 {
            if self.long_solid() {
                writer.write_long(to.solid as i32)?;
            } else {
                writer.write_short(to.solid as i16)?;
            }
        }
        Ok(true)
    }

    /// Read a delta entity body (`readDeltaEntity`).
    pub fn read_delta_entity(
        &self,
        reader: &mut MsgReader<'_>,
        from: &EntityState,
        number: u16,
        bits: u32,
    ) -> Result<EntityState, MsgError> {
        let mut to = from.clone();
        to.old_origin = from.origin;
        to.number = number;
        if (bits & protocol::U_MODEL) != 0 {
            to.modelindex = u16::from(reader.byte()?);
        }
        if (bits & protocol::U_MODEL2) != 0 {
            to.modelindex2 = u16::from(reader.byte()?);
        }
        if (bits & protocol::U_MODEL3) != 0 {
            to.modelindex3 = u16::from(reader.byte()?);
        }
        if (bits & protocol::U_MODEL4) != 0 {
            to.modelindex4 = u16::from(reader.byte()?);
        }
        if (bits & protocol::U_FRAME8) != 0 {
            to.frame = i32::from(reader.byte()?);
        }
        if (bits & protocol::U_FRAME16) != 0 {
            to.frame = i32::from(reader.word()?);
        }
        if (bits & protocol::U_SKIN8) != 0 && (bits & protocol::U_SKIN16) != 0 {
            to.skinnum = reader.long()?;
        } else if (bits & protocol::U_SKIN8) != 0 {
            to.skinnum = i32::from(reader.byte()?);
        } else if (bits & protocol::U_SKIN16) != 0 {
            to.skinnum = i32::from(reader.word()?);
        }
        if (bits & (protocol::U_EFFECTS8 | protocol::U_EFFECTS16)) == (protocol::U_EFFECTS8 | protocol::U_EFFECTS16) {
            to.effects = reader.long()?;
        } else if (bits & protocol::U_EFFECTS8) != 0 {
            to.effects = i32::from(reader.byte()?);
        } else if (bits & protocol::U_EFFECTS16) != 0 {
            to.effects = i32::from(reader.word()?);
        }
        if (bits & (protocol::U_RENDERFX8 | protocol::U_RENDERFX16)) == (protocol::U_RENDERFX8 | protocol::U_RENDERFX16)
        {
            to.renderfx = reader.long()?;
        } else if (bits & protocol::U_RENDERFX8) != 0 {
            to.renderfx = i32::from(reader.byte()?);
        } else if (bits & protocol::U_RENDERFX16) != 0 {
            to.renderfx = i32::from(reader.word()?);
        }
        if (bits & protocol::U_ORIGIN1) != 0 {
            to.origin[0] = reader.coord()?;
        }
        if (bits & protocol::U_ORIGIN2) != 0 {
            to.origin[1] = reader.coord()?;
        }
        if (bits & protocol::U_ORIGIN3) != 0 {
            to.origin[2] = reader.coord()?;
        }
        if (bits & protocol::U_ANGLE1) != 0 {
            to.angles[0] = reader.q2_angle()?;
        }
        if (bits & protocol::U_ANGLE2) != 0 {
            to.angles[1] = reader.q2_angle()?;
        }
        if (bits & protocol::U_ANGLE3) != 0 {
            to.angles[2] = reader.q2_angle()?;
        }
        if (bits & protocol::U_OLDORIGIN) != 0 {
            to.old_origin = reader.q2_pos()?;
        }
        if (bits & protocol::U_SOUND) != 0 {
            to.sound = u16::from(reader.byte()?);
        }
        if (bits & protocol::U_EVENT) != 0 {
            to.event = reader.byte()?;
        } else {
            to.event = 0;
        }
        if (bits & protocol::U_SOLID) != 0 {
            to.solid = if self.long_solid() {
                reader.long()? as u32
            } else {
                u32::from(reader.short()? as u16)
            };
        }
        Ok(to)
    }

    /// Write a spawn baseline (`writeSpawnBaseline`).
    pub fn write_spawn_baseline(&self, writer: &mut MsgWriter, base: &EntityState) -> Result<bool, Q2CodecError> {
        writer.write_byte(protocol::Svc::Spawnbaseline as u8)?;
        self.write_delta_entity(writer, &EntityState::default(), base, true, true)
    }

    /// Encode an R1Q2 player-state delta (`encodePlayerStateDelta`).
    #[must_use]
    pub fn encode_player_state(from: &PlayerState, to: &PlayerState) -> R1q2PlayerDelta {
        let mut flags = 0u16;
        let mut extraflags = 0u8;
        if to.pmove.pm_type != from.pmove.pm_type {
            flags |= protocol::PS_M_TYPE as u16;
        }
        if to.pmove.origin[0] != from.pmove.origin[0] || to.pmove.origin[1] != from.pmove.origin[1] {
            flags |= protocol::PS_M_ORIGIN as u16;
        }
        if to.pmove.origin[2] != from.pmove.origin[2] {
            extraflags |= protocol::EPS_M_ORIGIN2 as u8;
        }
        if to.pmove.velocity[0] != from.pmove.velocity[0] || to.pmove.velocity[1] != from.pmove.velocity[1] {
            flags |= protocol::PS_M_VELOCITY as u16;
        }
        if to.pmove.velocity[2] != from.pmove.velocity[2] {
            extraflags |= protocol::EPS_M_VELOCITY2 as u8;
        }
        if to.pmove.pm_time != from.pmove.pm_time {
            flags |= protocol::PS_M_TIME as u16;
        }
        if to.pmove.pm_flags != from.pmove.pm_flags {
            flags |= protocol::PS_M_FLAGS as u16;
        }
        if to.pmove.gravity != from.pmove.gravity {
            flags |= protocol::PS_M_GRAVITY as u16;
        }
        if to.pmove.delta_angles != from.pmove.delta_angles {
            flags |= protocol::PS_M_DELTA_ANGLES as u16;
        }
        if to.viewoffset != from.viewoffset {
            flags |= protocol::PS_VIEWOFFSET as u16;
        }
        if to.viewangles[0] != from.viewangles[0] || to.viewangles[1] != from.viewangles[1] {
            flags |= protocol::PS_VIEWANGLES as u16;
        }
        if to.viewangles[2] != from.viewangles[2] {
            extraflags |= protocol::EPS_VIEWANGLE2 as u8;
        }
        if to.kick_angles != from.kick_angles {
            flags |= protocol::PS_KICKANGLES as u16;
        }
        if to.blend != from.blend {
            flags |= protocol::PS_BLEND as u16;
        }
        if to.fov != from.fov {
            flags |= protocol::PS_FOV as u16;
        }
        if to.rdflags != from.rdflags {
            flags |= protocol::PS_RDFLAGS as u16;
        }
        flags |= protocol::PS_WEAPONINDEX as u16;
        if to.gunframe != from.gunframe {
            flags |= protocol::PS_WEAPONFRAME as u16;
        }
        if to.gunoffset != from.gunoffset {
            extraflags |= protocol::EPS_GUNOFFSET as u8;
        }
        if to.gunangles != from.gunangles {
            extraflags |= protocol::EPS_GUNANGLES as u8;
        }
        let mut statbits = 0u32;
        for i in 0..MAX_STATS {
            if to.stats[i] != from.stats[i] {
                statbits |= 1 << i;
            }
        }
        if statbits != 0 {
            extraflags |= protocol::EPS_STATS as u8;
        }
        R1q2PlayerDelta {
            flags,
            extraflags,
            statbits,
        }
    }

    /// Write an R1Q2 player-state delta body after its flags.
    pub fn write_player_state_body(
        writer: &mut MsgWriter,
        to: &PlayerState,
        delta: R1q2PlayerDelta,
    ) -> Result<(), MsgError> {
        let flags = u32::from(delta.flags);
        let extraflags = u32::from(delta.extraflags);
        if (flags & protocol::PS_M_TYPE) != 0 {
            writer.write_byte(to.pmove.pm_type)?;
        }
        if (flags & protocol::PS_M_ORIGIN) != 0 {
            writer.write_short(to.pmove.origin[0] as i16)?;
            writer.write_short(to.pmove.origin[1] as i16)?;
        }
        if (extraflags & protocol::EPS_M_ORIGIN2) != 0 {
            writer.write_short(to.pmove.origin[2] as i16)?;
        }
        if (flags & protocol::PS_M_VELOCITY) != 0 {
            writer.write_short(to.pmove.velocity[0] as i16)?;
            writer.write_short(to.pmove.velocity[1] as i16)?;
        }
        if (extraflags & protocol::EPS_M_VELOCITY2) != 0 {
            writer.write_short(to.pmove.velocity[2] as i16)?;
        }
        if (flags & protocol::PS_M_TIME) != 0 {
            writer.write_byte(to.pmove.pm_time as u8)?;
        }
        if (flags & protocol::PS_M_FLAGS) != 0 {
            writer.write_byte(to.pmove.pm_flags as u8)?;
        }
        if (flags & protocol::PS_M_GRAVITY) != 0 {
            writer.write_short(to.pmove.gravity)?;
        }
        if (flags & protocol::PS_M_DELTA_ANGLES) != 0 {
            for axis in to.pmove.delta_angles {
                writer.write_short(axis)?;
            }
        }
        if (flags & protocol::PS_VIEWOFFSET) != 0 {
            for axis in to.viewoffset {
                writer.write_char(scaled_trunc(axis, 4.0) as i8)?;
            }
        }
        if (flags & protocol::PS_VIEWANGLES) != 0 {
            writer.write_q2_angle16(to.viewangles[0])?;
            writer.write_q2_angle16(to.viewangles[1])?;
        }
        if (extraflags & protocol::EPS_VIEWANGLE2) != 0 {
            writer.write_q2_angle16(to.viewangles[2])?;
        }
        if (flags & protocol::PS_KICKANGLES) != 0 {
            for axis in to.kick_angles {
                writer.write_char(scaled_trunc(axis, 4.0) as i8)?;
            }
        }
        if (flags & protocol::PS_WEAPONINDEX) != 0 {
            writer.write_byte(to.gunindex as u8)?;
        }
        if (flags & protocol::PS_WEAPONFRAME) != 0 {
            writer.write_byte(to.gunframe as u8)?;
        }
        if (extraflags & protocol::EPS_GUNOFFSET) != 0 {
            for axis in to.gunoffset {
                writer.write_char(scaled_trunc(axis, 4.0) as i8)?;
            }
        }
        if (extraflags & protocol::EPS_GUNANGLES) != 0 {
            for axis in to.gunangles {
                writer.write_char(scaled_trunc(axis, 4.0) as i8)?;
            }
        }
        if (flags & protocol::PS_BLEND) != 0 {
            for axis in to.blend {
                writer.write_byte(scaled_trunc(axis, 255.0) as u8)?;
            }
        }
        if (flags & protocol::PS_FOV) != 0 {
            writer.write_byte(to.fov)?;
        }
        if (flags & protocol::PS_RDFLAGS) != 0 {
            writer.write_byte(to.rdflags)?;
        }
        if (extraflags & protocol::EPS_STATS) != 0 {
            writer.write_long(delta.statbits as i32)?;
            for i in 0..MAX_STATS {
                if (delta.statbits & (1 << i)) != 0 {
                    writer.write_short(to.stats[i])?;
                }
            }
        }
        Ok(())
    }

    /// Write an R1Q2 player-state delta (flags only, no opcode).
    pub fn write_player_state_delta(
        writer: &mut MsgWriter,
        from: &PlayerState,
        to: &PlayerState,
    ) -> Result<(), MsgError> {
        let delta = Self::encode_player_state(from, to);
        writer.write_short(delta.flags as i16)?;
        writer.write_byte(delta.extraflags)?;
        Self::write_player_state_body(writer, to, delta)
    }

    /// Read an R1Q2 player-state delta body.
    pub fn read_player_state_body(
        reader: &mut MsgReader<'_>,
        from: &PlayerState,
        flags: u16,
        extraflags: u8,
    ) -> Result<PlayerState, MsgError> {
        let mut to = from.clone();
        let flags = u32::from(flags);
        let extraflags = u32::from(extraflags);
        if (flags & protocol::PS_M_TYPE) != 0 {
            to.pmove.pm_type = reader.byte()?;
        }
        if (flags & protocol::PS_M_ORIGIN) != 0 {
            to.pmove.origin[0] = i32::from(reader.short()?);
            to.pmove.origin[1] = i32::from(reader.short()?);
        }
        if (extraflags & protocol::EPS_M_ORIGIN2) != 0 {
            to.pmove.origin[2] = i32::from(reader.short()?);
        }
        if (flags & protocol::PS_M_VELOCITY) != 0 {
            to.pmove.velocity[0] = i32::from(reader.short()?);
            to.pmove.velocity[1] = i32::from(reader.short()?);
        }
        if (extraflags & protocol::EPS_M_VELOCITY2) != 0 {
            to.pmove.velocity[2] = i32::from(reader.short()?);
        }
        if (flags & protocol::PS_M_TIME) != 0 {
            to.pmove.pm_time = i32::from(reader.byte()?);
        }
        if (flags & protocol::PS_M_FLAGS) != 0 {
            to.pmove.pm_flags = i32::from(reader.byte()?);
        }
        if (flags & protocol::PS_M_GRAVITY) != 0 {
            to.pmove.gravity = reader.short()?;
        }
        if (flags & protocol::PS_M_DELTA_ANGLES) != 0 {
            for axis in &mut to.pmove.delta_angles {
                *axis = reader.short()?;
            }
        }
        if (flags & protocol::PS_VIEWOFFSET) != 0 {
            for axis in &mut to.viewoffset {
                *axis = f64::from(reader.char()?) * 0.25;
            }
        }
        if (flags & protocol::PS_VIEWANGLES) != 0 {
            to.viewangles[0] = short_to_angle(reader.short()?);
            to.viewangles[1] = short_to_angle(reader.short()?);
        }
        if (extraflags & protocol::EPS_VIEWANGLE2) != 0 {
            to.viewangles[2] = short_to_angle(reader.short()?);
        }
        if (flags & protocol::PS_KICKANGLES) != 0 {
            for axis in &mut to.kick_angles {
                *axis = f64::from(reader.char()?) * 0.25;
            }
        }
        if (flags & protocol::PS_WEAPONINDEX) != 0 {
            to.gunindex = i32::from(reader.byte()?);
        }
        if (flags & protocol::PS_WEAPONFRAME) != 0 {
            to.gunframe = i32::from(reader.byte()?);
        }
        if (extraflags & protocol::EPS_GUNOFFSET) != 0 {
            for axis in &mut to.gunoffset {
                *axis = f64::from(reader.char()?) * 0.25;
            }
        }
        if (extraflags & protocol::EPS_GUNANGLES) != 0 {
            for axis in &mut to.gunangles {
                *axis = f64::from(reader.char()?) * 0.25;
            }
        }
        if (flags & protocol::PS_BLEND) != 0 {
            for axis in &mut to.blend {
                *axis = f64::from(reader.byte()?) / 255.0;
            }
        }
        if (flags & protocol::PS_FOV) != 0 {
            to.fov = reader.byte()?;
        }
        if (flags & protocol::PS_RDFLAGS) != 0 {
            to.rdflags = reader.byte()?;
        }
        if (extraflags & protocol::EPS_STATS) != 0 {
            let statbits = reader.long()? as u32;
            for i in 0..MAX_STATS {
                if (statbits & (1 << i)) != 0 {
                    to.stats[i] = reader.short()?;
                }
            }
        }
        Ok(to)
    }

    /// Read an R1Q2 player-state delta (`readPlayerStateDelta`).
    pub fn read_player_state_delta(reader: &mut MsgReader<'_>, from: &PlayerState) -> Result<PlayerState, MsgError> {
        let flags = reader.short()? as u16;
        let extraflags = reader.byte()?;
        Self::read_player_state_body(reader, from, flags, extraflags)
    }

    /// Write an R1Q2 frame (`writeFrame`).
    pub fn write_frame<E>(
        writer: &mut MsgWriter,
        params: &FrameWrite<'_>,
        write_entities: impl FnOnce(&mut MsgWriter) -> Result<(), E>,
    ) -> Result<(), E>
    where
        E: From<MsgError>,
    {
        let base = PlayerState::default();
        let from = params.ps_from.unwrap_or(&base);
        let delta = Self::encode_player_state(from, params.ps_to);
        let opcode = protocol::Svc::Frame as u8 | (((u16::from(delta.extraflags) & 0xf0) << 1) as u8);
        writer.write_byte(opcode)?;
        let offset = if params.lastframe == -1 {
            31
        } else {
            params.framenum - params.lastframe
        };
        writer.write_long((params.framenum & 0x07ff_ffff) | (offset.wrapping_shl(27)))?;
        writer.write_byte(((params.surpress_count & 0x0f) | ((i32::from(delta.extraflags) & 0x0f) << 4)) as u8)?;
        writer.write_byte(params.areabits.len() as u8)?;
        writer.write_bytes(params.areabits)?;
        writer.write_short(delta.flags as i16)?;
        Self::write_player_state_body(writer, params.ps_to, delta)?;
        write_entities(writer)
    }

    /// Read an R1Q2 frame header (`readFrameHeader`).
    pub fn read_frame_header(
        &mut self,
        reader: &mut MsgReader<'_>,
        areabits: &mut Vec<u8>,
    ) -> Result<FrameHeader, MsgError> {
        let encoded = reader.long()? as u32;
        let offset = (encoded >> 27) as i32;
        let serverframe = (encoded & 0x07ff_ffff) as i32;
        let deltaframe = if offset == 31 { -1 } else { serverframe - offset };
        let suppress = reader.byte()?;
        let extraflags = (u16::from(self.frame_extrabits) >> 1) | (u16::from(suppress & 0xf0) >> 4);
        self.frame_extraflags = extraflags as u8;
        let surpress_count = i32::from(suppress & 0x0f);
        let len = usize::from(reader.byte()?);
        areabits.clear();
        areabits.extend_from_slice(reader.bytes(len)?);
        Ok(FrameHeader {
            serverframe,
            deltaframe,
            surpress_count,
            areabytes: len,
        })
    }

    /// Read the player state of an R1Q2 frame (`readFramePlayerstate`).
    pub fn read_frame_playerstate(
        &mut self,
        reader: &mut MsgReader<'_>,
        from: &PlayerState,
    ) -> Result<PlayerState, MsgError> {
        let flags = reader.short()? as u16;
        Self::read_player_state_body(reader, from, flags, self.frame_extraflags)
    }

    /// Write a delta user command, compressing movements on new revisions.
    pub fn write_delta_usercmd(&self, writer: &mut MsgWriter, from: &Usercmd, cmd: &Usercmd) -> Result<(), MsgError> {
        let mut bits = 0u8;
        if cmd.angles[0] != from.angles[0] {
            bits |= 1 << 0;
        }
        if cmd.angles[1] != from.angles[1] {
            bits |= 1 << 1;
        }
        if cmd.angles[2] != from.angles[2] {
            bits |= 1 << 2;
        }
        if cmd.forwardmove != from.forwardmove {
            bits |= 1 << 3;
        }
        if cmd.sidemove != from.sidemove {
            bits |= 1 << 4;
        }
        if cmd.upmove != from.upmove {
            bits |= 1 << 5;
        }
        if cmd.buttons != from.buttons {
            bits |= 1 << 6;
        }
        if cmd.impulse != from.impulse {
            bits |= 1 << 7;
        }
        writer.write_byte(bits)?;
        let mut buttons = if (bits & (1 << 6)) != 0 { cmd.buttons } else { 0 };
        if self.compressed_movements() && (bits & (1 << 6)) != 0 {
            if (bits & (1 << 3)) != 0 && cmd.forwardmove % 5 == 0 {
                buttons |= BUTTON_UCMD_DBLFORWARD;
            }
            if (bits & (1 << 4)) != 0 && cmd.sidemove % 5 == 0 {
                buttons |= BUTTON_UCMD_DBLSIDE;
            }
            if (bits & (1 << 5)) != 0 && cmd.upmove % 5 == 0 {
                buttons |= BUTTON_UCMD_DBLUP;
            }
            if (bits & (1 << 0)) != 0 && cmd.angles[0] % 64 == 0 && (i32::from(cmd.angles[0]) / 64).abs() < 128 {
                buttons |= BUTTON_UCMD_DBL_ANGLE1;
            }
            if (bits & (1 << 1)) != 0 && cmd.angles[1] % 256 == 0 {
                buttons |= BUTTON_UCMD_DBL_ANGLE2;
            }
            writer.write_byte(buttons)?;
        }
        if (bits & (1 << 0)) != 0 {
            if (buttons & BUTTON_UCMD_DBL_ANGLE1) != 0 {
                writer.write_char((i32::from(cmd.angles[0]) / 64) as i8)?;
            } else {
                writer.write_short(cmd.angles[0])?;
            }
        }
        if (bits & (1 << 1)) != 0 {
            if (buttons & BUTTON_UCMD_DBL_ANGLE2) != 0 {
                writer.write_char((i32::from(cmd.angles[1]) / 256) as i8)?;
            } else {
                writer.write_short(cmd.angles[1])?;
            }
        }
        if (bits & (1 << 2)) != 0 {
            writer.write_short(cmd.angles[2])?;
        }
        if (bits & (1 << 3)) != 0 {
            if (buttons & BUTTON_UCMD_DBLFORWARD) != 0 {
                writer.write_char((cmd.forwardmove / 5) as i8)?;
            } else {
                writer.write_short(cmd.forwardmove)?;
            }
        }
        if (bits & (1 << 4)) != 0 {
            if (buttons & BUTTON_UCMD_DBLSIDE) != 0 {
                writer.write_char((cmd.sidemove / 5) as i8)?;
            } else {
                writer.write_short(cmd.sidemove)?;
            }
        }
        if (bits & (1 << 5)) != 0 {
            if (buttons & BUTTON_UCMD_DBLUP) != 0 {
                writer.write_char((cmd.upmove / 5) as i8)?;
            } else {
                writer.write_short(cmd.upmove)?;
            }
        }
        if !self.compressed_movements() && (bits & (1 << 6)) != 0 {
            writer.write_byte(cmd.buttons)?;
        }
        if (bits & (1 << 7)) != 0 {
            writer.write_byte(cmd.impulse)?;
        }
        writer.write_byte(cmd.msec)?;
        writer.write_byte(cmd.lightlevel)
    }

    /// Read a delta user command.
    pub fn read_delta_usercmd(&self, reader: &mut MsgReader<'_>, from: &Usercmd) -> Result<Usercmd, MsgError> {
        let mut cmd = from.clone();
        let bits = reader.byte()?;
        let mut buttons = 0u8;
        if self.compressed_movements() && (bits & (1 << 6)) != 0 {
            buttons = reader.byte()?;
        }
        if (bits & (1 << 0)) != 0 {
            cmd.angles[0] = if (buttons & BUTTON_UCMD_DBL_ANGLE1) != 0 {
                i16::from(reader.char()?) * 64
            } else {
                reader.short()?
            };
        }
        if (bits & (1 << 1)) != 0 {
            cmd.angles[1] = if (buttons & BUTTON_UCMD_DBL_ANGLE2) != 0 {
                i16::from(reader.char()?) * 256
            } else {
                reader.short()?
            };
        }
        if (bits & (1 << 2)) != 0 {
            cmd.angles[2] = reader.short()?;
        }
        if (bits & (1 << 3)) != 0 {
            cmd.forwardmove = if (buttons & BUTTON_UCMD_DBLFORWARD) != 0 {
                i16::from(reader.char()?) * 5
            } else {
                reader.short()?
            };
        }
        if (bits & (1 << 4)) != 0 {
            cmd.sidemove = if (buttons & BUTTON_UCMD_DBLSIDE) != 0 {
                i16::from(reader.char()?) * 5
            } else {
                reader.short()?
            };
        }
        if (bits & (1 << 5)) != 0 {
            cmd.upmove = if (buttons & BUTTON_UCMD_DBLUP) != 0 {
                i16::from(reader.char()?) * 5
            } else {
                reader.short()?
            };
        }
        if !self.compressed_movements() && (bits & (1 << 6)) != 0 {
            buttons = reader.byte()?;
        }
        if (bits & (1 << 7)) != 0 {
            cmd.impulse = reader.byte()?;
        }
        cmd.msec = reader.byte()?;
        cmd.lightlevel = reader.byte()?;
        if (bits & (1 << 6)) != 0 {
            cmd.buttons = buttons
                & !(BUTTON_UCMD_DBLFORWARD
                    | BUTTON_UCMD_DBLSIDE
                    | BUTTON_UCMD_DBLUP
                    | BUTTON_UCMD_DBL_ANGLE1
                    | BUTTON_UCMD_DBL_ANGLE2);
        }
        Ok(cmd)
    }
}

/// Doubled forward-move flag stolen from the button byte.
const BUTTON_UCMD_DBLFORWARD: u8 = 1 << 2;
/// Doubled side-move flag stolen from the button byte.
const BUTTON_UCMD_DBLSIDE: u8 = 1 << 3;
/// Doubled up-move flag stolen from the button byte.
const BUTTON_UCMD_DBLUP: u8 = 1 << 4;
/// Doubled pitch flag stolen from the button byte.
const BUTTON_UCMD_DBL_ANGLE1: u8 = 1 << 5;
/// Doubled yaw flag stolen from the button byte.
const BUTTON_UCMD_DBL_ANGLE2: u8 = 1 << 6;

// ---------------------------------------------------------------------------
// Q2Pro
// ---------------------------------------------------------------------------

/// Negotiated Q2Pro revision and flags (`Q2ProFeatures`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q2ProFeatures {
    /// Protocol revision.
    pub revision: u16,
    /// Negotiated wire flags.
    pub flags: u16,
}

impl Q2ProFeatures {
    /// Whether extended game fields are negotiated (`q2proExtensions`).
    #[must_use]
    pub fn extensions(self) -> bool {
        (self.revision >= 1024 && (self.flags & 8) != 0) || (self.revision >= 1025 && (self.flags & 16) != 0)
    }

    /// Whether v2 extended fields are negotiated (`q2proExtensionsV2`).
    #[must_use]
    pub fn extensions_v2(self) -> bool {
        self.revision >= 1025 && (self.flags & 16) != 0
    }
}

/// Read a Q2Pro 23-bit fixed-point integer (`readQ2ProInt23`).
pub fn read_q2pro_int23(reader: &mut MsgReader<'_>, previous: i32) -> Result<i32, MsgError> {
    let word = reader.short()?;
    if (word & 1) != 0 {
        let lo = i32::from(word as u16);
        let hi = i32::from(reader.char()?) << 16;
        Ok((lo | hi) >> 1)
    } else {
        Ok(previous + (i32::from(word) >> 1))
    }
}

/// Write a Q2Pro 23-bit fixed-point integer (`writeQ2ProInt23`).
pub fn write_q2pro_int23(writer: &mut MsgWriter, current: i32, previous: i32) -> Result<(), VariantError> {
    if !(-4_194_304..=4_194_303).contains(&current) {
        return Err(VariantError::CoordinateRange(current));
    }
    let delta = current - previous;
    if (-16384..16384).contains(&delta) {
        writer.write_short((delta << 1) as i16)?;
    } else {
        let value = ((current << 1) | 1) as u32;
        writer.write_short(value as i16)?;
        writer.write_byte((value >> 16) as u8)?;
    }
    Ok(())
}

/// Read a Q2Pro variable-length uint64 (`readQ2ProVar64`).
pub fn read_q2pro_var64(reader: &mut MsgReader<'_>) -> Result<u64, VariantError> {
    let mut result = 0u64;
    for shift in (0..70).step_by(7) {
        let byte = reader.byte()?;
        if shift == 63 && byte > 1 {
            return Err(VariantError::BadVar64);
        }
        result |= u64::from(byte & 127) << shift;
        if (byte & 128) == 0 {
            return Ok(result);
        }
    }
    Err(VariantError::UnterminatedVar64)
}

/// Write a Q2Pro variable-length uint64 (`writeQ2ProVar64`).
pub fn write_q2pro_var64(writer: &mut MsgWriter, mut value: u64) -> Result<(), MsgError> {
    loop {
        let byte = (value & 127) as u8;
        value >>= 7;
        writer.write_byte(byte | if value != 0 { 128 } else { 0 })?;
        if value == 0 {
            return Ok(());
        }
    }
}

/// Compute Q2Pro fog change bits (`q2proFogBits`).
#[must_use]
pub fn q2pro_fog_bits(from: &Q2ProFog, to: &Q2ProFog) -> u8 {
    let mut bits = 0;
    if from.color != to.color {
        bits |= 1;
    }
    if from.density != to.density || from.sky_factor != to.sky_factor {
        bits |= 2;
    }
    if from.height_density != to.height_density {
        bits |= 4;
    }
    if from.height_falloff != to.height_falloff {
        bits |= 8;
    }
    if from.height_start_color != to.height_start_color {
        bits |= 16;
    }
    if from.height_end_color != to.height_end_color {
        bits |= 32;
    }
    if from.height_start_distance != to.height_start_distance {
        bits |= 64;
    }
    if from.height_end_distance != to.height_end_distance {
        bits |= 128;
    }
    bits
}

/// Read Q2Pro player fog (`readQ2ProFog`).
pub fn read_q2pro_fog(reader: &mut MsgReader<'_>, from: &Q2ProFog) -> Result<Q2ProFog, VariantError> {
    let mut to = from.clone();
    let bits = reader.byte()?;
    if (bits & 1) != 0 {
        to.color = [reader.byte()?, reader.byte()?, reader.byte()?];
    }
    if (bits & 2) != 0 {
        to.density = reader.word()?;
        to.sky_factor = reader.word()?;
    }
    if (bits & 4) != 0 {
        to.height_density = reader.word()?;
    }
    if (bits & 8) != 0 {
        to.height_falloff = reader.word()?;
    }
    if (bits & 16) != 0 {
        to.height_start_color = [reader.byte()?, reader.byte()?, reader.byte()?];
    }
    if (bits & 32) != 0 {
        to.height_end_color = [reader.byte()?, reader.byte()?, reader.byte()?];
    }
    if (bits & 64) != 0 {
        to.height_start_distance = read_q2pro_int23(reader, 0)?;
    }
    if (bits & 128) != 0 {
        to.height_end_distance = read_q2pro_int23(reader, 0)?;
    }
    Ok(to)
}

/// Write Q2Pro player fog (`writeQ2ProFog`).
pub fn write_q2pro_fog(writer: &mut MsgWriter, bits: u8, fog: &Q2ProFog) -> Result<(), VariantError> {
    writer.write_byte(bits)?;
    if (bits & 1) != 0 {
        for value in fog.color {
            writer.write_byte(value)?;
        }
    }
    if (bits & 2) != 0 {
        writer.write_short(fog.density as i16)?;
        writer.write_short(fog.sky_factor as i16)?;
    }
    if (bits & 4) != 0 {
        writer.write_short(fog.height_density as i16)?;
    }
    if (bits & 8) != 0 {
        writer.write_short(fog.height_falloff as i16)?;
    }
    if (bits & 16) != 0 {
        for value in fog.height_start_color {
            writer.write_byte(value)?;
        }
    }
    if (bits & 32) != 0 {
        for value in fog.height_end_color {
            writer.write_byte(value)?;
        }
    }
    if (bits & 64) != 0 {
        write_q2pro_int23(writer, fog.height_start_distance, 0)?;
    }
    if (bits & 128) != 0 {
        write_q2pro_int23(writer, fog.height_end_distance, 0)?;
    }
    Ok(())
}

const Q2P_ORIGIN_BITS: [u64; 3] = [1, 2, 512];
const Q2P_ANGLE_BITS: [u64; 3] = [1024, 4, 8];
const Q2P_MODEL_BITS: [u64; 4] = [2048, 1_048_576, 2_097_152, 4_194_304];
const Q2P_NUMBER16: u64 = 256;
const Q2P_ANGLE16: u64 = 8192;
const Q2P_MODEL16: u64 = 268_435_456;
const Q2P_MOREFX8: u64 = 536_870_912;
const Q2P_ALPHA: u64 = 1_073_741_824;
const Q2P_SCALE: u64 = 4_294_967_296;
const Q2P_MOREFX16: u64 = 8_589_934_592;
const Q2P_FRAME8: u64 = 16;
const Q2P_FRAME16: u64 = 131_072;
const Q2P_SKIN8: u64 = 65_536;
const Q2P_SKIN32: u64 = 33_554_432;
const Q2P_EFFECTS8: u64 = 16_384;
const Q2P_EFFECTS32: u64 = 524_288;
const Q2P_RENDERFX8: u64 = 4096;
const Q2P_RENDERFX32: u64 = 262_144;
const Q2P_SOLID: u64 = 134_217_728;
const Q2P_EVENT: u64 = 32;
const Q2P_SOUND: u64 = 67_108_864;
const Q2P_OLDORIGIN: u64 = 16_777_216;

/// Pick a Q2Pro field width for an unsigned value.
fn q2pro_width(value: u32, byte: u64, word: u64) -> u64 {
    if value < 256 {
        byte
    } else if value < 65536 {
        word
    } else {
        byte | word
    }
}

/// Write a Q2Pro width-selected field.
fn write_q2pro_width(writer: &mut MsgWriter, bits: u64, value: i32, byte: u64, word: u64) -> Result<(), MsgError> {
    if (bits & (byte | word)) == (byte | word) {
        writer.write_long(value)?;
    } else if (bits & byte) != 0 {
        writer.write_byte(value as u8)?;
    } else if (bits & word) != 0 {
        writer.write_short(value as i16)?;
    }
    Ok(())
}

/// Read a Q2Pro width-selected field.
fn read_q2pro_width(
    reader: &mut MsgReader<'_>,
    bits: u64,
    previous: i32,
    byte: u64,
    word: u64,
) -> Result<i32, MsgError> {
    if (bits & (byte | word)) == (byte | word) {
        Ok(reader.long()?)
    } else if (bits & byte) != 0 {
        Ok(i32::from(reader.byte()?))
    } else if (bits & word) != 0 {
        Ok(i32::from(reader.word()?))
    } else {
        Ok(previous)
    }
}

/// Read a Q2Pro entity header (`readQ2ProEntityBits`).
pub fn read_q2pro_entity_bits(reader: &mut MsgReader<'_>) -> Result<(u16, u64), MsgError> {
    let mut bits = u64::from(reader.byte()?);
    for i in 1..=4 {
        if (bits & (1 << (i * 8 - 1))) != 0 {
            bits |= u64::from(reader.byte()?) << (i * 8);
        }
    }
    let number = if (bits & Q2P_NUMBER16) != 0 {
        reader.short()? as u16
    } else {
        u16::from(reader.byte()?)
    };
    Ok((number, bits))
}

/// Write a Q2Pro delta entity (`writeQ2ProEntity`).
pub fn write_q2pro_entity(
    writer: &mut MsgWriter,
    features: Q2ProFeatures,
    from: &EntityState,
    to: &EntityState,
    force: bool,
    new_entity: bool,
) -> Result<bool, VariantError> {
    if to.number < 1 || to.number > 8191 {
        return Err(VariantError::EntityRange(to.number));
    }
    let extended = features.extensions();
    let v2 = features.extensions_v2();
    let mut bits = 0u64;
    for i in 0..3 {
        if from.origin[i] != to.origin[i] {
            bits |= Q2P_ORIGIN_BITS[i];
        }
        if from.angles[i] != to.angles[i] {
            bits |= Q2P_ANGLE_BITS[i];
        }
    }
    if (bits & (1024 | 4 | 8)) != 0 && features.revision >= 1018 {
        bits |= Q2P_ANGLE16;
    }
    let old_models = [from.modelindex, from.modelindex2, from.modelindex3, from.modelindex4];
    let models = [to.modelindex, to.modelindex2, to.modelindex3, to.modelindex4];
    for i in 0..4 {
        if old_models[i] != models[i] {
            bits |= Q2P_MODEL_BITS[i];
            if models[i] > 255 {
                bits |= Q2P_MODEL16;
            }
        }
    }
    if from.frame != to.frame {
        bits |= if to.frame < 256 { Q2P_FRAME8 } else { Q2P_FRAME16 };
    }
    if from.skinnum != to.skinnum {
        bits |= q2pro_width(to.skinnum as u32, Q2P_SKIN8, Q2P_SKIN32);
    }
    if from.effects != to.effects {
        bits |= q2pro_width(to.effects as u32, Q2P_EFFECTS8, Q2P_EFFECTS32);
    }
    if from.renderfx != to.renderfx {
        bits |= q2pro_width(to.renderfx as u32, Q2P_RENDERFX8, Q2P_RENDERFX32);
    }
    if from.morefx != to.morefx {
        bits |= q2pro_width(to.morefx as u32, Q2P_MOREFX8, Q2P_MOREFX16);
    }
    if from.alpha != to.alpha {
        bits |= Q2P_ALPHA;
    }
    if from.scale != to.scale {
        bits |= Q2P_SCALE;
    }
    if !extended && (bits & (Q2P_MODEL16 | Q2P_MOREFX8 | Q2P_MOREFX16 | Q2P_ALPHA | Q2P_SCALE)) != 0 {
        return Err(VariantError::ExtensionsRequired);
    }
    if from.solid != to.solid {
        bits |= Q2P_SOLID;
    }
    if to.event != 0 {
        bits |= Q2P_EVENT;
    }
    let volume_changed = from.loop_volume != to.loop_volume;
    let attenuation_changed = from.loop_attenuation != to.loop_attenuation;
    if from.sound != to.sound || volume_changed || attenuation_changed {
        bits |= Q2P_SOUND;
    }
    if !extended && (to.sound > 255 || volume_changed || attenuation_changed) {
        return Err(VariantError::ExtensionsRequired);
    }
    if new_entity || ((to.renderfx & RF_BEAM) != 0 && (features.revision < 1017 || from.old_origin != to.old_origin)) {
        bits |= Q2P_OLDORIGIN;
    }
    if bits == 0 && !force {
        return Ok(false);
    }
    if to.number >= 256 {
        bits |= Q2P_NUMBER16;
    }
    for i in (1..=4).rev() {
        if (bits >> (i * 8)) != 0 {
            bits |= 1 << (i * 8 - 1);
        }
    }
    writer.write_byte((bits & 255) as u8)?;
    for i in 1..=4 {
        if (bits & (1 << (i * 8 - 1))) != 0 {
            writer.write_byte(((bits >> (i * 8)) & 255) as u8)?;
        }
    }
    if (bits & Q2P_NUMBER16) != 0 {
        writer.write_short(to.number as i16)?;
    } else {
        writer.write_byte(to.number as u8)?;
    }
    for i in 0..4 {
        if (bits & Q2P_MODEL_BITS[i]) != 0 {
            if (bits & Q2P_MODEL16) != 0 {
                writer.write_short(models[i] as i16)?;
            } else {
                writer.write_byte(models[i] as u8)?;
            }
        }
    }
    if (bits & Q2P_FRAME8) != 0 {
        writer.write_byte(to.frame as u8)?;
    } else if (bits & Q2P_FRAME16) != 0 {
        writer.write_short(to.frame as i16)?;
    }
    write_q2pro_width(writer, bits, to.skinnum, Q2P_SKIN8, Q2P_SKIN32)?;
    write_q2pro_width(writer, bits, to.effects, Q2P_EFFECTS8, Q2P_EFFECTS32)?;
    write_q2pro_width(writer, bits, to.renderfx, Q2P_RENDERFX8, Q2P_RENDERFX32)?;
    for ((bit, current_origin), previous_origin) in Q2P_ORIGIN_BITS.iter().zip(to.origin.iter()).zip(from.origin.iter())
    {
        if (bits & *bit) != 0 {
            let current = scaled_trunc(*current_origin, 8.0);
            if v2 {
                write_q2pro_int23(writer, current, scaled_trunc(*previous_origin, 8.0))?;
            } else {
                writer.write_short(current as i16)?;
            }
        }
    }
    for (bit, angle) in Q2P_ANGLE_BITS.iter().zip(to.angles.iter()) {
        if (bits & *bit) != 0 {
            if (bits & Q2P_ANGLE16) != 0 {
                writer.write_q2_angle16(*angle)?;
            } else {
                writer.write_q2_angle(*angle)?;
            }
        }
    }
    if (bits & Q2P_OLDORIGIN) != 0 {
        for axis in to.old_origin {
            if v2 {
                write_q2pro_int23(writer, scaled_trunc(axis, 8.0), 0)?;
            } else {
                writer.write_short(scaled_trunc(axis, 8.0) as i16)?;
            }
        }
    }
    if (bits & Q2P_SOUND) != 0 {
        if extended {
            let mut sound = to.sound;
            if volume_changed {
                sound |= 16384;
            }
            if attenuation_changed {
                sound |= 32768;
            }
            writer.write_short(sound as i16)?;
            if volume_changed {
                writer.write_byte(scaled_trunc(to.loop_volume, 255.0) as u8)?;
            }
            if attenuation_changed {
                writer.write_byte(if to.loop_attenuation == -1.0 {
                    192
                } else {
                    scaled_trunc(to.loop_attenuation, 64.0) as u8
                })?;
            }
        } else {
            writer.write_byte(to.sound as u8)?;
        }
    }
    if (bits & Q2P_EVENT) != 0 {
        writer.write_byte(to.event)?;
    }
    if (bits & Q2P_SOLID) != 0 {
        writer.write_long(to.solid as i32)?;
    }
    write_q2pro_width(writer, bits, to.morefx, Q2P_MOREFX8, Q2P_MOREFX16)?;
    if (bits & Q2P_ALPHA) != 0 {
        writer.write_byte(if to.alpha == 0.0 {
            0
        } else {
            scaled_trunc(to.alpha, 255.0).clamp(1, 255) as u8
        })?;
    }
    if (bits & Q2P_SCALE) != 0 {
        writer.write_byte(if to.scale == 0.0 {
            0
        } else {
            scaled_trunc(to.scale, 16.0).clamp(1, 255) as u8
        })?;
    }
    Ok(true)
}

/// Read a Q2Pro delta entity body (`readQ2ProEntity`).
pub fn read_q2pro_entity(
    reader: &mut MsgReader<'_>,
    features: Q2ProFeatures,
    from: &EntityState,
    number: u16,
    bits: u64,
) -> Result<EntityState, VariantError> {
    let extended = features.extensions();
    let v2 = features.extensions_v2();
    let mut to = from.clone();
    to.old_origin = from.origin;
    to.number = number;
    to.event = 0;
    let mut models = [to.modelindex, to.modelindex2, to.modelindex3, to.modelindex4];
    for i in 0..4 {
        if (bits & Q2P_MODEL_BITS[i]) != 0 {
            models[i] = if extended && (bits & Q2P_MODEL16) != 0 {
                reader.word()?
            } else {
                u16::from(reader.byte()?)
            };
        }
    }
    [to.modelindex, to.modelindex2, to.modelindex3, to.modelindex4] = models;
    if (bits & Q2P_FRAME8) != 0 {
        to.frame = i32::from(reader.byte()?);
    } else if (bits & Q2P_FRAME16) != 0 {
        to.frame = i32::from(reader.word()?);
    }
    to.skinnum = read_q2pro_width(reader, bits, to.skinnum, Q2P_SKIN8, Q2P_SKIN32)?;
    to.effects = read_q2pro_width(reader, bits, to.effects, Q2P_EFFECTS8, Q2P_EFFECTS32)?;
    to.renderfx = read_q2pro_width(reader, bits, to.renderfx, Q2P_RENDERFX8, Q2P_RENDERFX32)?;
    for ((bit, slot), previous) in Q2P_ORIGIN_BITS.iter().zip(to.origin.iter_mut()).zip(from.origin.iter()) {
        if (bits & *bit) != 0 {
            let raw = if v2 {
                read_q2pro_int23(reader, scaled_trunc(*previous, 8.0))?
            } else {
                i32::from(reader.short()?)
            };
            *slot = f64::from(raw) / 8.0;
        }
    }
    for (bit, slot) in Q2P_ANGLE_BITS.iter().zip(to.angles.iter_mut()) {
        if (bits & *bit) != 0 {
            *slot = if (bits & Q2P_ANGLE16) != 0 {
                short_to_angle(reader.short()?)
            } else {
                reader.q2_angle()?
            };
        }
    }
    if (bits & Q2P_OLDORIGIN) != 0 {
        for axis in &mut to.old_origin {
            let raw = if v2 {
                read_q2pro_int23(reader, 0)?
            } else {
                i32::from(reader.short()?)
            };
            *axis = f64::from(raw) / 8.0;
        }
    }
    if (bits & Q2P_SOUND) != 0 {
        if extended {
            let sound = reader.word()?;
            to.sound = sound & 16383;
            if (sound & 16384) != 0 {
                to.loop_volume = f64::from(reader.byte()?) / 255.0;
            }
            if (sound & 32768) != 0 {
                let value = reader.byte()?;
                to.loop_attenuation = if value == 192 { -1.0 } else { f64::from(value) / 64.0 };
            }
        } else {
            to.sound = u16::from(reader.byte()?);
        }
    }
    if (bits & Q2P_EVENT) != 0 {
        to.event = reader.byte()?;
    }
    if (bits & Q2P_SOLID) != 0 {
        to.solid = reader.long()? as u32;
    }
    to.morefx = read_q2pro_width(reader, bits, to.morefx, Q2P_MOREFX8, Q2P_MOREFX16)?;
    if (bits & Q2P_ALPHA) != 0 {
        to.alpha = f64::from(reader.byte()?) / 255.0;
    }
    if (bits & Q2P_SCALE) != 0 {
        to.scale = f64::from(reader.byte()?) / 16.0;
    }
    Ok(to)
}

/// Q2Pro server data (`ServerDataParamsT` / `ServerDataReadResultT`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2ProServerData {
    /// Server count.
    pub servercount: i32,
    /// Attract loop.
    pub attractloop: bool,
    /// Game directory.
    pub gamedir: String,
    /// Client number.
    pub clientnum: i16,
    /// Level name.
    pub levelname: String,
    /// Q2Pro revision.
    pub version: u16,
    /// Server state.
    pub server_state: u8,
    /// Negotiated wire flags.
    pub wire_flags: u16,
}

impl Q2ProServerData {
    /// Strafejump hack negotiated.
    #[must_use]
    pub fn strafejump_hack(&self) -> bool {
        (self.wire_flags & 1) != 0
    }

    /// QW movement mode negotiated.
    #[must_use]
    pub fn qw_mode(&self) -> bool {
        (self.wire_flags & 2) != 0
    }

    /// Waterjump hack negotiated.
    #[must_use]
    pub fn waterjump_hack(&self) -> bool {
        (self.wire_flags & 4) != 0
    }
}

/// Encoded Q2Pro player-state delta.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q2ProPlayerDelta {
    /// Main flags.
    pub flags: u32,
    /// Extra flags.
    pub extraflags: u8,
    /// Changed stat bits.
    pub statbits: u64,
    /// Changed blend component bits.
    pub blendbits: u8,
    /// Changed fog bits.
    pub fogbits: u8,
}

/// Q2Pro protocol codec (`createQ2ProCodec`).
#[derive(Debug, Clone)]
pub struct Q2ProCodec {
    features: Q2ProFeatures,
    frame_opcode_extrabits: u8,
    frame_extraflags: u8,
}

impl Q2ProCodec {
    /// Build a codec with negotiated features.
    #[must_use]
    pub fn new(features: Q2ProFeatures) -> Self {
        Self {
            features,
            frame_opcode_extrabits: 0,
            frame_extraflags: 0,
        }
    }

    /// Negotiated features.
    #[must_use]
    pub fn features(&self) -> Q2ProFeatures {
        self.features
    }

    /// Note frame opcode extra bits before reading a frame header.
    pub fn note_frame_opcode_extrabits(&mut self, extrabits: u8) {
        self.frame_opcode_extrabits = extrabits;
    }

    /// Write server data, negotiating the carried revision and flags.
    pub fn write_server_data(&mut self, writer: &mut MsgWriter, params: &Q2ProServerData) -> Result<(), MsgError> {
        writer.write_byte(protocol::Svc::Serverdata as u8)?;
        writer.write_long(protocol::PROTOCOL_VERSION_Q2PRO as i32)?;
        writer.write_long(params.servercount)?;
        writer.write_byte(u8::from(params.attractloop))?;
        writer.write_string(&params.gamedir)?;
        writer.write_short(params.clientnum)?;
        writer.write_string(&params.levelname)?;
        self.features.revision = params.version;
        self.features.flags = params.wire_flags;
        writer.write_short(params.version as i16)?;
        writer.write_byte(params.server_state)?;
        if params.version >= 1024 {
            writer.write_short(params.wire_flags as i16)?;
        } else {
            writer.write_byte(u8::from((params.wire_flags & 1) != 0))?;
            writer.write_byte(u8::from((params.wire_flags & 2) != 0))?;
            writer.write_byte(u8::from((params.wire_flags & 4) != 0))?;
        }
        Ok(())
    }

    /// Read server data, negotiating the carried revision and flags.
    pub fn read_server_data(
        &mut self,
        reader: &mut MsgReader<'_>,
        minor_version: u16,
    ) -> Result<Q2ProServerData, MsgError> {
        let servercount = reader.long()?;
        let attractloop = reader.byte()? != 0;
        let gamedir = reader.string(2047);
        let clientnum = reader.short()?;
        let levelname = reader.string(2047);
        let version = reader.word()?;
        self.features.revision = if version == 0 { minor_version } else { version };
        let server_state = reader.byte()?;
        self.features.flags = if self.features.revision >= 1024 {
            reader.word()?
        } else {
            u16::from(reader.byte()? != 0)
                | (u16::from(reader.byte()? != 0) << 1)
                | (u16::from(reader.byte()? != 0) << 2)
        };
        Ok(Q2ProServerData {
            servercount,
            attractloop,
            gamedir,
            clientnum,
            levelname,
            version: self.features.revision,
            server_state,
            wire_flags: self.features.flags,
        })
    }

    /// Write a delta entity; `Ok(false)` writes nothing when unchanged and not forced.
    pub fn write_delta_entity(
        &self,
        writer: &mut MsgWriter,
        from: &EntityState,
        to: &EntityState,
        force: bool,
        new_entity: bool,
    ) -> Result<bool, VariantError> {
        write_q2pro_entity(writer, self.features, from, to, force, new_entity)
    }

    /// Read a delta entity body.
    pub fn read_delta_entity(
        &self,
        reader: &mut MsgReader<'_>,
        from: &EntityState,
        number: u16,
        bits: u64,
    ) -> Result<EntityState, VariantError> {
        read_q2pro_entity(reader, self.features, from, number, bits)
    }

    /// Write a spawn baseline (`writeSpawnBaseline`).
    pub fn write_spawn_baseline(&self, writer: &mut MsgWriter, base: &EntityState) -> Result<bool, VariantError> {
        writer.write_byte(protocol::Svc::Spawnbaseline as u8)?;
        self.write_delta_entity(writer, &EntityState::default(), base, true, true)
    }

    /// Encode a Q2Pro player-state delta.
    pub fn encode_player_state(&self, from: &PlayerState, to: &PlayerState) -> Result<Q2ProPlayerDelta, VariantError> {
        let v2 = self.features.extensions_v2();
        let mut flags = 0u32;
        let mut extraflags = 0u8;
        if to.pmove.pm_type != from.pmove.pm_type {
            flags |= protocol::PS_M_TYPE;
        }
        if to.pmove.origin[0] != from.pmove.origin[0] || to.pmove.origin[1] != from.pmove.origin[1] {
            flags |= protocol::PS_M_ORIGIN;
        }
        if to.pmove.origin[2] != from.pmove.origin[2] {
            extraflags |= protocol::EPS_M_ORIGIN2 as u8;
        }
        if to.pmove.velocity[0] != from.pmove.velocity[0] || to.pmove.velocity[1] != from.pmove.velocity[1] {
            flags |= protocol::PS_M_VELOCITY;
        }
        if to.pmove.velocity[2] != from.pmove.velocity[2] {
            extraflags |= protocol::EPS_M_VELOCITY2 as u8;
        }
        if to.pmove.pm_time != from.pmove.pm_time {
            flags |= protocol::PS_M_TIME;
        }
        if to.pmove.pm_flags != from.pmove.pm_flags {
            flags |= protocol::PS_M_FLAGS;
        }
        if to.pmove.gravity != from.pmove.gravity {
            flags |= protocol::PS_M_GRAVITY;
        }
        if to.pmove.delta_angles != from.pmove.delta_angles {
            flags |= protocol::PS_M_DELTA_ANGLES;
        }
        if to.viewoffset != from.viewoffset {
            flags |= protocol::PS_VIEWOFFSET;
        }
        if to.viewangles[0] != from.viewangles[0] || to.viewangles[1] != from.viewangles[1] {
            flags |= protocol::PS_VIEWANGLES;
        }
        if to.viewangles[2] != from.viewangles[2] {
            extraflags |= protocol::EPS_VIEWANGLE2 as u8;
        }
        if to.kick_angles != from.kick_angles {
            flags |= protocol::PS_KICKANGLES;
        }
        if to.gunindex != from.gunindex || to.gunskin != from.gunskin {
            flags |= protocol::PS_WEAPONINDEX;
        }
        if to.gunframe != from.gunframe {
            flags |= protocol::PS_WEAPONFRAME;
        }
        if to.gunoffset != from.gunoffset {
            extraflags |= protocol::EPS_GUNOFFSET as u8;
        }
        if to.gunangles != from.gunangles {
            extraflags |= protocol::EPS_GUNANGLES as u8;
        }
        if to.blend != from.blend {
            flags |= protocol::PS_BLEND;
        }
        if to.fov != from.fov {
            flags |= protocol::PS_FOV;
        }
        if to.rdflags != from.rdflags {
            flags |= protocol::PS_RDFLAGS;
        }
        let numstats = if v2 { MAX_STATS_STORAGE } else { MAX_STATS };
        let mut statbits = 0u64;
        for i in 0..numstats {
            if to.stats[i] != from.stats[i] {
                statbits |= 1 << i;
            }
        }
        if statbits != 0 {
            extraflags |= protocol::EPS_STATS as u8;
        }
        if to.clientnum != from.clientnum {
            extraflags |= 64;
        }
        let mut blendbits = 0u8;
        for i in 0..4 {
            if to.blend[i] != from.blend[i] {
                blendbits |= 1 << i;
            }
            if to.damage_blend[i] != from.damage_blend[i] {
                blendbits |= 16 << i;
            }
        }
        if v2 && blendbits != 0 {
            flags |= protocol::PS_BLEND;
        }
        let fogbits = q2pro_fog_bits(&from.fog, &to.fog);
        if fogbits != 0 {
            if self.features.revision < 1026 {
                return Err(VariantError::FogRevision(self.features.revision));
            }
            flags |= 0x18000;
        }
        Ok(Q2ProPlayerDelta {
            flags,
            extraflags,
            statbits,
            blendbits,
            fogbits,
        })
    }

    /// Write a Q2Pro player-state delta body after its flags.
    pub fn write_player_state_body(
        &self,
        writer: &mut MsgWriter,
        to: &PlayerState,
        from: &PlayerState,
        delta: Q2ProPlayerDelta,
    ) -> Result<(), VariantError> {
        let extended = self.features.extensions();
        let v2 = self.features.extensions_v2();
        let flags = delta.flags;
        let extraflags = u32::from(delta.extraflags);
        if (flags & protocol::PS_M_TYPE) != 0 {
            writer.write_byte(to.pmove.pm_type)?;
        }
        if (flags & protocol::PS_M_ORIGIN) != 0 {
            for i in 0..2 {
                if v2 {
                    write_q2pro_int23(writer, to.pmove.origin[i], from.pmove.origin[i])?;
                } else {
                    writer.write_short(to.pmove.origin[i] as i16)?;
                }
            }
        }
        if (extraflags & protocol::EPS_M_ORIGIN2) != 0 {
            if v2 {
                write_q2pro_int23(writer, to.pmove.origin[2], from.pmove.origin[2])?;
            } else {
                writer.write_short(to.pmove.origin[2] as i16)?;
            }
        }
        if (flags & protocol::PS_M_VELOCITY) != 0 {
            for i in 0..2 {
                if v2 {
                    write_q2pro_int23(writer, to.pmove.velocity[i], from.pmove.velocity[i])?;
                } else {
                    writer.write_short(to.pmove.velocity[i] as i16)?;
                }
            }
        }
        if (extraflags & protocol::EPS_M_VELOCITY2) != 0 {
            if v2 {
                write_q2pro_int23(writer, to.pmove.velocity[2], from.pmove.velocity[2])?;
            } else {
                writer.write_short(to.pmove.velocity[2] as i16)?;
            }
        }
        if (flags & protocol::PS_M_TIME) != 0 {
            if v2 {
                writer.write_short(to.pmove.pm_time as i16)?;
            } else {
                writer.write_byte(to.pmove.pm_time as u8)?;
            }
        }
        if (flags & protocol::PS_M_FLAGS) != 0 {
            if v2 {
                writer.write_short(to.pmove.pm_flags as i16)?;
            } else {
                writer.write_byte(to.pmove.pm_flags as u8)?;
            }
        }
        if (flags & protocol::PS_M_GRAVITY) != 0 {
            writer.write_short(to.pmove.gravity)?;
        }
        if (flags & protocol::PS_M_DELTA_ANGLES) != 0 {
            for axis in to.pmove.delta_angles {
                writer.write_short(axis)?;
            }
        }
        if (flags & protocol::PS_VIEWOFFSET) != 0 {
            for axis in to.viewoffset {
                writer.write_char(scaled_trunc(axis, 4.0) as i8)?;
            }
        }
        if (flags & protocol::PS_VIEWANGLES) != 0 {
            writer.write_q2_angle16(to.viewangles[0])?;
            writer.write_q2_angle16(to.viewangles[1])?;
        }
        if (extraflags & protocol::EPS_VIEWANGLE2) != 0 {
            writer.write_q2_angle16(to.viewangles[2])?;
        }
        if (flags & protocol::PS_KICKANGLES) != 0 {
            for axis in to.kick_angles {
                writer.write_char(scaled_trunc(axis, 4.0) as i8)?;
            }
        }
        if (flags & protocol::PS_WEAPONINDEX) != 0 {
            if extended {
                writer.write_short((to.gunindex | (to.gunskin << 13)) as i16)?;
            } else {
                if to.gunindex > 255 || to.gunskin != 0 {
                    return Err(VariantError::ExtensionsRequired);
                }
                writer.write_byte(to.gunindex as u8)?;
            }
        }
        if (flags & protocol::PS_WEAPONFRAME) != 0 {
            writer.write_byte(to.gunframe as u8)?;
        }
        if (extraflags & protocol::EPS_GUNOFFSET) != 0 {
            for axis in to.gunoffset {
                writer.write_char(scaled_trunc(axis, 4.0) as i8)?;
            }
        }
        if (extraflags & protocol::EPS_GUNANGLES) != 0 {
            for axis in to.gunangles {
                writer.write_char(scaled_trunc(axis, 4.0) as i8)?;
            }
        }
        if (flags & protocol::PS_BLEND) != 0 {
            if v2 {
                writer.write_byte(delta.blendbits)?;
                for i in 0..4 {
                    if (delta.blendbits & (1 << i)) != 0 {
                        writer.write_byte(scaled_trunc(to.blend[i], 255.0) as u8)?;
                    }
                }
                for i in 0..4 {
                    if (delta.blendbits & (16 << i)) != 0 {
                        writer.write_byte(scaled_trunc(to.damage_blend[i], 255.0) as u8)?;
                    }
                }
            } else {
                for axis in to.blend {
                    writer.write_byte(scaled_trunc(axis, 255.0) as u8)?;
                }
            }
        }
        if (flags & 0x10000) != 0 {
            write_q2pro_fog(writer, delta.fogbits, &to.fog)?;
        }
        if (flags & protocol::PS_FOV) != 0 {
            writer.write_byte(to.fov)?;
        }
        if (flags & protocol::PS_RDFLAGS) != 0 {
            writer.write_byte(to.rdflags)?;
        }
        if (extraflags & protocol::EPS_STATS) != 0 {
            let numstats = if v2 { MAX_STATS_STORAGE } else { MAX_STATS };
            if v2 {
                write_q2pro_var64(writer, delta.statbits)?;
            } else {
                writer.write_long(delta.statbits as i32)?;
            }
            for i in 0..numstats {
                if (delta.statbits & (1 << i)) != 0 {
                    writer.write_short(to.stats[i])?;
                }
            }
        }
        if (extraflags & 64) != 0 {
            if self.features.revision >= 1022 {
                writer.write_short(to.clientnum as i16)?;
            } else {
                writer.write_byte(to.clientnum as u8)?;
            }
        }
        Ok(())
    }

    /// Write a Q2Pro player-state delta (`writePlayerStateDelta`).
    pub fn write_player_state_delta(
        &self,
        writer: &mut MsgWriter,
        from: &PlayerState,
        to: &PlayerState,
    ) -> Result<(), VariantError> {
        let delta = self.encode_player_state(from, to)?;
        writer.write_byte(protocol::Svc::Playerinfo as u8)?;
        writer.write_short(delta.flags as i16)?;
        if (delta.flags & 0x8000) != 0 {
            writer.write_byte((delta.flags >> 16) as u8)?;
        }
        writer.write_byte(delta.extraflags)?;
        self.write_player_state_body(writer, to, from, delta)
    }

    /// Read a Q2Pro player-state delta body.
    pub fn read_player_state_body(
        &self,
        reader: &mut MsgReader<'_>,
        from: &PlayerState,
        flags: u32,
        extraflags: u8,
    ) -> Result<PlayerState, VariantError> {
        let extended = self.features.extensions();
        let v2 = self.features.extensions_v2();
        let mut to = from.clone();
        let extraflags = u32::from(extraflags);
        if (flags & protocol::PS_M_TYPE) != 0 {
            to.pmove.pm_type = reader.byte()?;
        }
        if (flags & protocol::PS_M_ORIGIN) != 0 {
            for i in 0..2 {
                to.pmove.origin[i] = if v2 {
                    read_q2pro_int23(reader, from.pmove.origin[i])?
                } else {
                    i32::from(reader.short()?)
                };
            }
        }
        if (extraflags & protocol::EPS_M_ORIGIN2) != 0 {
            to.pmove.origin[2] = if v2 {
                read_q2pro_int23(reader, from.pmove.origin[2])?
            } else {
                i32::from(reader.short()?)
            };
        }
        if (flags & protocol::PS_M_VELOCITY) != 0 {
            for i in 0..2 {
                to.pmove.velocity[i] = if v2 {
                    read_q2pro_int23(reader, from.pmove.velocity[i])?
                } else {
                    i32::from(reader.short()?)
                };
            }
        }
        if (extraflags & protocol::EPS_M_VELOCITY2) != 0 {
            to.pmove.velocity[2] = if v2 {
                read_q2pro_int23(reader, from.pmove.velocity[2])?
            } else {
                i32::from(reader.short()?)
            };
        }
        if (flags & protocol::PS_M_TIME) != 0 {
            to.pmove.pm_time = if v2 {
                i32::from(reader.word()?)
            } else {
                i32::from(reader.byte()?)
            };
        }
        if (flags & protocol::PS_M_FLAGS) != 0 {
            to.pmove.pm_flags = if v2 {
                i32::from(reader.word()?)
            } else {
                i32::from(reader.byte()?)
            };
        }
        if (flags & protocol::PS_M_GRAVITY) != 0 {
            to.pmove.gravity = reader.short()?;
        }
        if (flags & protocol::PS_M_DELTA_ANGLES) != 0 {
            for axis in &mut to.pmove.delta_angles {
                *axis = reader.short()?;
            }
        }
        if (flags & protocol::PS_VIEWOFFSET) != 0 {
            for axis in &mut to.viewoffset {
                *axis = f64::from(reader.char()?) * 0.25;
            }
        }
        if (flags & protocol::PS_VIEWANGLES) != 0 {
            to.viewangles[0] = short_to_angle(reader.short()?);
            to.viewangles[1] = short_to_angle(reader.short()?);
        }
        if (extraflags & protocol::EPS_VIEWANGLE2) != 0 {
            to.viewangles[2] = short_to_angle(reader.short()?);
        }
        if (flags & protocol::PS_KICKANGLES) != 0 {
            for axis in &mut to.kick_angles {
                *axis = f64::from(reader.char()?) * 0.25;
            }
        }
        if (flags & protocol::PS_WEAPONINDEX) != 0 {
            if extended {
                let gun = reader.word()?;
                to.gunindex = i32::from(gun & 8191);
                to.gunskin = i32::from(gun >> 13);
            } else {
                to.gunindex = i32::from(reader.byte()?);
            }
        }
        if (flags & protocol::PS_WEAPONFRAME) != 0 {
            to.gunframe = i32::from(reader.byte()?);
        }
        if (extraflags & protocol::EPS_GUNOFFSET) != 0 {
            for axis in &mut to.gunoffset {
                *axis = f64::from(reader.char()?) * 0.25;
            }
        }
        if (extraflags & protocol::EPS_GUNANGLES) != 0 {
            for axis in &mut to.gunangles {
                *axis = f64::from(reader.char()?) * 0.25;
            }
        }
        if (flags & protocol::PS_BLEND) != 0 {
            if v2 {
                let bits = reader.byte()?;
                for i in 0..4 {
                    if (bits & (1 << i)) != 0 {
                        to.blend[i] = f64::from(reader.byte()?) / 255.0;
                    }
                }
                for i in 0..4 {
                    if (bits & (16 << i)) != 0 {
                        to.damage_blend[i] = f64::from(reader.byte()?) / 255.0;
                    }
                }
            } else {
                for axis in &mut to.blend {
                    *axis = f64::from(reader.byte()?) / 255.0;
                }
            }
        }
        if (flags & 0x10000) != 0 {
            to.fog = read_q2pro_fog(reader, &from.fog)?;
        }
        if (flags & protocol::PS_FOV) != 0 {
            to.fov = reader.byte()?;
        }
        if (flags & protocol::PS_RDFLAGS) != 0 {
            to.rdflags = reader.byte()?;
        }
        if (extraflags & protocol::EPS_STATS) != 0 {
            let numstats = if v2 { MAX_STATS_STORAGE } else { MAX_STATS };
            let statbits = if v2 {
                read_q2pro_var64(reader)?
            } else {
                u64::from(reader.long()? as u32)
            };
            for i in 0..numstats {
                if (statbits & (1 << i)) != 0 {
                    to.stats[i] = reader.short()?;
                }
            }
        }
        if (extraflags & 64) != 0 {
            to.clientnum = if self.features.revision >= 1022 {
                i32::from(reader.short()?)
            } else {
                i32::from(reader.byte()?)
            };
        }
        Ok(to)
    }

    /// Read a Q2Pro player-state delta (`readPlayerStateDelta`).
    pub fn read_player_state_delta(
        &self,
        reader: &mut MsgReader<'_>,
        from: &PlayerState,
    ) -> Result<PlayerState, VariantError> {
        let mut flags = u32::from(reader.word()?);
        if self.features.revision >= 1026 && (flags & 0x8000) != 0 {
            flags |= u32::from(reader.byte()?) << 16;
        }
        let extraflags = reader.byte()?;
        self.read_player_state_body(reader, from, flags, extraflags)
    }

    /// Write a Q2Pro frame (`writeFrame`).
    pub fn write_frame<E>(
        &self,
        writer: &mut MsgWriter,
        params: &FrameWrite<'_>,
        write_entities: impl FnOnce(&mut MsgWriter) -> Result<(), E>,
    ) -> Result<(), E>
    where
        E: From<MsgError> + From<VariantError>,
    {
        let base = PlayerState::default();
        let from = params.ps_from.unwrap_or(&base);
        let delta = self.encode_player_state(from, params.ps_to)?;
        let extrabits = ((u16::from(delta.extraflags) & 0x70) << 1) as u8;
        writer.write_byte(protocol::Svc::Frame as u8 | extrabits)?;
        let offset = if params.lastframe == -1 {
            31
        } else {
            params.framenum - params.lastframe
        };
        writer.write_long((params.framenum & 0x07ff_ffff) | offset.wrapping_shl(27))?;
        writer.write_byte(((params.surpress_count & 0x0f) | ((i32::from(delta.extraflags) & 0x0f) << 4)) as u8)?;
        writer.write_byte(params.areabits.len() as u8)?;
        writer.write_bytes(params.areabits)?;
        writer.write_short(delta.flags as i16)?;
        if (delta.flags & 0x8000) != 0 {
            writer.write_byte((delta.flags >> 16) as u8)?;
        }
        self.write_player_state_body(writer, params.ps_to, from, delta)?;
        write_entities(writer)
    }

    /// Read a Q2Pro frame header (`readFrameHeader`).
    pub fn read_frame_header(
        &mut self,
        reader: &mut MsgReader<'_>,
        areabits: &mut Vec<u8>,
    ) -> Result<FrameHeader, MsgError> {
        let encoded = reader.long()? as u32;
        let offset = (encoded >> 27) as i32;
        let serverframe = (encoded & 0x07ff_ffff) as i32;
        let deltaframe = if offset == 31 { -1 } else { serverframe - offset };
        let extra_high = ((u16::from(self.frame_opcode_extrabits) >> 1) & 0x70) as u8;
        self.frame_opcode_extrabits = 0;
        let suppress = reader.byte()?;
        let surpress_count = i32::from(suppress & 0x0f);
        self.frame_extraflags = extra_high | ((suppress & 0xf0) >> 4);
        let len = usize::from(reader.byte()?);
        areabits.clear();
        areabits.extend_from_slice(reader.bytes(len)?);
        Ok(FrameHeader {
            serverframe,
            deltaframe,
            surpress_count,
            areabytes: len,
        })
    }

    /// Read the player state of a Q2Pro frame (`readFramePlayerstate`).
    pub fn read_frame_playerstate(
        &mut self,
        reader: &mut MsgReader<'_>,
        from: &PlayerState,
    ) -> Result<PlayerState, VariantError> {
        let mut flags = u32::from(reader.word()?);
        if self.features.revision >= 1026 && (flags & 0x8000) != 0 {
            flags |= u32::from(reader.byte()?) << 16;
        }
        self.read_player_state_body(reader, from, flags, self.frame_extraflags)
    }

    /// Write a batched move (`writeBatchMove`).
    pub fn write_batch_move(
        writer: &mut MsgWriter,
        lastframe: Option<i32>,
        frames: &[BatchMoveFrame],
    ) -> Result<(), VariantError> {
        if frames.is_empty()
            || frames.len() >= MAX_BATCH_MOVE_FRAMES
            || frames.iter().any(|frame| frame.cmds.len() > MAX_BATCH_MOVE_CMDS - 1)
        {
            return Err(VariantError::BatchBounds);
        }
        if let Some(lastframe) = lastframe {
            writer.write_long(lastframe)?;
        }
        let lightlevel = frames
            .last()
            .and_then(|frame| frame.cmds.last())
            .map_or(0, |cmd| cmd.lightlevel);
        writer.write_byte(lightlevel)?;
        let mut bits = BatchBitWriter::new(writer);
        write_batch_move_frames(&mut bits, frames, encode_q2pro_batch_cmd)
    }

    /// Read a batched move (`readBatchMove`); `opcode_extra` carries `num_dups`.
    pub fn read_batch_move(
        reader: &mut MsgReader<'_>,
        nodelta: bool,
        opcode_extra: u8,
    ) -> Result<BatchMove, VariantError> {
        let num_dups = usize::from(opcode_extra);
        if num_dups >= MAX_BATCH_MOVE_FRAMES - 1 {
            return Err(VariantError::BatchNumDups(opcode_extra));
        }
        let lastframe = if nodelta { -1 } else { reader.long()? };
        let lightlevel = reader.byte()?;
        let _ = lightlevel;
        let mut bits = BatchBitReader::new(reader);
        let frames = read_batch_move_frames(&mut bits, num_dups, decode_q2pro_batch_cmd)?;
        Ok(BatchMove {
            lastframe,
            num_dups,
            frames,
        })
    }
}

/// Decode one Q2Pro batch command.
fn decode_q2pro_batch_cmd(
    reader: &mut BatchBitReader<'_, '_>,
    prev: Option<&Usercmd>,
) -> Result<Usercmd, VariantError> {
    let mut cmd = seed_from_prev(prev);
    if reader.read_unsigned(1)? == 0 {
        return Ok(cmd);
    }
    let bits = reader.read_unsigned(8)?;
    let prev_angle = |axis: usize| prev.map_or(0, |cmd| cmd.angles[axis]);
    if (bits & protocol::CM_ANGLE1) != 0 {
        cmd.angles[0] = read_batch_move_angle(reader, prev_angle(0))?;
    }
    if (bits & protocol::CM_ANGLE2) != 0 {
        cmd.angles[1] = read_batch_move_angle(reader, prev_angle(1))?;
    }
    if (bits & protocol::CM_ANGLE3) != 0 {
        cmd.angles[2] = reader.read_signed(16)? as i16;
    }
    if (bits & protocol::CM_FORWARD) != 0 {
        cmd.forwardmove = reader.read_signed(10)? as i16;
    }
    if (bits & protocol::CM_SIDE) != 0 {
        cmd.sidemove = reader.read_signed(10)? as i16;
    }
    if (bits & protocol::CM_UP) != 0 {
        cmd.upmove = reader.read_signed(10)? as i16;
    }
    if (bits & protocol::CM_BUTTONS) != 0 {
        let raw = reader.read_unsigned(3)?;
        cmd.buttons = ((raw & 3) | ((raw & 4) << 5)) as u8;
    }
    cmd.msec = if (bits & protocol::CM_IMPULSE) != 0 {
        reader.read_unsigned(8)? as u8
    } else {
        prev.map_or(0, |cmd| cmd.msec)
    };
    Ok(cmd)
}

/// Encode one Q2Pro batch command.
fn encode_q2pro_batch_cmd(
    writer: &mut BatchBitWriter<'_>,
    cmd: &Usercmd,
    prev: Option<&Usercmd>,
) -> Result<(), VariantError> {
    let base = Usercmd::default();
    let from = prev.unwrap_or(&base);
    let mut bits = 0u32;
    if cmd.angles[0] != from.angles[0] {
        bits |= protocol::CM_ANGLE1;
    }
    if cmd.angles[1] != from.angles[1] {
        bits |= protocol::CM_ANGLE2;
    }
    if cmd.angles[2] != from.angles[2] {
        bits |= protocol::CM_ANGLE3;
    }
    if cmd.forwardmove != from.forwardmove {
        bits |= protocol::CM_FORWARD;
    }
    if cmd.sidemove != from.sidemove {
        bits |= protocol::CM_SIDE;
    }
    if cmd.upmove != from.upmove {
        bits |= protocol::CM_UP;
    }
    if cmd.buttons != from.buttons {
        bits |= protocol::CM_BUTTONS;
    }
    if cmd.msec != from.msec {
        bits |= protocol::CM_IMPULSE;
    }
    writer.write_unsigned(u32::from(bits != 0), 1)?;
    if bits == 0 {
        return Ok(());
    }
    writer.write_unsigned(bits, 8)?;
    for axis in 0..2 {
        if (bits & (protocol::CM_ANGLE1 << axis)) != 0 {
            let delta = i32::from(cmd.angles[axis]) - i32::from(from.angles[axis]);
            if (-128..=127).contains(&delta) {
                writer.write_unsigned(1, 1)?;
                writer.write_signed(delta, 8)?;
            } else {
                writer.write_unsigned(0, 1)?;
                writer.write_signed(i32::from(cmd.angles[axis]), 16)?;
            }
        }
    }
    if (bits & protocol::CM_ANGLE3) != 0 {
        writer.write_signed(i32::from(cmd.angles[2]), 16)?;
    }
    if (bits & protocol::CM_FORWARD) != 0 {
        writer.write_signed(i32::from(cmd.forwardmove), 10)?;
    }
    if (bits & protocol::CM_SIDE) != 0 {
        writer.write_signed(i32::from(cmd.sidemove), 10)?;
    }
    if (bits & protocol::CM_UP) != 0 {
        writer.write_signed(i32::from(cmd.upmove), 10)?;
    }
    if (bits & protocol::CM_BUTTONS) != 0 {
        writer.write_unsigned(u32::from(cmd.buttons & 3) | (u32::from((cmd.buttons >> 5) & 4)), 3)?;
    }
    if (bits & protocol::CM_IMPULSE) != 0 {
        writer.write_unsigned(u32::from(cmd.msec), 8)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Rerelease (q2repro)
// ---------------------------------------------------------------------------

const RR_U_ANGLE16: u32 = 1 << 13;
const RR_U_MODEL16: u32 = 1 << 28;
const RR_U_MOREFX8: u32 = 1 << 29;
const RR_U_ALPHA: u32 = 1 << 30;
const RR_HI_SCALE: u8 = 1;
const RR_HI_MOREFX16: u8 = 2;
const RR_SOUND_FLAG_VOLUME: u16 = 1 << 14;
const RR_SOUND_FLAG_ATTENUATION: u16 = 1 << 15;
const RR_EPS_GUNRATE: u8 = 1 << 7;
const RR_GUNINDEX_BITS: u32 = 13;
const RR_GUNINDEX_MASK: u16 = (1 << RR_GUNINDEX_BITS) - 1;
const RR_VIEWOFFSET_SCALE: f64 = 16.0;
const RR_GUNOFFSET_SCALE: f64 = 512.0;
const RR_KICK_ANGLE_SCALE: f64 = 1024.0;
const RR_GUNANGLE_SCALE: f64 = 4096.0;
const RR_ENCODE_LOOP_NONE: u8 = 192;

/// JavaScript `Math.round` (half up) for wire floats.
fn js_round(value: f32) -> f32 {
    (value + 0.5).floor()
}

/// Clamp to the `i16` range.
fn clamp_int16(value: i32) -> i16 {
    value.clamp(-32768, 32767) as i16
}

/// Encode a fixed-point 16-bit value (`encodeFixed16`).
fn encode_fixed16(value: f64, scale: f64) -> i16 {
    clamp_int16(scaled_trunc(value, scale))
}

/// Pick a rerelease field width: 0 = byte, 1 = short, 2 = long (`widthOf`).
fn width_of(value: u32, uint16_safe: bool) -> u8 {
    let mask32 = if uint16_safe { 0xffff_0000 } else { 0xffff_8000 };
    if (value & mask32) != 0 {
        2
    } else if (value & 0xff00) != 0 {
        1
    } else {
        0
    }
}

/// Convert a float pmove component to eighths (`pmFloatToShort`).
fn pm_float_to_short(value: f32) -> i16 {
    clamp_int16(js_round(value * 8.0) as i32)
}

/// Encode entity alpha (`encodeAlpha`).
fn encode_alpha(value: f64) -> u8 {
    if value == 0.0 {
        0
    } else {
        scaled_trunc(value, 255.0).clamp(1, 255) as u8
    }
}

/// Decode entity alpha (`decodeAlpha`).
fn decode_alpha(byte: u8) -> f64 {
    if byte == 0 {
        0.0
    } else {
        f64::from(byte) / 255.0
    }
}

/// Encode entity scale (`encodeScale`).
fn encode_scale(value: f64) -> u8 {
    if value == 0.0 {
        0
    } else {
        scaled_trunc(value, 16.0).clamp(1, 255) as u8
    }
}

/// Decode entity scale (`decodeScale`).
fn decode_scale(byte: u8) -> f64 {
    if byte == 0 {
        0.0
    } else {
        f64::from(byte) / 16.0
    }
}

/// Encode looping-sound volume (`encodeLoopVolume`).
fn encode_loop_volume(value: f64) -> u8 {
    if value == 0.0 {
        return 0;
    }
    let mut encoded = scaled_trunc(value, 255.0).clamp(0, 255);
    if encoded == 255 {
        encoded = 0;
    }
    encoded as u8
}

/// Decode looping-sound volume (`decodeLoopVolume`).
fn decode_loop_volume(byte: u8) -> f64 {
    if byte == 0 {
        0.0
    } else {
        f64::from(byte) / 255.0
    }
}

/// Encode looping-sound attenuation (`encodeLoopAttenuation`).
fn encode_loop_attenuation(value: f64) -> u8 {
    if value == -1.0 {
        return RR_ENCODE_LOOP_NONE;
    }
    let mut encoded = scaled_trunc(value, 64.0).clamp(0, 255);
    if encoded == i32::from(RR_ENCODE_LOOP_NONE) {
        encoded = 0;
    }
    encoded as u8
}

/// Decode looping-sound attenuation (`decodeLoopAttenuation`).
fn decode_loop_attenuation(byte: u8) -> f64 {
    if byte == RR_ENCODE_LOOP_NONE {
        -1.0
    } else {
        f64::from(byte) / 64.0
    }
}

/// Rerelease server data (`ServerDataParamsT` / `ServerDataReadResultT`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RereleaseServerData {
    /// Server count.
    pub servercount: i32,
    /// Attract loop.
    pub attractloop: bool,
    /// Game directory.
    pub gamedir: String,
    /// Client number.
    pub clientnum: i16,
    /// Level name.
    pub levelname: String,
    /// Protocol revision.
    pub protocol_revision: u16,
    /// Server state.
    pub server_state: u8,
    /// Negotiated wire flags.
    pub wire_flags: u16,
    /// Server frame rate.
    pub server_fps: u8,
}

/// Encoded rerelease player-state delta.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RereleasePlayerDelta {
    /// Main flags.
    pub flags: u16,
    /// Extra flags.
    pub extraflags: u8,
    /// Changed stat bits.
    pub statbits: u64,
    /// Changed blend component bits.
    pub blend_bits: u8,
    /// Changed damage-blend component bits.
    pub damage_blend_bits: u8,
}

/// Rerelease fog data (`SvcFogDataT`).
#[derive(Debug, Clone, PartialEq)]
pub struct FogData {
    /// Field bits.
    pub bits: u16,
    /// Density.
    pub density: f32,
    /// Sky factor.
    pub skyfactor: u8,
    /// Red.
    pub red: u8,
    /// Green.
    pub green: u8,
    /// Blue.
    pub blue: u8,
    /// Blend time.
    pub time: u16,
    /// Height-fog falloff.
    pub hf_falloff: f32,
    /// Height-fog density.
    pub hf_density: f32,
    /// Height-fog start color.
    pub hf_start: [u8; 3],
    /// Height-fog start distance.
    pub hf_start_dist: i32,
    /// Height-fog end color.
    pub hf_end: [u8; 3],
    /// Height-fog end distance.
    pub hf_end_dist: i32,
}

/// Rerelease fog field bits (`SvcFogDataBitsT`).
pub mod fog_bits {
    /// Density + sky factor follow.
    pub const DENSITY: u16 = 1 << 0;
    /// Red follows.
    pub const R: u16 = 1 << 1;
    /// Green follows.
    pub const G: u16 = 1 << 2;
    /// Blue follows.
    pub const B: u16 = 1 << 3;
    /// Blend time follows.
    pub const TIME: u16 = 1 << 4;
    /// Height-fog falloff follows.
    pub const HEIGHTFOG_FALLOFF: u16 = 1 << 5;
    /// Height-fog density follows.
    pub const HEIGHTFOG_DENSITY: u16 = 1 << 6;
    /// A second bits byte follows.
    pub const MORE_BITS: u16 = 1 << 7;
    /// Height-fog start red follows.
    pub const HEIGHTFOG_START_R: u16 = 1 << 8;
    /// Height-fog start green follows.
    pub const HEIGHTFOG_START_G: u16 = 1 << 9;
    /// Height-fog start blue follows.
    pub const HEIGHTFOG_START_B: u16 = 1 << 10;
    /// Height-fog start distance follows.
    pub const HEIGHTFOG_START_DIST: u16 = 1 << 11;
    /// Height-fog end red follows.
    pub const HEIGHTFOG_END_R: u16 = 1 << 12;
    /// Height-fog end green follows.
    pub const HEIGHTFOG_END_G: u16 = 1 << 13;
    /// Height-fog end blue follows.
    pub const HEIGHTFOG_END_B: u16 = 1 << 14;
    /// Height-fog end distance follows.
    pub const HEIGHTFOG_END_DIST: u16 = 1 << 15;
}

/// Read rerelease fog data (`readFog`).
pub fn read_fog(reader: &mut MsgReader<'_>) -> Result<FogData, MsgError> {
    let mut bits = u16::from(reader.byte()?);
    if (bits & fog_bits::MORE_BITS) != 0 {
        bits |= u16::from(reader.byte()?) << 8;
    }
    let mut fog = FogData {
        bits,
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
    };
    if (bits & fog_bits::DENSITY) != 0 {
        fog.density = reader.float()?;
        fog.skyfactor = reader.byte()?;
    }
    if (bits & fog_bits::R) != 0 {
        fog.red = reader.byte()?;
    }
    if (bits & fog_bits::G) != 0 {
        fog.green = reader.byte()?;
    }
    if (bits & fog_bits::B) != 0 {
        fog.blue = reader.byte()?;
    }
    if (bits & fog_bits::TIME) != 0 {
        fog.time = reader.word()?;
    }
    if (bits & fog_bits::HEIGHTFOG_FALLOFF) != 0 {
        fog.hf_falloff = reader.float()?;
    }
    if (bits & fog_bits::HEIGHTFOG_DENSITY) != 0 {
        fog.hf_density = reader.float()?;
    }
    if (bits & fog_bits::HEIGHTFOG_START_R) != 0 {
        fog.hf_start[0] = reader.byte()?;
    }
    if (bits & fog_bits::HEIGHTFOG_START_G) != 0 {
        fog.hf_start[1] = reader.byte()?;
    }
    if (bits & fog_bits::HEIGHTFOG_START_B) != 0 {
        fog.hf_start[2] = reader.byte()?;
    }
    if (bits & fog_bits::HEIGHTFOG_START_DIST) != 0 {
        fog.hf_start_dist = reader.long()?;
    }
    if (bits & fog_bits::HEIGHTFOG_END_R) != 0 {
        fog.hf_end[0] = reader.byte()?;
    }
    if (bits & fog_bits::HEIGHTFOG_END_G) != 0 {
        fog.hf_end[1] = reader.byte()?;
    }
    if (bits & fog_bits::HEIGHTFOG_END_B) != 0 {
        fog.hf_end[2] = reader.byte()?;
    }
    if (bits & fog_bits::HEIGHTFOG_END_DIST) != 0 {
        fog.hf_end_dist = reader.long()?;
    }
    Ok(fog)
}

/// Rerelease protocol codec (`Q2REPRO_CODEC` / `Q2REPRO_CLASSIC_CODEC`).
#[derive(Debug, Clone)]
pub struct RereleaseCodec {
    classic: bool,
    frame_extraflags: u8,
}

impl RereleaseCodec {
    /// Build a rerelease (`1038`) or classic-compatible (`4038`) codec.
    #[must_use]
    pub fn new(classic: bool) -> Self {
        Self {
            classic,
            frame_extraflags: 0,
        }
    }

    /// Whether this is the classic-compatible flavor.
    #[must_use]
    pub fn is_classic(&self) -> bool {
        self.classic
    }

    /// Write server data (`writeServerData`).
    pub fn write_server_data(&self, writer: &mut MsgWriter, params: &RereleaseServerData) -> Result<(), MsgError> {
        let version = if self.classic {
            protocol::PROTOCOL_VERSION_RERELEASE_CLASSIC
        } else {
            protocol::PROTOCOL_VERSION_RERELEASE
        };
        writer.write_byte(protocol::Svc::Serverdata as u8)?;
        writer.write_long(version as i32)?;
        writer.write_long(params.servercount)?;
        writer.write_byte(u8::from(params.attractloop))?;
        writer.write_string(&params.gamedir)?;
        writer.write_short(params.clientnum)?;
        writer.write_string(&params.levelname)?;
        writer.write_short(params.protocol_revision as i16)?;
        writer.write_byte(params.server_state)?;
        writer.write_short(params.wire_flags as i16)?;
        writer.write_byte(params.server_fps)
    }

    /// Read server data (`readServerData`).
    pub fn read_server_data(reader: &mut MsgReader<'_>) -> Result<RereleaseServerData, MsgError> {
        let servercount = reader.long()?;
        let attractloop = reader.byte()? != 0;
        let gamedir = reader.string(2047);
        let clientnum = reader.short()?;
        let levelname = reader.string(2047);
        let protocol_revision = reader.word()?;
        let server_state = reader.byte()?;
        let wire_flags = reader.word()?;
        let server_fps = reader.byte()?;
        Ok(RereleaseServerData {
            servercount,
            attractloop,
            gamedir,
            clientnum,
            levelname,
            protocol_revision,
            server_state,
            wire_flags,
            server_fps,
        })
    }

    /// Write a delta entity; `Ok(false)` writes nothing when unchanged and not forced.
    pub fn write_delta_entity(
        writer: &mut MsgWriter,
        from: &EntityState,
        to: &EntityState,
        force: bool,
        newentity: bool,
    ) -> Result<bool, VariantError> {
        let mut lo = 0u32;
        let mut hi = 0u8;
        if to.origin[0] != from.origin[0] {
            lo |= protocol::U_ORIGIN1;
        }
        if to.origin[1] != from.origin[1] {
            lo |= protocol::U_ORIGIN2;
        }
        if to.origin[2] != from.origin[2] {
            lo |= protocol::U_ORIGIN3;
        }
        let to_angle = [
            angle_to_short(to.angles[0]),
            angle_to_short(to.angles[1]),
            angle_to_short(to.angles[2]),
        ];
        let from_angle = [
            angle_to_short(from.angles[0]),
            angle_to_short(from.angles[1]),
            angle_to_short(from.angles[2]),
        ];
        if to_angle[0] != from_angle[0] {
            lo |= protocol::U_ANGLE1;
        }
        if to_angle[1] != from_angle[1] {
            lo |= protocol::U_ANGLE2;
        }
        if to_angle[2] != from_angle[2] {
            lo |= protocol::U_ANGLE3;
        }
        if (lo & (protocol::U_ANGLE1 | protocol::U_ANGLE2 | protocol::U_ANGLE3)) != 0 {
            lo |= RR_U_ANGLE16;
        }
        if to.skinnum != from.skinnum {
            match width_of(to.skinnum as u32, true) {
                2 => lo |= protocol::U_SKIN8 | protocol::U_SKIN16,
                1 => lo |= protocol::U_SKIN16,
                _ => lo |= protocol::U_SKIN8,
            }
        }
        if to.frame != from.frame {
            lo |= if to.frame >= 256 {
                protocol::U_FRAME16
            } else {
                protocol::U_FRAME8
            };
        }
        if to.effects != from.effects {
            match width_of(to.effects as u32, true) {
                2 => lo |= protocol::U_EFFECTS8 | protocol::U_EFFECTS16,
                1 => lo |= protocol::U_EFFECTS16,
                _ => lo |= protocol::U_EFFECTS8,
            }
        }
        if to.morefx != from.morefx {
            match width_of(to.morefx as u32, true) {
                0 => lo |= RR_U_MOREFX8,
                1 => hi |= RR_HI_MOREFX16,
                _ => {
                    lo |= RR_U_MOREFX8;
                    hi |= RR_HI_MOREFX16;
                }
            }
        }
        if to.renderfx != from.renderfx {
            match width_of(to.renderfx as u32, true) {
                2 => lo |= protocol::U_RENDERFX8 | protocol::U_RENDERFX16,
                1 => lo |= protocol::U_RENDERFX16,
                _ => lo |= protocol::U_RENDERFX8,
            }
        }
        if to.solid != from.solid {
            lo |= protocol::U_SOLID;
        }
        if to.event != 0 {
            lo |= protocol::U_EVENT;
        }
        if to.modelindex != from.modelindex {
            lo |= protocol::U_MODEL;
        }
        if to.modelindex2 != from.modelindex2 {
            lo |= protocol::U_MODEL2;
        }
        if to.modelindex3 != from.modelindex3 {
            lo |= protocol::U_MODEL3;
        }
        if to.modelindex4 != from.modelindex4 {
            lo |= protocol::U_MODEL4;
        }
        if ((lo & protocol::U_MODEL) != 0 && to.modelindex > 255)
            || ((lo & protocol::U_MODEL2) != 0 && to.modelindex2 > 255)
            || ((lo & protocol::U_MODEL3) != 0 && to.modelindex3 > 255)
            || ((lo & protocol::U_MODEL4) != 0 && to.modelindex4 > 255)
        {
            lo |= RR_U_MODEL16;
        }
        let loop_volume_changed = to.loop_volume != from.loop_volume;
        let loop_attenuation_changed = to.loop_attenuation != from.loop_attenuation;
        if to.sound != from.sound {
            lo |= protocol::U_SOUND;
        }
        if newentity || (to.renderfx & RF_BEAM) != 0 {
            lo |= protocol::U_OLDORIGIN;
        }
        if to.alpha != from.alpha {
            lo |= RR_U_ALPHA;
        }
        if to.scale != from.scale {
            hi |= RR_HI_SCALE;
        }
        if lo == 0 && hi == 0 && !force {
            return Ok(false);
        }
        write_entity_bits_wide(writer, lo, hi, to.number)?;
        if (lo & RR_U_MODEL16) != 0 {
            if (lo & protocol::U_MODEL) != 0 {
                writer.write_short(to.modelindex as i16)?;
            }
            if (lo & protocol::U_MODEL2) != 0 {
                writer.write_short(to.modelindex2 as i16)?;
            }
            if (lo & protocol::U_MODEL3) != 0 {
                writer.write_short(to.modelindex3 as i16)?;
            }
            if (lo & protocol::U_MODEL4) != 0 {
                writer.write_short(to.modelindex4 as i16)?;
            }
        } else {
            if (lo & protocol::U_MODEL) != 0 {
                writer.write_byte(to.modelindex as u8)?;
            }
            if (lo & protocol::U_MODEL2) != 0 {
                writer.write_byte(to.modelindex2 as u8)?;
            }
            if (lo & protocol::U_MODEL3) != 0 {
                writer.write_byte(to.modelindex3 as u8)?;
            }
            if (lo & protocol::U_MODEL4) != 0 {
                writer.write_byte(to.modelindex4 as u8)?;
            }
        }
        if (lo & protocol::U_FRAME16) != 0 {
            writer.write_short(to.frame as i16)?;
        } else if (lo & protocol::U_FRAME8) != 0 {
            writer.write_byte(to.frame as u8)?;
        }
        if (lo & (protocol::U_SKIN8 | protocol::U_SKIN16)) == (protocol::U_SKIN8 | protocol::U_SKIN16) {
            writer.write_long(to.skinnum)?;
        } else if (lo & protocol::U_SKIN16) != 0 {
            writer.write_short(to.skinnum as i16)?;
        } else if (lo & protocol::U_SKIN8) != 0 {
            writer.write_byte(to.skinnum as u8)?;
        }
        if (lo & (protocol::U_EFFECTS8 | protocol::U_EFFECTS16)) == (protocol::U_EFFECTS8 | protocol::U_EFFECTS16) {
            writer.write_long(to.effects)?;
        } else if (lo & protocol::U_EFFECTS16) != 0 {
            writer.write_short(to.effects as i16)?;
        } else if (lo & protocol::U_EFFECTS8) != 0 {
            writer.write_byte(to.effects as u8)?;
        }
        if (lo & (protocol::U_RENDERFX8 | protocol::U_RENDERFX16)) == (protocol::U_RENDERFX8 | protocol::U_RENDERFX16) {
            writer.write_long(to.renderfx)?;
        } else if (lo & protocol::U_RENDERFX16) != 0 {
            writer.write_short(to.renderfx as i16)?;
        } else if (lo & protocol::U_RENDERFX8) != 0 {
            writer.write_byte(to.renderfx as u8)?;
        }
        if (lo & protocol::U_ORIGIN1) != 0 {
            writer.write_float(to.origin[0] as f32)?;
        }
        if (lo & protocol::U_ORIGIN2) != 0 {
            writer.write_float(to.origin[1] as f32)?;
        }
        if (lo & protocol::U_ORIGIN3) != 0 {
            writer.write_float(to.origin[2] as f32)?;
        }
        if (lo & protocol::U_ANGLE1) != 0 {
            writer.write_short(to_angle[0] as i16)?;
        }
        if (lo & protocol::U_ANGLE2) != 0 {
            writer.write_short(to_angle[1] as i16)?;
        }
        if (lo & protocol::U_ANGLE3) != 0 {
            writer.write_short(to_angle[2] as i16)?;
        }
        if (lo & protocol::U_OLDORIGIN) != 0 {
            writer.write_float(to.old_origin[0] as f32)?;
            writer.write_float(to.old_origin[1] as f32)?;
            writer.write_float(to.old_origin[2] as f32)?;
        }
        if (lo & protocol::U_SOUND) != 0 {
            let mut sound_word = to.sound & 0x3fff;
            if loop_attenuation_changed {
                sound_word |= RR_SOUND_FLAG_ATTENUATION;
            }
            if loop_volume_changed {
                sound_word |= RR_SOUND_FLAG_VOLUME;
            }
            writer.write_short(sound_word as i16)?;
            if (sound_word & RR_SOUND_FLAG_VOLUME) != 0 {
                writer.write_byte(encode_loop_volume(to.loop_volume))?;
            }
            if (sound_word & RR_SOUND_FLAG_ATTENUATION) != 0 {
                writer.write_byte(encode_loop_attenuation(to.loop_attenuation))?;
            }
        }
        if (lo & protocol::U_EVENT) != 0 {
            writer.write_byte(to.event)?;
        }
        if (lo & protocol::U_SOLID) != 0 {
            writer.write_long(to.solid as i32)?;
        }
        if ((hi & RR_HI_MOREFX16) != 0) && ((lo & RR_U_MOREFX8) != 0) {
            writer.write_long(to.morefx)?;
        } else if (hi & RR_HI_MOREFX16) != 0 {
            writer.write_short(to.morefx as i16)?;
        } else if (lo & RR_U_MOREFX8) != 0 {
            writer.write_byte(to.morefx as u8)?;
        }
        if (lo & RR_U_ALPHA) != 0 {
            writer.write_byte(encode_alpha(to.alpha))?;
        }
        if (hi & RR_HI_SCALE) != 0 {
            writer.write_byte(encode_scale(to.scale))?;
        }
        Ok(true)
    }

    /// Read a delta entity body (`readDeltaEntity`).
    pub fn read_delta_entity(
        reader: &mut MsgReader<'_>,
        from: &EntityState,
        number: u16,
        bits: WideEntityBits,
    ) -> Result<EntityState, VariantError> {
        let mut to = from.clone();
        to.old_origin = from.origin;
        to.number = number;
        let lo = bits.lo;
        let hi = bits.hi;
        let model16 = (lo & RR_U_MODEL16) != 0;
        if (lo & protocol::U_MODEL) != 0 {
            to.modelindex = if model16 {
                reader.word()?
            } else {
                u16::from(reader.byte()?)
            };
        }
        if (lo & protocol::U_MODEL2) != 0 {
            to.modelindex2 = if model16 {
                reader.word()?
            } else {
                u16::from(reader.byte()?)
            };
        }
        if (lo & protocol::U_MODEL3) != 0 {
            to.modelindex3 = if model16 {
                reader.word()?
            } else {
                u16::from(reader.byte()?)
            };
        }
        if (lo & protocol::U_MODEL4) != 0 {
            to.modelindex4 = if model16 {
                reader.word()?
            } else {
                u16::from(reader.byte()?)
            };
        }
        if (lo & protocol::U_FRAME16) != 0 {
            to.frame = i32::from(reader.word()?);
        } else if (lo & protocol::U_FRAME8) != 0 {
            to.frame = i32::from(reader.byte()?);
        }
        if (lo & (protocol::U_SKIN8 | protocol::U_SKIN16)) == (protocol::U_SKIN8 | protocol::U_SKIN16) {
            to.skinnum = reader.long()?;
        } else if (lo & protocol::U_SKIN16) != 0 {
            to.skinnum = i32::from(reader.word()?);
        } else if (lo & protocol::U_SKIN8) != 0 {
            to.skinnum = i32::from(reader.byte()?);
        }
        if (lo & (protocol::U_EFFECTS8 | protocol::U_EFFECTS16)) == (protocol::U_EFFECTS8 | protocol::U_EFFECTS16) {
            to.effects = reader.long()?;
        } else if (lo & protocol::U_EFFECTS16) != 0 {
            to.effects = i32::from(reader.word()?);
        } else if (lo & protocol::U_EFFECTS8) != 0 {
            to.effects = i32::from(reader.byte()?);
        }
        if (lo & (protocol::U_RENDERFX8 | protocol::U_RENDERFX16)) == (protocol::U_RENDERFX8 | protocol::U_RENDERFX16) {
            to.renderfx = reader.long()?;
        } else if (lo & protocol::U_RENDERFX16) != 0 {
            to.renderfx = i32::from(reader.word()?);
        } else if (lo & protocol::U_RENDERFX8) != 0 {
            to.renderfx = i32::from(reader.byte()?);
        }
        if (lo & protocol::U_ORIGIN1) != 0 {
            to.origin[0] = f64::from(reader.float()?);
        }
        if (lo & protocol::U_ORIGIN2) != 0 {
            to.origin[1] = f64::from(reader.float()?);
        }
        if (lo & protocol::U_ORIGIN3) != 0 {
            to.origin[2] = f64::from(reader.float()?);
        }
        if (lo & RR_U_ANGLE16) != 0 {
            if (lo & protocol::U_ANGLE1) != 0 {
                to.angles[0] = short_to_angle(reader.short()?);
            }
            if (lo & protocol::U_ANGLE2) != 0 {
                to.angles[1] = short_to_angle(reader.short()?);
            }
            if (lo & protocol::U_ANGLE3) != 0 {
                to.angles[2] = short_to_angle(reader.short()?);
            }
        } else {
            if (lo & protocol::U_ANGLE1) != 0 {
                to.angles[0] = f64::from(reader.byte()?) * (360.0 / 256.0);
            }
            if (lo & protocol::U_ANGLE2) != 0 {
                to.angles[1] = f64::from(reader.byte()?) * (360.0 / 256.0);
            }
            if (lo & protocol::U_ANGLE3) != 0 {
                to.angles[2] = f64::from(reader.byte()?) * (360.0 / 256.0);
            }
        }
        if (lo & protocol::U_OLDORIGIN) != 0 {
            to.old_origin[0] = f64::from(reader.float()?);
            to.old_origin[1] = f64::from(reader.float()?);
            to.old_origin[2] = f64::from(reader.float()?);
        }
        if (lo & protocol::U_SOUND) != 0 {
            let sound_word = reader.word()?;
            to.sound = sound_word & 0x3fff;
            if (sound_word & RR_SOUND_FLAG_VOLUME) != 0 {
                to.loop_volume = decode_loop_volume(reader.byte()?);
            }
            if (sound_word & RR_SOUND_FLAG_ATTENUATION) != 0 {
                to.loop_attenuation = decode_loop_attenuation(reader.byte()?);
            }
        }
        if (lo & protocol::U_EVENT) != 0 {
            to.event = reader.byte()?;
        } else {
            to.event = 0;
        }
        if (lo & protocol::U_SOLID) != 0 {
            to.solid = reader.long()? as u32;
        }
        if ((hi & RR_HI_MOREFX16) != 0) && ((lo & RR_U_MOREFX8) != 0) {
            to.morefx = reader.long()?;
        } else if (hi & RR_HI_MOREFX16) != 0 {
            to.morefx = i32::from(reader.word()?);
        } else if (lo & RR_U_MOREFX8) != 0 {
            to.morefx = i32::from(reader.byte()?);
        }
        if (lo & RR_U_ALPHA) != 0 {
            to.alpha = decode_alpha(reader.byte()?);
        }
        if (hi & RR_HI_SCALE) != 0 {
            to.scale = decode_scale(reader.byte()?);
        }
        Ok(to)
    }

    /// Write a spawn baseline (`writeSpawnBaseline`).
    pub fn write_spawn_baseline(writer: &mut MsgWriter, base: &EntityState) -> Result<bool, VariantError> {
        writer.write_byte(protocol::Svc::Spawnbaseline as u8)?;
        Self::write_delta_entity(writer, &EntityState::default(), base, true, true)
    }

    /// Encode a rerelease player-state delta (`encodePlayerStateDelta`).
    #[must_use]
    pub fn encode_player_state(from: &PlayerState, to: &PlayerState) -> RereleasePlayerDelta {
        let mut flags = 0u16;
        let mut extraflags = 0u8;
        if to.pmove.pm_type != from.pmove.pm_type {
            flags |= protocol::PS_M_TYPE as u16;
        }
        if to.pmove.origin_f[0] != from.pmove.origin_f[0] || to.pmove.origin_f[1] != from.pmove.origin_f[1] {
            flags |= protocol::PS_M_ORIGIN as u16;
        }
        if to.pmove.origin_f[2] != from.pmove.origin_f[2] {
            extraflags |= protocol::EPS_M_ORIGIN2 as u8;
        }
        if to.pmove.velocity_f[0] != from.pmove.velocity_f[0] || to.pmove.velocity_f[1] != from.pmove.velocity_f[1] {
            flags |= protocol::PS_M_VELOCITY as u16;
        }
        if to.pmove.velocity_f[2] != from.pmove.velocity_f[2] {
            extraflags |= protocol::EPS_M_VELOCITY2 as u8;
        }
        if to.pmove.pm_time != from.pmove.pm_time {
            flags |= protocol::PS_M_TIME as u16;
        }
        if to.pmove.pm_flags != from.pmove.pm_flags {
            flags |= protocol::PS_M_FLAGS as u16;
        }
        if to.pmove.gravity != from.pmove.gravity {
            flags |= protocol::PS_M_GRAVITY as u16;
        }
        if to.pmove.delta_angles != from.pmove.delta_angles {
            flags |= protocol::PS_M_DELTA_ANGLES as u16;
        }
        if to.pmove.viewheight != from.pmove.viewheight {
            flags |= protocol::PS_RR_VIEWHEIGHT as u16;
        }
        let to_viewoffset = [
            encode_fixed16(to.viewoffset[0], RR_VIEWOFFSET_SCALE),
            encode_fixed16(to.viewoffset[1], RR_VIEWOFFSET_SCALE),
            encode_fixed16(to.viewoffset[2], RR_VIEWOFFSET_SCALE),
        ];
        let from_viewoffset = [
            encode_fixed16(from.viewoffset[0], RR_VIEWOFFSET_SCALE),
            encode_fixed16(from.viewoffset[1], RR_VIEWOFFSET_SCALE),
            encode_fixed16(from.viewoffset[2], RR_VIEWOFFSET_SCALE),
        ];
        if to_viewoffset != from_viewoffset {
            flags |= protocol::PS_VIEWOFFSET as u16;
        }
        let to_viewangle = [
            angle_to_short(to.viewangles[0]),
            angle_to_short(to.viewangles[1]),
            angle_to_short(to.viewangles[2]),
        ];
        let from_viewangle = [
            angle_to_short(from.viewangles[0]),
            angle_to_short(from.viewangles[1]),
            angle_to_short(from.viewangles[2]),
        ];
        if to_viewangle[0] != from_viewangle[0] || to_viewangle[1] != from_viewangle[1] {
            flags |= protocol::PS_VIEWANGLES as u16;
        }
        if to_viewangle[2] != from_viewangle[2] {
            extraflags |= protocol::EPS_VIEWANGLE2 as u8;
        }
        let to_kick = [
            encode_fixed16(to.kick_angles[0], RR_KICK_ANGLE_SCALE),
            encode_fixed16(to.kick_angles[1], RR_KICK_ANGLE_SCALE),
            encode_fixed16(to.kick_angles[2], RR_KICK_ANGLE_SCALE),
        ];
        let from_kick = [
            encode_fixed16(from.kick_angles[0], RR_KICK_ANGLE_SCALE),
            encode_fixed16(from.kick_angles[1], RR_KICK_ANGLE_SCALE),
            encode_fixed16(from.kick_angles[2], RR_KICK_ANGLE_SCALE),
        ];
        if to_kick != from_kick {
            flags |= protocol::PS_KICKANGLES as u16;
        }
        let mut blend_bits = 0u8;
        for i in 0..4 {
            if scaled_trunc(to.blend[i], 255.0) as u8 != scaled_trunc(from.blend[i], 255.0) as u8 {
                blend_bits |= 1 << i;
            }
        }
        let mut damage_blend_bits = 0u8;
        for i in 0..4 {
            if scaled_trunc(to.damage_blend[i], 255.0) as u8 != scaled_trunc(from.damage_blend[i], 255.0) as u8 {
                damage_blend_bits |= 1 << i;
            }
        }
        if blend_bits != 0 || damage_blend_bits != 0 {
            flags |= protocol::PS_BLEND as u16;
        }
        if to.fov != from.fov {
            flags |= protocol::PS_FOV as u16;
        }
        if to.rdflags != from.rdflags {
            flags |= protocol::PS_RDFLAGS as u16;
        }
        if to.gunindex != from.gunindex || to.gunskin != from.gunskin {
            flags |= protocol::PS_WEAPONINDEX as u16;
        }
        if to.gunframe != from.gunframe {
            flags |= protocol::PS_WEAPONFRAME as u16;
        }
        let to_gunoffset = [
            encode_fixed16(to.gunoffset[0], RR_GUNOFFSET_SCALE),
            encode_fixed16(to.gunoffset[1], RR_GUNOFFSET_SCALE),
            encode_fixed16(to.gunoffset[2], RR_GUNOFFSET_SCALE),
        ];
        let from_gunoffset = [
            encode_fixed16(from.gunoffset[0], RR_GUNOFFSET_SCALE),
            encode_fixed16(from.gunoffset[1], RR_GUNOFFSET_SCALE),
            encode_fixed16(from.gunoffset[2], RR_GUNOFFSET_SCALE),
        ];
        if to_gunoffset != from_gunoffset {
            extraflags |= protocol::EPS_GUNOFFSET as u8;
        }
        let to_gunangles = [
            encode_fixed16(to.gunangles[0], RR_GUNANGLE_SCALE),
            encode_fixed16(to.gunangles[1], RR_GUNANGLE_SCALE),
            encode_fixed16(to.gunangles[2], RR_GUNANGLE_SCALE),
        ];
        let from_gunangles = [
            encode_fixed16(from.gunangles[0], RR_GUNANGLE_SCALE),
            encode_fixed16(from.gunangles[1], RR_GUNANGLE_SCALE),
            encode_fixed16(from.gunangles[2], RR_GUNANGLE_SCALE),
        ];
        if to_gunangles != from_gunangles {
            extraflags |= protocol::EPS_GUNANGLES as u8;
        }
        let mut statbits = 0u64;
        for i in 0..MAX_STATS_STORAGE {
            if to.stats[i] != from.stats[i] {
                statbits |= 1 << i;
            }
        }
        if statbits != 0 {
            extraflags |= protocol::EPS_STATS as u8;
        }
        if to.gunrate != from.gunrate {
            extraflags |= RR_EPS_GUNRATE;
        }
        RereleasePlayerDelta {
            flags,
            extraflags,
            statbits,
            blend_bits,
            damage_blend_bits,
        }
    }

    /// Write a rerelease player-state delta body after its flags.
    pub fn write_player_state_body(
        writer: &mut MsgWriter,
        to: &PlayerState,
        delta: RereleasePlayerDelta,
    ) -> Result<(), MsgError> {
        let flags = u32::from(delta.flags);
        let extraflags = u32::from(delta.extraflags);
        if (flags & protocol::PS_M_TYPE) != 0 {
            writer.write_byte(to.pmove.pm_type)?;
        }
        if (flags & protocol::PS_M_ORIGIN) != 0 {
            writer.write_float(to.pmove.origin_f[0])?;
            writer.write_float(to.pmove.origin_f[1])?;
        }
        if (extraflags & protocol::EPS_M_ORIGIN2) != 0 {
            writer.write_float(to.pmove.origin_f[2])?;
        }
        if (flags & protocol::PS_M_VELOCITY) != 0 {
            writer.write_float(to.pmove.velocity_f[0])?;
            writer.write_float(to.pmove.velocity_f[1])?;
        }
        if (extraflags & protocol::EPS_M_VELOCITY2) != 0 {
            writer.write_float(to.pmove.velocity_f[2])?;
        }
        if (flags & protocol::PS_M_TIME) != 0 {
            writer.write_short(to.pmove.pm_time as i16)?;
        }
        if (flags & protocol::PS_M_FLAGS) != 0 {
            writer.write_short(to.pmove.pm_flags as i16)?;
        }
        if (flags & protocol::PS_M_GRAVITY) != 0 {
            writer.write_short(to.pmove.gravity)?;
        }
        if (flags & protocol::PS_M_DELTA_ANGLES) != 0 {
            for axis in to.pmove.delta_angles {
                writer.write_short(axis)?;
            }
        }
        if (flags & protocol::PS_VIEWOFFSET) != 0 {
            for axis in to.viewoffset {
                writer.write_short(encode_fixed16(axis, RR_VIEWOFFSET_SCALE))?;
            }
        }
        if (flags & protocol::PS_VIEWANGLES) != 0 {
            writer.write_short(angle_to_short(to.viewangles[0]) as i16)?;
            writer.write_short(angle_to_short(to.viewangles[1]) as i16)?;
        }
        if (extraflags & protocol::EPS_VIEWANGLE2) != 0 {
            writer.write_short(angle_to_short(to.viewangles[2]) as i16)?;
        }
        if (flags & protocol::PS_KICKANGLES) != 0 {
            for axis in to.kick_angles {
                writer.write_short(encode_fixed16(axis, RR_KICK_ANGLE_SCALE))?;
            }
        }
        if (flags & protocol::PS_WEAPONINDEX) != 0 {
            let packed = ((to.gunindex & 0xffff) | ((to.gunskin << RR_GUNINDEX_BITS) & 0xffff)) as u16;
            writer.write_short(packed as i16)?;
        }
        if (flags & protocol::PS_WEAPONFRAME) != 0 {
            writer.write_short(to.gunframe as i16)?;
        }
        if (extraflags & protocol::EPS_GUNOFFSET) != 0 {
            for axis in to.gunoffset {
                writer.write_short(encode_fixed16(axis, RR_GUNOFFSET_SCALE))?;
            }
        }
        if (extraflags & protocol::EPS_GUNANGLES) != 0 {
            for axis in to.gunangles {
                writer.write_short(encode_fixed16(axis, RR_GUNANGLE_SCALE))?;
            }
        }
        if (flags & protocol::PS_BLEND) != 0 {
            writer.write_byte((delta.blend_bits & 0xf) | ((delta.damage_blend_bits & 0xf) << 4))?;
            for i in 0..4 {
                if (delta.blend_bits & (1 << i)) != 0 {
                    writer.write_byte(scaled_trunc(to.blend[i], 255.0) as u8)?;
                }
            }
            for i in 0..4 {
                if (delta.damage_blend_bits & (1 << i)) != 0 {
                    writer.write_byte(scaled_trunc(to.damage_blend[i], 255.0) as u8)?;
                }
            }
        }
        if (flags & protocol::PS_FOV) != 0 {
            writer.write_byte(to.fov)?;
        }
        if (flags & protocol::PS_RDFLAGS) != 0 {
            writer.write_byte(to.rdflags)?;
        }
        if (extraflags & protocol::EPS_STATS) != 0 {
            writer.write_long64(delta.statbits as i64)?;
            for i in 0..MAX_STATS_STORAGE {
                if (delta.statbits & (1 << i)) != 0 {
                    writer.write_short(to.stats[i])?;
                }
            }
        }
        if (extraflags & u32::from(RR_EPS_GUNRATE)) != 0 {
            writer.write_byte(to.gunrate)?;
        }
        if (flags & protocol::PS_RR_VIEWHEIGHT) != 0 {
            writer.write_char(to.pmove.viewheight as i8)?;
        }
        Ok(())
    }

    /// Write a rerelease player-state delta (`writePlayerStateDelta`).
    pub fn write_player_state_delta(
        writer: &mut MsgWriter,
        from: &PlayerState,
        to: &PlayerState,
    ) -> Result<(), MsgError> {
        let delta = Self::encode_player_state(from, to);
        writer.write_byte(protocol::Svc::Playerinfo as u8)?;
        writer.write_short(delta.flags as i16)?;
        writer.write_byte(delta.extraflags)?;
        Self::write_player_state_body(writer, to, delta)
    }

    /// Read a rerelease player-state delta body.
    pub fn read_player_state_body(
        reader: &mut MsgReader<'_>,
        from: &PlayerState,
        flags: u16,
        extraflags: u8,
    ) -> Result<PlayerState, MsgError> {
        let mut to = from.clone();
        let flags = u32::from(flags);
        let extraflags = u32::from(extraflags);
        if (flags & protocol::PS_M_TYPE) != 0 {
            to.pmove.pm_type = reader.byte()?;
        }
        if (flags & protocol::PS_M_ORIGIN) != 0 || (extraflags & protocol::EPS_M_ORIGIN2) != 0 {
            let mut origin = to.pmove.origin_f;
            if (flags & protocol::PS_M_ORIGIN) != 0 {
                origin[0] = reader.float()?;
                origin[1] = reader.float()?;
            }
            if (extraflags & protocol::EPS_M_ORIGIN2) != 0 {
                origin[2] = reader.float()?;
            }
            to.pmove.origin_f = origin;
            for (slot, value) in to.pmove.origin.iter_mut().zip(origin.iter()) {
                *slot = i32::from(pm_float_to_short(*value));
            }
        }
        if (flags & protocol::PS_M_VELOCITY) != 0 || (extraflags & protocol::EPS_M_VELOCITY2) != 0 {
            let mut velocity = to.pmove.velocity_f;
            if (flags & protocol::PS_M_VELOCITY) != 0 {
                velocity[0] = reader.float()?;
                velocity[1] = reader.float()?;
            }
            if (extraflags & protocol::EPS_M_VELOCITY2) != 0 {
                velocity[2] = reader.float()?;
            }
            to.pmove.velocity_f = velocity;
            for (slot, value) in to.pmove.velocity.iter_mut().zip(velocity.iter()) {
                *slot = i32::from(pm_float_to_short(*value));
            }
        }
        if (flags & protocol::PS_M_TIME) != 0 {
            to.pmove.pm_time = i32::from(reader.word()?);
        }
        if (flags & protocol::PS_M_FLAGS) != 0 {
            to.pmove.pm_flags = i32::from(reader.word()?);
        }
        if (flags & protocol::PS_M_GRAVITY) != 0 {
            to.pmove.gravity = reader.short()?;
        }
        if (flags & protocol::PS_M_DELTA_ANGLES) != 0 {
            for axis in &mut to.pmove.delta_angles {
                *axis = reader.short()?;
            }
        }
        if (flags & protocol::PS_VIEWOFFSET) != 0 {
            for axis in &mut to.viewoffset {
                *axis = f64::from(reader.short()?) / RR_VIEWOFFSET_SCALE;
            }
        }
        if (flags & protocol::PS_VIEWANGLES) != 0 {
            to.viewangles[0] = short_to_angle(reader.short()?);
            to.viewangles[1] = short_to_angle(reader.short()?);
        }
        if (extraflags & protocol::EPS_VIEWANGLE2) != 0 {
            to.viewangles[2] = short_to_angle(reader.short()?);
        }
        if (flags & protocol::PS_KICKANGLES) != 0 {
            for axis in &mut to.kick_angles {
                *axis = f64::from(reader.short()?) / RR_KICK_ANGLE_SCALE;
            }
        }
        if (flags & protocol::PS_WEAPONINDEX) != 0 {
            let packed = reader.word()?;
            to.gunindex = i32::from(packed & RR_GUNINDEX_MASK);
            to.gunskin = i32::from(packed >> RR_GUNINDEX_BITS);
        }
        if (flags & protocol::PS_WEAPONFRAME) != 0 {
            to.gunframe = i32::from(reader.word()?);
        }
        if (extraflags & protocol::EPS_GUNOFFSET) != 0 {
            for axis in &mut to.gunoffset {
                *axis = f64::from(reader.short()?) / RR_GUNOFFSET_SCALE;
            }
        }
        if (extraflags & protocol::EPS_GUNANGLES) != 0 {
            for axis in &mut to.gunangles {
                *axis = f64::from(reader.short()?) / RR_GUNANGLE_SCALE;
            }
        }
        if (flags & protocol::PS_BLEND) != 0 {
            let bits = reader.byte()?;
            for i in 0..4 {
                if (bits & (1 << i)) != 0 {
                    to.blend[i] = f64::from(reader.byte()?) / 255.0;
                }
            }
            for i in 0..4 {
                if (bits & (1 << (i + 4))) != 0 {
                    to.damage_blend[i] = f64::from(reader.byte()?) / 255.0;
                }
            }
        }
        if (flags & protocol::PS_FOV) != 0 {
            to.fov = reader.byte()?;
        }
        if (flags & protocol::PS_RDFLAGS) != 0 {
            to.rdflags = reader.byte()?;
        }
        if (extraflags & protocol::EPS_STATS) != 0 {
            let statbits = reader.long64()? as u64;
            for i in 0..MAX_STATS_STORAGE {
                if (statbits & (1 << i)) != 0 {
                    to.stats[i] = reader.short()?;
                }
            }
        }
        if (extraflags & u32::from(RR_EPS_GUNRATE)) != 0 {
            to.gunrate = reader.byte()?;
        }
        if (flags & protocol::PS_RR_VIEWHEIGHT) != 0 {
            to.pmove.viewheight = i32::from(reader.char()?);
        }
        Ok(to)
    }

    /// Read a rerelease player-state delta (`readPlayerStateDelta`).
    pub fn read_player_state_delta(reader: &mut MsgReader<'_>, from: &PlayerState) -> Result<PlayerState, MsgError> {
        let flags = reader.word()?;
        let extraflags = reader.byte()?;
        Self::read_player_state_body(reader, from, flags, extraflags)
    }

    /// Write a rerelease frame (`writeFrame`).
    pub fn write_frame<E>(
        writer: &mut MsgWriter,
        params: &FrameWrite<'_>,
        write_entities: impl FnOnce(&mut MsgWriter) -> Result<(), E>,
    ) -> Result<(), E>
    where
        E: From<MsgError>,
    {
        writer.write_byte(protocol::Svc::Frame as u8)?;
        let offset = if params.lastframe == -1 {
            31
        } else {
            params.framenum - params.lastframe
        };
        writer.write_long((params.framenum & 0x07ff_ffff) | offset.wrapping_shl(27))?;
        writer.write_byte(u8::from(params.surpress_count != 0))?;
        let base = PlayerState::default();
        let from = params.ps_from.unwrap_or(&base);
        let delta = Self::encode_player_state(from, params.ps_to);
        writer.write_byte(delta.extraflags)?;
        writer.write_byte(params.areabits.len() as u8)?;
        writer.write_bytes(params.areabits)?;
        writer.write_short(delta.flags as i16)?;
        Self::write_player_state_body(writer, params.ps_to, delta)?;
        write_entities(writer)
    }

    /// Read a rerelease frame header (`readFrameHeader`).
    pub fn read_frame_header(
        &mut self,
        reader: &mut MsgReader<'_>,
        areabits: &mut Vec<u8>,
    ) -> Result<FrameHeader, MsgError> {
        let encoded = reader.long()? as u32;
        let offset = (encoded >> 27) as i32;
        let serverframe = (encoded & 0x07ff_ffff) as i32;
        let deltaframe = if offset == 31 { -1 } else { serverframe - offset };
        let frame_flags = reader.byte()? & 0x0f;
        self.frame_extraflags = reader.byte()?;
        let len = usize::from(reader.byte()?);
        areabits.clear();
        areabits.extend_from_slice(reader.bytes(len)?);
        Ok(FrameHeader {
            serverframe,
            deltaframe,
            surpress_count: i32::from(frame_flags & 1),
            areabytes: len,
        })
    }

    /// Read the player state of a rerelease frame (`readFramePlayerstate`).
    pub fn read_frame_playerstate(
        &mut self,
        reader: &mut MsgReader<'_>,
        from: &PlayerState,
    ) -> Result<PlayerState, MsgError> {
        let flags = reader.word()?;
        Self::read_player_state_body(reader, from, flags, self.frame_extraflags)
    }

    /// Write a non-batched delta user command.
    pub fn write_delta_usercmd(
        &self,
        writer: &mut MsgWriter,
        from: &Usercmd,
        cmd: &Usercmd,
    ) -> Result<(), VariantError> {
        if !self.classic && cmd.upmove != from.upmove {
            return Err(VariantError::NonBatchableUpMove);
        }
        Ok(crate::q2::write_delta_usercmd(writer, from, cmd)?)
    }

    /// Read a non-batched delta user command.
    pub fn read_delta_usercmd(&self, reader: &mut MsgReader<'_>, from: &Usercmd) -> Result<Usercmd, VariantError> {
        if !self.classic {
            let mut probe = reader.clone();
            if let Ok(bits) = probe.byte() {
                if (bits & (protocol::CM_UP as u8)) != 0 {
                    return Err(VariantError::ReservedUpMove);
                }
            }
        }
        Ok(crate::q2::read_delta_usercmd(reader, from)?)
    }

    /// Write a batched move (`writeBatchMove`).
    pub fn write_batch_move(
        &self,
        writer: &mut MsgWriter,
        lastframe: Option<i32>,
        frames: &[BatchMoveFrame],
    ) -> Result<(), VariantError> {
        if let Some(lastframe) = lastframe {
            writer.write_long(lastframe)?;
        }
        writer.write_byte(frames.len().wrapping_sub(1) as u8)?;
        let mut lightlevel = 0u8;
        if self.classic {
            for frame in frames {
                for cmd in &frame.cmds {
                    lightlevel = cmd.lightlevel;
                }
            }
        }
        writer.write_byte(lightlevel)?;
        let classic = self.classic;
        let mut bits = BatchBitWriter::new(writer);
        write_batch_move_frames(&mut bits, frames, |bw, cmd, prev| {
            encode_rerelease_batch_cmd(bw, cmd, prev, classic)
        })
    }

    /// Read a batched move (`readBatchMove`).
    pub fn read_batch_move(&self, reader: &mut MsgReader<'_>, nodelta: bool) -> Result<BatchMove, VariantError> {
        let lastframe = if nodelta { -1 } else { reader.long()? };
        let num_dups = reader.byte()?;
        if usize::from(num_dups) >= MAX_BATCH_MOVE_FRAMES - 1 {
            return Err(VariantError::BatchNumDups(num_dups));
        }
        let lightlevel = reader.byte()?;
        let mut bits = BatchBitReader::new(reader);
        let classic = self.classic;
        let mut frames = read_batch_move_frames(&mut bits, usize::from(num_dups), |br, prev| {
            decode_rerelease_batch_cmd(br, prev, classic)
        })?;
        if classic {
            for frame in &mut frames {
                for cmd in &mut frame.cmds {
                    cmd.lightlevel = lightlevel;
                }
            }
        }
        Ok(BatchMove {
            lastframe,
            num_dups: usize::from(num_dups),
            frames,
        })
    }
}

/// Decode one rerelease batch command.
fn decode_rerelease_batch_cmd(
    reader: &mut BatchBitReader<'_, '_>,
    prev: Option<&Usercmd>,
    classic_fields: bool,
) -> Result<Usercmd, VariantError> {
    let mut cmd = seed_from_prev(prev);
    if reader.read_unsigned(1)? == 0 {
        return Ok(cmd);
    }
    let bits = reader.read_unsigned(8)?;
    let prev_angle = |axis: usize| prev.map_or(0, |cmd| cmd.angles[axis]);
    if (bits & protocol::CM_ANGLE1) != 0 {
        cmd.angles[0] = read_batch_move_angle(reader, prev_angle(0))?;
    }
    if (bits & protocol::CM_ANGLE2) != 0 {
        cmd.angles[1] = read_batch_move_angle(reader, prev_angle(1))?;
    }
    if (bits & protocol::CM_ANGLE3) != 0 {
        cmd.angles[2] = reader.read_signed(16)? as i16;
    }
    if (bits & protocol::CM_FORWARD) != 0 {
        cmd.forwardmove = reader.read_signed(10)? as i16;
    }
    if (bits & protocol::CM_SIDE) != 0 {
        cmd.sidemove = reader.read_signed(10)? as i16;
    }
    if (bits & protocol::CM_UP) != 0 {
        if !classic_fields {
            return Err(VariantError::BatchCmUp);
        }
        cmd.upmove = reader.read_signed(10)? as i16;
    }
    if (bits & protocol::CM_BUTTONS) != 0 {
        cmd.buttons = reader.read_unsigned(8)? as u8;
    }
    cmd.msec = if (bits & protocol::CM_IMPULSE) != 0 {
        reader.read_unsigned(8)? as u8
    } else {
        prev.map_or(0, |cmd| cmd.msec)
    };
    Ok(cmd)
}

/// Encode one rerelease batch command.
fn encode_rerelease_batch_cmd(
    writer: &mut BatchBitWriter<'_>,
    cmd: &Usercmd,
    prev: Option<&Usercmd>,
    classic_fields: bool,
) -> Result<(), VariantError> {
    writer.write_unsigned(1, 1)?;
    let base = Usercmd::default();
    let from = prev.unwrap_or(&base);
    let mut bits = protocol::CM_IMPULSE;
    if cmd.angles[0] != from.angles[0] {
        bits |= protocol::CM_ANGLE1;
    }
    if cmd.angles[1] != from.angles[1] {
        bits |= protocol::CM_ANGLE2;
    }
    if cmd.angles[2] != from.angles[2] {
        bits |= protocol::CM_ANGLE3;
    }
    if cmd.forwardmove != from.forwardmove {
        bits |= protocol::CM_FORWARD;
    }
    if cmd.sidemove != from.sidemove {
        bits |= protocol::CM_SIDE;
    }
    if classic_fields && cmd.upmove != from.upmove {
        bits |= protocol::CM_UP;
    }
    if cmd.buttons != from.buttons {
        bits |= protocol::CM_BUTTONS;
    }
    writer.write_unsigned(bits, 8)?;
    if (bits & protocol::CM_ANGLE1) != 0 {
        writer.write_unsigned(0, 1)?;
        writer.write_signed(i32::from(cmd.angles[0]), 16)?;
    }
    if (bits & protocol::CM_ANGLE2) != 0 {
        writer.write_unsigned(0, 1)?;
        writer.write_signed(i32::from(cmd.angles[1]), 16)?;
    }
    if (bits & protocol::CM_ANGLE3) != 0 {
        writer.write_signed(i32::from(cmd.angles[2]), 16)?;
    }
    if (bits & protocol::CM_FORWARD) != 0 {
        writer.write_signed(i32::from(cmd.forwardmove).clamp(-512, 511), 10)?;
    }
    if (bits & protocol::CM_SIDE) != 0 {
        writer.write_signed(i32::from(cmd.sidemove).clamp(-512, 511), 10)?;
    }
    if (bits & protocol::CM_UP) != 0 {
        writer.write_signed(i32::from(cmd.upmove).clamp(-512, 511), 10)?;
    }
    if (bits & protocol::CM_BUTTONS) != 0 {
        writer.write_unsigned(u32::from(cmd.buttons), 8)?;
    }
    writer.write_unsigned(u32::from(cmd.msec), 8)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// KEX
// ---------------------------------------------------------------------------

/// KEX demo protocol version.
pub const PROTOCOL_KEX_DEMOS: u32 = 2022;
/// KEX retail protocol version.
pub const PROTOCOL_KEX: u32 = 2023;

const KEX_U_MODEL16: u32 = 1 << 28;
const KEX_U_EFFECTS64: u32 = 1 << 29;
const KEX_U_ALPHA: u32 = 1 << 30;
const KEX_HI_INSTANCE: u8 = RR_HI_MOREFX16;
const KEX_HI_OWNER: u8 = 4;
const KEX_HI_OLDFRAME: u8 = 8;
const KEX_PS_MOREBITS: u32 = 1 << 15;
const KEX_PS_DAMAGE_BLEND: u32 = 1 << 16;
const KEX_PS_TEAM_ID: u32 = 1 << 17;
const KEX_GUNBIT_OFFSET_X: u16 = 1 << 0;
const KEX_GUNBIT_OFFSET_Y: u16 = 1 << 1;
const KEX_GUNBIT_OFFSET_Z: u16 = 1 << 2;
const KEX_GUNBIT_ANGLES_X: u16 = 1 << 3;
const KEX_GUNBIT_ANGLES_Y: u16 = 1 << 4;
const KEX_GUNBIT_ANGLES_Z: u16 = 1 << 5;
const KEX_GUNBIT_GUNRATE: u16 = 1 << 6;
const KEX_SND_VOLUME: u8 = 1 << 0;
const KEX_SND_ATTENUATION: u8 = 1 << 1;
const KEX_SND_POS: u8 = 1 << 2;
const KEX_SND_ENT: u8 = 1 << 3;
const KEX_SND_OFFSET: u8 = 1 << 4;
const KEX_SND_LARGE_ENT: u8 = 1 << 6;
const KEX_MAX_DAMAGE_INDICATORS: usize = 4;
const KEX_MAX_LOCALIZATION_ARGS: u8 = 8;
const KEX_COORD_SHORT_SCALE: f64 = 0.125;

/// Read a KEX user command (`readKexUsercmd`).
///
/// Moves travel as floats; fractional parts truncate toward zero on read.
pub fn read_kex_usercmd(reader: &mut MsgReader<'_>, from: &Usercmd) -> Result<Usercmd, MsgError> {
    let mut to = from.clone();
    let bits = reader.byte()?;
    to.upmove = 0;
    to.impulse = 0;
    for axis in 0..3 {
        if (bits & (1 << axis)) != 0 {
            to.angles[axis] = angle_to_short(f64::from(reader.float()?)) as i16;
        }
    }
    if (bits & 8) != 0 {
        to.forwardmove = reader.float()? as i16;
    }
    if (bits & 16) != 0 {
        to.sidemove = reader.float()? as i16;
    }
    if (bits & 64) != 0 {
        to.buttons = reader.byte()?;
    }
    if (bits & 128) != 0 {
        to.server_frame = reader.long()?;
    }
    to.msec = reader.byte()?;
    Ok(to)
}

/// Write a KEX user command (`writeKexUsercmd`).
pub fn write_kex_usercmd(writer: &mut MsgWriter, from: &Usercmd, to: &Usercmd) -> Result<(), MsgError> {
    let mut bits = 0u8;
    for axis in 0..3 {
        if from.angles[axis] != to.angles[axis] {
            bits |= 1 << axis;
        }
    }
    if from.forwardmove != to.forwardmove {
        bits |= 8;
    }
    if from.sidemove != to.sidemove {
        bits |= 16;
    }
    if from.buttons != to.buttons {
        bits |= 64;
    }
    if from.server_frame != to.server_frame {
        bits |= 128;
    }
    writer.write_byte(bits)?;
    for axis in 0..3 {
        if (bits & (1 << axis)) != 0 {
            writer.write_float(short_to_angle(to.angles[axis]) as f32)?;
        }
    }
    if (bits & 8) != 0 {
        writer.write_float(to.forwardmove as f32)?;
    }
    if (bits & 16) != 0 {
        writer.write_float(to.sidemove as f32)?;
    }
    if (bits & 64) != 0 {
        writer.write_byte(to.buttons)?;
    }
    if (bits & 128) != 0 {
        writer.write_long(to.server_frame)?;
    }
    writer.write_byte(to.msec)
}

/// KEX server data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KexServerData {
    /// Server count.
    pub servercount: i32,
    /// Attract loop.
    pub attractloop: bool,
    /// Server frame rate.
    pub server_fps: u8,
    /// Game directory.
    pub gamedir: String,
    /// Client numbers (one per split-screen player).
    pub clientnums: Vec<i16>,
    /// Level name.
    pub levelname: String,
}

impl KexServerData {
    /// Primary client number.
    #[must_use]
    pub fn clientnum(&self) -> i16 {
        self.clientnums.first().copied().unwrap_or(-1)
    }
}

/// KEX damage indicator (`KexDamageIndicatorT`).
#[derive(Debug, Clone, PartialEq)]
pub struct KexDamageIndicator {
    /// Damage amount.
    pub damage: u8,
    /// Health damage.
    pub health: bool,
    /// Armor damage.
    pub armor: bool,
    /// Shield damage.
    pub shield: bool,
    /// Attack direction.
    pub direction: [f64; 3],
}

/// KEX point of interest (`KexPoiT`).
#[derive(Debug, Clone, PartialEq)]
pub struct KexPoi {
    /// Key.
    pub key: u16,
    /// Time.
    pub time: u16,
    /// Position.
    pub pos: [f32; 3],
    /// Image.
    pub image: u16,
    /// Color.
    pub color: u8,
    /// Flags.
    pub flags: u8,
}

/// KEX help path node (`KexHelpPathT`).
#[derive(Debug, Clone, PartialEq)]
pub struct KexHelpPath {
    /// Path start.
    pub start: bool,
    /// Position.
    pub pos: [f32; 3],
    /// Direction.
    pub dir: [f64; 3],
}

/// KEX muzzle flash (`KexMuzzleflash3T`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KexMuzzleflash3 {
    /// Entity.
    pub entity: i16,
    /// Weapon.
    pub weapon: u16,
}

/// KEX localized print (`KexLocprintT`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KexLocprint {
    /// Flags.
    pub flags: u8,
    /// Base string.
    pub base: String,
    /// Format arguments.
    pub args: Vec<String>,
}

/// KEX sound (`KexSoundT`).
#[derive(Debug, Clone, PartialEq)]
pub struct KexSound {
    /// Field flags.
    pub flags: u8,
    /// Sound index.
    pub index: u16,
    /// Volume.
    pub volume: f64,
    /// Attenuation.
    pub attenuation: f64,
    /// Time offset.
    pub timeofs: f64,
    /// Entity.
    pub entity: u32,
    /// Channel.
    pub channel: u8,
    /// Position.
    pub pos: Option<[f32; 3]>,
}

/// KEX configstring blast record (`KexConfigstringRecordT`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KexConfigstringRecord {
    /// Index.
    pub index: u16,
    /// Value.
    pub value: String,
}

/// KEX spawn baseline blast entry.
#[derive(Debug, Clone, PartialEq)]
pub struct KexBaseline {
    /// Entity number.
    pub entnum: u16,
    /// Baseline state.
    pub state: EntityState,
}

/// KEX protocol codec (`KEX_DEMO_CODEC`).
#[derive(Debug, Clone)]
pub struct KexCodec {
    protocol: u32,
    split_players: usize,
    edict_solid: HashMap<u16, bool>,
}

impl KexCodec {
    /// Build a codec for a KEX protocol version (2022 demos or 2023 retail).
    #[must_use]
    pub fn new(protocol: u32) -> Self {
        Self {
            protocol,
            split_players: 1,
            edict_solid: HashMap::new(),
        }
    }

    /// Active KEX protocol version.
    #[must_use]
    pub fn protocol(&self) -> u32 {
        self.protocol
    }

    /// Switch the active KEX protocol version.
    pub fn set_protocol(&mut self, protocol: u32) {
        self.protocol = protocol;
    }

    /// Whether the demo protocol (low-precision origins) is active.
    #[must_use]
    pub fn is_demo_protocol(&self) -> bool {
        self.protocol == PROTOCOL_KEX_DEMOS
    }

    /// Split-screen player count from the last server data.
    #[must_use]
    pub fn split_player_count(&self) -> usize {
        self.split_players
    }

    /// Write server data (`writeServerData`).
    pub fn write_server_data(&self, writer: &mut MsgWriter, params: &KexServerData) -> Result<(), VariantError> {
        writer.write_byte(protocol::Svc::Serverdata as u8)?;
        writer.write_long(self.protocol as i32)?;
        writer.write_long(params.servercount)?;
        writer.write_byte(u8::from(params.attractloop))?;
        writer.write_byte(params.server_fps)?;
        writer.write_string(&params.gamedir)?;
        if params.clientnums.len() > 1 {
            if params.clientnums.len() > 8 {
                return Err(VariantError::TooManySplitPlayers(params.clientnums.len()));
            }
            writer.write_short(-2)?;
            writer.write_short(params.clientnums.len() as i16)?;
            for number in &params.clientnums {
                writer.write_short(*number)?;
            }
        } else {
            writer.write_short(params.clientnums.first().copied().unwrap_or(0))?;
        }
        writer.write_string(&params.levelname)?;
        Ok(())
    }

    /// Read server data (`readServerData`).
    pub fn read_server_data(&mut self, reader: &mut MsgReader<'_>) -> Result<KexServerData, VariantError> {
        self.edict_solid.clear();
        let servercount = reader.long()?;
        let attractloop = reader.byte()? != 0;
        let server_fps = reader.byte()?;
        let gamedir = reader.string(2047);
        let clientnum = reader.short()?;
        let mut clientnums = Vec::new();
        if clientnum == -2 {
            let count = reader.short()?;
            if !(1..=8).contains(&count) {
                return Err(VariantError::BadSplitCount(count));
            }
            for _ in 0..count {
                clientnums.push(reader.short()?);
            }
        } else {
            clientnums.push(clientnum);
        }
        self.split_players = clientnums.len();
        let levelname = reader.string(2047);
        Ok(KexServerData {
            servercount,
            attractloop,
            server_fps,
            gamedir,
            clientnums,
            levelname,
        })
    }

    /// Write a delta entity; `Ok(false)` writes nothing when unchanged and not forced.
    pub fn write_delta_entity(
        &self,
        writer: &mut MsgWriter,
        from: &EntityState,
        to: &EntityState,
        force: bool,
        new_entity: bool,
    ) -> Result<bool, VariantError> {
        let mut bits = 0u32;
        let mut high = 0u8;
        if to.origin[0] != from.origin[0] {
            bits |= protocol::U_ORIGIN1;
        }
        if to.origin[1] != from.origin[1] {
            bits |= protocol::U_ORIGIN2;
        }
        if to.origin[2] != from.origin[2] {
            bits |= protocol::U_ORIGIN3;
        }
        if to.angles[0] != from.angles[0] {
            bits |= protocol::U_ANGLE1;
        }
        if to.angles[1] != from.angles[1] {
            bits |= protocol::U_ANGLE2;
        }
        if to.angles[2] != from.angles[2] {
            bits |= protocol::U_ANGLE3;
        }
        if to.frame != from.frame {
            bits |= if to.frame >= 256 {
                protocol::U_FRAME16
            } else {
                protocol::U_FRAME8
            };
        }
        if to.skinnum != from.skinnum {
            match width_of(to.skinnum as u32, false) {
                2 => bits |= protocol::U_SKIN8 | protocol::U_SKIN16,
                1 => bits |= protocol::U_SKIN16,
                _ => bits |= protocol::U_SKIN8,
            }
        }
        if to.effects != from.effects || to.morefx != from.morefx {
            if to.morefx != 0 {
                bits |= KEX_U_EFFECTS64;
                match width_of(to.morefx as u32, false) {
                    2 => bits |= protocol::U_EFFECTS8 | protocol::U_EFFECTS16,
                    1 => bits |= protocol::U_EFFECTS16,
                    _ => bits |= protocol::U_EFFECTS8,
                }
            } else {
                match width_of(to.effects as u32, false) {
                    2 => bits |= protocol::U_EFFECTS8 | protocol::U_EFFECTS16,
                    1 => bits |= protocol::U_EFFECTS16,
                    _ => bits |= protocol::U_EFFECTS8,
                }
            }
        }
        if to.renderfx != from.renderfx {
            match width_of(to.renderfx as u32, false) {
                2 => bits |= protocol::U_RENDERFX8 | protocol::U_RENDERFX16,
                1 => bits |= protocol::U_RENDERFX16,
                _ => bits |= protocol::U_RENDERFX8,
            }
        }
        if to.solid != from.solid {
            bits |= protocol::U_SOLID;
        }
        if to.event != 0 {
            bits |= protocol::U_EVENT;
        }
        let models = [
            (protocol::U_MODEL, to.modelindex, from.modelindex),
            (protocol::U_MODEL2, to.modelindex2, from.modelindex2),
            (protocol::U_MODEL3, to.modelindex3, from.modelindex3),
            (protocol::U_MODEL4, to.modelindex4, from.modelindex4),
        ];
        for (flag, value, old) in models {
            if value != old {
                bits |= flag;
                if value > 255 {
                    bits |= KEX_U_MODEL16;
                }
            }
        }
        let volume = encode_loop_volume(to.loop_volume);
        let attenuation = encode_loop_attenuation(to.loop_attenuation);
        let volume_changed = volume != encode_loop_volume(from.loop_volume);
        let attenuation_changed = attenuation != encode_loop_attenuation(from.loop_attenuation);
        if to.sound != from.sound || volume_changed || attenuation_changed {
            bits |= protocol::U_SOUND;
        }
        if new_entity || (to.renderfx & RF_BEAM) != 0 {
            bits |= protocol::U_OLDORIGIN;
        }
        if encode_alpha(to.alpha) != encode_alpha(from.alpha) {
            bits |= KEX_U_ALPHA;
        }
        if encode_scale(to.scale) != encode_scale(from.scale) {
            high |= RR_HI_SCALE;
        }
        if to.instance_bits != from.instance_bits {
            high |= KEX_HI_INSTANCE;
        }
        if to.owner != from.owner {
            high |= KEX_HI_OWNER;
        }
        if to.old_frame != from.old_frame {
            high |= KEX_HI_OLDFRAME;
        }
        if bits == 0 && high == 0 && !force {
            return Ok(false);
        }
        // The native KEX reader sign-extends bit 31 into the high word, so the
        // writer emits every high flag once any of them is set.
        if high != 0 {
            high = 255;
        }
        write_entity_bits_wide(writer, bits, high, to.number)?;
        for (flag, value, _) in models {
            if (bits & flag) != 0 {
                if (bits & KEX_U_MODEL16) != 0 {
                    writer.write_short(value as i16)?;
                } else {
                    writer.write_byte(value as u8)?;
                }
            }
        }
        if (bits & protocol::U_FRAME16) != 0 {
            writer.write_short(to.frame as i16)?;
        } else if (bits & protocol::U_FRAME8) != 0 {
            writer.write_byte(to.frame as u8)?;
        }
        write_kex_width(writer, to.skinnum, bits, protocol::U_SKIN8, protocol::U_SKIN16)?;
        if (bits & KEX_U_EFFECTS64) != 0 {
            writer.write_long(to.effects)?;
        }
        let effects_value = if (bits & KEX_U_EFFECTS64) != 0 {
            to.morefx
        } else {
            to.effects
        };
        write_kex_width(writer, effects_value, bits, protocol::U_EFFECTS8, protocol::U_EFFECTS16)?;
        write_kex_width(writer, to.renderfx, bits, protocol::U_RENDERFX8, protocol::U_RENDERFX16)?;
        if (bits & protocol::U_SOLID) != 0 {
            writer.write_long(to.solid as i32)?;
        }
        let float_coords = self.protocol != PROTOCOL_KEX_DEMOS || to.solid != 0;
        for (axis, flag) in [
            (to.origin[0], protocol::U_ORIGIN1),
            (to.origin[1], protocol::U_ORIGIN2),
            (to.origin[2], protocol::U_ORIGIN3),
        ] {
            if (bits & flag) != 0 {
                if float_coords {
                    writer.write_float(axis as f32)?;
                } else {
                    writer.write_q2_coord(axis)?;
                }
            }
        }
        if (bits & protocol::U_OLDORIGIN) != 0 {
            for axis in to.old_origin {
                if float_coords {
                    writer.write_float(axis as f32)?;
                } else {
                    writer.write_q2_coord(axis)?;
                }
            }
        }
        if (bits & protocol::U_ANGLE1) != 0 {
            writer.write_float(to.angles[0] as f32)?;
        }
        if (bits & protocol::U_ANGLE2) != 0 {
            writer.write_float(to.angles[1] as f32)?;
        }
        if (bits & protocol::U_ANGLE3) != 0 {
            writer.write_float(to.angles[2] as f32)?;
        }
        if (bits & protocol::U_SOUND) != 0 {
            let mut sound = to.sound;
            if volume_changed {
                sound |= 1 << 14;
            }
            if attenuation_changed {
                sound |= 1 << 15;
            }
            writer.write_short(sound as i16)?;
            if volume_changed {
                writer.write_byte(volume)?;
            }
            if attenuation_changed {
                writer.write_byte(attenuation)?;
            }
        }
        if (bits & protocol::U_EVENT) != 0 {
            writer.write_byte(to.event)?;
        }
        if (bits & KEX_U_ALPHA) != 0 {
            writer.write_byte(encode_alpha(to.alpha))?;
        }
        if (high & RR_HI_SCALE) != 0 {
            writer.write_byte(encode_scale(to.scale))?;
        }
        if (high & KEX_HI_INSTANCE) != 0 {
            writer.write_byte(to.instance_bits)?;
        }
        if (high & KEX_HI_OWNER) != 0 {
            writer.write_short(to.owner as i16)?;
        }
        if (high & KEX_HI_OLDFRAME) != 0 {
            writer.write_short(to.old_frame as i16)?;
        }
        Ok(true)
    }

    /// Read a delta entity body (`readDeltaEntity`).
    pub fn read_delta_entity(
        &mut self,
        reader: &mut MsgReader<'_>,
        from: &EntityState,
        number: u16,
        bits: WideEntityBits,
    ) -> Result<EntityState, VariantError> {
        let mut to = from.clone();
        to.number = number;
        let lo = bits.lo;
        let hi = bits.hi;
        let model16 = (lo & KEX_U_MODEL16) != 0;
        if (lo & protocol::U_MODEL) != 0 {
            to.modelindex = if model16 {
                reader.word()?
            } else {
                u16::from(reader.byte()?)
            };
        }
        if (lo & protocol::U_MODEL2) != 0 {
            to.modelindex2 = if model16 {
                reader.word()?
            } else {
                u16::from(reader.byte()?)
            };
        }
        if (lo & protocol::U_MODEL3) != 0 {
            to.modelindex3 = if model16 {
                reader.word()?
            } else {
                u16::from(reader.byte()?)
            };
        }
        if (lo & protocol::U_MODEL4) != 0 {
            to.modelindex4 = if model16 {
                reader.word()?
            } else {
                u16::from(reader.byte()?)
            };
        }
        if (lo & protocol::U_FRAME8) != 0 {
            to.frame = i32::from(reader.byte()?);
        } else if (lo & protocol::U_FRAME16) != 0 {
            to.frame = i32::from(reader.word()?);
        }
        if (lo & (protocol::U_SKIN8 | protocol::U_SKIN16)) == (protocol::U_SKIN8 | protocol::U_SKIN16) {
            to.skinnum = reader.long()?;
        } else if (lo & protocol::U_SKIN16) != 0 {
            to.skinnum = i32::from(reader.word()?);
        } else if (lo & protocol::U_SKIN8) != 0 {
            to.skinnum = i32::from(reader.byte()?);
        }
        let mut low_effects = 0u32;
        if (lo & KEX_U_EFFECTS64) != 0 {
            low_effects = reader.long()? as u32;
        }
        let mut effects32 = 0u32;
        match lo & (protocol::U_EFFECTS8 | protocol::U_EFFECTS16) {
            combo if combo == (protocol::U_EFFECTS8 | protocol::U_EFFECTS16) => {
                effects32 = reader.long()? as u32;
            }
            combo if (combo & protocol::U_EFFECTS16) != 0 => {
                effects32 = u32::from(reader.word()?);
            }
            combo if (combo & protocol::U_EFFECTS8) != 0 => {
                effects32 = u32::from(reader.byte()?);
            }
            _ => {}
        }
        if (lo & (KEX_U_EFFECTS64 | protocol::U_EFFECTS8 | protocol::U_EFFECTS16)) != 0 {
            if (lo & KEX_U_EFFECTS64) != 0 {
                to.effects = low_effects as i32;
                to.morefx = effects32 as i32;
            } else {
                to.effects = effects32 as i32;
                to.morefx = 0;
            }
        }
        if (lo & (protocol::U_RENDERFX8 | protocol::U_RENDERFX16)) == (protocol::U_RENDERFX8 | protocol::U_RENDERFX16) {
            to.renderfx = reader.long()?;
        } else if (lo & protocol::U_RENDERFX16) != 0 {
            to.renderfx = i32::from(reader.word()?);
        } else if (lo & protocol::U_RENDERFX8) != 0 {
            to.renderfx = i32::from(reader.byte()?);
        }
        let nonzero_solid = if (lo & protocol::U_SOLID) != 0 {
            to.solid = reader.long()? as u32;
            let nonzero = to.solid != 0;
            self.edict_solid.insert(number, nonzero);
            nonzero
        } else {
            self.edict_solid.get(&number).copied().unwrap_or(false)
        };
        // Note the origin read order: OLDORIGIN rides with the origins here, ahead of
        // the angles, unlike the vanilla layout.
        if self.protocol != PROTOCOL_KEX_DEMOS || nonzero_solid {
            if (lo & protocol::U_ORIGIN1) != 0 {
                to.origin[0] = f64::from(reader.float()?);
            }
            if (lo & protocol::U_ORIGIN2) != 0 {
                to.origin[1] = f64::from(reader.float()?);
            }
            if (lo & protocol::U_ORIGIN3) != 0 {
                to.origin[2] = f64::from(reader.float()?);
            }
            if (lo & protocol::U_OLDORIGIN) != 0 {
                to.old_origin[0] = f64::from(reader.float()?);
                to.old_origin[1] = f64::from(reader.float()?);
                to.old_origin[2] = f64::from(reader.float()?);
            }
        } else {
            if (lo & protocol::U_ORIGIN1) != 0 {
                to.origin[0] = f64::from(reader.short()?) * KEX_COORD_SHORT_SCALE;
            }
            if (lo & protocol::U_ORIGIN2) != 0 {
                to.origin[1] = f64::from(reader.short()?) * KEX_COORD_SHORT_SCALE;
            }
            if (lo & protocol::U_ORIGIN3) != 0 {
                to.origin[2] = f64::from(reader.short()?) * KEX_COORD_SHORT_SCALE;
            }
            if (lo & protocol::U_OLDORIGIN) != 0 {
                to.old_origin[0] = f64::from(reader.short()?) * KEX_COORD_SHORT_SCALE;
                to.old_origin[1] = f64::from(reader.short()?) * KEX_COORD_SHORT_SCALE;
                to.old_origin[2] = f64::from(reader.short()?) * KEX_COORD_SHORT_SCALE;
            }
        }
        if (lo & protocol::U_ANGLE1) != 0 {
            to.angles[0] = f64::from(reader.float()?);
        }
        if (lo & protocol::U_ANGLE2) != 0 {
            to.angles[1] = f64::from(reader.float()?);
        }
        if (lo & protocol::U_ANGLE3) != 0 {
            to.angles[2] = f64::from(reader.float()?);
        }
        if (lo & protocol::U_SOUND) != 0 {
            let sound_word = reader.word()?;
            to.sound = sound_word & 0x3fff;
            if (sound_word & RR_SOUND_FLAG_VOLUME) != 0 {
                to.loop_volume = decode_loop_volume(reader.byte()?);
            }
            if (sound_word & RR_SOUND_FLAG_ATTENUATION) != 0 {
                to.loop_attenuation = decode_loop_attenuation(reader.byte()?);
            }
        }
        if (lo & protocol::U_EVENT) != 0 {
            to.event = reader.byte()?;
        } else {
            to.event = 0;
        }
        if (lo & KEX_U_ALPHA) != 0 {
            to.alpha = decode_alpha(reader.byte()?);
        }
        if (hi & RR_HI_SCALE) != 0 {
            to.scale = decode_scale(reader.byte()?);
        }
        if (hi & KEX_HI_INSTANCE) != 0 {
            to.instance_bits = reader.byte()?;
        }
        if (hi & KEX_HI_OWNER) != 0 {
            to.owner = reader.word()?;
        }
        if (hi & KEX_HI_OLDFRAME) != 0 {
            to.old_frame = reader.word()?;
        }
        Ok(to)
    }

    /// Write an entity removal (`writeEntityRemove`).
    pub fn write_entity_remove(writer: &mut MsgWriter, number: u16) -> Result<(), MsgError> {
        write_entity_bits_wide(writer, 1 << 6, 0, number)
    }

    /// Write a spawn baseline (`writeSpawnBaseline`).
    pub fn write_spawn_baseline(&self, writer: &mut MsgWriter, entity: &EntityState) -> Result<bool, VariantError> {
        writer.write_byte(protocol::Svc::Spawnbaseline as u8)?;
        self.write_delta_entity(writer, &EntityState::default(), entity, true, true)
    }

    /// Read the delta-angle value the KEX writer compares.
    fn delta_angle(state: &PlayerState, index: usize) -> f64 {
        if state.pmove.delta_angle_float {
            f64::from(state.pmove.delta_angles_f[index])
        } else {
            short_to_angle(state.pmove.delta_angles[index])
        }
    }

    /// Encode a KEX player-state delta, returning flags and gun bits.
    fn encode_player_state(from: &PlayerState, to: &PlayerState) -> (u32, u16) {
        let mut flags = 0u32;
        let mut gunbits = 0u16;
        if to.pmove.pm_type != from.pmove.pm_type {
            flags |= protocol::PS_M_TYPE;
        }
        if to.pmove.origin_f != from.pmove.origin_f {
            flags |= protocol::PS_M_ORIGIN;
        }
        if to.pmove.velocity_f != from.pmove.velocity_f {
            flags |= protocol::PS_M_VELOCITY;
        }
        if to.pmove.pm_time != from.pmove.pm_time {
            flags |= protocol::PS_M_TIME;
        }
        if to.pmove.pm_flags != from.pmove.pm_flags {
            flags |= protocol::PS_M_FLAGS;
        }
        if to.pmove.gravity != from.pmove.gravity {
            flags |= protocol::PS_M_GRAVITY;
        }
        for i in 0..3 {
            if Self::delta_angle(to, i) != Self::delta_angle(from, i) {
                flags |= protocol::PS_M_DELTA_ANGLES;
            }
        }
        let viewoffset_changed = [0, 1, 2].iter().any(|&i| {
            encode_fixed16(to.viewoffset[i], RR_VIEWOFFSET_SCALE)
                != encode_fixed16(from.viewoffset[i], RR_VIEWOFFSET_SCALE)
        });
        if viewoffset_changed || to.pmove.viewheight != from.pmove.viewheight {
            flags |= protocol::PS_VIEWOFFSET;
        }
        if to.viewangles != from.viewangles {
            flags |= protocol::PS_VIEWANGLES;
        }
        let kick_changed = [0, 1, 2].iter().any(|&i| {
            encode_fixed16(to.kick_angles[i], RR_KICK_ANGLE_SCALE)
                != encode_fixed16(from.kick_angles[i], RR_KICK_ANGLE_SCALE)
        });
        if kick_changed {
            flags |= protocol::PS_KICKANGLES;
        }
        if [0, 1, 2, 3]
            .iter()
            .any(|&i| kex_byte_color(to.blend[i]) != kex_byte_color(from.blend[i]))
        {
            flags |= protocol::PS_BLEND;
        }
        if [0, 1, 2, 3]
            .iter()
            .any(|&i| kex_byte_color(to.damage_blend[i]) != kex_byte_color(from.damage_blend[i]))
        {
            flags |= KEX_PS_DAMAGE_BLEND;
        }
        if to.team_id != from.team_id {
            flags |= KEX_PS_TEAM_ID;
        }
        if to.fov != from.fov {
            flags |= protocol::PS_FOV;
        }
        if to.rdflags != from.rdflags {
            flags |= protocol::PS_RDFLAGS;
        }
        if to.gunindex != from.gunindex || to.gunskin != from.gunskin {
            flags |= protocol::PS_WEAPONINDEX;
        }
        for i in 0..3 {
            if to.gunoffset[i] != from.gunoffset[i] {
                gunbits |= 1 << i;
            }
            if to.gunangles[i] != from.gunangles[i] {
                gunbits |= 1 << (i + 3);
            }
        }
        if to.gunrate != from.gunrate {
            gunbits |= KEX_GUNBIT_GUNRATE;
        }
        if to.gunframe != from.gunframe || gunbits != 0 {
            flags |= protocol::PS_WEAPONFRAME;
        }
        (flags, gunbits)
    }

    /// Write a KEX player-state delta (flags only, no opcode).
    pub fn write_player_state_delta(
        writer: &mut MsgWriter,
        from: &PlayerState,
        to: &PlayerState,
    ) -> Result<(), VariantError> {
        let (mut flags, gunbits) = Self::encode_player_state(from, to);
        if flags > 65535 {
            flags |= KEX_PS_MOREBITS;
        }
        writer.write_short(flags as i16)?;
        if (flags & KEX_PS_MOREBITS) != 0 {
            writer.write_short((flags >> 16) as i16)?;
        }
        if (flags & protocol::PS_M_TYPE) != 0 {
            writer.write_byte(to.pmove.pm_type)?;
        }
        if (flags & protocol::PS_M_ORIGIN) != 0 {
            for axis in to.pmove.origin_f {
                writer.write_float(axis)?;
            }
        }
        if (flags & protocol::PS_M_VELOCITY) != 0 {
            for axis in to.pmove.velocity_f {
                writer.write_float(axis)?;
            }
        }
        if (flags & protocol::PS_M_TIME) != 0 {
            writer.write_short(to.pmove.pm_time as i16)?;
        }
        if (flags & protocol::PS_M_FLAGS) != 0 {
            writer.write_short(to.pmove.pm_flags as i16)?;
        }
        if (flags & protocol::PS_M_GRAVITY) != 0 {
            writer.write_short(to.pmove.gravity)?;
        }
        if (flags & protocol::PS_M_DELTA_ANGLES) != 0 {
            for i in 0..3 {
                writer.write_float(Self::delta_angle(to, i) as f32)?;
            }
        }
        if (flags & protocol::PS_VIEWOFFSET) != 0 {
            for axis in to.viewoffset {
                writer.write_short(encode_fixed16(axis, RR_VIEWOFFSET_SCALE))?;
            }
            writer.write_char(to.pmove.viewheight as i8)?;
        }
        if (flags & protocol::PS_VIEWANGLES) != 0 {
            for axis in to.viewangles {
                writer.write_float(axis as f32)?;
            }
        }
        if (flags & protocol::PS_KICKANGLES) != 0 {
            for axis in to.kick_angles {
                writer.write_short(encode_fixed16(axis, RR_KICK_ANGLE_SCALE))?;
            }
        }
        if (flags & protocol::PS_WEAPONINDEX) != 0 {
            writer.write_short((to.gunindex | (to.gunskin << 13)) as i16)?;
        }
        if (flags & protocol::PS_WEAPONFRAME) != 0 {
            if !(0..512).contains(&to.gunframe) {
                return Err(VariantError::GunRange(to.gunframe));
            }
            writer.write_short((to.gunframe | (i32::from(gunbits) << 9)) as i16)?;
            for i in 0..3 {
                if (gunbits & (1 << i)) != 0 {
                    writer.write_float(to.gunoffset[i] as f32)?;
                }
            }
            for i in 0..3 {
                if (gunbits & (1 << (i + 3))) != 0 {
                    writer.write_float(to.gunangles[i] as f32)?;
                }
            }
            if (gunbits & KEX_GUNBIT_GUNRATE) != 0 {
                writer.write_byte(to.gunrate)?;
            }
        }
        if (flags & protocol::PS_BLEND) != 0 {
            for axis in to.blend {
                writer.write_byte(kex_byte_color(axis))?;
            }
        }
        if (flags & protocol::PS_FOV) != 0 {
            writer.write_byte(to.fov)?;
        }
        if (flags & protocol::PS_RDFLAGS) != 0 {
            writer.write_byte(to.rdflags)?;
        }
        for half in 0..2 {
            let mut mask = 0u32;
            for i in 0..32 {
                if to.stats[i + half * 32] != from.stats[i + half * 32] {
                    mask |= 1 << i;
                }
            }
            writer.write_long(mask as i32)?;
            for i in 0..32 {
                if (mask & (1 << i)) != 0 {
                    writer.write_short(to.stats[i + half * 32])?;
                }
            }
        }
        if (flags & KEX_PS_DAMAGE_BLEND) != 0 {
            for axis in to.damage_blend {
                writer.write_byte(kex_byte_color(axis))?;
            }
        }
        if (flags & KEX_PS_TEAM_ID) != 0 {
            writer.write_byte(to.team_id)?;
        }
        Ok(())
    }

    /// Read KEX player-state flags (`readKexFlags`).
    fn read_player_state_flags(reader: &mut MsgReader<'_>) -> Result<u32, MsgError> {
        let mut flags = u32::from(reader.word()?);
        if (flags & KEX_PS_MOREBITS) != 0 {
            flags |= u32::from(reader.word()?) << 16;
        }
        Ok(flags)
    }

    /// Read a KEX player-state delta (`readPlayerStateDelta`).
    pub fn read_player_state_delta(
        reader: &mut MsgReader<'_>,
        from: &PlayerState,
    ) -> Result<PlayerState, VariantError> {
        let flags = Self::read_player_state_flags(reader)?;
        Self::read_player_state_body(reader, from, flags)
    }

    /// Read a KEX player-state body.
    fn read_player_state_body(
        reader: &mut MsgReader<'_>,
        from: &PlayerState,
        flags: u32,
    ) -> Result<PlayerState, VariantError> {
        let mut to = from.clone();
        if (flags & protocol::PS_M_TYPE) != 0 {
            to.pmove.pm_type = reader.byte()?;
        }
        if (flags & protocol::PS_M_ORIGIN) != 0 {
            for i in 0..3 {
                let value = reader.float()?;
                to.pmove.origin_f[i] = value;
                to.pmove.origin[i] = i32::from(pm_float_to_short(value));
            }
        }
        if (flags & protocol::PS_M_VELOCITY) != 0 {
            for i in 0..3 {
                let value = reader.float()?;
                to.pmove.velocity_f[i] = value;
                to.pmove.velocity[i] = i32::from(pm_float_to_short(value));
            }
        }
        if (flags & protocol::PS_M_TIME) != 0 {
            to.pmove.pm_time = i32::from(reader.word()?);
        }
        if (flags & protocol::PS_M_FLAGS) != 0 {
            to.pmove.pm_flags = i32::from(reader.word()?);
        }
        if (flags & protocol::PS_M_GRAVITY) != 0 {
            to.pmove.gravity = reader.short()?;
        }
        if (flags & protocol::PS_M_DELTA_ANGLES) != 0 {
            to.pmove.delta_angle_float = true;
            for i in 0..3 {
                let value = reader.float()?;
                to.pmove.delta_angles_f[i] = value;
                to.pmove.delta_angles[i] = angle_to_short(f64::from(value)) as i16;
            }
        }
        if (flags & protocol::PS_VIEWOFFSET) != 0 {
            for axis in &mut to.viewoffset {
                *axis = f64::from(reader.short()?) / RR_VIEWOFFSET_SCALE;
            }
            to.pmove.viewheight = i32::from(reader.char()?);
        }
        if (flags & protocol::PS_VIEWANGLES) != 0 {
            for axis in &mut to.viewangles {
                *axis = f64::from(reader.float()?);
            }
        }
        if (flags & protocol::PS_KICKANGLES) != 0 {
            for axis in &mut to.kick_angles {
                *axis = f64::from(reader.short()?) / RR_KICK_ANGLE_SCALE;
            }
        }
        if (flags & protocol::PS_WEAPONINDEX) != 0 {
            let packed = reader.word()?;
            to.gunindex = i32::from(packed & RR_GUNINDEX_MASK);
            to.gunskin = i32::from(packed >> RR_GUNINDEX_BITS);
        }
        if (flags & protocol::PS_WEAPONFRAME) != 0 {
            let mut gunbits = reader.word()?;
            to.gunframe = i32::from(gunbits & 0x1ff);
            gunbits >>= 9;
            if (gunbits & KEX_GUNBIT_OFFSET_X) != 0 {
                to.gunoffset[0] = f64::from(reader.float()?);
            }
            if (gunbits & KEX_GUNBIT_OFFSET_Y) != 0 {
                to.gunoffset[1] = f64::from(reader.float()?);
            }
            if (gunbits & KEX_GUNBIT_OFFSET_Z) != 0 {
                to.gunoffset[2] = f64::from(reader.float()?);
            }
            if (gunbits & KEX_GUNBIT_ANGLES_X) != 0 {
                to.gunangles[0] = f64::from(reader.float()?);
            }
            if (gunbits & KEX_GUNBIT_ANGLES_Y) != 0 {
                to.gunangles[1] = f64::from(reader.float()?);
            }
            if (gunbits & KEX_GUNBIT_ANGLES_Z) != 0 {
                to.gunangles[2] = f64::from(reader.float()?);
            }
            if (gunbits & KEX_GUNBIT_GUNRATE) != 0 {
                to.gunrate = reader.byte()?;
            }
        }
        if (flags & protocol::PS_BLEND) != 0 {
            for axis in &mut to.blend {
                *axis = f64::from(reader.byte()?) / 255.0;
            }
        }
        if (flags & protocol::PS_FOV) != 0 {
            to.fov = reader.byte()?;
        }
        if (flags & protocol::PS_RDFLAGS) != 0 {
            to.rdflags = reader.byte()?;
        }
        let statbits1 = reader.long()? as u32;
        for i in 0..32 {
            if (statbits1 & (1 << i)) != 0 && i < MAX_STATS_STORAGE {
                to.stats[i] = reader.short()?;
            }
        }
        let statbits2 = reader.long()? as u32;
        for i in 0..32 {
            if (statbits2 & (1 << i)) != 0 && 32 + i < MAX_STATS_STORAGE {
                to.stats[32 + i] = reader.short()?;
            }
        }
        if (flags & KEX_PS_DAMAGE_BLEND) != 0 {
            for axis in &mut to.damage_blend {
                *axis = f64::from(reader.byte()?) / 255.0;
            }
        }
        if (flags & KEX_PS_TEAM_ID) != 0 {
            to.team_id = reader.byte()?;
        }
        Ok(to)
    }

    /// Write a KEX frame (`writeFrame`).
    pub fn write_frame<E>(
        writer: &mut MsgWriter,
        params: &FrameWrite<'_>,
        write_entities: impl FnOnce(&mut MsgWriter) -> Result<(), E>,
    ) -> Result<(), E>
    where
        E: From<MsgError> + From<VariantError>,
    {
        writer.write_byte(protocol::Svc::Frame as u8)?;
        writer.write_long(params.framenum)?;
        writer.write_long(params.lastframe)?;
        writer.write_byte(params.surpress_count as u8)?;
        writer.write_byte(params.areabits.len() as u8)?;
        writer.write_bytes(params.areabits)?;
        writer.write_byte(protocol::Svc::Playerinfo as u8)?;
        let base = PlayerState::default();
        let from = params.ps_from.unwrap_or(&base);
        Self::write_player_state_delta(writer, from, params.ps_to)?;
        write_entities(writer)
    }

    /// Read a KEX frame header (`readFrameHeader`).
    pub fn read_frame_header(reader: &mut MsgReader<'_>, areabits: &mut Vec<u8>) -> Result<FrameHeader, MsgError> {
        let serverframe = reader.long()?;
        let deltaframe = reader.long()?;
        let surpress_count = i32::from(reader.byte()?);
        let len = usize::from(reader.byte()?);
        areabits.clear();
        areabits.extend_from_slice(reader.bytes(len)?);
        Ok(FrameHeader {
            serverframe,
            deltaframe,
            surpress_count,
            areabytes: len,
        })
    }

    /// Read the player state of a KEX frame (`readFramePlayerstate`).
    pub fn read_frame_playerstate(reader: &mut MsgReader<'_>, from: &PlayerState) -> Result<PlayerState, VariantError> {
        let opcode = reader.byte()?;
        if opcode != protocol::Svc::Playerinfo as u8 {
            return Err(VariantError::Codec(Q2CodecError::UnexpectedOpcode {
                expected: protocol::Svc::Playerinfo as u8,
                found: opcode,
            }));
        }
        let flags = Self::read_player_state_flags(reader)?;
        Self::read_player_state_body(reader, from, flags)
    }

    /// Consume the packet-entities opcode of a KEX frame (`readPacketEntitiesBegin`).
    pub fn read_packet_entities_begin(reader: &mut MsgReader<'_>) -> Result<(), VariantError> {
        let opcode = reader.byte()?;
        if opcode != protocol::Svc::Packetentities as u8 {
            return Err(VariantError::Codec(Q2CodecError::UnexpectedOpcode {
                expected: protocol::Svc::Packetentities as u8,
                found: opcode,
            }));
        }
        Ok(())
    }

    /// Read KEX damage indicators (`readDamageKex`).
    pub fn read_damage(reader: &mut MsgReader<'_>) -> Result<Vec<KexDamageIndicator>, VariantError> {
        let count = reader.byte()?;
        let mut out = Vec::new();
        for i in 0..count {
            let encoded = reader.byte()?;
            let direction = read_dir(reader)?;
            if usize::from(i) >= KEX_MAX_DAMAGE_INDICATORS {
                continue;
            }
            out.push(KexDamageIndicator {
                damage: encoded & 0x1f,
                health: (encoded & 0x20) != 0,
                armor: (encoded & 0x40) != 0,
                shield: (encoded & 0x80) != 0,
                direction,
            });
        }
        Ok(out)
    }

    /// Read a KEX point of interest (`readPoiKex`).
    pub fn read_poi(reader: &mut MsgReader<'_>) -> Result<KexPoi, MsgError> {
        let key = reader.word()?;
        let time = reader.word()?;
        let pos = [reader.float()?, reader.float()?, reader.float()?];
        let image = reader.word()?;
        let color = reader.byte()?;
        let flags = reader.byte()?;
        Ok(KexPoi {
            key,
            time,
            pos,
            image,
            color,
            flags,
        })
    }

    /// Read a KEX help path node (`readHelpPathKex`).
    pub fn read_help_path(reader: &mut MsgReader<'_>) -> Result<KexHelpPath, VariantError> {
        let start = reader.byte()? != 0;
        let pos = [reader.float()?, reader.float()?, reader.float()?];
        let dir = read_dir(reader)?;
        Ok(KexHelpPath { start, pos, dir })
    }

    /// Read a KEX muzzle flash (`readMuzzleflash3Kex`).
    pub fn read_muzzleflash3(reader: &mut MsgReader<'_>) -> Result<KexMuzzleflash3, MsgError> {
        let entity = reader.short()?;
        let weapon = reader.word()?;
        Ok(KexMuzzleflash3 { entity, weapon })
    }

    /// Read a KEX achievement string (`readAchievementKex`).
    pub fn read_achievement(reader: &mut MsgReader<'_>) -> String {
        reader.string(2047)
    }

    /// Read a KEX localized print (`readLocprintKex`).
    pub fn read_locprint(reader: &mut MsgReader<'_>) -> Result<KexLocprint, VariantError> {
        let flags = reader.byte()?;
        let base = reader.string(2047);
        let num_args = reader.byte()?;
        if num_args > KEX_MAX_LOCALIZATION_ARGS {
            return Err(VariantError::TooManyLocArgs(num_args));
        }
        let mut args = Vec::with_capacity(usize::from(num_args));
        for _ in 0..num_args {
            args.push(reader.string(2047));
        }
        Ok(KexLocprint { flags, base, args })
    }

    /// Read a KEX split-client index (`readSplitclientKex`).
    pub fn read_splitclient(reader: &mut MsgReader<'_>) -> Result<u8, MsgError> {
        reader.byte()
    }

    /// Read a KEX sound (`readSoundKex`).
    pub fn read_sound(&self, reader: &mut MsgReader<'_>) -> Result<KexSound, MsgError> {
        let flags = reader.byte()?;
        let index = reader.word()?;
        let volume = if (flags & KEX_SND_VOLUME) != 0 {
            f64::from(reader.byte()?) / 255.0
        } else {
            1.0
        };
        let attenuation = if (flags & KEX_SND_ATTENUATION) != 0 {
            f64::from(reader.byte()?) / 64.0
        } else {
            1.0
        };
        let timeofs = if (flags & KEX_SND_OFFSET) != 0 {
            f64::from(reader.byte()?) / 1000.0
        } else {
            0.0
        };
        let mut entity = 0u32;
        let mut channel = 0u8;
        if (flags & KEX_SND_ENT) != 0 {
            let entchan = if (flags & KEX_SND_LARGE_ENT) != 0 {
                reader.long()? as u32
            } else {
                u32::from(reader.word()?)
            };
            entity = entchan >> 3;
            channel = (entchan & 7) as u8;
        }
        let mut pos = None;
        if (flags & KEX_SND_POS) != 0 {
            if self.is_demo_protocol() {
                pos = Some(
                    [
                        f64::from(reader.short()?) * KEX_COORD_SHORT_SCALE,
                        f64::from(reader.short()?) * KEX_COORD_SHORT_SCALE,
                        f64::from(reader.short()?) * KEX_COORD_SHORT_SCALE,
                    ]
                    .map(|axis| axis as f32),
                );
            } else {
                pos = Some([reader.float()?, reader.float()?, reader.float()?]);
            }
        }
        Ok(KexSound {
            flags,
            index,
            volume,
            attenuation,
            timeofs,
            entity,
            channel,
            pos,
        })
    }

    /// Inflate a zlib block after its two length words.
    fn inflate_block(reader: &mut MsgReader<'_>) -> Result<Vec<u8>, VariantError> {
        let compressed_len = usize::from(reader.word()?);
        reader.word()?;
        let compressed = reader.bytes(compressed_len)?.to_vec();
        let mut decoder = ZlibDecoder::new(&compressed[..]);
        let mut out = Vec::new();
        decoder
            .read_to_end(&mut out)
            .map_err(|err| VariantError::Zlib(err.to_string()))?;
        Ok(out)
    }

    /// Read a KEX configstring blast (`readConfigblastKex`).
    pub fn read_configblast(reader: &mut MsgReader<'_>) -> Result<Vec<KexConfigstringRecord>, VariantError> {
        let inflated = Self::inflate_block(reader)?;
        let mut cursor = MsgReader::new(&inflated);
        let mut records = Vec::new();
        while cursor.remaining() > 0 {
            let index = cursor.word()?;
            let value = cursor.string(2047);
            cursor.finish()?;
            records.push(KexConfigstringRecord { index, value });
        }
        Ok(records)
    }

    /// Read a KEX spawn baseline blast (`readSpawnbaselineblastKex`).
    pub fn read_spawnbaselineblast(&mut self, reader: &mut MsgReader<'_>) -> Result<Vec<KexBaseline>, VariantError> {
        let inflated = Self::inflate_block(reader)?;
        let mut cursor = MsgReader::new(&inflated);
        let mut out = Vec::new();
        while cursor.remaining() > 0 {
            let header = read_entity_bits_wide(&mut cursor)?;
            let state = self.read_delta_entity(&mut cursor, &EntityState::default(), header.number, header)?;
            cursor.finish()?;
            out.push(KexBaseline {
                entnum: header.number,
                state,
            });
        }
        Ok(out)
    }
}

/// Write a KEX width-selected field (`writeWidth` in `kex-write.ts`).
fn write_kex_width(writer: &mut MsgWriter, value: i32, bits: u32, byte: u32, short: u32) -> Result<(), MsgError> {
    if (bits & (byte | short)) == (byte | short) {
        writer.write_long(value)?;
    } else if (bits & short) != 0 {
        writer.write_short(value as i16)?;
    } else if (bits & byte) != 0 {
        writer.write_byte(value as u8)?;
    }
    Ok(())
}

/// Clamped color byte (`byteColor` in `kex-write.ts`).
fn kex_byte_color(value: f64) -> u8 {
    scaled_trunc(value, 255.0).clamp(0, 255) as u8
}

// ---------------------------------------------------------------------------
// zpacket
// ---------------------------------------------------------------------------

/// Minimum payload worth compressing (`ZPACKET_MIN_COMPRESS_SIZE`).
pub const ZPACKET_MIN_COMPRESS_SIZE: usize = 21;
const ZPACKET_HEADER_SIZE: usize = 5;

/// Wrap a payload in a compressed zpacket when it pays off (`tryWrapZPacket`).
///
/// Returns `None` for tiny, huge, serverdata-leading, or incompressible
/// payloads. The deflate encoding itself is implementation-defined; only
/// the header layout and the round-trip are wire guarantees.
pub fn try_wrap_zpacket(data: &[u8], max_out: usize) -> Option<Vec<u8>> {
    let len = data.len();
    if !(ZPACKET_MIN_COMPRESS_SIZE..=0xffff).contains(&len) {
        return None;
    }
    if !data.is_empty() && data[0] == protocol::Svc::Serverdata as u8 {
        return None;
    }
    let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&data[..len]).ok()?;
    let compressed = encoder.finish().ok()?;
    if compressed.len() > 0xffff {
        return None;
    }
    let total = ZPACKET_HEADER_SIZE + compressed.len();
    if total >= len || total > max_out {
        return None;
    }
    let mut out = Vec::with_capacity(total);
    out.push(protocol::SVC_ZPACKET);
    out.push((compressed.len() & 0xff) as u8);
    out.push(((compressed.len() >> 8) & 0xff) as u8);
    out.push((len & 0xff) as u8);
    out.push(((len >> 8) & 0xff) as u8);
    out.extend_from_slice(&compressed);
    Some(out)
}

/// Read a zpacket payload (`readZPacketPayload`); the opcode is consumed by the caller.
pub fn read_zpacket_payload(reader: &mut MsgReader<'_>) -> Result<Vec<u8>, VariantError> {
    let compressed_len = usize::from(reader.word()?);
    let expected = usize::from(reader.word()?);
    reader.finish()?;
    let compressed = reader.bytes(compressed_len)?.to_vec();
    reader.finish()?;
    let decoder = DeflateDecoder::new(&compressed[..]);
    let mut out = Vec::new();
    decoder
        .take(expected.saturating_add(1) as u64)
        .read_to_end(&mut out)
        .map_err(|err| VariantError::Zlib(err.to_string()))?;
    if out.len() != expected {
        return Err(VariantError::ZpacketLength {
            expected,
            found: out.len(),
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// MVD
// ---------------------------------------------------------------------------

/// MVD file magic (`MVD_MAGIC`).
pub const MVD_MAGIC: u32 = 0x3244_564d;
/// MVD protocol version (`PROTOCOL_VERSION_MVD`).
pub const PROTOCOL_VERSION_MVD: u32 = 37;
/// Minimum MVD revision.
pub const PROTOCOL_VERSION_MVD_MINIMUM: u16 = 2009;
/// Default MVD revision.
pub const PROTOCOL_VERSION_MVD_DEFAULT: u16 = 2010;
/// Extended-limits MVD revision.
pub const PROTOCOL_VERSION_MVD_EXTENDED_LIMITS: u16 = 2011;
/// Second extended-limits MVD revision.
pub const PROTOCOL_VERSION_MVD_EXTENDED_LIMITS_2: u16 = 2012;
/// Current MVD revision.
pub const PROTOCOL_VERSION_MVD_CURRENT: u16 = 2013;
/// Rerelease MVD revision.
pub const PROTOCOL_VERSION_MVD_RERELEASE: u16 = 3038;
/// MVD opcode bits (`SVCMD_BITS`).
pub const SVCMD_BITS: u32 = 5;
/// MVD opcode mask (`SVCMD_MASK`).
pub const SVCMD_MASK: u8 = 31;

/// MVD server opcodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum MvdOp {
    /// Invalid.
    Bad = 0,
    /// No operation.
    Nop = 1,
    /// Disconnect.
    Disconnect = 2,
    /// Reconnect.
    Reconnect = 3,
    /// Server data.
    Serverdata = 4,
    /// Configstring.
    Configstring = 5,
    /// Frame.
    Frame = 6,
    /// Frame without delta.
    FrameNodelta = 7,
    /// Unicast.
    Unicast = 8,
    /// Reliable unicast.
    UnicastR = 9,
    /// Multicast all.
    MulticastAll = 10,
    /// Multicast PHS.
    MulticastPhs = 11,
    /// Multicast PVS.
    MulticastPvs = 12,
    /// Reliable multicast all.
    MulticastAllR = 13,
    /// Reliable multicast PHS.
    MulticastPhsR = 14,
    /// Reliable multicast PVS.
    MulticastPvsR = 15,
    /// Sound.
    Sound = 16,
    /// Print.
    Print = 17,
    /// Stuff text.
    Stufftext = 18,
}

impl MvdOp {
    /// Decode an opcode value.
    #[must_use]
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Bad),
            1 => Some(Self::Nop),
            2 => Some(Self::Disconnect),
            3 => Some(Self::Reconnect),
            4 => Some(Self::Serverdata),
            5 => Some(Self::Configstring),
            6 => Some(Self::Frame),
            7 => Some(Self::FrameNodelta),
            8 => Some(Self::Unicast),
            9 => Some(Self::UnicastR),
            10 => Some(Self::MulticastAll),
            11 => Some(Self::MulticastPhs),
            12 => Some(Self::MulticastPvs),
            13 => Some(Self::MulticastAllR),
            14 => Some(Self::MulticastPhsR),
            15 => Some(Self::MulticastPvsR),
            16 => Some(Self::Sound),
            17 => Some(Self::Print),
            18 => Some(Self::Stufftext),
            _ => None,
        }
    }
}

/// MVD stream flag: no server messages (`MVF_NOMSGS`).
pub const MVF_NOMSGS: u16 = 1;
/// MVD stream flag: extended limits (`MVF_EXTLIMITS`).
pub const MVF_EXTLIMITS: u16 = 4;
/// MVD stream flag: second extended limits (`MVF_EXTLIMITS_2`).
pub const MVF_EXTLIMITS_2: u16 = 8;
/// MVD player-list terminator (`CLIENTNUM_NONE`).
pub const CLIENTNUM_NONE: u8 = 255;

/// MVD player flags: move type.
pub const PPS_M_TYPE: u16 = 1 << 0;
/// MVD player flags: planar origin.
pub const PPS_M_ORIGIN: u16 = 1 << 1;
/// MVD player flags: vertical origin.
pub const PPS_M_ORIGIN2: u16 = 1 << 2;
/// MVD player flags: view offset.
pub const PPS_VIEWOFFSET: u16 = 1 << 3;
/// MVD player flags: planar view angles.
pub const PPS_VIEWANGLES: u16 = 1 << 4;
/// MVD player flags: roll view angle.
pub const PPS_VIEWANGLE2: u16 = 1 << 5;
/// MVD player flags: kick angles.
pub const PPS_KICKANGLES: u16 = 1 << 6;
/// MVD player flags: blend.
pub const PPS_BLEND: u16 = 1 << 7;
/// MVD player flags: field of view.
pub const PPS_FOV: u16 = 1 << 8;
/// MVD player flags: weapon index.
pub const PPS_WEAPONINDEX: u16 = 1 << 9;
/// MVD player flags: weapon frame.
pub const PPS_WEAPONFRAME: u16 = 1 << 10;
/// MVD player flags: gun offset.
pub const PPS_GUNOFFSET: u16 = 1 << 11;
/// MVD player flags: gun angles.
pub const PPS_GUNANGLES: u16 = 1 << 12;
/// MVD player flags: refresh flags.
pub const PPS_RDFLAGS: u16 = 1 << 13;
/// MVD player flags: stats.
pub const PPS_STATS: u16 = 1 << 14;
/// MVD player flags: removal / more bits.
pub const PPS_MOREBITS: u16 = 1 << 15;

/// GTV protocol version (`GTV_PROTOCOL_VERSION`).
pub const GTV_PROTOCOL_VERSION: u16 = 0xed04;
/// Maximum GTV client message length (`MAX_GTC_MSGLEN`).
pub const MAX_GTC_MSGLEN: usize = 256;
/// GTV stream flag: deflated (`GTF_DEFLATE`).
pub const GTF_DEFLATE: u8 = 1;
/// GTV stream flag: string commands (`GTF_STRINGCMDS`).
pub const GTF_STRINGCMDS: u8 = 2;

/// GTV server opcodes (`GtvServerOpT`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum GtvServerOp {
    /// Hello.
    Hello = 0,
    /// Pong.
    Pong = 1,
    /// Stream start.
    StreamStart = 2,
    /// Stream stop.
    StreamStop = 3,
    /// Stream data.
    StreamData = 4,
    /// Error.
    Error = 5,
    /// Bad request.
    BadRequest = 6,
    /// No access.
    NoAccess = 7,
    /// Disconnect.
    Disconnect = 8,
    /// Reconnect.
    Reconnect = 9,
}

/// GTV client opcodes (`GtvClientOpT`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum GtvClientOp {
    /// Hello.
    Hello = 0,
    /// Ping.
    Ping = 1,
    /// Stream start.
    StreamStart = 2,
    /// Stream stop.
    StreamStop = 3,
    /// String command.
    Stringcmd = 4,
}

/// Parsed MVD command byte (`MvdCmdT`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MvdCmd {
    /// Opcode.
    pub op: u8,
    /// Extra bits.
    pub extrabits: u8,
}

/// Write an MVD command byte (`MSG_WriteMvdCmd`).
pub fn write_mvd_cmd(writer: &mut MsgWriter, op: u8, extrabits: u8) -> Result<(), MsgError> {
    writer.write_byte(op | (extrabits << SVCMD_BITS))
}

/// Read an MVD command byte (`MSG_ReadMvdCmd`).
pub fn read_mvd_cmd(reader: &mut MsgReader<'_>) -> Result<MvdCmd, MsgError> {
    let byte = reader.byte()?;
    Ok(MvdCmd {
        op: byte & SVCMD_MASK,
        extrabits: byte >> SVCMD_BITS,
    })
}

/// Write the MVD player-list terminator (`MSG_WriteMvdPlayersEnd`).
pub fn write_mvd_players_end(writer: &mut MsgWriter) -> Result<(), MsgError> {
    writer.write_byte(CLIENTNUM_NONE)
}

/// Whether a byte is a valid MVD client number (`MSG_ValidMvdClientNumber`).
#[must_use]
pub fn valid_mvd_client_number(number: u8) -> bool {
    number < CLIENTNUM_NONE
}

/// Parsed MVD player state (`MvdPlayerReadResultT`).
#[derive(Debug, Clone, PartialEq)]
pub struct MvdPlayer {
    /// Client number.
    pub number: u8,
    /// Player removed.
    pub removed: bool,
    /// Player state.
    pub ps: PlayerState,
}

/// Write a classic MVD player-state delta.
///
/// Returns `Ok(false)` (writing nothing) when no flags changed and `force`
/// is false, mirroring the donor's early return.
pub fn write_delta_mvd_playerstate(
    writer: &mut MsgWriter,
    from: Option<&PlayerState>,
    to: Option<&PlayerState>,
    number: u8,
    force: bool,
) -> Result<bool, VariantError> {
    if !valid_mvd_client_number(number) {
        return Err(VariantError::BadMvdLimits);
    }
    let Some(to) = to else {
        writer.write_byte(number)?;
        writer.write_short(PPS_MOREBITS as i16)?;
        return Ok(true);
    };
    let base = PlayerState::default();
    let from = from.unwrap_or(&base);
    let mut pflags = 0u16;
    if to.pmove.pm_type != from.pmove.pm_type {
        pflags |= PPS_M_TYPE;
    }
    if to.pmove.origin[0] != from.pmove.origin[0] || to.pmove.origin[1] != from.pmove.origin[1] {
        pflags |= PPS_M_ORIGIN;
    }
    if to.pmove.origin[2] != from.pmove.origin[2] {
        pflags |= PPS_M_ORIGIN2;
    }
    if to.viewoffset != from.viewoffset {
        pflags |= PPS_VIEWOFFSET;
    }
    let to_yaw = [
        angle_to_short(to.viewangles[0]),
        angle_to_short(to.viewangles[1]),
        angle_to_short(to.viewangles[2]),
    ];
    let from_yaw = [
        angle_to_short(from.viewangles[0]),
        angle_to_short(from.viewangles[1]),
        angle_to_short(from.viewangles[2]),
    ];
    if to_yaw[0] != from_yaw[0] || to_yaw[1] != from_yaw[1] {
        pflags |= PPS_VIEWANGLES;
    }
    if to_yaw[2] != from_yaw[2] {
        pflags |= PPS_VIEWANGLE2;
    }
    if to.kick_angles != from.kick_angles {
        pflags |= PPS_KICKANGLES;
    }
    if to.blend != from.blend {
        pflags |= PPS_BLEND;
    }
    if to.fov != from.fov {
        pflags |= PPS_FOV;
    }
    if to.rdflags != from.rdflags {
        pflags |= PPS_RDFLAGS;
    }
    if to.gunindex != from.gunindex {
        pflags |= PPS_WEAPONINDEX;
    }
    if to.gunframe != from.gunframe {
        pflags |= PPS_WEAPONFRAME;
    }
    if to.gunoffset != from.gunoffset {
        pflags |= PPS_GUNOFFSET;
    }
    if to.gunangles != from.gunangles {
        pflags |= PPS_GUNANGLES;
    }
    let mut statbits = 0u32;
    for i in 0..MAX_STATS {
        if to.stats[i] != from.stats[i] {
            statbits |= 1 << i;
        }
    }
    if statbits != 0 {
        pflags |= PPS_STATS;
    }
    if pflags == 0 && !force {
        return Ok(false);
    }
    writer.write_byte(number)?;
    writer.write_short(pflags as i16)?;
    if (pflags & PPS_M_TYPE) != 0 {
        writer.write_byte(to.pmove.pm_type)?;
    }
    if (pflags & PPS_M_ORIGIN) != 0 {
        writer.write_short(to.pmove.origin[0] as i16)?;
        writer.write_short(to.pmove.origin[1] as i16)?;
    }
    if (pflags & PPS_M_ORIGIN2) != 0 {
        writer.write_short(to.pmove.origin[2] as i16)?;
    }
    if (pflags & PPS_VIEWOFFSET) != 0 {
        for axis in to.viewoffset {
            writer.write_char(scaled_trunc(axis, 4.0) as i8)?;
        }
    }
    if (pflags & PPS_VIEWANGLES) != 0 {
        writer.write_short(to_yaw[0] as i16)?;
        writer.write_short(to_yaw[1] as i16)?;
    }
    if (pflags & PPS_VIEWANGLE2) != 0 {
        writer.write_short(to_yaw[2] as i16)?;
    }
    if (pflags & PPS_KICKANGLES) != 0 {
        for axis in to.kick_angles {
            writer.write_char(scaled_trunc(axis, 4.0) as i8)?;
        }
    }
    if (pflags & PPS_WEAPONINDEX) != 0 {
        writer.write_byte(to.gunindex as u8)?;
    }
    if (pflags & PPS_WEAPONFRAME) != 0 {
        writer.write_byte(to.gunframe as u8)?;
    }
    if (pflags & PPS_GUNOFFSET) != 0 {
        for axis in to.gunoffset {
            writer.write_char(scaled_trunc(axis, 4.0) as i8)?;
        }
    }
    if (pflags & PPS_GUNANGLES) != 0 {
        for axis in to.gunangles {
            writer.write_char(scaled_trunc(axis, 4.0) as i8)?;
        }
    }
    if (pflags & PPS_BLEND) != 0 {
        for axis in to.blend {
            writer.write_byte(scaled_trunc(axis, 255.0) as u8)?;
        }
    }
    if (pflags & PPS_FOV) != 0 {
        writer.write_byte(to.fov)?;
    }
    if (pflags & PPS_RDFLAGS) != 0 {
        writer.write_byte(to.rdflags)?;
    }
    if (pflags & PPS_STATS) != 0 {
        writer.write_long(statbits as i32)?;
        for i in 0..MAX_STATS {
            if (statbits & (1 << i)) != 0 {
                writer.write_short(to.stats[i])?;
            }
        }
    }
    Ok(true)
}

/// Read a classic MVD player-state body after its number.
fn read_delta_mvd_playerstate_body(
    reader: &mut MsgReader<'_>,
    from: Option<&PlayerState>,
    number: u8,
) -> Result<MvdPlayer, MsgError> {
    let pflags = reader.short()? as u16;
    let mut ps = PlayerState::default();
    if let Some(from) = from {
        ps.pmove.pm_type = from.pmove.pm_type;
        ps.pmove.origin = from.pmove.origin;
        ps.viewoffset = from.viewoffset;
        ps.viewangles = from.viewangles;
        ps.kick_angles = from.kick_angles;
        ps.gunangles = from.gunangles;
        ps.gunoffset = from.gunoffset;
        ps.gunindex = from.gunindex;
        ps.gunframe = from.gunframe;
        ps.blend = from.blend;
        ps.fov = from.fov;
        ps.rdflags = from.rdflags;
        ps.stats = from.stats;
    }
    if pflags == PPS_MOREBITS {
        return Ok(MvdPlayer {
            number,
            removed: true,
            ps,
        });
    }
    if (pflags & PPS_M_TYPE) != 0 {
        ps.pmove.pm_type = reader.byte()?;
    }
    if (pflags & PPS_M_ORIGIN) != 0 {
        ps.pmove.origin[0] = i32::from(reader.short()?);
        ps.pmove.origin[1] = i32::from(reader.short()?);
    }
    if (pflags & PPS_M_ORIGIN2) != 0 {
        ps.pmove.origin[2] = i32::from(reader.short()?);
    }
    if (pflags & PPS_VIEWOFFSET) != 0 {
        for axis in &mut ps.viewoffset {
            *axis = f64::from(reader.char()?) * 0.25;
        }
    }
    if (pflags & PPS_VIEWANGLES) != 0 {
        ps.viewangles[0] = short_to_angle(reader.short()?);
        ps.viewangles[1] = short_to_angle(reader.short()?);
    }
    if (pflags & PPS_VIEWANGLE2) != 0 {
        ps.viewangles[2] = short_to_angle(reader.short()?);
    }
    if (pflags & PPS_KICKANGLES) != 0 {
        for axis in &mut ps.kick_angles {
            *axis = f64::from(reader.char()?) * 0.25;
        }
    }
    if (pflags & PPS_WEAPONINDEX) != 0 {
        ps.gunindex = i32::from(reader.byte()?);
    }
    if (pflags & PPS_WEAPONFRAME) != 0 {
        ps.gunframe = i32::from(reader.byte()?);
    }
    if (pflags & PPS_GUNOFFSET) != 0 {
        for axis in &mut ps.gunoffset {
            *axis = f64::from(reader.char()?) * 0.25;
        }
    }
    if (pflags & PPS_GUNANGLES) != 0 {
        for axis in &mut ps.gunangles {
            *axis = f64::from(reader.char()?) * 0.25;
        }
    }
    if (pflags & PPS_BLEND) != 0 {
        for axis in &mut ps.blend {
            *axis = f64::from(reader.byte()?) / 255.0;
        }
    }
    if (pflags & PPS_FOV) != 0 {
        ps.fov = reader.byte()?;
    }
    if (pflags & PPS_RDFLAGS) != 0 {
        ps.rdflags = reader.byte()?;
    }
    if (pflags & PPS_STATS) != 0 {
        let statbits = reader.long()? as u32;
        for i in 0..MAX_STATS {
            if (statbits & (1 << i)) != 0 {
                ps.stats[i] = reader.short()?;
            }
        }
    }
    Ok(MvdPlayer {
        number,
        removed: false,
        ps,
    })
}

/// Read a classic MVD player-state delta (`MSG_ReadDeltaMvdPlayerstate`).
pub fn read_delta_mvd_playerstate(
    reader: &mut MsgReader<'_>,
    from: Option<&PlayerState>,
) -> Result<MvdPlayer, MsgError> {
    let number = reader.byte()?;
    read_delta_mvd_playerstate_body(reader, from, number)
}

/// MVD rerelease stat slots (`MAX_STATS_NEW`).
pub const MAX_STATS_NEW: usize = 64;
/// MVD rerelease gun-index bits (`GUNINDEX_BITS`).
pub const MVD_GUNINDEX_BITS: u32 = 13;

/// Pack an MVD rerelease gun index + skin.
fn packed_mvd_gun_index(ps: &PlayerState) -> u16 {
    ((ps.gunindex & 0x1fff) | ((ps.gunskin & 0x7) << MVD_GUNINDEX_BITS)) as u16
}

/// Write an MVD rerelease blend delta (`writeDeltaBlend`).
fn write_mvd_delta_blend(writer: &mut MsgWriter, from: &PlayerState, to: &PlayerState) -> Result<(), MsgError> {
    let mut bflags = 0u8;
    for i in 0..4 {
        if scaled_trunc(to.blend[i], 255.0).clamp(0, 255) != scaled_trunc(from.blend[i], 255.0).clamp(0, 255) {
            bflags |= 1 << i;
        }
        if scaled_trunc(to.damage_blend[i], 255.0).clamp(0, 255)
            != scaled_trunc(from.damage_blend[i], 255.0).clamp(0, 255)
        {
            bflags |= 1 << (4 + i);
        }
    }
    writer.write_byte(bflags)?;
    for i in 0..4 {
        if (bflags & (1 << i)) != 0 {
            writer.write_byte(scaled_trunc(to.blend[i], 255.0).clamp(0, 255) as u8)?;
        }
    }
    for i in 0..4 {
        if (bflags & (1 << (4 + i))) != 0 {
            writer.write_byte(scaled_trunc(to.damage_blend[i], 255.0).clamp(0, 255) as u8)?;
        }
    }
    Ok(())
}

/// Read an MVD rerelease blend delta (`readDeltaBlend`).
fn read_mvd_delta_blend(reader: &mut MsgReader<'_>, ps: &mut PlayerState) -> Result<(), MsgError> {
    let bflags = reader.byte()?;
    for i in 0..4 {
        if (bflags & (1 << i)) != 0 {
            ps.blend[i] = f64::from(reader.byte()?) / 255.0;
        }
    }
    for i in 0..4 {
        if (bflags & (1 << (4 + i))) != 0 {
            ps.damage_blend[i] = f64::from(reader.byte()?) / 255.0;
        }
    }
    Ok(())
}

/// Whether an MVD rerelease blend changed (`blendChanged`).
fn mvd_blend_changed(from: &PlayerState, to: &PlayerState) -> bool {
    (0..4).any(|i| {
        scaled_trunc(to.blend[i], 255.0).clamp(0, 255) != scaled_trunc(from.blend[i], 255.0).clamp(0, 255)
            || scaled_trunc(to.damage_blend[i], 255.0).clamp(0, 255)
                != scaled_trunc(from.damage_blend[i], 255.0).clamp(0, 255)
    })
}

/// Write an MVD rerelease player-state delta.
///
/// Returns `Ok(false)` (writing nothing) when no flags changed and `force`
/// is false, mirroring the donor's early return.
pub fn write_delta_mvd_playerstate_rerelease(
    writer: &mut MsgWriter,
    from: Option<&PlayerState>,
    to: Option<&PlayerState>,
    number: u8,
    force: bool,
) -> Result<bool, VariantError> {
    if !valid_mvd_client_number(number) {
        return Err(VariantError::BadMvdLimits);
    }
    let Some(to) = to else {
        writer.write_byte(number)?;
        writer.write_short(PPS_MOREBITS as i16)?;
        return Ok(true);
    };
    let base = PlayerState::default();
    let from = from.unwrap_or(&base);
    let mut pflags = 0u16;
    if to.pmove.pm_type != from.pmove.pm_type {
        pflags |= PPS_M_TYPE;
    }
    if to.pmove.origin[0] != from.pmove.origin[0] || to.pmove.origin[1] != from.pmove.origin[1] {
        pflags |= PPS_M_ORIGIN;
    }
    if to.pmove.origin[2] != from.pmove.origin[2] {
        pflags |= PPS_M_ORIGIN2;
    }
    let to_viewoffset = [
        encode_fixed16(to.viewoffset[0], RR_VIEWOFFSET_SCALE),
        encode_fixed16(to.viewoffset[1], RR_VIEWOFFSET_SCALE),
        encode_fixed16(to.viewoffset[2], RR_VIEWOFFSET_SCALE),
    ];
    let from_viewoffset = [
        encode_fixed16(from.viewoffset[0], RR_VIEWOFFSET_SCALE),
        encode_fixed16(from.viewoffset[1], RR_VIEWOFFSET_SCALE),
        encode_fixed16(from.viewoffset[2], RR_VIEWOFFSET_SCALE),
    ];
    if to_viewoffset != from_viewoffset {
        pflags |= PPS_VIEWOFFSET;
    }
    let to_yaw = [
        angle_to_short(to.viewangles[0]),
        angle_to_short(to.viewangles[1]),
        angle_to_short(to.viewangles[2]),
    ];
    let from_yaw = [
        angle_to_short(from.viewangles[0]),
        angle_to_short(from.viewangles[1]),
        angle_to_short(from.viewangles[2]),
    ];
    if to_yaw[0] != from_yaw[0] || to_yaw[1] != from_yaw[1] {
        pflags |= PPS_VIEWANGLES;
    }
    if to_yaw[2] != from_yaw[2] {
        pflags |= PPS_VIEWANGLE2;
    }
    let to_kick = [
        encode_fixed16(to.kick_angles[0], RR_KICK_ANGLE_SCALE),
        encode_fixed16(to.kick_angles[1], RR_KICK_ANGLE_SCALE),
        encode_fixed16(to.kick_angles[2], RR_KICK_ANGLE_SCALE),
    ];
    let from_kick = [
        encode_fixed16(from.kick_angles[0], RR_KICK_ANGLE_SCALE),
        encode_fixed16(from.kick_angles[1], RR_KICK_ANGLE_SCALE),
        encode_fixed16(from.kick_angles[2], RR_KICK_ANGLE_SCALE),
    ];
    if to_kick != from_kick {
        pflags |= PPS_KICKANGLES;
    }
    if mvd_blend_changed(from, to) {
        pflags |= PPS_BLEND;
    }
    if to.fov != from.fov {
        pflags |= PPS_FOV;
    }
    if to.rdflags != from.rdflags {
        pflags |= PPS_RDFLAGS;
    }
    let to_gun = packed_mvd_gun_index(to);
    let from_gun = packed_mvd_gun_index(from);
    if to_gun != from_gun {
        pflags |= PPS_WEAPONINDEX;
    }
    if to.gunframe != from.gunframe {
        pflags |= PPS_WEAPONFRAME;
    }
    let to_gunoffset = [
        encode_fixed16(to.gunoffset[0], RR_GUNOFFSET_SCALE),
        encode_fixed16(to.gunoffset[1], RR_GUNOFFSET_SCALE),
        encode_fixed16(to.gunoffset[2], RR_GUNOFFSET_SCALE),
    ];
    let from_gunoffset = [
        encode_fixed16(from.gunoffset[0], RR_GUNOFFSET_SCALE),
        encode_fixed16(from.gunoffset[1], RR_GUNOFFSET_SCALE),
        encode_fixed16(from.gunoffset[2], RR_GUNOFFSET_SCALE),
    ];
    if to_gunoffset != from_gunoffset {
        pflags |= PPS_GUNOFFSET;
    }
    let to_gunangles = [
        encode_fixed16(to.gunangles[0], RR_GUNANGLE_SCALE),
        encode_fixed16(to.gunangles[1], RR_GUNANGLE_SCALE),
        encode_fixed16(to.gunangles[2], RR_GUNANGLE_SCALE),
    ];
    let from_gunangles = [
        encode_fixed16(from.gunangles[0], RR_GUNANGLE_SCALE),
        encode_fixed16(from.gunangles[1], RR_GUNANGLE_SCALE),
        encode_fixed16(from.gunangles[2], RR_GUNANGLE_SCALE),
    ];
    if to_gunangles != from_gunangles {
        pflags |= PPS_GUNANGLES;
    }
    let mut statbits = 0u64;
    for i in 0..MAX_STATS_NEW {
        if to.stats[i] != from.stats[i] {
            statbits |= 1 << i;
        }
    }
    if statbits != 0 {
        pflags |= PPS_STATS;
    }
    if pflags == 0 && !force {
        return Ok(false);
    }
    writer.write_byte(number)?;
    writer.write_short(pflags as i16)?;
    if (pflags & PPS_M_TYPE) != 0 {
        writer.write_byte(to.pmove.pm_type)?;
    }
    if (pflags & PPS_M_ORIGIN) != 0 {
        writer.write_short(to.pmove.origin[0] as i16)?;
        writer.write_short(to.pmove.origin[1] as i16)?;
    }
    if (pflags & PPS_M_ORIGIN2) != 0 {
        writer.write_short(to.pmove.origin[2] as i16)?;
    }
    if (pflags & PPS_VIEWOFFSET) != 0 {
        for axis in to_viewoffset {
            writer.write_short(axis)?;
        }
    }
    if (pflags & PPS_VIEWANGLES) != 0 {
        writer.write_short(to_yaw[0] as i16)?;
        writer.write_short(to_yaw[1] as i16)?;
    }
    if (pflags & PPS_VIEWANGLE2) != 0 {
        writer.write_short(to_yaw[2] as i16)?;
    }
    if (pflags & PPS_KICKANGLES) != 0 {
        for axis in to_kick {
            writer.write_short(axis)?;
        }
    }
    if (pflags & PPS_WEAPONINDEX) != 0 {
        writer.write_short(to_gun as i16)?;
    }
    if (pflags & PPS_WEAPONFRAME) != 0 {
        writer.write_short(to.gunframe as i16)?;
    }
    if (pflags & PPS_GUNOFFSET) != 0 {
        for axis in to_gunoffset {
            writer.write_short(axis)?;
        }
    }
    if (pflags & PPS_GUNANGLES) != 0 {
        for axis in to_gunangles {
            writer.write_short(axis)?;
        }
    }
    if (pflags & PPS_BLEND) != 0 {
        write_mvd_delta_blend(writer, from, to)?;
    }
    if (pflags & PPS_FOV) != 0 {
        writer.write_byte(to.fov)?;
    }
    if (pflags & PPS_RDFLAGS) != 0 {
        writer.write_byte(to.rdflags)?;
    }
    if (pflags & PPS_STATS) != 0 {
        writer.write_long64(statbits as i64)?;
        for i in 0..MAX_STATS_NEW {
            if (statbits & (1 << i)) != 0 {
                writer.write_short(to.stats[i])?;
            }
        }
    }
    Ok(true)
}

/// Read an MVD rerelease player-state body after its number.
fn read_delta_mvd_playerstate_rerelease_body(
    reader: &mut MsgReader<'_>,
    from: Option<&PlayerState>,
    number: u8,
) -> Result<MvdPlayer, MsgError> {
    let pflags = reader.short()? as u16;
    let mut ps = PlayerState::default();
    if let Some(from) = from {
        ps.pmove.pm_type = from.pmove.pm_type;
        ps.pmove.origin = from.pmove.origin;
        ps.viewoffset = from.viewoffset;
        ps.viewangles = from.viewangles;
        ps.kick_angles = from.kick_angles;
        ps.gunangles = from.gunangles;
        ps.gunoffset = from.gunoffset;
        ps.gunindex = from.gunindex;
        ps.gunskin = from.gunskin;
        ps.gunframe = from.gunframe;
        ps.blend = from.blend;
        ps.damage_blend = from.damage_blend;
        ps.fov = from.fov;
        ps.rdflags = from.rdflags;
        ps.stats = from.stats;
    }
    if pflags == PPS_MOREBITS {
        return Ok(MvdPlayer {
            number,
            removed: true,
            ps,
        });
    }
    if (pflags & PPS_M_TYPE) != 0 {
        ps.pmove.pm_type = reader.byte()?;
    }
    if (pflags & PPS_M_ORIGIN) != 0 {
        ps.pmove.origin[0] = i32::from(reader.short()?);
        ps.pmove.origin[1] = i32::from(reader.short()?);
    }
    if (pflags & PPS_M_ORIGIN2) != 0 {
        ps.pmove.origin[2] = i32::from(reader.short()?);
    }
    if (pflags & PPS_VIEWOFFSET) != 0 {
        for axis in &mut ps.viewoffset {
            *axis = f64::from(reader.short()?) / RR_VIEWOFFSET_SCALE;
        }
    }
    if (pflags & PPS_VIEWANGLES) != 0 {
        ps.viewangles[0] = short_to_angle(reader.short()?);
        ps.viewangles[1] = short_to_angle(reader.short()?);
    }
    if (pflags & PPS_VIEWANGLE2) != 0 {
        ps.viewangles[2] = short_to_angle(reader.short()?);
    }
    if (pflags & PPS_KICKANGLES) != 0 {
        for axis in &mut ps.kick_angles {
            *axis = f64::from(reader.short()?) / RR_KICK_ANGLE_SCALE;
        }
    }
    if (pflags & PPS_WEAPONINDEX) != 0 {
        let packed = reader.short()? as u16;
        ps.gunindex = i32::from(packed & 0x1fff);
        ps.gunskin = i32::from((packed >> MVD_GUNINDEX_BITS) & 0x7);
    }
    if (pflags & PPS_WEAPONFRAME) != 0 {
        ps.gunframe = i32::from(reader.word()?);
    }
    if (pflags & PPS_GUNOFFSET) != 0 {
        for axis in &mut ps.gunoffset {
            *axis = f64::from(reader.short()?) / RR_GUNOFFSET_SCALE;
        }
    }
    if (pflags & PPS_GUNANGLES) != 0 {
        for axis in &mut ps.gunangles {
            *axis = f64::from(reader.short()?) / RR_GUNANGLE_SCALE;
        }
    }
    if (pflags & PPS_BLEND) != 0 {
        read_mvd_delta_blend(reader, &mut ps)?;
    }
    if (pflags & PPS_FOV) != 0 {
        ps.fov = reader.byte()?;
    }
    if (pflags & PPS_RDFLAGS) != 0 {
        ps.rdflags = reader.byte()?;
    }
    if (pflags & PPS_STATS) != 0 {
        let statbits = reader.long64()? as u64;
        for i in 0..MAX_STATS_NEW {
            if (statbits & (1 << i)) != 0 {
                ps.stats[i] = reader.short()?;
            }
        }
    }
    Ok(MvdPlayer {
        number,
        removed: false,
        ps,
    })
}

/// Read an MVD rerelease player-state delta (`MSG_ReadDeltaMvdPlayerstateRerelease`).
pub fn read_delta_mvd_playerstate_rerelease(
    reader: &mut MsgReader<'_>,
    from: Option<&PlayerState>,
) -> Result<MvdPlayer, MsgError> {
    let number = reader.byte()?;
    read_delta_mvd_playerstate_rerelease_body(reader, from, number)
}

/// MVD stream protocol identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MvdProtocol {
    /// Classic protocol 34.
    Classic,
    /// Q2Pro protocol 36 at a revision.
    Q2Pro {
        /// Q2Pro revision.
        revision: u16,
    },
    /// Rerelease protocol 1038.
    Rerelease,
}

/// MVD stream profile (`MvdProfile`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MvdProfile {
    /// Stream revision.
    pub revision: u16,
    /// Stream flags.
    pub flags: u16,
    /// Protocol identity.
    pub protocol: MvdProtocol,
    /// Rerelease stream.
    pub rerelease: bool,
    /// Extended-limits stream.
    pub extended: bool,
    /// Second-generation extended stream.
    pub v2: bool,
    /// Fog fields present.
    pub fog: bool,
    /// Configstring space.
    pub max_config_strings: usize,
    /// `maxclients` configstring index.
    pub max_clients_index: usize,
    /// Entity space.
    pub max_entities: usize,
}

/// Derive an MVD stream profile (`mvdProfile`).
pub fn mvd_profile(revision: u16, flags: u16) -> Result<MvdProfile, VariantError> {
    if !((2009..=2013).contains(&revision) || revision == PROTOCOL_VERSION_MVD_RERELEASE) {
        return Err(VariantError::UnsupportedMvdRevision(revision));
    }
    let rerelease = revision == PROTOCOL_VERSION_MVD_RERELEASE;
    let extended = rerelease || (revision >= 2011 && (flags & MVF_EXTLIMITS) != 0);
    let v2 = !rerelease && revision >= 2012 && (flags & MVF_EXTLIMITS_2) != 0;
    if v2 && !extended {
        return Err(VariantError::MvdV2NeedsExtended);
    }
    let protocol = if rerelease {
        MvdProtocol::Rerelease
    } else if extended {
        MvdProtocol::Q2Pro {
            revision: if revision == 2013 {
                1026
            } else if v2 {
                1025
            } else {
                1024
            },
        }
    } else {
        MvdProtocol::Classic
    };
    Ok(MvdProfile {
        revision,
        flags,
        protocol,
        rerelease,
        extended,
        v2,
        fog: v2 && revision >= 2013,
        max_config_strings: if rerelease {
            12448
        } else if extended {
            13630
        } else {
            2080
        },
        max_clients_index: if extended { 60 } else { 30 },
        max_entities: if extended { 8192 } else { 1024 },
    })
}

/// Parsed MVD gamestate header (`MvdHeader`).
#[derive(Debug, Clone, PartialEq)]
pub struct MvdHeader {
    /// Stream profile.
    pub profile: MvdProfile,
    /// Server count.
    pub servercount: i32,
    /// Game directory.
    pub gamedir: String,
    /// Level name (configstring 0).
    pub levelname: String,
    /// Configstrings by index.
    pub config_strings: BTreeMap<u16, String>,
    /// Offset where frames start.
    pub frame_offset: usize,
    /// Dummy client.
    pub dummy: i16,
    /// Player slots.
    pub max_clients: usize,
    /// Q2Pro revision for extended streams.
    pub q2pro_version: Option<u16>,
    /// Q2Pro wire flags for extended streams.
    pub wire_flags: Option<u16>,
}

/// Read an MVD gamestate header (`readMvdHeader`).
pub fn read_mvd_header(reader: &mut MsgReader<'_>) -> Result<MvdHeader, VariantError> {
    let command = reader.byte()?;
    if (command & SVCMD_MASK) != MvdOp::Serverdata as u8 || reader.long()? != PROTOCOL_VERSION_MVD as i32 {
        return Err(VariantError::BadMvdHeader);
    }
    let revision = reader.word()?;
    let flags = if revision >= 2012 && revision != PROTOCOL_VERSION_MVD_RERELEASE {
        reader.word()?
    } else {
        u16::from(command >> SVCMD_BITS)
    };
    let profile = mvd_profile(revision, flags)?;
    let servercount = reader.long()?;
    let gamedir = reader.string(2047);
    let dummy = reader.short()?;
    let mut config_strings = BTreeMap::new();
    loop {
        let index = reader.word()?;
        reader.finish()?;
        if usize::from(index) == profile.max_config_strings {
            break;
        }
        if usize::from(index) > profile.max_config_strings {
            return Err(VariantError::BadConfigstringIndex(index));
        }
        config_strings.insert(index, reader.string(2047));
    }
    let max_clients = config_strings
        .get(&(profile.max_clients_index as u16))
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.fract() == 0.0 && (1.0..=256.0).contains(value))
        .map(|value| value as usize);
    let Some(max_clients) = max_clients else {
        return Err(VariantError::BadMvdLimits);
    };
    if dummy < -1 || dummy >= max_clients as i16 {
        return Err(VariantError::BadMvdLimits);
    }
    reader.finish()?;
    let levelname = config_strings.get(&0).cloned().unwrap_or_default();
    let (q2pro_version, wire_flags) = match profile.protocol {
        MvdProtocol::Q2Pro { revision } => (Some(revision), Some(if profile.v2 { 24 } else { 8 })),
        _ => (None, None),
    };
    Ok(MvdHeader {
        profile,
        servercount,
        gamedir,
        levelname,
        config_strings,
        frame_offset: reader.offset(),
        dummy,
        max_clients,
        q2pro_version,
        wire_flags,
    })
}

/// Read one native packet player under an MVD profile (`readMvdPlayer`).
pub fn read_mvd_player(
    reader: &mut MsgReader<'_>,
    from: Option<&PlayerState>,
    number: u8,
    profile: &MvdProfile,
) -> Result<MvdPlayer, VariantError> {
    if !profile.extended {
        return Ok(read_delta_mvd_playerstate_body(reader, from, number)?);
    }
    let mut flags = u32::from(reader.word()?);
    if (flags & u32::from(PPS_MOREBITS)) != 0 {
        if profile.fog {
            flags |= u32::from(reader.byte()?) << 16;
        } else {
            return Ok(MvdPlayer {
                number,
                removed: true,
                ps: from.cloned().unwrap_or_default(),
            });
        }
    }
    let mut ps = PlayerState::default();
    if let Some(from) = from {
        ps.pmove.pm_type = from.pmove.pm_type;
        ps.pmove.origin = from.pmove.origin;
        ps.pmove.origin_f = from.pmove.origin_f;
        ps.viewoffset = from.viewoffset;
        ps.viewangles = from.viewangles;
        ps.kick_angles = from.kick_angles;
        ps.gunoffset = from.gunoffset;
        ps.gunangles = from.gunangles;
        ps.gunindex = from.gunindex;
        ps.gunskin = from.gunskin;
        ps.gunframe = from.gunframe;
        ps.blend = from.blend;
        ps.damage_blend = from.damage_blend;
        ps.stats = from.stats;
        ps.fov = from.fov;
        ps.rdflags = from.rdflags;
        ps.fog = from.fog.clone();
    }
    for axis in 0..3 {
        let flag = if axis == 2 { PPS_M_ORIGIN2 } else { PPS_M_ORIGIN };
        if (flags & u32::from(flag)) != 0 {
            if profile.rerelease {
                ps.pmove.origin_f[axis] = reader.float()?;
            } else if profile.v2 {
                ps.pmove.origin[axis] = read_q2pro_int23(reader, ps.pmove.origin[axis])?;
            } else {
                ps.pmove.origin[axis] = i32::from(reader.short()?);
            }
        }
    }
    if (flags & u32::from(PPS_VIEWOFFSET)) != 0 {
        for axis in &mut ps.viewoffset {
            *axis = if profile.rerelease {
                f64::from(reader.short()?) / RR_VIEWOFFSET_SCALE
            } else {
                f64::from(reader.char()?) * 0.25
            };
        }
    }
    if (flags & u32::from(PPS_VIEWANGLES)) != 0 {
        ps.viewangles[0] = short_to_angle(reader.short()?);
        ps.viewangles[1] = short_to_angle(reader.short()?);
    }
    if (flags & u32::from(PPS_VIEWANGLE2)) != 0 {
        ps.viewangles[2] = short_to_angle(reader.short()?);
    }
    if (flags & u32::from(PPS_KICKANGLES)) != 0 {
        for axis in &mut ps.kick_angles {
            *axis = if profile.rerelease {
                f64::from(reader.short()?) / RR_KICK_ANGLE_SCALE
            } else {
                f64::from(reader.char()?) * 0.25
            };
        }
    }
    if (flags & u32::from(PPS_WEAPONINDEX)) != 0 {
        let packed = reader.word()?;
        ps.gunindex = i32::from(packed & 8191);
        ps.gunskin = i32::from(packed >> 13);
    }
    if (flags & u32::from(PPS_WEAPONFRAME)) != 0 {
        ps.gunframe = if profile.rerelease {
            i32::from(reader.word()?)
        } else {
            i32::from(reader.byte()?)
        };
    }
    if (flags & u32::from(PPS_GUNOFFSET)) != 0 {
        for axis in &mut ps.gunoffset {
            *axis = if profile.rerelease {
                f64::from(reader.short()?) / RR_GUNOFFSET_SCALE
            } else {
                f64::from(reader.short()?) / 8.0
            };
        }
    }
    if (flags & u32::from(PPS_GUNANGLES)) != 0 {
        for axis in &mut ps.gunangles {
            *axis = if profile.rerelease {
                f64::from(reader.short()?) / RR_GUNANGLE_SCALE
            } else {
                f64::from(reader.short()?) / (65536.0 / 360.0)
            };
        }
    }
    if (flags & u32::from(PPS_BLEND)) != 0 {
        if profile.rerelease || profile.v2 {
            read_mvd_delta_blend(reader, &mut ps)?;
        } else {
            for axis in &mut ps.blend {
                *axis = f64::from(reader.byte()?) / 255.0;
            }
        }
    }
    if (flags & (1 << 17)) != 0 {
        ps.fog = read_q2pro_fog(reader, &ps.fog)?;
    }
    if (flags & u32::from(PPS_FOV)) != 0 {
        ps.fov = reader.byte()?;
    }
    if (flags & u32::from(PPS_RDFLAGS)) != 0 {
        ps.rdflags = reader.byte()?;
    }
    if (flags & u32::from(PPS_STATS)) != 0 {
        let wide = profile.rerelease || profile.v2;
        let statbits = if profile.rerelease {
            reader.long64()? as u64
        } else if profile.v2 {
            read_q2pro_var64(reader)?
        } else {
            u64::from(reader.long()? as u32)
        };
        for i in 0..if wide { MAX_STATS_STORAGE } else { MAX_STATS } {
            if (statbits & (1 << i)) != 0 {
                ps.stats[i] = reader.short()?;
            }
        }
    }
    let removed = (flags & (1 << 16)) != 0;
    Ok(MvdPlayer { number, removed, ps })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q2::Q2ProFog;

    fn decode_hex(text: &str) -> Vec<u8> {
        assert!(text.len().is_multiple_of(2), "odd hex length");
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
            .collect()
    }

    fn writer() -> MsgWriter {
        MsgWriter::new(65536, false)
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-9,
            "actual {actual} != expected {expected}"
        );
    }

    fn r1_entity() -> EntityState {
        EntityState {
            number: 300,
            origin: [100.0, -50.5, 0.0],
            angles: [0.0, 90.0, 0.0],
            old_origin: [1.0, 2.0, 3.0],
            modelindex: 5,
            frame: 300,
            skinnum: 70000,
            effects: 0x9000,
            renderfx: 41,
            solid: 0x12345678,
            sound: 9,
            event: 7,
            ..EntityState::default()
        }
    }

    fn r1_player() -> PlayerState {
        let mut ps = PlayerState::default();
        ps.pmove.pm_type = 1;
        ps.pmove.origin = [800, -400, 120];
        ps.pmove.velocity = [10, -20, 30];
        ps.pmove.pm_time = 5;
        ps.pmove.pm_flags = 3;
        ps.pmove.gravity = 800;
        ps.pmove.delta_angles = [100, 200, 300];
        ps.viewoffset = [0.25, -0.5, 1.0];
        ps.viewangles = [10.0, 20.0, 30.0];
        ps.kick_angles = [1.0, 2.0, 3.0];
        ps.gunindex = 5;
        ps.gunframe = 7;
        ps.gunoffset = [0.5, 0.5, 0.5];
        ps.gunangles = [4.0, 5.0, 6.0];
        ps.blend = [0.1, 0.2, 0.3, 0.4];
        ps.fov = 90;
        ps.rdflags = 1;
        ps.stats[0] = 100;
        ps.stats[5] = -3;
        ps
    }

    fn r1_cmd() -> Usercmd {
        Usercmd {
            msec: 50,
            buttons: 3,
            angles: [640, 512, 300],
            forwardmove: 400,
            sidemove: 33,
            upmove: 10,
            impulse: 8,
            lightlevel: 200,
            server_frame: 0,
        }
    }

    fn q2pro_fog() -> Q2ProFog {
        Q2ProFog {
            color: [10, 20, 30],
            density: 100,
            sky_factor: 200,
            height_density: 5,
            height_falloff: 6,
            height_start_color: [1, 2, 3],
            height_end_color: [4, 5, 6],
            height_start_distance: 800,
            height_end_distance: -1600,
        }
    }

    fn q2pro_codec() -> Q2ProCodec {
        Q2ProCodec::new(Q2ProFeatures {
            revision: 1026,
            flags: 24,
        })
    }

    fn q2pro_entity() -> EntityState {
        EntityState {
            number: 300,
            origin: [100.5, -50.25, 12.125],
            angles: [0.0, 90.0, 45.0],
            old_origin: [1.0, 2.0, 3.0],
            modelindex: 300,
            frame: 300,
            skinnum: 70000,
            effects: 0x120000,
            renderfx: 0x3456,
            morefx: 0x9abcdef,
            alpha: 0.5,
            scale: 2.0,
            solid: 0x11223344,
            sound: 300,
            loop_volume: 0.5,
            loop_attenuation: 2.0,
            event: 7,
            ..EntityState::default()
        }
    }

    fn q2pro_player() -> PlayerState {
        let mut ps = r1_player();
        ps.clientnum = 300;
        ps.pmove.pm_type = 2;
        ps.pmove.pm_time = 300;
        ps.pmove.pm_flags = 400;
        ps.pmove.delta_angles = [1, 2, 3];
        ps.viewoffset = [0.25, 0.5, 0.75];
        ps.gunindex = 300;
        ps.gunskin = 3;
        ps.gunframe = 9;
        ps.gunoffset = [0.1, 0.2, 0.3];
        ps.gunangles = [1.0, 2.0, 3.0];
        ps.damage_blend = [0.5, 0.0, 0.0, 0.0];
        ps.stats[5] = 0;
        ps.stats[40] = -5;
        ps.fog = q2pro_fog();
        ps
    }

    #[test]
    fn r1q2_serverdata_byte_exact() {
        let codec = R1q2Codec::new(1905);
        let params = R1q2ServerData {
            servercount: 7,
            attractloop: false,
            gamedir: "baseq2".to_string(),
            clientnum: 2,
            levelname: "q2dm1".to_string(),
            version: 1905,
            strafejump_hack: true,
        };
        let mut out = writer();
        codec.write_server_data(&mut out, &params).unwrap();
        assert_eq!(
            out.bytes(),
            decode_hex("0c2300000007000000006261736571320002007132646d31000071070001").as_slice()
        );
        let mut reader = MsgReader::new(out.bytes());
        assert_eq!(reader.byte().unwrap(), protocol::Svc::Serverdata as u8);
        assert_eq!(reader.long().unwrap(), 35);
        assert_eq!(R1q2Codec::read_server_data(&mut reader).unwrap(), params);
        reader.finish().unwrap();
    }

    #[test]
    fn r1q2_entity_byte_exact() {
        let codec = R1q2Codec::new(1905);
        assert!(codec.long_solid());
        let mut out = writer();
        assert!(codec
            .write_delta_entity(&mut out, &EntityState::default(), &r1_entity(), false, true)
            .unwrap());
        assert_eq!(
            out.bytes(),
            decode_hex("a7d98b0f2c01052c0170110100009000002920036cfe40080010001800090778563412").as_slice()
        );
        let mut reader = MsgReader::new(out.bytes());
        let (number, bits) = crate::q2::read_entity_bits(&mut reader).unwrap();
        assert_eq!(number, 300);
        let decoded = codec
            .read_delta_entity(&mut reader, &EntityState::default(), number, bits)
            .unwrap();
        reader.finish().unwrap();
        assert_eq!(decoded.origin, [100.0, -50.5, 0.0]);
        assert_eq!(decoded.angles, [0.0, 90.0, 0.0]);
        assert_eq!(decoded.old_origin, [1.0, 2.0, 3.0]);
        assert_eq!(decoded.modelindex, 5);
        assert_eq!(decoded.frame, 300);
        assert_eq!(decoded.skinnum, 70000);
        assert_eq!(decoded.effects, 0x9000);
        assert_eq!(decoded.solid, 0x12345678);
        assert_eq!(decoded.sound, 9);
        assert_eq!(decoded.event, 7);
    }

    #[test]
    fn r1q2_entity_short_solid_round_trip() {
        let codec = R1q2Codec::new(1903);
        assert!(!codec.long_solid());
        assert!(!codec.compressed_movements());
        let to = EntityState {
            number: 9,
            solid: 0x1234,
            ..Default::default()
        };
        let mut out = writer();
        codec
            .write_delta_entity(&mut out, &EntityState::default(), &to, false, false)
            .unwrap();
        let mut reader = MsgReader::new(out.bytes());
        let (number, bits) = crate::q2::read_entity_bits(&mut reader).unwrap();
        let decoded = codec
            .read_delta_entity(&mut reader, &EntityState::default(), number, bits)
            .unwrap();
        assert_eq!(decoded.solid, 0x1234);
    }

    #[test]
    fn r1q2_playerstate_byte_exact() {
        let mut out = writer();
        R1q2Codec::write_player_state_delta(&mut out, &PlayerState::default(), &r1_player()).unwrap();
        assert_eq!(out.bytes(), decode_hex("ff7f3f01200370fe78000a00ecff1e00050320036400c8002c0101fe041c07380e551504080c050702020210141819334c665a01210000006400fdff").as_slice());
        let mut reader = MsgReader::new(out.bytes());
        let decoded = R1q2Codec::read_player_state_delta(&mut reader, &PlayerState::default()).unwrap();
        reader.finish().unwrap();
        assert_eq!(decoded.pmove.pm_type, 1);
        assert_eq!(decoded.pmove.origin, [800, -400, 120]);
        assert_eq!(decoded.pmove.velocity, [10, -20, 30]);
        assert_eq!(decoded.pmove.pm_time, 5);
        assert_eq!(decoded.pmove.pm_flags, 3);
        assert_eq!(decoded.pmove.gravity, 800);
        assert_eq!(decoded.pmove.delta_angles, [100, 200, 300]);
        assert_eq!(decoded.viewoffset, [0.25, -0.5, 1.0]);
        for i in 0..3 {
            assert_close(
                decoded.viewangles[i],
                short_to_angle(angle_to_short([10.0, 20.0, 30.0][i]) as i16),
            );
        }
        assert_eq!(decoded.gunindex, 5);
        assert_eq!(decoded.gunframe, 7);
        assert_eq!(decoded.fov, 90);
        assert_eq!(decoded.rdflags, 1);
        assert_eq!(decoded.stats[0], 100);
        assert_eq!(decoded.stats[5], -3);
    }

    #[test]
    fn r1q2_frame_byte_exact() {
        let ps = PlayerState {
            viewangles: [45.0, 0.0, 0.0],
            gunindex: 3,
            ..Default::default()
        };
        let mut out = writer();
        R1q2Codec::write_frame(
            &mut out,
            &FrameWrite {
                framenum: 100,
                lastframe: 98,
                surpress_count: 3,
                areabits: &[0xaa, 0x55],
                ps_from: None,
                ps_to: &ps,
            },
            |w| w.write_short(0).map_err(VariantError::from),
        )
        .unwrap();
        assert_eq!(
            out.bytes(),
            decode_hex("14640000100302aa55001100200000030000").as_slice()
        );
        let mut codec = R1q2Codec::new(1905);
        let mut reader = MsgReader::new(out.bytes());
        let opcode = reader.byte().unwrap();
        assert_eq!(opcode, protocol::Svc::Frame as u8);
        codec.set_frame_extrabits(opcode & !31);
        let mut areas = Vec::new();
        let header = codec.read_frame_header(&mut reader, &mut areas).unwrap();
        assert_eq!(
            header,
            FrameHeader {
                serverframe: 100,
                deltaframe: 98,
                surpress_count: 3,
                areabytes: 2,
            }
        );
        assert_eq!(areas, vec![0xaa, 0x55]);
        let decoded = codec
            .read_frame_playerstate(&mut reader, &PlayerState::default())
            .unwrap();
        assert_close(decoded.viewangles[0], short_to_angle(angle_to_short(45.0) as i16));
        assert_eq!(decoded.gunindex, 3);
        assert_eq!(reader.short().unwrap(), 0);
        reader.finish().unwrap();
    }

    #[test]
    fn r1q2_usercmd_byte_exact() {
        for (version, name) in [
            (1903, "ff800200022c01900121000a00030832c8"),
            (1905, "ff770a022c01502100020832c8"),
        ] {
            let codec = R1q2Codec::new(version);
            let mut out = writer();
            codec
                .write_delta_usercmd(&mut out, &Usercmd::default(), &r1_cmd())
                .unwrap();
            assert_eq!(out.bytes(), decode_hex(name).as_slice(), "version {version}");
            let mut reader = MsgReader::new(out.bytes());
            let decoded = codec.read_delta_usercmd(&mut reader, &Usercmd::default()).unwrap();
            reader.finish().unwrap();
            assert_eq!(decoded, r1_cmd(), "version {version}");
        }
    }

    #[test]
    fn q2pro_int23_vectors() {
        for (current, previous, name) in [
            (100000, 0, "410d03"),
            (100, 90, "1400"),
            (-4194304, 0, "010080"),
            (4194303, 0, "ffff7f"),
        ] {
            let mut out = writer();
            write_q2pro_int23(&mut out, current, previous).unwrap();
            assert_eq!(out.bytes(), decode_hex(name).as_slice());
            let mut reader = MsgReader::new(out.bytes());
            assert_eq!(read_q2pro_int23(&mut reader, previous).unwrap(), current);
        }
        let mut out = writer();
        assert_eq!(
            write_q2pro_int23(&mut out, 4194304, 0),
            Err(VariantError::CoordinateRange(4194304))
        );
        assert_eq!(
            write_q2pro_int23(&mut out, -4194305, 0),
            Err(VariantError::CoordinateRange(-4194305))
        );
    }

    #[test]
    fn q2pro_var64_vectors() {
        for (value, name) in [(300u64, "ac02"), (0u64, "00"), (u64::MAX, "ffffffffffffffffff01")] {
            let mut out = writer();
            write_q2pro_var64(&mut out, value).unwrap();
            assert_eq!(out.bytes(), decode_hex(name).as_slice());
            let mut reader = MsgReader::new(out.bytes());
            assert_eq!(read_q2pro_var64(&mut reader).unwrap(), value);
        }
        // Nine continuation bytes run out of input before the tenth limb.
        let mut reader = MsgReader::new(&[0x80u8; 9]);
        assert!(matches!(
            read_q2pro_var64(&mut reader),
            Err(VariantError::Msg(MsgError::Truncated(_)))
        ));
        let mut reader = MsgReader::new(&[0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x02]);
        assert_eq!(read_q2pro_var64(&mut reader), Err(VariantError::BadVar64));
    }

    #[test]
    fn q2pro_fog_byte_exact() {
        assert_eq!(q2pro_fog_bits(&Q2ProFog::default(), &q2pro_fog()), 0xff);
        let mut out = writer();
        write_q2pro_fog(&mut out, 0xff, &q2pro_fog()).unwrap();
        assert_eq!(
            out.bytes(),
            decode_hex("ff0a141e6400c80005000600010203040506400680f3").as_slice()
        );
        let mut reader = MsgReader::new(out.bytes());
        assert_eq!(read_q2pro_fog(&mut reader, &Q2ProFog::default()).unwrap(), q2pro_fog());
    }

    #[test]
    fn q2pro_serverdata_byte_exact() {
        let mut codec = q2pro_codec();
        let params = Q2ProServerData {
            servercount: 9,
            attractloop: true,
            gamedir: "baseq2".to_string(),
            clientnum: 1,
            levelname: "q2dm1".to_string(),
            version: 1026,
            server_state: 2,
            wire_flags: 24,
        };
        let mut out = writer();
        codec.write_server_data(&mut out, &params).unwrap();
        assert_eq!(
            out.bytes(),
            decode_hex("0c2400000009000000016261736571320001007132646d31000204021800").as_slice()
        );
        assert!(!params.strafejump_hack());
        assert!(!params.qw_mode());
        assert!(!params.waterjump_hack());
        let mut reader = MsgReader::new(out.bytes());
        assert_eq!(reader.byte().unwrap(), protocol::Svc::Serverdata as u8);
        assert_eq!(reader.long().unwrap(), 36);
        let decoded = codec.read_server_data(&mut reader, 1019).unwrap();
        reader.finish().unwrap();
        assert_eq!(decoded, params);
        assert_eq!(
            codec.features(),
            Q2ProFeatures {
                revision: 1026,
                flags: 24
            }
        );
    }

    #[test]
    fn q2pro_serverdata_legacy_flags() {
        let mut codec = Q2ProCodec::new(Q2ProFeatures {
            revision: 1015,
            flags: 0,
        });
        let params = Q2ProServerData {
            servercount: 1,
            attractloop: false,
            gamedir: "baseq2".to_string(),
            clientnum: 0,
            levelname: "base1".to_string(),
            version: 1015,
            server_state: 0,
            wire_flags: 3,
        };
        let mut out = writer();
        codec.write_server_data(&mut out, &params).unwrap();
        assert_eq!(&out.bytes()[out.cursize() - 3..], &[1, 1, 0]);
        let mut reader = MsgReader::new(out.bytes());
        reader.byte().unwrap();
        reader.long().unwrap();
        let decoded = codec.read_server_data(&mut reader, 1019).unwrap();
        assert_eq!(decoded.wire_flags, 3);
        assert!(decoded.strafejump_hack());
        assert!(decoded.qw_mode());
        assert!(!decoded.waterjump_hack());
    }

    #[test]
    fn q2pro_entity_byte_exact() {
        let codec = q2pro_codec();
        let mut out = writer();
        assert!(codec
            .write_delta_entity(&mut out, &EntityState::default(), &q2pro_entity(), false, true)
            .unwrap());
        assert_eq!(out.bytes(), decode_hex("afeb8fff032c012c012c01701101000000120056344806dcfcc200004000201000200030002cc17f800744332211efcdab097f20").as_slice());
        let mut reader = MsgReader::new(out.bytes());
        let (number, bits) = read_q2pro_entity_bits(&mut reader).unwrap();
        assert_eq!(number, 300);
        let decoded = codec
            .read_delta_entity(&mut reader, &EntityState::default(), number, bits)
            .unwrap();
        reader.finish().unwrap();
        assert_eq!(decoded.origin, [100.5, -50.25, 12.125]);
        assert_close(decoded.angles[1], short_to_angle(angle_to_short(90.0) as i16));
        assert_close(decoded.angles[2], short_to_angle(angle_to_short(45.0) as i16));
        assert_eq!(decoded.old_origin, [1.0, 2.0, 3.0]);
        assert_eq!(decoded.modelindex, 300);
        assert_eq!(decoded.frame, 300);
        assert_eq!(decoded.skinnum, 70000);
        assert_eq!(decoded.effects, 0x120000);
        assert_eq!(decoded.renderfx, 0x3456);
        assert_eq!(decoded.morefx, 0x9abcdef);
        assert_close(decoded.alpha, 127.0 / 255.0);
        assert_eq!(decoded.scale, 2.0);
        assert_eq!(decoded.solid, 0x11223344);
        assert_eq!(decoded.sound, 300);
        assert_close(decoded.loop_volume, 127.0 / 255.0);
        assert_eq!(decoded.loop_attenuation, 2.0);
        assert_eq!(decoded.event, 7);
    }

    #[test]
    fn q2pro_entity_byte_angles_without_extensions() {
        let codec = Q2ProCodec::new(Q2ProFeatures {
            revision: 1015,
            flags: 0,
        });
        let to = EntityState {
            number: 9,
            angles: [0.0, 90.0, 0.0],
            renderfx: RF_BEAM,
            old_origin: [4.0, 5.0, 6.0],
            ..Default::default()
        };
        let mut out = writer();
        codec
            .write_delta_entity(&mut out, &EntityState::default(), &to, false, false)
            .unwrap();
        let mut reader = MsgReader::new(out.bytes());
        let (number, bits) = read_q2pro_entity_bits(&mut reader).unwrap();
        assert_eq!(number, 9);
        // Revision 1015 omits the 16-bit angle flag, and old origins always
        // ride along below revision 1017.
        assert_eq!(bits & Q2P_ANGLE16, 0);
        assert_ne!(bits & Q2P_OLDORIGIN, 0);
        let decoded = codec
            .read_delta_entity(&mut reader, &EntityState::default(), number, bits)
            .unwrap();
        assert_eq!(decoded.angles[1], 90.0);
        assert_eq!(decoded.old_origin, [4.0, 5.0, 6.0]);
    }

    #[test]
    fn q2pro_entity_errors() {
        let codec = q2pro_codec();
        let mut out = writer();
        let mut bad = EntityState::default();
        assert_eq!(
            codec.write_delta_entity(&mut out, &EntityState::default(), &bad, true, false),
            Err(VariantError::EntityRange(0))
        );
        bad.number = 8192;
        assert_eq!(
            codec.write_delta_entity(&mut out, &EntityState::default(), &bad, true, false),
            Err(VariantError::EntityRange(8192))
        );
        let plain = Q2ProCodec::new(Q2ProFeatures {
            revision: 1015,
            flags: 0,
        });
        let wide = EntityState {
            number: 5,
            modelindex: 300,
            ..Default::default()
        };
        assert_eq!(
            plain.write_delta_entity(&mut out, &EntityState::default(), &wide, true, false),
            Err(VariantError::ExtensionsRequired)
        );
        let mut foggy = PlayerState::default();
        foggy.fog.color = [1, 2, 3];
        assert_eq!(
            plain.encode_player_state(&PlayerState::default(), &foggy),
            Err(VariantError::FogRevision(1015))
        );
    }

    #[test]
    fn q2pro_playerstate_byte_exact() {
        let codec = q2pro_codec();
        let mut out = writer();
        codec
            .write_player_state_delta(&mut out, &PlayerState::default(), &q2pro_player())
            .unwrap();
        assert_eq!(out.bytes(), decode_hex("11ffff017f024006e0fcf0001400d8ff3c002c01900120030100020003000102031c07380e551504080c2c610900000104080c1f19334c667fff0a141e6400c80005000600010203040506400680f35a018180808080206400fbff2c01").as_slice());
        let mut reader = MsgReader::new(out.bytes());
        assert_eq!(reader.byte().unwrap(), protocol::Svc::Playerinfo as u8);
        let decoded = codec
            .read_player_state_delta(&mut reader, &PlayerState::default())
            .unwrap();
        reader.finish().unwrap();
        assert_eq!(decoded.pmove.pm_type, 2);
        assert_eq!(decoded.pmove.origin, [800, -400, 120]);
        assert_eq!(decoded.pmove.velocity, [10, -20, 30]);
        assert_eq!(decoded.pmove.pm_time, 300);
        assert_eq!(decoded.pmove.pm_flags, 400);
        assert_eq!(decoded.gunindex, 300);
        assert_eq!(decoded.gunskin, 3);
        assert_eq!(decoded.gunframe, 9);
        assert_eq!(decoded.clientnum, 300);
        assert_eq!(decoded.stats[0], 100);
        assert_eq!(decoded.stats[40], -5);
        assert_eq!(decoded.fog, q2pro_fog());
        assert_close(decoded.damage_blend[0], 127.0 / 255.0);
    }

    #[test]
    fn q2pro_frame_byte_exact() {
        let codec = q2pro_codec();
        let ps = PlayerState {
            viewangles: [30.0, 60.0, 90.0],
            gunindex: 7,
            ..Default::default()
        };
        let mut out = writer();
        codec
            .write_frame(
                &mut out,
                &FrameWrite {
                    framenum: 200,
                    lastframe: 198,
                    surpress_count: 1,
                    areabits: &[0x7f],
                    ps_from: None,
                    ps_to: &ps,
                },
                |w| w.write_short(0).map_err(VariantError::from),
            )
            .unwrap();
        assert_eq!(
            out.bytes(),
            decode_hex("34c800001001017f00115515aa2a004007000000").as_slice()
        );
        let mut reader = MsgReader::new(out.bytes());
        let opcode = reader.byte().unwrap();
        assert_eq!(opcode, 0x34);
        let mut codec = q2pro_codec();
        codec.note_frame_opcode_extrabits(opcode & !31);
        let mut areas = Vec::new();
        let header = codec.read_frame_header(&mut reader, &mut areas).unwrap();
        assert_eq!(
            header,
            FrameHeader {
                serverframe: 200,
                deltaframe: 198,
                surpress_count: 1,
                areabytes: 1,
            }
        );
        assert_eq!(areas, vec![0x7f]);
        let decoded = codec
            .read_frame_playerstate(&mut reader, &PlayerState::default())
            .unwrap();
        assert_close(decoded.viewangles[2], short_to_angle(angle_to_short(90.0) as i16));
        assert_eq!(decoded.gunindex, 7);
        assert_eq!(reader.short().unwrap(), 0);
        reader.finish().unwrap();
    }

    #[test]
    fn q2pro_batch_byte_exact() {
        let cmd0 = Usercmd {
            angles: [640, 0, 0],
            forwardmove: 100,
            msec: 50,
            ..Usercmd::default()
        };
        let cmd2 = Usercmd {
            angles: [650, 0, 0],
            forwardmove: 100,
            sidemove: -50,
            buttons: 3,
            msec: 60,
            ..Usercmd::default()
        };
        let frames = vec![
            BatchMoveFrame {
                cmds: vec![cmd0.clone(), cmd0.clone()],
            },
            BatchMoveFrame {
                cmds: vec![cmd2.clone()],
            },
        ];
        let mut out = writer();
        Q2ProCodec::write_batch_move(&mut out, Some(50), &frames).unwrap();
        assert_eq!(out.bytes(), decode_hex("320000000062224001326484d1159c1f0f").as_slice());
        let mut reader = MsgReader::new(out.bytes());
        let parsed = Q2ProCodec::read_batch_move(&mut reader, false, 1).unwrap();
        reader.finish().unwrap();
        assert_eq!(parsed.lastframe, 50);
        assert_eq!(parsed.num_dups, 1);
        assert_eq!(parsed.frames.len(), 2);
        assert_eq!(parsed.frames[0].cmds, vec![cmd0.clone(), cmd0]);
        assert_eq!(parsed.frames[1].cmds, vec![cmd2]);
        let mut reader = MsgReader::new(&[]);
        assert_eq!(
            Q2ProCodec::read_batch_move(&mut reader, true, 3),
            Err(VariantError::BatchNumDups(3))
        );
    }

    fn rerelease_entity() -> EntityState {
        EntityState {
            number: 300,
            origin: [100.5, -50.25, 12.125],
            angles: [0.0, 90.0, 45.0],
            old_origin: [1.0, 2.0, 3.0],
            modelindex: 300,
            modelindex2: 5,
            frame: 300,
            skinnum: 70000,
            effects: 0x120000,
            morefx: 0x3456,
            renderfx: 41,
            solid: 0x11223344,
            sound: 300,
            loop_volume: 0.5,
            loop_attenuation: 2.0,
            event: 7,
            alpha: 0.5,
            scale: 2.0,
            ..EntityState::default()
        }
    }

    fn rerelease_player() -> PlayerState {
        let mut ps = PlayerState::default();
        ps.pmove.pm_type = 2;
        ps.pmove.origin_f = [100.5, -50.25, 12.125];
        ps.pmove.velocity_f = [10.5, -20.5, 30.5];
        ps.pmove.pm_time = 300;
        ps.pmove.pm_flags = 400;
        ps.pmove.gravity = 800;
        ps.pmove.delta_angles = [1, 2, 3];
        ps.pmove.viewheight = 22;
        ps.viewoffset = [0.25, 0.5, 0.75];
        ps.viewangles = [10.0, 20.0, 30.0];
        ps.kick_angles = [1.0, 2.0, 3.0];
        ps.gunindex = 300;
        ps.gunskin = 3;
        ps.gunframe = 500;
        ps.gunoffset = [0.1, 0.2, 0.3];
        ps.gunangles = [1.0, 2.0, 3.0];
        ps.blend = [0.1, 0.2, 0.3, 0.4];
        ps.damage_blend = [0.0, 0.0, 0.6, 0.0];
        ps.fov = 90;
        ps.rdflags = 1;
        ps.gunrate = 7;
        ps.stats[0] = 100;
        ps.stats[40] = -5;
        ps
    }

    #[test]
    fn rerelease_serverdata_byte_exact() {
        let codec = RereleaseCodec::new(false);
        let params = RereleaseServerData {
            servercount: 5,
            attractloop: false,
            gamedir: "baseq2".to_string(),
            clientnum: 0,
            levelname: "base1".to_string(),
            protocol_revision: 1024,
            server_state: 1,
            wire_flags: 3,
            server_fps: 20,
        };
        let mut out = writer();
        codec.write_server_data(&mut out, &params).unwrap();
        assert_eq!(
            out.bytes(),
            decode_hex("0c0e0400000500000000626173657132000000626173653100000401030014").as_slice()
        );
        let mut reader = MsgReader::new(out.bytes());
        assert_eq!(reader.byte().unwrap(), protocol::Svc::Serverdata as u8);
        assert_eq!(reader.long().unwrap(), 1038);
        assert_eq!(RereleaseCodec::read_server_data(&mut reader).unwrap(), params);
        reader.finish().unwrap();
    }

    #[test]
    fn rerelease_entity_byte_exact() {
        let mut out = writer();
        assert!(RereleaseCodec::write_delta_entity(
            &mut out,
            &EntityState::default(),
            &rerelease_entity(),
            false,
            true
        )
        .unwrap());
        assert_eq!(out.bytes(), decode_hex("affb9bdf032c012c0105002c017011010000001200290000c942000049c200004241004000200000803f00000040000040402cc17f80074433221156347f20").as_slice());
        let mut reader = MsgReader::new(out.bytes());
        let header = read_entity_bits_wide(&mut reader).unwrap();
        assert_eq!(header.number, 300);
        let decoded =
            RereleaseCodec::read_delta_entity(&mut reader, &EntityState::default(), header.number, header).unwrap();
        reader.finish().unwrap();
        assert_eq!(decoded.origin, [100.5, -50.25, 12.125]);
        for i in 0..3 {
            assert_close(
                decoded.angles[i],
                short_to_angle(angle_to_short([0.0, 90.0, 45.0][i]) as i16),
            );
        }
        assert_eq!(decoded.modelindex, 300);
        assert_eq!(decoded.modelindex2, 5);
        assert_eq!(decoded.frame, 300);
        assert_eq!(decoded.skinnum, 70000);
        assert_eq!(decoded.effects, 0x120000);
        assert_eq!(decoded.morefx, 0x3456);
        assert_eq!(decoded.solid, 0x11223344);
        assert_eq!(decoded.sound, 300);
        assert_close(decoded.loop_volume, 127.0 / 255.0);
        assert_eq!(decoded.loop_attenuation, 2.0);
        assert_eq!(decoded.event, 7);
        assert_close(decoded.alpha, 127.0 / 255.0);
        assert_eq!(decoded.scale, 2.0);
    }

    #[test]
    fn rerelease_playerstate_byte_exact() {
        let mut out = writer();
        RereleaseCodec::write_player_state_delta(&mut out, &PlayerState::default(), &rerelease_player()).unwrap();
        assert_eq!(out.bytes(), decode_hex("11ffffbf020000c942000049c200004241000028410000a4c10000f4412c0190012003010002000300040008000c001c07380e551500040008000c2c61f4013300660099000010002000304f19334c66995a0101000000000100006400fbff0716").as_slice());
        let mut reader = MsgReader::new(out.bytes());
        assert_eq!(reader.byte().unwrap(), protocol::Svc::Playerinfo as u8);
        let decoded = RereleaseCodec::read_player_state_delta(&mut reader, &PlayerState::default()).unwrap();
        reader.finish().unwrap();
        assert_eq!(decoded.pmove.pm_type, 2);
        assert_eq!(decoded.pmove.origin_f, [100.5, -50.25, 12.125]);
        assert_eq!(decoded.pmove.velocity_f, [10.5, -20.5, 30.5]);
        assert_eq!(decoded.pmove.origin, [804, -402, 97]);
        assert_eq!(decoded.pmove.velocity, [84, -164, 244]);
        assert_eq!(decoded.pmove.pm_time, 300);
        assert_eq!(decoded.pmove.pm_flags, 400);
        assert_eq!(decoded.pmove.viewheight, 22);
        assert_eq!(decoded.gunindex, 300);
        assert_eq!(decoded.gunskin, 3);
        assert_eq!(decoded.gunframe, 500);
        assert_eq!(decoded.gunrate, 7);
        assert_eq!(decoded.stats[0], 100);
        assert_eq!(decoded.stats[40], -5);
        assert_close(decoded.damage_blend[2], 153.0 / 255.0);
    }

    #[test]
    fn rerelease_frame_byte_exact() {
        let ps = PlayerState {
            fov: 110,
            ..Default::default()
        };
        let mut out = writer();
        RereleaseCodec::write_frame(
            &mut out,
            &FrameWrite {
                framenum: 300,
                lastframe: 299,
                surpress_count: 2,
                areabits: &[1, 2],
                ps_from: None,
                ps_to: &ps,
            },
            |w| w.write_short(0).map_err(VariantError::from),
        )
        .unwrap();
        assert_eq!(out.bytes(), decode_hex("142c010008010002010200086e0000").as_slice());
        let mut codec = RereleaseCodec::new(false);
        let mut reader = MsgReader::new(out.bytes());
        assert_eq!(reader.byte().unwrap(), protocol::Svc::Frame as u8);
        let mut areas = Vec::new();
        let header = codec.read_frame_header(&mut reader, &mut areas).unwrap();
        assert_eq!(
            header,
            FrameHeader {
                serverframe: 300,
                deltaframe: 299,
                surpress_count: 1,
                areabytes: 2,
            }
        );
        assert_eq!(areas, vec![1, 2]);
        let decoded = codec
            .read_frame_playerstate(&mut reader, &PlayerState::default())
            .unwrap();
        assert_eq!(decoded.fov, 110);
        assert_eq!(reader.short().unwrap(), 0);
        reader.finish().unwrap();
    }

    #[test]
    fn rerelease_batch_byte_exact() {
        let cmd = Usercmd {
            angles: [100, 200, 300],
            forwardmove: 400,
            sidemove: -33,
            buttons: 0x83,
            msec: 50,
            ..Usercmd::default()
        };
        let codec = RereleaseCodec::new(false);
        let mut out = writer();
        codec
            .write_batch_move(
                &mut out,
                Some(60),
                &[BatchMoveFrame {
                    cmds: vec![cmd.clone()],
                }],
            )
            .unwrap();
        assert_eq!(
            out.bytes(),
            decode_hex("3c0000000000e1373200c8002c01907d3f2803").as_slice()
        );
        let mut reader = MsgReader::new(out.bytes());
        let parsed = codec.read_batch_move(&mut reader, false).unwrap();
        reader.finish().unwrap();
        assert_eq!(parsed.lastframe, 60);
        assert_eq!(parsed.num_dups, 0);
        assert_eq!(parsed.frames.len(), 1);
        assert_eq!(parsed.frames[0].cmds, vec![cmd]);
    }

    #[test]
    fn rerelease_batch_classic_byte_exact() {
        let cmd = Usercmd {
            upmove: 100,
            buttons: 5,
            msec: 40,
            lightlevel: 150,
            ..Default::default()
        };
        let codec = RereleaseCodec::new(true);
        let mut out = writer();
        codec
            .write_batch_move(
                &mut out,
                Some(61),
                &[BatchMoveFrame {
                    cmds: vec![cmd.clone()],
                }],
            )
            .unwrap();
        assert_eq!(out.bytes(), decode_hex("3d00000000962138190528").as_slice());
        let mut reader = MsgReader::new(out.bytes());
        let parsed = codec.read_batch_move(&mut reader, false).unwrap();
        reader.finish().unwrap();
        assert_eq!(parsed.lastframe, 61);
        assert_eq!(parsed.frames[0].cmds, vec![cmd]);
    }

    #[test]
    fn rerelease_usercmd_restrictions() {
        let codec = RereleaseCodec::new(false);
        let classic = RereleaseCodec::new(true);
        let cmd = Usercmd {
            upmove: 10,
            ..Default::default()
        };
        let mut out = writer();
        assert_eq!(
            codec.write_delta_usercmd(&mut out, &Usercmd::default(), &cmd),
            Err(VariantError::NonBatchableUpMove)
        );
        classic
            .write_delta_usercmd(&mut out, &Usercmd::default(), &cmd)
            .unwrap();
        // Reserved CM_UP bit on the 1038 read path.
        let bytes = out.bytes().to_vec();
        assert_ne!(bytes[0] & (protocol::CM_UP as u8), 0);
        let mut reader = MsgReader::new(&bytes);
        assert_eq!(
            codec.read_delta_usercmd(&mut reader, &Usercmd::default()),
            Err(VariantError::ReservedUpMove)
        );
        // CM_UP inside a non-classic batch body.
        let mut batch = writer();
        classic
            .write_batch_move(
                &mut batch,
                None,
                &[BatchMoveFrame {
                    cmds: vec![cmd.clone()],
                }],
            )
            .unwrap();
        let mut reader = MsgReader::new(batch.bytes());
        assert_eq!(codec.read_batch_move(&mut reader, true), Err(VariantError::BatchCmUp));
    }

    #[test]
    fn fog_read_matches_donor() {
        let bytes = [
            0xff, 0xff, 0x00, 0x00, 0x00, 0x3f, 200, 10, 20, 30, 0x90, 0x01, 0x00, 0x00, 0xc0, 0x3f, 0x00, 0x00, 0x20,
            0x40, 1, 2, 3, 0xe8, 0x03, 0x00, 0x00, 4, 5, 6, 0xd0, 0x07, 0x00, 0x00,
        ];
        let mut reader = MsgReader::new(&bytes);
        let fog = read_fog(&mut reader).unwrap();
        reader.finish().unwrap();
        assert_eq!(fog.bits, 0xffff);
        assert_eq!(fog.density, 0.5);
        assert_eq!(fog.skyfactor, 200);
        assert_eq!((fog.red, fog.green, fog.blue), (10, 20, 30));
        assert_eq!(fog.time, 400);
        assert_eq!(fog.hf_falloff, 1.5);
        assert_eq!(fog.hf_density, 2.5);
        assert_eq!(fog.hf_start, [1, 2, 3]);
        assert_eq!(fog.hf_start_dist, 1000);
        assert_eq!(fog.hf_end, [4, 5, 6]);
        assert_eq!(fog.hf_end_dist, 2000);
    }

    fn kex_entity() -> EntityState {
        EntityState {
            number: 300,
            origin: [100.5, -50.25, 12.125],
            angles: [0.0, 90.0, 45.0],
            old_origin: [1.0, 2.0, 3.0],
            modelindex: 300,
            frame: 300,
            skinnum: 70000,
            effects: 0x11111111,
            morefx: 0x2222,
            renderfx: 0x3456,
            solid: 0x11223344,
            sound: 300,
            loop_volume: 0.5,
            loop_attenuation: 2.0,
            event: 7,
            alpha: 0.5,
            scale: 2.0,
            instance_bits: 5,
            owner: 600,
            old_frame: 44,
            ..EntityState::default()
        }
    }

    fn kex_player() -> PlayerState {
        let mut ps = rerelease_player();
        ps.pmove.velocity_f = [1.0, 2.0, 3.0];
        ps.pmove.delta_angles = [1000, 2000, 3000];
        ps.gunframe = 100;
        ps.gunoffset = [0.1, 0.0, 0.0];
        ps.gunangles = [0.0, 2.0, 0.0];
        ps.damage_blend = [0.5, 0.0, 0.0, 0.0];
        ps.team_id = 3;
        ps
    }

    #[test]
    fn kex_usercmd_byte_exact() {
        let cmd = Usercmd {
            angles: [1000, 2000, 3000],
            forwardmove: 400,
            sidemove: -200,
            buttons: 7,
            server_frame: 1234,
            msec: 50,
            ..Usercmd::default()
        };
        let mut out = writer();
        write_kex_usercmd(&mut out, &Usercmd::default(), &cmd).unwrap();
        assert_eq!(
            out.bytes(),
            decode_hex("df00c8af4000c82f4100d683410000c843000048c307d204000032").as_slice()
        );
        let mut reader = MsgReader::new(out.bytes());
        let decoded = read_kex_usercmd(&mut reader, &Usercmd::default()).unwrap();
        reader.finish().unwrap();
        assert_eq!(decoded, cmd);
    }

    #[test]
    fn kex_entity_2023_byte_exact() {
        let mut codec = KexCodec::new(PROTOCOL_KEX);
        let mut out = writer();
        assert!(codec
            .write_delta_entity(&mut out, &EntityState::default(), &kex_entity(), false, true)
            .unwrap());
        assert_eq!(out.bytes(), decode_hex("af8b8fffff2c012c012c01701101001111111122225634443322110000c942000049c2000042410000803f00000040000040400000b442000034422cc17f80077f200558022c00").as_slice());
        let mut reader = MsgReader::new(out.bytes());
        let header = read_entity_bits_wide(&mut reader).unwrap();
        assert_eq!(header.number, 300);
        assert_eq!(header.hi, 0xff);
        let decoded = codec
            .read_delta_entity(&mut reader, &EntityState::default(), header.number, header)
            .unwrap();
        reader.finish().unwrap();
        assert_eq!(decoded.origin, [100.5, -50.25, 12.125]);
        assert_eq!(decoded.angles, [0.0, 90.0, 45.0]);
        assert_eq!(decoded.old_origin, [1.0, 2.0, 3.0]);
        assert_eq!(decoded.modelindex, 300);
        assert_eq!(decoded.skinnum, 70000);
        assert_eq!(decoded.effects, 0x11111111);
        assert_eq!(decoded.morefx, 0x2222);
        assert_eq!(decoded.renderfx, 0x3456);
        assert_eq!(decoded.solid, 0x11223344);
        assert_eq!(decoded.sound, 300);
        assert_close(decoded.loop_volume, 127.0 / 255.0);
        assert_eq!(decoded.loop_attenuation, 2.0);
        assert_eq!(decoded.event, 7);
        assert_close(decoded.alpha, 127.0 / 255.0);
        assert_eq!(decoded.scale, 2.0);
        assert_eq!(decoded.instance_bits, 5);
        assert_eq!(decoded.owner, 600);
        assert_eq!(decoded.old_frame, 44);
    }

    #[test]
    fn kex_entity_2022_byte_exact() {
        let mut codec = KexCodec::new(PROTOCOL_KEX_DEMOS);
        assert!(codec.is_demo_protocol());
        let to = EntityState {
            number: 7,
            origin: [100.5, -50.25, 12.125],
            angles: [10.0, 20.0, 30.0],
            old_origin: [1.0, 2.0, 3.0],
            ..EntityState::default()
        };
        let mut out = writer();
        codec
            .write_delta_entity(&mut out, &EntityState::default(), &to, false, true)
            .unwrap();
        assert_eq!(
            out.bytes(),
            decode_hex("8f8680010724036efe6100080010001800000020410000a0410000f041").as_slice()
        );
        let mut reader = MsgReader::new(out.bytes());
        let header = read_entity_bits_wide(&mut reader).unwrap();
        let decoded = codec
            .read_delta_entity(&mut reader, &EntityState::default(), header.number, header)
            .unwrap();
        reader.finish().unwrap();
        assert_eq!(decoded.origin, [100.5, -50.25, 12.125]);
        assert_eq!(decoded.old_origin, [1.0, 2.0, 3.0]);
        assert_eq!(decoded.angles, [10.0, 20.0, 30.0]);
    }

    #[test]
    fn kex_playerstate_byte_exact() {
        let mut out = writer();
        KexCodec::write_player_state_delta(&mut out, &PlayerState::default(), &kex_player()).unwrap();
        assert_eq!(out.bytes(), decode_hex("ffff0300020000c942000049c2000042410000803f00000040000040402c019001200300c8af4000c82f4100d68341040008000c0016000020410000a0410000f04100040008000c2c6164a2cdcccc3d000000400719334c665a0101000000640000010000fbff7f00000003").as_slice());
        let mut reader = MsgReader::new(out.bytes());
        let decoded = KexCodec::read_player_state_delta(&mut reader, &PlayerState::default()).unwrap();
        reader.finish().unwrap();
        assert_eq!(decoded.pmove.pm_type, 2);
        assert_eq!(decoded.pmove.origin_f, [100.5, -50.25, 12.125]);
        assert!(decoded.pmove.delta_angle_float);
        assert_eq!(decoded.pmove.delta_angles, [1000, 2000, 3000]);
        assert_eq!(decoded.pmove.viewheight, 22);
        assert_eq!(decoded.viewangles, [10.0, 20.0, 30.0]);
        assert_eq!(decoded.gunindex, 300);
        assert_eq!(decoded.gunskin, 3);
        assert_eq!(decoded.gunframe, 100);
        assert_close(decoded.gunoffset[0], f64::from(0.1f32));
        assert_eq!(decoded.gunrate, 7);
        assert_eq!(decoded.stats[0], 100);
        assert_eq!(decoded.stats[40], -5);
        assert_eq!(decoded.team_id, 3);
        assert_close(decoded.damage_blend[0], 127.0 / 255.0);
    }

    #[test]
    fn kex_serverdata_byte_exact() {
        let mut codec = KexCodec::new(PROTOCOL_KEX);
        let params = KexServerData {
            servercount: 11,
            attractloop: false,
            server_fps: 40,
            gamedir: "baseq2".to_string(),
            clientnums: vec![2, 3],
            levelname: "base1".to_string(),
        };
        let mut out = writer();
        codec.write_server_data(&mut out, &params).unwrap();
        assert_eq!(
            out.bytes(),
            decode_hex("0ce70700000b000000002862617365713200feff020002000300626173653100").as_slice()
        );
        let mut reader = MsgReader::new(out.bytes());
        assert_eq!(reader.byte().unwrap(), protocol::Svc::Serverdata as u8);
        assert_eq!(reader.long().unwrap(), 2023);
        let decoded = codec.read_server_data(&mut reader).unwrap();
        reader.finish().unwrap();
        assert_eq!(decoded, params);
        assert_eq!(decoded.clientnum(), 2);
        assert_eq!(codec.split_player_count(), 2);
    }

    #[test]
    fn kex_frame_byte_exact() {
        let ps = PlayerState {
            fov: 100,
            ..Default::default()
        };
        let mut out = writer();
        KexCodec::write_frame(
            &mut out,
            &FrameWrite {
                framenum: 400,
                lastframe: 399,
                surpress_count: 0,
                areabits: &[0xff],
                ps_from: None,
                ps_to: &ps,
            },
            |w| w.write_short(0).map_err(VariantError::from),
        )
        .unwrap();
        assert_eq!(
            out.bytes(),
            decode_hex("14900100008f0100000001ff1100086400000000000000000000").as_slice()
        );
        let mut reader = MsgReader::new(out.bytes());
        assert_eq!(reader.byte().unwrap(), protocol::Svc::Frame as u8);
        let mut areas = Vec::new();
        let header = KexCodec::read_frame_header(&mut reader, &mut areas).unwrap();
        assert_eq!(
            header,
            FrameHeader {
                serverframe: 400,
                deltaframe: 399,
                surpress_count: 0,
                areabytes: 1,
            }
        );
        assert_eq!(areas, vec![0xff]);
        let decoded = KexCodec::read_frame_playerstate(&mut reader, &PlayerState::default()).unwrap();
        assert_eq!(decoded.fov, 100);
        KexCodec::read_packet_entities_begin(&mut MsgReader::new(&[18])).unwrap();
        assert_eq!(reader.short().unwrap(), 0);
        reader.finish().unwrap();
    }

    #[test]
    fn kex_message_readers_match_donor() {
        let codec = KexCodec::new(PROTOCOL_KEX);
        let bytes = [
            0x05, b'b', b'a', b's', b'e', b'_', b's', b't', b'r', b'i', b'n', b'g', 0, 2, b'a', b'r', b'g', b'1', 0,
            b'a', b'r', b'g', b'2', 0,
        ];
        let mut reader = MsgReader::new(&bytes);
        let loc = KexCodec::read_locprint(&mut reader).unwrap();
        assert_eq!(loc.flags, 5);
        assert_eq!(loc.base, "base_string");
        assert_eq!(loc.args, vec!["arg1".to_string(), "arg2".to_string()]);
        let mut reader = MsgReader::new(&[0, 0, 9]);
        assert_eq!(
            KexCodec::read_locprint(&mut reader),
            Err(VariantError::TooManyLocArgs(9))
        );

        let sound = [
            95, 0x34, 0x12, 200, 100, 50, 0xc5, 0x12, 0x00, 0x00, 0x00, 0x00, 0xc0, 0x3f, 0x00, 0x00, 0x20, 0x40, 0x00,
            0x00, 0x60, 0x40,
        ];
        let mut reader = MsgReader::new(&sound);
        let parsed = codec.read_sound(&mut reader).unwrap();
        reader.finish().unwrap();
        assert_eq!(parsed.flags, 95);
        assert_eq!(parsed.index, 4660);
        assert_close(parsed.volume, 200.0 / 255.0);
        assert_eq!(parsed.attenuation, 1.5625);
        assert_close(parsed.timeofs, 0.05);
        assert_eq!(parsed.entity, 600);
        assert_eq!(parsed.channel, 5);
        assert_eq!(parsed.pos, Some([1.5, 2.5, 3.5]));

        let mut reader = MsgReader::new(&[2, 0xa5, 10, 0x43, 100]);
        let damage = KexCodec::read_damage(&mut reader).unwrap();
        reader.finish().unwrap();
        assert_eq!(damage.len(), 2);
        assert_eq!(damage[0].damage, 5);
        assert!(damage[0].health && !damage[0].armor && damage[0].shield);
        assert_eq!(
            damage[0].direction,
            crate::q2::read_dir(&mut MsgReader::new(&[10])).unwrap()
        );
        assert_eq!(damage[1].damage, 3);
        assert!(!damage[1].health && damage[1].armor && !damage[1].shield);

        let poi = [
            0x34, 0x12, 0x50, 0x00, 0x00, 0x00, 0xc0, 0x3f, 0x00, 0x00, 0x20, 0x40, 0x00, 0x00, 0x60, 0x40, 0xaa, 0x00,
            7, 3,
        ];
        let mut reader = MsgReader::new(&poi);
        let parsed = KexCodec::read_poi(&mut reader).unwrap();
        assert_eq!(parsed.key, 4660);
        assert_eq!(parsed.time, 80);
        assert_eq!(parsed.pos, [1.5, 2.5, 3.5]);
        assert_eq!(parsed.image, 170);
        assert_eq!((parsed.color, parsed.flags), (7, 3));

        let help = [
            1, 0x00, 0x00, 0x90, 0x40, 0x00, 0x00, 0xb0, 0x40, 0x00, 0x00, 0xd0, 0x40, 20,
        ];
        let mut reader = MsgReader::new(&help);
        let parsed = KexCodec::read_help_path(&mut reader).unwrap();
        assert!(parsed.start);
        assert_eq!(parsed.pos, [4.5, 5.5, 6.5]);
        assert_eq!(parsed.dir, crate::q2::read_dir(&mut MsgReader::new(&[20])).unwrap());

        let mut reader = MsgReader::new(&[0x78, 0x56, 0xbc, 0x9a]);
        let flash = KexCodec::read_muzzleflash3(&mut reader).unwrap();
        assert_eq!(flash.entity, 22136);
        assert_eq!(flash.weapon, 39612);

        let mut reader = MsgReader::new(&[3, b't', b'r', b'o', b'p', b'h', b'y', 0]);
        assert_eq!(KexCodec::read_splitclient(&mut reader).unwrap(), 3);
        assert_eq!(KexCodec::read_achievement(&mut reader), "trophy");
        reader.finish().unwrap();
    }

    #[test]
    fn kex_configblast_matches_donor() {
        let bytes = decode_hex("15000d00789c636028344ac935649063303463000010b4022b");
        let mut reader = MsgReader::new(&bytes);
        let records = KexCodec::read_configblast(&mut reader).unwrap();
        reader.finish().unwrap();
        assert_eq!(
            records,
            vec![
                KexConfigstringRecord {
                    index: 0,
                    value: "q2dm1".to_string(),
                },
                KexConfigstringRecord {
                    index: 30,
                    value: "16".to_string(),
                },
            ]
        );
    }

    #[test]
    fn kex_codec_errors() {
        let mut out = writer();
        assert_eq!(
            KexCodec::write_player_state_delta(
                &mut out,
                &PlayerState::default(),
                &PlayerState {
                    gunframe: 512,
                    ..Default::default()
                },
            ),
            Err(VariantError::GunRange(512))
        );
        let codec = KexCodec::new(PROTOCOL_KEX);
        let mut out = writer();
        let params = KexServerData {
            servercount: 0,
            attractloop: false,
            server_fps: 40,
            gamedir: String::new(),
            clientnums: vec![0; 9],
            levelname: String::new(),
        };
        assert_eq!(
            codec.write_server_data(&mut out, &params),
            Err(VariantError::TooManySplitPlayers(9))
        );
        let mut codec = KexCodec::new(PROTOCOL_KEX);
        let bytes = [7, 0, 0, 0, 0, 40, b'b', 0, 0xfe, 0xff, 0, 0];
        let mut reader = MsgReader::new(&bytes);
        assert_eq!(codec.read_server_data(&mut reader), Err(VariantError::BadSplitCount(0)));
    }

    #[test]
    fn zpacket_node_blob_inflates_exactly() {
        let wrapped = decode_hex("1508008000637372761c480400");
        let payload = decode_hex("0642434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142");
        assert_eq!(wrapped[0], protocol::SVC_ZPACKET);
        let mut reader = MsgReader::new(&wrapped);
        assert_eq!(reader.byte().unwrap(), protocol::SVC_ZPACKET);
        assert_eq!(read_zpacket_payload(&mut reader).unwrap(), payload);
        reader.finish().unwrap();
    }

    #[test]
    fn zpacket_wrap_round_trip_and_rejections() {
        let payload = decode_hex("0642434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142434142");
        let wrapped = try_wrap_zpacket(&payload, 1400).expect("compressible payload");
        assert_eq!(wrapped[0], protocol::SVC_ZPACKET);
        assert_eq!(u16::from_le_bytes([wrapped[3], wrapped[4]]), 128);
        let mut reader = MsgReader::new(&wrapped);
        reader.byte().unwrap();
        assert_eq!(read_zpacket_payload(&mut reader).unwrap(), payload);
        // Tiny payloads never compress.
        assert_eq!(try_wrap_zpacket(&[6u8; 20], 1400), None);
        // Serverdata-leading payloads are left alone.
        let mut leading = vec![protocol::Svc::Serverdata as u8];
        leading.extend_from_slice(&[b'A'; 200]);
        assert_eq!(try_wrap_zpacket(&leading, 1400), None);
        // Length mismatches fail loudly.
        let bad = [8u8, 0, 9, 0, 0x63, 0x37, 0x37, 0x32, 0x76, 0x1c, 0x48, 0x04, 0x00];
        let mut reader = MsgReader::new(&bad);
        assert!(matches!(
            read_zpacket_payload(&mut reader),
            Err(VariantError::ZpacketLength { .. } | VariantError::Zlib(_))
        ));
    }

    fn mvd_player() -> PlayerState {
        let mut ps = PlayerState::default();
        ps.pmove.pm_type = 2;
        ps.pmove.origin = [800, -400, 120];
        ps.viewoffset = [0.25, 0.5, 0.75];
        ps.viewangles = [10.0, 20.0, 30.0];
        ps.kick_angles = [1.0, 2.0, 3.0];
        ps.gunindex = 5;
        ps.gunframe = 7;
        ps.gunoffset = [0.5, 0.5, 0.5];
        ps.gunangles = [4.0, 5.0, 6.0];
        ps.blend = [0.1, 0.2, 0.3, 0.4];
        ps.fov = 90;
        ps.rdflags = 1;
        ps.stats[0] = 100;
        ps
    }

    #[test]
    fn mvd_playerstate_byte_exact() {
        let mut out = writer();
        assert!(write_delta_mvd_playerstate(&mut out, None, Some(&mvd_player()), 7, false).unwrap());
        assert_eq!(
            out.bytes(),
            decode_hex("07ff7f02200370fe78000102031c07380e551504080c050702020210141819334c665a01010000006400")
                .as_slice()
        );
        let mut reader = MsgReader::new(out.bytes());
        let parsed = read_delta_mvd_playerstate(&mut reader, None).unwrap();
        reader.finish().unwrap();
        assert_eq!(parsed.number, 7);
        assert!(!parsed.removed);
        assert_eq!(parsed.ps.pmove.pm_type, 2);
        assert_eq!(parsed.ps.pmove.origin, [800, -400, 120]);
        assert_eq!(parsed.ps.viewoffset, [0.25, 0.5, 0.75]);
        assert_eq!(parsed.ps.gunindex, 5);
        assert_eq!(parsed.ps.gunframe, 7);
        assert_eq!(parsed.ps.fov, 90);
        assert_eq!(parsed.ps.stats[0], 100);
    }

    #[test]
    fn mvd_playerstate_rerelease_byte_exact() {
        let mut to = mvd_player();
        to.gunindex = 300;
        to.gunskin = 3;
        to.gunframe = 500;
        to.gunoffset = [0.1, 0.2, 0.3];
        to.gunangles = [1.0, 2.0, 3.0];
        to.damage_blend = [0.0, 0.0, 0.6, 0.0];
        to.stats[40] = -5;
        let mut out = writer();
        assert!(write_delta_mvd_playerstate_rerelease(&mut out, None, Some(&to), 7, false).unwrap());
        assert_eq!(out.bytes(), decode_hex("07ff7f02200370fe7800040008000c001c07380e551500040008000c2c61f4013300660099000010002000304f19334c66995a0101000000000100006400fbff").as_slice());
        let mut reader = MsgReader::new(out.bytes());
        let parsed = read_delta_mvd_playerstate_rerelease(&mut reader, None).unwrap();
        reader.finish().unwrap();
        assert_eq!(parsed.number, 7);
        assert!(!parsed.removed);
        assert_eq!(parsed.ps.gunindex, 300);
        assert_eq!(parsed.ps.gunskin, 3);
        assert_eq!(parsed.ps.gunframe, 500);
        assert_eq!(parsed.ps.stats[40], -5);
        assert_close(parsed.ps.damage_blend[2], 153.0 / 255.0);
    }

    #[test]
    fn mvd_playerstate_removal_and_empty() {
        let mut out = writer();
        assert!(write_delta_mvd_playerstate(&mut out, None, None, 7, false).unwrap());
        assert_eq!(out.bytes(), &[7, 0, 0x80]);
        let mut reader = MsgReader::new(out.bytes());
        let parsed = read_delta_mvd_playerstate(&mut reader, None).unwrap();
        assert!(parsed.removed);
        let mut out = writer();
        let ps = mvd_player();
        assert!(!write_delta_mvd_playerstate(&mut out, Some(&ps), Some(&ps), 7, false).unwrap());
        assert_eq!(out.cursize(), 0);
        let mut out = writer();
        assert_eq!(
            write_delta_mvd_playerstate(&mut out, None, Some(&ps), 255, false),
            Err(VariantError::BadMvdLimits)
        );
    }

    #[test]
    fn mvd_extended_read_matches_donor() {
        let profile = mvd_profile(2013, 12).unwrap();
        assert!(profile.extended && profile.v2 && profile.fog);
        let bytes = decode_hex("92424006e0fc1c07380e2c614119998180808080206400fbff");
        let mut reader = MsgReader::new(&bytes);
        let parsed = read_mvd_player(&mut reader, None, 9, &profile).unwrap();
        reader.finish().unwrap();
        assert_eq!(parsed.number, 9);
        assert!(!parsed.removed);
        assert_eq!(parsed.ps.pmove.origin, [800, -400, 0]);
        assert_close(parsed.ps.viewangles[0], short_to_angle(angle_to_short(10.0) as i16));
        assert_close(parsed.ps.viewangles[1], short_to_angle(angle_to_short(20.0) as i16));
        assert_eq!(parsed.ps.gunindex, 300);
        assert_eq!(parsed.ps.gunskin, 3);
        assert_close(parsed.ps.blend[0], 25.0 / 255.0);
        assert_close(parsed.ps.damage_blend[2], 153.0 / 255.0);
        assert_eq!(parsed.ps.stats[0], 100);
        assert_eq!(parsed.ps.stats[40], -5);
    }

    #[test]
    fn mvd_profile_vectors() {
        let classic = mvd_profile(2010, 0).unwrap();
        assert_eq!(classic.protocol, MvdProtocol::Classic);
        assert!(!classic.extended && !classic.v2 && !classic.fog);
        assert_eq!(classic.max_config_strings, 2080);
        assert_eq!(classic.max_clients_index, 30);
        assert_eq!(classic.max_entities, 1024);
        let ext = mvd_profile(2013, 12).unwrap();
        assert_eq!(ext.protocol, MvdProtocol::Q2Pro { revision: 1026 });
        assert!(ext.extended && ext.v2 && ext.fog);
        assert_eq!(ext.max_config_strings, 13630);
        assert_eq!(mvd_profile(2008, 0), Err(VariantError::UnsupportedMvdRevision(2008)));
        assert_eq!(mvd_profile(2012, 8), Err(VariantError::MvdV2NeedsExtended));
        let rr = mvd_profile(3038, 0).unwrap();
        assert_eq!(rr.protocol, MvdProtocol::Rerelease);
        assert!(rr.rerelease && rr.extended && !rr.v2);
    }

    #[test]
    fn mvd_headers_match_donor() {
        let classic: Vec<u8> = [
            vec![4, 37, 0, 0, 0, 0xda, 0x07, 7, 0, 0, 0],
            b"baseq2\0".to_vec(),
            vec![0, 0, 0, 0],
            b"q2dm1\0".to_vec(),
            vec![30, 0],
            b"8\0".to_vec(),
            vec![0x20, 0x08],
        ]
        .concat();
        let mut reader = MsgReader::new(&classic);
        let header = read_mvd_header(&mut reader).unwrap();
        assert_eq!(header.profile.revision, 2010);
        assert_eq!(header.profile.flags, 0);
        assert_eq!(header.profile.protocol, MvdProtocol::Classic);
        assert_eq!(header.servercount, 7);
        assert_eq!(header.gamedir, "baseq2");
        assert_eq!(header.levelname, "q2dm1");
        assert_eq!(header.config_strings.len(), 2);
        assert_eq!(header.config_strings.get(&30).unwrap(), "8");
        assert_eq!(header.frame_offset, 34);
        assert_eq!(header.dummy, 0);
        assert_eq!(header.max_clients, 8);
        assert_eq!(header.q2pro_version, None);

        let ext: Vec<u8> = [
            vec![4, 37, 0, 0, 0, 0xdd, 0x07, 12, 0, 9, 0, 0, 0],
            b"baseq2\0".to_vec(),
            vec![1, 0, 0, 0],
            b"base1\0".to_vec(),
            vec![60, 0],
            b"16\0".to_vec(),
            vec![0x3e, 0x35],
        ]
        .concat();
        let mut reader = MsgReader::new(&ext);
        let header = read_mvd_header(&mut reader).unwrap();
        assert_eq!(header.profile.revision, 2013);
        assert_eq!(header.profile.flags, 12);
        assert_eq!(header.profile.protocol, MvdProtocol::Q2Pro { revision: 1026 });
        assert_eq!(header.servercount, 9);
        assert_eq!(header.levelname, "base1");
        assert_eq!(header.frame_offset, 37);
        assert_eq!(header.dummy, 1);
        assert_eq!(header.max_clients, 16);
        assert_eq!(header.q2pro_version, Some(1026));
        assert_eq!(header.wire_flags, Some(24));

        let mut reader = MsgReader::new(&[0, 37, 0, 0, 0]);
        assert_eq!(read_mvd_header(&mut reader), Err(VariantError::BadMvdHeader));
    }

    #[test]
    fn misc_codecs_round_trip() {
        let mut out = writer();
        write_mvd_cmd(&mut out, MvdOp::Frame as u8, 3).unwrap();
        write_mvd_players_end(&mut out).unwrap();
        out.write_string("skin").unwrap();
        out.write_string("male/grunt").unwrap();
        out.write_short(11).unwrap();
        out.write_short(22).unwrap();
        let mut reader = MsgReader::new(out.bytes());
        let cmd = read_mvd_cmd(&mut reader).unwrap();
        assert_eq!(cmd.op, MvdOp::Frame as u8);
        assert_eq!(cmd.extrabits, 3);
        assert_eq!(MvdOp::from_u8(cmd.op), Some(MvdOp::Frame));
        assert_eq!(MvdOp::from_u8(19), None);
        assert_eq!(reader.byte().unwrap(), CLIENTNUM_NONE);
        assert!(!valid_mvd_client_number(255));
        assert!(valid_mvd_client_number(0));
        let (name, value) = read_userinfo_delta(&mut reader).unwrap();
        assert_eq!((name.as_str(), value.as_str()), ("skin", "male/grunt"));
        assert_eq!(read_client_setting(&mut reader).unwrap(), (11, 22));
        reader.finish().unwrap();
    }
}
