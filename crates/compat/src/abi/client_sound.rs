//! Q3 CLIENT imports decode into the shared sound event and CLIENT publisher.
use super::Invocation;
use crate::services::{CallError, ENGINE_CALLS};
use qa_core::primitives::{RuleSetId, SoundAction, SoundBus, SoundEvent, SoundId};

fn integer(c: &Invocation<'_, '_, '_>, index: usize) -> Result<i32, CallError> {
    Ok(c.arg(index)? as u32 as i32)
}
fn entity(c: &Invocation<'_, '_, '_>, index: usize) -> Result<i32, CallError> {
    let value = integer(c, index)?;
    (0..1024)
        .contains(&value)
        .then_some(value)
        .ok_or(CallError::Entity)
}
fn sound(
    c: &mut Invocation<'_, '_, '_>,
    index: usize,
) -> Result<Option<(SoundId, u32)>, CallError> {
    let handle = integer(c, index)?;
    let sound = (ENGINE_CALLS.registered_sound)(c.services, handle)?;
    if sound.is_none() {
        let _ = (ENGINE_CALLS.console_formatted)(
            c.services,
            format_args!("native CLIENT sound: handle {handle} is unavailable\n"),
        );
    }
    Ok(sound.map(|sound| (sound, handle as u32)))
}
fn completed(result: Result<(), CallError>) -> Result<u64, CallError> {
    match result {
        Ok(()) | Err(CallError::Capacity) => Ok(0),
        Err(error) => Err(error),
    }
}
fn event(c: &Invocation<'_, '_, '_>, action: SoundAction) -> SoundEvent {
    SoundEvent {
        action,
        queued_at: c.platform_time,
        spatial: c.context.sound_policy,
        listener_owner: Some(c.context.module),
        volume: 1.0,
        attenuation: 1.0,
        ..SoundEvent::default()
    }
}
pub(super) fn play(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let positioned = !c.null_pointer(0)?;
    // Original StartSound bounds-checks only dynamically sourced entity rows.
    let native_entity = if positioned {
        integer(c, 1)?
    } else {
        entity(c, 1)?
    };
    let Some((sound, ordinal)) = sound(c, 3)? else {
        return Ok(0);
    };
    let mut event = event(c, SoundAction::Play);
    event.sound = sound;
    event.source_ordinal = Some(ordinal);
    event.channel = integer(c, 2)?;
    event.positioned = positioned;
    if positioned {
        event.position = c.vector(0, false)?;
    }
    event.playback = qa_gameplay::rules::native_sound_playback(RuleSetId::Quake3);
    completed((ENGINE_CALLS.client_sound)(
        c.services,
        event,
        Some(native_entity),
    ))
}
pub(super) fn local<const UI: bool>(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let Some((sound, ordinal)) = sound(c, 0)? else {
        return Ok(0);
    };
    let mut event = event(c, SoundAction::Play);
    event.sound = sound;
    event.source_ordinal = Some(ordinal);
    event.channel = integer(c, 1)?;
    event.bus = SoundBus::Local;
    event.playback = qa_gameplay::rules::native_sound_playback(RuleSetId::Quake3);
    if UI {
        completed((ENGINE_CALLS.local_sound)(c.services, event))
    } else {
        completed((ENGINE_CALLS.client_sound)(c.services, event, None))
    }
}
pub(super) fn clear(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let action = if integer(c, 0)? == 0 {
        SoundAction::ClearTemporaryLoops
    } else {
        SoundAction::ClearClientLoops
    };
    let event = event(c, action);
    completed((ENGINE_CALLS.client_sound)(c.services, event, None))
}
pub(super) fn add_loop<const PERSISTENT: bool>(
    c: &mut Invocation<'_, '_, '_>,
) -> Result<u64, CallError> {
    let Some((sound, ordinal)) = sound(c, 3)? else {
        return Ok(0);
    };
    let native_entity = entity(c, 0)?;
    let origin = c.vector(1, false)?;
    let velocity = c.vector(2, false)?;
    let mut event = event(c, SoundAction::StartLoop);
    event.sound = sound;
    event.source_ordinal = Some(ordinal);
    event.position = origin;
    event.velocity = velocity;
    event.positioned = false;
    event.looping = Some(
        qa_gameplay::rules::loop_rules(RuleSetId::Quake3)
            .ok_or(CallError::Unsupported)?
            .native(PERSISTENT),
    );
    completed((ENGINE_CALLS.client_sound)(
        c.services,
        event,
        Some(native_entity),
    ))
}
pub(super) fn position(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let native_entity = entity(c, 0)?;
    let mut event = event(c, SoundAction::UpdateEmitterPosition);
    event.position = c.vector(1, false)?;
    completed((ENGINE_CALLS.client_sound)(
        c.services,
        event,
        Some(native_entity),
    ))
}
pub(super) fn listener(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let native_entity = entity(c, 0)? as u32;
    let origin = c.vector(1, false)?;
    let axes = c.required_pointer(2)?;
    c.memory.read(axes, 36)?;
    let right = c
        .memory
        .read_vec3(axes.checked_add(12).ok_or(CallError::Memory)?)?;
    if !right.is_finite() {
        return Err(CallError::Geometry);
    }
    // Original DMA does not consume the inwater argument.
    completed((ENGINE_CALLS.client_listener)(
        c.services,
        c.context.module,
        c.platform_time,
        native_entity,
        origin,
        right,
    ))
}
pub(super) fn stop_loop(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let native_entity = entity(c, 0)?;
    let event = event(c, SoundAction::StopLoop);
    completed((ENGINE_CALLS.client_sound)(
        c.services,
        event,
        Some(native_entity),
    ))
}
