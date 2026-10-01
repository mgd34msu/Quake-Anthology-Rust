//! QuakeWorld engine console variables.
//!
//! Provenance: `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/simulation/quakeworld-cvars.ts`.

use qa_core::cvar::{flags, CvarError, CvarRegistry};

/// Register the engine cvars the QuakeWorld source reads, without overriding
/// values the session already registered.
pub fn register_quake_world_engine_cvars(cvars: &mut CvarRegistry) -> Result<(), CvarError> {
    for (name, value) in [
        ("sv_phs", "1"),
        ("sv_stopspeed", "100"),
        ("sv_spectatormaxspeed", "500"),
        ("sv_accelerate", "10"),
        ("sv_airaccelerate", "0.7"),
        ("sv_wateraccelerate", "10"),
        ("sv_friction", "4"),
        ("sv_waterfriction", "4"),
        ("password", ""),
        ("spectator_password", ""),
        ("sv_highchars", "1"),
    ] {
        if cvars.get(name).is_none() {
            cvars.register(name, value, 0)?;
        }
    }
    cvars.register("maxspectators", "8", flags::SERVER_INFO)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use qa_core::cmd::Dialect;

    use super::*;

    fn registry() -> CvarRegistry {
        CvarRegistry::new(Dialect::Q1Quakeworld)
    }

    #[test]
    fn registers_engine_defaults() {
        let mut cvars = registry();
        register_quake_world_engine_cvars(&mut cvars).unwrap();
        assert_eq!(cvars.variable_string("sv_stopspeed"), "100");
        assert_eq!(cvars.variable_string("sv_airaccelerate"), "0.7");
        assert_eq!(cvars.variable_string("maxspectators"), "8");
        assert_eq!(cvars.variable_string("password"), "");
    }

    #[test]
    fn keeps_existing_values() {
        let mut cvars = registry();
        cvars.register("sv_friction", "9", 0).unwrap();
        register_quake_world_engine_cvars(&mut cvars).unwrap();
        assert_eq!(cvars.variable_string("sv_friction"), "9");
    }

    #[test]
    fn maxspectators_is_server_info() {
        let mut cvars = registry();
        register_quake_world_engine_cvars(&mut cvars).unwrap();
        let snapshot = cvars.get("maxspectators").expect("registered");
        assert_ne!(snapshot.flags & flags::SERVER_INFO, 0);
    }
}
