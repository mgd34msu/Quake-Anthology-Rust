use qa_formats::{
    FormatError,
    archive::{Archive, ArchiveKind},
};
use std::{
    fs::File,
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
        std::fs::write(&path, bytes).unwrap();
        let archive = Archive::parse(Arc::new(File::open(&path).unwrap()));
        (Self(path), archive)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_file(&self.0).unwrap();
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
    for deflate in [false, true] {
        for descriptor in [false, true] {
            let bytes = zip(b"123456789", deflate, descriptor, b"stub prefix");
            let (_fixture, archive) = Fixture::parse(&bytes);
            let archive = archive.unwrap();
            assert_eq!(archive.kind, ArchiveKind::Zip);
            assert_eq!(archive.entries[0].crc32, Some(0xcbf43926));
            let mut out = [0; 9];
            assert_eq!(archive.read_into(0, &mut out).unwrap(), 9);
            assert_eq!(&out, b"123456789");
            assert_eq!(
                archive.read_into(0, &mut [0; 8]),
                Err(FormatError::InvalidRange)
            );
        }
    }
}

#[test]
fn corrupt_crc_truncation_and_local_name_disagreement_are_scoped_errors() {
    let mut crc = zip(b"123456789", false, false, b"");
    crc[30 + b"maps/fixture.bsp".len()] ^= 1;
    let (_fixture, archive) = Fixture::parse(&crc);
    assert_eq!(
        archive.unwrap().read_into(0, &mut [0; 9]),
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
