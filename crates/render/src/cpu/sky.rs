//! Original indexed sky span kernels. WinQuake d_sky.c/r_sky.c use a
//! 32-pixel flattened screen ray and an integer-shifted masked layer.
//! ref_soft r_rast.c/r_edge.c use one unlit cube background per view.
use super::Camera;
use super::world::{Buffers, WorldStats};
use crate::assets::{Assets, CubeSkyParams, DepthFunc, ImageId, Material, Sky, StageTexture};
use crate::edges::Span;
use crate::scene::{CpuPresentation, Viewport};
use crate::shader::{AlphaFunc, AlphaGen, BlendFactor, RgbGen, TexCoordGen};
use crate::sky::{LayeredSphere, Rotation};
use crate::surface_cache::{IndexedMip, PaletteLighting};
use qa_core::primitives::Vec3;

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

pub(super) struct SkyState {
    pub clip: crate::sky::SkyClip,
    pub prepared: bool,
}
impl Default for SkyState {
    fn default() -> Self {
        Self {
            clip: crate::sky::SkyClip::new(),
            prepared: false,
        }
    }
}
impl SkyState {
    pub fn clear(&mut self) {
        self.clip.clear();
        self.prepared = false;
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
struct LayeredDraw {
    axes: [Vec3; 3],
    center: [i64; 2],
    extent: f32,
    sphere: LayeredSphere,
    scroll: f32,
    shift: i64,
}
impl LayeredDraw {
    fn load(camera: &Camera, width: u32, height: u32, sphere: LayeredSphere) -> Self {
        let seat = camera.refdef.blend_viewport.unwrap_or(Viewport {
            x: 0,
            y: 0,
            width,
            height,
        });
        // R_SetSkyFrame's native period is 512 seconds for speeds 8 and 2.
        let time = camera.refdef.time_ms as f32 * 0.001;
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
        let mut ray = std::array::from_fn::<_, 3, _>(|i| {
            4096.0 * self.axes[0].0[i] - wu * self.axes[1].0[i] + wv * self.axes[2].0[i]
        });
        ray[2] *= self.sphere.flatten_z;
        let inverse_length = 1.0 / (ray[0] * ray[0] + ray[1] * ray[1] + ray[2] * ray[2]).sqrt();
        ray = ray.map(|value| value * inverse_length);
        std::array::from_fn(|i| {
            ((self.scroll + self.sphere.projected_scale * ray[i]) * 65536.0) as i64
        })
    }
}

pub(super) fn layered_span(
    width: u32,
    height: u32,
    span: Span,
    source: Source,
    camera: &Camera,
    assets: &Assets,
    depth: [f32; 3],
    rank: u32,
    buffers: &mut Buffers<'_>,
    stats: &mut WorldStats,
) {
    let Source::Layered { images, sphere } = source else {
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
    let draw = LayeredDraw::load(camera, width, height, sphere);
    let mut x = span.x;
    let end = span.x + span.count;
    let mut current = draw.endpoint(x, span.y);
    while x < end {
        let count = 32.min(end - x);
        let complete = x + count < end;
        let distance = if complete { 32 } else { count - 1 };
        let next = draw.endpoint(x + distance, span.y);
        let step = if complete {
            std::array::from_fn(|i| (next[i] - current[i]) >> 5)
        } else if count > 1 {
            std::array::from_fn(|i| (next[i] - current[i]) / i64::from(count - 1))
        } else {
            [0; 2]
        };
        for offset in 0..count {
            let sx = (current[0] >> 16) & 127;
            let sy = (current[1] >> 16) & 127;
            let foreground = front.indices()
                [(((sy + draw.shift) & 127) * 128 + ((sx + draw.shift) & 127)) as usize];
            let color = if foreground == 0 {
                back.indices()[(sy * 128 + sx) as usize]
            } else {
                foreground
            };
            let px = x + offset;
            let zi = depth[2] + depth[1] * span.y as f32 + depth[0] * px as f32;
            let index = buffers.offset(width, px, span.y);
            if zi.is_finite()
                && zi > 0.0
                && super::depth_passes(
                    DepthFunc::Lequal,
                    zi,
                    buffers.inverse_depth[index],
                    rank,
                    buffers.depth_ranks[index],
                )
            {
                write(index, color, palette, palette_id, zi, rank, buffers);
                stats.pixels += 1;
            }
            current[0] += step[0];
            current[1] += step[1];
        }
        x += count;
        current = next;
    }
    stats.sky_pixels = stats
        .sky_pixels
        .saturating_add(stats.pixels.saturating_sub(before));
}

#[derive(Clone, Copy, Default)]
struct CubeGradient {
    x: f32,
    y: f32,
    origin: f32,
}
impl CubeGradient {
    fn at(self, x: u32, y: u32) -> f32 {
        self.origin + self.y * y as f32 + self.x * x as f32
    }
}
#[derive(Clone, Copy)]
struct CubePlanes {
    depth: CubeGradient,
    texture: [CubeGradient; 2],
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
    let scale = [
        camera.refdef.viewport.width as f32 / (2.0 * camera.tangent[0]),
        camera.refdef.viewport.height as f32 / (2.0 * camera.tangent[1]),
    ];
    let center = [
        camera.refdef.viewport.x as f32 + camera.refdef.viewport.width as f32 * 0.5 - 0.5,
        camera.refdef.viewport.y as f32 + camera.refdef.viewport.height as f32 * 0.5 - 0.5,
    ];
    let plane = |axis: Vec3, factor: f32| {
        let x = -axis.dot(axes[1]) * ((1.0 / scale[0]) * factor);
        let y = -axis.dot(axes[2]) * ((1.0 / scale[1]) * factor);
        CubeGradient {
            x,
            y,
            origin: axis.dot(axes[0]) * factor - center[0] * x - center[1] * y,
        }
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
                camera.refdef.axes.map(|axis| {
                    crate::sky::unrotate(axis, rotation, camera.refdef.time_ms as f32 * 0.001)
                })
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
            if planes[candidate].depth.at(x, y) > planes[face].depth.at(x, y) {
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
            let extent = [
                ((i64::from(mip.width)) << 16) - 1,
                ((i64::from(mip.height)) << 16) - 1,
            ];
            let endpoint = |divided: [f32; 2], zi: f32, minimum: i64| {
                let z = 65536.0 / zi;
                std::array::from_fn::<_, 2, _>(|i| {
                    ((divided[i] * z) as i64 + plane.adjust[i]).clamp(minimum, extent[i])
                })
            };
            let mut cursor = start;
            let mut zi = plane.depth.at(cursor, y);
            let mut divided = plane.texture.map(|value| value.at(cursor, y));
            let mut current = endpoint(divided, zi, 0);
            while cursor < x {
                let count = chunk.min(x - cursor);
                let complete = cursor + count < x;
                let distance = if complete { chunk } else { count - 1 } as f32;
                divided[0] += plane.texture[0].x * distance;
                divided[1] += plane.texture[1].x * distance;
                zi += plane.depth.x * distance;
                let next = endpoint(divided, zi, chunk as i64);
                let step = if complete {
                    std::array::from_fn(|i| (next[i] - current[i]) >> chunk.trailing_zeros())
                } else if count > 1 {
                    std::array::from_fn(|i| (next[i] - current[i]) / i64::from(count - 1))
                } else {
                    [0; 2]
                };
                for offset in 0..count {
                    let sx = (current[0] >> 16).clamp(0, i64::from(mip.width) - 1) as usize;
                    let sy = (current[1] >> 16).clamp(0, i64::from(mip.height) - 1) as usize;
                    let color = mip.indices()[sy * mip.width as usize + sx];
                    let index = buffers.offset(width, cursor + offset, y);
                    write(index, color, palette, palette_id, -0.9, 0, buffers);
                    stats.pixels += 1;
                    current[0] += step[0];
                    current[1] += step[1];
                }
                cursor += count;
                current = next;
            }
            stats.sky_pixels = stats
                .sky_pixels
                .saturating_add(stats.pixels.saturating_sub(before));
        }
    }
}

fn write(
    index: usize,
    color: u8,
    palette: &PaletteLighting,
    palette_id: u32,
    depth: f32,
    rank: u32,
    buffers: &mut Buffers<'_>,
) {
    buffers.pixels[index] = palette.color(color);
    buffers.indices[index] = color;
    buffers.palettes[index] = palette_id;
    buffers.inverse_depth[index] = depth;
    buffers.depth_ranks[index] = rank;
}
