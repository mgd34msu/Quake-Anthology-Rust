use qa_content::vfs::Vfs;
use qa_core::primitives::{Bounds, Plane, RuleSetId, Vec3};
use qa_formats::{
    bsp::{
        Bsp, Face, IndexSpan, Leaf, Map, Model, Shader, Surface, SurfaceKind, TextureInfo, Vertex,
    },
    image::MipTexture,
};
use qa_render::{
    Assets, BlendPhase, CpuPresentation, FrontEnd, Limits, Refdef, Viewport,
    assets::{ImageId, PaletteId, StageTexture, TcMod},
    cpu::CpuBackend,
    material::resources::{ImageSettings, Images, PaletteSource, load_palette},
    material::world_load::{WorldLoadOptions, load_world},
    shader::{RgbGen, TexMod},
    surface_cache::{IndexedLighting, IndexedTexture},
    world::{VisibleSurface, geometry::TextureCoordinates},
};

fn map_bytes(family: u8) -> Vec<u8> {
    let (magic, count, light_lump, samples) = match family {
        1 => (29u32.to_le_bytes().to_vec(), 15, 8, 8),
        2 => (
            [b"IBSP".as_slice(), &38u32.to_le_bytes()].concat(),
            19,
            7,
            24,
        ),
        _ => (
            [b"IBSP".as_slice(), &46u32.to_le_bytes()].concat(),
            17,
            14,
            128 * 128 * 3,
        ),
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

fn empty_map(bytes: &[u8]) -> Result<Map<'_>, String> {
    Ok(Map {
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
    })
}

fn legacy_map<'a>(bytes: &'a [u8], family: u8) -> Result<Map<'a>, String> {
    let mut map = empty_map(bytes)?;
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
            edges: IndexSpan { first: 0, count: 4 },
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
        faces: IndexSpan { first: 0, count: 2 },
        brushes: IndexSpan { first: 0, count: 0 },
        ambient: [0; 4],
    });
    map.models.push(Model {
        bounds,
        origin: Vec3([0.0; 3]),
        headnodes: [-1; 4],
        visible_leaves: 0,
        faces: IndexSpan { first: 0, count: 2 },
        brushes: IndexSpan { first: 0, count: 0 },
        membership_from_tree: false,
    });
    Ok(map)
}

fn resources(label: &str) -> Result<std::path::PathBuf, String> {
    let root =
        std::env::temp_dir().join(format!("qa-world-materials-{label}-{}", std::process::id()));
    for directory in ["gfx", "pics", "textures", "scripts"] {
        std::fs::create_dir_all(root.join(directory)).map_err(|e| e.to_string())?;
    }
    let mut q1_palette: Vec<_> = (0..256).flat_map(|value| [value as u8; 3]).collect();
    q1_palette[30..33].copy_from_slice(&[120, 30, 8]);
    q1_palette[33..36].copy_from_slice(&[65, 15, 4]);
    q1_palette[43 * 3..44 * 3].copy_from_slice(&[3, 65, 210]);
    q1_palette[45 * 3..46 * 3].copy_from_slice(&[1, 30, 100]);
    std::fs::write(root.join("gfx/palette.lmp"), &q1_palette).map_err(|e| e.to_string())?;
    let mut colormap = vec![0; 16385];
    for row in colormap[..16384].chunks_exact_mut(256) {
        row[10] = 11;
        row[43] = 45;
    }
    colormap[16384] = 32;
    std::fs::write(root.join("gfx/colormap.lmp"), colormap).map_err(|e| e.to_string())?;
    let mut q2_palette: Vec<_> = (0..256).flat_map(|value| [value as u8; 3]).collect();
    q2_palette[30..33].copy_from_slice(&[3, 65, 210]);
    q2_palette[36..39].copy_from_slice(&[1, 30, 100]);
    q2_palette[42 * 3..43 * 3].copy_from_slice(&[120, 30, 8]);
    q2_palette[44 * 3..45 * 3].copy_from_slice(&[65, 15, 4]);
    let mut pcx = vec![0; 128];
    pcx[..4].copy_from_slice(&[10, 5, 1, 8]);
    pcx[8..10].copy_from_slice(&255u16.to_le_bytes());
    pcx[10..12].copy_from_slice(&319u16.to_le_bytes());
    let mut q2_colormap = vec![0; 256 * 64];
    for row in q2_colormap.chunks_exact_mut(256) {
        row[10] = 12;
        row[42] = 44;
    }
    pcx.extend_from_slice(&q2_colormap);
    pcx.extend(std::iter::repeat_n(0, 256 * 256));
    pcx.push(12);
    pcx.extend_from_slice(&q2_palette);
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
    let mut tga = vec![0; 18];
    tga[2] = 2;
    tga[12..14].copy_from_slice(&16u16.to_le_bytes());
    tga[14..16].copy_from_slice(&16u16.to_le_bytes());
    tga[16] = 24;
    tga[17] = 32;
    tga.extend(std::iter::repeat_n([90, 60, 30], 16 * 16).flatten());
    for pixel in tga[18..18 + 16 * 3].chunks_exact_mut(3) {
        pixel.copy_from_slice(&[3, 2, 1]);
    }
    std::fs::write(root.join("textures/modern.tga"), tga).map_err(|e| e.to_string())?;
    Ok(root)
}

