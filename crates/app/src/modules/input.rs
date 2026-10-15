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
    fn input_recipient(&self, seat: SeatId, catcher: u32) -> Option<Recipient> {
        let defaults = self.modules.as_ref()?.input[seat.index()];
        let fallback = if catcher == key_catchers::UI {
            defaults.ui
        } else {
            defaults.cgame
        };
        let view = (catcher == key_catchers::CGAME)
            .then(|| {
                let client = self.local_clients[seat.index()]?;
                self.runtime.snapshots[client.0 as usize]
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
        .into_iter()
        .flatten()
        .find_map(|module| self.input_binding(module, seat, catcher))
    }

    fn input_binding(&self, module: ModuleId, seat: SeatId, catcher: u32) -> Option<Recipient> {
        let row = self.module_row(module)?;
        let input = row.input?;
        if row.state != State::Running
            || row.request.phase != Phase::Client
            || row.request.context.console.seat != seat
            || input.catcher != catcher
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
            if connected.connection.is_none() || connected.entity != lifetime {
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
        Some(Recipient {
            module,
            input,
            time,
        })
    }

    fn input_export(&mut self, recipient: Recipient, callback: u32, words: &[u64]) {
        let result = self.call_module_export(
            recipient.module,
            Export::words(CallbackId(callback), words),
            recipient.time,
        );
        if result.is_err() && self.module_state(recipient.module) == Some(State::Running) {
            if let Some(row) = self.module_row_mut(recipient.module) {
                row.state = State::Failed;
            }
            self.disconnect_module_recipients(recipient.module);
            self.retire_module_state(recipient.module);
        }
    }

    pub(crate) fn module_input(&mut self, atom: InputAtom) -> InputCapture {
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
                let recipient = self.input_recipient(seat, key_catchers::CGAME);
                self.runtime
                    .input
                    .set_catcher(seat, key_catchers::CGAME, false, time);
                if let Some(recipient) = recipient
                    && let Some(callback) = recipient.input.event_handling
                {
                    self.input_export(recipient, callback, &[0]);
                }
                return InputCapture::Caught;
            }
            if catchers & key_catchers::UI == 0
                && let Some(recipient) = self.input_recipient(seat, key_catchers::UI)
                && let Some(callback) = recipient.input.menu
            {
                let active = self.local_clients[seat.index()]
                    .and_then(|id| self.runtime.snapshots[id.0 as usize].as_ref())
                    .is_some_and(|snapshot| {
                        snapshot.applied != 0 && snapshot.interpolation().is_some()
                    });
                self.runtime.input.set_text_seat(seat);
                self.input_export(recipient, callback, &[if active { 2 } else { 1 }]);
                return InputCapture::Caught;
            }
        }
        let catcher = if catchers & key_catchers::UI != 0 {
            key_catchers::UI
        } else if catchers & key_catchers::CGAME != 0 {
            key_catchers::CGAME
        } else {
            return InputCapture::Pass;
        };
        if let Some(recipient) = self.input_recipient(seat, catcher) {
            match atom {
                InputAtom::Key { source, down, .. } => {
                    if let Some(key) = qa_input::keys::native_number(source.control) {
                        self.input_export(
                            recipient,
                            recipient.input.key,
                            &[u64::from(key), u64::from(down)],
                        );
                    }
                }
                InputAtom::Character { value, .. } if recipient.input.character => {
                    self.input_export(
                        recipient,
                        recipient.input.key,
                        &[u64::from(u32::from(value) | 1024), 1],
                    );
                }
                InputAtom::Character { .. } => {}
                InputAtom::Mouse { dx, dy, .. } => {
                    self.input_export(recipient, recipient.input.mouse, &[dx as u64, dy as u64]);
                }
            }
        }
        // A failed/missing recipient never turns the caught physical press
        // into a gameplay command during the same dispatch continuation.
        InputCapture::Caught
    }

    pub(crate) fn native_input_wants_mouse(&self) -> bool {
        SeatId::ALL.into_iter().any(|seat| {
            let mask = self.runtime.input.key_catchers(seat);
            mask & key_catchers::CONSOLE == 0
                && [key_catchers::UI, key_catchers::CGAME]
                    .into_iter()
                    .any(|bit| mask & bit != 0 && self.input_recipient(seat, bit).is_some())
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
                        && self.input_recipient(seat, key_catchers::UI).is_some()
                })
            });
            self.runtime
                .input
                .set_text_seat(seat.unwrap_or(SeatId::FIRST));
        }
    }
}
