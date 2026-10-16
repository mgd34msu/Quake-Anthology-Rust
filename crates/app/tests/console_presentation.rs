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

#[test]
fn retail_rerelease_png_font_is_shared_and_retains_a_software_palette() {
    let Some(root) = std::env::var_os("QA_Q2_RERELEASE_ROOT") else {
        return;
    };
    let mut vfs = Vfs::default();
    vfs.mount_product(std::path::Path::new(&root), 0).unwrap();
    assert!(vfs.open(b"pics/conchars.png").is_some());
    assert!(vfs.open(b"pics/conchars.pcx").is_none());
    let mut assets = Assets::load().unwrap();
    let mut palettes = qa_app::render_settings::PaletteRegistrations::default();
    let font = qa_app::renderer::console::FontAssets::load(
        &vfs,
        &mut assets,
        RuleSetId::Quake2Rerelease,
        &mut palettes,
        ImageSettings::native(RuleSetId::Quake2Rerelease),
    )
    .unwrap();
    assert!(font.native);
    let material = assets
        .material(
            assets
                .shader_binding(font.font.unwrap().shader)
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

#[test]
fn native_fonts_and_backgrounds_become_shared_2d_packets_and_cpu_pixels() {
    for rules in RuleSetId::ALL {
        let root = Fixture::new();
        root.native(rules);
        let mut assets = Assets::load().unwrap();
        let palette = (rules != RuleSetId::Quake3).then(|| palette(&mut assets));
        let mut palettes = qa_app::render_settings::PaletteRegistrations::from_registered([
            palette, palette, palette, palette, None,
        ]);
        let mut console = ConsoleAssets::load(
            &root.vfs(),
            &mut assets,
            rules,
            &mut palettes,
            ImageSettings::native(rules),
            160,
            120,
        )
        .unwrap();
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
        let mut palettes = qa_app::render_settings::PaletteRegistrations::from_registered([
            Some(palette),
            Some(palette),
            Some(palette),
            Some(palette),
            None,
        ]);
        let settings = ImageSettings::native(rules);
        let console =
            ConsoleAssets::load(&vfs, &mut assets, rules, &mut palettes, settings, 160, 120)
                .unwrap();
        let hud = qa_app::renderer::console::FontAssets::load(
            &vfs,
            &mut assets,
            rules,
            &mut palettes,
            settings,
        )
        .unwrap();
        let console_font = console.font_assets().font.unwrap();
        let hud_font = hud.font.unwrap();
        let image = |font: qa_app::renderer::text::BitmapFont| {
            let material = assets
                .material(assets.shader_binding(font.shader).unwrap().material)
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
    let mut assets = Assets::load().unwrap();
    let mut palettes = qa_app::render_settings::PaletteRegistrations::default();
    let mut console = ConsoleAssets::load(
        &Vfs::default(),
        &mut assets,
        RuleSetId::Quake3,
        &mut palettes,
        ImageSettings::native(RuleSetId::Quake3),
        160,
        120,
    )
    .unwrap();
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
    );
    assert_eq!(missing.glyphs, 0);
    assert_eq!(missing.missing_font, 1);
    assert_eq!(missing.missing_background, 1);
    assert!(missing.rejected >= 2);
}

#[test]
fn quake_background_uses_one_alpha_material_with_native_backend_opacity() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        let root = Fixture::new();
        root.native(rules);
        let mut assets = Assets::load().unwrap();
        let palette = palette(&mut assets);
        let mut palettes = qa_app::render_settings::PaletteRegistrations::from_registered([
            Some(palette),
            Some(palette),
            Some(palette),
            Some(palette),
            None,
        ]);
        let mut console = ConsoleAssets::load(
            &root.vfs(),
            &mut assets,
            rules,
            &mut palettes,
            ImageSettings::native(rules),
            160,
            120,
        )
        .unwrap();
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
                    .append(&mut frame, &assets, view(), 160, 120, software)
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
