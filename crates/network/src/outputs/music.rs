//! Native CD-track widths. Playback itself is a common music-ring request.
use super::{Error, Protocol, configstring};
use crate::message::{Encoding, Reader, Writer};
use qa_core::text::FixedText;
use std::fmt::Write;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CdTrack {
    pub track: u8,
    /// NetQuake carries this byte even though its client loops `track`.
    pub loop_track: Option<u8>,
}

pub fn read_cd_track(protocol: Protocol, bytes: &[u8]) -> Result<(CdTrack, usize), Error> {
    if !matches!(protocol, Protocol::NetQuake15 | Protocol::QuakeWorld28) {
        return Err(Error::Unsupported);
    }
    let mut reader = Reader::new(bytes, Encoding::Bytes);
    if reader.read_bits(8)? != 32 {
        return Err(Error::Opcode);
    }
    let track = reader.read_bits(8)? as u8;
    let loop_track = if protocol == Protocol::NetQuake15 {
        Some(reader.read_bits(8)? as u8)
    } else {
        None
    };
    Ok((CdTrack { track, loop_track }, reader.byte_position()))
}

pub fn cd_track(protocol: Protocol, track: CdTrack, bytes: &mut [u8]) -> Result<usize, Error> {
    if matches!(protocol, Protocol::Quake2_34 | Protocol::Quake2Repro1038) {
        let index = protocol
            .config_layout()
            .and_then(|layout| layout.cd_track)
            .ok_or(Error::Unsupported)?;
        let mut text = FixedText::<3>::default();
        write!(text, "{}", track.track).map_err(|_| Error::Unsupported)?;
        return configstring(protocol, index as u32, text.as_bytes(), bytes);
    }
    if !matches!(protocol, Protocol::NetQuake15 | Protocol::QuakeWorld28) {
        return Err(Error::Unsupported);
    }
    let mut writer = Writer::new(bytes, Encoding::Bytes);
    writer.write_bits(32, 8)?;
    writer.write_bits(u32::from(track.track), 8)?;
    if protocol == Protocol::NetQuake15 {
        writer.write_bits(u32::from(track.loop_track.unwrap_or(track.track)), 8)?;
    }
    Ok(writer.size())
}
