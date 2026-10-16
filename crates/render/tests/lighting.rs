use qa_core::{math, primitives::Vec3};
use qa_render::{
    lighting::{setup_entity, unlit_sample},
    scene::{Light, SceneEntity},
    stage::EntityLighting,
};

#[test]
fn legacy_point_samples_preserve_style_arithmetic_dark_hits_and_texture_truncation() {
    use qa_core::primitives::{Bounds, IndexSpan, Plane};
    use qa_render::{lighting::PointLighting, world::geometry::*};
    use qa_world::visibility::{PvsRows, SurfaceRayScratch, VisLeaf, VisNode, VisibilityWorld};
    let bounds = Bounds {
        mins: Vec3([-32.; 3]),
        maxs: Vec3([32.; 3]),
    };
    let plane = Plane::oriented(Vec3([0., 0., 1.]), 0.);
    let visibility = VisibilityWorld::load(
        vec![plane],
        vec![VisNode {
            plane: 0,
            children: [-1, -1],
            bounds,
            surfaces: IndexSpan { first: 0, count: 1 },
        }],
        vec![VisLeaf {
            selector: None,
            area: None,
            solid: false,
            bounds,
            surfaces: IndexSpan::default(),
        }],
        vec![],
        1,
        0,
        PvsRows::all_visible(0),
    )
    .unwrap();
    let mut scratch = SurfaceRayScratch::load(&visibility);
    let surface = WorldSurface {
        source_id: 0,
        kind: GeometryKind::Polygon,
        vertices: IndexSpan::default(),
        indices: IndexSpan::default(),
        boundaries: IndexSpan::default(),
        plane: Some(plane),
        bounds,
        texture_coordinates: TextureCoordinates::Texels,
        texture_projection: [[1., 0., 0., 0.], [0., 1., 0., 0.]],
        texture_minima: [-16; 2],
        texture_extents: [32; 2],
        lightmap_projection: None,
        lightmap_grid: [3; 2],
        styles: [0, 1, 255, 255],
        light_source: LightSource::Samples,
        light_encoding: LightEncoding::Rgb,
        light_samples: IndexSpan {
            first: 0,
            count: 18,
        },
        source_texture: None,
        source_texture_info: None,
        source_shader: None,
        source_flags: 0,
        source_contents: 0,
        no_draw: false,
        source_fog: -1,
        source_brush_side: -1,
        source_lightmap: -1,
        lightmap_rect: [0; 4],
        lightmap_origin: Vec3::default(),
        lightmap_vectors: [Vec3::default(); 3],
        patch: None,
    };
    let mut geometry = WorldGeometry {
        partition: GeometryPartition::SplitBsp,
        world_has_lightdata: true,
        vertices: vec![],
        indices: vec![],
        boundaries: vec![],
        surfaces: vec![surface],
        light_samples: [[64, 32, 16]; 9]
            .into_iter()
            .chain([[8, 4, 2]; 9])
            .collect(),
        models: vec![],
        patch_stats: PatchStats::default(),
    };
    let mut styles = [qa_render::LightStyle::default(); 256];
    styles[0].indexed_scale = 128;
    styles[0].rgb = [1., 0.5, 2.];
    styles[1].rgb = [0.5, 1., 0.];
    let indexed = PointLighting::load(true, [true].into_iter());
    let rgb = PointLighting::load(false, [true].into_iter());
    let point = Vec3([-16.9, -16., 8.]);
    assert_eq!(
        indexed
            .sample(&geometry, &visibility, &mut scratch, point, &styles, 2.)
            .unwrap()
            .ambient,
        [40., 20., 10.]
    );
    assert_eq!(
        rgb.sample(&geometry, &visibility, &mut scratch, point, &styles, 2.)
            .unwrap()
            .ambient,
        [136., 40., 64.]
    );
    assert_eq!(
        indexed
            .sample(
                &geometry,
                &visibility,
                &mut scratch,
                Vec3([17., 0., 8.]),
                &styles,
                1.
            )
            .unwrap()
            .ambient,
        [0.; 3]
    );
    // Independent BSPX coordinates select the light sample even when the
    // texture lattice cannot represent this point. Swap axes and translate.
    geometry.surfaces[0].lightmap_projection = Some([[0., 0.125, 0., 2.], [0.125, 0., 0., 2.]]);
    geometry.surfaces[0].texture_minima = [i32::MAX; 2];
    geometry.light_samples[3] = [128, 64, 32];
    geometry.light_samples[12] = [16, 8, 4];
    let decoupled_point = Vec3([-8., -16., 8.]);
    assert_eq!(
        rgb.sample(
            &geometry,
            &visibility,
            &mut scratch,
            decoupled_point,
            &styles,
            2.
        )
        .unwrap()
        .ambient,
        [272., 80., 128.]
    );
    assert_eq!(
        indexed
            .sample(
                &geometry,
                &visibility,
                &mut scratch,
                decoupled_point,
                &styles,
                2.
            )
            .unwrap()
            .ambient,
        [80., 40., 20.]
    );
    assert_eq!(
        rgb.sample(
            &geometry,
            &visibility,
            &mut scratch,
            Vec3([-17., -16., 8.]),
            &styles,
            2.
        )
        .unwrap()
        .ambient,
        [0.; 3]
    );
    geometry.surfaces[0].lightmap_projection = None;
    geometry.surfaces[0].texture_minima = [-16; 2];
    geometry.surfaces[0].light_samples.count = 0;
    assert_eq!(
        indexed
            .sample(&geometry, &visibility, &mut scratch, point, &styles, 1.)
            .unwrap()
            .ambient,
        [0.; 3]
    );
    geometry.world_has_lightdata = false;
    assert_eq!(
        indexed
            .sample(&geometry, &visibility, &mut scratch, point, &styles, 1.)
            .unwrap()
            .ambient,
        [255.; 3]
    );
}

