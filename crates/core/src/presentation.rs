//! Load-time renderer capabilities shared by module and presentation boundaries.
use crate::text::FixedText;

pub const RENDERER_TEXT_BYTES: usize = 1024;
pub const RENDERER_EXTENSIONS_BYTES: usize = 8192;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DisplayConfiguration {
    pub width: u32,
    pub height: u32,
    pub frequency: i32,
    pub fullscreen: bool,
    pub color_bits: u32,
    pub depth_bits: u32,
    pub stencil_bits: u32,
    pub stereo: bool,
}

/// Backend capabilities, captured once before client-module registration.
/// Extension strings describe the actual context; feature flags describe what
/// the selected engine backend exposes, rather than every driver extension.
#[derive(Clone, Debug, Default)]
pub struct RendererConfiguration {
    pub renderer: FixedText<{ RENDERER_TEXT_BYTES - 1 }>,
    pub vendor: FixedText<{ RENDERER_TEXT_BYTES - 1 }>,
    pub version: FixedText<{ RENDERER_TEXT_BYTES - 1 }>,
    pub extensions: FixedText<{ RENDERER_EXTENSIONS_BYTES - 1 }>,
    pub display: DisplayConfiguration,
    pub max_texture_size: u32,
    pub max_active_textures: u32,
    pub hardware_gamma: bool,
    pub compressed_textures: bool,
    pub texture_env_add: bool,
    pub render_worker: bool,
}
