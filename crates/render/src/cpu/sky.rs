//! Original indexed sky span kernels. WinQuake d_sky.c/r_sky.c use a
//! 32-pixel flattened screen ray and an integer-shifted masked layer.
//! ref_soft r_rast.c/r_edge.c use one unlit cube background per view.
use super::Camera;
use super::world::{Affine, Buffers, WorldStats};
use crate::assets::{Assets, CubeSkyParams, DepthFunc, ImageId, Material, Sky, StageTexture};
use crate::edges::Span;
use crate::scene::{CpuPresentation, Viewport};
use crate::shader::{AlphaFunc, AlphaGen, BlendFactor, RgbGen, TexCoordGen};
use crate::sky::{LayeredSphere, Rotation};
use crate::surface_cache::{IndexedMip, PaletteLighting};
use qa_core::{math, primitives::Vec3};

/// tr_sky.c clips source geometry once per material, then consumes a fixed
/// eight-division cloud grid. Its triangles are boundary inputs to the same
/// world edge scanner, rather than a second world rasterizer.
pub(super) struct SkyDefinition {
    pub cloud: Option<Box<crate::sky::CloudGrid>>,
    pub enabled: bool,
}
impl SkyDefinition {
    pub fn load(material: &Material) -> Result<Self, &'static str> {
        let enabled = matches!(material.settings.sky, Some(Sky::Cube { params, .. }) if !params.cpu_background)
            && material.settings.fog.is_none()
            && !material.settings.portal
            && !material.settings.polygon_offset;
        let cloud = if enabled && !material.stages.is_empty() {
            let Some(Sky::Cube { clouds, .. }) = material.settings.sky else {
                return Err("invalid cloud sky material");
            };
            Some(Box::new(crate::sky::CloudGrid::generate(clouds)?))
        } else {
            None
        };
        Ok(Self { cloud, enabled })
    }
}

pub(super) const CLOUD_BOUNDARIES: usize = 5 * 8 * 8 * 2;
pub(super) const BOX_BOUNDARIES: usize = 6;
pub(super) const SKY_EDGES: usize = CLOUD_BOUNDARIES * 9 + BOX_BOUNDARIES * 10;

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Source {
    Layered {
        images: [ImageId; 2],
        sphere: LayeredSphere,
    },
    BackgroundCube {
        images: [ImageId; 6],
        rotation: Option<Rotation>,
        params: CubeSkyParams,
    },
}

impl Source {
    /// Assets are frozen before backend construction. Unsupported scripted
    /// sky effects remain rejected, rather than silently losing their stages.
    pub(super) fn load(material: &Material, assets: &Assets) -> Option<Self> {
        let settings = material.settings;
        if settings.fog.is_some()
            || settings.portal
            || settings.polygon_offset
            || settings.deforms.iter().any(Option::is_some)
            || settings.time_offset != 0.0
            || settings.clamp_time.is_some()
        {
            return None;
        }
        match settings.sky? {
            Sky::Layered { images, sphere } => {
                if !sphere.valid() || sphere.texture_size != 128.0 || material.stages.len() != 2 {
                    return None;
                }
                for (layer, stage) in material.stages.iter().enumerate() {
                    let image = assets.image(images[layer])?;
                    let texture = image.indexed.as_ref()?;
                    let mip = texture.mip(0)?;
                    if mip.width != 128
                        || mip.height != 128
                        || texture.transparent_index() != if layer == 0 { None } else { Some(0) }
                        || stage.texture != StageTexture::Image(images[layer])
                        || stage.texgen
                            != (TexCoordGen::LayeredSky {
                                flatten_z: sphere.flatten_z,
                                projected_scale: sphere.projected_scale,
                                texture_size: sphere.texture_size,
                                scroll_speed: sphere.scroll_speeds[layer],
                            })
                        || stage.rgb_gen != RgbGen::Identity
                        || stage.alpha_gen != AlphaGen::Identity
                        || stage.alpha_test != AlphaFunc::None
                        || stage.tcmods.iter().any(Option::is_some)
                        || stage.depth_func != DepthFunc::Lequal
                        || stage.depth_write != (layer == 0)
                        || stage.detail
                        || stage.sampler.mipmaps
                        || stage.sampler.wrap != crate::assets::Wrap::Repeat
                        || if layer == 0 {
                            stage.blend.is_some()
                        } else {
                            stage.blend.is_none_or(|blend| {
                                blend.source != BlendFactor::SourceAlpha
                                    || blend.destination != BlendFactor::OneMinusSourceAlpha
                            })
                        }
                    {
                        return None;
                    }
                }
                Some(Self::Layered { images, sphere })
            }
            Sky::Cube {
                outer_box: Some(images),
                inner_box: None,
                rotation,
                params,
                ..
            } if params.cpu_background && material.stages.is_empty() => {
                for image in images {
                    let texture = assets.image(image)?.indexed.as_ref()?;
                    texture.mip(0)?;
                    if texture.cutout() {
                        return None;
                    }
                }
                Some(Self::BackgroundCube {
                    images,
                    rotation,
                    params,
                })
            }
            _ => None,
        }
    }
}

