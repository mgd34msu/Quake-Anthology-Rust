use qa_app::renderer::{
    console::FontAssets,
    hud::{HudAssets, HudView},
    text::BitmapFont,
};
use qa_core::{
    configstrings::{ConfigLayout, ConfigStrings},
    events::TextStore,
    primitives::{HudLine, HudState, RuleSetId, ThinkTime},
    sys_events::EventTime,
};
use qa_render::{Assets, Command, FrontEnd, Limits, MaterialId, Refdef, Viewport};
use qa_ui::values::ValueLayout;
static LOCALIZATION: std::sync::LazyLock<qa_content::localization::Catalog> =
    std::sync::LazyLock::new(Default::default);
#[path = "model_assets/support.rs"]
#[allow(dead_code)]
mod image_support;

fn assets(rules: RuleSetId, width: u32) -> HudAssets {
    HudAssets::from_font(
        rules,
        FontAssets {
            font: Some(BitmapFont {
                material: MaterialId(0),
                height: 8,
            }),
            palette: None,
            native: true,
        },
        width,
    )
    .unwrap()
}
fn configs(layout: ConfigLayout) -> ConfigStrings {
    ConfigStrings::load((0..layout.count).map(|index| layout.capacity(index).unwrap())).unwrap()
}
fn view<'a>(
    hud: &'a HudState,
    texts: &'a TextStore,
    configs: &'a ConfigStrings,
    values: &'a ValueLayout,
    rules: RuleSetId,
    layout: ConfigLayout,
) -> HudView<'a> {
    HudView {
        hud,
        texts,
        configs,
        values,
        localization: &LOCALIZATION,
        rules,
        layout,
        time: EventTime::from_milliseconds(1000),
        style_time: ThinkTime::Milliseconds(100),
        server_frame: 0,
        client_lifetime: qa_core::primitives::EntityId {
            slot: 0,
            generation: 0,
        },
        style_interpolation: 0,
        show_text: true,
    }
}
#[test]
fn display_leases_draw_without_consumption_and_notifications_clip_to_the_seat() {
    let values = ValueLayout::load(&[]).unwrap();
    let mut hud = HudState::with_capacity(0, 0, 0, values.capacity());
    let mut texts = TextStore::load(4, 128).unwrap();
    hud.notify[0] = Some(HudLine {
        text: texts.insert(b"expired\n").unwrap(),
        started_at: 0.0,
        until: 0.5,
    });
    hud.notify[1] = Some(HudLine {
        text: texts.insert(b"live\n").unwrap(),
        started_at: 0.9,
        until: 2.0,
    });
    hud.centerprint = Some(HudLine {
        text: texts.insert(b"center").unwrap(),
        started_at: 0.9,
        until: 2.0,
    });
    let id = hud.notify[1].as_ref().unwrap().text.id();
    let layout = ConfigLayout::QUAKE;
    let configs = configs(layout);
    let render_assets = Assets::load().unwrap();
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    let viewport = Viewport {
        x: 32,
        y: 10,
        width: 64,
        height: 100,
    };
    let stats = assets(RuleSetId::Quake, 128).append(
        &mut frame,
        &render_assets,
        view(&hud, &texts, &configs, &values, RuleSetId::Quake, layout),
        Refdef {
            viewport,
            ..Refdef::default()
        },
    );
    assert_eq!(stats.glyphs, 10);
    assert_eq!(stats.expired, 1);
    assert_eq!(stats.rejected, 0);
    assert_eq!(texts.get(id), Some(b"live\n".as_slice()));
    let packet = frame.finish();
    let draws: Vec<_> = packet
        .commands()
        .iter()
        .filter_map(|command| {
            if let Command::Draw2d(draw) = command {
                Some(draw)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(draws[0].rect, [40.0, 10.0, 8.0, 8.0]);
    assert_eq!(draws[4].rect, [40.0, 45.0, 8.0, 8.0]);
    assert!(
        draws
            .iter()
            .all(|draw| draw.rect[0] >= 32.0 && draw.rect[0] + draw.rect[2] <= 96.0)
    );
}
#[test]
fn native_layout_uses_client_values_configstrings_and_known_operand_alignment() {
    let values = ValueLayout::load(&[]).unwrap();
    let mut hud = HudState::with_capacity(0, 0, 0, values.capacity());
    let native = values.native(RuleSetId::Quake2);
    assert!(native.stats[2].import(&mut hud.values, 100));
    assert!(native.stats[13].import(&mut hud.values, 1));
    let mut texts = TextStore::load(2, 128).unwrap();
    hud.layout_text = Some(texts.insert(b"xl 0 yt 20 string2 \"go\"").unwrap());
    let layout = ConfigLayout::QUAKE2;
    let mut configs = configs(layout);
    configs
        .set(
            layout.statusbar().unwrap(),
            b"xv 0 yt 0 string \"ready\" pic 0 if 1 string \"hidden\" endif stat_string 2",
        )
        .unwrap();
    configs.set(100, b"value").unwrap();
    let render_assets = Assets::load().unwrap();
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    let stats = assets(RuleSetId::Quake2, 320).append(
        &mut frame,
        &render_assets,
        view(&hud, &texts, &configs, &values, RuleSetId::Quake2, layout),
        Refdef {
            viewport: Viewport {
                width: 320,
                height: 200,
                ..Viewport::default()
            },
            ..Refdef::default()
        },
    );
    assert_eq!(stats.glyphs, 12);
    assert_eq!(stats.unsupported, 0);
    assert_eq!(stats.rejected, 0);
    let packet = frame.finish();
    let last = packet
        .commands()
        .iter()
        .filter_map(|command| {
            if let Command::Draw2d(draw) = command {
                Some(draw)
            } else {
                None
            }
        })
        .last()
        .unwrap();
    assert_eq!(last.rect, [8.0, 20.0, 8.0, 8.0]);
    assert_eq!(last.texcoords[1], 14.0 * 0.0625);
}
#[test]
fn received_lightstyles_update_view_scales_without_using_presentation_as_source() {
    let values = ValueLayout::load(&[]).unwrap();
    let hud = HudState::with_capacity(0, 0, 0, values.capacity());
    let texts = TextStore::load(1, 1).unwrap();
    let mut configs = configs(ConfigLayout::QUAKE);
    configs
        .set(ConfigLayout::QUAKE.style_index(0).unwrap(), b"am")
        .unwrap();
    let mut view = view(
        &hud,
        &texts,
        &configs,
        &values,
        RuleSetId::Quake2,
        ConfigLayout::QUAKE,
    );
    view.style_time = ThinkTime::Seconds(0.1);
    let mut refdef = Refdef::default();
    let stats = view.apply_lightstyles(&mut refdef, &mut Default::default());
    assert_eq!(stats.styles, 64);
    assert_eq!(refdef.lightstyles[0].indexed_scale, 264);
    assert_eq!(refdef.lightstyles[0].rgb, [1.03125; 3]);
}

#[test]
fn style_histories_are_client_owned_and_reset_on_lifetime_reuse() {
    let values = ValueLayout::load(&[]).unwrap();
    let hud = HudState::with_capacity(0, 0, 0, values.capacity());
    let texts = TextStore::load(1, 1).unwrap();
    let layout = ConfigLayout::QUAKE2_RERELEASE;
    let mut configs = configs(layout);
    configs.set(layout.style_index(0).unwrap(), b"am").unwrap();
    let mut input = view(
        &hud,
        &texts,
        &configs,
        &values,
        RuleSetId::Quake2Rerelease,
        layout,
    );
    input.style_interpolation = 1;
    let mut first = qa_app::renderer::hud::LightStyles::default();
    let mut second = qa_app::renderer::hud::LightStyles::default();
    let mut refdef = Refdef::default();
    input.style_time = ThinkTime::Milliseconds(0);
    input.apply_lightstyles(&mut refdef, &mut first);
    input.style_time = ThinkTime::Milliseconds(100);
    input.apply_lightstyles(&mut refdef, &mut first);
    input.style_time = ThinkTime::Milliseconds(150);
    let stats = input.apply_lightstyles(&mut refdef, &mut first);
    assert_eq!(refdef.lightstyles[0].rgb, [0.5; 3]);
    assert_eq!(stats.unsupported, 0);
    input.apply_lightstyles(&mut refdef, &mut second);
    assert_eq!(refdef.lightstyles[0].rgb, [1.; 3]);
    input.client_lifetime.generation += 1;
    input.apply_lightstyles(&mut refdef, &mut first);
    assert_eq!(refdef.lightstyles[0].rgb, [1.; 3]);
}

fn picture_assets(rules: RuleSetId) -> (Assets, HudAssets, [[MaterialId; 11]; 2], MaterialId) {
    use qa_render::assets::{MaterialSettings, Stage, StageTexture};
    let mut render = Assets::load().unwrap();
    let mut hud = assets(rules, 320);
    let image = render
        .register_image(16, 24, &vec![255; 16 * 24 * 4])
        .unwrap();
    let numbers = std::array::from_fn(|color| {
        std::array::from_fn(|digit| {
            let prefix = if color == 0 { "num" } else { "anum" };
            let name = if digit == 10 {
                format!("pics/{prefix}_minus.pcx")
            } else {
                format!("pics/{prefix}_{digit}.pcx")
            };
            let material = render
                .register_material(
                    &name,
                    &[Stage {
                        texture: StageTexture::Image(image),
                        ..Stage::default()
                    }],
                    MaterialSettings::default(),
                )
                .unwrap();
            hud.register_picture(&mut render, &name, material).unwrap();
            material
        })
    });
    let icon = render
        .register_material(
            "pics/i_health.pcx",
            &[Stage {
                texture: StageTexture::Image(image),
                ..Stage::default()
            }],
            MaterialSettings::default(),
        )
        .unwrap();
    hud.register_picture(&mut render, "pics/i_health.pcx", icon)
        .unwrap();
    (render, hud, numbers, icon)
}

#[test]
fn native_icons_and_numeric_fields_keep_decimal_truncation_and_seat_clipping() {
    let rules = RuleSetId::Quake2;
    let (render, mut assets, numbers, icon) = picture_assets(rules);
    let values = ValueLayout::load(&[]).unwrap();
    let mut hud = HudState::with_capacity(0, 0, 0, values.capacity());
    let native = values.native(rules);
    for (index, value) in [(0, 1u32), (5, (-12345i32) as u32), (1, 24)] {
        assert!(native.stats[index].import(&mut hud.values, value));
    }
    let layout = ConfigLayout::for_rules(rules);
    let mut configs = configs(layout);
    configs
        .set((layout.resources[2].first + 1) as usize, b"i_health")
        .unwrap();
    configs
        .set(
            layout.statusbar().unwrap(),
            b"xl -8 yt 0 pic 0 xl 16 yt 24 num 3 5 xl 0 yt 50 hnum",
        )
        .unwrap();
    let texts = TextStore::load(1, 1).unwrap();
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    let viewport = Viewport {
        x: 40,
        y: 20,
        width: 320,
        height: 200,
    };
    let mut input = view(&hud, &texts, &configs, &values, rules, layout);
    input.server_frame = 4;
    let stats = assets.append(
        &mut frame,
        &render,
        input,
        Refdef {
            viewport,
            ..Refdef::default()
        },
    );
    assert_eq!(stats.pictures, 6);
    assert_eq!(stats.rejected, 0);
    assert_eq!(stats.unsupported, 0);
    assert_eq!(stats.missing_picture, 0);
    let packet = frame.finish();
    let draws: Vec<_> = packet
        .commands()
        .iter()
        .filter_map(|c| {
            if let Command::Draw2d(d) = c {
                Some(*d)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(draws[0].material, icon);
    assert_eq!(draws[0].rect, [40., 20., 8., 24.]);
    assert_eq!(draws[0].texcoords, [0.5, 0., 1., 1.]);
    assert_eq!(draws[1].rect, [58., 44., 16., 24.]);
    assert_eq!(
        draws[1..4].iter().map(|d| d.material).collect::<Vec<_>>(),
        vec![numbers[0][10], numbers[0][1], numbers[0][2]]
    );
    assert_eq!(draws[4].rect, [58., 70., 16., 24.]);
    assert_eq!(
        draws[4..].iter().map(|d| d.material).collect::<Vec<_>>(),
        vec![numbers[1][2], numbers[1][4]]
    );
}

#[test]
fn hud_flash_clock_armor_zero_and_negative_ammo_keep_each_native_rule() {
    for rules in [RuleSetId::Quake2, RuleSetId::Quake2Rerelease] {
        let (render, mut assets, numbers, _) = picture_assets(rules);
        let values = ValueLayout::load(&[]).unwrap();
        let mut hud = HudState::with_capacity(0, 0, 0, values.capacity());
        let native = values.native(rules);
        for (index, value) in [(1, 24), (3, (-1i32) as u32), (5, 0)] {
            assert!(native.stats[index].import(&mut hud.values, value));
        }
        let layout = ConfigLayout::for_rules(rules);
        let mut configs = configs(layout);
        configs
            .set(
                layout.statusbar().unwrap(),
                b"xl 0 yt 0 hnum xl 50 anum xl 100 rnum",
            )
            .unwrap();
        let texts = TextStore::load(1, 1).unwrap();
        let mut front = FrontEnd::load(Limits::default()).unwrap();
        let mut frame = front.begin_frame([0; 4]).unwrap();
        let stats = assets.append(
            &mut frame,
            &render,
            view(&hud, &texts, &configs, &values, rules, layout),
            Refdef {
                viewport: Viewport {
                    width: 320,
                    height: 200,
                    ..Viewport::default()
                },
                ..Refdef::default()
            },
        );
        let packet = frame.finish();
        let materials: Vec<_> = packet
            .commands()
            .iter()
            .filter_map(|c| {
                if let Command::Draw2d(d) = c {
                    Some(d.material)
                } else {
                    None
                }
            })
            .collect();
        if rules == RuleSetId::Quake2 {
            assert_eq!(materials, vec![numbers[0][2], numbers[0][4]]);
        } else {
            assert_eq!(materials, vec![numbers[1][2], numbers[1][4], numbers[0][0]]);
        }
        assert_eq!(stats.missing_picture, 0);
        assert_eq!(stats.rejected, 0);
    }
}

#[test]
fn hud_picture_registration_survives_removed_files_and_received_config_replacement() {
    let fixture = image_support::Fixture::new(&[
        ("pics/first.pcx", image_support::pcx(11)),
        ("pics/second.pcx", image_support::pcx(22)),
    ]);
    let mut render = Assets::load().unwrap();
    let palette = image_support::palette(&mut render);
    let mut assets = HudAssets::load(
        &fixture.vfs,
        &mut render,
        RuleSetId::Quake2,
        FontAssets {
            font: None,
            palette: Some(palette),
            native: true,
        },
        qa_render::material::resources::ImageSettings::native(RuleSetId::Quake2),
        320,
    )
    .unwrap();
    fixture.remove("pics/first.pcx");
    fixture.remove("pics/second.pcx");
    let values = ValueLayout::load(&[]).unwrap();
    let mut hud = HudState::with_capacity(0, 0, 0, values.capacity());
    assert!(values.native(RuleSetId::Quake2).stats[0].import(&mut hud.values, 1));
    let layout = ConfigLayout::QUAKE2;
    let mut configs = configs(layout);
    configs
        .set(layout.statusbar().unwrap(), b"xl 0 yt 0 pic 0")
        .unwrap();
    let texts = TextStore::load(1, 1).unwrap();
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let mut cpu = qa_render::cpu::CpuBackend::load(320, 200).unwrap();
    let mut handles = Vec::new();
    for (name, index) in [(b"FIRST".as_slice(), 11u8), (b"second".as_slice(), 22u8)] {
        configs
            .set((layout.resources[2].first + 1) as usize, name)
            .unwrap();
        let mut frame = front.begin_frame([0; 4]).unwrap();
        let stats = assets.append(
            &mut frame,
            &render,
            view(&hud, &texts, &configs, &values, RuleSetId::Quake2, layout),
            Refdef {
                viewport: Viewport {
                    width: 320,
                    height: 200,
                    ..Viewport::default()
                },
                ..Refdef::default()
            },
        );
        assert_eq!(stats.pictures, 1);
        assert_eq!(stats.rejected, 0);
        let packet = frame.finish();
        assert_eq!(cpu.render(&packet, &render).rejected, 0);
        assert_eq!(
            cpu.pixels()[0],
            u32::from_le_bytes([index, 255 - index, index / 2, 255])
        );
        handles.push(
            packet
                .commands()
                .iter()
                .find_map(|c| {
                    if let Command::Draw2d(d) = c {
                        Some(d.material)
                    } else {
                        None
                    }
                })
                .unwrap(),
        );
        assert!(front.recycle(packet).is_ok());
    }
    assert_ne!(handles[0], handles[1]);
}

#[test]
fn localized_layout_text_uses_the_shared_catalog_and_native_alignment() {
    let rules = RuleSetId::Quake2Rerelease;
    let localization =
        qa_content::localization::Catalog::load(b"pickup = \"Ammo: {0}\"\nweapon = \"Blaster\"\n")
            .unwrap();
    let values = ValueLayout::load(&[]).unwrap();
    let mut hud = HudState::with_capacity(0, 0, 0, values.capacity());
    assert!(values.native(rules).stats[2].import(&mut hud.values, 100));
    let layout = ConfigLayout::for_rules(rules);
    let mut configs = configs(layout);
    configs.set(100, b"$weapon").unwrap();
    configs.set(layout.statusbar().unwrap(),b"xl 100 yt 0 loc_string 1 \"$pickup\" \"$weapon\" xl 200 yt 16 loc_stat_rstring 2 xl 0 yt 32 loc_cstring2 0 \"$weapon\"").unwrap();
    let texts = TextStore::load(1, 1).unwrap();
    let render = Assets::load().unwrap();
    let mut assets = assets(rules, 320);
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    let mut input = view(&hud, &texts, &configs, &values, rules, layout);
    input.localization = &localization;
    let stats = assets.append(
        &mut frame,
        &render,
        input,
        Refdef {
            viewport: Viewport {
                width: 320,
                height: 200,
                ..Viewport::default()
            },
            ..Refdef::default()
        },
    );
    assert_eq!(stats.glyphs, 26);
    assert_eq!(stats.unsupported, 0);
    assert_eq!(stats.rejected, 0);
    let packet = frame.finish();
    let draws: Vec<_> = packet
        .commands()
        .iter()
        .filter_map(|c| {
            if let Command::Draw2d(d) = c {
                Some(d)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(draws[0].rect, [100., 0., 8., 8.]);
    assert_eq!(draws[12].rect, [144., 16., 8., 8.]);
    assert_eq!(draws[19].rect, [132., 32., 8., 8.]);
    assert_eq!(draws[19].texcoords[1], 0.75);
}
