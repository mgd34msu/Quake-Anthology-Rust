//! Shared entity/poly raster and ordered 2D command consumer.
//!
//! Worlds use the shared edge/span scanner and native indexed surface cache.
//! Entity/poly meshes use the shared stage evaluator and a 1/Z buffer.
use crate::BackendStats;
use crate::affine::clip_polygon_plane;
use crate::assets::{
    AssetAdmission, AssetCounts, Assets, DepthFunc, Filter, Image, Material, Sampler,
    TextureIntensity, Vertex, Wrap,
};
use crate::shader::{Cull, StageBlend};
use crate::stage::{DrawInputs, PreparedStage, StageEvaluator, alpha_pass, blend_pixel};
mod indexed;
mod particles;
mod rgba;
mod sky;
mod span;
mod world;
use crate::scene::{
    BlendPhase, Command, CommandList, CpuPresentation, Draw2d, DrawKind, PaletteOperation, Poly,
    Refdef, SceneEntity, Viewport,
};
use qa_core::primitives::Vec3;
pub use world::{
    CpuJob, CpuLimits, JobKind, MAX_BANDS, RasterBands, RasterConfig, WorldStats, run_cpu_job,
};

const CLIP_VERTICES: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreparePoint {
    Begin,
    End,
}

pub struct CpuBackend {
    frame: Framebuffer,
    clear_color: [u8; 4],
    material_frame: Framebuffer,
    warp_frame: Framebuffer,
    warp_material_frame: Framebuffer,
    warp_sine: Box<[u8]>,
    warp_columns: Box<[usize]>,
    warp_rows: Box<[usize]>,
    warp_dimensions: Option<(u32, u32)>,
    world: Option<world::WorldRaster>,
    limits: CpuLimits,
    admission: Option<AssetAdmission>,
    evaluator: StageEvaluator,
    presentation: CpuPresentation,
    time_ms: i64,
}

struct Framebuffer {
    width: u32,
    height: u32,
    pixels: Box<[u32]>,
    inverse_depth: Box<[f32]>,
    depth_ranks: Box<[u32]>,
    indices: Box<[u8]>,
    palettes: Box<[u32]>,
}
impl Framebuffer {
    fn capacity_bytes(&self) -> usize {
        std::mem::size_of_val(&*self.pixels)
            + std::mem::size_of_val(&*self.inverse_depth)
            + std::mem::size_of_val(&*self.depth_ranks)
            + std::mem::size_of_val(&*self.indices)
            + std::mem::size_of_val(&*self.palettes)
    }
    fn clear(&mut self, color: [u8; 4]) {
        self.pixels.fill(u32::from_le_bytes(color));
        self.inverse_depth.fill(0.0);
        self.depth_ranks.fill(0);
        self.palettes.fill(u32::MAX);
    }
    fn load(width: u32, height: u32) -> Result<Self, &'static str> {
        let count = (width as usize)
            .checked_mul(height as usize)
            .ok_or("CPU framebuffer dimensions overflow")?;
        Ok(Self {
            width,
            height,
            pixels: vec![0; count].into_boxed_slice(),
            inverse_depth: vec![0.0; count].into_boxed_slice(),
            depth_ranks: vec![0; count].into_boxed_slice(),
            indices: vec![0; count].into_boxed_slice(),
            palettes: vec![u32::MAX; count].into_boxed_slice(),
        })
    }
    fn seed_from(&mut self, source: &Self) {
        self.pixels.copy_from_slice(&source.pixels);
        self.inverse_depth.copy_from_slice(&source.inverse_depth);
        self.depth_ranks.copy_from_slice(&source.depth_ranks);
        self.indices.copy_from_slice(&source.indices);
        self.palettes.copy_from_slice(&source.palettes);
    }
    fn composite_material(
        &mut self,
        completed: &Self,
        opacity: crate::scene::MaterialOpacity,
        camera: &Camera,
        assets: &Assets,
    ) {
        let width = self.width;
        let mut buffers = self.buffers();
        let Some(rows) = buffers.rows(width, camera.refdef.viewport) else {
            return;
        };
        for y in rows {
            for x in
                camera.refdef.viewport.x..camera.refdef.viewport.x + camera.refdef.viewport.width
            {
                let index = buffers.offset(width, x, y);
                let original = buffers.pixels[index];
                let color = completed.pixels[index];
                if color == original {
                    continue;
                }
                if let crate::scene::CpuPresentation::Indexed { palette, .. } =
                    camera.refdef.cpu_presentation
                    && let Some(resource) = assets.palette(palette)
                {
                    let color_index = if completed.palettes[index] == palette.0 {
                        completed.indices[index]
                    } else {
                        let [r, g, b, _] = color.to_le_bytes();
                        resource.nearest_index([r, g, b], None)
                    };
                    let destination = if buffers.palettes[index] == palette.0 {
                        buffers.indices[index]
                    } else {
                        let [r, g, b, _] = original.to_le_bytes();
                        resource.nearest_index([r, g, b], None)
                    };
                    let color = indexed::composite_color(
                        resource,
                        color_index,
                        Some(destination),
                        opacity.value(),
                        None,
                        false,
                    )
                    .map(|(color, _)| color)
                    .unwrap_or_else(|| {
                        let [r, g, b, _] = opacity.composite(original, color).to_le_bytes();
                        resource.nearest_index([r, g, b], None)
                    });
                    buffers.write_indexed(index, color, resource, palette.0, None);
                } else {
                    buffers.write_rgba(index, opacity.composite(original, color), None);
                }
            }
        }
    }
    fn buffers(&mut self) -> world::Buffers<'_> {
        world::Buffers {
            first_row: 0,
            frame_width: self.width,
            frame_height: self.height,
            pixels: &mut self.pixels,
            inverse_depth: &mut self.inverse_depth,
            depth_ranks: &mut self.depth_ranks,
            indices: &mut self.indices,
            palettes: &mut self.palettes,
        }
    }
}

#[derive(Clone, Copy, Default)]
struct ClipVertex {
    camera: Vec3,
    texcoord: [f32; 2],
    lightmap_coord: [f32; 2],
    color: [f32; 4],
    indexed_light: f32,
}

impl ClipVertex {
    fn from_vertex(vertex: Vertex, camera: Vec3, color: [f32; 4]) -> Self {
        Self {
            camera,
            texcoord: vertex.texcoord,
            lightmap_coord: vertex.lightmap_coord,
            color,
            indexed_light: 0.0,
        }
    }

    fn finite(self) -> bool {
        self.camera.is_finite()
            && self.indexed_light.is_finite()
            && self
                .texcoord
                .iter()
                .chain(self.lightmap_coord.iter())
                .chain(self.color.iter())
                .all(|f| f.is_finite())
    }

    fn lerp(self, end: Self, fraction: f32) -> Self {
        Self {
            camera: self.camera.lerp(end.camera, fraction),
            texcoord: qa_core::math::lerp_array(self.texcoord, end.texcoord, fraction),
            lightmap_coord: qa_core::math::lerp_array(
                self.lightmap_coord,
                end.lightmap_coord,
                fraction,
            ),
            color: qa_core::math::lerp_array(self.color, end.color, fraction),
            indexed_light: qa_core::math::lerp(self.indexed_light, end.indexed_light, fraction),
        }
    }
}

#[derive(Clone, Copy, Default)]
struct ScreenVertex {
    xy: [f32; 2],
    inverse_depth: f32,
    texcoord_over_depth: [f32; 2],
    lightmap_over_depth: [f32; 2],
    color_over_depth: [f32; 4],
    indexed_texcoord: [f32; 2],
    indexed_light: f32,
}

impl ScreenVertex {
    fn with_attributes(self, vertex: ClipVertex) -> Self {
        Self {
            texcoord_over_depth: vertex.texcoord.map(|value| value * self.inverse_depth),
            lightmap_over_depth: vertex
                .lightmap_coord
                .map(|value| value * self.inverse_depth),
            color_over_depth: vertex.color.map(|value| value * self.inverse_depth),
            indexed_texcoord: vertex.texcoord,
            indexed_light: vertex.indexed_light,
            ..self
        }
    }

