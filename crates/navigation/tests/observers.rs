use qa_core::primitives::EntityId;
use qa_navigation::Observers;

#[test]
fn observer_removal_checks_the_full_lifetime_and_preserves_other_slots() {
    let mut observers = Observers::load(3);
    let old = EntityId {
        slot: 1,
        generation: 1,
    };
    let next = EntityId {
        slot: 1,
        generation: 2,
    };
    let other = EntityId {
        slot: 2,
        generation: 1,
    };
    assert!(!observers.unregister(old));
    assert!(observers.register(old));
    assert!(observers.register(other));
    assert!(observers.register(next));
    assert!(!observers.unregister(old));
    assert!(observers.contains(next));
    assert!(observers.contains(other));
    assert_eq!(observers.iter().collect::<Vec<_>>(), [next, other]);
    assert!(observers.unregister(next));
    assert!(!observers.unregister(next));
    assert_eq!(observers.iter().collect::<Vec<_>>(), [other]);
    assert!(!observers.register(EntityId {
        slot: 3,
        generation: 1
    }));
}
