//! View computation: refdef, viewport, FOV, camera, and kick offsets.
//!
//! Donor provenance: `src/content/q3/presentation/refdef.ts` (`Refdef`,
//! `RDF_*`, `copyRenderText`), `src/content/q3/presentation/view.ts`
//! (`ViewRuntime::{calculateViewValues,offsetThirdPerson,offsetFirstPerson,
//! calculateFov,damageBlendBlob}`) and `src/render/scene/view.ts`
//! (`perspectiveProjection`, `createViewProjector`, `cameraFrustum`,
//! `boundsInFrustum`, `farClip`, `portalClipPlane`, model transforms).
//!
//! `f32` storage follows the workspace math library; a few donor `f64`
//! transcendentals (`tan`, `sin`, `atan2`, `hypot`) are evaluated in `f32`
//! here, which only affects sub-ulp rendering inputs.

use std::f32::consts::PI;

use qa_core::math::{add3, dot3, scale3, sub3, vec3, Axis, Bounds, Mat4, Plane, Vec3, Vec4};

use crate::ClientError;

/// `RDF_NOWORLDMODEL`: hyperspace hides the world model.
pub const RDF_NOWORLDMODEL: u32 = 1;
/// `RDF_HYPERSPACE`: teleport hyperspace flag.
pub const RDF_HYPERSPACE: u32 = 4;
/// Default near clip distance (`perspectiveProjection`).
pub const DEFAULT_NEAR: f32 = 4.0;
/// Minimum/maximum view size percentages.
pub const MIN_VIEW_SIZE: i32 = 30;
/// Maximum view size percentage.
pub const MAX_VIEW_SIZE: i32 = 100;

/// Validated `refdef_t` render-text table: eight NUL-aware byte rows.
pub fn validate_render_text(rows: &[String]) -> Result<[String; 8], ClientError> {
    if rows.len() != 8 {
        return Err(ClientError::BadRenderText(
            "refdef requires eight render strings".to_string(),
        ));
    }
    for row in rows {
        if !row.chars().all(|c| (c as u32) <= 0xFF) {
            return Err(ClientError::BadRenderText(
                "refdef render strings require byte characters".to_string(),
            ));
        }
        if row.len() > 32 || (row.len() == 32 && !row.contains('\0')) {
            return Err(ClientError::BadRenderText(
                "refdef render string requires a NUL within 32 bytes".to_string(),
            ));
        }
    }
    Ok([
        rows[0].clone(),
        rows[1].clone(),
        rows[2].clone(),
        rows[3].clone(),
        rows[4].clone(),
        rows[5].clone(),
        rows[6].clone(),
        rows[7].clone(),
    ])
}

/// Renderer-independent reference definition (`refdef_t`).
#[derive(Debug, Clone, PartialEq)]
pub struct Refdef {
    /// Viewport origin.
    pub x: i32,
    /// Viewport origin.
    pub y: i32,
    /// Viewport size (even).
    pub width: i32,
    /// Viewport size (even).
    pub height: i32,
    /// Horizontal field of view in degrees.
    pub fov_x: f32,
    /// Vertical field of view in degrees.
    pub fov_y: f32,
    /// Camera origin.
    pub view_origin: Vec3,
    /// Camera basis.
    pub view_axis: Axis,
    /// Client time in milliseconds.
    pub time_ms: i32,
    /// `RDF_*` render flags.
    pub render_flags: u32,
    /// 32 area-mask bytes.
    pub area_mask: [u8; 32],
    /// Eight render strings.
    pub text: [String; 8],
}

impl Refdef {
    /// Zeroed refdef (`createRefdef`).
    #[must_use]
    pub fn new() -> Self {
        Self {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            fov_x: 0.0,
            fov_y: 0.0,
            view_origin: vec3(0.0, 0.0, 0.0),
            view_axis: [vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0)],
            time_ms: 0,
            render_flags: 0,
            area_mask: [0; 32],
            text: Default::default(),
        }
    }

    /// Publish clock and area visibility (`finishRefdef`).
    pub fn finish(&mut self, time_ms: i32, area_mask: &[u8]) -> Result<(), ClientError> {
        if area_mask.len() != 32 {
            return Err(ClientError::BadAreaMask);
        }
        self.time_ms = time_ms;
        self.area_mask.copy_from_slice(area_mask);
        Ok(())
    }
}

impl Default for Refdef {
    fn default() -> Self {
        Self::new()
    }
}

/// Computed viewport rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Viewport {
    /// Origin.
    pub x: i32,
    /// Origin.
    pub y: i32,
    /// Even-truncated width.
    pub width: i32,
    /// Even-truncated height.
    pub height: i32,
    /// Clamped view size that produced the rectangle.
    pub size: i32,
}

