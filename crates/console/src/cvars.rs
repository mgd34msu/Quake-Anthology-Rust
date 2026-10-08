use crate::{
    catalog::Condition,
    cvars_generated::{BINDINGS, DEFAULTS, DEFINITIONS},
};
use qa_core::primitives::CvarHandle;
use std::borrow::Cow;

pub use crate::catalog::Definition;

struct Value {
    name: &'static str,
    row: usize,
    seat: u8,
    text: Cow<'static, str>,
    number: f32,
}

pub struct Cvars {
    values: Vec<Value>,
    offsets: Vec<usize>,
}

impl Cvars {
    pub fn new() -> Self {
        let mut values =
            Vec::with_capacity(DEFINITIONS.iter().map(|d| d.family_count as usize).sum());
        let mut offsets = Vec::with_capacity(DEFINITIONS.len());
        for (row, definition) in DEFINITIONS.iter().enumerate() {
            offsets.push(values.len());
            for slot in 0..definition.family_count {
                let seat = if definition.family_count == 1 {
                    0
                } else {
                    slot + 1
                };
                let binding = BINDINGS
                    .iter()
                    .find(|b| b.canonical && b.row as usize == row && b.seat == seat);
                let name = binding.map_or(definition.name, |b| b.name);
                let text = default_text(definition);
                values.push(Value {
                    name,
                    row,
                    seat,
                    text: Cow::Borrowed(text),
                    number: number(text),
                });
            }
        }
        Self { values, offsets }
    }

    /// Resolve a name to its canonical numeric slot for engine consumers.
    /// Converted alias views at command/module boundaries are a separate operation.
    pub fn find(&self, name: &str) -> Option<CvarHandle> {
        BINDINGS
            .iter()
            .find(|b| b.name.eq_ignore_ascii_case(name) && b.scope != crate::catalog::Scope::Server)
            .map(|b| {
                CvarHandle(
                    (self.offsets[b.row as usize] + b.seat.saturating_sub(1) as usize) as u32,
                )
            })
    }

    pub fn value(&self, handle: CvarHandle) -> f32 {
        self.values[handle.0 as usize].number
    }

    pub fn text(&self, handle: CvarHandle) -> &str {
        &self.values[handle.0 as usize].text
    }

    pub fn set(&mut self, handle: CvarHandle, value: f32) {
        self.set_text(handle, &value.to_string());
    }

    pub fn set_text(&mut self, handle: CvarHandle, text: &str) {
        let value = &mut self.values[handle.0 as usize];
        value.number = number(text);
        value.text = Cow::Owned(text.to_owned());
    }

    pub fn entries(
        &self,
    ) -> impl Iterator<Item = (CvarHandle, &'static str, &str, &'static Definition, u8)> {
        self.values.iter().enumerate().map(|(index, v)| {
            (
                CvarHandle(index as u32),
                v.name,
                v.text.as_ref(),
                &DEFINITIONS[v.row],
                v.seat,
            )
        })
    }
}

impl Default for Cvars {
    fn default() -> Self {
        Self::new()
    }
}

fn number(text: &str) -> f32 {
    text.parse().unwrap_or(0.0)
}

fn default_text(definition: &Definition) -> &'static str {
    // Initial host is the Q3-default window shell. THE-623 adds source views,
    // alias conversions and unresolved default policy at session boundaries.
    DEFAULTS[definition.defaults[4].defaults.clone()]
        .iter()
        .find(|d| match d.condition {
            Condition::Always | Condition::Client | Condition::Engine => true,
            Condition::Linux => cfg!(target_os = "linux"),
            Condition::NotLinux => !cfg!(target_os = "linux"),
            Condition::Mac => cfg!(target_os = "macos"),
            Condition::NotMac => !cfg!(target_os = "macos"),
            Condition::Windows => cfg!(target_os = "windows"),
            Condition::NotWindows => !cfg!(target_os = "windows"),
            _ => false,
        })
        .map_or("", |d| d.value)
}
