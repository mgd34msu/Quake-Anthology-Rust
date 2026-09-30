//! Equipment movement: speed, pose, and body-shape projection.
//!
//! Provenance: `src/compat/qvm/game-equipment-movement.ts`.
//!
//! Async branches collapse to the sync path. The hub never executes registered
//! regions, so the locomotion body runs white-box through the call's recorded
//! bindings. Hook failures record through
//! [`QvmEquipmentMovement::take_error`].
//!
//! Local mirrors: the used surface of `src/contracts/movement.ts`
//! ([`QvmFixedPose`]), `src/movement/body-shape.ts` ([`QvmBodyShape`]), and the
//! button/move bytes of `src/compat/qvm/client-state-record.ts`
//! ([`qvm_command_layout`]). Movement scopes reuse
//! [`super::game_input::QvmApplicationScope`].
//!
//! [`QvmGameData`]: super::game_data::QvmGameData

use std::cell::RefCell;
use std::rc::Rc;

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3};

use super::game_data::{
    qualify_qvm_region, AbiProfile, QvmArtifact, QvmFunctionCall, QvmGameData, QvmHookFn, QvmModule, QvmOpcode,
    QvmRegionBinding, QvmRegionDecision,
};
use super::game_input::QvmApplicationScope;
use crate::error::GuestError;

/// Fixed movement pose.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QvmFixedPose {
    /// Whether crouched.
    pub crouched: bool,
    /// Collision bounds.
    pub bounds: Bounds,
    /// View height.
    pub view_height: i32,
}

/// Movement body shape.
#[derive(Clone)]
pub struct QvmBodyShape {
    /// Current bounds.
    pub current: Bounds,
    /// Requested bounds, if any.
    pub requested: Option<Bounds>,
    /// Assert the current actor.
    pub current_actor: Rc<dyn Fn()>,
}

/// Equipment motion.
#[derive(Clone)]
pub struct QvmEquipmentMotion {
    /// Speed multiplier.
    pub speed_multiplier: f64,
    /// Fixed pose, if any.
    pub pose: Option<QvmFixedPose>,
    /// Whether equipment owns holdable input.
    pub owns_holdable_input: bool,
    /// Body shape, if any.
    pub body: Option<QvmBodyShape>,
}

/// Equipment movement profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QvmEquipmentMovementProfile {
    /// Move entry.
    pub move_entry: usize,
    /// Slice entry.
    pub slice: usize,
    /// Duck entry.
    pub duck: usize,
    /// Movement global address.
    pub movement_global: usize,
    /// Locomotion region.
    pub locomotion: QvmLocomotion,
    /// Mins offset within the movement record.
    pub mins: usize,
    /// Maxs offset within the movement record.
    pub maxs: usize,
    /// Body-trace hook, if any.
    pub body_trace: Option<QvmBodyTrace>,
}

/// Locomotion region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmLocomotion {
    /// Region entry.
    pub entry: usize,
    /// Region join.
    pub join: usize,
}

/// Body-trace hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmBodyTrace {
    /// Callback offset within the movement record.
    pub callback: usize,
    /// Mask offset within the movement record.
    pub mask: usize,
}

/// Equipment services.
#[derive(Clone)]
pub struct QvmEquipmentServices {
    /// Canonical actor for a slot.
    pub actor: Rc<dyn Fn(usize) -> Option<ActorId>>,
    /// Whether an actor is live.
    pub live: Rc<dyn Fn(&ActorId) -> bool>,
    /// Equipment motion, if any.
    pub equipment: Rc<dyn Fn(&ActorId) -> Option<QvmEquipmentMotion>>,
}

/// User-command move/button layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QvmCommandLayout {
    /// Forward byte.
    pub forward: usize,
    /// Right byte.
    pub right: usize,
    /// Up byte.
    pub up: usize,
    /// Buttons offset.
    pub buttons_offset: usize,
    /// Buttons width (1 or 4).
    pub buttons_bytes: usize,
}

/// Command layout for an ABI profile.
#[must_use]
pub const fn qvm_command_layout(profile: AbiProfile) -> QvmCommandLayout {
    if profile.is_modern() {
        QvmCommandLayout {
            forward: 21,
            right: 22,
            up: 23,
            buttons_offset: 16,
            buttons_bytes: 4,
        }
    } else {
        QvmCommandLayout {
            forward: 20,
            right: 21,
            up: 22,
            buttons_offset: 4,
            buttons_bytes: 1,
        }
    }
}

