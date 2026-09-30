//! Source particle constructors from Quake `r_part.c` and Quake II `cl_fx.c`.
//!
//! Donor: `src/app/bootstrap/effects/particles.ts`
//! (Copyright (C) 1996-2005 Id Software, Inc. GPL-2.0-or-later).

use std::f32::consts::PI;

use qa_client::render::scene::particles::{
    legacy::{advance_q1_particle, sample_q2_particle},
    Q1ParticleState, Q1ParticleType, Q2ParticleState, SceneParticle,
};
use qa_content::normals::ALIAS_NORMALS;
use qa_core::math::{add3, angles_to_axis, cross3, dot3, length3, normalize3_or_zero, scale3, sub3, vec3, Vec3};

use crate::bootstrap::simulation::random::SourceRandom;

const ZERO: Vec3 = Vec3 { x: 0.0, y: 0.0, z: 0.0 };
const GRAVITY: Vec3 = Vec3 {
    x: 0.0,
    y: 0.0,
    z: -40.0,
};
const Q1_FIRE_RAMP: [u8; 6] = [109, 107, 6, 5, 4, 3];
const DEFAULT_CAPACITY: usize = 4096;

/// Render sample of both particle pools.
pub struct ParticleSample {
    /// Q1 particles, newest first.
    pub q1: Vec<SceneParticle>,
    /// Q2 particles, newest first.
    pub q2: Vec<SceneParticle>,
}

/// Errors from fallible particle constructors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ParticleError {
    /// Colored explosion palette range is empty.
    #[error("Invalid Quake colored explosion palette range")]
    InvalidColorRange,
}

/// Q2 impact particle spread.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Q2ImpactVariant {
    /// Random shade, wide spread.
    #[default]
    Normal,
    /// Fixed shade, tight spread.
    Fixed,
    /// Fixed shade, reversed gravity.
    Up,
    /// Blaster bolts.
    Blaster,
}

/// Q2 diminishing trail content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2TrailKind {
    /// Rocket smoke plus fire.
    Rocket,
    /// Smoke.
    Smoke,
    /// Blood.
    Blood,
    /// Green blood.
    GreenBlood,
}

/// Q2 respawn/teleport effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2RespawnKind {
    /// Item respawn.
    Item,
    /// Player login.
    Login,
    /// Player logout.
    Logout,
    /// Player respawn.
    Respawn,
}

/// Quake/Quake II source particle pools.
pub struct SourceParticles {
    random: SourceRandom,
    capacity: usize,
    q1: Vec<Q1ParticleState>,
    q2: Vec<Q2ParticleState>,
    tracer_count: u32,
    angular_velocities: Vec<Vec3>,
}

impl SourceParticles {
    /// Create pools with the default capacity of 4096 per pool.
    #[must_use]
    pub fn new(random: SourceRandom) -> Self {
        Self::with_capacity(random, DEFAULT_CAPACITY)
    }

    /// Create pools with an explicit per-pool capacity.
    #[must_use]
    pub fn with_capacity(random: SourceRandom, capacity: usize) -> Self {
        Self {
            random,
            capacity,
            q1: Vec::new(),
            q2: Vec::new(),
            tracer_count: 0,
            angular_velocities: Vec::new(),
        }
    }

    /// Per-pool capacity.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Number of live Q1 particles.
    #[must_use]
    pub fn q1_count(&self) -> usize {
        self.q1.len()
    }

    /// Number of live Q2 particles.
    #[must_use]
    pub fn q2_count(&self) -> usize {
        self.q2.len()
    }

    /// Borrow the underlying random stream.
    #[must_use]
    pub fn random(&self) -> &SourceRandom {
        &self.random
    }

    /// Mutably borrow the underlying random stream.
    pub fn random_mut(&mut self) -> &mut SourceRandom {
        &mut self.random
    }

    fn rand(&mut self) -> u32 {
        self.random.next_integer()
    }

    fn unit(&mut self) -> f32 {
        (self.rand() & 32767) as f32 / 32767.0
    }

    fn signed(&mut self) -> f32 {
        2.0 * self.unit() - 1.0
    }

    fn push_q1(&mut self, particle: Q1ParticleState) -> bool {
        if self.q1.len() == self.capacity {
            return false;
        }
        self.q1.push(particle);
        true
    }

    fn push_q2(&mut self, particle: Q2ParticleState) -> bool {
        if self.q2.len() == self.capacity {
            return false;
        }
        self.q2.push(particle);
        true
    }

    fn spawn_ms(seconds: f32) -> i32 {
        (seconds * 1000.0) as i32
    }

    /// Drop all live particles (tracer count and angular velocities survive).
    pub fn clear(&mut self) {
        self.q1.clear();
        self.q2.clear();
    }

