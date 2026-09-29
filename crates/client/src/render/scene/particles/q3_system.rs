//! Q3 particle pools (donor `src/render/scene/particles/q3-system.ts`).
//!
//! `cg_marks.c` and `cg_particles.c` particle allocation, emission, and
//! polygon generation.

use qa_core::math::{angle_vectors, length3, scale3, sub3, vec2, vec3, vec4, vector_to_angles, Vec3, Vec4};
use qa_core::rng::Qrand;

use crate::render::error::RenderError;

use super::q3_types::{
    ParticleClientEntity, ParticleClientState, ParticleResources, ParticleShader, ParticleTracer, RefPoly,
    RefPolyVertex, TraceSolidity,
};

const ENTITYNUM_WORLD: i32 = 1022;

/// `cg_marks.c` pool capacity.
pub const MAX_PARTICLES: usize = 1024;

/// Particle behavior kind. The pinned source never constructs some variants;
/// they stay so the enum matches the donor exactly.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParticleType {
    None,
    Weather,
    Flat,
    Smoke,
    Rotate,
    WeatherTurbulent,
    Animated,
    Bat,
    Bleed,
    FlatScaleUp,
    FlatScaleUpFade,
    WeatherFlurry,
    SmokeImpact,
    Bubble,
    BubbleTurbulent,
    Sprite,
}

/// Particle color mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParticleColor {
    White,
    Blood,
    EmissiveFade,
    Grey75,
}

#[derive(Debug, Clone)]
struct Particle {
    next: i32,
    time: f32,
    end_time: f32,
    origin: Vec3,
    velocity: Vec3,
    acceleration: Vec3,
    color: ParticleColor,
    alpha: f32,
    alpha_velocity: f32,
    particle_type: ParticleType,
    shader: Option<ParticleShader>,
    height: f32,
    width: f32,
    end_height: f32,
    end_width: f32,
    start: f32,
    end: f32,
    start_fade: f32,
    rotate: bool,
    snum: i32,
    link: bool,
    shader_animation: usize,
    roll: i32,
    accumulated_roll: i32,
}

/// Registered animation frames for one particle animation.
pub type AnimationFrames = Vec<Option<ParticleShader>>;

/// `cg_marks.c` particle animations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParticleAnimations {
    /// Explosion frames.
    pub explode1: AnimationFrames,
}

/// `cg_particles.c` standalone particle animations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StandaloneParticleAnimations {
    /// Base animations.
    pub base: ParticleAnimations,
    /// Black smoke frames.
    pub blacksmokeanim: AnimationFrames,
    /// Twilt frames.
    pub twiltb2: AnimationFrames,
    /// Blue explosion frames.
    pub expblue: AnimationFrames,
    /// Alternate smoke frames.
    pub blacksmokeanimb: AnimationFrames,
    /// Blood frames.
    pub blood: AnimationFrames,
}

/// Animation set selecting the source profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParticleAnimationSet {
    /// `cg_marks.c` profile.
    Marks(ParticleAnimations),
    /// `cg_particles.c` profile.
    Standalone(StandaloneParticleAnimations),
}

/// Particle media shaders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParticleMedia {
    /// Tracer shader.
    pub tracer_shader: Option<ParticleShader>,
    /// Smoke puff shader.
    pub smoke_puff_shader: Option<ParticleShader>,
    /// Water bubble shader.
    pub water_bubble_shader: Option<ParticleShader>,
}

/// Hardware-specific particle behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParticleHardware {
    /// Default behavior.
    Generic,
    /// Rage Pro: smoke never fades.
    RagePro,
}

/// Particle system host services.
pub struct ParticleHost {
    /// Registered animations.
    pub animations: ParticleAnimationSet,
    /// Media shaders.
    pub media: ParticleMedia,
    /// Prediction tracer.
    pub prediction: Box<dyn ParticleTracer>,
    /// Game random stream.
    pub random: Qrand,
    /// Hardware behavior.
    pub hardware: ParticleHardware,
    /// Config-string lookup.
    pub config_string: Box<dyn Fn(usize) -> String>,
    /// Diagnostic printer.
    pub print: Box<dyn Fn(&str)>,
}

impl std::fmt::Debug for ParticleHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParticleHost")
            .field("animations", &self.animations)
            .field("media", &self.media)
            .field("hardware", &self.hardware)
            .finish()
    }
}

/// Particle explosion request.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParticleExplosionRequest {
    /// Animation name.
    pub animation: &'static str,
    /// Origin.
    pub origin: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Duration in milliseconds (negative pins the roll).
    pub duration: i32,
    /// Start size.
    pub size_start: i32,
    /// End size.
    pub size_end: i32,
}

struct RegisteredAnimation {
    name: &'static str,
    count: usize,
    aspect_ratio: f32,
    frames: AnimationFrames,
}

const ANIMATION_SPECS: [(&str, usize, f32); 6] = [
    ("explode1", 23, 1.405),
    ("blacksmokeanim", 25, 1.0),
    ("twiltb2", 45, 1.0),
    ("expblue", 25, 1.0),
    ("blacksmokeanimb", 23, 1.0),
    ("blood", 5, 1.0),
];

fn load_frames(resources: &mut dyn ParticleResources, name: &str, count: usize) -> AnimationFrames {
    (1..=count)
        .map(|frame| resources.register_shader(&format!("{name}{frame}")))
        .collect()
}

/// Register `cg_marks.c` particle animations before constructing a system.
#[must_use]
pub fn load_particle_animations(resources: &mut dyn ParticleResources) -> ParticleAnimations {
    ParticleAnimations {
        explode1: load_frames(resources, "explode1", 23),
    }
}

/// Register `cg_particles.c` standalone animations before constructing a system.
#[must_use]
pub fn load_standalone_particle_animations(resources: &mut dyn ParticleResources) -> StandaloneParticleAnimations {
    StandaloneParticleAnimations {
        base: load_particle_animations(resources),
        blacksmokeanim: load_frames(resources, "blacksmokeanim", 25),
        twiltb2: load_frames(resources, "twiltb2", 45),
        expblue: load_frames(resources, "expblue", 25),
        blacksmokeanimb: load_frames(resources, "blacksmokeanimb", 23),
        blood: load_frames(resources, "blood", 5),
    }
}

fn registered_animation(
    name: &'static str,
    frames: &AnimationFrames,
    aspect_ratio: f32,
) -> Result<RegisteredAnimation, RenderError> {
    let count = ANIMATION_SPECS
        .iter()
        .find(|spec| spec.0 == name)
        .map_or(0, |spec| spec.1);
    if frames.len() != count {
        return Err(RenderError::BadBatch {
            index: 0,
            detail: format!("{name} requires its {count} registered source frames"),
        });
    }
    Ok(RegisteredAnimation {
        name,
        count,
        aspect_ratio,
        frames: frames.clone(),
    })
}

fn zero_particle(next: i32) -> Particle {
    Particle {
        next,
        time: 0.0,
        end_time: 0.0,
        origin: vec3(0.0, 0.0, 0.0),
        velocity: vec3(0.0, 0.0, 0.0),
        acceleration: vec3(0.0, 0.0, 0.0),
        color: ParticleColor::White,
        alpha: 0.0,
        alpha_velocity: 0.0,
        particle_type: ParticleType::None,
        shader: None,
        height: 0.0,
        width: 0.0,
        end_height: 0.0,
        end_width: 0.0,
        start: 0.0,
        end: 0.0,
        start_fade: 0.0,
        rotate: false,
        snum: 0,
        link: false,
        shader_animation: 0,
        roll: 0,
        accumulated_roll: 0,
    }
}

fn float_to_int(value: f32) -> i32 {
    value.trunc() as i32
}

