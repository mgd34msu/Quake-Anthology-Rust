//! Quake III client view adjustments.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/q3-client/view.ts`
//! (`q3WeaponCamera`, `offsetQ3ViewEntity`, `offsetQ3ViewReference`).
//! [`SceneEntity`](qa_client::render::SceneEntity) carries no attachment
//! list, so the entity offset has no recursion target; optional lighting
//! and shadow fields offset only when present.

use qa_client::render::SceneEntity;
use qa_client::view::{CameraClip, SceneCamera};
use qa_content::q3::presentation::ref_entity::{Q3AdmittedRefEntity, RefEntity};
use qa_content::q3::presentation::scene::PresentSceneEntity;
use qa_core::math::Vec3;

/// Standard-narrow aspect the original weapon framing expects.
const NARROW_ASPECT: f32 = 4.0 / 3.0;

fn add(left: Vec3, right: Vec3) -> Vec3 {
    Vec3 {
        x: left.x + right.x,
        y: left.y + right.y,
        z: left.z + right.z,
    }
}

/// Keep the original weapon's vertical framing inside a wide split-screen seat.
#[must_use]
pub fn q3_weapon_camera(camera: &SceneCamera, split_screen: bool) -> SceneCamera {
    let aspect = camera.viewport.width as f32 / camera.viewport.height as f32;
    if !split_screen || aspect <= NARROW_ASPECT || matches!(camera.clip, CameraClip::Portal { .. }) {
        return *camera;
    }
    let scale = NARROW_ASPECT / aspect;
    let source = camera.projection;
    let mut projection = source;
    projection[0] = source[0] * scale;
    projection[5] = source[5] * scale;
    SceneCamera { projection, ..*camera }
}

/// Keep the original first-person mesh attached to its translated camera.
#[must_use]
pub fn offset_q3_view_entity(entity: &SceneEntity, delta: Option<Vec3>) -> SceneEntity {
    let Some(delta) = delta else {
        return *entity;
    };
    let mut result = *entity;
    result.transform.origin = add(result.transform.origin, delta);
    result.previous_origin = add(result.previous_origin, delta);
    if let Some(origin) = result.lighting_origin {
        result.lighting_origin = Some(add(origin, delta));
    }
    if let Some(plane) = result.shadow_plane {
        result.shadow_plane = Some(plane + delta.z);
    }
    result
}

/// Keep a presented model entity attached to its translated camera.
///
/// Presented entities carry no attachments, so the donor's recursion has no
/// target here.
#[must_use]
pub fn offset_q3_view_presented_entity(entity: &PresentSceneEntity, delta: Option<Vec3>) -> PresentSceneEntity {
    let Some(delta) = delta else {
        return entity.clone();
    };
    let mut result = entity.clone();
    result.origin = add(result.origin, delta);
    result.previous_origin = add(result.previous_origin, delta);
    result.lighting_origin = add(result.lighting_origin, delta);
    result.shadow_plane += delta.z;
    result
}

