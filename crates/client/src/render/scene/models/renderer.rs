//! Scene model renderer (donor `src/render/scene/models/renderer.ts`).
//!
//! Material caching with preloaded textures, per-family vertex lighting, and
//! ordered batch submission for prepared model surfaces.

use std::cell::RefCell;
use std::collections::HashMap;

use qa_content::normals::ALIAS_NORMALS;
use qa_core::math::{add3, dot3, normalize3, scale3, sub3, vec3, vec4, Vec2, Vec3, Vec4};

use crate::materials::color::diffuse_color;
use crate::materials::q3_lighting::DynamicLight;
use crate::render::error::RenderError;
use crate::render::types::{
    AlphaTest, BatchLighting, BatchPrimitive, BatchVertices, BlendFactor, CullFace, DrawBatch, ImageLevel,
    Q2FragmentLight, Q2LightPass, Q2ModelFragmentLight, RenderImage, RenderState, RenderVertex, RendererImage,
    TextureBinding, TextureFilter, TextureSampling,
};
use crate::view::{project_point, SceneCamera};

use super::image_path::model_image_path;
use super::light_sampler::{ModelLightSampler, ModelLightViewInput, SamplerDynamicLight};
use super::lighting::{
    alias_shade_divisor, alias_shadow_light_fractions, q1_alias_shadow_direction, q1_alias_shadow_point,
    q2_alias_light, q2_shell_color, Q2_SHELL_MASK,
};
use super::prepare::{prepare_scene_entity, prepared_model_groups, repair_frames, sample_frame_set};
use super::replacements::{replacement_entity, ModelReplacementPolicy};
use super::shadedots::{shade_dot, shade_row_for_yaw};
use super::transform::{attach_scene_entity, model_attachment_tag, model_world_point};
use super::types::{
    at, model_image, DefaultImageReason, EntityFlags, ModelDrawGroup, ModelGroupContext, ModelGroupOrder,
    ModelImageSelection, ModelPalette, ModelPreparationContext, ModelPreparationHooks, ModelSkinningFrame,
    ModelSourceOptions, ModelVertexLighting, PreparePurpose, PreparedModelSurface, SceneEntity, SceneModel, ScenePose,
};

/// Source content family driving material selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderFamily {
    /// Quake.
    Q1,
    /// Quake II.
    Q2,
    /// Quake III.
    Q3,
}

/// Texture provider backing model materials. No asset IO occurs during
/// prepare; every surface image resolves through preloaded entries.
pub trait ModelMaterialProvider {
    /// Source content family.
    fn family(&self) -> RenderFamily;
    /// Content palette for indexed skins.
    fn palette(&self) -> Option<&ModelPalette>;
    /// Solid white texture.
    fn white_image(&self) -> RendererImage;
    /// Missing-texture fallback.
    fn missing_image(&self) -> RendererImage;
    /// Register an indexed skin image.
    fn register_indexed(
        &mut self,
        name: &str,
        image: RenderImage,
        sampling: TextureSampling,
    ) -> Result<RendererImage, RenderError>;
    /// Load an external skin image, or `None` when absent.
    fn load_external(&mut self, path: &str, sprite: bool) -> Result<Option<RendererImage>, RenderError>;
    /// Resolve the representative image for a Q3 shader name.
    fn shader_image(&mut self, name: &str) -> Result<Option<RendererImage>, RenderError>;
}

/// Cached model material.
#[derive(Debug, Clone, PartialEq)]
pub enum ModelMaterial {
    /// Q3 shader drawn in compiled order.
    Q3 {
        /// Shader name.
        name: String,
        /// Representative image.
        image: RendererImage,
    },
    /// Legacy textured material.
    Legacy {
        /// Texture.
        image: RendererImage,
        /// Fullbright index range, when the texture is indexed.
        fullbright: Option<(u8, u8)>,
    },
}

/// View inputs for one model preparation pass.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelViewInput {
    /// View camera.
    pub camera: SceneCamera,
    /// Clock in seconds.
    pub time_seconds: f64,
    /// Legacy dynamic lights.
    pub dynamic_lights: Vec<SamplerDynamicLight>,
    /// Q2 fragment lights with shadow projections.
    pub q2_lights: Vec<Q2FragmentLight>,
    /// Q2 shadow atlas image.
    pub q2_atlas: Option<RendererImage>,
    /// Q3 dynamic lights.
    pub q3_lights: Vec<DynamicLight>,
    /// Identity-light scale.
    pub identity_light: f32,
    /// Whether the world map is Q3 BSP (grid entity lighting).
    pub q3_world: bool,
    /// Whether the world map is Q2 BSP (legacy light rules).
    pub q2_world: bool,
}

/// One shadow-caster mesh in world space.
#[derive(Debug, Clone, PartialEq)]
pub struct ShadowCasterMesh {
    /// World positions.
    pub positions: Vec<Vec3>,
    /// Triangle indices.
    pub indices: Vec<u32>,
}

/// One model shadow caster.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelShadowCaster {
    /// Caster origin.
    pub origin: Vec3,
    /// World-space meshes.
    pub meshes: Vec<ShadowCasterMesh>,
}

/// Whether an entity casts shadows (view models and sprites never do).
#[must_use]
pub fn entity_casts_shadow(entity: &SceneEntity, view_model: bool) -> bool {
    if view_model || matches!(entity.model, SceneModel::Q1Spr(_) | SceneModel::Q2Sp2(_)) {
        return false;
    }
    match entity.flags {
        EntityFlags::Q2 { bits } => bits & (4 | 16 | 32 | 128 | 8192 | 0x0020_0000) == 0,
        EntityFlags::Q3 { bits } => bits & (4 | 8 | 64) == 0 && entity.color.w >= 1.0,
        EntityFlags::Q1 { .. } => entity.color.w >= 1.0,
    }
}

/// Q1 player colormap translation for shirt/shorts colors in 0..=13.
pub fn q1_player_translation(top: u8, bottom: u8) -> Result<Vec<u8>, RenderError> {
    if top > 13 || bottom > 13 {
        return Err(RenderError::BadWire("Q1 player colors must be in 0..13".to_string()));
    }
    let mut translation: Vec<u8> = (0..=255u8).collect();
    for index in 0..16u8 {
        let top_base = u16::from(top) * 16;
        let bottom_base = u16::from(bottom) * 16;
        translation[(16 + index) as usize] = if top_base < 128 {
            (top_base + u16::from(index)) as u8
        } else {
            (top_base + 15 - u16::from(index)) as u8
        };
        translation[(96 + index) as usize] = if bottom_base < 128 {
            (bottom_base + u16::from(index)) as u8
        } else {
            (bottom_base + 15 - u16::from(index)) as u8
        };
    }
    Ok(translation)
}

