//! Shared entity/poly raster and ordered 2D command consumer.
//!
//! THE-862 adds the native world edge/span cache and indexed palette path. The
//! final-palette blend phase here is an RGBA approximation, not stock Q1 proof.
use crate::BackendStats;
use crate::assets::{AlphaTest, Assets, Blend, DepthFunc, Image, Material, Stage, TcGen, Vertex};
use crate::scene::{BlendPhase, Command, CommandList, Draw2d, Refdef, SceneEntity, Viewport};
use qa_core::primitives::Vec3;

const CLIP_VERTICES: usize = 12;

pub struct CpuBackend {
    width: u32,
    height: u32,
    pixels: Box<[u32]>,
    inverse_depth: Box<[f32]>,
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

#[derive(Clone, Copy)]
struct ScreenVertex {
    xy: [f32; 2],
    inverse_depth: f32,
    texcoord_over_depth: [f32; 2],
    lightmap_over_depth: [f32; 2],
    color_over_depth: [f32; 4],
}

#[derive(Clone, Copy)]
struct Camera {
    refdef: Refdef,
    tangent: [f32; 2],
}

#[derive(Clone, Copy)]
struct RasterPass<'a> {
    material: &'a Material,
    stage: &'a Stage,
    image: &'a Image,
    color: [f32; 4],
    depth_hack: bool,
    first_stage: bool,
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
        })
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
        for command in list.commands() {
            match *command {
                Command::Empty => {}
                Command::Clear(color) => {
                    self.pixels.fill(u32::from_le_bytes(color));
                    self.inverse_depth.fill(0.0);
                }
                Command::View(view) => {
                    let Some(camera) = Camera::load(view.refdef, self.width, self.height) else {
                        stats.rejected = stats.rejected.saturating_add(1);
                        continue;
                    };
                    stats.views = stats.views.saturating_add(1);
                    stats.pending_lights =
                        stats.pending_lights.saturating_add(view.scene.lights.count);
                    self.clear_depth(camera.refdef.viewport);
                    for entity in list.entities(view.scene.entities) {
                        self.entity(camera, entity, assets, &mut stats);
                    }
                    for poly in list.polys(view.scene.polys) {
                        let Some(material) = assets.material(poly.material) else {
                            stats.rejected = stats.rejected.saturating_add(1);
                            continue;
                        };
                        let vertices = list.vertices(poly.vertices);
                        for (stage_index, stage) in material.stages.iter().enumerate() {
                            let Some(image) = assets.image(stage.image) else {
                                stats.rejected = stats.rejected.saturating_add(1);
                                continue;
                            };
                            let pass = RasterPass {
                                material,
                                stage,
                                image,
                                color: [1.0; 4],
                                depth_hack: false,
                                first_stage: stage_index == 0,
                            };
                            for i in 1..vertices.len().saturating_sub(1) {
                                let triangle = [vertices[0], vertices[i], vertices[i + 1]];
                                self.triangle(
                                    camera,
                                    triangle.map(|v| camera.vertex(v, v.position)),
                                    pass,
                                    &mut stats,
                                );
                            }
                        }
                    }
                    if camera.refdef.blend_phase == BlendPhase::AfterView {
                        self.tint(camera.refdef, false);
                    }
                }
                Command::Draw2d(draw) => {
                    self.draw_2d(draw, assets, &mut stats);
                }
            }
        }
        // Keep final-palette blends after HUD/console commands. The indexed
        // presentation in THE-862 replaces this RGB approximation.
        for command in list.commands() {
            if let Command::View(view) = *command
                && view.refdef.blend_phase == BlendPhase::FinalPalette
                && let Some(camera) = Camera::load(view.refdef, self.width, self.height)
            {
                self.tint(camera.refdef, true);
            }
        }
        stats
    }

    fn clear_depth(&mut self, viewport: Viewport) {
        for y in viewport.y..viewport.y + viewport.height {
            let start = y as usize * self.width as usize + viewport.x as usize;
            self.inverse_depth[start..start + viewport.width as usize].fill(0.0);
        }
    }

    fn entity(
        &mut self,
        camera: Camera,
        entity: &SceneEntity,
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
        let color = entity.color.map(|c| c as f32 / 255.0);
        for (stage_index, stage) in material.stages.iter().enumerate() {
            let Some(image) = assets.image(stage.image) else {
                stats.rejected = stats.rejected.saturating_add(1);
                continue;
            };
            let pass = RasterPass {
                material,
                stage,
                image,
                color,
                depth_hack: entity.depth_hack,
                first_stage: stage_index == 0,
            };
            for indices in model.indices.chunks_exact(3) {
                let triangle = std::array::from_fn(|i| {
                    let vertex = model.vertices[indices[i] as usize];
                    let position = Vec3(std::array::from_fn(|axis| {
                        entity.origin.0[axis]
                            + vertex.position.0[0] * entity.axes[0].0[axis]
                            + vertex.position.0[1] * entity.axes[1].0[axis]
                            + vertex.position.0[2] * entity.axes[2].0[axis]
                    }));
                    camera.vertex(vertex, position)
                });
                self.triangle(camera, triangle, pass, stats);
            }
        }
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
        if area == 0.0 || (!pass.material.two_sided && area < 0.0) {
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
                    pass.stage.depth_func,
                    depth_score,
                    self.inverse_depth[index],
                ) {
                    continue;
                }
                let depth = 1.0 / inverse_depth;
                let coordinates_over_depth = vertices.map(|v| match pass.stage.texgen {
                    TcGen::Texture => v.texcoord_over_depth,
                    TcGen::Lightmap => v.lightmap_over_depth,
                });
                let coordinates = std::array::from_fn(|i| {
                    (weights[0] * coordinates_over_depth[0][i]
                        + weights[1] * coordinates_over_depth[1][i]
                        + weights[2] * coordinates_over_depth[2][i])
                        * depth
                });
                let vertex_color = if pass.stage.vertex_color {
                    std::array::from_fn(|i| {
                        (weights[0] * vertices[0].color_over_depth[i]
                            + weights[1] * vertices[1].color_over_depth[i]
                            + weights[2] * vertices[2].color_over_depth[i])
                            * depth
                    })
                } else {
                    [1.0; 4]
                };
                let source = shade(
                    pass.image,
                    pass.stage,
                    coordinates,
                    pass.color,
                    vertex_color,
                );
                if !alpha_passes(pass.stage.alpha_test, source[3]) {
                    continue;
                }
                self.pixels[index] = composite(self.pixels[index], source, pass.stage.blend);
                if pass.stage.depth_write {
                    self.inverse_depth[index] = depth_score;
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
        let color = draw.color.map(|c| c as f32 / 255.0);
        for stage in material.stages.iter() {
            let Some(image) = assets.image(stage.image) else {
                stats.rejected = stats.rejected.saturating_add(1);
                continue;
            };
            for y in y_start..y_end {
                let v = (y as f32 + 0.5 - draw.rect[1]) / draw.rect[3];
                for x in x_start..x_end {
                    let u = (x as f32 + 0.5 - draw.rect[0]) / draw.rect[2];
                    let coordinates = [
                        draw.texcoords[0] + u * (draw.texcoords[2] - draw.texcoords[0]),
                        draw.texcoords[1] + v * (draw.texcoords[3] - draw.texcoords[1]),
                    ];
                    let index = y as usize * self.width as usize + x as usize;
                    let source = shade(image, stage, coordinates, color, [1.0; 4]);
                    if alpha_passes(stage.alpha_test, source[3]) {
                        let blend = if stage.blend == Blend::Opaque {
                            Blend::Alpha
                        } else {
                            stage.blend
                        };
                        self.pixels[index] = composite(self.pixels[index], source, blend);
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
                *pixel = composite(*pixel, source, Blend::Alpha);
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

fn shade(
    image: &Image,
    stage: &Stage,
    coordinates: [f32; 2],
    color: [f32; 4],
    vertex_color: [f32; 4],
) -> [f32; 4] {
    let x = ((coordinates[0].rem_euclid(1.0) * image.width as f32) as usize)
        .min(image.width as usize - 1);
    let y = ((coordinates[1].rem_euclid(1.0) * image.height as f32) as usize)
        .min(image.height as usize - 1);
    let offset = (y * image.width as usize + x) * 4;
    std::array::from_fn(|i| {
        image.rgba[offset + i] as f32 / 255.0
            * color[i]
            * if stage.vertex_color {
                vertex_color[i]
            } else {
                1.0
            }
    })
}

fn alpha_passes(test: AlphaTest, alpha: f32) -> bool {
    match test {
        AlphaTest::None => true,
        AlphaTest::GreaterZero => alpha > 0.0,
        AlphaTest::AtLeastHalf => alpha >= 0.5,
    }
}

fn depth_passes(test: DepthFunc, incoming: f32, existing: f32) -> bool {
    match test {
        DepthFunc::Lequal => incoming >= existing,
        DepthFunc::Equal => incoming == existing,
        DepthFunc::Always => true,
    }
}

fn composite(destination: u32, source: [f32; 4], blend: Blend) -> u32 {
    let destination = destination.to_le_bytes();
    let color = std::array::from_fn(|i| {
        let d = destination[i] as f32 / 255.0;
        let value = match blend {
            Blend::Opaque => source[i],
            Blend::Alpha => source[i] * source[3] + d * (1.0 - source[3]),
            Blend::Add => source[i] + d,
            Blend::Multiply => source[i] * d,
        };
        (value.clamp(0.0, 1.0) * 255.0).round() as u8
    });
    u32::from_le_bytes(color)
}
