use qa_core::primitives::{CommandUnits, RuleSetId};
use qa_gameplay::rules::command_units;

#[test]
fn native_command_references_are_independent_of_speed_cvars() {
    for (rules, units) in [
        (
            RuleSetId::Quake,
            CommandUnits::Speed {
                normalized_reference: [320.0; 3],
            },
        ),
        (
            RuleSetId::QuakeWorld,
            CommandUnits::Speed {
                normalized_reference: [320.0; 3],
            },
        ),
        (
            RuleSetId::Quake2,
            CommandUnits::Speed {
                normalized_reference: [200.0; 3],
            },
        ),
        (
            RuleSetId::Quake2Rerelease,
            CommandUnits::Speed {
                normalized_reference: [200.0; 3],
            },
        ),
        (
            RuleSetId::Quake3,
            CommandUnits::Normalized {
                full_scale: [127.0; 3],
            },
        ),
    ] {
        assert_eq!(command_units(rules), units);
    }
}
