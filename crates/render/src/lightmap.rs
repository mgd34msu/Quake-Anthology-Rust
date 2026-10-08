//! Load-time lightmap packing and native GL lighting conversions.

pub const PAGE_SIZE: u32 = 128;
const PAGE_BYTES: usize = PAGE_SIZE as usize * PAGE_SIZE as usize * 3;

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
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl AtlasRegion {
    /// Sample zero is the center of the region's first lightmap texel.
    pub fn uv_at_sample(self, sample: [f32; 2]) -> [f32; 2] {
        [
            (self.x as f32 + sample[0] + 0.5) / PAGE_SIZE as f32,
            (self.y as f32 + sample[1] + 0.5) / PAGE_SIZE as f32,
        ]
    }

    /// GLQuake/ref_gl polygon coordinates: (texture - minimum + atlas*16 + 8)/2048.
    pub fn uv_from_texture(self, texture: [f32; 2], texture_minimum: [i32; 2]) -> [f32; 2] {
        let mut coordinate = texture;
        for (axis, origin) in [self.x, self.y].into_iter().enumerate() {
            coordinate[axis] -= texture_minimum[axis] as f32;
            coordinate[axis] += (origin * 16) as f32;
            coordinate[axis] += 8.0;
            coordinate[axis] /= (PAGE_SIZE * 16) as f32;
        }
        coordinate
    }
}

pub struct AtlasPage {
    pub rgb: Box<[u8]>,
}

pub struct Atlas {
    pages: Box<[AtlasPage]>,
}

impl Atlas {
    pub fn page_size(&self) -> u32 {
        PAGE_SIZE
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
    columns: [u16; PAGE_SIZE as usize],
    rgb: Box<[u8]>,
}

pub struct AtlasBuilder {
    pages: Vec<PackingPage>,
    current: Option<usize>,
    max_pages: usize,
}

impl AtlasBuilder {
    pub fn load(page_size: u32, max_pages: u32) -> Result<Self, LightmapError> {
        if page_size != PAGE_SIZE {
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
        // Native LM_AllocBlock excludes x == BLOCK_WIDTH-w, including w == 128.
        if width == 0 || width >= PAGE_SIZE || height == 0 || height > PAGE_SIZE {
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
            // Width < 128 and height <= 128 guarantee a location on the fresh page.
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
            let to = ((y as usize + row) * PAGE_SIZE as usize + x as usize) * 3;
            self.pages[page].rgb[to..to + width as usize * 3]
                .copy_from_slice(&rgb[from..from + width as usize * 3]);
        }
        Ok(AtlasRegion {
            page: page as u32,
            x,
            y,
            width,
            height,
        })
    }

    /// Authored Q3 pages stay intact; subsequent packed grids begin a new page.
    pub fn insert_page(&mut self, rgb: &[u8]) -> Result<AtlasRegion, LightmapError> {
        check_samples(PAGE_SIZE, PAGE_SIZE, 1, rgb)?;
        let page = self.append_page(rgb)?;
        self.current = None;
        Ok(AtlasRegion {
            page: page as u32,
            x: 0,
            y: 0,
            width: PAGE_SIZE,
            height: PAGE_SIZE,
        })
    }

    pub fn finish(self) -> Atlas {
        Atlas {
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
        pixels
            .try_reserve_exact(PAGE_BYTES)
            .map_err(|_| LightmapError::Allocation)?;
        pixels.resize(PAGE_BYTES, 0);
        if !initial.is_empty() {
            pixels.copy_from_slice(initial);
        }
        let index = self.pages.len();
        self.pages.push(PackingPage {
            columns: [0; PAGE_SIZE as usize],
            rgb: pixels.into_boxed_slice(),
        });
        Ok(index)
    }
}

/// qsrc quake-2/ref_gl/gl_rsurf.c:1408 LM_AllocBlock, including its strict ties.
fn allocate_columns(
    columns: &mut [u16; PAGE_SIZE as usize],
    width: u32,
    height: u32,
) -> Option<(u32, u32)> {
    let mut best = PAGE_SIZE;
    let mut spot = None;
    for x in 0..PAGE_SIZE - width {
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
    if best + height > PAGE_SIZE {
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
    let shift = match scale {
        Q1GlScale::OriginalOverbright => 7,
        Q1GlScale::Standard => 8,
    };
    let mut rgb = vec![0; bytes];
    for (component, output) in rgb.iter_mut().enumerate() {
        let mut accumulated = 0u32;
        for (style, &weight) in style_8_8.iter().enumerate() {
            accumulated = accumulated.wrapping_add(
                u32::from(style_major_rgb[style * bytes + component]).wrapping_mul(weight),
            );
        }
        *output = (accumulated >> shift).min(255) as u8;
    }
    Ok(rgb)
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
    if !modulate.is_finite() || styles.iter().flatten().any(|value| !value.is_finite()) {
        return Err(LightmapError::Scalar);
    }
    let mut rgb = vec![0; bytes];
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
    Ok(rgb)
}

/// Native Q3 tr_bsp.c:100-125 shifts by map bits minus renderer bits, then normalizes.
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
        let mut channels = [
            u32::from(pixel[0]) << shift,
            u32::from(pixel[1]) << shift,
            u32::from(pixel[2]) << shift,
        ];
        let maximum = channels[0].max(channels[1]).max(channels[2]);
        if maximum > 255 {
            channels = channels.map(|value| value * 255 / maximum);
        }
        destination.copy_from_slice(&channels.map(|value| value as u8));
    }
    Ok(output)
}

fn check_samples(
    width: u32,
    height: u32,
    styles: usize,
    samples: &[u8],
) -> Result<usize, LightmapError> {
    if width == 0 || height == 0 {
        return Err(LightmapError::Dimensions);
    }
    let bytes = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(3))
        .ok_or(LightmapError::Size)?;
    let expected = bytes.checked_mul(styles).ok_or(LightmapError::Size)?;
    if samples.len() != expected {
        return Err(LightmapError::SampleLength {
            expected,
            actual: samples.len(),
        });
    }
    Ok(bytes)
}
