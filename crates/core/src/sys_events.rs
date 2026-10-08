//! Ordered system input. Output sound/effect/print events remain in `events`.
use std::net::SocketAddr;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct EventTime(pub u64);
impl EventTime {
    pub fn since(self, earlier: Self) -> u64 {
        self.0.saturating_sub(earlier.0)
    }
    pub fn milliseconds(self) -> u64 {
        self.0 / 1_000_000
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceId {
    Keyboard,
    Mouse(u32),
    Controller(i32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeatId(u8);
impl SeatId {
    pub const ALL: [Self; 4] = [Self(0), Self(1), Self(2), Self(3)];
    pub const COUNT: usize = Self::ALL.len();
    pub fn new(index: u8) -> Option<Self> {
        (usize::from(index) < Self::COUNT).then_some(Self(index))
    }
    pub fn index(self) -> usize {
        usize::from(self.0)
    }
    pub const FIRST: Self = Self(0);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind<'a> {
    Time,
    Key {
        device: DeviceId,
        code: u16,
        symbol: i32,
        down: bool,
        repeat: bool,
    },
    Char {
        device: DeviceId,
        value: char,
    },
    Mouse {
        device: DeviceId,
        dx: i32,
        dy: i32,
    },
    MouseButton {
        device: DeviceId,
        button: u8,
        down: bool,
    },
    MouseWheel {
        device: DeviceId,
        x: i32,
        y: i32,
    },
    ControllerAxis {
        device: DeviceId,
        axis: u8,
        value: i16,
    },
    ControllerButton {
        device: DeviceId,
        button: u8,
        down: bool,
    },
    DeviceRemoved(DeviceId),
    Focus(bool),
    Quit,
    ConsoleLine(&'a str),
    Packet {
        socket: u16,
        from: SocketAddr,
        bytes: &'a [u8],
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SysEvent<'a> {
    pub time: EventTime,
    pub kind: EventKind<'a>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueError {
    Capacity,
    Full,
    PayloadFull,
}
#[derive(Clone, Copy)]
enum StoredKind {
    Plain(EventKind<'static>),
    Console,
    Packet { socket: u16, from: SocketAddr },
}
#[derive(Clone, Copy)]
struct Slot {
    time: EventTime,
    kind: StoredKind,
    start: usize,
    length: usize,
    reserved: usize,
}

/// Payloads borrow the ring until the receiver finishes dispatching one event.
/// FIFO release reuses bytes without allocating or retaining packet pointers.
pub struct SysEventQueue {
    slots: Box<[Option<Slot>]>,
    bytes: Box<[u8]>,
    head: usize,
    length: usize,
    write: usize,
    used: usize,
    rejected: u64,
}
impl SysEventQueue {
    pub fn load(events: usize, payload_bytes: usize) -> Result<Self, QueueError> {
        if !(2..=65536).contains(&events) || !(1..=64 * 1024 * 1024).contains(&payload_bytes) {
            return Err(QueueError::Capacity);
        }
        Ok(Self {
            slots: vec![None; events].into_boxed_slice(),
            bytes: vec![0; payload_bytes].into_boxed_slice(),
            head: 0,
            length: 0,
            write: 0,
            used: 0,
            rejected: 0,
        })
    }
    pub fn len(&self) -> usize {
        self.length
    }
    pub fn is_empty(&self) -> bool {
        self.length == 0
    }
    pub fn capacity(&self) -> usize {
        self.slots.len()
    }
    pub fn rejected(&self) -> u64 {
        self.rejected
    }

    pub fn push(&mut self, event: SysEvent<'_>) -> Result<(), QueueError> {
        let result = self.admit(event);
        if result.is_err() {
            self.rejected = self.rejected.saturating_add(1);
        }
        result
    }
    fn admit(&mut self, event: SysEvent<'_>) -> Result<(), QueueError> {
        // Ordinary events cannot consume the frame's final time-marker slot.
        let reserved_time = usize::from(!matches!(event.kind, EventKind::Time));
        if self.length >= self.slots.len() - reserved_time {
            return Err(QueueError::Full);
        }
        let (kind, payload) = match event.kind {
            EventKind::ConsoleLine(text) => (StoredKind::Console, text.as_bytes()),
            EventKind::Packet {
                socket,
                from,
                bytes,
            } => (StoredKind::Packet { socket, from }, bytes),
            EventKind::Time => (StoredKind::Plain(EventKind::Time), &[][..]),
            EventKind::Key {
                device,
                code,
                symbol,
                down,
                repeat,
            } => (
                StoredKind::Plain(EventKind::Key {
                    device,
                    code,
                    symbol,
                    down,
                    repeat,
                }),
                &[][..],
            ),
            EventKind::Char { device, value } => (
                StoredKind::Plain(EventKind::Char { device, value }),
                &[][..],
            ),
            EventKind::Mouse { device, dx, dy } => (
                StoredKind::Plain(EventKind::Mouse { device, dx, dy }),
                &[][..],
            ),
            EventKind::MouseButton {
                device,
                button,
                down,
            } => (
                StoredKind::Plain(EventKind::MouseButton {
                    device,
                    button,
                    down,
                }),
                &[][..],
            ),
            EventKind::MouseWheel { device, x, y } => (
                StoredKind::Plain(EventKind::MouseWheel { device, x, y }),
                &[][..],
            ),
            EventKind::ControllerAxis {
                device,
                axis,
                value,
            } => (
                StoredKind::Plain(EventKind::ControllerAxis {
                    device,
                    axis,
                    value,
                }),
                &[][..],
            ),
            EventKind::ControllerButton {
                device,
                button,
                down,
            } => (
                StoredKind::Plain(EventKind::ControllerButton {
                    device,
                    button,
                    down,
                }),
                &[][..],
            ),
            EventKind::DeviceRemoved(device) => {
                (StoredKind::Plain(EventKind::DeviceRemoved(device)), &[][..])
            }
            EventKind::Focus(value) => (StoredKind::Plain(EventKind::Focus(value)), &[][..]),
            EventKind::Quit => (StoredKind::Plain(EventKind::Quit), &[][..]),
        };
        let length = payload.len();
        let padding = if self.write + length > self.bytes.len() {
            self.bytes.len() - self.write
        } else {
            0
        };
        let reserved = padding + length;
        if reserved > self.bytes.len() - self.used {
            return Err(QueueError::PayloadFull);
        }
        let start = if padding == 0 { self.write } else { 0 };
        self.bytes[start..start + length].copy_from_slice(payload);
        self.write = (start + length) % self.bytes.len();
        self.used += reserved;
        self.slots[(self.head + self.length) % self.slots.len()] = Some(Slot {
            time: event.time,
            kind,
            start,
            length,
            reserved,
        });
        self.length += 1;
        Ok(())
    }
    pub fn pop(&mut self) -> Option<SysEvent<'_>> {
        if self.length == 0 {
            return None;
        }
        let slot = self.slots[self.head].take()?;
        self.head = (self.head + 1) % self.slots.len();
        self.length -= 1;
        self.used -= slot.reserved;
        if self.used == 0 {
            self.write = 0;
        }
        let payload = &self.bytes[slot.start..slot.start + slot.length];
        let kind = match slot.kind {
            StoredKind::Plain(kind) => kind,
            StoredKind::Console => EventKind::ConsoleLine(std::str::from_utf8(payload).ok()?),
            StoredKind::Packet { socket, from } => EventKind::Packet {
                socket,
                from,
                bytes: payload,
            },
        };
        Some(SysEvent {
            time: slot.time,
            kind,
        })
    }
}