fn move_along(origin: Vec3, scale: f32, direction: Vec3) -> Vec3 {
    vec3(
        origin.x + scale * direction.x,
        origin.y + scale * direction.y,
        origin.z + scale * direction.z,
    )
}

fn game_atoi(text: &str) -> i32 {
    let text = text.trim_start();
    let (sign, digits) = match text.strip_prefix('-') {
        Some(rest) => (-1i64, rest),
        None => (1i64, text.strip_prefix('+').unwrap_or(text)),
    };
    let mut value = 0i64;
    for byte in digits.bytes() {
        if !byte.is_ascii_digit() {
            break;
        }
        value = value.saturating_mul(10).saturating_add(i64::from(byte - b'0'));
    }
    (sign * value).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

fn game_atof(text: &str) -> f32 {
    let text = text.trim_start();
    let mut length = 0;
    for (index, byte) in text.bytes().enumerate() {
        let ok = byte.is_ascii_digit() || byte == b'.' || byte == b'+' || byte == b'-' || byte == b'e' || byte == b'E';
        if !ok {
            break;
        }
        length = index + 1;
    }
    text[..length].parse::<f64>().unwrap_or(0.0) as f32
}

/// Pooled Q3 particle system.
pub struct ParticleSystem {
    state: ParticleClientState,
    host: ParticleHost,
    particles: Vec<Particle>,
    active: i32,
    free: i32,
    count: usize,
    old_time: f32,
    view_roll: f32,
    view_axis: qa_core::math::Axis,
    rotated_axes: [Vec3; 3],
    animations: Vec<RegisteredAnimation>,
    capacity: usize,
    explosion_alpha: f32,
}

impl std::fmt::Debug for ParticleSystem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParticleSystem")
            .field("count", &self.count)
            .field("capacity", &self.capacity)
            .finish()
    }
}

impl ParticleSystem {
    /// System over client state and a fully registered host.
    pub fn new(state: ParticleClientState, host: ParticleHost) -> Result<Self, RenderError> {
        let (animations, capacity, explosion_alpha) = match &host.animations {
            ParticleAnimationSet::Marks(animations) => (
                vec![registered_animation("explode1", &animations.explode1, 1.0)?],
                MAX_PARTICLES,
                0.5,
            ),
            ParticleAnimationSet::Standalone(animations) => {
                let specs = [
                    ("explode1", &animations.base.explode1, 1.405f32),
                    ("blacksmokeanim", &animations.blacksmokeanim, 1.0),
                    ("twiltb2", &animations.twiltb2, 1.0),
                    ("expblue", &animations.expblue, 1.0),
                    ("blacksmokeanimb", &animations.blacksmokeanimb, 1.0),
                    ("blood", &animations.blood, 1.0),
                ];
                let mut registered = Vec::new();
                for (name, frames, aspect) in specs {
                    registered.push(registered_animation(name, frames, aspect)?);
                }
                (registered, 8192, 1.0)
            }
        };
        let mut system = Self {
            old_time: state.time,
            state,
            host,
            particles: Vec::new(),
            active: -1,
            free: 0,
            count: 0,
            view_roll: 0.0,
            view_axis: [vec3(0.0, 0.0, 0.0); 3],
            rotated_axes: [vec3(0.0, 0.0, 0.0); 3],
            animations,
            capacity,
            explosion_alpha,
        };
        system.reset_pool();
        Ok(system)
    }

    /// Active particle count.
    #[must_use]
    pub fn active_count(&self) -> usize {
        self.count
    }

    /// Pool capacity.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Reset the pool between rounds.
    pub fn reset_round(&mut self) {
        self.reset_pool();
    }

    /// Reset the pool and re-register animation frames.
    pub fn clear(&mut self, resources: &mut dyn ParticleResources) {
        self.reset_round();
        for animation in &mut self.animations {
            for frame in 0..animation.count {
                animation.frames[frame] = resources.register_shader(&format!("{}{}", animation.name, frame + 1));
            }
        }
    }

    /// Advance the client clock observed by the pool.
    pub fn set_time(&mut self, time: f32, view_axis: qa_core::math::Axis) {
        self.state.time = time;
        self.state.view_axis = view_axis;
    }

    fn reset_pool(&mut self) {
        self.particles.clear();
        for index in 0..self.capacity {
            let next = if index + 1 == self.capacity {
                -1
            } else {
                index as i32 + 1
            };
            self.particles.push(zero_particle(next));
        }
        self.active = -1;
        self.free = 0;
        self.count = 0;
        self.old_time = self.state.time;
    }

    fn allocate(&mut self) -> Option<usize> {
        if self.free == -1 {
            return None;
        }
        let index = self.free as usize;
        let next = self.particles[index].next;
        self.free = next;
        self.particles[index].next = self.active;
        self.active = index as i32;
        self.count += 1;
        self.particles[index].time = self.state.time;
        Some(index)
    }

    fn release(&mut self, index: usize) {
        self.particles[index].next = self.free;
        self.free = index as i32;
        self.particles[index].particle_type = ParticleType::None;
        self.particles[index].color = ParticleColor::White;
        self.particles[index].alpha = 0.0;
        self.count -= 1;
    }

    fn warn_shader(&self, shader: &Option<ParticleShader>, name: &str) {
        if shader.is_none() {
            (self.host.print)(&format!("{name} pshader == ZERO!\n"));
        }
    }

    fn after(&self, duration: i32) -> f32 {
        (self.state.time + duration as f32).trunc()
    }

    /// Animated explosion particle.
    pub fn explosion(&mut self, request: &ParticleExplosionRequest) -> Result<(), RenderError> {
        let lowered = request.animation.to_lowercase();
        let animation_index = self
            .animations
            .iter()
            .position(|animation| animation.name == lowered)
            .ok_or_else(|| {
                RenderError::Backend(format!(
                    "CG_ParticleExplosion: unknown animation string: {}\n",
                    request.animation
                ))
            })?;
        let aspect = self.animations[animation_index].aspect_ratio;
        let Some(index) = self.allocate() else {
            return Ok(());
        };
        let mut duration = request.duration;
        self.particles[index].alpha = self.explosion_alpha;
        self.particles[index].alpha_velocity = 0.0;
        if duration < 0 {
            duration = duration.wrapping_neg();
            self.particles[index].roll = 0;
        } else {
            self.particles[index].roll = float_to_int(self.host.random.next_unit() as f32 * 179.0);
        }
        let end_time = self.after(duration);
        let particle = &mut self.particles[index];
        particle.shader_animation = animation_index;
        particle.width = request.size_start as f32;
        particle.height = request.size_start as f32 * aspect;
        particle.end_height = request.size_end as f32;
        particle.end_width = request.size_end as f32 * aspect;
        particle.end_time = end_time;
        particle.particle_type = ParticleType::Animated;
        particle.origin = request.origin;
        particle.velocity = request.velocity;
        particle.acceleration = vec3(0.0, 0.0, 0.0);
        Ok(())
    }

