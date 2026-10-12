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
        rules,
        layout,
        time: EventTime::from_milliseconds(1000),
        style_time: ThinkTime::Milliseconds(100),
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
    assert_eq!(stats.unsupported, 1);
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
    let stats = view.apply_lightstyles(&mut refdef);
    assert_eq!(stats.styles, 64);
    assert_eq!(refdef.lightstyles[0].indexed_scale, 264);
    assert_eq!(refdef.lightstyles[0].rgb, [1.03125; 3]);
}
