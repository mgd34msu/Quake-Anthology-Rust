//! Native output payloads. Protocol limits and opcodes are connection data.
use crate::{
    commands::packet::Protocol,
    message::{Encoding, Error as MessageError, ErrorKind, Reader, Writer},
};
use qa_core::primitives::{PrintKind, RuleSetId};

mod configstrings;
mod cues;
mod player_values;
mod static_entities;
pub use configstrings::{
    ConfigChunk, ConfigCommandReader, ConfigCommandStep, ConfigString, config_command,
    configstring, read_configstring, read_configstrings,
};
use cues::project_cue;
pub use cues::{Cue, MuzzleFlash, MuzzleFormat, Sound, read_cue, sound, write_cue};
pub use player_values::{
    PLAYER_VALUE_SLOTS, player_value, read_player_value, read_set_angles, separate_player_values,
    set_angles,
};
pub use static_entities::{STATIC_ENTITY_BYTES, read_static_entity, static_entity};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Message(MessageError),
    Unsupported,
    Truncated,
    Opcode,
    /// A producer resource has not reached its current native ACK yet.
    Pending,
}
impl From<MessageError> for Error {
    fn from(error: MessageError) -> Self {
        Self::Message(error)
    }
}
fn string_error(error: MessageError) -> Error {
    if error.kind == ErrorKind::Truncated {
        Error::Truncated
    } else {
        Error::Message(error)
    }
}
fn read_text<'a>(protocol: Protocol, reader: &mut Reader<'a>) -> Result<&'a [u8], Error> {
    reader
        .read_string(protocol.string_policy())
        .map_err(string_error)
}
/// One borrowed service stream uses the same scalar readers as CLIENT ingress.
/// Unknown opcodes stop the stream: their payload length cannot be inferred.
pub enum NativeService<'a> {
    Cue(Cue),
    Print(Print<'a>),
    Nop,
}
pub fn read_native_service(
    protocol: Protocol,
    bytes: &[u8],
) -> Result<(NativeService<'_>, usize), Error> {
    match (protocol, bytes.first().copied().ok_or(Error::Truncated)?) {
        (Protocol::NetQuake15 | Protocol::QuakeWorld28, 29)
        | (Protocol::Quake2_34 | Protocol::Quake2Repro1038, 1 | 2 | 9 | 32) => {
            let (cue, length) = read_cue(protocol, bytes)?;
            Ok((NativeService::Cue(cue), length))
        }
        (Protocol::NetQuake15 | Protocol::QuakeWorld28, 8 | 26)
        | (Protocol::Quake2_34 | Protocol::Quake2Repro1038, 4 | 10 | 15) => {
            let mut prints = Prints::new(protocol, bytes);
            let value = prints.next().ok_or(Error::Truncated)??;
            Ok((
                NativeService::Print(value),
                bytes.len() - prints.remaining().len(),
            ))
        }
        (Protocol::NetQuake15 | Protocol::QuakeWorld28, 1)
        | (Protocol::Quake2_34 | Protocol::Quake2Repro1038, 6) => Ok((NativeService::Nop, 1)),
        _ => Err(Error::Unsupported),
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NativeEncoding {
    pub length: usize,
    /// Recognized services that the destination cannot represent.
    pub unsupported: u64,
}

/// gi.WritePosition selects short coordinates for classic Q2 and float
/// coordinates for the rerelease. Convert recognized services through their
/// existing readers/writers; never send one coordinate stream as the other.
/// Same-source opaque payloads retain their original bytes.
pub fn native(
    protocol: Protocol,
    source: RuleSetId,
    bytes: &[u8],
    output: &mut [u8],
    primary: bool,
    mut project: impl FnMut(Cue, usize) -> Result<Cue, Error>,
) -> Result<NativeEncoding, Error> {
    let source_rules = source;
    let source = Protocol::native(source);
    if source == Protocol::Quake3_68 {
        return Err(Error::Unsupported);
    }
    if protocol == Protocol::Quake3_68 {
        return Err(Error::Unsupported);
    }
    let mut encoded = NativeEncoding::default();
    let mut remaining = bytes;
    while !remaining.is_empty() {
        let offset = bytes.len() - remaining.len();
        let (service, consumed) = match read_native_service(source, remaining) {
            Ok(value) => value,
            Err(Error::Unsupported) if primary && source == protocol => {
                let capacity = output.len();
                let target = output
                    .get_mut(encoded.length..encoded.length + remaining.len())
                    .ok_or(Error::Message(MessageError {
                        byte: capacity,
                        kind: crate::message::ErrorKind::Capacity,
                    }))?;
                target.copy_from_slice(remaining);
                encoded.length += remaining.len();
                return Ok(encoded);
            }
            Err(error) => return Err(error),
        };
        remaining = &remaining[consumed..];
        let target = &mut output[encoded.length..];
        let result = match service {
            NativeService::Cue(cue) => project(cue, offset)
                .and_then(|cue| project_cue(protocol, cue))
                .and_then(|cue| write_cue(protocol, cue, target)),
            NativeService::Print(value) => print(
                protocol,
                value.kind,
                value.level.unwrap_or(value.kind.default_level()),
                value.text,
                Some(PrintSource {
                    source: source_rules,
                    primary,
                    wire: &bytes[offset..offset + consumed],
                    consumed,
                }),
                target,
            ),
            NativeService::Nop => {
                let mut writer = Writer::new(target, Encoding::Bytes);
                writer
                    .write_bits(
                        if matches!(protocol, Protocol::NetQuake15 | Protocol::QuakeWorld28) {
                            1
                        } else {
                            6
                        },
                        8,
                    )
                    .map(|()| writer.size())
                    .map_err(Error::from)
            }
        };
        match result {
            Ok(length) => encoded.length += length,
            Err(Error::Unsupported) => encoded.unsupported += 1,
            Err(error) => return Err(error),
        }
    }
    Ok(encoded)
}

#[derive(Clone, Copy)]
struct PrintFormat {
    print: u8,
    center: u8,
    layout: Option<u8>,
    level: bool,
    chat_prefix: bool,
    localized: bool,
}
enum Format {
    Bytes(PrintFormat),
    Commands,
}
fn format(protocol: Protocol) -> Result<Format, Error> {
    if protocol == Protocol::Quake3_68 {
        return Ok(Format::Commands);
    }
    Ok(Format::Bytes(match protocol {
        Protocol::NetQuake15 => PrintFormat {
            print: 8,
            center: 26,
            layout: None,
            level: false,
            chat_prefix: true,
            localized: false,
        },
        Protocol::QuakeWorld28 => PrintFormat {
            print: 8,
            center: 26,
            layout: None,
            level: true,
            chat_prefix: false,
            localized: false,
        },
        Protocol::Quake2_34 | Protocol::Quake2Repro1038 => PrintFormat {
            print: 10,
            center: 15,
            layout: Some(4),
            level: true,
            chat_prefix: false,
            localized: protocol == Protocol::Quake2Repro1038,
        },
        _ => return Err(Error::Unsupported),
    }))
}
/// Original source bytes are retained only for the primary native namespace.
#[derive(Clone, Copy)]
pub struct PrintSource<'a> {
    pub source: RuleSetId,
    pub primary: bool,
    pub wire: &'a [u8],
    pub consumed: usize,
}
pub fn print(
    protocol: Protocol,
    kind: PrintKind,
    mut level: u8,
    text: &[u8],
    source: Option<PrintSource<'_>>,
    output: &mut [u8],
) -> Result<usize, Error> {
    if let Some(source) = source {
        if source.primary && protocol == Protocol::native(source.source) {
            let mut writer = Writer::new(output, Encoding::Bytes);
            writer.write_data(source.wire)?;
            return Ok(writer.size());
        }
        if source.consumed != source.wire.len() {
            return Err(Error::Unsupported);
        }
        // Rerelease levels 4/5 request center display, unlike classic priorities.
        if protocol == Protocol::Quake2Repro1038
            && kind != PrintKind::Center
            && PrintKind::from_localized_level(level) != kind
        {
            level = kind.default_level();
        }
    }
    let Format::Bytes(format) = format(protocol)? else {
        let verb = match kind {
            PrintKind::Center => &b"cp \""[..],
            PrintKind::Chat => &b"chat \""[..],
            PrintKind::Layout => return Err(Error::Unsupported),
            _ => &b"print \""[..],
        };
        let n = qa_core::text::until_nul(text).len();
        let need = verb.len() + n + 2;
        let capacity = output.len();
        let out = output.get_mut(..need).ok_or(Error::Message(MessageError {
            byte: capacity,
            kind: crate::message::ErrorKind::Capacity,
        }))?;
        out[..verb.len()].copy_from_slice(verb);
        out[verb.len()..verb.len() + n].copy_from_slice(&text[..n]);
        out[need - 2] = b'"';
        out[need - 1] = 0;
        return Ok(need);
    };
    let (opcode, has_level) = match kind {
        PrintKind::Center
            if format.localized && PrintKind::from_localized_level(level) == PrintKind::Center =>
        {
            (format.print, format.level)
        }
        PrintKind::Center => (format.center, false),
        PrintKind::Layout => (format.layout.ok_or(Error::Unsupported)?, false),
        _ => (format.print, format.level),
    };
    let chat_prefix = format.chat_prefix && kind == PrintKind::Chat;
    let policy = protocol.string_policy().reserved(usize::from(chat_prefix));
    let n = policy.payload(text).len();
    let need = n + 2 + usize::from(has_level) + usize::from(chat_prefix);
    let capacity = output.len();
    let output = output.get_mut(..need).ok_or(Error::Message(MessageError {
        byte: capacity,
        kind: crate::message::ErrorKind::Capacity,
    }))?;
    let mut writer = Writer::new(output, Encoding::Bytes);
    writer.write_bits(opcode.into(), 8)?;
    if has_level {
        writer.write_bits(level.into(), 8)?;
    }
    if chat_prefix {
        writer.write_bits(1, 8)?;
    }
    writer.write_string(policy, &text[..n])?;
    Ok(writer.size())
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Print<'a> {
    pub kind: PrintKind,
    pub level: Option<u8>,
    pub text: &'a [u8],
}
pub struct Prints<'a> {
    protocol: Protocol,
    rest: &'a [u8],
}
impl<'a> Prints<'a> {
    pub fn new(protocol: Protocol, bytes: &'a [u8]) -> Self {
        Self {
            protocol,
            rest: bytes,
        }
    }
    /// Allows a service stream to continue after the same native print parser.
    pub fn remaining(&self) -> &'a [u8] {
        self.rest
    }
}
impl<'a> Iterator for Prints<'a> {
    type Item = Result<Print<'a>, Error>;
    fn next(&mut self) -> Option<Self::Item> {
        let (&opcode, rest) = self.rest.split_first()?;
        let format = match format(self.protocol) {
            Ok(Format::Bytes(format)) => format,
            Ok(Format::Commands) => {
                self.rest = &[];
                return Some(Err(Error::Unsupported));
            }
            Err(error) => {
                self.rest = &[];
                return Some(Err(error));
            }
        };
        let (mut kind, has_level) = if opcode == format.center {
            (PrintKind::Center, false)
        } else if format.layout == Some(opcode) {
            (PrintKind::Layout, false)
        } else if opcode == format.print {
            (PrintKind::Console, format.level)
        } else {
            self.rest = &[];
            return Some(Err(Error::Opcode));
        };
        let (level, rest) = if has_level {
            let Some((&level, rest)) = rest.split_first() else {
                self.rest = &[];
                return Some(Err(Error::Truncated));
            };
            kind = if format.localized {
                PrintKind::from_localized_level(level)
            } else {
                PrintKind::from_level(level)
            };
            (Some(level), rest)
        } else {
            (None, rest)
        };
        let mut reader = Reader::new(rest, Encoding::Bytes);
        let text = match read_text(self.protocol, &mut reader) {
            Ok(text) => text,
            Err(error) => {
                self.rest = &[];
                return Some(Err(error));
            }
        };
        self.rest = &rest[reader.byte_position()..];
        let mut text = text;
        if format.chat_prefix && kind == PrintKind::Console && text.first() == Some(&1) {
            kind = PrintKind::Chat;
            text = &text[1..];
        }
        Some(Ok(Print { kind, level, text }))
    }
}
