//! Sector membership ported from `src/world/spatial/index.ts` (Quake
//! `sv_world.c`, Quake III `sv_world.c`). One membership owner; queries
//! observe the most recent explicit link snapshot. Each area node carries
//! separate solid and trigger lists like `areanode_t` (`solid_edicts`,
//! `trigger_edicts`); visits are deterministic in sector order, never in
//! hash order.

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3};

use crate::body::LinkedBody;

/// Collision family of a linked actor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollisionFamily {
    /// Quake I hull collision.
    Q1,
    /// Quake II brush collision.
    Q2,
    /// Quake III brush collision.
    Q3,
}

/// Collision shape of a linked actor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollisionShape {
    /// Axis-aligned box.
    Box,
    /// Capsule.
    Capsule,
    /// Brush model index.
    Model(u32),
}

/// Query role of a linked actor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollisionRole {
    /// Blocks movement traces.
    Solid,
    /// Trigger volume.
    Trigger,
}

/// Q3 entity/owner numbers carried on a collision record (donor
/// `ActorCollision.q3Owner` from `src/world/collision/index.ts`).
/// Owner number `1023` means unowned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3OwnerRef {
    /// Entity number.
    pub entity_number: i32,
    /// Owner entity number.
    pub owner_number: i32,
}

/// Alias kept for call sites using the shorter name.
pub type Q3Owner = Q3OwnerRef;

/// Collision record attached to a linked body.
#[derive(Debug, Clone, PartialEq)]
pub struct ActorCollision {
    /// Collision family.
    pub family: CollisionFamily,
    /// Collision shape.
    pub shape: CollisionShape,
    /// Native contents flags.
    pub contents: i32,
    /// Owning actor for pass-through rules.
    pub owner: Option<ActorId>,
    /// Query role.
    pub role: CollisionRole,
    /// Monster flag for missile expansion rules.
    pub monster: bool,
    /// Dead-monster flag for contents mapping.
    pub dead_monster: bool,
    /// Rerelease corpse policy: point attacks hit; bodies pass through.
    pub q1_corpse: bool,
    /// Q3 entity/owner numbers, when the source collision carries them.
    pub q3_owner: Option<Q3OwnerRef>,
}

/// Linked body plus its collision record.
#[derive(Debug, Clone, PartialEq)]
pub struct SpatialActor {
    /// Linked body snapshot.
    pub body: LinkedBody,
    /// Collision record.
    pub collision: ActorCollision,
}

/// Query role filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryRole {
    /// Solid actors only.
    Solid,
    /// Trigger actors only.
    Trigger,
    /// Both roles.
    Both,
}

/// Visitor control for [`SpatialIndex::visit`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visit {
    /// Continue visiting.
    Continue,
    /// Skip the rest of this sector.
    StopSector,
    /// Stop the whole query.
    Stop,
}

