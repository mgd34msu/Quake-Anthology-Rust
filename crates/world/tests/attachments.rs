use qa_core::primitives::ThinkTime;
use qa_core::primitives::{BodyAttachment, BodyFollow, Bounds, EntityId, ModuleId, Vec3};
use qa_world::{
    area::{AreaGrid, LinkFlags, LinkIntent, LinkOrder},
    entities::{AllocationPolicy, AttachmentError, EntityTable},
};

fn setup(capacity: usize, reserved: usize) -> (EntityTable, AreaGrid) {
    (
        EntityTable::new(capacity, reserved).unwrap(),
        AreaGrid::load(
            capacity,
            Bounds {
                mins: Vec3([-1024.0; 3]),
                maxs: Vec3([1024.0; 3]),
            },
        )
        .unwrap(),
    )
}
fn spawn(table: &mut EntityTable) -> EntityId {
    table
        .allocate(
            ThinkTime::Seconds(3.0),
            ModuleId(1),
            AllocationPolicy::QUAKEWORLD,
        )
        .unwrap()
        .id
}
fn follow(anchor: EntityId, mode: BodyFollow, offset: [f32; 3]) -> BodyAttachment {
    BodyAttachment {
        anchor,
        follow: mode,
        offset: Vec3(offset),
    }
}

#[test]
fn follows_parent_first_and_preserves_body_fields_and_native_link_order() {
    let (mut table, mut grid) = setup(8, 1);
    let root = spawn(&mut table);
    let parent = spawn(&mut table);
    let child = spawn(&mut table);
    let other = spawn(&mut table);
    table.columns.position[root.slot as usize] = Vec3([20.0, 30.0, 40.0]);
    for id in [parent, child, other] {
        let slot = id.slot as usize;
        table.columns.mins[slot] = Vec3([-32.0, -32.0, -8.0]);
        table.columns.maxs[slot] = Vec3([32.0, 32.0, 16.0]);
        table.columns.velocity[slot] = Vec3([1.0, 2.0, 3.0]);
        table.columns.angles[slot] = Vec3([4.0, 5.0, 6.0]);
        assert!(grid.link(
            &table,
            id,
            LinkFlags::TRIGGER,
            LinkOrder::Tail,
            LinkIntent::Explicit
        ));
    }
    // Child inserted first must capture its mode before transporting the parent.
    table
        .attach(child, follow(parent, BodyFollow::Center, [999.0; 3]))
        .unwrap();
    table
        .attach(
            parent,
            follow(root, BodyFollow::Translation, [1.0, 2.0, 3.0]),
        )
        .unwrap();
    table
        .attach(other, follow(root, BodyFollow::BoundsMin, [3.0; 3]))
        .unwrap();
    let first = grid.transport_attachments(&mut table);
    assert_eq!((first.visited, first.moved, first.relinked), (3, 3, 3));
    assert_eq!(
        table.columns.position[parent.slot as usize],
        Vec3([21.0, 32.0, 43.0])
    );
    assert_eq!(
        table.columns.position[child.slot as usize],
        Vec3([21.0, 32.0, 47.0])
    );
    for id in [parent, child, other] {
        let slot = id.slot as usize;
        assert_eq!(table.columns.velocity[slot], Vec3([1.0, 2.0, 3.0]));
        assert_eq!(table.columns.angles[slot], Vec3([4.0, 5.0, 6.0]));
        assert_eq!(table.columns.mins[slot], Vec3([-32.0, -32.0, -8.0]));
        assert_eq!(table.columns.maxs[slot], Vec3([32.0, 32.0, 16.0]));
    }
    let order: Vec<_> = grid
        .query(
            &table,
            Bounds {
                mins: Vec3([-1024.0; 3]),
                maxs: Vec3([1024.0; 3]),
            },
            LinkFlags::TRIGGER,
        )
        .map(|row| row.id)
        .collect();
    // The first two share a node; reinsert is parent, then child.
    assert!(order.iter().position(|&id| id == parent) < order.iter().position(|&id| id == child));
    let stable = grid.transport_attachments(&mut table);
    assert_eq!((stable.visited, stable.moved, stable.relinked), (3, 0, 0));

    // Head-inserted entities reverse that order in the same node.
    for id in [parent, child] {
        assert!(grid.link(
            &table,
            id,
            LinkFlags::SOLID,
            LinkOrder::Head,
            LinkIntent::Explicit
        ));
    }
    table.columns.position[root.slot as usize].0[2] += 1.0;
    grid.transport_attachments(&mut table);
    let order: Vec<_> = grid
        .query(
            &table,
            Bounds {
                mins: Vec3([-1024.0; 3]),
                maxs: Vec3([1024.0; 3]),
            },
            LinkFlags::SOLID,
        )
        .map(|row| row.id)
        .collect();
    assert_eq!(order, [child, parent]);
}

