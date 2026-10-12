//! Console assets are registered once; the shared scene owns all 2D commands.
use qa_console::presentation::{Cell, ConsoleView, Layout, Style};
use qa_content::vfs::Vfs;
use qa_core::primitives::RuleSetId;
use qa_formats::{
    archive::ArchiveReader,
    image::{self, DecodedImage, ImageFormat, IndexedImage, Wad, WadImage},
};
use qa_render::{
    Assets, CpuPresentation, Draw2d, Frame, MaterialId, PaletteId, Refdef, Viewport,
    assets::{AlphaTest, DepthFunc, Filter, MaterialSettings, Sampler, Stage, StageTexture, Wrap},
    material::{
        self,
        resources::{ImageSettings, ImageUse, Images, PaletteSource, load_palette},
        world_load::compile_definition,
    },
    shader::{AlphaGen, Cull, RgbGen, StageBlend},
    surface_cache::{IndexedLighting, IndexedTexture},
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
    font: Option<MaterialId>,
    background: Option<MaterialId>,
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
        palette: Option<PaletteId>,
        settings: ImageSettings,
        width: u32,
        height: u32,
    ) -> Result<Self, String> {
        let q3 = rules == RuleSetId::Quake3;
        let glyph_height = if q3 { 16 } else { 8 };
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
        let mut console = Self {
            rules,
            font: None,
            background: None,
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
        if !q3 && console.palette.is_none() {
            let source = if matches!(rules, RuleSetId::Quake | RuleSetId::QuakeWorld) {
                PaletteSource::Lmp {
                    colors: b"gfx/palette.lmp",
                    shades: b"gfx/colormap.lmp",
                }
            } else {
                PaletteSource::Pcx {
                    image: b"pics/colormap.pcx",
                }
            };
            match load_palette(vfs, assets, source) {
                Ok(palette) => console.palette = Some(palette),
                Err(error) => {
                    qa_console::logger::console(format_args!(
                        "console palette unavailable: {error:?}\n"
                    ));
                    console.report();
                    return Ok(console);
                }
            }
        }
        let mut images = Images::new(vfs, assets, rules, settings)
            .map_err(|error| format!("console images: {error:?}"))?;
        if q3 {
            let catalog = material::load_catalog(vfs)
                .map_err(|error| format!("console shaders: {error:?}"))?;
            for (name, slot) in [
                ("gfx/2d/bigchars", &mut console.font),
                ("console", &mut console.background),
            ] {
                let result = if let Some(definition) = catalog.find_canonical(name) {
                    compile_definition(definition, &mut images)
                        .map_err(|error| format!("{error:?}"))
                } else {
                    images
                        .raster(name, ImageUse::pic())
                        .map_err(|error| format!("{error:?}"))
                        .and_then(|image| {
                            bitmap_material(&mut images, name, image.id, image.sampler, None)
                                .map_err(str::to_owned)
                        })
                };
                install(slot, name, result);
            }
        } else if let Some(palette) = console.palette {
            if matches!(rules, RuleSetId::Quake | RuleSetId::QuakeWorld) {
                let mut reader = ArchiveReader::default();
                let font = vfs
                    .read_file(b"gfx.wad", 16 * 1024 * 1024, &mut reader)
                    .map_err(|error| format!("{error:?}"))
                    .and_then(|bytes| {
                        let wad = Wad::parse(&bytes).map_err(|error| format!("{error:?}"))?;
                        let index = wad.find(b"conchars").ok_or("missing conchars")?;
                        let WadImage::Image(image) =
                            wad.image(index).map_err(|error| format!("{error:?}"))?
                        else {
                            return Err("conchars is not a bitmap".into());
                        };
                        indexed_material(
                            &mut images,
                            "console/conchars",
                            &image,
                            palette,
                            Some(0),
                            None,
                        )
                        .map_err(str::to_owned)
                    });
                install(&mut console.font, "gfx.wad:conchars", font);
                let background = vfs
                    .read_file(b"gfx/conback.lmp", 16 * 1024 * 1024, &mut reader)
                    .map_err(|error| format!("{error:?}"))
                    .and_then(|bytes| {
                        let DecodedImage::Indexed(image) =
                            image::decode(&bytes, ImageFormat::Lmp, rules.lineage())
                                .map_err(|error| format!("{error:?}"))?
                        else {
                            return Err("conback is not indexed".into());
                        };
                        indexed_material(
                            &mut images,
                            "gfx/conback.lmp",
                            &image,
                            palette,
                            None,
                            Some(StageBlend::ALPHA),
                        )
                        .map_err(str::to_owned)
                    });
                install(&mut console.background, "gfx/conback.lmp", background);
            } else {
                for (path, mask, slot) in [
                    ("pics/conchars.pcx", Some(255), &mut console.font),
                    ("pics/conback.pcx", None, &mut console.background),
                ] {
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
                            bitmap_material(&mut images, path, image, sampler, None)
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
            self.font.is_some() && self.rules != RuleSetId::Quake2Rerelease,
            qa_console::presentation::SCROLLBACK_RECORDS,
        ));
    }
    pub fn command_capacity(&self) -> usize {
        (self.layout.rows() + 1) * self.layout.columns() + 3
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
            time_ms: view.time.milliseconds(),
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
        if let Some(material) = self.background {
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
                material,
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
                material: MaterialId(0),
            }));
        }
        let Some(font) = self.font else {
            counts.missing_font += 1;
            counts.rejected += 1;
            return counts;
        };
        counts.non_native_font += u32::from(self.rules == RuleSetId::Quake2Rerelease);
        for (row, cells) in self.layout.lines() {
            append_line(
                frame,
                font,
                cells,
                self.body_y + row as u32 * self.glyph_height,
                self.glyph_height,
                &mut counts,
            );
        }
        append_line(
            frame,
            font,
            self.layout.input(),
            self.input_y,
            self.glyph_height,
            &mut counts,
        );
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

