//! Quake III foundation: presentation.
//!
//! Donor provenance: `src/content/q3/foundation/presentation.ts`.

use crate::contract::{ContentId, ModelTransform, ResolvedResourceReference};
use crate::md3::{interpolate_md3_tags, Md3Tag, SkinSurface};
use crate::md5::{sample_md5_pose, Md5AnimationFrame, Md5Joint, SkeletonJointPose};
use crate::q3scene::{joint_attachment_tag, SceneMd3};
use qa_core::identity::ActorId;
use qa_core::math::{add3, scale3, sub3, vec3, Axis, Vec3, Vec4};
use qa_core::time::SourceTime;
use std::rc::Rc;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::foundation::animation::*;
use crate::q3::foundation::assets::*;
use crate::q3::foundation::mirrors::*;
use crate::q3::foundation::player_pose::*;

// ---------------------------------------------------------------------------
// presentation.ts: CG_Player, CG_PlayerAnimation, powerup passes.
// ---------------------------------------------------------------------------

/// Presented model (the `q3-md3` and `md5` arms of `DecodedModel` the
/// attachment lookup supports).
#[derive(Debug, Clone, PartialEq)]
pub enum Q3PresentedModel {
    /// MD3 scene model.
    Md3(SceneMd3),
    /// MD5 joints and frames.
    Md5 {
        /// Joints.
        joints: Vec<Md5Joint>,
        /// Frames.
        frames: Vec<Md5AnimationFrame>,
    },
}

/// Presented model pose (`ModelPose`, foundation arms).
#[derive(Debug, Clone, PartialEq)]
pub enum Q3ModelPose {
    /// Frame interpolation.
    Frame {
        /// Current frame.
        frame: i32,
        /// Previous frame.
        previous_frame: i32,
        /// Blend factor.
        back_lerp: f32,
    },
    /// Skeleton joints.
    Skeleton {
        /// Joints.
        joints: Vec<SkeletonJointPose>,
    },
}

/// Attached entity (`SceneEntity` attachment entry).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3Attachment {
    /// Tag name.
    pub tag: String,
    /// Attached entity.
    pub entity: Box<Q3SceneEntity>,
}

/// Presented scene entity (`SceneEntity`, foundation fields).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3SceneEntity {
    /// Actor.
    pub actor: Option<ActorId>,
    /// Resource.
    pub resource: ResolvedResourceReference,
    /// Model.
    pub model: Q3PresentedModel,
    /// Opacity.
    pub opacity: f32,
    /// Transform.
    pub transform: ModelTransform,
    /// Previous origin.
    pub previous_origin: Vec3,
    /// Pose.
    pub pose: Q3ModelPose,
    /// Skin index.
    pub skin: i32,
    /// Color.
    pub color: Vec4,
    /// Shader time.
    pub shader_time: SourceTime,
    /// Q3 flag bits.
    pub flags: i32,
    /// Lighting origin.
    pub lighting_origin: Vec3,
    /// Shadow plane.
    pub shadow_plane: f32,
    /// Attachments.
    pub attachments: Vec<Q3Attachment>,
}

/// Resolved attachment tag (`ModelTag` with scale).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3AttachmentTag {
    /// Name.
    pub name: String,
    /// Origin.
    pub origin: Vec3,
    /// Axis.
    pub axis: Axis,
    /// Scale.
    pub scale: f32,
}

pub(crate) fn model_world_direction(transform: &ModelTransform, value: Vec3) -> Vec3 {
    let x = value.x * transform.scale.x;
    let y = value.y * transform.scale.y;
    let z = value.z * transform.scale.z;
    let [forward, left, up] = transform.axis;
    vec3(
        forward.x * x + left.x * y + up.x * z,
        forward.y * x + left.y * y + up.y * z,
        forward.z * x + left.z * y + up.z * z,
    )
}

