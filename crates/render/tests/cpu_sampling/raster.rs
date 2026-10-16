use super::super::*;
use crate::assets::{MaterialSettings, Stage, StageTexture};

#[test]
fn mesh_uses_shared_sample_depth_gate_without_changing_depth_policy() {
    let background = u32::from_le_bytes([11, 23, 37, 255]);
    let mut assets = Assets::load().unwrap();
    let image_id = assets.register_image(1, 1, &[255; 4]).unwrap();
    let camera = Camera::load(
        Refdef {
            viewport: Viewport {
                width: 2,
                height: 2,
                ..Viewport::default()
            },
            near: 0.25,
            far: 64.0,
            ..Refdef::default()
        },
        2,
        2,
    )
    .unwrap();
    for (zi, stored, stored_rank, rank, func, depth_hack, writes) in [
        (-1.0, 0.0, 0, 8, DepthFunc::Always, false, false),
        (-1.0, 0.0, 0, 8, DepthFunc::Always, true, false),
        (0.0, 0.0, 0, 8, DepthFunc::Always, true, false),
        (0.25, 0.5, 7, 8, DepthFunc::Lequal, false, false),
        (0.25, 0.125, 7, 8, DepthFunc::Lequal, false, true),
        (0.25, 0.25, 7, 6, DepthFunc::Lequal, false, false),
        (0.25, 0.25, 7, 7, DepthFunc::Lequal, false, true),
        (0.25, 0.25, 7, 6, DepthFunc::Equal, false, false),
        (0.25, 0.25, 7, 7, DepthFunc::Equal, false, true),
        (0.25, 0.5, 7, 8, DepthFunc::Equal, false, false),
        (0.25, 0.5, 7, 6, DepthFunc::Always, false, true),
        (0.25, 1.5, 7, 8, DepthFunc::Lequal, true, true),
    ] {
        let material_id = assets
            .register_material(
                "mesh depth gate",
                &[Stage {
                    texture: StageTexture::Image(image_id),
                    sampler: Sampler {
                        filter: Filter::Nearest,
                        mipmaps: false,
                        ..Sampler::default()
                    },
                    depth_func: func,
                    depth_write: true,
                    ..Stage::default()
                }],
                MaterialSettings {
                    cull: Cull::None,
                    ..MaterialSettings::default()
                },
            )
            .unwrap();
        let material = assets.material(material_id).unwrap();
        let image = assets.image(image_id).unwrap();
        let mut cpu = CpuBackend::load(2, 2).unwrap();
        cpu.frame.pixels.fill(background);
        cpu.frame.inverse_depth.fill(stored);
        cpu.frame.depth_ranks.fill(stored_rank);
        let stage = cpu
            .evaluator
            .prepare(
                &material.stages[0],
                material.settings,
                DrawInputs::for_view(&camera.refdef),
            )
            .unwrap();
        let vertices = [[0.0, 0.0], [2.0, 0.0], [0.0, 2.0]].map(|xy| ScreenVertex {
            xy,
            inverse_depth: zi,
            color_over_depth: [zi; 4],
            ..ScreenVertex::default()
        });
        let mut stats = BackendStats::default();
        cpu.raster_triangle(
            &camera,
            vertices,
            RasterPass {
                settings: material.settings,
                stage,
                image,
                sampler: stage_sampler(image, stage.stage.sampler),
                depth_hack,
                first_stage: true,
                draw_rank: rank,
                indexed: None,
            },
            &mut stats,
        );
        assert_eq!(stats.rejected, 0);
        assert_eq!(
            cpu.frame.pixels[0] == u32::MAX,
            writes,
            "{zi} {func:?} {depth_hack}"
        );
        if writes {
            let depth = if depth_hack {
                zi * 0.3 + 0.7 / camera.refdef.near
            } else {
                zi
            };
            assert_eq!(cpu.frame.inverse_depth[0].to_bits(), depth.to_bits());
            assert_eq!(cpu.frame.depth_ranks[0], rank);
        } else {
            assert_eq!(cpu.frame.pixels[0], background);
            assert_eq!(cpu.frame.inverse_depth[0].to_bits(), stored.to_bits());
            assert_eq!(cpu.frame.depth_ranks[0], stored_rank);
        }
    }
}
