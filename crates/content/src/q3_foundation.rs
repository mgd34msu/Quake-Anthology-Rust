//! Quake III content foundation: animation config, lerp frames, player poses,
//! arsenal adapter, character lifecycle, event presentation, character assets,
//! movement hooks, render presentation, weapon behavior, and weapon poses.
//!
//! Donor provenance: `src/content/q3/foundation/` (`animation-config.ts`,
//! `animation.ts`, `arsenal.ts`, `assets.ts`, `character.ts`, `events.ts`,
//! `held-weapons.ts`, `movement-hooks.ts`, `player-pose.ts`, `presentation.ts`,
//! `weapon-behavior.ts`, `weapon-pose.ts`). `index.ts` is a content-free
//! re-export barrel and contributes no items.
//!
//! Anything the donors import from outside `qa-core`, `qa-platform`, and the
//! already-ported `qa-content` modules is mirrored locally in this module so it
//! stays self-contained: the `movement/q3` constants and the weapon/animation
//! operations, the `bg_lib` number scans, `COM_Parse`, the QVM math profile,
//! and the minimal movement/combat/body/scene contract shapes the foundation
//! touches. Each mirror cites its donor. Async donor loading is sync here.

use std::cell::RefCell;
use std::rc::Rc;

use qa_core::binary::BinaryError;
use qa_core::identity::{ActorId, OwnedActor, ProviderId};
use qa_core::math::{add3, dot3, length3, normalize3, scale3, sub3, vec3, Axis, Bounds, Vec3, Vec4};
use qa_core::numeric::qvm_float_to_int;
use qa_core::time::{FrameContext, SourceTime};

use crate::contract::{
    ArmorState, ContentId, InventoryEntry, ItemId, ModelTransform, PoweredProtectionState, ProjectileRole,
    RegularArmorState, ResolvedResourceReference,
};
use crate::md3::{interpolate_md3_tags, parse_md3, parse_skin, Md3Tag, SkinSurface};
use crate::md5::{sample_md5_pose, Md5AnimationFrame, Md5Joint, SkeletonJointPose};
use crate::mounts::OpenedResource;
use crate::q3anim::{PlayerFootsteps, PlayerGender};
use crate::q3scene::{joint_attachment_tag, to_scene_md3, SceneMd3};

/// Foundation failure (donor `TextParseError`, `RangeError`, `TypeError`,
/// `Error`, and `CommonError` throws).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Q3FoundationError {
    /// Text parse failure with source position (`TextParseError`).
    Parse {
        /// Source name.
        source: String,
        /// 1-based line.
        line: i32,
        /// 1-based column.
        column: i32,
        /// Message.
        message: String,
    },
    /// Out-of-range value (donor `RangeError`).
    Range(String),
    /// Wrong provider or state kind (donor `TypeError`).
    Type(String),
    /// Operation failure (donor `Error`).
    Failed(String),
    /// Dropped client/server state (donor `CommonError` with `"drop"`).
    Drop(String),
    /// Wrapped decoder failure.
    Binary(BinaryError),
}

impl std::fmt::Display for Q3FoundationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse {
                source,
                line,
                column,
                message,
            } => write!(f, "{source}:{line}:{column}: {message}"),
            Self::Range(message) | Self::Type(message) | Self::Failed(message) => {
                write!(f, "{message}")
            }
            Self::Drop(message) => write!(f, "drop: {message}"),
            Self::Binary(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for Q3FoundationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Binary(error) => Some(error),
            _ => None,
        }
    }
}

impl From<BinaryError> for Q3FoundationError {
    fn from(error: BinaryError) -> Self {
        Self::Binary(error)
    }
}

fn range(message: impl Into<String>) -> Q3FoundationError {
    Q3FoundationError::Range(message.into())
}

fn failed(message: impl Into<String>) -> Q3FoundationError {
    Q3FoundationError::Failed(message.into())
}

fn type_error(message: impl Into<String>) -> Q3FoundationError {
    Q3FoundationError::Type(message.into())
}

// ---------------------------------------------------------------------------
// Q3 source constants (mirror of `src/movement/q3/constants.ts`).
// Plain integers in the donors; associated constants preserve the arithmetic
// (`1 << weapon`, `event - EV_DEATH1 + 1`, `animation & ~128`) exactly.
// ---------------------------------------------------------------------------

/// Q3 product family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q3Product {
    /// Base Quake III Arena.
    BaseQ3,
    /// Team Arena mission pack.
    MissionPack,
}

impl Q3Product {
    /// Exclusive weapon upper bound (11 for base, 14 for mission pack).
    #[must_use]
    pub fn weapon_limit(self) -> i32 {
        match self {
            Q3Product::BaseQ3 => 11,
            Q3Product::MissionPack => 14,
        }
    }
}

/// Movement types (`MoveType`).
pub struct Q3MoveType;

impl Q3MoveType {
    /// Normal movement.
    pub const NORMAL: i32 = 0;
    /// Noclip.
    pub const NOCLIP: i32 = 1;
    /// Spectator.
    pub const SPECTATOR: i32 = 2;
    /// Dead.
    pub const DEAD: i32 = 3;
    /// Frozen.
    pub const FREEZE: i32 = 4;
    /// Intermission.
    pub const INTERMISSION: i32 = 5;
    /// Single-player intermission.
    pub const SPINTERMISSION: i32 = 6;
}

/// Weapon phases (`WeaponState`).
pub struct Q3WeaponPhase;

impl Q3WeaponPhase {
    /// Ready.
    pub const READY: i32 = 0;
    /// Raising.
    pub const RAISING: i32 = 1;
    /// Dropping.
    pub const DROPPING: i32 = 2;
    /// Firing.
    pub const FIRING: i32 = 3;
}

/// Powerups (`Powerup`).
pub struct Q3Powerup;

impl Q3Powerup {
    /// None.
    pub const NONE: i32 = 0;
    /// Quad damage.
    pub const QUAD: i32 = 1;
    /// Battle suit.
    pub const BATTLESUIT: i32 = 2;
    /// Haste.
    pub const HASTE: i32 = 3;
    /// Invisibility.
    pub const INVIS: i32 = 4;
    /// Regeneration.
    pub const REGEN: i32 = 5;
    /// Flight.
    pub const FLIGHT: i32 = 6;
    /// Red flag.
    pub const REDFLAG: i32 = 7;
    /// Blue flag.
    pub const BLUEFLAG: i32 = 8;
    /// Neutral flag.
    pub const NEUTRALFLAG: i32 = 9;
    /// Scout.
    pub const SCOUT: i32 = 10;
    /// Guard.
    pub const GUARD: i32 = 11;
    /// Doubler.
    pub const DOUBLER: i32 = 12;
    /// Ammo regeneration.
    pub const AMMOREGEN: i32 = 13;
    /// Invulnerability.
    pub const INVULNERABILITY: i32 = 14;
    /// Powerup count.
    pub const NUM_POWERUPS: i32 = 15;
}

/// Holdable items (`Holdable`).
pub struct Q3Holdable;

impl Q3Holdable {
    /// None.
    pub const NONE: i32 = 0;
    /// Teleporter.
    pub const TELEPORTER: i32 = 1;
    /// Medkit.
    pub const MEDKIT: i32 = 2;
    /// Kamikaze.
    pub const KAMIKAZE: i32 = 3;
    /// Portal.
    pub const PORTAL: i32 = 4;
    /// Invulnerability.
    pub const INVULNERABILITY: i32 = 5;
    /// Holdable count.
    pub const NUM_HOLDABLE: i32 = 6;
}

/// Weapons (`Weapon`).
pub struct Q3Weapon;

impl Q3Weapon {
    /// No weapon.
    pub const NONE: i32 = 0;
    /// Gauntlet.
    pub const GAUNTLET: i32 = 1;
    /// Machinegun.
    pub const MACHINEGUN: i32 = 2;
    /// Shotgun.
    pub const SHOTGUN: i32 = 3;
    /// Grenade launcher.
    pub const GRENADE_LAUNCHER: i32 = 4;
    /// Rocket launcher.
    pub const ROCKET_LAUNCHER: i32 = 5;
    /// Lightning gun.
    pub const LIGHTNING: i32 = 6;
    /// Railgun.
    pub const RAILGUN: i32 = 7;
    /// Plasmagun.
    pub const PLASMAGUN: i32 = 8;
    /// BFG.
    pub const BFG: i32 = 9;
    /// Grappling hook.
    pub const GRAPPLING_HOOK: i32 = 10;
    /// Nailgun.
    pub const NAILGUN: i32 = 11;
    /// Proximity launcher.
    pub const PROX_LAUNCHER: i32 = 12;
    /// Chaingun.
    pub const CHAINGUN: i32 = 13;
}

/// Entity events (`EntityEvent`).
pub struct Q3EntityEvent;

impl Q3EntityEvent {
    /// None.
    pub const NONE: i32 = 0;
    /// Footstep.
    pub const FOOTSTEP: i32 = 1;
    /// Metal footstep.
    pub const FOOTSTEP_METAL: i32 = 2;
    /// Splash footstep.
    pub const FOOTSPLASH: i32 = 3;
    /// Wade footstep.
    pub const FOOTWADE: i32 = 4;
    /// Swim.
    pub const SWIM: i32 = 5;
    /// Step 4.
    pub const STEP_4: i32 = 6;
    /// Step 8.
    pub const STEP_8: i32 = 7;
    /// Step 12.
    pub const STEP_12: i32 = 8;
    /// Step 16.
    pub const STEP_16: i32 = 9;
    /// Short fall.
    pub const FALL_SHORT: i32 = 10;
    /// Medium fall.
    pub const FALL_MEDIUM: i32 = 11;
    /// Far fall.
    pub const FALL_FAR: i32 = 12;
    /// Jump pad.
    pub const JUMP_PAD: i32 = 13;
    /// Jump.
    pub const JUMP: i32 = 14;
    /// Water touch.
    pub const WATER_TOUCH: i32 = 15;
    /// Water leave.
    pub const WATER_LEAVE: i32 = 16;
    /// Water under.
    pub const WATER_UNDER: i32 = 17;
    /// Water clear.
    pub const WATER_CLEAR: i32 = 18;
    /// Item pickup.
    pub const ITEM_PICKUP: i32 = 19;
    /// Global item pickup.
    pub const GLOBAL_ITEM_PICKUP: i32 = 20;
    /// No ammo.
    pub const NOAMMO: i32 = 21;
    /// Change weapon.
    pub const CHANGE_WEAPON: i32 = 22;
    /// Fire weapon.
    pub const FIRE_WEAPON: i32 = 23;
    /// Use item 0.
    pub const USE_ITEM0: i32 = 24;
    /// Item respawn.
    pub const ITEM_RESPAWN: i32 = 40;
    /// Item pop.
    pub const ITEM_POP: i32 = 41;
    /// Player teleport in.
    pub const PLAYER_TELEPORT_IN: i32 = 42;
    /// Player teleport out.
    pub const PLAYER_TELEPORT_OUT: i32 = 43;
    /// Grenade bounce.
    pub const GRENADE_BOUNCE: i32 = 44;
    /// General sound.
    pub const GENERAL_SOUND: i32 = 45;
    /// Global sound.
    pub const GLOBAL_SOUND: i32 = 46;
    /// Global team sound.
    pub const GLOBAL_TEAM_SOUND: i32 = 47;
    /// Bullet hit flesh.
    pub const BULLET_HIT_FLESH: i32 = 48;
    /// Bullet hit wall.
    pub const BULLET_HIT_WALL: i32 = 49;
    /// Missile hit.
    pub const MISSILE_HIT: i32 = 50;
    /// Missile miss.
    pub const MISSILE_MISS: i32 = 51;
    /// Missile miss metal.
    pub const MISSILE_MISS_METAL: i32 = 52;
    /// Rail trail.
    pub const RAILTRAIL: i32 = 53;
    /// Shotgun.
    pub const SHOTGUN: i32 = 54;
    /// Bullet.
    pub const BULLET: i32 = 55;
    /// Pain.
    pub const PAIN: i32 = 56;
    /// Death 1.
    pub const DEATH1: i32 = 57;
    /// Death 2.
    pub const DEATH2: i32 = 58;
    /// Death 3.
    pub const DEATH3: i32 = 59;
    /// Obituary.
    pub const OBITUARY: i32 = 60;
    /// Powerup quad.
    pub const POWERUP_QUAD: i32 = 61;
    /// Powerup battlesuit.
    pub const POWERUP_BATTLESUIT: i32 = 62;
    /// Powerup regen.
    pub const POWERUP_REGEN: i32 = 63;
    /// Gib player.
    pub const GIB_PLAYER: i32 = 64;
    /// Score plum.
    pub const SCOREPLUM: i32 = 65;
    /// Proximity mine stick.
    pub const PROXIMITY_MINE_STICK: i32 = 66;
    /// Proximity mine trigger.
    pub const PROXIMITY_MINE_TRIGGER: i32 = 67;
    /// Kamikaze.
    pub const KAMIKAZE: i32 = 68;
    /// Obelisk explode.
    pub const OBELISKEXPLODE: i32 = 69;
    /// Obelisk pain.
    pub const OBELISKPAIN: i32 = 70;
    /// Invulnerability impact.
    pub const INVUL_IMPACT: i32 = 71;
    /// Juiced.
    pub const JUICED: i32 = 72;
    /// Lightning bolt.
    pub const LIGHTNINGBOLT: i32 = 73;
    /// Debug line.
    pub const DEBUG_LINE: i32 = 74;
    /// Stop looping sound.
    pub const STOPLOOPINGSOUND: i32 = 75;
    /// Taunt.
    pub const TAUNT: i32 = 76;
    /// Taunt yes.
    pub const TAUNT_YES: i32 = 77;
    /// Taunt no.
    pub const TAUNT_NO: i32 = 78;
    /// Taunt follow me.
    pub const TAUNT_FOLLOWME: i32 = 79;
    /// Taunt get flag.
    pub const TAUNT_GETFLAG: i32 = 80;
    /// Taunt guard base.
    pub const TAUNT_GUARDBASE: i32 = 81;
    /// Taunt patrol.
    pub const TAUNT_PATROL: i32 = 82;
}

/// Movement flags (`MoveFlags`).
pub struct Q3MoveFlags;

impl Q3MoveFlags {
    /// Ducked.
    pub const DUCKED: i32 = 1;
    /// Jump held.
    pub const JUMP_HELD: i32 = 2;
    /// Backwards jump.
    pub const BACKWARDS_JUMP: i32 = 8;
    /// Backwards run.
    pub const BACKWARDS_RUN: i32 = 16;
    /// Landing timer.
    pub const TIME_LAND: i32 = 32;
    /// Knockback timer.
    pub const TIME_KNOCKBACK: i32 = 64;
    /// Water jump timer.
    pub const TIME_WATERJUMP: i32 = 256;
    /// Just respawned.
    pub const RESPAWNED: i32 = 512;
    /// Use-item held.
    pub const USE_ITEM_HELD: i32 = 1024;
    /// Grapple pull.
    pub const GRAPPLE_PULL: i32 = 2048;
    /// Follow.
    pub const FOLLOW: i32 = 4096;
    /// Scoreboard.
    pub const SCOREBOARD: i32 = 8192;
    /// Invulnerability expand.
    pub const INVULEXPAND: i32 = 16384;
}

/// Command buttons (`CommandButtons`).
pub struct Q3CommandButtons;

impl Q3CommandButtons {
    /// Attack.
    pub const ATTACK: i32 = 1;
    /// Talk.
    pub const TALK: i32 = 2;
    /// Use holdable.
    pub const USE_HOLDABLE: i32 = 4;
    /// Gesture.
    pub const GESTURE: i32 = 8;
    /// Walking.
    pub const WALKING: i32 = 16;
    /// Affirmative.
    pub const AFFIRMATIVE: i32 = 32;
    /// Negative.
    pub const NEGATIVE: i32 = 64;
    /// Get flag.
    pub const GETFLAG: i32 = 128;
    /// Guard base.
    pub const GUARDBASE: i32 = 256;
    /// Patrol.
    pub const PATROL: i32 = 512;
    /// Follow me.
    pub const FOLLOWME: i32 = 1024;
    /// Any.
    pub const ANY: i32 = 2048;
}

/// Player animations (`PlayerAnimation`).
pub struct Q3PlayerAnimation;

impl Q3PlayerAnimation {
    /// Both death 1.
    pub const BOTH_DEATH1: i32 = 0;
    /// Both dead 1.
    pub const BOTH_DEAD1: i32 = 1;
    /// Both death 2.
    pub const BOTH_DEATH2: i32 = 2;
    /// Both dead 2.
    pub const BOTH_DEAD2: i32 = 3;
    /// Both death 3.
    pub const BOTH_DEATH3: i32 = 4;
    /// Both dead 3.
    pub const BOTH_DEAD3: i32 = 5;
    /// Torso gesture.
    pub const TORSO_GESTURE: i32 = 6;
    /// Torso attack.
    pub const TORSO_ATTACK: i32 = 7;
    /// Torso attack 2.
    pub const TORSO_ATTACK2: i32 = 8;
    /// Torso drop.
    pub const TORSO_DROP: i32 = 9;
    /// Torso raise.
    pub const TORSO_RAISE: i32 = 10;
    /// Torso stand.
    pub const TORSO_STAND: i32 = 11;
    /// Torso stand 2.
    pub const TORSO_STAND2: i32 = 12;
    /// Legs crouch walk.
    pub const LEGS_WALKCR: i32 = 13;
    /// Legs walk.
    pub const LEGS_WALK: i32 = 14;
    /// Legs run.
    pub const LEGS_RUN: i32 = 15;
    /// Legs back.
    pub const LEGS_BACK: i32 = 16;
    /// Legs swim.
    pub const LEGS_SWIM: i32 = 17;
    /// Legs jump.
    pub const LEGS_JUMP: i32 = 18;
    /// Legs land.
    pub const LEGS_LAND: i32 = 19;
    /// Legs jump back.
    pub const LEGS_JUMPB: i32 = 20;
    /// Legs land back.
    pub const LEGS_LANDB: i32 = 21;
    /// Legs idle.
    pub const LEGS_IDLE: i32 = 22;
    /// Legs idle crouch.
    pub const LEGS_IDLECR: i32 = 23;
    /// Legs turn.
    pub const LEGS_TURN: i32 = 24;
    /// Torso get flag.
    pub const TORSO_GETFLAG: i32 = 25;
    /// Torso guard base.
    pub const TORSO_GUARDBASE: i32 = 26;
    /// Torso patrol.
    pub const TORSO_PATROL: i32 = 27;
    /// Torso follow me.
    pub const TORSO_FOLLOWME: i32 = 28;
    /// Torso affirmative.
    pub const TORSO_AFFIRMATIVE: i32 = 29;
    /// Torso negative.
    pub const TORSO_NEGATIVE: i32 = 30;
    /// Legs back crouch (derived).
    pub const LEGS_BACKCR: i32 = 32;
    /// Legs back walk (derived).
    pub const LEGS_BACKWALK: i32 = 33;
    /// Flag run (derived).
    pub const FLAG_RUN: i32 = 34;
    /// Flag stand (derived).
    pub const FLAG_STAND: i32 = 35;
    /// Flag stand to run (derived).
    pub const FLAG_STAND2RUN: i32 = 36;
}

// ---------------------------------------------------------------------------
// bg_lib number scans (mirror of `src/core/game-numeric.ts` scalar paths).
// ---------------------------------------------------------------------------

struct NumberInput<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> NumberInput<'a> {
    fn new(text: &'a str) -> Result<Self, Q3FoundationError> {
        if text.chars().any(|c| c as u32 > 255) {
            return Err(range("Game numbers require byte characters"));
        }
        Ok(Self {
            bytes: text.as_bytes(),
            offset: 0,
        })
    }

    fn byte(&self) -> Result<i32, Q3FoundationError> {
        if self.offset > self.bytes.len() {
            return Err(range("Game number scan reads beyond its backing string"));
        }
        if self.offset == self.bytes.len() {
            return Ok(0);
        }
        let byte = self.bytes[self.offset];
        Ok(if byte < 128 {
            i32::from(byte)
        } else {
            i32::from(byte) - 256
        })
    }

    fn take(&mut self) -> Result<i32, Q3FoundationError> {
        let byte = self.byte()?;
        self.offset += 1;
        Ok(byte)
    }

    fn skip_whitespace(&mut self) -> Result<(), Q3FoundationError> {
        while self.byte()? <= 32 && self.byte()? != 0 {
            self.offset += 1;
        }
        Ok(())
    }

    fn sign(&mut self) -> Result<i32, Q3FoundationError> {
        let byte = self.byte()?;
        if byte != 43 && byte != 45 {
            return Ok(1);
        }
        self.offset += 1;
        Ok(if byte == 45 { -1 } else { 1 })
    }
}

fn read_game_float(input: &mut NumberInput<'_>) -> Result<f32, Q3FoundationError> {
    input.skip_whitespace()?;
    if input.byte()? == 0 {
        return Ok(0.0);
    }
    let sign = input.sign()?;
    let mut value = 0.0f32;
    let mut character = input.byte()?;
    if input.byte()? != 46 {
        loop {
            character = input.take()?;
            if !(48..=57).contains(&character) {
                break;
            }
            value = value * 10.0 + (character - 48) as f32;
        }
    } else {
        input.offset += 1;
    }
    if character == 46 {
        let mut fraction = 0.1f32;
        loop {
            character = input.take()?;
            if !(48..=57).contains(&character) {
                break;
            }
            value += (character - 48) as f32 * fraction;
            fraction *= 0.1f32;
        }
    }
    Ok(value * sign as f32)
}

/// `bg_lib` atof: decimal prefix only, binary32 operations (`gameAtof`).
pub fn game_atof(text: &str) -> Result<f32, Q3FoundationError> {
    read_game_float(&mut NumberInput::new(text)?)
}

/// `bg_lib` atoi: wraps every integer digit operation (`gameAtoi`).
pub fn game_atoi(text: &str) -> Result<i32, Q3FoundationError> {
    let mut input = NumberInput::new(text)?;
    input.skip_whitespace()?;
    if input.byte()? == 0 {
        return Ok(0);
    }
    let sign = input.sign()?;
    let mut value = 0i32;
    loop {
        let character = input.take()?;
        if !(48..=57).contains(&character) {
            break;
        }
        value = value.wrapping_mul(10).wrapping_add(character - 48);
    }
    Ok(value.wrapping_mul(sign))
}

// ---------------------------------------------------------------------------
// COM_Parse (mirror of the `CommonParseCursor`/`CommonParseState` paths in
// `src/core/common-parse.ts` used by the animation config parser).
// ---------------------------------------------------------------------------

const COM_TOKEN_MAX: usize = 1024;

/// Byte cursor over a Latin-1 source string (`CommonParseCursor`).
#[derive(Debug, Clone)]
pub struct CommonParseCursor {
    bytes: Vec<u8>,
    terminator: usize,
    offset: Option<usize>,
}

impl CommonParseCursor {
    /// Build a cursor; non-Latin-1 sources are rejected.
    pub fn new(source: &str) -> Result<Self, Q3FoundationError> {
        if source.chars().any(|c| c as u32 > 255) {
            return Err(range("COM_Parse source is not a Latin-1 byte string"));
        }
        let bytes = source.as_bytes().to_vec();
        let terminator = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
        Ok(Self {
            bytes,
            terminator,
            offset: Some(0),
        })
    }

    /// Current offset; `None` once exhausted.
    #[must_use]
    pub fn offset(&self) -> Option<usize> {
        self.offset
    }

    /// Reposition the cursor within the C byte string.
    pub fn set_offset(&mut self, value: Option<usize>) -> Result<(), Q3FoundationError> {
        if let Some(offset) = value {
            if offset > self.terminator {
                return Err(range("COM_Parse cursor is outside its C byte string"));
            }
        }
        self.offset = value;
        Ok(())
    }

    fn signed_byte(&self, offset: usize) -> i32 {
        if offset >= self.bytes.len() {
            return 0;
        }
        let byte = self.bytes[offset];
        if byte >= 128 {
            i32::from(byte) - 256
        } else {
            i32::from(byte)
        }
    }

    fn char_at(&self, offset: usize) -> char {
        self.bytes.get(offset).map_or('\0', |b| *b as char)
    }
}

/// Tokenizer state with the shared token and line (`CommonParseState`).
#[derive(Debug, Clone, Default)]
pub struct CommonParseState {
    token: String,
    line: i32,
}

impl CommonParseState {
    /// Fresh tokenizer state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Current 0-based line.
    #[must_use]
    pub fn line(&self) -> i32 {
        self.line
    }

    /// Current shared token.
    #[must_use]
    pub fn token(&self) -> &str {
        &self.token
    }

    fn push_char(&mut self, c: char) {
        if self.token.chars().count() < COM_TOKEN_MAX {
            self.token.push(c);
        }
    }

    /// Parse one token (`COM_Parse` with line breaks allowed).
    pub fn parse(&mut self, cursor: &mut CommonParseCursor) -> Result<String, Q3FoundationError> {
        let Some(mut data) = cursor.offset else {
            self.token.clear();
            return Ok(String::new());
        };
        self.token.clear();
        let mut has_new_lines = false;
        let mut c: i32;
        loop {
            loop {
                c = cursor.signed_byte(data);
                if c > 32 {
                    break;
                }
                if c == 0 {
                    cursor.offset = None;
                    return Ok(String::new());
                }
                if c == 10 {
                    self.line = self.line.wrapping_add(1);
                    has_new_lines = true;
                }
                data += 1;
            }
            let _ = has_new_lines;
            if c == 47 && cursor.signed_byte(data + 1) == 47 {
                data += 2;
                while {
                    c = cursor.signed_byte(data);
                    c != 0 && c != 10
                } {
                    data += 1;
                }
            } else if c == 47 && cursor.signed_byte(data + 1) == 42 {
                data += 2;
                while cursor.signed_byte(data) != 0
                    && (cursor.signed_byte(data) != 42 || cursor.signed_byte(data + 1) != 47)
                {
                    data += 1;
                }
                if cursor.signed_byte(data) != 0 {
                    data += 2;
                }
            } else {
                break;
            }
        }
        if c == 34 {
            data += 1;
            loop {
                c = cursor.signed_byte(data);
                data += 1;
                if c == 34 || c == 0 {
                    if self.token.chars().count() == COM_TOKEN_MAX {
                        return Err(range("COM_Parse quoted token terminator exceeds 1024-byte storage"));
                    }
                    cursor.offset = if c == 0 { None } else { Some(data) };
                    return Ok(std::mem::take(&mut self.token));
                }
                self.push_char(cursor.char_at(data - 1));
            }
        }
        loop {
            self.push_char(cursor.char_at(data));
            data += 1;
            c = cursor.signed_byte(data);
            if c == 10 {
                self.line = self.line.wrapping_add(1);
            }
            if c <= 32 {
                break;
            }
        }
        if self.token.chars().count() == COM_TOKEN_MAX {
            self.token.clear();
        }
        cursor.offset = Some(data);
        Ok(std::mem::take(&mut self.token))
    }
}

// ---------------------------------------------------------------------------
// animation-config.ts: CG_ParseAnimationFile.
// ---------------------------------------------------------------------------

const MAX_TEXT_BYTES: usize = 19_998;
const SOURCE_ANIMATION_COUNT: usize = 31;
const TOTAL_ANIMATION_COUNT: usize = 37;
const MAX_ANIMATIONS_SENTINEL: usize = 31;

/// One parsed animation row (`Animation`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Animation {
    /// First frame.
    pub first_frame: i32,
    /// Frame count.
    pub num_frames: i32,
    /// Loop frames.
    pub loop_frames: i32,
    /// Milliseconds per frame.
    pub frame_lerp: i32,
    /// Initial lerp.
    pub initial_lerp: i32,
    /// Reversed playback.
    pub reversed: bool,
    /// Flip-flop playback.
    pub flipflop: bool,
}

impl Default for Animation {
    fn default() -> Self {
        Self {
            first_frame: 0,
            num_frames: 0,
            loop_frames: 0,
            frame_lerp: 0,
            initial_lerp: 0,
            reversed: false,
            flipflop: false,
        }
    }
}

/// Retained client cells a parse writes into (`PlayerAnimationTarget`).
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerAnimationTarget {
    /// Footsteps.
    pub footsteps: PlayerFootsteps,
    /// Head offset.
    pub head_offset: Vec3,
    /// Gender.
    pub gender: PlayerGender,
    /// Fixed legs.
    pub fixed_legs: bool,
    /// Fixed torso.
    pub fixed_torso: bool,
    /// Animation cells.
    pub animations: [Animation; TOTAL_ANIMATION_COUNT],
}

