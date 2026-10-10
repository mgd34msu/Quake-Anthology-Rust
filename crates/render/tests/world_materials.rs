use qa_content::vfs::Vfs;
use qa_core::primitives::{Bounds, Plane, Vec3};
use qa_formats::{
    bsp::{Bsp, Face, IndexRange, Leaf, Map, Model, TextureInfo, Vertex},
    image::MipTexture,
};
use qa_render::{
    Assets,
    assets::{StageTexture, TcMod},
    material::world_load::{WorldLoadOptions, load_world},
    shader::{RgbGen, TexMod},
};

fn map_bytes(family: u8) -> Vec<u8> {
    let (magic, count, light_lump, samples) = if family == 1 {
        (29u32.to_le_bytes().to_vec(), 15, 8, 8)
    } else {
        (
            [b"IBSP".as_slice(), &38u32.to_le_bytes()].concat(),
            19,
            7,
            24,
        )
    };
    let header = magic.len() + count * 8;
    let mut bytes = vec![0; header];
    bytes[..magic.len()].copy_from_slice(&magic);
    for index in 0..count {
        let at = magic.len() + index * 8;
        bytes[at..at + 4].copy_from_slice(&(header as u32).to_le_bytes());
        if index == light_lump {
            bytes[at + 4..at + 8].copy_from_slice(&(samples as u32).to_le_bytes());
        }
    }
    bytes.extend(std::iter::repeat_n(128, samples));
    bytes
}

fn legacy_map<'a>(bytes: &'a [u8], family: u8) -> Result<Map<'a>, String> {
    let mut map = Map {
        bsp: Bsp::parse(bytes).map_err(|e| format!("BSP: {e:?}"))?,
        planes: vec![],
        vertices: vec![],
        nodes: vec![],
        leaves: vec![],
        edges: vec![],
        surface_edges: vec![],
        faces: vec![],
        leaf_faces: vec![],
        leaf_brushes: vec![],
        clipnodes: vec![],
        texture_info: vec![],
        textures: vec![],
        models: vec![],
        brushes: vec![],
        brush_sides: vec![],
        shaders: vec![],
        fogs: vec![],
        surfaces: vec![],
        indices: vec![],
        areas: vec![],
        area_portals: vec![],
        extensions: vec![],
    };
    let bounds = Bounds {
        mins: Vec3([0.0; 3]),
        maxs: Vec3([16.0, 16.0, 0.0]),
    };
    map.planes.push(Plane {
        encoding: None,
        normal: Vec3([0.0, 0.0, 1.0]),
        distance: 0.0,
        axis: None,
    });
    map.vertices = [
        [0.0, 0.0, 0.0],
        [16.0, 0.0, 0.0],
        [16.0, 16.0, 0.0],
        [0.0, 16.0, 0.0],
    ]
    .map(|position| Vertex {
        position: Vec3(position),
        normal: Vec3([0.0, 0.0, 1.0]),
        texcoord: [0.0; 2],
        lightmap_coord: [0.0; 2],
        color: [255; 4],
    })
    .to_vec();
    map.edges = vec![[0, 1], [1, 2], [2, 3], [3, 0]];
    map.surface_edges = vec![0, 1, 2, 3];
    for (index, name) in [b"wall".as_slice(), b"other"].into_iter().enumerate() {
        map.texture_info.push(TextureInfo {
            projection: [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0]],
            flags: 0,
            texture: index as i32,
            value: 0,
            next: -1,
            name,
        });
        map.textures.push(Some(MipTexture {
            name,
            width: 16,
            height: 16,
            shift: 0,
            levels: [&[10; 256], &[10; 64], &[10; 16], &[10; 4]],
            animation: &[],
            flags: 0,
            contents: 0,
            value: 0,
        }));
        map.faces.push(Face {
            plane: 0,
            flags: 0,
            edges: IndexRange { first: 0, count: 4 },
            texture_info: index as u32,
            styles: [0, 255, 255, 255],
            lighting_offset: (index * if family == 1 { 4 } else { 12 }) as i32,
        });
    }
    map.leaf_faces = vec![0, 1];
    map.leaves.push(Leaf {
        contents: if family == 1 { -1 } else { 0 },
        cluster: -1,
        area: -1,
        visibility_offset: -1,
        bounds,
        faces: IndexRange { first: 0, count: 2 },
        brushes: IndexRange { first: 0, count: 0 },
        ambient: [0; 4],
    });
    map.models.push(Model {
        bounds,
        origin: Vec3([0.0; 3]),
        headnodes: [-1; 4],
        visible_leaves: 0,
        faces: IndexRange { first: 0, count: 2 },
        brushes: IndexRange { first: 0, count: 0 },
        membership_from_tree: false,
    });
    Ok(map)
}

