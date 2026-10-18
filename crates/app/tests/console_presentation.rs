use qa_app::model_assets::ModelAssets;
use qa_app::renderer::console::ConsoleAssets;
use qa_console::{
    editor::Editor,
    presentation::{ConsoleView, Scrollback},
};
use qa_content::vfs::Vfs;
use qa_core::{
    events::TextStore,
    primitives::RuleSetId,
    sys_events::{EventTime, SeatId},
};
use qa_render::{
    Assets, Command, FrontEnd, Limits,
    cpu::CpuBackend,
    material::resources::ImageSettings,
    shader::{BlendFactor, StageBlend},
    surface_cache::{IndexedLighting, PaletteLighting},
};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

#[path = "console_presentation/late_hud.rs"]
mod late_hud;
#[path = "console_presentation/replacements.rs"]
mod replacements;

#[test]
fn retail_rerelease_png_font_is_shared_and_retains_a_software_palette() {
    let Some(root) = std::env::var_os("QA_Q2_RERELEASE_ROOT") else {
        return;
    };
    let mut vfs = Vfs::default();
    vfs.mount_product(std::path::Path::new(&root), 0).unwrap();
    assert!(vfs.open(b"pics/conchars.png").is_some());
    assert!(vfs.open(b"pics/conchars.pcx").is_none());
    let assets = Assets::load().unwrap();
    let mut owner = registered_assets(
        &vfs,
        assets,
        qa_app::render_settings::PaletteRegistrations::new(&vfs),
    );
    let font = qa_app::renderer::console::FontAssets::load(
        &vfs,
        &mut owner,
        RuleSetId::Quake2Rerelease,
        ImageSettings::native(RuleSetId::Quake2Rerelease),
        qa_render::material::ShaderCompilePolicy::default(),
    )
    .unwrap();
    assert!(font.native);
    let assets = owner.assets();
    let material = assets
        .material(
            assets
                .shader_binding(font.registration.unwrap().shader)
                .unwrap()
                .material,
        )
        .unwrap();
    assert_eq!(material.stages[0].blend, Some(StageBlend::ALPHA));
    let qa_render::assets::StageTexture::Image(id) = material.stages[0].texture else {
        panic!()
    };
    let image = assets.image(id).unwrap();
    assert_eq!([image.width, image.height], [128, 128]);
    assert!(image.indexed.is_some());
    assert!(font.palette.is_some());
}
struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "qa-console-scene-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        for path in ["gfx/2d", "pics", "scripts"] {
            std::fs::create_dir_all(root.join(path)).unwrap();
        }
        Self(root)
    }
    fn write(&self, path: &str, bytes: &[u8]) {
        std::fs::write(self.0.join(path), bytes).unwrap();
    }
    fn vfs(&self) -> Vfs {
        let mut vfs = Vfs::default();
        vfs.mount_directory(&self.0, 0).unwrap();
        vfs
    }
    fn native(&self, rules: RuleSetId) {
        let mut font = vec![
            if matches!(rules, RuleSetId::Quake | RuleSetId::QuakeWorld) {
                0
            } else {
                255
            };
            128 * 128
        ];
        for y in (0..128).step_by(8) {
            for x in (0..128).step_by(8) {
                font[y * 128 + x] = 1;
            }
        }
        match rules {
            RuleSetId::Quake | RuleSetId::QuakeWorld => {
                let mut wad = b"WAD2".to_vec();
                wad.extend_from_slice(&1i32.to_le_bytes());
                wad.extend_from_slice(&(12i32 + font.len() as i32).to_le_bytes());
                wad.extend_from_slice(&font);
                wad.extend_from_slice(&12i32.to_le_bytes());
                wad.extend_from_slice(&(font.len() as i32).to_le_bytes());
                wad.extend_from_slice(&(font.len() as i32).to_le_bytes());
                wad.extend_from_slice(&[66, 0, 0, 0]);
                let mut name = [0; 16];
                name[..8].copy_from_slice(b"conchars");
                wad.extend_from_slice(&name);
                self.write("gfx.wad", &wad);
                let mut pic = 16i32.to_le_bytes().to_vec();
                pic.extend_from_slice(&16i32.to_le_bytes());
                pic.extend_from_slice(&[2; 16 * 16]);
                self.write("gfx/conback.lmp", &pic);
            }
            RuleSetId::Quake2 | RuleSetId::Quake2Rerelease => {
                self.pcx("pics/conchars.pcx", 128, 128, &font);
                self.pcx("pics/conback.pcx", 16, 16, &[2; 16 * 16]);
            }
            RuleSetId::Quake3 => {
                self.tga(
                    "gfx/2d/bigchars.tga",
                    128,
                    128,
                    &font
                        .iter()
                        .flat_map(|&index| if index == 1 { [255; 4] } else { [0; 4] })
                        .collect::<Vec<_>>(),
                );
                self.tga(
                    "gfx/2d/conback.tga",
                    16,
                    16,
                    &[17, 23, 31, 255].repeat(16 * 16),
                );
                self.write("scripts/console.shader", b"gfx/2d/bigchars { nopicmip nomipmaps { map gfx/2d/bigchars.tga blendFunc blend rgbGen vertex } }\nconsole { nopicmip nomipmaps { map gfx/2d/conback.tga } }\n");
            }
        }
    }
    fn pcx(&self, path: &str, width: u16, height: u16, indices: &[u8]) {
        let mut bytes = vec![0; 128];
        bytes[..4].copy_from_slice(&[10, 5, 1, 8]);
        bytes[8..10].copy_from_slice(&(width - 1).to_le_bytes());
        bytes[10..12].copy_from_slice(&(height - 1).to_le_bytes());
        bytes[65] = 1;
        bytes[66..68].copy_from_slice(&width.to_le_bytes());
        for &index in indices {
            if index >= 192 {
                bytes.push(0xc1);
            }
            bytes.push(index);
        }
        bytes.push(12);
        bytes.extend_from_slice(&[0; 768]);
        self.write(path, &bytes);
    }
    fn tga(&self, path: &str, width: u16, height: u16, rgba: &[u8]) {
        let mut bytes = vec![0; 18];
        bytes[2] = 2;
        bytes[12..14].copy_from_slice(&width.to_le_bytes());
        bytes[14..16].copy_from_slice(&height.to_le_bytes());
        bytes[16] = 32;
        bytes[17] = 40;
        for pixel in rgba.chunks_exact(4) {
            bytes.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
        }
        self.write(path, &bytes);
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
fn palette(assets: &mut Assets) -> qa_render::PaletteId {
    let mut rgb = vec![0; 768];
    rgb[3..6].copy_from_slice(&[255; 3]);
    rgb[6..9].copy_from_slice(&[17, 23, 31]);
    let shades: Vec<_> = (0..64).flat_map(|_| 0..=255u8).collect();
    assets
        .register_palette(PaletteLighting::load(&rgb, &shades, None, 256).unwrap())
        .unwrap()
}

fn registered_assets(
    vfs: &Vfs,
    assets: Assets,
    palettes: qa_app::render_settings::PaletteRegistrations,
) -> ModelAssets {
    let mut owner = ModelAssets::new(vfs, assets, [], [], Default::default()).unwrap();
    owner.adopt_palettes(palettes).unwrap();
    owner
}

#[test]
fn native_fonts_and_backgrounds_become_shared_2d_packets_and_cpu_pixels() {
    for rules in RuleSetId::ALL {
        let root = Fixture::new();
        root.native(rules);
        let vfs = root.vfs();
        let mut assets = Assets::load().unwrap();
        let palette = (rules != RuleSetId::Quake3).then(|| palette(&mut assets));
        let palettes = qa_app::render_settings::PaletteRegistrations::from_registered(
            &vfs,
            [palette, palette, palette, palette, None],
        );
        let mut owner = registered_assets(&vfs, assets, palettes);
        let mut console = ConsoleAssets::load(
            &vfs,
            &mut owner,
            rules,
            ImageSettings::native(rules),
            qa_render::material::ShaderCompilePolicy::default(),
            160,
            120,
        )
        .unwrap();
        let assets = owner.assets();
        let mut front = FrontEnd::load(Limits::default()).unwrap();
        let mut frame = front.begin_frame([3, 5, 7, 255]).unwrap();
        let mut editor = Editor::default();
        editor.toggle(SeatId::ALL[1]);
        editor.character(SeatId::ALL[1], 'x').unwrap();
        let mut texts = TextStore::load(2, 64).unwrap();
        let lease = texts.insert(b"\x80A\n").unwrap();
        let mut log = Scrollback::default();
        assert!(log.append(&mut texts, lease.id()));
        texts.release(lease);
        let counts = console.append(
            &mut frame,
            &assets,
            ConsoleView {
                editor: &editor,
                scrollback: &log,
                texts: &texts,
                rules,
                time: EventTime(0),
            },
            160,
            120,
            true,
            qa_render::shader::reference::ShaderView::direct(),
        );
        assert_eq!(counts.rejected, 0, "{rules:?}");
        assert_eq!(counts.backgrounds, 1);
        assert_eq!(counts.missing_font, 0);
        assert_eq!(
            counts.non_native_font,
            u32::from(rules == RuleSetId::Quake2Rerelease)
        );
        assert!(counts.glyphs >= 5);
        let packet = frame.finish();
        let high = packet
            .commands()
            .iter()
            .find_map(|command| match command {
                Command::Draw2d(draw) if draw.texcoords == [0.0, 0.5, 0.0625, 0.5625] => Some(draw),
                _ => None,
            })
            .unwrap();
        let high_position = [high.rect[0] as usize, high.rect[1] as usize];
        let mut cpu = CpuBackend::load_with_assets(160, 120, &assets).unwrap();
        let rendered = cpu.render(&packet, &assets);
        assert_eq!(rendered.rejected, 0, "{rules:?}");
        assert!(rendered.draws_2d >= counts.glyphs + 1);
        assert_eq!(cpu.pixels()[0], u32::from_le_bytes([17, 23, 31, 255]));
        if rules != RuleSetId::Quake3 {
            // Native indexed glyphs retain their exact palette values and holes.
            assert_eq!(
                cpu.pixels()[high_position[1] * 160 + high_position[0]],
                u32::from_le_bytes([255; 4])
            );
            assert_eq!(
                cpu.pixels()[high_position[1] * 160 + high_position[0] + 1],
                u32::from_le_bytes([17, 23, 31, 255])
            );
        }
        assert_eq!(cpu.pixels()[119 * 160], u32::from_le_bytes([3, 5, 7, 255]));
    }
}

#[test]
fn quake_console_and_hud_fonts_keep_registered_indexed_pixels_and_uploads() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let root = Fixture::new();
        root.native(rules);
        let vfs = root.vfs();
        let mut assets = Assets::load().unwrap();
        let palette = palette(&mut assets);
        let palettes = qa_app::render_settings::PaletteRegistrations::from_registered(
            &vfs,
            [
                Some(palette),
                Some(palette),
                Some(palette),
                Some(palette),
                None,
            ],
        );
        let settings = ImageSettings::native(rules);
        let mut owner = registered_assets(&vfs, assets, palettes);
        let console = ConsoleAssets::load(
            &vfs,
            &mut owner,
            rules,
            settings,
            qa_render::material::ShaderCompilePolicy::default(),
            160,
            120,
        )
        .unwrap();
        let hud = qa_app::renderer::console::FontAssets::load(
            &vfs,
            &mut owner,
            rules,
            settings,
            qa_render::material::ShaderCompilePolicy::default(),
        )
        .unwrap();
        let console_font = console.font_assets().registration.unwrap().shader;
        let hud_font = hud.registration.unwrap().shader;
        let assets = owner.assets();
        let image = |shader: qa_render::ShaderBindingId| {
            let material = assets
                .material(assets.shader_binding(shader).unwrap().material)
                .unwrap();
            assert_eq!(
                material.stages[0].sampler.wrap,
                qa_render::assets::Wrap::Clamp
            );
            assert_eq!(
                material.stages[0].sampler.filter,
                qa_render::assets::Filter::Nearest
            );
            assert!(!material.stages[0].sampler.mipmaps);
            let qa_render::assets::StageTexture::Image(image) = material.stages[0].texture else {
                panic!("native font must select a registered image");
            };
            assets.image(image).unwrap()
        };
        let console_image = image(console_font);
        let hud_image = image(hud_font);
        assert_eq!(console_image.rgba, hud_image.rgba);
        assert_eq!(console_image.prepared, hud_image.prepared);
        let texture = console_image.indexed.as_ref().unwrap();
        assert_eq!(
            (
                texture.mip(0).unwrap().width,
                texture.mip(0).unwrap().height
            ),
            (128, 128)
        );
        assert_eq!(&texture.mip(0).unwrap().indices()[..2], &[1, 0]);
        assert_eq!(texture.transparent_index(), Some(0));
        assert_eq!(&console_image.rgba[..8], &[255, 255, 255, 255, 0, 0, 0, 0]);
        assert_eq!(console_image.prepared.as_ref().unwrap().levels.len(), 1);
        let background = assets
            .materials()
            .iter()
            .find(|material| assets.name(material.name) == Some("gfx/conback.lmp"))
            .unwrap();
        assert_eq!(background.stages[0].blend, Some(StageBlend::ALPHA));
        let qa_render::assets::StageTexture::Image(id) = background.stages[0].texture else {
            panic!("native background must select a registered image");
        };
        let background = assets.image(id).unwrap();
        assert_eq!(
            background
                .indexed
                .as_ref()
                .unwrap()
                .mip(0)
                .unwrap()
                .indices(),
            &[2; 16 * 16]
        );
        assert_eq!(
            background.indexed.as_ref().unwrap().transparent_index(),
            None
        );
        assert_eq!(&background.rgba[..4], &[17, 23, 31, 255]);
    }
}