/// Flood the connected skin background before mipmapping (`GL_FloodFillSkin`).
#[must_use]
pub fn flood_skin(indices: &[u8], width: usize, height: usize, palette: &ModelPalette) -> Vec<u8> {
    let mut result = indices.to_vec();
    let mut black = 0u8;
    for index in 0..256usize {
        if palette.colors.get(index * 3) == Some(&0)
            && palette.colors.get(index * 3 + 1) == Some(&0)
            && palette.colors.get(index * 3 + 2) == Some(&0)
        {
            black = index as u8;
            break;
        }
    }
    let Some(&fill) = result.first() else {
        return result;
    };
    if fill == black || fill == 255 {
        return result;
    }
    result[0] = 255;
    let mut queue = vec![0usize];
    let mut head = 0;
    while head < queue.len() {
        let pixel = queue[head];
        head += 1;
        let x = pixel % width;
        let y = pixel / width;
        let mut color = black;
        let neighbors = [
            x.checked_sub(1).map(|_| pixel - 1),
            (x + 1 < width).then(|| pixel + 1),
            y.checked_sub(1).map(|_| pixel - width),
            (y + 1 < height).then(|| pixel + width),
        ];
        for next in neighbors.into_iter().flatten() {
            let value = result[next];
            if value == fill {
                result[next] = 255;
                queue.push(next);
            } else if value != 255 {
                color = value;
            }
        }
        result[pixel] = color;
    }
    result
}

fn material_key(
    resource_id: &str,
    model: &SceneModel,
    image: &ModelImageSelection,
    options: &ModelSourceOptions,
) -> String {
    let translated = matches!(image, ModelImageSelection::Indexed { .. })
        || matches!(model, SceneModel::Md5(model) if matches!(model.skin_selection, qa_content::md5::SkinSelection::Q1MdlReplacement { .. }));
    let translation = if translated {
        options
            .player_colors
            .map_or_else(String::new, |colors| format!("{}:{}", colors.top, colors.bottom))
    } else {
        String::new()
    };
    let name = match image {
        ModelImageSelection::External { name } | ModelImageSelection::Indexed { name, .. } => name.clone(),
        ModelImageSelection::White => "white".to_string(),
        ModelImageSelection::Default { reason } => match reason {
            DefaultImageReason::MissingSkinSurface => "missing-skin-surface".to_string(),
            DefaultImageReason::NoSkin => "no-skin".to_string(),
        },
    };
    let kind = match image {
        ModelImageSelection::External { .. } => "external",
        ModelImageSelection::Indexed { .. } => "indexed",
        ModelImageSelection::White => "white",
        ModelImageSelection::Default { .. } => "default",
    };
    format!("{resource_id}\0{kind}\0{name}\0{translation}")
}

fn normal_lookup() -> HashMap<(u32, u32, u32), usize> {
    ALIAS_NORMALS
        .iter()
        .enumerate()
        .map(|(index, normal)| ((normal[0].to_bits(), normal[1].to_bits(), normal[2].to_bits()), index))
        .collect()
}

/// Scene model renderer: preloaded materials plus ordered submission.
pub struct SceneModelRenderer<P> {
    provider: P,
    materials: HashMap<String, ModelMaterial>,
    lighting: ModelLightSampler,
    model_policy: Option<ModelReplacementPolicy>,
    identity_light: f32,
    q3_world: bool,
    q2_world: bool,
}

impl<P: ModelMaterialProvider> SceneModelRenderer<P> {
    /// Renderer over a texture provider and light sampler.
    pub fn new(provider: P, lighting: ModelLightSampler) -> Self {
        Self {
            provider,
            materials: HashMap::new(),
            lighting,
            model_policy: None,
            identity_light: 1.0,
            q3_world: false,
            q2_world: false,
        }
    }

    /// Replacement policy.
    pub fn set_model_policy(&mut self, policy: Option<ModelReplacementPolicy>) {
        self.model_policy = policy;
    }

    /// World-map lighting mode.
    pub fn set_world(&mut self, q3_world: bool, q2_world: bool, identity_light: f32) {
        self.q3_world = q3_world;
        self.q2_world = q2_world;
        self.identity_light = identity_light;
    }

    /// Borrow the texture provider.
    pub fn provider(&self) -> &P {
        &self.provider
    }

    /// Mutably borrow the texture provider.
    pub fn provider_mut(&mut self) -> &mut P {
        &mut self.provider
    }

    /// Borrow the light sampler.
    pub fn lighting(&self) -> &ModelLightSampler {
        &self.lighting
    }

    /// Image selections covering every surface an entity can draw.
    fn selections(
        &self,
        resource_id: &str,
        model: &SceneModel,
        options: &ModelSourceOptions,
    ) -> Vec<ModelImageSelection> {
        let mut result = vec![
            ModelImageSelection::White,
            ModelImageSelection::Default {
                reason: DefaultImageReason::MissingSkinSurface,
            },
            ModelImageSelection::Default {
                reason: DefaultImageReason::NoSkin,
            },
        ];
        if let Some(shader) = &options.custom_shader {
            result.push(ModelImageSelection::External { name: shader.clone() });
        }
        if let Some(skin) = &options.custom_skin {
            for entry in skin {
                result.push(ModelImageSelection::External {
                    name: entry.shader.clone(),
                });
            }
        }
        match model {
            SceneModel::Q1Mdl { model, .. } => {
                if let Some(skin) = &options.indexed_skin {
                    result.push(ModelImageSelection::Indexed {
                        name: skin.name.clone(),
                        width: skin.width,
                        height: skin.height,
                        pixels: skin.pixels.clone(),
                        transparent_index: None,
                        fullbright: true,
                    });
                } else {
                    for (skin, group) in model.skins.iter().enumerate() {
                        for (frame, pixels) in super::types::timed_frame_list(group).iter().enumerate() {
                            result.push(ModelImageSelection::Indexed {
                                name: format!("{resource_id}:skin:{skin}:{frame}"),
                                width: model.skin_width as u32,
                                height: model.skin_height as u32,
                                pixels: (*pixels).clone(),
                                transparent_index: None,
                                fullbright: true,
                            });
                        }
                    }
                }
            }
            SceneModel::Q1Spr(model) => {
                for (frame, group) in model.frames.iter().enumerate() {
                    for (subframe, sprite) in super::types::timed_frame_list(group).iter().enumerate() {
                        result.push(ModelImageSelection::Indexed {
                            name: format!("{resource_id}:frame:{frame}:{subframe}"),
                            width: sprite.width as u32,
                            height: sprite.height as u32,
                            pixels: sprite.pixels.clone(),
                            transparent_index: Some(255),
                            fullbright: true,
                        });
                    }
                }
            }
            SceneModel::Q2Md2 { model, .. } => {
                for skin in &model.skins {
                    result.push(ModelImageSelection::External { name: skin.clone() });
                }
            }
            SceneModel::Q2Sp2(model) => {
                for frame in &model.frames {
                    result.push(ModelImageSelection::External {
                        name: frame.image.clone(),
                    });
                }
            }
            SceneModel::Q3Md3(model) => {
                let single = vec![Some(model.clone())];
                let lods = options.q3_lods.as_ref().unwrap_or(&single);
                for lod in lods.iter().flatten() {
                    for surface in &lod.surfaces {
                        for shader in &surface.shaders {
                            result.push(ModelImageSelection::External { name: shader.clone() });
                        }
                    }
                }
            }
            SceneModel::Q3Md4(model) => {
                for lod in &model.lods {
                    for surface in &lod.surfaces {
                        result.push(ModelImageSelection::External {
                            name: surface.shader.clone(),
                        });
                    }
                }
            }
            SceneModel::Md5(model) => match &model.skin_selection {
                qa_content::md5::SkinSelection::Q1MdlReplacement { mesh_skin_groups, .. } => {
                    for mesh in mesh_skin_groups {
                        for group in mesh {
                            for name in super::types::timed_frame_list(group) {
                                result.push(ModelImageSelection::External {
                                    name: format!("{name}.lmp"),
                                });
                            }
                        }
                    }
                }
                qa_content::md5::SkinSelection::Q2Md2Replacement { skins, .. } => {
                    for name in skins {
                        result.push(ModelImageSelection::External { name: name.clone() });
                    }
                }
                qa_content::md5::SkinSelection::MeshShaders => {
                    for mesh in &model.meshes {
                        result.push(ModelImageSelection::External {
                            name: mesh.shader.clone(),
                        });
                    }
                }
            },
            SceneModel::BrushModel => {}
        }
        result
    }