    /// Snow flurry particle from an entity state.
    pub fn snow_flurry(&mut self, shader: Option<ParticleShader>, entity: &ParticleClientEntity) {
        self.warn_shader(&shader, "CG_ParticleSnowFlurry");
        let Some(index) = self.allocate() else {
            return;
        };
        let source = entity;
        let end_time = self.after(source.time);
        let start_fade = self.after(source.time2);
        let big = self.host.random.next_integer() % 100 > 90;
        let (vx, vy, ax, ay) = (
            self.host.random.next_unit() as f32,
            self.host.random.next_unit() as f32,
            self.host.random.next_centered() as f32,
            self.host.random.next_centered() as f32,
        );
        let particle = &mut self.particles[index];
        particle.color = ParticleColor::White;
        particle.alpha = 0.9;
        particle.alpha_velocity = 0.0;
        particle.start = source.origin2.x;
        particle.end = source.origin2.y;
        particle.end_time = end_time;
        particle.start_fade = start_fade;
        particle.shader = shader;
        if big {
            particle.height = 32.0;
            particle.width = 32.0;
            particle.alpha = 0.1;
        } else {
            particle.height = 1.0;
            particle.width = 1.0;
        }
        particle.particle_type = ParticleType::WeatherFlurry;
        particle.origin = source.origin;
        particle.velocity = vec3(
            source.angles.x * 32.0 + vx * 16.0,
            source.angles.y * 32.0 + vy * 16.0,
            -10.0 + source.angles.z,
        );
        particle.acceleration = vec3(ax * 16.0, ay * 16.0, 0.0);
    }

    /// Falling snow particle, optionally turbulent.
    pub fn snow(
        &mut self,
        shader: Option<ParticleShader>,
        origin: Vec3,
        end: Vec3,
        turbulent: bool,
        range: f32,
        snum: i32,
    ) -> Result<(), RenderError> {
        if origin.z <= end.z {
            return Err(RenderError::BadBatch {
                index: 0,
                detail: "snow start must exceed end to keep the source wrap loop finite".to_string(),
            });
        }
        self.warn_shader(&shader, "CG_ParticleSnow");
        let Some(index) = self.allocate() else {
            return Ok(());
        };
        let (rx, ry, rz, vx, vy) = (
            self.host.random.next_centered() as f32,
            self.host.random.next_centered() as f32,
            self.host.random.next_centered() as f32,
            self.host.random.next_centered() as f32,
            self.host.random.next_centered() as f32,
        );
        let particle = &mut self.particles[index];
        particle.color = ParticleColor::White;
        particle.alpha = 0.4;
        particle.alpha_velocity = 0.0;
        particle.start = origin.z;
        particle.end = end.z;
        particle.shader = shader;
        particle.height = 1.0;
        particle.width = 1.0;
        particle.particle_type = if turbulent {
            ParticleType::WeatherTurbulent
        } else {
            ParticleType::Weather
        };
        particle.origin = vec3(
            origin.x + rx * range,
            origin.y + ry * range,
            origin.z + rz * (particle.start - particle.end),
        );
        particle.velocity = vec3(
            if turbulent { vx * 16.0 } else { 0.0 },
            if turbulent { vy * 16.0 } else { 0.0 },
            if turbulent { -50.0 * 1.3 } else { -50.0 },
        );
        particle.acceleration = vec3(0.0, 0.0, 0.0);
        particle.snum = snum;
        particle.link = true;
        Ok(())
    }

    /// Rising bubble particle, optionally turbulent.
    pub fn bubble(
        &mut self,
        shader: Option<ParticleShader>,
        origin: Vec3,
        end: Vec3,
        turbulent: bool,
        range: f32,
        snum: i32,
    ) {
        self.warn_shader(&shader, "CG_ParticleSnow");
        let Some(index) = self.allocate() else {
            return;
        };
        let (size_roll, vz_roll, rx, ry, rz, vx, vy) = (
            self.host.random.next_centered() as f32,
            self.host.random.next_centered() as f32,
            self.host.random.next_centered() as f32,
            self.host.random.next_centered() as f32,
            self.host.random.next_centered() as f32,
            self.host.random.next_centered() as f32,
            self.host.random.next_centered() as f32,
        );
        let particle = &mut self.particles[index];
        particle.color = ParticleColor::White;
        particle.alpha = 0.4;
        particle.alpha_velocity = 0.0;
        particle.start = origin.z;
        particle.end = end.z;
        particle.shader = shader;
        particle.height = 1.0 + size_roll * 0.5;
        particle.width = particle.height;
        let mut velocity_z = 50.0 + vz_roll * 10.0;
        if turbulent {
            velocity_z = 50.0 * 1.3;
        }
        particle.particle_type = if turbulent {
            ParticleType::BubbleTurbulent
        } else {
            ParticleType::Bubble
        };
        particle.origin = vec3(
            origin.x + rx * range,
            origin.y + ry * range,
            origin.z + rz * (particle.start - particle.end),
        );
        particle.velocity = vec3(
            if turbulent { vx * 4.0 } else { 0.0 },
            if turbulent { vy * 4.0 } else { 0.0 },
            velocity_z,
        );
        particle.acceleration = vec3(0.0, 0.0, 0.0);
        particle.snum = snum;
        particle.link = true;
    }

    /// Smoke column particle from an entity state.
    pub fn smoke(&mut self, shader: Option<ParticleShader>, entity: &ParticleClientEntity) {
        if shader.is_none() {
            (self.host.print)("CG_ParticleSmoke == ZERO!\n");
        }
        let Some(index) = self.allocate() else {
            return;
        };
        let end_time = self.after(entity.time);
        let start_fade = self.after(entity.time2);
        let roll = float_to_int(8.0 + self.host.random.next_centered() as f32 * 4.0);
        let particle = &mut self.particles[index];
        particle.end_time = end_time;
        particle.start_fade = start_fade;
        particle.color = ParticleColor::White;
        particle.alpha = 1.0;
        particle.alpha_velocity = 0.0;
        particle.start = entity.origin.z;
        particle.end = entity.origin2.z;
        particle.shader = shader;
        particle.rotate = false;
        particle.height = 8.0;
        particle.width = 8.0;
        particle.end_height = 32.0;
        particle.end_width = 32.0;
        particle.particle_type = ParticleType::Smoke;
        particle.origin = entity.origin;
        particle.velocity = vec3(0.0, 0.0, if entity.frame == 1 { -5.0 } else { 5.0 });
        particle.acceleration = vec3(0.0, 0.0, 0.0);
        particle.roll = roll;
    }

    /// Bullet-impact debris particle.
    pub fn bullet_debris(&mut self, origin: Vec3, velocity: Vec3, duration: i32) {
        let Some(index) = self.allocate() else {
            return;
        };
        let end_time = self.after(duration);
        let start_fade = self.after(duration / 2);
        let shader = self.host.media.tracer_shader.clone();
        let particle = &mut self.particles[index];
        particle.end_time = end_time;
        particle.start_fade = start_fade;
        particle.color = ParticleColor::EmissiveFade;
        particle.alpha = 1.0;
        particle.alpha_velocity = 0.0;
        particle.height = 0.5;
        particle.width = 0.5;
        particle.end_height = 0.5;
        particle.end_width = 0.5;
        particle.shader = shader;
        particle.particle_type = ParticleType::Smoke;
        particle.origin = origin;
        particle.velocity = vec3(velocity.x, velocity.y, velocity.z - 20.0);
        particle.acceleration = vec3(0.0, 0.0, -60.0);
    }

    /// `CG_AddParticleShrapnel` is an intentional empty function in the source.
    pub fn add_particle_shrapnel(&self) {}

    /// Spawn a weather/bubble area from a config string.
    pub fn new_particle_area(&mut self, index: usize) -> Result<bool, RenderError> {
        let text = (self.host.config_string)(index);
        if text.is_empty() {
            return Ok(false);
        }
        let tokens: Vec<&str> = text.split_whitespace().collect();
        let token = |position: usize| tokens.get(position).copied().unwrap_or("");
        let kind = game_atoi(token(0));
        let range = match kind {
            0 => 256.0,
            1 => 128.0,
            2 | 7 => 64.0,
            3 | 6 => 32.0,
            4 => 8.0,
            5 => 16.0,
            _ => 0.0,
        };
        let origin = vec3(game_atof(token(1)), game_atof(token(2)), game_atof(token(3)));
        let end = vec3(game_atof(token(4)), game_atof(token(5)), game_atof(token(6)));
        let count = game_atoi(token(7));
        let turbulent = game_atoi(token(8)) != 0;
        let snum = game_atoi(token(9));
        for _ in 0..count.max(0) {
            let shader = self.host.media.water_bubble_shader.clone();
            if kind >= 4 {
                self.bubble(shader, origin, end, turbulent, range, snum);
            } else {
                self.snow(shader, origin, end, turbulent, range, snum)?;
            }
        }
        Ok(true)
    }

