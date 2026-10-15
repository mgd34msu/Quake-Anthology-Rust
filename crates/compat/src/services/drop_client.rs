use super::{CallError, EngineServices};
use qa_core::{
    primitives::{ClientId, EntityId, NativeEntity, RuleSetId},
    sys_events::EventTime,
    text::FixedText,
};

/// Native FREE rows have no invented canonical connection. Connected rows
/// retain their exact canonical lifetime throughout synchronous Disconnect.
#[derive(Clone, Copy, Debug)]
pub struct DropClientTarget {
    pub native: NativeEntity,
    pub client: Option<(ClientId, EntityId)>,
}

fn append(command: &mut FixedText<1024>, text: &[u8]) -> Result<(), CallError> {
    let count = text.len().min(1023 - command.as_bytes().len());
    command
        .append_bytes(&text[..count])
        .map_err(|_| CallError::Capacity)
}

impl EngineServices<'_> {
    pub(super) fn publish_server_command(
        &mut self,
        client: Option<ClientId>,
        module: qa_core::primitives::ModuleId,
        source: RuleSetId,
        text: &[u8],
    ) -> Result<u64, CallError> {
        let sequence = self
            .server
            .events
            .server_command(client, module, source, text)
            .map_err(|_| CallError::Capacity)?;
        if client.is_none() {
            for client in &self.server.clients {
                if !client.is_active()
                    && let Some(output) = client.output
                {
                    self.server.events.submit(
                        output,
                        sequence,
                        qa_core::events::OutputSubmission::BestEffort,
                    );
                }
            }
        }
        Ok(sequence)
    }
    pub(super) fn drop_client_target(
        &mut self,
        native: NativeEntity,
        time: EventTime,
    ) -> Result<Option<DropClientTarget>, CallError> {
        let range = self.storage.range(native.module).ok_or(CallError::Entity)?;
        let limit = range.client_capacity.ok_or(CallError::Entity)?;
        let Some(ordinal) = native
            .slot
            .checked_sub(range.client_first)
            .and_then(|n| usize::try_from(n).ok())
            .filter(|&n| n < limit as usize)
        else {
            return Ok(None);
        };
        let client = if let Some(id) = self.server.client_for_native(native) {
            let client = &self.server.clients[id.0 as usize];
            if !client.is_active() {
                return Ok(None);
            }
            Some((id, client.entity))
        } else {
            if let Some(closing) = range
                .free_closing
                .get(ordinal)
                .copied()
                .ok_or(CallError::Entity)?
            {
                if closing.in_callback {
                    return Ok(None);
                }
                let seconds = self
                    .cvars
                    .integer_in(self.storage.zombie_time, range.native_rules);
                if !qa_session::clients::close_timeout_expired(closing.started, time, seconds) {
                    return Ok(None);
                }
                let range = self
                    .storage
                    .range_mut(native.module)
                    .ok_or(CallError::Entity)?;
                *range
                    .free_closing
                    .get_mut(ordinal)
                    .ok_or(CallError::Entity)? = None;
            }
            None
        };
        Ok(Some(DropClientTarget { native, client }))
    }

    pub(super) fn begin_drop_client(
        &mut self,
        target: DropClientTarget,
        reason: &[u8],
        time: EventTime,
    ) -> Result<(), CallError> {
        let mut command = FixedText::<1024>::default();
        append(&mut command, b"print \"")?;
        if let Some((id, _)) = target.client {
            let info = self.server.userinfo(id).ok_or(CallError::Entity)?;
            let name = qa_core::text::info_value(info, b"name", true)
                .map(|range| &info[range])
                .unwrap_or(b"");
            append(&mut command, &name[..name.len().min(31)])?;
        }
        append(&mut command, b"^7 ")?;
        append(&mut command, reason)?;
        append(&mut command, b"\n\"")?;
        self.publish_server_command(
            None,
            target.native.module,
            RuleSetId::Quake3,
            command.as_bytes(),
        )?;
        if let Some((id, lifetime)) = target.client {
            if !self.server.begin_close(id, lifetime, time) {
                return Err(CallError::Entity);
            }
        } else {
            let range = self
                .storage
                .range_mut(target.native.module)
                .ok_or(CallError::Entity)?;
            let ordinal = target
                .native
                .slot
                .checked_sub(range.client_first)
                .and_then(|n| usize::try_from(n).ok())
                .ok_or(CallError::Entity)?;
            *range
                .free_closing
                .get_mut(ordinal)
                .ok_or(CallError::Entity)? = Some(super::FreeClientClosing {
                started: time,
                in_callback: true,
            });
        }
        Ok(())
    }

    pub(super) fn finish_drop_client(
        &mut self,
        target: DropClientTarget,
        reason: &[u8],
    ) -> Result<(), CallError> {
        let Some((id, lifetime)) = target.client else {
            let range = self
                .storage
                .range_mut(target.native.module)
                .ok_or(CallError::Entity)?;
            let ordinal = target
                .native
                .slot
                .checked_sub(range.client_first)
                .and_then(|ordinal| usize::try_from(ordinal).ok())
                .ok_or(CallError::Entity)?;
            let closing = range
                .free_closing
                .get_mut(ordinal)
                .and_then(Option::as_mut)
                .ok_or(CallError::Entity)?;
            closing.in_callback = false;
            return Ok(());
        };
        if self.server.client_for_native(target.native) != Some(id)
            || self.server.entities.resolve(lifetime).is_none()
            || self.server.clients[id.0 as usize].entity != lifetime
        {
            return Err(CallError::Entity);
        }
        if self.server.clients[id.0 as usize].output.is_none() {
            return self.storage.set_userinfo(self.server, target.native, b"");
        }
        let mut command = FixedText::<1024>::default();
        append(&mut command, b"disconnect \"")?;
        append(&mut command, reason)?;
        append(&mut command, b"\"")?;
        let sequence = self.publish_server_command(
            Some(id),
            target.native.module,
            RuleSetId::Quake3,
            command.as_bytes(),
        )?;
        if !self.server.finish_close(id, lifetime, sequence) {
            return Err(CallError::Entity);
        }
        self.storage.set_userinfo(self.server, target.native, b"")
    }
}
