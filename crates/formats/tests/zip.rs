use qa_formats::{
    FormatError,
    archive::{Archive, ArchiveKind},
};
use qa_platform::files::ReadFile;
use std::{
    io::Write,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture(std::path::PathBuf);
impl Fixture {
    fn parse(bytes: &[u8]) -> (Self, Result<Archive, FormatError>) {
        let path = std::env::temp_dir().join(format!(
            "qa-rust-zip-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        qa_platform::files::write(&path, bytes).unwrap();
        let archive = Archive::parse(Arc::new(ReadFile::open(&path).unwrap()));
        (Self(path), archive)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        qa_platform::files::remove_file(&self.0).unwrap();
    }
}

fn zip(payload: &[u8], deflate: bool, descriptor: bool, prefix: &[u8]) -> Vec<u8> {
    let name = b"maps/fixture.bsp";
    let compressed = if deflate {
        let mut encoder =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(payload).unwrap();
        encoder.finish().unwrap()
    } else {
        payload.to_vec()
    };
    let crc = crc32fast::hash(payload);
    let flags: u16 = if descriptor { 8 } else { 0 };
    let method: u16 = if deflate { 8 } else { 0 };
    let mut local = [0; 30];
    local[..4].copy_from_slice(&0x04034b50u32.to_le_bytes());
    local[4..6].copy_from_slice(&20u16.to_le_bytes());
    local[6..8].copy_from_slice(&flags.to_le_bytes());
    local[8..10].copy_from_slice(&method.to_le_bytes());
    if !descriptor {
        local[14..18].copy_from_slice(&crc.to_le_bytes());
        local[18..22].copy_from_slice(&(compressed.len() as u32).to_le_bytes());
        local[22..26].copy_from_slice(&(payload.len() as u32).to_le_bytes());
    }
    local[26..28].copy_from_slice(&(name.len() as u16).to_le_bytes());
    let mut bytes = prefix.to_vec();
    bytes.extend(local);
    bytes.extend(name);
    bytes.extend(&compressed);
    if descriptor {
        for value in [
            0x08074b50,
            crc,
            compressed.len() as u32,
            payload.len() as u32,
        ] {
            bytes.extend(value.to_le_bytes());
        }
    }
    let central_at = bytes.len() - prefix.len();
    let mut central = [0; 46];
    central[..4].copy_from_slice(&0x02014b50u32.to_le_bytes());
    central[6..8].copy_from_slice(&20u16.to_le_bytes());
    central[8..10].copy_from_slice(&flags.to_le_bytes());
    central[10..12].copy_from_slice(&method.to_le_bytes());
    central[16..20].copy_from_slice(&crc.to_le_bytes());
    central[20..24].copy_from_slice(&(compressed.len() as u32).to_le_bytes());
    central[24..28].copy_from_slice(&(payload.len() as u32).to_le_bytes());
    central[28..30].copy_from_slice(&(name.len() as u16).to_le_bytes());
    bytes.extend(central);
    bytes.extend(name);
    let mut end = [0; 22];
    end[..4].copy_from_slice(&0x06054b50u32.to_le_bytes());
    end[8..10].copy_from_slice(&1u16.to_le_bytes());
    end[10..12].copy_from_slice(&1u16.to_le_bytes());
    end[12..16].copy_from_slice(&((46 + name.len()) as u32).to_le_bytes());
    end[16..20].copy_from_slice(&(central_at as u32).to_le_bytes());
    bytes.extend(end);
    bytes
}

#[test]
fn stored_deflate_descriptors_and_stub_prefix_read_into_bounded_buffers() {
    let mut archive_reader = qa_formats::archive::ArchiveReader::default();
    for deflate in [false, true] {
        for descriptor in [false, true] {
            let bytes = zip(b"123456789", deflate, descriptor, b"stub prefix");
            let (_fixture, archive) = Fixture::parse(&bytes);
            let archive = archive.unwrap();
            assert_eq!(archive.kind, ArchiveKind::Zip);
            assert_eq!(archive.entries[0].crc32, Some(0xcbf43926));
            let mut out = [0; 9];
            assert_eq!(
                archive
                    .read_into_reusing(0, &mut out, &mut archive_reader)
                    .unwrap(),
                9
            );
            assert_eq!(&out, b"123456789");
            assert_eq!(
                archive.read_into_reusing(0, &mut [0; 8], &mut archive_reader),
                Err(FormatError::InvalidRange)
            );
        }
    }
}

#[test]
fn reused_decoder_range_reads_preserve_large_members_and_check_unread_tail_crc() {
    let payload: Vec<_> = (0..40000).map(|i| ((i * 73) ^ (i >> 5)) as u8).collect();
    for deflate in [false, true] {
        let bytes = zip(&payload, deflate, false, b"");
        let (_fixture, archive) = Fixture::parse(&bytes);
        let archive = archive.unwrap();
        let mut reader = qa_formats::archive::ArchiveReader::default();
        let mut out = [0; 4096];
        for offset in [0, 8000, 18000, 39990, 40000] {
            let n = archive
                .read_range_reusing(0, offset, &mut out, &mut reader)
                .unwrap();
            assert_eq!(&out[..n], &payload[offset as usize..offset as usize + n]);
        }
        assert_eq!(
            archive.read_range_reusing(0, 40001, &mut out, &mut reader),
            Err(FormatError::InvalidRange)
        );
        if deflate {
            let mut bad = bytes.clone();
            // Corrupt the authoritative ZIP CRC, including its unread tail.
            let at = bad
                .windows(4)
                .position(|w| w == 0x02014b50u32.to_le_bytes())
                .unwrap();
            bad[at + 16] ^= 1;
            bad[14] ^= 1;
            let (_fixture, archive) = Fixture::parse(&bad);
            assert_eq!(
                archive
                    .unwrap()
                    .read_range_reusing(0, 8000, &mut out, &mut reader),
                Err(FormatError::Checksum)
            );
        }
    }
}

#[test]
fn corrupt_crc_truncation_and_local_name_disagreement_are_scoped_errors() {
    let mut archive_reader = qa_formats::archive::ArchiveReader::default();
    let mut crc = zip(b"123456789", false, false, b"");
    crc[30 + b"maps/fixture.bsp".len()] ^= 1;
    let (_fixture, archive) = Fixture::parse(&crc);
    assert_eq!(
        archive
            .unwrap()
            .read_into_reusing(0, &mut [0; 9], &mut archive_reader),
        Err(FormatError::Checksum)
    );
    let mut wrong_name = zip(b"123456789", true, true, b"");
    wrong_name[30] ^= 1;
    let (_fixture, archive) = Fixture::parse(&wrong_name);
    assert!(matches!(archive, Err(FormatError::InvalidRecordSize)));
    let truncated = &wrong_name[..wrong_name.len() - 10];
    let (_fixture, archive) = Fixture::parse(truncated);
    assert!(archive.is_err());
}

#[test]
fn one_loaded_inflater_streams_large_members_and_resets_after_empty_and_bad_streams() {
    let mut reader = qa_formats::archive::ArchiveReader::default();
    let mut seed = 0x434d4442u32;
    let large: Vec<_> = (0..70000)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as u8
        })
        .collect();
    for payload in [b"small".as_slice(), &large, &[], b"after empty", &large] {
        let (_fixture, archive) = Fixture::parse(&zip(payload, true, true, b""));
        let mut destination = vec![0; payload.len()];
        assert_eq!(
            archive
                .unwrap()
                .read_into_reusing(0, &mut destination, &mut reader)
                .unwrap(),
            payload.len()
        );
        assert_eq!(destination, payload);
    }
    let (_fixture, archive) = Fixture::parse(&zip(b"bad stream", true, true, b""));
    let mut archive = archive.unwrap();
    archive.entries[0].compressed_length -= 1;
    assert_eq!(
        archive.read_into_reusing(0, &mut [0; 10], &mut reader),
        Err(FormatError::Compression)
    );
    let (_fixture, archive) = Fixture::parse(&zip(b"after error", true, true, b""));
    let mut destination = [0; 11];
    archive
        .unwrap()
        .read_into_reusing(0, &mut destination, &mut reader)
        .unwrap();
    assert_eq!(&destination, b"after error");
}

#[test]
fn streamed_members_advance_once_rewind_and_reset_between_files() {
    let payload: Vec<_> = (0..40000).map(|i| ((i * 73) ^ (i >> 5)) as u8).collect();
    for deflate in [false, true] {
        let bytes = zip(&payload, deflate, false, b"");
        let (fixture, archive) = Fixture::parse(&bytes);
        let archive = archive.unwrap();
        let mut reader = qa_formats::archive::ArchiveReader::default();
        let mut out = [0; 4096];
        assert_eq!(
            archive
                .read_stream_reusing(0, 0, &mut out, &mut reader)
                .unwrap(),
            out.len()
        );
        assert_eq!(&out, &payload[..out.len()]);
        assert_eq!(reader.stream_position(), out.len() as u64);
        // Already-consumed source bytes can change without being read again.
        // A restarted whole-member decode would reject the altered prefix.
        let mut changed = bytes;
        changed[30 + b"maps/fixture.bsp".len()] ^= 1;
        qa_platform::files::write(&fixture.0, &changed).unwrap();
        let mut at = out.len();
        while at < payload.len() {
            let count = archive
                .read_stream_reusing(0, at as u64, &mut out, &mut reader)
                .unwrap();
            assert_eq!(&out[..count], &payload[at..at + count]);
            at += count;
            assert_eq!(reader.stream_position(), at as u64);
        }
        assert_eq!(
            archive
                .read_stream_reusing(0, at as u64, &mut out, &mut reader)
                .unwrap(),
            0
        );
        assert!(
            archive
                .read_stream_reusing(0, 0, &mut [0; 40000], &mut reader)
                .is_err()
        );
        let (_other, other) = Fixture::parse(&zip(&payload, deflate, false, b""));
        let other = other.unwrap();
        assert_eq!(
            other
                .read_stream_reusing(0, 1234, &mut out, &mut reader)
                .unwrap(),
            out.len()
        );
        assert_eq!(&out, &payload[1234..1234 + out.len()]);
        assert_eq!(reader.stream_position(), 1234 + out.len() as u64);
        assert_eq!(
            other
                .read_stream_reusing(0, 0, &mut out, &mut reader)
                .unwrap(),
            out.len()
        );
        assert_eq!(&out, &payload[..out.len()]);
    }
}

#[test]
fn streaming_checks_unread_tail_crc_when_reaching_eof() {
    let payload = vec![123; 20000];
    for deflate in [false, true] {
        let mut bytes = zip(&payload, deflate, false, b"");
        let at = bytes
            .windows(4)
            .position(|w| w == 0x02014b50u32.to_le_bytes())
            .unwrap();
        bytes[at + 16] ^= 1;
        bytes[14] ^= 1;
        let (_fixture, archive) = Fixture::parse(&bytes);
        let archive = archive.unwrap();
        let mut reader = qa_formats::archive::ArchiveReader::default();
        assert_eq!(
            archive
                .read_stream_reusing(0, 0, &mut [0; 1024], &mut reader)
                .unwrap(),
            1024
        );
        assert_eq!(
            archive.read_stream_reusing(0, 19000, &mut [0; 1024], &mut reader),
            Err(FormatError::Checksum)
        );
        assert!(
            archive
                .read_stream_reusing(0, 20000, &mut [], &mut reader)
                .is_err()
        );
    }
}

#[test]
fn shared_inflater_keeps_exact_archive_consumption_and_announced_length_errors() {
    let payload = b"bounded archive member";
    let mut reader = qa_formats::archive::ArchiveReader::default();
    for (compressed_delta, length_delta) in [(1i64, 0i64), (0, -1), (0, 1)] {
        let (_fixture, archive) = Fixture::parse(&zip(payload, true, false, b""));
        let mut archive = archive.unwrap();
        archive.entries[0].compressed_length =
            (archive.entries[0].compressed_length as i64 + compressed_delta) as u64;
        archive.entries[0].length = (archive.entries[0].length as i64 + length_delta) as u64;
        assert_eq!(
            archive.read_into_reusing(0, &mut [0; 64], &mut reader),
            Err(FormatError::Compression)
        );
    }
    // The retained decoder still resets for a healthy source after each failure.
    let (_fixture, archive) = Fixture::parse(&zip(payload, true, false, b""));
    let archive = archive.unwrap();
    let mut out = [0; 64];
    let count = archive
        .read_stream_reusing(0, 5, &mut out, &mut reader)
        .unwrap();
    assert_eq!(&out[..count], &payload[5..]);
    let count = archive
        .read_stream_reusing(0, 0, &mut out, &mut reader)
        .unwrap();
    assert_eq!(&out[..count], payload);
}

#[path = "support/stored_zip.rs"]
mod stored;

#[test]
fn original_q3_package_metadata_uses_directory_order_and_skips_empty_members() {
    let first = stored::stored_zip(&[(b"z.bin", b"first"), (b"empty", b""), (b"a.bin", b"second")]);
    let renamed = stored::stored_zip(&[
        (b"renamed-a", b"first"),
        (b"directory/", b""),
        (b"renamed-z", b"second"),
    ]);
    let reversed = stored::stored_zip(&[(b"a.bin", b"second"), (b"z.bin", b"first")]);
    let (_one, archive) = Fixture::parse(&first);
    let archive = archive.unwrap();
    let (_two, same) = Fixture::parse(&renamed);
    let same = same.unwrap();
    let (_three, other) = Fixture::parse(&reversed);
    let other = other.unwrap();
    let first = archive.q3_checksums().unwrap();
    assert_eq!(first.checksum(), same.q3_checksums().unwrap().checksum());
    assert_eq!(
        first.pure_checksum(0x12345678),
        same.q3_checksums().unwrap().pure_checksum(0x12345678)
    );
    assert_ne!(first.checksum(), other.q3_checksums().unwrap().checksum());
    assert_ne!(first.pure_checksum(0), first.pure_checksum(1));
    // Keyed values remain connection data; another connection cannot replace one.
    let key = first.pure_checksum(0x12345678);
    first.pure_checksum(7);
    assert_eq!(first.pure_checksum(0x12345678), key);
}

#[test]
#[ignore = "requires owned retail Q3 pak0; set QA_RETAIL_Q3_PAK"]
fn retail_q3_package_matches_original_native_wire_values() {
    let path = std::env::var_os("QA_RETAIL_Q3_PAK").expect("set QA_RETAIL_Q3_PAK");
    let archive = Archive::parse(Arc::new(
        ReadFile::open(std::path::Path::new(&path)).unwrap(),
    ))
    .unwrap();
    let native = archive.q3_checksums().unwrap();
    assert_eq!(native.checksum(), 1566731103);
    assert_eq!(native.pure_checksum(0x12345678), 3017657714);
}