    /// Sample both pools at `seconds`, then advance Q1 states by `elapsed`.
    /// Call once per client frame, never once per seat.
    pub fn sample(&mut self, seconds: f32, elapsed: f32) -> ParticleSample {
        self.q1.retain(|particle| particle.die >= seconds);
        let q1: Vec<SceneParticle> = self
            .q1
            .iter()
            .rev()
            .map(|particle| SceneParticle::Indexed {
                palette_index: particle.color,
                alpha: 1.0,
                size: 1.0,
                origin: particle.origin,
            })
            .collect();
        let advanced: Vec<Q1ParticleState> = std::mem::take(&mut self.q1)
            .into_iter()
            .map(|particle| advance_q1_particle(&particle, elapsed, 800.0))
            .collect();
        self.q1 = advanced;
        let mut q2 = Vec::new();
        let mut retained = Vec::new();
        let milliseconds = Self::spawn_ms(seconds);
        for particle in std::mem::take(&mut self.q2) {
            if let Some(sampled) = sample_q2_particle(&particle, milliseconds) {
                q2.push(sampled);
                if particle.alpha_velocity != -10000.0 {
                    retained.push(particle);
                }
            }
        }
        q2.reverse();
        self.q2 = retained;
        ParticleSample { q1, q2 }
    }

    /// Q1 impact burst; `count == 1024` is the shotgun-double explosion alias.
    pub fn q1_impact(&mut self, origin: Vec3, direction: Vec3, color: u8, count: u32, seconds: f32) {
        if count == 1024 {
            self.q1_explosion(origin, seconds, false);
            return;
        }
        for _ in 0..count {
            let die = seconds + 0.1 * (self.rand() % 5) as f32;
            let shade = (color & !7) + (self.rand() & 7) as u8;
            let ox = origin.x + (self.rand() & 15) as f32 - 8.0;
            let oy = origin.y + (self.rand() & 15) as f32 - 8.0;
            let oz = origin.z + (self.rand() & 15) as f32 - 8.0;
            if !self.push_q1(Q1ParticleState {
                origin: vec3(ox, oy, oz),
                velocity: scale3(direction, 15.0),
                color: shade,
                ramp: 0.0,
                die,
                particle_type: Q1ParticleType::SlowGravity,
            }) {
                return;
            }
        }
    }

    /// Q1 entity-explosion shell over the alias normal table.
    pub fn q1_entity(&mut self, origin: Vec3, seconds: f32) {
        if self.angular_velocities.first().is_none_or(|velocity| velocity.x == 0.0) {
            let mut velocities = Vec::with_capacity(ALIAS_NORMALS.len());
            for _ in 0..ALIAS_NORMALS.len() {
                let x = (self.rand() & 255) as f32 * 0.01;
                let y = (self.rand() & 255) as f32 * 0.01;
                let z = (self.rand() & 255) as f32 * 0.01;
                velocities.push(vec3(x, y, z));
            }
            self.angular_velocities = velocities;
        }
        for (index, normal) in ALIAS_NORMALS.iter().enumerate() {
            if self.q1.len() == self.capacity {
                return;
            }
            let velocity = self.angular_velocities[index];
            let yaw = seconds * velocity.x;
            let pitch = seconds * velocity.y;
            let (cp, sp) = (pitch.cos(), pitch.sin());
            let (cy, sy) = (yaw.cos(), yaw.sin());
            let forward = vec3(cp * cy, cp * sy, -sp);
            let normal = vec3(normal[0], normal[1], normal[2]);
            let point = vec3(
                origin.x + normal.x * 64.0 + forward.x * 16.0,
                origin.y + normal.y * 64.0 + forward.y * 16.0,
                origin.z + normal.z * 64.0 + forward.z * 16.0,
            );
            self.push_q1(Q1ParticleState {
                origin: point,
                velocity: ZERO,
                color: 111,
                ramp: 0.0,
                die: seconds + 0.01,
                particle_type: Q1ParticleType::Explode,
            });
        }
    }

    /// Q1 projectile trail. `kind` is the donor `0..=6` trail id
    /// (0/1 fire, 2/4 blood, 3/5 tracer, 6 spark); values above 6 take the
    /// type-6 branch.
    pub fn q1_trail(&mut self, start: Vec3, end: Vec3, kind: u8, seconds: f32) {
        let delta = vec3(end.x - start.x, end.y - start.y, end.z - start.z);
        let mut remaining = (delta.x * delta.x + delta.y * delta.y + delta.z * delta.z).sqrt();
        let inverse = if remaining == 0.0 { 0.0 } else { 1.0 / remaining };
        let direction = vec3(delta.x * inverse, delta.y * inverse, delta.z * inverse);
        let mut point = start;
        while remaining > 0.0 {
            remaining -= 3.0;
            if self.q1.len() == self.capacity {
                return;
            }
            let mut particle_origin = point;
            let mut velocity = ZERO;
            let color: u8;
            let mut ramp = 0.0f32;
            let mut die = seconds + 2.0;
            let mut particle_type = Q1ParticleType::Static;
            if kind == 0 || kind == 1 {
                let ramp_index = (self.rand() & 3) + u32::from(kind == 1) * 2;
                ramp = ramp_index as f32;
                color = Q1_FIRE_RAMP[ramp_index as usize];
                particle_type = Q1ParticleType::Fire;
                let ox = point.x + (self.rand() % 6) as f32 - 3.0;
                let oy = point.y + (self.rand() % 6) as f32 - 3.0;
                let oz = point.z + (self.rand() % 6) as f32 - 3.0;
                particle_origin = vec3(ox, oy, oz);
            } else if kind == 2 || kind == 4 {
                color = 67 + (self.rand() & 3) as u8;
                particle_type = Q1ParticleType::Gravity;
                let ox = point.x + (self.rand() % 6) as f32 - 3.0;
                let oy = point.y + (self.rand() % 6) as f32 - 3.0;
                let oz = point.z + (self.rand() % 6) as f32 - 3.0;
                particle_origin = vec3(ox, oy, oz);
                if kind == 4 {
                    remaining -= 3.0;
                }
            } else if kind == 3 || kind == 5 {
                die = seconds + 0.5;
                color = (if kind == 3 { 52 } else { 230 }) + (((self.tracer_count & 4) << 1) as u8);
                self.tracer_count = self.tracer_count.wrapping_add(1);
                velocity = if self.tracer_count & 1 != 0 {
                    vec3(30.0 * direction.y, 30.0 * -direction.x, 0.0)
                } else {
                    vec3(30.0 * -direction.y, 30.0 * direction.x, 0.0)
                };
            } else {
                color = 152 + (self.rand() & 3) as u8;
                die = seconds + 0.3;
                let ox = point.x + (self.rand() & 15) as f32 - 8.0;
                let oy = point.y + (self.rand() & 15) as f32 - 8.0;
                let oz = point.z + (self.rand() & 15) as f32 - 8.0;
                particle_origin = vec3(ox, oy, oz);
            }
            self.push_q1(Q1ParticleState {
                origin: particle_origin,
                velocity,
                color,
                ramp,
                die,
                particle_type,
            });
            point = vec3(point.x + direction.x, point.y + direction.y, point.z + direction.z);
        }
    }

