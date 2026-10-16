//! Single-purpose correctness fixture. Run only inside an owned private display.
use qa_core::primitives::{Plane, Vec3};
use qa_platform::Window;
use qa_render::{
    Assets, FrontEnd, Limits, Refdef, SceneEntity, Vertex, Viewport,
    assets::{Cull, DepthFunc, MaterialSettings, Stage, StageTexture},
    cpu::CpuBackend,
    gl::{GlBackend, GlContext},
    scene::{MaterialOpacity, SceneModel},
    shader::{BlendFactor, StageBlend},
};

const WIDTH: u32 = 64;
const HEIGHT: u32 = 48;

#[test]
#[ignore = "requires an owned private GL display"]
fn gl_material_transactions_keep_complete_stage_results_depth_and_context_state() {
    assert!(std::env::var_os("QA_PRIVATE_GL").is_some());
    let window = Window::open_gl(WIDTH as i32, HEIGHT as i32).unwrap();
    let mut assets = Assets::load().unwrap();
    let base = assets.register_image(1, 1, &[200, 200, 200, 255]).unwrap();
    let light = assets.register_image(1, 1, &[128, 128, 128, 255]).unwrap();
    let material = {
        let material = assets
            .register_material(
                "opacity/complete",
                &[
                    Stage {
                        texture: StageTexture::Image(base),
                        ..Default::default()
                    },
                    Stage {
                        texture: StageTexture::Image(light),
                        blend: Some(StageBlend {
                            source: BlendFactor::DestinationColor,
                            destination: BlendFactor::Zero,
                        }),
                        depth_func: DepthFunc::Equal,
                        depth_write: false,
                        ..Default::default()
                    },
                ],
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
    let vertices = [
        Vec3([4., 4., 4.]),
        Vec3([4., -4., 4.]),
        Vec3([4., -4., -4.]),
        Vec3([4., 4., -4.]),
    ]
    .map(|position| Vertex {
        position,
        ..Default::default()
    });
    let model = assets
        .register_model(&vertices, &[0, 1, 2, 0, 2, 3], material)
        .unwrap();
    let blue = assets.register_image(1, 1, &[0, 0, 240, 255]).unwrap();
    let back_material = {
        let material = assets
            .register_material(
                "opacity/back-after",
                &[Stage {
                    texture: StageTexture::Image(blue),
                    ..Default::default()
                }],
                MaterialSettings {
                    cull: Cull::None,
                    sort: 7.,
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
    let mut cpu = CpuBackend::load(WIDTH, HEIGHT).unwrap();
    // SAFETY: The owned window's context stays current on this thread and
    // outlives all GPU objects. No driver calls enter a foreign context.
    let mut gl = unsafe {
        GlBackend::load(
            GlContext::load(
                |name| window.gl_proc(name),
                window.display_configuration().unwrap(),
            )
            .unwrap(),
            &assets,
            WIDTH,
            HEIGHT,
            1024,
        )
        .unwrap()
    };
    println!("GL identity {:?}", gl.renderer_info());
    let view = Refdef {
        viewport: Viewport {
            width: WIDTH,
            height: HEIGHT,
            ..Default::default()
        },
        fov: [90.; 2],
        near: 1.,
        far: 64.,
        ..Default::default()
    };
    let mut pixels = vec![0; (WIDTH * HEIGHT * 4) as usize];
    for (amount, expected) in [(0., 40u8), (0.25, 55), (0.5, 70), (1., 100)] {
        let mut front = FrontEnd::load(Limits::default()).unwrap();
        let mut frame = front.begin_frame([40, 40, 40, 255]).unwrap();
        assert!(frame.add_entity(SceneEntity {
            model: SceneModel::Mesh(model),
            material_opacity: MaterialOpacity::new(amount),
            ..Default::default()
        }));
        assert!(frame.render_scene(view, &[], &assets));
        let packet = frame.finish();
        assert_eq!(gl.render(&packet, &assets).rejected, 0);
        assert_eq!(cpu.render(&packet, &assets).rejected, 0);
        assert!(gl.read_pixels(&mut pixels));
        let center = ((HEIGHT / 2 * WIDTH + WIDTH / 2) * 4) as usize;
        for &channel in &pixels[center..center + 3] {
            assert!(
                channel.abs_diff(expected) <= 1,
                "opacity {amount}: {channel} != {expected}"
            );
        }
        assert_eq!(
            &cpu.pixels()[(HEIGHT / 2 * WIDTH + WIDTH / 2) as usize].to_le_bytes()[..3],
            &[expected; 3]
        );
        assert!(gl.take_error().is_none());
    }
    // A later opaque draw behind a faded draw must still pass depth testing.
    // It also proves program, sampler, viewport and state-cache restoration.
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = front.begin_frame([40, 40, 40, 255]).unwrap();
    assert!(frame.add_entity(SceneEntity {
        model: SceneModel::Mesh(model),
        material_opacity: MaterialOpacity::new(0.25),
        ..Default::default()
    }));
    assert!(frame.add_entity(SceneEntity {
        model: SceneModel::Mesh(model),
        shader: Some(back_material),
        origin: Vec3([4., 0., 0.]),
        ..Default::default()
    }));
    assert!(frame.render_scene(view, &[], &assets));
    let packet = frame.finish();
    assert_eq!(gl.render(&packet, &assets).rejected, 0);
    assert!(gl.read_pixels(&mut pixels));
    let center = ((HEIGHT / 2 * WIDTH + WIDTH / 2) * 4) as usize;
    assert_eq!(&pixels[center..center + 3], &[0, 0, 240]);
    // The private clip plane must preserve the untouched destination half.
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = front.begin_frame([40, 40, 40, 255]).unwrap();
    assert!(frame.add_entity(SceneEntity {
        model: SceneModel::Mesh(model),
        material_opacity: MaterialOpacity::new(0.25),
        ..Default::default()
    }));
    assert!(frame.render_scene(
        Refdef {
            clip_plane: Some(Plane::oriented(Vec3([0., 1., 0.]), 0.)),
            ..view
        },
        &[],
        &assets
    ));
    assert_eq!(gl.render(&frame.finish(), &assets).rejected, 0);
    assert!(gl.read_pixels(&mut pixels));
    let mut background = 0;
    let mut shaded = 0;
    for pixel in pixels.as_chunks::<4>().0 {
        if pixel[..3] == [40; 3] {
            background += 1;
        }
        if pixel[0].abs_diff(55) <= 1 {
            shaded += 1;
        }
    }
    assert!(background > 0 && shaded > 0);
    assert!(gl.take_error().is_none());
}
