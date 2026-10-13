use qa_core::{
    effects::{
        EffectCatalog, NativeParticleBindings, ParticleColor, ParticleDefinition, ParticleEmission,
        ParticleLifetime, ParticleMotion, ParticleRamp, ParticleVelocity, RandomOffset,
        RandomSample, VelocityChange,
    },
    primitives::{ClientId, EffectEvent, EffectId, EntityId, RuleSetId, Vec3},
    sys_events::EventTime,
};
use qa_render::{
    Assets, Command, FrontEnd, Limits, MaterialId, PaletteId, ParticlePresentation, Refdef,
    effects::{EffectClient, Effects, ParticleBinding, ParticleView},
    particles::register_point_material,
    surface_cache::PaletteLighting,
};
use std::{num::NonZeroU32, sync::Arc};

const CLIENT: ClientId = ClientId(1);
const LIFE: EntityId = EntityId {
    slot: 7,
    generation: 1,
};
const RAMP1: [u8; 8] = [0x6f, 0x6d, 0x6b, 0x69, 0x67, 0x65, 0x63, 0x61];
const RAMP2: [u8; 8] = [0x6f, 0x6e, 0x6d, 0x6c, 0x6b, 0x6a, 0x68, 0x66];
const DIRECTIONAL: [ParticleMotion; 1] = [ParticleMotion {
    ramp: None,
    velocity: VelocityChange::None,
    gravity_scale: 0.05,
    alpha_rate: 0.0,
}];
const EXPLOSION: [ParticleMotion; 2] = [
    ParticleMotion {
        ramp: Some(ParticleRamp {
            rate: 15.0,
            colors: &RAMP2,
        }),
        velocity: VelocityChange::SubtractScaled(1.0),
        gravity_scale: 0.05,
        alpha_rate: 0.0,
    },
    ParticleMotion {
        ramp: Some(ParticleRamp {
            rate: 10.0,
            colors: &RAMP1,
        }),
        velocity: VelocityChange::AddScaled(4.0),
        gravity_scale: 0.05,
        alpha_rate: 0.0,
    },
];

fn modulo(value: u32) -> RandomSample {
    RandomSample::Modulo(NonZeroU32::new(value).unwrap())
}
fn catalog() -> Arc<EffectCatalog> {
    let directional = ParticleDefinition {
        source: RuleSetId::Quake,
        emission: ParticleEmission {
            lifetime: ParticleLifetime {
                seconds: 0.1,
                random: Some(modulo(5)),
            },
            color: ParticleColor {
                event_mask: Some(!7),
                base: 0,
                random: Some(RandomSample::Mask(7)),
            },
            ramp: None,
            position: RandomOffset {
                sample: RandomSample::Mask(15),
                bias: -8,
            },
            velocity: ParticleVelocity {
                direction_scale: 15.0,
                random: None,
            },
        },
        variants: &DIRECTIONAL,
    };
    let explosion = ParticleDefinition {
        source: RuleSetId::Quake,
        emission: ParticleEmission {
            lifetime: ParticleLifetime {
                seconds: 5.0,
                random: None,
            },
            color: ParticleColor {
                event_mask: None,
                base: 0x6f,
                random: None,
            },
            ramp: Some(RandomSample::Mask(3)),
            position: RandomOffset {
                sample: modulo(32),
                bias: -16,
            },
            velocity: ParticleVelocity {
                direction_scale: 0.0,
                random: Some(RandomOffset {
                    sample: modulo(512),
                    bias: -256,
                }),
            },
        },
        variants: &EXPLOSION,
    };
    Arc::new(EffectCatalog::new(
        vec![directional, explosion].into_boxed_slice(),
        [
            Some(NativeParticleBindings {
                directional: EffectId(1),
                explosion: EffectId(2),
            }),
            None,
            None,
            None,
            None,
        ],
    ))
}
fn binding(palette: PaletteId, material: MaterialId) -> ParticleBinding {
    ParticleBinding {
        palette,
        material,
        presentation: ParticlePresentation::Square { pixel_aspect: 1.5 },
    }
}
fn effects(capacity: usize) -> Effects {
    let mut effects = Effects::load(
        catalog(),
        capacity,
        3,
        &[
            None,
            Some(binding(PaletteId(0), MaterialId(0))),
            Some(binding(PaletteId(0), MaterialId(0))),
        ],
    )
    .unwrap();
    effects.synchronize(&[Some(EffectClient {
        client: CLIENT,
        lifetime: LIFE,
    })]);
    effects
}
fn event(effect: u32, count: u16) -> EffectEvent {
    EffectEvent {
        effect: EffectId(effect),
        count,
        client: Some(CLIENT),
        client_lifetime: Some(LIFE),
        ..EffectEvent::default()
    }
}
fn state(effects: &Effects) -> Vec<ParticleView> {
    let mut result = Vec::new();
    effects.visit(CLIENT, LIFE, |particle| result.push(particle));
    result
}
fn bits(value: Vec3) -> [u32; 3] {
    value.0.map(f32::to_bits)
}

