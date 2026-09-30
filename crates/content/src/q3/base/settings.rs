//! Quake III base: settings.
//!
//! Donor provenance: `src/content/q3/base/settings.ts`.

use crate::value::{arr, boolean, int, num, obj, str as save_str, SaveJson, SaveReader, ValueError};
use qa_core::cvar::{flags as cvar_flags, CvarRegistry, CvarSnapshot};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::shared::definitions::*;
use qa_core::cmd::ascii_fold;

// ---------------------------------------------------------------------------
// settings.ts
// ---------------------------------------------------------------------------

/// Cvar definition (`CvarDefinition`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CvarDefinition {
    /// Name.
    pub name: String,
    /// Default value.
    pub value: String,
    /// Flags.
    pub flags: u32,
    /// Announce changes.
    pub track: bool,
    /// Remap team shaders on change.
    pub team_shader: bool,
}

pub(crate) fn cvar_definition(name: &str, value: &str, flags: u32, track: bool, team_shader: bool) -> CvarDefinition {
    CvarDefinition {
        name: name.to_string(),
        value: value.to_string(),
        flags,
        track,
        team_shader,
    }
}

/// Q3 game cvar definitions (`q3GameCvarDefinitions`).
#[must_use]
pub fn q3_game_cvar_definitions(product: Product) -> Vec<CvarDefinition> {
    let archive = cvar_flags::ARCHIVE;
    let server_info = cvar_flags::SERVER_INFO;
    let user_info = cvar_flags::USER_INFO;
    let latch = cvar_flags::LATCH;
    let read_only = cvar_flags::READ_ONLY;
    let no_restart = cvar_flags::NO_RESTART;
    let system_info = cvar_flags::SYSTEM_INFO;
    let mut definitions = vec![
        cvar_definition("sv_cheats", "", 0, false, false),
        cvar_definition("g_restarted", "0", read_only, false, false),
        cvar_definition("g_gametype", "0", server_info | user_info | latch, false, false),
        cvar_definition("sv_maxclients", "8", server_info | latch | archive, false, false),
        cvar_definition("g_maxGameClients", "0", server_info | latch | archive, false, false),
        cvar_definition("dmflags", "0", server_info | archive, true, false),
        cvar_definition("fraglimit", "20", server_info | archive | no_restart, true, false),
        cvar_definition("timelimit", "0", server_info | archive | no_restart, true, false),
        cvar_definition("capturelimit", "8", server_info | archive | no_restart, true, false),
        cvar_definition("g_synchronousClients", "0", system_info, false, false),
        cvar_definition("g_friendlyFire", "0", archive, true, false),
        cvar_definition("g_teamAutoJoin", "0", archive, false, false),
        cvar_definition("g_teamForceBalance", "0", archive, false, false),
        cvar_definition("g_warmup", "20", archive, true, false),
        cvar_definition("g_doWarmup", "0", 0, true, false),
        cvar_definition("g_log", "games.log", archive, false, false),
        cvar_definition("g_logSync", "0", archive, false, false),
        cvar_definition("g_password", "", user_info, false, false),
        cvar_definition("g_banIPs", "", archive, false, false),
        cvar_definition("g_filterBan", "1", archive, false, false),
        cvar_definition("g_needpass", "0", server_info | read_only, false, false),
        cvar_definition("dedicated", "0", 0, false, false),
        cvar_definition("g_speed", "320", 0, true, false),
        cvar_definition("g_gravity", "800", 0, true, false),
        cvar_definition("g_knockback", "1000", 0, true, false),
        cvar_definition("g_quadfactor", "3", 0, true, false),
        cvar_definition("g_weaponrespawn", "5", 0, true, false),
        cvar_definition("g_weaponTeamRespawn", "30", 0, true, false),
        cvar_definition("g_forcerespawn", "20", 0, true, false),
        cvar_definition("g_inactivity", "0", 0, true, false),
        cvar_definition("g_debugMove", "0", 0, false, false),
        cvar_definition("g_debugDamage", "0", 0, false, false),
        cvar_definition("g_debugAlloc", "0", 0, false, false),
        cvar_definition("g_motd", "", 0, false, false),
        cvar_definition("com_blood", "1", 0, false, false),
        cvar_definition("g_podiumDist", "80", 0, false, false),
        cvar_definition("g_podiumDrop", "70", 0, false, false),
        cvar_definition("g_allowVote", "1", archive, false, false),
        cvar_definition("g_listEntity", "0", 0, false, false),
    ];
    if product == Product::Missionpack {
        definitions.extend([
            cvar_definition("g_obeliskHealth", "2500", 0, false, false),
            cvar_definition("g_obeliskRegenPeriod", "1", 0, false, false),
            cvar_definition("g_obeliskRegenAmount", "15", 0, false, false),
            cvar_definition("g_obeliskRespawnDelay", "10", server_info, false, false),
            cvar_definition("g_cubeTimeout", "30", 0, false, false),
            cvar_definition("g_redteam", "Stroggs", archive | server_info | user_info, true, true),
            cvar_definition("g_blueteam", "Pagans", archive | server_info | user_info, true, true),
            cvar_definition("ui_singlePlayerActive", "", 0, false, false),
            cvar_definition("g_enableDust", "0", server_info, true, false),
            cvar_definition("g_enableBreath", "0", server_info, true, false),
            cvar_definition("g_proxMineTimeout", "20000", 0, false, false),
        ]);
    }
    definitions.extend([
        cvar_definition("g_smoothClients", "1", 0, false, false),
        cvar_definition("pmove_fixed", "0", system_info, false, false),
        cvar_definition("pmove_msec", "8", system_info, false, false),
        cvar_definition("g_rankings", "0", 0, false, false),
        cvar_definition("sv_enableRankings", "0", 0, false, false),
        cvar_definition("sv_rankingsActive", "0", read_only, false, false),
    ]);
    definitions
}

