//! Quake III base/game: missile.
//!
//! Donor provenance: `src/content/q3/base/game/missile.ts`.

use qa_core::math::{add3, length3, normalize3, scale3, sub3, vec3, vector_to_angles, Bounds, Vec3};
use qa_core::numeric::qvm_float_to_int;
use std::collections::HashMap;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::item_motion::*;
use crate::q3::base::game::mirrors_game_items::*;

// ---------------------------------------------------------------------------
// missile.ts: g_missile.c and g_weapon.c grapple helpers
// ---------------------------------------------------------------------------

pub(crate) const MASK_SHOT: i32 = 1 | 0x2000_0000 | 0x4000_0000;

pub(crate) const EF_NODRAW: i32 = 0x80;

pub(crate) const EF_TICKING: i32 = 2;

pub(crate) const SURF_METALSTEPS: i32 = 0x1000;

/// Grapple speed (`Q3_GRAPPLE_SPEED`).
pub const Q3_GRAPPLE_SPEED: f32 = 800.0;

/// Grapple lifetime (`Q3_GRAPPLE_LIFETIME`).
pub const Q3_GRAPPLE_LIFETIME: i32 = 10000;

/// Grapple think interval (`Q3_GRAPPLE_THINK_INTERVAL`).
pub const Q3_GRAPPLE_THINK_INTERVAL: i32 = 100;

/// Proximity-stick think name.
pub const MISSILE_SPECIAL_THINK: &str = "q3.base.game.missile.specialImpact.think";

/// Proximity die name.
pub const MISSILE_SPECIAL_DIE: &str = "q3.base.game.missile.specialImpact.die";

/// Hook think name.
pub const MISSILE_HOOK_THINK: &str = "q3.base.game.missile.hookThink";

/// Proximity explode think name.
pub const MISSILE_PROXIMITY_DIE_THINK: &str = "q3.base.game.missile.proximityDie.think";

/// Proximity trigger touch name.
pub const MISSILE_PROXIMITY_TOUCH: &str = "q3.base.game.missile.proximityActivate.touch";

/// Merged-mine free think name.
pub const MISSILE_PROXIMITY_PLAYER_THINK: &str = "q3.base.game.missile.proximityPlayer.think";

/// Attached-mine explode think name.
pub const MISSILE_PROXIMITY_ON_PLAYER: &str = "q3.base.game.missile.proximityExplodeOnPlayer";

/// Missile expiry think name.
pub const MISSILE_LAUNCH_THINK: &str = "q3.base.game.missile.launch.think";

/// Grapple expiry think name.
pub const MISSILE_GRAPPLE_THINK: &str = "q3.base.game.missile.fireGrapple.think";

/// Missile fire direction; source `fire_*` normalizes it in place.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MissileDirection {
    /// X component.
    pub x: f32,
    /// Y component.
    pub y: f32,
    /// Z component.
    pub z: f32,
}

impl From<Vec3> for MissileDirection {
    fn from(value: Vec3) -> MissileDirection {
        MissileDirection {
            x: value.x,
            y: value.y,
            z: value.z,
        }
    }
}

/// Missile parameters (`q3MissileParameters`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MissileParameters {
    /// Speed.
    pub speed: f32,
    /// Duration in ms.
    pub duration: i32,
    /// Gravity flight.
    pub gravity: bool,
    /// Direct damage.
    pub direct: i32,
    /// Splash damage.
    pub splash: i32,
    /// Splash radius.
    pub radius: f32,
    /// Means of death.
    pub method: i32,
    /// Splash means of death.
    pub splash_method: i32,
}

/// Missile parameters per weapon (`q3MissileParameters`).
pub fn q3_missile_parameters(weapon: Weapon) -> Q3GameItemsResult<MissileParameters> {
    match weapon {
        Weapon::GrenadeLauncher => Ok(MissileParameters {
            speed: 700.0,
            duration: 2500,
            gravity: true,
            direct: 100,
            splash: 100,
            radius: 150.0,
            method: 4,
            splash_method: 5,
        }),
        Weapon::RocketLauncher => Ok(MissileParameters {
            speed: 900.0,
            duration: 15000,
            gravity: false,
            direct: 100,
            splash: 100,
            radius: 120.0,
            method: 6,
            splash_method: 7,
        }),
        Weapon::Plasmagun => Ok(MissileParameters {
            speed: 2000.0,
            duration: 10000,
            gravity: false,
            direct: 20,
            splash: 15,
            radius: 20.0,
            method: 8,
            splash_method: 9,
        }),
        Weapon::Bfg => Ok(MissileParameters {
            speed: 2000.0,
            duration: 10000,
            gravity: false,
            direct: 100,
            splash: 100,
            radius: 120.0,
            method: 12,
            splash_method: 13,
        }),
        _ => Err(invalid(format!(
            "no Q3 missile parameters for weapon {}",
            weapon as i32
        ))),
    }
}

/// Nail spread velocity (`q3NailVelocity`).
pub fn q3_nail_velocity(start: Vec3, forward: Vec3, right: Vec3, up: Vec3, random: &mut dyn GameRandom) -> Vec3 {
    let angle = random.random() * std::f32::consts::PI * 2.0;
    let vertical = (angle as f64).sin() as f32 * random.crandom() * 500.0 * 16.0;
    let horizontal = (angle as f64).cos() as f32 * random.crandom() * 500.0 * 16.0;
    let end = add3(
        add3(add3(start, scale3(forward, 8192.0 * 16.0)), scale3(right, horizontal)),
        scale3(up, vertical),
    );
    let direction = normalize3(sub3(end, start));
    scale3(direction, 555.0 + random.random() * 1800.0)
}

/// Grapple target point (`q3GrappleTarget`).
#[must_use]
pub fn q3_grapple_target(origin: Vec3, bounds: Bounds) -> Vec3 {
    add3(origin, scale3(add3(bounds.min, bounds.max), 0.5))
}

/// Snap a vector to integers (`snapVector`).
#[must_use]
pub fn snap_vector(value: Vec3) -> Vec3 {
    vec3(
        qvm_float_to_int(value.x) as f32,
        qvm_float_to_int(value.y) as f32,
        qvm_float_to_int(value.z) as f32,
    )
}

/// Snap toward a point with truncation-plus-one (`snapVectorTowards`).
#[must_use]
pub fn snap_vector_towards(value: Vec3, toward: Vec3) -> Vec3 {
    let axis = |v: f32, to: f32| -> f32 { qvm_float_to_int(v).wrapping_add(if to <= v { 0 } else { 1 }) as f32 };
    vec3(
        axis(value.x, toward.x),
        axis(value.y, toward.y),
        axis(value.z, toward.z),
    )
}

/// Rigid-body state mirror.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BodyState {
    /// Origin.
    pub origin: Vec3,
    /// Velocity.
    pub velocity: Vec3,
}

/// Shared body table (`SharedBodyTable`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BodyTable {
    bodies: HashMap<ActorId, BodyState>,
}

impl BodyTable {
    /// Read a body.
    #[must_use]
    pub fn read(&self, actor: ActorId) -> Option<BodyState> {
        self.bodies.get(&actor).copied()
    }

    /// Write a body.
    pub fn write(&mut self, actor: ActorId, body: BodyState) {
        self.bodies.insert(actor, body);
    }
}

/// Invulnerability impact outcome.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InvulnerabilityOutcome {
    /// No effect.
    Miss,
    /// Bounce with direction.
    Hit {
        /// Bounce direction.
        bounce_direction: Vec3,
    },
}

/// Missile host services (combat, world, bodies, behavior, random, missionpack).
pub trait MissileHost {
    /// Combat host.
    fn combat(&mut self) -> &mut dyn CombatOps;
    /// World host.
    fn world(&mut self) -> &mut dyn WorldOps;
    /// Combat and world pair for calls needing both.
    fn combat_and_world(&mut self) -> (&mut dyn CombatOps, &mut dyn WorldOps);
    /// Body table.
    fn bodies(&mut self) -> &mut BodyTable;
    /// Game random.
    fn random(&mut self) -> &mut dyn GameRandom;
    /// Whether a weapon-behavior port is installed.
    fn has_weapon_behavior(&self) -> bool;
    /// Weapon-behavior launch override.
    fn weapon_behavior_launch(
        &mut self,
        projectile: ActorId,
        shooter: ActorId,
        weapon: Weapon,
        time_seconds: f64,
        origin: Vec3,
        velocity: Vec3,
    ) -> Option<BodyState>;
    /// Weapon-behavior step override.
    fn weapon_behavior_step(
        &mut self,
        projectile: ActorId,
        origin: Vec3,
        velocity: Vec3,
        time_seconds: f64,
    ) -> Option<BodyState>;
    /// Whether missionpack services exist.
    fn is_missionpack(&self) -> bool;
    /// Proximity mine timeout.
    fn prox_mine_timeout(&self) -> i32;
    /// Missionpack sound index.
    fn missionpack_sound_index(&mut self, path: &str) -> i32;
    /// Invulnerability impact effect.
    fn invulnerability_impact(
        &mut self,
        pool: &mut EntityPool,
        target: Slot,
        direction: Vec3,
        point: Vec3,
    ) -> InvulnerabilityOutcome;
}

