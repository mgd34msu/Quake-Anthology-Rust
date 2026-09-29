//! True-color depth fog for the CPU backend.
//!
//! Donor provenance: `src/render/cpu/fog.ts` in full — Q1 depth fog adapted
//! from `quake-1-re-ts` `ref_soft/r_fog.ts` (copyright (C) 2002-2009 John
//! Fitzgibbons and others, (C) 2010-2014 QuakeSpasm developers) and Q2
//! rerelease fog following `quake-2-re-ts` `ref_gl/gl_fog.ts` `GL_DrawFogPass`.

use qa_core::math::{Mat4, Vec3};

use super::super::types::{Q2FogOperation, Rect};
use crate::materials::fog::{
    global_fog_amount, height_fog_amount, height_fog_dir_z, height_fog_extinction, height_fog_fraction, q1_fog_color,
    Q1Fog,
};

use super::triangle_kernel::{byte, clamp, Framebuffer};

/// Eye depth from window depth. Q1 projections keep clip Z/W independent of eye X/Y.
#[must_use]
pub fn eye_depth_from_window_depth(projection: &Mat4, window_depth: f32, depth_range: [f32; 2]) -> f32 {
    let normalized = (window_depth - depth_range[0]) / (depth_range[1] - depth_range[0]) * 2.0 - 1.0;
    ((projection[14] - normalized * projection[15]) / (normalized * projection[11] - projection[10])).abs()
}

/// Q1 output path: true-color fog blends, classic indexed output skips.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q1FogOutput {
    /// Blend fog into true-color pixels.
    TrueColor,
    /// Leave classic indexed pixels alone.
    ClassicIndexed,
}

/// Run after Q1 world/entities/particles and before underwater warp and HUD.
pub fn apply_q1_depth_fog(
    frame: &mut Framebuffer,
    viewport: &Rect,
    projection: &Mat4,
    fog: &Q1Fog,
    sky_fraction: f32,
    output: Q1FogOutput,
    clear_depth: f32,
) {
    if output == Q1FogOutput::ClassicIndexed || fog.density <= 0.0 {
        return;
    }
    if projection[2] != 0.0 || projection[6] != 0.0 || projection[3] != 0.0 || projection[7] != 0.0 {
        panic!("Q1 fog requires a projection whose depth is independent of eye X/Y");
    }
    let sky = clamp(sky_fraction);
    let min_x = 0.max(viewport.x as i32);
    let max_x = (frame.width as i32).min(viewport.x as i32 + viewport.width as i32);
    let min_y = 0.max(viewport.y as i32);
    let max_y = (frame.height as i32).min(viewport.y as i32 + viewport.height as i32);
    for y in min_y..max_y {
        for x in min_x..max_x {
            let index = frame.pixel_index(x, y);
            let depth = frame.depth[index];
            let offset = index * 4;
            let color = Vec3 {
                x: f32::from(frame.pixels[offset]) / 255.0,
                y: f32::from(frame.pixels[offset + 1]) / 255.0,
                z: f32::from(frame.pixels[offset + 2]) / 255.0,
            };
            let result = if depth == clear_depth {
                Vec3 {
                    x: color.x + (fog.color.x - color.x) * sky,
                    y: color.y + (fog.color.y - color.y) * sky,
                    z: color.z + (fog.color.z - color.z) * sky,
                }
            } else {
                q1_fog_color(color, fog, eye_depth_from_window_depth(projection, depth, [0.0, 1.0]))
            };
            frame.pixels[offset] = byte(result.x);
            frame.pixels[offset + 1] = byte(result.y);
            frame.pixels[offset + 2] = byte(result.z);
        }
    }
}

