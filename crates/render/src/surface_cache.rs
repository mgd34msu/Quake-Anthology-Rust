//! Indexed texture × lightmap surfaces, using the native Q1/Q2 block math.
//!
//! References: WinQuake/d_surf.c D_SCAlloc/D_CacheSurface; WinQuake/r_surf.c
//! R_BuildLightMap and R_DrawSurfaceBlock8_mip0..3; Q2 ref_soft/r_light.c and
//! r_surf.c. Native Q2 RGB conversion preserves r_model.c's strict comparisons,
//! including its blue result on an R/G tie. True maximum is a separate policy.
//! The C port's pin-aware rover fix prevents eviction before pending spans end.
//! Fullbright colors are encoded by the supplied colormap rows; the cutoff
//! metadata does not bypass that table. Only explicit fence cutouts skip it.

use std::sync::atomic::{AtomicU64, Ordering};

const COLORMAP_BYTES: usize = 64 * 256;
const ALPHAMAP_BYTES: usize = 256 * 256;
const MAX_DIMENSION: u32 = 8192;
static NEXT_RESOURCE: AtomicU64 = AtomicU64::new(1);

fn resource_id() -> Result<u64, &'static str> {
    NEXT_RESOURCE
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
        .map_err(|_| "indexed resource ids exhausted")
}

/// Palette presentation is selected independently of map and movement rules.
pub struct PaletteLighting {
    id: u64,
    colors: Box<[u32]>,
    colormap: Box<[u8]>,
    translucency: Option<Box<[u8]>>,
    first_fullbright: u16,
}

impl PaletteLighting {
    pub fn load(
        rgb: &[u8],
        colormap: &[u8],
        translucency: Option<&[u8]>,
        first_fullbright: u16,
    ) -> Result<Self, &'static str> {
        if rgb.len() != 256 * 3
            || colormap.len() != COLORMAP_BYTES
            || translucency.is_some_and(|map| map.len() != ALPHAMAP_BYTES)
            || first_fullbright > 256
        {
            return Err("invalid indexed palette resources");
        }
        let colors = rgb
            .as_chunks::<3>()
            .0
            .iter()
            .map(|&[r, g, b]| u32::from_le_bytes([r, g, b, 255]))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Ok(Self {
            id: resource_id()?,
            colors,
            colormap: colormap.into(),
            translucency: translucency.map(Into::into),
            first_fullbright,
        })
    }

    pub fn color(&self, index: u8) -> u32 {
        self.colors[index as usize]
    }

    pub fn colormap(&self) -> &[u8] {
        &self.colormap
    }

    pub fn translucency_map(&self) -> Option<&[u8]> {
        self.translucency.as_deref()
    }

    pub fn first_fullbright(&self) -> u16 {
        self.first_fullbright
    }
}

pub struct IndexedMip {
    pub width: u32,
    pub height: u32,
    indices: Box<[u8]>,
}

impl IndexedMip {
    pub fn indices(&self) -> &[u8] {
        &self.indices
    }
}

pub struct IndexedTexture {
    id: u64,
    mips: [Option<IndexedMip>; 4],
    transparent_index: Option<u8>,
}

