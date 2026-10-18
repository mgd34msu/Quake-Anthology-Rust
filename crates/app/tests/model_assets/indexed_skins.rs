use super::*;
use qa_app::model_assets::{IndexedSkinError, SkinSelection};
use qa_render::cpu::{CpuBackend, CpuLimits};
use qa_render::material::resources::{
    ImageRole, ImageUse, IndexedSkinLayout, IndexedSkinTranslation,
};

fn image(owner: &ModelAssets, selection: SkinSelection) -> &qa_render::assets::Image {
    let shader = owner
        .catalog()
        .indexed_skin(selection, Default::default())
        .unwrap()
        .source(&())
        .unwrap();
    let binding = owner.assets().shader_binding(shader).unwrap();
    let StageTexture::Image(image) =
        owner.assets().material(binding.material).unwrap().stages[0].texture
    else {
        panic!("indexed player texture")
    };
    owner.assets().image(image).unwrap()
}

#[test]
fn indexed_player_variants_share_source_and_publish_exact_dual_color_bodies() {
    let fixture = support::Fixture::new(&[
        ("progs/player.mdl", support::mdl()),
        ("skins/test.pcx", support::pcx_dimensions(320, 200, 16)),
        ("skins/bad.pcx", b"invalid PCX".to_vec()),
    ]);
    let mut assets = Assets::load().unwrap();
    let palette = support::palette(&mut assets);
    let request = ModelRequest::native("progs/player.mdl", RuleSetId::QuakeWorld, Some(palette));
    let profile = request.profile();
    let mut owner = load(&fixture, assets, &[request], Default::default()).unwrap();
    let mut cpu = CpuBackend::load_with_limits(
        64,
        48,
        owner.assets(),
        CpuLimits {
            cache_bytes: 1_048_576,
            ..Default::default()
        },
    )
    .unwrap();
    owner
        .publish_admission(cpu.admitted_assets().unwrap())
        .unwrap();
    owner.end_initial_registration();
    let registration = owner
        .catalog()
        .registration(request.path, profile, None)
        .unwrap();
    assert_eq!(
        owner.catalog().native_skin(registration),
        Some(([2, 1], palette))
    );
    let layout = IndexedSkinLayout {
        storage: Some([320, 200]),
        padding: 0,
        omit_last_row: true,
    };
    let translation = IndexedSkinTranslation {
        palette,
        target: Some(palette),
        indexed_colors: Some([13, 2]),
        rgba_colors: Some([0, 2]),
        crop: Some([0, 0, 296, 194]),
        coordinate_extent: [2, 1],
        upload_extent: Some([128, 64]),
        usage: ImageUse {
            role: ImageRole::Skin,
            ..Default::default()
        },
    };
    let first = owner
        .prepare_indexed_skin(
            &fixture.vfs,
            "skins/test.pcx",
            profile,
            false,
            layout,
            translation,
            None,
            |assets, _| cpu.prepare_assets(assets, None).map_err(str::to_owned),
        )
        .unwrap();
    let first_image = image(&owner, first);
    assert_eq!([first_image.width, first_image.height], [296, 194]);
    assert_eq!(first_image.logical_size(), [320, 200]);
    assert_eq!(&first_image.rgba[..4], &[0, 255, 0, 255]);
    let indexed = first_image.indexed.as_ref().unwrap();
    assert_eq!(indexed.coordinate_extent(), [2, 1]);
    assert_eq!(indexed.mip(0).unwrap().indices()[0], 223);
    assert_eq!(indexed.mip(0).unwrap().indices()[199 * 320], 0);
    let prepared = &first_image.prepared.as_ref().unwrap().levels[0];
    assert_eq!([prepared.width, prepared.height], [128, 64]);
    assert_eq!(
        owner
            .catalog()
            .skin_handle(b"skins/test.pcx", RuleSetId::QuakeWorld, None),
        None
    );
    let old_rgba = first_image.rgba.clone();
    let old_counts = owner.assets().counts();
    let old_capacity = owner.catalog().capacity();
    fixture.remove("skins/test.pcx");
    fixture.vfs.take_lookup_count();
    assert_eq!(
        owner
            .prepare_indexed_skin(
                &fixture.vfs,
                "SKINS/TEST.PCX",
                profile,
                false,
                layout,
                translation,
                None,
                |_, _| panic!("cached indexed body prepared twice")
            )
            .unwrap(),
        first
    );
    assert_eq!(fixture.vfs.take_lookup_count(), 0);
    let second_request = IndexedSkinTranslation {
        indexed_colors: Some([3, 4]),
        rgba_colors: Some([3, 4]),
        ..translation
    };
    assert!(matches!(
        owner.prepare_indexed_skin(
            &fixture.vfs,
            "skins/test.pcx",
            profile,
            false,
            layout,
            second_request,
            None,
            |_, _| Err("backend rejection".to_owned())
        ),
        Err(IndexedSkinError::Rejected(_))
    ));
    assert_eq!(owner.assets().counts(), old_counts);
    assert_eq!(owner.catalog().capacity(), old_capacity);
    assert_eq!(image(&owner, first).rgba, old_rgba);
    let second = owner
        .prepare_indexed_skin(
            &fixture.vfs,
            "skins/test.pcx",
            profile,
            false,
            layout,
            second_request,
            None,
            |assets, _| cpu.prepare_assets(assets, None).map_err(str::to_owned),
        )
        .unwrap();
    assert_ne!(second, first);
    assert_eq!(&image(&owner, second).rgba[..4], &[48, 207, 24, 255]);
    assert_eq!(image(&owner, first).rgba, old_rgba);
    assert_eq!(fixture.vfs.take_lookup_count(), 0);
    assert!(matches!(
        owner.prepare_indexed_skin(
            &fixture.vfs,
            "skins/missing.pcx",
            profile,
            false,
            layout,
            translation,
            None,
            |_, _| panic!("missing skin reached backend")
        ),
        Err(IndexedSkinError::Missing)
    ));
    assert!(matches!(
        owner.prepare_indexed_skin(
            &fixture.vfs,
            "skins/bad.pcx",
            profile,
            false,
            layout,
            translation,
            None,
            |_, _| panic!("invalid skin reached backend")
        ),
        Err(IndexedSkinError::InvalidSource(_))
    ));
    fixture.vfs.take_lookup_count();
    assert!(matches!(
        owner.prepare_indexed_skin(
            &fixture.vfs,
            "skins/missing.pcx",
            profile,
            false,
            layout,
            translation,
            None,
            |_, _| panic!()
        ),
        Err(IndexedSkinError::Missing)
    ));
    assert!(matches!(
        owner.prepare_indexed_skin(
            &fixture.vfs,
            "skins/bad.pcx",
            profile,
            false,
            layout,
            translation,
            None,
            |_, _| panic!()
        ),
        Err(IndexedSkinError::InvalidSource(_))
    ));
    assert_eq!(fixture.vfs.take_lookup_count(), 0);
}
