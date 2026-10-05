//! Q1 foundation data types (`src/content/q1/foundation/types.ts`).
//!
//! Q1 gameplay adapted from id Software Quake / Quake rerelease QuakeC.
//! Copyright (C) 1996-2022 id Software LLC. GPL-2.0-or-later.
//!
//! qsrc functionality reference: `quake/progs106/defs.qc:245-256`
//! (`.movetype` constants: `MOVETYPE_WALK=3` players-only,
//! `MOVETYPE_STEP=4` monsters).

use std::collections::HashMap;

use qa_core::identity::{ActorId, OwnedActor, ProviderId};
use qa_core::math::{Bounds, Vec3};

use crate::contract::ItemId;
use crate::q1::Q1Error;

/// Official Q1 provider (`Q1_PROVIDER`).
#[must_use]
pub fn q1_provider() -> ProviderId {
    ProviderId::new("q1", "official")
}

/// Zero vector (`ZERO`).
pub const ZERO: Vec3 = Vec3 { x: 0.0, y: 0.0, z: 0.0 };
/// Degenerate point bounds (`POINT`).
pub const POINT: Bounds = Bounds { min: ZERO, max: ZERO };
/// Player collision bounds (`PLAYER_BOUNDS`).
pub const PLAYER_BOUNDS: Bounds = Bounds {
    min: Vec3 {
        x: -16.0,
        y: -16.0,
        z: -24.0,
    },
    max: Vec3 {
        x: 16.0,
        y: 16.0,
        z: 32.0,
    },
};

/// Base id1 weapon (`Q1BaseWeapon`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q1BaseWeapon {
    /// Axe.
    Axe,
    /// Shotgun.
    Shotgun,
    /// Double-barrelled shotgun.
    Supershotgun,
    /// Nailgun.
    Nailgun,
    /// Super nailgun.
    Supernailgun,
    /// Grenade launcher.
    Grenadelauncher,
    /// Rocket launcher.
    Rocketlauncher,
    /// Thunderbolt.
    Lightning,
}

impl Q1BaseWeapon {
    /// Donor weapon id text.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Q1BaseWeapon::Axe => "axe",
            Q1BaseWeapon::Shotgun => "shotgun",
            Q1BaseWeapon::Supershotgun => "supershotgun",
            Q1BaseWeapon::Nailgun => "nailgun",
            Q1BaseWeapon::Supernailgun => "supernailgun",
            Q1BaseWeapon::Grenadelauncher => "grenadelauncher",
            Q1BaseWeapon::Rocketlauncher => "rocketlauncher",
            Q1BaseWeapon::Lightning => "lightning",
        }
    }
}

/// Base weapon roster in donor order (`WEAPONS`).
pub const WEAPONS: [Q1BaseWeapon; 8] = [
    Q1BaseWeapon::Axe,
    Q1BaseWeapon::Shotgun,
    Q1BaseWeapon::Supershotgun,
    Q1BaseWeapon::Nailgun,
    Q1BaseWeapon::Supernailgun,
    Q1BaseWeapon::Grenadelauncher,
    Q1BaseWeapon::Rocketlauncher,
    Q1BaseWeapon::Lightning,
];

/// Full Q1 weapon id, including mission-pack, MG3, and CTF additions
/// (`Q1Weapon`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q1Weapon {
    /// Axe.
    Axe,
    /// Shotgun.
    Shotgun,
    /// Double-barrelled shotgun.
    Supershotgun,
    /// Nailgun.
    Nailgun,
    /// Super nailgun.
    Supernailgun,
    /// Grenade launcher.
    Grenadelauncher,
    /// Rocket launcher.
    Rocketlauncher,
    /// Thunderbolt.
    Lightning,
    /// Hipnotic laser cannon.
    HipnoticLaser,
    /// Hipnotic mjolnir.
    HipnoticMjolnir,
    /// Hipnotic proximity gun.
    HipnoticProximity,
    /// Rogue lava nailgun.
    RogueLavaNailgun,
    /// Rogue lava super nailgun.
    RogueLavaSupernailgun,
    /// Rogue multi grenade.
    RogueMultiGrenade,
    /// Rogue multi rocket.
    RogueMultiRocket,
    /// Rogue plasma.
    RoguePlasma,
    /// Rogue grapple.
    RogueGrapple,
    /// MG3 laser.
    Mg3Laser,
    /// MG3 mjolnir.
    Mg3Mjolnir,
    /// CTF grapple.
    CtfGrapple,
}