impl IndexedTexture {
    /// Preserve disk mips, including each mip's own index-255 cutout mask.
    pub fn load(
        width: u32,
        height: u32,
        mips: [&[u8]; 4],
        cutout: bool,
    ) -> Result<Self, &'static str> {
        Self::load_masked(width, height, mips, cutout.then_some(255))
    }

    /// Native fences use 255; a layered sky's front cloud mask uses zero.
    pub fn load_masked(
        width: u32,
        height: u32,
        mips: [&[u8]; 4],
        transparent_index: Option<u8>,
    ) -> Result<Self, &'static str> {
        if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
            return Err("invalid indexed texture dimensions");
        }
        for (mip, indices) in mips.iter().enumerate() {
            let w = (width >> mip).max(1) as usize;
            let h = (height >> mip).max(1) as usize;
            if indices.len() != w * h {
                return Err("invalid indexed texture mip");
            }
        }
        Ok(Self {
            id: resource_id()?,
            mips: std::array::from_fn(|mip| {
                Some(IndexedMip {
                    width: (width >> mip).max(1),
                    height: (height >> mip).max(1),
                    indices: mips[mip].into(),
                })
            }),
            transparent_index,
        })
    }

    /// PCX skies provide one original base image, without disk mip levels.
    pub fn load_base(
        width: u32,
        height: u32,
        indices: &[u8],
        transparent_index: Option<u8>,
    ) -> Result<Self, &'static str> {
        if width == 0
            || height == 0
            || width > MAX_DIMENSION
            || height > MAX_DIMENSION
            || indices.len() != width as usize * height as usize
        {
            return Err("invalid indexed base image dimensions");
        }
        Ok(Self {
            id: resource_id()?,
            mips: [
                Some(IndexedMip {
                    width,
                    height,
                    indices: indices.into(),
                }),
                None,
                None,
                None,
            ],
            transparent_index,
        })
    }
    pub fn mip(&self, mip: u8) -> Option<&IndexedMip> {
        self.mips.get(mip as usize)?.as_ref()
    }

    pub fn cutout(&self) -> bool {
        self.transparent_index.is_some()
    }
    pub fn transparent_index(&self) -> Option<u8> {
        self.transparent_index
    }
}

/// Samples are style-major, then row-major RGB. Gray file data is copied into
/// all channels at load; Q2 disk RGB and Q3 page subregions retain their RGB.
pub struct LightGrid {
    width: u32,
    height: u32,
    styles: [u8; 4],
    style_count: usize,
    samples: Box<[[u8; 3]]>,
}

impl LightGrid {
    pub fn gray(
        width: u32,
        height: u32,
        styles: [u8; 4],
        samples: &[u8],
    ) -> Result<Self, &'static str> {
        let (cells, style_count) = grid_shape(width, height, styles)?;
        if samples.len() != cells * style_count {
            return Err("invalid gray lightmap samples");
        }
        Ok(Self {
            width,
            height,
            styles,
            style_count,
            samples: samples
                .iter()
                .map(|&value| [value; 3])
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        })
    }

    pub fn rgb(
        width: u32,
        height: u32,
        styles: [u8; 4],
        samples: &[u8],
    ) -> Result<Self, &'static str> {
        let (cells, style_count) = grid_shape(width, height, styles)?;
        if samples.len() != cells * style_count * 3 {
            return Err("invalid RGB lightmap samples");
        }
        Ok(Self {
            width,
            height,
            styles,
            style_count,
            samples: samples.as_chunks::<3>().0.into(),
        })
    }

    pub fn rgb_page(
        page_width: u32,
        page_height: u32,
        page: &[u8],
        origin: [u32; 2],
        size: [u32; 2],
        style: u8,
    ) -> Result<Self, &'static str> {
        let (cells, style_count) = grid_shape(size[0], size[1], [style, 255, 255, 255])?;
        if style_count != 1
            || page_width == 0
            || page_height == 0
            || page_width > MAX_DIMENSION
            || page_height > MAX_DIMENSION
            || page.len() != page_width as usize * page_height as usize * 3
            || origin[0] as u64 + size[0] as u64 > page_width as u64
            || origin[1] as u64 + size[1] as u64 > page_height as u64
        {
            return Err("invalid RGB lightmap page region");
        }
        let mut samples = Vec::with_capacity(cells);
        for y in origin[1]..origin[1] + size[1] {
            let start = (y as usize * page_width as usize + origin[0] as usize) * 3;
            let row = &page[start..start + size[0] as usize * 3];
            samples.extend_from_slice(row.as_chunks::<3>().0);
        }
        Ok(Self {
            width: size[0],
            height: size[1],
            styles: [style, 255, 255, 255],
            style_count,
            samples: samples.into_boxed_slice(),
        })
    }

    pub fn dimensions(&self) -> [u32; 2] {
        [self.width, self.height]
    }

    pub fn styles(&self) -> [u8; 4] {
        self.styles
    }

    pub fn samples(&self) -> &[[u8; 3]] {
        &self.samples
    }
}

fn grid_shape(width: u32, height: u32, styles: [u8; 4]) -> Result<(usize, usize), &'static str> {
    if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err("invalid lightmap grid dimensions");
    }
    let count = styles.iter().position(|&s| s == 255).unwrap_or(4);
    if styles[count..].iter().any(|&s| s != 255) {
        return Err("invalid lightmap style sequence");
    }
    Ok((width as usize * height as usize, count))
}

