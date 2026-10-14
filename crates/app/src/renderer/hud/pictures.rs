//! Load-time image registration; layout execution uses the shared asset names.
use super::{
    super::console::{FontAssets, bitmap_material, bitmap_sampler},
    HudStats,
};
use qa_content::vfs::Vfs;
use qa_core::{
    primitives::{NameId, RuleSetId},
    text::FixedText,
};
use qa_render::{
    Assets, Draw2d, Frame, MaterialId, Viewport,
    material::resources::{ImageSettings, ImageUse, Images},
};
use std::fmt::Write;

#[derive(Clone, Copy)]
pub struct Picture {
    material: MaterialId,
    size: [u32; 2],
}
#[derive(Default)]
pub struct Pictures {
    entries: Vec<(NameId, Picture)>,
    numbers: [[Option<Picture>; 11]; 2],
}
impl Pictures {
    pub fn load(
        vfs: &Vfs,
        assets: &mut Assets,
        rules: RuleSetId,
        font: FontAssets,
        settings: ImageSettings,
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
                (path.ends_with(".pcx")
                    && (path.starts_with("pics/")
                        || path.starts_with("players/")
                        || path.starts_with("tags/")))
                .then_some(path)
            })
            .collect();
        paths.sort_unstable();
        paths.dedup();
        let mut images = Images::new(vfs, assets, rules, rules, settings, None)
            .map_err(|e| format!("HUD images: {e:?}"))?;
        for path in paths {
            let loaded = images
                .indexed_pcx(
                    path,
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
                    bitmap_material(&mut images, path, image, bitmap_sampler(), None)
                        .map_err(str::to_owned)
                });
            match loaded {
                Ok(material) => result.register(images.assets, path, material)?,
                Err(error) => {
                    qa_console::logger::console(format_args!("HUD picture {path}: {error}\n"))
                }
            }
        }
        Ok(result)
    }
    /// Called only during cold registration, never from layout execution.
    pub fn register(
        &mut self,
        assets: &mut Assets,
        path: &str,
        material: MaterialId,
    ) -> Result<(), String> {
        let image = assets
            .material(material)
            .and_then(|m| m.stages.first())
            .and_then(|stage| {
                if let qa_render::assets::StageTexture::Image(id) = stage.texture {
                    assets.image(id)
                } else {
                    None
                }
            })
            .ok_or("HUD picture needs an image material")?;
        let picture = Picture {
            material,
            size: [image.width, image.height],
        };
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
                self.numbers[color][digit] = self.find(assets, name.as_str());
            }
        }
        Ok(())
    }
    fn find(&self, assets: &Assets, name: &str) -> Option<Picture> {
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
        Some(self.entries[index].1)
    }
    pub fn picture(
        &self,
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
        if let Some(picture) = self.find(assets, name) {
            picture.draw(frame, x, y, viewport, remaining, stats);
        } else {
            stats.missing_picture += 1;
        }
    }
    pub fn number(
        &self,
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
            if let Some(picture) = self.numbers[usize::from(alternate)][digit] {
                picture.draw(frame, x, y, viewport, remaining, stats);
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
        frame: &mut Frame,
        x: i32,
        y: i32,
        viewport: Viewport,
        remaining: &mut usize,
        stats: &mut HudStats,
    ) {
        let Some(draw) = (Draw2d {
            rect: [x as f32, y as f32, self.size[0] as f32, self.size[1] as f32],
            texcoords: [0., 0., 1., 1.],
            color: [255; 4],
            material: self.material,
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
