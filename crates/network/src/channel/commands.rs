//! Q3's native reliable command fields, shared by both channel directions.
use super::{Channel, TransmitError};
use crate::{
    commands::packet,
    message::{Encoding, Reader, Writer},
};
use qa_core::{events::NativeReceipt, loopback::Endpoint};

const SLOTS: usize = 64;
const STRING_BYTES: usize = 1024;

struct Slot {
    text: [u8; STRING_BYTES],
    length: usize,
    sequence: u32,
    receipt: Option<NativeReceipt>,
    first_message: Option<u32>,
}
struct Window {
    slots: Box<[Slot]>,
    sequence: u32,
    acknowledged: u32,
    sent: u32,
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
                })
                .collect(),
            sequence: 0,
            acknowledged: 0,
            sent: 0,
        }
    }
    fn text(&self, sequence: u32) -> &[u8] {
        let slot = &self.slots[sequence as usize & (SLOTS - 1)];
        &slot.text[..slot.length]
    }
    fn store(&mut self, sequence: u32, text: &[u8], receipt: Option<NativeReceipt>) {
        let length = text
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(text.len())
            .min(STRING_BYTES - 1);
        let slot = &mut self.slots[sequence as usize & (SLOTS - 1)];
        slot.text[..length].copy_from_slice(&text[..length]);
        slot.text[length] = 0;
        slot.length = length;
        slot.sequence = sequence;
        slot.receipt = receipt;
        slot.first_message = None;
        self.sequence = sequence;
    }
    fn queue(&mut self, text: &[u8], receipt: NativeReceipt) -> Result<(), TransmitError> {
        if self.sequence - self.acknowledged >= SLOTS as u32 || self.sequence == i32::MAX as u32 {
            return Err(TransmitError::Full);
        }
        self.store(self.sequence + 1, text, Some(receipt));
        Ok(())
    }
    fn submitted(&mut self, sequence: u32, message: u32) {
        for n in self.sent + 1..=sequence {
            let slot = &mut self.slots[n as usize & (SLOTS - 1)];
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
            let slot = &self.slots[n as usize & (SLOTS - 1)];
            if slot.sequence != n || slot.first_message.is_none_or(|sent| message < sent) {
                return Err(packet::Error::Context);
            }
        }
        let mut count = 0;
        for n in self.acknowledged + 1..=sequence {
            if let Some(receipt) = self.slots[n as usize & (SLOTS - 1)].receipt {
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
    ) -> Result<(), TransmitError> {
        self.outgoing.queue(text, receipt)
    }
    pub(super) fn submitted(&mut self, sequence: u32, message: u32) {
        self.outgoing.submitted(sequence, message);
    }
}

// msg.c uses bytewise native string conversion inside its existing MSG stream.
pub fn write_string(writer: &mut Writer<'_>, text: &[u8]) -> Result<(), packet::Error> {
    let n = text.iter().position(|&b| b == 0).unwrap_or(text.len());
    if n < STRING_BYTES {
        for &b in &text[..n] {
            writer.write_bits(u32::from(if b > 127 { b'.' } else { b }), 8)?;
        }
    }
    writer.write_bits(0, 8)?;
    Ok(())
}
pub fn read_string<'a>(
    reader: &mut Reader<'_>,
    text: &'a mut [u8; STRING_BYTES],
) -> Result<&'a [u8], packet::Error> {
    let mut n = 0;
    while n < STRING_BYTES - 1 {
        let b = reader.read_bits(8)? as u8;
        if b == 0 {
            break;
        }
        text[n] = if b > 127 || b == b'%' { b'.' } else { b };
        n += 1;
    }
    text[n] = 0;
    Ok(&text[..n])
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
        let opcode = if self.endpoint() == Endpoint::Server {
            5
        } else {
            4
        };
        for n in c.outgoing.acknowledged + 1..=c.outgoing.sequence {
            writer.write_bits(opcode, 8)?;
            writer.write_bits(n, 32)?;
            write_string(writer, c.outgoing.text(n))?;
        }
        Ok(())
    }

    /// Original SV_UpdateServerCommandsToClient and SV_Netchan_Encode payload.
    /// The shared prepare/submitted path still owns headers and fragmentation.
    pub fn encode_server_output(
        &self,
        out: &mut [u8],
        body: impl FnOnce(&mut Writer<'_>) -> Result<(), packet::Error>,
    ) -> Result<usize, packet::Error> {
        if self.endpoint() != Endpoint::Server {
            return Err(packet::Error::Context);
        }
        let c = self.commands.as_ref().ok_or(packet::Error::Context)?;
        let mut writer = Writer::new(out, Encoding::Q3);
        writer.write_bits(c.incoming.sequence, 32)?;
        self.write_command_records(&mut writer)?;
        body(&mut writer)?;
        writer.write_bits(8, 8)?; // svc_EOF
        let n = writer.size();
        packet::xor(
            &mut out[..n],
            4,
            (c.context.challenge ^ self.send_state().sequence) as u8,
            c.incoming.text(c.incoming.sequence),
        );
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
                    let text = read_string(&mut reader, &mut text)?;
                    if c.incoming.receive(seq, text, true)? {
                        consume(seq, text);
                    }
                }
                1 => {} // svc_nop
                7 => {
                    let snapshots = self.snapshots.as_mut().ok_or(packet::Error::Context)?;
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
