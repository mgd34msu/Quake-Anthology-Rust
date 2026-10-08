//! Static surfaces precombine native texture mip texels and bilinear lightmaps
//! in the shared rover. Spans copy packed cache pixels on that texture lattice.
//! Non-affine attributes and oversized charts retain the scoped pair fallback.
use super::{TexelView, sampler_function};
use crate::assets::{
    Assets, DepthFunc, ImageId, Material, MaterialId, MaterialSettings, Stage, StageTexture, Vertex,
};
use crate::scene::Refdef;
use crate::shader::{AlphaFunc, AlphaGen, BlendFactor, RgbGen, TexCoordGen, TexMod};
use crate::stage::{DrawInputs, StageEvaluator};
use crate::surface_cache::{RgbaBuildState, SurfaceSource};
use crate::world::{
    SurfaceBinding,
    geometry::{LightSource, WorldGeometry},
};

#[derive(Clone)]
pub(super) enum Recipe {
    Product(Product),
    Pair(Pair),
}

#[derive(Clone, Copy)]
pub(super) enum Prepared {
    Product(ProductPrepared),
    Pair,
}

#[derive(Clone)]
pub(super) struct Product {
    pub cache: u32,
    pub base: ImageId,
    pub base_stage: usize,
    pub minima: [i32; 2],
    pub extents: [u32; 2],
    pub mip_count: u8,
    pub texture: [[f32; 3]; 2],
    layouts: [([i32; 2], [u32; 2]); 32],
    chart: [[f64; 4]; 2],
    source_coordinates: Box<[[f32; 2]]>,
    lightmap: ImageId,
    lightmap_region: Option<crate::lightmap::AtlasRegion>,
    lightcoord: [[f32; 3]; 2],
    color_basis: [Vertex; 3],
    color_checks: Box<[(Vertex, [f64; 2])]>,
    basis_coordinates: [[f64; 2]; 3],
    styles: [u8; 4],
    stages: [Stage; 2],
    settings: MaterialSettings,
    material: MaterialId,
    scale: [f32; 2],
    revisions: [u64; 2],
    samplers: [crate::assets::Sampler; 2],
    combination: Combination,
    color_plans: [ColorPlan; 2],
}

#[derive(Clone, Copy)]
enum Combination {
    Modulate,
    SeparatePasses,
}

#[derive(Clone, Copy)]
pub(super) struct ProductPrepared {
    pub state: RgbaBuildState,
    colors: [[[f64; 3]; 4]; 2],
}

#[derive(Clone, Copy)]
struct StageColors {
    uniform: [u8; 4],
    fields: [[f64; 3]; 4],
}

#[derive(Clone, Copy)]
enum ColorPlan {
    Fixed(Option<StageColors>),
    IdentityLight,
}

#[derive(Clone, Copy)]
struct ProductColors {
    uniform: [[u8; 4]; 2],
    fields: [[[f64; 3]; 4]; 2],
}

#[derive(Clone, Copy, PartialEq)]
struct ColorInput {
    source: u32,
    identity_light: f32,
}

/// One load-sized slot per immutable Product, owned by the view preparation.
/// Native byte rounding can reject a polygon, so failures are retained too.
#[derive(Clone, Copy, Default)]
pub(super) struct ProductColorCache {
    input: Option<ColorInput>,
    colors: Option<ProductColors>,
    refreshes: u64,
}

impl ProductColorCache {
    #[cfg(test)]
    pub(super) fn refreshes(&self) -> u64 {
        self.refreshes
    }
}

#[derive(Clone)]
pub(super) struct Pair {
    /// Factor ids in material stage order, not base/lightmap order.
    pub factors: [usize; 2],
    images: [ImageId; 2],
    revisions: [u64; 2],
    samplers: [crate::assets::Sampler; 2],
    stages: [Stage; 2],
}

/// One image/region owns fixed cache sources for its native levels. Several
/// surfaces and stage tables may borrow those raw bytes without duplicating it.
pub(super) struct Factor {
    image: ImageId,
    region: Option<crate::lightmap::AtlasRegion>,
    revision: u64,
    levels: [Option<u32>; 32],
}

