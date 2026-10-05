//! Selected Quake III weapon presentation for world and view models.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/q3-selected-weapon.ts`
//! (`SelectedQ3WeaponPresenter`). Poses, barrels, lerp frames, animation configs, tags,
//! and attachment math are the ported weapon-pose, animation, and presentation helpers;
//! assets and the simulation presentation (`./assets.ts`, `./simulation/types.ts`, both out
//! of scope) arrive through the [`Q3WeaponAssets`] seam and the source shims below, and
//! the donor's async loads are sync through the host. The animation/model caches keep
//! values instead of promises, which is equivalent once loads are sync.

use std::collections::HashMap;
use std::f64::consts::PI;

use qa_content::contract::{ContentId, ModelTransform, ResolvedResourceReference};
use qa_content::q3::base::game::numeric::GameRandom;
use qa_content::q3::base::shared::definitions::Powerup;
use qa_content::q3::foundation::animation::{
    create_lerp_frame, run_lerp_frame, AnimationError, LerpFrame, RunLerpFrameInput,
};
use qa_content::q3::foundation::animation_config::{
    parse_player_animation_config, AnimationConfigError, PlayerAnimationConfig,
};
use qa_content::q3::foundation::player_pose::qvm_angles_to_axis;
use qa_content::q3::foundation::presentation::{
    attach_scene_entity, model_attachment_tag, ModelSourceOptions, PresentationError, Q3Attachment, Q3CharacterPass,
    Q3ModelPose, Q3PresentedModel, Q3SceneEntity,
};
use qa_content::q3::foundation::weapon_pose::{
    q3_torso_weapon_frame, q3_weapon_view_pose, Q3WeaponBarrel, Q3WeaponViewMotion, WeaponPoseError,
};
use qa_content::q3::presentation::ref_entity::{
    RF_DEPTHHACK, RF_FIRST_PERSON, RF_LIGHTING_ORIGIN, RF_MINLIGHT, RF_THIRD_PERSON,
};
use qa_core::identity::ActorId;
use qa_core::math::{add3, scale3, vec3, vec4, Vec3};
use qa_core::time::SourceTime;
use thiserror::Error;

/// One loaded model (donor `ModelAsset`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3WeaponModelAsset {
    /// Resolved resource.
    pub resource: ResolvedResourceReference,
    /// Presented model.
    pub model: Q3PresentedModel,
}

/// Asset reads (donor `ApplicationAssets` subset).
pub trait Q3WeaponAssets {
    /// Open a file, or [`None`] when absent.
    fn open(&mut self, content: &ContentId, path: &str) -> Option<Vec<u8>>;
    /// Load a model, or [`None`] when absent.
    fn model(&mut self, content: &ContentId, path: &str) -> Option<Q3WeaponModelAsset>;
}

/// Weapon presentation state (donor `SimulationPresentation["q3Weapon"]`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3WeaponView {
    /// Clock in milliseconds.
    pub time_ms: i32,
    /// Whether the trigger is held.
    pub firing: bool,
    /// Selected weapon number.
    pub weapon: i32,
    /// Last fire time, when fired.
    pub last_fire_ms: Option<i32>,
    /// Torso animation number.
    pub torso_animation: i32,
    /// Horizontal speed.
    pub horizontal_speed: f32,
    /// Bob cycle.
    pub bob_cycle: i32,
}

/// View-model anchor override (donor `SimulationPresentation["modelAnchor"]`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Q3WeaponAnchor {
    /// Hand model path override.
    pub path: Option<String>,
    /// View offset override.
    pub offset: Option<Vec3>,
    /// Field-of-view offset override.
    pub fov_offset: Option<Q3FovOffset>,
    /// Weapon tag override.
    pub tag: Option<String>,
}

/// Field-of-view offset (donor anchor `fovOffset`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3FovOffset {
    /// Offset scale.
    pub scale: f32,
    /// Field of view the offset starts above.
    pub above: f32,
}

