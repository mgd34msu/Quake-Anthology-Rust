use qa_content::vfs::Vfs;
use qa_core::primitives::{RuleSetId, Vec3};
use qa_formats::{archive::ArchiveReader, bsp::Map};
use qa_render::{
    Assets, FrontEnd, Limits, Refdef, Viewport,
    cpu::CpuBackend,
    material::{
        resources::{PaletteSource, load_palette},
        world_load::{WorldLoadOptions, WorldPresentationProfile, load_world},
    },
    world::VisibleSurface,
};

#[path = "../src/affine.rs"]
#[allow(dead_code)]
mod affine;

#[test]
#[ignore = "requires owned rerelease files; set QA_RETAIL_Q2_RERELEASE and use --include-ignored"]
fn retail_base1_decoupled_samples_reach_the_indexed_surface_cache() {
    let product = std::env::var_os("QA_RETAIL_Q2_RERELEASE").expect("QA_RETAIL_Q2_RERELEASE");
    let mut vfs = Vfs::default();
    vfs.mount_product(std::path::Path::new(&product), 0)
        .unwrap();
    let bytes = vfs
        .read_file(
            b"maps/base1.bsp",
            256 * 1024 * 1024,
            &mut ArchiveReader::default(),
        )
        .unwrap();
    let map = Map::parse(&bytes).unwrap();
    assert_eq!(map.faces.len(), 16_787);
    assert_eq!(map.decoupled_lightmaps.len(), map.faces.len());
    assert_eq!(
        map.decoupled_lightmaps
            .iter()
            .flatten()
            .filter(|row| row.offset.is_some())
            .count(),
        16_330
    );
    let mut assets = Assets::load().unwrap();
    let palette = load_palette(
        (&vfs).read_view(None),
        &mut assets,
        PaletteSource::native(RuleSetId::Quake2Rerelease).unwrap(),
    )
    .unwrap();
    let loaded = load_world(
        &vfs,
        &map,
        &mut assets,
        Some(palette),
        &[WorldPresentationProfile::native(RuleSetId::Quake2Rerelease, Some(palette)).into()],
        WorldLoadOptions::default(),
        false,
    )
    .unwrap();
    let geometry = assets.world(loaded.world).unwrap().geometry();
    assert!(geometry.light_samples.iter().any(|rgb| *rgb != [0; 3]));
    let world = assets.world(loaded.world).unwrap();
    let mut point_view = qa_render::world::WorldView::load(world);
    let styles = [qa_render::LightStyle::default(); 256];
    let point_sample = geometry
        .surfaces
        .iter()
        .filter(|surface| {
            surface.plane.is_some_and(|plane| plane.normal.0[2] == 1.)
                && surface.lightmap_projection.is_some()
                && surface.light_samples.count > 0
                && surface.source_flags & 12 == 0
        })
        .find_map(|surface| {
            let vertices = &geometry.vertices[surface.vertices.range()];
            let point = vertices
                .iter()
                .fold(Vec3::default(), |sum, vertex| sum + vertex.vertex.position)
                / vertices.len() as f32
                + Vec3([0., 0., 1.]);
            let light = point_view
                .point_lighting(world, point, &styles, 1.)
                .unwrap();
            (light.ambient.iter().any(|&value| value > 0.)).then_some((point, light.ambient))
        })
        .expect("production BSPX point-light sample above a lit floor");
    assert!(point_sample.1.iter().all(|value| value.is_finite()));
    println!("retail BSPX point-light sample {point_sample:?}");
    // These real lit, opaque, non-collinear faces have rank-one texture UVs.
    // Their independent lightmaps need a two-dimensional cache chart.
    let rank_one_faces = [
        373, 379, 382, 442, 480, 6155, 6318, 6323, 6364, 11915, 11953, 14070, 14418,
    ];
    for surface_index in rank_one_faces {
        let surface = &geometry.surfaces[surface_index];
        assert!(surface.lightmap_projection.is_some());
        assert!(surface.light_samples.count > 0);
        let points = &geometry.vertices[surface.vertices.range()];
        assert!(
            affine::widest_fan_basis(points.len(), |i| {
                points[i].vertex.texcoord.map(f64::from)
            })
            .is_none(),
            "face {surface_index} texture basis"
        );
    }
    // One cold backend admits the entire map, including all retained slivers.
    // Draw a nondegenerate axial wall: a rounded sliver need not cover a pixel.
    let mut cpu = CpuBackend::load_with_assets(64, 64, &assets).unwrap();
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let (surface_index, surface) = geometry
        .surfaces
        .iter()
        .enumerate()
        .find(|(_, surface)| {
            let normal = surface.plane.unwrap().normal.0;
            let points = &geometry.vertices[surface.vertices.range()];
            surface.source_flags & 124 == 0
                && surface.texture_extents.iter().all(|&v| v > 16)
                && normal.iter().any(|v| v.abs() == 1.0)
                && geometry.light_samples[surface.light_samples.range()]
                    .iter()
                    .any(|rgb| rgb[2] > 64)
                && points.len() >= 3
                && affine::widest_fan_basis(points.len(), |i| {
                    points[i].vertex.texcoord.map(f64::from)
                })
                .is_some()
        })
        .expect("a real lit axial wall with a two-dimensional texture basis");
    let points = &geometry.vertices[surface.vertices.range()];
    let normal = surface.plane.unwrap().normal;
    let center = points
        .iter()
        .fold(Vec3::default(), |sum, p| sum + p.vertex.position)
        / points.len() as f32;
    let forward = -normal;
    let hint = if normal.0[2].abs() < 0.9 {
        Vec3([0.0, 0.0, 1.0])
    } else {
        Vec3([0.0, 1.0, 0.0])
    };
    let left = qa_core::math::normalized(qa_core::math::cross(hint, forward));
    let distance =
        (qa_core::math::length(surface.bounds.maxs - surface.bounds.mins) * 0.75).clamp(1.0, 32.0);
    let mut view = Refdef {
        viewport: Viewport {
            width: 64,
            height: 64,
            ..Viewport::default()
        },
        origin: center + normal * distance,
        axes: [forward, left, qa_core::math::cross(forward, left)],
        near: 0.01,
        far: 128.0,
        fov: [90.0; 2],
        ..Refdef::default()
    };
    loaded.presentations[0].defaults.apply(&mut view);
    let mut frame = frontend.begin_frame([0, 0, 0, 255]).unwrap();
    assert!(frame.add_world(
        loaded.world,
        loaded.presentations[0].id,
        &[VisibleSurface {
            surface: surface_index as u32,
            depth_key: 1,
        }]
    ));
    assert!(frame.render_scene(
        view,
        &[],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    let packet = frame.finish();
    assert_eq!(cpu.render(&packet, &assets).rejected, 0);
    assert!(cpu.world_stats().pixels > 0, "face {surface_index}");
    assert!(cpu.world_stats().indexed_pixels > 0, "face {surface_index}");
    assert!(cpu.world_stats().cache.fills > 0, "face {surface_index}");
    assert!(
        cpu.pixels()
            .iter()
            .any(|pixel| pixel.to_le_bytes()[..3] != [0; 3])
    );
}