pub(crate) fn model_world_point(transform: &ModelTransform, value: Vec3) -> Vec3 {
    let direction = model_world_direction(transform, value);
    vec3(
        transform.origin.x + direction.x,
        transform.origin.y + direction.y,
        transform.origin.z + direction.z,
    )
}

pub(crate) fn compose_model_transform(parent: &ModelTransform, child: &ModelTransform) -> ModelTransform {
    ModelTransform {
        origin: model_world_point(parent, child.origin),
        axis: [
            model_world_direction(parent, scale3(child.axis[0], child.scale.x)),
            model_world_direction(parent, scale3(child.axis[1], child.scale.y)),
            model_world_direction(parent, scale3(child.axis[2], child.scale.z)),
        ],
        scale: vec3(1.0, 1.0, 1.0),
    }
}

/// Resolve a named attachment tag (`modelAttachmentTag`).
pub fn model_attachment_tag(entity: &Q3SceneEntity, name: &str) -> Result<Option<Q3AttachmentTag>, Q3FoundationError> {
    match &entity.model {
        Q3PresentedModel::Md3(scene) => {
            let Q3ModelPose::Frame {
                frame,
                previous_frame,
                back_lerp,
            } = &entity.pose
            else {
                return Ok(None);
            };
            if scene.tags.is_empty() || scene.frames.is_empty() {
                return Ok(None);
            }
            if !back_lerp.is_finite() {
                return Err(range("MD3 tag fraction must be finite"));
            }
            let last = scene.frames.len() - 1;
            let previous_index = (*previous_frame).min(last as i32);
            let current_index = (*frame).min(last as i32);
            if previous_index < 0 || current_index < 0 {
                return Ok(None);
            }
            let first = scene
                .tags
                .get(previous_index as usize)
                .and_then(|tags| tags.iter().find(|tag| tag.name == name));
            let second = scene
                .tags
                .get(current_index as usize)
                .and_then(|tags| tags.iter().find(|tag| tag.name == name));
            let (Some(first), Some(second)) = (first, second) else {
                return Ok(None);
            };
            let tag = interpolate_md3_tags(
                &Md3Tag {
                    name: first.name.clone(),
                    origin: first.origin,
                    axes: first.axis,
                },
                &Md3Tag {
                    name: second.name.clone(),
                    origin: second.origin,
                    axes: second.axis,
                },
                name,
                1.0 - *back_lerp,
            );
            Ok(Some(Q3AttachmentTag {
                name: name.to_string(),
                origin: tag.origin,
                axis: tag.axes,
                scale: 1.0,
            }))
        }
        Q3PresentedModel::Md5 { joints, frames } => {
            let Some(index) = joints.iter().position(|joint| joint.name == name) else {
                return Ok(None);
            };
            let poses: Vec<SkeletonJointPose> = match &entity.pose {
                Q3ModelPose::Skeleton { joints } => joints.clone(),
                Q3ModelPose::Frame {
                    frame,
                    previous_frame,
                    back_lerp,
                } => {
                    if *frame < 0 || *previous_frame < 0 || !back_lerp.is_finite() || frames.is_empty() {
                        return Err(range("Invalid MD5 frame selection"));
                    }
                    sample_md5_pose(frames, *frame, *previous_frame, *back_lerp)
                }
            };
            let Some(joint) = poses.get(index) else {
                return Ok(None);
            };
            let tag = joint_attachment_tag(name, joint);
            Ok(Some(Q3AttachmentTag {
                name: tag.name,
                origin: tag.origin,
                axis: tag.axis,
                scale: tag.scale,
            }))
        }
    }
}

