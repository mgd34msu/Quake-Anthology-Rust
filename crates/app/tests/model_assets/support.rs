use qa_content::vfs::Vfs;
use qa_render::surface_cache::PaletteLighting;
use qa_render::{Assets, PaletteId};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

pub struct Fixture {
    pub vfs: Vfs,
    root: std::path::PathBuf,
}
impl Fixture {
    pub fn new(entries: &[(&str, Vec<u8>)]) -> Self {
        let root = std::env::temp_dir().join(format!(
            "qa-rust-model-assets-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        qa_platform::files::create_dir_all(&root).unwrap();
        for (path, bytes) in entries {
            let path = root.join(path);
            qa_platform::files::create_dir_all(path.parent().unwrap()).unwrap();
            qa_platform::files::write(path, bytes).unwrap();
        }
        let mut vfs = Vfs::default();
        vfs.mount_directory(&root, 0).unwrap();
        Self { vfs, root }
    }
    pub fn models() -> Self {
        Self::new(&[
            ("models/test/tris.md2", md2()),
            ("models/test/skin0.pcx", pcx(11)),
            ("models/test/skin1.pcx", pcx(22)),
            ("progs/test.mdl", mdl()),
        ])
    }
    pub fn surfaces() -> Self {
        Self::with_surfaces(md3_surfaces())
    }
    pub fn with_surfaces(model: Vec<u8>) -> Self {
        Self::new(&[
            ("models/test.md3", model),
            ("textures/body.tga", tga(2, 2, &[[90, 20, 10, 255]; 4])),
            ("textures/alternate.tga", tga(2, 2, &[[10, 90, 20, 255]; 4])),
            ("textures/glow.tga", tga(2, 2, &[[20, 10, 90, 255]; 4])),
            ("scripts/test.shader", b"textures/glow { cull none { map textures/glow.tga rgbGen wave sin 0.5 0.5 0 1 } { map textures/glow.tga blendFunc add rgbGen identity } }".to_vec()),
        ])
    }
    pub fn remove(&self, path: &str) {
        qa_platform::files::remove_file(self.root.join(path)).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        qa_platform::files::remove_dir_all(&self.root).unwrap();
    }
}

pub fn palette(assets: &mut Assets) -> PaletteId {
    shifted_palette(assets, 0)
}
pub fn shifted_palette(assets: &mut Assets, shift: u8) -> PaletteId {
    let rgb: Vec<_> = (0..=255u8)
        .flat_map(|index| {
            let value = index.wrapping_add(shift);
            [value, 255 - value, value / 2]
        })
        .collect();
    let shades: Vec<_> = (0..64u8)
        .flat_map(|shade| (0..=255u8).map(move |index| index.wrapping_add(shade)))
        .collect();
    assets
        .register_palette(PaletteLighting::load(&rgb, &shades, None, 224).unwrap())
        .unwrap()
}

fn name(bytes: &mut Vec<u8>, name: &[u8], width: usize) {
    let first = bytes.len();
    bytes.extend(name);
    bytes.resize(first + width, 0);
}
fn integer(bytes: &mut Vec<u8>, value: i32) {
    bytes.extend(value.to_le_bytes());
}
fn vector(bytes: &mut Vec<u8>, value: [f32; 3]) {
    for value in value {
        bytes.extend(value.to_le_bytes());
    }
}

/// Two frames with separate vertex/UV indices and two external skins.
pub fn md2() -> Vec<u8> {
    let skin_at = 68;
    let uv_at = skin_at + 128;
    let triangle_at = uv_at + 16;
    let frame_at = triangle_at + 24;
    let stride = 40 + 4 * 4;
    let command_at = frame_at + stride * 2;
    let mut bytes = b"IDP2".to_vec();
    for value in [
        8,
        8,
        4,
        stride,
        2,
        4,
        4,
        2,
        0,
        2,
        skin_at,
        uv_at,
        triangle_at,
        frame_at,
        command_at,
        command_at,
    ] {
        integer(&mut bytes, value);
    }
    name(&mut bytes, b"models/test/skin0.pcx", 64);
    name(&mut bytes, b"models/test/skin1.pcx", 64);
    for [s, t] in [[0i16, 0], [7, 0], [7, 3], [0, 3]] {
        bytes.extend(s.to_le_bytes());
        bytes.extend(t.to_le_bytes());
    }
    for (vertices, coordinates) in [([0i16, 1, 2], [0i16, 1, 2]), ([0, 2, 3], [3, 2, 1])] {
        for index in vertices.into_iter().chain(coordinates) {
            bytes.extend(index.to_le_bytes());
        }
    }
    for (scale, translation, frame) in [
        ([1.0, 2.0, 3.0], [10.0, 20.0, 30.0], 0u8),
        ([0.5, 1.0, 1.5], [-5.0, 5.0, 15.0], 1),
    ] {
        vector(&mut bytes, scale);
        vector(&mut bytes, translation);
        name(
            &mut bytes,
            if frame == 0 { b"first" } else { b"second" },
            16,
        );
        for point in [[1u8, 2, 3], [4, 5, 6], [7, 8, 9], [10, 11, 12]] {
            bytes.extend(point.map(|value| value + frame));
            bytes.push(0);
        }
    }
    assert_eq!(bytes.len(), command_at as usize);
    bytes
}

pub fn md3() -> Vec<u8> {
    let mut bytes = include_bytes!("../../../formats/tests/fixtures/models/mesh.md3").to_vec();
    let frames = i32::from_le_bytes(bytes[92..96].try_into().unwrap()) as usize;
    // Keep frame zero native, but make the all-frame bounds distinguishable.
    bytes[frames + 56 + 12..frames + 56 + 16].copy_from_slice(&100.0f32.to_le_bytes());
    bytes
}

/// Two surfaces, each with its own shader list and two vertex frames.
pub fn md3_surfaces() -> Vec<u8> {
    let mut bytes = md3();
    let tags = i32::from_le_bytes(bytes[96..100].try_into().unwrap()) as usize;
    let axes = tags + 112 + 76;
    for (index, value) in [0.0f32, 1.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 1.0]
        .into_iter()
        .enumerate()
    {
        bytes[axes + index * 4..axes + index * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
    let at = i32::from_le_bytes(bytes[100..104].try_into().unwrap()) as usize;
    let original = bytes[at..].to_vec();
    let shader_at = i32::from_le_bytes(original[92..96].try_into().unwrap()) as usize;
    let mut first = original.clone();
    let mut shader = vec![0; 68];
    shader[..18].copy_from_slice(b"textures/alternate");
    first.splice(shader_at + 68..shader_at + 68, shader);
    first[76..80].copy_from_slice(&2i32.to_le_bytes());
    for offset in [96, 100, 104] {
        let value = i32::from_le_bytes(first[offset..offset + 4].try_into().unwrap());
        first[offset..offset + 4].copy_from_slice(&(value + 68).to_le_bytes());
    }
    let mut second = original;
    second[4..68].fill(0);
    second[4..10].copy_from_slice(b"glow_1");
    second[shader_at..shader_at + 64].fill(0);
    second[shader_at..shader_at + 13].copy_from_slice(b"textures/glow");
    bytes.truncate(at);
    bytes.extend(first);
    bytes.extend(second);
    bytes[84..88].copy_from_slice(&2i32.to_le_bytes());
    let end = bytes.len() as i32;
    bytes[104..108].copy_from_slice(&end.to_le_bytes());
    bytes
}

pub fn md2_skin(path: &[u8]) -> Vec<u8> {
    assert!(path.len() < 64);
    let mut bytes = md2();
    for skin in 0..2 {
        let first = 68 + skin * 64;
        bytes[first..first + 64].fill(0);
        bytes[first..first + path.len()].copy_from_slice(path);
    }
    bytes
}

pub fn md2_quad_skin(path: &[u8]) -> Vec<u8> {
    let mut bytes = md2_skin(path);
    let frame_at = i32::from_le_bytes(bytes[56..60].try_into().unwrap()) as usize;
    let stride = i32::from_le_bytes(bytes[16..20].try_into().unwrap()) as usize;
    for frame in 0..2 {
        for (index, point) in [[8, 0, 0], [8, 4, 0], [8, 4, 4], [8, 0, 4]]
            .into_iter()
            .enumerate()
        {
            let first = frame_at + frame * stride + 40 + index * 4;
            bytes[first..first + 3].copy_from_slice(&point);
        }
    }
    bytes
}

pub fn tga(width: u16, height: u16, pixels: &[[u8; 4]]) -> Vec<u8> {
    assert_eq!(pixels.len(), usize::from(width) * usize::from(height));
    let mut bytes = vec![0; 18];
    bytes[2] = 2;
    bytes[12..14].copy_from_slice(&width.to_le_bytes());
    bytes[14..16].copy_from_slice(&height.to_le_bytes());
    bytes[16] = 32;
    bytes[17] = 32;
    for &[r, g, b, a] in pixels {
        bytes.extend([b, g, r, a]);
    }
    bytes
}

pub fn pcx(index: u8) -> Vec<u8> {
    pcx_dimensions(8, 4, index)
}
pub fn pcx_dimensions(width: u16, height: u16, index: u8) -> Vec<u8> {
    let mut bytes = vec![0; 128];
    bytes[..4].copy_from_slice(&[10, 5, 1, 8]);
    bytes[8..10].copy_from_slice(&(width - 1).to_le_bytes());
    bytes[10..12].copy_from_slice(&(height - 1).to_le_bytes());
    bytes[65] = 1;
    bytes[66..68].copy_from_slice(&width.to_le_bytes());
    bytes.extend(std::iter::repeat_n(
        index,
        usize::from(width) * usize::from(height),
    ));
    bytes.push(12);
    // Deliberately different from the selected global palette.
    bytes.extend((0..=255u8).flat_map(|value| [255, value, value]));
    bytes
}

/// Embedded indexed skin, seam-adjusted UVs and two single-member frames.
pub fn mdl() -> Vec<u8> {
    let mut bytes = b"IDPO".to_vec();
    integer(&mut bytes, 6);
    vector(&mut bytes, [1.0; 3]);
    vector(&mut bytes, [0.0; 3]);
    bytes.extend(20.0f32.to_le_bytes());
    vector(&mut bytes, [0.0; 3]);
    for value in [1, 2, 1, 3, 1, 2, 0, 0] {
        integer(&mut bytes, value);
    }
    bytes.extend(1.0f32.to_le_bytes());
    assert_eq!(bytes.len(), 84);
    integer(&mut bytes, 0);
    bytes.extend([1, 2]);
    for (seam, s) in [(1, 0), (0, 1), (0, 0)] {
        for value in [seam, s, 0] {
            integer(&mut bytes, value);
        }
    }
    for value in [0, 0, 1, 2] {
        integer(&mut bytes, value);
    }
    for frame in [0u8, 1] {
        integer(&mut bytes, 0);
        bytes.extend([0, 0, 0, 0, 16, 16, 16, 0]);
        name(
            &mut bytes,
            if frame == 0 { b"first" } else { b"second" },
            16,
        );
        for point in [[8u8, 4, 0], [8, 0, 4], [8, 0, 0]] {
            bytes.extend([point[0] + frame * 2, point[1], point[2], 0]);
        }
    }
    bytes
}
