//! Protocol68 connectionless CLIENT records. The connected MSG codec is separate.
use super::{Protocol, Reader, Writer, packet};
use crate::message::{AdaptiveHuffman, StringPolicy};
use qa_core::text::{FixedText, InfoUpdate};
use qa_formats::text::Tokens;
use std::fmt::Write;

pub const Q3_CONNECT_USERINFO_BYTES: usize = crate::message::COMMAND_STRING_BYTES;
// Native MSG_ReadStringLine has 1023 bytes for `connect "<info>"`.
const CONNECT_INFO_BYTES: usize = Q3_CONNECT_USERINFO_BYTES - 10;
const CONNECT_PREFIX: &[u8; 12] = b"\xff\xff\xff\xffconnect ";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Q3Connectionless<'a> {
    Challenge(i32),
    Accepted,
    /// Borrowed source bytes; native display sanitization belongs to the reader
    /// used by the print consumer, not to challenge/token recognition.
    Print(&'a [u8]),
    Disconnect,
}

pub fn read_q3_connectionless(bytes: &[u8]) -> Result<Q3Connectionless<'_>, packet::Error> {
    let reader = Reader::out_of_band(bytes)?;
    let raw = &bytes[reader.byte_position()..];
    let bounded = StringPolicy::Q3.tail(raw);
    let line_end = bounded.iter().position(|&byte| byte == b'\n');
    let line = line_end.map_or(bounded, |end| &bounded[..end]);
    let mut words = Tokens::entity(line, b"", true, Q3_CONNECT_USERINFO_BYTES);
    let command = words
        .next()
        .map_err(|_| packet::Error::Context)?
        .ok_or(packet::Error::Context)?;
    if command.eq_ignore_ascii_case(b"challengeResponse") {
        let challenge = words
            .next()
            .map_err(|_| packet::Error::Context)?
            .unwrap_or_default();
        Ok(Q3Connectionless::Challenge(qa_core::text::atoi_prefix(
            challenge,
        )))
    } else if command.eq_ignore_ascii_case(b"connectResponse") {
        Ok(Q3Connectionless::Accepted)
    } else if command.eq_ignore_ascii_case(b"disconnect") {
        Ok(Q3Connectionless::Disconnect)
    } else if command.eq_ignore_ascii_case(b"print") {
        let consumed = line_end.map_or_else(
            || bounded.len() + usize::from(raw.get(bounded.len()) == Some(&0)),
            |end| end + 1,
        );
        Ok(Q3Connectionless::Print(
            StringPolicy::Q3.tail(&raw[consumed..]),
        ))
    } else {
        Err(packet::Error::Context)
    }
}

pub fn write_q3_getchallenge(output: &mut [u8]) -> Result<usize, packet::Error> {
    let mut writer = Writer::out_of_band(output)?;
    writer.write_data(b"getchallenge")?;
    Ok(writer.size())
}

/// Overlay the three native handshake keys, then adaptively compress the quoted
/// info at absolute packet offset12. The receipt caller retains ordinary cvar
/// userinfo, without these generated handshake fields.
pub fn write_q3_connect(
    codec: &mut AdaptiveHuffman,
    qport: u16,
    challenge: i32,
    userinfo: &[u8],
    output: &mut [u8],
) -> Result<usize, packet::Error> {
    if userinfo.len() >= Q3_CONNECT_USERINFO_BYTES {
        return Err(packet::Error::Count);
    }
    if userinfo
        .iter()
        .any(|&byte| matches!(byte, 0 | b'"' | b'\r' | b'\n'))
    {
        return Err(packet::Error::Context);
    }
    let mut info = [0; Q3_CONNECT_USERINFO_BYTES];
    info[..userinfo.len()].copy_from_slice(userinfo);
    let mut length = userinfo.len();
    let mut port = FixedText::<16>::default();
    let mut number = FixedText::<16>::default();
    write!(port, "{qport}").map_err(|_| packet::Error::Count)?;
    write!(number, "{challenge}").map_err(|_| packet::Error::Count)?;
    let rules = Protocol::Quake3_68
        .userinfo_rules()
        .ok_or(packet::Error::Context)?;
    for (key, value) in [
        (b"protocol".as_slice(), b"68".as_slice()),
        (b"qport".as_slice(), port.as_bytes()),
        (b"challenge".as_slice(), number.as_bytes()),
    ] {
        InfoUpdate::new(key, value, rules)
            .map_err(|_| packet::Error::Context)?
            .first_only()
            .prepend()
            .apply(&mut info, &mut length)
            .map_err(|_| packet::Error::Count)?;
    }
    if length >= CONNECT_INFO_BYTES {
        return Err(packet::Error::Count);
    }
    let mut quoted = [0; Q3_CONNECT_USERINFO_BYTES];
    quoted[0] = b'"';
    quoted[1..length + 1].copy_from_slice(&info[..length]);
    quoted[length + 1] = b'"';
    let payload = output
        .get_mut(CONNECT_PREFIX.len()..)
        .ok_or(packet::Error::Count)?;
    let encoded = codec.compress(&quoted[..length + 2], payload)?;
    output[..CONNECT_PREFIX.len()].copy_from_slice(CONNECT_PREFIX);
    Ok(CONNECT_PREFIX.len() + encoded)
}