impl Q1Weapon {
    /// Donor weapon id text.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Q1Weapon::Axe => "axe",
            Q1Weapon::Shotgun => "shotgun",
            Q1Weapon::Supershotgun => "supershotgun",
            Q1Weapon::Nailgun => "nailgun",
            Q1Weapon::Supernailgun => "supernailgun",
            Q1Weapon::Grenadelauncher => "grenadelauncher",
            Q1Weapon::Rocketlauncher => "rocketlauncher",
            Q1Weapon::Lightning => "lightning",
            Q1Weapon::HipnoticLaser => "hipnotic:laser",
            Q1Weapon::HipnoticMjolnir => "hipnotic:mjolnir",
            Q1Weapon::HipnoticProximity => "hipnotic:proximity",
            Q1Weapon::RogueLavaNailgun => "rogue:lava-nailgun",
            Q1Weapon::RogueLavaSupernailgun => "rogue:lava-supernailgun",
            Q1Weapon::RogueMultiGrenade => "rogue:multi-grenade",
            Q1Weapon::RogueMultiRocket => "rogue:multi-rocket",
            Q1Weapon::RoguePlasma => "rogue:plasma",
            Q1Weapon::RogueGrapple => "rogue:grapple",
            Q1Weapon::Mg3Laser => "mg3:laser",
            Q1Weapon::Mg3Mjolnir => "mg3:mjolnir",
            Q1Weapon::CtfGrapple => "ctf:grapple",
        }
    }

    /// Parse a donor weapon id.
    pub fn parse(text: &str) -> Result<Self, Q1Error> {
        for weapon in Q1_WEAPON_IDS {
            if weapon.as_str() == text {
                return Ok(weapon);
            }
        }
        Err(Q1Error::Message(format!("Unknown Q1 weapon: {text}")))
    }
}

impl From<Q1BaseWeapon> for Q1Weapon {
    fn from(weapon: Q1BaseWeapon) -> Self {
        match weapon {
            Q1BaseWeapon::Axe => Q1Weapon::Axe,
            Q1BaseWeapon::Shotgun => Q1Weapon::Shotgun,
            Q1BaseWeapon::Supershotgun => Q1Weapon::Supershotgun,
            Q1BaseWeapon::Nailgun => Q1Weapon::Nailgun,
            Q1BaseWeapon::Supernailgun => Q1Weapon::Supernailgun,
            Q1BaseWeapon::Grenadelauncher => Q1Weapon::Grenadelauncher,
            Q1BaseWeapon::Rocketlauncher => Q1Weapon::Rocketlauncher,
            Q1BaseWeapon::Lightning => Q1Weapon::Lightning,
        }
    }
}

/// Full weapon roster in donor order (`Q1_WEAPON_IDS`).
pub const Q1_WEAPON_IDS: [Q1Weapon; 20] = [
    Q1Weapon::Axe,
    Q1Weapon::Shotgun,
    Q1Weapon::Supershotgun,
    Q1Weapon::Nailgun,
    Q1Weapon::Supernailgun,
    Q1Weapon::Grenadelauncher,
    Q1Weapon::Rocketlauncher,
    Q1Weapon::Lightning,
    Q1Weapon::HipnoticLaser,
    Q1Weapon::HipnoticMjolnir,
    Q1Weapon::HipnoticProximity,
    Q1Weapon::RogueLavaNailgun,
    Q1Weapon::RogueLavaSupernailgun,
    Q1Weapon::RogueMultiGrenade,
    Q1Weapon::RogueMultiRocket,
    Q1Weapon::RoguePlasma,
    Q1Weapon::RogueGrapple,
    Q1Weapon::Mg3Laser,
    Q1Weapon::Mg3Mjolnir,
    Q1Weapon::CtfGrapple,
];

/// Whether the weapon is a base id1 weapon (`isQ1BaseWeapon`).
#[must_use]
pub fn is_q1_base_weapon(weapon: Q1Weapon) -> bool {
    WEAPONS.iter().any(|candidate| Q1Weapon::from(*candidate) == weapon)
}

/// Inventory item for a weapon (`weaponItem`).
#[must_use]
pub fn weapon_item(weapon: Q1Weapon) -> ItemId {
    format!("q1:weapon/{}", weapon.as_str())
}

