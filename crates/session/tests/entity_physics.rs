use qa_core::primitives::{ModuleId, Motion, ThinkTime, Vec3};
use qa_session::physics::{MotionWrite, PhysicsError, noclip};
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
