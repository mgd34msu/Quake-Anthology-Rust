//! Instance-owned cvar registry ported from `src/core/cvars/` (Q1/QW
//! `cvar.c`, Q2 `qcommon/cvar.c`, Q3 `cvar.c`) with policies from
//! `src/contracts/common.ts`.
//!
//! One registry per owner, parameterized by [`Dialect`]: exact vs
//! ASCII-folded name lookup, newest-first ordering, latch/archive/info
//! rules. Donor `print` calls become queued notifications plus a
//! structured [`CvarEffect`] queue; VM mirrors, aliases, value bindings,
//! and save/restore are follow-ups owned by later phases.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use thiserror::Error;

use crate::cmd::{ascii_fold, source_command_text, Dialect};
use crate::numeric::{native_atof, native_atoi};

/// Error for cvar operations.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CvarError {
    /// Command text must be source bytes.
    #[error(transparent)]
    Command(#[from] crate::cmd::CmdError),
    /// A value domain was violated; carries the donor message.
    #[error("{0}")]
    Domain(String),
}

/// Q3 ABI flag word. Q1 uses `ARCHIVE` and `SERVER_INFO` for declaration
/// booleans; the Archive/UserInfo/ServerInfo values match Q2.
pub mod flags {
    /// No flags.
    pub const NONE: u32 = 0;
    /// Archive to configuration.
    pub const ARCHIVE: u32 = 1;
    /// Send in client userinfo.
    pub const USER_INFO: u32 = 2;
    /// Send in server info.
    pub const SERVER_INFO: u32 = 4;
    /// Send in system info (Q3).
    pub const SYSTEM_INFO: u32 = 8;
    /// Write protected (Q3).
    pub const INIT: u32 = 16;
    /// Latched until restart (Q3).
    pub const LATCH: u32 = 32;
    /// Read only.
    pub const READ_ONLY: u32 = 64;
    /// Created by the user, not code.
    pub const USER_CREATED: u32 = 128;
    /// Temporary (Q3).
    pub const TEMPORARY: u32 = 256;
    /// Cheat protected.
    pub const CHEAT: u32 = 512;
    /// Survives `cvar_restart` (Q3).
    pub const NO_RESTART: u32 = 1024;
}

/// Quake II flag word.
pub mod q2_flags {
    /// No flags.
    pub const NONE: u32 = 0;
    /// Archive to configuration.
    pub const ARCHIVE: u32 = 1;
    /// Send in client userinfo.
    pub const USER_INFO: u32 = 2;
    /// Send in server info.
    pub const SERVER_INFO: u32 = 4;
    /// Write protected.
    pub const NO_SET: u32 = 8;
    /// Latched until the next game.
    pub const LATCH: u32 = 16;
    /// Cheat protected.
    pub const CHEAT: u32 = 32;
    /// Never sent to clients.
    pub const PRIVATE: u32 = 64;
    /// Read only.
    pub const READ_ONLY: u32 = 128;
    /// Modified since last check.
    pub const MODIFIED: u32 = 256;
    /// Created by the user, not code.
    pub const CUSTOM: u32 = 512;
    /// Ignored when set from the command line.
    pub const WEAK: u32 = 1024;
    /// Created by the game module.
    pub const GAME: u32 = 2048;
    /// Never archive.
    pub const NO_ARCHIVE: u32 = 4096;
    /// Added to `sv_paks` downloads.
    pub const FILES: u32 = 8192;
    /// Refresh needed.
    pub const REFRESH: u32 = 16384;
    /// Sound restart needed.
    pub const SOUND: u32 = 32768;
}

const Q2_NO_ARCHIVE: u32 =
    q2_flags::NO_SET | q2_flags::CHEAT | q2_flags::PRIVATE | q2_flags::READ_ONLY | q2_flags::NO_ARCHIVE;
const MAX_CVARS: usize = 1024;

/// Quake `Q_atof`: sign, `0x` hex, `'c'` character constant, or decimal
/// with an optional point. Stops at the first unrecognized byte.
#[must_use]
pub fn quake_atof(text: &str) -> f64 {
    let chars: Vec<char> = text.chars().collect();
    let at = |index: usize| chars.get(index).copied().unwrap_or('\0');
    let mut offset = 0;
    let mut sign = 1.0;
    if at(offset) == '-' {
        sign = -1.0;
        offset += 1;
    }
    if at(offset) == '0' && matches!(at(offset + 1), 'x' | 'X') {
        offset += 2;
        let mut value = 0.0;
        while offset < chars.len() {
            let digit = chars[offset];
            let digit = if digit.is_ascii_digit() {
                digit as i32 - 48
            } else if ('a'..='f').contains(&digit) {
                digit as i32 - 87
            } else if ('A'..='F').contains(&digit) {
                digit as i32 - 55
            } else {
                -1
            };
            offset += 1;
            if digit < 0 {
                break;
            }
            value = value * 16.0 + f64::from(digit);
        }
        return value * sign;
    }
    if at(offset) == '\'' {
        let code = if offset + 1 < chars.len() {
            chars[offset + 1] as u32
        } else {
            0
        };
        return sign * f64::from(code);
    }
    let mut value = 0.0;
    let mut decimal: i64 = -1;
    let mut total: i64 = 0;
    while offset < chars.len() {
        let byte = chars[offset] as u32;
        offset += 1;
        if byte == 46 {
            decimal = total;
            continue;
        }
        if !(48..=57).contains(&byte) {
            break;
        }
        value = value * 10.0 + f64::from(byte - 48);
        total += 1;
    }
    if decimal != -1 {
        while total > decimal {
            value /= 10.0;
            total -= 1;
        }
    }
    value * sign
}

/// C-locale `%f` rendering of a binary32 value: six decimals, ties to even.
/// With the integer shortcut, integral in-range values render plainly.
pub fn cvar_value_text(input: f64, integer_shortcut: bool) -> Result<String, CvarError> {
    let value = input as f32;
    if !value.is_finite() {
        return Err(CvarError::Domain("Cvar_SetValue requires a finite float".to_string()));
    }
    if integer_shortcut && (-2_147_483_648.0..2_147_483_648.0).contains(&value) && value == value.trunc() {
        return Ok(format!("{}", value as i32));
    }
    Ok(format!("{value:.6}"))
}

/// Info-string target for filtering rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InfoTarget {
    /// Client userinfo string.
    ClientUserinfo,
    /// Server info string.
    ServerInfo,
}

