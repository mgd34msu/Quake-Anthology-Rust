//! Shared entity/poly raster and ordered 2D command consumer.
//!
//! Worlds use the shared edge/span scanner and native indexed surface cache.
//! Entity/poly meshes use the shared stage evaluator and a 1/Z buffer.
use crate::BackendStats;
use crate::assets::{Assets, DepthFunc, Filter, Image, Material, Sampler, Vertex, Wrap};
use crate::shader::{BlendFactor, Cull, StageBlend};
use crate::stage::{DrawInputs, PreparedStage, StageEvaluator, alpha_pass, blend_pixel};
mod sky;
mod world;
use crate::scene::{
    BlendPhase, Command, CommandList, CpuPresentation, Draw2d, DrawKind, PaletteOperation, Poly,
    Refdef, SceneEntity, Viewport,
};
use qa_core::primitives::Vec3;
pub use world::{CpuLimits, WorldStats};

const CLIP_VERTICES: usize = 12;

pub struct CpuBackend {
    width: u32,
    height: u32,
    pixels: Box<[u32]>,
    inverse_depth: Box<[f32]>,
    depth_ranks: Box<[u32]>,
    indices: Box<[u8]>,
    palettes: Box<[u32]>,
    world: Option<world::WorldRaster>,
    evaluator: StageEvaluator,
    presentation: CpuPresentation,
    time_ms: u64,
}

#[derive(Clone, Copy, Default)]
struct ClipVertex {
    camera: [f32; 3],
    texcoord: [f32; 2],
    lightmap_coord: [f32; 2],
    color: [f32; 4],
}

impl ClipVertex {
    fn finite(self) -> bool {
        self.camera
            .iter()
            .chain(self.texcoord.iter())
            .chain(self.lightmap_coord.iter())
            .chain(self.color.iter())
            .all(|f| f.is_finite())
    }

