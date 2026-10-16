//! Logical native shader creation order, independent of structural binding deduplication.
use crate::assets::ShaderRegistration;
use qa_core::primitives::{NameId, Vec3};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShaderNodeId(pub u32);
#[derive(Clone, Copy)]
struct Node {
    registration: ShaderRegistration,
    next: Option<ShaderNodeId>,
    remapped: Option<ShaderNodeId>,
    time_offset: f32,
}
pub struct ShaderContext {
    heads: Box<[Option<ShaderNodeId>]>,
    nodes: Vec<Node>,
    capacity: usize,
    sun_direction: Vec3,
    sun_light: Vec3,
}
impl ShaderContext {
    pub fn load(name_capacity: usize, node_capacity: usize) -> Result<Self, &'static str> {
        if node_capacity == 0 || node_capacity > u32::MAX as usize {
            return Err("shader context capacity");
        }
        let mut nodes = Vec::new();
        nodes
            .try_reserve_exact(node_capacity)
            .map_err(|_| "shader context capacity")?;
        Ok(Self {
            heads: vec![None; name_capacity].into_boxed_slice(),
            nodes,
            capacity: node_capacity,
            sun_direction: crate::lighting::default_sun_direction(),
            sun_light: Vec3::default(),
        })
    }
    pub fn len(&self) -> usize {
        self.nodes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
    pub fn node(&self, id: ShaderNodeId) -> Option<&ShaderRegistration> {
        self.nodes.get(id.0 as usize).map(|node| &node.registration)
    }
    pub fn latest(&self, key: NameId) -> Option<ShaderNodeId> {
        *self.heads.get(key.0 as usize)?
    }
    /// Native remaps select one target pointer, never a transitive name rule.
    pub fn resolved_node(&self, source: ShaderNodeId) -> Option<(ShaderNodeId, f32)> {
        let target = self
            .nodes
            .get(source.0 as usize)?
            .remapped
            .unwrap_or(source);
        Some((target, self.nodes.get(target.0 as usize)?.time_offset))
    }
    pub fn remap_existing(
        &mut self,
        key: NameId,
        target: ShaderNodeId,
        offset: Option<f32>,
    ) -> Result<(), &'static str> {
        self.nodes
            .get(target.0 as usize)
            .ok_or("shader remap target")?;
        let mut next = self.latest(key).ok_or("shader remap source")?;
        loop {
            let node = &mut self.nodes[next.0 as usize];
            node.remapped = (next != target).then_some(target);
            match node.next {
                Some(id) => next = id,
                None => break,
            }
        }
        if let Some(offset) = offset {
            self.nodes[target.0 as usize].time_offset = offset;
        }
        Ok(())
    }
    pub fn sun_direction(&self) -> Vec3 {
        self.sun_direction
    }
    pub fn sun_light(&self) -> Vec3 {
        self.sun_light
    }
    pub fn find(
        &self,
        key: NameId,
        requested: crate::world::geometry::LightSource,
    ) -> Option<ShaderNodeId> {
        let mut next = *self.heads.get(key.0 as usize)?;
        while let Some(id) = next {
            let node = &self.nodes[id.0 as usize];
            if node.registration.defaulted.is_some() || node.registration.finished == requested {
                return Some(id);
            }
            next = node.next;
        }
        None
    }
    pub fn append(&mut self) -> ShaderContextAppend<'_> {
        let retained = self.nodes.len();
        let sun_direction = self.sun_direction;
        let sun_light = self.sun_light;
        ShaderContextAppend {
            context: self,
            retained,
            sun_direction,
            sun_light,
            committed: false,
        }
    }
    pub fn reset(&mut self) {
        self.heads.fill(None);
        self.nodes.clear();
        self.sun_direction = crate::lighting::default_sun_direction();
        self.sun_light = Vec3::default();
    }
}
pub struct ShaderContextAppend<'a> {
    context: &'a mut ShaderContext,
    retained: usize,
    sun_direction: Vec3,
    sun_light: Vec3,
    committed: bool,
}
impl ShaderContextAppend<'_> {
    pub fn node(&self, id: ShaderNodeId) -> Option<&ShaderRegistration> {
        self.context.node(id)
    }
    pub fn reset_sun_direction(&mut self) {
        self.context.sun_direction = crate::lighting::default_sun_direction();
    }
    pub fn admit(
        &mut self,
        registration: ShaderRegistration,
    ) -> Result<ShaderNodeId, &'static str> {
        if let Some(id) = self.context.find(registration.key, registration.requested) {
            return Ok(id);
        }
        if self.context.nodes.len() == self.context.capacity {
            return Err("shader context capacity");
        }
        let head = self
            .context
            .heads
            .get_mut(registration.key.0 as usize)
            .ok_or("shader context name")?;
        let id = ShaderNodeId(self.context.nodes.len() as u32);
        self.context.nodes.push(Node {
            registration,
            next: *head,
            remapped: None,
            time_offset: 0.0,
        });
        *head = Some(id);
        if let Some(sun) = registration.sun {
            self.context.sun_direction = sun.direction();
            self.context.sun_light = qa_core::math::normalized(Vec3(sun.color)) * sun.intensity;
        }
        Ok(id)
    }
    pub fn commit(mut self) {
        self.committed = true;
    }
}
impl Drop for ShaderContextAppend<'_> {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        for node in self.context.nodes[self.retained..].iter().rev() {
            self.context.heads[node.registration.key.0 as usize] = node.next;
        }
        self.context.nodes.truncate(self.retained);
        self.context.sun_direction = self.sun_direction;
        self.context.sun_light = self.sun_light;
    }
}
