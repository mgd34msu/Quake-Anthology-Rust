//! Native key services borrow the shared device, bind, editor and command owners.
use super::{CallError, EngineServices};
use qa_console::{commands::BindOutput, views::Context};
use qa_core::{
    names::compare_folded,
    sys_events::{EventTime, SeatId, key_catchers},
    text::FixedText,
};
use qa_input::keys;

fn control(key: i32) -> Option<u16> {
    keys::from_native(u8::try_from(key).ok()?)
}

pub(super) fn key_name(key: i32, output: &mut FixedText<32>) -> Result<(), CallError> {
    keys::write_native_name(key, output).map_err(|_| CallError::Capacity)
}

impl EngineServices<'_> {
    pub(super) fn key_down(&self, seat: SeatId, key: i32) -> bool {
        control(key).is_some_and(|control| self.input.is_down(seat, control))
    }

    pub(super) fn key_binding(&self, key: i32) -> &[u8] {
        control(key)
            .and_then(|control| self.input.binding(control))
            .map_or(&[], qa_input::Binding::bytes)
    }

    pub(super) fn key_for_binding(&self, text: Option<&[u8]>) -> i32 {
        let Some(text) = text else { return -1 };
        for key in 0..=u8::MAX {
            if let Some(binding) = keys::from_native(key).and_then(|key| self.input.binding(key))
                && compare_folded(binding.bytes(), text).is_eq()
            {
                return i32::from(key);
            }
        }
        -1
    }

    pub(super) fn set_key_binding(
        &mut self,
        key: i32,
        text: &[u8],
        context: Context,
        time: EventTime,
    ) -> Result<(), CallError> {
        let Some(control) = control(key) else {
            return Ok(());
        };
        let mut output = BindOutput::new(self.commands, context);
        self.input
            .bind_bytes(control, text, time, &mut output)
            .map_err(|_| CallError::Capacity)?;
        let outcome = output.outcome();
        self.input_outcome(outcome);
        Ok(())
    }

    pub(super) fn set_key_catchers(
        &mut self,
        seat: SeatId,
        mask: u32,
        module: qa_core::primitives::ModuleId,
        time: EventTime,
    ) -> Result<(), CallError> {
        let editor = self.editor.as_deref_mut().ok_or(CallError::Unsupported)?;
        editor.set_active(seat, mask & key_catchers::CONSOLE != 0);
        let owner = editor.seat();
        self.input
            .replace_owned_key_catchers(seat, mask, module, time);
        for seat in SeatId::ALL {
            self.input
                .set_catcher(seat, key_catchers::CONSOLE, owner == Some(seat), time);
        }
        if let Some(seat) = owner {
            self.input.set_text_seat(seat);
        } else if mask & (key_catchers::UI | key_catchers::MESSAGE) != 0 {
            self.input.set_text_seat(seat);
        }
        Ok(())
    }

    pub(super) fn clear_keys(
        &mut self,
        seat: SeatId,
        context: Context,
        time: EventTime,
    ) -> Result<(), CallError> {
        let mut output = BindOutput::new(self.commands, context);
        self.input.clear_seat(seat, time, &mut output);
        let outcome = output.outcome();
        self.input_outcome(outcome);
        Ok(())
    }

    fn input_outcome(
        &mut self,
        (count, result): (u64, Result<(), qa_console::command_text::TextError>),
    ) {
        for _ in 0..count {
            self.server.events.reject_publication();
        }
        if let Err(error) = result {
            let _ = self
                .server
                .events
                .console_formatted(format_args!("Input command rejected: {error:?}\n"));
        }
    }
}
