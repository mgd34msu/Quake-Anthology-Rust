use qa_formats::{Bsp, BspFormat, FormatError, ModelHeader, Pak};

#[test]
fn pack_uses_first_duplicate_and_borrows_member_bytes() {
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
    let pak = Pak::parse(&bytes).unwrap();
    let member = pak.find(b"dup").unwrap();
    assert_eq!(member, b"AB");
    assert_eq!(member.as_ptr(), bytes[12..].as_ptr());
    assert!(pak.find(b"missing").is_none());
    bytes[72..76].copy_from_slice(&(-1i32).to_le_bytes());
    assert!(matches!(Pak::parse(&bytes), Err(FormatError::InvalidRange)));
}

#[test]
fn malformed_bsp_records_are_rejected_at_admission() {
    let mut bytes = vec![0; 124];
    bytes[..4].copy_from_slice(&29u32.to_le_bytes());
    assert_eq!(Bsp::parse(&bytes).unwrap().record_count(1), Some(0));
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
    let bytes = std::fs::read(path).unwrap();
    let pak = Pak::parse(&bytes).unwrap();
    let bsp = Bsp::parse(pak.find(b"maps/e1m1.bsp").unwrap()).unwrap();
    assert_eq!(bsp.format, BspFormat::Quake);
    assert_eq!(bsp.record_count(1), Some(1810));
    assert_eq!(bsp.record_count(3), Some(7358));
    assert_eq!(bsp.record_count(7), Some(5516));
    assert_eq!(bsp.record_count(14), Some(58));
    let mdl = ModelHeader::parse(pak.find(b"progs/player.mdl").unwrap()).unwrap();
    assert_eq!((mdl.vertices, mdl.triangles, mdl.frames), (212, 408, 143));
}
