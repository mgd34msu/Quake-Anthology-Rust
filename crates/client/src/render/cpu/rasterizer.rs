//! CPU software renderer.
//!
//! Donor provenance: `src/render/cpu/rasterizer.ts` in full — the Quake III
//! CPU backend following id Software `tr_backend.c` and `tr_shadows.c`.
//! Copyright (C) 1999-2005 Id Software, Inc. Clipping and rasterization
//! algorithms are from the TypeScript donor.
//!
//! The ordered command-stream contract is consumed synchronously through
//! [`OrderedBackend`]; scene submission for headless use goes through
//! [`CpuRenderer`] and [`RendererBackend`].

use std::sync::Arc;

use qa_core::identity::SessionId;
use qa_core::math::{vec2, vec4, Mat4, Vec2, Vec3, Vec4};

use super::super::types::{
    fresh_owner_identity, AlphaTest, BatchLighting, BatchPrimitive, BatchVertices, BlendFactor, CullFace, DepthTest,
    DrawBatch, DrawBuffer, FogEffect, ImageLevel, ImageResourceOperation, MultitextureVertex, OrderedBackend,
    PairEnvironment, PreparedDraw, Q2LightPass, Rect, RenderOperation, RenderState, RenderVertex, RenderViewState,
    RendererImage, ResourceOwner, RetainedDraw, TextureBinding, TextureRect, ViewClear,
};
use super::super::{FrameStats, RenderView, RendererBackend, SceneDecal, SceneEntity, SceneLight, SceneParticle};
use crate::materials::fog::Q1Fog;
use crate::render::scene::retained::{fill_retained_batch_vertices, resolve_retained_positions};
use crate::view::ViewProjector;

use super::super::stage_timings::StageTimer;
use super::fog::{apply_q1_depth_fog, apply_q2_depth_fog, Q1FogOutput};
use super::lighting::{shade_q2_fragment, world_attributes, CpuLighting, CpuTriangleLighting, CpuVertex};
use super::lines::{rasterize_aliased_line, LineFragment, LineScissor};
use super::textures::CpuImages;
use super::triangle_kernel::{
    blend, byte, clamp, passes_alpha, run_triangle_rows, sample_line_bound, stencil_fragment, texture_color,
    texture_has_alpha, BlendMode, BoundTexture, Framebuffer, Sample, StencilFunction, StencilOp, StencilTest,
    TexturePlaneDerivative, TriangleSetup,
};

/// Precision used for the polygon-offset resolvable-depth unit, not depth quantization.
pub const CPU_OFFSET_DEPTH_BITS: u32 = 24;

#[derive(Debug, Clone, Copy)]
struct ScreenVertex {
    x: f32,
    y: f32,
    z: f32,
    inverse_w: f32,
    fog_depth_scale: f32,
    world_position: Vec3,
    world_normal: Vec3,
    tex_coord: Vec2,
    tex_coord2: Vec2,
    r: f32,
    g: f32,
    b: f32,
    a: f32,
}

#[derive(Debug, Clone, Copy)]
enum ClipPlane {
    Left,
    Right,
    Bottom,
    Top,
    Near,
    Far,
    Custom(Vec4),
}

const CLIP_PLANES: [ClipPlane; 6] = [
    ClipPlane::Left,
    ClipPlane::Right,
    ClipPlane::Bottom,
    ClipPlane::Top,
    ClipPlane::Near,
    ClipPlane::Far,
];

fn finite_vector(value: &Vec4) -> bool {
    value.x.is_finite() && value.y.is_finite() && value.z.is_finite() && value.w.is_finite()
}

fn indexed_vertex(batch: &DrawBatch, offset: usize) -> CpuVertex {
    let index = *batch.indices.get(offset).expect("missing triangle index") as usize;
    let (world_position, world_normal) = world_attributes(&batch.lighting, index);
    match &batch.vertices {
        BatchVertices::Pair { vertices, .. } => {
            let vertex = vertices.get(index).expect("missing triangle vertex");
            CpuVertex {
                position: vertex.base.position,
                world_position,
                world_normal,
                color: vertex.base.color,
                tex_coord: vertex.base.tex_coord,
                tex_coord2: vertex.tex_coord2,
            }
        }
        BatchVertices::Single(vertices) => {
            let vertex = vertices.get(index).expect("missing triangle vertex");
            CpuVertex {
                position: vertex.position,
                world_position,
                world_normal,
                color: vertex.color,
                tex_coord: vertex.tex_coord,
                tex_coord2: vec2(0.0, 0.0),
            }
        }
    }
}

fn plane_distance(vertex: &CpuVertex, plane: &ClipPlane) -> f32 {
    let p = &vertex.position;
    match plane {
        ClipPlane::Custom(plane) => p.x * plane.x + p.y * plane.y + p.z * plane.z + p.w * plane.w,
        ClipPlane::Left => p.w + p.x,
        ClipPlane::Right => p.w - p.x,
        ClipPlane::Bottom => p.w + p.y,
        ClipPlane::Top => p.w - p.y,
        ClipPlane::Near => p.w + p.z,
        ClipPlane::Far => p.w - p.z,
    }
}

fn intersect(a: &CpuVertex, b: &CpuVertex, da: f32, db: f32, plane: &ClipPlane) -> CpuVertex {
    // Symmetric weights give the same intersection for a shared edge in either direction.
    let scale = da.abs().max(db.abs());
    let ad = da.abs() / scale;
    let bd = db.abs() / scale;
    let aw = bd / (ad + bd);
    let bw = ad / (ad + bd);
    let coordinate = |left: f32, right: f32| {
        let anchor = left.min(right);
        anchor + (left - anchor) * aw + (right - anchor) * bw
    };
    let w = a.position.w * aw + b.position.w * bw;
    let mut x = a.position.x * aw + b.position.x * bw;
    let mut y = a.position.y * aw + b.position.y * bw;
    let mut z = a.position.z * aw + b.position.z * bw;
    match plane {
        ClipPlane::Left => x = -w,
        ClipPlane::Right => x = w,
        ClipPlane::Bottom => y = -w,
        ClipPlane::Top => y = w,
        ClipPlane::Near => z = -w,
        ClipPlane::Far => z = w,
        ClipPlane::Custom(_) => {}
    }
    CpuVertex {
        position: vec4(x, y, z, w),
        world_position: Vec3 {
            x: a.world_position.x * aw + b.world_position.x * bw,
            y: a.world_position.y * aw + b.world_position.y * bw,
            z: a.world_position.z * aw + b.world_position.z * bw,
        },
        world_normal: Vec3 {
            x: a.world_normal.x * aw + b.world_normal.x * bw,
            y: a.world_normal.y * aw + b.world_normal.y * bw,
            z: a.world_normal.z * aw + b.world_normal.z * bw,
        },
        tex_coord: Vec2 {
            x: coordinate(a.tex_coord.x, b.tex_coord.x),
            y: coordinate(a.tex_coord.y, b.tex_coord.y),
        },
        tex_coord2: Vec2 {
            x: coordinate(a.tex_coord2.x, b.tex_coord2.x),
            y: coordinate(a.tex_coord2.y, b.tex_coord2.y),
        },
        color: vec4(
            a.color.x * aw + b.color.x * bw,
            a.color.y * aw + b.color.y * bw,
            a.color.z * aw + b.color.z * bw,
            a.color.w * aw + b.color.w * bw,
        ),
    }
}

fn clip_polygon(vertices: &[CpuVertex], extra_plane: Option<Vec4>) -> Vec<CpuVertex> {
    let mut polygon = vertices.to_vec();
    let mut magnitude = 0.0f32;
    for vertex in &polygon {
        let p = &vertex.position;
        magnitude = magnitude.max(p.x.abs()).max(p.y.abs()).max(p.z.abs()).max(p.w.abs());
    }
    if magnitude > f32::MAX / 4.0 {
        // Homogeneous coordinates admit a common scale. Bound the plane sums
        // without rejecting otherwise finite input or overflowing intersections.
        for vertex in &mut polygon {
            vertex.position.x /= magnitude;
            vertex.position.y /= magnitude;
            vertex.position.z /= magnitude;
            vertex.position.w /= magnitude;
        }
    }
    let mut planes: Vec<ClipPlane> = CLIP_PLANES.to_vec();
    if let Some(extra) = extra_plane {
        planes.push(ClipPlane::Custom(extra));
    }
    for plane in &planes {
        if polygon.is_empty() {
            return polygon;
        }
        if polygon.iter().all(|vertex| plane_distance(vertex, plane) >= 0.0) {
            continue;
        }
        let mut output = Vec::new();
        let mut previous = polygon[polygon.len() - 1];
        let mut previous_distance = plane_distance(&previous, plane);
        for current in &polygon {
            let distance = plane_distance(current, plane);
            if (distance >= 0.0) != (previous_distance >= 0.0) {
                output.push(intersect(&previous, current, previous_distance, distance, plane));
            }
            if distance >= 0.0 {
                output.push(*current);
            }
            previous = *current;
            previous_distance = distance;
        }
        polygon = output;
    }
    // All six planes permit w=0 only at the homogeneous origin. It has no
    // projected area; remove it before division instead of inventing an epsilon plane.
    polygon.into_iter().filter(|vertex| vertex.position.w > 0.0).collect()
}

fn subpixel(value: f32, scale: f32) -> f32 {
    let scaled = value * scale;
    let lower = scaled.floor();
    let fraction = scaled - lower;
    (if fraction < 0.5 || (fraction == 0.5 && lower % 2.0 == 0.0) {
        lower
    } else {
        lower + 1.0
    }) / scale
}

fn project(vertex: &CpuVertex, viewport: &Rect, w_scale: f32, subpixel_scale: f32, fog_scale: f32) -> ScreenVertex {
    let p = &vertex.position;
    // A common scale preserves all perspective ratios and bounds reciprocals.
    let inverse_w = w_scale / p.w;
    ScreenVertex {
        x: subpixel(viewport.x + (p.x / p.w + 1.0) * viewport.width * 0.5, subpixel_scale),
        y: subpixel(viewport.y + (1.0 - p.y / p.w) * viewport.height * 0.5, subpixel_scale),
        z: p.z / p.w,
        inverse_w,
        fog_depth_scale: w_scale * fog_scale,
        world_position: vertex.world_position,
        world_normal: vertex.world_normal,
        tex_coord: vertex.tex_coord,
        tex_coord2: vertex.tex_coord2,
        r: vertex.color.x * inverse_w,
        g: vertex.color.y * inverse_w,
        b: vertex.color.z * inverse_w,
        a: vertex.color.w * inverse_w,
    }
}

fn edge(a: &ScreenVertex, b: &ScreenVertex, x: f32, y: f32) -> f32 {
    (a.y - b.y) * x + (b.x - a.x) * y + (a.x * b.y - a.y * b.x)
}

fn lower_left(a: &ScreenVertex, b: &ScreenVertex) -> bool {
    b.y < a.y || (b.y == a.y && b.x < a.x)
}

fn classify_blend(blend: &(BlendFactor, BlendFactor)) -> BlendMode {
    if *blend == (BlendFactor::One, BlendFactor::Zero) {
        BlendMode::Opaque
    } else if *blend == (BlendFactor::SrcAlpha, BlendFactor::OneMinusSrcAlpha) {
        BlendMode::Alpha
    } else if *blend == (BlendFactor::One, BlendFactor::One) {
        BlendMode::Add
    } else if *blend == (BlendFactor::DstColor, BlendFactor::Zero)
        || *blend == (BlendFactor::Zero, BlendFactor::SrcColor)
    {
        BlendMode::Multiply
    } else if *blend == (BlendFactor::DstColor, BlendFactor::OneMinusDstAlpha) {
        BlendMode::DstColorInverseDstAlpha
    } else {
        BlendMode::General
    }
}