/// Options for [`set_info_value`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InfoOptions {
    /// Console dialect.
    pub dialect: Dialect,
    /// Maximum info length including the terminator.
    pub maximum_length: usize,
    /// Which string is updated.
    pub target: InfoTarget,
    /// Whether the server keeps high bytes in server info (QW).
    pub server_high_characters: bool,
}

/// Set or remove a `\\key\\value` pair, following QW/Q2/Q3 `q_shared.c`.
/// Prints and keeps the input on rejected keys, values, or overflow.
pub fn set_info_value(
    input: &str,
    key_input: &str,
    value_input: &str,
    options: InfoOptions,
    print: &mut dyn FnMut(&str),
) -> Result<String, CvarError> {
    let info = source_command_text(input)?;
    let key = source_command_text(key_input)?;
    let value = source_command_text(value_input)?;
    let q3 = options.dialect == Dialect::Q3;
    let qw = options.dialect == Dialect::Q1Quakeworld;
    if info.chars().count() >= options.maximum_length {
        return Err(CvarError::Domain(
            "Info_SetValueForKey: oversize infostring".to_string(),
        ));
    }
    if key.contains('\\') || value.contains('\\') {
        print("Can't use keys or values with a \\\n");
        return Ok(info);
    }
    if (!qw && key.contains(';')) || (q3 && value.contains(';')) {
        print("Can't use keys or values with a semicolon\n");
        return Ok(info);
    }
    if key.contains('"') || value.contains('"') {
        print("Can't use keys or values with a \"\n");
        return Ok(info);
    }
    if qw && key.starts_with('*') {
        print("Can't set * keys\n");
        return Ok(info);
    }
    if !q3 && (key.chars().count() > 63 || value.chars().count() > 63) {
        print("Keys and values must be < 64 characters.\n");
        return Ok(info);
    }
    let chars: Vec<char> = info.chars().collect();
    let mut result = info.clone();
    let mut cursor = 0;
    while cursor < chars.len() {
        let start = cursor;
        if chars[cursor] == '\\' {
            cursor += 1;
        }
        let Some(separator) = chars[cursor..]
            .iter()
            .position(|c| *c == '\\')
            .map(|index| index + cursor)
        else {
            break;
        };
        let next = chars[separator + 1..]
            .iter()
            .position(|c| *c == '\\')
            .map(|index| index + separator + 1);
        let end = next.unwrap_or(chars.len());
        if chars[cursor..separator].iter().collect::<String>() == key {
            if qw
                && end > separator + 1
                && value.chars().count() - (end - separator - 1) + chars.len() > options.maximum_length
            {
                print("Info string length exceeded\n");
                return Ok(info);
            }
            result = chars[..start].iter().collect::<String>() + &chars[end..].iter().collect::<String>();
            break;
        }
        cursor = end;
    }
    if value.is_empty() {
        return Ok(result);
    }
    let mut pair = format!("\\{key}\\{value}");
    if q3 && pair.chars().count() >= options.maximum_length {
        pair = pair.chars().take(options.maximum_length - 1).collect();
    }
    if pair.chars().count() + result.chars().count() > options.maximum_length {
        print("Info string length exceeded\n");
        return Ok(result);
    }
    if !q3 {
        let mut filtered = String::new();
        for byte in pair.chars().map(|c| c as u32) {
            let mut byte = byte;
            if qw {
                let strip = if options.target == InfoTarget::ServerInfo {
                    !options.server_high_characters
                } else {
                    ascii_fold(&key) != "name"
                };
                if strip {
                    byte &= 127;
                    if !(32..=127).contains(&byte) {
                        continue;
                    }
                    if options.target == InfoTarget::ClientUserinfo
                        && ascii_fold(&key) == "team"
                        && (65..=90).contains(&byte)
                    {
                        byte += 32;
                    }
                }
                if byte > 13 {
                    filtered.push(char::from_u32(byte).unwrap_or('?'));
                }
            } else {
                byte &= 127;
                if (32..127).contains(&byte) {
                    filtered.push(char::from_u32(byte).unwrap_or('?'));
                }
            }
        }
        pair = filtered;
    }
    if pair.chars().count() + result.chars().count() == options.maximum_length {
        return Err(CvarError::Domain("Info string overflows source terminator".to_string()));
    }
    if q3 && options.maximum_length != 8192 {
        Ok(pair + &result)
    } else {
        Ok(result + &pair)
    }
}

/// Immutable snapshot of one variable.
#[derive(Debug, Clone, PartialEq)]
pub struct CvarSnapshot {
    /// Variable name in declaration case.
    pub name: String,
    /// Current value.
    pub value: String,
    /// Reset (default) value.
    pub reset_value: String,
    /// Latched value awaiting restart, if any.
    pub latched_value: Option<String>,
    /// Flag word.
    pub flags: u32,
    /// Modified since creation or last clear.
    pub modified: bool,
    /// Modification counter.
    pub modification_count: u32,
    /// Numeric value (`Q_atof`/`atof`, binary32).
    pub numeric_value: f32,
    /// Integer value (`atoi`).
    pub integer_value: i32,
}

/// Queued side effect for the owner to drain (broadcasts, info updates,
/// game-directory switches). Carries string payloads; session-context
/// wiring is a follow-up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CvarEffect {
    /// Client userinfo changed.
    Userinfo {
        /// Variable name.
        name: String,
        /// New value.
        value: String,
        /// Updated info string.
        info: String,
        /// `setinfo` command for the server, when connected.
        command: Option<String>,
    },
    /// Server info changed.
    ServerInfo {
        /// Variable name.
        name: String,
        /// New value.
        value: String,
        /// Updated info string.
        info: String,
    },
    /// Broadcast line for connected clients.
    Broadcast {
        /// Line text.
        text: String,
    },
    /// The `game` directory changed; the owner executes `autoexec`.
    GameDirectory {
        /// New directory.
        directory: String,
    },
}

/// Archive entry for configuration files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CvarArchiveEntry {
    /// Variable name.
    pub name: String,
    /// Archived value.
    pub value: String,
}

type CommandExistsCallback = dyn Fn(&str) -> bool;

/// Which `set*` command family flags a variable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]

pub enum SetCommandKind {
    /// `seta`: archive.
    Archive,
    /// `setu`: userinfo.
    Userinfo,
    /// `sets`: serverinfo.
    Serverinfo,
}

#[derive(Debug, Clone)]
struct CvarState {
    name: String,
    value: String,
    reset_value: String,
    latched_value: Option<String>,
    flags: u32,
    modified: bool,
    modification_count: u32,
    numeric_value: f32,
    integer_value: i32,
}

