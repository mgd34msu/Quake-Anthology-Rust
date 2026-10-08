//! One scene interface and owned frame packets for every client module.
pub mod assets;
pub mod cpu;
pub mod edges;
pub mod gl;
pub mod lightmap;
pub mod material;
pub mod scene;
pub mod shader;
pub mod surface_cache;
pub mod world;
pub use assets::{Assets, ImageId, MaterialId, ModelId, PaletteId, Vertex};
pub use scene::{
    BlendPhase, Command, CommandList, CpuPresentation, Draw2d, Frame, FrontEnd, Light, LightStyle,
    Limits, PerspectiveStep, Refdef, SceneEntity, Viewport,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BackendStats {
    pub views: u32,
    pub triangles: u32,
    pub surfaces: u32,
    pub stages: u32,
    pub draws_2d: u32,
    pub rejected: u32,
    /// Scene lighting awaits THE-862 world/material preparation.
    pub pending_lights: u32,
}
