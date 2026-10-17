use super::*;
use qa_app::{
    model_assets::ModelAssets,
    renderer::{
        AssetPreparer, Context, Renderer, SceneComposer,
        hud::{HudStats, HudView},
    },
};
use qa_core::{
    configstrings::{ConfigLayout, ConfigStrings},
    primitives::{EntityId, HudLine, HudState, ThinkTime},
};
use qa_render::{
    Refdef, Viewport,
    cpu::{CpuLimits, RasterBands},
};
use qa_ui::values::ValueLayout;
use std::sync::{Arc, LazyLock};

static LOCALIZATION: LazyLock<qa_content::localization::Catalog> = LazyLock::new(Default::default);

fn tiny_limits() -> Limits {
    Limits {
        commands: 4,
        entities: 1,
        model_choices: 0,
        model_joints: 0,
        polys: 1,
        vertices: 3,
        lights: 1,
        particles: 1,
        area_bytes: 1,
        surfaces: 1,
        draws: 3,
    }
}

fn draw_hud(
    scene: &mut SceneComposer,
    owner: &ModelAssets,
    cpu: &mut CpuBackend,
    rules: RuleSetId,
    limits: Limits,
) -> HudStats {
    let values = ValueLayout::load(&[]).unwrap();
    let mut hud = HudState::with_capacity(0, 0, 0, values.capacity());
    let mut texts = TextStore::load(1, 16).unwrap();
    hud.notify[0] = Some(HudLine {
        text: texts.insert(b"A\n").unwrap(),
        started_at: 0.9,
        until: 2.0,
    });
    // Wire/value provenance remains independent of the selected HUD artwork.
    let layout = ConfigLayout::QUAKE2;
    let configs =
        ConfigStrings::load((0..layout.count).map(|index| layout.capacity(index).unwrap()))
            .unwrap();
    let mut front = FrontEnd::load(limits).unwrap();
    let mut frame = front.begin_frame([3, 5, 7, 255]).unwrap();
    let stats = scene.append_hud(
        &mut frame,
        owner.assets(),
        HudView {
            hud: &hud,
            texts: &texts,
            configs: &configs,
            layout,
            values: &values,
            localization: &LOCALIZATION,
            rules,
            time: EventTime::from_milliseconds(1000),
            style_time: ThinkTime::Milliseconds(100),
            server_frame: 0,
            client: qa_core::primitives::ClientId(0),
            client_lifetime: EntityId {
                slot: 0,
                generation: 1,
            },
            style_interpolation: 0,
            show_text: true,
        },
        Refdef {
            viewport: Viewport {
                x: 24,
                y: 16,
                width: 80,
                height: 64,
            },
            ..Default::default()
        },
        &(),
        qa_render::shader::registration::ShaderOwner::Engine,
    );
    let packet = frame.finish();
    assert_eq!(packet.rejected, 0);
    assert_eq!(cpu.render(&packet, owner.assets()).rejected, 0);
    assert_eq!(cpu.pixels()[0], u32::from_le_bytes([3, 5, 7, 255]));
    assert_eq!(
        cpu.pixels()[119 * 160 + 159],
        u32::from_le_bytes([3, 5, 7, 255])
    );
    stats
}

#[test]
fn late_hud_backend_rejection_restores_palette_and_frontend_before_retry() {
    let root = Fixture::new();
    root.native(RuleSetId::Quake);
    let mut rgb = vec![0; 768];
    rgb[3..6].fill(255);
    root.write("gfx/palette.lmp", &rgb);
    let mut shades: Vec<_> = (0..64).flat_map(|_| 0..=255u8).collect();
    shades.push(0);
    root.write("gfx/colormap.lmp", &shades);
    let vfs = root.vfs();
    let mut owner =
        ModelAssets::new(&vfs, Assets::load().unwrap(), [], [], Default::default()).unwrap();
    let (mut scene, initial_limits) =
        SceneComposer::load(tiny_limits(), owner.assets_mut(), vec![]).unwrap();
    let mut cpu = CpuBackend::load_with_limits(
        160,
        120,
        owner.assets(),
        CpuLimits {
            scene: initial_limits,
            cache_bytes: 4096,
            ..Default::default()
        },
    )
    .unwrap();
    let initial = cpu.admitted_assets().unwrap();
    owner.publish_admission(initial).unwrap();
    owner.end_initial_registration();
    assert!(scene.asset_stage_ready());
    assert!(owner.palettes().get(RuleSetId::Quake).is_none());
    let rejected = owner.prepare_hud(
        &vfs,
        RuleSetId::Quake,
        ImageSettings::native(RuleSetId::Quake),
        qa_render::material::ShaderCompilePolicy::default(),
        160,
        None,
        |_, candidate| scene.stage_hud(candidate),
        |assets, staged| {
            assert!(assets.counts().palettes > initial.palettes);
            let mut invalid = staged.limits();
            invalid.surfaces = 0;
            cpu.prepare_assets(assets, Some(invalid))
                .map_err(str::to_owned)
        },
    );
    assert!(rejected.is_err());
    drop(rejected);
    assert!(!scene.has_hud(RuleSetId::Quake));
    assert!(scene.asset_stage_ready());
    assert_eq!(owner.assets().counts(), initial);
    assert_eq!(cpu.admitted_assets(), Some(initial));
    assert!(owner.palettes().get(RuleSetId::Quake).is_none());

    let (staged, counts) = owner
        .prepare_hud(
            &vfs,
            RuleSetId::Quake,
            ImageSettings::native(RuleSetId::Quake),
            qa_render::material::ShaderCompilePolicy::default(),
            160,
            None,
            |_, candidate| scene.stage_hud(candidate),
            |assets, staged| {
                cpu.prepare_assets(assets, Some(staged.limits()))
                    .map_err(str::to_owned)
            },
        )
        .unwrap();
    let limits = staged.limits();
    assert_eq!(
        limits.commands,
        initial_limits.commands + 2049 * SeatId::COUNT
    );
    assert_eq!(
        limits.vertices,
        initial_limits.vertices + 2049 * SeatId::COUNT * 6
    );
    staged.commit();
    assert_eq!(cpu.admitted_assets(), Some(counts));
    assert_eq!(
        owner.palettes().get(RuleSetId::Quake),
        owner.palettes().get(RuleSetId::QuakeWorld)
    );
    let stats = draw_hud(&mut scene, &owner, &mut cpu, RuleSetId::Quake, limits);
    assert_eq!(stats.glyphs, 1);
    assert_eq!(stats.missing_font, 0);
    assert_eq!(stats.rejected, 0);
    assert_eq!(cpu.pixels()[16 * 160 + 32], u32::from_le_bytes([255; 4]));
    assert_eq!(
        cpu.pixels()[16 * 160 + 33],
        u32::from_le_bytes([3, 5, 7, 255])
    );
}