fn presentation<'a>(assets: &'a Assets, camera: &Camera) -> Option<(&'a PaletteLighting, u32)> {
    let CpuPresentation::Indexed { palette, .. } = camera.refdef.cpu_presentation else {
        return None;
    };
    Some((assets.palette(palette)?, palette.0))
}

fn base(assets: &Assets, image: ImageId) -> Option<&IndexedMip> {
    assets.image(image)?.indexed.as_ref()?.mip(0)
}

/// Native screen rays use the full video/seat integer center and viewport
/// extent, independently of FOV. The extra cloud shift truncates separately.
pub(super) struct LayeredDraw {
    axes: [Vec3; 3],
    center: [i64; 2],
    extent: f32,
    sphere: LayeredSphere,
    scroll: f32,
    shift: i64,
}
impl LayeredDraw {
    pub(super) fn prepare(
        source: Source,
        camera: &Camera,
        width: u32,
        height: u32,
    ) -> Option<Self> {
        let Source::Layered { sphere, .. } = source else {
            return None;
        };
        Some(Self::load(camera, width, height, sphere))
    }
    fn load(camera: &Camera, width: u32, height: u32, sphere: LayeredSphere) -> Self {
        let seat = camera.refdef.blend_viewport.unwrap_or(Viewport {
            x: 0,
            y: 0,
            width,
            height,
        });
        // R_SetSkyFrame's native period is 512 seconds for speeds 8 and 2.
        let time = camera.refdef.time_seconds();
        let time = time - (time / 512.0).trunc() * 512.0;
        Self {
            axes: camera.refdef.axes,
            center: [
                i64::from(seat.x) + i64::from(seat.width >> 1),
                i64::from(seat.y) + i64::from(seat.height >> 1),
            ],
            extent: camera
                .refdef
                .viewport
                .width
                .max(camera.refdef.viewport.height) as f32,
            sphere,
            scroll: time * sphere.scroll_speeds[0],
            shift: (time * (sphere.scroll_speeds[1] - sphere.scroll_speeds[0])) as i64,
        }
    }
    fn endpoint(&self, x: u32, y: u32) -> [i64; 2] {
        let wu = 8192.0 * (i64::from(x) - self.center[0]) as f32 / self.extent;
        let wv = 8192.0 * (self.center[1] - i64::from(y)) as f32 / self.extent;
        let mut ray = self.axes[0] * 4096.0 - self.axes[1] * wu + self.axes[2] * wv;
        ray.0[2] *= self.sphere.flatten_z;
        // The previous reciprocal kernel casts its zero-ray NaNs to zero.
        if ray.0 == [0.0; 3] {
            return [0; 2];
        }
        let ray = math::normalized(ray);
        std::array::from_fn(|i| {
            ((self.scroll + self.sphere.projected_scale * ray.0[i]) * 65536.0) as i64
        })
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "Keep independent validated draw and span inputs explicit at the raster dispatch boundary"
)]
pub(super) fn layered_span(
    width: u32,
    span: Span,
    source: Source,
    camera: &Camera,
    assets: &Assets,
    depth: Affine,
    rank: u32,
    draw: &LayeredDraw,
    buffers: &mut Buffers<'_>,
    stats: &mut WorldStats,
) {
    let Source::Layered { images, .. } = source else {
        stats.rejected += 1;
        return;
    };
    let Some((palette, palette_id)) = presentation(assets, camera) else {
        stats.rejected += 1;
        return;
    };
    let (Some(back), Some(front)) = (base(assets, images[0]), base(assets, images[1])) else {
        stats.rejected += 1;
        return;
    };
    let before = stats.pixels;
    stats.sky_spans = stats.sky_spans.saturating_add(1);
    super::span::walk(
        span.x,
        span.count,
        32,
        |x, _, _| draw.endpoint(x, span.y),
        |px, current| {
            let sx = (current[0] >> 16) & 127;
            let sy = (current[1] >> 16) & 127;
            let foreground = front.indices()
                [(((sy + draw.shift) & 127) * 128 + ((sx + draw.shift) & 127)) as usize];
            let color = if foreground == 0 {
                back.indices()[(sy * 128 + sx) as usize]
            } else {
                foreground
            };
            let zi = depth.at(px as f32, span.y as f32);
            let index = buffers.offset(width, px, span.y);
            if buffers.depth_test(index, zi, zi, rank, DepthFunc::Lequal) {
                buffers.write_indexed(index, color, palette, palette_id, Some((zi, rank)));
                stats.pixels += 1;
            }
        },
    );
    stats.sky_pixels = stats
        .sky_pixels
        .saturating_add(stats.pixels.saturating_sub(before));
}

