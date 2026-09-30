//! Quake III presentation: weapons.
//!
//! Donor provenance: `src/content/q3/presentation/weapons.ts`.

use qa_core::math::{
    add3, angle_mod, angle_vectors, angles_to_axis, dot3, length3, normalize3, perpendicular_vector,
    rotate_point_around_vector, scale3, sub3, vec2, vec3, vec4, Axis, Bounds, Vec3, Vec4,
};
use qa_core::numeric::qvm_float_to_int;
use std::collections::HashSet;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::presentation::effects::*;
use crate::q3::presentation::entities::*;
use crate::q3::presentation::local_entities::*;
use crate::q3::presentation::marks::*;
use crate::q3::presentation::mirrors_present_scene::*;
use crate::q3::presentation::model_access::*;
use crate::q3::presentation::movement_host::*;
use crate::q3::presentation::ref_entity::*;

// ---------------------------------------------------------------------------
// weapons.ts
// ---------------------------------------------------------------------------

/// Water contents.
pub const CONTENTS_WATER: i32 = 32;

/// Shot mask.
pub const MASK_SHOT: i32 = 1 | 0x2000000 | 0x4000000;

/// No-impact surface flag.
pub const SURF_NOIMPACT: i32 = 16;

/// Metal-steps surface flag.
pub const SURF_METALSTEPS: i32 = 4096;

/// Impact sound (`ImpactSound`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum ImpactSound {
    /// Default.
    Default = 0,
    /// Metal.
    Metal = 1,
    /// Flesh.
    Flesh = 2,
}

/// Shotgun trace result.
#[derive(Debug, Clone, PartialEq)]
pub struct ShotgunTrace<Target> {
    /// End.
    pub end: Vec3,
    /// Normal.
    pub normal: Vec3,
    /// Surface flags.
    pub surface_flags: i32,
    /// Target.
    pub target: Option<Target>,
}

/// Shotgun presentation host (`ShotgunPresentationHost`).
pub trait ShotgunPresentationHost<Target> {
    /// Smoke enabled.
    fn smoke_enabled(&self) -> bool;
    /// Trace.
    fn trace(&self, start: Vec3, end: Vec3) -> ShotgunTrace<Target>;
    /// Water boundary.
    fn water(&self, start: Vec3, end: Vec3) -> Vec3;
    /// Contents.
    fn contents(&self, point: Vec3) -> i32;
    /// Whether a target is a player.
    fn is_player(&self, target: &Target) -> bool;
    /// Blood at a point.
    fn blood(&mut self, point: Vec3, normal: Vec3, target: Target);
    /// Wall impact.
    fn wall(&mut self, point: Vec3, normal: Vec3, sound: ImpactSound);
    /// Bubbles.
    fn bubbles(&mut self, start: Vec3, end: Vec3);
    /// Smoke.
    fn smoke(&mut self, origin: Vec3);
}

/// Emit shotgun presentation (`emitShotgunPresentation`).
pub fn emit_shotgun_presentation<Target>(host: &mut dyn ShotgunPresentationHost<Target>, shot: &Q3ShotgunEvent) {
    if host.smoke_enabled() && host.contents(shot.muzzle) & CONTENTS_WATER == 0 {
        let direction = normalize3(sub3(shot.direction, shot.muzzle));
        host.smoke(add3(shot.muzzle, scale3(direction, 32.0)));
    }
    for end in q3_shotgun_endpoints(shot.muzzle, shot.direction, shot.seed) {
        let trace = host.trace(shot.muzzle, end);
        let source_contents = host.contents(shot.muzzle);
        let destination_contents = host.contents(trace.end);
        if source_contents == destination_contents {
            if source_contents & CONTENTS_WATER != 0 {
                host.bubbles(shot.muzzle, trace.end);
            }
        } else if source_contents & CONTENTS_WATER != 0 {
            let water = host.water(end, shot.muzzle);
            host.bubbles(shot.muzzle, water);
        } else if destination_contents & CONTENTS_WATER != 0 {
            let water = host.water(shot.muzzle, end);
            host.bubbles(trace.end, water);
        }
        if trace.surface_flags & SURF_NOIMPACT != 0 {
            continue;
        }
        match trace.target {
            Some(target) if host.is_player(&target) => host.blood(trace.end, trace.normal, target),
            _ => host.wall(
                trace.end,
                trace.normal,
                if trace.surface_flags & SURF_METALSTEPS != 0 {
                    ImpactSound::Metal
                } else {
                    ImpactSound::Default
                },
            ),
        }
    }
}

/// Bullet hit (`BulletHit`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BulletHit {
    /// Wall.
    Wall {
        /// Normal.
        normal: Vec3,
    },
    /// Flesh.
    Flesh {
        /// Entity number.
        entity_num: i32,
    },
}

/// Ejected brass kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EjectBrass {
    /// Machinegun.
    Machinegun,
    /// Shotgun.
    Shotgun,
    /// Nailgun.
    Nailgun,
}

/// Client weapon info (`ClientWeaponInfo`).
#[derive(Debug, Clone, PartialEq)]
pub struct ClientWeaponInfo {
    /// Packet info.
    pub packet: PacketWeaponInfo,
    /// Item.
    pub item: Option<ItemDefinition>,
    /// Hands model.
    pub hands_model: SceneModel,
    /// Flash model.
    pub flash_model: SceneModel,
    /// Ammo model.
    pub ammo_model: SceneModel,
    /// Weapon icon.
    pub weapon_icon: Option<SceneShader>,
    /// Ammo icon.
    pub ammo_icon: Option<SceneShader>,
    /// Flash light color.
    pub flash_dlight_color: Vec3,
    /// Flash sounds.
    pub flash_sounds: [Option<PresentSound>; 4],
    /// Ejected brass.
    pub eject_brass: Option<EjectBrass>,
    /// Ready sound.
    pub ready_sound: Option<PresentSound>,
    /// Firing sound.
    pub firing_sound: Option<PresentSound>,
    /// Looping fire sound.
    pub loop_fire_sound: bool,
}

pub(crate) fn empty_packet_weapon() -> PacketWeaponInfo {
    PacketWeaponInfo {
        weapon_model: default_model(),
        weapon_midpoint: zero_vec3(),
        barrel_model: None,
        missile_model: default_model(),
        missile_renderfx: 0,
        missile_sound: None,
        missile_dlight: 0.0,
        missile_dlight_color: zero_vec3(),
        missile_trail: None,
        trail_radius: 0.0,
        trail_time: 0,
    }
}

pub(crate) fn empty_weapon() -> ClientWeaponInfo {
    ClientWeaponInfo {
        packet: empty_packet_weapon(),
        item: None,
        hands_model: default_model(),
        flash_model: default_model(),
        ammo_model: default_model(),
        weapon_icon: None,
        ammo_icon: None,
        flash_dlight_color: zero_vec3(),
        flash_sounds: [None, None, None, None],
        eject_brass: None,
        ready_sound: None,
        firing_sound: None,
        loop_fire_sound: false,
    }
}

/// Client weapon selection (`ClientWeaponSelection`).
#[derive(Default)]
pub struct ClientWeaponSelection {
    /// Selection callback.
    pub on_select: Option<Box<dyn FnMut(i32)>>,
}

impl ClientWeaponSelection {
    /// New selection.
    #[must_use]
    pub fn new() -> Self {
        Self { on_select: None }
    }

    fn selectable(&self, state: &ClientGameState, number: i32) -> PresentResult<bool> {
        let snap = state
            .snap
            .as_ref()
            .ok_or_else(|| PresentError::state("CG_WeaponSelectable: cg.snap == NULL"))?;
        Ok(snap.player_state.ammo.get(number as usize) != 0
            && snap.player_state.stats.get(stat_schema(state.product).weapons) & (1 << number) != 0)
    }

    /// Next weapon.
    pub fn next_weapon(&mut self, state: &mut ClientGameState) -> PresentResult<()> {
        self.cycle_weapon(state, 1)
    }

    /// Previous weapon.
    pub fn previous_weapon(&mut self, state: &mut ClientGameState) -> PresentResult<()> {
        self.cycle_weapon(state, -1)
    }

    fn cycle_weapon(&mut self, state: &mut ClientGameState, direction: i32) -> PresentResult<()> {
        let followed = state
            .snap
            .as_ref()
            .is_some_and(|snap| snap.player_state.pm_flags & MoveFlags::FOLLOW != 0);
        if state.snap.is_none() || followed {
            return Ok(());
        }
        state.weapon_select_time = state.time;
        let original = state.weapon_select;
        for _ in 0..16 {
            state.weapon_select = (state.weapon_select + direction + 16) % 16;
            if state.weapon_select != Weapon::Gauntlet as i32 && self.selectable(state, state.weapon_select)? {
                let selected = state.weapon_select;
                if let Some(on_select) = self.on_select.as_mut() {
                    on_select(selected);
                }
                return Ok(());
            }
        }
        state.weapon_select = original;
        Ok(())
    }

    /// Select a weapon.
    pub fn select_weapon(&mut self, state: &mut ClientGameState, number: i32) {
        let followed = state
            .snap
            .as_ref()
            .is_some_and(|snap| snap.player_state.pm_flags & MoveFlags::FOLLOW != 0);
        if state.snap.is_none() || followed || !(1..=15).contains(&number) {
            return;
        }
        state.weapon_select_time = state.time;
        let has = state
            .snap
            .as_ref()
            .is_some_and(|snap| snap.player_state.stats.get(stat_schema(state.product).weapons) & (1 << number) != 0);
        if has {
            state.weapon_select = number;
            if let Some(on_select) = self.on_select.as_mut() {
                on_select(number);
            }
        }
    }

    /// Change on empty (`outOfAmmoChange`).
    pub fn out_of_ammo_change(&mut self, state: &mut ClientGameState) -> PresentResult<()> {
        state.weapon_select_time = state.time;
        for number in (1..=15).rev() {
            if self.selectable(state, number)? {
                state.weapon_select = number;
                return Ok(());
            }
        }
        Ok(())
    }
}

/// Weapon presentation models.
#[derive(Debug, Clone, PartialEq)]
pub struct WeaponPresentationModels {
    /// Machinegun brass.
    pub machinegun_brass: SceneModel,
    /// Shotgun brass.
    pub shotgun_brass: SceneModel,
    /// Dish flash.
    pub dish_flash: SceneModel,
    /// Ring flash.
    pub ring_flash: SceneModel,
    /// Bullet flash.
    pub bullet_flash: SceneModel,
}

/// Weapon presentation shaders.
#[derive(Debug, Clone, PartialEq)]
pub struct WeaponPresentationShaders {
    /// Smoke puff.
    pub smoke_puff: Option<SceneShader>,
    /// Nail puff.
    pub nail_puff: Option<SceneShader>,
    /// Shotgun smoke puff.
    pub shotgun_smoke_puff: Option<SceneShader>,
    /// Invisibility.
    pub invis: Option<SceneShader>,
    /// Battle weapon.
    pub battle_weapon: Option<SceneShader>,
    /// Quad weapon.
    pub quad_weapon: Option<SceneShader>,
    /// Select.
    pub select: Option<SceneShader>,
    /// No ammo.
    pub noammo: Option<SceneShader>,
    /// Hole mark.
    pub hole_mark: Option<SceneShader>,
    /// Burn mark.
    pub burn_mark: Option<SceneShader>,
    /// Energy mark.
    pub energy_mark: Option<SceneShader>,
    /// Bullet mark.
    pub bullet_mark: Option<SceneShader>,
    /// Tracer.
    pub tracer: Option<SceneShader>,
}

