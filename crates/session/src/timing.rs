//! Independent world/provider clocks on one event timeline (Q3 SV_Frame).
use qa_core::{primitives::ModuleId, sys_events::EventTime};
use std::num::NonZeroU32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TickRate {
    FrameDriven,
    FixedMilliseconds(NonZeroU32),
}
impl TickRate {
    pub fn fixed(milliseconds: u32) -> Option<Self> {
        NonZeroU32::new(milliseconds).map(Self::FixedMilliseconds)
    }
    fn period(self) -> Option<u64> {
        match self {
            Self::FrameDriven => None,
            Self::FixedMilliseconds(ms) => Some(u64::from(ms.get()) * 1_000_000),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TickTarget {
    World,
    Provider(ModuleId),
}
impl TickTarget {
    fn order(self) -> u32 {
        match self {
            Self::World => 0,
            Self::Provider(id) => u32::from(id.0) + 1,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tick {
    pub target: TickTarget,
    /// Load-resolved source slot: world 0, providers in module-id order.
    pub source_slot: usize,
    pub start: EventTime,
    pub end: EventTime,
    pub index: u64,
}

struct Clock {
    target: TickTarget,
    rate: TickRate,
    last: EventTime,
    next: EventTime,
    index: u64,
}
pub struct Timeline {
    clocks: Box<[Clock]>,
    seeded: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum TimingError {
    DuplicateProvider,
}

impl Timeline {
    pub fn load(
        world: TickRate,
        providers: impl IntoIterator<Item = (ModuleId, TickRate)>,
    ) -> Result<Self, TimingError> {
        let mut clocks = vec![Clock::new(TickTarget::World, world)];
        for (id, rate) in providers {
            clocks.push(Clock::new(TickTarget::Provider(id), rate));
        }
        clocks.sort_unstable_by_key(|clock| clock.target.order());
        if clocks
            .windows(2)
            .any(|pair| pair[0].target == pair[1].target)
        {
            return Err(TimingError::DuplicateProvider);
        }
        Ok(Self {
            clocks: clocks.into_boxed_slice(),
            seeded: false,
        })
    }

    /// Called at world load/first frame, so startup time never becomes debt.
    pub fn seed(&mut self, time: EventTime) {
        for clock in &mut self.clocks {
            clock.last = time;
            clock.next = EventTime(time.0.saturating_add(clock.rate.period().unwrap_or(0)));
            clock.index = 0;
        }
        self.seeded = true;
    }

    /// Catch up in timestamp order, then target order. The callback observes
    /// native tick durations; client cadence never changes provider rates.
    pub fn advance(&mut self, time: EventTime, mut tick: impl FnMut(Tick)) -> u64 {
        if !self.seeded {
            self.seed(time);
            return 0;
        }
        let mut count = 0;
        while let Some(slot) = self
            .clocks
            .iter()
            .enumerate()
            .filter(|(_, clock)| match clock.rate {
                TickRate::FrameDriven => clock.last < time,
                TickRate::FixedMilliseconds(_) => clock.next <= time && clock.last < clock.next,
            })
            .min_by_key(|(_, clock)| {
                (
                    clock.rate.period().map_or(time, |_| clock.next),
                    clock.target.order(),
                )
            })
            .map(|(slot, _)| slot)
        {
            let clock = &mut self.clocks[slot];
            let end = clock.rate.period().map_or(time, |_| clock.next);
            clock.index += 1;
            tick(Tick {
                target: clock.target,
                source_slot: slot,
                start: clock.last,
                end,
                index: clock.index,
            });
            clock.last = end;
            clock.next = EventTime(end.0.saturating_add(clock.rate.period().unwrap_or(0)));
            count += 1;
        }
        count
    }
}

impl Clock {
    fn new(target: TickTarget, rate: TickRate) -> Self {
        Self {
            target,
            rate,
            last: EventTime::default(),
            next: EventTime::default(),
            index: 0,
        }
    }
}