#[test]
fn grid_interpolation_renormalizes_wall_holes_and_keeps_source_row_aliases() {
    use qa_core::primitives::Bounds;
    use qa_formats::bsp::LightGridPoint;
    use qa_render::lighting::LightGrid;
    let bounds = Bounds {
        mins: Vec3::default(),
        maxs: Vec3([1.; 3]),
    };
    let points = std::array::from_fn::<_, 8, _>(|index| LightGridPoint {
        ambient: [10 * (index as u8 + 1), 20, 30],
        directed: [8, 16, 24],
        longitude: 0,
        latitude: 0,
    });
    let grid = LightGrid::load(bounds, Vec3([1.; 3]), points.into_iter(), 0).unwrap();
    let light = grid.sample(Vec3([0.25, 0.5, 0.75]), 1., 1.);
    assert_eq!(light.ambient, [52.5, 20., 30.]);
    assert_eq!(light.directed, [8., 16., 24.]);
    assert_eq!(light.direction, Vec3([0., 0., 1.]));
    // The upper X corner of row zero aliases the next in-allocation row.
    assert_eq!(
        grid.sample(Vec3([1.5, 0., 0.]), 1., 1.).ambient,
        [25., 20., 30.]
    );
    let mut holes = [LightGridPoint {
        ambient: [64, 32, 16],
        directed: [8; 3],
        longitude: 0,
        latitude: 0,
    }; 8];
    holes[0].ambient = [0; 3];
    let grid = LightGrid::load(bounds, Vec3([1.; 3]), holes.into_iter(), 0).unwrap();
    assert_eq!(
        grid.sample(Vec3([0.5, 0., 0.]), 1., 1.).ambient,
        [64., 32., 16.]
    );
    let points = [LightGridPoint {
        ambient: [100, 50, 25],
        directed: [20, 10, 5],
        longitude: 0,
        latitude: 0,
    }; 8];
    let grid = LightGrid::load(bounds, Vec3([1.; 3]), points.into_iter(), 2).unwrap();
    let light = grid.sample(Vec3::default(), 2., 0.5);
    assert_eq!(light.ambient, [510., 254., 126.]);
    assert_eq!(light.directed, [40., 20., 10.]);
    assert!(LightGrid::load(bounds, Vec3([0., 1., 1.]), points.into_iter(), 0).is_err());
    assert!(LightGrid::load(bounds, Vec3([1.; 3]), points[..7].iter().copied(), 0).is_err());
}