/// Live movement frame.
#[derive(Clone)]
struct QvmMotionFrame {
    /// Frame ordinal.
    id: u64,
    /// Accepted bounds, if any.
    accepted_bounds: Option<Bounds>,
    /// Acting actor.
    actor: ActorId,
    /// Player address.
    player: usize,
    /// Movement address.
    movement: usize,
    /// Fixed pose, if any.
    pose: Option<QvmFixedPose>,
    /// Whether equipment owns holdable input.
    owns_holdable_input: bool,
    /// Body shape, if any.
    body: Option<QvmBodyShape>,
}

/// Shared equipment state behind hook closures.
struct QvmEquipmentInner {
    /// Source module.
    module: QvmModule,
    /// Located game data.
    data: QvmGameData,
    /// Movement profile.
    profile: QvmEquipmentMovementProfile,
    /// Host services.
    services: QvmEquipmentServices,
    /// Body-trace scratch address.
    body_scratch: usize,
    /// Frame stack (vacant slots admit no equipment).
    frames: RefCell<Vec<Option<QvmMotionFrame>>>,
    /// Next frame ordinal.
    next_frame: RefCell<u64>,
    /// Bound hook ids.
    hooks: RefCell<Vec<u64>>,
    /// First recorded hook failure.
    error: RefCell<Option<GuestError>>,
}

/// Whether an actor is live.
fn live(inner: &Rc<QvmEquipmentInner>, actor: &ActorId) -> bool {
    (inner.services.live)(actor)
}

/// Admit equipment for a movement invocation.
fn admit(inner: &Rc<QvmEquipmentInner>, call: &QvmFunctionCall) -> Result<Option<QvmMotionFrame>, GuestError> {
    let movement = call.argument(0)?;
    let movement_addr =
        usize::try_from(movement).map_err(|_| GuestError::invalid("Selected movement scope escapes its allocation"))?;
    let player = call.guest.read_i32(movement_addr)?;
    let located = inner.data.checkpoint();
    let slot = usize::try_from(player)
        .ok()
        .and_then(|player| player.checked_sub(located.clients_word))
        .filter(|offset| located.client_stride != 0 && offset % located.client_stride == 0)
        .map(|offset| offset / located.client_stride);
    let Some(slot) = slot else {
        return Ok(None);
    };
    if slot >= inner.data.num_clients() {
        return Ok(None);
    }
    let actor = (inner.services.actor)(slot);
    let Some(actor) = actor else {
        return Ok(None);
    };
    if !live(inner, &actor) {
        return Ok(None);
    }
    let equipment = (inner.services.equipment)(&actor);
    let Some(equipment) = equipment else {
        return Ok(None);
    };
    if !equipment.speed_multiplier.is_finite() || equipment.speed_multiplier <= 0.0 {
        return Err(GuestError::invalid(
            "Selected movement speed must be positive and finite",
        ));
    }
    let player_addr = usize::try_from(player)
        .map_err(|_| GuestError::invalid("Selected movement speed escapes its player record"))?;
    let address = player_addr
        .checked_add(52)
        .ok_or_else(|| GuestError::invalid("Selected movement speed escapes its player record"))?;
    let memory = inner.module.memory();
    let scaled = f64::from(memory.read_i32(address)?) * equipment.speed_multiplier;
    memory.write_i32(address, (scaled as f32).trunc().rem_euclid(4294967296.0) as i64 as i32)?;
    let mut next = inner.next_frame.borrow_mut();
    let id = *next;
    *next += 1;
    Ok(Some(QvmMotionFrame {
        id,
        accepted_bounds: None,
        actor,
        player: player_addr,
        movement: movement_addr,
        pose: equipment.pose,
        owns_holdable_input: equipment.owns_holdable_input,
        body: equipment.body,
    }))
}