fn mount(root: &std::path::Path) -> Result<Vfs, String> {
    let mut vfs = Vfs::default();
    vfs.mount_directory(root, 0)
        .map_err(|e| format!("VFS: {e:?}"))?;
    Ok(vfs)
}

#[test]
fn authored_stages_replace_only_matching_legacy_faces_and_keep_their_lightmaps()
-> Result<(), String> {
    let root = resources("authored")?;
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
            let vfs = mount(&root)?;
            let mut assets = Assets::load().map_err(|e| format!("assets: {e:?}"))?;
            let loaded = load_world(
                &vfs,
                &map,
                &mut assets,
                map.bsp.format.rule_set(),
                WorldLoadOptions::default(),
            )
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

fn surface_image(assets: &Assets, world: qa_render::world::WorldId) -> ImageId {
    let material = assets
        .material(assets.world(world).unwrap().bindings()[0].material)
        .unwrap();
    material
        .stages
        .iter()
        .find_map(|stage| match stage.texture {
            StageTexture::Image(image) => Some(image),
            _ => None,
        })
        .unwrap()
}

#[test]
fn selected_cpu_palette_preserves_source_colors_with_load_time_index_translation()
-> Result<(), String> {
    let root = resources("palettes")?;
    let vfs = mount(&root)?;
    for (family, source_color) in [(1, [120, 30, 8, 255]), (2, [3, 65, 210, 255])] {
        let bytes = map_bytes(family);
        let mut map = legacy_map(&bytes, family)?;
        // Native legacy materials cull front winding. Keep this face's boundary
        // clockwise when viewed from its positive-Z plane normal.
        map.surface_edges.reverse();
        for rules in [
            RuleSetId::Quake,
            RuleSetId::QuakeWorld,
            RuleSetId::Quake2,
            RuleSetId::Quake2Rerelease,
        ] {
            let mut assets = Assets::load().map_err(|e| format!("assets: {e:?}"))?;
            let loaded = load_world(&vfs, &map, &mut assets, rules, WorldLoadOptions::default())
                .map_err(|e| format!("material: {e:?}"))?;
            assert_eq!(loaded.presentation_rules, rules);
            let q1 = matches!(rules, RuleSetId::Quake | RuleSetId::QuakeWorld);
            let CpuPresentation::Indexed {
                palette, lighting, ..
            } = loaded.presentation.cpu
            else {
                panic!("selected legacy palette must retain indexed presentation");
            };
            let same_resource =
                PaletteSource::native(rules) == PaletteSource::native(map.bsp.format.rule_set());
            assert_eq!(palette, PaletteId(u32::from(!same_resource)));
            assert!(assets.palette(PaletteId(palette.0 + 1)).is_none());
            assert_eq!(
                lighting,
                if q1 {
                    IndexedLighting::Gray
                } else {
                    IndexedLighting::NativeRgb
                }
            );
            assert_eq!(
                loaded.presentation.lightstyles[0].indexed_scale,
                if q1 { 264 } else { 384 }
            );
            assert_eq!(loaded.presentation.blend_phase, BlendPhase::FinalPalette);
            let image = assets.image(surface_image(&assets, loaded.world)).unwrap();
            assert_eq!(&image.rgba[..4], &source_color);
            let expected_index = if same_resource {
                10
            } else if q1 {
                43
            } else {
                42
            };
            for mip in 0..4 {
                assert!(
                    image
                        .indexed
                        .as_ref()
                        .unwrap()
                        .mip(mip)
                        .unwrap()
                        .indices()
                        .iter()
                        .all(|&index| index == expected_index)
                );
            }
            let expected = if family == 1 {
                [65, 15, 4, 255]
            } else {
                [1, 30, 100, 255]
            };
            let mut cpu = CpuBackend::load_with_assets(8, 8, &assets).map_err(str::to_owned)?;
            let mut frontend = FrontEnd::load(Limits::default()).map_err(str::to_owned)?;
            let mut frame = frontend.begin_frame([0, 0, 0, 255]).ok_or("frame")?;
            assert!(frame.add_world(
                loaded.world,
                &[VisibleSurface {
                    surface: 0,
                    depth_key: 0
                }]
            ));
            let mut view = Refdef {
                viewport: Viewport {
                    width: 8,
                    height: 8,
                    ..Viewport::default()
                },
                origin: Vec3([8.0, 8.0, 8.0]),
                axes: [
                    Vec3([0.0, 0.0, -1.0]),
                    Vec3([0.0, 1.0, 0.0]),
                    Vec3([1.0, 0.0, 0.0]),
                ],
                fov: [90.0; 2],
                near: 0.25,
                far: 64.0,
                ..Refdef::default()
            };
            loaded.presentation.apply(&mut view);
            assert!(frame.render_scene(view, &[], &assets));
            let packet = frame.finish();
            let stats = cpu.render(&packet, &assets);
            assert_eq!(stats.rejected, 0);
            assert_eq!(cpu.world_stats().polygons, 1);
            // The selected colormap consumes remapped software indices;
            // source expansion still owns unchanged GL RGBA color bytes.
            assert_eq!(cpu.pixels()[4 * 8 + 4], u32::from_le_bytes(expected));
        }
    }
    std::fs::remove_dir_all(root).map_err(|e| e.to_string())?;
    Ok(())
}

#[test]
fn loading_one_map_with_two_presentations_keeps_previous_assets_unchanged() -> Result<(), String> {
    let root = resources("cache-roles")?;
    let vfs = mount(&root)?;
    let bytes = map_bytes(1);
    let map = legacy_map(&bytes, 1)?;
    let mut assets = Assets::load().map_err(|e| format!("assets: {e:?}"))?;
    let first = load_world(
        &vfs,
        &map,
        &mut assets,
        RuleSetId::Quake,
        WorldLoadOptions::default(),
    )
    .map_err(|e| format!("first material: {e:?}"))?;
    let first_binding = assets.world(first.world).unwrap().bindings()[0];
    let first_stages = assets
        .material(first_binding.material)
        .unwrap()
        .stages
        .to_vec();
    let first_image = surface_image(&assets, first.world);
    let original = assets.image(first_image).unwrap();
    let original_revision = original.preparation_revision;
    let original_pixels = original.prepared.as_ref().unwrap().levels[0].rgba.to_vec();
    assert_eq!(original.prepared.as_ref().unwrap().levels[0].width, 16);
    let second = load_world(
        &vfs,
        &map,
        &mut assets,
        RuleSetId::Quake3,
        WorldLoadOptions::default(),
    )
    .map_err(|e| format!("second material: {e:?}"))?;
    assert_eq!(second.presentation.cpu, CpuPresentation::Rgb);
    let second_image = surface_image(&assets, second.world);
    assert_ne!(second.world, first.world);
    assert_ne!(second_image, first_image);
    assert_ne!(
        assets.world(second.world).unwrap().bindings()[0].material,
        first_binding.material
    );
    assert_eq!(
        assets.image(second_image).unwrap().rgba,
        assets.image(first_image).unwrap().rgba
    );
    assert_eq!(
        assets
            .image(second_image)
            .unwrap()
            .prepared
            .as_ref()
            .unwrap()
            .levels[0]
            .width,
        8
    );
    assert_eq!(
        assets
            .material(first_binding.material)
            .unwrap()
            .stages
            .as_ref(),
        first_stages
    );
    assert_eq!(
        assets.image(first_image).unwrap().preparation_revision,
        original_revision
    );
    assert_eq!(
        assets
            .image(first_image)
            .unwrap()
            .prepared
            .as_ref()
            .unwrap()
            .levels[0]
            .rgba
            .as_ref(),
        original_pixels
    );
    // An explicit upload override is honored independently of presentation.
    let third = load_world(
        &vfs,
        &map,
        &mut assets,
        RuleSetId::Quake3,
        WorldLoadOptions {
            image_settings: Some(ImageSettings {
                picmip: 2,
                ..ImageSettings::native(RuleSetId::Quake3)
            }),
            ..WorldLoadOptions::default()
        },
    )
    .map_err(|e| format!("third material: {e:?}"))?;
    assert_eq!(
        assets
            .image(surface_image(&assets, third.world))
            .unwrap()
            .prepared
            .as_ref()
            .unwrap()
            .levels[0]
            .width,
        4
    );
    std::fs::remove_dir_all(root).map_err(|e| e.to_string())?;
    Ok(())
}

#[test]
fn foreign_presentation_keeps_q3_geometry_coordinates_and_source_overbright() -> Result<(), String>
{
    let root = resources("q3-source")?;
    let vfs = mount(&root)?;
    let bytes = map_bytes(3);
    let mut map = legacy_map(&bytes, 3)?;
    map.faces.clear();
    map.leaf_faces = vec![0];
    map.leaves[0].faces.count = 1;
    map.models[0].faces.count = 1;
    map.shaders.push(Shader {
        name: b"textures/modern",
        surface_flags: 0,
        content_flags: 0,
    });
    map.indices = vec![0, 1, 2, 0, 2, 3];
    for vertex in &mut map.vertices {
        vertex.texcoord = [0.25, 0.75];
        vertex.lightmap_coord = [0.125, 0.875];
        vertex.color = [10, 20, 30, 255];
    }
    map.surfaces.push(Surface {
        kind: SurfaceKind::Planar,
        shader: Some(0),
        shader_name: b"textures/modern",
        brush_side: -1,
        fog: -1,
        vertices: IndexSpan { first: 0, count: 4 },
        indices: IndexSpan { first: 0, count: 6 },
        lightmap: 0,
        lightmap_rect: [0; 4],
        lightmap_origin: Vec3::default(),
        lightmap_vectors: [Vec3::default(); 3],
        patch: [0; 2],
        triangle_fan: false,
    });
    let mut assets = Assets::load().map_err(|e| format!("assets: {e:?}"))?;
    for rules in [RuleSetId::Quake3, RuleSetId::Quake, RuleSetId::Quake2] {
        let loaded = load_world(&vfs, &map, &mut assets, rules, WorldLoadOptions::default())
            .map_err(|e| format!("material: {e:?}"))?;
        let world = assets.world(loaded.world).unwrap();
        assert_eq!(
            world.geometry().surfaces[0].texture_coordinates,
            TextureCoordinates::Normalized
        );
        let vertex = world.geometry().vertices[0].vertex;
        assert_eq!(vertex.texcoord, [0.25, 0.75]);
        assert_eq!(vertex.lightmap_coord, [0.125, 0.875]);
        assert_eq!(vertex.color, [40, 80, 120, 255]);
        let image = assets.image(surface_image(&assets, loaded.world)).unwrap();
        // Source Q3 ignores the top-origin bit, even with a Q1/Q2 upload
        // policy. A presentation-selected decoder would return [1, 2, 3].
        assert_eq!(&image.rgba[..4], &[30, 60, 90, 255]);
        assert!(image.indexed.is_none());
        assert_eq!(
            matches!(loaded.presentation.cpu, CpuPresentation::Rgb),
            rules == RuleSetId::Quake3
        );
    }
    std::fs::remove_dir_all(root).map_err(|e| e.to_string())?;
    Ok(())
}

#[test]
fn palette_translation_preserves_cutout_and_cloud_mask_indices() -> Result<(), String> {
    let root = resources("masks")?;
    let vfs = mount(&root)?;
    let mut assets = Assets::load().map_err(|e| format!("assets: {e:?}"))?;
    let source = load_palette(
        &vfs,
        &mut assets,
        PaletteSource::native(RuleSetId::Quake).unwrap(),
    )
    .map_err(|e| format!("source palette: {e:?}"))?;
    let target = load_palette(
        &vfs,
        &mut assets,
        PaletteSource::native(RuleSetId::Quake2).unwrap(),
    )
    .map_err(|e| format!("target palette: {e:?}"))?;
    for (presentation, selected) in [(RuleSetId::Quake, source), (RuleSetId::Quake2, target)] {
        let mut images = Images::new(
            &vfs,
            &mut assets,
            RuleSetId::Quake,
            presentation,
            ImageSettings::native(presentation),
            Some((source, selected)),
        )
        .map_err(|e| format!("images: {e:?}"))?;
        for mask in [None, Some(0), Some(255)] {
            let indices = [0, 10, 255, 10];
            let rgba: Vec<_> = indices
                .iter()
                .flat_map(|&index| {
                    let mut color = images
                        .assets
                        .palette(source)
                        .unwrap()
                        .color(index)
                        .to_le_bytes();
                    if mask == Some(index) {
                        color[3] = 0;
                    }
                    color
                })
                .collect();
            let texture = IndexedTexture::load_base(4, 1, &indices, mask).map_err(str::to_owned)?;
            let image = images
                .register_indexed_rgba(4, 1, &rgba, texture, source)
                .map_err(|e| format!("indexed image: {e:?}"))?;
            let image = images.assets.image(image).unwrap();
            assert_eq!(image.rgba.as_ref(), rgba);
            let texture = image.indexed.as_ref().unwrap();
            assert_eq!(texture.transparent_index(), mask);
            let converted = texture.mip(0).unwrap().indices();
            if selected == source {
                assert_eq!(converted, indices);
            } else {
                assert_eq!(converted[1], 42);
                assert_eq!(converted[3], 42);
                for (original, converted) in indices.iter().zip(converted) {
                    assert_eq!(mask == Some(*original), mask == Some(*converted));
                }
            }
        }
    }
    std::fs::remove_dir_all(root).map_err(|e| e.to_string())?;
    Ok(())
}
