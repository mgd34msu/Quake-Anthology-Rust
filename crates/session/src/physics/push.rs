use super::{MotionWrite, PhysicsError};
use qa_core::{
    math,
    primitives::{CollisionOwner, ThinkTime, Vec3},
};
use qa_gameplay::rules::motion::PushRules;
use qa_world::entities::EntityReference;

#[derive(Clone, Copy, Debug)]
pub struct PushFrame {
    pub elapsed: f64,
    pub world_time: ThinkTime,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PushNumber {
    Motion(MotionWrite),
    LocalTime,
    NextThink,
    MoveType,
    Solid,
    Min(usize),
    Max(usize),
    AbsoluteMin(usize),
    AbsoluteMax(usize),
}

/// Raw native references keep their backing across free/reuse. Lifetime
/// references remain generation checked. No body/VM borrow survives a method.
pub trait PushHost {
    type Error: From<PhysicsError>;
    type Cursor;
    fn start_candidates(&self) -> Self::Cursor;
    fn next_candidate(
        &mut self,
        cursor: &mut Self::Cursor,
    ) -> Result<Option<CollisionOwner>, Self::Error>;
    fn free(&mut self, owner: CollisionOwner) -> Result<bool, Self::Error>;
    fn number(&mut self, owner: CollisionOwner, field: PushNumber) -> Result<f32, Self::Error>;
    fn store(
        &mut self,
        owner: CollisionOwner,
        field: PushNumber,
        value: f32,
    ) -> Result<(), Self::Error>;
    fn flags(&mut self, owner: CollisionOwner) -> Result<u32, Self::Error>;
    fn store_flags(&mut self, owner: CollisionOwner, value: u32) -> Result<(), Self::Error>;
    fn ground_owner(&mut self, owner: CollisionOwner) -> Result<CollisionOwner, Self::Error>;
    fn same_owner(&self, left: CollisionOwner, right: CollisionOwner) -> bool;
    fn test_position(&mut self, owner: CollisionOwner) -> Result<bool, Self::Error>;
    fn link(&mut self, owner: CollisionOwner, touch: bool) -> Result<(), Self::Error>;
    fn push_entity(&mut self, owner: CollisionOwner, displacement: Vec3)
    -> Result<(), Self::Error>;
    fn capture_blocker(
        &self,
        owner: CollisionOwner,
    ) -> Result<Option<EntityReference>, Self::Error>;
    fn blocked(
        &mut self,
        pusher: CollisionOwner,
        blocker: CollisionOwner,
        captured: Option<EntityReference>,
    ) -> Result<(), Self::Error>;
    /// Clear native deadline, prepare world time/self/other in source order,
    /// select the resulting current function and call it exactly once.
    fn think_once(
        &mut self,
        pusher: CollisionOwner,
        world_time: ThinkTime,
    ) -> Result<(), Self::Error>;
}

#[derive(Clone, Copy)]
struct PushUndoEntry {
    owner: CollisionOwner,
    origin: Vec3,
}
pub struct PushUndo {
    entries: Box<[Option<PushUndoEntry>]>,
    used: usize,
}
impl PushUndo {
    pub fn load(maximum_candidates: usize) -> Self {
        Self {
            entries: vec![None; maximum_candidates].into_boxed_slice(),
            used: 0,
        }
    }
    fn clear(&mut self) {
        self.entries[..self.used].fill(None);
        self.used = 0;
    }
    fn admit(&mut self, owner: CollisionOwner, origin: Vec3) -> Result<(), PhysicsError> {
        *self
            .entries
            .get_mut(self.used)
            .ok_or(PhysicsError::Capacity)? = Some(PushUndoEntry { owner, origin });
        self.used += 1;
        Ok(())
    }
    fn remove_last(&mut self) {
        if self.used != 0 {
            self.used -= 1;
            self.entries[self.used] = None;
        }
    }
}

fn vector<H: PushHost>(
    host: &mut H,
    owner: CollisionOwner,
    field: impl Fn(usize) -> PushNumber,
) -> Result<Vec3, H::Error> {
    let mut value = Vec3::default();
    for axis in 0..3 {
        value.0[axis] = host.number(owner, field(axis))?;
    }
    Ok(value)
}
fn origin(axis: usize) -> PushNumber {
    PushNumber::Motion(MotionWrite::Origin(axis))
}
fn velocity(axis: usize) -> PushNumber {
    PushNumber::Motion(MotionWrite::Velocity(axis))
}
fn set_origin<H: PushHost>(
    host: &mut H,
    owner: CollisionOwner,
    value: Vec3,
) -> Result<(), H::Error> {
    for axis in 0..3 {
        host.store(owner, origin(axis), value.0[axis])?;
    }
    Ok(())
}
fn shift_origin<H: PushHost>(
    host: &mut H,
    owner: CollisionOwner,
    movement: Vec3,
    subtract: bool,
) -> Result<(), H::Error> {
    for axis in 0..3 {
        let current = host.number(owner, origin(axis))?;
        let value = if subtract {
            current - movement.0[axis]
        } else {
            current + movement.0[axis]
        };
        host.store(owner, origin(axis), value)?;
    }
    Ok(())
}
fn nonsolid<H: PushHost, T>(
    host: &mut H,
    pusher: CollisionOwner,
    rules: &PushRules,
    call: impl FnOnce(&mut H) -> Result<T, H::Error>,
) -> Result<T, H::Error> {
    let result = host
        .store(pusher, PushNumber::Solid, rules.solid_not())
        .and_then(|()| call(host));
    let restored = host.store(pusher, PushNumber::Solid, rules.solid_bsp());
    result.and_then(|value| restored.map(|()| value))
}

/// Native local-time admission over the current provider's float cells and
/// double frame interval. It neither repeats thinks nor advances world time.
pub fn push_frame<H: PushHost>(
    host: &mut H,
    pusher: CollisionOwner,
    undo: &mut PushUndo,
    frame: PushFrame,
    rules: &PushRules,
) -> Result<(), H::Error> {
    undo.clear();
    let result = (|| {
        let old_time = host.number(pusher, PushNumber::LocalTime)?;
        let deadline = host.number(pusher, PushNumber::NextThink)?;
        let mut move_time = if f64::from(deadline)
            < f64::from(host.number(pusher, PushNumber::LocalTime)?) + frame.elapsed
        {
            deadline - host.number(pusher, PushNumber::LocalTime)?
        } else {
            frame.elapsed as f32
        };
        if move_time < 0.0 {
            move_time = 0.0;
        }
        if move_time != 0.0 {
            move_for_time(host, pusher, undo, move_time, rules)?;
        }
        if deadline > old_time && deadline <= host.number(pusher, PushNumber::LocalTime)? {
            let before = if rules.post_think_snap.is_some() {
                Some(vector(host, pusher, origin)?)
            } else {
                None
            };
            host.think_once(pusher, frame.world_time)?;
            if host.free(pusher)? {
                return Ok(());
            }
            if let (Some(before), Some(threshold)) = (before, rules.post_think_snap) {
                let movement = vector(host, pusher, origin)? - before;
                if math::length(movement) > threshold {
                    set_origin(host, pusher, before)?;
                    displace(host, pusher, undo, movement, None, rules)?;
                }
            }
        }
        Ok(())
    })();
    undo.clear();
    result
}

fn move_for_time<H: PushHost>(
    host: &mut H,
    pusher: CollisionOwner,
    undo: &mut PushUndo,
    time: f32,
    rules: &PushRules,
) -> Result<(), H::Error> {
    let mut stationary = true;
    for axis in 0..3 {
        if host.number(pusher, velocity(axis))? != 0.0 {
            stationary = false;
            break;
        }
    }
    if stationary {
        let value = host.number(pusher, PushNumber::LocalTime)? + time;
        return host.store(pusher, PushNumber::LocalTime, value);
    }
    let mut movement = Vec3::default();
    for axis in 0..3 {
        movement.0[axis] = host.number(pusher, velocity(axis))? * time;
    }
    let moved = displace(
        host,
        pusher,
        undo,
        movement,
        rules.time_before_move.then_some(time),
        rules,
    )?;
    if moved && !rules.time_before_move {
        let value = host.number(pusher, PushNumber::LocalTime)? + time;
        host.store(pusher, PushNumber::LocalTime, value)?;
    }
    Ok(())
}

fn displace<H: PushHost>(
    host: &mut H,
    pusher: CollisionOwner,
    undo: &mut PushUndo,
    movement: Vec3,
    advance: Option<f32>,
    rules: &PushRules,
) -> Result<bool, H::Error> {
    undo.clear();
    let mins = vector(host, pusher, PushNumber::AbsoluteMin)? + movement;
    let maxs = vector(host, pusher, PushNumber::AbsoluteMax)? + movement;
    let before = vector(host, pusher, origin)?;
    shift_origin(host, pusher, movement, false)?;
    if let Some(time) = advance {
        let value = host.number(pusher, PushNumber::LocalTime)? + time;
        host.store(pusher, PushNumber::LocalTime, value)?;
    }
    host.link(pusher, false)?;
    let mut cursor = host.start_candidates();
    while let Some(candidate) = host.next_candidate(&mut cursor)? {
        if host.free(candidate)? || !rules.candidate(host.number(candidate, PushNumber::MoveType)?)
        {
            continue;
        }
        if rules.test_preexisting_stuck
            && nonsolid(host, pusher, rules, |host| host.test_position(candidate))?
        {
            continue;
        }
        let standing = host.flags(candidate)? & rules.ground_flag != 0 && {
            let ground = host.ground_owner(candidate)?;
            host.same_owner(ground, pusher)
        };
        if !standing {
            let mut separated = false;
            for axis in 0..3 {
                if host.number(candidate, PushNumber::AbsoluteMin(axis))? >= maxs.0[axis] {
                    separated = true;
                    break;
                }
            }
            if separated {
                continue;
            }
            for axis in 0..3 {
                if host.number(candidate, PushNumber::AbsoluteMax(axis))? <= mins.0[axis] {
                    separated = true;
                    break;
                }
            }
            if separated || !host.test_position(candidate)? {
                continue;
            }
        }
        if undo.used == undo.entries.len() {
            return Err(PhysicsError::Capacity.into());
        }
        if rules.clear_ground_nonwalk
            && !rules.walking(host.number(candidate, PushNumber::MoveType)?)
        {
            let flags = host.flags(candidate)?;
            host.store_flags(candidate, flags & !rules.ground_flag)?;
        }
        let original = vector(host, candidate, origin)?;
        undo.admit(candidate, original)?;
        let captured = host.capture_blocker(candidate)?;
        if rules.traced_passengers {
            nonsolid(host, pusher, rules, |host| {
                host.push_entity(candidate, movement)
            })?;
        } else {
            shift_origin(host, candidate, movement, false)?;
        }
        if !host.test_position(candidate)? {
            if !rules.traced_passengers {
                host.link(candidate, false)?;
            }
            continue;
        }
        if rules.leave_behind {
            shift_origin(host, candidate, movement, true)?;
            if !host.test_position(candidate)? {
                undo.remove_last();
                continue;
            }
        }
        if host.number(candidate, PushNumber::Min(0))?
            == host.number(candidate, PushNumber::Max(0))?
        {
            if rules.relink_exceptions {
                host.link(candidate, false)?;
            }
            continue;
        }
        if rules.corpse(host.number(candidate, PushNumber::Solid)?) {
            host.store(candidate, PushNumber::Min(0), 0.0)?;
            host.store(candidate, PushNumber::Min(1), 0.0)?;
            for axis in 0..3 {
                let value = host.number(candidate, PushNumber::Min(axis))?;
                host.store(candidate, PushNumber::Max(axis), value)?;
            }
            if rules.relink_exceptions {
                host.link(candidate, false)?;
            }
            continue;
        }
        if rules.traced_passengers {
            set_origin(host, candidate, original)?;
            host.link(candidate, true)?;
        }
        set_origin(host, pusher, before)?;
        host.link(pusher, false)?;
        if let Some(time) = advance {
            let value = host.number(pusher, PushNumber::LocalTime)? - time;
            host.store(pusher, PushNumber::LocalTime, value)?;
        }
        host.blocked(pusher, candidate, captured)?;
        for index in 0..undo.used {
            let entry = undo.entries[index].ok_or(PhysicsError::Entity)?;
            set_origin(host, entry.owner, entry.origin)?;
            host.link(entry.owner, false)?;
        }
        return Ok(false);
    }
    Ok(true)
}