impl Default for PlayerAnimationTarget {
    fn default() -> Self {
        Self {
            footsteps: PlayerFootsteps::Normal,
            head_offset: vec3(0.0, 0.0, 0.0),
            gender: PlayerGender::Male,
            fixed_legs: false,
            fixed_torso: false,
            animations: [Animation::default(); TOTAL_ANIMATION_COUNT],
        }
    }
}

/// Diagnostic warning with source position (`AnimationWarning`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnimationWarning {
    /// 1-based line.
    pub line: i32,
    /// 1-based column.
    pub column: i32,
    /// Message.
    pub message: String,
}

/// Parsed player animation config (`PlayerAnimationConfig`).
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerAnimationConfig {
    /// Footsteps.
    pub footsteps: PlayerFootsteps,
    /// Head offset.
    pub head_offset: Vec3,
    /// Gender.
    pub gender: PlayerGender,
    /// Fixed legs.
    pub fixed_legs: bool,
    /// Fixed torso.
    pub fixed_torso: bool,
    /// Indexed by player animation; the `MAX_ANIMATIONS` sentinel is `None`.
    pub animations: [Option<Animation>; TOTAL_ANIMATION_COUNT],
    /// Diagnostics.
    pub warnings: Vec<AnimationWarning>,
}

fn animation_cell(target: &mut PlayerAnimationTarget, index: usize) -> Result<&mut Animation, Q3FoundationError> {
    target
        .animations
        .get_mut(index)
        .ok_or_else(|| range(format!("Missing client animation cell {index}")))
}

fn cg_print(format: &str, args: &[&str], print: Option<&mut (dyn FnMut(&str) + '_)>) -> Result<String, Q3FoundationError> {
    let mut position = 0;
    let mut message = String::new();
    let mut rest = format;
    while let Some(hit) = rest.find("%s") {
        message.push_str(&rest[..hit]);
        message.push_str(args.get(position).copied().unwrap_or(""));
        position += 1;
        rest = &rest[hit + 2..];
    }
    message.push_str(rest);
    if message.chars().count() >= 1024 {
        return Err(range("CG_Printf exceeds its 1024-byte source buffer"));
    }
    if let Some(print) = print {
        print(&message);
    }
    Ok(message)
}

/// Parse into retained client cells (`parsePlayerAnimationConfig` with context).
pub fn parse_player_animation_config_into(
    target: &mut PlayerAnimationTarget,
    parser: &mut CommonParseState,
    text: &str,
    source: &str,
    mut print: Option<&mut (dyn FnMut(&str) + '_)>,
) -> Result<PlayerAnimationConfig, Q3FoundationError> {
    let mut warnings: Vec<AnimationWarning> = Vec::new();
    if text.is_empty() {
        return Err(Q3FoundationError::Parse {
            source: source.to_string(),
            line: 1,
            column: 1,
            message: "empty animation file".to_string(),
        });
    }
    if text.len() > MAX_TEXT_BYTES {
        let message = cg_print("File %s too long\n", &[source], print.as_deref_mut())?;
        return Err(Q3FoundationError::Parse {
            source: source.to_string(),
            line: 1,
            column: 1,
            message,
        });
    }
    let mut cursor = CommonParseCursor::new(text)?;
    target.footsteps = PlayerFootsteps::Normal;
    target.head_offset = vec3(0.0, 0.0, 0.0);
    target.gender = PlayerGender::Male;
    target.fixed_legs = false;
    target.fixed_torso = false;

    loop {
        let previous = cursor.offset();
        let token = parser.parse(&mut cursor)?;
        let directive = token.to_lowercase();
        if directive == "footsteps" {
            let value = parser.parse(&mut cursor)?;
            match value.to_lowercase().as_str() {
                "default" | "normal" => target.footsteps = PlayerFootsteps::Normal,
                "boot" => target.footsteps = PlayerFootsteps::Boot,
                "flesh" => target.footsteps = PlayerFootsteps::Flesh,
                "mech" => target.footsteps = PlayerFootsteps::Mech,
                "energy" => target.footsteps = PlayerFootsteps::Energy,
                _ => {
                    let message = cg_print(
                        "Bad footsteps parm in %s: %s\n",
                        &[source, &value],
                        print.as_deref_mut(),
                    )?;
                    warnings.push(AnimationWarning {
                        line: parser.line().wrapping_add(1),
                        column: 1,
                        message,
                    });
                }
            }
        } else if directive == "headoffset" {
            let x = game_atof(&parser.parse(&mut cursor)?)?;
            let y = game_atof(&parser.parse(&mut cursor)?)?;
            let z = game_atof(&parser.parse(&mut cursor)?)?;
            target.head_offset = vec3(x, y, z);
        } else if directive == "sex" {
            let first = parser
                .parse(&mut cursor)?
                .chars()
                .next()
                .map(|c| c.to_lowercase().next().unwrap_or(c))
                .unwrap_or('\0');
            target.gender = if first == 'f' {
                PlayerGender::Female
            } else if first == 'n' {
                PlayerGender::Neuter
            } else {
                PlayerGender::Male
            };
        } else if directive == "fixedlegs" {
            target.fixed_legs = true;
        } else if directive == "fixedtorso" {
            target.fixed_torso = true;
        } else {
            let first = token.chars().next();
            if matches!(first, Some('0'..='9')) {
                cursor.set_offset(previous)?;
                break;
            }
            let message = cg_print("unknown token '%s' is %s\n", &[&token, source], print.as_deref_mut())?;
            warnings.push(AnimationWarning {
                line: parser.line().wrapping_add(1),
                column: 1,
                message,
            });
            if previous == cursor.offset() {
                return Err(range("CG animation prelude reached the source nonprogress cycle"));
            }
        }
    }

    let mut skip = 0i32;
    let mut index = 0usize;
    while index < SOURCE_ANIMATION_COUNT {
        let first_token = parser.parse(&mut cursor)?;
        if first_token.is_empty() {
            if (Q3PlayerAnimation::TORSO_GETFLAG as usize..=Q3PlayerAnimation::TORSO_NEGATIVE as usize).contains(&index)
            {
                let gesture = target.animations[Q3PlayerAnimation::TORSO_GESTURE as usize];
                let animation = animation_cell(target, index)?;
                animation.first_frame = gesture.first_frame;
                animation.frame_lerp = gesture.frame_lerp;
                animation.initial_lerp = gesture.initial_lerp;
                animation.loop_frames = gesture.loop_frames;
                animation.num_frames = gesture.num_frames;
                animation.reversed = false;
                animation.flipflop = false;
                index += 1;
                continue;
            }
            break;
        }
        let first_frame = game_atoi(&first_token)?;
        animation_cell(target, index)?.first_frame = first_frame;
        if index == Q3PlayerAnimation::LEGS_WALKCR as usize {
            let gesture = target.animations[Q3PlayerAnimation::TORSO_GESTURE as usize].first_frame;
            skip = target.animations[index].first_frame.wrapping_sub(gesture);
        }
        if (Q3PlayerAnimation::LEGS_WALKCR as usize..Q3PlayerAnimation::TORSO_GETFLAG as usize).contains(&index) {
            let animation = animation_cell(target, index)?;
            animation.first_frame = animation.first_frame.wrapping_sub(skip);
        }
        let count_token = parser.parse(&mut cursor)?;
        if count_token.is_empty() {
            break;
        }
        let num_frames = game_atoi(&count_token)?;
        let animation = animation_cell(target, index)?;
        animation.num_frames = num_frames;
        animation.reversed = false;
        animation.flipflop = false;
        if animation.num_frames < 0 {
            animation.num_frames = animation.num_frames.wrapping_neg();
            animation.reversed = true;
        }
        let loop_token = parser.parse(&mut cursor)?;
        if loop_token.is_empty() {
            break;
        }
        animation_cell(target, index)?.loop_frames = game_atoi(&loop_token)?;
        let fps_token = parser.parse(&mut cursor)?;
        if fps_token.is_empty() {
            break;
        }
        let mut fps = game_atof(&fps_token)?;
        if fps == 0.0 {
            fps = 1.0;
        }
        let lerp = qvm_float_to_int((1000.0f64 / f64::from(fps)) as f32);
        let animation = animation_cell(target, index)?;
        animation.frame_lerp = lerp;
        animation.initial_lerp = lerp;
        index += 1;
    }
    if index != SOURCE_ANIMATION_COUNT {
        let message = cg_print("Error parsing animation file: %s", &[source], print.as_deref_mut())?;
        return Err(Q3FoundationError::Parse {
            source: source.to_string(),
            line: parser.line().wrapping_add(1),
            column: 1,
            message,
        });
    }

    let walk_cr = target.animations[Q3PlayerAnimation::LEGS_WALKCR as usize];
    let back_cr = animation_cell(target, Q3PlayerAnimation::LEGS_BACKCR as usize)?;
    *back_cr = walk_cr;
    back_cr.reversed = true;
    let walk = target.animations[Q3PlayerAnimation::LEGS_WALK as usize];
    let back_walk = animation_cell(target, Q3PlayerAnimation::LEGS_BACKWALK as usize)?;
    *back_walk = walk;
    back_walk.reversed = true;
    let flag_run = animation_cell(target, Q3PlayerAnimation::FLAG_RUN as usize)?;
    flag_run.first_frame = 0;
    flag_run.num_frames = 16;
    flag_run.loop_frames = 16;
    flag_run.frame_lerp = 66;
    flag_run.initial_lerp = 66;
    flag_run.reversed = false;
    let flag_stand = animation_cell(target, Q3PlayerAnimation::FLAG_STAND as usize)?;
    flag_stand.first_frame = 16;
    flag_stand.num_frames = 5;
    flag_stand.loop_frames = 0;
    flag_stand.frame_lerp = 50;
    flag_stand.initial_lerp = 50;
    flag_stand.reversed = false;
    let flag_run2 = animation_cell(target, Q3PlayerAnimation::FLAG_STAND2RUN as usize)?;
    flag_run2.first_frame = 16;
    flag_run2.num_frames = 5;
    flag_run2.loop_frames = 1;
    flag_run2.frame_lerp = 66;
    flag_run2.initial_lerp = 66;
    flag_run2.reversed = true;

    let mut animations: [Option<Animation>; TOTAL_ANIMATION_COUNT] = [None; TOTAL_ANIMATION_COUNT];
    for (slot, cell) in animations.iter_mut().enumerate() {
        if slot != MAX_ANIMATIONS_SENTINEL {
            *cell = Some(target.animations[slot]);
        }
    }
    Ok(PlayerAnimationConfig {
        footsteps: target.footsteps,
        head_offset: target.head_offset,
        gender: target.gender,
        fixed_legs: target.fixed_legs,
        fixed_torso: target.fixed_torso,
        animations,
        warnings,
    })
}

/// Parse an owned config snapshot (`parsePlayerAnimationConfig` standalone).
pub fn parse_player_animation_config(text: &str, source: &str) -> Result<PlayerAnimationConfig, Q3FoundationError> {
    let mut target = PlayerAnimationTarget::default();
    let mut parser = CommonParseState::new();
    parse_player_animation_config_into(&mut target, &mut parser, text, source, None)
}

// ---------------------------------------------------------------------------
// animation.ts: CG_SetLerpFrameAnimation, CG_RunLerpFrame, CG_ClearLerpFrame.
// ---------------------------------------------------------------------------

/// Animation toggle bit.
pub const ANIMATION_TOGGLE_BIT: i32 = 128;
const MAX_TOTAL_ANIMATIONS: usize = TOTAL_ANIMATION_COUNT;

/// Interpolated frame state (`LerpFrame`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LerpFrame {
    /// Previous frame.
    pub old_frame: i32,
    /// Previous frame time.
    pub old_frame_time: i32,
    /// Current frame.
    pub frame: i32,
    /// Current frame time.
    pub frame_time: i32,
    /// Blend factor.
    pub back_lerp: f32,
    /// Animation number with toggle bit.
    pub animation_number: i32,
    /// Current animation.
    pub current_animation: Option<Animation>,
    /// Animation start time.
    pub animation_time: i32,
}

/// Fresh lerp frame (`createLerpFrame`).
#[must_use]
pub fn create_lerp_frame() -> LerpFrame {
    LerpFrame {
        old_frame: 0,
        old_frame_time: 0,
        frame: 0,
        frame_time: 0,
        back_lerp: 0.0,
        animation_number: 0,
        current_animation: None,
        animation_time: 0,
    }
}

/// Lerp frame step input (`RunLerpFrameInput`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RunLerpFrameInput {
    /// Clock in milliseconds.
    pub time_ms: i32,
    /// Requested animation number.
    pub new_animation: i32,
    /// Speed scale (already `Math.fround` semantics: `f32` input is exact).
    pub speed_scale: f32,
    /// Freeze on frame zero.
    pub no_player_animations: bool,
}

fn animation_at(config: &PlayerAnimationConfig, index: i32) -> Result<Animation, Q3FoundationError> {
    if index < 0 || index as usize >= MAX_TOTAL_ANIMATIONS {
        return Err(Q3FoundationError::Drop(format!("Bad animation number: {index}")));
    }
    config.animations[index as usize].ok_or_else(|| range(format!("animation slot {index} is not playable")))
}

/// Select an animation and schedule its first frame (`setLerpFrameAnimation`).
pub fn set_lerp_frame_animation(
    config: &PlayerAnimationConfig,
    state: &mut LerpFrame,
    new_animation: i32,
    print: Option<&mut (dyn FnMut(&str) + '_)>,
) -> Result<(), Q3FoundationError> {
    state.animation_number = new_animation;
    let animation = animation_at(config, new_animation & !ANIMATION_TOGGLE_BIT)?;
    state.current_animation = Some(animation);
    state.animation_time = state.frame_time.wrapping_add(animation.initial_lerp);
    if let Some(print) = print {
        print(&format!("Anim: {}\n", new_animation & !ANIMATION_TOGGLE_BIT));
    }
    Ok(())
}

fn current_animation(state: &LerpFrame) -> Result<Animation, Q3FoundationError> {
    state
        .current_animation
        .ok_or_else(|| failed("lerp frame has no current animation"))
}

/// Advance one lerp-frame step (`runLerpFrame`).
pub fn run_lerp_frame(
    config: &PlayerAnimationConfig,
    state: &mut LerpFrame,
    input: &RunLerpFrameInput,
    mut print: Option<&mut (dyn FnMut(&str) + '_)>,
) -> Result<(), Q3FoundationError> {
    if input.no_player_animations {
        state.old_frame = 0;
        state.frame = 0;
        state.back_lerp = 0.0;
        return Ok(());
    }
    if !input.speed_scale.is_finite() || input.speed_scale < 0.0 {
        return Err(range(format!(
            "animation speed scale {} must be finite and non-negative",
            input.speed_scale
        )));
    }
    if input.new_animation != state.animation_number || state.current_animation.is_none() {
        let sink = print.as_mut().map(|held| &mut **held);
        set_lerp_frame_animation(config, state, input.new_animation, sink)?;
    }
    if input.time_ms >= state.frame_time {
        state.old_frame = state.frame;
        state.old_frame_time = state.frame_time;
        let animation = current_animation(state)?;
        if animation.frame_lerp == 0 {
            return Ok(());
        }
        if input.time_ms < state.animation_time {
            state.frame_time = state.animation_time;
        } else {
            state.frame_time = state.old_frame_time.wrapping_add(animation.frame_lerp);
        }
        let diff = state.frame_time.wrapping_sub(state.animation_time);
        let truncated = (f64::from(diff) / f64::from(animation.frame_lerp)).trunc();
        let mut frame_offset = qvm_float_to_int(truncated as f32 * input.speed_scale);
        let mut frame_count = animation.num_frames;
        if animation.flipflop {
            frame_count = frame_count.wrapping_mul(2);
        }
        if frame_offset >= frame_count {
            frame_offset = frame_offset.wrapping_sub(frame_count);
            if animation.loop_frames != 0 {
                frame_offset = frame_offset.wrapping_rem(animation.loop_frames);
                frame_offset = frame_offset
                    .wrapping_add(animation.num_frames)
                    .wrapping_sub(animation.loop_frames);
            } else {
                frame_offset = frame_count.wrapping_sub(1);
                state.frame_time = input.time_ms;
            }
        }
        if animation.reversed {
            state.frame = animation
                .first_frame
                .wrapping_add(animation.num_frames)
                .wrapping_sub(1)
                .wrapping_sub(frame_offset);
        } else if animation.flipflop && frame_offset >= animation.num_frames {
            let flip = if animation.num_frames == 0 {
                0
            } else {
                frame_offset.wrapping_rem(animation.num_frames)
            };
            state.frame = animation
                .first_frame
                .wrapping_add(animation.num_frames)
                .wrapping_sub(1)
                .wrapping_sub(flip);
        } else {
            state.frame = animation.first_frame.wrapping_add(frame_offset);
        }
        if input.time_ms > state.frame_time {
            state.frame_time = input.time_ms;
            if let Some(print) = print.as_mut() {
                print("Clamp lf->frameTime\n");
            }
        }
    }
    if state.frame_time > input.time_ms.wrapping_add(200) {
        state.frame_time = input.time_ms;
    }
    if state.old_frame_time > input.time_ms {
        state.old_frame_time = input.time_ms;
    }
    if state.frame_time == state.old_frame_time {
        state.back_lerp = 0.0;
    } else {
        let elapsed = input.time_ms.wrapping_sub(state.old_frame_time) as f32;
        let duration = state.frame_time.wrapping_sub(state.old_frame_time) as f32;
        state.back_lerp = 1.0 - elapsed / duration;
    }
    Ok(())
}

/// Reset interpolation to an animation's first frame (`clearLerpFrame`).
pub fn clear_lerp_frame(
    config: &PlayerAnimationConfig,
    state: &mut LerpFrame,
    animation: i32,
    time_ms: i32,
    print: Option<&mut (dyn FnMut(&str) + '_)>,
) -> Result<(), Q3FoundationError> {
    state.frame_time = time_ms;
    state.old_frame_time = time_ms;
    set_lerp_frame_animation(config, state, animation, print)?;
    let selected = current_animation(state)?;
    state.old_frame = selected.first_frame;
    state.frame = selected.first_frame;
    Ok(())
}

// ---------------------------------------------------------------------------
// QVM math profile (mirror of `src/core/qvm-math.ts`).
// ---------------------------------------------------------------------------

const QVM_ANGLE_SCALE: f32 = (65536.0f64 / 360.0) as f32;
const QVM_ANGLE_UNSCALE: f32 = (360.0f64 / 65536.0) as f32;
const QVM_ANGLE_RADIANS: f32 = (std::f64::consts::PI * 2.0 / 360.0) as f32;

/// QVM `AngleMod` (`qvmAngleMod`).
#[must_use]
pub fn qvm_angle_mod(angle: f32) -> f32 {
    let scaled = angle * QVM_ANGLE_SCALE;
    f64::from(qvm_float_to_int(scaled) & 65535) as f32 * QVM_ANGLE_UNSCALE
}

/// Forward/right/up vectors (`AngleVectors`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QvmAngleVectors {
    /// Forward direction.
    pub forward: Vec3,
    /// Right direction.
    pub right: Vec3,
    /// Up direction.
    pub up: Vec3,
}

/// QVM angle vectors (`qvmAngleVectors`).
#[must_use]
pub fn qvm_angle_vectors(angles: Vec3) -> QvmAngleVectors {
    let yaw = angles.y * QVM_ANGLE_RADIANS;
    let pitch = angles.x * QVM_ANGLE_RADIANS;
    let roll = angles.z * QVM_ANGLE_RADIANS;
    let sy = f64::from(yaw).sin() as f32;
    let cy = f64::from(yaw).cos() as f32;
    let sp = f64::from(pitch).sin() as f32;
    let cp = f64::from(pitch).cos() as f32;
    let sr = f64::from(roll).sin() as f32;
    let cr = f64::from(roll).cos() as f32;
    QvmAngleVectors {
        forward: vec3(cp * cy, cp * sy, -sp),
        right: vec3((-sr * sp) * cy + -cr * -sy, (-sr * sp) * sy + -cr * cy, -sr * cp),
        up: vec3((cr * sp) * cy + -sr * -sy, (cr * sp) * sy + -sr * cy, cr * cp),
    }
}

/// QVM angles-to-axis (`qvmAnglesToAxis`).
#[must_use]
pub fn qvm_angles_to_axis(angles: Vec3) -> Axis {
    let vectors = qvm_angle_vectors(angles);
    [vectors.forward, sub3(vec3(0.0, 0.0, 0.0), vectors.right), vectors.up]
}

// ---------------------------------------------------------------------------
// player-pose.ts: CG_SwingAngles, CG_PlayerAngles, CG_AddPainTwitch.
// ---------------------------------------------------------------------------

const PAIN_TWITCH_TIME: i32 = 200;
const DEAD_ENTITY_FLAG: i32 = 1;
const MOVEMENT_OFFSETS: [f32; 8] = [0.0, 22.0, 45.0, -22.0, 0.0, 22.0, -45.0, -22.0];

/// Lerp frame with swing state (`PoseLerpFrame`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PoseLerpFrame {
    /// Frame interpolation.
    pub lerp: LerpFrame,
    /// Current yaw.
    pub yaw_angle: f32,
    /// Yaw in motion.
    pub yawing: bool,
    /// Current pitch.
    pub pitch_angle: f32,
    /// Pitch in motion.
    pub pitching: bool,
}

/// Player pose state (`PlayerPoseState`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerPoseState {
    /// Legs frame.
    pub legs: PoseLerpFrame,
    /// Torso frame.
    pub torso: PoseLerpFrame,
    /// Last pain time.
    pub pain_time: i32,
    /// Pain direction toggle.
    pub pain_direction: bool,
}

fn create_pose_lerp_frame() -> PoseLerpFrame {
    PoseLerpFrame {
        lerp: create_lerp_frame(),
        yaw_angle: 0.0,
        yawing: false,
        pitch_angle: 0.0,
        pitching: false,
    }
}

/// Fresh pose state (`createPlayerPoseState`).
#[must_use]
pub fn create_player_pose_state() -> PlayerPoseState {
    PlayerPoseState {
        legs: create_pose_lerp_frame(),
        torso: create_pose_lerp_frame(),
        pain_time: 0,
        pain_direction: false,
    }
}

/// Swing input (`SwingAnglesInput`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SwingAnglesInput {
    /// Destination angle.
    pub destination: f32,
    /// Tolerance that starts swinging.
    pub swing_tolerance: f32,
    /// Tolerance that clamps.
    pub clamp_tolerance: f32,
    /// Degrees per millisecond factor.
    pub speed: f32,
    /// Frame time in milliseconds.
    pub frame_time_ms: i32,
    /// Current angle.
    pub angle: f32,
    /// Already swinging.
    pub swinging: bool,
}

/// Swing result (`SwingAnglesResult`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SwingAnglesResult {
    /// New angle.
    pub angle: f32,
    /// Still swinging.
    pub swinging: bool,
}

/// Pain twitch input (`PainTwitchInput`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PainTwitchInput {
    /// Clock in milliseconds.
    pub time_ms: i32,
    /// Pain time.
    pub pain_time: i32,
    /// Pain direction.
    pub pain_direction: bool,
}

/// Pose entity snapshot (`PoseEntityState`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PoseEntityState {
    /// Entity flags.
    pub e_flags: i32,
    /// Velocity.
    pub velocity: Vec3,
    /// Movement direction.
    pub movement_direction: f32,
    /// Legs animation.
    pub legs_anim: i32,
    /// Torso animation.
    pub torso_anim: i32,
}

/// Pose calculation input (`CalculatePlayerPoseInput`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CalculatePlayerPoseInput {
    /// Entity snapshot.
    pub entity: PoseEntityState,
    /// Fixed legs.
    pub fixed_legs: bool,
    /// Fixed torso.
    pub fixed_torso: bool,
    /// Interpolated angles.
    pub lerp_angles: Vec3,
    /// Clock in milliseconds.
    pub time_ms: i32,
    /// Frame time in milliseconds.
    pub frame_time_ms: i32,
    /// Swing speed.
    pub swing_speed: f32,
}

/// Computed hierarchical axes (`PlayerPose`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerPose {
    /// Legs axis.
    pub legs: Axis,
    /// Torso axis.
    pub torso: Axis,
    /// Head axis.
    pub head: Axis,
}

fn angle_subtract(first: f32, second: f32) -> f32 {
    let mut angle = first - second;
    while angle > 180.0 {
        angle -= 360.0;
    }
    while angle < -180.0 {
        angle += 360.0;
    }
    angle
}

/// Swing one angle toward its destination (`swingAngles`).
pub fn swing_angles(input: &SwingAnglesInput) -> Result<SwingAnglesResult, Q3FoundationError> {
    if !input.destination.is_finite() {
        return Err(range("swing destination must be a finite float32 value"));
    }
    if !input.swing_tolerance.is_finite() {
        return Err(range("swing tolerance must be a finite float32 value"));
    }
    if !input.clamp_tolerance.is_finite() {
        return Err(range("clamp tolerance must be a finite float32 value"));
    }
    if !input.speed.is_finite() {
        return Err(range("swing speed must be a finite float32 value"));
    }
    if !input.angle.is_finite() {
        return Err(range("swing angle must be a finite float32 value"));
    }
    if input.frame_time_ms < 0 {
        return Err(range("frame time must be a non-negative int32 millisecond value"));
    }
    let mut angle = input.angle;
    let mut swinging = input.swinging;
    if input.swing_tolerance < 0.0 || input.clamp_tolerance < 1.0 || input.speed < 0.0 {
        return Err(range("swing tolerances and speed are outside source ranges"));
    }
    if !swinging {
        let swing = angle_subtract(angle, input.destination);
        if swing > input.swing_tolerance || swing < -input.swing_tolerance {
            swinging = true;
        }
    }
    if !swinging {
        return Ok(SwingAnglesResult { angle, swinging });
    }
    let swing = angle_subtract(input.destination, angle);
    let distance = swing.abs();
    let scale = if distance < input.swing_tolerance * 0.5 {
        0.5
    } else if distance < input.swing_tolerance {
        1.0
    } else {
        2.0
    };
    if swing >= 0.0 {
        let mut step = input.frame_time_ms as f32 * scale * input.speed;
        if step >= swing {
            step = swing;
            swinging = false;
        }
        angle = qvm_angle_mod(angle + step);
    } else {
        let mut step = input.frame_time_ms as f32 * scale * -input.speed;
        if step <= swing {
            step = swing;
            swinging = false;
        }
        angle = qvm_angle_mod(angle + step);
    }
    let swing = angle_subtract(input.destination, angle);
    if swing > input.clamp_tolerance {
        angle = qvm_angle_mod(input.destination - (input.clamp_tolerance - 1.0));
    } else if swing < -input.clamp_tolerance {
        angle = qvm_angle_mod(input.destination + (input.clamp_tolerance - 1.0));
    }
    Ok(SwingAnglesResult { angle, swinging })
}

/// Add the decaying pain roll (`addPainTwitch`).
#[must_use]
pub fn add_pain_twitch(torso_angles: Vec3, input: &PainTwitchInput) -> Vec3 {
    let elapsed = input.time_ms.wrapping_sub(input.pain_time);
    if elapsed >= PAIN_TWITCH_TIME {
        return vec3(torso_angles.x, torso_angles.y, torso_angles.z);
    }
    let fraction = 1.0 - elapsed as f32 / PAIN_TWITCH_TIME as f32;
    let roll = 20.0 * fraction;
    vec3(
        torso_angles.x,
        torso_angles.y,
        if input.pain_direction {
            torso_angles.z + roll
        } else {
            torso_angles.z - roll
        },
    )
}

fn subtract_angles(first: Vec3, second: Vec3) -> Vec3 {
    vec3(
        angle_subtract(first.x, second.x),
        angle_subtract(first.y, second.y),
        angle_subtract(first.z, second.z),
    )
}

fn movement_offset(entity: &PoseEntityState) -> Result<f32, Q3FoundationError> {
    if entity.e_flags & DEAD_ENTITY_FLAG != 0 {
        return Ok(0.0);
    }
    let direction = qvm_float_to_int(entity.movement_direction);
    if direction < 0 || direction as usize >= MOVEMENT_OFFSETS.len() {
        return Err(Q3FoundationError::Drop("Bad player movement angle".to_string()));
    }
    MOVEMENT_OFFSETS
        .get(direction as usize)
        .copied()
        .ok_or_else(|| range(format!("missing player movement offset {direction}")))
}

