//! Doors, plats, and pusher transactions ported from
//! `src/movement/q1/pusher.ts` (`SV_PushMove`). Movers travel between two
//! positions at a fixed speed with wait times; each step is one pusher
//! transaction where riders and overlapping entities are carried, solid
//! obstacles block (rolling positions back after the `blocked` callback
//! runs), and think timing uses local pusher time. Unsolid (`SOLID_NOT`)
//! pushers skip the transaction and move freely — stock never finds
//! anything inside them (`sv_phys.c:499`). Rotation is an explicit
//! extension; ordinary NetQuake PUSH is translational.

use qa_core::identity::ActorId;
use qa_core::math::{vec3, Vec3};

/// Mover kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoverKind {
    /// Door swinging between closed and open.
    Door,
    /// Button dipping in and returning (`func_button`).
    Button,
    /// Plat rising and falling.
    Plat,
    /// Generic velocity-driven pusher.
    Pusher,
}

/// Mover phase between its two endpoints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoverPhase {
    /// Resting at position 1.
    AtPos1,
    /// Resting at position 2.
    AtPos2,
    /// Travelling toward position 2.
    ToPos2,
    /// Travelling toward position 1.
    ToPos1,
}

/// Pusher transaction outcome (`Q1PusherResult.status`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PusherOutcome {
    /// Moved (possibly carrying entities).
    Moved,
    /// Blocked by a solid obstacle; positions rolled back.
    Blocked,
    /// Pusher removed mid-transaction.
    ActorRemoved,
}

/// Mover state for one actor.
#[derive(Debug, Clone, PartialEq)]
pub struct MoverState {
    /// Mover kind.
    pub kind: MoverKind,
    /// Current phase.
    pub phase: MoverPhase,
    /// Position 1 (closed/down).
    pub pos1: Vec3,
    /// Position 2 (open/up).
    pub pos2: Vec3,
    /// Travel speed in units per second.
    pub speed: f64,
    /// Wait at position 2 before the wait think fires (-1 waits forever).
    pub wait_seconds: f64,
    /// Local pusher time in seconds.
    pub local_time_seconds: f64,
    /// Next think in local pusher time (0 = none).
    pub next_think_seconds: f64,
    /// Whether the pusher is solid: unsolid (`SOLID_NOT`) pushers move
    /// freely, carrying nothing and blocked by nothing.
    pub solid: bool,
}

impl MoverState {
    /// New solid mover resting at position 1.
    #[must_use]
    pub const fn new(kind: MoverKind, pos1: Vec3, pos2: Vec3, speed: f64, wait_seconds: f64) -> Self {
        Self {
            kind,
            phase: MoverPhase::AtPos1,
            pos1,
            pos2,
            speed,
            wait_seconds,
            local_time_seconds: 0.0,
            next_think_seconds: 0.0,
            solid: true,
        }
    }

    /// Current endpoint target for the active phase.
    #[must_use]
    pub const fn target(&self) -> Vec3 {
        match self.phase {
            MoverPhase::AtPos1 | MoverPhase::ToPos1 => self.pos1,
            MoverPhase::AtPos2 | MoverPhase::ToPos2 => self.pos2,
        }
    }
}

/// One mover step: displacement for this frame plus arrival state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MoverStep {
    /// Displacement to apply this frame.
    pub displacement: Vec3,
    /// Whether the endpoint was reached.
    pub arrived: bool,
    /// Think due after this step (local pusher time crossed).
    pub think_due: bool,
}

/// Advance local pusher time and compute this frame's displacement,
/// following `stepQ1Pusher` exactly: the move clips at the think instant,
/// local time advances only by the moved time, and a think fires when its
/// local time is crossed. With no future think (`next_think <= local`)
/// the pusher holds still and local time does not advance — the donor
/// test notes a stopped pusher advances "when a future think exists".
pub fn step_mover(state: &mut MoverState, origin: Vec3, elapsed_seconds: f64) -> MoverStep {
    debug_assert!(elapsed_seconds >= 0.0);
    let old_time = state.local_time_seconds;
    let think_time = state.next_think_seconds;
    let move_time = if think_time < old_time + elapsed_seconds {
        (think_time - old_time).max(0.0)
    } else {
        elapsed_seconds
    };
    if move_time == 0.0 {
        return MoverStep {
            displacement: vec3(0.0, 0.0, 0.0),
            arrived: false,
            think_due: false,
        };
    }
    state.local_time_seconds = old_time + move_time;
    let think_due = think_time > old_time && think_time <= state.local_time_seconds;
    if think_due {
        state.next_think_seconds = 0.0;
    }
    match state.phase {
        MoverPhase::AtPos1 | MoverPhase::AtPos2 => MoverStep {
            displacement: vec3(0.0, 0.0, 0.0),
            arrived: false,
            think_due,
        },
        MoverPhase::ToPos1 | MoverPhase::ToPos2 => {
            let target = state.target();
            let delta = Vec3 {
                x: target.x - origin.x,
                y: target.y - origin.y,
                z: target.z - origin.z,
            };
            let distance = f64::from(delta.x * delta.x + delta.y * delta.y + delta.z * delta.z).sqrt();
            let travel = state.speed * move_time;
            if distance <= travel {
                state.phase = match state.phase {
                    MoverPhase::ToPos2 => MoverPhase::AtPos2,
                    _ => MoverPhase::AtPos1,
                };
                if state.phase == MoverPhase::AtPos2 && state.wait_seconds > 0.0 {
                    state.next_think_seconds = state.local_time_seconds + state.wait_seconds;
                }
                MoverStep {
                    displacement: delta,
                    arrived: true,
                    think_due,
                }
            } else {
                let scale = (travel / distance) as f32;
                MoverStep {
                    displacement: vec3(delta.x * scale, delta.y * scale, delta.z * scale),
                    arrived: false,
                    think_due,
                }
            }
        }
    }
}