    fn lerp(self, end: Self, fraction: f32) -> Self {
        Self {
            camera: std::array::from_fn(|i| {
                self.camera[i] + fraction * (end.camera[i] - self.camera[i])
            }),
            texcoord: std::array::from_fn(|i| {
                self.texcoord[i] + fraction * (end.texcoord[i] - self.texcoord[i])
            }),
            lightmap_coord: std::array::from_fn(|i| {
                self.lightmap_coord[i] + fraction * (end.lightmap_coord[i] - self.lightmap_coord[i])
            }),
            color: std::array::from_fn(|i| {
                self.color[i] + fraction * (end.color[i] - self.color[i])
            }),
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
}

impl ScreenVertex {
    fn finite(self) -> bool {
        self.xy
            .iter()
            .chain(self.texcoord_over_depth.iter())
            .chain(self.lightmap_over_depth.iter())
            .chain(self.color_over_depth.iter())
            .all(|v| v.is_finite())
            && self.inverse_depth.is_finite()
    }
}

#[derive(Clone, Copy)]
struct Camera {
    refdef: Refdef,
    tangent: [f32; 2],
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
}

impl Camera {
    fn load(refdef: Refdef, width: u32, height: u32) -> Option<Self> {
        if !valid_viewport(refdef.viewport, width, height)
            || (refdef.blend_phase == BlendPhase::FinalPalette
                && refdef
                    .blend_viewport
                    .is_some_and(|viewport| !valid_viewport(viewport, width, height)))
            || !refdef
                .origin
                .0
                .iter()
                .chain(refdef.axes.iter().flat_map(|axis| axis.0.iter()))
                .chain(refdef.fov.iter())
                .chain(refdef.blend.iter())
                .all(|f| f.is_finite())
            || !refdef.identity_light.is_finite()
            || refdef.palette_transform.is_some_and(|transform| {
                transform.shifts.iter().any(|shift| {
                    !(0..=256).contains(&shift.percent)
                        || shift
                            .destination
                            .iter()
                            .any(|value| !(0..=255).contains(value))
                })
            })
            || !refdef.near.is_finite()
            || !refdef.far.is_finite()
            || refdef.near <= 0.0
            || !(1.0 / refdef.near).is_finite()
            || refdef.far <= refdef.near
            || refdef.fov.iter().any(|&f| f <= 0.0 || f >= 179.0)
        {
            return None;
        }
        let tangent = refdef.fov.map(|f| (f.to_radians() * 0.5).tan());
        if tangent.iter().any(|&t| t <= 0.0 || !t.is_finite()) {
            return None;
        }
        Some(Self { refdef, tangent })
    }

    fn vertex(self, vertex: Vertex, world_position: Vec3) -> ClipVertex {
        let delta = Vec3(std::array::from_fn(|i| {
            world_position.0[i] - self.refdef.origin.0[i]
        }));
        ClipVertex {
            camera: [
                -delta.dot(self.refdef.axes[1]),
                delta.dot(self.refdef.axes[2]),
                delta.dot(self.refdef.axes[0]),
            ],
            texcoord: vertex.texcoord,
            lightmap_coord: vertex.lightmap_coord,
            color: vertex.color.map(|c| c as f32 / 255.0),
        }
    }

    fn project(self, vertex: ClipVertex) -> ScreenVertex {
        let inverse_depth = 1.0 / vertex.camera[2];
        let viewport = self.refdef.viewport;
        ScreenVertex {
            xy: [
                viewport.x as f32
                    + viewport.width as f32
                        * 0.5
                        * (1.0 + vertex.camera[0] * inverse_depth / self.tangent[0]),
                viewport.y as f32
                    + viewport.height as f32
                        * 0.5
                        * (1.0 - vertex.camera[1] * inverse_depth / self.tangent[1]),
            ],
            inverse_depth,
            texcoord_over_depth: vertex.texcoord.map(|c| c * inverse_depth),
            lightmap_over_depth: vertex.lightmap_coord.map(|c| c * inverse_depth),
            color_over_depth: vertex.color.map(|c| c * inverse_depth),
        }
    }

    fn distance(self, vertex: ClipVertex, plane: usize) -> f32 {
        let [x, y, z] = vertex.camera;
        match plane {
            0 => z - self.refdef.near,
            1 => self.refdef.far - z,
            2 => x + z * self.tangent[0],
            3 => z * self.tangent[0] - x,
            4 => y + z * self.tangent[1],
            _ => z * self.tangent[1] - y,
        }
    }
}

impl CpuBackend {
    pub fn load(width: u32, height: u32) -> Result<Self, &'static str> {
        if width == 0 || height == 0 || width > 8192 || height > 8192 {
            return Err("invalid CPU framebuffer dimensions");
        }
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
            world: None,
            evaluator: StageEvaluator::load(),
            presentation: CpuPresentation::Rgb,
            time_ms: 0,
        })
    }

    /// Assets are frozen before this load-sized world scratch is allocated.
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
        backend.world = Some(world::WorldRaster::load(width, height, assets, limits)?);
        Ok(backend)
    }
    pub fn world_stats(&self) -> WorldStats {
        self.world
            .as_ref()
            .map_or(WorldStats::default(), |w| w.stats())
    }

    pub fn pixels(&self) -> &[u32] {
        &self.pixels
    }

    pub fn dimensions(&self) -> [u32; 2] {
        [self.width, self.height]
    }

    pub fn render(&mut self, list: &CommandList, assets: &Assets) -> BackendStats {
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
                    self.pixels.fill(u32::from_le_bytes(color));
                    self.inverse_depth.fill(0.0);
                    self.depth_ranks.fill(0);
                    self.palettes.fill(u32::MAX);
                    self.presentation = CpuPresentation::Rgb;
                }
                Command::View(view) => {
                    let Some(camera) = Camera::load(view.refdef, self.width, self.height) else {
                        stats.rejected = stats.rejected.saturating_add(1);
                        continue;
                    };
                    stats.views = stats.views.saturating_add(1);
                    stats.pending_lights =
                        stats.pending_lights.saturating_add(view.scene.lights.count);
                    self.presentation = camera.refdef.cpu_presentation;
                    self.time_ms = camera.refdef.time_ms;
                    self.clear_depth(camera.refdef.viewport, camera.refdef.far);
                    if let Some(world) = &mut self.world {
                        world.render_opaque(
                            camera,
                            list,
                            view.scene,
                            assets,
                            &self.evaluator,
                            world::Buffers {
                                pixels: &mut self.pixels,
                                inverse_depth: &mut self.inverse_depth,
                                depth_ranks: &mut self.depth_ranks,
                                indices: &mut self.indices,
                                palettes: &mut self.palettes,
                            },
                            &mut stats,
                        );
                    } else {
                        stats.rejected = stats.rejected.saturating_add(view.scene.surfaces.count);
                    }
                    for (draw_rank, item) in list.draws(view.scene.draws).iter().enumerate() {
                        if let Some(world) = &mut self.world
                            && world.draw_sky_item(
                                camera,
                                *item,
                                draw_rank as u32,
                                list,
                                assets,
                                &self.evaluator,
                                world::Buffers {
                                    pixels: &mut self.pixels,
                                    inverse_depth: &mut self.inverse_depth,
                                    depth_ranks: &mut self.depth_ranks,
                                    indices: &mut self.indices,
                                    palettes: &mut self.palettes,
                                },
                                &mut stats,
                            )
                        {
                            continue;
                        }
                        match item.kind {
                            DrawKind::Entity => self.entity(
                                camera,
                                list.entity(item.index),
                                draw_rank as u32,
                                assets,
                                &mut stats,
                            ),
                            DrawKind::Poly => self.poly(
                                camera,
                                list.poly(item.index),
                                draw_rank as u32,
                                list,
                                assets,
                                &mut stats,
                            ),
                            DrawKind::Surface => {
                                if let Some(world) = &mut self.world {
                                    world.draw_surface(
                                        camera,
                                        item.index,
                                        assets,
                                        &self.evaluator,
                                        world::Buffers {
                                            pixels: &mut self.pixels,
                                            inverse_depth: &mut self.inverse_depth,
                                            depth_ranks: &mut self.depth_ranks,
                                            indices: &mut self.indices,
                                            palettes: &mut self.palettes,
                                        },
                                        &mut stats,
                                    );
                                }
                            }
                        }
                    }
                    if camera.refdef.blend_phase == BlendPhase::AfterView {
                        self.view_blend(camera.refdef, false, assets);
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
                && let Some(camera) = Camera::load(view.refdef, self.width, self.height)
            {
                self.view_blend(camera.refdef, true, assets);
            }
        }
        stats
    }

    fn clear_depth(&mut self, viewport: Viewport, far: f32) {
        // The GL clear value and depthRange(1,1) both represent the view's far
        // plane. Keep that equality in the shared inverse-depth surface.
        let far_depth = 1.0 / far;
        for y in viewport.y..viewport.y + viewport.height {
            let start = y as usize * self.width as usize + viewport.x as usize;
            self.inverse_depth[start..start + viewport.width as usize].fill(far_depth);
            self.depth_ranks[start..start + viewport.width as usize].fill(0);
        }
    }

    fn poly(
        &mut self,
        camera: Camera,
        poly: &Poly,
        draw_rank: u32,
        list: &CommandList,
        assets: &Assets,
        stats: &mut BackendStats,
    ) {
        let Some(material) = assets.material(poly.material) else {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        };
        if !self.mesh_supported(camera, material) {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        }
        let vertices = list.vertices(poly.vertices);
        let inputs = DrawInputs {
            time_ms: camera.refdef.time_ms,
            view_origin: camera.refdef.origin,
            identity_light: camera.refdef.identity_light,
            ..DrawInputs::default()
        };
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
            let pass = RasterPass {
                material,
                stage,
                image,
                sampler: stage_sampler(image, stage.stage.sampler),
                depth_hack: false,
                first_stage: stage_index == 0,
                draw_rank,
            };
            stats.stages = stats.stages.saturating_add(1);
            for i in 1..vertices.len().saturating_sub(1) {
                let triangle = [vertices[0], vertices[i], vertices[i + 1]].map(|v| {
                    let v = self.evaluator.apply_deforms(deforms, v);
                    let evaluated = self.evaluator.evaluate(&stage, &v);
                    camera.vertex(
                        Vertex {
                            texcoord: evaluated.texcoord,
                            color: evaluated.color,
                            ..v
                        },
                        evaluated.position,
                    )
                });
                self.triangle(camera, triangle, pass, stats);
            }
        }
    }

    fn entity(
        &mut self,
        camera: Camera,
        entity: &SceneEntity,
        draw_rank: u32,
        assets: &Assets,
        stats: &mut BackendStats,
    ) {
        let Some(model) = assets.model(entity.model) else {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        };
        let Some(material) = assets.material(entity.material.unwrap_or(model.material)) else {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        };
        if !entity
            .origin
            .0
            .iter()
            .chain(entity.axes.iter().flat_map(|axis| axis.0.iter()))
            .all(|f| f.is_finite())
        {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        }
        if !self.mesh_supported(camera, material) {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        }
        let local_view = crate::stage::entity_view_origin(camera.refdef.origin, entity);
        let inputs = DrawInputs {
            time_ms: camera.refdef.time_ms,
            entity_color: entity.color,
            entity_texcoord: entity.shader_texcoord,
            entity_shader_time: entity.shader_time,
            lighting: entity.lighting,
            view_origin: local_view,
            identity_light: camera.refdef.identity_light,
            ..DrawInputs::default()
        };
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
            let pass = RasterPass {
                material,
                stage,
                image,
                sampler: stage_sampler(image, stage.stage.sampler),
                depth_hack: entity.depth_hack,
                first_stage: stage_index == 0,
                draw_rank,
            };
            stats.stages = stats.stages.saturating_add(1);
            for indices in model.indices.chunks_exact(3) {
                let triangle = std::array::from_fn(|i| {
                    let vertex = self
                        .evaluator
                        .apply_deforms(deforms, model.vertices[indices[i] as usize]);
                    let evaluated = self.evaluator.evaluate(&stage, &vertex);
                    let position = Vec3(std::array::from_fn(|axis| {
                        entity.origin.0[axis]
                            + evaluated.position.0[0] * entity.axes[0].0[axis]
                            + evaluated.position.0[1] * entity.axes[1].0[axis]
                            + evaluated.position.0[2] * entity.axes[2].0[axis]
                    }));
                    camera.vertex(
                        Vertex {
                            texcoord: evaluated.texcoord,
                            color: evaluated.color,
                            ..vertex
                        },
                        position,
                    )
                });
                self.triangle(camera, triangle, pass, stats);
            }
        }
    }

    fn mesh_supported(&self, camera: Camera, material: &Material) -> bool {
        // Native indexed model/light shading is a separate native kernel; an
        // RGB model must not masquerade as stock indexed presentation.
        matches!(camera.refdef.cpu_presentation, CpuPresentation::Rgb)
            && material.settings.sky.is_none()
            && material.settings.fog.is_none()
            && !material.settings.portal
            && !material.settings.polygon_offset
    }

    fn triangle(
        &mut self,
        camera: Camera,
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
        for plane in 0..6 {
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
        for i in 1..count - 1 {
            self.raster_triangle(
                camera,
                [
                    first,
                    camera.project(vertices[i]),
                    camera.project(vertices[i + 1]),
                ],
                pass,
                stats,
            );
        }
    }

    fn raster_triangle(
        &mut self,
        camera: Camera,
        mut vertices: [ScreenVertex; 3],
        pass: RasterPass<'_>,
        stats: &mut BackendStats,
    ) {
        if vertices.iter().any(|v| {
            !v.xy
                .iter()
                .chain(v.texcoord_over_depth.iter())
                .chain(v.lightmap_over_depth.iter())
                .chain(v.color_over_depth.iter())
                .all(|f| f.is_finite())
                || !v.inverse_depth.is_finite()
        }) {
            stats.rejected = stats.rejected.saturating_add(1);
            return;
        }
        let mut area = edge(vertices[0].xy, vertices[1].xy, vertices[2].xy);
        // Q3 CT_FRONT_SIDED culls GL_FRONT with GL_CCW. The retained winding
        // therefore has positive area in this top-left-origin framebuffer.
        if area == 0.0
            || (match pass.material.settings.cull {
                Cull::Front => area < 0.0,
                Cull::Back => area > 0.0,
                Cull::None => false,
            })
        {
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
        for y in y_start..y_end {
            for x in x_start..x_end {
                let sample = [x as f32 + 0.5, y as f32 + 0.5];
                let weights = edges.map(|(a, b)| edge(a, b, sample));
                if weights
                    .iter()
                    .zip(include_edge)
                    .any(|(&weight, inclusive)| weight < 0.0 || (weight == 0.0 && !inclusive))
                {
                    continue;
                }
                let weights = weights.map(|w| w * area_inverse);
                let inverse_depth = weights[0] * vertices[0].inverse_depth
                    + weights[1] * vertices[1].inverse_depth
                    + weights[2] * vertices[2].inverse_depth;
                let depth_score = if pass.depth_hack {
                    inverse_depth * 0.3 + 0.7 / camera.refdef.near
                } else {
                    inverse_depth
                };
                let index = y as usize * self.width as usize + x as usize;
                if !depth_passes(
                    pass.stage.stage.depth_func,
                    depth_score,
                    self.inverse_depth[index],
                    pass.draw_rank,
                    self.depth_ranks[index],
                ) {
                    continue;
                }
                let depth = 1.0 / inverse_depth;
                let coordinates = std::array::from_fn(|i| {
                    (weights[0] * vertices[0].texcoord_over_depth[i]
                        + weights[1] * vertices[1].texcoord_over_depth[i]
                        + weights[2] * vertices[2].texcoord_over_depth[i])
                        * depth
                });
                let color = std::array::from_fn(|i| {
                    (weights[0] * vertices[0].color_over_depth[i]
                        + weights[1] * vertices[1].color_over_depth[i]
                        + weights[2] * vertices[2].color_over_depth[i])
                        * depth
                });
                let source = (pass.sampler)(pass.image, coordinates, color);
                if !alpha_pass(pass.stage.stage.alpha_test, source[3]) {
                    continue;
                }
                self.pixels[index] = composite(self.pixels[index], source, pass.stage.stage.blend);
                self.palettes[index] = u32::MAX;
                if pass.stage.stage.depth_write {
                    self.inverse_depth[index] = depth_score;
                    self.depth_ranks[index] = pass.draw_rank;
                }
            }
        }
    }

    fn draw_2d(&mut self, draw: Draw2d, assets: &Assets, stats: &mut BackendStats) {
        let Some(material) = assets.material(draw.material) else {
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
            self.width,
        );
        let (y_start, y_end) = pixel_bounds(
            [draw.rect[1], draw.rect[1] + draw.rect[3], draw.rect[1]],
            0,
            self.height,
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
                    if stage.blend.is_some() || draw.color != [255; 4] {
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
            for y in y_start..y_end {
                let v = (y as f32 + 0.5 - draw.rect[1]) / draw.rect[3];
                for x in x_start..x_end {
                    let u = (x as f32 + 0.5 - draw.rect[0]) / draw.rect[2];
                    let vertex = Vertex {
                        position: Vec3([x as f32, y as f32, 0.0]),
                        texcoord: [
                            draw.texcoords[0] + u * (draw.texcoords[2] - draw.texcoords[0]),
                            draw.texcoords[1] + v * (draw.texcoords[3] - draw.texcoords[1]),
                        ],
                        color: draw.color,
                        ..Vertex::default()
                    };
                    let evaluated = self.evaluator.evaluate(&prepared, &vertex);
                    let index = y as usize * self.width as usize + x as usize;
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
                        self.pixels[index] = palette.color(color);
                        self.indices[index] = color;
                        self.palettes[index] = palette_id.0;
                    } else {
                        let source = sampler(
                            image,
                            evaluated.texcoord,
                            evaluated.color.map(|c| c as f32 / 255.0),
                        );
                        if alpha_pass(stage.alpha_test, source[3]) {
                            let blend = stage.blend.or(Some(ALPHA_BLEND));
                            self.pixels[index] = composite(self.pixels[index], source, blend);
                            self.palettes[index] = u32::MAX;
                        }
                    }
                }
            }
        }
    }

    fn view_blend(&mut self, refdef: Refdef, final_phase: bool, assets: &Assets) {
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
                for y in viewport.y..viewport.y + viewport.height {
                    let start = y as usize * self.width as usize + viewport.x as usize;
                    for index in start..start + viewport.width as usize {
                        if self.palettes[index] == palette.0 {
                            self.pixels[index] = transformed[self.indices[index] as usize];
                        }
                    }
                }
            }
        }
    }

    fn tint(&mut self, refdef: Refdef, preserve_alpha: bool) {
        let source = refdef.blend.map(|c| c.clamp(0.0, 1.0));
        if source[3] == 0.0 {
            return;
        }
        let viewport = if preserve_alpha {
            refdef.blend_viewport.unwrap_or(refdef.viewport)
        } else {
            refdef.viewport
        };
        for y in viewport.y..viewport.y + viewport.height {
            let start = y as usize * self.width as usize + viewport.x as usize;
            for pixel in &mut self.pixels[start..start + viewport.width as usize] {
                let alpha = *pixel & 0xff00_0000;
                *pixel = composite(*pixel, source, Some(ALPHA_BLEND));
                if preserve_alpha {
                    *pixel = (*pixel & 0x00ff_ffff) | alpha;
                }
            }
        }
    }
}