pub struct SurfaceSource {
    texture_mins: [i32; 2],
    extents: [u32; 2],
    lightmap: Option<LightGrid>,
    world_has_lightdata: bool,
}

impl SurfaceSource {
    pub fn load(
        texture_mins: [i32; 2],
        extents: [u32; 2],
        lightmap: Option<LightGrid>,
        world_has_lightdata: bool,
    ) -> Result<Self, &'static str> {
        if extents
            .iter()
            .any(|&e| e == 0 || e > MAX_DIMENSION || !e.is_multiple_of(16))
            || lightmap
                .as_ref()
                .is_some_and(|grid| grid.dimensions() != [extents[0] / 16 + 1, extents[1] / 16 + 1])
        {
            return Err("invalid indexed surface lightmap layout");
        }
        Ok(Self {
            texture_mins,
            extents,
            lightmap,
            world_has_lightdata,
        })
    }

    pub fn texture_mins(&self) -> [i32; 2] {
        self.texture_mins
    }

    pub fn extents(&self) -> [u32; 2] {
        self.extents
    }

    pub fn lightmap(&self) -> Option<&LightGrid> {
        self.lightmap.as_ref()
    }

    fn grid_cells(&self) -> usize {
        (self.extents[0] as usize / 16 + 1) * (self.extents[1] as usize / 16 + 1)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IndexedLighting {
    /// Original scalar Q1 grids are replicated into RGB; select that scalar.
    #[default]
    Gray,
    /// Literal Q2 r_model.c conversion; ties without a strict R/G winner use B.
    NativeRgb,
    /// Q2 software gray: max(R,G,B) per disk sample, before style scaling.
    BrightestRgb,
}

/// Style scales have already been converted at the presentation boundary:
/// Q1 letter*22/empty=256; Q2 lightstyle.white*128. Dynamic values are signed
/// additive 8.8 scalar light on this surface's sample grid.
#[derive(Clone, Copy)]
pub struct BuildState<'a> {
    pub texture_id: u32,
    pub texture_generation: u64,
    pub palette_generation: u64,
    pub style_scales: [u16; 4],
    pub dynamic_generation: u64,
    pub dynamic: Option<&'a [i32]>,
    pub fullbright: bool,
    pub ambient: u8,
    pub lighting: IndexedLighting,
}