/// Projectile phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProjectilePhase {
    /// Retained event.
    Event,
    /// In flight.
    Flight,
    /// Attached.
    Attached,
}

/// Projectile impact emission.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ImpactEmit {
    /// Grenade bounce.
    Bounce {
        /// Surface normal.
        normal: Vec3,
    },
    /// Missile impact.
    Impact {
        /// Surface normal.
        normal: Vec3,
        /// Hit target.
        target: Option<ActorId>,
        /// Flesh hit.
        flesh: bool,
        /// Surface flags.
        surface_flags: i32,
    },
}

/// Projectile target record for the driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectileTarget {
    /// Damageable.
    pub damageable: bool,
    /// Player.
    pub player: bool,
    /// Invulnerable.
    pub invulnerable: bool,
    /// Accuracy eligible.
    pub accuracy_eligible: bool,
}

/// Mutable projectile state shared with the driver (trajectory + flags).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProjectileState {
    /// Trajectory.
    pub trajectory: Trajectory,
    /// Entity flags.
    pub flags: i32,
}

/// Read-only projectile spec for the driver.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProjectileSpec {
    /// Weapon.
    pub weapon: Weapon,
    /// Direct damage.
    pub direct: i32,
    /// Splash damage.
    pub splash: i32,
    /// Splash radius.
    pub radius: f32,
    /// Means of death.
    pub method: i32,
    /// Splash means of death.
    pub splash_method: i32,
    /// Damage point.
    pub damage_point: Vec3,
}

/// Projectile launch result (`q3LaunchProjectile`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProjectileLaunch {
    /// Expiry time.
    pub expires: i32,
    /// Trajectory.
    pub trajectory: Trajectory,
}

/// Projectile driver host (`q3*Projectile` sibling surface).
pub trait ProjectileDriver {
    /// Launch a projectile.
    fn launch(
        &mut self,
        start: Vec3,
        direction: Vec3,
        speed: f32,
        gravity: bool,
        duration: i32,
        time: i32,
    ) -> ProjectileLaunch;
    /// Bounce a projectile.
    fn bounce(&mut self, context: &mut ProjectileContext<'_>, trace: &ActorTraceResult) -> Q3GameItemsResult<()>;
    /// Explode a projectile.
    fn explode(&mut self, context: &mut ProjectileContext<'_>) -> Q3GameItemsResult<()>;
    /// Impact a projectile.
    fn impact(&mut self, context: &mut ProjectileContext<'_>, trace: &ActorTraceResult) -> Q3GameItemsResult<()>;
    /// Step a projectile.
    fn step(&mut self, context: &mut ProjectileContext<'_>) -> Q3GameItemsResult<()>;
}

/// Projectile callback context (the donor's `projectileHost` closures).
pub struct ProjectileContext<'a> {
    /// Missile runtime.
    pub runtime: &'a mut MissileRuntime,
    /// Entity pool.
    pub pool: &'a mut EntityPool,
    /// Missile host.
    pub host: &'a mut dyn MissileHost,
    /// Projectile slot.
    pub slot: Slot,
    /// Mutable shared state.
    pub state: ProjectileState,
    /// Read-only spec.
    pub spec: ProjectileSpec,
}

impl ProjectileContext<'_> {
    /// Write shared state back to the entity (skipped once released).
    pub fn write_back(&mut self) -> Q3GameItemsResult<()> {
        if self.pool.get(self.slot).is_none() {
            return Ok(());
        }
        let entity = &mut self.pool.entities[self.slot];
        entity.s.pos = self.state.trajectory;
        entity.s.e_flags = self.state.flags;
        Ok(())
    }

    fn projectile(&self) -> Q3GameItemsResult<NativeProjectile> {
        self.runtime.projectile(self.pool, self.slot)
    }

    /// Current time.
    pub fn time(&mut self) -> i32 {
        self.host.combat().time()
    }

    /// Previous frame time.
    pub fn previous_time(&mut self) -> i32 {
        self.host.combat().previous_time()
    }

    /// Whether the projectile record is live.
    pub fn live(&mut self) -> Q3GameItemsResult<bool> {
        let projectile = self.projectile()?;
        Ok(self.pool.native_by_actor(projectile.actor) == Some(self.slot))
    }

    /// Projectile phase.
    pub fn phase(&mut self) -> Q3GameItemsResult<ProjectilePhase> {
        self.pool.require_owned(self.slot)?;
        let entity = &self.pool.entities[self.slot];
        Ok(if entity.free_after_event {
            ProjectilePhase::Event
        } else if entity.s.e_type == EntityType::Missile as i32 {
            ProjectilePhase::Flight
        } else {
            ProjectilePhase::Attached
        })
    }

    /// Event time.
    pub fn event_time(&mut self) -> Q3GameItemsResult<i32> {
        Ok(self.pool.at(self.slot)?.event_time)
    }

    /// Clear the event.
    pub fn clear_event(&mut self) -> Q3GameItemsResult<()> {
        self.pool.require_owned(self.slot)?;
        self.pool.entities[self.slot].s.event = 0;
        Ok(())
    }

    /// Current origin.
    pub fn origin(&mut self) -> Q3GameItemsResult<Vec3> {
        Ok(self.pool.at(self.slot)?.r.current_origin)
    }

    /// Move the shared body.
    pub fn move_body(&mut self, origin: Vec3, velocity: Vec3) -> Q3GameItemsResult<()> {
        let projectile = self.projectile()?;
        if self.host.bodies().read(projectile.actor).is_some() {
            self.host
                .bodies()
                .write(projectile.actor, BodyState { origin, velocity });
        }
        Ok(())
    }

    /// Set the origin and stop the shared body.
    pub fn set_origin_stop(&mut self, origin: Vec3) -> Q3GameItemsResult<()> {
        let projectile = self.projectile()?;
        set_origin(self.pool, self.slot, origin)?;
        if self.host.bodies().read(projectile.actor).is_some() {
            self.host.bodies().write(
                projectile.actor,
                BodyState {
                    origin,
                    velocity: vec3(0.0, 0.0, 0.0),
                },
            );
        }
        Ok(())
    }

    /// Link the projectile.
    pub fn link(&mut self) -> Q3GameItemsResult<()> {
        let slot = self.slot;
        self.host.world().link(self.pool, slot)
    }

    /// Release the projectile.
    pub fn release(&mut self) -> Q3GameItemsResult<()> {
        let slot = self.slot;
        self.pool.free(slot)
    }

    /// Trace the projectile sweep.
    pub fn trace(
        &mut self,
        start: Vec3,
        end: Vec3,
        pass_actor: Option<ActorId>,
    ) -> Q3GameItemsResult<ActorTraceResult> {
        self.pool.require_owned(self.slot)?;
        let entity = &self.pool.entities[self.slot];
        let query = ActorTraceQuery {
            start,
            end,
            shape: TraceShape::Box {
                mins: entity.r.mins,
                maxs: entity.r.maxs,
            },
            pass_actor,
            mask: entity.clipmask,
        };
        Ok(self.host.world().trace_actor(self.pool, &query))
    }

    /// World actor.
    #[must_use]
    pub fn world_actor(&self) -> ActorId {
        ActorId::from_slot(ENTITYNUM_WORLD)
    }

    /// Target record for an actor.
    pub fn target(&mut self, actor: ActorId) -> Q3GameItemsResult<Option<ProjectileTarget>> {
        let state = self.host.combat().authority(self.pool, actor);
        let Some(state) = state else {
            return Ok(None);
        };
        let projectile = self.projectile()?;
        let native = self.pool.native_by_actor(actor);
        let attacker = self.host.combat().authority(self.pool, projectile.owner);
        let owner_slot = self.pool.native_by_actor(projectile.owner);
        let game_type = self.host.combat().game_type();
        let is_player = self.host.combat().is_player(self.pool, actor);
        let target = AccuracyTarget {
            actor,
            damageable: state.can_take_damage,
            player: is_player,
            health: state.health,
            team: state.team,
        };
        let attacker_target = AccuracyTarget {
            actor: projectile.owner,
            damageable: attacker.is_some_and(|state| state.can_take_damage),
            player: owner_slot.is_some_and(|slot| self.pool.get(slot).is_some_and(|entity| entity.client.is_some())),
            health: attacker.map_or(0, |state| state.health),
            team: attacker.and_then(|state| state.team),
        };
        let accuracy_eligible =
            self.host
                .combat()
                .accuracy_hit(game_type >= GameType::Team as i32, &target, &attacker_target);
        let invulnerable = native.is_some_and(|slot| {
            self.pool.get(slot).is_some_and(|entity| {
                entity
                    .client
                    .as_ref()
                    .is_some_and(|client| client.invulnerability_time > self.host.combat().time())
            })
        });
        Ok(Some(ProjectileTarget {
            damageable: state.can_take_damage,
            player: is_player,
            invulnerable,
            accuracy_eligible,
        }))
    }

    /// Emit an impact event.
    pub fn emit(&mut self, event: &ImpactEmit) -> Q3GameItemsResult<()> {
        let slot = self.slot;
        match *event {
            ImpactEmit::Bounce { .. } => self
                .host
                .combat()
                .add_event(self.pool, slot, EntityEvent::GrenadeBounce, 0),
            ImpactEmit::Impact {
                normal,
                target,
                flesh,
                surface_flags,
            } => {
                if let (true, Some(actor)) = (flesh, target) {
                    let target_slot = self
                        .pool
                        .native_by_actor(actor)
                        .ok_or_else(|| invalid("Q3 admitted player has no native hit-event client"))?;
                    if self.pool.at(target_slot)?.client.is_none() {
                        return Err(invalid("Q3 admitted player has no native hit-event client"));
                    }
                    self.host.combat().add_event(
                        self.pool,
                        slot,
                        EntityEvent::MissileHit,
                        direction_to_byte(Some(normal)),
                    )?;
                    let number = self.pool.at(target_slot)?.s.number;
                    self.pool.at_mut(slot)?.s.other_entity_num = number;
                    Ok(())
                } else {
                    let event = if surface_flags & SURF_METALSTEPS != 0 {
                        EntityEvent::MissileMissMetal
                    } else {
                        EntityEvent::MissileMiss
                    };
                    self.host
                        .combat()
                        .add_event(self.pool, slot, event, direction_to_byte(Some(normal)))
                }
            }
        }
    }

    /// Retain as an event entity.
    pub fn retain(&mut self) -> Q3GameItemsResult<()> {
        self.pool.require_owned(self.slot)?;
        let entity = &mut self.pool.entities[self.slot];
        entity.free_after_event = true;
        entity.s.e_type = EntityType::General as i32;
        Ok(())
    }

    /// Apply direct damage.
    pub fn damage(&mut self, target: ActorId, direction: Option<Vec3>, point: Vec3) -> Q3GameItemsResult<()> {
        let projectile = self.projectile()?;
        let target_slot = self
            .pool
            .native_by_actor(target)
            .ok_or_else(|| invalid("missile target has no pooled participant"))?;
        let slot = self.slot;
        self.pool.require_owned(slot)?;
        let amount = self.pool.entities[slot].damage;
        let method = self.pool.entities[slot].method_of_death;
        let attacker = self.runtime.owner_participant(self.pool, &projectile);
        self.host.combat().damage(
            self.pool,
            target_slot,
            DamageParticipant::Entity(slot),
            attacker,
            direction,
            Some(point),
            amount,
            0,
            method,
            Some(projectile.actor),
        )
    }

    /// Apply splash damage.
    pub fn radius(&mut self, origin: Vec3, ignore: Option<ActorId>) -> Q3GameItemsResult<bool> {
        let projectile = self.projectile()?;
        let slot = self.slot;
        self.pool.require_owned(slot)?;
        let splash = self.pool.entities[slot].splash_damage;
        let radius = self.pool.entities[slot].splash_radius;
        let method = self.pool.entities[slot].splash_method_of_death;
        let attacker = self
            .runtime
            .owner_slot(self.pool, &projectile)
            .ok_or_else(|| invalid("missile owner has no pooled participant"))?;
        let ignore_slot = ignore.and_then(|actor| self.pool.native_by_actor(actor));
        Ok(self.host.combat().radius_damage(
            self.pool,
            origin,
            attacker,
            splash,
            radius,
            ignore_slot,
            method,
            Some(projectile.actor),
        ))
    }

    /// Record owner accuracy.
    pub fn accuracy(&mut self) -> Q3GameItemsResult<()> {
        let projectile = self.projectile()?;
        if let Some(owner) = self.pool.native_by_actor(projectile.owner) {
            if self.pool.at(owner)?.client.is_some() {
                let hits = self
                    .pool
                    .at(owner)?
                    .client
                    .as_ref()
                    .expect("client checked")
                    .accuracy_hits;
                self.pool.at_mut(owner)?.client_mut()?.accuracy_hits = hits.wrapping_add(1);
            }
        }
        Ok(())
    }

    /// Run the projectile think callback.
    pub fn think(&mut self, driver: &mut dyn ProjectileDriver) -> Q3GameItemsResult<()> {
        let time = self.host.combat().time();
        run_think(self.pool, self.slot, time, &mut |pool, slot, name| {
            self.runtime
                .dispatch_think(pool, self.host, driver, slot, name)
                .map(|_| ())
        })
    }

    /// Whether special hook/prox handling applies.
    pub fn has_special(&mut self) -> Q3GameItemsResult<bool> {
        self.pool.require_owned(self.slot)?;
        let entity = &self.pool.entities[self.slot];
        Ok(entity.classname.as_deref() == Some("hook") || entity.s.weapon == Weapon::ProxLauncher)
    }

    /// Special hook/prox impact; returns true when handled.
    pub fn special_impact(&mut self, trace: &ActorTraceResult, target: ActorId) -> Q3GameItemsResult<bool> {
        let slot = self.slot;
        self.runtime.special_impact(self.pool, self.host, slot, trace, target)
    }

    /// Post-move prox arming check.
    pub fn after_move(&mut self) -> Q3GameItemsResult<()> {
        let slot = self.slot;
        self.runtime.after_move(self.pool, self.host, slot)
    }

    /// Clear a dangling hook reference on no impact.
    pub fn no_impact(&mut self) -> Q3GameItemsResult<()> {
        let slot = self.slot;
        self.runtime.no_impact(self.pool, slot)
    }

    /// Whether invulnerability reflection applies.
    pub fn has_reflection(&self) -> bool {
        self.host.is_missionpack()
    }

    /// Invulnerability reflection impact.
    pub fn reflection_impact(
        &mut self,
        target: ActorId,
        direction: Vec3,
        point: Vec3,
    ) -> Q3GameItemsResult<InvulnerabilityOutcome> {
        let current = self
            .pool
            .native_by_actor(target)
            .ok_or_else(|| invalid("invulnerable Q3 player lost its native client"))?;
        if self.pool.at(current)?.client.is_none() {
            return Err(invalid("invulnerable Q3 player lost its native client"));
        }
        Ok(self.host.invulnerability_impact(self.pool, current, direction, point))
    }
}

/// Projectile attachment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProjectileAttachment {
    /// Unattached.
    None,
    /// Attached to a player actor.
    Player(ActorId),
}

/// Native projectile record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NativeProjectile {
    /// Projectile actor.
    pub actor: ActorId,
    /// Owner actor.
    pub owner: ActorId,
    /// Actor to pass through.
    pub pass: Option<ActorId>,
    /// Attachment.
    pub attachment: ProjectileAttachment,
    /// Proximity trigger actor.
    pub trigger: Option<ActorId>,
}

