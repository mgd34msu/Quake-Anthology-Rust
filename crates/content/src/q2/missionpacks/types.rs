//! Q2 mission-pack types (`src/content/q2/missionpacks/types.ts`).

use qa_core::identity::ActorId;
use qa_core::math::Vec3;

use crate::q2::foundation::host::Q2GameServices;
use crate::q2::foundation::monsters::types::MonsterContext;

/// Mission pack (`Q2MissionPack`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q2MissionPack {
    /// Xatrix.
    Xatrix,
    /// Rogue.
    Rogue,
}

/// Mission-pack damage causes (`q2MissionPackDamage`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Q2MissionPackDamage {
    /// Ripper.
    pub ripper: i32,
    /// Phalanx.
    pub phalanx: i32,
    /// Brain tentacle.
    pub brain_tentacle: i32,
    /// Blastoff.
    pub blastoff: i32,
    /// Gekk.
    pub gekk: i32,
    /// Trap.
    pub trap: i32,
    /// Chainfist.
    pub chainfist: i32,
    /// Disintegrator.
    pub disintegrator: i32,
    /// Flechette.
    pub flechette: i32,
    /// Blaster2.
    pub blaster2: i32,
    /// Heatbeam.
    pub heatbeam: i32,
    /// Tesla.
    pub tesla: i32,
    /// Prox.
    pub prox: i32,
    /// Nuke.
    pub nuke: i32,
    /// Vengeance sphere.
    pub vengeance_sphere: i32,
    /// Hunter sphere.
    pub hunter_sphere: i32,
    /// Defender sphere.
    pub defender_sphere: i32,
    /// Tracker.
    pub tracker: i32,
    /// Deathball crush.
    pub deathball_crush: i32,
    /// Dopple explode.
    pub dopple_explode: i32,
    /// Dopple vengeance.
    pub dopple_vengeance: i32,
    /// Dopple hunter.
    pub dopple_hunter: i32,
}

/// Mission-pack damage causes.
pub const Q2_MISSION_PACK_DAMAGE: Q2MissionPackDamage = Q2MissionPackDamage {
    ripper: 34,
    phalanx: 35,
    brain_tentacle: 36,
    blastoff: 37,
    gekk: 38,
    trap: 39,
    chainfist: 40,
    disintegrator: 41,
    flechette: 42,
    blaster2: 43,
    heatbeam: 44,
    tesla: 45,
    prox: 46,
    nuke: 47,
    vengeance_sphere: 48,
    hunter_sphere: 49,
    defender_sphere: 50,
    tracker: 51,
    deathball_crush: 52,
    dopple_explode: 53,
    dopple_vengeance: 54,
    dopple_hunter: 55,
};

/// Mission-pack player effect (`Q2MissionPackProjectileHooks["playerEffect"]` payload).
#[derive(Debug, Clone, PartialEq)]
pub enum Q2MissionPackPlayerEffect {
    /// Tracker pain overlay.
    TrackerPain {
        /// Actor.
        actor: ActorId,
        /// Effect until.
        until: f64,
    },
    /// Nuke blindness overlay.
    NukeBlind {
        /// Actor.
        actor: ActorId,
        /// Effect until.
        until: f64,
    },
    /// IR overlay.
    Ir {
        /// Actor.
        actor: ActorId,
        /// Effect until.
        until: f64,
    },
    /// Sphere camera.
    SphereCamera {
        /// Actor.
        actor: ActorId,
        /// Sphere.
        sphere: Option<ActorId>,
        /// Origin.
        origin: Vec3,
        /// Angles.
        angles: Vec3,
    },
}

/// Mission-pack projectile hooks (`Q2MissionPackProjectileHooks`).
///
/// The donor `base` ballistics module maps to the port's ballistics
/// free functions and weapon input tables, so the handle carries only
/// the session-provided hooks.
#[derive(Debug, Clone, Copy)]
pub struct Q2MissionPackProjectileHooks {
    /// Strong mines.
    pub strong_mines: bool,
    /// Gravity override.
    pub gravity: Option<fn(&Q2GameServices) -> f64>,
    /// Resolve a monster context.
    pub monster: for<'a> fn(ActorId, &'a mut Q2GameServices) -> Option<MonsterContext<'a>>,
    /// Emit a player effect.
    pub player_effect: fn(&Q2GameServices, Q2MissionPackPlayerEffect),
}
