use qa_content::vfs::Vfs;
use qa_render::surface_cache::PaletteLighting;
use qa_render::{Assets, PaletteId};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

#[path = "../../../formats/tests/support/brushlist.rs"]
pub mod brushlist;

/// Parsed Q1 world with one embedded-texture face and two authored clip models.
/// No spawn entity is needed by a receiving CLIENT.
pub fn world_map(size: f32) -> Vec<u8> {
    world_map_with_lumps(size, |_| {})
}

pub fn world_map_with_lumps(size: f32, edit: impl FnOnce(&mut [Vec<u8>])) -> Vec<u8> {
    let brushes = [
        brushlist::chunk(0, &[brushlist::SourceBrush::cube(0.0, -2)]),
        brushlist::chunk(1, &[brushlist::SourceBrush::cube(10.0, -2)]),
    ]
    .concat();
    brushlist::bsp_with_lumps(29, 2, true, Some(&brushes), |lumps| {
        lumps[0] = b"{\"classname\" \"worldspawn\"}\0".to_vec();
        let mut texture = vec![0; 40];
        texture[..4].copy_from_slice(b"wall");
        for (at, value) in [
            (16, 16u32),
            (20, 16),
            (24, 40),
            (28, 296),
            (32, 360),
            (36, 376),
        ] {
            texture[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        texture.resize(380, 11);
        lumps[2] = [
            1u32.to_le_bytes().as_slice(),
            8u32.to_le_bytes().as_slice(),
            &texture,
        ]
        .concat();
        lumps[3] = [
            [0.0, -size, -size],
            [0.0, -size, size],
            [0.0, size, size],
            [0.0, size, -size],
        ]
        .into_iter()
        .flatten()
        .flat_map(f32::to_le_bytes)
        .collect();
        for (at, value) in [
            (8, -512i16),
            (10, -512),
            (12, -512),
            (14, 512),
            (16, 512),
            (18, 512),
            (22, 1),
        ] {
            lumps[5][at..at + 2].copy_from_slice(&value.to_le_bytes());
        }
        lumps[6] = [0.0f32, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0]
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .chain([0; 8])
            .collect();
        lumps[7] = vec![0; 20];
        lumps[7][8..10].copy_from_slice(&4u16.to_le_bytes());
        lumps[7][12..16].fill(255);
        lumps[7][16..20].copy_from_slice(&(-1i32).to_le_bytes());
        for leaf in 0..2 {
            for (axis, value) in [-512i16, -512, -512, 512, 512, 512].into_iter().enumerate() {
                let at = leaf * 28 + 8 + axis * 2;
                lumps[10][at..at + 2].copy_from_slice(&value.to_le_bytes());
            }
        }
        lumps[10][50..52].copy_from_slice(&1u16.to_le_bytes());
        lumps[11] = 0u16.to_le_bytes().to_vec();
        lumps[12] = [0u16, 1, 1, 2, 2, 3, 3, 0]
            .into_iter()
            .flat_map(u16::to_le_bytes)
            .collect();
        lumps[13] = [0i32, 1, 2, 3]
            .into_iter()
            .flat_map(i32::to_le_bytes)
            .collect();
        for model in 0..2 {
            lumps[14][model * 64 + 60..model * 64 + 64].copy_from_slice(&1i32.to_le_bytes());
        }
        edit(lumps);
    })
}

pub struct Fixture {
    pub vfs: Vfs,
    root: std::path::PathBuf,
}
impl Fixture {
    pub fn new(entries: &[(&str, Vec<u8>)]) -> Self {
        let root = std::env::temp_dir().join(format!(
            "qa-rust-model-assets-{}-{}-{}",
            std::process::id(),
            module_path!().replace("::", "-"),
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
            ("models/test.skin", b"tag_weapon,\nBODY,textures/alternate\nbody,textures/glow\nglow,textures/body\n".to_vec()),
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
    md3_named_surfaces(&[
        (b"BODY_1", &[b"textures/body", b"textures/alternate"]),
        (b"glow_1", &[b"textures/glow"]),
    ])
}

/// Reuse the real MD3 mesh while varying its native surface and shader declarations.
pub fn md3_named_surfaces(surfaces: &[(&[u8], &[&[u8]])]) -> Vec<u8> {
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
    let old_count = i32::from_le_bytes(original[76..80].try_into().unwrap()) as usize;
    bytes.truncate(at);
    for &(surface_name, shaders) in surfaces {
        assert!(surface_name.len() < 64);
        let mut surface = original.clone();
        surface[4..68].fill(0);
        surface[4..4 + surface_name.len()].copy_from_slice(surface_name);
        let mut declarations = Vec::new();
        for shader in shaders {
            assert!(shader.len() < 64);
            name(&mut declarations, shader, 64);
            integer(&mut declarations, 0);
        }
        let change = declarations.len() as i32 - (old_count * 68) as i32;
        surface.splice(shader_at..shader_at + old_count * 68, declarations);
        surface[76..80].copy_from_slice(&(shaders.len() as i32).to_le_bytes());
        for offset in [96, 100, 104] {
            let value = i32::from_le_bytes(surface[offset..offset + 4].try_into().unwrap());
            surface[offset..offset + 4].copy_from_slice(&(value + change).to_le_bytes());
        }
        bytes.extend(surface);
    }
    bytes[84..88].copy_from_slice(&(surfaces.len() as i32).to_le_bytes());
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

pub fn spr() -> Vec<u8> {
    let mut bytes =
        include_bytes!("../../../formats/tests/fixtures/models/group.spr")[..36].to_vec();
    bytes[8..12].copy_from_slice(&2i32.to_le_bytes());
    bytes[12..16].copy_from_slice(&8f32.to_le_bytes());
    bytes[16..20].copy_from_slice(&8i32.to_le_bytes());
    bytes[20..24].copy_from_slice(&8i32.to_le_bytes());
    bytes[24..28].copy_from_slice(&3i32.to_le_bytes());
    bytes[28..32].copy_from_slice(&1.5f32.to_le_bytes());
    bytes[32..36].copy_from_slice(&1i32.to_le_bytes());
    for (group, frames) in [
        (false, &[(-2, 2, 4, 4, 11u8)][..]),
        (true, &[(-3, 1, 4, 2, 22), (-1, 3, 4, 4, 33)][..]),
        (false, &[(-4, 1, 5, 3, 44)][..]),
    ] {
        integer(&mut bytes, i32::from(group));
        if group {
            integer(&mut bytes, frames.len() as i32);
            bytes.extend([0.125f32, 0.5].into_iter().flat_map(f32::to_le_bytes));
        }
        for &(x, y, width, height, index) in frames {
            for value in [x, y, width, height] {
                integer(&mut bytes, value);
            }
            bytes.extend(std::iter::repeat_n(index, (width * height) as usize));
        }
    }
    bytes
}

pub fn sp2() -> Vec<u8> {
    let mut bytes = b"IDS2".to_vec();
    integer(&mut bytes, 2);
    integer(&mut bytes, 2);
    for (width, height, x, y, path) in [
        (8, 4, 2, 1, "sprites/first.pcx"),
        (12, 6, -4, 3, "sprites/second.pcx"),
    ] {
        for value in [width, height, x, y] {
            integer(&mut bytes, value);
        }
        name(&mut bytes, path.as_bytes(), 64);
    }
    bytes
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

use qa_core::primitives::{Bounds, IndexSpan, Vec3};
use qa_render::world::{
    SurfaceMaterial, WorldId,
    geometry::{GeometryPartition, PatchStats, WorldGeometry, WorldModel},
};
use qa_world::visibility::{PvsRows, VisLeaf, VisibilityWorld};
pub fn world(assets: &mut Assets, origin: f32) -> WorldId {
    world_variants(assets, origin, &[&[]])
}

pub fn world_variants(
    assets: &mut Assets,
    origin: f32,
    variants: &[&[SurfaceMaterial]],
) -> WorldId {
    world_mutated(assets, origin, variants, |_| {})
}

pub fn world_mutated(
    assets: &mut Assets,
    origin: f32,
    variants: &[&[SurfaceMaterial]],
    mutate: impl FnOnce(&mut WorldGeometry),
) -> WorldId {
    let (geometry, visibility) = world_inputs(origin, mutate);
    let inputs: Vec<_> = variants
        .iter()
        .map(|&surfaces| qa_render::world::WorldPresentationInput::new(surfaces))
        .collect();
    assets
        .register_world(geometry, visibility, &inputs)
        .unwrap()
}

pub fn world_inputs(
    origin: f32,
    mutate: impl FnOnce(&mut WorldGeometry),
) -> (WorldGeometry, VisibilityWorld) {
    let bounds = Bounds {
        mins: Vec3([origin; 3]),
        maxs: Vec3([origin + 1.0; 3]),
    };
    let mut geometry = WorldGeometry {
        partition: GeometryPartition::SplitBsp,
        world_has_lightdata: false,
        vertices: vec![],
        indices: vec![],
        boundaries: vec![],
        surfaces: vec![],
        light_samples: vec![],
        models: vec![
            WorldModel {
                bounds,
                origin: Vec3([origin; 3]),
                surfaces: IndexSpan::default(),
            },
            WorldModel {
                bounds,
                origin: Vec3([origin + 1.0; 3]),
                surfaces: IndexSpan::default(),
            },
        ],
        patch_stats: PatchStats::default(),
    };
    mutate(&mut geometry);
    let mut bounds = bounds;
    for surface in &geometry.surfaces {
        bounds.add_bounds(surface.bounds);
    }
    let count = geometry.surfaces.len() as u32;
    let visibility = VisibilityWorld::load(
        vec![],
        vec![],
        vec![VisLeaf {
            selector: None,
            area: None,
            solid: false,
            bounds,
            surfaces: IndexSpan { first: 0, count },
        }],
        (0..count).collect(),
        count as usize,
        -1,
        PvsRows::all_visible(0),
    )
    .unwrap();
    (geometry, visibility)
}
