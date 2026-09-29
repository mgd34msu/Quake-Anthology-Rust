//! Dynamic texture resolution at draw time.
//!
//! Donor provenance: `src/render/commands/dynamic-texture.ts`
//! (`createTextureResolver`, `resolveDrawTextures`). Each dynamic source
//! resolves once per draw: uploads run through `apply` first, then the
//! returned image binds. Sources compare by [`Arc`] pointer identity, so two
//! handles to one source share a single resolution.

use std::collections::HashMap;
use std::sync::Arc;

use super::types::{BatchVertices, DrawBatch, ImageResourceOperation, RendererImage, TextureBinding};

/// Per-draw cache from dynamic source identity to resolved image.
#[derive(Debug, Default)]
pub struct TextureResolver {
    resolved: HashMap<*const (), RendererImage>,
}

impl TextureResolver {
    /// Empty resolver.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolve one binding, running uploads through `apply` on first sight
    /// of each source. Non-dynamic bindings pass through untouched.
    pub fn resolve(
        &mut self,
        binding: &TextureBinding,
        apply: &mut dyn FnMut(ImageResourceOperation),
    ) -> TextureBinding {
        let TextureBinding::DynamicImage(source) = binding else {
            return binding.clone();
        };
        let key = Arc::as_ptr(source) as *const ();
        if let Some(image) = self.resolved.get(&key) {
            return TextureBinding::BindImage(image.clone());
        }
        let image = source.resolve(apply);
        self.resolved.insert(key, image.clone());
        TextureBinding::BindImage(image)
    }
}

/// Build a fresh per-draw resolver.
#[must_use]
pub fn create_texture_resolver() -> TextureResolver {
    TextureResolver::new()
}