/// Extra view-model attachment (donor `SimulationPresentation["modelAttachments"]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3WeaponAttachment {
    /// Tag name.
    pub tag: String,
    /// Model path.
    pub path: String,
}

/// Weapon source presentation (donor `SimulationPresentation` fields this module reads).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3WeaponSource {
    /// Presenting actor.
    pub actor: ActorId,
    /// Source content.
    pub content: ContentId,
    /// Weapon model path.
    pub path: String,
    /// Whether the source is visible.
    pub visible: bool,
    /// World origin.
    pub origin: Vec3,
    /// World angles.
    pub angles: Vec3,
    /// Current frame.
    pub frame: i32,
    /// Previous frame.
    pub old_frame: i32,
    /// Frame blend, when interpolated.
    pub back_lerp: Option<f32>,
    /// Weapon state, when selected.
    pub weapon: Option<Q3WeaponView>,
    /// View-model anchor override.
    pub model_anchor: Option<Q3WeaponAnchor>,
    /// Extra view-model attachments.
    pub model_attachments: Vec<Q3WeaponAttachment>,
}

/// Character origin and powerups (donor `Q3CharacterView` pick).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3WeaponCharacter {
    /// Character origin.
    pub origin: Vec3,
    /// Powerup bitmask.
    pub powerups: i32,
}

