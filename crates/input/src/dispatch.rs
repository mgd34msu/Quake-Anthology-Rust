use super::*;
use qa_core::text::FixedText;
use std::{fmt::Write, str::Chars};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputCapture {
    Pass,
    Caught,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputAtom {
    Key {
        seat: SeatId,
        source: KeySource,
        time: EventTime,
        down: bool,
        repeat: bool,
    },
    Character {
        seat: SeatId,
        device: DeviceId,
        time: EventTime,
        value: char,
    },
    Mouse {
        seat: SeatId,
        device: DeviceId,
        time: EventTime,
        dx: i32,
        dy: i32,
    },
}

/// Dispatch progress borrows neither Input nor a binding across callbacks.
/// Text may borrow its system event; synthetic key clearing owns its state.
pub struct DispatchCursor<'event> {
    time: EventTime,
    state: State<'event>,
}
pub struct DispatchResume<'event> {
    cursor: DispatchCursor<'event>,
    pending: Pending,
}
pub enum DispatchStep<'event> {
    Done,
    Atom {
        atom: InputAtom,
        resume: DispatchResume<'event>,
    },
}

enum State<'event> {
    Event(EventKind<'event>),
    Text {
        device: DeviceId,
        chars: Chars<'event>,
    },
    Wheel {
        device: DeviceId,
        delta: [i32; 2],
        phase: u8,
    },
    Release {
        filter: ReleaseFilter,
        next: usize,
        cleanup: Cleanup,
    },
    ClearSeat {
        seat: SeatId,
        next_native: u16,
    },
    Done,
}
#[derive(Clone, Copy)]
enum ReleaseFilter {
    All,
    Mice,
    Device(DeviceId),
    UnmappedSeat(SeatId),
}
impl ReleaseFilter {
    fn matches(self, device: &Device, control: u16) -> bool {
        match self {
            Self::All => true,
            Self::Mice => matches!(device.id, Some(DeviceId::Mouse(_))),
            Self::Device(id) => device.id == Some(id),
            Self::UnmappedSeat(seat) => {
                device.seat == Some(seat) && keys::native_number(control).is_none()
            }
        }
    }
}
#[derive(Clone, Copy)]
enum Cleanup {
    None,
    Mouse,
    Focus,
    Device(DeviceId),
    Seat(SeatId),
}
enum Pending {
    Key {
        source: KeySource,
        seat: SeatId,
        index: usize,
        transition: bool,
        down: bool,
    },
    Mouse {
        device: DeviceId,
        seat: SeatId,
        dx: i32,
        dy: i32,
    },
    Character,
    Released,
}

impl DispatchCursor<'static> {
    /// Q3 Key_ClearStates visits native keys in byte order, releasing acquired
    /// bindings before each zero-time key-up callback. Physical aliases share
    /// one callback, while cleanup keeps the caller's current engine time.
    pub fn clear_seat(seat: SeatId, cleanup_time: EventTime) -> Self {
        Self {
            time: cleanup_time,
            state: State::ClearSeat {
                seat,
                next_native: 0,
            },
        }
    }
}

impl<'event> DispatchCursor<'event> {
    pub fn new(event: SysEvent<'event>) -> Self {
        Self {
            time: event.time,
            state: State::Event(event.kind),
        }
    }
    fn release(time: EventTime, filter: ReleaseFilter, cleanup: Cleanup) -> Self {
        Self {
            time,
            state: State::Release {
                filter,
                next: 0,
                cleanup,
            },
        }
    }
    fn atom(self, atom: InputAtom, pending: Pending) -> DispatchStep<'event> {
        DispatchStep::Atom {
            atom,
            resume: DispatchResume {
                cursor: self,
                pending,
            },
        }
    }
}

impl Input {
    pub fn dispatch(&mut self, event: SysEvent<'_>, target: &mut impl Target) {
        self.dispatch_cursor(DispatchCursor::new(event), target);
    }
    fn dispatch_cursor(&mut self, mut cursor: DispatchCursor<'_>, target: &mut impl Target) {
        loop {
            let DispatchStep::Atom { atom, resume } = self.advance(cursor, target) else {
                return;
            };
            let caught = match atom {
                InputAtom::Key {
                    seat,
                    source,
                    down,
                    repeat,
                    ..
                } => target.key(seat, source.control, down, repeat),
                InputAtom::Character { seat, value, .. } => {
                    target.character(seat, value);
                    true
                }
                InputAtom::Mouse { .. } => false,
            };
            cursor = self.resume(
                resume,
                if caught {
                    InputCapture::Caught
                } else {
                    InputCapture::Pass
                },
                target,
            );
        }
    }

