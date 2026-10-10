//! Original-C frame-prefix fixture IO; player/entity bodies are excluded.
use qa_network::{
    message::{Encoding, Reader, Writer},
    snapshots::Q2Header,
};
use qa_platform::allocations;
use std::io::{Read, Write};

#[global_allocator]
static ALLOCATOR: allocations::CountingAllocator = allocations::CountingAllocator;

fn main() -> Result<(), String> {
    allocations::begin_frame();
    let positive = Box::new(std::hint::black_box(1u64));
    std::hint::black_box(&positive);
    drop(positive);
    if allocations::end_frame().allocations != 1 {
        return Err("heap positive control".into());
    }
    let mut input = Vec::new();
    std::io::stdin()
        .read_to_end(&mut input)
        .map_err(|e| e.to_string())?;
    let mut input_reader = Reader::new(&input, Encoding::Bytes);
    let mut output = std::io::BufWriter::new(std::io::stdout().lock());
    let mut checks = 0;
    while input_reader.byte_position() < input.len() {
        let mode = input_reader.read_bits(8).map_err(|e| e.to_string())?;
        if mode > 2 {
            return Err("frame fixture mode".into());
        }
        let header = Q2Header {
            sequence: input_reader.read_bits(32).map_err(|e| e.to_string())?,
            delta: input_reader.read_bits(32).map_err(|e| e.to_string())? as i32,
            flags: input_reader.read_bits(8).map_err(|e| e.to_string())? as u8,
            player_flags: input_reader.read_bits(8).map_err(|e| e.to_string())? as u8,
        };
        let count = input_reader.read_bits(8).map_err(|e| e.to_string())? as usize;
        let mut areas = [0; 255];
        input_reader
            .read_data(&mut areas[..count])
            .map_err(|e| e.to_string())?;
        let mut bytes = [0; 300];
        let mut decoded_areas = [0; 255];
        allocations::begin_frame();
        let result = (|| {
            let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
            if mode == 2 {
                header.write::<true>(&mut writer, &areas[..count])?;
            } else {
                header.write::<false>(&mut writer, &areas[..count])?;
                // Empty body sentinels match the cold C prefix-only stubs.
                writer.write_bits(17, 8)?;
                writer.write_bits(18, 8)?;
            }
            let length = writer.size();
            let mut reader = Reader::new(writer.bytes(), Encoding::Bytes);
            if reader.read_bits(8)? != 20 {
                return Err(qa_network::commands::packet::Error::Opcode);
            }
            let (decoded, n) = if mode == 2 {
                Q2Header::read::<true>(&mut reader, &mut decoded_areas, 255)?
            } else {
                Q2Header::read::<false>(&mut reader, &mut decoded_areas, 255)?
            };
            if mode != 2 && (reader.read_bits(8)? != 17 || reader.read_bits(8)? != 18) {
                return Err(qa_network::commands::packet::Error::Opcode);
            }
            if n != count || reader.byte_position() != length {
                return Err(qa_network::commands::packet::Error::Count);
            }
            Ok((length, decoded))
        })();
        let heap = allocations::end_frame();
        let (length, decoded) = result.map_err(|e| e.to_string())?;
        if heap != allocations::Counts::default() {
            return Err(format!("frame prefix caller heap {heap:?}"));
        }
        output
            .write_all(&(length as u32).to_le_bytes())
            .map_err(|e| e.to_string())?;
        output
            .write_all(&bytes[..length])
            .map_err(|e| e.to_string())?;
        output
            .write_all(&decoded.sequence.to_le_bytes())
            .map_err(|e| e.to_string())?;
        output
            .write_all(&decoded.delta.to_le_bytes())
            .map_err(|e| e.to_string())?;
        output
            .write_all(&[decoded.flags, decoded.player_flags, count as u8])
            .map_err(|e| e.to_string())?;
        output
            .write_all(&decoded_areas[..count])
            .map_err(|e| e.to_string())?;
        checks += 1;
    }
    output.flush().map_err(|e| e.to_string())?;
    eprintln!(
        "{{\"scope\":\"native Q2 frame prefix, caller heap only; player/entity bodies excluded\",\"cases\":{checks},\"allocations\":0,\"reallocations\":0,\"requested_bytes\":0,\"positive_control_allocations\":1,\"timing_run\":false}}"
    );
    Ok(())
}