/// RGBA rows are top to bottom; batch positions are homogeneous clip coordinates.
pub struct SoftwareRenderer {
    width: u32,
    height: u32,
    owner: ResourceOwner,
    subpixel_bits: u32,
    stencil_bits: u32,
    alpha_bits: u32,
    subpixel_scale: f32,
    stencil_maximum: u32,
    framebuffer: Framebuffer,
    opacity_framebuffer: Option<Framebuffer>,
    opacity_active: bool,
    images: CpuImages,
    sampled: Sample,
    viewport: Rect,
    clip_plane: Option<Vec4>,
    retained_state: RenderState,
    clear_color: Vec4,
    clear_depth: f32,
    stencil_enabled: bool,
    stencil_function: StencilFunction,
    stencil_compare_mask: u32,
    stencil_write_mask: u32,
    stencil_depth_fail: StencilOp,
    stencil_depth_pass: StencilOp,
    color_write: bool,
    gamma_table: Option<[u8; 256]>,
    output_pixels: Option<Vec<u8>>,
    closed: bool,
    /// Triangle setups needed before strip-parallel shading kicks in.
    parallel_min_setups: usize,
    /// Strip count for parallel shading.
    parallel_threads: usize,
    /// Reused scratch for retained-draw resolution (no per-frame allocs).
    retained_positions: Vec<Vec4>,
    /// Reused single-textured vertex scratch.
    retained_single: Vec<RenderVertex>,
    /// Reused paired-textured vertex scratch.
    retained_pair: Vec<MultitextureVertex>,
    /// Reused index scratch.
    retained_indices: Vec<u32>,
}

impl SoftwareRenderer {
    /// Create a renderer with 8 subpixel bits, 8 stencil bits, and 8 alpha bits.
    #[must_use]
    pub fn new(width: u32, height: u32, owner: ResourceOwner) -> Self {
        Self::with_config(width, height, owner, 8, 8, 8)
    }

    /// Create a renderer with explicit precision.
    #[must_use]
    pub fn with_config(
        width: u32,
        height: u32,
        owner: ResourceOwner,
        subpixel_bits: u32,
        stencil_bits: u32,
        alpha_bits: u32,
    ) -> Self {
        if width < 1 || height < 1 || u64::from(width) * u64::from(height) > isize::MAX as u64 / 8 {
            panic!("image dimensions must be positive integers with a safe storage size");
        }
        if !(4..=16).contains(&subpixel_bits) {
            panic!("CPU subpixel precision must be between 4 and 16 bits");
        }
        if stencil_bits > 32 {
            panic!("CPU stencil precision must be between 0 and 32 bits");
        }
        if alpha_bits != 0 && alpha_bits != 8 {
            panic!("CPU alpha precision must be 0 or 8 bits");
        }
        let mut framebuffer = Framebuffer::new(width, height, stencil_bits != 0);
        if alpha_bits == 0 {
            for chunk in framebuffer.pixels.chunks_mut(4) {
                chunk.copy_from_slice(&[0, 0, 0, 255]);
            }
        }
        Self {
            width,
            height,
            subpixel_bits,
            stencil_bits,
            alpha_bits,
            subpixel_scale: 2.0f32.powi(subpixel_bits as i32),
            stencil_maximum: if stencil_bits == 32 {
                u32::MAX
            } else {
                (1u32 << stencil_bits) - 1
            },
            viewport: Rect {
                x: 0.0,
                y: 0.0,
                width: width as f32,
                height: height as f32,
            },
            images: CpuImages::new(owner.clone()),
            owner,
            framebuffer,
            opacity_framebuffer: None,
            opacity_active: false,
            sampled: Sample {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            },
            clip_plane: None,
            retained_state: RenderState::opaque(CullFace::None),
            clear_color: vec4(0.0, 0.0, 0.0, 0.0),
            clear_depth: 1.0,
            stencil_enabled: false,
            stencil_function: StencilFunction::Always,
            stencil_compare_mask: 0xffff_ffff,
            stencil_write_mask: 0xffff_ffff,
            stencil_depth_fail: StencilOp::Keep,
            stencil_depth_pass: StencilOp::Keep,
            color_write: true,
            gamma_table: None,
            output_pixels: None,
            closed: false,
            parallel_min_setups: 512,
            parallel_threads: std::thread::available_parallelism().map_or(4, |threads| threads.get()),
            retained_positions: Vec::new(),
            retained_single: Vec::new(),
            retained_pair: Vec::new(),
            retained_indices: Vec::new(),
        }
    }

    fn assert_open(&self) {
        if self.closed {
            panic!("CPU renderer is closed");
        }
    }