#[test]
fn closed_console_adds_no_commands_and_missing_assets_are_counted() {
    let vfs = Vfs::default();
    let mut owner = registered_assets(
        &vfs,
        Assets::load().unwrap(),
        qa_app::render_settings::PaletteRegistrations::new(&vfs),
    );
    let mut console = ConsoleAssets::load(
        &vfs,
        &mut owner,
        RuleSetId::Quake3,
        ImageSettings::native(RuleSetId::Quake3),
        qa_render::material::ShaderCompilePolicy::default(),
        160,
        120,
    )
    .unwrap();
    let profile = console.profile();
    let mut cpu = CpuBackend::load_with_assets(160, 120, owner.assets()).unwrap();
    owner
        .publish_admission(cpu.admitted_assets().unwrap())
        .unwrap();
    owner.end_initial_registration();
    let fallback = owner.catalog().native_default(profile).unwrap();
    for name in [b"gfx/2d/bigchars".as_slice(), b"white", b"console"] {
        let receipt = owner
            .catalog()
            .shader_outcome(
                name,
                profile,
                qa_compat::services::ShaderLighting::TwoDimensional,
                None,
            )
            .unwrap();
        assert_eq!(
            receipt.defaulted,
            Some(qa_render::NativeDefaultKind::MissingImage)
        );
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
    }
    let admitted = owner.assets().counts();
    vfs.take_lookup_count();
    let cached = qa_app::renderer::console::FontAssets::load(
        &vfs,
        &mut owner,
        RuleSetId::Quake3,
        ImageSettings::native(RuleSetId::Quake3),
        qa_render::material::ShaderCompilePolicy::default(),
    )
    .unwrap();
    assert!(!cached.native);
    assert_eq!(cached.registration, console.font_assets().registration);
    assert_eq!(owner.assets().counts(), admitted);
    assert_eq!(vfs.take_lookup_count(), 0);
    let assets = owner.assets();
    let mut front = FrontEnd::load(Limits::default()).unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    let mut editor = Editor::default();
    let texts = TextStore::load(1, 64).unwrap();
    let log = Scrollback::default();
    let closed = console.append(
        &mut frame,
        &assets,
        ConsoleView {
            editor: &editor,
            scrollback: &log,
            texts: &texts,
            rules: RuleSetId::Quake3,
            time: EventTime(0),
        },
        160,
        120,
        true,
        qa_render::shader::reference::ShaderView::direct(),
    );
    assert_eq!(closed.glyphs, 0);
    assert_eq!(frame.finish().commands().len(), 1);
    let mut frame = front.begin_frame([0; 4]).unwrap();
    editor.toggle(SeatId::FIRST);
    let missing = console.append(
        &mut frame,
        &assets,
        ConsoleView {
            editor: &editor,
            scrollback: &log,
            texts: &texts,
            rules: RuleSetId::Quake3,
            time: EventTime(0),
        },
        160,
        120,
        true,
        qa_render::shader::reference::ShaderView::direct(),
    );
    assert_eq!(missing.glyphs, 0);
    assert_eq!(missing.missing_font, 1);
    assert_eq!(missing.missing_background, 0);
    assert_eq!(missing.backgrounds, 1);
    assert_eq!(missing.rejected, 1);
    let packet = frame.finish();
    let graphics: Vec<_> = packet
        .commands()
        .iter()
        .filter_map(|command| match command {
            Command::Draw2d(draw) => Some(draw),
            _ => None,
        })
        .collect();
    assert_eq!(
        graphics.len(),
        2,
        "fallback background and red console edge only"
    );
    assert!(
        graphics
            .iter()
            .all(|draw| draw.shader.source(&()) == Some(fallback))
    );
    assert_eq!(cpu.render(&packet, assets).rejected, 0);
    assert_eq!(cpu.pixels()[0], u32::from_le_bytes([255; 4]));
    assert_eq!(cpu.pixels()[30 * 160 + 80], u32::from_le_bytes([4; 4]));
    assert_eq!(assets.counts(), admitted);
    assert_eq!(vfs.take_lookup_count(), 0);
}