/// Classic weapon bit (`q1WeaponBit`).
#[must_use]
pub fn q1_weapon_bit(weapon: Q1BaseWeapon) -> i32 {
    match weapon {
        Q1BaseWeapon::Axe => 4096,
        Q1BaseWeapon::Shotgun => 1,
        Q1BaseWeapon::Supershotgun => 2,
        Q1BaseWeapon::Nailgun => 4,
        Q1BaseWeapon::Supernailgun => 8,
        Q1BaseWeapon::Grenadelauncher => 16,
        Q1BaseWeapon::Rocketlauncher => 32,
        Q1BaseWeapon::Lightning => 64,
    }
}

/// Timed powerup (`Q1Powerup`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q1Powerup {
    /// Quad damage.
    Quad,
    /// Pentagram of protection.
    Invulnerability,
    /// Ring of shadows.
    Invisibility,
    /// Biosuit.
    Suit,
    /// Hipnotic wetsuit.
    HipnoticWetsuit,
    /// Hipnotic empathy shield.
    HipnoticEmpathy,
    /// Rogue shield.
    RogueShield,
    /// Rogue antigrav.
    RogueAntigrav,
    /// MG3 lava suit.
    Mg3Lavasuit,
}

impl Q1Powerup {
    /// Donor powerup id text.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Q1Powerup::Quad => "quad",
            Q1Powerup::Invulnerability => "invulnerability",
            Q1Powerup::Invisibility => "invisibility",
            Q1Powerup::Suit => "suit",
            Q1Powerup::HipnoticWetsuit => "hipnotic:wetsuit",
            Q1Powerup::HipnoticEmpathy => "hipnotic:empathy",
            Q1Powerup::RogueShield => "rogue:shield",
            Q1Powerup::RogueAntigrav => "rogue:antigrav",
            Q1Powerup::Mg3Lavasuit => "mg3:lavasuit",
        }
    }

    /// Parse a donor powerup id.
    pub fn parse(text: &str) -> Result<Self, Q1Error> {
        for powerup in Q1_POWERUP_IDS {
            if powerup.as_str() == text {
                return Ok(powerup);
            }
        }
        Err(Q1Error::Message(format!("Unknown Q1 powerup: {text}")))
    }
}

/// Powerup roster in donor order (`Q1_POWERUP_IDS`).
pub const Q1_POWERUP_IDS: [Q1Powerup; 9] = [
    Q1Powerup::Quad,
    Q1Powerup::Invulnerability,
    Q1Powerup::Invisibility,
    Q1Powerup::Suit,
    Q1Powerup::HipnoticWetsuit,
    Q1Powerup::HipnoticEmpathy,
    Q1Powerup::RogueShield,
    Q1Powerup::RogueAntigrav,
    Q1Powerup::Mg3Lavasuit,
];

/// Sound channel: named source channels or a raw engine channel
/// (`Q1SoundChannel`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q1SoundChannel {
    /// Automatic channel.
    Auto,
    /// Weapon channel.
    Weapon,
    /// Voice channel.
    Voice,
    /// Item channel.
    Item,
    /// Body channel.
    Body,
    /// Raw engine channel (`-1`, `5`, `6`, `7` in the donor).
    Raw(i32),
}

/// Beam presentation style (`Q1BeamStyle`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q1BeamStyle {
    /// Lightning bolt variant 1.
    Lightning1,
    /// Lightning bolt variant 2.
    Lightning2,
    /// Lightning bolt variant 3.
    Lightning3,
    /// Grapple beam.
    Grapple,
}

impl Q1BeamStyle {
    /// Donor style text.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Q1BeamStyle::Lightning1 => "lightning1",
            Q1BeamStyle::Lightning2 => "lightning2",
            Q1BeamStyle::Lightning3 => "lightning3",
            Q1BeamStyle::Grapple => "grapple",
        }
    }
}

/// Collision solidity (`Q1Solid`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Q1Solid {
    /// Not solid.
    #[default]
    None,
    /// Trigger volume.
    Trigger,
    /// Bounding box.
    Bbox,
    /// Monster bounding box.
    Slidebox,
    /// Brush model.
    Bsp,
    /// Corpse.
    Corpse,
}

impl Q1Solid {
    /// Donor solidity text.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Q1Solid::None => "none",
            Q1Solid::Trigger => "trigger",
            Q1Solid::Bbox => "bbox",
            Q1Solid::Slidebox => "slidebox",
            Q1Solid::Bsp => "bsp",
            Q1Solid::Corpse => "corpse",
        }
    }

    /// Parse donor solidity text.
    pub fn parse(text: &str) -> Result<Self, Q1Error> {
        match text {
            "none" => Ok(Q1Solid::None),
            "trigger" => Ok(Q1Solid::Trigger),
            "bbox" => Ok(Q1Solid::Bbox),
            "slidebox" => Ok(Q1Solid::Slidebox),
            "bsp" => Ok(Q1Solid::Bsp),
            "corpse" => Ok(Q1Solid::Corpse),
            _ => Err(Q1Error::Message(format!("Unknown Q1 solidity: {text}"))),
        }
    }
}