#[test]
fn attachment_updates_keep_insertion_order_and_rejections_leave_previous_follow() {
    let (mut table, _) = setup(6, 1);
    let root = spawn(&mut table);
    let a = spawn(&mut table);
    let b = spawn(&mut table);
    table
        .attach(a, follow(root, BodyFollow::Translation, [1.0; 3]))
        .unwrap();
    table
        .attach(b, follow(a, BodyFollow::BoundsMin, [2.0; 3]))
        .unwrap();
    let replacement = follow(root, BodyFollow::Center, [3.0; 3]);
    table.attach(a, replacement).unwrap();
    assert_eq!(
        table.attach(a, follow(b, BodyFollow::Translation, [0.0; 3])),
        Err(AttachmentError::Cycle)
    );
    assert_eq!(
        table.attach(a, follow(a, BodyFollow::Center, [0.0; 3])),
        Err(AttachmentError::Cycle)
    );
    assert_eq!(
        table.attach(a, follow(root, BodyFollow::Translation, [f32::NAN; 3])),
        Err(AttachmentError::Offset)
    );
    assert_eq!(table.attachment(a), Some(replacement));
    assert_eq!(
        table.attachments().map(|(id, _)| id).collect::<Vec<_>>(),
        [a, b]
    );
    assert!(table.detach(a));
    table.attach(a, replacement).unwrap();
    assert_eq!(
        table.attachments().map(|(id, _)| id).collect::<Vec<_>>(),
        [b, a]
    );
}

#[test]
fn release_reset_and_qw_displacement_detach_only_direct_children() {
    for action in 0..3 {
        let (mut table, mut grid) = setup(5, if action == 1 { 2 } else { 1 });
        let root = if action == 1 {
            table.id_at(1).unwrap()
        } else {
            spawn(&mut table)
        };
        let a = spawn(&mut table);
        let b = spawn(&mut table);
        table
            .attach(a, follow(root, BodyFollow::Translation, [1.0; 3]))
            .unwrap();
        table
            .attach(b, follow(a, BodyFollow::Translation, [1.0; 3]))
            .unwrap();
        let old = root;
        let new = match action {
            0 => {
                assert!(table.release(root, ThinkTime::Seconds(3.0)));
                table
                    .allocate(
                        ThinkTime::Seconds(4.0),
                        ModuleId(1),
                        AllocationPolicy::EDICT,
                    )
                    .unwrap()
                    .id
            }
            1 => table.reset_client(root).unwrap(),
            _ => {
                // Fill, then make the anchor the last unprotected owned slot.
                let last = spawn(&mut table);
                table.detach(a);
                table
                    .attach(a, follow(last, BodyFollow::Translation, [1.0; 3]))
                    .unwrap();
                table.set_never_free(root, true);
                table.set_never_free(a, true);
                table.set_never_free(b, true);
                let replacement = table
                    .allocate(
                        ThinkTime::Seconds(3.0),
                        ModuleId(1),
                        AllocationPolicy::QUAKEWORLD,
                    )
                    .unwrap();
                assert_eq!(replacement.displaced, Some(last));
                assert!(table.resolve(last).is_none());
                assert!(table.attachment(a).is_none());
                assert!(table.attachment(b).is_some());
                continue;
            }
        };
        assert_eq!(new.slot, old.slot);
        assert_ne!(new.generation, old.generation);
        assert!(table.resolve(old).is_none());
        assert!(table.attachment(a).is_none());
        assert_eq!(table.attachment(b).unwrap().anchor, a);
        assert_eq!(
            table.attach(a, follow(old, BodyFollow::Center, [0.0; 3])),
            Err(AttachmentError::Anchor)
        );
        let result = grid.transport_attachments(&mut table);
        assert_eq!(result.visited, 1);
    }
}

