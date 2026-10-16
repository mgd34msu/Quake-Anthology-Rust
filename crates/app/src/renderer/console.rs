//! Console assets are registered once; the shared scene owns all 2D commands.
use super::text::BitmapFont;
use crate::model_assets::{ModelAssets, ModelProfile};
use qa_console::presentation::{ConsoleView, Layout, Style};
use qa_content::vfs::Vfs;
use qa_core::primitives::RuleSetId;
use qa_render::{
    Assets, CpuPresentation, Draw2d, Frame, PaletteId, Refdef, ShaderBinding, ShaderBindingId,
    Viewport,
    assets::{AlphaTest, DepthFunc, Filter, MaterialSettings, Sampler, Stage, StageTexture, Wrap},
    material::resources::{ImageSettings, ImageUse, Images},
    shader::{AlphaGen, Cull, RgbGen, StageBlend},
    surface_cache::IndexedLighting,
    world::geometry::LightSource,
};
use qa_render::{
    ShaderRegistration,
    shader::reference::{ShaderChoice, ShaderDefault, ShaderOccurrence, ShaderRef, ShaderView},
};
use std::sync::Arc;

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
    font: Arc<FontAssets>,
    background: Option<ShaderRegistration>,
    white: Option<ShaderRegistration>,
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
        owner: &mut ModelAssets,
        rules: RuleSetId,
        settings: ImageSettings,
        compile_policy: qa_render::material::ShaderCompilePolicy,
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
        let font = FontAssets::load(vfs, owner, rules, settings, compile_policy)?;
        let palette = font.palette;
        let mut console = Self {
            rules,
            font: Arc::clone(&font),
            background: None,
            white: None,
            palette,
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
        if q3 {
            install(
                &mut console.white,
                "white",
                owner.graphic_shader(vfs, font.profile, "white", compile_policy.ui_fullscreen),
            );
            install(
                &mut console.background,
                "console",
                owner.graphic_shader(vfs, font.profile, "console", compile_policy.ui_fullscreen),
            );
        } else if let Some(palette) = palette {
            let result = owner.graphic_images(vfs, font.profile, |images| {
                let path = if matches!(rules, RuleSetId::Quake | RuleSetId::QuakeWorld) {
                    "gfx/conback.lmp"
                } else {
                    "pics/conback.pcx"
                };
                let shader = if matches!(rules, RuleSetId::Quake | RuleSetId::QuakeWorld) {
                    let image = images
                        .indexed_lmp(
                            path,
                            palette,
                            None,
                            ImageUse {
                                sampler: bitmap_sampler(),
                                ..ImageUse::pic()
                            },
                        )
                        .map_err(|error| format!("{error:?}"))?;
                    bitmap_shader(
                        images,
                        path,
                        image.id,
                        image.sampler,
                        Some(StageBlend::ALPHA),
                    )
                    .map_err(str::to_owned)?
                } else {
                    let image = images
                        .indexed_pcx(
                            path,
                            palette,
                            None,
                            None,
                            ImageUse {
                                sampler: bitmap_sampler(),
                                ..ImageUse::pic()
                            },
                        )
                        .map_err(|error| format!("{error:?}"))?;
                    bitmap_shader(images, path, image, bitmap_sampler(), None)
                        .map_err(str::to_owned)?
                };
                Ok(shader)
            });
            install(&mut console.background, "background", result);
        }
        console.report();
        Ok(console)
    }
    fn report(&self) {
        qa_console::logger::console(format_args!(
            "{{\"event\":\"console_resources\",\"rules\":\"{}\",\"font_available\":{},\"background_available\":{},\"native_font\":{},\"scrollback_records\":{}}}\n",
            self.rules.name(),
            self.font.native,
            self.background.is_some(),
            self.font.native,
            qa_console::presentation::SCROLLBACK_RECORDS,
        ));
    }
    pub fn command_capacity(&self) -> usize {
        (self.layout.rows() + 1) * self.layout.columns() + 3
    }
    pub fn font_for(&self, rules: RuleSetId) -> Option<Arc<FontAssets>> {
        (self.rules == rules).then(|| Arc::clone(&self.font))
    }
    pub fn font_assets(&self) -> Arc<FontAssets> {
        Arc::clone(&self.font)
    }
    pub fn profile(&self) -> ModelProfile {
        self.font.profile
    }
    pub(crate) fn registrations(&self) -> impl Iterator<Item = &ShaderRegistration> + Clone {
        self.font
            .registration
            .iter()
            .chain(self.white.iter())
            .chain(self.background.iter())
    }
    pub fn append(
        &mut self,
        frame: &mut Frame,
        assets: &Assets,
        view: ConsoleView<'_>,
        width: u32,
        height: u32,
        software: bool,
        shaders: ShaderView<'_>,
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
        counts.rejected += u32::from(!frame.render_scene(refdef, &[], assets, shaders));
        if let Some(shader) = self
            .background
            .and_then(|registration| self.font.graphic(shaders, &registration))
        {
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
        if self.q3
            && let Some(shader) = self
                .white
                .and_then(|registration| self.font.graphic(shaders, &registration))
        {
            counts.rejected += u32::from(!frame.draw_2d(Draw2d {
                rect: [0.0, self.height.saturating_sub(2) as f32, width as f32, 2.0],
                texcoords: [0.0, 0.0, 1.0, 1.0],
                color: [255, 0, 0, 255],
                shader,
            }));
        }
        let Some(font) = self.font.painter(shaders) else {
            counts.missing_font += 1;
            counts.rejected += 1;
            return counts;
        };
        counts.non_native_font += u32::from(self.rules == RuleSetId::Quake2Rerelease);
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
    slot: &mut Option<ShaderRegistration>,
    name: &str,
    result: Result<ShaderRegistration, String>,
) {
    match result {
        Ok(shader) => *slot = Some(shader),
        Err(error) => {
            qa_console::logger::console(format_args!("console {name} unavailable: {error}\n"))
        }
    }
}
/// One retained font registration is shared by console and HUD owners.
pub struct FontAssets {
    pub(crate) compile_policy: qa_render::material::ShaderCompilePolicy,
    pub profile: ModelProfile,
    pub registration: Option<ShaderRegistration>,
    pub palette: Option<PaletteId>,
    pub native: bool,
    height: u32,
    native_default: Option<ShaderBindingId>,
}
impl FontAssets {
    pub(crate) fn registration_request(
        rules: RuleSetId,
    ) -> Option<qa_compat::services::RendererRequest<&'static [u8]>> {
        (rules == RuleSetId::Quake3).then_some(qa_compat::services::RendererRequest {
            name: b"gfx/2d/bigchars",
            source: rules,
            kind: qa_compat::services::RendererRegistration::Shader {
                mipmaps: true,
                lighting: qa_compat::services::ShaderLighting::TwoDimensional,
            },
            world: None,
        })
    }

    pub fn load(
        vfs: &Vfs,
        owner: &mut ModelAssets,
        rules: RuleSetId,
        settings: ImageSettings,
        compile_policy: qa_render::material::ShaderCompilePolicy,
    ) -> Result<Arc<Self>, String> {
        let profile = owner.graphics_profile(vfs, rules, settings, compile_policy.finish)?;
        let result = if let Some(request) = Self::registration_request(rules) {
            owner.graphic_shader(
                vfs,
                profile,
                std::str::from_utf8(request.name).map_err(|_| "font registration name")?,
                compile_policy.ui_fullscreen,
            )
        } else {
            owner.graphic_images(vfs, profile, |images| {
                register_font(images, vfs, rules, profile.palette)
            })
        };
        let mut registration = None;
        install(&mut registration, "native bitmap font", result);
        Ok(Arc::new(Self {
            compile_policy,
            profile,
            registration,
            palette: profile.palette,
            native: registration.is_some_and(|registration| registration.defaulted.is_none()),
            height: qa_gameplay::rules::presentation::text_rules(rules).glyph_height,
            native_default: owner.graphic_default(profile),
        }))
    }
    pub(crate) fn graphic(
        &self,
        shaders: ShaderView<'_>,
        registration: &ShaderRegistration,
    ) -> Option<ShaderRef> {
        let occurrence = ShaderOccurrence(0);
        let choice = if self.profile.source_rules == RuleSetId::Quake3 {
            ShaderChoice::Occurrence(occurrence)
        } else {
            ShaderChoice::Converted {
                binding: registration.shader,
                occurrence,
            }
        };
        shaders.graphic(
            registration,
            choice,
            ShaderDefault::Global(self.native_default),
        )
    }
    pub(crate) fn painter(&self, shaders: ShaderView<'_>) -> Option<BitmapFont> {
        if !self.native {
            return None;
        }
        Some(BitmapFont {
            shader: self.graphic(shaders, self.registration.as_ref()?)?,
            height: self.height,
        })
    }
}
fn register_font(
    images: &mut Images<'_>,
    vfs: &Vfs,
    rules: RuleSetId,
    palette: Option<PaletteId>,
) -> Result<ShaderRegistration, String> {
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
) -> Result<ShaderRegistration, &'static str> {
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
    let shader = images
        .assets
        .register_shader_binding(ShaderBinding::material(
            material,
            LightSource::TwoDimensional,
        ))?;
    images.assets.shader_registration(name.as_bytes(), shader)
}
