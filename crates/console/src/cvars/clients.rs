//! CLIENT columns share names, bindings, conversions and the ordinary value path.
use super::*;
use qa_core::primitives::{ClientId, EntityId};
use qa_core::sys_events::SeatId;

pub(super) struct ClientColumn {
    owner: Option<(ClientId, EntityId)>,
    local: Option<(SeatId, RuleSetId)>,
}

struct ClientArchive {
    values: Vec<(CvarHandle, FixedText<MAX_TEXT>, [Option<u32>; 5])>,
    details: Vec<(View, FixedText<MAX_TEXT>)>,
}

fn inherited(root: &Value) -> Value {
    Value {
        module_defaults: [None; 5],
        explicit: false,
        command_flags: [None; 5],
        revision: 0,
        pending_revision: 0,
        latch_active: false,
        ..root.clone()
    }
}

pub(super) fn native_context(context: Context) -> Context {
    Context {
        file_search: None,
        event_time: None,
        key_source: None,
        producer: None,
        client_lifetime: None,
        cvar_seat: None,
        client_policy: None,
        ..context
    }
}

impl Cvars {
    pub(super) fn column(&self, handle: CvarHandle) -> usize {
        handle.0 as usize / self.stride
    }
    pub(super) fn root(&self, handle: CvarHandle) -> CvarHandle {
        CvarHandle((handle.0 as usize % self.stride) as u32)
    }
    pub(super) fn in_column(&self, handle: CvarHandle, column: usize) -> CvarHandle {
        let root = self.root(handle);
        CvarHandle((column * self.stride + root.0 as usize) as u32)
    }
    pub(super) fn selected_handle(&self, handle: CvarHandle, column: usize) -> CvarHandle {
        let column = if self.shared_values.contains(self.root(handle).0 as usize) {
            0
        } else {
            column
        };
        self.in_column(handle, column)
    }
    pub(super) fn defined(&self, index: usize) -> bool {
        index % self.stride < self.registered_values
    }
    pub(super) fn projection_count(&self) -> usize {
        self.projection_rows
            .last()
            .map_or(0, |span| span.range().end)
    }
    pub(super) fn column_owner(&self, column: usize) -> Option<(ClientId, EntityId)> {
        column
            .checked_sub(1)
            .and_then(|index| self.clients[index].owner)
    }
    pub(super) fn column_seat(&self, column: usize) -> Option<SeatId> {
        column
            .checked_sub(1)
            .and_then(|index| self.clients[index].local.map(|(seat, _)| seat))
    }
    fn seat_column(&self, seat: SeatId) -> Option<usize> {
        self.clients
            .iter()
            .position(|row| row.local.is_some_and(|(local, _)| local == seat))
            .map(|index| index + 1)
    }
    pub fn has_seat_values(&self, seat: SeatId) -> bool {
        self.seat_column(seat).is_some()
    }
    pub(super) fn context_column(&self, context: Context) -> Option<usize> {
        if context.side == Scope::Server {
            return Some(0);
        }
        match context.client_lifetime {
            None => context
                .cvar_seat
                .map_or(Some(0), |seat| self.seat_column(seat)),
            Some(owner) => self
                .clients
                .iter()
                .position(|row| row.owner == Some(owner))
                .map(|index| index + 1),
        }
    }
    pub(super) fn context_handle(
        &self,
        handle: CvarHandle,
        context: Context,
    ) -> Option<CvarHandle> {
        Some(self.selected_handle(handle, self.context_column(context)?))
    }
    pub(super) fn slot_in(&self, row: usize, view: View) -> CvarHandle {
        self.selected_handle(self.slot(row, 0), self.column(view.handle))
    }
    pub(super) fn inherits(&self, handle: CvarHandle) -> bool {
        self.column(handle) != 0 && !self.values[handle.0 as usize].explicit
    }
    pub(super) fn pending_affects(&self, changed: CvarHandle, handle: CvarHandle) -> bool {
        changed == handle || self.inherits(handle) && changed == self.root(handle)
    }
    pub(super) fn validate_view(&self, view: View) -> Result<(), conversion::Error> {
        (self.context_handle(view.handle, view.context) == Some(view.handle))
            .then_some(())
            .ok_or(conversion::Error::PolicyRequired)
    }
    pub(super) fn pad_values(&mut self) {
        let empty = self.values[0].clone();
        self.values.resize_with(self.stride, || empty.clone());
    }
    pub(super) fn add_client_definition(&mut self, handle: CvarHandle) {
        for column in 1..self.texts.len() {
            let row = self.in_column(handle, column).0 as usize;
            self.values[row] = inherited(&self.values[handle.0 as usize]);
        }
    }