    /// Q1 1024-particle explosion (`blob` selects the alternate palette).
    pub fn q1_explosion(&mut self, origin: Vec3, seconds: f32, blob: bool) {
        for i in 0..1024u32 {
            let die = seconds
                + if blob {
                    1.0 + (self.rand() & 8) as f32 * 0.05
                } else {
                    5.0
                };
            let ramp = if blob { 0.0 } else { (self.rand() & 3) as f32 };
            let color = if blob {
                (if i & 1 == 1 { 66 } else { 150 }) + self.rand() % 6
            } else {
                111
            } as u8;
            let ox = origin.x + (self.rand() % 32) as f32 - 16.0;
            let vx = (self.rand() % 512) as f32 - 256.0;
            let oy = origin.y + (self.rand() % 32) as f32 - 16.0;
            let vy = (self.rand() % 512) as f32 - 256.0;
            let oz = origin.z + (self.rand() % 32) as f32 - 16.0;
            let vz = (self.rand() % 512) as f32 - 256.0;
            let particle_type = if blob {
                if i & 1 == 1 {
                    Q1ParticleType::Blob
                } else {
                    Q1ParticleType::Blob2
                }
            } else if i & 1 == 1 {
                Q1ParticleType::Explode
            } else {
                Q1ParticleType::Explode2
            };
            if !self.push_q1(Q1ParticleState {
                origin: vec3(ox, oy, oz),
                velocity: vec3(vx, vy, vz),
                color,
                ramp,
                die,
                particle_type,
            }) {
                return;
            }
        }
    }

    /// Q1 512-particle colored explosion cycling `color_length` palette entries.
    pub fn q1_color_explosion(
        &mut self,
        origin: Vec3,
        seconds: f32,
        color_start: u8,
        color_length: u8,
    ) -> Result<(), ParticleError> {
        if color_length == 0 {
            return Err(ParticleError::InvalidColorRange);
        }
        let length = u32::from(color_length);
        for i in 0..512u32 {
            if self.q1.len() >= self.capacity {
                break;
            }
            let ox = origin.x + (self.rand() % 32) as f32 - 16.0;
            let vx = (self.rand() % 512) as f32 - 256.0;
            let oy = origin.y + (self.rand() % 32) as f32 - 16.0;
            let vy = (self.rand() % 512) as f32 - 256.0;
            let oz = origin.z + (self.rand() % 32) as f32 - 16.0;
            let vz = (self.rand() % 512) as f32 - 256.0;
            self.push_q1(Q1ParticleState {
                origin: vec3(ox, oy, oz),
                velocity: vec3(vx, vy, vz),
                color: color_start.wrapping_add((i % length) as u8),
                ramp: 0.0,
                die: seconds + 0.3,
                particle_type: Q1ParticleType::Blob,
            });
        }
        Ok(())
    }

    /// Q1 splash (water) or lava spray.
    pub fn q1_splash(&mut self, origin: Vec3, seconds: f32, lava: bool) {
        let step: i32 = if lava { 1 } else { 4 };
        let mut i = -16;
        while i < 16 {
            let mut j = -16;
            while j < 16 {
                let mut k = if lava { 0 } else { -24 };
                let k_end = if lava { 1 } else { 32 };
                while k < k_end {
                    let die = seconds
                        + if lava {
                            2.0 + (self.rand() & 31) as f32 * 0.02
                        } else {
                            0.2 + (self.rand() & 7) as f32 * 0.02
                        };
                    let color = ((if lava { 224 } else { 7 }) + (self.rand() & 7)) as u8;
                    let direction = if lava {
                        let dx = (j * 8) as f32 + (self.rand() & 7) as f32;
                        let dy = (i * 8) as f32 + (self.rand() & 7) as f32;
                        vec3(dx, dy, 256.0)
                    } else {
                        vec3((j * 8) as f32, (i * 8) as f32, (k * 8) as f32)
                    };
                    let point = if lava {
                        vec3(
                            origin.x + direction.x,
                            origin.y + direction.y,
                            origin.z + (self.rand() & 63) as f32,
                        )
                    } else {
                        let jx = i as f32 + (self.rand() & 3) as f32;
                        let jy = j as f32 + (self.rand() & 3) as f32;
                        let jz = k as f32 + (self.rand() & 3) as f32;
                        add3(origin, vec3(jx, jy, jz))
                    };
                    let velocity = scale3(normalize3_or_zero(direction), 50.0 + (self.rand() & 63) as f32);
                    if !self.push_q1(Q1ParticleState {
                        origin: point,
                        velocity,
                        color,
                        ramp: 0.0,
                        die,
                        particle_type: Q1ParticleType::SlowGravity,
                    }) {
                        return;
                    }
                    k += 4;
                }
                j += step;
            }
            i += step;
        }
    }