fn install(slot: &mut Option<MaterialId>, name: &str, result: Result<MaterialId, String>) {
    match result {
        Ok(material) => *slot = Some(material),
        Err(error) => {
            qa_console::logger::console(format_args!("console {name} unavailable: {error}\n"))
        }
    }
}
fn bitmap_sampler() -> Sampler {
    Sampler {
        wrap: Wrap::Clamp,
        filter: Filter::Nearest,
        mipmaps: false,
    }
}
fn bitmap_material(
    images: &mut Images<'_>,
    name: &str,
    image: qa_render::ImageId,
    sampler: Sampler,
    blend: Option<StageBlend>,
) -> Result<MaterialId, &'static str> {
    images.assets.register_material(
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
    )
}
fn indexed_material(
    images: &mut Images<'_>,
    name: &str,
    image: &IndexedImage<'_>,
    palette: PaletteId,
    transparent: Option<u8>,
    blend: Option<StageBlend>,
) -> Result<MaterialId, &'static str> {
    let texture =
        IndexedTexture::load_base(image.width, image.height, &image.indices, transparent)?;
    let id = images.assets.register_indexed_image(texture, palette)?;
    let params = images
        .upload_params(ImageUse {
            sampler: bitmap_sampler(),
            ..ImageUse::pic()
        })
        .map_err(|_| "console image settings")?;
    images.assets.prepare_image(id, params)?;
    bitmap_material(images, name, id, bitmap_sampler(), blend)
}
fn append_line(
    frame: &mut Frame,
    font: MaterialId,
    cells: &[Cell],
    y: u32,
    height: u32,
    counts: &mut ConsoleStats,
) {
    const COLORS: [[u8; 4]; 8] = [
        [0, 0, 0, 255],
        [255, 0, 0, 255],
        [0, 255, 0, 255],
        [255, 255, 0, 255],
        [0, 0, 255, 255],
        [0, 255, 255, 255],
        [255, 0, 255, 255],
        [255; 4],
    ];
    for (column, cell) in cells.iter().enumerate() {
        if cell.glyph & 127 == b' ' {
            continue;
        }
        let s = f32::from(cell.glyph & 15) * 0.0625;
        let t = f32::from(cell.glyph >> 4) * 0.0625;
        let accepted = frame.draw_2d(Draw2d {
            rect: [((column + 1) * 8) as f32, y as f32, 8.0, height as f32],
            texcoords: [s, t, s + 0.0625, t + 0.0625],
            color: COLORS[usize::from(cell.color)],
            material: font,
        });
        counts.glyphs += u32::from(accepted);
        counts.rejected += u32::from(!accepted);
    }
}
