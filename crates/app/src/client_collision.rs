//! Prediction borrows accepted CLIENT lifetimes, never SERVER spatial state.
use crate::{
    client_scene::{ClientModelBindings, visit_raw},
    output_namespace::NativeNumbers,
};
use qa_core::{
    primitives::{
        Body, Bounds, ClientId, CollisionOwner, CollisionShape, CollisionTags, EntityId,
        ModelRules, ModuleId, ThinkTime, Vec3,
    },
    solids::{PackedSolid, SolidCodec},
    stamps::StampSet,
};
use qa_network::{
    commands::packet::Protocol,
    projection::{EntityProjection, EntityRecord},
    snapshots::{ReceivedFrame, snapshot_limits},
};
use qa_world::{
    area::{AreaGrid, LinkFlags, LinkIntent, LinkOrder},
    collision::{CollisionStore, Contents, TraceScratch, WorldTrace},
    entities::{AllocationPolicy, EntityTable, FullTable, MAX_ENTITIES, ReuseDelay},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientCollisionError {
    Waiting,
    Capacity,
    Entity(u32),
    Model(u32),
    Geometry,
}

const CLIENT_ALLOCATION: AllocationPolicy = AllocationPolicy {
    reuse: ReuseDelay::Milliseconds {
        milliseconds: 0,
        grace_end: 0,
    },
    full: FullTable::Reject,
};

#[derive(Clone, Copy)]
struct CollisionPolicy {
    model_rules: ModelRules,
    solids: Option<SolidCodec>,
    roles: qa_gameplay::rules::entities::EntityRoles,
    world_number: u32,
    passes: usize,
}

pub struct ClientCollision {
    client: ClientId,
    protocol: Protocol,
    policy: CollisionPolicy,
    entities: EntityTable,
    area: AreaGrid,
    numbers: NativeNumbers,
    seen: StampSet,
    ordered: Vec<EntityId>,
    scratch: TraceScratch,
    selected: Option<(qa_core::primitives::GeometryId, u32)>,
    accepted: Option<(u32, u64)>,
    self_number: Option<u32>,
    valid: bool,
    pub rejected_snapshots: u64,
    rejected: Option<(u32, u64)>,
}

impl ClientCollision {
    pub fn load(
        client: ClientId,
        protocol: Protocol,
        store: &CollisionStore,
    ) -> Result<Self, ClientCollisionError> {
        // Area partitions classify links; these bounds do not cap geometry.
        let bounds = Bounds {
            mins: Vec3([-1.; 3]),
            maxs: Vec3([1.; 3]),
        };
        Ok(Self {
            client,
            protocol,
            policy: CollisionPolicy {
                model_rules: qa_gameplay::rules::model_rules(protocol.rule_set()),
                solids: SolidCodec::load(protocol.rule_set()),
                roles: qa_gameplay::rules::entities::entity_roles(protocol.rule_set()),
                world_number: if protocol == Protocol::Quake3_68 {
                    1022
                } else {
                    0
                },
                passes: if protocol == Protocol::QuakeWorld28 {
                    2
                } else {
                    1
                },
            },
            entities: EntityTable::new(MAX_ENTITIES, 1)
                .map_err(|_| ClientCollisionError::Capacity)?,
            area: AreaGrid::load(MAX_ENTITIES, bounds)
                .map_err(|_| ClientCollisionError::Capacity)?,
            numbers: NativeNumbers::load(protocol),
            seen: StampSet::new(snapshot_limits(protocol).native_numbers),
            ordered: Vec::with_capacity(MAX_ENTITIES),
            scratch: store.scratch(),
            selected: None,
            accepted: None,
            self_number: None,
            valid: false,
            rejected_snapshots: 0,
            rejected: None,
        })
    }

    pub(crate) fn prepare_scratch(&mut self, store: &CollisionStore) {
        self.scratch = store.scratch();
    }

    pub fn reset(&mut self) {
        let mut next = 1;
        while let Some(id) = self.entities.next_active(next) {
            next = id.slot as usize + 1;
            self.area.unlink(id);
            self.entities.release(id, ThinkTime::Milliseconds(0));
        }
        self.numbers.reset();
        self.ordered.clear();
        self.selected = None;
        self.accepted = None;
        self.self_number = None;
        self.valid = false;
        self.rejected = None;
    }

    pub(crate) fn invalidate(&mut self) {
        self.valid = false;
    }

    pub fn entity(&self, number: u32) -> Option<EntityId> {
        if !self.valid {
            return None;
        }
        let world = number == self.policy.world_number;
        if world {
            self.entities.id_at(0)
        } else {
            self.numbers
                .entity(number)
                .filter(|&id| self.entities.resolve(id).is_some())
        }
    }

    /// Only the selected accepted channel frame is supplied by production.
    /// Late/repeated callbacks cannot replace its canonical CLIENT lifetimes.
    pub fn synchronize(
        &mut self,
        frame: ReceivedFrame<'_>,
        projection: &EntityProjection,
        models: &ClientModelBindings,
        store: &CollisionStore,
        self_number: Option<u32>,
    ) -> Result<(), ClientCollisionError> {
        if self.accepted.is_some_and(|(sequence, _)| {
            let distance = frame.sequence().wrapping_sub(sequence);
            distance != 0 && distance >= (1 << 31)
        }) {
            return Ok(());
        }
        let marker = (frame.sequence(), models.revision());
        let result = self.synchronize_inner(frame, projection, models, store, self_number);
        if result.is_err() && self.rejected != Some(marker) {
            self.rejected_snapshots = self.rejected_snapshots.saturating_add(1);
            self.rejected = Some(marker);
        }
        result
    }

    fn synchronize_inner(
        &mut self,
        frame: ReceivedFrame<'_>,
        projection: &EntityProjection,
        models: &ClientModelBindings,
        store: &CollisionStore,
        self_number: Option<u32>,
    ) -> Result<(), ClientCollisionError> {
        let marker = (frame.sequence(), models.revision());
        if self.valid && self.accepted == Some(marker) && self.self_number == self_number {
            return Ok(());
        }
        self.valid = false;
        let selected = models
            .collision_world()
            .ok_or(ClientCollisionError::Waiting)?;
        store
            .model_bounds(selected.0, selected.1)
            .ok_or(ClientCollisionError::Geometry)?;
        self.seen.begin();
        let mut count = 0;
        let mut error = None;
        visit_raw(
            frame,
            projection,
            self.client,
            |number, record, _peer, _dead| {
                if error.is_some() {
                    return;
                }
                if number as usize >= self.seen.len() || self.seen.contains(number as usize) {
                    error = Some(ClientCollisionError::Entity(number));
                    return;
                }
                self.seen.mark(number as usize);
                count += 1;
                error = record
                    .filter(|record| record.position.is_finite() && record.angles.is_finite())
                    .ok_or(ClientCollisionError::Entity(number))
                    .err();
            },
        );
        if let Some(error) = error {
            return Err(error);
        }
        if let Some(number) = self_number {
            if number as usize >= self.seen.len() {
                return Err(ClientCollisionError::Entity(number));
            }
            if !self.seen.contains(number as usize) {
                count += 1;
                self.seen.mark(number as usize);
            }
        }
        if count > self.entities.capacity() - self.entities.reserved_slots() {
            return Err(ClientCollisionError::Capacity);
        }
        let mut next = 1;
        while let Some(id) = self.entities.next_active(next) {
            next = id.slot as usize + 1;
            if self
                .numbers
                .number(id)
                .is_none_or(|number| !self.seen.contains(number as usize))
            {
                self.area.unlink(id);
                self.entities.release(id, ThinkTime::Milliseconds(0));
            }
        }
        self.numbers.begin(&self.entities, None, self.protocol);
        self.ordered.clear();
        // Original QW builds BSP physents before accepted living player boxes.
        let policy = self.policy;
        let passes = policy.passes;
        for pass in 0..passes {
            visit_raw(
                frame,
                projection,
                self.client,
                |number, record, peer, dead| {
                    if error.is_some() || passes == 2 && peer != (pass == 1) {
                        return;
                    }
                    let Some(record) = record else {
                        error = Some(ClientCollisionError::Entity(number));
                        return;
                    };
                    let target = Self::target(policy, number, record, peer, dead, models, store);
                    let Ok((body, angles, shape, rules, contents, tags)) = target else {
                        error = target.err();
                        return;
                    };
                    let Some(id) = self.claim(number) else {
                        error = Some(ClientCollisionError::Capacity);
                        return;
                    };
                    let slot = id.slot as usize;
                    let columns = &mut self.entities.columns;
                    columns.set_body(slot, body);
                    columns.angles[slot] = angles;
                    columns.appearance[slot] = record.appearance;
                    columns.collision_shape[slot] = shape;
                    columns.model_rules[slot] = rules;
                    columns.collision_contents[slot] = contents.0;
                    columns.collision_tags[slot] = tags;
                    if shape != CollisionShape::None {
                        self.area.link(
                            &self.entities,
                            id,
                            LinkFlags::SOLID,
                            LinkOrder::Tail,
                            LinkIntent::Commit,
                        );
                        self.ordered.push(id);
                    } else {
                        self.area.unlink(id);
                    }
                },
            );
        }
        if let Some(error) = error {
            return Err(error);
        }
        if let Some(number) = self_number
            && self.numbers.entity(number).is_none()
            && self.claim(number).is_none()
        {
            return Err(ClientCollisionError::Capacity);
        }
        self.selected = Some(selected);
        self.accepted = Some(marker);
        self.self_number = self_number;
        self.valid = true;
        Ok(())
    }

    fn claim(&mut self, number: u32) -> Option<EntityId> {
        if let Some(id) = self.numbers.entity(number) {
            return Some(id);
        }
        // Engine-owned rows have no native module identity. Native ordinal
        // binding is exclusively the connection's existing NativeNumbers map.
        let id = self
            .entities
            .allocate(
                ThinkTime::Milliseconds(0),
                ModuleId::default(),
                CLIENT_ALLOCATION,
            )?
            .id;
        self.numbers.bind(id, number).then_some(id)
    }

    fn target(
        policy: CollisionPolicy,
        number: u32,
        record: EntityRecord,
        peer: bool,
        dead: bool,
        models: &ClientModelBindings,
        store: &CollisionStore,
    ) -> Result<
        (
            Body,
            Vec3,
            CollisionShape,
            ModelRules,
            Contents,
            CollisionTags,
        ),
        ClientCollisionError,
    > {
        let rules = policy.model_rules;
        let mut body = Body {
            position: record.position,
            velocity: record.velocity,
            ..Default::default()
        };
        let mut shape = CollisionShape::None;
        let mut contents = Contents::BODY;
        let mut tags = CollisionTags::default();
        let packed = policy
            .solids
            .map(|codec| codec.decode(record.appearance.native.solid));
        let brush = matches!(packed, Some(PackedSolid::Brush)) || packed.is_none() && !peer;
        if !policy
            .roles
            .client_solid(record.appearance.native.entity_type)
        {
            return Ok((body, record.angles, shape, rules, contents, tags));
        }
        if peer && !dead {
            body.mins = Vec3([-16., -16., -24.]);
            body.maxs = Vec3([16., 16., 32.]);
            shape = CollisionShape::Box;
            tags = CollisionTags::PLAYER;
        } else if let Some(PackedSolid::Box(bounds)) = packed {
            if !bounds.is_valid() {
                return Err(ClientCollisionError::Entity(number));
            }
            body.mins = bounds.mins;
            body.maxs = bounds.maxs;
            shape = CollisionShape::Box;
        } else if brush {
            let binding =
                record.appearance.models[0].and_then(|model| models.collision_model(model));
            if let Some((geometry, model)) = binding {
                let bounds = store
                    .model_bounds(geometry, model)
                    .ok_or(ClientCollisionError::Geometry)?;
                body.mins = bounds.mins;
                body.maxs = bounds.maxs;
                shape = CollisionShape::Model {
                    geometry,
                    index: model,
                };
                contents = Contents::SOLID;
            } else if packed.is_some() {
                return Err(ClientCollisionError::Model(number));
            }
        }
        let angles = if matches!(shape, CollisionShape::Box) {
            Vec3::default()
        } else {
            record.angles
        };
        Ok((body, angles, shape, rules, contents, tags))
    }

    pub fn trace<'a>(
        &'a mut self,
        store: &'a CollisionStore,
    ) -> Result<WorldTrace<'a>, ClientCollisionError> {
        if !self.valid {
            return Err(ClientCollisionError::Waiting);
        }
        let (geometry, model) = self.selected.ok_or(ClientCollisionError::Waiting)?;
        let pass = self
            .self_number
            .and_then(|number| self.entity(number))
            .map_or(CollisionOwner::None, CollisionOwner::Lifetime);
        Ok(WorldTrace::new(
            store,
            geometry,
            model,
            &self.entities,
            &self.area,
            &mut self.scratch,
            pass,
        )
        .with_candidates(&self.ordered))
    }
}