impl Default for BuildState<'_> {
    fn default() -> Self {
        Self {
            texture_id: 0,
            texture_generation: 0,
            palette_generation: 0,
            style_scales: [256; 4],
            dynamic_generation: 0,
            dynamic: None,
            fullbright: false,
            ambient: 0,
            lighting: IndexedLighting::Gray,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub hits: u64,
    pub fills: u64,
    pub evictions: u64,
    pub rejected: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct CacheSpan {
    pub width: u32,
    pub height: u32,
    pub mip: u8,
    pub texture_mins: [i32; 2],
    pub cutout: bool,
    pub transparent_index: Option<u8>,
    cache_id: u64,
    block: usize,
    generation: u64,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Stamp {
    texture_resource: u64,
    palette_resource: u64,
    texture_id: u32,
    texture_generation: u64,
    palette_generation: u64,
    style_scales: [u16; 4],
    dynamic_generation: u64,
    dynamic: bool,
    fullbright: bool,
    ambient: u8,
    lighting: IndexedLighting,
    cutout: bool,
}

impl Stamp {
    fn from_state(
        state: BuildState<'_>,
        texture: &IndexedTexture,
        palette: &PaletteLighting,
    ) -> Self {
        Self {
            texture_resource: texture.id,
            palette_resource: palette.id,
            texture_id: state.texture_id,
            texture_generation: state.texture_generation,
            palette_generation: state.palette_generation,
            style_scales: state.style_scales,
            dynamic_generation: state.dynamic_generation,
            dynamic: state.dynamic.is_some(),
            fullbright: state.fullbright,
            ambient: state.ambient,
            lighting: state.lighting,
            cutout: texture.cutout(),
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Slot {
    block: Option<usize>,
    stamp: Stamp,
}

#[derive(Clone, Copy, Default)]
struct Block {
    offset: usize,
    bytes: usize,
    next: Option<usize>,
    owner: Option<usize>,
    generation: u64,
    pin_batch: u64,
}

pub struct SurfaceCache {
    id: u64,
    surfaces: Box<[SurfaceSource]>,
    slots: Box<[Slot]>,
    arena: Box<[u8]>,
    blocks: Box<[Block]>,
    free_metadata: Option<usize>,
    rover: usize,
    light_scratch: Box<[i32]>,
    batch: u64,
    batch_active: bool,
    generation: u64,
    stats: CacheStats,
}

impl SurfaceCache {
    pub fn load(surfaces: Vec<SurfaceSource>, arena_bytes: usize) -> Result<Self, &'static str> {
        let slot_count = surfaces
            .len()
            .checked_mul(4)
            .ok_or("surface slot count overflow")?;
        let block_count = slot_count
            .checked_mul(2)
            .and_then(|count| count.checked_add(1))
            .ok_or("surface block count overflow")?;
        if surfaces.len() > u32::MAX as usize || arena_bytes == 0 {
            return Err("invalid surface cache capacity");
        }
        let scratch_cells = surfaces
            .iter()
            .map(SurfaceSource::grid_cells)
            .max()
            .unwrap_or(0);
        let mut blocks = vec![Block::default(); block_count].into_boxed_slice();
        blocks[0].bytes = arena_bytes;
        for (index, block) in blocks.iter_mut().enumerate().skip(1) {
            block.next = (index + 1 < block_count).then_some(index + 1);
        }
        Ok(Self {
            id: resource_id()?,
            surfaces: surfaces.into_boxed_slice(),
            slots: vec![Slot::default(); slot_count].into_boxed_slice(),
            arena: vec![0; arena_bytes].into_boxed_slice(),
            blocks,
            free_metadata: (block_count > 1).then_some(1),
            rover: 0,
            light_scratch: vec![0; scratch_cells].into_boxed_slice(),
            batch: 0,
            batch_active: false,
            generation: 0,
            stats: CacheStats::default(),
        })
    }

    pub fn surface(&self, surface: u32) -> Option<&SurfaceSource> {
        self.surfaces.get(surface as usize)
    }

    pub fn stats(&self) -> CacheStats {
        self.stats
    }

    /// Every returned span remains pinned until end_batch. Begin does not
    /// discard an unfinished batch; callers flush bounded span work first.
    pub fn begin_batch(&mut self) -> bool {
        if self.batch_active {
            self.stats.rejected = self.stats.rejected.saturating_add(1);
            return false;
        }
        self.batch = self.batch.wrapping_add(1);
        if self.batch == 0 {
            for block in &mut self.blocks {
                block.pin_batch = 0;
            }
            self.batch = 1;
        }
        self.batch_active = true;
        true
    }

    pub fn end_batch(&mut self) {
        self.batch_active = false;
    }

    pub fn prepare(
        &mut self,
        surface: u32,
        mip: u8,
        texture: &IndexedTexture,
        palette: &PaletteLighting,
        state: BuildState<'_>,
    ) -> Option<CacheSpan> {
        let Some(source) = self.surfaces.get(surface as usize) else {
            self.stats.rejected = self.stats.rejected.saturating_add(1);
            return None;
        };
        if mip >= 4
            || state
                .dynamic
                .is_some_and(|grid| grid.len() != source.grid_cells())
        {
            self.stats.rejected = self.stats.rejected.saturating_add(1);
            return None;
        }
        let Some(texture_level) = texture.mip(mip) else {
            self.stats.rejected = self.stats.rejected.saturating_add(1);
            return None;
        };
        let width = source.extents[0] >> mip;
        let height = source.extents[1] >> mip;
        let bytes = width as usize * height as usize;
        let key = surface as usize * 4 + mip as usize;
        let stamp = Stamp::from_state(state, texture, palette);
        let block = if let Some(block) = self.slots[key].block {
            if self.slots[key].stamp == stamp {
                self.pin(block);
                self.stats.hits = self.stats.hits.saturating_add(1);
                return Some(self.span(surface as usize, mip, block, texture.transparent_index));
            }
            if self.pinned(block) {
                self.stats.rejected = self.stats.rejected.saturating_add(1);
                return None;
            }
            block
        } else {
            let Some(block) = self.allocate(bytes) else {
                self.stats.rejected = self.stats.rejected.saturating_add(1);
                return None;
            };
            self.blocks[block].owner = Some(key);
            self.slots[key].block = Some(block);
            block
        };
        self.generation = self.generation.wrapping_add(1);
        self.blocks[block].generation = self.generation;
        self.slots[key].stamp = stamp;
        let source = &self.surfaces[surface as usize];
        build_lightmap(source, state, &mut self.light_scratch);
        let offset = self.blocks[block].offset;
        fill_surface(
            source,
            mip,
            texture_level,
            texture.transparent_index,
            palette,
            &self.light_scratch,
            &mut self.arena[offset..offset + bytes],
        );
        self.pin(block);
        self.stats.fills = self.stats.fills.saturating_add(1);
        Some(self.span(surface as usize, mip, block, texture.transparent_index))
    }

    /// A generation check rejects a span whose block has since been reused.
    pub fn pixels(&self, span: CacheSpan) -> Option<&[u8]> {
        if span.cache_id != self.id {
            return None;
        }
        let block = self.blocks.get(span.block)?;
        let key = block.owner?;
        if block.generation != span.generation
            || self.slots[key].block != Some(span.block)
            || key % 4 != span.mip as usize
        {
            return None;
        }
        let source = &self.surfaces[key / 4];
        if span.width != source.extents[0] >> span.mip
            || span.height != source.extents[1] >> span.mip
        {
            return None;
        }
        let bytes = span.width as usize * span.height as usize;
        self.arena.get(block.offset..block.offset + bytes)
    }

    fn span(
        &self,
        surface: usize,
        mip: u8,
        block: usize,
        transparent_index: Option<u8>,
    ) -> CacheSpan {
        let source = &self.surfaces[surface];
        CacheSpan {
            width: source.extents[0] >> mip,
            height: source.extents[1] >> mip,
            mip,
            texture_mins: source.texture_mins,
            cutout: transparent_index.is_some(),
            transparent_index,
            cache_id: self.id,
            block,
            generation: self.blocks[block].generation,
        }
    }

    fn pin(&mut self, block: usize) {
        if self.batch_active {
            self.blocks[block].pin_batch = self.batch;
        }
    }

    fn pinned(&self, block: usize) -> bool {
        self.batch_active
            && self.blocks[block].owner.is_some()
            && self.blocks[block].pin_batch == self.batch
    }

    fn clear_owner(&mut self, block: usize) {
        if let Some(key) = self.blocks[block].owner.take() {
            self.slots[key].block = None;
            self.stats.evictions = self.stats.evictions.saturating_add(1);
        }
        self.blocks[block].pin_batch = 0;
    }

    fn allocate(&mut self, bytes: usize) -> Option<usize> {
        let bytes = bytes.checked_add(7)? & !7;
        if bytes > self.arena.len() {
            return None;
        }
        let start = self.rover;
        let mut cursor = Some(start);
        while let Some(block) = cursor {
            if self.take_block(block, bytes, None) {
                return Some(block);
            }
            cursor = self.blocks[block].next;
        }
        cursor = Some(0);
        while let Some(block) = cursor {
            if block == start {
                break;
            }
            if self.take_block(block, bytes, Some(start)) {
                return Some(block);
            }
            cursor = self.blocks[block].next;
        }
        None
    }

    fn take_block(&mut self, block: usize, bytes: usize, limit: Option<usize>) -> bool {
        if self.pinned(block) {
            return false;
        }
        while self.blocks[block].bytes < bytes {
            let Some(next) = self.blocks[block].next else {
                return false;
            };
            if Some(next) == limit || self.pinned(next) {
                return false;
            }
            self.clear_owner(next);
            self.blocks[block].bytes += self.blocks[next].bytes;
            self.blocks[block].next = self.blocks[next].next;
            if self.rover == next {
                self.rover = block;
            }
            self.blocks[next] = Block {
                next: self.free_metadata,
                ..Block::default()
            };
            self.free_metadata = Some(next);
        }
        self.clear_owner(block);
        if self.blocks[block].bytes > bytes
            && let Some(fragment) = self.free_metadata
        {
            self.free_metadata = self.blocks[fragment].next;
            self.blocks[fragment] = Block {
                offset: self.blocks[block].offset + bytes,
                bytes: self.blocks[block].bytes - bytes,
                next: self.blocks[block].next,
                ..Block::default()
            };
            self.blocks[block].bytes = bytes;
            self.blocks[block].next = Some(fragment);
        }
        self.rover = self.blocks[block].next.unwrap_or(0);
        true
    }
}

fn build_lightmap(source: &SurfaceSource, state: BuildState<'_>, scratch: &mut [i32]) {
    let cells = source.grid_cells();
    if state.fullbright || !source.world_has_lightdata {
        scratch[..cells].fill(0);
        return;
    }
    for (cell, output) in scratch[..cells].iter_mut().enumerate() {
        let mut light = i64::from(state.ambient) << 8;
        if let Some(grid) = &source.lightmap {
            for style in 0..grid.style_count {
                let rgb = grid.samples[style * cells + cell];
                let value = match state.lighting {
                    IndexedLighting::Gray => rgb[0],
                    IndexedLighting::NativeRgb => {
                        if rgb[0] > rgb[1] && rgb[0] > rgb[2] {
                            rgb[0]
                        } else if rgb[1] > rgb[0] && rgb[1] > rgb[2] {
                            rgb[1]
                        } else {
                            rgb[2]
                        }
                    }
                    IndexedLighting::BrightestRgb => rgb[0].max(rgb[1]).max(rgb[2]),
                };
                light += i64::from(value) * i64::from(state.style_scales[style]);
            }
        }
        if let Some(dynamic) = state.dynamic {
            light += i64::from(dynamic[cell]);
        }
        *output = ((255 * 256 - light.max(0)) >> 2).max(64) as i32;
    }
}

fn fill_surface(
    source: &SurfaceSource,
    mip: u8,
    texture: &IndexedMip,
    transparent_index: Option<u8>,
    palette: &PaletteLighting,
    lights: &[i32],
    output: &mut [u8],
) {
    let shift = 4 - mip;
    let block_size = 1usize << shift;
    let width = (source.extents[0] >> mip) as usize;
    let light_width = source.extents[0] as usize / 16 + 1;
    let blocks_x = source.extents[0] as usize / 16;
    let blocks_y = source.extents[1] as usize / 16;
    let texture_width = texture.width as usize;
    let texture_height = texture.height as usize;
    let texture_x =
        i64::from(source.texture_mins[0] >> mip).rem_euclid(i64::from(texture.width)) as usize;
    let texture_y =
        i64::from(source.texture_mins[1] >> mip).rem_euclid(i64::from(texture.height)) as usize;
    for by in 0..blocks_y {
        for bx in 0..blocks_x {
            let top = by * light_width + bx;
            let bottom = top + light_width;
            let mut left = lights[top];
            let mut right = lights[top + 1];
            let left_step = (lights[bottom] - left) >> shift;
            let right_step = (lights[bottom + 1] - right) >> shift;
            let last_sample_x = (texture_x + bx * block_size + block_size - 1) % texture_width;
            let mut sample_y = (texture_y + by * block_size) % texture_height;
            for row in 0..block_size {
                let y = by * block_size + row;
                let step = (left - right) >> shift;
                let mut light = right;
                let mut sample_x = last_sample_x;
                let texture_row =
                    &texture.indices[sample_y * texture_width..(sample_y + 1) * texture_width];
                let start = y * width + bx * block_size;
                let destination = &mut output[start..start + block_size];
                for column in (0..block_size).rev() {
                    let index = texture_row[sample_x];
                    destination[column] = if Some(index) == transparent_index {
                        index
                    } else {
                        palette.colormap[(light as usize & 0xff00) + index as usize]
                    };
                    light += step;
                    sample_x = if sample_x == 0 {
                        texture_width - 1
                    } else {
                        sample_x - 1
                    };
                }
                left += left_step;
                right += right_step;
                sample_y += 1;
                if sample_y == texture_height {
                    sample_y = 0;
                }
            }
        }
    }
}
