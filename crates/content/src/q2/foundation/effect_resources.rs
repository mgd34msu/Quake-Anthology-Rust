//! Q2 effect resources (`src/content/q2/foundation/effect-resources.ts`).
//!
//! Gameplay logic adapted from id Software's Quake II game and the
//! rerelease game DLL (GPL-2.0-or-later).

/// Transient models (`Q2_TRANSIENT_MODELS`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Q2TransientModels {
    /// Cable segment.
    pub cable: &'static str,
    /// Parasite segment.
    pub parasite: &'static str,
    /// Explosion.
    pub explosion: &'static str,
    /// Muzzle flash.
    pub flash: &'static str,
    /// Rocket explosion.
    pub rocket_explosion: &'static str,
    /// Smoke puff.
    pub smoke: &'static str,
    /// Lightning bolt.
    pub lightning: &'static str,
    /// BFG explosion.
    pub bfg_explosion: &'static str,
}

/// Transient models consumed by the common effect renderer.
pub const Q2_TRANSIENT_MODELS: Q2TransientModels = Q2TransientModels {
    cable: "models/ctf/segment/tris.md2",
    parasite: "models/monsters/parasite/segment/tris.md2",
    explosion: "models/objects/explode/tris.md2",
    flash: "models/objects/flash/tris.md2",
    rocket_explosion: "models/objects/r_explode/tris.md2",
    smoke: "models/objects/smoke/tris.md2",
    lightning: "models/proj/lightning/tris.md2",
    bfg_explosion: "sprites/s_bfg2.sp2",
};

/// Transient sounds (`Q2_TRANSIENT_SOUNDS`).
pub const Q2_TRANSIENT_SOUNDS: [&str; 14] = [
    "world/ric1.wav",
    "world/ric2.wav",
    "world/ric3.wav",
    "weapons/lashit.wav",
    "world/spark5.wav",
    "world/spark6.wav",
    "world/spark7.wav",
    "weapons/railgf1a.wav",
    "weapons/rocklx1a.wav",
    "weapons/grenlx1a.wav",
    "weapons/xpld_wat.wav",
    "player/land1.wav",
    "player/fall2.wav",
    "player/fall1.wav",
];

/// Rogue transient sounds (`Q2_ROGUE_TRANSIENT_SOUNDS`).
pub const Q2_ROGUE_TRANSIENT_SOUNDS: [&str; 2] =
    ["weapons/tesla.wav", "weapons/disrupthit.wav"];
