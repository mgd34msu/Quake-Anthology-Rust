//! Lightmap packing and the native conversion shared by load and GL refresh.

use crate::scene::LightStyle;

pub const PAGE_SIZE: u32 = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LightmapError {
    PageSize,
    PageLimit,
    Dimensions,
    SampleLength { expected: usize, actual: usize },
    Scalar,
    Overbright,
    Size,
    Allocation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AtlasRegion {
    pub page: u32,
    pub page_size: u32,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl AtlasRegion {
    /// Sample zero is the center of the region's first lightmap texel.
    pub fn uv_at_sample(self, sample: [f32; 2]) -> [f32; 2] {
        [
            (self.x as f32 + sample[0] + 0.5) / self.page_size as f32,
            (self.y as f32 + sample[1] + 0.5) / self.page_size as f32,
        ]
    }

    /// GLQuake/ref_gl polygon coordinates: (texture - minimum + atlas*16 + 8)/2048.
    pub fn uv_from_texture(self, texture: [f32; 2], texture_minimum: [i32; 2]) -> [f32; 2] {
        let mut coordinate = texture;
        for (axis, origin) in [self.x, self.y].into_iter().enumerate() {
            coordinate[axis] -= texture_minimum[axis] as f32;
            coordinate[axis] += (origin * 16) as f32;
            coordinate[axis] += 8.0;
            coordinate[axis] /= (self.page_size * 16) as f32;
        }
        coordinate
    }
}

pub struct AtlasPage {
    pub rgb: Box<[u8]>,
}

pub struct Atlas {
    page_size: u32,
    pages: Box<[AtlasPage]>,
}

impl Atlas {
    pub fn page_size(&self) -> u32 {
        self.page_size
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    pub fn page(&self, index: u32) -> Option<&[u8]> {
        self.pages.get(index as usize).map(|page| &*page.rgb)
    }

    pub fn pages(&self) -> &[AtlasPage] {
        &self.pages
    }
}

struct PackingPage {
    columns: Box<[u16]>,
    rgb: Box<[u8]>,
}

pub struct AtlasBuilder {
    page_size: u32,
    pages: Vec<PackingPage>,
    current: Option<usize>,
    max_pages: usize,
}

impl AtlasBuilder {
    pub fn load(page_size: u32, max_pages: u32) -> Result<Self, LightmapError> {
        if crate::assets::upload::validate_dimensions(page_size, page_size).is_err()
            || !page_size.is_power_of_two()
        {
            return Err(LightmapError::PageSize);
        }
        if max_pages == 0 {
            return Err(LightmapError::PageLimit);
        }
        let mut pages = Vec::new();
        pages
            .try_reserve_exact(max_pages as usize)
            .map_err(|_| LightmapError::Allocation)?;
        Ok(Self {
            page_size,
            pages,
            current: None,
            max_pages: max_pages as usize,
        })
    }

    pub fn insert(
        &mut self,
        width: u32,
        height: u32,
        rgb: &[u8],
    ) -> Result<AtlasRegion, LightmapError> {
        // Native LM_AllocBlock excludes x == BLOCK_WIDTH-w.
        if width == 0 || width >= self.page_size || height == 0 || height > self.page_size {
            return Err(LightmapError::Dimensions);
        }
        check_samples(width, height, 1, rgb)?;
        let mut location = self.current.and_then(|page| {
            allocate_columns(&mut self.pages[page].columns, width, height)
                .map(|(x, y)| (page, x, y))
        });
        if location.is_none() {
            let page = self.append_page(&[])?;
            self.current = Some(page);
            // Admitted dimensions guarantee a location on the fresh page.
            let Some((x, y)) = allocate_columns(&mut self.pages[page].columns, width, height)
            else {
                return Err(LightmapError::Dimensions);
            };
            location = Some((page, x, y));
        }
        let Some((page, x, y)) = location else {
            return Err(LightmapError::Dimensions);
        };
        for row in 0..height as usize {
            let from = row * width as usize * 3;
            let to = ((y as usize + row) * self.page_size as usize + x as usize) * 3;
            self.pages[page].rgb[to..to + width as usize * 3]
                .copy_from_slice(&rgb[from..from + width as usize * 3]);
        }
        Ok(AtlasRegion {
            page: page as u32,
            page_size: self.page_size,
            x,
            y,
            width,
            height,
        })
    }

    /// Authored Q3 pages stay intact; subsequent packed grids begin a new page.
    pub fn insert_page(&mut self, rgb: &[u8]) -> Result<AtlasRegion, LightmapError> {
        check_samples(self.page_size, self.page_size, 1, rgb)?;
        let page = self.append_page(rgb)?;
        self.current = None;
        Ok(AtlasRegion {
            page: page as u32,
            page_size: self.page_size,
            x: 0,
            y: 0,
            width: self.page_size,
            height: self.page_size,
        })
    }

    pub fn finish(self) -> Atlas {
        Atlas {
            page_size: self.page_size,
            pages: self
                .pages
                .into_iter()
                .map(|page| AtlasPage { rgb: page.rgb })
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        }
    }

    fn append_page(&mut self, initial: &[u8]) -> Result<usize, LightmapError> {
        if self.pages.len() == self.max_pages {
            return Err(LightmapError::PageLimit);
        }
        let mut pixels = Vec::new();
        let page_bytes = self.page_size as usize * self.page_size as usize * 3;
        pixels
            .try_reserve_exact(page_bytes)
            .map_err(|_| LightmapError::Allocation)?;
        pixels.resize(page_bytes, 0);
        if !initial.is_empty() {
            pixels.copy_from_slice(initial);
        }
        let index = self.pages.len();
        self.pages.push(PackingPage {
            columns: vec![0; self.page_size as usize].into_boxed_slice(),
            rgb: pixels.into_boxed_slice(),
        });
        Ok(index)
    }
}

/// qsrc quake-2/ref_gl/gl_rsurf.c:1408 LM_AllocBlock, including its strict ties.
fn allocate_columns(columns: &mut [u16], width: u32, height: u32) -> Option<(u32, u32)> {
    let page_size = columns.len() as u32;
    let mut best = page_size;
    let mut spot = None;
    for x in 0..page_size - width {
        let mut maximum = 0;
        let mut scanned = 0;
        for &column in &columns[x as usize..(x + width) as usize] {
            if u32::from(column) >= best {
                break;
            }
            maximum = maximum.max(u32::from(column));
            scanned += 1;
        }
        if scanned == width {
            best = maximum;
            spot = Some((x, best));
        }
    }
    if best + height > page_size {
        return None;
    }
    let (x, y) = spot?;
    columns[x as usize..(x + width) as usize].fill((y + height) as u16);
    Some((x, y))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Q1GlScale {
    /// GLQuake gl_rsurf.c:196/213 uses >>7; its stored inverse becomes RGB here.
    OriginalOverbright,
    /// Explicit unit-scale presentation uses the 8.8 divisor without overbright.
    Standard,
}

/// Arithmetic selected once at the lighting boundary, independently of the
/// view's movement, protocol and module identities.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LightmapConversion {
    Scalar(Q1GlScale),
    Rgb { modulate: f32 },
}
impl LightmapConversion {
    pub(crate) fn referenced_styles(
        self,
        ids: [u8; 4],
        styles: &[LightStyle; 256],
    ) -> ([LightStyle; 4], usize) {
        let mut selected = [LightStyle {
            rgb: [0.0; 3],
            indexed_scale: 0,
        }; 4];
        let count = active_style_count(&ids);
        for (output, &id) in selected.iter_mut().zip(&ids[..count]) {
            match self {
                Self::Scalar(_) => output.indexed_scale = styles[id as usize].indexed_scale,
                Self::Rgb { .. } => output.rgb = styles[id as usize].rgb,
            }
        }
        (selected, count)
    }
    pub(crate) fn build_rgb_into(
        self,
        width: u32,
        height: u32,
        samples: &[u8],
        styles: &[LightStyle],
        rgb: &mut [u8],
    ) -> Result<(), LightmapError> {
        if styles.len() > 4 {
            return Err(LightmapError::Dimensions);
        }
        match self {
            Self::Scalar(scale) => {
                let scales: [u32; 4] = std::array::from_fn(|index| {
                    styles.get(index).map_or(0, |s| u32::from(s.indexed_scale))
                });
                build_quake_rgb_into(width, height, samples, &scales[..styles.len()], scale, rgb)
            }
            Self::Rgb { modulate } => {
                let scales: [[f32; 3]; 4] =
                    std::array::from_fn(|index| styles.get(index).map_or([0.0; 3], |s| s.rgb));
                build_quake2_rgb_into(
                    width,
                    height,
                    samples,
                    &scales[..styles.len()],
                    modulate,
                    rgb,
                )
            }
        }
    }
}

fn active_style_count(ids: &[u8; 4]) -> usize {
    ids.iter().take_while(|&&id| id != 255).count()
}

/// GPU atlas residency remembers the weights for each presentation's surfaces.
/// Geometry and samples stay in the world's existing immutable Assets storage.
pub struct AtlasUpdates {
    offsets: Box<[usize]>,
    styles: Box<[Option<[LightStyle; 4]>]>,
    rgb: Box<[u8]>,
    rgba: Box<[u8]>,
}
impl AtlasUpdates {
    /// Assets retain numeric presentation, surface and image prefixes while
    /// cold append preparation extends the same residency owner.
    pub fn load(assets: &crate::Assets) -> Result<Self, &'static str> {
        let mut offsets = Vec::with_capacity(assets.world_presentations().len());
        let mut region_edges = Vec::new();
        let mut count = 0usize;
        let mut bytes = 0usize;
        for presentation in assets.world_presentations() {
            let world = assets
                .world(presentation.world())
                .ok_or("invalid GL styled world")?;
            offsets.push(count);
            count = count
                .checked_add(world.geometry().surfaces.len())
                .ok_or("GL styled surface count overflow")?;
            for (surface, binding) in world
                .geometry()
                .surfaces
                .iter()
                .zip(presentation.bindings())
            {
                let binding = assets
                    .shader_binding(binding.shader)
                    .ok_or("invalid GL styled binding")?;
                if binding.lightmap_conversion.is_none() {
                    continue;
                }
                let Some(region) = binding.lightmap_region else {
                    return Err("GL styled surface has no atlas region");
                };
                if surface.light_source != crate::world::geometry::LightSource::Samples
                    || [region.width, region.height] != surface.lightmap_grid
                {
                    return Err("GL styled surface sample dimensions disagree");
                }
                let samples = world
                    .geometry()
                    .light_samples
                    .get(surface.light_samples.range())
                    .ok_or("GL styled surface has invalid samples")?;
                let active = active_style_count(&surface.styles);
                let required =
                    check_samples(region.width, region.height, active, samples.as_flattened())
                        .map_err(|_| "GL styled surface has invalid samples")?;
                bytes = bytes.max(required);
                // Assets already admitted these rectangle bounds. Track starts
                // and ends; ties retire an old rectangle before admitting the
                // next, so adjacent texels retain independent ownership.
                let right = region.x + region.width;
                let bottom = region.y + region.height;
                region_edges.push((binding.lightmap.0, region.x, true, region.y, bottom));
                region_edges.push((binding.lightmap.0, right, false, region.y, bottom));
            }
        }
        region_edges.sort_unstable();
        let mut current_image = None;
        let mut active = std::collections::BTreeMap::<u32, u32>::new();
        for (image, _, start, top, bottom) in region_edges {
            if current_image != Some(image) {
                active.clear();
                current_image = Some(image);
            }
            if !start {
                active.remove(&top);
                continue;
            }
            if active
                .range(..=top)
                .next_back()
                .is_some_and(|(_, &end)| end > top)
                || active
                    .range(top..)
                    .next()
                    .is_some_and(|(&begin, _)| begin < bottom)
            {
                return Err("GL styled atlas regions overlap");
            }
            active.insert(top, bottom);
        }
        let rgba_bytes = bytes
            .checked_div(3)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or("GL lightmap scratch size overflow")?;
        Ok(Self {
            offsets: offsets.into_boxed_slice(),
            styles: vec![None; count].into_boxed_slice(),
            rgb: vec![0; bytes].into_boxed_slice(),
            rgba: vec![0; rgba_bytes].into_boxed_slice(),
        })
    }

    /// Stage appended presentations without changing any admitted style
    /// residency. Scratch and atlas validation use the initial load path.
    pub fn prepare_append(&self, assets: &crate::Assets) -> Result<Option<Self>, &'static str> {
        let presentations = assets.world_presentations().len();
        if presentations < self.offsets.len() {
            return Err("GL styled presentation prefix shrank");
        }
        if presentations == self.offsets.len() {
            return Ok(None);
        }
        let mut candidate = Self::load(assets)?;
        if candidate.offsets.get(..self.offsets.len()) != Some(self.offsets.as_ref())
            || candidate.offsets.get(self.offsets.len()) != Some(&self.styles.len())
        {
            return Err("GL styled presentation prefix changed");
        }
        candidate
            .styles
            .get_mut(..self.styles.len())
            .ok_or("GL styled surface prefix shrank")?
            .copy_from_slice(&self.styles);
        Ok(Some(candidate))
    }

    /// The upload callback borrows scratch only during this call. A true return
    /// records successful submission; rejection keeps the previous residency.
    pub fn refresh(
        &mut self,
        presentation_id: crate::world::WorldPresentationId,
        surface_id: u32,
        styles: &[LightStyle; 256],
        assets: &crate::Assets,
        upload: impl FnOnce(crate::assets::ImageId, AtlasRegion, &[u8]) -> bool,
    ) -> Result<bool, &'static str> {
        let presentation = assets
            .world_presentation(presentation_id)
            .ok_or("invalid GL styled presentation")?;
        let world = assets
            .world(presentation.world())
            .ok_or("invalid GL styled world")?;
        let surface = world
            .geometry()
            .surfaces
            .get(surface_id as usize)
            .ok_or("invalid GL styled surface")?;
        let binding = assets
            .surface_binding(presentation_id, surface_id)
            .ok_or("invalid GL styled binding")?;
        let binding = assets
            .shader_binding(binding.shader)
            .ok_or("invalid GL styled binding")?;
        let Some(conversion) = binding.lightmap_conversion else {
            return Ok(false);
        };
        let index = self
            .offsets
            .get(presentation_id.0 as usize)
            .and_then(|offset| offset.checked_add(surface_id as usize))
            .ok_or("invalid GL style residency index")?;
        let last = self
            .styles
            .get_mut(index)
            .ok_or("invalid GL style residency index")?;
        let (selected, count) = conversion.referenced_styles(surface.styles, styles);
        if *last == Some(selected) {
            return Ok(false);
        }
        let region = binding
            .lightmap_region
            .ok_or("GL styled surface has no atlas region")?;
        let bytes =
            rgb_bytes(region.width, region.height).map_err(|_| "invalid GL lightmap dimensions")?;
        let rgba_bytes = bytes / 3 * 4;
        let rgb = self
            .rgb
            .get_mut(..bytes)
            .ok_or("GL lightmap scratch exhausted")?;
        let rgba = self
            .rgba
            .get_mut(..rgba_bytes)
            .ok_or("GL lightmap scratch exhausted")?;
        let samples = world
            .geometry()
            .light_samples
            .get(surface.light_samples.range())
            .ok_or("invalid GL styled samples")?;
        conversion
            .build_rgb_into(
                region.width,
                region.height,
                samples.as_flattened(),
                &selected[..count],
                rgb,
            )
            .map_err(|_| "invalid GL lightmap style conversion")?;
        for (source, output) in rgb
            .as_chunks::<3>()
            .0
            .iter()
            .zip(rgba.as_chunks_mut::<4>().0)
        {
            output.copy_from_slice(&[source[0], source[1], source[2], 255]);
        }
        if !upload(binding.lightmap, region, rgba) {
            return Err("GL lightmap upload rejected");
        }
        *last = Some(selected);
        Ok(true)
    }
}