/// Movement type (`Q1MoveType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Q1MoveType {
    /// No movement.
    #[default]
    None,
    /// Pusher.
    Push,
    /// Player ground movement (`MOVETYPE_WALK`; players only per
    /// `quake/progs106/defs.qc:248`). Only `Walk` targets take T_Damage
    /// knockback (`quake/progs106/combat.qc:141`).
    Walk,
    /// Step.
    Step,
    /// Toss.
    Toss,
    /// Bounce.
    Bounce,
    /// Fly.
    Fly,
    /// Fly missile.
    Flymissile,
    /// Noclip.
    Noclip,
    /// Gib.
    Gib,
}

impl Q1MoveType {
    /// Donor movement text.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Q1MoveType::None => "none",
            Q1MoveType::Push => "push",
            Q1MoveType::Walk => "walk",
            Q1MoveType::Step => "step",
            Q1MoveType::Toss => "toss",
            Q1MoveType::Bounce => "bounce",
            Q1MoveType::Fly => "fly",
            Q1MoveType::Flymissile => "flymissile",
            Q1MoveType::Noclip => "noclip",
            Q1MoveType::Gib => "gib",
        }
    }

    /// Parse donor movement text.
    pub fn parse(text: &str) -> Result<Self, Q1Error> {
        match text {
            "none" => Ok(Q1MoveType::None),
            "push" => Ok(Q1MoveType::Push),
            "walk" => Ok(Q1MoveType::Walk),
            "step" => Ok(Q1MoveType::Step),
            "toss" => Ok(Q1MoveType::Toss),
            "bounce" => Ok(Q1MoveType::Bounce),
            "fly" => Ok(Q1MoveType::Fly),
            "flymissile" => Ok(Q1MoveType::Flymissile),
            "noclip" => Ok(Q1MoveType::Noclip),
            "gib" => Ok(Q1MoveType::Gib),
            _ => Err(Q1Error::Message(format!("Unknown Q1 movement: {text}"))),
        }
    }
}

/// Trace result (`Q1Trace`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1Trace {
    /// Fraction of the requested distance travelled.
    pub fraction: f64,
    /// Trace end position.
    pub end: Vec3,
    /// Impact plane normal.
    pub normal: Vec3,
    /// Hit actor, if any.
    pub actor: Option<ActorId>,
    /// The trace started inside solid geometry.
    pub start_solid: bool,
    /// The whole trace stayed inside solid geometry.
    pub all_solid: bool,
    /// The trace hit sky.
    pub sky: bool,
    /// The trace ended in open space.
    pub in_open: bool,
    /// The trace ended in water.
    pub in_water: bool,
}

/// Trace request (`Q1TraceRequest`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1TraceRequest {
    /// Trace start.
    pub start: Vec3,
    /// Trace end.
    pub end: Vec3,
    /// Trace bounds.
    pub bounds: Bounds,
    /// Actor to ignore.
    pub ignore: Option<ActorId>,
    /// Whether monsters block the trace.
    pub monsters: bool,
    /// Whether to use the source missile trace policy.
    pub missile: bool,
}

/// Angle basis (`Q1Basis`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Q1Basis {
    /// Forward direction.
    pub forward: Vec3,
    /// Right direction.
    pub right: Vec3,
    /// Up direction.
    pub up: Vec3,
}

/// Presented character attack (`Q1CharacterAttack`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q1CharacterAttack {
    /// Axe swing with animation variant.
    Axe {
        /// Animation variant (`0..=3`).
        variant: u8,
    },
    /// Shotgun attack.
    Shotgun,
    /// Rocket attack.
    Rocket,
    /// Nail attack.
    Nail,
    /// Lightning attack.
    Lightning,
}

/// Message format part (`Q1MessagePart`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1MessagePart {
    /// Format text.
    pub text: String,
    /// Format arguments.
    pub args: Option<Vec<Q1MessageArg>>,
}

/// Message format argument.
#[derive(Debug, Clone, PartialEq)]
pub enum Q1MessageArg {
    /// String argument.
    Text(String),
    /// Numeric argument.
    Number(f64),
}

