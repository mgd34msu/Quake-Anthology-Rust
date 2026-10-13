//! Native ordinal/width bindings over the common bank, resolved at load.
use qa_core::primitives::{
    NATIVE_VALUE_LAYOUT, NameId, PlayerValueBindings, RuleSetId, ValueBank, ValueBinding, ValueId,
    ValueReset, ValueWidth,
};
pub struct NativeValues {
    /// These are immutable numeric bindings, not additional value storage.
    pub stats: Box<[ValueBinding]>,
    pub persistent: Box<[ValueBinding]>,
    pub ammo: Box<[ValueBinding]>,
    pub powerups: Box<[ValueBinding]>,
    /// NQ clientdata owns its mask independently of STAT_ITEMS.
    pub items: Option<ValueBinding>,
}
impl NativeValues {
    pub fn bindings(&self) -> PlayerValueBindings<'_> {
        PlayerValueBindings {
            arrays: [&self.stats, &self.persistent, &self.ammo, &self.powerups],
            items: self.items,
        }
    }
    /// A module life transition clears only its explicitly bound fields.
    pub fn reset_life(&self, bank: &mut ValueBank) {
        for binding in self.bindings().iter() {
            binding.reset_life(bank);
        }
    }
    /// Each native module may bind its own explicitly reserved bank range.
    /// Source/protocol/presentation choices never change the bank's identity.
    pub fn load(rules: RuleSetId, first: ValueId) -> Option<Self> {
        let layout = NATIVE_VALUE_LAYOUT[rules as usize];
        first.0.checked_add(layout.values())?;
        let counts = layout.arrays();
        let mut offset = first.0;
        let [stats, persistent, ammo, powerups] = std::array::from_fn(|array| {
            let reset = if array == 1 {
                ValueReset::Session
            } else {
                ValueReset::Life
            };
            let fields = (0..counts[array])
                .map(|ordinal| ValueBinding {
                    id: ValueId(offset + ordinal),
                    width: layout.width,
                    reset,
                })
                .collect::<Box<[_]>>();
            offset += counts[array];
            fields
        });
        Some(Self {
            stats,
            persistent,
            ammo,
            powerups,
            items: layout.items.then_some(ValueBinding {
                id: ValueId(offset),
                width: ValueWidth::Signed32,
                reset: ValueReset::Life,
            }),
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
        let native_count = NATIVE_VALUE_LAYOUT
            .iter()
            .try_fold(0u32, |count, layout| count.checked_add(layout.values()))?;
        let capacity = native_count.checked_add(u32::try_from(extensions.len()).ok()?)?;
        let mut first = 0u32;
        let mut native = Vec::with_capacity(RuleSetId::ALL.len());
        for rules in RuleSetId::ALL {
            let values = NativeValues::load(rules, ValueId(first))?;
            first = first.checked_add(NATIVE_VALUE_LAYOUT[rules as usize].values())?;
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
        for table in &self.native {
            table.reset_life(bank);
        }
        for (_, binding) in &self.extensions {
            binding.reset_life(bank);
        }
    }
}
