//! Quake II server cvar ownership: restore, registration helpers, and live
//! rule bindings.
//!
//! Donor provenance: `/home/buzzkill/Projects/quake-typescript/src/settings/server/q2-owner.ts`
//! (the brief's `settings/q2/tables.ts` does not exist in the donor) and
//! `/home/buzzkill/Projects/quake-typescript/src/settings/server/lmctf-cvars.ts`.
//!
//! The donor wires rules objects to cvars with `defineProperty`, which has
//! no Rust equivalent. Binding pulls cvar values into the rules objects
//! once, installs the `needpass` password hooks, and returns a
//! [`Q2ServerCvarBindings`] guard whose `sync_from_cvars` / `sync_to_cvars`
//! keep both sides aligned at frame boundaries. Same-registry write-back
//! inside `changed` callbacks would re-borrow the registry, so password
//! changes mark the guard dirty and the guard recomputes `needpass` on
//! sync.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

use qa_content::q2::base::player::types::Q2PlayerRules;
use qa_content::q2::multiplayer::ctf::types::Q2CtfRules;
use qa_content::q2::multiplayer::lmctf::types::{create_lmctf_rules, LmctfRules};
use qa_content::q2::rerelease::types::Q2RereleaseOptions;
use qa_core::cmd::Dialect;
use qa_core::cvar::{q2_flags, BindingToken, CvarError, CvarRegistry, CvarSaveState, FnValueBinding, SavedCvarState};

use super::SettingsError;

/// Random-item policy read from rerelease cvars.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q2RandomItemPolicy {
    /// Random items enabled.
    pub enabled: bool,
    /// Mines excluded.
    pub no_mines: bool,
    /// Nukes excluded.
    pub no_nukes: bool,
    /// Spheres excluded.
    pub no_spheres: bool,
}

/// Live rerelease item services over a registry
/// (donor `q2RereleaseItemServices`).
#[derive(Clone, Copy)]
pub struct Q2RereleaseItemServices<'a> {
    cvars: &'a CvarRegistry,
}

impl<'a> Q2RereleaseItemServices<'a> {
    /// Resolve services; `None` outside the rerelease dialect.
    #[must_use]
    pub fn resolve(cvars: &'a CvarRegistry) -> Option<Self> {
        if cvars.dialect() != Dialect::Q2Rerelease {
            return None;
        }
        Some(Self { cvars })
    }

    /// Current random-item policy.
    #[must_use]
    pub fn random_items(&self) -> Q2RandomItemPolicy {
        let flags = q2_source_deathmatch_flags(self.cvars);
        Q2RandomItemPolicy {
            enabled: self.cvars.variable_value("g_dm_random_items") != 0.0,
            no_mines: self.cvars.variable_value("g_no_mines") != 0.0 || (flags & 0x20000) != 0,
            no_nukes: self.cvars.variable_value("g_no_nukes") != 0.0 || (flags & 0x80000) != 0,
            no_spheres: self.cvars.variable_value("g_no_spheres") != 0.0 || (flags & 0x40000) != 0,
        }
    }

    /// Whether quad-fire drops.
    #[must_use]
    pub fn drop_quad_fire(&self) -> bool {
        self.cvars.variable_value("g_dm_no_quadfire_drop") == 0.0
    }
}

/// Deathmatch flags with rerelease toggles folded in
/// (donor `q2SourceDeathmatchFlags`).
#[must_use]
pub fn q2_source_deathmatch_flags(cvars: &CvarRegistry) -> i32 {
    let mut flags = cvars.variable_value("dmflags").trunc() as i32;
    if cvars.dialect() != Dialect::Q2Rerelease {
        return flags;
    }
    for (name, mask, inverted) in [
        ("g_dm_weapons_stay", 4, false),
        ("g_dm_instant_items", 16, false),
        ("g_dm_same_level", 32, false),
        ("g_dm_no_quad_drop", 16384, true),
    ] {
        let enabled = (cvars.variable_value(name) != 0.0) != inverted;
        if enabled {
            flags |= mask;
        } else {
            flags &= !mask;
        }
    }
    flags
}

