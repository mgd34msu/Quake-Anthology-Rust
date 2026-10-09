use qa_console::{
    catalog::Scope,
    cvars::{Cvars, WriteError},
    cvars_generated::{BINDINGS, CONVERSIONS, DEFINITIONS, OPERANDS},
    views::{Context, Role, RuleSetId},
};

fn context(source: RuleSetId) -> Context {
    Context {
        source,
        ..Context::default()
    }
}
fn write(cvars: &mut Cvars, name: &str, value: &str, context: Context) {
    cvars
        .write(cvars.bind(name, context).unwrap(), value)
        .unwrap();
}
fn read(cvars: &Cvars, name: &str, context: Context) -> String {
    cvars
        .read(cvars.bind(name, context).unwrap())
        .unwrap()
        .as_str()
        .to_owned()
}

#[test]
fn all_sources_share_canonical_values_with_native_units_and_stable_handles() {
    let mut cvars = Cvars::new().unwrap();
    let sensitivity = cvars.find("sensitivity").unwrap();
    for source in RuleSetId::ALL {
        let context = context(source);
        cvars.select_context(context);
        assert_eq!(sensitivity, cvars.find("SENSITIVITY").unwrap());
        assert_eq!(
            cvars.value(sensitivity),
            if source == RuleSetId::Quake3 {
                5.0
            } else {
                3.0
            }
        );
        write(&mut cvars, "fov", "110", context);
        write(&mut cvars, "viewsize", "80", context);
        write(&mut cvars, "gamma", "0.8", context);
        assert_eq!(read(&cvars, "cg_fov", context), "110");
        assert_eq!(read(&cvars, "cg_viewsize", context), "80");
        assert_eq!(read(&cvars, "r_gamma", context), "1.250000");
        assert_eq!(read(&cvars, "vid_gamma", context), "0.800000");
        assert_eq!(read(&cvars, "gamma", context), "0.800000");
    }
    cvars.set_text(sensitivity, "4.25").unwrap();
    cvars.select_context(context(RuleSetId::Quake));
    assert_eq!(cvars.value(sensitivity), 4.25);
    cvars.reset(sensitivity);
    assert_eq!(cvars.value(sensitivity), 3.0);
    cvars.select_context(context(RuleSetId::Quake3));
    assert_eq!(cvars.value(sensitivity), 5.0);
    let unresolved = cvars.find("net_qport").unwrap();
    assert!(!cvars.default_available(unresolved, RuleSetId::Quake3));
}

#[test]
fn native_alias_details_survive_only_while_their_operands_stay_unchanged() {
    let mut cvars = Cvars::new().unwrap();
    let q2 = context(RuleSetId::Quake2);
    write(&mut cvars, "cl_gun", "3", q2);
    assert_eq!(read(&cvars, "cg_drawGun", q2), "1.000000");
    assert_eq!(read(&cvars, "cl_gun", q2), "3");
    write(&mut cvars, "cg_drawGun", "1.000000", q2);
    assert_eq!(read(&cvars, "cl_gun", q2), "1.000000");
    let q1 = context(RuleSetId::Quake);
    write(&mut cvars, "_cl_color", "18", q1);
    assert_eq!(read(&cvars, "_cl_color", q1), "18");
    write(&mut cvars, "color2", "1", q1);
    // Shirt row 1 and pants row 4 use the owner's reverse colour table.
    assert_eq!(read(&cvars, "_cl_color", q1), "196.000000");
}