fn update_yaw(
    state: &mut PoseLerpFrame,
    destination: f32,
    tolerance: f32,
    input: &CalculatePlayerPoseInput,
) -> Result<f32, Q3FoundationError> {
    let result = swing_angles(&SwingAnglesInput {
        destination,
        swing_tolerance: tolerance,
        clamp_tolerance: 90.0,
        speed: input.swing_speed,
        frame_time_ms: input.frame_time_ms,
        angle: state.yaw_angle,
        swinging: state.yawing,
    })?;
    state.yaw_angle = result.angle;
    state.yawing = result.swinging;
    Ok(result.angle)
}

/// Compute hierarchical axes while updating swing state (`calculatePlayerPose`).
pub fn calculate_player_pose(
    state: &mut PlayerPoseState,
    input: &CalculatePlayerPoseInput,
) -> Result<PlayerPose, Q3FoundationError> {
    if input.frame_time_ms < 0 {
        return Err(range("frame time must be a non-negative int32 millisecond value"));
    }
    if !input.swing_speed.is_finite() {
        return Err(range("swing speed must be a finite float32 value"));
    }
    if input.swing_speed < 0.0 {
        return Err(range("swing speed must be non-negative"));
    }
    let head_angles = vec3(
        input.lerp_angles.x,
        qvm_angle_mod(input.lerp_angles.y),
        input.lerp_angles.z,
    );
    if input.entity.legs_anim & !ANIMATION_TOGGLE_BIT != Q3PlayerAnimation::LEGS_IDLE
        || input.entity.torso_anim & !ANIMATION_TOGGLE_BIT != Q3PlayerAnimation::TORSO_STAND
    {
        state.torso.yawing = true;
        state.torso.pitching = true;
        state.legs.yawing = true;
    }

    let offset = movement_offset(&input.entity)?;
    let legs_destination = head_angles.y + offset;
    let torso_destination = head_angles.y + 0.25 * offset;
    let torso_yaw = update_yaw(&mut state.torso, torso_destination, 25.0, input)?;
    let legs_yaw = update_yaw(&mut state.legs, legs_destination, 40.0, input)?;
    let mut torso_angles = vec3(0.0, torso_yaw, 0.0);
    let mut legs_angles = vec3(0.0, legs_yaw, 0.0);

    let pitch_destination = if head_angles.x > 180.0 {
        (-360.0 + head_angles.x) * 0.75
    } else {
        head_angles.x * 0.75
    };
    let pitch = swing_angles(&SwingAnglesInput {
        destination: pitch_destination,
        swing_tolerance: 15.0,
        clamp_tolerance: 30.0,
        speed: 0.1,
        frame_time_ms: input.frame_time_ms,
        angle: state.torso.pitch_angle,
        swinging: state.torso.pitching,
    })?;
    state.torso.pitch_angle = pitch.angle;
    state.torso.pitching = pitch.swinging;
    torso_angles = vec3(pitch.angle, torso_angles.y, torso_angles.z);

    if input.fixed_torso {
        torso_angles = vec3(0.0, torso_angles.y, torso_angles.z);
    }

    let speed = length3(input.entity.velocity);
    if speed != 0.0 {
        let velocity = normalize3(input.entity.velocity);
        let lean_speed = speed * 0.05;
        let legs_axis = qvm_angles_to_axis(legs_angles);
        let side = lean_speed * dot3(velocity, legs_axis[1]);
        let forward = lean_speed * dot3(velocity, legs_axis[0]);
        legs_angles = vec3(legs_angles.x + forward, legs_angles.y, legs_angles.z - side);
    }

    if input.fixed_legs {
        legs_angles = vec3(0.0, torso_angles.y, 0.0);
    }
    torso_angles = add_pain_twitch(
        torso_angles,
        &PainTwitchInput {
            time_ms: input.time_ms,
            pain_time: state.pain_time,
            pain_direction: state.pain_direction,
        },
    );
    let head_local = subtract_angles(head_angles, torso_angles);
    let torso_local = subtract_angles(torso_angles, legs_angles);
    Ok(PlayerPose {
        legs: qvm_angles_to_axis(legs_angles),
        torso: qvm_angles_to_axis(torso_local),
        head: qvm_angles_to_axis(head_local),
    })
}

// ---------------------------------------------------------------------------
// weapon-pose.ts: CG_CalculateWeaponPosition, CG_MapTorsoToWeaponFrame,
// CG_MachinegunSpinAngle.
// ---------------------------------------------------------------------------

/// Weapon view motion (`Q3WeaponViewMotion`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3WeaponViewMotion {
    /// View origin.
    pub origin: Vec3,
    /// View angles.
    pub angles: Vec3,
    /// Clock in milliseconds.
    pub time_ms: i32,
    /// Horizontal speed.
    pub horizontal_speed: f32,
    /// Bob cycle.
    pub bob_cycle: i32,
    /// Bob fraction sine.
    pub bob_fraction_sine: f32,
    /// Land time.
    pub land_time: i32,
    /// Land change.
    pub land_change: f32,
}

/// Weapon view pose (`q3WeaponViewPose`).
#[must_use]
pub fn q3_weapon_view_pose(input: &Q3WeaponViewMotion) -> (Vec3, Vec3) {
    let scale = if input.bob_cycle & 1 != 0 {
        -input.horizontal_speed
    } else {
        input.horizontal_speed
    };
    let roll = scale * input.bob_fraction_sine * 0.005;
    let yaw = scale * input.bob_fraction_sine * 0.01;
    let pitch = input.horizontal_speed * input.bob_fraction_sine * 0.005;
    let mut origin = input.origin;
    let mut angles = add3(input.angles, vec3(pitch, yaw, roll));
    let delta = input.time_ms.wrapping_sub(input.land_time);
    if delta < 150 {
        origin = add3(origin, vec3(0.0, 0.0, input.land_change * 0.25 * delta as f32 / 150.0));
    } else if delta < 450 {
        origin = add3(
            origin,
            vec3(
                0.0,
                0.0,
                input.land_change * 0.25 * 450_i32.wrapping_sub(delta) as f32 / 300.0,
            ),
        );
    }
    let drift = (input.horizontal_speed + 40.0) * f64::from(input.time_ms as f32 * 0.001).sin() as f32 * 0.01;
    angles = add3(angles, vec3(drift, drift, drift));
    (origin, angles)
}

/// Map a torso frame to its weapon frame (`q3TorsoWeaponFrame`).
pub fn q3_torso_weapon_frame(config: &PlayerAnimationConfig, frame: i32) -> Result<i32, Q3FoundationError> {
    for index in [
        Q3PlayerAnimation::TORSO_DROP,
        Q3PlayerAnimation::TORSO_ATTACK,
        Q3PlayerAnimation::TORSO_ATTACK2,
    ] {
        let animation = config.animations[index as usize]
            .ok_or_else(|| failed(format!("Missing Q3 weapon torso animation {index}")))?;
        let drop = index == Q3PlayerAnimation::TORSO_DROP;
        let width = if drop { 9 } else { 6 };
        if frame >= animation.first_frame && frame < animation.first_frame + width {
            return Ok(frame - animation.first_frame + if drop { 6 } else { 1 });
        }
    }
    Ok(0)
}

/// Machinegun barrel spin state (`Q3WeaponBarrel`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Q3WeaponBarrel {
    time: i32,
    angle: f32,
    spinning: bool,
}

/// Barrel step result.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q3BarrelStep {
    /// Barrel angle.
    pub angle: f32,
    /// Just stopped.
    pub stopped: bool,
}

impl Q3WeaponBarrel {
    /// Advance the barrel (`step`).
    pub fn step(&mut self, time_ms: i32, firing: bool) -> Q3BarrelStep {
        let mut delta = time_ms.wrapping_sub(self.time);
        let angle = if self.spinning {
            self.angle + delta as f32 * 0.9
        } else {
            if delta > 1000 {
                delta = 1000;
            }
            let speed = 0.5 * (0.9 + 1000_i32.wrapping_sub(delta) as f32 / 1000.0);
            self.angle + delta as f32 * speed
        };
        let stopped = self.spinning && !firing;
        if self.spinning != firing {
            self.time = time_ms;
            self.angle = qvm_angle_mod(angle);
            self.spinning = firing;
        }
        Q3BarrelStep { angle, stopped }
    }
}

// ---------------------------------------------------------------------------
// Movement mirrors (shapes from `src/contracts/movement.ts`, `src/contracts/
// gameplay.ts`, and `src/movement/q3/types.ts` touched by the foundation).
// Only the Q3 arms the foundation reads or writes are mirrored.
// ---------------------------------------------------------------------------

/// Q3 weapon state (the `q3` arm of `WeaponState`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3WeaponState {
    /// Source weapon.
    pub source_weapon: i32,
    /// Weapon phase.
    pub state: i32,
    /// Weapon time in milliseconds.
    pub time_milliseconds: i32,
}

/// Q3 animation state (the `q3` arm of `AnimationState`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3AnimationState {
    /// Legs animation with toggle bit.
    pub legs: i32,
    /// Torso animation with toggle bit.
    pub torso: i32,
    /// Legs timer in milliseconds.
    pub legs_timer_ms: f64,
    /// Torso timer in milliseconds.
    pub torso_timer_ms: f64,
}

/// Arsenal state (`ArsenalState`).
#[derive(Debug, Clone, PartialEq)]
pub struct ArsenalState {
    /// Owning provider.
    pub provider: ProviderId,
    /// Active weapon item.
    pub active_weapon: Option<ItemId>,
    /// Weapon state.
    pub state: Q3WeaponState,
    /// Ammo and weapon inventory.
    pub ammo: Vec<InventoryEntry>,
}

/// Actor animation state (`ActorAnimationState`).
#[derive(Debug, Clone, PartialEq)]
pub struct ActorAnimationState {
    /// Owning provider.
    pub provider: ProviderId,
    /// Animation state.
    pub state: Q3AnimationState,
}

/// Movement environment fields the arsenal reads (`MovementEnvironment`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MovementEnvironment {
    /// Health.
    pub health: i32,
    /// Haste.
    pub haste: bool,
}

/// Predictable movement event (`PredictableMovementEvent`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PredictableMovementEvent {
    /// Owning provider.
    pub provider: ProviderId,
    /// Sequence.
    pub sequence: i32,
    /// Event.
    pub event: i32,
    /// Parameter.
    pub parameter: i32,
}

/// Ordered movement effect (`MovementEffect`, foundation-emitted arms).
#[derive(Debug, Clone, PartialEq)]
pub enum MovementEffect {
    /// Predictable event.
    Event(PredictableMovementEvent),
    /// Weapon state change.
    Weapon {
        /// Owning provider.
        provider: ProviderId,
        /// Before.
        before: Q3WeaponState,
        /// After.
        after: Q3WeaponState,
    },
    /// Weapon selection change.
    WeaponSelection {
        /// Owning provider.
        provider: ProviderId,
        /// Before.
        before: Option<ItemId>,
        /// After.
        after: Option<ItemId>,
    },
    /// Ammo count change.
    Ammo {
        /// Item.
        item: ItemId,
        /// Before.
        before: f64,
        /// After.
        after: f64,
    },
    /// Animation state change.
    Animation {
        /// Owning provider.
        provider: ProviderId,
        /// Before.
        before: Q3AnimationState,
        /// After.
        after: Q3AnimationState,
    },
}

/// Weapon step input (`WeaponStepInput`, foundation-read fields).
#[derive(Debug, Clone, PartialEq)]
pub struct WeaponStepInput {
    /// Actor.
    pub actor: OwnedActor,
    /// Frame clock.
    pub frame: FrameContext,
    /// Arsenal.
    pub arsenal: ArsenalState,
    /// Animation.
    pub animation: ActorAnimationState,
    /// Environment.
    pub environment: MovementEnvironment,
    /// Gauntlet contact.
    pub gauntlet_hit: bool,
}

/// Weapon step result (`WeaponStepResult`).
#[derive(Debug, Clone, PartialEq)]
pub struct WeaponStepResult {
    /// Arsenal.
    pub arsenal: ArsenalState,
    /// Animation.
    pub animation: ActorAnimationState,
    /// Effects.
    pub effects: Vec<MovementEffect>,
}

/// Semantic locomotion (`LocomotionAnimation`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocomotionAnimation {
    /// Idle.
    Idle,
    /// Walk.
    Walk,
    /// Run.
    Run,
    /// Backward.
    Backward,
    /// Crouch.
    Crouch,
    /// Jump.
    Jump,
    /// Land.
    Land,
    /// Swim.
    Swim,
}

/// Animation step input (`AnimationStepInput`).
#[derive(Debug, Clone, PartialEq)]
pub struct AnimationStepInput {
    /// Frame clock.
    pub frame: FrameContext,
    /// Animation.
    pub animation: ActorAnimationState,
    /// Locomotion.
    pub locomotion: LocomotionAnimation,
    /// Moving backwards.
    pub backwards: bool,
    /// Force selection.
    pub force: bool,
}

/// Animation step result (`AnimationStepResult`).
#[derive(Debug, Clone, PartialEq)]
pub struct AnimationStepResult {
    /// Animation.
    pub animation: ActorAnimationState,
    /// Effects.
    pub effects: Vec<MovementEffect>,
}

// ---------------------------------------------------------------------------
// Q3 animation operations (behavior mirror of `src/movement/q3/animation.ts`).
// ---------------------------------------------------------------------------

/// Animation request (`Q3AnimationRequest`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3AnimationRequest {
    /// Select a legs animation.
    Legs {
        /// Animation.
        animation: i32,
        /// Force selection.
        force: bool,
    },
    /// Set the legs timer.
    LegsTimer {
        /// Milliseconds.
        milliseconds: i32,
    },
    /// Drop timers by elapsed time.
    DropTimers,
    /// Gesture selection.
    Gesture,
}

/// Animation operation context (`Q3AnimationContext`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3AnimationContext {
    /// Animation.
    pub animation: ActorAnimationState,
    /// Dead.
    pub dead: bool,
    /// Elapsed milliseconds.
    pub elapsed_ms: f64,
    /// Buttons.
    pub buttons: i32,
    /// Product.
    pub product: Q3Product,
    /// Event sequence.
    pub event_sequence: i32,
}

fn animation_result(
    context: &Q3AnimationContext,
    state: Q3AnimationState,
    force_emit: bool,
    events: &[i32],
) -> AnimationStepResult {
    let mut effects = Vec::new();
    if force_emit || state != context.animation.state {
        effects.push(MovementEffect::Animation {
            provider: context.animation.provider.clone(),
            before: context.animation.state.clone(),
            after: state.clone(),
        });
    }
    for event in events {
        effects.push(MovementEffect::Event(PredictableMovementEvent {
            provider: context.animation.provider.clone(),
            sequence: context.event_sequence,
            event: *event,
            parameter: 0,
        }));
    }
    AnimationStepResult {
        animation: ActorAnimationState {
            provider: context.animation.provider.clone(),
            state,
        },
        effects,
    }
}

/// Run one animation operation (`runQ3AnimationOperation`).
pub fn run_q3_animation_operation(request: &Q3AnimationRequest, context: &Q3AnimationContext) -> AnimationStepResult {
    let state = &context.animation.state;
    match *request {
        Q3AnimationRequest::Legs { animation, force } => {
            let timer = if force { 0.0 } else { state.legs_timer_ms };
            if context.dead || timer > 0.0 || (!force && state.legs & !ANIMATION_TOGGLE_BIT == animation) {
                let mut next = state.clone();
                next.legs_timer_ms = timer;
                return animation_result(context, next, false, &[]);
            }
            animation_result(
                context,
                Q3AnimationState {
                    legs_timer_ms: timer,
                    legs: (state.legs & ANIMATION_TOGGLE_BIT ^ ANIMATION_TOGGLE_BIT) | animation,
                    ..state.clone()
                },
                false,
                &[],
            )
        }
        Q3AnimationRequest::LegsTimer { milliseconds } => {
            let mut next = state.clone();
            next.legs_timer_ms = f64::from(milliseconds);
            animation_result(context, next, true, &[])
        }
        Q3AnimationRequest::DropTimers => {
            let elapsed = context.elapsed_ms;
            let mut next = state.clone();
            if next.legs_timer_ms > 0.0 {
                next.legs_timer_ms = (next.legs_timer_ms - elapsed).max(0.0);
            }
            if next.torso_timer_ms > 0.0 {
                next.torso_timer_ms = (next.torso_timer_ms - elapsed).max(0.0);
            }
            animation_result(context, next, true, &[])
        }
        Q3AnimationRequest::Gesture => {
            if state.torso_timer_ms != 0.0 {
                return animation_result(context, state.clone(), false, &[]);
            }
            if context.buttons & Q3CommandButtons::GESTURE != 0 {
                let torso = if context.dead {
                    state.torso
                } else {
                    (state.torso & ANIMATION_TOGGLE_BIT ^ ANIMATION_TOGGLE_BIT) | Q3PlayerAnimation::TORSO_GESTURE
                };
                let mut next = state.clone();
                next.torso = torso;
                next.torso_timer_ms = f64::from(34 * 66 + 50);
                return animation_result(context, next, false, &[Q3EntityEvent::TAUNT]);
            }
            if context.product == Q3Product::MissionPack {
                let gestures = [
                    (Q3CommandButtons::GETFLAG, Q3PlayerAnimation::TORSO_GETFLAG),
                    (Q3CommandButtons::GUARDBASE, Q3PlayerAnimation::TORSO_GUARDBASE),
                    (Q3CommandButtons::PATROL, Q3PlayerAnimation::TORSO_PATROL),
                    (Q3CommandButtons::FOLLOWME, Q3PlayerAnimation::TORSO_FOLLOWME),
                    (Q3CommandButtons::AFFIRMATIVE, Q3PlayerAnimation::TORSO_AFFIRMATIVE),
                    (Q3CommandButtons::NEGATIVE, Q3PlayerAnimation::TORSO_NEGATIVE),
                ];
                for (button, animation) in gestures {
                    if context.buttons & button != 0 {
                        let torso = if context.dead {
                            state.torso
                        } else {
                            (state.torso & ANIMATION_TOGGLE_BIT ^ ANIMATION_TOGGLE_BIT) | animation
                        };
                        let mut next = state.clone();
                        next.torso = torso;
                        next.torso_timer_ms = 600.0;
                        return animation_result(context, next, false, &[]);
                    }
                }
            }
            animation_result(context, state.clone(), false, &[])
        }
    }
}

/// Run one torso operation (`runQ3TorsoOperation`).
pub fn run_q3_torso_operation(
    animation: i32,
    context: &Q3AnimationContext,
    continue_animation: bool,
) -> AnimationStepResult {
    let state = &context.animation.state;
    if context.dead
        || (continue_animation && (state.torso & !ANIMATION_TOGGLE_BIT == animation || state.torso_timer_ms > 0.0))
    {
        return animation_result(context, state.clone(), false, &[]);
    }
    let mut next = state.clone();
    next.torso = (state.torso & ANIMATION_TOGGLE_BIT ^ ANIMATION_TOGGLE_BIT) | animation;
    animation_result(context, next, false, &[])
}

// ---------------------------------------------------------------------------
// Q3 weapon step (behavior mirror of `src/movement/q3/weapon.ts`).
// ---------------------------------------------------------------------------

/// External weapon slot phase (`Q3ExternalWeaponSlot`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3ExternalWeaponSlot {
    /// Active.
    Active,
    /// Holster requested.
    HolsterRequested,
    /// Dropping.
    Dropping,
    /// Holstered.
    Holstered,
    /// Resume requested.
    ResumeRequested,
}

fn q3_weapon_delay(milliseconds: i32, persistent: i32, haste: bool) -> i32 {
    if persistent == Q3Powerup::SCOUT {
        return (f64::from(milliseconds) / 1.5).trunc() as i32;
    }
    if persistent == Q3Powerup::AMMOREGEN || haste {
        return (f64::from(milliseconds) / 1.3).trunc() as i32;
    }
    milliseconds
}

struct WeaponStepWork {
    product: Q3Product,
    pm_flags: i32,
    weapon: i32,
    weapon_state: i32,
    weapon_time: i32,
    owned_weapons: i32,
    health: i32,
    max_health: i32,
    spectator: bool,
    haste: bool,
    persistent_powerup_tag: i32,
    holdable_item: i32,
    holdable_tag: i32,
    entries: Vec<InventoryEntry>,
    provider: ProviderId,
    effects: Vec<MovementEffect>,
    event_sequence: i32,
    torso_requests: Vec<i32>,
    external_slot: Q3ExternalWeaponSlot,
    buttons: i32,
    requested_command_weapon: i32,
    msec: i32,
    gauntlet_hit: bool,
}

impl WeaponStepWork {
    fn snapshot(&self) -> Q3WeaponState {
        Q3WeaponState {
            source_weapon: self.weapon,
            state: self.weapon_state,
            time_milliseconds: self.weapon_time,
        }
    }

    fn changed(&mut self, before: &Q3WeaponState) {
        let after = self.snapshot();
        if before.source_weapon != after.source_weapon
            || before.state != after.state
            || before.time_milliseconds != after.time_milliseconds
        {
            self.effects.push(MovementEffect::Weapon {
                provider: self.provider.clone(),
                before: before.clone(),
                after,
            });
        }
    }

    fn set_weapon(&mut self, value: i32) {
        let before = self.snapshot();
        self.weapon = value;
        self.changed(&before);
        if before.source_weapon != value {
            self.effects.push(MovementEffect::WeaponSelection {
                provider: self.provider.clone(),
                before: q3_weapon_item(before.source_weapon).map(|item| item.item.to_string()),
                after: q3_weapon_item(value).map(|item| item.item.to_string()),
            });
        }
    }

    fn set_weapon_state(&mut self, value: i32) {
        let before = self.snapshot();
        self.weapon_state = value;
        self.changed(&before);
    }

    fn set_weapon_time(&mut self, value: i32) {
        let before = self.snapshot();
        self.weapon_time = value;
        self.changed(&before);
    }

    fn emit(&mut self, event: i32) {
        let sequence = self.event_sequence;
        self.event_sequence = self.event_sequence.wrapping_add(1);
        self.effects.push(MovementEffect::Event(PredictableMovementEvent {
            provider: self.provider.clone(),
            sequence,
            event,
            parameter: 0,
        }));
    }

    fn start_torso(&mut self, animation: i32) {
        self.torso_requests.push(animation);
    }

    fn ammo_get(&self, weapon: i32) -> f64 {
        let Some(item) = q3_weapon_item(weapon) else {
            return 0.0;
        };
        let Some(ammo) = item.ammo else {
            return -1.0;
        };
        self.entries
            .iter()
            .find(|entry| entry.item == ammo)
            .map_or(0.0, |entry| entry.count)
    }

    fn ammo_set(&mut self, weapon: i32, count: f64) -> Result<(), Q3FoundationError> {
        let item = q3_weapon_item(weapon).filter(|item| item.ammo.is_some());
        let Some(item) = item else {
            return Err(failed(format!("Q3 weapon has no consumable ammo slot: {weapon}")));
        };
        let ammo = item.ammo.unwrap_or("");
        let Some(entry) = self.entries.iter_mut().find(|entry| entry.item == ammo) else {
            return Err(failed(format!("Missing Q3 ammo inventory entry: {ammo}")));
        };
        let before = entry.count;
        entry.count = count;
        self.effects.push(MovementEffect::Ammo {
            item: ammo.to_string(),
            before,
            after: count,
        });
        Ok(())
    }

    fn step_holdable(&mut self, pressed: bool) -> bool {
        if pressed {
            if self.pm_flags & Q3MoveFlags::USE_ITEM_HELD == 0 {
                let tag = self.holdable_tag;
                if tag != Q3Holdable::MEDKIT || self.health < self.max_health + 25 {
                    self.pm_flags |= Q3MoveFlags::USE_ITEM_HELD;
                    self.emit(Q3EntityEvent::USE_ITEM0.wrapping_add(tag));
                    self.holdable_item = 0;
                    self.holdable_tag = 0;
                }
                return true;
            }
        } else {
            self.pm_flags &= !Q3MoveFlags::USE_ITEM_HELD;
        }
        false
    }

    fn begin_drop(&mut self) {
        self.emit(Q3EntityEvent::CHANGE_WEAPON);
        self.set_weapon_state(Q3WeaponPhase::DROPPING);
        self.set_weapon_time(self.weapon_time.wrapping_add(200));
        self.start_torso(Q3PlayerAnimation::TORSO_DROP);
    }

    fn begin_weapon_change(&mut self, weapon: i32) {
        if weapon <= Q3Weapon::NONE
            || weapon >= self.product.weapon_limit()
            || self.owned_weapons & (1 << weapon) == 0
            || self.weapon_state == Q3WeaponPhase::DROPPING
        {
            return;
        }
        self.begin_drop();
    }

    fn finish_weapon_change(&mut self) {
        let requested = self.requested_command_weapon;
        let mut weapon = match requested {
            0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 | 11 | 12 | 13 => requested,
            _ => Q3Weapon::NONE,
        };
        if weapon >= self.product.weapon_limit() {
            weapon = Q3Weapon::NONE;
        }
        if self.owned_weapons & (1 << weapon) == 0 {
            weapon = Q3Weapon::NONE;
        }
        self.set_weapon(weapon);
        self.set_weapon_state(Q3WeaponPhase::RAISING);
        self.set_weapon_time(self.weapon_time.wrapping_add(250));
        self.start_torso(Q3PlayerAnimation::TORSO_RAISE);
    }