    fn finite(self) -> bool {
        self.xy
            .iter()
            .chain(self.texcoord_over_depth.iter())
            .chain(self.lightmap_over_depth.iter())
            .chain(self.color_over_depth.iter())
            .all(|v| v.is_finite())
            && self.inverse_depth.is_finite()
            && self.indexed_light.is_finite()
    }
}

struct Camera {
    refdef: Refdef,
    tangent: [f32; 2],
    screen_scale: [f32; 2],
    screen_center: [f32; 2],
    clip_plane: Option<qa_core::primitives::Plane>,
}

#[derive(Clone, Copy)]
struct RasterPass<'a> {
    material: &'a Material,
    stage: PreparedStage,
    image: &'a Image,
    sampler: ShadeFn,
    depth_hack: bool,
    first_stage: bool,
    draw_rank: u32,
    indexed: Option<indexed::Raster<'a>>,
}

#[derive(Clone, Copy)]
enum GradientForm {
    ScaleThenFactor,
    FactorFolded,
}

impl Camera {
    fn load(refdef: Refdef, width: u32, height: u32) -> Option<Self> {
        let tangent = refdef.validate(width, height)?;
        if refdef.palette_transform.is_some_and(|transform| {
            transform.shifts.iter().any(|shift| {
                !(0..=256).contains(&shift.percent)
                    || shift
                        .destination
                        .iter()
                        .any(|value| !(0..=255).contains(value))
            })
        }) {
            return None;
        }
        let viewport = refdef.viewport;
        let screen_scale = [
            viewport.width as f32 / (2.0 * tangent[0]),
            viewport.height as f32 / (2.0 * tangent[1]),
        ];
        let screen_center = [
            viewport.x as f32 + viewport.width as f32 * 0.5 - 0.5,
            viewport.y as f32 + viewport.height as f32 * 0.5 - 0.5,
        ];
        Some(Self {
            clip_plane: refdef.clip_plane.map(|plane| {
                plane.to_local(
                    refdef.origin,
                    [-refdef.axes[1], refdef.axes[2], refdef.axes[0]],
                )
            }),
            refdef,
            tangent,
            screen_scale,
            screen_center,
        })
    }

    fn screen_scale(&self) -> [f32; 2] {
        self.screen_scale
    }
    fn screen_center(&self) -> [f32; 2] {
        self.screen_center
    }
    fn for_world_entity(&self, entity: &SceneEntity) -> Option<Self> {
        if !entity.origin.is_finite()
            || entity.non_normalized_axes
            || entity.depth_hack
            || entity
                .axes
                .iter()
                .any(|axis| !axis.is_finite() || (axis.dot(*axis) - 1.0).abs() > 1.0e-4)
            || [(0, 1), (0, 2), (1, 2)]
                .into_iter()
                .any(|(a, b)| entity.axes[a].dot(entity.axes[b]).abs() > 1.0e-4)
        {
            return None;
        }
        let origin =
            qa_core::math::inverse_transform_point(entity.origin, entity.axes, self.refdef.origin);
        let axes = self
            .refdef
            .axes
            .map(|axis| qa_core::math::inverse_transform_point(Vec3::default(), entity.axes, axis));
        if !origin.is_finite() || !axes.iter().all(|axis| axis.is_finite()) {
            return None;
        }
        Some(Self {
            refdef: Refdef {
                origin,
                axes,
                ..self.refdef
            },
            tangent: self.tangent,
            screen_scale: self.screen_scale,
            screen_center: self.screen_center,
            clip_plane: self.clip_plane,
        })
    }
    fn gradient(&self, axis: Vec3, factor: f32, form: GradientForm) -> world::Affine {
        let scale = self.screen_scale();
        let center = self.screen_center();
        let component = |value: f32, scale: f32| match form {
            GradientForm::ScaleThenFactor => value * (1.0 / scale) * factor,
            GradientForm::FactorFolded => value * ((1.0 / scale) * factor),
        };
        let x = component(axis.0[0], scale[0]);
        let y = component(-axis.0[1], scale[1]);
        world::Affine {
            x,
            y,
            origin: axis.0[2] * factor - center[0] * x - center[1] * y,
        }
    }

    fn vertex(&self, vertex: Vertex, world_position: Vec3, color: [f32; 4]) -> ClipVertex {
        ClipVertex::from_vertex(vertex, self.to_view(world_position), color)
    }

    fn rgb(&self) -> bool {
        matches!(self.refdef.cpu_presentation, CpuPresentation::Rgb)
    }

    fn to_view(&self, point: Vec3) -> Vec3 {
        view_point(self.refdef.origin, self.refdef.axes, point)
    }

    fn direction_to_view(&self, direction: Vec3) -> Vec3 {
        // Native gradients negate the axis before the dot; vertex projection
        // negates its result. Keep cancellation's signed zero at each boundary.
        qa_core::math::inverse_transform_point(
            Vec3::default(),
            [
                -self.refdef.axes[1],
                self.refdef.axes[2],
                self.refdef.axes[0],
            ],
            direction,
        )
    }

    fn project(&self, vertex: ClipVertex) -> ScreenVertex {
        let inverse_depth = 1.0 / vertex.camera.0[2];
        let viewport = self.refdef.viewport;
        ScreenVertex {
            xy: [
                viewport.x as f32
                    + viewport.width as f32
                        * 0.5
                        * (1.0 + vertex.camera.0[0] * inverse_depth / self.tangent[0]),
                viewport.y as f32
                    + viewport.height as f32
                        * 0.5
                        * (1.0 - vertex.camera.0[1] * inverse_depth / self.tangent[1]),
            ],
            inverse_depth,
            ..ScreenVertex::default()
        }
        .with_attributes(vertex)
    }

    fn distance(&self, vertex: ClipVertex, plane: usize) -> f32 {
        let [x, y, z] = vertex.camera.0;
        match plane {
            0 => z - self.refdef.near,
            1 => self.refdef.far - z,
            2 => x + z * self.tangent[0],
            3 => z * self.tangent[0] - x,
            4 => y + z * self.tangent[1],
            5 => z * self.tangent[1] - y,
            _ => self
                .clip_plane
                .map_or(1., |plane| plane.signed_distance(vertex.camera)),
        }
    }
    fn clip_plane_count(&self) -> usize {
        6 + usize::from(self.clip_plane.is_some())
    }
}