    /// Presented pixels, gamma-corrected when a table is set.
    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        self.output_pixels.as_deref().unwrap_or(&self.framebuffer.pixels)
    }

    /// Display gamma uses the Q3 convention: values above one brighten output.
    pub fn set_output_gamma(&mut self, gamma: f32) {
        self.assert_open();
        if !gamma.is_finite() || gamma < 0.5 || gamma > 3.0 {
            panic!("output gamma must be between 0.5 and 3");
        }
        if gamma == 1.0 {
            self.gamma_table = None;
            self.output_pixels = None;
        } else {
            let mut table = [0u8; 256];
            for (index, slot) in table.iter_mut().enumerate() {
                let value = 255.0 * (f32::from(index as u8) / 255.0).powf(1.0 / gamma) + 0.5;
                *slot = (value as i32).clamp(0, 255) as u8;
            }
            self.gamma_table = Some(table);
            self.output_pixels = Some(vec![0u8; self.framebuffer.pixels.len()]);
        }
        self.finish();
    }

    /// Read back the finished frame.
    pub fn read_rgba(&mut self) -> ImageLevel {
        self.assert_open();
        self.finish();
        ImageLevel {
            width: self.width,
            height: self.height,
            pixels: self.pixels().to_vec(),
        }
    }

    fn fragment_lighting<'batch>(&self, batch: &'batch DrawBatch) -> CpuLighting<'batch> {
        match &batch.lighting {
            BatchLighting::Vertex => CpuLighting {
                parameters: &batch.lighting,
                depth: None,
            },
            BatchLighting::Q2World { atlas, .. } => {
                let depth = atlas.as_ref().map(|atlas| {
                    self.images
                        .depth_shared(&atlas.image)
                        .unwrap_or_else(|error| panic!("{error}"))
                });
                CpuLighting {
                    parameters: &batch.lighting,
                    depth,
                }
            }
            BatchLighting::Q2ModelShadow { atlas, .. } => {
                let depth = self
                    .images
                    .depth_shared(&atlas.image)
                    .unwrap_or_else(|error| panic!("{error}"));
                CpuLighting {
                    parameters: &batch.lighting,
                    depth: Some(depth),
                }
            }
        }
    }

    /// Apply Q1 depth fog to the current viewport.
    pub fn apply_q1_fog(&mut self, projection: &Mat4, fog: &Q1Fog, sky_fraction: f32) {
        self.assert_open();
        let viewport = self.viewport;
        let clear_depth = self.clear_depth;
        apply_q1_depth_fog(
            &mut self.framebuffer,
            &viewport,
            projection,
            fog,
            sky_fraction,
            Q1FogOutput::TrueColor,
            clear_depth,
        );
    }

    fn apply_diagnostic_state(&mut self, state: RenderState) {
        self.retained_state = state;
    }

    /// Draw one batch: prepare, bind, draw, release.
    pub fn draw(&mut self, batch: &DrawBatch) {
        let mut prepared = self.prepare_geometry(batch);
        let texture = prepared.batch.texture.clone();
        let second = match &prepared.batch.vertices {
            BatchVertices::Pair { second_texture, .. } => Some(second_texture.binding.clone()),
            BatchVertices::Single(_) => None,
        };
        prepared.begin();
        prepared.apply_texture(0, &texture);
        if let Some(second) = second {
            prepared.apply_texture(1, &second);
        }
        prepared.draw();
        prepared.cleanup();
    }

    /// Resolve a retained draw through the frame projector into reused
    /// scratch and draw each batch. Positions resolve with the exact legacy
    /// projection, so output matches the immediate path bitwise. Scratch
    /// vectors round-trip through each assembled batch, so steady-state
    /// resolution allocates nothing.
    fn draw_retained(&mut self, draw: &RetainedDraw) {
        self.assert_open();
        let projector = ViewProjector::from_rows(draw.eye, draw.projection);
        resolve_retained_positions(draw, &projector, &mut self.retained_positions);
        for index in 0..draw.batches.len() {
            let paired = draw.batches[index].second_texture.is_some();
            {
                let Self {
                    retained_positions,
                    retained_single,
                    retained_pair,
                    retained_indices,
                    ..
                } = self;
                fill_retained_batch_vertices(
                    draw,
                    index,
                    retained_positions,
                    paired,
                    retained_single,
                    retained_pair,
                    retained_indices,
                );
            }
            let batch = &draw.batches[index];
            let vertices = if paired {
                BatchVertices::Pair {
                    vertices: std::mem::take(&mut self.retained_pair),
                    second_texture: batch
                        .second_texture
                        .clone()
                        .expect("paired retained batch keeps its bundle"),
                }
            } else {
                BatchVertices::Single(std::mem::take(&mut self.retained_single))
            };
            let assembled = DrawBatch {
                fog: batch.fog,
                luminance_alpha: batch.luminance_alpha,
                indices: std::mem::take(&mut self.retained_indices),
                texture: batch.texture.clone(),
                state: batch.state,
                lighting: batch.lighting.clone(),
                primitive: batch.primitive,
                vertices,
            };
            self.draw(&assembled);
            match assembled.vertices {
                BatchVertices::Single(vertices) => self.retained_single = vertices,
                BatchVertices::Pair { vertices, .. } => self.retained_pair = vertices,
            }
            self.retained_indices = assembled.indices;
        }
    }

    /// Stretch a picture over the full framebuffer with the 2D color.
    pub fn draw_stretch_pic(&mut self, image: &RendererImage, rect: &Rect, uv: &TextureRect, color: &Vec4) {
        self.begin_view(&RenderViewState {
            viewport: Rect {
                x: 0.0,
                y: 0.0,
                width: self.width as f32,
                height: self.height as f32,
            },
            clear: None,
            clip_plane: None,
        });
        let points = [
            (rect.x, rect.y, uv.s1, uv.t1),
            (rect.x + rect.width, rect.y, uv.s2, uv.t1),
            (rect.x + rect.width, rect.y + rect.height, uv.s2, uv.t2),
            (rect.x, rect.y + rect.height, uv.s1, uv.t2),
        ];
        self.draw(&DrawBatch {
            fog: None,
            luminance_alpha: false,
            indices: vec![0, 1, 2, 0, 2, 3],
            texture: TextureBinding::BindImage(image.clone()),
            state: RenderState {
                blend: (BlendFactor::SrcAlpha, BlendFactor::OneMinusSrcAlpha),
                depth_test: DepthTest::Always,
                depth_write: false,
                ..RenderState::opaque(CullFace::None)
            },
            lighting: BatchLighting::Vertex,
            primitive: BatchPrimitive::Triangles,
            vertices: BatchVertices::Single(
                points
                    .iter()
                    .map(|(x, y, s, t)| RenderVertex {
                        position: vec4(
                            x * 2.0 / self.width as f32 - 1.0,
                            1.0 - y * 2.0 / self.height as f32,
                            -1.0,
                            1.0,
                        ),
                        tex_coord: vec2(*s, *t),
                        color: *color,
                    })
                    .collect(),
            ),
        });
    }

    fn draw_batch_internal(&mut self, batch: &DrawBatch, texture: &BoundTexture, secondary: &BoundTexture) {
        if batch.indices.is_empty() {
            return;
        }
        match batch.primitive {
            BatchPrimitive::Lines { line_width } => {
                let mut offset = 0;
                while offset < batch.indices.len() {
                    let (a, b) = (indexed_vertex(batch, offset), indexed_vertex(batch, offset + 1));
                    self.draw_line(a, b, line_width, batch, texture, secondary);
                    offset += 2;
                }
            }
            BatchPrimitive::Triangles => {
                let setups = self.build_batch_setups(batch, texture, secondary);
                self.shade_setups(&setups);
            }
        }
    }

    /// Build owned triangle setups for one triangle batch. Lines shade
    /// immediately and never collect.
    fn build_batch_setups<'batch>(
        &self,
        batch: &'batch DrawBatch,
        texture: &BoundTexture,
        secondary: &BoundTexture,
    ) -> Vec<TriangleSetup<'batch>> {
        if !matches!(batch.primitive, BatchPrimitive::Triangles) {
            panic!("triangle setup collection needs a triangle batch");
        }
        let mut setups = Vec::new();
        let mut offset = 0;
        while offset < batch.indices.len() {
            let (a, b, c) = (
                indexed_vertex(batch, offset),
                indexed_vertex(batch, offset + 1),
                indexed_vertex(batch, offset + 2),
            );
            self.collect_triangle(&mut setups, a, b, c, batch, texture, secondary);
            offset += 3;
        }
        setups
    }

    /// Shade collected setups into the live framebuffer: serially below the
    /// parallel threshold, or across row strips on scoped threads above it.
    /// Strips borrow disjoint rows and setups run in order per strip, so
    /// parallel shading is pixel-identical to serial shading.
    fn shade_setups(&mut self, setups: &[TriangleSetup<'_>]) {
        if setups.is_empty() {
            return;
        }
        if setups.len() < self.parallel_min_setups || self.parallel_threads < 2 {
            let mut target = self.framebuffer.whole();
            for setup in setups {
                run_triangle_rows(setup, &mut target, &mut self.sampled, setup.min_y, setup.max_y);
            }
            return;
        }
        let mut strips = self.framebuffer.split_strips(self.parallel_threads);
        std::thread::scope(|scope| {
            for mut strip in strips.drain(..) {
                let origin_y = strip.origin_y;
                let rows = strip.depth.len() / strip.stride.max(1) as usize;
                let last_y = origin_y + rows as i32 - 1;
                scope.spawn(move || {
                    let mut sampled = Sample {
                        r: 1.0,
                        g: 1.0,
                        b: 1.0,
                        a: 1.0,
                    };
                    for setup in setups {
                        let first_y = setup.min_y.max(origin_y);
                        let strip_last_y = setup.max_y.min(last_y);
                        if first_y <= strip_last_y {
                            run_triangle_rows(setup, &mut strip, &mut sampled, first_y, strip_last_y);
                        }
                    }
                });
            }
        });
    }

    /// Draw many triangle batches: begin and bind each one serially, collect
    /// every triangle setup, then shade the whole run at once. Backends
    /// gather consecutive triangle batches here so strip-parallel shading
    /// amortizes one scoped spawn over the run instead of paying it per
    /// tiny batch. Setup and shading attribute to their own stages.
    pub fn draw_batches(&mut self, batches: &[&DrawBatch], timer: &mut StageTimer) {
        timer.section("cpu_setup");
        let mut setups = Vec::new();
        for batch in batches {
            if batch.indices.is_empty() {
                continue;
            }
            if !matches!(batch.primitive, BatchPrimitive::Triangles) {
                panic!("batched draws need triangle batches");
            }
            let mut prepared = self.prepare_geometry(batch);
            let texture = prepared.batch.texture.clone();
            let second = match &prepared.batch.vertices {
                BatchVertices::Pair { second_texture, .. } => Some(second_texture.binding.clone()),
                BatchVertices::Single(_) => None,
            };
            prepared.begin();
            prepared.apply_texture(0, &texture);
            if let Some(second) = second {
                prepared.apply_texture(1, &second);
            }
            setups.extend(prepared.collect_triangle_setups(batch));
            prepared.cleanup();
        }
        timer.section("cpu_shade");
        self.shade_setups(&setups);
    }

    fn draw_line(
        &mut self,
        mut a: CpuVertex,
        mut b: CpuVertex,
        width: f32,
        batch: &DrawBatch,
        texture: &BoundTexture,
        secondary: &BoundTexture,
    ) {
        if let Some(clip) = self.clip_plane {
            let plane = ClipPlane::Custom(clip);
            let (da, db) = (plane_distance(&a, &plane), plane_distance(&b, &plane));
            if da < 0.0 && db < 0.0 {
                return;
            }
            if da < 0.0 {
                a = intersect(&a, &b, da, db, &plane);
            } else if db < 0.0 {
                b = intersect(&a, &b, da, db, &plane);
            }
        }
        self.rasterize_line(&a, &b, width, batch, texture, secondary);
    }

    fn rasterize_line(
        &mut self,
        a: &CpuVertex,
        b: &CpuVertex,
        width: f32,
        batch: &DrawBatch,
        texture: &BoundTexture,
        secondary: &BoundTexture,
    ) {
        let viewport = self.viewport;
        let scissor = LineScissor {
            min_x: 0.max(-(viewport.x as i32)),
            min_y: 0.max(-(viewport.y as i32)),
            max_x: (viewport.width as i32 - 1).min(self.width as i32 - 1 - viewport.x as i32),
            max_y: (viewport.height as i32 - 1).min(self.height as i32 - 1 - viewport.y as i32),
        };
        let mut fragments = Vec::new();
        rasterize_aliased_line(
            a,
            b,
            viewport.width,
            viewport.height,
            width,
            &scissor,
            &mut |fragment| {
                fragments.push(fragment);
            },
        );
        for fragment in fragments {
            let (x, y) = (fragment.x + viewport.x as i32, fragment.y + viewport.y as i32);
            if x >= 0 && x < self.width as i32 && y >= 0 && y < self.height as i32 {
                self.line_fragment(&LineFragment { x, y, ..fragment }, batch, texture, secondary);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn collect_triangle<'batch>(
        &self,
        setups: &mut Vec<TriangleSetup<'batch>>,
        a: CpuVertex,
        b: CpuVertex,
        c: CpuVertex,
        batch: &'batch DrawBatch,
        texture: &BoundTexture,
        secondary: &BoundTexture,
    ) {
        let original = [a, b, c];
        let mut magnitude = 0.0f32;
        if batch.fog.is_some() {
            for vertex in &original {
                let p = &vertex.position;
                magnitude = magnitude.max(p.x.abs()).max(p.y.abs()).max(p.z.abs()).max(p.w.abs());
            }
        }
        let fog_scale = if magnitude > f32::MAX / 4.0 { magnitude } else { 1.0 };
        let polygon = clip_polygon(&original, self.clip_plane);
        let Some(first) = polygon.first().copied() else {
            return;
        };
        let mut interpolation: Option<[ScreenVertex; 3]> = None;
        if original.iter().all(|vertex| {
            vertex.position.w > 0.0 && vertex.position.z >= -vertex.position.w && vertex.position.z <= vertex.position.w
        }) && original
            .iter()
            .any(|vertex| vertex.position.x.abs() > vertex.position.w || vertex.position.y.abs() > vertex.position.w)
        {
            // The measured GL profile retains the original snapped interpolation
            // plane. Rebuilding it from newly snapped clip vertices changes UVs on
            // thin triangles. Coverage still uses the bounded clipped polygon.
            let scale = original[0]
                .position
                .w
                .min(original[1].position.w)
                .min(original[2].position.w);
            let projected = [
                project(&original[0], &self.viewport, scale, self.subpixel_scale, fog_scale),
                project(&original[1], &self.viewport, scale, self.subpixel_scale, fog_scale),
                project(&original[2], &self.viewport, scale, self.subpixel_scale, fog_scale),
            ];
            let area = edge(&projected[0], &projected[1], projected[2].x, projected[2].y);
            if area.is_finite() {
                if area == 0.0 {
                    return;
                }
                interpolation = Some(projected);
            }
        }
        for pair in polygon[1..].windows(2) {
            let (second, third) = (pair[0], pair[1]);
            let w_scale = first.position.w.min(second.position.w).min(third.position.w);
            let viewport = self.viewport;
            let subpixel_scale = self.subpixel_scale;
            if let Some(setup) = self.triangle_setup(
                project(&first, &viewport, w_scale, subpixel_scale, fog_scale),
                project(&second, &viewport, w_scale, subpixel_scale, fog_scale),
                project(&third, &viewport, w_scale, subpixel_scale, fog_scale),
                batch,
                texture,
                secondary,
                interpolation,
            ) {
                setups.push(setup);
            }
        }
    }

    fn line_fragment(
        &mut self,
        fragment: &LineFragment,
        batch: &DrawBatch,
        texture: &BoundTexture,
        secondary_texture: &BoundTexture,
    ) {
        let index = fragment.y as usize * self.width as usize + fragment.x as usize;
        let previous_depth = *self
            .framebuffer
            .depth
            .get(index)
            .expect("line fragment outside framebuffer");
        let state = batch.state;
        let (near, far) = (clamp(state.depth_range[0]), clamp(state.depth_range[1]));
        let depth = clamp(fragment.depth) * (far - near) + near;
        let depth_passed = !((state.depth_test == DepthTest::LessEqual && depth > previous_depth)
            || (state.depth_test == DepthTest::Equal && depth != previous_depth));
        if !depth_passed && !self.stencil_enabled {
            return;
        }
        let texture_consumed = self.color_write || state.alpha_test != AlphaTest::None;
        let (mut r, mut g, mut b, mut alpha) = (
            clamp(fragment.color.x),
            clamp(fragment.color.y),
            clamp(fragment.color.z),
            clamp(fragment.color.w),
        );
        let vertex_color = vec4(r, g, b, alpha);
        let mut texel = Sample {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
        };
        if texture_consumed && !matches!(texture, BoundTexture::Incomplete) {
            sample_line_bound(
                texture,
                &fragment.tex_coord,
                &fragment.tex_coord_derivative,
                &mut self.sampled,
            );
            let mut sampled = self.sampled;
            let has_alpha = bound_alpha(texture);
            texel = Sample {
                r: sampled.r,
                g: sampled.g,
                b: sampled.b,
                a: if has_alpha { sampled.a } else { 1.0 },
            };
            if batch.luminance_alpha {
                let modulation = (sampled.r + sampled.g + sampled.b) / 3.0 * alpha;
                sampled.r *= modulation;
                sampled.g *= modulation;
                sampled.b *= modulation;
                texel.r = sampled.r;
                texel.g = sampled.g;
                texel.b = sampled.b;
            }
            r *= sampled.r;
            g *= sampled.g;
            b *= sampled.b;
            if has_alpha {
                alpha *= sampled.a;
            }
        }
        if texture_consumed && !matches!(batch.lighting, BatchLighting::Vertex) {
            let lighting = self.fragment_lighting(batch);
            let result = shade_q2_fragment(
                &lighting,
                fragment.world_position,
                fragment.world_normal,
                vertex_color,
                &texel,
            );
            r = result.r;
            g = result.g;
            b = result.b;
            alpha = result.a;
        }
        if texture_consumed && !matches!(secondary_texture, BoundTexture::Incomplete) {
            if let BatchVertices::Pair { second_texture, .. } = &batch.vertices {
                sample_line_bound(
                    secondary_texture,
                    &fragment.tex_coord2,
                    &fragment.tex_coord2_derivative,
                    &mut self.sampled,
                );
                let sampled = self.sampled;
                let environment = second_texture.environment;
                r = texture_color(r, sampled.r, environment);
                g = texture_color(g, sampled.g, environment);
                b = texture_color(b, sampled.b, environment);
                if bound_alpha(secondary_texture) {
                    alpha = if environment == PairEnvironment::Replace {
                        sampled.a
                    } else {
                        alpha * sampled.a
                    };
                }
            }
        }
        if let Some(fog) = &batch.fog {
            apply_line_fog(fog, fragment.eye_depth, &mut r, &mut g, &mut b, &mut alpha);
        }
        if !passes_alpha(alpha, state.alpha_test) {
            return;
        }
        if self.stencil_enabled
            && !stencil_fragment(
                self.framebuffer.stencil.as_deref_mut(),
                index,
                depth_passed,
                StencilTest {
                    function: self.stencil_function,
                    compare_mask: self.stencil_compare_mask,
                    write_mask: self.stencil_write_mask,
                    maximum: self.stencil_maximum,
                    depth_fail: self.stencil_depth_fail,
                    depth_pass: self.stencil_depth_pass,
                },
            )
        {
            return;
        }
        if !self.color_write {
            return;
        }
        let [dr, dg, db, da_full] = self.framebuffer.load_normalized(index);
        let da = if self.alpha_bits == 0 { 1.0 } else { da_full };
        let (r, g, b, alpha) = (clamp(r), clamp(g), clamp(b), clamp(alpha));
        self.framebuffer.store_bytes(
            index,
            [
                blend(r, dr, alpha, da, &state.blend, false),
                blend(g, dg, alpha, da, &state.blend, false),
                blend(b, db, alpha, da, &state.blend, false),
                if self.alpha_bits == 0 {
                    255
                } else {
                    blend(alpha, da, alpha, da, &state.blend, true)
                },
            ],
        );
        if state.depth_write {
            self.framebuffer.depth[index] = depth;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn triangle_setup<'batch>(
        &self,
        a: ScreenVertex,
        mut b: ScreenVertex,
        mut c: ScreenVertex,
        batch: &'batch DrawBatch,
        texture: &BoundTexture,
        secondary_texture: &BoundTexture,
        interpolation: Option<[ScreenVertex; 3]>,
    ) -> Option<TriangleSetup<'batch>> {
        let mut area = edge(&a, &b, c.x, c.y);
        if !area.is_finite() || area == 0.0 {
            return None;
        }
        if (batch.state.cull == CullFace::Back && area > 0.0) || (batch.state.cull == CullFace::Front && area < 0.0) {
            return None;
        }
        if area < 0.0 {
            std::mem::swap(&mut b, &mut c);
            area = -area;
        }
        let viewport = self.viewport;
        let min_x = 0
            .max(viewport.x as i32)
            .max((a.x.min(b.x).min(c.x) - 0.5).ceil() as i32);
        let max_x = (self.width as i32 - 1)
            .min(viewport.x as i32 + viewport.width as i32 - 1)
            .min((a.x.max(b.x).max(c.x) - 0.5).floor() as i32);
        let min_y = 0
            .max(viewport.y as i32)
            .max((a.y.min(b.y).min(c.y) - 0.5).ceil() as i32);
        let max_y = (self.height as i32 - 1)
            .min(viewport.y as i32 + viewport.height as i32 - 1)
            .min((a.y.max(b.y).max(c.y) - 0.5).floor() as i32);
        let edge_a_inclusive = lower_left(&b, &c);
        let edge_b_inclusive = lower_left(&c, &a);
        let edge_c_inclusive = lower_left(&a, &b);
        let attributes = interpolation.unwrap_or([a, b, c]);
        let [ia, ib, ic] = attributes;
        let inverse_area = 1.0
            / if interpolation.is_some() {
                edge(&ia, &ib, ic.x, ic.y)
            } else {
                area
            };
        let state = batch.state;
        let (depth_near, depth_far) = (clamp(state.depth_range[0]), clamp(state.depth_range[1]));
        let white = attributes
            .iter()
            .all(|v| v.r == v.inverse_w && v.g == v.inverse_w && v.b == v.inverse_w && v.a == v.inverse_w);
        let blending = classify_blend(&state.blend);
        let texture_consumed = self.color_write || state.alpha_test != AlphaTest::None;
        let secondary_environment = match &batch.vertices {
            BatchVertices::Pair { second_texture, .. } => Some(second_texture.environment),
            BatchVertices::Single(_) => None,
        };
        let (edge_ax, edge_ay, edge_ac) = (b.y - c.y, c.x - b.x, b.x * c.y - b.y * c.x);
        let (edge_bx, edge_by, edge_bc) = (c.y - a.y, a.x - c.x, c.x * a.y - c.y * a.x);
        let (edge_cx, edge_cy, edge_cc) = (a.y - b.y, b.x - a.x, a.x * b.y - a.y * b.x);
        let (attribute_ax, attribute_ay, attribute_ac) = (ib.y - ic.y, ic.x - ib.x, ib.x * ic.y - ib.y * ic.x);
        let (attribute_bx, attribute_by, attribute_bc) = (ic.y - ia.y, ia.x - ic.x, ic.x * ia.y - ic.y * ia.x);
        let (attribute_cx, attribute_cy, attribute_cc) = (ia.y - ib.y, ib.x - ia.x, ia.x * ib.y - ia.y * ib.x);
        let (az, bz, cz) = (ia.z, ib.z, ic.z);
        let constant_depth = az == bz && bz == cz;
        let slope = (az * attribute_ax + bz * attribute_bx + cz * attribute_cx)
            .abs()
            .max((az * attribute_ay + bz * attribute_by + cz * attribute_cy).abs())
            * inverse_area.abs()
            * 0.5
            * (depth_far - depth_near).abs();
        // Fixed-point 24-bit depth resolution, matching the SDL GL depth buffer.
        let polygon_depth_offset = state.polygon_offset.map_or(0.0, |offset| {
            slope * offset.factor + 2.0f32.powi(-(CPU_OFFSET_DEPTH_BITS as i32)) * offset.units
        });
        let plane_depth = clamp(clamp(az * 0.5 + 0.5) * (depth_far - depth_near) + depth_near + polygon_depth_offset);
        let (aiw, biw, ciw) = (ia.inverse_w, ib.inverse_w, ic.inverse_w);
        // Delay UV/W until a common anchor is known. Subtracting U' - s*Q' with
        // large absolute s can invent a nonzero LOD for a constant coordinate.
        let (u_anchor, v_anchor) = (ia.tex_coord.x, ia.tex_coord.y);
        let (au, bu, cu) = (
            (ia.tex_coord.x - u_anchor) * aiw,
            (ib.tex_coord.x - u_anchor) * biw,
            (ic.tex_coord.x - u_anchor) * ciw,
        );
        let (av, bv, cv) = (
            (ia.tex_coord.y - v_anchor) * aiw,
            (ib.tex_coord.y - v_anchor) * biw,
            (ic.tex_coord.y - v_anchor) * ciw,
        );
        let (u2_anchor, v2_anchor) = (ia.tex_coord2.x, ia.tex_coord2.y);
        let (au2, bu2, cu2) = (
            (ia.tex_coord2.x - u2_anchor) * aiw,
            (ib.tex_coord2.x - u2_anchor) * biw,
            (ic.tex_coord2.x - u2_anchor) * ciw,
        );
        let (av2, bv2, cv2) = (
            (ia.tex_coord2.y - v2_anchor) * aiw,
            (ib.tex_coord2.y - v2_anchor) * biw,
            (ic.tex_coord2.y - v2_anchor) * ciw,
        );
        let q_dx = (aiw * attribute_ax + biw * attribute_bx + ciw * attribute_cx) * inverse_area;
        let q_dy = (aiw * attribute_ay + biw * attribute_by + ciw * attribute_cy) * inverse_area;
        let derivative = TexturePlaneDerivative {
            u_anchor,
            v_anchor,
            u_dx: (au * attribute_ax + bu * attribute_bx + cu * attribute_cx) * inverse_area,
            v_dx: (av * attribute_ax + bv * attribute_bx + cv * attribute_cx) * inverse_area,
            q_dx,
            u_dy: (au * attribute_ay + bu * attribute_by + cu * attribute_cy) * inverse_area,
            v_dy: (av * attribute_ay + bv * attribute_by + cv * attribute_cy) * inverse_area,
            q_dy,
        };
        let secondary_derivative = TexturePlaneDerivative {
            u_anchor: u2_anchor,
            v_anchor: v2_anchor,
            u_dx: (au2 * attribute_ax + bu2 * attribute_bx + cu2 * attribute_cx) * inverse_area,
            v_dx: (av2 * attribute_ax + bv2 * attribute_bx + cv2 * attribute_cx) * inverse_area,
            q_dx,
            u_dy: (au2 * attribute_ay + bu2 * attribute_by + cu2 * attribute_cy) * inverse_area,
            v_dy: (av2 * attribute_ay + bv2 * attribute_by + cv2 * attribute_cy) * inverse_area,
            q_dy,
        };
        let lighting = self.fragment_lighting(batch);
        Some(TriangleSetup {
            fog: batch.fog,
            fog_depth_scale: ia.fog_depth_scale,
            luminance_alpha: batch.luminance_alpha,
            lighting: CpuTriangleLighting {
                parameters: lighting.parameters,
                depth: lighting.depth,
                positions: [ia.world_position, ib.world_position, ic.world_position],
                normals: [ia.world_normal, ib.world_normal, ic.world_normal],
            },
            min_x,
            max_x,
            min_y,
            max_y,
            inverse_area,
            depth_near,
            depth_far,
            edge_ax,
            edge_ay,
            edge_ac,
            edge_bx,
            edge_by,
            edge_bc,
            edge_cx,
            edge_cy,
            edge_cc,
            attribute_ax,
            attribute_ay,
            attribute_ac,
            attribute_bx,
            attribute_by,
            attribute_bc,
            attribute_cx,
            attribute_cy,
            attribute_cc,
            az,
            bz,
            cz,
            polygon_depth_offset,
            plane_depth,
            aiw,
            biw,
            ciw,
            au,
            bu,
            cu,
            av,
            bv,
            cv,
            au2,
            bu2,
            cu2,
            av2,
            bv2,
            cv2,
            ar: ia.r,
            br: ib.r,
            cr: ic.r,
            ag: ia.g,
            bg: ib.g,
            cg: ic.g,
            ab: ia.b,
            bb: ib.b,
            cb: ic.b,
            aa: ia.a,
            ba: ib.a,
            ca: ic.a,
            edge_a_inclusive,
            edge_b_inclusive,
            edge_c_inclusive,
            white,
            depth_write: state.depth_write,
            stencil_enabled: self.stencil_enabled,
            color_write: self.color_write,
            texture_consumed,
            primary_alpha: bound_alpha(texture),
            secondary_alpha: bound_alpha(secondary_texture),
            constant_depth,
            width: self.width,
            height: self.height,
            blend: state.blend,
            alpha_bits: self.alpha_bits,
            blending,
            depth_test: state.depth_test,
            alpha_test: state.alpha_test,
            secondary_environment,
            texture: texture.clone(),
            secondary_texture: secondary_texture.clone(),
            derivative,
            secondary_derivative,
            stencil_function: self.stencil_function,
            stencil_compare_mask: self.stencil_compare_mask,
            stencil_write_mask: self.stencil_write_mask,
            stencil_maximum: self.stencil_maximum,
            stencil_depth_fail: self.stencil_depth_fail,
            stencil_depth_pass: self.stencil_depth_pass,
        })
    }

    fn shadow_pass(
        &mut self,
        positions: &[Vec4],
        indices: &[u32],
        mirror: bool,
        white_image: &RendererImage,
        finishing: bool,
    ) {
        if self.stencil_bits < 4 {
            panic!("CPU stencil shadows require at least four stencil bits");
        }
        let shade = if finishing { 0.6f32 } else { 0.2f32 };
        let vertices: Vec<RenderVertex> = positions
            .iter()
            .map(|position| RenderVertex {
                position: *position,
                tex_coord: vec2(0.0, 0.0),
                color: vec4(shade, shade, shade, 1.0),
            })
            .collect();
        self.stencil_enabled = true;
        self.stencil_function = if finishing {
            StencilFunction::NonZero
        } else {
            StencilFunction::Always
        };
        self.stencil_compare_mask = 255;
        self.stencil_depth_fail = StencilOp::Keep;
        self.stencil_depth_pass = if finishing {
            StencilOp::Keep
        } else {
            StencilOp::Increment
        };
        self.color_write = finishing;
        if finishing {
            self.clip_plane = None;
        }
        let mut state = self.retained_state;
        state.alpha_test = AlphaTest::None;
        state.depth_test = DepthTest::LessEqual;
        state.depth_write = finishing;
        state.blend = if finishing {
            (BlendFactor::DstColor, BlendFactor::Zero)
        } else {
            (BlendFactor::One, BlendFactor::Zero)
        };
        state.cull = if finishing {
            CullFace::None
        } else if mirror {
            CullFace::Front
        } else {
            CullFace::Back
        };
        let batch = DrawBatch {
            fog: None,
            luminance_alpha: false,
            indices: indices.to_vec(),
            texture: TextureBinding::BindImage(white_image.clone()),
            state,
            lighting: BatchLighting::Vertex,
            primitive: BatchPrimitive::Triangles,
            vertices: BatchVertices::Single(vertices),
        };
        self.draw(&batch);
        if !finishing {
            self.stencil_depth_pass = StencilOp::Decrement;
            let mut flipped = batch;
            flipped.state.cull = if flipped.state.cull == CullFace::Front {
                CullFace::Back
            } else {
                CullFace::Front
            };
            self.draw(&flipped);
        }
        self.color_write = true;
        if finishing {
            self.stencil_enabled = false;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpacityAction {
    Skip,
    Direct,
    Scoped,
}

impl SoftwareRenderer {
    fn classify_opacity(&self, opacity: f32) -> OpacityAction {
        if !opacity.is_finite() || opacity < 0.0 || opacity > 1.0 {
            panic!("object opacity must be in 0..1");
        }
        if self.opacity_active {
            panic!("object opacity scopes cannot nest");
        }
        if opacity == 0.0 {
            OpacityAction::Skip
        } else if opacity == 1.0 {
            OpacityAction::Direct
        } else {
            OpacityAction::Scoped
        }
    }

    fn enter_opacity_scope(&mut self) {
        if self.opacity_framebuffer.is_none() {
            let parent = &self.framebuffer;
            self.opacity_framebuffer = Some(Framebuffer {
                width: parent.width,
                height: parent.height,
                pixels: vec![0u8; parent.pixels.len()],
                depth: vec![1.0f32; parent.depth.len()],
                stencil: parent.stencil.as_ref().map(|stencil| vec![0u32; stencil.len()]),
                origin_x: parent.origin_x,
                origin_y: parent.origin_y,
                stride: parent.stride,
            });
        }
        let scratch = self
            .opacity_framebuffer
            .as_mut()
            .expect("opacity scratch allocated above");
        scratch.pixels.copy_from_slice(&self.framebuffer.pixels);
        scratch.depth.copy_from_slice(&self.framebuffer.depth);
        if let (Some(scratch), Some(parent)) = (scratch.stencil.as_mut(), self.framebuffer.stencil.as_ref()) {
            scratch.copy_from_slice(parent);
        }
        self.opacity_active = true;
        std::mem::swap(
            &mut self.framebuffer,
            self.opacity_framebuffer
                .as_mut()
                .expect("opacity scratch allocated above"),
        );
    }

    fn exit_opacity_scope(&mut self, opacity: f32) {
        std::mem::swap(
            &mut self.framebuffer,
            self.opacity_framebuffer
                .as_mut()
                .expect("opacity scratch allocated above"),
        );
        self.opacity_active = false;
        let scratch = self
            .opacity_framebuffer
            .as_ref()
            .expect("opacity scratch allocated above");
        let viewport = self.viewport;
        let alpha_bits = self.alpha_bits;
        let left = 0.max(viewport.x as i32);
        let right = (self.width as i32).min(viewport.x as i32 + viewport.width as i32);
        for y in (viewport.y as i32).max(0)..(viewport.y as i32 + viewport.height as i32).min(self.height as i32) {
            for offset in (y * self.width as i32 + left) as usize * 4..(y * self.width as i32 + right) as usize * 4 {
                let parent = &mut self.framebuffer;
                let backdrop = f32::from(parent.pixels[offset]);
                let result = f32::from(scratch.pixels[offset]);
                parent.pixels[offset] = if alpha_bits == 0 && offset % 4 == 3 {
                    255
                } else {
                    (backdrop * (1.0 - opacity) + result * opacity).round() as u8
                };
            }
        }
    }
}

/// Restores the opacity scope when the scoped draw panics.
struct OpacityGuard {
    renderer: *mut SoftwareRenderer,
    opacity: f32,
    dismissed: bool,
}

impl Drop for OpacityGuard {
    fn drop(&mut self) {
        if !self.dismissed {
            // The scoped draw panicked; its renderer borrow is dead, so the
            // raw pointer is the only live path back into the scope.
            unsafe {
                (*self.renderer).exit_opacity_scope(self.opacity);
            }
        }
    }
}

fn bound_alpha(texture: &BoundTexture) -> bool {
    match texture {
        BoundTexture::Incomplete => false,
        BoundTexture::Image(image) => texture_has_alpha(image.internal_format),
        BoundTexture::Constant(constant) => texture_has_alpha(constant.internal_format),
    }
}

/// Prepared CPU draw: begin, bind in slot order, draw, release.
pub struct SoftwarePrepared<'a> {
    renderer: &'a mut SoftwareRenderer,
    batch: DrawBatch,
    phase: PreparedPhase,
    next_unit: u32,
    resolved: Vec<(usize, RendererImage)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreparedPhase {
    Prepared,
    Begun,
    Drawn,
    Cleaned,
}

impl PreparedDraw for SoftwarePrepared<'_> {
    fn begin(&mut self) {
        self.renderer.assert_open();
        if self.phase != PreparedPhase::Prepared {
            panic!("CPU prepared draw has already begun");
        }
        let state = self.batch.state;
        self.renderer.apply_diagnostic_state(state);
        self.phase = PreparedPhase::Begun;
    }

    fn apply_texture(&mut self, unit: u32, binding: &TextureBinding) {
        self.renderer.assert_open();
        let paired = matches!(self.batch.vertices, BatchVertices::Pair { .. });
        if self.phase != PreparedPhase::Begun || unit != self.next_unit || unit > 1 || (unit == 1 && !paired) {
            panic!("CPU texture slots must execute in order");
        }
        // Resolve each dynamic source once per draw, after uploads and before binding.
        let resolved = match binding {
            TextureBinding::DynamicImage(source) => {
                let key = Arc::as_ptr(source) as *const () as usize;
                if let Some(hit) = self.resolved.iter().find(|(known, _)| *known == key) {
                    TextureBinding::BindImage(hit.1.clone())
                } else {
                    let renderer = &mut *self.renderer;
                    let image = source.resolve(&mut |operation| renderer.apply_image_resource(&operation));
                    self.resolved.push((key, image.clone()));
                    TextureBinding::BindImage(image)
                }
            }
            TextureBinding::BindImage(image) => TextureBinding::BindImage(image.clone()),
            TextureBinding::RetainCurrentTexture => TextureBinding::RetainCurrentTexture,
        };
        self.renderer.images.bind(unit, &resolved);
        self.next_unit += 1;
    }

    fn draw(&mut self) {
        self.renderer.assert_open();
        let paired = matches!(self.batch.vertices, BatchVertices::Pair { .. });
        if self.phase != PreparedPhase::Begun || self.next_unit != u32::from(paired) + 1 {
            panic!("CPU prepared draw has unapplied texture slots");
        }
        let primary = self.renderer.images.bound(0);
        let secondary = if paired {
            self.renderer.images.bound(1)
        } else {
            BoundTexture::Incomplete
        };
        self.renderer.draw_batch_internal(&self.batch, &primary, &secondary);
        self.phase = PreparedPhase::Drawn;
    }

    fn cleanup(&mut self) {
        self.renderer.assert_open();
        if self.phase != PreparedPhase::Drawn {
            panic!("CPU prepared draw has not completed");
        }
        self.phase = PreparedPhase::Cleaned;
    }
}

impl SoftwarePrepared<'_> {
    /// Collect triangle setups without shading, for batched runs. `batch`
    /// is the original (uncloned) batch so setups borrow from data that
    /// outlives this prepared draw; it holds identical values to the
    /// prepared clone. The caller finishes with `cleanup` without `draw`.
    fn collect_triangle_setups<'batch>(&mut self, batch: &'batch DrawBatch) -> Vec<TriangleSetup<'batch>> {
        self.renderer.assert_open();
        let paired = matches!(self.batch.vertices, BatchVertices::Pair { .. });
        if self.phase != PreparedPhase::Begun || self.next_unit != u32::from(paired) + 1 {
            panic!("CPU prepared draw has unapplied texture slots");
        }
        let primary = self.renderer.images.bound(0);
        let secondary = if paired {
            self.renderer.images.bound(1)
        } else {
            BoundTexture::Incomplete
        };
        let setups = self.renderer.build_batch_setups(batch, &primary, &secondary);
        self.phase = PreparedPhase::Drawn;
        setups
    }
}

impl OrderedBackend for SoftwareRenderer {
    type Prepared<'a> = SoftwarePrepared<'a>;

    fn owner(&self) -> &ResourceOwner {
        &self.owner
    }

    fn width(&self) -> u32 {
        self.width
    }

    fn height(&self) -> u32 {
        self.height
    }

    fn stencil_bits(&self) -> u32 {
        self.stencil_bits
    }

    fn apply_image_resource(&mut self, operation: &ImageResourceOperation) {
        self.assert_open();
        self.images.apply(operation);
    }

    fn select_draw_buffer(&mut self, buffer: DrawBuffer, clear: bool) {
        self.assert_open();
        if !matches!(buffer, DrawBuffer::Front | DrawBuffer::Back) {
            panic!("CPU stereo draw buffers are unavailable");
        }
        if clear {
            let viewport = self.viewport;
            let clip_plane = self.clip_plane;
            self.begin_view(&RenderViewState {
                viewport,
                clear: Some(ViewClear {
                    depth: 1.0,
                    color: Some(vec4(1.0, 0.0, 0.5, 1.0)),
                    stencil: false,
                }),
                clip_plane,
            });
        }
    }

    fn set_overdraw_measurement(&mut self, enabled: bool) {
        self.assert_open();
        if enabled && self.framebuffer.stencil.is_none() {
            panic!("CPU overdraw measurement requires stencil storage");
        }
        self.stencil_enabled = enabled;
        self.stencil_function = StencilFunction::Always;
        self.stencil_compare_mask = 0xffff_ffff;
        self.stencil_write_mask = 0xffff_ffff;
        self.stencil_depth_fail = StencilOp::Increment;
        self.stencil_depth_pass = StencilOp::Increment;
    }

    fn read_stencil_overdraw(&self, destination: &mut [u8]) {
        self.assert_open();
        let Some(stencil) = &self.framebuffer.stencil else {
            panic!("CPU stencil readback requires stencil storage");
        };
        let stride = self.width.div_ceil(4) as usize * 4;
        if destination.len() < stride * (self.height as usize - 1) + self.width as usize {
            panic!("CPU stencil destination is too small for PACK_ALIGNMENT=4");
        }
        for row in 0..self.height as usize {
            for x in 0..self.width as usize {
                let value = stencil[(self.height as usize - 1 - row) * self.width as usize + x];
                destination[row * stride + x] = (value & 255) as u8;
            }
        }
    }

    fn read_depth_pixel(&self, window_x: i32, window_y: i32) -> f32 {
        self.assert_open();
        if window_x < 0 || window_y < 0 || window_x >= self.width as i32 || window_y >= self.height as i32 {
            panic!("CPU depth readback coordinates must be inside the framebuffer");
        }
        self.framebuffer.depth[(self.height as i32 - 1 - window_y) as usize * self.width as usize + window_x as usize]
    }

    fn begin_view(&mut self, view: &RenderViewState) {
        self.assert_open();
        let viewport = &view.viewport;
        if viewport.x.fract() != 0.0
            || viewport.y.fract() != 0.0
            || viewport.width.fract() != 0.0
            || viewport.height.fract() != 0.0
            || viewport.width <= 0.0
            || viewport.height <= 0.0
        {
            panic!("CPU viewport must have integer coordinates and positive dimensions");
        }
        if let Some(clip) = &view.clip_plane {
            if !finite_vector(clip) {
                panic!("CPU clip plane must be finite");
            }
        }
        self.viewport = *viewport;
        self.clip_plane = view.clip_plane;
        let Some(clear) = &view.clear else {
            return;
        };
        if !clear.depth.is_finite() || clear.color.is_some_and(|color| !finite_vector(&color)) {
            panic!("CPU clear values must be finite");
        }
        self.clear_depth = clamp(clear.depth);
        if let Some(color) = clear.color {
            self.clear_color = color;
            self.clear_color_buffer();
        }
        let (x, y, width, height) = (
            viewport.x as i32,
            viewport.y as i32,
            viewport.width as i32,
            viewport.height as i32,
        );
        for row in y.max(0)..(y + height).min(self.height as i32) {
            let begin = (row * self.width as i32 + x.max(0)) as usize;
            let end = (row * self.width as i32 + (x + width).min(self.width as i32)) as usize;
            if begin >= end {
                continue;
            }
            self.framebuffer.depth[begin..end].fill(self.clear_depth);
            if clear.stencil {
                if let Some(stencil) = self.framebuffer.stencil.as_mut() {
                    stencil[begin..end].fill(0);
                }
            }
        }
    }

    fn with_object_opacity(&mut self, opacity: f32, draw: impl FnOnce(&mut Self)) {
        self.assert_open();
        match self.classify_opacity(opacity) {
            OpacityAction::Skip => {}
            OpacityAction::Direct => {
                self.opacity_active = true;
                draw(self);
                self.opacity_active = false;
            }
            OpacityAction::Scoped => {
                self.enter_opacity_scope();
                let mut guard = OpacityGuard {
                    renderer: self as *mut Self,
                    opacity,
                    dismissed: false,
                };
                draw(self);
                self.exit_opacity_scope(opacity);
                guard.dismissed = true;
            }
        }
    }

    fn draw_immediate(&mut self, operation: &RenderOperation) {
        self.assert_open();
        match operation {
            RenderOperation::Draw(batches) => {
                for batch in batches {
                    self.draw(batch);
                }
            }
            RenderOperation::ObjectOpacity { opacity, batches } => {
                self.with_object_opacity(*opacity, |renderer| {
                    for batch in batches {
                        renderer.draw(batch);
                    }
                });
            }
            RenderOperation::Q2Fog(operation) => {
                let alpha_bits = self.alpha_bits;
                apply_q2_depth_fog(&mut self.framebuffer, operation, alpha_bits);
            }
            RenderOperation::DepthAtlas { image, passes } => {
                let (width, height) = self
                    .images
                    .depth_dimensions(image)
                    .unwrap_or_else(|error| panic!("{error}"));
                // Atlas image row zero is V=0. Raster rows run from the top of the target.
                let mut target =
                    SoftwareRenderer::with_config(width, height, self.owner.clone(), self.subpixel_bits, 0, 8);
                let atlas = self
                    .images
                    .depth_copy_out(image)
                    .unwrap_or_else(|error| panic!("{error}"));
                for (y, row) in atlas.chunks(width as usize).enumerate() {
                    let dest = (height as usize - 1 - y) * width as usize;
                    target.framebuffer.depth[dest..dest + width as usize].copy_from_slice(row);
                }
                for pass in passes {
                    target.begin_view(&RenderViewState {
                        viewport: Rect {
                            x: pass.viewport.x,
                            y: height as f32 - pass.viewport.y - pass.viewport.height,
                            width: pass.viewport.width,
                            height: pass.viewport.height,
                        },
                        clear: pass.clear_depth.map(|depth| ViewClear {
                            depth,
                            color: None,
                            stencil: false,
                        }),
                        clip_plane: None,
                    });
                    for draw in &pass.draws {
                        target.draw(&DrawBatch {
                            fog: None,
                            luminance_alpha: false,
                            indices: draw.indices.clone(),
                            texture: TextureBinding::RetainCurrentTexture,
                            state: RenderState {
                                cull: draw.cull,
                                polygon_offset: draw.polygon_offset,
                                ..RenderState::opaque(CullFace::None)
                            },
                            lighting: BatchLighting::Vertex,
                            primitive: BatchPrimitive::Triangles,
                            vertices: BatchVertices::Single(
                                draw.positions
                                    .iter()
                                    .map(|position| RenderVertex {
                                        position: *position,
                                        tex_coord: vec2(0.0, 0.0),
                                        color: vec4(1.0, 1.0, 1.0, 1.0),
                                    })
                                    .collect(),
                            ),
                        });
                    }
                }
                let mut back = vec![0.0f32; atlas.len()];
                for y in 0..height as usize {
                    let src = (height as usize - 1 - y) * width as usize;
                    back[y * width as usize..(y + 1) * width as usize]
                        .copy_from_slice(&target.framebuffer.depth[src..src + width as usize]);
                }
                target.close();
                self.images
                    .depth_copy_in(image, &back)
                    .unwrap_or_else(|error| panic!("{error}"));
            }
            RenderOperation::DepthRange(range) => {
                self.retained_state.depth_range = *range;
            }
            RenderOperation::Cull(cull) => {
                self.retained_state.cull = *cull;
            }
            RenderOperation::PolygonOffset(value) => {
                self.retained_state.polygon_offset = *value;
            }
            RenderOperation::DisablePortalClip => {
                self.clip_plane = None;
            }
            RenderOperation::SkySide { image, color, strips } => {
                for strip in strips {
                    if strip.len() < 2 || strip.len() % 2 != 0 {
                        panic!("CPU sky strips require paired row vertices");
                    }
                    let mut indices = Vec::new();
                    for index in 0..strip.len() - 2 {
                        indices.push((index + index % 2) as u32);
                        indices.push((index + 1 - index % 2) as u32);
                        indices.push((index + 2) as u32);
                    }
                    let state = self.retained_state;
                    self.draw(&DrawBatch {
                        fog: None,
                        luminance_alpha: false,
                        indices,
                        texture: TextureBinding::BindImage(image.clone()),
                        state,
                        lighting: BatchLighting::Vertex,
                        primitive: BatchPrimitive::Triangles,
                        vertices: BatchVertices::Single(
                            strip
                                .iter()
                                .map(|vertex| RenderVertex {
                                    position: vertex.position,
                                    tex_coord: vertex.tex_coord,
                                    color: *color,
                                })
                                .collect(),
                        ),
                    });
                }
            }
            RenderOperation::ShadowVolume {
                positions,
                indices,
                mirror,
                white_image,
            } => {
                self.shadow_pass(positions, indices, *mirror, white_image, false);
            }
            RenderOperation::ShadowFinish { positions, white_image } => {
                let positions = positions.to_vec();
                self.shadow_pass(&positions, &[0, 1, 2, 0, 2, 3], false, white_image, true);
            }
            RenderOperation::RetainedDraw(draw) => {
                self.draw_retained(draw);
            }
        }
    }

    fn prepare_geometry(&mut self, batch: &DrawBatch) -> Self::Prepared<'_> {
        self.assert_open();
        if !matches!(batch.lighting, BatchLighting::Vertex) {
            let lights = match &batch.lighting {
                BatchLighting::Vertex => 0,
                BatchLighting::Q2World { pass, .. } => match pass {
                    Q2LightPass::Lightmap { lights }
                    | Q2LightPass::Texture { lights }
                    | Q2LightPass::MaterialLightmap { lights } => lights.len(),
                    Q2LightPass::Model { lights, .. } => lights.len(),
                },
                BatchLighting::Q2ModelShadow { lights, .. } => lights.len(),
            };
            if lights > 8 {
                panic!("Q2 fragment lighting accepts at most eight selected lights per draw");
            }
            for index in &batch.indices {
                let (position, normal) = world_attributes(&batch.lighting, *index as usize);
                if ![position.x, position.y, position.z, normal.x, normal.y, normal.z]
                    .iter()
                    .all(|v| v.is_finite())
                {
                    panic!("CPU fragment lighting attributes must be finite");
                }
            }
            let _ = self.fragment_lighting(batch);
        }
        let step = if matches!(batch.primitive, BatchPrimitive::Lines { .. }) {
            2
        } else {
            3
        };
        if !batch.indices.len().is_multiple_of(step) {
            panic!("incomplete CPU primitive indices");
        }
        if let BatchPrimitive::Lines { line_width } = batch.primitive {
            if !line_width.is_finite() || line_width <= 0.0 {
                panic!("CPU line width must be positive");
            }
        }
        let vertex_count = match &batch.vertices {
            BatchVertices::Single(vertices) => vertices.len(),
            BatchVertices::Pair { vertices, .. } => vertices.len(),
        };
        for index in &batch.indices {
            if (*index as usize) >= vertex_count {
                panic!("CPU vertex index is outside its array");
            }
        }
        let finite_vertex = |position: &Vec4, color: &Vec4, tex_coord: &Vec2| {
            finite_vector(position) && finite_vector(color) && tex_coord.x.is_finite() && tex_coord.y.is_finite()
        };
        match &batch.vertices {
            BatchVertices::Single(vertices) => {
                for vertex in vertices {
                    if !finite_vertex(&vertex.position, &vertex.color, &vertex.tex_coord) {
                        panic!("CPU vertex attributes must be finite");
                    }
                }
            }
            BatchVertices::Pair { vertices, .. } => {
                for vertex in vertices {
                    if !finite_vertex(&vertex.base.position, &vertex.base.color, &vertex.base.tex_coord)
                        || !vertex.tex_coord2.x.is_finite()
                        || !vertex.tex_coord2.y.is_finite()
                    {
                        panic!("CPU vertex attributes must be finite");
                    }
                }
            }
        }
        SoftwarePrepared {
            renderer: self,
            batch: batch.clone(),
            phase: PreparedPhase::Prepared,
            next_unit: 0,
            resolved: Vec::new(),
        }
    }

    fn clear_color_buffer(&mut self) {
        self.assert_open();
        if !self.color_write {
            return;
        }
        let viewport = self.viewport;
        let color = self.clear_color;
        let bytes = [
            byte(color.x),
            byte(color.y),
            byte(color.z),
            if self.alpha_bits == 0 { 255 } else { byte(color.w) },
        ];
        let (x, y, width, height) = (
            viewport.x as i32,
            viewport.y as i32,
            viewport.width as i32,
            viewport.height as i32,
        );
        for row in y.max(0)..(y + height).min(self.height as i32) {
            let begin = (row * self.width as i32 + x.max(0)) as usize;
            let end = (row * self.width as i32 + (x + width).min(self.width as i32)) as usize;
            for pixel in begin..end {
                self.framebuffer.store_bytes(pixel, bytes);
            }
        }
    }

    fn draw_show_image(&mut self, image: &RendererImage, rect: &Rect, proportional: bool) {
        let width = rect.width * (if proportional { image.width as f32 / 512.0 } else { 1.0 });
        let height = rect.height * (if proportional { image.height as f32 / 512.0 } else { 1.0 });
        let points = [
            (rect.x, rect.y, 0.0, 0.0),
            (rect.x + width, rect.y, 1.0, 0.0),
            (rect.x + width, rect.y + height, 1.0, 1.0),
            (rect.x, rect.y + height, 0.0, 1.0),
        ];
        let state = self.retained_state;
        let (frame_width, frame_height) = (self.width, self.height);
        self.draw(&DrawBatch {
            fog: None,
            luminance_alpha: false,
            indices: vec![0, 1, 2, 0, 2, 3],
            texture: TextureBinding::BindImage(image.clone()),
            state,
            lighting: BatchLighting::Vertex,
            primitive: BatchPrimitive::Triangles,
            vertices: BatchVertices::Single(
                points
                    .iter()
                    .map(|(x, y, s, t)| RenderVertex {
                        position: vec4(
                            x * 2.0 / frame_width as f32 - 1.0,
                            1.0 - y * 2.0 / frame_height as f32,
                            -1.0,
                            1.0,
                        ),
                        tex_coord: vec2(*s, *t),
                        color: vec4(1.0, 1.0, 1.0, 1.0),
                    })
                    .collect(),
            ),
        });
    }

    fn finish(&mut self) {
        self.assert_open();
        let (Some(table), Some(output)) = (&self.gamma_table, &mut self.output_pixels) else {
            return;
        };
        for (offset, pixel) in self.framebuffer.pixels.iter().enumerate() {
            output[offset] = if offset % 4 == 3 {
                *pixel
            } else {
                table[*pixel as usize]
            };
        }
    }

    fn close(&mut self) {
        if self.closed {
            return;
        }
        self.images.clear();
        self.opacity_framebuffer = None;
        self.closed = true;
    }
}

