//! Checked projection of the shared renderer configuration into native ABIs.
use crate::services::{CallError, RendererRegistration};
use qa_core::presentation::{
    RENDERER_EXTENSIONS_BYTES, RENDERER_TEXT_BYTES, RendererConfiguration,
};
use qa_core::primitives::{RuleSetId, Vec3};
use qa_render::{
    Vertex,
    scene::{EntityVisibility, Light, Refdef, SceneEntity},
    world::WorldId,
};

/// The caller owns its renderer role; each invocation borrows the session's
/// one scene owner. Guest addresses never cross this interface.
pub trait SceneService {
    fn registration_available(&self) -> bool {
        false
    }
    fn registration_cached(
        &self,
        kind: RendererRegistration,
        name: &[u8],
        source: RuleSetId,
    ) -> Option<i32> {
        let _ = (kind, name, source);
        None
    }
    fn active(&self) -> bool;
    fn viewport(&self) -> Option<qa_render::Viewport>;
    fn select_world(&mut self, world: WorldId) -> Result<(), CallError>;
    fn selected_world(&self) -> Option<WorldId>;
    fn clear(&mut self);
    fn light(&mut self, light: Light) -> bool;
    fn entity(&mut self, entity: EntitySubmission) -> bool;
    fn poly(&mut self, shader: i32, count: usize, vertex: &mut dyn FnMut(usize) -> Vertex) -> bool;
    fn color(&mut self, color: Option<[f32; 4]>);
    fn picture(&mut self, shader: i32, rect: [f32; 4], texcoords: [f32; 4]) -> bool;
    fn view(&mut self, view: SceneView) -> Result<bool, CallError>;
}

pub const Q3_REFDEF_BYTES: usize = 368;
pub const Q3_POLY_VERTEX_BYTES: usize = 24;
pub const Q3_REF_ENTITY_BYTES: usize = 140;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityKind {
    Model,
    PolyMarker,
    Sprite,
    Beam,
    RailCore,
    RailRings,
    Lightning,
    Portal,
}
#[derive(Clone, Copy, Debug)]
pub struct EntitySubmission {
    pub kind: EntityKind,
    pub pose: SceneEntity,
    pub model: i32,
    pub skin: i32,
    pub custom_skin: i32,
    pub custom_shader: i32,
    pub frame: i32,
    pub old_frame: i32,
    pub old_origin: Vec3,
    pub radius: f32,
    pub rotation: f32,
    pub wrap_frames: bool,
}
struct Record<'a>(&'a [u8]);
impl Record<'_> {
    fn word(&self, at: usize) -> i32 {
        i32::from_le_bytes([self.0[at], self.0[at + 1], self.0[at + 2], self.0[at + 3]])
    }
    fn scalar(&self, at: usize) -> f32 {
        f32::from_bits(self.word(at) as u32)
    }
    fn vector(&self, at: usize) -> Vec3 {
        Vec3(std::array::from_fn(|axis| self.scalar(at + axis * 4)))
    }
}
pub fn read_q3_entity(bytes: &[u8]) -> Result<EntitySubmission, CallError> {
    if bytes.len() != Q3_REF_ENTITY_BYTES {
        return Err(CallError::Memory);
    }
    let r = Record(bytes);
    let kind = match r.word(0) {
        0 => EntityKind::Model,
        1 => EntityKind::PolyMarker,
        2 => EntityKind::Sprite,
        3 => EntityKind::Beam,
        4 => EntityKind::RailCore,
        5 => EntityKind::RailRings,
        6 => EntityKind::Lightning,
        7 => EntityKind::Portal,
        _ => return Err(CallError::Geometry),
    };
    let flags = r.word(4);
    let visibility = match (flags & 4 != 0, flags & 2 != 0) {
        (false, false) => EntityVisibility::Always,
        (true, false) => EntityVisibility::Direct,
        (false, true) => EntityVisibility::Portal,
        (true, true) => EntityVisibility::Hidden,
    };
    Ok(EntitySubmission {
        kind,
        model: r.word(8),
        frame: r.word(80),
        old_frame: r.word(96),
        old_origin: r.vector(84),
        skin: r.word(104),
        custom_skin: r.word(108),
        custom_shader: r.word(112),
        radius: r.scalar(132),
        rotation: r.scalar(136),
        wrap_frames: flags & 512 != 0,
        pose: SceneEntity {
            origin: r.vector(68),
            axes: std::array::from_fn(|axis| r.vector(28 + axis * 12)),
            non_normalized_axes: r.word(64) != 0,
            color: [bytes[116], bytes[117], bytes[118], bytes[119]],
            back_lerp: r.scalar(100),
            depth_hack: flags & 8 != 0,
            shader_texcoord: [r.scalar(120), r.scalar(124)],
            shader_time: r.scalar(128),
            lighting_origin: (flags & 128 != 0).then(|| r.vector(12)),
            shadow_plane: (flags & 256 != 0).then(|| r.scalar(24)),
            minimum_light: flags & 1 != 0,
            casts_shadow: flags & 64 == 0,
            visibility,
            ..SceneEntity::default()
        },
    })
}