#[test]
fn grid_recoloring_restores_authored_rgb_and_preserves_layout_directions_and_holes() {
    use qa_core::primitives::Bounds;
    use qa_formats::bsp::LightGridPoint;
    use qa_render::lighting::LightGrid;
    let bounds = Bounds {
        mins: Vec3([8., 16., 24.]),
        maxs: Vec3([10., 18., 26.]),
    };
    let mut points = std::array::from_fn::<_, 8, _>(|index| LightGridPoint {
        ambient: [100 + index as u8, 50 + index as u8, 25 + index as u8],
        directed: [7 + index as u8, 11 + index as u8, 15 + index as u8],
        latitude: 19 * index as u8,
        longitude: 13 * index as u8,
    });
    points[0].ambient = [0; 3];
    let grid = LightGrid::load(bounds, Vec3([2.; 3]), points.into_iter(), 0).unwrap();
    let queries = [
        bounds.mins,
        Vec3([10., 16., 24.]),
        Vec3([9., 16., 24.]),
        Vec3([9., 17., 25.]),
        Vec3([11., 16., 24.]),
        Vec3([14., 20., 28.]),
    ];
    let original = queries.map(|point| grid.sample(point, 1., 1.));
    assert_eq!(original[0].ambient, [0.; 3]);
    assert_eq!(original[0].directed, [0.; 3]);
    assert_eq!(original[1].ambient, [101., 51., 26.]);
    assert_eq!(original[1].directed, [8., 12., 16.]);
    // A hole contributes no light or direction; the adjacent valid corner is
    // renormalized to full strength at their midpoint.
    assert_eq!(original[2], original[1]);

    let shifted = grid.recolored(2).unwrap();
    let bright = queries.map(|point| shifted.sample(point, 1., 1.));
    assert_eq!(bright[0], original[0]);
    assert_eq!(bright[1].ambient, [255., 128., 65.]);
    assert_eq!(bright[1].directed, [32., 48., 64.]);
    for (before, after) in original.iter().zip(bright) {
        assert_eq!(before.direction, after.direction);
    }
    assert_eq!(queries.map(|point| grid.sample(point, 1., 1.)), original);
    let restored = shifted.recolored(0).unwrap();
    assert_eq!(
        queries.map(|point| restored.sample(point, 1., 1.)),
        original
    );
    let initially_shifted = LightGrid::load(bounds, Vec3([2.; 3]), points.into_iter(), 2).unwrap();
    let initially_restored = initially_shifted.recolored(0).unwrap();
    assert_eq!(
        queries.map(|point| initially_restored.sample(point, 1., 1.)),
        original
    );
    assert_eq!(
        shifted
            .recolored(-1)
            .unwrap()
            .sample(queries[1], 1., 1.)
            .ambient,
        [50., 25., 13.]
    );
    assert_eq!(
        shifted
            .recolored(15)
            .unwrap()
            .sample(queries[1], 1., 1.)
            .ambient,
        [255., 128., 65.]
    );
    for shift in [-8, 15] {
        assert!(grid.recolored(shift).is_ok());
        assert!(LightGrid::load(bounds, Vec3([2.; 3]), points.into_iter(), shift).is_ok());
    }
    for shift in [-9, 16] {
        assert!(grid.recolored(shift).is_err());
        assert!(LightGrid::load(bounds, Vec3([2.; 3]), points.into_iter(), shift).is_err());
    }
    assert_eq!(queries.map(|point| grid.sample(point, 1., 1.)), original);
}

#[test]
fn native_unlit_sample_keeps_minimum_add_clamp_and_model_local_direction() {
    let sun = math::normalized(Vec3([0.45, 0.3, 0.9]));
    for identity in [0.0, 0.25, 0.5, 1.0] {
        for minimum_light in [false, true] {
            let entity = SceneEntity {
                minimum_light,
                axes: [Vec3([0., 1., 0.]), Vec3([-1., 0., 0.]), Vec3([0., 0., 1.])],
                ..Default::default()
            };
            let result = setup_entity(&entity, unlit_sample(identity, sun), identity, &[]);
            let expected =
                ((identity * 150.) + (identity * 32.)).min((identity * 255.) as i32 as f32);
            assert_eq!(result.ambient, [expected; 3]);
            assert_eq!(result.directed, [identity * 150.; 3]);
            if identity != 0. {
                assert!(
                    result.direction.0[0] > 0.
                        && result.direction.0[1] < 0.
                        && result.direction.0[2] > 0.
                );
            } else {
                assert_eq!(result.direction, Vec3::default());
            }
        }
    }
}

