//! Native packet oracle IO. No recorded input is ever sent to a game.
use qa_network::{
    commands::{
        Q1Move, delta,
        packet::{self, Acknowledgements, Key, Move, ZERO_Q3},
    },
    message::{Encoding, Reader},
};
use qa_platform::allocations;
use std::io::{Read, Write};

#[global_allocator]
static ALLOCATOR: allocations::CountingAllocator = allocations::CountingAllocator;

fn words(r: &mut Reader<'_>) -> Result<[u32; 11], String> {
    let mut words = [0; 11];
    for word in &mut words {
        *word = r.read_bits(32).map_err(|e| e.to_string())?;
    }
    Ok(words)
}
fn native_words(movement: Move) -> [[u32; 11]; 3] {
    let mut rows = [[0; 11]; 3];
    match movement {
        Move::NetQuake { command, .. } => {
            rows[2][..3].copy_from_slice(&command.view_angles.0.map(f32::to_bits));
            rows[2][3..6].copy_from_slice(&command.movement.map(|v| v as u32));
            rows[2][6] = command.buttons.into();
            rows[2][7] = command.impulse.into();
        }
        Move::QuakeWorld { commands, .. } => rows = commands.map(delta::qw_words),
        Move::Quake2 { commands, .. } => rows = commands.map(delta::q2_words),
        Move::Quake3 { commands, .. } => {
            rows = std::array::from_fn(|i| delta::q3_words(commands[i]))
        }
    }
    rows
}
fn main() -> Result<(), String> {
    allocations::begin_frame();
    let control = Box::new(std::hint::black_box(1u64));
    std::hint::black_box(&control);
    drop(control);
    if allocations::end_frame().allocations != 1 {
        return Err("heap positive control".into());
    }
    let mut qw_connection = qa_network::commands::connection::Commands::load(
        qa_network::commands::packet::Protocol::QuakeWorld28,
    );
    let mut qw_channel = qa_network::channel::Channel::load(
        qa_network::channel::QUAKEWORLD,
        qa_core::loopback::Endpoint::Server,
        8192,
        8,
    )
    .map_err(|e| e.to_string())?;
    let mut checks = 0;
    let mut input = Vec::new();
    std::io::stdin()
        .read_to_end(&mut input)
        .map_err(|e| e.to_string())?;
    let mut r = Reader::new(&input, Encoding::Bytes);
    let mut output = std::io::BufWriter::new(std::io::stdout().lock());
    while r.byte_position() < input.len() {
        let mode = r.read_bits(8).map_err(|e| e.to_string())?;
        let sequence = r.read_bits(32).map_err(|e| e.to_string())?;
        let mut context = [0; 5];
        for word in &mut context {
            *word = r.read_bits(32).map_err(|e| e.to_string())?;
        }
        let loss = r.read_bits(8).map_err(|e| e.to_string())? as u8;
        let timestamp = r.read_float().map_err(|e| e.to_string())?;
        let rows = [words(&mut r)?, words(&mut r)?, words(&mut r)?];
        let length = r.read_bits(8).map_err(|e| e.to_string())? as usize;
        let mut text = [0; 255];
        r.read_data(&mut text[..length])
            .map_err(|e| e.to_string())?;
        let movement = match mode {
            0 => Some(Move::NetQuake {
                timestamp,
                command: Q1Move {
                    view_angles: delta::qw_from_words(&rows[2]).view_angles,
                    movement: delta::qw_from_words(&rows[2]).movement,
                    buttons: rows[2][6] as u8,
                    impulse: rows[2][7] as u8,
                },
            }),
            1 => Some(Move::QuakeWorld {
                loss,
                commands: rows.map(|v| delta::qw_from_words(&v)),
                delta_request: (context[1] != 0 && sequence.wrapping_sub(context[1]) < 63)
                    .then_some(context[1] as u8),
            }),
            2 => Some(Move::Quake2 {
                last_frame: -1,
                commands: rows.map(|v| delta::q2_from_words(&v)),
            }),
            3 => {
                let mut commands = [ZERO_Q3; 32];
                for (c, v) in commands.iter_mut().zip(&rows) {
                    *c = delta::q3_from_words(v);
                }
                Some(Move::Quake3 {
                    commands,
                    count: 3,
                    delta: false,
                })
            }
            4 => None,
            _ => return Err("fixture protocol".into()),
        };
        let key = Key {
            acknowledgements: Acknowledgements {
                server_id: context[0] as i32,
                message: context[1] as i32,
                reliable: context[2] as i32,
            },
            challenge: context[3],
            checksum_feed: context[4],
            server_command: &text[..length],
        };
        let mut packet = [0; 1400];
        allocations::begin_frame();
        let encoded = (|| {
            let Some(movement) = movement else {
                let commands = rows.map(|v| delta::q2_rr_from_words(&v));
                let frame = context[1] as i32;
                let n = packet::write_q2_repro_move(&mut packet, frame, &commands)?;
                let (decoded_frame, commands) = packet::read_q2_repro_move(&packet[..n])?;
                if decoded_frame != frame {
                    return Err(packet::Error::Context);
                }
                return Ok((n, commands.map(delta::q2_rr_words)));
            };
            let n = packet::write(&mut packet, &movement, sequence, key)?;
            let mut scratch = packet;
            let decoded = packet::read(movement.protocol(), &mut scratch[..n], sequence, key)?;
            if let Move::QuakeWorld { delta_request, .. } = decoded {
                let length = qw_connection.stage(&packet[..n])?;
                qw_connection.decode(length, sequence, 0, 0, &mut qw_channel)?;
                if qw_connection.delta_request() != delta_request.map(u32::from) {
                    return Err(packet::Error::Context);
                }
            }
            Ok((n, native_words(decoded)))
        })();
        let heap = allocations::end_frame();
        let (n, decoded) = encoded.map_err(|e: packet::Error| e.to_string())?;
        if heap != allocations::Counts::default() {
            return Err(format!("move packet caller heap {heap:?}"));
        }
        checks += 1;
        output
            .write_all(&(n as u32).to_le_bytes())
            .map_err(|e| e.to_string())?;
        output.write_all(&packet[..n]).map_err(|e| e.to_string())?;
        for row in decoded {
            for word in row {
                output
                    .write_all(&word.to_le_bytes())
                    .map_err(|e| e.to_string())?;
            }
        }
    }
    eprintln!(
        "{{\"scope\":\"native move packet encode/decode and QW SERVER delta request; caller Rust heap, no app/workers/OS/gameplay\",\"cases\":{checks},\"positive_control_allocations\":1,\"allocations\":0,\"reallocations\":0,\"requested_bytes\":0,\"timing_run\":false}}"
    );
    output.flush().map_err(|e| e.to_string())
}
