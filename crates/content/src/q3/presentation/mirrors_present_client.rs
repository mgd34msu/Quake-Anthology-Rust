//! Quake III presentation client (`q3_present_client`) support: shared mirrors, group error, and tests.
//!
//! Self-containment mirrors: minimal local copies of items the donors import from
//! modules outside this port (sibling q3 donors, engine contracts, and math/text
//! helpers), plus the group error type. Sibling-owned mirrors carry SIBLING-MIRROR
//! notes and unify with the canonical ports at merge time.

use crate::q3anim::{PlayerAnimation, PlayerFootsteps, PlayerGender};
use qa_core::math::{add3, scale3, vec3, vector_to_angles, Bounds, Plane, Vec3};
use std::cell::RefCell;
use std::rc::Rc;
use thiserror::Error;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::presentation::retail_snapshot::*;
use crate::q3::presentation::state::*;

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// Presentation-client failure (`CommonError("drop")`, `RangeError`, `Error`).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PresentClientError {
    /// Source `drop`: the session must tear down with this message.
    #[error("dropped: {0}")]
    Drop(String),
    /// Source `RangeError`: value outside its defined bounds.
    #[error("out of range: {0}")]
    Range(String),
    /// Any other source `Error`: misuse or violated invariant.
    #[error("client presentation error: {0}")]
    State(String),
}

/// Fallible presentation-client result.
pub type PresentResult<T> = Result<T, PresentClientError>;

pub(crate) fn drop_msg(message: impl Into<String>) -> PresentClientError {
    PresentClientError::Drop(message.into())
}

pub(crate) fn range_msg(message: impl Into<String>) -> PresentClientError {
    PresentClientError::Range(message.into())
}

pub(crate) fn state_msg(message: impl Into<String>) -> PresentClientError {
    PresentClientError::State(message.into())
}

pub(crate) fn at<'a, T>(items: &'a [T], index: usize, what: &str) -> PresentResult<&'a T> {
    items
        .get(index)
        .ok_or_else(|| range_msg(format!("{what} index {index} outside {}", items.len())))
}

pub(crate) fn at_mut<'a, T>(items: &'a mut [T], index: usize, what: &str) -> PresentResult<&'a mut T> {
    let len = items.len();
    items
        .get_mut(index)
        .ok_or_else(|| range_msg(format!("{what} index {index} outside {len}")))
}

// ---------------------------------------------------------------------------
// Small numeric and text mirrors
// ---------------------------------------------------------------------------

/// `qvmFloatToInt`: float32 truncate with the native indefinite-integer edge.
#[must_use]
pub fn qvm_float_to_int(value: f32) -> i32 {
    if (-2147483648.0..2147483648.0).contains(&value) {
        value.trunc() as i32
    } else {
        i32::MIN
    }
}

/// `qvmAngleMod`: QVM angle reduction through the 65536-unit circle.
#[must_use]
pub fn qvm_angle_mod(angle: f32) -> f32 {
    let scaled = angle * ((65536.0f64 / 360.0) as f32);
    ((qvm_float_to_int(scaled) & 65535) as f32) * ((360.0f64 / 65536.0) as f32)
}

/// `bg_lib` atoi: wrapping decimal scan after whitespace and sign.
#[must_use]
pub fn game_atoi(text: &str) -> i32 {
    let bytes = text.as_bytes();
    let mut offset = 0;
    while offset < bytes.len() && bytes[offset] <= b' ' {
        offset += 1;
    }
    if offset >= bytes.len() || bytes[offset] == 0 {
        return 0;
    }
    let mut sign = 1i32;
    if bytes[offset] == b'+' || bytes[offset] == b'-' {
        if bytes[offset] == b'-' {
            sign = -1;
        }
        offset += 1;
    }
    let mut value = 0i32;
    while offset < bytes.len() {
        let digit = bytes[offset];
        if !digit.is_ascii_digit() {
            break;
        }
        value = value.wrapping_mul(10).wrapping_add((digit - b'0') as i32);
        offset += 1;
    }
    value.wrapping_mul(sign)
}

/// Compact native `atof` for `showevents`-style scalar cvars.
#[must_use]
pub fn native_atof(text: &str) -> f64 {
    let bytes = text.as_bytes();
    let mut offset = 0;
    while offset < bytes.len() && bytes[offset].is_ascii_whitespace() {
        offset += 1;
    }
    let mut negative = false;
    if offset < bytes.len() && (bytes[offset] == b'+' || bytes[offset] == b'-') {
        negative = bytes[offset] == b'-';
        offset += 1;
    }
    let mut value = 0.0f64;
    while offset < bytes.len() && bytes[offset].is_ascii_digit() {
        value = value * 10.0 + f64::from(bytes[offset] - b'0');
        offset += 1;
    }
    if offset < bytes.len() && bytes[offset] == b'.' {
        offset += 1;
        let mut factor = 0.1f64;
        while offset < bytes.len() && bytes[offset].is_ascii_digit() {
            value += f64::from(bytes[offset] - b'0') * factor;
            factor *= 0.1;
            offset += 1;
        }
    }
    if offset < bytes.len() && (bytes[offset] == b'e' || bytes[offset] == b'E') {
        offset += 1;
        let mut exp_negative = false;
        if offset < bytes.len() && (bytes[offset] == b'+' || bytes[offset] == b'-') {
            exp_negative = bytes[offset] == b'-';
            offset += 1;
        }
        let mut exponent = 0i32;
        while offset < bytes.len() && bytes[offset].is_ascii_digit() {
            exponent = exponent
                .saturating_mul(10)
                .saturating_add((bytes[offset] - b'0') as i32);
            offset += 1;
        }
        let scale = 10f64.powi(if exp_negative { -exponent } else { exponent });
        value *= scale;
    }
    if negative {
        -value
    } else {
        value
    }
}

/// Truncate at NUL and require source byte characters (`bytes`).
pub fn source_bytes(input: &str) -> PresentResult<String> {
    let end = input.find('\0').unwrap_or(input.len());
    let text = &input[..end];
    if text.chars().any(|c| c as u32 > 255) {
        return Err(range_msg("Cgame text requires source byte characters"));
    }
    Ok(text.to_owned())
}

/// ASCII case fold (`fold`).
#[must_use]
pub fn ascii_fold(input: &str) -> String {
    input.to_ascii_lowercase()
}

/// `Info_ValueForKey` over a backslash-delimited info string.
pub fn info_value_for_key(input: &str, wanted: &str, maximum_length: usize) -> PresentResult<String> {
    if !(1..=8192).contains(&maximum_length) {
        return Err(range_msg("Invalid source info-string bound"));
    }
    let end = input.find('\0').unwrap_or(input.len());
    let key_end = wanted.find('\0').unwrap_or(wanted.len());
    let text = &input[..end];
    let key = &wanted[..key_end];
    if text.chars().count() >= maximum_length {
        return Err(drop_msg("Info_ValueForKey: oversize infostring"));
    }
    for value in [text, key] {
        if value.chars().any(|c| c as u32 > 255) {
            return Err(range_msg("Info_ValueForKey requires byte characters"));
        }
    }
    let bytes = text.as_bytes();
    let folded_key = ascii_fold(key);
    let mut cursor = usize::from(text.starts_with('\\'));
    while cursor < bytes.len() {
        let rest = &bytes[cursor..];
        let separator = rest.iter().position(|b| *b == b'\\');
        let Some(sep) = separator else { return Ok(String::new()) };
        let separator = cursor + sep;
        let after = &bytes[separator + 1..];
        let next = after.iter().position(|b| *b == b'\\');
        let value_end = next.map_or(bytes.len(), |n| separator + 1 + n);
        let name = String::from_utf8_lossy(&bytes[cursor..separator]);
        if ascii_fold(&name) == folded_key {
            return Ok(String::from_utf8_lossy(&bytes[separator + 1..value_end]).into_owned());
        }
        cursor = value_end + 1;
    }
    Ok(String::new())
}

/// `gameFormat` argument.
#[derive(Debug, Clone)]
pub enum GameFormatArg {
    /// Signed integer (`%i`, `%d`, `%c`).
    Int(i32),
    /// Unsigned integer (`%u`).
    UInt(u32),
    /// Floating point (`%f`).
    Float(f64),
    /// Byte string (`%s`).
    Text(String),
    /// Null string pointer, formats as `(null)`.
    Null,
}

impl From<i32> for GameFormatArg {
    fn from(value: i32) -> Self {
        Self::Int(value)
    }
}

impl From<u32> for GameFormatArg {
    fn from(value: u32) -> Self {
        Self::UInt(value)
    }
}

impl From<f32> for GameFormatArg {
    fn from(value: f32) -> Self {
        Self::Float(f64::from(value))
    }
}

impl From<f64> for GameFormatArg {
    fn from(value: f64) -> Self {
        Self::Float(value)
    }
}

impl From<&str> for GameFormatArg {
    fn from(value: &str) -> Self {
        Self::Text(value.to_owned())
    }
}

impl From<String> for GameFormatArg {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

pub(crate) fn format_pad(text: &str, width: i32, left: bool, zero: bool, numeric: bool) -> String {
    let padding = (width as usize).saturating_sub(text.len());
    if padding == 0 {
        return text.to_owned();
    }
    if left {
        format!("{text}{:1$}", "", padding)
    } else if zero && numeric {
        let (sign, digits) = text.strip_prefix('-').map_or(("", text), |rest| ("-", rest));
        format!("{sign}{:0>1$}", digits, digits.len() + padding)
    } else {
        format!("{:1$}{text}", "", padding)
    }
}

/// `gameFormat` with the QVM `bg_lib.c` rules used by cgame, then
/// `Q_strncpyz` bounds (`max_bytes`, default callers pass the big buffer).
pub fn game_format(format: &str, args: &[GameFormatArg], max_bytes: usize) -> PresentResult<String> {
    if max_bytes < 1 {
        return Err(range_msg(
            "game format destination capacity must be a positive safe integer",
        ));
    }
    if format.chars().any(|c| c as u32 > 255) {
        return Err(range_msg("game format strings must contain byte-valued code units"));
    }
    let mut output = String::new();
    let bytes = format.as_bytes();
    let mut cursor = 0;
    let mut argument = 0;
    let take = |argument: &mut usize| -> PresentResult<&GameFormatArg> {
        let value = args
            .get(*argument)
            .ok_or_else(|| range_msg("game format argument missing"))?;
        *argument += 1;
        Ok(value)
    };
    while cursor < bytes.len() {
        let byte = bytes[cursor];
        if byte == 0 {
            break;
        }
        if byte != b'%' {
            output.push(byte as char);
            cursor += 1;
            continue;
        }
        cursor += 1;
        let mut left = false;
        let mut zero = false;
        let mut width = 0i32;
        let mut precision = -1i32;
        loop {
            let spec = *bytes
                .get(cursor)
                .ok_or_else(|| range_msg("unterminated game format specifier"))?;
            cursor += 1;
            if spec == b'-' {
                left = true;
                continue;
            }
            if spec == b'.' {
                let mut parsed = 0i32;
                while let Some(digit) = bytes.get(cursor) {
                    if !digit.is_ascii_digit() {
                        break;
                    }
                    parsed = parsed.wrapping_mul(10).wrapping_add((digit - b'0') as i32);
                    cursor += 1;
                }
                precision = if parsed < 0 { -1 } else { parsed };
                continue;
            }
            if spec == b'0' {
                zero = true;
                continue;
            }
            if spec.is_ascii_digit() {
                let mut parsed = 0i32;
                let mut digit = spec;
                loop {
                    parsed = parsed.wrapping_mul(10).wrapping_add((digit - b'0') as i32);
                    let Some(next) = bytes.get(cursor) else { break };
                    if !next.is_ascii_digit() {
                        break;
                    }
                    digit = *next;
                    cursor += 1;
                }
                width = parsed;
                continue;
            }
            match spec {
                b'%' => output.push('%'),
                b's' => {
                    let text = match take(&mut argument)? {
                        GameFormatArg::Text(value) => value.clone(),
                        GameFormatArg::Null => "(null)".to_owned(),
                        GameFormatArg::Int(value) => value.to_string(),
                        GameFormatArg::UInt(value) => value.to_string(),
                        GameFormatArg::Float(value) => format!("{value}"),
                    };
                    let clipped = if precision >= 0 {
                        text.as_bytes()
                            .get(..precision.max(0) as usize)
                            .map_or_else(|| text.clone(), |slice| String::from_utf8_lossy(slice).into_owned())
                    } else {
                        text
                    };
                    output.push_str(&format_pad(&clipped, width, left, false, false));
                }
                b'd' | b'i' => {
                    let value = match take(&mut argument)? {
                        GameFormatArg::Int(value) => *value,
                        GameFormatArg::UInt(value) => *value as i32,
                        GameFormatArg::Float(value) => *value as i32,
                        GameFormatArg::Text(value) => game_atoi(value),
                        GameFormatArg::Null => 0,
                    };
                    let mut digits = value.to_string();
                    if precision >= 0 {
                        let negative = digits.starts_with('-');
                        let body = digits.trim_start_matches('-');
                        if body.len() < precision as usize {
                            digits = format!("{}{:0>2$}", if negative { "-" } else { "" }, body, precision as usize);
                        } else if precision == 0 && value == 0 {
                            digits = String::new();
                        }
                    }
                    output.push_str(&format_pad(&digits, width, left, zero, true));
                }
                b'u' => {
                    let value = match take(&mut argument)? {
                        GameFormatArg::UInt(value) => *value,
                        GameFormatArg::Int(value) => *value as u32,
                        GameFormatArg::Float(value) => *value as u32,
                        GameFormatArg::Text(value) => game_atoi(value) as u32,
                        GameFormatArg::Null => 0,
                    };
                    output.push_str(&format_pad(&value.to_string(), width, left, zero, true));
                }
                b'f' => {
                    let value = match take(&mut argument)? {
                        GameFormatArg::Float(value) => *value,
                        GameFormatArg::Int(value) => f64::from(*value),
                        GameFormatArg::UInt(value) => f64::from(*value),
                        GameFormatArg::Text(value) => native_atof(value),
                        GameFormatArg::Null => 0.0,
                    };
                    let precision = if precision < 0 { 6 } else { precision as usize };
                    output.push_str(&format_pad(&format!("{value:.precision$}"), width, left, zero, true));
                }
                b'c' => {
                    let value = match take(&mut argument)? {
                        GameFormatArg::Int(value) => *value,
                        GameFormatArg::UInt(value) => *value as i32,
                        _ => return Err(range_msg("game format %c requires an integer")),
                    };
                    output.push(char::from_u32((value as u8) as u32).unwrap_or('?'));
                }
                _ => return Err(range_msg("unsupported game format specifier")),
            }
            break;
        }
    }
    let capacity = max_bytes - 1;
    if output.len() > capacity {
        let mut end = capacity;
        while end > 0 && !output.is_char_boundary(end) {
            end -= 1;
        }
        output.truncate(end);
    }
    Ok(output)
}

// ---------------------------------------------------------------------------
// Shared game constants (SIBLING-MIRROR of base/shared/definitions.ts and
// movement/q3/constants.ts)
// ---------------------------------------------------------------------------

macro_rules! q3_int_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident = $value:expr,)* }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        #[repr(i32)]
        pub enum $name { $($variant = $value,)* }
        impl $name {
            /// Raw source value lookup.
            #[must_use]
            pub const fn from_i32(value: i32) -> Option<Self> {
                match value { $($value => Some(Self::$variant),)* _ => None }
            }
        }
    };
}

/// Product (`Product`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q3Product {
    /// Base Quake III.
    BaseQ3,
    /// Mission pack.
    MissionPack,
}

impl Q3Product {
    /// Source product tag.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BaseQ3 => "baseq3",
            Self::MissionPack => "missionpack",
        }
    }
}

/// Default gravity (`DEFAULT_GRAVITY`).
pub const DEFAULT_GRAVITY: f32 = 800.0;

/// Gib health threshold (`GIB_HEALTH`).
pub const GIB_HEALTH: i32 = -40;

/// Event bit 1 (`EV_EVENT_BIT1`).
pub const EV_EVENT_BIT1: i32 = 0x100;

/// Event bit 2 (`EV_EVENT_BIT2`).
pub const EV_EVENT_BIT2: i32 = 0x200;

/// Event bits mask (`EV_EVENT_BITS`).
pub const EV_EVENT_BITS: i32 = 0x300;

/// World entity number (`ENTITYNUM_WORLD`).
pub const ENTITYNUM_WORLD: i32 = 1022;

/// No-entity number (`ENTITYNUM_NONE`).
pub const ENTITYNUM_NONE: i32 = 1023;

/// Parse-entity ring size (`MAX_PARSE_ENTITIES`).
pub const MAX_PARSE_ENTITIES: i32 = 2048;

/// Lerp-frame animation toggle bit (`ANIMATION_TOGGLE_BIT`).
pub const ANIMATION_TOGGLE_BIT: i32 = 128;

/// Third-person render flag (`RF_THIRD_PERSON`).
pub const RF_THIRD_PERSON: i32 = 2;

/// Lighting-origin render flag (`RF_LIGHTING_ORIGIN`).
pub const RF_LIGHTING_ORIGIN: i32 = 128;

/// Shadow-plane render flag (`RF_SHADOW_PLANE`).
pub const RF_SHADOW_PLANE: i32 = 256;

/// Snapshot history window (`PACKET_BACKUP`-style 32-slot check).
pub const SNAPSHOT_HISTORY_WINDOW: i32 = 32;

/// Maximum clients (`MAX_CLIENTS` cgame slots).
pub const MAX_CLIENTS: usize = 64;

/// Maximum entities (`MAX_GENTITIES` cgame slots).
pub const MAX_ENTITIES: usize = 1024;

q3_int_enum! {
    /// Game type (`GameType`).
    GameType {
        Ffa = 0,
        Tournament = 1,
        SinglePlayer = 2,
        Team = 3,
        Ctf = 4,
        OneFctf = 5,
        Obelisk = 6,
        Harvester = 7,
        MaxGameType = 8,
    }
}

impl PartialOrd for GameType {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for GameType {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (*self as i32).cmp(&(*other as i32))
    }
}

q3_int_enum! {
    /// Team (`Team`).
    Team {
        Free = 0,
        Red = 1,
        Blue = 2,
        Spectator = 3,
        NumTeams = 4,
    }
}

q3_int_enum! {
    /// Item type (`ItemType`).
    ItemType {
        Bad = 0,
        Weapon = 1,
        Ammo = 2,
        Armor = 3,
        Health = 4,
        Powerup = 5,
        Holdable = 6,
        PersistantPowerup = 7,
        Team = 8,
    }
}

q3_int_enum! {
    /// Entity type (`EntityType`).
    EntityType {
        General = 0,
        Player = 1,
        Item = 2,
        Missile = 3,
        Mover = 4,
        Beam = 5,
        Portal = 6,
        Speaker = 7,
        PushTrigger = 8,
        TeleportTrigger = 9,
        Invisible = 10,
        Grapple = 11,
        Team = 12,
        Events = 13,
    }
}

q3_int_enum! {
    /// Persistent player-state index (`PersistentIndex`).
    PersistentIndex {
        Score = 0,
        Hits = 1,
        Rank = 2,
        Team = 3,
        SpawnCount = 4,
        PlayerEvents = 5,
        Attacker = 6,
        AttackeeArmor = 7,
        Killed = 8,
        ImpressiveCount = 9,
        ExcellentCount = 10,
        DefendCount = 11,
        AssistCount = 12,
        GauntletFragCount = 13,
        Captures = 14,
    }
}

q3_int_enum! {
    /// Movement type (`MoveType`).
    MoveType {
        Normal = 0,
        Noclip = 1,
        Spectator = 2,
        Dead = 3,
        Freeze = 4,
        Intermission = 5,
        SpIntermission = 6,
    }
}

q3_int_enum! {
    /// Powerup tag (`Powerup`).
    Powerup {
        None = 0,
        Quad = 1,
        Battlesuit = 2,
        Haste = 3,
        Invis = 4,
        Regen = 5,
        Flight = 6,
        RedFlag = 7,
        BlueFlag = 8,
        NeutralFlag = 9,
        Scout = 10,
        Guard = 11,
        Doubler = 12,
        Ammoregen = 13,
        Invulnerability = 14,
        NumPowerups = 15,
    }
}

q3_int_enum! {
    /// Holdable tag (`Holdable`).
    Holdable {
        None = 0,
        Teleporter = 1,
        Medkit = 2,
        Kamikaze = 3,
        Portal = 4,
        Invulnerability = 5,
        NumHoldable = 6,
    }
}

q3_int_enum! {
    /// Weapon tag (`Weapon`).
    Weapon {
        None = 0,
        Gauntlet = 1,
        Machinegun = 2,
        Shotgun = 3,
        GrenadeLauncher = 4,
        RocketLauncher = 5,
        Lightning = 6,
        Railgun = 7,
        Plasmagun = 8,
        Bfg = 9,
        GrapplingHook = 10,
        Nailgun = 11,
        ProxLauncher = 12,
        Chaingun = 13,
    }
}