/// Attach a child entity at a tag (`attachSceneEntity`).
#[must_use]
pub fn attach_scene_entity(parent: &Q3SceneEntity, child: &Q3SceneEntity, tag: &Q3AttachmentTag) -> Q3SceneEntity {
    let tag_transform = compose_model_transform(
        &parent.transform,
        &ModelTransform {
            origin: tag.origin,
            axis: tag.axis,
            scale: vec3(tag.scale, tag.scale, tag.scale),
        },
    );
    let transform = compose_model_transform(&tag_transform, &child.transform);
    let delta = model_world_direction(&tag_transform, sub3(child.previous_origin, child.transform.origin));
    Q3SceneEntity {
        transform,
        previous_origin: add3(transform.origin, delta),
        lighting_origin: parent.lighting_origin,
        ..child.clone()
    }
}

/// Character view (`Q3CharacterView`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3CharacterView {
    /// Actor.
    pub actor: ActorId,
    /// Origin.
    pub origin: Vec3,
    /// Angles.
    pub angles: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Movement direction.
    pub movement_direction: f32,
    /// Animation.
    pub animation: Q3AnimationState,
    /// Source flags.
    pub source_flags: i32,
    /// Powerups bitmask.
    pub powerups: i32,
    /// Team.
    pub team: Option<Q3Team>,
    /// Color.
    pub color: Vec4,
    /// Visual scale.
    pub scale: Option<f32>,
    /// Opacity.
    pub opacity: Option<f32>,
}

/// Model source options the character passes set (`ModelSourceOptions`
/// fields used by the foundation).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ModelSourceOptions {
    /// Custom shader override.
    pub custom_shader: Option<String>,
    /// Custom skin surfaces.
    pub custom_skin: Option<Vec<SkinSurface>>,
}

/// Pass option resolver (`Q3CharacterPass` options closure).
#[derive(Clone)]
pub enum Q3PassOptions {
    /// Resolve from character parts.
    Character,
    /// Foreign resolver, preserved across attachment.
    Foreign(Rc<dyn Fn(&Q3SceneEntity) -> ModelSourceOptions>),
}

/// Character render pass (`Q3CharacterPass`).
#[derive(Clone)]
pub struct Q3CharacterPass {
    /// Content.
    pub content: Option<ContentId>,
    /// Entity.
    pub entity: Q3SceneEntity,
    /// Shader override.
    pub shader: Option<String>,
    /// Option resolver.
    pub options: Q3PassOptions,
}

impl Q3CharacterPass {
    /// Build a foreign (weapon) pass.
    pub fn weapon_pass(
        content: Option<ContentId>,
        entity: Q3SceneEntity,
        shader: Option<String>,
        options: Rc<dyn Fn(&Q3SceneEntity) -> ModelSourceOptions>,
    ) -> Self {
        Self {
            content,
            entity,
            shader,
            options: Q3PassOptions::Foreign(options),
        }
    }

    /// Resolve model source options for an entity.
    #[must_use]
    pub fn options(&self, assets: &Q3CharacterAssets, entity: &Q3SceneEntity) -> ModelSourceOptions {
        match &self.options {
            Q3PassOptions::Foreign(resolve) => resolve(entity),
            Q3PassOptions::Character => {
                let mut resolved = ModelSourceOptions::default();
                let part = [&assets.lower, &assets.upper, &assets.head]
                    .into_iter()
                    .find(|part| part.resource.id == entity.resource.id);
                if let Some(part) = part {
                    resolved.custom_skin = Some(part.surfaces.clone());
                }
                if self.shader.is_some() {
                    resolved.custom_shader = self.shader.clone();
                }
                resolved
            }
        }
    }
}

/// Character render options (`Q3CharacterRenderOptions`).
#[derive(Clone)]
pub struct Q3CharacterRenderOptions {
    /// Time in milliseconds.
    pub time_ms: i32,
    /// Frame time in milliseconds.
    pub frame_ms: i32,
    /// Shader time.
    pub shader_time: SourceTime,
    /// Swing speed.
    pub swing_speed: f32,
    /// Freeze animations.
    pub no_player_animations: bool,
    /// Personal model flag.
    pub personal_model: bool,
    /// Shadow plane.
    pub shadow_plane: Option<f32>,
    /// Prepared weapon passes in tag_weapon coordinates.
    pub weapon: Vec<Q3CharacterPass>,
}

