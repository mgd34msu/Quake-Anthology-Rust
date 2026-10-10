//! Native packet oracle IO. No recorded input is ever sent to a game.
use qa_core::primitives::Vec3;
use qa_network::{
    commands::{
        Q1Move, Q2Cmd, Q3Cmd, QwCmd,
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
fn qw(v: &[u32; 11]) -> QwCmd {
    QwCmd {
        view_angles: Vec3(std::array::from_fn(|i| f32::from_bits(v[i]))),
        movement: std::array::from_fn(|i| v[3 + i] as i16),
        buttons: v[6] as u8,
        impulse: v[7] as u8,
        msec: v[8] as u8,
    }
}
fn q2(v: &[u32; 11]) -> Q2Cmd {
    Q2Cmd {
        angles: std::array::from_fn(|i| v[i] as i16),
        movement: std::array::from_fn(|i| v[3 + i] as i16),
        buttons: v[6] as u8,
        impulse: v[7] as u8,
        msec: v[8] as u8,
        light_level: v[9] as u8,
    }
}
fn q3(v: &[u32; 11]) -> Q3Cmd {
    Q3Cmd {
        server_time: v[10] as i32,
        angles: std::array::from_fn(|i| v[i] as i32),
        movement: std::array::from_fn(|i| v[3 + i] as i8),
        buttons: v[6],
        weapon: v[7] as u8,
    }
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
        Move::QuakeWorld { commands, .. } => {
            for (v, c) in rows.iter_mut().zip(commands) {
                v[..3].copy_from_slice(&c.view_angles.0.map(f32::to_bits));
                v[3..6].copy_from_slice(&c.movement.map(|v| v as u32));
                v[6] = c.buttons.into();
                v[7] = c.impulse.into();
                v[8] = c.msec.into();
            }
        }
        Move::Quake2 { commands, .. } => {
            for (v, c) in rows.iter_mut().zip(commands) {
                v[..3].copy_from_slice(&c.angles.map(|v| v as u32));
                v[3..6].copy_from_slice(&c.movement.map(|v| v as u32));
                v[6] = c.buttons.into();
                v[7] = c.impulse.into();
                v[8] = c.msec.into();
                v[9] = c.light_level.into();
            }
        }
        Move::Quake3 { commands, .. } => {
            for (v, c) in rows.iter_mut().zip(commands) {
                v[..3].copy_from_slice(&c.angles.map(|v| v as u32));
                v[3..6].copy_from_slice(&c.movement.map(|v| v as u32));
                v[6] = c.buttons;
                v[7] = c.weapon.into();
                v[10] = c.server_time as u32;
            }
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
            0 => Move::NetQuake {
                timestamp,
                command: Q1Move {
                    view_angles: qw(&rows[2]).view_angles,
                    movement: qw(&rows[2]).movement,
                    buttons: rows[2][6] as u8,
                    impulse: rows[2][7] as u8,
                },
            },
            1 => Move::QuakeWorld {
                loss,
                commands: rows.map(|v| qw(&v)),
                delta_request: (context[1] != 0 && sequence.wrapping_sub(context[1]) < 63)
                    .then_some(context[1] as u8),
            },
            2 => Move::Quake2 {
                last_frame: -1,
                commands: rows.map(|v| q2(&v)),
            },
            3 => {
                let mut commands = [ZERO_Q3; 32];
                for (c, v) in commands.iter_mut().zip(&rows) {
                    *c = q3(v);
                }
                Move::Quake3 {
                    commands,
                    count: 3,
                    delta: false,
                }
            }
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
            Ok((n, decoded))
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
        for row in native_words(decoded) {
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
