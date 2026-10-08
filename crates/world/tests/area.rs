use qa_core::primitives::{Bounds, ModuleId, Vec3};
use qa_world::entities::AllocationPolicy;
use qa_world::{
    area::{AreaGrid, LinkFlags},
    entities::EntityTable,
};

#[test]
fn only_changed_rows_relink_and_queries_borrow_live_columns() {
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
    assert!(grid.link(&table, solid, LinkFlags::SOLID));
    assert!(grid.link(&table, trigger, LinkFlags::TRIGGER));
    assert!(!grid.link(&table, solid, LinkFlags::SOLID));
    assert_eq!(grid.relinks, 2);
    let row = grid.query(&table, bounds, LinkFlags::SOLID).next().unwrap();
    assert!(std::ptr::eq(
        row.position,
        &table.columns.position[solid.slot as usize]
    ));
    assert_eq!(row.id, solid);
    assert_eq!(grid.query(&table, bounds, LinkFlags::TRIGGER).count(), 1);
    table.columns.position[solid.slot as usize] = Vec3([100.0; 3]);
    assert!(grid.link(&table, solid, LinkFlags::SOLID));
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
    assert!(grid.link(&table, replacement, LinkFlags::SOLID));
    assert!(!grid.unlink(solid));
    assert_eq!(
        grid.query(&table, bounds, LinkFlags::SOLID)
            .next()
            .unwrap()
            .id,
        replacement
    );
}
