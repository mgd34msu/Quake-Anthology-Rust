use qa_core::primitives::{ModuleId, Motion, ThinkTime, Vec3};
use qa_session::physics::{MotionWrite, PhysicsError, VelocityCheck, check_velocity, noclip};
use qa_world::entities::{AllocationPolicy, EntityTable};

#[test]
fn motion_uses_shared_columns_in_native_component_order() {
    let mut entities = EntityTable::new(3, 1).unwrap();
    let entity = entities
        .allocate(
            ThinkTime::Seconds(0.0),
            ModuleId(4),
            AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    let slot = entity.slot as usize;
    let angles = Vec3([-123.25, -0.0, 31.0]);
    let angular = Vec3([0.75, -0.0, 8.125]);
    let origin = Vec3([1234.5679, -0.0, 0.25]);
    let velocity = Vec3([-23.125, 0.0, -3.125]);
    entities.columns.angles[slot] = angles;
    entities.columns.angular_velocity[slot] = Some(angular);
    entities.columns.position[slot] = origin;
    entities.columns.velocity[slot] = velocity;
    // Think may select the next frame's mode; the caller has already captured
    // this phase, and must not classify it a second time after the callback.
    entities.columns.motion[slot] = Motion::Toss;
    let mut writes = Vec::new();
    noclip::<PhysicsError>(&mut entities, entity, 0.333, |_, write| {
        writes.push(write);
        Ok(())
    })
    .unwrap();
    assert_eq!(
        writes,
        [
            MotionWrite::Angles(0),
            MotionWrite::Angles(1),
            MotionWrite::Angles(2),
            MotionWrite::Origin(0),
            MotionWrite::Origin(1),
            MotionWrite::Origin(2)
        ]
    );
    for (actual, start, speed) in [
        (entities.columns.angles[slot], angles, angular),
        (entities.columns.position[slot], origin, velocity),
    ] {
        assert_eq!(
            actual.0.map(f32::to_bits),
            std::array::from_fn::<_, 3, _>(|axis| {
                (start.0[axis] + 0.333f32 * speed.0[axis]).to_bits()
            })
        );
    }
}

#[test]
fn stale_or_unpublished_motion_cannot_write_the_entity() {
    let mut entities = EntityTable::new(2, 1).unwrap();
    let entity = entities
        .allocate(
            ThinkTime::Seconds(0.0),
            ModuleId(4),
            AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    assert_eq!(
        entities.columns.motion[entity.slot as usize],
        Motion::Unsupported
    );
    let mut writes = 0;
    assert_eq!(
        noclip::<PhysicsError>(&mut entities, entity, 1.0, |_, _| {
            writes += 1;
            Ok(())
        }),
        Err(PhysicsError::AngularVelocity)
    );
    assert_eq!(writes, 0);
    entities.release(entity, ThinkTime::Seconds(3.0));
    assert_eq!(
        entities.columns.angular_velocity[entity.slot as usize],
        None
    );
    assert_eq!(
        entities.columns.motion[entity.slot as usize],
        Motion::Unsupported
    );
    assert_eq!(
        noclip::<PhysicsError>(&mut entities, entity, 1.0, |_, _| {
            writes += 1;
            Ok(())
        }),
        Err(PhysicsError::Entity)
    );
    assert_eq!(writes, 0);
}

#[test]
fn native_boundary_failure_stops_before_later_components() {
    let mut entities = EntityTable::new(2, 1).unwrap();
    let entity = entities
        .allocate(
            ThinkTime::Seconds(0.0),
            ModuleId(4),
            AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    let slot = entity.slot as usize;
    entities.columns.angular_velocity[slot] = Some(Vec3([1.0; 3]));
    entities.columns.velocity[slot] = Vec3([2.0; 3]);
    let mut writes = Vec::new();
    assert_eq!(
        noclip::<PhysicsError>(&mut entities, entity, 1.0, |_, write| {
            writes.push(write);
            if write == MotionWrite::Angles(1) {
                Err(PhysicsError::Entity)
            } else {
                Ok(())
            }
        }),
        Err(PhysicsError::Entity)
    );
    assert_eq!(writes, [MotionWrite::Angles(0), MotionWrite::Angles(1)]);
    assert_eq!(entities.columns.position[slot], Vec3::default());
    assert_eq!(entities.columns.angles[slot], Vec3([1.0, 1.0, 0.0]));
}

#[test]
fn boundary_lifetime_change_stops_before_writing_the_replacement() {
    let mut entities = EntityTable::new(2, 1).unwrap();
    let entity = entities
        .allocate(
            ThinkTime::Seconds(0.0),
            ModuleId(4),
            AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    entities.columns.angular_velocity[entity.slot as usize] = Some(Vec3([1.0; 3]));
    let mut replacement = None;
    let mut writes = 0;
    assert_eq!(
        noclip::<PhysicsError>(&mut entities, entity, 1.0, |entities, _| {
            writes += 1;
            entities.release(entity, ThinkTime::Seconds(3.0));
            replacement = Some(
                entities
                    .allocate(
                        ThinkTime::Seconds(4.0),
                        ModuleId(4),
                        AllocationPolicy::EDICT,
                    )
                    .unwrap()
                    .id,
            );
            Ok(())
        }),
        Err(PhysicsError::Entity)
    );
    assert_eq!(writes, 1);
    let replacement = replacement.unwrap();
    assert_eq!(replacement.slot, entity.slot);
    assert_ne!(replacement.generation, entity.generation);
    assert_eq!(
        entities.columns.angles[replacement.slot as usize],
        Vec3::default()
    );
    assert_eq!(
        entities.columns.position[replacement.slot as usize],
        Vec3::default()
    );
}

#[test]
fn velocity_check_preserves_finite_bits_without_bounds_or_angular_fields() {
    let mut entities = EntityTable::new(2, 1).unwrap();
    let entity = entities
        .allocate(
            ThinkTime::Seconds(0.0),
            ModuleId(4),
            AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    let slot = entity.slot as usize;
    let origin = Vec3([-0.0, f32::MIN_POSITIVE, -12.5]);
    let velocity = Vec3([-0.0, 5.0, -5.0]);
    entities.columns.position[slot] = origin;
    entities.columns.velocity[slot] = velocity;
    entities.columns.mins[slot] = Vec3([f32::NAN; 3]);
    entities.columns.maxs[slot] = Vec3([f32::NEG_INFINITY; 3]);
    assert_eq!(entities.columns.angular_velocity[slot], None);
    let mut writes = 0;
    assert_eq!(
        check_velocity::<PhysicsError>(&mut entities, entity, 5.0, |_, _, _| {
            writes += 1;
            Ok(())
        }),
        Ok(VelocityCheck::default())
    );
    assert_eq!(writes, 0);
    assert_eq!(
        entities.columns.position[slot].0.map(f32::to_bits),
        origin.0.map(f32::to_bits)
    );
    assert_eq!(
        entities.columns.velocity[slot].0.map(f32::to_bits),
        velocity.0.map(f32::to_bits)
    );
}

#[test]
fn velocity_check_clears_nonfinite_scalars_in_native_axis_order() {
    let mut entities = EntityTable::new(2, 1).unwrap();
    let entity = entities
        .allocate(
            ThinkTime::Seconds(0.0),
            ModuleId(4),
            AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    let slot = entity.slot as usize;
    entities.columns.position[slot] = Vec3([f32::NEG_INFINITY, f32::NAN, f32::INFINITY]);
    entities.columns.velocity[slot] = Vec3([f32::NAN, f32::INFINITY, f32::NEG_INFINITY]);
    let mut writes = Vec::new();
    assert_eq!(
        check_velocity::<PhysicsError>(&mut entities, entity, 5.0, |_, write, invalid| {
            assert!(invalid);
            writes.push(write);
            Ok(())
        }),
        Ok(VelocityCheck {
            invalid_velocity: 0b111,
            invalid_origin: 0b111
        })
    );
    assert_eq!(
        writes,
        [
            MotionWrite::Velocity(0),
            MotionWrite::Origin(0),
            MotionWrite::Velocity(1),
            MotionWrite::Origin(1),
            MotionWrite::Velocity(2),
            MotionWrite::Origin(2),
        ]
    );
    assert_eq!(entities.columns.position[slot].0.map(f32::to_bits), [0; 3]);
    assert_eq!(entities.columns.velocity[slot].0.map(f32::to_bits), [0; 3]);
}

#[test]
fn velocity_limits_keep_native_comparison_order_and_boundary_values() {
    let mut entities = EntityTable::new(2, 1).unwrap();
    let entity = entities
        .allocate(
            ThinkTime::Seconds(0.0),
            ModuleId(4),
            AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    let slot = entity.slot as usize;
    for (max, input, expected, expected_writes) in [
        (5.0, [-6.0, 5.0, 6.0], [-5.0, 5.0, 5.0], vec![0, 2]),
        (-5.0, [0.0, -6.0, 6.0], [-5.0, 5.0, -5.0], vec![0, 1, 2]),
        (-0.0, [-0.0, -1.0, 1.0], [-0.0, 0.0, -0.0], vec![1, 2]),
        (f32::NAN, [-6.0, -0.0, 6.0], [-6.0, -0.0, 6.0], vec![]),
        (f32::INFINITY, [-6.0, -0.0, 6.0], [-6.0, -0.0, 6.0], vec![]),
        (
            f32::NEG_INFINITY,
            [-6.0, -0.0, 6.0],
            [f32::NEG_INFINITY; 3],
            vec![0, 1, 2],
        ),
    ] {
        entities.columns.velocity[slot] = Vec3(input);
        let mut writes = Vec::new();
        assert_eq!(
            check_velocity::<PhysicsError>(&mut entities, entity, max, |_, write, invalid| {
                assert!(!invalid);
                writes.push(write);
                Ok(())
            }),
            Ok(VelocityCheck::default())
        );
        assert_eq!(
            entities.columns.velocity[slot].0.map(f32::to_bits),
            expected.map(f32::to_bits)
        );
        assert_eq!(
            writes,
            expected_writes
                .into_iter()
                .map(MotionWrite::Velocity)
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn velocity_check_rereads_origin_and_velocity_after_each_native_write() {
    let mut entities = EntityTable::new(2, 1).unwrap();
    let entity = entities
        .allocate(
            ThinkTime::Seconds(0.0),
            ModuleId(4),
            AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    let slot = entity.slot as usize;
    entities.columns.velocity[slot] = Vec3([f32::NAN, 1.0, 2.0]);
    let mut writes = Vec::new();
    let result =
        check_velocity::<PhysicsError>(&mut entities, entity, 5.0, |entities, write, invalid| {
            writes.push(write);
            assert_eq!(invalid, writes.len() <= 2);
            match writes.len() {
                1 => entities.columns.position[slot].0[0] = f32::NAN,
                2 => entities.columns.velocity[slot].0[0] = 99.0,
                _ => {}
            }
            Ok(())
        })
        .unwrap();
    assert_eq!(
        result,
        VelocityCheck {
            invalid_velocity: 1,
            invalid_origin: 1
        }
    );
    assert_eq!(
        writes,
        [
            MotionWrite::Velocity(0),
            MotionWrite::Origin(0),
            MotionWrite::Velocity(0)
        ]
    );
    assert_eq!(entities.columns.position[slot], Vec3::default());
    assert_eq!(entities.columns.velocity[slot], Vec3([5.0, 1.0, 2.0]));
}

#[test]
fn velocity_check_preserves_shifted_native_field_aliases() {
    let mut entities = EntityTable::new(2, 1).unwrap();
    let entity = entities
        .allocate(
            ThinkTime::Seconds(0.0),
            ModuleId(4),
            AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    let slot = entity.slot as usize;
    // origin starts one native word before velocity. A velocity write can
    // repair a later origin scalar; an origin write can repair prior velocity.
    let mut native = [f32::NAN, f32::NAN, 8.0, -8.0];
    entities.columns.position[slot] = Vec3([native[0], native[1], native[2]]);
    entities.columns.velocity[slot] = Vec3([native[1], native[2], native[3]]);
    let mut writes = Vec::new();
    let mut diagnostics = Vec::new();
    let result =
        check_velocity::<PhysicsError>(&mut entities, entity, 5.0, |entities, write, invalid| {
            writes.push(write);
            if invalid {
                // This stands for an aliased native classname cell. The boundary
                // reads it before committing the scalar that changes its bytes.
                diagnostics.push((write, native[1].to_bits()));
            }
            match write {
                MotionWrite::Velocity(axis) => {
                    native[axis + 1] = entities.columns.velocity[slot].0[axis]
                }
                MotionWrite::Origin(axis) => native[axis] = entities.columns.position[slot].0[axis],
                MotionWrite::Angles(_) | MotionWrite::AngularVelocity(_) => panic!("velocity checking must not write angles"),
            }
            entities.columns.position[slot] = Vec3([native[0], native[1], native[2]]);
            entities.columns.velocity[slot] = Vec3([native[1], native[2], native[3]]);
            Ok(())
        })
        .unwrap();
    assert_eq!(
        result,
        VelocityCheck {
            invalid_velocity: 1,
            invalid_origin: 1
        }
    );
    assert_eq!(
        writes,
        [
            MotionWrite::Velocity(0),
            MotionWrite::Origin(0),
            MotionWrite::Velocity(1),
            MotionWrite::Velocity(2)
        ]
    );
    assert_eq!(
        diagnostics,
        [
            (MotionWrite::Velocity(0), f32::NAN.to_bits()),
            (MotionWrite::Origin(0), 0.0f32.to_bits()),
        ]
    );
    assert_eq!(
        native.map(f32::to_bits),
        [0.0f32, 0.0, 5.0, -5.0].map(f32::to_bits)
    );
    assert_eq!(entities.columns.position[slot], Vec3([0.0, 0.0, 5.0]));
    assert_eq!(entities.columns.velocity[slot], Vec3([0.0, 5.0, -5.0]));
}

#[test]
fn velocity_check_rejects_a_stale_lifetime_without_writes() {
    let mut entities = EntityTable::new(2, 1).unwrap();
    let entity = entities
        .allocate(
            ThinkTime::Seconds(0.0),
            ModuleId(4),
            AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    assert!(entities.release(entity, ThinkTime::Seconds(3.0)));
    let mut writes = 0;
    assert_eq!(
        check_velocity::<PhysicsError>(&mut entities, entity, 5.0, |_, _, _| {
            writes += 1;
            Ok(())
        }),
        Err(PhysicsError::Entity)
    );
    assert_eq!(writes, 0);
}

#[test]
fn velocity_check_rejects_reuse_after_first_or_final_scalar_write() {
    for axis in [0, 2] {
        let mut entities = EntityTable::new(2, 1).unwrap();
        let entity = entities
            .allocate(
                ThinkTime::Seconds(0.0),
                ModuleId(4),
                AllocationPolicy::EDICT,
            )
            .unwrap()
            .id;
        entities.columns.velocity[entity.slot as usize].0[axis] = 6.0;
        let mut replacement = None;
        let mut writes = Vec::new();
        assert_eq!(
            check_velocity::<PhysicsError>(
                &mut entities,
                entity,
                5.0,
                |entities, write, invalid| {
                    assert!(!invalid);
                    writes.push(write);
                    assert!(entities.release(entity, ThinkTime::Seconds(3.0)));
                    let next = entities
                        .allocate(
                            ThinkTime::Seconds(4.0),
                            ModuleId(4),
                            AllocationPolicy::EDICT,
                        )
                        .unwrap()
                        .id;
                    entities.columns.position[next.slot as usize] = Vec3([12.0; 3]);
                    entities.columns.velocity[next.slot as usize] = Vec3([99.0; 3]);
                    replacement = Some(next);
                    Ok(())
                }
            ),
            Err(PhysicsError::Entity)
        );
        assert_eq!(writes, [MotionWrite::Velocity(axis)]);
        let replacement = replacement.unwrap();
        assert_eq!(replacement.slot, entity.slot);
        assert_ne!(replacement.generation, entity.generation);
        assert_eq!(
            entities.columns.position[replacement.slot as usize],
            Vec3([12.0; 3])
        );
        assert_eq!(
            entities.columns.velocity[replacement.slot as usize],
            Vec3([99.0; 3])
        );
    }
}

#[test]
fn velocity_check_stops_on_boundary_error_before_clamping_or_later_axes() {
    let mut entities = EntityTable::new(2, 1).unwrap();
    let entity = entities
        .allocate(
            ThinkTime::Seconds(0.0),
            ModuleId(4),
            AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    let slot = entity.slot as usize;
    entities.columns.position[slot] = Vec3([f32::NAN, 10.0, 20.0]);
    entities.columns.velocity[slot] = Vec3([f32::NAN, 9.0, -9.0]);
    let mut writes = Vec::new();
    assert_eq!(
        check_velocity::<PhysicsError>(&mut entities, entity, 5.0, |entities, write, invalid| {
            assert!(invalid);
            writes.push(write);
            if write == MotionWrite::Origin(0) {
                entities.columns.velocity[slot].0[0] = 99.0;
                Err(PhysicsError::Entity)
            } else {
                Ok(())
            }
        }),
        Err(PhysicsError::Entity)
    );
    assert_eq!(writes, [MotionWrite::Velocity(0), MotionWrite::Origin(0)]);
    assert_eq!(entities.columns.position[slot], Vec3([0.0, 10.0, 20.0]));
    assert_eq!(entities.columns.velocity[slot], Vec3([99.0, 9.0, -9.0]));
}

#[test]
fn shared_noclip_ma_rereads_shifted_native_position_velocity_after_each_axis() {
    let mut entities = EntityTable::new(2, 1).unwrap();
    let entity = entities
        .allocate(
            ThinkTime::Seconds(0.0),
            ModuleId(4),
            AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    let slot = entity.slot as usize;
    let mut words = [2.0_f32, 3.0, 4.0, 5.0];
    entities.columns.position[slot] = Vec3([words[1], words[2], words[3]]);
    entities.columns.velocity[slot] = Vec3([words[0], words[1], words[2]]);
    entities.columns.angular_velocity[slot] = Some(Vec3::default());
    let mut origins = Vec::new();
    noclip::<PhysicsError>(&mut entities, entity, 2.0, |entities, write| {
        if let MotionWrite::Origin(axis) = write {
            words[axis + 1] = entities.columns.position[slot].0[axis];
            entities.columns.position[slot] = Vec3([words[1], words[2], words[3]]);
            entities.columns.velocity[slot] = Vec3([words[0], words[1], words[2]]);
            origins.push(axis);
        }
        Ok(())
    })
    .unwrap();
    assert_eq!(words, [2.0, 7.0, 18.0, 41.0]);
    assert_eq!(origins, [0, 1, 2]);
}

#[test]
fn final_noclip_scalar_callback_must_preserve_the_captured_lifetime() {
    let mut entities = EntityTable::new(2, 1).unwrap();
    let entity = entities
        .allocate(
            ThinkTime::Seconds(0.0),
            ModuleId(4),
            AllocationPolicy::EDICT,
        )
        .unwrap()
        .id;
    let slot = entity.slot as usize;
    entities.columns.angular_velocity[slot] = Some(Vec3::default());
    entities.columns.velocity[slot] = Vec3([1.0; 3]);
    let mut replacement = None;
    assert_eq!(
        noclip::<PhysicsError>(&mut entities, entity, 1.0, |entities, write| {
            if write == MotionWrite::Origin(2) {
                assert!(entities.release(entity, ThinkTime::Seconds(3.0)));
                replacement = Some(
                    entities
                        .allocate(
                            ThinkTime::Seconds(4.0),
                            ModuleId(4),
                            AllocationPolicy::EDICT,
                        )
                        .unwrap()
                        .id,
                );
            }
            Ok(())
        }),
        Err(PhysicsError::Entity)
    );
    let replacement = replacement.unwrap();
    assert_eq!(replacement.slot, entity.slot);
    assert_ne!(replacement.generation, entity.generation);
    assert_eq!(
        entities.columns.position[replacement.slot as usize],
        Vec3::default()
    );
}