/// Missile save entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MissileSaveEntry {
    /// Entity slot.
    pub entity: Slot,
    /// Actor.
    pub actor: u32,
    /// Owner actor.
    pub owner: u32,
    /// Pass actor.
    pub pass: Option<u32>,
    /// Trigger actor.
    pub trigger: Option<u32>,
    /// Attached player actor.
    pub attachment: Option<u32>,
}

/// Missile fire surface for shooter entities.
pub trait MissileFire {
    /// Product.
    fn product(&self) -> Product;
    /// Fire a grenade.
    fn fire_grenade(
        &mut self,
        pool: &mut EntityPool,
        host: &mut dyn MissileHost,
        driver: &mut dyn ProjectileDriver,
        entity: Slot,
        start: Vec3,
        direction: &mut MissileDirection,
    ) -> Q3GameItemsResult<Slot>;
    /// Fire a rocket.
    fn fire_rocket(
        &mut self,
        pool: &mut EntityPool,
        host: &mut dyn MissileHost,
        driver: &mut dyn ProjectileDriver,
        entity: Slot,
        start: Vec3,
        direction: &mut MissileDirection,
    ) -> Q3GameItemsResult<Slot>;
    /// Fire plasma.
    fn fire_plasma(
        &mut self,
        pool: &mut EntityPool,
        host: &mut dyn MissileHost,
        driver: &mut dyn ProjectileDriver,
        entity: Slot,
        start: Vec3,
        direction: &mut MissileDirection,
    ) -> Q3GameItemsResult<Slot>;
}

pub(crate) fn normalize_direction(direction: &mut MissileDirection) -> Vec3 {
    let normalized = normalize3(vec3(direction.x, direction.y, direction.z));
    direction.x = normalized.x;
    direction.y = normalized.y;
    direction.z = normalized.z;
    normalized
}

pub(crate) fn trace_normal(trace: &ActorTraceResult) -> Vec3 {
    match trace.contact {
        TraceContact::Plane { normal } => normal,
        TraceContact::None => vec3(0.0, 0.0, 0.0),
    }
}