impl CpuBackend {
    pub fn load(width: u32, height: u32) -> Result<Self, &'static str> {
        if width == 0 || height == 0 || width > 8192 || height > 8192 {
            return Err("invalid CPU framebuffer dimensions");
        }
        Ok(Self {
            frame: Framebuffer::load(width, height)?,
            clear_color: [0; 4],
            material_frame: Framebuffer::load(width, height)?,
            warp_frame: Framebuffer::load(320, 240)?,
            warp_material_frame: Framebuffer::load(320, 240)?,
            warp_sine: (0..width.max(height) as usize + 128)
                .map(|i| (3.0 + (i as f64 * 3.14159 * 2.0 / 128.0).sin() * 3.0) as u8)
                .collect(),
            warp_columns: vec![0; width as usize + 6].into_boxed_slice(),
            warp_rows: vec![0; height as usize + 6].into_boxed_slice(),
            warp_dimensions: None,
            world: None,
            limits: CpuLimits::default(),
            admission: None,
            evaluator: StageEvaluator::load(),
            presentation: CpuPresentation::Rgb,
            time_ms: 0,
        })
    }

    /// Prepare the initial asset prefix and its load-sized world scratch.
    pub fn load_with_assets(
        width: u32,
        height: u32,
        assets: &Assets,
    ) -> Result<Self, &'static str> {
        Self::load_with_limits(width, height, assets, CpuLimits::default())
    }
    pub fn load_with_limits(
        width: u32,
        height: u32,
        assets: &Assets,
        limits: CpuLimits,
    ) -> Result<Self, &'static str> {
        let mut backend = Self::load(width, height)?;
        backend.limits = limits;
        backend.prepare_assets(assets, None)?;
        Ok(backend)
    }

    /// Cold preparation requires all prior scoped jobs to have completed.
    /// Failed candidates leave the admitted raster and framebuffer untouched.
    pub fn prepare_assets(
        &mut self,
        assets: &Assets,
        scene: Option<crate::scene::Limits>,
    ) -> Result<(), &'static str> {
        let previous = self.limits.scene.resolved()?;
        let scene = scene.unwrap_or(previous).validate_growth(previous)?;
        let admission = AssetAdmission::prepare(self.admission.as_ref(), assets)?;
        if admission.is_none() && scene == previous {
            return Ok(());
        }
        let mut limits = self.limits;
        limits.scene = scene;
        if let Some(world) = &self.world {
            limits.bands = world.config().bands;
            limits.auto_bands = false;
        }
        let world = world::WorldRaster::load(
            self.frame.width,
            self.frame.height,
            assets,
            limits,
            &self.evaluator,
        )?;
        self.world = Some(world);
        self.limits.scene = scene;
        if let Some(admission) = admission {
            self.admission = Some(admission);
        }
        Ok(())
    }
    pub fn admitted_assets(&self) -> Option<AssetCounts> {
        self.admission.as_ref().map(AssetAdmission::counts)
    }
    pub fn world_stats(&self) -> WorldStats {
        self.world
            .as_ref()
            .map_or(WorldStats::default(), |w| w.stats())
    }

    pub fn pixels(&self) -> &[u32] {
        &self.frame.pixels
    }

    pub fn inverse_depth(&self) -> &[f32] {
        &self.frame.inverse_depth
    }

    pub fn dimensions(&self) -> [u32; 2] {
        [self.frame.width, self.frame.height]
    }

    pub fn raster_config(&self) -> RasterConfig {
        let mut config = self
            .world
            .as_ref()
            .map_or(RasterConfig::default(), |world| world.config());
        config.material_capacity_bytes = self.material_frame.capacity_bytes();
        config.view_effect_capacity_bytes = self.warp_frame.capacity_bytes()
            + self.warp_material_frame.capacity_bytes()
            + std::mem::size_of_val(&*self.warp_sine)
            + std::mem::size_of_val(&*self.warp_columns)
            + std::mem::size_of_val(&*self.warp_rows);
        config
    }
    pub fn band_stats(&self, output: &mut [WorldStats; MAX_BANDS]) -> usize {
        if let Some(world) = &self.world {
            world.band_stats(output)
        } else {
            output.fill(WorldStats::default());
            0
        }
    }

    pub fn render(&mut self, list: &CommandList, assets: &Assets) -> BackendStats {
        let result: Result<BackendStats, std::convert::Infallible> =
            self.render_with_dispatch(list, assets, |jobs| {
                for job in jobs {
                    run_cpu_job(job);
                }
                Ok(())
            });
        match result {
            Ok(stats) => stats,
            Err(never) => match never {},
        }
    }

    /// The dispatcher must complete each job before returning. An error stops
    /// this packet; callers can inspect attempted band counts and skip presenting.
    pub fn render_with_dispatch<E>(
        &mut self,
        list: &CommandList,
        assets: &Assets,
        dispatch: impl for<'job> FnMut(&mut [CpuJob<'job>]) -> Result<(), E>,
    ) -> Result<BackendStats, E> {
        self.render_observed(list, assets, dispatch, |_| {})
    }

    /// Optional stage observation owns no clock and shares the ordinary draw
    /// implementation. The normal entry's empty observer is optimized away.
    pub fn render_observed<E>(
        &mut self,
        list: &CommandList,
        assets: &Assets,
        mut dispatch: impl for<'job> FnMut(&mut [CpuJob<'job>]) -> Result<(), E>,
        mut observe: impl FnMut(PreparePoint),
    ) -> Result<BackendStats, E> {
        let mut stats = BackendStats {
            rejected: list.rejected.min(u32::MAX as u64) as u32,
            ..BackendStats::default()
        };
        if let Some(world) = &mut self.world {
            world.reset_stats();
        }
        for command in list.commands() {
            match *command {
                Command::Empty => {}
                Command::Clear(color) => {
                    self.frame.clear(color);
                    self.clear_color = color;
                    self.presentation = CpuPresentation::Rgb;
                }
                Command::View(view) => {
                    if view
                        .refdef
                        .validate(self.frame.width, self.frame.height)
                        .is_none()
                    {
                        stats.rejected = stats.rejected.saturating_add(1);
                        continue;
                    }
                    let warped = view.refdef.software_waterwarp
                        && view
                            .refdef
                            .flags
                            .contains(qa_core::primitives::PlayerViewFlags::UNDERWATER)
                        && !view.refdef.hyperspace;
                    let mut raster = view.refdef;
                    if warped {
                        raster.viewport = Viewport {
                            width: view.refdef.viewport.width.min(320),
                            height: view.refdef.viewport.height.min(240),
                            ..Viewport::default()
                        };
                        raster.blend_viewport = None;
                    }
                    let (width, height) = if warped {
                        (320, 240)
                    } else {
                        (self.frame.width, self.frame.height)
                    };
                    let Some(camera) = Camera::load(raster, width, height) else {
                        stats.rejected = stats.rejected.saturating_add(1);
                        continue;
                    };
                    if warped {
                        self.warp_frame.clear(self.clear_color);
                        std::mem::swap(&mut self.frame, &mut self.warp_frame);
                        std::mem::swap(&mut self.material_frame, &mut self.warp_material_frame);
                    }
                    let result = (|| -> Result<(), E> {
                        stats.views = stats.views.saturating_add(1);
                        stats.pending_lights =
                            stats.pending_lights.saturating_add(view.scene.lights.count);
                        self.presentation = camera.refdef.cpu_presentation;
                        self.time_ms = camera.refdef.time_ms;
                        self.clear_depth(camera.refdef.viewport, camera.refdef.far);
                        if camera.refdef.hyperspace {
                            let color = u32::from_le_bytes(camera.refdef.hyperspace_color());
                            let width = self.frame.width;
                            let mut buffers = self.frame.buffers();
                            if let Some(rows) = buffers.rows(width, camera.refdef.viewport) {
                                for y in rows {
                                    for x in camera.refdef.viewport.x
                                        ..camera.refdef.viewport.x + camera.refdef.viewport.width
                                    {
                                        let index = buffers.offset(width, x, y);
                                        buffers.write_rgba(index, color, None);
                                    }
                                }
                            }
                            return Ok(());
                        }
                        if let Some(world) = &mut self.world {
                            observe(PreparePoint::Begin);
                            let prepared = world.prepare_view_dispatched(
                                &camera,
                                list,
                                view.scene,
                                assets,
                                &self.evaluator,
                                &mut stats,
                                &mut dispatch,
                            );
                            observe(PreparePoint::End);
                            if prepared? {
                                world.render_opaque(
                                    &camera,
                                    assets,
                                    self.frame.buffers(),
                                    &mut stats,
                                    &mut dispatch,
                                )?;
                            }
                        } else {
                            stats.rejected =
                                stats.rejected.saturating_add(view.scene.surfaces.count);
                        }
                        let draws = list.draws(view.scene.draws);
                        let mut draw_rank = 0;
                        while draw_rank < draws.len() {
                            let item = draws[draw_rank];
                            let opacity = crate::scene::draw_entity(list, item)
                                .and_then(|entity| entity.material_opacity);
                            if opacity.is_some_and(|value| value.value() == 0.0) {
                                draw_rank += 1;
                                continue;
                            }
                            if opacity.is_some() {
                                self.material_frame.seed_from(&self.frame);
                                std::mem::swap(&mut self.frame, &mut self.material_frame);
                            }
                            let before = stats.rejected;
                            let result = self.draw_item(
                                &camera,
                                list,
                                view.scene.draws,
                                assets,
                                draw_rank,
                                &mut stats,
                                &mut dispatch,
                            );
                            if let Some(opacity) = opacity {
                                // Every dispatcher loan has ended, including errors. Restore
                                // scene ownership before propagating a failed dispatch.
                                std::mem::swap(&mut self.frame, &mut self.material_frame);
                                if result.is_ok() && stats.rejected == before {
                                    self.frame.composite_material(
                                        &self.material_frame,
                                        opacity,
                                        &camera,
                                        assets,
                                    );
                                }
                            }
                            draw_rank = result?;
                        }
                        particles::draw(
                            &camera,
                            self.frame.width,
                            list.particles(view.scene.particles),
                            assets,
                            self.frame.buffers(),
                            &mut stats,
                        );
                        Ok(())
                    })();
                    if warped {
                        std::mem::swap(&mut self.frame, &mut self.warp_frame);
                        std::mem::swap(&mut self.material_frame, &mut self.warp_material_frame);
                        if result.is_ok() {
                            self.resolve_water(&view.refdef);
                        }
                    }
                    result?;
                    if !view.refdef.hyperspace {
                        if view.refdef.blend_phase == BlendPhase::AfterView {
                            self.view_blend(&view.refdef, false, assets);
                        }
                        self.damage_vignette(&view.refdef, assets);
                    }
                }
                Command::Draw2d(draw) => {
                    self.draw_2d(draw, assets, &mut stats);
                }
            }
        }
        // Native indexed palette shifts run after the submitted HUD/console.
        for command in list.commands() {
            if let Command::View(view) = *command
                && view.refdef.blend_phase == BlendPhase::FinalPalette
                && let Some(camera) = Camera::load(view.refdef, self.frame.width, self.frame.height)
            {
                self.view_blend(&camera.refdef, true, assets);
            }
        }
        Ok(stats)
    }

    fn clear_depth(&mut self, viewport: Viewport, far: f32) {
        // The GL clear value and depthRange(1,1) both represent the view's far
        // plane. Keep that equality in the shared inverse-depth surface.
        let far_depth = 1.0 / far;
        let width = self.frame.width;
        let buffers = self.frame.buffers();
        if let Some(rows) = buffers.rows(width, viewport) {
            for y in rows {
                let start = buffers.offset(width, viewport.x, y);
                buffers.inverse_depth[start..start + viewport.width as usize].fill(far_depth);
                buffers.depth_ranks[start..start + viewport.width as usize].fill(0);
            }
        }
    }

    fn resolve_water(&mut self, view: &Refdef) {
        let viewport = view.viewport;
        let phase =
            qa_core::math::native_integer(f64::from(view.time_seconds() * 20.0)) as usize & 127;
        if self.warp_dimensions != Some((viewport.width, viewport.height)) {
            for (x, column) in self.warp_columns[..viewport.width as usize + 6]
                .iter_mut()
                .enumerate()
            {
                *column = (x as f32 / (viewport.width + 6) as f32 * viewport.width.min(320) as f32)
                    as usize;
            }
            for (y, row) in self.warp_rows[..viewport.height as usize + 6]
                .iter_mut()
                .enumerate()
            {
                *row = (y as f32 / (viewport.height + 6) as f32 * viewport.height.min(240) as f32)
                    as usize;
            }
            self.warp_dimensions = Some((viewport.width, viewport.height));
        }
        let target_width = self.frame.width;
        let source_width = self.warp_frame.width;
        let target = self.frame.buffers();
        let source = self.warp_frame.buffers();
        for y in 0..viewport.height {
            for x in 0..viewport.width {
                let source_x =
                    self.warp_columns[x as usize + usize::from(self.warp_sine[phase + y as usize])];
                let source_y =
                    self.warp_rows[y as usize + usize::from(self.warp_sine[phase + x as usize])];
                let source_index = source.offset(source_width, source_x as u32, source_y as u32);
                let destination = target.offset(target_width, viewport.x + x, viewport.y + y);
                target.pixels[destination] = source.pixels[source_index];
                target.indices[destination] = source.indices[source_index];
                target.palettes[destination] = source.palettes[source_index];
                target.inverse_depth[destination] = source.inverse_depth[source_index];
                target.depth_ranks[destination] = source.depth_ranks[source_index];
            }
        }
    }

    fn poly(
        &mut self,
        camera: &Camera,
        poly: &Poly,
        draw_rank: u32,
        list: &CommandList,
        assets: &Assets,
        stats: &mut BackendStats,
    ) {
        let Some(binding) = assets.shader_binding(poly.shader) else {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        };
        let Some(material) = assets.material(binding.material) else {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        };
        if !self.mesh_supported(material) {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        }
        let vertices = list.vertices(poly.vertices);
        let inputs = DrawInputs {
            lightmap: binding.lightmap,
            ..DrawInputs::for_view(&camera.refdef).for_poly(poly, list)
        };
        self.mesh_stages(
            camera,
            material,
            inputs,
            false,
            draw_rank,
            |index| vertices[index],
            |index| vertices[index].normal,
            crate::affine::fan(vertices.len()),
            |position| position,
            assets,
            stats,
        );
    }

    #[expect(
        clippy::chunks_exact_to_as_chunks,
        reason = "Retain packed native pixel or triangle traversal and its incomplete-tail behavior"
    )]
    fn draw_item<E>(
        &mut self,
        camera: &Camera,
        list: &CommandList,
        draws: crate::scene::Span,
        assets: &Assets,
        rank: usize,
        stats: &mut BackendStats,
        dispatch: &mut impl for<'job> FnMut(&mut [CpuJob<'job>]) -> Result<(), E>,
    ) -> Result<usize, E> {
        if let Some(world) = &mut self.world
            && let Some(end) = world.draw_run(
                camera,
                rank,
                list,
                assets,
                self.frame.buffers(),
                stats,
                dispatch,
            )?
        {
            return Ok(end);
        }
        let item = list.draws(draws)[rank];
        match item.kind {
            DrawKind::Entity => {
                self.entity(camera, list.entity(item.index), rank as u32, assets, stats)
            }
            DrawKind::Poly => self.poly(
                camera,
                list.poly(item.index),
                rank as u32,
                list,
                assets,
                stats,
            ),
            DrawKind::Surface => {}
        }
        Ok(rank + 1)
    }

    fn entity(
        &mut self,
        camera: &Camera,
        entity: &SceneEntity,
        draw_rank: u32,
        assets: &Assets,
        stats: &mut BackendStats,
    ) {
        let Some(model) = entity.model.mesh().and_then(|id| assets.model(id)) else {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        };
        let Some(frames) = model.posed_frames(entity) else {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        };
        let Some(binding) = entity
            .shader(assets)
            .and_then(|id| assets.shader_binding(id))
        else {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        };
        let Some(material) = assets.material(binding.material) else {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        };
        if !entity.origin.is_finite() || !entity.axes.iter().all(|axis| axis.is_finite()) {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        }
        if !self.mesh_supported(material) {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        }
        let inputs = DrawInputs {
            lightmap: binding.lightmap,
            ..DrawInputs::for_view(&camera.refdef).for_entity(entity)
        };
        self.mesh_stages(
            camera,
            material,
            inputs,
            entity.depth_hack,
            draw_rank,
            |index| frames.vertex(index),
            |index| frames.current_vertex(index).normal,
            model.indices.chunks_exact(3).map(|indices| {
                [
                    indices[0] as usize,
                    indices[1] as usize,
                    indices[2] as usize,
                ]
            }),
            |position| qa_core::math::transform_point(entity.origin, entity.axes, position),
            assets,
            stats,
        );
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Shared mesh stages preserve each submission's transform and depth policy"
    )]
    fn mesh_stages(
        &mut self,
        camera: &Camera,
        material: &Material,
        inputs: DrawInputs,
        depth_hack: bool,
        draw_rank: u32,
        vertex: impl Fn(usize) -> Vertex,
        lighting_normal: impl Fn(usize) -> Vec3,
        triangles: impl Iterator<Item = [usize; 3]> + Clone,
        to_world: impl Fn(Vec3) -> Vec3,
        assets: &Assets,
        stats: &mut BackendStats,
    ) {
        if material
            .settings
            .software_alias
            .is_some_and(|alias| alias.missing_skin)
        {
            return;
        }
        let Ok(deforms) = self.evaluator.prepare_deforms(&material.settings, &inputs) else {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        };
        for (stage_index, stage) in material.stages.iter().enumerate() {
            let Ok(stage) = self.evaluator.prepare(stage, material.settings, inputs) else {
                stats.rejected = stats.rejected.saturating_add(1);
                continue;
            };
            let Some(image) = assets.image(stage.image) else {
                stats.rejected = stats.rejected.saturating_add(1);
                continue;
            };
            let Ok(indexed) = indexed::Raster::prepare(
                camera.refdef.cpu_presentation,
                image,
                &stage,
                material.settings.software_alias,
                inputs.lighting,
                assets,
            ) else {
                stats.rejected = stats.rejected.saturating_add(1);
                continue;
            };
            let pass = RasterPass {
                material,
                stage,
                image,
                sampler: stage_sampler(image, stage.stage.sampler),
                depth_hack,
                first_stage: stage_index == 0,
                draw_rank,
                indexed,
            };
            stats.stages = stats.stages.saturating_add(1);
            for indices in triangles.clone() {
                let triangle = indices.map(|i| {
                    let vertex =
                        world::evaluated_vertex(vertex(i), deforms, Some(stage), &self.evaluator);
                    let mut clipped = camera.vertex(
                        vertex,
                        to_world(vertex.position),
                        stage.unit_color(vertex.color),
                    );
                    if let Some(indexed) = indexed {
                        clipped.indexed_light = indexed.light(lighting_normal(i), vertex.color);
                    }
                    clipped
                });
                self.triangle(camera, triangle, pass, stats);
            }
        }
    }

    fn mesh_supported(&self, material: &Material) -> bool {
        material.settings.sky.is_none()
            && material.settings.fog.is_none()
            && !material.settings.polygon_offset
    }

    fn triangle(
        &mut self,
        camera: &Camera,
        triangle: [ClipVertex; 3],
        pass: RasterPass<'_>,
        stats: &mut BackendStats,
    ) {
        if triangle.iter().any(|v| !v.finite()) {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        }
        let mut vertices = [ClipVertex::default(); CLIP_VERTICES];
        vertices[..3].copy_from_slice(&triangle);
        let mut count = 3;
        for plane in 0..camera.clip_plane_count() {
            let mut output = [ClipVertex::default(); CLIP_VERTICES];
            let Some(next_count) = clip_plane(camera, &vertices[..count], &mut output, plane)
            else {
                stats.rejected = stats.rejected.saturating_add(1);
                return;
            };
            if next_count < 3 {
                return;
            }
            vertices = output;
            count = next_count;
        }
        let first = camera.project(vertices[0]);
        for [_, i, j] in crate::affine::fan(count) {
            self.raster_triangle(
                camera,
                [
                    first,
                    camera.project(vertices[i]),
                    camera.project(vertices[j]),
                ],
                pass,
                stats,
            );
        }
    }

    fn raster_triangle(
        &mut self,
        camera: &Camera,
        mut vertices: [ScreenVertex; 3],
        pass: RasterPass<'_>,
        stats: &mut BackendStats,
    ) {
        if vertices.iter().any(|vertex| !vertex.finite()) {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        }
        let mut area = crate::affine::cross2(vertices[0].xy, vertices[1].xy, vertices[2].xy);
        // Q3 CT_FRONT_SIDED culls GL_FRONT with GL_CCW. The retained winding
        // therefore has positive area in this top-left-origin framebuffer.
        if culled(
            pass.material
                .settings
                .cull
                .for_view(camera.refdef.reflected_view),
            area,
        ) {
            return;
        }
        if area < 0.0 {
            vertices.swap(1, 2);
            area = -area;
        }
        if pass.first_stage {
            stats.triangles = stats.triangles.saturating_add(1);
        }
        let viewport = camera.refdef.viewport;
        let (x_start, x_end) = pixel_bounds(
            vertices.map(|v| v.xy[0]),
            viewport.x,
            viewport.x + viewport.width,
        );
        let (y_start, y_end) = pixel_bounds(
            vertices.map(|v| v.xy[1]),
            viewport.y,
            viewport.y + viewport.height,
        );
        let edges = [
            (vertices[1].xy, vertices[2].xy),
            (vertices[2].xy, vertices[0].xy),
            (vertices[0].xy, vertices[1].xy),
        ];
        let include_edge = edges.map(|(a, b)| top_left(a, b));
        let area_inverse = 1.0 / area;
        let mip_planes = (pass.indexed.is_none()
            && image_mips(pass.image, pass.stage.stage.sampler) > 1)
            .then(|| {
                let points = vertices.map(|v| v.xy.map(f64::from));
                let determinant = crate::affine::determinant(points);
                (
                    world::Affine::fit(points, vertices.map(|v| v.inverse_depth), determinant),
                    std::array::from_fn(|coordinate| {
                        world::Affine::fit(
                            points,
                            vertices.map(|v| v.texcoord_over_depth[coordinate]),
                            determinant,
                        )
                    }),
                )
            });
        let width = self.frame.width;
        let mut buffers = self.frame.buffers();
        for y in y_start..y_end {
            let mip = triangle_mip(
                pass.image,
                pass.stage.stage.sampler,
                mip_planes,
                [(x_start as f32 + x_end as f32) * 0.5, y as f32 + 0.5],
            );
            let texels = TexelView::image(pass.image, mip, pass.stage.stage.texture_intensity);
            for x in x_start..x_end {
                let sample = [x as f32 + 0.5, y as f32 + 0.5];
                let weights = edges.map(|(a, b)| crate::affine::cross2(a, b, sample));
                if weights
                    .iter()
                    .zip(include_edge)
                    .any(|(&weight, inclusive)| weight < 0.0 || (weight == 0.0 && !inclusive))
                {
                    continue;
                }
                let weights = Vec3(weights.map(|w| w * area_inverse));
                let inverse_depth = weights.dot(Vec3(vertices.map(|v| v.inverse_depth)));
                let depth_score = if pass.depth_hack {
                    inverse_depth * 0.3 + 0.7 / camera.refdef.near
                } else {
                    inverse_depth
                };
                let index = buffers.offset(width, x, y);
                if !buffers.depth_test(
                    index,
                    inverse_depth,
                    depth_score,
                    pass.draw_rank,
                    pass.stage.stage.depth_func,
                ) {
                    continue;
                }
                let depth = 1.0 / inverse_depth;
                let coordinates = std::array::from_fn(|i| {
                    if pass.indexed.is_some() {
                        return weights.dot(Vec3(vertices.map(|v| v.indexed_texcoord[i])));
                    }
                    weights.dot(Vec3(vertices.map(|v| v.texcoord_over_depth[i]))) * depth
                });
                let color = std::array::from_fn(|i| {
                    weights.dot(Vec3(vertices.map(|v| v.color_over_depth[i]))) * depth
                });
                if let Some(indexed) = pass.indexed {
                    if !alpha_pass(pass.stage.stage.alpha_test, color[3]) {
                        continue;
                    }
                    let light = weights.dot(Vec3(vertices.map(|v| v.indexed_light)));
                    let Some((color, write_depth)) = indexed.sample(
                        coordinates,
                        light,
                        color[3],
                        buffers.indices[index],
                        buffers.palettes[index],
                    ) else {
                        continue;
                    };
                    buffers.write_indexed(
                        index,
                        color,
                        indexed.palette,
                        indexed.palette_id.0,
                        write_depth.then_some((depth_score, pass.draw_rank)),
                    );
                    continue;
                }
                let source = (pass.sampler)(texels, coordinates, color);
                if !alpha_pass(pass.stage.stage.alpha_test, source[3]) {
                    continue;
                }
                let color = composite(buffers.pixels[index], source, pass.stage.stage.blend);
                buffers.write_rgba(
                    index,
                    color,
                    pass.stage
                        .stage
                        .depth_write
                        .then_some((depth_score, pass.draw_rank)),
                );
            }
        }
    }

    fn draw_2d(&mut self, draw: Draw2d, assets: &Assets, stats: &mut BackendStats) {
        let Some(binding) = assets.shader_binding(draw.shader) else {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        };
        let Some(material) = assets.material(binding.material) else {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        };
        if !draw
            .rect
            .iter()
            .chain(draw.texcoords.iter())
            .all(|f| f.is_finite())
            || draw.rect[2] == 0.0
            || draw.rect[3] == 0.0
            || !(draw.rect[0] + draw.rect[2]).is_finite()
            || !(draw.rect[1] + draw.rect[3]).is_finite()
        {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        }
        stats.draws_2d = stats.draws_2d.saturating_add(1);
        let (x_start, x_end) = pixel_bounds(
            [draw.rect[0], draw.rect[0] + draw.rect[2], draw.rect[0]],
            0,
            self.frame.width,
        );
        let (y_start, y_end) = pixel_bounds(
            [draw.rect[1], draw.rect[1] + draw.rect[3], draw.rect[1]],
            0,
            self.frame.height,
        );
        if material.settings.sky.is_some()
            || material.settings.fog.is_some()
            || material.settings.portal
            || material.settings.deforms.iter().any(Option::is_some)
        {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        }
        let inputs = DrawInputs {
            time_ms: self.time_ms,
            entity_color: draw.color,
            lightmap: binding.lightmap,
            ..DrawInputs::default()
        };
        for stage in material.stages.iter() {
            let Ok(prepared) = self.evaluator.prepare(stage, material.settings, inputs) else {
                stats.rejected = stats.rejected.saturating_add(1);
                continue;
            };
            let Some(image) = assets.image(prepared.image) else {
                stats.rejected = stats.rejected.saturating_add(1);
                continue;
            };
            let indexed = match self.presentation {
                CpuPresentation::Rgb => None,
                CpuPresentation::Indexed { palette, .. } => {
                    let Some(texture) = image.indexed.as_ref() else {
                        stats.rejected = stats.rejected.saturating_add(1);
                        continue;
                    };
                    // An opaque alpha-blended indexed image is still an exact copy.
                    if (stage.blend.is_some() && stage.blend != Some(StageBlend::ALPHA))
                        || draw.color != [255; 4]
                    {
                        stats.rejected = stats.rejected.saturating_add(1);
                        continue;
                    }
                    let Some(palette_resource) = assets.palette(palette) else {
                        stats.rejected = stats.rejected.saturating_add(1);
                        continue;
                    };
                    let Some(mip) = texture.mip(0) else {
                        stats.rejected = stats.rejected.saturating_add(1);
                        continue;
                    };
                    Some((palette, palette_resource, texture, mip))
                }
            };
            let sampler = stage_sampler(image, stage.sampler);
            let origin = [x_start as f32, y_start as f32];
            let coordinates = [
                origin,
                [origin[0] + 1.0, origin[1]],
                [origin[0], origin[1] + 1.0],
            ]
            .map(|at| {
                self.evaluator
                    .evaluate(&prepared, &draw_vertex(draw, at))
                    .texcoord
            });
            let mip = image_mip(
                image,
                stage.sampler,
                std::array::from_fn(|axis| {
                    std::array::from_fn(|coordinate| {
                        coordinates[axis + 1][coordinate] - coordinates[0][coordinate]
                    })
                }),
            );
            let texels = TexelView::image(image, mip, stage.texture_intensity);
            let width = self.frame.width;
            let mut buffers = self.frame.buffers();
            for y in y_start..y_end {
                for x in x_start..x_end {
                    let vertex = draw_vertex(draw, [x as f32, y as f32]);
                    let evaluated = self.evaluator.evaluate(&prepared, &vertex);
                    let index = buffers.offset(width, x, y);
                    if let Some((palette_id, palette, texture, mip)) = indexed {
                        if evaluated.color != [255; 4] {
                            continue;
                        }
                        let tx = texel(evaluated.texcoord[0], mip.width, stage.sampler.wrap);
                        let ty = texel(evaluated.texcoord[1], mip.height, stage.sampler.wrap);
                        let color = mip.indices()[ty * mip.width as usize + tx];
                        if texture.transparent_index() == Some(color) {
                            continue;
                        }
                        buffers.write_indexed(index, color, palette, palette_id.0, None);
                    } else {
                        let source = sampler(
                            texels,
                            evaluated.texcoord,
                            prepared.unit_color(evaluated.color),
                        );
                        if alpha_pass(stage.alpha_test, source[3]) {
                            let blend = stage.blend.or(Some(StageBlend::ALPHA));
                            let color = composite(buffers.pixels[index], source, blend);
                            buffers.write_rgba(index, color, None);
                        }
                    }
                }
            }
        }
    }

    fn view_blend(&mut self, refdef: &Refdef, final_phase: bool, assets: &Assets) {
        match refdef.cpu_presentation {
            CpuPresentation::Rgb => self.tint(refdef, final_phase),
            CpuPresentation::Indexed { palette, .. } => {
                let Some(transform) = refdef.palette_transform else {
                    return;
                };
                let Some(resource) = assets.palette(palette) else {
                    return;
                };
                let transformed = transformed_palette(resource, &transform, refdef.blend);
                let viewport = if final_phase {
                    refdef.blend_viewport.unwrap_or(refdef.viewport)
                } else {
                    refdef.viewport
                };
                let width = self.frame.width;
                let buffers = self.frame.buffers();
                if let Some(rows) = buffers.rows(width, viewport) {
                    for y in rows {
                        let start = buffers.offset(width, viewport.x, y);
                        for index in start..start + viewport.width as usize {
                            if buffers.palettes[index] == palette.0 {
                                buffers.pixels[index] =
                                    transformed[buffers.indices[index] as usize];
                            }
                        }
                    }
                }
            }
        }
    }

    fn tint(&mut self, refdef: &Refdef, preserve_alpha: bool) {
        let source = refdef.blend.map(|c| c.clamp(0.0, 1.0));
        if source[3] == 0.0 {
            return;
        }
        let viewport = if preserve_alpha {
            refdef.blend_viewport.unwrap_or(refdef.viewport)
        } else {
            refdef.viewport
        };
        let width = self.frame.width;
        let buffers = self.frame.buffers();
        if let Some(rows) = buffers.rows(width, viewport) {
            for y in rows {
                let start = buffers.offset(width, viewport.x, y);
                for pixel in &mut buffers.pixels[start..start + viewport.width as usize] {
                    let alpha = *pixel & 0xff00_0000;
                    *pixel = composite(*pixel, source, Some(StageBlend::ALPHA));
                    if preserve_alpha {
                        *pixel = (*pixel & 0x00ff_ffff) | alpha;
                    }
                }
            }
        }
    }

    fn damage_vignette(&mut self, refdef: &Refdef, assets: &Assets) {
        let Some(vignette) = crate::scene::DamageVignette::load(refdef) else {
            return;
        };
        let viewport = refdef.viewport;
        let palette = match refdef.cpu_presentation {
            CpuPresentation::Indexed { palette, .. } => {
                assets.palette(palette).map(|resource| (palette, resource))
            }
            CpuPresentation::Rgb => None,
        };
        let width = self.frame.width;
        let mut buffers = self.frame.buffers();
        if let Some(rows) = buffers.rows(width, viewport) {
            for y in rows {
                for x in viewport.x..viewport.x + viewport.width {
                    let color = vignette.color_at(viewport, x, y);
                    if color[3] == 0.0 {
                        continue;
                    }
                    let index = buffers.offset(width, x, y);
                    let pixel = composite(buffers.pixels[index], color, Some(StageBlend::ALPHA));
                    if let Some((palette, resource)) = palette {
                        let [r, g, b, _] = pixel.to_le_bytes();
                        let color = resource.nearest_index([r, g, b], None);
                        buffers.write_indexed(index, color, resource, palette.0, None);
                    } else {
                        buffers.write_rgba(index, pixel, None);
                    }
                }
            }
        }
    }
}