    fn run(&mut self, firing_delay: Option<&mut dyn FnMut(i32) -> i32>) -> Result<(), Q3FoundationError> {
        if self.pm_flags & Q3MoveFlags::RESPAWNED != 0 || self.spectator {
            return Ok(());
        }
        if self.health <= 0 {
            self.set_weapon(Q3Weapon::NONE);
            return Ok(());
        }
        if self.step_holdable(self.buttons & Q3CommandButtons::USE_HOLDABLE != 0) {
            return Ok(());
        }
        if self.weapon_time > 0 {
            let time = self.weapon_time.wrapping_sub(self.msec);
            self.set_weapon_time(time);
        }
        if self.external_slot == Q3ExternalWeaponSlot::Holstered {
            return Ok(());
        }
        if self.external_slot == Q3ExternalWeaponSlot::Dropping {
            if self.weapon_time <= 0 {
                self.external_slot = Q3ExternalWeaponSlot::Holstered;
            }
            return Ok(());
        }
        if self.external_slot == Q3ExternalWeaponSlot::ResumeRequested {
            if self.weapon_time > 0 {
                return Ok(());
            }
            self.finish_weapon_change();
            self.external_slot = Q3ExternalWeaponSlot::Active;
            return Ok(());
        }
        if self.external_slot == Q3ExternalWeaponSlot::HolsterRequested
            && self.weapon_time <= 0
            && (self.weapon_state == Q3WeaponPhase::READY || self.weapon_state == Q3WeaponPhase::FIRING)
        {
            let requested = self.requested_command_weapon;
            let native_switch = requested != self.weapon
                && requested > Q3Weapon::NONE
                && requested < self.product.weapon_limit()
                && self.owned_weapons & (1 << requested) != 0;
            if !native_switch {
                self.begin_drop();
                self.external_slot = Q3ExternalWeaponSlot::Dropping;
                return Ok(());
            }
        }
        if self.weapon_time <= 0 || self.weapon_state != Q3WeaponPhase::FIRING {
            if self.weapon != self.requested_command_weapon {
                self.begin_weapon_change(self.requested_command_weapon);
            }
        }
        if self.weapon_time > 0 {
            return Ok(());
        }
        if self.weapon_state == Q3WeaponPhase::DROPPING {
            self.finish_weapon_change();
            return Ok(());
        }
        if self.weapon_state == Q3WeaponPhase::RAISING {
            self.set_weapon_state(Q3WeaponPhase::READY);
            self.start_torso(if self.weapon == Q3Weapon::GAUNTLET {
                Q3PlayerAnimation::TORSO_STAND2
            } else {
                Q3PlayerAnimation::TORSO_STAND
            });
            return Ok(());
        }
        if self.buttons & Q3CommandButtons::ATTACK == 0 || (self.weapon == Q3Weapon::GAUNTLET && !self.gauntlet_hit) {
            self.set_weapon_time(0);
            self.set_weapon_state(Q3WeaponPhase::READY);
            return Ok(());
        }
        self.start_torso(if self.weapon == Q3Weapon::GAUNTLET {
            Q3PlayerAnimation::TORSO_ATTACK2
        } else {
            Q3PlayerAnimation::TORSO_ATTACK
        });
        self.set_weapon_state(Q3WeaponPhase::FIRING);
        let ammo = self.ammo_get(self.weapon);
        if ammo == 0.0 {
            self.emit(Q3EntityEvent::NOAMMO);
            self.set_weapon_time(self.weapon_time.wrapping_add(500));
            return Ok(());
        }
        if ammo != -1.0 {
            self.ammo_set(self.weapon, ammo - 1.0)?;
        }
        self.emit(Q3EntityEvent::FIRE_WEAPON);
        let add_time = match self.weapon {
            x if x == Q3Weapon::LIGHTNING => 50,
            x if x == Q3Weapon::SHOTGUN => 1000,
            x if x == Q3Weapon::MACHINEGUN || x == Q3Weapon::PLASMAGUN => 100,
            x if x == Q3Weapon::GRENADE_LAUNCHER || x == Q3Weapon::ROCKET_LAUNCHER => 800,
            x if x == Q3Weapon::RAILGUN => 1500,
            x if x == Q3Weapon::BFG => 200,
            x if x == Q3Weapon::NAILGUN => {
                if self.product == Q3Product::MissionPack {
                    1000
                } else {
                    400
                }
            }
            x if x == Q3Weapon::PROX_LAUNCHER => {
                if self.product == Q3Product::MissionPack {
                    800
                } else {
                    400
                }
            }
            x if x == Q3Weapon::CHAINGUN => {
                if self.product == Q3Product::MissionPack {
                    30
                } else {
                    400
                }
            }
            _ => 400,
        };
        let persistent = if self.product == Q3Product::MissionPack {
            self.persistent_powerup_tag
        } else {
            0
        };
        let delay = match firing_delay {
            Some(delay) => delay(add_time),
            None => q3_weapon_delay(add_time, persistent, self.haste),
        };
        self.set_weapon_time(self.weapon_time.wrapping_add(delay));
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// arsenal.ts: PM_Weapon adapter and ClientSpawn loadout.
// ---------------------------------------------------------------------------

/// Weapon item mapping (`Q3WeaponItem`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3WeaponItem {
    /// Source weapon.
    pub weapon: i32,
    /// Weapon item.
    pub item: &'static str,
    /// Ammo item, when consumable.
    pub ammo: Option<&'static str>,
}

/// Weapon to item mapping (`Q3_WEAPON_ITEMS`).
pub const Q3_WEAPON_ITEMS: [Q3WeaponItem; 13] = [
    Q3WeaponItem {
        weapon: Q3Weapon::GAUNTLET,
        item: "q3:weapon/gauntlet",
        ammo: None,
    },
    Q3WeaponItem {
        weapon: Q3Weapon::MACHINEGUN,
        item: "q3:weapon/machinegun",
        ammo: Some("q3:ammo/machinegun"),
    },
    Q3WeaponItem {
        weapon: Q3Weapon::SHOTGUN,
        item: "q3:weapon/shotgun",
        ammo: Some("q3:ammo/shotgun"),
    },
    Q3WeaponItem {
        weapon: Q3Weapon::GRENADE_LAUNCHER,
        item: "q3:weapon/grenadelauncher",
        ammo: Some("q3:ammo/grenadelauncher"),
    },
    Q3WeaponItem {
        weapon: Q3Weapon::ROCKET_LAUNCHER,
        item: "q3:weapon/rocketlauncher",
        ammo: Some("q3:ammo/rocketlauncher"),
    },
    Q3WeaponItem {
        weapon: Q3Weapon::LIGHTNING,
        item: "q3:weapon/lightning",
        ammo: Some("q3:ammo/lightning"),
    },
    Q3WeaponItem {
        weapon: Q3Weapon::RAILGUN,
        item: "q3:weapon/railgun",
        ammo: Some("q3:ammo/railgun"),
    },
    Q3WeaponItem {
        weapon: Q3Weapon::PLASMAGUN,
        item: "q3:weapon/plasmagun",
        ammo: Some("q3:ammo/plasmagun"),
    },
    Q3WeaponItem {
        weapon: Q3Weapon::BFG,
        item: "q3:weapon/bfg",
        ammo: Some("q3:ammo/bfg"),
    },
    Q3WeaponItem {
        weapon: Q3Weapon::GRAPPLING_HOOK,
        item: "q3:weapon/grapple",
        ammo: None,
    },
    Q3WeaponItem {
        weapon: Q3Weapon::NAILGUN,
        item: "q3:weapon/nailgun",
        ammo: Some("q3:ammo/nailgun"),
    },
    Q3WeaponItem {
        weapon: Q3Weapon::PROX_LAUNCHER,
        item: "q3:weapon/proxlauncher",
        ammo: Some("q3:ammo/proxlauncher"),
    },
    Q3WeaponItem {
        weapon: Q3Weapon::CHAINGUN,
        item: "q3:weapon/chaingun",
        ammo: Some("q3:ammo/chaingun"),
    },
];

/// Look up a weapon item (`q3WeaponItem`).
#[must_use]
pub fn q3_weapon_item(weapon: i32) -> Option<Q3WeaponItem> {
    Q3_WEAPON_ITEMS.iter().find(|entry| entry.weapon == weapon).copied()
}

/// Arsenal controls (`Q3ArsenalControls`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3ArsenalControls {
    /// Attack held.
    pub attack: bool,
    /// Use-holdable held.
    pub use_holdable: bool,
    /// Requested weapon.
    pub requested_weapon: i32,
}

/// Arsenal runtime state (`Q3ArsenalRuntimeState`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3ArsenalRuntimeState {
    /// Product.
    pub product: Q3Product,
    /// Maximum health.
    pub max_health: i32,
    /// Spectator.
    pub spectator: bool,
    /// Persistent powerup tag.
    pub persistent_powerup_tag: i32,
    /// Holdable item.
    pub holdable_item: i32,
    /// Holdable tag.
    pub holdable_tag: i32,
    /// Respawned flag.
    pub respawned: bool,
    /// Use-item held flag.
    pub use_item_held: bool,
    /// Event sequence.
    pub event_sequence: i32,
    /// Fractional milliseconds.
    pub fractional_ms: f64,
    /// External slot.
    pub external_slot: Q3ExternalWeaponSlot,
    /// Requested weapon override.
    pub requested_weapon: Option<i32>,
}

/// Request a weapon (`q3RequestWeapon`).
pub fn q3_request_weapon(
    runtime: &Q3ArsenalRuntimeState,
    weapon: i32,
) -> Result<Q3ArsenalRuntimeState, Q3FoundationError> {
    let owned = Q3_WEAPON_ITEMS
        .iter()
        .any(|entry| entry.weapon == weapon && (runtime.product == Q3Product::MissionPack || weapon <= 10));
    if !owned {
        return Err(failed("Requested weapon does not belong to the Q3 product"));
    }
    let mut next = runtime.clone();
    next.requested_weapon = Some(weapon);
    Ok(next)
}

/// Request a holster (`q3RequestWeaponHolster`).
#[must_use]
pub fn q3_request_weapon_holster(runtime: &Q3ArsenalRuntimeState) -> Q3ArsenalRuntimeState {
    if runtime.external_slot == Q3ExternalWeaponSlot::ResumeRequested {
        let mut next = runtime.clone();
        next.external_slot = Q3ExternalWeaponSlot::Holstered;
        return next;
    }
    if runtime.external_slot == Q3ExternalWeaponSlot::Active {
        let mut next = runtime.clone();
        next.external_slot = Q3ExternalWeaponSlot::HolsterRequested;
        return next;
    }
    runtime.clone()
}

/// Request a resume (`q3RequestWeaponResume`).
pub fn q3_request_weapon_resume(runtime: &Q3ArsenalRuntimeState) -> Result<Q3ArsenalRuntimeState, Q3FoundationError> {
    if runtime.external_slot == Q3ExternalWeaponSlot::Active
        || runtime.external_slot == Q3ExternalWeaponSlot::ResumeRequested
    {
        return Ok(runtime.clone());
    }
    if runtime.external_slot != Q3ExternalWeaponSlot::Holstered {
        return Err(failed("Q3 primary must finish its source drop before resuming"));
    }
    let mut next = runtime.clone();
    next.external_slot = Q3ExternalWeaponSlot::ResumeRequested;
    Ok(next)
}

/// Arsenal step result (`Q3ArsenalStep`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3ArsenalStep {
    /// Arsenal.
    pub arsenal: ArsenalState,
    /// Animation.
    pub animation: ActorAnimationState,
    /// Effects.
    pub effects: Vec<MovementEffect>,
    /// Runtime.
    pub runtime: Q3ArsenalRuntimeState,
    /// Torso animation requests.
    pub torso_animations: Vec<i32>,
}

/// Step the Q3 arsenal (`stepQ3Arsenal`).
pub fn step_q3_arsenal(
    input: &WeaponStepInput,
    runtime: &Q3ArsenalRuntimeState,
    controls: &Q3ArsenalControls,
    firing_delay: Option<&mut dyn FnMut(i32) -> i32>,
) -> Result<Q3ArsenalStep, Q3FoundationError> {
    let elapsed = match input.frame.elapsed {
        SourceTime::Milliseconds(value) => f64::from(value),
        SourceTime::Seconds(value) => f64::from(value) * 1000.0,
    };
    let clock = elapsed + runtime.fractional_ms;
    let msec_value = clock.trunc();
    if !clock.is_finite() || msec_value < 0.0 {
        return Err(range("Q3 weapon step clock must be finite and nonnegative"));
    }
    let msec = msec_value as i32;
    let mut owned_weapons = 0i32;
    for weapon in &Q3_WEAPON_ITEMS {
        let count = input
            .arsenal
            .ammo
            .iter()
            .find(|entry| entry.item == weapon.item)
            .map_or(0.0, |entry| entry.count);
        if count > 0.0 {
            owned_weapons |= 1 << weapon.weapon;
        }
    }
    let mut pm_flags = (if runtime.respawned { Q3MoveFlags::RESPAWNED } else { 0 })
        | (if runtime.use_item_held {
            Q3MoveFlags::USE_ITEM_HELD
        } else {
            0
        });
    if input.environment.health > 0 && !controls.attack && !controls.use_holdable {
        pm_flags &= !Q3MoveFlags::RESPAWNED;
    }
    let mut work = WeaponStepWork {
        product: runtime.product,
        pm_flags,
        weapon: input.arsenal.state.source_weapon,
        weapon_state: input.arsenal.state.state,
        weapon_time: input.arsenal.state.time_milliseconds,
        owned_weapons,
        health: input.environment.health,
        max_health: runtime.max_health,
        spectator: runtime.spectator,
        haste: input.environment.haste,
        persistent_powerup_tag: runtime.persistent_powerup_tag,
        holdable_item: runtime.holdable_item,
        holdable_tag: runtime.holdable_tag,
        entries: input.arsenal.ammo.clone(),
        provider: input.arsenal.provider.clone(),
        effects: Vec::new(),
        event_sequence: runtime.event_sequence,
        torso_requests: Vec::new(),
        external_slot: runtime.external_slot,
        buttons: (if controls.attack { Q3CommandButtons::ATTACK } else { 0 })
            | (if controls.use_holdable {
                Q3CommandButtons::USE_HOLDABLE
            } else {
                0
            }),
        requested_command_weapon: runtime.requested_weapon.unwrap_or(controls.requested_weapon),
        msec,
        gauntlet_hit: input.gauntlet_hit,
    };
    work.run(firing_delay)?;
    let mut animation = input.animation.clone();
    let mut effects = std::mem::take(&mut work.effects);
    let mut torso_animations = Vec::new();
    let event_sequence = work.event_sequence;
    for torso in work.torso_requests.drain(..) {
        torso_animations.push(torso);
        if input.environment.health <= 0 {
            continue;
        }
        let result = run_q3_torso_operation(
            torso,
            &Q3AnimationContext {
                animation: animation.clone(),
                dead: false,
                elapsed_ms: f64::from(msec),
                buttons: 0,
                product: runtime.product,
                event_sequence,
            },
            false,
        );
        effects.extend(result.effects);
        animation = result.animation;
    }
    let weapon = work.snapshot();
    let active_weapon = q3_weapon_item(work.weapon).map(|item| item.item.to_string());
    Ok(Q3ArsenalStep {
        arsenal: ArsenalState {
            provider: input.arsenal.provider.clone(),
            active_weapon,
            state: weapon.clone(),
            ammo: work.entries.clone(),
        },
        animation,
        effects,
        torso_animations,
        runtime: Q3ArsenalRuntimeState {
            holdable_item: work.holdable_item,
            holdable_tag: work.holdable_tag,
            respawned: work.pm_flags & Q3MoveFlags::RESPAWNED != 0,
            use_item_held: work.pm_flags & Q3MoveFlags::USE_ITEM_HELD != 0,
            fractional_ms: clock - msec_value,
            event_sequence: work.event_sequence,
            external_slot: work.external_slot,
            requested_weapon: if runtime.requested_weapon == Some(weapon.source_weapon) {
                None
            } else {
                runtime.requested_weapon
            },
            ..runtime.clone()
        },
    })
}

/// Spawn runtime state (`q3SpawnArsenalRuntime`).
#[must_use]
pub fn q3_spawn_arsenal_runtime(product: Q3Product, max_health: i32, event_sequence: i32) -> Q3ArsenalRuntimeState {
    Q3ArsenalRuntimeState {
        product,
        max_health,
        spectator: false,
        persistent_powerup_tag: 0,
        holdable_item: 0,
        holdable_tag: 0,
        respawned: true,
        use_item_held: false,
        event_sequence,
        fractional_ms: 0.0,
        external_slot: Q3ExternalWeaponSlot::Active,
        requested_weapon: None,
    }
}

/// Spawn loadout (`q3SpawnLoadout`).
#[must_use]
pub fn q3_spawn_loadout(provider: ProviderId, product: Q3Product, team_deathmatch: bool) -> ArsenalState {
    let mut ammo = Vec::new();
    for weapon in &Q3_WEAPON_ITEMS {
        if product == Q3Product::BaseQ3 && weapon.weapon >= Q3Weapon::NAILGUN {
            continue;
        }
        ammo.push(InventoryEntry {
            item: weapon.item.to_string(),
            count: if weapon.weapon == Q3Weapon::GAUNTLET || weapon.weapon == Q3Weapon::MACHINEGUN {
                1.0
            } else {
                0.0
            },
            capacity: 1.0,
            count_policy: None,
        });
        if let Some(rounds) = weapon.ammo {
            ammo.push(InventoryEntry {
                item: rounds.to_string(),
                count: if weapon.weapon == Q3Weapon::MACHINEGUN {
                    if team_deathmatch {
                        50.0
                    } else {
                        100.0
                    }
                } else {
                    0.0
                },
                capacity: 200.0,
                count_policy: None,
            });
        }
    }
    ArsenalState {
        provider,
        active_weapon: Some("q3:weapon/machinegun".to_string()),
        state: Q3WeaponState {
            source_weapon: Q3Weapon::MACHINEGUN,
            state: Q3WeaponPhase::READY,
            time_milliseconds: 0,
        },
        ammo,
    }
}

/// Spawn animation (`q3SpawnAnimation`).
#[must_use]
pub fn q3_spawn_animation() -> Q3AnimationState {
    Q3AnimationState {
        legs: Q3PlayerAnimation::LEGS_IDLE,
        torso: Q3PlayerAnimation::TORSO_STAND,
        legs_timer_ms: 0.0,
        torso_timer_ms: 0.0,
    }
}

// ---------------------------------------------------------------------------
// movement-hooks.ts: PM_Firing, PM_Animate, PM_Weapon, PM_TorsoAnimation.
// ---------------------------------------------------------------------------

/// Hook execution mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3Execution {
    /// Authoritative.
    Authoritative,
    /// Prediction.
    Prediction,
}

/// Hook command fields (`Q3Command` fields the hooks read).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3HookCommand {
    /// Buttons.
    pub buttons: i32,
    /// Requested weapon.
    pub weapon: i32,
}

/// Locomotion work state fields the hooks read (`Q3Motion` subset).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3MotionWork {
    /// Movement type.
    pub pm_type: i32,
    /// Movement flags.
    pub pm_flags: i32,
    /// Event sequence.
    pub event_sequence: i32,
    /// Product.
    pub product: Q3Product,
}

/// Hook input fields (`Q3MovementInput` subset).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3HookInput {
    /// Actor.
    pub actor: OwnedActor,
    /// Execution mode.
    pub execution: Q3Execution,
    /// Environment.
    pub environment: MovementEnvironment,
}

/// Movement hook context (`Q3HookContext`, foundation-read fields).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3HookContext {
    /// Input.
    pub input: Q3HookInput,
    /// Motion.
    pub motion: Q3MotionWork,
    /// Command.
    pub command: Q3HookCommand,
    /// Frame clock.
    pub frame: FrameContext,
    /// Arsenal.
    pub arsenal: ArsenalState,
    /// Animation.
    pub animation: ActorAnimationState,
}

/// Arsenal runtime storage (`Q3ArsenalRuntimeAccess`).
pub trait Q3ArsenalRuntimeAccess {
    /// Read runtime state.
    fn read(&self, actor: &OwnedActor, execution: Q3Execution) -> Q3ArsenalRuntimeState;
    /// Write runtime state.
    fn write(&self, actor: &OwnedActor, execution: Q3Execution, state: Q3ArsenalRuntimeState);
    /// Gauntlet contact test.
    fn gauntlet_hit(&self, context: &Q3HookContext) -> bool;
}

/// Derive arsenal controls from a command (`q3SourceArsenalControls`).
#[must_use]
pub fn q3_source_arsenal_controls(context: &Q3HookContext) -> Q3ArsenalControls {
    Q3ArsenalControls {
        attack: context.command.buttons & Q3CommandButtons::ATTACK != 0,
        use_holdable: context.command.buttons & Q3CommandButtons::USE_HOLDABLE != 0,
        requested_weapon: context.command.weapon,
    }
}

/// Weapon phase result (`Q3WeaponPhaseResult`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3WeaponPhaseResult {
    /// Arsenal.
    pub arsenal: ArsenalState,
    /// Animation.
    pub animation: ActorAnimationState,
    /// Effects.
    pub effects: Vec<MovementEffect>,
    /// Movement flags.
    pub movement_flags: i32,
}

/// Selected movement hooks (`Q3MovementHooks`).
pub trait Q3MovementHooks {
    /// Firing test.
    fn firing(&self, context: &Q3HookContext) -> bool;
    /// Animation request.
    fn animation(&self, request: &Q3AnimationRequest, context: &Q3HookContext) -> AnimationStepResult;
    /// Weapon stage.
    fn weapon(&self, context: &Q3HookContext) -> Result<Q3WeaponPhaseResult, Q3FoundationError>;
    /// Torso stage.
    fn torso(&self, context: &Q3HookContext) -> AnimationStepResult;
}

fn hook_animation_context(context: &Q3HookContext) -> Q3AnimationContext {
    let elapsed_ms = match context.frame.elapsed {
        SourceTime::Milliseconds(value) => f64::from(value),
        SourceTime::Seconds(value) => f64::from(value),
    };
    Q3AnimationContext {
        animation: context.animation.clone(),
        dead: context.motion.pm_type >= Q3MoveType::DEAD,
        elapsed_ms,
        buttons: context.command.buttons,
        product: context.motion.product,
        event_sequence: context.motion.event_sequence,
    }
}

/// Source Q3 movement hooks (`createQ3SourceMovementHooks`).
#[derive(Debug)]
pub struct Q3SourceMovementHooks<R> {
    runtime: R,
}

impl<R> Q3SourceMovementHooks<R> {
    /// Wrap runtime storage.
    pub fn new(runtime: R) -> Self {
        Self { runtime }
    }
}

/// Build source movement hooks (`createQ3SourceMovementHooks`).
pub fn create_q3_source_movement_hooks<R: Q3ArsenalRuntimeAccess>(runtime: R) -> Q3SourceMovementHooks<R> {
    Q3SourceMovementHooks::new(runtime)
}

impl<R: Q3ArsenalRuntimeAccess> Q3MovementHooks for Q3SourceMovementHooks<R> {
    fn firing(&self, context: &Q3HookContext) -> bool {
        let Some(item) = q3_weapon_item(context.arsenal.state.source_weapon) else {
            return false;
        };
        match item.ammo {
            None => true,
            Some(ammo) => {
                context
                    .arsenal
                    .ammo
                    .iter()
                    .find(|entry| entry.item == ammo)
                    .map_or(0.0, |entry| entry.count)
                    != 0.0
            }
        }
    }

    fn animation(&self, request: &Q3AnimationRequest, context: &Q3HookContext) -> AnimationStepResult {
        run_q3_animation_operation(request, &hook_animation_context(context))
    }

    fn weapon(&self, context: &Q3HookContext) -> Result<Q3WeaponPhaseResult, Q3FoundationError> {
        let previous = self.runtime.read(&context.input.actor, context.input.execution);
        let state = Q3ArsenalRuntimeState {
            respawned: context.motion.pm_flags & Q3MoveFlags::RESPAWNED != 0,
            use_item_held: context.motion.pm_flags & Q3MoveFlags::USE_ITEM_HELD != 0,
            event_sequence: context.motion.event_sequence,
            ..previous
        };
        let result = step_q3_arsenal(
            &WeaponStepInput {
                actor: context.input.actor.clone(),
                frame: context.frame.clone(),
                arsenal: context.arsenal.clone(),
                animation: context.animation.clone(),
                environment: context.input.environment,
                gauntlet_hit: self.runtime.gauntlet_hit(context),
            },
            &state,
            &q3_source_arsenal_controls(context),
            None,
        )?;
        self.runtime
            .write(&context.input.actor, context.input.execution, result.runtime.clone());
        let movement_flags = (context.motion.pm_flags & !(Q3MoveFlags::RESPAWNED | Q3MoveFlags::USE_ITEM_HELD))
            | (if result.runtime.respawned {
                Q3MoveFlags::RESPAWNED
            } else {
                0
            })
            | (if result.runtime.use_item_held {
                Q3MoveFlags::USE_ITEM_HELD
            } else {
                0
            });
        Ok(Q3WeaponPhaseResult {
            arsenal: result.arsenal,
            animation: result.animation,
            effects: result.effects,
            movement_flags,
        })
    }

    fn torso(&self, context: &Q3HookContext) -> AnimationStepResult {
        if context.arsenal.state.state != Q3WeaponPhase::READY {
            return AnimationStepResult {
                animation: context.animation.clone(),
                effects: Vec::new(),
            };
        }
        let animation = if context.arsenal.state.source_weapon == Q3Weapon::GAUNTLET {
            Q3PlayerAnimation::TORSO_STAND2
        } else {
            Q3PlayerAnimation::TORSO_STAND
        };
        run_q3_torso_operation(animation, &hook_animation_context(context), true)
    }
}

// ---------------------------------------------------------------------------
// character.ts: ClientSpawn, player_die, character animation admission.
// ---------------------------------------------------------------------------

/// Character collision bounds (`Q3_CHARACTER_BOUNDS`).
pub const Q3_CHARACTER_BOUNDS: Bounds = Bounds {
    min: Vec3 {
        x: -15.0,
        y: -15.0,
        z: -24.0,
    },
    max: Vec3 {
        x: 15.0,
        y: 15.0,
        z: 32.0,
    },
};

/// Character view height (`Q3_CHARACTER_VIEW_HEIGHT`).
pub const Q3_CHARACTER_VIEW_HEIGHT: i32 = 26;

/// Combat state (`CombatState`, foundation-read fields).
#[derive(Debug, Clone, PartialEq)]
pub struct CombatState {
    /// Health.
    pub health: i32,
    /// Armor.
    pub armor: ArmorState,
    /// Mass.
    pub mass: f64,
    /// Can take damage.
    pub can_take_damage: bool,
    /// Invulnerable.
    pub invulnerable: bool,
    /// No knockback.
    pub no_knockback: bool,
    /// Team.
    pub team: Option<String>,
}

/// Combat trait changes (`Partial<CombatTraits>`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CombatTraitChanges {
    /// Can take damage.
    pub can_take_damage: Option<bool>,
    /// Mass.
    pub mass: Option<f64>,
    /// Invulnerable.
    pub invulnerable: Option<bool>,
    /// Team.
    pub team: Option<Option<String>>,
    /// No knockback.
    pub no_knockback: Option<bool>,
}

impl CombatTraitChanges {
    /// Copy every trait from a combat state.
    #[must_use]
    pub fn from_combat(combat: &CombatState) -> Self {
        Self {
            can_take_damage: Some(combat.can_take_damage),
            mass: Some(combat.mass),
            invulnerable: Some(combat.invulnerable),
            team: Some(combat.team.clone()),
            no_knockback: Some(combat.no_knockback),
        }
    }
}

/// Body state (`BodyState`).
#[derive(Debug, Clone, PartialEq)]
pub struct BodyState {
    /// Origin.
    pub origin: Vec3,
    /// Angles.
    pub angles: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Bounds.
    pub bounds: Bounds,
    /// Ground actor.
    pub ground: Option<ActorId>,
}

/// Character event (`Q3CharacterEvent`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3CharacterEvent {
    /// Actor.
    pub actor: OwnedActor,
    /// Sequence.
    pub sequence: i32,
    /// Time in milliseconds.
    pub time_ms: i32,
    /// Event.
    pub event: i32,
    /// Parameter.
    pub parameter: i32,
}

/// Death context (`Q3CharacterDeathContext`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3CharacterDeathContext {
    /// Blood.
    pub blood: bool,
    /// No item drop.
    pub no_drop: bool,
    /// Suicide.
    pub suicide: bool,
    /// Killer source slot.
    pub killer_source_slot: i32,
}

/// Spawn input (`Q3CharacterSpawn`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3CharacterSpawn {
    /// Body.
    pub body: BodyState,
    /// Combat.
    pub combat: CombatState,
    /// Inventory.
    pub inventory: Vec<InventoryEntry>,
}

/// Character checkpoint (`Q3CharacterCheckpoint`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3CharacterCheckpoint {
    /// Version (always 1).
    pub version: u32,
    /// Product.
    pub product: Q3Product,
    /// Animation.
    pub animation: Q3AnimationState,
    /// Source flags.
    pub flags: i32,
    /// Event sequence.
    pub event_sequence: i32,
    /// Respawn time.
    pub respawn_time: i32,
    /// Spawn count.
    pub spawn_count: i32,
    /// Dead.
    pub dead: bool,
    /// Gibbed.
    pub gibbed: bool,
    /// Initialized.
    pub initialized: bool,
}

/// Death animation checkpoint (`Q3DeathAnimationCheckpoint`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3DeathAnimationCheckpoint {
    /// Version (always 1).
    pub version: u32,
    /// Index.
    pub index: u32,
}

/// Placement mode (`Q3CharacterServices` placement arm).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3Placement {
    /// Source game placement.
    SourceGame,
    /// Character placement.
    Character,
}

/// Bound character callbacks (pain reads current health itself, as the donor
/// closure does; die runs the shared die path).
pub struct Q3CharacterCallbackSet {
    /// Pain handler.
    pub pain: Box<dyn Fn()>,
    /// Die handler.
    pub die: Box<dyn Fn()>,
}

/// Character services (`Q3CharacterServices`).
pub trait Q3CharacterServices {
    /// Read a body.
    fn read_body(&self, actor: &ActorId) -> Option<BodyState>;
    /// Write a body.
    fn write_body(&self, actor: &OwnedActor, body: BodyState);
    /// Link a body.
    fn link_body(&self, actor: &OwnedActor);
    /// Unlink a body.
    fn unlink_body(&self, actor: &OwnedActor);
    /// Bind pain/die callbacks.
    fn bind_callbacks(&self, actor: &OwnedActor, callbacks: Q3CharacterCallbackSet);
    /// Read combat.
    fn read_combat(&self, actor: &ActorId) -> Option<CombatState>;
    /// Create combat.
    fn create_combat(&self, actor: &OwnedActor, combat: CombatState);
    /// Set health.
    fn set_health(&self, actor: &OwnedActor, health: i32);
    /// Set armor.
    fn set_armor(&self, actor: &OwnedActor, armor: ArmorState);
    /// Set traits.
    fn set_traits(&self, actor: &OwnedActor, traits: CombatTraitChanges);
    /// Inventory presence.
    fn has_inventory(&self, actor: &ActorId) -> bool;
    /// Create inventory.
    fn create_inventory(&self, actor: &OwnedActor, entries: Vec<InventoryEntry>);
    /// Inventory entries.
    fn inventory_entries(&self, actor: &ActorId) -> Vec<InventoryEntry>;
    /// Configure one entry.
    fn configure_inventory(&self, actor: &OwnedActor, entry: InventoryEntry);
    /// Current time in milliseconds.
    fn time_ms(&self) -> i32;
    /// Emit an event.
    fn emit(&self, event: Q3CharacterEvent);
    /// Death context.
    fn death_context(&self, actor: &OwnedActor) -> Q3CharacterDeathContext;
    /// Placement mode.
    fn placement(&self) -> Q3Placement;
    /// Run spawn targets.
    fn spawn_targets(&self, actor: &OwnedActor);
    /// Run kill box.
    fn kill_box(&self, actor: &OwnedActor);
}

