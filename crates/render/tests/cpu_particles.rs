use qa_core::primitives::Vec3;
use qa_render::{
    Assets, BlendPhase, CpuPresentation, FrontEnd, Limits, PaletteId, PaletteShift,
    PaletteTransform, ParticlePresentation, Refdef, SceneParticle, Viewport,
    assets::{Cull, MaterialSettings, Stage, StageTexture, Vertex},
    cpu::CpuBackend,
    surface_cache::{IndexedLighting, PaletteLighting},
};

const BACKGROUND: [u8; 4] = [3, 5, 7, 255];

fn resources() -> (Assets, PaletteId) {
    let mut assets = Assets::load().unwrap();
    let rgb: Vec<_> = (0..=255u8)
        .flat_map(|index| [index, index.wrapping_mul(2), index.wrapping_mul(3)])
        .collect();
    let colormap: Vec<_> = (0..64).flat_map(|_| 0..=255u8).collect();
    let palette = assets
        .register_palette(PaletteLighting::load(&rgb, &colormap, None, 256).unwrap())
        .unwrap();
    (assets, palette)
}

fn view(viewport: Viewport, palette: PaletteId) -> Refdef {
    Refdef {
        viewport,
        fov: [90.0; 2],
        near: 0.25,
        far: 1_000_000.0,
        cpu_presentation: CpuPresentation::Indexed {
            palette,
            lighting: IndexedLighting::Gray,
            ambient: 0,
            fullbright: false,
        },
        ..Refdef::default()
    }
}

fn point(palette: PaletteId, position: [f32; 3], pixel_aspect: f32, color: u8) -> SceneParticle {
    SceneParticle {
        position: Vec3(position),
        palette,
        color,
        presentation: ParticlePresentation::Square { pixel_aspect },
        ..SceneParticle::default()
    }
}

fn draw(
    assets: &Assets,
    dimensions: [u32; 2],
    view: Refdef,
    particles: &[SceneParticle],
) -> CpuBackend {
    let mut frontend = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = frontend.begin_frame(BACKGROUND).unwrap();
    for particle in particles {
        assert!(frame.add_particle(*particle));
    }
    assert!(frame.render_scene(view, &[], assets));
    let mut cpu = CpuBackend::load(dimensions[0], dimensions[1]).unwrap();
    assert_eq!(cpu.render(&frame.finish(), assets).rejected, 0);
    cpu
}

// Independent d_part.c/d_modech.c arithmetic for an identity camera, fov90
// and native point size1. No engine projection/depth helper forms expectations.
fn original_footprint(
    viewport: Viewport,
    position: [f32; 3],
    aspect: f32,
) -> Option<(u32, u32, u32, u32, i32)> {
    let [depth, left, up] = position;
    if depth < 8.0 {
        return None;
    }
    let xscale = (viewport.width as f32 - 6.0) / 2.0;
    let yscale = xscale * aspect;
    let transformed_x = left * -xscale;
    let transformed_y = up * yscale;
    let zi = 1.0 / depth;
    let xcenter = viewport.width as f32 * 0.5 + viewport.x as f32 - 0.5;
    let ycenter = viewport.height as f32 * 0.5 + viewport.y as f32 - 0.5;
    let u = (f64::from(xcenter + zi * transformed_x) + 0.5) as i64;
    let v = (f64::from(ycenter - zi * transformed_y) + 0.5) as i64;
    let maximum = ((f64::from(viewport.width as f32) / 80.0 + 0.5) as i32).max(1);
    let minimum = (viewport.width as i32 / 320).max(1);
    let shift = 8 - (f64::from(viewport.width as f32) / 320.0 + 0.5) as i32;
    let aspect_shift = i32::from(f64::from(aspect) > 1.4);
    if u < i64::from(viewport.x)
        || v < i64::from(viewport.y)
        || u > i64::from(viewport.x + viewport.width) - i64::from(maximum)
        || v > i64::from(viewport.y + viewport.height) - i64::from(maximum << aspect_shift)
    {
        return None;
    }
    let izi = (zi * 32768.0) as i32;
    let pixels = (izi >> shift).clamp(minimum, maximum);
    Some((
        u as u32,
        v as u32,
        pixels as u32,
        (pixels << aspect_shift) as u32,
        izi,
    ))
}

