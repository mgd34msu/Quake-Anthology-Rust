use super::*;
use crate::assets::{Sampler, Stage, StageTexture};
use crate::stage::{DrawInputs, StageEvaluator};
use crate::surface_cache::IndexedTexture;

// Literal scalar operations from Q1/Q2 R_AliasSetupLighting and the C
// transform loop. This does not duplicate packed MD2 position interpolation.
fn native_light(ambient: f32, shade: f32, normal: [f32; 3], direction: [f32; 3]) -> i32 {
    let mut light = ambient as i32;
    if light < 5 {
        light = 5;
    }
    light = (255 - light) << 6;
    if light < 5 {
        light = 5;
    }
    let mut shade = shade;
    if shade < 0.0 {
        shade = 0.0;
    }
    shade *= 64.0;
    let cosine = normal[0] * direction[0] + normal[1] * direction[1] + normal[2] * direction[2];
    if cosine < 0.0 {
        light += (shade * cosine) as i32;
        if light < 0 {
            light = 0;
        }
    }
    light
}

#[test]
fn alias_scalar_lighting_keeps_native_fixed_point_rounding_and_negative_dot_gate() {
    let directions = [[-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [-0.25, 0.5, -0.75]];
    let normals = [[1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [-0.0, 0.6, 0.8]];
    for policy in [IndexedLighting::Gray, IndexedLighting::BrightestRgb] {
        for ambient in [-10.0, 0.0, 5.0, 23.7, 80.0, 128.0, 255.0, 300.0] {
            for shade in [-1.0, 0.0, 15.5, 64.0, 128.0, 192.0] {
                for normal in normals {
                    for direction in directions {
                        let light = AliasLight::prepare(
                            EntityLighting {
                                ambient: [ambient; 3],
                                directed: [shade; 3],
                                direction: Vec3(direction),
                            },
                            policy,
                        )
                        .unwrap();
                        assert_eq!(
                            light.vertex(Vec3(normal)).to_bits(),
                            (native_light(ambient, shade, normal, direction) as f32).to_bits()
                        );
                    }
                }
            }
        }
    }
}

fn palette(with_alpha: bool) -> PaletteLighting {
    let colors: Vec<_> = (0..=255u8)
        .flat_map(|value| [value, 255 - value, value])
        .collect();
    let shades: Vec<_> = (0..64u8)
        .flat_map(|shade| (0..=255u8).map(move |index| index.wrapping_add(shade)))
        .collect();
    let alpha: Vec<_> = (0..=255u8)
        .flat_map(|major| (0..=255u8).map(move |minor| major.wrapping_mul(3).wrapping_add(minor)))
        .collect();
    PaletteLighting::load(
        &colors,
        &shades,
        with_alpha.then_some(alpha.as_slice()),
        224,
    )
    .unwrap()
}

fn prepare(
    assets: &Assets,
    image: crate::assets::ImageId,
    blend: Option<StageBlend>,
    fullbright: bool,
) -> Raster<'_> {
    let stage = Stage {
        texture: StageTexture::Image(image),
        blend,
        sampler: Sampler {
            wrap: Wrap::Clamp,
            ..Sampler::default()
        },
        ..Stage::default()
    };
    let prepared = StageEvaluator::load()
        .prepare(&stage, Default::default(), DrawInputs::default())
        .unwrap();
    Raster::prepare(
        CpuPresentation::Indexed {
            palette: PaletteId(0),
            lighting: IndexedLighting::Gray,
            ambient: 0,
            fullbright,
        },
        assets.image(image).unwrap(),
        &prepared,
        None,
        None,
        assets,
    )
    .unwrap()
    .unwrap()
}

#[test]
fn indexed_mesh_sampler_retains_raw_cutouts_colormap_rows_and_native_wrap() {
    let mut assets = Assets::load().unwrap();
    assets.register_palette(palette(false)).unwrap();
    let texture = IndexedTexture::load_base(2, 1, &[7, 255], Some(255)).unwrap();
    let image = assets
        .register_indexed_image(texture, PaletteId(0))
        .unwrap();
    assets
        .set_image_native_sampler(
            image,
            Sampler {
                wrap: Wrap::Repeat,
                ..Sampler::default()
            },
        )
        .unwrap();
    let raster = prepare(&assets, image, None, false);
    assert_eq!(
        raster.sample([0.25, 0.5], 5.0 * 256.0 + 255.0, 1.0, 0, u32::MAX),
        Some((12, true))
    );
    assert_eq!(
        raster.sample([1.25, 0.5], 5.0 * 256.0, 1.0, 0, u32::MAX),
        Some((12, true))
    );
    assert_eq!(raster.sample([0.75, 0.5], 0.0, 1.0, 0, u32::MAX), None);
    assert_eq!(
        raster.sample([0.25, 0.5], 63.0 * 256.0 + 255.0, 1.0, 0, u32::MAX),
        Some((70, true))
    );
    assert_eq!(
        raster.sample([0.25, 0.5], -1.0, 1.0, 0, u32::MAX),
        Some((7, true))
    );
    let unlit = prepare(&assets, image, None, true);
    assert_eq!(unlit.light(Vec3([0.0, 0.0, 1.0]), [0; 4]), 0.0);
}

#[test]
fn indexed_mesh_translucency_uses_native_one_third_and_two_thirds_table_order() {
    let mut assets = Assets::load().unwrap();
    assets.register_palette(palette(true)).unwrap();
    let image = assets
        .register_indexed_image(
            IndexedTexture::load_base(1, 1, &[7], None).unwrap(),
            PaletteId(0),
        )
        .unwrap();
    let raster = prepare(&assets, image, Some(StageBlend::ALPHA), false);
    assert_eq!(
        raster.sample([0.0; 2], 0.0, 0.33, 19, 0),
        Some((19 * 3 + 7, true))
    );
    assert_eq!(
        raster.sample([0.0; 2], 0.0, 0.66, 19, 0),
        Some((7 * 3 + 19, true))
    );
    assert_eq!(raster.sample([0.0; 2], 0.0, 1.0, 19, 0), Some((7, true)));
    assert_eq!(raster.sample([0.0; 2], 0.0, 0.0, 19, 0), None);
    assert_eq!(raster.sample([0.0; 2], 0.0, 0.66, 19, u32::MAX), None);
}

#[test]
fn indexed_light_selection_keeps_native_ties_and_rejects_nonfinite_inputs() {
    assert_eq!(IndexedLighting::Gray.scalar([100, 80, 60]), 100);
    assert_eq!(IndexedLighting::NativeRgb.scalar([100, 100, 60]), 60);
    assert_eq!(IndexedLighting::BrightestRgb.scalar([100, 100, 60]), 100);
    for lighting in [
        EntityLighting {
            ambient: [f32::NAN; 3],
            ..Default::default()
        },
        EntityLighting {
            directed: [f32::INFINITY; 3],
            ..Default::default()
        },
        EntityLighting {
            direction: Vec3([0.0, f32::NAN, 1.0]),
            ..Default::default()
        },
    ] {
        assert!(AliasLight::prepare(lighting, IndexedLighting::Gray).is_none());
    }
}

#[test]
fn q2_alias_policy_keeps_native_alpha_cutoffs_map_order_and_depth_writes() {
    let mut assets = Assets::load().unwrap();
    assets.register_palette(palette(true)).unwrap();
    let image = assets
        .register_indexed_image(
            IndexedTexture::load_base(1, 1, &[7], None).unwrap(),
            PaletteId(0),
        )
        .unwrap();
    let stage = Stage {
        texture: StageTexture::Image(image),
        blend: Some(StageBlend::ALPHA),
        depth_write: false, // GL translucent depth policy stays independent.
        ..Stage::default()
    };
    let prepared = StageEvaluator::load()
        .prepare(&stage, Default::default(), DrawInputs::default())
        .unwrap();
    let presentation = CpuPresentation::Indexed {
        palette: PaletteId(0),
        lighting: IndexedLighting::Gray,
        ambient: 0,
        fullbright: false,
    };
    let raster = Raster::prepare(
        presentation,
        assets.image(image).unwrap(),
        &prepared,
        Some(SoftwareAlias {
            missing_skin: false,
            infrared_map: None,
            fullbright_light: None,
            alpha_cutoffs: Some([0.33, 0.66]),
        }),
        None,
        &assets,
    )
    .unwrap()
    .unwrap();
    for alpha in [0.0, 0.1, 0.33] {
        assert_eq!(
            raster.sample([0.0; 2], 0.0, alpha, 19, 0),
            Some((19 * 3 + 7, false))
        );
    }
    for alpha in [f32::from_bits(0.33f32.to_bits() + 1), 0.5, 0.66] {
        assert_eq!(
            raster.sample([0.0; 2], 0.0, alpha, 19, 0),
            Some((7 * 3 + 19, true))
        );
    }
    for alpha in [f32::from_bits(0.66f32.to_bits() + 1), 0.9, 1.0] {
        assert_eq!(
            raster.sample([0.0; 2], 0.0, alpha, 19, u32::MAX),
            Some((7, true))
        );
    }
    assert_eq!(raster.sample([0.0; 2], 0.0, 0.5, 19, u32::MAX), None);
    let ordinary = Raster::prepare(
        presentation,
        assets.image(image).unwrap(),
        &prepared,
        None,
        None,
        &assets,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        ordinary.sample([0.0; 2], 0.0, 0.9, 19, 0),
        Some((7 * 3 + 19, false))
    );
    assert_eq!(ordinary.sample([0.0; 2], 0.0, 0.0, 19, 0), None);
}

#[test]
fn q2_alias_fullbright_retains_native_clamps_and_the_supplied_local_direction() {
    let mut assets = Assets::load().unwrap();
    assets.register_palette(palette(false)).unwrap();
    let image = assets
        .register_indexed_image(
            IndexedTexture::load_base(1, 1, &[7], None).unwrap(),
            PaletteId(0),
        )
        .unwrap();
    let stage = Stage {
        texture: StageTexture::Image(image),
        rgb_gen: RgbGen::Identity,
        ..Stage::default()
    };
    let prepared = StageEvaluator::load()
        .prepare(&stage, Default::default(), DrawInputs::default())
        .unwrap();
    let direction = Vec3([-1.0, 0.0, 0.0]);
    let raster = Raster::prepare(
        CpuPresentation::Indexed {
            palette: PaletteId(0),
            lighting: IndexedLighting::Gray,
            ambient: 0,
            fullbright: false,
        },
        assets.image(image).unwrap(),
        &prepared,
        Some(SoftwareAlias {
            missing_skin: false,
            infrared_map: None,
            fullbright_light: Some([128.0, 64.0]),
            alpha_cutoffs: None,
        }),
        Some(EntityLighting {
            ambient: [0.0; 3],
            directed: [0.0; 3],
            direction,
        }),
        &assets,
    )
    .unwrap()
    .unwrap();
    for normal in [[1.0, 0.0, 0.0], [-1.0, 0.0, 0.0], [0.5, 0.0, 0.75]] {
        assert_eq!(
            raster.light(Vec3(normal), [255; 4]).to_bits(),
            (native_light(128.0, 64.0, normal, direction.0) as f32).to_bits()
        );
    }
    assert_eq!(raster.light(Vec3([1.0, 0.0, 0.0]), [255; 4]), 4032.0);
    assert_eq!(raster.light(Vec3([-1.0, 0.0, 0.0]), [255; 4]), 8128.0);
}
