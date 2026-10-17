use super::{Protocol, Reader, Writer, packet};
use qa_formats::text::Tokens;

/// Native protocol34 and1038 limits include the terminating byte.
pub const Q2_CONNECT_USERINFO_BYTES: usize = 512;
pub const Q2_CONNECT_PACKET_LENGTH: i32 = 1390;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Q2ConnectAccepted<'a> {
    pub map: Option<&'a [u8]>,
    pub download_server: Option<&'a [u8]>,
    pub anticheat: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Q2Connectionless<'a> {
    Challenge(i32),
    Accepted(Q2ConnectAccepted<'a>),
    Print(&'a [u8]),
}

fn next_word<'a>(words: &mut Tokens<'a>) -> Result<Option<&'a [u8]>, packet::Error> {
    words.next().map_err(|_| packet::Error::Context)
}

pub fn read_q2_connectionless(
    protocol: Protocol,
    bytes: &[u8],
) -> Result<Q2Connectionless<'_>, packet::Error> {
    if !matches!(protocol, Protocol::Quake2_34 | Protocol::Quake2Repro1038) {
        return Err(packet::Error::Context);
    }
    let reader = Reader::out_of_band(bytes)?;
    let raw = &bytes[reader.byte_position()..];
    let text = protocol.string_policy().tail(raw);
    let line_end = text.iter().position(|&byte| byte == b'\n');
    let line = line_end.map_or(text, |end| &text[..end]);
    if protocol == Protocol::Quake2Repro1038 && line.len() >= crate::message::COMMAND_STRING_BYTES {
        return Err(packet::Error::Count);
    }
    let mut words = Tokens::words(line);
    match next_word(&mut words)? {
        Some(b"challenge") => {
            let value = match next_word(&mut words)? {
                Some(value) => qa_core::text::atoi_prefix(value),
                None if protocol == Protocol::Quake2_34 => 0,
                None => return Err(packet::Error::Context),
            };
            let mut offered = false;
            let mut compatible = false;
            while let Some(word) = next_word(&mut words)? {
                if let Some(versions) = word.strip_prefix(b"p=") {
                    offered = true;
                    compatible |= versions.split(|&byte| byte == b',').any(|version| {
                        qa_core::text::atoi_prefix(version) == protocol.number() as i32
                    });
                }
            }
            if (!offered && protocol == Protocol::Quake2_34) || compatible {
                Ok(Q2Connectionless::Challenge(value))
            } else {
                Err(packet::Error::Context)
            }
        }
        Some(b"client_connect") => {
            let mut accepted = Q2ConnectAccepted::default();
            let default_netchan = i32::from(protocol == Protocol::Quake2Repro1038);
            let mut netchan = default_netchan;
            while let Some(word) = next_word(&mut words)? {
                if let Some(kind) = word.strip_prefix(b"nc=") {
                    if !kind.is_empty() {
                        netchan = qa_core::text::atoi_prefix(kind);
                        if !matches!(netchan, 0 | 1) {
                            return Err(packet::Error::Context);
                        }
                    }
                } else if let Some(map) = word.strip_prefix(b"map=") {
                    accepted.map = Some(map);
                } else if let Some(server) = word.strip_prefix(b"dlserver=") {
                    if accepted.download_server.is_none() {
                        accepted.download_server = Some(server);
                    }
                } else if let Some(anticheat) = word.strip_prefix(b"ac=") {
                    if !anticheat.is_empty() {
                        accepted.anticheat = qa_core::text::atoi_prefix(anticheat);
                    }
                }
            }
            // Native valid assignments overwrite earlier ones. Only their
            // final result selects the connection's supported framing.
            if netchan != default_netchan {
                return Err(packet::Error::Context);
            }
            Ok(Q2Connectionless::Accepted(accepted))
        }
        Some(b"print") => Ok(Q2Connectionless::Print(line_end.map_or(&b""[..], |end| {
            protocol.string_policy().tail(&raw[end + 1..])
        }))),
        _ => Err(packet::Error::Context),
    }
}

pub fn write_q2_getchallenge(output: &mut [u8]) -> Result<usize, packet::Error> {
    let mut writer = Writer::out_of_band(output)?;
    writer.write_data(b"getchallenge\n")?;
    Ok(writer.size())
}

/// Protocol1038's extra arguments are packet length and DEFLATE support.
/// Its internal revision and forced new-channel choice are not wire arguments.
pub fn write_q2_connect(
    protocol: Protocol,
    qport: u16,
    challenge: i32,
    userinfo: &[u8],
    packet_length: i32,
    output: &mut [u8],
) -> Result<usize, packet::Error> {
    use std::fmt::Write;
    if !matches!(protocol, Protocol::Quake2_34 | Protocol::Quake2Repro1038)
        || userinfo.len() >= Q2_CONNECT_USERINFO_BYTES
        || userinfo.iter().any(|&byte| {
            matches!(byte, 0 | b'"' | b'\r' | b'\n')
                || protocol == Protocol::Quake2_34 && byte == 255
        })
    {
        return Err(packet::Error::Context);
    }
    let qport = if protocol == Protocol::Quake2Repro1038 {
        u16::from(qport as u8)
    } else {
        qport
    };
    let mut prefix = qa_core::text::FixedText::<64>::default();
    write!(
        prefix,
        "connect {} {qport} {challenge} \"",
        protocol.number()
    )
    .map_err(|_| packet::Error::Count)?;
    let mut suffix = qa_core::text::FixedText::<32>::default();
    if protocol == Protocol::Quake2Repro1038 {
        // Native net_maxmsglen's zero disables its hard limit; nonzero
        // values use the same native cvar range as the original client.
        let length = if packet_length == 0 {
            0
        } else {
            packet_length.clamp(512, 4086)
        };
        write!(suffix, "\" {length} 1\n").map_err(|_| packet::Error::Count)?;
    } else {
        suffix.set("\"\n").map_err(|_| packet::Error::Count)?;
    }
    if 4 + prefix.as_bytes().len() + userinfo.len() + suffix.as_bytes().len() > output.len() {
        return Err(packet::Error::Count);
    }
    let mut writer = Writer::out_of_band(output)?;
    writer.write_data(prefix.as_bytes())?;
    writer.write_data(userinfo)?;
    writer.write_data(suffix.as_bytes())?;
    Ok(writer.size())
}