pub(crate) fn missile_center(pool: &EntityPool, slot: Slot) -> Q3GameItemsResult<Vec3> {
    let entity = pool.at(slot)?;
    Ok(q3_grapple_target(
        entity.r.current_origin,
        Bounds {
            min: entity.r.mins,
            max: entity.r.maxs,
        },
    ))
}

/// Missile runtime (`MissileRuntime`).
#[derive(Debug, Default)]
pub struct MissileRuntime {
    projectiles: HashMap<Slot, NativeProjectile>,
    product: Option<Product>,
}

impl MissileRuntime {
    /// Empty runtime.
    #[must_use]
    pub fn new() -> MissileRuntime {
        MissileRuntime {
            projectiles: HashMap::new(),
            product: None,
        }
    }

    /// Bound product.
    #[must_use]
    pub fn product(&self) -> Option<Product> {
        self.product
    }

    /// Close the runtime.
    pub fn close(&mut self) {
        self.projectiles.clear();
    }

    fn check_host(&mut self, pool: &EntityPool, host: &mut dyn MissileHost) -> Q3GameItemsResult<()> {
        if pool.product() != host.combat().product() {
            return Err(invalid("missile product does not match its entity pool"));
        }
        if let Some(product) = self.product {
            if product != pool.product() {
                return Err(invalid("missile product does not match its entity pool"));
            }
        } else {
            self.product = Some(pool.product());
        }
        Ok(())
    }

    /// Capture the save image.
    #[must_use]
    pub fn capture_save_state(&self) -> Vec<MissileSaveEntry> {
        let mut entries: Vec<MissileSaveEntry> = self
            .projectiles
            .iter()
            .map(|(entity, projectile)| MissileSaveEntry {
                entity: *entity,
                actor: projectile.actor.0,
                owner: projectile.owner.0,
                pass: projectile.pass.map(|actor| actor.0),
                trigger: projectile.trigger.map(|actor| actor.0),
                attachment: match projectile.attachment {
                    ProjectileAttachment::None => None,
                    ProjectileAttachment::Player(actor) => Some(actor.0),
                },
            })
            .collect();
        entries.sort_by_key(|entry| entry.entity);
        entries
    }

    /// Restore the save image.
    pub fn restore_save_state(
        &mut self,
        pool: &EntityPool,
        entries: &[MissileSaveEntry],
        resolve_actor: &dyn Fn(u32) -> Q3GameItemsResult<ActorId>,
    ) -> Q3GameItemsResult<()> {
        let mut restored = HashMap::new();
        for entry in entries {
            pool.require_owned(entry.entity)?;
            let actor = resolve_actor(entry.actor)?;
            if ActorId::from_slot(entry.entity) != actor {
                return Err(invalid("projectile actor differs from its source record"));
            }
            if restored.contains_key(&entry.entity) {
                return Err(invalid("duplicate native projectile"));
            }
            restored.insert(
                entry.entity,
                NativeProjectile {
                    actor,
                    owner: resolve_actor(entry.owner)?,
                    pass: entry.pass.map(resolve_actor).transpose()?,
                    attachment: match entry.attachment {
                        None => ProjectileAttachment::None,
                        Some(saved) => ProjectileAttachment::Player(resolve_actor(saved)?),
                    },
                    trigger: entry.trigger.map(resolve_actor).transpose()?,
                },
            );
        }
        self.projectiles = restored;
        Ok(())
    }

    /// Owner of a projectile actor.
    #[must_use]
    pub fn owner_of(&self, pool: &EntityPool, actor: ActorId) -> Option<ActorId> {
        let slot = pool.native_by_actor(actor)?;
        let projectile = self.projectiles.get(&slot)?;
        (projectile.actor == actor).then_some(projectile.owner)
    }

    fn owner_slot(&self, pool: &EntityPool, projectile: &NativeProjectile) -> Option<Slot> {
        pool.native_by_actor(projectile.owner)
    }

    fn owner_participant(&self, pool: &EntityPool, projectile: &NativeProjectile) -> DamageParticipant {
        self.owner_slot(pool, projectile)
            .map(DamageParticipant::Entity)
            .unwrap_or(DamageParticipant::SharedActor(projectile.owner))
    }

    fn release_projectile(&mut self, pool: &mut EntityPool, projectile: &NativeProjectile) -> Q3GameItemsResult<()> {
        if let Some(slot) = pool.native_by_actor(projectile.actor) {
            pool.free(slot)?;
        }
        Ok(())
    }

    /// Handle an actor release (host calls this from its registry hook).
    pub fn released(&mut self, pool: &mut EntityPool, actor: ActorId) -> Q3GameItemsResult<()> {
        let snapshot: Vec<(Slot, NativeProjectile)> = self
            .projectiles
            .iter()
            .map(|(slot, projectile)| (*slot, *projectile))
            .collect();
        for (slot, projectile) in snapshot {
            if self.projectiles.get(&slot) != Some(&projectile) {
                continue;
            }
            if projectile.actor == actor {
                self.projectiles.remove(&slot);
                let weapon = pool.at(slot)?.s.weapon;
                if weapon == Weapon::GrapplingHook {
                    if let Some(owner) = self.owner_slot(pool, &projectile) {
                        let hook = pool.at(owner)?.client.as_ref().and_then(|client| client.hook);
                        if hook == Some(slot) {
                            let client = pool.at_mut(owner)?.client_mut()?;
                            client.hook = None;
                            client.ps.pm_flags &= !MoveFlags::GRAPPLE_PULL;
                        }
                    }
                }
                if let ProjectileAttachment::Player(attached) = projectile.attachment {
                    if weapon == Weapon::ProxLauncher {
                        if let Some(target) = pool.native_by_actor(attached) {
                            let activator = pool.at(target)?.activator;
                            if pool.at(target)?.client.is_some() && activator == Some(slot) {
                                let target_ref = pool.at_mut(target)?;
                                if let Some(client) = target_ref.client.as_mut() {
                                    client.ps.e_flags &= !EF_TICKING;
                                }
                                target_ref.activator = None;
                            }
                        }
                    }
                }
                if let Some(trigger) = projectile.trigger {
                    if let Some(trigger_slot) = pool.native_by_actor(trigger) {
                        pool.free(trigger_slot)?;
                    }
                }
            } else if (pool.at(slot)?.s.weapon == Weapon::GrapplingHook && projectile.owner == actor)
                || matches!(projectile.attachment, ProjectileAttachment::Player(a) if a == actor)
            {
                self.release_projectile(pool, &projectile)?;
            } else if projectile.trigger == Some(actor) {
                if let Some(record) = self.projectiles.get_mut(&slot) {
                    record.trigger = None;
                }
                if pool.native_by_actor(projectile.actor) == Some(slot) {
                    pool.at_mut(slot)?.activator = None;
                }
            }
        }
        Ok(())
    }

    /// Whether an entity is a proximity trigger.
    pub fn is_proximity_trigger(&self, pool: &EntityPool, slot: Slot) -> Q3GameItemsResult<bool> {
        pool.require_owned(slot)?;
        Ok(pool.at(slot)?.touch == Some(CallbackName(MISSILE_PROXIMITY_TOUCH)))
    }

    fn missionpack(host: &dyn MissileHost) -> Q3GameItemsResult<()> {
        if host.is_missionpack() {
            Ok(())
        } else {
            Err(invalid("projectile requires missionpack"))
        }
    }

    fn projectile(&self, pool: &EntityPool, slot: Slot) -> Q3GameItemsResult<NativeProjectile> {
        let projectile = self
            .projectiles
            .get(&slot)
            .copied()
            .ok_or_else(|| invalid("missile continuation does not own this actor lifetime"))?;
        if pool.native_by_actor(projectile.actor) != Some(slot) {
            return Err(invalid("missile continuation does not own this actor lifetime"));
        }
        Ok(projectile)
    }

    fn projectile_state(&self, pool: &EntityPool, slot: Slot) -> Q3GameItemsResult<(ProjectileState, ProjectileSpec)> {
        self.projectile(pool, slot)?;
        let entity = pool.at(slot)?;
        Ok((
            ProjectileState {
                trajectory: entity.s.pos,
                flags: entity.s.e_flags,
            },
            ProjectileSpec {
                weapon: entity.s.weapon,
                direct: entity.damage,
                splash: entity.splash_damage,
                radius: entity.splash_radius,
                method: entity.method_of_death,
                splash_method: entity.splash_method_of_death,
                damage_point: entity.s.origin,
            },
        ))
    }