#[test]
fn authored_stages_replace_only_matching_legacy_faces_and_keep_their_lightmaps()
-> Result<(), String> {
    let root = std::env::temp_dir().join(format!("qa-world-materials-{}", std::process::id()));
    for directory in ["gfx", "pics", "textures", "scripts"] {
        std::fs::create_dir_all(root.join(directory)).map_err(|e| e.to_string())?;
    }
    let palette: Vec<_> = (0..256).flat_map(|value| [value as u8; 3]).collect();
    std::fs::write(root.join("gfx/palette.lmp"), &palette).map_err(|e| e.to_string())?;
    std::fs::write(root.join("gfx/colormap.lmp"), vec![0; 16385]).map_err(|e| e.to_string())?;
    let mut pcx = vec![0; 128];
    pcx[..4].copy_from_slice(&[10, 5, 1, 8]);
    pcx[8..10].copy_from_slice(&255u16.to_le_bytes());
    pcx[10..12].copy_from_slice(&319u16.to_le_bytes());
    pcx.extend(std::iter::repeat_n(0, 256 * 320));
    pcx.push(12);
    pcx.extend_from_slice(&palette);
    std::fs::write(root.join("pics/colormap.pcx"), pcx).map_err(|e| e.to_string())?;
    let mut wal = vec![0; 100];
    wal[32..36].copy_from_slice(&16u32.to_le_bytes());
    wal[36..40].copy_from_slice(&16u32.to_le_bytes());
    for (mip, offset) in [100u32, 356, 420, 436].into_iter().enumerate() {
        wal[40 + mip * 4..44 + mip * 4].copy_from_slice(&offset.to_le_bytes());
    }
    wal.extend(std::iter::repeat_n(10, 340));
    for name in ["wall", "other"] {
        std::fs::write(root.join(format!("textures/{name}.wal")), &wal)
            .map_err(|e| e.to_string())?;
    }
    for family in [1, 2] {
        let bytes = map_bytes(family);
        let map = legacy_map(&bytes, family)?;
        for authored in [false, true] {
            let shader = if authored {
                "Textures/WALL { { map $whiteimage rgbGen wave sin 0.5 0.5 0 1 tcMod scroll 1 0 } { map $lightmap blendFunc filter } }"
            } else {
                "unrelated { { map $whiteimage } }"
            };
            std::fs::write(root.join("scripts/test.shader"), shader).map_err(|e| e.to_string())?;
            let mut vfs = Vfs::default();
            vfs.mount_directory(&root, 0)
                .map_err(|e| format!("VFS: {e:?}"))?;
            let mut assets = Assets::load().map_err(|e| format!("assets: {e:?}"))?;
            let loaded = load_world(&vfs, &map, &mut assets, WorldLoadOptions::default())
                .map_err(|e| format!("material: {e:?}"))?;
            let world = assets.world(loaded.world).ok_or("world")?;
            let first = world.bindings()[0];
            let second = world.bindings()[1];
            let changed = assets.material(first.material).ok_or("first material")?;
            let stock = assets.material(second.material).ok_or("second material")?;
            assert_eq!(assets.name(stock.name), Some("other"));
            assert_eq!(stock.stages.len(), 2);
            assert!(stock.stages.iter().all(|stage| stage.tcmods == [None; 4]));
            assert_ne!(first.lightmap, qa_render::assets::ImageId(0));
            assert_eq!(first.lightmap, second.lightmap);
            assert!(first.lightmap_region.is_some());
            assert_eq!(first.texture_scale, [1.0 / 16.0; 2]);
            if authored {
                assert_eq!(assets.name(changed.name), Some("textures/wall"));
                assert!(matches!(changed.stages[0].rgb_gen, RgbGen::Wave(_)));
                assert_eq!(
                    changed.stages[0].tcmods[0],
                    Some(TcMod::Script(TexMod::Scroll([1.0, 0.0])))
                );
            } else {
                assert_eq!(assets.name(changed.name), Some("wall"));
                assert!(changed.stages.iter().all(|stage| stage.tcmods == [None; 4]));
            }
            assert!(
                changed
                    .stages
                    .iter()
                    .any(|stage| matches!(stage.texture, StageTexture::Lightmap))
            );
        }
    }
    std::fs::remove_dir_all(&root).map_err(|e| e.to_string())?;
    Ok(())
}