q3_int_enum! {
    /// Entity event (`EntityEvent`).
    EntityEvent {
        None = 0,
        Footstep = 1,
        FootstepMetal = 2,
        Footsplash = 3,
        Footwade = 4,
        Swim = 5,
        Step4 = 6,
        Step8 = 7,
        Step12 = 8,
        Step16 = 9,
        FallShort = 10,
        FallMedium = 11,
        FallFar = 12,
        JumpPad = 13,
        Jump = 14,
        WaterTouch = 15,
        WaterLeave = 16,
        WaterUnder = 17,
        WaterClear = 18,
        ItemPickup = 19,
        GlobalItemPickup = 20,
        Noammo = 21,
        ChangeWeapon = 22,
        FireWeapon = 23,
        UseItem0 = 24,
        UseItem1 = 25,
        UseItem2 = 26,
        UseItem3 = 27,
        UseItem4 = 28,
        UseItem5 = 29,
        UseItem6 = 30,
        UseItem7 = 31,
        UseItem8 = 32,
        UseItem9 = 33,
        UseItem10 = 34,
        UseItem11 = 35,
        UseItem12 = 36,
        UseItem13 = 37,
        UseItem14 = 38,
        UseItem15 = 39,
        ItemRespawn = 40,
        ItemPop = 41,
        PlayerTeleportIn = 42,
        PlayerTeleportOut = 43,
        GrenadeBounce = 44,
        GeneralSound = 45,
        GlobalSound = 46,
        GlobalTeamSound = 47,
        BulletHitFlesh = 48,
        BulletHitWall = 49,
        MissileHit = 50,
        MissileMiss = 51,
        MissileMissMetal = 52,
        Railtrail = 53,
        Shotgun = 54,
        Bullet = 55,
        Pain = 56,
        Death1 = 57,
        Death2 = 58,
        Death3 = 59,
        Obituary = 60,
        PowerupQuad = 61,
        PowerupBattlesuit = 62,
        PowerupRegen = 63,
        GibPlayer = 64,
        Scoreplum = 65,
        ProximityMineStick = 66,
        ProximityMineTrigger = 67,
        Kamikaze = 68,
        ObeliskExplode = 69,
        ObeliskPain = 70,
        InvulImpact = 71,
        Juiced = 72,
        LightningBolt = 73,
        DebugLine = 74,
        StopLoopingSound = 75,
        Taunt = 76,
        TauntYes = 77,
        TauntNo = 78,
        TauntFollowMe = 79,
        TauntGetFlag = 80,
        TauntGuardBase = 81,
        TauntPatrol = 82,
    }
}

q3_int_enum! {
    /// Player animation (`PlayerAnimation`).
    PlayerAnimationNumber {
        BothDeath1 = 0,
        BothDead1 = 1,
        BothDeath2 = 2,
        BothDead2 = 3,
        BothDeath3 = 4,
        BothDead3 = 5,
        TorsoGesture = 6,
        TorsoAttack = 7,
        TorsoAttack2 = 8,
        TorsoDrop = 9,
        TorsoRaise = 10,
        TorsoStand = 11,
        TorsoStand2 = 12,
        LegsWalkCr = 13,
        LegsWalk = 14,
        LegsRun = 15,
        LegsBack = 16,
        LegsSwim = 17,
        LegsJump = 18,
        LegsLand = 19,
        LegsJumpB = 20,
        LegsLandB = 21,
        LegsIdle = 22,
        LegsIdleCr = 23,
        LegsTurn = 24,
        TorsoGetFlag = 25,
        TorsoGuardBase = 26,
        TorsoPatrol = 27,
        TorsoFollowMe = 28,
        TorsoAffirmative = 29,
        TorsoNegative = 30,
        LegsBackCr = 32,
        LegsBackWalk = 33,
        FlagRun = 34,
        FlagStand = 35,
        FlagStand2Run = 36,
    }
}

/// Player movement flags (`MoveFlags`).
pub enum MoveFlags {}

impl MoveFlags {
    /// Ducked.
    pub const DUCKED: i32 = 1;
    /// Jump held.
    pub const JUMP_HELD: i32 = 2;
    /// Backwards jump.
    pub const BACKWARDS_JUMP: i32 = 8;
    /// Backwards run.
    pub const BACKWARDS_RUN: i32 = 16;
    /// Landing timer active.
    pub const TIME_LAND: i32 = 32;
    /// Knockback timer active.
    pub const TIME_KNOCKBACK: i32 = 64;
    /// Water-jump timer active.
    pub const TIME_WATERJUMP: i32 = 256;
    /// Respawned this frame.
    pub const RESPAWNED: i32 = 512;
    /// Use-holdable held.
    pub const USE_ITEM_HELD: i32 = 1024;
    /// Grapple pull.
    pub const GRAPPLE_PULL: i32 = 2048;
    /// Following another player.
    pub const FOLLOW: i32 = 4096;
    /// Scoreboard held.
    pub const SCOREBOARD: i32 = 8192;
    /// Invulnerability expansion.
    pub const INVULEXPAND: i32 = 16384;
}

/// Player-state stat slots (`StatSchema` plus product tags).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatSchema {
    /// Product.
    pub product: Q3Product,
    /// Health slot.
    pub health: i32,
    /// Holdable-item slot.
    pub holdable_item: i32,
    /// Mission-pack persistent-powerup slot.
    pub persistent_powerup: Option<i32>,
    /// Weapons bitmask slot.
    pub weapons: i32,
    /// Armor slot.
    pub armor: i32,
    /// Dead-yaw slot.
    pub dead_yaw: i32,
    /// Clients-ready slot.
    pub clients_ready: i32,
    /// Max-health slot.
    pub max_health: i32,
}

/// Stat schema for a product (`statSchema`).
#[must_use]
pub const fn stat_schema(product: Q3Product) -> StatSchema {
    match product {
        Q3Product::BaseQ3 => StatSchema {
            product,
            health: 0,
            holdable_item: 1,
            persistent_powerup: None,
            weapons: 2,
            armor: 3,
            dead_yaw: 4,
            clients_ready: 5,
            max_health: 6,
        },
        Q3Product::MissionPack => StatSchema {
            product,
            health: 0,
            holdable_item: 1,
            persistent_powerup: Some(2),
            weapons: 3,
            armor: 4,
            dead_yaw: 5,
            clients_ready: 6,
            max_health: 7,
        },
    }
}

/// Weapon count for a product (`weaponCount`).
#[must_use]
pub const fn weapon_count(product: Q3Product) -> i32 {
    match product {
        Q3Product::BaseQ3 => 11,
        Q3Product::MissionPack => 14,
    }
}

// ---------------------------------------------------------------------------
// Trajectory (SIBLING-MIRROR of base/shared/trajectory.ts)
// ---------------------------------------------------------------------------

/// Trajectory type (`TrajectoryType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum TrajectoryType {
    /// Stationary.
    Stationary = 0,
    /// Interpolate.
    Interpolate = 1,
    /// Linear.
    Linear = 2,
    /// Linear with stop.
    LinearStop = 3,
    /// Sine.
    Sine = 4,
    /// Gravity.
    Gravity = 5,
}

/// Position trajectory (`Trajectory`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Trajectory {
    /// Trajectory type tag.
    pub type_tag: i32,
    /// Start time.
    pub time: i32,
    /// Duration.
    pub duration: i32,
    /// Base position.
    pub base: Vec3,
    /// Delta (velocity or amplitude).
    pub delta: Vec3,
}

impl Default for Trajectory {
    fn default() -> Self {
        Self {
            type_tag: 0,
            time: 0,
            duration: 0,
            base: vec3(0.0, 0.0, 0.0),
            delta: vec3(0.0, 0.0, 0.0),
        }
    }
}

pub(crate) fn trajectory_seconds(milliseconds: i32) -> f32 {
    milliseconds as f32 * (0.001f64 as f32)
}

pub(crate) fn periodic_radians(trajectory: &Trajectory, at_time: i32) -> f32 {
    let fraction = at_time.wrapping_sub(trajectory.time) as f32 / trajectory.duration as f32;
    fraction * (std::f32::consts::PI) * 2.0
}

/// `BG_EvaluateTrajectory` with QVM binary32 rounding.
pub fn evaluate_trajectory(trajectory: &Trajectory, at_time: i32) -> PresentResult<Vec3> {
    match trajectory.type_tag {
        0 | 1 => Ok(vec3(trajectory.base.x, trajectory.base.y, trajectory.base.z)),
        2 => {
            let delta_time = trajectory_seconds(at_time.wrapping_sub(trajectory.time));
            Ok(add3(trajectory.base, scale3(trajectory.delta, delta_time)))
        }
        4 => {
            let phase = periodic_radians(trajectory, at_time).sin();
            Ok(add3(trajectory.base, scale3(trajectory.delta, phase)))
        }
        3 => {
            let end = trajectory.time.wrapping_add(trajectory.duration);
            let time = if at_time > end { end } else { at_time };
            let delta_time = trajectory_seconds(time.wrapping_sub(trajectory.time)).max(0.0);
            Ok(add3(trajectory.base, scale3(trajectory.delta, delta_time)))
        }
        5 => {
            let delta_time = trajectory_seconds(at_time.wrapping_sub(trajectory.time));
            let result = add3(trajectory.base, scale3(trajectory.delta, delta_time));
            let fall = (0.5 * f64::from(DEFAULT_GRAVITY) * f64::from(delta_time)) as f32 * delta_time;
            Ok(vec3(result.x, result.y, result.z - fall))
        }
        _ => Err(drop_msg(format!(
            "BG_EvaluateTrajectory: unknown trType: {}",
            trajectory.time
        ))),
    }
}

// ---------------------------------------------------------------------------
// Direction byte (SIBLING-MIRROR of base/shared/direction-byte.ts)
// ---------------------------------------------------------------------------

/// Vertex-normal table (`BYTE_DIRECTIONS`, 162 entries).
pub const BYTE_DIRECTIONS: [[f32; 3]; 162] = [
    [-0.525731, 0.000000, 0.850651],
    [-0.442863, 0.238856, 0.864188],
    [-0.295242, 0.000000, 0.955423],
    [-0.309017, 0.500000, 0.809017],
    [-0.162460, 0.262866, 0.951056],
    [0.000000, 0.000000, 1.000000],
    [0.000000, 0.850651, 0.525731],
    [-0.147621, 0.716567, 0.681718],
    [0.147621, 0.716567, 0.681718],
    [0.000000, 0.525731, 0.850651],
    [0.309017, 0.500000, 0.809017],
    [0.525731, 0.000000, 0.850651],
    [0.295242, 0.000000, 0.955423],
    [0.442863, 0.238856, 0.864188],
    [0.162460, 0.262866, 0.951056],
    [-0.681718, 0.147621, 0.716567],
    [-0.809017, 0.309017, 0.500000],
    [-0.587785, 0.425325, 0.688191],
    [-0.850651, 0.525731, 0.000000],
    [-0.864188, 0.442863, 0.238856],
    [-0.716567, 0.681718, 0.147621],
    [-0.688191, 0.587785, 0.425325],
    [-0.500000, 0.809017, 0.309017],
    [-0.238856, 0.864188, 0.442863],
    [-0.425325, 0.688191, 0.587785],
    [-0.716567, 0.681718, -0.147621],
    [-0.500000, 0.809017, -0.309017],
    [-0.525731, 0.850651, 0.000000],
    [0.000000, 0.850651, -0.525731],
    [-0.238856, 0.864188, -0.442863],
    [0.000000, 0.955423, -0.295242],
    [-0.262866, 0.951056, -0.162460],
    [0.000000, 1.000000, 0.000000],
    [0.000000, 0.955423, 0.295242],
    [-0.262866, 0.951056, 0.162460],
    [0.238856, 0.864188, 0.442863],
    [0.262866, 0.951056, 0.162460],
    [0.500000, 0.809017, 0.309017],
    [0.238856, 0.864188, -0.442863],
    [0.262866, 0.951056, -0.162460],
    [0.500000, 0.809017, -0.309017],
    [0.850651, 0.525731, 0.000000],
    [0.716567, 0.681718, 0.147621],
    [0.716567, 0.681718, -0.147621],
    [0.525731, 0.850651, 0.000000],
    [0.425325, 0.688191, 0.587785],
    [0.864188, 0.442863, 0.238856],
    [0.688191, 0.587785, 0.425325],
    [0.809017, 0.309017, 0.500000],
    [0.681718, 0.147621, 0.716567],
    [0.587785, 0.425325, 0.688191],
    [0.955423, 0.295242, 0.000000],
    [1.000000, 0.000000, 0.000000],
    [0.951056, 0.162460, 0.262866],
    [0.850651, -0.525731, 0.000000],
    [0.955423, -0.295242, 0.000000],
    [0.864188, -0.442863, 0.238856],
    [0.951056, -0.162460, 0.262866],
    [0.809017, -0.309017, 0.500000],
    [0.681718, -0.147621, 0.716567],
    [0.850651, 0.000000, 0.525731],
    [0.864188, 0.442863, -0.238856],
    [0.809017, 0.309017, -0.500000],
    [0.951056, 0.162460, -0.262866],
    [0.525731, 0.000000, -0.850651],
    [0.681718, 0.147621, -0.716567],
    [0.681718, -0.147621, -0.716567],
    [0.850651, 0.000000, -0.525731],
    [0.809017, -0.309017, -0.500000],
    [0.864188, -0.442863, -0.238856],
    [0.951056, -0.162460, -0.262866],
    [0.147621, 0.716567, -0.681718],
    [0.309017, 0.500000, -0.809017],
    [0.425325, 0.688191, -0.587785],
    [0.442863, 0.238856, -0.864188],
    [0.587785, 0.425325, -0.688191],
    [0.688191, 0.587785, -0.425325],
    [-0.147621, 0.716567, -0.681718],
    [-0.309017, 0.500000, -0.809017],
    [0.000000, 0.525731, -0.850651],
    [-0.525731, 0.000000, -0.850651],
    [-0.442863, 0.238856, -0.864188],
    [-0.295242, 0.000000, -0.955423],
    [-0.162460, 0.262866, -0.951056],
    [0.000000, 0.000000, -1.000000],
    [0.295242, 0.000000, -0.955423],
    [0.162460, 0.262866, -0.951056],
    [-0.442863, -0.238856, -0.864188],
    [-0.309017, -0.500000, -0.809017],
    [-0.162460, -0.262866, -0.951056],
    [0.000000, -0.850651, -0.525731],
    [-0.147621, -0.716567, -0.681718],
    [0.147621, -0.716567, -0.681718],
    [0.000000, -0.525731, -0.850651],
    [0.309017, -0.500000, -0.809017],
    [0.442863, -0.238856, -0.864188],
    [0.162460, -0.262866, -0.951056],
    [0.238856, -0.864188, -0.442863],
    [0.500000, -0.809017, -0.309017],
    [0.425325, -0.688191, -0.587785],
    [0.716567, -0.681718, -0.147621],
    [0.688191, -0.587785, -0.425325],
    [0.587785, -0.425325, -0.688191],
    [0.000000, -0.955423, -0.295242],
    [0.000000, -1.000000, 0.000000],
    [0.262866, -0.951056, -0.162460],
    [0.000000, -0.850651, 0.525731],
    [0.000000, -0.955423, 0.295242],
    [0.238856, -0.864188, 0.442863],
    [0.262866, -0.951056, 0.162460],
    [0.500000, -0.809017, 0.309017],
    [0.716567, -0.681718, 0.147621],
    [0.525731, -0.850651, 0.000000],
    [-0.238856, -0.864188, -0.442863],
    [-0.500000, -0.809017, -0.309017],
    [-0.262866, -0.951056, -0.162460],
    [-0.850651, -0.525731, 0.000000],
    [-0.716567, -0.681718, -0.147621],
    [-0.716567, -0.681718, 0.147621],
    [-0.525731, -0.850651, 0.000000],
    [-0.500000, -0.809017, 0.309017],
    [-0.238856, -0.864188, 0.442863],
    [-0.262866, -0.951056, 0.162460],
    [-0.864188, -0.442863, 0.238856],
    [-0.809017, -0.309017, 0.500000],
    [-0.688191, -0.587785, 0.425325],
    [-0.681718, -0.147621, 0.716567],
    [-0.442863, -0.238856, 0.864188],
    [-0.587785, -0.425325, 0.688191],
    [-0.309017, -0.500000, 0.809017],
    [-0.147621, -0.716567, 0.681718],
    [-0.425325, -0.688191, 0.587785],
    [-0.162460, -0.262866, 0.951056],
    [0.442863, -0.238856, 0.864188],
    [0.162460, -0.262866, 0.951056],
    [0.309017, -0.500000, 0.809017],
    [0.147621, -0.716567, 0.681718],
    [0.000000, -0.525731, 0.850651],
    [0.425325, -0.688191, 0.587785],
    [0.587785, -0.425325, 0.688191],
    [0.688191, -0.587785, 0.425325],
    [-0.955423, 0.295242, 0.000000],
    [-0.951056, 0.162460, 0.262866],
    [-1.000000, 0.000000, 0.000000],
    [-0.850651, 0.000000, 0.525731],
    [-0.955423, -0.295242, 0.000000],
    [-0.951056, -0.162460, 0.262866],
    [-0.864188, 0.442863, -0.238856],
    [-0.951056, 0.162460, -0.262866],
    [-0.809017, 0.309017, -0.500000],
    [-0.864188, -0.442863, -0.238856],
    [-0.951056, -0.162460, -0.262866],
    [-0.809017, -0.309017, -0.500000],
    [-0.681718, 0.147621, -0.716567],
    [-0.681718, -0.147621, -0.716567],
    [-0.850651, 0.000000, -0.525731],
    [-0.688191, 0.587785, -0.425325],
    [-0.587785, 0.425325, -0.688191],
    [-0.425325, 0.688191, -0.587785],
    [-0.425325, -0.688191, -0.587785],
    [-0.587785, -0.425325, -0.688191],
    [-0.688191, -0.587785, -0.425325],
];

/// `byteToDirection`: table lookup, zero outside the table.
#[must_use]
pub fn byte_to_direction(byte: i32) -> Vec3 {
    if byte < 0 {
        return vec3(0.0, 0.0, 0.0);
    }
    BYTE_DIRECTIONS
        .get(byte as usize)
        .map_or(vec3(0.0, 0.0, 0.0), |row| vec3(row[0], row[1], row[2]))
}

// ---------------------------------------------------------------------------
// Entity state (SIBLING-MIRROR of network/q3/state/entity.ts)
// ---------------------------------------------------------------------------

/// Owned `entityState_t` storage (`EntityState`).
#[derive(Debug, Clone, PartialEq)]
pub struct EntityState {
    /// Entity number.
    pub number: i32,
    /// Entity type tag.
    pub e_type: i32,
    /// Entity flags.
    pub e_flags: i32,
    /// Position trajectory.
    pub pos: Trajectory,
    /// Angle trajectory.
    pub apos: Trajectory,
    /// Time.
    pub time: i32,
    /// Secondary time.
    pub time2: i32,
    /// Origin.
    pub origin: Vec3,
    /// Secondary origin.
    pub origin2: Vec3,
    /// Angles.
    pub angles: Vec3,
    /// Secondary angles.
    pub angles2: Vec3,
    /// Other entity number.
    pub other_entity_num: i32,
    /// Second other entity number.
    pub other_entity_num2: i32,
    /// Ground entity number.
    pub ground_entity_num: i32,
    /// Constant light.
    pub constant_light: i32,
    /// Loop sound.
    pub loop_sound: i32,
    /// Model index.
    pub modelindex: i32,
    /// Second model index.
    pub modelindex2: i32,
    /// Client number.
    pub client_num: i32,
    /// Frame.
    pub frame: i32,
    /// Solid encoding.
    pub solid: i32,
    /// Event with sequence bits.
    pub event: i32,
    /// Event parameter.
    pub event_parm: i32,
    /// Powerup bitmask.
    pub powerups: i32,
    /// Weapon tag.
    pub weapon: i32,
    /// Legs animation.
    pub legs_anim: i32,
    /// Torso animation.
    pub torso_anim: i32,
    /// Generic value.
    pub generic1: i32,
}

