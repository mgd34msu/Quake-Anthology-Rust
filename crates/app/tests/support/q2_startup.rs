//! Authored protocol-34 world and WAL bytes; production readers own decoding.
pub fn world(texture: &[u8]) -> Vec<u8> {
    assert!(texture.len() < 32);
    let mut lumps = vec![vec![]; 19];
    lumps[0] = b"{\n\"classname\" \"worldspawn\"\n}\n\0".to_vec();
    for value in [1.0f32, 0.0, 0.0, 0.0] {
        lumps[1].extend(value.to_le_bytes());
    }
    lumps[1].extend(0i32.to_le_bytes());
    for position in [
        [0.0f32, -16.0, -16.0],
        [0.0, -16.0, 16.0],
        [0.0, 16.0, 16.0],
        [0.0, 16.0, -16.0],
    ] {
        for value in position {
            lumps[2].extend(value.to_le_bytes());
        }
    }
    lumps[4] = vec![0; 28];
    lumps[4][4..8].copy_from_slice(&(-1i32).to_le_bytes());
    lumps[4][8..12].copy_from_slice(&(-2i32).to_le_bytes());
    for (index, value) in [-512i16, -512, -512, 512, 512, 512].into_iter().enumerate() {
        lumps[4][12 + index * 2..14 + index * 2].copy_from_slice(&value.to_le_bytes());
    }
    lumps[4][26..28].copy_from_slice(&1u16.to_le_bytes());
    lumps[5] = vec![0; 76];
    for (index, value) in [0.0f32, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0]
        .into_iter()
        .enumerate()
    {
        lumps[5][index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
    lumps[5][40..40 + texture.len()].copy_from_slice(texture);
    lumps[5][72..76].copy_from_slice(&(-1i32).to_le_bytes());
    lumps[6] = vec![0; 20];
    lumps[6][8..10].copy_from_slice(&4u16.to_le_bytes());
    lumps[6][12..16].fill(255);
    lumps[6][16..20].copy_from_slice(&(-1i32).to_le_bytes());
    lumps[8] = vec![0; 56];
    lumps[8][0..4].copy_from_slice(&1i32.to_le_bytes());
    for leaf in 0..2 {
        lumps[8][leaf * 28 + 4..leaf * 28 + 6].copy_from_slice(&(-1i16).to_le_bytes());
        for (index, value) in [-512i16, -512, -512, 512, 512, 512].into_iter().enumerate() {
            let at = leaf * 28 + 8 + index * 2;
            lumps[8][at..at + 2].copy_from_slice(&value.to_le_bytes());
        }
    }
    lumps[8][50..52].copy_from_slice(&1u16.to_le_bytes());
    lumps[9] = 0u16.to_le_bytes().to_vec();
    lumps[11] = [0u16, 1, 1, 2, 2, 3, 3, 0]
        .into_iter()
        .flat_map(u16::to_le_bytes)
        .collect();
    lumps[12] = [0i32, 1, 2, 3]
        .into_iter()
        .flat_map(i32::to_le_bytes)
        .collect();
    lumps[13] = vec![0; 48];
    for (index, value) in [-512.0f32, -512.0, -512.0, 512.0, 512.0, 512.0]
        .into_iter()
        .enumerate()
    {
        lumps[13][index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
    lumps[13][44..48].copy_from_slice(&1u32.to_le_bytes());
    lumps[17] = vec![0; 8];
    let mut bytes = vec![0; 8 + lumps.len() * 8];
    bytes[..4].copy_from_slice(b"IBSP");
    bytes[4..8].copy_from_slice(&38u32.to_le_bytes());
    for (index, lump) in lumps.iter().enumerate() {
        let at = 8 + index * 8;
        let offset = bytes.len() as u32;
        bytes[at..at + 4].copy_from_slice(&offset.to_le_bytes());
        bytes[at + 4..at + 8].copy_from_slice(&(lump.len() as u32).to_le_bytes());
        bytes.extend(lump);
    }
    bytes
}

pub fn wal(index: u8) -> Vec<u8> {
    let mut bytes = vec![0; 100];
    bytes[..4].copy_from_slice(b"wall");
    bytes[32..36].copy_from_slice(&16u32.to_le_bytes());
    bytes[36..40].copy_from_slice(&16u32.to_le_bytes());
    for mip in 0..4 {
        let offset = bytes.len() as u32;
        bytes[40 + mip * 4..44 + mip * 4].copy_from_slice(&offset.to_le_bytes());
        bytes.extend(std::iter::repeat_n(index, (16 >> mip) * (16 >> mip)));
    }
    bytes
}