fn apply_line_fog(
    fog: &super::super::types::BatchFog,
    eye_depth: f32,
    r: &mut f32,
    g: &mut f32,
    b: &mut f32,
    alpha: &mut f32,
) {
    use super::super::types::BatchFog;
    let (amount, effect, color) = match fog {
        BatchFog::Exp2 { color, density, effect } => {
            let d = density * eye_depth / 64.0;
            (1.0 - (-d * d).exp(), *effect, color)
        }
        BatchFog::Constant { color, amount } => (*amount, FogEffect::Color, color),
    };
    if effect != FogEffect::None {
        *r = clamp(*r);
        *g = clamp(*g);
        *b = clamp(*b);
    }
    if effect == FogEffect::Color {
        *r += (color.x - *r) * amount;
        *g += (color.y - *g) * amount;
        *b += (color.z - *b) * amount;
    }
    if effect == FogEffect::Rgb || effect == FogEffect::Rgba {
        *r *= 1.0 - amount;
        *g *= 1.0 - amount;
        *b *= 1.0 - amount;
    }
    if effect == FogEffect::Alpha || effect == FogEffect::Rgba {
        *alpha *= 1.0 - amount;
    }
    if effect == FogEffect::Overlay {
        *r = color.x;
        *g = color.y;
        *b = color.z;
        *alpha *= amount;
    }
}

