//! `checkclient` client visibility host and the `PF_aim` aim binding.
//!
//! Ported from donor `src/compat/qc/client-host.ts` (`QcClientHost`,
//! `createQcAimBinding`).
//!
//! Local mirrors: [`VisibilityScene`] mirrors the `Q1ClientVisibilityScene`
//! leaf queries behind `Q1ClientVisibility` in
//! `src/world/gameplay/q1-client-visibility.ts`, reduced to a line-of-sight
//! predicate; [`AimScene`] mirrors the trace callback behind `aimQ1` in
//! `src/world/gameplay/q1-aim.ts`. Entity words live in
//! [`FieldTable`](crate::fields::FieldTable).

use qa_core::identity::ActorId;
use qa_core::math::Vec3;

use super::actor_state::FLAG_NOTARGET;
use crate::error::GuestError;
use crate::fields::FieldTable;

/// Reserved slot registry behind the client host.
pub trait ClientSlots {
    /// Current entity row count.
    fn entity_count(&self) -> usize;
    /// Actor bound to a slot, if any.
    fn at(&self, slot: usize) -> Option<ActorId>;
    /// Whether an actor is live.
    fn is_live(&self, actor: &ActorId) -> bool;
    /// Whether a slot is free.
    fn is_free(&self, slot: usize) -> bool;
}

/// Line-of-sight scene behind client visibility checks.
pub trait VisibilityScene {
    /// Whether `to` is visible from `from`.
    fn visible(&self, from: Vec3, to: Vec3) -> bool;
}

/// One reserved client row projected from entity fields.
#[derive(Debug, Clone, PartialEq)]
pub struct ClientRow {
    /// Bound actor, if any.
    pub actor: Option<ActorId>,
    /// Whether the row is unusable.
    pub free: bool,
    /// Client health.
    pub health: f32,
    /// `FL_NOTARGET` flag.
    pub notarget: bool,
    /// Client origin.
    pub origin: Vec3,
    /// View offset added to the origin.
    pub view_offset: Vec3,
}

/// QC owns the fields and lifetimes; the shared policy owns client
/// rotation and visibility checks.
pub struct QcClientHost<S> {
    scene: S,
    max_clients: usize,
    last_checked: usize,
}

impl<S: VisibilityScene> QcClientHost<S> {
    /// Build the host, validating the reserved client range exactly like
    /// the donor constructor.
    pub fn new<C: ClientSlots>(slots: &C, scene: S, max_clients: usize) -> Result<Self, GuestError> {
        if max_clients < 1 || max_clients >= slots.entity_count() {
            return Err(GuestError::invalid("invalid reserved QC client range"));
        }
        for slot in 0..=max_clients {
            if slots.at(slot).is_none() {
                return Err(GuestError::invalid(format!(
                    "missing reserved QC client/world slot {slot}"
                )));
            }
        }
        Ok(Self {
            scene,
            max_clients,
            last_checked: 0,
        })
    }

    /// Maximum client slot.
    #[must_use]
    pub const fn max_clients(&self) -> usize {
        self.max_clients
    }

    /// Project one reserved client row.
    pub fn client_row<C: ClientSlots>(
        &self,
        fields: &FieldTable,
        slots: &C,
        slot: usize,
    ) -> Result<ClientRow, GuestError> {
        let actor = slots.at(slot);
        let words = fields;
        let (health, notarget, origin, view_offset) = match &actor {
            Some(actor) => (
                words.get(actor, "health")?.as_float("health")?,
                words.get(actor, "flags")?.as_float("flags")? as i32 & FLAG_NOTARGET != 0,
                words.get(actor, "origin")?.as_vector("origin")?,
                words.get(actor, "view_ofs")?.as_vector("view_ofs")?,
            ),
            None => (
                0.0,
                false,
                Vec3 { x: 0.0, y: 0.0, z: 0.0 },
                Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            ),
        };
        let free = slots.is_free(slot) || actor.as_ref().is_none_or(|actor| !slots.is_live(actor));
        Ok(ClientRow {
            actor,
            free,
            health,
            notarget,
            origin,
            view_offset,
        })
    }