    fn projectile_context<'b>(
        &'b mut self,
        pool: &'b mut EntityPool,
        host: &'b mut dyn MissileHost,
        slot: Slot,
    ) -> Q3GameItemsResult<ProjectileContext<'b>> {
        let (state, spec) = self.projectile_state(pool, slot)?;
        Ok(ProjectileContext {
            runtime: self,
            pool,
            host,
            slot,
            state,
            spec,
        })
    }

    /// Run an owned actor projectile (`runOwned`).
    pub fn run_owned(
        &mut self,
        pool: &mut EntityPool,
        host: &mut dyn MissileHost,
        driver: &mut dyn ProjectileDriver,
        actor: OwnedActor,
    ) -> Q3GameItemsResult<bool> {
        let Some(slot) = pool.native_by_actor(actor.id) else {
            return Ok(false);
        };
        let projectile = self.projectiles.get(&slot).copied();
        let Some(projectile) = projectile else {
            return Ok(false);
        };
        if projectile.actor != actor.id {
            return Ok(false);
        }
        self.step_projectile(pool, host, driver, slot)?;
        Ok(true)
    }

    /// Bounce a projectile (`bounce`).
    pub fn bounce(
        &mut self,
        pool: &mut EntityPool,
        host: &mut dyn MissileHost,
        driver: &mut dyn ProjectileDriver,
        slot: Slot,
        trace: &ServerTraceResult,
    ) -> Q3GameItemsResult<()> {
        self.projectile(pool, slot)?;
        if trace.entity_num < 0 || trace.entity_num >= MAX_GENTITIES as i32 {
            return Err(range("missile trace entity outside 1024"));
        }
        let actor = ActorId::from_slot(trace.entity_num as usize);
        let trace = trace.with_actor_hit(actor);
        let mut context = self.projectile_context(pool, host, slot)?;
        driver.bounce(&mut context, &trace)?;
        context.write_back()
    }

    /// Explode a projectile (`explode`).
    pub fn explode(
        &mut self,
        pool: &mut EntityPool,
        host: &mut dyn MissileHost,
        driver: &mut dyn ProjectileDriver,
        slot: Slot,
    ) -> Q3GameItemsResult<()> {
        self.projectile(pool, slot)?;
        let mut context = self.projectile_context(pool, host, slot)?;
        driver.explode(&mut context)?;
        context.write_back()
    }

    /// Impact a projectile (`impact`).
    pub fn impact(
        &mut self,
        pool: &mut EntityPool,
        host: &mut dyn MissileHost,
        driver: &mut dyn ProjectileDriver,
        slot: Slot,
        trace: &ServerTraceResult,
    ) -> Q3GameItemsResult<()> {
        self.projectile(pool, slot)?;
        if trace.entity_num < 0 || trace.entity_num >= MAX_GENTITIES as i32 {
            return Err(range("missile trace entity outside 1024"));
        }
        let actor = ActorId::from_slot(trace.entity_num as usize);
        let trace = trace.with_actor_hit(actor);
        let mut context = self.projectile_context(pool, host, slot)?;
        driver.impact(&mut context, &trace)?;
        context.write_back()
    }

    /// Run a projectile (`run`).
    pub fn run(
        &mut self,
        pool: &mut EntityPool,
        host: &mut dyn MissileHost,
        driver: &mut dyn ProjectileDriver,
        slot: Slot,
    ) -> Q3GameItemsResult<()> {
        self.projectile(pool, slot)?;
        self.step_projectile(pool, host, driver, slot)
    }

    fn step_projectile(
        &mut self,
        pool: &mut EntityPool,
        host: &mut dyn MissileHost,
        driver: &mut dyn ProjectileDriver,
        slot: Slot,
    ) -> Q3GameItemsResult<()> {
        self.projectile(pool, slot)?;
        let entity = pool.at(slot)?;
        if entity.s.e_type == EntityType::Missile as i32 && !entity.free_after_event {
            let actor = self.projectiles.get(&slot).expect("projectile checked").actor;
            if let Some(body) = host.bodies().read(actor) {
                if host.has_weapon_behavior() {
                    let previous = host.combat().previous_time();
                    if let Some(update) =
                        host.weapon_behavior_step(actor, body.origin, body.velocity, f64::from(previous) / 1000.0)
                    {
                        host.bodies().write(actor, update);
                        let entity = pool.at_mut(slot)?;
                        entity.s.pos.base = update.origin;
                        entity.s.pos.delta = update.velocity;
                        entity.s.pos.time = previous;
                        entity.r.current_origin = update.origin;
                    }
                }
            }
        }
        let mut context = self.projectile_context(pool, host, slot)?;
        driver.step(&mut context)?;
        context.write_back()
    }

    #[allow(clippy::too_many_lines)]
    fn special_impact(
        &mut self,
        pool: &mut EntityPool,
        host: &mut dyn MissileHost,
        slot: Slot,
        trace: &ActorTraceResult,
        actor: ActorId,
    ) -> Q3GameItemsResult<bool> {
        let other = pool.native_by_actor(actor);
        if other.is_none() && host.combat().is_player(pool, actor) {
            return Err(invalid("admitted Q3 map player has no native client behavior record"));
        }
        let normal = trace_normal(trace);
        if host.is_missionpack() && pool.at(slot)?.s.weapon == Weapon::ProxLauncher {
            if pool.at(slot)?.s.pos.ty != TrajectoryType::Gravity {
                return Ok(true);
            }
            if let Some(other) = other {
                let record = pool.at(other)?;
                if record.s.e_type == EntityType::Player as i32 && record.health > 0 {
                    self.proximity_player(pool, host, slot, other)?;
                    return Ok(true);
                }
            }
            let base = pool.at(slot)?.s.pos.base;
            let stopped = snap_vector_towards(trace.end, base);
            set_origin(pool, slot, stopped)?;
            host.combat()
                .add_event(pool, slot, EntityEvent::ProximityMineStick, trace.surface_flags)?;
            let think = pool.think_cbs.resolve(MISSILE_SPECIAL_THINK)?;
            let time = host.combat().time();
            let die = pool.die_cbs.resolve(MISSILE_SPECIAL_DIE)?;
            pool.at_mut(slot)?.think = Some(think);
            pool.at_mut(slot)?.nextthink = time.wrapping_add(2000);
            let angles = vector_to_angles(normal);
            let entity = pool.at_mut(slot)?;
            entity.s.angles = vec3(angles.x + 90.0, angles.y, angles.z);
            entity.enemy = other;
            entity.die = Some(die);
            entity.movedir = normal;
            entity.r.mins = vec3(-4.0, -4.0, -4.0);
            entity.r.maxs = vec3(4.0, 4.0, 4.0);
            host.world().link(pool, slot)?;
            return Ok(true);
        }
        if pool.at(slot)?.classname.as_deref() == Some("hook") {
            let event = pool.spawn()?;
            let position = match other {
                Some(other) if pool.at(other)?.takedamage && pool.at(other)?.client.is_some() => {
                    host.combat()
                        .add_event(pool, event, EntityEvent::MissileHit, direction_to_byte(Some(normal)))?;
                    let number = pool.at(other)?.s.number;
                    pool.at_mut(event)?.s.other_entity_num = number;
                    if let Some(record) = self.projectiles.get_mut(&slot) {
                        record.attachment = ProjectileAttachment::Player(ActorId::from_slot(other));
                    }
                    let center = missile_center(pool, other)?;
                    snap_vector_towards(center, pool.at(slot)?.s.pos.base)
                }
                _ => {
                    host.combat()
                        .add_event(pool, event, EntityEvent::MissileMiss, direction_to_byte(Some(normal)))?;
                    pool.at_mut(slot)?.enemy = None;
                    trace.end
                }
            };
            let base = pool.at(slot)?.s.pos.base;
            let position = snap_vector_towards(position, base);
            pool.at_mut(event)?.free_after_event = true;
            pool.at_mut(event)?.s.e_type = EntityType::General as i32;
            pool.at_mut(slot)?.s.e_type = EntityType::Grapple as i32;
            set_origin(pool, slot, position)?;
            set_origin(pool, event, position)?;
            let think = pool.think_cbs.resolve(MISSILE_HOOK_THINK)?;
            let time = host.combat().time();
            pool.at_mut(slot)?.think = Some(think);
            pool.at_mut(slot)?.nextthink = time.wrapping_add(Q3_GRAPPLE_THINK_INTERVAL);
            let projectile = self.projectile(pool, slot)?;
            let Some(owner) = self.owner_slot(pool, &projectile) else {
                pool.free(slot)?;
                pool.free(event)?;
                return Ok(true);
            };
            if pool.at(owner)?.client.is_none() {
                pool.free(slot)?;
                pool.free(event)?;
                return Ok(true);
            }
            pool.at_mut(owner)?.client_mut()?.ps.pm_flags |= MoveFlags::GRAPPLE_PULL;
            let current = pool.at(slot)?.r.current_origin;
            pool.at_mut(owner)?.client_mut()?.ps.grapple_point = current;
            host.world().link(pool, slot)?;
            host.world().link(pool, event)?;
            return Ok(true);
        }
        Ok(false)
    }

    /// Free a hook (`hookFree`).
    pub fn hook_free(&mut self, pool: &mut EntityPool, slot: Slot) -> Q3GameItemsResult<()> {
        pool.require_owned(slot)?;
        let projectile = self.projectile(pool, slot)?;
        self.release_projectile(pool, &projectile)
    }

    /// Think a hook (`hookThink`).
    pub fn hook_think(&mut self, pool: &mut EntityPool, slot: Slot) -> Q3GameItemsResult<()> {
        let projectile = self.projectile(pool, slot)?;
        let Some(owner) = self.owner_slot(pool, &projectile) else {
            return self.release_projectile(pool, &projectile);
        };
        if pool.at(owner)?.client.is_none() {
            return self.release_projectile(pool, &projectile);
        }
        if let ProjectileAttachment::Player(attached) = projectile.attachment {
            let Some(target) = pool.native_by_actor(attached) else {
                return self.release_projectile(pool, &projectile);
            };
            let center = missile_center(pool, target)?;
            let current = pool.at(slot)?.r.current_origin;
            set_origin(pool, slot, snap_vector_towards(center, current))?;
        }
        let current = pool.at(slot)?.r.current_origin;
        pool.at_mut(owner)?.client_mut()?.ps.grapple_point = current;
        Ok(())
    }

    fn proximity_explode(
        &mut self,
        pool: &mut EntityPool,
        host: &mut dyn MissileHost,
        driver: &mut dyn ProjectileDriver,
        mine: Slot,
    ) -> Q3GameItemsResult<()> {
        let projectile = self.projectile(pool, mine)?;
        let trigger = projectile.trigger;
        self.explode(pool, host, driver, mine)?;
        if let Some(trigger) = trigger {
            if let Some(slot) = pool.native_by_actor(trigger) {
                pool.free(slot)?;
            }
        }
        Ok(())
    }

    fn proximity_die(
        &mut self,
        pool: &mut EntityPool,
        host: &mut dyn MissileHost,
        mine: Slot,
    ) -> Q3GameItemsResult<()> {
        let think = pool.think_cbs.resolve(MISSILE_PROXIMITY_DIE_THINK)?;
        let time = host.combat().time();
        pool.at_mut(mine)?.think = Some(think);
        pool.at_mut(mine)?.nextthink = time.wrapping_add(1);
        Ok(())
    }

    fn proximity_trigger(
        &mut self,
        pool: &mut EntityPool,
        host: &mut dyn MissileHost,
        trigger: Slot,
        other: Slot,
    ) -> Q3GameItemsResult<()> {
        if pool.at(other)?.client.is_none() {
            return Ok(());
        }
        let mine = pool
            .at(trigger)?
            .parent
            .ok_or_else(|| invalid("missile rule requires its source parent"))?;
        let distance = length3(sub3(pool.at(trigger)?.s.pos.base, pool.at(other)?.s.pos.base));
        if distance > pool.at(mine)?.splash_radius {
            return Ok(());
        }
        let game_type = host.combat().game_type();
        let team = pool
            .at(other)?
            .client
            .as_ref()
            .expect("client checked")
            .sess
            .session_team;
        if game_type >= GameType::Team as i32 && pool.at(mine)?.s.generic1 == team as i32 {
            return Ok(());
        }
        let base = pool.at(trigger)?.s.pos.base;
        if !host.combat().can_damage(pool, other, base) {
            return Ok(());
        }
        pool.at_mut(mine)?.s.loop_sound = 0;
        host.combat()
            .add_event(pool, mine, EntityEvent::ProximityMineTrigger, 0)?;
        let time = host.combat().time();
        pool.at_mut(mine)?.nextthink = time.wrapping_add(500);
        pool.free(trigger)?;
        Ok(())
    }

    fn proximity_activate(
        &mut self,
        pool: &mut EntityPool,
        host: &mut dyn MissileHost,
        mine: Slot,
    ) -> Q3GameItemsResult<()> {
        Self::missionpack(host)?;
        let think = pool.think_cbs.resolve(MISSILE_PROXIMITY_DIE_THINK)?;
        let time = host.combat().time();
        let timeout = host.prox_mine_timeout();
        let die = pool.die_cbs.resolve(MISSILE_SPECIAL_DIE)?;
        let tick = host.missionpack_sound_index("sound/weapons/proxmine/wstbtick.wav");
        pool.at_mut(mine)?.think = Some(think);
        pool.at_mut(mine)?.nextthink = time.wrapping_add(timeout);
        pool.at_mut(mine)?.takedamage = true;
        pool.at_mut(mine)?.health = 1;
        pool.at_mut(mine)?.die = Some(die);
        pool.at_mut(mine)?.s.loop_sound = tick;
        let radius = pool.at(mine)?.splash_radius;
        let base = pool.at(mine)?.s.pos.base;
        let trigger = pool.spawn()?;
        let touch = pool.touch_cbs.resolve(MISSILE_PROXIMITY_TOUCH)?;
        pool.at_mut(trigger)?.classname = Some("proxmine_trigger".to_string());
        pool.at_mut(trigger)?.r.mins = vec3(-radius, -radius, -radius);
        pool.at_mut(trigger)?.r.maxs = vec3(radius, radius, radius);
        set_origin(pool, trigger, base)?;
        pool.at_mut(trigger)?.parent = Some(mine);
        pool.at_mut(trigger)?.r.contents = CONTENTS_TRIGGER;
        pool.at_mut(trigger)?.touch = Some(touch);
        host.world().link(pool, trigger)?;
        pool.at_mut(mine)?.activator = Some(trigger);
        if let Some(record) = self.projectiles.get_mut(&mine) {
            record.trigger = Some(ActorId::from_slot(trigger));
        }
        Ok(())
    }

    fn proximity_explode_on_player(
        &mut self,
        pool: &mut EntityPool,
        host: &mut dyn MissileHost,
        driver: &mut dyn ProjectileDriver,
        mine: Slot,
    ) -> Q3GameItemsResult<()> {
        let projectile = self.projectile(pool, mine)?;
        let ProjectileAttachment::Player(attached) = projectile.attachment else {
            return Err(invalid("attached proximity mine requires its player lifetime"));
        };
        let Some(player) = pool.native_by_actor(attached) else {
            return self.release_projectile(pool, &projectile);
        };
        if pool.at(player)?.client.is_none() {
            return Err(invalid("missile rule requires a client entity"));
        }
        pool.at_mut(player)?.client_mut()?.ps.e_flags &= !EF_TICKING;
        let time = host.combat().time();
        let invulnerable = pool
            .at(player)?
            .client
            .as_ref()
            .expect("client checked")
            .invulnerability_time
            > time;
        if invulnerable {
            let owner = self.owner_participant(pool, &projectile);
            let origin = pool.at(mine)?.s.origin;
            host.combat().damage(
                pool,
                player,
                owner,
                owner,
                Some(vec3(0.0, 0.0, 0.0)),
                Some(origin),
                1000,
                DamageFlags::NO_KNOCKBACK,
                27,
                Some(projectile.actor),
            )?;
            if pool.native_by_actor(attached) != Some(player) {
                return Ok(());
            }
            pool.at_mut(player)?.client_mut()?.invulnerability_time = 0;
            let origin = pool.at(player)?.client.as_ref().expect("client checked").ps.origin;
            let (combat, world) = host.combat_and_world();
            combat.temp_entity(pool, world, origin, EntityEvent::Juiced)?;
        } else {
            let base = pool.at(player)?.s.pos.base;
            set_origin(pool, mine, base)?;
            pool.at_mut(mine)?.r.sv_flags &= !ServerEntityFlags::NOCLIENT;
            pool.at_mut(mine)?.splash_method_of_death = 25;
            self.explode(pool, host, driver, mine)?;
        }
        Ok(())
    }

    fn proximity_player(
        &mut self,
        pool: &mut EntityPool,
        host: &mut dyn MissileHost,
        mine: Slot,
        player: Slot,
    ) -> Q3GameItemsResult<()> {
        if pool.at(mine)?.s.e_flags & EF_NODRAW != 0 {
            return Ok(());
        }
        host.combat()
            .add_event(pool, mine, EntityEvent::ProximityMineStick, 0)?;
        if pool.at(player)?.s.e_flags & EF_TICKING != 0 {
            let activator = pool
                .at(player)?
                .activator
                .ok_or_else(|| invalid("ticking player requires its proximity mine"))?;
            let splash = pool.at(mine)?.splash_damage;
            let total = pool.at(activator)?.splash_damage.wrapping_add(splash);
            pool.at_mut(activator)?.splash_damage = total;
            let radius = pool.at(activator)?.splash_radius * 1.5;
            pool.at_mut(activator)?.splash_radius = radius;
            let think = pool.think_cbs.resolve(MISSILE_PROXIMITY_PLAYER_THINK)?;
            let time = host.combat().time();
            pool.at_mut(mine)?.think = Some(think);
            pool.at_mut(mine)?.nextthink = time;
            return Ok(());
        }
        if pool.at(player)?.client.is_none() {
            return Err(invalid("missile rule requires a client entity"));
        }
        pool.at_mut(player)?.client_mut()?.ps.e_flags |= EF_TICKING;
        pool.at_mut(player)?.activator = Some(mine);
        pool.at_mut(mine)?.s.e_flags |= EF_NODRAW;
        pool.at_mut(mine)?.r.sv_flags |= ServerEntityFlags::NOCLIENT;
        pool.at_mut(mine)?.s.pos.ty = TrajectoryType::Linear;
        pool.at_mut(mine)?.s.pos.delta = vec3(0.0, 0.0, 0.0);
        if let Some(record) = self.projectiles.get_mut(&mine) {
            record.attachment = ProjectileAttachment::Player(ActorId::from_slot(player));
        }
        let think = pool.think_cbs.resolve(MISSILE_PROXIMITY_ON_PLAYER)?;
        let time = host.combat().time();
        let invulnerable = pool
            .at(player)?
            .client
            .as_ref()
            .expect("client checked")
            .invulnerability_time
            > time;
        pool.at_mut(mine)?.think = Some(think);
        pool.at_mut(mine)?.nextthink = time.wrapping_add(if invulnerable { 2000 } else { 10000 });
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn launch(
        &mut self,
        pool: &mut EntityPool,
        host: &mut dyn MissileHost,
        driver: &mut dyn ProjectileDriver,
        owner_slot: Slot,
        start: Vec3,
        direction: Vec3,
        weapon: Weapon,
        classname: &str,
        speed: f32,
        duration: i32,
        gravity: bool,
        direct: i32,
        splash: i32,
        radius: f32,
        method: i32,
        splash_method: i32,
    ) -> Q3GameItemsResult<Slot> {
        pool.require_owned(owner_slot)?;
        self.check_host(pool, host)?;
        let owner = ActorId::from_slot(owner_slot);
        let time = host.combat().time();
        let fired = driver.launch(start, direction, speed, gravity, duration, time);
        let bolt = pool.spawn()?;
        let think = pool.think_cbs.resolve(MISSILE_LAUNCH_THINK)?;
        let number = pool.at(owner_slot)?.s.number;
        pool.at_mut(bolt)?.classname = Some(classname.to_string());
        pool.at_mut(bolt)?.nextthink = fired.expires;
        pool.at_mut(bolt)?.think = Some(think);
        pool.at_mut(bolt)?.s.e_type = EntityType::Missile as i32;
        pool.at_mut(bolt)?.r.sv_flags = ServerEntityFlags::USE_CURRENT_ORIGIN;
        pool.at_mut(bolt)?.s.weapon = weapon;
        pool.at_mut(bolt)?.r.owner_num = number;
        pool.at_mut(bolt)?.parent = if weapon == Weapon::GrapplingHook || weapon == Weapon::ProxLauncher {
            Some(owner_slot)
        } else {
            None
        };
        pool.at_mut(bolt)?.damage = direct;
        pool.at_mut(bolt)?.splash_damage = splash;
        pool.at_mut(bolt)?.splash_radius = radius;
        pool.at_mut(bolt)?.method_of_death = method;
        pool.at_mut(bolt)?.splash_method_of_death = splash_method;
        pool.at_mut(bolt)?.clipmask = MASK_SHOT;
        pool.at_mut(bolt)?.target_ent = None;
        pool.at_mut(bolt)?.s.pos = fired.trajectory;
        pool.at_mut(bolt)?.r.current_origin = start;
        self.projectiles.insert(
            bolt,
            NativeProjectile {
                actor: ActorId::from_slot(bolt),
                owner,
                pass: Some(owner),
                attachment: ProjectileAttachment::None,
                trigger: None,
            },
        );
        // Donor spawn registers the session body; the local pool cannot reach the
        // host table, so launch completes the registration before behavior runs.
        host.bodies().write(
            ActorId::from_slot(bolt),
            BodyState {
                origin: start,
                velocity: fired.trajectory.delta,
            },
        );
        if weapon != Weapon::Nailgun {
            self.apply_behavior_launch(pool, host, bolt, owner)?;
        }
        Ok(bolt)
    }

    fn apply_behavior_launch(
        &mut self,
        pool: &mut EntityPool,
        host: &mut dyn MissileHost,
        bolt: Slot,
        owner: ActorId,
    ) -> Q3GameItemsResult<()> {
        let actor = ActorId::from_slot(bolt);
        let body = host
            .bodies()
            .read(actor)
            .ok_or_else(|| invalid("launched Q3 missile lost its body"))?;
        if !host.has_weapon_behavior() {
            return Ok(());
        }
        let time = host.combat().time();
        let weapon = pool.at(bolt)?.s.weapon;
        let delta = pool.at(bolt)?.s.pos.delta;
        let update = host.weapon_behavior_launch(actor, owner, weapon, f64::from(time) / 1000.0, body.origin, delta);
        if let Some(update) = update {
            host.bodies().write(actor, update);
            pool.at_mut(bolt)?.s.pos.base = update.origin;
            pool.at_mut(bolt)?.s.pos.delta = update.velocity;
            pool.at_mut(bolt)?.s.pos.time = time;
            pool.at_mut(bolt)?.r.current_origin = update.origin;
        }
        Ok(())
    }

    /// Fire plasma (`firePlasma`).
    pub fn fire_plasma(
        &mut self,
        pool: &mut EntityPool,
        host: &mut dyn MissileHost,
        driver: &mut dyn ProjectileDriver,
        owner: Slot,
        start: Vec3,
        direction: &mut MissileDirection,
    ) -> Q3GameItemsResult<Slot> {
        let spec = q3_missile_parameters(Weapon::Plasmagun)?;
        let direction = normalize_direction(direction);
        self.launch(
            pool,
            host,
            driver,
            owner,
            start,
            direction,
            Weapon::Plasmagun,
            "plasma",
            spec.speed,
            spec.duration,
            spec.gravity,
            spec.direct,
            spec.splash,
            spec.radius,
            spec.method,
            spec.splash_method,
        )
    }

    /// Fire a grenade (`fireGrenade`).
    pub fn fire_grenade(
        &mut self,
        pool: &mut EntityPool,
        host: &mut dyn MissileHost,
        driver: &mut dyn ProjectileDriver,
        owner: Slot,
        start: Vec3,
        direction: &mut MissileDirection,
    ) -> Q3GameItemsResult<Slot> {
        let spec = q3_missile_parameters(Weapon::GrenadeLauncher)?;
        let direction = normalize_direction(direction);
        let bolt = self.launch(
            pool,
            host,
            driver,
            owner,
            start,
            direction,
            Weapon::GrenadeLauncher,
            "grenade",
            spec.speed,
            spec.duration,
            spec.gravity,
            spec.direct,
            spec.splash,
            spec.radius,
            spec.method,
            spec.splash_method,
        )?;
        pool.at_mut(bolt)?.s.e_flags = EF_BOUNCE_HALF;
        Ok(bolt)
    }

    /// Fire a rocket (`fireRocket`).
    pub fn fire_rocket(
        &mut self,
        pool: &mut EntityPool,
        host: &mut dyn MissileHost,
        driver: &mut dyn ProjectileDriver,
        owner: Slot,
        start: Vec3,
        direction: &mut MissileDirection,
    ) -> Q3GameItemsResult<Slot> {
        let spec = q3_missile_parameters(Weapon::RocketLauncher)?;
        let direction = normalize_direction(direction);
        self.launch(
            pool,
            host,
            driver,
            owner,
            start,
            direction,
            Weapon::RocketLauncher,
            "rocket",
            spec.speed,
            spec.duration,
            spec.gravity,
            spec.direct,
            spec.splash,
            spec.radius,
            spec.method,
            spec.splash_method,
        )
    }

    /// Fire BFG (`fireBfg`).
    pub fn fire_bfg(
        &mut self,
        pool: &mut EntityPool,
        host: &mut dyn MissileHost,
        driver: &mut dyn ProjectileDriver,
        owner: Slot,
        start: Vec3,
        direction: &mut MissileDirection,
    ) -> Q3GameItemsResult<Slot> {
        let spec = q3_missile_parameters(Weapon::Bfg)?;
        let direction = normalize_direction(direction);
        self.launch(
            pool,
            host,
            driver,
            owner,
            start,
            direction,
            Weapon::Bfg,
            "bfg",
            spec.speed,
            spec.duration,
            spec.gravity,
            spec.direct,
            spec.splash,
            spec.radius,
            spec.method,
            spec.splash_method,
        )
    }

    /// Fire a grapple (`fireGrapple`).
    pub fn fire_grapple(
        &mut self,
        pool: &mut EntityPool,
        host: &mut dyn MissileHost,
        driver: &mut dyn ProjectileDriver,
        owner: Slot,
        start: Vec3,
        direction: &mut MissileDirection,
    ) -> Q3GameItemsResult<Slot> {
        let direction = normalize_direction(direction);
        let baseq3 = host.combat().product() == Product::Baseq3;
        let bolt = self.launch(
            pool,
            host,
            driver,
            owner,
            start,
            direction,
            Weapon::GrapplingHook,
            "hook",
            Q3_GRAPPLE_SPEED,
            Q3_GRAPPLE_LIFETIME,
            false,
            0,
            0,
            0.0,
            if baseq3 { 23 } else { 28 },
            0,
        )?;
        let think = pool.think_cbs.resolve(MISSILE_GRAPPLE_THINK)?;
        let number = pool.at(owner)?.s.number;
        pool.at_mut(bolt)?.think = Some(think);
        pool.at_mut(bolt)?.s.other_entity_num = number;
        if pool.at(owner)?.client.is_none() {
            return Err(invalid("missile rule requires a client entity"));
        }
        pool.at_mut(owner)?.client_mut()?.hook = Some(bolt);
        Ok(bolt)
    }

    /// Fire a proximity mine (`fireProx`).
    pub fn fire_prox(
        &mut self,
        pool: &mut EntityPool,
        host: &mut dyn MissileHost,
        driver: &mut dyn ProjectileDriver,
        owner: Slot,
        start: Vec3,
        direction: &mut MissileDirection,
    ) -> Q3GameItemsResult<Slot> {
        Self::missionpack(host)?;
        let direction = normalize_direction(direction);
        let bolt = self.launch(
            pool,
            host,
            driver,
            owner,
            start,
            direction,
            Weapon::ProxLauncher,
            "prox mine",
            700.0,
            3000,
            true,
            0,
            100,
            150.0,
            25,
            25,
        )?;
        if pool.at(owner)?.client.is_none() {
            return Err(invalid("missile rule requires a client entity"));
        }
        let team = pool
            .at(owner)?
            .client
            .as_ref()
            .expect("client checked")
            .sess
            .session_team;
        pool.at_mut(bolt)?.s.generic1 = team as i32;
        Ok(bolt)
    }

    /// Fire a nail (`fireNail`).
    #[allow(clippy::too_many_arguments)]
    pub fn fire_nail(
        &mut self,
        pool: &mut EntityPool,
        host: &mut dyn MissileHost,
        driver: &mut dyn ProjectileDriver,
        owner: Slot,
        start: Vec3,
        forward: Vec3,
        right: Vec3,
        up: Vec3,
    ) -> Q3GameItemsResult<Slot> {
        Self::missionpack(host)?;
        let bolt = self.launch(
            pool,
            host,
            driver,
            owner,
            start,
            vec3(0.0, 0.0, 0.0),
            Weapon::Nailgun,
            "nail",
            0.0,
            10000,
            false,
            20,
            0,
            0.0,
            23,
            0,
        )?;
        let velocity = q3_nail_velocity(start, forward, right, up, host.random());
        let time = host.combat().time();
        pool.at_mut(bolt)?.s.pos.time = time;
        pool.at_mut(bolt)?.s.pos.delta = snap_vector(velocity);
        self.apply_behavior_launch(pool, host, bolt, ActorId::from_slot(owner))?;
        Ok(bolt)
    }

    fn after_move(&mut self, pool: &mut EntityPool, host: &mut dyn MissileHost, slot: Slot) -> Q3GameItemsResult<()> {
        if !host.is_missionpack() {
            return Ok(());
        }
        let entity = pool.at(slot)?;
        if entity.s.weapon != Weapon::ProxLauncher || entity.count != 0 {
            return Ok(());
        }
        let query = ActorTraceQuery {
            start: entity.r.current_origin,
            end: entity.r.current_origin,
            shape: TraceShape::Box {
                mins: entity.r.mins,
                maxs: entity.r.maxs,
            },
            pass_actor: None,
            mask: entity.clipmask,
        };
        let trace = host.world().trace_actor(pool, &query);
        let projectile = self.projectile(pool, slot)?;
        if trace.solidity == TraceSolidity::Clear
            || !matches!(trace.hit, TraceHit::Actor(actor) if actor == projectile.owner)
        {
            pool.at_mut(slot)?.count = 1;
            if let Some(record) = self.projectiles.get_mut(&slot) {
                record.pass = None;
            }
        }
        Ok(())
    }

    fn no_impact(&mut self, pool: &mut EntityPool, slot: Slot) -> Q3GameItemsResult<()> {
        let projectile = self.projectile(pool, slot)?;
        if let Some(owner) = self.owner_slot(pool, &projectile) {
            let hook = pool.at(owner)?.client.as_ref().and_then(|client| client.hook);
            if hook == Some(slot) {
                pool.at_mut(owner)?.client_mut()?.hook = None;
            }
        }
        Ok(())
    }

    /// Dispatch a think callback by name; returns false when unhandled.
    pub fn dispatch_think(
        &mut self,
        pool: &mut EntityPool,
        host: &mut dyn MissileHost,
        driver: &mut dyn ProjectileDriver,
        slot: Slot,
        name: CallbackName,
    ) -> Q3GameItemsResult<bool> {
        match name.0 {
            MISSILE_SPECIAL_THINK => {
                self.proximity_activate(pool, host, slot)?;
                Ok(true)
            }
            MISSILE_HOOK_THINK => {
                self.hook_think(pool, slot)?;
                Ok(true)
            }
            MISSILE_PROXIMITY_DIE_THINK => {
                self.proximity_explode(pool, host, driver, slot)?;
                Ok(true)
            }
            MISSILE_PROXIMITY_PLAYER_THINK => {
                pool.free(slot)?;
                Ok(true)
            }
            MISSILE_PROXIMITY_ON_PLAYER => {
                self.proximity_explode_on_player(pool, host, driver, slot)?;
                Ok(true)
            }
            MISSILE_LAUNCH_THINK => {
                self.explode(pool, host, driver, slot)?;
                Ok(true)
            }
            MISSILE_GRAPPLE_THINK => {
                self.hook_free(pool, slot)?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// Dispatch a touch callback; returns false when unhandled.
    pub fn dispatch_touch(
        &mut self,
        pool: &mut EntityPool,
        host: &mut dyn MissileHost,
        slot: Slot,
        other: DamageParticipant,
    ) -> Q3GameItemsResult<bool> {
        if pool.at(slot)?.touch != Some(CallbackName(MISSILE_PROXIMITY_TOUCH)) {
            return Ok(false);
        }
        if let DamageParticipant::Entity(other) = other {
            self.proximity_trigger(pool, host, slot, other)?;
        }
        Ok(true)
    }

    /// Dispatch a die callback; returns false when unhandled.
    pub fn dispatch_die(
        &mut self,
        pool: &mut EntityPool,
        host: &mut dyn MissileHost,
        slot: Slot,
    ) -> Q3GameItemsResult<bool> {
        if pool.at(slot)?.die != Some(CallbackName(MISSILE_SPECIAL_DIE)) {
            return Ok(false);
        }
        self.proximity_die(pool, host, slot)?;
        Ok(true)
    }

    /// Register missile save callbacks (`bindSaveCallbacks`).
    pub fn bind_save_callbacks(&self, pool: &mut EntityPool) {
        pool.think_cbs.intern(MISSILE_SPECIAL_THINK);
        pool.die_cbs.intern(MISSILE_SPECIAL_DIE);
        pool.think_cbs.intern(MISSILE_HOOK_THINK);
        pool.think_cbs.intern(MISSILE_PROXIMITY_DIE_THINK);
        pool.touch_cbs.intern(MISSILE_PROXIMITY_TOUCH);
        pool.think_cbs.intern(MISSILE_PROXIMITY_PLAYER_THINK);
        pool.think_cbs.intern(MISSILE_PROXIMITY_ON_PLAYER);
        pool.think_cbs.intern(MISSILE_LAUNCH_THINK);
        pool.think_cbs.intern(MISSILE_GRAPPLE_THINK);
    }
}

impl MissileFire for MissileRuntime {
    fn product(&self) -> Product {
        self.product.unwrap_or(Product::Baseq3)
    }

    fn fire_grenade(
        &mut self,
        pool: &mut EntityPool,
        host: &mut dyn MissileHost,
        driver: &mut dyn ProjectileDriver,
        entity: Slot,
        start: Vec3,
        direction: &mut MissileDirection,
    ) -> Q3GameItemsResult<Slot> {
        MissileRuntime::fire_grenade(self, pool, host, driver, entity, start, direction)
    }

    fn fire_rocket(
        &mut self,
        pool: &mut EntityPool,
        host: &mut dyn MissileHost,
        driver: &mut dyn ProjectileDriver,
        entity: Slot,
        start: Vec3,
        direction: &mut MissileDirection,
    ) -> Q3GameItemsResult<Slot> {
        MissileRuntime::fire_rocket(self, pool, host, driver, entity, start, direction)
    }

    fn fire_plasma(
        &mut self,
        pool: &mut EntityPool,
        host: &mut dyn MissileHost,
        driver: &mut dyn ProjectileDriver,
        entity: Slot,
        start: Vec3,
        direction: &mut MissileDirection,
    ) -> Q3GameItemsResult<Slot> {
        MissileRuntime::fire_plasma(self, pool, host, driver, entity, start, direction)
    }
}
