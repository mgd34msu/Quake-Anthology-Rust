pub fn stored_zip(entries: &[(&[u8], &[u8])]) -> Vec<u8> {
    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = u32::MAX;
        for &byte in bytes {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                let mask = 0u32.wrapping_sub(crc & 1);
                crc = (crc >> 1) ^ (0xedb8_8320 & mask);
            }
        }
        !crc
    }

    let mut bytes = Vec::new();
    let mut directory = Vec::new();
    for &(name, data) in entries {
        let offset = u32::try_from(bytes.len()).unwrap();
        let name_length = u16::try_from(name.len()).unwrap();
        let data_length = u32::try_from(data.len()).unwrap();
        let crc = crc32(data);
        let mut local = [0; 30];
        local[..4].copy_from_slice(&0x0403_4b50u32.to_le_bytes());
        local[4..6].copy_from_slice(&20u16.to_le_bytes());
        local[14..18].copy_from_slice(&crc.to_le_bytes());
        local[18..22].copy_from_slice(&data_length.to_le_bytes());
        local[22..26].copy_from_slice(&data_length.to_le_bytes());
        local[26..28].copy_from_slice(&name_length.to_le_bytes());
        bytes.extend(local);
        bytes.extend(name);
        bytes.extend(data);

        let mut central = [0; 46];
        central[..4].copy_from_slice(&0x0201_4b50u32.to_le_bytes());
        central[4..6].copy_from_slice(&20u16.to_le_bytes());
        central[6..8].copy_from_slice(&20u16.to_le_bytes());
        central[16..20].copy_from_slice(&crc.to_le_bytes());
        central[20..24].copy_from_slice(&data_length.to_le_bytes());
        central[24..28].copy_from_slice(&data_length.to_le_bytes());
        central[28..30].copy_from_slice(&name_length.to_le_bytes());
        central[42..46].copy_from_slice(&offset.to_le_bytes());
        directory.extend(central);
        directory.extend(name);
    }
    let directory_offset = u32::try_from(bytes.len()).unwrap();
    let directory_length = u32::try_from(directory.len()).unwrap();
    bytes.extend(directory);
    let count = u16::try_from(entries.len()).unwrap();
    let mut end = [0; 22];
    end[..4].copy_from_slice(&0x0605_4b50u32.to_le_bytes());
    end[8..10].copy_from_slice(&count.to_le_bytes());
    end[10..12].copy_from_slice(&count.to_le_bytes());
    end[12..16].copy_from_slice(&directory_length.to_le_bytes());
    end[16..20].copy_from_slice(&directory_offset.to_le_bytes());
    bytes.extend(end);
    bytes
}
