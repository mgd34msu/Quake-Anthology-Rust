use qa_core::primitives::{Axis, Vec3};
use qa_formats::{FormatError, bsp::Map};

fn append(lumps: &[Vec<u8>], magic: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0; magic.len() + lumps.len() * 8];
    bytes[..magic.len()].copy_from_slice(magic);
    for (index, lump) in lumps.iter().enumerate() {
        let at = magic.len() + index * 8;
        let offset = bytes.len() as u32;
        bytes[at..at + 4].copy_from_slice(&offset.to_le_bytes());
        bytes[at + 4..at + 8].copy_from_slice(&(lump.len() as u32).to_le_bytes());
        bytes.extend(lump);
    }
    bytes
}
fn put(bytes: &mut [u8], at: usize, value: i32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
fn fixture(version: u32) -> Vec<u8> {
    if version == 38 || version == 0x50534251 {
        let wide = version == 0x50534251;
        let mut lumps = vec![Vec::new(); 19];
        lumps[1] = vec![0; 20];
        put(&mut lumps[1], 0, 1.0f32.to_bits() as i32);
        put(&mut lumps[1], 12, 10.0f32.to_bits() as i32);
        lumps[4] = vec![0; if wide { 44 } else { 28 }];
        put(&mut lumps[4], 4, -1);
        put(&mut lumps[4], 8, -1);
        lumps[8] = vec![0; if wide { 52 } else { 28 }];
        put(&mut lumps[8], 0, 1);
        if wide {
            put(&mut lumps[8], 4, -1);
        } else {
            lumps[8][4..6].copy_from_slice(&u16::MAX.to_le_bytes());
        }
        lumps[13] = vec![0; 48];
        lumps[17] = vec![0; 8];
        let mut magic = if wide {
            b"QBSP".to_vec()
        } else {
            b"IBSP".to_vec()
        };
        magic.extend(38u32.to_le_bytes());
        return append(&lumps, &magic);
    }
    if matches!(version, 44 | 46 | 47) {
        let test = version == 44;
        let mut lumps = vec![
            Vec::new();
            if test {
                15
            } else if version == 47 {
                18
            } else {
                17
            }
        ];
        let (plane, node, leaf, model) = if test { (1, 2, 3, 6) } else { (2, 3, 4, 7) };
        lumps[plane] = vec![0; if test { 20 } else { 16 }];
        put(&mut lumps[plane], 0, 1.0f32.to_bits() as i32);
        put(&mut lumps[plane], 12, 10.0f32.to_bits() as i32);
        lumps[node] = vec![0; 36];
        put(&mut lumps[node], 4, -1);
        put(&mut lumps[node], 8, -1);
        lumps[leaf] = vec![0; 48];
        put(&mut lumps[leaf], 0, -1);
        put(&mut lumps[leaf], 4, -1);
        lumps[model] = vec![0; if test { 48 } else { 40 }];
        let mut magic = b"IBSP".to_vec();
        magic.extend(version.to_le_bytes());
        return append(&lumps, &magic);
    }
    let wide = matches!(version, 0x32505342 | 0x42535032);
    let floats = version == 0x32505342;
    let mut lumps = vec![Vec::new(); 15];
    lumps[1] = vec![0; 20];
    put(&mut lumps[1], 0, 1.0f32.to_bits() as i32);
    put(&mut lumps[1], 12, 10.0f32.to_bits() as i32);
    lumps[5] = vec![
        0;
        if floats {
            44
        } else if wide {
            32
        } else {
            24
        }
    ];
    if wide {
        put(&mut lumps[5], 4, -1);
        put(&mut lumps[5], 8, -1);
    } else {
        lumps[5][4..8].fill(255);
    }
    lumps[10] = vec![
        0;
        if floats {
            44
        } else if wide {
            32
        } else {
            28
        }
    ];
    put(&mut lumps[10], 0, -2);
    put(&mut lumps[10], 4, -1);
    lumps[14] = vec![0; 64];
    for at in [40, 44, 48] {
        put(&mut lumps[14], at, -2);
    }
    append(&lumps, &version.to_le_bytes())
}

const VERSIONS: &[u32] = &[
    29, 30, 0x32505342, 0x42535032, 0x51363420, 38, 0x50534251, 44, 46, 47,
];

#[test]
fn native_layouts_convert_into_the_same_flat_tree_and_core_plane() {
    for &version in VERSIONS {
        let bytes = fixture(version);
        let map = Map::parse(&bytes).unwrap();
        assert_eq!(map.planes.len(), 1);
        assert_eq!(map.nodes[0].children, [-1, -1]);
        assert_eq!(map.models.len(), 1);
        assert_eq!(map.planes[0].normal, Vec3([1.0, 0.0, 0.0]));
        assert_eq!(map.planes[0].axis, Some(Axis::X));
        assert_eq!(map.planes[0].signed_distance(Vec3([12.0, 4.0, 2.0])), 2.0);
        assert_eq!(map.models[0].membership_from_tree, version == 44);
    }
}

#[test]
fn admission_rejects_cycles_indices_and_nonfinite_geometry() {
    let mut bytes = fixture(29);
    let node = u32::from_le_bytes(bytes[44..48].try_into().unwrap()) as usize;
    bytes[node + 4..node + 6].copy_from_slice(&0u16.to_le_bytes());
    assert!(matches!(Map::parse(&bytes), Err(FormatError::Cycle)));
    put(&mut bytes, node, 100);
    assert!(matches!(
        Map::parse(&bytes),
        Err(FormatError::InvalidReference("node plane", 0))
    ));
    let mut bytes = fixture(46);
    let plane = u32::from_le_bytes(bytes[24..28].try_into().unwrap()) as usize;
    put(&mut bytes, plane, f32::NAN.to_bits() as i32);
    assert!(matches!(Map::parse(&bytes), Err(FormatError::InvalidValue)));
}

#[test]
fn malformed_lumps_and_seeded_mutations_return_without_panicking() {
    let mut seed = 0x42535046u32;
    for &version in VERSIONS {
        let original = fixture(version);
        for length in 0..original.len() {
            let _ = Map::parse(&original[..length]);
        }
        for _ in 0..1000 {
            let mut bytes = original.clone();
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let index = seed as usize % bytes.len();
            bytes[index] ^= 1 << (seed % 8);
            let _ = Map::parse(&bytes);
        }
    }
}

#[test]
fn q2_truncated_entity_tail_and_unused_pop_are_bounded_with_diagnostics() {
    for lump in [0usize, 16] {
        let mut bytes = fixture(38);
        let offset = bytes.len() as i32;
        put(&mut bytes, 8 + lump * 8, offset);
        put(&mut bytes, 12 + lump * 8, 256);
        bytes.extend(b"{\n}\n\0");
        let map = Map::parse(&bytes).unwrap();
        assert_eq!(map.bsp.entity_tail_clamped, lump == 0);
        assert_eq!(map.bsp.pop_tail_clamped, lump == 16);
        assert_eq!(map.bsp.lump(lump).unwrap(), b"{\n}\n\0");
    }
    let mut bytes = fixture(38);
    let offset = bytes.len() as i32;
    put(&mut bytes, 8 + 7 * 8, offset);
    put(&mut bytes, 12 + 7 * 8, 256);
    assert!(matches!(Map::parse(&bytes), Err(FormatError::InvalidRange)));
}

#[test]
fn q3_retail_flare_empty_fog_and_external_lightmaps_preserve_raw_values() {
    use qa_formats::{Bsp, bsp::LightmapSource};
    let original = fixture(46);
    let bsp = Bsp::parse(&original).unwrap();
    let mut lumps: Vec<_> = (0..17).map(|i| bsp.lump(i).unwrap().to_vec()).collect();
    lumps[1] = vec![0; 72];
    lumps[13] = vec![0; 104];
    put(&mut lumps[13], 8, 4);
    let bytes = append(&lumps, &original[..8]);
    let map = Map::parse(&bytes).unwrap();
    assert_eq!(map.surfaces[0].fog, 0);
    assert_eq!(
        map.surface_lightmap(&map.surfaces[0]),
        LightmapSource::Vertex
    );
    put(&mut lumps[13], 8, 1);
    put(&mut lumps[13], 4, -1);
    put(&mut lumps[13], 28, 1);
    let bytes = append(&lumps, &original[..8]);
    let map = Map::parse(&bytes).unwrap();
    assert_eq!(map.surfaces[0].lightmap, 1);
    assert_eq!(
        map.surface_lightmap(&map.surfaces[0]),
        LightmapSource::External(1)
    );
    put(&mut lumps[13], 4, 0);
    let invalid = append(&lumps, &original[..8]);
    assert!(matches!(
        Map::parse(&invalid),
        Err(FormatError::InvalidReference("surface fog", 0))
    ));
}