/// Default CPU backend: a [`SoftwareRenderer`] with headless scene submission.
pub struct CpuRenderer {
    inner: SoftwareRenderer,
    view: Option<RenderView>,
    entities: Vec<SceneEntity>,
    particles: Vec<SceneParticle>,
    decals: Vec<SceneDecal>,
    lights: Vec<SceneLight>,
    frames: u64,
}

impl CpuRenderer {
    /// Create a CPU renderer. Zero dimensions clamp to one pixel.
    #[must_use]
    pub fn new(width: u32, height: u32, session: SessionId) -> Self {
        let owner = ResourceOwner::new(fresh_owner_identity(), session, 0);
        Self {
            inner: SoftwareRenderer::new(width.max(1), height.max(1), owner),
            view: None,
            entities: Vec::new(),
            particles: Vec::new(),
            decals: Vec::new(),
            lights: Vec::new(),
            frames: 0,
        }
    }

    /// Presented pixels.
    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        self.inner.pixels()
    }

    /// Framebuffer width.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.inner.width
    }

    /// Framebuffer height.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.inner.height
    }

    /// Owning renderer lifetime.
    #[must_use]
    pub fn owner(&self) -> &ResourceOwner {
        self.inner.owner()
    }

    /// Completed scene frames.
    #[must_use]
    pub const fn frames(&self) -> u64 {
        self.frames
    }

    /// Last submitted view.
    #[must_use]
    pub const fn view(&self) -> Option<&RenderView> {
        self.view.as_ref()
    }
}

