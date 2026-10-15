//! Weak native PUSH fields use the existing checked edict scalar boundary.
use super::{Scalar, Vm};
use crate::services::{CallContext, CallError, ENGINE_CALLS, EngineServices};
use qa_core::primitives::{CollisionOwner, NativeEntity, ThinkBinding, ThinkTime};
use qa_session::physics::PushNumber;
use qa_world::{
    collision::{QuakeTraceKind, TraceQuery, trace_policy},
    entities::EntityTable,
};

impl From<PushNumber> for Scalar {
    fn from(value: PushNumber) -> Self {
        match value {
            PushNumber::Motion(value) => value.into(),
            PushNumber::LocalTime => Self::LocalTime,
            PushNumber::NextThink => Self::NextThink,
            PushNumber::MoveType => Self::MoveType,
            PushNumber::Solid => Self::Solid,
            PushNumber::Min(axis) => Self::Min(axis),
            PushNumber::Max(axis) => Self::Max(axis),
            PushNumber::AbsoluteMin(axis) => Self::AbsoluteMin(axis),
            PushNumber::AbsoluteMax(axis) => Self::AbsoluteMax(axis),
        }
    }
}

impl Vm {
    /// The provider starts at ordinal one. Every observed native row is
    /// visited, including client and freed backing; each call rereads count.
    pub fn push_candidate(&self, cursor: &mut usize) -> Result<Option<NativeEntity>, CallError> {
        self.projection.quakec_push_candidate(cursor)
    }
    pub fn push_free(
        &self,
        entities: &EntityTable,
        native: NativeEntity,
    ) -> Result<bool, CallError> {
        self.projection.quakec_push_free(entities, native)
    }
    pub fn push_number(&self, value: u32, field: PushNumber) -> Result<f32, CallError> {
        self.projection
            .quakec_scalar_number(&self.entities, value, field.into())
    }
    fn store_raw_scalar(
        &mut self,
        entities: &mut EntityTable,
        context: CallContext,
        value: u32,
        field: Scalar,
        word: u32,
    ) -> Result<(), CallError> {
        // Reject another module before changing native bytes. Liveness is not
        // required: native rollback addresses the retained ordinal backing.
        self.projection.quakec_validate_owner(context.module)?;
        let address = self
            .projection
            .quakec_scalar_store(&self.entities, value, field)?;
        self.store_entity_word(address, word)
            .map_err(|_| CallError::Memory)?;
        self.projection
            .quakec_refresh_raw_motion(entities, &self.entities, context, value)
    }
    pub fn push_store(
        &mut self,
        entities: &mut EntityTable,
        context: CallContext,
        value: u32,
        field: PushNumber,
        number: f32,
    ) -> Result<(), CallError> {
        self.store_raw_scalar(entities, context, value, field.into(), number.to_bits())
    }
    pub fn push_flags(&self, value: u32) -> Result<u32, CallError> {
        self.projection.quakec_monster_flags(&self.entities, value)
    }
    pub fn push_store_flags(
        &mut self,
        entities: &mut EntityTable,
        context: CallContext,
        value: u32,
        flags: u32,
    ) -> Result<(), CallError> {
        self.store_raw_scalar(
            entities,
            context,
            value,
            Scalar::Flags,
            (flags as i32 as f32).to_bits(),
        )
    }
    pub fn push_ground(
        &self,
        context: CallContext,
        value: u32,
    ) -> Result<CollisionOwner, CallError> {
        self.projection.quakec_validate_owner(context.module)?;
        let address = self
            .projection
            .quakec_scalar_store(&self.entities, value, Scalar::Ground)?;
        let value = self.entities.read_word(address)? as u32;
        self.projection
            .quakec_entity_owner(&self.entities, context.module, value)
            .map_err(|_| CallError::Memory)
    }
    pub fn push_test_position(
        &self,
        services: &mut EngineServices<'_>,
        context: CallContext,
        value: u32,
    ) -> Result<bool, CallError> {
        let origin = self
            .projection
            .quakec_monster_origin(&self.entities, value)?;
        let bounds = self
            .projection
            .quakec_monster_bounds(&self.entities, value)?;
        let (rules, entities) = trace_policy(context.trace_rules);
        let mut query = TraceQuery::point(
            origin,
            origin,
            rules,
            entities.with_quake_kind(Some(QuakeTraceKind::Normal)),
        );
        query.mins = bounds.mins;
        query.maxs = bounds.maxs;
        self.with_trace_view(context, value, services.native_geometry, |view, pass| {
            query.pass = pass;
            Ok((ENGINE_CALLS.trace)(services, query, Some(view))?.start_solid)
        })
    }
    /// Resolve after restore-trigger callbacks, rather than caching a selector
    /// before a callback may change it. No liveness is implied by this pointer.
    pub fn blocked_binding(&self, value: u32) -> Result<Option<ThinkBinding>, CallError> {
        let word = self
            .projection
            .quakec_function_word(&self.entities, value, true)?;
        if word == 0 {
            Ok(None)
        } else {
            self.projection.quakec_function_binding(word).map(Some)
        }
    }
    /// Native blocked calls leave time unchanged and intentionally do not save
    /// or restore either entity global. Assignment order matters for aliases.
    pub fn prepare_blocked(
        &mut self,
        native_self: u32,
        native_other: u32,
    ) -> Result<(), CallError> {
        self.projection
            .quakec_check_address(&self.entities, native_self)?;
        self.projection
            .quakec_check_address(&self.entities, native_other)?;
        let self_at = self.call_fields.self_global.ok_or(CallError::Memory)?;
        let other_at = self.call_fields.other_global.ok_or(CallError::Memory)?;
        self.image.globals[self_at] = native_self;
        self.image.globals[other_at] = native_other;
        Ok(())
    }
    /// Clear the native deadline before rereading its potentially aliased
    /// function word. This is shared by ordinary think and admitted PUSH think.
    pub fn prepare_think(
        &mut self,
        value: u32,
        time: ThinkTime,
    ) -> Result<ThinkBinding, CallError> {
        let deadline =
            self.projection
                .quakec_scalar_store(&self.entities, value, Scalar::NextThink)?;
        self.store_entity_word(deadline, 0)
            .map_err(|_| CallError::Memory)?;
        if self.call_fields.time_global.is_some() {
            self.set_time(time)?;
        }
        if let Some(self_at) = self.call_fields.self_global {
            self.image.globals[self_at] = value;
        }
        if let Some(other_at) = self.call_fields.other_global {
            self.image.globals[other_at] = 0;
        }
        let word = self
            .projection
            .quakec_function_word(&self.entities, value, false)?;
        self.projection.quakec_function_binding(word)
    }
}