    /// Q2 impact burst.
    pub fn q2_impact(
        &mut self,
        origin: Vec3,
        direction: Vec3,
        color: u8,
        count: u32,
        seconds: f32,
        variant: Q2ImpactVariant,
    ) {
        let blaster = variant == Q2ImpactVariant::Blaster;
        for _ in 0..count {
            let shade = if matches!(variant, Q2ImpactVariant::Fixed | Q2ImpactVariant::Up) {
                color
            } else {
                color.wrapping_add((self.rand() & 7) as u8)
            };
            let mask = match variant {
                Q2ImpactVariant::Normal => 31,
                Q2ImpactVariant::Blaster => 15,
                _ => 7,
            };
            let distance = (self.rand() & mask) as f32;
            let spawn_milliseconds = Self::spawn_ms(seconds);
            let ox = origin.x + (self.rand() & 7) as f32 - 4.0 + distance * direction.x;
            let vx = if blaster {
                direction.x * 30.0 + self.signed() * 40.0
            } else {
                self.signed() * 20.0
            };
            let oy = origin.y + (self.rand() & 7) as f32 - 4.0 + distance * direction.y;
            let vy = if blaster {
                direction.y * 30.0 + self.signed() * 40.0
            } else {
                self.signed() * 20.0
            };
            let oz = origin.z + (self.rand() & 7) as f32 - 4.0 + distance * direction.z;
            let vz = if blaster {
                direction.z * 30.0 + self.signed() * 40.0
            } else {
                self.signed() * 20.0
            };
            let acceleration = if variant == Q2ImpactVariant::Up {
                scale3(GRAVITY, -1.0)
            } else {
                GRAVITY
            };
            let alpha_velocity = -1.0 / (0.5 + self.unit() * 0.3);
            if !self.push_q2(Q2ParticleState {
                spawn_milliseconds,
                origin: vec3(ox, oy, oz),
                velocity: vec3(vx, vy, vz),
                acceleration,
                color: shade,
                alpha: 1.0,
                alpha_velocity,
            }) {
                return;
            }
        }
    }

    /// Q2 256-particle explosion.
    pub fn q2_explosion(&mut self, origin: Vec3, seconds: f32, bfg: bool) {
        for _ in 0..256u32 {
            let color = ((if bfg { 0xd0 } else { 0xe0 }) + (self.rand() & 7)) as u8;
            let ox = origin.x + (self.rand() % 32) as f32 - 16.0;
            let vx = (self.rand() % 384) as f32 - 192.0;
            let oy = origin.y + (self.rand() % 32) as f32 - 16.0;
            let vy = (self.rand() % 384) as f32 - 192.0;
            let oz = origin.z + (self.rand() % 32) as f32 - 16.0;
            let vz = (self.rand() % 384) as f32 - 192.0;
            let alpha_velocity = -0.8 / (0.5 + self.unit() * 0.3);
            if !self.push_q2(Q2ParticleState {
                spawn_milliseconds: Self::spawn_ms(seconds),
                origin: vec3(ox, oy, oz),
                velocity: vec3(vx, vy, vz),
                acceleration: GRAVITY,
                color,
                alpha: 1.0,
                alpha_velocity,
            }) {
                return;
            }
        }
    }

    /// Q2 128-particle colored explosion.
    ///
    /// # Panics
    /// Panics when `run` is 0 (the donor yields a NaN color there, which is
    /// unrepresentable in the ported state).
    pub fn q2_color_explosion(&mut self, origin: Vec3, seconds: f32, color: u8, run: u32) {
        for _ in 0..128u32 {
            if self.q2.len() >= self.capacity {
                break;
            }
            let shade = (u32::from(color) + self.rand() % run) as u8;
            let ox = origin.x + (self.rand() % 32) as f32 - 16.0;
            let vx = (self.rand() % 256) as f32 - 128.0;
            let oy = origin.y + (self.rand() % 32) as f32 - 16.0;
            let vy = (self.rand() % 256) as f32 - 128.0;
            let oz = origin.z + (self.rand() % 32) as f32 - 16.0;
            let vz = (self.rand() % 256) as f32 - 128.0;
            let alpha_velocity = -0.4 / (0.6 + self.unit() * 0.2);
            self.push_q2(Q2ParticleState {
                spawn_milliseconds: Self::spawn_ms(seconds),
                origin: vec3(ox, oy, oz),
                velocity: vec3(vx, vy, vz),
                acceleration: GRAVITY,
                color: shade,
                alpha: 1.0,
                alpha_velocity,
            });
        }
    }