impl Default for EntityState {
    fn default() -> Self {
        Self {
            number: 0,
            e_type: 0,
            e_flags: 0,
            pos: Trajectory::default(),
            apos: Trajectory::default(),
            time: 0,
            time2: 0,
            origin: vec3(0.0, 0.0, 0.0),
            origin2: vec3(0.0, 0.0, 0.0),
            angles: vec3(0.0, 0.0, 0.0),
            angles2: vec3(0.0, 0.0, 0.0),
            other_entity_num: 0,
            other_entity_num2: 0,
            ground_entity_num: 0,
            constant_light: 0,
            loop_sound: 0,
            modelindex: 0,
            modelindex2: 0,
            client_num: 0,
            frame: 0,
            solid: 0,
            event: 0,
            event_parm: 0,
            powerups: 0,
            weapon: 0,
            legs_anim: 0,
            torso_anim: 0,
            generic1: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Player state (SIBLING-MIRROR of base/shared/player-state.ts)
// ---------------------------------------------------------------------------

/// Fixed source slot array with checked access (`PlayerStateSlots`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerStateSlots {
    values: Vec<i32>,
}

impl PlayerStateSlots {
    /// Zeroed slots of a fixed length.
    #[must_use]
    pub fn new(length: usize) -> Self {
        Self {
            values: vec![0; length],
        }
    }

    /// Slot count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Whether there are no slots.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Checked read.
    pub fn get(&self, index: i32) -> PresentResult<i32> {
        if index < 0 {
            return Err(range_msg(format!(
                "Player state slot {index} outside {}",
                self.values.len()
            )));
        }
        at(&self.values, index as usize, "Player state slot").copied()
    }

    /// Checked write.
    pub fn set(&mut self, index: i32, value: i32) -> PresentResult<()> {
        if index < 0 {
            return Err(range_msg(format!(
                "Player state slot {index} outside {}",
                self.values.len()
            )));
        }
        let slot = at_mut(&mut self.values, index as usize, "Player state slot")?;
        *slot = value;
        Ok(())
    }

    fn copy_from_slots(&mut self, source: &Self) -> PresentResult<()> {
        if self.len() != source.len() {
            return Err(range_msg("Player state slot length mismatch"));
        }
        self.values.copy_from_slice(&source.values);
        Ok(())
    }
}

/// Predictable event debug module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventDebugModule {
    /// Game module.
    Game,
    /// Cgame module.
    Cgame,
}

/// Predictable event debug sink (`PredictableEventDebug`).
#[derive(Clone)]
pub struct PredictableEventDebug {
    /// Owning module label.
    pub module: EventDebugModule,
    /// Live `showevents` value.
    pub show_events: Rc<dyn Fn() -> String>,
    /// Debug print.
    pub print: Rc<RefCell<dyn FnMut(String)>>,
}

impl std::fmt::Debug for PredictableEventDebug {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PredictableEventDebug")
            .field("module", &self.module)
            .finish_non_exhaustive()
    }
}

/// Recorded predictable event (`PredictableEvent`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PredictableEvent {
    /// Sequence number.
    pub sequence: i32,
    /// Event.
    pub event: i32,
    /// Parameter.
    pub parameter: i32,
}

// `bg_misc.c:eventnames`, including its omitted `EV_OBELISKPAIN` entry.
pub(crate) const EVENT_NAMES: [&str; 76] = [
    "EV_NONE",
    "EV_FOOTSTEP",
    "EV_FOOTSTEP_METAL",
    "EV_FOOTSPLASH",
    "EV_FOOTWADE",
    "EV_SWIM",
    "EV_STEP_4",
    "EV_STEP_8",
    "EV_STEP_12",
    "EV_STEP_16",
    "EV_FALL_SHORT",
    "EV_FALL_MEDIUM",
    "EV_FALL_FAR",
    "EV_JUMP_PAD",
    "EV_JUMP",
    "EV_WATER_TOUCH",
    "EV_WATER_LEAVE",
    "EV_WATER_UNDER",
    "EV_WATER_CLEAR",
    "EV_ITEM_PICKUP",
    "EV_GLOBAL_ITEM_PICKUP",
    "EV_NOAMMO",
    "EV_CHANGE_WEAPON",
    "EV_FIRE_WEAPON",
    "EV_USE_ITEM0",
    "EV_USE_ITEM1",
    "EV_USE_ITEM2",
    "EV_USE_ITEM3",
    "EV_USE_ITEM4",
    "EV_USE_ITEM5",
    "EV_USE_ITEM6",
    "EV_USE_ITEM7",
    "EV_USE_ITEM8",
    "EV_USE_ITEM9",
    "EV_USE_ITEM10",
    "EV_USE_ITEM11",
    "EV_USE_ITEM12",
    "EV_USE_ITEM13",
    "EV_USE_ITEM14",
    "EV_USE_ITEM15",
    "EV_ITEM_RESPAWN",
    "EV_ITEM_POP",
    "EV_PLAYER_TELEPORT_IN",
    "EV_PLAYER_TELEPORT_OUT",
    "EV_GRENADE_BOUNCE",
    "EV_GENERAL_SOUND",
    "EV_GLOBAL_SOUND",
    "EV_GLOBAL_TEAM_SOUND",
    "EV_BULLET_HIT_FLESH",
    "EV_BULLET_HIT_WALL",
    "EV_MISSILE_HIT",
    "EV_MISSILE_MISS",
    "EV_MISSILE_MISS_METAL",
    "EV_RAILTRAIL",
    "EV_SHOTGUN",
    "EV_BULLET",
    "EV_PAIN",
    "EV_DEATH1",
    "EV_DEATH2",
    "EV_DEATH3",
    "EV_OBITUARY",
    "EV_POWERUP_QUAD",
    "EV_POWERUP_BATTLESUIT",
    "EV_POWERUP_REGEN",
    "EV_GIB_PLAYER",
    "EV_SCOREPLUM",
    "EV_PROXIMITY_MINE_STICK",
    "EV_PROXIMITY_MINE_TRIGGER",
    "EV_KAMIKAZE",
    "EV_OBELISKEXPLODE",
    "EV_INVUL_IMPACT",
    "EV_JUICED",
    "EV_LIGHTNINGBOLT",
    "EV_DEBUG_LINE",
    "EV_STOPLOOPINGSOUND",
    "EV_TAUNT",
];

/// Owned `playerState_t` storage (`SourcePlayerState`).
#[derive(Debug)]
pub struct PlayerState {
    /// Product.
    pub product: Q3Product,
    /// Command time.
    pub command_time: i32,
    /// Movement type tag.
    pub pm_type: i32,
    /// Bob cycle.
    pub bob_cycle: i32,
    /// Movement flags.
    pub pm_flags: i32,
    /// Movement timer.
    pub pm_time: i32,
    /// Origin.
    pub origin: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Weapon time.
    pub weapon_time: i32,
    /// Gravity.
    pub gravity: i32,
    /// Speed.
    pub speed: i32,
    /// Delta angles.
    pub delta_angles: Vec3,
    /// Ground entity number.
    pub ground_entity_num: i32,
    /// Legs timer.
    pub legs_timer: i32,
    /// Legs animation.
    pub legs_anim: i32,
    /// Torso timer.
    pub torso_timer: i32,
    /// Torso animation.
    pub torso_anim: i32,
    /// Movement direction.
    pub movement_dir: i32,
    /// Grapple point.
    pub grapple_point: Vec3,
    /// Entity flags.
    pub e_flags: i32,
    /// Event sequence.
    pub event_sequence: i32,
    /// Predictable events ring (2).
    pub events: PlayerStateSlots,
    /// Predictable event parameters ring (2).
    pub event_parms: PlayerStateSlots,
    /// External event.
    pub external_event: i32,
    /// External event parameter.
    pub external_event_parm: i32,
    /// External event time.
    pub external_event_time: i32,
    /// Client number.
    pub client_num: i32,
    /// Weapon tag.
    pub weapon: i32,
    /// Weapon state tag.
    pub weapon_state: i32,
    /// View angles.
    pub viewangles: Vec3,
    /// View height.
    pub viewheight: i32,
    /// Damage event.
    pub damage_event: i32,
    /// Damage yaw byte.
    pub damage_yaw: i32,
    /// Damage pitch byte.
    pub damage_pitch: i32,
    /// Damage count.
    pub damage_count: i32,
    /// Stats (16).
    pub stats: PlayerStateSlots,
    /// Persistant stats (16).
    pub persistant: PlayerStateSlots,
    /// Powerup times (16).
    pub powerups: PlayerStateSlots,
    /// Ammo (16).
    pub ammo: PlayerStateSlots,
    /// Generic value.
    pub generic1: i32,
    /// Loop sound.
    pub loop_sound: i32,
    /// Jump-pad entity.
    pub jumppad_ent: i32,
    /// Ping.
    pub ping: i32,
    /// Pmove frame count.
    pub pmove_framecount: i32,
    /// Jump-pad frame.
    pub jumppad_frame: i32,
    /// Entity event sequence.
    pub entity_event_sequence: i32,
    event_debug: Option<PredictableEventDebug>,
}

impl Clone for PlayerState {
    fn clone(&self) -> Self {
        let mut copy = Self::new(self.product);
        copy.copy_from_state(self).expect("same-shape player-state copy");
        copy
    }
}

impl PlayerState {
    /// Zeroed retail record (`new PlayerStateRecord(product, 0, 0, 0)`).
    #[must_use]
    pub fn new(product: Q3Product) -> Self {
        Self {
            product,
            command_time: 0,
            pm_type: 0,
            bob_cycle: 0,
            pm_flags: 0,
            pm_time: 0,
            origin: vec3(0.0, 0.0, 0.0),
            velocity: vec3(0.0, 0.0, 0.0),
            weapon_time: 0,
            gravity: 0,
            speed: 0,
            delta_angles: vec3(0.0, 0.0, 0.0),
            ground_entity_num: 0,
            legs_timer: 0,
            legs_anim: 0,
            torso_timer: 0,
            torso_anim: 0,
            movement_dir: 0,
            grapple_point: vec3(0.0, 0.0, 0.0),
            e_flags: 0,
            event_sequence: 0,
            events: PlayerStateSlots::new(2),
            event_parms: PlayerStateSlots::new(2),
            external_event: 0,
            external_event_parm: 0,
            external_event_time: 0,
            client_num: 0,
            weapon: 0,
            weapon_state: 0,
            viewangles: vec3(0.0, 0.0, 0.0),
            viewheight: 0,
            damage_event: 0,
            damage_yaw: 0,
            damage_pitch: 0,
            damage_count: 0,
            stats: PlayerStateSlots::new(16),
            persistant: PlayerStateSlots::new(16),
            powerups: PlayerStateSlots::new(16),
            ammo: PlayerStateSlots::new(16),
            generic1: 0,
            loop_sound: 0,
            jumppad_ent: 0,
            ping: 0,
            pmove_framecount: 0,
            jumppad_frame: 0,
            entity_event_sequence: 0,
            event_debug: None,
        }
    }

    /// Deep copy (`copy`; the debug sink never transfers).
    #[must_use]
    pub fn copy(&self) -> Self {
        self.clone()
    }

    /// Full field copy (`copyFrom` without engine authority bindings).
    pub fn copy_from_state(&mut self, source: &Self) -> PresentResult<()> {
        self.product = source.product;
        self.command_time = source.command_time;
        self.pm_type = source.pm_type;
        self.bob_cycle = source.bob_cycle;
        self.pm_flags = source.pm_flags;
        self.pm_time = source.pm_time;
        self.origin = source.origin;
        self.velocity = source.velocity;
        self.weapon_time = source.weapon_time;
        self.gravity = source.gravity;
        self.speed = source.speed;
        self.delta_angles = source.delta_angles;
        self.ground_entity_num = source.ground_entity_num;
        self.legs_timer = source.legs_timer;
        self.legs_anim = source.legs_anim;
        self.torso_timer = source.torso_timer;
        self.torso_anim = source.torso_anim;
        self.movement_dir = source.movement_dir;
        self.grapple_point = source.grapple_point;
        self.e_flags = source.e_flags;
        self.event_sequence = source.event_sequence;
        self.events.copy_from_slots(&source.events)?;
        self.event_parms.copy_from_slots(&source.event_parms)?;
        self.external_event = source.external_event;
        self.external_event_parm = source.external_event_parm;
        self.external_event_time = source.external_event_time;
        self.client_num = source.client_num;
        self.weapon = source.weapon;
        self.weapon_state = source.weapon_state;
        self.viewangles = source.viewangles;
        self.viewheight = source.viewheight;
        self.damage_event = source.damage_event;
        self.damage_yaw = source.damage_yaw;
        self.damage_pitch = source.damage_pitch;
        self.damage_count = source.damage_count;
        self.stats.copy_from_slots(&source.stats)?;
        self.persistant.copy_from_slots(&source.persistant)?;
        self.powerups.copy_from_slots(&source.powerups)?;
        self.ammo.copy_from_slots(&source.ammo)?;
        self.generic1 = source.generic1;
        self.loop_sound = source.loop_sound;
        self.jumppad_ent = source.jumppad_ent;
        self.ping = source.ping;
        self.pmove_framecount = source.pmove_framecount;
        self.jumppad_frame = source.jumppad_frame;
        self.entity_event_sequence = source.entity_event_sequence;
        Ok(())
    }

    /// Health stat.
    pub fn health(&self) -> PresentResult<i32> {
        self.stats.get(stat_schema(self.product).health)
    }

    /// Write the health stat.
    pub fn set_health(&mut self, value: i32) -> PresentResult<()> {
        let slot = stat_schema(self.product).health;
        self.stats.set(slot, value)
    }

    /// Install or clear the predictable-event debug sink.
    pub fn set_event_debug(&mut self, debug: Option<PredictableEventDebug>) {
        self.event_debug = debug;
    }

    /// Append a predictable event (`addEvent`).
    pub fn add_event(&mut self, event: i32, parameter: i32) -> PresentResult<PredictableEvent> {
        if let Some(debug) = &self.event_debug {
            let text = (debug.show_events)();
            let head = text.get(..text.len().min(255)).unwrap_or("");
            if native_atof(head) != 0.0 {
                let name = if event < 0 {
                    None
                } else {
                    EVENT_NAMES.get(event as usize).copied()
                };
                let Some(name) = name else {
                    return Err(range_msg(format!("bg_misc.c eventnames has no entry for {event}")));
                };
                let label = match debug.module {
                    EventDebugModule::Game => " game",
                    EventDebugModule::Cgame => "Cgame",
                };
                (debug.print.borrow_mut())(format!(
                    "{label} event svt {:5} -> {:5}: num = {name:>20} parm {parameter}\n",
                    self.pmove_framecount, self.event_sequence
                ));
            }
        }
        let sequence = self.event_sequence;
        self.events.set(sequence & 1, event)?;
        self.event_parms.set(sequence & 1, parameter)?;
        self.event_sequence = sequence.wrapping_add(1);
        Ok(PredictableEvent {
            sequence,
            event,
            parameter,
        })
    }
}

/// User command (`UserCommand`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UserCommand {
    /// Server time.
    pub server_time: i32,
    /// Angles.
    pub angles: Vec3,
    /// Buttons.
    pub buttons: i32,
    /// Weapon tag.
    pub weapon: i32,
    /// Forward move.
    pub forwardmove: i32,
    /// Right move.
    pub rightmove: i32,
    /// Up move.
    pub upmove: i32,
}

impl Default for UserCommand {
    fn default() -> Self {
        Self {
            server_time: 0,
            angles: vec3(0.0, 0.0, 0.0),
            buttons: 0,
            weapon: Weapon::None as i32,
            forwardmove: 0,
            rightmove: 0,
            upmove: 0,
        }
    }
}

pub(crate) fn source_snap_component(component: f32) -> i32 {
    if (-2147483648.0..2147483648.0).contains(&component) {
        component.trunc() as i32
    } else {
        i32::MIN
    }
}

pub(crate) fn copy_position(value: Vec3, snap: bool) -> Vec3 {
    if snap {
        vec3(
            source_snap_component(value.x) as f32,
            source_snap_component(value.y) as f32,
            source_snap_component(value.z) as f32,
        )
    } else {
        value
    }
}

/// `BG_PlayerStateToEntityState` (SIBLING-MIRROR of snapshot-state.ts).
pub fn player_state_to_entity_state(
    ps: &mut PlayerState,
    destination: &mut EntityState,
    snap: bool,
) -> PresentResult<()> {
    let invisible = ps.pm_type == MoveType::Intermission as i32
        || ps.pm_type == MoveType::Spectator as i32
        || ps.health()? <= GIB_HEALTH;
    destination.e_type = if invisible {
        EntityType::Invisible as i32
    } else {
        EntityType::Player as i32
    };
    destination.number = ps.client_num;
    destination.pos = Trajectory {
        type_tag: TrajectoryType::Interpolate as i32,
        base: copy_position(ps.origin, snap),
        delta: ps.velocity,
        time: destination.pos.time,
        duration: destination.pos.duration,
    };
    destination.apos.type_tag = TrajectoryType::Interpolate as i32;
    destination.apos.base = copy_position(ps.viewangles, snap);
    destination.angles2.y = ps.movement_dir as f32;
    destination.legs_anim = ps.legs_anim;
    destination.torso_anim = ps.torso_anim;
    destination.client_num = ps.client_num;
    destination.e_flags = if ps.health()? <= 0 {
        ps.e_flags | 1
    } else {
        ps.e_flags & !1
    };
    if ps.external_event != 0 {
        destination.event = ps.external_event;
        destination.event_parm = ps.external_event_parm;
    } else if ps.entity_event_sequence < ps.event_sequence {
        let oldest = ps.event_sequence.wrapping_sub(2);
        if ps.entity_event_sequence < oldest {
            ps.entity_event_sequence = oldest;
        }
        let slot = ps.entity_event_sequence & 1;
        destination.event = ps.events.get(slot)? | ((ps.entity_event_sequence & 3) << 8);
        destination.event_parm = ps.event_parms.get(slot)?;
        ps.entity_event_sequence = ps.entity_event_sequence.wrapping_add(1);
    }
    destination.weapon = ps.weapon;
    destination.ground_entity_num = ps.ground_entity_num;
    destination.powerups = 0;
    for index in 0..ps.powerups.len() as i32 {
        if ps.powerups.get(index)? != 0 {
            destination.powerups |= 1 << index;
        }
    }
    destination.loop_sound = ps.loop_sound;
    destination.generic1 = ps.generic1;
    Ok(())
}

// ---------------------------------------------------------------------------
// Items (SIBLING-MIRROR of base/shared/items.ts: table substance plus the
// grab/touch/find helpers cgame calls)
// ---------------------------------------------------------------------------

/// Item row substance used by prediction and events (`ItemDefinition`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemMirror {
    /// Item type.
    pub item_type: ItemType,
    /// Weapon, powerup, or holdable tag (0 otherwise).
    pub tag: i32,
    /// Quantity.
    pub quantity: i32,
    /// Pickup name.
    pub pickup_name: Option<&'static str>,
    /// Pickup sound path.
    pub pickup_sound: Option<&'static str>,
}

pub(crate) const fn item(
    item_type: ItemType,
    tag: i32,
    quantity: i32,
    pickup_name: Option<&'static str>,
    pickup_sound: Option<&'static str>,
) -> ItemMirror {
    ItemMirror {
        item_type,
        tag,
        quantity,
        pickup_name,
        pickup_sound,
    }
}