impl Factor {
    fn register(
        image: ImageId,
        region: Option<crate::lightmap::AtlasRegion>,
        assets: &Assets,
        factors: &mut Vec<Self>,
        sources: &mut Vec<SurfaceSource>,
    ) -> Result<usize, &'static str> {
        let resource = assets.image(image).ok_or("invalid static factor image")?;
        if let Some(index) = factors.iter().position(|factor| {
            factor.image == image
                && factor.region == region
                && factor.revision == resource.preparation_revision
        }) {
            return Ok(index);
        }
        let count = resource
            .prepared
            .as_ref()
            .map_or(1, |p| p.levels.len().min(32));
        let mut levels = [None; 32];
        for (mip, source) in levels.iter_mut().enumerate().take(count) {
            let Some(view) = TexelView::image(
                resource,
                mip as u8,
                crate::assets::TextureIntensity::Preserve,
            )
            .region(region) else {
                // The generic sampler also rejects a region outside this
                // prepared level. No invalid ROI cache source is registered.
                continue;
            };
            let id = u32::try_from(sources.len()).map_err(|_| "too many static factor levels")?;
            sources.push(SurfaceSource::load_rgba(
                [view.bounds[0] as i32, view.bounds[1] as i32],
                [view.bounds[2], view.bounds[3]],
                1,
            )?);
            *source = Some(id);
        }
        let index = factors.len();
        factors.push(Self {
            image,
            region,
            revision: resource.preparation_revision,
            levels,
        });
        Ok(index)
    }

    pub(super) fn prepare(
        &self,
        mip: u8,
        assets: &Assets,
        cache: &mut crate::surface_cache::SurfaceCache,
    ) -> Option<crate::surface_cache::CacheSpan> {
        let image = assets.image(self.image)?;
        let view = TexelView::image(image, mip, crate::assets::TextureIntensity::Preserve)
            .region(self.region)?;
        let source = self.levels.get(mip as usize).copied().flatten()?;
        cache.prepare_rgba(
            source,
            0,
            RgbaBuildState {
                base_image_id: self.image.0,
                base_revision: self.revision,
                ..RgbaBuildState::default()
            },
            |out| {
                let row_bytes = view.bounds[2] as usize * 4;
                for (row, destination) in out.chunks_exact_mut(row_bytes).enumerate() {
                    let start = ((view.bounds[1] as usize + row) * view.width as usize
                        + view.bounds[0] as usize)
                        * 4;
                    destination.copy_from_slice(&view.rgba[start..start + row_bytes]);
                }
            },
        )
    }
}

impl Recipe {
    pub(super) fn load(
        geometry: &WorldGeometry,
        boundary: usize,
        binding: SurfaceBinding,
        material: &Material,
        assets: &Assets,
        evaluator: &StageEvaluator,
        sources: &mut Vec<SurfaceSource>,
        factors: &mut Vec<Factor>,
        cache_bytes: usize,
    ) -> Result<Option<Self>, &'static str> {
        let cache = u32::try_from(sources.len()).map_err(|_| "too many cached boundaries")?;
        if let Some(product) = Product::load(
            geometry,
            boundary,
            binding,
            material,
            assets,
            evaluator,
            cache,
            cache_bytes,
        ) {
            sources.push(product.source()?);
            return Ok(Some(Self::Product(product)));
        }
        let Some(base_stage) = eligible(material) else {
            return Ok(None);
        };
        let StageTexture::Image(base) = material.stages[base_stage].texture else {
            return Ok(None);
        };
        let images = if base_stage == 0 {
            [base, binding.lightmap]
        } else {
            [binding.lightmap, base]
        };
        let regions = if base_stage == 0 {
            [None, binding.lightmap_region]
        } else {
            [binding.lightmap_region, None]
        };
        let (Some(a), Some(b)) = (assets.image(images[0]), assets.image(images[1])) else {
            return Ok(None);
        };
        for (index, image) in [a, b].into_iter().enumerate() {
            let stage = material.stages[index];
            for mip in 0..super::image_mips(image, stage.sampler) {
                if TexelView::image(image, mip, stage.texture_intensity)
                    .region(regions[index])
                    .is_none()
                {
                    // Generic stages reject independently. Keep that path if
                    // any reachable ROI is invalid rather than suppressing a
                    // valid opaque stage in the fixed combined span kernel.
                    return Ok(None);
                }
            }
        }
        let factors = [
            Factor::register(images[0], regions[0], assets, factors, sources)?,
            Factor::register(images[1], regions[1], assets, factors, sources)?,
        ];
        Ok(Some(Self::Pair(Pair {
            factors,
            images,
            revisions: [a.preparation_revision, b.preparation_revision],
            samplers: [
                super::effective_sampler(a, material.stages[0].sampler),
                super::effective_sampler(b, material.stages[1].sampler),
            ],
            stages: [material.stages[0], material.stages[1]],
        })))
    }
    pub(super) fn product(&self) -> bool {
        matches!(self, Self::Product(_))
    }
    pub(super) fn current(&self, assets: &Assets) -> bool {
        match self {
            Self::Product(product) => product.current(assets),
            Self::Pair(pair) => {
                let (Some(a), Some(b)) =
                    (assets.image(pair.images[0]), assets.image(pair.images[1]))
                else {
                    return false;
                };
                [a.preparation_revision, b.preparation_revision] == pair.revisions
                    && [
                        super::effective_sampler(a, pair.stages[0].sampler),
                        super::effective_sampler(b, pair.stages[1].sampler),
                    ] == pair.samplers
            }
        }
    }
    pub(super) fn coordinate(&self, position: qa_core::primitives::Vec3) -> Option<[f32; 2]> {
        match self {
            Self::Product(product) => Some(product.coordinate(position)),
            Self::Pair(_) => None,
        }
    }
    pub(super) fn coordinate_corner(&self, corner: usize) -> Option<[f32; 2]> {
        match self {
            Self::Product(product) => product.coordinate_corner(corner),
            Self::Pair(_) => None,
        }
    }
    pub(super) fn prepare_cached(
        &self,
        refdef: &Refdef,
        evaluator: &StageEvaluator,
        cache: &mut ProductColorCache,
    ) -> Option<Prepared> {
        match self {
            Self::Product(product) => product
                .prepare_cached(refdef, evaluator, cache)
                .map(Prepared::Product),
            Self::Pair(_) => Some(Prepared::Pair),
        }
    }
}