/// Settings host services (`Q3SettingsHost`).
pub trait Q3SettingsHost {
    /// Cvar registry.
    fn cvars(&self) -> Rc<RefCell<CvarRegistry>>;
    /// Send a server command.
    fn send_server_command(&self, client: i32, command: String);
    /// Remap team shaders.
    fn remap_teams(&self);
    /// Format a tracked cvar change (`gameFormat('print "Server: %s
    /// changed to %s\n"', ...)`, game/format.ts).
    fn format_tracked_change(&self, name: &str, value: &str) -> String;
}

/// Capture a module cvar snapshot (`captureModuleCvar`,
/// game/save-module-values.ts).
#[must_use]
pub fn capture_module_cvar(value: &CvarSnapshot) -> SaveJson {
    obj(vec![
        ("name", save_str(&value.name)),
        ("value", save_str(&value.value)),
        ("resetValue", save_str(&value.reset_value)),
        (
            "latchedValue",
            value
                .latched_value
                .as_ref()
                .map_or(SaveJson::Null, |latched| save_str(latched)),
        ),
        ("flags", int(i64::from(value.flags))),
        ("modified", boolean(value.modified)),
        ("modificationCount", int(i64::from(value.modification_count))),
        ("numericValue", num(f64::from(value.numeric_value))),
        ("integerValue", int(i64::from(value.integer_value))),
    ])
}

/// Read a module cvar snapshot (`readModuleCvar`,
/// game/save-module-values.ts).
pub fn read_module_cvar(reader: SaveReader<'_>) -> Result<CvarSnapshot, ValueError> {
    Ok(CvarSnapshot {
        name: reader.field("name").string()?,
        value: reader.field("value").string()?,
        reset_value: reader.field("resetValue").string()?,
        latched_value: reader.field("latchedValue").nullable(|item| item.string())?,
        flags: reader.field("flags").integer(i64::MIN)? as u32,
        modified: reader.field("modified").boolean()?,
        modification_count: reader.field("modificationCount").integer(i64::MIN)? as u32,
        numeric_value: reader.field("numericValue").number()? as f32,
        integer_value: reader.field("integerValue").integer(i64::MIN)? as i32,
    })
}

/// Q3 game settings (`Q3GameSettings`).
///
/// Copies values at the source `G_UpdateCvars` point, independent of
/// changes to the shared cvars.
pub struct Q3GameSettings {
    host: Rc<dyn Q3SettingsHost>,
    product: Product,
    definitions: Vec<CvarDefinition>,
    snapshots: RefCell<HashMap<String, CvarSnapshot>>,
}