    pub fn advance<'event>(
        &mut self,
        mut cursor: DispatchCursor<'event>,
        commands: &mut impl Target,
    ) -> DispatchStep<'event> {
        let time = cursor.time;
        loop {
            match std::mem::replace(&mut cursor.state, State::Done) {
                State::Event(kind) => match kind {
                    EventKind::Key {
                        device,
                        code,
                        down,
                        repeat,
                        ..
                    } => {
                        if let Some((atom, pending)) =
                            self.prepare_key(device, code, down, repeat, cursor.time, commands)
                        {
                            return cursor.atom(atom, pending);
                        }
                    }
                    EventKind::MouseButton {
                        device,
                        button,
                        down,
                    } => {
                        if let Some(control) = keys::mouse_button(button)
                            && let Some((atom, pending)) = self.prepare_key(
                                device,
                                control,
                                down,
                                false,
                                cursor.time,
                                commands,
                            )
                        {
                            return cursor.atom(atom, pending);
                        }
                    }
                    EventKind::ControllerButton {
                        device,
                        button,
                        down,
                    } if button < 32 => {
                        if let Some((atom, pending)) = self.prepare_key(
                            device,
                            544 + u16::from(button),
                            down,
                            false,
                            cursor.time,
                            commands,
                        ) {
                            return cursor.atom(atom, pending);
                        }
                    }
                    EventKind::MouseWheel { device, x, y } => {
                        cursor.state = State::Wheel {
                            device,
                            delta: [y, x],
                            phase: 0,
                        };
                    }
                    EventKind::Text { device, text } => {
                        cursor.state = State::Text {
                            device,
                            chars: text.chars(),
                        };
                    }
                    EventKind::Mouse { device, dx, dy } => {
                        if let Some(index) = self.ensure_device(device)
                            && let Some(seat) = self.devices[index].seat
                        {
                            return cursor.atom(
                                InputAtom::Mouse {
                                    seat,
                                    device,
                                    time,
                                    dx,
                                    dy,
                                },
                                Pending::Mouse {
                                    device,
                                    seat,
                                    dx,
                                    dy,
                                },
                            );
                        }
                    }
                    EventKind::ControllerAxis {
                        device,
                        axis,
                        value,
                    } => {
                        if let Some(index) = self.device_index(device)
                            && let Some(slot) = self.devices[index].axes.get_mut(usize::from(axis))
                        {
                            *slot = value;
                        }
                    }
                    EventKind::Time => self.seed(cursor.time),
                    EventKind::MouseCapture(_) => {
                        cursor.state = State::Release {
                            filter: ReleaseFilter::Mice,
                            next: 0,
                            cleanup: Cleanup::Mouse,
                        };
                    }
                    EventKind::Focus(false) => {
                        cursor.state = State::Release {
                            filter: ReleaseFilter::All,
                            next: 0,
                            cleanup: Cleanup::Focus,
                        };
                    }
                    EventKind::DeviceRemoved(id) => {
                        cursor.state = State::Release {
                            filter: ReleaseFilter::Device(id),
                            next: 0,
                            cleanup: Cleanup::Device(id),
                        };
                    }
                    _ => {}
                },
                State::Text { device, mut chars } => {
                    if let Some(value) = chars.next() {
                        cursor.state = State::Text { device, chars };
                        let seat = if device == DeviceId::Keyboard(0) {
                            Some(self.text_seat)
                        } else {
                            self.device_seat(device)
                        };
                        if let Some(seat) = seat {
                            return cursor.atom(
                                InputAtom::Character {
                                    seat,
                                    device,
                                    time,
                                    value,
                                },
                                Pending::Character,
                            );
                        }
                    }
                }
                State::Wheel {
                    device,
                    delta,
                    mut phase,
                } => {
                    while phase < 4 {
                        let axis = usize::from(phase / 2);
                        if delta[axis] == 0 {
                            phase += 2;
                            continue;
                        }
                        let control = match (axis, delta[axis] > 0) {
                            (0, true) => 576,
                            (0, false) => 577,
                            (_, true) => 579,
                            (_, false) => 578,
                        };
                        let down = phase % 2 == 0;
                        phase += 1;
                        cursor.state = State::Wheel {
                            device,
                            delta,
                            phase,
                        };
                        if let Some((atom, pending)) =
                            self.prepare_key(device, control, down, false, cursor.time, commands)
                        {
                            return cursor.atom(atom, pending);
                        }
                    }
                    cursor.state = State::Done;
                }
                State::Release {
                    filter,
                    mut next,
                    cleanup,
                } => {
                    while next < self.devices.len() * CONTROLS {
                        let index = next / CONTROLS;
                        let control = (next % CONTROLS) as u16;
                        next += 1;
                        let device = &self.devices[index];
                        if filter.matches(device, control)
                            && device.held[usize::from(control)]
                            && let Some(id) = device.id
                        {
                            cursor.state = State::Release {
                                filter,
                                next,
                                cleanup,
                            };
                            if let Some((atom, pending)) =
                                self.prepare_key(id, control, false, false, cursor.time, commands)
                            {
                                return cursor.atom(atom, pending);
                            }
                        }
                    }
                    self.finish_release(cleanup, cursor.time);
                    cursor.state = State::Done;
                }
                State::ClearSeat {
                    seat,
                    mut next_native,
                } => {
                    while let Ok(native) = u8::try_from(next_native) {
                        next_native += 1;
                        let Some(control) = keys::from_native(native) else {
                            continue;
                        };
                        // CL_KeyEvent consumes console-key ups before native
                        // binding/catcher handling. Existing engine holds still
                        // need retiring, but a caught console press has none.
                        let console_key = matches!(native, b'`' | b'~');
                        let reserved = 768 + u16::from(native);
                        let mut source = None;
                        for index in 0..self.devices.len() {
                            let device = &self.devices[index];
                            if device.seat != Some(seat) || device.held_count == 0 {
                                continue;
                            }
                            let Some(id) = device.id else { continue };
                            for physical in keys::physical_controls(control)
                                .chain((control != reserved).then_some(reserved))
                            {
                                if !self.devices[index].held[usize::from(physical)] {
                                    continue;
                                }
                                let acquired = matches!(
                                    self.devices[index].acquired[usize::from(physical)],
                                    BindingHold::Acquired(_)
                                );
                                if let Some((
                                    InputAtom::Key {
                                        source: released, ..
                                    },
                                    _,
                                )) = self.prepare_key(
                                    id,
                                    physical,
                                    false,
                                    false,
                                    EventTime(0),
                                    commands,
                                ) {
                                    // Native Key_ClearStates also releases the current
                                    // button binding when UI caught its original press.
                                    if !acquired && !console_key {
                                        self.emit_binding(
                                            released,
                                            seat,
                                            index,
                                            false,
                                            EventTime(0),
                                            commands,
                                        );
                                    }
                                    source.get_or_insert(released);
                                }
                            }
                        }
                        if let Some(source) = source.filter(|_| !console_key) {
                            cursor.state = State::ClearSeat { seat, next_native };
                            return cursor.atom(
                                InputAtom::Key {
                                    seat,
                                    source,
                                    time: EventTime(0),
                                    down: false,
                                    repeat: false,
                                },
                                Pending::Released,
                            );
                        }
                    }
                    cursor.state = State::Release {
                        filter: ReleaseFilter::UnmappedSeat(seat),
                        next: 0,
                        cleanup: Cleanup::Seat(seat),
                    };
                }
                State::Done => return DispatchStep::Done,
            }
        }
    }

    pub fn resume<'event>(
        &mut self,
        resume: DispatchResume<'event>,
        capture: InputCapture,
        commands: &mut impl Target,
    ) -> DispatchCursor<'event> {
        let DispatchResume { cursor, pending } = resume;
        match pending {
            Pending::Key {
                source,
                seat,
                index,
                transition: true,
                down: true,
            } => {
                let device = &mut self.devices[index];
                let hold = &mut device.acquired[usize::from(source.control)];
                if device.id == Some(source.device)
                    && device.seat == Some(seat)
                    && let BindingHold::Pending(producer) = *hold
                {
                    if capture == InputCapture::Pass && device.held[usize::from(source.control)] {
                        *hold = BindingHold::Acquired(producer);
                        self.emit_binding(source, seat, index, true, cursor.time, commands);
                    } else {
                        *hold = BindingHold::None;
                    }
                }
            }
            Pending::Mouse {
                device,
                seat,
                dx,
                dy,
            } if capture == InputCapture::Pass => {
                if self.device_seat(device) == Some(seat) {
                    let state = &mut self.seats[seat.index()];
                    state.mouse[0] = state.mouse[0].saturating_add(i64::from(dx));
                    state.mouse[1] = state.mouse[1].saturating_add(i64::from(dy));
                }
            }
            _ => {}
        }
        cursor
    }

    fn prepare_key(
        &mut self,
        id: DeviceId,
        control: u16,
        down: bool,
        repeat: bool,
        time: EventTime,
        commands: &mut impl Target,
    ) -> Option<(InputAtom, Pending)> {
        if usize::from(control) >= CONTROLS {
            return None;
        }
        let index = if down {
            self.ensure_device(id)
        } else {
            self.device_index(id)
        }?;
        let device = &mut self.devices[index];
        let seat = device.seat?;
        let was_held = device.set_held(control, down)?;
        let transition = was_held != down;
        let source = KeySource {
            device: id,
            control,
        };
        if transition {
            if down {
                let producer = self.bindings[usize::from(keys::normalize(control))]
                    .as_ref()
                    .and_then(Binding::producer);
                device.acquired[usize::from(control)] = BindingHold::Pending(producer);
            } else if matches!(
                std::mem::take(&mut device.acquired[usize::from(control)]),
                BindingHold::Acquired(_)
            ) {
                self.emit_binding(source, seat, index, false, time, commands);
            }
        }
        Some((
            InputAtom::Key {
                seat,
                source,
                time,
                down,
                repeat,
            },
            Pending::Key {
                source,
                seat,
                index,
                transition,
                down,
            },
        ))
    }

    fn emit_binding(
        &mut self,
        source: KeySource,
        seat: SeatId,
        index: usize,
        down: bool,
        time: EventTime,
        target: &mut impl Target,
    ) {
        if let Some(binding) = &self.bindings[usize::from(keys::normalize(source.control))] {
            let mut button_seen = false;
            for part in &binding.parts[..binding.count] {
                button_seen |= part.button;
                let text = &binding.bytes()[part.start as usize..part.end as usize];
                if !part.button && !down && !button_seen {
                    continue;
                }
                let mut command = FixedText::<1088>::default();
                let text = if part.button {
                    if down {
                        let _ = command.append_bytes(text);
                    } else {
                        let _ = command.append_bytes(b"-");
                        let _ = command.append_bytes(&text[1..]);
                    }
                    let _ = write!(
                        command,
                        " {} {}",
                        index * CONTROLS + usize::from(source.control),
                        time.milliseconds()
                    );
                    command.as_bytes()
                } else {
                    text
                };
                if !target.command_bytes(seat, time, Some(source), binding.producer(), text) {
                    let producer = binding.producer();
                    target.cancel_commands(source, producer);
                    self.cancel_source(source, producer, time);
                    return;
                }
            }
        }
    }

    pub(super) fn key(
        &mut self,
        device: DeviceId,
        code: u16,
        down: bool,
        repeat: bool,
        time: EventTime,
        target: &mut impl Target,
    ) {
        self.dispatch(
            SysEvent {
                time,
                kind: EventKind::Key {
                    device,
                    code,
                    symbol: 0,
                    down,
                    repeat,
                },
            },
            target,
        );
    }
    pub fn release_device(&mut self, id: DeviceId, time: EventTime, target: &mut impl Target) {
        self.dispatch_cursor(
            DispatchCursor::release(time, ReleaseFilter::Device(id), Cleanup::None),
            target,
        );
    }
    pub fn clear_seat(&mut self, seat: SeatId, time: EventTime, target: &mut impl Target) {
        self.dispatch_cursor(DispatchCursor::clear_seat(seat, time), target);
    }
    fn finish_release(&mut self, cleanup: Cleanup, time: EventTime) {
        match cleanup {
            Cleanup::None => {}
            Cleanup::Mouse => self.clear_mouse(),
            Cleanup::Device(id) => self.remove(id, time),
            Cleanup::Focus => {
                for seat in &mut self.seats {
                    seat.actions.fill(Button::default());
                }
                self.clear_mouse();
                for device in &mut self.devices {
                    device.axes.fill(0);
                }
            }
            Cleanup::Seat(seat) => {
                let state = &mut self.seats[seat.index()];
                state.actions.fill(Button::default());
                state.clear_mouse();
                for device in &mut self.devices {
                    if device.seat == Some(seat) {
                        device.axes.fill(0);
                    }
                }
            }
        }
    }
}