    fn load(
        &mut self,
        resource_id: &str,
        model: &SceneModel,
        selection: &ModelImageSelection,
        options: &ModelSourceOptions,
    ) -> Result<(), RenderError> {
        let key = material_key(resource_id, model, selection, options);
        if self.materials.contains_key(&key) {
            return Ok(());
        }
        if self.provider.family() == RenderFamily::Q3
            && !matches!(
                selection,
                ModelImageSelection::Indexed { .. } | ModelImageSelection::White
            )
        {
            let name = match selection {
                ModelImageSelection::External { name } => name.clone(),
                ModelImageSelection::Default { .. } => "*default".to_string(),
                _ => unreachable!("indexed and white selections take the legacy path"),
            };
            let image = self
                .provider
                .shader_image(&name)?
                .unwrap_or_else(|| self.provider.missing_image());
            self.materials.insert(key, ModelMaterial::Q3 { name, image });
            return Ok(());
        }
        let material = match selection {
            ModelImageSelection::White => ModelMaterial::Legacy {
                image: self.provider.white_image(),
                fullbright: None,
            },
            ModelImageSelection::Default { .. } => ModelMaterial::Legacy {
                image: self.provider.missing_image(),
                fullbright: None,
            },
            ModelImageSelection::Indexed {
                name,
                width,
                height,
                pixels,
                transparent_index,
                fullbright,
            } => {
                let palette = self
                    .provider
                    .palette()
                    .ok_or_else(|| RenderError::Backend(format!("indexed model {name} has no content palette")))?;
                let translation = match options.player_colors {
                    None => None,
                    Some(colors) => Some(q1_player_translation(colors.top, colors.bottom)?),
                };
                let flooded = if matches!(model, SceneModel::Q1Mdl { .. }) {
                    flood_skin(pixels, *width as usize, *height as usize, palette)
                } else {
                    pixels.clone()
                };
                let cache = match options.player_colors {
                    None => name.clone(),
                    Some(colors) => format!("{name}:{}:{}", colors.top, colors.bottom),
                };
                let image = model_image(
                    name,
                    ImageLevel {
                        width: *width,
                        height: *height,
                        pixels: flooded,
                    },
                    *transparent_index,
                    *fullbright,
                    palette,
                    translation,
                );
                let handle = self.provider.register_indexed(
                    &cache,
                    image,
                    TextureSampling {
                        repeat: true,
                        filter: TextureFilter::Linear,
                    },
                )?;
                ModelMaterial::Legacy {
                    image: handle,
                    fullbright: fullbright.then(|| (224, if *transparent_index == Some(255) { 254 } else { 255 })),
                }
            }
            ModelImageSelection::External { name } => {
                let sprite = matches!(model, SceneModel::Q2Sp2(_));
                let image = self
                    .provider
                    .load_external(&model_image_path(name)?, sprite)?
                    .unwrap_or_else(|| self.provider.missing_image());
                ModelMaterial::Legacy {
                    image,
                    fullbright: None,
                }
            }
        };
        self.materials.insert(key, material);
        Ok(())
    }

    /// Preload every material an entity list can draw, including replacements
    /// and attachments. One cache per selected content provider.
    pub fn preload(
        &mut self,
        entities: &[SceneEntity],
        options: &dyn Fn(&SceneEntity) -> ModelSourceOptions,
    ) -> Result<(), RenderError> {
        fn visit<P: ModelMaterialProvider>(
            renderer: &mut SceneModelRenderer<P>,
            entity: &SceneEntity,
            options: &dyn Fn(&SceneEntity) -> ModelSourceOptions,
        ) -> Result<(), RenderError> {
            let source = options(entity);
            for selection in renderer.selections(&entity.resource.id, &entity.model, &source) {
                renderer.load(&entity.resource.id, &entity.model, &selection, &source)?;
            }
            let skip_replacement = matches!(entity.model, SceneModel::Q1Mdl { .. }) && source.indexed_skin.is_some();
            if !skip_replacement {
                if let Some(replacement) = replacement_entity(entity) {
                    for selection in renderer.selections(&replacement.resource.id, &replacement.model, &source) {
                        renderer.load(&replacement.resource.id, &replacement.model, &selection, &source)?;
                    }
                }
            }
            for attachment in &entity.attachments {
                visit(renderer, &attachment.entity, options)?;
            }
            Ok(())
        }
        for entity in entities {
            visit(self, entity, options)?;
        }
        Ok(())
    }

