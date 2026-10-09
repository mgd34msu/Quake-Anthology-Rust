//! Native ordinal/width bindings over the common bank, resolved at load.
use qa_core::primitives::{
    NameId, RuleSetId, ValueBank, ValueBinding, ValueId, ValueReset, ValueWidth,
};
pub struct NativeValues {
    /// These are immutable numeric bindings, not additional value storage.
    pub stats: Box<[ValueBinding]>,
    pub persistent: Box<[ValueBinding]>,
}
// Native boundary counts/widths from client.h and the three q_shared/game.h files.
const NATIVE: [(u32, u32, ValueWidth); 5] = [
    (32, 0, ValueWidth::Signed32),
    (32, 0, ValueWidth::Signed32),
    (32, 0, ValueWidth::Signed16),
    (64, 0, ValueWidth::Signed16),
    (16, 16, ValueWidth::Signed32),
];
impl NativeValues {
    /// A module life transition clears only its explicitly bound fields.
    pub fn reset_life(&self, bank: &mut ValueBank) {
        for binding in self.stats.iter().chain(&self.persistent) {
            binding.reset_life(bank);
        }
    }
    /// Each native module may bind its own explicitly reserved bank range.
    /// Source/protocol/presentation choices never change the bank's identity.
    pub fn load(rules: RuleSetId, first: ValueId) -> Option<Self> {
        let (stats, persistent, width) = NATIVE[rules as usize];
        first.0.checked_add(stats)?.checked_add(persistent)?;
        let fields = |offset: u32, count: u32, reset| {
            (0..count)
                .map(|ordinal| ValueBinding {
                    id: ValueId(first.0 + offset + ordinal),
                    width,
                    reset,
                })
                .collect::<Box<[_]>>()
        };
        Some(Self {
            stats: fields(0, stats, ValueReset::Life),
            persistent: fields(stats, persistent, ValueReset::Session),
        })
    }
}
#[derive(Clone, Copy, Debug)]
pub struct ExtensionValue {
    pub name: NameId,
    pub width: ValueWidth,
    pub reset: ValueReset,
}
pub struct ValueLayout {
    native: Box<[NativeValues]>,
    pub extensions: Box<[(NameId, ValueBinding)]>,
    capacity: usize,
}
impl ValueLayout {
    pub fn load(extensions: &[ExtensionValue]) -> Option<Self> {
        let native_count: u32 = NATIVE
            .iter()
            .map(|&(stats, persistent, _)| stats + persistent)
            .sum();
        let capacity = native_count.checked_add(u32::try_from(extensions.len()).ok()?)?;
        let mut first = 0;
        let mut native = Vec::with_capacity(RuleSetId::ALL.len());
        for rules in RuleSetId::ALL {
            let values = NativeValues::load(rules, ValueId(first))?;
            first += (values.stats.len() + values.persistent.len()) as u32;
            native.push(values);
        }
        Some(Self {
            native: native.into_boxed_slice(),
            extensions: extensions
                .iter()
                .enumerate()
                .map(|(index, spec)| {
                    (
                        spec.name,
                        ValueBinding {
                            id: ValueId(native_count + index as u32),
                            width: spec.width,
                            reset: spec.reset,
                        },
                    )
                })
                .collect(),
            capacity: capacity as usize,
        })
    }
    pub fn capacity(&self) -> usize {
        self.capacity
    }
    pub fn native(&self, rules: RuleSetId) -> &NativeValues {
        &self.native[rules as usize]
    }
    /// Clear a common life. A module-only transition uses NativeValues instead.
    /// Disconnect and
    /// session replacement clear the whole bank through PlayerState::reset.
    pub fn reset_life(&self, bank: &mut ValueBank) {
        for binding in self
            .native
            .iter()
            .flat_map(|table| table.stats.iter().chain(&table.persistent))
            .chain(self.extensions.iter().map(|(_, binding)| binding))
        {
            binding.reset_life(bank);
        }
    }
}