/// Weapon presentation sounds.
#[derive(Debug, Clone, PartialEq)]
pub struct WeaponPresentationSounds {
    /// Quad.
    pub quad: Option<PresentSound>,
    /// Nail hit flesh.
    pub nail_hit_flesh: Option<PresentSound>,
    /// Nail hit metal.
    pub nail_hit_metal: Option<PresentSound>,
    /// Nail hit.
    pub nail_hit: Option<PresentSound>,
    /// Prox explosion.
    pub prox_explosion: Option<PresentSound>,
    /// Rocket explosion.
    pub rocket_explosion: Option<PresentSound>,
    /// Plasma explosion.
    pub plasma_explosion: Option<PresentSound>,
    /// Chaingun hit flesh.
    pub chaingun_hit_flesh: Option<PresentSound>,
    /// Chaingun hit metal.
    pub chaingun_hit_metal: Option<PresentSound>,
    /// Chaingun hit.
    pub chaingun_hit: Option<PresentSound>,
    /// Ricochet 1.
    pub ricochet1: Option<PresentSound>,
    /// Ricochet 2.
    pub ricochet2: Option<PresentSound>,
    /// Ricochet 3.
    pub ricochet3: Option<PresentSound>,
    /// Tracer.
    pub tracer: Option<PresentSound>,
}

/// Weapon presentation media (`WeaponPresentationMedia`).
#[derive(Debug, Clone, PartialEq)]
pub struct WeaponPresentationMedia {
    /// Models.
    pub models: WeaponPresentationModels,
    /// Shaders.
    pub shaders: WeaponPresentationShaders,
    /// Sounds.
    pub sounds: WeaponPresentationSounds,
}

/// Weapon presentation settings (`WeaponPresentationSettings`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WeaponPresentationSettings {
    /// Brass time.
    pub brass_time: i32,
    /// Rail trail time.
    pub rail_trail_time: i32,
    /// Old rail.
    pub old_rail: bool,
    /// No projectile trail.
    pub no_projectile_trail: bool,
    /// Old plasma.
    pub old_plasma: bool,
    /// Old rocket.
    pub old_rocket: bool,
    /// True lightning blend.
    pub true_lightning: f32,
    /// Draw gun.
    pub draw_gun: bool,
    /// FOV.
    pub fov: f32,
    /// Gun X.
    pub gun_x: f32,
    /// Gun Y.
    pub gun_y: f32,
    /// Gun Z.
    pub gun_z: f32,
    /// Gun frame override.
    pub gun_frame: i32,
    /// Tracer length.
    pub tracer_length: f32,
    /// Tracer width.
    pub tracer_width: f32,
    /// Tracer chance.
    pub tracer_chance: f32,
    /// Rage Pro hardware.
    pub hardware_rage_pro: bool,
}

/// Weapon selection drawing (`WeaponSelectionDrawing`).
pub trait WeaponSelectionDrawing {
    /// Fade color.
    fn fade_color(&self, start: i32, duration: i32) -> Option<Vec4>;
    /// Set color.
    fn set_color(&mut self, color: Option<Vec4>);
    /// Draw a picture.
    fn draw_pic(&mut self, x: i32, y: i32, width: i32, height: i32, shader: Option<SceneShader>);
    /// Draw string length.
    fn draw_string_length(&self, text: &str) -> usize;
    /// Draw a big string.
    fn draw_big_string_color(&mut self, x: i32, y: i32, text: &str, color: Vec4);
}

/// Client animation reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientAnimRef {
    /// First frame.
    pub first_frame: i32,
}

/// Client info view (`ClientInfo`, minimal mirror).
#[derive(Debug, Clone, PartialEq)]
pub struct ClientInfoView {
    /// Color 1.
    pub color1: Vec3,
    /// Color 2.
    pub color2: Vec3,
    /// Animations.
    pub animations: Vec<Option<ClientAnimRef>>,
}

/// Client weapon host (`ClientWeaponHost`).
pub trait ClientWeaponHost {
    /// Add a reference entity.
    fn add_ref_entity(&mut self, entity: RefEntity);
    /// Add a light.
    fn add_light(&mut self, light: DynamicLight);
    /// Start a sound.
    fn start_sound(&mut self, origin: Option<Vec3>, entity: i32, channel: i32, sound: Option<PresentSound>);
    /// Add a loop sound.
    fn add_loop_sound(
        &mut self,
        entity: i32,
        origin: Vec3,
        velocity: Vec3,
        sound: Option<PresentSound>,
        real_loop: bool,
    );
    /// Trace with entity skipping.
    fn trace_mover(&self, start: Vec3, end: Vec3, bounds: Bounds, skip_number: i32, mask: i32) -> MovementTrace;
    /// Point contents with entity passing.
    fn point_contents_pred(&self, point: Vec3, pass_entity: i32) -> i32;
    /// Raw shape trace.
    fn collision_trace(&self, start: Vec3, end: Vec3, mask: i32) -> TraceResult;
    /// Raw point contents.
    fn collision_contents(&self, point: Vec3) -> i32;
    /// Random integer.
    fn rand_i32(&mut self) -> i32;
    /// Random fraction.
    fn random_f32(&mut self) -> f32;
    /// Centered random fraction.
    fn crandom_f32(&mut self) -> f32;
    /// Effects.
    fn weapon_effects(&mut self) -> &mut ClientEffects;
    /// Project an impact mark.
    fn impact_mark(&mut self, request: &ImpactMarkRequest) -> Vec<RefPoly>;
    /// Particle explosion.
    fn particle_explosion(&mut self, request: &ParticleExplosion);
    /// Media.
    fn weapon_media(&self) -> &WeaponPresentationMedia;
    /// Settings.
    fn weapon_settings(&self) -> WeaponPresentationSettings;
    /// Client info.
    fn client_info_view(&self, number: i32) -> ClientInfoView;
    /// Load a sound synchronously.
    fn load_sound(&mut self, path: &str) -> Option<PresentSound>;
    /// Add a polygon.
    fn add_poly(&mut self, poly: RefPoly);
    /// Drawing.
    fn drawing(&mut self) -> &mut dyn WeaponSelectionDrawing;
}

pub(crate) fn weapon_ma(origin: Vec3, scale: f32, direction: Vec3) -> Vec3 {
    add3(origin, scale3(direction, scale))
}

pub(crate) fn weapon_transform(value: Vec3, axis: &Axis) -> Vec3 {
    vec3(
        dot3(value, vec3(axis[0].x, axis[1].x, axis[2].x)),
        dot3(value, vec3(axis[0].y, axis[1].y, axis[2].y)),
        dot3(value, vec3(axis[0].z, axis[1].z, axis[2].z)),
    )
}

pub(crate) fn weapon_bytes(color: Vec3, scale: f32, alpha: f32) -> Vec4 {
    vec4(
        (qvm_float_to_int(color.x * scale) & 255) as f32,
        (qvm_float_to_int(color.y * scale) & 255) as f32,
        (qvm_float_to_int(color.z * scale) & 255) as f32,
        alpha,
    )
}

/// Client weapon runtime (`ClientWeaponRuntime`).
pub struct ClientWeaponRuntime {
    /// Selection.
    pub selection: ClientWeaponSelection,
    /// Registry.
    pub registry: ClientWeaponMediaRegistry,
    /// Host.
    pub host: Box<dyn ClientWeaponHost>,
}

impl ClientWeaponRuntime {
    /// New runtime.
    pub fn new(
        product: Product,
        registry: ClientWeaponMediaRegistry,
        host: Box<dyn ClientWeaponHost>,
    ) -> PresentResult<Self> {
        if product != registry.product {
            return Err(PresentError::state("Weapon media product differs from cgame state"));
        }
        Ok(Self {
            selection: ClientWeaponSelection::new(),
            registry,
            host,
        })
    }

    /// Fire a weapon (`fireWeapon`).
    pub fn fire_weapon(
        &mut self,
        state: &mut ClientGameState,
        pool: &mut LocalEntityPool,
        frame: &EffectFrame,
        target: PresentEntityTarget,
    ) -> PresentResult<()> {
        let ent = match target {
            PresentEntityTarget::Predicted => state.predicted_player_entity.current_state.clone(),
            PresentEntityTarget::Indexed(index) => state.entity_at(index).current_state.clone(),
        };
        if ent.weapon == Weapon::None as i32 {
            return Ok(());
        }
        if ent.weapon >= weapon_count(state.product) {
            return Err(PresentError::drop("CG_FireWeapon: ent->weapon >= WP_NUM_WEAPONS"));
        }
        let weapon = self.registry.weapon(ent.weapon)?.clone();
        {
            let time = state.time;
            let entity = match target {
                PresentEntityTarget::Predicted => &mut state.predicted_player_entity,
                PresentEntityTarget::Indexed(index) => state.entity_at(index),
            };
            entity.muzzle_flash_time = time;
        }
        let lightning_firing = match target {
            PresentEntityTarget::Predicted => state.predicted_player_entity.player.lightning_firing,
            PresentEntityTarget::Indexed(index) => state.entity_at(index).player.lightning_firing,
        };
        if ent.weapon == Weapon::Lightning as i32 && lightning_firing != 0 {
            return Ok(());
        }
        if ent.powerups & (1 << Powerup::Quad as i32) != 0 {
            let quad = self.host.weapon_media().sounds.quad.clone();
            self.host.start_sound(None, ent.number, 4, quad);
        }
        let length = weapon
            .flash_sounds
            .iter()
            .position(|sound| sound.is_none())
            .unwrap_or(4);
        if length > 0 {
            let sound = weapon.flash_sounds[(self.host.rand_i32() % length as i32).max(0) as usize % 4].clone();
            if sound.is_some() {
                self.host.start_sound(None, ent.number, 2, sound);
            }
        }
        if weapon.eject_brass.is_some() && self.host.weapon_settings().brass_time > 0 {
            self.eject_brass(
                state,
                pool,
                frame,
                target,
                weapon.eject_brass.unwrap_or(EjectBrass::Machinegun),
            )?;
        }
        Ok(())
    }