/// Global death animation cycling (`Q3DeathAnimationSequence`).
#[derive(Debug, Clone, Default)]
pub struct Q3DeathAnimationSequence {
    index: u32,
}

impl Q3DeathAnimationSequence {
    /// Fresh sequence.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Capture the index.
    #[must_use]
    pub fn capture(&self) -> Q3DeathAnimationCheckpoint {
        Q3DeathAnimationCheckpoint {
            version: 1,
            index: self.index,
        }
    }

    /// Restore the index.
    pub fn restore(&mut self, checkpoint: &Q3DeathAnimationCheckpoint) -> Result<(), Q3FoundationError> {
        if checkpoint.version != 1 || checkpoint.index > 2 {
            return Err(type_error("Invalid Q3 death animation checkpoint"));
        }
        self.index = checkpoint.index;
        Ok(())
    }

    /// Next death animation and event.
    pub fn next(&mut self) -> (i32, i32) {
        let result = if self.index == 0 {
            (Q3PlayerAnimation::BOTH_DEATH1, Q3EntityEvent::DEATH1)
        } else if self.index == 1 {
            (Q3PlayerAnimation::BOTH_DEATH2, Q3EntityEvent::DEATH2)
        } else {
            (Q3PlayerAnimation::BOTH_DEATH3, Q3EntityEvent::DEATH3)
        };
        self.index = (self.index + 1) % 3;
        result
    }
}

/// Maximum health from a handicap (`q3MaximumHealth`).
pub fn q3_maximum_health(handicap: &str) -> Result<i32, Q3FoundationError> {
    let maximum = game_atoi(handicap)?;
    Ok(if maximum < 1 || maximum > 100 { 100 } else { maximum })
}

/// Initial combat state (`q3InitialCombat`).
pub fn q3_initial_combat(handicap: &str, team: Option<String>) -> Result<CombatState, Q3FoundationError> {
    Ok(CombatState {
        health: q3_maximum_health(handicap)?.wrapping_add(25),
        armor: ArmorState {
            regular: RegularArmorState::Q3 {
                points: 0.0,
                protection: f64::from(0.66f32),
            },
            powered: PoweredProtectionState::None,
        },
        mass: 200.0,
        can_take_damage: true,
        invulnerable: false,
        no_knockback: false,
        team,
    })
}

#[derive(Debug, Clone, PartialEq)]
struct Q3CharacterInner {
    animation: Q3AnimationState,
    flags: i32,
    sequence: i32,
    respawn_time: i32,
    spawn_count: i32,
    dead: bool,
    gibbed: bool,
    initialized: bool,
}

/// Q3 character actor (`Q3CharacterActor`). Clones share identity, like the
/// donor object; bound callbacks reenter through the shared state.
#[derive(Debug, Clone)]
pub struct Q3CharacterActor<S> {
    actor: OwnedActor,
    provider: ProviderId,
    product: Q3Product,
    services: Rc<S>,
    death_animations: Rc<RefCell<Q3DeathAnimationSequence>>,
    inner: Rc<RefCell<Q3CharacterInner>>,
}

impl<S: Q3CharacterServices + 'static> Q3CharacterActor<S> {
    /// Bind a character to its services.
    pub fn new(
        actor: OwnedActor,
        provider: ProviderId,
        product: Q3Product,
        services: Rc<S>,
        death_animations: Rc<RefCell<Q3DeathAnimationSequence>>,
    ) -> Self {
        Self {
            actor,
            provider,
            product,
            services,
            death_animations,
            inner: Rc::new(RefCell::new(Q3CharacterInner {
                animation: q3_spawn_animation(),
                flags: 0,
                sequence: 0,
                respawn_time: 0,
                spawn_count: 0,
                dead: false,
                gibbed: false,
                initialized: false,
            })),
        }
    }

    /// Current animation.
    #[must_use]
    pub fn animation(&self) -> ActorAnimationState {
        ActorAnimationState {
            provider: self.provider.clone(),
            state: self.inner.borrow().animation.clone(),
        }
    }

    /// Source flags.
    #[must_use]
    pub fn source_flags(&self) -> i32 {
        self.inner.borrow().flags
    }

    /// Event sequence.
    #[must_use]
    pub fn event_sequence(&self) -> i32 {
        self.inner.borrow().sequence
    }

    /// Respawn eligibility time.
    #[must_use]
    pub fn respawn_eligible_after_ms(&self) -> i32 {
        self.inner.borrow().respawn_time
    }

    /// Spawn count.
    #[must_use]
    pub fn spawns(&self) -> i32 {
        self.inner.borrow().spawn_count
    }

    /// Jump event.
    pub fn jump(&self) {
        Self::emit_inner(&self.inner, &self.services, &self.actor, Q3EntityEvent::JUMP, 0);
    }

    fn emit_inner(
        inner: &Rc<RefCell<Q3CharacterInner>>,
        services: &Rc<S>,
        actor: &OwnedActor,
        event: i32,
        parameter: i32,
    ) {
        let sequence = {
            let mut guard = inner.borrow_mut();
            let sequence = guard.sequence;
            guard.sequence = guard.sequence.wrapping_add(1);
            sequence
        };
        services.emit(Q3CharacterEvent {
            actor: actor.clone(),
            sequence,
            time_ms: services.time_ms(),
            event,
            parameter,
        });
    }

    fn emit(&self, event: i32, parameter: i32) {
        Self::emit_inner(&self.inner, &self.services, &self.actor, event, parameter);
    }

    /// Capture a checkpoint.
    #[must_use]
    pub fn capture(&self) -> Q3CharacterCheckpoint {
        let inner = self.inner.borrow();
        Q3CharacterCheckpoint {
            version: 1,
            product: self.product,
            animation: inner.animation.clone(),
            flags: inner.flags,
            event_sequence: inner.sequence,
            respawn_time: inner.respawn_time,
            spawn_count: inner.spawn_count,
            dead: inner.dead,
            gibbed: inner.gibbed,
            initialized: inner.initialized,
        }
    }

    /// Restore a checkpoint.
    pub fn restore(&self, checkpoint: &Q3CharacterCheckpoint) -> Result<(), Q3FoundationError> {
        if checkpoint.version != 1 || checkpoint.product != self.product {
            return Err(type_error("Q3 character checkpoint belongs to another source product"));
        }
        if !checkpoint.initialized && self.inner.borrow().initialized {
            return Err(failed(
                "Cannot restore an unadmitted Q3 character over an admitted binding",
            ));
        }
        if checkpoint.initialized && !self.inner.borrow().initialized {
            if self.services.read_body(self.actor.id()).is_none()
                || self.services.read_combat(self.actor.id()).is_none()
                || !self.services.has_inventory(self.actor.id())
            {
                return Err(failed("Restore shared Q3 character stores before private state"));
            }
            self.bind_callbacks();
        }
        let mut inner = self.inner.borrow_mut();
        inner.animation = checkpoint.animation.clone();
        inner.flags = checkpoint.flags;
        inner.sequence = checkpoint.event_sequence;
        inner.respawn_time = checkpoint.respawn_time;
        inner.spawn_count = checkpoint.spawn_count;
        inner.dead = checkpoint.dead;
        inner.gibbed = checkpoint.gibbed;
        Ok(())
    }

    fn bind_callbacks(&self) {
        if self.inner.borrow().initialized {
            return;
        }
        let pain_inner = Rc::clone(&self.inner);
        let pain_services = Rc::clone(&self.services);
        let pain_actor = self.actor.clone();
        let die_inner = Rc::clone(&self.inner);
        let die_services = Rc::clone(&self.services);
        let die_actor = self.actor.clone();
        let die_animations = Rc::clone(&self.death_animations);
        self.services.bind_callbacks(
            &self.actor,
            Q3CharacterCallbackSet {
                pain: Box::new(move || {
                    let health = pain_services
                        .read_combat(pain_actor.id())
                        .map_or(0, |combat| combat.health);
                    Self::emit_inner(&pain_inner, &pain_services, &pain_actor, Q3EntityEvent::PAIN, health);
                }),
                die: Box::new(move || {
                    Self::die_inner(&die_inner, &die_services, &die_actor, &die_animations)
                        .expect("Q3 character die failed");
                }),
            },
        );
        self.inner.borrow_mut().initialized = true;
    }

    /// Spawn the character.
    pub fn spawn(&self, input: &Q3CharacterSpawn) -> Result<(), Q3FoundationError> {
        if self.services.read_combat(self.actor.id()).is_none() {
            self.services.create_combat(&self.actor, input.combat.clone());
        } else {
            self.services.set_health(&self.actor, input.combat.health);
            self.services.set_armor(&self.actor, input.combat.armor.clone());
            self.services
                .set_traits(&self.actor, CombatTraitChanges::from_combat(&input.combat));
        }
        if !self.services.has_inventory(self.actor.id()) {
            self.services.create_inventory(&self.actor, input.inventory.clone());
        } else {
            for entry in self.services.inventory_entries(self.actor.id()) {
                let mut cleared = entry.clone();
                cleared.count = 0.0;
                self.services.configure_inventory(&self.actor, cleared);
            }
            for entry in &input.inventory {
                self.services.configure_inventory(&self.actor, entry.clone());
            }
        }
        {
            let mut inner = self.inner.borrow_mut();
            inner.animation = q3_spawn_animation();
            inner.flags = (inner.flags & (4 | 0x4000 | 0x80000)) ^ 4;
            inner.dead = false;
            inner.gibbed = false;
            inner.respawn_time = self.services.time_ms();
            inner.spawn_count = inner.spawn_count.wrapping_add(1);
        }
        self.services.write_body(&self.actor, input.body.clone());
        self.bind_callbacks();
        if self.services.placement() != Q3Placement::SourceGame {
            self.services.kill_box(&self.actor);
            self.services.link_body(&self.actor);
            self.services.spawn_targets(&self.actor);
            if self.inner.borrow().spawn_count > 1 {
                self.emit(Q3EntityEvent::PLAYER_TELEPORT_IN, 0);
            }
        }
        Ok(())
    }

    fn gib_inner(inner: &Rc<RefCell<Q3CharacterInner>>, services: &Rc<S>, actor: &OwnedActor, killer_source_slot: i32) {
        {
            let mut guard = inner.borrow_mut();
            guard.gibbed = true;
            guard.flags |= 0x80;
        }
        services.set_traits(
            actor,
            CombatTraitChanges {
                can_take_damage: Some(false),
                ..CombatTraitChanges::default()
            },
        );
        services.unlink_body(actor);
        Self::emit_inner(inner, services, actor, Q3EntityEvent::GIB_PLAYER, killer_source_slot);
    }

    fn die_inner(
        inner: &Rc<RefCell<Q3CharacterInner>>,
        services: &Rc<S>,
        actor: &OwnedActor,
        death_animations: &Rc<RefCell<Q3DeathAnimationSequence>>,
    ) -> Result<(), Q3FoundationError> {
        if inner.borrow().gibbed {
            return Ok(());
        }
        let health = services
            .read_combat(actor.id())
            .map(|combat| combat.health)
            .ok_or_else(|| failed("Q3 character has no health binding"))?;
        let context = services.death_context(actor);
        if inner.borrow().dead {
            if health <= -40 && context.blood {
                Self::gib_inner(inner, services, actor, context.killer_source_slot);
            } else if !context.blood && health <= -40 {
                services.set_health(actor, -39);
            }
            return Ok(());
        }
        {
            let mut guard = inner.borrow_mut();
            guard.dead = true;
            guard.flags |= 1;
            guard.respawn_time = services.time_ms().wrapping_add(1700);
        }
        if let Some(body) = services.read_body(actor.id()) {
            services.write_body(
                actor,
                BodyState {
                    angles: vec3(0.0, body.angles.y, 0.0),
                    bounds: Bounds {
                        max: vec3(body.bounds.max.x, body.bounds.max.y, -8.0),
                        ..body.bounds
                    },
                    ..body
                },
            );
            services.link_body(actor);
        }
        if (health <= -40 && !context.no_drop && context.blood) || context.suicide {
            Self::gib_inner(inner, services, actor, context.killer_source_slot);
        } else {
            if health <= -40 {
                services.set_health(actor, -39);
            }
            let (animation, event) = death_animations.borrow_mut().next();
            {
                let mut guard = inner.borrow_mut();
                guard.animation.legs = (guard.animation.legs & ANIMATION_TOGGLE_BIT ^ ANIMATION_TOGGLE_BIT) | animation;
                guard.animation.torso =
                    (guard.animation.torso & ANIMATION_TOGGLE_BIT ^ ANIMATION_TOGGLE_BIT) | animation;
            }
            Self::emit_inner(inner, services, actor, event, context.killer_source_slot);
        }
        Ok(())
    }

    /// Run death after lethal damage committed.
    pub fn die(&self) -> Result<(), Q3FoundationError> {
        Self::die_inner(&self.inner, &self.services, &self.actor, &self.death_animations)
    }

    /// Respawn eligibility (`wantsRespawn`).
    #[must_use]
    pub fn wants_respawn(&self, now_ms: i32, attack: bool, use_holdable: bool, force_respawn_seconds: i32) -> bool {
        let inner = self.inner.borrow();
        if !inner.dead || now_ms <= inner.respawn_time {
            return false;
        }
        attack
            || use_holdable
            || (force_respawn_seconds > 0
                && now_ms.wrapping_sub(inner.respawn_time) > force_respawn_seconds.wrapping_mul(1000))
    }

    /// Commit an animation from the animation owner.
    pub fn commit_animation(&self, animation: &ActorAnimationState) -> Result<(), Q3FoundationError> {
        if animation.provider != self.provider {
            return Err(type_error("Q3 character animation belongs to another provider"));
        }
        self.inner.borrow_mut().animation = animation.state.clone();
        Ok(())
    }
}

/// Step semantic locomotion into Q3 clips (`stepQ3CharacterAnimation`).
#[must_use]
pub fn step_q3_character_animation(
    input: &AnimationStepInput,
    product: Q3Product,
    dead: bool,
    event_sequence: i32,
) -> AnimationStepResult {
    let elapsed_ms = match input.frame.elapsed {
        SourceTime::Milliseconds(value) => f64::from(value),
        SourceTime::Seconds(value) => (f64::from(value) * 1000.0).trunc(),
    };
    let context = Q3AnimationContext {
        animation: input.animation.clone(),
        dead,
        elapsed_ms,
        buttons: 0,
        product,
        event_sequence,
    };
    let dropped = run_q3_animation_operation(&Q3AnimationRequest::DropTimers, &context);
    let animation = match input.locomotion {
        LocomotionAnimation::Idle => Q3PlayerAnimation::LEGS_IDLE,
        LocomotionAnimation::Walk => {
            if input.backwards {
                Q3PlayerAnimation::LEGS_BACKWALK
            } else {
                Q3PlayerAnimation::LEGS_WALK
            }
        }
        LocomotionAnimation::Run => {
            if input.backwards {
                Q3PlayerAnimation::LEGS_BACK
            } else {
                Q3PlayerAnimation::LEGS_RUN
            }
        }
        LocomotionAnimation::Backward => Q3PlayerAnimation::LEGS_BACK,
        LocomotionAnimation::Crouch => {
            if input.backwards {
                Q3PlayerAnimation::LEGS_BACKCR
            } else {
                Q3PlayerAnimation::LEGS_WALKCR
            }
        }
        LocomotionAnimation::Jump => {
            if input.backwards {
                Q3PlayerAnimation::LEGS_JUMPB
            } else {
                Q3PlayerAnimation::LEGS_JUMP
            }
        }
        LocomotionAnimation::Land => {
            if input.backwards {
                Q3PlayerAnimation::LEGS_LANDB
            } else {
                Q3PlayerAnimation::LEGS_LAND
            }
        }
        LocomotionAnimation::Swim => Q3PlayerAnimation::LEGS_SWIM,
    };
    let selected = run_q3_animation_operation(
        &Q3AnimationRequest::Legs {
            animation,
            force: input.force,
        },
        &Q3AnimationContext {
            animation: dropped.animation.clone(),
            ..context.clone()
        },
    );
    if input.locomotion != LocomotionAnimation::Land {
        let mut effects = dropped.effects;
        effects.extend(selected.effects);
        return AnimationStepResult {
            animation: selected.animation,
            effects,
        };
    }
    let landed = run_q3_animation_operation(
        &Q3AnimationRequest::LegsTimer { milliseconds: 130 },
        &Q3AnimationContext {
            animation: selected.animation.clone(),
            ..context
        },
    );
    let mut effects = dropped.effects;
    effects.extend(selected.effects);
    effects.extend(landed.effects);
    AnimationStepResult {
        animation: landed.animation,
        effects,
    }
}

// ---------------------------------------------------------------------------
// events.ts: CG_EntityEvent and CG_PainEvent character behavior.
// ---------------------------------------------------------------------------

/// Sound channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3SoundChannel {
    /// Auto.
    Auto,
    /// Voice.
    Voice,
    /// Body.
    Body,
}

/// Footstep material.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3StepMaterial {
    /// Normal.
    Normal,
    /// Boot.
    Boot,
    /// Flesh.
    Flesh,
    /// Mech.
    Mech,
    /// Energy.
    Energy,
    /// Metal.
    Metal,
    /// Splash.
    Splash,
}

impl From<PlayerFootsteps> for Q3StepMaterial {
    fn from(footsteps: PlayerFootsteps) -> Self {
        match footsteps {
            PlayerFootsteps::Normal => Q3StepMaterial::Normal,
            PlayerFootsteps::Boot => Q3StepMaterial::Boot,
            PlayerFootsteps::Flesh => Q3StepMaterial::Flesh,
            PlayerFootsteps::Mech => Q3StepMaterial::Mech,
            PlayerFootsteps::Energy => Q3StepMaterial::Energy,
        }
    }
}

/// Teleport direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3TeleportDirection {
    /// In.
    In,
    /// Out.
    Out,
}

/// Character presentation effect (`Q3CharacterPresentationEffect`).
#[derive(Debug, Clone, PartialEq)]
pub enum Q3CharacterPresentationEffect {
    /// Custom character sound.
    CustomSound {
        /// Channel.
        channel: Q3SoundChannel,
        /// Name.
        name: String,
    },
    /// Sound path.
    Sound {
        /// Channel.
        channel: Q3SoundChannel,
        /// Path.
        path: String,
    },
    /// Footstep.
    Footstep {
        /// Material.
        material: Q3StepMaterial,
        /// Variant.
        variant: i32,
    },
    /// Jump pad smoke (radius 32, 1000ms).
    JumpPadSmoke,
    /// Teleport effect.
    Teleport {
        /// Direction.
        direction: Q3TeleportDirection,
    },
    /// Weapon fire.
    WeaponFire,
    /// Out of ammo.
    OutOfAmmo,
    /// Gib player.
    GibPlayer,
    /// Stop looping sound.
    StopLoopingSound,
    /// Retained source event for its owner.
    SourceEvent(Q3CharacterEvent),
}

/// Event presentation options (`Q3CharacterEventOptions`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3CharacterEventOptions {
    /// Local seat.
    pub local: bool,
    /// Footsteps enabled.
    pub footsteps: bool,
    /// Predict steps.
    pub predict_steps: bool,
    /// Source flags.
    pub source_flags: i32,
}

/// Cgame random stream (`{ rand(): number }`).
pub trait Q3EventRandom {
    /// Next random value.
    fn rand(&mut self) -> i32;
}

/// Character event presenter (`Q3CharacterEventPresenter`).
#[derive(Debug)]
pub struct Q3CharacterEventPresenter<'a, R> {
    /// Step time.
    pub step_time: i32,
    /// Step change.
    pub step_change: f32,
    /// Land time.
    pub land_time: i32,
    /// Land change.
    pub land_change: f32,
    /// Muzzle flash time.
    pub muzzle_flash_time: i32,
    pose: &'a mut PlayerPoseState,
    footsteps: PlayerFootsteps,
    random: R,
}

impl<'a, R: Q3EventRandom> Q3CharacterEventPresenter<'a, R> {
    /// Bind a pose, footsteps, and random stream.
    pub fn new(pose: &'a mut PlayerPoseState, footsteps: PlayerFootsteps, random: R) -> Self {
        Self {
            step_time: 0,
            step_change: 0.0,
            land_time: 0,
            land_change: 0.0,
            muzzle_flash_time: -99999,
            pose,
            footsteps,
            random,
        }
    }

    /// Pain sound gating (`pain`).
    pub fn pain(&mut self, time_ms: i32, health: i32) -> Vec<Q3CharacterPresentationEffect> {
        if time_ms.wrapping_sub(self.pose.pain_time) < 500 {
            return Vec::new();
        }
        let level = if health < 25 {
            25
        } else if health < 50 {
            50
        } else if health < 75 {
            75
        } else {
            100
        };
        self.pose.pain_time = time_ms;
        self.pose.pain_direction = !self.pose.pain_direction;
        vec![Q3CharacterPresentationEffect::CustomSound {
            channel: Q3SoundChannel::Voice,
            name: format!("*pain{level}_1.wav"),
        }]
    }

    /// Present one source event (`event`).
    pub fn event(
        &mut self,
        source: &Q3CharacterEvent,
        options: &Q3CharacterEventOptions,
    ) -> Vec<Q3CharacterPresentationEffect> {
        use Q3CharacterPresentationEffect as Effect;
        let event = source.event & !0x300;
        let time = source.time_ms;
        match event {
            x if x == Q3EntityEvent::NONE => Vec::new(),
            x if x == Q3EntityEvent::FOOTSTEP
                || x == Q3EntityEvent::FOOTSTEP_METAL
                || x == Q3EntityEvent::FOOTSPLASH
                || x == Q3EntityEvent::FOOTWADE
                || x == Q3EntityEvent::SWIM =>
            {
                if !options.footsteps {
                    return Vec::new();
                }
                let material = if event == Q3EntityEvent::FOOTSTEP {
                    self.footsteps.into()
                } else if event == Q3EntityEvent::FOOTSTEP_METAL {
                    Q3StepMaterial::Metal
                } else {
                    Q3StepMaterial::Splash
                };
                vec![Effect::Footstep {
                    material,
                    variant: self.random.rand() & 3,
                }]
            }
            x if x == Q3EntityEvent::FALL_SHORT || x == Q3EntityEvent::FALL_MEDIUM || x == Q3EntityEvent::FALL_FAR => {
                if event == Q3EntityEvent::FALL_FAR {
                    self.pose.pain_time = time;
                }
                if options.local {
                    self.land_change = -8.0 * (event - Q3EntityEvent::FALL_SHORT + 1) as f32;
                    self.land_time = time;
                }
                if event == Q3EntityEvent::FALL_SHORT {
                    vec![Effect::Sound {
                        channel: Q3SoundChannel::Auto,
                        path: "sound/player/land1.wav".to_string(),
                    }]
                } else if event == Q3EntityEvent::FALL_MEDIUM {
                    vec![Effect::CustomSound {
                        channel: Q3SoundChannel::Voice,
                        name: "*pain100_1.wav".to_string(),
                    }]
                } else {
                    vec![Effect::CustomSound {
                        channel: Q3SoundChannel::Auto,
                        name: "*fall1.wav".to_string(),
                    }]
                }
            }
            x if x == Q3EntityEvent::STEP_4
                || x == Q3EntityEvent::STEP_8
                || x == Q3EntityEvent::STEP_12
                || x == Q3EntityEvent::STEP_16 =>
            {
                if !options.local || !options.predict_steps {
                    return Vec::new();
                }
                let elapsed = time.wrapping_sub(self.step_time);
                let previous = if elapsed < 200 {
                    self.step_change * 200_i32.wrapping_sub(elapsed) as f32 / 200.0
                } else {
                    0.0
                };
                self.step_change = (previous + 4.0 * (event - Q3EntityEvent::STEP_4 + 1) as f32).min(32.0);
                self.step_time = time;
                Vec::new()
            }
            x if x == Q3EntityEvent::JUMP_PAD => vec![
                Effect::JumpPadSmoke,
                Effect::Sound {
                    channel: Q3SoundChannel::Voice,
                    path: "sound/world/jumppad.wav".to_string(),
                },
                Effect::CustomSound {
                    channel: Q3SoundChannel::Voice,
                    name: "*jump1.wav".to_string(),
                },
            ],
            x if x == Q3EntityEvent::JUMP => vec![Effect::CustomSound {
                channel: Q3SoundChannel::Voice,
                name: "*jump1.wav".to_string(),
            }],
            x if x == Q3EntityEvent::TAUNT => vec![Effect::CustomSound {
                channel: Q3SoundChannel::Voice,
                name: "*taunt.wav".to_string(),
            }],
            x if x == Q3EntityEvent::WATER_TOUCH => vec![Effect::Sound {
                channel: Q3SoundChannel::Auto,
                path: "sound/player/watr_in.wav".to_string(),
            }],
            x if x == Q3EntityEvent::WATER_LEAVE => vec![Effect::Sound {
                channel: Q3SoundChannel::Auto,
                path: "sound/player/watr_out.wav".to_string(),
            }],
            x if x == Q3EntityEvent::WATER_UNDER => vec![Effect::Sound {
                channel: Q3SoundChannel::Auto,
                path: "sound/player/watr_un.wav".to_string(),
            }],
            x if x == Q3EntityEvent::WATER_CLEAR => vec![Effect::CustomSound {
                channel: Q3SoundChannel::Auto,
                name: "*gasp.wav".to_string(),
            }],
            x if x == Q3EntityEvent::NOAMMO => {
                if options.local {
                    vec![Effect::OutOfAmmo]
                } else {
                    Vec::new()
                }
            }
            x if x == Q3EntityEvent::CHANGE_WEAPON => vec![Effect::Sound {
                channel: Q3SoundChannel::Auto,
                path: "sound/weapons/change.wav".to_string(),
            }],
            x if x == Q3EntityEvent::FIRE_WEAPON => {
                self.muzzle_flash_time = time;
                vec![Effect::WeaponFire]
            }
            x if x == Q3EntityEvent::PLAYER_TELEPORT_IN => vec![
                Effect::Sound {
                    channel: Q3SoundChannel::Auto,
                    path: "sound/world/telein.wav".to_string(),
                },
                Effect::Teleport {
                    direction: Q3TeleportDirection::In,
                },
            ],
            x if x == Q3EntityEvent::PLAYER_TELEPORT_OUT => vec![
                Effect::Sound {
                    channel: Q3SoundChannel::Auto,
                    path: "sound/world/teleout.wav".to_string(),
                },
                Effect::Teleport {
                    direction: Q3TeleportDirection::Out,
                },
            ],
            x if x == Q3EntityEvent::PAIN => {
                if options.local {
                    Vec::new()
                } else {
                    self.pain(time, source.parameter)
                }
            }
            x if x == Q3EntityEvent::DEATH1 || x == Q3EntityEvent::DEATH2 || x == Q3EntityEvent::DEATH3 => {
                vec![Effect::CustomSound {
                    channel: Q3SoundChannel::Voice,
                    name: format!("*death{}.wav", event - Q3EntityEvent::DEATH1 + 1),
                }]
            }
            x if x == Q3EntityEvent::GIB_PLAYER => {
                if options.source_flags & 0x200 != 0 {
                    vec![Effect::GibPlayer]
                } else {
                    vec![
                        Effect::Sound {
                            channel: Q3SoundChannel::Body,
                            path: "sound/player/gibsplt1.wav".to_string(),
                        },
                        Effect::GibPlayer,
                    ]
                }
            }
            x if x == Q3EntityEvent::STOPLOOPINGSOUND => vec![Effect::StopLoopingSound],
            _ => vec![Effect::SourceEvent(source.clone())],
        }
    }
}

// ---------------------------------------------------------------------------
// weapon-behavior.ts: projectile roles.
// ---------------------------------------------------------------------------

/// Projectile behavior (`q3ProjectileBehavior` result).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3ProjectileBehavior {
    /// Weapon item.
    pub weapon: ItemId,
    /// Projectile role.
    pub role: ProjectileRole,
}

/// Projectile behavior for a weapon (`q3ProjectileBehavior`).
pub fn q3_projectile_behavior(weapon: i32) -> Result<Q3ProjectileBehavior, Q3FoundationError> {
    let item = q3_weapon_item(weapon).ok_or_else(|| failed(format!("Unknown Q3 projectile weapon {weapon}")))?;
    let role = if weapon == Q3Weapon::GRENADE_LAUNCHER || weapon == Q3Weapon::PROX_LAUNCHER {
        ProjectileRole::Grenade
    } else if weapon == Q3Weapon::ROCKET_LAUNCHER {
        ProjectileRole::Rocket
    } else if weapon == Q3Weapon::PLASMAGUN {
        ProjectileRole::Plasma
    } else if weapon == Q3Weapon::BFG {
        ProjectileRole::Energy
    } else if weapon == Q3Weapon::GRAPPLING_HOOK {
        ProjectileRole::Grapple
    } else if weapon == Q3Weapon::NAILGUN {
        ProjectileRole::Nail
    } else {
        return Err(failed(format!("Q3 weapon {weapon} does not launch a projectile")));
    };
    Ok(Q3ProjectileBehavior {
        weapon: item.item.to_string(),
        role,
    })
}

