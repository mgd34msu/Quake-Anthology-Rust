//! CLIENT columns share names, bindings, conversions and the ordinary value path.
use super::*;
use qa_core::primitives::{ClientId, EntityId};

pub(super) struct ClientColumn {
    owner: (ClientId, EntityId),
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
        cvar_client: None,
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
        column.checked_sub(1).map(|index| self.clients[index].owner)
    }
    pub(super) fn context_column(&self, context: Context) -> Option<usize> {
        if context.side == Scope::Server {
            return Some(0);
        }
        match context.cvar_client.or(context.client_lifetime) {
            None => Some(0),
            Some(owner) => self
                .clients
                .iter()
                .position(|row| row.owner == owner)
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
        self.clients.iter().any(|row| row.owner == owner)
    }

    /// Reserve a connection's value column at load, before any of its modules
    /// bind cvars. Rebinding the same lifetime is a no-op. A reused client slot
    /// discards only that connection's values, latches and alias details.
    pub fn load_client_values(&mut self, owner: (ClientId, EntityId)) -> Result<(), WriteError> {
        if self.clients.iter().any(|row| row.owner == owner) {
            return Ok(());
        }
        self.refresh_changes();
        let previous = self.clients.iter().position(|row| row.owner.0 == owner.0);
        let column = previous.map_or(self.texts.len(), |index| index + 1);
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
            self.clients.push(ClientColumn { owner });
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
            self.clients[column - 1].owner = owner;
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
        self.refresh_changes();
        Ok(())
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
            cvar_client: None,
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
        view.context.cvar_client = None;
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
            cvar_client: None,
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
                cvar_client: None,
                side: Scope::Client,
                ..self.context
            },
        )
    }
}