    /// q2repro `CL_BerserkSlamParticles`.
    pub fn q2_berserk_slam(&mut self, origin: Vec3, direction: Vec3, seconds: f32) {
        let initial = vec3(direction.z, -direction.x, direction.y);
        let right = normalize3_or_zero(sub3(initial, scale3(direction, dot3(initial, direction))));
        let up = cross3(right, direction);
        for _ in 0..700u32 {
            if self.q2.len() >= self.capacity {
                break;
            }
            let color = (110 + 2 * (self.rand() & 3)) as u8;
            let forward = self.unit();
            let sx = self.signed();
            let sy = self.signed();
            let velocity = add3(
                add3(scale3(direction, forward * 192.0), scale3(right, sx * 192.0)),
                scale3(up, sy * 192.0),
            );
            let alpha_velocity = -1.0 / (0.5 + self.unit() * 0.3);
            self.push_q2(Q2ParticleState {
                spawn_milliseconds: Self::spawn_ms(seconds),
                origin,
                velocity,
                acceleration: ZERO,
                color,
                alpha: 1.0,
                alpha_velocity,
            });
        }
    }

    /// Q2 steam/smoke jet. Returns false when the pool filled mid-emission.
    #[allow(clippy::too_many_arguments)]
    pub fn q2_steam(
        &mut self,
        origin: Vec3,
        direction: Vec3,
        color: u8,
        count: u32,
        magnitude: f32,
        seconds: f32,
        smoke: bool,
    ) -> bool {
        let initial = vec3(direction.z, -direction.x, direction.y);
        let right = normalize3_or_zero(sub3(initial, scale3(direction, dot3(initial, direction))));
        let up = cross3(right, direction);
        for _ in 0..count {
            if self.q2.len() == self.capacity {
                return false;
            }
            let shade = color.wrapping_add((self.rand() & 7) as u8);
            let jx = magnitude * 0.1 * self.signed();
            let jy = magnitude * 0.1 * self.signed();
            let jz = magnitude * 0.1 * self.signed();
            let point = add3(origin, vec3(jx, jy, jz));
            let rx = self.signed();
            let ux = self.signed();
            let velocity = add3(
                add3(scale3(direction, magnitude), scale3(right, rx * magnitude / 3.0)),
                scale3(up, ux * magnitude / 3.0),
            );
            let alpha_velocity = -1.0 / (0.5 + self.unit() * 0.3);
            self.push_q2(Q2ParticleState {
                spawn_milliseconds: Self::spawn_ms(seconds),
                origin: point,
                velocity,
                acceleration: if smoke { ZERO } else { scale3(GRAVITY, 0.5) },
                color: shade,
                alpha: 1.0,
                alpha_velocity,
            });
        }
        true
    }

    /// Q2 force wall sheet.
    pub fn q2_force_wall(&mut self, start: Vec3, end: Vec3, color: u8, seconds: f32) {
        let delta = sub3(end, start);
        let length = length3(delta);
        let direction = normalize3_or_zero(delta);
        let mut distance = 0.0f32;
        while distance < length {
            if self.q2.len() >= self.capacity {
                break;
            }
            if self.unit() <= 0.3 {
                distance += 4.0;
                continue;
            }
            let alpha_velocity = -1.0 / (3.0 + self.unit() * 0.5);
            let jx = self.signed() * 3.0;
            let jy = self.signed() * 3.0;
            let jz = self.signed() * 3.0;
            let point = add3(add3(start, scale3(direction, distance)), vec3(jx, jy, jz));
            let vz = -40.0 - self.signed() * 10.0;
            self.push_q2(Q2ParticleState {
                spawn_milliseconds: Self::spawn_ms(seconds),
                origin: point,
                velocity: vec3(0.0, 0.0, vz),
                acceleration: ZERO,
                color,
                alpha: 1.0,
                alpha_velocity,
            });
            distance += 4.0;
        }
    }

    /// Q2 tracker trail helix.
    pub fn q2_tracker_trail(&mut self, start: Vec3, end: Vec3, seconds: f32) {
        let delta = sub3(end, start);
        let length = length3(delta);
        let direction = normalize3_or_zero(delta);
        let horizontal = direction.x.hypot(direction.y);
        let yaw = if horizontal == 0.0 {
            0.0
        } else {
            direction.y.atan2(direction.x) * 180.0 / PI
        };
        let pitch = -direction.z.atan2(horizontal) * 180.0 / PI;
        let axis = angles_to_axis(vec3(pitch, yaw, 0.0));
        let mut distance = 0.0f32;
        while distance < length {
            if self.q2.len() >= self.capacity {
                break;
            }
            let point = add3(start, scale3(direction, distance));
            let point_origin = add3(point, scale3(axis[2], 8.0 * dot3(point, axis[0]).cos()));
            self.push_q2(Q2ParticleState {
                spawn_milliseconds: Self::spawn_ms(seconds),
                origin: point_origin,
                velocity: vec3(0.0, 0.0, 5.0),
                acceleration: ZERO,
                color: 0,
                alpha: 1.0,
                alpha_velocity: -2.0,
            });
            distance += 3.0;
        }
    }

