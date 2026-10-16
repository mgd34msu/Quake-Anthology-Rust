//! Console assets are registered once; the shared scene owns all 2D commands.
use super::text::BitmapFont;
use qa_console::presentation::{ConsoleView, Layout, Style};
use qa_content::vfs::Vfs;
use qa_core::primitives::RuleSetId;
use qa_render::{
    Assets, CpuPresentation, Draw2d, Frame, PaletteId, Refdef, ShaderBinding, ShaderBindingId,
    Viewport,
    assets::{AlphaTest, DepthFunc, Filter, MaterialSettings, Sampler, Stage, StageTexture, Wrap},
    material::{
        self,
        resources::{ImageSettings, ImageUse, Images},
        world_load::compile_definition,
    },
    shader::{AlphaGen, Cull, RgbGen, StageBlend},
    surface_cache::IndexedLighting,
    world::geometry::LightSource,
};

#[derive(Clone, Copy, Debug, Default)]
pub struct ConsoleStats {
    pub glyphs: u32,
    pub backgrounds: u32,
    pub missing_font: u32,
    pub missing_background: u32,
    pub non_native_font: u32,
    pub rejected: u32,
}

pub struct ConsoleAssets {
    rules: RuleSetId,
    font: Option<ShaderBindingId>,
    background: Option<ShaderBindingId>,
    palette: Option<PaletteId>,
    layout: Layout,
    height: u32,
    glyph_height: u32,
    input_y: u32,
    body_y: u32,
    q3: bool,
    legacy_gl_alpha: bool,
}
impl ConsoleAssets {
    pub fn load(
        vfs: &Vfs,
        assets: &mut Assets,
        rules: RuleSetId,
        palettes: &mut crate::render_settings::PaletteRegistrations,
        settings: ImageSettings,
        width: u32,
        height: u32,
    ) -> Result<Self, String> {
        let q3 = rules == RuleSetId::Quake3;
        let glyph_height = qa_gameplay::rules::presentation::text_rules(rules).glyph_height;
        let height = (height / 2).max(glyph_height * 3);
        let (input_offset, log_offset) = match rules {
            RuleSetId::Quake | RuleSetId::QuakeWorld => (16, 24),
            RuleSetId::Quake2 | RuleSetId::Quake2Rerelease => (22, 28),
            RuleSetId::Quake3 => (32, 48),
        };
        let rows = height.saturating_sub(log_offset) / glyph_height + 1;
        let columns = (width / 8).saturating_sub(2).max(2);
        let layout = Layout::load(
            columns as usize,
            rows as usize,
            Style {
                colors: q3,
                cursor_period_ms: if matches!(rules, RuleSetId::Quake | RuleSetId::QuakeWorld) {
                    250
                } else {
                    256
                },
                blink_cursor: q3,
            },
        )
        .map_err(str::to_owned)?;
        let palette = crate::render_settings::palette(vfs, assets, rules, palettes);
        let mut console = Self {
            rules,
            font: None,
            background: None,
            palette: palette.as_ref().ok().copied().flatten(),
            layout,
            height,
            glyph_height,
            input_y: height.saturating_sub(input_offset),
            body_y: height
                .saturating_sub(log_offset)
                .saturating_sub((rows - 1) * glyph_height),
            q3,
            legacy_gl_alpha: matches!(rules, RuleSetId::Quake | RuleSetId::QuakeWorld),
        };
        if let Err(error) = palette {
            qa_console::logger::console(format_args!("console palette unavailable: {error}\n"));
            console.report();
            return Ok(console);
        }
        let mut images = Images::new(vfs, assets, rules, rules, settings, None)
            .map_err(|error| format!("console images: {error:?}"))?;
        install(
            &mut console.font,
            "native bitmap font",
            register_font(&mut images, vfs, rules, console.palette),
        );
        if q3 {
            let catalog = material::load_catalog(vfs)
                .map_err(|error| format!("console shaders: {error:?}"))?;
            for (name, slot) in [("console", &mut console.background)] {
                let result = if let Some(definition) = catalog.find_canonical(name) {
                    compile_definition(definition, &mut images)
                        .map_err(|error| format!("{error:?}"))
                        .and_then(|material| {
                            images
                                .assets
                                .register_shader_binding(ShaderBinding::material(
                                    material,
                                    LightSource::TwoDimensional,
                                ))
                                .map_err(str::to_owned)
                        })
                } else {
                    images
                        .raster(name, ImageUse::pic(), None)
                        .map_err(|error| format!("{error:?}"))
                        .and_then(|image| {
                            bitmap_shader(&mut images, name, image.id, image.sampler, None)
                                .map_err(str::to_owned)
                        })
                };
                install(slot, name, result);
            }
        } else if let Some(palette) = console.palette {
            if matches!(rules, RuleSetId::Quake | RuleSetId::QuakeWorld) {
                let background = images
                    .indexed_lmp(
                        "gfx/conback.lmp",
                        palette,
                        None,
                        ImageUse {
                            sampler: bitmap_sampler(),
                            ..ImageUse::pic()
                        },
                    )
                    .map_err(|error| format!("{error:?}"))
                    .and_then(|image| {
                        bitmap_shader(
                            &mut images,
                            "gfx/conback.lmp",
                            image.id,
                            image.sampler,
                            Some(StageBlend::ALPHA),
                        )
                        .map_err(str::to_owned)
                    });
                install(&mut console.background, "gfx/conback.lmp", background);
            } else {
                for (path, mask, slot) in [("pics/conback.pcx", None, &mut console.background)] {
                    let sampler = bitmap_sampler();
                    let result = images
                        .indexed_pcx(
                            path,
                            palette,
                            mask,
                            None,
                            ImageUse {
                                sampler,
                                ..ImageUse::pic()
                            },
                        )
                        .map_err(|error| format!("{error:?}"))
                        .and_then(|image| {
                            bitmap_shader(&mut images, path, image, sampler, None)
                                .map_err(str::to_owned)
                        });
                    install(slot, path, result);
                }
            }
        }
        console.report();
        Ok(console)
    }
    fn report(&self) {
        qa_console::logger::console(format_args!(
            "{{\"event\":\"console_resources\",\"rules\":\"{}\",\"font_available\":{},\"background_available\":{},\"native_font\":{},\"scrollback_records\":{}}}\n",
            self.rules.name(),
            self.font.is_some(),
            self.background.is_some(),
            self.font.is_some(),
            qa_console::presentation::SCROLLBACK_RECORDS,
        ));
    }
    pub fn command_capacity(&self) -> usize {
        (self.layout.rows() + 1) * self.layout.columns() + 3
    }
    pub fn font_for(&self, rules: RuleSetId) -> Option<FontAssets> {
        (self.rules == rules).then(|| self.font_assets())
    }
    pub fn font_assets(&self) -> FontAssets {
        FontAssets {
            font: self.font.map(|shader| BitmapFont {
                shader,
                height: self.glyph_height,
            }),
            palette: self.palette,
            native: self.font.is_some(),
        }
    }
    pub fn append(
        &mut self,
        frame: &mut Frame,
        assets: &Assets,
        view: ConsoleView<'_>,
        width: u32,
        height: u32,
        software: bool,
    ) -> ConsoleStats {
        let mut counts = ConsoleStats::default();
        if view.editor.seat().is_none() {
            return counts;
        }
        if view.rules != self.rules {
            counts.rejected += 1;
            return counts;
        }
        self.layout.prepare(view);
        frame.clear_scene();
        let refdef = Refdef {
            viewport: Viewport {
                x: 0,
                y: 0,
                width,
                height,
            },
            time_ms: view.time.milliseconds() as i64,
            cpu_presentation: self.palette.map_or(CpuPresentation::Rgb, |palette| {
                CpuPresentation::Indexed {
                    palette,
                    lighting: IndexedLighting::Gray,
                    ambient: 0,
                    fullbright: true,
                }
            }),
            ..Refdef::default()
        };
        counts.rejected += u32::from(!frame.render_scene(refdef, &[], assets));
        if let Some(shader) = self.background {
            let draw = Draw2d {
                rect: [
                    0.0,
                    0.0,
                    width as f32,
                    if self.q3 {
                        self.height.saturating_sub(2)
                    } else {
                        self.height
                    } as f32,
                ],
                texcoords: if self.q3 {
                    [0.0, 0.0, 1.0, 1.0]
                } else {
                    [0.0, 1.0 - self.height as f32 / height as f32, 1.0, 1.0]
                },
                color: self.background_color(height, software),
                shader,
            };
            let accepted = frame.draw_2d(draw);
            counts.backgrounds += u32::from(accepted);
            counts.rejected += u32::from(!accepted);
        } else {
            counts.missing_background += 1;
            counts.rejected += 1;
        }
        if self.q3 {
            counts.rejected += u32::from(!frame.draw_2d(Draw2d {
                rect: [0.0, self.height.saturating_sub(2) as f32, width as f32, 2.0],
                texcoords: [0.0, 0.0, 1.0, 1.0],
                color: [255, 0, 0, 255],
                shader: ShaderBindingId(0),
            }));
        }
        let Some(font) = self.font else {
            counts.missing_font += 1;
            counts.rejected += 1;
            return counts;
        };
        counts.non_native_font += u32::from(self.rules == RuleSetId::Quake2Rerelease);
        let font = BitmapFont {
            shader: font,
            height: self.glyph_height,
        };
        for (row, cells) in self.layout.lines() {
            let text = font.cells(
                frame,
                cells,
                8,
                (self.body_y + row as u32 * self.glyph_height) as i64,
                refdef.viewport,
            );
            counts.glyphs += text.glyphs;
            counts.rejected += text.rejected;
        }
        let text = font.cells(
            frame,
            self.layout.input(),
            8,
            self.input_y as i64,
            refdef.viewport,
        );
        counts.glyphs += text.glyphs;
        counts.rejected += text.rejected;
        counts
    }
    fn background_color(&self, height: u32, software: bool) -> [u8; 4] {
        let mut color = [255; 4];
        if self.legacy_gl_alpha && !software {
            let threshold = (u64::from(height) * 3) >> 2;
            if u64::from(self.height) <= threshold && threshold != 0 {
                // GLQuake computes the double product, narrows it, then divides.
                let alpha = (1.2 * f64::from(self.height)) as f32 / threshold as f32;
                color[3] = (alpha * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
        color
    }
}

fn install(
    slot: &mut Option<ShaderBindingId>,
    name: &str,
    result: Result<ShaderBindingId, String>,
) {
    match result {
        Ok(shader) => *slot = Some(shader),
        Err(error) => {
            qa_console::logger::console(format_args!("console {name} unavailable: {error}\n"))
        }
    }
}
/// Cold registration shared by console and independently selected HUD roles.
#[derive(Clone, Copy)]
pub struct FontAssets {
    pub font: Option<BitmapFont>,
    pub palette: Option<PaletteId>,
    pub native: bool,
}
impl FontAssets {
    pub fn load(
        vfs: &Vfs,
        assets: &mut Assets,
        rules: RuleSetId,
        palettes: &mut crate::render_settings::PaletteRegistrations,
        settings: ImageSettings,
    ) -> Result<Self, String> {
        let palette = crate::render_settings::palette(vfs, assets, rules, palettes)
            .ok()
            .flatten();
        let mut images = Images::new(vfs, assets, rules, rules, settings, None)
            .map_err(|error| format!("HUD images: {error:?}"))?;
        let mut font = None;
        install(
            &mut font,
            "native bitmap font",
            register_font(&mut images, vfs, rules, palette),
        );
        Ok(Self {
            font: font.map(|shader| BitmapFont {
                shader,
                height: qa_gameplay::rules::presentation::text_rules(rules).glyph_height,
            }),
            palette,
            native: font.is_some(),
        })
    }
}
fn register_font(
    images: &mut Images<'_>,
    vfs: &Vfs,
    rules: RuleSetId,
    palette: Option<PaletteId>,
) -> Result<ShaderBindingId, String> {
    if rules == RuleSetId::Quake3 {
        let catalog =
            material::load_catalog(vfs).map_err(|error| format!("console shaders: {error:?}"))?;
        let name = "gfx/2d/bigchars";
        if let Some(definition) = catalog.find_canonical(name) {
            let material =
                compile_definition(definition, images).map_err(|error| format!("{error:?}"))?;
            return images
                .assets
                .register_shader_binding(ShaderBinding::material(
                    material,
                    LightSource::TwoDimensional,
                ))
                .map_err(str::to_owned);
        }
        let image = images
            .raster(name, ImageUse::pic(), None)
            .map_err(|error| format!("{error:?}"))?;
        return bitmap_shader(images, name, image.id, image.sampler, None).map_err(str::to_owned);
    }
    let palette = palette.ok_or("native font palette unavailable")?;
    if matches!(rules, RuleSetId::Quake | RuleSetId::QuakeWorld) {
        let image = images
            .indexed_wad(
                "gfx.wad",
                b"conchars",
                palette,
                Some(0),
                ImageUse {
                    sampler: bitmap_sampler(),
                    ..ImageUse::pic()
                },
            )
            .map_err(|error| format!("{error:?}"))?;
        return bitmap_shader(images, "console/conchars", image.id, image.sampler, None)
            .map_err(str::to_owned);
    }
    if rules == RuleSetId::Quake2Rerelease && vfs.open(b"pics/conchars.png").is_some() {
        let image = images
            .raster(
                "pics/conchars.png",
                ImageUse {
                    sampler: bitmap_sampler(),
                    ..ImageUse::pic()
                },
                Some(qa_render::material::resources::IndexedTarget {
                    palette,
                    transparent_index: Some(255),
                }),
            )
            .map_err(|error| format!("{error:?}"))?;
        return bitmap_shader(
            images,
            "pics/conchars.png",
            image.id,
            image.sampler,
            Some(StageBlend::ALPHA),
        )
        .map_err(str::to_owned);
    }
    let sampler = bitmap_sampler();
    let image = images
        .indexed_pcx(
            "pics/conchars.pcx",
            palette,
            Some(255),
            None,
            ImageUse {
                sampler,
                ..ImageUse::pic()
            },
        )
        .map_err(|error| format!("{error:?}"))?;
    bitmap_shader(images, "pics/conchars.pcx", image, sampler, None).map_err(str::to_owned)
}
pub(crate) fn bitmap_sampler() -> Sampler {
    Sampler {
        wrap: Wrap::Clamp,
        filter: Filter::Nearest,
        mipmaps: false,
    }
}
pub(crate) fn bitmap_shader(
    images: &mut Images<'_>,
    name: &str,
    image: qa_render::ImageId,
    sampler: Sampler,
    blend: Option<StageBlend>,
) -> Result<ShaderBindingId, &'static str> {
    let material = images.assets.register_material(
        name,
        &[Stage {
            texture: StageTexture::Image(image),
            sampler,
            blend,
            rgb_gen: RgbGen::ExactVertex,
            alpha_gen: AlphaGen::Vertex,
            alpha_test: AlphaTest::GreaterZero,
            depth_func: DepthFunc::Always,
            depth_write: false,
            ..Stage::default()
        }],
        MaterialSettings {
            cull: Cull::None,
            ..MaterialSettings::default()
        },
    )?;
    images
        .assets
        .register_shader_binding(ShaderBinding::material(
            material,
            LightSource::TwoDimensional,
        ))
}
