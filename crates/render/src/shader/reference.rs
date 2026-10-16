//! Owned shader selections from frontend submission through backend handoff.
use super::{
    context::ShaderNodeId,
    registration::{ModelProfile, ShaderOwner, ShaderResource},
};
use crate::{Assets, ShaderBindingId, ShaderRegistration, assets::MaterialSettings};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShaderContextId {
    pub slot: u32,
    pub generation: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeShaderRef {
    pub context: ShaderContextId,
    pub node: ShaderNodeId,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShaderOccurrence(pub u32);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShaderChoice {
    Direct(ShaderBindingId),
    Occurrence(ShaderOccurrence),
    /// Generated surface rules retain their compiled body while participating
    /// in the same named history used by explicit native replacements.
    Converted {
        binding: ShaderBindingId,
        occurrence: ShaderOccurrence,
    },
}

impl Default for ShaderChoice {
    fn default() -> Self {
        Self::Direct(ShaderBindingId(0))
    }
}

#[derive(Clone, Copy)]
pub enum ShaderDefault {
    Named,
    Global(Option<ShaderBindingId>),
}
#[derive(Clone, Copy)]
pub struct ShaderView<'a> {
    pub native: &'a dyn NativeShaders,
    pub context: Option<ShaderContextId>,
}
impl ShaderView<'static> {
    pub const fn direct() -> Self {
        Self {
            native: &(),
            context: None,
        }
    }
}
impl ShaderView<'_> {
    pub fn graphic(
        self,
        registration: &ShaderRegistration,
        choice: ShaderChoice,
        default: ShaderDefault,
    ) -> Option<ShaderRef> {
        let nodes = self.context.and_then(|context| {
            self.native
                .resource_nodes(context, ShaderResource::Graphic(registration.key))
        });
        self.choice(
            choice,
            std::slice::from_ref(registration),
            nodes,
            default,
        )
    }
    pub fn choice(
        self,
        choice: ShaderChoice,
        registrations: &[ShaderRegistration],
        nodes: Option<&[ShaderNodeId]>,
        default: ShaderDefault,
    ) -> Option<ShaderRef> {
        let (occurrence, converted) = match choice {
            ShaderChoice::Direct(binding) => return Some(ShaderRef::Direct(binding)),
            ShaderChoice::Occurrence(occurrence) => (occurrence, None),
            ShaderChoice::Converted {
                binding,
                occurrence,
            } => (occurrence, Some(binding)),
        };
        let native = match self.context {
            Some(context) => Some(NativeShaderRef {
                context,
                node: *nodes?.get(occurrence.0 as usize)?,
            }),
            None => None,
        };
        let registration = match native {
            Some(reference) => self.native.source(reference)?,
            None => registrations.get(occurrence.0 as usize)?,
        };
        if registration.defaulted.is_some()
            && let ShaderDefault::Global(binding) = default
        {
            return Some(ShaderRef::Direct(binding?));
        }
        Some(match (native, converted) {
            (Some(reference), Some(binding)) => ShaderRef::Surface {
                reference,
                lighting: binding,
                body: Some(binding),
            },
            (None, Some(binding)) => ShaderRef::Direct(binding),
            (Some(reference), None) => ShaderRef::Native(reference),
            (None, None) => ShaderRef::Direct(registration.shader),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedShader {
    binding: ShaderBindingId,
    time_offset: f32,
    cache_compatible: bool,
    surface_lighting: Option<ShaderBindingId>,
}
impl ResolvedShader {
    pub fn select(
        assets: &Assets,
        source: ShaderBindingId,
        target: ShaderBindingId,
        time_offset: f32,
    ) -> Option<Self> {
        let source_material = assets.material(assets.shader_binding(source)?.material)?;
        assets.material(assets.shader_binding(target)?.material)?;
        Some(Self {
            binding: target,
            surface_lighting: None,
            time_offset,
            cache_compatible: source == target
                && source_material.settings.time_offset.to_bits() == time_offset.to_bits(),
        })
    }
    pub fn binding(self) -> ShaderBindingId {
        self.binding
    }
    pub fn time_offset(self) -> f32 {
        self.time_offset
    }
    pub fn resources(self, assets: &Assets) -> Option<crate::ShaderBinding> {
        let mut binding = *assets.shader_binding(self.binding)?;
        if let Some(surface) = self.surface_lighting {
            let lighting = assets.shader_binding(surface)?;
            binding.lightmap = lighting.lightmap;
            binding.lightmap_region = lighting.lightmap_region;
            binding.lightmap_conversion = lighting.lightmap_conversion;
            binding.styled_surface = lighting.styled_surface;
        }
        Some(binding)
    }
    fn for_surface(mut self, assets: &Assets, surface: ShaderBindingId) -> Option<Self> {
        let binding = assets.shader_binding(self.binding)?;
        let lighting = assets.shader_binding(surface)?;
        if binding.light_source == crate::world::geometry::LightSource::Samples
            && lighting.light_source == crate::world::geometry::LightSource::Samples
        {
            self.surface_lighting = Some(surface);
        }
        Some(self)
    }
    pub fn matches_cache(self, assets: &Assets, source: ShaderBindingId) -> bool {
        self.cache_compatible && self.resources(assets) == assets.shader_binding(source).copied()
    }
    pub fn settings(self, assets: &Assets) -> Option<MaterialSettings> {
        let mut settings = assets
            .material(assets.shader_binding(self.binding)?.material)?
            .settings;
        settings.time_offset = self.time_offset;
        Some(settings)
    }
}

pub trait NativeShaders {
    fn source(&self, _reference: NativeShaderRef) -> Option<&ShaderRegistration> {
        None
    }
    fn resolve(&self, _reference: NativeShaderRef, _assets: &Assets) -> Option<ResolvedShader> {
        None
    }
    fn context(&self, _owner: ShaderOwner, _profile: ModelProfile) -> Option<ShaderContextId> {
        None
    }
    fn resource_nodes(
        &self,
        _context: ShaderContextId,
        _resource: ShaderResource,
    ) -> Option<&[ShaderNodeId]> {
        None
    }
}
impl NativeShaders for () {}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ShaderRef {
    Direct(ShaderBindingId),
    Native(NativeShaderRef),
    /// Converted legacy sample grids belong to the face, independently of the
    /// shared named shader body and its native lookup history.
    Surface {
        reference: NativeShaderRef,
        lighting: ShaderBindingId,
        body: Option<ShaderBindingId>,
    },
    Resolved(ResolvedShader),
}
impl Default for ShaderRef {
    fn default() -> Self {
        Self::Direct(ShaderBindingId(0))
    }
}
impl From<ShaderBindingId> for ShaderRef {
    fn from(binding: ShaderBindingId) -> Self {
        Self::Direct(binding)
    }
}
impl ShaderRef {
    pub fn source(self, native: &dyn NativeShaders) -> Option<ShaderBindingId> {
        match self {
            Self::Direct(binding) => Some(binding),
            Self::Native(reference)
            | Self::Surface {
                reference,
                body: None,
                ..
            } => Some(native.source(reference)?.shader),
            Self::Surface {
                reference,
                body: Some(binding),
                ..
            } => {
                native.source(reference)?;
                Some(binding)
            }
            Self::Resolved(shader) => Some(shader.binding()),
        }
    }
    pub fn resolve(self, assets: &Assets, native: &dyn NativeShaders) -> Option<ResolvedShader> {
        match self {
            Self::Direct(binding) => {
                let offset = assets
                    .material(assets.shader_binding(binding)?.material)?
                    .settings
                    .time_offset;
                ResolvedShader::select(assets, binding, binding, offset)
            }
            Self::Native(reference) => native.resolve(reference, assets),
            Self::Surface {
                reference,
                lighting,
                body,
            } => {
                let mut shader = native.resolve(reference, assets)?;
                if let Some(body) = body
                    && shader.binding() == native.source(reference)?.shader
                {
                    shader = ResolvedShader::select(assets, body, body, shader.time_offset())?;
                }
                shader.for_surface(assets, lighting)
            }
            Self::Resolved(shader) => Some(shader),
        }
    }
    pub fn for_surface(self, assets: &Assets, lighting: ShaderBindingId) -> Option<Self> {
        let binding = assets.shader_binding(lighting)?;
        Some(match self {
            Self::Native(reference)
                if binding.light_source == crate::world::geometry::LightSource::Samples =>
            {
                Self::Surface {
                    reference,
                    lighting,
                    body: None,
                }
            }
            shader => shader,
        })
    }
}