fn eligible(material: &Material) -> Option<usize> {
    let [a, b] = material.stages.as_ref() else {
        return None;
    };
    if material.settings.sky.is_some()
        || material.settings.fog.is_some()
        || material.settings.portal
        || material.settings.polygon_offset
        || material.settings.deforms.iter().any(Option::is_some)
        || a.blend.is_some()
        || a.depth_func != DepthFunc::Lequal
        || !a.depth_write
        || b.depth_write
        || !matches!(b.depth_func, DepthFunc::Lequal | DepthFunc::Equal)
        || !matches!(b.blend, Some(blend) if
            (blend.source == BlendFactor::DestinationColor && blend.destination == BlendFactor::Zero)
            || (blend.source == BlendFactor::Zero && blend.destination == BlendFactor::SourceColor))
    {
        return None;
    }
    for stage in [a, b] {
        if stage.alpha_test != AlphaFunc::None
            || !matches!(
                stage.rgb_gen,
                RgbGen::Identity
                    | RgbGen::IdentityLighting
                    | RgbGen::Const(_)
                    | RgbGen::ExactVertex
                    | RgbGen::Vertex
                    | RgbGen::OneMinusVertex
            )
            || !matches!(
                stage.alpha_gen,
                AlphaGen::Identity
                    | AlphaGen::Skip
                    | AlphaGen::Const(_)
                    | AlphaGen::Vertex
                    | AlphaGen::OneMinusVertex
            )
            || stage.tcmods.iter().flatten().any(|modifier| {
                !matches!(
                    modifier,
                    crate::assets::TcMod::Script(TexMod::Scale(_) | TexMod::Transform { .. })
                )
            })
        {
            return None;
        }
    }
    match (a.texture, a.texgen, b.texture, b.texgen) {
        (
            StageTexture::Image(_),
            TexCoordGen::Texture,
            StageTexture::Lightmap,
            TexCoordGen::Lightmap,
        ) => Some(0),
        (
            StageTexture::Lightmap,
            TexCoordGen::Lightmap,
            StageTexture::Image(_),
            TexCoordGen::Texture,
        ) => Some(1),
        _ => None,
    }
}