#[test]
fn dynamic_light_uses_lighting_origin_and_minimum_radius_without_double_normalization() {
    let entity = SceneEntity {
        origin: Vec3([100., 0., 0.]),
        lighting_origin: Some(Vec3([1., 0., 0.])),
        ..Default::default()
    };
    let sample = EntityLighting {
        ambient: [300., -10., 4.],
        directed: [0.; 3],
        direction: Vec3::default(),
    };
    let light = Light {
        origin: Vec3([5., 0., 0.]),
        radius: 8.,
        color: [1., 2., 3.],
        additive: true,
    };
    let result = setup_entity(&entity, sample, 1., &[light]);
    assert_eq!(result.ambient, [255., 22., 36.]);
    assert_eq!(result.directed, [4., 8., 12.]);
    assert_eq!(result.direction, Vec3([1., 0., 0.]));
}

#[test]
fn shared_view_lighting_keeps_explicit_values_and_does_not_light_generated_geometry() {
    use qa_render::{Assets, FrontEnd, Limits, Refdef, SceneModel, ShaderBindingId, Vertex};
    let mut assets = Assets::load().unwrap();
    let model = assets
        .register_model(&[Vertex::default(); 3], &[0, 1, 2], ShaderBindingId(0))
        .unwrap();
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    let explicit = EntityLighting::default();
    assert!(frame.add_entity(SceneEntity {
        model: model.into(),
        lighting: Some(explicit),
        ..Default::default()
    }));
    assert!(frame.add_entity(SceneEntity {
        model: model.into(),
        ..Default::default()
    }));
    assert!(frame.add_entity(SceneEntity {
        model: SceneModel::Sprite {
            radius: 1.,
            rotation: 0.
        },
        shader: Some(ShaderBindingId(0).into()),
        ..Default::default()
    }));
    let mut samples = 0;
    frame.light_entities(1., |_| {
        samples += 1;
        unlit_sample(1., Vec3([0., 0., 1.]))
    });
    assert_eq!(samples, 1);
    assert!(frame.render_scene(
        Refdef::default(),
        &[],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    let packet = frame.finish();
    let qa_render::scene::Command::View(view) = packet.commands()[1] else {
        panic!("view")
    };
    let entities = packet.entities(view.scene.entities);
    assert_eq!(entities[0].lighting, Some(explicit));
    assert_eq!(entities[1].lighting.unwrap().ambient, [182.; 3]);
    assert_eq!(entities[2].lighting, None);
}

#[test]
fn no_world_diffuse_models_reach_cpu_pixels_through_shared_lighting() {
    use qa_render::{
        Assets, FrontEnd, Limits, Refdef, Vertex, Viewport,
        assets::{MaterialSettings, Stage},
        cpu::CpuBackend,
        shader::{Cull, RgbGen},
    };
    let mut assets = Assets::load().unwrap();
    let material = {
        let material = assets
            .register_material(
                "preview diffuse",
                &[Stage {
                    rgb_gen: RgbGen::LightingDiffuse,
                    ..Default::default()
                }],
                MaterialSettings {
                    cull: Cull::None,
                    ..Default::default()
                },
            )
            .unwrap();
        assets
            .register_shader_binding(qa_render::ShaderBinding::material(
                material,
                qa_render::world::geometry::LightSource::TwoDimensional,
            ))
            .unwrap()
    };
    let vertices =
        [[8., -2., -2.], [8., 2., -2.], [8., 2., 2.], [8., -2., 2.]].map(|position| Vertex {
            position: Vec3(position),
            normal: Vec3([0., 0., 1.]),
            ..Default::default()
        });
    let model = assets
        .register_model(&vertices, &[0, 1, 2, 0, 2, 3], material)
        .unwrap();
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = front.begin_frame([0, 0, 0, 255]).unwrap();
    assert!(frame.add_entity(SceneEntity {
        model: model.into(),
        ..Default::default()
    }));
    frame.light_entities(1., |_| unlit_sample(1., Vec3([0., 0., 1.])));
    assert!(frame.render_scene(
        Refdef {
            viewport: Viewport {
                width: 32,
                height: 24,
                ..Default::default()
            },
            ..Default::default()
        },
        &[],
        &assets,
        qa_render::shader::reference::ShaderView::direct()
    ));
    let mut cpu = CpuBackend::load(32, 24).unwrap();
    let stats = cpu.render(&frame.finish(), &assets);
    assert_eq!(stats.rejected, 0);
    assert!(cpu.inverse_depth().iter().any(|&depth| depth > 0.));
    assert!(
        cpu.pixels()
            .iter()
            .any(|&pixel| pixel != u32::from_le_bytes([0, 0, 0, 255]))
    );
}