#[test]
fn quake_background_uses_one_alpha_material_with_native_backend_opacity() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let root = Fixture::new();
        root.native(rules);
        let vfs = root.vfs();
        let mut assets = Assets::load().unwrap();
        let palette = palette(&mut assets);
        let palettes = qa_app::render_settings::PaletteRegistrations::from_registered(
            &vfs,
            [
                Some(palette),
                Some(palette),
                Some(palette),
                Some(palette),
                None,
            ],
        );
        let mut owner = registered_assets(&vfs, assets, palettes);
        let mut console = ConsoleAssets::load(
            &vfs,
            &mut owner,
            rules,
            ImageSettings::native(rules),
            qa_render::material::ShaderCompilePolicy::default(),
            160,
            120,
        )
        .unwrap();
        let assets = owner.assets_mut();
        let mut editor = Editor::default();
        editor.toggle(SeatId::FIRST);
        let texts = TextStore::load(1, 64).unwrap();
        let log = Scrollback::default();
        let view = || ConsoleView {
            editor: &editor,
            scrollback: &log,
            texts: &texts,
            rules,
            time: EventTime(0),
        };
        let mut front = FrontEnd::load(Limits::default()).unwrap();
        let mut background_material = None;
        for software in [true, false] {
            let mut frame = front.begin_frame([3, 5, 7, 255]).unwrap();
            assert_eq!(
                console
                    .append(
                        &mut frame,
                        &assets,
                        view(),
                        160,
                        120,
                        software,
                        qa_render::shader::reference::ShaderView::direct()
                    )
                    .rejected,
                0
            );
            let packet = frame.finish();
            let background = packet
                .commands()
                .iter()
                .find_map(|command| match command {
                    Command::Draw2d(draw) if draw.rect[2] == 160.0 => Some(*draw),
                    _ => None,
                })
                .unwrap();
            assert_eq!(
                background.color,
                [255, 255, 255, if software { 255 } else { 204 }]
            );
            assert_eq!(background.texcoords, [0.0, 0.5, 1.0, 1.0]);
            let material = assets
                .material(
                    assets
                        .shader_binding(background.shader.source(&()).unwrap())
                        .unwrap()
                        .material,
                )
                .unwrap();
            assert_eq!(material.stages[0].blend, Some(StageBlend::ALPHA));
            assert_eq!(
                *background_material.get_or_insert(background.shader.source(&()).unwrap()),
                background.shader.source(&()).unwrap()
            );
            assert!(front.recycle(packet).is_ok());
        }
        let source = assets
            .material(
                assets
                    .shader_binding(background_material.unwrap())
                    .unwrap()
                    .material,
            )
            .unwrap();
        let settings = source.settings;
        let mut stage = source.stages[0];
        stage.blend = Some(StageBlend {
            source: BlendFactor::One,
            destination: BlendFactor::One,
        });
        let unsupported = assets
            .register_material("console/unsupported-add", &[stage], settings)
            .unwrap();
        let unsupported = assets
            .register_shader_binding(qa_render::ShaderBinding::material(
                unsupported,
                qa_render::world::geometry::LightSource::TwoDimensional,
            ))
            .unwrap();
        let mut cpu = CpuBackend::load_with_assets(160, 120, &assets).unwrap();
        for (material, color) in [
            (unsupported, [255; 4]),
            (background_material.unwrap(), [255, 255, 255, 128]),
        ] {
            let mut frame = front.begin_frame([3, 5, 7, 255]).unwrap();
            assert!(frame.render_scene(
                qa_render::Refdef {
                    viewport: qa_render::Viewport {
                        x: 0,
                        y: 0,
                        width: 160,
                        height: 120
                    },
                    cpu_presentation: qa_render::CpuPresentation::Indexed {
                        palette,
                        lighting: IndexedLighting::Gray,
                        ambient: 0,
                        fullbright: true,
                    },
                    ..qa_render::Refdef::default()
                },
                &[],
                &assets,
                qa_render::shader::reference::ShaderView::direct()
            ));
            assert!(frame.draw_2d(qa_render::Draw2d {
                rect: [0.0, 0.0, 160.0, 60.0],
                color,
                shader: material.into(),
                ..qa_render::Draw2d::default()
            }));
            let packet = frame.finish();
            assert_eq!(cpu.render(&packet, &assets).rejected, 1);
            assert_eq!(cpu.pixels()[0], u32::from_le_bytes([3, 5, 7, 255]));
            assert!(front.recycle(packet).is_ok());
        }
    }
}