/// Centered even-sized viewport (`CG_CalcViewValues`).
///
/// Intermission forces size 100; otherwise `viewSize` clamps to
/// `[30, 100]`.
pub fn viewport(video_width: i32, video_height: i32, size: i32, intermission: bool) -> Result<Viewport, ClientError> {
    let size = if intermission {
        MAX_VIEW_SIZE
    } else {
        size.clamp(MIN_VIEW_SIZE, MAX_VIEW_SIZE)
    };
    let width = (video_width.wrapping_mul(size) / 100) & !1;
    let height = (video_height.wrapping_mul(size) / 100) & !1;
    if width <= 0 || height <= 0 {
        return Err(ClientError::EmptyViewport);
    }
    Ok(Viewport {
        x: (video_width - width) / 2,
        y: (video_height - height) / 2,
        width,
        height,
        size,
    })
}

/// Inputs for [`compute_fov`] (`CG_CalcFov`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FovInputs {
    /// `cg_fov` value.
    pub fov: f32,
    /// `cg_zoomFov` value.
    pub zoom_fov: f32,
    /// `dmflags` word (`DF_FIXED_FOV` is bit 4).
    pub dmflags: i32,
    /// Whether the zoom key is held.
    pub zoomed: bool,
    /// Zoom state change time in milliseconds.
    pub zoom_time_ms: i32,
    /// Current time in milliseconds.
    pub now_ms: i32,
    /// Viewport width in pixels.
    pub width: i32,
    /// Viewport height in pixels.
    pub height: i32,
    /// Intermission forces 90 degrees.
    pub intermission: bool,
    /// Underwater views wobble.
    pub in_water: bool,
}

/// Computed fields of view and zoom sensitivity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FovOutput {
    /// Horizontal field of view in degrees.
    pub fov_x: f32,
    /// Vertical field of view in degrees.
    pub fov_y: f32,
    /// Mouse sensitivity scale while zoomed.
    pub zoom_sensitivity: f32,
}

/// Horizontal/vertical FOV with zoom interpolation and water wobble.
pub fn compute_fov(inputs: &FovInputs) -> FovOutput {
    let mut fov = 90.0f32;
    if !inputs.intermission {
        fov = if inputs.dmflags & 16 != 0 {
            90.0
        } else {
            inputs.fov.clamp(1.0, 160.0)
        };
        let zoom = inputs.zoom_fov.clamp(1.0, 160.0);
        let fraction = (inputs.now_ms - inputs.zoom_time_ms) as f32 / 150.0;
        if inputs.zoomed {
            fov = if fraction > 1.0 {
                zoom
            } else {
                fov + fraction * (zoom - fov)
            };
        } else if fraction <= 1.0 {
            fov = zoom + fraction * (fov - zoom);
        }
    }
    let radians = fov / 360.0 * PI;
    let tangent = radians.sin() / radians.cos();
    let x = inputs.width as f32 / tangent;
    let mut vertical = (inputs.height as f32).atan2(x) * 360.0 / PI;
    if inputs.in_water {
        let phase = inputs.now_ms as f32 / 1000.0 * 0.4 * PI * 2.0;
        let wave = phase.sin();
        fov += wave;
        vertical -= wave;
    }
    FovOutput {
        fov_x: fov,
        fov_y: vertical,
        zoom_sensitivity: if inputs.zoomed { vertical / 75.0 } else { 1.0 },
    }
}

/// Symmetric perspective projection (`tr_main.c`).
///
/// Column-major `Mat4`; throws on non-finite FOVs outside `(0, 180)` or
/// unordered/non-positive clip distances.
pub fn perspective_projection(fov_x: f32, fov_y: f32, far: f32, near: f32) -> Result<Mat4, ClientError> {
    if ![fov_x, fov_y]
        .iter()
        .all(|value| value.is_finite() && *value > 0.0 && *value < 180.0)
        || !far.is_finite()
        || !near.is_finite()
        || near <= 0.0
        || far <= near
    {
        return Err(ClientError::BadProjection);
    }
    let width = 2.0 * (f64::from(near) * (f64::from(fov_x) * PI as f64 / 360.0).tan()) as f32;
    let height = 2.0 * (f64::from(near) * (f64::from(fov_y) * PI as f64 / 360.0).tan()) as f32;
    let depth = far - near;
    Ok([
        2.0 * near / width,
        0.0,
        0.0,
        0.0,
        0.0,
        2.0 * near / height,
        0.0,
        0.0,
        0.0,
        0.0,
        -(far + near) / depth,
        -1.0,
        0.0,
        0.0,
        -2.0 * far * near / depth,
        0.0,
    ])
}

