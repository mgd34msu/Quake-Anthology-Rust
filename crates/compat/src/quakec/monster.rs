//! Native monster fields adapt to the shared displacement/support solver.
use super::{ClientCallRestore, EntityAccess, Scalar, Trap, Vm};
use crate::services::{CallContext, CallError, ENGINE_CALLS, EngineServices};
use qa_core::primitives::{Bounds, NativeEntityRef, ResourceNamespace, Vec3};
use qa_gameplay::rules::motion::{StepRules, step_rules};
use qa_session::physics::{
    GoalCursor, GoalHost, GoalStep, MonsterHost, MotionTrace, StepMove, StepPost,
};
use qa_world::collision::{
    EntityTracePolicy, QuakeTraceKind, TraceQuery, TraceRules, trace_policy,
};

#[derive(Debug)]
pub(crate) struct WalkAfterTouch {
    pub mover: NativeEntityRef,
    post: StepPost,
    rules: StepRules,
    restore: ClientCallRestore,
}
pub(crate) enum WalkMove {
    Rejected,
    Blocked(ClientCallRestore),
    Moved(WalkAfterTouch),
}

struct Host<'a, 'engine> {
    vm: &'a mut Vm,
    services: &'a mut EngineServices<'engine>,
    context: CallContext,
    native: u32,
    trace_rules: TraceRules,
    entity_rules: EntityTracePolicy,
}
impl<'a, 'engine> Host<'a, 'engine> {
    fn new(
        vm: &'a mut Vm,
        services: &'a mut EngineServices<'engine>,
        context: CallContext,
        native: u32,
    ) -> Self {
        let (trace_rules, entity_rules) = trace_policy(context.trace_rules);
        Self {
            vm,
            services,
            context,
            native,
            trace_rules,
            entity_rules,
        }
    }
    fn read_target(&mut self, target: NativeEntityRef, field: Scalar) -> Result<f32, CallError> {
        let native = self.vm.weak_entity_offset(self.context.module, target)?;
        let address = self
            .vm
            .projection
            .quakec_scalar_address(&self.vm.entities, native, field)?;
        let access = EntityAccess::engine(self.services);
        let words =
            self.vm
                .read_entity_words(&access, address, 1)
                .map_err(|error| match error {
                    Trap::EntityAccess => CallError::Unsupported,
                    _ => CallError::Memory,
                })?;
        Ok(f32::from_bits(words[0]))
    }
    fn store(&mut self, field: Scalar, word: u32) -> Result<(), CallError> {
        let address =
            self.vm
                .projection
                .quakec_scalar_store(&self.vm.entities, self.native, field)?;
        self.vm
            .store_entity_word(address, word)
            .map_err(|_| CallError::Memory)
    }
}
impl MonsterHost for Host<'_, '_> {
    type Error = CallError;
    fn origin(&mut self) -> Result<Vec3, CallError> {
        self.vm
            .projection
            .quakec_monster_origin(&self.vm.entities, self.native)
    }
    fn bounds(&mut self) -> Result<Bounds, CallError> {
        self.vm
            .projection
            .quakec_monster_bounds(&self.vm.entities, self.native)
    }
    fn flags(&mut self) -> Result<u32, CallError> {
        self.vm
            .projection
            .quakec_monster_flags(&self.vm.entities, self.native)
    }
    fn enemy(&mut self) -> Result<Option<NativeEntityRef>, CallError> {
        let native = self
            .vm
            .projection
            .quakec_monster_enemy(&self.vm.entities, self.native)?;
        if native == 0 {
            return Ok(None);
        }
        let ordinal = self
            .vm
            .entity_number(native)
            .map_err(|_| CallError::Memory)?;
        Ok(Some(NativeEntityRef::entity(
            ResourceNamespace::Module(self.context.module),
            ordinal,
        )))
    }
    fn target_origin(&mut self, target: NativeEntityRef) -> Result<Vec3, CallError> {
        Ok(Vec3([
            self.read_target(target, Scalar::Origin(0))?,
            self.read_target(target, Scalar::Origin(1))?,
            self.read_target(target, Scalar::Origin(2))?,
        ]))
    }
    fn contents(&mut self, point: Vec3) -> Result<i32, CallError> {
        let services = &mut self.services;
        let entity_rules = self.entity_rules;
        self.vm
            .with_trace_view(self.context, self.native, |view, pass| {
                Ok(
                    (ENGINE_CALLS.point_contents)(services, point, entity_rules, pass, Some(view))?
                        .to_q1(),
                )
            })
    }
    fn trace(
        &mut self,
        start: Vec3,
        end: Vec3,
        bounds: Bounds,
        kind: QuakeTraceKind,
    ) -> Result<MotionTrace, CallError> {
        let mut query = TraceQuery::point(
            start,
            end,
            self.trace_rules,
            self.entity_rules.with_quake_kind(Some(kind)),
        );
        query.mins = bounds.mins;
        query.maxs = bounds.maxs;
        let services = &mut self.services;
        let trace = self
            .vm
            .with_trace_view(self.context, self.native, |view, pass| {
                query.pass = pass;
                (ENGINE_CALLS.trace)(services, query, Some(view))
            })?;
        let ground = trace.entity.and_then(|hit| {
            // The collision store's world hit is the common world slot. Its
            // native pointer is this QC module's explicit world ordinal zero.
            if Some(hit) == self.services.server.entities.id_at(0) {
                return Some(NativeEntityRef::entity(
                    ResourceNamespace::Module(self.context.module),
                    0,
                ));
            }
            // Blocking/support traces do not need a native ground pointer.
            // The solver requires this only on a supported landing write.
            let slot = self.services.server.entities.resolve(hit)?;
            let native = self.services.server.entities.columns.native_entity[slot]?;
            Some(NativeEntityRef::entity(
                ResourceNamespace::Module(native.module),
                u32::try_from(native.slot).ok()?,
            ))
        });
        Ok(MotionTrace { trace, ground })
    }
    fn write_origin(&mut self, axis: usize, value: f32) -> Result<(), CallError> {
        self.store(Scalar::Origin(axis), value.to_bits())
    }
    fn write_flags(&mut self, flags: u32) -> Result<(), CallError> {
        self.store(Scalar::Flags, (flags as i32 as f32).to_bits())
    }
    fn write_ground(&mut self, ground: NativeEntityRef) -> Result<(), CallError> {
        let admitted =
            self.vm
                .admit_reference(&self.services.server.entities, self.context, ground)?;
        let native = admitted.offset();
        self.vm.release_entity(admitted)?;
        self.store(Scalar::Ground, native)
    }
}