    fn palette_color(&self, index: u8) -> Result<Vec3, RenderError> {
        let palette = self
            .provider
            .palette()
            .ok_or_else(|| RenderError::Backend("indexed model effects require a source palette".to_string()))?;
        let base = usize::from(index) * 3;
        Ok(vec3(
            f32::from(palette.colors.get(base).copied().unwrap_or(0)),
            f32::from(palette.colors.get(base + 1).copied().unwrap_or(0)),
            f32::from(palette.colors.get(base + 2).copied().unwrap_or(0)),
        ))
    }

    fn light_input(&self, input: &ModelViewInput) -> ModelLightViewInput {
        ModelLightViewInput {
            dynamic: input.dynamic_lights.clone(),
            q3_dynamic: input.q3_lights.clone(),
            identity_light: input.identity_light,
        }
    }

    /// Base alias light shared by vertex closures and shadow fractions.
    fn base_light(
        &self,
        entity: &SceneEntity,
        options: &ModelSourceOptions,
        input: &ModelViewInput,
    ) -> Result<Vec3, RenderError> {
        let lights = self.light_input(input);
        let sampled = self.lighting.sample(entity.transform.origin, &lights, true)?.color;
        if self.provider.family() == RenderFamily::Q2 {
            let bits = match entity.flags {
                EntityFlags::Q2 { bits } => bits,
                _ => 0,
            };
            return Ok(q2_alias_light(
                bits,
                sampled,
                input.time_seconds,
                false,
                options.infrared,
            ));
        }
        if self.q2_world && !options.view_model {
            return Ok(sampled);
        }
        let static_light = if self.q2_world {
            self.lighting.sample(entity.transform.origin, &lights, false)?.color
        } else {
            sampled
        };
        let channel = |value: f32, total: f32| {
            let mut ambient = value * 255.0;
            let mut shade = ambient;
            if options.view_model && ambient < 24.0 {
                ambient = 24.0;
                shade = 24.0;
            }
            if self.q2_world {
                let amount = (total - value) * 255.0;
                ambient += amount;
                shade += amount;
            }
            if !self.q2_world {
                for dynamic in &input.dynamic_lights {
                    let difference = sub3(entity.transform.origin, dynamic.origin);
                    let amount = dynamic.radius
                        - (difference.x * difference.x + difference.y * difference.y + difference.z * difference.z)
                            .sqrt();
                    if amount > 0.0 {
                        ambient += amount;
                        shade += amount;
                    }
                }
            }
            ambient = ambient.min(128.0);
            shade = shade.min(192.0 - ambient);
            if (options.player || entity.resource.requested_path == "progs/player.mdl") && ambient < 8.0 {
                shade = 8.0;
            }
            if ["progs/flame.mdl", "progs/flame2.mdl"].contains(&entity.resource.requested_path.as_str()) {
                shade = 256.0;
            }
            shade / 200.0
                * options
                    .overbright_models
                    .map_or(2.0, |overbright| if overbright { 2.0 } else { 1.0 })
        };
        Ok(vec3(
            channel(static_light.x, sampled.x),
            channel(static_light.y, sampled.y),
            channel(static_light.z, sampled.z),
        ))
    }

    /// Full scene-model submission: prepare every entity, then draw each
    /// prepared surface through this renderer's materials.
    pub fn prepare(
        &self,
        entities: &[SceneEntity],
        input: &ModelViewInput,
        options: &dyn Fn(&SceneEntity) -> ModelSourceOptions,
        skinning_frame: Option<&ModelSkinningFrame>,
    ) -> Result<Vec<ModelDrawGroup>, RenderError> {
        let hooks = RendererHooks {
            renderer: self,
            input,
            options,
            time_seconds: input.time_seconds,
            normals: normal_lookup(),
        };
        let context = ModelPreparationContext {
            skinning_frame,
            model_policy: self.model_policy.clone(),
            purpose: PreparePurpose::View,
            camera: input.camera,
            time_seconds: input.time_seconds,
            frustum: Some(crate::view::camera_frustum(&input.camera)),
            no_cull: false,
            hooks: &hooks,
        };
        let sink = DrawSink { renderer: self, input };
        let mut groups = Vec::new();
        for entity in entities {
            groups.extend(prepared_model_groups(&prepare_scene_entity(entity, &context)?, &sink));
        }
        Ok(groups)
    }

    /// Shadow casters for a light view: no culling, shells stripped, beam and
    /// sprite entities skipped.
    pub fn prepare_shadow_casters(
        &self,
        entities: &[SceneEntity],
        input: &ModelViewInput,
        options: &dyn Fn(&SceneEntity) -> ModelSourceOptions,
        skinning_frame: Option<&ModelSkinningFrame>,
        retain_body: Option<&dyn Fn(&super::types::ShadowSphere) -> bool>,
    ) -> Result<Vec<ModelShadowCaster>, RenderError> {
        let hooks = RendererHooks {
            renderer: self,
            input,
            options,
            time_seconds: input.time_seconds,
            normals: normal_lookup(),
        };
        let mut casters = Vec::new();
        let mut visit = ShadowVisit {
            renderer: self,
            hooks: &hooks,
            input,
            skinning_frame,
            retain_body,
            casters: &mut casters,
        };
        for entity in entities {
            visit.run(entity, entity)?;
        }
        Ok(casters)
    }
}

struct ShadowVisit<'r, 'h, 'c, P> {
    renderer: &'r SceneModelRenderer<P>,
    hooks: &'h RendererHooks<'r, P>,
    input: &'r ModelViewInput,
    skinning_frame: Option<&'r ModelSkinningFrame>,
    retain_body: Option<&'r dyn Fn(&super::types::ShadowSphere) -> bool>,
    casters: &'c mut Vec<ModelShadowCaster>,
}

