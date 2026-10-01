//! Component primary body material passes.
//!
//! Donor: `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/component-bodies.ts`
//! (`bodyMaterials`, `bodyBaseVisible`).
//! Equal passes from distinct source body parts collapse; repetitions
//! inside one helper remain. The material key is canonical over bits and
//! names (the donor stringifies the same tuple as JSON).

use std::collections::{HashMap, HashSet};

use qa_client::render::scene::models::types::ModelSourceOptions;
use qa_client::render::SceneEntity;
use qa_content::contract::{ContentId, PresentationOwner, QvmBodyPart};
use qa_content::q3::presentation::ref_entity::{SceneShader, SceneSkin};
use qa_core::identity::ActorId;
use qa_core::math::{Vec2, Vec3, Vec4};

/// One material pass (`BodyMaterial`).
#[derive(Debug, Clone, PartialEq)]
pub struct BodyMaterial {
    /// Custom shader override.
    pub custom_shader: Option<SceneShader>,
    /// Custom skin override.
    pub custom_skin: Option<SceneSkin>,
    /// Shader color.
    pub shader_rgba: Vec4,
    /// Shader texture coordinate.
    pub shader_tex_coord: Vec2,
    /// Shader clock offset.
    pub shader_time: f32,
    /// Render flags.
    pub render_flags: i32,
    /// Lighting sample origin.
    pub lighting_origin: Vec3,
    /// Shadow plane.
    pub shadow_plane: f32,
    /// Non-normalized axes.
    pub non_normalized_axes: bool,
}

/// One source body part with its passes.
#[derive(Debug, Clone, PartialEq)]
pub struct ComponentBodyPart {
    /// Body part.
    pub part: QvmBodyPart,
    /// Base pass submitted.
    pub base: bool,
    /// Material passes.
    pub passes: Vec<BodyMaterial>,
}

/// One component body (`ComponentBody`).
#[derive(Debug, Clone, PartialEq)]
pub struct ComponentBody {
    /// Presenting owner.
    pub owner: PresentationOwner,
    /// Actor.
    pub actor: ActorId,
    /// Content identity.
    pub content: ContentId,
    /// Presentation time.
    pub time: f64,
    /// Source parts.
    pub parts: Vec<ComponentBodyPart>,
}

/// A native primary body already posed by its original cgame, including
/// its original material passes (`PreparedPrimaryBody`).
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedPrimaryBody {
    /// Actor.
    pub actor: ActorId,
    /// Body part.
    pub part: QvmBodyPart,
    /// Content identity.
    pub content: ContentId,
    /// Posed entity.
    pub entity: SceneEntity,
    /// Base pass submitted.
    pub base: bool,
    /// Model source options.
    pub options: ModelSourceOptions,
    /// Shader content identity.
    pub shader_content: ContentId,
    /// Presentation time.
    pub time: f64,
}

fn material_key(pass: &BodyMaterial) -> String {
    let rgba = &pass.shader_rgba;
    let uv = &pass.shader_tex_coord;
    let light = &pass.lighting_origin;
    // An explicit Q3 shader replaces skin lookup; inactive body-part skin
    // maps do not create extra passes.
    let skin = if pass.custom_shader.is_none() {
        pass.custom_skin.as_ref().map(|skin| {
            skin.surfaces
                .iter()
                .map(|mapping| format!("{}={}", mapping.name, mapping.shader))
                .collect::<Vec<_>>()
                .join(",")
        })
    } else {
        None
    };
    format!(
        "{}|{:08x}{:08x}{:08x}{:08x}|{:08x}{:08x}|{:08x}|{}|{:08x}{:08x}{:08x}|{:08x}|{}|{}",
        pass.custom_shader
            .as_ref()
            .map_or("\u{0}", |shader| shader.name.as_str()),
        rgba.x.to_bits(),
        rgba.y.to_bits(),
        rgba.z.to_bits(),
        rgba.w.to_bits(),
        uv.x.to_bits(),
        uv.y.to_bits(),
        pass.shader_time.to_bits(),
        pass.render_flags,
        light.x.to_bits(),
        light.y.to_bits(),
        light.z.to_bits(),
        pass.shadow_plane.to_bits(),
        pass.non_normalized_axes,
        skin.as_deref().unwrap_or("\u{0}"),
    )
}

