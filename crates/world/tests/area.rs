use qa_core::primitives::{Bounds, EntityId, ModuleId, Vec3};
use qa_world::entities::AllocationPolicy;
use qa_world::{
    area::{AreaGrid, LinkFlags, LinkIntent, LinkOrder},
    entities::EntityTable,
};

#[test]
fn unchanged_internal_commits_keep_links_and_queries_borrow_live_columns() {
    let bounds = Bounds {
        mins: Vec3([-1024.0; 3]),
        maxs: Vec3([1024.0; 3]),
    };
    let mut grid = AreaGrid::load(64, bounds).unwrap();
    let mut table = EntityTable::new(64, 1).unwrap();
    let solid = table
        .allocate(3.0, ModuleId(1), AllocationPolicy::EDICT)
        .unwrap()
        .id;
    let trigger = table
        .allocate(3.0, ModuleId(1), AllocationPolicy::EDICT)
        .unwrap()
        .id;
    table.columns.mins[solid.slot as usize] = Vec3([-16.0; 3]);
    table.columns.maxs[solid.slot as usize] = Vec3([16.0; 3]);
    assert!(grid.link(
        &table,
        solid,
        LinkFlags::SOLID,
        LinkOrder::Tail,
        LinkIntent::Commit
    ));
    assert!(grid.link(
        &table,
        trigger,
        LinkFlags::TRIGGER,
        LinkOrder::Tail,
        LinkIntent::Commit
    ));
    assert!(!grid.link(
        &table,
        solid,
        LinkFlags::SOLID,
        LinkOrder::Tail,
        LinkIntent::Commit
    ));
    assert_eq!(grid.relinks, 2);
    let row = grid.query(&table, bounds, LinkFlags::SOLID).next().unwrap();
    assert!(std::ptr::eq(
        row.position,
        &table.columns.position[solid.slot as usize]
    ));
    assert_eq!(row.id, solid);
    assert_eq!(grid.query(&table, bounds, LinkFlags::TRIGGER).count(), 1);
    table.columns.position[solid.slot as usize] = Vec3([100.0; 3]);
    assert!(grid.link(
        &table,
        solid,
        LinkFlags::SOLID,
        LinkOrder::Tail,
        LinkIntent::Commit
    ));
    assert_eq!(grid.relinks, 3);
    assert_eq!(
        grid.query(&table, Bounds::default(), LinkFlags::SOLID)
            .count(),
        0
    );
    assert!(grid.unlink(solid));
    table.release(solid, 3.0);
    let replacement = table
        .allocate(4.0, ModuleId(1), AllocationPolicy::EDICT)
        .unwrap()
        .id;
    assert!(grid.link(
        &table,
        replacement,
        LinkFlags::SOLID,
        LinkOrder::Tail,
        LinkIntent::Explicit
    ));
    assert!(!grid.unlink(solid));
    assert_eq!(
        grid.query(&table, bounds, LinkFlags::SOLID)
            .next()
            .unwrap()
            .id,
        replacement
    );
}

fn bounds() -> Bounds {
    Bounds {
        mins: Vec3([-1024.0; 3]),
        maxs: Vec3([1024.0; 3]),
    }
}

fn setup() -> (EntityTable, AreaGrid) {
    (
        EntityTable::new(16, 1).unwrap(),
        AreaGrid::load(16, bounds()).unwrap(),
    )
}

fn allocate(table: &mut EntityTable) -> EntityId {
    table
        .allocate(3.0, ModuleId(1), AllocationPolicy::EDICT)
        .unwrap()
        .id
}

fn ids(grid: &AreaGrid, table: &EntityTable, flags: LinkFlags) -> Vec<EntityId> {
    grid.query(table, bounds(), flags)
        .map(|row| row.id)
        .collect()
}

#[test]
fn native_tail_relink_changes_touch_order_without_a_body_change() {
    // WinQuake world.c:376-377,463-470; Q2 sv_world.c:339-343.
    let (mut table, mut grid) = setup();
    let first = allocate(&mut table);
    let solid = allocate(&mut table);
    let second = allocate(&mut table);
    for (id, flags) in [
        (first, LinkFlags::TRIGGER),
        (solid, LinkFlags::SOLID),
        (second, LinkFlags::TRIGGER),
    ] {
        assert!(grid.link(&table, id, flags, LinkOrder::Tail, LinkIntent::Explicit));
    }
    assert_eq!(ids(&grid, &table, LinkFlags::TRIGGER), [first, second]);
    assert!(!grid.link(
        &table,
        first,
        LinkFlags::TRIGGER,
        LinkOrder::Tail,
        LinkIntent::Commit
    ));
    assert_eq!(ids(&grid, &table, LinkFlags::TRIGGER), [first, second]);
    assert!(grid.link(
        &table,
        first,
        LinkFlags::TRIGGER,
        LinkOrder::Tail,
        LinkIntent::Explicit
    ));
    assert_eq!(ids(&grid, &table, LinkFlags::TRIGGER), [second, first]);
    assert_eq!(
        ids(&grid, &table, LinkFlags::LINKED),
        [solid, second, first]
    );
    assert_eq!(grid.relinks, 4);
}