#[test]
fn signed_zero_change_moves_once_without_registering_an_unlinked_child() {
    let (mut table, mut grid) = setup(3, 1);
    let root = spawn(&mut table);
    let child = spawn(&mut table);
    table.columns.position[root.slot as usize] = Vec3([-0.0; 3]);
    table
        .attach(child, follow(root, BodyFollow::Translation, [-0.0; 3]))
        .unwrap();
    let result = grid.transport_attachments(&mut table);
    assert_eq!((result.moved, result.relinked), (1, 0));
    assert_eq!(
        table.columns.position[child.slot as usize]
            .0
            .map(f32::to_bits),
        [(-0.0f32).to_bits(); 3]
    );
    assert_eq!(grid.transport_attachments(&mut table).moved, 0);
}

#[test]
fn overflowing_follow_is_scoped_and_mismatched_scratch_cannot_grow_or_panic() {
    let (mut table, mut grid) = setup(4, 1);
    let root = spawn(&mut table);
    let a = spawn(&mut table);
    let b = spawn(&mut table);
    table.columns.position[root.slot as usize] = Vec3([f32::MAX, 2.0, 3.0]);
    table
        .attach(a, follow(root, BodyFollow::Translation, [f32::MAX; 3]))
        .unwrap();
    table
        .attach(b, follow(root, BodyFollow::Translation, [0.0; 3]))
        .unwrap();
    let result = grid.transport_attachments(&mut table);
    assert_eq!((result.moved, result.rejected), (1, 1));
    assert_eq!(table.columns.position[a.slot as usize], Vec3::default());
    assert_eq!(
        table.columns.position[b.slot as usize],
        Vec3([f32::MAX, 2.0, 3.0])
    );
    let (_, mut smaller) = setup(2, 1);
    let result = smaller.transport_attachments(&mut table);
    assert_eq!((result.visited, result.rejected), (0, 2));
}

#[test]
fn predicted_chain_uses_unmapped_intermediate_anchors_and_keeps_physical_state_frozen() {
    let (mut table, mut grid) = setup(6, 1);
    let root = spawn(&mut table);
    let middle = spawn(&mut table);
    let child = spawn(&mut table);
    let remote = spawn(&mut table);
    table.columns.mins[middle.slot as usize] = Vec3([-10.0; 3]);
    table.columns.maxs[middle.slot as usize] = Vec3([14.0; 3]);
    table.columns.position[remote.slot as usize] = Vec3([500.0; 3]);
    table
        .attach(child, follow(middle, BodyFollow::Center, [99.0; 3]))
        .unwrap();
    table
        .attach(middle, follow(root, BodyFollow::Translation, [1.0; 3]))
        .unwrap();
    for id in [root, middle, child] {
        grid.link(
            &table,
            id,
            LinkFlags::SOLID,
            LinkOrder::Head,
            LinkIntent::Explicit,
        );
    }
    let physical = table.columns.position.to_vec();
    let relinks = grid.relinks;
    let mut poses = [
        Some((child, Vec3([-100.0; 3]))),
        None,
        Some((root, Vec3([20.0; 3]))),
    ];
    let result = grid.predict_attachments(&table, &mut poses);
    assert_eq!(
        (
            result.visited,
            result.moved,
            result.relinked,
            result.rejected
        ),
        (2, 2, 0, 0)
    );
    assert_eq!(poses[0], Some((child, Vec3([23.0; 3]))));
    assert_eq!(poses[2], Some((root, Vec3([20.0; 3]))));
    assert_eq!(&*table.columns.position, &physical);
    assert_eq!(grid.relinks, relinks);
    // Root is now remote: use its authoritative position, without old marks.
    table
        .attach(middle, follow(remote, BodyFollow::BoundsMin, [2.0; 3]))
        .unwrap();
    grid.predict_attachments(&table, &mut poses);
    assert_eq!(poses[0], Some((child, Vec3([504.0; 3]))));
    assert_eq!(&*table.columns.position, &physical);
    assert_eq!(grid.relinks, relinks);
}
