use qa_content::vfs::Vfs;
use qa_core::primitives::{Bounds, Plane, RuleSetId, Vec3};
use qa_formats::{
    bsp::{
        Bsp, DecoupledLightmap, Face, IndexSpan, Leaf, Map, Model, Shader, Surface, SurfaceKind,
        TextureInfo, Vertex,
    },
    image::MipTexture,
};
use qa_render::{
    Assets, BlendPhase, CpuPresentation, FrontEnd, Limits, Refdef, Viewport,
    assets::{ImageId, PaletteId, StageTexture, TcMod},
    cpu::CpuBackend,
    material::resources::{ImageSettings, Images, PaletteSource, load_palette},
    material::world_load::{
        LoadedWorld, LoadedWorldPresentation, WorldLoadOptions, WorldPresentationProfile,
        load_world,
    },
    shader::{RgbGen, TexMod},
    surface_cache::{IndexedLighting, IndexedTexture},
    world::{VisibleSurface, WorldPresentationId, geometry::TextureCoordinates},
};

fn load_native_world(
    vfs: &Vfs,
    map: &Map<'_>,
    assets: &mut Assets,
    rules: RuleSetId,
    options: WorldLoadOptions,
) -> Result<LoadedWorld, qa_render::material::world_load::WorldMaterialError> {
    let source = PaletteSource::native(map.bsp.format.rule_set());
    let source_palette = source
        .map(|source| load_palette(vfs, assets, source))
        .transpose()?;
    let target = PaletteSource::native(rules);
    let target_palette = if target == source {
        source_palette
    } else {
        target
            .map(|source| load_palette(vfs, assets, source))
            .transpose()?
    };
    load_world(
        vfs,
        map,
        assets,
        source_palette,
        &[WorldPresentationProfile::native(rules, target_palette).into()],
        options,
    )
}

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
        authored_brushes: false,
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
        decoupled_lightmaps: vec![],
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

