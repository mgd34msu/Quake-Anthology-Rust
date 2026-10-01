//! Engine-owned Q3 server storage shared by the selected game and its host.
//!
//! Port of donor `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/simulation/q3/server-state.ts`
//! (`Q3ServerState`, `Q3ServerStateOptions`).
//!
//! Two donor pieces arrive through seams. `registerQ3ServerCvars` (donor
//! `src/app/bootstrap/q3-common-cvars.ts`, outside the sim lane) does not
//! exist in this worktree, so [`Q3ServerStateOptions::register_server_cvars`]
//! injects it: the caller supplies the registration callback and this module
//! never duplicates the definition table. The `CvarRegistry` save image (donor
//! `CvarRegistry.captureSaveState`/`restoreSaveState`) likewise has no
//! `qa-core` home (the registry port defers it), so this module captures the
//! variable table through the public snapshot API: names, values, reset
//! values, flags, and latched values round-trip exactly, while
//! modified-counters follow live registry semantics.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use qa_core::cmd::Dialect;
use qa_core::cvar::{flags as cvar_flags, CvarArchiveEntry, CvarRegistry, SetCommandKind};
use qa_core::identity::SessionId;
use qa_world::save::value::{arr, boolean, int, obj, str as save_str, SaveJson, SaveReader};
use qa_world::WorldError;

use super::host::Q3HostSettings;

/// Maximum configstring slots (donor `1024`).
pub const Q3_CONFIGSTRINGS: i32 = 1024;

/// Maximum client slots (donor `64`).
pub const Q3_MAX_CLIENT_SLOTS: i32 = 64;

/// Stored client user command, mirroring donor `WireUserCommand` from
/// `src/network/q3/message.ts` (unpacked numbers form; `qa-net`'s packed
/// `WireUserCommand` is a different representation).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3StoredUserCommand {
    /// Server time in milliseconds.
    pub server_time: i32,
    /// View angles as wire shorts.
    pub angles: [i32; 3],
    /// Forward impulse.
    pub forwardmove: i32,
    /// Right impulse.
    pub rightmove: i32,
    /// Vertical impulse.
    pub upmove: i32,
    /// Button bitmask.
    pub buttons: i32,
    /// Selected weapon.
    pub weapon: i32,
}

/// Server cvar registration seam: donor `registerQ3ServerCvars` from
/// `src/app/bootstrap/q3-common-cvars.ts` (missing value, injected by the
/// caller; never duplicated here). Arguments are the registry, the maximum
/// client count, and the map name.
pub type Q3ServerCvarRegistration = Rc<dyn Fn(&mut CvarRegistry, usize, &str)>;

/// Construction options, mirroring donor `Q3ServerStateOptions`.
pub struct Q3ServerStateOptions {
    /// Owning session (donor registry context; the Rust registry is
    /// context-free, so this is retained for provenance only).
    pub session: SessionId,
    /// Host settings.
    pub settings: Q3HostSettings,
    /// Millisecond clock.
    pub now: Rc<dyn Fn() -> i32>,
    /// Print sink.
    pub print: Rc<dyn Fn(&str)>,
    /// Server cvar registration callback (see [`Q3ServerCvarRegistration`]).
    pub register_server_cvars: Q3ServerCvarRegistration,
}

/// Engine-owned storage shared by the selected game and its network host.
///
/// Shared by handle (`Rc`) like the donor object; interior mutability keeps
/// the donor's by-reference aliasing between `serverState`, the host engine,
/// and the guest runtime.
pub struct Q3ServerState {
    /// Cvar registry (adopted from settings or created here).
    pub cvars: Rc<RefCell<CvarRegistry>>,
    values: RefCell<HashMap<i32, String>>,
    userinfo: RefCell<HashMap<i32, String>>,
    commands: RefCell<HashMap<i32, Q3StoredUserCommand>>,
    now: Rc<dyn Fn() -> i32>,
    print: Rc<dyn Fn(&str)>,
}

impl std::fmt::Debug for Q3ServerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Q3ServerState")
            .field("configstrings", &self.values.borrow().len())
            .field("userinfo", &self.userinfo.borrow().len())
            .field("commands", &self.commands.borrow().len())
            .finish_non_exhaustive()
    }
}

