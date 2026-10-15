use qa_core::primitives::{CommandUnits, RuleSetId};

/// Fixed mixed-command references from simulation/q3-commands.ts. Native
/// construction still applies each source's cvars before crossing units.
pub const fn command_units(rules: RuleSetId) -> CommandUnits {
    match rules {
        RuleSetId::Quake | RuleSetId::QuakeWorld => CommandUnits::Speed {
            normalized_reference: [320.0; 3],
        },
        RuleSetId::Quake2 | RuleSetId::Quake2Rerelease => CommandUnits::Speed {
            normalized_reference: [200.0; 3],
        },
        RuleSetId::Quake3 => CommandUnits::Normalized {
            full_scale: [127.0; 3],
        },
    }
}
