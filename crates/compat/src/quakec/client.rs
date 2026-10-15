//! Cached QC player publication and native client lifecycle ABI.
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
    settings: Option<[CvarHandle; 5]>,
    stat_globals: [Option<usize>; 4],
}

/// Native callers restore only the globals they explicitly saved. The token
/// owns raw words, not another global-field cache or client state.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ClientCallRestore {
    self_word: Option<u32>,
    other_word: Option<u32>,
}
impl ClientCallRestore {
    pub(super) fn self_only(word: u32) -> Self {
        Self {
            self_word: Some(word),
            other_word: None,
        }
    }
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
    fn require(&self) -> Result<RuleSetId, CallError> {
        match self.rules {
            Some(rules @ (RuleSetId::Quake | RuleSetId::QuakeWorld)) => Ok(rules),
            _ => Err(CallError::Unsupported),
        }
    }
    fn require_packet_movement(&self) -> Result<(), CallError> {
        if self.require()? == RuleSetId::QuakeWorld {
            Ok(())
        } else {
            Err(CallError::Unsupported)
        }
    }
    fn require_frame_movement(&self, context: CallContext) -> Result<(), CallError> {
        if self.require()? == RuleSetId::Quake && context.native_rules == RuleSetId::Quake {
            Ok(())
        } else {
            Err(CallError::Unsupported)
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
        let source = self.client_globals.require()?;
        let handle = |name| vars.find(name).ok_or(CallError::Cvar);
        self.client_globals.settings = Some([
            handle("sv_maxspeed")?,
            handle("cl_rollangle")?,
            handle("cl_rollspeed")?,
            handle("sv_maxvelocity")?,
            handle("sv_gravity")?,
        ]);
        self.projection
            .quakec_bind_client_registry(registry, source)
    }
    pub fn bind_player_values(
        &mut self,
        stats: &[ValueBinding],
        items: Option<ValueBinding>,
    ) -> Result<(), CallError> {
        let source = self.client_globals.require()?;
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
    ) -> Result<ClientCallRestore, CallError> {
        self.client_globals.callback(event)?;
        if event == ClientCallback::NewParameters {
            return Ok(ClientCallRestore::default());
        }
        self.projection
            .quakec_client_address(&self.entities, native)?;
        let rules = self.client_globals.require()?;
        // NQ PutClientInServer follows ClientConnect without resetting the
        // globals that ClientConnect may have changed.
        if rules == RuleSetId::Quake && event == ClientCallback::Begin {
            return Ok(ClientCallRestore::default());
        }
        let self_at = self.call_fields.self_global.ok_or(CallError::Memory)?;
        let time_at = if event == ClientCallback::Disconnect {
            None
        } else {
            Some(self.call_fields.time_global.ok_or(CallError::Memory)?)
        };
        let restore = ClientCallRestore {
            self_word: (rules == RuleSetId::Quake && event == ClientCallback::Disconnect)
                .then_some(self.image.globals[self_at]),
            other_word: None,
        };
        self.image.globals[self_at] = native;
        if time_at.is_some() {
            self.set_time(time)?;
        }
        Ok(restore)
    }
    pub fn finish_client_call(&mut self, restore: ClientCallRestore) -> Result<(), CallError> {
        if let Some(word) = restore.self_word {
            let at = self.call_fields.self_global.ok_or(CallError::Memory)?;
            self.image.globals[at] = word;
        }
        if let Some(word) = restore.other_word {
            let at = self.call_fields.other_global.ok_or(CallError::Memory)?;
            self.image.globals[at] = word;
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
        colors: u8,
    ) -> Result<(), CallError> {
        let source = self.client_globals.require()?;
        if context.native_rules != source
            || userinfo.len() >= super::QW_USERINFO_BYTES
            || userinfo.contains(&0)
        {
            return Err(CallError::Memory);
        }
        let maxspeed = if source == RuleSetId::QuakeWorld {
            let settings = self.client_globals.settings.ok_or(CallError::Cvar)?;
            Some(
                services
                    .cvars
                    .native_number(settings[0], source)
                    .ok_or(CallError::Cvar)?,
            )
        } else {
            None
        };
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
            colors,
        )
    }
    pub fn receive_client_command(
        &mut self,
        context: CallContext,
        native: u32,
        command: &UserCmd,
    ) -> Result<(), CallError> {
        if self.client_globals.require()? != context.native_rules {
            return Err(CallError::Unsupported);
        }
        self.projection
            .quakec_receive_client_command(&mut self.entities, context, native, command)
    }
    pub fn client_phase(
        &self,
        context: CallContext,
        native: u32,
    ) -> Result<qa_core::primitives::Motion, CallError> {
        if self.client_globals.require()? != context.native_rules {
            return Err(CallError::Unsupported);
        }
        self.projection
            .quakec_client_phase(&self.entities, context, native)
    }
    pub fn max_velocity(&self, vars: &qa_console::cvars::Cvars) -> Result<f32, CallError> {
        let source = self.client_globals.require()?;
        let settings = self.client_globals.settings.ok_or(CallError::Cvar)?;
        vars.native_number(settings[3], source)
            .ok_or(CallError::Cvar)
    }
    pub fn motion_world_gravity(&self, vars: &qa_console::cvars::Cvars) -> Result<f32, CallError> {
        let source = self.client_globals.require()?;
        let settings = self.client_globals.settings.ok_or(CallError::Cvar)?;
        vars.native_number(settings[4], source)
            .ok_or(CallError::Cvar)
    }
    pub fn classname(&self, native: u32) -> Result<&[u8], CallError> {
        self.client_globals.require()?;
        self.projection
            .quakec_classname(&self.entities, &self.strings, native)
    }
    pub fn prepare_velocity(
        &mut self,
        entities: &mut qa_world::entities::EntityTable,
        context: CallContext,
        native: u32,
    ) -> Result<(), CallError> {
        if self.client_globals.require()? != context.native_rules {
            return Err(CallError::Unsupported);
        }
        self.projection
            .quakec_prepare_velocity(entities, &self.entities, context, native)
    }
    pub fn prepare_client_command(
        &mut self,
        services: &mut EngineServices<'_>,
        context: CallContext,
        native: u32,
        command: &UserCmd,
    ) -> Result<f64, CallError> {
        self.client_globals.require_packet_movement()?;
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
    pub fn prepare_control(
        &mut self,
        services: &mut EngineServices<'_>,
        context: CallContext,
        native: u32,
        seconds: f64,
    ) -> Result<Option<qa_movement::MoveOptions>, CallError> {
        self.client_globals.require_frame_movement(context)?;
        let settings = self.client_globals.settings.ok_or(CallError::Cvar)?;
        let number = |handle| {
            services
                .cvars
                .native_number(handle, context.native_rules)
                .ok_or(CallError::Cvar)
        };
        let roll = qa_gameplay::rules::appearance::AliasRoll {
            angle: number(settings[1])?,
            speed: number(settings[2])?,
        };
        self.projection
            .quakec_prepare_control(&mut self.entities, context, native, seconds, roll)
            .map(|angles| {
                angles.map(|body_angles| qa_movement::MoveOptions {
                    phase: qa_movement::MovePhase::Control {
                        body_angles,
                        clock: context.clock,
                    },
                    ..Default::default()
                })
            })
    }
    pub fn client_noclip(&self, context: CallContext, native: u32) -> Result<(), CallError> {
        self.client_globals.require_frame_movement(context)?;
        self.projection
            .quakec_client_noclip(&self.entities, context, native)
    }
    pub fn read_frame_client(
        &self,
        entities: &qa_world::entities::EntityTable,
        context: CallContext,
        native: u32,
        entity: EntityId,
        player: &mut qa_core::primitives::PlayerState,
    ) -> Result<(), CallError> {
        self.client_globals.require_frame_movement(context)?;
        self.projection.quakec_read_frame_client(
            entities,
            &self.entities,
            context,
            native,
            entity,
            player,
        )
    }
    pub fn write_frame_scalar(
        &mut self,
        entities: &qa_world::entities::EntityTable,
        context: CallContext,
        native: u32,
        entity: EntityId,
        player: &mut qa_core::primitives::PlayerState,
        stage: qa_core::primitives::MotionWrite,
    ) -> Result<(), CallError> {
        self.client_globals.require_frame_movement(context)?;
        self.projection.quakec_write_frame_scalar(
            entities,
            &mut self.entities,
            context,
            native,
            entity,
            player,
            stage,
        )
    }
    pub(super) fn sync_client_globals(&mut self) -> Result<(), CallError> {
        let number =
            |at| qa_core::math::native_integer(f32::from_bits(self.image.globals[at])) as u32;
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
        self.client_globals.require_packet_movement()?;
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
        self.client_globals.require_packet_movement()?;
        self.sync_client_globals()?;
        self.with_entity_stores(|projection, memory, strings, store| {
            projection
                .quakec_write_client(services, memory, strings, context, native, offset, store)
        })
    }
    pub fn touch_callback(
        &self,
        native: u32,
        trigger_only: bool,
    ) -> Result<Option<CallbackId>, CallError> {
        self.client_globals.require()?;
        self.projection
            .quakec_touch_callback(&self.entities, native, trigger_only)
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
    fn save_touch_globals(&self) -> Result<(ClientCallRestore, [usize; 2]), CallError> {
        let self_at = self.call_fields.self_global.ok_or(CallError::Memory)?;
        let other_at = self.call_fields.other_global.ok_or(CallError::Memory)?;
        Ok((
            ClientCallRestore {
                self_word: Some(self.image.globals[self_at]),
                other_word: Some(self.image.globals[other_at]),
            },
            [self_at, other_at],
        ))
    }
    /// Save impact globals before setting its clock, including aliased words.
    pub fn prepare_impact(&mut self, time: ThinkTime) -> Result<ClientCallRestore, CallError> {
        self.client_globals.require()?;
        let (restore, _) = self.save_touch_globals()?;
        self.set_time(time)?;
        Ok(restore)
    }
    pub fn prepare_touch_call(
        &mut self,
        native_self: u32,
        native_other: u32,
        time: Option<ThinkTime>,
    ) -> Result<ClientCallRestore, CallError> {
        self.client_globals.require()?;
        self.projection
            .quakec_check_address(&self.entities, native_self)?;
        self.projection
            .quakec_check_address(&self.entities, native_other)?;
        let (restore, [self_at, other_at]) = self.save_touch_globals()?;
        time.map(|_| self.call_fields.time_global.ok_or(CallError::Memory))
            .transpose()?;
        self.image.globals[self_at] = native_self;
        self.image.globals[other_at] = native_other;
        if let Some(time) = time {
            self.set_time(time)?;
        }
        Ok(restore)
    }

    pub fn set_time(&mut self, time: ThinkTime) -> Result<(), CallError> {
        let at = self.call_fields.time_global.ok_or(CallError::Memory)?;
        self.image.globals[at] = (time.seconds() as f32).to_bits();
        Ok(())
    }
}