    /// Q2 tracker shell (instant particles, retired after one sample).
    pub fn q2_tracker_shell(&mut self, origin: Vec3, seconds: f32) {
        for _ in 0..300u32 {
            if self.q2.len() >= self.capacity {
                break;
            }
            let dx = self.signed();
            let dy = self.signed();
            let dz = self.signed();
            let point = add3(origin, scale3(normalize3_or_zero(vec3(dx, dy, dz)), 40.0));
            self.push_q2(Q2ParticleState {
                spawn_milliseconds: Self::spawn_ms(seconds),
                origin: point,
                velocity: ZERO,
                acceleration: ZERO,
                color: 0,
                alpha: 1.0,
                alpha_velocity: -10000.0,
            });
        }
    }

    /// Q2 item/respawn/login/logout burst.
    pub fn q2_respawn(&mut self, origin: Vec3, seconds: f32, kind: Q2RespawnKind) {
        let logout = kind != Q2RespawnKind::Item;
        let base = match kind {
            Q2RespawnKind::Login => 0xd0,
            Q2RespawnKind::Logout => 0x40,
            Q2RespawnKind::Respawn => 0xe0,
            Q2RespawnKind::Item => 0xd4,
        };
        let total = if logout { 500u32 } else { 64u32 };
        for _ in 0..total {
            let color = (base + (self.rand() & if logout { 7 } else { 3 })) as u8;
            let point = if logout {
                let ux = self.unit();
                let uy = self.unit();
                let uz = self.unit();
                add3(origin, vec3(-16.0 + ux * 32.0, -16.0 + uy * 32.0, -24.0 + uz * 56.0))
            } else {
                let sx = self.signed();
                let sy = self.signed();
                let sz = self.signed();
                add3(origin, vec3(sx * 8.0, sy * 8.0, sz * 8.0))
            };
            let scale = if logout { 20.0 } else { 8.0 };
            let vx = self.signed();
            let vy = self.signed();
            let vz = self.signed();
            let alpha_velocity = -1.0 / (1.0 + self.unit() * 0.3);
            if !self.push_q2(Q2ParticleState {
                spawn_milliseconds: Self::spawn_ms(seconds),
                origin: point,
                velocity: vec3(vx * scale, vy * scale, vz * scale),
                acceleration: scale3(GRAVITY, if logout { 1.0 } else { 0.2 }),
                color,
                alpha: 1.0,
                alpha_velocity,
            }) {
                return;
            }
        }
    }

    /// Q2 teleport burst.
    pub fn q2_teleport(&mut self, origin: Vec3, seconds: f32) {
        for i in (-16..=16).step_by(4) {
            for j in (-16..=16).step_by(4) {
                for k in (-16..=32).step_by(4) {
                    let color = (7 + (self.rand() & 7)) as u8;
                    let alpha_velocity = -1.0 / (0.3 + (self.rand() & 7) as f32 * 0.02);
                    let jx = i as f32 + (self.rand() & 3) as f32;
                    let jy = j as f32 + (self.rand() & 3) as f32;
                    let jz = k as f32 + (self.rand() & 3) as f32;
                    let point = add3(origin, vec3(jx, jy, jz));
                    let outward = vec3((j * 8) as f32, (i * 8) as f32, (k * 8) as f32);
                    let velocity = scale3(normalize3_or_zero(outward), 50.0 + (self.rand() & 63) as f32);
                    if !self.push_q2(Q2ParticleState {
                        spawn_milliseconds: Self::spawn_ms(seconds),
                        origin: point,
                        velocity,
                        acceleration: GRAVITY,
                        color,
                        alpha: 1.0,
                        alpha_velocity,
                    }) {
                        return;
                    }
                }
            }
        }
    }

    /// Q2 big teleport vortex.
    pub fn q2_big_teleport(&mut self, origin: Vec3, seconds: f32) {
        const COLORS: [u8; 4] = [16, 104, 168, 144];
        for _ in 0..4096u32 {
            if self.q2.len() >= self.capacity {
                break;
            }
            let color = COLORS[(self.rand() & 3) as usize];
            let angle = PI * 2.0 * ((self.rand() & 1023) as f32 / 1023.0);
            let distance = (self.rand() & 31) as f32;
            let (x, y) = (angle.cos(), angle.sin());
            let vx = x * (70.0 + (self.rand() & 63) as f32);
            let vy = y * (70.0 + (self.rand() & 63) as f32);
            let oz = origin.z + 8.0 + (self.rand() % 90) as f32;
            let vz = -100.0 + (self.rand() & 31) as f32;
            let alpha_velocity = -0.3 / (0.5 + self.unit() * 0.3);
            self.push_q2(Q2ParticleState {
                spawn_milliseconds: Self::spawn_ms(seconds),
                origin: vec3(origin.x + x * distance, origin.y + y * distance, oz),
                velocity: vec3(vx, vy, vz),
                acceleration: vec3(-x * 100.0, -y * 100.0, 160.0),
                color,
                alpha: 1.0,
                alpha_velocity,
            });
        }
    }

    /// Q2 teleporter splash.
    pub fn q2_teleporter(&mut self, origin: Vec3, seconds: f32) {
        for _ in 0..8u32 {
            if self.q2.len() >= self.capacity {
                break;
            }
            let ox = origin.x - 16.0 + (self.rand() & 31) as f32;
            let vx = self.signed() * 14.0;
            let oy = origin.y - 16.0 + (self.rand() & 31) as f32;
            let vy = self.signed() * 14.0;
            let oz = origin.z - 8.0 + (self.rand() & 7) as f32;
            let vz = 80.0 + (self.rand() & 7) as f32;
            self.push_q2(Q2ParticleState {
                spawn_milliseconds: Self::spawn_ms(seconds),
                origin: vec3(ox, oy, oz),
                velocity: vec3(vx, vy, vz),
                acceleration: GRAVITY,
                color: 0xdb,
                alpha: 1.0,
                alpha_velocity: -0.5,
            });
        }
    }

