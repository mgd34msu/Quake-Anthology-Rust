//! Original-C fixture IO for native command records, never game input.
use qa_core::{loopback::Endpoint, primitives::UserCmd, sys_events::EventTime};
use qa_network::{
    channel::{self, Channel, Delivery, commands::CommandContext},
    commands::{connection::Commands, packet::Protocol},
};
use std::io::{self, Read, Write};

fn word(input: &mut &[u8]) -> Result<u32, String> {
    let bytes = input.get(..4).ok_or("word")?;
    let value = u32::from_le_bytes(bytes.try_into().map_err(|_| "word")?);
    *input = &input[4..];
    Ok(value)
}
fn string(input: &mut &[u8]) -> Result<Vec<u8>, String> {
    let bytes = input.get(..2).ok_or("length")?;
    let n = u16::from_le_bytes(bytes.try_into().map_err(|_| "length")?) as usize;
    let text = input.get(2..2 + n).ok_or("string")?.to_vec();
    *input = &input[2 + n..];
    Ok(text)
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut input = Vec::new();
    io::stdin().read_to_end(&mut input)?;
    let mut input = &input[..];
    let mut output = io::stdout().lock();
    while !input.is_empty() {
        let context = CommandContext {
            server_id: word(&mut input)? as i32,
            challenge: word(&mut input)?,
            checksum_feed: word(&mut input)?,
        };
        let command = string(&mut input)?;
        let count = word(&mut input)?;
        let mut client = Channel::load(channel::QUAKE3, Endpoint::Client, 16384, 8)?;
        let mut server = Channel::load(channel::QUAKE3, Endpoint::Server, 16384, 8)?;
        for channel in [&mut client, &mut server] {
            channel.set_command_context(context)?;
        }
        client.queue_reliable(&command)?;
        let mut codec = Commands::load(Protocol::Quake3_68);
        let mut bytes = [0; 16384];
        let n = codec.encode(
            &UserCmd {
                server_time_ms: 16,
                ..UserCmd::default()
            },
            &client,
            &mut bytes,
        )?;
        let wire = client
            .prepare_move(&bytes[..n], EventTime(0), None)?
            .ok_or("move packet")?
            .bytes
            .to_vec();
        client.submitted(EventTime(0))?;
        let received = server.receive(&wire, EventTime(0))?;
        let sequence = received.header.sequence;
        let Delivery::Payload(body) = received.delivery else {
            return Err("move payload".into());
        };
        let n = codec.stage(body)?;
        codec.decode(n, sequence, 16, 16_000_000, &mut server)?;
        for _ in 0..count {
            server.queue_reliable(&string(&mut input)?)?;
        }
        let n = server.encode_server_output(&mut bytes, |_| Ok(()))?;
        output.write_all(&(n as u32).to_le_bytes())?;
        output.write_all(&bytes[..n])?;
        let mut parsed: Vec<(u32, Vec<u8>)> = Vec::new();
        let mut opcode = 8;
        loop {
            let wire = server
                .prepare_output(EventTime(1))?
                .ok_or("output packet")?
                .bytes
                .to_vec();
            server.submitted(EventTime(1))?;
            let received = client.receive(&wire, EventTime(1))?;
            let sequence = received.header.sequence;
            if let Delivery::Payload(body) = received.delivery {
                let mut bytes = body.to_vec();
                let result = client.decode_server_output(&mut bytes, sequence, |sequence, text| {
                    parsed.push((sequence, text.to_vec()))
                });
                match result {
                    Ok(()) => {}
                    Err(qa_network::commands::packet::Error::Opcode) => opcode = 0,
                    Err(e) => return Err(e.into()),
                }
                break;
            }
        }
        output.write_all(
            &client
                .command_state()
                .ok_or("state")?
                .acknowledged
                .to_le_bytes(),
        )?;
        output.write_all(&(parsed.len() as u32).to_le_bytes())?;
        for (sequence, text) in parsed {
            output.write_all(&sequence.to_le_bytes())?;
            output.write_all(&(text.len() as u16).to_le_bytes())?;
            output.write_all(&text)?;
        }
        output.write_all(&[opcode])?;
    }
    Ok(())
}