/// Q2 rerelease fog: global and height passes blend separately after the
/// finished scene; sky is flat. The donor fog depth is
/// `windowDepth * eyeW`, including for translucent pixels whose depth
/// remains that of the opaque geometry behind them.
pub fn apply_q2_depth_fog(frame: &mut Framebuffer, operation: &Q2FogOperation, alpha_bits: u32) {
    let camera = &operation.camera;
    let fog = &operation.fog;
    let viewport = &camera.viewport;
    let global = fog.density > 0.0;
    let height = fog.height.density > 0.0 && fog.height.falloff > 0.0;
    let sky = operation.sky_drawn && fog.sky_factor > 0.0;
    if !global && !height && !sky {
        return;
    }
    let projection = &camera.projection;
    if projection[1] != 0.0
        || projection[2] != 0.0
        || projection[3] != 0.0
        || projection[4] != 0.0
        || projection[6] != 0.0
        || projection[7] != 0.0
        || projection[8] != 0.0
        || projection[9] != 0.0
        || projection[12] != 0.0
        || projection[13] != 0.0
        || projection[11] != -1.0
        || projection[15] != 0.0
        || projection[0] == 0.0
        || projection[5] == 0.0
    {
        panic!("Q2 fog requires the source symmetric perspective projection");
    }
    let [forward, left, up] = camera.axis;
    let origin = camera.origin;
    let a = -projection[10];
    let b = -projection[14];
    let tan_x = 1.0 / projection[0];
    let tan_y = 1.0 / projection[5];
    let density = fog.density / 64.0;
    let threshold = operation.far_depth;
    let min_x = 0.max(viewport.x as i32);
    let max_x = (frame.width as i32).min(viewport.x as i32 + viewport.width as i32);
    let min_y = 0.max(viewport.y as i32);
    let max_y = (frame.height as i32).min(viewport.y as i32 + viewport.height as i32);
    // Each pass converts its result into the byte framebuffer before the
    // next pass reads it. Alpha uses the same source-alpha blend factors as RGB.
    let blend_fog = |pixels: &mut [u8], offset: usize, red: f32, green: f32, blue: f32, amount: f32| {
        let factor = clamp(amount);
        let inverse = 1.0 - factor;
        pixels[offset] = byte(clamp(red) * factor + f32::from(pixels[offset]) / 255.0 * inverse);
        pixels[offset + 1] = byte(clamp(green) * factor + f32::from(pixels[offset + 1]) / 255.0 * inverse);
        pixels[offset + 2] = byte(clamp(blue) * factor + f32::from(pixels[offset + 2]) / 255.0 * inverse);
        pixels[offset + 3] = if alpha_bits == 0 {
            255
        } else {
            byte(factor * factor + f32::from(pixels[offset + 3]) / 255.0 * inverse)
        };
    };
    for y in min_y..max_y {
        let ndc_y = 1.0 - (y as f32 + 0.5 - viewport.y) * 2.0 / viewport.height;
        for x in min_x..max_x {
            let index = frame.pixel_index(x, y);
            let offset = index * 4;
            let depth = frame.depth[index];
            if depth >= threshold {
                if sky {
                    blend_fog(
                        &mut frame.pixels,
                        offset,
                        fog.color.x,
                        fog.color.y,
                        fog.color.z,
                        fog.sky_factor,
                    );
                }
                continue;
            }
            let eye_w = b / (a - (2.0 * depth - 1.0));
            let frag_depth = depth * eye_w;
            if global {
                blend_fog(
                    &mut frame.pixels,
                    offset,
                    fog.color.x,
                    fog.color.y,
                    fog.color.z,
                    global_fog_amount(density, frag_depth),
                );
            }
            if height {
                let ndc_x = (x as f32 + 0.5 - viewport.x) * 2.0 / viewport.width - 1.0;
                let ray_x = forward.x - left.x * ndc_x * tan_x + up.x * ndc_y * tan_y;
                let ray_y = forward.y - left.y * ndc_x * tan_x + up.y * ndc_y * tan_y;
                let ray_z = forward.z - left.z * ndc_x * tan_x + up.z * ndc_y * tan_y;
                let world_x = origin.x + ray_x * eye_w;
                let world_y = origin.y + ray_y * eye_w;
                let world_z = origin.z + ray_z * eye_w;
                let distance =
                    ((world_x - origin.x).powi(2) + (world_y - origin.y).powi(2) + (world_z - origin.z).powi(2)).sqrt();
                let direction_z = height_fog_dir_z(if distance == 0.0 {
                    0.0
                } else {
                    (world_z - origin.z) / distance
                });
                let stop = &fog.height;
                let extinction =
                    height_fog_extinction(origin.z, world_z, stop.start.distance, stop.falloff, direction_z);
                let fraction = height_fog_fraction(world_z, stop.start.distance, stop.end.distance);
                blend_fog(
                    &mut frame.pixels,
                    offset,
                    (stop.start.color.x + (stop.end.color.x - stop.start.color.x) * fraction) * extinction,
                    (stop.start.color.y + (stop.end.color.y - stop.start.color.y) * fraction) * extinction,
                    (stop.start.color.z + (stop.end.color.z - stop.start.color.z) * fraction) * extinction,
                    height_fog_amount(stop.density, frag_depth, extinction),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use qa_core::math::vec3;

    use super::super::super::types::{Q2Fog, Q2HeightFog, Q2HeightStop, RenderCamera, ViewClip};
    use super::*;

    fn symmetric_projection() -> Mat4 {
        // Symmetric perspective with a=2, b=3: depth-only terms plus focal terms.
        [
            1.0, 0.0, 0.0, 0.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, -2.0, -1.0, //
            0.0, 0.0, -3.0, 0.0,
        ]
    }

    #[test]
    fn eye_depth_inverts_window_depth() {
        // NDC z = (a*eye + b)/(eye*w sign...) — check monotonic growth instead of one constant.
        let projection = symmetric_projection();
        let near = eye_depth_from_window_depth(&projection, 0.25, [0.0, 1.0]);
        let far = eye_depth_from_window_depth(&projection, 0.75, [0.0, 1.0]);
        assert!(near > 0.0 && far > near);
    }

    #[test]
    fn q1_fog_blends_one_pixel_toward_fog_color() {
        let mut frame = Framebuffer::new(1, 1, false);
        frame.pixels = vec![0, 0, 0, 255];
        frame.depth = vec![0.5];
        let viewport = Rect {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        };
        let fog = Q1Fog {
            density: 64.0,
            color: vec3(1.0, 0.0, 0.0),
        };
        apply_q1_depth_fog(
            &mut frame,
            &viewport,
            &symmetric_projection(),
            &fog,
            0.0,
            Q1FogOutput::TrueColor,
            1.0,
        );
        // Density 64 with unit-scale depth fogs almost fully red.
        assert!(frame.pixels[0] > 200);
        assert_eq!(frame.pixels[1], 0);
        assert_eq!(frame.pixels[2], 0);
        assert_eq!(frame.pixels[3], 255);
    }

    #[test]
    fn q1_fog_skips_classic_indexed_output() {
        let mut frame = Framebuffer::new(1, 1, false);
        frame.pixels = vec![10, 20, 30, 255];
        let viewport = Rect {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        };
        let fog = Q1Fog {
            density: 64.0,
            color: vec3(1.0, 1.0, 1.0),
        };
        apply_q1_depth_fog(
            &mut frame,
            &viewport,
            &symmetric_projection(),
            &fog,
            1.0,
            Q1FogOutput::ClassicIndexed,
            1.0,
        );
        assert_eq!(frame.pixels, vec![10, 20, 30, 255]);
    }

    #[test]
    fn q2_global_fog_blends_scene_pixel() {
        let mut frame = Framebuffer::new(2, 1, false);
        frame.pixels = vec![0, 0, 0, 255, 0, 0, 0, 255];
        frame.depth = vec![0.5, 1.0];
        let operation = Q2FogOperation {
            camera: RenderCamera {
                origin: vec3(0.0, 0.0, 0.0),
                axis: [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)],
                projection: symmetric_projection(),
                viewport: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 2.0,
                    height: 1.0,
                },
                clip: ViewClip::None,
            },
            fog: Q2Fog {
                color: vec3(1.0, 0.0, 0.0),
                density: 64.0,
                sky_factor: 0.5,
                height: Q2HeightFog {
                    start: Q2HeightStop {
                        color: vec3(0.0, 0.0, 0.0),
                        distance: 0.0,
                    },
                    end: Q2HeightStop {
                        color: vec3(0.0, 0.0, 0.0),
                        distance: 100.0,
                    },
                    density: 0.0,
                    falloff: 0.0,
                },
            },
            far_depth: 1.0 - 1e-6,
            sky_drawn: true,
        };
        apply_q2_depth_fog(&mut frame, &operation, 8);
        // Scene pixel fogs toward red; sky pixel blends by the sky factor.
        assert!(frame.pixels[0] > 100);
        assert_eq!(frame.pixels[4], 128);
        assert_eq!(frame.pixels[5], 0);
    }
}
