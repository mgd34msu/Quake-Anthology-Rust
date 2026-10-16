//! Load-selected module input exports consume the current system-event atom.
use super::{Export, Module, Phase, State};
use crate::host::FrameHost;
use qa_compat::abi::ClientInputExports;
use qa_core::{
    primitives::{CallbackId, ModuleId, ThinkTime},
    sys_events::{SeatId, key_catchers},
};
use qa_input::{InputAtom, InputCapture};

#[derive(Clone, Copy, Default)]
pub(super) struct Recipients {
    ui: Option<ModuleId>,
    cgame: Option<ModuleId>,
}

pub(super) fn recipients(rows: &[Option<Module>]) -> [Recipients; SeatId::COUNT] {
    let mut recipients = [Recipients::default(); SeatId::COUNT];
    for row in rows.iter().flatten() {
        let Some(input) = row.input.filter(|_| row.request.phase == Phase::Client) else {
            continue;
        };
        let seat = &mut recipients[row.request.context.console.seat.index()];
        let target = if input.menu.is_some() {
            &mut seat.ui
        } else {
            &mut seat.cgame
        };
        target.get_or_insert(row.request.context.module);
    }
    recipients
}

#[derive(Clone, Copy)]
struct Recipient {
    module: ModuleId,
    input: ClientInputExports,
    time: ThinkTime,
}

impl FrameHost {
    fn input_recipient(
        &self,
        seat: SeatId,
        catcher: u32,
        active: bool,
        lifecycle: Option<ModuleId>,
    ) -> Option<Recipient> {
        self.client_module_candidates(seat, catcher)
            .into_iter()
            .flatten()
            .find_map(|module| self.input_binding(module, seat, catcher, active, lifecycle))
    }

    pub(super) fn client_module_candidates(
        &self,
        seat: SeatId,
        catcher: u32,
    ) -> [Option<ModuleId>; 3] {
        let defaults = self
            .modules
            .as_ref()
            .map(|modules| modules.input[seat.index()])
            .unwrap_or_default();
        let fallback = if catcher == key_catchers::UI {
            defaults.ui
        } else {
            defaults.cgame
        };
        let view = (catcher == key_catchers::CGAME)
            .then(|| {
                let client = self.local_clients[seat.index()]?;
                self.runtime
                    .snapshots
                    .get(client.0 as usize)?
                    .as_ref()?
                    .client_view_owner()
                    .map(|(module, _)| module)
            })
            .flatten();
        [
            self.runtime.input.catcher_module(seat, catcher),
            view,
            fallback,
        ]
    }

    fn input_binding(
        &self,
        module: ModuleId,
        seat: SeatId,
        catcher: u32,
        active: bool,
        lifecycle: Option<ModuleId>,
    ) -> Option<Recipient> {
        let row = self.module_row(module)?;
        let input = row.input?;
        if input.catcher != catcher {
            return None;
        }
        let time = self.client_module_time_scoped(module, seat, active, lifecycle)?;
        Some(Recipient {
            module,
            input,
            time,
        })
    }

    pub(super) fn client_module_time_scoped(
        &self,
        module: ModuleId,
        seat: SeatId,
        active: bool,
        lifecycle: Option<ModuleId>,
    ) -> Option<ThinkTime> {
        let row = self.module_row(module)?;
        let lifecycle = lifecycle == Some(module) && active && row.request.program.is_active();
        if !row.ready_for_callback(active)
            || !lifecycle && !self.module_generation_ready(module)
            || row.request.phase != Phase::Client
            || row.request.context.console.seat != seat
            || self
                .runtime
                .server
                .entities
                .resolve(row.request.anchor)
                .is_none()
        {
            return None;
        }
        let time = if let Some((client, lifetime)) = row.client_connection {
            let connected = self.runtime.server.clients.get(client.0 as usize)?;
            if !connected.is_active() || connected.entity != lifetime {
                return None;
            }
            self.runtime
                .snapshots
                .get(client.0 as usize)?
                .as_ref()?
                .interpolation()
                .map_or(row.request.context.clock, |sample| sample.time)
        } else {
            row.time(self.time)
        };
        Some(time)
    }

    fn input_export(
        &mut self,
        recipient: Recipient,
        callback: u32,
        words: &[u64],
        driver: &mut crate::modules::Driver<'_>,
    ) {
        let result = self.call_module_export_with_driver(
            recipient.module,
            Export::words(CallbackId(callback), words),
            recipient.time,
            driver,
        );
        if result.is_err() {
            self.fail_input_recipient(recipient.module, driver);
        }
    }

