//! Borrowed admission of the original Q3 svc_gamestate body.
use super::CommandContext;
use crate::{
    commands::packet,
    message::{Reader, StringPolicy, Writer},
    outputs::ConfigString,
    snapshots::{Entity, Q3Ring},
    states,
};
use qa_core::configstrings::{
    ConfigLayout, MAX_TEXT, Q3_GAME_STATE_DATA_BYTES, Q3_GAME_STATE_ROWS,
};

/// Original svc_gamestate body, borrowed directly from the source configuration
/// and baseline rows. Cloneable iterators permit complete input validation before
/// output begins without a second packed gamestate or baseline buffer.
pub fn write_gamestate<'a, C, I>(
    writer: &mut Writer<'_>,
    command_sequence: u32,
    native_client: u32,
    checksum_feed: u32,
    configs: C,
    baselines: I,
) -> Result<(), packet::Error>
where
    C: IntoIterator<Item = (usize, &'a [u8])>,
    C::IntoIter: Clone,
    I: IntoIterator<Item = Entity<{ states::ENTITY_WORDS }>>,
    I::IntoIter: Clone,
{
    if command_sequence > i32::MAX as u32 || native_client >= 64 {
        return Err(packet::Error::Context);
    }
    let configs = configs.into_iter();
    let baselines = baselines.into_iter();
    let mut previous = None;
    let mut data_count = 1usize;
    for (index, text) in configs.clone() {
        if index >= Q3_GAME_STATE_ROWS
            || previous.is_some_and(|previous| index <= previous)
            || text.len() >= MAX_TEXT
            || text.contains(&0)
        {
            return Err(packet::Error::Count);
        }
        previous = Some(index);
        if !text.is_empty() {
            data_count = data_count
                .checked_add(text.len() + 1)
                .filter(|&count| count <= Q3_GAME_STATE_DATA_BYTES)
                .ok_or(packet::Error::Count)?;
        }
    }
    if baselines.clone().any(|entity| entity.number >= 1024) {
        return Err(packet::Error::Count);
    }
    writer.write_bits(2, 8)?;
    writer.write_bits(command_sequence, 32)?;
    for (index, text) in configs {
        if !text.is_empty() {
            writer.write_bits(3, 8)?;
            writer.write_bits(index as u32, 16)?;
            writer.write_string(StringPolicy::Q3_BIG, text)?;
        }
    }
    for entity in baselines {
        // The original server omits the zero-number baseline, including client0.
        if entity.number != 0 {
            writer.write_bits(4, 8)?;
            states::write_q3_entity(
                writer,
                entity.number,
                &[0; states::ENTITY_WORDS],
                Some(&entity.words),
                true,
            )?;
        }
    }
    writer.write_bits(8, 8)?;
    writer.write_bits(native_client, 32)?;
    writer.write_bits(checksum_feed, 32)?;
    Ok(())
}

/// Accepted native startup scalars; no packed configuration or frame copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Q3Startup {
    pub message_sequence: u32,
    pub server_command_sequence: u32,
    pub last_executed_server_command: u32,
    pub client_num: u32,
    pub checksum_feed: u32,
}

/// Every byte of this body was checked before its configurations or baselines
/// replace accepted state. Its rows borrow a parked immutable message cursor.
#[derive(Clone)]
pub struct Q3GameState<'a> {
    reader: Reader<'a>,
    native_numbers: usize,
    server_id: i32,
    pub message_sequence: u32,
    pub server_command_sequence: u32,
    pub client_num: u32,
    pub checksum_feed: u32,
}

struct Scalars {
    commands: u32,
    client: u32,
    checksum: u32,
    server_id: i32,
}

// Both validation and admission use this one native record walker. Duplicate
// rows and even empty records contribute their native terminator to dataCount.
fn walk(
    reader: &mut Reader<'_>,
    native_numbers: usize,
    mut config: impl FnMut(ConfigString<'_>),
    mut baseline: impl FnMut(u32, &[u32; states::ENTITY_WORDS]),
) -> Result<Scalars, packet::Error> {
    let commands = reader.read_bits(32)?;
    if commands > i32::MAX as u32 {
        return Err(packet::Error::Context);
    }
    let mut context = CommandContext::default();
    let mut data_count = 1usize;
    let mut text = [0; 8192];
    loop {
        match reader.read_bits(8)? {
            8 => break,
            3 => {
                let index = reader.read_signed(16)?;
                if index < 0 || index as usize >= Q3_GAME_STATE_ROWS {
                    return Err(packet::Error::Count);
                }
                let text = reader.read_string_into(StringPolicy::Q3_BIG, &mut text)?;
                data_count = data_count
                    .checked_add(text.len() + 1)
                    .filter(|&n| n <= Q3_GAME_STATE_DATA_BYTES)
                    .ok_or(packet::Error::Count)?;
                if index as usize == ConfigLayout::Q3_SYSTEM_INFO {
                    context.set_system_info(text);
                }
                config(ConfigString {
                    index: index as u32,
                    text,
                });
            }
            4 => {
                let delta = states::read_q3_entity(reader, &[0; states::ENTITY_WORDS])?;
                if usize::from(delta.number) >= native_numbers {
                    return Err(packet::Error::Count);
                }
                baseline(
                    u32::from(delta.number),
                    &delta.words.unwrap_or([0; states::ENTITY_WORDS]),
                );
            }
            _ => return Err(packet::Error::Opcode),
        }
    }
    let client = reader.read_signed(32)?;
    if !(0..64).contains(&client) {
        return Err(packet::Error::Context);
    }
    let checksum = reader.read_bits(32)?;
    Ok(Scalars {
        commands,
        client: client as u32,
        checksum,
        server_id: context.server_id,
    })
}

impl<'a> Q3GameState<'a> {
    pub(crate) fn read(
        reader: &mut Reader<'a>,
        message_sequence: u32,
        native_numbers: usize,
    ) -> Result<Self, packet::Error> {
        let start = reader.clone();
        let scalars = walk(reader, native_numbers, |_| {}, |_, _| {})?;
        Ok(Self {
            reader: start,
            native_numbers,
            server_id: scalars.server_id,
            message_sequence,
            server_command_sequence: scalars.commands,
            client_num: scalars.client,
            checksum_feed: scalars.checksum,
        })
    }

    pub fn for_each_config(
        &self,
        consume: impl FnMut(ConfigString<'_>),
    ) -> Result<(), packet::Error> {
        walk(
            &mut self.reader.clone(),
            self.native_numbers,
            consume,
            |_, _| {},
        )?;
        Ok(())
    }

    pub(super) fn install_baselines(&self, ring: &mut Q3Ring) -> Result<(), packet::Error> {
        ring.reset_gamestate();
        walk(
            &mut self.reader.clone(),
            self.native_numbers,
            |_| {},
            |number, words| {
                // Validated native ordinal and freshly reset ring establish admission.
                let accepted = ring.set_baseline(number, words);
                debug_assert!(accepted);
            },
        )?;
        Ok(())
    }

    pub(super) fn install_context(&self, context: &mut CommandContext) {
        context.server_id = self.server_id;
        context.checksum_feed = self.checksum_feed;
    }

    pub(super) fn startup(&self, last_executed_server_command: u32) -> Q3Startup {
        Q3Startup {
            message_sequence: self.message_sequence,
            server_command_sequence: self.server_command_sequence,
            last_executed_server_command,
            client_num: self.client_num,
            checksum_feed: self.checksum_feed,
        }
    }
}