    fn eject_brass(
        &mut self,
        state: &mut ClientGameState,
        pool: &mut LocalEntityPool,
        frame: &EffectFrame,
        target: PresentEntityTarget,
        kind: EjectBrass,
    ) -> PresentResult<()> {
        let (lerp_origin, lerp_angles) = match target {
            PresentEntityTarget::Predicted => (
                state.predicted_player_entity.lerp_origin,
                state.predicted_player_entity.lerp_angles,
            ),
            PresentEntityTarget::Indexed(index) => {
                (state.entity_at(index).lerp_origin, state.entity_at(index).lerp_angles)
            }
        };
        let axis = angles_to_axis(lerp_angles);
        let time = state.time;
        if kind == EjectBrass::Nailgun {
            let shader = self.host.weapon_media().shaders.smoke_puff.clone();
            let handle = self.host.weapon_effects().smoke_puff(
                pool,
                frame,
                &SmokePuffOptions {
                    origin: add3(lerp_origin, weapon_transform(vec3(0.0, -12.0, 24.0), &axis)),
                    velocity: vec3(0.0, 0.0, 64.0),
                    radius: 32.0,
                    color: vec4(1.0, 1.0, 1.0, 0.33),
                    duration: 700,
                    start_time: time,
                    fade_in_time: 0,
                    flags: 0,
                    shader,
                },
            )?;
            if let Some(smoke) = pool.get_mut(handle) {
                smoke.le_type = LocalEntityType::ScaleFade;
            }
            return Ok(());
        }
        let brass_time = self.host.weapon_settings().brass_time;
        if brass_time <= 0 {
            return Ok(());
        }
        let shotgun = kind == EjectBrass::Shotgun;
        for i in 0..if shotgun { 2 } else { 1 } {
            let model = if shotgun {
                self.host.weapon_media().models.shotgun_brass.clone()
            } else {
                self.host.weapon_media().models.machinegun_brass.clone()
            };
            let re = create_model_entity(model);
            let handle = pool.allocate(LocalEntityType::Fragment, RefEntity::Model(re))?;
            let velocity = if shotgun {
                vec3(
                    60.0 + 60.0 * self.host.crandom_f32(),
                    (if i == 0 { 40.0 } else { -40.0 }) + 10.0 * self.host.crandom_f32(),
                    100.0 + 50.0 * self.host.crandom_f32(),
                )
            } else {
                vec3(
                    0.0,
                    -50.0 + 40.0 * self.host.crandom_f32(),
                    100.0 + 50.0 * self.host.crandom_f32(),
                )
            };
            let random = self.host.random_f32();
            let rand_bits = self.host.rand_i32();
            let end_time = qvm_float_to_int(
                time.wrapping_add(brass_time * if shotgun { 3 } else { 1 }) as f32
                    + (if shotgun { brass_time } else { brass_time / 4 }) as f32 * random,
            );
            let pos_time = if shotgun {
                time
            } else {
                time.wrapping_sub(rand_bits & 15)
            };
            let origin = add3(
                lerp_origin,
                weapon_transform(vec3(8.0, if shotgun { 0.0 } else { -4.0 }, 24.0), &axis),
            );
            let water = if self.host.point_contents_pred(origin, -1) & CONTENTS_WATER != 0 {
                0.1
            } else {
                1.0
            };
            if let Some(le) = pool.get_mut(handle) {
                le.start_time = time;
                le.end_time = end_time;
                le.pos = Trajectory {
                    type_: TrajectoryType::Gravity,
                    time: pos_time,
                    duration: 0,
                    base: origin,
                    delta: scale3(weapon_transform(velocity, &axis), water),
                };
                if let RefEntity::Model(re) = &mut le.ref_entity {
                    re.origin = origin;
                    re.axis = identity_axis_full();
                }
                le.bounce_factor = if shotgun { 0.3 } else { 0.4 * water };
                le.angles = Trajectory {
                    type_: TrajectoryType::Linear,
                    time,
                    duration: 0,
                    base: vec3(
                        (self.host.rand_i32() & 31) as f32,
                        (self.host.rand_i32() & 31) as f32,
                        (self.host.rand_i32() & 31) as f32,
                    ),
                    delta: if shotgun {
                        vec3(1.0, 0.5, 0.0)
                    } else {
                        vec3(2.0, 1.0, 0.0)
                    },
                };
                le.le_flags = LE_TUMBLE;
                le.le_bounce_sound_type = LocalBounceSoundType::Brass;
                le.le_mark_type = LocalMarkType::None;
            }
        }
        Ok(())
    }

    /// Rail trail (`railTrail`).
    pub fn rail_trail(
        &mut self,
        time: i32,
        pool: &mut LocalEntityPool,
        frame: &EffectFrame,
        client_num: i32,
        start: &mut Vec3,
        end: Vec3,
    ) -> PresentResult<()> {
        let effects = self.registry.effects.clone();
        emit_rail_trail(time, &effects, &mut *self.host, pool, frame, client_num, start, end)
    }

    /// Missile trail (`missileTrail`).
    pub fn missile_trail(
        &mut self,
        state: &mut ClientGameState,
        pool: &mut LocalEntityPool,
        frame: &EffectFrame,
        kind: MissileTrail,
        target: PresentEntityTarget,
        weapon: &PacketWeaponInfo,
    ) -> PresentResult<()> {
        if kind == MissileTrail::Grapple {
            self.grapple_trail(state, target)?;
            return Ok(());
        }
        if kind == MissileTrail::Plasma {
            self.plasma_trail(state, pool, target)?;
            return Ok(());
        }
        if self.host.weapon_settings().no_projectile_trail {
            return Ok(());
        }
        let (pos, trail_time) = {
            let entity = match target {
                PresentEntityTarget::Predicted => &state.predicted_player_entity,
                PresentEntityTarget::Indexed(index) => state.entity_at(index),
            };
            (entity.current_state.pos, entity.trail_time)
        };
        let time = state.time;
        let mut t = 50 * ((trail_time + 50) / 50);
        let origin = evaluate_trajectory(&pos, time);
        let contents = self.host.point_contents_pred(origin, -1);
        if pos.type_ == TrajectoryType::Stationary {
            let entity = match target {
                PresentEntityTarget::Predicted => &mut state.predicted_player_entity,
                PresentEntityTarget::Indexed(index) => state.entity_at(index),
            };
            entity.trail_time = time;
            return Ok(());
        }
        let previous = evaluate_trajectory(&pos, trail_time);
        let last_contents = self.host.point_contents_pred(previous, -1);
        {
            let entity = match target {
                PresentEntityTarget::Predicted => &mut state.predicted_player_entity,
                PresentEntityTarget::Indexed(index) => state.entity_at(index),
            };
            entity.trail_time = time;
        }
        if contents & (32 | 16 | 8) != 0 {
            if contents & last_contents & CONTENTS_WATER != 0 {
                self.host
                    .weapon_effects()
                    .bubble_trail(pool, frame, previous, origin, 8.0)?;
            }
            return Ok(());
        }
        while t <= time {
            let shader = if kind == MissileTrail::Nail {
                self.host.weapon_media().shaders.nail_puff.clone()
            } else {
                self.host.weapon_media().shaders.smoke_puff.clone()
            };
            let handle = self.host.weapon_effects().smoke_puff(
                pool,
                frame,
                &SmokePuffOptions {
                    origin: evaluate_trajectory(&pos, t),
                    velocity: zero_vec3(),
                    radius: weapon.trail_radius,
                    color: vec4(1.0, 1.0, 1.0, 0.33),
                    duration: weapon.trail_time,
                    start_time: t,
                    fade_in_time: 0,
                    flags: 0,
                    shader,
                },
            )?;
            if let Some(smoke) = pool.get_mut(handle) {
                smoke.le_type = LocalEntityType::ScaleFade;
            }
            t += 50;
        }
        Ok(())
    }

    fn plasma_trail(
        &mut self,
        state: &ClientGameState,
        pool: &mut LocalEntityPool,
        target: PresentEntityTarget,
    ) -> PresentResult<()> {
        let (pos, lerp_angles, weapon) = match target {
            PresentEntityTarget::Predicted => (
                state.predicted_player_entity.current_state.pos,
                state.predicted_player_entity.lerp_angles,
                state.predicted_player_entity.current_state.weapon,
            ),
            PresentEntityTarget::Indexed(index) => match state.entity_ref(index) {
                Some(entity) => (
                    entity.current_state.pos,
                    entity.lerp_angles,
                    entity.current_state.weapon,
                ),
                None => return Ok(()),
            },
        };
        let origin = evaluate_trajectory(&pos, state.time);
        let effects = self.registry.effects.clone();
        let flash_color = self.registry.weapon(weapon)?.flash_dlight_color;
        emit_plasma_trail(
            state.time,
            origin,
            lerp_angles,
            flash_color,
            effects.rail_rings_shader.clone(),
            &mut *self.host,
            pool,
        )
    }

    /// Grapple trail (`grappleTrail`).
    pub fn grapple_trail(&mut self, state: &mut ClientGameState, target: PresentEntityTarget) -> PresentResult<()> {
        let (pos, other) = match target {
            PresentEntityTarget::Predicted => (
                state.predicted_player_entity.current_state.pos,
                state.predicted_player_entity.current_state.other_entity_num,
            ),
            PresentEntityTarget::Indexed(index) => (
                state.entity_at(index).current_state.pos,
                state.entity_at(index).current_state.other_entity_num,
            ),
        };
        let origin = evaluate_trajectory(&pos, state.time);
        {
            let time = state.time;
            let entity = match target {
                PresentEntityTarget::Predicted => &mut state.predicted_player_entity,
                PresentEntityTarget::Indexed(index) => state.entity_at(index),
            };
            entity.trail_time = time;
        }
        let owner = state.entity_at(other.max(0) as usize).clone();
        let Some((start, end)) = q3_grapple_cable(owner.lerp_origin, angle_vectors(owner.lerp_angles).up, origin)
        else {
            return Ok(());
        };
        let mut beam = create_lightning_entity();
        beam.origin = start;
        beam.old_origin = end;
        beam.shading.custom_shader = self.registry.effects.lightning_shader.clone();
        beam.shading.shader_rgba = vec4(255.0, 255.0, 255.0, 255.0);
        self.host.add_ref_entity(RefEntity::Lightning(beam));
        Ok(())
    }

    /// Missile hit wall (`missileHitWall`).
    #[allow(clippy::too_many_arguments)]
    pub fn missile_hit_wall(
        &mut self,
        product: Product,
        pool: &mut LocalEntityPool,
        frame: &EffectFrame,
        weapon: Weapon,
        client_num: i32,
        origin: Vec3,
        direction: Vec3,
        sound_type: ImpactSound,
    ) -> PresentResult<()> {
        let effects = self.registry.effects.clone();
        emit_weapon_impact(
            product,
            &effects,
            &mut *self.host,
            pool,
            frame,
            weapon,
            client_num,
            origin,
            direction,
            sound_type,
        )
    }

    /// Missile hit player (`missileHitPlayer`).
    #[allow(clippy::too_many_arguments)]
    pub fn missile_hit_player(
        &mut self,
        product: Product,
        pool: &mut LocalEntityPool,
        frame: &EffectFrame,
        weapon: Weapon,
        origin: Vec3,
        direction: Vec3,
        entity_num: i32,
    ) -> PresentResult<()> {
        self.host.weapon_effects().bleed(pool, frame, origin, entity_num)?;
        if weapon == Weapon::GrenadeLauncher
            || weapon == Weapon::RocketLauncher
            || (product == Product::MissionPack
                && (weapon == Weapon::Nailgun || weapon == Weapon::Chaingun || weapon == Weapon::ProxLauncher))
        {
            self.missile_hit_wall(product, pool, frame, weapon, 0, origin, direction, ImpactSound::Flesh)?;
        }
        Ok(())
    }

