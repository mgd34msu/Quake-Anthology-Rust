//! Model preparation and rendering (donor `src/render/scene/models/*`).

pub mod attachment;
pub mod grip;
pub mod image_path;
pub mod light_sampler;
pub mod lighting;
pub mod md3_bounds;
pub mod prepare;
pub mod renderer;
pub mod replacements;
pub mod shadedots;
pub mod shadow_bounds;
pub mod sprites;
pub mod transform;
pub mod types;

pub use types::{
    EntityFlags, EntityTransform, ModelAttachment, ModelCull, ModelDrawGroup, ModelGroupContext, ModelGroupOrder,
    ModelImageSelection, ModelPreparationContext, ModelPreparationHooks, ModelResource, ModelSkinningFrame,
    ModelSourceOptions, ModelVertexLighting, PreparedModelEntity, PreparedModelSurface, SceneEntity, SceneModel,
    ScenePose,
};
