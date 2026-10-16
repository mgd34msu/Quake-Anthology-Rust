//! Renderer lifetime contexts shared by CLIENT UI and cgame, independent of the frontend.
use crate::model_assets::{ModelProfile, RendererAdmission, ShaderResource};
use qa_core::primitives::{ClientId, EntityId, IndexSpan, Vec3};
use qa_render::shader::{
    context::{ShaderContext, ShaderContextAppend, ShaderNodeId},
    reference::{NativeShaderRef, NativeShaders, ResolvedShader, ShaderContextId},
};

pub(super) use qa_render::shader::registration::ShaderOwner as Owner;
pub(super) type Handle = ShaderContextId;
struct ResourceAdmission {
    resource: ShaderResource,
    uses: IndexSpan,
}
struct Context {
    owner: Owner,
    profile: ModelProfile,
    generation: u64,
    users: usize,
    reset: Option<Option<u64>>,
    shaders: ShaderContext,
    uses: Vec<ShaderNodeId>,
    resources: Vec<ResourceAdmission>,
}
pub(crate) struct Contexts {
    slots: Vec<Option<Context>>,
    generation: u64,
}
impl Contexts {
    pub fn load(capacity: usize) -> Self {
        Self {
            slots: (0..capacity).map(|_| None).collect(),
            generation: 0,
        }
    }
    pub fn reserve(&mut self, capacity: usize) -> Result<(), String> {
        if capacity > u32::MAX as usize {
            return Err("renderer context slots".into());
        }
        if self.slots.len() < capacity {
            self.slots
                .try_reserve_exact(capacity - self.slots.len())
                .map_err(|_| "renderer context slots")?;
            self.slots.resize_with(capacity, || None);
        }
        Ok(())
    }
    pub fn retain_clients(&mut self, mut live: impl FnMut(ClientId, EntityId) -> bool) {
        for slot in &mut self.slots {
            let Some(context) = slot.as_ref().filter(|context| context.users == 0) else {
                continue;
            };
            if let Owner::Client(client, lifetime) = context.owner
                && !live(client, lifetime)
            {
                *slot = None;
            }
        }
    }
    pub(super) fn bind(
        &mut self,
        previous: Option<Handle>,
        owner: Owner,
        profile: ModelProfile,
        assets: &qa_render::Assets,
        budget: (usize, usize),
        reset: Option<u64>,
    ) -> Result<Handle, String> {
        if let Some(handle) = previous
            && self.get(handle).is_some_and(|context| {
                context.owner == owner && context.profile == profile && context.reset == Some(reset)
            })
        {
            return Ok(handle);
        }
        if let Some((slot, context)) =
            self.slots
                .iter_mut()
                .enumerate()
                .find_map(|(slot, context)| {
                    context
                        .as_mut()
                        .filter(|context| context.owner == owner && context.profile == profile)
                        .map(|context| (slot, context))
                })
        {
            if context.reset.is_some_and(|previous| previous != reset) {
                if context.users != 0 {
                    return Err("renderer context reset still active".into());
                }
                self.generation = self
                    .generation
                    .checked_add(1)
                    .ok_or("renderer context generation")?;
                context.generation = self.generation;
                context.shaders.reset();
                context.uses.clear();
                context.resources.clear();
            }
            context.reset = Some(reset);
            context.users += 1;
            let generation = context.generation;
            self.release(previous);
            return Ok(Handle {
                slot: u32::try_from(slot).map_err(|_| "renderer context slots")?,
                generation,
            });
        }
        let slot = self
            .slots
            .iter()
            .position(Option::is_none)
            .or_else(|| {
                previous
                    .filter(|handle| {
                        self.get(*handle).is_some_and(|context| {
                            context.users == 1 && matches!(context.owner, Owner::Private(..))
                        })
                    })
                    .map(|handle| handle.slot as usize)
            })
            .ok_or("renderer context slots")?;
        let shaders = ShaderContext::load(assets.name_capacity(), assets.shader_binding_capacity())
            .map_err(str::to_owned)?;
        let mut uses = Vec::new();
        uses.try_reserve_exact(budget.0)
            .map_err(|_| "renderer occurrence capacity")?;
        let mut resources = Vec::new();
        resources
            .try_reserve_exact(budget.1)
            .map_err(|_| "renderer resource capacity")?;
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or("renderer context generation")?;
        self.release(previous);
        self.slots[slot] = Some(Context {
            owner,
            profile,
            generation: self.generation,
            users: 1,
            reset: Some(reset),
            shaders,
            uses,
            resources,
        });
        Ok(Handle {
            slot: u32::try_from(slot).map_err(|_| "renderer context slots")?,
            generation: self.generation,
        })
    }
    pub(crate) fn ensure_owner(
        &mut self,
        owner: Owner,
        profile: ModelProfile,
        assets: &qa_render::Assets,
        budget: (usize, usize),
        reset: Option<u64>,
    ) -> Result<Handle, String> {
        if let Some(handle) = self.find(owner, profile)
            && self
                .get(handle)
                .is_some_and(|context| context.reset == Some(reset))
        {
            return Ok(handle);
        }
        if self.find(owner, profile).is_none() {
            self.reserve(
                self.slots
                    .len()
                    .checked_add(1)
                    .ok_or("renderer context slots")?,
            )?;
        }
        let handle = self.bind(None, owner, profile, assets, budget, reset)?;
        self.release(Some(handle));
        Ok(handle)
    }
    pub(crate) fn replay_graphics<'r>(
        &mut self,
        handle: Handle,
        registrations: impl Iterator<Item = &'r qa_render::ShaderRegistration> + Clone,
    ) -> Result<(), String> {
        let context = self
            .slots
            .get_mut(handle.slot as usize)
            .and_then(Option::as_mut)
            .filter(|context| context.generation == handle.generation)
            .ok_or("stale renderer context")?;
        stage_entries(
            context,
            (0, 0),
            registrations.map(|registration| {
                (
                    Some(ShaderResource::Graphic(registration.key)),
                    std::slice::from_ref(registration),
                )
            }),
        )?
        .commit();
        Ok(())
    }
    fn find(&self, owner: Owner, profile: ModelProfile) -> Option<Handle> {
        let (slot, context) = self.slots.iter().enumerate().find_map(|(slot, entry)| {
            entry
                .as_ref()
                .filter(|context| context.owner == owner && context.profile == profile)
                .map(|context| (slot, context))
        })?;
        Some(Handle {
            slot: u32::try_from(slot).ok()?,
            generation: context.generation,
        })
    }
    fn get(&self, handle: Handle) -> Option<&Context> {
        self.slots
            .get(handle.slot as usize)?
            .as_ref()
            .filter(|context| context.generation == handle.generation)
    }
    pub fn release(&mut self, handle: Option<Handle>) {
        let Some(handle) = handle else { return };
        if let Some(context) = self
            .slots
            .get_mut(handle.slot as usize)
            .and_then(Option::as_mut)
            .filter(|context| context.generation == handle.generation)
        {
            context.users -= 1;
            if context.users == 0 && matches!(context.owner, Owner::Private(..)) {
                self.slots[handle.slot as usize] = None;
            }
        }
    }
    pub fn owns_client(&self, handle: Handle, client: ClientId, lifetime: EntityId) -> bool {
        self.get(handle)
            .is_some_and(|context| context.owner == Owner::Client(client, lifetime))
    }
    pub fn reset_client(&mut self, client: ClientId, lifetime: EntityId) -> Result<(), String> {
        if self
            .slots
            .iter()
            .flatten()
            .any(|context| context.owner == Owner::Client(client, lifetime) && context.users != 0)
        {
            return Err("renderer context reset still active".into());
        }
        let count = self
            .slots
            .iter()
            .flatten()
            .filter(|context| context.owner == Owner::Client(client, lifetime))
            .count();
        self.generation
            .checked_add(u64::try_from(count).map_err(|_| "renderer context generation")?)
            .ok_or("renderer context generation")?;
        for context in self
            .slots
            .iter_mut()
            .flatten()
            .filter(|context| context.owner == Owner::Client(client, lifetime))
        {
            context.shaders.reset();
            context.uses.clear();
            context.resources.clear();
            context.reset = None;
            self.generation = self
                .generation
                .checked_add(1)
                .ok_or("renderer context generation")?;
            context.generation = self.generation;
        }
        Ok(())
    }
    pub fn resource_nodes(
        &self,
        handle: Handle,
        resource: ShaderResource,
    ) -> Option<&[ShaderNodeId]> {
        let context = self.get(handle)?;
        let span = context
            .resources
            .iter()
            .find(|entry| entry.resource == resource)?
            .uses;
        context.uses.get(span.range())
    }
    pub fn shader(&self, context: Handle, handle: i32) -> Option<NativeShaderRef> {
        let node = ShaderNodeId(u32::try_from(handle.checked_sub(1)?).ok()?);
        self.get(context)?.shaders.node(node)?;
        Some(NativeShaderRef { context, node })
    }
    pub fn sun(&self, handle: Option<Handle>) -> Option<Vec3> {
        Some(self.get(handle?)?.shaders.sun_direction())
    }
    pub(super) fn stage(
        &mut self,
        handle: Handle,
        admission: RendererAdmission<'_>,
    ) -> Result<Append<'_>, String> {
        let context = self
            .slots
            .get_mut(handle.slot as usize)
            .and_then(Option::as_mut)
            .filter(|context| context.generation == handle.generation)
            .ok_or("stale renderer context")?;
        stage_entries(
            context,
            admission.budget(),
            std::iter::once((admission.resource, admission.registrations)),
        )
    }
    pub(crate) fn stage_graphics<'a, 'r>(
        &'a mut self,
        registrations: impl Iterator<Item = (ModelProfile, &'r qa_render::ShaderRegistration)> + Clone,
    ) -> Result<Vec<Append<'a>>, String> {
        self.slots
            .iter_mut()
            .flatten()
            .filter_map(|context| {
                let profile = context.profile;
                let selected = registrations
                    .clone()
                    .filter(move |(owner, _)| *owner == profile);
                if selected.clone().next().is_none() {
                    return None;
                }
                Some(stage_entries(
                    context,
                    (0, 0),
                    selected.map(|(_, registration)| {
                        (
                            Some(ShaderResource::Graphic(registration.key)),
                            std::slice::from_ref(registration),
                        )
                    }),
                ))
            })
            .collect()
    }
}
fn stage_entries<'a, 'r>(
    context: &'a mut Context,
    budget: (usize, usize),
    entries: impl Iterator<Item = (Option<ShaderResource>, &'r [qa_render::ShaderRegistration])> + Clone,
) -> Result<Append<'a>, String> {
    let additional = entries
        .clone()
        .filter(|(resource, _)| {
            resource.is_some_and(|resource| {
                !context
                    .resources
                    .iter()
                    .any(|entry| entry.resource == resource)
            })
        })
        .try_fold((0usize, 0usize), |(uses, resources), (_, registrations)| {
            Ok::<_, String>((
                uses.checked_add(registrations.len())
                    .ok_or("renderer occurrence capacity")?,
                resources
                    .checked_add(1)
                    .ok_or("renderer resource capacity")?,
            ))
        })?;
    let uses = budget.0.max(
        context
            .uses
            .len()
            .checked_add(additional.0)
            .ok_or("renderer occurrence capacity")?,
    );
    let resources = budget.1.max(
        context
            .resources
            .len()
            .checked_add(additional.1)
            .ok_or("renderer resource capacity")?,
    );
    if context.uses.capacity() < uses {
        context
            .uses
            .try_reserve_exact(uses - context.uses.len())
            .map_err(|_| "renderer occurrence capacity")?;
    }
    if context.resources.capacity() < resources {
        context
            .resources
            .try_reserve_exact(resources - context.resources.len())
            .map_err(|_| "renderer resource capacity")?;
    }
    let retained_uses = context.uses.len();
    let retained_resources = context.resources.len();
    let mut append = Append {
        shaders: Some(context.shaders.append()),
        uses: &mut context.uses,
        resources: &mut context.resources,
        retained_uses,
        retained_resources,
        committed: false,
        shader_handle: 0,
    };
    for (resource, registrations) in entries {
        if let Some(resource) = resource {
            if let Some(previous) = append
                .resources
                .iter()
                .find(|entry| entry.resource == resource)
            {
                if previous.uses.range().end > append.uses.len() {
                    return Err("renderer occurrence span".into());
                }
                continue;
            }
            if matches!(resource, ShaderResource::World(_)) {
                append
                    .shaders
                    .as_mut()
                    .ok_or("renderer context append")?
                    .reset_sun_direction();
            }
            let first = u32::try_from(append.uses.len()).map_err(|_| "renderer occurrence span")?;
            for &registration in registrations {
                append.uses.push(
                    append
                        .shaders
                        .as_mut()
                        .ok_or("renderer context append")?
                        .admit(registration)
                        .map_err(str::to_owned)?,
                );
            }
            append.resources.push(ResourceAdmission {
                resource,
                uses: IndexSpan {
                    first,
                    count: u32::try_from(registrations.len())
                        .map_err(|_| "renderer occurrence span")?,
                },
            });
        } else {
            for &registration in registrations {
                let shaders = append.shaders.as_mut().ok_or("renderer context append")?;
                let node = shaders.admit(registration).map_err(str::to_owned)?;
                let selected = shaders.node(node).ok_or("renderer shader node")?;
                append.shader_handle = if selected.defaulted.is_some() {
                    0
                } else {
                    node.0
                        .checked_add(1)
                        .and_then(|value| i32::try_from(value).ok())
                        .ok_or("native shader handle capacity")?
                };
            }
        }
    }
    Ok(append)
}