#[test]
fn initial_renderer_reserves_all_hud_roles_and_lends_the_console_font() {
    let root = Fixture::new();
    root.native(RuleSetId::Quake);
    root.native(RuleSetId::Quake3);
    let vfs = root.vfs();
    let mut assets = Assets::load().unwrap();
    let palette = palette(&mut assets);
    let mut owner = ModelAssets::new(&vfs, assets, [], [], Default::default()).unwrap();
    owner
        .adopt_palettes(
            qa_app::render_settings::PaletteRegistrations::from_registered([
                Some(palette),
                Some(palette),
                None,
                None,
                None,
            ]),
        )
        .unwrap();
    let console = ConsoleAssets::load(
        &vfs,
        &mut owner,
        RuleSetId::Quake,
        ImageSettings::native(RuleSetId::Quake),
        qa_render::material::ShaderCompilePolicy::default(),
        160,
        120,
    )
    .unwrap();
    let font_material = console.font_assets().registration.unwrap().shader;
    let effects = qa_render::effects::Effects::load(
        Arc::new(qa_core::effects::EffectCatalog::new(
            Box::default(),
            [None; 5],
        )),
        1,
        SeatId::COUNT,
        &[None],
    )
    .unwrap();
    let configuration = Arc::new(qa_core::presentation::RendererConfiguration {
        display: qa_core::presentation::DisplayConfiguration {
            width: 160,
            height: 120,
            ..Default::default()
        },
        ..Default::default()
    });
    let model_choices = owner.catalog().max_choice_nodes();
    let (mut renderer, mut scene) = Renderer::load(
        Context::Cpu(configuration),
        160,
        120,
        owner.assets_mut(),
        vec![],
        Some(RasterBands::One),
        console,
        std::array::from_fn(|_| None),
        effects,
        model_choices,
    )
    .unwrap();
    owner
        .publish_admission(renderer.admitted_assets().unwrap())
        .unwrap();
    owner.end_initial_registration();
    let before = owner.assets().counts();
    let reuse = scene.hud_font(RuleSetId::Quake).unwrap();
    assert_eq!(reuse.registration.unwrap().shader, font_material);
    assert!(scene.hud_font(RuleSetId::Quake3).is_none());
    vfs.take_lookup_count();
    let (staged, _) = owner
        .prepare_hud(
            &vfs,
            RuleSetId::Quake,
            ImageSettings::native(RuleSetId::Quake),
            qa_render::material::ShaderCompilePolicy::default(),
            160,
            Some(reuse),
            |_, candidate| scene.stage_hud(candidate),
            |assets, staged| renderer.prepare_assets(assets, Some(staged.limits())),
        )
        .unwrap();
    let reserved = staged.limits();
    staged.commit();
    assert_eq!(vfs.take_lookup_count(), 0);
    assert_eq!(owner.assets().counts(), before);
    let (staged, _) = owner
        .prepare_hud(
            &vfs,
            RuleSetId::Quake3,
            ImageSettings::native(RuleSetId::Quake3),
            qa_render::material::ShaderCompilePolicy::default(),
            160,
            None,
            |_, candidate| scene.stage_hud(candidate),
            |assets, staged| renderer.prepare_assets(assets, Some(staged.limits())),
        )
        .unwrap();
    assert_eq!(staged.limits(), reserved);
    staged.commit();
    assert!(scene.has_hud(RuleSetId::Quake));
    assert!(scene.has_hud(RuleSetId::Quake3));
    let mut cpu = CpuBackend::load_with_assets(160, 120, owner.assets()).unwrap();
    assert_eq!(
        draw_hud(&mut scene, &owner, &mut cpu, RuleSetId::Quake, reserved).glyphs,
        1
    );
    assert_eq!(cpu.pixels()[16 * 160 + 32], u32::from_le_bytes([255; 4]));
}

