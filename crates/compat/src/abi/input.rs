//! Q3 native input imports share the existing engine services and checked memory.
use super::{CallError, ENGINE_CALLS, Invocation};

pub(super) fn key_down(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    Ok(u64::from((ENGINE_CALLS.key_down)(
        c.services,
        c.context.console.seat,
        c.arg(0)? as u32 as i32,
    )))
}
pub(super) fn catchers(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    Ok(u64::from((ENGINE_CALLS.key_catchers)(
        c.services,
        c.context.console.seat,
    )))
}
pub(super) fn set_catchers(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    (ENGINE_CALLS.set_key_catchers)(
        c.services,
        c.context.console.seat,
        c.arg(0)? as u32,
        c.context.module,
        c.platform_time,
    )?;
    Ok(0)
}
pub(super) fn get_key(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let text = if c.null_pointer(0)? {
        None
    } else {
        Some(c.memory.cstring(c.pointer(0)?)?)
    };
    Ok((ENGINE_CALLS.key_for_binding)(c.services, text) as u32 as u64)
}
pub(super) fn binding(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let key = c.arg(0)? as u32 as i32;
    let output = c.required_pointer(1)?;
    let length = c.length(2)?;
    if length == 0 {
        return Err(CallError::Memory);
    }
    c.memory.read(output, length)?;
    let text = (ENGINE_CALLS.key_binding)(c.services, key);
    c.memory.write_string(output, length, text)?;
    Ok(0)
}
pub(super) fn set_binding(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let key = c.arg(0)? as u32 as i32;
    // Native Key_SetBinding(-1, ...) returns before consuming its pointer.
    if u8::try_from(key).is_err() {
        return Ok(0);
    }
    let text = c.memory.cstring(c.required_pointer(1)?)?;
    (ENGINE_CALLS.set_key_binding)(c.services, key, text, c.context.console, c.platform_time)?;
    Ok(0)
}
pub(super) fn name(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    let key = c.arg(0)? as u32 as i32;
    let output = c.required_pointer(1)?;
    let length = c.length(2)?;
    if length == 0 {
        return Err(CallError::Memory);
    }
    c.memory.read(output, length)?;
    let mut name = qa_core::text::FixedText::<32>::default();
    (ENGINE_CALLS.key_name)(key, &mut name)?;
    c.memory.write_string(output, length, name.as_bytes())?;
    Ok(0)
}
pub(super) fn overstrike(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    (ENGINE_CALLS.overstrike)(c.services).map(u64::from)
}
pub(super) fn set_overstrike(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    (ENGINE_CALLS.set_overstrike)(c.services, c.arg(0)? as u32 != 0)?;
    Ok(0)
}
pub(super) fn clear(c: &mut Invocation<'_, '_, '_>) -> Result<u64, CallError> {
    (ENGINE_CALLS.clear_keys)(
        c.services,
        c.context.console.seat,
        c.context.console,
        c.platform_time,
    )?;
    Ok(0)
}