    /// Shotgun fire (`shotgunFire`).
    pub fn shotgun_fire(
        &mut self,
        state: &mut ClientGameState,
        pool: &mut LocalEntityPool,
        frame: &EffectFrame,
        es: &EntityState,
    ) -> PresentResult<()> {
        struct Adapter<'a> {
            host: &'a mut dyn ClientWeaponHost,
            state: &'a ClientGameState,
            pool: &'a mut LocalEntityPool,
            frame: EffectFrame,
            product: Product,
            effects_registry: RegisteredWeaponEffects,
            shooter: i32,
            smoke_enabled: bool,
            error: Option<PresentError>,
        }
        impl ShotgunPresentationHost<i32> for Adapter<'_> {
            fn smoke_enabled(&self) -> bool {
                self.smoke_enabled
            }
            fn trace(&self, start: Vec3, end: Vec3) -> ShotgunTrace<i32> {
                let zero = zero_vec3();
                let trace = self
                    .host
                    .trace_mover(start, end, Bounds { min: zero, max: zero }, self.shooter, MASK_SHOT);
                ShotgunTrace {
                    end: trace.base.end,
                    normal: match trace.base.contact {
                        TraceContact::Plane { plane } => plane.normal,
                        TraceContact::None => zero_vec3(),
                    },
                    surface_flags: trace.base.surface_flags,
                    target: Some(trace.entity_num),
                }
            }
            fn water(&self, start: Vec3, end: Vec3) -> Vec3 {
                self.host.collision_trace(start, end, CONTENTS_WATER).end
            }
            fn contents(&self, point: Vec3) -> i32 {
                self.host.collision_contents(point)
            }
            fn is_player(&self, target: &i32) -> bool {
                self.state
                    .entity_ref((*target).max(0) as usize)
                    .is_some_and(|entity| entity.current_state.e_type == EntityType::Player as i32)
            }
            fn blood(&mut self, point: Vec3, _normal: Vec3, target: i32) {
                // Shotgun flesh routes through missileHitPlayer, which bleeds
                // only for shotguns (no wall impact).
                let result = self.host.weapon_effects().bleed(self.pool, &self.frame, point, target);
                if self.error.is_none() {
                    self.error = result.err();
                }
            }
            fn wall(&mut self, point: Vec3, normal: Vec3, sound: ImpactSound) {
                let effects = self.effects_registry.clone();
                let result = emit_weapon_impact(
                    self.product,
                    &effects,
                    self.host,
                    self.pool,
                    &self.frame,
                    Weapon::Shotgun,
                    0,
                    point,
                    normal,
                    sound,
                );
                if self.error.is_none() {
                    self.error = result.err();
                }
            }
            fn bubbles(&mut self, start: Vec3, end: Vec3) {
                let result = self
                    .host
                    .weapon_effects()
                    .bubble_trail(self.pool, &self.frame, start, end, 32.0);
                if self.error.is_none() {
                    self.error = result.err();
                }
            }
            fn smoke(&mut self, origin: Vec3) {
                let shader = self.host.weapon_media().shaders.shotgun_smoke_puff.clone();
                let time = self.frame.time;
                let result = self.host.weapon_effects().smoke_puff(
                    self.pool,
                    &self.frame,
                    &SmokePuffOptions {
                        origin,
                        velocity: vec3(0.0, 0.0, 8.0),
                        radius: 32.0,
                        color: vec4(1.0, 1.0, 1.0, 0.33),
                        duration: 900,
                        start_time: time,
                        fade_in_time: 0,
                        flags: LE_PUFF_DONT_SCALE,
                        shader,
                    },
                );
                if self.error.is_none() {
                    self.error = result.err().map(|_| PresentError::state("shotgun smoke failed"));
                }
            }
        }
        let smoke_enabled = !self.host.weapon_settings().hardware_rage_pro;
        let effects_registry = self.registry.effects.clone();
        let product = state.product;
        let mut adapter = Adapter {
            host: &mut *self.host,
            state: &*state,
            pool,
            frame: *frame,
            product,
            effects_registry,
            shooter: es.other_entity_num,
            smoke_enabled,
            error: None,
        };
        emit_shotgun_presentation(
            &mut adapter,
            &Q3ShotgunEvent {
                muzzle: es.pos.base,
                direction: es.origin2,
                seed: es.event_parm,
            },
        );
        if let Some(error) = adapter.error {
            return Err(error);
        }
        Ok(())
    }

    fn muzzle_point(&mut self, state: &mut ClientGameState, entity_num: i32) -> PresentResult<Option<Vec3>> {
        let snap = state
            .snap
            .clone()
            .ok_or_else(|| PresentError::state("CG_CalcMuzzlePoint: cg.snap == NULL"))?;
        if entity_num == snap.player_state.client_num {
            let origin = add3(snap.player_state.origin, vec3(0.0, 0.0, snap.player_state.viewheight));
            return Ok(Some(weapon_ma(
                origin,
                14.0,
                angle_vectors(snap.player_state.viewangles).forward,
            )));
        }
        let cent = state.entity_at(entity_num.max(0) as usize).clone();
        if !cent.current_valid {
            return Ok(None);
        }
        let anim = cent.current_state.legs_anim & !128;
        let height = if anim == PlayerAnimation::LEGS_WALKCR || anim == PlayerAnimation::LEGS_IDLECR {
            12.0
        } else {
            26.0
        };
        Ok(Some(weapon_ma(
            add3(cent.current_state.pos.base, vec3(0.0, 0.0, height)),
            14.0,
            angle_vectors(cent.current_state.apos.base).forward,
        )))
    }

    /// Bullet impact (`bullet`).
    pub fn bullet(
        &mut self,
        state: &mut ClientGameState,
        pool: &mut LocalEntityPool,
        frame: &EffectFrame,
        end: Vec3,
        source: i32,
        hit: BulletHit,
    ) -> PresentResult<()> {
        if source >= 0 && self.host.weapon_settings().tracer_chance > 0.0 {
            if let Some(start) = self.muzzle_point(state, source)? {
                let a = self.host.collision_contents(start);
                let b = self.host.collision_contents(end);
                if a == b && a & CONTENTS_WATER != 0 {
                    self.host.weapon_effects().bubble_trail(pool, frame, start, end, 32.0)?;
                } else if a & CONTENTS_WATER != 0 {
                    let water = self.host.collision_trace(end, start, CONTENTS_WATER).end;
                    self.host
                        .weapon_effects()
                        .bubble_trail(pool, frame, start, water, 32.0)?;
                } else if b & CONTENTS_WATER != 0 {
                    let water = self.host.collision_trace(start, end, CONTENTS_WATER).end;
                    self.host.weapon_effects().bubble_trail(pool, frame, water, end, 32.0)?;
                }
                if self.host.random_f32() < self.host.weapon_settings().tracer_chance {
                    self.tracer(state, source, start, end);
                }
            }
        }
        match hit {
            BulletHit::Flesh { entity_num } => {
                self.host.weapon_effects().bleed(pool, frame, end, entity_num)?;
            }
            BulletHit::Wall { normal } => {
                self.missile_hit_wall(
                    state.product,
                    pool,
                    frame,
                    Weapon::Machinegun,
                    0,
                    end,
                    normal,
                    ImpactSound::Default,
                )?;
            }
        }
        Ok(())
    }

    /// Tracer (`tracer`).
    pub fn tracer(&mut self, state: &ClientGameState, _source: i32, source: Vec3, destination: Vec3) {
        let delta = sub3(destination, source);
        let length = length3(delta);
        if length < 100.0 {
            return;
        }
        let forward = normalize3(delta);
        let settings = self.host.weapon_settings();
        let begin = 50.0 + self.host.random_f32() * (length - 60.0);
        let end = (begin + settings.tracer_length).min(length);
        let start = weapon_ma(source, begin, forward);
        let finish = weapon_ma(source, end, forward);
        let axis = state.refdef.view_axis;
        let right = normalize3(weapon_ma(
            scale3(axis[1], dot3(forward, axis[2])),
            -dot3(forward, axis[1]),
            axis[2],
        ));
        let white = vec4(255.0, 255.0, 255.0, 255.0);
        let shader = self.host.weapon_media().shaders.tracer.clone();
        let tracer_sound = self.host.weapon_media().sounds.tracer.clone();
        self.host.add_poly(RefPoly {
            shader,
            vertices: vec![
                RefPolyVertex {
                    position: weapon_ma(finish, settings.tracer_width, right),
                    tex_coord: vec2(0.0, 1.0),
                    color: white,
                },
                RefPolyVertex {
                    position: weapon_ma(finish, -settings.tracer_width, right),
                    tex_coord: vec2(1.0, 0.0),
                    color: white,
                },
                RefPolyVertex {
                    position: weapon_ma(start, -settings.tracer_width, right),
                    tex_coord: vec2(1.0, 1.0),
                    color: white,
                },
                RefPolyVertex {
                    position: weapon_ma(start, settings.tracer_width, right),
                    tex_coord: vec2(0.0, 0.0),
                    color: white,
                },
            ],
        });
        self.host
            .start_sound(Some(scale3(add3(start, finish), 0.5)), 1022, 0, tracer_sound);
    }

    fn lightning_bolt(
        &mut self,
        state: &mut ClientGameState,
        target: PresentEntityTarget,
        origin: Vec3,
    ) -> PresentResult<()> {
        let cent = match target {
            PresentEntityTarget::Predicted => state.predicted_player_entity.clone(),
            PresentEntityTarget::Indexed(index) => state.entity_at(index).clone(),
        };
        if cent.current_state.weapon != Weapon::Lightning as i32 {
            return Ok(());
        }
        let mut angles = cent.lerp_angles;
        let true_lightning = self.host.weapon_settings().true_lightning;
        if cent.current_state.number == state.predicted_player_state.client_num && true_lightning != 0.0 {
            let view = state.refdef_view_angles;
            let blend = |actual: f32, view: f32| {
                let mut a = actual - view;
                if a > 180.0 {
                    a -= 360.0;
                }
                if a < -180.0 {
                    a += 360.0;
                }
                let mut angle = view + a * (1.0 - true_lightning);
                if angle < 0.0 {
                    angle += 360.0;
                }
                if angle > 360.0 {
                    angle -= 360.0;
                }
                angle
            };
            angles = vec3(
                blend(angles.x, view.x),
                blend(angles.y, view.y),
                blend(angles.z, view.z),
            );
        }
        let forward = angle_vectors(angles).forward;
        let muzzle = weapon_ma(add3(cent.lerp_origin, vec3(0.0, 0.0, 26.0)), 14.0, forward);
        let zero = zero_vec3();
        let trace = self.host.trace_mover(
            muzzle,
            weapon_ma(muzzle, 768.0, forward),
            Bounds { min: zero, max: zero },
            cent.current_state.number,
            MASK_SHOT,
        );
        let mut beam = create_lightning_entity();
        beam.origin = origin;
        beam.old_origin = trace.base.end;
        beam.shading.custom_shader = self.registry.effects.lightning_shader.clone();
        self.host.add_ref_entity(RefEntity::Lightning(beam.clone()));
        if trace.base.fraction < 1.0 {
            let mut re = create_model_entity(self.registry.effects.lightning_explosion_model.clone());
            re.origin = weapon_ma(trace.base.end, -16.0, normalize3(sub3(beam.old_origin, beam.origin)));
            re.axis = angles_to_axis(vec3(
                (self.host.rand_i32() % 360) as f32,
                (self.host.rand_i32() % 360) as f32,
                (self.host.rand_i32() % 360) as f32,
            ));
            self.host.add_ref_entity(RefEntity::Model(re));
        }
        Ok(())
    }

    fn spin_angle(&mut self, state: &mut ClientGameState, target: PresentEntityTarget) -> f32 {
        let player = match target {
            PresentEntityTarget::Predicted => state.predicted_player_entity.player,
            PresentEntityTarget::Indexed(index) => state.entity_at(index).player,
        };
        let mut delta = state.time.wrapping_sub(player.barrel_time);
        let angle = if player.barrel_spinning {
            player.barrel_angle + delta as f32 * 0.9
        } else {
            if delta > 1000 {
                delta = 1000;
            }
            let speed = 0.5 * (0.9 + ((1000 - delta) as f32) / 1000.0);
            player.barrel_angle + delta as f32 * speed
        };
        let current = match target {
            PresentEntityTarget::Predicted => state.predicted_player_entity.current_state.clone(),
            PresentEntityTarget::Indexed(index) => state.entity_at(index).current_state.clone(),
        };
        let firing = current.e_flags & 256 != 0;
        if player.barrel_spinning != firing {
            let time = state.time;
            let entity = match target {
                PresentEntityTarget::Predicted => &mut state.predicted_player_entity,
                PresentEntityTarget::Indexed(index) => state.entity_at(index),
            };
            entity.player.barrel_time = time;
            entity.player.barrel_angle = angle_mod(f64::from(angle)) as f32;
            entity.player.barrel_spinning = firing;
            if state.product == Product::MissionPack && current.weapon == Weapon::Chaingun as i32 && !firing {
                let sound = self.host.load_sound("sound/weapons/vulcan/wvulwind.wav");
                self.host.start_sound(None, current.number, 2, sound);
            }
        }
        angle
    }

    fn add_weapon_with_powerups(&mut self, gun: &RefModelEntity, powerups: i32) {
        let shaders = self.host.weapon_media().shaders.clone();
        if powerups & (1 << Powerup::Invis as i32) != 0 {
            let mut gun = gun.clone();
            gun.shading.custom_shader = shaders.invis;
            self.host.add_ref_entity(RefEntity::Model(gun));
            return;
        }
        self.host.add_ref_entity(RefEntity::Model(gun.clone()));
        if powerups & (1 << Powerup::Battlesuit as i32) != 0 {
            let mut gun = gun.clone();
            gun.shading.custom_shader = shaders.battle_weapon;
            self.host.add_ref_entity(RefEntity::Model(gun));
        }
        if powerups & (1 << Powerup::Quad as i32) != 0 {
            let mut gun = gun.clone();
            gun.shading.custom_shader = shaders.quad_weapon;
            self.host.add_ref_entity(RefEntity::Model(gun));
        }
    }

    /// Add a player weapon (`addPlayerWeapon`).
    #[allow(clippy::too_many_arguments)]
    pub fn add_player_weapon(
        &mut self,
        state: &mut ClientGameState,
        pool: &mut LocalEntityPool,
        frame: &EffectFrame,
        parent: &RefModelEntity,
        ps: Option<&SourcePlayerState>,
        target: PresentEntityTarget,
        _team: Team,
    ) -> PresentResult<()> {
        let cent = match target {
            PresentEntityTarget::Predicted => state.predicted_player_entity.clone(),
            PresentEntityTarget::Indexed(index) => state.entity_at(index).clone(),
        };
        let weapon_num = cent.current_state.weapon;
        let weapon = self.registry.require_weapon(weapon_num)?.clone();
        let attached = |model: SceneModel| {
            let mut re = create_model_entity(model);
            re.lighting_origin = parent.lighting_origin;
            re.shadow_plane = parent.shadow_plane;
            re.shading.render_flags = parent.shading.render_flags;
            re
        };
        let mut gun = attached(weapon.packet.weapon_model.clone());
        if ps.is_some() {
            if state.predicted_player_state.weapon == Weapon::Railgun as i32
                && state.predicted_player_state.weapon_state == WeaponState::Firing
            {
                let fraction = state.predicted_player_state.weapon_time as f32 / 1500.0;
                let color = (qvm_float_to_int(255.0 * (1.0 - fraction)) & 255) as f32;
                gun.shading.shader_rgba = vec4(color, 0.0, color, 0.0);
            } else {
                gun.shading.shader_rgba = vec4(255.0, 255.0, 255.0, 255.0);
            }
        }
        if gun.model.is_default() {
            return Ok(());
        }
        if ps.is_none() {
            let entity = match target {
                PresentEntityTarget::Predicted => &mut state.predicted_player_entity,
                PresentEntityTarget::Indexed(index) => state.entity_at(index),
            };
            entity.player.lightning_firing = 0;
            if cent.current_state.e_flags & 256 != 0 && weapon.firing_sound.is_some() {
                self.host.add_loop_sound(
                    cent.current_state.number,
                    cent.lerp_origin,
                    zero_vec3(),
                    weapon.firing_sound.clone(),
                    false,
                );
                let entity = match target {
                    PresentEntityTarget::Predicted => &mut state.predicted_player_entity,
                    PresentEntityTarget::Indexed(index) => state.entity_at(index),
                };
                entity.player.lightning_firing = 1;
            } else if weapon.ready_sound.is_some() {
                self.host.add_loop_sound(
                    cent.current_state.number,
                    cent.lerp_origin,
                    zero_vec3(),
                    weapon.ready_sound.clone(),
                    false,
                );
            }
        }
        position_entity_on_tag(&mut gun, parent, &parent.model, "tag_weapon");
        self.add_weapon_with_powerups(&gun, cent.current_state.powerups);
        if let Some(barrel_model) = weapon.packet.barrel_model.clone() {
            let mut barrel = attached(barrel_model);
            let spin = self.spin_angle(state, target);
            barrel.axis = angles_to_axis(vec3(0.0, 0.0, spin));
            position_rotated_entity_on_tag(&mut barrel, &gun, &weapon.packet.weapon_model, "tag_barrel");
            self.add_weapon_with_powerups(&barrel, cent.current_state.powerups);
        }
        let non_predicted_index = cent.current_state.client_num.max(0) as usize;
        let non_predicted = state.entity_at(non_predicted_index).clone();
        if !((weapon_num == Weapon::Lightning as i32
            || weapon_num == Weapon::Gauntlet as i32
            || weapon_num == Weapon::GrapplingHook as i32)
            && non_predicted.current_state.e_flags & 256 != 0)
        {
            let railgun_flash = match target {
                PresentEntityTarget::Predicted => state.predicted_player_entity.player.railgun_flash,
                PresentEntityTarget::Indexed(index) => state.entity_at(index).player.railgun_flash,
            };
            if state.time.wrapping_sub(cent.muzzle_flash_time) > 20 && !railgun_flash {
                return Ok(());
            }
        }
        let mut flash = attached(weapon.flash_model.clone());
        if flash.model.is_default() {
            return Ok(());
        }
        flash.axis = angles_to_axis(vec3(0.0, 0.0, self.host.crandom_f32() * 10.0));
        if weapon_num == Weapon::Railgun as i32 {
            let color = self.host.client_info_view(cent.current_state.client_num).color1;
            flash.shading.shader_rgba = weapon_bytes(color, 255.0, 0.0);
        }
        position_rotated_entity_on_tag(&mut flash, &gun, &weapon.packet.weapon_model, "tag_flash");
        self.host.add_ref_entity(RefEntity::Model(flash.clone()));
        if ps.is_some()
            || state.rendering_third_person
            || cent.current_state.number != state.predicted_player_state.client_num
        {
            self.lightning_bolt(state, PresentEntityTarget::Indexed(non_predicted_index), flash.origin)?;
            if weapon_num == Weapon::Railgun as i32 {
                let railgun_flash = match target {
                    PresentEntityTarget::Predicted => state.predicted_player_entity.player.railgun_flash,
                    PresentEntityTarget::Indexed(index) => state.entity_at(index).player.railgun_flash,
                };
                if railgun_flash {
                    let impact = match target {
                        PresentEntityTarget::Predicted => state.predicted_player_entity.player.railgun_impact,
                        PresentEntityTarget::Indexed(index) => state.entity_at(index).player.railgun_impact,
                    };
                    {
                        let entity = match target {
                            PresentEntityTarget::Predicted => &mut state.predicted_player_entity,
                            PresentEntityTarget::Indexed(index) => state.entity_at(index),
                        };
                        entity.player.railgun_flash = true;
                    }
                    let mut start = flash.origin;
                    self.rail_trail(
                        state.time,
                        pool,
                        frame,
                        cent.current_state.client_num,
                        &mut start,
                        impact,
                    )?;
                }
            }
            let color = weapon.flash_dlight_color;
            if color.x != 0.0 || color.y != 0.0 || color.z != 0.0 {
                let radius = 300.0 + (self.host.rand_i32() & 31) as f32;
                self.host.add_light(DynamicLight {
                    origin: flash.origin,
                    radius,
                    color,
                    additive: false,
                });
            }
        }
        Ok(())
    }

    fn weapon_position(&self, state: &ClientGameState) -> (Vec3, Vec3) {
        let scale = if state.bob_cycle & 1 != 0 {
            -state.xyspeed
        } else {
            state.xyspeed
        };
        let roll = scale * state.bob_frac_sin * 0.005;
        let yaw = scale * state.bob_frac_sin * 0.01;
        let pitch = state.xyspeed * state.bob_frac_sin * 0.005;
        let mut origin = state.refdef.view_origin;
        let mut angles = add3(state.refdef_view_angles, vec3(pitch, yaw, roll));
        let delta = state.time.wrapping_sub(state.land_time);
        if delta < 150 {
            origin = add3(origin, vec3(0.0, 0.0, state.land_change * 0.25 * delta as f32 / 150.0));
        } else if delta < 450 {
            origin = add3(
                origin,
                vec3(0.0, 0.0, state.land_change * 0.25 * (450 - delta) as f32 / 300.0),
            );
        }
        let drift = (state.xyspeed + 40.0) * (state.time as f32 * 0.001).sin() * 0.01;
        angles = add3(angles, vec3(drift, drift, drift));
        (origin, angles)
    }

    fn map_torso_frame(&mut self, client_num: i32, frame: i32) -> PresentResult<i32> {
        let animations = self.host.client_info_view(client_num).animations;
        for index in [
            PlayerAnimation::TORSO_DROP,
            PlayerAnimation::TORSO_ATTACK,
            PlayerAnimation::TORSO_ATTACK2,
        ] {
            let animation = animations
                .get(index)
                .and_then(|slot| *slot)
                .ok_or_else(|| PresentError::state(format!("Missing weapon torso animation {index}")))?;
            let length = if index == PlayerAnimation::TORSO_DROP { 9 } else { 6 };
            if frame >= animation.first_frame && frame < animation.first_frame + length {
                return Ok(frame - animation.first_frame + if index == PlayerAnimation::TORSO_DROP { 6 } else { 1 });
            }
        }
        Ok(0)
    }

    /// Add the view weapon (`addViewWeapon`).
    pub fn add_view_weapon(
        &mut self,
        state: &mut ClientGameState,
        pool: &mut LocalEntityPool,
        frame: &EffectFrame,
        ps: &SourcePlayerState,
    ) -> PresentResult<()> {
        if ps.persistant.get(PersistentIndex::PERS_TEAM) == Team::Spectator as i32
            || ps.pm_type == MoveType::Intermission
            || state.rendering_third_person
        {
            return Ok(());
        }
        let settings = self.host.weapon_settings();
        if !settings.draw_gun {
            if state.predicted_player_state.e_flags & 256 != 0 {
                let origin = weapon_ma(state.refdef.view_origin, -8.0, state.refdef.view_axis[2]);
                let target = PresentEntityTarget::Indexed(ps.client_num.max(0) as usize);
                self.lightning_bolt(state, target, origin)?;
            }
            return Ok(());
        }
        if state.test_gun {
            return Ok(());
        }
        let offset = if settings.fov > 90.0 {
            -0.2 * (settings.fov - 90.0)
        } else {
            0.0
        };
        let cent = state.predicted_player_entity.clone();
        let weapon = self.registry.require_weapon(ps.weapon)?.clone();
        let (position_origin, position_angles) = self.weapon_position(state);
        let mut hand = create_model_entity(weapon.hands_model.clone());
        hand.origin = weapon_ma(
            weapon_ma(
                weapon_ma(position_origin, settings.gun_x, state.refdef.view_axis[0]),
                settings.gun_y,
                state.refdef.view_axis[1],
            ),
            settings.gun_z + offset,
            state.refdef.view_axis[2],
        );
        hand.axis = angles_to_axis(position_angles);
        if settings.gun_frame != 0 {
            hand.frame = settings.gun_frame;
            hand.old_frame = settings.gun_frame;
            hand.back_lerp = 0.0;
        } else {
            let client_num = cent.current_state.client_num;
            hand.frame = self.map_torso_frame(client_num, cent.player.torso.frame)?;
            hand.old_frame = self.map_torso_frame(client_num, cent.player.torso.old_frame)?;
            hand.back_lerp = cent.player.torso.back_lerp;
        }
        hand.shading.render_flags = RF_DEPTHHACK | RF_FIRST_PERSON | RF_MINLIGHT;
        let team = ps.persistant.get(PersistentIndex::PERS_TEAM);
        if Team::from_i32(team).is_none() {
            return Err(PresentError::range("Invalid view weapon team"));
        }
        let team = Team::from_i32(team).unwrap_or(Team::Free);
        let ps_owned = ps.clone();
        self.add_player_weapon(
            state,
            pool,
            frame,
            &hand,
            Some(&ps_owned),
            PresentEntityTarget::Predicted,
            team,
        )
    }

    /// Draw the weapon selection (`drawWeaponSelect`).
    pub fn draw_weapon_select(&mut self, state: &mut ClientGameState) -> PresentResult<()> {
        if state.predicted_player_state.health <= 0 {
            return Ok(());
        }
        let color = match self.host.drawing().fade_color(state.weapon_select_time, 1400) {
            Some(color) => color,
            None => return Ok(()),
        };
        self.host.drawing().set_color(Some(color));
        state.item_pickup_time = 0;
        let snap = state
            .snap
            .clone()
            .ok_or_else(|| PresentError::state("CG_DrawWeaponSelect: cg.snap == NULL"))?;
        let bits = snap.player_state.stats.get(stat_schema(state.product).weapons);
        let mut count = 0;
        for i in 1..16 {
            if bits & (1 << i) != 0 {
                count += 1;
            }
        }
        let mut x = 320 - count * 20;
        for i in 1..16 {
            if bits & (1 << i) == 0 {
                continue;
            }
            let icon = self.registry.require_weapon(i)?.weapon_icon.clone();
            self.host.drawing().draw_pic(x, 380, 32, 32, icon);
            if i == state.weapon_select {
                let select = self.host.weapon_media().shaders.select.clone();
                self.host.drawing().draw_pic(x - 4, 376, 40, 40, select);
            }
            if snap.player_state.ammo.get(i as usize) == 0 {
                let noammo = self.host.weapon_media().shaders.noammo.clone();
                self.host.drawing().draw_pic(x, 380, 32, 32, noammo);
            }
            x += 40;
        }
        let item = self.registry.weapon(state.weapon_select)?.item.clone();
        if let Some(item) = item {
            if let Some(name) = item.pickup_name {
                let width = self.host.drawing().draw_string_length(&name) * 16;
                self.host
                    .drawing()
                    .draw_big_string_color((640 - width as i32) / 2, 358, &name, color);
            }
        }
        self.host.drawing().set_color(None);
        Ok(())
    }
}