impl<P: ModelMaterialProvider> ShadowVisit<'_, '_, '_, P> {
    fn run(&mut self, entity: &SceneEntity, original: &SceneEntity) -> Result<(), RenderError> {
        let hooks_options = self.hooks.options;
        let source = hooks_options(original);
        if !entity_casts_shadow(entity, source.view_model) {
            return Ok(());
        }
        let flags = match entity.flags {
            EntityFlags::Q2 { bits } => EntityFlags::Q2 {
                bits: bits & !Q2_SHELL_MASK,
            },
            other => other,
        };
        let body = SceneEntity {
            attachments: Vec::new(),
            flags,
            pose: match &entity.pose {
                ScenePose::Frame {
                    frame,
                    previous_frame,
                    back_lerp,
                } => ScenePose::Frame {
                    frame: *frame,
                    previous_frame: *previous_frame,
                    back_lerp: back_lerp.clamp(0.0, 1.0),
                },
                ScenePose::Skeleton { joints } => ScenePose::Skeleton { joints: joints.clone() },
            },
            ..entity.clone()
        };
        let shadow_hooks = ShadowHooks {
            inner: self.hooks,
            retain_body: self.retain_body,
        };
        let context = ModelPreparationContext {
            skinning_frame: self.skinning_frame,
            model_policy: self.renderer.model_policy.clone(),
            purpose: PreparePurpose::Shadow,
            camera: self.input.camera,
            time_seconds: self.input.time_seconds,
            frustum: None,
            no_cull: true,
            hooks: &shadow_hooks,
        };
        let prepared = prepare_scene_entity(&body, &context)?;
        let mut meshes = Vec::new();
        for surface in &prepared.surfaces {
            let key = material_key(
                &surface.entity.resource.id,
                &surface.entity.model,
                &surface.image,
                &source,
            );
            if !self.renderer.materials.contains_key(&key) {
                return Err(RenderError::Backend(format!(
                    "shadow material was not preloaded: {}/{}",
                    entity.resource.requested_path, surface.name
                )));
            }
            meshes.push(ShadowCasterMesh {
                positions: surface.geometry.vertices.iter().map(|vertex| vertex.position).collect(),
                indices: surface.geometry.indices.clone(),
            });
        }
        if !meshes.is_empty() {
            self.casters.push(ModelShadowCaster {
                origin: entity.transform.origin,
                meshes,
            });
        }
        let parent = SceneEntity {
            pose: match (&body.pose, &prepared) {
                (ScenePose::Frame { back_lerp, .. }, prepared) => ScenePose::Frame {
                    frame: prepared.frame as i32,
                    previous_frame: prepared.previous_frame as i32,
                    back_lerp: *back_lerp,
                },
                (ScenePose::Skeleton { joints }, _) => ScenePose::Skeleton { joints: joints.clone() },
            },
            ..prepared.entity.clone()
        };
        for attachment in &entity.attachments {
            if let Some((tag, scale)) = model_attachment_tag(&parent, &attachment.tag) {
                self.run(
                    &attach_scene_entity(&parent, &attachment.entity, &tag, scale),
                    &attachment.entity,
                )?;
            }
        }
        Ok(())
    }
}

struct RendererHooks<'a, P> {
    renderer: &'a SceneModelRenderer<P>,
    input: &'a ModelViewInput,
    options: &'a dyn Fn(&SceneEntity) -> ModelSourceOptions,
    time_seconds: f64,
    normals: HashMap<(u32, u32, u32), usize>,
}

impl<P: ModelMaterialProvider> ModelPreparationHooks for RendererHooks<'_, P> {
    fn options(&self, entity: &SceneEntity) -> ModelSourceOptions {
        (self.options)(entity)
    }

    fn prepare_vertex_lighting(
        &self,
        entity: &SceneEntity,
        source: &ModelSourceOptions,
    ) -> Option<ModelVertexLighting> {
        let family = self.renderer.provider.family();
        if family == RenderFamily::Q3 {
            return Some(Box::new(|_, _, _| vec3(1.0, 1.0, 1.0)));
        }
        // Q1 old-pose normal indices per vertex, resolved eagerly so the
        // per-vertex closure owns everything it reads.
        let mut old_normals: Option<(Vec<[u32; 3]>, Vec<u8>)> = None;
        let mut back_lerp = 0.0f32;
        if family == RenderFamily::Q1 {
            if let SceneModel::Q1Mdl { model, .. } = &entity.model {
                if let ScenePose::Frame { back_lerp: lerp, .. } = &entity.pose {
                    back_lerp = *lerp;
                    if let Ok(frames) = repair_frames(entity) {
                        if let Ok(set) = at(&model.frames, frames.previous_frame, "MDL old frame") {
                            if let Ok(sampled) = sample_frame_set(set, self.time_seconds, source.sync_base) {
                                old_normals = Some((
                                    model.triangles.iter().map(|triangle| triangle.vertices).collect(),
                                    sampled
                                        .compressed_vertices
                                        .iter()
                                        .map(|vertex| vertex.normal_index)
                                        .collect(),
                                ));
                            }
                        }
                    }
                }
            }
        }
        let grid_lighting = if family == RenderFamily::Q1 && self.renderer.q3_world {
            let lights = self.renderer.light_input(self.input);
            self.renderer.lighting.entity_lighting(entity, &lights, false).ok()
        } else {
            None
        };
        let base = self.renderer.base_light(entity, source, self.input).ok()?;
        let shell = matches!(entity.flags, EntityFlags::Q2 { bits } if q2_shell_color(bits).is_some());
        let normals = self.normals.clone();
        let yaw = f64::from(entity.transform.axis[0].y).atan2(f64::from(entity.transform.axis[0].x)) as f32;
        let row = shade_row_for_yaw(yaw);
        let q2_cache: Option<RefCell<HashMap<[u32; 3], Vec3>>> =
            (family == RenderFamily::Q2).then(|| RefCell::new(HashMap::new()));
        Some(Box::new(move |normal, _position, corner| {
            let previous_normal_index = old_normals.as_ref().and_then(|(triangles, indices)| {
                triangles
                    .get(corner / 3)
                    .and_then(|triangle| triangle.get(corner % 3))
                    .and_then(|vertex| indices.get(*vertex as usize).copied())
                    .map(usize::from)
            });
            if let Some(lighting) = grid_lighting {
                let current = diffuse_color(&normal, &lighting);
                let mut color = vec3(f32::from(current[0]), f32::from(current[1]), f32::from(current[2]));
                if let Some(previous) = previous_normal_index.and_then(|index| ALIAS_NORMALS.get(index)) {
                    let old = diffuse_color(&vec3(previous[0], previous[1], previous[2]), &lighting);
                    color = add3(
                        scale3(color, 1.0 - back_lerp),
                        scale3(vec3(f32::from(old[0]), f32::from(old[1]), f32::from(old[2])), back_lerp),
                    );
                }
                return scale3(color, 1.0 / 255.0);
            }
            if let Some(cache) = &q2_cache {
                let key = [normal.x.to_bits(), normal.y.to_bits(), normal.z.to_bits()];
                if let Some(cached) = cache.borrow().get(&key) {
                    return *cached;
                }
            }
            if shell {
                return base;
            }
            let index = normals
                .get(&(normal.x.to_bits(), normal.y.to_bits(), normal.z.to_bits()))
                .copied();
            let mut shade = index.map(|index| shade_dot(row, index));
            if let Some(previous) = previous_normal_index {
                if let Some(current) = shade {
                    shade = Some(current * (1.0 - back_lerp) + shade_dot(row, previous) * back_lerp);
                }
            }
            let shade = shade.unwrap_or_else(|| {
                let direction = normalize3(vec3((-yaw).cos(), (-yaw).sin(), 1.0));
                let d = dot3(normal, direction);
                1.0 + if d < 0.0 { d * 0.3 } else { d }
            });
            let result = scale3(base, shade);
            if let Some(cache) = &q2_cache {
                cache
                    .borrow_mut()
                    .insert([normal.x.to_bits(), normal.y.to_bits(), normal.z.to_bits()], result);
            }
            result
        }))
    }

    fn palette_color(&self, _entity: &SceneEntity, index: u8) -> Option<Vec3> {
        self.renderer.palette_color(index).ok()
    }
}