/// Lit style grids are RGB-normalized at their file boundary, in style-major order.
/// Missing lighting/fullbright surfaces are prepared by the caller, independently.
pub fn build_quake_rgb(
    width: u32,
    height: u32,
    style_major_rgb: &[u8],
    style_8_8: &[u32],
    scale: Q1GlScale,
) -> Result<Vec<u8>, LightmapError> {
    let bytes = check_samples(width, height, style_8_8.len(), style_major_rgb)?;
    let mut rgb = vec![0; bytes];
    build_quake_rgb_into(width, height, style_major_rgb, style_8_8, scale, &mut rgb)?;
    Ok(rgb)
}

pub fn build_quake_rgb_into(
    width: u32,
    height: u32,
    style_major_rgb: &[u8],
    style_8_8: &[u32],
    scale: Q1GlScale,
    rgb: &mut [u8],
) -> Result<(), LightmapError> {
    let bytes = check_samples(width, height, style_8_8.len(), style_major_rgb)?;
    check_output(bytes, rgb)?;
    let shift = match scale {
        Q1GlScale::OriginalOverbright => 7,
        Q1GlScale::Standard => 8,
    };
    for (component, output) in rgb.iter_mut().enumerate() {
        let mut accumulated = 0u32;
        for (style, &weight) in style_8_8.iter().enumerate() {
            accumulated = accumulated.wrapping_add(
                u32::from(style_major_rgb[style * bytes + component]).wrapping_mul(weight),
            );
        }
        *output = (accumulated >> shift).min(255) as u8;
    }
    Ok(())
}