fn draw_vertex(draw: Draw2d, at: [f32; 2]) -> Vertex {
    let fraction = std::array::from_fn::<_, 2, _>(|axis| {
        (at[axis] + 0.5 - draw.rect[axis]) / draw.rect[axis + 2]
    });
    Vertex {
        position: Vec3([at[0], at[1], 0.0]),
        texcoord: std::array::from_fn(|axis| {
            qa_core::math::lerp(
                draw.texcoords[axis],
                draw.texcoords[axis + 2],
                fraction[axis],
            )
        }),
        color: draw.color,
        ..Vertex::default()
    }
}

fn view_point(origin: Vec3, axes: [Vec3; 3], point: Vec3) -> Vec3 {
    let local = qa_core::math::inverse_transform_point(origin, axes, point);
    Vec3([-local.0[1], local.0[2], local.0[0]])
}

fn clip_plane(
    camera: &Camera,
    input: &[ClipVertex],
    output: &mut [ClipVertex; CLIP_VERTICES],
    plane: usize,
) -> Option<usize> {
    clip_polygon_plane(
        input,
        output,
        |_, vertex| camera.distance(vertex, plane),
        |previous, current, fraction| {
            let vertex = previous.lerp(current, fraction);
            vertex.finite().then_some(vertex)
        },
    )
}