struct ShadowHooks<'a, 'b, P> {
    inner: &'a RendererHooks<'a, P>,
    retain_body: Option<&'b dyn Fn(&super::types::ShadowSphere) -> bool>,
}

impl<P: ModelMaterialProvider> ModelPreparationHooks for ShadowHooks<'_, '_, P> {
    fn options(&self, entity: &SceneEntity) -> ModelSourceOptions {
        self.inner.options(entity)
    }

    fn retain_shadow_body(
        &self,
        _entity: &SceneEntity,
        sphere: &super::types::ShadowSphere,
        _images: &[ModelImageSelection],
        _options: &ModelSourceOptions,
    ) -> bool {
        self.retain_body.is_none_or(|retain| retain(sphere))
    }
}

struct DrawSink<'a, P> {
    renderer: &'a SceneModelRenderer<P>,
    input: &'a ModelViewInput,
}

impl<P: ModelMaterialProvider> DrawSink<'_, P> {
    fn project(&self, surface: &PreparedModelSurface, point: Vec3) -> Result<Vec4, RenderError> {
        project_point(&self.input.camera, None, point)
            .map(|mut projected| {
                if surface.mirror_weapon {
                    projected.x = -projected.x;
                }
                projected
            })
            .map_err(|error| RenderError::Backend(error.to_string()))
    }

    fn packed_vertices(
        &self,
        surface: &PreparedModelSurface,
        shade_scale: f32,
    ) -> Result<Vec<RenderVertex>, RenderError> {
        surface
            .geometry
            .vertices
            .iter()
            .map(|vertex| {
                Ok(RenderVertex {
                    position: self.project(surface, vertex.position)?,
                    tex_coord: vertex.tex_coord,
                    color: vec4(
                        f32::from(vertex.color[0]) / 255.0 / shade_scale,
                        f32::from(vertex.color[1]) / 255.0 / shade_scale,
                        f32::from(vertex.color[2]) / 255.0 / shade_scale,
                        f32::from(vertex.color[3]) / 255.0,
                    ),
                })
            })
            .collect()
    }
}

impl<P: ModelMaterialProvider> ModelGroupContext for DrawSink<'_, P> {
    fn draw(&self, surface: &PreparedModelSurface) -> Vec<ModelDrawGroup> {
        self.draw_inner(surface).unwrap_or_default()
    }
}

