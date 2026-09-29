//! Sector membership ported from `src/world/spatial/index.ts` (Quake
//! `sv_world.c`, Quake III `sv_world.c`). One membership owner; queries
//! observe the most recent explicit link snapshot.

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
    members: Vec<SpatialActor>,
}

fn make_sector(bounds: &Bounds, depth: u32) -> Sector {
    if depth == 4 {
        return Sector {
            split: None,
            members: Vec::new(),
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
        members: Vec::new(),
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

    /// Link a body snapshot. Q3 members prepend; Q1/Q2 members append.
    pub fn link(&mut self, body: &LinkedBody, collision: &ActorCollision) {
        self.unlink(&body.actor);
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
        if collision.family == CollisionFamily::Q3 {
            sector.members.insert(0, actor);
        } else {
            sector.members.push(actor);
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

    /// Visit intersecting actors in sector order.
    pub fn visit(&self, bounds: &Bounds, visit: &mut dyn FnMut(&SpatialActor) -> Visit) {
        walk(&self.root, bounds, visit);
    }

    /// Collect intersecting actors by role.
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
    if let Some(index) = sector
        .members
        .iter()
        .position(|member| same_slot(&member.body.actor, actor))
    {
        sector.members.remove(index);
        return;
    }
    if let Some(split) = sector.split.as_mut() {
        unlink_from(&mut split.front, actor);
        unlink_from(&mut split.back, actor);
    }
}

fn get_from(sector: &Sector, actor: &ActorId) -> Option<SpatialActor> {
    if let Some(member) = sector
        .members
        .iter()
        .find(|member| same_slot(&member.body.actor, actor))
    {
        return Some(member.clone());
    }
    let split = sector.split.as_ref()?;
    get_from(&split.front, actor).or_else(|| get_from(&split.back, actor))
}

fn walk(sector: &Sector, bounds: &Bounds, visit: &mut dyn FnMut(&SpatialActor) -> Visit) -> bool {
    for member in &sector.members {
        if bounds_intersect(&member.body.absolute_bounds, bounds) {
            match visit(member) {
                Visit::Stop => return false,
                Visit::StopSector => return true,
                Visit::Continue => {}
            }
        }
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
    sector.members.clear();
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
