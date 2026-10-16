//! Load-time renderer capabilities shared by module and presentation boundaries.
use crate::text::FixedText;

pub const RENDERER_TEXT_BYTES: usize = 1024;
pub const RENDERER_EXTENSIONS_BYTES: usize = 8192;
pub const MAX_WINDOW_DIMENSION: u32 = 8192;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DisplayMode {
    pub width: u32,
    pub height: u32,
    pub frequency: i32,
    /// Native display storage depth, including unused packed pixel bits.
    /// GL framebuffer RGB precision belongs to DisplayConfiguration instead.
    pub color_bits: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VideoCapabilities {
    pub desktop: DisplayMode,
    pub fullscreen: Box<[DisplayMode]>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WindowMode {
    #[default]
    Windowed,
    Desktop,
    Exclusive {
        frequency: i32,
        color_bits: u32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GlContextRequest {
    pub color_bits: u32,
    pub depth_bits: u32,
    pub stencil_bits: u32,
    pub stereo: bool,
    pub swap_interval: i32,
}
impl Default for GlContextRequest {
    fn default() -> Self {
        Self {
            color_bits: 24,
            depth_bits: 24,
            stencil_bits: 0,
            stereo: false,
            swap_interval: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowRequest {
    /// Fixed software framebuffer and ordinary window dimensions. Desktop
    /// fullscreen changes the display extent without changing this selection.
    pub width: u32,
    pub height: u32,
    pub mode: WindowMode,
    pub gl: GlContextRequest,
}
impl WindowRequest {
    pub fn windowed(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            mode: WindowMode::Windowed,
            gl: GlContextRequest::default(),
        }
    }
    pub fn validate(&self, gl: bool) -> Result<(), &'static str> {
        if !(1..=MAX_WINDOW_DIMENSION).contains(&self.width)
            || !(1..=MAX_WINDOW_DIMENSION).contains(&self.height)
        {
            return Err("window dimensions must be between 1 and 8192");
        }
        if let WindowMode::Exclusive {
            frequency,
            color_bits,
        } = self.mode
        {
            if frequency < 0 {
                return Err("invalid fullscreen frequency");
            }
            if color_bits > i32::MAX as u32 {
                return Err("invalid fullscreen color depth");
            }
        }
        if gl
            && [self.gl.color_bits, self.gl.depth_bits, self.gl.stencil_bits]
                .iter()
                .any(|&bits| bits > i32::MAX as u32)
        {
            return Err("invalid GL context precision");
        }
        Ok(())
    }
}
impl Default for WindowRequest {
    fn default() -> Self {
        Self::windowed(640, 480)
    }
}

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