fn culled(cull: Cull, area: f32) -> bool {
    area == 0.0
        || match cull {
            Cull::Front => area < 0.0,
            Cull::Back => area > 0.0,
            Cull::None => false,
        }
}

fn top_left(a: [f32; 2], b: [f32; 2]) -> bool {
    b[1] < a[1] || (b[1] == a[1] && b[0] > a[0])
}

fn pixel_bounds(coordinates: [f32; 3], low: u32, high: u32) -> (u32, u32) {
    let minimum = coordinates[0].min(coordinates[1]).min(coordinates[2]);
    let maximum = coordinates[0].max(coordinates[1]).max(coordinates[2]);
    (
        ((minimum - 0.5).ceil() as u32).clamp(low, high),
        ((maximum - 0.5).ceil() as u32).clamp(low, high),
    )
}

fn texel(coordinate: f32, size: u32, wrap: Wrap) -> usize {
    let coordinate = match wrap {
        Wrap::Repeat => coordinate.rem_euclid(1.0),
        Wrap::Clamp => coordinate.clamp(0.0, 1.0),
    };
    ((coordinate * size as f32) as usize).min(size as usize - 1)
}
/// Borrow one owned native upload level. Indexed resources remain separate.
#[derive(Clone, Copy)]
struct TexelView<'a> {
    width: u32,
    height: u32,
    rgba: &'a [u8],
    bounds: [u32; 4],
    row_stride: u32,
    storage_origin: [u32; 2],
    intensity: f32,
}
impl<'a> TexelView<'a> {
    fn image(image: &'a Image, mip: u8, intensity: TextureIntensity) -> Self {
        if let Some(prepared) = &image.prepared
            && let Some(level) = prepared
                .levels
                .get(mip as usize)
                .or(prepared.levels.first())
        {
            return Self {
                width: level.width,
                height: level.height,
                rgba: &level.rgba,
                bounds: [0, 0, level.width, level.height],
                row_stride: level.width,
                storage_origin: [0, 0],
                intensity: if intensity == TextureIntensity::NeutralizeUpload {
                    prepared.inverse_intensity
                } else {
                    1.0
                },
            };
        }
        Self {
            width: image.width,
            height: image.height,
            rgba: &image.rgba,
            bounds: [0, 0, image.width, image.height],
            row_stride: image.width,
            storage_origin: [0, 0],
            intensity: 1.0,
        }
    }
    #[cfg(test)]
    fn cache(width: u32, height: u32, rgba: &'a [[u8; 4]]) -> Self {
        Self {
            width,
            height,
            rgba: rgba.as_flattened(),
            bounds: [0, 0, width, height],
            row_stride: width,
            storage_origin: [0, 0],
            intensity: 1.0,
        }
    }
    /// Borrow a tightly packed copy of the current bounds, retaining native UVs.
    fn copied<'b>(self, rgba: &'b [[u8; 4]]) -> Option<TexelView<'b>> {
        if !self.owns(self.bounds)
            || (self.bounds[2] as usize).checked_mul(self.bounds[3] as usize)? != rgba.len()
        {
            return None;
        }
        Some(TexelView {
            width: self.width,
            height: self.height,
            rgba: rgba.as_flattened(),
            bounds: self.bounds,
            row_stride: self.bounds[2],
            storage_origin: [self.bounds[0], self.bounds[1]],
            intensity: self.intensity,
        })
    }
    #[expect(
        clippy::manual_is_multiple_of,
        reason = "Keep the explicit zero-row check and storage-boundary remainder test"
    )]
    fn owns(self, bounds: [u32; 4]) -> bool {
        let [x, y, width, height] = bounds;
        if width == 0
            || height == 0
            || x.checked_add(width).is_none_or(|end| end > self.width)
            || y.checked_add(height).is_none_or(|end| end > self.height)
        {
            return false;
        }
        let Some(row_bytes) = (self.row_stride as usize).checked_mul(4) else {
            return false;
        };
        if row_bytes == 0 || self.rgba.len() % row_bytes != 0 {
            return false;
        }
        let rows = self.rgba.len() / row_bytes;
        let Some(local_x) = x.checked_sub(self.storage_origin[0]) else {
            return false;
        };
        let Some(local_y) = y.checked_sub(self.storage_origin[1]) else {
            return false;
        };
        local_x
            .checked_add(width)
            .is_some_and(|end| end <= self.row_stride)
            && (local_y as usize)
                .checked_add(height as usize)
                .is_some_and(|end| end <= rows)
    }
    fn region(mut self, region: Option<crate::lightmap::AtlasRegion>) -> Option<Self> {
        if let Some(region) = region {
            self.bounds = [region.x, region.y, region.width, region.height];
        }
        self.owns(self.bounds).then_some(self)
    }
    fn offset(self, x: usize, y: usize) -> usize {
        ((y - self.storage_origin[1] as usize) * self.row_stride as usize + x
            - self.storage_origin[0] as usize)
            * 4
    }
}