impl Product {
    pub(super) fn load(
        geometry: &WorldGeometry,
        boundary: usize,
        binding: SurfaceBinding,
        material: &Material,
        assets: &Assets,
        evaluator: &StageEvaluator,
        cache: u32,
        cache_bytes: usize,
    ) -> Option<Self> {
        let base_stage = eligible(material)?;
        let StageTexture::Image(base) = material.stages[base_stage].texture else {
            return None;
        };
        let image = assets.image(base)?;
        let lightmap = assets.image(binding.lightmap)?;
        let base_view = TexelView::image(image, 0, material.stages[base_stage].texture_intensity);
        let light_view = TexelView::image(
            lightmap,
            0,
            material.stages[1 - base_stage].texture_intensity,
        )
        .region(binding.lightmap_region)?;
        let indices = geometry
            .indices
            .get(geometry.boundaries.get(boundary)?.indices())?;
        let vertices: Vec<_> = indices
            .iter()
            .map(|&index| {
                let vertex = geometry.vertices[index as usize];
                Vertex {
                    normal: vertex.normal,
                    ..vertex.vertex
                }
            })
            .collect();
        let (axes, basis) = geometric_basis(&vertices)?;
        let inputs = DrawInputs {
            lightmap: binding.lightmap,
            texture_scale: binding.texture_scale,
            ..DrawInputs::default()
        };
        let prepared = [
            evaluator
                .prepare(&material.stages[0], material.settings, inputs)
                .ok()?,
            evaluator
                .prepare(&material.stages[1], material.settings, inputs)
                .ok()?,
        ];
        let texture: Vec<_> = vertices
            .iter()
            .map(|v| {
                evaluator
                    .evaluate(&prepared[base_stage], v)
                    .texcoord
                    .map(f64::from)
            })
            .collect();
        let lights: Vec<_> = vertices
            .iter()
            .map(|v| {
                evaluator
                    .evaluate(&prepared[1 - base_stage], v)
                    .texcoord
                    .map(f64::from)
            })
            .collect();
        let geometric: Vec<_> = vertices
            .iter()
            .map(|v| axes.map(|axis| f64::from(v.position.0[axis])))
            .collect();
        let basis_geometry = basis.map(|i| geometric[i]);
        let texel_coordinates: Vec<_> = texture
            .iter()
            .map(|v| {
                [
                    v[0] * f64::from(base_view.width),
                    v[1] * f64::from(base_view.height),
                ]
            })
            .collect();
        // An invertible base-texel chart gives the exact native texel grid.
        // Constant/one-dimensional UVs retain their geometric area instead.
        let use_texture = determinant(basis.map(|i| texel_coordinates[i])) != 0.0;
        let coordinates = if use_texture {
            texel_coordinates
        } else {
            let texture_fields = std::array::from_fn::<_, 2, _>(|axis| {
                fit(basis_geometry, basis.map(|i| texture[i][axis]))
            });
            let light_fields = std::array::from_fn::<_, 2, _>(|axis| {
                fit(basis_geometry, basis.map(|i| lights[i][axis]))
            });
            let scale = std::array::from_fn::<_, 2, _>(|axis| {
                (texture_fields[0][axis].abs() * f64::from(base_view.width))
                    .max(texture_fields[1][axis].abs() * f64::from(base_view.height))
                    .max(light_fields[0][axis].abs() * f64::from(light_view.width))
                    .max(light_fields[1][axis].abs() * f64::from(light_view.height))
                    .max(1.0)
            });
            let varies = std::array::from_fn::<_, 2, _>(|axis| {
                texel_coordinates
                    .iter()
                    .any(|v| v[axis] != texel_coordinates[0][axis])
            });
            if varies == [false, false] || varies == [true, true] {
                geometric
                    .iter()
                    .map(|v| [v[0] * scale[0], v[1] * scale[1]])
                    .collect()
            } else {
                let variable = usize::from(varies[1]);
                let other = 1 - variable;
                let mut widest = 0.0;
                let mut selected = None;
                for axis in 0..2 {
                    let area = determinant(
                        basis.map(|i| [texel_coordinates[i][variable], geometric[i][axis]]),
                    )
                    .abs();
                    if area > widest {
                        widest = area;
                        selected = Some(axis);
                    }
                }
                let axis = selected?;
                texel_coordinates
                    .iter()
                    .zip(&geometric)
                    .map(|(texel, point)| {
                        let mut chart = [0.0; 2];
                        chart[variable] = texel[variable];
                        chart[other] = point[axis] * scale[axis];
                        chart
                    })
                    .collect()
            }
        };
        let basis_coordinates = basis.map(|i| coordinates[i]);
        let mut minima = [0; 2];
        let mut extents = [0; 2];
        for axis in 0..2 {
            let lo = coordinates
                .iter()
                .map(|v| v[axis])
                .fold(f64::INFINITY, f64::min)
                .floor()
                - 1.0;
            let hi = coordinates
                .iter()
                .map(|v| v[axis])
                .fold(f64::NEG_INFINITY, f64::max)
                .ceil()
                + 1.0;
            if !lo.is_finite()
                || !hi.is_finite()
                || lo < i32::MIN as f64
                || hi > i32::MAX as f64
                || !(1.0..=8192.0).contains(&(hi - lo))
            {
                return None;
            }
            minima[axis] = lo as i32;
            extents[axis] = (hi - lo) as u32;
        }
        let fields = |values: &[[f64; 2]]| {
            std::array::from_fn(|axis| {
                fit(basis_coordinates, basis.map(|i| values[i][axis])).map(|v| v as f32)
            })
        };
        let chart: [[f64; 4]; 2] = std::array::from_fn(|coordinate| {
            let affine = fit(basis_geometry, basis.map(|i| coordinates[i][coordinate]));
            let mut projection = [0.0; 4];
            projection[axes[0]] = affine[0];
            projection[axes[1]] = affine[1];
            projection[3] = affine[2];
            projection
        });
        let vertex_colors = material.stages.iter().any(|stage| {
            matches!(
                stage.rgb_gen,
                RgbGen::ExactVertex | RgbGen::Vertex | RgbGen::OneMinusVertex
            ) || matches!(stage.alpha_gen, AlphaGen::Vertex | AlphaGen::OneMinusVertex)
        });
        // Non-affine polygon attributes retain the explicit pair fallback.
        // Triangles, including tessellated patches, always have one affine map.
        for (index, vertex) in vertices.iter().enumerate() {
            for values in [&texture, &lights] {
                for axis in 0..2 {
                    let affine = fit(basis_coordinates, basis.map(|i| values[i][axis]));
                    if (at64(affine, coordinates[index]) - values[index][axis]).abs() > 1.0e-5 {
                        return None;
                    }
                }
            }
            if vertex_colors {
                for channel in 0..4 {
                    let affine = fit(
                        basis_coordinates,
                        basis.map(|i| f64::from(vertices[i].color[channel])),
                    );
                    if affine.iter().any(|&value| !(value as f32).is_finite())
                        || (at64(affine, coordinates[index]) - f64::from(vertex.color[channel]))
                            .abs()
                            > 1.0e-5
                    {
                        return None;
                    }
                }
            }
        }
        let texture: [[f32; 3]; 2] = fields(&texture);
        let lightcoord: [[f32; 3]; 2] = fields(&lights);
        if texture
            .as_flattened()
            .iter()
            .chain(lightcoord.as_flattened())
            .any(|value| !value.is_finite())
            || chart.as_flattened().iter().any(|value| !value.is_finite())
        {
            return None;
        }
        let mip_count = super::image_mips(image, material.stages[base_stage].sampler);
        let source = SurfaceSource::load_rgba_texels(minima, extents, mip_count).ok()?;
        let mut layouts = [([0; 2], [1; 2]); 32];
        for (mip, layout) in layouts.iter_mut().enumerate().take(mip_count as usize) {
            if source.reservation_bytes(mip as u8)? > cache_bytes {
                return None;
            }
            *layout = source.mip_layout(mip as u8)?;
        }
        let surface = geometry
            .surfaces
            .iter()
            .find(|surface| surface.boundaries.indices().contains(&boundary))?;
        // qsrc gl_rsurf.c styled grids retain styles; tr_bsp.c:199-205
        // authored pages are static and have no lightstyle dependency.
        let styles = match surface.light_source {
            LightSource::Samples => surface.styles,
            _ => [255; 4],
        };
        let source_coordinates = vertices
            .iter()
            .map(|vertex| chart_coordinate(chart, vertex.position))
            .collect();
        let mut product = Self {
            cache,
            base,
            base_stage,
            minima,
            extents,
            mip_count,
            texture,
            lightcoord,
            layouts,
            chart,
            source_coordinates,
            lightmap: binding.lightmap,
            lightmap_region: binding.lightmap_region,
            color_basis: basis.map(|i| vertices[i]),
            color_checks: if vertex_colors {
                vertices
                    .iter()
                    .copied()
                    .zip(coordinates)
                    .enumerate()
                    .filter(|(index, _)| !basis.contains(index))
                    .map(|(_, entry)| entry)
                    .collect()
            } else {
                Box::default()
            },
            basis_coordinates,
            styles,
            stages: [material.stages[0], material.stages[1]],
            settings: material.settings,
            material: binding.material,
            scale: binding.texture_scale,
            revisions: [image.preparation_revision, lightmap.preparation_revision],
            samplers: [
                super::effective_sampler(image, material.stages[base_stage].sampler),
                super::effective_sampler(lightmap, material.stages[1 - base_stage].sampler),
            ],
            // qsrc tr_shader.c CollapseMultitexture ignores depth-write bits,
            // but requires matching other state and color-generator kinds.
            combination: if material.stages[0].depth_func == material.stages[1].depth_func
                && std::mem::discriminant(&material.stages[0].rgb_gen)
                    == std::mem::discriminant(&material.stages[1].rgb_gen)
                && std::mem::discriminant(&material.stages[0].alpha_gen)
                    == std::mem::discriminant(&material.stages[1].alpha_gen)
            {
                Combination::Modulate
            } else {
                Combination::SeparatePasses
            },
            color_plans: [ColorPlan::Fixed(None); 2],
        };
        // qsrc tr_shade.c:662-715 generates source byte colors before clipping.
        // Identity/Const/ExactVertex fields are immutable; vertex scaling and
        // identity lighting retain their exact parameter-dependent rounding.
        for stage in 0..2 {
            product.color_plans[stage] = if matches!(
                product.stages[stage].rgb_gen,
                RgbGen::IdentityLighting | RgbGen::Vertex | RgbGen::OneMinusVertex
            ) {
                ColorPlan::IdentityLight
            } else {
                ColorPlan::Fixed(product.fit_stage_colors(stage, 1.0, evaluator))
            };
        }
        Some(product)
    }
    pub(super) fn current(&self, assets: &Assets) -> bool {
        let (Some(base), Some(light)) = (assets.image(self.base), assets.image(self.lightmap))
        else {
            return false;
        };
        [base.preparation_revision, light.preparation_revision] == self.revisions
            && [
                super::effective_sampler(base, self.stages[self.base_stage].sampler),
                super::effective_sampler(light, self.stages[1 - self.base_stage].sampler),
            ] == self.samplers
    }
    pub(super) fn source(&self) -> Result<SurfaceSource, &'static str> {
        SurfaceSource::load_rgba_texels(self.minima, self.extents, self.mip_count)
    }
    pub(super) fn coordinate(&self, position: qa_core::primitives::Vec3) -> [f32; 2] {
        chart_coordinate(self.chart, position)
    }
    pub(super) fn coordinate_corner(&self, corner: usize) -> Option<[f32; 2]> {
        self.source_coordinates.get(corner).copied()
    }
    #[cfg(test)]
    pub(super) fn prepare(
        &self,
        refdef: &Refdef,
        evaluator: &StageEvaluator,
    ) -> Option<ProductPrepared> {
        let colors = self.prepare_colors(refdef.identity_light, evaluator)?;
        Some(self.with_state(refdef, colors))
    }
    pub(super) fn prepare_cached(
        &self,
        refdef: &Refdef,
        evaluator: &StageEvaluator,
        cache: &mut ProductColorCache,
    ) -> Option<ProductPrepared> {
        let colors = if self.identity_dependent() {
            let input = ColorInput {
                source: self.cache,
                identity_light: refdef.identity_light,
            };
            if cache.input != Some(input) {
                cache.input = Some(input);
                cache.colors = self.prepare_colors(refdef.identity_light, evaluator);
                cache.refreshes = cache.refreshes.saturating_add(1);
            }
            cache.colors?
        } else {
            self.prepare_colors(1.0, evaluator)?
        };
        Some(self.with_state(refdef, colors))
    }
    fn identity_dependent(&self) -> bool {
        self.color_plans
            .iter()
            .any(|plan| matches!(plan, ColorPlan::IdentityLight))
    }
    fn prepare_colors(
        &self,
        identity_light: f32,
        evaluator: &StageEvaluator,
    ) -> Option<ProductColors> {
        let mut colors = ProductColors {
            uniform: [[255; 4]; 2],
            fields: [[[0.0; 3]; 4]; 2],
        };
        for stage in 0..2 {
            let color = match self.color_plans[stage] {
                ColorPlan::Fixed(color) => color?,
                ColorPlan::IdentityLight => {
                    self.fit_stage_colors(stage, identity_light, evaluator)?
                }
            };
            colors.uniform[stage] = color.uniform;
            colors.fields[stage] = color.fields;
        }
        Some(colors)
    }
    fn fit_stage_colors(
        &self,
        stage: usize,
        identity_light: f32,
        evaluator: &StageEvaluator,
    ) -> Option<StageColors> {
        let inputs = DrawInputs {
            identity_light,
            lightmap: self.lightmap,
            texture_scale: self.scale,
            ..DrawInputs::default()
        };
        let prepared = evaluator
            .prepare(&self.stages[stage], self.settings, inputs)
            .ok()?;
        let values = self
            .color_basis
            .map(|vertex| evaluator.evaluate(&prepared, &vertex).color);
        let fields = std::array::from_fn::<_, 4, _>(|channel| {
            fit(
                self.basis_coordinates,
                values.map(|value| f64::from(value[channel]) / 255.0),
            )
        });
        // Native CGEN_VERTEX truncation can break a polygon's affine field.
        for &(vertex, coordinate) in &self.color_checks {
            let value = evaluator.evaluate(&prepared, &vertex).color;
            if fields.iter().zip(value).any(|(&field, channel)| {
                (at64(field, coordinate) - f64::from(channel) / 255.0).abs() > 1.0e-7
            }) {
                return None;
            }
        }
        if fields.as_flattened().iter().any(|value| !value.is_finite()) {
            return None;
        }
        Some(StageColors {
            uniform: values[0],
            fields,
        })
    }
    fn with_state(&self, refdef: &Refdef, colors: ProductColors) -> ProductPrepared {
        ProductPrepared {
            colors: colors.fields,
            state: RgbaBuildState {
                material_id: self.material.0,
                base_image_id: self.base.0,
                lightmap_image_id: Some(self.lightmap.0),
                stage_colors: colors.uniform,
                identity_light: if self.identity_dependent() {
                    refdef.identity_light
                } else {
                    1.0
                },
                base_revision: self.revisions[0],
                lightmap_revision: self.revisions[1],
                style_scales: self.styles.map(|style| {
                    if style == 255 {
                        [1.0; 3]
                    } else {
                        refdef.lightstyles[style as usize].rgb
                    }
                }),
                ..RgbaBuildState::default()
            },
        }
    }
    pub(super) fn fill(
        &self,
        prepared: ProductPrepared,
        mip: u8,
        assets: &Assets,
        destination: &mut [u8],
    ) {
        let (Some(base), Some(light)) = (assets.image(self.base), assets.image(self.lightmap))
        else {
            return;
        };
        let base_stage = self.stages[self.base_stage];
        let light_stage = self.stages[1 - self.base_stage];
        let base_pixels = TexelView::image(base, mip, base_stage.texture_intensity);
        let Some(light_pixels) =
            TexelView::image(light, 0, light_stage.texture_intensity).region(self.lightmap_region)
        else {
            return;
        };
        // Native software cache fills fetch texture mip texels once. Linear
        // material admission does not add another filter to the filled lattice.
        let base_sample = sampler_function(crate::assets::Sampler {
            filter: crate::assets::Filter::Nearest,
            ..super::effective_sampler(base, base_stage.sampler)
        });
        let light_sample = sampler_function(crate::assets::Sampler {
            wrap: crate::assets::Wrap::Clamp,
            filter: crate::assets::Filter::Linear,
            mipmaps: false,
        });
        let (minimum, [width, _]) = self.layouts[mip as usize];
        let step = (1u64 << mip) as f32;
        for (index, out) in destination.chunks_exact_mut(4).enumerate() {
            let coordinate = [
                f64::from(minimum[0])
                    + f64::from(index as u32 % width) * f64::from(step)
                    + 0.5 * f64::from(step),
                f64::from(minimum[1])
                    + f64::from(index as u32 / width) * f64::from(step)
                    + 0.5 * f64::from(step),
            ];
            let colors = prepared
                .colors
                .map(|stage| stage.map(|field| at64(field, coordinate).clamp(0.0, 1.0) as f32));
            let coordinate = coordinate.map(|value| value as f32);
            let mut samples = [[0.0; 4]; 2];
            let sample_colors = match self.combination {
                Combination::Modulate => [[1.0; 4]; 2],
                Combination::SeparatePasses => colors,
            };
            samples[self.base_stage] = base_sample(
                base_pixels,
                self.texture.map(|p| at(p, coordinate)),
                sample_colors[self.base_stage],
            );
            samples[1 - self.base_stage] = light_sample(
                light_pixels,
                self.lightcoord.map(|p| at(p, coordinate)),
                sample_colors[1 - self.base_stage],
            );
            let pixel = match self.combination {
                Combination::Modulate => super::composite(
                    0,
                    std::array::from_fn(|channel| {
                        samples[0][channel] * samples[1][channel] * colors[0][channel]
                    }),
                    None,
                ),
                Combination::SeparatePasses => {
                    let first = super::composite(0, samples[0], None);
                    super::composite(first, samples[1], self.stages[1].blend)
                }
            };
            out.copy_from_slice(&pixel.to_le_bytes());
        }
    }
    pub(super) fn base_sampler(&self) -> crate::assets::Sampler {
        self.stages[self.base_stage].sampler
    }
    pub(super) fn cached_pixel(
        block: crate::surface_cache::CacheSpan,
        pixels: &[[u8; 4]],
        chart: [f32; 2],
    ) -> u32 {
        let step = (1u64 << block.mip) as f32;
        let coordinate: [usize; 2] = std::array::from_fn(|axis| {
            (((chart[axis] - block.texture_mins[axis] as f32) / step).floor() as usize)
                .min([block.width, block.height][axis] as usize - 1)
        });
        u32::from_le_bytes(pixels[coordinate[1] * block.width as usize + coordinate[0]])
    }
}

