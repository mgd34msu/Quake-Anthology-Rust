//! Particle systems (donor `src/render/scene/particles/*`).

pub mod legacy;
pub mod primitives;
pub mod q3_system;
pub mod q3_types;

pub use legacy::{IndexedProfile, Q1ParticleState, Q1ParticleType, Q2ParticleState, SceneParticle};
pub use q3_types::{ParticleClientEntity, ParticleClientState, ParticleShader, RefPoly, RefPolyVertex};