pub(crate) struct Append<'a> {
    shaders: Option<ShaderContextAppend<'a>>,
    uses: &'a mut Vec<ShaderNodeId>,
    resources: &'a mut Vec<ResourceAdmission>,
    retained_uses: usize,
    retained_resources: usize,
    committed: bool,
    shader_handle: i32,
}
impl Append<'_> {
    pub fn shader_handle(&self) -> i32 {
        self.shader_handle
    }
    pub fn commit(mut self) {
        if let Some(shaders) = self.shaders.take() {
            shaders.commit();
        }
        self.committed = true;
    }
}
impl Drop for Append<'_> {
    fn drop(&mut self) {
        if !self.committed {
            self.uses.truncate(self.retained_uses);
            self.resources.truncate(self.retained_resources);
        }
    }
}

impl NativeShaders for Contexts {
    fn source(&self, reference: NativeShaderRef) -> Option<&qa_render::ShaderRegistration> {
        self.get(reference.context)?.shaders.node(reference.node)
    }
    fn resolve(
        &self,
        reference: NativeShaderRef,
        assets: &qa_render::Assets,
    ) -> Option<ResolvedShader> {
        let source = self.source(reference)?.shader;
        let offset = assets
            .material(assets.shader_binding(source)?.material)?
            .settings
            .time_offset;
        ResolvedShader::select(assets, source, source, offset)
    }
    fn context(&self, owner: Owner, profile: ModelProfile) -> Option<ShaderContextId> {
        self.find(owner, profile)
    }
    fn resource_nodes(
        &self,
        context: ShaderContextId,
        resource: ShaderResource,
    ) -> Option<&[ShaderNodeId]> {
        Contexts::resource_nodes(self, context, resource)
    }
}
