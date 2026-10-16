//! Renderer lifetime contexts shared by CLIENT UI and cgame, independent of the frontend.
use crate::model_assets::{ModelProfile, RendererAdmission, ShaderResource};
use qa_core::primitives::{ClientId, EntityId, IndexSpan, ModuleId, Vec3};
use qa_render::shader::context::{ShaderContext, ShaderContextAppend, ShaderNodeId};

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Owner {
    Client(ClientId, EntityId),
    Private(ModuleId, EntityId),
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct Handle {
    slot: usize,
    generation: u64,
}
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
pub(super) struct Contexts {
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
    pub fn bind(
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
            return Ok(Handle { slot, generation });
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
                    .map(|handle| handle.slot)
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
            slot,
            generation: self.generation,
        })
    }
    fn get(&self, handle: Handle) -> Option<&Context> {
        self.slots
            .get(handle.slot)?
            .as_ref()
            .filter(|context| context.generation == handle.generation)
    }
    pub fn release(&mut self, handle: Option<Handle>) {
        let Some(handle) = handle else { return };
        if let Some(context) = self
            .slots
            .get_mut(handle.slot)
            .and_then(Option::as_mut)
            .filter(|context| context.generation == handle.generation)
        {
            context.users -= 1;
            if context.users == 0 && matches!(context.owner, Owner::Private(..)) {
                self.slots[handle.slot] = None;
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
    pub fn sun(&self, handle: Option<Handle>) -> Option<Vec3> {
        Some(self.get(handle?)?.shaders.sun_direction())
    }
    pub fn stage(
        &mut self,
        handle: Handle,
        admission: RendererAdmission<'_>,
    ) -> Result<Append<'_>, String> {
        let context = self
            .slots
            .get_mut(handle.slot)
            .and_then(Option::as_mut)
            .filter(|context| context.generation == handle.generation)
            .ok_or("stale renderer context")?;
        let repeated = admission.resource.is_some_and(|resource| {
            context
                .resources
                .iter()
                .any(|previous| previous.resource == resource)
        });
        if admission.resource.is_some()
            && !repeated
            && (context.uses.capacity()
                < context
                    .uses
                    .len()
                    .saturating_add(admission.registrations.len())
                || context.resources.capacity() == context.resources.len())
        {
            let budget = admission.budget();
            if context.uses.capacity() < budget.0 {
                context
                    .uses
                    .try_reserve_exact(budget.0 - context.uses.len())
                    .map_err(|_| "renderer occurrence capacity")?;
            }
            if context.resources.capacity() < budget.1 {
                context
                    .resources
                    .try_reserve_exact(budget.1 - context.resources.len())
                    .map_err(|_| "renderer resource capacity")?;
            }
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
        };
        if let Some(resource) = admission.resource {
            if let Some(previous) = append
                .resources
                .iter()
                .find(|previous| previous.resource == resource)
            {
                // This span is the original occurrence-to-node mapping, not another seen set.
                if previous.uses.range().end > append.uses.len() {
                    return Err("renderer occurrence span".into());
                }
                return Ok(append);
            }
            if matches!(resource, ShaderResource::World(_)) {
                append
                    .shaders
                    .as_mut()
                    .ok_or("renderer context append")?
                    .reset_sun_direction();
            }
            let first = u32::try_from(append.uses.len()).map_err(|_| "renderer occurrence span")?;
            for &registration in admission.registrations {
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
                    count: u32::try_from(admission.registrations.len())
                        .map_err(|_| "renderer occurrence span")?,
                },
            });
        } else {
            for &registration in admission.registrations {
                append
                    .shaders
                    .as_mut()
                    .ok_or("renderer context append")?
                    .admit(registration)
                    .map_err(str::to_owned)?;
            }
        }
        Ok(append)
    }
}
pub(super) struct Append<'a> {
    shaders: Option<ShaderContextAppend<'a>>,
    uses: &'a mut Vec<ShaderNodeId>,
    resources: &'a mut Vec<ResourceAdmission>,
    retained_uses: usize,
    retained_resources: usize,
    committed: bool,
}
impl Append<'_> {
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
