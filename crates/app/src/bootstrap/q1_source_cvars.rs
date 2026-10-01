//! Q1 source-registry staging and bot controls.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/q1-source-cvars.ts`
//! (`cloneQ1SourceCvars`, `registerQ1BotControls`). The merged
//! [`CvarRegistry`](qa_core::cvar::CvarRegistry) has no session context, print sink, or
//! save-state API, so the clone copies the dialect plus every variable — oldest first —
//! with its reset value, flags, current value, and latched value through the public API.
//! One documented gap: console-created membership does not survive the transfer because
//! the merged registry exposes no marking API; the variables themselves (names, values,
//! flags) are all retained.

use qa_core::cvar::{CvarError, CvarRegistry};

/// Clone a source registry into fresh storage (donor `cloneQ1SourceCvars`).
pub fn clone_q1_source_cvars(source: &CvarRegistry) -> Result<CvarRegistry, CvarError> {
    let mut candidate = CvarRegistry::new(source.dialect());
    let mut snapshots = source.snapshots(0);
    snapshots.reverse();
    for snapshot in &snapshots {
        candidate.register(&snapshot.name, &snapshot.reset_value, snapshot.flags)?;
        candidate.set(&snapshot.name, &snapshot.value, true)?;
        if let Some(latched) = &snapshot.latched_value {
            candidate.stage(&snapshot.name, latched)?;
        }
    }
    Ok(candidate)
}

/// Declare the bot population control (donor `registerQ1BotControls`).
pub fn register_q1_bot_controls(cvars: &mut CvarRegistry) -> Result<(), CvarError> {
    if cvars.get("bot_minplayers").is_none() || cvars.is_console_created("bot_minplayers") {
        cvars.register("bot_minplayers", "0", 0)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::cmd::Dialect;
    use qa_core::cvar::flags;

    #[test]
    fn clone_copies_values_and_flags() {
        let mut source = CvarRegistry::new(Dialect::Q1Netquake);
        source.register("sv_cheats", "0", flags::SERVER_INFO).unwrap();
        source.set("sv_cheats", "1", true).unwrap();
        source.register("_cl_name", "Player 1", flags::ARCHIVE).unwrap();
        let clone = clone_q1_source_cvars(&source).unwrap();
        assert_eq!(clone.dialect(), Dialect::Q1Netquake);
        let cheats = clone.get("sv_cheats").unwrap();
        assert_eq!(cheats.value, "1");
        assert_eq!(cheats.reset_value, "0");
        assert_eq!(cheats.flags, flags::SERVER_INFO);
        assert_eq!(clone.variable_string("_cl_name"), "Player 1");
    }

    #[test]
    fn clone_preserves_latched_values() {
        let mut source = CvarRegistry::new(Dialect::Q1Netquake);
        source.register("latched", "a", 0).unwrap();
        source.stage("latched", "b").unwrap();
        let clone = clone_q1_source_cvars(&source).unwrap();
        assert_eq!(clone.get("latched").unwrap().latched_value.as_deref(), Some("b"));
    }

    #[test]
    fn clone_is_independent_storage() {
        let mut source = CvarRegistry::new(Dialect::Q1Quakeworld);
        source.register("name", "Player 1", flags::ARCHIVE).unwrap();
        let mut clone = clone_q1_source_cvars(&source).unwrap();
        clone.set("name", "Changed", true).unwrap();
        assert_eq!(source.variable_string("name"), "Player 1");
        assert_eq!(clone.variable_string("name"), "Changed");
    }

    #[test]
    fn bot_controls_register_once() {
        let mut cvars = CvarRegistry::new(Dialect::Q1Netquake);
        register_q1_bot_controls(&mut cvars).unwrap();
        assert_eq!(cvars.variable_string("bot_minplayers"), "0");
        cvars.set("bot_minplayers", "4", true).unwrap();
        register_q1_bot_controls(&mut cvars).unwrap();
        assert_eq!(cvars.variable_string("bot_minplayers"), "4");
    }
}