#[test]
fn native_points_match_square_projection_size_aspect_and_palette_pixels() {
    let (assets, palette) = resources();
    for (width, height, aspect, position) in [
        (32, 24, 1.0, [8.0, -0.1, 0.15]),
        (160, 100, 5.0 / 6.0, [8.0, 1.0, 2.0]),
        (320, 200, 1.0, [8.0, 0.0, 0.0]),
        (320, 200, 1.4, [8.0, 0.0, 0.0]),
        (
            320,
            200,
            f32::from_bits(1.4f32.to_bits() + 1),
            [8.0, 0.0, 0.0],
        ),
        (320, 200, 1.5, [8.0, 0.0, 0.0]),
        (320, 200, 1.0, [4096.0, 0.0, 0.0]),
    ] {
        let viewport = Viewport {
            width,
            height,
            ..Viewport::default()
        };
        let cpu = draw(
            &assets,
            [width, height],
            view(viewport, palette),
            &[point(palette, position, aspect, 37)],
        );
        let (x, y, columns, rows, izi) = original_footprint(viewport, position, aspect).unwrap();
        let mut expected = vec![u32::from_le_bytes(BACKGROUND); width as usize * height as usize];
        for row in y..y + rows {
            for col in x..x + columns {
                expected[(row * width + col) as usize] = u32::from_le_bytes([37, 74, 111, 255]);
                assert_eq!(
                    cpu.inverse_depth()[(row * width + col) as usize].to_bits(),
                    (izi as f32 / 32768.0).to_bits()
                );
            }
        }
        assert_eq!(cpu.pixels(), expected);
    }
}

#[test]
fn native_near_clip_viewport_offset_and_conservative_maximum_bounds_survive() {
    let (assets, palette) = resources();
    let viewport = Viewport {
        x: 7,
        y: 5,
        width: 64,
        height: 48,
    };
    let cpu = draw(
        &assets,
        [80, 64],
        view(viewport, palette),
        &[point(palette, [8.0, 0.0, 0.0], 1.0, 37)],
    );
    let mut expected = vec![u32::from_le_bytes(BACKGROUND); 80 * 64];
    expected[29 * 80 + 39] = u32::from_le_bytes([37, 74, 111, 255]);
    assert_eq!(cpu.pixels(), expected);
    for position in [[7.999, 0.0, 0.0], [8.0, -100.0, 0.0], [8.0, 0.0, 100.0]] {
        let cpu = draw(
            &assets,
            [80, 64],
            view(viewport, palette),
            &[point(palette, position, 1.0, 37)],
        );
        assert!(
            cpu.pixels()
                .iter()
                .all(|pixel| *pixel == u32::from_le_bytes(BACKGROUND))
        );
    }
    let viewport = Viewport {
        width: 320,
        height: 200,
        ..Viewport::default()
    };
    let outside = [4096.0, -4096.0, 0.0]; // u317; conservative maximum bound316, despite actual size1.
    assert!(original_footprint(viewport, outside, 1.0).is_none());
    let cpu = draw(
        &assets,
        [320, 200],
        view(viewport, palette),
        &[point(palette, outside, 1.0, 37)],
    );
    assert!(
        cpu.pixels()
            .iter()
            .all(|pixel| *pixel == u32::from_le_bytes(BACKGROUND))
    );
}

