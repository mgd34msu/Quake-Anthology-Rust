//! Native held-weapon attachments on scene carriers.
//!
//! Donor provenance: `src/app/bootstrap/native-held-weapon.ts`
//! (`NativeHeldWeapons`). Definition loading (attachment files with Q2
//! digest fallback), grip caching, carrier inheritance, and the
//! hand-alignment composition are a direct port over the workspace's
//! replacement, selection, and transform siblings. Per-pose grip sampling
//! stays behind [`NativeHeldGripBinder`] (the `GripDefinition` scheme
//! drifted from attachment definitions), and carrier inheritance copies
//! flags plus lighting origin (`SceneEntity` has no opacity or shadow
//! plane fields).

use qa_client::render::scene::models::attachment::align_model_attachment;
use qa_client::render::scene::models::replacements::{replacement_entity, select_model_entity, ModelReplacementPolicy};
use qa_client::render::scene::models::transform::compose_model_transform;
use qa_client::render::scene::models::types::{EntityTransform, SceneEntity};
use qa_client::render::RenderError;
use qa_content::contract::{ContentId, ModelAttachmentDefinition, ModelTransform};
use qa_content::model_attachment::{read_model_attachment, ModelAttachmentError};
use qa_content::q2::foundation::weapon_attachments::q2_weapon_attachment;
use qa_content::q3::foundation::held_weapons::Q3_WEAPON_HAND_GRIP;
use qa_core::math::Vec3;
use std::collections::HashMap;
use std::rc::Rc;
use thiserror::Error;

/// Failure of native held-weapon resolution.
#[derive(Debug, Error)]
pub enum NativeHeldWeaponError {
    /// Resolution failure.
    #[error("{0}")]
    Resolve(String),
    /// Attachment file failure.
    #[error(transparent)]
    Attachment(#[from] ModelAttachmentError),
    /// Grip or transform failure.
    #[error(transparent)]
    Render(#[from] RenderError),
}

/// Attachment-file and digest loading for native held weapons.
pub trait NativeHeldMounts {
    /// Open an attachment file, returning its bytes when present.
    fn open_attachment(&mut self, content: &ContentId, path: &str) -> Result<Option<Vec<u8>>, NativeHeldWeaponError>;
    /// Hex digest selecting the Q2 fallback attachment.
    fn attachment_digest(&self, resource: &qa_client::render::scene::models::types::ModelResource) -> String;
}

/// Bound per-pose grip sampler.
type NativeGrip = Rc<dyn Fn(&SceneEntity) -> Result<Option<EntityTransform>, NativeHeldWeaponError>>;

/// Per-pose grip sampling bound to a reference entity.
pub trait NativeHeldGripBinder {
    /// Bind a definition to its reference entity.
    fn bind_grip(
        &mut self,
        entity: &SceneEntity,
        definition: &ModelAttachmentDefinition,
    ) -> Result<NativeGrip, NativeHeldWeaponError>;
}

/// View or shadow resolution purpose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeldPurpose {
    /// View entity.
    View,
    /// Shadow entity.
    Shadow,
}

/// Resolved held entity for one camera and purpose.
pub type ResolvedHeldEntity = Box<dyn Fn(Vec3, HeldPurpose) -> Result<Option<SceneEntity>, NativeHeldWeaponError>>;

fn entity_of(transform: &ModelTransform) -> EntityTransform {
    EntityTransform {
        origin: transform.origin,
        axis: transform.axis,
        scale: transform.scale,
    }
}

/// Native held weapons with cached carrier grips.
pub struct NativeHeldWeapons<M, G> {
    mounts: M,
    binder: G,
    model_policy: ModelReplacementPolicy,
    grips: HashMap<String, NativeGrip>,
}

impl<M: NativeHeldMounts, G: NativeHeldGripBinder> NativeHeldWeapons<M, G> {
    /// Create native held weapons over mounts, a grip binder, and a policy.
    pub fn new(mounts: M, binder: G, model_policy: ModelReplacementPolicy) -> Self {
        Self {
            mounts,
            binder,
            model_policy,
            grips: HashMap::new(),
        }
    }