/// Pixel rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    /// Origin.
    pub x: i32,
    /// Origin.
    pub y: i32,
    /// Size.
    pub width: i32,
    /// Size.
    pub height: i32,
}

/// Camera clipping mode.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CameraClip {
    /// No portal clipping.
    None,
    /// Portal plane clipping.
    Portal {
        /// Portal plane.
        plane: Plane,
        /// Whether the portal mirrors.
        mirror: bool,
    },
}

/// Scene camera (`SceneCamera` contract).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneCamera {
    /// Camera origin.
    pub origin: Vec3,
    /// Camera basis (forward, left, up).
    pub axis: Axis,
    /// Projection matrix.
    pub projection: Mat4,
    /// Viewport rectangle.
    pub viewport: Rect,
    /// Clipping mode.
    pub clip: CameraClip,
}

/// Rigid model transform with uniform scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelTransform {
    /// Model origin in world space.
    pub origin: Vec3,
    /// Model basis.
    pub axis: Axis,
    /// Uniform scale (nonzero finite).
    pub scale: f32,
}

/// Validate a model scale.
pub fn model_scale(model: &ModelTransform) -> Result<f32, ClientError> {
    if !model.scale.is_finite() || model.scale == 0.0 {
        return Err(ClientError::NotFinite("model scale"));
    }
    Ok(model.scale)
}

/// World direction into model space.
pub fn local_vector(vector: Vec3, model: &ModelTransform) -> Result<Vec3, ClientError> {
    let scale = model_scale(model)?;
    Ok(vec3(
        dot3(vector, model.axis[0]) / scale,
        dot3(vector, model.axis[1]) / scale,
        dot3(vector, model.axis[2]) / scale,
    ))
}

/// World point into model space.
pub fn local_point(point: Vec3, model: &ModelTransform) -> Result<Vec3, ClientError> {
    local_vector(sub3(point, model.origin), model)
}

/// Model direction into world space.
pub fn world_vector(point: Vec3, model: &ModelTransform) -> Result<Vec3, ClientError> {
    let scale = model_scale(model)?;
    Ok(add3(
        add3(
            scale3(model.axis[0], point.x * scale),
            scale3(model.axis[1], point.y * scale),
        ),
        scale3(model.axis[2], point.z * scale),
    ))
}

/// Model point into world space.
pub fn world_point(point: Vec3, model: &ModelTransform) -> Result<Vec3, ClientError> {
    Ok(add3(model.origin, world_vector(point, model)?))
}

/// Camera/model projector with precomputed eye-space rows.
///
/// Building the rows once per view/model pair replaces the per-vertex row
/// math in [`project_point`]; per-point work is then one 4x4-equivalent
/// multiply with the exact same operations, so results match bit for bit.
#[derive(Debug, Clone, Copy)]
pub struct ViewProjector {
    eye_x: Vec4,
    eye_y: Vec4,
    eye_z: Vec4,
    projection: Mat4,
    scale_valid: bool,
}

impl ViewProjector {
    /// Precompute the projection rows for one camera/model pair.
    #[must_use]
    pub fn new(camera: &SceneCamera, model: Option<&ModelTransform>) -> Self {
        let scale = match model {
            Some(model) => model.scale,
            None => 1.0,
        };
        let row = |axis: Vec3, translation: f32| -> Vec4 {
            match model {
                None => Vec4 {
                    x: axis.x,
                    y: axis.y,
                    z: axis.z,
                    w: translation,
                },
                Some(model) => Vec4 {
                    x: dot3(model.axis[0], axis) * scale,
                    y: dot3(model.axis[1], axis) * scale,
                    z: dot3(model.axis[2], axis) * scale,
                    w: dot3(model.origin, axis) + translation,
                },
            }
        };
        Self {
            eye_x: row(scale3(camera.axis[1], -1.0), dot3(camera.origin, camera.axis[1])),
            eye_y: row(camera.axis[2], -dot3(camera.origin, camera.axis[2])),
            eye_z: row(scale3(camera.axis[0], -1.0), dot3(camera.origin, camera.axis[0])),
            projection: camera.projection,
            scale_valid: model.is_none_or(|model| model.scale.is_finite() && model.scale != 0.0),
        }
    }

