//! Renderer lifetime contexts shared by CLIENT UI and cgame, independent of the frontend.
use crate::model_assets::{ModelProfile, RendererAdmission, ShaderResource};
use qa_content::vfs::{FileSearch, SearchView};
use qa_core::primitives::{ClientId, EntityId, IndexSpan, NameId, Vec3};
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
impl ResourceAdmission {
    fn retained(
        resources: &[Self],
        resource: ShaderResource,
        registrations: usize,
    ) -> Option<(usize, bool)> {
        resources
            .iter()
            .position(|entry| entry.resource == resource)
            .map(|index| {
                (
                    index,
                    resources[index].uses.count != 0 || registrations == 0,
                )
            })
    }
}
struct Context {
    owner: Owner,
    files: FileSearch,
    profile: ModelProfile,
    generation: u64,
    users: usize,
    reset: Option<Option<u64>>,
    shaders: ShaderContext,
    uses: Vec<ShaderNodeId>,
    resources: Vec<ResourceAdmission>,
    resource_undo: Vec<(usize, IndexSpan)>,
}
impl Context {
    fn matches(&self, owner: Owner, profile: ModelProfile, search: &SearchView) -> bool {
        self.owner == owner && self.profile == profile && self.files.view().same_selection(search)
    }
}
pub(crate) struct Contexts {
    default: SearchView,
    slots: Vec<Option<Context>>,
    generation: u64,
}
impl Contexts {
    pub fn load(default: SearchView, capacity: usize) -> Self {
        Self {
            default,
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
        self.retain_unused(|context| match context.owner {
            Owner::Client(client, lifetime) => live(client, lifetime),
            _ => true,
        });
    }
    pub(crate) fn retain_profiles(&mut self, mut retained: impl FnMut(ModelProfile) -> bool) {
        self.retain_unused(|context| retained(context.profile));
    }
    fn retain_unused(&mut self, mut retained: impl FnMut(&Context) -> bool) {
        for slot in &mut self.slots {
            let Some(context) = slot.as_ref().filter(|context| context.users == 0) else {
                continue;
            };
            if !retained(context) {
                *slot = None;
            }
        }
    }
    pub(super) fn bind(
        &mut self,
        previous: Option<Handle>,
        owner: Owner,
        profile: ModelProfile,
        files: &FileSearch,
        assets: &qa_render::Assets,
        budget: (usize, usize),
        reset: Option<u64>,
    ) -> Result<Handle, String> {
        let search = files.view();
        if let Some(handle) = previous
            && self.get(handle).is_some_and(|context| {
                context.matches(owner, profile, &search) && context.reset == Some(reset)
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
                        .filter(|context| context.matches(owner, profile, &search))
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
        let available = self.slots.iter().position(Option::is_none).or_else(|| {
            previous
                .filter(|handle| {
                    self.get(*handle).is_some_and(|context| {
                        context.users == 1 && matches!(context.owner, Owner::Private(..))
                    })
                })
                .map(|handle| handle.slot as usize)
        });
        let slot = match available {
            Some(slot) => slot,
            None => {
                let slot = self.slots.len();
                self.reserve(slot.checked_add(1).ok_or("renderer context slots")?)?;
                slot
            }
        };
        let shaders = ShaderContext::load(assets.name_capacity(), assets.shader_binding_capacity())
            .map_err(str::to_owned)?;
        let mut uses = Vec::new();
        uses.try_reserve_exact(budget.0)
            .map_err(|_| "renderer occurrence capacity")?;
        let mut resources = Vec::new();
        resources
            .try_reserve_exact(budget.1)
            .map_err(|_| "renderer resource capacity")?;
        let mut resource_undo = Vec::new();
        resource_undo
            .try_reserve_exact(budget.1)
            .map_err(|_| "renderer resource capacity")?;
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or("renderer context generation")?;
        self.release(previous);
        self.slots[slot] = Some(Context {
            owner,
            files: files.clone(),
            profile,
            generation: self.generation,
            users: 1,
            reset: Some(reset),
            shaders,
            uses,
            resources,
            resource_undo,
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
        files: &FileSearch,
        assets: &qa_render::Assets,
        budget: (usize, usize),
        reset: Option<u64>,
    ) -> Result<Handle, String> {
        let search = files.view();
        let existing = self.find(owner, profile, Some(&search));
        if let Some(handle) = existing
            && self
                .get(handle)
                .is_some_and(|context| context.reset == Some(reset))
        {
            return Ok(handle);
        }
        if existing.is_none() {
            self.reserve(
                self.slots
                    .len()
                    .checked_add(1)
                    .ok_or("renderer context slots")?,
            )?;
        }
        let handle = self.bind(None, owner, profile, files, assets, budget, reset)?;
        self.release(Some(handle));
        Ok(handle)
    }
    pub(crate) fn replay_graphics<'r>(
        &mut self,
        handle: Handle,
        registrations: impl Iterator<Item = crate::renderer::GraphicRegistration<'r>> + Clone,
    ) -> Result<(), String> {
        let context = self
            .slots
            .get_mut(handle.slot as usize)
            .and_then(Option::as_mut)
            .filter(|context| context.generation == handle.generation)
            .ok_or("stale renderer context")?;
        let profile = context.profile;
        let files = context.files.clone();
        let selected =
            registrations.filter(move |registration| registration.matches(profile, &files));
        stage_entries(
            context,
            (0, 0),
            selected.map(crate::renderer::GraphicRegistration::entry),
        )?
        .commit();
        Ok(())
    }
    fn find(
        &self,
        owner: Owner,
        profile: ModelProfile,
        search: Option<&SearchView>,
    ) -> Option<Handle> {
        let search = search.unwrap_or(&self.default);
        self.find_source(owner, profile, |files| files.view().same_selection(search))
    }
    fn find_source(
        &self,
        owner: Owner,
        profile: ModelProfile,
        selected: impl Fn(&FileSearch) -> bool,
    ) -> Option<Handle> {
        let (slot, context) = self.slots.iter().enumerate().find_map(|(slot, entry)| {
            entry
                .as_ref()
                .filter(|context| {
                    context.owner == owner && context.profile == profile && selected(&context.files)
                })
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
    pub(crate) fn search(&self, handle: Handle) -> Option<&SearchView> {
        Some(self.get(handle)?.files.view())
    }
    pub(super) fn matches_search(&self, handle: Handle, search: Option<&SearchView>) -> bool {
        self.search(handle)
            .is_some_and(|current| current.same_selection(search.unwrap_or(&self.default)))
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
        self.reset_selected(|owner| owner == Owner::Client(client, lifetime))
    }
    pub fn reset_all(&mut self) -> Result<(), String> {
        self.reset_selected(|_| true)
    }
    fn reset_selected(&mut self, selected: impl Fn(Owner) -> bool) -> Result<(), String> {
        if self
            .slots
            .iter()
            .flatten()
            .any(|context| selected(context.owner) && context.users != 0)
        {
            return Err("renderer context reset still active".into());
        }
        let count = self
            .slots
            .iter()
            .flatten()
            .filter(|context| selected(context.owner))
            .count();
        self.generation
            .checked_add(u64::try_from(count).map_err(|_| "renderer context generation")?)
            .ok_or("renderer context generation")?;
        for context in self
            .slots
            .iter_mut()
            .flatten()
            .filter(|context| selected(context.owner))
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
    pub(super) fn contains(
        &self,
        handle: Handle,
        registration: &qa_render::ShaderRegistration,
    ) -> bool {
        self.get(handle).is_some_and(|context| {
            context
                .shaders
                .find(registration.key, registration.requested)
                .is_some()
        })
    }
    pub(super) fn latest(&self, handle: Handle, key: NameId) -> Option<ShaderNodeId> {
        self.get(handle)?.shaders.latest(key)
    }
    pub(super) fn find_shader(
        &self,
        handle: Handle,
        key: NameId,
        requested: qa_render::world::geometry::LightSource,
    ) -> Option<(ShaderNodeId, &qa_render::ShaderRegistration)> {
        let context = self.get(handle)?;
        let node = context.shaders.find(key, requested)?;
        Some((node, context.shaders.node(node)?))
    }
    pub(super) fn remap(
        &mut self,
        handle: Handle,
        key: NameId,
        target: ShaderNodeId,
        offset: Option<f32>,
    ) -> Result<(), String> {
        self.slots
            .get_mut(handle.slot as usize)
            .and_then(Option::as_mut)
            .filter(|context| context.generation == handle.generation)
            .ok_or("stale renderer context")?
            .shaders
            .remap_existing(key, target, offset)
            .map_err(str::to_owned)
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
        registrations: impl Iterator<Item = crate::renderer::GraphicRegistration<'r>> + Clone,
    ) -> Result<Vec<Append<'a>>, String> {
        self.slots
            .iter_mut()
            .flatten()
            .filter_map(|context| {
                let profile = context.profile;
                let files = context.files.clone();
                let selected = registrations
                    .clone()
                    .filter(move |registration| registration.matches(profile, &files));
                if selected.clone().next().is_none() {
                    return None;
                }
                Some(stage_entries(
                    context,
                    (0, 0),
                    selected.map(crate::renderer::GraphicRegistration::entry),
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
    let additional = entries.clone().try_fold(
        (0usize, 0usize),
        |(uses, resources), (resource, registrations)| {
            let Some(resource) = resource else {
                return Ok((uses, resources));
            };
            let previous =
                ResourceAdmission::retained(&context.resources, resource, registrations.len());
            if previous.is_some_and(|(_, retained)| retained) {
                return Ok((uses, resources));
            }
            Ok::<_, String>((
                uses.checked_add(registrations.len())
                    .ok_or("renderer occurrence capacity")?,
                resources
                    .checked_add(usize::from(previous.is_none()))
                    .ok_or("renderer resource capacity")?,
            ))
        },
    )?;
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
    if context.resource_undo.capacity() < resources {
        context
            .resource_undo
            .try_reserve_exact(resources)
            .map_err(|_| "renderer resource capacity")?;
    }
    context.resource_undo.clear();
    let retained_uses = context.uses.len();
    let retained_resources = context.resources.len();
    let mut append = Append {
        shaders: Some(context.shaders.append()),
        uses: &mut context.uses,
        resources: &mut context.resources,
        resource_undo: &mut context.resource_undo,
        retained_uses,
        retained_resources,
        committed: false,
        shader_handle: 0,
        shader_node: None,
    };
    for (resource, registrations) in entries {
        if let Some(resource) = resource {
            let previous =
                ResourceAdmission::retained(append.resources, resource, registrations.len());
            if let Some((index, retained)) = previous {
                let previous = &append.resources[index];
                if previous.uses.range().end > append.uses.len() {
                    return Err("renderer occurrence span".into());
                }
                if retained {
                    continue;
                }
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
            let uses = IndexSpan {
                first,
                count: u32::try_from(registrations.len())
                    .map_err(|_| "renderer occurrence span")?,
            };
            if let Some((index, _)) = previous {
                append
                    .resource_undo
                    .push((index, append.resources[index].uses));
                append.resources[index].uses = uses;
            } else {
                append.resources.push(ResourceAdmission { resource, uses });
            }
        } else {
            for &registration in registrations {
                let shaders = append.shaders.as_mut().ok_or("renderer context append")?;
                let node = shaders.admit(registration).map_err(str::to_owned)?;
                let selected = shaders.node(node).ok_or("renderer shader node")?;
                append.shader_node = Some(node);
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
    resource_undo: &'a mut Vec<(usize, IndexSpan)>,
    retained_uses: usize,
    retained_resources: usize,
    committed: bool,
    shader_handle: i32,
    shader_node: Option<ShaderNodeId>,
}
impl Append<'_> {
    pub fn shader(&self) -> Option<(ShaderNodeId, &qa_render::ShaderRegistration)> {
        let node = self.shader_node?;
        Some((node, self.shaders.as_ref()?.node(node)?))
    }
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
            for &(index, uses) in self.resource_undo.iter().rev() {
                self.resources[index].uses = uses;
            }
            self.uses.truncate(self.retained_uses);
            self.resources.truncate(self.retained_resources);
        }
        self.resource_undo.clear();
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
        let context = self.get(reference.context)?;
        let source = context.shaders.node(reference.node)?.shader;
        let (target, offset) = context.shaders.resolved_node(reference.node)?;
        let binding = context.shaders.node(target)?.shader;
        ResolvedShader::select(assets, source, binding, offset, target == reference.node)
    }
    fn context(
        &self,
        owner: Owner,
        profile: ModelProfile,
        source: Option<qa_core::primitives::FileSearchId>,
    ) -> Option<ShaderContextId> {
        self.find_source(owner, profile, |files| files.id() == source)
    }
    fn resource_nodes(
        &self,
        context: ShaderContextId,
        resource: ShaderResource,
    ) -> Option<&[ShaderNodeId]> {
        Contexts::resource_nodes(self, context, resource)
    }
}

#[cfg(test)]
#[path = "../../tests/modules/renderer_sources.rs"]
mod tests;
