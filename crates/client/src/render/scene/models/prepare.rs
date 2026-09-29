//! Scene entity preparation (donor `src/render/scene/models/prepare.ts`).
//!
//! Frame repair, pose interpolation, LOD selection, frustum culling, and
//! per-family surface assembly from Q1 `r_alias.c`/`r_sprite.c`, Q2
//! `gl_mesh.c`, and Q3 `tr_mesh.c`/`tr_surface.c`.

use qa_content::common::TimedFrames;
use qa_content::md2::{interpolate_alias_frames, map_md2_geometry, sample_timed_frame, Md2Model};
use qa_content::md3::interpolate_md3_frames;
use qa_content::md4::skin_md4_surface;
use qa_content::md5::{sample_md5_pose, skin_md5_mesh, Q1AnimationTiming, SkinSelection};
use qa_content::mdl::ModelVertex as ContentVertex;
use qa_core::math::{
    add3, add_point_to_bounds, angles_to_axis, dot3, empty_bounds, length3, radius_from_bounds, scale3, sub3, vec2,
    vec3, vector_to_angles, Bounds, Vec2, Vec3,
};
use qa_core::rng::Qrand;

use crate::materials::geometry::{MaterialGeometry, MaterialVertex};
use crate::render::error::RenderError;
use crate::render::types::{AlphaTest, CullFace};
use crate::view::{CameraClip, SceneCamera};

use super::super::particles::legacy::q2_beam_geometry;
use super::lighting::{q2_alias_light, q2_shell_color};
use super::md3_bounds::Md3EnvelopeCache;
use super::replacements::{select_model_entity, DEFAULT_MODEL_REPLACEMENT_POLICY};
use super::shadow_bounds::md5_shadow_envelope;
use super::sprites::{q1_sprite_geometry, q2_sprite_geometry};
use super::transform::{
    attach_scene_entity, model_attachment_tag, model_local_delta, model_world_bounds, model_world_direction,
    model_world_point,
};
use super::types::{
    at, byte_color, color_bytes, EntityFlags, EntityTransform, FogSphere, ModelCull, ModelDrawGroup, ModelGroupContext,
    ModelImageSelection, ModelPreparationContext, ModelSourceOptions, ModelVertexLighting, PreparePurpose,
    PreparedModelEntity, PreparedModelSurface, SceneEntity, SceneModel, ScenePose, SkinnedMeshVertex,
};

/// Repaired frame selection.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameRepair {
    /// Current frame.
    pub frame: usize,
    /// Previous frame.
    pub previous_frame: usize,
    /// Blend toward the previous frame.
    pub back_lerp: f32,
    /// A frame index fell back to zero.
    pub fallback: bool,
}

/// Repair an entity's frame selection against its model's frame count.
pub fn repair_frames(entity: &SceneEntity) -> Result<FrameRepair, RenderError> {
    let invalid = |detail: &str| RenderError::BadBatch {
        index: 0,
        detail: format!("invalid scene model pose: {detail}"),
    };
    let ScenePose::Frame {
        frame,
        previous_frame,
        back_lerp,
    } = &entity.pose
    else {
        return Ok(FrameRepair {
            frame: 0,
            previous_frame: 0,
            back_lerp: 0.0,
            fallback: false,
        });
    };
    if !back_lerp.is_finite() {
        return Err(invalid("back-lerp is not finite"));
    }
    let (mut frame, mut previous, mut back_lerp) = (*frame, *previous_frame, *back_lerp);
    if matches!(entity.flags, EntityFlags::Q2 { bits } if bits & 128 != 0) {
        return Ok(FrameRepair {
            frame: frame.max(0) as usize,
            previous_frame: previous.max(0) as usize,
            back_lerp,
            fallback: false,
        });
    }
    if let SceneModel::Md5(model) = &entity.model {
        if let SkinSelection::Q2Md2Replacement { source_frame_count, .. } = &model.skin_selection {
            let count = *source_frame_count as i32;
            let fallback = frame < 0 || frame >= count || previous < 0 || previous >= count;
            if fallback {
                frame = 0;
                previous = 0;
            }
            return Ok(FrameRepair {
                frame: frame as usize,
                previous_frame: previous as usize,
                back_lerp: if frame == previous { 0.0 } else { back_lerp },
                fallback,
            });
        }
    }
    let count = entity.model.frame_count() as i32;
    if count == 0 {
        return Err(invalid("model has no animation frames"));
    }
    let wraps = matches!(entity.model, SceneModel::Q2Sp2(_))
        || matches!(&entity.model, SceneModel::Md5(model) if !matches!(model.skin_selection, SkinSelection::Q1MdlReplacement { .. }))
        || matches!(entity.flags, EntityFlags::Q3 { bits } if bits & 512 != 0);
    if wraps {
        frame %= count;
        previous %= count;
    }
    let bad_frame = frame < 0 || frame >= count;
    let bad_old = previous < 0 || previous >= count;
    if (matches!(entity.flags, EntityFlags::Q3 { .. }) || matches!(entity.model, SceneModel::Q2Md2 { .. }))
        && (bad_frame || bad_old)
    {
        frame = 0;
        previous = 0;
    } else {
        if bad_frame {
            frame = 0;
        }
        if bad_old {
            previous = 0;
        }
    }
    if frame == previous && !matches!(entity.model, SceneModel::Q2Md2 { .. }) {
        back_lerp = 0.0;
    }
    Ok(FrameRepair {
        frame: frame as usize,
        previous_frame: previous as usize,
        back_lerp,
        fallback: bad_frame || bad_old,
    })
}

/// Interpolated model-space vertex.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneModelVertex {
    /// Position.
    pub position: Vec3,
    /// Normal.
    pub normal: Vec3,
}

impl From<ContentVertex> for SceneModelVertex {
    fn from(vertex: ContentVertex) -> Self {
        Self {
            position: vec3(vertex.position[0], vertex.position[1], vertex.position[2]),
            normal: vec3(vertex.normal[0], vertex.normal[1], vertex.normal[2]),
        }
    }
}

impl From<SceneModelVertex> for ContentVertex {
    fn from(vertex: SceneModelVertex) -> Self {
        Self {
            position: [vertex.position.x, vertex.position.y, vertex.position.z],
            normal: [vertex.normal.x, vertex.normal.y, vertex.normal.z],
        }
    }
}