impl Q3ServerState {
    /// Build server storage, registering server cvars and applying settings.
    #[must_use]
    pub fn new(options: Q3ServerStateOptions) -> Self {
        let settings = &options.settings;
        let cvars = options
            .settings
            .source_registry
            .clone()
            .unwrap_or_else(|| Rc::new(RefCell::new(CvarRegistry::new(Dialect::Q3))));
        {
            let mut registry = cvars.borrow_mut();
            (options.register_server_cvars)(&mut registry, settings.max_clients, &settings.map_name);
            set_forced(&mut registry, "sv_mapname", &settings.map_name);
            if options.settings.source_registry.is_none() {
                for entry in &settings.source_archive {
                    apply_archive_entry(&mut registry, entry);
                }
            }
            set_forced(&mut registry, "sv_maxclients", &settings.max_clients.to_string());
            set_forced(&mut registry, "sv_mapname", &settings.map_name);
            set_forced(&mut registry, "mapname", &settings.map_name);
            for variable in &settings.cvars {
                set_forced(&mut registry, &variable.name, &variable.value);
            }
            drain_notifications(&mut registry, &options.print);
        }
        let state = Self {
            cvars,
            values: RefCell::new(HashMap::new()),
            userinfo: RefCell::new(HashMap::new()),
            commands: RefCell::new(HashMap::new()),
            now: options.now,
            print: options.print,
        };
        state.refresh_server_info();
        state
    }

    /// Current time in milliseconds.
    #[must_use]
    pub fn now(&self) -> i32 {
        (self.now)()
    }

    /// Refresh the server-info configstring; returns the new value, if any.
    pub fn refresh_server_info(&self) -> Option<String> {
        let value = self.server_info();
        if self.values.borrow().get(&0).is_some_and(|current| *current == value) {
            return None;
        }
        self.values.borrow_mut().insert(0, value.clone());
        Some(value)
    }

    /// Current server-info string.
    #[must_use]
    pub fn server_info(&self) -> String {
        self.cvars
            .borrow_mut()
            .info_string(cvar_flags::SERVER_INFO, None)
            .expect("q3 server info string")
    }

    fn config_index(index: i32) {
        if !(0..Q3_CONFIGSTRINGS).contains(&index) {
            panic!("Q3 configstring outside source range");
        }
    }

    /// Whether a configstring slot is set.
    #[must_use]
    pub fn has_configstring(&self, index: i32) -> bool {
        Self::config_index(index);
        self.values.borrow().contains_key(&index)
    }

    /// Read a configstring (`""` when unset).
    #[must_use]
    pub fn configstring_get(&self, index: i32) -> String {
        Self::config_index(index);
        self.values.borrow().get(&index).cloned().unwrap_or_default()
    }

    /// Write a configstring.
    pub fn configstring_set(&self, index: i32, value: &str) {
        Self::config_index(index);
        self.values.borrow_mut().insert(index, value.to_string());
    }

    /// Print engine text.
    pub fn print(&self, text: &str) {
        (self.print)(text);
    }

    /// Read a client userinfo string.
    #[must_use]
    pub fn get_userinfo(&self, slot: i32) -> Option<String> {
        self.userinfo.borrow().get(&slot).cloned()
    }

    /// Write a client userinfo string.
    pub fn set_userinfo(&self, slot: i32, value: &str) {
        self.userinfo.borrow_mut().insert(slot, value.to_string());
    }

    /// Read a stored client command.
    #[must_use]
    pub fn get_user_command(&self, slot: i32) -> Option<Q3StoredUserCommand> {
        self.commands.borrow().get(&slot).copied()
    }

    /// Store a client command.
    pub fn set_user_command(&self, slot: i32, value: Q3StoredUserCommand) {
        self.commands.borrow_mut().insert(slot, value);
    }

    /// Drop client storage.
    pub fn clear_client(&self, slot: i32) {
        self.userinfo.borrow_mut().remove(&slot);
        self.commands.borrow_mut().remove(&slot);
    }