    /// Enable or disable snow linkage for an entity's weather particles.
    pub fn snow_link(&mut self, entity: &ParticleClientEntity, enabled: bool) {
        let mut index = self.active;
        while index != -1 {
            let next = self.particles[index as usize].next;
            let particle = &mut self.particles[index as usize];
            if matches!(
                particle.particle_type,
                ParticleType::Weather | ParticleType::WeatherTurbulent
            ) && particle.snum == entity.frame
            {
                particle.link = enabled;
            }
            index = next;
        }
    }

    /// Impact smoke puff particle.
    pub fn impact_smoke_puff(&mut self, shader: Option<ParticleShader>, origin: Vec3) {
        self.warn_shader(&shader, "CG_ParticleImpactSmokePuff");
        let Some(index) = self.allocate() else {
            return;
        };
        let roll = float_to_int(self.host.random.next_centered() as f32 * 179.0);
        let end_time = self.after(500);
        let start_fade = self.after(100);
        let width = (self.host.random.next_integer() % 4 + 8) as f32;
        let height = (self.host.random.next_integer() % 4 + 8) as f32;
        let particle = &mut self.particles[index];
        particle.alpha = 0.25;
        particle.alpha_velocity = 0.0;
        particle.roll = roll;
        particle.shader = shader;
        particle.end_time = end_time;
        particle.start_fade = start_fade;
        particle.width = width;
        particle.height = height;
        particle.end_height = height * 2.0;
        particle.end_width = width * 2.0;
        particle.particle_type = ParticleType::SmokeImpact;
        particle.origin = origin;
        particle.velocity = vec3(0.0, 0.0, 20.0);
        particle.acceleration = vec3(0.0, 0.0, 20.0);
        particle.rotate = true;
    }

    /// Blood spray particle.
    pub fn bleed(&mut self, shader: Option<ParticleShader>, start: Vec3, flesh_entity_num: i32, duration: i32) {
        self.warn_shader(&shader, "CG_Particle_Bleed");
        let Some(index) = self.allocate() else {
            return;
        };
        let end_time = self.after(duration);
        let start_fade = self.after(if flesh_entity_num != 0 { 0 } else { 100 });
        let growth = self.host.random.next_integer() % 3;
        let roll = self.host.random.next_integer() % 179;
        let particle = &mut self.particles[index];
        particle.alpha = 0.75;
        particle.alpha_velocity = 0.0;
        particle.shader = shader;
        particle.end_time = end_time;
        particle.start_fade = start_fade;
        particle.width = 4.0;
        particle.height = 4.0;
        particle.end_height = 4.0 + growth as f32;
        particle.end_width = particle.end_height;
        particle.particle_type = ParticleType::Smoke;
        particle.origin = start;
        particle.velocity = vec3(0.0, 0.0, -20.0);
        particle.acceleration = vec3(0.0, 0.0, 0.0);
        particle.rotate = false;
        particle.roll = roll;
        particle.color = ParticleColor::Blood;
    }

    /// Dripping oil particle.
    pub fn oil_particle(&mut self, shader: Option<ParticleShader>, entity: &ParticleClientEntity) {
        if shader.is_none() {
            (self.host.print)("CG_Particle_OilParticle == ZERO!\n");
        }
        let ratio = 1.0 - self.state.time / (self.state.time + entity.time as f32).trunc();
        let Some(index) = self.allocate() else {
            return;
        };
        let end_time = self.state.time + 1500.0;
        let roll = self.host.random.next_integer() % 179;
        let particle = &mut self.particles[index];
        particle.alpha = 0.75;
        particle.alpha_velocity = 0.0;
        particle.shader = shader;
        particle.end_time = end_time;
        particle.start_fade = end_time;
        particle.width = 1.0;
        particle.end_width = 1.0;
        particle.height = 3.0;
        particle.end_height = 3.0;
        particle.particle_type = ParticleType::Smoke;
        particle.origin = entity.origin;
        particle.velocity = vec3(
            entity.origin2.x * (16.0 * ratio),
            entity.origin2.y * (16.0 * ratio),
            entity.origin2.z,
        );
        particle.snum = 1;
        particle.acceleration = vec3(0.0, 0.0, -20.0);
        particle.rotate = false;
        particle.roll = roll;
    }

    /// Flat oil slick particle.
    pub fn oil_slick(&mut self, shader: Option<ParticleShader>, entity: &ParticleClientEntity) {
        if shader.is_none() {
            (self.host.print)("CG_Particle_OilSlick == ZERO!\n");
        }
        let Some(index) = self.allocate() else {
            return;
        };
        let end_time = if entity.angles2.z != 0.0 {
            self.state.time + entity.angles2.z
        } else {
            self.after(60000)
        };
        let jitter = self.host.random.next_centered() as f32;
        let roll = self.host.random.next_integer() % 179;
        let particle = &mut self.particles[index];
        particle.end_time = end_time;
        particle.start_fade = end_time;
        particle.alpha = 0.75;
        particle.alpha_velocity = 0.0;
        particle.shader = shader;
        let sized = entity.angles2.x != 0.0 || entity.angles2.y != 0.0;
        particle.width = if sized { entity.angles2.x } else { 8.0 };
        particle.height = particle.width;
        particle.end_height = if sized { entity.angles2.y } else { 16.0 };
        particle.end_width = particle.end_height;
        particle.particle_type = ParticleType::FlatScaleUp;
        particle.snum = 1;
        particle.origin = vec3(entity.origin.x, entity.origin.y, entity.origin.z + 0.55 + jitter * 0.5);
        particle.velocity = vec3(0.0, 0.0, 0.0);
        particle.acceleration = vec3(0.0, 0.0, 0.0);
        particle.rotate = false;
        particle.roll = roll;
    }

    /// Fade out oil slicks.
    pub fn oil_slick_remove(&mut self) {
        let mut index = self.active;
        while index != -1 {
            let next = self.particles[index as usize].next;
            let end_time = self.after(100);
            let particle = &mut self.particles[index as usize];
            if particle.particle_type == ParticleType::FlatScaleUp && particle.snum == 1 {
                particle.end_time = end_time;
                particle.start_fade = end_time;
                particle.particle_type = ParticleType::FlatScaleUpFade;
            }
            index = next;
        }
    }

    /// Whether a blood pool fits on clear ground at a trace end.
    pub fn valid_blood_pool(&self, start: Vec3) -> bool {
        let axes = angle_vectors(vector_to_angles(vec3(0.0, 0.0, 1.0)));
        let center = move_along(start, 0.5, vec3(0.0, 0.0, 1.0));
        let bounds = qa_core::math::Bounds {
            min: vec3(0.0, 0.0, 0.0),
            max: vec3(0.0, 0.0, 0.0),
        };
        let mut x = -8.0f32;
        while x < 16.0 {
            let mut y = -8.0f32;
            while y < 16.0 {
                let point = move_along(move_along(center, x, axes.right), y, axes.up);
                let end = move_along(point, -1.0, vec3(0.0, 0.0, 1.0));
                let trace = self.host.prediction.trace(point, end, bounds, -1, 1);
                if trace.entity_num < ENTITYNUM_WORLD || trace.solidity != TraceSolidity::Clear || trace.fraction >= 1.0
                {
                    return false;
                }
                y += 16.0;
            }
            x += 16.0;
        }
        true
    }