/// Collapse equal passes across the parts contributing to `part`
/// (`bodyMaterials`).
pub fn body_materials(body: &ComponentBody, part: QvmBodyPart) -> Vec<&BodyMaterial> {
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    for source in &body.parts {
        if part != QvmBodyPart::Body && source.part != QvmBodyPart::Body && source.part != part {
            continue;
        }
        let mut occurrences: HashMap<String, usize> = HashMap::new();
        for pass in &source.passes {
            let key = material_key(pass);
            let ordinal = occurrences.get(&key).copied().unwrap_or(0);
            occurrences.insert(key.clone(), ordinal + 1);
            let occurrence = format!("{key}/{ordinal}");
            if seen.insert(occurrence) {
                result.push(pass);
            }
        }
    }
    result
}

/// Whether every body of an actor shows its base pass for `part`
/// (`bodyBaseVisible`).
pub fn body_base_visible(bodies: &[ComponentBody], actor: &ActorId, part: QvmBodyPart) -> bool {
    bodies.iter().filter(|body| &body.actor == actor).all(|body| {
        !body.parts.is_empty()
            && body
                .parts
                .iter()
                .filter(|source| part == QvmBodyPart::Body || source.part == QvmBodyPart::Body || source.part == part)
                .all(|source| source.base)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::{IdentityOwner, ProviderId};
    use qa_core::math::{vec2, vec3, vec4};

    fn owner() -> (PresentationOwner, ActorId, IdentityOwner) {
        let authority = IdentityOwner::create("component-bodies").unwrap();
        let actor = authority.actor(1, 0);
        let owner = PresentationOwner {
            provider: ProviderId::new("test", "bodies"),
            generation: 1,
        };
        (owner, actor, authority)
    }

    fn pass(shader: Option<&str>, rgba_x: f32) -> BodyMaterial {
        BodyMaterial {
            custom_shader: shader.map(SceneShader::new),
            custom_skin: None,
            shader_rgba: vec4(rgba_x, 0.0, 0.0, 1.0),
            shader_tex_coord: vec2(0.0, 0.0),
            shader_time: 0.0,
            render_flags: 0,
            lighting_origin: vec3(0.0, 0.0, 0.0),
            shadow_plane: 0.0,
            non_normalized_axes: false,
        }
    }

    fn body() -> (ComponentBody, ActorId) {
        let (owner, actor, _) = owner();
        let body = ComponentBody {
            owner,
            actor: actor.clone(),
            content: ContentId("q3:classic:baseq3:1".to_string()),
            time: 1.0,
            parts: vec![
                ComponentBodyPart {
                    part: QvmBodyPart::Body,
                    base: true,
                    passes: vec![pass(Some("body"), 1.0), pass(Some("body"), 1.0)],
                },
                ComponentBodyPart {
                    part: QvmBodyPart::Head,
                    base: false,
                    passes: vec![pass(Some("body"), 1.0), pass(Some("head"), 2.0)],
                },
            ],
        };
        (body, actor)
    }

    #[test]
    fn equal_passes_collapse_across_parts() {
        let (body, _) = body();
        // The body's first pass and the head's first pass share key and
        // ordinal, so only one survives; the body's repeated pass has a
        // distinct ordinal and remains.
        let materials = body_materials(&body, QvmBodyPart::Body);
        assert_eq!(materials.len(), 3);
        assert_eq!(materials[0].custom_shader.as_ref().unwrap().name, "body");
        assert_eq!(materials[2].custom_shader.as_ref().unwrap().name, "head");
    }

    #[test]
    fn part_filtering_includes_body_passes() {
        let (body, _) = body();
        let head = body_materials(&body, QvmBodyPart::Head);
        assert_eq!(head.len(), 3);
        let upper = body_materials(&body, QvmBodyPart::Upper);
        // No upper passes exist, but the shared body helper still applies.
        assert_eq!(upper.len(), 2);
    }

    #[test]
    fn base_visibility_requires_every_filtered_helper() {
        let (body, actor) = body();
        // The body view includes every helper, so the head's missing base
        // fails it; the upper view only sees the shared body helper.
        assert!(!body_base_visible(
            std::slice::from_ref(&body),
            &actor,
            QvmBodyPart::Body
        ));
        assert!(!body_base_visible(
            std::slice::from_ref(&body),
            &actor,
            QvmBodyPart::Head
        ));
        assert!(body_base_visible(
            std::slice::from_ref(&body),
            &actor,
            QvmBodyPart::Upper
        ));
        let foreign = IdentityOwner::create("foreign").unwrap().actor(1, 0);
        assert!(body_base_visible(
            std::slice::from_ref(&body),
            &foreign,
            QvmBodyPart::Body
        ));
    }
}