#[test]
fn native_head_order_includes_triggers_and_noncolliding_linked_entities() {
    // Q3 sv_world.c:350-355,389-409 uses one list, not solids then triggers.
    let (mut table, mut grid) = setup();
    let solid = allocate(&mut table);
    let trigger = allocate(&mut table);
    let noncolliding = allocate(&mut table);
    for (id, flags) in [
        (solid, LinkFlags::SOLID),
        (trigger, LinkFlags::TRIGGER),
        (noncolliding, LinkFlags::LINKED),
    ] {
        assert!(grid.link(&table, id, flags, LinkOrder::Head, LinkIntent::Explicit));
    }
    assert_eq!(
        ids(&grid, &table, LinkFlags::LINKED),
        [noncolliding, trigger, solid]
    );
    assert_eq!(
        ids(
            &grid,
            &table,
            LinkFlags(LinkFlags::SOLID.0 | LinkFlags::TRIGGER.0)
        ),
        [trigger, solid]
    );
    assert_eq!(ids(&grid, &table, LinkFlags::SOLID), [solid]);
    assert_eq!(ids(&grid, &table, LinkFlags::TRIGGER), [trigger]);
    assert!(grid.link(
        &table,
        solid,
        LinkFlags::SOLID,
        LinkOrder::Head,
        LinkIntent::Explicit
    ));
    assert_eq!(
        ids(&grid, &table, LinkFlags::LINKED),
        [solid, noncolliding, trigger]
    );
}

#[test]
fn combined_node_keeps_each_entity_insertion_rule() {
    let (mut table, mut grid) = setup();
    let a = allocate(&mut table);
    let b = allocate(&mut table);
    let c = allocate(&mut table);
    let d = allocate(&mut table);
    let e = allocate(&mut table);
    let f = allocate(&mut table);
    for (id, flags, order) in [
        (a, LinkFlags::SOLID, LinkOrder::Tail),
        (b, LinkFlags::TRIGGER, LinkOrder::Head),
        (c, LinkFlags::TRIGGER, LinkOrder::Tail),
        (d, LinkFlags::SOLID, LinkOrder::Head),
        (e, LinkFlags::SOLID, LinkOrder::Tail),
        (f, LinkFlags::LINKED, LinkOrder::Head),
    ] {
        assert!(grid.link(&table, id, flags, order, LinkIntent::Explicit));
    }
    assert_eq!(ids(&grid, &table, LinkFlags::LINKED), [f, d, b, a, c, e]);
    assert_eq!(ids(&grid, &table, LinkFlags::SOLID), [d, a, e]);
    assert_eq!(ids(&grid, &table, LinkFlags::TRIGGER), [b, c]);
    assert!(!grid.link(
        &table,
        a,
        LinkFlags::SOLID,
        LinkOrder::Tail,
        LinkIntent::Commit
    ));
    assert_eq!(ids(&grid, &table, LinkFlags::LINKED), [f, d, b, a, c, e]);
    assert!(grid.link(
        &table,
        a,
        LinkFlags::SOLID,
        LinkOrder::Tail,
        LinkIntent::Explicit
    ));
    assert_eq!(ids(&grid, &table, LinkFlags::LINKED), [f, d, b, c, e, a]);
    assert!(grid.link(
        &table,
        b,
        LinkFlags::TRIGGER,
        LinkOrder::Tail,
        LinkIntent::Commit
    ));
    assert_eq!(ids(&grid, &table, LinkFlags::LINKED), [f, d, c, e, a, b]);
}

