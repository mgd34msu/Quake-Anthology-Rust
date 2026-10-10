//! Native entity-prefix oracle IO and per-record allocation qualification.
use qa_network::{
    message::{Encoding, Reader, Writer},
    states,
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
    let mut reader = Reader::new(&input, Encoding::Bytes);
    let mut output = std::io::BufWriter::new(std::io::stdout().lock());
    let mut checks = 0;
    while reader.byte_position() < input.len() {
        let mode = reader.read_bits(8).map_err(|e| e.to_string())?;
        if mode > 1 {
            return Err("fixture header width".into());
        }
        let number = reader.read_bits(16).map_err(|e| e.to_string())? as u16;
        let flags = u64::from(reader.read_bits(32).map_err(|e| e.to_string())?)
            | (u64::from(reader.read_bits(32).map_err(|e| e.to_string())?) << 32);
        let mut bytes = [0; 7];
        allocations::begin_frame();
        let result = (|| {
            let mut writer = Writer::new(&mut bytes, Encoding::Bytes);
            let flags = states::write_q2_entity_prefix(&mut writer, number, flags, mode == 1)?;
            let length = writer.size();
            let mut reader = Reader::new(writer.bytes(), Encoding::Bytes);
            let decoded = states::read_q2_entity_prefix(&mut reader, mode == 1)?;
            if decoded.flags != flags
                || decoded.number != number
                || reader.byte_position() != length
            {
                return Err(qa_network::message::Error {
                    byte: reader.byte_position(),
                    kind: qa_network::message::ErrorKind::Symbol,
                });
            }
            Ok((length, decoded))
        })();
        let heap = allocations::end_frame();
        let (length, decoded) = result.map_err(|e| e.to_string())?;
        if heap != allocations::Counts::default() {
            return Err(format!("entity header caller heap {heap:?}"));
        }
        checks += 1;
        output
            .write_all(&(length as u32).to_le_bytes())
            .map_err(|e| e.to_string())?;
        output
            .write_all(&bytes[..length])
            .map_err(|e| e.to_string())?;
        output
            .write_all(&decoded.flags.to_le_bytes())
            .map_err(|e| e.to_string())?;
        output
            .write_all(&decoded.number.to_le_bytes())
            .map_err(|e| e.to_string())?;
    }
    eprintln!(
        "{{\"scope\":\"native Q2 entity-prefix write/read; calling Rust thread; no body, frame, transport or gameplay\",\"cases\":{checks},\"positive_control_allocations\":1,\"allocations\":0,\"reallocations\":0,\"requested_bytes\":0,\"timing_run\":false}}"
    );
    output.flush().map_err(|e| e.to_string())
}