    /// Flat blood pool particle at a trace end.
    pub fn blood_pool(&mut self, shader: Option<ParticleShader>, end: Vec3) {
        self.warn_shader(&shader, "CG_BloodPool");
        if self.free == -1 || !self.valid_blood_pool(end) {
            return;
        }
        let Some(index) = self.allocate() else {
            return;
        };
        let end_time = self.after(3000);
        let size = 0.4 + self.host.random.next_unit() as f32 * 0.6;
        let roll = self.host.random.next_integer() % 179;
        let particle = &mut self.particles[index];
        particle.end_time = end_time;
        particle.start_fade = end_time;
        particle.alpha = 0.75;
        particle.alpha_velocity = 0.0;
        particle.shader = shader;
        particle.width = 8.0 * size;
        particle.height = particle.width;
        particle.end_height = 16.0 * size;
        particle.end_width = particle.end_height;
        particle.particle_type = ParticleType::FlatScaleUp;
        particle.origin = end;
        particle.velocity = vec3(0.0, 0.0, 0.0);
        particle.acceleration = vec3(0.0, 0.0, 0.0);
        particle.rotate = false;
        particle.roll = roll;
        particle.color = ParticleColor::Blood;
    }

    /// Blood cloud puff along a damage direction.
    pub fn blood_cloud(&mut self, origin: Vec3, direction: Vec3) {
        let count = (length3(direction) / 32.0).max(1.0);
        let mut emitted = 0.0f32;
        while emitted < count {
            emitted += 1.0;
            let Some(index) = self.allocate() else {
                return;
            };
            let jitter = self.host.random.next_centered() as f32;
            let roll = self.host.random.next_integer() % 179;
            let end_time = (self.state.time + 350.0).trunc() + jitter * 100.0;
            let shader = self.host.media.smoke_puff_shader.clone();
            let time = self.state.time;
            let particle = &mut self.particles[index];
            particle.alpha = 0.75;
            particle.alpha_velocity = 0.0;
            particle.shader = shader;
            particle.end_time = end_time;
            particle.start_fade = time;
            particle.width = 32.0;
            particle.height = 32.0;
            particle.end_height = 32.0;
            particle.end_width = 32.0;
            particle.particle_type = ParticleType::Smoke;
            particle.origin = origin;
            particle.velocity = vec3(0.0, 0.0, -1.0);
            particle.acceleration = vec3(0.0, 0.0, 0.0);
            particle.rotate = false;
            particle.roll = roll;
            particle.color = ParticleColor::Blood;
        }
    }

    /// Spark shower particle.
    pub fn sparks(&mut self, origin: Vec3, velocity: Vec3, duration: i32, x: f32, y: f32, speed: f32) {
        let Some(index) = self.allocate() else {
            return;
        };
        let end_time = self.after(duration);
        let start_fade = self.after(duration / 2);
        let (jx, jy, vx, vy, vz, ax, ay) = (
            self.host.random.next_centered() as f32,
            self.host.random.next_centered() as f32,
            self.host.random.next_centered() as f32,
            self.host.random.next_centered() as f32,
            self.host.random.next_centered() as f32,
            self.host.random.next_centered() as f32,
            self.host.random.next_centered() as f32,
        );
        let shader = self.host.media.tracer_shader.clone();
        let particle = &mut self.particles[index];
        particle.end_time = end_time;
        particle.start_fade = start_fade;
        particle.color = ParticleColor::EmissiveFade;
        particle.alpha = 0.4;
        particle.alpha_velocity = 0.0;
        particle.height = 0.5;
        particle.width = 0.5;
        particle.end_height = 0.5;
        particle.end_width = 0.5;
        particle.shader = shader;
        particle.particle_type = ParticleType::Smoke;
        particle.origin = vec3(origin.x + jx * x, origin.y + jy * y, origin.z);
        particle.velocity = vec3(
            velocity.x + vx * 4.0,
            velocity.y + vy * 4.0,
            velocity.z + (20.0 + vz * 10.0) * speed,
        );
        particle.acceleration = vec3(ax * 4.0, ay * 4.0, 0.0);
    }

    /// Dust trail along a shot direction, returning the negated direction.
    pub fn dust(&mut self, origin: Vec3, direction: Vec3) -> Vec3 {
        let negated = vec3(-direction.x, -direction.y, -direction.z);
        let length = length3(negated);
        let forward = angle_vectors(vector_to_angles(negated)).forward;
        let count = (length / 32.0).max(1.0);
        let mut point = origin;
        let mut emitted = 0.0f32;
        while emitted < count {
            emitted += 1.0;
            point = move_along(point, 32.0, forward);
            let Some(index) = self.allocate() else {
                break;
            };
            let base = if length != 0.0 { 4500.0 } else { 750.0 };
            let spread = if length != 0.0 { 3500.0 } else { 500.0 };
            let jitter = self.host.random.next_centered() as f32;
            let end_time = (self.state.time + base).trunc() + jitter * spread;
            let (vx, vy, vz) = (
                self.host.random.next_centered() as f32,
                self.host.random.next_centered() as f32,
                self.host.random.next_unit() as f32,
            );
            self.host.random.next_centered();
            self.host.random.next_centered();
            let roll = self.host.random.next_integer() % 179;
            let shader = self.host.media.smoke_puff_shader.clone();
            let time = self.state.time;
            let particle = &mut self.particles[index];
            particle.alpha = 0.75;
            particle.alpha_velocity = 0.0;
            particle.shader = shader;
            particle.end_time = end_time;
            particle.start_fade = time;
            particle.width = if length != 0.0 { 32.0 } else { 32.0 * 0.2 };
            particle.height = particle.width;
            particle.end_height = if length != 0.0 { 96.0 } else { 16.0 };
            particle.end_width = particle.end_height;
            particle.particle_type = ParticleType::Smoke;
            particle.origin = point;
            particle.velocity = vec3(vx * 6.0, vy * 6.0, vz * 20.0);
            particle.acceleration = vec3(0.0, 0.0, 0.0);
            particle.rotate = false;
            particle.roll = roll;
        }
        negated
    }

    /// Billboard sprite particle with constant size.
    pub fn misc(&mut self, shader: Option<ParticleShader>, origin: Vec3, size: f32, duration: i32) {
        self.warn_shader(&shader, "CG_ParticleImpactSmokePuff");
        let Some(index) = self.allocate() else {
            return;
        };
        let roll = self.host.random.next_integer() % 179;
        let end_time = if duration > 0 {
            self.after(duration)
        } else {
            duration as f32
        };
        let time = self.state.time;
        let particle = &mut self.particles[index];
        particle.alpha = 1.0;
        particle.alpha_velocity = 0.0;
        particle.roll = roll;
        particle.shader = shader;
        particle.end_time = end_time;
        particle.start_fade = time;
        particle.width = size;
        particle.height = size;
        particle.end_height = size;
        particle.end_width = size;
        particle.particle_type = ParticleType::Sprite;
        particle.origin = origin;
        particle.rotate = false;
    }