/// Full item table; baseq3 uses the first 36 rows.
pub static Q3_ITEMS: [ItemMirror; 52] = [
    item(ItemType::Bad, 0, 0, None, None),
    item(
        ItemType::Armor,
        0,
        5,
        Some("Armor Shard"),
        Some("sound/misc/ar1_pkup.wav"),
    ),
    item(ItemType::Armor, 0, 50, Some("Armor"), Some("sound/misc/ar2_pkup.wav")),
    item(
        ItemType::Armor,
        0,
        100,
        Some("Heavy Armor"),
        Some("sound/misc/ar2_pkup.wav"),
    ),
    item(
        ItemType::Health,
        0,
        5,
        Some("5 Health"),
        Some("sound/items/s_health.wav"),
    ),
    item(
        ItemType::Health,
        0,
        25,
        Some("25 Health"),
        Some("sound/items/n_health.wav"),
    ),
    item(
        ItemType::Health,
        0,
        50,
        Some("50 Health"),
        Some("sound/items/l_health.wav"),
    ),
    item(
        ItemType::Health,
        0,
        100,
        Some("Mega Health"),
        Some("sound/items/m_health.wav"),
    ),
    item(
        ItemType::Weapon,
        Weapon::Gauntlet as i32,
        0,
        Some("Gauntlet"),
        Some("sound/misc/w_pkup.wav"),
    ),
    item(
        ItemType::Weapon,
        Weapon::Shotgun as i32,
        10,
        Some("Shotgun"),
        Some("sound/misc/w_pkup.wav"),
    ),
    item(
        ItemType::Weapon,
        Weapon::Machinegun as i32,
        40,
        Some("Machinegun"),
        Some("sound/misc/w_pkup.wav"),
    ),
    item(
        ItemType::Weapon,
        Weapon::GrenadeLauncher as i32,
        10,
        Some("Grenade Launcher"),
        Some("sound/misc/w_pkup.wav"),
    ),
    item(
        ItemType::Weapon,
        Weapon::RocketLauncher as i32,
        10,
        Some("Rocket Launcher"),
        Some("sound/misc/w_pkup.wav"),
    ),
    item(
        ItemType::Weapon,
        Weapon::Lightning as i32,
        100,
        Some("Lightning Gun"),
        Some("sound/misc/w_pkup.wav"),
    ),
    item(
        ItemType::Weapon,
        Weapon::Railgun as i32,
        10,
        Some("Railgun"),
        Some("sound/misc/w_pkup.wav"),
    ),
    item(
        ItemType::Weapon,
        Weapon::Plasmagun as i32,
        50,
        Some("Plasma Gun"),
        Some("sound/misc/w_pkup.wav"),
    ),
    item(
        ItemType::Weapon,
        Weapon::Bfg as i32,
        20,
        Some("BFG10K"),
        Some("sound/misc/w_pkup.wav"),
    ),
    item(
        ItemType::Weapon,
        Weapon::GrapplingHook as i32,
        0,
        Some("Grappling Hook"),
        Some("sound/misc/w_pkup.wav"),
    ),
    item(
        ItemType::Ammo,
        Weapon::Shotgun as i32,
        10,
        Some("Shells"),
        Some("sound/misc/am_pkup.wav"),
    ),
    item(
        ItemType::Ammo,
        Weapon::Machinegun as i32,
        50,
        Some("Bullets"),
        Some("sound/misc/am_pkup.wav"),
    ),
    item(
        ItemType::Ammo,
        Weapon::GrenadeLauncher as i32,
        5,
        Some("Grenades"),
        Some("sound/misc/am_pkup.wav"),
    ),
    item(
        ItemType::Ammo,
        Weapon::Plasmagun as i32,
        30,
        Some("Cells"),
        Some("sound/misc/am_pkup.wav"),
    ),
    item(
        ItemType::Ammo,
        Weapon::Lightning as i32,
        60,
        Some("Lightning"),
        Some("sound/misc/am_pkup.wav"),
    ),
    item(
        ItemType::Ammo,
        Weapon::RocketLauncher as i32,
        5,
        Some("Rockets"),
        Some("sound/misc/am_pkup.wav"),
    ),
    item(
        ItemType::Ammo,
        Weapon::Railgun as i32,
        10,
        Some("Slugs"),
        Some("sound/misc/am_pkup.wav"),
    ),
    item(
        ItemType::Ammo,
        Weapon::Bfg as i32,
        15,
        Some("Bfg Ammo"),
        Some("sound/misc/am_pkup.wav"),
    ),
    item(
        ItemType::Holdable,
        Holdable::Teleporter as i32,
        60,
        Some("Personal Teleporter"),
        Some("sound/items/holdable.wav"),
    ),
    item(
        ItemType::Holdable,
        Holdable::Medkit as i32,
        60,
        Some("Medkit"),
        Some("sound/items/holdable.wav"),
    ),
    item(
        ItemType::Powerup,
        Powerup::Quad as i32,
        30,
        Some("Quad Damage"),
        Some("sound/items/quaddamage.wav"),
    ),
    item(
        ItemType::Powerup,
        Powerup::Battlesuit as i32,
        30,
        Some("Battle Suit"),
        Some("sound/items/protect.wav"),
    ),
    item(
        ItemType::Powerup,
        Powerup::Haste as i32,
        30,
        Some("Speed"),
        Some("sound/items/haste.wav"),
    ),
    item(
        ItemType::Powerup,
        Powerup::Invis as i32,
        30,
        Some("Invisibility"),
        Some("sound/items/invisibility.wav"),
    ),
    item(
        ItemType::Powerup,
        Powerup::Regen as i32,
        30,
        Some("Regeneration"),
        Some("sound/items/regeneration.wav"),
    ),
    item(
        ItemType::Powerup,
        Powerup::Flight as i32,
        60,
        Some("Flight"),
        Some("sound/items/flight.wav"),
    ),
    item(ItemType::Team, Powerup::RedFlag as i32, 0, Some("Red Flag"), None),
    item(ItemType::Team, Powerup::BlueFlag as i32, 0, Some("Blue Flag"), None),
    item(
        ItemType::Holdable,
        Holdable::Kamikaze as i32,
        60,
        Some("Kamikaze"),
        Some("sound/items/holdable.wav"),
    ),
    item(
        ItemType::Holdable,
        Holdable::Portal as i32,
        60,
        Some("Portal"),
        Some("sound/items/holdable.wav"),
    ),
    item(
        ItemType::Holdable,
        Holdable::Invulnerability as i32,
        60,
        Some("Invulnerability"),
        Some("sound/items/holdable.wav"),
    ),
    item(
        ItemType::Ammo,
        Weapon::Nailgun as i32,
        20,
        Some("Nails"),
        Some("sound/misc/am_pkup.wav"),
    ),
    item(
        ItemType::Ammo,
        Weapon::ProxLauncher as i32,
        10,
        Some("Proximity Mines"),
        Some("sound/misc/am_pkup.wav"),
    ),
    item(
        ItemType::Ammo,
        Weapon::Chaingun as i32,
        100,
        Some("Chaingun Belt"),
        Some("sound/misc/am_pkup.wav"),
    ),
    item(
        ItemType::PersistantPowerup,
        Powerup::Scout as i32,
        30,
        Some("Scout"),
        Some("sound/items/scout.wav"),
    ),
    item(
        ItemType::PersistantPowerup,
        Powerup::Guard as i32,
        30,
        Some("Guard"),
        Some("sound/items/guard.wav"),
    ),
    item(
        ItemType::PersistantPowerup,
        Powerup::Doubler as i32,
        30,
        Some("Doubler"),
        Some("sound/items/doubler.wav"),
    ),
    item(
        ItemType::PersistantPowerup,
        Powerup::Ammoregen as i32,
        30,
        Some("Ammo Regen"),
        Some("sound/items/ammoregen.wav"),
    ),
    item(
        ItemType::Team,
        Powerup::NeutralFlag as i32,
        0,
        Some("Neutral Flag"),
        None,
    ),
    item(ItemType::Team, 0, 0, Some("Red Cube"), Some("sound/misc/am_pkup.wav")),
    item(ItemType::Team, 0, 0, Some("Blue Cube"), Some("sound/misc/am_pkup.wav")),
    item(
        ItemType::Weapon,
        Weapon::Nailgun as i32,
        10,
        Some("Nailgun"),
        Some("sound/misc/w_pkup.wav"),
    ),
    item(
        ItemType::Weapon,
        Weapon::ProxLauncher as i32,
        5,
        Some("Prox Launcher"),
        Some("sound/misc/w_pkup.wav"),
    ),
    item(
        ItemType::Weapon,
        Weapon::Chaingun as i32,
        80,
        Some("Chaingun"),
        Some("sound/misc/w_pkup.wav"),
    ),
];

/// Item list for a product (`itemList`).
#[must_use]
pub fn item_list(product: Q3Product) -> &'static [ItemMirror] {
    match product {
        Q3Product::BaseQ3 => &Q3_ITEMS[..36],
        Q3Product::MissionPack => &Q3_ITEMS[..],
    }
}

/// Item by index (`itemAt`).
pub fn item_at(product: Q3Product, index: i32) -> PresentResult<&'static ItemMirror> {
    if index < 0 {
        return Err(range_msg(format!("Item index out of range: {index}")));
    }
    item_list(product)
        .get(index as usize)
        .ok_or_else(|| range_msg(format!("Item index out of range: {index}")))
}

/// Item for a holdable tag (`findItemForHoldable`).
pub fn find_item_for_holdable(product: Q3Product, holdable: i32) -> PresentResult<&'static ItemMirror> {
    item_list(product)
        .iter()
        .find(|item| item.item_type == ItemType::Holdable && item.tag == holdable)
        .ok_or_else(|| drop_msg("HoldableItem not found"))
}

/// Pickup entity substance (`PickupEntity`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PickupEntity {
    /// Item model index.
    pub model_index: i32,
    /// Second model index.
    pub model_index2: i32,
    /// Generic value.
    pub generic1: i32,
}

/// Player inventory view (`PlayerInventory`).
#[derive(Debug, Clone, Copy)]
pub struct PlayerInventory<'a> {
    /// Product.
    pub product: Q3Product,
    /// Health.
    pub health: i32,
    /// Armor.
    pub armor: i32,
    /// Max health.
    pub max_health: i32,
    /// Holdable item.
    pub holdable_item: i32,
    /// Team tag.
    pub team: i32,
    /// Mission-pack persistent-powerup item index.
    pub persistent_powerup_index: i32,
    ammo: &'a PlayerStateSlots,
    powerups: &'a PlayerStateSlots,
}

impl<'a> PlayerInventory<'a> {
    /// Build an inventory view over a player state.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        product: Q3Product,
        health: i32,
        armor: i32,
        max_health: i32,
        holdable_item: i32,
        team: i32,
        persistent_powerup_index: i32,
        ammo: &'a PlayerStateSlots,
        powerups: &'a PlayerStateSlots,
    ) -> Self {
        Self {
            product,
            health,
            armor,
            max_health,
            holdable_item,
            team,
            persistent_powerup_index,
            ammo,
            powerups,
        }
    }

    /// Ammo for a weapon tag.
    pub fn ammo(&self, weapon: i32) -> PresentResult<i32> {
        self.ammo.get(weapon)
    }

    /// Powerup time for a powerup tag.
    pub fn powerup(&self, powerup: i32) -> PresentResult<i32> {
        self.powerups.get(powerup)
    }
}

/// `BG_CanQ3ArmorBeGrabbed` armor rule.
pub fn can_q3_armor_be_grabbed(ps: &PlayerInventory) -> PresentResult<bool> {
    if ps.product == Q3Product::MissionPack {
        if item_at(ps.product, ps.persistent_powerup_index)?.tag == Powerup::Scout as i32 {
            return Ok(false);
        }
        let upper = if item_at(ps.product, ps.persistent_powerup_index)?.tag == Powerup::Guard as i32 {
            ps.max_health
        } else {
            ps.max_health.wrapping_mul(2)
        };
        return Ok(ps.armor < upper);
    }
    Ok(ps.armor < ps.max_health.wrapping_mul(2))
}

/// `BG_CanItemBeGrabbed` grab rule.
pub fn can_item_be_grabbed(gametype: i32, ent: &PickupEntity, ps: &PlayerInventory) -> PresentResult<bool> {
    if ent.model_index < 1 || ent.model_index >= item_list(ps.product).len() as i32 {
        return Err(drop_msg("BG_CanItemBeGrabbed: index out of range"));
    }
    let item = item_at(ps.product, ent.model_index)?;
    match item.item_type {
        ItemType::Weapon => Ok(true),
        ItemType::Ammo => Ok(ps.ammo(item.tag)? < 200),
        ItemType::Armor => can_q3_armor_be_grabbed(ps),
        ItemType::Health => {
            if ps.product == Q3Product::MissionPack
                && item_at(ps.product, ps.persistent_powerup_index)?.tag == Powerup::Guard as i32
            {
                return Ok(ps.health < ps.max_health);
            }
            let scale = if item.quantity == 5 || item.quantity == 100 {
                2
            } else {
                1
            };
            Ok(ps.health < ps.max_health.wrapping_mul(scale))
        }
        ItemType::Powerup => Ok(true),
        ItemType::PersistantPowerup => {
            if ps.product == Q3Product::BaseQ3 || ps.persistent_powerup_index != 0 {
                return Ok(false);
            }
            if (ent.generic1 & 2) != 0 && ps.team != Team::Red as i32 {
                return Ok(false);
            }
            if (ent.generic1 & 4) != 0 && ps.team != Team::Blue as i32 {
                return Ok(false);
            }
            Ok(true)
        }
        ItemType::Team => {
            if ps.product == Q3Product::MissionPack && gametype == GameType::OneFctf as i32 {
                if item.tag == Powerup::NeutralFlag as i32 {
                    return Ok(true);
                }
                if ps.team == Team::Red as i32
                    && item.tag == Powerup::BlueFlag as i32
                    && ps.powerup(Powerup::NeutralFlag as i32)? != 0
                {
                    return Ok(true);
                }
                if ps.team == Team::Blue as i32
                    && item.tag == Powerup::RedFlag as i32
                    && ps.powerup(Powerup::NeutralFlag as i32)? != 0
                {
                    return Ok(true);
                }
            }
            if gametype == GameType::Ctf as i32 {
                if ps.team == Team::Red as i32 {
                    return Ok(item.tag == Powerup::BlueFlag as i32
                        || (item.tag == Powerup::RedFlag as i32
                            && (ent.model_index2 != 0 || ps.powerup(Powerup::BlueFlag as i32)? != 0)));
                }
                if ps.team == Team::Blue as i32 {
                    return Ok(item.tag == Powerup::RedFlag as i32
                        || (item.tag == Powerup::BlueFlag as i32
                            && (ent.model_index2 != 0 || ps.powerup(Powerup::RedFlag as i32)? != 0)));
                }
            }
            Ok(ps.product == Q3Product::MissionPack && gametype == GameType::Harvester as i32)
        }
        ItemType::Holdable => Ok(ps.holdable_item == 0),
        ItemType::Bad => Err(drop_msg("BG_CanItemBeGrabbed: IT_BAD")),
    }
}

/// Item touch box test (`playerTouchesItem`).
pub fn player_touches_item(player_origin: Vec3, item_position: &Trajectory, at_time: i32) -> PresentResult<bool> {
    let origin = evaluate_trajectory(item_position, at_time)?;
    let x = player_origin.x - origin.x;
    let y = player_origin.y - origin.y;
    let z = player_origin.z - origin.z;
    Ok(!(x > 44.0 || x < -50.0 || y > 36.0 || y < -36.0 || z > 36.0 || z < -36.0))
}

/// `BG_TouchJumpPad` (SIBLING-MIRROR of base/shared/jump-pad.ts).
pub fn touch_jump_pad(state: &mut PlayerState, jump_pad: &EntityState) -> PresentResult<()> {
    if state.pm_type != MoveType::Normal as i32 || state.powerups.get(Powerup::Flight as i32)? != 0 {
        return Ok(());
    }
    if state.jumppad_ent != jump_pad.number {
        let pitch = qa_core::math::angle_normalize180(f64::from(vector_to_angles(jump_pad.origin2).x)).abs();
        state.add_event(EntityEvent::JumpPad as i32, i32::from(pitch >= 45.0))?;
    }
    state.jumppad_ent = jump_pad.number;
    state.jumppad_frame = state.pmove_framecount;
    state.velocity = jump_pad.origin2;
    Ok(())
}

// ---------------------------------------------------------------------------
// Client info (SIBLING-MIRROR of presentation/client-info.ts)
// ---------------------------------------------------------------------------

/// Animation table length (`FLAG_STAND2RUN + 1`).
pub const CLIENT_ANIMATION_COUNT: usize = 37;

/// Client sound table length.
pub const CLIENT_SOUND_COUNT: usize = 32;

/// Per-client presentation info (`ClientInfo`).
#[derive(Debug, Clone)]
pub struct ClientInfo {
    /// Info valid.
    pub info_valid: bool,
    /// Name.
    pub name: String,
    /// Team.
    pub team: Team,
    /// Bot skill.
    pub bot_skill: i32,
    /// Shirt color.
    pub color1: Vec3,
    /// Pants color.
    pub color2: Vec3,
    /// Score.
    pub score: i32,
    /// Location.
    pub location: i32,
    /// Health.
    pub health: i32,
    /// Armor.
    pub armor: i32,
    /// Current weapon.
    pub cur_weapon: i32,
    /// Handicap.
    pub handicap: i32,
    /// Wins.
    pub wins: i32,
    /// Losses.
    pub losses: i32,
    /// Team task.
    pub team_task: i32,
    /// Team leader.
    pub team_leader: bool,
    /// Powerups.
    pub powerups: i32,
    /// Medkit usage time.
    pub medkit_usage_time: i32,
    /// Invulnerability start time.
    pub invulnerability_start_time: i32,
    /// Invulnerability stop time.
    pub invulnerability_stop_time: i32,
    /// Breath puff time.
    pub breath_puff_time: i32,
    /// Model name.
    pub model_name: String,
    /// Skin name.
    pub skin_name: String,
    /// Head model name.
    pub head_model_name: String,
    /// Head skin name.
    pub head_skin_name: String,
    /// Red team skin.
    pub red_team: String,
    /// Blue team skin.
    pub blue_team: String,
    /// Deferred load.
    pub deferred: bool,
    /// New-style animations (`tag_flag` present).
    pub new_anims: bool,
    /// Fixed legs.
    pub fixed_legs: bool,
    /// Fixed torso.
    pub fixed_torso: bool,
    /// Head offset.
    pub head_offset: Vec3,
    /// Footsteps.
    pub footsteps: PlayerFootsteps,
    /// Gender.
    pub gender: PlayerGender,
    /// Legs model.
    pub legs_model: SceneModel,
    /// Torso model.
    pub torso_model: SceneModel,
    /// Head model.
    pub head_model: SceneModel,
    /// Legs skin.
    pub legs_skin: Option<SceneSkin>,
    /// Torso skin.
    pub torso_skin: Option<SceneSkin>,
    /// Head skin.
    pub head_skin: Option<SceneSkin>,
    /// Model icon.
    pub model_icon: Option<SceneShader>,
    /// Animation cells.
    pub animations: Vec<Option<PlayerAnimation>>,
    /// Custom sounds.
    pub sounds: [Option<PcmSound>; CLIENT_SOUND_COUNT],
}

impl ClientInfo {
    /// Blank client info (`new ClientInfo()`).
    #[must_use]
    pub fn new() -> Self {
        Self {
            info_valid: false,
            name: String::new(),
            team: Team::Free,
            bot_skill: 0,
            color1: vec3(0.0, 0.0, 0.0),
            color2: vec3(0.0, 0.0, 0.0),
            score: 0,
            location: 0,
            health: 0,
            armor: 0,
            cur_weapon: 0,
            handicap: 0,
            wins: 0,
            losses: 0,
            team_task: 0,
            team_leader: false,
            powerups: 0,
            medkit_usage_time: 0,
            invulnerability_start_time: 0,
            invulnerability_stop_time: 0,
            breath_puff_time: 0,
            model_name: String::new(),
            skin_name: String::new(),
            head_model_name: String::new(),
            head_skin_name: String::new(),
            red_team: String::new(),
            blue_team: String::new(),
            deferred: false,
            new_anims: false,
            fixed_legs: false,
            fixed_torso: false,
            head_offset: vec3(0.0, 0.0, 0.0),
            footsteps: PlayerFootsteps::Normal,
            gender: PlayerGender::Male,
            legs_model: default_model(),
            torso_model: default_model(),
            head_model: default_model(),
            legs_skin: None,
            torso_skin: None,
            head_skin: None,
            model_icon: None,
            animations: vec![None; CLIENT_ANIMATION_COUNT],
            sounds: std::array::from_fn(|_| None),
        }
    }

    /// Install a parsed animation table (`setAnimations`).
    pub fn set_animations(&mut self, animations: Vec<Option<PlayerAnimation>>) -> PresentResult<()> {
        if animations.len() != CLIENT_ANIMATION_COUNT {
            return Err(range_msg("Client animation table has the wrong length"));
        }
        for (index, cell) in animations.iter().enumerate() {
            if cell.is_none() && index == PlayerAnimationNumber::TorsoNegative as usize + 1 {
                continue;
            }
            if cell.is_none() {
                return Err(range_msg(format!("Missing client animation cell {index}")));
            }
        }
        self.animations = animations;
        Ok(())
    }
}

impl Default for ClientInfo {
    fn default() -> Self {
        Self::new()
    }
}

/// Copy a client model between slots (`CG_CopyClientInfoModel`).
pub fn copy_client_model(from: &ClientInfo, to: &mut ClientInfo) -> PresentResult<()> {
    to.head_offset = from.head_offset;
    to.footsteps = from.footsteps;
    to.gender = from.gender;
    to.legs_model = from.legs_model.clone();
    to.legs_skin = from.legs_skin.clone();
    to.torso_model = from.torso_model.clone();
    to.torso_skin = from.torso_skin.clone();
    to.head_model = from.head_model.clone();
    to.head_skin = from.head_skin.clone();
    to.model_icon = from.model_icon.clone();
    to.new_anims = from.new_anims;
    to.set_animations(from.animations.clone())?;
    to.sounds = from.sounds;
    Ok(())
}

// ---------------------------------------------------------------------------
// Collision and movement mirrors (collision-host.ts, movement-host.ts)
// ---------------------------------------------------------------------------

/// Trace solidity (`TraceResult.solidity`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceSolidity {
    /// Clear.
    Clear,
    /// Start solid.
    StartSolid,
    /// All solid.
    AllSolid,
}

/// Trace contact (`TraceContact`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TraceContact {
    /// No contact.
    None,
    /// Plane contact.
    Plane {
        /// Contact plane.
        plane: Plane,
    },
}

/// Trace shape (`TraceShape`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TraceShape {
    /// Point.
    Point,
    /// Box.
    Box {
        /// Minimum corner.
        mins: Vec3,
        /// Maximum corner.
        maxs: Vec3,
    },
}

/// Trace query (`TraceQuery`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TraceQuery {
    /// Start.
    pub start: Vec3,
    /// End.
    pub end: Vec3,
    /// Shape.
    pub shape: TraceShape,
    /// Mask.
    pub mask: i32,
    /// Model index.
    pub model_index: Option<i32>,
}

/// Trace result (`TraceResult`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TraceResult {
    /// Fraction.
    pub fraction: f32,
    /// End position.
    pub end: Vec3,
    /// Solidity.
    pub solidity: TraceSolidity,
    /// Contact.
    pub contact: TraceContact,
    /// Contents.
    pub contents: i32,
    /// Surface flags.
    pub surface_flags: i32,
}

/// Movement trace with entity number (`MovementTrace`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MovementTrace {
    /// Fraction.
    pub fraction: f32,
    /// End position.
    pub end: Vec3,
    /// Solidity.
    pub solidity: TraceSolidity,
    /// Contact.
    pub contact: TraceContact,
    /// Contents.
    pub contents: i32,
    /// Surface flags.
    pub surface_flags: i32,
    /// Entity number.
    pub entity_num: i32,
}