impl CvarState {
    fn snapshot(&self) -> CvarSnapshot {
        CvarSnapshot {
            name: self.name.clone(),
            value: self.value.clone(),
            reset_value: self.reset_value.clone(),
            latched_value: self.latched_value.clone(),
            flags: self.flags,
            modified: self.modified,
            modification_count: self.modification_count,
            numeric_value: self.numeric_value,
            integer_value: self.integer_value,
        }
    }
}

/// Instance-owned cvar registry for one dialect.
#[derive(Clone)]
pub struct CvarRegistry {
    dialect: Dialect,
    variables: HashMap<String, CvarState>,
    order: Vec<String>,
    changed_flags: u32,
    cheats_enabled: bool,
    cheats_override: Option<bool>,
    server_active: bool,
    client_connected: bool,
    high_characters: bool,
    client_info: String,
    server_info: String,
    userinfo_dirty: bool,
    console_variables: HashSet<String>,
    info_targets: Vec<InfoTarget>,
    command_exists: Option<Rc<CommandExistsCallback>>,
    notifications: Vec<String>,
    effects: Vec<CvarEffect>,
}

impl CvarRegistry {
    /// Create an empty registry for a dialect.
    #[must_use]
    pub fn new(dialect: Dialect) -> Self {
        Self {
            dialect,
            variables: HashMap::new(),
            order: Vec::new(),
            changed_flags: 0,
            cheats_enabled: true,
            cheats_override: None,
            server_active: false,
            client_connected: false,
            high_characters: false,
            client_info: String::new(),
            server_info: String::new(),
            userinfo_dirty: false,
            console_variables: HashSet::new(),
            info_targets: vec![InfoTarget::ClientUserinfo, InfoTarget::ServerInfo],
            command_exists: None,
            notifications: Vec::new(),
            effects: Vec::new(),
        }
    }

    /// Registry dialect.
    #[must_use]
    pub fn dialect(&self) -> Dialect {
        self.dialect
    }

    /// Install the `commandExists` callback used by Q1 registration.
    pub fn set_command_exists(&mut self, callback: Rc<CommandExistsCallback>) {
        self.command_exists = Some(callback);
    }

    /// Apply archived values as `seta` commands (donor `applyArchive`).
    pub fn apply_archive(&mut self, entries: &[CvarArchiveEntry]) -> Result<(), CvarError> {
        for entry in entries {
            self.set_command_flags(&entry.name, &entry.value, SetCommandKind::Archive)?;
        }
        Ok(())
    }

    /// Override cheat permission (a local game consults its live authority).
    pub fn set_cheats_override(&mut self, allowed: Option<bool>) {
        self.cheats_override = allowed;
    }

    /// Take queued notification lines.
    #[must_use]
    pub fn take_notifications(&mut self) -> Vec<String> {
        std::mem::take(&mut self.notifications)
    }

    /// Take queued effects.
    #[must_use]
    pub fn take_effects(&mut self) -> Vec<CvarEffect> {
        std::mem::take(&mut self.effects)
    }

    fn print(&mut self, text: &str) {
        self.notifications.push(text.to_string());
    }

    fn key(&self, name: &str) -> String {
        if self.dialect == Dialect::Q3 {
            ascii_fold(name)
        } else {
            name.to_string()
        }
    }

    fn numbers(&self, value: &str) -> (f32, i32) {
        let numeric = if self.dialect.is_q1() {
            quake_atof(value) as f32
        } else {
            native_atof(value).unwrap_or(0.0) as f32
        };
        (numeric, native_atoi(value).unwrap_or(0))
    }