#[derive(Clone, Copy)]
struct CubePlanes {
    depth: Affine,
    texture: [Affine; 2],
    adjust: [i64; 2],
}

/// Cube planes match ref_soft's 128-unit box and texture vectors. Retained
/// image dimensions scale the native 256-texel extent, independently of TGA.
fn cube_planes(
    camera: &Camera,
    axes: [Vec3; 3],
    face: usize,
    width: u32,
    height: u32,
) -> CubePlanes {
    const NORMAL: [[f32; 3]; 6] = [
        [1., 0., 0.],
        [-1., 0., 0.],
        [0., 1., 0.],
        [0., -1., 0.],
        [0., 0., 1.],
        [0., 0., -1.],
    ];
    const TEXTURE: [[[f32; 3]; 2]; 6] = [
        [[0., -1., 0.], [0., 0., -1.]],
        [[0., 1., 0.], [0., 0., -1.]],
        [[1., 0., 0.], [0., 0., -1.]],
        [[-1., 0., 0.], [0., 0., -1.]],
        [[0., -1., 0.], [1., 0., 0.]],
        [[0., -1., 0.], [-1., 0., 0.]],
    ];
    let plane = |axis: Vec3, factor: f32| {
        // Sky axes may be independently rotated; preserve post-dot negation.
        let axis = super::view_point(Vec3::default(), axes, axis);
        camera.gradient(axis, factor, super::GradientForm::FactorFolded)
    };
    CubePlanes {
        depth: plane(Vec3(NORMAL[face]), 1.0 / 128.0),
        texture: [
            plane(Vec3(TEXTURE[face][0]), width as f32 / 256.0),
            plane(Vec3(TEXTURE[face][1]), height as f32 / 256.0),
        ],
        adjust: [i64::from(width) << 15, i64::from(height) << 15],
    }
}

#[derive(Clone, Copy)]
pub(super) struct BackgroundDraw {
    images: [ImageId; 6],
    planes: [CubePlanes; 6],
}
impl BackgroundDraw {
    pub(super) fn prepare(source: Source, camera: &Camera, assets: &Assets) -> Option<Self> {
        let Source::BackgroundCube {
            images,
            rotation,
            params,
        } = source
        else {
            return None;
        };
        let mips = images.map(|image| base(assets, image));
        if mips.iter().any(Option::is_none) {
            return None;
        }
        let axes = if params.cpu_rotation {
            rotation.map_or(camera.refdef.axes, |rotation| {
                camera
                    .refdef
                    .axes
                    .map(|axis| crate::sky::unrotate(axis, rotation, camera.refdef.time_seconds()))
            })
        } else {
            camera.refdef.axes
        };
        let planes = std::array::from_fn(|face| {
            let (w, h) = mips[face].map_or((1, 1), |mip| (mip.width, mip.height));
            cube_planes(camera, axes, face, w, h)
        });
        Some(Self { images, planes })
    }
}