/// Q2 scaled packed-coordinate interpolation after translation/origin blending.
pub fn interpolate_scene_md2(
    model: &Md2Model,
    entity: &SceneEntity,
    frame: usize,
    old_frame: usize,
    back_lerp: f32,
    shell: bool,
) -> Result<Vec<SceneModelVertex>, RenderError> {
    let current = at(&model.frames, frame, "MD2 frame")?;
    let old = at(&model.frames, old_frame, "MD2 old frame")?;
    let delta = model_local_delta(&entity.transform, sub3(entity.previous_origin, entity.transform.origin))?;
    let front = 1.0 - back_lerp;
    let blended = |d: f32, o: f32, c: f32| back_lerp * (d + o) + front * c;
    let offset = vec3(
        blended(delta.x, old.translation[0], current.translation[0]),
        blended(delta.y, old.translation[1], current.translation[1]),
        blended(delta.z, old.translation[2], current.translation[2]),
    );
    let push = if shell { 4.0 } else { 0.0 };
    current
        .compressed_vertices
        .iter()
        .enumerate()
        .map(|(index, vertex)| {
            let previous = at(&old.compressed_vertices, index, "MD2 old vertex")?;
            let normal = at(&current.vertices, index, "MD2 normal")?.normal;
            let component = |axis: usize, packed: u8, old_packed: u8| {
                offset_component(offset, axis)
                    + f32::from(old_packed) * (back_lerp * old.scale[axis])
                    + f32::from(packed) * (front * current.scale[axis])
                    + normal[axis] * push
            };
            Ok(SceneModelVertex {
                normal: vec3(normal[0], normal[1], normal[2]),
                position: vec3(
                    component(0, vertex.position[0], previous.position[0]),
                    component(1, vertex.position[1], previous.position[1]),
                    component(2, vertex.position[2], previous.position[2]),
                ),
            })
        })
        .collect()
}

fn offset_component(offset: Vec3, axis: usize) -> f32 {
    match axis {
        0 => offset.x,
        1 => offset.y,
        _ => offset.z,
    }
}

fn selected_shader(
    name: &str,
    shaders: &[String],
    entity: &SceneEntity,
    options: &ModelSourceOptions,
) -> Result<ModelImageSelection, RenderError> {
    if let Some(shader) = &options.custom_shader {
        return Ok(ModelImageSelection::External { name: shader.clone() });
    }
    if let Some(skin) = &options.custom_skin {
        let Some(selected) = skin.iter().find(|surface| surface.name == name) else {
            return Ok(ModelImageSelection::Default {
                reason: super::types::DefaultImageReason::MissingSkinSurface,
            });
        };
        return Ok(ModelImageSelection::External {
            name: selected.shader.clone(),
        });
    }
    if shaders.is_empty() {
        return Ok(ModelImageSelection::Default {
            reason: super::types::DefaultImageReason::NoSkin,
        });
    }
    let index = if matches!(entity.model, SceneModel::Q3Md3(_)) {
        if entity.skin < 0 {
            return Err(RenderError::BadBatch {
                index: 0,
                detail: "negative MD3 skin".to_string(),
            });
        }
        entity.skin as usize % shaders.len()
    } else if entity.skin >= 0 && (entity.skin as usize) < shaders.len() {
        entity.skin as usize
    } else {
        0
    };
    Ok(ModelImageSelection::External {
        name: at(shaders, index, "model skin")?.clone(),
    })
}

fn lod_index(
    entity: &SceneEntity,
    count: usize,
    radius: f32,
    camera: &SceneCamera,
    options: &ModelSourceOptions,
) -> usize {
    let mut lod = 0i32;
    if count > 1 {
        let distance = dot3(camera.axis[0], entity.transform.origin) - dot3(camera.axis[0], camera.origin);
        let matrix = camera.projection;
        let projected = if distance > 0.0 {
            ((radius * matrix[5] - distance * matrix[9] + matrix[13])
                / (radius * matrix[7] - distance * matrix[11] + matrix[15]))
                .min(1.0)
        } else {
            0.0
        };
        let mut fraction = if projected != 0.0 {
            1.0 - projected * options.lod_scale.unwrap_or(5.0).min(20.0)
        } else {
            0.0
        };
        if !fraction.is_finite() {
            fraction = 0.0;
        }
        lod = (fraction * count as f32).trunc().clamp(0.0, (count - 1) as f32) as i32;
    }
    (lod + options.lod_bias.unwrap_or(0)).clamp(0, count as i32 - 1) as usize
}