/// Restore a server save image, preserving live variables the image does
/// not name (donor `restoreQ2ServerCvars`).
pub fn restore_q2_server_cvars(cvars: &mut CvarRegistry, state: &CvarSaveState) -> Result<(), CvarError> {
    let mut validated = CvarRegistry::with_session(cvars.dialect(), cvars.session().cloned());
    validated.restore_save_state(state)?;
    let saved = validated.capture_save_state()?;
    let names: HashSet<&String> = saved.order.iter().collect();
    let missing: Vec<SavedCvarState> = cvars
        .snapshots(0)
        .into_iter()
        .filter(|variable| !names.contains(&variable.name))
        .map(|variable| SavedCvarState {
            name: variable.name,
            value: variable.value,
            reset_value: variable.reset_value,
            latched_value: variable.latched_value,
            flags: variable.flags,
            modified: variable.modified,
            modification_count: variable.modification_count,
            numeric_value: variable.numeric_value,
            integer_value: variable.integer_value,
        })
        .collect();
    let mut merged = saved;
    let mut order: Vec<String> = missing.iter().map(|variable| variable.name.clone()).collect();
    order.extend(merged.order);
    merged.order = order;
    merged.variables.extend(missing.into_iter().map(Some));
    cvars.restore_save_state(&merged)
}