impl std::fmt::Debug for Q3GameSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Q3GameSettings")
            .field("product", &self.product)
            .finish()
    }
}

impl Q3GameSettings {
    /// Settings over a host and product.
    #[must_use]
    pub fn new(host: Rc<dyn Q3SettingsHost>, product: Product) -> Self {
        let definitions = q3_game_cvar_definitions(product);
        Self {
            host,
            product,
            definitions,
            snapshots: RefCell::new(HashMap::new()),
        }
    }

    /// Owning product.
    #[must_use]
    pub fn product(&self) -> Product {
        self.product
    }

    /// Cvar definitions.
    #[must_use]
    pub fn definitions(&self) -> &[CvarDefinition] {
        &self.definitions
    }

    /// Capture snapshot save words.
    #[must_use]
    pub fn capture_save_state(&self) -> SaveJson {
        let snapshots = self.snapshots.borrow();
        arr(self
            .definitions
            .iter()
            .filter_map(|definition| snapshots.get(&definition.name))
            .map(capture_module_cvar)
            .collect())
    }

    /// Restore snapshot save words.
    pub fn restore_save_state(&self, value: &SaveJson) -> Result<(), ValueError> {
        let reader = SaveReader::at(value, "q3.settings");
        let snapshots = reader.list(read_module_cvar)?;
        let definitions: HashMap<String, &str> = self
            .definitions
            .iter()
            .map(|definition| (ascii_fold(&definition.name), definition.name.as_str()))
            .collect();
        let mut restored = HashMap::new();
        for snapshot in &snapshots {
            let Some(name) = definitions.get(&ascii_fold(&snapshot.name)) else {
                return Err(reader.fail("invalid settings snapshot names"));
            };
            if restored.contains_key(*name) {
                return Err(reader.fail("invalid settings snapshot names"));
            }
            restored.insert((*name).to_string(), snapshot.clone());
        }
        if restored.len() != self.definitions.len() {
            return Err(reader.fail("invalid settings snapshot names"));
        }
        *self.snapshots.borrow_mut() = restored;
        Ok(())
    }

    /// Register every game cvar.
    ///
    /// # Panics
    ///
    /// Panics when a game cvar cannot be registered.
    pub fn register(&self, build_date: &str) {
        for definition in &self.definitions {
            if definition.name == "g_restarted" {
                let gamename = self.host.cvars().borrow_mut().register(
                    "gamename",
                    "baseq3",
                    cvar_flags::SERVER_INFO | cvar_flags::READ_ONLY,
                );
                if !matches!(gamename, Ok(Some(_))) {
                    panic!("Could not register Q3 game cvar gamename");
                }
                let gamedate = self
                    .host
                    .cvars()
                    .borrow_mut()
                    .register("gamedate", build_date, cvar_flags::READ_ONLY);
                if !matches!(gamedate, Ok(Some(_))) {
                    panic!("Could not register Q3 game cvar gamedate");
                }
            }
            let current =
                self.host
                    .cvars()
                    .borrow_mut()
                    .register(&definition.name, &definition.value, definition.flags);
            let Ok(Some(current)) = current else {
                panic!("Could not register Q3 game cvar {}", definition.name);
            };
            self.snapshots.borrow_mut().insert(definition.name.clone(), current);
        }
    }

    /// Snapshot for a registered cvar.
    ///
    /// # Panics
    ///
    /// Panics when the cvar is not registered.
    #[must_use]
    pub fn snapshot(&self, name: &str) -> CvarSnapshot {
        self.snapshots
            .borrow()
            .get(name)
            .unwrap_or_else(|| panic!("Unregistered Q3 game cvar {name}"))
            .clone()
    }

    /// Integer value for a registered cvar.
    ///
    /// # Panics
    ///
    /// Panics when the cvar is not registered.
    #[must_use]
    pub fn integer(&self, name: &str) -> i32 {
        self.snapshot(name).integer_value
    }