/// Current live frame matching the movement global.
fn current(inner: &Rc<QvmEquipmentInner>) -> Result<Option<QvmMotionFrame>, GuestError> {
    let frame = inner.frames.borrow().last().cloned().flatten();
    let Some(frame) = frame else {
        return Ok(None);
    };
    if !live(inner, &frame.actor)
        || (frame.pose.is_none() && frame.body.is_none())
        || inner.module.memory().read_i32(inner.profile.movement_global)? as usize != frame.movement
    {
        return Ok(None);
    }
    Ok(Some(frame))
}

/// Run one hook closure, recording the first failure.
fn run_hook(inner: &Rc<QvmEquipmentInner>, run: impl FnOnce() -> Result<i32, GuestError>) -> i32 {
    match run() {
        Ok(result) => result,
        Err(error) => {
            if inner.error.borrow().is_none() {
                *inner.error.borrow_mut() = Some(error);
            }
            0
        }
    }
}

/// Zero held movement under a fixed pose.
fn slice(
    inner: &Rc<QvmEquipmentInner>,
    call: &mut QvmFunctionCall,
    run: &mut dyn FnMut(&mut QvmFunctionCall) -> Result<i32, GuestError>,
) -> Result<i32, GuestError> {
    let frame = inner.frames.borrow().last().cloned().flatten();
    let outcome = Rc::new(RefCell::new(None));
    if let Some(frame) = frame {
        if frame.pose.is_some() && live(inner, &frame.actor) {
            let inner_ref = Rc::clone(inner);
            let stashed = Rc::clone(&outcome);
            call.regions(vec![QvmRegionBinding {
                entry: inner.profile.locomotion.entry,
                join: inner.profile.locomotion.join,
                run: Box::new(move |_| {
                    let stopped = (|| -> Result<QvmRegionDecision, GuestError> {
                        if current(&inner_ref)?.map_or(true, |live| live.id != frame.id) {
                            return Ok(QvmRegionDecision::Execute);
                        }
                        let memory = inner_ref.module.memory();
                        let layout = qvm_command_layout(inner_ref.module.abi_profile());
                        let command = frame.movement + 4;
                        for offset in [layout.forward, layout.right, layout.up] {
                            memory.write_bytes(command + offset, &[0])?;
                        }
                        memory.write_vec3(frame.player + 32, &Vec3::default())?;
                        Ok(QvmRegionDecision::Skip)
                    })();
                    match stopped {
                        Ok(decision) => decision,
                        Err(error) => {
                            *stashed.borrow_mut() = Some(error);
                            QvmRegionDecision::Execute
                        }
                    }
                }),
                completed: None,
            }]);
        }
    }
    let result = run(call);
    if let Some(error) = outcome.borrow_mut().take() {
        return Err(error);
    }
    result
}

/// Apply accepted bounds with optional duck restoration.
fn apply_shape(
    inner: &Rc<QvmEquipmentInner>,
    call: &mut QvmFunctionCall,
    frame: &QvmMotionFrame,
    body: &QvmBodyShape,
    accepted: Bounds,
    fallback: bool,
    previous_duck: i32,
    previous_height: i32,
    result: i32,
) -> Result<i32, GuestError> {
    (body.current_actor)();
    let memory = inner.module.memory();
    if fallback {
        let flags = memory.read_i32(frame.player + 12)?;
        memory.write_i32(frame.player + 12, (flags & !1) | previous_duck)?;
        memory.write_i32(frame.player + 164, previous_height)?;
    }
    if let Some(Some(live)) = inner.frames.borrow_mut().last_mut() {
        if live.id == frame.id {
            live.accepted_bounds = Some(accepted);
        }
    }
    memory.write_vec3(frame.movement + inner.profile.mins, &accepted.min)?;
    memory.write_vec3(frame.movement + inner.profile.maxs, &accepted.max)?;
    call.effect(|| {});
    Ok(result)
}