    /// Capture the save image.
    #[must_use]
    pub fn capture_save_state(&self) -> SaveJson {
        let mut configstrings: Vec<(i32, String)> = self
            .values
            .borrow()
            .iter()
            .map(|(slot, value)| (*slot, value.clone()))
            .collect();
        configstrings.sort_by_key(|(slot, _)| *slot);
        let mut userinfo: Vec<(i32, String)> = self
            .userinfo
            .borrow()
            .iter()
            .map(|(slot, value)| (*slot, value.clone()))
            .collect();
        userinfo.sort_by_key(|(slot, _)| *slot);
        let mut commands: Vec<(i32, Q3StoredUserCommand)> = self
            .commands
            .borrow()
            .iter()
            .map(|(slot, value)| (*slot, *value))
            .collect();
        commands.sort_by_key(|(slot, _)| *slot);
        obj(vec![
            ("cvars", capture_cvars(&self.cvars.borrow())),
            (
                "configstrings",
                arr(configstrings
                    .into_iter()
                    .map(|(slot, value)| obj(vec![("slot", int(i64::from(slot))), ("value", save_str(&value))]))
                    .collect()),
            ),
            (
                "userinfo",
                arr(userinfo
                    .into_iter()
                    .map(|(slot, value)| obj(vec![("slot", int(i64::from(slot))), ("value", save_str(&value))]))
                    .collect()),
            ),
            (
                "commands",
                arr(commands
                    .into_iter()
                    .map(|(slot, value)| {
                        obj(vec![
                            ("slot", int(i64::from(slot))),
                            (
                                "value",
                                obj(vec![
                                    ("serverTime", int(i64::from(value.server_time))),
                                    (
                                        "angles",
                                        arr(value.angles.iter().map(|angle| int(i64::from(*angle))).collect()),
                                    ),
                                    ("forwardmove", int(i64::from(value.forwardmove))),
                                    ("rightmove", int(i64::from(value.rightmove))),
                                    ("upmove", int(i64::from(value.upmove))),
                                    ("buttons", int(i64::from(value.buttons))),
                                    ("weapon", int(i64::from(value.weapon))),
                                ]),
                            ),
                        ])
                    })
                    .collect()),
            ),
        ])
    }

    /// Restore the save image, replacing cvars, configstrings, userinfo, and
    /// stored commands wholesale like the donor.
    pub fn restore_save_state(&self, value: &SaveJson) -> Result<(), WorldError> {
        let reader = SaveReader::at(value, "q3.server");
        let configstrings = read_slots(reader.field("configstrings"), Q3_CONFIGSTRINGS, |entry| entry.string())?;
        let userinfo = read_slots(reader.field("userinfo"), Q3_MAX_CLIENT_SLOTS, |entry| entry.string())?;
        let commands = read_slots(reader.field("commands"), Q3_MAX_CLIENT_SLOTS, read_command)?;
        let cvars = restore_cvars(reader.field("cvars"))?;
        *self.cvars.borrow_mut() = cvars;
        *self.values.borrow_mut() = configstrings;
        *self.userinfo.borrow_mut() = userinfo;
        *self.commands.borrow_mut() = commands;
        Ok(())
    }
}

fn set_forced(registry: &mut CvarRegistry, name: &str, value: &str) {
    registry
        .set(name, value, true)
        .unwrap_or_else(|error| panic!("q3 server cvar {name}: {error}"));
}

fn apply_archive_entry(registry: &mut CvarRegistry, entry: &CvarArchiveEntry) {
    registry
        .set_command_flags(&entry.name, &entry.value, SetCommandKind::Archive)
        .unwrap_or_else(|error| panic!("q3 server archive {}: {error}", entry.name));
}

fn drain_notifications(registry: &mut CvarRegistry, print: &Rc<dyn Fn(&str)>) {
    for line in registry.take_notifications() {
        print(&line);
    }
}