/// Match-specific rules bound alongside the player rules.
pub enum Q2ServerMatchRules<'a> {
    /// Standard match.
    Standard,
    /// CTF match with capture-limit rules.
    Ctf(&'a mut Q2CtfRules),
    /// LMCTF match with console rules.
    Lmctf(&'a mut LmctfRules),
}

fn pull_player_numbers(cvars: &CvarRegistry, rules: &mut Q2PlayerRules) {
    rules.max_spectators = cvars.variable_value("maxspectators").trunc() as i32;
    rules.flood_messages = cvars.variable_value("flood_msgs").trunc() as i32;
    rules.flood_seconds = f64::from(cvars.variable_value("flood_persecond"));
    rules.flood_wait_seconds = f64::from(cvars.variable_value("flood_waitdelay"));
    rules.roll_speed = f64::from(cvars.variable_value("sv_rollspeed"));
    rules.roll_angle = f64::from(cvars.variable_value("sv_rollangle"));
    rules.run_pitch = f64::from(cvars.variable_value("run_pitch"));
    rules.run_roll = f64::from(cvars.variable_value("run_roll"));
    rules.bob_up = f64::from(cvars.variable_value("bob_up"));
    rules.bob_pitch = f64::from(cvars.variable_value("bob_pitch"));
    rules.bob_roll = f64::from(cvars.variable_value("bob_roll"));
    rules.password = cvars.variable_string("password");
    rules.spectator_password = cvars.variable_string("spectator_password");
    rules.cheats = cvars.variable_value("cheats") != 0.0;
    rules.gun_offset.x = cvars.variable_value("gun_x");
    rules.gun_offset.y = cvars.variable_value("gun_y");
    rules.gun_offset.z = cvars.variable_value("gun_z");
}

fn pull_map_list(cvars: &CvarRegistry, rules: &mut Q2PlayerRules, rerelease: bool) {
    let list = if rerelease { "g_map_list" } else { "sv_maplist" };
    let text = cvars.variable_string(list);
    rules.map_list = if rerelease {
        text.split_whitespace().map(str::to_string).collect()
    } else {
        text.split(|c: char| c.is_whitespace() || c == ',')
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .collect()
    };
}

fn pull_rerelease_options(cvars: &CvarRegistry, rerelease: &mut Q2RereleaseOptions) {
    rerelease.coop_squad_respawn = cvars.variable_value("g_coop_squad_respawn") != 0.0;
    rerelease.coop_instanced_items = cvars.variable_value("g_coop_instanced_items") != 0.0;
    rerelease.coop_lives = cvars.variable_value("g_coop_enable_lives") != 0.0;
    rerelease.coop_player_collision = cvars.variable_value("g_coop_player_collision") != 0.0;
    rerelease.deathmatch_force_respawn = cvars.variable_value("g_dm_force_respawn") != 0.0;
    rerelease.deathmatch_no_fall_damage = cvars.variable_value("g_dm_no_fall_damage") != 0.0;
    rerelease.deathmatch_spawn_farthest = cvars.variable_value("g_dm_spawn_farthest") != 0.0;
    rerelease.deathmatch_allow_exit = cvars.variable_value("g_dm_allow_exit") != 0.0;
    rerelease.coop_num_lives = cvars.variable_value("g_coop_num_lives").trunc() as i32;
    rerelease.deathmatch_force_respawn_time = f64::from(cvars.variable_value("g_dm_force_respawn_time"));
}

fn push_player_numbers(cvars: &mut CvarRegistry, rules: &Q2PlayerRules) -> Result<(), CvarError> {
    for (name, value) in [
        ("maxspectators", rules.max_spectators.to_string()),
        ("flood_msgs", rules.flood_messages.to_string()),
        ("flood_persecond", rules.flood_seconds.to_string()),
        ("flood_waitdelay", rules.flood_wait_seconds.to_string()),
        ("sv_rollspeed", rules.roll_speed.to_string()),
        ("sv_rollangle", rules.roll_angle.to_string()),
        ("run_pitch", rules.run_pitch.to_string()),
        ("run_roll", rules.run_roll.to_string()),
        ("bob_up", rules.bob_up.to_string()),
        ("bob_pitch", rules.bob_pitch.to_string()),
        ("bob_roll", rules.bob_roll.to_string()),
        ("password", rules.password.clone()),
        ("spectator_password", rules.spectator_password.clone()),
    ] {
        cvars.set(name, &value, true)?;
    }
    cvars.set("cheats", if rules.cheats { "1" } else { "0" }, true)?;
    cvars.set("gun_x", &rules.gun_offset.x.to_string(), true)?;
    cvars.set("gun_y", &rules.gun_offset.y.to_string(), true)?;
    cvars.set("gun_z", &rules.gun_offset.z.to_string(), true)?;
    Ok(())
}

fn push_map_list(cvars: &mut CvarRegistry, rules: &Q2PlayerRules, rerelease: bool) -> Result<(), CvarError> {
    let list = if rerelease { "g_map_list" } else { "sv_maplist" };
    cvars.set(list, &rules.map_list.join(" "), true)?;
    if rerelease {
        cvars.set(
            "g_map_list_shuffle",
            if rules.map_list_shuffle { "1" } else { "0" },
            true,
        )?;
    }
    Ok(())
}

fn push_rerelease_options(cvars: &mut CvarRegistry, rerelease: &Q2RereleaseOptions) -> Result<(), CvarError> {
    for (name, value) in [
        ("g_coop_squad_respawn", rerelease.coop_squad_respawn),
        ("g_coop_instanced_items", rerelease.coop_instanced_items),
        ("g_coop_enable_lives", rerelease.coop_lives),
        ("g_coop_player_collision", rerelease.coop_player_collision),
        ("g_dm_force_respawn", rerelease.deathmatch_force_respawn),
        ("g_dm_no_fall_damage", rerelease.deathmatch_no_fall_damage),
        ("g_dm_spawn_farthest", rerelease.deathmatch_spawn_farthest),
        ("g_dm_allow_exit", rerelease.deathmatch_allow_exit),
    ] {
        cvars.set(name, if value { "1" } else { "0" }, true)?;
    }
    cvars.set("g_coop_num_lives", &rerelease.coop_num_lives.to_string(), true)?;
    cvars.set(
        "g_dm_force_respawn_time",
        &rerelease.deathmatch_force_respawn_time.to_string(),
        true,
    )?;
    Ok(())
}

fn update_needpass(cvars: &mut CvarRegistry) -> Result<(), CvarError> {
    let required = |name: &str| {
        let value = cvars.variable_string(name);
        !value.is_empty() && value.to_lowercase() != "none"
    };
    let bits = (i32::from(required("password"))) | (i32::from(required("spectator_password")) << 1);
    cvars.set("needpass", &bits.to_string(), true)?;
    Ok(())
}

fn pull_lmctf(cvars: &CvarRegistry, rules: &mut LmctfRules) {
    rules.ctf_flags = cvars.variable_value("ctfflags").trunc() as i32;
    rules.ref_flags = cvars.variable_value("refset").trunc() as i32;
    rules.runes = cvars.variable_value("runes").trunc() as i32;
    rules.skin_set = cvars.variable_value("skinset").trunc() as i32;
    rules.disabled_weapons = cvars.variable_value("disabled_weps").trunc() as i32;
    rules.time_limit_minutes = f64::from(cvars.variable_value("timelimit"));
    rules.frag_limit = cvars.variable_value("fraglimit").trunc() as i32;
    rules.countdown_seconds = f64::from(cvars.variable_value("countdown_time"));
    rules.flag_init = cvars.variable_value("flag_init") != 0.0;
    rules.fast_switch = cvars.variable_value("fastswitch") != 0.0;
    rules.auto_lock = cvars.variable_value("autolock").trunc() != 0.0;
    rules.ref_password = cvars.variable_string("refpassword");
    rules.rcon_password = cvars.variable_string("rcon_password");
}

fn push_lmctf(cvars: &mut CvarRegistry, rules: &LmctfRules) -> Result<(), CvarError> {
    for (name, value) in [
        ("ctfflags", rules.ctf_flags.to_string()),
        ("refset", rules.ref_flags.to_string()),
        ("runes", rules.runes.to_string()),
        ("skinset", rules.skin_set.to_string()),
        ("disabled_weps", rules.disabled_weapons.to_string()),
        ("timelimit", rules.time_limit_minutes.to_string()),
        ("fraglimit", rules.frag_limit.to_string()),
        ("countdown_time", rules.countdown_seconds.to_string()),
        ("flag_init", if rules.flag_init { "1" } else { "0" }.to_string()),
        ("fastswitch", if rules.fast_switch { "1" } else { "0" }.to_string()),
        ("autolock", if rules.auto_lock { "1" } else { "0" }.to_string()),
        ("refpassword", rules.ref_password.clone()),
        ("rcon_password", rules.rcon_password.clone()),
    ] {
        cvars.set(name, &value, false)?;
    }
    Ok(())
}

/// LMCTF console cvar names plus the map-list file
/// (donor `LMCTF_CONSOLE_NAMES`).
pub const LMCTF_CONSOLE_NAMES: [&str; 14] = [
    "ctfflags",
    "refset",
    "runes",
    "skinset",
    "disabled_weps",
    "timelimit",
    "fraglimit",
    "countdown_time",
    "flag_init",
    "fastswitch",
    "autolock",
    "refpassword",
    "rcon_password",
    "maplist_file",
];

/// Register LMCTF console cvars, seeding absent ones from the rules
/// (donor `bindLmctfConsoleRules` registration half).
pub fn register_lmctf_console_rules(cvars: &mut CvarRegistry, rules: &LmctfRules) -> Result<(), CvarError> {
    let table: [(&str, String, u32); 13] = [
        ("ctfflags", rules.ctf_flags.to_string(), q2_flags::SERVER_INFO),
        ("refset", rules.ref_flags.to_string(), q2_flags::SERVER_INFO),
        ("runes", rules.runes.to_string(), q2_flags::SERVER_INFO),
        ("skinset", rules.skin_set.to_string(), q2_flags::SERVER_INFO),
        ("disabled_weps", rules.disabled_weapons.to_string(), 0),
        ("timelimit", rules.time_limit_minutes.to_string(), q2_flags::SERVER_INFO),
        ("fraglimit", rules.frag_limit.to_string(), q2_flags::SERVER_INFO),
        ("countdown_time", rules.countdown_seconds.to_string(), 0),
        ("flag_init", if rules.flag_init { "1" } else { "0" }.to_string(), 0),
        ("fastswitch", if rules.fast_switch { "1" } else { "0" }.to_string(), 0),
        ("autolock", if rules.auto_lock { "1" } else { "0" }.to_string(), 0),
        ("refpassword", rules.ref_password.clone(), 0),
        ("rcon_password", rules.rcon_password.clone(), 0),
    ];
    let defaults = create_lmctf_rules();
    let default_text = |name: &str| -> String {
        match name {
            "ctfflags" => defaults.ctf_flags.to_string(),
            "refset" => defaults.ref_flags.to_string(),
            "runes" => defaults.runes.to_string(),
            "skinset" => defaults.skin_set.to_string(),
            "disabled_weps" => defaults.disabled_weapons.to_string(),
            "timelimit" => defaults.time_limit_minutes.to_string(),
            "fraglimit" => defaults.frag_limit.to_string(),
            "countdown_time" => defaults.countdown_seconds.to_string(),
            "flag_init" => "0".to_string(),
            "fastswitch" => "0".to_string(),
            "autolock" => "0".to_string(),
            "refpassword" | "rcon_password" => String::new(),
            _ => String::new(),
        }
    };
    for (name, initial, flags) in table {
        let present = cvars.get(name).is_some();
        cvars.register(name, &default_text(name), flags)?;
        if !present && initial != default_text(name) {
            cvars.set(name, &initial, false)?;
        }
    }
    cvars.register("maplist_file", "maplist.txt", 0)?;
    Ok(())
}

/// Live bindings between server cvars and Q2 rules objects. The guard
/// releases its password hooks on [`Q2ServerCvarBindings::close`].
pub struct Q2ServerCvarBindings {
    cvars: Rc<RefCell<CvarRegistry>>,
    tokens: Vec<BindingToken>,
    passwords_dirty: Rc<Cell<bool>>,
    rerelease: bool,
}

impl Q2ServerCvarBindings {
    /// Pull cvar values into the rules objects.
    pub fn sync_from_cvars(
        &self,
        rules: &mut Q2PlayerRules,
        rerelease: Option<&mut Q2RereleaseOptions>,
        match_rules: Q2ServerMatchRules,
    ) -> Result<(), CvarError> {
        let cvars = self.cvars.borrow();
        pull_player_numbers(&cvars, rules);
        pull_map_list(&cvars, rules, self.rerelease);
        if let Some(rerelease) = rerelease {
            pull_rerelease_options(&cvars, rerelease);
        }
        match match_rules {
            Q2ServerMatchRules::Standard => {
                rules.time_limit_minutes = cvars.variable_value("timelimit").trunc() as i32;
                rules.frag_limit = cvars.variable_value("fraglimit").trunc() as i32;
            }
            Q2ServerMatchRules::Ctf(ctf) => {
                rules.time_limit_minutes = cvars.variable_value("timelimit").trunc() as i32;
                rules.frag_limit = cvars.variable_value("fraglimit").trunc() as i32;
                ctf.capture_limit = cvars.variable_value("capturelimit").trunc() as i32;
            }
            Q2ServerMatchRules::Lmctf(lmctf) => pull_lmctf(&cvars, lmctf),
        }
        drop(cvars);
        self.sync_passwords()
    }

    /// Push rules values into the cvars.
    pub fn sync_to_cvars(
        &self,
        rules: &Q2PlayerRules,
        rerelease: Option<&Q2RereleaseOptions>,
        match_rules: Q2ServerMatchRules,
    ) -> Result<(), CvarError> {
        let mut cvars = self.cvars.borrow_mut();
        push_player_numbers(&mut cvars, rules)?;
        push_map_list(&mut cvars, rules, self.rerelease)?;
        if let Some(rerelease) = rerelease {
            push_rerelease_options(&mut cvars, rerelease)?;
        }
        match match_rules {
            Q2ServerMatchRules::Standard => {
                cvars.set("timelimit", &rules.time_limit_minutes.to_string(), false)?;
                cvars.set("fraglimit", &rules.frag_limit.to_string(), false)?;
            }
            Q2ServerMatchRules::Ctf(ctf) => {
                cvars.set("timelimit", &rules.time_limit_minutes.to_string(), false)?;
                cvars.set("fraglimit", &rules.frag_limit.to_string(), false)?;
                cvars.set("capturelimit", &ctf.capture_limit.to_string(), false)?;
            }
            Q2ServerMatchRules::Lmctf(lmctf) => {
                push_lmctf(&mut cvars, lmctf)?;
            }
        }
        update_needpass(&mut cvars)
    }

    /// Recompute `needpass` when a password changed.
    pub fn sync_passwords(&self) -> Result<(), CvarError> {
        if self.passwords_dirty.get() {
            self.passwords_dirty.set(false);
            update_needpass(&mut self.cvars.borrow_mut())?;
        }
        Ok(())
    }

    /// Release the password hooks.
    pub fn close(&mut self) {
        for token in self.tokens.drain(..) {
            self.cvars.borrow_mut().release_value_binding(token);
        }
    }
}

/// Bind player cvars to rules objects (donor `bindQ2PlayerCvars`). Pulls
/// current values once, installs the `needpass` hooks, and returns the
/// live-sync guard.
pub fn bind_q2_player_cvars(
    cvars: Rc<RefCell<CvarRegistry>>,
    rules: &mut Q2PlayerRules,
    rerelease: Option<&mut Q2RereleaseOptions>,
) -> Result<Q2ServerCvarBindings, SettingsError> {
    let passwords_dirty = Rc::new(Cell::new(false));
    let mut tokens = Vec::new();
    for name in ["password", "spectator_password"] {
        let dirty = Rc::clone(&passwords_dirty);
        tokens.push(cvars.borrow_mut().bind_value(
            name,
            Box::new(FnValueBinding::new(
                |_| None,
                move |_| {
                    dirty.set(true);
                },
            )),
        )?);
    }
    update_needpass(&mut cvars.borrow_mut())?;
    let has_rerelease = rerelease.is_some();
    let bindings = Q2ServerCvarBindings {
        cvars,
        tokens,
        passwords_dirty,
        rerelease: has_rerelease,
    };
    bindings.sync_passwords()?;
    {
        let guard = &bindings;
        let cvars = guard.cvars.borrow();
        pull_player_numbers(&cvars, rules);
        pull_map_list(&cvars, rules, guard.rerelease);
        if let Some(rerelease) = rerelease {
            pull_rerelease_options(&cvars, rerelease);
        }
    }
    Ok(bindings)
}

/// Bind server cvars to a product's rules (donor `bindQ2ServerCvars`).
pub fn bind_q2_server_cvars(
    cvars: Rc<RefCell<CvarRegistry>>,
    rules: &mut Q2PlayerRules,
    rerelease: Option<&mut Q2RereleaseOptions>,
    match_rules: Q2ServerMatchRules,
) -> Result<Q2ServerCvarBindings, SettingsError> {
    let bindings = bind_q2_player_cvars(Rc::clone(&cvars), rules, rerelease)?;
    match match_rules {
        Q2ServerMatchRules::Standard => {
            let borrowed = cvars.borrow();
            rules.time_limit_minutes = borrowed.variable_value("timelimit").trunc() as i32;
            rules.frag_limit = borrowed.variable_value("fraglimit").trunc() as i32;
        }
        Q2ServerMatchRules::Ctf(ctf) => {
            let borrowed = cvars.borrow();
            rules.time_limit_minutes = borrowed.variable_value("timelimit").trunc() as i32;
            rules.frag_limit = borrowed.variable_value("fraglimit").trunc() as i32;
            ctf.capture_limit = borrowed.variable_value("capturelimit").trunc() as i32;
        }
        Q2ServerMatchRules::Lmctf(lmctf) => {
            register_lmctf_console_rules(&mut cvars.borrow_mut(), lmctf)?;
            pull_lmctf(&cvars.borrow(), lmctf);
        }
    }
    bindings.sync_passwords()?;
    Ok(bindings)
}

/// Bind LMCTF console rules (donor `bindLmctfConsoleRules`).
pub fn bind_lmctf_console_rules(cvars: &mut CvarRegistry, rules: &mut LmctfRules) -> Result<(), SettingsError> {
    register_lmctf_console_rules(cvars, rules)?;
    pull_lmctf(cvars, rules);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_content::q2::multiplayer::ctf::types::create_q2_ctf_rules;

    fn registry() -> Rc<RefCell<CvarRegistry>> {
        Rc::new(RefCell::new(CvarRegistry::new(Dialect::Q2Classic)))
    }

    fn registered() -> Rc<RefCell<CvarRegistry>> {
        let cvars = registry();
        super::super::server::register_q2_server_cvars(&mut cvars.borrow_mut(), "q2:dm").unwrap();
        cvars
    }

    #[test]
    fn deathmatch_flags_fold_rerelease_toggles() {
        let cvars = registered();
        cvars.borrow_mut().set("dmflags", "64", false).unwrap();
        assert_eq!(q2_source_deathmatch_flags(&cvars.borrow()), 64);
        let mut rerelease = CvarRegistry::new(Dialect::Q2Rerelease);
        super::super::server::register_q2_server_cvars(&mut rerelease, "q2:dm").unwrap();
        rerelease.set("g_dm_weapons_stay", "1", false).unwrap();
        rerelease.set("g_dm_no_quad_drop", "1", false).unwrap();
        // weapons-stay (4) plus the default instant-items (16); no-quad-drop
        // is inverted, so setting it clears 16384.
        assert_eq!(q2_source_deathmatch_flags(&rerelease), 20);
        assert!(Q2RereleaseItemServices::resolve(&rerelease).unwrap().drop_quad_fire());
        assert!(Q2RereleaseItemServices::resolve(&cvars.borrow()).is_none());
    }

    #[test]
    fn player_bindings_sync_both_directions() {
        let cvars = registered();
        cvars.borrow_mut().set("maxspectators", "8", false).unwrap();
        cvars.borrow_mut().set("password", "secret", false).unwrap();
        let mut rules = Q2PlayerRules::default();
        let bindings = bind_q2_player_cvars(Rc::clone(&cvars), &mut rules, None).unwrap();
        assert_eq!(rules.max_spectators, 8);
        assert_eq!(rules.password, "secret");
        assert_eq!(cvars.borrow().variable_string("needpass"), "1");

        cvars.borrow_mut().set("sv_rollspeed", "300", false).unwrap();
        bindings
            .sync_from_cvars(&mut rules, None, Q2ServerMatchRules::Standard)
            .unwrap();
        assert_eq!(rules.roll_speed, 300.0);

        rules.frag_limit = 25;
        bindings
            .sync_to_cvars(&rules, None, Q2ServerMatchRules::Standard)
            .unwrap();
        assert_eq!(cvars.borrow().variable_string("fraglimit"), "25");
    }

    #[test]
    fn password_changes_recompute_needpass_on_sync() {
        let cvars = registered();
        let mut rules = Q2PlayerRules::default();
        let bindings = bind_q2_player_cvars(Rc::clone(&cvars), &mut rules, None).unwrap();
        assert_eq!(cvars.borrow().variable_string("needpass"), "0");
        cvars.borrow_mut().set("spectator_password", "spec", false).unwrap();
        bindings.sync_passwords().unwrap();
        assert_eq!(cvars.borrow().variable_string("needpass"), "2");
        cvars.borrow_mut().set("password", "none", false).unwrap();
        bindings.sync_passwords().unwrap();
        assert_eq!(cvars.borrow().variable_string("needpass"), "2");
    }

    #[test]
    fn map_list_splits_on_commas_for_classic() {
        let cvars = registered();
        cvars
            .borrow_mut()
            .set("sv_maplist", "q2dm1, q2dm2  q2dm3,,", false)
            .unwrap();
        let mut rules = Q2PlayerRules::default();
        bind_q2_player_cvars(Rc::clone(&cvars), &mut rules, None).unwrap();
        assert_eq!(
            rules.map_list,
            vec!["q2dm1".to_string(), "q2dm2".to_string(), "q2dm3".to_string()]
        );
    }

    #[test]
    fn ctf_and_lmctf_match_rules_bind() {
        let cvars = registered();
        let mut rules = Q2PlayerRules::default();
        let mut ctf = create_q2_ctf_rules();
        cvars
            .borrow_mut()
            .register("capturelimit", "8", q2_flags::SERVER_INFO)
            .unwrap();
        let bindings = bind_q2_server_cvars(Rc::clone(&cvars), &mut rules, None, Q2ServerMatchRules::Standard).unwrap();
        bindings
            .sync_from_cvars(&mut rules, None, Q2ServerMatchRules::Ctf(&mut ctf))
            .unwrap();
        assert_eq!(ctf.capture_limit, 8);

        let mut lmctf = create_lmctf_rules();
        lmctf.runes = 7;
        bind_lmctf_console_rules(&mut cvars.borrow_mut(), &mut lmctf).unwrap();
        assert_eq!(cvars.borrow().variable_string("runes"), "7");
        assert_eq!(cvars.borrow().variable_string("maplist_file"), "maplist.txt");
        assert_eq!(LMCTF_CONSOLE_NAMES.len(), 14);
    }

    #[test]
    fn restore_preserves_unnamed_live_variables() {
        let cvars = registered();
        cvars.borrow_mut().set("fraglimit", "20", false).unwrap();
        cvars.borrow_mut().register("live_extra", "1", 0).unwrap();
        let image = cvars.borrow().capture_save_state().unwrap();
        cvars.borrow_mut().set("fraglimit", "30", false).unwrap();
        restore_q2_server_cvars(&mut cvars.borrow_mut(), &image).unwrap();
        assert_eq!(cvars.borrow().variable_string("fraglimit"), "20");
        assert_eq!(cvars.borrow().variable_string("live_extra"), "1");
    }
}
