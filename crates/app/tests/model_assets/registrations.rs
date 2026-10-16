use super::*;
use qa_compat::services::{RendererRegistration, ShaderLighting};

#[test]
fn native_membership_keeps_all_model_shaders_and_skin_declaration_order() {
    let fixture = support::Fixture::surfaces();
    let mut assets = Assets::load().unwrap();
    // The renderer and native model catalog have distinct existing name arenas.
    assets
        .intern_resource_name(b"renderer-only-prefix")
        .unwrap();
    let profile = shader_profile(RuleSetId::Quake3);
    let mut owner = ModelAssets::new(assets, [profile], [], Default::default()).unwrap();
    let model = owner
        .register_model(&fixture.vfs, b"models/test.md3", profile.source_rules)
        .unwrap();
    let skin = owner
        .register_skin(&fixture.vfs, b"models/test.skin", profile.source_rules)
        .unwrap();
    let model_registrations = owner
        .catalog()
        .renderer_registrations(RendererRegistration::Model, model, profile)
        .unwrap();
    assert_eq!(
        model_registrations
            .iter()
            .map(|entry| owner.assets().name(entry.key).unwrap())
            .collect::<Vec<_>>(),
        ["textures/body", "textures/alternate", "textures/glow"]
    );
    let registration = owner
        .catalog()
        .renderer_registration(model, profile)
        .unwrap();
    assert_eq!(
        owner
            .catalog()
            .bindings(registration, 0, Default::default())
            .unwrap()
            .len(),
        2
    );
    let skin_registrations = owner
        .catalog()
        .renderer_registrations(RendererRegistration::Skin, skin, profile)
        .unwrap();
    // Two declarations address BODY. The mapping's sorted/deduplicated draw
    // table must not erase the later shader's original registration request.
    assert_eq!(
        skin_registrations
            .iter()
            .map(|entry| owner.assets().name(entry.key).unwrap())
            .collect::<Vec<_>>(),
        ["textures/alternate", "textures/glow", "textures/body"]
    );
    assert_eq!(skin_registrations[0], model_registrations[1]);
    assert_eq!(skin_registrations[1], model_registrations[2]);
    assert_eq!(skin_registrations[2], model_registrations[0]);
}

#[test]
fn ordinary_registration_keys_keep_raw_separator_identity_and_fold_only_ascii_case() {
    let fixture =
        support::Fixture::new(&[("ui/pic.tga", support::tga(1, 1, &[[31, 43, 59, 255]]))]);
    let profile = shader_profile(RuleSetId::Quake3);
    let mut owner =
        ModelAssets::new(Assets::load().unwrap(), [profile], [], Default::default()).unwrap();
    let kind = RendererRegistration::Shader {
        mipmaps: false,
        lighting: ShaderLighting::TwoDimensional,
    };
    let mut entries = Vec::new();
    for raw in [b"ui/pic.tga".as_slice(), b"ui\\pic.tga", b"UI/PIC.jpg"] {
        let handle = owner
            .register_shader(
                &fixture.vfs,
                raw,
                profile.source_rules,
                false,
                ShaderLighting::TwoDimensional,
            )
            .unwrap();
        entries.push(
            owner
                .catalog()
                .renderer_registrations(kind, handle, profile)
                .unwrap()[0],
        );
    }
    assert_eq!(entries[0], entries[2]);
    assert_ne!(entries[0].key, entries[1].key);
    assert_eq!(owner.assets().name(entries[0].key), Some("ui/pic"));
    assert_eq!(owner.assets().name(entries[1].key), Some("ui\\pic"));
    // Path-canonical compiled values may be shared; logical native names remain
    // distinct even when the same shader instantiation serves both requests.
    assert_eq!(entries[0].shader, entries[1].shader);
}
