#[path = "support/world_surfaces.rs"]
mod world_surfaces;

use qa_core::primitives::Vec3;
use qa_render::{
    Assets, Draw2d, FrontEnd, Limits, Refdef, ShaderBinding, ShaderBindingId, Vertex, Viewport,
    assets::{Cull, MaterialSettings, Sampler, Stage, StageTexture},
    cpu::CpuBackend,
    lightmap::{AtlasRegion, LightmapConversion},
    world::{SurfaceMaterial, geometry::LightSource},
};

fn stage(image: qa_render::ImageId) -> Stage {
    Stage {
        texture: StageTexture::Image(image),
        sampler: Sampler {
            mipmaps: false,
            ..Default::default()
        },
        ..Default::default()
    }
}

#[test]
fn nonparallel_binding_ids_select_distinct_lightmaps_from_one_material() {
    let mut assets = Assets::load_with_shader_capacity(8).unwrap();
    assert_eq!(assets.shader_binding_capacity(), 8);
    for name in ["unused first", "unused second"] {
        assets
            .register_material(name, &[stage(qa_render::ImageId(0))], Default::default())
            .unwrap();
    }
    let material = assets
        .register_material(
            "shared lightmap stages",
            &[Stage {
                texture: StageTexture::Lightmap,
                ..stage(qa_render::ImageId(0))
            }],
            Default::default(),
        )
        .unwrap();
    let red = assets.register_image(1, 1, &[255, 0, 0, 255]).unwrap();
    let blue = assets.register_image(1, 1, &[0, 0, 255, 255]).unwrap();
    let first = ShaderBinding {
        lightmap: red,
        ..ShaderBinding::material(material, LightSource::Page(7))
    };
    let second = ShaderBinding {
        lightmap: blue,
        ..ShaderBinding::material(material, LightSource::Page(9))
    };
    let red_shader = assets.register_shader_binding(first).unwrap();
    let blue_shader = assets.register_shader_binding(second).unwrap();
    assert_ne!(red_shader, blue_shader);
    assert_ne!(red_shader.0, material.0);
    assert_ne!(blue_shader.0, material.0);
    assert_eq!(assets.register_shader_binding(first).unwrap(), red_shader);
    assert_eq!(
        assets.shader_binding(red_shader).unwrap().material,
        material
    );
    assert_eq!(
        assets.shader_binding(blue_shader).unwrap().material,
        material
    );
    let diffuse = assets
        .register_shader_binding(ShaderBinding::material(material, LightSource::None))
        .unwrap();
    let two_d = assets
        .register_shader_binding(ShaderBinding::material(
            material,
            LightSource::TwoDimensional,
        ))
        .unwrap();
    assert_ne!(diffuse, two_d);

    let mut cpu = CpuBackend::load_with_assets(4, 2, &assets).unwrap();
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = frontend.begin_frame([0; 4]).unwrap();
    for (x, shader) in [(0., red_shader), (2., blue_shader)] {
        assert!(frame.draw_2d(Draw2d {
            rect: [x, 0., 2., 2.],
            shader,
            color: [255; 4],
            ..Default::default()
        }));
    }
    let packet = frame.finish();
    assert_eq!(cpu.render(&packet, &assets).rejected, 0);
    for row in cpu.pixels().chunks_exact(4) {
        assert_eq!(row, &[0xff0000ff, 0xff0000ff, 0xffff0000, 0xffff0000]);
    }
}