/// Registered weapon effects (`RegisteredWeaponEffects`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RegisteredWeaponEffects {
    /// Lightning shader.
    pub lightning_shader: Option<SceneShader>,
    /// Lightning explosion model.
    pub lightning_explosion_model: SceneModel,
    /// Lightning hit sounds.
    pub lightning_hit_sounds: [Option<PresentSound>; 3],
    /// Bullet explosion shader.
    pub bullet_explosion_shader: Option<SceneShader>,
    /// Rocket explosion shader.
    pub rocket_explosion_shader: Option<SceneShader>,
    /// Grenade explosion shader.
    pub grenade_explosion_shader: Option<SceneShader>,
    /// Plasma explosion shader.
    pub plasma_explosion_shader: Option<SceneShader>,
    /// Rail explosion shader.
    pub rail_explosion_shader: Option<SceneShader>,
    /// BFG explosion shader.
    pub bfg_explosion_shader: Option<SceneShader>,
    /// Rail rings shader.
    pub rail_rings_shader: Option<SceneShader>,
    /// Rail core shader.
    pub rail_core_shader: Option<SceneShader>,
}

impl Default for SceneModel {
    fn default() -> Self {
        Self::default_model()
    }
}

/// Renderer resources (`RendererResources`, minimal mirror, synchronous).
pub trait PresentRendererResources {
    /// Register a model.
    fn register_model(&mut self, path: &str) -> SceneModel;
    /// Register a skin.
    fn register_skin(&mut self, path: &str) -> Option<SceneSkin>;
    /// Register a shader.
    fn register_shader(&mut self, name: &str) -> Option<SceneShader>;
    /// Register a shader without mipmaps.
    fn register_shader_no_mip(&mut self, name: &str) -> Option<SceneShader>;
    /// Load a world.
    fn load_world(&mut self, mapname: &str) -> PresentWorldScene;
    /// Load particle animations.
    fn load_particle_animations(&mut self) -> PresentParticleAnimations;
}

