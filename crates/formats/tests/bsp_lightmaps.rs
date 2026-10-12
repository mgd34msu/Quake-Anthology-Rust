use qa_formats::bsp::{Bsp, DecoupledLightmap, Map};

fn row(width: u16, height: u16, offset: u32, projection: [[f32; 4]; 2]) -> Vec<u8> {
    let mut bytes = width.to_le_bytes().to_vec();
    bytes.extend(height.to_le_bytes());
    bytes.extend(offset.to_le_bytes());
    for value in projection.into_iter().flatten() {
        bytes.extend(value.to_le_bytes());
    }
    bytes
}

const PROJECTION: [[f32; 4]; 2] = [[0.0, 0.125, 0.0, -3.5], [0.0, 0.0, -0.25, 9.0]];

fn fixture(version: u32, styles: &[[u8; 4]], lighting: &[u8], extension: Option<&[u8]>) -> Vec<u8> {
    let q2 = version == 38;
    let count = if q2 { 19 } else { 15 };
    let mut lumps = vec![Vec::new(); count];
    let (vertices, nodes, info, faces, light, leaves, edges, surface_edges, models) = if q2 {
        (2, 4, 5, 6, 7, 8, 11, 12, 13)
    } else {
        (3, 5, 6, 7, 8, 10, 12, 13, 14)
    };
    let plane: [f32; 4] = [1.0, 0.0, 0.0, 10.0];
    for value in plane {
        lumps[1].extend(value.to_le_bytes());
    }
    lumps[1].extend(0i32.to_le_bytes());
    for point in [[10.0f32, 0.0, 0.0], [10.0, 32.0, 0.0], [10.0, 0.0, 32.0]] {
        for value in point {
            lumps[vertices].extend(value.to_le_bytes());
        }
    }
    lumps[nodes] = vec![0; if q2 { 28 } else { 24 }];
    if q2 {
        lumps[nodes][4..12].copy_from_slice(&[u8::MAX; 8]);
    } else {
        lumps[nodes][4..8].copy_from_slice(&[u8::MAX; 4]);
    }
    lumps[leaves] = vec![0; 28];
    lumps[leaves][..4].copy_from_slice(&(if q2 { 1i32 } else { -2 }).to_le_bytes());
    if q2 {
        lumps[leaves][4..6].copy_from_slice(&u16::MAX.to_le_bytes());
    } else {
        lumps[leaves][4..8].copy_from_slice(&(-1i32).to_le_bytes());
    }
    lumps[info] = vec![0; if q2 { 76 } else { 40 }];
    for (at, value) in [(4, 1.0f32), (24, 1.0)] {
        lumps[info][at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    if q2 {
        lumps[info][72..76].copy_from_slice(&(-1i32).to_le_bytes());
    }
    for style in styles {
        let mut face = vec![0; 20];
        face[8..10].copy_from_slice(&3u16.to_le_bytes());
        face[12..16].copy_from_slice(style);
        face[16..20].copy_from_slice(&(-1i32).to_le_bytes());
        lumps[faces].extend(face);
    }
    lumps[light].extend_from_slice(lighting);
    for edge in [[0u16, 1], [1, 2], [2, 0]] {
        for value in edge {
            lumps[edges].extend(value.to_le_bytes());
        }
    }
    for edge in 0..3i32 {
        lumps[surface_edges].extend(edge.to_le_bytes());
    }
    lumps[models] = vec![0; if q2 { 48 } else { 64 }];
    if !q2 {
        for at in [40, 44, 48] {
            lumps[models][at..at + 4].copy_from_slice(&(-2i32).to_le_bytes());
        }
    }
    let at = if q2 { 44 } else { 60 };
    lumps[models][at..at + 4].copy_from_slice(&(styles.len() as u32).to_le_bytes());
    if q2 {
        lumps[17] = vec![0; 8];
    }
    let mut magic = if q2 { b"IBSP".to_vec() } else { vec![] };
    magic.extend(version.to_le_bytes());
    let header = magic.len() + count * 8;
    let mut bytes = vec![0; header];
    bytes[..magic.len()].copy_from_slice(&magic);
    for (index, lump) in lumps.iter().enumerate() {
        let at = magic.len() + index * 8;
        let offset = bytes.len() as u32;
        bytes[at..at + 4].copy_from_slice(&offset.to_le_bytes());
        bytes[at + 4..at + 8].copy_from_slice(&(lump.len() as u32).to_le_bytes());
        bytes.extend_from_slice(lump);
    }
    if let Some(extension) = extension {
        bytes.resize((bytes.len() + 3) & !3, 0);
        let offset = bytes.len() + 40;
        bytes.extend_from_slice(b"BSPX");
        bytes.extend(1u32.to_le_bytes());
        let mut name = [0; 24];
        name[..12].copy_from_slice(b"DECOUPLED_LM");
        bytes.extend(name);
        bytes.extend((offset as u32).to_le_bytes());
        bytes.extend((extension.len() as u32).to_le_bytes());
        bytes.extend_from_slice(extension);
    }
    bytes
}

#[test]
fn typed_projection_overrides_classic_unlit_offset_and_preserves_style_major_rgb() {
    let lighting: Vec<_> = (0..38u8).collect();
    let record = row(3, 2, 2, PROJECTION);
    let bytes = fixture(38, &[[0, 7, 255, 255]], &lighting, Some(&record));
    let map = Map::parse(&bytes).unwrap();
    assert_eq!(map.faces[0].lighting_offset, -1);
    assert_eq!(map.faces[0].styles, [0, 7, 255, 255]);
    assert_eq!(
        map.decoupled_lightmaps,
        [Some(DecoupledLightmap {
            width: 3,
            height: 2,
            offset: Some(2),
            projection: PROJECTION,
        })]
    );
    assert_eq!(map.bsp.bytes(qa_formats::bsp::Lump::Lighting), lighting);
    assert_eq!(map.extensions[0].bytes, record);
}

#[test]
fn sentinel_and_bad_complete_sample_spans_retain_dimensions_but_are_unlit() {
    for (width, height, offset, styles) in [
        (0, 0, u32::MAX, [0, 255, 255, 255]),
        (3, 2, 0, [0, 7, 255, 255]), // Last byte of the second style is missing.
        (3, 2, 35, [0, 255, 255, 255]),
        (3, 2, u32::MAX - 1, [0, 255, 255, 255]),
        (u16::MAX, u16::MAX, 0, [0, 1, 2, 3]),
        (0, 2, 0, [0, 255, 255, 255]),
        (3, 2, 0, [255; 4]),
        (3, 2, 0, [0, 255, 7, 255]),
    ] {
        let record = row(width, height, offset, PROJECTION);
        let bytes = fixture(38, &[styles], &[31; 35], Some(&record));
        let map = Map::parse(&bytes).unwrap();
        assert_eq!(
            map.decoupled_lightmaps,
            [Some(DecoupledLightmap {
                width: u32::from(width),
                height: u32::from(height),
                offset: None,
                projection: PROJECTION,
            })]
        );
    }
}

#[test]
fn invalid_whole_lumps_and_nonfinite_projection_fall_back_without_losing_the_map() {
    for (faces, record) in [
        (1, vec![]),
        (1, vec![0; 39]),
        (1, vec![0; 41]),
        (2, row(3, 2, 0, PROJECTION)),
    ] {
        let bytes = fixture(
            38,
            &vec![[0, 255, 255, 255]; faces],
            &[31; 36],
            Some(&record),
        );
        let map = Map::parse(&bytes).unwrap();
        assert_eq!(map.decoupled_lightmaps, vec![None; faces]);
        assert_eq!(map.extensions[0].bytes, record);
    }
    for malformed in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let mut projection = PROJECTION;
        projection[1][3] = malformed;
        let record = row(3, 2, 0, projection);
        let bytes = fixture(38, &[[0, 255, 255, 255]], &[31; 18], Some(&record));
        assert_eq!(Map::parse(&bytes).unwrap().decoupled_lightmaps, [None]);
    }
}

#[test]
fn records_stay_face_indexed_and_valid_extra_rows_are_ignored() {
    let mut records = row(3, 2, 2, PROJECTION);
    records.extend(row(1, 1, u32::MAX, [[0.0; 4]; 2]));
    records.extend(row(4, 4, 0, [[f32::NAN; 4]; 2]));
    let bytes = fixture(38, &[[0, 255, 255, 255]; 2], &[31; 20], Some(&records));
    let map = Map::parse(&bytes).unwrap();
    assert_eq!(map.decoupled_lightmaps.len(), 2);
    assert_eq!(map.decoupled_lightmaps[0].unwrap().offset, Some(2));
    assert_eq!(map.decoupled_lightmaps[1].unwrap().offset, None);
    let bytes = fixture(38, &[[0, 255, 255, 255]], &[31; 20], None);
    assert_eq!(Map::parse(&bytes).unwrap().decoupled_lightmaps, [None]);
}

#[test]
fn admitted_legacy_sample_widths_bound_the_entire_span_in_bytes() {
    for (version, stride) in [(29, 1), (30, 3), (0x51363420, 2), (38, 3)] {
        let record = row(3, 2, 2, PROJECTION);
        let lighting = vec![31; 2 + 12 * stride];
        let bytes = fixture(version, &[[0, 7, 255, 255]], &lighting, Some(&record));
        assert_eq!(
            Map::parse(&bytes).unwrap().decoupled_lightmaps[0]
                .unwrap()
                .offset,
            Some(2)
        );
        let bytes = fixture(
            version,
            &[[0, 7, 255, 255]],
            &lighting[..lighting.len() - 1],
            Some(&record),
        );
        assert_eq!(
            Map::parse(&bytes).unwrap().decoupled_lightmaps[0]
                .unwrap()
                .offset,
            None
        );
        assert_eq!(
            Bsp::parse(&bytes)
                .unwrap()
                .bytes(qa_formats::bsp::Lump::Lighting)
                .len(),
            lighting.len() - 1
        );
    }
}