/// Character presenter (`Q3CharacterPresenter`).
#[derive(Debug, Clone)]
pub struct Q3CharacterPresenter {
    /// Pose state.
    pub pose: PlayerPoseState,
    /// Assets.
    pub assets: Q3CharacterAssets,
}

impl Q3CharacterPresenter {
    /// Bind assets.
    #[must_use]
    pub fn new(assets: Q3CharacterAssets) -> Self {
        Self {
            pose: create_player_pose_state(),
            assets,
        }
    }

    /// Reset to a view (`reset`).
    pub fn reset(&mut self, view: &Q3CharacterView, time_ms: i32) -> Result<(), Q3FoundationError> {
        clear_lerp_frame(
            &self.assets.animation,
            &mut self.pose.legs.lerp,
            view.animation.legs,
            time_ms,
            None,
        )?;
        clear_lerp_frame(
            &self.assets.animation,
            &mut self.pose.torso.lerp,
            view.animation.torso,
            time_ms,
            None,
        )?;
        self.pose.legs.lerp = create_lerp_frame();
        self.pose.legs.yaw_angle = view.angles.y;
        self.pose.legs.yawing = false;
        self.pose.legs.pitch_angle = 0.0;
        self.pose.legs.pitching = false;
        self.pose.torso.lerp = create_lerp_frame();
        self.pose.torso.yaw_angle = view.angles.y;
        self.pose.torso.yawing = false;
        self.pose.torso.pitch_angle = view.angles.x;
        self.pose.torso.pitching = false;
        Ok(())
    }

