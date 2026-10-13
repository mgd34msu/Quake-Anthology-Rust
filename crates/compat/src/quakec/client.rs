//! Cached QC player publication and QuakeWorld client lifecycle ABI.
use super::{Vm, definition};
use crate::services::{CallContext, CallError, EngineServices};
use qa_core::primitives::{
    CallbackId, ClientCallback, CvarHandle, EntityId, NativeEntity, RuleSetId, ThinkTime, UserCmd,
    ValueBinding, Vec3,
};

pub(super) struct ClientGlobals {
    pub rules: Option<RuleSetId>,
    callbacks: [Option<usize>; 6],
    parms: [Option<usize>; 16],
    settings: Option<[CvarHandle; 3]>,
    stat_globals: [Option<usize>; 4],
}

impl ClientGlobals {
    pub fn load(image: &qa_formats::program::quakec::Image) -> Self {
        Self {
            rules: None,
            callbacks: [
                b"SetNewParms".as_slice(),
                b"ClientConnect",
                b"PutClientInServer",
                b"PlayerPreThink",
                b"PlayerPostThink",
                b"ClientDisconnect",
            ]
            .map(|name| definition(image, false, name, 6)),
            parms: std::array::from_fn(|index| {
                let mut name = qa_core::text::FixedText::<8>::default();
                use std::fmt::Write;
                write!(&mut name, "parm{}", index + 1).ok()?;
                definition(image, false, name.as_bytes(), 2)
            }),
            settings: None,
            stat_globals: [
                b"total_secrets".as_slice(),
                b"total_monsters",
                b"found_secrets",
                b"killed_monsters",
            ]
            .map(|name| definition(image, false, name, 2)),
        }
    }
    fn require(&self) -> Result<(), CallError> {
        if self.rules == Some(RuleSetId::QuakeWorld) {
            Ok(())
        } else {
            Err(CallError::Unsupported)
        }
    }
    fn publication_rules(&self) -> Result<RuleSetId, CallError> {
        match self.rules {
            Some(rules @ (RuleSetId::Quake | RuleSetId::QuakeWorld)) => Ok(rules),
            _ => Err(CallError::Unsupported),
        }
    }
    fn callback(&self, event: ClientCallback) -> Result<usize, CallError> {
        self.require()?;
        let index = match event {
            ClientCallback::NewParameters => 0,
            ClientCallback::Connect => 1,
            ClientCallback::Begin => 2,
            ClientCallback::BeforeMove => 3,
            ClientCallback::AfterMove => 4,
            ClientCallback::Disconnect => 5,
            ClientCallback::Command | ClientCallback::UserinfoChanged => {
                return Err(CallError::Unsupported);
            }
        };
        self.callbacks[index].ok_or(CallError::Memory)
    }
}