    /// Project one point with the precomputed rows.
    pub fn project(&self, point: Vec3) -> Result<Vec4, ClientError> {
        if !self.scale_valid {
            return Err(ClientError::NotFinite("model scale"));
        }
        let pick = |row: Vec4| vec3(row.x, row.y, row.z);
        let x = dot3(point, pick(self.eye_x)) + self.eye_x.w;
        let y = dot3(point, pick(self.eye_y)) + self.eye_y.w;
        let z = dot3(point, pick(self.eye_z)) + self.eye_z.w;
        let projection = self.projection;
        let component = |a: f32, b: f32, c: f32, d: f32| (x * a + y * b) + z * c + d;
        Ok(Vec4 {
            x: component(projection[0], projection[4], projection[8], projection[12]),
            y: component(projection[1], projection[5], projection[9], projection[13]),
            z: component(projection[2], projection[6], projection[10], projection[14]),
            w: component(projection[3], projection[7], projection[11], projection[15]),
        })
    }

    /// Project one point, mapping an invalid model scale to the zero
    /// fallback the surface builders use.
    #[must_use]
    pub fn project_or_zero(&self, point: Vec3) -> Vec4 {
        self.project(point).unwrap_or(Vec4 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            w: 1.0,
        })
    }

    /// Eye rows plus projection for retained operations. The model is
    /// already folded into the rows, so positions stay model-local.
    #[must_use]
    pub fn rows(&self) -> ([Vec4; 3], Mat4) {
        ([self.eye_x, self.eye_y, self.eye_z], self.projection)
    }

    /// Whether the model scale validated. Retained emission requires a
    /// valid scale so resolve-time projection matches `project` exactly.
    #[must_use]
    pub fn scale_valid(&self) -> bool {
        self.scale_valid
    }

    /// Rebuild a projector from retained rows. The caller guarantees the
    /// rows came from a valid projector, so the scale stays valid.
    #[must_use]
    pub fn from_rows(eye: [Vec4; 3], projection: Mat4) -> Self {
        Self {
            eye_x: eye[0],
            eye_y: eye[1],
            eye_z: eye[2],
            projection,
            scale_valid: true,
        }
    }
}

/// Compose the column-major model-view-projection matrix matching
/// [`ViewProjector::project`]: `clip = projection * eye`, with the eye
/// transform's implicit fourth row `(0, 0, 0, 1)`. The GL retained path
/// uploads this as `u_mvp`; GPU evaluation rounds independently of the CPU
/// two-step path, so backends compare against `project` with tolerance.
#[must_use]
pub fn compose_retained_mvp(eye: &[Vec4; 3], projection: &Mat4) -> Mat4 {
    let eye_row = |row: usize, col: usize| -> f32 {
        let values = [eye[row].x, eye[row].y, eye[row].z, eye[row].w];
        values[col]
    };
    let mut mvp = [0.0f32; 16];
    for column in 0..4 {
        for row in 0..4 {
            let mut sum = 0.0;
            for k in 0..4 {
                let eye_value = if k < 3 {
                    eye_row(k, column)
                } else {
                    f32::from(column == 3)
                };
                sum += projection[k * 4 + row] * eye_value;
            }
            mvp[column * 4 + row] = sum;
        }
    }
    mvp
}

/// Project one point through the camera (`createViewProjector`).
pub fn project_point(camera: &SceneCamera, model: Option<&ModelTransform>, point: Vec3) -> Result<Vec4, ClientError> {
    ViewProjector::new(camera, model).project(point)
}

/// Four-sided (plus portal) camera frustum.
pub fn camera_frustum(camera: &SceneCamera) -> Vec<Plane> {
    let side = |direction: Vec3, scale: f32, sign: f32| -> Plane {
        let inverse = (1.0 / (1.0 + f64::from(scale) * f64::from(scale)).sqrt()) as f32;
        let normal = add3(
            scale3(camera.axis[0], inverse),
            scale3(direction, scale * inverse * sign),
        );
        Plane {
            normal,
            distance: dot3(camera.origin, normal),
        }
    };
    let mut planes = vec![
        side(camera.axis[1], camera.projection[0], 1.0),
        side(camera.axis[1], camera.projection[0], -1.0),
        side(camera.axis[2], camera.projection[5], 1.0),
        side(camera.axis[2], camera.projection[5], -1.0),
    ];
    if let CameraClip::Portal { plane, .. } = camera.clip {
        planes.push(plane);
    }
    planes
}

/// Whether any part of `bounds` survives the frustum planes.
#[must_use]
pub fn bounds_in_frustum(bounds: &Bounds, planes: &[Plane]) -> bool {
    planes.iter().all(|plane| {
        let corner = vec3(
            if plane.normal.x >= 0.0 {
                bounds.max.x
            } else {
                bounds.min.x
            },
            if plane.normal.y >= 0.0 {
                bounds.max.y
            } else {
                bounds.min.y
            },
            if plane.normal.z >= 0.0 {
                bounds.max.z
            } else {
                bounds.min.z
            },
        );
        dot3(corner, plane.normal) >= plane.distance
    })
}

