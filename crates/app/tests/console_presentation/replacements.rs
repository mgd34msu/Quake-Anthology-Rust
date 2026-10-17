use super::*;
use qa_app::renderer::{
    console::FontAssets,
    hud::{HudAssets, HudView},
};
use qa_core::{
    configstrings::{ConfigLayout, ConfigStrings},
    primitives::{ClientId, EntityId, HudState, ThinkTime},
};
use qa_render::{
    Refdef, Viewport,
    assets::{Stage, StageTexture},
    shader::reference::ShaderView,
};
use qa_ui::values::ValueLayout;

fn owner(vfs: &Vfs) -> ModelAssets {
    let mut assets = Assets::load().unwrap();
    let palette = palette(&mut assets);
    registered_assets(
        vfs,
        assets,
        qa_app::render_settings::PaletteRegistrations::from_registered([
            None,
            None,
            Some(palette),
            Some(palette),
            None,
        ]),
    )
}

fn font_stage<'a>(owner: &'a ModelAssets, font: &FontAssets) -> &'a Stage {
    let binding = owner
        .assets()
        .shader_binding(font.registration.unwrap().shader)
        .unwrap();
    &owner.assets().material(binding.material).unwrap().stages[0]
}

#[test]
fn replacement_only_rerelease_font_uses_native_identity_indexed_palette_and_fractional_blend() {
    let root = Fixture::new();
    let mut pixels = vec![0; 128 * 128 * 4];
    for y in (0..128).step_by(8) {
        for x in (0..128).step_by(8) {
            pixels[(y * 128 + x) * 4..(y * 128 + x + 1) * 4].copy_from_slice(&[255, 255, 255, 128]);
        }
    }
    root.tga("pics/conchars.tga", 128, 128, &pixels);
    let vfs = root.vfs();
    assert!(vfs.open(b"pics/conchars.pcx").is_none());
    let mut owner = owner(&vfs);
    let font = FontAssets::load(
        &vfs,
        &mut owner,
        RuleSetId::Quake2Rerelease,
        ImageSettings::native(RuleSetId::Quake2Rerelease),
        Default::default(),
    )
    .unwrap();
    assert!(font.native);
    assert!(font.palette.is_some());
    let registration = font.registration.unwrap();
    assert_eq!(owner.assets().name(registration.key), Some("pics/conchars"));
    let stage = font_stage(&owner, &font);
    assert_eq!(stage.blend, Some(StageBlend::ALPHA));
    let StageTexture::Image(id) = stage.texture else {
        panic!("font image")
    };
    let image = owner.assets().image(id).unwrap();
    assert_eq!(image.logical_size(), [128, 128]);
    assert_eq!(&image.rgba[..8], &[255, 255, 255, 128, 0, 0, 0, 0]);
    let indexed = image.indexed.as_ref().unwrap();
    assert_eq!(indexed.transparent_index(), Some(255));
    assert_eq!(&indexed.mip(0).unwrap().indices()[..2], &[1, 255]);
    let prefix = owner.assets().counts();
    vfs.take_lookup_count();
    let cached = FontAssets::load(
        &vfs,
        &mut owner,
        RuleSetId::Quake2Rerelease,
        ImageSettings::native(RuleSetId::Quake2Rerelease),
        Default::default(),
    )
    .unwrap();
    assert_eq!(cached.registration, font.registration);
    assert_eq!(owner.assets().counts(), prefix);
    assert_eq!(vfs.take_lookup_count(), 0);
}

#[test]
fn rerelease_font_mask_zero_keeps_binary_pcx_despite_present_truecolor_replacement() {
    let root = Fixture::new();
    root.native(RuleSetId::Quake2Rerelease);
    root.tga(
        "pics/conchars.tga",
        256,
        256,
        &[13, 37, 91, 128].repeat(256 * 256),
    );
    let vfs = root.vfs();
    let mut owner = owner(&vfs);
    let settings = ImageSettings {
        override_textures: 2,
        texture_overrides: 0,
        ..ImageSettings::native(RuleSetId::Quake2Rerelease)
    };
    let font = FontAssets::load(
        &vfs,
        &mut owner,
        RuleSetId::Quake2Rerelease,
        settings,
        Default::default(),
    )
    .unwrap();
    assert!(font.native);
    let stage = font_stage(&owner, &font);
    assert_eq!(stage.blend, None);
    let StageTexture::Image(id) = stage.texture else {
        panic!("font image")
    };
    let image = owner.assets().image(id).unwrap();
    assert_eq!([image.width, image.height], [128, 128]);
    assert_eq!(image.logical_size(), [128, 128]);
    assert_eq!(&image.rgba[..4], &[255; 4]);
    assert_eq!(
        &image.indexed.as_ref().unwrap().mip(0).unwrap().indices()[..2],
        &[1, 255]
    );
}

