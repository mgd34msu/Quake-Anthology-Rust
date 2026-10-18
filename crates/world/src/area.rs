use crate::entities::{EntityTable, MAX_ENTITIES};
use qa_core::{
    primitives::{
        BodyAttachment, BodyFollow, Bounds, CollisionShape, EntityId, RotatedLinkBounds, Vec3,
    },
    stamps::StampSet,
};

const NONE: u32 = u32::MAX;
const AREA_DEPTH: u8 = 4;
const AREA_NODES: usize = (1 << (AREA_DEPTH + 1)) - 1;

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
    nodes: [Node; AREA_NODES],
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
            Self::Authoritative(table) => table.columns.transport_position(slot, position),
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

fn create(nodes: &mut [Node; AREA_NODES], next: &mut u8, depth: u8, bounds: Bounds) -> u8 {
    let index = *next;
    *next += 1;
    if depth == AREA_DEPTH {
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
        let mut nodes = [Node::default(); AREA_NODES];
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

    /// Move an existing body's spatial identity without relinking it. Native
    /// client backing can outlive its connection; its bounds and list position
    /// stay unchanged when the connection's common lifetime ends.
    pub fn transfer_link(&mut self, table: &EntityTable, from: EntityId, to: EntityId) -> bool {
        if table.resolve(from).is_none()
            || table.resolve(to).is_none()
            || to.slot == 0
            || from.slot == to.slot
            || self
                .links
                .get(to.slot as usize)
                .is_none_or(|link| link.id.is_some())
        {
            return false;
        }
        let Some(mut link) = self
            .links
            .get(from.slot as usize)
            .copied()
            .filter(|link| link.id == Some(from))
        else {
            return false;
        };
        let node = &mut self.nodes[link.node as usize];
        if link.previous == NONE {
            node.head = to.slot;
        } else {
            self.links[link.previous as usize].next = to.slot;
        }
        if link.next == NONE {
            node.tail = to.slot;
        } else {
            self.links[link.next as usize].previous = to.slot;
        }
        link.id = Some(to);
        self.links[to.slot as usize] = link;
        self.links[from.slot as usize] = Link::default();
        true
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
        if bodies.table().attachment_ids().is_empty() {
            return result;
        }
        if bodies.table().capacity() != self.links.len() {
            result.rejected = bodies.table().attachment_ids().len() as u32;
            return result;
        }
        let predicted = matches!(&bodies, AttachmentBodies::Predicted { .. });
        self.attachment_visited.begin();
        for index in 0..bodies.table().attachment_ids().len() {
            let mut id = bodies.table().attachment_ids()[index];
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
                let offset = match follow.follow {
                    BodyFollow::Translation => follow.offset,
                    BodyFollow::Center => {
                        qa_core::math::midpoint(columns.mins[anchor], columns.maxs[anchor])
                    }
                    BodyFollow::BoundsMin => columns.mins[anchor] + follow.offset,
                };
                let position = anchor_position + offset;
                if !position.is_finite() {
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
        self.query_ordered(table, bounds, flags, None)
    }

    /// The cursor owns traversal state only. Each next call releases both
    /// spatial and entity borrows before a caller executes module callbacks.
    pub fn query_cursor(&self, bounds: Bounds, flags: LinkFlags) -> AreaCursor {
        AreaCursor {
            bounds,
            flags,
            pending: [CursorNode::default(); AREA_DEPTH as usize + 1],
            count: 1,
            remaining: self.links.len(),
            interrupted: false,
        }
    }

    fn linked<'a>(&'a self, table: &'a EntityTable, id: EntityId) -> Option<LinkedEntity<'a>> {
        let slot = table.resolve(id)?;
        let link = self.links.get(slot).filter(|link| link.id == Some(id))?;
        let columns = &table.columns;
        Some(LinkedEntity {
            id,
            position: &columns.position[slot],
            velocity: &columns.velocity[slot],
            mins: &columns.mins[slot],
            maxs: &columns.maxs[slot],
            flags: link.flags,
        })
    }

    /// CLIENT clipping follows accepted native entity order rather than area
    /// node order. Both paths consult the same generation-checked links.
    pub fn query_ordered<'a>(
        &'a self,
        table: &'a EntityTable,
        bounds: Bounds,
        flags: LinkFlags,
        order: Option<&'a [EntityId]>,
    ) -> impl Iterator<Item = LinkedEntity<'a>> + 'a {
        let mut spatial = self.query_cursor(bounds, flags);
        let mut ordered = order.map(|order| order.iter());
        std::iter::from_fn(move || {
            if let Some(order) = &mut ordered {
                order.find_map(|&id| {
                    let Some(link) = self.links.get(id.slot as usize) else {
                        return None;
                    };
                    if link.id == Some(id)
                        && link.flags.0 & flags.0 != 0
                        && bounds.overlaps(link.bounds)
                    {
                        self.linked(table, id)
                    } else {
                        None
                    }
                })
            } else {
                spatial.next(self, table)
            }
        })
    }
}

#[derive(Clone, Copy, Default)]
enum CursorPhase {
    #[default]
    Start,
    List,
    Front,
    Back,
    Done,
}

#[derive(Clone, Copy, Default)]
struct CursorNode {
    node: u8,
    phase: CursorPhase,
    next: Option<CursorLink>,
}

#[derive(Clone, Copy)]
struct CursorLink {
    id: EntityId,
    live: bool,
}
impl CursorLink {
    fn at(grid: &AreaGrid, table: &EntityTable, slot: u32) -> Option<Self> {
        let id = grid.links.get(slot as usize)?.id?;
        Some(Self {
            id,
            live: table.resolve(id).is_some(),
        })
    }
}

pub struct AreaCursor {
    bounds: Bounds,
    flags: LinkFlags,
    pending: [CursorNode; AREA_DEPTH as usize + 1],
    count: usize,
    remaining: usize,
    interrupted: bool,
}
impl AreaCursor {
    /// Trigger callbacks can move their querying entity. Remaining overlap
    /// checks and each front/back descent use its newly linked bounds.
    pub fn set_bounds(&mut self, bounds: Bounds) {
        self.bounds = bounds;
    }

    /// A deleted/reused or moved pending link ends that node's list. Other
    /// nodes remain queryable. A capacity-sized visit budget bounds callbacks
    /// which repeatedly reinsert links; no stale lifetime is followed.
    pub fn interrupted(&self) -> bool {
        self.interrupted
    }

    pub fn next<'a>(
        &mut self,
        grid: &'a AreaGrid,
        table: &'a EntityTable,
    ) -> Option<LinkedEntity<'a>> {
        while self.count != 0 {
            let top = self.count - 1;
            let frame = self.pending[top];
            let node = grid.nodes[frame.node as usize];
            match frame.phase {
                CursorPhase::Start => {
                    self.pending[top].phase = CursorPhase::List;
                    self.pending[top].next = CursorLink::at(grid, table, node.head);
                }
                CursorPhase::List => {
                    let Some(next) = frame.next else {
                        self.pending[top].phase = CursorPhase::Front;
                        continue;
                    };
                    let id = next.id;
                    let Some(link) = grid.links.get(id.slot as usize).copied().filter(|link| {
                        link.id == Some(id)
                            && link.node == frame.node
                            && (!next.live || table.resolve(id).is_some())
                    }) else {
                        self.interrupted = true;
                        self.pending[top].phase = CursorPhase::Front;
                        continue;
                    };
                    if self.remaining == 0 {
                        self.interrupted = true;
                        self.count = 0;
                        return None;
                    }
                    self.remaining -= 1;
                    // Native SV_TouchLinks loads next before invoking touch.
                    self.pending[top].next = CursorLink::at(grid, table, link.next);
                    if link.flags.0
                        & self.flags.0
                        & (LinkFlags::SOLID.0 | LinkFlags::TRIGGER.0 | LinkFlags::LINKED.0)
                        != 0
                        && self.bounds.overlaps(link.bounds)
                        && table.resolve(id).is_some()
                    {
                        return grid.linked(table, id);
                    }
                }
                CursorPhase::Front | CursorPhase::Back => {
                    let front = matches!(frame.phase, CursorPhase::Front);
                    self.pending[top].phase = if front {
                        CursorPhase::Back
                    } else {
                        CursorPhase::Done
                    };
                    if let Some(axis) = node.axis
                        && if front {
                            self.bounds.maxs.0[axis] > node.distance
                        } else {
                            self.bounds.mins.0[axis] < node.distance
                        }
                    {
                        self.pending[self.count] = CursorNode {
                            node: node.children[usize::from(!front)],
                            ..CursorNode::default()
                        };
                        self.count += 1;
                    }
                }
                CursorPhase::Done => self.count -= 1,
            }
        }
        None
    }
}
