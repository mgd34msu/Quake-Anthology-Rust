use qa_core::primitives::Vec3;

pub struct SourceBrush {
    pub mins: [f32; 3],
    pub maxs: [f32; 3],
    pub contents: i16,
    pub planes: Vec<[f32; 4]>,
}
impl SourceBrush {
    pub fn cube(x: f32, contents: i16) -> Self {
        Self {
            mins: [x - 1.0, -1.0, -1.0],
            maxs: [x + 1.0, 1.0, 1.0],
            contents,
            planes: vec![],
        }
    }
}
fn put(bytes: &mut [u8], at: usize, value: i32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
pub fn chunk(model: u32, brushes: &[SourceBrush]) -> Vec<u8> {
    let mut output = Vec::new();
    for word in [
        1,
        model,
        brushes.len() as u32,
        brushes.iter().map(|brush| brush.planes.len() as u32).sum(),
    ] {
        output.extend(word.to_le_bytes());
    }
    for brush in brushes {
        for value in brush.mins.into_iter().chain(brush.maxs) {
            output.extend(value.to_le_bytes());
        }
        output.extend(brush.contents.to_le_bytes());
        output.extend((brush.planes.len() as u16).to_le_bytes());
        for plane in &brush.planes {
            for value in plane {
                output.extend(value.to_le_bytes());
            }
        }
    }
    output
}
pub fn bsp(
    version: u32,
    model_count: usize,
    omitted_clips: bool,
    extension: Option<&[u8]>,
) -> Vec<u8> {
    bsp_with_lumps(version, model_count, omitted_clips, extension, |_| {})
}
pub fn bsp_with_lumps(
    version: u32,
    model_count: usize,
    omitted_clips: bool,
    extension: Option<&[u8]>,
    edit: impl FnOnce(&mut [Vec<u8>]),
) -> Vec<u8> {
    let wide = matches!(version, 0x32505342 | 0x42535032);
    let floats = version == 0x32505342;
    let mut lumps = vec![Vec::new(); 15];
    lumps[1] = vec![0; 20];
    put(&mut lumps[1], 0, 1.0f32.to_bits() as i32);
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
        put(&mut lumps[5], 4, -2);
        put(&mut lumps[5], 8, -1);
    } else {
        lumps[5][4..6].copy_from_slice(&(-2i16).to_le_bytes());
        lumps[5][6..8].copy_from_slice(&(-1i16).to_le_bytes());
    }
    let stride = if floats {
        44
    } else if wide {
        32
    } else {
        28
    };
    lumps[10] = vec![0; stride * 2];
    for leaf in 0..2 {
        put(
            &mut lumps[10],
            leaf * stride,
            if leaf == 0 { -2 } else { -1 },
        );
        put(&mut lumps[10], leaf * stride + 4, -1);
    }
    lumps[14] = vec![0; 64 * model_count];
    for model in 0..model_count {
        for (axis, value) in Vec3([-512.0; 3])
            .0
            .into_iter()
            .chain(Vec3([512.0; 3]).0)
            .enumerate()
        {
            put(
                &mut lumps[14],
                model * 64 + axis * 4,
                value.to_bits() as i32,
            );
        }
        for at in [40, 44, 48] {
            put(
                &mut lumps[14],
                model * 64 + at,
                if omitted_clips { 0 } else { -2 },
            );
        }
        put(&mut lumps[14], model * 64 + 52, 1);
    }
    edit(&mut lumps);
    let mut bytes = vec![0; 124];
    put(&mut bytes, 0, version as i32);
    for (index, lump) in lumps.iter().enumerate() {
        let at = 4 + index * 8;
        let offset = bytes.len() as i32;
        put(&mut bytes, at, offset);
        put(&mut bytes, at + 4, lump.len() as i32);
        bytes.extend(lump);
    }
    if let Some(extension) = extension {
        bytes.resize((bytes.len() + 3) & !3, 0);
        let payload = bytes.len() + 40;
        bytes.extend_from_slice(b"BSPX");
        bytes.extend(1u32.to_le_bytes());
        let mut name = [0; 24];
        name[..9].copy_from_slice(b"BRUSHLIST");
        bytes.extend(name);
        bytes.extend((payload as u32).to_le_bytes());
        bytes.extend((extension.len() as u32).to_le_bytes());
        bytes.extend(extension);
    }
    bytes
}