fn capture_cvars(cvars: &CvarRegistry) -> SaveJson {
    let mut snapshots = cvars.snapshots(0);
    snapshots.reverse();
    obj(vec![
        ("dialect", save_str("q3")),
        (
            "variables",
            arr(snapshots
                .into_iter()
                .map(|snapshot| {
                    obj(vec![
                        ("name", save_str(&snapshot.name)),
                        ("value", save_str(&snapshot.value)),
                        ("resetValue", save_str(&snapshot.reset_value)),
                        (
                            "latchedValue",
                            snapshot
                                .latched_value
                                .as_ref()
                                .map_or(SaveJson::Null, |latched| save_str(latched)),
                        ),
                        ("flags", int(i64::from(snapshot.flags))),
                        ("modified", boolean(snapshot.modified)),
                        ("modificationCount", int(i64::from(snapshot.modification_count))),
                    ])
                })
                .collect()),
        ),
    ])
}

fn restore_cvars(reader: SaveReader) -> Result<CvarRegistry, WorldError> {
    if reader.field("dialect").string()? != "q3" {
        return Err(reader.field("dialect").fail("q3 server cvars require the q3 dialect"));
    }
    let mut cvars = CvarRegistry::new(Dialect::Q3);
    let mut seen = HashSet::new();
    for entry in reader.field("variables").list(|cell| {
        Ok::<_, WorldError>((
            cell.field("name").string()?,
            cell.field("value").string()?,
            cell.field("resetValue").string()?,
            cell.field("latchedValue").nullable(|latched| latched.string())?,
            cell.field("flags").integer(0)?,
        ))
    })? {
        let (name, value, reset_value, latched, flags) = entry;
        if !seen.insert(name.clone()) {
            return Err(reader.fail("duplicate cvar"));
        }
        let flags = u32::try_from(flags).map_err(|_| reader.fail("cvar flags out of range"))?;
        cvars
            .register(&name, &reset_value, flags)
            .map_err(|error| reader.fail(&error.to_string()))?;
        cvars
            .set(&name, &value, true)
            .map_err(|error| reader.fail(&error.to_string()))?;
        if let Some(latched) = latched {
            cvars
                .stage(&name, &latched)
                .map_err(|error| reader.fail(&error.to_string()))?;
        }
    }
    Ok(cvars)
}

fn read_slots<T>(
    reader: SaveReader,
    maximum: i32,
    read: impl Fn(SaveReader) -> Result<T, WorldError>,
) -> Result<HashMap<i32, T>, WorldError> {
    let mut result = HashMap::new();
    for entry in
        reader.list(|cell| Ok::<_, WorldError>((cell.field("slot").integer(0)?, read(cell.field("value"))?)))?
    {
        let (slot, value) = entry;
        let slot = i32::try_from(slot).map_err(|_| reader.fail("invalid or duplicate source storage slot"))?;
        if slot >= maximum || result.contains_key(&slot) {
            return Err(reader.fail("invalid or duplicate source storage slot"));
        }
        result.insert(slot, value);
    }
    Ok(result)
}

fn read_command(reader: SaveReader) -> Result<Q3StoredUserCommand, WorldError> {
    let angles = reader.field("angles").list(|value| value.integer(i64::MIN))?;
    if angles.len() != 3 {
        return Err(reader.fail("command requires three angles"));
    }
    let angle = |index: usize| i32::try_from(angles[index]).map_err(|_| reader.fail("command requires three angles"));
    let number = |reader: SaveReader| {
        let value = reader.integer(i64::MIN)?;
        i32::try_from(value).map_err(|_| reader.fail("command field out of range"))
    };
    Ok(Q3StoredUserCommand {
        server_time: number(reader.field("serverTime"))?,
        angles: [angle(0)?, angle(1)?, angle(2)?],
        forwardmove: number(reader.field("forwardmove"))?,
        rightmove: number(reader.field("rightmove"))?,
        upmove: number(reader.field("upmove"))?,
        buttons: number(reader.field("buttons"))?,
        weapon: number(reader.field("weapon"))?,
    })
}

#[cfg(test)]
mod tests {
    use qa_core::cmd::Dialect;
    use qa_core::identity::IdentityOwner;

    use super::super::host::Q3HostSettings;
    use super::*;

    fn settings() -> Q3HostSettings {
        Q3HostSettings {
            game_type: 0,
            single_player: false,
            max_clients: 8,
            map_name: "q3dm1".to_string(),
            source_registry: None,
            source_archive: Vec::new(),
            cvars: Vec::new(),
        }
    }