/// Inclusive bounds intersection.
#[must_use]
pub fn bounds_intersect(first: &Bounds, second: &Bounds) -> bool {
    first.min.x <= second.max.x
        && first.min.y <= second.max.y
        && first.min.z <= second.max.z
        && first.max.x >= second.min.x
        && first.max.y >= second.min.y
        && first.max.z >= second.min.z
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Axis {
    X,
    Y,
}

impl Axis {
    fn get(self, value: &Vec3) -> f32 {
        match self {
            Axis::X => value.x,
            Axis::Y => value.y,
        }
    }
}

#[derive(Debug)]
struct Split {
    axis: Axis,
    distance: f32,
    front: Box<Sector>,
    back: Box<Sector>,
}

#[derive(Debug)]
struct Sector {
    split: Option<Split>,
    solid_members: Vec<SpatialActor>,
    trigger_members: Vec<SpatialActor>,
}

fn make_sector(bounds: &Bounds, depth: u32) -> Sector {
    if depth == 4 {
        return Sector {
            split: None,
            solid_members: Vec::new(),
            trigger_members: Vec::new(),
        };
    }
    let axis = if bounds.max.x - bounds.min.x > bounds.max.y - bounds.min.y {
        Axis::X
    } else {
        Axis::Y
    };
    let (min, max) = match axis {
        Axis::X => (bounds.min.x, bounds.max.x),
        Axis::Y => (bounds.min.y, bounds.max.y),
    };
    let distance = (min + max) * 0.5;
    let mut front_min = bounds.min;
    let mut back_max = bounds.max;
    match axis {
        Axis::X => {
            front_min.x = distance;
            back_max.x = distance;
        }
        Axis::Y => {
            front_min.y = distance;
            back_max.y = distance;
        }
    }
    Sector {
        split: Some(Split {
            axis,
            distance,
            front: Box::new(make_sector(
                &Bounds {
                    min: front_min,
                    max: bounds.max,
                },
                depth + 1,
            )),
            back: Box::new(make_sector(
                &Bounds {
                    min: bounds.min,
                    max: back_max,
                },
                depth + 1,
            )),
        }),
        solid_members: Vec::new(),
        trigger_members: Vec::new(),
    }
}

fn same_slot(actor: &ActorId, other: &ActorId) -> bool {
    actor == other
}

/// Sector-based spatial index.
#[derive(Debug)]
pub struct SpatialIndex {
    root: Sector,
}

impl SpatialIndex {
    /// Create an index over world bounds.
    #[must_use]
    pub fn new(bounds: &Bounds) -> Self {
        Self {
            root: make_sector(bounds, 0),
        }
    }

    /// Link a body snapshot into the node's solid or trigger list.
    /// Q3 members prepend; Q1/Q2 members append. Relinking moves the
    /// actor when its role changed.
    pub fn link(&mut self, body: &LinkedBody, collision: &ActorCollision) {
        self.unlink(&body.actor);
        self.link_fresh(body, collision);
    }

    /// Link a body snapshot known to be absent, skipping the unlink scan.
    ///
    /// The caller must guarantee the actor is not already linked (for
    /// example, the index was just cleared and every actor links once);
    /// linking a present actor a second time leaves a duplicate behind.
    /// Q3 members prepend; Q1/Q2 members append, exactly like [`Self::link`].
    pub fn link_fresh(&mut self, body: &LinkedBody, collision: &ActorCollision) {
        let actor = SpatialActor {
            body: body.clone(),
            collision: collision.clone(),
        };
        let mut sector = &mut self.root;
        let bounds = &body.absolute_bounds;
        while let Some(split) = sector.split.as_mut() {
            let axis = split.axis;
            if bounds.min.get(axis) > split.distance {
                sector = &mut split.front;
            } else if bounds.max.get(axis) < split.distance {
                sector = &mut split.back;
            } else {
                break;
            }
        }
        let members = match collision.role {
            CollisionRole::Solid => &mut sector.solid_members,
            CollisionRole::Trigger => &mut sector.trigger_members,
        };
        if collision.family == CollisionFamily::Q3 {
            members.insert(0, actor);
        } else {
            members.push(actor);
        }
    }

    /// Unlink an actor.
    pub fn unlink(&mut self, actor: &ActorId) {
        unlink_from(&mut self.root, actor);
    }

    /// Fetch a linked actor.
    #[must_use]
    pub fn get(&self, actor: &ActorId) -> Option<SpatialActor> {
        get_from(&self.root, actor)
    }

    /// Visit intersecting actors in sector order: solids then triggers
    /// at each node, front child then back. Within a list, Q1/Q2 visit
    /// in link order and Q3 in reverse link order.
    pub fn visit(&self, bounds: &Bounds, visit: &mut dyn FnMut(&SpatialActor) -> Visit) {
        walk(&self.root, bounds, visit);
    }

    /// Collect intersecting actors by role, in visit order.
    #[must_use]
    pub fn query(&self, bounds: &Bounds, role: QueryRole) -> Vec<SpatialActor> {
        let mut result = Vec::new();
        self.visit(bounds, &mut |actor| {
            let wanted = match role {
                QueryRole::Both => true,
                QueryRole::Solid => actor.collision.role == CollisionRole::Solid,
                QueryRole::Trigger => actor.collision.role == CollisionRole::Trigger,
            };
            if wanted {
                result.push(actor.clone());
            }
            Visit::Continue
        });
        result
    }

    /// Remove every member.
    pub fn clear(&mut self) {
        clear_sector(&mut self.root);
    }
}

trait AxisGet {
    fn get(&self, axis: Axis) -> f32;
}

impl AxisGet for Vec3 {
    fn get(&self, axis: Axis) -> f32 {
        axis.get(self)
    }
}

fn unlink_from(sector: &mut Sector, actor: &ActorId) {
    for members in [&mut sector.solid_members, &mut sector.trigger_members] {
        if let Some(index) = members.iter().position(|member| same_slot(&member.body.actor, actor)) {
            members.remove(index);
            return;
        }
    }
    if let Some(split) = sector.split.as_mut() {
        unlink_from(&mut split.front, actor);
        unlink_from(&mut split.back, actor);
    }
}

fn get_from(sector: &Sector, actor: &ActorId) -> Option<SpatialActor> {
    if let Some(member) = sector
        .solid_members
        .iter()
        .chain(sector.trigger_members.iter())
        .find(|member| same_slot(&member.body.actor, actor))
    {
        return Some(member.clone());
    }
    let split = sector.split.as_ref()?;
    get_from(&split.front, actor).or_else(|| get_from(&split.back, actor))
}

fn walk_list(members: &[SpatialActor], bounds: &Bounds, visit: &mut dyn FnMut(&SpatialActor) -> Visit) -> Option<bool> {
    for member in members {
        if bounds_intersect(&member.body.absolute_bounds, bounds) {
            match visit(member) {
                Visit::Stop => return Some(false),
                Visit::StopSector => return Some(true),
                Visit::Continue => {}
            }
        }
    }
    None
}

fn walk(sector: &Sector, bounds: &Bounds, visit: &mut dyn FnMut(&SpatialActor) -> Visit) -> bool {
    if let Some(done) = walk_list(&sector.solid_members, bounds, visit) {
        return done;
    }
    if let Some(done) = walk_list(&sector.trigger_members, bounds, visit) {
        return done;
    }
    if let Some(split) = sector.split.as_ref() {
        if bounds.max.get(split.axis) > split.distance && !walk(&split.front, bounds, visit) {
            return false;
        }
        if bounds.min.get(split.axis) < split.distance && !walk(&split.back, bounds, visit) {
            return false;
        }
    }
    true
}

fn clear_sector(sector: &mut Sector) {
    sector.solid_members.clear();
    sector.trigger_members.clear();
    if let Some(split) = sector.split.as_mut() {
        clear_sector(&mut split.front);
        clear_sector(&mut split.back);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::{IdentityOwner, ProviderId};
    use qa_core::math::{vec3, Bounds};

    use crate::body::BodyState;
    use crate::registry::ActorRegistry;

    fn world_bounds() -> Bounds {
        Bounds {
            min: vec3(-1024.0, -1024.0, -256.0),
            max: vec3(1024.0, 1024.0, 256.0),
        }
    }

    fn linked_at(
        registry: &mut ActorRegistry,
        x: f32,
        family: CollisionFamily,
        role: CollisionRole,
    ) -> (LinkedBody, ActorCollision) {
        let owner = ProviderId::new("q3", "game");
        let actor = registry.allocate(owner, "test:actor").unwrap();
        let state = BodyState {
            origin: vec3(x, 0.0, 0.0),
            angles: vec3(0.0, 0.0, 0.0),
            velocity: vec3(0.0, 0.0, 0.0),
            bounds: Bounds {
                min: vec3(-8.0, -8.0, -8.0),
                max: vec3(8.0, 8.0, 8.0),
            },
            ground: None,
        };
        let absolute_bounds = crate::body::translated_body_bounds(&state);
        (
            LinkedBody {
                actor: actor.id().clone(),
                state,
                absolute_bounds,
                link_count: 1,
            },
            ActorCollision {
                family,
                shape: CollisionShape::Box,
                contents: 1,
                owner: None,
                role,
                monster: false,
                dead_monster: false,
                q1_corpse: false,
                q3_owner: None,
            },
        )
    }

    #[test]
    fn q1_appends_and_q3_prepends_members() {
        let owner = IdentityOwner::create("test").unwrap();
        let mut registry = ActorRegistry::new(owner, 8).unwrap();
        let mut index = SpatialIndex::new(&world_bounds());
        let (first, first_collision) = linked_at(&mut registry, -500.0, CollisionFamily::Q1, CollisionRole::Solid);
        let (second, second_collision) = linked_at(&mut registry, 500.0, CollisionFamily::Q1, CollisionRole::Solid);
        index.link(&first, &first_collision);
        index.link(&second, &second_collision);
        let actors = index.query(&world_bounds(), QueryRole::Solid);
        assert_eq!(actors.len(), 2);

        let mut q3 = SpatialIndex::new(&world_bounds());
        let (third, third_collision) = linked_at(&mut registry, -100.0, CollisionFamily::Q3, CollisionRole::Solid);
        let (fourth, fourth_collision) = linked_at(&mut registry, -120.0, CollisionFamily::Q3, CollisionRole::Solid);
        q3.link(&third, &third_collision);
        q3.link(&fourth, &fourth_collision);
        let tight = Bounds {
            min: vec3(-200.0, -64.0, -64.0),
            max: vec3(0.0, 64.0, 64.0),
        };
        let actors = q3.query(&tight, QueryRole::Solid);
        assert_eq!(actors.len(), 2);
        assert_eq!(actors[0].body.actor, fourth.actor);
        assert_eq!(actors[1].body.actor, third.actor);
    }

    #[test]
    fn queries_filter_roles_and_unlink_removes() {
        let owner = IdentityOwner::create("test").unwrap();
        let mut registry = ActorRegistry::new(owner, 8).unwrap();
        let mut index = SpatialIndex::new(&world_bounds());
        let (solid, solid_collision) = linked_at(&mut registry, 0.0, CollisionFamily::Q2, CollisionRole::Solid);
        let (trigger, trigger_collision) = linked_at(&mut registry, 0.0, CollisionFamily::Q2, CollisionRole::Trigger);
        index.link(&solid, &solid_collision);
        index.link(&trigger, &trigger_collision);
        assert_eq!(index.query(&world_bounds(), QueryRole::Solid).len(), 1);
        assert_eq!(index.query(&world_bounds(), QueryRole::Trigger).len(), 1);
        index.unlink(&solid.actor);
        assert!(index.get(&solid.actor).is_none());
        assert_eq!(index.query(&world_bounds(), QueryRole::Both).len(), 1);
    }

    #[test]
    fn link_fresh_matches_link_for_absent_actors() {
        let owner = IdentityOwner::create("test").unwrap();
        let mut registry = ActorRegistry::new(owner, 16).unwrap();
        let mut index = SpatialIndex::new(&world_bounds());
        let mut entries = Vec::new();
        for (x, family) in [
            (-500.0, CollisionFamily::Q1),
            (500.0, CollisionFamily::Q1),
            (-100.0, CollisionFamily::Q3),
            (-120.0, CollisionFamily::Q3),
        ] {
            entries.push(linked_at(&mut registry, x, family, CollisionRole::Solid));
        }
        for (body, collision) in &entries {
            index.link(body, collision);
        }
        let linked: Vec<ActorId> = index
            .query(&world_bounds(), QueryRole::Solid)
            .iter()
            .map(|actor| actor.body.actor.clone())
            .collect();

        // Sweep-style rebuild: clear, then link every actor exactly once.
        index.clear();
        assert!(index.query(&world_bounds(), QueryRole::Both).is_empty());
        for (body, collision) in &entries {
            index.link_fresh(body, collision);
        }
        let rebuilt: Vec<ActorId> = index
            .query(&world_bounds(), QueryRole::Solid)
            .iter()
            .map(|actor| actor.body.actor.clone())
            .collect();
        assert_eq!(rebuilt, linked);
        for (body, _) in &entries {
            assert!(index.get(&body.actor).is_some());
        }
    }

    #[test]
    fn both_lists_visit_solids_first_in_link_order() {
        let owner = IdentityOwner::create("test").unwrap();
        let mut registry = ActorRegistry::new(owner, 16).unwrap();
        let mut index = SpatialIndex::new(&world_bounds());
        // Interleave roles at one spot so every member lands in one node.
        let (trigger_first, trigger_first_collision) =
            linked_at(&mut registry, 0.0, CollisionFamily::Q1, CollisionRole::Trigger);
        let (solid_first, solid_first_collision) =
            linked_at(&mut registry, 0.0, CollisionFamily::Q1, CollisionRole::Solid);
        let (trigger_second, trigger_second_collision) =
            linked_at(&mut registry, 0.0, CollisionFamily::Q1, CollisionRole::Trigger);
        let (solid_second, solid_second_collision) =
            linked_at(&mut registry, 0.0, CollisionFamily::Q1, CollisionRole::Solid);
        for (body, collision) in [
            (&trigger_first, &trigger_first_collision),
            (&solid_first, &solid_first_collision),
            (&trigger_second, &trigger_second_collision),
            (&solid_second, &solid_second_collision),
        ] {
            index.link(body, collision);
        }
        let both: Vec<ActorId> = index
            .query(&world_bounds(), QueryRole::Both)
            .iter()
            .map(|actor| actor.body.actor.clone())
            .collect();
        assert_eq!(
            both,
            vec![
                solid_first.actor.clone(),
                solid_second.actor.clone(),
                trigger_first.actor.clone(),
                trigger_second.actor.clone(),
            ]
        );
        // Relinking under a new role moves the actor without duplicating it.
        index.link(&solid_first, &trigger_first_collision);
        assert_eq!(index.query(&world_bounds(), QueryRole::Solid).len(), 1);
        assert_eq!(index.query(&world_bounds(), QueryRole::Trigger).len(), 3);
        assert_eq!(index.query(&world_bounds(), QueryRole::Both).len(), 4);
        let moved = index.get(&solid_first.actor).expect("relinked actor");
        assert_eq!(moved.collision.role, CollisionRole::Trigger);
    }

    #[test]
    fn bounds_intersection_is_inclusive() {
        let first = Bounds {
            min: vec3(0.0, 0.0, 0.0),
            max: vec3(8.0, 8.0, 8.0),
        };
        let touching = Bounds {
            min: vec3(8.0, 8.0, 8.0),
            max: vec3(16.0, 16.0, 16.0),
        };
        let apart = Bounds {
            min: vec3(9.0, 0.0, 0.0),
            max: vec3(16.0, 16.0, 16.0),
        };
        assert!(bounds_intersect(&first, &touching));
        assert!(!bounds_intersect(&first, &apart));
    }
}
