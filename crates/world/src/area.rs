use crate::entities::{EntityTable, MAX_ENTITIES};
use qa_core::primitives::{Bounds, EntityId, Vec3};

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LinkOrder {
    Head,
    #[default]
    Tail,
}

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
        if (0..3).any(|axis| {
            !bounds.mins.0[axis].is_finite()
                || !bounds.maxs.0[axis].is_finite()
                || bounds.mins.0[axis] > bounds.maxs.0[axis]
        }) {
            return Err(AreaError::Bounds);
        }
        let mut nodes = [Node::default(); 31];
        create(&mut nodes, &mut 0, 0, bounds);
        Ok(Self {
            nodes,
            links: vec![Link::default(); capacity].into_boxed_slice(),
            relinks: 0,
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
        let bounds = Bounds {
            mins: Vec3(std::array::from_fn(|axis| {
                columns.position[slot].0[axis] + columns.mins[slot].0[axis] - expansion(axis)
            })),
            maxs: Vec3(std::array::from_fn(|axis| {
                columns.position[slot].0[axis] + columns.maxs[slot].0[axis] + expansion(axis)
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
