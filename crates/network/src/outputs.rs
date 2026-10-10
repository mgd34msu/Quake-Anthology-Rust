//! Native output payloads. Protocol limits and opcodes are connection data.
use crate::{
    commands::packet::Protocol,
    message::{Encoding, Error as MessageError, Writer},
};
use qa_core::primitives::PrintKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Message(MessageError),
    Unsupported,
    Truncated,
    Opcode,
}
impl From<MessageError> for Error {
    fn from(error: MessageError) -> Self {
        Self::Message(error)
    }
}
#[derive(Clone, Copy)]
struct PrintFormat {
    print: u8,
    center: u8,
    layout: Option<u8>,
    level: bool,
    chat_prefix: bool,
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
        },
        Protocol::QuakeWorld28 => PrintFormat {
            print: 8,
            center: 26,
            layout: None,
            level: true,
            chat_prefix: false,
        },
        Protocol::Quake2_34 | Protocol::Quake2Repro1038 => PrintFormat {
            print: 10,
            center: 15,
            layout: Some(4),
            level: true,
            chat_prefix: false,
        },
        _ => return Err(Error::Unsupported),
    }))
}
pub fn print(
    protocol: Protocol,
    kind: PrintKind,
    level: u8,
    text: &[u8],
    output: &mut [u8],
) -> Result<usize, Error> {
    let Format::Bytes(format) = format(protocol)? else {
        let verb = match kind {
            PrintKind::Center => &b"cp \""[..],
            PrintKind::Chat => &b"chat \""[..],
            PrintKind::Layout => return Err(Error::Unsupported),
            _ => &b"print \""[..],
        };
        let n = text.iter().position(|&b| b == 0).unwrap_or(text.len());
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
        PrintKind::Center => (format.center, false),
        PrintKind::Layout => (format.layout.ok_or(Error::Unsupported)?, false),
        _ => (format.print, format.level),
    };
    let chat_prefix = format.chat_prefix && kind == PrintKind::Chat;
    let n = text.iter().position(|&b| b == 0).unwrap_or(text.len());
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
    writer.write_data(&text[..n])?;
    writer.write_bits(0, 8)?;
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
            kind = if level == 3 {
                PrintKind::Chat
            } else if level < 2 {
                PrintKind::Notify
            } else {
                PrintKind::Console
            };
            (Some(level), rest)
        } else {
            (None, rest)
        };
        let Some(n) = rest.iter().position(|&b| b == 0) else {
            self.rest = &[];
            return Some(Err(Error::Truncated));
        };
        let mut text = &rest[..n];
        self.rest = &rest[n + 1..];
        if format.chat_prefix && kind == PrintKind::Console && text.first() == Some(&1) {
            kind = PrintKind::Chat;
            text = &text[1..];
        }
        Some(Ok(Print { kind, level, text }))
    }
}