/// Temp-entity effect (`effect` event effect union).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q1Effect {
    /// Blood spray.
    Blood,
    /// Bullet impact.
    Gunshot,
    /// Nail impact.
    Spike,
    /// Super nail impact.
    Superspike,
    /// Explosion.
    Explosion,
    /// Teleport splash.
    Teleport,
    /// Muzzle flash.
    Muzzleflash,
    /// Item pickup.
    Pickup,
    /// Lava splash.
    LavaSplash,
    /// Tar explosion.
    TarExplosion,
    /// Meat spray.
    MeatSpray,
    /// Wizard spike impact.
    WizardSpike,
    /// Knight spike impact.
    KnightSpike,
}

impl Q1Effect {
    /// Donor effect text.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Q1Effect::Blood => "blood",
            Q1Effect::Gunshot => "gunshot",
            Q1Effect::Spike => "spike",
            Q1Effect::Superspike => "superspike",
            Q1Effect::Explosion => "explosion",
            Q1Effect::Teleport => "teleport",
            Q1Effect::Muzzleflash => "muzzleflash",
            Q1Effect::Pickup => "pickup",
            Q1Effect::LavaSplash => "lava-splash",
            Q1Effect::TarExplosion => "tar-explosion",
            Q1Effect::MeatSpray => "meat-spray",
            Q1Effect::WizardSpike => "wizard-spike",
            Q1Effect::KnightSpike => "knight-spike",
        }
    }
}

/// Engine presentation event (`Q1Event`).
#[derive(Debug, Clone, PartialEq)]
pub enum Q1Event {
    /// Stop a looping sound.
    StopSound {
        /// Sound owner.
        actor: ActorId,
        /// Channel to stop.
        channel: i32,
    },
    /// Play a positioned sound.
    Sound {
        /// Explicit origin, when not derived from the actor.
        origin: Option<Vec3>,
        /// Sound owner.
        actor: ActorId,
        /// Sound path.
        path: String,
        /// Sound channel.
        channel: Q1SoundChannel,
        /// Distance attenuation.
        attenuation: f64,
        /// Playback volume.
        volume: f64,
    },
    /// Play an ambient sound.
    Ambient {
        /// Sound origin.
        origin: Vec3,
        /// Sound path.
        path: String,
        /// Playback volume.
        volume: f64,
        /// Distance attenuation.
        attenuation: f64,
    },
    /// Center/print a message to a player.
    Message {
        /// Receiving player.
        player: ActorId,
        /// Message text.
        text: String,
        /// Whether the message is center-printed.
        center: bool,
        /// Format arguments.
        args: Option<Vec<Q1MessageArg>>,
        /// Format parts.
        parts: Option<Vec<Q1MessagePart>>,
    },
    /// Broadcast a temp-entity effect.
    Effect {
        /// Effect kind.
        effect: Q1Effect,
        /// Associated actor, if any.
        actor: Option<ActorId>,
        /// Effect origin.
        origin: Vec3,
        /// Effect magnitude.
        amount: i32,
        /// Muzzle orientation, when relevant.
        muzzle: Option<Q1Muzzle>,
    },
    /// Broadcast a colored explosion.
    ColoredExplosion {
        /// Explosion origin.
        origin: Vec3,
        /// First particle color.
        color_start: i32,
        /// Particle color run length.
        color_length: i32,
    },
    /// Register a static model.
    StaticModel {
        /// Model path.
        path: String,
        /// Model frame.
        frame: i32,
        /// Color map.
        color_map: i32,
        /// Model skin.
        skin: i32,
        /// Model origin.
        origin: Vec3,
        /// Model angles.
        angles: Vec3,
    },
    /// Broadcast particles.
    Particles {
        /// Particle origin.
        origin: Vec3,
        /// Particle direction.
        direction: Vec3,
        /// Particle color.
        color: i32,
        /// Particle count.
        count: i32,
    },
    /// Issue a server command.
    ServerCommand {
        /// Command text.
        text: String,
    },
    /// Move a player camera.
    Camera {
        /// Viewing player.
        player: ActorId,
        /// Camera origin.
        origin: Vec3,
        /// Camera angles.
        angles: Vec3,
        /// View offset override.
        view_offset: Option<Vec3>,
    },
    /// Draw a beam.
    Beam {
        /// Beam style.
        style: Q1BeamStyle,
        /// Owning actor.
        actor: ActorId,
        /// Beam start.
        start: Vec3,
        /// Beam end.
        end: Vec3,
    },
    /// Set a light style.
    Lightstyle {
        /// Style slot.
        style: i32,
        /// Light pattern.
        pattern: String,
    },
    /// Report the monster total.
    MonsterTotal {
        /// Total monsters.
        total: i32,
    },
    /// Report a found secret.
    Secret {
        /// Triggering actor.
        actor: ActorId,
        /// Total secrets.
        total: i32,
        /// Secrets found.
        found: i32,
    },
    /// Report a monster kill.
    MonsterKilled {
        /// Killed monster.
        actor: ActorId,
        /// Total monsters.
        total: i32,
        /// Monsters killed.
        found: i32,
    },
    /// Present a player weapon.
    Weapon {
        /// Owning player.
        player: ActorId,
        /// Presented weapon.
        weapon: Q1Weapon,
        /// View model path.
        view_model: String,
        /// View model frame.
        frame: i32,
        /// View punch.
        punch: i32,
        /// Presented attack, if any.
        attack: Option<Q1CharacterAttack>,
    },
    /// Lock a teleported player to new angles.
    TeleportPlayer {
        /// Teleported player.
        player: ActorId,
        /// New view angles.
        angles: Vec3,
        /// Control lock expiry in seconds.
        lock_until: f64,
    },
    /// Present a timed powerup.
    Powerup {
        /// Owning player.
        player: ActorId,
        /// Powerup kind.
        powerup: Q1Powerup,
        /// Expiry in seconds.
        expires: f64,
    },
    /// Begin an intermission camera.
    Intermission {
        /// Camera origin.
        origin: Vec3,
        /// Camera angles.
        angles: Vec3,
        /// Destination map.
        map: String,
        /// Exit availability in seconds.
        exit_after: f64,
        /// CD track.
        track: i32,
    },
    /// Show finale text.
    Finale {
        /// Finale text.
        text: String,
        /// Finale stage (`1..=6`).
        stage: u8,
    },
    /// Award an achievement.
    Achievement {
        /// Earning player, if any.
        player: Option<ActorId>,
        /// Achievement id.
        id: String,
    },
}