#[test]
fn directional_emission_consumes_native_words_in_order_and_zero_count_consumes_none() {
    let mut effects = effects(1);
    let mut words = [4, 5, 1, 8, 15].into_iter();
    assert!(effects.accept_with_random(
        EffectEvent {
            position: Vec3([10.0, 20.0, 30.0]),
            direction: Vec3([1.0, -2.0, 0.5]),
            color: 22,
            ..event(1, 1)
        },
        || words.next().unwrap()
    ));
    assert_eq!(words.next(), None);
    let particle = state(&effects)[0];
    assert_eq!(particle.position, Vec3([3.0, 20.0, 37.0]));
    assert_eq!(particle.velocity, Vec3([15.0, -30.0, 7.5]));
    assert_eq!(particle.color, 21);
    assert_eq!(particle.death.to_bits(), 0.4_f32.to_bits());
    assert!(effects.accept_with_random(event(1, 0), || panic!("zero count consumed random")));
    assert_eq!(effects.stats.spawned, 1);
    assert_eq!(effects.stats.accepted_events, 2);
}

#[test]
fn explosion_variants_keep_native_order_and_draw_before_one_incremental_update() {
    let mut effects = effects(2);
    assert!(effects.begin_frame(CLIENT, LIFE, EventTime(0), 800.0));
    assert!(effects.finish_frame(CLIENT, LIFE, EventTime(0), 800.0));
    let mut words = [0, 16, 276, 16, 296, 16, 316, 1, 18, 277, 20, 297, 22, 317].into_iter();
    assert!(effects.accept_with_random(
        EffectEvent {
            direction: Vec3([f32::NAN; 3]),
            ..event(2, 1024)
        },
        || words.next().unwrap()
    ));
    assert_eq!(words.next(), None);
    assert_eq!(effects.stats.dropped_particles, 1022);
    let before = state(&effects);
    assert_eq!(before[0].position, Vec3([2.0, 4.0, 6.0]));
    assert_eq!(before[0].velocity, Vec3([21.0, 41.0, 61.0]));
    assert_eq!(before[1].position, Vec3([0.0; 3]));
    assert_eq!(before[1].velocity, Vec3([20.0, 40.0, 60.0]));
    let time = EventTime(100_000_000);
    assert!(effects.begin_frame(CLIENT, LIFE, time, 800.0));
    assert_eq!(bits(state(&effects)[0].position), bits(before[0].position));
    assert!(effects.finish_frame(CLIENT, LIFE, time, 800.0));
    let after = state(&effects);
    let step = 0.1_f32;
    let grav = (f64::from(step * 800.0) * 0.05) as f32;
    assert_eq!(
        bits(after[0].position),
        bits(Vec3([
            2.0 + 21.0 * step,
            4.0 + 41.0 * step,
            6.0 + 61.0 * step
        ]))
    );
    assert_eq!(
        bits(after[0].velocity),
        bits(Vec3([
            21.0 + 21.0 * (4.0 * step),
            41.0 + 41.0 * (4.0 * step),
            61.0 + 61.0 * (4.0 * step) - grav
        ]))
    );
    assert_eq!(
        bits(after[1].velocity),
        bits(Vec3([
            20.0 - 20.0 * step,
            40.0 - 40.0 * step,
            60.0 - 60.0 * step - grav
        ]))
    );
    assert_eq!((after[0].color, after[1].color), (0x6b, 0x6e));
    assert!(!effects.finish_frame(CLIENT, LIFE, time, 800.0));
    assert_eq!(bits(state(&effects)[0].position), bits(after[0].position));
}

#[test]
fn native_gravity_promotes_only_the_last_unsuffixed_literal_product() {
    let mut effects = effects(1);
    assert!(effects.begin_frame(CLIENT, LIFE, EventTime(0), 18.0));
    assert!(effects.finish_frame(CLIENT, LIFE, EventTime(0), 18.0));
    let mut words = [0, 16, 256, 16, 256, 16, 256].into_iter();
    assert!(effects.accept_with_random(event(2, 1), || words.next().unwrap()));
    assert!(effects.begin_frame(CLIENT, LIFE, EventTime(500_000_000), 18.0));
    assert!(effects.finish_frame(CLIENT, LIFE, EventTime(500_000_000), 18.0));
    assert_eq!(
        state(&effects)[0].velocity.0[2].to_bits(),
        (-0.45_f32).to_bits()
    );
    assert_ne!((9.0_f32 * 0.05_f32).to_bits(), 0.45_f32.to_bits());
}