impl GoalHost for Host<'_, '_> {
    fn ideal_yaw(&mut self) -> Result<f32, CallError> {
        self.vm
            .projection
            .quakec_scalar_number(&self.vm.entities, self.native, Scalar::IdealYaw)
    }
    fn yaw(&mut self) -> Result<f32, CallError> {
        self.vm
            .projection
            .quakec_scalar_number(&self.vm.entities, self.native, Scalar::Yaw)
    }
    fn write_ideal_yaw(&mut self, value: f32) -> Result<(), CallError> {
        self.store(Scalar::IdealYaw, value.to_bits())
    }
    fn change_yaw(&mut self) -> Result<(), CallError> {
        self.vm.change_yaw()
    }
    fn enemy_present(&mut self) -> Result<bool, CallError> {
        Ok(self
            .vm
            .projection
            .quakec_monster_enemy(&self.vm.entities, self.native)?
            != 0)
    }
    fn absolute_min(&mut self, target: NativeEntityRef, axis: usize) -> Result<f32, CallError> {
        self.read_target(target, Scalar::AbsoluteMin(axis))
    }
    fn absolute_max(&mut self, target: NativeEntityRef, axis: usize) -> Result<f32, CallError> {
        self.read_target(target, Scalar::AbsoluteMax(axis))
    }
    fn random_word(&mut self) -> u32 {
        self.vm.random_word()
    }
}

