use super::legacy_kind;
use qa_core::primitives::RuleSetId;

#[test]
fn names_and_flags_follow_the_selected_native_rules() {
    for rules in [RuleSetId::Quake, RuleSetId::QuakeWorld] {
        for flags in [0, 4, 8, 12] {
            assert_eq!(legacy_kind(rules, b"wall", flags), (false, false));
            assert_eq!(legacy_kind(rules, b"*water", flags), (true, false));
            assert_eq!(legacy_kind(rules, b"sky1", flags), (false, true));
            assert_eq!(legacy_kind(rules, b"Sky1", flags), (false, false));
        }
    }
    for rules in [RuleSetId::Quake2, RuleSetId::Quake2Rerelease] {
        for name in [b"wall".as_slice(), b"*water", b"sky1"] {
            assert_eq!(legacy_kind(rules, name, 0), (false, false));
            assert_eq!(legacy_kind(rules, name, 4), (false, true));
            assert_eq!(legacy_kind(rules, name, 8), (true, false));
            assert_eq!(legacy_kind(rules, name, 12), (true, true));
        }
    }
    assert_eq!(legacy_kind(RuleSetId::Quake3, b"sky1", 12), (false, false));
}
