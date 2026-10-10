use qa_core::primitives::ThinkTime;
use qa_core::primitives::{
    CollisionOwner, CollisionShape, CollisionTags, EntityId, ModuleId, NativeEntity,
};
use qa_world::{
    collision::Contents,
    entities::{AllocationPolicy, EntityTable},
};

fn attach_collision(table: &mut EntityTable, id: EntityId, collision_owner: CollisionOwner) {
    let slot = table.resolve(id).unwrap();
    table.columns.native_entity[slot] = Some(NativeEntity {
        module: table.columns.owner[slot],
        slot: 17,
    });
    table.columns.collision_owner[slot] = collision_owner;
    table.columns.collision_shape[slot] = CollisionShape::Box;
    table.columns.collision_contents[slot] =
        (Contents::SOLID | Contents::BODY | Contents::WINDOW).0;
    table.columns.collision_tags[slot] = CollisionTags::MONSTER | CollisionTags::DEAD_MONSTER;
}

fn assert_collision_cleared(table: &EntityTable, slot: usize) {
    assert_eq!(table.columns.native_entity[slot], None);
    assert_eq!(table.columns.collision_owner[slot], CollisionOwner::None);
    assert_eq!(table.columns.collision_shape[slot], CollisionShape::None);
    assert_eq!(table.columns.collision_contents[slot], 0);
    assert_eq!(table.columns.collision_tags[slot], CollisionTags::default());
}