impl RendererBackend for CpuRenderer {
    fn begin_frame(&mut self, view: &RenderView) {
        self.view = Some(*view);
        self.entities.clear();
        self.particles.clear();
        self.decals.clear();
        self.lights.clear();
    }

    fn submit_entities(&mut self, entities: &[SceneEntity]) {
        self.entities.extend_from_slice(entities);
    }

    fn submit_particles(&mut self, particles: &[SceneParticle]) {
        self.particles.extend_from_slice(particles);
    }

    fn submit_decals(&mut self, decals: &[SceneDecal]) {
        self.decals.extend_from_slice(decals);
    }

    fn submit_lights(&mut self, lights: &[SceneLight]) {
        self.lights.extend_from_slice(lights);
    }

    fn end_frame(&mut self) -> FrameStats {
        self.frames += 1;
        FrameStats {
            entities: self.entities.len(),
            particles: self.particles.len(),
            decals: self.decals.len(),
            lights: self.lights.len(),
        }
    }
}

impl OrderedBackend for CpuRenderer {
    type Prepared<'a> = SoftwarePrepared<'a>;

    fn owner(&self) -> &ResourceOwner {
        self.inner.owner()
    }

    fn width(&self) -> u32 {
        self.inner.width()
    }

    fn height(&self) -> u32 {
        self.inner.height()
    }