fn valid_viewport(viewport: Viewport, width: u32, height: u32) -> bool {
    viewport.width > 0
        && viewport.height > 0
        && viewport.x as u64 + viewport.width as u64 <= width as u64
        && viewport.y as u64 + viewport.height as u64 <= height as u64
}

fn clip_plane(
    camera: Camera,
    input: &[ClipVertex],
    output: &mut [ClipVertex; CLIP_VERTICES],
    plane: usize,
) -> Option<usize> {
    let mut count = 0;
    let mut previous = input[input.len() - 1];
    let mut previous_distance = camera.distance(previous, plane);
    for &current in input {
        let distance = camera.distance(current, plane);
        if !distance.is_finite() || !previous_distance.is_finite() {
            return None;
        }
        let inside = distance >= 0.0;
        let previous_inside = previous_distance >= 0.0;
        if inside != previous_inside {
            if count == output.len() {
                return None;
            }
            let denominator = previous_distance - distance;
            if !denominator.is_finite() {
                return None;
            }
            let fraction = previous_distance / denominator;
            let intersection = previous.lerp(current, fraction);
            if !intersection.finite() {
                return None;
            }
            output[count] = intersection;
            count += 1;
        }
        if inside {
            if count == output.len() {
                return None;
            }
            output[count] = current;
            count += 1;
        }
        previous = current;
        previous_distance = distance;
    }
    Some(count)
}

