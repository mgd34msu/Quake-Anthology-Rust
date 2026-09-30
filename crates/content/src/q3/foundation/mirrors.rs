//! Quake III content foundation (`q3_foundation`) support: shared mirrors, group error, and tests.
//!
//! Self-containment mirrors: minimal local copies of items the donors import from
//! modules outside this port (sibling q3 donors, engine contracts, and math/text
//! helpers), plus the group error type. Sibling-owned mirrors carry SIBLING-MIRROR
//! notes and unify with the canonical ports at merge time.

use crate::contract::{InventoryEntry, ItemId};
use qa_core::binary::BinaryError;
use qa_core::identity::{OwnedActor, ProviderId};
use qa_core::math::{sub3, vec3, Axis, Vec3};
use qa_core::numeric::qvm_float_to_int;
use qa_core::time::FrameContext;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::foundation::animation::*;

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

pub(crate) fn range(message: impl Into<String>) -> Q3FoundationError {
    Q3FoundationError::Range(message.into())
}

pub(crate) fn failed(message: impl Into<String>) -> Q3FoundationError {
    Q3FoundationError::Failed(message.into())
}

pub(crate) fn type_error(message: impl Into<String>) -> Q3FoundationError {
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

pub(crate) struct NumberInput<'a> {
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

pub(crate) fn read_game_float(input: &mut NumberInput<'_>) -> Result<f32, Q3FoundationError> {
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

pub(crate) const COM_TOKEN_MAX: usize = 1024;

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
// QVM math profile (mirror of `src/core/qvm-math.ts`).
// ---------------------------------------------------------------------------

pub(crate) const QVM_ANGLE_SCALE: f32 = (65536.0f64 / 360.0) as f32;

pub(crate) const QVM_ANGLE_UNSCALE: f32 = (360.0f64 / 65536.0) as f32;

pub(crate) const QVM_ANGLE_RADIANS: f32 = (std::f64::consts::PI * 2.0 / 360.0) as f32;

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

pub(crate) fn animation_result(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::ArmorState;
    use crate::contract::ContentId;
    use crate::contract::InventoryEntry;
    use crate::contract::PoweredProtectionState;
    use crate::contract::ProjectileRole;
    use crate::contract::RegularArmorState;
    use crate::contract::ResolvedResourceReference;
    use crate::md3::SkinSurface;
    use crate::md5::SkeletonJointPose;
    use crate::q3::foundation::character::*;
    use crate::q3::foundation::movement_hooks::*;
    use crate::q3anim::PlayerFootsteps;
    use crate::q3scene::SceneMd3;
    use qa_core::math::Vec4;

    use qa_core::identity::{ActorId, OwnedActor, ProviderId};

    use qa_core::time::{FrameContext, SourceTime};
    use std::cell::RefCell;
    use std::rc::Rc;

    use crate::q3::foundation::animation_config::*;

    use crate::q3::foundation::assets::*;

    use crate::q3::foundation::events::*;
    use crate::q3::foundation::held_weapons::*;

    use crate::q3::foundation::player_pose::*;
    use crate::q3::foundation::presentation::*;
    use crate::q3::foundation::weapon_behavior::*;
    use crate::q3::foundation::weapon_pose::*;

    use std::cell::Cell;
    use std::collections::HashMap;

    use crate::contract::{
        ContentDigest, LooseMount, MountId, MountIdentity, MountPlanId, ResourceId, ResourceProvenance,
        ResourceResolution,
    };
    use crate::md3::Md3Model;
    use crate::q3::base::shared::definitions::Product;
    use crate::q3::foundation::arsenal::WeaponStepInput as ArsenalStepInput;
    use crate::q3::foundation::arsenal::*;
    use crate::q3scene::{SceneMd3Frame, SceneMd3Tag};
    use qa_core::identity::{IdentityOwner, SavedActorId};
    use qa_core::time::FramePhase;
    use qa_world::movement::q3::weapon::Q3ExternalWeaponSlot as WorldExternalWeaponSlot;
    use qa_world::movement::types::{
        ActorAnimationState as WorldActorAnimationState, AnimationState, MovementEffect as WorldMovementEffect,
        MovementEnvironment as WorldMovementEnvironment, Q3UserCommand, UserCommand, WeaponState as FamilyWeaponState,
    };

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

    fn test_command() -> UserCommand {
        UserCommand::Q3(Q3UserCommand {
            server_time_milliseconds: 0,
            angle_words: [0, 0, 0],
            buttons: 0,
            weapon: Q3Weapon::MACHINEGUN,
            forward_move: 0,
            right_move: 0,
            up_move: 0,
        })
    }

    fn test_environment(health: f64) -> WorldMovementEnvironment {
        WorldMovementEnvironment {
            client_outputs: None,
            speed_multiplier: None,
            pose: None,
            health,
            flight: false,
            haste: false,
            invulnerable: false,
            gravity_multiplier: 1.0,
        }
    }

    fn q3_weapon(state: &FamilyWeaponState) -> (i32, i32, i32) {
        let FamilyWeaponState::Q3 {
            source_weapon,
            state,
            time_milliseconds,
        } = state
        else {
            panic!("q3 step keeps q3 weapon state");
        };
        (*source_weapon, *state, *time_milliseconds)
    }

    fn q3_torso(state: &AnimationState) -> i32 {
        let AnimationState::Q3 { torso, .. } = state else {
            panic!("q3 step keeps q3 animation");
        };
        *torso
    }

    fn spawn_q3_animation() -> Q3AnimationState {
        Q3AnimationState {
            legs: Q3PlayerAnimation::LEGS_IDLE,
            torso: Q3PlayerAnimation::TORSO_STAND,
            legs_timer_ms: 0.0,
            torso_timer_ms: 0.0,
        }
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
        let base = q3_spawn_loadout(provider.clone(), Product::Baseq3, false);
        assert_eq!(base.active_weapon.as_deref(), Some("q3:weapon/machinegun"));
        assert_eq!(q3_weapon(&base.state).0, Q3Weapon::MACHINEGUN);
        let mg_ammo = base
            .ammo
            .iter()
            .find(|entry| entry.item == "q3:ammo/machinegun")
            .unwrap();
        assert_eq!(mg_ammo.count, 100.0);
        assert!(base.ammo.iter().all(|entry| !entry.item.contains("nailgun")));
        let tdm = q3_spawn_loadout(provider.clone(), Product::Baseq3, true);
        assert_eq!(
            tdm.ammo
                .iter()
                .find(|entry| entry.item == "q3:ammo/machinegun")
                .unwrap()
                .count,
            50.0
        );
        let pack = q3_spawn_loadout(provider, Product::Missionpack, false);
        assert!(pack.ammo.iter().any(|entry| entry.item == "q3:weapon/nailgun"));

        let runtime = q3_spawn_arsenal_runtime(Product::Baseq3, 100.0, 0);
        assert!(runtime.respawned);
        assert!(q3_request_weapon(&runtime, Q3Weapon::SHOTGUN).is_ok());
        assert!(q3_request_weapon(&runtime, Q3Weapon::NAILGUN).is_err());
        let pack_runtime = q3_spawn_arsenal_runtime(Product::Missionpack, 100.0, 0);
        assert!(q3_request_weapon(&pack_runtime, Q3Weapon::NAILGUN).is_ok());

        let holstered = q3_request_weapon_holster(&runtime);
        assert_eq!(holstered.external_slot, WorldExternalWeaponSlot::HolsterRequested);
        let mut dropping = holstered.clone();
        dropping.external_slot = WorldExternalWeaponSlot::Dropping;
        assert!(q3_request_weapon_resume(&dropping).is_err());
        let mut parked = runtime.clone();
        parked.external_slot = WorldExternalWeaponSlot::Holstered;
        let resumed = q3_request_weapon_resume(&parked).unwrap();
        assert_eq!(resumed.external_slot, WorldExternalWeaponSlot::ResumeRequested);
        let back = q3_request_weapon_holster(&resumed);
        assert_eq!(back.external_slot, WorldExternalWeaponSlot::Holstered);
    }

    fn arsenal_fixture() -> (ArsenalStepInput, Q3ArsenalRuntimeState) {
        let (actor, provider) = test_actor();
        let arsenal = q3_spawn_loadout(provider.clone(), Product::Baseq3, false);
        let input = ArsenalStepInput {
            actor,
            command: test_command(),
            frame: test_frame(SourceTime::Milliseconds(8)),
            arsenal,
            animation: WorldActorAnimationState {
                provider: provider.clone(),
                state: q3_spawn_animation(),
            },
            environment: test_environment(100.0),
            gauntlet_hit: false,
        };
        let runtime = q3_spawn_arsenal_runtime(Product::Baseq3, 100.0, 0);
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
        assert_eq!(q3_weapon(&step.arsenal.state).1, Q3WeaponPhase::FIRING);
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
            WorldMovementEffect::Event(event) if event.event == Q3EntityEvent::FIRE_WEAPON
        )));
        assert_eq!(step.torso_animations, vec![Q3PlayerAnimation::TORSO_ATTACK]);
        assert_ne!(q3_torso(&step.animation.state), q3_torso(&input.animation.state));

        let mut dry = input.clone();
        for entry in dry.arsenal.ammo.iter_mut() {
            if entry.item == "q3:ammo/machinegun" {
                entry.count = 0.0;
            }
        }
        let step = step_q3_arsenal(&dry, &ready, &controls, None).unwrap();
        assert!(step.effects.iter().any(|effect| matches!(
            effect,
            WorldMovementEffect::Event(event) if event.event == Q3EntityEvent::NOAMMO
        )));
        assert_eq!(q3_weapon(&step.arsenal.state).2, 500);

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
        assert_eq!(q3_weapon(&step.arsenal.state).1, Q3WeaponPhase::DROPPING);
        assert_eq!(step.torso_animations, vec![Q3PlayerAnimation::TORSO_DROP]);

        let mut dead = input.clone();
        dead.environment.health = 0.0;
        let step = step_q3_arsenal(&dead, &ready, &controls, None).unwrap();
        assert_eq!(q3_weapon(&step.arsenal.state).0, Q3Weapon::NONE);
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
            WorldMovementEffect::Event(event) if event.event == Q3EntityEvent::USE_ITEM0 + Q3Holdable::MEDKIT
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
        let delay = |milliseconds: i32| milliseconds * 2;
        let step = step_q3_arsenal(&input, &ready, &controls, Some(&delay)).unwrap();
        assert_eq!(q3_weapon(&step.arsenal.state).2, 200);
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
        let runtime = q3_spawn_arsenal_runtime(Product::Baseq3, 100.0, 7);
        let context = Q3HookContext {
            input: Q3HookInput {
                actor,
                execution: Q3Execution::Authoritative,
                command: test_command(),
                environment: test_environment(100.0),
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
            arsenal: ArsenalState {
                provider: provider.clone(),
                active_weapon: Some("q3:weapon/machinegun".to_string()),
                state: Q3WeaponState {
                    source_weapon: Q3Weapon::MACHINEGUN,
                    state: Q3WeaponPhase::READY,
                    time_milliseconds: 0,
                },
                ammo: vec![
                    InventoryEntry {
                        item: "q3:weapon/gauntlet".to_string(),
                        count: 1.0,
                        capacity: 1.0,
                        count_policy: None,
                    },
                    InventoryEntry {
                        item: "q3:weapon/machinegun".to_string(),
                        count: 1.0,
                        capacity: 1.0,
                        count_policy: None,
                    },
                    InventoryEntry {
                        item: "q3:ammo/machinegun".to_string(),
                        count: 100.0,
                        capacity: 200.0,
                        count_policy: None,
                    },
                ],
            },
            animation: WorldActorAnimationState {
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
        assert_eq!(q3_weapon(&phase.arsenal.state).1, Q3WeaponPhase::FIRING);
        assert_eq!(phase.movement_flags, Q3MoveFlags::DUCKED);

        let AnimationState::Q3 { torso, .. } = &mut context.animation.state else {
            panic!("hook animation stays q3");
        };
        *torso = Q3PlayerAnimation::TORSO_ATTACK;
        let torso = hooks.torso(&context);
        assert_ne!(torso.animation.state.torso, q3_torso(&context.animation.state));
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
            state: spawn_q3_animation(),
        };
        assert!(character.commit_animation(&wrong).is_err());
        let own = ActorAnimationState {
            provider,
            state: Q3AnimationState {
                legs: 5,
                ..spawn_q3_animation()
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
                    ..spawn_q3_animation()
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

    fn view_fixture() -> Q3CharacterView {
        let (actor, _) = test_actor();
        Q3CharacterView {
            actor: actor.id().clone(),
            origin: vec3(10.0, 20.0, 30.0),
            angles: vec3(0.0, 45.0, 0.0),
            velocity: vec3(0.0, 0.0, 0.0),
            movement_direction: 0.0,
            animation: spawn_q3_animation(),
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