pub(super) fn background(
    width: u32,
    draw: BackgroundDraw,
    camera: &Camera,
    assets: &Assets,
    buffers: &mut Buffers<'_>,
    stats: &mut WorldStats,
) {
    let Some((palette, palette_id)) = presentation(assets, camera) else {
        stats.rejected += 1;
        return;
    };
    let mips = draw.images.map(|image| base(assets, image));
    if mips.iter().any(Option::is_none) {
        stats.rejected += 1;
        return;
    }
    let planes = draw.planes;
    let viewport = camera.refdef.viewport;
    let face_at = |x, y| {
        let mut face = 0;
        for candidate in 1..6 {
            if planes[candidate].depth.at(x as f32, y as f32)
                > planes[face].depth.at(x as f32, y as f32)
            {
                face = candidate;
            }
        }
        face
    };
    let chunk = camera.refdef.perspective_step.pixels();
    let Some(rows) = buffers.rows(width, viewport) else {
        return;
    };
    for y in rows {
        let mut x = viewport.x;
        while x < viewport.x + viewport.width {
            let face = face_at(x, y);
            let start = x;
            while x < viewport.x + viewport.width && face_at(x, y) == face {
                x += 1;
            }
            stats.spans = stats.spans.saturating_add(1);
            let Some(mip) = mips[face] else {
                stats.rejected += 1;
                continue;
            };
            let before = stats.pixels;
            stats.sky_spans = stats.sky_spans.saturating_add(1);
            let plane = planes[face];
            let mut perspective = super::span::Perspective::new(
                plane.depth,
                plane.texture,
                plane.adjust,
                [mip.width, mip.height],
                start,
                y,
            );
            super::span::walk(
                start,
                x - start,
                chunk,
                |_, distance, minimum| perspective.endpoint(distance, minimum),
                |px, current| {
                    let color = mip.indices()[super::span::texel(current, mip.width, mip.height)];
                    let index = buffers.offset(width, px, y);
                    buffers.write_indexed(index, color, palette, palette_id, Some((-0.9, 0)));
                    stats.pixels += 1;
                },
            );
            stats.sky_pixels = stats
                .sky_pixels
                .saturating_add(stats.pixels.saturating_sub(before));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_endpoints_preserve_previous_operation_order() {
        let mut seed = 0x534b5955u32;
        let mut next = || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed as i32 as f32) * (1.0 / i32::MAX as f32)
        };
        for index in 0..16384 {
            let draw = LayeredDraw {
                axes: std::array::from_fn(|_| Vec3(std::array::from_fn(|_| next()))),
                center: [320, 200],
                extent: 640.0,
                sphere: LayeredSphere::NATIVE,
                scroll: next() * 8192.0,
                shift: 0,
            };
            let x = index % 641;
            let y = index % 401;
            let wu = 8192.0 * (i64::from(x) - draw.center[0]) as f32 / draw.extent;
            let wv = 8192.0 * (draw.center[1] - i64::from(y)) as f32 / draw.extent;
            let mut ray = std::array::from_fn::<_, 3, _>(|i| {
                4096.0 * draw.axes[0].0[i] - wu * draw.axes[1].0[i] + wv * draw.axes[2].0[i]
            });
            ray[2] *= draw.sphere.flatten_z;
            let inverse = 1.0 / (ray[0] * ray[0] + ray[1] * ray[1] + ray[2] * ray[2]).sqrt();
            let expected: [i64; 2] = std::array::from_fn(|i| {
                ((draw.scroll + draw.sphere.projected_scale * (ray[i] * inverse)) * 65536.0) as i64
            });
            assert_eq!(draw.endpoint(x, y), expected, "ray {index}");
        }
    }

    #[test]
    fn zero_ray_keeps_the_previous_integer_cast_result() {
        let draw = LayeredDraw {
            axes: [
                Vec3([1.0, 0.0, 0.0]),
                Vec3([1.0, 0.0, 0.0]),
                Vec3::default(),
            ],
            center: [320, 200],
            extent: 640.0,
            sphere: LayeredSphere::NATIVE,
            scroll: 32.0,
            shift: 0,
        };
        assert_eq!(draw.endpoint(640, 200), [0; 2]);
    }

    #[test]
    fn original_c_integer_endpoints() -> Result<(), Box<dyn std::error::Error>> {
        let Some(path) = std::env::var_os("QA_SKY_ENDPOINT_FIXTURE") else {
            return Ok(());
        };
        let bytes = std::fs::read(path)?;
        assert_eq!(bytes.len() % 72, 0);
        for (index, row) in bytes.as_chunks::<72>().0.iter().enumerate() {
            let word = |at: usize| u32::from_le_bytes(std::array::from_fn(|i| row[at + i]));
            let (width, height, x, y) = (word(0), word(4), word(8), word(12));
            let draw = LayeredDraw {
                axes: std::array::from_fn(|axis| {
                    Vec3(std::array::from_fn(|i| {
                        f32::from_bits(word(16 + (axis * 3 + i) * 4))
                    }))
                }),
                center: [i64::from(width >> 1), i64::from(height >> 1)],
                extent: width.max(height) as f32,
                sphere: LayeredSphere::NATIVE,
                scroll: f32::from_bits(word(52)) * 8.0,
                shift: 0,
            };
            let expected: [i64; 2] = std::array::from_fn(|i| {
                i64::from_le_bytes(std::array::from_fn(|j| row[56 + i * 8 + j]))
            });
            assert_eq!(draw.endpoint(x, y), expected, "original-C ray {index}");
        }
        Ok(())
    }
}