/// Trigger a resting mover toward the other endpoint (doors toggle; plats
/// rise when at the bottom) and arm the arrival think, like QC door code
/// setting `nextthink` to the arrival time. Already-travelling movers are
/// untouched; a zero-distance trigger snaps to the endpoint.
pub fn use_mover(state: &mut MoverState, origin: Vec3) {
    state.phase = match state.phase {
        MoverPhase::AtPos1 => MoverPhase::ToPos2,
        MoverPhase::AtPos2 => MoverPhase::ToPos1,
        MoverPhase::ToPos1 | MoverPhase::ToPos2 => return,
    };
    let target = state.target();
    let delta = Vec3 {
        x: target.x - origin.x,
        y: target.y - origin.y,
        z: target.z - origin.z,
    };
    let distance = f64::from(delta.x * delta.x + delta.y * delta.y + delta.z * delta.z).sqrt();
    if distance == 0.0 {
        state.phase = match state.phase {
            MoverPhase::ToPos2 => MoverPhase::AtPos2,
            _ => MoverPhase::AtPos1,
        };
        return;
    }
    if state.speed > 0.0 {
        state.next_think_seconds = state.local_time_seconds + distance / state.speed;
    }
}

/// Candidate entity for a pusher transaction.
#[derive(Debug, Clone, PartialEq)]
pub struct PushCandidate {
    /// Candidate actor.
    pub actor: ActorId,
    /// Riding the pusher (grounded on it).
    pub rider: bool,
    /// Bounds overlap the swept pusher bounds.
    pub overlaps: bool,
    /// Q1 movement type (`Q1_MOVE_*`).
    pub move_type: i32,
    /// Zero-size (point) entity.
    pub point_sized: bool,
    /// Non-blocking solidity (`not`, `trigger`, `corpse`).
    pub soft_solid: bool,
}

/// Resolve one pusher transaction over ordered candidates, mirroring
/// `pushQ1Pusher`: PUSH/NONE/NOCLIP movetypes are skipped, riders are
/// always carried, others only on overlap. `push` carries one entity and
/// reports whether it ended in solid; `blocked` runs before rollback.
/// Returns the outcome plus carried actors in visit order.
pub fn push_transaction(
    candidates: &[PushCandidate],
    push: &mut dyn FnMut(&ActorId) -> bool,
    blocked: &mut dyn FnMut(&ActorId),
) -> (PusherOutcome, Vec<ActorId>) {
    const Q1_MOVE_NONE: i32 = 0;
    const Q1_MOVE_PUSH: i32 = 7;
    const Q1_MOVE_NOCLIP: i32 = 8;
    let mut moved = Vec::new();
    for candidate in candidates {
        if candidate.move_type == Q1_MOVE_PUSH
            || candidate.move_type == Q1_MOVE_NONE
            || candidate.move_type == Q1_MOVE_NOCLIP
        {
            continue;
        }
        if !candidate.rider && !candidate.overlaps {
            continue;
        }
        moved.push(candidate.actor.clone());
        if push(&candidate.actor) {
            continue;
        }
        if candidate.point_sized || candidate.soft_solid {
            continue;
        }
        blocked(&candidate.actor);
        return (PusherOutcome::Blocked, moved);
    }
    (PusherOutcome::Moved, moved)
}

/// Mover table keyed by actor.
#[derive(Debug, Clone, Default)]
pub struct MoverTable {
    movers: Vec<(ActorId, MoverState)>,
}

impl MoverTable {
    /// Empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or replace a mover.
    pub fn insert(&mut self, actor: ActorId, state: MoverState) {
        if let Some(slot) = self.movers.iter_mut().find(|(id, _)| *id == actor) {
            slot.1 = state;
        } else {
            self.movers.push((actor, state));
        }
    }

    /// Read a mover.
    #[must_use]
    pub fn get(&self, actor: &ActorId) -> Option<&MoverState> {
        self.movers.iter().find(|(id, _)| id == actor).map(|(_, state)| state)
    }

    /// Read a mover mutably.
    pub fn get_mut(&mut self, actor: &ActorId) -> Option<&mut MoverState> {
        self.movers
            .iter_mut()
            .find(|(id, _)| *id == *actor)
            .map(|(_, state)| state)
    }