/// Loaded world scene (`WorldScene`, minimal mirror).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresentWorldScene {
    /// Submodel count.
    pub model_count: usize,
}

/// Particle animations (`ParticleAnimations`, minimal mirror).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresentParticleAnimations {
    /// Animation names.
    pub names: Vec<String>,
}

/// Weapon registration audio (`WeaponRegistrationAudio`, synchronous).
pub trait WeaponRegistrationAudio {
    /// Register a sound (uncompressed).
    fn register_sound(&mut self, path: &str) -> Option<PresentSound>;
}

/// Client weapon media registry (`ClientWeaponMediaRegistry`).
pub struct ClientWeaponMediaRegistry {
    /// Product.
    pub product: Product,
    /// Resources.
    pub resources: Box<dyn PresentRendererResources>,
    /// Audio.
    pub audio: Box<dyn WeaponRegistrationAudio>,
    /// Effects.
    pub effects: RegisteredWeaponEffects,
    weapon_records: [ClientWeaponInfo; 16],
    item_records: Vec<PacketItemVisual>,
    registered_weapons: HashSet<i32>,
    ready_weapons: HashSet<i32>,
    registered_items: HashSet<i32>,
}

impl ClientWeaponMediaRegistry {
    /// New registry.
    pub fn new(
        product: Product,
        resources: Box<dyn PresentRendererResources>,
        audio: Box<dyn WeaponRegistrationAudio>,
    ) -> Self {
        Self {
            product,
            resources,
            audio,
            effects: RegisteredWeaponEffects {
                lightning_explosion_model: default_model(),
                ..RegisteredWeaponEffects::default()
            },
            weapon_records: std::array::from_fn(|_| empty_weapon()),
            item_records: Vec::new(),
            registered_weapons: HashSet::new(),
            ready_weapons: HashSet::new(),
            registered_items: HashSet::new(),
        }
    }

    /// Weapons.
    #[must_use]
    pub fn weapons(&self) -> &[ClientWeaponInfo; 16] {
        &self.weapon_records
    }

    /// Items.
    #[must_use]
    pub fn items(&self) -> &[PacketItemVisual] {
        &self.item_records
    }

    /// Weapon by number.
    pub fn weapon(&self, number: i32) -> PresentResult<&ClientWeaponInfo> {
        self.weapon_records
            .get(number as usize)
            .ok_or_else(|| PresentError::range(format!("Invalid weapon media index {number}")))
    }

    /// Require a registered weapon.
    pub fn require_weapon(&self, number: i32) -> PresentResult<&ClientWeaponInfo> {
        if number != 0 && !self.ready_weapons.contains(&number) {
            return Err(PresentError::state(format!(
                "Weapon {number} must finish registration before synchronous presentation"
            )));
        }
        self.weapon(number)
    }

    /// Register a weapon (`registerWeapon`).
    pub fn register_weapon(&mut self, number: i32, items: &dyn PresentItemTable) -> PresentResult<()> {
        self.register_weapon_now(number, items)
    }

    /// Register item visuals (`registerItemVisuals`).
    pub fn register_item_visuals(&mut self, number: i32, items: &dyn PresentItemTable) -> PresentResult<()> {
        self.register_item_now(number, items)
    }

    fn ensure_items(&mut self, items: &dyn PresentItemTable) {
        let count = items.item_count(self.product);
        if self.item_records.len() < count {
            self.item_records.resize_with(count, || PacketItemVisual {
                models: [default_model(), default_model()],
                has_second: false,
                icon: None,
            });
        }
    }

    fn register_item_now(&mut self, number: i32, items: &dyn PresentItemTable) -> PresentResult<()> {
        let count = items.item_count(self.product);
        if number < 0 || number as usize >= count {
            return Err(PresentError::drop(format!(
                "CG_RegisterItemVisuals: itemNum {number} out of range [0-{}]",
                count.saturating_sub(1)
            )));
        }
        let item = items.item_at(self.product, number as usize).ok_or_else(|| {
            PresentError::drop(format!(
                "CG_RegisterItemVisuals: itemNum {number} out of range [0-{}]",
                count.saturating_sub(1)
            ))
        })?;
        if self.registered_items.contains(&number) {
            return Ok(());
        }
        self.registered_items.insert(number);
        self.ensure_items(items);
        let model = match &item.world_models[0] {
            None => default_model(),
            Some(path) => self.resources.register_model(path),
        };
        let icon = match &item.icon {
            None => None,
            Some(icon) => self.resources.register_shader(icon),
        };
        self.item_records[number as usize] = PacketItemVisual {
            models: [model.clone(), default_model()],
            has_second: false,
            icon: icon.clone(),
        };
        if item.item_type == ItemType::Weapon {
            self.register_weapon_now(item.tag, items)?;
        }
        if (item.item_type == ItemType::Powerup
            || item.item_type == ItemType::Health
            || item.item_type == ItemType::Armor
            || item.item_type == ItemType::Holdable)
            && item.world_models[1].is_some()
        {
            let second = self
                .resources
                .register_model(item.world_models[1].as_ref().unwrap_or(&String::new()).as_str());
            self.item_records[number as usize] = PacketItemVisual {
                models: [model, second],
                has_second: true,
                icon,
            };
        }
        Ok(())
    }

    fn register_weapon_now(&mut self, number: i32, items: &dyn PresentItemTable) -> PresentResult<()> {
        self.weapon(number)?;
        if number == 0 || self.registered_weapons.contains(&number) {
            return Ok(());
        }
        self.registered_weapons.insert(number);
        let count = items.item_count(self.product);
        let mut found = None;
        for index in 0..count {
            if let Some(item) = items.item_at(self.product, index) {
                if item.item_type == ItemType::Weapon && item.tag == number {
                    found = Some((index, item));
                    break;
                }
            }
        }
        let (index, item) = found.ok_or_else(|| PresentError::drop(format!("Couldn't find weapon {number}")))?;
        let mut weapon = empty_weapon();
        weapon.item = Some(item.clone());
        self.weapon_records[number as usize] = weapon;
        self.register_item_now(index as i32, items)?;
        let path = item.world_models[0].clone();
        let icon = item.icon.clone();
        let (Some(path), Some(icon)) = (path, icon) else {
            return Err(PresentError::state(format!(
                "Weapon {number} has no world model or icon"
            )));
        };
        let model = self.resources.register_model(&path);
        let bounds = model_bounds(&model);
        let midpoint = vec3(
            bounds.min.x + 0.5 * (bounds.max.x - bounds.min.x),
            bounds.min.y + 0.5 * (bounds.max.y - bounds.min.y),
            bounds.min.z + 0.5 * (bounds.max.z - bounds.min.z),
        );
        {
            let weapon = &mut self.weapon_records[number as usize];
            weapon.packet.weapon_model = model;
            weapon.packet.weapon_midpoint = midpoint;
        }
        let weapon_icon = self.resources.register_shader(&icon);
        let ammo_icon = self.resources.register_shader(&icon);
        {
            let weapon = &mut self.weapon_records[number as usize];
            weapon.weapon_icon = weapon_icon;
            weapon.ammo_icon = ammo_icon;
        }
        for index in 0..count {
            if let Some(candidate) = items.item_at(self.product, index) {
                if candidate.item_type == ItemType::Ammo && candidate.tag == number {
                    if let Some(ammo_path) = candidate.world_models[0].clone() {
                        let ammo_model = self.resources.register_model(&ammo_path);
                        self.weapon_records[number as usize].ammo_model = ammo_model;
                    }
                    break;
                }
            }
        }
        let stem = path
            .find('.')
            .map(|dot| path[..dot].to_string())
            .unwrap_or(path.clone());
        let flash = self.resources.register_model(&format!("{stem}_flash.md3"));
        let barrel = self.resources.register_model(&format!("{stem}_barrel.md3"));
        let mut hands = self.resources.register_model(&format!("{stem}_hand.md3"));
        if hands.is_default() {
            hands = self
                .resources
                .register_model("models/weapons2/shotgun/shotgun_hand.md3");
        }
        {
            let weapon = &mut self.weapon_records[number as usize];
            weapon.flash_model = flash;
            weapon.packet.barrel_model = if barrel.is_default() { None } else { Some(barrel) };
            weapon.hands_model = hands;
        }
        self.register_weapon_specific(number)?;
        self.ready_weapons.insert(number);
        Ok(())
    }