// ---------------------------------------------------------------------------
// held-weapons.ts: shared tag registration.
// ---------------------------------------------------------------------------

/// Native weapon hand grip in tag_weapon coordinates (`Q3_WEAPON_HAND_GRIP`).
pub const Q3_WEAPON_HAND_GRIP: ModelTransform = ModelTransform {
    origin: Vec3 {
        x: -2.9841071642362156f64 as f32,
        y: -0.7671715473899474f64 as f32,
        z: -2.208833547738882f64 as f32,
    },
    axis: [
        Vec3 { x: 1.0, y: 0.0, z: 0.0 },
        Vec3 { x: 0.0, y: 1.0, z: 0.0 },
        Vec3 { x: 0.0, y: 0.0, z: 1.0 },
    ],
    scale: Vec3 { x: 1.0, y: 1.0, z: 1.0 },
};

// ---------------------------------------------------------------------------
// assets.ts: CG_FindClientModelFile, CG_FindClientHeadFile,
// CG_RegisterClientModelname.
// ---------------------------------------------------------------------------

/// Character team.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3Team {
    /// Red.
    Red,
    /// Blue.
    Blue,
}

impl Q3Team {
    fn as_str(self) -> &'static str {
        match self {
            Q3Team::Red => "red",
            Q3Team::Blue => "blue",
        }
    }
}

/// Character selection (`Q3CharacterSelection`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3CharacterSelection {
    /// Model name.
    pub model: String,
    /// Skin name.
    pub skin: String,
    /// Head model name.
    pub head_model: String,
    /// Head skin name.
    pub head_skin: String,
    /// Team.
    pub team: Option<Q3Team>,
    /// Team name.
    pub team_name: String,
}

/// Character part (`Q3CharacterPart`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3CharacterPart {
    /// Mesh resource.
    pub resource: ResolvedResourceReference,
    /// Mesh model.
    pub model: SceneMd3,
    /// Skin resource.
    pub skin_resource: ResolvedResourceReference,
    /// Skin surfaces.
    pub surfaces: Vec<SkinSurface>,
}

/// Character assets (`Q3CharacterAssets`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3CharacterAssets {
    /// Selection.
    pub selection: Q3CharacterSelection,
    /// Lower part.
    pub lower: Q3CharacterPart,
    /// Upper part.
    pub upper: Q3CharacterPart,
    /// Head part.
    pub head: Q3CharacterPart,
    /// Animation resource.
    pub animation_resource: ResolvedResourceReference,
    /// Animation config.
    pub animation: PlayerAnimationConfig,
    /// Icon resource.
    pub icon: Option<ResolvedResourceReference>,
}

/// Character resource bytes (`Q3CharacterResources`, sync adaptation of the
/// donor async plan reader).
pub trait Q3CharacterResources {
    /// Open a resource path.
    fn open(&self, path: &str) -> Result<Option<OpenedResource>, Q3FoundationError>;
}

fn byte_text(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| *byte as char).collect()
}

fn validate_component(value: &str, name: &str, star: bool) -> Result<(), Q3FoundationError> {
    let plain = if star && value.starts_with('*') {
        &value[1..]
    } else {
        value
    };
    if plain.is_empty() || plain == "." || plain == ".." || plain.contains(['\0', '/', '\\']) {
        return Err(range(format!("Invalid Q3 {name}: {value}")));
    }
    Ok(())
}

fn first_resource(
    resources: &impl Q3CharacterResources,
    paths: &[String],
) -> Result<Option<OpenedResource>, Q3FoundationError> {
    for path in paths {
        let resource = resources.open(path)?;
        if resource.as_ref().is_some_and(|open| !open.bytes.is_empty()) {
            return Ok(resource);
        }
    }
    Ok(None)
}

fn required_resource(
    resources: &impl Q3CharacterResources,
    paths: &[String],
) -> Result<OpenedResource, Q3FoundationError> {
    first_resource(resources, paths)?
        .ok_or_else(|| failed(format!("Q3 character resource missing: {}", paths.join(", "))))
}

fn truncate_path(path: String, limit: usize) -> String {
    path.chars().take(limit).collect()
}

fn body_files(selection: &Q3CharacterSelection, base: &str, team_prefix: &str) -> Vec<String> {
    let team = selection.team.map_or("default", Q3Team::as_str);
    let fallback = match selection.team {
        Some(team) => team.as_str(),
        None => selection.skin.as_str(),
    };
    let mut paths = Vec::new();
    for folder in ["", "characters/"] {
        let prefixes: &[&str] = if team_prefix.is_empty() {
            &[""]
        } else {
            &[team_prefix, ""]
        };
        for prefix in prefixes {
            paths.push(truncate_path(
                format!(
                    "models/players/{folder}{}/{prefix}{base}_{}_{team}.skin",
                    selection.model, selection.skin
                ),
                63,
            ));
            paths.push(truncate_path(
                format!(
                    "models/players/{folder}{}/{prefix}{base}_{fallback}.skin",
                    selection.model
                ),
                63,
            ));
        }
    }
    paths
}

fn head_files(selection: &Q3CharacterSelection, base: &str, extension: &str, team_prefix: &str) -> Vec<String> {
    let model = if selection.head_model.is_empty() {
        selection.model.as_str()
    } else {
        selection.head_model.as_str()
    };
    let name = model.strip_prefix('*').unwrap_or(model);
    let team = selection.team.map_or("default", Q3Team::as_str);
    let fallback = match selection.team {
        Some(team) => team.as_str(),
        None => selection.head_skin.as_str(),
    };
    let limit = if base == "head" { 63 } else { 127 };
    let folders: &[&str] = if model.starts_with('*') {
        &["heads/"]
    } else {
        &["", "heads/"]
    };
    let mut paths = Vec::new();
    for folder in folders {
        let prefixes: &[&str] = if team_prefix.is_empty() {
            &[""]
        } else {
            &[team_prefix, ""]
        };
        for prefix in prefixes {
            paths.push(truncate_path(
                format!(
                    "models/players/{folder}{name}/{}/{prefix}{base}_{team}.{extension}",
                    selection.head_skin
                ),
                limit,
            ));
            paths.push(truncate_path(
                format!("models/players/{folder}{name}/{prefix}{base}_{fallback}.{extension}"),
                limit,
            ));
        }
    }
    paths
}

/// Load character assets (`loadQ3Character`).
pub fn load_q3_character(
    resources: &impl Q3CharacterResources,
    selection: &Q3CharacterSelection,
) -> Result<Q3CharacterAssets, Q3FoundationError> {
    validate_component(&selection.model, "model", false)?;
    validate_component(&selection.skin, "skin", false)?;
    if !selection.head_model.is_empty() {
        validate_component(&selection.head_model, "head model", true)?;
    }
    validate_component(&selection.head_skin, "head skin", false)?;
    if !selection.team_name.is_empty() {
        validate_component(&selection.team_name, "team name", false)?;
    }
    let model = selection.model.as_str();
    let head = if selection.head_model.is_empty() {
        model
    } else {
        selection.head_model.as_str()
    };
    let head_name = head.strip_prefix('*').unwrap_or(head);
    let lower = required_resource(
        resources,
        &[
            format!("models/players/{model}/lower.md3"),
            format!("models/players/characters/{model}/lower.md3"),
        ],
    )?;
    let upper = required_resource(
        resources,
        &[
            format!("models/players/{model}/upper.md3"),
            format!("models/players/characters/{model}/upper.md3"),
        ],
    )?;
    let face_paths: Vec<String> = if head.starts_with('*') {
        vec![format!("models/players/heads/{head_name}/{head_name}.md3")]
    } else {
        vec![
            format!("models/players/{head}/head.md3"),
            format!("models/players/heads/{head_name}/{head_name}.md3"),
        ]
    };
    let face = required_resource(resources, &face_paths)?;
    let animation = required_resource(
        resources,
        &[
            format!("models/players/{model}/animation.cfg"),
            format!("models/players/characters/{model}/animation.cfg"),
        ],
    )?;
    let team_names: Vec<String> = if selection.team_name.is_empty() {
        vec![String::new()]
    } else {
        vec![
            format!("{}/", selection.team_name),
            if selection.team == Some(Q3Team::Blue) {
                "Pagans/".to_string()
            } else {
                "Stroggs/".to_string()
            },
        ]
    };
    let mut skins: Option<[OpenedResource; 3]> = None;
    for team in &team_names {
        let legs = first_resource(resources, &body_files(selection, "lower", team))?;
        let torso = first_resource(resources, &body_files(selection, "upper", team))?;
        let head_skin = first_resource(resources, &head_files(selection, "head", "skin", team))?;
        if let (Some(legs), Some(torso), Some(head_skin)) = (legs, torso, head_skin) {
            skins = Some([legs, torso, head_skin]);
            break;
        }
    }
    let Some(skins) = skins else {
        return Err(failed(format!(
            "Q3 character skin missing: {}/{}, {}/{}",
            selection.model, selection.skin, head, selection.head_skin
        )));
    };
    let [legs_skin, torso_skin, head_skin] = skins;
    let make_part = |mesh: OpenedResource, skin: OpenedResource| -> Result<Q3CharacterPart, Q3FoundationError> {
        Ok(Q3CharacterPart {
            resource: mesh.reference.clone(),
            model: to_scene_md3(parse_md3(&mesh.bytes, &mesh.reference.requested_path)?.model),
            skin_resource: skin.reference.clone(),
            surfaces: parse_skin(&byte_text(&skin.bytes))?,
        })
    };
    let icon_prefix = if selection.team_name.is_empty() {
        String::new()
    } else {
        format!("{}/", selection.team_name)
    };
    let mut icon_paths = head_files(selection, "icon", "skin", &icon_prefix);
    icon_paths.extend(head_files(selection, "icon", "tga", &icon_prefix));
    let icon = first_resource(resources, &icon_paths)?;
    let animation_text = byte_text(&animation.bytes);
    let animation_path = animation.reference.requested_path.clone();
    Ok(Q3CharacterAssets {
        selection: selection.clone(),
        lower: make_part(lower, legs_skin)?,
        upper: make_part(upper, torso_skin)?,
        head: make_part(face, head_skin)?,
        animation_resource: animation.reference.clone(),
        animation: parse_player_animation_config(&animation_text, &animation_path)?,
        icon: icon.map(|open| open.reference),
    })
}

// ---------------------------------------------------------------------------
// presentation.ts: CG_Player, CG_PlayerAnimation, powerup passes.
// ---------------------------------------------------------------------------

/// Presented model (the `q3-md3` and `md5` arms of `DecodedModel` the
/// attachment lookup supports).
#[derive(Debug, Clone, PartialEq)]
pub enum Q3PresentedModel {
    /// MD3 scene model.
    Md3(SceneMd3),
    /// MD5 joints and frames.
    Md5 {
        /// Joints.
        joints: Vec<Md5Joint>,
        /// Frames.
        frames: Vec<Md5AnimationFrame>,
    },
}

/// Presented model pose (`ModelPose`, foundation arms).
#[derive(Debug, Clone, PartialEq)]
pub enum Q3ModelPose {
    /// Frame interpolation.
    Frame {
        /// Current frame.
        frame: i32,
        /// Previous frame.
        previous_frame: i32,
        /// Blend factor.
        back_lerp: f32,
    },
    /// Skeleton joints.
    Skeleton {
        /// Joints.
        joints: Vec<SkeletonJointPose>,
    },
}

/// Attached entity (`SceneEntity` attachment entry).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3Attachment {
    /// Tag name.
    pub tag: String,
    /// Attached entity.
    pub entity: Box<Q3SceneEntity>,
}

/// Presented scene entity (`SceneEntity`, foundation fields).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3SceneEntity {
    /// Actor.
    pub actor: Option<ActorId>,
    /// Resource.
    pub resource: ResolvedResourceReference,
    /// Model.
    pub model: Q3PresentedModel,
    /// Opacity.
    pub opacity: f32,
    /// Transform.
    pub transform: ModelTransform,
    /// Previous origin.
    pub previous_origin: Vec3,
    /// Pose.
    pub pose: Q3ModelPose,
    /// Skin index.
    pub skin: i32,
    /// Color.
    pub color: Vec4,
    /// Shader time.
    pub shader_time: SourceTime,
    /// Q3 flag bits.
    pub flags: i32,
    /// Lighting origin.
    pub lighting_origin: Vec3,
    /// Shadow plane.
    pub shadow_plane: f32,
    /// Attachments.
    pub attachments: Vec<Q3Attachment>,
}

/// Resolved attachment tag (`ModelTag` with scale).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3AttachmentTag {
    /// Name.
    pub name: String,
    /// Origin.
    pub origin: Vec3,
    /// Axis.
    pub axis: Axis,
    /// Scale.
    pub scale: f32,
}

fn model_world_direction(transform: &ModelTransform, value: Vec3) -> Vec3 {
    let x = value.x * transform.scale.x;
    let y = value.y * transform.scale.y;
    let z = value.z * transform.scale.z;
    let [forward, left, up] = transform.axis;
    vec3(
        forward.x * x + left.x * y + up.x * z,
        forward.y * x + left.y * y + up.y * z,
        forward.z * x + left.z * y + up.z * z,
    )
}

fn model_world_point(transform: &ModelTransform, value: Vec3) -> Vec3 {
    let direction = model_world_direction(transform, value);
    vec3(
        transform.origin.x + direction.x,
        transform.origin.y + direction.y,
        transform.origin.z + direction.z,
    )
}

fn compose_model_transform(parent: &ModelTransform, child: &ModelTransform) -> ModelTransform {
    ModelTransform {
        origin: model_world_point(parent, child.origin),
        axis: [
            model_world_direction(parent, scale3(child.axis[0], child.scale.x)),
            model_world_direction(parent, scale3(child.axis[1], child.scale.y)),
            model_world_direction(parent, scale3(child.axis[2], child.scale.z)),
        ],
        scale: vec3(1.0, 1.0, 1.0),
    }
}

/// Resolve a named attachment tag (`modelAttachmentTag`).
pub fn model_attachment_tag(entity: &Q3SceneEntity, name: &str) -> Result<Option<Q3AttachmentTag>, Q3FoundationError> {
    match &entity.model {
        Q3PresentedModel::Md3(scene) => {
            let Q3ModelPose::Frame {
                frame,
                previous_frame,
                back_lerp,
            } = &entity.pose
            else {
                return Ok(None);
            };
            if scene.tags.is_empty() || scene.frames.is_empty() {
                return Ok(None);
            }
            if !back_lerp.is_finite() {
                return Err(range("MD3 tag fraction must be finite"));
            }
            let last = scene.frames.len() - 1;
            let previous_index = (*previous_frame).min(last as i32);
            let current_index = (*frame).min(last as i32);
            if previous_index < 0 || current_index < 0 {
                return Ok(None);
            }
            let first = scene
                .tags
                .get(previous_index as usize)
                .and_then(|tags| tags.iter().find(|tag| tag.name == name));
            let second = scene
                .tags
                .get(current_index as usize)
                .and_then(|tags| tags.iter().find(|tag| tag.name == name));
            let (Some(first), Some(second)) = (first, second) else {
                return Ok(None);
            };
            let tag = interpolate_md3_tags(
                &Md3Tag {
                    name: first.name.clone(),
                    origin: first.origin,
                    axes: first.axis,
                },
                &Md3Tag {
                    name: second.name.clone(),
                    origin: second.origin,
                    axes: second.axis,
                },
                name,
                1.0 - *back_lerp,
            );
            Ok(Some(Q3AttachmentTag {
                name: name.to_string(),
                origin: tag.origin,
                axis: tag.axes,
                scale: 1.0,
            }))
        }
        Q3PresentedModel::Md5 { joints, frames } => {
            let Some(index) = joints.iter().position(|joint| joint.name == name) else {
                return Ok(None);
            };
            let poses: Vec<SkeletonJointPose> = match &entity.pose {
                Q3ModelPose::Skeleton { joints } => joints.clone(),
                Q3ModelPose::Frame {
                    frame,
                    previous_frame,
                    back_lerp,
                } => {
                    if *frame < 0 || *previous_frame < 0 || !back_lerp.is_finite() || frames.is_empty() {
                        return Err(range("Invalid MD5 frame selection"));
                    }
                    sample_md5_pose(frames, *frame, *previous_frame, *back_lerp)
                }
            };
            let Some(joint) = poses.get(index) else {
                return Ok(None);
            };
            let tag = joint_attachment_tag(name, joint);
            Ok(Some(Q3AttachmentTag {
                name: tag.name,
                origin: tag.origin,
                axis: tag.axis,
                scale: tag.scale,
            }))
        }
    }
}

/// Attach a child entity at a tag (`attachSceneEntity`).
#[must_use]
pub fn attach_scene_entity(parent: &Q3SceneEntity, child: &Q3SceneEntity, tag: &Q3AttachmentTag) -> Q3SceneEntity {
    let tag_transform = compose_model_transform(
        &parent.transform,
        &ModelTransform {
            origin: tag.origin,
            axis: tag.axis,
            scale: vec3(tag.scale, tag.scale, tag.scale),
        },
    );
    let transform = compose_model_transform(&tag_transform, &child.transform);
    let delta = model_world_direction(&tag_transform, sub3(child.previous_origin, child.transform.origin));
    Q3SceneEntity {
        transform,
        previous_origin: add3(transform.origin, delta),
        lighting_origin: parent.lighting_origin,
        ..child.clone()
    }
}

/// Character view (`Q3CharacterView`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3CharacterView {
    /// Actor.
    pub actor: ActorId,
    /// Origin.
    pub origin: Vec3,
    /// Angles.
    pub angles: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Movement direction.
    pub movement_direction: f32,
    /// Animation.
    pub animation: Q3AnimationState,
    /// Source flags.
    pub source_flags: i32,
    /// Powerups bitmask.
    pub powerups: i32,
    /// Team.
    pub team: Option<Q3Team>,
    /// Color.
    pub color: Vec4,
    /// Visual scale.
    pub scale: Option<f32>,
    /// Opacity.
    pub opacity: Option<f32>,
}

/// Model source options the character passes set (`ModelSourceOptions`
/// fields used by the foundation).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ModelSourceOptions {
    /// Custom shader override.
    pub custom_shader: Option<String>,
    /// Custom skin surfaces.
    pub custom_skin: Option<Vec<SkinSurface>>,
}

/// Pass option resolver (`Q3CharacterPass` options closure).
#[derive(Clone)]
pub enum Q3PassOptions {
    /// Resolve from character parts.
    Character,
    /// Foreign resolver, preserved across attachment.
    Foreign(Rc<dyn Fn(&Q3SceneEntity) -> ModelSourceOptions>),
}

/// Character render pass (`Q3CharacterPass`).
#[derive(Clone)]
pub struct Q3CharacterPass {
    /// Content.
    pub content: Option<ContentId>,
    /// Entity.
    pub entity: Q3SceneEntity,
    /// Shader override.
    pub shader: Option<String>,
    /// Option resolver.
    pub options: Q3PassOptions,
}

impl Q3CharacterPass {
    /// Build a foreign (weapon) pass.
    pub fn weapon_pass(
        content: Option<ContentId>,
        entity: Q3SceneEntity,
        shader: Option<String>,
        options: Rc<dyn Fn(&Q3SceneEntity) -> ModelSourceOptions>,
    ) -> Self {
        Self {
            content,
            entity,
            shader,
            options: Q3PassOptions::Foreign(options),
        }
    }

    /// Resolve model source options for an entity.
    #[must_use]
    pub fn options(&self, assets: &Q3CharacterAssets, entity: &Q3SceneEntity) -> ModelSourceOptions {
        match &self.options {
            Q3PassOptions::Foreign(resolve) => resolve(entity),
            Q3PassOptions::Character => {
                let mut resolved = ModelSourceOptions::default();
                let part = [&assets.lower, &assets.upper, &assets.head]
                    .into_iter()
                    .find(|part| part.resource.id == entity.resource.id);
                if let Some(part) = part {
                    resolved.custom_skin = Some(part.surfaces.clone());
                }
                if self.shader.is_some() {
                    resolved.custom_shader = self.shader.clone();
                }
                resolved
            }
        }
    }
}

/// Character render options (`Q3CharacterRenderOptions`).
#[derive(Clone)]
pub struct Q3CharacterRenderOptions {
    /// Time in milliseconds.
    pub time_ms: i32,
    /// Frame time in milliseconds.
    pub frame_ms: i32,
    /// Shader time.
    pub shader_time: SourceTime,
    /// Swing speed.
    pub swing_speed: f32,
    /// Freeze animations.
    pub no_player_animations: bool,
    /// Personal model flag.
    pub personal_model: bool,
    /// Shadow plane.
    pub shadow_plane: Option<f32>,
    /// Prepared weapon passes in tag_weapon coordinates.
    pub weapon: Vec<Q3CharacterPass>,
}

/// Character presenter (`Q3CharacterPresenter`).
#[derive(Debug, Clone)]
pub struct Q3CharacterPresenter {
    /// Pose state.
    pub pose: PlayerPoseState,
    /// Assets.
    pub assets: Q3CharacterAssets,
}

impl Q3CharacterPresenter {
    /// Bind assets.
    #[must_use]
    pub fn new(assets: Q3CharacterAssets) -> Self {
        Self {
            pose: create_player_pose_state(),
            assets,
        }
    }

    /// Reset to a view (`reset`).
    pub fn reset(&mut self, view: &Q3CharacterView, time_ms: i32) -> Result<(), Q3FoundationError> {
        clear_lerp_frame(
            &self.assets.animation,
            &mut self.pose.legs.lerp,
            view.animation.legs,
            time_ms,
            None,
        )?;
        clear_lerp_frame(
            &self.assets.animation,
            &mut self.pose.torso.lerp,
            view.animation.torso,
            time_ms,
            None,
        )?;
        self.pose.legs.lerp = create_lerp_frame();
        self.pose.legs.yaw_angle = view.angles.y;
        self.pose.legs.yawing = false;
        self.pose.legs.pitch_angle = 0.0;
        self.pose.legs.pitching = false;
        self.pose.torso.lerp = create_lerp_frame();
        self.pose.torso.yaw_angle = view.angles.y;
        self.pose.torso.yawing = false;
        self.pose.torso.pitch_angle = view.angles.x;
        self.pose.torso.pitching = false;
        Ok(())
    }

    /// Build render passes (`frame`).
    pub fn frame(
        &mut self,
        view: &Q3CharacterView,
        options: &Q3CharacterRenderOptions,
    ) -> Result<Vec<Q3CharacterPass>, Q3FoundationError> {
        if view.source_flags & 0x80 != 0 {
            return Ok(Vec::new());
        }
        let axes = calculate_player_pose(
            &mut self.pose,
            &CalculatePlayerPoseInput {
                entity: PoseEntityState {
                    e_flags: view.source_flags,
                    velocity: view.velocity,
                    movement_direction: view.movement_direction,
                    legs_anim: view.animation.legs,
                    torso_anim: view.animation.torso,
                },
                fixed_legs: self.assets.animation.fixed_legs,
                fixed_torso: self.assets.animation.fixed_torso,
                lerp_angles: view.angles,
                time_ms: options.time_ms,
                frame_time_ms: options.frame_ms,
                swing_speed: options.swing_speed,
            },
        )?;
        let speed_scale = if view.powerups & (1 << Q3Powerup::HASTE) != 0 {
            1.5
        } else {
            1.0
        };
        let legs_animation =
            if self.pose.legs.yawing && view.animation.legs & !ANIMATION_TOGGLE_BIT == Q3PlayerAnimation::LEGS_IDLE {
                Q3PlayerAnimation::LEGS_TURN
            } else {
                view.animation.legs
            };
        run_lerp_frame(
            &self.assets.animation,
            &mut self.pose.legs.lerp,
            &RunLerpFrameInput {
                time_ms: options.time_ms,
                new_animation: legs_animation,
                speed_scale,
                no_player_animations: options.no_player_animations,
            },
            None,
        )?;
        run_lerp_frame(
            &self.assets.animation,
            &mut self.pose.torso.lerp,
            &RunLerpFrameInput {
                time_ms: options.time_ms,
                new_animation: view.animation.torso,
                speed_scale,
                no_player_animations: options.no_player_animations,
            },
            None,
        )?;
        let flags = 0x80
            | (if options.personal_model { 2 } else { 0 })
            | (if options.shadow_plane.is_some() { 0x40 } else { 0 });
        let unit = vec3(1.0, 1.0, 1.0);
        let zero = vec3(0.0, 0.0, 0.0);
        let scale_value = view.scale.unwrap_or(1.0);
        let part = |asset: &Q3CharacterPart,
                    axis: Axis,
                    origin: Vec3,
                    frame: Option<&LerpFrame>,
                    attachments: Vec<Q3Attachment>,
                    scaled: bool|
         -> Q3SceneEntity {
            Q3SceneEntity {
                actor: Some(view.actor.clone()),
                resource: asset.resource.clone(),
                model: Q3PresentedModel::Md3(asset.model.clone()),
                opacity: view.opacity.unwrap_or(1.0),
                transform: ModelTransform {
                    origin,
                    axis,
                    scale: if scaled {
                        vec3(scale_value, scale_value, scale_value)
                    } else {
                        unit
                    },
                },
                previous_origin: origin,
                pose: Q3ModelPose::Frame {
                    frame: frame.map_or(0, |lerp| lerp.frame),
                    previous_frame: frame.map_or(0, |lerp| lerp.old_frame),
                    back_lerp: frame.map_or(0.0, |lerp| lerp.back_lerp),
                },
                skin: 0,
                color: view.color,
                shader_time: options.shader_time,
                flags,
                lighting_origin: view.origin,
                shadow_plane: options.shadow_plane.unwrap_or(0.0),
                attachments,
            }
        };
        let head = part(&self.assets.head, axes.head, zero, None, Vec::new(), false);
        let torso = part(
            &self.assets.upper,
            axes.torso,
            zero,
            Some(&self.pose.torso.lerp),
            vec![Q3Attachment {
                tag: "tag_head".to_string(),
                entity: Box::new(head),
            }],
            false,
        );
        let legs = part(
            &self.assets.lower,
            axes.legs,
            view.origin,
            Some(&self.pose.legs.lerp),
            vec![Q3Attachment {
                tag: "tag_torso".to_string(),
                entity: Box::new(torso.clone()),
            }],
            true,
        );
        let pass = |shader: Option<&str>| Q3CharacterPass {
            content: None,
            entity: legs.clone(),
            shader: shader.map(str::to_string),
            options: Q3PassOptions::Character,
        };
        let mut passes = if view.powerups & (1 << Q3Powerup::INVIS) != 0 {
            vec![pass(Some("powerups/invisibility"))]
        } else {
            vec![pass(None)]
        };
        if view.powerups & (1 << Q3Powerup::INVIS) == 0 {
            if view.powerups & (1 << Q3Powerup::QUAD) != 0 {
                passes.push(pass(Some(if view.team == Some(Q3Team::Red) {
                    "powerups/blueflag"
                } else {
                    "powerups/quad"
                })));
            }
            if view.powerups & (1 << Q3Powerup::REGEN) != 0 && options.time_ms / 100 % 10 == 1 {
                passes.push(pass(Some("powerups/regen")));
            }
            if view.powerups & (1 << Q3Powerup::BATTLESUIT) != 0 {
                passes.push(pass(Some("powerups/battleSuit")));
            }
        }
        if let Some(torso_tag) = model_attachment_tag(&legs, "tag_torso")? {
            let world_torso = attach_scene_entity(&legs, &torso, &torso_tag);
            if let Some(weapon_tag) = model_attachment_tag(&world_torso, "tag_weapon")? {
                for weapon in &options.weapon {
                    passes.push(Q3CharacterPass {
                        content: weapon.content.clone(),
                        entity: attach_scene_entity(&world_torso, &weapon.entity, &weapon_tag),
                        shader: weapon.shader.clone(),
                        options: weapon.options.clone(),
                    });
                }
            }
        }
        Ok(passes)
    }
}

