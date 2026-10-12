use qa_core::primitives::ThinkTime;
use qa_core::primitives::{EntityId, ModuleId, NativeEntity};
use qa_world::entities::{AllocationPolicy, EntityTable};

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
    assert!(!table.release(world, ThinkTime::Seconds(0.0)));
    let client = table.id_at(63).ok_or("client")?;
    let reset = table.reset_client(client).ok_or("reset")?;
    assert_eq!(reset.slot, client.slot);
    assert_ne!(reset.generation, client.generation);
    assert!(table.resolve(client).is_none());
    assert!(table.resolve(reset).is_some());
    assert_eq!(table.len(), 65);
    assert!(table.set_never_free(id, false));
    assert!(table.release(id, ThinkTime::Seconds(0.0)));
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