    fn options(settings: Q3HostSettings) -> Q3ServerStateOptions {
        let owner = IdentityOwner::create("q3-server-test").unwrap();
        Q3ServerStateOptions {
            session: owner.session().clone(),
            settings,
            now: Rc::new(|| 4242),
            print: Rc::new(|_| {}),
            register_server_cvars: Rc::new(|registry, max_clients, map_name| {
                registry.register("sv_maxclients", &max_clients.to_string(), 0).unwrap();
                registry.register("mapname", map_name, 0).unwrap();
            }),
        }
    }

    #[test]
    fn constructor_applies_settings_and_server_info() {
        let state = Q3ServerState::new(options(settings()));
        let registry = state.cvars.borrow();
        assert_eq!(registry.variable_string("sv_maxclients"), "8");
        assert_eq!(registry.variable_string("mapname"), "q3dm1");
        assert_eq!(registry.variable_string("sv_mapname"), "q3dm1");
        drop(registry);
        assert_eq!(state.configstring_get(0), state.server_info());
        assert!(state.refresh_server_info().is_none());
    }

    #[test]
    fn configstring_userinfo_and_command_storage() {
        let state = Q3ServerState::new(options(settings()));
        assert!(!state.has_configstring(7));
        state.configstring_set(7, "hello");
        assert!(state.has_configstring(7));
        assert_eq!(state.configstring_get(7), "hello");
        assert_eq!(state.configstring_get(9), "");
        assert_eq!(state.get_userinfo(0), None);
        state.set_userinfo(0, "name\\x");
        assert_eq!(state.get_userinfo(0).as_deref(), Some("name\\x"));
        let command = Q3StoredUserCommand {
            server_time: 100,
            angles: [1, 2, 3],
            forwardmove: 4,
            rightmove: 5,
            upmove: 6,
            buttons: 7,
            weapon: 8,
        };
        assert_eq!(state.get_user_command(0), None);
        state.set_user_command(0, command);
        assert_eq!(state.get_user_command(0), Some(command));
        state.clear_client(0);
        assert_eq!(state.get_userinfo(0), None);
        assert_eq!(state.get_user_command(0), None);
    }

    #[test]
    #[should_panic(expected = "Q3 configstring outside source range")]
    fn configstring_range_checked() {
        let state = Q3ServerState::new(options(settings()));
        let _ = state.configstring_get(1024);
    }

    #[test]
    fn save_round_trip_restores_tables() {
        let state = Q3ServerState::new(options(settings()));
        state.configstring_set(3, "three");
        state.set_userinfo(1, "u1");
        state.set_user_command(
            1,
            Q3StoredUserCommand {
                server_time: 9,
                angles: [0, 0, 0],
                forwardmove: 1,
                rightmove: 2,
                upmove: 3,
                buttons: 4,
                weapon: 5,
            },
        );
        state.cvars.borrow_mut().set("mapname", "q3dm7", true).unwrap();
        let image = state.capture_save_state();
        let fresh = Q3ServerState::new(options(settings()));
        fresh.restore_save_state(&image).unwrap();
        assert_eq!(fresh.configstring_get(3), "three");
        assert_eq!(fresh.get_userinfo(1).as_deref(), Some("u1"));
        assert_eq!(fresh.get_user_command(1).map(|command| command.server_time), Some(9));
        assert_eq!(fresh.cvars.borrow().variable_string("mapname"), "q3dm7");
        assert_eq!(fresh.cvars.borrow().dialect(), Dialect::Q3);
    }

    #[test]
    fn restore_rejects_bad_slots() {
        let state = Q3ServerState::new(options(settings()));
        let image = state.capture_save_state();
        let bad = obj(vec![
            ("cvars", image.get("cvars").unwrap().clone()),
            (
                "configstrings",
                arr(vec![obj(vec![("slot", int(1024)), ("value", save_str("x"))])]),
            ),
            ("userinfo", arr(Vec::new())),
            ("commands", arr(Vec::new())),
        ]);
        assert!(state.restore_save_state(&bad).is_err());
    }
}