/// Farthest bounds corner from `origin`.
#[must_use]
pub fn far_clip(origin: Vec3, bounds: &Bounds) -> f32 {
    let mut maximum = 0.0f32;
    for x in [bounds.min.x, bounds.max.x] {
        for y in [bounds.min.y, bounds.max.y] {
            for z in [bounds.min.z, bounds.max.z] {
                let relative = sub3(vec3(x, y, z), origin);
                maximum = maximum.max(dot3(relative, relative));
            }
        }
    }
    maximum.sqrt()
}

/// Portal clip plane in clip space, or [`None`] without a portal.
#[must_use]
pub fn portal_clip_plane(camera: &SceneCamera) -> Option<Vec4> {
    let CameraClip::Portal { plane, .. } = camera.clip else {
        return None;
    };
    let projection = camera.projection;
    let a = dot3(camera.axis[0], plane.normal);
    let b = dot3(camera.axis[1], plane.normal);
    let c = dot3(camera.axis[2], plane.normal);
    let d = dot3(plane.normal, camera.origin) - plane.distance;
    Some(Vec4 {
        x: -b / projection[0],
        y: c / projection[5],
        z: d / projection[14],
        w: a + d * projection[10] / projection[14],
    })
}

/// Damage-kick blend ratio, or [`None`] when the kick expired.
///
/// Ramps up over the first 100 ms, then decays over 400 ms.
#[must_use]
pub fn damage_kick_ratio(now_ms: i32, damage_time_ms: i32) -> Option<f32> {
    if damage_time_ms == 0 {
        return None;
    }
    let elapsed = (now_ms - damage_time_ms) as f32;
    let ratio = if elapsed < 100.0 {
        elapsed / 100.0
    } else {
        1.0 - (elapsed - 100.0) / 400.0
    };
    if elapsed < 100.0 || ratio > 0.0 {
        Some(ratio)
    } else {
        None
    }
}

/// Landing dip height offset (150 ms down, 300 ms recover).
#[must_use]
pub fn land_height(now_ms: i32, land_time_ms: i32, land_change: f32) -> f32 {
    let delta = (now_ms - land_time_ms) as f32;
    if delta < 150.0 {
        land_change * (delta / 150.0)
    } else if delta < 450.0 {
        land_change * (1.0 - (delta - 150.0) / 300.0)
    } else {
        0.0
    }
}

/// Step-smoothing height offset (200 ms recover).
#[must_use]
pub fn step_height(now_ms: i32, step_time_ms: i32, step_change: f32) -> f32 {
    let delta = now_ms - step_time_ms;
    if delta < 200 {
        -(step_change * (200 - delta) as f32 / 200.0)
    } else {
        0.0
    }
}

/// Crouch-transition height offset (100 ms).
#[must_use]
pub fn duck_height(now_ms: i32, duck_time_ms: i32, duck_change: f32) -> f32 {
    let delta = now_ms - duck_time_ms;
    if delta < 100 {
        -(duck_change * (100 - delta) as f32 / 100.0)
    } else {
        0.0
    }
}

/// View-bob height (`min(6, bobFracSin * xyspeed * bobUp)`).
#[must_use]
pub fn bob_height(bob_frac_sin: f32, xyspeed: f32, bob_up: f32) -> f32 {
    (bob_frac_sin * xyspeed * bob_up).min(6.0)
}

/// Velocity-lean pitch/roll deltas (`cg_runPitch` / `cg_runRoll`).
#[must_use]
pub fn run_tilt(velocity: Vec3, forward: Vec3, right: Vec3, run_pitch: f32, run_roll: f32) -> (f32, f32) {
    (dot3(velocity, forward) * run_pitch, -(dot3(velocity, right) * run_roll))
}

/// Damage-direction blob sprite, or [`None`] when hidden.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DamageBlob {
    /// Sprite radius (`damageValue * 3`).
    pub radius: f32,
    /// Fade alpha over 500 ms.
    pub alpha: u8,
}

