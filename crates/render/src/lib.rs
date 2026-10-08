//! One scene interface and owned frame packets for every client module.
pub mod assets;
pub mod cpu;
pub mod gl;
pub mod material;
pub mod shader;
pub mod scene;
pub use assets::{Assets, ImageId, MaterialId, ModelId, Vertex};
pub use scene::{
    BlendPhase, Command, CommandList, Draw2d, Frame, FrontEnd, Light, Limits, Refdef, SceneEntity,
    Viewport,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BackendStats {
    pub views: u32,
    pub triangles: u32,
    pub draws_2d: u32,
    pub rejected: u32,
    /// Scene lighting awaits THE-862 world/material preparation.
    pub pending_lights: u32,
}
