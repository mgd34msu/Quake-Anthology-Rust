//! Q3's native reliable command fields, shared by both channel directions.
use super::{Channel, TransmitError};
use crate::{
    commands::packet,
    message::{Encoding, Reader, StringPolicy, Writer},
};
use qa_core::{events::NativeReceipt, loopback::Endpoint};

const SLOTS: usize = 64;

const fn slot(sequence: u32) -> usize {
    sequence as usize & (SLOTS - 1)
}
const fn command_opcode(endpoint: Endpoint) -> u8 {
    match endpoint {
        Endpoint::Server => 5,
        Endpoint::Client => 4,
    }
}
pub(crate) use crate::message::COMMAND_STRING_BYTES as STRING_BYTES;

struct Slot {
    text: [u8; STRING_BYTES],
    length: usize,
    sequence: u32,
    receipt: Option<NativeReceipt>,
    first_message: Option<u32>,
    encoded_bits: usize,
}
struct Window {
    slots: Box<[Slot]>,
    sequence: u32,
    acknowledged: u32,
    sent: u32,
    encoded_bits: usize,
}
impl Window {
    fn load() -> Self {
        Self {
            slots: (0..SLOTS)
                .map(|_| Slot {
                    text: [0; STRING_BYTES],
                    length: 0,
                    sequence: 0,
                    receipt: None,
                    first_message: None,
                    encoded_bits: 0,
                })
                .collect(),
            sequence: 0,
            acknowledged: 0,
            sent: 0,
            encoded_bits: 0,
        }
    }
    fn text(&self, sequence: u32) -> &[u8] {
        let slot = &self.slots[slot(sequence)];
        &slot.text[..slot.length]
    }
    fn store(&mut self, sequence: u32, text: &[u8], receipt: Option<NativeReceipt>) {
        let length = qa_core::text::until_nul(text).len().min(STRING_BYTES - 1);
        let slot = &mut self.slots[slot(sequence)];
        slot.text[..length].copy_from_slice(&text[..length]);
        slot.text[length] = 0;
        slot.length = length;
        slot.sequence = sequence;
        slot.receipt = receipt;
        slot.first_message = None;
        slot.encoded_bits = 0;
        self.sequence = sequence;
    }
    fn queue(
        &mut self,
        text: &[u8],
        receipt: NativeReceipt,
        opcode: u8,
        maximum: Option<usize>,
    ) -> Result<(), TransmitError> {
        // MSG_ReadString stops after STRING_BYTES - 1 payload bytes. Keep
        // room for its terminator so the next service remains readable.
        if qa_core::text::until_nul(text).len() >= STRING_BYTES - 1 {
            return Err(TransmitError::MessageTooLarge);
        }
        if self.sequence - self.acknowledged >= SLOTS as u32 || self.sequence == i32::MAX as u32 {
            return Err(TransmitError::Full);
        }
        let sequence = self.sequence + 1;
        let encoded_bits = crate::message::q3_bits(u32::from(opcode), 8)
            + crate::message::q3_bits(sequence, 32)
            + command_bytes(text)
                .map(|byte| crate::message::q3_bits(u32::from(byte), 8))
                .sum::<usize>();
        if let Some(maximum) = maximum {
            // Reserve the changing native ACK field, svc_EOF and MSG's padding
            // bit. Every admitted SERVER window must drain in a command-only
            // message, even before any of its commands can receive an ACK.
            let reserved = 4 * crate::message::Q3_MAX_BYTE_BITS + crate::message::q3_bits(8, 8) + 1;
            let budget = maximum.saturating_mul(8).saturating_sub(reserved);
            if encoded_bits > budget {
                return Err(TransmitError::MessageTooLarge);
            }
            if self.encoded_bits + encoded_bits > budget {
                return Err(TransmitError::Full);
            }
        }
        self.store(self.sequence + 1, text, Some(receipt));
        self.slots[slot(sequence)].encoded_bits = encoded_bits;
        self.encoded_bits += encoded_bits;
        Ok(())
    }
    fn submitted(&mut self, sequence: u32, message: u32) {
        for n in self.sent + 1..=sequence {
            let slot = &mut self.slots[slot(n)];
            slot.first_message.get_or_insert(message);
        }
        self.sent = self.sent.max(sequence);
    }
    fn acknowledge(
        &mut self,
        sequence: u32,
        message: u32,
        receipts: &mut [NativeReceipt],
    ) -> Result<usize, packet::Error> {
        if sequence > self.sent {
            return Err(packet::Error::Context);
        }
        if sequence <= self.acknowledged {
            return Ok(0);
        }
        for n in self.acknowledged + 1..=sequence {
            let slot = &self.slots[slot(n)];
            if slot.sequence != n || slot.first_message.is_none_or(|sent| message < sent) {
                return Err(packet::Error::Context);
            }
        }
        let mut count = 0;
        for n in self.acknowledged + 1..=sequence {
            self.encoded_bits -= self.slots[slot(n)].encoded_bits;
            if let Some(receipt) = self.slots[slot(n)].receipt {
                receipts[count] = receipt;
                count += 1;
            }
        }
        self.acknowledged = sequence;
        Ok(count)
    }
    fn receive(&mut self, sequence: u32, text: &[u8], gaps: bool) -> Result<bool, packet::Error> {
        if sequence > i32::MAX as u32 {
            return Err(packet::Error::Context);
        }
        if sequence <= self.sequence {
            return Ok(false);
        }
        if !gaps && sequence != self.sequence + 1 {
            return Err(packet::Error::Context);
        }
        self.store(sequence, text, None);
        Ok(true)
    }
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct CommandContext {
    pub server_id: i32,
    pub challenge: u32,
    pub checksum_feed: u32,
}
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct CommandState {
    pub queued: u32,
    pub submitted: u32,
    pub acknowledged: u32,
    pub received: u32,
    pub message_acknowledged: u32,
}
pub(super) struct CommandMessages {
    outgoing: Window,
    incoming: Window,
    context: CommandContext,
    message_acknowledged: u32,
}
impl CommandMessages {
    pub(super) fn load() -> Self {
        Self {
            outgoing: Window::load(),
            incoming: Window::load(),
            context: CommandContext::default(),
            message_acknowledged: 0,
        }
    }
    pub(super) fn pending(&self) -> bool {
        self.outgoing.sequence != self.outgoing.acknowledged
    }
    pub(super) fn queue(
        &mut self,
        text: &[u8],
        receipt: NativeReceipt,
        endpoint: Endpoint,
        maximum: Option<usize>,
    ) -> Result<(), TransmitError> {
        self.outgoing
            .queue(text, receipt, command_opcode(endpoint), maximum)
    }
    pub(super) fn submitted(&mut self, sequence: u32, message: u32) {
        self.outgoing.submitted(sequence, message);
    }
}

// msg.c uses bytewise native string conversion inside its existing MSG stream.
fn command_bytes(text: &[u8]) -> impl Iterator<Item = u8> + '_ {
    let n = qa_core::text::until_nul(text).len();
    text[..if n < STRING_BYTES { n } else { 0 }]
        .iter()
        .copied()
        .map(|byte| if byte > 127 { b'.' } else { byte })
        .chain(std::iter::once(0))
}
pub fn write_string(writer: &mut Writer<'_>, text: &[u8]) -> Result<(), packet::Error> {
    for byte in command_bytes(text) {
        writer.write_bits(u32::from(byte), 8)?;
    }
    Ok(())
}
impl Channel {
    pub fn command_state(&self) -> Option<CommandState> {
        let c = self.commands.as_ref()?;
        Some(CommandState {
            queued: c.outgoing.sequence,
            submitted: c.outgoing.sent,
            acknowledged: c.outgoing.acknowledged,
            received: c.incoming.sequence,
            message_acknowledged: c.message_acknowledged,
        })
    }
    pub fn set_command_context(&mut self, context: CommandContext) -> Result<(), packet::Error> {
        let c = self.commands.as_mut().ok_or(packet::Error::Context)?;
        // A new gamestate changes serverId, not reliable-command lifetimes.
        c.context = context;
        Ok(())
    }
    pub fn command_key(
        &self,
        ack: Option<packet::Acknowledgements>,
    ) -> Result<packet::Key<'_>, packet::Error> {
        let c = self.commands.as_ref().ok_or(packet::Error::Context)?;
        let acknowledgements = if let Some(ack) = ack {
            if ack.server_id != c.context.server_id
                || ack.message < 0
                || ack.reliable < 0
                || ack.message as u32 > self.send_state().sequence.saturating_sub(1)
                || ack.reliable as u32 > c.outgoing.sent
                || (ack.reliable as u32).saturating_add(SLOTS as u32) < c.outgoing.sequence
            {
                return Err(packet::Error::Context);
            }
            ack
        } else {
            packet::Acknowledgements {
                server_id: c.context.server_id,
                message: c.message_acknowledged as i32,
                reliable: c.incoming.sequence as i32,
            }
        };
        Ok(packet::Key {
            challenge: c.context.challenge,
            checksum_feed: c.context.checksum_feed,
            acknowledgements,
            server_command: if ack.is_some() {
                c.outgoing.text(acknowledgements.reliable as u32)
            } else {
                c.incoming.text(c.incoming.sequence)
            },
        })
    }
    pub fn acknowledge_commands(
        &mut self,
        reliable: u32,
        message: u32,
    ) -> Result<(), packet::Error> {
        let c = self.commands.as_mut().ok_or(packet::Error::Context)?;
        self.transmit.receipt_count =
            c.outgoing
                .acknowledge(reliable, message, &mut self.transmit.receipts)?;
        Ok(())
    }
    pub fn receive_command(&mut self, sequence: u32, text: &[u8]) -> Result<bool, packet::Error> {
        let gaps = self.endpoint() == Endpoint::Client;
        self.commands
            .as_mut()
            .ok_or(packet::Error::Context)?
            .incoming
            .receive(sequence, text, gaps)
    }
    pub fn received_command(&self) -> Option<(u32, &[u8])> {
        let c = self.commands.as_ref()?;
        Some((c.incoming.sequence, c.incoming.text(c.incoming.sequence)))
    }
    pub fn write_command_records(&self, writer: &mut Writer<'_>) -> Result<(), packet::Error> {
        let c = self.commands.as_ref().ok_or(packet::Error::Context)?;
        let opcode = command_opcode(self.endpoint());
        for n in c.outgoing.acknowledged + 1..=c.outgoing.sequence {
            writer.write_bits(u32::from(opcode), 8)?;
            writer.write_bits(n, 32)?;
            write_string(writer, c.outgoing.text(n))?;
        }
        Ok(())
    }

    /// Original SV_UpdateServerCommandsToClient and SV_Netchan_Encode payload.
    /// The shared prepare/submitted path still owns headers and fragmentation.
    pub fn encode_server_output(
        &mut self,
        out: &mut [u8],
        body: impl FnOnce(&mut Writer<'_>, &mut Channel) -> Result<(), packet::Error>,
    ) -> Result<usize, packet::Error> {
        if self.endpoint() != Endpoint::Server {
            return Err(packet::Error::Context);
        }
        let c = self.commands.as_ref().ok_or(packet::Error::Context)?;
        let incoming = c.incoming.sequence;
        let key = (c.context.challenge ^ self.send_state().sequence) as u8;
        let text = c.incoming.text(incoming);
        let length = text.len();
        let mut command_text = [0; STRING_BYTES];
        command_text[..length].copy_from_slice(text);
        let mut writer = Writer::new(out, Encoding::Q3);
        writer.write_bits(incoming, 32)?;
        self.write_command_records(&mut writer)?;
        body(&mut writer, self)?;
        writer.write_bits(8, 8)?; // svc_EOF
        let n = writer.size();
        packet::xor(&mut out[..n], 4, key, &command_text[..length]);
        Ok(n)
    }
    /// Native server commands and snapshots share this MSG stream. Gamestate
    /// and download opcodes remain explicit until their adapters are present.
    pub fn decode_server_output(
        &mut self,
        bytes: &mut [u8],
        sequence: u32,
        mut consume: impl FnMut(u32, &[u8]),
    ) -> Result<(), packet::Error> {
        if self.endpoint() != Endpoint::Client {
            return Err(packet::Error::Context);
        }
        let c = self.commands.as_mut().ok_or(packet::Error::Context)?;
        let reliable = Reader::new(bytes, Encoding::Q3).read_bits(32)?;
        if reliable > c.outgoing.sent || reliable.saturating_add(SLOTS as u32) < c.outgoing.sequence
        {
            return Err(packet::Error::Context);
        }
        packet::xor(
            bytes,
            4,
            (c.context.challenge ^ sequence) as u8,
            c.outgoing.text(reliable),
        );
        self.transmit.receipt_count =
            c.outgoing
                .acknowledge(reliable, u32::MAX, &mut self.transmit.receipts)?;
        // The server's native ACK field has no message ACK. Its accepted channel
        // sequence establishes delivery; it still cannot ACK an unsent command.
        let mut reader = Reader::new(bytes, Encoding::Q3);
        reader.read_bits(32)?;
        let mut text = [0; STRING_BYTES];
        // Each opcode consumes bits; the fixed MSG boundary bounds this loop.
        // A full reliable window followed by a snapshot must still reach EOF.
        loop {
            match reader.read_bits(8)? {
                8 => {
                    c.message_acknowledged = sequence;
                    return Ok(());
                }
                5 => {
                    let seq = reader.read_bits(32)?;
                    let text = reader.read_string_into(StringPolicy::Q3, &mut text)?;
                    if c.incoming.receive(seq, text, true)? {
                        consume(seq, text);
                    }
                }
                1 => {} // svc_nop
                7 => {
                    let Some(crate::snapshots::Storage::Quake3(snapshots)) =
                        self.snapshots.as_mut()
                    else {
                        return Err(packet::Error::Context);
                    };
                    crate::snapshots::read_q3(
                        &mut reader,
                        snapshots,
                        sequence,
                        c.incoming.sequence,
                    )?;
                }
                _ => return Err(packet::Error::Opcode),
            }
        }
    }
}
