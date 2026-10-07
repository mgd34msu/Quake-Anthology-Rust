use qa_formats::{Bsp, BspFormat, FormatError, archive::Archive, model::Model};
use std::{
    fs::File,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture(std::path::PathBuf);
impl Fixture {
    fn write(bytes: &[u8]) -> Self {
        let path = std::env::temp_dir().join(format!(
            "qa-rust-pak-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, bytes).unwrap();
        Self(path)
    }
    fn parse(&self) -> Result<Archive, FormatError> {
        Archive::parse(Arc::new(File::open(&self.0).unwrap()))
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_file(&self.0).unwrap();
    }
}

#[test]
fn pack_uses_first_duplicate_and_caller_member_buffer() {
    let mut bytes = vec![0; 144];
    bytes[..4].copy_from_slice(b"PACK");
    bytes[4..8].copy_from_slice(&16u32.to_le_bytes());
    bytes[8..12].copy_from_slice(&128u32.to_le_bytes());
    bytes[12..16].copy_from_slice(b"ABCD");
    for (offset, payload) in [(16, 12u32), (80, 14u32)] {
        bytes[offset..offset + 3].copy_from_slice(b"dup");
        bytes[offset + 56..offset + 60].copy_from_slice(&payload.to_le_bytes());
        bytes[offset + 60..offset + 64].copy_from_slice(&2u32.to_le_bytes());
    }
    let fixture = Fixture::write(&bytes);
    let pak = fixture.parse().unwrap();
    let mut member = [0; 2];
    assert_eq!(
        pak.read_into(pak.find(b"dup").unwrap(), &mut member)
            .unwrap(),
        2
    );
    assert_eq!(&member, b"AB");
    assert!(pak.find(b"missing").is_none());
    bytes[72..76].copy_from_slice(&(-1i32).to_le_bytes());
    let invalid = Fixture::write(&bytes);
    assert!(matches!(invalid.parse(), Err(FormatError::InvalidRange)));
}

#[test]
fn malformed_bsp_records_are_rejected_at_admission() {
    let mut bytes = vec![0; 124];
    bytes[..4].copy_from_slice(&29u32.to_le_bytes());
    assert_eq!(Bsp::parse(&bytes).unwrap().record_count(1), Some(0));
    bytes.push(0);
    bytes[12..16].copy_from_slice(&124u32.to_le_bytes());
    bytes[16..20].copy_from_slice(&1u32.to_le_bytes());
    assert!(matches!(
        Bsp::parse(&bytes),
        Err(FormatError::InvalidRecordSize)
    ));
    bytes[12..16].copy_from_slice(&200u32.to_le_bytes());
    assert!(matches!(Bsp::parse(&bytes), Err(FormatError::InvalidRange)));
}

#[test]
#[ignore = "requires an owned retail PAK; set QA_RETAIL_PAK and use --include-ignored"]
fn retail_e1m1_lumps_and_player_frames_match_observed_data() {
    let path = std::env::var_os("QA_RETAIL_PAK").expect("QA_RETAIL_PAK");
    let pak = Archive::parse(Arc::new(File::open(path).unwrap())).unwrap();
    let load = |name: &[u8]| {
        let entry = pak.find(name).unwrap();
        let mut bytes = vec![0; pak.entries[entry].length as usize];
        pak.read_into(entry, &mut bytes).unwrap();
        bytes
    };
    let map = load(b"maps/e1m1.bsp");
    let bsp = Bsp::parse(&map).unwrap();
    assert_eq!(bsp.format, BspFormat::Quake);
    assert_eq!(bsp.record_count(1), Some(1810));
    assert_eq!(bsp.record_count(3), Some(7358));
    assert_eq!(bsp.record_count(7), Some(5516));
    assert_eq!(bsp.record_count(14), Some(58));
    let model = load(b"progs/player.mdl");
    let mdl = Model::parse(&model).unwrap();
    assert_eq!(
        (
            mdl.meshes[0].vertices_per_frame,
            mdl.meshes[0].triangles.len(),
            mdl.frames.len()
        ),
        (212, 408, 143)
    );
}
