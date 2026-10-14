//! Pointer-free system-event journal. Protocol packets remain byte-exact;
//! common times, physical devices and loopback client identities keep their
//! full engine widths. IO belongs to the existing platform file boundary.
use crate::{
    FormatError,
    read::{Reader, Writer},
};
use qa_core::{
    primitives::ClientId,
    sys_events::{DeviceId, EventKind, EventTime, Peer, SysEvent},
};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV6};

pub const FILE_HEADER: [u8; 8] = *b"QAJR\x01\0\0\0";
pub const RECORD_BYTES: usize = 48;

pub fn check_file_header(bytes: &[u8]) -> Result<(), FormatError> {
    if bytes == FILE_HEADER {
        Ok(())
    } else {
        Err(FormatError::Unsupported)
    }
}

fn write_device(writer: &mut Writer<'_>, device: DeviceId) -> Result<(), FormatError> {
    let (kind, number) = match device {
        DeviceId::Keyboard(number) => (0, number),
        DeviceId::Mouse(number) => (1, number),
        DeviceId::Controller(number) => (2, number as u32),
    };
    writer.u8(kind)?;
    writer.u32(number)
}
fn read_device(reader: &mut Reader<'_>) -> Result<DeviceId, FormatError> {
    let kind = reader.u8()?;
    let number = reader.u32()?;
    match kind {
        0 => Ok(DeviceId::Keyboard(number)),
        1 => Ok(DeviceId::Mouse(number)),
        2 => Ok(DeviceId::Controller(number as i32)),
        _ => Err(FormatError::InvalidValue),
    }
}
fn write_peer(writer: &mut Writer<'_>, peer: Peer) -> Result<(), FormatError> {
    match peer {
        Peer::Loopback(client) => {
            writer.u8(0)?;
            writer.u32(client.0)
        }
        Peer::Socket(SocketAddr::V4(address)) => {
            writer.u8(1)?;
            writer.put(&address.ip().octets())?;
            writer.u16(address.port())
        }
        Peer::Socket(SocketAddr::V6(address)) => {
            writer.u8(2)?;
            writer.put(&address.ip().octets())?;
            writer.u16(address.port())?;
            writer.u32(address.flowinfo())?;
            writer.u32(address.scope_id())
        }
    }
}
fn read_peer(reader: &mut Reader<'_>) -> Result<Peer, FormatError> {
    match reader.u8()? {
        0 => Ok(Peer::Loopback(ClientId(reader.u32()?))),
        1 => {
            let address = Ipv4Addr::from(
                <[u8; 4]>::try_from(reader.take(4)?).map_err(|_| FormatError::Truncated)?,
            );
            Ok(Peer::Socket(SocketAddr::new(
                IpAddr::V4(address),
                reader.u16()?,
            )))
        }
        2 => {
            let address = Ipv6Addr::from(
                <[u8; 16]>::try_from(reader.take(16)?).map_err(|_| FormatError::Truncated)?,
            );
            let port = reader.u16()?;
            let flow = reader.u32()?;
            let scope = reader.u32()?;
            Ok(Peer::Socket(SocketAddr::V6(SocketAddrV6::new(
                address, port, flow, scope,
            ))))
        }
        _ => Err(FormatError::InvalidValue),
    }
}