#[test]
fn optional_missing_font_commits_once_and_busy_lists_prevent_cold_work() {
    let root = Fixture::new();
    let vfs = root.vfs();
    let mut owner =
        ModelAssets::new(&vfs, Assets::load().unwrap(), [], [], Default::default()).unwrap();
    let (mut scene, limits) =
        SceneComposer::load(tiny_limits(), owner.assets_mut(), vec![]).unwrap();
    let mut cpu = CpuBackend::load_with_limits(
        160,
        120,
        owner.assets(),
        CpuLimits {
            scene: limits,
            cache_bytes: 4096,
            ..Default::default()
        },
    )
    .unwrap();
    owner
        .publish_admission(cpu.admitted_assets().unwrap())
        .unwrap();
    owner.end_initial_registration();
    assert!(scene.begin_frame());
    assert!(!scene.asset_stage_ready());
    let packet = scene.finish_frame().unwrap();
    assert!(!scene.asset_stage_ready());
    assert!(scene.recycle(packet).is_ok());
    assert!(scene.asset_stage_ready());
    let before = owner.assets().counts();
    let ((staged, profile), _) = owner
        .prepare_hud(
            &vfs,
            RuleSetId::Quake3,
            ImageSettings::native(RuleSetId::Quake3),
            qa_render::material::ShaderCompilePolicy::default(),
            160,
            None,
            |_, candidate| {
                let profile = candidate.profile();
                scene.stage_hud(candidate).map(|staged| (staged, profile))
            },
            |assets, (staged, _)| {
                cpu.prepare_assets(assets, Some(staged.limits()))
                    .map_err(str::to_owned)
            },
        )
        .unwrap();
    let limits = staged.limits();
    staged.commit();
    // A genuine Q3 profile admits its global fallback and the named missing-font body.
    let mut expected = before;
    expected.images += 1;
    expected.materials += 2;
    expected.shader_bindings += 2;
    assert_eq!(owner.assets().counts(), expected);
    assert_eq!(cpu.admitted_assets(), Some(expected));
    assert!(scene.has_hud(RuleSetId::Quake3));
    let receipt = *owner
        .catalog()
        .shader_outcome(
            b"gfx/2d/bigchars",
            profile,
            qa_compat::services::ShaderLighting::TwoDimensional,
        )
        .unwrap();
    assert_eq!(
        receipt.defaulted,
        Some(qa_render::NativeDefaultKind::MissingImage)
    );
    let fallback = owner.catalog().native_default(profile).unwrap();
    assert_ne!(receipt.shader, fallback);
    let body = owner.assets().shader_binding(receipt.shader).unwrap();
    assert!(
        owner
            .assets()
            .material(body.material)
            .unwrap()
            .stages
            .is_empty()
    );
    let fallback_body = owner.assets().shader_binding(fallback).unwrap();
    assert_eq!(
        owner
            .assets()
            .material(fallback_body.material)
            .unwrap()
            .stages
            .len(),
        1
    );
    assert!(vfs.take_lookup_count() > 0);
    let stats = draw_hud(&mut scene, &owner, &mut cpu, RuleSetId::Quake3, limits);
    assert_eq!(stats.glyphs, 0);
    assert_eq!(stats.missing_font, 1);
    assert_eq!(stats.rejected, 0);
    assert_eq!(vfs.take_lookup_count(), 0);
    assert!(
        cpu.pixels()
            .iter()
            .all(|&pixel| pixel == u32::from_le_bytes([3, 5, 7, 255]))
    );
    let cached = qa_app::renderer::console::FontAssets::load(
        &vfs,
        &mut owner,
        RuleSetId::Quake3,
        ImageSettings::native(RuleSetId::Quake3),
        qa_render::material::ShaderCompilePolicy::default(),
    )
    .unwrap();
    assert_eq!(cached.profile, profile);
    assert_eq!(cached.registration, Some(receipt));
    assert!(!cached.native);
    assert_eq!(owner.assets().counts(), expected);
    assert_eq!(cpu.admitted_assets(), Some(expected));
    assert_eq!(vfs.take_lookup_count(), 0);
    let repeated = draw_hud(&mut scene, &owner, &mut cpu, RuleSetId::Quake3, limits);
    assert_eq!(
        (repeated.glyphs, repeated.missing_font, repeated.rejected),
        (0, 1, 0)
    );
    assert!(
        cpu.pixels()
            .iter()
            .all(|&pixel| pixel == u32::from_le_bytes([3, 5, 7, 255]))
    );
    assert_eq!(vfs.take_lookup_count(), 0);
}