/// Keep an admitted reference entity attached to its translated camera.
#[must_use]
pub fn offset_q3_view_reference(entity: &Q3AdmittedRefEntity, delta: Option<Vec3>) -> Q3AdmittedRefEntity {
    let Some(delta) = delta else {
        return entity.clone();
    };
    let Q3AdmittedRefEntity::Entity(reference) = entity else {
        return entity.clone();
    };
    match reference {
        RefEntity::Portal(_) => entity.clone(),
        RefEntity::Model(model) => {
            let mut next = model.clone();
            next.origin = add(next.origin, delta);
            next.old_origin = add(next.old_origin, delta);
            next.lighting_origin = add(next.lighting_origin, delta);
            next.shadow_plane += delta.z;
            Q3AdmittedRefEntity::Entity(RefEntity::Model(next))
        }
        RefEntity::Sprite(sprite) => {
            let mut next = sprite.clone();
            next.origin = add(next.origin, delta);
            Q3AdmittedRefEntity::Entity(RefEntity::Sprite(next))
        }
        RefEntity::Beam(beam) => {
            let mut next = beam.clone();
            next.origin = add(next.origin, delta);
            next.old_origin = add(next.old_origin, delta);
            Q3AdmittedRefEntity::Entity(RefEntity::Beam(next))
        }
        RefEntity::RailCore(core) => {
            let mut next = core.clone();
            next.origin = add(next.origin, delta);
            next.old_origin = add(next.old_origin, delta);
            Q3AdmittedRefEntity::Entity(RefEntity::RailCore(next))
        }
        RefEntity::RailRings(rings) => {
            let mut next = rings.clone();
            next.origin = add(next.origin, delta);
            next.old_origin = add(next.old_origin, delta);
            Q3AdmittedRefEntity::Entity(RefEntity::RailRings(next))
        }
        RefEntity::Lightning(lightning) => {
            let mut next = lightning.clone();
            next.origin = add(next.origin, delta);
            next.old_origin = add(next.old_origin, delta);
            Q3AdmittedRefEntity::Entity(RefEntity::Lightning(next))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_client::view::{ModelTransform, Rect};
    use qa_content::q3::presentation::ref_entity::{
        create_beam_entity, create_lightning_entity, create_model_entity, create_portal_entity,
        create_rail_core_entity, create_rail_rings_entity, create_sprite_entity,
    };
    use qa_core::math::{vec3, Axis};

    fn axis() -> Axis {
        [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)]
    }

    fn camera(width: i32, height: i32) -> SceneCamera {
        SceneCamera {
            origin: vec3(0.0, 0.0, 0.0),
            axis: axis(),
            projection: [
                1.0, 0.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 0.0, 3.0, 0.0, 0.0, 0.0, 4.0, 1.0,
            ],
            viewport: Rect {
                x: 0,
                y: 0,
                width,
                height,
            },
            clip: CameraClip::None,
        }
    }

    fn entity() -> SceneEntity {
        SceneEntity {
            entity_number: 1,
            model: 2,
            transform: ModelTransform {
                origin: vec3(1.0, 2.0, 3.0),
                axis: axis(),
                scale: 1.0,
            },
            previous_origin: vec3(4.0, 5.0, 6.0),
            pose: qa_client::render::ModelPose {
                frame: 0,
                old_frame: 0,
                back_lerp: 0.0,
            },
            skin: 0,
            color: qa_core::math::Vec4 {
                x: 1.0,
                y: 1.0,
                z: 1.0,
                w: 1.0,
            },
            flags: 0,
            lighting_origin: Some(vec3(7.0, 8.0, 9.0)),
            shadow_plane: Some(10.0),
            opacity: None,
        }
    }

    #[test]
    fn narrow_camera_is_unchanged() {
        let camera = camera(640, 480);
        assert_eq!(q3_weapon_camera(&camera, true), camera);
        assert_eq!(q3_weapon_camera(&camera, false), camera);
    }

    #[test]
    fn wide_split_screen_scales_weapon_axes() {
        let camera = camera(1280, 480);
        let scaled = q3_weapon_camera(&camera, true);
        let aspect = 1280.0f32 / 480.0;
        let scale = (4.0 / 3.0) / aspect;
        assert_eq!(scaled.projection[0], scale);
        assert_eq!(scaled.projection[5], 2.0 * scale);
        assert_eq!(&scaled.projection[1..5], &[0.0, 0.0, 0.0, 0.0]);
        assert_eq!(scaled.viewport, camera.viewport);
        assert_eq!(q3_weapon_camera(&camera, false), camera);
    }

    #[test]
    fn portal_clip_skips_weapon_scaling() {
        let mut camera = camera(1280, 480);
        camera.clip = CameraClip::Portal {
            plane: qa_core::math::Plane {
                normal: vec3(0.0, 0.0, 1.0),
                distance: 0.0,
            },
            mirror: false,
        };
        assert_eq!(q3_weapon_camera(&camera, true), camera);
    }

    #[test]
    fn entity_offset_moves_origins_and_shadow() {
        let moved = offset_q3_view_entity(&entity(), Some(vec3(1.0, -2.0, 4.0)));
        assert_eq!(moved.transform.origin, vec3(2.0, 0.0, 7.0));
        assert_eq!(moved.previous_origin, vec3(5.0, 3.0, 10.0));
        assert_eq!(moved.lighting_origin, Some(vec3(8.0, 6.0, 13.0)));
        assert_eq!(moved.shadow_plane, Some(14.0));
        assert_eq!(moved.entity_number, 1);
    }

    #[test]
    fn entity_offset_handles_missing_delta_and_fields() {
        let source = entity();
        assert_eq!(offset_q3_view_entity(&source, None), source);
        let mut bare = source;
        bare.lighting_origin = None;
        bare.shadow_plane = None;
        let moved = offset_q3_view_entity(&bare, Some(vec3(1.0, 1.0, 1.0)));
        assert_eq!(moved.lighting_origin, None);
        assert_eq!(moved.shadow_plane, None);
        assert_eq!(moved.transform.origin, vec3(2.0, 3.0, 4.0));
    }

    #[test]
    fn reference_model_offsets_all_origins() {
        let mut model = create_model_entity(qa_content::q3::presentation::ref_entity::SceneModel::default_model());
        model.origin = vec3(1.0, 0.0, 0.0);
        model.old_origin = vec3(0.0, 1.0, 0.0);
        model.lighting_origin = vec3(0.0, 0.0, 1.0);
        model.shadow_plane = 5.0;
        let source = Q3AdmittedRefEntity::Entity(RefEntity::Model(model));
        let moved = offset_q3_view_reference(&source, Some(vec3(1.0, 1.0, 1.0)));
        let Q3AdmittedRefEntity::Entity(RefEntity::Model(next)) = moved else {
            panic!("expected a model reference");
        };
        assert_eq!(next.origin, vec3(2.0, 1.0, 1.0));
        assert_eq!(next.old_origin, vec3(1.0, 2.0, 1.0));
        assert_eq!(next.lighting_origin, vec3(1.0, 1.0, 2.0));
        assert_eq!(next.shadow_plane, 6.0);
        assert_eq!(offset_q3_view_reference(&source, None), source);
    }

    #[test]
    fn reference_sprite_offsets_origin_only() {
        let mut sprite = create_sprite_entity();
        sprite.origin = vec3(1.0, 2.0, 3.0);
        let source = Q3AdmittedRefEntity::Entity(RefEntity::Sprite(sprite));
        let moved = offset_q3_view_reference(&source, Some(vec3(0.0, 0.0, 2.0)));
        let Q3AdmittedRefEntity::Entity(RefEntity::Sprite(next)) = moved else {
            panic!("expected a sprite reference");
        };
        assert_eq!(next.origin, vec3(1.0, 2.0, 5.0));
        assert_eq!(next.radius, 0.0);
    }

    #[test]
    fn reference_beams_offset_both_ends() {
        let delta = Some(vec3(1.0, 0.0, 0.0));
        let beam = Q3AdmittedRefEntity::Entity(RefEntity::Beam(create_beam_entity()));
        let Q3AdmittedRefEntity::Entity(RefEntity::Beam(next)) = offset_q3_view_reference(&beam, delta) else {
            panic!("expected a beam reference");
        };
        assert_eq!(
            (next.origin, next.old_origin),
            (vec3(1.0, 0.0, 0.0), vec3(1.0, 0.0, 0.0))
        );
        for reference in [
            RefEntity::RailCore(create_rail_core_entity()),
            RefEntity::RailRings(create_rail_rings_entity()),
            RefEntity::Lightning(create_lightning_entity()),
        ] {
            let source = Q3AdmittedRefEntity::Entity(reference);
            let moved = offset_q3_view_reference(&source, delta);
            assert_ne!(moved, source);
        }
    }

    #[test]
    fn reference_portal_is_unchanged() {
        let source = Q3AdmittedRefEntity::Entity(RefEntity::Portal(create_portal_entity()));
        assert_eq!(offset_q3_view_reference(&source, Some(vec3(9.0, 9.0, 9.0))), source);
    }

    #[test]
    fn presented_entity_offsets_pose() {
        use qa_content::q3::presentation::ref_entity::{PresentResource, SceneModel};
        use qa_content::q3::presentation::scene::PresentEntityModel;
        use qa_core::math::vec4;

        let entity = PresentSceneEntity {
            actor: None,
            resource: PresentResource::new("models/box.md3"),
            model: PresentEntityModel::Decoded(qa_content::q3::presentation::ref_entity::Q3DecodedModel::Framed {
                frames: Vec::new(),
            }),
            origin: vec3(1.0, 2.0, 3.0),
            axis: axis(),
            previous_origin: vec3(4.0, 5.0, 6.0),
            frame: 0,
            previous_frame: 0,
            back_lerp: 0.0,
            skin: 0,
            color: vec4(1.0, 1.0, 1.0, 1.0),
            shader_time: 0.0,
            render_flags: 0,
            lighting_origin: vec3(7.0, 8.0, 9.0),
            shadow_plane: 2.0,
        };
        assert_eq!(offset_q3_view_presented_entity(&entity, None), entity);
        let moved = offset_q3_view_presented_entity(&entity, Some(vec3(1.0, 1.0, 1.0)));
        assert_eq!(moved.origin, vec3(2.0, 3.0, 4.0));
        assert_eq!(moved.previous_origin, vec3(5.0, 6.0, 7.0));
        assert_eq!(moved.lighting_origin, vec3(8.0, 9.0, 10.0));
        assert_eq!(moved.shadow_plane, 3.0);
        assert_eq!(moved.model, entity.model);
        let _ = SceneModel::default_model();
    }
}