    /// Find a variable snapshot by name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<CvarSnapshot> {
        let key = self.key(&source_command_text(name).ok()?);
        self.variables.get(&key).map(CvarState::snapshot)
    }

    /// Current value text, or empty when unregistered.
    #[must_use]
    pub fn variable_string(&self, name: &str) -> String {
        self.get(name).map_or_else(String::new, |snapshot| snapshot.value)
    }

    /// Current numeric value, or zero when unregistered.
    #[must_use]
    pub fn variable_value(&self, name: &str) -> f32 {
        self.get(name).map_or(0.0, |snapshot| snapshot.numeric_value)
    }

    /// Whether the name was created from the console.
    #[must_use]
    pub fn is_console_created(&self, name: &str) -> bool {
        source_command_text(name).is_ok_and(|text| self.console_variables.contains(&self.key(&text)))
    }

    /// Snapshots newest-first, optionally filtered by flag mask.
    #[must_use]
    pub fn snapshots(&self, flags: u32) -> Vec<CvarSnapshot> {
        self.order
            .iter()
            .rev()
            .filter_map(|key| self.variables.get(key))
            .filter(|state| flags == 0 || state.flags & flags != 0)
            .map(CvarState::snapshot)
            .collect()
    }

    /// Complete a partial name against registered variables.
    pub fn complete(&self, partial_input: &str) -> Result<Option<String>, CvarError> {
        let partial = source_command_text(partial_input)?;
        if partial.is_empty() {
            return Ok(None);
        }
        if self.dialect != Dialect::Q1Netquake {
            if let Some(found) = self.get(&partial) {
                return Ok(Some(found.name));
            }
        }
        let prefix = self.key(&partial);
        Ok(self
            .snapshots(0)
            .into_iter()
            .find(|state| self.key(&state.name).starts_with(&prefix))
            .map(|state| state.name))
    }

    fn allow_cheats(&self) -> bool {
        if let Some(allowed) = self.cheats_override {
            return allowed;
        }
        self.get("sv_cheats")
            .map_or(self.cheats_enabled, |cheats| cheats.integer_value != 0)
    }

    fn apply_value(&mut self, key: &str, value: String, mark: bool) {
        let (numeric_value, integer_value) = self.numbers(&value);
        if let Some(state) = self.variables.get_mut(key) {
            if mark {
                state.modified = true;
                state.modification_count += 1;
            }
            state.value = value;
            state.numeric_value = numeric_value;
            state.integer_value = integer_value;
        }
    }

    fn valid_info(text: &str) -> bool {
        !text.chars().any(|c| c == '\\' || c == '"' || c == ';')
    }

    /// Register a variable. Re-registration merges flags per dialect and
    /// never replaces a live value.
    pub fn register(
        &mut self,
        name_input: &str,
        default_input: &str,
        flag_input: u32,
    ) -> Result<Option<CvarSnapshot>, CvarError> {
        let mut name = source_command_text(name_input)?;
        let default_value = source_command_text(default_input)?;
        if self.dialect == Dialect::Q3 && !Self::valid_info(&name) {
            self.print(&format!("invalid cvar name string: {name}\n"));
            name = "BADNAME".to_string();
        }
        if self.dialect.is_q2() && (flag_input & 6) != 0 && !Self::valid_info(&name) {
            self.print("invalid info cvar name\n");
            return Ok(None);
        }
        let key = self.key(&name);
        if let Some(existing) = self.variables.get(&key).map(CvarState::snapshot) {
            if self.dialect.is_q1() {
                if self.console_variables.remove(&key) {
                    let flags = flag_input;
                    if let Some(state) = self.variables.get_mut(&key) {
                        state.reset_value = default_value.clone();
                        state.flags |= flags;
                    }
                    if self.dialect == Dialect::Q1Quakeworld {
                        let value = default_value.clone();
                        self.propagate(&key, &value, true)?;
                    }
                } else {
                    self.print(&format!("Can't register variable {name}, allready defined\n"));
                }
                return Ok(Some(existing));
            }
            if self.dialect == Dialect::Q3 {
                if (existing.flags & flags::USER_CREATED) != 0
                    && (flag_input & flags::USER_CREATED) == 0
                    && !default_value.is_empty()
                {
                    if let Some(state) = self.variables.get_mut(&key) {
                        state.flags &= !flags::USER_CREATED;
                        state.reset_value = default_value.clone();
                    }
                    self.changed_flags |= flag_input;
                }
                if existing.reset_value.is_empty() {
                    if let Some(state) = self.variables.get_mut(&key) {
                        state.reset_value = default_value.clone();
                    }
                }
            }
            if self.dialect.is_q2() && (existing.flags & q2_flags::CUSTOM) != 0 && (flag_input & q2_flags::CUSTOM) == 0
            {
                let needs_reset = (flag_input & (q2_flags::READ_ONLY | q2_flags::NO_SET)) != 0
                    || ((flag_input & q2_flags::CHEAT) != 0 && !self.allow_cheats())
                    || ((flag_input & (q2_flags::USER_INFO | q2_flags::SERVER_INFO)) != 0
                        && !Self::valid_info(&existing.value));
                if let Some(state) = self.variables.get_mut(&key) {
                    state.reset_value = default_value.clone();
                    state.flags &= !q2_flags::CUSTOM;
                }
                if needs_reset {
                    let default = default_value.clone();
                    self.set(&name.clone(), &default, true)?;
                }
            }
            if let Some(state) = self.variables.get_mut(&key) {
                state.flags |= flag_input;
                if self.dialect.is_q2() && (flag_input & Q2_NO_ARCHIVE) != 0 {
                    state.flags &= !q2_flags::ARCHIVE;
                }
            }
            if self.dialect == Dialect::Q3 {
                let latched = self.variables.get(&key).and_then(|state| state.latched_value.clone());
                if let Some(latched) = latched {
                    if let Some(state) = self.variables.get_mut(&key) {
                        state.latched_value = None;
                    }
                    self.set(&name.clone(), &latched, true)?;
                }
            }
            return Ok(self.variables.get(&key).map(CvarState::snapshot));
        }
        if self.dialect.is_q1() && self.command_exists.as_ref().is_some_and(|exists| exists(&name)) {
            self.print(&format!("Cvar_RegisterVariable: {name} is a command\n"));
            return Ok(None);
        }
        if self.dialect.is_q2() && (flag_input & 6) != 0 && !Self::valid_info(&default_value) {
            self.print("invalid info cvar value\n");
            return Ok(None);
        }
        if self.dialect == Dialect::Q3 && self.variables.len() == MAX_CVARS {
            return Err(CvarError::Domain("MAX_CVARS".to_string()));
        }
        let (numeric_value, integer_value) = self.numbers(&default_value);
        let state = CvarState {
            name: name.clone(),
            value: default_value.clone(),
            reset_value: default_value.clone(),
            latched_value: None,
            flags: flag_input,
            modified: true,
            modification_count: 1,
            numeric_value,
            integer_value,
        };
        self.variables.insert(key.clone(), state);
        self.order.push(key.clone());
        if self.dialect == Dialect::Q1Quakeworld {
            self.propagate(&key, &default_value, true)?;
        }
        Ok(self.variables.get(&key).map(CvarState::snapshot))
    }

    /// Set a variable. Q3/Q2 create unknown variables; Q1 reports them.
    pub fn set(&mut self, name_input: &str, value_input: &str, force: bool) -> Result<Option<CvarSnapshot>, CvarError> {
        let mut name = source_command_text(name_input)?;
        if self.dialect == Dialect::Q3 && !Self::valid_info(&name) {
            self.print(&format!("invalid cvar name string: {name}\n"));
            name = "BADNAME".to_string();
        }
        let value = source_command_text(value_input)?;
        let key = self.key(&name);
        if !self.variables.contains_key(&key) {
            if self.dialect.is_q1() {
                self.print(&format!("Cvar_Set: variable {name} not found\n"));
                return Ok(None);
            }
            let flags = if self.dialect == Dialect::Q3 && !force {
                flags::USER_CREATED
            } else {
                0
            };
            return self.register(&name, &value, flags);
        }
        if self.dialect.is_q1() {
            let changed = self.variables.get(&key).is_some_and(|state| state.value != value);
            self.propagate(&key, &value, changed)?;
            self.apply_value(&key, value, false);
            return Ok(self.variables.get(&key).map(CvarState::snapshot));
        }
        if self.dialect.is_q2() {
            let info = self.variables.get(&key).is_some_and(|state| (state.flags & 6) != 0);
            if info && !Self::valid_info(&value) {
                self.print("invalid info cvar value\n");
                return Ok(self.variables.get(&key).map(CvarState::snapshot));
            }
            if !force {
                let state = self.variables.get(&key).map(CvarState::snapshot).unwrap();
                if (state.flags & q2_flags::READ_ONLY) != 0 {
                    self.print(&format!("{name} is read only.\n"));
                    return Ok(Some(state));
                }
                if (state.flags & q2_flags::CHEAT) != 0 && !self.allow_cheats() {
                    self.print(&format!("{name} is cheat protected.\n"));
                    return Ok(Some(state));
                }
                if (state.flags & q2_flags::NO_SET) != 0 {
                    self.print(&format!("{name} is write protected.\n"));
                    return Ok(Some(state));
                }
                if (state.flags & q2_flags::LATCH) != 0 {
                    let current = state.latched_value.clone().unwrap_or(state.value.clone());
                    if value == current {
                        return Ok(Some(state));
                    }
                    if let Some(stored) = self.variables.get_mut(&key) {
                        stored.latched_value = None;
                    }
                    if self.server_active {
                        self.print(&format!("{name} will be changed for next game.\n"));
                        if let Some(stored) = self.variables.get_mut(&key) {
                            stored.latched_value = Some(value);
                        }
                    } else {
                        self.apply_value(&key, value, false);
                        self.game_directory(&key);
                    }
                    return Ok(self.variables.get(&key).map(CvarState::snapshot));
                }
            } else if let Some(stored) = self.variables.get_mut(&key) {
                stored.latched_value = None;
            }
        } else {
            let state = self.variables.get(&key).map(CvarState::snapshot).unwrap();
            if value == state.value {
                return Ok(Some(state));
            }
            self.changed_flags |= state.flags;
            if !force {
                if (state.flags & flags::READ_ONLY) != 0 {
                    self.print(&format!("{name} is read only.\n"));
                    return Ok(Some(state));
                }
                if (state.flags & flags::INIT) != 0 {
                    self.print(&format!("{name} is write protected.\n"));
                    return Ok(Some(state));
                }
                if (state.flags & flags::LATCH) != 0 {
                    if state.latched_value.as_deref() == Some(value.as_str()) {
                        return Ok(Some(state));
                    }
                    self.print(&format!("{name} will be changed upon restarting.\n"));
                    if let Some(stored) = self.variables.get_mut(&key) {
                        stored.latched_value = Some(value);
                        stored.modified = true;
                        stored.modification_count += 1;
                    }
                    return Ok(self.variables.get(&key).map(CvarState::snapshot));
                }
                if (state.flags & flags::CHEAT) != 0 && !self.allow_cheats() {
                    self.print(&format!("{name} is cheat protected.\n"));
                    return Ok(Some(state));
                }
            } else if let Some(stored) = self.variables.get_mut(&key) {
                stored.latched_value = None;
            }
        }
        let changed = self.variables.get(&key).is_some_and(|state| state.value != value);
        if changed {
            self.apply_value(&key, value, true);
            if self.dialect.is_q2()
                && self
                    .variables
                    .get(&key)
                    .is_some_and(|state| (state.flags & q2_flags::USER_INFO) != 0)
            {
                self.userinfo_dirty = true;
            }
        }
        Ok(self.variables.get(&key).map(CvarState::snapshot))
    }

    /// Console `set`: like [`CvarRegistry::set`], but a Q2 write of the
    /// current value clears the latch instead.
    pub fn set_console(&mut self, name: &str, value: &str) -> Result<Option<CvarSnapshot>, CvarError> {
        let key = self.key(&source_command_text(name)?);
        if self.dialect.is_q2() && self.variables.get(&key).is_some_and(|state| state.value == value) {
            if let Some(state) = self.variables.get_mut(&key) {
                state.latched_value = None;
            }
            return Ok(self.variables.get(&key).map(CvarState::snapshot));
        }
        self.set(name, value, false)
    }

    /// `seta`/`setu`/`sets`: set a value and OR in the command flag,
    /// creating console/user variables as needed.
    pub fn set_command_flags(&mut self, name: &str, value: &str, kind: SetCommandKind) -> Result<(), CvarError> {
        let q2 = self.dialect.is_q2();
        let flag = match kind {
            SetCommandKind::Archive => {
                if q2 {
                    q2_flags::ARCHIVE
                } else {
                    flags::ARCHIVE
                }
            }
            SetCommandKind::Userinfo => {
                if q2 {
                    q2_flags::USER_INFO
                } else {
                    flags::USER_INFO
                }
            }
            SetCommandKind::Serverinfo => {
                if q2 {
                    q2_flags::SERVER_INFO
                } else {
                    flags::SERVER_INFO
                }
            }
        };
        let clean_name = source_command_text(name)?;
        let clean_value = source_command_text(value)?;
        let key = self.key(&clean_name);
        let previous_flags = self.variables.get(&key).map_or(0, |state| state.flags);
        if kind != SetCommandKind::Archive
            && (!Self::valid_info(&clean_name)
                || !Self::valid_info(&clean_value)
                || (q2 && (clean_name.len() >= 64 || clean_value.len() >= 64)))
        {
            self.print("invalid info cvar name or value\n");
            return Ok(());
        }
        if !self.variables.contains_key(&key) {
            let create_flags = flag
                | if q2 {
                    q2_flags::CUSTOM
                } else if self.dialect == Dialect::Q3 {
                    flags::USER_CREATED
                } else {
                    0
                };
            let created = self.register(&clean_name, &clean_value, create_flags)?;
            if let Some(created) = created {
                if self.dialect.is_q1() {
                    let created_key = self.key(&created.name);
                    self.console_variables.insert(created_key);
                }
            } else {
                return Ok(());
            }
        } else {
            self.set_console(&clean_name, &clean_value)?;
            let retained_bad = self
                .variables
                .get(&key)
                .is_some_and(|state| !Self::valid_info(&state.value) || (q2 && state.value.len() >= 64));
            if kind != SetCommandKind::Archive && retained_bad {
                self.print("invalid retained info cvar value\n");
                return Ok(());
            }
            if kind != SetCommandKind::Archive && q2 {
                if let Some(state) = self.variables.get_mut(&key) {
                    state.flags &= !(q2_flags::USER_INFO | q2_flags::SERVER_INFO);
                }
            }
            if kind == SetCommandKind::Archive || !q2 || (previous_flags & Q2_NO_ARCHIVE) == 0 {
                if let Some(state) = self.variables.get_mut(&key) {
                    state.flags |= flag;
                }
            }
        }
        if q2
            && self
                .variables
                .get(&key)
                .is_some_and(|state| (state.flags & Q2_NO_ARCHIVE) != 0)
        {
            if let Some(state) = self.variables.get_mut(&key) {
                state.flags &= !q2_flags::ARCHIVE;
            }
        }
        if kind != SetCommandKind::Archive {
            if q2 {
                let current = self.variables.get(&key).map_or(0, |state| state.flags);
                if ((previous_flags | current) & q2_flags::USER_INFO) != 0 {
                    self.userinfo_dirty = true;
                }
            } else {
                let value = self
                    .variables
                    .get(&key)
                    .map_or_else(String::new, |state| state.value.clone());
                self.propagate(&key, &value, true)?;
            }
        }
        Ok(())
    }

    /// Quake II `Cvar_FullSet`: replace the value and the whole flag word.
    pub fn full_set(&mut self, name: &str, value: &str, flag_word: u32) -> Result<Option<CvarSnapshot>, CvarError> {
        if !self.dialect.is_q2() {
            return Err(CvarError::Domain("Cvar_FullSet belongs to Quake II".to_string()));
        }
        let key = self.key(&source_command_text(name)?);
        let clean_value = source_command_text(value)?;
        if !self.variables.contains_key(&key) {
            return self.register(name, &clean_value, flag_word);
        }
        if self
            .variables
            .get(&key)
            .is_some_and(|state| (state.flags & q2_flags::USER_INFO) != 0)
        {
            self.userinfo_dirty = true;
        }
        self.apply_value(&key, clean_value, true);
        if let Some(state) = self.variables.get_mut(&key) {
            state.flags = flag_word;
        }
        Ok(self.variables.get(&key).map(CvarState::snapshot))
    }

    /// `Cvar_SetValue`: format a float and set it (forced on Q3).
    pub fn set_value(&mut self, name: &str, value: f64) -> Result<Option<CvarSnapshot>, CvarError> {
        let text = cvar_value_text(value, !self.dialect.is_q1())?;
        if text.len() >= 32 {
            if self.dialect.is_q1() {
                return Err(CvarError::Domain("Cvar_SetValue overflows source val[32]".to_string()));
            }
            self.print(&format!("Com_sprintf: overflow of {} in 32\n", text.len()));
        }
        let truncated: String = text.chars().take(31).collect();
        self.set(name, &truncated, self.dialect == Dialect::Q3)
    }

    /// Defer a value on a registered variable without changing flags.
    pub fn stage(&mut self, name: &str, input: &str) -> Result<CvarSnapshot, CvarError> {
        let key = self.key(&source_command_text(name)?);
        let value = source_command_text(input)?;
        let Some(state) = self.variables.get(&key).map(CvarState::snapshot) else {
            return Err(CvarError::Domain(format!("Cannot stage an unregistered cvar {name}")));
        };
        if self.dialect == Dialect::Q3 && (state.flags & (flags::READ_ONLY | flags::INIT)) != 0
            || self.dialect.is_q2() && (state.flags & q2_flags::NO_SET) != 0
        {
            return Err(CvarError::Domain(format!("Cannot stage a protected cvar {name}")));
        }
        if self.dialect.is_q2() && (state.flags & 6) != 0 && !Self::valid_info(&value) {
            return Err(CvarError::Domain(format!("Invalid staged info cvar {name}")));
        }
        let pending = if value == state.value { None } else { Some(value) };
        if self.variables.get(&key).and_then(|stored| stored.latched_value.clone()) != pending {
            if let Some(stored) = self.variables.get_mut(&key) {
                stored.latched_value = pending;
                stored.modified = true;
                stored.modification_count += 1;
            }
            if self.dialect == Dialect::Q3 {
                self.changed_flags |= state.flags;
            }
        }
        Ok(self.variables.get(&key).map(CvarState::snapshot).unwrap())
    }

    /// Apply latched values (all, or one name).
    pub fn apply_latched(&mut self, name: Option<&str>) -> Result<Vec<CvarSnapshot>, CvarError> {
        let filter = name
            .map(|text| source_command_text(text).map(|clean| self.key(&clean)))
            .transpose()?;
        let mut changed = Vec::new();
        let order: Vec<String> = self.order.iter().rev().cloned().collect();
        for key in order {
            if filter.as_ref().is_some_and(|wanted| *wanted != key) {
                continue;
            }
            let Some(latched) = self.variables.get(&key).and_then(|state| state.latched_value.clone()) else {
                continue;
            };
            if let Some(state) = self.variables.get_mut(&key) {
                state.latched_value = None;
            }
            let mark = self.dialect == Dialect::Q3;
            self.apply_value(&key, latched, mark);
            if self.dialect.is_q2() {
                self.game_directory(&key);
            }
            if let Some(state) = self.variables.get(&key) {
                changed.push(state.snapshot());
            }
        }
        Ok(changed)
    }

    /// Reset one variable to its default.
    pub fn reset(&mut self, name: &str, force: bool) -> Result<Option<CvarSnapshot>, CvarError> {
        let Some(state) = self.get(name) else { return Ok(None) };
        self.set(name, &state.reset_value.clone(), force)
    }

    /// Console `reset`: like `set_console` to the default, skipping
    /// protected variables when resetting all.
    pub fn reset_console(&mut self, name: &str, all: bool) -> Result<(), CvarError> {
        let Some(state) = self.get(name) else { return Ok(()) };
        if all && (name == "game" || name == "fs_game") {
            return Ok(());
        }
        if all
            && (self.dialect.is_q2() && (state.flags & (q2_flags::NO_SET | q2_flags::READ_ONLY)) != 0
                || self.dialect == Dialect::Q3
                    && (state.flags & (flags::READ_ONLY | flags::INIT | flags::NO_RESTART)) != 0)
        {
            return Ok(());
        }
        self.set_console(name, &state.reset_value.clone())?;
        Ok(())
    }

    /// Quake III `cvar_restart`: reset every variable and delete
    /// user-created ones.
    pub fn reset_all(&mut self) -> Result<(), CvarError> {
        if self.dialect != Dialect::Q3 {
            return Err(CvarError::Domain("cvar_restart belongs to Quake III".to_string()));
        }
        let order: Vec<String> = self.order.iter().rev().cloned().collect();
        for key in order {
            let Some(state) = self.variables.get(&key).map(CvarState::snapshot) else {
                continue;
            };
            if (state.flags & (flags::READ_ONLY | flags::INIT | flags::NO_RESTART)) != 0 {
                continue;
            }
            if (state.flags & flags::USER_CREATED) != 0 {
                self.variables.remove(&key);
                self.order.retain(|entry| *entry != key);
            } else {
                self.set(&state.name.clone(), &state.reset_value.clone(), true)?;
            }
        }
        Ok(())
    }

    /// Enable or disable cheats. Disabling on Q3 resets cheat variables.
    pub fn set_cheats_enabled(&mut self, enabled: bool) -> Result<(), CvarError> {
        self.cheats_enabled = enabled;
        if enabled || self.dialect != Dialect::Q3 {
            return Ok(());
        }
        let order: Vec<String> = self.order.iter().rev().cloned().collect();
        for key in order {
            let cheat = self
                .variables
                .get(&key)
                .is_some_and(|state| (state.flags & flags::CHEAT) != 0);
            if !cheat {
                continue;
            }
            if let Some(state) = self.variables.get_mut(&key) {
                state.latched_value = None;
            }
            let snapshot = self.variables.get(&key).map(CvarState::snapshot).unwrap();
            self.set(&snapshot.name.clone(), &snapshot.reset_value.clone(), true)?;
        }
        Ok(())
    }

    /// Build the info string for a flag mask, newest-first.
    pub fn info_string(&mut self, flag_mask: u32, maximum_length: Option<usize>) -> Result<String, CvarError> {
        let maximum_length = maximum_length.unwrap_or(if self.dialect == Dialect::Q3 { 1024 } else { 512 });
        let target = if (flag_mask & flags::USER_INFO) != 0 {
            InfoTarget::ClientUserinfo
        } else {
            InfoTarget::ServerInfo
        };
        let mut info = String::new();
        let order: Vec<String> = self.order.iter().rev().cloned().collect();
        for key in order {
            let Some(state) = self.variables.get(&key).map(CvarState::snapshot) else {
                continue;
            };
            if (state.flags & flag_mask) == 0 {
                continue;
            }
            if self.dialect.is_q2() && (state.flags & q2_flags::PRIVATE) != 0 {
                continue;
            }
            let options = InfoOptions {
                dialect: self.dialect,
                maximum_length,
                target,
                server_high_characters: self.high_characters,
            };
            let mut notifications = Vec::new();
            let updated = set_info_value(&info, &state.name, &state.value, options, &mut |text| {
                notifications.push(text.to_string());
            })?;
            self.notifications.extend(notifications);
            info = updated;
        }
        Ok(info)
    }

    /// Last propagated info string for a target (QW).
    #[must_use]
    pub fn propagated_info(&self, target: InfoTarget) -> &str {
        match target {
            InfoTarget::ClientUserinfo => &self.client_info,
            InfoTarget::ServerInfo => &self.server_info,
        }
    }

    /// Whether userinfo changed since the last clear.
    #[must_use]
    pub fn userinfo_modified(&self) -> bool {
        self.userinfo_dirty
    }

    /// Clear the userinfo-modified flag.
    pub fn clear_userinfo_modified(&mut self) {
        self.userinfo_dirty = false;
    }

    /// Take and clear accumulated modification flags.
    #[must_use]
    pub fn take_modified_flags(&mut self) -> u32 {
        let flags = self.changed_flags;
        self.changed_flags = 0;
        flags
    }

    /// OR flags into the modification accumulator.
    pub fn mark_modified_flags(&mut self, flag_mask: u32) {
        self.changed_flags |= flag_mask;
    }

    /// Clear flags from the modification accumulator.
    pub fn clear_modified_flags(&mut self, flag_mask: u32) {
        self.changed_flags &= !flag_mask;
    }

    /// OR flags into a variable.
    pub fn add_flags(&mut self, name: &str, flag_mask: u32) -> Result<(), CvarError> {
        let key = self.key(&source_command_text(name)?);
        if let Some(state) = self.variables.get_mut(&key) {
            state.flags |= flag_mask;
        }
        Ok(())
    }

    /// Clear a variable's modified bit.
    pub fn clear_modified(&mut self, name: &str) -> Result<(), CvarError> {
        let key = self.key(&source_command_text(name)?);
        if let Some(state) = self.variables.get_mut(&key) {
            state.modified = false;
        }
        Ok(())
    }

    /// Note server activity (drives Q2 latch and Q1 broadcast behavior).
    pub fn set_server_active(&mut self, active: bool) {
        self.server_active = active;
    }

    /// Note client connection (drives QW `setinfo` commands).
    pub fn set_client_connected(&mut self, connected: bool) {
        self.client_connected = connected;
    }

    /// Note high-character support for QW server info.
    pub fn set_server_high_characters(&mut self, enabled: bool) {
        self.high_characters = enabled;
    }

    fn archive_states(&self, include: &dyn Fn(&str) -> bool) -> Vec<CvarSnapshot> {
        self.order
            .iter()
            .rev()
            .filter_map(|key| self.variables.get(key))
            .filter(|state| include(&state.name))
            .filter(|state| !(self.dialect.is_q2() && (state.flags & Q2_NO_ARCHIVE) != 0))
            .filter(|state| (state.flags & flags::ARCHIVE) != 0)
            .filter(|state| !(self.dialect == Dialect::Q3 && ascii_fold(&state.name) == "cl_cdkey"))
            .map(CvarState::snapshot)
            .collect()
    }

    fn archive_value(&self, state: &CvarSnapshot) -> String {
        if self.dialect == Dialect::Q3 {
            state.latched_value.clone().unwrap_or_else(|| state.value.clone())
        } else {
            state.value.clone()
        }
    }

    /// Archive entries for configuration files.
    #[must_use]
    pub fn archive_entries(&self, include: &dyn Fn(&str) -> bool) -> Vec<CvarArchiveEntry> {
        self.archive_states(include)
            .iter()
            .map(|state| CvarArchiveEntry {
                name: state.name.clone(),
                value: self.archive_value(state),
            })
            .collect()
    }

    /// Archive lines (`seta`/`set`/bare per dialect and origin).
    #[must_use]
    pub fn archive_commands(&self, include: &dyn Fn(&str) -> bool) -> Vec<String> {
        self.archive_states(include)
            .iter()
            .map(|state| {
                let custom = self.dialect.is_q2() && (state.flags & q2_flags::CUSTOM) != 0;
                let prefix =
                    if self.dialect == Dialect::Q3 || self.console_variables.contains(&self.key(&state.name)) || custom
                    {
                        "seta "
                    } else if self.dialect.is_q2() {
                        "set "
                    } else {
                        ""
                    };
                format!("{prefix}{} \"{}\"", state.name, self.archive_value(state))
            })
            .collect()
    }

    /// Write archive lines, truncating non-Q1 lines at 1023 bytes.
    pub fn write_variables(&mut self, include: &dyn Fn(&str) -> bool, write: &mut dyn FnMut(&str)) {
        for command in self.archive_commands(include) {
            let line = format!("{command}\n");
            if !self.dialect.is_q1() && line.len() >= 1024 {
                self.print(&format!("Com_sprintf: overflow of {} in 1024\n", line.len()));
            }
            if self.dialect.is_q1() {
                write(&line);
            } else {
                write(&line.chars().take(1023).collect::<String>());
            }
        }
    }

    fn game_directory(&mut self, key: &str) {
        if let Some(state) = self.variables.get(key) {
            if state.name == "game" {
                self.effects.push(CvarEffect::GameDirectory {
                    directory: state.value.clone(),
                });
            }
        }
    }

    fn propagate(&mut self, key: &str, value: &str, changed: bool) -> Result<(), CvarError> {
        let Some(state) = self.variables.get(key).map(CvarState::snapshot) else {
            return Ok(());
        };
        if self.dialect == Dialect::Q1Netquake {
            if (state.flags & flags::SERVER_INFO) != 0 && changed && self.server_active {
                self.effects.push(CvarEffect::Broadcast {
                    text: format!("\"{}\" changed to \"{value}\"\n", state.name),
                });
            }
            return Ok(());
        }
        if self.dialect != Dialect::Q1Quakeworld {
            return Ok(());
        }
        for target in self.info_targets.clone() {
            let flag = if target == InfoTarget::ClientUserinfo {
                flags::USER_INFO
            } else {
                flags::SERVER_INFO
            };
            if (state.flags & flag) == 0 {
                continue;
            }
            let maximum_length = if target == InfoTarget::ClientUserinfo { 196 } else { 512 };
            let options = InfoOptions {
                dialect: self.dialect,
                maximum_length,
                target,
                server_high_characters: self.high_characters,
            };
            let mut notifications = Vec::new();
            let current = self.propagated_info(target).to_string();
            let updated = set_info_value(&current, &state.name, value, options, &mut |text| {
                notifications.push(text.to_string());
            })?;
            self.notifications.extend(notifications);
            if target == InfoTarget::ClientUserinfo {
                self.client_info = updated.clone();
                self.effects.push(CvarEffect::Userinfo {
                    name: state.name.clone(),
                    value: value.to_string(),
                    info: updated,
                    command: self
                        .client_connected
                        .then(|| format!("setinfo \"{}\" \"{value}\"\n", state.name)),
                });
            } else {
                self.server_info = updated.clone();
                self.effects.push(CvarEffect::ServerInfo {
                    name: state.name.clone(),
                    value: value.to_string(),
                    info: updated,
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::flags;
    use super::*;

    #[test]
    fn quake_atof_matches_donor() {
        assert_eq!(quake_atof("-42"), -42.0);
        assert_eq!(quake_atof("0x10"), 16.0);
        assert_eq!(quake_atof("0Xff"), 255.0);
        assert_eq!(quake_atof("'A"), 65.0);
        assert_eq!(quake_atof("3.5"), 3.5);
        assert_eq!(quake_atof("12abc"), 12.0);
        assert_eq!(quake_atof(""), 0.0);
    }

    #[test]
    fn cvar_value_text_rounds_like_donor() {
        assert_eq!(cvar_value_text(5.0, true).unwrap(), "5");
        assert_eq!(cvar_value_text(5.0, false).unwrap(), "5.000000");
        assert_eq!(cvar_value_text(0.007_812_5, false).unwrap(), "0.007812");
        assert_eq!(cvar_value_text(0.023_437_5, false).unwrap(), "0.023438");
        assert_eq!(cvar_value_text(-0.0, true).unwrap(), "0");
        assert!(cvar_value_text(f64::INFINITY, false).is_err());
    }

    #[test]
    fn info_strings_follow_family_rules() {
        let mut printed = Vec::new();
        let options = InfoOptions {
            dialect: Dialect::Q3,
            maximum_length: 1024,
            target: InfoTarget::ServerInfo,
            server_high_characters: false,
        };
        let info = set_info_value("", "map", "q3dm1", options, &mut |text| printed.push(text.to_string())).unwrap();
        assert_eq!(info, "\\map\\q3dm1");
        let removed = set_info_value(&info, "map", "", options, &mut |text| printed.push(text.to_string())).unwrap();
        assert_eq!(removed, "");
        let kept = set_info_value("", "a\\b", "c", options, &mut |text| printed.push(text.to_string())).unwrap();
        assert_eq!(kept, "");
        assert!(!printed.is_empty());
    }

    #[test]
    fn registry_tracks_values_newest_first() {
        let mut registry = CvarRegistry::new(Dialect::Q3);
        registry.register("sv_fps", "20", flags::ARCHIVE).unwrap();
        registry.register("g_speed", "320", flags::NONE).unwrap();
        let names: Vec<String> = registry
            .snapshots(0)
            .into_iter()
            .map(|snapshot| snapshot.name)
            .collect();
        assert_eq!(names, vec!["g_speed".to_string(), "sv_fps".to_string()]);
        let updated = registry.set("SV_FPS", "30", false).unwrap().unwrap();
        assert_eq!(updated.value, "30");
        assert_eq!(updated.integer_value, 30);
        assert!((f64::from(updated.numeric_value) - 30.0).abs() < f64::EPSILON);
        assert_eq!(registry.variable_string("sv_fps"), "30");
        let missing = CvarRegistry::new(Dialect::Q1Netquake).variable_string("nope");
        assert_eq!(missing, "");
    }

    #[test]
    fn latch_restart_and_cheat_rules_match_donor() {
        let mut registry = CvarRegistry::new(Dialect::Q3);
        registry.register("r_mode", "3", flags::LATCH).unwrap();
        let latched = registry.set("r_mode", "4", false).unwrap().unwrap();
        assert_eq!(latched.value, "3");
        assert_eq!(latched.latched_value.as_deref(), Some("4"));
        let applied = registry.apply_latched(None).unwrap();
        assert_eq!(applied[0].value, "4");

        registry.register("god", "0", flags::CHEAT).unwrap();
        let denied = registry.set("god", "1", false).unwrap().unwrap();
        assert_eq!(denied.value, "1");
        registry.register("sv_cheats", "0", flags::NONE).unwrap();
        let blocked = registry.set("god", "0", false).unwrap().unwrap();
        assert_eq!(blocked.value, "1");

        let mut q2 = CvarRegistry::new(Dialect::Q2Classic);
        q2.register("gl_mode", "3", q2_flags::LATCH).unwrap();
        q2.set_server_active(true);
        let held = q2.set("gl_mode", "4", false).unwrap().unwrap();
        assert_eq!(held.latched_value.as_deref(), Some("4"));
        let applied = q2.apply_latched(None).unwrap();
        assert_eq!(applied[0].value, "4");

        let mut q1 = CvarRegistry::new(Dialect::Q1Netquake);
        assert!(q1.set("nope", "1", false).unwrap().is_none());
        q1.register("temp", "1", flags::NONE).unwrap();
        q1.set_command_flags("temp2", "2", SetCommandKind::Archive).unwrap();
        assert!(q1.is_console_created("temp2"));
        let commands = q1.archive_commands(&|_| true);
        assert!(commands.iter().any(|line| line == "seta temp2 \"2\""));
    }
}