fn cull_geometry(bounds: Option<&Bounds>, context: &ModelPreparationContext) -> ModelCull {
    let (Some(frustum), Some(bounds)) = (context.frustum.as_ref(), bounds) else {
        return ModelCull::Clip;
    };
    if context.no_cull {
        return ModelCull::Clip;
    }
    let mut clipped = false;
    for plane in frustum {
        let near = vec3(
            if plane.normal.x >= 0.0 {
                bounds.min.x
            } else {
                bounds.max.x
            },
            if plane.normal.y >= 0.0 {
                bounds.min.y
            } else {
                bounds.max.y
            },
            if plane.normal.z >= 0.0 {
                bounds.min.z
            } else {
                bounds.max.z
            },
        );
        let far = vec3(
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
        if dot3(far, plane.normal) <= plane.distance {
            return ModelCull::Out;
        }
        if dot3(near, plane.normal) <= plane.distance {
            clipped = true;
        }
    }
    if clipped {
        ModelCull::Clip
    } else {
        ModelCull::In
    }
}

/// Prepare an entity, subdividing model beams into stretched segments.
pub fn prepare_scene_entity(
    entity: &SceneEntity,
    context: &ModelPreparationContext,
) -> Result<PreparedModelEntity, RenderError> {
    let Some(beam) = context.hooks.options(entity).model_beam else {
        return prepare_entity_at_transform(entity, entity, context);
    };
    let delta = sub3(entity.previous_origin, entity.transform.origin);
    let distance = length3(delta);
    let segment_length = if beam.segment_length == 0.0 {
        30.0
    } else {
        beam.segment_length
    };
    if !segment_length.is_finite() || segment_length <= 0.0 {
        return Err(RenderError::BadBatch {
            index: 0,
            detail: "invalid model beam segment length".to_string(),
        });
    }
    let direction = if distance == 0.0 {
        delta
    } else {
        scale3(delta, 1.0 / distance)
    };
    let angles = vector_to_angles(direction);
    let seed = (context.time_seconds * 1000.0).trunc() as i64 + i64::from(entity.actor_slot.unwrap_or(0));
    let mut random = Qrand::new(seed as i32, 0);
    let mut segments = Vec::new();
    let mut offset = 0.0f32;
    while offset < distance {
        let length = (distance - offset).min(segment_length);
        let count = entity.model.frame_count().max(1) as i32;
        let frame = random.next_integer().rem_euclid(count);
        let segment = SceneEntity {
            transform: EntityTransform {
                origin: add3(entity.transform.origin, scale3(direction, offset + length * 0.5)),
                axis: angles_to_axis(vec3(angles.x, angles.y, random.next_integer().rem_euclid(360) as f32)),
                scale: vec3(length / segment_length, 1.0, 1.0),
            },
            pose: ScenePose::Frame {
                frame,
                previous_frame: frame,
                back_lerp: 0.0,
            },
            flags: match entity.flags {
                EntityFlags::Q1 { .. } => EntityFlags::Q1 { bits: 8192 },
                EntityFlags::Q2 { .. } => EntityFlags::Q2 { bits: 8192 },
                EntityFlags::Q3 { .. } => EntityFlags::Q3 { bits: 8192 },
            },
            attachments: Vec::new(),
            ..entity.clone()
        };
        segments.push(prepare_entity_at_transform(&segment, entity, context)?);
        offset += segment_length;
    }
    Ok(PreparedModelEntity {
        entity: entity.clone(),
        frame: 0,
        previous_frame: 0,
        frame_fallback: false,
        lod: 0,
        bounds: None,
        cull: ModelCull::In,
        personal_model: false,
        surfaces: Vec::new(),
        attachments: segments,
        missing_attachments: Vec::new(),
        model_effect_flags: 0,
    })
}

struct SurfaceBuilder<'a, 'b> {
    surfaces: Vec<PreparedModelSurface>,
    entity: &'a SceneEntity,
    options: &'a ModelSourceOptions,
    context: &'b ModelPreparationContext<'b>,
    color: Vec4Like,
    shell: bool,
    fog_sphere: Option<FogSphere>,
    vertex_lighting: Option<ModelVertexLighting>,
    translucent: bool,
    alpha: f32,
}

#[derive(Clone, Copy)]
struct Vec4Like {
    x: f32,
    y: f32,
    z: f32,
    w: f32,
}

impl Vec4Like {
    fn from_scaled(color: qa_core::math::Vec4) -> Self {
        Self {
            x: color.x,
            y: color.y,
            z: color.z,
            w: color.w,
        }
    }

    fn bytes(self) -> [u8; 4] {
        color_bytes(qa_core::math::vec4(self.x, self.y, self.z, self.w))
    }
}

impl SurfaceBuilder<'_, '_> {
    fn ensure_lighting(&mut self) {
        if self.context.purpose == PreparePurpose::Shadow || self.vertex_lighting.is_some() {
            return;
        }
        self.vertex_lighting = self.context.hooks.prepare_vertex_lighting(self.entity, self.options);
    }

    fn local_vertex(
        &self,
        position: Vec3,
        normal: Vec3,
        tex_coord: Vec2,
        corner: usize,
        unlit: bool,
        vertex_color: Option<[u8; 4]>,
    ) -> MaterialVertex {
        let base = vertex_color.map(|color| Vec4Like {
            x: f32::from(color[0]),
            y: f32::from(color[1]),
            z: f32::from(color[2]),
            w: f32::from(color[3]),
        });
        if self.context.purpose == PreparePurpose::Shadow {
            return MaterialVertex::new(
                position,
                normal,
                tex_coord,
                vec2(0.0, 0.0),
                base.unwrap_or(self.color).bytes(),
            );
        }
        let sampled = self.context.hooks.light_vertex(self.entity, normal, position);
        let light = self
            .vertex_lighting
            .as_ref()
            .map(|light| light(normal, position, corner))
            .unwrap_or_else(|| match self.entity.flags {
                EntityFlags::Q2 { bits } => q2_alias_light(
                    bits,
                    sampled.unwrap_or(vec3(1.0, 1.0, 1.0)),
                    self.context.time_seconds,
                    false,
                    self.options.infrared,
                ),
                _ => sampled.unwrap_or(vec3(1.0, 1.0, 1.0)),
            });
        let base = base.unwrap_or(self.color);
        let lit = if unlit && !self.shell {
            base
        } else {
            Vec4Like {
                x: base.x * light.x,
                y: base.y * light.y,
                z: base.z * light.z,
                w: base.w,
            }
        };
        MaterialVertex::new(position, normal, tex_coord, vec2(0.0, 0.0), lit.bytes())
    }

    fn append(
        &mut self,
        name: &str,
        image: ModelImageSelection,
        vertices: Vec<(SceneModelVertex, Vec2, Option<[u8; 4]>)>,
        indices: Vec<u32>,
        unlit: bool,
        world: bool,
    ) {
        self.ensure_lighting();
        let local = MaterialGeometry {
            indices,
            vertices: vertices
                .into_iter()
                .enumerate()
                .map(|(corner, (vertex, tex_coord, color))| {
                    self.local_vertex(vertex.position, vertex.normal, tex_coord, corner, unlit, color)
                })
                .collect(),
        };
        self.append_geometry(name, image, local, unlit, world);
    }

    fn append_geometry(
        &mut self,
        name: &str,
        image: ModelImageSelection,
        local_geometry: MaterialGeometry,
        unlit: bool,
        world: bool,
    ) {
        let geometry = if world {
            local_geometry.clone()
        } else {
            MaterialGeometry {
                indices: local_geometry.indices.clone(),
                vertices: local_geometry
                    .vertices
                    .iter()
                    .map(|vertex| {
                        MaterialVertex::new(
                            model_world_point(&self.entity.transform, vertex.position),
                            model_world_direction(&self.entity.transform, vertex.normal),
                            vertex.tex_coord,
                            vertex.lightmap_coord,
                            vertex.color,
                        )
                    })
                    .collect(),
            }
        };
        let depth_hack = match self.entity.flags {
            EntityFlags::Q1 { .. } => self.options.view_model,
            EntityFlags::Q2 { bits } => bits & 16 != 0,
            EntityFlags::Q3 { bits } => bits & 8 != 0,
        };
        let cull = match &self.entity.model {
            SceneModel::Q1Spr(_) | SceneModel::Q2Sp2(_) => CullFace::None,
            SceneModel::Q1Mdl { .. } | SceneModel::Q2Md2 { .. } => CullFace::Front,
            SceneModel::Md5(model) => match model.skin_selection {
                SkinSelection::Q1MdlReplacement { .. } | SkinSelection::Q2Md2Replacement { .. } => CullFace::Front,
                SkinSelection::MeshShaders => CullFace::Back,
            },
            _ => CullFace::Back,
        };
        let alpha_test = match &self.entity.model {
            SceneModel::Q1Spr(_) => AlphaTest::GreaterZero,
            SceneModel::Q2Sp2(_) if self.alpha == 1.0 => AlphaTest::GreaterEqual128,
            _ => AlphaTest::None,
        };
        let index = self.surfaces.len();
        self.surfaces.push(PreparedModelSurface {
            surface_index: index,
            fog_sphere: self.fog_sphere,
            options: self.options.clone(),
            name: name.to_string(),
            entity: self.entity.clone(),
            transform: self.entity.transform,
            image,
            local_geometry,
            geometry,
            depth_range: if depth_hack { [0.0, 0.3] } else { [0.0, 1.0] },
            cull,
            alpha_test,
            translucent: self.translucent,
            unlit: unlit || self.shell,
            mirror_weapon: matches!(self.entity.flags, EntityFlags::Q2 { bits } if bits & 4 != 0)
                && self.options.left_hand == 1,
        });
    }
}

pub(crate) fn sample_frame_set(
    set: &qa_content::mdl::FrameSet,
    time_seconds: f64,
    sync_base: f64,
) -> Result<&qa_content::mdl::AliasFrame, RenderError> {
    use qa_content::mdl::FrameSet;
    match set {
        FrameSet::Single(frame) => Ok(frame),
        FrameSet::Group { frames, .. } => {
            let last = at(frames, frames.len().saturating_sub(1), "group frame")?;
            let time = time_seconds + sync_base;
            let endpoint = f64::from(last.interval_seconds);
            let target = time - (time / endpoint).trunc() * endpoint;
            for item in frames {
                if f64::from(item.interval_seconds) > target {
                    return Ok(&item.frame);
                }
            }
            Ok(&last.frame)
        }
    }
}

fn sample_timed_index<T>(
    frames: &TimedFrames<T>,
    time_seconds: f64,
    sync_base: f64,
) -> Result<(usize, &T), RenderError> {
    match frames {
        TimedFrames::Single(frame) => Ok((0, frame)),
        TimedFrames::Group(items) => {
            let last = at(items, items.len().saturating_sub(1), "group frame")?;
            let time = time_seconds + sync_base;
            let endpoint = f64::from(last.interval_seconds);
            let target = time - (time / endpoint).trunc() * endpoint;
            for (index, item) in items.iter().enumerate() {
                if f64::from(item.interval_seconds) > target {
                    return Ok((index, &item.frame));
                }
            }
            Ok((items.len() - 1, &last.frame))
        }
    }
}

fn prepare_entity_at_transform(
    entity: &SceneEntity,
    source: &SceneEntity,
    context: &ModelPreparationContext,
) -> Result<PreparedModelEntity, RenderError> {
    let options = context.hooks.options(source);
    let pose = repair_frames(entity)?;
    let native = entity.clone();
    let resolved;
    let entity = if !matches!(entity.model, SceneModel::Q1Mdl { .. }) || options.indexed_skin.is_none() {
        resolved = select_model_entity(
            entity,
            context.camera.origin,
            context
                .model_policy
                .as_ref()
                .unwrap_or(&DEFAULT_MODEL_REPLACEMENT_POLICY),
            context.purpose == PreparePurpose::Shadow,
        );
        &resolved
    } else {
        entity
    };
    let bits = entity.flags.bits();
    let shell = matches!(entity.flags, EntityFlags::Q2 { .. }) && q2_shell_color(bits).is_some();
    let portal = matches!(context.camera.clip, CameraClip::Portal { .. });
    let personal_model = matches!(entity.flags, EntityFlags::Q3 { bits } if bits & 2 != 0) && !portal;
    let invisible_weapon = matches!(entity.flags, EntityFlags::Q3 { bits } if bits & 4 != 0 && portal)
        || matches!(entity.flags, EntityFlags::Q2 { bits } if bits & 4 != 0 && options.left_hand == 2);
    let translucent =
        entity.color.w < 1.0 || matches!(entity.flags, EntityFlags::Q2 { bits } if bits & (32 | 128) != 0);
    let alpha = if translucent { entity.color.w } else { 1.0 };
    let color = Vec4Like::from_scaled(byte_color(qa_core::math::vec4(
        entity.color.x,
        entity.color.y,
        entity.color.z,
        alpha,
    )));
    let mut builder = SurfaceBuilder {
        surfaces: Vec::new(),
        entity,
        options: &options,
        context,
        color,
        shell,
        fog_sphere: None,
        vertex_lighting: None,
        translucent,
        alpha,
    };
    let mut lod = 0usize;
    let mut bounds = match &native.model {
        SceneModel::Q1Mdl { bounds, .. } => Some(model_world_bounds(&native.transform, bounds)),
        SceneModel::Q2Md2 { model, .. } if bits & 128 == 0 => {
            let mut local = empty_bounds();
            for index in [pose.frame, pose.previous_frame] {
                let frame = at(&model.frames, index, "MD2 bounds frame")?;
                local = add_point_to_bounds(
                    local,
                    vec3(frame.translation[0], frame.translation[1], frame.translation[2]),
                );
                local = add_point_to_bounds(
                    local,
                    add3(
                        vec3(frame.translation[0], frame.translation[1], frame.translation[2]),
                        scale3(vec3(frame.scale[0], frame.scale[1], frame.scale[2]), 255.0),
                    ),
                );
            }
            Some(model_world_bounds(&native.transform, &local))
        }
        _ => None,
    };
    let weapon = matches!(entity.flags, EntityFlags::Q2 { bits } if options.view_model || bits & 4 != 0);
    let source_cull = if invisible_weapon {
        Some(ModelCull::Out)
    } else if weapon {
        Some(ModelCull::In)
    } else {
        bounds.as_ref().map(|bounds| cull_geometry(Some(bounds), context))
    };
    let model = &entity.model;
    if source_cull != Some(ModelCull::Out) && matches!(entity.flags, EntityFlags::Q2 { bits } if bits & 128 != 0) {
        let palette = context
            .hooks
            .palette_color(entity, (entity.skin & 255) as u8)
            .ok_or_else(|| RenderError::Backend("Q2 beam preparation requires its source palette".to_string()))?;
        let geometry = q2_beam_geometry(
            entity.transform.origin,
            entity.previous_origin,
            pose.frame as f32,
            [
                palette.x.clamp(0.0, 255.0) as u8,
                palette.y.clamp(0.0, 255.0) as u8,
                palette.z.clamp(0.0, 255.0) as u8,
                (entity.color.w * 255.0).clamp(0.0, 255.0) as u8,
            ],
        );
        let vertices = geometry
            .vertices
            .into_iter()
            .map(|vertex| {
                (
                    SceneModelVertex {
                        position: vertex.position,
                        normal: vertex.normal,
                    },
                    vertex.tex_coord,
                    Some(vertex.color),
                )
            })
            .collect();
        builder.append(
            "beam",
            ModelImageSelection::White,
            vertices,
            geometry.indices,
            true,
            true,
        );
    } else if source_cull != Some(ModelCull::Out) {
        match model {
            SceneModel::Q1Mdl { model, .. } => {
                if !(0.0..=1.0).contains(&pose.back_lerp) {
                    return Err(RenderError::BadBatch {
                        index: 0,
                        detail: "back-lerp must be in 0..1".to_string(),
                    });
                }
                let current = sample_frame_set(
                    at(&model.frames, pose.frame, "MDL frame")?,
                    context.time_seconds,
                    options.sync_base,
                )?;
                let previous = sample_frame_set(
                    at(&model.frames, pose.previous_frame, "MDL old frame")?,
                    context.time_seconds,
                    options.sync_base,
                )?;
                let pose_vertices =
                    interpolate_alias_frames(&current.vertices, &previous.vertices, pose.back_lerp, [0.0, 0.0, 0.0]);
                let skin = if entity.skin >= 0 && (entity.skin as usize) < model.skins.len() {
                    entity.skin as usize
                } else {
                    0
                };
                let skin_frames = at(&model.skins, skin, "MDL skin")?;
                let (subframe, pixels) = sample_timed_index(skin_frames, context.time_seconds, options.sync_base)?;
                let mut vertices = Vec::new();
                let mut indices = Vec::new();
                for triangle in &model.triangles {
                    for corner in triangle.vertices {
                        let vertex = SceneModelVertex::from(*at(&pose_vertices, corner as usize, "pose vertex")?);
                        let coordinate = at(&model.texture_coordinates, corner as usize, "texture coordinate")?;
                        let s = coordinate.s as f32
                            + if !triangle.front && coordinate.on_seam {
                                model.skin_width as f32 / 2.0
                            } else {
                                0.0
                            };
                        indices.push(vertices.len() as u32);
                        vertices.push((
                            vertex,
                            vec2(
                                (s + 0.5) / model.skin_width as f32,
                                (coordinate.t as f32 + 0.5) / model.skin_height as f32,
                            ),
                            None,
                        ));
                    }
                }
                let image = match &options.indexed_skin {
                    None => ModelImageSelection::Indexed {
                        name: format!("{}:skin:{skin}:{subframe}", entity.resource.id),
                        width: model.skin_width as u32,
                        height: model.skin_height as u32,
                        pixels: pixels.clone(),
                        transparent_index: None,
                        fullbright: true,
                    },
                    Some(skin) => ModelImageSelection::Indexed {
                        name: skin.name.clone(),
                        width: skin.width,
                        height: skin.height,
                        pixels: skin.pixels.clone(),
                        transparent_index: None,
                        fullbright: true,
                    },
                };
                builder.append("alias", image, vertices, indices, false, false);
            }
            SceneModel::Q2Md2 { model, .. } => {
                let pose_vertices =
                    interpolate_scene_md2(model, entity, pose.frame, pose.previous_frame, pose.back_lerp, shell)?;
                let content_pose: Vec<ContentVertex> = pose_vertices
                    .iter()
                    .map(|vertex| ContentVertex::from(*vertex))
                    .collect();
                let image = if shell {
                    ModelImageSelection::White
                } else {
                    selected_shader("alias", &model.skins, entity, &options)?
                };
                builder.ensure_lighting();
                let mapped = map_md2_geometry(model, &content_pose, |vertex, tex_coord, corner| {
                    builder.local_vertex(
                        vec3(vertex.position[0], vertex.position[1], vertex.position[2]),
                        vec3(vertex.normal[0], vertex.normal[1], vertex.normal[2]),
                        vec2(tex_coord[0], tex_coord[1]),
                        corner,
                        shell,
                        None,
                    )
                });
                builder.append_geometry(
                    "alias",
                    image,
                    MaterialGeometry {
                        vertices: mapped.vertices,
                        indices: mapped.indices,
                    },
                    shell,
                    false,
                );
            }
            SceneModel::Q3Md3(model) => {
                let single;
                let choices: &[Option<qa_content::q3scene::SceneMd3>] = match &options.q3_lods {
                    Some(lods) => lods,
                    None => {
                        single = vec![Some(model.clone())];
                        &single
                    }
                };
                let frame_record = at(&model.frames, pose.frame, "MD3 frame")?;
                let radius = radius_from_bounds(frame_record.bounds);
                lod = lod_index(entity, choices.len().max(1), radius, &context.camera, &options);
                let selected = at(choices, lod, "MD3 LOD")?
                    .as_ref()
                    .ok_or_else(|| RenderError::BadBatch {
                        index: lod,
                        detail: format!("missing source MD3 LOD slot {lod}"),
                    })?;
                let fog_frame = at(&selected.frames, pose.frame, "MD3 fog frame")?;
                builder.fog_sphere = Some(FogSphere {
                    local_origin: fog_frame.local_origin,
                    radius: fog_frame.radius,
                });
                let mut culled = false;
                if !context.no_cull
                    && context.frustum.is_some()
                    && source_cull != Some(ModelCull::In)
                    && entity.skin >= 0
                {
                    if let Some(envelope) = Md3EnvelopeCache::new().world_envelope(
                        selected,
                        pose.frame,
                        pose.previous_frame,
                        pose.back_lerp,
                        &entity.transform,
                    ) {
                        if cull_geometry(Some(&envelope), context) == ModelCull::Out {
                            bounds = Some(envelope);
                            culled = true;
                        }
                    }
                }
                if !culled {
                    for surface in &selected.surfaces {
                        let current = at(&surface.frames, pose.frame, "MD3 frame")?;
                        let vertices = if pose.back_lerp == 0.0 {
                            current.clone()
                        } else {
                            interpolate_md3_frames(
                                current,
                                at(&surface.frames, pose.previous_frame, "MD3 old frame")?,
                                pose.back_lerp,
                            )
                        };
                        let mut mapped = Vec::with_capacity(vertices.len());
                        for (index, vertex) in vertices.iter().enumerate() {
                            mapped.push((
                                SceneModelVertex {
                                    position: vertex.position,
                                    normal: vertex.normal,
                                },
                                *at(&surface.texture_coordinates, index, "MD3 UV")?,
                                None,
                            ));
                        }
                        builder.append(
                            &surface.name,
                            selected_shader(&surface.name, &surface.shaders, entity, &options)?,
                            mapped,
                            surface.indices.clone(),
                            false,
                            false,
                        );
                    }
                }
            }
            SceneModel::Q3Md4(model) => {
                let frame_record = at(&model.frames, pose.frame, "MD4 frame")?;
                lod = lod_index(
                    entity,
                    model.lods.len().max(1),
                    frame_record.radius,
                    &context.camera,
                    &options,
                );
                let lod_surface_count = at(&model.lods, lod, "MD4 LOD")?.surfaces.len();
                for surface_index in 0..lod_surface_count {
                    let surface = at(&model.lods, lod, "MD4 LOD")?.surfaces[surface_index].clone();
                    let vertices = skin_md4_surface(
                        model,
                        &surface,
                        pose.frame as i32,
                        pose.previous_frame as i32,
                        pose.back_lerp,
                    );
                    let mut mapped = Vec::with_capacity(vertices.len());
                    for (index, vertex) in vertices.iter().enumerate() {
                        mapped.push((
                            SceneModelVertex {
                                position: vertex.position,
                                normal: vertex.normal,
                            },
                            at(&surface.vertices, index, "MD4 UV")?.tex_coords,
                            None,
                        ));
                    }
                    builder.append(
                        &surface.name,
                        selected_shader(&surface.name, std::slice::from_ref(&surface.shader), entity, &options)?,
                        mapped,
                        surface.triangles.iter().flat_map(|triangle| triangle.indices).collect(),
                        false,
                        false,
                    );
                }
            }
            SceneModel::Md5(model) => {
                let selection = &model.skin_selection;
                let elapsed = match selection {
                    SkinSelection::Q1MdlReplacement {
                        timing: Q1AnimationTiming::ElapsedTime { frame_rate },
                        ..
                    } => {
                        let step = ((context.time_seconds + options.sync_base) * f64::from(*frame_rate)) as i32;
                        Some(step.rem_euclid(model.frames.len().max(1) as i32))
                    }
                    _ => None,
                };
                let pose_frame = elapsed.unwrap_or(pose.frame as i32);
                let pose_previous = elapsed.unwrap_or(pose.previous_frame as i32);
                let pose_lerp = if elapsed.is_some() { 0.0 } else { pose.back_lerp };
                let joints = match &entity.pose {
                    ScenePose::Skeleton { joints } => joints.clone(),
                    ScenePose::Frame { .. } => {
                        let key = (
                            model.mesh_source.clone(),
                            pose_frame,
                            pose_previous,
                            pose_lerp.to_bits(),
                        );
                        let cached = context
                            .skinning_frame
                            .and_then(|frame| frame.poses.borrow().get(&key).cloned());
                        if let Some(cached) = cached {
                            cached
                        } else {
                            if model.frames.is_empty() {
                                return Err(RenderError::BadBatch {
                                    index: 0,
                                    detail: "MD5 model has no animation frames".to_string(),
                                });
                            }
                            let sampled = sample_md5_pose(&model.frames, pose_frame, pose_previous, pose_lerp);
                            if let Some(frame) = context.skinning_frame {
                                frame.poses.borrow_mut().insert(key, sampled.clone());
                            }
                            sampled
                        }
                    }
                };
                let mut images = Vec::with_capacity(model.meshes.len());
                for (index, mesh) in model.meshes.iter().enumerate() {
                    let shaders: Vec<String> = match selection {
                        SkinSelection::Q2Md2Replacement { skins, .. } => skins.clone(),
                        SkinSelection::Q1MdlReplacement { mesh_skin_groups, .. } => {
                            at(mesh_skin_groups, index, "Q1 replacement mesh skin")?
                                .iter()
                                .map(|group| {
                                    format!(
                                        "{}.lmp",
                                        sample_timed_frame(group, context.time_seconds, options.sync_base)
                                    )
                                })
                                .collect()
                        }
                        SkinSelection::MeshShaders => vec![mesh.shader.clone()],
                    };
                    images.push(if shell {
                        ModelImageSelection::White
                    } else {
                        selected_shader(&format!("mesh{index}"), &shaders, entity, &options)?
                    });
                }
                let shadow_culled = context.purpose == PreparePurpose::Shadow
                    && !shell
                    && options.model_beam.is_none()
                    && md5_shadow_envelope(&model.meshes, &joints, &entity.transform).is_some_and(|envelope| {
                        !context.hooks.retain_shadow_body(entity, &envelope, &images, &options)
                    });
                if shadow_culled {
                    // Shadow body culled; surfaces stay empty.
                } else {
                    for (index, mesh) in model.meshes.iter().enumerate() {
                        let mut identity = joints.len() as u64;
                        for joint in &joints {
                            identity = identity
                                .wrapping_mul(31)
                                .wrapping_add(joint.position.x.to_bits() as u64)
                                .wrapping_add(joint.scale.to_bits() as u64);
                        }
                        let key = (model.mesh_source.clone(), index, identity);
                        let skinned: Vec<SkinnedMeshVertex> = match context.skinning_frame {
                            Some(frame) => {
                                if let Some(cached) = frame.meshes.borrow().get(&key) {
                                    cached.clone()
                                } else {
                                    let fresh: Vec<SkinnedMeshVertex> = skin_md5_mesh(mesh, &joints)
                                        .iter()
                                        .enumerate()
                                        .map(|(vertex_index, vertex)| {
                                            at(&mesh.vertices, vertex_index, "MD5 UV").map(|source| SkinnedMeshVertex {
                                                position: vertex.position,
                                                normal: vertex.normal,
                                                tex_coord: source.tex_coord,
                                            })
                                        })
                                        .collect::<Result<Vec<_>, _>>()?;
                                    frame.meshes.borrow_mut().insert(key, fresh.clone());
                                    fresh
                                }
                            }
                            None => skin_md5_mesh(mesh, &joints)
                                .iter()
                                .enumerate()
                                .map(|(vertex_index, vertex)| {
                                    at(&mesh.vertices, vertex_index, "MD5 UV").map(|source| SkinnedMeshVertex {
                                        position: vertex.position,
                                        normal: vertex.normal,
                                        tex_coord: source.tex_coord,
                                    })
                                })
                                .collect::<Result<Vec<_>, _>>()?,
                        };
                        let mapped = skinned
                            .into_iter()
                            .map(|vertex| {
                                let position = if shell {
                                    add3(vertex.position, scale3(vertex.normal, 4.0))
                                } else {
                                    vertex.position
                                };
                                (
                                    SceneModelVertex {
                                        position,
                                        normal: vertex.normal,
                                    },
                                    vertex.tex_coord,
                                    None,
                                )
                            })
                            .collect();
                        builder.append(
                            &format!("mesh{index}"),
                            at(&images, index, "MD5 material")?.clone(),
                            mapped,
                            mesh.indices.clone(),
                            shell,
                            false,
                        );
                    }
                }
            }
            SceneModel::Q1Spr(model) => {
                let frames = at(&model.frames, pose.frame, "SPR frame")?;
                let (subframe, selected) = sample_timed_index(frames, context.time_seconds, options.sync_base)?;
                let geometry = q1_sprite_geometry(
                    model,
                    selected,
                    &entity.transform,
                    &context.camera,
                    color.bytes(),
                    options.sprite_roll,
                );
                let vertices = geometry
                    .vertices
                    .into_iter()
                    .map(|vertex| {
                        (
                            SceneModelVertex {
                                position: vertex.position,
                                normal: vertex.normal,
                            },
                            vertex.tex_coord,
                            Some(vertex.color),
                        )
                    })
                    .collect();
                builder.append(
                    "sprite",
                    ModelImageSelection::Indexed {
                        name: format!("{}:frame:{}:{subframe}", entity.resource.id, pose.frame),
                        width: selected.width as u32,
                        height: selected.height as u32,
                        pixels: selected.pixels.clone(),
                        transparent_index: Some(255),
                        fullbright: true,
                    },
                    vertices,
                    geometry.indices,
                    true,
                    true,
                );
            }
            SceneModel::Q2Sp2(model) => {
                let selected = at(&model.frames, pose.frame, "SP2 frame")?;
                let geometry = q2_sprite_geometry(selected, &entity.transform, &context.camera, color.bytes());
                let vertices = geometry
                    .vertices
                    .into_iter()
                    .map(|vertex| {
                        (
                            SceneModelVertex {
                                position: vertex.position,
                                normal: vertex.normal,
                            },
                            vertex.tex_coord,
                            Some(vertex.color),
                        )
                    })
                    .collect();
                builder.append(
                    "sprite",
                    ModelImageSelection::External {
                        name: selected.image.clone(),
                    },
                    vertices,
                    geometry.indices,
                    true,
                    true,
                );
            }
            SceneModel::BrushModel => {}
        }
    }
    if bounds.is_none() {
        let mut accumulated: Option<Bounds> = None;
        for surface in &builder.surfaces {
            for vertex in &surface.geometry.vertices {
                accumulated = Some(add_point_to_bounds(
                    accumulated.unwrap_or_else(empty_bounds),
                    vertex.position,
                ));
            }
        }
        bounds = accumulated;
    }
    let cull = source_cull.unwrap_or_else(|| cull_geometry(bounds.as_ref(), context));
    let mut attachments = Vec::new();
    let mut missing_attachments = Vec::new();
    let repaired = match &entity.pose {
        ScenePose::Frame { .. } => SceneEntity {
            pose: ScenePose::Frame {
                frame: pose.frame as i32,
                previous_frame: pose.previous_frame as i32,
                back_lerp: pose.back_lerp,
            },
            ..entity.clone()
        },
        ScenePose::Skeleton { .. } => entity.clone(),
    };
    for attachment in &entity.attachments {
        match model_attachment_tag(&repaired, &attachment.tag) {
            None => missing_attachments.push(attachment.tag.clone()),
            Some((tag, scale)) => attachments.push(prepare_entity_at_transform(
                &attach_scene_entity(&repaired, &attachment.entity, &tag, scale),
                &attachment.entity,
                context,
            )?),
        }
    }
    Ok(PreparedModelEntity {
        entity: entity.clone(),
        frame: pose.frame,
        previous_frame: pose.previous_frame,
        frame_fallback: pose.fallback,
        lod,
        bounds,
        cull,
        personal_model,
        surfaces: builder.surfaces,
        attachments,
        missing_attachments,
        model_effect_flags: match model {
            SceneModel::Q1Mdl { flags, .. } => *flags,
            SceneModel::Md5(model) => match &model.skin_selection {
                SkinSelection::Q1MdlReplacement { flags, .. } => *flags,
                _ => 0,
            },
            _ => 0,
        },
    })
}

/// Draw groups for a prepared entity: own surfaces, then attachments.
#[must_use]
pub fn prepared_model_groups(model: &PreparedModelEntity, material: &dyn ModelGroupContext) -> Vec<ModelDrawGroup> {
    let mut groups = Vec::new();
    if model.cull != ModelCull::Out && !model.personal_model {
        for surface in &model.surfaces {
            groups.extend(material.draw(surface));
        }
    }
    for attachment in &model.attachments {
        groups.extend(prepared_model_groups(attachment, material));
    }
    groups
}

/// Prepare an entity and draw it through a material sink.
pub fn prepare_scene_entity_groups(
    entity: &SceneEntity,
    context: &ModelPreparationContext,
    material: &dyn ModelGroupContext,
) -> Result<Vec<ModelDrawGroup>, RenderError> {
    Ok(prepared_model_groups(&prepare_scene_entity(entity, context)?, material))
}

/// Projected radius fraction: radius over camera distance, one at zero range.
#[must_use]
pub fn model_projected_radius(entity: &SceneEntity, radius: f32, camera_origin: Vec3) -> f32 {
    let distance = length3(sub3(entity.transform.origin, camera_origin));
    if distance == 0.0 {
        1.0
    } else {
        radius / distance
    }
}

#[cfg(test)]
mod tests {
    use super::super::types::{ModelPreparationHooks, ModelResource};
    use super::*;
    use qa_core::math::{vec3, vec4};

    struct TestHooks;

    impl ModelPreparationHooks for TestHooks {}

    fn context(camera: SceneCamera) -> ModelPreparationContext<'static> {
        // Leaked hooks live for the test duration.
        let hooks: &'static TestHooks = Box::leak(Box::new(TestHooks));
        ModelPreparationContext {
            skinning_frame: None,
            model_policy: None,
            purpose: PreparePurpose::View,
            camera,
            time_seconds: 0.0,
            frustum: None,
            no_cull: true,
            hooks,
        }
    }

    fn camera() -> SceneCamera {
        SceneCamera {
            origin: vec3(0.0, 0.0, 0.0),
            axis: [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)],
            projection: qa_core::math::identity_mat4(),
            viewport: crate::view::Rect {
                x: 0,
                y: 0,
                width: 640,
                height: 480,
            },
            clip: CameraClip::None,
        }
    }

    fn entity(model: SceneModel) -> SceneEntity {
        SceneEntity {
            resource: ModelResource {
                id: "test".to_string(),
                requested_path: "test".to_string(),
                digest: 0,
            },
            model,
            pose: ScenePose::Frame {
                frame: 0,
                previous_frame: 0,
                back_lerp: 0.0,
            },
            transform: EntityTransform::identity(),
            previous_origin: vec3(0.0, 0.0, 0.0),
            lighting_origin: vec3(0.0, 0.0, 0.0),
            color: vec4(1.0, 1.0, 1.0, 1.0),
            skin: 0,
            shader_time_seconds: 0.0,
            flags: EntityFlags::Q1 { bits: 0 },
            attachments: Vec::new(),
            actor_slot: None,
        }
    }

    #[test]
    fn skeleton_pose_repairs_to_zero() {
        let mut entity = entity(SceneModel::BrushModel);
        entity.pose = ScenePose::Skeleton { joints: Vec::new() };
        let repaired = repair_frames(&entity).expect("repair");
        assert_eq!(repaired.frame, 0);
        assert!(!repaired.fallback);
    }

    #[test]
    fn bad_frames_fall_back_to_zero() {
        let mut entity = entity(SceneModel::BrushModel);
        entity.pose = ScenePose::Frame {
            frame: 99,
            previous_frame: 0,
            back_lerp: 0.5,
        };
        let repaired = repair_frames(&entity).expect("repair");
        assert_eq!(repaired.frame, 0);
        assert_eq!(repaired.back_lerp, 0.0);
        assert!(repaired.fallback);
    }

    #[test]
    fn q2_beam_pose_passes_through() {
        let mut entity = entity(SceneModel::BrushModel);
        entity.flags = EntityFlags::Q2 { bits: 128 };
        entity.pose = ScenePose::Frame {
            frame: 12,
            previous_frame: 12,
            back_lerp: 0.0,
        };
        let repaired = repair_frames(&entity).expect("repair");
        assert_eq!(repaired.frame, 12);
        assert!(!repaired.fallback);
    }

    #[test]
    fn brush_model_prepares_no_surfaces() {
        let prepared = prepare_scene_entity(&entity(SceneModel::BrushModel), &context(camera())).expect("prepare");
        assert!(prepared.surfaces.is_empty());
        assert_eq!(prepared.cull, ModelCull::Clip);
    }

    #[test]
    fn sp2_sprite_prepares_one_quad() {
        let model = SceneModel::Q2Sp2(qa_content::spr::Sp2Model {
            frames: vec![qa_content::spr::Sp2Frame {
                width: 64,
                height: 64,
                origin_x: 32,
                origin_y: 32,
                image: "sprites/bolt.sp2".to_string(),
            }],
            bounds: qa_content::common::Bounds {
                min: [0.0; 3],
                max: [0.0; 3],
            },
        });
        let prepared = prepare_scene_entity(&entity(model), &context(camera())).expect("prepare");
        assert_eq!(prepared.surfaces.len(), 1);
        assert_eq!(prepared.surfaces[0].geometry.vertices.len(), 4);
        assert_eq!(prepared.surfaces[0].geometry.indices.len(), 6);
        assert!(prepared.bounds.is_some());
    }

    #[test]
    fn missing_attachment_is_reported() {
        let mut entity = entity(SceneModel::BrushModel);
        entity.attachments.push(super::super::types::ModelAttachment {
            tag: "tag_missing".to_string(),
            entity: Box::new(entity.clone()),
        });
        let prepared = prepare_scene_entity(&entity, &context(camera())).expect("prepare");
        assert_eq!(prepared.missing_attachments, vec!["tag_missing".to_string()]);
    }

    #[test]
    fn projected_radius_handles_zero_distance() {
        let entity = entity(SceneModel::BrushModel);
        assert_eq!(model_projected_radius(&entity, 5.0, vec3(0.0, 0.0, 0.0)), 1.0);
        assert_eq!(model_projected_radius(&entity, 4.0, vec3(2.0, 0.0, 0.0)), 2.0);
    }

    #[test]
    fn out_culled_models_draw_nothing() {
        struct Counting;
        impl ModelGroupContext for Counting {
            fn draw(&self, _surface: &PreparedModelSurface) -> Vec<ModelDrawGroup> {
                vec![ModelDrawGroup {
                    order: super::super::types::ModelGroupOrder::Opaque,
                    batches: Vec::new(),
                }]
            }
        }
        let mut prepared = prepare_scene_entity(&entity(SceneModel::BrushModel), &context(camera())).expect("prepare");
        prepared.cull = ModelCull::Out;
        assert!(prepared_model_groups(&prepared, &Counting).is_empty());
    }
}
