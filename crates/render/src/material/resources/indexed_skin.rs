use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IndexedSkinLayout {
    pub storage: Option<[u32; 2]>,
    pub padding: u8,
    pub omit_last_row: bool,
}

/// Accepted source bytes, independent of names, palettes and player colors.
#[derive(Debug)]
pub struct IndexedSkinSource {
    decoded: [u32; 2],
    layout: IndexedSkinLayout,
    indices: Box<[u8]>,
}
impl IndexedSkinSource {
    pub fn decoded_extent(&self) -> [u32; 2] {
        self.decoded
    }
    pub fn storage_extent(&self) -> [u32; 2] {
        self.layout.storage.unwrap_or(self.decoded)
    }
    pub fn indices(&self) -> &[u8] {
        &self.indices
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndexedSkinTranslation {
    pub palette: PaletteId,
    pub target: Option<PaletteId>,
    pub indexed_colors: Option<[u8; 2]>,
    pub rgba_colors: Option<[u8; 2]>,
    pub crop: Option<[u32; 4]>,
    pub coordinate_extent: [u32; 2],
    pub upload_extent: Option<[u32; 2]>,
    pub usage: ImageUse,
}

impl Images<'_> {
    pub fn indexed_skin_source(
        &mut self,
        name: &str,
        layout: IndexedSkinLayout,
    ) -> Result<IndexedSkinSource, ResourceError> {
        let texture = self.native_texture(name, SourceFormat::Raster(ImageFormat::Pcx), None)?;
        let base = texture
            .mip(0)
            .ok_or(ResourceError::Asset("missing indexed skin base"))?;
        let decoded = [base.width, base.height];
        let [width, height] = layout.storage.unwrap_or(decoded);
        upload::validate_dimensions(width, height)?;
        if base.width > width || base.height > height {
            return Err(ResourceError::Size);
        }
        let mut indices = vec![layout.padding; width as usize * height as usize];
        let rows = base.height - u32::from(layout.omit_last_row);
        for row in 0..rows as usize {
            let source = row * base.width as usize;
            let target = row * width as usize;
            indices[target..target + base.width as usize]
                .copy_from_slice(&base.indices()[source..source + base.width as usize]);
        }
        Ok(IndexedSkinSource {
            decoded,
            layout,
            indices: indices.into_boxed_slice(),
        })
    }

    pub fn translated_skin(
        &mut self,
        name: &str,
        source: &IndexedSkinSource,
        settings: IndexedSkinTranslation,
    ) -> Result<ResolvedImage, ResourceError> {
        let key = ImageKey {
            name: self
                .assets
                .intern_name(name)
                .map_err(ResourceError::Asset)?,
            recipe: ImageRecipe::TranslatedSkin {
                decoded: source.decoded,
                layout: source.layout,
                settings,
            },
        };
        if let Some(index) = self.find(key) {
            return Ok(self.state.resolved[index].image);
        }
        let [width, height] = source.storage_extent();
        let [x, y, crop_width, crop_height] = settings.crop.unwrap_or([0, 0, width, height]);
        upload::validate_dimensions(crop_width, crop_height)?;
        if x.checked_add(crop_width).is_none_or(|end| end > width)
            || y.checked_add(crop_height).is_none_or(|end| end > height)
        {
            return Err(ResourceError::Size);
        }
        let indexed_map = settings
            .indexed_colors
            .map(|[top, bottom]| qa_formats::image::player_translation(top, bottom))
            .transpose()?;
        let rgba_map = settings
            .rgba_colors
            .map(|[top, bottom]| qa_formats::image::player_translation(top, bottom))
            .transpose()?;
        let mut params = self.upload_params(settings.usage)?;
        if let Some([width, height]) = settings.upload_extent {
            upload::validate_dimensions(width, height)?;
            params.extent = upload::UploadExtent::Exact { width, height };
            params.resize = upload::ResizeFilter::Nearest;
        }
        self.try_registration(|images| -> Result<ResolvedImage, ResourceError> {
            let translated: Vec<_> = source
                .indices
                .iter()
                .map(|&index| {
                    indexed_map
                        .as_ref()
                        .map_or(index, |map| map[index as usize])
                })
                .collect();
            let texture = IndexedTexture::load_base(width, height, &translated, None)
                .map_err(ResourceError::Asset)?
                .with_coordinate_extent(settings.coordinate_extent)
                .map_err(ResourceError::Asset)?;
            let mut crop = Vec::with_capacity(crop_width as usize * crop_height as usize);
            for row in y..y + crop_height {
                let start = row as usize * width as usize + x as usize;
                crop.extend_from_slice(&source.indices[start..start + crop_width as usize]);
            }
            let colors = images
                .assets
                .palette(settings.palette)
                .ok_or(ResourceError::Asset("missing source skin palette"))?;
            let rgba = IndexedImage {
                width: crop_width,
                height: crop_height,
                indices: std::borrow::Cow::Borrowed(&crop),
                palette: None,
            }
            .expand(
                &Palette(Box::new(colors.rgba_table())),
                &PaletteOptions {
                    translation: rgba_map.as_ref(),
                    ..Default::default()
                },
            )?;
            let id = images.register_indexed_rgba_to(
                crop_width,
                crop_height,
                &rgba.pixels,
                texture,
                settings.palette,
                settings.target,
            )?;
            images
                .assets
                .prepare_image(id, params)
                .map_err(ResourceError::Asset)?;
            if images.state.recipes.rules == RuleSetId::Quake3 {
                images
                    .assets
                    .set_image_native_sampler(
                        id,
                        images.state.recipes.effective_use(settings.usage).sampler,
                    )
                    .map_err(ResourceError::Asset)?;
            }
            Ok(images.remember(key, key.name, id, settings.usage))
        })
    }
}