    fn register_weapon_specific(&mut self, number: i32) -> PresentResult<()> {
        let weapon = Weapon::from_i32(number);
        match weapon {
            Some(Weapon::Gauntlet) => {
                self.weapon_records[number as usize].flash_dlight_color = vec3(0.6, 0.6, 1.0);
                let firing = self.audio.register_sound("sound/weapons/melee/fstrun.wav");
                let flash = self.audio.register_sound("sound/weapons/melee/fstatck.wav");
                let weapon = &mut self.weapon_records[number as usize];
                weapon.firing_sound = firing;
                weapon.flash_sounds = [flash, None, None, None];
            }
            Some(Weapon::Lightning) => {
                self.weapon_records[number as usize].flash_dlight_color = vec3(0.6, 0.6, 1.0);
                let ready = self.audio.register_sound("sound/weapons/melee/fsthum.wav");
                let firing = self.audio.register_sound("sound/weapons/lightning/lg_hum.wav");
                let flash = self.audio.register_sound("sound/weapons/lightning/lg_fire.wav");
                let shader = self.resources.register_shader("lightningBoltNew");
                let model = self.resources.register_model("models/weaphits/crackle.md3");
                let hit = [
                    self.audio.register_sound("sound/weapons/lightning/lg_hit.wav"),
                    self.audio.register_sound("sound/weapons/lightning/lg_hit2.wav"),
                    self.audio.register_sound("sound/weapons/lightning/lg_hit3.wav"),
                ];
                let weapon = &mut self.weapon_records[number as usize];
                weapon.ready_sound = ready;
                weapon.firing_sound = firing;
                weapon.flash_sounds = [flash, None, None, None];
                self.effects.lightning_shader = shader;
                self.effects.lightning_explosion_model = model;
                self.effects.lightning_hit_sounds = hit;
            }
            Some(Weapon::GrapplingHook) => {
                let shader = self.resources.register_shader("lightningBoltNew");
                let model = self.resources.register_model("models/ammo/rocket/rocket.md3");
                let ready = self.audio.register_sound("sound/weapons/melee/fsthum.wav");
                let firing = self.audio.register_sound("sound/weapons/melee/fstrun.wav");
                let weapon = &mut self.weapon_records[number as usize];
                weapon.flash_dlight_color = vec3(0.6, 0.6, 1.0);
                weapon.packet.missile_model = model;
                weapon.packet.missile_trail = Some(MissileTrail::Grapple);
                weapon.packet.missile_dlight = 200.0;
                weapon.packet.trail_time = 2000;
                weapon.packet.trail_radius = 64.0;
                weapon.packet.missile_dlight_color = vec3(1.0, 0.75, 0.0);
                weapon.ready_sound = ready;
                weapon.firing_sound = firing;
                self.effects.lightning_shader = shader;
            }
            Some(Weapon::Chaingun) => {
                let firing = self.audio.register_sound("sound/weapons/vulcan/wvulfire.wav");
                let flashes = [
                    self.audio.register_sound("sound/weapons/vulcan/vulcanf1b.wav"),
                    self.audio.register_sound("sound/weapons/vulcan/vulcanf2b.wav"),
                    self.audio.register_sound("sound/weapons/vulcan/vulcanf3b.wav"),
                    self.audio.register_sound("sound/weapons/vulcan/vulcanf4b.wav"),
                ];
                let shader = self.resources.register_shader("bulletExplosion");
                let weapon = &mut self.weapon_records[number as usize];
                weapon.firing_sound = firing;
                weapon.loop_fire_sound = true;
                weapon.flash_dlight_color = vec3(1.0, 1.0, 0.0);
                weapon.flash_sounds = flashes;
                weapon.eject_brass = Some(EjectBrass::Machinegun);
                self.effects.bullet_explosion_shader = shader;
            }
            Some(Weapon::Machinegun) => {
                let flashes = [
                    self.audio.register_sound("sound/weapons/machinegun/machgf1b.wav"),
                    self.audio.register_sound("sound/weapons/machinegun/machgf2b.wav"),
                    self.audio.register_sound("sound/weapons/machinegun/machgf3b.wav"),
                    self.audio.register_sound("sound/weapons/machinegun/machgf4b.wav"),
                ];
                let shader = self.resources.register_shader("bulletExplosion");
                let weapon = &mut self.weapon_records[number as usize];
                weapon.flash_dlight_color = vec3(1.0, 1.0, 0.0);
                weapon.flash_sounds = flashes;
                weapon.eject_brass = Some(EjectBrass::Machinegun);
                self.effects.bullet_explosion_shader = shader;
            }
            Some(Weapon::Shotgun) => {
                let flash = self.audio.register_sound("sound/weapons/shotgun/sshotf1b.wav");
                let weapon = &mut self.weapon_records[number as usize];
                weapon.flash_dlight_color = vec3(1.0, 1.0, 0.0);
                weapon.flash_sounds = [flash, None, None, None];
                weapon.eject_brass = Some(EjectBrass::Shotgun);
            }
            Some(Weapon::RocketLauncher) => {
                let model = self.resources.register_model("models/ammo/rocket/rocket.md3");
                let sound = self.audio.register_sound("sound/weapons/rocket/rockfly.wav");
                let flash = self.audio.register_sound("sound/weapons/rocket/rocklf1a.wav");
                let shader = self.resources.register_shader("rocketExplosion");
                let weapon = &mut self.weapon_records[number as usize];
                weapon.packet.missile_model = model;
                weapon.packet.missile_sound = sound;
                weapon.packet.missile_trail = Some(MissileTrail::Rocket);
                weapon.packet.missile_dlight = 200.0;
                weapon.packet.trail_time = 2000;
                weapon.packet.trail_radius = 64.0;
                weapon.packet.missile_dlight_color = vec3(1.0, 0.75, 0.0);
                weapon.flash_dlight_color = vec3(1.0, 0.75, 0.0);
                weapon.flash_sounds = [flash, None, None, None];
                self.effects.rocket_explosion_shader = shader;
            }
            Some(Weapon::ProxLauncher) | Some(Weapon::GrenadeLauncher) => {
                let prox = weapon == Some(Weapon::ProxLauncher);
                let model = self.resources.register_model(if prox {
                    "models/weaphits/proxmine.md3"
                } else {
                    "models/ammo/grenade1.md3"
                });
                let flash = self.audio.register_sound(if prox {
                    "sound/weapons/proxmine/wstbfire.wav"
                } else {
                    "sound/weapons/grenade/grenlf1a.wav"
                });
                let shader = self.resources.register_shader("grenadeExplosion");
                let weapon_record = &mut self.weapon_records[number as usize];
                weapon_record.packet.missile_model = model;
                weapon_record.packet.missile_trail = Some(MissileTrail::Grenade);
                weapon_record.packet.trail_time = 700;
                weapon_record.packet.trail_radius = 32.0;
                weapon_record.flash_dlight_color = vec3(1.0, 0.7, 0.0);
                weapon_record.flash_sounds = [flash, None, None, None];
                self.effects.grenade_explosion_shader = shader;
            }
            Some(Weapon::Nailgun) => {
                let model = self.resources.register_model("models/weaphits/nail.md3");
                let flash = self.audio.register_sound("sound/weapons/nailgun/wnalfire.wav");
                let weapon = &mut self.weapon_records[number as usize];
                weapon.eject_brass = Some(EjectBrass::Nailgun);
                weapon.packet.missile_trail = Some(MissileTrail::Nail);
                weapon.packet.trail_radius = 16.0;
                weapon.packet.trail_time = 250;
                weapon.packet.missile_model = model;
                weapon.flash_dlight_color = vec3(1.0, 0.75, 0.0);
                weapon.flash_sounds = [flash, None, None, None];
            }
            Some(Weapon::Plasmagun) => {
                let sound = self.audio.register_sound("sound/weapons/plasma/lasfly.wav");
                let flash = self.audio.register_sound("sound/weapons/plasma/hyprbf1a.wav");
                let plasma = self.resources.register_shader("plasmaExplosion");
                let rings = self.resources.register_shader("railDisc");
                let weapon = &mut self.weapon_records[number as usize];
                weapon.packet.missile_trail = Some(MissileTrail::Plasma);
                weapon.packet.missile_sound = sound;
                weapon.flash_dlight_color = vec3(0.6, 0.6, 1.0);
                weapon.flash_sounds = [flash, None, None, None];
                self.effects.plasma_explosion_shader = plasma;
                self.effects.rail_rings_shader = rings;
            }
            Some(Weapon::Railgun) => {
                let ready = self.audio.register_sound("sound/weapons/railgun/rg_hum.wav");
                let flash = self.audio.register_sound("sound/weapons/railgun/railgf1a.wav");
                let explosion = self.resources.register_shader("railExplosion");
                let rings = self.resources.register_shader("railDisc");
                let core = self.resources.register_shader("railCore");
                let weapon = &mut self.weapon_records[number as usize];
                weapon.ready_sound = ready;
                weapon.flash_dlight_color = vec3(1.0, 0.5, 0.0);
                weapon.flash_sounds = [flash, None, None, None];
                self.effects.rail_explosion_shader = explosion;
                self.effects.rail_rings_shader = rings;
                self.effects.rail_core_shader = core;
            }
            Some(Weapon::Bfg) => {
                let ready = self.audio.register_sound("sound/weapons/bfg/bfg_hum.wav");
                let flash = self.audio.register_sound("sound/weapons/bfg/bfg_fire.wav");
                let shader = self.resources.register_shader("bfgExplosion");
                let model = self.resources.register_model("models/weaphits/bfg.md3");
                let sound = self.audio.register_sound("sound/weapons/rocket/rockfly.wav");
                let weapon = &mut self.weapon_records[number as usize];
                weapon.ready_sound = ready;
                weapon.flash_dlight_color = vec3(1.0, 0.7, 1.0);
                weapon.flash_sounds = [flash, None, None, None];
                weapon.packet.missile_model = model;
                weapon.packet.missile_sound = sound;
                self.effects.bfg_explosion_shader = shader;
            }
            _ => {
                let flash = self.audio.register_sound("sound/weapons/rocket/rocklf1a.wav");
                let weapon = &mut self.weapon_records[number as usize];
                weapon.flash_dlight_color = vec3(1.0, 1.0, 1.0);
                weapon.flash_sounds = [flash, None, None, None];
            }
        }
        Ok(())
    }
}

