use qa_core::{
    effects::{RandomSample, VelocityChange},
    names::NameTable,
    primitives::{EffectId, RuleSetId, Vec3},
};
use qa_gameplay::{effects::native_catalog, registry::Registry};
use std::sync::Arc;

#[test]
fn native_particle_binding_admits_only_the_actual_netquake_abi() {
    let catalog = native_catalog();
    assert_eq!(catalog.len(), 2);
    assert!(!catalog.is_empty());
    assert!(catalog.definition(EffectId(0)).is_none());
    assert!(catalog.definition(EffectId(3)).is_none());
    assert!(catalog.definition(EffectId(u32::MAX)).is_none());
    let native = catalog.native_particles(RuleSetId::Quake).unwrap();
    assert_ne!(native.directional, native.explosion);
    for source in RuleSetId::ALL {
        if source != RuleSetId::Quake {
            assert!(catalog.native_particles(source).is_none());
        }
    }
    for definition in catalog.definitions() {
        assert_eq!(definition.source, RuleSetId::Quake);
        assert!(!definition.variants.is_empty());
    }
}

#[test]
fn directional_properties_preserve_native_mask_bias_and_double_duration() {
    let catalog = native_catalog();
    let native = catalog.native_particles(RuleSetId::Quake).unwrap();
    let event = native.event(Vec3([1.0, 2.0, 3.0]), Vec3([4.0, 5.0, 6.0]), 226, 20);
    let definition = catalog.definition(event.effect).unwrap();
    let emission = definition.emission;
    assert_eq!(emission.lifetime.seconds.to_bits(), 0.1_f64.to_bits());
    let lifetime = emission.lifetime.random.unwrap();
    assert!(matches!(lifetime, RandomSample::Modulo(divisor) if divisor.get() == 5));
    assert_eq!(lifetime.sample(0), 0);
    assert_eq!(lifetime.sample(9), 4);
    assert_eq!(emission.color.event_mask, Some(248));
    assert_eq!(emission.color.base, 0);
    assert_eq!(emission.color.random.unwrap().sample(15), 7);
    assert_eq!(emission.position.sample(0), -8);
    assert_eq!(emission.position.sample(15), 7);
    assert_eq!(emission.position.sample(16), -8);
    assert_eq!(emission.velocity.direction_scale, 15.0);
    assert_eq!(emission.velocity.random, None);
    assert_eq!(emission.ramp, None);
    let motion = definition.variants[0];
    assert_eq!(motion.ramp, None);
    assert_eq!(motion.velocity, VelocityChange::None);
    assert_eq!(motion.gravity_scale.to_bits(), 0.05_f64.to_bits());
    assert_eq!(motion.alpha_rate, 0.0);
}

#[test]
fn explosion_properties_keep_even_and_odd_native_motion_and_palette_ramps() {
    let catalog = native_catalog();
    let native = catalog.native_particles(RuleSetId::Quake).unwrap();
    let event = native.event(Vec3::default(), Vec3([9.0, 8.0, 7.0]), 226, 255);
    assert_eq!(event.count, 1024);
    let definition = catalog.definition(event.effect).unwrap();
    let emission = definition.emission;
    assert_eq!(emission.lifetime.seconds, 5.0);
    assert_eq!(emission.lifetime.random, None);
    assert_eq!(emission.color.event_mask, None);
    assert_eq!(emission.color.base, 0x6f);
    assert_eq!(emission.color.random, None);
    assert_eq!(emission.ramp.unwrap().sample(7), 3);
    assert_eq!(emission.position.sample(0), -16);
    assert_eq!(emission.position.sample(31), 15);
    assert_eq!(emission.position.sample(32), -16);
    assert_eq!(emission.velocity.direction_scale, 0.0);
    let velocity = emission.velocity.random.unwrap();
    assert_eq!(velocity.sample(0), -256);
    assert_eq!(velocity.sample(511), 255);
    assert_eq!(velocity.sample(512), -256);
    assert_eq!(definition.variants.len(), 2);
    let even = definition.variants[0];
    let odd = definition.variants[1];
    assert_eq!(even.velocity, VelocityChange::SubtractScaled(1.0));
    assert_eq!(odd.velocity, VelocityChange::AddScaled(4.0));
    assert_eq!(even.ramp.unwrap().rate, 15.0);
    assert_eq!(odd.ramp.unwrap().rate, 10.0);
    assert_eq!(
        even.ramp.unwrap().colors,
        [0x6f, 0x6e, 0x6d, 0x6c, 0x6b, 0x6a, 0x68, 0x66]
    );
    assert_eq!(
        odd.ramp.unwrap().colors,
        [0x6f, 0x6d, 0x6b, 0x69, 0x67, 0x65, 0x63, 0x61]
    );
    assert_eq!(even.gravity_scale, odd.gravity_scale);
}

#[test]
fn registry_presents_one_shared_immutable_catalog() {
    let names = NameTable::load(Registry::names_needed()).unwrap();
    let registry = Registry::load(&names).unwrap();
    let renderer = Arc::clone(&registry.effects);
    assert!(Arc::ptr_eq(&renderer, &registry.effects));
    assert_eq!(renderer.len(), 2);
    let native = renderer.native_particles(RuleSetId::Quake).unwrap();
    assert_eq!(
        renderer.definition(native.directional).unwrap(),
        registry.effects.definition(native.directional).unwrap()
    );
}