#[test]
fn native_equal_integer_depth_bucket_overwrites_float_wall_but_nearer_bucket_occludes() {
    let (mut assets, palette) = resources();
    let wall_color = [9, 17, 29, 255];
    let image = assets.register_image(1, 1, &wall_color).unwrap();
    let material = assets
        .register_material(
            "particle wall",
            &[Stage {
                texture: StageTexture::Image(image),
                ..Stage::default()
            }],
            MaterialSettings {
                cull: Cull::None,
                ..MaterialSettings::default()
            },
        )
        .unwrap();
    for (wall_depth, visible) in [(7.99, false), (7.999, true), (8.0, true), (8.01, true)] {
        let vertices = [
            [wall_depth, -wall_depth, wall_depth],
            [wall_depth, wall_depth, wall_depth],
            [wall_depth, wall_depth, -wall_depth],
            [wall_depth, -wall_depth, -wall_depth],
        ]
        .map(|position| Vertex {
            position: Vec3(position),
            ..Vertex::default()
        });
        let mut frontend = FrontEnd::load(Limits::default()).unwrap();
        let mut frame = frontend.begin_frame(BACKGROUND).unwrap();
        assert!(frame.add_poly(material, &vertices));
        assert!(frame.add_particle(point(palette, [8.0, 0.0, 0.0], 1.0, 37)));
        assert!(frame.render_scene(
            Refdef {
                viewport: Viewport {
                    width: 32,
                    height: 32,
                    ..Viewport::default()
                },
                fov: [90.0; 2],
                near: 0.25,
                ..Refdef::default()
            },
            &[],
            &assets
        ));
        let mut cpu = CpuBackend::load(32, 32).unwrap();
        assert_eq!(cpu.render(&frame.finish(), &assets).rejected, 0);
        assert_eq!(
            cpu.pixels()[16 * 32 + 16],
            u32::from_le_bytes(if visible {
                [37, 74, 111, 255]
            } else {
                wall_color
            })
        );
        let original_wall = ((1.0f32 / wall_depth) * 32768.0) as i32;
        assert_eq!(original_wall <= 4096, visible);
    }
}

#[test]
fn serial_equal_depth_particles_and_zero_native_depth_keep_inclusive_ties() {
    let (assets, palette) = resources();
    let viewport = Viewport {
        width: 32,
        height: 32,
        ..Viewport::default()
    };
    let cpu = draw(
        &assets,
        [32, 32],
        view(viewport, palette),
        &[
            point(palette, [8.0, 0.0, 0.0], 1.0, 37),
            point(palette, [8.0, 0.0, 0.0], 1.0, 38),
        ],
    );
    assert_eq!(
        cpu.pixels()[16 * 32 + 16],
        u32::from_le_bytes([38, 76, 114, 255])
    );
    let cpu = draw(
        &assets,
        [32, 32],
        view(viewport, palette),
        &[point(palette, [65536.0, 0.0, 0.0], 1.0, 37)],
    );
    assert_eq!(
        cpu.pixels()[16 * 32 + 16],
        u32::from_le_bytes([37, 74, 111, 255])
    );
    assert_eq!(
        cpu.inverse_depth()[16 * 32 + 16].to_bits(),
        0.0f32.to_bits()
    );
}

#[test]
fn explicit_source_palette_supplies_rgb_and_native_final_palette_shift() {
    let (assets, source) = resources();
    let viewport = Viewport {
        width: 32,
        height: 32,
        ..Viewport::default()
    };
    let cpu = draw(
        &assets,
        [32, 32],
        Refdef {
            cpu_presentation: CpuPresentation::Rgb,
            ..view(viewport, source)
        },
        &[point(source, [8.0, 0.0, 0.0], 1.0, 37)],
    );
    assert_eq!(
        cpu.pixels()[16 * 32 + 16],
        u32::from_le_bytes([37, 74, 111, 255])
    );
    let mut transform = PaletteTransform::default();
    transform.shifts[0] = PaletteShift {
        destination: [200, 0, 100],
        percent: 128,
    };
    let cpu = draw(
        &assets,
        [32, 32],
        Refdef {
            blend_phase: BlendPhase::FinalPalette,
            palette_transform: Some(transform),
            ..view(viewport, source)
        },
        &[point(source, [8.0, 0.0, 0.0], 1.0, 37)],
    );
    assert_eq!(
        cpu.pixels()[16 * 32 + 16],
        u32::from_le_bytes([118, 37, 105, 255])
    );
}

#[test]
fn indexed_view_uses_the_particles_explicit_source_palette() {
    let (mut assets, source) = resources();
    let rgb = vec![255u8; 256 * 3];
    let colormap: Vec<_> = (0..64).flat_map(|_| 0..=255u8).collect();
    let unrelated_view = assets
        .register_palette(PaletteLighting::load(&rgb, &colormap, None, 256).unwrap())
        .unwrap();
    let viewport = Viewport {
        width: 32,
        height: 32,
        ..Viewport::default()
    };
    let cpu = draw(
        &assets,
        [32, 32],
        view(viewport, unrelated_view),
        &[point(source, [8.0, 0.0, 0.0], 1.0, 37)],
    );
    assert_eq!(
        cpu.pixels()[16 * 32 + 16],
        u32::from_le_bytes([37, 74, 111, 255])
    );
}
