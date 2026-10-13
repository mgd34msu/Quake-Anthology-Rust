//! Checked projection of the shared renderer configuration into native ABIs.
use crate::services::CallError;
use qa_core::presentation::{
    RENDERER_EXTENSIONS_BYTES, RENDERER_TEXT_BYTES, RendererConfiguration,
};

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
