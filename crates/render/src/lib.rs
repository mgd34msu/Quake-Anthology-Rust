//! One scene interface and owned frame packets for every client module.
mod affine;
pub mod assets;
pub mod cpu;
pub mod edges;
pub mod effects;
pub mod entities;
pub mod gl;
pub mod lighting;
pub mod lightmap;
pub mod material;
pub mod particles;
pub mod scene;
pub mod shader;
pub mod sky;
pub mod stage;
pub mod surface_cache;
pub mod world;
pub use assets::{Assets, ImageId, MaterialId, ModelId, PaletteId, Vertex};
pub use scene::{
    BlendPhase, Command, CommandList, CpuPresentation, Draw2d, DrawItem, DrawKind, Frame, FrontEnd,
    Light, LightStyle, Limits, PaletteOperation, PaletteShift, PaletteTransform,
    ParticlePresentation, PerspectiveStep, Refdef, SceneEntity, SceneModel, SceneParticle,
    Viewport,
};
pub use world::WorldPresentationId;

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

fn next_owner_id() -> Result<u64, &'static str> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
        .map_err(|_| "render owner ids exhausted")
}
