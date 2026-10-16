//! Borrowed native startup records enter the existing CLIENT resource rows.
use super::{packet, packet::Protocol};
use crate::{message::Reader, outputs::ConfigString};

#[derive(Clone)]
pub struct NqServerInfo<'a> {
    reader: Reader<'a>,
    pub max_clients: u8,
    pub game_type: u8,
    pub level_name: &'a [u8],
}

struct Header<'a> {
    max_clients: u8,
    game_type: u8,
    level_name: &'a [u8],
}

fn walk<'a>(
    reader: &mut Reader<'a>,
    mut consume: impl FnMut(ConfigString<'a>),
) -> Result<Header<'a>, packet::Error> {
    let protocol = Protocol::NetQuake15;
    if reader.read_bits(32)? != protocol.number() {
        return Err(packet::Error::Context);
    }
    let max_clients = reader.read_bits(8)? as u8;
    if max_clients == 0
        || u32::from(max_clients) > crate::snapshots::snapshot_limits(protocol).client_capacity
    {
        return Err(packet::Error::Count);
    }
    let game_type = reader.read_bits(8)? as u8;
    let level_name = reader.read_string(protocol.string_policy())?;
    for range in &protocol.resource_layout().resources[..2] {
        let mut ordinal = 1;
        loop {
            let text = reader.read_string(protocol.string_policy())?;
            if text.is_empty() {
                break;
            }
            let index = range.get(ordinal).ok_or(packet::Error::Count)?;
            if text.len() >= 64 {
                return Err(packet::Error::Count);
            }
            consume(ConfigString {
                index: index as u32,
                text,
            });
            ordinal += 1;
        }
    }
    Ok(Header {
        max_clients,
        game_type,
        level_name,
    })
}

impl<'a> NqServerInfo<'a> {
    pub(crate) fn read(reader: &mut Reader<'a>) -> Result<Self, packet::Error> {
        let start = reader.clone();
        let header = walk(reader, |_| {})?;
        Ok(Self {
            reader: start,
            max_clients: header.max_clients,
            game_type: header.game_type,
            level_name: header.level_name,
        })
    }

    pub fn for_each_config(
        &self,
        consume: impl FnMut(ConfigString<'a>),
    ) -> Result<(), packet::Error> {
        walk(&mut self.reader.clone(), consume)?;
        Ok(())
    }
}