    /// Physical host controls (one window, worker pool or audio stream) remain
    /// common to every CLIENT. The capability owner declares them at load.
    pub fn share_with_clients(&mut self, handle: CvarHandle) {
        self.shared_values.mark(self.root(handle).0 as usize);
    }

    pub fn has_client_values(&self, owner: (ClientId, EntityId)) -> bool {
        self.clients.iter().any(|row| row.owner == Some(owner))
    }

    /// Reserve a connection's value column at load, before any of its modules
    /// bind cvars. Rebinding the same lifetime is a no-op.
    /// Local seats retain their archived preferences across connections, even
    /// when a different native client slot is assigned. Remote rows retain no
    /// settings when their connection lifetime changes.
    pub fn load_client_values(
        &mut self,
        owner: (ClientId, EntityId),
        local: Option<(SeatId, RuleSetId)>,
    ) -> Result<(), WriteError> {
        let reserved = local.and_then(|(seat, _)| self.seat_column(seat).map(|column| column - 1));
        if let Some(index) = self.clients.iter().position(|row| row.owner == Some(owner))
            && reserved.is_none_or(|reserved| reserved == index)
            && local.is_none_or(|(seat, _)| {
                self.clients[index].local.is_none_or(|(old, _)| seat == old)
            })
        {
            let row = &mut self.clients[index];
            if local.is_some() {
                row.local = local;
            }
            return Ok(());
        }
        self.refresh_changes();
        let previous = if local.is_some() {
            reserved
        } else {
            self.clients.iter().position(|row| {
                row.local.is_none() && row.owner.is_none_or(|previous| previous.0 == owner.0)
            })
        };
        let column = previous.map_or(self.texts.len(), |index| index + 1);
        let retained = previous
            .and_then(|index| self.clients[index].local)
            .map(|(seat, source)| self.local_archive(column, seat, source))
            .transpose()?;
        for row in &mut self.clients {
            if row.owner.is_some_and(|previous| previous.0 == owner.0) {
                row.owner = None;
            }
        }
        if previous.is_none() {
            let total = self
                .stride
                .checked_mul(column + 1)
                .ok_or(WriteError::Capacity)?;
            u32::try_from(total).map_err(|_| WriteError::Capacity)?;
            let text = qa_core::text::TextSlots::load(
                (0..self.stride).map(|_| MAX_TEXT),
                qa_core::text::SlotsLimits {
                    rows: self.stride,
                    width: MAX_TEXT,
                    bytes: 512 * 1024 * 1024,
                },
            )
            .map_err(|_| WriteError::Capacity)?;
            self.values.reserve(self.stride);
            for index in 0..self.stride {
                self.values.push(inherited(&self.values[index]));
            }
            self.texts.push(text);
            self.clients.push(ClientColumn {
                owner: Some(owner),
                local,
            });
            self.info_revisions.push([[0; INFO_FLAGS.len()]; 5]);
            let projections = self.projection_count();
            // Each column has its own alias retention, while immutable schema
            // and numeric dependency lists remain shared by every client.
            for index in 0..projections {
                self.details.push(std::array::from_fn(|source| {
                    self.details[index][source].as_ref().map(|_| Box::default())
                }));
            }
            self.projections
                .resize(self.projections.len() + projections, [[Ok((0.0, 0)); 3]; 5]);
            self.pending.reserve(total - self.pending.len());
            self.dirty_handles.reserve(total - self.dirty_handles.len());
            self.dirty_bindings
                .reserve((column + 1) * BINDINGS.len() - self.dirty_bindings.len());
            self.dirty_values = StampSet::new(total);
            self.dirty_projections = StampSet::new((column + 1) * BINDINGS.len());
        } else {
            let stride = self.stride;
            self.remove_pending(|pending| {
                pending
                    .handles()
                    .any(|handle| handle.0 as usize / stride == column)
            });
            self.clients[column - 1] = ClientColumn {
                owner: Some(owner),
                local,
            };
            for index in 0..self.stride {
                self.values[column * self.stride + index] = inherited(&self.values[index]);
                let _ = self.texts[column].set_length(index, 0);
            }
            for index in column * self.projection_count()..(column + 1) * self.projection_count() {
                for detail in self.details[index].iter_mut().flatten() {
                    detail.present = false;
                }
            }
        }
        for index in 0..self.registered_values {
            let handle = self.in_column(CvarHandle(index as u32), column);
            self.mark_change(handle);
        }
        if let Some(retained) = retained {
            for (handle, text, flags) in retained.values {
                self.values[handle.0 as usize].command_flags = flags;
                self.publish(handle, text.as_str())?;
            }
            for (mut view, text) in retained.details {
                view.context.client_lifetime = Some(owner);
                self.save_detail(view, text.as_str(), false)?;
            }
        }
        self.refresh_changes();
        Ok(())
    }