    /// Remove a mover.
    pub fn remove(&mut self, actor: &ActorId) {
        self.movers.retain(|(id, _)| id != actor);
    }

    /// Checkpoint all movers.
    #[must_use]
    pub fn checkpoint(&self) -> Vec<((u32, u32), MoverState)> {
        self.movers
            .iter()
            .map(|(id, state)| ((id.slot(), id.generation()), state.clone()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn door() -> MoverState {
        MoverState::new(MoverKind::Door, vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 64.0), 64.0, 3.0)
    }

    #[test]
    fn mover_travels_at_speed_and_arms_wait_think() {
        let mut state = door();
        use_mover(&mut state, vec3(0.0, 0.0, 0.0));
        assert_eq!(state.phase, MoverPhase::ToPos2);
        assert_eq!(state.next_think_seconds, 1.0);
        let step = step_mover(&mut state, vec3(0.0, 0.0, 0.0), 0.5);
        assert!(!step.arrived);
        assert!(!step.think_due);
        assert_eq!(step.displacement, vec3(0.0, 0.0, 32.0));
        assert_eq!(state.local_time_seconds, 0.5);
        let step = step_mover(&mut state, vec3(0.0, 0.0, 32.0), 0.5);
        assert!(step.arrived);
        assert!(step.think_due);
        assert_eq!(state.phase, MoverPhase::AtPos2);
        assert_eq!(state.local_time_seconds, 1.0);
        assert_eq!(state.next_think_seconds, 4.0);
        let step = step_mover(&mut state, vec3(0.0, 0.0, 64.0), 3.0);
        assert!(!step.arrived);
        assert!(step.think_due);
        assert_eq!(step.displacement, vec3(0.0, 0.0, 0.0));
        assert_eq!(state.next_think_seconds, 0.0);
    }

    #[test]
    fn think_time_clips_move_and_fires_once() {
        let mut state = door();
        use_mover(&mut state, vec3(0.0, 0.0, 0.0));
        state.next_think_seconds = 0.25;
        let step = step_mover(&mut state, vec3(0.0, 0.0, 0.0), 0.5);
        assert!(step.think_due);
        assert!(!step.arrived);
        assert_eq!(step.displacement, vec3(0.0, 0.0, 16.0));
        assert_eq!(state.next_think_seconds, 0.0);
        assert_eq!(state.local_time_seconds, 0.25);
    }

    #[test]
    fn mover_without_future_think_holds_still() {
        let mut state = door();
        state.phase = MoverPhase::ToPos2;
        state.next_think_seconds = 0.0;
        let step = step_mover(&mut state, vec3(0.0, 0.0, 0.0), 0.5);
        assert_eq!(step.displacement, vec3(0.0, 0.0, 0.0));
        assert!(!step.think_due);
        assert_eq!(state.local_time_seconds, 0.0);
        assert_eq!(state.phase, MoverPhase::ToPos2);
    }

    #[test]
    fn push_transaction_skips_carries_and_blocks() {
        use qa_core::identity::{IdentityOwner, ProviderId};
        let owner = IdentityOwner::create("test").unwrap();
        let provider = ProviderId::new("q1", "game");
        let a = owner.actor(1, 1);
        let b = owner.actor(2, 1);
        let _ = provider;
        let candidates = vec![
            PushCandidate {
                actor: a.clone(),
                rider: false,
                overlaps: false,
                move_type: 3,
                point_sized: false,
                soft_solid: false,
            },
            PushCandidate {
                actor: b.clone(),
                rider: true,
                overlaps: false,
                move_type: 3,
                point_sized: false,
                soft_solid: false,
            },
        ];
        let mut blocked = Vec::new();
        let (outcome, moved) = push_transaction(&candidates, &mut |_| false, &mut |actor| blocked.push(actor.clone()));
        assert_eq!(outcome, PusherOutcome::Blocked);
        assert_eq!(moved, vec![b.clone()]);
        assert_eq!(blocked, vec![b]);
    }

    #[test]
    fn push_transaction_skips_pusher_movetypes() {
        use qa_core::identity::IdentityOwner;
        let owner = IdentityOwner::create("test").unwrap();
        let actor = owner.actor(1, 1);
        let candidates = vec![PushCandidate {
            actor,
            rider: true,
            overlaps: true,
            move_type: 7,
            point_sized: false,
            soft_solid: false,
        }];
        let (outcome, moved) = push_transaction(&candidates, &mut |_| false, &mut |_| {});
        assert_eq!(outcome, PusherOutcome::Moved);
        assert!(moved.is_empty());
    }

    #[test]
    fn mover_table_checkpoint_round_trip() {
        use qa_core::identity::IdentityOwner;
        let owner = IdentityOwner::create("test").unwrap();
        let actor = owner.actor(4, 2);
        let mut table = MoverTable::new();
        table.insert(actor.clone(), door());
        let saved = table.checkpoint();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].0, (4, 2));
        assert_eq!(table.get(&actor).unwrap().speed, 64.0);
    }
}