impl<P: ModelMaterialProvider> DrawSink<'_, P> {
    fn draw_inner(&self, surface: &PreparedModelSurface) -> Result<Vec<ModelDrawGroup>, RenderError> {
        let key = material_key(
            &surface.entity.resource.id,
            &surface.entity.model,
            &surface.image,
            &surface.options,
        );
        let material = self.renderer.materials.get(&key).ok_or_else(|| {
            RenderError::Backend(format!(
                "model material was not preloaded: {}/{}",
                surface.entity.resource.requested_path, surface.name
            ))
        })?;
        let options = &surface.options;
        let flags = match surface.entity.flags {
            EntityFlags::Q2 { bits } => bits,
            _ => 0,
        };
        let cone_lights: Vec<Q2FragmentLight> = self
            .input
            .q2_lights
            .iter()
            .filter(|light| light.cone.is_some())
            .cloned()
            .collect();
        let has_cones = !cone_lights.is_empty();
        let receives_cone = has_cones
            && self.input.q2_atlas.is_some()
            && !surface.unlit
            && !options.view_model
            && flags & (Q2_SHELL_MASK | 8 | 4 | 16) == 0
            && !(options.infrared && flags & 32768 != 0);
        if let ModelMaterial::Q3 { image, .. } = material {
            let state = RenderState {
                blend: (BlendFactor::One, BlendFactor::Zero),
                depth_test: crate::render::types::DepthTest::LessEqual,
                depth_write: true,
                alpha_test: surface.alpha_test,
                cull: surface.cull,
                depth_range: surface.depth_range,
                polygon_offset: None,
            };
            let batch = DrawBatch {
                fog: None,
                luminance_alpha: false,
                indices: surface.geometry.indices.clone(),
                texture: TextureBinding::BindImage(image.clone()),
                state,
                lighting: BatchLighting::Vertex,
                primitive: BatchPrimitive::Triangles,
                vertices: BatchVertices::Single(self.packed_vertices(surface, 1.0)?),
            };
            return Ok(vec![ModelDrawGroup {
                order: ModelGroupOrder::Compiled,
                batches: vec![batch],
            }]);
        }
        let ModelMaterial::Legacy { image, .. } = material else {
            return Err(RenderError::Backend("unreachable material kind".to_string()));
        };
        let alpha = if surface.translucent {
            surface.entity.color.w
        } else {
            1.0
        };
        let mut state = RenderState::opaque(surface.cull);
        state.alpha_test = if surface.alpha_test == AlphaTest::None {
            state.alpha_test
        } else {
            surface.alpha_test
        };
        if surface.mirror_weapon {
            state.cull = match state.cull {
                CullFace::Front => CullFace::Back,
                CullFace::Back => CullFace::Front,
                CullFace::None => CullFace::None,
            };
        }
        if surface.translucent {
            state.blend = (BlendFactor::SrcAlpha, BlendFactor::OneMinusSrcAlpha);
        }
        state.depth_range = surface.depth_range;
        let shade = self.renderer.base_light(&surface.entity, options, self.input).ok();
        let receives = self.renderer.provider.family() == RenderFamily::Q2
            && shade.is_some()
            && self.input.q2_atlas.is_some()
            && !surface.unlit
            && !options.view_model
            && flags & (Q2_SHELL_MASK | 8 | 4 | 16) == 0
            && !(options.infrared && flags & 32768 != 0);
        let affecting = if receives {
            let shade = shade.unwrap_or(vec3(1.0, 1.0, 1.0));
            let lights: Vec<Q2FragmentLight> = if has_cones {
                self.input
                    .q2_lights
                    .iter()
                    .filter(|light| light.cone.is_none())
                    .cloned()
                    .collect()
            } else {
                self.input.q2_lights.clone()
            };
            alias_shadow_light_fractions(surface.entity.transform.origin, shade, &lights, 1.0, false)
        } else {
            Vec::new()
        };
        let shade_scale = if receives && !affecting.is_empty() {
            alias_shade_divisor(shade.unwrap_or(vec3(1.0, 1.0, 1.0)))
        } else {
            1.0
        };
        let lighting = if affecting.is_empty() {
            BatchLighting::Vertex
        } else if let Some(atlas) = &self.input.q2_atlas {
            BatchLighting::Q2ModelShadow {
                world_positions: surface.geometry.vertices.iter().map(|vertex| vertex.position).collect(),
                lights: affecting.clone(),
                shade_scale,
                atlas: crate::render::types::Q2ShadowAtlas {
                    image: atlas.clone(),
                    texel_size: 0.0,
                    near_plane: 0.0,
                },
            }
        } else {
            BatchLighting::Vertex
        };
        let lighting = if receives_cone {
            let atlas = self.input.q2_atlas.clone().unwrap_or_else(|| image.clone());
            BatchLighting::Q2World {
                world_positions: surface.geometry.vertices.iter().map(|vertex| vertex.position).collect(),
                normals: surface
                    .geometry
                    .vertices
                    .iter()
                    .map(|vertex| normalize3(vertex.normal))
                    .collect(),
                atlas: Some(crate::render::types::Q2ShadowAtlas {
                    image: atlas,
                    texel_size: 0.0,
                    near_plane: 0.0,
                }),
                pass: Q2LightPass::Model {
                    lights: self
                        .input
                        .q2_lights
                        .iter()
                        .map(|light| {
                            let fraction = affecting
                                .iter()
                                .find(|point| point.origin == light.origin)
                                .map_or(vec3(0.0, 0.0, 0.0), |point| point.fraction);
                            if light.cone.is_none() {
                                Q2ModelFragmentLight {
                                    light: Q2FragmentLight {
                                        color: vec3(0.0, 0.0, 0.0),
                                        scale: 0.0,
                                        ..*light
                                    },
                                    fraction,
                                }
                            } else {
                                Q2ModelFragmentLight {
                                    light: *light,
                                    fraction,
                                }
                            }
                        })
                        .collect(),
                    shade_scale: matches!(lighting, BatchLighting::Q2ModelShadow { .. }).then_some(shade_scale),
                },
            }
        } else {
            lighting
        };
        let mut batches = vec![DrawBatch {
            fog: None,
            luminance_alpha: false,
            indices: surface.geometry.indices.clone(),
            texture: TextureBinding::BindImage(image.clone()),
            state,
            lighting,
            primitive: BatchPrimitive::Triangles,
            vertices: BatchVertices::Single(self.packed_vertices(surface, shade_scale)?),
        }];
        if options.planar_shadow
            && !options.view_model
            && self.renderer.provider.family() == RenderFamily::Q1
            && matches!(surface.entity.model, SceneModel::Q1Mdl { .. } | SceneModel::Md5(_))
        {
            let lights = self.renderer.light_input(self.input);
            if let Ok(sample) = self.renderer.lighting.sample(surface.transform.origin, &lights, false) {
                if let Some(floor) = sample.floor {
                    let yaw =
                        f64::from(surface.transform.axis[0].y).atan2(f64::from(surface.transform.axis[0].x)) as f32;
                    let direction = q1_alias_shadow_direction(yaw);
                    let shadow_transform = super::types::EntityTransform {
                        scale: vec3(1.0, 1.0, 1.0),
                        ..surface.transform
                    };
                    let mut vertices = Vec::new();
                    for vertex in &surface.local_geometry.vertices {
                        let scaled = vec3(
                            vertex.position.x * surface.transform.scale.x,
                            vertex.position.y * surface.transform.scale.y,
                            vertex.position.z * surface.transform.scale.z,
                        );
                        vertices.push(RenderVertex {
                            position: self.project(
                                surface,
                                model_world_point(
                                    &shadow_transform,
                                    q1_alias_shadow_point(scaled, direction, surface.transform.origin.z, floor.point.z),
                                ),
                            )?,
                            tex_coord: Vec2 { x: 0.0, y: 0.0 },
                            color: vec4(0.0, 0.0, 0.0, 0.5 * alpha),
                        });
                    }
                    batches.push(DrawBatch {
                        fog: None,
                        luminance_alpha: false,
                        indices: surface.local_geometry.indices.clone(),
                        texture: TextureBinding::BindImage(self.renderer.provider.white_image()),
                        state: RenderState {
                            blend: (BlendFactor::SrcAlpha, BlendFactor::OneMinusSrcAlpha),
                            depth_test: crate::render::types::DepthTest::LessEqual,
                            depth_write: true,
                            alpha_test: AlphaTest::None,
                            cull: surface.cull,
                            depth_range: surface.depth_range,
                            polygon_offset: None,
                        },
                        lighting: BatchLighting::Vertex,
                        primitive: BatchPrimitive::Triangles,
                        vertices: BatchVertices::Single(vertices),
                    });
                }
            }
        }
        Ok(vec![ModelDrawGroup {
            order: if alpha < 1.0 {
                ModelGroupOrder::Translucent
            } else {
                ModelGroupOrder::Opaque
            },
            batches,
        }])
    }
}

#[cfg(test)]
mod tests {
    use super::super::types::{EntityTransform, ModelResource};
    use super::*;
    use crate::render::types::{ImageSource, ResourceOwner};
    use qa_core::math::{vec3, vec4};

    struct TestProvider {
        family: RenderFamily,
        palette: Option<ModelPalette>,
        next_ordinal: u32,
        owner: ResourceOwner,
    }

    impl TestProvider {
        fn handle(&mut self) -> RendererImage {
            let ordinal = self.next_ordinal;
            self.next_ordinal += 1;
            RendererImage {
                owner: self.owner.clone(),
                ordinal,
                source: ImageSource::Generated {
                    name: format!("test{ordinal}"),
                },
                width: 8,
                height: 8,
            }
        }
    }

    impl ModelMaterialProvider for TestProvider {
        fn family(&self) -> RenderFamily {
            self.family
        }

        fn palette(&self) -> Option<&ModelPalette> {
            self.palette.as_ref()
        }

        fn white_image(&self) -> RendererImage {
            RendererImage {
                owner: self.owner.clone(),
                ordinal: 1000,
                source: ImageSource::Generated {
                    name: "white".to_string(),
                },
                width: 1,
                height: 1,
            }
        }

        fn missing_image(&self) -> RendererImage {
            RendererImage {
                owner: self.owner.clone(),
                ordinal: 1001,
                source: ImageSource::Generated {
                    name: "missing".to_string(),
                },
                width: 1,
                height: 1,
            }
        }

