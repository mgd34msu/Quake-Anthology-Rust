//! Load-time image registration; layout execution uses the shared asset names.
use super::{
    super::console::{FontAssets, bitmap_sampler, bitmap_shader},
    HudStats,
};
use crate::model_assets::ModelAssets;
use qa_content::vfs::Vfs;
use qa_core::{
    primitives::{NameId, RuleSetId},
    text::FixedText,
};
use qa_formats::image::ImageFormat;
use qa_render::{Assets, Draw2d, Frame, ShaderBindingId, Viewport, material::resources::ImageUse};
use qa_render::{ShaderRegistration, shader::reference::ShaderView};
use std::fmt::Write;

#[derive(Clone, Copy)]
pub struct Picture {
    shader: ShaderRegistration,
    size: [u32; 2],
}
#[derive(Default)]
pub struct Pictures {
    entries: Vec<(NameId, Picture)>,
    numbers: [[Option<usize>; 11]; 2],
}
impl Pictures {
    pub fn load(
        vfs: &Vfs,
        owner: &mut ModelAssets,
        rules: RuleSetId,
        font: &FontAssets,
    ) -> Result<Self, String> {
        let mut result = Self::default();
        if !matches!(rules, RuleSetId::Quake2 | RuleSetId::Quake2Rerelease) {
            return Ok(result);
        }
        let Some(palette) = font.palette else {
            return Ok(result);
        };
        let mut paths: Vec<_> = vfs
            .files()
            .filter_map(|(_, path)| {
                let path = std::str::from_utf8(path).ok()?;
                if ImageFormat::from_path(path.as_bytes())? == ImageFormat::Lmp
                    || !(path.starts_with("pics/")
                        || path.starts_with("players/")
                        || path.starts_with("tags/"))
                {
                    return None;
                }
                let (base, _) = path.rsplit_once('.')?;
                Some(format!("{base}.pcx"))
            })
            .collect();
        paths.sort_unstable();
        paths.dedup();
        owner.graphic_images(vfs, font.profile, |images| {
            for path in paths {
                let loaded = images
                    .indexed_pcx(
                        &path,
                        palette,
                        Some(255),
                        None,
                        ImageUse {
                            sampler: bitmap_sampler(),
                            ..ImageUse::pic()
                        },
                    )
                    .map_err(|e| format!("{e:?}"))
                    .and_then(|image| {
                        bitmap_shader(images, &path, image, bitmap_sampler(), None)
                            .map_err(str::to_owned)
                    });
                match loaded {
                    Ok(shader) => result.register_receipt(images.assets, &path, shader)?,
                    Err(error) => {
                        qa_console::logger::console(format_args!("HUD picture {path}: {error}\n"))
                    }
                }
            }
            Ok(())
        })?;
        Ok(result)
    }
    pub(crate) fn registrations(&self) -> impl Iterator<Item = &ShaderRegistration> + Clone {
        self.entries.iter().map(|entry| &entry.1.shader)
    }
    /// Called only during cold registration, never from layout execution.
    pub fn register(
        &mut self,
        assets: &mut Assets,
        path: &str,
        shader: ShaderBindingId,
    ) -> Result<(), String> {
        let registration = assets
            .shader_registration(path.as_bytes(), shader)
            .map_err(str::to_owned)?;
        self.register_receipt(assets, path, registration)
    }
    fn register_receipt(
        &mut self,
        assets: &mut Assets,
        path: &str,
        shader: ShaderRegistration,
    ) -> Result<(), String> {
        let image = assets
            .shader_binding(shader.shader)
            .and_then(|binding| assets.material(binding.material))
            .and_then(|m| m.stages.first())
            .and_then(|stage| {
                if let qa_render::assets::StageTexture::Image(id) = stage.texture {
                    assets.image(id)
                } else {
                    None
                }
            })
            .ok_or("HUD picture needs an image shader")?;
        let size = image.logical_size();
        let picture = Picture { shader, size };
        let name = assets.intern_name(path).map_err(str::to_owned)?;
        match self
            .entries
            .binary_search_by_key(&name.0, |entry| entry.0.0)
        {
            Ok(index) => self.entries[index].1 = picture,
            Err(index) => self.entries.insert(index, (name, picture)),
        }
        for color in 0..2 {
            for digit in 0..11 {
                let mut name = FixedText::<32>::default();
                let prefix = if color == 0 { "num" } else { "anum" };
                if digit == 10 {
                    write!(name, "{prefix}_minus").map_err(|_| "HUD number name")?;
                } else {
                    write!(name, "{prefix}_{digit}").map_err(|_| "HUD number name")?;
                }
                self.numbers[color][digit] = self.find_index(assets, name.as_str());
            }
        }
        Ok(())
    }
    fn find_index(&self, assets: &Assets, name: &str) -> Option<usize> {
        if name.is_empty() {
            return None;
        }
        let mut path = FixedText::<1024>::default();
        if name.starts_with(['/', '\\']) {
            path.set(&name[1..]).ok()?;
        } else {
            write!(path, "pics/{name}.pcx").ok()?;
        }
        let id = assets.find_name(path.as_str())?;
        let index = self
            .entries
            .binary_search_by_key(&id.0, |entry| entry.0.0)
            .ok()?;
        Some(index)
    }
    pub fn picture(
        &self,
        shaders: ShaderView<'_>,
        font: &FontAssets,
        assets: &Assets,
        name: &str,
        frame: &mut Frame,
        x: i32,
        y: i32,
        viewport: Viewport,
        remaining: &mut usize,
        stats: &mut HudStats,
    ) {
        if name.is_empty() {
            return;
        }
        if let Some(index) = self.find_index(assets, name) {
            self.entries[index]
                .1
                .draw(shaders, font, frame, x, y, viewport, remaining, stats);
        } else {
            stats.missing_picture += 1;
        }
    }
    pub fn number(
        &self,
        shaders: ShaderView<'_>,
        font: &FontAssets,
        frame: &mut Frame,
        x: i32,
        y: i32,
        width: i32,
        value: i32,
        alternate: bool,
        viewport: Viewport,
        remaining: &mut usize,
        stats: &mut HudStats,
    ) {
        if width < 1 {
            return;
        }
        let width = width.min(5) as usize;
        let mut text = FixedText::<16>::default();
        if write!(text, "{value}").is_err() {
            stats.rejected += 1;
            return;
        }
        let length = text.as_str().len().min(width);
        let mut x = x.saturating_add(2 + 16 * (width - length) as i32);
        for &digit in &text.as_str().as_bytes()[..length] {
            let digit = if digit == b'-' {
                10
            } else {
                usize::from(digit - b'0')
            };
            if let Some(index) = self.numbers[usize::from(alternate)][digit] {
                self.entries[index]
                    .1
                    .draw(shaders, font, frame, x, y, viewport, remaining, stats);
            } else {
                stats.missing_picture += 1;
            }
            x = x.saturating_add(16);
        }
    }
}
impl Picture {
    fn draw(
        self,
        shaders: ShaderView<'_>,
        font: &FontAssets,
        frame: &mut Frame,
        x: i32,
        y: i32,
        viewport: Viewport,
        remaining: &mut usize,
        stats: &mut HudStats,
    ) {
        let Some(shader) = font.graphic(shaders, &self.shader) else {
            stats.rejected += 1;
            return;
        };
        let Some(draw) = (Draw2d {
            rect: [x as f32, y as f32, self.size[0] as f32, self.size[1] as f32],
            texcoords: [0., 0., 1., 1.],
            color: [255; 4],
            shader,
        })
        .clipped(viewport) else {
            return;
        };
        if *remaining == 0 || !frame.draw_2d(draw) {
            stats.rejected += 1;
        } else {
            *remaining -= 1;
            stats.pictures += 1;
        }
    }
}
