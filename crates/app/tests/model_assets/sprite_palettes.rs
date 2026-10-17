use super::*;
use qa_app::model_assets::RegistrationVariant;
use qa_render::cpu::{CpuBackend, CpuLimits};
use qa_render::shader::reference::ShaderView;

fn native_files(shift: u8) -> Vec<(&'static str, Vec<u8>)> {
    let rgb = (0..=255u8)
        .flat_map(|index| {
            let color = index.wrapping_add(shift);
            [color, 255 - color, color / 2]
        })
        .collect();
    let mut shades = (0..64).flat_map(|_| 0..=255u8).collect::<Vec<_>>();
    shades.push(32);
    vec![
        ("gfx/palette.lmp", rgb),
        ("gfx/colormap.lmp", shades),
        ("progs/test.mdl", support::mdl()),
        ("progs/test.spr", support::spr()),
    ]
}

fn first_image<'a>(
    owner: &'a ModelAssets,
    path: &str,
    profile: ModelProfile,
) -> &'a qa_render::assets::Image {
    let registration = owner.catalog().registration(path, profile).unwrap();
    let choice = owner
        .catalog()
        .bindings(registration, 0, Default::default())
        .unwrap()
        .choices()
        .next()
        .unwrap();
    let source = choice.shaders().next().unwrap();
    let shader = choice
        .resolve_shader(source, ShaderView::direct(), None)
        .unwrap()
        .source(&())
        .unwrap();
    let material = owner.assets().shader_binding(shader).unwrap().material;
    let StageTexture::Image(image) = owner.assets().material(material).unwrap().stages[0].texture
    else {
        panic!("model texture")
    };
    owner.assets().image(image).unwrap()
}

#[test]
fn indexed_models_resolve_native_source_under_q3_without_changing_profile_or_target() {
    for indexed_target in [false, true] {
        let fixture = support::Fixture::new(&native_files(0));
        let mut assets = Assets::load().unwrap();
        let target = indexed_target.then(|| support::shifted_palette(&mut assets, 63));
        let mut profile = shader_profile(RuleSetId::Quake3);
        profile.palette = target;
        let before = assets.counts();
        let requests = ["progs/test.mdl", "progs/test.spr"].map(|path| ModelRequest {
            path,
            source_rules: profile.source_rules,
            rules: profile.rules,
            source_palette: None,
            palette: target,
            image_settings: profile.image_settings,
            finish: profile.finish,
            required: true,
        });
        let owner = load(&fixture, assets, &requests, Default::default()).unwrap();
        assert!(owner.catalog().diagnostics.is_empty());
        assert_eq!(owner.assets().counts().palettes, before.palettes + 1);
        assert!(owner.catalog().profiles().all(|actual| actual == profile));
        let source = owner.palettes().get(RuleSetId::Quake).unwrap();
        assert_eq!(owner.palettes().get(RuleSetId::QuakeWorld), Some(source));
        for (path, authored) in [("progs/test.mdl", 1u8), ("progs/test.spr", 11)] {
            let image = first_image(&owner, path, profile);
            assert_eq!(
                &image.rgba[..4],
                &[authored, 255 - authored, authored / 2, 255]
            );
            let expected = target.map_or(authored, |target| {
                owner.assets().palette(source).unwrap().translation_to(
                    owner.assets().palette(target).unwrap(),
                    (path.ends_with(".spr")).then_some(255),
                )[authored as usize]
            });
            assert_eq!(
                image.indexed.as_ref().unwrap().mip(0).unwrap().indices()[0],
                expected
            );
        }
    }
}

#[test]
fn explicit_indexed_source_wins_without_native_palette_files() {
    let fixture = support::Fixture::new(&[("progs/test.spr", support::spr())]);
    let mut assets = Assets::load().unwrap();
    let source = support::shifted_palette(&mut assets, 101);
    let request = ModelRequest::native("progs/test.spr", RuleSetId::Quake3, Some(source));
    let mut request = request;
    request.palette = None;
    let owner = load(&fixture, assets, &[request], Default::default()).unwrap();
    assert_eq!(owner.assets().counts().palettes, 1);
    assert!(owner.palettes().get(RuleSetId::Quake).is_none());
    assert_eq!(
        &first_image(&owner, request.path, request.profile()).rgba[..4],
        &[112, 143, 56, 255]
    );
}

#[test]
fn rejected_source_palette_and_translation_are_rebuilt_by_the_same_registration() {
    let mut first = support::Fixture::new(&native_files(0));
    let replacement = support::Fixture::new(&native_files(37));
    let mut assets = Assets::load().unwrap();
    let target = support::palette(&mut assets);
    let mut profile = shader_profile(RuleSetId::Quake3);
    profile.palette = Some(target);
    let mut owner = ModelAssets::new(assets, [profile], [], Default::default()).unwrap();
    let limits = CpuLimits {
        cache_bytes: 1_048_576,
        ..Default::default()
    };
    let mut cpu = CpuBackend::load_with_limits(32, 32, owner.assets(), limits).unwrap();
    owner
        .publish_admission(cpu.admitted_assets().unwrap())
        .unwrap();
    owner.end_initial_registration();
    let before = owner.assets().counts();
    let request = renderer_request(
        "progs/test.spr",
        qa_compat::services::RendererRegistration::Model,
        profile.source_rules,
        None,
    );
    let error = owner
        .prepare_registration(
            &first.vfs,
            &request,
            profile,
            RegistrationVariant::Guard(false),
            |_| Ok(()),
            |assets, _| {
                CpuBackend::load_with_limits(32, 32, assets, limits).unwrap();
                Err("reject prepared sprite".to_owned())
            },
        )
        .unwrap_err();
    assert!(error.contains("reject prepared sprite"), "{error}");
    assert_eq!(owner.assets().counts(), before);
    assert!(owner.palettes().get(RuleSetId::Quake).is_none());
    let replacement_root = replacement.vfs.mounts().next().unwrap().path.clone();
    first.vfs.mount_directory(&replacement_root, 10).unwrap();
    owner
        .prepare_registration(
            &first.vfs,
            &request,
            profile,
            RegistrationVariant::Guard(false),
            |_| Ok(()),
            |assets, _| cpu.prepare_assets(assets, None).map_err(str::to_owned),
        )
        .unwrap();
    let image = first_image(&owner, "progs/test.spr", profile);
    assert_eq!(&image.rgba[..4], &[48, 207, 24, 255]);
    assert_eq!(
        image.indexed.as_ref().unwrap().mip(0).unwrap().indices()[0],
        48
    );
    assert_eq!(owner.assets().counts().palettes, before.palettes + 1);
}