    /// Q2 blaster trail.
    pub fn q2_blaster_trail(&mut self, start: Vec3, end: Vec3, seconds: f32, green: bool) {
        let delta = sub3(end, start);
        let length = length3(delta);
        let direction = normalize3_or_zero(delta);
        let mut distance = 0.0f32;
        while distance < length {
            if self.q2.len() >= self.capacity {
                break;
            }
            let alpha_velocity = -1.0 / (0.3 + self.unit() * 0.2);
            let point = add3(start, scale3(direction, distance));
            let ox = point.x + self.signed();
            let vx = self.signed() * 5.0;
            let oy = point.y + self.signed();
            let vy = self.signed() * 5.0;
            let oz = point.z + self.signed();
            let vz = self.signed() * 5.0;
            self.push_q2(Q2ParticleState {
                spawn_milliseconds: Self::spawn_ms(seconds),
                origin: vec3(ox, oy, oz),
                velocity: vec3(vx, vy, vz),
                acceleration: ZERO,
                color: if green { 0xd0 } else { 0xe0 },
                alpha: 1.0,
                alpha_velocity,
            });
            distance += 5.0;
        }
    }

    /// Q2 diminishing trail; returns the decayed count (floored at 100).
    pub fn q2_diminishing_trail(&mut self, start: Vec3, end: Vec3, seconds: f32, count: i32, kind: Q2TrailKind) -> i32 {
        let mut count = count;
        let origin_scale = if count > 900 {
            4.0
        } else if count > 800 {
            2.0
        } else {
            1.0
        };
        let velocity_scale = if count > 900 {
            15.0
        } else if count > 800 {
            10.0
        } else {
            5.0
        };
        let blood = matches!(kind, Q2TrailKind::Blood | Q2TrailKind::GreenBlood);
        let delta = sub3(end, start);
        let length = length3(delta);
        let direction = normalize3_or_zero(delta);
        let mut distance = 0.0f32;
        while distance < length {
            if self.q2.len() >= self.capacity {
                break;
            }
            if ((self.rand() & 1023) as i32) < count {
                let alpha_velocity = -1.0 / (1.0 + self.unit() * if blood { 0.4 } else { 0.2 });
                let base = if blood {
                    if kind == Q2TrailKind::GreenBlood {
                        0xdb
                    } else {
                        0xe8
                    }
                } else {
                    4
                };
                let color = (base + (self.rand() & 7)) as u8;
                let point = add3(start, scale3(direction, distance));
                let ox = point.x + self.signed() * origin_scale;
                let vx = self.signed() * velocity_scale;
                let oy = point.y + self.signed() * origin_scale;
                let vy = self.signed() * velocity_scale;
                let oz = point.z + self.signed() * origin_scale;
                let vz = self.signed() * velocity_scale - if blood { 40.0 } else { 0.0 };
                self.push_q2(Q2ParticleState {
                    spawn_milliseconds: Self::spawn_ms(seconds),
                    origin: vec3(ox, oy, oz),
                    velocity: vec3(vx, vy, vz),
                    acceleration: if blood { ZERO } else { vec3(0.0, 0.0, 20.0) },
                    color,
                    alpha: 1.0,
                    alpha_velocity,
                });
            }
            count = (count - 5).max(100);
            distance += 0.5;
        }
        if kind == Q2TrailKind::Rocket {
            let mut distance = 0.0f32;
            while distance < length {
                if self.q2.len() >= self.capacity {
                    break;
                }
                if self.rand() & 7 != 0 {
                    distance += 1.0;
                    continue;
                }
                let alpha_velocity = -1.0 / (1.0 + self.unit() * 0.2);
                let color = (0xdc + (self.rand() & 3)) as u8;
                let point = add3(start, scale3(direction, distance));
                let ox = point.x + self.signed() * 5.0;
                let vx = self.signed() * 20.0;
                let oy = point.y + self.signed() * 5.0;
                let vy = self.signed() * 20.0;
                let oz = point.z + self.signed() * 5.0;
                let vz = self.signed() * 20.0;
                self.push_q2(Q2ParticleState {
                    spawn_milliseconds: Self::spawn_ms(seconds),
                    origin: vec3(ox, oy, oz),
                    velocity: vec3(vx, vy, vz),
                    acceleration: GRAVITY,
                    color,
                    alpha: 1.0,
                    alpha_velocity,
                });
                distance += 1.0;
            }
        }
        count
    }