impl Vm {
    pub(crate) fn move_to_goal(
        &mut self,
        services: &mut EngineServices<'_>,
        context: CallContext,
        distance: f32,
    ) -> Result<GoalStep, CallError> {
        let self_at = self.call_fields.self_global.ok_or(CallError::Memory)?;
        let native = self.image.globals[self_at];
        let goal = self
            .projection
            .quakec_monster_goal(&self.entities, native)?;
        let reference = |value| {
            self.entity_number(value)
                .map(|ordinal| {
                    NativeEntityRef::entity(ResourceNamespace::Module(context.module), ordinal)
                })
                .map_err(|_| CallError::Memory)
        };
        let actor = reference(native)?;
        let goal = reference(goal)?;
        let rules = step_rules(context.native_rules).ok_or(CallError::Unsupported)?;
        let step = GoalCursor::begin(
            &mut Host::new(self, services, context, native),
            actor,
            goal,
            distance,
            rules,
        )?;
        self.link_goal(services, context, step)
    }
    pub(crate) fn resume_goal(
        &mut self,
        services: &mut EngineServices<'_>,
        context: CallContext,
        cursor: GoalCursor,
    ) -> Result<GoalStep, CallError> {
        let native = self.weak_entity_offset(context.module, cursor.actor())?;
        let step = cursor.resume(&mut Host::new(self, services, context, native))?;
        self.link_goal(services, context, step)
    }
    fn link_goal(
        &mut self,
        services: &mut EngineServices<'_>,
        context: CallContext,
        step: GoalStep,
    ) -> Result<GoalStep, CallError> {
        if let GoalStep::Link(cursor) = &step {
            let native = self.weak_entity_offset(context.module, cursor.actor())?;
            self.with_entity_stores(|projection, memory, strings, store| {
                projection.quakec_link(services, memory, strings, context, native, store)
            })?;
        }
        Ok(step)
    }
    pub(crate) fn walk_move(
        &mut self,
        services: &mut EngineServices<'_>,
        context: CallContext,
        yaw: f32,
        distance: f32,
    ) -> Result<WalkMove, CallError> {
        let self_at = self.call_fields.self_global.ok_or(CallError::Memory)?;
        let native = self.image.globals[self_at];
        let rules = step_rules(context.native_rules).ok_or(CallError::Unsupported)?;
        if self
            .projection
            .quakec_monster_flags(&self.entities, native)?
            & rules.mobility_flags()
            == 0
        {
            return Ok(WalkMove::Rejected);
        }
        let movement = qa_movement::yaw_displacement(yaw, distance);
        let restore = ClientCallRestore::self_only(self.image.globals[self_at]);
        let outcome = qa_session::physics::move_step(
            &mut Host::new(self, services, context, native),
            movement,
            &rules,
        )?;
        let StepMove::Moved(post) = outcome else {
            return Ok(WalkMove::Blocked(restore));
        };
        self.with_entity_stores(|projection, memory, strings, store| {
            projection.quakec_link(services, memory, strings, context, native, store)
        })?;
        let ordinal = self.entity_number(native).map_err(|_| CallError::Memory)?;
        Ok(WalkMove::Moved(WalkAfterTouch {
            mover: NativeEntityRef::entity(ResourceNamespace::Module(context.module), ordinal),
            post,
            rules,
            restore,
        }))
    }
    pub(crate) fn finish_walk_move(
        &mut self,
        services: &mut EngineServices<'_>,
        context: CallContext,
        walk: &WalkAfterTouch,
    ) -> Result<(), CallError> {
        let native = self.weak_entity_offset(context.module, walk.mover)?;
        qa_session::physics::finish_step(
            &mut Host::new(self, services, context, native),
            walk.post,
            &walk.rules,
        )
    }
    pub(crate) fn restore_walk_move(&mut self, walk: WalkAfterTouch) -> Result<(), CallError> {
        self.finish_client_call(walk.restore)
    }
}