#[test]
fn rerelease_hud_discovers_replacement_only_picture_and_keeps_original_logical_extent() {
    for original in [false, true] {
        let root = Fixture::new();
        root.native(RuleSetId::Quake2Rerelease);
        root.tga("pics/health.tga", 4, 2, &[17, 23, 31, 255].repeat(8));
        if original {
            root.pcx("pics/health.pcx", 2, 1, &[2, 2]);
        }
        let vfs = root.vfs();
        let mut owner = owner(&vfs);
        let font = FontAssets::load(
            &vfs,
            &mut owner,
            RuleSetId::Quake2Rerelease,
            ImageSettings {
                intensity: 1.0,
                ..ImageSettings::native(RuleSetId::Quake2Rerelease)
            },
            Default::default(),
        )
        .unwrap();
        let mut hud =
            HudAssets::load(&vfs, &mut owner, RuleSetId::Quake2Rerelease, font, 160).unwrap();
        let mut cpu = CpuBackend::load_with_assets(160, 120, owner.assets()).unwrap();
        let values = ValueLayout::load(&[]).unwrap();
        let state = HudState::with_capacity(0, 0, 0, values.capacity());
        let texts = TextStore::load(1, 64).unwrap();
        let layout = ConfigLayout::QUAKE2;
        let mut configs =
            ConfigStrings::load((0..layout.count).map(|i| layout.capacity(i).unwrap())).unwrap();
        for name in ["health", "/pics/health.tga", "\\pics\\health.TGA"] {
            configs
                .set(
                    layout.statusbar().unwrap(),
                    format!("xl 4 yt 6 picn {name}").as_bytes(),
                )
                .unwrap();
            let localization = qa_content::localization::Catalog::default();
            let mut front = FrontEnd::load(Limits::default()).unwrap();
            let mut frame = front.begin_frame([3, 5, 7, 255]).unwrap();
            vfs.take_lookup_count();
            let counts = hud.append(
                &mut frame,
                owner.assets(),
                HudView {
                    hud: &state,
                    texts: &texts,
                    configs: &configs,
                    layout,
                    values: &values,
                    localization: &localization,
                    rules: RuleSetId::Quake2Rerelease,
                    time: EventTime(0),
                    style_time: ThinkTime::Milliseconds(0),
                    server_frame: 0,
                    client: ClientId(0),
                    client_lifetime: EntityId {
                        slot: 0,
                        generation: 1,
                    },
                    style_interpolation: 0,
                    show_text: true,
                },
                Refdef {
                    viewport: Viewport {
                        width: 160,
                        height: 120,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                ShaderView::direct(),
            );
            assert_eq!(
                (counts.pictures, counts.missing_picture, counts.rejected),
                (1, 0, 0)
            );
            let packet = frame.finish();
            assert_eq!(packet.rejected, 0);
            let draw = packet
                .commands()
                .iter()
                .find_map(|command| match command {
                    Command::Draw2d(draw) => Some(draw),
                    _ => None,
                })
                .unwrap();
            let size = if original { [2, 1] } else { [4, 2] };
            assert_eq!(draw.rect, [4.0, 6.0, size[0] as f32, size[1] as f32]);
            let binding = owner
                .assets()
                .shader_binding(draw.shader.source(&()).unwrap())
                .unwrap();
            let StageTexture::Image(id) =
                owner.assets().material(binding.material).unwrap().stages[0].texture
            else {
                panic!("HUD picture image")
            };
            let image = owner.assets().image(id).unwrap();
            assert_eq!([image.width, image.height], [4, 2]);
            assert_eq!(image.logical_size(), size);
            assert_eq!(cpu.render(&packet, owner.assets()).rejected, 0);
            assert_eq!(
                cpu.pixels()[6 * 160 + 4],
                u32::from_le_bytes([17, 23, 31, 255])
            );
            assert_eq!(
                cpu.pixels()[6 * 160 + 4 + size[0] as usize],
                u32::from_le_bytes([3, 5, 7, 255])
            );
            assert_eq!(vfs.take_lookup_count(), 0);
        }
    }
}
