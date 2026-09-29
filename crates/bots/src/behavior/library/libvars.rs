//! Bot library variables from `src/bots/behavior/library/libvars.ts`
//! (`be_interface.c`: `LibVarGetString`/`LibVarGetValue`/`LibVarSet`).
//!
//! Libvars are named string/number pairs with a modified flag. Numeric
//! values parse with the donor's decimal scanner (leading whitespace,
//! optional sign, fraction, no exponents).

/// Maximum distinct libvars, matching the donor scan bound.
pub const BOT_LIBVAR_SCAN_BOUND: usize = 99999;

/// One library variable.
#[derive(Debug, Clone, PartialEq)]
pub struct BotLibVar {
    /// Variable name.
    pub name: String,
    /// String value.
    pub string: String,
    /// Flags (donor reserves the field; always zero here).
    pub flags: i32,
    /// Whether the value changed since the last acknowledge.
    pub modified: bool,
    /// Parsed numeric value.
    pub value: f32,
}

/// Parse a libvar numeric value with the donor's decimal scanner.
#[must_use]
pub fn lib_var_string_value(text: &str) -> f32 {
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() && bytes[index].is_ascii_whitespace() {
        index += 1;
    }
    let mut sign = 1.0f64;
    if index < bytes.len() && (bytes[index] == b'-' || bytes[index] == b'+') {
        if bytes[index] == b'-' {
            sign = -1.0;
        }
        index += 1;
    }
    let mut value = 0.0f64;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        value = value.mul_add(10.0, f64::from(bytes[index] - b'0'));
        index += 1;
    }
    if index < bytes.len() && bytes[index] == b'.' {
        index += 1;
        let mut denominator = 10.0f64;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            value += f64::from(bytes[index] - b'0') / denominator;
            denominator *= 10.0;
            index += 1;
        }
    }
    (sign * value) as f32
}

/// Bot library variable table.
#[derive(Debug, Clone, Default)]
pub struct BotLibVars {
    vars: Vec<BotLibVar>,
}

impl BotLibVars {
    /// Empty table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Look up a variable by name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&BotLibVar> {
        self.vars.iter().find(|var| var.name == name)
    }

    /// String value, or empty when undefined.
    #[must_use]
    pub fn get_string(&self, name: &str) -> &str {
        self.get(name).map_or("", |var| var.string.as_str())
    }

    /// Numeric value, or 0 when undefined.
    #[must_use]
    pub fn get_value(&self, name: &str) -> f32 {
        self.get(name).map_or(0.0, |var| var.value)
    }

    /// Look up or create a variable with a default value.
    pub fn get_or_create(&mut self, name: &str, default_value: &str) -> &BotLibVar {
        if !self.vars.iter().any(|var| var.name == name) {
            debug_assert!(self.vars.len() < BOT_LIBVAR_SCAN_BOUND);
            self.vars.push(BotLibVar {
                name: name.to_owned(),
                string: default_value.to_owned(),
                flags: 0,
                modified: true,
                value: lib_var_string_value(default_value),
            });
        }
        self.get(name).expect("libvar just created")
    }

    /// String value with creation default.
    pub fn string(&mut self, name: &str, default_value: &str) -> String {
        self.get_or_create(name, default_value).string.clone()
    }

    /// Numeric value with creation default.
    pub fn value(&mut self, name: &str, default_value: &str) -> f32 {
        self.get_or_create(name, default_value).value
    }

    /// Set a variable, creating it when missing.
    pub fn set(&mut self, name: &str, value: &str) {
        let parsed = lib_var_string_value(value);
        match self.vars.iter_mut().find(|var| var.name == name) {
            Some(var) => {
                var.string = value.to_owned();
                var.value = parsed;
                var.modified = true;
            }
            None => self.vars.push(BotLibVar {
                name: name.to_owned(),
                string: value.to_owned(),
                flags: 0,
                modified: true,
                value: parsed,
            }),
        }
    }

    /// Whether the variable changed since the last acknowledge.
    #[must_use]
    pub fn changed(&self, name: &str) -> bool {
        self.get(name).is_some_and(|var| var.modified)
    }

    /// Acknowledge the modified flag.
    pub fn set_not_modified(&mut self, name: &str) {
        if let Some(var) = self.vars.iter_mut().find(|var| var.name == name) {
            var.modified = false;
        }
    }

    /// Clear the table.
    pub fn clear(&mut self) {
        self.vars.clear();
    }

    /// Checkpoint all variables.
    #[must_use]
    pub fn checkpoint(&self) -> Vec<BotLibVar> {
        self.vars.clone()
    }

    /// Restore checkpointed variables.
    pub fn restore(&mut self, vars: &[BotLibVar]) {
        self.vars = vars.to_vec();
    }
}