fn effective_sampler(image: &Image, sampler: Sampler) -> Sampler {
    image.native_sampler.unwrap_or(sampler)
}
fn image_mips(image: &Image, sampler: Sampler) -> u8 {
    if effective_sampler(image, sampler).mipmaps {
        image
            .prepared
            .as_ref()
            .map_or(1, |prepared| prepared.levels.len().clamp(1, 32) as u8)
    } else {
        1
    }
}
fn image_mip(image: &Image, sampler: Sampler, derivatives: [[f32; 2]; 2]) -> u8 {
    let count = image_mips(image, sampler);
    if count == 1 {
        return 0;
    }
    let base = TexelView::image(image, 0, TextureIntensity::Preserve);
    let rho = derivatives
        .map(|d| ((d[0] * base.width as f32).powi(2) + (d[1] * base.height as f32).powi(2)).sqrt())
        .into_iter()
        .fold(0.0_f32, f32::max);
    if rho > 1.0 && rho.is_finite() {
        ((rho.log2() + 0.5).floor() as u8).min(count - 1)
    } else {
        0
    }
}
fn uv_derivatives(
    inverse_depth: world::Affine,
    texture: [world::Affine; 2],
    x: f32,
    y: f32,
) -> [[f32; 2]; 2] {
    let zi = inverse_depth.at(x, y);
    let depth = [inverse_depth.x, inverse_depth.y];
    std::array::from_fn(|axis| {
        std::array::from_fn(|coordinate| {
            let p = texture[coordinate];
            let gradient = [p.x, p.y];
            (gradient[axis] * zi - p.at(x, y) * depth[axis]) / (zi * zi)
        })
    })
}
fn triangle_mip(
    image: &Image,
    sampler: Sampler,
    planes: Option<(world::Affine, [world::Affine; 2])>,
    at: [f32; 2],
) -> u8 {
    let Some((depth, texture)) = planes else {
        return 0;
    };
    image_mip(image, sampler, uv_derivatives(depth, texture, at[0], at[1]))
}

