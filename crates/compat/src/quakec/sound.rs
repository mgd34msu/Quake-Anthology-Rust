//! QuakeC start-sound admission shared by builtins and native entity motion.
use super::Vm;
use crate::{
    entities::EntityProjection,
    memory::ModuleMemory,
    services::{CallContext, CallError, ENGINE_CALLS, EngineServices},
};
use qa_core::{
    primitives::{IndexSpan, NativeDelivery, NativeTarget, RuleSetId, SoundEmitter, SoundEvent},
    sys_events::EventTime,
    text::FixedText,
};

pub(crate) fn precached_sound(
    services: &mut EngineServices<'_>,
    context: CallContext,
    name: &[u8],
    prefix: &[u8],
    suffix: &[u8],
) -> Result<Option<(IndexSpan, u32)>, CallError> {
    let sounds = services
        .storage
        .resources(context.module)
        .ok_or(CallError::ConfigString)?[1];
    if let Some(ordinal) = (ENGINE_CALLS.resource_find)(services, context.module, sounds, name)? {
        return Ok(Some((sounds, ordinal)));
    }
    let mut text = FixedText::<8240>::default();
    text.append_bytes(prefix).map_err(|_| CallError::Capacity)?;
    text.append_bytes(name).map_err(|_| CallError::Capacity)?;
    text.append_bytes(suffix).map_err(|_| CallError::Capacity)?;
    match (ENGINE_CALLS.console)(services, text.as_bytes()) {
        Ok(_) | Err(CallError::Capacity) => Ok(None),
        Err(error) => Err(error),
    }
}

impl Vm {
    pub fn start_sound(
        &mut self,
        services: &mut EngineServices<'_>,
        context: CallContext,
        platform_time: EventTime,
        native: u32,
        name: &[u8],
        channel: i32,
        volume: i32,
        attenuation: f32,
    ) -> Result<(), CallError> {
        start_sound(
            &mut self.projection,
            &self.entities,
            &self.strings,
            services,
            context,
            platform_time,
            native,
            || Ok(name),
            channel,
            volume,
            attenuation,
        )
    }

    pub(crate) fn start_sound_string(
        &mut self,
        services: &mut EngineServices<'_>,
        context: CallContext,
        platform_time: EventTime,
        native: u32,
        name: u32,
        channel: i32,
        volume: i32,
        attenuation: f32,
    ) -> Result<(), CallError> {
        start_sound(
            &mut self.projection,
            &self.entities,
            &self.strings,
            services,
            context,
            platform_time,
            native,
            || Ok(self.strings.cstring(u64::from(name))?),
            channel,
            volume,
            attenuation,
        )
    }
}

fn start_sound<'name>(
    projection: &mut EntityProjection,
    entities: &ModuleMemory<'_>,
    strings: &ModuleMemory<'_>,
    services: &mut EngineServices<'_>,
    context: CallContext,
    platform_time: EventTime,
    native: u32,
    name: impl FnOnce() -> Result<&'name [u8], CallError>,
    channel: i32,
    volume: i32,
    attenuation: f32,
) -> Result<(), CallError> {
    let rules = context.native_rules;
    let maximum_channel = if rules == RuleSetId::QuakeWorld {
        15
    } else {
        7
    };
    if !(0..=maximum_channel).contains(&channel)
        || !(0..=255).contains(&volume)
        || !(0.0..=4.0).contains(&attenuation)
    {
        return Err(CallError::Text);
    }
    let Some((sounds, ordinal)) = precached_sound(
        services,
        context,
        name()?,
        b"SV_StartSound: ",
        b" not precached\n",
    )?
    else {
        return Ok(());
    };
    let Some(sound) = (ENGINE_CALLS.sound_id)(services, context.module, sounds, ordinal as i32)?
    else {
        return Ok(());
    };
    let (entity, position, positioned) =
        projection.quakec_sound_source(services, entities, strings, context, native)?;
    let hearing = rules == RuleSetId::QuakeWorld
        && channel & 8 == 0
        && services.storage.uses_sound_phs(services.cvars);
    let reliable = rules == RuleSetId::QuakeWorld && channel & 8 != 0;
    let delivery = NativeDelivery {
        module: context.module,
        source: rules,
        target: if hearing {
            NativeTarget::Hearing
        } else {
            NativeTarget::All
        },
        origin: position,
        reliable,
        dupe_key: 0,
        component: 0,
        emission: None,
        check_portals: false,
    };
    let event = SoundEvent {
        sound,
        source_ordinal: Some(ordinal),
        spatial: context.sound_policy,
        entity: Some(SoundEmitter::Entity(entity)),
        position,
        channel: channel & 7,
        queued_at: platform_time,
        volume: volume as f32 / 255.0,
        attenuation,
        positioned,
        ..SoundEvent::default()
    };
    match (ENGINE_CALLS.sound)(services, delivery, event) {
        Ok(_) | Err(CallError::Capacity) => Ok(()),
        Err(error) => Err(error),
    }
}