/// Failure to present the selected weapon, with donor messages.
#[derive(Debug, Error)]
pub enum Q3SelectedWeaponError {
    /// The source has no weapon state.
    #[error("Selected Q3 weapon presentation state is missing")]
    NoWeaponState,
    /// The animation configuration is missing.
    #[error("Selected Q3 weapon animation configuration is missing")]
    NoAnimationConfig,
    /// The hands model has no weapon tag.
    #[error("Selected Q3 hands model has no {0}")]
    NoHandsTag(String),
    /// A required weapon model is missing (the donor host throws here).
    #[error("Selected Q3 weapon model is missing: {0}")]
    MissingAsset(String),
    /// Animation failure.
    #[error(transparent)]
    Animation(#[from] AnimationError),
    /// Animation configuration failure.
    #[error(transparent)]
    AnimationConfig(#[from] AnimationConfigError),
    /// Weapon pose failure.
    #[error(transparent)]
    WeaponPose(#[from] WeaponPoseError),
    /// Presentation failure.
    #[error(transparent)]
    Presentation(#[from] PresentationError),
}

/// Strip the last extension (donor `path.replace(/\.[^.]+$/, "")`).
fn stem(path: &str) -> &str {
    path.rsplit_once('.').map_or(path, |(stem, _)| stem)
}

/// Whether a powerup bit is set.
fn has_powerup(powerups: i32, powerup: Powerup) -> bool {
    powerups & (1 << powerup as i32) != 0
}

/// Selected weapon presenter (donor `SelectedQ3WeaponPresenter`).
pub struct SelectedQ3WeaponPresenter {
    torso: LerpFrame,
    barrel: Q3WeaponBarrel,
    world_barrel: Q3WeaponBarrel,
    random: GameRandom,
    character_animation: Option<PlayerAnimationConfig>,
    animations: HashMap<ContentId, PlayerAnimationConfig>,
    models: HashMap<String, Option<Q3WeaponModelAsset>>,
}

impl SelectedQ3WeaponPresenter {
    /// Build the presenter with an optional shared animation config.
    pub fn new(character_animation: Option<PlayerAnimationConfig>) -> Self {
        Self {
            torso: create_lerp_frame(),
            barrel: Q3WeaponBarrel::default(),
            world_barrel: Q3WeaponBarrel::default(),
            random: GameRandom::new(0),
            character_animation,
            animations: HashMap::new(),
            models: HashMap::new(),
        }
    }

    /// Load (or reuse) the animation config for content (donor `animation`).
    fn animation(
        &mut self,
        assets: &mut impl Q3WeaponAssets,
        content: &ContentId,
    ) -> Result<PlayerAnimationConfig, Q3SelectedWeaponError> {
        if let Some(config) = &self.character_animation {
            return Ok(config.clone());
        }
        if let Some(config) = self.animations.get(content) {
            return Ok(config.clone());
        }
        let path = "models/players/sarge/animation.cfg";
        let Some(bytes) = assets.open(content, path) else {
            return Err(Q3SelectedWeaponError::NoAnimationConfig);
        };
        let config = parse_player_animation_config(&String::from_utf8_lossy(&bytes), path)?;
        self.animations.insert(content.clone(), config.clone());
        Ok(config)
    }

    /// Load (or reuse) a nullable model (donor `model`).
    fn model(
        &mut self,
        assets: &mut impl Q3WeaponAssets,
        content: &ContentId,
        path: &str,
    ) -> Option<Q3WeaponModelAsset> {
        let key = format!("{content}/{path}");
        if let Some(cached) = self.models.get(&key) {
            return cached.clone();
        }
        let asset = if assets.open(content, path).is_none() {
            None
        } else {
            assets.model(content, path)
        };
        self.models.insert(key, asset.clone());
        asset
    }

    /// Load a required model.
    fn required_model(
        &mut self,
        assets: &mut impl Q3WeaponAssets,
        content: &ContentId,
        path: &str,
    ) -> Result<Q3WeaponModelAsset, Q3SelectedWeaponError> {
        assets
            .model(content, path)
            .ok_or_else(|| Q3SelectedWeaponError::MissingAsset(path.to_string()))
    }

    /// World-model weapon passes (donor `world`).
    pub fn world(
        &mut self,
        assets: &mut impl Q3WeaponAssets,
        source: &Q3WeaponSource,
        character: &Q3WeaponCharacter,
        personal_model: bool,
    ) -> Result<Vec<Q3CharacterPass>, Q3SelectedWeaponError> {
        let Some(view) = &source.weapon else {
            return Ok(Vec::new());
        };
        if !source.visible || source.path.is_empty() {
            return Ok(Vec::new());
        }
        let stem = stem(&source.path);
        let gun_asset = self.required_model(assets, &source.content, &source.path)?;
        let barrel_asset = self.model(assets, &source.content, &format!("{stem}_barrel.md3"));
        let flash_asset = self.model(assets, &source.content, &format!("{stem}_flash.md3"));
        let part = |asset: &Q3WeaponModelAsset, angles: Vec3| Q3SceneEntity {
            actor: Some(source.actor.clone()),
            resource: asset.resource.clone(),
            model: asset.model.clone(),
            opacity: 1.0,
            transform: ModelTransform {
                origin: vec3(0.0, 0.0, 0.0),
                axis: qvm_angles_to_axis(angles),
                scale: vec3(1.0, 1.0, 1.0),
            },
            previous_origin: vec3(0.0, 0.0, 0.0),
            pose: Q3ModelPose::Frame {
                frame: 0,
                previous_frame: 0,
                back_lerp: 0.0,
            },
            skin: 0,
            color: vec4(1.0, 1.0, 1.0, 1.0),
            shader_time: SourceTime::Seconds(0.0),
            flags: RF_LIGHTING_ORIGIN | (if personal_model { RF_THIRD_PERSON } else { 0 }),
            lighting_origin: character.origin,
            shadow_plane: 0.0,
            attachments: Vec::new(),
        };
        let spin = self.world_barrel.step(view.time_ms, view.firing);
        let mut gun = part(&gun_asset, vec3(0.0, 0.0, 0.0));
        if let Some(barrel) = &barrel_asset {
            gun.attachments.push(Q3Attachment {
                tag: "tag_barrel".to_string(),
                entity: Box::new(part(barrel, vec3(0.0, 0.0, spin.angle))),
            });
        }
        let pass = |entity: Q3SceneEntity, shader: Option<String>| {
            let custom = shader.clone();
            Q3CharacterPass::weapon_pass(
                Some(source.content.clone()),
                entity,
                shader,
                std::rc::Rc::new(move |_| ModelSourceOptions {
                    custom_shader: custom.clone(),
                    custom_skin: None,
                }),
            )
        };
        let invisible = has_powerup(character.powerups, Powerup::PwInvis);
        let mut passes = vec![pass(
            gun.clone(),
            invisible.then(|| "powerups/invisibility".to_string()),
        )];
        if !invisible {
            if has_powerup(character.powerups, Powerup::PwBattlesuit) {
                passes.push(pass(gun.clone(), Some("powerups/battleWeapon".to_string())));
            }
            if has_powerup(character.powerups, Powerup::PwQuad) {
                passes.push(pass(gun.clone(), Some("powerups/quadWeapon".to_string())));
            }
        }
        let continuous = view.firing && matches!(view.weapon, 1 | 6 | 10);
        if let Some(flash) = &flash_asset {
            if continuous || view.last_fire_ms.is_some_and(|last| view.time_ms - last <= 20) {
                if let Some(tag) = model_attachment_tag(&gun, "tag_flash")? {
                    let muzzle = part(flash, vec3(0.0, 0.0, self.random.crandom() * 10.0));
                    passes.push(pass(attach_scene_entity(&gun, &muzzle, &tag), None));
                }
            }
        }
        Ok(passes)
    }

    /// View-model weapon entity (donor `frame`).
    pub fn frame(
        &mut self,
        assets: &mut impl Q3WeaponAssets,
        source: &Q3WeaponSource,
        field_of_view: f32,
    ) -> Result<Q3SceneEntity, Q3SelectedWeaponError> {
        let Some(view) = &source.weapon else {
            return Err(Q3SelectedWeaponError::NoWeaponState);
        };
        let stem = stem(&source.path);
        let gun_asset = self.required_model(assets, &source.content, &source.path)?;
        let hand_path = source
            .model_anchor
            .as_ref()
            .and_then(|anchor| anchor.path.clone())
            .unwrap_or_else(|| format!("{stem}_hand.md3"));
        let own_hand = self.model(assets, &source.content, &hand_path);
        let (barrel_asset, flash_asset, animation) = if source.model_anchor.is_none() {
            (
                self.model(assets, &source.content, &format!("{stem}_barrel.md3")),
                self.model(assets, &source.content, &format!("{stem}_flash.md3")),
                Some(self.animation(assets, &source.content)?),
            )
        } else {
            (None, None, None)
        };
        let hand_asset = match own_hand {
            Some(asset) => asset,
            None => self.required_model(assets, &source.content, "models/weapons2/shotgun/shotgun_hand.md3")?,
        };
        if let Some(animation) = &animation {
            run_lerp_frame(
                animation,
                &mut self.torso,
                &RunLerpFrameInput {
                    time_ms: view.time_ms,
                    new_animation: view.torso_animation,
                    speed_scale: 1.0,
                    no_player_animations: false,
                },
                None,
            )?;
        }
        let (view_origin, view_angles) = q3_weapon_view_pose(&Q3WeaponViewMotion {
            origin: source.origin,
            angles: source.angles,
            time_ms: view.time_ms,
            horizontal_speed: view.horizontal_speed,
            bob_cycle: (view.bob_cycle & 128) >> 7,
            bob_fraction_sine: (((view.bob_cycle & 127) as f64 / 127.0 * PI).sin().abs()) as f32,
            land_time: 0,
            land_change: 0.0,
        });
        let part = |asset: &Q3WeaponModelAsset, origin: Vec3, angles: Vec3| Q3SceneEntity {
            actor: Some(source.actor.clone()),
            resource: asset.resource.clone(),
            model: asset.model.clone(),
            opacity: 1.0,
            transform: ModelTransform {
                origin,
                axis: qvm_angles_to_axis(angles),
                scale: vec3(1.0, 1.0, 1.0),
            },
            previous_origin: origin,
            pose: Q3ModelPose::Frame {
                frame: 0,
                previous_frame: 0,
                back_lerp: 0.0,
            },
            skin: 0,
            color: vec4(1.0, 1.0, 1.0, 1.0),
            shader_time: SourceTime::Milliseconds(0),
            flags: RF_MINLIGHT | RF_FIRST_PERSON | RF_DEPTHHACK,
            lighting_origin: source.origin,
            shadow_plane: 0.0,
            attachments: Vec::new(),
        };
        let axis = qvm_angles_to_axis(source.angles);
        let offset = source
            .model_anchor
            .as_ref()
            .and_then(|anchor| anchor.offset)
            .unwrap_or_else(|| vec3(0.0, 0.0, 0.0));
        let fov_offset = source
            .model_anchor
            .as_ref()
            .and_then(|anchor| anchor.fov_offset)
            .map_or(0.0, |fov| fov.scale * (field_of_view - fov.above).max(0.0));
        let origin = add3(
            view_origin,
            add3(
                add3(scale3(axis[0], offset.x), scale3(axis[1], offset.y)),
                scale3(axis[2], offset.z + fov_offset),
            ),
        );
        let (frame_number, previous_number, back_lerp) = match &animation {
            None => (source.frame, source.old_frame, source.back_lerp.unwrap_or(0.0)),
            Some(animation) => (
                q3_torso_weapon_frame(animation, self.torso.frame)?,
                q3_torso_weapon_frame(animation, self.torso.old_frame)?,
                self.torso.back_lerp,
            ),
        };
        let mut hand = part(&hand_asset, origin, view_angles);
        hand.pose = Q3ModelPose::Frame {
            frame: frame_number,
            previous_frame: previous_number,
            back_lerp,
        };
        let tag_name = source
            .model_anchor
            .as_ref()
            .and_then(|anchor| anchor.tag.clone())
            .unwrap_or_else(|| "tag_weapon".to_string());
        let Some(tag) = model_attachment_tag(&hand, &tag_name)? else {
            return Err(Q3SelectedWeaponError::NoHandsTag(tag_name));
        };
        let mut attachments = Vec::new();
        for attachment in &source.model_attachments {
            let asset = self.required_model(assets, &source.content, &attachment.path)?;
            attachments.push(Q3Attachment {
                tag: attachment.tag.clone(),
                entity: Box::new(part(&asset, vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0))),
            });
        }
        let spin = self.barrel.step(view.time_ms, view.firing);
        if let Some(barrel) = &barrel_asset {
            attachments.push(Q3Attachment {
                tag: "tag_barrel".to_string(),
                entity: Box::new(part(barrel, vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, spin.angle))),
            });
        }
        let continuous = view.firing && matches!(view.weapon, 1 | 6 | 10);
        if let Some(flash) = &flash_asset {
            if continuous || view.last_fire_ms.is_some_and(|last| view.time_ms - last <= 20) {
                attachments.push(Q3Attachment {
                    tag: "tag_flash".to_string(),
                    entity: Box::new(part(
                        flash,
                        vec3(0.0, 0.0, 0.0),
                        vec3(0.0, 0.0, self.random.crandom() * 10.0),
                    )),
                });
            }
        }
        let mut gun = part(&gun_asset, vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0));
        gun.attachments = attachments;
        Ok(attach_scene_entity(&hand, &gun, &tag))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_content::contract::{
        LooseMount, MountId, MountIdentity, MountPlanId, ResourceId, ResourceIdentity, ResourceProvenance,
        ResourceResolution,
    };
    use qa_content::md3::Md3Model;
    use qa_content::q3::foundation::presentation::q3_identity_axis;
    use qa_content::q3scene::{SceneMd3, SceneMd3Frame, SceneMd3Tag};
    use qa_core::identity::IdentityOwner;
    use qa_core::math::Bounds;

    struct Stub {
        files: HashMap<String, Vec<u8>>,
        models: HashMap<String, Q3WeaponModelAsset>,
    }

    impl Q3WeaponAssets for Stub {
        fn open(&mut self, _content: &ContentId, path: &str) -> Option<Vec<u8>> {
            self.files.get(path).cloned()
        }
        fn model(&mut self, _content: &ContentId, path: &str) -> Option<Q3WeaponModelAsset> {
            self.models.get(path).cloned()
        }
    }

    fn reference(path: &str) -> ResolvedResourceReference {
        ResolvedResourceReference {
            id: ResourceId(format!("resource:{path}")),
            requested_path: path.to_string(),
            provenance: ResourceProvenance::Loose {
                mount: LooseMount {
                    identity: MountIdentity {
                        id: MountId("mount:test:1".to_string()),
                        content: ContentId("q3:classic:baseq3:1".to_string()),
                        generation: 1,
                    },
                    root_path: "/test".to_string(),
                },
                member_path: path.to_string(),
            },
            identity: ResourceIdentity::parse("identity:1:0:0:0").unwrap(),
            byte_length: 1,
            resolution: ResourceResolution::DefaultOrder {
                plan: MountPlanId("mount-plan:test:weapon".to_string()),
                rank: 0,
            },
        }
    }

    fn md3_asset(path: &str, tags: &[&str]) -> Q3WeaponModelAsset {
        Q3WeaponModelAsset {
            resource: reference(path),
            model: Q3PresentedModel::Md3(SceneMd3 {
                name: path.to_string(),
                source_model: Md3Model {
                    name: path.to_string(),
                    flags: 0,
                    skin_count: 0,
                    frames: Vec::new(),
                    tags: Vec::new(),
                    surfaces: Vec::new(),
                },
                frames: vec![SceneMd3Frame {
                    name: "frame".to_string(),
                    bounds: Bounds {
                        min: vec3(0.0, 0.0, 0.0),
                        max: vec3(0.0, 0.0, 0.0),
                    },
                    local_origin: vec3(0.0, 0.0, 0.0),
                    radius: 1.0,
                }],
                tags: vec![tags
                    .iter()
                    .map(|name| SceneMd3Tag {
                        name: name.to_string(),
                        origin: vec3(0.0, 0.0, 0.0),
                        axis: q3_identity_axis(),
                    })
                    .collect()],
                surfaces: Vec::new(),
            }),
        }
    }

    fn source(actor: ActorId) -> Q3WeaponSource {
        Q3WeaponSource {
            actor,
            content: ContentId("q3:classic:baseq3:1".to_string()),
            path: "models/weapons2/shotgun/shotgun.md3".to_string(),
            visible: true,
            origin: vec3(0.0, 0.0, 0.0),
            angles: vec3(0.0, 0.0, 0.0),
            frame: 0,
            old_frame: 0,
            back_lerp: None,
            weapon: Some(Q3WeaponView {
                time_ms: 1000,
                firing: false,
                weapon: 3,
                last_fire_ms: None,
                torso_animation: 0,
                horizontal_speed: 0.0,
                bob_cycle: 0,
            }),
            model_anchor: None,
            model_attachments: Vec::new(),
        }
    }

    fn stub() -> Stub {
        let mut models = HashMap::new();
        models.insert(
            "models/weapons2/shotgun/shotgun.md3".to_string(),
            md3_asset("models/weapons2/shotgun/shotgun.md3", &["tag_flash"]),
        );
        models.insert(
            "models/weapons2/shotgun/shotgun_hand.md3".to_string(),
            md3_asset("models/weapons2/shotgun/shotgun_hand.md3", &["tag_weapon"]),
        );
        let mut files = HashMap::new();
        files.insert("models/weapons2/shotgun/shotgun.md3".to_string(), vec![1]);
        files.insert("models/weapons2/shotgun/shotgun_hand.md3".to_string(), vec![1]);
        Stub { files, models }
    }

    #[test]
    fn stem_strips_last_extension() {
        assert_eq!(stem("a/b.md3"), "a/b");
        assert_eq!(stem("a.b/c"), "a");
        assert_eq!(stem("plain"), "plain");
    }

    #[test]
    fn world_builds_gun_passes() {
        let owner = IdentityOwner::create("q3-weapon").unwrap();
        let mut presenter = SelectedQ3WeaponPresenter::new(None);
        let mut assets = stub();
        let passes = presenter
            .world(
                &mut assets,
                &source(owner.actor(0, 1)),
                &Q3WeaponCharacter {
                    origin: vec3(1.0, 2.0, 3.0),
                    powerups: 0,
                },
                false,
            )
            .unwrap();
        assert_eq!(passes.len(), 1);
        assert_eq!(passes[0].shader, None);
        assert_eq!(passes[0].entity.lighting_origin, vec3(1.0, 2.0, 3.0));
        assert_eq!(passes[0].entity.flags & RF_LIGHTING_ORIGIN, RF_LIGHTING_ORIGIN);
    }

    #[test]
    fn world_adds_powerup_passes() {
        let owner = IdentityOwner::create("q3-weapon-power").unwrap();
        let mut presenter = SelectedQ3WeaponPresenter::new(None);
        let mut assets = stub();
        let passes = presenter
            .world(
                &mut assets,
                &source(owner.actor(0, 1)),
                &Q3WeaponCharacter {
                    origin: vec3(0.0, 0.0, 0.0),
                    powerups: 1 << Powerup::PwQuad as i32,
                },
                true,
            )
            .unwrap();
        assert_eq!(passes.len(), 2);
        assert_eq!(passes[1].shader.as_deref(), Some("powerups/quadWeapon"));
        assert_eq!(passes[0].entity.flags & RF_THIRD_PERSON, RF_THIRD_PERSON);
    }

    #[test]
    fn world_hides_without_state() {
        let owner = IdentityOwner::create("q3-weapon-hidden").unwrap();
        let mut presenter = SelectedQ3WeaponPresenter::new(None);
        let mut assets = stub();
        let mut hidden = source(owner.actor(0, 1));
        hidden.visible = false;
        let passes = presenter
            .world(
                &mut assets,
                &hidden,
                &Q3WeaponCharacter {
                    origin: vec3(0.0, 0.0, 0.0),
                    powerups: 0,
                },
                false,
            )
            .unwrap();
        assert!(passes.is_empty());
    }

    #[test]
    fn frame_attaches_gun_to_hand() {
        let owner = IdentityOwner::create("q3-weapon-frame").unwrap();
        let mut presenter = SelectedQ3WeaponPresenter::new(None);
        let mut assets = stub();
        let mut cfg = String::from("sex m\n");
        for frame in 0..31 {
            cfg.push_str(&format!("{frame} 6 0 10\n"));
        }
        assets
            .files
            .insert("models/players/sarge/animation.cfg".to_string(), cfg.into_bytes());
        let entity = presenter.frame(&mut assets, &source(owner.actor(0, 1)), 90.0).unwrap();
        assert_eq!(entity.flags & RF_FIRST_PERSON, RF_FIRST_PERSON);
        assert!(entity.resource.requested_path.ends_with("shotgun.md3"));
    }

    #[test]
    fn frame_requires_weapon_state() {
        let owner = IdentityOwner::create("q3-weapon-missing").unwrap();
        let mut presenter = SelectedQ3WeaponPresenter::new(None);
        let mut assets = stub();
        let mut stateless = source(owner.actor(0, 1));
        stateless.weapon = None;
        let error = presenter.frame(&mut assets, &stateless, 90.0).unwrap_err();
        assert!(matches!(error, Q3SelectedWeaponError::NoWeaponState));
    }
}