    fn local_archive(
        &self,
        column: usize,
        seat: SeatId,
        source: RuleSetId,
    ) -> Result<ClientArchive, WriteError> {
        let context = Context {
            source,
            seat,
            client_lifetime: None,
            cvar_seat: None,
            ..self.context
        };
        let mut retained = Vec::new();
        for index in 0..self.registered_values {
            let handle = self.in_column(CvarHandle(index as u32), column);
            let value = &self.values[handle.0 as usize];
            let Some(mut view) = self.bind_id(value.name_id, context) else {
                continue;
            };
            view.handle = handle;
            let flags = self.flags(view);
            if flags & 1 == 0 || flags & (4 | 8 | 16 | 64 | 512) != 0 {
                continue;
            }
            let pending = self
                .pending
                .iter()
                .any(|group| group.handles().any(|changed| changed == handle));
            if !value.explicit && !pending {
                continue;
            }
            let text = if source == RuleSetId::Quake3 {
                self.pending_text(handle, source)
            } else {
                self.effective(handle, source)
            };
            retained.push((
                handle,
                native_text(Text::Borrowed(text))?,
                value.command_flags,
            ));
        }
        let mut details = Vec::new();
        for (binding, definition) in BINDINGS.iter().enumerate() {
            let Some(handle) = self.binding_slot(definition, context) else {
                continue;
            };
            let handle = self.in_column(handle, column);
            if !retained.iter().any(|(saved, _, _)| *saved == handle) {
                continue;
            }
            for source in RuleSetId::ALL {
                let view = View {
                    handle,
                    binding: binding as u16,
                    context: Context { source, ..context },
                };
                let prior = |handle| {
                    if source == RuleSetId::Quake3 {
                        self.pending_text(handle, source)
                    } else {
                        self.effective(handle, source)
                    }
                };
                let unchanged = |handle| {
                    let next = retained
                        .iter()
                        .find(|(saved, _, _)| *saved == handle)
                        .map_or_else(
                            || self.effective(self.root(handle), source),
                            |(_, text, _)| text.as_str(),
                        );
                    prior(handle) == next
                };
                if !unchanged(handle)
                    || !self
                        .dependency_rows(view)
                        .all(|row| unchanged(self.slot_in(row as usize, view)))
                {
                    continue;
                }
                let text = if source == RuleSetId::Quake3 {
                    self.pending_detail(view)
                } else {
                    self.detail(view)
                };
                if let Some(text) = text {
                    details.push((view, native_text(Text::Borrowed(text))?));
                }
            }
        }
        Ok(ClientArchive {
            values: retained,
            details,
        })
    }