/// Resolve a batch's primary (and paired secondary) texture bindings.
#[must_use]
pub fn resolve_draw_textures(batch: &DrawBatch, apply: &mut dyn FnMut(ImageResourceOperation)) -> DrawBatch {
    let mut resolver = create_texture_resolver();
    let mut resolved = batch.clone();
    resolved.texture = resolver.resolve(&batch.texture, apply);
    if let BatchVertices::Pair { second_texture, .. } = &batch.vertices {
        let binding = resolver.resolve(&second_texture.binding, apply);
        if let BatchVertices::Pair { second_texture, .. } = &mut resolved.vertices {
            second_texture.binding = binding;
        }
    }
    resolved
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use qa_core::identity::IdentityOwner;
    use qa_core::math::{vec2, vec4};

    use super::super::types::{
        BatchLighting, BatchPrimitive, DynamicImageSource, ImageSource, MultitextureVertex, PairEnvironment,
        RenderState, RenderVertex, ResourceOwner, TextureBundle,
    };
    use super::*;

    fn owner() -> ResourceOwner {
        let authority = IdentityOwner::create("dynamic-texture-test").unwrap();
        ResourceOwner::new(1, authority.session().clone(), 0)
    }

    fn image(owner: &ResourceOwner, ordinal: u32) -> RendererImage {
        RendererImage {
            owner: owner.clone(),
            ordinal,
            source: ImageSource::Generated {
                name: "dyn".to_string(),
            },
            width: 1,
            height: 1,
        }
    }

    fn vertex() -> RenderVertex {
        RenderVertex {
            position: vec4(0.0, 0.0, 0.0, 1.0),
            tex_coord: vec2(0.0, 0.0),
            color: vec4(1.0, 1.0, 1.0, 1.0),
        }
    }

    fn batch(texture: TextureBinding) -> DrawBatch {
        DrawBatch {
            fog: None,
            luminance_alpha: false,
            indices: vec![0],
            texture,
            state: RenderState::opaque(super::super::types::CullFace::None),
            lighting: BatchLighting::Vertex,
            primitive: BatchPrimitive::Triangles,
            vertices: BatchVertices::Single(vec![vertex()]),
        }
    }

    struct CountingSource {
        image: RendererImage,
        calls: Mutex<u32>,
    }

    impl DynamicImageSource for CountingSource {
        fn resolve(&self, apply: &mut dyn FnMut(ImageResourceOperation)) -> RendererImage {
            *self.calls.lock().unwrap() += 1;
            apply(ImageResourceOperation::TextureMode {
                filter: super::super::types::TextureFilter::Linear,
            });
            self.image.clone()
        }
    }

    #[test]
    fn static_bindings_pass_through_without_apply() {
        let owner = owner();
        let mut resolver = create_texture_resolver();
        let mut applied = Vec::new();
        let mut apply = |operation: ImageResourceOperation| applied.push(operation);
        let binding = TextureBinding::BindImage(image(&owner, 0));
        assert_eq!(resolver.resolve(&binding, &mut apply), binding);
        assert_eq!(
            resolver.resolve(&TextureBinding::RetainCurrentTexture, &mut apply),
            TextureBinding::RetainCurrentTexture
        );
        assert!(applied.is_empty());
    }

    #[test]
    fn same_source_resolves_once_per_resolver() {
        let owner = owner();
        let source = Arc::new(CountingSource {
            image: image(&owner, 4),
            calls: Mutex::new(0),
        });
        let binding = TextureBinding::DynamicImage(source.clone());
        let mut resolver = create_texture_resolver();
        let mut applied = 0;
        let first = resolver.resolve(&binding, &mut |_| applied += 1);
        let second = resolver.resolve(&binding, &mut |_| applied += 1);
        assert_eq!(first, TextureBinding::BindImage(image(&owner, 4)));
        assert_eq!(second, first);
        assert_eq!(*source.calls.lock().unwrap(), 1);
        assert_eq!(applied, 1);
    }

    #[test]
    fn cloned_handles_share_one_resolution() {
        let owner = owner();
        let source = Arc::new(CountingSource {
            image: image(&owner, 1),
            calls: Mutex::new(0),
        });
        let mut resolver = create_texture_resolver();
        let mut applied = 0;
        let mut apply = |_: ImageResourceOperation| applied += 1;
        resolver.resolve(&TextureBinding::DynamicImage(source.clone()), &mut apply);
        resolver.resolve(&TextureBinding::DynamicImage(source.clone()), &mut apply);
        assert_eq!(*source.calls.lock().unwrap(), 1);
        assert_eq!(applied, 1);
    }

    #[test]
    fn distinct_sources_resolve_independently() {
        let owner = owner();
        let first = Arc::new(CountingSource {
            image: image(&owner, 1),
            calls: Mutex::new(0),
        });
        let second = Arc::new(CountingSource {
            image: image(&owner, 2),
            calls: Mutex::new(0),
        });
        let mut resolver = create_texture_resolver();
        let mut applied = 0;
        let mut apply = |_: ImageResourceOperation| applied += 1;
        let a = resolver.resolve(&TextureBinding::DynamicImage(first.clone()), &mut apply);
        let b = resolver.resolve(&TextureBinding::DynamicImage(second.clone()), &mut apply);
        assert_eq!(a, TextureBinding::BindImage(image(&owner, 1)));
        assert_eq!(b, TextureBinding::BindImage(image(&owner, 2)));
        assert_eq!(applied, 2);
    }

    #[test]
    fn draw_resolution_covers_both_pair_bindings() {
        let owner = owner();
        let source = Arc::new(CountingSource {
            image: image(&owner, 7),
            calls: Mutex::new(0),
        });
        let dynamic = TextureBinding::DynamicImage(source.clone());
        let mut input = batch(dynamic.clone());
        input.vertices = BatchVertices::Pair {
            vertices: vec![MultitextureVertex {
                base: vertex(),
                tex_coord2: vec2(1.0, 1.0),
            }],
            second_texture: TextureBundle {
                binding: dynamic,
                environment: PairEnvironment::Modulate,
            },
        };
        let mut applied = 0;
        let resolved = resolve_draw_textures(&input, &mut |_: ImageResourceOperation| applied += 1);
        assert_eq!(resolved.texture, TextureBinding::BindImage(image(&owner, 7)));
        let BatchVertices::Pair { second_texture, .. } = &resolved.vertices else {
            panic!("expected pair vertices");
        };
        assert_eq!(second_texture.binding, TextureBinding::BindImage(image(&owner, 7)));
        assert_eq!(second_texture.environment, PairEnvironment::Modulate);
        assert_eq!(*source.calls.lock().unwrap(), 1, "one resolution per draw");
        assert_eq!(applied, 1);
        assert_eq!(resolved.indices, input.indices);
    }
}