/// Muzzle orientation for an effect.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q1Muzzle {
    /// Muzzle origin.
    pub origin: Vec3,
    /// Muzzle angles.
    pub angles: Vec3,
}

/// Precache phase (`Q1PrecacheTables["phase"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Q1PrecachePhase {
    /// Accepting declarations.
    #[default]
    Loading,
    /// Sealed after spawn.
    Frozen,
}

/// Declared precache tables (`Q1PrecacheTables`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Q1PrecacheTables {
    /// Registry phase.
    pub phase: Q1PrecachePhase,
    /// Declared models; slot zero is the source empty string.
    pub models: Vec<String>,
    /// Declared sounds; slot zero is the source empty string.
    pub sounds: Vec<String>,
}

/// Source content edition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q1Edition {
    /// Classic id1.
    Classic,
    /// Rerelease.
    Rerelease,
}

/// Source program allowed native precache calls (`precacheProgram`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q1PrecacheProgram {
    /// Original id1 program.
    Id1,
}

/// Foundation options (`Q1FoundationOptions`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1FoundationOptions {
    /// Operating provider.
    pub provider: Option<ProviderId>,
    /// Source program allowed native precache calls.
    pub precache_program: Option<Q1PrecacheProgram>,
    /// Source content edition.
    pub edition: Q1Edition,
    /// Selected engine physics behavior.
    pub physics_edition: Option<Q1Edition>,
    /// Skill level (`0..=3`).
    pub skill: i32,
    /// Deathmatch mode.
    pub deathmatch: i32,
    /// Cooperative mode.
    pub coop: bool,
    /// Campaign provider.
    pub campaign: ProviderId,
    /// Combat provider.
    pub combat_provider: ProviderId,
    /// Movement provider.
    pub movement_provider: ProviderId,
    /// Inventory provider.
    pub inventory_provider: ProviderId,
    /// Gravity magnitude.
    pub gravity: f64,
    /// Reserved client slots.
    pub max_clients: Option<i32>,
    /// Exit suppression.
    pub no_exit: Option<i32>,
    /// Teamplay mode.
    pub teamplay: Option<i32>,
    /// Autoaim alignment threshold.
    pub aim_threshold: Option<f64>,
}

/// Presented entity snapshot (`Q1Presentation`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1Presentation {
    /// Entity actor.
    pub actor: ActorId,
    /// Entity classname.
    pub classname: String,
    /// Model path.
    pub model: String,
    /// Model frame.
    pub frame: i32,
    /// Model skin.
    pub skin: i32,
    /// Effect flags.
    pub effects: i32,
    /// Solidity.
    pub solid: Q1Solid,
    /// Movement type.
    pub movement: Q1MoveType,
    /// Target name.
    pub targetname: String,
    /// Source entity ordinal.
    pub source_ordinal: Option<i32>,
}