/// Body-shape interception.
fn body_shape(
    inner: &Rc<QvmEquipmentInner>,
    call: &mut QvmFunctionCall,
    frame: &QvmMotionFrame,
) -> Result<i32, GuestError> {
    let shaped = frame.body.clone();
    let traced = inner.profile.body_trace;
    if shaped.is_none() || traced.is_none() {
        if shaped.as_ref().is_some_and(|shaped| shaped.requested.is_some()) {
            return Err(GuestError::invalid(
                "Original QVM body shape requires an admitted trace callback layout",
            ));
        }
        return Ok(call.proceed());
    }
    let (Some(body), Some(trace)) = (shaped, traced) else {
        return Ok(call.proceed());
    };
    let memory = inner.module.memory();
    let previous_duck = memory.read_i32(frame.player + 12)? & 1;
    let previous_height = memory.read_i32(frame.player + 164)?;
    let result = call.proceed();
    (body.current_actor)();
    let source = Bounds {
        min: memory.read_vec3(frame.movement + inner.profile.mins)?,
        max: memory.read_vec3(frame.movement + inner.profile.maxs)?,
    };
    let requested = body.requested.unwrap_or(source);
    let previous = inner
        .frames
        .borrow()
        .last()
        .cloned()
        .flatten()
        .and_then(|frame| frame.accepted_bounds)
        .unwrap_or(body.current);
    let expands = requested.min.x < previous.min.x
        || requested.min.y < previous.min.y
        || requested.min.z < previous.min.z
        || requested.max.x > previous.max.x
        || requested.max.y > previous.max.y
        || requested.max.z > previous.max.z;
    if !expands {
        return apply_shape(
            inner,
            call,
            frame,
            &body,
            requested,
            false,
            previous_duck,
            previous_height,
            result,
        );
    }
    let at = inner.body_scratch;
    let saved = memory.read_bytes(at, 92)?;
    memory.write_vec3(at + 56, &memory.read_vec3(frame.player + 20)?)?;
    memory.write_vec3(at + 68, &requested.min)?;
    memory.write_vec3(at + 80, &requested.max)?;
    call.effect(|| {});
    let callback = memory.read_i32(frame.movement + trace.callback)?;
    let mask = memory.read_i32(frame.movement + trace.mask)?;
    let contents = memory.read_i32(frame.player + 140)?;
    let address =
        |offset: usize| i32::try_from(offset).map_err(|_| GuestError::invalid("QVM body trace escapes its allocation"));
    inner.module.invoke_source_callback(
        usize::try_from(callback).map_err(|_| GuestError::invalid("QVM body trace escapes its allocation"))?,
        &[
            address(at)?,
            address(at + 56)?,
            address(at + 68)?,
            address(at + 80)?,
            address(at + 56)?,
            contents,
            mask,
        ],
    );
    let accepted = if memory.read_i32(at)? == 0 { requested } else { previous };
    let fallback = accepted != requested;
    let outcome = apply_shape(
        inner,
        call,
        frame,
        &body,
        accepted,
        fallback,
        previous_duck,
        previous_height,
        result,
    );
    memory.write_bytes(at, &saved)?;
    outcome
}

/// Duck interception.
fn duck(inner: &Rc<QvmEquipmentInner>, call: &mut QvmFunctionCall) -> Result<i32, GuestError> {
    let frame = current(inner)?;
    let Some(frame) = frame else {
        return Ok(call.proceed());
    };
    let Some(pose) = frame.pose else {
        return body_shape(inner, call, &frame);
    };
    let memory = inner.module.memory();
    let flags = memory.read_i32(frame.player + 12)?;
    memory.write_i32(frame.player + 12, if pose.crouched { flags | 1 } else { flags & !1 })?;
    memory.write_i32(frame.player + 164, pose.view_height)?;
    memory.write_vec3(frame.movement + inner.profile.mins, &pose.bounds.min)?;
    memory.write_vec3(frame.movement + inner.profile.maxs, &pose.bounds.max)?;
    Ok(0)
}

/// Equipment movement over original Pmove.
pub struct QvmEquipmentMovement {
    /// Shared state.
    inner: Rc<QvmEquipmentInner>,
}