/// Identity axis (`q3IdentityAxis`).
#[must_use]
pub fn q3_identity_axis() -> Axis {
    qvm_angles_to_axis(vec3(0.0, 0.0, 0.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::collections::HashMap;

    use crate::contract::{
        ContentDigest, LooseMount, MountId, MountIdentity, MountPlanId, ResourceId, ResourceProvenance,
        ResourceResolution,
    };
    use crate::md3::Md3Model;
    use crate::q3scene::{SceneMd3Frame, SceneMd3Tag};
    use qa_core::identity::{IdentityOwner, SavedActorId};
    use qa_core::time::FramePhase;

    fn animation_fixture() -> String {
        let mut text = String::from("sex f\nfootsteps boot\nheadoffset 1 2 3\nfixedlegs\nfixedtorso\n");
        for frame in 0..31 {
            text.push_str(&format!("{frame} 6 0 10\n"));
        }
        text
    }

    fn test_frame(elapsed: SourceTime) -> FrameContext {
        FrameContext {
            frame: 1,
            time: SourceTime::Milliseconds(100),
            elapsed,
            phase: FramePhase::FrameEntry,
        }
    }

    fn test_actor() -> (OwnedActor, ProviderId) {
        let owner = IdentityOwner::create("test").unwrap();
        let provider = ProviderId::new("q3", "test");
        let owned = owner.owned_actor(&owner.actor(3, 1), provider.clone()).unwrap();
        (owned, provider)
    }

    fn actor_key(actor: &ActorId) -> SavedActorId {
        SavedActorId::from(actor)
    }

    fn dummy_reference(path: &str, len: usize) -> ResolvedResourceReference {
        ResolvedResourceReference {
            id: ResourceId(format!("resource:test:{path}")),
            requested_path: path.to_string(),
            provenance: ResourceProvenance::Loose {
                mount: LooseMount {
                    identity: MountIdentity {
                        id: MountId("mount:test:loose".to_string()),
                        content: ContentId("q3:test:pkg:1".to_string()),
                        generation: 0,
                    },
                    root_path: "/test".to_string(),
                },
                member_path: path.to_string(),
            },
            digest: ContentDigest("sha256:00".to_string()),
            byte_length: len as u64,
            resolution: ResourceResolution::DefaultOrder {
                plan: MountPlanId("mount-plan:test:p".to_string()),
                rank: 0,
            },
        }
    }

    fn empty_scene(name: &str) -> SceneMd3 {
        SceneMd3 {
            name: name.to_string(),
            source_model: Md3Model {
                name: name.to_string(),
                flags: 0,
                skin_count: 0,
                frames: Vec::new(),
                tags: Vec::new(),
                surfaces: Vec::new(),
            },
            frames: Vec::new(),
            tags: Vec::new(),
            surfaces: Vec::new(),
        }
    }

    fn dummy_part(name: &str, shader: &str) -> Q3CharacterPart {
        Q3CharacterPart {
            resource: dummy_reference(&format!("models/{name}.md3"), 8),
            model: empty_scene(name),
            skin_resource: dummy_reference(&format!("models/{name}.skin"), 8),
            surfaces: vec![SkinSurface {
                name: name.to_string(),
                shader: shader.to_string(),
            }],
        }
    }

    fn dummy_assets() -> Q3CharacterAssets {
        Q3CharacterAssets {
            selection: Q3CharacterSelection {
                model: "sarge".to_string(),
                skin: "default".to_string(),
                head_model: String::new(),
                head_skin: "default".to_string(),
                team: None,
                team_name: String::new(),
            },
            lower: dummy_part("lower", "models/lower"),
            upper: dummy_part("upper", "models/upper"),
            head: dummy_part("head", "models/head"),
            animation_resource: dummy_reference("models/animation.cfg", 8),
            animation: parse_player_animation_config(&animation_fixture(), "<test>").unwrap(),
            icon: None,
        }
    }

    #[test]
    fn parses_animation_config_fixture() {
        let config = parse_player_animation_config(&animation_fixture(), "<test>").unwrap();
        assert_eq!(config.footsteps, PlayerFootsteps::Boot);
        assert_eq!(config.gender, PlayerGender::Female);
        assert_eq!(config.head_offset, vec3(1.0, 2.0, 3.0));
        assert!(config.fixed_legs);
        assert!(config.fixed_torso);
        assert!(config.warnings.is_empty());
        let death = config.animations[0].as_ref().unwrap();
        assert_eq!(death.first_frame, 0);
        assert_eq!(death.num_frames, 6);
        assert_eq!(death.frame_lerp, 100);
        assert_eq!(death.initial_lerp, 100);
        assert!(config.animations[31].is_none());
        let walk_cr = config.animations[13].as_ref().unwrap();
        assert_eq!(walk_cr.first_frame, 6);
        let back_cr = config.animations[32].as_ref().unwrap();
        assert_eq!(back_cr.first_frame, 6);
        assert!(back_cr.reversed);
        let back_walk = config.animations[33].as_ref().unwrap();
        assert_eq!(
            back_walk.first_frame,
            config.animations[14].as_ref().unwrap().first_frame
        );
        assert!(back_walk.reversed);
        assert_eq!(
            config.animations[34].as_ref().unwrap(),
            &Animation {
                first_frame: 0,
                num_frames: 16,
                loop_frames: 16,
                frame_lerp: 66,
                initial_lerp: 66,
                reversed: false,
                flipflop: false,
            }
        );
        assert_eq!(config.animations[35].as_ref().unwrap().first_frame, 16);
        assert!(config.animations[36].as_ref().unwrap().reversed);
    }

    #[test]
    fn animation_config_directives_warnings_and_errors() {
        let mut text = String::from("sex n\nfootsteps squeak\nmystery\n");
        for frame in 0..31 {
            text.push_str(&format!("{frame} 6 0 10\n"));
        }
        let config = parse_player_animation_config(&text, "<test>").unwrap();
        assert_eq!(config.gender, PlayerGender::Neuter);
        assert_eq!(config.warnings.len(), 2);
        assert!(config.warnings[0].message.contains("Bad footsteps"));
        assert!(config.warnings[1].message.contains("unknown token"));

        let err = parse_player_animation_config("", "<s>").unwrap_err();
        assert!(matches!(err, Q3FoundationError::Parse { line: 1, .. }));

        let long = "x".repeat(19_999);
        assert!(parse_player_animation_config(&long, "<s>").is_err());

        let mut short = String::from("sex m\n");
        for frame in 0..20 {
            short.push_str(&format!("{frame} 6 0 10\n"));
        }
        let err = parse_player_animation_config(&short, "<s>").unwrap_err();
        assert!(matches!(err, Q3FoundationError::Parse { .. }));

        let err = parse_player_animation_config("foo", "<s>").unwrap_err();
        assert!(matches!(err, Q3FoundationError::Range(_)));
    }

    #[test]
    fn animation_config_row_edge_cases() {
        let mut text = String::new();
        for frame in 0..31 {
            if frame == 0 {
                text.push_str("5 -4 2 0\n");
            } else {
                text.push_str(&format!("{frame} 6 0 10\n"));
            }
        }
        let config = parse_player_animation_config(&text, "<t>").unwrap();
        let first = config.animations[0].as_ref().unwrap();
        assert_eq!(first.num_frames, 4);
        assert!(first.reversed);
        assert_eq!(first.frame_lerp, 1000);

        let mut partial = String::new();
        for frame in 0..25 {
            partial.push_str(&format!("{frame} 6 0 10\n"));
        }
        let config = parse_player_animation_config(&partial, "<t>").unwrap();
        let gesture = config.animations[6].as_ref().unwrap().clone();
        for index in 25..31 {
            let row = config.animations[index].as_ref().unwrap();
            assert_eq!(row.first_frame, gesture.first_frame);
            assert!(!row.reversed);
        }

        let mut target = PlayerAnimationTarget::default();
        let mut parser = CommonParseState::new();
        let mut printed = Vec::new();
        let mut sink = |message: &str| printed.push(message.to_string());
        let config = parse_player_animation_config_into(
            &mut target,
            &mut parser,
            "bogus\n0 6 0 10\n1 6 0 10\n2 6 0 10\n3 6 0 10\n4 6 0 10\n5 6 0 10\n6 6 0 10\n7 6 0 10\n8 6 0 10\n9 6 0 10\n10 6 0 10\n11 6 0 10\n12 6 0 10\n13 6 0 10\n14 6 0 10\n15 6 0 10\n16 6 0 10\n17 6 0 10\n18 6 0 10\n19 6 0 10\n20 6 0 10\n21 6 0 10\n22 6 0 10\n23 6 0 10\n24 6 0 10\n25 6 0 10\n26 6 0 10\n27 6 0 10\n28 6 0 10\n29 6 0 10\n30 6 0 10\n",
            "<t>",
            Some(&mut sink),
        )
        .unwrap();
        assert_eq!(printed.len(), 1);
        assert!(printed[0].contains("unknown token"));
        assert_eq!(config.warnings.len(), 1);
        assert_eq!(target.animations[0].first_frame, 0);
    }

    #[test]
    fn com_parse_tokens_comments_and_limits() {
        let mut parser = CommonParseState::new();
        let mut cursor = CommonParseCursor::new("hello // rest\n\"quoted token\" /* block */ word").unwrap();
        assert_eq!(parser.parse(&mut cursor).unwrap(), "hello");
        assert_eq!(parser.parse(&mut cursor).unwrap(), "quoted token");
        assert_eq!(parser.parse(&mut cursor).unwrap(), "word");
        assert_eq!(parser.parse(&mut cursor).unwrap(), "");
        assert_eq!(parser.line(), 1);

        let mut parser = CommonParseState::new();
        let mut cursor = CommonParseCursor::new(&"w".repeat(2000)).unwrap();
        assert_eq!(parser.parse(&mut cursor).unwrap(), "");

        let mut parser = CommonParseState::new();
        let mut cursor = CommonParseCursor::new(&format!("\"{}\"", "q".repeat(1024))).unwrap();
        assert!(parser.parse(&mut cursor).is_err());

        assert!(CommonParseCursor::new("héllo \u{0100}").is_err());
    }

    #[test]
    fn game_numbers_match_bg_lib() {
        assert_eq!(game_atof("3.5").unwrap(), 3.5);
        assert_eq!(game_atof("  -12x").unwrap(), -12.0);
        assert_eq!(game_atof("abc").unwrap(), 0.0);
        assert_eq!(game_atof(".5").unwrap(), 0.5);
        assert_eq!(game_atof("").unwrap(), 0.0);
        assert_eq!(game_atoi("  +42 ").unwrap(), 42);
        assert_eq!(game_atoi("-7up").unwrap(), -7);
        assert_eq!(game_atoi("2147483648").unwrap(), i32::MIN);
        assert_eq!(game_atoi("").unwrap(), 0);
        assert!(game_atof("ÿ\u{0100}").is_err());
    }

    #[test]
    fn lerp_frame_selects_advances_and_resets() {
        let config = parse_player_animation_config(&animation_fixture(), "<t>").unwrap();
        let mut state = create_lerp_frame();
        set_lerp_frame_animation(&config, &mut state, 129, None).unwrap();
        assert_eq!(state.animation_number, 129);
        assert_eq!(
            state.current_animation.unwrap().first_frame,
            config.animations[1].as_ref().unwrap().first_frame
        );
        assert!(set_lerp_frame_animation(&config, &mut state, 99, None).is_err());
        assert!(set_lerp_frame_animation(&config, &mut state, 31, None).is_err());

        run_lerp_frame(
            &config,
            &mut state,
            &RunLerpFrameInput {
                time_ms: 500,
                new_animation: 129,
                speed_scale: 1.0,
                no_player_animations: false,
            },
            None,
        )
        .unwrap();
        assert!(state.frame >= state.old_frame);
        assert!((0.0..=1.0).contains(&state.back_lerp));

        run_lerp_frame(
            &config,
            &mut state,
            &RunLerpFrameInput {
                time_ms: 600,
                new_animation: 129,
                speed_scale: 1.0,
                no_player_animations: true,
            },
            None,
        )
        .unwrap();
        assert_eq!((state.frame, state.back_lerp), (0, 0.0));

        assert!(run_lerp_frame(
            &config,
            &mut state,
            &RunLerpFrameInput {
                time_ms: 600,
                new_animation: 129,
                speed_scale: f32::NAN,
                no_player_animations: false,
            },
            None,
        )
        .is_err());

        clear_lerp_frame(&config, &mut state, 0, 700, None).unwrap();
        assert_eq!(state.frame, 0);
        assert_eq!(state.frame_time, 700);
    }

    #[test]
    fn lerp_frame_loops_and_reverses() {
        let mut config = parse_player_animation_config(&animation_fixture(), "<t>").unwrap();
        config.animations[0] = Some(Animation {
            first_frame: 10,
            num_frames: 4,
            loop_frames: 2,
            frame_lerp: 100,
            initial_lerp: 100,
            reversed: false,
            flipflop: false,
        });
        let mut state = create_lerp_frame();
        for time in [0, 100, 200, 300, 400, 500, 600] {
            run_lerp_frame(
                &config,
                &mut state,
                &RunLerpFrameInput {
                    time_ms: time,
                    new_animation: 0,
                    speed_scale: 1.0,
                    no_player_animations: false,
                },
                None,
            )
            .unwrap();
        }
        assert!((10..14).contains(&state.frame));

        config.animations[1] = Some(Animation {
            first_frame: 20,
            num_frames: 4,
            loop_frames: 0,
            frame_lerp: 100,
            initial_lerp: 100,
            reversed: true,
            flipflop: false,
        });
        let mut state = create_lerp_frame();
        run_lerp_frame(
            &config,
            &mut state,
            &RunLerpFrameInput {
                time_ms: 0,
                new_animation: 1,
                speed_scale: 1.0,
                no_player_animations: false,
            },
            None,
        )
        .unwrap();
        let first = state.frame;
        run_lerp_frame(
            &config,
            &mut state,
            &RunLerpFrameInput {
                time_ms: 100,
                new_animation: 1,
                speed_scale: 1.0,
                no_player_animations: false,
            },
            None,
        )
        .unwrap();
        assert!(state.frame <= first);
    }

    #[test]
    fn swing_angles_pain_twitch_and_pose() {
        let held = swing_angles(&SwingAnglesInput {
            destination: 10.0,
            swing_tolerance: 30.0,
            clamp_tolerance: 90.0,
            speed: 0.1,
            frame_time_ms: 16,
            angle: 0.0,
            swinging: false,
        })
        .unwrap();
        assert!(!held.swinging);
        assert_eq!(held.angle, 0.0);

        let moving = swing_angles(&SwingAnglesInput {
            destination: 100.0,
            swing_tolerance: 10.0,
            clamp_tolerance: 90.0,
            speed: 0.5,
            frame_time_ms: 10,
            angle: 0.0,
            swinging: false,
        })
        .unwrap();
        assert!(moving.swinging);
        assert!(moving.angle > 0.0);

        assert!(swing_angles(&SwingAnglesInput {
            destination: 0.0,
            swing_tolerance: 10.0,
            clamp_tolerance: 0.5,
            speed: 0.1,
            frame_time_ms: 16,
            angle: 0.0,
            swinging: false,
        })
        .is_err());

        let twitch = add_pain_twitch(
            vec3(1.0, 2.0, 3.0),
            &PainTwitchInput {
                time_ms: 100,
                pain_time: 0,
                pain_direction: true,
            },
        );
        assert!(twitch.z > 3.0);
        let settled = add_pain_twitch(
            vec3(1.0, 2.0, 3.0),
            &PainTwitchInput {
                time_ms: 500,
                pain_time: 0,
                pain_direction: true,
            },
        );
        assert_eq!(settled.z, 3.0);

        let mut pose = create_player_pose_state();
        let result = calculate_player_pose(
            &mut pose,
            &CalculatePlayerPoseInput {
                entity: PoseEntityState {
                    e_flags: 0,
                    velocity: vec3(100.0, 0.0, 0.0),
                    movement_direction: 1.0,
                    legs_anim: Q3PlayerAnimation::LEGS_IDLE,
                    torso_anim: Q3PlayerAnimation::TORSO_STAND,
                },
                fixed_legs: false,
                fixed_torso: false,
                lerp_angles: vec3(0.0, 90.0, 0.0),
                time_ms: 1000,
                frame_time_ms: 16,
                swing_speed: 0.2,
            },
        )
        .unwrap();
        assert!(pose.legs.yawing);
        assert_eq!(result.legs.len(), 3);

        let mut pose = create_player_pose_state();
        assert!(calculate_player_pose(
            &mut pose,
            &CalculatePlayerPoseInput {
                entity: PoseEntityState {
                    e_flags: 0,
                    velocity: vec3(0.0, 0.0, 0.0),
                    movement_direction: 9.0,
                    legs_anim: 0,
                    torso_anim: 0,
                },
                fixed_legs: false,
                fixed_torso: false,
                lerp_angles: vec3(0.0, 0.0, 0.0),
                time_ms: 0,
                frame_time_ms: 16,
                swing_speed: 0.2,
            },
        )
        .is_err());
    }

    #[test]
    fn weapon_view_pose_torso_frames_and_barrel() {
        let motion = Q3WeaponViewMotion {
            origin: vec3(1.0, 2.0, 3.0),
            angles: vec3(0.0, 0.0, 0.0),
            time_ms: 1000,
            horizontal_speed: 200.0,
            bob_cycle: 2,
            bob_fraction_sine: 0.5,
            land_time: 900,
            land_change: 8.0,
        };
        let first = q3_weapon_view_pose(&motion);
        assert_eq!(first, q3_weapon_view_pose(&motion));
        let odd = Q3WeaponViewMotion { bob_cycle: 3, ..motion };
        assert_ne!(first.1.z, q3_weapon_view_pose(&odd).1.z);

        let config = parse_player_animation_config(&animation_fixture(), "<t>").unwrap();
        let drop = config.animations[Q3PlayerAnimation::TORSO_DROP as usize]
            .as_ref()
            .unwrap()
            .first_frame;
        assert_eq!(q3_torso_weapon_frame(&config, drop + 2).unwrap(), 8);
        let attack = config.animations[Q3PlayerAnimation::TORSO_ATTACK as usize]
            .as_ref()
            .unwrap()
            .first_frame;
        assert_eq!(q3_torso_weapon_frame(&config, attack).unwrap(), 1);
        assert_eq!(q3_torso_weapon_frame(&config, 5000).unwrap(), 0);
        let mut missing = config.clone();
        missing.animations[Q3PlayerAnimation::TORSO_DROP as usize] = None;
        assert!(q3_torso_weapon_frame(&missing, drop).is_err());

        let mut barrel = Q3WeaponBarrel::default();
        let spin = barrel.step(100, true);
        assert!(!spin.stopped);
        barrel.step(200, true);
        let stop = barrel.step(300, false);
        assert!(stop.stopped);
    }

    #[test]
    fn qvm_math_matches_source_profile() {
        assert_eq!(qvm_angle_mod(0.0), 0.0);
        assert_eq!(qvm_angle_mod(360.0), 0.0);
        assert_eq!(qvm_angle_mod(720.0), 0.0);
        assert!((qvm_angle_mod(-90.0) - 270.0).abs() < 0.01);
        let axis = q3_identity_axis();
        assert!((axis[0].x - 1.0).abs() < 1e-6);
        assert!((axis[1].y - 1.0).abs() < 1e-6);
        assert!((axis[2].z - 1.0).abs() < 1e-6);
    }

    #[test]
    fn spawn_loadout_and_weapon_requests() {
        let (_, provider) = test_actor();
        let base = q3_spawn_loadout(provider.clone(), Q3Product::BaseQ3, false);
        assert_eq!(base.active_weapon.as_deref(), Some("q3:weapon/machinegun"));
        assert_eq!(base.state.source_weapon, Q3Weapon::MACHINEGUN);
        let mg_ammo = base
            .ammo
            .iter()
            .find(|entry| entry.item == "q3:ammo/machinegun")
            .unwrap();
        assert_eq!(mg_ammo.count, 100.0);
        assert!(base.ammo.iter().all(|entry| !entry.item.contains("nailgun")));
        let tdm = q3_spawn_loadout(provider.clone(), Q3Product::BaseQ3, true);
        assert_eq!(
            tdm.ammo
                .iter()
                .find(|entry| entry.item == "q3:ammo/machinegun")
                .unwrap()
                .count,
            50.0
        );
        let pack = q3_spawn_loadout(provider, Q3Product::MissionPack, false);
        assert!(pack.ammo.iter().any(|entry| entry.item == "q3:weapon/nailgun"));

        let runtime = q3_spawn_arsenal_runtime(Q3Product::BaseQ3, 100, 0);
        assert!(runtime.respawned);
        assert!(q3_request_weapon(&runtime, Q3Weapon::SHOTGUN).is_ok());
        assert!(q3_request_weapon(&runtime, Q3Weapon::NAILGUN).is_err());
        let pack_runtime = q3_spawn_arsenal_runtime(Q3Product::MissionPack, 100, 0);
        assert!(q3_request_weapon(&pack_runtime, Q3Weapon::NAILGUN).is_ok());

        let holstered = q3_request_weapon_holster(&runtime);
        assert_eq!(holstered.external_slot, Q3ExternalWeaponSlot::HolsterRequested);
        let mut dropping = holstered.clone();
        dropping.external_slot = Q3ExternalWeaponSlot::Dropping;
        assert!(q3_request_weapon_resume(&dropping).is_err());
        let mut parked = runtime.clone();
        parked.external_slot = Q3ExternalWeaponSlot::Holstered;
        let resumed = q3_request_weapon_resume(&parked).unwrap();
        assert_eq!(resumed.external_slot, Q3ExternalWeaponSlot::ResumeRequested);
        let back = q3_request_weapon_holster(&resumed);
        assert_eq!(back.external_slot, Q3ExternalWeaponSlot::Holstered);
    }

    fn arsenal_fixture() -> (WeaponStepInput, Q3ArsenalRuntimeState) {
        let (actor, provider) = test_actor();
        let arsenal = q3_spawn_loadout(provider.clone(), Q3Product::BaseQ3, false);
        let input = WeaponStepInput {
            actor,
            frame: test_frame(SourceTime::Milliseconds(8)),
            arsenal,
            animation: ActorAnimationState {
                provider: provider.clone(),
                state: q3_spawn_animation(),
            },
            environment: MovementEnvironment {
                health: 100,
                haste: false,
            },
            gauntlet_hit: false,
        };
        let runtime = q3_spawn_arsenal_runtime(Q3Product::BaseQ3, 100, 0);
        (input, runtime)
    }

    #[test]
    fn arsenal_step_fires_consumes_and_switches() {
        let (input, runtime) = arsenal_fixture();
        let ready = Q3ArsenalRuntimeState {
            respawned: false,
            ..runtime.clone()
        };
        let controls = Q3ArsenalControls {
            attack: true,
            use_holdable: false,
            requested_weapon: Q3Weapon::MACHINEGUN,
        };
        let step = step_q3_arsenal(&input, &ready, &controls, None).unwrap();
        assert_eq!(step.arsenal.state.state, Q3WeaponPhase::FIRING);
        assert_eq!(
            step.arsenal
                .ammo
                .iter()
                .find(|entry| entry.item == "q3:ammo/machinegun")
                .unwrap()
                .count,
            99.0
        );
        assert!(step.effects.iter().any(|effect| matches!(
            effect,
            MovementEffect::Event(event) if event.event == Q3EntityEvent::FIRE_WEAPON
        )));
        assert_eq!(step.torso_animations, vec![Q3PlayerAnimation::TORSO_ATTACK]);
        assert_ne!(step.animation.state.torso, input.animation.state.torso);

        let mut dry = input.clone();
        for entry in dry.arsenal.ammo.iter_mut() {
            if entry.item == "q3:ammo/machinegun" {
                entry.count = 0.0;
            }
        }
        let step = step_q3_arsenal(&dry, &ready, &controls, None).unwrap();
        assert!(step.effects.iter().any(|effect| matches!(
            effect,
            MovementEffect::Event(event) if event.event == Q3EntityEvent::NOAMMO
        )));
        assert_eq!(step.arsenal.state.time_milliseconds, 500);

        let mut stocked = input.clone();
        for entry in stocked.arsenal.ammo.iter_mut() {
            if entry.item == "q3:weapon/shotgun" {
                entry.count = 1.0;
            }
        }
        let switch = Q3ArsenalControls {
            attack: false,
            use_holdable: false,
            requested_weapon: Q3Weapon::SHOTGUN,
        };
        let step = step_q3_arsenal(&stocked, &ready, &switch, None).unwrap();
        assert_eq!(step.arsenal.state.state, Q3WeaponPhase::DROPPING);
        assert_eq!(step.torso_animations, vec![Q3PlayerAnimation::TORSO_DROP]);

        let mut dead = input.clone();
        dead.environment.health = 0;
        let step = step_q3_arsenal(&dead, &ready, &controls, None).unwrap();
        assert_eq!(step.arsenal.state.source_weapon, Q3Weapon::NONE);
        assert!(step.torso_animations.is_empty());

        let idle = Q3ArsenalControls {
            attack: false,
            use_holdable: false,
            requested_weapon: Q3Weapon::MACHINEGUN,
        };
        let step = step_q3_arsenal(&input, &runtime, &idle, None).unwrap();
        assert!(!step.runtime.respawned);

        let holdable = Q3ArsenalRuntimeState {
            respawned: false,
            holdable_item: 1,
            holdable_tag: Q3Holdable::MEDKIT,
            ..runtime.clone()
        };
        let use_controls = Q3ArsenalControls {
            attack: true,
            use_holdable: true,
            requested_weapon: Q3Weapon::MACHINEGUN,
        };
        let step = step_q3_arsenal(&input, &holdable, &use_controls, None).unwrap();
        assert_eq!(step.runtime.holdable_tag, 0);
        assert!(step.effects.iter().any(|effect| matches!(
            effect,
            MovementEffect::Event(event) if event.event == Q3EntityEvent::USE_ITEM0 + Q3Holdable::MEDKIT
        )));
    }

    #[test]
    fn arsenal_step_accepts_seconds_and_firing_delay() {
        let (mut input, runtime) = arsenal_fixture();
        input.frame = test_frame(SourceTime::Seconds(0.008));
        let ready = Q3ArsenalRuntimeState {
            respawned: false,
            ..runtime
        };
        let controls = Q3ArsenalControls {
            attack: true,
            use_holdable: false,
            requested_weapon: Q3Weapon::MACHINEGUN,
        };
        let mut delay = |milliseconds: i32| milliseconds * 2;
        let step = step_q3_arsenal(&input, &ready, &controls, Some(&mut delay)).unwrap();
        assert_eq!(step.arsenal.state.time_milliseconds, 200);
    }

    struct FakeRuntime {
        state: RefCell<Q3ArsenalRuntimeState>,
        gauntlet: bool,
    }

    impl Q3ArsenalRuntimeAccess for FakeRuntime {
        fn read(&self, _actor: &OwnedActor, _execution: Q3Execution) -> Q3ArsenalRuntimeState {
            self.state.borrow().clone()
        }

        fn write(&self, _actor: &OwnedActor, _execution: Q3Execution, state: Q3ArsenalRuntimeState) {
            *self.state.borrow_mut() = state;
        }

        fn gauntlet_hit(&self, _context: &Q3HookContext) -> bool {
            self.gauntlet
        }
    }

    fn hook_fixture() -> (Q3HookContext, Q3ArsenalRuntimeState) {
        let (actor, provider) = test_actor();
        let runtime = q3_spawn_arsenal_runtime(Q3Product::BaseQ3, 100, 7);
        let context = Q3HookContext {
            input: Q3HookInput {
                actor,
                execution: Q3Execution::Authoritative,
                environment: MovementEnvironment {
                    health: 100,
                    haste: false,
                },
            },
            motion: Q3MotionWork {
                pm_type: Q3MoveType::NORMAL,
                pm_flags: Q3MoveFlags::DUCKED,
                event_sequence: 7,
                product: Q3Product::BaseQ3,
            },
            command: Q3HookCommand {
                buttons: Q3CommandButtons::ATTACK,
                weapon: Q3Weapon::MACHINEGUN,
            },
            frame: test_frame(SourceTime::Milliseconds(8)),
            arsenal: q3_spawn_loadout(provider.clone(), Q3Product::BaseQ3, false),
            animation: ActorAnimationState {
                provider,
                state: q3_spawn_animation(),
            },
        };
        (context, runtime)
    }

    #[test]
    fn source_movement_hooks_run_firing_weapon_and_torso() {
        let (mut context, runtime) = hook_fixture();
        let hooks = create_q3_source_movement_hooks(FakeRuntime {
            state: RefCell::new(Q3ArsenalRuntimeState {
                respawned: false,
                ..runtime
            }),
            gauntlet: false,
        });
        assert!(hooks.firing(&context));
        context.arsenal.state.source_weapon = 99;
        assert!(!hooks.firing(&context));
        context.arsenal.state.source_weapon = Q3Weapon::MACHINEGUN;

        let phase = hooks.weapon(&context).unwrap();
        assert_eq!(phase.arsenal.state.state, Q3WeaponPhase::FIRING);
        assert_eq!(phase.movement_flags, Q3MoveFlags::DUCKED);

        let torso = hooks.torso(&context);
        assert_ne!(torso.animation.state.torso, context.animation.state.torso);
        context.arsenal.state.state = Q3WeaponPhase::DROPPING;
        let held = hooks.torso(&context);
        assert!(held.effects.is_empty());

        let legs = hooks.animation(
            &Q3AnimationRequest::Legs {
                animation: Q3PlayerAnimation::LEGS_RUN,
                force: true,
            },
            &context,
        );
        assert_eq!(
            legs.animation.state.legs & !ANIMATION_TOGGLE_BIT,
            Q3PlayerAnimation::LEGS_RUN
        );
    }

    struct FakeServices {
        bodies: RefCell<HashMap<SavedActorId, BodyState>>,
        linked: RefCell<HashMap<SavedActorId, bool>>,
        combat: RefCell<HashMap<SavedActorId, CombatState>>,
        inventory: RefCell<HashMap<SavedActorId, Vec<InventoryEntry>>>,
        events: RefCell<Vec<Q3CharacterEvent>>,
        time: Cell<i32>,
        placement: Q3Placement,
        death_ctx: Q3CharacterDeathContext,
        callbacks: RefCell<HashMap<SavedActorId, Q3CharacterCallbackSet>>,
        spawn_targets_calls: Cell<u32>,
        kill_box_calls: Cell<u32>,
    }

    impl FakeServices {
        fn new(placement: Q3Placement) -> Self {
            Self {
                bodies: RefCell::new(HashMap::new()),
                linked: RefCell::new(HashMap::new()),
                combat: RefCell::new(HashMap::new()),
                inventory: RefCell::new(HashMap::new()),
                events: RefCell::new(Vec::new()),
                time: Cell::new(1000),
                placement,
                death_ctx: Q3CharacterDeathContext {
                    blood: true,
                    no_drop: false,
                    suicide: false,
                    killer_source_slot: 2,
                },
                callbacks: RefCell::new(HashMap::new()),
                spawn_targets_calls: Cell::new(0),
                kill_box_calls: Cell::new(0),
            }
        }

        fn fire_pain(&self, actor: &ActorId) {
            let guard = self.callbacks.borrow();
            (guard.get(&actor_key(actor)).unwrap().pain)();
        }

        fn fire_die(&self, actor: &ActorId) {
            let guard = self.callbacks.borrow();
            (guard.get(&actor_key(actor)).unwrap().die)();
        }
    }

    impl Q3CharacterServices for FakeServices {
        fn read_body(&self, actor: &ActorId) -> Option<BodyState> {
            self.bodies.borrow().get(&actor_key(actor)).cloned()
        }

        fn write_body(&self, actor: &OwnedActor, body: BodyState) {
            self.bodies.borrow_mut().insert(actor_key(actor.id()), body);
        }

        fn link_body(&self, actor: &OwnedActor) {
            self.linked.borrow_mut().insert(actor_key(actor.id()), true);
        }

        fn unlink_body(&self, actor: &OwnedActor) {
            self.linked.borrow_mut().insert(actor_key(actor.id()), false);
        }

        fn bind_callbacks(&self, actor: &OwnedActor, callbacks: Q3CharacterCallbackSet) {
            self.callbacks.borrow_mut().insert(actor_key(actor.id()), callbacks);
        }

        fn read_combat(&self, actor: &ActorId) -> Option<CombatState> {
            self.combat.borrow().get(&actor_key(actor)).cloned()
        }

        fn create_combat(&self, actor: &OwnedActor, combat: CombatState) {
            self.combat.borrow_mut().insert(actor_key(actor.id()), combat);
        }

        fn set_health(&self, actor: &OwnedActor, health: i32) {
            if let Some(combat) = self.combat.borrow_mut().get_mut(&actor_key(actor.id())) {
                combat.health = health;
            }
        }

        fn set_armor(&self, actor: &OwnedActor, armor: ArmorState) {
            if let Some(combat) = self.combat.borrow_mut().get_mut(&actor_key(actor.id())) {
                combat.armor = armor;
            }
        }

        fn set_traits(&self, actor: &OwnedActor, traits: CombatTraitChanges) {
            if let Some(combat) = self.combat.borrow_mut().get_mut(&actor_key(actor.id())) {
                if let Some(value) = traits.can_take_damage {
                    combat.can_take_damage = value;
                }
                if let Some(value) = traits.mass {
                    combat.mass = value;
                }
                if let Some(value) = traits.invulnerable {
                    combat.invulnerable = value;
                }
                if let Some(value) = traits.team {
                    combat.team = value;
                }
                if let Some(value) = traits.no_knockback {
                    combat.no_knockback = value;
                }
            }
        }

        fn has_inventory(&self, actor: &ActorId) -> bool {
            self.inventory.borrow().contains_key(&actor_key(actor))
        }

        fn create_inventory(&self, actor: &OwnedActor, entries: Vec<InventoryEntry>) {
            self.inventory.borrow_mut().insert(actor_key(actor.id()), entries);
        }

        fn inventory_entries(&self, actor: &ActorId) -> Vec<InventoryEntry> {
            self.inventory
                .borrow()
                .get(&actor_key(actor))
                .cloned()
                .unwrap_or_default()
        }

        fn configure_inventory(&self, actor: &OwnedActor, entry: InventoryEntry) {
            let mut guard = self.inventory.borrow_mut();
            let entries = guard.entry(actor_key(actor.id())).or_default();
            if let Some(existing) = entries.iter_mut().find(|item| item.item == entry.item) {
                *existing = entry;
            } else {
                entries.push(entry);
            }
        }

        fn time_ms(&self) -> i32 {
            self.time.get()
        }

        fn emit(&self, event: Q3CharacterEvent) {
            self.events.borrow_mut().push(event);
        }

        fn death_context(&self, _actor: &OwnedActor) -> Q3CharacterDeathContext {
            self.death_ctx
        }

        fn placement(&self) -> Q3Placement {
            self.placement
        }

        fn spawn_targets(&self, _actor: &OwnedActor) {
            self.spawn_targets_calls.set(self.spawn_targets_calls.get() + 1);
        }

        fn kill_box(&self, _actor: &OwnedActor) {
            self.kill_box_calls.set(self.kill_box_calls.get() + 1);
        }
    }

    fn spawn_input() -> Q3CharacterSpawn {
        Q3CharacterSpawn {
            body: BodyState {
                origin: vec3(0.0, 0.0, 0.0),
                angles: vec3(0.0, 90.0, 0.0),
                velocity: vec3(0.0, 0.0, 0.0),
                bounds: Q3_CHARACTER_BOUNDS,
                ground: None,
            },
            combat: q3_initial_combat("100", None).unwrap(),
            inventory: Vec::new(),
        }
    }

    #[test]
    fn character_spawns_dies_and_gibs() {
        let (actor, provider) = test_actor();
        let services = Rc::new(FakeServices::new(Q3Placement::Character));
        let deaths = Rc::new(RefCell::new(Q3DeathAnimationSequence::new()));
        let character = Q3CharacterActor::new(
            actor.clone(),
            provider.clone(),
            Q3Product::BaseQ3,
            Rc::clone(&services),
            Rc::clone(&deaths),
        );
        character.spawn(&spawn_input()).unwrap();
        assert_eq!(character.spawns(), 1);
        assert_eq!(character.source_flags(), 4);
        assert_eq!(services.kill_box_calls.get(), 1);
        assert!(services.events.borrow().is_empty());
        character.spawn(&spawn_input()).unwrap();
        assert_eq!(services.events.borrow().len(), 1);
        assert_eq!(services.events.borrow()[0].event, Q3EntityEvent::PLAYER_TELEPORT_IN);
        character.jump();
        assert_eq!(services.events.borrow().len(), 2);

        services.fire_pain(actor.id());
        let pain = services.events.borrow_mut().pop().unwrap();
        assert_eq!(pain.event, Q3EntityEvent::PAIN);
        assert_eq!(pain.parameter, 125);

        character.die().unwrap();
        let death = services.events.borrow_mut().pop().unwrap();
        assert_eq!(death.event, Q3EntityEvent::DEATH1);
        assert_eq!(death.parameter, 2);
        assert_eq!(
            character.animation().state.legs & !ANIMATION_TOGGLE_BIT,
            Q3PlayerAnimation::BOTH_DEATH1
        );
        assert_eq!(character.respawn_eligible_after_ms(), 2700);
        assert!(!character.wants_respawn(2000, true, false, 0));
        assert!(character.wants_respawn(3000, true, false, 0));

        services.set_health(&actor, -50);
        character.die().unwrap();
        let gib = services.events.borrow_mut().pop().unwrap();
        assert_eq!(gib.event, Q3EntityEvent::GIB_PLAYER);
        assert_eq!(character.source_flags() & 0x80, 0x80);
        assert_eq!(services.linked.borrow().get(&actor_key(actor.id())), Some(&false));
        character.die().unwrap();

        let checkpoint = character.capture();
        assert_eq!(checkpoint.version, 1);
        assert!(checkpoint.dead && checkpoint.gibbed && checkpoint.initialized);
        let owner2 = IdentityOwner::create("test2").unwrap();
        let actor2 = owner2.owned_actor(&owner2.actor(9, 1), provider.clone()).unwrap();
        let other = Q3CharacterActor::new(
            actor2,
            provider,
            Q3Product::BaseQ3,
            Rc::clone(&services),
            Rc::clone(&deaths),
        );
        assert!(other.restore(&checkpoint).is_err());
        other.spawn(&spawn_input()).unwrap();
        let foreign = Q3CharacterCheckpoint {
            product: Q3Product::MissionPack,
            ..checkpoint.clone()
        };
        assert!(other.restore(&foreign).is_err());
        let unadmitted = Q3CharacterCheckpoint {
            initialized: false,
            ..checkpoint.clone()
        };
        assert!(other.restore(&unadmitted).is_err());
    }

    #[test]
    fn character_suicide_gibs_and_death_cycles() {
        let (actor, provider) = test_actor();
        let services = Rc::new(FakeServices {
            death_ctx: Q3CharacterDeathContext {
                blood: false,
                no_drop: false,
                suicide: true,
                killer_source_slot: 0,
            },
            ..FakeServices::new(Q3Placement::SourceGame)
        });
        let character = Q3CharacterActor::new(
            actor.clone(),
            provider.clone(),
            Q3Product::BaseQ3,
            Rc::clone(&services),
            Rc::new(RefCell::new(Q3DeathAnimationSequence::new())),
        );
        character.spawn(&spawn_input()).unwrap();
        assert_eq!(services.kill_box_calls.get(), 0);
        character.die().unwrap();
        assert_eq!(
            services.events.borrow_mut().pop().unwrap().event,
            Q3EntityEvent::GIB_PLAYER
        );

        let mut sequence = Q3DeathAnimationSequence::new();
        assert_eq!(sequence.next(), (0, Q3EntityEvent::DEATH1));
        assert_eq!(sequence.next(), (2, Q3EntityEvent::DEATH2));
        assert_eq!(sequence.next(), (4, Q3EntityEvent::DEATH3));
        assert_eq!(sequence.next(), (0, Q3EntityEvent::DEATH1));
        let checkpoint = sequence.capture();
        sequence.next();
        sequence.restore(&checkpoint).unwrap();
        assert_eq!(sequence.next(), (2, Q3EntityEvent::DEATH2));
        assert!(sequence
            .restore(&Q3DeathAnimationCheckpoint { version: 2, index: 0 })
            .is_err());

        assert_eq!(q3_maximum_health("80").unwrap(), 80);
        assert_eq!(q3_maximum_health("500").unwrap(), 100);
        assert_eq!(
            q3_initial_combat("90", Some("red".to_string())).unwrap(),
            CombatState {
                health: 115,
                armor: ArmorState {
                    regular: RegularArmorState::Q3 {
                        points: 0.0,
                        protection: f64::from(0.66f32),
                    },
                    powered: PoweredProtectionState::None,
                },
                mass: 200.0,
                can_take_damage: true,
                invulnerable: false,
                no_knockback: false,
                team: Some("red".to_string()),
            }
        );

        let wrong = ActorAnimationState {
            provider: ProviderId::new("q3", "other"),
            state: q3_spawn_animation(),
        };
        assert!(character.commit_animation(&wrong).is_err());
        let own = ActorAnimationState {
            provider,
            state: Q3AnimationState {
                legs: 5,
                ..q3_spawn_animation()
            },
        };
        character.commit_animation(&own).unwrap();
        assert_eq!(character.animation().state.legs, 5);
        assert!(character.wants_respawn(5000, false, false, 1));
        services.fire_die(actor.id());
    }

    #[test]
    fn character_animation_steps_map_locomotion() {
        let (_, provider) = test_actor();
        let input = AnimationStepInput {
            frame: test_frame(SourceTime::Milliseconds(16)),
            animation: ActorAnimationState {
                provider,
                state: Q3AnimationState {
                    legs_timer_ms: 100.0,
                    ..q3_spawn_animation()
                },
            },
            locomotion: LocomotionAnimation::Run,
            backwards: true,
            force: true,
        };
        let result = step_q3_character_animation(&input, Q3Product::BaseQ3, false, 3);
        assert_eq!(
            result.animation.state.legs & !ANIMATION_TOGGLE_BIT,
            Q3PlayerAnimation::LEGS_BACK
        );
        assert_eq!(result.animation.state.legs_timer_ms, 0.0);
        assert!(!result.effects.is_empty());

        let land = AnimationStepInput {
            locomotion: LocomotionAnimation::Land,
            backwards: false,
            force: true,
            ..input.clone()
        };
        let result = step_q3_character_animation(&land, Q3Product::BaseQ3, false, 3);
        assert_eq!(result.animation.state.legs_timer_ms, 130.0);

        let dead = step_q3_character_animation(&input, Q3Product::BaseQ3, true, 3);
        assert_eq!(
            dead.animation.state.legs & !ANIMATION_TOGGLE_BIT,
            Q3PlayerAnimation::LEGS_IDLE
        );
    }

    struct StepRandom {
        value: i32,
    }

    impl Q3EventRandom for StepRandom {
        fn rand(&mut self) -> i32 {
            self.value
        }
    }

    fn event_fixture(event: i32, parameter: i32) -> Q3CharacterEvent {
        let (actor, _) = test_actor();
        Q3CharacterEvent {
            actor,
            sequence: 4,
            time_ms: 2000,
            event,
            parameter,
        }
    }

    #[test]
    fn event_presenter_covers_character_events() {
        let mut pose = create_player_pose_state();
        let options = Q3CharacterEventOptions {
            local: false,
            footsteps: true,
            predict_steps: true,
            source_flags: 0,
        };
        let mut presenter = Q3CharacterEventPresenter::new(&mut pose, PlayerFootsteps::Boot, StepRandom { value: 7 });
        assert!(presenter.pain(100, 90).is_empty());
        let pain = presenter.pain(600, 20);
        assert_eq!(pain.len(), 1);
        assert!(matches!(
            &pain[0],
            Q3CharacterPresentationEffect::CustomSound { name, .. } if name == "*pain25_1.wav"
        ));

        let steps = presenter.event(&event_fixture(Q3EntityEvent::FOOTSTEP, 0), &options);
        assert!(matches!(
            &steps[0],
            Q3CharacterPresentationEffect::Footstep {
                material: Q3StepMaterial::Boot,
                variant: 3
            }
        ));
        let metal = presenter.event(&event_fixture(Q3EntityEvent::FOOTSTEP_METAL, 0), &options);
        assert!(matches!(
            &metal[0],
            Q3CharacterPresentationEffect::Footstep {
                material: Q3StepMaterial::Metal,
                ..
            }
        ));
        let quiet = Q3CharacterEventOptions {
            footsteps: false,
            ..options
        };
        assert!(presenter
            .event(&event_fixture(Q3EntityEvent::FOOTSTEP, 0), &quiet)
            .is_empty());

        let fire = presenter.event(&event_fixture(Q3EntityEvent::FIRE_WEAPON, 0), &options);
        assert_eq!(fire, vec![Q3CharacterPresentationEffect::WeaponFire]);
        assert_eq!(presenter.muzzle_flash_time, 2000);

        let death = presenter.event(&event_fixture(Q3EntityEvent::DEATH2, 0), &options);
        assert!(matches!(
            &death[0],
            Q3CharacterPresentationEffect::CustomSound { name, .. } if name == "*death2.wav"
        ));

        let gib = presenter.event(&event_fixture(Q3EntityEvent::GIB_PLAYER, 0), &options);
        assert_eq!(gib.len(), 2);
        let flagged = Q3CharacterEventOptions {
            source_flags: 0x200,
            ..options
        };
        let gib = presenter.event(&event_fixture(Q3EntityEvent::GIB_PLAYER, 0), &flagged);
        assert_eq!(gib, vec![Q3CharacterPresentationEffect::GibPlayer]);

        let remote_pain = presenter.event(&event_fixture(Q3EntityEvent::PAIN, 60), &options);
        assert!(!remote_pain.is_empty());
        let local = Q3CharacterEventOptions { local: true, ..options };
        assert!(presenter
            .event(&event_fixture(Q3EntityEvent::PAIN, 60), &local)
            .is_empty());
        let tele = presenter.event(&event_fixture(Q3EntityEvent::PLAYER_TELEPORT_OUT, 0), &options);
        assert_eq!(tele.len(), 2);
        let kept = presenter.event(&event_fixture(Q3EntityEvent::OBITUARY, 9), &options);
        assert!(matches!(
            &kept[0],
            Q3CharacterPresentationEffect::SourceEvent(event) if event.parameter == 9
        ));
        let nop = presenter.event(&event_fixture(0x300 | Q3EntityEvent::JUMP, 0), &options);
        assert_eq!(nop.len(), 1);

        let mut pose = create_player_pose_state();
        let mut presenter = Q3CharacterEventPresenter::new(&mut pose, PlayerFootsteps::Normal, StepRandom { value: 0 });
        let local = Q3CharacterEventOptions { local: true, ..options };
        presenter.event(&event_fixture(Q3EntityEvent::FALL_FAR, 0), &local);
        assert_eq!(presenter.land_time, 2000);
        assert_eq!(presenter.land_change, -24.0);
        presenter.event(&event_fixture(Q3EntityEvent::STEP_8, 0), &local);
        assert_eq!(presenter.step_time, 2000);
        assert!(presenter.step_change > 0.0);
        let pad = presenter.event(&event_fixture(Q3EntityEvent::JUMP_PAD, 0), &options);
        assert_eq!(pad.len(), 3);
        assert!(matches!(pad[0], Q3CharacterPresentationEffect::JumpPadSmoke));
    }

    #[test]
    fn projectile_behavior_maps_roles() {
        assert_eq!(
            q3_projectile_behavior(Q3Weapon::ROCKET_LAUNCHER).unwrap(),
            Q3ProjectileBehavior {
                weapon: "q3:weapon/rocketlauncher".to_string(),
                role: ProjectileRole::Rocket,
            }
        );
        assert_eq!(
            q3_projectile_behavior(Q3Weapon::PROX_LAUNCHER).unwrap().role,
            ProjectileRole::Grenade
        );
        assert_eq!(
            q3_projectile_behavior(Q3Weapon::NAILGUN).unwrap().role,
            ProjectileRole::Nail
        );
        assert!(q3_projectile_behavior(99).is_err());
        assert!(q3_projectile_behavior(Q3Weapon::MACHINEGUN).is_err());
    }

    #[test]
    fn weapon_hand_grip_matches_sarge_registration() {
        assert_eq!(Q3_WEAPON_HAND_GRIP.origin.x, -2.9841071642362156f64 as f32);
        assert_eq!(Q3_WEAPON_HAND_GRIP.axis[0], vec3(1.0, 0.0, 0.0));
        assert_eq!(Q3_WEAPON_HAND_GRIP.scale, vec3(1.0, 1.0, 1.0));
        assert_eq!(Q3_CHARACTER_VIEW_HEIGHT, 26);
        assert_eq!(Q3_CHARACTER_BOUNDS.min, vec3(-15.0, -15.0, -24.0));
    }

    struct FakeResources {
        files: HashMap<String, Vec<u8>>,
    }

    impl Q3CharacterResources for FakeResources {
        fn open(&self, path: &str) -> Result<Option<OpenedResource>, Q3FoundationError> {
            Ok(self.files.get(path).map(|bytes| OpenedResource {
                reference: dummy_reference(path, bytes.len()),
                bytes: bytes.clone(),
            }))
        }
    }

    fn selection_fixture() -> Q3CharacterSelection {
        Q3CharacterSelection {
            model: "sarge".to_string(),
            skin: "default".to_string(),
            head_model: String::new(),
            head_skin: "default".to_string(),
            team: Some(Q3Team::Blue),
            team_name: String::new(),
        }
    }

    #[test]
    fn character_asset_search_validates_and_reports() {
        let selection = selection_fixture();
        let files = body_files(&selection, "lower", "");
        assert_eq!(files[0], "models/players/sarge/lower_default_blue.skin".to_string());
        assert_eq!(files[1], "models/players/sarge/lower_blue.skin".to_string());
        let heads = head_files(&selection, "head", "skin", "");
        assert!(heads[0].contains("heads/") || heads[0].contains("sarge/default"));

        let empty = FakeResources { files: HashMap::new() };
        let err = load_q3_character(&empty, &selection).unwrap_err();
        assert!(matches!(err, Q3FoundationError::Failed(_)));

        let bad = Q3CharacterSelection {
            model: "../evil".to_string(),
            ..selection.clone()
        };
        assert!(load_q3_character(&empty, &bad).is_err());
        let bad = Q3CharacterSelection {
            head_model: "*".to_string(),
            ..selection.clone()
        };
        assert!(load_q3_character(&empty, &bad).is_err());

        let mut files = HashMap::new();
        for path in [
            "models/players/sarge/lower.md3",
            "models/players/sarge/upper.md3",
            "models/players/sarge/head.md3",
            "models/players/sarge/animation.cfg",
        ] {
            files.insert(path.to_string(), vec![1, 2, 3]);
        }
        let partial = FakeResources { files };
        let err = load_q3_character(&partial, &selection).unwrap_err();
        assert!(format!("{err}").contains("skin missing"));
    }

    fn view_fixture() -> Q3CharacterView {
        let (actor, _) = test_actor();
        Q3CharacterView {
            actor: actor.id().clone(),
            origin: vec3(10.0, 20.0, 30.0),
            angles: vec3(0.0, 45.0, 0.0),
            velocity: vec3(0.0, 0.0, 0.0),
            movement_direction: 0.0,
            animation: q3_spawn_animation(),
            source_flags: 0,
            powerups: 0,
            team: None,
            color: Vec4 {
                x: 1.0,
                y: 1.0,
                z: 1.0,
                w: 1.0,
            },
            scale: None,
            opacity: None,
        }
    }

    fn render_options() -> Q3CharacterRenderOptions {
        Q3CharacterRenderOptions {
            time_ms: 1000,
            frame_ms: 16,
            shader_time: SourceTime::Milliseconds(1000),
            swing_speed: 0.2,
            no_player_animations: false,
            personal_model: false,
            shadow_plane: None,
            weapon: Vec::new(),
        }
    }

    #[test]
    fn presenter_resets_and_renders_passes() {
        let assets = dummy_assets();
        let mut presenter = Q3CharacterPresenter::new(assets);
        let view = view_fixture();
        presenter.reset(&view, 500).unwrap();
        assert_eq!(presenter.pose.legs.yaw_angle, 45.0);
        assert_eq!(presenter.pose.torso.pitch_angle, 0.0);

        let passes = presenter.frame(&view, &render_options()).unwrap();
        assert_eq!(passes.len(), 1);
        assert!(passes[0].shader.is_none());
        assert_eq!(passes[0].entity.attachments.len(), 1);
        assert_eq!(passes[0].entity.transform.origin, view.origin);

        let mut gibbed = view.clone();
        gibbed.source_flags = 0x80;
        assert!(presenter.frame(&gibbed, &render_options()).unwrap().is_empty());

        let mut quad = view.clone();
        quad.powerups = 1 << Q3Powerup::QUAD;
        quad.team = Some(Q3Team::Red);
        let passes = presenter.frame(&quad, &render_options()).unwrap();
        assert_eq!(passes.len(), 2);
        assert_eq!(passes[1].shader.as_deref(), Some("powerups/blueflag"));

        let mut invis = view.clone();
        invis.powerups = (1 << Q3Powerup::INVIS) | (1 << Q3Powerup::QUAD);
        let passes = presenter.frame(&invis, &render_options()).unwrap();
        assert_eq!(passes.len(), 1);
        assert_eq!(passes[0].shader.as_deref(), Some("powerups/invisibility"));

        let mut regen = view.clone();
        regen.powerups = 1 << Q3Powerup::REGEN;
        let options = Q3CharacterRenderOptions {
            time_ms: 100,
            ..render_options()
        };
        let passes = presenter.frame(&regen, &options).unwrap();
        assert_eq!(passes.len(), 2);

        let resolved = passes[0].options(&presenter.assets, &passes[0].entity);
        assert_eq!(resolved.custom_skin.as_ref().unwrap()[0].shader, "models/lower");
        let mut foreign = passes[0].entity.clone();
        foreign.resource = dummy_reference("other.md3", 4);
        let resolved = passes[0].options(&presenter.assets, &foreign);
        assert!(resolved.custom_skin.is_none());
    }

    #[test]
    fn attachment_tags_resolve_md3_and_md5() {
        let tag = SceneMd3Tag {
            name: "tag_torso".to_string(),
            origin: vec3(1.0, 2.0, 3.0),
            axis: q3_identity_axis(),
        };
        let mut scene = empty_scene("lower");
        scene.frames = vec![SceneMd3Frame {
            name: "f0".to_string(),
            bounds: Q3_CHARACTER_BOUNDS,
            local_origin: vec3(0.0, 0.0, 0.0),
            radius: 1.0,
        }];
        scene.tags = vec![vec![tag]];
        let entity = Q3SceneEntity {
            actor: None,
            resource: dummy_reference("lower.md3", 8),
            model: Q3PresentedModel::Md3(scene),
            opacity: 1.0,
            transform: Q3_WEAPON_HAND_GRIP,
            previous_origin: vec3(0.0, 0.0, 0.0),
            pose: Q3ModelPose::Frame {
                frame: 0,
                previous_frame: 0,
                back_lerp: 0.0,
            },
            skin: 0,
            color: Vec4 {
                x: 1.0,
                y: 1.0,
                z: 1.0,
                w: 1.0,
            },
            shader_time: SourceTime::Milliseconds(0),
            flags: 0,
            lighting_origin: vec3(0.0, 0.0, 0.0),
            shadow_plane: 0.0,
            attachments: Vec::new(),
        };
        let resolved = model_attachment_tag(&entity, "tag_torso").unwrap().unwrap();
        assert_eq!(resolved.origin, vec3(1.0, 2.0, 3.0));
        assert_eq!(resolved.scale, 1.0);
        assert!(model_attachment_tag(&entity, "tag_missing").unwrap().is_none());

        let joint = SkeletonJointPose {
            position: vec3(4.0, 5.0, 6.0),
            orientation: Vec4 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                w: 1.0,
            },
            scale: 2.0,
        };
        let md5 = Q3SceneEntity {
            model: Q3PresentedModel::Md5 {
                joints: vec![crate::md5::Md5Joint {
                    name: "tag_weapon".to_string(),
                    parent: -1,
                    scale_positions: false,
                }],
                frames: Vec::new(),
            },
            pose: Q3ModelPose::Skeleton { joints: vec![joint] },
            ..entity.clone()
        };
        let resolved = model_attachment_tag(&md5, "tag_weapon").unwrap().unwrap();
        assert_eq!(resolved.origin, vec3(4.0, 5.0, 6.0));
        assert_eq!(resolved.scale, 2.0);
        assert!(model_attachment_tag(&md5, "nope").unwrap().is_none());

        let broken = Q3SceneEntity {
            pose: Q3ModelPose::Frame {
                frame: -1,
                previous_frame: 0,
                back_lerp: 0.0,
            },
            ..md5.clone()
        };
        assert!(model_attachment_tag(&broken, "tag_weapon").is_err());

        let attached = attach_scene_entity(&entity, &entity, &resolved);
        assert_eq!(attached.lighting_origin, entity.lighting_origin);
    }
}