    /// Q3 CL_SystemInfoChanged sets native cvars in the receiving CLIENT.
    /// All writes use the same policy/conversion/publication functions as local
    /// commands; SERVER and other CLIENT columns remain independent.
    pub fn apply_client_system_info(
        &mut self,
        owner: (ClientId, EntityId),
        info: &[u8],
    ) -> Result<(), WriteError> {
        let context = Context {
            source: RuleSetId::Quake3,
            side: Scope::Client,
            role: Role::Cgame,
            client_lifetime: Some(owner),
            cvar_seat: None,
            ..self.context
        };
        self.context_column(context).ok_or(WriteError::Capacity)?;
        // Reject invalid text before changing any value. The native wire parser
        // already supplies one complete configstring with no embedded NUL.
        for (key, value) in qa_core::text::info_pairs(info) {
            std::str::from_utf8(key).map_err(|_| WriteError::InvalidInfo)?;
            std::str::from_utf8(value).map_err(|_| WriteError::InvalidInfo)?;
        }
        if qa_core::text::info_integer(info, b"sv_cheats") == 0 {
            for index in 0..self.registered_values {
                let name = self.values[index].name_id;
                if let Some(view) = self.bind_id(name, context)
                    && self.column(view.handle) != 0
                    && self.flags(view) & 512 != 0
                    && self.native_default_available(view.handle, context.source)
                {
                    self.cancel_pending(view.handle);
                    self.reset_view(view, true)?;
                }
            }
        }
        let mut game_set = false;
        for (key, value) in qa_core::text::info_pairs(info) {
            let name = std::str::from_utf8(key).map_err(|_| WriteError::InvalidInfo)?;
            let value = std::str::from_utf8(value).map_err(|_| WriteError::InvalidInfo)?;
            if name.is_empty() {
                break;
            }
            game_set |= name.eq_ignore_ascii_case("fs_game");
            let view = match self.bind(name, context) {
                Some(view) => view,
                None => self.register(name, Some(value), USER_CREATED, context)?,
            };
            // A remote setting cannot seize a physical host control shared
            // with other CLIENTs. Its wire text remains in their gamestate.
            if self.column(view.handle) != 0
                && self.values[view.handle.0 as usize].row
                    != self.values[self.download_handle.0 as usize].row
            {
                // TypeScript's CLIENT retains the user's download choice when
                // applying systeminfo; a server cannot enable that permission.
                self.force_write(view, value)?;
            }
        }
        if !game_set {
            let view = self.register("fs_game", Some(""), 0, context)?;
            self.force_write(view, "")?;
        }
        Ok(())
    }

    pub fn client_view(&self, mut view: View, owner: (ClientId, EntityId)) -> Option<View> {
        view.context.client_lifetime = Some(owner);
        view.context.cvar_seat = None;
        view.handle = self.context_handle(view.handle, view.context)?;
        Some(view)
    }
    pub fn client_views<const N: usize>(
        &self,
        mut views: [View; N],
        owner: (ClientId, EntityId),
    ) -> Option<[View; N]> {
        for view in &mut views {
            *view = self.client_view(*view, owner)?;
        }
        Some(views)
    }

    pub fn client_handles<const N: usize>(
        &self,
        handles: [CvarHandle; N],
        owner: (ClientId, EntityId),
    ) -> Option<[CvarHandle; N]> {
        let column = self.context_column(Context {
            client_lifetime: Some(owner),
            cvar_seat: None,
            side: Scope::Client,
            ..self.context
        })?;
        Some(handles.map(|handle| self.selected_handle(handle, column)))
    }

    /// Cache a numeric engine handle in the same column as native View reads.
    /// No name lookup or string conversion occurs here.
    pub fn client_handle(
        &self,
        handle: CvarHandle,
        owner: (ClientId, EntityId),
    ) -> Option<CvarHandle> {
        self.context_handle(
            handle,
            Context {
                client_lifetime: Some(owner),
                cvar_seat: None,
                side: Scope::Client,
                ..self.context
            },
        )
    }
}