impl QvmEquipmentMovement {
    /// Bind equipment entries after validating the profile.
    pub fn new(
        module: QvmModule,
        data: QvmGameData,
        artifact: &QvmArtifact,
        profile: QvmEquipmentMovementProfile,
        services: QvmEquipmentServices,
    ) -> Result<Self, GuestError> {
        let image = &artifact.image;
        let scratch = (image.data_length + image.literal_length + image.bss_length).div_ceil(16) * 16;
        if profile.body_trace.is_some()
            && image
                .allocated_data_length
                .checked_sub(65536)
                .map_or(true, |room| scratch.checked_add(92).map_or(true, |end| end > room))
        {
            return Err(GuestError::invalid(
                "Original body trace requires scratch outside source data and stack",
            ));
        }
        for entry in [profile.move_entry, profile.slice, profile.duck] {
            if image
                .instructions
                .get(entry)
                .map_or(true, |instruction| instruction.opcode != QvmOpcode::OpEnter)
            {
                return Err(GuestError::invalid(
                    "Selected equipment movement requires an original function boundary",
                ));
            }
        }
        qualify_qvm_region(
            &image.instructions,
            profile.slice,
            profile.locomotion.entry,
            profile.locomotion.join,
        )?;
        let inner = Rc::new(QvmEquipmentInner {
            module,
            data,
            profile,
            services,
            body_scratch: scratch,
            frames: RefCell::new(Vec::new()),
            next_frame: RefCell::new(0),
            hooks: RefCell::new(Vec::new()),
            error: RefCell::new(None),
        });
        let hooked = Rc::clone(&inner);
        let hook: QvmHookFn = Rc::new(move |call| run_hook(&hooked, || duck(&hooked, call)));
        inner
            .hooks
            .borrow_mut()
            .push(inner.module.bind_invocation(inner.profile.duck, hook));
        Ok(Self { inner })
    }

    /// Run a movement invocation through equipment frames.
    pub fn movement(
        &self,
        call: &mut QvmFunctionCall,
        kind: QvmApplicationScope,
        run: &mut dyn FnMut(&mut QvmFunctionCall) -> Result<i32, GuestError>,
    ) -> Result<i32, GuestError> {
        if kind == QvmApplicationScope::MovementSlice {
            return slice(&self.inner, call, run);
        }
        let frame = admit(&self.inner, call)?;
        let token = frame.as_ref().map(|frame| frame.id);
        self.inner.frames.borrow_mut().push(frame);
        let outcome = run(call);
        let top = self
            .inner
            .frames
            .borrow()
            .last()
            .cloned()
            .flatten()
            .map(|frame| frame.id);
        if top != token {
            return outcome.and(Err(GuestError::invalid(
                "Selected movement scope completed out of order",
            )));
        }
        self.inner.frames.borrow_mut().pop();
        outcome
    }

    /// Project holdable input; returns a one-shot restore.
    pub fn prepare_weapon(
        &self,
        actor: &ActorId,
        call: &mut QvmFunctionCall,
    ) -> Result<Option<Box<dyn FnOnce()>>, GuestError> {
        let frame = self.inner.frames.borrow().last().cloned().flatten();
        let Some(frame) = frame else {
            return Ok(None);
        };
        if frame.actor != *actor
            || !live(&self.inner, &frame.actor)
            || !frame.owns_holdable_input
            || self
                .inner
                .module
                .memory()
                .read_i32(self.inner.profile.movement_global)? as usize
                != frame.movement
        {
            return Ok(None);
        }
        let layout = qvm_command_layout(self.inner.module.abi_profile());
        let memory = self.inner.module.memory();
        let address = frame.movement + 4 + layout.buttons_offset;
        let previous = if layout.buttons_bytes == 1 {
            i32::from(memory.read_bytes(address, 1)?[0])
        } else {
            memory.read_i32(address)?
        };
        let projected = previous & !4;
        if layout.buttons_bytes == 1 {
            memory.write_bytes(address, &[projected as u8])?;
        } else {
            memory.write_i32(address, projected)?;
        }
        let services = self.inner.services.clone();
        Ok(Some(Box::new(move || {
            if !(services.live)(&frame.actor) {
                return;
            }
            let current = if layout.buttons_bytes == 1 {
                memory
                    .read_bytes(address, 1)
                    .map(|bytes| i32::from(bytes[0]))
                    .unwrap_or(-1)
            } else {
                memory.read_i32(address).unwrap_or(-1)
            };
            if current == projected {
                if layout.buttons_bytes == 1 {
                    let _ = memory.write_bytes(address, &[previous as u8]);
                } else {
                    let _ = memory.write_i32(address, previous);
                }
            }
        })))
    }

    /// Take the first recorded hook failure, if any.
    pub fn take_error(&self) -> Option<GuestError> {
        self.inner.error.borrow_mut().take()
    }