#[test]
fn independent_lightmap_projection_packs_large_grids_with_original_styles() -> Result<(), String> {
    let root = resources("independent-lightmaps")?;
    let vfs = mount(&root)?;
    for (width, expected_page_size) in [(129u32, 1024u32), (1024, 2048)] {
        let cells = width as usize * 2;
        let lighting: Vec<_> = std::iter::repeat_n([20, 40, 80], cells)
            .chain(std::iter::repeat_n([5, 10, 20], cells))
            .flatten()
            .collect();
        let mut bytes = map_bytes(2);
        let header = 8 + 19 * 8;
        bytes.truncate(header);
        let light_length = 8 + 7 * 8 + 4;
        bytes[light_length..light_length + 4]
            .copy_from_slice(&(lighting.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&lighting);
        let mut map = legacy_map(&bytes, 2)?;
        map.faces[0].styles = [0, 7, 255, 255];
        let projection = [[0.0, 1.0 / 16.0, 0.0, 0.0], [-1.0 / 16.0, 0.0, 0.0, 1.0]];
        map.decoupled_lightmaps = vec![
            Some(DecoupledLightmap {
                width,
                height: 2,
                offset: Some(0),
                projection,
            }),
            Some(DecoupledLightmap {
                width: 0,
                height: 0,
                offset: None,
                projection,
            }),
        ];
        let mut assets = Assets::load().map_err(|e| format!("assets: {e:?}"))?;
        let loaded = load_native_world(
            &vfs,
            &map,
            &mut assets,
            RuleSetId::Quake2,
            WorldLoadOptions::default(),
        )
        .map_err(|e| format!("independent materials: {e:?}"))?;
        assert!(matches!(
            loaded.presentations[0].defaults.cpu,
            CpuPresentation::Indexed {
                lighting: IndexedLighting::NativeRgb,
                ..
            }
        ));
        let world = assets.world(loaded.world).ok_or("world")?;
        let binding = *assets
            .surface_binding(loaded.presentations[0].id, 0)
            .unwrap();
        let binding = assets
            .shader_binding(binding.shader)
            .ok_or("shader binding")?;
        let region = binding.lightmap_region.ok_or("lit region")?;
        assert_eq!(region.page_size, expected_page_size);
        assert_eq!((region.width, region.height), (width, 2));
        let image = assets.image(binding.lightmap).ok_or("lightmap image")?;
        assert_eq!(
            (image.width, image.height),
            (expected_page_size, expected_page_size)
        );
        assert_eq!(&image.rgba[..4], &[25, 50, 100, 255]);
        let geometry = world.geometry();
        let surface = &geometry.surfaces[0];
        assert_eq!(surface.texture_minima, [0; 2]);
        assert_eq!(surface.texture_extents, [16; 2]);
        assert_eq!(surface.styles, [0, 7, 255, 255]);
        for (vertex, raw) in geometry.vertices[surface.vertices.range()].iter().zip([
            [0.0, 1.0],
            [0.0, 0.0],
            [1.0, 0.0],
            [1.0, 1.0],
        ]) {
            assert_eq!(vertex.vertex.lightmap_coord, region.uv_at_sample(raw));
        }
        assert!(
            assets
                .shader_binding(
                    assets
                        .surface_binding(loaded.presentations[0].id, 1)
                        .unwrap()
                        .shader
                )
                .unwrap()
                .lightmap_region
                .is_none()
        );
        assert_eq!(
            assets
                .shader_binding(
                    assets
                        .surface_binding(loaded.presentations[0].id, 1)
                        .unwrap()
                        .shader
                )
                .unwrap()
                .lightmap,
            qa_render::assets::ImageId(0)
        );
        assert_eq!(geometry.surfaces[1].light_samples.count, 0);
    }
    std::fs::remove_dir_all(root).map_err(|e| e.to_string())?;
    Ok(())
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
fn registered_suns_follow_creation_order_and_remain_owned_by_each_presentation()
-> Result<(), String> {
    let root = resources("world-suns")?;
    let bytes = map_bytes(1);
    let map = legacy_map(&bytes, 1)?;
    let mut assets = Assets::load().unwrap();
    let mut presentations = Vec::new();
    for (shader, expected) in [
        (
            "unused {\n q3map_sun 1 1 1 200 90 0\n { map $whiteimage }\n }",
            qa_render::lighting::default_sun_direction(),
        ),
        (
            "textures/wall {\n q3map_sun 1 1 1 200 90 60\n { map $whiteimage }\n }\n textures/other {\n q3map_sun 1 1 1 200 0 0\n { map $whiteimage }\n }",
            Vec3([1., 0., 0.]),
        ),
        (
            "textures/wall {\n q3map_sun 1 1 1 200 90 0\n { map $whiteimage }\n }",
            Vec3([
                (f64::from(std::f32::consts::FRAC_PI_2).cos()) as f32,
                1.,
                0.,
            ]),
        ),
    ] {
        std::fs::write(root.join("scripts/test.shader"), shader).unwrap();
        let loaded = load_native_world(
            &mount(&root)?,
            &map,
            &mut assets,
            RuleSetId::Quake,
            WorldLoadOptions::default(),
        )
        .map_err(|error| format!("sun world: {error:?}"))?;
        assert_eq!(
            assets
                .world_presentation(loaded.presentations[0].id)
                .unwrap()
                .sun_direction(),
            expected
        );
        presentations.push((loaded.presentations[0].id, expected));
        for &(presentation, expected) in &presentations {
            assert_eq!(
                assets
                    .world_presentation(presentation)
                    .unwrap()
                    .sun_direction(),
                expected
            );
        }
    }
    std::fs::remove_dir_all(root).unwrap();
    Ok(())
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
            let loaded = load_native_world(
                &vfs,
                &map,
                &mut assets,
                map.bsp.format.rule_set(),
                WorldLoadOptions::default(),
            )
            .map_err(|e| format!("material: {e:?}"))?;
            assets.world(loaded.world).ok_or("world")?;
            let first = *assets
                .surface_binding(loaded.presentations[0].id, 0)
                .unwrap();
            let second = *assets
                .surface_binding(loaded.presentations[0].id, 1)
                .unwrap();
            assert_eq!(first.texture_scale, [1.0 / 16.0; 2]);
            let first = assets.shader_binding(first.shader).ok_or("first shader")?;
            let second = assets
                .shader_binding(second.shader)
                .ok_or("second shader")?;
            let changed = assets.material(first.material).ok_or("first material")?;
            let stock = assets.material(second.material).ok_or("second material")?;
            assert_eq!(assets.name(stock.name), Some("other"));
            assert_eq!(stock.stages.len(), 2);
            assert!(stock.stages.iter().all(|stage| stage.tcmods == [None; 4]));
            assert_ne!(first.lightmap, qa_render::assets::ImageId(0));
            assert_eq!(first.lightmap, second.lightmap);
            assert!(first.lightmap_region.is_some());
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

#[test]
fn opaque_native_water_and_flow_register_software_sampling_without_changing_gl_stages()
-> Result<(), String> {
    let root = resources("software-tiled")?;
    for (family, flags, warp, speed, opaque) in [
        (1, 0, true, None, true),
        (2, 8, true, None, true),
        (2, 64, false, Some(0.77), true),
        (2, 72, true, Some(0.25), true),
        (2, 8 | 16, true, None, false),
        (2, 64 | 32, false, Some(0.77), false),
    ] {
        let bytes = map_bytes(family);
        let mut map = legacy_map(&bytes, family)?;
        map.texture_info[0].flags = flags;
        if family == 1 {
            map.textures[0].as_mut().ok_or("embedded water")?.name = b"*water";
        }
        let vfs = mount(&root)?;
        let mut assets = Assets::load().map_err(|e| e.to_string())?;
        let loaded = load_native_world(
            &vfs,
            &map,
            &mut assets,
            map.bsp.format.rule_set(),
            WorldLoadOptions::default(),
        )
        .map_err(|e| format!("water material: {e:?}"))?;
        let binding = *assets
            .surface_binding(loaded.presentations[0].id, 0)
            .ok_or("binding")?;
        let binding = assets
            .shader_binding(binding.shader)
            .ok_or("shader binding")?;
        let material = assets.material(binding.material).ok_or("material")?;
        let base = &material.stages[0];
        if opaque {
            let tiled = material
                .settings
                .software_tiled
                .ok_or("software tiled sampling")?;
            assert_eq!(tiled.turbulent, warp);
            assert_eq!(tiled.flow_speed, speed);
            assert_eq!(tiled.time_f32, family == 2);
            assert!(base.blend.is_none());
            assert!(base.depth_write);
        } else {
            assert!(material.settings.software_tiled.is_none());
            assert!(base.blend.is_some());
            assert!(!base.depth_write);
        }
        assert_eq!(matches!(base.tcmods[0], Some(TcMod::Warp(_))), warp);
        if flags & 64 != 0 {
            let Some(TcMod::Flow(flow)) = base.tcmods[usize::from(warp)] else {
                return Err("GL flow stage missing".into());
            };
            assert_eq!(
                flow.speed.to_bits(),
                (if warp { 0.5_f32 } else { 1.0 / 40.0 }).to_bits()
            );
            assert_eq!(
                flow.amplitude,
                if warp { [-1.0, 0.0] } else { [-64.0, 0.0] }
            );
            assert_eq!(flow.cycle_start, if warp { [0.0; 2] } else { [-64.0, 0.0] });
        }
    }
    std::fs::remove_dir_all(root).map_err(|e| e.to_string())
}

fn surface_image(assets: &Assets, presentation: WorldPresentationId) -> ImageId {
    let material = assets
        .material(
            assets
                .shader_binding(assets.surface_binding(presentation, 0).unwrap().shader)
                .unwrap()
                .material,
        )
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
            let loaded =
                load_native_world(&vfs, &map, &mut assets, rules, WorldLoadOptions::default())
                    .map_err(|e| format!("material: {e:?}"))?;
            assert_eq!(loaded.presentations[0].profile.rules, rules);
            let q1 = matches!(rules, RuleSetId::Quake | RuleSetId::QuakeWorld);
            let CpuPresentation::Indexed {
                palette, lighting, ..
            } = loaded.presentations[0].defaults.cpu
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
                loaded.presentations[0].defaults.lightstyles[0].indexed_scale,
                if q1 { 264 } else { 384 }
            );
            assert_eq!(
                loaded.presentations[0].defaults.blend_phase,
                BlendPhase::FinalPalette
            );
            let image = assets
                .image(surface_image(&assets, loaded.presentations[0].id))
                .unwrap();
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
                loaded.presentations[0].id,
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
            loaded.presentations[0].defaults.apply(&mut view);
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
fn one_world_registers_exact_profiles_once_without_mutating_previous_assets() -> Result<(), String>
{
    let root = resources("cache-roles")?;
    let vfs = mount(&root)?;
    let bytes = map_bytes(1);
    let map = legacy_map(&bytes, 1)?;
    let mut assets = Assets::load().map_err(|e| format!("assets: {e:?}"))?;
    let palette = load_palette(
        &vfs,
        &mut assets,
        PaletteSource::native(RuleSetId::Quake).unwrap(),
    )
    .map_err(|e| format!("palette: {e:?}"))?;
    let first = WorldPresentationProfile::native(RuleSetId::Quake, Some(palette));
    let second = WorldPresentationProfile::native(RuleSetId::Quake3, None);
    let third = WorldPresentationProfile {
        image_settings: ImageSettings {
            picmip: 2,
            ..second.image_settings
        },
        ..second
    };
    let loaded = load_world(
        &vfs,
        &map,
        &mut assets,
        Some(palette),
        &[first.into(), second.into(), third.into(), first.into()],
        WorldLoadOptions::default(),
    )
    .map_err(|e| format!("material variants: {e:?}"))?;
    assert_eq!(assets.worlds().len(), 1);
    assert_eq!(assets.models().len(), 2); // The empty model and one static mesh.
    assert_eq!(loaded.presentations.len(), 4);
    assert_eq!(loaded.presentations[0].id, loaded.presentations[3].id);
    assert_eq!(assets.world(loaded.world).unwrap().presentations().len(), 3);
    let mesh_range = assets
        .surface_binding(loaded.presentations[0].id, 0)
        .unwrap()
        .mesh_indices;
    let original = surface_image(&assets, loaded.presentations[0].id);
    let original_pixels = assets
        .image(original)
        .unwrap()
        .prepared
        .as_ref()
        .unwrap()
        .levels[0]
        .rgba
        .to_vec();
    let revision = assets.image(original).unwrap().preparation_revision;
    let mut images = Vec::new();
    for (registration, width) in loaded.presentations.iter().take(3).zip([16, 8, 4]) {
        let binding = assets.surface_binding(registration.id, 0).unwrap();
        assert_eq!(
            assets.world_presentation(registration.id).unwrap().world(),
            loaded.world
        );
        assert_eq!(binding.mesh_indices, mesh_range);
        let image = surface_image(&assets, registration.id);
        assert!(!images.contains(&image));
        images.push(image);
        assert_eq!(
            assets
                .image(image)
                .unwrap()
                .prepared
                .as_ref()
                .unwrap()
                .levels[0]
                .width,
            width
        );
        assert_eq!(
            assets.image(image).unwrap().rgba,
            assets.image(original).unwrap().rgba
        );
    }
    assert_eq!(
        assets.image(original).unwrap().preparation_revision,
        revision
    );
    assert_eq!(
        assets
            .image(original)
            .unwrap()
            .prepared
            .as_ref()
            .unwrap()
            .levels[0]
            .rgba
            .as_ref(),
        original_pixels
    );
    assert_eq!(surface_image(&assets, loaded.presentations[3].id), original);
    std::fs::remove_dir_all(root).map_err(|e| e.to_string())?;
    Ok(())
}

fn draw_registered_profiles(
    assets: &Assets,
    world: qa_render::world::WorldId,
    profiles: &[LoadedWorldPresentation],
) -> Result<Vec<u32>, String> {
    let width = profiles.len() as u32 * 8;
    let mut cpu = CpuBackend::load_with_assets(width, 8, assets).map_err(str::to_owned)?;
    let mut frontend = FrontEnd::load(Limits::default()).map_err(str::to_owned)?;
    let mut frame = frontend.begin_frame([0, 0, 0, 255]).ok_or("frame")?;
    for (seat, profile) in profiles.iter().enumerate() {
        assert!(frame.add_world(world, profile.id, &[VisibleSurface::default()]));
        let mut view = Refdef {
            viewport: Viewport {
                x: seat as u32 * 8,
                width: 8,
                height: 8,
                ..Default::default()
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
        profile.defaults.apply(&mut view);
        assert!(frame.render_scene(view, &[], assets));
    }
    let packet = frame.finish();
    assert_eq!(cpu.render(&packet, assets).rejected, 0);
    Ok(cpu.pixels().to_vec())
}

#[test]
fn one_geometry_presents_explicit_palette_variants_in_both_seat_orders() -> Result<(), String> {
    let root = resources("same-world-palettes")?;
    let vfs = mount(&root)?;
    let bytes = map_bytes(1);
    let mut map = legacy_map(&bytes, 1)?;
    map.surface_edges.reverse();
    for reversed in [false, true] {
        let mut assets = Assets::load().map_err(|e| e.to_string())?;
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
        let mut profiles = [
            WorldPresentationProfile::native(RuleSetId::Quake, Some(source)),
            WorldPresentationProfile::native(RuleSetId::Quake2, Some(target)),
        ];
        if reversed {
            profiles.reverse();
        }
        let loaded = load_world(
            &vfs,
            &map,
            &mut assets,
            Some(source),
            &profiles.map(Into::into),
            WorldLoadOptions::default(),
        )
        .map_err(|e| format!("palette variants: {e:?}"))?;
        assert_eq!(assets.worlds().len(), 1);
        assert_eq!(assets.models().len(), 2);
        let mut expected = Vec::new();
        for profile in &loaded.presentations {
            let image = assets.image(surface_image(&assets, profile.id)).unwrap();
            let expected_index = if profile.profile.rules == RuleSetId::Quake {
                10
            } else {
                42
            };
            assert!(
                image
                    .indexed
                    .as_ref()
                    .unwrap()
                    .mip(0)
                    .unwrap()
                    .indices()
                    .iter()
                    .all(|&index| index == expected_index)
            );
            expected.push(draw_registered_profiles(
                &assets,
                loaded.world,
                &[*profile],
            )?);
        }
        let joined = draw_registered_profiles(&assets, loaded.world, &loaded.presentations)?;
        for (seat, oracle) in expected.iter().enumerate() {
            let actual: Vec<_> = joined
                .chunks_exact(16)
                .flat_map(|row| row[seat * 8..seat * 8 + 8].iter().copied())
                .collect();
            assert_eq!(&actual, oracle);
        }
    }
    std::fs::remove_dir_all(root).map_err(|e| e.to_string())?;
    Ok(())
}

fn modern_map(bytes: &[u8]) -> Result<Map<'_>, String> {
    let mut map = legacy_map(bytes, 3)?;
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
    Ok(map)
}

#[test]
fn foreign_presentation_keeps_q3_geometry_coordinates_and_source_overbright() -> Result<(), String>
{
    let root = resources("q3-source")?;
    std::fs::write(
        root.join("scripts/test.shader"),
        "textures/modern {\n q3map_sun 1 1 1 200 0 0\n { map textures/modern }\n }",
    )
    .unwrap();
    let vfs = mount(&root)?;
    let bytes = map_bytes(3);
    let map = modern_map(&bytes)?;
    let mut assets = Assets::load().map_err(|e| format!("assets: {e:?}"))?;
    for rules in [RuleSetId::Quake3, RuleSetId::Quake, RuleSetId::Quake2] {
        let loaded = load_native_world(&vfs, &map, &mut assets, rules, WorldLoadOptions::default())
            .map_err(|e| format!("material: {e:?}"))?;
        let world = assets.world(loaded.world).unwrap();
        assert_eq!(
            assets
                .world_presentation(loaded.presentations[0].id)
                .unwrap()
                .sun_direction(),
            Vec3([1., 0., 0.])
        );
        assert_eq!(
            world.geometry().surfaces[0].texture_coordinates,
            TextureCoordinates::Normalized
        );
        let vertex = world.geometry().vertices[0].vertex;
        assert_eq!(vertex.texcoord, [0.25, 0.75]);
        assert_eq!(vertex.lightmap_coord, [0.125, 0.875]);
        assert_eq!(vertex.color, [40, 80, 120, 255]);
        let image = assets
            .image(surface_image(&assets, loaded.presentations[0].id))
            .unwrap();
        // Source Q3 ignores the top-origin bit, even with a Q1/Q2 upload
        // policy. A presentation-selected decoder would return [1, 2, 3].
        assert_eq!(&image.rgba[..4], &[30, 60, 90, 255]);
        assert_eq!(image.indexed.is_none(), rules == RuleSetId::Quake3);
        assert_eq!(
            matches!(loaded.presentations[0].defaults.cpu, CpuPresentation::Rgb),
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

#[test]
fn ordered_native_requests_keep_named_default_bodies_for_hidden_surfaces() -> Result<(), String> {
    use qa_formats::bsp::Fog;
    use qa_render::{ShaderBindingId, world::geometry::LightSource};
    let root = resources("native-requests")?;
    std::fs::write(
        root.join("scripts/test.shader"),
        r#"
        textures/fog { surfaceparm fog fogParms ( .1 .2 .3 ) 128 }
        textures/modern { { map $whiteimage } }
        textures/hidden { surfaceparm nodraw { map $whiteimage } }
        textures/stageless { surfaceparm nodraw }
    "#,
    )
    .map_err(|error| error.to_string())?;
    let vfs = mount(&root)?;
    let bytes = map_bytes(3);
    let mut map = modern_map(&bytes)?;
    map.fogs = vec![
        Fog {
            name: b"Textures/Fog",
            brush: -1,
            visible_side: -1,
        },
        Fog {
            name: b"textures/missing_fog",
            brush: -1,
            visible_side: -1,
        },
    ];
    map.shaders.extend([
        Shader {
            name: b"textures/missing",
            surface_flags: 0x80,
            content_flags: 0,
        },
        Shader {
            name: b"textures/hidden",
            surface_flags: 0x80,
            content_flags: 0,
        },
        Shader {
            name: b"textures/stageless",
            surface_flags: 0x80,
            content_flags: 0,
        },
    ]);
    let face = map.surfaces[0];
    for shader in [1, 0, 2, 3] {
        map.surfaces.push(Surface {
            shader: Some(shader),
            ..face
        });
    }
    map.leaf_faces = (0..5).collect();
    map.leaves[0].faces.count = 5;
    map.models[0].faces.count = 1;
    map.models.push(Model {
        faces: IndexSpan { first: 1, count: 4 },
        ..map.models[0]
    });
    let mut assets = Assets::load().map_err(|error| format!("{error:?}"))?;
    let loaded = load_native_world(
        &vfs,
        &map,
        &mut assets,
        RuleSetId::Quake3,
        WorldLoadOptions::default(),
    )
    .map_err(|error| format!("{error:?}"))?;
    let presentation = assets
        .world_presentation(loaded.presentations[0].id)
        .ok_or("presentation")?;
    let registrations = presentation.shader_registrations();
    let names: Vec<_> = registrations
        .iter()
        .map(|registration| assets.name(registration.key).unwrap().to_ascii_lowercase())
        .collect();
    assert_eq!(
        names,
        [
            "textures/fog",
            "textures/missing_fog",
            "textures/modern",
            "textures/missing",
            "textures/modern",
            "textures/hidden",
            "textures/stageless",
        ]
    );
    assert_eq!(registrations[2].shader, presentation.bindings()[0].shader);
    assert_eq!(registrations[4].shader, presentation.bindings()[2].shader);
    assert_eq!(registrations[2], registrations[4]);
    assert_eq!(
        assets
            .shader_binding(registrations[0].shader)
            .unwrap()
            .light_source,
        LightSource::None
    );
    assert!(
        assets
            .material(
                assets
                    .shader_binding(registrations[0].shader)
                    .unwrap()
                    .material
            )
            .unwrap()
            .settings
            .fog
            .is_some()
    );
    for index in [1, 3] {
        assert_ne!(registrations[index].shader, ShaderBindingId(0));
        assert_eq!(
            registrations[index].defaulted,
            Some(qa_render::NativeDefaultKind::MissingImage)
        );
        let material = assets
            .material(
                assets
                    .shader_binding(registrations[index].shader)
                    .unwrap()
                    .material,
            )
            .unwrap();
        assert!(material.stages.is_empty());
        assert_eq!(material.settings.sort, 7.0);
        assert!(material.settings.fog.is_none());
    }
    assert_ne!(registrations[5].shader, ShaderBindingId(0));
    assert!(registrations[6].defaulted.is_some());
    assert_ne!(
        registrations[6].defaulted,
        Some(qa_render::NativeDefaultKind::MissingImage)
    );
    let stageless = assets
        .material(
            assets
                .shader_binding(registrations[6].shader)
                .unwrap()
                .material,
        )
        .unwrap();
    assert!(stageless.stages.is_empty());
    assert_eq!(stageless.settings.sort, 7.0);
    for surface in [1, 3, 4] {
        assert_eq!(presentation.bindings()[surface].shader, ShaderBindingId(0));
        assert!(assets.world(loaded.world).unwrap().geometry().surfaces[surface].no_draw);
    }
    assert_eq!(
        loaded
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.starts_with("native shader registration omitted:"))
            .count(),
        0
    );
    std::fs::remove_dir_all(root).map_err(|error| error.to_string())?;
    Ok(())
}

#[test]
fn unreferenced_native_page_zero_is_real_converted_resource_and_late_metadata_rolls_back()
-> Result<(), String> {
    use qa_render::material::world_load::append_world_presentation;
    let root = resources("unreferenced-page-zero")?;
    let vfs = mount(&root)?;
    let mut bytes = map_bytes(3);
    let header = 8 + 17 * 8;
    bytes[8 + 14 * 8 + 4..8 + 14 * 8 + 8].copy_from_slice(&(2u32 * 128 * 128 * 3).to_le_bytes());
    bytes.truncate(header);
    bytes.extend(std::iter::repeat_n([1, 2, 3], 128 * 128).flatten());
    bytes.extend(std::iter::repeat_n([7, 11, 13], 128 * 128).flatten());
    let mut map = modern_map(&bytes)?;
    map.surfaces[0].lightmap = 1;
    let mut assets = Assets::load().map_err(|error| format!("{error:?}"))?;
    let loaded = load_native_world(
        &vfs,
        &map,
        &mut assets,
        RuleSetId::Quake3,
        WorldLoadOptions::default(),
    )
    .map_err(|error| format!("{error:?}"))?;
    let presentation = assets
        .world_presentation(loaded.presentations[0].id)
        .ok_or("presentation")?;
    let page_zero = presentation.native_page0().ok_or("page zero")?;
    let source = assets
        .shader_binding(presentation.bindings()[0].shader)
        .ok_or("source binding")?;
    assert_ne!(page_zero, source.lightmap);
    assert!(
        source.lightmap.0 < page_zero.0,
        "visible atlas order is retained"
    );
    assert_eq!(
        &assets.image(source.lightmap).unwrap().rgba[..4],
        &[28, 44, 52, 255]
    );
    assert_eq!(
        &assets.image(page_zero).unwrap().rgba[..4],
        &[4, 8, 12, 255]
    );
    let prefix = assets.counts();
    let old_registration = presentation.shader_registrations()[0];
    let old_mesh = assets.world(loaded.world).unwrap().mesh();
    let mut profile = loaded.presentations[0].profile;
    profile.image_settings.picmip = 0;
    let candidate = append_world_presentation(&vfs, &mut assets, loaded.world, profile.into())
        .map_err(|error| format!("{error:?}"))?;
    let candidate_id = candidate.presentations[0].id;
    assert_eq!(assets.world(loaded.world).unwrap().mesh(), old_mesh);
    assert_eq!(
        assets
            .world_presentation(candidate_id)
            .unwrap()
            .shader_registrations()
            .len(),
        1
    );
    assets
        .discard_registration_suffix(prefix)
        .map_err(str::to_owned)?;
    assert_eq!(assets.counts(), prefix);
    assert_eq!(
        assets.world(loaded.world).unwrap().presentations(),
        &[loaded.presentations[0].id]
    );
    assert_eq!(
        assets
            .world_presentation(loaded.presentations[0].id)
            .unwrap()
            .shader_registrations(),
        &[old_registration]
    );
    assert_eq!(
        &assets.image(page_zero).unwrap().rgba[..4],
        &[4, 8, 12, 255]
    );
    let retry = append_world_presentation(&vfs, &mut assets, loaded.world, profile.into())
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(retry.presentations[0].id, candidate_id);
    let admitted = assets.world_presentation(candidate_id).unwrap();
    assert!(
        assets
            .image(admitted.native_page0().ok_or("retry page zero")?)
            .is_some()
    );
    assert_eq!(
        admitted.shader_registrations()[0].shader,
        admitted.bindings()[0].shader
    );
    std::fs::remove_dir_all(root).map_err(|error| error.to_string())?;
    Ok(())
}

#[test]
fn native_hidden_out_of_range_lightmap_registers_vertex_lighting_before_draw_skip()
-> Result<(), String> {
    use qa_render::world::geometry::LightSource;
    let root = resources("hidden-unavailable-lightmap")?;
    let vfs = mount(&root)?;
    let mut bytes = map_bytes(3);
    bytes[8 + 14 * 8 + 4..8 + 14 * 8 + 8].copy_from_slice(&0u32.to_le_bytes());
    bytes.truncate(8 + 17 * 8);
    let mut map = modern_map(&bytes)?;
    map.shaders[0].surface_flags = 0x80;
    let mut assets = Assets::load().map_err(|error| format!("{error:?}"))?;
    let loaded = load_native_world(
        &vfs,
        &map,
        &mut assets,
        RuleSetId::Quake3,
        WorldLoadOptions::default(),
    )
    .map_err(|error| format!("{error:?}"))?;
    let presentation = assets
        .world_presentation(loaded.presentations[0].id)
        .ok_or("presentation")?;
    assert_eq!(presentation.native_page0(), None);
    assert_eq!(
        presentation.bindings()[0].shader,
        qa_render::ShaderBindingId(0),
        "nodraw surfaces have no draw binding"
    );
    let registration = presentation.shader_registrations()[0];
    assert_ne!(registration.shader, qa_render::ShaderBindingId(0));
    let shader = assets
        .shader_binding(registration.shader)
        .ok_or("registration")?;
    assert_eq!(shader.light_source, LightSource::Vertex);
    let material = assets.material(shader.material).ok_or("material")?;
    assert_eq!(material.stages.len(), 1);
    assert_eq!(material.stages[0].rgb_gen, RgbGen::ExactVertex);
    assert_ne!(material.stages[0].texture, StageTexture::Lightmap);
    std::fs::remove_dir_all(root).map_err(|error| error.to_string())?;
    Ok(())
}

#[test]
fn named_outcomes_keep_requested_finished_modes_and_typed_native_defaults() -> Result<(), String> {
    use qa_render::{
        NativeDefaultKind,
        material::world_load::{NamedShaderLighting, WorldMaterialError, compile_named_shader},
        shader::{ShaderSource, parse_sources},
        world::geometry::LightSource,
    };
    let root = resources("named-outcomes")?;
    let vfs = mount(&root)?;
    let catalog = parse_sources(&[ShaderSource {
        name: "typed.shader",
        bytes: br#"
            textures/plain { { map $whiteimage } }
            textures/lit { { map $lightmap } { map $whiteimage blendFunc filter } }
            textures/authored_missing { { map textures/not_present.tga } }
            textures/invalid { surfaceparm nodraw }
            textures/unsupported { deformVertexes autosprite { map $whiteimage } }
        "#,
    }]);
    let mut assets = Assets::load().map_err(|error| format!("{error:?}"))?;
    let lightmap = assets
        .register_image(1, 1, &[127; 4])
        .map_err(str::to_owned)?;
    let mut images = Images::new(
        &vfs,
        &mut assets,
        RuleSetId::Quake3,
        RuleSetId::Quake3,
        ImageSettings::native(RuleSetId::Quake3),
        None,
    )
    .map_err(|error| format!("{error:?}"))?;
    let mut registrations = Vec::new();
    for source in [
        LightSource::Page(0),
        LightSource::Vertex,
        LightSource::Page(0),
    ] {
        registrations.push(
            compile_named_shader(
                &catalog,
                b"Textures/Plain.tga",
                true,
                NamedShaderLighting { source, lightmap },
                &mut images,
            )
            .map_err(|error| format!("{error:?}"))?,
        );
    }
    assert_eq!(
        registrations
            .iter()
            .map(|row| row.requested)
            .collect::<Vec<_>>(),
        [
            LightSource::Page(0),
            LightSource::Vertex,
            LightSource::Page(0)
        ]
    );
    assert_eq!(
        registrations
            .iter()
            .map(|row| row.finished)
            .collect::<Vec<_>>(),
        [LightSource::None, LightSource::Vertex, LightSource::None]
    );
    assert_eq!(
        registrations.len(),
        3,
        "ordered occurrences survive structural dedup"
    );
    assert_eq!(registrations[0].shader, registrations[2].shader);
    assert!(registrations.iter().all(|row| row.defaulted.is_none()));
    let lit = compile_named_shader(
        &catalog,
        b"textures/lit",
        true,
        NamedShaderLighting {
            source: LightSource::Page(0),
            lightmap,
        },
        &mut images,
    )
    .map_err(|error| format!("{error:?}"))?;
    assert_eq!(lit.finished, LightSource::Page(0));
    for source in [
        LightSource::Page(0),
        LightSource::Vertex,
        LightSource::TwoDimensional,
    ] {
        let default = compile_named_shader(
            &catalog,
            b"textures/implicit_missing",
            true,
            NamedShaderLighting { source, lightmap },
            &mut images,
        )
        .map_err(|error| format!("{error:?}"))?;
        assert_eq!(default.defaulted, Some(NativeDefaultKind::MissingImage));
        assert_eq!(default.requested, source);
        assert_eq!(
            default.finished,
            if source == LightSource::Page(0) {
                LightSource::None
            } else {
                source
            }
        );
        assert_ne!(default.shader, qa_render::ShaderBindingId(0));
        let material = images
            .assets
            .material(
                images
                    .assets
                    .shader_binding(default.shader)
                    .unwrap()
                    .material,
            )
            .unwrap();
        assert!(material.stages.is_empty());
        assert_eq!(material.settings.sort, 7.0);
        assert!(material.settings.fog.is_none() && material.settings.sky.is_none());
        let settings = material.settings;
        assert!(
            images
                .assets
                .register_material("textures/implicit_missing", &[], settings)
                .is_err(),
            "the public material boundary still rejects accidental empty bodies"
        );
    }
    for (raw, sort) in [
        (b"textures/authored_missing".as_slice(), 3.0),
        (b"textures/invalid".as_slice(), 7.0),
    ] {
        let registration = compile_named_shader(
            &catalog,
            raw,
            true,
            NamedShaderLighting {
                source: LightSource::None,
                lightmap,
            },
            &mut images,
        )
        .map_err(|error| format!("{error:?}"))?;
        assert!(registration.defaulted.is_some());
        assert_ne!(
            registration.defaulted,
            Some(NativeDefaultKind::MissingImage)
        );
        let body = images
            .assets
            .material(
                images
                    .assets
                    .shader_binding(registration.shader)
                    .unwrap()
                    .material,
            )
            .unwrap();
        assert!(body.stages.is_empty());
        assert_eq!(body.settings.sort, sort);
    }
    for raw in [b"textures/unsupported".as_slice()] {
        let counts = images.assets.counts();
        let result = compile_named_shader(
            &catalog,
            raw,
            true,
            NamedShaderLighting {
                source: LightSource::None,
                lightmap,
            },
            &mut images,
        );
        assert!(matches!(
            result,
            Err(WorldMaterialError::Resource(_)) | Err(WorldMaterialError::Shader(_))
        ));
        assert_eq!(images.assets.counts(), counts);
    }
    std::fs::remove_dir_all(root).map_err(|error| error.to_string())?;
    Ok(())
}

#[test]
fn implicit_missing_world_shader_keeps_named_body_and_draws_global_fallback() -> Result<(), String>
{
    use qa_render::NativeDefaultKind;
    let root = resources("missing-visible-world")?;
    let vfs = mount(&root)?;
    let bytes = map_bytes(3);
    let mut map = modern_map(&bytes)?;
    // Present the authored front face to the existing above-plane camera.
    map.indices = vec![0, 2, 1, 0, 3, 2];
    map.shaders[0].name = b"textures/missing_visible";
    let mut assets = Assets::load().map_err(|error| format!("{error:?}"))?;
    let loaded = load_native_world(
        &vfs,
        &map,
        &mut assets,
        RuleSetId::Quake3,
        WorldLoadOptions::default(),
    )
    .map_err(|error| format!("{error:?}"))?;
    let presentation = assets
        .world_presentation(loaded.presentations[0].id)
        .unwrap();
    let registration = presentation.shader_registrations()[0];
    assert_eq!(
        registration.defaulted,
        Some(NativeDefaultKind::MissingImage)
    );
    assert_ne!(registration.shader, presentation.bindings()[0].shader);
    let fallback = presentation.native_default().unwrap();
    assert_ne!(fallback, qa_render::ShaderBindingId(0));
    assert_eq!(presentation.bindings()[0].shader, fallback);
    let shader = assets.shader_binding(fallback).unwrap();
    let material = assets.material(shader.material).unwrap();
    assert_eq!(assets.name(material.name), Some("<default>"));
    let qa_render::assets::StageTexture::Image(image) = material.stages[0].texture else {
        panic!("global default image")
    };
    assert_eq!(
        &assets.image(image).unwrap().rgba[(8 * 16 + 8) * 4..(8 * 16 + 8) * 4 + 4],
        &[32; 4]
    );
    assert_ne!(registration.shader, qa_render::ShaderBindingId(0));
    assert_eq!(
        assets
            .world(loaded.world)
            .unwrap()
            .geometry()
            .surfaces
            .len(),
        1
    );
    let pixels = draw_registered_profiles(&assets, loaded.world, &loaded.presentations)?;
    assert!(
        pixels
            .iter()
            .any(|&pixel| pixel != u32::from_le_bytes([0, 0, 0, 255])),
        "the source world draws the existing global fallback rather than its named zero-pass body"
    );
    std::fs::remove_dir_all(root).map_err(|error| error.to_string())?;
    Ok(())
}

#[test]
fn q3_default_world_uses_independent_indexed_presentation_without_dropping_native_size()
-> Result<(), String> {
    let root = resources("indexed-default-world")?;
    let mut shades: Vec<_> = (0..64).flat_map(|_| 0..=255u8).collect();
    shades.push(32);
    std::fs::write(root.join("gfx/colormap.lmp"), shades).map_err(|error| error.to_string())?;
    let vfs = mount(&root)?;
    let bytes = map_bytes(3);
    let mut map = modern_map(&bytes)?;
    // Present the authored front face to the existing above-plane camera.
    map.indices = vec![0, 2, 1, 0, 3, 2];
    map.shaders[0].name = b"textures/missing_indexed";
    let mut assets = Assets::load().map_err(|error| format!("{error:?}"))?;
    let palette = load_palette(
        &vfs,
        &mut assets,
        PaletteSource::native(RuleSetId::Quake).unwrap(),
    )
    .map_err(|error| format!("{error:?}"))?;
    let mut profile = WorldPresentationProfile::native(RuleSetId::Quake, Some(palette));
    profile.image_settings.picmip = 2;
    let loaded = load_world(
        &vfs,
        &map,
        &mut assets,
        None,
        &[profile.into()],
        WorldLoadOptions::default(),
    )
    .map_err(|error| format!("{error:?}"))?;
    let presentation = assets
        .world_presentation(loaded.presentations[0].id)
        .unwrap();
    let fallback = presentation.native_default().unwrap();
    assert_eq!(presentation.bindings()[0].shader, fallback);
    let material = assets
        .material(assets.shader_binding(fallback).unwrap().material)
        .unwrap();
    let qa_render::assets::StageTexture::Image(id) = material.stages[0].texture else {
        panic!("default image")
    };
    let image = assets.image(id).unwrap();
    assert_eq!(
        (
            image.prepared.as_ref().unwrap().levels[0].width,
            image.prepared.as_ref().unwrap().levels[0].height
        ),
        (16, 16)
    );
    assert_eq!(image.indexed.as_ref().unwrap().transparent_index(), None);
    let pixels = draw_registered_profiles(&assets, loaded.world, &loaded.presentations)?;
    assert!(
        pixels
            .iter()
            .any(|&pixel| pixel != u32::from_le_bytes([0, 0, 0, 255]))
    );
    std::fs::remove_dir_all(root).map_err(|error| error.to_string())?;
    Ok(())
}

#[path = "support/world_surfaces.rs"]
mod indexed_world_fixture;

#[test]
fn indexed_foreground_rejection_and_cutout_preserve_background_coverage() {
    for cutout in [false, true] {
        use qa_render::{
            ShaderBinding,
            assets::{MaterialSettings, Stage, Vertex},
            shader::{AlphaGen, Cull},
            world::{SurfaceMaterial, WorldPresentationInput, geometry::WorldVertex},
        };
        let mut assets = Assets::load().unwrap();
        let colors: Vec<_> = (0..=255u8).flat_map(|i| [i; 3]).collect();
        let shades: Vec<_> = (0..64).flat_map(|_| 0..=255u8).collect();
        let palette = assets
            .register_palette(
                qa_render::surface_cache::PaletteLighting::load(&colors, &shades, None, 256)
                    .unwrap(),
            )
            .unwrap();
        let front = if cutout {
            assets
                .register_indexed_image(
                    IndexedTexture::load_base(1, 1, &[255], Some(255)).unwrap(),
                    palette,
                )
                .unwrap()
        } else {
            assets.register_image(1, 1, &[200, 0, 0, 255]).unwrap()
        };
        let back = assets
            .register_indexed_image(
                IndexedTexture::load_base(1, 1, &[77], None).unwrap(),
                palette,
            )
            .unwrap();
        let material = |assets: &mut Assets, name, image| {
            assets
                .register_material(
                    name,
                    &[Stage {
                        texture: StageTexture::Image(image),
                        rgb_gen: RgbGen::IdentityLighting,
                        alpha_gen: AlphaGen::Identity,
                        ..Default::default()
                    }],
                    MaterialSettings {
                        cull: Cull::None,
                        ..Default::default()
                    },
                )
                .unwrap()
        };
        let front = material(&mut assets, "unsupported foreground", front);
        let back = material(&mut assets, "indexed background", back);
        let (mut geometry, visibility) = indexed_world_fixture::inputs(2);
        geometry.vertices = [1f32, 2.]
            .into_iter()
            .flat_map(|x| {
                [[x, -x, -x], [x, 0., x], [x, x, -x]].map(|position| WorldVertex {
                    vertex: Vertex {
                        position: Vec3(position),
                        normal: Vec3([-1., 0., 0.]),
                        texcoord: [0.5; 2],
                        ..Default::default()
                    },
                })
            })
            .collect();
        geometry.indices = vec![0, 1, 2, 3, 4, 5];
        geometry
            .boundaries
            .push(qa_core::primitives::IndexSpan { first: 3, count: 3 });
        geometry.surfaces[1].vertices.first = 3;
        geometry.surfaces[1].indices.first = 3;
        geometry.surfaces[1].boundaries.first = 1;
        let surfaces = [front, back].map(|material| SurfaceMaterial {
            shader: ShaderBinding::material(
                material,
                qa_render::world::geometry::LightSource::None,
            ),
            ..Default::default()
        });
        let world = assets
            .register_world(
                geometry,
                visibility,
                &[WorldPresentationInput::new(&surfaces)],
            )
            .unwrap();
        let presentation = assets.world(world).unwrap().presentations()[0];
        let mut cpu = CpuBackend::load_with_assets(16, 16, &assets).unwrap();
        let mut frontend = FrontEnd::load(Default::default()).unwrap();
        let mut frame = frontend.begin_frame([0, 0, 0, 255]).unwrap();
        assert!(frame.add_world(
            world,
            presentation,
            &[
                VisibleSurface {
                    surface: 0,
                    depth_key: 0
                },
                VisibleSurface {
                    surface: 1,
                    depth_key: 0
                }
            ]
        ));
        assert!(frame.render_scene(
            Refdef {
                viewport: Viewport {
                    width: 16,
                    height: 16,
                    ..Default::default()
                },
                fov: [90.; 2],
                near: 0.1,
                far: 16.,
                cpu_presentation: CpuPresentation::Indexed {
                    palette,
                    lighting: IndexedLighting::Gray,
                    ambient: 0,
                    fullbright: true
                },
                ..Default::default()
            },
            &[],
            &assets
        ));
        let stats = cpu.render(&frame.finish(), &assets);
        if cutout {
            assert_eq!(stats.rejected, 0);
        } else {
            assert!(stats.rejected > 0);
        }
        assert_eq!(
            cpu.pixels()[8 * 16 + 8],
            u32::from_le_bytes([77, 77, 77, 255])
        );
    }
}

#[test]
fn legacy_worlds_use_the_shared_global_default_after_authored_shader_parse_failure() {
    let root = resources("legacy-native-default").unwrap();
    std::fs::write(
        root.join("scripts/test.shader"),
        br#"
        textures/wall { { map $whiteimage
                        }
                        bogus
        }
    "#,
    )
    .unwrap();
    let vfs = mount(&root).unwrap();
    for family in [1, 2] {
        let bytes = map_bytes(family);
        let map = legacy_map(&bytes, family).unwrap();
        let mut assets = Assets::load().unwrap();
        let loaded = load_native_world(
            &vfs,
            &map,
            &mut assets,
            map.bsp.format.rule_set(),
            Default::default(),
        )
        .unwrap();
        let presentation = assets
            .world_presentation(loaded.presentations[0].id)
            .unwrap();
        let registration = presentation.shader_registrations()[0];
        assert!(registration.defaulted.is_some());
        let named = assets
            .material(assets.shader_binding(registration.shader).unwrap().material)
            .unwrap();
        assert_eq!(named.stages.len(), 1);
        assert_eq!(named.stages[0].texture, StageTexture::Image(ImageId(0)));
        let fallback = presentation.native_default().unwrap();
        assert_eq!(presentation.bindings()[0].shader, fallback);
        assert_ne!(fallback, registration.shader);
        let body = assets
            .material(assets.shader_binding(fallback).unwrap().material)
            .unwrap();
        assert_eq!(assets.name(body.name), Some("<default>"));
        let StageTexture::Image(image) = body.stages[0].texture else {
            panic!("default bitmap")
        };
        assert!(assets.image(image).unwrap().indexed.is_some());
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[path = "world_materials/ordered.rs"]
mod ordered;
