//! Fixed particle state consumed from the shared CLIENT output ring.
use crate::scene::{ParticlePresentation, SceneParticle};
use crate::{Frame, PaletteId, ShaderBindingId};
use qa_core::{
    effects::{EffectCatalog, ParticleMotion, VelocityChange},
    primitives::{ClientId, EffectEvent, EffectId, EntityId, Vec3},
    random::Random,
    stamps::StampSet,
    sys_events::EventTime,
};
use std::sync::Arc;

const NONE: usize = usize::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EffectClient {
    pub client: ClientId,
    pub lifetime: EntityId,
}
#[derive(Clone, Copy, Debug)]
pub struct ParticleBinding {
    pub palette: PaletteId,
    pub shader: ShaderBindingId,
    pub presentation: ParticlePresentation,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EffectStats {
    pub accepted_events: u64,
    pub rejected_events: u64,
    pub spawned: u64,
    pub dropped_particles: u64,
    pub expired: u64,
    pub retired: u64,
    pub scene_rejected: u64,
}
#[derive(Clone, Copy, Debug)]
pub struct ParticleView {
    pub effect: EffectId,
    pub position: Vec3,
    pub velocity: Vec3,
    pub color: u8,
    pub alpha: f32,
    pub ramp: f32,
    pub death: f32,
}
#[derive(Clone, Copy, Default)]
struct ClientFrame {
    lifetime: Option<EntityId>,
    completed: Option<EventTime>,
    prepared: Option<EventTime>,
}

struct Particles {
    next: Box<[usize]>,
    clients: Box<[ClientId]>,
    lifetimes: Box<[EntityId]>,
    definitions: Box<[EffectId]>,
    variants: Box<[usize]>,
    positions: Box<[Vec3]>,
    velocities: Box<[Vec3]>,
    colors: Box<[u8]>,
    alphas: Box<[f32]>,
    ramps: Box<[f32]>,
    deaths: Box<[f32]>,
    active: usize,
    free: usize,
    count: usize,
}
impl Particles {
    fn load(capacity: usize) -> Self {
        Self {
            next: (0..capacity)
                .map(|index| {
                    if index + 1 == capacity {
                        NONE
                    } else {
                        index + 1
                    }
                })
                .collect(),
            clients: vec![ClientId(0); capacity].into_boxed_slice(),
            lifetimes: vec![
                EntityId {
                    slot: 0,
                    generation: 0
                };
                capacity
            ]
            .into_boxed_slice(),
            definitions: vec![EffectId(0); capacity].into_boxed_slice(),
            variants: vec![0; capacity].into_boxed_slice(),
            positions: vec![Vec3::default(); capacity].into_boxed_slice(),
            velocities: vec![Vec3::default(); capacity].into_boxed_slice(),
            colors: vec![0; capacity].into_boxed_slice(),
            alphas: vec![1.0; capacity].into_boxed_slice(),
            ramps: vec![0.0; capacity].into_boxed_slice(),
            deaths: vec![0.0; capacity].into_boxed_slice(),
            active: NONE,
            free: if capacity == 0 { NONE } else { 0 },
            count: 0,
        }
    }
    fn remove_if(&mut self, mut remove: impl FnMut(&Self, usize) -> bool) -> u64 {
        let mut previous = NONE;
        let mut current = self.active;
        let mut removed = 0;
        while current != NONE {
            let next = self.next[current];
            if remove(self, current) {
                if previous == NONE {
                    self.active = next;
                } else {
                    self.next[previous] = next;
                }
                self.next[current] = self.free;
                self.free = current;
                self.count -= 1;
                removed += 1;
            } else {
                previous = current;
            }
            current = next;
        }
        removed
    }
    fn accept(
        &mut self,
        catalog: &EffectCatalog,
        bindings: &[Option<ParticleBinding>],
        clients: &[ClientFrame],
        event: EffectEvent,
        mut random: impl FnMut() -> u32,
        stats: &mut EffectStats,
    ) -> bool {
        let Some(definition) = catalog.definition(event.effect) else {
            stats.rejected_events += 1;
            return false;
        };
        let (Some(client), Some(lifetime)) = (event.client, event.client_lifetime) else {
            stats.rejected_events += 1;
            return false;
        };
        let emission = definition.emission;
        if !event.position.is_finite()
            || ((emission.velocity.random.is_none() || emission.velocity.direction_scale != 0.0)
                && !event.direction.is_finite())
            || !bindings
                .get(event.effect.0 as usize)
                .is_some_and(Option::is_some)
            || !clients
                .get(client.0 as usize)
                .is_some_and(|state| state.lifetime == Some(lifetime))
        {
            stats.rejected_events += 1;
            return false;
        }
        let now = event.time.seconds();
        let mut spawned = 0;
        for ordinal in 0..usize::from(event.count) {
            if self.free == NONE {
                stats.dropped_particles += u64::from(event.count) - spawned;
                break;
            }
            let slot = self.free;
            self.free = self.next[slot];
            self.next[slot] = self.active;
            self.active = slot;
            self.count += 1;
            self.clients[slot] = client;
            self.lifetimes[slot] = lifetime;
            self.definitions[slot] = event.effect;
            self.variants[slot] = ordinal % definition.variants.len();
            // Original emission consumes lifetime/color/ramp first, then each
            // axis's position and velocity words in that order.
            let duration = emission.lifetime.seconds
                * emission
                    .lifetime
                    .random
                    .map_or(1.0, |sample| f64::from(sample.sample(random())));
            self.deaths[slot] = (now + duration) as f32;
            self.colors[slot] = emission
                .color
                .event_mask
                .map_or(0, |mask| event.color & mask)
                + emission.color.base
                + emission
                    .color
                    .random
                    .map_or(0, |sample| sample.sample(random()) as u8);
            self.ramps[slot] = emission
                .ramp
                .map_or(0.0, |sample| sample.sample(random()) as f32);
            self.alphas[slot] = 1.0;
            for axis in 0..3 {
                let offset = emission.position.sample(random());
                self.positions[slot].0[axis] = event.position.0[axis] + offset as f32;
                self.velocities[slot].0[axis] = if let Some(offset) = emission.velocity.random {
                    let velocity = offset.sample(random());
                    if emission.velocity.direction_scale == 0.0 {
                        velocity as f32
                    } else {
                        event.direction.0[axis] * emission.velocity.direction_scale
                            + velocity as f32
                    }
                } else {
                    event.direction.0[axis] * emission.velocity.direction_scale
                };
            }
            spawned += 1;
        }
        stats.spawned += spawned;
        let accepted = event.count == 0 || spawned != 0;
        if accepted {
            stats.accepted_events += 1;
        } else {
            stats.rejected_events += 1;
        }
        accepted
    }
    fn view(&self, slot: usize) -> ParticleView {
        ParticleView {
            effect: self.definitions[slot],
            position: self.positions[slot],
            velocity: self.velocities[slot],
            color: self.colors[slot],
            alpha: self.alphas[slot],
            ramp: self.ramps[slot],
            death: self.deaths[slot],
        }
    }
    fn update(&mut self, slot: usize, motion: ParticleMotion, step: f32, gravity: f32) {
        self.positions[slot] = self.positions[slot] + self.velocities[slot] * step;
        if let Some(ramp) = motion.ramp {
            self.ramps[slot] += step * ramp.rate;
            if self.ramps[slot] >= ramp.colors.len() as f32 {
                self.deaths[slot] = -1.0;
            } else {
                self.colors[slot] = ramp.colors[self.ramps[slot] as usize];
            }
        }
        let velocity = self.velocities[slot];
        self.velocities[slot] = match motion.velocity {
            VelocityChange::None => velocity,
            VelocityChange::AddScaled(scale) => velocity + velocity * (scale * step),
            VelocityChange::SubtractScaled(scale) => velocity - velocity * (scale * step),
        };
        let grav = (f64::from(step * gravity) * motion.gravity_scale) as f32;
        self.velocities[slot].0[2] -= grav;
        self.alphas[slot] -= step * motion.alpha_rate;
        if self.alphas[slot] <= 0.0 {
            self.deaths[slot] = -1.0;
        }
    }
}

/// One load-sized SoA. The client frame prepares, submits every view, then
/// updates once; scene traversal never advances simulation or reads SERVER.
pub struct Effects {
    catalog: Arc<EffectCatalog>,
    bindings: Box<[Option<ParticleBinding>]>,
    clients: Box<[ClientFrame]>,
    seen: StampSet,
    particles: Particles,
    random: Random,
    pub stats: EffectStats,
}
impl Effects {
    pub fn load(
        catalog: Arc<EffectCatalog>,
        capacity: usize,
        clients: usize,
        bindings: &[Option<ParticleBinding>],
    ) -> Result<Self, &'static str> {
        if capacity > 65536 || clients > u32::MAX as usize || bindings.len() != catalog.len() + 1 {
            return Err("invalid effect storage limits");
        }
        if catalog.definitions().iter().any(|definition| {
            definition.variants.is_empty()
                || !definition.emission.lifetime.seconds.is_finite()
                || definition.emission.lifetime.seconds < 0.0
                || !definition.emission.velocity.direction_scale.is_finite()
                || definition.variants.iter().any(|motion| {
                    !motion.gravity_scale.is_finite()
                        || !motion.alpha_rate.is_finite()
                        || motion.ramp.is_some_and(|ramp| {
                            ramp.colors.is_empty() || !ramp.rate.is_finite() || ramp.rate < 0.0
                        })
                        || match motion.velocity {
                            VelocityChange::None => false,
                            VelocityChange::AddScaled(scale)
                            | VelocityChange::SubtractScaled(scale) => !scale.is_finite(),
                        }
                })
        }) {
            return Err("invalid particle definition");
        }
        Ok(Self {
            catalog,
            bindings: bindings.into(),
            clients: vec![ClientFrame::default(); clients].into_boxed_slice(),
            seen: StampSet::new(clients),
            particles: Particles::load(capacity),
            random: Random::new(0),
            stats: EffectStats::default(),
        })
    }
    pub fn capacity(&self) -> usize {
        self.particles.next.len()
    }
    pub fn random_word(&mut self) -> u32 {
        self.random.next_u32()
    }
    pub fn len(&self) -> usize {
        self.particles.count
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Fill an existing recipe slot without replacing explicit bindings or
    /// changing the active particles, client clocks, or random stream.
    pub fn bind_missing(&mut self, effect: EffectId, binding: ParticleBinding) -> bool {
        if self.catalog.definition(effect).is_none() {
            return false;
        }
        let Some(slot) = self.bindings.get_mut(effect.0 as usize) else {
            return false;
        };
        slot.get_or_insert(binding);
        true
    }
    /// Caller supplies actual connected canonical lifetimes, not seat ordinals.
    pub fn synchronize(&mut self, connected: &[Option<EffectClient>]) {
        self.seen.begin();
        for client in connected.iter().flatten() {
            let index = client.client.0 as usize;
            let Some(state) = self.clients.get_mut(index) else {
                continue;
            };
            if self.seen.test_and_set(index) {
                continue;
            }
            if state.lifetime != Some(client.lifetime) {
                *state = ClientFrame {
                    lifetime: Some(client.lifetime),
                    ..ClientFrame::default()
                };
            }
        }
        for (index, state) in self.clients.iter_mut().enumerate() {
            if !self.seen.contains(index) {
                *state = ClientFrame::default();
            }
        }
        let clients = &self.clients;
        self.stats.retired += self.particles.remove_if(|particles, slot| {
            clients
                .get(particles.clients[slot].0 as usize)
                .is_none_or(|state| state.lifetime != Some(particles.lifetimes[slot]))
        });
    }
    pub fn reset_client(&mut self, client: ClientId, lifetime: EntityId) {
        if let Some(state) = self.clients.get_mut(client.0 as usize)
            && state.lifetime == Some(lifetime)
        {
            state.completed = None;
            state.prepared = None;
        }
        self.stats.retired += self.particles.remove_if(|particles, slot| {
            particles.clients[slot] == client && particles.lifetimes[slot] == lifetime
        });
    }
    pub fn accept(&mut self, event: EffectEvent) -> bool {
        let random = &mut self.random;
        self.particles.accept(
            &self.catalog,
            &self.bindings,
            &self.clients,
            event,
            || random.next_u32(),
            &mut self.stats,
        )
    }
    /// Arithmetic fixtures supply words; runtime consumes the existing owned MT.
    pub fn accept_with_random(&mut self, event: EffectEvent, random: impl FnMut() -> u32) -> bool {
        self.particles.accept(
            &self.catalog,
            &self.bindings,
            &self.clients,
            event,
            random,
            &mut self.stats,
        )
    }
    pub fn begin_frame(
        &mut self,
        client: ClientId,
        lifetime: EntityId,
        time: EventTime,
        gravity: f32,
    ) -> bool {
        let Some(state) = self.clients.get_mut(client.0 as usize) else {
            return false;
        };
        if state.lifetime != Some(lifetime)
            || !gravity.is_finite()
            || state.completed.is_some_and(|completed| time < completed)
        {
            return false;
        }
        state.prepared = Some(time);
        self.stats.expired += self.particles.remove_if(|particles, slot| {
            particles.clients[slot] == client
                && particles.lifetimes[slot] == lifetime
                && f64::from(particles.deaths[slot]) < time.seconds()
        });
        true
    }
    pub fn visit(&self, client: ClientId, lifetime: EntityId, mut visit: impl FnMut(ParticleView)) {
        let mut slot = self.particles.active;
        while slot != NONE {
            if self.particles.clients[slot] == client && self.particles.lifetimes[slot] == lifetime
            {
                visit(self.particles.view(slot));
            }
            slot = self.particles.next[slot];
        }
    }
    pub fn append(&mut self, frame: &mut Frame, client: ClientId, lifetime: EntityId) -> u32 {
        let mut slot = self.particles.active;
        let mut appended = 0;
        while slot != NONE {
            if self.particles.clients[slot] == client && self.particles.lifetimes[slot] == lifetime
            {
                let binding = self.bindings[self.particles.definitions[slot].0 as usize];
                if let Some(binding) = binding {
                    if frame.add_particle(SceneParticle {
                        position: self.particles.positions[slot],
                        color: self.particles.colors[slot],
                        alpha: (self.particles.alphas[slot].clamp(0.0, 1.0) * 255.0) as u8,
                        palette: binding.palette,
                        shader: binding.shader,
                        size: 1.0,
                        presentation: binding.presentation,
                    }) {
                        appended += 1;
                    } else {
                        self.stats.scene_rejected += 1;
                    }
                } else {
                    self.stats.scene_rejected += 1;
                }
            }
            slot = self.particles.next[slot];
        }
        appended
    }
    pub fn finish_frame(
        &mut self,
        client: ClientId,
        lifetime: EntityId,
        time: EventTime,
        gravity: f32,
    ) -> bool {
        let Some(state) = self.clients.get_mut(client.0 as usize) else {
            return false;
        };
        if state.lifetime != Some(lifetime)
            || state.prepared != Some(time)
            || state.completed == Some(time)
            || !gravity.is_finite()
        {
            return false;
        }
        let step = state
            .completed
            .map_or(0.0, |previous| (time.seconds() - previous.seconds()) as f32);
        state.completed = Some(time);
        state.prepared = None;
        let mut slot = self.particles.active;
        while slot != NONE {
            if self.particles.clients[slot] == client && self.particles.lifetimes[slot] == lifetime
            {
                let Some(definition) = self.catalog.definition(self.particles.definitions[slot])
                else {
                    return false;
                };
                self.particles.update(
                    slot,
                    definition.variants[self.particles.variants[slot]],
                    step,
                    gravity,
                );
            }
            slot = self.particles.next[slot];
        }
        true
    }
}
