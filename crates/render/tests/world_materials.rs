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
        .map(|source| load_palette((vfs).read_view(None), assets, source))
        .transpose()?;
    let target = PaletteSource::native(rules);
    let target_palette = if target == source {
        source_palette
    } else {
        target
            .map(|source| load_palette((vfs).read_view(None), assets, source))
            .transpose()?
    };
    load_world(
        vfs,
        map,
        assets,
        source_palette,
        &[WorldPresentationProfile::native(rules, target_palette).into()],
        options,
        false,
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
        let binding = assets
            .shader_binding(
                assets
                    .surface_shader(loaded.presentations[0].id, 0)
                    .unwrap(),
            )
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
                        .surface_shader(loaded.presentations[0].id, 1)
                        .unwrap()
                )
                .unwrap()
                .lightmap_region
                .is_none()
        );
        assert_eq!(
            assets
                .shader_binding(
                    assets
                        .surface_shader(loaded.presentations[0].id, 1)
                        .unwrap()
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
            assert_eq!(first.texture_scale, [1.0 / 16.0; 2]);
            let first = assets
                .shader_binding(
                    assets
                        .surface_shader(loaded.presentations[0].id, 0)
                        .unwrap(),
                )
                .ok_or("first shader")?;
            let second = assets
                .shader_binding(
                    assets
                        .surface_shader(loaded.presentations[0].id, 1)
                        .unwrap(),
                )
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
        let binding = assets
            .shader_binding(
                assets
                    .surface_shader(loaded.presentations[0].id, 0)
                    .unwrap(),
            )
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
                .shader_binding(assets.surface_shader(presentation, 0).unwrap())
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
            assert!(frame.render_scene(
                view,
                &[],
                &assets,
                qa_render::shader::reference::ShaderView::direct()
            ));
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
        (&vfs).read_view(None),
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
        false,
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
        assert!(frame.render_scene(
            view,
            &[],
            assets,
            qa_render::shader::reference::ShaderView::direct()
        ));
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
            (&vfs).read_view(None),
            &mut assets,
            PaletteSource::native(RuleSetId::Quake).unwrap(),
        )
        .map_err(|e| format!("source palette: {e:?}"))?;
        let target = load_palette(
            (&vfs).read_view(None),
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
            false,
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
fn world_lighting_rebuild_restores_authored_bytes_and_keeps_old_presentations_immutable()
-> Result<(), String> {
    use qa_render::{
        lighting::LightGrid,
        material::world_load::append_world_presentation,
        world::{WorldLighting, WorldRenderSettings},
    };
    let root = resources("world-lighting-rebuild")?;
    let vfs = mount(&root)?;
    let bytes = map_bytes(3);
    let mut map = modern_map(&bytes)?;
    for vertex in &mut map.vertices {
        vertex.color = [200, 100, 50, 17];
    }
    let mut assets = Assets::load().unwrap();
    let loaded = load_native_world(
        &vfs,
        &map,
        &mut assets,
        RuleSetId::Quake3,
        WorldLoadOptions::default(),
    )
    .map_err(|error| format!("initial world: {error:?}"))?;
    let world = loaded.world;
    let original = loaded.presentations[0];
    let source_mesh = assets.world(world).unwrap().mesh();
    let grid = LightGrid::load(
        Bounds {
            mins: Vec3::default(),
            maxs: Vec3([1.; 3]),
        },
        Vec3([1.; 3]),
        [qa_formats::bsp::LightGridPoint {
            ambient: [200, 100, 50],
            directed: [20, 30, 40],
            latitude: 0,
            longitude: 0,
        }; 8]
            .into_iter(),
        original.lighting.shift(),
    )
    .map_err(str::to_owned)?;
    assets.set_light_grid(world, Some(grid)).unwrap();
    let old_grid =
        assets
            .world(world)
            .unwrap()
            .light_grid()
            .unwrap()
            .sample(Vec3::default(), 1., 1.);
    assert_eq!(old_grid.ambient, [255., 127., 63.]);
    let old_binding = *assets
        .shader_binding(assets.surface_shader(original.id, 0).unwrap())
        .unwrap();
    assert_eq!(
        &assets.image(old_binding.lightmap).unwrap().rgba[..4],
        &[255; 4]
    );
    let old_vertices = assets.model(source_mesh).unwrap().vertices.to_vec();
    assert_eq!(old_vertices[0].color, [255, 127, 63, 17]);
    let retained = assets.counts();
    let mut cpu = CpuBackend::load_with_assets(16, 16, &assets).unwrap();
    let original_config = cpu.raster_config();
    let settings = WorldRenderSettings {
        lighting: WorldLighting::from_requests(0, 0, None).unwrap(),
        ..Default::default()
    };
    let rebuild = assets.rebuild_worlds(&[world], settings).unwrap();
    assert_eq!(rebuild.worlds().collect::<Vec<_>>(), [world]);
    assert_eq!(assets.counts(), retained);
    assert_eq!(assets.world(world).unwrap().mesh(), source_mesh);
    assert_eq!(
        assets.model(source_mesh).unwrap().vertices[0].color,
        [200, 100, 50, 17]
    );
    let light = assets
        .world(world)
        .unwrap()
        .light_grid()
        .unwrap()
        .sample(Vec3::default(), 1., 1.);
    assert_eq!(light.ambient, [200., 100., 50.]);
    assert_eq!(light.directed, [20., 30., 40.]);
    let changed =
        append_world_presentation(&vfs, &mut assets, world, original.profile.into(), false)
            .map_err(|error| format!("lighting presentation: {error:?}"))?
            .presentations[0];
    assert_ne!(changed.id, original.id);
    assert_eq!(changed.profile, original.profile);
    assert_eq!(changed.lighting, settings.lighting);
    let binding = assets
        .shader_binding(assets.surface_shader(changed.id, 0).unwrap())
        .unwrap();
    assert_ne!(binding.lightmap, old_binding.lightmap);
    assert_eq!(
        &assets.image(binding.lightmap).unwrap().rgba[..4],
        &[128, 128, 128, 255]
    );
    assert_eq!(
        &assets.image(old_binding.lightmap).unwrap().rgba[..4],
        &[255; 4]
    );
    assert_eq!(
        assets.world_presentation_registration(original.id),
        Some(original)
    );
    assert!(
        cpu.prepare_assets(
            &assets,
            Some(Limits {
                draws: 1,
                ..Default::default()
            }),
        )
        .is_err()
    );
    assert_eq!(cpu.raster_config(), original_config);
    rebuild.rollback(&mut assets).unwrap();
    assets.discard_registration_suffix(retained).unwrap();
    assert_eq!(assets.counts(), retained);
    assert_eq!(
        assets
            .world(world)
            .unwrap()
            .render_settings()
            .unwrap()
            .lighting,
        original.lighting
    );
    assert_eq!(
        assets.model(source_mesh).unwrap().vertices.as_ref(),
        old_vertices
    );
    assert_eq!(
        assets
            .world(world)
            .unwrap()
            .light_grid()
            .unwrap()
            .sample(Vec3::default(), 1., 1.),
        old_grid
    );

    let rebuild = assets.rebuild_worlds(&[world], settings).unwrap();
    let retry = append_world_presentation(&vfs, &mut assets, world, original.profile.into(), false)
        .map_err(|error| format!("retry presentation: {error:?}"))?
        .presentations[0];
    assert_eq!(retry, changed);
    cpu.prepare_assets(&assets, None).unwrap();
    rebuild.commit();
    let retained = assets.counts();
    let cached =
        append_world_presentation(&vfs, &mut assets, world, original.profile.into(), false)
            .map_err(|error| format!("cached presentation: {error:?}"))?;
    assert_eq!(cached.presentations[0], retry);
    assert_eq!(assets.counts(), retained);
    assert_eq!(assets.world(world).unwrap().mesh(), source_mesh);
    let rebuild = assets
        .rebuild_worlds(&[world], WorldRenderSettings::default())
        .unwrap();
    let restored =
        append_world_presentation(&vfs, &mut assets, world, original.profile.into(), false)
            .map_err(|error| format!("restored presentation: {error:?}"))?;
    assert_eq!(restored.presentations[0], original);
    assert_eq!(assets.counts(), retained);
    assert_eq!(
        assets.model(source_mesh).unwrap().vertices.as_ref(),
        old_vertices
    );
    cpu.prepare_assets(&assets, None).unwrap();
    rebuild.commit();
    std::fs::remove_dir_all(root).map_err(|error| error.to_string())?;
    Ok(())
}

#[test]
fn palette_translation_preserves_cutout_and_cloud_mask_indices() -> Result<(), String> {
    let root = resources("masks")?;
    let vfs = mount(&root)?;
    let mut assets = Assets::load().map_err(|e| format!("assets: {e:?}"))?;
    let source = load_palette(
        (&vfs).read_view(None),
        &mut assets,
        PaletteSource::native(RuleSetId::Quake).unwrap(),
    )
    .map_err(|e| format!("source palette: {e:?}"))?;
    let target = load_palette(
        (&vfs).read_view(None),
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
            None,
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
    assert_eq!(
        registrations[2].shader,
        presentation.surface_shader(0).unwrap()
    );
    assert_eq!(
        registrations[4].shader,
        presentation.surface_shader(2).unwrap()
    );
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
        assert_eq!(
            presentation.surface_shader(surface as u32).unwrap(),
            ShaderBindingId(0)
        );
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
        .shader_binding(presentation.surface_shader(0).unwrap())
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
    let candidate =
        append_world_presentation(&vfs, &mut assets, loaded.world, profile.into(), false)
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
    let retry = append_world_presentation(&vfs, &mut assets, loaded.world, profile.into(), false)
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
        admitted.surface_shader(0).unwrap()
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
        presentation.surface_shader(0).unwrap(),
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
                qa_render::material::ShaderCompilePolicy::default(),
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
        qa_render::material::ShaderCompilePolicy::default(),
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
            qa_render::material::ShaderCompilePolicy::default(),
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
            qa_render::material::ShaderCompilePolicy::default(),
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
            qa_render::material::ShaderCompilePolicy::default(),
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
    assert_ne!(registration.shader, presentation.surface_shader(0).unwrap());
    let fallback = presentation.native_default().unwrap();
    assert_ne!(fallback, qa_render::ShaderBindingId(0));
    assert_eq!(presentation.surface_shader(0).unwrap(), fallback);
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
        (&vfs).read_view(None),
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
        false,
    )
    .map_err(|error| format!("{error:?}"))?;
    let presentation = assets
        .world_presentation(loaded.presentations[0].id)
        .unwrap();
    let fallback = presentation.native_default().unwrap();
    assert_eq!(presentation.surface_shader(0).unwrap(), fallback);
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
            &assets,
            qa_render::shader::reference::ShaderView::direct()
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
        assert_eq!(presentation.surface_shader(0).unwrap(), fallback);
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

#[test]
fn generated_same_texture_legacy_variants_keep_their_cold_body_and_history_ordinal()
-> Result<(), String> {
    use qa_render::shader::reference::ShaderChoice;
    let root = resources("converted-variants")?;
    let bytes = map_bytes(2);
    let mut map = legacy_map(&bytes, 2)?;
    map.texture_info[1].name = map.texture_info[0].name;
    map.texture_info[1].texture = map.texture_info[0].texture;
    map.texture_info[1].flags = 64 | 32;
    let vfs = mount(&root)?;
    let mut assets = Assets::load().map_err(|error| format!("{error:?}"))?;
    let loaded = load_native_world(
        &vfs,
        &map,
        &mut assets,
        RuleSetId::Quake2,
        WorldLoadOptions::default(),
    )
    .map_err(|error| format!("converted materials: {error:?}"))?;
    let presentation = loaded.presentations[0].id;
    let receipts = assets
        .world_presentation(presentation)
        .ok_or("presentation")?
        .shader_registrations();
    assert_eq!(receipts.len(), 2);
    assert_eq!(receipts[0].key, receipts[1].key);
    let shaders = [0, 1].map(|surface| {
        let ShaderChoice::Converted {
            binding,
            occurrence,
        } = assets
            .surface_binding(presentation, surface)
            .unwrap()
            .shader
        else {
            panic!("generated legacy surface needs a converted choice");
        };
        assert_eq!(occurrence.0, surface);
        assert_eq!(assets.surface_shader(presentation, surface), Some(binding));
        binding
    });
    let first = assets
        .material(
            assets
                .shader_binding(shaders[0])
                .ok_or("first binding")?
                .material,
        )
        .ok_or("first material")?;
    let second = assets
        .material(
            assets
                .shader_binding(shaders[1])
                .ok_or("second binding")?
                .material,
        )
        .ok_or("second material")?;
    assert_ne!(
        assets.shader_binding(shaders[0]).unwrap().material,
        assets.shader_binding(shaders[1]).unwrap().material
    );
    assert!(first.stages[0].blend.is_none());
    assert!(first.stages[0].tcmods.iter().all(Option::is_none));
    assert!(second.stages[0].blend.is_some());
    assert!(
        second.stages[0]
            .tcmods
            .iter()
            .any(|modifier| matches!(modifier, Some(TcMod::Flow(_))))
    );
    std::fs::remove_dir_all(root).map_err(|error| error.to_string())
}

#[test]
fn world_profiles_normalize_lighting_before_the_fullscreen_finish_guard() -> Result<(), String> {
    use qa_render::{
        material::{
            MaterialFinishPolicy,
            world_load::{NamedShaderLighting, append_world_presentation, compile_named_shader},
        },
        shader::{ShaderSource, parse_sources},
        world::geometry::LightSource,
    };
    let root = resources("world-finish-profiles")?;
    let script = br#"textures/modern {
        { map $lightmap tcGen lightmap }
        { map textures/modern tcGen texture blendFunc filter }
    }"#;
    std::fs::write(root.join("scripts/finish.shader"), script)
        .map_err(|error| error.to_string())?;
    let vfs = mount(&root)?;
    let bytes = map_bytes(3);
    let map = modern_map(&bytes)?;
    let mut assets = Assets::load().map_err(|error| format!("{error:?}"))?;
    let profiles = [(false, false), (true, false), (false, true), (true, true)].map(
        |(vertex_light, fullbright)| {
            let mut profile = WorldPresentationProfile::native(RuleSetId::Quake3, None);
            profile.finish = MaterialFinishPolicy {
                vertex_light,
                fullbright,
                ..Default::default()
            };
            profile
        },
    );
    let loaded = load_world(
        &vfs,
        &map,
        &mut assets,
        None,
        &profiles.map(Into::into),
        Default::default(),
        false,
    )
    .map_err(|error| format!("{error:?}"))?;
    let expected = [
        LightSource::Page(0),
        LightSource::Vertex,
        LightSource::White,
        LightSource::White,
    ];
    let mesh = assets.world(loaded.world).unwrap().mesh();
    for (index, profile) in profiles.into_iter().enumerate() {
        let original = loaded.presentations[index];
        assert_eq!(original.profile, profile);
        assert_eq!(original.compile_policy, profile.finish.compile(false));
        let row = assets
            .world_presentation(original.id)
            .unwrap()
            .shader_registrations()[0];
        assert_eq!(row.requested, expected[index]);
        let before_guard = assets.counts();
        vfs.take_lookup_count();
        let fullscreen =
            append_world_presentation(&vfs, &mut assets, loaded.world, profile.into(), true)
                .map_err(|error| format!("{error:?}"))?
                .presentations[0];
        if profile.finish.vertex_light {
            assert_ne!(fullscreen.id, original.id);
            assert_eq!(fullscreen.compile_policy, profile.finish.compile(true));
        } else {
            assert_eq!(
                fullscreen, original,
                "an irrelevant guard does not create a presentation"
            );
            assert_eq!(assets.counts(), before_guard);
            assert_eq!(vfs.take_lookup_count(), 0);
        }
        let fullscreen_row = assets
            .world_presentation(fullscreen.id)
            .unwrap()
            .shader_registrations()[0];
        assert_eq!(
            fullscreen_row.requested, expected[index],
            "world request policy precedes the fullscreen guard"
        );
        if profile.finish.vertex_light {
            assert_ne!(fullscreen_row.shader, row.shader);
            let collapsed = assets
                .material(assets.shader_binding(row.shader).unwrap().material)
                .unwrap();
            let retained = assets
                .material(
                    assets
                        .shader_binding(fullscreen_row.shader)
                        .unwrap()
                        .material,
                )
                .unwrap();
            assert_eq!(collapsed.stages.len(), 1);
            assert_eq!(retained.stages.len(), 2);
        }
        assert_eq!(assets.world(loaded.world).unwrap().mesh(), mesh);
        let counts = assets.counts();
        vfs.take_lookup_count();
        let cached =
            append_world_presentation(&vfs, &mut assets, loaded.world, profile.into(), true)
                .map_err(|error| format!("{error:?}"))?
                .presentations[0];
        assert_eq!(cached, fullscreen);
        assert_eq!(assets.counts(), counts);
        assert_eq!(vfs.take_lookup_count(), 0);
        assert_eq!(
            assets.world_presentation_registration(original.id),
            Some(original)
        );
    }
    let catalog = parse_sources(&[ShaderSource {
        name: "generic.shader",
        bytes: script,
    }]);
    let lightmap = assets
        .world_presentation(loaded.presentations[0].id)
        .unwrap()
        .native_page0()
        .unwrap();
    let mut images = Images::new(
        &vfs,
        &mut assets,
        RuleSetId::Quake3,
        RuleSetId::Quake3,
        profiles[0].image_settings,
        None,
        None,
    )
    .map_err(|error| format!("{error:?}"))?;
    for profile in profiles {
        let generic = compile_named_shader(
            &catalog,
            b"textures/modern",
            true,
            NamedShaderLighting {
                source: LightSource::Page(0),
                lightmap,
            },
            profile.finish.compile(true),
            &mut images,
        )
        .map_err(|error| format!("{error:?}"))?;
        assert_eq!(
            generic.requested,
            LightSource::Page(0),
            "named/remap requests do not use world normalization"
        );
        assert_eq!(generic.finished, LightSource::Page(0));
    }
    drop(images);
    std::fs::remove_dir_all(root).map_err(|error| error.to_string())?;
    Ok(())
}

#[test]
fn finish_and_guard_appends_reuse_raster_images_for_every_source_family() -> Result<(), String> {
    use qa_render::material::world_load::append_world_presentation;
    for family in [1, 2, 3] {
        let root = resources(&format!("retained-images-{family}"))?;
        let vfs = mount(&root)?;
        let mut bytes = map_bytes(family);
        if family == 3 {
            bytes[8 + 14 * 8 + 4..8 + 14 * 8 + 8].copy_from_slice(&0u32.to_le_bytes());
            bytes.truncate(8 + 17 * 8);
        }
        let mut map = if family == 3 {
            modern_map(&bytes)?
        } else {
            legacy_map(&bytes, family)?
        };
        for face in &mut map.faces {
            face.lighting_offset = -1;
        }
        for surface in &mut map.surfaces {
            surface.lightmap = -1;
        }
        let mut assets = Assets::load().map_err(|error| format!("{error:?}"))?;
        let loaded = load_native_world(
            &vfs,
            &map,
            &mut assets,
            RuleSetId::Quake3,
            Default::default(),
        )
        .map_err(|error| format!("{error:?}"))?;
        let original = loaded.presentations[0];
        let image = surface_image(&assets, original.id);
        let pixels = assets.image(image).unwrap().rgba.to_vec();
        let revision = assets.image(image).unwrap().preparation_revision;
        let image_count = assets.counts().images;
        for path in if family == 2 {
            vec!["textures/wall.wal", "textures/other.wal"]
        } else if family == 3 {
            vec!["textures/modern.tga"]
        } else {
            vec![]
        } {
            std::fs::remove_file(root.join(path)).map_err(|error| error.to_string())?;
        }
        for (vertex_light, fullbright, ui_fullscreen) in [
            (false, false, true),
            (true, false, false),
            (true, false, true),
            (false, true, false),
        ] {
            let mut profile = original.profile;
            profile.finish.vertex_light = vertex_light;
            profile.finish.fullbright = fullbright;
            vfs.take_lookup_count();
            let appended = append_world_presentation(
                &vfs,
                &mut assets,
                loaded.world,
                profile.into(),
                ui_fullscreen,
            )
            .map_err(|error| format!("family {family}: {error:?}"))?
            .presentations[0];
            let material = assets
                .material(
                    assets
                        .shader_binding(assets.surface_shader(appended.id, 0).unwrap())
                        .unwrap()
                        .material,
                )
                .unwrap();
            assert!(
                material
                    .stages
                    .iter()
                    .any(|stage| stage.texture == StageTexture::Image(image)),
                "family {family} retains its exact raster image"
            );
            assert_eq!(
                assets.counts().images,
                image_count,
                "unlit finish changes do not upload images"
            );
            assert_eq!(assets.image(image).unwrap().rgba.as_ref(), pixels);
            assert_eq!(assets.image(image).unwrap().preparation_revision, revision);
            assert_eq!(
                vfs.take_lookup_count(),
                0,
                "retained image lookup must not reread a missing raster"
            );
        }
        std::fs::remove_dir_all(root).map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[test]
fn q1_embedded_and_split_sky_recipes_reuse_images_across_finish_profiles() -> Result<(), String> {
    use qa_render::{assets::Sky, material::world_load::append_world_presentation};
    let root = resources("retained-embedded-sky")?;
    let vfs = mount(&root)?;
    let bytes = map_bytes(1);
    let mut map = legacy_map(&bytes, 1)?;
    for face in &mut map.faces {
        face.lighting_offset = -1;
    }
    let mut sky_base = vec![10; 256 * 128];
    for row in sky_base.chunks_exact_mut(256) {
        row[..128].fill(0);
        row[128..].fill(11);
    }
    let sky_mips = [
        sky_base,
        vec![10; 128 * 64],
        vec![10; 64 * 32],
        vec![10; 32 * 16],
    ];
    map.textures[1] = Some(MipTexture {
        name: b"sky-retained",
        width: 256,
        height: 128,
        shift: 0,
        levels: [&sky_mips[0], &sky_mips[1], &sky_mips[2], &sky_mips[3]],
        animation: &[],
        flags: 0,
        contents: 0,
        value: 0,
    });
    map.texture_info[1].name = b"sky-retained";
    let mut assets = Assets::load().map_err(|error| format!("{error:?}"))?;
    let loaded = load_native_world(
        &vfs,
        &map,
        &mut assets,
        RuleSetId::Quake3,
        Default::default(),
    )
    .map_err(|error| format!("{error:?}"))?;
    let original = loaded.presentations[0];
    let base = surface_image(&assets, original.id);
    let sky = |assets: &Assets, presentation| {
        let material = assets
            .material(
                assets
                    .shader_binding(assets.surface_shader(presentation, 1).unwrap())
                    .unwrap()
                    .material,
            )
            .unwrap();
        let Sky::Layered { images, .. } = material.settings.sky.unwrap() else {
            panic!("native layered sky")
        };
        images
    };
    let layers = sky(&assets, original.id);
    assert_ne!(layers[0], layers[1]);
    let retained = [base, layers[0], layers[1]].map(|id| {
        (
            id,
            assets.image(id).unwrap().rgba.to_vec(),
            assets.image(id).unwrap().preparation_revision,
        )
    });
    let image_count = assets.counts().images;
    for (detail_textures, vertex_light, ui_fullscreen) in [
        (false, false, false),
        (false, true, false),
        (false, true, true),
    ] {
        let mut profile = original.profile;
        profile.finish.detail_textures = detail_textures;
        profile.finish.vertex_light = vertex_light;
        vfs.take_lookup_count();
        let candidate = append_world_presentation(
            &vfs,
            &mut assets,
            loaded.world,
            profile.into(),
            ui_fullscreen,
        )
        .map_err(|error| format!("{error:?}"))?
        .presentations[0];
        assert_eq!(surface_image(&assets, candidate.id), base);
        assert_eq!(sky(&assets, candidate.id), layers);
        assert_eq!(assets.counts().images, image_count);
        assert_eq!(vfs.take_lookup_count(), 0);
        for (id, pixels, revision) in &retained {
            assert_eq!(assets.image(*id).unwrap().rgba.as_ref(), pixels);
            assert_eq!(assets.image(*id).unwrap().preparation_revision, *revision);
        }
    }
    std::fs::remove_dir_all(root).map_err(|error| error.to_string())?;
    Ok(())
}

#[test]
fn discarded_world_image_policy_retries_from_source_without_dangling_image_ids()
-> Result<(), String> {
    use qa_render::material::world_load::append_world_presentation;
    let root = resources("retained-policy-rollback")?;
    let vfs = mount(&root)?;
    let mut bytes = map_bytes(3);
    bytes[8 + 14 * 8 + 4..8 + 14 * 8 + 8].copy_from_slice(&0u32.to_le_bytes());
    bytes.truncate(8 + 17 * 8);
    let mut map = modern_map(&bytes)?;
    map.surfaces[0].lightmap = -1;
    let mut assets = Assets::load().map_err(|error| format!("{error:?}"))?;
    let loaded = load_native_world(
        &vfs,
        &map,
        &mut assets,
        RuleSetId::Quake3,
        Default::default(),
    )
    .map_err(|error| format!("{error:?}"))?;
    let original = loaded.presentations[0];
    let image = surface_image(&assets, original.id);
    let pixels = assets.image(image).unwrap().rgba.to_vec();
    let prefix = assets.counts();
    let mut profile = original.profile;
    profile.image_settings.picmip += 1;
    let rejected =
        append_world_presentation(&vfs, &mut assets, loaded.world, profile.into(), false)
            .map_err(|error| format!("{error:?}"))?
            .presentations[0];
    let rejected_image = surface_image(&assets, rejected.id);
    let rejected_pixels = assets.image(rejected_image).unwrap().rgba.to_vec();
    assert_ne!(rejected_image, image);
    assets
        .discard_registration_suffix(prefix)
        .map_err(str::to_owned)?;
    assert_eq!(assets.counts(), prefix);
    assert!(assets.image(rejected_image).is_none());
    assert_eq!(
        assets.world(loaded.world).unwrap().presentations(),
        [original.id]
    );
    let mut changed =
        std::fs::read(root.join("textures/modern.tga")).map_err(|error| error.to_string())?;
    for pixel in changed[18..].chunks_exact_mut(3) {
        pixel.copy_from_slice(&[7, 17, 137]);
    }
    std::fs::write(root.join("textures/modern.tga"), changed).map_err(|error| error.to_string())?;
    vfs.take_lookup_count();
    let retry = append_world_presentation(&vfs, &mut assets, loaded.world, profile.into(), false)
        .map_err(|error| format!("{error:?}"))?
        .presentations[0];
    let retry_image = surface_image(&assets, retry.id);
    assert_eq!(retry.id, rejected.id);
    assert_eq!(retry_image, rejected_image);
    assert_ne!(
        assets.image(retry_image).unwrap().rgba.as_ref(),
        rejected_pixels
    );
    assert!(
        vfs.take_lookup_count() > 0,
        "discarded policy must not retain unpublished cache entries"
    );
    assert_eq!(assets.image(image).unwrap().rgba.as_ref(), pixels);
    let counts = assets.counts();
    vfs.take_lookup_count();
    let cached = append_world_presentation(&vfs, &mut assets, loaded.world, profile.into(), false)
        .map_err(|error| format!("{error:?}"))?
        .presentations[0];
    assert_eq!(cached, retry);
    assert_eq!(assets.counts(), counts);
    assert_eq!(vfs.take_lookup_count(), 0);
    std::fs::remove_dir_all(root).map_err(|error| error.to_string())?;
    Ok(())
}

#[test]
fn vertex_world_omits_native_pages_but_fullbright_keeps_real_generic_page_zero()
-> Result<(), String> {
    use qa_render::{
        material::world_load::{
            NamedShaderLighting, append_world_presentation, compile_named_shader,
        },
        shader::{ShaderSource, parse_sources},
        world::geometry::LightSource,
    };
    let root = resources("retained-native-pages")?;
    let vfs = mount(&root)?;
    let bytes = map_bytes(3);
    let map = modern_map(&bytes)?;
    let mut assets = Assets::load().map_err(|error| format!("{error:?}"))?;
    let mut vertex = WorldPresentationProfile::native(RuleSetId::Quake3, None);
    vertex.finish.vertex_light = true;
    let loaded = load_world(
        &vfs,
        &map,
        &mut assets,
        None,
        &[vertex.into()],
        Default::default(),
        false,
    )
    .map_err(|error| format!("{error:?}"))?;
    let vertex_id = loaded.presentations[0].id;
    assert_eq!(
        assets.world_presentation(vertex_id).unwrap().native_page0(),
        None
    );
    assert!(
        assets
            .images()
            .iter()
            .all(|image| (image.width, image.height) != (128, 128))
    );
    let raster = surface_image(&assets, vertex_id);
    let images = assets.counts().images;
    let mut fullbright = vertex;
    fullbright.finish.vertex_light = false;
    fullbright.finish.fullbright = true;
    let appended =
        append_world_presentation(&vfs, &mut assets, loaded.world, fullbright.into(), false)
            .map_err(|error| format!("{error:?}"))?
            .presentations[0];
    let presentation = assets.world_presentation(appended.id).unwrap();
    let page = presentation
        .native_page0()
        .ok_or("fullbright native page zero")?;
    assert_eq!(
        presentation.shader_registrations()[0].requested,
        LightSource::White
    );
    let material = assets
        .material(
            assets
                .shader_binding(presentation.surface_shader(0).unwrap())
                .unwrap()
                .material,
        )
        .unwrap();
    assert!(
        material
            .stages
            .iter()
            .any(|stage| stage.texture == StageTexture::Image(raster))
    );
    assert_eq!(assets.counts().images, images + 1);
    assert_eq!(
        (
            assets.image(page).unwrap().width,
            assets.image(page).unwrap().height
        ),
        (128, 128)
    );
    assert_eq!(&assets.image(page).unwrap().rgba[..4], &[255; 4]);
    let catalog = parse_sources(&[ShaderSource {
        name: "generic-page.shader",
        bytes: b"generic/page { { map $lightmap } { map $whiteimage blendFunc filter } }",
    }]);
    let mut images = Images::new(
        &vfs,
        &mut assets,
        RuleSetId::Quake3,
        RuleSetId::Quake3,
        fullbright.image_settings,
        None,
        None,
    )
    .map_err(|error| format!("{error:?}"))?;
    let generic = compile_named_shader(
        &catalog,
        b"generic/page",
        true,
        NamedShaderLighting {
            source: LightSource::Page(0),
            lightmap: page,
        },
        fullbright.finish.compile(false),
        &mut images,
    )
    .map_err(|error| format!("{error:?}"))?;
    drop(images);
    assert_eq!(generic.requested, LightSource::Page(0));
    assert_eq!(generic.finished, LightSource::Page(0));
    assert_eq!(
        assets.shader_binding(generic.shader).unwrap().lightmap,
        page
    );
    assert_eq!(
        assets.world_presentation(vertex_id).unwrap().native_page0(),
        None
    );
    std::fs::remove_dir_all(root).map_err(|error| error.to_string())?;
    Ok(())
}

#[test]
fn retained_world_images_report_only_the_current_compile_flag_conflicts() -> Result<(), String> {
    use qa_render::material::world_load::append_world_presentation;
    let root = resources("retained-image-conflicts")?;
    std::fs::write(root.join("scripts/flags.shader"), b"textures/modern { { map textures/modern } }\ntextures/otherflags { nomipmaps { map textures/modern } }").map_err(|error| error.to_string())?;
    let vfs = mount(&root)?;
    let mut bytes = map_bytes(3);
    bytes[8 + 14 * 8 + 4..8 + 14 * 8 + 8].copy_from_slice(&0u32.to_le_bytes());
    bytes.truncate(8 + 17 * 8);
    let mut map = modern_map(&bytes)?;
    map.surfaces[0].lightmap = -1;
    map.shaders.push(Shader {
        name: b"textures/otherflags",
        surface_flags: 0,
        content_flags: 0,
    });
    map.surfaces.push(Surface {
        shader: Some(1),
        ..map.surfaces[0]
    });
    map.leaf_faces.push(1);
    map.leaves[0].faces.count = 2;
    map.models[0].faces.count = 2;
    let mut assets = Assets::load().map_err(|error| format!("{error:?}"))?;
    let loaded = load_native_world(
        &vfs,
        &map,
        &mut assets,
        RuleSetId::Quake3,
        Default::default(),
    )
    .map_err(|error| format!("{error:?}"))?;
    let conflicts = |loaded: &LoadedWorld| {
        loaded
            .diagnostics
            .iter()
            .filter(|text| text.starts_with("native first-image flags retained:"))
            .count()
    };
    assert_eq!(conflicts(&loaded), 1);
    let original = loaded.presentations[0];
    let image = surface_image(&assets, original.id);
    let image_count = assets.counts().images;
    std::fs::remove_file(root.join("textures/modern.tga")).map_err(|error| error.to_string())?;
    let mut profile = original.profile;
    profile.finish.detail_textures = false;
    vfs.take_lookup_count();
    let appended =
        append_world_presentation(&vfs, &mut assets, loaded.world, profile.into(), false)
            .map_err(|error| format!("{error:?}"))?;
    assert_eq!(
        conflicts(&appended),
        1,
        "prior compilation notices are not replayed"
    );
    assert_eq!(surface_image(&assets, appended.presentations[0].id), image);
    assert_eq!(assets.counts().images, image_count);
    assert_eq!(vfs.take_lookup_count(), 0);
    let cached = append_world_presentation(&vfs, &mut assets, loaded.world, profile.into(), false)
        .map_err(|error| format!("{error:?}"))?;
    assert_eq!(
        conflicts(&cached),
        0,
        "a presentation hit performs no compilation"
    );
    std::fs::remove_dir_all(root).map_err(|error| error.to_string())?;
    Ok(())
}

#[test]
fn physical_lightmap_recipes_ignore_raster_finish_and_palette_policies() -> Result<(), String> {
    use qa_render::{lightmap::Q1GlScale, material::world_load::append_world_presentation};
    for family in [1, 2, 3] {
        let root = resources(&format!("physical-lightmap-recipes-{family}"))?;
        let vfs = mount(&root)?;
        let bytes = map_bytes(family);
        let map = if family == 3 {
            modern_map(&bytes)?
        } else {
            legacy_map(&bytes, family)?
        };
        let mut assets = Assets::load().map_err(|error| format!("{error:?}"))?;
        let loaded = load_native_world(
            &vfs,
            &map,
            &mut assets,
            RuleSetId::Quake3,
            Default::default(),
        )
        .map_err(|error| format!("{error:?}"))?;
        let original = loaded.presentations[0];
        let binding = |assets: &Assets, presentation| {
            *assets
                .shader_binding(assets.surface_shader(presentation, 0).unwrap())
                .unwrap()
        };
        let first = binding(&assets, original.id);
        assert_ne!(first.lightmap, ImageId(0));
        assert_eq!(first.lightmap_region.is_some(), family != 3);
        let pixels = assets.image(first.lightmap).unwrap().rgba.to_vec();
        let revision = assets.image(first.lightmap).unwrap().preparation_revision;
        let target_palette = load_palette(
            (&vfs).read_view(None),
            &mut assets,
            PaletteSource::native(RuleSetId::Quake).unwrap(),
        )
        .map_err(|error| format!("{error:?}"))?;
        let mut profiles = [original.profile; 3];
        profiles[0].finish.detail_textures = false;
        profiles[1].image_settings.picmip += 1;
        profiles[2].indexed_palette = Some(target_palette);
        for profile in profiles {
            let variant =
                append_world_presentation(&vfs, &mut assets, loaded.world, profile.into(), false)
                    .map_err(|error| format!("family {family}: {error:?}"))?
                    .presentations[0];
            let current = binding(&assets, variant.id);
            assert_eq!(
                current.lightmap, first.lightmap,
                "family {family} shares its physical recipe"
            );
            assert_eq!(current.lightmap_region, first.lightmap_region);
            assert_eq!(current.lightmap_conversion, first.lightmap_conversion);
            assert_eq!(assets.image(first.lightmap).unwrap().rgba.as_ref(), pixels);
            assert_eq!(
                assets.image(first.lightmap).unwrap().preparation_revision,
                revision
            );
            if family == 3 {
                assert_eq!(
                    assets
                        .world_presentation(variant.id)
                        .unwrap()
                        .native_page0(),
                    Some(first.lightmap)
                );
            }
            let counts = assets.counts();
            vfs.take_lookup_count();
            let cached =
                append_world_presentation(&vfs, &mut assets, loaded.world, profile.into(), true)
                    .map_err(|error| format!("{error:?}"))?
                    .presentations[0];
            assert_eq!(
                cached, variant,
                "guard is irrelevant without vertex collapse"
            );
            assert_eq!(assets.counts(), counts);
            assert_eq!(vfs.take_lookup_count(), 0);
        }
        let prefix = assets.counts();
        let mut changed = original.profile;
        changed.q1_gl_scale = Q1GlScale::Standard;
        changed.q2_modulate = 2.0;
        vfs.take_lookup_count();
        let rejected =
            append_world_presentation(&vfs, &mut assets, loaded.world, changed.into(), false)
                .map_err(|error| format!("{error:?}"))?
                .presentations[0];
        let candidate = binding(&assets, rejected.id);
        if family == 3 {
            assert_eq!(
                candidate.lightmap, first.lightmap,
                "native pages ignore legacy conversion knobs"
            );
        } else {
            assert_ne!(candidate.lightmap, first.lightmap);
            assert_ne!(
                assets.image(candidate.lightmap).unwrap().rgba.as_ref(),
                pixels
            );
        }
        assert_eq!(
            vfs.take_lookup_count(),
            0,
            "the warmed raster policy needs no lookup for a lightmap recipe change"
        );
        let candidate_pixels = assets.image(candidate.lightmap).unwrap().rgba.to_vec();
        assets
            .discard_registration_suffix(prefix)
            .map_err(str::to_owned)?;
        assert_eq!(assets.counts(), prefix);
        assert_eq!(assets.image(first.lightmap).unwrap().rgba.as_ref(), pixels);
        if family != 3 {
            assert!(assets.image(candidate.lightmap).is_none());
        }
        vfs.take_lookup_count();
        let retry =
            append_world_presentation(&vfs, &mut assets, loaded.world, changed.into(), false)
                .map_err(|error| format!("{error:?}"))?
                .presentations[0];
        assert_eq!(retry.id, rejected.id);
        let retried = binding(&assets, retry.id);
        assert_eq!(retried.lightmap, candidate.lightmap);
        assert_eq!(
            assets.image(retried.lightmap).unwrap().rgba.as_ref(),
            candidate_pixels
        );
        assert_eq!(assets.image(first.lightmap).unwrap().rgba.as_ref(), pixels);
        assert_eq!(vfs.take_lookup_count(), 0);
        std::fs::remove_dir_all(root).map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[test]
fn format_reload_recompiles_each_world_family_without_replacing_geometry_or_lightmaps()
-> Result<(), String> {
    use qa_render::material::{
        resources::ImageSearchOrder,
        world_load::{append_world_presentation, reload_world_presentation},
    };
    for family in [1, 2, 3] {
        let root = resources(&format!("reload-world-{family}"))?;
        let tga = std::fs::read(root.join("textures/modern.tga")).unwrap();
        for name in ["wall", "other", "modern"] {
            std::fs::write(root.join(format!("textures/{name}.tga")), &tga).unwrap();
            std::fs::write(
                root.join(format!("textures/{name}.jpeg")),
                include_bytes!("resources/blue.jpg"),
            )
            .unwrap();
        }
        let vfs = mount(&root)?;
        let bytes = map_bytes(family);
        let map = if family == 3 {
            modern_map(&bytes)?
        } else {
            legacy_map(&bytes, family)?
        };
        let mut assets = Assets::load().unwrap();
        assets.set_image_search_order(ImageSearchOrder::parse(b"tga").unwrap());
        let loaded = load_native_world(
            &vfs,
            &map,
            &mut assets,
            map.bsp.format.rule_set(),
            Default::default(),
        )
        .map_err(|error| format!("family {family}: {error:?}"))?;
        drop(map);
        let world = loaded.world;
        let old = loaded.presentations[0];
        let old_image = surface_image(&assets, old.id);
        let old_rgba = assets.image(old_image).unwrap().rgba.to_vec();
        let indices = assets
            .image(old_image)
            .unwrap()
            .indexed
            .as_ref()
            .map(|image| image.mip(0).unwrap().indices().to_vec());
        let mesh = assets.world(world).unwrap().mesh();
        let vertices = assets.model(mesh).unwrap().vertices.to_vec();
        let lightmap = assets
            .shader_binding(assets.surface_shader(old.id, 0).unwrap())
            .unwrap()
            .lightmap;
        assert_ne!(lightmap, ImageId(0));
        let light_rgba = assets.image(lightmap).unwrap().rgba.to_vec();
        let light_revision = assets.image(lightmap).unwrap().preparation_revision;
        let prefix = assets.counts();
        assets.set_image_search_order(ImageSearchOrder::parse(b"jpeg").unwrap());
        let rejected = assets.begin_world_image_reload().unwrap();
        rejected.replay(&vfs, &mut assets).unwrap();
        let candidate =
            reload_world_presentation(&vfs, &mut assets, world, old.id, old.profile.into(), false)
                .map_err(|error| format!("family {family}: {error:?}"))?
                .presentations[0];
        let candidate_image = surface_image(&assets, candidate.id);
        assert_ne!(candidate_image, old_image);
        assert_eq!(candidate.profile, old.profile);
        assert_eq!(
            assets.world(world).unwrap().presentations(),
            &[candidate.id, old.id]
        );
        assert_eq!(assets.world_presentation_registration(old.id), Some(old));
        CpuBackend::load_with_assets(16, 16, &assets).unwrap();
        rejected.restore(&mut assets).unwrap();
        assets.discard_registration_suffix(prefix).unwrap();
        assert_eq!(assets.counts(), prefix);
        assert_eq!(assets.world(world).unwrap().presentations(), &[old.id]);
        assert_eq!(surface_image(&assets, old.id), old_image);
        let reload = assets.begin_world_image_reload().unwrap();
        reload.replay(&vfs, &mut assets).unwrap();
        let next =
            reload_world_presentation(&vfs, &mut assets, world, old.id, old.profile.into(), false)
                .map_err(|error| format!("family {family}: {error:?}"))?
                .presentations[0];
        assert_eq!(next.id, candidate.id);
        let image = surface_image(&assets, next.id);
        assert_eq!(image, candidate_image);
        let refreshed = assets.image(image).unwrap();
        assert!(refreshed.rgba[2] > 230 && refreshed.rgba[0] < 10);
        assert_eq!(
            refreshed
                .indexed
                .as_ref()
                .map(|image| image.mip(0).unwrap().indices().to_vec()),
            indices
        );
        assert_eq!(assets.counts().models, prefix.models);
        assert_eq!(assets.world(world).unwrap().mesh(), mesh);
        assert_eq!(assets.model(mesh).unwrap().vertices.as_ref(), vertices);
        assert_eq!(assets.image(old_image).unwrap().rgba.as_ref(), old_rgba);
        assert_eq!(assets.image(lightmap).unwrap().rgba.as_ref(), light_rgba);
        assert_eq!(
            assets.image(lightmap).unwrap().preparation_revision,
            light_revision
        );
        assert_eq!(
            assets
                .shader_binding(assets.surface_shader(next.id, 0).unwrap())
                .unwrap()
                .lightmap,
            lightmap
        );
        let count = assets.counts();
        vfs.take_lookup_count();
        let cached =
            append_world_presentation(&vfs, &mut assets, world, old.profile.into(), false).unwrap();
        assert_eq!(cached.presentations[0], next);
        assert_eq!(vfs.take_lookup_count(), 0);
        assert_eq!(assets.counts(), count);
        std::fs::remove_dir_all(root).map_err(|error| error.to_string())?;
    }
    Ok(())
}