    fn grip(&mut self, content: &ContentId, entity: &SceneEntity) -> Result<NativeGrip, NativeHeldWeaponError> {
        let key = format!("{}/{}", content.as_str(), entity.resource.id);
        if let Some(cached) = self.grips.get(&key) {
            return Ok(Rc::clone(cached));
        }
        let path = format!("{}.attachment.json", entity.resource.requested_path);
        let file = self.mounts.open_attachment(content, &path)?;
        let definition = match file {
            Some(bytes) => Some(read_model_attachment(&bytes)?),
            None => q2_weapon_attachment(&self.mounts.attachment_digest(&entity.resource)),
        };
        let Some(definition) = definition else {
            return Err(NativeHeldWeaponError::Resolve(format!(
                "Native held model {}/{} requires {}",
                content.as_str(),
                entity.resource.requested_path,
                path,
            )));
        };
        let bound = self.binder.bind_grip(entity, &definition)?;
        self.grips.insert(key, Rc::clone(&bound));
        Ok(bound)
    }

    /// Bind a carrier, returning the child-to-resolved-entity resolver.
    pub fn attachment(
        &mut self,
        content: &ContentId,
        carrier: &SceneEntity,
    ) -> Result<Box<dyn Fn(SceneEntity) -> ResolvedHeldEntity>, NativeHeldWeaponError> {
        let original = self.grip(content, carrier)?;
        let replacement = replacement_entity(carrier);
        let enhanced = match replacement.as_ref() {
            Some(replacement) => Some(self.grip(content, replacement)?),
            None => None,
        };
        let carrier = carrier.clone();
        let policy = self.model_policy.clone();
        Ok(Box::new(move |child: SceneEntity| {
            fn inherit(carrier: &SceneEntity, entity: &SceneEntity) -> SceneEntity {
                let mut inherited = entity.clone();
                inherited.flags = carrier.flags;
                inherited.lighting_origin = carrier.lighting_origin;
                inherited.attachments = entity
                    .attachments
                    .iter()
                    .map(|attachment| {
                        let mut child = attachment.clone();
                        child.entity = Box::new(inherit(carrier, &attachment.entity));
                        child
                    })
                    .collect();
                inherited
            }
            let held = inherit(&carrier, &child);
            let carrier = carrier.clone();
            let original = Rc::clone(&original);
            let enhanced = enhanced.clone();
            let policy = policy.clone();
            Box::new(
                move |camera: Vec3, purpose: HeldPurpose| -> Result<Option<SceneEntity>, NativeHeldWeaponError> {
                    let selected = select_model_entity(&carrier, camera, &policy, purpose == HeldPurpose::Shadow);
                    let sample = if selected.resource.id == carrier.resource.id {
                        Some(Rc::clone(&original))
                    } else {
                        enhanced.clone()
                    };
                    let Some(sample) = sample else {
                        return Err(NativeHeldWeaponError::Resolve(
                            "Native attachment replacement changed after preparation".to_owned(),
                        ));
                    };
                    let Some(grip) = sample(&selected)? else {
                        return Ok(None);
                    };
                    let hand = compose_model_transform(&selected.transform, &grip);
                    let transform = compose_model_transform(
                        &align_model_attachment(&entity_of(&Q3_WEAPON_HAND_GRIP), &hand)?,
                        &held.transform,
                    );
                    Ok(Some(SceneEntity {
                        previous_origin: transform.origin,
                        transform,
                        ..held.clone()
                    }))
                },
            )
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_client::render::scene::models::types::{EntityFlags, ModelResource, SceneModel, ScenePose};
    use qa_core::math::{vec3, vec4};

    struct FakeMounts {
        files: HashMap<String, Vec<u8>>,
    }

    impl NativeHeldMounts for FakeMounts {
        fn open_attachment(
            &mut self,
            content: &ContentId,
            path: &str,
        ) -> Result<Option<Vec<u8>>, NativeHeldWeaponError> {
            Ok(self.files.get(&format!("{}:{path}", content.as_str())).cloned())
        }

        fn attachment_digest(&self, _resource: &qa_client::render::scene::models::types::ModelResource) -> String {
            "unknown".to_owned()
        }
    }

    struct FakeBinder;

    impl NativeHeldGripBinder for FakeBinder {
        fn bind_grip(
            &mut self,
            _entity: &SceneEntity,
            _definition: &ModelAttachmentDefinition,
        ) -> Result<
            Rc<dyn Fn(&SceneEntity) -> Result<Option<EntityTransform>, NativeHeldWeaponError>>,
            NativeHeldWeaponError,
        > {
            Ok(Rc::new(|_| Ok(Some(EntityTransform::identity()))))
        }
    }

    fn entity(id: &str, path: &str) -> SceneEntity {
        SceneEntity {
            resource: ModelResource {
                id: id.to_owned(),
                requested_path: path.to_owned(),
                digest: 1,
            },
            model: SceneModel::BrushModel,
            pose: ScenePose::Frame {
                frame: 0,
                previous_frame: 0,
                back_lerp: 0.0,
            },
            transform: EntityTransform::identity(),
            previous_origin: vec3(0.0, 0.0, 0.0),
            lighting_origin: vec3(1.0, 2.0, 3.0),
            color: vec4(1.0, 1.0, 1.0, 1.0),
            skin: 0,
            shader_time_seconds: 0.0,
            flags: EntityFlags::Q2 { bits: 0 },
            attachments: Vec::new(),
            actor_slot: None,
        }
    }

    fn policy() -> ModelReplacementPolicy {
        ModelReplacementPolicy {
            q1_enhanced: false,
            q2_load: false,
            q2_use: false,
            q2_distance: 0.0,
            distance: qa_client::render::scene::models::replacements::ReplacementDistance::Source,
        }
    }

    #[test]
    fn missing_definition_errors_with_attachment_path() {
        let mounts = FakeMounts { files: HashMap::new() };
        let mut weapons = NativeHeldWeapons::new(mounts, FakeBinder, policy());
        let carrier = entity("carrier", "models/carrier.md2");
        let err = match weapons.attachment(&ContentId("q2".to_owned()), &carrier) {
            Ok(_) => panic!("missing definition"),
            Err(err) => err,
        };
        assert_eq!(
            err.to_string(),
            "Native held model q2/models/carrier.md2 requires models/carrier.md2.attachment.json"
        );
    }

    #[test]
    fn attachment_resolves_hand_aligned_child() {
        let definition = serde_json_stub();
        let mut files = HashMap::new();
        files.insert("q2:models/carrier.md2.attachment.json".to_owned(), definition);
        let mounts = FakeMounts { files };
        let mut weapons = NativeHeldWeapons::new(mounts, FakeBinder, policy());
        let carrier = entity("carrier", "models/carrier.md2");
        let attach = weapons
            .attachment(&ContentId("q2".to_owned()), &carrier)
            .expect("attach");
        let child = entity("child", "models/child.md2");
        let resolved = attach(child)(vec3(0.0, 0.0, 0.0), HeldPurpose::View)
            .expect("resolve")
            .expect("entity");
        assert_eq!(resolved.previous_origin, resolved.transform.origin);
        assert_eq!(resolved.lighting_origin, vec3(1.0, 2.0, 3.0));
    }

    fn serde_json_stub() -> Vec<u8> {
        br#"{"version":1,"digest":"sha256:0000000000000000000000000000000000000000000000000000000000000000","grip":{"origin":{"x":0,"y":0,"z":0},"axis":[{"x":1,"y":0,"z":0},{"x":0,"y":1,"z":0},{"x":0,"y":0,"z":1}],"scale":{"x":1,"y":1,"z":1}},"kind":"joint","name":"tag_weapon"}"#.to_vec()
    }
}
