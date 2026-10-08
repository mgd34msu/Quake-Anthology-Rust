use qa_core::checksum::*;
use std::{fs, path::PathBuf};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(std::env::args_os().nth(1).ok_or("evidence root required")?);
    let input = fs::read(root.join("inputs.bin"))?;
    let expected = fs::read(root.join("expected.bin"))?;
    let mut at = 0;
    let mut actual = Vec::with_capacity(expected.len());
    let mut cases = 0;
    while at < input.len() {
        let length = u32::from_le_bytes(input[at..at + 4].try_into()?) as usize;
        let key = u32::from_le_bytes(input[at + 4..at + 8].try_into()?);
        let sequence = u32::from_le_bytes(input[at + 8..at + 12].try_into()?);
        at += 12;
        let bytes = &input[at..at + length];
        at += length;
        actual.extend(block_checksum(bytes).to_le_bytes());
        actual.extend(block_checksum_key(bytes, key).to_le_bytes());
        actual.extend(crc_block(bytes).to_le_bytes());
        actual.push(qw_sequence_crc(bytes, sequence));
        actual.push(q2_sequence_crc(bytes, sequence));
        cases += 1;
    }
    assert_eq!(actual, expected);
    assert_eq!(cases, 10130);
    println!(
        "PASS: {cases} original-C inputs; 50650 block,keyed,CRC,QW/Q2 sequence checksum results bit-identical"
    );
    Ok(())
}
