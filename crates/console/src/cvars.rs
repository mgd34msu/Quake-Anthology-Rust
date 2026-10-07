use qa_core::primitives::CvarHandle;

pub struct Definition {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub default: f32,
}

pub struct Cvars {
    definitions: &'static [Definition],
    values: Vec<f32>,
}

impl Cvars {
    pub fn new(definitions: &'static [Definition]) -> Self {
        Self {
            definitions,
            values: definitions.iter().map(|entry| entry.default).collect(),
        }
    }

    pub fn find(&self, name: &str) -> Option<CvarHandle> {
        self.definitions
            .iter()
            .position(|entry| {
                entry.name.eq_ignore_ascii_case(name)
                    || entry
                        .aliases
                        .iter()
                        .any(|alias| alias.eq_ignore_ascii_case(name))
            })
            .map(|index| CvarHandle(index as u32))
    }

    pub fn value(&self, handle: CvarHandle) -> f32 {
        self.values[handle.0 as usize]
    }

    pub fn set(&mut self, handle: CvarHandle, value: f32) {
        self.values[handle.0 as usize] = value;
    }
}
