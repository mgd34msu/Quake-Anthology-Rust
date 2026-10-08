use qa_core::checksum::*;
#[test]
fn original_block_and_crc_vectors_and_prefix_key_use_one_implementation() {
    // RFC 1320 digests reduced using original Com_BlockChecksum's word XOR.
    for (bytes, words) in [
        (
            b"".as_slice(),
            [0xe0cfd631u32, 0x31e96ad1, 0xd7593cb7, 0xc089c0e0],
        ),
        (b"a", [0xb32ce5bd, 0x463ee31d, 0xfb055e24, 0x24fbd6db]),
        (b"abc", [0x7a0148a4, 0x52d821af, 0xe80ac15f, 0x9d72a67a]),
    ] {
        assert_eq!(
            block_checksum(bytes),
            words.into_iter().fold(0, |a, b| a ^ b)
        );
    }
    assert_eq!(crc_block(b""), 0xffff);
    assert_eq!(crc_block(b"123456789"), 0x29b1);
    let mut bytes = 0xabcdef12u32.to_le_bytes().to_vec();
    bytes.extend(b"message");
    assert_eq!(
        block_checksum_key(b"message", 0xabcdef12),
        block_checksum(&bytes)
    );
    assert_eq!(
        q2_sequence_crc(&[7; 100], 1021),
        q2_sequence_crc(&[7; 60], 1021)
    );
    assert_eq!(
        qw_sequence_crc(&[7; 100], 1021),
        qw_sequence_crc(&[7; 60], 1021)
    );
}