impl Vm {
    pub fn client_count(&self) -> Result<usize, CallError> {
        self.projection.quakec_client_count()
    }
    pub fn bind_client(
        &mut self,
        entities: &qa_world::entities::EntityTable,
        native: NativeEntity,
        entity: EntityId,
    ) -> Result<(), CallError> {
        self.client_globals.require()?;
        self.projection.quakec_bind_client(entities, native, entity)
    }
    /// Resolve numeric handles/registry mappings only at the cold module boundary.
    pub fn bind_client_settings(
        &mut self,
        vars: &qa_console::cvars::Cvars,
        registry: &qa_gameplay::registry::Registry,
    ) -> Result<(), CallError> {
        let source = self.client_globals.publication_rules()?;
        let handle = |name| vars.find(name).ok_or(CallError::Cvar);
        self.client_globals.settings = Some([
            handle("sv_maxspeed")?,
            handle("cl_rollangle")?,
            handle("cl_rollspeed")?,
        ]);
        self.projection
            .quakec_bind_client_registry(registry, source)
    }
    pub fn bind_player_values(
        &mut self,
        stats: &[ValueBinding],
        items: Option<ValueBinding>,
    ) -> Result<(), CallError> {
        let source = self.client_globals.publication_rules()?;
        let items = if source == RuleSetId::QuakeWorld {
            stats.get(15).copied()
        } else {
            items
        };
        self.projection.quakec_bind_player_values(stats, items)
    }
    pub fn client_callback(&self, event: ClientCallback) -> Result<CallbackId, CallError> {
        let global = self.client_globals.callback(event)?;
        self.projection.quakec_callback(self.image.globals[global])
    }
    pub fn prepare_client_call(
        &mut self,
        event: ClientCallback,
        native: u32,
        time: ThinkTime,
    ) -> Result<(), CallError> {
        self.client_globals.callback(event)?;
        if event == ClientCallback::NewParameters {
            return Ok(());
        }
        self.projection
            .quakec_client_address(&self.entities, native)?;
        let self_at = self.call_fields.self_global.ok_or(CallError::Memory)?;
        let time_at = if event == ClientCallback::Disconnect {
            None
        } else {
            Some(self.call_fields.time_global.ok_or(CallError::Memory)?)
        };
        self.image.globals[self_at] = native;
        if let Some(at) = time_at {
            self.image.globals[at] = (time.seconds() as f32).to_bits();
        }
        Ok(())
    }
    pub fn read_spawn_parms(&self) -> Result<[u32; 16], CallError> {
        self.client_globals.require()?;
        let mut words = [0; 16];
        for (word, field) in words.iter_mut().zip(self.client_globals.parms) {
            *word = self.image.globals[field.ok_or(CallError::Memory)?];
        }
        Ok(words)
    }
    pub fn write_spawn_parms(&mut self, words: [u32; 16]) -> Result<(), CallError> {
        self.client_globals.require()?;
        if self.client_globals.parms.iter().any(Option::is_none) {
            return Err(CallError::Memory);
        }
        for (word, field) in words.into_iter().zip(self.client_globals.parms) {
            self.image.globals[field.ok_or(CallError::Memory)?] = word;
        }
        Ok(())
    }
    pub fn prepare_client_spawn(
        &mut self,
        services: &mut EngineServices<'_>,
        context: CallContext,
        native: u32,
        userinfo: &[u8],
    ) -> Result<(), CallError> {
        self.client_globals.require()?;
        if context.native_rules != RuleSetId::QuakeWorld
            || userinfo.len() >= super::QW_USERINFO_BYTES
            || userinfo.contains(&0)
        {
            return Err(CallError::Memory);
        }
        let settings = self.client_globals.settings.ok_or(CallError::Cvar)?;
        let maxspeed = services
            .cvars
            .native_number(settings[0], RuleSetId::QuakeWorld)
            .ok_or(CallError::Cvar)?;
        self.projection
            .quakec_validate_client_spawn(&self.entities, context, native)?;
        let name = qa_core::text::info_value(userinfo, b"name", false)
            .map_or(&b""[..], |range| &userinfo[range]);
        let name = self.insert_string(name).map_err(|_| CallError::Text)? as u32;
        self.projection.quakec_client_spawn(
            services,
            &mut self.entities,
            context,
            native,
            name,
            maxspeed,
        )
    }
    pub fn prepare_client_command(
        &mut self,
        services: &mut EngineServices<'_>,
        context: CallContext,
        native: u32,
        command: &UserCmd,
    ) -> Result<f64, CallError> {
        self.client_globals.require()?;
        let settings = self.client_globals.settings.ok_or(CallError::Cvar)?;
        let number = |handle| {
            services
                .cvars
                .native_number(handle, RuleSetId::QuakeWorld)
                .ok_or(CallError::Cvar)
        };
        let roll_angle = number(settings[1])?;
        let roll_speed = number(settings[2])?;
        let frame_time = self.call_fields.frame_time.ok_or(CallError::Memory)?;
        self.projection.quakec_client_command(
            &mut self.entities,
            context,
            native,
            command,
            roll_angle,
            roll_speed,
        )?;
        let step = (f64::from(command.duration_ms) * 0.001).min(0.1);
        self.image.globals[frame_time] = (step as f32).to_bits();
        Ok(step)
    }
    pub(super) fn sync_client_globals(&mut self) -> Result<(), CallError> {
        let number =
            |at| crate::numbers::native_integer(f32::from_bits(self.image.globals[at])) as u32;
        let server_flags = self.map_globals.server_flags.map_or(0, number);
        let extra_stats = self.client_globals.stat_globals.map(|at| at.map(number));
        self.projection
            .quakec_client_globals(server_flags, extra_stats)
    }
    pub fn read_client(
        &mut self,
        services: &mut EngineServices<'_>,
        context: CallContext,
        native: u32,
    ) -> Result<Vec3, CallError> {
        self.client_globals.require()?;
        self.sync_client_globals()?;
        self.projection
            .quakec_read_client(services, &self.entities, &self.strings, context, native)
    }
    pub fn write_client(
        &mut self,
        services: &mut EngineServices<'_>,
        context: CallContext,
        native: u32,
        offset: Vec3,
    ) -> Result<(), CallError> {
        self.client_globals.require()?;
        self.sync_client_globals()?;
        self.projection.quakec_write_client(
            services,
            &mut self.entities,
            &self.strings,
            context,
            native,
            offset,
        )
    }
    pub fn touch_callback(&self, native: u32) -> Result<Option<CallbackId>, CallError> {
        self.client_globals.require()?;
        self.projection
            .quakec_touch_callback(&self.entities, native)
    }
    pub fn client_set_angles(&self, native: u32) -> Result<Option<Vec3>, CallError> {
        self.client_globals.require()?;
        self.projection
            .quakec_client_fixangle(&self.entities, native)
    }
    pub fn clear_client_set_angles(&mut self, native: u32) -> Result<(), CallError> {
        self.client_globals.require()?;
        self.projection
            .quakec_clear_client_fixangle(&mut self.entities, native)
    }
    pub fn prepare_touch_call(
        &mut self,
        native_self: u32,
        native_other: u32,
        time: Option<ThinkTime>,
    ) -> Result<(), CallError> {
        self.client_globals.require()?;
        self.projection
            .quakec_check_address(&self.entities, native_self)?;
        self.projection
            .quakec_check_address(&self.entities, native_other)?;
        let self_at = self.call_fields.self_global.ok_or(CallError::Memory)?;
        let other_at = self.call_fields.other_global.ok_or(CallError::Memory)?;
        let time_at = time
            .map(|_| self.call_fields.time_global.ok_or(CallError::Memory))
            .transpose()?;
        self.image.globals[self_at] = native_self;
        self.image.globals[other_at] = native_other;
        if let (Some(at), Some(time)) = (time_at, time) {
            self.image.globals[at] = (time.seconds() as f32).to_bits();
        }
        Ok(())
    }
}