type ShadeFn = fn(TexelView<'_>, [f32; 2], [f32; 4]) -> [f32; 4];
fn stage_sampler(image: &Image, sampler: Sampler) -> ShadeFn {
    let sampler = effective_sampler(image, sampler);
    if sampler.filter == Filter::Linear
        && sampler.wrap == Wrap::Repeat
        && image_repeat_mask(image, sampler.mipmaps)
    {
        sample::<true, true, true>
    } else {
        sampler_function(sampler)
    }
}
fn image_repeat_mask(image: &Image, mipmaps: bool) -> bool {
    if let Some(prepared) = &image.prepared
        && !prepared.levels.is_empty()
    {
        prepared
            .levels
            .iter()
            .take(if mipmaps { 32 } else { 1 })
            .all(|level| level.width.is_power_of_two() && level.height.is_power_of_two())
    } else {
        image.width.is_power_of_two() && image.height.is_power_of_two()
    }
}
fn sampler_function(sampler: Sampler) -> ShadeFn {
    match (sampler.filter, sampler.wrap) {
        (Filter::Nearest, Wrap::Repeat) => sample::<false, true, false>,
        (Filter::Nearest, Wrap::Clamp) => sample::<false, false, false>,
        (Filter::Linear, Wrap::Repeat) => sample::<true, true, false>,
        (Filter::Linear, Wrap::Clamp) => sample::<true, false, false>,
    }
}
fn linear_taps<const REPEAT: bool, const MASK_REPEAT: bool>(
    base: f32,
    size: u32,
    low: u32,
    count: u32,
) -> [usize; 2] {
    let taps = if REPEAT && (-16_777_216.0..=16_777_215.0).contains(&base) {
        // Both consecutive integer taps are exactly representable in this range.
        if MASK_REPEAT {
            let mask = size - 1;
            let first = (base as i32 as u32) & mask;
            [first as usize, ((first + 1) & mask) as usize]
        } else {
            let first = (base as i32).rem_euclid(size as i32) as usize;
            [
                first,
                if first + 1 == size as usize {
                    0
                } else {
                    first + 1
                },
            ]
        }
    } else {
        // Large floats can round base+1 back to base; keep the original operations.
        [base + 0.0, base + 1.0].map(|value| {
            if REPEAT {
                value.rem_euclid(size as f32) as usize
            } else {
                value.clamp(0.0, size as f32 - 1.0) as usize
            }
        })
    };
    taps.map(|tap| tap.clamp(low as usize, (low + count - 1) as usize))
}
fn sample<const LINEAR: bool, const REPEAT: bool, const MASK_REPEAT: bool>(
    image: TexelView<'_>,
    coordinates: [f32; 2],
    color: [f32; 4],
) -> [f32; 4] {
    let texture: [f32; 4] = if LINEAR {
        let size = [image.width, image.height];
        let p = std::array::from_fn::<_, 2, _>(|i| coordinates[i] * size[i] as f32 - 0.5);
        let base = p.map(f32::floor);
        let fraction = [p[0] - base[0], p[1] - base[1]];
        let [x0, x1] =
            linear_taps::<REPEAT, MASK_REPEAT>(base[0], size[0], image.bounds[0], image.bounds[2]);
        let [y0, y1] =
            linear_taps::<REPEAT, MASK_REPEAT>(base[1], size[1], image.bounds[1], image.bounds[3]);
        let offsets = [
            image.offset(x0, y0),
            image.offset(x1, y0),
            image.offset(x0, y1),
            image.offset(x1, y1),
        ];
        let samples = offsets
            .map(|offset| crate::stage::unit_rgba(std::array::from_fn(|i| image.rgba[offset + i])));
        qa_core::math::lerp_array(
            qa_core::math::lerp_array(samples[0], samples[1], fraction[0]),
            qa_core::math::lerp_array(samples[2], samples[3], fraction[0]),
            fraction[1],
        )
    } else {
        let wrap = if REPEAT { Wrap::Repeat } else { Wrap::Clamp };
        let x = texel(coordinates[0], image.width, wrap).clamp(
            image.bounds[0] as usize,
            (image.bounds[0] + image.bounds[2] - 1) as usize,
        );
        let y = texel(coordinates[1], image.height, wrap).clamp(
            image.bounds[1] as usize,
            (image.bounds[1] + image.bounds[3] - 1) as usize,
        );
        let offset = image.offset(x, y);
        crate::stage::unit_rgba(std::array::from_fn(|i| image.rgba[offset + i]))
    };
    std::array::from_fn(|i| texture[i] * color[i] * if i < 3 { image.intensity } else { 1.0 })
}

fn transformed_palette(
    resource: &crate::surface_cache::PaletteLighting,
    transform: &crate::scene::PaletteTransform,
    blend: [f32; 4],
) -> [u32; 256] {
    std::array::from_fn(|index| {
        let base = resource.color(index as u8).to_le_bytes();
        let rgb: [u8; 3] = std::array::from_fn(|axis| {
            let value = match transform.operation {
                PaletteOperation::SequentialShifts => {
                    let mut value = base[axis] as i64;
                    for shift in transform.shifts {
                        value +=
                            (shift.percent as i64 * (shift.destination[axis] as i64 - value)) >> 8;
                    }
                    value.clamp(0, 255) as usize
                }
                PaletteOperation::ScreenBlend => {
                    let alpha = blend[3].clamp(0.0, 1.0);
                    ((base[axis] as f32 * (1.0 - alpha)
                        + blend[axis].clamp(0.0, 1.0) * alpha * 255.0)
                        as usize)
                        .min(255)
                }
            };
            transform.gamma[value]
        });
        u32::from_le_bytes([rgb[0], rgb[1], rgb[2], base[3]])
    })
}

fn depth_passes(
    test: DepthFunc,
    incoming: f32,
    existing: f32,
    incoming_rank: u32,
    existing_rank: u32,
) -> bool {
    match test {
        DepthFunc::Lequal => {
            incoming > existing || (incoming == existing && incoming_rank >= existing_rank)
        }
        DepthFunc::Equal => incoming == existing && incoming_rank >= existing_rank,
        DepthFunc::Always => true,
    }
}

fn composite(destination: u32, source: [f32; 4], blend: Option<StageBlend>) -> u32 {
    let destination = crate::stage::unit_rgba(destination.to_le_bytes());
    let result = blend_pixel(blend, source, destination);
    crate::stage::pack_unit_rgba(result)
}

#[cfg(test)]
#[path = "../tests/cpu_sampling/oracle.rs"]
mod sampling_tests;

#[cfg(test)]
#[path = "../tests/camera/oracle.rs"]
mod camera_tests;
