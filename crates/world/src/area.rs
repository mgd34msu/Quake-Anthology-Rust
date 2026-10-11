use crate::entities::{EntityTable, MAX_ENTITIES};
use qa_core::{
    primitives::{
        BodyAttachment, BodyFollow, Bounds, CollisionShape, EntityId, RotatedLinkBounds, Vec3,
    },
    stamps::StampSet,
};

const NONE: u32 = u32::MAX;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LinkFlags(pub u8);
impl LinkFlags {
    pub const SOLID: Self = Self(1);
    pub const TRIGGER: Self = Self(2);
    pub const ITEM: u8 = 4;
    /// All spatially linked rows, including non-colliding Q3 entities.
    pub const LINKED: Self = Self(8);
}

pub use qa_core::primitives::LinkOrder;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkIntent {
    /// A module's LinkEntity call preserves native unlink/reinsert ordering.
    Explicit,
    /// An internal body commit need not republish unchanged spatial state.
    Commit,
}

#[derive(Clone, Copy)]
struct Node {
    axis: Option<usize>,
    distance: f32,
    children: [u8; 2],
    head: u32,
    tail: u32,
}
impl Default for Node {
    fn default() -> Self {
        Self {
            axis: None,
            distance: 0.0,
            children: [0; 2],
            head: NONE,
            tail: NONE,
        }
    }
}
#[derive(Clone, Copy)]
struct Link {
    id: Option<EntityId>,
    bounds: Bounds,
    flags: LinkFlags,
    order: LinkOrder,
    node: u8,
    previous: u32,
    next: u32,
}
impl Default for Link {
    fn default() -> Self {
        Self {
            id: None,
            bounds: Bounds::default(),
            flags: LinkFlags::default(),
            order: LinkOrder::Tail,
            node: 0,
            previous: NONE,
            next: NONE,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum AreaError {
    Capacity,
    Bounds,
}

pub struct AreaGrid {
    nodes: [Node; 31],
    links: Box<[Link]>,
    pub relinks: u64,
    attachment_visited: StampSet,
    attachment_chain: Vec<(EntityId, BodyAttachment)>,
    attachment_positions: Box<[Vec3]>,
}

enum AttachmentBodies<'a> {
    Authoritative(&'a mut EntityTable),
    Predicted {
        table: &'a EntityTable,
        poses: &'a mut [Option<(EntityId, Vec3)>],
    },
}

impl AttachmentBodies<'_> {
    fn table(&self) -> &EntityTable {
        match self {
            Self::Authoritative(table) => table,
            Self::Predicted { table, .. } => table,
        }
    }

    fn position(&self, id: EntityId, slot: usize) -> Vec3 {
        if let Self::Predicted { poses, .. } = self
            && let Some((_, position)) = poses.iter().flatten().find(|(entity, _)| *entity == id)
        {
            return *position;
        }
        self.table().columns.position[slot]
    }

    fn set_position(&mut self, id: EntityId, slot: usize, position: Vec3) {
        match self {
            Self::Authoritative(table) => table.columns.position[slot] = position,
            Self::Predicted { poses, .. } => {
                if let Some((_, pose)) =
                    poses.iter_mut().flatten().find(|(entity, _)| *entity == id)
                {
                    *pose = position;
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AttachmentTransport {
    pub visited: u32,
    pub moved: u32,
    pub relinked: u32,
    pub rejected: u32,
}

fn create(nodes: &mut [Node; 31], next: &mut u8, depth: u8, bounds: Bounds) -> u8 {
    let index = *next;
    *next += 1;
    if depth == 4 {
        return index;
    }
    let axis =
        usize::from(bounds.maxs.0[0] - bounds.mins.0[0] <= bounds.maxs.0[1] - bounds.mins.0[1]);
    let distance = 0.5 * (bounds.maxs.0[axis] + bounds.mins.0[axis]);
    let mut front = bounds;
    let mut back = bounds;
    front.mins.0[axis] = distance;
    back.maxs.0[axis] = distance;
    let children = [
        create(nodes, next, depth + 1, front),
        create(nodes, next, depth + 1, back),
    ];
    nodes[index as usize] = Node {
        axis: Some(axis),
        distance,
        children,
        ..Node::default()
    };
    index
}

pub struct LinkedEntity<'a> {
    pub id: EntityId,
    pub position: &'a Vec3,
    pub velocity: &'a Vec3,
    pub mins: &'a Vec3,
    pub maxs: &'a Vec3,
    pub flags: LinkFlags,
}

impl AreaGrid {
    pub fn load(capacity: usize, bounds: Bounds) -> Result<Self, AreaError> {
        if capacity == 0 || capacity > MAX_ENTITIES {
            return Err(AreaError::Capacity);
        }
        if !bounds.is_valid() {
            return Err(AreaError::Bounds);
        }
        let mut nodes = [Node::default(); 31];
        create(&mut nodes, &mut 0, 0, bounds);
        Ok(Self {
            nodes,
            links: vec![Link::default(); capacity].into_boxed_slice(),
            relinks: 0,
            attachment_visited: StampSet::new(capacity),
            attachment_chain: Vec::with_capacity(capacity),
            attachment_positions: vec![Vec3::default(); capacity].into_boxed_slice(),
        })
    }

    pub fn unlink(&mut self, id: EntityId) -> bool {
        let Some(link) = self
            .links
            .get(id.slot as usize)
            .copied()
            .filter(|link| link.id == Some(id))
        else {
            return false;
        };
        let node = &mut self.nodes[link.node as usize];
        if link.previous == NONE {
            node.head = link.next;
        } else {
            self.links[link.previous as usize].next = link.next;
        }
        if link.next == NONE {
            node.tail = link.previous;
        } else {
            self.links[link.next as usize].previous = link.previous;
        }
        self.links[id.slot as usize] = Link::default();
        true
    }

    pub fn bounds(&self, id: EntityId) -> Option<Bounds> {
        self.links
            .get(id.slot as usize)
            .filter(|link| link.id == Some(id))
            .map(|link| link.bounds)
    }

    pub fn link(
        &mut self,
        table: &EntityTable,
        id: EntityId,
        flags: LinkFlags,
        order: LinkOrder,
        intent: LinkIntent,
    ) -> bool {
        let Some(slot) = table
            .resolve(id)
            .filter(|slot| *slot != 0 && *slot < self.links.len())
        else {
            return false;
        };
        let flags =
            if flags.0 & (LinkFlags::SOLID.0 | LinkFlags::TRIGGER.0 | LinkFlags::LINKED.0) != 0 {
                LinkFlags(flags.0 | LinkFlags::LINKED.0)
            } else {
                flags
            };
        let columns = &table.columns;
        let item = flags.0 & LinkFlags::ITEM != 0;
        let expansion = |axis| {
            if item {
                if axis < 2 { 15.0 } else { 0.0 }
            } else {
                1.0
            }
        };
        let rotated = matches!(columns.collision_shape[slot], CollisionShape::Model { .. })
            && columns.angles[slot].0.iter().any(|&angle| angle != 0.0);
        let radius = if rotated {
            match columns.model_rules[slot].link_bounds {
                RotatedLinkBounds::Unrotated => None,
                // qsrc Q2 sv_world.c:219-245 uses the largest absolute bound.
                RotatedLinkBounds::MaxAbsCube => Some(
                    columns.mins[slot]
                        .0
                        .iter()
                        .chain(&columns.maxs[slot].0)
                        .fold(0.0f32, |radius, value| radius.max(value.abs())),
                ),
                // qsrc Q3 q_math.c:1050-1061 first chooses each farthest corner.
                RotatedLinkBounds::RadiusCube => {
                    let corner = std::array::from_fn::<_, 3, _>(|axis| {
                        columns.mins[slot].0[axis]
                            .abs()
                            .max(columns.maxs[slot].0[axis].abs())
                    });
                    Some(qa_core::math::length(Vec3(corner)))
                }
            }
        } else {
            None
        };
        let bounds = Bounds {
            mins: Vec3(std::array::from_fn(|axis| {
                columns.position[slot].0[axis]
                    + radius.map_or(columns.mins[slot].0[axis], |radius| -radius)
                    - expansion(axis)
            })),
            maxs: Vec3(std::array::from_fn(|axis| {
                columns.position[slot].0[axis]
                    + radius.unwrap_or(columns.maxs[slot].0[axis])
                    + expansion(axis)
            })),
        };
        let old = self.links[slot];
        if intent == LinkIntent::Commit
            && old.id == Some(id)
            && old.bounds == bounds
            && old.flags == flags
            && old.order == order
        {
            return false;
        }
        if let Some(old_id) = old.id {
            self.unlink(old_id);
        }
        if flags.0 & LinkFlags::LINKED.0 == 0 {
            return false;
        }
        let mut index = 0;
        while let Some(axis) = self.nodes[index].axis {
            let node = self.nodes[index];
            if bounds.mins.0[axis] > node.distance {
                index = node.children[0] as usize;
            } else if bounds.maxs.0[axis] < node.distance {
                index = node.children[1] as usize;
            } else {
                break;
            }
        }
        let node = &mut self.nodes[index];
        // Q1 world.c:463-466 appends; Q3 sv_world.c:350-353 prepends.
        // One list preserves Q3's observable order across solid/trigger roles.
        let (previous, next) = match order {
            LinkOrder::Head => (NONE, node.head),
            LinkOrder::Tail => (node.tail, NONE),
        };
        self.links[slot] = Link {
            id: Some(id),
            bounds,
            flags,
            order,
            node: index as u8,
            previous,
            next,
        };
        if previous == NONE {
            node.head = slot as u32;
        } else {
            self.links[previous as usize].next = slot as u32;
        }
        if next == NONE {
            node.tail = slot as u32;
        } else {
            self.links[next as usize].previous = slot as u32;
        }
        self.relinks += 1;
        true
    }

    /// C body.c:644-695: insertion order, parent first, captured follow modes.
    /// No module callback runs inside this commit, so a slot's lifetime cannot
    /// change while its shared StampSet mark is in use. Attach rejects cycles.
    pub fn transport_attachments(&mut self, table: &mut EntityTable) -> AttachmentTransport {
        self.transport_attachment_bodies(AttachmentBodies::Authoritative(table))
    }

    /// Current-command prediction uses the same graph over separately owned
    /// client poses. Unmapped intermediate anchors keep computed positions in
    /// load-sized scratch; the physical entity columns and area links stay frozen.
    pub fn predict_attachments(
        &mut self,
        table: &EntityTable,
        poses: &mut [Option<(EntityId, Vec3)>],
    ) -> AttachmentTransport {
        self.transport_attachment_bodies(AttachmentBodies::Predicted { table, poses })
    }

    fn transport_attachment_bodies(
        &mut self,
        mut bodies: AttachmentBodies<'_>,
    ) -> AttachmentTransport {
        let mut result = AttachmentTransport::default();
        if bodies.table().attachment_count() == 0 {
            return result;
        }
        if bodies.table().capacity() != self.links.len() {
            result.rejected = bodies.table().attachment_count() as u32;
            return result;
        }
        let predicted = matches!(&bodies, AttachmentBodies::Predicted { .. });
        self.attachment_visited.begin();
        for index in 0..bodies.table().attachment_count() {
            let mut id = bodies.table().attachment_at(index);
            while let Some(follow) = bodies.table().attachment(id) {
                if self.attachment_visited.contains(id.slot as usize) {
                    break;
                }
                self.attachment_chain.push((id, follow));
                id = follow.anchor;
            }
            while let Some((id, follow)) = self.attachment_chain.pop() {
                self.attachment_visited.mark(id.slot as usize);
                let Some(slot) = bodies.table().resolve(id) else {
                    continue;
                };
                let Some(anchor) = bodies.table().resolve(follow.anchor) else {
                    continue;
                };
                result.visited += 1;
                let current = bodies.position(id, slot);
                if predicted {
                    self.attachment_positions[slot] = current;
                }
                let anchor_position = if predicted && self.attachment_visited.contains(anchor) {
                    self.attachment_positions[anchor]
                } else {
                    bodies.position(follow.anchor, anchor)
                };
                let columns = &bodies.table().columns;
                let position = Vec3(std::array::from_fn(|axis| {
                    let offset = match follow.follow {
                        BodyFollow::Translation => follow.offset.0[axis],
                        BodyFollow::Center => {
                            (columns.mins[anchor].0[axis] + columns.maxs[anchor].0[axis]) * 0.5
                        }
                        BodyFollow::BoundsMin => {
                            columns.mins[anchor].0[axis] + follow.offset.0[axis]
                        }
                    };
                    anchor_position.0[axis] + offset
                }));
                if !position.0.iter().all(|value| value.is_finite()) {
                    result.rejected += 1;
                    continue;
                }
                let unchanged = (0..3).all(|axis| {
                    let left = current.0[axis];
                    let right = position.0[axis];
                    left == right
                        && (left != 0.0 || left.is_sign_negative() == right.is_sign_negative())
                });
                if unchanged {
                    continue;
                }
                if predicted {
                    self.attachment_positions[slot] = position;
                }
                bodies.set_position(id, slot, position);
                result.moved += 1;
                let link = self.links[slot];
                if !predicted
                    && link.id == Some(id)
                    && self.link(
                        bodies.table(),
                        id,
                        link.flags,
                        link.order,
                        LinkIntent::Explicit,
                    )
                {
                    result.relinked += 1;
                }
            }
        }
        result
    }

    pub fn query<'a>(
        &'a self,
        table: &'a EntityTable,
        bounds: Bounds,
        flags: LinkFlags,
    ) -> impl Iterator<Item = LinkedEntity<'a>> + 'a {
        Query {
            grid: self,
            bounds,
            flags,
            pending: [0; 31],
            count: 1,
            current: NONE,
        }
        .filter_map(move |id| {
            let slot = table.resolve(id)?;
            let columns = &table.columns;
            Some(LinkedEntity {
                id,
                position: &columns.position[slot],
                velocity: &columns.velocity[slot],
                mins: &columns.mins[slot],
                maxs: &columns.maxs[slot],
                flags: self.links[slot].flags,
            })
        })
    }
}

struct Query<'a> {
    grid: &'a AreaGrid,
    bounds: Bounds,
    flags: LinkFlags,
    pending: [u8; 31],
    count: usize,
    current: u32,
}
impl Iterator for Query<'_> {
    type Item = EntityId;
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            while self.current != NONE {
                let slot = self.current;
                let link = self.grid.links[slot as usize];
                self.current = link.next;
                if link.flags.0
                    & self.flags.0
                    & (LinkFlags::SOLID.0 | LinkFlags::TRIGGER.0 | LinkFlags::LINKED.0)
                    != 0
                    && self.bounds.overlaps(link.bounds)
                {
                    return link.id;
                }
            }
            if self.count == 0 {
                return None;
            }
            self.count -= 1;
            let node = self.grid.nodes[self.pending[self.count] as usize];
            if let Some(axis) = node.axis {
                // Native area queries visit front then back, strictly across splits.
                if self.bounds.mins.0[axis] < node.distance {
                    self.pending[self.count] = node.children[1];
                    self.count += 1;
                }
                if self.bounds.maxs.0[axis] > node.distance {
                    self.pending[self.count] = node.children[0];
                    self.count += 1;
                }
            }
            self.current = node.head;
        }
    }
}