        fn register_indexed(
            &mut self,
            _name: &str,
            _image: RenderImage,
            _sampling: TextureSampling,
        ) -> Result<RendererImage, RenderError> {
            Ok(self.handle())
        }

        fn load_external(&mut self, _path: &str, _sprite: bool) -> Result<Option<RendererImage>, RenderError> {
            Ok(Some(self.handle()))
        }

        fn shader_image(&mut self, _name: &str) -> Result<Option<RendererImage>, RenderError> {
            Ok(Some(self.handle()))
        }
    }

    fn provider(family: RenderFamily) -> TestProvider {
        let authority = qa_core::identity::IdentityOwner::create("test").expect("owner");
        TestProvider {
            family,
            palette: Some(ModelPalette {
                colors: vec![0; 768],
                source: "palette".to_string(),
            }),
            next_ordinal: 1,
            owner: ResourceOwner::new(1, authority.session().clone(), 0),
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
            clip: crate::view::CameraClip::None,
        }
    }

    fn input(camera: SceneCamera) -> ModelViewInput {
        ModelViewInput {
            camera,
            time_seconds: 0.0,
            dynamic_lights: Vec::new(),
            q2_lights: Vec::new(),
            q2_atlas: None,
            q3_lights: Vec::new(),
            identity_light: 1.0,
            q3_world: false,
            q2_world: false,
        }
    }

    fn sprite_entity() -> SceneEntity {
        SceneEntity {
            resource: ModelResource {
                id: "sprite".to_string(),
                requested_path: "sprite".to_string(),
                digest: 0,
            },
            model: SceneModel::Q2Sp2(qa_content::spr::Sp2Model {
                frames: vec![qa_content::spr::Sp2Frame {
                    width: 64,
                    height: 64,
                    origin_x: 32,
                    origin_y: 32,
                    image: "sprites/bolt".to_string(),
                }],
                bounds: qa_content::common::Bounds {
                    min: [0.0; 3],
                    max: [0.0; 3],
                },
            }),
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
            flags: EntityFlags::Q2 { bits: 0 },
            attachments: Vec::new(),
            actor_slot: None,
        }
    }

    #[test]
    fn preload_then_prepare_emits_batches() {
        let mut renderer = SceneModelRenderer::new(provider(RenderFamily::Q2), ModelLightSampler::fullbright());
        let entities = vec![sprite_entity()];
        renderer
            .preload(&entities, &|_| ModelSourceOptions::default())
            .expect("preload");
        let groups = renderer
            .prepare(&entities, &input(camera()), &|_| ModelSourceOptions::default(), None)
            .expect("prepare");
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].order, ModelGroupOrder::Opaque);
        assert_eq!(groups[0].batches.len(), 1);
        match &groups[0].batches[0].vertices {
            BatchVertices::Single(vertices) => assert_eq!(vertices.len(), 4),
            _ => panic!("expected single-textured batch"),
        }
    }

    #[test]
    fn draw_without_preload_yields_no_groups() {
        let renderer = SceneModelRenderer::new(provider(RenderFamily::Q2), ModelLightSampler::fullbright());
        let entities = vec![sprite_entity()];
        let groups = renderer
            .prepare(&entities, &input(camera()), &|_| ModelSourceOptions::default(), None)
            .expect("prepare");
        assert!(groups.is_empty());
    }

    #[test]
    fn q3_materials_submit_in_compiled_order() {
        let mut renderer = SceneModelRenderer::new(provider(RenderFamily::Q3), ModelLightSampler::fullbright());
        let entities = vec![sprite_entity()];
        renderer
            .preload(&entities, &|_| ModelSourceOptions::default())
            .expect("preload");
        let groups = renderer
            .prepare(&entities, &input(camera()), &|_| ModelSourceOptions::default(), None)
            .expect("prepare");
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].order, ModelGroupOrder::Compiled);
    }

    #[test]
    fn translucent_entities_submit_after_opaque() {
        let mut renderer = SceneModelRenderer::new(provider(RenderFamily::Q2), ModelLightSampler::fullbright());
        let mut faded = sprite_entity();
        faded.color.w = 0.5;
        let entities = vec![faded];
        renderer
            .preload(&entities, &|_| ModelSourceOptions::default())
            .expect("preload");
        let groups = renderer
            .prepare(&entities, &input(camera()), &|_| ModelSourceOptions::default(), None)
            .expect("prepare");
        assert_eq!(groups[0].order, ModelGroupOrder::Translucent);
    }

    #[test]
    fn sprites_cast_no_shadows() {
        assert!(!entity_casts_shadow(&sprite_entity(), false));
        let mut brush = sprite_entity();
        brush.model = SceneModel::BrushModel;
        assert!(entity_casts_shadow(&brush, false));
        assert!(!entity_casts_shadow(&brush, true));
    }

    #[test]
    fn player_translation_remaps_ranges() {
        let table = q1_player_translation(4, 12).expect("table");
        assert_eq!(table.len(), 256);
        assert_eq!(table[0], 0);
        assert_eq!(table[16], 64);
        assert_eq!(table[96], 192 + 15);
        assert!(q1_player_translation(14, 0).is_err());
    }

    #[test]
    fn flood_fill_replaces_background() {
        let palette = ModelPalette {
            colors: vec![0; 768],
            source: "palette".to_string(),
        };
        let skin = vec![7u8, 7, 7, 1];
        let flooded = flood_skin(&skin, 2, 2, &palette);
        assert_eq!(flooded[3], 1);
        assert!(flooded[0] != 255 || flooded[1] != 255 || flooded[2] != 255);
    }

    #[test]
    fn q2_shadow_lights_add_model_shadow_pass() {
        let mut renderer = SceneModelRenderer::new(provider(RenderFamily::Q2), ModelLightSampler::fullbright());
        let entities = vec![sprite_entity()];
        renderer
            .preload(&entities, &|_| ModelSourceOptions::default())
            .expect("preload");
        let atlas = renderer.provider.white_image();
        let mut view = input(camera());
        view.q2_atlas = Some(atlas);
        view.q2_lights = vec![Q2FragmentLight {
            origin: vec3(0.0, 0.0, 10.0),
            radius: 300.0,
            color: vec3(1.0, 1.0, 1.0),
            scale: 1.0,
            cone: None,
            shadow: crate::render::types::Q2ShadowProjection::Point {
                atlas_rect: vec4(0.0, 0.0, 1.0, 1.0),
            },
        }];
        let groups = renderer
            .prepare(&entities, &view, &|_| ModelSourceOptions::default(), None)
            .expect("prepare");
        assert_eq!(groups.len(), 1);
        // Sprites are unlit, so no shadow pass applies.
        assert!(matches!(groups[0].batches[0].lighting, BatchLighting::Vertex));
    }
}