#[test]
fn single_list_unlink_repairs_neighbors_across_roles_and_can_empty_the_node() {
    let (mut table, mut grid) = setup();
    let first = allocate(&mut table);
    let middle = allocate(&mut table);
    let last = allocate(&mut table);
    for (id, flags) in [
        (first, LinkFlags::SOLID),
        (middle, LinkFlags::TRIGGER),
        (last, LinkFlags::SOLID),
    ] {
        assert!(grid.link(&table, id, flags, LinkOrder::Tail, LinkIntent::Explicit));
    }
    assert!(grid.unlink(middle));
    assert_eq!(ids(&grid, &table, LinkFlags::LINKED), [first, last]);
    assert!(grid.unlink(first));
    assert!(grid.unlink(last));
    assert!(ids(&grid, &table, LinkFlags::LINKED).is_empty());
    assert!(!grid.unlink(last));
    assert!(grid.link(
        &table,
        middle,
        LinkFlags::TRIGGER,
        LinkOrder::Head,
        LinkIntent::Explicit
    ));
    assert_eq!(ids(&grid, &table, LinkFlags::LINKED), [middle]);
}

#[test]
fn stale_lifetime_cannot_unlink_or_republish_a_replacement() {
    let (mut table, mut grid) = setup();
    let stale = allocate(&mut table);
    let neighbor = allocate(&mut table);
    assert!(grid.link(
        &table,
        stale,
        LinkFlags::TRIGGER,
        LinkOrder::Head,
        LinkIntent::Explicit
    ));
    assert!(grid.link(
        &table,
        neighbor,
        LinkFlags::SOLID,
        LinkOrder::Tail,
        LinkIntent::Explicit
    ));
    assert!(table.release(stale, 3.0));
    assert_eq!(ids(&grid, &table, LinkFlags::LINKED), [neighbor]);
    let replacement = table
        .allocate(4.0, ModuleId(2), AllocationPolicy::EDICT)
        .unwrap()
        .id;
    assert_eq!(replacement.slot, stale.slot);
    assert_ne!(replacement.generation, stale.generation);
    assert!(grid.link(
        &table,
        replacement,
        LinkFlags::SOLID,
        LinkOrder::Tail,
        LinkIntent::Commit
    ));
    assert!(!grid.unlink(stale));
    assert!(!grid.link(
        &table,
        stale,
        LinkFlags::TRIGGER,
        LinkOrder::Head,
        LinkIntent::Explicit
    ));
    assert_eq!(
        ids(&grid, &table, LinkFlags::LINKED),
        [neighbor, replacement]
    );
    assert!(grid.unlink(replacement));
    assert_eq!(ids(&grid, &table, LinkFlags::LINKED), [neighbor]);
}

#[test]
fn role_changes_and_unlink_intent_do_not_duplicate_a_row() {
    let (mut table, mut grid) = setup();
    let entity = allocate(&mut table);
    assert!(grid.link(
        &table,
        entity,
        LinkFlags::SOLID,
        LinkOrder::Head,
        LinkIntent::Commit
    ));
    let both = LinkFlags(LinkFlags::SOLID.0 | LinkFlags::TRIGGER.0 | LinkFlags::ITEM);
    assert!(grid.link(&table, entity, both, LinkOrder::Head, LinkIntent::Commit));
    assert_eq!(ids(&grid, &table, both), [entity]);
    assert_eq!(ids(&grid, &table, LinkFlags::TRIGGER), [entity]);
    assert!(ids(&grid, &table, LinkFlags(LinkFlags::ITEM)).is_empty());
    assert!(!grid.link(
        &table,
        entity,
        LinkFlags::default(),
        LinkOrder::Head,
        LinkIntent::Explicit
    ));
    assert!(ids(&grid, &table, LinkFlags::LINKED).is_empty());
}

#[test]
fn native_queries_visit_node_members_then_front_then_back() {
    // WinQuake world.c:284,315-318 and Q3 sv_world.c:389,417-421.
    let (mut table, mut grid) = setup();
    let back = allocate(&mut table);
    let front = allocate(&mut table);
    let root = allocate(&mut table);
    table.columns.position[back.slot as usize] = Vec3([0.0, -32.0, 0.0]);
    table.columns.position[front.slot as usize] = Vec3([0.0, 32.0, 0.0]);
    for id in [back, front, root] {
        assert!(grid.link(
            &table,
            id,
            LinkFlags::TRIGGER,
            LinkOrder::Tail,
            LinkIntent::Explicit
        ));
    }
    assert_eq!(ids(&grid, &table, LinkFlags::TRIGGER), [root, front, back]);
    assert_eq!(
        grid.query(&table, Bounds::default(), LinkFlags::TRIGGER)
            .map(|row| row.id)
            .collect::<Vec<_>>(),
        [root]
    );
    let boundary = Bounds {
        mins: Vec3([0.0, 31.0, 0.0]),
        maxs: Vec3([0.0, 31.0, 0.0]),
    };
    assert_eq!(
        grid.query(&table, boundary, LinkFlags::TRIGGER)
            .map(|row| row.id)
            .collect::<Vec<_>>(),
        [front]
    );
}