    /// `checkclient`: cycle to the next visible, damageable client seen
    /// from `eye`. Returns the client actor, if any.
    pub fn check_client<C: ClientSlots>(
        &mut self,
        fields: &FieldTable,
        slots: &C,
        eye: Vec3,
    ) -> Result<Option<ActorId>, GuestError> {
        for _ in 0..self.max_clients {
            self.last_checked = (self.last_checked % self.max_clients) + 1;
            let row = self.client_row(fields, slots, self.last_checked)?;
            let Some(actor) = row.actor else {
                continue;
            };
            if row.free || row.health <= 0.0 || row.notarget {
                continue;
            }
            let target = Vec3 {
                x: row.origin.x + row.view_offset.x,
                y: row.origin.y + row.view_offset.y,
                z: row.origin.z + row.view_offset.z,
            };
            if self.scene.visible(eye, target) {
                return Ok(Some(actor));
            }
        }
        Ok(None)
    }
}

/// Scene behind aim target selection.
pub trait AimScene {
    /// Trace a hitscan ray; returns the first actor hit, if any.
    fn trace_hit_actor(&self, start: Vec3, end: Vec3, pass: &ActorId) -> Option<ActorId>;
    /// Read an actor's origin.
    fn body_origin(&self, actor: &ActorId) -> Option<Vec3>;
}

/// Projected aim target in native source-slot order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AimTarget {
    /// Target actor.
    pub actor: ActorId,
    /// Target entity reference.
    pub reference: i32,
}

/// Aim tuning services.
#[derive(Debug, Clone)]
pub struct AimServices {
    /// Minimum aim deflection score.
    pub aim_threshold: f32,
    /// Teamplay mode (0 disables team filtering).
    pub teamplay: f32,
    /// Candidate targets.
    pub targets: Vec<AimTarget>,
}

/// `PF_aim` request: shooter, aim direction, and the unused speed word.
#[derive(Debug, Clone, PartialEq)]
pub struct AimRequest {
    /// Shooting actor.
    pub shooter: ActorId,
    /// Forward direction.
    pub forward: Vec3,
    /// Speed word, read but never used (donor parity).
    pub speed: f32,
}