/// RagePro hardware and zero/expired damage hide the blob.
#[must_use]
pub fn damage_blob(damage_value: i32, elapsed_ms: i32, rage_pro: bool) -> Option<DamageBlob> {
    if damage_value == 0 || rage_pro || elapsed_ms <= 0 || elapsed_ms >= 500 {
        return None;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let alpha = (200.0 * (1.0 - elapsed_ms as f32 / 500.0)) as u8;
    Some(DamageBlob {
        radius: damage_value as f32 * 3.0,
        alpha,
    })
}

/// Apply view kick to angles and origin (`offsetFirstPerson`).
#[must_use]
pub fn apply_kick(angles: Vec3, origin: Vec3, kick_angles: Vec3, kick_origin: Vec3) -> (Vec3, Vec3) {
    (add3(angles, kick_angles), add3(origin, kick_origin))
}

/// Third-person focus pitch clamp (never above 45 degrees).
#[must_use]
pub fn third_person_focus_pitch(pitch: f32) -> f32 {
    pitch.min(45.0)
}

/// Third-person view pitch halving.
#[must_use]
pub fn third_person_halve_pitch(pitch: f32) -> f32 {
    pitch * 0.5
}

/// Third-person camera pullback along the view basis.
#[must_use]
pub fn third_person_pullback(view: Vec3, forward: Vec3, right: Vec3, range: f32, angle_degrees: f32) -> Vec3 {
    let radians = angle_degrees / 180.0 * PI;
    let view = add3(view, scale3(forward, -range * radians.cos()));
    add3(view, scale3(right, -range * radians.sin()))
}

/// Third-person aim angles from the focus point back to the camera.
#[must_use]
pub fn third_person_final_angles(focus_point: Vec3, view: Vec3, yaw: f32, roll: f32, angle_degrees: f32) -> Vec3 {
    let delta = sub3(focus_point, view);
    let distance = (delta.x * delta.x + delta.y * delta.y).sqrt().max(1.0);
    vec3(-180.0 / PI * delta.z.atan2(distance), yaw - angle_degrees, roll)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn axis() -> Axis {
        [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)]
    }

    #[test]
    fn viewport_centers_and_truncates_even() {
        let view = viewport(640, 480, 100, false).unwrap();
        assert_eq!(
            view,
            Viewport {
                x: 0,
                y: 0,
                width: 640,
                height: 480,
                size: 100
            }
        );
        let small = viewport(641, 481, 10, false).unwrap();
        assert_eq!(small.size, 30);
        assert_eq!(small.width % 2, 0);
        assert_eq!(small.height % 2, 0);
        assert!(viewport(0, 0, 100, false).is_err());
        assert_eq!(viewport(640, 480, 50, true).unwrap().size, 100);
    }

    #[test]
    fn fov_zoom_interpolates_and_water_wobbles() {
        let base = FovInputs {
            fov: 90.0,
            zoom_fov: 20.0,
            dmflags: 0,
            zoomed: true,
            zoom_time_ms: 1000,
            now_ms: 1000,
            width: 640,
            height: 480,
            intermission: false,
            in_water: false,
        };
        let start = compute_fov(&base);
        assert!((start.fov_x - 90.0).abs() < 1e-4);
        let settled = compute_fov(&FovInputs { now_ms: 1200, ..base });
        assert!((settled.fov_x - 20.0).abs() < 1e-4);
        assert!(settled.zoom_sensitivity < 1.0);
        let wet = compute_fov(&FovInputs {
            now_ms: 1200,
            in_water: true,
            ..base
        });
        assert!((wet.fov_x - settled.fov_x).abs() > 1e-6);
        let fixed = compute_fov(&FovInputs {
            dmflags: 16,
            fov: 120.0,
            zoomed: false,
            now_ms: 1200,
            ..base
        });
        assert!((fixed.fov_x - 90.0).abs() < 1e-4);
    }

    #[test]
    fn projection_centers_and_rejects_bad_clips() {
        let projection = perspective_projection(90.0, 90.0, 100.0, DEFAULT_NEAR).unwrap();
        assert!((projection[0] - 1.0).abs() < 1e-5);
        assert_eq!(projection[11], -1.0);
        assert!(perspective_projection(0.0, 90.0, 100.0, 4.0).is_err());
        assert!(perspective_projection(90.0, 90.0, 4.0, 4.0).is_err());
        let camera = SceneCamera {
            origin: vec3(0.0, 0.0, 0.0),
            axis: axis(),
            projection,
            viewport: Rect {
                x: 0,
                y: 0,
                width: 640,
                height: 480,
            },
            clip: CameraClip::None,
        };
        let center = project_point(&camera, None, vec3(10.0, 0.0, 0.0)).unwrap();
        assert!(center.x.abs() < 1e-4);
        assert!(center.y.abs() < 1e-4);
        assert!(center.w > 0.0);
        let frustum = camera_frustum(&camera);
        assert_eq!(frustum.len(), 4);
        let ahead = Bounds {
            min: vec3(9.0, -1.0, -1.0),
            max: vec3(11.0, 1.0, 1.0),
        };
        let behind = Bounds {
            min: vec3(-11.0, -1.0, -1.0),
            max: vec3(-9.0, 1.0, 1.0),
        };
        assert!(bounds_in_frustum(&ahead, &frustum));
        assert!(!bounds_in_frustum(&behind, &frustum));
        assert!((far_clip(vec3(0.0, 0.0, 0.0), &ahead) - 11.0f32.hypot(2.0f32.sqrt())).abs() < 1e-3);
        assert!(portal_clip_plane(&camera).is_none());
    }

    #[test]
    fn projector_matches_point_projection_bitwise() {
        let projection = perspective_projection(90.0, 90.0, 100.0, DEFAULT_NEAR).unwrap();
        let camera = SceneCamera {
            origin: vec3(4.0, -2.0, 8.0),
            axis: axis(),
            projection,
            viewport: Rect {
                x: 0,
                y: 0,
                width: 640,
                height: 480,
            },
            clip: CameraClip::None,
        };
        let model = ModelTransform {
            origin: vec3(1.0, 2.0, 3.0),
            axis: axis(),
            scale: 2.0,
        };
        for point in [
            vec3(0.0, 0.0, 0.0),
            vec3(10.0, -3.0, 7.0),
            vec3(-5.0, 5.0, -5.0),
            vec3(64.0, 32.0, 16.0),
        ] {
            for model in [None, Some(&model)] {
                let projector = ViewProjector::new(&camera, model);
                let expected = project_point(&camera, model, point).unwrap();
                assert_eq!(projector.project(point).unwrap(), expected);
                assert_eq!(projector.project_or_zero(point), expected);
            }
        }
        let bad = ModelTransform { scale: 0.0, ..model };
        let projector = ViewProjector::new(&camera, Some(&bad));
        assert!(projector.project(vec3(1.0, 2.0, 3.0)).is_err());
        assert!(project_point(&camera, Some(&bad), vec3(1.0, 2.0, 3.0)).is_err());
        assert_eq!(
            projector.project_or_zero(vec3(1.0, 2.0, 3.0)),
            Vec4 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                w: 1.0
            }
        );
    }

    #[test]
    fn projector_rows_round_trip_bitwise() {
        let projection = perspective_projection(90.0, 90.0, 100.0, DEFAULT_NEAR).unwrap();
        let camera = SceneCamera {
            origin: vec3(4.0, -2.0, 8.0),
            axis: axis(),
            projection,
            viewport: Rect {
                x: 0,
                y: 0,
                width: 640,
                height: 480,
            },
            clip: CameraClip::None,
        };
        let model = ModelTransform {
            origin: vec3(1.0, 2.0, 3.0),
            axis: axis(),
            scale: 2.0,
        };
        for model in [None, Some(&model)] {
            let projector = ViewProjector::new(&camera, model);
            assert!(projector.scale_valid());
            let (eye, projection) = projector.rows();
            let rebuilt = ViewProjector::from_rows(eye, projection);
            for point in [vec3(0.0, 0.0, 0.0), vec3(10.0, -3.0, 7.0), vec3(-5.0, 5.0, -5.0)] {
                assert_eq!(rebuilt.project(point).unwrap(), projector.project(point).unwrap());
            }
        }
        let bad = ModelTransform { scale: 0.0, ..model };
        assert!(!ViewProjector::new(&camera, Some(&bad)).scale_valid());
    }

    #[test]
    fn retained_mvp_matches_project_within_float_tolerance() {
        let projection = perspective_projection(90.0, 90.0, 100.0, DEFAULT_NEAR).unwrap();
        let camera = SceneCamera {
            origin: vec3(4.0, -2.0, 8.0),
            axis: axis(),
            projection,
            viewport: Rect {
                x: 0,
                y: 0,
                width: 640,
                height: 480,
            },
            clip: CameraClip::None,
        };
        let model = ModelTransform {
            origin: vec3(1.0, 2.0, 3.0),
            axis: axis(),
            scale: 2.0,
        };
        for model in [None, Some(&model)] {
            let projector = ViewProjector::new(&camera, model);
            let (eye, projection) = projector.rows();
            let mvp = compose_retained_mvp(&eye, &projection);
            for point in [
                vec3(0.0, 0.0, 0.0),
                vec3(10.0, -3.0, 7.0),
                vec3(-5.0, 5.0, -5.0),
                vec3(64.0, 32.0, 16.0),
            ] {
                let expected = projector.project(point).unwrap();
                let input = [point.x, point.y, point.z, 1.0];
                let output = [0, 1, 2, 3]
                    .map(|row| mvp[row] * input[0] + mvp[4 + row] * input[1] + mvp[8 + row] * input[2] + mvp[12 + row]);
                for (actual, expected) in output.iter().zip([expected.x, expected.y, expected.z, expected.w]) {
                    let tolerance = 1e-3 * expected.abs().max(1.0);
                    assert!(
                        (actual - expected).abs() <= tolerance,
                        "mvp {output:?} vs project {expected:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn model_transforms_round_trip() {
        let model = ModelTransform {
            origin: vec3(1.0, 2.0, 3.0),
            axis: axis(),
            scale: 2.0,
        };
        let local = local_point(vec3(3.0, 4.0, 5.0), &model).unwrap();
        assert_eq!(local, vec3(1.0, 1.0, 1.0));
        let world = world_point(local, &model).unwrap();
        assert_eq!(world, vec3(3.0, 4.0, 5.0));
        let bad = ModelTransform { scale: 0.0, ..model };
        assert!(model_scale(&bad).is_err());
    }

    #[test]
    fn first_person_offsets_match_donor_ramps() {
        assert_eq!(damage_kick_ratio(50, 0), None);
        assert_eq!(damage_kick_ratio(150, 100), Some(0.5));
        assert_eq!(damage_kick_ratio(300, 100), Some(0.75));
        assert_eq!(damage_kick_ratio(600, 100), None);
        assert!((land_height(100, 0, 8.0) - 8.0 * (100.0 / 150.0)).abs() < 1e-6);
        assert!((land_height(300, 0, 8.0) - 8.0 * 0.5).abs() < 1e-6);
        assert_eq!(land_height(500, 0, 8.0), 0.0);
        assert_eq!(step_height(100, 0, 10.0), -5.0);
        assert_eq!(step_height(300, 0, 10.0), 0.0);
        assert_eq!(duck_height(50, 0, 12.0), -6.0);
        assert_eq!(bob_height(1.0, 300.0, 0.005), 1.5);
        assert_eq!(bob_height(1.0, 4000.0, 0.002), 6.0);
        let (pitch, roll) = run_tilt(
            vec3(100.0, 0.0, 0.0),
            vec3(1.0, 0.0, 0.0),
            vec3(0.0, 1.0, 0.0),
            0.002,
            0.005,
        );
        assert!((pitch - 0.2).abs() < 1e-6);
        assert_eq!(roll, 0.0);
        let blob = damage_blob(10, 250, false).unwrap();
        assert_eq!(blob.radius, 30.0);
        assert_eq!(blob.alpha, 100);
        assert!(damage_blob(10, 500, false).is_none());
        assert!(damage_blob(0, 10, false).is_none());
        let (angles, origin) = apply_kick(
            vec3(1.0, 2.0, 3.0),
            vec3(4.0, 5.0, 6.0),
            vec3(0.5, 0.0, 0.0),
            vec3(0.0, 1.0, 0.0),
        );
        assert_eq!(angles, vec3(1.5, 2.0, 3.0));
        assert_eq!(origin, vec3(4.0, 6.0, 6.0));
    }

    #[test]
    fn third_person_math_matches_donor() {
        assert_eq!(third_person_focus_pitch(60.0), 45.0);
        assert_eq!(third_person_focus_pitch(-10.0), -10.0);
        assert_eq!(third_person_halve_pitch(20.0), 10.0);
        let pulled = third_person_pullback(
            vec3(0.0, 0.0, 8.0),
            vec3(1.0, 0.0, 0.0),
            vec3(0.0, 1.0, 0.0),
            100.0,
            0.0,
        );
        assert!((pulled.x + 100.0).abs() < 1e-4);
        let aimed = third_person_final_angles(vec3(512.0, 0.0, 0.0), vec3(0.0, 0.0, 8.0), 90.0, 0.0, 0.0);
        assert!(aimed.x > 0.0);
        assert_eq!(aimed.y, 90.0);
    }

    #[test]
    fn refdef_validates_text_and_mask() {
        let rows = vec![String::new(); 8];
        let text = validate_render_text(&rows).unwrap();
        assert_eq!(text.len(), 8);
        assert!(validate_render_text(&rows[..7]).is_err());
        assert!(validate_render_text(&vec!["é".repeat(40); 8]).is_err());
        let mut refdef = Refdef::new();
        assert!(refdef.finish(100, &[1; 31]).is_err());
        refdef.finish(100, &[1; 32]).unwrap();
        assert_eq!(refdef.time_ms, 100);
        assert_eq!(refdef.area_mask, [1; 32]);
    }
}