    fn stencil_bits(&self) -> u32 {
        self.inner.stencil_bits()
    }

    fn apply_image_resource(&mut self, operation: &ImageResourceOperation) {
        self.inner.apply_image_resource(operation);
    }

    fn select_draw_buffer(&mut self, buffer: DrawBuffer, clear: bool) {
        self.inner.select_draw_buffer(buffer, clear);
    }

    fn set_overdraw_measurement(&mut self, enabled: bool) {
        self.inner.set_overdraw_measurement(enabled);
    }

    fn read_stencil_overdraw(&self, destination: &mut [u8]) {
        self.inner.read_stencil_overdraw(destination);
    }

    fn read_depth_pixel(&self, window_x: i32, window_y: i32) -> f32 {
        self.inner.read_depth_pixel(window_x, window_y)
    }

    fn begin_view(&mut self, view: &RenderViewState) {
        self.inner.begin_view(view);
    }

    fn with_object_opacity(&mut self, opacity: f32, draw: impl FnOnce(&mut Self)) {
        self.inner.assert_open();
        match self.inner.classify_opacity(opacity) {
            OpacityAction::Skip => {}
            OpacityAction::Direct => {
                self.inner.opacity_active = true;
                draw(self);
                self.inner.opacity_active = false;
            }
            OpacityAction::Scoped => {
                self.inner.enter_opacity_scope();
                let mut guard = OpacityGuard {
                    renderer: &mut self.inner as *mut SoftwareRenderer,
                    opacity,
                    dismissed: false,
                };
                draw(self);
                self.inner.exit_opacity_scope(opacity);
                guard.dismissed = true;
            }
        }
    }

    fn draw_immediate(&mut self, operation: &RenderOperation) {
        self.inner.draw_immediate(operation);
    }

    fn prepare_geometry(&mut self, batch: &DrawBatch) -> Self::Prepared<'_> {
        self.inner.prepare_geometry(batch)
    }

    fn clear_color_buffer(&mut self) {
        self.inner.clear_color_buffer();
    }

    fn draw_show_image(&mut self, image: &RendererImage, rect: &Rect, proportional: bool) {
        self.inner.draw_show_image(image, rect, proportional);
    }

    fn finish(&mut self) {
        self.inner.finish();
    }

    fn close(&mut self) {
        self.inner.close();
    }
}

#[cfg(test)]
mod tests {
    use qa_core::identity::IdentityOwner;
    use qa_core::math::vec3;

    use super::super::super::types::{ImageSource, TextureFilter, TextureSampling};
    use super::super::super::{ModelPose, RenderView as SceneRenderView};
    use super::*;

    fn owner() -> ResourceOwner {
        let authority = IdentityOwner::create("cpu-rasterizer").unwrap();
        ResourceOwner::new(fresh_owner_identity(), authority.session().clone(), 0)
    }

    fn image(owner: &ResourceOwner, ordinal: u32, width: u32, height: u32) -> RendererImage {
        RendererImage {
            owner: owner.clone(),
            ordinal,
            source: ImageSource::Generated {
                name: format!("test{ordinal}"),
            },
            width,
            height,
        }
    }

    fn batch(vertices: Vec<RenderVertex>) -> DrawBatch {
        DrawBatch {
            fog: None,
            luminance_alpha: false,
            indices: (0..vertices.len() as u32).collect(),
            texture: TextureBinding::RetainCurrentTexture,
            state: RenderState::opaque(CullFace::None),
            lighting: BatchLighting::Vertex,
            primitive: BatchPrimitive::Triangles,
            vertices: BatchVertices::Single(vertices),
        }
    }