/// `PF_aim`: select the best deflect target along the request forward, or
/// return it unchanged when no target qualifies.
pub fn qc_aim<A: AimScene>(
    scene: &A,
    fields: &FieldTable,
    request: &AimRequest,
    services: &AimServices,
    no_aim: &dyn Fn(&ActorId) -> bool,
) -> Result<Vec3, GuestError> {
    let AimRequest {
        shooter,
        forward,
        speed: _,
    } = request;
    if no_aim(shooter) {
        return Ok(*forward);
    }
    for name in ["team", "takedamage"] {
        if fields.get(shooter, name).is_err() {
            return Err(GuestError::invalid(format!("Missing QC aim float field {name}")));
        }
    }
    let Some(origin) = scene.body_origin(shooter) else {
        return Err(GuestError::invalid("QC aim actor has no body"));
    };
    let shooter_team = fields.get(shooter, "team")?.as_float("team")?;
    let mut best: Option<(f32, Vec3)> = None;
    for target in &services.targets {
        if target.actor == *shooter {
            continue;
        }
        if fields.get(&target.actor, "takedamage")?.as_float("takedamage")? != 2.0 {
            continue;
        }
        if services.teamplay != 0.0 && shooter_team > 0.0 {
            let team = fields.get(&target.actor, "team")?.as_float("team")?;
            if (team - shooter_team).abs() < f32::EPSILON {
                continue;
            }
        }
        let Some(target_origin) = scene.body_origin(&target.actor) else {
            continue;
        };
        let delta = Vec3 {
            x: target_origin.x - origin.x,
            y: target_origin.y - origin.y,
            z: target_origin.z - origin.z,
        };
        let length = delta.x.hypot(delta.y).hypot(delta.z);
        if length == 0.0 {
            continue;
        }
        let direction = Vec3 {
            x: delta.x / length,
            y: delta.y / length,
            z: delta.z / length,
        };
        let score = forward.x * direction.x + forward.y * direction.y + forward.z * direction.z;
        if score < services.aim_threshold {
            continue;
        }
        // The shared Q1 policy only aims at targets the trace can reach.
        let hit = scene.trace_hit_actor(origin, target_origin, shooter);
        if hit.as_ref() != Some(&target.actor) {
            continue;
        }
        if best.is_none_or(|(best_score, _)| score > best_score) {
            best = Some((score, direction));
        }
    }
    Ok(best.map(|(_, direction)| direction).unwrap_or(*forward))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fields::{FieldLayout, FieldValue};
    use qa_core::identity::IdentityOwner;
    use qa_core::math::vec3;

    struct FakeSlots {
        actors: Vec<Option<ActorId>>,
    }

    impl ClientSlots for FakeSlots {
        fn entity_count(&self) -> usize {
            self.actors.len()
        }

        fn at(&self, slot: usize) -> Option<ActorId> {
            self.actors.get(slot).and_then(Clone::clone)
        }

        fn is_live(&self, actor: &ActorId) -> bool {
            self.actors.iter().any(|entry| entry.as_ref() == Some(actor))
        }

        fn is_free(&self, slot: usize) -> bool {
            self.at(slot).is_none()
        }
    }

    struct FakeScene {
        visible: bool,
    }

    impl VisibilityScene for FakeScene {
        fn visible(&self, _from: Vec3, _to: Vec3) -> bool {
            self.visible
        }
    }

    struct FakeAim {
        origins: Vec<(ActorId, Vec3)>,
        hit: Option<ActorId>,
    }

    impl AimScene for FakeAim {
        fn trace_hit_actor(&self, _start: Vec3, _end: Vec3, _pass: &ActorId) -> Option<ActorId> {
            self.hit.clone()
        }

        fn body_origin(&self, actor: &ActorId) -> Option<Vec3> {
            self.origins
                .iter()
                .find(|(id, _)| id == actor)
                .map(|(_, origin)| *origin)
        }
    }

    fn layout() -> FieldLayout {
        FieldLayout::qc_entity()
            .field("flags", "float")
            .field("health", "float")
            .field("view_ofs", "vector")
            .field("team", "float")
    }

    fn harness() -> (IdentityOwner, FieldTable, FakeSlots) {
        let owner = IdentityOwner::create("client-host").unwrap();
        let mut fields = FieldTable::new();
        let mut actors = Vec::new();
        for slot in 0..4 {
            let actor = owner.actor(slot, 1);
            fields.allocate(&actor, &layout()).unwrap();
            actors.push(Some(actor));
        }
        let one = actors[1].clone().unwrap();
        fields.set(&one, "health", FieldValue::Float(100.0)).unwrap();
        fields
            .set(&one, "origin", FieldValue::Vector(vec3(64.0, 0.0, 0.0)))
            .unwrap();
        fields
            .set(&one, "view_ofs", FieldValue::Vector(vec3(0.0, 0.0, 22.0)))
            .unwrap();
        let two = actors[2].clone().unwrap();
        fields.set(&two, "health", FieldValue::Float(100.0)).unwrap();
        (owner, fields, FakeSlots { actors })
    }

    #[test]
    fn constructor_validates_reserved_range() {
        let (_owner, _fields, slots) = harness();
        assert_eq!(
            QcClientHost::new(&slots, FakeScene { visible: true }, 2)
                .unwrap()
                .max_clients(),
            2
        );
        assert!(QcClientHost::new(&slots, FakeScene { visible: true }, 0).is_err());
        assert!(QcClientHost::new(&slots, FakeScene { visible: true }, 4).is_err());
        let sparse = FakeSlots {
            actors: vec![None, None],
        };
        assert!(QcClientHost::new(&sparse, FakeScene { visible: true }, 1).is_err());
    }

    #[test]
    fn check_client_cycles_visible_clients() {
        let (owner, fields, slots) = harness();
        let mut host = QcClientHost::new(&slots, FakeScene { visible: true }, 2).unwrap();
        let eye = vec3(0.0, 0.0, 22.0);
        assert_eq!(
            host.check_client(&fields, &slots, eye).unwrap(),
            Some(owner.actor(1, 1))
        );
        assert_eq!(
            host.check_client(&fields, &slots, eye).unwrap(),
            Some(owner.actor(2, 1))
        );
        assert_eq!(
            host.check_client(&fields, &slots, eye).unwrap(),
            Some(owner.actor(1, 1))
        );
    }

    #[test]
    fn check_client_skips_dead_notarget_and_hidden() {
        let (owner, mut fields, slots) = harness();
        let one = owner.actor(1, 1);
        fields.set(&one, "flags", FieldValue::Float(128.0)).unwrap();
        let two = owner.actor(2, 1);
        fields.set(&two, "health", FieldValue::Float(0.0)).unwrap();
        let mut host = QcClientHost::new(&slots, FakeScene { visible: true }, 2).unwrap();
        assert_eq!(host.check_client(&fields, &slots, vec3(0.0, 0.0, 0.0)).unwrap(), None);
        let (_owner, fields, slots) = harness();
        let mut host = QcClientHost::new(&slots, FakeScene { visible: false }, 2).unwrap();
        assert_eq!(host.check_client(&fields, &slots, vec3(0.0, 0.0, 0.0)).unwrap(), None);
    }

    #[test]
    fn aim_selects_best_traceable_target() {
        let (owner, mut fields, _slots) = harness();
        let (shooter, target) = (owner.actor(1, 1), owner.actor(2, 1));
        fields.set(&target, "takedamage", FieldValue::Float(2.0)).unwrap();
        fields.set(&shooter, "team", FieldValue::Float(1.0)).unwrap();
        fields.set(&target, "team", FieldValue::Float(2.0)).unwrap();
        let scene = FakeAim {
            origins: vec![
                (shooter.clone(), vec3(0.0, 0.0, 0.0)),
                (target.clone(), vec3(100.0, 0.0, 0.0)),
            ],
            hit: Some(target.clone()),
        };
        let services = AimServices {
            aim_threshold: 0.9,
            teamplay: 1.0,
            targets: vec![AimTarget {
                actor: target.clone(),
                reference: 2,
            }],
        };
        let none = |_: &ActorId| false;
        let request = AimRequest {
            shooter: shooter.clone(),
            forward: vec3(1.0, 0.0, 0.0),
            speed: 1000.0,
        };
        let aimed = qc_aim(&scene, &fields, &request, &services, &none).unwrap();
        assert_eq!(aimed, vec3(1.0, 0.0, 0.0));
        // Same-team targets are skipped under teamplay.
        fields.set(&target, "team", FieldValue::Float(1.0)).unwrap();
        let request = AimRequest {
            shooter: shooter.clone(),
            forward: vec3(0.0, 1.0, 0.0),
            speed: 1000.0,
        };
        let forward = qc_aim(&scene, &fields, &request, &services, &none).unwrap();
        assert_eq!(forward, vec3(0.0, 1.0, 0.0));
    }

    #[test]
    fn aim_honors_no_aim_and_trace_blocks() {
        let (owner, mut fields, _slots) = harness();
        let (shooter, target) = (owner.actor(1, 1), owner.actor(2, 1));
        fields.set(&target, "takedamage", FieldValue::Float(2.0)).unwrap();
        let blocked = FakeAim {
            origins: vec![
                (shooter.clone(), vec3(0.0, 0.0, 0.0)),
                (target.clone(), vec3(100.0, 0.0, 0.0)),
            ],
            hit: None,
        };
        let services = AimServices {
            aim_threshold: 0.5,
            teamplay: 0.0,
            targets: vec![AimTarget {
                actor: target,
                reference: 2,
            }],
        };
        let none = |_: &ActorId| false;
        let request = AimRequest {
            shooter: shooter.clone(),
            forward: vec3(1.0, 0.0, 0.0),
            speed: 500.0,
        };
        let forward = qc_aim(&blocked, &fields, &request, &services, &none).unwrap();
        assert_eq!(forward, vec3(1.0, 0.0, 0.0));
        let all = |_: &ActorId| true;
        let request = AimRequest {
            shooter: shooter.clone(),
            forward: vec3(0.0, 0.0, 1.0),
            speed: 500.0,
        };
        let forward = qc_aim(&blocked, &fields, &request, &services, &all).unwrap();
        assert_eq!(forward, vec3(0.0, 0.0, 1.0));
    }
}