fn edge(a: [f32; 2], b: [f32; 2], point: [f32; 2]) -> f32 {
    (b[0] - a[0]) * (point[1] - a[1]) - (b[1] - a[1]) * (point[0] - a[0])
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

const ALPHA_BLEND: StageBlend = StageBlend {
    source: BlendFactor::SourceAlpha,
    destination: BlendFactor::OneMinusSourceAlpha,
};
fn texel(coordinate: f32, size: u32, wrap: Wrap) -> usize {
    let coordinate = match wrap {
        Wrap::Repeat => coordinate.rem_euclid(1.0),
        Wrap::Clamp => coordinate.clamp(0.0, 1.0),
    };
    ((coordinate * size as f32) as usize).min(size as usize - 1)
}
type ShadeFn = fn(&Image, [f32; 2], [f32; 4]) -> [f32; 4];
fn stage_sampler(image: &Image, sampler: Sampler) -> ShadeFn {
    let sampler = image.native_sampler.unwrap_or(sampler);
    match (sampler.filter, sampler.wrap) {
        (Filter::Nearest, Wrap::Repeat) => sample::<false, true>,
        (Filter::Nearest, Wrap::Clamp) => sample::<false, false>,
        (Filter::Linear, Wrap::Repeat) => sample::<true, true>,
        (Filter::Linear, Wrap::Clamp) => sample::<true, false>,
    }
}
fn sample<const LINEAR: bool, const REPEAT: bool>(
    image: &Image,
    coordinates: [f32; 2],
    color: [f32; 4],
) -> [f32; 4] {
    let texture: [f32; 4] = if LINEAR {
        let size = [image.width, image.height];
        let p = std::array::from_fn::<_, 2, _>(|i| coordinates[i] * size[i] as f32 - 0.5);
        let base = p.map(f32::floor);
        let fraction = [p[0] - base[0], p[1] - base[1]];
        let samples: [[f32; 4]; 4] = std::array::from_fn(|corner| {
            let xy = std::array::from_fn::<_, 2, _>(|i| {
                let value = base[i] + ((corner >> i) & 1) as f32;
                if REPEAT {
                    value.rem_euclid(size[i] as f32) as usize
                } else {
                    value.clamp(0.0, size[i] as f32 - 1.0) as usize
                }
            });
            let offset = (xy[1] * image.width as usize + xy[0]) * 4;
            std::array::from_fn(|i| image.rgba[offset + i] as f32 / 255.0)
        });
        std::array::from_fn(|i| {
            let a = samples[0][i] + fraction[0] * (samples[1][i] - samples[0][i]);
            let b = samples[2][i] + fraction[0] * (samples[3][i] - samples[2][i]);
            a + fraction[1] * (b - a)
        })
    } else {
        let wrap = if REPEAT { Wrap::Repeat } else { Wrap::Clamp };
        let x = texel(coordinates[0], image.width, wrap);
        let y = texel(coordinates[1], image.height, wrap);
        let offset = (y * image.width as usize + x) * 4;
        std::array::from_fn(|i| image.rgba[offset + i] as f32 / 255.0)
    };
    std::array::from_fn(|i| texture[i] * color[i])
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
    let destination = destination.to_le_bytes().map(|c| c as f32 / 255.0);
    let result = blend_pixel(blend, source, destination);
    u32::from_le_bytes(result.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8))
}