/// The existing CPU separate-pass contract rounds the first opaque framebuffer
/// before multiplying the independently filtered second factor. Native Q3's
/// collapsed multitexture rounding is a separate presentation requirement.
pub(super) fn multiply_pixel(first: u32, second: [f32; 4]) -> u32 {
    u32::from_le_bytes(std::array::from_fn(|channel| {
        let byte = first.to_le_bytes()[channel] as f32 / 255.0;
        ((second[channel] * byte).clamp(0.0, 1.0) * 255.0).round() as u8
    }))
}

fn at(field: [f32; 3], coordinate: [f32; 2]) -> f32 {
    field[2] + field[0] * coordinate[0] + field[1] * coordinate[1]
}
fn chart_coordinate(chart: [[f64; 4]; 2], position: qa_core::primitives::Vec3) -> [f32; 2] {
    chart.map(|p| {
        (p[3]
            + p[0] * f64::from(position.0[0])
            + p[1] * f64::from(position.0[1])
            + p[2] * f64::from(position.0[2])) as f32
    })
}

fn at64(field: [f64; 3], coordinate: [f64; 2]) -> f64 {
    field[2] + field[0] * coordinate[0] + field[1] * coordinate[1]
}
fn determinant(points: [[f64; 2]; 3]) -> f64 {
    (points[1][0] - points[0][0]) * (points[2][1] - points[0][1])
        - (points[2][0] - points[0][0]) * (points[1][1] - points[0][1])
}
fn fit(points: [[f64; 2]; 3], values: [f64; 3]) -> [f64; 3] {
    let ab = [points[1][0] - points[0][0], points[1][1] - points[0][1]];
    let ac = [points[2][0] - points[0][0], points[2][1] - points[0][1]];
    let vb = values[1] - values[0];
    let vc = values[2] - values[0];
    let area = determinant(points);
    let x = (vb * ac[1] - vc * ab[1]) / area;
    let y = (ab[0] * vc - ac[0] * vb) / area;
    [x, y, values[0] - x * points[0][0] - y * points[0][1]]
}
fn geometric_basis(vertices: &[Vertex]) -> Option<([usize; 2], [usize; 3])> {
    vertices.first()?;
    let mut largest = 0.0;
    let mut result = None;
    for axes in [[0, 1], [0, 2], [1, 2]] {
        for index in 1..vertices.len().saturating_sub(1) {
            let basis = [0, index, index + 1];
            let area = determinant(
                basis.map(|i| axes.map(|axis| f64::from(vertices[i].position.0[axis]))),
            )
            .abs();
            if area > largest {
                largest = area;
                result = Some((axes, basis));
            }
        }
    }
    result
}

#[cfg(test)]
#[path = "../../tests/cpu_factors/oracle.rs"]
mod factor_tests;

#[cfg(test)]
#[path = "../../tests/cpu_static/oracle.rs"]
mod static_tests;