#[test]
fn bounded_admission_retains_deadline_ties_and_retires_only_stale_client_lifetimes() {
    let mut effects = effects(1);
    let mut words = [0, 0, 8, 8, 8].into_iter();
    assert!(effects.accept_with_random(event(1, 1), || words.next().unwrap()));
    assert!(!effects.accept_with_random(event(1, 1), || panic!("full pool consumed random")));
    assert_eq!(effects.stats.dropped_particles, 1);
    assert!(effects.begin_frame(CLIENT, LIFE, EventTime(0), 800.0));
    assert_eq!(effects.len(), 1);
    assert!(effects.finish_frame(CLIENT, LIFE, EventTime(0), 800.0));
    assert!(effects.begin_frame(CLIENT, LIFE, EventTime(1), 800.0));
    assert!(effects.is_empty());
    assert_eq!(effects.stats.expired, 1);
    assert!(effects.accept_with_random(event(2, 1), || 256));
    let next = EntityId {
        generation: 2,
        ..LIFE
    };
    effects.synchronize(&[Some(EffectClient {
        client: CLIENT,
        lifetime: next,
    })]);
    assert!(effects.is_empty());
    assert_eq!(effects.stats.retired, 1);
    assert!(!effects.accept_with_random(event(1, 1), || panic!("stale lifetime consumed random")));
    assert!(effects.accept_with_random(
        EffectEvent {
            client_lifetime: Some(next),
            ..event(2, 1)
        },
        || 256
    ));
    effects.synchronize(&[]);
    assert!(effects.is_empty());
    assert_eq!(effects.stats.retired, 2);
}

#[test]
fn repeated_views_copy_source_palette_and_do_not_advance_particle_state() {
    let mut assets = Assets::load().unwrap();
    let rgb = vec![0; 256 * 3];
    let colormap = vec![0; 64 * 256];
    assets
        .register_palette(PaletteLighting::load(&rgb, &colormap, None, 256).unwrap())
        .unwrap();
    let source = assets
        .register_palette(PaletteLighting::load(&vec![17; 256 * 3], &colormap, None, 256).unwrap())
        .unwrap();
    let material = register_point_material(&mut assets, "particles/effect-fixture").unwrap();
    let mut effects = Effects::load(
        catalog(),
        1,
        3,
        &[
            None,
            Some(binding(source, material)),
            Some(binding(source, material)),
        ],
    )
    .unwrap();
    effects.synchronize(&[Some(EffectClient {
        client: CLIENT,
        lifetime: LIFE,
    })]);
    assert!(effects.accept_with_random(event(2, 1), || 256));
    assert!(effects.begin_frame(CLIENT, LIFE, EventTime(0), 800.0));
    let before = state(&effects)[0];
    let mut front = FrontEnd::load(Limits {
        particles: 2,
        ..Limits::default()
    })
    .unwrap();
    let mut frame = front.begin_frame([0; 4]).unwrap();
    assert_eq!(effects.append(&mut frame, CLIENT, LIFE), 1);
    assert!(frame.render_scene(Refdef::default(), &[], &assets));
    frame.clear_scene();
    assert_eq!(effects.append(&mut frame, CLIENT, LIFE), 1);
    assert!(frame.render_scene(Refdef::default(), &[], &assets));
    let packet = frame.finish();
    let Command::View(first) = packet.commands()[1] else {
        panic!()
    };
    let Command::View(second) = packet.commands()[2] else {
        panic!()
    };
    let first = packet.particles(first.scene.particles)[0];
    let second = packet.particles(second.scene.particles)[0];
    assert_eq!(first, second);
    assert_eq!(first.palette, source);
    assert_eq!(
        first.presentation,
        ParticlePresentation::Square { pixel_aspect: 1.5 }
    );
    assert_eq!(bits(first.position), bits(before.position));
    assert_eq!(bits(state(&effects)[0].position), bits(before.position));
    assert_eq!(effects.stats.scene_rejected, 0);
    assert!(effects.finish_frame(CLIENT, LIFE, EventTime(0), 800.0));
}

#[test]
fn malformed_empty_variants_and_missing_owners_fail_without_emission() {
    let good = *catalog().definition(EffectId(1)).unwrap();
    let invalid = Arc::new(EffectCatalog::new(
        vec![ParticleDefinition {
            variants: &[],
            ..good
        }]
        .into_boxed_slice(),
        [None; 5],
    ));
    assert!(Effects::load(invalid, 1, 1, &[None, None]).is_err());
    let mut effects = effects(1);
    for invalid in [
        event(0, 1),
        EffectEvent {
            client: None,
            ..event(1, 1)
        },
        EffectEvent {
            client_lifetime: None,
            ..event(1, 1)
        },
        EffectEvent {
            client: Some(ClientId(2)),
            ..event(1, 1)
        },
    ] {
        assert!(!effects.accept_with_random(invalid, || panic!("invalid event consumed random")));
    }
    assert_eq!(effects.stats.rejected_events, 4);
    assert!(effects.is_empty());
}