    /// Remove bound hooks.
    pub fn close(&self) {
        for id in self.inner.hooks.borrow_mut().drain(..).rev() {
            self.inner.module.remove_hook(id);
        }
    }

    /// Drive the duck hook with a crafted call (test seam).
    #[cfg(test)]
    fn test_duck(&self, call: &mut QvmFunctionCall) -> Result<i32, GuestError> {
        duck(&self.inner, call)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use qa_core::identity::IdentityOwner;
    use qa_core::math::vec3;

    use super::super::game_data::{ModuleIdentity, QvmArtifact, QvmImage, QvmInstruction, QvmOpcode, QvmRole};
    use super::*;

    const CLIENTS: usize = 8192;
    const CLIENT_STRIDE: usize = 512;
    const MOVEMENT: usize = 16384;

    fn instructions() -> Vec<QvmInstruction> {
        [
            (QvmOpcode::OpEnter, 0),
            (QvmOpcode::OpEnter, 0),
            (QvmOpcode::OpEnter, 0),
        ]
        .into_iter()
        .chain([(QvmOpcode::OpConst, 0), (QvmOpcode::OpPop, 0), (QvmOpcode::OpIgnore, 0)])
        .enumerate()
        .map(|(index, (opcode, operand))| QvmInstruction::word(opcode, operand, index * 8))
        .collect()
    }

    struct Fixture {
        movement: QvmEquipmentMovement,
        module: QvmModule,
        actor: ActorId,
        motion: Rc<RefCell<Option<QvmEquipmentMotion>>>,
        dead: Rc<RefCell<bool>>,
    }

    impl Fixture {
        fn memory(&self) -> super::super::game_data::QvmSharedMemory {
            self.module.memory()
        }
    }

    fn fixture() -> Fixture {
        let owner = IdentityOwner::create("equipment-test").unwrap();
        let actor = owner.actor(0, 1);
        let mut image = QvmImage::default();
        image.instructions = instructions();
        image.allocated_data_length = 65536;
        image.data_length = 1024;
        let artifact = QvmArtifact {
            module: ModuleIdentity {
                id: "test:qagame".to_string(),
                artifact_path: "test".to_string(),
                digest: "test".to_string(),
                revision: "1".to_string(),
            },
            role: QvmRole::Qagame,
            abi_profile: None,
            image,
        };
        let module = QvmModule::new(artifact.clone(), None, None).unwrap();
        let data = QvmGameData::new(module.memory(), AbiProfile::Modern);
        data.locate(4096, 4, 256, CLIENTS as i32, CLIENT_STRIDE).unwrap();
        data.set_client_count(4).unwrap();
        module.memory().write_i32(MOVEMENT, CLIENTS as i32).unwrap();
        module.memory().write_i32(CLIENTS + 52, 100).unwrap();
        module.memory().write_i32(1024, MOVEMENT as i32).unwrap();
        let motion = Rc::new(RefCell::new(None));
        let moved = Rc::clone(&motion);
        let dead = Rc::new(RefCell::new(false));
        let gone = Rc::clone(&dead);
        let found = actor.clone();
        let movement = QvmEquipmentMovement::new(
            module.clone(),
            data.clone(),
            &artifact,
            QvmEquipmentMovementProfile {
                move_entry: 0,
                slice: 1,
                duck: 2,
                movement_global: 1024,
                locomotion: QvmLocomotion { entry: 3, join: 5 },
                mins: 64,
                maxs: 76,
                body_trace: None,
            },
            QvmEquipmentServices {
                actor: Rc::new(move |slot| (slot == 0).then(|| found.clone())),
                live: Rc::new(move |_| !*gone.borrow()),
                equipment: Rc::new(move |_| moved.borrow().clone()),
            },
        )
        .unwrap();
        Fixture {
            movement,
            module,
            actor,
            motion,
            dead,
        }
    }

    fn motion(speed: f64) -> QvmEquipmentMotion {
        QvmEquipmentMotion {
            speed_multiplier: speed,
            pose: None,
            owns_holdable_input: false,
            body: None,
        }
    }

    #[test]
    fn command_layouts_match_wire_records() {
        assert_eq!(
            qvm_command_layout(AbiProfile::Modern),
            QvmCommandLayout {
                forward: 21,
                right: 22,
                up: 23,
                buttons_offset: 16,
                buttons_bytes: 4,
            }
        );
        assert_eq!(
            qvm_command_layout(AbiProfile::Legacy),
            QvmCommandLayout {
                forward: 20,
                right: 21,
                up: 22,
                buttons_offset: 4,
                buttons_bytes: 1,
            }
        );
    }

    #[test]
    fn movement_admits_and_scales_speed() {
        let fixture = fixture();
        *fixture.motion.borrow_mut() = Some(motion(1.5));
        let mut call = QvmFunctionCall::entered(0, vec![MOVEMENT as i32], fixture.memory());
        let result = fixture
            .movement
            .movement(&mut call, QvmApplicationScope::ClientCommand, &mut |call| {
                Ok(call.proceed())
            })
            .unwrap();
        assert_eq!(result, 0);
        assert_eq!(fixture.memory().read_i32(CLIENTS + 52).unwrap(), 150);

        *fixture.motion.borrow_mut() = Some(motion(0.0));
        let mut call = QvmFunctionCall::entered(0, vec![MOVEMENT as i32], fixture.memory());
        assert!(fixture
            .movement
            .movement(&mut call, QvmApplicationScope::ClientCommand, &mut |call| Ok(
                call.proceed()
            ))
            .is_err());

        *fixture.motion.borrow_mut() = None;
        let mut call = QvmFunctionCall::entered(0, vec![MOVEMENT as i32], fixture.memory());
        assert!(fixture
            .movement
            .movement(&mut call, QvmApplicationScope::ClientCommand, &mut |call| Ok(
                call.proceed()
            ))
            .is_ok());

        let mut foreign = QvmFunctionCall::entered(0, vec![4], fixture.memory());
        assert!(fixture
            .movement
            .movement(&mut foreign, QvmApplicationScope::ClientCommand, &mut |call| Ok(
                call.proceed()
            ))
            .is_ok());
    }

    #[test]
    fn slice_zeroes_pose_movement() {
        let fixture = fixture();
        *fixture.motion.borrow_mut() = Some(QvmEquipmentMotion {
            pose: Some(QvmFixedPose {
                crouched: true,
                bounds: Bounds {
                    min: vec3(-16.0, -16.0, -24.0),
                    max: vec3(16.0, 16.0, 32.0),
                },
                view_height: 26,
            }),
            ..motion(1.0)
        });
        fixture.memory().write_bytes(MOVEMENT + 4 + 21, &[9, 8, 7]).unwrap();
        fixture.memory().write_vec3(CLIENTS + 32, &vec3(1.0, 2.0, 3.0)).unwrap();
        let mut outer = QvmFunctionCall::entered(0, vec![MOVEMENT as i32], fixture.memory());
        fixture
            .movement
            .movement(&mut outer, QvmApplicationScope::ClientCommand, &mut |_| {
                let mut inner = QvmFunctionCall::entered(1, vec![MOVEMENT as i32], fixture.memory());
                fixture
                    .movement
                    .movement(&mut inner, QvmApplicationScope::MovementSlice, &mut |call| {
                        Ok(call.proceed())
                    })?;
                assert_eq!(inner.region_bindings.len(), 1);
                let mut binding = inner.region_bindings.pop().unwrap();
                let mut control = super::super::game_data::QvmRegionControl::default();
                assert_eq!((binding.run)(&mut control), QvmRegionDecision::Skip);
                Ok(0)
            })
            .unwrap();
        assert_eq!(
            fixture.memory().read_bytes(MOVEMENT + 4 + 21, 3).unwrap(),
            vec![0, 0, 0]
        );
        assert_eq!(fixture.memory().read_vec3(CLIENTS + 32).unwrap(), Vec3::default());
    }

    #[test]
    fn prepare_weapon_projects_and_restores_buttons() {
        let fixture = fixture();
        *fixture.motion.borrow_mut() = Some(QvmEquipmentMotion {
            owns_holdable_input: true,
            ..motion(1.0)
        });
        fixture.memory().write_i32(MOVEMENT + 4 + 16, 7).unwrap();
        let mut outer = QvmFunctionCall::entered(0, vec![MOVEMENT as i32], fixture.memory());
        fixture
            .movement
            .movement(&mut outer, QvmApplicationScope::ClientCommand, &mut |call| {
                let restore = fixture.movement.prepare_weapon(&fixture.actor, call)?.unwrap();
                assert_eq!(fixture.memory().read_i32(MOVEMENT + 4 + 16).unwrap(), 3);
                restore();
                assert_eq!(fixture.memory().read_i32(MOVEMENT + 4 + 16).unwrap(), 7);
                Ok(0)
            })
            .unwrap();

        *fixture.dead.borrow_mut() = true;
        let mut outer = QvmFunctionCall::entered(0, vec![MOVEMENT as i32], fixture.memory());
        fixture
            .movement
            .movement(&mut outer, QvmApplicationScope::ClientCommand, &mut |call| {
                assert!(fixture.movement.prepare_weapon(&fixture.actor, call)?.is_none());
                Ok(0)
            })
            .unwrap();
    }

    #[test]
    fn body_shape_traces_expansions() {
        let fixture = fixture();
        let shape = |requested: Option<Bounds>| QvmBodyShape {
            current: Bounds {
                min: vec3(-16.0, -16.0, -24.0),
                max: vec3(16.0, 16.0, 32.0),
            },
            requested,
            current_actor: Rc::new(|| {}),
        };
        *fixture.motion.borrow_mut() = Some(QvmEquipmentMotion {
            body: Some(shape(None)),
            ..motion(1.0)
        });
        let mut outer = QvmFunctionCall::entered(0, vec![MOVEMENT as i32], fixture.memory());
        fixture
            .movement
            .movement(&mut outer, QvmApplicationScope::ClientCommand, &mut |_| {
                let mut duck = QvmFunctionCall::entered(2, vec![MOVEMENT as i32], fixture.memory());
                assert_eq!(fixture.movement.test_duck(&mut duck).unwrap(), 0);
                Ok(0)
            })
            .unwrap();

        *fixture.motion.borrow_mut() = Some(QvmEquipmentMotion {
            body: Some(shape(Some(Bounds {
                min: vec3(-32.0, -32.0, -48.0),
                max: vec3(32.0, 32.0, 64.0),
            }))),
            ..motion(1.0)
        });
        let mut outer = QvmFunctionCall::entered(0, vec![MOVEMENT as i32], fixture.memory());
        let error = fixture
            .movement
            .movement(&mut outer, QvmApplicationScope::ClientCommand, &mut |_| {
                let mut duck = QvmFunctionCall::entered(2, vec![MOVEMENT as i32], fixture.memory());
                fixture.movement.test_duck(&mut duck)?;
                Ok(0)
            })
            .unwrap_err();
        assert!(format!("{error:?}").contains("admitted trace callback"));
    }

    #[test]
    fn duck_applies_pose_or_proceeds() {
        let fixture = fixture();
        *fixture.motion.borrow_mut() = Some(QvmEquipmentMotion {
            pose: Some(QvmFixedPose {
                crouched: true,
                bounds: Bounds {
                    min: vec3(-16.0, -16.0, -24.0),
                    max: vec3(16.0, 16.0, 32.0),
                },
                view_height: 26,
            }),
            ..motion(1.0)
        });
        let mut outer = QvmFunctionCall::entered(0, vec![MOVEMENT as i32], fixture.memory());
        fixture
            .movement
            .movement(&mut outer, QvmApplicationScope::ClientCommand, &mut |_| {
                let mut duck = QvmFunctionCall::entered(2, vec![MOVEMENT as i32], fixture.memory());
                assert_eq!(fixture.movement.test_duck(&mut duck).unwrap(), 0);
                assert_eq!(fixture.memory().read_i32(CLIENTS + 12).unwrap() & 1, 1);
                assert_eq!(fixture.memory().read_i32(CLIENTS + 164).unwrap(), 26);
                assert_eq!(
                    fixture.memory().read_vec3(MOVEMENT + 64).unwrap(),
                    vec3(-16.0, -16.0, -24.0)
                );
                Ok(0)
            })
            .unwrap();

        let mut bare = QvmFunctionCall::entered(2, vec![MOVEMENT as i32], fixture.memory());
        assert_eq!(fixture.movement.test_duck(&mut bare).unwrap(), 0);
        fixture.movement.close();
        assert_eq!(fixture.module.calls().len(), 0);
    }
}