/// Native ref_gl/gl_light.c:503-568, 590-635; RGB styles and modulate precede normalization.
pub fn build_quake2_rgb(
    width: u32,
    height: u32,
    style_major_rgb: &[u8],
    styles: &[[f32; 3]],
    modulate: f32,
) -> Result<Vec<u8>, LightmapError> {
    let bytes = check_samples(width, height, styles.len(), style_major_rgb)?;
    let mut rgb = vec![0; bytes];
    build_quake2_rgb_into(width, height, style_major_rgb, styles, modulate, &mut rgb)?;
    Ok(rgb)
}

pub fn build_quake2_rgb_into(
    width: u32,
    height: u32,
    style_major_rgb: &[u8],
    styles: &[[f32; 3]],
    modulate: f32,
    rgb: &mut [u8],
) -> Result<(), LightmapError> {
    let bytes = check_samples(width, height, styles.len(), style_major_rgb)?;
    check_output(bytes, rgb)?;
    if !modulate.is_finite() || styles.iter().flatten().any(|value| !value.is_finite()) {
        return Err(LightmapError::Scalar);
    }
    for pixel in 0..bytes / 3 {
        let mut accumulated = [0.0f32; 3];
        for (style, weights) in styles.iter().enumerate() {
            for component in 0..3 {
                let weight = modulate * weights[component];
                accumulated[component] +=
                    f32::from(style_major_rgb[style * bytes + pixel * 3 + component]) * weight;
            }
        }
        if accumulated
            .iter()
            .any(|&value| !value.is_finite() || value >= 2147483648.0 || value < -2147483648.0)
        {
            return Err(LightmapError::Scalar);
        }
        // Linux q_shared.h:151 Q_ftol is truncation; normalize after integer conversion.
        let mut channels = accumulated.map(|value| (value as i32).max(0));
        let maximum = channels[0].max(channels[1]).max(channels[2]);
        if maximum > 255 {
            let factor = 255.0f32 / maximum as f32;
            channels = channels.map(|value| (value as f32 * factor) as i32);
        }
        for component in 0..3 {
            rgb[pixel * 3 + component] = channels[component] as u8;
        }
    }
    Ok(())
}

