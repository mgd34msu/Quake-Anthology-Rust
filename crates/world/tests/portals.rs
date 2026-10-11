use qa_world::portals::{AreaPortals, Portal, PortalError};

fn edge(number: u32, first: u32, second: u32) -> Portal {
    Portal {
        number,
        first,
        second,
    }
}

#[test]
fn numbered_portals_share_pairs_without_losing_an_independent_open_door() {
    let mut areas = AreaPortals::load(
        4,
        4,
        vec![edge(1, 1, 2), edge(1, 2, 1), edge(2, 1, 2), edge(3, 2, 3)],
    )
    .unwrap();
    assert_eq!(areas.connected(1, 2), Some(false));
    assert!(areas.set(1, true).unwrap());
    assert!(!areas.set(1, true).unwrap());
    assert!(areas.set(2, true).unwrap());
    assert!(areas.set(1, false).unwrap());
    assert_eq!(areas.connected(1, 2), Some(true));
    areas.set(3, true).unwrap();
    assert_eq!(areas.connected(1, 3), Some(true));
    areas.set(2, false).unwrap();
    assert_eq!(areas.connected(1, 3), Some(false));
    assert_eq!(areas.connected(2, 3), Some(true));
    assert_eq!(areas.connected(0, 1), Some(false));
}

#[test]
fn counted_pairs_preserve_multiple_users_and_reject_an_unbalanced_close() {
    let mut areas = AreaPortals::load(3, 0, vec![]).unwrap();
    areas.adjust(0, 1, true).unwrap();
    areas.adjust(1, 0, true).unwrap();
    areas.adjust(1, 2, true).unwrap();
    areas.adjust(0, 1, false).unwrap();
    assert_eq!(areas.connected(0, 2), Some(true));
    areas.adjust(1, 0, false).unwrap();
    assert_eq!(areas.connected(0, 2), Some(false));
    assert_eq!(areas.adjust(0, 1, false), Err(PortalError::Balance));
    assert_eq!(areas.connected(1, 2), Some(true));
    areas.adjust(0, 0, true).unwrap();
    areas.adjust(0, 0, false).unwrap();
    assert_eq!(areas.adjust(0, 0, false), Err(PortalError::Balance));
}

#[test]
fn cyclic_flood_and_every_edge_of_one_number_use_the_same_graph() {
    let mut areas =
        AreaPortals::load(5, 2, vec![edge(1, 0, 1), edge(1, 1, 2), edge(1, 2, 0)]).unwrap();
    areas.set(1, true).unwrap();
    assert_eq!(areas.connected(0, 2), Some(true));
    assert_eq!(areas.connected(0, 3), Some(false));
    areas.set(1, false).unwrap();
    assert_eq!(areas.connected(0, 2), Some(false));
    assert!(areas.set(0, true).unwrap()); // A native numbered hole has no edges.
    assert_eq!(areas.connected(0, 2), Some(false));
}

#[test]
fn malformed_load_and_runtime_indices_leave_existing_connectivity_intact() {
    assert!(matches!(
        AreaPortals::load(2, 1, vec![edge(0, 0, 2)]),
        Err(PortalError::Area)
    ));
    assert!(matches!(
        AreaPortals::load(2, 1, vec![edge(1, 0, 1)]),
        Err(PortalError::Number)
    ));
    assert!(matches!(
        AreaPortals::load(usize::MAX, 0, vec![]),
        Err(PortalError::Size)
    ));
    let mut areas = AreaPortals::load(2, 1, vec![edge(0, 0, 1)]).unwrap();
    assert_eq!(areas.connected(2, 0), None);
    assert_eq!(areas.set(1, true), Err(PortalError::Number));
    assert_eq!(areas.adjust(0, 2, true), Err(PortalError::Area));
    assert_eq!(areas.connected(0, 1), Some(false));
    areas.set(0, true).unwrap();
    assert_eq!(areas.connected(0, 1), Some(true));
}