    /// Build render passes (`frame`).
    pub fn frame(
        &mut self,
        view: &Q3CharacterView,
        options: &Q3CharacterRenderOptions,
    ) -> Result<Vec<Q3CharacterPass>, Q3FoundationError> {
        if view.source_flags & 0x80 != 0 {
            return Ok(Vec::new());
        }
        let axes = calculate_player_pose(
            &mut self.pose,
            &CalculatePlayerPoseInput {
                entity: PoseEntityState {
                    e_flags: view.source_flags,
                    velocity: view.velocity,
                    movement_direction: view.movement_direction,
                    legs_anim: view.animation.legs,
                    torso_anim: view.animation.torso,
                },
                fixed_legs: self.assets.animation.fixed_legs,
                fixed_torso: self.assets.animation.fixed_torso,
                lerp_angles: view.angles,
                time_ms: options.time_ms,
                frame_time_ms: options.frame_ms,
                swing_speed: options.swing_speed,
            },
        )?;
        let speed_scale = if view.powerups & (1 << Q3Powerup::HASTE) != 0 {
            1.5
        } else {
            1.0
        };
        let legs_animation =
            if self.pose.legs.yawing && view.animation.legs & !ANIMATION_TOGGLE_BIT == Q3PlayerAnimation::LEGS_IDLE {
                Q3PlayerAnimation::LEGS_TURN
            } else {
                view.animation.legs
            };
        run_lerp_frame(
            &self.assets.animation,
            &mut self.pose.legs.lerp,
            &RunLerpFrameInput {
                time_ms: options.time_ms,
                new_animation: legs_animation,
                speed_scale,
                no_player_animations: options.no_player_animations,
            },
            None,
        )?;
        run_lerp_frame(
            &self.assets.animation,
            &mut self.pose.torso.lerp,
            &RunLerpFrameInput {
                time_ms: options.time_ms,
                new_animation: view.animation.torso,
                speed_scale,
                no_player_animations: options.no_player_animations,
            },
            None,
        )?;
        let flags = 0x80
            | (if options.personal_model { 2 } else { 0 })
            | (if options.shadow_plane.is_some() { 0x40 } else { 0 });
        let unit = vec3(1.0, 1.0, 1.0);
        let zero = vec3(0.0, 0.0, 0.0);
        let scale_value = view.scale.unwrap_or(1.0);
        let part = |asset: &Q3CharacterPart,
                    axis: Axis,
                    origin: Vec3,
                    frame: Option<&LerpFrame>,
                    attachments: Vec<Q3Attachment>,
                    scaled: bool|
         -> Q3SceneEntity {
            Q3SceneEntity {
                actor: Some(view.actor.clone()),
                resource: asset.resource.clone(),
                model: Q3PresentedModel::Md3(asset.model.clone()),
                opacity: view.opacity.unwrap_or(1.0),
                transform: ModelTransform {
                    origin,
                    axis,
                    scale: if scaled {
                        vec3(scale_value, scale_value, scale_value)
                    } else {
                        unit
                    },
                },
                previous_origin: origin,
                pose: Q3ModelPose::Frame {
                    frame: frame.map_or(0, |lerp| lerp.frame),
                    previous_frame: frame.map_or(0, |lerp| lerp.old_frame),
                    back_lerp: frame.map_or(0.0, |lerp| lerp.back_lerp),
                },
                skin: 0,
                color: view.color,
                shader_time: options.shader_time,
                flags,
                lighting_origin: view.origin,
                shadow_plane: options.shadow_plane.unwrap_or(0.0),
                attachments,
            }
        };
        let head = part(&self.assets.head, axes.head, zero, None, Vec::new(), false);
        let torso = part(
            &self.assets.upper,
            axes.torso,
            zero,
            Some(&self.pose.torso.lerp),
            vec![Q3Attachment {
                tag: "tag_head".to_string(),
                entity: Box::new(head),
            }],
            false,
        );
        let legs = part(
            &self.assets.lower,
            axes.legs,
            view.origin,
            Some(&self.pose.legs.lerp),
            vec![Q3Attachment {
                tag: "tag_torso".to_string(),
                entity: Box::new(torso.clone()),
            }],
            true,
        );
        let pass = |shader: Option<&str>| Q3CharacterPass {
            content: None,
            entity: legs.clone(),
            shader: shader.map(str::to_string),
            options: Q3PassOptions::Character,
        };
        let mut passes = if view.powerups & (1 << Q3Powerup::INVIS) != 0 {
            vec![pass(Some("powerups/invisibility"))]
        } else {
            vec![pass(None)]
        };
        if view.powerups & (1 << Q3Powerup::INVIS) == 0 {
            if view.powerups & (1 << Q3Powerup::QUAD) != 0 {
                passes.push(pass(Some(if view.team == Some(Q3Team::Red) {
                    "powerups/blueflag"
                } else {
                    "powerups/quad"
                })));
            }
            if view.powerups & (1 << Q3Powerup::REGEN) != 0 && options.time_ms / 100 % 10 == 1 {
                passes.push(pass(Some("powerups/regen")));
            }
            if view.powerups & (1 << Q3Powerup::BATTLESUIT) != 0 {
                passes.push(pass(Some("powerups/battleSuit")));
            }
        }
        if let Some(torso_tag) = model_attachment_tag(&legs, "tag_torso")? {
            let world_torso = attach_scene_entity(&legs, &torso, &torso_tag);
            if let Some(weapon_tag) = model_attachment_tag(&world_torso, "tag_weapon")? {
                for weapon in &options.weapon {
                    passes.push(Q3CharacterPass {
                        content: weapon.content.clone(),
                        entity: attach_scene_entity(&world_torso, &weapon.entity, &weapon_tag),
                        shader: weapon.shader.clone(),
                        options: weapon.options.clone(),
                    });
                }
            }
        }
        Ok(passes)
    }
}

/// Identity axis (`q3IdentityAxis`).
#[must_use]
pub fn q3_identity_axis() -> Axis {
    qvm_angles_to_axis(vec3(0.0, 0.0, 0.0))
}