/// Automatic weapon switch policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Q1AutoSwitch {
    /// Always switch.
    #[default]
    Always,
    /// Switch to new weapons only.
    New,
    /// Never switch.
    Never,
}

/// Per-player Q1 arsenal state (`Q1PlayerState`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1PlayerState {
    /// Fade alpha.
    pub alpha: f64,
    /// Model scale.
    pub scale: f64,
    /// Owning actor.
    pub actor: OwnedActor,
    /// Selected weapon.
    pub weapon: Q1Weapon,
    /// Whether the primary weapon is holstered.
    pub primary_holstered: bool,
    /// Next allowed attack time in seconds.
    pub attack_finished: f64,
    /// Whether attack is held.
    pub attack_held: bool,
    /// Whether jump is held.
    pub jump_held: bool,
    /// Teleport control lock expiry in seconds.
    pub teleport_until: f64,
    /// View model frame.
    pub weapon_frame: i32,
    /// Weapon animation start in seconds (`-1` when idle).
    pub weapon_animation_at: f64,
    /// Weapon animation base frame.
    pub weapon_animation_base: i32,
    /// Whether the weapon fires continuously.
    pub continuous_firing: bool,
    /// Next continuous-fire time in seconds.
    pub next_weapon_frame: f64,
    /// Next lightning-loop time in seconds.
    pub lightning_sound_at: f64,
    /// View punch angles.
    pub punch_angles: Vec3,
    /// Nailgun muzzle side.
    pub nail_side: f64,
    /// Maximum health.
    pub max_health: f64,
    /// Next megahealth rot time in seconds (`-1` when idle).
    pub mega_rot_at: f64,
    /// Hostility expiry in seconds.
    pub hostile_until: f64,
    /// View angles.
    pub view_angles: Vec3,
    /// Water depth level.
    pub water_level: i32,
    /// Air supply expiry in seconds.
    pub air_finished: f64,
    /// Pending drown damage.
    pub drown_damage: f64,
    /// Next drown tick in seconds.
    pub drown_at: f64,
    /// Next hazard tick in seconds.
    pub hazard_at: f64,
    /// Automatic weapon switch policy.
    pub auto_switch: Q1AutoSwitch,
    /// Active powerup expiries in seconds.
    pub powerups: HashMap<Q1Powerup, f64>,
}

/// Component-wise binary32 addition (`vadd`).
#[must_use]
pub fn vadd(a: Vec3, b: Vec3) -> Vec3 {
    Vec3 {
        x: a.x + b.x,
        y: a.y + b.y,
        z: a.z + b.z,
    }
}

/// Component-wise binary32 subtraction (`vsub`).
#[must_use]
pub fn vsub(a: Vec3, b: Vec3) -> Vec3 {
    Vec3 {
        x: a.x - b.x,
        y: a.y - b.y,
        z: a.z - b.z,
    }
}

/// Binary32 scalar multiplication with a binary64 factor (`vscale`).
#[must_use]
pub fn vscale(a: Vec3, scale: f64) -> Vec3 {
    Vec3 {
        x: (f64::from(a.x) * scale) as f32,
        y: (f64::from(a.y) * scale) as f32,
        z: (f64::from(a.z) * scale) as f32,
    }
}

/// Binary32 dot product with donor rounding order (`dot`).
#[must_use]
pub fn dot(a: Vec3, b: Vec3) -> f32 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

/// Binary32 vector length (`length`).
#[must_use]
pub fn length(a: Vec3) -> f32 {
    f64::from(dot(a, a)).sqrt() as f32
}

/// Binary32 normalization; zero stays zero (`normalize`).
#[must_use]
pub fn normalize(a: Vec3) -> Vec3 {
    let magnitude = length(a);
    if magnitude == 0.0 {
        ZERO
    } else {
        vscale(a, 1.0 / f64::from(magnitude))
    }
}