fn check_output(bytes: usize, output: &[u8]) -> Result<(), LightmapError> {
    if output.len() != bytes {
        return Err(LightmapError::SampleLength {
            expected: bytes,
            actual: output.len(),
        });
    }
    Ok(())
}

/// Native Q3 tr_bsp.c:100-125 shifts by map bits minus renderer bits, then normalizes.
#[expect(
    clippy::chunks_exact_to_as_chunks,
    reason = "Native R_ColorShiftLightingBytes operates on ordered RGB byte triplets; the boundary rejects incomplete pixels."
)]
pub fn shift_quake3_rgb(
    rgb: &[u8],
    map_overbright: u8,
    renderer_overbright: u8,
) -> Result<Vec<u8>, LightmapError> {
    if !rgb.len().is_multiple_of(3) {
        return Err(LightmapError::SampleLength {
            expected: rgb.len().div_ceil(3) * 3,
            actual: rgb.len(),
        });
    }
    let shift = map_overbright
        .checked_sub(renderer_overbright)
        .ok_or(LightmapError::Overbright)?;
    if shift > 8 {
        return Err(LightmapError::Overbright);
    }
    let mut output = vec![0; rgb.len()];
    for (pixel, destination) in rgb.chunks_exact(3).zip(output.chunks_exact_mut(3)) {
        destination.copy_from_slice(&color_shift_rgb(
            [pixel[0], pixel[1], pixel[2]],
            shift as i8,
        ));
    }
    Ok(output)
}