    /// Q2 railgun spiral plus spark core.
    pub fn q2_rail(&mut self, start: Vec3, end: Vec3, seconds: f32) {
        let delta = sub3(end, start);
        let length = length3(delta);
        let direction = normalize3_or_zero(delta);
        let initial = vec3(direction.z, -direction.x, direction.y);
        let right = normalize3_or_zero(sub3(initial, scale3(direction, dot3(initial, direction))));
        let up = cross3(right, direction);
        let mut i = 0.0f32;
        while i < length {
            let radial = add3(scale3(right, (i * 0.1).cos()), scale3(up, (i * 0.1).sin()));
            let alpha_velocity = -1.0 / (1.0 + self.unit() * 0.2);
            let color = (0x74 + (self.rand() & 7)) as u8;
            if !self.push_q2(Q2ParticleState {
                spawn_milliseconds: Self::spawn_ms(seconds),
                origin: add3(add3(start, scale3(direction, i)), scale3(radial, 3.0)),
                velocity: scale3(radial, 6.0),
                acceleration: ZERO,
                color,
                alpha: 1.0,
                alpha_velocity,
            }) {
                return;
            }
            i += 1.0;
        }
        let mut i = 0.0f32;
        while i < length {
            let alpha_velocity = -1.0 / (0.6 + self.unit() * 0.2);
            let color = (self.rand() & 15) as u8;
            let point = add3(start, scale3(direction, i));
            let ox = point.x + self.signed() * 3.0;
            let vx = self.signed() * 3.0;
            let oy = point.y + self.signed() * 3.0;
            let vy = self.signed() * 3.0;
            let oz = point.z + self.signed() * 3.0;
            let vz = self.signed() * 3.0;
            if !self.push_q2(Q2ParticleState {
                spawn_milliseconds: Self::spawn_ms(seconds),
                origin: vec3(ox, oy, oz),
                velocity: vec3(vx, vy, vz),
                acceleration: ZERO,
                color,
                alpha: 1.0,
                alpha_velocity,
            }) {
                return;
            }
            i += 0.75;
        }
    }

    /// Q2 bubble trail.
    pub fn q2_bubbles(&mut self, start: Vec3, end: Vec3, seconds: f32) {
        let delta = sub3(end, start);
        let length = length3(delta);
        let direction = normalize3_or_zero(delta);
        let mut i = 0.0f32;
        while i < length {
            let alpha_velocity = -1.0 / (1.0 + self.unit() * 0.2);
            let color = (4 + (self.rand() & 7)) as u8;
            let point = add3(start, scale3(direction, i));
            let ox = point.x + self.signed() * 2.0;
            let vx = self.signed() * 5.0;
            let oy = point.y + self.signed() * 2.0;
            let vy = self.signed() * 5.0;
            let oz = point.z + self.signed() * 2.0;
            let vz = self.signed() * 5.0 + 6.0;
            if !self.push_q2(Q2ParticleState {
                spawn_milliseconds: Self::spawn_ms(seconds),
                origin: vec3(ox, oy, oz),
                velocity: vec3(vx, vy, vz),
                acceleration: ZERO,
                color,
                alpha: 1.0,
                alpha_velocity,
            }) {
                return;
            }
            i += 32.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn particles() -> SourceParticles {
        SourceParticles::new(SourceRandom::new(1))
    }

    fn origin() -> Vec3 {
        vec3(10.0, 20.0, 30.0)
    }

    #[test]
    fn q1_impact_1024_delegates_to_explosion() {
        let mut particles = particles();
        particles.q1_impact(origin(), vec3(0.0, 0.0, 1.0), 0xe0, 1024, 0.0);
        assert_eq!(particles.q1_count(), 1024);
    }

    #[test]
    fn q1_trail_zero_length_spawns_nothing() {
        let mut particles = particles();
        particles.q1_trail(origin(), origin(), 0, 0.0);
        assert_eq!(particles.q1_count(), 0);
    }

    #[test]
    fn q1_color_explosion_validates_range() {
        let mut particles = particles();
        assert_eq!(
            particles.q1_color_explosion(origin(), 0.0, 16, 0),
            Err(ParticleError::InvalidColorRange)
        );
        assert_eq!(particles.q1_color_explosion(origin(), 0.0, 16, 8), Ok(()));
        assert_eq!(particles.q1_count(), 512);
    }

    #[test]
    fn q1_explosion_renders_palette_111() {
        let mut particles = particles();
        particles.q1_explosion(origin(), 0.0, false);
        let sample = particles.sample(0.0, 0.0);
        assert_eq!(sample.q1.len(), 1024);
        assert!(sample
            .q1
            .iter()
            .all(|particle| matches!(particle, SceneParticle::Indexed { palette_index: 111, .. })));
    }

    #[test]
    fn sample_retires_q2_instant_particles() {
        let mut particles = particles();
        particles.q2_tracker_shell(origin(), 0.0);
        assert_eq!(particles.q2_count(), 300);
        assert_eq!(particles.sample(0.0, 0.0).q2.len(), 300);
        assert_eq!(particles.sample(0.0, 0.0).q2.len(), 0);
    }

    #[test]
    fn q2_steam_reports_full_pool() {
        let mut particles = SourceParticles::with_capacity(SourceRandom::new(2), 2);
        assert!(!particles.q2_steam(origin(), vec3(0.0, 0.0, 1.0), 8, 5, 20.0, 0.0, false));
        assert_eq!(particles.q2_count(), 2);
    }

    #[test]
    fn q2_diminishing_trail_floors_count_at_100() {
        let mut particles = particles();
        let count =
            particles.q2_diminishing_trail(vec3(0.0, 0.0, 0.0), vec3(10.0, 0.0, 0.0), 0.0, 120, Q2TrailKind::Rocket);
        assert_eq!(count, 100);
    }
}