#[test]
fn collision_columns_start_empty_and_clear_on_free_and_reuse() {
    let mut table = EntityTable::new(2, 1).unwrap();
    for slot in 0..table.capacity() {
        assert_collision_cleared(&table, slot);
    }
    let first = table
        .allocate(
            ThinkTime::Seconds(10.0),
            ModuleId(2),
            AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    let owner = NativeEntity {
        module: ModuleId(3),
        slot: 9,
    };
    attach_collision(&mut table, first, CollisionOwner::Native(owner));
    let slot = first.slot as usize;
    assert_eq!(table.columns.owner[slot], ModuleId(2));
    assert_eq!(
        table.columns.collision_owner[slot],
        CollisionOwner::Native(owner)
    );
    assert_ne!(
        table.columns.collision_contents[slot] & Contents::WINDOW.0,
        0
    );
    assert!(table.release(first, ThinkTime::Seconds(10.0)));
    assert!(table.resolve(first).is_none());
    assert_collision_cleared(&table, slot);
    let current = table
        .allocate(
            ThinkTime::Seconds(10.501),
            ModuleId(4),
            AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    assert_eq!(current.slot, first.slot);
    assert_ne!(current.generation, first.generation);
    assert_collision_cleared(&table, slot);
    assert_eq!(table.columns.owner[slot], ModuleId(4));
}

#[test]
fn qw_displacement_clears_collision_data_before_the_new_lifetime_is_exposed() {
    let mut table = EntityTable::new(3, 1).unwrap();
    let first = table
        .allocate(
            ThinkTime::Seconds(10.0),
            ModuleId(2),
            AllocationPolicy::QUAKEWORLD,
        )
        .unwrap()
        .id;
    let last = table
        .allocate(
            ThinkTime::Seconds(10.0),
            ModuleId(2),
            AllocationPolicy::QUAKEWORLD,
        )
        .unwrap()
        .id;
    attach_collision(&mut table, first, CollisionOwner::Lifetime(last));
    attach_collision(&mut table, last, CollisionOwner::Lifetime(first));
    let replacement = table
        .allocate(
            ThinkTime::Seconds(10.0),
            ModuleId(2),
            AllocationPolicy::QUAKEWORLD,
        )
        .unwrap();
    assert_eq!(replacement.displaced, Some(last));
    assert_eq!(replacement.id.slot, last.slot);
    assert_ne!(replacement.id.generation, last.generation);
    assert!(table.resolve(last).is_none());
    assert_collision_cleared(&table, replacement.id.slot as usize);
    // Clearing the displaced row does not rewrite another entity's strong
    // owner into the new lifetime occupying that row.
    assert_eq!(
        table.columns.collision_owner[first.slot as usize],
        CollisionOwner::Lifetime(last)
    );
    assert_eq!(
        table.columns.collision_shape[first.slot as usize],
        CollisionShape::Box
    );
}

#[test]
fn client_reset_clears_collision_identity_without_releasing_its_reserved_slot() {
    let mut table = EntityTable::new(2, 2).unwrap();
    let client = table.id_at(1).unwrap();
    table.columns.owner[1] = ModuleId(7);
    attach_collision(
        &mut table,
        client,
        CollisionOwner::Native(NativeEntity {
            module: ModuleId(3),
            slot: 1023,
        }),
    );
    assert!(!table.release(client, ThinkTime::Seconds(10.0)));
    assert_eq!(table.columns.collision_shape[1], CollisionShape::Box);
    let next = table.reset_client(client).unwrap();
    assert_eq!(next.slot, client.slot);
    assert_ne!(next.generation, client.generation);
    assert!(table.resolve(client).is_none());
    assert_eq!(table.resolve(next), Some(1));
    assert_collision_cleared(&table, 1);
    assert_eq!(table.columns.owner[1], ModuleId::default());
    assert_eq!(table.len(), 2);
}

#[test]
fn native_weak_owner_keeps_its_slot_across_target_lifetime_reuse() {
    let mut table = EntityTable::new(4, 1).unwrap();
    let target = table
        .allocate(
            ThinkTime::Seconds(10.0),
            ModuleId(1),
            AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    let weak = table
        .allocate(
            ThinkTime::Seconds(10.0),
            ModuleId(2),
            AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    let strong = table
        .allocate(
            ThinkTime::Seconds(10.0),
            ModuleId(3),
            AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    let native_target = NativeEntity {
        module: ModuleId(1),
        slot: 29,
    };
    table.columns.native_entity[target.slot as usize] = Some(native_target);
    attach_collision(&mut table, weak, CollisionOwner::Native(native_target));
    attach_collision(&mut table, strong, CollisionOwner::Lifetime(target));
    assert!(table.release(target, ThinkTime::Seconds(10.0)));
    assert!(table.resolve(target).is_none());
    assert_eq!(
        table.columns.collision_owner[weak.slot as usize],
        CollisionOwner::Native(native_target)
    );
    let replacement = table
        .allocate(
            ThinkTime::Seconds(10.501),
            ModuleId(1),
            AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    assert_eq!(replacement.slot, target.slot);
    assert_ne!(replacement.generation, target.generation);
    table.columns.native_entity[replacement.slot as usize] = Some(native_target);
    // The native namespace uses29 although the engine table uses slot1.
    // No generation is inserted into the weak reference on either transition.
    assert_eq!(
        table.columns.collision_owner[weak.slot as usize],
        CollisionOwner::Native(table.columns.native_entity[replacement.slot as usize].unwrap())
    );
    assert_eq!(
        table.columns.collision_owner[strong.slot as usize],
        CollisionOwner::Lifetime(target)
    );
    assert_ne!(
        CollisionOwner::Lifetime(target),
        CollisionOwner::Lifetime(replacement)
    );
    assert_ne!(
        native_target,
        NativeEntity {
            module: ModuleId(2),
            slot: 29
        }
    );
}

#[test]
fn native_owner_sentinels_are_preserved_without_slot_or_lifetime_normalization() {
    let mut table = EntityTable::new(2, 1).unwrap();
    let entity = table
        .allocate(
            ThinkTime::Seconds(10.0),
            ModuleId(8),
            AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    let slot = entity.slot as usize;
    // Q1/Q2 world slot0, Q3 pass-owner -1, WORLD1022 and NONE1023 remain
    // distinct native values. Their meaning belongs to the caller's rules.
    for native_slot in [-1, 0, 1022, 1023] {
        let native = NativeEntity {
            module: ModuleId(3),
            slot: native_slot,
        };
        table.columns.collision_owner[slot] = CollisionOwner::Native(native);
        assert_eq!(
            table.columns.collision_owner[slot],
            CollisionOwner::Native(native)
        );
        assert_ne!(table.columns.collision_owner[slot], CollisionOwner::None);
        assert_eq!(table.columns.owner[slot], ModuleId(8));
    }
}
