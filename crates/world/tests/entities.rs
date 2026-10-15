use qa_core::primitives::ThinkTime;
use qa_core::primitives::{EntityId, ModuleId, NativeEntity};
use qa_world::entities::{AllocationPolicy, EntityTable};

#[test]
fn captured_native_reference_authenticates_its_pair_before_callbacks() {
    use qa_core::primitives::{NativeEntityRef, ResourceNamespace};

    let mut table = EntityTable::new(3, 1).unwrap();
    let id = table
        .allocate(
            ThinkTime::Seconds(0.0),
            ModuleId(7),
            AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    let slot = id.slot as usize;
    assert!(table.capture_reference(id).is_none());
    table.columns.native_entity[slot] = Some(NativeEntity {
        module: ModuleId(8),
        slot: 0,
    });
    assert!(table.capture_reference(id).is_none());
    table.columns.native_entity[slot] = Some(NativeEntity {
        module: ModuleId(7),
        slot: -1,
    });
    assert!(table.capture_reference(id).is_none());
    table.columns.native_entity[slot] = Some(NativeEntity {
        module: ModuleId(7),
        slot: 0,
    });
    let captured = table.capture_reference(id).unwrap();
    assert_eq!(captured.entity(), id);
    assert_eq!(
        captured.reference(),
        NativeEntityRef::entity(ResourceNamespace::Module(ModuleId(7)), 0)
    );
    assert!(table.release(id, ThinkTime::Seconds(0.0)));
    assert!(table.capture_reference(id).is_none());
    let replacement = table
        .allocate(
            ThinkTime::Seconds(1.0),
            ModuleId(7),
            AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    assert_eq!(replacement.slot, id.slot);
    assert_ne!(replacement, captured.entity());
    assert_eq!(captured.entity(), id);
}

#[test]
fn published_fields_follow_actual_writes_and_end_with_the_entity_lifetime() {
    use qa_core::primitives::{Body, EntityFields, Vec3};

    let mut table = EntityTable::new(4, 2).unwrap();
    let client = table.id_at(1).unwrap();
    let id = table
        .allocate(
            ThinkTime::Seconds(0.0),
            ModuleId(1),
            AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    let slot = id.slot as usize;
    assert_eq!(
        table.columns.published_fields[slot],
        EntityFields::default()
    );

    table.columns.set_position(slot, Vec3::default());
    assert_eq!(table.columns.published_fields[slot], EntityFields::POSITION);
    assert!(!table.columns.published_fields[slot].contains(EntityFields::VELOCITY));
    table
        .columns
        .set_bounds(slot, Vec3([-1.0; 3]), Vec3([1.0; 3]));
    table.columns.set_angles(slot, Vec3::default());
    assert!(table.columns.published_fields[slot].contains(EntityFields::BOUNDS));
    assert!(table.columns.published_fields[slot].contains(EntityFields::ANGLES));
    assert!(!table.columns.published_fields[slot].contains(EntityFields::VELOCITY));
    table.columns.set_velocity(slot, Vec3::default());
    assert!(table.columns.published_fields[slot].contains(EntityFields::BODY));

    assert!(table.release(id, ThinkTime::Seconds(0.0)));
    let replacement = table
        .allocate(
            ThinkTime::Seconds(1.0),
            ModuleId(1),
            AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    assert_eq!(replacement.slot, id.slot);
    assert_ne!(replacement.generation, id.generation);
    assert_eq!(
        table.columns.published_fields[slot],
        EntityFields::default()
    );

    table
        .columns
        .set_body(client.slot as usize, Body::default());
    assert!(table.columns.published_fields[client.slot as usize].contains(EntityFields::BODY));
    let client = table.reset_client(client).unwrap();
    assert_eq!(
        table.columns.published_fields[client.slot as usize],
        EntityFields::default()
    );
}

#[test]
fn client_classification_is_a_common_lifetime_role_not_a_native_ordinal() {
    use qa_core::primitives::CollisionTags;
    let mut table = EntityTable::new(6, 3).unwrap();
    let world = table.id_at(0).unwrap();
    let client = table.id_at(2).unwrap();
    let body = table
        .allocate(
            ThinkTime::Seconds(0.0),
            ModuleId(2),
            AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    table.columns.native_entity[client.slot as usize] = Some(NativeEntity {
        module: ModuleId(8),
        slot: 0,
    });
    table.columns.native_entity[body.slot as usize] = Some(NativeEntity {
        module: ModuleId(9),
        slot: 1,
    });
    assert_eq!(table.client_index(world), None);
    assert_eq!(table.client_index(client), Some(1));
    assert_eq!(table.client_index(body), None);
    assert_eq!(
        table.collision_tags(world, CollisionTags::MONSTER),
        Some(CollisionTags::MONSTER)
    );
    assert_eq!(
        table.collision_tags(client, CollisionTags::MONSTER),
        Some(CollisionTags(
            CollisionTags::PLAYER.0 | CollisionTags::MONSTER.0
        ))
    );
    assert_eq!(
        table.collision_tags(body, CollisionTags::PROJECTILE),
        Some(CollisionTags::PROJECTILE)
    );
    let replacement = table.reset_client(client).unwrap();
    assert_eq!(table.client_index(client), None);
    assert_eq!(table.collision_tags(client, CollisionTags::default()), None);
    assert_eq!(table.client_index(replacement), Some(1));
    assert_eq!(
        table.collision_tags(replacement, CollisionTags::default()),
        Some(CollisionTags::PLAYER)
    );
    // A fresh native view replaces old native flags rather than retaining them.
    assert_eq!(
        table.collision_tags(body, CollisionTags::default()),
        Some(CollisionTags::default())
    );
}

#[test]
fn native_lookup_keeps_module_identity_exclusion_and_live_generation() -> Result<(), &'static str> {
    let mut table = EntityTable::new(4, 1).map_err(|_| "table")?;
    let native = NativeEntity {
        module: ModuleId(1),
        slot: 2,
    };
    let other = NativeEntity {
        module: ModuleId(2),
        ..native
    };
    let now = ThinkTime::Seconds(3.0);
    let first = table
        .allocate(now, native.module, AllocationPolicy::EDICT)
        .ok_or("first")?
        .id;
    let second = table
        .allocate(now, other.module, AllocationPolicy::EDICT)
        .ok_or("second")?
        .id;
    table.columns.native_entity[first.slot as usize] = Some(native);
    table.columns.native_entity[second.slot as usize] = Some(other);
    assert_eq!(table.find_native(native, None), Some(first));
    assert_eq!(table.find_native(other, None), Some(second));
    assert_eq!(table.find_native(native, Some(first)), None);
    // Conflict detection must find a later duplicate even when self is first.
    table.columns.native_entity[second.slot as usize] = Some(native);
    assert_eq!(table.find_native(native, Some(first)), Some(second));
    table.columns.native_entity[second.slot as usize] = Some(other);
    assert!(table.release(first, now));
    assert_eq!(table.find_native(native, None), None);
    let replacement = table
        .allocate(
            ThinkTime::Seconds(4.0),
            native.module,
            AllocationPolicy::EDICT,
        )
        .ok_or("replacement")?
        .id;
    assert_eq!(replacement.slot, first.slot);
    assert_ne!(replacement.generation, first.generation);
    table.columns.native_entity[replacement.slot as usize] = Some(native);
    assert_eq!(table.find_native(native, None), Some(replacement));
    assert!(table.resolve(first).is_none());
    Ok(())
}

#[test]
fn ascending_lifetimes_cross_word_and_capacity_boundaries() -> Result<(), &'static str> {
    for capacity in [1, 63, 64, 65, 127, 128, 129, 8192] {
        let reserved = capacity.min(2);
        let mut table = EntityTable::new(capacity, reserved).map_err(|_| "table")?;
        let mut expected: Vec<_> = (0..reserved)
            .map(|slot| EntityId {
                slot: slot as u32,
                generation: 1,
            })
            .collect();
        for slot in reserved..capacity {
            let id = table
                .allocate(
                    ThinkTime::Seconds(0.0),
                    ModuleId(1),
                    AllocationPolicy::EDICT,
                )
                .ok_or("allocation")?
                .id;
            assert_eq!(id.slot as usize, slot);
            expected.push(id);
        }
        assert_eq!(table.active().collect::<Vec<_>>(), expected);
        assert_eq!(table.len(), capacity);
        assert!(
            table
                .allocate(
                    ThinkTime::Seconds(0.0),
                    ModuleId(1),
                    AllocationPolicy::EDICT
                )
                .is_none()
        );
        expected.retain(|id| {
            if id.slot as usize >= reserved && id.slot % 3 == 1 {
                assert!(table.release(*id, ThinkTime::Seconds(0.0)));
                assert!(table.resolve(*id).is_none());
                false
            } else {
                true
            }
        });
        assert_eq!(table.active().collect::<Vec<_>>(), expected);
        assert_eq!(table.len(), expected.len());
        for start in [0, 1, 62, 63, 64, 65, 127, 128, capacity, usize::MAX] {
            assert_eq!(
                table.next_active(start),
                expected
                    .iter()
                    .copied()
                    .find(|id| id.slot as usize >= start)
            );
        }
        assert!(table.id_at(capacity).is_none());
        assert!(table.id_at(usize::MAX).is_none());
    }
    Ok(())
}

#[test]
fn reserved_prefix_and_never_free_do_not_become_allocatable() -> Result<(), &'static str> {
    let mut table = EntityTable::new(65, 64).map_err(|_| "table")?;
    let id = table
        .allocate(
            ThinkTime::Seconds(0.0),
            ModuleId(1),
            AllocationPolicy::QUAKEWORLD,
        )
        .ok_or("entity")?
        .id;
    assert_eq!(id.slot, 64);
    assert!(table.set_never_free(id, true));
    assert!(!table.can_release(id));
    assert!(!table.release(id, ThinkTime::Seconds(0.0)));
    assert!(
        table
            .allocate(
                ThinkTime::Seconds(0.0),
                ModuleId(1),
                AllocationPolicy::QUAKEWORLD
            )
            .is_none()
    );
    let world = table.id_at(0).ok_or("world")?;
    assert!(!table.can_release(world));
    assert!(!table.release(world, ThinkTime::Seconds(0.0)));
    let client = table.id_at(63).ok_or("client")?;
    let reset = table.reset_client(client).ok_or("reset")?;
    assert_eq!(reset.slot, client.slot);
    assert_ne!(reset.generation, client.generation);
    assert!(table.resolve(client).is_none());
    assert!(table.resolve(reset).is_some());
    assert_eq!(table.len(), 65);
    assert!(table.set_never_free(id, false));
    assert!(table.can_release(id));
    assert!(table.release(id, ThinkTime::Seconds(0.0)));
    assert!(!table.can_release(id));
    let reused = table
        .allocate(
            ThinkTime::Seconds(0.0),
            ModuleId(1),
            AllocationPolicy::EDICT,
        )
        .ok_or("reuse")?
        .id;
    assert_eq!(reused.slot, 64);
    assert_ne!(reused.generation, id.generation);
    assert!(table.resolve(id).is_none());
    Ok(())
}
