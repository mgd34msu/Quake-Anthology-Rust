//! Software rasterizer backend (donor `src/render/cpu/*`).

pub mod commands;
pub mod fog;
pub mod lighting;
pub mod lines;
pub mod rasterizer;
pub mod source;
pub mod textures;
pub mod triangle_kernel;

pub use rasterizer::CpuRenderer;