    #[test]
    fn clear_color_fills_viewport() {
        let owner = owner();
        let mut renderer = SoftwareRenderer::new(2, 2, owner);
        renderer.begin_view(&RenderViewState {
            viewport: Rect {
                x: 0.0,
                y: 0.0,
                width: 2.0,
                height: 2.0,
            },
            clear: Some(ViewClear {
                depth: 0.25,
                color: Some(vec4(0.0, 1.0, 0.0, 1.0)),
                stencil: true,
            }),
            clip_plane: None,
        });
        assert_eq!(renderer.pixels(), [0u8, 255, 0, 255].repeat(4).as_slice());
        assert_eq!(renderer.read_depth_pixel(0, 0), 0.25);
        // PACK_ALIGNMENT=4 pads each 2-byte row to a 4-byte stride.
        let mut stencil = vec![9u8; 8];
        renderer.read_stencil_overdraw(&mut stencil);
        assert_eq!(stencil, vec![0, 0, 9, 9, 0, 0, 9, 9]);
    }

    #[test]
    fn flat_triangle_draws_exact_pixels() {
        let owner = owner();
        let mut renderer = SoftwareRenderer::new(4, 4, owner);
        renderer.begin_view(&RenderViewState {
            viewport: Rect {
                x: 0.0,
                y: 0.0,
                width: 4.0,
                height: 4.0,
            },
            clear: Some(ViewClear {
                depth: 1.0,
                color: Some(vec4(0.0, 0.0, 0.0, 1.0)),
                stencil: false,
            }),
            clip_plane: None,
        });
        // NDC triangle covering window (0,0), (4,0), (0,4): covered where x + y < 3.
        let red = vec4(1.0, 0.0, 0.0, 1.0);
        renderer.draw(&batch(vec![
            RenderVertex {
                position: vec4(-1.0, 1.0, 0.0, 1.0),
                tex_coord: vec2(0.0, 0.0),
                color: red,
            },
            RenderVertex {
                position: vec4(1.0, 1.0, 0.0, 1.0),
                tex_coord: vec2(1.0, 0.0),
                color: red,
            },
            RenderVertex {
                position: vec4(-1.0, -1.0, 0.0, 1.0),
                tex_coord: vec2(0.0, 1.0),
                color: red,
            },
        ]));
        let mut expected = vec![0u8; 4 * 4 * 4];
        for pixel in expected.chunks_mut(4) {
            pixel.copy_from_slice(&[0, 0, 0, 255]);
        }
        for (x, y) in [(0, 0), (1, 0), (2, 0), (0, 1), (1, 1), (0, 2)] {
            expected[(y * 4 + x) * 4..(y * 4 + x) * 4 + 4].copy_from_slice(&[255, 0, 0, 255]);
        }
        assert_eq!(renderer.pixels(), expected.as_slice());
        assert_eq!(renderer.read_depth_pixel(0, 3), 0.5);
        assert_eq!(renderer.read_depth_pixel(3, 0), 1.0);
    }

    #[test]
    fn stretch_pic_maps_texels_one_to_one() {
        let owner = owner();
        let mut renderer = SoftwareRenderer::new(2, 2, owner.clone());
        let handle = image(&owner, 1, 2, 2);
        renderer.apply_image_resource(&ImageResourceOperation::CreateImage {
            image: handle.clone(),
            content: super::super::super::types::RenderImage::Rgba8 {
                levels: vec![ImageLevel {
                    width: 2,
                    height: 2,
                    pixels: vec![255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255],
                }],
                border_color: vec4(0.0, 0.0, 0.0, 0.0),
            },
            sampling: TextureSampling {
                repeat: true,
                filter: TextureFilter::Nearest,
            },
        });
        renderer.draw_stretch_pic(
            &handle,
            &Rect {
                x: 0.0,
                y: 0.0,
                width: 2.0,
                height: 2.0,
            },
            &TextureRect {
                s1: 0.0,
                t1: 0.0,
                s2: 1.0,
                t2: 1.0,
            },
            &vec4(1.0, 1.0, 1.0, 1.0),
        );
        assert_eq!(
            renderer.pixels(),
            &[255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255]
        );
    }

    #[test]
    fn line_batch_draws_one_row() {
        let owner = owner();
        let mut renderer = SoftwareRenderer::new(4, 4, owner);
        renderer.begin_view(&RenderViewState {
            viewport: Rect {
                x: 0.0,
                y: 0.0,
                width: 4.0,
                height: 4.0,
            },
            clear: Some(ViewClear {
                depth: 1.0,
                color: Some(vec4(0.0, 0.0, 0.0, 1.0)),
                stencil: false,
            }),
            clip_plane: None,
        });
        let white = vec4(1.0, 1.0, 1.0, 1.0);
        renderer.draw(&DrawBatch {
            fog: None,
            luminance_alpha: false,
            indices: vec![0, 1],
            texture: TextureBinding::RetainCurrentTexture,
            state: RenderState::opaque(CullFace::None),
            lighting: BatchLighting::Vertex,
            primitive: BatchPrimitive::Lines { line_width: 1.0 },
            vertices: BatchVertices::Single(vec![
                RenderVertex {
                    position: vec4(-1.0, 0.0, 0.0, 1.0),
                    tex_coord: vec2(0.0, 0.0),
                    color: white,
                },
                RenderVertex {
                    position: vec4(1.0, 0.0, 0.0, 1.0),
                    tex_coord: vec2(1.0, 0.0),
                    color: white,
                },
            ]),
        });
        let mut expected = vec![0u8; 4 * 4 * 4];
        for pixel in expected.chunks_mut(4) {
            pixel.copy_from_slice(&[0, 0, 0, 255]);
        }
        for x in 0..4 {
            expected[(2 * 4 + x) * 4..(2 * 4 + x) * 4 + 4].copy_from_slice(&[255, 255, 255, 255]);
        }
        assert_eq!(renderer.pixels(), expected.as_slice());
    }

    #[test]
    fn depth_atlas_renders_and_writes_back() {
        let owner = owner();
        let mut renderer = SoftwareRenderer::new(2, 2, owner.clone());
        let handle = image(&owner, 2, 2, 2);
        renderer.apply_image_resource(&ImageResourceOperation::CreateImage {
            image: handle.clone(),
            content: super::super::super::types::RenderImage::Depth32f {
                levels: vec![super::super::super::types::DepthImageLevel {
                    width: 2,
                    height: 2,
                    pixels: vec![1.0; 4],
                }],
            },
            sampling: TextureSampling {
                repeat: true,
                filter: TextureFilter::Nearest,
            },
        });
        renderer.draw_immediate(&RenderOperation::DepthAtlas {
            image: handle.clone(),
            passes: vec![super::super::super::types::DepthAtlasPass {
                viewport: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 2.0,
                    height: 2.0,
                },
                clear_depth: Some(1.0),
                draws: vec![super::super::super::types::DepthAtlasDraw {
                    positions: vec![
                        vec4(-1.0, 1.0, 0.0, 1.0),
                        vec4(1.0, 1.0, 0.0, 1.0),
                        vec4(-1.0, -1.0, 0.0, 1.0),
                    ],
                    indices: vec![0, 1, 2],
                    cull: CullFace::None,
                    polygon_offset: None,
                }],
            }],
        });
        let depth = renderer.images.depth_copy_out(&handle).unwrap();
        // Only raster pixel (0,0) is strictly inside x + y < 2; atlas row 0 is V=0.
        assert_eq!(depth, vec![1.0, 1.0, 0.5, 1.0]);
    }

    #[test]
    fn cpu_renderer_clamps_size_and_counts_submissions() {
        use crate::view::{CameraClip, ModelTransform, Rect as ViewRect, SceneCamera};
        let authority = IdentityOwner::create("cpu-renderer").unwrap();
        let mut renderer = CpuRenderer::new(0, 0, authority.session().clone());
        assert_eq!((renderer.width(), renderer.height()), (1, 1));
        assert_eq!(renderer.pixels().len(), 4);
        let axis = [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)];
        let view = SceneRenderView {
            camera: SceneCamera {
                origin: vec3(0.0, 0.0, 0.0),
                axis,
                projection: [0.0; 16],
                viewport: ViewRect {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1,
                },
                clip: CameraClip::None,
            },
            time_ms: 0,
            flags: 0,
        };
        renderer.begin_frame(&view);
        renderer.submit_entities(&[SceneEntity {
            entity_number: 1,
            model: 2,
            transform: ModelTransform {
                origin: vec3(0.0, 0.0, 0.0),
                axis,
                scale: 1.0,
            },
            previous_origin: vec3(0.0, 0.0, 0.0),
            pose: ModelPose {
                frame: 0,
                old_frame: 0,
                back_lerp: 0.0,
            },
            skin: 0,
            color: vec4(1.0, 1.0, 1.0, 1.0),
            flags: 0,
            lighting_origin: None,
            shadow_plane: None,
            opacity: None,
        }]);
        renderer.submit_particles(&[]);
        renderer.submit_decals(&[]);
        renderer.submit_lights(&[]);
        let stats = renderer.end_frame();
        assert_eq!(
            (stats.entities, stats.particles, stats.decals, stats.lights),
            (1, 0, 0, 0)
        );
        assert_eq!(renderer.frames(), 1);
        assert_eq!(renderer.view(), Some(&view));
    }

    #[test]
    fn batched_parallel_shading_matches_serial_draws() {
        fn view() -> RenderViewState {
            RenderViewState {
                viewport: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 8.0,
                    height: 8.0,
                },
                clear: Some(ViewClear {
                    depth: 1.0,
                    color: Some(vec4(0.0, 0.0, 0.0, 1.0)),
                    stencil: false,
                }),
                clip_plane: None,
            }
        }
        fn tri(color: Vec4, z: f32) -> DrawBatch {
            // NDC triangle covering window (0,0), (8,0), (0,8).
            batch(vec![
                RenderVertex {
                    position: vec4(-1.0, 1.0, z, 1.0),
                    tex_coord: vec2(0.0, 0.0),
                    color,
                },
                RenderVertex {
                    position: vec4(1.0, 1.0, z, 1.0),
                    tex_coord: vec2(1.0, 0.0),
                    color,
                },
                RenderVertex {
                    position: vec4(-1.0, -1.0, z, 1.0),
                    tex_coord: vec2(0.0, 1.0),
                    color,
                },
            ])
        }
        // Overlapping depths: nearest (blue) must win everywhere covered.
        let batches = vec![
            tri(vec4(1.0, 0.0, 0.0, 1.0), 0.5),
            tri(vec4(0.0, 1.0, 0.0, 1.0), 0.0),
            tri(vec4(0.0, 0.0, 1.0, 1.0), -0.5),
        ];
        let owner = owner();
        let mut serial = SoftwareRenderer::new(8, 8, owner.clone());
        serial.begin_view(&view());
        for batch in &batches {
            serial.draw(batch);
        }
        let expected_pixels = serial.pixels().to_vec();
        let expected_depth = serial.framebuffer.depth.clone();
        assert!(expected_pixels.as_chunks::<4>().0.iter().any(|pixel| pixel[2] == 255));

        let refs: Vec<&DrawBatch> = batches.iter().collect();
        let mut timer = StageTimer::new(false);
        let mut batched_serial = SoftwareRenderer::new(8, 8, owner.clone());
        batched_serial.begin_view(&view());
        batched_serial.parallel_min_setups = usize::MAX;
        batched_serial.draw_batches(&refs, &mut timer);
        assert_eq!(batched_serial.pixels(), expected_pixels.as_slice());
        assert_eq!(batched_serial.framebuffer.depth, expected_depth);

        let mut parallel = SoftwareRenderer::new(8, 8, owner);
        parallel.begin_view(&view());
        parallel.parallel_min_setups = 1;
        parallel.parallel_threads = 4;
        parallel.draw_batches(&refs, &mut timer);
        assert_eq!(parallel.pixels(), expected_pixels.as_slice());
        assert_eq!(parallel.framebuffer.depth, expected_depth);
    }
}