/// Emit a weapon impact (`emitWeaponImpact`).
#[allow(clippy::too_many_arguments)]
pub fn emit_weapon_impact(
    product: Product,
    registry_effects: &RegisteredWeaponEffects,
    host: &mut dyn ClientWeaponHost,
    pool: &mut LocalEntityPool,
    frame: &EffectFrame,
    weapon: Weapon,
    client_num: i32,
    origin: Vec3,
    direction: Vec3,
    sound_type: ImpactSound,
) -> PresentResult<()> {
    let media = host.weapon_media().clone();
    let mark: Option<SceneShader>;
    let mut shader: Option<SceneShader> = None;
    let mut model = default_model();
    let mut sound: Option<PresentSound> = None;
    let radius: f32;
    let mut light = 0.0f32;
    let mut light_color = vec3(1.0, 1.0, 0.0);
    let mut sprite = false;
    let mut duration = 600;
    let impact_weapon = if product == Product::BaseQ3 && (weapon == Weapon::ProxLauncher || weapon == Weapon::Chaingun)
    {
        Weapon::None
    } else {
        weapon
    };
    match impact_weapon {
        Weapon::Nailgun | Weapon::None | Weapon::Gauntlet | Weapon::GrapplingHook => {
            if product == Product::MissionPack {
                sound = match sound_type {
                    ImpactSound::Flesh => media.sounds.nail_hit_flesh.clone(),
                    ImpactSound::Metal => media.sounds.nail_hit_metal.clone(),
                    ImpactSound::Default => media.sounds.nail_hit.clone(),
                };
                mark = media.shaders.hole_mark.clone();
                radius = 12.0;
            } else {
                let r = host.rand_i32() & 3;
                sound = registry_effects.lightning_hit_sounds[if r < 2 {
                    1
                } else if r == 2 {
                    0
                } else {
                    2
                }]
                .clone();
                mark = media.shaders.hole_mark.clone();
                radius = 12.0;
            }
        }
        Weapon::Lightning => {
            let r = host.rand_i32() & 3;
            sound = registry_effects.lightning_hit_sounds[if r < 2 {
                1
            } else if r == 2 {
                0
            } else {
                2
            }]
            .clone();
            mark = media.shaders.hole_mark.clone();
            radius = 12.0;
        }
        Weapon::ProxLauncher => {
            model = media.models.dish_flash.clone();
            shader = registry_effects.grenade_explosion_shader.clone();
            sound = media.sounds.prox_explosion.clone();
            mark = media.shaders.burn_mark.clone();
            radius = 64.0;
            light = 300.0;
            sprite = true;
        }
        Weapon::GrenadeLauncher => {
            model = media.models.dish_flash.clone();
            shader = registry_effects.grenade_explosion_shader.clone();
            sound = media.sounds.rocket_explosion.clone();
            mark = media.shaders.burn_mark.clone();
            radius = 64.0;
            light = 300.0;
            sprite = true;
        }
        Weapon::RocketLauncher => {
            model = media.models.dish_flash.clone();
            shader = registry_effects.rocket_explosion_shader.clone();
            sound = media.sounds.rocket_explosion.clone();
            mark = media.shaders.burn_mark.clone();
            radius = 64.0;
            light = 300.0;
            sprite = true;
            duration = 1000;
            light_color = vec3(1.0, 0.75, 0.0);
            if !host.weapon_settings().old_rocket {
                host.particle_explosion(&ParticleExplosion {
                    animation: "explode1".to_string(),
                    origin: weapon_ma(origin, 24.0, direction),
                    velocity: scale3(direction, 64.0),
                    duration: 1400,
                    size_start: 20.0,
                    size_end: 30.0,
                });
            }
        }
        Weapon::Railgun => {
            model = media.models.ring_flash.clone();
            shader = registry_effects.rail_explosion_shader.clone();
            sound = media.sounds.plasma_explosion.clone();
            mark = media.shaders.energy_mark.clone();
            radius = 24.0;
        }
        Weapon::Plasmagun => {
            model = media.models.ring_flash.clone();
            shader = registry_effects.plasma_explosion_shader.clone();
            sound = media.sounds.plasma_explosion.clone();
            mark = media.shaders.energy_mark.clone();
            radius = 16.0;
        }
        Weapon::Bfg => {
            model = media.models.dish_flash.clone();
            shader = registry_effects.bfg_explosion_shader.clone();
            sound = media.sounds.rocket_explosion.clone();
            mark = media.shaders.burn_mark.clone();
            radius = 32.0;
            sprite = true;
        }
        Weapon::Shotgun => {
            model = media.models.bullet_flash.clone();
            shader = registry_effects.bullet_explosion_shader.clone();
            mark = media.shaders.bullet_mark.clone();
            radius = 4.0;
        }
        Weapon::Chaingun => {
            model = media.models.bullet_flash.clone();
            mark = media.shaders.bullet_mark.clone();
            // Donor selects flesh/metal first, then overwrites with ricochet; keep the final value.
            let r = host.rand_i32() & 3;
            sound = if r < 2 {
                media.sounds.ricochet1.clone()
            } else if r == 2 {
                media.sounds.ricochet2.clone()
            } else {
                media.sounds.ricochet3.clone()
            };
            radius = 8.0;
        }
        Weapon::Machinegun => {
            model = media.models.bullet_flash.clone();
            shader = registry_effects.bullet_explosion_shader.clone();
            mark = media.shaders.bullet_mark.clone();
            let r = host.rand_i32() & 3;
            sound = if r == 0 {
                media.sounds.ricochet1.clone()
            } else if r == 1 {
                media.sounds.ricochet2.clone()
            } else {
                media.sounds.ricochet3.clone()
            };
            radius = 8.0;
        }
    }
    if sound.is_some() {
        host.start_sound(Some(origin), 1022, 0, sound);
    }
    if !model.is_default() {
        let handle = host.weapon_effects().make_explosion(
            pool,
            frame,
            &ExplosionOptions {
                origin,
                direction: Some(direction),
                model,
                shader,
                duration,
                sprite,
            },
        )?;
        if let Some(le) = pool.get_mut(handle) {
            le.light = light;
            le.light_color = light_color;
            if weapon == Weapon::Railgun {
                let color = host.client_info_view(client_num).color1;
                le.color = vec4(color.x, color.y, color.z, le.color.w);
            }
        }
    }
    let color = if weapon == Weapon::Railgun {
        host.client_info_view(client_num).color2
    } else {
        vec3(1.0, 1.0, 1.0)
    };
    let orientation = host.random_f32() * 360.0;
    let alpha_fade = mark == media.shaders.energy_mark;
    host.impact_mark(&ImpactMarkRequest {
        shader: mark,
        origin,
        direction,
        orientation,
        color: vec4(color.x, color.y, color.z, 1.0),
        alpha_fade,
        radius,
        temporary: false,
    });
    Ok(())
}

/// Emit a rail trail (`emitRailTrail`).
#[allow(clippy::too_many_arguments)]
pub fn emit_rail_trail(
    time: i32,
    registry_effects: &RegisteredWeaponEffects,
    host: &mut dyn ClientWeaponHost,
    pool: &mut LocalEntityPool,
    _frame: &EffectFrame,
    client_num: i32,
    start: &mut Vec3,
    end: Vec3,
) -> PresentResult<()> {
    let ci = host.client_info_view(client_num);
    let settings = host.weapon_settings();
    start.z -= 4.0;
    let mut position = *start;
    let delta = sub3(end, *start);
    let length = length3(delta);
    let direction = normalize3(delta);
    let temp = perpendicular_vector(direction);
    let axis: Vec<Vec3> = (0..36)
        .map(|i| rotate_point_around_vector(direction, temp, f64::from(i * 10)))
        .collect();
    let mut re = create_rail_core_entity();
    let handle = pool.allocate(LocalEntityType::FadeRgb, RefEntity::RailCore(re.clone()))?;
    re.shading.shader_time = time as f32 / 1000.0;
    re.shading.custom_shader = registry_effects.rail_core_shader.clone();
    re.origin = *start;
    re.old_origin = end;
    re.shading.shader_rgba = weapon_bytes(ci.color1, 255.0, 255.0);
    if let Some(le) = pool.get_mut(handle) {
        le.start_time = time;
        le.end_time = qvm_float_to_int(time as f32 + settings.rail_trail_time as f32);
        le.life_rate = 1.0 / (le.end_time.wrapping_sub(time) as f32);
        le.color = vec4(ci.color1.x * 0.75, ci.color1.y * 0.75, ci.color1.z * 0.75, 1.0);
        le.ref_entity = RefEntity::RailCore(re.clone());
    }
    position = weapon_ma(position, 20.0, direction);
    let step = scale3(direction, 5.0);
    if settings.old_rail {
        if let Some(le) = pool.get_mut(handle) {
            if let RefEntity::RailCore(re) = &mut le.ref_entity {
                re.origin = add3(re.origin, vec3(0.0, 0.0, -8.0));
                re.old_origin = add3(re.old_origin, vec3(0.0, 0.0, -8.0));
            }
        }
        return Ok(());
    }
    let mut skip = -1;
    let mut j = 18usize;
    let mut i = 0i32;
    while (i as f32) < length {
        if i != skip {
            skip = i + 5;
            let mut re = create_sprite_entity();
            let handle = pool.allocate(LocalEntityType::MoveScaleFade, RefEntity::Sprite(re.clone()))?;
            let side = axis[j];
            re.shading.shader_time = time as f32 / 1000.0;
            re.radius = 1.1;
            re.shading.custom_shader = registry_effects.rail_rings_shader.clone();
            re.shading.shader_rgba = weapon_bytes(ci.color2, 255.0, 255.0);
            if let Some(le) = pool.get_mut(handle) {
                le.le_flags = LE_PUFF_DONT_SCALE;
                le.start_time = time;
                le.end_time = time.wrapping_add(i >> 1).wrapping_add(600);
                le.life_rate = 1.0 / (le.end_time.wrapping_sub(time) as f32);
                le.color = vec4(ci.color2.x * 0.75, ci.color2.y * 0.75, ci.color2.z * 0.75, 1.0);
                le.pos = Trajectory {
                    type_: TrajectoryType::Linear,
                    time,
                    duration: 0,
                    base: weapon_ma(position, 4.0, side),
                    delta: scale3(side, 6.0),
                };
                le.ref_entity = RefEntity::Sprite(re);
            }
        }
        position = add3(position, step);
        j = (j + 1) % 36;
        i += 5;
    }
    Ok(())
}

/// Emit a plasma trail (`emitPlasmaTrail`).
#[allow(clippy::too_many_arguments)]
pub fn emit_plasma_trail(
    time: i32,
    origin: Vec3,
    angles: Vec3,
    flash_color: Vec3,
    rail_rings_shader: Option<SceneShader>,
    host: &mut dyn ClientWeaponHost,
    pool: &mut LocalEntityPool,
) -> PresentResult<()> {
    let settings = host.weapon_settings();
    if settings.no_projectile_trail || settings.old_plasma {
        return Ok(());
    }
    let mut re = create_sprite_entity();
    let handle = pool.allocate(LocalEntityType::MoveScaleFade, RefEntity::Sprite(re.clone()))?;
    let velocity = vec3(
        60.0 - 120.0 * host.crandom_f32(),
        40.0 - 80.0 * host.crandom_f32(),
        100.0 - 200.0 * host.crandom_f32(),
    );
    let axis = angles_to_axis(angles);
    re.origin = add3(origin, weapon_transform(vec3(2.0, 2.0, 2.0), &axis));
    let water = if host.point_contents_pred(re.origin, -1) & CONTENTS_WATER != 0 {
        0.1
    } else {
        1.0
    };
    re.shading.shader_time = time as f32 / 1000.0;
    re.radius = 0.25;
    re.shading.custom_shader = rail_rings_shader;
    re.shading.shader_rgba = weapon_bytes(flash_color, 63.0, 63.0);
    let rand_bits = [host.rand_i32() & 31, host.rand_i32() & 31, host.rand_i32() & 31];
    if let Some(le) = pool.get_mut(handle) {
        le.le_flags = LE_TUMBLE;
        le.start_time = time;
        le.end_time = time.wrapping_add(600);
        le.pos = Trajectory {
            type_: TrajectoryType::Gravity,
            time,
            duration: 0,
            base: re.origin,
            delta: scale3(weapon_transform(velocity, &axis), water),
        };
        le.bounce_factor = 0.3;
        le.color = vec4(flash_color.x * 0.2, flash_color.y * 0.2, flash_color.z * 0.2, 0.25);
        le.angles = Trajectory {
            type_: TrajectoryType::Linear,
            time,
            duration: 0,
            base: vec3(rand_bits[0] as f32, rand_bits[1] as f32, rand_bits[2] as f32),
            delta: vec3(1.0, 0.5, 0.0),
        };
        le.ref_entity = RefEntity::Sprite(re);
    }
    Ok(())
}