/// Angle basis with binary64 trigonometry rounded per component
/// (`vectors`).
#[must_use]
pub fn vectors(angles: Vec3) -> Q1Basis {
    let yaw = f64::from(angles.y) * std::f64::consts::PI / 180.0;
    let pitch = f64::from(angles.x) * std::f64::consts::PI / 180.0;
    let roll = f64::from(angles.z) * std::f64::consts::PI / 180.0;
    let (sy, cy) = yaw.sin_cos();
    let (sp, cp) = pitch.sin_cos();
    let (sr, cr) = roll.sin_cos();
    Q1Basis {
        forward: Vec3 {
            x: (cp * cy) as f32,
            y: (cp * sy) as f32,
            z: (-sp) as f32,
        },
        right: Vec3 {
            x: (-sr * sp * cy + cr * sy) as f32,
            y: (-sr * sp * sy - cr * cy) as f32,
            z: (-sr * cp) as f32,
        },
        up: Vec3 {
            x: (cr * sp * cy + sr * sy) as f32,
            y: (cr * sp * sy - sr * cy) as f32,
            z: (cr * cp) as f32,
        },
    }
}

/// Yaw in degrees for a direction (`yawFor`).
#[must_use]
pub fn yaw_for(direction: Vec3) -> f64 {
    let yaw = f64::from(direction.y).atan2(f64::from(direction.x)) * 180.0 / std::f64::consts::PI;
    if yaw < 0.0 {
        yaw + 360.0
    } else {
        yaw
    }
}

/// Inclusive bounds overlap test (`overlaps`).
#[must_use]
pub fn overlaps(a: &Bounds, b: &Bounds) -> bool {
    a.min.x <= b.max.x
        && a.max.x >= b.min.x
        && a.min.y <= b.max.y
        && a.max.y >= b.min.y
        && a.min.z <= b.max.z
        && a.max.z >= b.min.z
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weapon_tables_match_donor_order() {
        assert_eq!(WEAPONS.len(), 8);
        assert_eq!(Q1_WEAPON_IDS.len(), 20);
        assert_eq!(Q1_WEAPON_IDS[8].as_str(), "hipnotic:laser");
        assert_eq!(Q1_WEAPON_IDS[19].as_str(), "ctf:grapple");
        assert_eq!(Q1_POWERUP_IDS.len(), 9);
        assert!(is_q1_base_weapon(Q1Weapon::Lightning));
        assert!(!is_q1_base_weapon(Q1Weapon::RoguePlasma));
        assert_eq!(weapon_item(Q1Weapon::Shotgun), "q1:weapon/shotgun");
        assert_eq!(q1_weapon_bit(Q1BaseWeapon::Axe), 4096);
        assert_eq!(Q1Weapon::parse("rogue:grapple"), Ok(Q1Weapon::RogueGrapple));
        assert!(Q1Weapon::parse("q2:railgun").is_err());
    }

    #[test]
    fn vector_math_rounds_like_fround() {
        let sum = vadd(Vec3 { x: 0.1, y: 0.2, z: 0.3 }, Vec3 { x: 0.1, y: 0.2, z: 0.3 });
        assert_eq!(
            sum,
            Vec3 {
                x: 0.1f32 + 0.1,
                y: 0.2f32 + 0.2,
                z: 0.3f32 + 0.3
            }
        );
        assert_eq!(
            dot(Vec3 { x: 1.0, y: 2.0, z: 3.0 }, Vec3 { x: 4.0, y: 5.0, z: 6.0 }),
            32.0
        );
        assert_eq!(normalize(ZERO), ZERO);
        assert_eq!(yaw_for(Vec3 { x: 1.0, y: 0.0, z: 0.0 }), 0.0);
        assert_eq!(
            yaw_for(Vec3 {
                x: -1.0,
                y: 0.0,
                z: 0.0
            }),
            180.0
        );
        assert!(overlaps(&PLAYER_BOUNDS, &PLAYER_BOUNDS));
        assert!(overlaps(&POINT, &PLAYER_BOUNDS));
        assert!(!overlaps(
            &POINT,
            &Bounds {
                min: Vec3 { x: 1.0, y: 1.0, z: 1.0 },
                max: Vec3 { x: 2.0, y: 2.0, z: 2.0 },
            }
        ));
    }

    #[test]
    fn basis_matches_cardinal_yaw() {
        let basis = vectors(Vec3 {
            x: 0.0,
            y: 90.0,
            z: 0.0,
        });
        assert!((f64::from(basis.forward.x) - 0.0).abs() < 1e-6);
        assert!((f64::from(basis.forward.y) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn walk_movetype_round_trips() {
        // Stock `MOVETYPE_WALK` (`defs.qc:248`), players only.
        assert_eq!(Q1MoveType::Walk.as_str(), "walk");
        assert_eq!(Q1MoveType::parse("walk"), Ok(Q1MoveType::Walk));
        assert_ne!(Q1MoveType::Walk, Q1MoveType::Step);
    }
}