    /// Step the pool and emit one polygon per visible particle.
    pub fn add_particles(&mut self, view_origin: Option<Vec3>) -> Result<Vec<RefPoly>, RenderError> {
        let mut output = Vec::new();
        self.view_axis = self.state.view_axis;
        let angles = vector_to_angles(self.view_axis[0]);
        self.view_roll += (self.state.time - self.old_time) * 0.1;
        let rotated = angle_vectors(vec3(angles.x, angles.y, angles.z + self.view_roll * 0.9));
        self.rotated_axes = [rotated.forward, rotated.right, rotated.up];
        self.old_time = self.state.time;
        let mut head = -1i32;
        let mut tail = -1i32;
        let mut index = self.active;
        while index != -1 {
            let current = index as usize;
            let next = self.particles[current].next;
            index = next;
            let particle = self.particles[current].clone();
            let elapsed = (self.state.time - particle.time) * 0.001;
            let alpha = particle.alpha + elapsed * particle.alpha_velocity;
            let timed = matches!(
                particle.particle_type,
                ParticleType::Smoke
                    | ParticleType::Animated
                    | ParticleType::Bleed
                    | ParticleType::SmokeImpact
                    | ParticleType::WeatherFlurry
                    | ParticleType::FlatScaleUpFade
            );
            if alpha <= 0.0 || (timed && self.state.time > particle.end_time) {
                self.release(current);
                continue;
            }
            if matches!(particle.particle_type, ParticleType::Bat | ParticleType::Sprite) && particle.end_time < 0.0 {
                let origin = particle.origin;
                self.add_to_scene(current, origin, &mut output, view_origin)?;
                self.release(current);
                continue;
            }
            self.particles[current].next = -1;
            if tail == -1 {
                head = current as i32;
            } else {
                self.particles[tail as usize].next = current as i32;
            }
            tail = current as i32;
            let squared = elapsed * elapsed;
            let origin = vec3(
                particle.origin.x + particle.velocity.x * elapsed + particle.acceleration.x * squared,
                particle.origin.y + particle.velocity.y * elapsed + particle.acceleration.y * squared,
                particle.origin.z + particle.velocity.z * elapsed + particle.acceleration.z * squared,
            );
            self.add_to_scene(current, origin, &mut output, view_origin)?;
        }
        self.active = head;
        Ok(output)
    }

    fn distance(&self, origin: Vec3, view_origin: Option<Vec3>) -> Result<f32, RenderError> {
        if let Some(view) = view_origin {
            return Ok(length3(sub3(view, origin)));
        }
        self.state
            .player_origin
            .map(|player| length3(sub3(player, origin)))
            .ok_or_else(|| RenderError::Backend("particle distance culling requires the current snapshot".to_string()))
    }

    /// Lateral view axis as Q3's right vector (our axis stores left).
    fn view_right(&self) -> Vec3 {
        scale3(self.view_axis[1], -1.0)
    }

    fn rolled_axes(&self, roll: i32) -> (Vec3, Vec3) {
        if roll == 0 {
            return (self.view_right(), self.view_axis[2]);
        }
        let angles = vector_to_angles(self.state.view_axis[0]);
        let axes = angle_vectors(vec3(angles.x, angles.y, angles.z + roll as f32));
        (axes.right, axes.up)
    }