/// Q3 R_ColorShiftLightingBytes, before patch interpolation or atlas upload.
pub(crate) fn color_shift_rgb(color: [u8; 3], shift: i8) -> [u8; 3] {
    let mut channels = color.map(|value| {
        if shift >= 0 {
            u32::from(value) << shift as u32
        } else {
            u32::from(value) >> shift.unsigned_abs() as u32
        }
    });
    let maximum = channels[0].max(channels[1]).max(channels[2]);
    if maximum > 255 {
        channels = channels.map(|value| value * 255 / maximum);
    }
    channels.map(|value| value as u8)
}

fn check_samples(
    width: u32,
    height: u32,
    styles: usize,
    samples: &[u8],
) -> Result<usize, LightmapError> {
    let bytes = rgb_bytes(width, height)?;
    let expected = bytes.checked_mul(styles).ok_or(LightmapError::Size)?;
    if samples.len() != expected {
        return Err(LightmapError::SampleLength {
            expected,
            actual: samples.len(),
        });
    }
    Ok(bytes)
}

pub(crate) fn rgb_bytes(width: u32, height: u32) -> Result<usize, LightmapError> {
    if width == 0 || height == 0 {
        return Err(LightmapError::Dimensions);
    }
    let bytes = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(3))
        .ok_or(LightmapError::Size)?;
    Ok(bytes)
}