    pub(crate) fn module_input_with_driver(
        &mut self,
        atom: InputAtom,
        driver: &mut crate::modules::Driver<'_>,
    ) -> InputCapture {
        if matches!(
            atom,
            InputAtom::Character {
                value: '`' | '~',
                ..
            }
        ) {
            return InputCapture::Caught;
        }
        let seat = match atom {
            InputAtom::Key { seat, .. }
            | InputAtom::Character { seat, .. }
            | InputAtom::Mouse { seat, .. } => seat,
        };
        let catchers = self.runtime.input.key_catchers(seat);
        if let InputAtom::Key {
            source,
            time,
            down: true,
            ..
        } = atom
            && qa_input::keys::native_number(source.control) == Some(27)
        {
            if catchers & key_catchers::CGAME != 0 {
                let recipient = self.input_recipient(seat, key_catchers::CGAME, false, None);
                self.runtime
                    .input
                    .set_catcher(seat, key_catchers::CGAME, false, time);
                if let Some(recipient) = recipient
                    && let Some(callback) = recipient.input.event_handling
                {
                    self.input_export(recipient, callback, &[0], driver);
                }
                return InputCapture::Caught;
            }
            if catchers & key_catchers::UI == 0
                && let Some(recipient) = self.input_recipient(seat, key_catchers::UI, false, None)
                && let Some(callback) = recipient.input.menu
            {
                let active = self.local_clients[seat.index()]
                    .and_then(|id| self.runtime.snapshots[id.0 as usize].as_ref())
                    .is_some_and(|snapshot| {
                        snapshot.applied != 0 && snapshot.interpolation().is_some()
                    });
                self.runtime.input.set_text_seat(seat);
                self.input_export(recipient, callback, &[if active { 2 } else { 1 }], driver);
                return InputCapture::Caught;
            }
        }
        if catchers & (key_catchers::UI | key_catchers::CGAME) == 0 {
            return InputCapture::Pass;
        }
        if let Some((module, callback, call)) = self.input_call(atom, false, None)
            && self
                .call_module_with_driver(callback, call, driver)
                .is_err()
        {
            self.fail_input_recipient(module, driver);
        }
        // A failed/missing recipient never turns the caught physical press
        // into a gameplay command during the same dispatch continuation.
        InputCapture::Caught
    }

    /// Ordinary atoms and synchronous clears share recipient and export
    /// preparation. Only a native clear may reach an actually active initializer.
    pub(super) fn input_call(
        &self,
        atom: InputAtom,
        active: bool,
        lifecycle: Option<ModuleId>,
    ) -> Option<(
        ModuleId,
        CallbackId,
        qa_core::primitives::CallbackCall<'static>,
    )> {
        let seat = match atom {
            InputAtom::Key { seat, .. }
            | InputAtom::Character { seat, .. }
            | InputAtom::Mouse { seat, .. } => seat,
        };
        let mask = self.runtime.input.key_catchers(seat);
        let catcher = if mask & key_catchers::UI != 0 {
            key_catchers::UI
        } else if mask & key_catchers::CGAME != 0 {
            key_catchers::CGAME
        } else {
            return None;
        };
        let recipient = self.input_recipient(seat, catcher, active, lifecycle)?;
        let (callback, words) = match atom {
            InputAtom::Key { source, down, .. } => (
                recipient.input.key,
                [
                    u64::from(qa_input::keys::native_number(source.control)?),
                    u64::from(down),
                ],
            ),
            InputAtom::Character { value, .. } if recipient.input.character => {
                (recipient.input.key, [u64::from(u32::from(value) | 1024), 1])
            }
            InputAtom::Character { .. } => return None,
            InputAtom::Mouse { dx, dy, .. } => (recipient.input.mouse, [dx as u64, dy as u64]),
        };
        let row = self.module_row(recipient.module)?;
        let mut arguments = [0; 9];
        arguments[..2].copy_from_slice(&words);
        Some((
            recipient.module,
            CallbackId(callback),
            qa_core::primitives::CallbackCall::Export {
                entity: row.request.anchor,
                time: recipient.time,
                arguments,
                client: None,
                command: Default::default(),
            },
        ))
    }

    pub(super) fn fail_input_recipient(
        &mut self,
        module: ModuleId,
        driver: &mut crate::modules::Driver<'_>,
    ) {
        if let Some(row) = self.module_row_mut(module)
            && row.state.can_invoke()
        {
            row.state = State::Failed;
            self.cleanup_failed_module(module, driver);
        }
    }

    pub(crate) fn native_input_wants_mouse(&self) -> bool {
        SeatId::ALL.into_iter().any(|seat| {
            let mask = self.runtime.input.key_catchers(seat);
            mask & key_catchers::CONSOLE == 0
                && [key_catchers::UI, key_catchers::CGAME]
                    .into_iter()
                    .any(|bit| {
                        mask & bit != 0 && self.input_recipient(seat, bit, false, None).is_some()
                    })
        })
    }

    pub(super) fn retire_module_input(&mut self, module: ModuleId) {
        if let Some(seat) = self.console.editor.seat()
            && self
                .runtime
                .input
                .catcher_module(seat, key_catchers::CONSOLE)
                == Some(module)
        {
            self.console.editor.set_active(seat, false);
        }
        self.runtime
            .input
            .retire_catcher_module(module, self.runtime.input_time);
        self.console
            .sync_editor_input(&mut self.runtime.input, self.runtime.input_time);
        let focus = self.runtime.input.text_seat();
        if self.runtime.input.key_catchers(focus)
            & (key_catchers::CONSOLE | key_catchers::UI | key_catchers::MESSAGE)
            == 0
        {
            let seat = self.console.editor.seat().or_else(|| {
                SeatId::ALL.into_iter().find(|&seat| {
                    self.runtime.input.key_catchers(seat) & key_catchers::UI != 0
                        && self
                            .input_recipient(seat, key_catchers::UI, false, None)
                            .is_some()
                })
            });
            self.runtime
                .input
                .set_text_seat(seat.unwrap_or(SeatId::FIRST));
        }
    }
}
