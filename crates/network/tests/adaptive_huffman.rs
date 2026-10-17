use qa_network::message::{AdaptiveHuffman, ErrorKind};

#[test]
fn native_adaptive_prefix_nyt_repetition_and_padding_match_authored_bytes() {
    let mut codec = AdaptiveHuffman::new();
    for (input, expected) in [
        (b"".as_slice(), b"".as_slice()),
        (b"A".as_slice(), b"\x00\x01\x82\x00".as_slice()),
        (b"AAAAAA".as_slice(), b"\x00\x06\x82\x1f".as_slice()),
        (
            b"BANANA".as_slice(),
            b"\x00\x06\x42\x04\x91\xbb\x00".as_slice(),
        ),
    ] {
        let mut encoded = [0xcc; 64];
        let length = codec.compress(input, &mut encoded).unwrap();
        assert_eq!(&encoded[..length], expected);
        assert_eq!(encoded[length], 0xcc);
        let mut decoded = [0xcc; 64];
        assert_eq!(
            codec.decompress(expected, &mut decoded).unwrap(),
            input.len()
        );
        assert_eq!(&decoded[..input.len()], input);
        assert_eq!(decoded[input.len()], 0xcc);
    }
}

#[test]
fn all_symbols_and_rank_repairs_match_independent_source_generated_packets() {
    // Generated once through quake-typescript network/q3/huffman.ts
    // compressAdaptive, using these authored byte sequences. No test codec.
    let all: Vec<u8> = (0..=255).collect();
    let ranks: Vec<u8> = all
        .iter()
        .copied()
        .chain(all.iter().rev().copied())
        .chain((0..1024).map(|index| ((index * 37 + (index >> 3)) & 255) as u8))
        .collect();
    let mut codec = AdaptiveHuffman::default();
    for (input, expected) in [
        (
            all.as_slice(),
            include_bytes!("fixtures/q3-adaptive-all-bytes.bin").as_slice(),
        ),
        (
            ranks.as_slice(),
            include_bytes!("fixtures/q3-adaptive-ranks.bin").as_slice(),
        ),
    ] {
        let mut encoded = [0xcc; 4096];
        let length = codec.compress(input, &mut encoded).unwrap();
        assert_eq!(&encoded[..length], expected);
        assert_eq!(encoded[length], 0xcc);
        let mut decoded = [0xcc; 2048];
        assert_eq!(
            codec.decompress(expected, &mut decoded).unwrap(),
            input.len()
        );
        assert_eq!(&decoded[..input.len()], input);
        assert_eq!(decoded[input.len()], 0xcc);
    }
}

#[test]
fn bounded_failures_reset_the_same_scratch_before_retry() {
    let mut codec = AdaptiveHuffman::new();
    for input in [b"\0".as_slice(), b"\0\x02\x82".as_slice()] {
        assert_eq!(
            codec.decompress(input, &mut [0; 2]).unwrap_err().kind,
            ErrorKind::Truncated
        );
    }
    let mut short = [0xcc; 1];
    assert_eq!(
        codec
            .decompress(b"\0\x02\x82\x01", &mut short)
            .unwrap_err()
            .kind,
        ErrorKind::Capacity
    );
    assert_eq!(short, [0xcc]);
    assert_eq!(
        codec.compress(b"A", &mut [0; 3]).unwrap_err().kind,
        ErrorKind::Capacity
    );
    assert_eq!(
        codec
            .compress(&vec![0; 65536], &mut [0; 64])
            .unwrap_err()
            .kind,
        ErrorKind::Capacity
    );
    let mut encoded = [0xcc; 4];
    assert_eq!(codec.compress(b"A", &mut encoded).unwrap(), 4);
    assert_eq!(encoded, [0, 1, 0x82, 0]);
    let mut decoded = [0xcc; 1];
    // Padding and bytes after the declared body are not decoded symbols.
    assert_eq!(
        codec
            .decompress(b"\0\x01\x82\0\xff\x42", &mut decoded)
            .unwrap(),
        1
    );
    assert_eq!(decoded, *b"A");
    assert_eq!(codec.decompress(b"\0\0", &mut []).unwrap(), 0);
}
