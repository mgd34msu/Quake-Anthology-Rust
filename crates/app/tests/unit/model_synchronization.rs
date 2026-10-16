use super::*;
use qa_core::primitives::{ClientId, ResourceNamespace, RuleSetId};

fn model(ordinal: u32) -> Option<ModelRef> {
    ModelRef::native(ResourceNamespace::Client(ClientId(0)), ordinal)
}
fn phase(raw: u32) -> f32 {
    (raw & 0x7fff) as f32 / 32767.0
}

#[test]
fn accepted_transitions_consume_one_phase_each_and_views_do_not() {
    let mut phases = ModelSynchronization::load(4).unwrap();
    let a = Ok(Some((NameId(7), 1)));
    let b = Ok(Some((NameId(8), 1)));
    assert!(phases.observe(1, 0, model(1), true, a));
    assert_eq!(phases.base(1, 0).to_bits(), phase(1804289383).to_bits());
    let first = phases.base(1, 0);
    // A new ordinal for the same loaded source and an image reload do not
    // change the native model identity or consume the next random value.
    assert!(phases.observe(1, 0, model(2), true, a));
    phases.resolve_pending(|_| panic!("resolved model requested again"));
    assert_eq!(phases.base(1, 0).to_bits(), first.to_bits());
    assert!(phases.observe(1, 0, model(3), true, b));
    assert_eq!(phases.base(1, 0).to_bits(), phase(846930886).to_bits());
    assert!(phases.observe(1, 0, model(1), true, a));
    assert_eq!(phases.base(1, 0).to_bits(), phase(1681692777).to_bits());
    assert_eq!(phases.base(2, 0), 0.0);
    phases.retire_missing(|number| number == 1);
    assert_eq!(phases.base(1, 0).to_bits(), phase(1681692777).to_bits());
    phases.retire_missing(|_| false);
    assert_eq!(phases.base(1, 0), 0.0);
    assert!(phases.observe(1, 0, model(1), true, a));
    assert_eq!(phases.base(1, 0).to_bits(), phase(1714636915).to_bits());
}

#[test]
fn pending_resources_bind_once_and_clear_preserves_the_client_random_stream() {
    let mut phases = ModelSynchronization::load(4).unwrap();
    assert!(phases.observe(2, 0, model(8), true, Err(())));
    assert_eq!(phases.base(2, 0), 0.0);
    let mut lookups = 0;
    phases.resolve_pending(|reference| {
        lookups += 1;
        assert_eq!(Some(reference), model(8));
        Err(())
    });
    assert_eq!(lookups, 1);
    phases.retire_missing(|_| false);
    phases.resolve_pending(|_| panic!("absent entity kept a pending resource"));
    assert!(phases.observe(2, 0, model(8), true, Err(())));
    phases.resolve_pending(|_| Ok(Some((NameId(11), 1))));
    assert_eq!(phases.base(2, 0).to_bits(), phase(1804289383).to_bits());
    phases.resolve_pending(|_| panic!("published phase stayed pending"));
    phases.clear();
    assert_eq!(phases.base(2, 0), 0.0);
    phases.resolve_pending(|_| panic!("map reset retained old resource"));
    assert!(phases.observe(2, 0, model(8), true, Ok(Some((NameId(11), 1)))));
    assert_eq!(phases.base(2, 0).to_bits(), phase(846930886).to_bits());
    let other_client = ModelSynchronization::load(4).unwrap();
    assert_eq!(other_client.base(2, 0), 0.0);
    assert!(!phases.observe(4, 0, model(1), true, Ok(None)));
    assert!(!phases.observe(0, 4, model(1), true, Ok(None)));
}

#[test]
fn only_native_netquake_rand_models_randomize_and_detachment_clears_phase() {
    for rules in [
        RuleSetId::Quake,
        RuleSetId::QuakeWorld,
        RuleSetId::Quake2,
        RuleSetId::Quake2Rerelease,
        RuleSetId::Quake3,
    ] {
        let randomize = qa_gameplay::rules::appearance::policy(rules).random_model_sync();
        let mut phases = ModelSynchronization::load(2).unwrap();
        for sync in [0, 2, -1] {
            assert!(phases.observe(1, 0, model(1), randomize, Ok(Some((NameId(1), sync)))));
            assert_eq!(phases.base(1, 0), 0.0);
        }
        assert!(phases.observe(1, 0, model(2), randomize, Ok(Some((NameId(2), 1)))));
        assert_eq!(
            phases.base(1, 0).to_bits(),
            if rules == RuleSetId::Quake {
                phase(1804289383).to_bits()
            } else {
                0
            }
        );
        assert!(phases.observe(1, 0, None, randomize, Ok(None)));
        assert_eq!(phases.base(1, 0), 0.0);
    }
}