impl MovementTrace {
    /// Wrap a world trace result.
    #[must_use]
    pub fn from_world(trace: &TraceResult) -> Self {
        Self {
            fraction: trace.fraction,
            end: trace.end,
            solidity: trace.solidity,
            contact: trace.contact,
            contents: trace.contents,
            surface_flags: trace.surface_flags,
            entity_num: if trace.fraction != 1.0 {
                ENTITYNUM_WORLD
            } else {
                ENTITYNUM_NONE
            },
        }
    }
}

/// Collision world (`CollisionWorld`).
pub trait CollisionWorld {
    /// Trace the world.
    fn trace(&mut self, query: &TraceQuery) -> TraceResult;
    /// Point contents.
    fn point_contents(&mut self, point: Vec3) -> i32;
    /// Transformed model trace.
    fn transformed_trace(&mut self, query: &TraceQuery, model_index: i32, origin: Vec3, angles: Vec3) -> TraceResult;
    /// Transformed point contents.
    fn transformed_point_contents(&mut self, point: Vec3, model_index: i32, origin: Vec3, angles: Vec3) -> i32;
    /// Box-model trace (`createBoxModel(...).transformedTrace`).
    fn box_trace(&mut self, mins: Vec3, maxs: Vec3, query: &TraceQuery, origin: Vec3) -> TraceResult;
}

/// Movement command timing (`commandTiming`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandTiming {
    /// Q3 fixed rounding.
    Q3,
    /// Provider timing.
    Provider,
}

/// Pmove options (`PresentationMovementOptions`).
pub struct PmoveOptions<'a> {
    /// Trace callback.
    pub trace: Box<dyn FnMut(Vec3, Vec3, Bounds, i32, i32) -> MovementTrace + 'a>,
    /// Point-contents callback.
    pub point_contents: Box<dyn FnMut(Vec3, i32) -> i32 + 'a>,
    /// Original server time.
    pub original_server_time: i32,
    /// Trace mask.
    pub trace_mask: i32,
    /// Fixed milliseconds.
    pub fixed_msec: Option<i32>,
    /// No footsteps.
    pub no_footsteps: bool,
    /// Gauntlet hit.
    pub gauntlet_hit: bool,
}

/// Pmove result bounds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PmoveResult {
    /// Player bounds.
    pub bounds: Bounds,
}

// ---------------------------------------------------------------------------
// HUD/contract mirrors (contracts/ui.ts substance used by player-state)
// ---------------------------------------------------------------------------

/// Arsenal ammo warning (`ArsenalAmmoWarning`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArsenalAmmoWarning {
    /// No warning.
    None,
    /// Low ammo.
    Low,
    /// Empty.
    Empty,
}

/// Weapon HUD ammo (`WeaponHudStatus.ammo`, reduced to read fields).
#[derive(Debug, Clone, PartialEq)]
pub enum WeaponHudAmmo {
    /// Unmetered.
    Unmetered,
    /// Finite count.
    Finite {
        /// Count.
        count: i32,
        /// Low flag.
        low: bool,
    },
}

/// Weapon HUD status (`WeaponHudStatus`, reduced to read fields).
#[derive(Debug, Clone, PartialEq)]
pub struct WeaponHudStatus {
    /// Label.
    pub label: String,
    /// Ammo.
    pub ammo: WeaponHudAmmo,
}

/// Weapon HUD reader (`WeaponHudReader`).
pub trait WeaponHudReader {
    /// Read the current HUD status and ammo warning.
    fn read(&mut self) -> (Option<WeaponHudStatus>, ArsenalAmmoWarning);
}

/// Event entity reference: a world entity or the predicted player entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventEntityRef {
    /// World entity number.
    Entity(usize),
    /// Predicted player entity.
    PredictedPlayer,
}

pub(crate) fn event_entity(state: &ClientGameState, entity_ref: EventEntityRef) -> PresentResult<&ClientEntity> {
    match entity_ref {
        EventEntityRef::Entity(number) => {
            let number = i32::try_from(number).map_err(|_| range_msg("Entity number outside int32"))?;
            state.entity_at(number)
        }
        EventEntityRef::PredictedPlayer => Ok(&state.predicted_player_entity),
    }
}