#[test]
fn shared_native_bitmap_name_keeps_console_opaque_and_hud_index255_transparent() {
    use qa_app::{
        Runtime,
        host::FrameHost,
        renderer::hud::{HudAssets, HudView},
    };
    use qa_console::{commands::Console, views::Context};
    use qa_core::{
        configstrings::{ConfigLayout, ConfigStrings},
        primitives::{ClientId, EntityId, HudState, ThinkTime},
    };
    use qa_render::{
        Refdef, Viewport,
        shader::{
            reference::{ShaderRef, ShaderView},
            registration::{ShaderOwner, ShaderResource},
        },
    };
    use qa_session::timing::TickRate;
    use qa_ui::values::ValueLayout;

    let root = Fixture::new();
    root.native(RuleSetId::Quake2);
    root.pcx("pics/conback.pcx", 16, 16, &[255; 16 * 16]);
    let vfs = root.vfs();
    let mut assets = Assets::load().unwrap();
    let palette = palette(&mut assets);
    let mut owner = registered_assets(
        &vfs,
        assets,
        qa_app::render_settings::PaletteRegistrations::from_registered(
            &vfs,
            [None, None, Some(palette), Some(palette), None],
        ),
    );
    let mut console = ConsoleAssets::load(
        &vfs,
        &mut owner,
        RuleSetId::Quake2,
        ImageSettings::native(RuleSetId::Quake2),
        qa_render::material::ShaderCompilePolicy::default(),
        160,
        120,
    )
    .unwrap();
    let profile = console.profile();
    let hud = HudAssets::load(
        &vfs,
        &mut owner,
        RuleSetId::Quake2,
        console.font_assets(),
        160,
    )
    .unwrap();
    let mut huds = std::array::from_fn(|_| None);
    huds[RuleSetId::Quake2 as usize] = Some(hud);
    let mut runtime = Runtime::load(1, []).unwrap();
    runtime.vfs = vfs;
    runtime.model_assets = Some(owner);
    let mut host = FrameHost::load(
        Console::new(Context::default()).unwrap(),
        runtime,
        TickRate::FrameDriven,
        vec![],
    )
    .unwrap();
    let (assets, staged) = host.stage_engine_graphics(&console, &huds).unwrap();
    let mut cpu = CpuBackend::load_with_assets(160, 120, assets).unwrap();
    staged.commit();
    host.publish_model_admission(cpu.admitted_assets().unwrap())
        .unwrap();
    host.bind_client_model_assets([]);
    let context = host
        .native_shaders()
        .context(ShaderOwner::Engine, profile)
        .unwrap();
    let assets = host.runtime.model_assets.as_ref().unwrap().assets();
    let key = assets.find_name("pics/conback").unwrap();
    let nodes = host
        .native_shaders()
        .resource_nodes(context, ShaderResource::Graphic(key))
        .unwrap();
    assert_eq!(nodes.len(), 1);

    let values = ValueLayout::load(&[]).unwrap();
    let hud_state = HudState::with_capacity(0, 0, 0, values.capacity());
    let layout = ConfigLayout::QUAKE2;
    let mut configs =
        ConfigStrings::load((0..layout.count).map(|i| layout.capacity(i).unwrap())).unwrap();
    configs
        .set(layout.statusbar().unwrap(), b"xl 16 yt 16 picn conback")
        .unwrap();
    let localization = qa_content::localization::Catalog::default();
    let texts = TextStore::load(1, 64).unwrap();
    let log = Scrollback::default();
    let mut editor = Editor::default();
    editor.toggle(SeatId::FIRST);
    let clear = u32::from_le_bytes([3, 5, 7, 255]);
    let mut direct = None;
    let mut physical = None;
    host.runtime.vfs.take_lookup_count();
    for native in [false, true] {
        let shaders = if native {
            ShaderView {
                native: host.native_shaders(),
                context: Some(context),
            }
        } else {
            ShaderView::direct()
        };
        let mut front = FrontEnd::load(Limits::default()).unwrap();
        let mut frame = front.begin_frame([3, 5, 7, 255]).unwrap();
        let counts = console.append(
            &mut frame,
            assets,
            ConsoleView {
                editor: &editor,
                scrollback: &log,
                texts: &texts,
                rules: RuleSetId::Quake2,
                time: EventTime(0),
            },
            160,
            120,
            true,
            shaders,
        );
        assert_eq!((counts.backgrounds, counts.rejected), (1, 0));
        let mut packet = frame.finish();
        let console_shader = packet
            .commands()
            .iter()
            .find_map(|command| match command {
                Command::Draw2d(draw) if draw.rect[2] == 160.0 => Some(draw.shader),
                _ => None,
            })
            .unwrap();
        packet.resolve_shaders(assets, host.native_shaders());
        assert_eq!(cpu.render(&packet, assets).rejected, 0);
        let console_pixels = cpu.pixels().to_vec();
        assert_eq!(
            console_pixels[20 * 160 + 20],
            u32::from_le_bytes([0, 0, 0, 255])
        );
        assert!(front.recycle(packet).is_ok());

        let mut frame = front.begin_frame([3, 5, 7, 255]).unwrap();
        let counts = huds[RuleSetId::Quake2 as usize].as_mut().unwrap().append(
            &mut frame,
            assets,
            HudView {
                hud: &hud_state,
                texts: &texts,
                configs: &configs,
                layout,
                values: &values,
                localization: &localization,
                rules: RuleSetId::Quake2,
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
            shaders,
        );
        assert_eq!(
            (counts.pictures, counts.missing_picture, counts.rejected),
            (1, 0, 0)
        );
        let mut packet = frame.finish();
        let hud_shader = packet
            .commands()
            .iter()
            .find_map(|command| match command {
                Command::Draw2d(draw) if draw.rect[2] == 16.0 => Some(draw.shader),
                _ => None,
            })
            .unwrap();
        packet.resolve_shaders(assets, host.native_shaders());
        assert_eq!(cpu.render(&packet, assets).rejected, 0);
        assert!(cpu.pixels().iter().all(|&pixel| pixel == clear));
        let pixels = (console_pixels, cpu.pixels().to_vec());
        if native {
            assert_eq!(direct.as_ref(), Some(&pixels));
        } else {
            direct = Some(pixels);
        }
        let bodies = (
            console_shader.source(host.native_shaders()).unwrap(),
            hud_shader.source(host.native_shaders()).unwrap(),
        );
        assert_ne!(bodies.0, bodies.1);
        if native {
            let reference = |shader| match shader {
                ShaderRef::Surface { reference, .. } => reference,
                _ => panic!("converted bitmap lost native name membership"),
            };
            assert_eq!(reference(console_shader), reference(hud_shader));
            assert_eq!(reference(hud_shader).node, nodes[0]);
            assert_eq!(physical, Some(bodies));
        } else {
            physical = Some(bodies);
        }
    }
    assert_eq!(host.runtime.vfs.take_lookup_count(), 0);
}