    /// Numeric value for a registered cvar.
    ///
    /// # Panics
    ///
    /// Panics when the cvar is not registered.
    #[must_use]
    pub fn number(&self, name: &str) -> f32 {
        self.snapshot(name).numeric_value
    }

    /// String value for a registered cvar.
    ///
    /// # Panics
    ///
    /// Panics when the cvar is not registered.
    #[must_use]
    pub fn string(&self, name: &str) -> String {
        self.snapshot(name).value.clone()
    }

    /// Copy changed values at the source `G_UpdateCvars` point.
    ///
    /// # Panics
    ///
    /// Panics when a game cvar disappeared from the registry.
    pub fn update(&self) {
        let mut remapped = false;
        for definition in &self.definitions {
            let previous = self.snapshot(&definition.name);
            let Some(current) = self.host.cvars().borrow().get(&definition.name) else {
                panic!("Game cvar disappeared: {}", definition.name);
            };
            self.snapshots
                .borrow_mut()
                .insert(definition.name.clone(), current.clone());
            if previous.modification_count == current.modification_count {
                continue;
            }
            if definition.track {
                self.host
                    .send_server_command(-1, self.host.format_tracked_change(&definition.name, &current.value));
            }
            if definition.team_shader {
                remapped = true;
            }
        }
        if remapped {
            self.host.remap_teams();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::cmd::Dialect;
    use std::cell::Cell;

    struct FakeSettingsHost {
        cvars: Rc<RefCell<CvarRegistry>>,
        commands: RefCell<Vec<(i32, String)>>,
        remapped: Cell<bool>,
    }

    impl FakeSettingsHost {
        fn new() -> Self {
            Self {
                cvars: Rc::new(RefCell::new(CvarRegistry::new(Dialect::Q3))),
                commands: RefCell::new(Vec::new()),
                remapped: Cell::new(false),
            }
        }
    }

    impl Q3SettingsHost for FakeSettingsHost {
        fn cvars(&self) -> Rc<RefCell<CvarRegistry>> {
            self.cvars.clone()
        }

        fn send_server_command(&self, client: i32, command: String) {
            self.commands.borrow_mut().push((client, command));
        }

        fn remap_teams(&self) {
            self.remapped.set(true);
        }

        fn format_tracked_change(&self, name: &str, value: &str) -> String {
            format!("print \"Server: {name} changed to {value}\n\"")
        }
    }

    #[test]
    fn settings_register_update_and_save_round_trip() {
        let host = Rc::new(FakeSettingsHost::new());
        let settings = Q3GameSettings::new(host.clone(), Product::Baseq3);
        assert_eq!(settings.definitions().len(), 45);
        let pack = Q3GameSettings::new(host.clone(), Product::Missionpack);
        assert_eq!(pack.definitions().len(), 56);
        settings.register("2026-09-30");
        assert_eq!(settings.integer("fraglimit"), 20);
        assert_eq!(settings.number("g_speed"), 320.0);
        assert_eq!(settings.string("g_motd"), "");
        host.cvars.borrow_mut().set("fraglimit", "30", true).expect("set");
        settings.update();
        assert_eq!(settings.integer("fraglimit"), 30);
        let commands = host.commands.borrow();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].0, -1);
        assert!(commands[0].1.contains("fraglimit"), "{}", commands[0].1);
        assert!(commands[0].1.contains("30"), "{}", commands[0].1);
        drop(commands);

        let saved = settings.capture_save_state();
        let host2 = Rc::new(FakeSettingsHost::new());
        let restored = Q3GameSettings::new(host2.clone(), Product::Baseq3);
        restored.register("2026-09-30");
        restored.restore_save_state(&saved).expect("restore");
        assert_eq!(restored.integer("fraglimit"), 30);
        assert!(restored.restore_save_state(&arr(Vec::new())).is_err());
    }

    #[test]
    fn settings_remap_team_shaders_on_change() {
        let host = Rc::new(FakeSettingsHost::new());
        let settings = Q3GameSettings::new(host.clone(), Product::Missionpack);
        settings.register("2026-09-30");
        host.cvars.borrow_mut().set("g_redteam", "Rangers", true).expect("set");
        settings.update();
        assert!(host.remapped.get());
    }
}