#[derive(Clone, Copy, Debug)]
pub struct SceneView {
    pub refdef: Refdef,
    pub hidden_areas: [u8; 32],
    pub world: bool,
}

/// Both module widths and QVM use the same pointer-free Q3 record. The
/// complete source is checked before decoding; the result owns every byte.
pub fn read_q3_view(bytes: &[u8]) -> Result<SceneView, CallError> {
    if bytes.len() != Q3_REFDEF_BYTES {
        return Err(CallError::Memory);
    }
    let r = Record(bytes);
    let word = |at| r.word(at);
    let scalar = |at| r.scalar(at);
    let vector = |at| r.vector(at);
    let mut refdef = Refdef {
        viewport: qa_render::Viewport {
            x: u32::try_from(word(0)).map_err(|_| CallError::Geometry)?,
            y: u32::try_from(word(4)).map_err(|_| CallError::Geometry)?,
            width: u32::try_from(word(8)).map_err(|_| CallError::Geometry)?,
            height: u32::try_from(word(12)).map_err(|_| CallError::Geometry)?,
        },
        fov: [scalar(16), scalar(20)],
        origin: vector(24),
        axes: std::array::from_fn(|axis| vector(36 + axis * 12)),
        time_ms: i64::from(word(72)),
        hyperspace: word(76) & 4 != 0,
        ..Refdef::default()
    };
    let mut hidden_areas = [0; 32];
    hidden_areas.copy_from_slice(&bytes[80..112]);
    for (target, source) in refdef.text.iter_mut().zip(bytes[112..].chunks_exact(32)) {
        if !source.contains(&0) {
            return Err(CallError::Text);
        }
        target.copy_from_slice(source);
    }
    Ok(SceneView {
        refdef,
        hidden_areas,
        world: word(76) & 1 == 0,
    })
}

/// Only called on an already-checked 24-byte polyVert_t extent.
pub fn q3_vertex(bytes: &[u8]) -> Vertex {
    let r = Record(bytes);
    Vertex {
        position: r.vector(0),
        texcoord: [r.scalar(12), r.scalar(16)],
        color: [bytes[20], bytes[21], bytes[22], bytes[23]],
        ..Vertex::default()
    }
}

pub const Q3_GL_CONFIG_BYTES: usize = 3 * RENDERER_TEXT_BYTES + RENDERER_EXTENSIONS_BYTES + 17 * 4;

/// Q3 tr_types.h glconfig_t uses identical four-byte cells in QVM and native
/// modules of either pointer width. Check the whole destination before writing.
pub fn write_q3(configuration: &RendererConfiguration, output: &mut [u8]) -> Result<(), CallError> {
    if output.len() != Q3_GL_CONFIG_BYTES {
        return Err(CallError::Memory);
    }
    let display = configuration.display;
    if display.width == 0 || display.height == 0 {
        return Err(CallError::Geometry);
    }
    output.fill(0);
    for (first, text) in [
        (0, configuration.renderer.as_bytes()),
        (RENDERER_TEXT_BYTES, configuration.vendor.as_bytes()),
        (2 * RENDERER_TEXT_BYTES, configuration.version.as_bytes()),
        (3 * RENDERER_TEXT_BYTES, configuration.extensions.as_bytes()),
    ] {
        output[first..first + text.len()].copy_from_slice(text);
    }
    let values = [
        configuration.max_texture_size,
        configuration.max_active_textures,
        display.color_bits,
        display.depth_bits,
        display.stencil_bits,
        0, // Modern integrated-window driver, GLDRV_ICD.
        0, // Generic hardware: no legacy 3dfx-specific restrictions.
        u32::from(configuration.hardware_gamma),
        u32::from(configuration.compressed_textures),
        u32::from(configuration.texture_env_add),
        display.width,
        display.height,
        (display.width as f32 / display.height as f32).to_bits(),
        display.frequency as u32,
        u32::from(display.fullscreen),
        u32::from(display.stereo),
        u32::from(configuration.render_worker),
    ];
    let first = 3 * RENDERER_TEXT_BYTES + RENDERER_EXTENSIONS_BYTES;
    for (cell, value) in output[first..].chunks_exact_mut(4).zip(values) {
        cell.copy_from_slice(&value.to_le_bytes());
    }
    Ok(())
}