pub(crate) fn event_entity_mut(
    state: &mut ClientGameState,
    entity_ref: EventEntityRef,
) -> PresentResult<&mut ClientEntity> {
    match entity_ref {
        EventEntityRef::Entity(number) => {
            let number = i32::try_from(number).map_err(|_| range_msg("Entity number outside int32"))?;
            state.entity_at_mut(number)
        }
        EventEntityRef::PredictedPlayer => Ok(&mut state.predicted_player_entity),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q3::presentation::client::*;
    use crate::q3::presentation::events::*;
    use crate::q3::presentation::frame::*;
    use crate::q3::presentation::player_state::*;
    use crate::q3::presentation::players::*;
    use crate::q3::presentation::prediction::*;
    use crate::q3::presentation::server_commands::*;
    use qa_core::math::vec3;
    use qa_core::math::vec4;
    use qa_core::math::Axis;
    use qa_core::math::Bounds;
    use qa_core::math::Plane;
    use qa_core::math::Vec3;

    use std::collections::HashMap;

    use crate::q3::presentation::resources::*;

    use crate::q3::presentation::snapshots::*;

    fn test_state() -> ClientGameState {
        ClientGameState::new(Q3Product::BaseQ3, 0, 0).unwrap()
    }

    fn test_static() -> ClientGameStaticState {
        ClientGameStaticState::new(Q3Product::BaseQ3)
    }

    // ---------- resource doubles ----------

    struct TestResourceHost {
        next_model: u32,
        next_skin: u32,
        next_shader: u32,
        cleared: u32,
        rendered: u32,
        entities: Vec<RefEntity>,
        polys: Vec<RefPoly>,
        lights: Vec<DynamicLight>,
        remaps: Vec<(String, String, String)>,
        world: WorldScene,
    }

    impl TestResourceHost {
        fn new() -> Self {
            Self {
                next_model: 1,
                next_skin: 1,
                next_shader: 1,
                cleared: 0,
                rendered: 0,
                entities: Vec::new(),
                polys: Vec::new(),
                lights: Vec::new(),
                remaps: Vec::new(),
                world: WorldScene {
                    model_bounds: Vec::new(),
                },
            }
        }
    }

    impl Q3ResourceHost for TestResourceHost {
        fn zero_picture(&self) -> MaterialPicture {
            SceneShader {
                id: 0,
                name: String::new(),
                material_order: 0,
            }
        }
        fn load_model(&mut self, _path: &str) -> PresentResult<SceneModel> {
            let id = self.next_model;
            self.next_model += 1;
            Ok(SceneModel::Loaded { id })
        }
        fn load_skin(&mut self, _path: &str) -> PresentResult<Option<SceneSkin>> {
            let id = self.next_skin;
            self.next_skin += 1;
            Ok(Some(SceneSkin {
                id,
                surfaces: Vec::new(),
            }))
        }
        fn load_shader(&mut self, path: &str, _mip: bool) -> PresentResult<Option<MaterialPicture>> {
            let id = self.next_shader;
            self.next_shader += 1;
            Ok(Some(SceneShader {
                id,
                name: path.to_string(),
                material_order: id as i32,
            }))
        }
        fn load_world_scene(&mut self, _requested_path: &str) -> PresentResult<WorldScene> {
            Ok(self.world.clone())
        }
        fn remap_shader(&mut self, original: &str, replacement: &str, offset: &str) -> PresentResult<()> {
            self.remaps
                .push((original.to_string(), replacement.to_string(), offset.to_string()));
            Ok(())
        }
        fn clear_scene(&mut self) {
            self.cleared += 1;
        }
        fn add_ref_entity(&mut self, entity: RefEntity) {
            self.entities.push(entity);
        }
        fn add_poly(&mut self, poly: RefPoly) {
            self.polys.push(poly);
        }
        fn add_light(&mut self, light: DynamicLight) {
            self.lights.push(light);
        }
        fn render_scene(&mut self, _refdef: &Refdef) {
            self.rendered += 1;
        }
    }

    struct TestWorld {
        map: ResourceWorldMap,
        pvs: Vec<u8>,
    }

    impl TestWorld {
        fn flat(cluster: i32, visible: bool) -> Self {
            Self {
                map: ResourceWorldMap {
                    entities: String::new(),
                    nodes: Vec::new(),
                    leaves: vec![ResourceBspLeaf { cluster }],
                    planes: Vec::new(),
                },
                pvs: vec![if visible { 1u8 << (cluster as u32 & 7) } else { 0 }],
            }
        }
    }

    impl ResourceWorld for TestWorld {
        fn resource_map(&self) -> &ResourceWorldMap {
            &self.map
        }
        fn cluster_pvs_byte(&self, _cluster: i32, offset: usize) -> u8 {
            *self.pvs.get(offset).unwrap_or(&0)
        }
    }

    // ---------- client-info doubles ----------

    struct TestClientInfoHost {
        files: HashMap<String, Vec<u8>>,
        settings: ClientInfoSettings,
        product: Q3Product,
        cache: HashMap<String, PcmSound>,
        prints: Vec<String>,
        parsed: Vec<String>,
        next_model: u32,
        next_skin: u32,
        next_shader: u32,
        next_sound: u32,
        memory: i64,
    }

    impl TestClientInfoHost {
        fn new() -> Self {
            Self {
                files: HashMap::new(),
                settings: ClientInfoSettings {
                    game_type: GameType::Ffa,
                    max_clients: MAX_CLIENTS as i32,
                    force_model: false,
                    model: String::new(),
                    head_model: String::new(),
                    red_team_name: String::new(),
                    blue_team_name: String::new(),
                    defer_players: false,
                    build_script: false,
                    loading: false,
                },
                product: Q3Product::BaseQ3,
                cache: HashMap::new(),
                prints: Vec::new(),
                parsed: Vec::new(),
                next_model: 1,
                next_skin: 1,
                next_shader: 1,
                next_sound: 1,
                memory: i64::MAX,
            }
        }

        fn with_sarge_files() -> Self {
            let mut host = Self::new();
            for path in [
                "models/players/sarge/lower_default_default.skin",
                "models/players/sarge/upper_default_default.skin",
                "models/players/sarge/default/head_default.skin",
                "models/players/sarge/animation.cfg",
                "models/players/sarge/default/icon_default.skin",
            ] {
                host.files.insert(path.to_string(), b"data".to_vec());
            }
            host
        }
    }

    impl AssetReader for TestClientInfoHost {
        fn read_asset(&mut self, path: &str) -> PresentResult<Vec<u8>> {
            self.files
                .get(path)
                .cloned()
                .ok_or_else(|| state_msg(format!("asset not found: {path}")))
        }
        fn has_asset(&self, path: &str) -> bool {
            self.files.contains_key(path)
        }
        fn list_assets(&self, prefix: Option<&str>) -> Vec<String> {
            let mut names: Vec<String> = self
                .files
                .keys()
                .filter(|name| prefix.is_none_or(|prefix| name.starts_with(prefix)))
                .cloned()
                .collect();
            names.sort();
            names
        }
    }

    impl ClientInfoHost for TestClientInfoHost {
        fn product(&self) -> Q3Product {
            self.product
        }
        fn settings(&self) -> ClientInfoSettings {
            self.settings.clone()
        }
        fn memory_remaining(&self) -> i64 {
            self.memory
        }
        fn register_model(&mut self, _path: &str) -> PresentResult<SceneModel> {
            let id = self.next_model;
            self.next_model += 1;
            Ok(SceneModel::Loaded { id })
        }
        fn register_skin(&mut self, _path: &str) -> PresentResult<Option<SceneSkin>> {
            let id = self.next_skin;
            self.next_skin += 1;
            Ok(Some(SceneSkin {
                id,
                surfaces: Vec::new(),
            }))
        }
        fn register_shader_no_mip(&mut self, name: &str) -> PresentResult<Option<SceneShader>> {
            let id = self.next_shader;
            self.next_shader += 1;
            Ok(Some(SceneShader {
                id,
                name: name.to_string(),
                material_order: id as i32,
            }))
        }
        fn register_sound(&mut self, _name: &str) -> PresentResult<Option<PcmSound>> {
            let id = self.next_sound;
            self.next_sound += 1;
            Ok(Some(PcmSound::new(id)))
        }
        fn sound(&self, name: &str) -> Option<PcmSound> {
            self.cache.get(name).copied()
        }
        fn parse_animation_config(&mut self, ci: &mut ClientInfo, _text: &str, path: &str) -> PresentResult<bool> {
            self.parsed.push(path.to_string());
            let dummy = PlayerAnimation {
                name: "test".to_string(),
                first_frame: 0,
                num_frames: 1,
                loop_frames: 0,
                frame_lerp: 100,
                initial_lerp: 100,
                reversed: false,
                flipflop: false,
            };
            ci.animations = vec![Some(dummy); CLIENT_ANIMATION_COUNT];
            Ok(true)
        }
        fn model_has_tag(&mut self, _model: &SceneModel, tag: &str) -> bool {
            tag == "tag_flag"
        }
        fn print(&mut self, message: &str) {
            self.prints.push(message.to_string());
        }
    }

    // ---------- player doubles ----------

    fn player_media_fixture() -> PlayerMedia {
        PlayerMedia {
            connection_shader: None,
            balloon_shader: None,
            medal_impressive: None,
            medal_excellent: None,
            medal_gauntlet: None,
            medal_defend: None,
            medal_assist: None,
            medal_capture: None,
            friend_shader: None,
            shadow_mark_shader: None,
            wake_mark_shader: None,
            invis_shader: None,
            quad_shader: None,
            red_quad_shader: None,
            regen_shader: None,
            battle_suit_shader: None,
            haste_puff_shader: None,
            flight_sound: None,
            red_flag_model: default_model(),
            blue_flag_model: default_model(),
            neutral_flag_model: default_model(),
            flag_pole_model: default_model(),
            flag_flap_model: default_model(),
            red_flag_flap_skin: None,
            blue_flag_flap_skin: None,
            neutral_flag_flap_skin: None,
        }
    }

    fn player_settings_fixture() -> PlayerPresentationSettings {
        PlayerPresentationSettings {
            game_type: GameType::Ffa,
            camera_mode: false,
            no_player_animations: true,
            animation_speed: 1.0,
            swing_speed: 1.0,
            draw_friend: false,
            shadows: 0,
            enable_breath: false,
            enable_dust: false,
            debug_position: false,
            debug_animation: false,
        }
    }

    struct TestPlayerHost {
        product: Q3Product,
        media: PlayerMedia,
        settings: PlayerPresentationSettings,
        entities: Vec<RefEntity>,
        polys: Vec<RefPoly>,
        lights: Vec<DynamicLight>,
        puffs: Vec<(SmokePuffOptions, bool)>,
        loops: Vec<(i32, Vec3, Option<PcmSound>)>,
        weapons_added: u32,
        prints: Vec<String>,
        trace_result: TraceResult,
        contents: i32,
        random: i32,
        light: LightingSample,
        pose: PlayerPoseAxes,
    }

    impl TestPlayerHost {
        fn new() -> Self {
            let axis = [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)];
            Self {
                product: Q3Product::BaseQ3,
                media: player_media_fixture(),
                settings: player_settings_fixture(),
                entities: Vec::new(),
                polys: Vec::new(),
                lights: Vec::new(),
                puffs: Vec::new(),
                loops: Vec::new(),
                weapons_added: 0,
                prints: Vec::new(),
                trace_result: TraceResult {
                    fraction: 1.0,
                    end: vec3(0.0, 0.0, 0.0),
                    solidity: TraceSolidity::Clear,
                    contact: TraceContact::None,
                    contents: 0,
                    surface_flags: 0,
                },
                contents: 0,
                random: 0,
                light: LightingSample {
                    ambient_light: vec3(10.0, 10.0, 10.0),
                    directed_light: vec3(20.0, 20.0, 20.0),
                    light_dir: vec3(0.0, 0.0, 1.0),
                },
                pose: PlayerPoseAxes {
                    legs: axis,
                    torso: axis,
                    head: axis,
                },
            }
        }
    }

    impl PlayerPresentationHost for TestPlayerHost {
        fn product(&self) -> Q3Product {
            self.product
        }
        fn media(&self) -> PlayerMedia {
            self.media.clone()
        }
        fn mission_media(&self) -> Option<MissionPlayerMedia> {
            None
        }
        fn settings(&self) -> PlayerPresentationSettings {
            self.settings.clone()
        }
        fn trace_world(&mut self, _start: Vec3, _end: Vec3, _mins: Vec3, _maxs: Vec3, _mask: i32) -> TraceResult {
            self.trace_result
        }
        fn trace_skip(
            &mut self,
            _start: Vec3,
            _end: Vec3,
            _mins: Vec3,
            _maxs: Vec3,
            _skip_number: i32,
            _mask: i32,
        ) -> TraceResult {
            self.trace_result
        }
        fn point_contents(&mut self, _point: Vec3) -> i32 {
            self.contents
        }
        fn add_entity(&mut self, entity: RefEntity) {
            self.entities.push(entity);
        }
        fn add_light(&mut self, light: DynamicLight) {
            self.lights.push(light);
        }
        fn add_poly(&mut self, poly: RefPoly) {
            self.polys.push(poly);
        }
        fn light_for_point(&mut self, _point: Vec3) -> LightingSample {
            self.light
        }
        fn impact_mark(&mut self, request: &ImpactMarkRequest) -> Vec<RefPoly> {
            vec![RefPoly {
                shader: request.shader.clone(),
                vertices: Vec::new(),
            }]
        }
        fn smoke_puff(&mut self, options: &SmokePuffOptions, scale_fade: bool) {
            self.puffs.push((options.clone(), scale_fade));
        }
        fn add_looping_sound(&mut self, entity_num: i32, origin: Vec3, _velocity: Vec3, sound: Option<PcmSound>) {
            self.loops.push((entity_num, origin, sound));
        }
        fn add_player_weapon(
            &mut self,
            _parent: &RefModelEntity,
            _ps: Option<&PlayerState>,
            _entity: &ClientEntity,
            _team: Team,
        ) {
            self.weapons_added += 1;
        }
        fn random_int(&mut self) -> i32 {
            self.random
        }
        fn calculate_pose(
            &mut self,
            _player: &ClientPlayerEntity,
            _current: &EntityState,
            _ci: &ClientInfo,
            _lerp_angles: Vec3,
            _time_ms: i32,
            _frame_time_ms: i32,
            _swing_speed: f32,
        ) -> PlayerPoseAxes {
            self.pose
        }
        fn clear_lerp_frame(
            &mut self,
            _ci: &ClientInfo,
            frame: &mut LerpFrame,
            animation: i32,
            time_ms: i32,
            _verbose: bool,
        ) {
            frame.animation_number = animation;
            frame.frame_time = time_ms;
            frame.old_frame_time = time_ms;
            frame.animation_time = time_ms;
        }
        fn run_lerp_frame(
            &mut self,
            _ci: &ClientInfo,
            frame: &mut LerpFrame,
            new_animation: i32,
            _speed_scale: f32,
            time_ms: i32,
            _frozen: bool,
            _verbose: bool,
        ) {
            frame.animation_number = new_animation;
            frame.frame = new_animation;
            frame.old_frame = new_animation;
            frame.frame_time = time_ms;
        }
        fn swing_angles(
            &mut self,
            destination: f32,
            _swing_tolerance: f32,
            _clamp_tolerance: f32,
            _speed: f32,
            _frame_time_ms: i32,
            _angle: f32,
            _swinging: bool,
        ) -> (f32, bool) {
            (destination, false)
        }
        fn position_on_tag(
            &mut self,
            entity: &mut RefModelEntity,
            parent: &RefModelEntity,
            _parent_model: &SceneModel,
            _tag: &str,
        ) -> PresentResult<()> {
            entity.origin = parent.origin;
            entity.axis = parent.axis;
            entity.back_lerp = parent.back_lerp;
            Ok(())
        }
        fn position_rotated_on_tag(
            &mut self,
            entity: &mut RefModelEntity,
            parent: &RefModelEntity,
            _parent_model: &SceneModel,
            _tag: &str,
        ) -> PresentResult<()> {
            entity.origin = parent.origin;
            Ok(())
        }
        fn print(&mut self, message: &str) {
            self.prints.push(message.to_string());
        }
    }

    // ---------- frame doubles ----------

    struct TestFrameHost {
        snap: Option<Snapshot>,
        cvars: HashMap<String, CvarSnapshot>,
        rage_pro: bool,
        weapons: Vec<i32>,
        pre_view_weapon: bool,
        view_weapons: u32,
        test_model: Option<RefEntity>,
        damage_blob: Option<RefEntity>,
        marks: Vec<RefPoly>,
        particles: Vec<RefPoly>,
        scene_entities: Vec<RefEntity>,
        scene_polys: Vec<RefPoly>,
        clears: u32,
        rendered: u32,
        loading_frames: u32,
        draws_2d: u32,
        scoreboards: u32,
        lag: u32,
        prints: Vec<String>,
        listener: Option<(i32, Vec3)>,
        user_command: Option<(i32, f32)>,
        timescale_cvar: Option<f32>,
        timescale: Option<f32>,
        looping: Vec<bool>,
    }

    impl TestFrameHost {
        fn new() -> Self {
            Self {
                snap: None,
                cvars: HashMap::new(),
                rage_pro: false,
                weapons: Vec::new(),
                pre_view_weapon: true,
                view_weapons: 0,
                test_model: None,
                damage_blob: None,
                marks: Vec::new(),
                particles: Vec::new(),
                scene_entities: Vec::new(),
                scene_polys: Vec::new(),
                clears: 0,
                rendered: 0,
                loading_frames: 0,
                draws_2d: 0,
                scoreboards: 0,
                lag: 0,
                prints: Vec::new(),
                listener: None,
                user_command: None,
                timescale_cvar: None,
                timescale: None,
                looping: Vec::new(),
            }
        }

        fn with_cvar(mut self, name: FrameCvar, numeric_value: f32, integer_value: i32) -> Self {
            self.cvars.insert(
                name.as_str().to_string(),
                CvarSnapshot {
                    name: name.as_str().to_string(),
                    value: numeric_value.to_string(),
                    numeric_value,
                    integer_value,
                },
            );
            self
        }
    }

    impl Q3PresentationSceneHost for TestFrameHost {
        fn update_cvars(&mut self) -> PresentResult<()> {
            Ok(())
        }
        fn register_weapon(&mut self, weapon: i32) -> PresentResult<()> {
            self.weapons.push(weapon);
            Ok(())
        }
        fn process_snapshots(
            &mut self,
            state: &mut ClientGameState,
            _static_state: &mut ClientGameStaticState,
        ) -> PresentResult<()> {
            state.snap = self.snap.clone();
            Ok(())
        }
        fn packet_options(&self, static_state: &ClientGameStaticState) -> PacketEntityOptions {
            PacketEntityOptions {
                game_type: static_state.game_type,
                smooth_clients: false,
                simple_items: false,
                obelisk_respawn_delay: 0,
            }
        }
        fn add_packet_entities(
            &mut self,
            _state: &mut ClientGameState,
            _static_state: &ClientGameStaticState,
            _options: &PacketEntityOptions,
        ) {
        }
        fn poll_impact_marks(&mut self, _state: &ClientGameState) -> Vec<RefPoly> {
            std::mem::take(&mut self.marks)
        }
        fn poll_particles(&mut self, _state: &ClientGameState) -> Vec<RefPoly> {
            std::mem::take(&mut self.particles)
        }
        fn add_local_entities(&mut self, _state: &ClientGameState) {}
        fn play_buffered_sounds(&mut self, _state: &mut ClientGameState) {}
        fn play_buffered_voice_chats(
            &mut self,
            _state: &mut ClientGameState,
            _static_state: &mut ClientGameStaticState,
        ) -> PresentResult<()> {
            Ok(())
        }
        fn add_poly(&mut self, poly: RefPoly) {
            self.scene_polys.push(poly);
        }
        fn add_ref_entity(&mut self, entity: RefEntity) {
            self.scene_entities.push(entity);
        }
        fn clear_scene(&mut self) {
            self.clears += 1;
            self.scene_entities.clear();
            self.scene_polys.clear();
        }
        fn render_scene(&mut self, _state: &ClientGameState) {
            self.rendered += 1;
        }
        fn enter_frame(&mut self, _frame: &Q3PresentationFrame) {}
        fn clear_looping_sounds(&mut self, kill_all: bool) {
            self.looping.push(kill_all);
        }
    }

    impl Q3PresentationFrameHost for TestFrameHost {
        fn hardware_is_rage_pro(&self) -> bool {
            self.rage_pro
        }
        fn predict_player_state(
            &mut self,
            state: &mut ClientGameState,
            _static_state: &mut ClientGameStaticState,
        ) -> PresentResult<()> {
            if let Some(snapshot) = &state.snap {
                state.predicted_player_state = snapshot.player_state.clone();
            }
            Ok(())
        }
        fn calculate_view_values(&mut self, _state: &mut ClientGameState) {}
        fn damage_blend_blob(&mut self, _state: &ClientGameState, _rage_pro: bool) -> Option<RefEntity> {
            self.damage_blob.clone()
        }
        fn pre_present_view_weapon(&mut self, _state: &ClientGameState) -> bool {
            self.pre_view_weapon
        }
        fn add_view_weapon(&mut self, _state: &mut ClientGameState) {
            self.view_weapons += 1;
        }
        fn post_present_view_weapon(&mut self, _state: &mut ClientGameState) {}
        fn add_test_model(&mut self, _state: &ClientGameState) -> PresentResult<Option<RefEntity>> {
            Ok(self.test_model.clone())
        }
        fn finish_refdef(&mut self, state: &mut ClientGameState) {
            state.refdef.time = state.time;
        }
        fn powerup_timer_sounds(&mut self, _state: &ClientGameState) {}
        fn set_listener(&mut self, client: i32, origin: Vec3, _axis: Axis) {
            self.listener = Some((client, origin));
        }
        fn add_lagometer_frame_info(&mut self, _state: &ClientGameState) {
            self.lag += 1;
        }
        fn read_frame_cvar(&self, name: FrameCvar) -> CvarSnapshot {
            self.cvars.get(name.as_str()).cloned().unwrap_or(CvarSnapshot {
                name: name.as_str().to_string(),
                value: "0".to_string(),
                numeric_value: 0.0,
                integer_value: 0,
            })
        }
        fn set_timescale_cvar(&mut self, value: f32) {
            self.timescale_cvar = Some(value);
        }
        fn set_timescale(&mut self, value: f32) {
            self.timescale = Some(value);
        }
        fn set_user_command_value(&mut self, weapon: i32, sensitivity: f32) {
            self.user_command = Some((weapon, sensitivity));
        }
        fn loading_frame(
            &mut self,
            _state: &mut ClientGameState,
            _static_state: &mut ClientGameStaticState,
        ) -> PresentResult<()> {
            self.loading_frames += 1;
            Ok(())
        }
        fn draw_tourney_scoreboard(&mut self, _state: &mut ClientGameState, _static_state: &mut ClientGameStaticState) {
            self.scoreboards += 1;
        }
        fn tile_clear(&mut self, _state: &ClientGameState) {}
        fn draw_2d(
            &mut self,
            _state: &mut ClientGameState,
            _static_state: &mut ClientGameStaticState,
        ) -> PresentResult<()> {
            self.draws_2d += 1;
            Ok(())
        }
        fn print(&mut self, text: &str) {
            self.prints.push(text.to_string());
        }
    }

    // ---------- session / server doubles ----------

    struct TestSession {
        product: Q3Product,
        client_number: i32,
        sequence: i32,
        last_command: i32,
        mode: PresentationSessionMode,
        game_state: Vec<String>,
        prints: Vec<String>,
    }

    impl TestSession {
        fn new(game_state: Vec<String>) -> Self {
            Self {
                product: Q3Product::BaseQ3,
                client_number: 0,
                sequence: 5,
                last_command: 4,
                mode: PresentationSessionMode::Live,
                game_state,
                prints: Vec::new(),
            }
        }
    }

    impl Q3SceneSession for TestSession {
        fn product(&self) -> Q3Product {
            self.product
        }
        fn client_number(&self) -> i32 {
            self.client_number
        }
        fn server_message_sequence(&self) -> i32 {
            self.sequence
        }
        fn last_executed_server_command(&self) -> i32 {
            self.last_command
        }
        fn session_mode(&self) -> PresentationSessionMode {
            self.mode
        }
        fn get_game_state(&mut self) -> Vec<String> {
            self.game_state.clone()
        }
        fn get_server_command(&mut self, _sequence: i32) -> PresentResult<Option<Vec<String>>> {
            Ok(None)
        }
        fn snapshot_ping(&mut self, _number: i32) -> Option<i32> {
            Some(50)
        }
        fn assert_current(&self) {}
        fn print(&mut self, text: &str) {
            self.prints.push(text.to_string());
        }
    }

    impl Q3PresentationSession for TestSession {
        fn add_reliable_command(&mut self, _text: &str) {}
        fn append_console_command(&mut self, _text: &str) {}
        fn register_cgame_command(&mut self, _name: &str) {}
        fn set_user_command_value(&mut self, _weapon: i32, _sensitivity: f32) {}
    }

    struct TestServerHost {
        config: Vec<String>,
        cvars_set: Vec<(String, String)>,
        background: Option<(String, String)>,
        remaps: Vec<(String, String, String)>,
        looping: Vec<bool>,
        refreshed: bool,
    }

    impl TestServerHost {
        fn new() -> Self {
            let mut config = vec![String::new(); 32];
            config[2] = "intro loop".to_string();
            config[5] = "0".to_string();
            config[6] = "3".to_string();
            config[7] = "4".to_string();
            config[21] = "222".to_string();
            Self {
                config,
                cvars_set: Vec::new(),
                background: None,
                remaps: Vec::new(),
                looping: Vec::new(),
                refreshed: false,
            }
        }
    }

    impl ClientServerCommandHost for TestServerHost {
        fn client_info<'a>(
            &self,
            static_state: &'a ClientGameStaticState,
            index: usize,
        ) -> PresentResult<&'a ClientInfo> {
            static_state
                .client_info
                .get(index)
                .ok_or_else(|| range_msg(format!("Source array index {index} outside 64")))
        }
        fn new_client_info(
            &mut self,
            _state: &mut ClientGameState,
            static_state: &mut ClientGameStaticState,
            index: usize,
            configstring: &str,
        ) -> PresentResult<()> {
            let slot = static_state
                .client_info
                .get_mut(index)
                .ok_or_else(|| range_msg(format!("Source array index {index} outside 64")))?;
            slot.info_valid = true;
            slot.name = info_value_for_key(configstring, "n", 8192).unwrap_or_default();
            Ok(())
        }
        fn load_deferred_players(
            &mut self,
            _state: &mut ClientGameState,
            _static_state: &mut ClientGameStaticState,
        ) -> PresentResult<()> {
            Ok(())
        }
        fn reset_clients(&mut self, static_state: &mut ClientGameStaticState) {
            for slot in static_state.client_info.iter_mut() {
                *slot = ClientInfo::new();
            }
        }
        fn register_model(&mut self, path: &str) -> PresentResult<SceneModel> {
            Ok(SceneModel::Loaded {
                id: path.len() as u32 + 1,
            })
        }
        fn assets_has(&self, _path: &str) -> bool {
            false
        }
        fn assets_read(&mut self, path: &str) -> PresentResult<Vec<u8>> {
            Err(state_msg(format!("no test assets: {path}")))
        }
        fn assets_read_sync(&self, path: &str) -> PresentResult<Vec<u8>> {
            Err(state_msg(format!("no test assets: {path}")))
        }
        fn get_server_command(&mut self, _sequence: i32) -> PresentResult<Option<Vec<String>>> {
            Ok(None)
        }
        fn refresh_game_state(&mut self) {
            self.refreshed = true;
        }
        fn config_string(&self, index: i32) -> PresentResult<String> {
            if index < 0 {
                return Err(range_msg(format!("CG_ConfigString: bad index: {index}")));
            }
            self.config
                .get(index as usize)
                .cloned()
                .ok_or_else(|| range_msg(format!("CG_ConfigString: bad index: {index}")))
        }
        fn read_vm_cvar(&self, name: ClientServerCommandCvar) -> CvarSnapshot {
            CvarSnapshot {
                name: format!("{name:?}"),
                value: "0".to_string(),
                numeric_value: 0.0,
                integer_value: 0,
            }
        }
        fn set_cvar(&mut self, name: &str, value: &str) {
            self.cvars_set.push((name.to_string(), value.to_string()));
        }
        fn print(&mut self, _text: &str) {}
        fn center_print(&mut self, _text: &str, _y: i32, _char_width: i32) {}
        fn send_console_command(&mut self, _text: &str) {}
        fn sound(&self, _name: ClientServerCommandSound) -> Option<PcmSound> {
            None
        }
        fn register_sound(&mut self, _path: &str, _compressed: bool) -> PresentResult<Option<PcmSound>> {
            Ok(None)
        }
        fn start_local_sound(&mut self, _sound: Option<PcmSound>, _channel: i32) {}
        fn start_background_track(&mut self, intro: &str, loop_track: &str) -> PresentResult<()> {
            self.background = Some((intro.to_string(), loop_track.to_string()));
            Ok(())
        }
        fn remap_shader(&mut self, original: &str, replacement: &str, time_offset: &str) -> PresentResult<()> {
            self.remaps
                .push((original.to_string(), replacement.to_string(), time_offset.to_string()));
            Ok(())
        }
        fn clear_local_entities(&mut self) {}
        fn clear_marks(&mut self) {}
        fn clear_particles(&mut self) -> PresentResult<()> {
            Ok(())
        }
        fn clear_looping_sounds(&mut self, kill_all: bool) {
            self.looping.push(kill_all);
        }
        fn set_score_selection(&mut self) {}
        fn show_response_head(&mut self) -> PresentResult<()> {
            Ok(())
        }
        fn memory_remaining(&self) -> i64 {
            i64::MAX
        }
        fn random_float(&mut self) -> f32 {
            0.5
        }
    }

    // ---------- filler doubles for assembly ----------

    struct TestSnapHost;

    impl SnapshotHost for TestSnapHost {
        fn source_current(&mut self) -> SnapshotCurrent {
            SnapshotCurrent {
                number: 0,
                server_time: 0,
            }
        }
        fn source_read(&mut self, _number: i32) -> PresentResult<Option<Snapshot>> {
            Ok(None)
        }
        fn demo_playback(&self) -> bool {
            false
        }
        fn no_predict(&self) -> bool {
            false
        }
        fn synchronous_clients(&self) -> bool {
            false
        }
        fn execute_server_commands(
            &mut self,
            _state: &mut ClientGameState,
            _static_state: &mut ClientGameStaticState,
            _sequence: i32,
        ) -> PresentResult<()> {
            Ok(())
        }
        fn respawn(&mut self, _state: &mut ClientGameState) -> PresentResult<()> {
            Ok(())
        }
        fn reset_player_entity(
            &mut self,
            _state: &mut ClientGameState,
            _static_state: &mut ClientGameStaticState,
            _entity_number: usize,
        ) {
        }
        fn check_events(
            &mut self,
            _state: &mut ClientGameState,
            _static_state: &mut ClientGameStaticState,
            _entity_number: usize,
        ) -> PresentResult<()> {
            Ok(())
        }
        fn transition_player_state(
            &mut self,
            _state: &mut ClientGameState,
            _static_state: &mut ClientGameStaticState,
            _current: &PlayerState,
            _previous: &mut PlayerState,
        ) -> PresentResult<()> {
            Ok(())
        }
        fn lagometer_snapshot(&mut self, _snapshot: Option<&Snapshot>) {}
        fn warn(&mut self, _message: &str) {}
    }

    struct TestCollision;

    impl CollisionWorld for TestCollision {
        fn trace(&mut self, query: &TraceQuery) -> TraceResult {
            TraceResult {
                fraction: 1.0,
                end: query.end,
                solidity: TraceSolidity::Clear,
                contact: TraceContact::None,
                contents: 0,
                surface_flags: 0,
            }
        }
        fn point_contents(&mut self, _point: Vec3) -> i32 {
            0
        }
        fn transformed_trace(
            &mut self,
            query: &TraceQuery,
            _model_index: i32,
            _origin: Vec3,
            _angles: Vec3,
        ) -> TraceResult {
            self.trace(query)
        }
        fn transformed_point_contents(&mut self, _point: Vec3, _model_index: i32, _origin: Vec3, _angles: Vec3) -> i32 {
            0
        }
        fn box_trace(&mut self, _mins: Vec3, _maxs: Vec3, query: &TraceQuery, _origin: Vec3) -> TraceResult {
            self.trace(query)
        }
    }

    struct TestPredHost;

    impl PredictionHost for TestPredHost {
        fn command_timing(&self) -> CommandTiming {
            CommandTiming::Q3
        }
        fn move_player(
            &mut self,
            _ps: &mut PlayerState,
            _command: &UserCommand,
            _options: &mut PmoveOptions,
        ) -> PmoveResult {
            PmoveResult {
                bounds: Bounds {
                    min: vec3(0.0, 0.0, 0.0),
                    max: vec3(0.0, 0.0, 0.0),
                },
            }
        }
        fn update_view_angles(&mut self, _ps: &mut PlayerState, _command: &UserCommand) {}
        fn current_command_number(&mut self) -> i32 {
            0
        }
        fn read_command(&mut self, _number: i32) -> PresentResult<Option<UserCommand>> {
            Ok(None)
        }
        fn settings(&self) -> PredictionSettings {
            PredictionSettings {
                game_type: GameType::Ffa,
                dm_flags: 0,
                demo_playback: false,
                no_predict: false,
                synchronous_clients: false,
                predict_items: false,
                pmove_fixed: false,
                pmove_msec: 8,
                error_decay_integer: 0,
                error_decay_value: 0.0,
                show_miss: 0,
            }
        }
        fn set_pmove_msec(&mut self, _value: i32) {}
        fn transition_player_state(
            &mut self,
            _state: &mut ClientGameState,
            _static_state: &mut ClientGameStaticState,
            _current: &PlayerState,
            _previous: &mut PlayerState,
        ) -> PresentResult<()> {
            Ok(())
        }
        fn event_debug(&self) -> Option<PredictableEventDebug> {
            None
        }
        fn pre_predict_item(&mut self, _state: &mut ClientGameState, _entity_number: usize) -> bool {
            false
        }
        fn post_predict_item(&mut self, _state: &mut ClientGameState, _entity_number: usize) {}
        fn warn(&mut self, _message: &str) {}
    }

    struct TestPsHost {
        sounds: PlayerStateSounds,
        medals: RewardMedals,
    }

    impl TestPsHost {
        fn new() -> Self {
            Self {
                sounds: PlayerStateSounds {
                    no_ammo_sound: None,
                    hit_sound: None,
                    hit_team_sound: None,
                    capture_award_sound: None,
                    impressive_sound: None,
                    excellent_sound: None,
                    humiliation_sound: None,
                    defend_sound: None,
                    assist_sound: None,
                    denied_sound: None,
                    holy_shit_sound: None,
                    you_have_flag_sound: None,
                    taken_lead_sound: None,
                    tied_lead_sound: None,
                    lost_lead_sound: None,
                    sudden_death_sound: None,
                    one_minute_sound: None,
                    five_minute_sound: None,
                    one_frag_sound: None,
                    two_frag_sound: None,
                    three_frag_sound: None,
                },
                medals: RewardMedals {
                    medal_capture: None,
                    medal_impressive: None,
                    medal_excellent: None,
                    medal_gauntlet: None,
                    medal_defend: None,
                    medal_assist: None,
                },
            }
        }
    }

    impl PlayerStateHost for TestPsHost {
        fn product(&self) -> Q3Product {
            Q3Product::BaseQ3
        }
        fn show_miss(&self) -> bool {
            false
        }
        fn weapon_hud(&mut self) -> Option<(Option<WeaponHudStatus>, ArsenalAmmoWarning)> {
            None
        }
        fn sounds(&self) -> &PlayerStateSounds {
            &self.sounds
        }
        fn medals(&self) -> &RewardMedals {
            &self.medals
        }
        fn mission_sounds(&self) -> Option<&MissionPlayerStateSounds> {
            None
        }
        fn entity_event(
            &mut self,
            _state: &mut ClientGameState,
            _static_state: &mut ClientGameStaticState,
            _entity_ref: EventEntityRef,
            _position: Vec3,
        ) -> PresentResult<()> {
            Ok(())
        }
        fn pain_event(
            &mut self,
            _state: &mut ClientGameState,
            _static_state: &mut ClientGameStaticState,
            _entity_ref: EventEntityRef,
            _health: i32,
        ) -> PresentResult<()> {
            Ok(())
        }
        fn start_local_sound(&mut self, _sound: Option<PcmSound>, _channel: i32) {}
        fn add_buffered_sound(&mut self, _sound: Option<PcmSound>) {}
        fn print(&mut self, _message: &str) {}
    }

    struct TestEventHost {
        media: ClientEventMedia,
    }

    impl TestEventHost {
        fn new() -> Self {
            Self {
                media: ClientEventMedia {
                    sounds: ClientEventSounds {
                        use_nothing_sound: None,
                        medkit_sound: None,
                        land_sound: None,
                        jump_pad_sound: None,
                        watr_in_sound: None,
                        watr_out_sound: None,
                        watr_un_sound: None,
                        n_health_sound: None,
                        select_sound: None,
                        tele_in_sound: None,
                        tele_out_sound: None,
                        respawn_sound: None,
                        hgrenb1a_sound: None,
                        hgrenb2a_sound: None,
                        capture_your_team_sound: None,
                        capture_opponent_sound: None,
                        return_your_team_sound: None,
                        return_opponent_sound: None,
                        blue_flag_returned_sound: None,
                        red_flag_returned_sound: None,
                        enemy_took_your_flag_sound: None,
                        your_team_took_enemy_flag_sound: None,
                        your_base_is_under_attack_sound: None,
                        red_scored_sound: None,
                        blue_scored_sound: None,
                        red_leads_sound: None,
                        blue_leads_sound: None,
                        teams_tied_sound: None,
                        quad_sound: None,
                        protect_sound: None,
                        regen_sound: None,
                        gib_sound: None,
                    },
                    footsteps: FootstepBank {
                        normal: [None; 4],
                        boot: [None; 4],
                        flesh: [None; 4],
                        mech: [None; 4],
                        energy: [None; 4],
                        metal: [None; 4],
                        splash: [None; 4],
                    },
                    game_sounds: Vec::new(),
                    smoke_puff_shader: None,
                },
            }
        }
    }

    impl ClientEventHost for TestEventHost {
        fn product(&self) -> Q3Product {
            Q3Product::BaseQ3
        }
        fn media(&self) -> &ClientEventMedia {
            &self.media
        }
        fn options(&self) -> ClientEventOptions {
            ClientEventOptions {
                game_type: GameType::Ffa,
                debug_events: false,
                footsteps: false,
                autoswitch: false,
                demo_playback: false,
                no_predict: false,
                synchronous_clients: false,
                single_player_active: false,
                camera_orbit: false,
            }
        }
        fn mission_sounds(&self) -> Option<&MissionEventSounds> {
            None
        }
        fn rand_int(&mut self) -> i32 {
            0
        }
        fn pre_present_event(
            &mut self,
            _state: &mut ClientGameState,
            _entity_ref: EventEntityRef,
            _position: Vec3,
        ) -> bool {
            false
        }
        fn post_present_event(&mut self, _state: &mut ClientGameState, _entity_ref: EventEntityRef, _position: Vec3) {}
        fn set_entity_sound_position(&mut self, _state: &ClientGameState, _entity_number: usize) {}
        fn beam(&mut self, _state: &mut ClientGameState, _entity_ref: EventEntityRef) {}
        fn out_of_ammo_change(&mut self, _state: &mut ClientGameState) {}
        fn fire_weapon(&mut self, _state: &mut ClientGameState, _entity_ref: EventEntityRef) {}
        fn missile_hit_player(
            &mut self,
            _state: &mut ClientGameState,
            _weapon: i32,
            _position: Vec3,
            _direction: Vec3,
            _other_entity_num: i32,
        ) {
        }
        fn missile_hit_wall(
            &mut self,
            _state: &mut ClientGameState,
            _weapon: i32,
            _client_num: i32,
            _position: Vec3,
            _direction: Vec3,
            _impact: ImpactSound,
        ) {
        }
        fn rail_trail(&mut self, _state: &mut ClientGameState, _client_num: i32, _origin2: Vec3, _base: Vec3) {}
        fn bullet(&mut self, _state: &mut ClientGameState, _base: Vec3, _other_entity_num: i32, _target: BulletTarget) {
        }
        fn shotgun_fire(&mut self, _state: &mut ClientGameState, _entity_ref: EventEntityRef) {}
        fn smoke_puff(
            &mut self,
            _state: &mut ClientGameState,
            _desc: &SmokePuffDesc,
            _le_type_override: Option<&str>,
        ) -> SpawnedPuff {
            SpawnedPuff { le_type: None }
        }
        fn spawn_effect(&mut self, _state: &mut ClientGameState, _position: Vec3) {}
        fn gib_player(&mut self, _state: &mut ClientGameState, _position: Vec3) {}
        fn score_plum(&mut self, _state: &mut ClientGameState, _other_entity_num: i32, _position: Vec3, _time: i32) {}
        fn kamikaze_effect(&mut self, _state: &mut ClientGameState, _position: Vec3) {}
        fn obelisk_explode(&mut self, _state: &mut ClientGameState, _position: Vec3) {}
        fn obelisk_pain(&mut self, _state: &mut ClientGameState, _position: Vec3) {}
        fn invulnerability_impact(&mut self, _state: &mut ClientGameState, _position: Vec3, _angles: Vec3) {}
        fn invulnerability_juiced(&mut self, _state: &mut ClientGameState, _position: Vec3) {}
        fn lightning_bolt_beam(&mut self, _state: &mut ClientGameState, _origin2: Vec3, _base: Vec3) {}
        fn client_info_brief(
            &self,
            static_state: &ClientGameStaticState,
            number: i32,
        ) -> PresentResult<EventClientInfo> {
            if number < 0 {
                return Err(range_msg(format!("Player index {number} outside clients")));
            }
            let slot = static_state
                .client_info
                .get(number as usize)
                .ok_or_else(|| range_msg(format!("Player index {number} outside clients")))?;
            Ok(EventClientInfo {
                gender: slot.gender,
                footsteps: slot.footsteps,
                team: slot.team,
            })
        }
        fn set_medkit_usage_time(
            &mut self,
            static_state: &mut ClientGameStaticState,
            client_num: i32,
            time: i32,
        ) -> PresentResult<()> {
            if client_num < 0 {
                return Err(range_msg(format!("Player index {client_num} outside clients")));
            }
            let slot = static_state
                .client_info
                .get_mut(client_num as usize)
                .ok_or_else(|| range_msg(format!("Player index {client_num} outside clients")))?;
            slot.medkit_usage_time = time;
            Ok(())
        }
        fn player_name(&self, _number: i32) -> Option<String> {
            None
        }
        fn sound_config_string(&self, _index: i32) -> String {
            String::new()
        }
        fn custom_sound(&mut self, _client_num: i32, _name: &str) -> Option<PcmSound> {
            None
        }
        fn register_sound(&mut self, _path: Option<&str>, _compressed: bool) -> Option<PcmSound> {
            None
        }
        fn start_sound(&mut self, _origin: Option<Vec3>, _entity_num: i32, _channel: i32, _sound: Option<PcmSound>) {}
        fn start_local_sound(&mut self, _sound: Option<PcmSound>, _channel: i32) {}
        fn stop_looping_sound(&mut self, _entity_num: i32) {}
        fn add_buffered_sound(&mut self, _sound: Option<PcmSound>) {}
        fn print(&mut self, _message: &str) {}
        fn center_print(&mut self, _message: &str, _y: i32, _char_width: i32) {}
        fn voice_chat_local(
            &mut self,
            _state: &mut ClientGameState,
            _static_state: &mut ClientGameStaticState,
            _mode: i32,
            _voice_only: bool,
            _client_num: i32,
            _color: i32,
            _command: &str,
        ) -> PresentResult<()> {
            Ok(())
        }
    }

    // ---------- resources tests ----------

    #[test]
    fn resource_models_cache_and_roundtrip_handles() {
        let mut resources: Q3RendererResources<TestResourceHost, TestWorld> =
            Q3RendererResources::new(TestResourceHost::new());
        assert!(resources.register_model(None).unwrap().is_default());
        assert!(resources.register_model(Some("")).unwrap().is_default());
        let first = resources.register_model(Some("models/a.md3")).unwrap();
        let second = resources.register_model(Some("models/a.md3")).unwrap();
        assert_eq!(first, second);
        assert_eq!(resources.model_handle(&first).unwrap(), 1);
        assert_eq!(resources.model_for_handle(1).unwrap(), first);
        assert!(resources.model_for_handle(0).unwrap().is_default());
        assert!(resources.model_for_handle(99).is_err());
        assert!(resources.model_for_handle(-1).is_err());
        assert!(resources.model_handle(&SceneModel::Loaded { id: 999 }).is_err());
        let rows = resources.registered_models().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].path, "models/a.md3");
        assert_eq!(rows[0].handle, 1);
    }

    #[test]
    fn resource_skins_cache_and_roundtrip_handles() {
        let mut resources: Q3RendererResources<TestResourceHost, TestWorld> =
            Q3RendererResources::new(TestResourceHost::new());
        let first = resources.register_skin("models/a.skin").unwrap().unwrap();
        let second = resources.register_skin("models/a.skin").unwrap().unwrap();
        assert_eq!(first, second);
        assert_eq!(resources.skin_handle(&Some(first.clone())).unwrap(), 1);
        assert_eq!(resources.skin_handle(&None).unwrap(), 0);
        assert_eq!(resources.skin_for_handle(1).unwrap(), Some(first));
        assert_eq!(resources.skin_for_handle(0).unwrap(), None);
        assert!(resources.skin_for_handle(7).is_err());
        assert!(resources.skin_for_handle(-1).is_err());
        let rows = resources.registered_skins().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].path, "models/a.skin");
    }

    #[test]
    fn resource_shaders_fold_case_and_zero_picture() {
        let mut resources: Q3RendererResources<TestResourceHost, TestWorld> =
            Q3RendererResources::new(TestResourceHost::new());
        let upper = resources.register_shader("textures/Foo").unwrap().unwrap();
        let lower = resources.register_shader("textures/foo").unwrap().unwrap();
        assert_eq!(upper, lower);
        assert_eq!(resources.register_shader_no_mip(None).unwrap(), None);
        let picture = resources.picture(Some(&upper)).unwrap();
        assert_eq!(picture.name, "textures/Foo");
        assert_eq!(resources.picture(None).unwrap().id, 0);
        assert_eq!(resources.shader_for_handle(0).unwrap(), None);
        let missing = SceneShader {
            id: 77,
            name: "nope".to_string(),
            material_order: 77,
        };
        assert!(resources.picture(Some(&missing)).is_err());
        assert!(resources.shader_for_handle(77).is_err());
    }

    #[test]
    fn resource_renderer_shader_uses_material_order() {
        let mut resources: Q3RendererResources<TestResourceHost, TestWorld> =
            Q3RendererResources::new(TestResourceHost::new());
        let shader = resources.register_shader("s").unwrap().unwrap();
        assert_eq!(resources.shader_handle(Some(&shader)).unwrap(), 1);
        assert_eq!(resources.shader_handle(None).unwrap(), 0);
        assert_eq!(resources.shader_for_handle(1).unwrap(), Some(shader));
    }

    #[test]
    fn resource_checkpoint_roundtrips_client_owner() {
        let mut resources =
            Q3RendererResources::with_world(TestResourceHost::new(), None::<TestWorld>, ResourceHandleOwner::Client);
        let model = resources.register_model(Some("m")).unwrap();
        let skin = resources.register_skin("s").unwrap();
        let shader = resources.register_shader("h").unwrap();
        let checkpoint = resources.capture_checkpoint().unwrap();
        assert_eq!(checkpoint.models.len(), 1);
        assert_eq!(checkpoint.models[0].handle, 1);
        assert_eq!(checkpoint.models[0].resource, model.resource_id());
        assert_eq!(checkpoint.skins[0].handle, 1);
        assert_eq!(checkpoint.shaders[0].handle, 1);
        let mut restored =
            Q3RendererResources::with_world(TestResourceHost::new(), None::<TestWorld>, ResourceHandleOwner::Client);
        restored.restore_checkpoint(&checkpoint).unwrap();
        assert_eq!(restored.model_for_handle(1).unwrap(), model);
        assert_eq!(restored.skin_for_handle(1).unwrap(), skin);
        assert_eq!(restored.shader_for_handle(1).unwrap(), shader);
        assert!(restored.restore_checkpoint(&checkpoint).is_err());
    }

    #[test]
    fn resource_checkpoint_requires_client_owner() {
        let resources: Q3RendererResources<TestResourceHost, TestWorld> =
            Q3RendererResources::new(TestResourceHost::new());
        assert!(resources.capture_checkpoint().is_err());
    }

    #[test]
    fn resource_entity_tokens_skip_comments_and_restart() {
        let world = TestWorld {
            map: ResourceWorldMap {
                entities: "{\n\"classname\" \"worldspawn\" // trailing\n/* block */ key value\n}".to_string(),
                nodes: Vec::new(),
                leaves: vec![ResourceBspLeaf { cluster: 0 }],
                planes: Vec::new(),
            },
            pvs: vec![1],
        };
        let mut resources =
            Q3RendererResources::with_world(TestResourceHost::new(), Some(world), ResourceHandleOwner::Renderer);
        resources.load_world("maps/test.bsp").unwrap();
        let mut tokens = Vec::new();
        loop {
            let mut token = String::new();
            let more = resources
                .get_entity_token(&mut |text: &str| token = text.to_string())
                .unwrap();
            tokens.push(token);
            if !more {
                break;
            }
        }
        assert_eq!(tokens, vec!["{", "classname", "worldspawn", "key", "value", "}", ""]);
        let mut restart = String::new();
        assert!(resources
            .get_entity_token(&mut |text: &str| restart = text.to_string())
            .unwrap());
        assert_eq!(restart, "{");
    }

    #[test]
    fn resource_entity_cursor_rejects_non_latin1() {
        assert!(EntityParseCursor::new("héllo \u{0100}").is_err());
        assert!(EntityParseCursor::new("plain").is_ok());
    }

    #[test]
    fn resource_pvs_reports_visibility() {
        let visible = TestWorld::flat(0, true);
        let resources =
            Q3RendererResources::with_world(TestResourceHost::new(), Some(visible), ResourceHandleOwner::Renderer);
        let mut resources = resources;
        resources.load_world("maps/test.bsp").unwrap();
        let mut zero = || vec3(0.0, 0.0, 0.0);
        let mut zero_b = || vec3(0.0, 0.0, 0.0);
        assert!(resources.in_pvs(&mut zero, &mut zero_b).unwrap());

        let hidden = TestWorld::flat(0, false);
        let mut resources =
            Q3RendererResources::with_world(TestResourceHost::new(), Some(hidden), ResourceHandleOwner::Renderer);
        resources.load_world("maps/test.bsp").unwrap();
        assert!(!resources.in_pvs(&mut zero, &mut zero_b).unwrap());
    }

    #[test]
    fn resource_point_cluster_walks_nodes() {
        let world = TestWorld {
            map: ResourceWorldMap {
                entities: String::new(),
                nodes: vec![
                    ResourceBspNode {
                        plane: 0,
                        children: [ResourceBspChild::Leaf { index: 0 }, ResourceBspChild::Node { index: 1 }],
                    },
                    ResourceBspNode {
                        plane: 0,
                        children: [ResourceBspChild::Leaf { index: 1 }, ResourceBspChild::Leaf { index: 1 }],
                    },
                ],
                leaves: vec![ResourceBspLeaf { cluster: 7 }, ResourceBspLeaf { cluster: 3 }],
                planes: vec![Plane {
                    normal: vec3(0.0, 0.0, 1.0),
                    distance: 0.0,
                }],
            },
            pvs: vec![0x80],
        };
        let mut resources =
            Q3RendererResources::with_world(TestResourceHost::new(), Some(world), ResourceHandleOwner::Renderer);
        resources.load_world("maps/test.bsp").unwrap();
        let mut front = || vec3(0.0, 0.0, 5.0);
        let mut front_b = || vec3(0.0, 0.0, 5.0);
        let mut back = || vec3(0.0, 0.0, -5.0);
        assert!(resources.in_pvs(&mut front, &mut front_b).unwrap());
        assert!(!resources.in_pvs(&mut front, &mut back).unwrap());
    }

    #[test]
    fn resource_point_cluster_validates_world() {
        let resources = Q3RendererResources::new(TestResourceHost::new());
        let resources: Q3RendererResources<TestResourceHost, TestWorld> = resources;
        let mut zero = || vec3(0.0, 0.0, 0.0);
        let mut zero_b = || vec3(0.0, 0.0, 0.0);
        assert!(matches!(
            resources.in_pvs(&mut zero, &mut zero_b),
            Err(PresentClientError::Drop(_))
        ));
        let world = TestWorld {
            map: ResourceWorldMap {
                entities: String::new(),
                nodes: vec![ResourceBspNode {
                    plane: 9,
                    children: [ResourceBspChild::Leaf { index: 0 }, ResourceBspChild::Leaf { index: 0 }],
                }],
                leaves: vec![ResourceBspLeaf { cluster: 0 }],
                planes: Vec::new(),
            },
            pvs: vec![1],
        };
        let mut resources =
            Q3RendererResources::with_world(TestResourceHost::new(), Some(world), ResourceHandleOwner::Renderer);
        resources.load_world("maps/test.bsp").unwrap();
        assert!(matches!(
            resources.in_pvs(&mut zero, &mut zero_b),
            Err(PresentClientError::Range(_))
        ));
    }

    // ---------- client-info tests ----------

    const CLIENT_CONFIG: &str = "\\n\\Newbie\\c1\\4\\c2\\3\\skill\\5\\hc\\100\\w\\7\\l\\2\\t\\0\\tt\\1\\tl\\0\\g_redteam\\redders\\g_blueteam\\blues\\model\\sarge\\hmodel\\sarge";

    #[test]
    fn client_info_empty_config_clears_slot() {
        let mut store = ClientInfoStore::new(TestClientInfoHost::new());
        let mut slots = vec![ClientInfo::new(); MAX_CLIENTS];
        slots[0].info_valid = true;
        store.new_client_info(&mut slots, 0, "").unwrap();
        assert!(!slots[0].info_valid);
        slots[0].info_valid = true;
        store.new_client_info(&mut slots, 0, "\0trailing").unwrap();
        assert!(!slots[0].info_valid);
    }

    #[test]
    fn client_info_rejects_bad_index_and_team() {
        let mut store = ClientInfoStore::new(TestClientInfoHost::new());
        let mut slots = vec![ClientInfo::new(); MAX_CLIENTS];
        assert!(store.new_client_info(&mut slots, 64, CLIENT_CONFIG).is_err());
        assert!(store.new_client_info(&mut slots, -1, CLIENT_CONFIG).is_err());
        assert!(store.new_client_info(&mut slots, 0, "\\t\\9").is_err());
    }

    #[test]
    fn client_info_loads_model_and_sounds() {
        let mut store = ClientInfoStore::new(TestClientInfoHost::with_sarge_files());
        let mut slots = vec![ClientInfo::new(); MAX_CLIENTS];
        store.new_client_info(&mut slots, 0, CLIENT_CONFIG).unwrap();
        let slot = &slots[0];
        assert!(slot.info_valid);
        assert_eq!(slot.name, "Newbie");
        assert_eq!(slot.color1, vec3(1.0, 0.0, 0.0));
        assert_eq!(slot.bot_skill, 5);
        assert_eq!(slot.team, Team::Free);
        assert_eq!(slot.model_name, "sarge");
        assert_eq!(slot.skin_name, "default");
        assert!(slot.new_anims);
        assert!(!slot.legs_model.is_default());
        assert!(slot.model_icon.is_some());
        assert!(slot.sounds[0].is_some());
        assert!(slot.sounds[12].is_some());
        assert!(slot.sounds[13].is_none());
        assert!(!slot.deferred);
        assert!(store.host.prints.is_empty());
    }

    #[test]
    fn client_info_reuses_matching_model() {
        let mut store = ClientInfoStore::new(TestClientInfoHost::with_sarge_files());
        let mut slots = vec![ClientInfo::new(); MAX_CLIENTS];
        store.new_client_info(&mut slots, 0, CLIENT_CONFIG).unwrap();
        let before = store.host.next_model;
        store.new_client_info(&mut slots, 1, CLIENT_CONFIG).unwrap();
        assert!(slots[1].info_valid);
        assert!(!slots[1].deferred);
        assert_eq!(slots[1].legs_model, slots[0].legs_model);
        assert_eq!(store.host.next_model, before);
    }

    #[test]
    fn client_info_defers_without_match() {
        let mut host = TestClientInfoHost::with_sarge_files();
        host.settings.defer_players = true;
        let mut store = ClientInfoStore::new(host);
        let mut slots = vec![ClientInfo::new(); MAX_CLIENTS];
        slots[0].info_valid = true;
        slots[0].model_name = "other".to_string();
        slots[0].skin_name = "other".to_string();
        slots[0].head_model_name = "other".to_string();
        slots[0].head_skin_name = "other".to_string();
        let dummy = PlayerAnimation {
            name: "x".to_string(),
            first_frame: 0,
            num_frames: 1,
            loop_frames: 0,
            frame_lerp: 100,
            initial_lerp: 100,
            reversed: false,
            flipflop: false,
        };
        slots[0].animations = vec![Some(dummy); CLIENT_ANIMATION_COUNT];
        store.new_client_info(&mut slots, 1, CLIENT_CONFIG).unwrap();
        assert!(slots[1].info_valid);
        assert!(slots[1].deferred);
        assert_eq!(slots[1].model_name, "sarge");
    }

    #[test]
    fn client_info_custom_sounds() {
        let mut host = TestClientInfoHost::new();
        host.cache.insert("sound/x.wav".to_string(), PcmSound::new(7));
        let mut store = ClientInfoStore::new(host);
        let mut slots = vec![ClientInfo::new(); MAX_CLIENTS];
        slots[0].sounds[0] = Some(PcmSound::new(1));
        assert_eq!(
            store.custom_sound(&slots, 0, "sound/x.wav").unwrap(),
            Some(PcmSound::new(7))
        );
        assert_eq!(store.custom_sound(&slots, 0, "sound/missing.wav").unwrap(), None);
        assert_eq!(
            store.custom_sound(&slots, 0, "*death1.wav").unwrap(),
            Some(PcmSound::new(1))
        );
        assert_eq!(
            store.custom_sound(&slots, 99, "*death1.wav").unwrap(),
            Some(PcmSound::new(1))
        );
        assert!(matches!(
            store.custom_sound(&slots, 0, "*nope.wav"),
            Err(PresentClientError::Drop(_))
        ));
    }

    #[test]
    fn custom_sound_fallback_names() {
        assert_eq!(custom_sound_fallback(Q3Product::MissionPack, true), "james");
        assert_eq!(custom_sound_fallback(Q3Product::MissionPack, false), "sarge");
        assert_eq!(custom_sound_fallback(Q3Product::BaseQ3, true), "sarge");
    }

    // ---------- presenter tests ----------

    fn render_fixture() -> (Vec<ClientInfo>, [SkullTrail; MAX_CLIENTS], ClientEntity) {
        let clients = vec![ClientInfo::new(); MAX_CLIENTS];
        let trails: [SkullTrail; MAX_CLIENTS] = std::array::from_fn(|_| SkullTrail {
            positions: [vec3(0.0, 0.0, 0.0); 10],
            num_positions: 0,
        });
        let mut entity = ClientEntity::new();
        entity.current_state.number = 7;
        entity.current_state.e_type = EntityType::Player as i32;
        entity.current_state.client_num = 0;
        (clients, trails, entity)
    }

    #[test]
    fn presenter_rejects_product_mismatch() {
        let mut host = TestPlayerHost::new();
        host.product = Q3Product::MissionPack;
        assert!(PlayerPresenter::new(host, Q3Product::BaseQ3).is_err());
    }

    #[test]
    fn presenter_rejects_bad_client_number() {
        let host = TestPlayerHost::new();
        let mut presenter = PlayerPresenter::new(host, Q3Product::BaseQ3).unwrap();
        let (mut clients, mut trails, mut entity) = render_fixture();
        entity.current_state.client_num = 99;
        let mut ctx = PlayerRenderContext {
            time: 1000,
            frame_time: 50,
            rendering_third_person: true,
            snapshot_client_num: 3,
            snapshot_team: Team::Red as i32,
            clients: &mut clients,
            skull_trails: &mut trails,
        };
        assert!(matches!(
            presenter.player(&mut ctx, &mut entity),
            Err(PresentClientError::Drop(_))
        ));
    }

    #[test]
    fn presenter_skips_invalid_info() {
        let host = TestPlayerHost::new();
        let mut presenter = PlayerPresenter::new(host, Q3Product::BaseQ3).unwrap();
        let (mut clients, mut trails, mut entity) = render_fixture();
        let mut ctx = PlayerRenderContext {
            time: 1000,
            frame_time: 50,
            rendering_third_person: true,
            snapshot_client_num: 3,
            snapshot_team: Team::Red as i32,
            clients: &mut clients,
            skull_trails: &mut trails,
        };
        presenter.player(&mut ctx, &mut entity).unwrap();
        assert!(presenter.host.entities.is_empty());
        assert_eq!(presenter.host.weapons_added, 0);
    }

    #[test]
    fn presenter_renders_body_and_weapon() {
        let host = TestPlayerHost::new();
        let mut presenter = PlayerPresenter::new(host, Q3Product::BaseQ3).unwrap();
        let (mut clients, mut trails, mut entity) = render_fixture();
        clients[0].info_valid = true;
        clients[0].team = Team::Red;
        clients[0].legs_model = SceneModel::Loaded { id: 1 };
        clients[0].torso_model = SceneModel::Loaded { id: 2 };
        clients[0].head_model = SceneModel::Loaded { id: 3 };
        let mut ctx = PlayerRenderContext {
            time: 1000,
            frame_time: 50,
            rendering_third_person: true,
            snapshot_client_num: 3,
            snapshot_team: Team::Red as i32,
            clients: &mut clients,
            skull_trails: &mut trails,
        };
        presenter.player(&mut ctx, &mut entity).unwrap();
        assert_eq!(presenter.host.entities.len(), 3);
        assert_eq!(presenter.host.weapons_added, 1);
        assert!(presenter.host.polys.is_empty());
    }

    #[test]
    fn presenter_quad_adds_shell_and_light() {
        let mut host = TestPlayerHost::new();
        host.media.quad_shader = Some(SceneShader {
            id: 9,
            name: "quad".to_string(),
            material_order: 9,
        });
        let mut presenter = PlayerPresenter::new(host, Q3Product::BaseQ3).unwrap();
        let (mut clients, mut trails, mut entity) = render_fixture();
        clients[0].info_valid = true;
        clients[0].team = Team::Red;
        clients[0].legs_model = SceneModel::Loaded { id: 1 };
        clients[0].torso_model = SceneModel::Loaded { id: 2 };
        clients[0].head_model = SceneModel::Loaded { id: 3 };
        entity.current_state.powerups = 1 << (Powerup::Quad as i32);
        let mut ctx = PlayerRenderContext {
            time: 1000,
            frame_time: 50,
            rendering_third_person: true,
            snapshot_client_num: 3,
            snapshot_team: Team::Red as i32,
            clients: &mut clients,
            skull_trails: &mut trails,
        };
        presenter.player(&mut ctx, &mut entity).unwrap();
        assert_eq!(presenter.host.entities.len(), 6);
        assert_eq!(presenter.host.lights.len(), 1);
        assert_eq!(presenter.host.lights[0].color, vec3(0.2, 0.2, 1.0));
    }

    #[test]
    fn presenter_shadow_reports_mark_plane() {
        let mut host = TestPlayerHost::new();
        host.settings.shadows = 1;
        host.trace_result = TraceResult {
            fraction: 0.5,
            end: vec3(0.0, 0.0, 10.0),
            solidity: TraceSolidity::Clear,
            contact: TraceContact::Plane {
                plane: Plane {
                    normal: vec3(0.0, 0.0, 1.0),
                    distance: 10.0,
                },
            },
            contents: 0,
            surface_flags: 0,
        };
        let mut presenter = PlayerPresenter::new(host, Q3Product::BaseQ3).unwrap();
        let (mut clients, mut trails, mut entity) = render_fixture();
        clients[0].info_valid = true;
        clients[0].team = Team::Red;
        clients[0].legs_model = SceneModel::Loaded { id: 1 };
        clients[0].torso_model = SceneModel::Loaded { id: 2 };
        clients[0].head_model = SceneModel::Loaded { id: 3 };
        let mut ctx = PlayerRenderContext {
            time: 1000,
            frame_time: 50,
            rendering_third_person: true,
            snapshot_client_num: 3,
            snapshot_team: Team::Red as i32,
            clients: &mut clients,
            skull_trails: &mut trails,
        };
        presenter.player(&mut ctx, &mut entity).unwrap();
        assert_eq!(presenter.host.polys.len(), 1);
        match &presenter.host.entities[0] {
            RefEntity::Model(legs) => assert_eq!(legs.shadow_plane, 11.0),
            RefEntity::Sprite(_) => panic!("legs must render as a model"),
        }
    }

    #[test]
    fn presenter_reset_rebuilds_lerp() {
        let mut host = TestPlayerHost::new();
        host.settings.debug_position = true;
        let mut presenter = PlayerPresenter::new(host, Q3Product::BaseQ3).unwrap();
        let mut entity = ClientEntity::new();
        entity.current_state.pos.base = vec3(10.0, 20.0, 30.0);
        let ci = ClientInfo::new();
        presenter.reset_player_entity(1000, &mut entity, &ci).unwrap();
        assert_eq!(entity.error_time, -99999);
        assert!(!entity.extrapolated);
        assert_eq!(entity.lerp_origin, vec3(10.0, 20.0, 30.0));
        assert_eq!(entity.raw_origin, vec3(10.0, 20.0, 30.0));
        assert_eq!(entity.player.legs.base.animation_number, 0);
        assert_eq!(presenter.host.prints.len(), 1);
        assert!(presenter.host.prints[0].contains("ResetPlayerEntity"));
    }

    #[test]
    fn presenter_light_verts_requires_vertices() {
        let host = TestPlayerHost::new();
        let mut presenter = PlayerPresenter::new(host, Q3Product::BaseQ3).unwrap();
        assert!(presenter.light_verts(vec3(0.0, 0.0, 1.0), &mut []).is_err());
        let mut vertices = vec![RefPolyVertex {
            position: vec3(0.0, 0.0, 0.0),
            tex_coord: [0.0, 0.0],
            color: vec4(0.0, 0.0, 0.0, 0.0),
        }];
        assert!(presenter.light_verts(vec3(0.0, 0.0, 1.0), &mut vertices).unwrap());
        assert_eq!(vertices[0].color, vec4(30.0, 30.0, 30.0, 255.0));
        assert!(presenter.light_verts(vec3(0.0, 0.0, -1.0), &mut vertices).unwrap());
        assert_eq!(vertices[0].color, vec4(10.0, 10.0, 10.0, 255.0));
    }

    // ---------- frame tests ----------

    fn snap_fixture() -> Snapshot {
        let mut ps = PlayerState::new(Q3Product::BaseQ3);
        ps.client_num = 3;
        ps.weapon = 2;
        ps.stats
            .set(stat_schema(Q3Product::BaseQ3).weapons, (1 << 2) | (1 << 5))
            .unwrap();
        ps.persistant
            .set(PersistentIndex::Team as i32, Team::Free as i32)
            .unwrap();
        Snapshot {
            message_number: 1,
            server_time: 1000,
            delta_number: 0,
            flags: 0,
            server_command_number: 0,
            parse_entities_number: 0,
            area_mask: [0; 32],
            player_state: ps,
            entities: vec![EntityState {
                number: 5,
                e_type: EntityType::Player as i32,
                weapon: 3,
                ..EntityState::default()
            }],
        }
    }

    fn frame_input() -> Q3PresentationFrame {
        Q3PresentationFrame {
            server_time: 100,
            stereo: FrameStereo::Center,
            demo_playback: false,
            engine_frame_number: 7,
        }
    }

    #[test]
    fn frame_scope_guards_hold() {
        let mut scene = Q3PresentationFrameRuntime::new_scene(TestFrameHost::new());
        let mut state = test_state();
        let mut static_state = test_static();
        assert!(scene
            .draw_active_frame(&mut state, &mut static_state, &frame_input())
            .is_err());
        let mut primary = Q3PresentationFrameRuntime::new(TestFrameHost::new());
        let camera = SceneCamera {
            origin: vec3(0.0, 0.0, 0.0),
            axis: [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)],
            projection: [1.0; 16],
            viewport_x: 0,
            viewport_y: 0,
            viewport_width: 640,
            viewport_height: 480,
        };
        assert!(primary
            .draw_scene_frame(&mut state, &mut static_state, &frame_input(), &camera)
            .is_err());
        primary.close().unwrap();
        assert!(primary
            .draw_active_frame(&mut state, &mut static_state, &frame_input())
            .is_err());
    }

    #[test]
    fn frame_registers_weapons_and_renders() {
        let mut host = TestFrameHost::new();
        host.snap = Some(snap_fixture());
        let mut frames = Q3PresentationFrameRuntime::new(host);
        let mut state = test_state();
        let mut static_state = test_static();
        state.weapon_select = 5;
        state.zoom_sensitivity = 1.5;
        state.hyperspace = false;
        state.rendering_third_person = false;
        state.entity_at_mut(5).unwrap().current_state.e_type = EntityType::Player as i32;
        state.entity_at_mut(5).unwrap().current_state.weapon = 3;
        let before = state.client_frame;
        frames
            .draw_active_frame(&mut state, &mut static_state, &frame_input())
            .unwrap();
        assert_eq!(frames.host.weapons, vec![2, 5, 3]);
        assert_eq!(frames.host.rendered, 1);
        assert_eq!(frames.host.draws_2d, 1);
        assert_eq!(frames.host.view_weapons, 1);
        assert_eq!(state.client_frame, before.wrapping_add(1));
        assert_eq!(state.frame_time, 100);
        assert_eq!(state.old_time, 100);
        assert_eq!(frames.host.user_command, Some((5, 1.5)));
        assert_eq!(frames.host.listener.unwrap().0, 3);
        assert_eq!(frames.host.lag, 1);
        assert_eq!(state.refdef.view_origin, vec3(0.0, 0.0, 0.0));
    }

    #[test]
    fn frame_loading_paths_skip_render() {
        let mut host = TestFrameHost::new();
        host.snap = Some(snap_fixture());
        let mut frames = Q3PresentationFrameRuntime::new(host);
        let mut state = test_state();
        let mut static_state = test_static();
        state.info_screen_text = "loading".to_string();
        frames
            .draw_active_frame(&mut state, &mut static_state, &frame_input())
            .unwrap();
        assert_eq!(frames.host.loading_frames, 1);
        assert_eq!(frames.host.rendered, 0);

        let mut frames = Q3PresentationFrameRuntime::new(TestFrameHost::new());
        let mut state = test_state();
        frames
            .draw_active_frame(&mut state, &mut static_state, &frame_input())
            .unwrap();
        assert_eq!(frames.host.loading_frames, 1);
        assert_eq!(frames.host.rendered, 0);
    }

    #[test]
    fn frame_spectator_scoreboard_skips_scene() {
        let mut snapshot = snap_fixture();
        snapshot
            .player_state
            .persistant
            .set(PersistentIndex::Team as i32, Team::Spectator as i32)
            .unwrap();
        snapshot.player_state.pm_flags = MoveFlags::SCOREBOARD;
        let mut host = TestFrameHost::new();
        host.snap = Some(snapshot);
        let mut frames = Q3PresentationFrameRuntime::new(host);
        let mut state = test_state();
        let mut static_state = test_static();
        frames
            .draw_active_frame(&mut state, &mut static_state, &frame_input())
            .unwrap();
        assert_eq!(frames.host.scoreboards, 1);
        assert_eq!(frames.host.rendered, 0);
        assert_eq!(frames.host.draws_2d, 0);
    }

    #[test]
    fn frame_right_eye_skips_frame_time() {
        let mut host = TestFrameHost::new();
        host.snap = Some(snap_fixture());
        let mut frames = Q3PresentationFrameRuntime::new(host);
        let mut state = test_state();
        let mut static_state = test_static();
        let input = Q3PresentationFrame {
            stereo: FrameStereo::Right,
            ..frame_input()
        };
        frames.draw_active_frame(&mut state, &mut static_state, &input).unwrap();
        assert_eq!(frames.host.lag, 0);
        assert_eq!(state.frame_time, 0);
        assert_eq!(frames.host.rendered, 1);
    }

    #[test]
    fn frame_timescale_fades_toward_end() {
        let mut host = TestFrameHost::new();
        host.snap = Some(snap_fixture());
        host = host
            .with_cvar(FrameCvar::CgTimescaleFadeEnd, 1.0, 1)
            .with_cvar(FrameCvar::CgTimescaleFadeSpeed, 1.0, 1)
            .with_cvar(FrameCvar::CgTimescale, 0.0, 0);
        let mut frames = Q3PresentationFrameRuntime::new(host);
        let mut state = test_state();
        let mut static_state = test_static();
        frames
            .draw_active_frame(&mut state, &mut static_state, &frame_input())
            .unwrap();
        let cvar = frames.host.timescale_cvar.unwrap();
        assert!((cvar - 0.1).abs() < 1e-6, "unexpected timescale {cvar}");
        assert!((frames.host.timescale.unwrap() - 0.1).abs() < 1e-6);
    }

    #[test]
    fn frame_scene_sets_refdef_from_camera() {
        let mut snapshot = snap_fixture();
        snapshot.entities[0].e_type = EntityType::Missile as i32;
        snapshot.entities[0].weapon = 99;
        let mut host = TestFrameHost::new();
        host.snap = Some(snapshot);
        let mut frames = Q3PresentationFrameRuntime::new_scene(host);
        let mut state = test_state();
        let mut static_state = test_static();
        let camera = SceneCamera {
            origin: vec3(1.0, 2.0, 3.0),
            axis: [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)],
            projection: [1.0; 16],
            viewport_x: 0,
            viewport_y: 0,
            viewport_width: 640,
            viewport_height: 480,
        };
        frames
            .draw_scene_frame(&mut state, &mut static_state, &frame_input(), &camera)
            .unwrap();
        assert_eq!(state.refdef.view_origin, vec3(1.0, 2.0, 3.0));
        assert_eq!(state.refdef.width, 640);
        assert_eq!(state.refdef.height, 480);
        assert!((state.refdef.fov_x - 90.0).abs() < 1e-4);
        assert!((state.refdef.fov_y - 90.0).abs() < 1e-4);
        assert_eq!(state.refdef.time, 100);
        assert_eq!(frames.host.weapons, vec![2, 0]);
    }

    #[test]
    fn frame_scene_rejects_backward_time() {
        let mut frames = Q3PresentationFrameRuntime::new_scene(TestFrameHost::new());
        let mut state = test_state();
        let mut static_state = test_static();
        state.old_time = 200;
        let camera = SceneCamera {
            origin: vec3(0.0, 0.0, 0.0),
            axis: [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)],
            projection: [1.0; 16],
            viewport_x: 0,
            viewport_y: 0,
            viewport_width: 640,
            viewport_height: 480,
        };
        assert!(frames
            .draw_scene_frame(&mut state, &mut static_state, &frame_input(), &camera)
            .is_err());
    }

    // ---------- assembly tests ----------

    fn session_game_state() -> Vec<String> {
        let mut strings = vec![String::new(); 22];
        strings[20] = "baseq3-1".to_string();
        strings[21] = "111".to_string();
        strings
    }

    #[test]
    fn game_match_reports_level_time() {
        assert_eq!(check_game_match(&session_game_state()).unwrap(), 111);
        let mut bad = session_game_state();
        bad[20] = "mission-1".to_string();
        assert!(matches!(check_game_match(&bad), Err(PresentClientError::Drop(_))));
        assert!(check_game_match(&[]).is_err());
    }

    #[test]
    fn create_client_presentation_initializes_and_closes() {
        let mut session = TestSession::new(session_game_state());
        let options = Q3ClientPresentationOptions {
            snapshot_host: TestSnapHost,
            collision: TestCollision,
            prediction_host: TestPredHost,
            player_state_host: TestPsHost::new(),
            event_host: TestEventHost::new(),
            server_command_host: TestServerHost::new(),
            client_info_host: TestClientInfoHost::new(),
            player_host: TestPlayerHost::new(),
            resource_host: TestResourceHost::new(),
            resource_world: None::<TestWorld>,
            resource_handles: ResourceHandleOwner::Renderer,
            frame_host: TestFrameHost::new(),
            hardware: ClientHardware::Generic,
        };
        let mut presentation = create_q3_client_presentation(&mut session, options).unwrap();
        assert_eq!(presentation.state.weapon_select, Weapon::Machinegun as i32);
        assert_eq!(presentation.static_state.redflag, -1);
        assert_eq!(presentation.static_state.blueflag, -1);
        assert_eq!(presentation.static_state.flag_status, -1);
        assert_eq!(presentation.static_state.server_command_sequence, 4);
        assert_eq!(presentation.static_state.level_start_time, 222);
        assert_eq!(presentation.static_state.game_type, GameType::Ffa);
        assert!(presentation.state.info_screen_text.is_empty());
        assert_eq!(
            presentation.server_commands.host.background,
            Some(("intro".to_string(), "loop".to_string()))
        );
        assert_eq!(presentation.server_commands.host.looping, vec![true]);
        presentation.close().unwrap();
        assert!(!presentation.static_state.client_info[0].info_valid);
        assert_eq!(presentation.resources.host.cleared, 1);
    }

    #[test]
    fn create_client_presentation_rejects_mismatch() {
        let mut strings = session_game_state();
        strings[20] = "other-1".to_string();
        let mut session = TestSession::new(strings);
        let options = Q3ClientPresentationOptions {
            snapshot_host: TestSnapHost,
            collision: TestCollision,
            prediction_host: TestPredHost,
            player_state_host: TestPsHost::new(),
            event_host: TestEventHost::new(),
            server_command_host: TestServerHost::new(),
            client_info_host: TestClientInfoHost::new(),
            player_host: TestPlayerHost::new(),
            resource_host: TestResourceHost::new(),
            resource_world: None::<TestWorld>,
            resource_handles: ResourceHandleOwner::Renderer,
            frame_host: TestFrameHost::new(),
            hardware: ClientHardware::Generic,
        };
        assert!(matches!(
            create_q3_client_presentation(&mut session, options),
            Err(PresentClientError::Drop(_))
        ));
    }

    #[test]
    fn create_scene_presentation_skips_music_and_closes() {
        let mut session = TestSession::new(session_game_state());
        let options = Q3ScenePresentationOptions {
            snapshot_host: TestSnapHost,
            event_host: TestEventHost::new(),
            server_command_host: TestServerHost::new(),
            client_info_host: TestClientInfoHost::new(),
            resource_host: TestResourceHost::new(),
            resource_world: None::<TestWorld>,
            resource_handles: ResourceHandleOwner::Renderer,
            frame_host: TestFrameHost::new(),
            hardware: ClientHardware::RagePro,
        };
        let mut presentation = create_q3_scene_presentation(&mut session, options).unwrap();
        assert_eq!(presentation.state.weapon_select, Weapon::Machinegun as i32);
        assert_eq!(presentation.static_state.level_start_time, 222);
        assert_eq!(presentation.server_commands.host.background, None);
        assert_eq!(presentation.hardware, ClientHardware::RagePro);
        presentation.close().unwrap();
    }

    #[test]
    fn scene_session_defaults_status_visible() {
        let session = TestSession::new(session_game_state());
        assert!(session.status_visible());
    }
}