    fn add_to_scene(
        &mut self,
        slot: usize,
        origin: Vec3,
        output: &mut Vec<RefPoly>,
        view_origin: Option<Vec3>,
    ) -> Result<(), RenderError> {
        let particle = self.particles[slot].clone();
        let white = vec4(255.0, 255.0, 255.0, 255.0);
        let vertex = |position: Vec3, s: f32, t: f32, color: Vec4| RefPolyVertex {
            position,
            tex_coord: vec2(s, t),
            color,
        };
        let byte = |value: f32| (float_to_int(255.0 * value) & 255) as f32;
        let point =
            |height: f32, width: f32, right: Vec3, up: Vec3| move_along(move_along(origin, height, up), width, right);
        let ratio = (self.state.time - particle.time) / (particle.end_time - particle.time);
        let size = |start: f32, end: f32, amount: f32| start + amount * (end - start);
        let rage_pro = self.host.hardware == ParticleHardware::RagePro;
        match particle.particle_type {
            ParticleType::Weather
            | ParticleType::WeatherTurbulent
            | ParticleType::WeatherFlurry
            | ParticleType::Bubble
            | ParticleType::BubbleTurbulent => {
                let bubble = matches!(
                    particle.particle_type,
                    ParticleType::Bubble | ParticleType::BubbleTurbulent
                );
                if particle.particle_type != ParticleType::WeatherFlurry {
                    if bubble && origin.z > particle.end {
                        let jitter = self.host.random.next_centered() as f32;
                        let slot_particle = &mut self.particles[slot];
                        slot_particle.time = self.state.time;
                        slot_particle.origin = vec3(origin.x, origin.y, particle.start + jitter * 4.0);
                        if particle.particle_type == ParticleType::BubbleTurbulent {
                            let (vx, vy) = (
                                self.host.random.next_centered() as f32,
                                self.host.random.next_centered() as f32,
                            );
                            let slot_particle = &mut self.particles[slot];
                            slot_particle.velocity = vec3(vx * 4.0, vy * 4.0, slot_particle.velocity.z);
                        }
                    } else if !bubble && origin.z < particle.end {
                        let mut z = origin.z;
                        let span = particle.start - particle.end;
                        loop {
                            let next = z + span;
                            if next <= z {
                                return Err(RenderError::BadBatch {
                                    index: 0,
                                    detail: "snow wrap cannot advance at source float32 precision".to_string(),
                                });
                            }
                            z = next;
                            if z >= particle.end {
                                break;
                            }
                        }
                        let slot_particle = &mut self.particles[slot];
                        slot_particle.time = self.state.time;
                        slot_particle.origin = vec3(origin.x, origin.y, z);
                        if particle.particle_type == ParticleType::WeatherTurbulent {
                            let (vx, vy) = (
                                self.host.random.next_centered() as f32,
                                self.host.random.next_centered() as f32,
                            );
                            let slot_particle = &mut self.particles[slot];
                            slot_particle.velocity = vec3(vx * 16.0, vy * 16.0, slot_particle.velocity.z);
                        }
                    }
                    if !particle.link {
                        return Ok(());
                    }
                    self.particles[slot].alpha = 1.0;
                }
                if self.distance(origin, view_origin)? > 1024.0 {
                    return Ok(());
                }
                let color = vec4(255.0, 255.0, 255.0, byte(self.particles[slot].alpha));
                let right = self.view_right();
                let up = self.view_axis[2];
                let vertices = if bubble {
                    vec![
                        vertex(point(-particle.height, -particle.width, right, up), 0.0, 0.0, color),
                        vertex(point(-particle.height, particle.width, right, up), 0.0, 1.0, color),
                        vertex(point(particle.height, particle.width, right, up), 1.0, 1.0, color),
                        vertex(point(particle.height, -particle.width, right, up), 1.0, 0.0, color),
                    ]
                } else {
                    vec![
                        vertex(point(-particle.height, -particle.width, right, up), 1.0, 0.0, color),
                        vertex(point(particle.height, -particle.width, right, up), 0.0, 0.0, color),
                        vertex(point(particle.height, particle.width, right, up), 0.0, 1.0, color),
                    ]
                };
                if particle.shader.is_some() {
                    output.push(RefPoly {
                        shader: particle.shader.clone(),
                        vertices,
                    });
                }
            }
            ParticleType::Sprite => {
                let (right, up) = self.rolled_axes(particle.roll);
                let width = size(particle.width, particle.end_width, ratio);
                let height = size(particle.height, particle.end_height, ratio);
                let a = point(-height, -width, right, up);
                let b = move_along(a, 2.0 * height, up);
                let c = move_along(b, 2.0 * width, right);
                let d = move_along(c, -2.0 * height, up);
                let vertices = vec![
                    vertex(a, 0.0, 0.0, white),
                    vertex(b, 0.0, 1.0, white),
                    vertex(c, 1.0, 1.0, white),
                    vertex(d, 1.0, 0.0, white),
                ];
                if particle.shader.is_some() {
                    output.push(RefPoly {
                        shader: particle.shader.clone(),
                        vertices,
                    });
                }
            }
            ParticleType::Smoke | ParticleType::SmokeImpact => {
                if particle.particle_type == ParticleType::SmokeImpact && self.distance(origin, view_origin)? > 1024.0 {
                    return Ok(());
                }
                let mut color = vec3(1.0, 1.0, 1.0);
                if particle.color == ParticleColor::Blood {
                    color = vec3(0.22, 0.0, 0.0);
                } else if particle.color == ParticleColor::Grey75 {
                    let distance = self.distance(origin, view_origin)?;
                    let grey = (0.25 * (4096.0 / if distance == 0.0 { 1.0 } else { distance })).min(0.5);
                    color = vec3(grey, grey, grey);
                }
                let mut inverse;
                if self.state.time > particle.start_fade {
                    inverse = 1.0 - (self.state.time - particle.start_fade) / (particle.end_time - particle.start_fade);
                    if particle.color == ParticleColor::EmissiveFade {
                        let fade = (inverse * inverse).max(0.0);
                        color = vec3(fade, fade, fade);
                    }
                    inverse *= particle.alpha;
                } else {
                    inverse = particle.alpha;
                }
                if rage_pro {
                    inverse = 1.0;
                }
                if inverse > 1.0 {
                    inverse = 1.0;
                }
                let (right, up) = if particle.particle_type != ParticleType::SmokeImpact {
                    let angles = vector_to_angles(self.rotated_axes[0]);
                    self.particles[slot].accumulated_roll =
                        self.particles[slot].accumulated_roll.wrapping_add(particle.roll);
                    let accumulated = self.particles[slot].accumulated_roll;
                    let axes = angle_vectors(vec3(angles.x, angles.y, angles.z + accumulated as f32 * 0.1));
                    (axes.right, axes.up)
                } else {
                    (self.rotated_axes[1], self.rotated_axes[2])
                };
                let (right, up) = if particle.rotate {
                    (right, up)
                } else {
                    (self.view_right(), self.view_axis[2])
                };
                let width = if particle.rotate {
                    size(particle.width, particle.end_width, ratio)
                } else {
                    particle.width
                };
                let height = if particle.rotate {
                    size(particle.height, particle.end_height, ratio)
                } else {
                    particle.height
                };
                let shaded = vec4(byte(color.x), byte(color.y), byte(color.z), byte(inverse));
                let vertices = vec![
                    vertex(point(-height, -width, right, up), 0.0, 0.0, shaded),
                    vertex(point(-height, width, right, up), 0.0, 1.0, shaded),
                    vertex(point(height, width, right, up), 1.0, 1.0, shaded),
                    vertex(point(height, -width, right, up), 1.0, 0.0, shaded),
                ];
                if particle.shader.is_some() {
                    output.push(RefPoly {
                        shader: particle.shader.clone(),
                        vertices,
                    });
                }
            }
            ParticleType::Bleed => {
                let (right, up) = self.rolled_axes(particle.roll);
                let shaded = vec4(111.0, 19.0, 9.0, byte(if rage_pro { 1.0 } else { particle.alpha }));
                let vertices = vec![
                    vertex(point(-particle.height, -particle.width, right, up), 0.0, 0.0, shaded),
                    vertex(point(-particle.height, particle.width, right, up), 0.0, 1.0, shaded),
                    vertex(point(particle.height, particle.width, right, up), 1.0, 1.0, shaded),
                    vertex(point(particle.height, -particle.width, right, up), 1.0, 0.0, shaded),
                ];
                if particle.shader.is_some() {
                    output.push(RefPoly {
                        shader: particle.shader.clone(),
                        vertices,
                    });
                }
            }
            ParticleType::FlatScaleUp => {
                let width = size(particle.width, particle.end_width, ratio).min(particle.end_width);
                let height = size(particle.height, particle.end_height, ratio).min(particle.end_height);
                let radians = particle.roll as f32 * std::f32::consts::PI / 180.0;
                let root2 = 2.0f32.sqrt();
                let sin = height * radians.sin() * root2;
                let cos = width * radians.cos() * root2;
                let channel = if particle.color == ParticleColor::Blood {
                    255.0
                } else {
                    byte(0.5)
                };
                let color = vec4(channel, channel, channel, 255.0);
                let vertices = vec![
                    vertex(vec3(origin.x - sin, origin.y - cos, origin.z), 0.0, 0.0, color),
                    vertex(vec3(origin.x - cos, origin.y + sin, origin.z), 0.0, 1.0, color),
                    vertex(vec3(origin.x + sin, origin.y + cos, origin.z), 1.0, 1.0, color),
                    vertex(vec3(origin.x + cos, origin.y - sin, origin.z), 1.0, 0.0, color),
                ];
                if particle.shader.is_some() {
                    output.push(RefPoly {
                        shader: particle.shader.clone(),
                        vertices,
                    });
                }
            }
            ParticleType::Flat => {
                let vertices = vec![
                    vertex(
                        vec3(origin.x - particle.height, origin.y - particle.width, origin.z),
                        0.0,
                        0.0,
                        white,
                    ),
                    vertex(
                        vec3(origin.x - particle.height, origin.y + particle.width, origin.z),
                        0.0,
                        1.0,
                        white,
                    ),
                    vertex(
                        vec3(origin.x + particle.height, origin.y + particle.width, origin.z),
                        1.0,
                        1.0,
                        white,
                    ),
                    vertex(
                        vec3(origin.x + particle.height, origin.y - particle.width, origin.z),
                        1.0,
                        0.0,
                        white,
                    ),
                ];
                if particle.shader.is_some() {
                    output.push(RefPoly {
                        shader: particle.shader.clone(),
                        vertices,
                    });
                }
            }
            ParticleType::Animated => {
                let mut amount = ratio;
                if amount >= 1.0 {
                    amount = 0.9999;
                }
                let width = size(particle.width, particle.end_width, amount);
                let height = size(particle.height, particle.end_height, amount);
                if self.distance(origin, view_origin)? < width / 1.5 {
                    return Ok(());
                }
                let animation =
                    self.animations
                        .get(particle.shader_animation)
                        .ok_or_else(|| RenderError::BadBatch {
                            index: particle.shader_animation,
                            detail: "invalid particle animation".to_string(),
                        })?;
                let frame = float_to_int((amount * animation.count as f32).floor());
                let shader = animation
                    .frames
                    .get(frame as usize)
                    .ok_or_else(|| RenderError::BadBatch {
                        index: frame as usize,
                        detail: "particle animation frame outside source range".to_string(),
                    })?;
                self.particles[slot].shader = shader.clone();
                let (right, up) = self.rolled_axes(self.particles[slot].roll);
                let a = point(-height, -width, right, up);
                let b = move_along(a, 2.0 * height, up);
                let c = move_along(b, 2.0 * width, right);
                let d = move_along(c, -2.0 * height, up);
                let vertices = vec![
                    vertex(a, 0.0, 0.0, white),
                    vertex(b, 0.0, 1.0, white),
                    vertex(c, 1.0, 1.0, white),
                    vertex(d, 1.0, 0.0, white),
                ];
                if shader.is_some() {
                    output.push(RefPoly {
                        shader: shader.clone(),
                        vertices,
                    });
                }
            }
            ParticleType::None | ParticleType::Rotate | ParticleType::Bat | ParticleType::FlatScaleUpFade => {
                if particle.shader.is_none() {
                    return Ok(());
                }
                return Err(RenderError::Backend(format!(
                    "source particle type {:?} has no initialized polygon geometry",
                    particle.particle_type
                )));
            }
        }
        Ok(())
    }
}

impl ParticleSystem {
    /// Borrowed host random access for tests.
    #[cfg(test)]
    fn test_counts() -> (usize, usize) {
        (ANIMATION_SPECS[0].1, ANIMATION_SPECS[5].1)
    }
}