#[test]
fn draw_keys_keep_material_order_when_binding_admission_order_is_reversed() {
    let mut assets = Assets::load().unwrap();
    assets
        .register_material(
            "unused",
            &[stage(qa_render::ImageId(0))],
            Default::default(),
        )
        .unwrap();
    let low = assets
        .register_material("low", &[stage(qa_render::ImageId(0))], Default::default())
        .unwrap();
    let high = assets
        .register_material("high", &[stage(qa_render::ImageId(0))], Default::default())
        .unwrap();
    let high = assets
        .register_shader_binding(ShaderBinding::material(high, LightSource::None))
        .unwrap();
    let low = assets
        .register_shader_binding(ShaderBinding::material(low, LightSource::None))
        .unwrap();
    assert!(high.0 < low.0);
    let vertices = [[8., 8., 8.], [8., -8., 8.], [8., 0., -8.]].map(|position| Vertex {
        position: Vec3(position),
        ..Default::default()
    });
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = frontend.begin_frame([0; 4]).unwrap();
    assert!(frame.add_poly(high, &vertices));
    assert!(frame.add_poly(low, &vertices));
    assert!(frame.render_scene(
        Refdef {
            viewport: Viewport {
                width: 4,
                height: 4,
                ..Default::default()
            },
            ..Default::default()
        },
        &[],
        &assets
    ));
    let packet = frame.finish();
    let qa_render::Command::View(view) = packet.commands()[1] else {
        panic!("expected view")
    };
    let ordered: Vec<_> = packet
        .draws(view.scene.draws)
        .iter()
        .map(|draw| packet.poly(draw.index).shader)
        .collect();
    assert_eq!(ordered, [low, high]);
}

#[test]
fn capacity_and_partial_world_failure_restore_binding_heads_and_styled_provenance() {
    let mut assets = Assets::load_with_shader_capacity(3).unwrap();
    let material = assets
        .register_material(
            "retained",
            &[stage(qa_render::ImageId(0))],
            MaterialSettings {
                cull: Cull::None,
                ..Default::default()
            },
        )
        .unwrap();
    let retained_binding = ShaderBinding::material(material, LightSource::TwoDimensional);
    let retained = assets.register_shader_binding(retained_binding).unwrap();
    let lightmap = assets.register_image(1, 1, &[128; 4]).unwrap();
    let prefix = assets.counts();
    let candidate = SurfaceMaterial {
        shader: ShaderBinding {
            lightmap,
            lightmap_region: Some(AtlasRegion {
                page: 0,
                page_size: 1,
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            }),
            lightmap_conversion: Some(LightmapConversion::Rgb { modulate: 1. }),
            ..ShaderBinding::material(material, LightSource::Samples)
        },
        texture_scale: [1.; 2],
    };
    let (geometry, visibility) = world_surfaces::inputs(2);
    assert!(
        assets
            .register_world(geometry, visibility, &[&[candidate; 2]])
            .is_err()
    );
    assert_eq!(assets.counts(), prefix);
    assert!(
        assets
            .shader_binding(ShaderBindingId(prefix.shader_bindings as u32))
            .is_none()
    );
    assert_eq!(
        assets.register_shader_binding(retained_binding).unwrap(),
        retained
    );
    let retry = assets
        .register_shader_binding(ShaderBinding::material(material, LightSource::None))
        .unwrap();
    assert_eq!(retry.0 as usize, prefix.shader_bindings);
    let full = assets.counts();
    assert!(
        assets
            .register_shader_binding(ShaderBinding::material(material, LightSource::Vertex))
            .is_err()
    );
    assert_eq!(assets.counts(), full);
    assets.discard_registration_suffix(prefix).unwrap();
    assert_eq!(
        assets.register_shader_binding(retained_binding).unwrap(),
        retained
    );

    let (mut geometry, visibility) = world_surfaces::inputs(1);
    geometry.surfaces[0].light_source = LightSource::Samples;
    geometry.surfaces[0].lightmap_grid = [1; 2];
    geometry.surfaces[0].light_samples = qa_core::primitives::IndexSpan { first: 0, count: 1 };
    geometry.light_samples = vec![[128; 3]];
    let world = assets
        .register_world(geometry, visibility, &[&[candidate]])
        .unwrap();
    let presentation = assets.world(world).unwrap().presentations()[0];
    let shader = assets.surface_binding(presentation, 0).unwrap().shader;
    assert_eq!(shader, retry);
    assert_eq!(
        assets.shader_binding(shader).unwrap().styled_surface,
        Some((presentation, 0))
    );
    assert_eq!(assets.shader_binding(retained), Some(&retained_binding));
    let mut cpu = CpuBackend::load_with_assets(4, 4, &assets).unwrap();
    assert_eq!(cpu.admitted_assets(), Some(assets.counts()));
    cpu.prepare_assets(&assets, None).unwrap();
}