pub fn encode_header(event: SysEvent<'_>) -> Result<[u8; RECORD_BYTES], FormatError> {
    let length =
        u32::try_from(event.kind.payload().len()).map_err(|_| FormatError::InvalidRange)?;
    let mut bytes = [0; RECORD_BYTES];
    let mut writer = Writer::new(&mut bytes);
    writer.u32(length)?;
    writer.u64(event.time.0)?;
    match event.kind {
        EventKind::Time => writer.u8(0)?,
        EventKind::Key {
            device,
            code,
            symbol,
            down,
            repeat,
        } => {
            writer.u8(1)?;
            write_device(&mut writer, device)?;
            writer.u16(code)?;
            writer.u32(symbol as u32)?;
            writer.u8(u8::from(down))?;
            writer.u8(u8::from(repeat))?;
        }
        EventKind::Text { device, .. } => {
            writer.u8(2)?;
            write_device(&mut writer, device)?;
        }
        EventKind::Mouse { device, dx, dy } => {
            writer.u8(3)?;
            write_device(&mut writer, device)?;
            writer.u32(dx as u32)?;
            writer.u32(dy as u32)?;
        }
        EventKind::MouseButton {
            device,
            button,
            down,
        } => {
            writer.u8(4)?;
            write_device(&mut writer, device)?;
            writer.u8(button)?;
            writer.u8(u8::from(down))?;
        }
        EventKind::MouseWheel { device, x, y } => {
            writer.u8(5)?;
            write_device(&mut writer, device)?;
            writer.u32(x as u32)?;
            writer.u32(y as u32)?;
        }
        EventKind::ControllerAxis {
            device,
            axis,
            value,
        } => {
            writer.u8(6)?;
            write_device(&mut writer, device)?;
            writer.u8(axis)?;
            writer.u16(value as u16)?;
        }
        EventKind::ControllerButton {
            device,
            button,
            down,
        } => {
            writer.u8(7)?;
            write_device(&mut writer, device)?;
            writer.u8(button)?;
            writer.u8(u8::from(down))?;
        }
        EventKind::DeviceRemoved(device) => {
            writer.u8(8)?;
            write_device(&mut writer, device)?;
        }
        EventKind::Focus(value) => {
            writer.u8(9)?;
            writer.u8(u8::from(value))?;
        }
        EventKind::MouseCapture(value) => {
            writer.u8(10)?;
            writer.u8(u8::from(value))?;
        }
        EventKind::Quit => writer.u8(11)?,
        EventKind::ConsoleLine(_) => writer.u8(12)?,
        EventKind::Packet { socket, from, .. } => {
            writer.u8(13)?;
            writer.u16(socket)?;
            write_peer(&mut writer, from)?;
        }
    }
    Ok(bytes)
}

pub fn payload_length(header: &[u8; RECORD_BYTES]) -> usize {
    // The fixed record header has already been supplied in full.
    u32::from_le_bytes([header[0], header[1], header[2], header[3]]) as usize
}
fn boolean(reader: &mut Reader<'_>) -> Result<bool, FormatError> {
    match reader.u8()? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(FormatError::InvalidValue),
    }
}
pub fn decode<'a>(
    header: &[u8; RECORD_BYTES],
    payload: &'a [u8],
) -> Result<SysEvent<'a>, FormatError> {
    let mut reader = Reader::new(header);
    if reader.u32()? as usize != payload.len() {
        return Err(FormatError::InvalidRange);
    }
    let time = EventTime(reader.u64()?);
    let tag = reader.u8()?;
    if !matches!(tag, 2 | 12 | 13) && !payload.is_empty() {
        return Err(FormatError::InvalidRange);
    }
    let kind = match tag {
        0 => EventKind::Time,
        1 => EventKind::Key {
            device: read_device(&mut reader)?,
            code: reader.u16()?,
            symbol: reader.i32()?,
            down: boolean(&mut reader)?,
            repeat: boolean(&mut reader)?,
        },
        2 => EventKind::Text {
            device: read_device(&mut reader)?,
            text: std::str::from_utf8(payload).map_err(|_| FormatError::InvalidValue)?,
        },
        3 => EventKind::Mouse {
            device: read_device(&mut reader)?,
            dx: reader.i32()?,
            dy: reader.i32()?,
        },
        4 => EventKind::MouseButton {
            device: read_device(&mut reader)?,
            button: reader.u8()?,
            down: boolean(&mut reader)?,
        },
        5 => EventKind::MouseWheel {
            device: read_device(&mut reader)?,
            x: reader.i32()?,
            y: reader.i32()?,
        },
        6 => EventKind::ControllerAxis {
            device: read_device(&mut reader)?,
            axis: reader.u8()?,
            value: reader.i16()?,
        },
        7 => EventKind::ControllerButton {
            device: read_device(&mut reader)?,
            button: reader.u8()?,
            down: boolean(&mut reader)?,
        },
        8 => EventKind::DeviceRemoved(read_device(&mut reader)?),
        9 => EventKind::Focus(boolean(&mut reader)?),
        10 => EventKind::MouseCapture(boolean(&mut reader)?),
        11 => EventKind::Quit,
        12 => EventKind::ConsoleLine(
            std::str::from_utf8(payload).map_err(|_| FormatError::InvalidValue)?,
        ),
        13 => EventKind::Packet {
            socket: reader.u16()?,
            from: read_peer(&mut reader)?,
            bytes: payload,
        },
        _ => return Err(FormatError::InvalidValue),
    };
    if reader
        .take(RECORD_BYTES - reader.at)?
        .iter()
        .any(|&byte| byte != 0)
    {
        return Err(FormatError::InvalidValue);
    }
    Ok(SysEvent { time, kind })
}