#[test]
fn teamplay_policy_updates_both_operands_and_latches_the_pair_together() {
    let binding = BINDINGS.iter().find(|b| b.name == "teamplay").unwrap();
    let conversion = &CONVERSIONS[binding.conversions[RuleSetId::Quake as usize] as usize];
    assert_eq!(conversion.operands.len(), 1);
    assert_eq!(
        DEFINITIONS[OPERANDS[conversion.operands.start].row as usize].name,
        "g_friendlyFire"
    );
    let mut cvars = Cvars::new().unwrap();
    let q1 = context(RuleSetId::Quake);
    for (mode, friendly) in [("1", "0.000000"), ("2", "1.000000")] {
        write(&mut cvars, "teamplay", mode, q1);
        assert_eq!(read(&cvars, "g_gametype", q1), "3.000000");
        assert_eq!(read(&cvars, "g_friendlyFire", q1), friendly);
        assert_eq!(read(&cvars, "teamplay", q1), mode);
    }
    cvars.server_active = true;
    write(&mut cvars, "teamplay", "1", q1);
    assert_eq!(read(&cvars, "teamplay", q1), "2");
    assert_eq!(read(&cvars, "g_friendlyFire", q1), "1.000000");
    cvars.apply_latches().unwrap();
    assert_eq!(read(&cvars, "teamplay", q1), "1");
    assert_eq!(read(&cvars, "g_friendlyFire", q1), "0.000000");
    write(&mut cvars, "teamplay", "2", q1);
    write(&mut cvars, "g_gametype", "9", q1);
    cvars.apply_latches().unwrap();
    assert_eq!(read(&cvars, "g_gametype", q1), "9");
    assert_eq!(read(&cvars, "g_friendlyFire", q1), "0.000000");
}

#[test]
fn scoped_names_and_native_flags_protect_only_the_requested_boundary() {
    let mut cvars = Cvars::new().unwrap();
    let client = context(RuleSetId::QuakeWorld);
    let server = Context {
        side: Scope::Server,
        ..client
    };
    assert_ne!(
        cvars.bind("password", client).unwrap().canonical(),
        cvars.bind("password", server).unwrap().canonical()
    );
    write(&mut cvars, "password", "client-secret", client);
    write(&mut cvars, "password", "server-secret", server);
    assert_eq!(read(&cvars, "password", client), "client-secret");
    assert_eq!(read(&cvars, "password", server), "server-secret");
    let q3 = context(RuleSetId::Quake3);
    assert_eq!(
        cvars.write(cvars.bind("name", q3).unwrap(), "bad;info"),
        Err(WriteError::InvalidInfo)
    );
    assert_eq!(
        cvars.write(cvars.bind("g_needpass", q3).unwrap(), "1"),
        Err(WriteError::ReadOnly)
    );
    assert_eq!(
        cvars.write(cvars.bind("r_fullbright", q3).unwrap(), "1"),
        Err(WriteError::Cheats)
    );
    cvars.initialized = true;
    assert_eq!(
        cvars.write(cvars.bind("fs_basepath", q3).unwrap(), "data"),
        Err(WriteError::InitOnly)
    );
    let q2 = context(RuleSetId::Quake2);
    assert!(
        cvars
            .write(cvars.bind("game", q2).unwrap(), "rogue")
            .is_ok()
    );
    assert!(matches!(
        cvars.write(cvars.bind("gamedir", q2).unwrap(), "rogue"),
        Err(WriteError::InitOnly | WriteError::ReadOnly)
    ));
    let game_role = Context {
        role: Role::Game,
        ..q3
    };
    assert!(cvars.bind("fov", game_role).is_some());
}

#[test]
fn interned_autoswitch_alias_keeps_text_details_and_numeric_native_modes() {
    let mut cvars = Cvars::new().unwrap();
    for source in RuleSetId::ALL {
        let context = context(source);
        let canonical = cvars.bind("CG_AUTOSWITCH", context).unwrap();
        for (text, expected) in [("NEVER", 0.0), ("new", 1.0), ("always", 1.0)] {
            write(&mut cvars, "QTS_WEAPON_AUTOSWITCH", text, context);
            assert_eq!(cvars.numeric(canonical).unwrap(), expected);
            assert_eq!(read(&cvars, "qts_weapon_autoswitch", context), text);
        }
        write(&mut cvars, "cg_autoswitch", "0", context);
        assert_eq!(read(&cvars, "qts_weapon_autoswitch", context), "never");
        write(&mut cvars, "autoswitch", "3", context);
        assert_eq!(cvars.numeric(canonical).unwrap(), 0.0);
        write(&mut cvars, "autoswitch", "2", context);
        assert_eq!(cvars.numeric(canonical).unwrap(), 1.0);
    }
}