/// Animation frame counts (test helper).
#[cfg(test)]
pub fn animation_frame_counts() -> std::collections::HashMap<&'static str, usize> {
    ANIMATION_SPECS.iter().map(|spec| (spec.0, spec.1)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::math::Bounds;

    struct TestResources;

    impl ParticleResources for TestResources {
        fn register_shader(&mut self, name: &str) -> Option<ParticleShader> {
            Some(ParticleShader::named(name))
        }
    }

    struct ClearTracer;

    impl ParticleTracer for ClearTracer {
        fn trace(
            &self,
            _start: Vec3,
            end: Vec3,
            _bounds: Bounds,
            _pass: i32,
            _contents: i32,
        ) -> super::super::q3_types::ParticleTrace {
            super::super::q3_types::ParticleTrace {
                end,
                entity_num: ENTITYNUM_WORLD,
                solidity: TraceSolidity::Clear,
                fraction: 0.5,
            }
        }
    }

    fn host(standalone: bool) -> ParticleHost {
        let mut resources = TestResources;
        let animations = if standalone {
            ParticleAnimationSet::Standalone(load_standalone_particle_animations(&mut resources))
        } else {
            ParticleAnimationSet::Marks(load_particle_animations(&mut resources))
        };
        ParticleHost {
            animations,
            media: ParticleMedia {
                tracer_shader: Some(ParticleShader::named("tracer")),
                smoke_puff_shader: Some(ParticleShader::named("smoke")),
                water_bubble_shader: Some(ParticleShader::named("bubble")),
            },
            prediction: Box::new(ClearTracer),
            random: Qrand::new(1234, 0),
            hardware: ParticleHardware::Generic,
            config_string: Box::new(|_| String::new()),
            print: Box::new(|_| {}),
        }
    }

    fn state() -> ParticleClientState {
        ParticleClientState {
            time: 1000.0,
            view_axis: [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)],
            player_origin: Some(vec3(0.0, 0.0, 0.0)),
        }
    }

    fn entity() -> ParticleClientEntity {
        ParticleClientEntity {
            origin: vec3(0.0, 0.0, 0.0),
            origin2: vec3(0.0, 0.0, 64.0),
            angles: vec3(0.0, 0.0, 0.0),
            angles2: vec3(0.0, 0.0, 0.0),
            time: 500,
            time2: 250,
            frame: 0,
        }
    }

    #[test]
    fn animation_counts_match_source() {
        assert_eq!(ParticleSystem::test_counts(), (23, 5));
        assert_eq!(animation_frame_counts()["twiltb2"], 45);
    }

    #[test]
    fn marks_profile_has_small_pool() {
        let system = ParticleSystem::new(state(), host(false)).expect("system");
        assert_eq!(system.capacity(), MAX_PARTICLES);
        assert_eq!(system.active_count(), 0);
    }

    #[test]
    fn standalone_profile_has_large_pool() {
        let system = ParticleSystem::new(state(), host(true)).expect("system");
        assert_eq!(system.capacity(), 8192);
    }

    #[test]
    fn explosion_emits_animated_poly() {
        let mut system = ParticleSystem::new(state(), host(false)).expect("system");
        system
            .explosion(&ParticleExplosionRequest {
                animation: "explode1",
                origin: vec3(200.0, 0.0, 0.0),
                velocity: vec3(0.0, 0.0, 0.0),
                duration: 500,
                size_start: 16,
                size_end: 64,
            })
            .expect("explosion");
        assert_eq!(system.active_count(), 1);
        let polys = system.add_particles(Some(vec3(0.0, 0.0, 0.0))).expect("polys");
        assert_eq!(polys.len(), 1);
        assert_eq!(polys[0].vertices.len(), 4);
    }

    #[test]
    fn unknown_explosion_animation_errors() {
        let mut system = ParticleSystem::new(state(), host(false)).expect("system");
        let result = system.explosion(&ParticleExplosionRequest {
            animation: "missing",
            origin: vec3(0.0, 0.0, 0.0),
            velocity: vec3(0.0, 0.0, 0.0),
            duration: 500,
            size_start: 16,
            size_end: 64,
        });
        assert!(result.is_err());
    }

    #[test]
    fn smoke_and_sparks_emit_polys() {
        let mut system = ParticleSystem::new(state(), host(false)).expect("system");
        system.smoke(Some(ParticleShader::named("smoke")), &entity());
        system.sparks(vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 10.0), 500, 4.0, 4.0, 1.0);
        system.bullet_debris(vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0), 500);
        assert_eq!(system.active_count(), 3);
        let polys = system.add_particles(Some(vec3(0.0, 0.0, 0.0))).expect("polys");
        assert_eq!(polys.len(), 3);
    }

    #[test]
    fn expired_particles_release() {
        let mut system = ParticleSystem::new(state(), host(false)).expect("system");
        system.misc(Some(ParticleShader::named("sprite")), vec3(0.0, 0.0, 0.0), 8.0, -1);
        assert_eq!(system.active_count(), 1);
        // Negative end times render once, then release.
        let polys = system.add_particles(Some(vec3(0.0, 0.0, 0.0))).expect("polys");
        assert_eq!(polys.len(), 1);
        assert_eq!(system.active_count(), 0);
    }

    #[test]
    fn weather_wraps_and_links() {
        let mut system = ParticleSystem::new(state(), host(true)).expect("system");
        system
            .snow(
                Some(ParticleShader::named("snow")),
                vec3(0.0, 0.0, 128.0),
                vec3(0.0, 0.0, 0.0),
                false,
                64.0,
                3,
            )
            .expect("snow");
        let mut linked = entity();
        linked.frame = 3;
        system.snow_link(&linked, false);
        let polys = system.add_particles(Some(vec3(0.0, 0.0, 64.0))).expect("polys");
        assert!(polys.is_empty());
        assert_eq!(system.active_count(), 1);
    }

    #[test]
    fn blood_pool_validates_ground() {
        let mut system = ParticleSystem::new(state(), host(true)).expect("system");
        assert!(system.valid_blood_pool(vec3(0.0, 0.0, 0.0)));
        system.blood_pool(Some(ParticleShader::named("blood")), vec3(0.0, 0.0, 0.0));
        assert_eq!(system.active_count(), 1);
    }

    #[test]
    fn oil_slick_fades_on_remove() {
        let mut system = ParticleSystem::new(state(), host(true)).expect("system");
        system.oil_slick(Some(ParticleShader::named("oil")), &entity());
        assert_eq!(system.active_count(), 1);
        system.oil_slick_remove();
        system.set_time(1200.0, state().view_axis);
        let polys = system.add_particles(Some(vec3(0.0, 0.0, 0.0))).expect("polys");
        assert!(polys.is_empty());
        assert_eq!(system.active_count(), 0);
    }

    #[test]
    fn dust_returns_negated_direction() {
        let mut system = ParticleSystem::new(state(), host(true)).expect("system");
        let negated = system.dust(vec3(0.0, 0.0, 0.0), vec3(32.0, 0.0, 0.0));
        assert_eq!(negated, vec3(-32.0, 0.0, -0.0));
        assert!(system.active_count() >= 1);
    }

    #[test]
    fn config_area_spawns_weather() {
        let mut host = host(true);
        host.config_string = Box::new(|_| "0 0 0 128 0 0 0 4 0 9".to_string());
        let mut system = ParticleSystem::new(state(), host).expect("system");
        assert!(system.new_particle_area(0).expect("area"));
        assert_eq!(system.active_count(), 4);
    }

    #[test]
    fn shrapnel_is_noop() {
        let system = ParticleSystem::new(state(), host(false)).expect("system");
        system.add_particle_shrapnel();
        assert_eq!(system.active_count(), 0);
    }
}
