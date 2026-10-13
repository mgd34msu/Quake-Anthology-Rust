//! Native software points over the scene's one particle arena and framebuffer.
use super::{Camera, composite, world};
use crate::BackendStats;
use crate::assets::{Assets, DepthFunc};
use crate::scene::{CpuPresentation, ParticlePresentation, SceneParticle};
use crate::shader::StageBlend;
use qa_core::primitives::Vec3;

const DEPTH_SCALE: f32 = 32768.0;
const Z_CLIP: f32 = 8.0;

#[derive(Clone, Copy)]
struct PointProjection {
    right: Vec3,
    up: Vec3,
    forward: Vec3,
    center: [f32; 2],
    minimum: u32,
    maximum: u32,
    shift: i32,
    aspect_shift: u32,
    size: f32,
}
impl PointProjection {
    fn load(camera: &Camera, pixel_aspect: f32, size: f32) -> Option<Self> {
        let width = camera.refdef.viewport.width;
        // R_DrawParticles scales the axes before DotProduct, using the
        // six-pixel-shrunk native horizontal FOV rather than xscale/yscale.
        let xscale = (width as f32 - 6.0) / (2.0 * camera.tangent[0]);
        let yscale = xscale * pixel_aspect;
        let native_minimum = (width / 320).max(1);
        let native_maximum = ((f64::from(width as f32) / (320.0 / 4.0) + 0.5) as u32).max(1);
        let projection = Self {
            right: -camera.refdef.axes[1] * xscale,
            up: camera.refdef.axes[2] * yscale,
            forward: camera.refdef.axes[0],
            center: camera.screen_center(),
            minimum: ((native_minimum as f32 * size) as u32).max(1),
            maximum: ((native_maximum as f32 * size) as u32).max(1),
            shift: 8 - (f64::from(width as f32) / 320.0 + 0.5) as i32,
            aspect_shift: u32::from(pixel_aspect > 1.4),
            size,
        };
        (projection.right.is_finite() && projection.up.is_finite()).then_some(projection)
    }

    fn footprint(self, camera: &Camera, position: Vec3) -> Option<(u32, u32, u32, u32, i32, f32)> {
        let local = position - camera.refdef.origin;
        let depth = local.dot(self.forward);
        if !depth.is_finite() || depth < Z_CLIP {
            return None;
        }
        let zi = 1.0 / depth;
        let u = (f64::from(self.center[0] + zi * local.dot(self.right)) + 0.5) as i64;
        let v = (f64::from(self.center[1] - zi * local.dot(self.up)) + 0.5) as i64;
        let viewport = camera.refdef.viewport;
        let maximum_height = self.maximum.checked_shl(self.aspect_shift)?;
        let right = i64::from(viewport.x) + i64::from(viewport.width) - i64::from(self.maximum);
        let bottom = i64::from(viewport.y) + i64::from(viewport.height) - i64::from(maximum_height);
        if u < i64::from(viewport.x) || v < i64::from(viewport.y) || u > right || v > bottom {
            return None;
        }
        let izi = quantized_depth(zi);
        let native_size = if self.shift >= 0 {
            izi.checked_shr(self.shift as u32).unwrap_or(0)
        } else {
            izi.checked_shl(self.shift.unsigned_abs())
                .unwrap_or(i32::MAX)
        };
        let width = ((native_size as f32 * self.size) as u32).clamp(self.minimum, self.maximum);
        Some((
            u as u32,
            v as u32,
            width,
            width << self.aspect_shift,
            izi,
            zi,
        ))
    }
}

fn quantized_depth(inverse_depth: f32) -> i32 {
    (inverse_depth * DEPTH_SCALE) as i32
}

pub(super) fn draw(
    camera: &Camera,
    width: u32,
    particles: &[SceneParticle],
    assets: &Assets,
    mut buffers: world::Buffers<'_>,
    stats: &mut BackendStats,
) {
    let mut previous = None;
    for particle in particles {
        if particle.alpha == 0 {
            continue;
        }
        let ParticlePresentation::Square { pixel_aspect } = particle.presentation else {
            // Material cards need their own admitted CPU stage capability.
            stats.rejected = stats.rejected.saturating_add(1);
            continue;
        };
        let Some(palette) = assets.palette(particle.palette) else {
            stats.rejected = stats.rejected.saturating_add(1);
            continue;
        };
        if matches!(
            camera.refdef.cpu_presentation,
            CpuPresentation::Indexed { .. }
        ) && particle.alpha != 255
        {
            stats.rejected = stats.rejected.saturating_add(1);
            continue;
        }
        let projection = match previous {
            Some((aspect, size, projection)) if aspect == pixel_aspect && size == particle.size => {
                projection
            }
            _ => {
                let Some(projection) = PointProjection::load(camera, pixel_aspect, particle.size)
                else {
                    stats.rejected = stats.rejected.saturating_add(1);
                    continue;
                };
                previous = Some((pixel_aspect, particle.size, projection));
                projection
            }
        };
        let Some((x, y, count, rows, izi, incoming_depth)) =
            projection.footprint(camera, particle.position)
        else {
            continue;
        };
        // The one framebuffer stores float wall depth. Quantize it only at the
        // native point boundary; particles write the exactly representable
        // native 15-bit reciprocal, keeping the original inclusive depth tie.
        let zi = izi as f32 / DEPTH_SCALE;
        let color = palette.color(particle.color);
        let mut rgba = crate::stage::unit_rgba(color.to_le_bytes());
        rgba[3] = f32::from(particle.alpha) / 255.0;
        for py in y..y + rows {
            for px in x..x + count {
                let index = buffers.offset(width, px, py);
                if !world::depth_gate(
                    incoming_depth,
                    izi as f32,
                    quantized_depth(buffers.inverse_depth[index]) as f32,
                    u32::MAX,
                    buffers.depth_ranks[index],
                    DepthFunc::Lequal,
                ) {
                    continue;
                }
                match camera.refdef.cpu_presentation {
                    CpuPresentation::Indexed { .. } => buffers.write_indexed(
                        index,
                        particle.color,
                        palette,
                        particle.palette.0,
                        Some((zi, u32::MAX)),
                    ),
                    CpuPresentation::Rgb => {
                        let pixel = if particle.alpha == 255 {
                            color
                        } else {
                            composite(buffers.pixels[index], rgba, Some(StageBlend::ALPHA))
                        };
                        buffers.write_rgba(index, pixel, Some((zi, u32::MAX)));
                    }
                }
            }
        }
    }
}
