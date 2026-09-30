//! Quake III presentation HUD (`q3_present_hud`) support: shared mirrors, group error, and tests.
//!
//! Self-containment mirrors: minimal local copies of items the donors import from
//! modules outside this port (sibling q3 donors, engine contracts, and math/text
//! helpers), plus the group error type. Sibling-owned mirrors carry SIBLING-MIRROR
//! notes and unify with the canonical ports at merge time.

use qa_core::cvar::{CvarRegistry, CvarSnapshot};
use qa_core::math::{vec3, vec4, Axis, Bounds, Vec3, Vec4};
use qa_core::numeric::q_rand;
use std::cell::RefCell;
use std::rc::Rc;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::presentation::audio::*;
use crate::q3::presentation::client_info::*;
use crate::q3::presentation::ui_adapters::*;

/// Shared ownership handle for donor reference-semantics objects.
pub type Shared<T> = Rc<RefCell<T>>;

/// Build a [`Shared`] handle.
pub fn shared<T>(value: T) -> Shared<T> {
    Rc::new(RefCell::new(value))
}

/// Pointer identity between two [`Shared`] handles.
pub fn same<T: ?Sized>(a: &Shared<T>, b: &Shared<T>) -> bool {
    Rc::ptr_eq(a, b)
}

/// Rejection-style failure (donor promise rejection / `HudError`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HudError {
    /// Donor message text.
    pub message: String,
}

impl HudError {
    /// Build a rejection carrying the donor message.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for HudError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for HudError {}

/// Content product (`Product`: `"baseq3" | "missionpack"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Product {
    /// Base Quake III.
    Baseq3,
    /// Team Arena.
    Missionpack,
}

impl Product {
    /// Donor string spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Baseq3 => "baseq3",
            Self::Missionpack => "missionpack",
        }
    }

    /// Whether this is the Team Arena product.
    #[must_use]
    pub fn is_missionpack(self) -> bool {
        matches!(self, Self::Missionpack)
    }
}

/// Game type (`GameType`), ordered so `>= GT_TEAM` / `< GT_CTF` work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(i32)]
pub enum GameType {
    /// Free for all.
    Ffa = 0,
    /// Tournament.
    Tournament = 1,
    /// Single player.
    SinglePlayer = 2,
    /// Team deathmatch.
    Team = 3,
    /// Capture the flag.
    Ctf = 4,
    /// One-flag CTF.
    OneFlagCtf = 5,
    /// Overload.
    Obelisk = 6,
    /// Harvester.
    Harvester = 7,
    /// Sentinel.
    MaxGameType = 8,
}

impl GameType {
    /// Convert a raw config integer.
    #[must_use]
    pub fn from_i32(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::Ffa),
            1 => Some(Self::Tournament),
            2 => Some(Self::SinglePlayer),
            3 => Some(Self::Team),
            4 => Some(Self::Ctf),
            5 => Some(Self::OneFlagCtf),
            6 => Some(Self::Obelisk),
            7 => Some(Self::Harvester),
            8 => Some(Self::MaxGameType),
            _ => None,
        }
    }
}

/// Team (`Team`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum Team {
    /// Free.
    Free = 0,
    /// Red.
    Red = 1,
    /// Blue.
    Blue = 2,
    /// Spectator.
    Spectator = 3,
    /// Sentinel.
    NumTeams = 4,
}

impl Team {
    /// Convert a raw config integer.
    #[must_use]
    pub fn from_i32(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::Free),
            1 => Some(Self::Red),
            2 => Some(Self::Blue),
            3 => Some(Self::Spectator),
            4 => Some(Self::NumTeams),
            _ => None,
        }
    }
}

/// Powerup (`Powerup`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum Powerup {
    /// None.
    None = 0,
    /// Quad.
    Quad = 1,
    /// Battlesuit.
    Battlesuit = 2,
    /// Haste.
    Haste = 3,
    /// Invisibility.
    Invis = 4,
    /// Regeneration.
    Regen = 5,
    /// Flight.
    Flight = 6,
    /// Red flag.
    RedFlag = 7,
    /// Blue flag.
    BlueFlag = 8,
    /// Neutral flag.
    NeutralFlag = 9,
    /// Scout.
    Scout = 10,
    /// Guard.
    Guard = 11,
    /// Doubler.
    Doubler = 12,
    /// Ammo regeneration.
    AmmoRegen = 13,
    /// Invulnerability.
    Invulnerability = 14,
    /// Sentinel / slot count.
    NumPowerups = 15,
}

impl Powerup {
    /// Convert a raw slot integer.
    #[must_use]
    pub fn from_i32(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::None),
            1 => Some(Self::Quad),
            2 => Some(Self::Battlesuit),
            3 => Some(Self::Haste),
            4 => Some(Self::Invis),
            5 => Some(Self::Regen),
            6 => Some(Self::Flight),
            7 => Some(Self::RedFlag),
            8 => Some(Self::BlueFlag),
            9 => Some(Self::NeutralFlag),
            10 => Some(Self::Scout),
            11 => Some(Self::Guard),
            12 => Some(Self::Doubler),
            13 => Some(Self::AmmoRegen),
            14 => Some(Self::Invulnerability),
            15 => Some(Self::NumPowerups),
            _ => None,
        }
    }
}

/// Weapon state (`WeaponState`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum WeaponState {
    /// Ready.
    Ready = 0,
    /// Raising.
    Raising = 1,
    /// Dropping.
    Dropping = 2,
    /// Firing.
    Firing = 3,
}

impl WeaponState {
    /// Convert a raw integer.
    #[must_use]
    pub fn from_i32(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::Ready),
            1 => Some(Self::Raising),
            2 => Some(Self::Dropping),
            3 => Some(Self::Firing),
            _ => None,
        }
    }
}

/// Movement type (`MoveType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum MoveType {
    /// Normal.
    Normal = 0,
    /// Noclip.
    Noclip = 1,
    /// Spectator.
    Spectator = 2,
    /// Dead.
    Dead = 3,
    /// Freeze.
    Freeze = 4,
    /// Intermission.
    Intermission = 5,
    /// Single-player intermission.
    SpIntermission = 6,
}

impl MoveType {
    /// Convert a raw integer.
    #[must_use]
    pub fn from_i32(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::Normal),
            1 => Some(Self::Noclip),
            2 => Some(Self::Spectator),
            3 => Some(Self::Dead),
            4 => Some(Self::Freeze),
            5 => Some(Self::Intermission),
            6 => Some(Self::SpIntermission),
            _ => None,
        }
    }
}

/// Persistent player-state slot (`PersistentIndex`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum PersistentIndex {
    /// Score.
    Score = 0,
    /// Hits.
    Hits = 1,
    /// Rank.
    Rank = 2,
    /// Team.
    Team = 3,
    /// Spawn count.
    SpawnCount = 4,
    /// Player events.
    PlayerEvents = 5,
    /// Attacker.
    Attacker = 6,
    /// Attackee armor.
    AttackeeArmor = 7,
    /// Killed.
    Killed = 8,
    /// Impressive count.
    ImpressiveCount = 9,
    /// Excellent count.
    ExcellentCount = 10,
    /// Defend count.
    DefendCount = 11,
    /// Assist count.
    AssistCount = 12,
    /// Gauntlet frag count.
    GauntletFragCount = 13,
    /// Captures.
    Captures = 14,
}

/// Base stat slot (`BaseStatIndex`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum BaseStatIndex {
    /// Health.
    Health = 0,
    /// Holdable item.
    HoldableItem = 1,
    /// Weapons.
    Weapons = 2,
    /// Armor.
    Armor = 3,
    /// Dead yaw.
    DeadYaw = 4,
    /// Clients ready.
    ClientsReady = 5,
    /// Max health.
    MaxHealth = 6,
}

/// Team Arena stat slot (`MissionpackStatIndex`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum MissionpackStatIndex {
    /// Health.
    Health = 0,
    /// Holdable item.
    HoldableItem = 1,
    /// Persistent powerup.
    PersistantPowerup = 2,
    /// Weapons.
    Weapons = 3,
    /// Armor.
    Armor = 4,
    /// Dead yaw.
    DeadYaw = 5,
    /// Clients ready.
    ClientsReady = 6,
    /// Max health.
    MaxHealth = 7,
}

/// Resolved stat schema (`statSchema`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatSchema {
    /// Health slot.
    pub health: i32,
    /// Holdable-item slot.
    pub holdable_item: i32,
    /// Weapons slot.
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

/// Resolve the stat schema for a product (`statSchema`).
#[must_use]
pub fn stat_schema(product: Product) -> StatSchema {
    match product {
        Product::Baseq3 => StatSchema {
            health: BaseStatIndex::Health as i32,
            holdable_item: BaseStatIndex::HoldableItem as i32,
            weapons: BaseStatIndex::Weapons as i32,
            armor: BaseStatIndex::Armor as i32,
            dead_yaw: BaseStatIndex::DeadYaw as i32,
            clients_ready: BaseStatIndex::ClientsReady as i32,
            max_health: BaseStatIndex::MaxHealth as i32,
        },
        Product::Missionpack => StatSchema {
            health: MissionpackStatIndex::Health as i32,
            holdable_item: MissionpackStatIndex::HoldableItem as i32,
            weapons: MissionpackStatIndex::Weapons as i32,
            armor: MissionpackStatIndex::Armor as i32,
            dead_yaw: MissionpackStatIndex::DeadYaw as i32,
            clients_ready: MissionpackStatIndex::ClientsReady as i32,
            max_health: MissionpackStatIndex::MaxHealth as i32,
        },
    }
}

/// Armor protection fraction (`ARMOR_PROTECTION`).
pub const ARMOR_PROTECTION: f64 = 0.66;

/// Follow movement flag (`MoveFlags.FOLLOW`).
pub const MOVE_FLAG_FOLLOW: i32 = 4096;

/// Player animation slot (`PlayerAnimation`); index 31 is the sentinel gap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum PlayerAnimation {
    /// Both death 1.
    BothDeath1 = 0,
    /// Both dead 1.
    BothDead1 = 1,
    /// Both death 2.
    BothDeath2 = 2,
    /// Both dead 2.
    BothDead2 = 3,
    /// Both death 3.
    BothDeath3 = 4,
    /// Both dead 3.
    BothDead3 = 5,
    /// Torso gesture.
    TorsoGesture = 6,
    /// Torso attack.
    TorsoAttack = 7,
    /// Torso attack 2.
    TorsoAttack2 = 8,
    /// Torso drop.
    TorsoDrop = 9,
    /// Torso raise.
    TorsoRaise = 10,
    /// Torso stand.
    TorsoStand = 11,
    /// Torso stand 2.
    TorsoStand2 = 12,
    /// Legs walk crouch.
    LegsWalkcr = 13,
    /// Legs walk.
    LegsWalk = 14,
    /// Legs run.
    LegsRun = 15,
    /// Legs back.
    LegsBack = 16,
    /// Legs swim.
    LegsSwim = 17,
    /// Legs jump.
    LegsJump = 18,
    /// Legs land.
    LegsLand = 19,
    /// Legs jump back.
    LegsJumpb = 20,
    /// Legs land back.
    LegsLandb = 21,
    /// Legs idle.
    LegsIdle = 22,
    /// Legs idle crouch.
    LegsIdlecr = 23,
    /// Legs turn.
    LegsTurn = 24,
    /// Torso get flag.
    TorsoGetflag = 25,
    /// Torso guard base.
    TorsoGuardbase = 26,
    /// Torso patrol.
    TorsoPatrol = 27,
    /// Torso follow me.
    TorsoFollowme = 28,
    /// Torso affirmative.
    TorsoAffirmative = 29,
    /// Torso negative.
    TorsoNegative = 30,
    /// Legs back crouch.
    LegsBackcr = 32,
    /// Legs back walk.
    LegsBackwalk = 33,
    /// Flag run.
    FlagRun = 34,
    /// Flag stand.
    FlagStand = 35,
    /// Flag stand-to-run.
    FlagStand2run = 36,
}

/// Animation table length (`FLAG_STAND2RUN + 1`).
pub const ANIMATION_COUNT: usize = 37;

/// Sentinel slot (`TORSO_NEGATIVE + 1`).
pub const ANIMATION_SENTINEL: usize = 31;

/// One parsed animation row (`Animation` / `AnimationCell`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AnimationCell {
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

/// Immutable animation row (`Animation`); same shape as the cell.
pub type Animation = AnimationCell;

/// 2D rectangle (`Rect` / `UiRect`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect2d {
    /// X.
    pub x: f32,
    /// Y.
    pub y: f32,
    /// Width.
    pub width: f32,
    /// Height.
    pub height: f32,
}

/// Build a rectangle.
#[must_use]
pub fn rect2d(x: f32, y: f32, width: f32, height: f32) -> Rect2d {
    Rect2d { x, y, width, height }
}

/// UI rectangle (same shape as [`Rect2d`]).
pub type UiRect = Rect2d;

/// Texture coordinates (`TextureRect`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TextureRect {
    /// S.
    pub s: f32,
    /// T.
    pub t: f32,
    /// S end.
    pub s2: f32,
    /// T end.
    pub t2: f32,
}

/// Zero UV rectangle.
pub const ZERO_UV: TextureRect = TextureRect {
    s: 0.0,
    t: 0.0,
    s2: 0.0,
    t2: 0.0,
};

/// Full UV rectangle.
pub const FULL_UV: TextureRect = TextureRect {
    s: 0.0,
    t: 0.0,
    s2: 1.0,
    t2: 1.0,
};

/// Registered renderer picture (`PictureAsset`); `order` is the source handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Picture {
    /// Material order / handle.
    pub order: u32,
}

/// Zero (null-material) picture.
pub const ZERO_PICTURE: Picture = Picture { order: 0 };

/// `gameFormat` argument (donor `number | string`).
#[derive(Debug, Clone, PartialEq)]
pub enum GameFormatArg {
    /// Signed integer (`%d` / `%i`).
    Int(i32),
    /// Unsigned integer (`%u`, `%o`, `%x`).
    UInt(u32),
    /// Float (`%f`).
    Float(f64),
    /// String (`%s`).
    Text(String),
    /// Character (`%c`).
    Char(char),
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

impl From<usize> for GameFormatArg {
    fn from(value: usize) -> Self {
        Self::Int(value as i32)
    }
}

impl From<f64> for GameFormatArg {
    fn from(value: f64) -> Self {
        Self::Float(value)
    }
}

impl From<&str> for GameFormatArg {
    fn from(value: &str) -> Self {
        Self::Text(value.to_string())
    }
}

impl From<String> for GameFormatArg {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<char> for GameFormatArg {
    fn from(value: char) -> Self {
        Self::Char(value)
    }
}

/// Format a source `va()` string (`gameFormat`), truncated to `max_bytes - 1
/// characters like the donor's NUL-terminated destination.
pub fn game_format(format: &str, args: &[GameFormatArg], max_bytes: usize) -> String {
    if max_bytes < 1 {
        panic!("game format destination capacity must be a positive safe integer");
    }
    let chars: Vec<char> = format.chars().collect();
    let mut out = String::new();
    let mut cursor = 0usize;
    let mut argument = 0usize;
    let take_arg = |argument: &mut usize| -> GameFormatArg {
        let value = args.get(*argument).cloned().unwrap_or(GameFormatArg::Int(0));
        *argument += 1;
        value
    };
    while cursor < chars.len() {
        let literal = chars[cursor];
        if literal != '%' {
            out.push(literal);
            cursor += 1;
            continue;
        }
        cursor += 1;
        if cursor >= chars.len() {
            panic!("unterminated game format specifier");
        }
        let mut left = false;
        let mut zero_pad = false;
        loop {
            match chars.get(cursor) {
                Some('-') => {
                    left = true;
                    cursor += 1;
                }
                Some('0') => {
                    zero_pad = true;
                    cursor += 1;
                }
                _ => break,
            }
        }
        let mut width = 0usize;
        while let Some(digit) = chars.get(cursor).and_then(|c| c.to_digit(10)) {
            width = width * 10 + digit as usize;
            cursor += 1;
        }
        let mut precision: Option<usize> = None;
        if chars.get(cursor) == Some(&'.') {
            cursor += 1;
            let mut value = 0usize;
            while let Some(digit) = chars.get(cursor).and_then(|c| c.to_digit(10)) {
                value = value * 10 + digit as usize;
                cursor += 1;
            }
            precision = Some(value);
        }
        let specifier = *chars.get(cursor).expect("unterminated game format specifier");
        cursor += 1;
        if specifier == '%' {
            out.push('%');
            continue;
        }
        let value = take_arg(&mut argument);
        let mut text = match specifier {
            'd' | 'i' => match value {
                GameFormatArg::Int(v) => format!("{v}"),
                GameFormatArg::UInt(v) => format!("{}", v as i32),
                GameFormatArg::Float(v) => format!("{}", v.trunc() as i32),
                GameFormatArg::Text(v) => v,
                GameFormatArg::Char(v) => format!("{}", v as u32),
            },
            'u' => match value {
                GameFormatArg::Int(v) => format!("{}", v as u32),
                GameFormatArg::UInt(v) => format!("{v}"),
                GameFormatArg::Float(v) => format!("{}", v.trunc() as i32 as u32),
                GameFormatArg::Text(v) => v,
                GameFormatArg::Char(v) => format!("{}", v as u32),
            },
            'o' => match value {
                GameFormatArg::Int(v) => format!("{:o}", v as u32),
                GameFormatArg::UInt(v) => format!("{v:o}"),
                _ => String::from("0"),
            },
            'x' => match value {
                GameFormatArg::Int(v) => format!("{:x}", v as u32),
                GameFormatArg::UInt(v) => format!("{v:x}"),
                _ => String::from("0"),
            },
            'X' => match value {
                GameFormatArg::Int(v) => format!("{:X}", v as u32),
                GameFormatArg::UInt(v) => format!("{v:X}"),
                _ => String::from("0"),
            },
            'f' => {
                let number = match value {
                    GameFormatArg::Float(v) => v,
                    GameFormatArg::Int(v) => f64::from(v),
                    GameFormatArg::UInt(v) => f64::from(v),
                    _ => 0.0,
                };
                match precision {
                    Some(places) => format!("{number:.places$}"),
                    None => format!("{number:.6}"),
                }
            }
            's' => {
                let text = match value {
                    GameFormatArg::Text(v) => v,
                    GameFormatArg::Int(v) => format!("{v}"),
                    GameFormatArg::UInt(v) => format!("{v}"),
                    GameFormatArg::Float(v) => format!("{v}"),
                    GameFormatArg::Char(v) => v.to_string(),
                };
                match precision {
                    Some(places) => text.chars().take(places).collect(),
                    None => text,
                }
            }
            'c' => match value {
                GameFormatArg::Char(v) => v.to_string(),
                GameFormatArg::Int(v) => char::from_u32(v as u32).unwrap_or('\0').to_string(),
                GameFormatArg::UInt(v) => char::from_u32(v).unwrap_or('\0').to_string(),
                _ => String::new(),
            },
            _ => panic!("unsupported game format specifier %{specifier}"),
        };
        if width > 0 {
            let length = text.chars().count();
            if length < width {
                let pad = if zero_pad && !left && matches!(specifier, 'd' | 'i' | 'u' | 'o' | 'x' | 'X' | 'f') {
                    "0".repeat(width - length)
                } else {
                    " ".repeat(width - length)
                };
                if left {
                    text.push_str(&pad);
                } else if zero_pad && text.starts_with('-') {
                    text = format!("-{}{}", pad, &text[1..]);
                } else {
                    text = format!("{pad}{text}");
                }
            }
        }
        out.push_str(&text);
    }
    out.chars().take(max_bytes - 1).collect()
}

/// Byte-oriented cursor over donor text (`NumberInput`).
#[derive(Debug, Clone)]
pub(crate) struct NumberCursor {
    /// Characters (donor UTF-16 units).
    chars: Vec<char>,
    /// Read offset.
    offset: usize,
}

impl NumberCursor {
    /// Wrap input text.
    fn new(text: &str) -> Self {
        Self {
            chars: text.chars().collect(),
            offset: 0,
        }
    }

    /// Peek the current byte (`0` past the end).
    fn byte(&self) -> u32 {
        self.chars.get(self.offset).copied().unwrap_or('\0') as u32
    }

    /// Consume the current byte.
    fn take(&mut self) -> u32 {
        let value = self.byte();
        if self.offset < self.chars.len() {
            self.offset += 1;
        }
        value
    }

    /// Skip ASCII whitespace.
    fn skip_whitespace(&mut self) {
        while matches!(self.byte(), 9 | 10 | 11 | 12 | 13 | 32) {
            self.offset += 1;
        }
    }

    /// Consume an optional sign, returning `+1` or `-1`.
    fn sign(&mut self) -> i32 {
        match self.byte() {
            45 => {
                self.offset += 1;
                -1
            }
            43 => {
                self.offset += 1;
                1
            }
            _ => 1,
        }
    }
}

/// `bg_lib` atoi with wrapped integer digit operations (`gameAtoi`).
#[must_use]
pub fn game_atoi(text: &str) -> i32 {
    let mut input = NumberCursor::new(text);
    input.skip_whitespace();
    if input.byte() == 0 {
        return 0;
    }
    let sign = input.sign();
    let mut value: i32 = 0;
    loop {
        let character = input.take();
        if !(48..=57).contains(&character) {
            break;
        }
        value = value.wrapping_mul(10).wrapping_add(character as i32 - 48);
    }
    value.wrapping_mul(sign)
}

/// `bg_lib` atof: decimal prefix only, binary32 steps (`gameAtof`).
#[must_use]
pub fn game_atof(text: &str) -> f64 {
    let mut input = NumberCursor::new(text);
    input.skip_whitespace();
    if input.byte() == 0 {
        return 0.0;
    }
    let sign = f64::from(input.sign());
    let mut value: f32 = 0.0;
    let mut character = input.byte();
    if input.byte() != 46 {
        loop {
            character = input.take();
            if !(48..=57).contains(&character) {
                break;
            }
            value = value * 10.0 + (character as f32 - 48.0);
        }
    } else {
        input.offset += 1;
    }
    if character == 46 {
        let mut fraction: f32 = 0.1;
        loop {
            character = input.take();
            if !(48..=57).contains(&character) {
                break;
            }
            value += (character as f32 - 48.0) * fraction;
            fraction *= 0.1;
        }
    }
    f64::from(value * sign as f32)
}

/// Look up a backslash info-string key (`Info_ValueForKey`).
pub fn info_value_for_key(input: &str, wanted: &str, maximum_length: usize) -> String {
    if !(1..=8192).contains(&maximum_length) {
        panic!("Invalid source info-string bound");
    }
    let cut = |value: &str| -> String {
        match value.find('\0') {
            Some(end) => value[..end].to_string(),
            None => value.to_string(),
        }
    };
    let text = cut(input);
    let key = cut(wanted);
    if text.chars().count() >= maximum_length {
        panic!("Info_ValueForKey: oversize infostring");
    }
    for value in [&text, &key] {
        for ch in value.chars() {
            if ch as u32 > 255 {
                panic!("Info_ValueForKey requires byte characters");
            }
        }
    }
    let fold = |value: &str| -> String {
        value
            .chars()
            .map(|ch| {
                if ch.is_ascii_uppercase() {
                    (ch as u8 + 32) as char
                } else {
                    ch
                }
            })
            .collect()
    };
    let units: Vec<char> = text.chars().collect();
    let mut cursor = if text.starts_with('\\') { 1 } else { 0 };
    while cursor < units.len() {
        let separator = units[cursor..]
            .iter()
            .position(|unit| *unit == '\\')
            .map(|offset| cursor + offset);
        let Some(separator) = separator else {
            return String::new();
        };
        let next = units[separator + 1..]
            .iter()
            .position(|unit| *unit == '\\')
            .map(|offset| separator + 1 + offset);
        let value_end = next.unwrap_or(units.len());
        let name: String = units[cursor..separator].iter().collect();
        if fold(&name) == fold(&key) {
            return units[separator + 1..value_end].iter().collect();
        }
        match next {
            Some(position) => cursor = position + 1,
            None => return String::new(),
        }
    }
    String::new()
}

/// Rank text with source color codes (`placeString`).
#[must_use]
pub fn place_string(rank: i32) -> String {
    let tied = (rank & 0x4000) != 0;
    let rank = rank & !0x4000;
    let place = if rank == 1 {
        "^41st^7".to_string()
    } else if rank == 2 {
        "^12nd^7".to_string()
    } else if rank == 3 {
        "^33rd^7".to_string()
    } else {
        let suffix = if rank == 11 || rank == 12 || rank == 13 {
            "th"
        } else if rank % 10 == 1 {
            "st"
        } else if rank % 10 == 2 {
            "nd"
        } else if rank % 10 == 3 {
            "rd"
        } else {
            "th"
        };
        game_format(&format!("%i{suffix}"), &[GameFormatArg::Int(rank)], 1024)
    };
    game_format(
        "%s%s",
        &[
            GameFormatArg::Text(if tied { "Tied for ".to_string() } else { String::new() }),
            GameFormatArg::Text(place),
        ],
        64,
    )
}

/// Instance-owned `bg_lib` rand/srand and game random (`GameRandom`).
#[derive(Debug, Clone)]
pub struct GameRandom {
    /// Current seed.
    seed: i32,
}

impl GameRandom {
    /// Create a generator at a seed.
    pub fn new(seed: i32) -> Self {
        Self { seed: 0 }.with_seed(seed)
    }

    /// Reset the seed.
    pub fn reset(&mut self, seed: i32) {
        self.seed = seed;
    }

    /// Current seed.
    #[must_use]
    pub fn seed(&self) -> i32 {
        self.seed
    }

    /// One `rand()` step (`qRand` masked to 15 bits).
    pub fn rand(&mut self) -> i32 {
        self.seed = q_rand(self.seed);
        self.seed & 0x7fff
    }

    /// One `random()` fraction.
    pub fn random(&mut self) -> f32 {
        self.rand() as f32 / (0x7fff as f32)
    }

    /// One `crandom()` fraction.
    pub fn crandom(&mut self) -> f32 {
        2.0 * (self.random() - 0.5)
    }

    /// Builder-style seed.
    fn with_seed(mut self, seed: i32) -> Self {
        self.reset(seed);
        self
    }
}

/// QVM angle vectors with binary32 steps (`qvmAngleVectors`).
#[must_use]
pub fn qvm_angle_vectors(angles: Vec3) -> qa_core::math::AngleVectors {
    const RADIANS: f32 = std::f32::consts::PI * 2.0 / 360.0;
    let yaw = angles.y * RADIANS;
    let pitch = angles.x * RADIANS;
    let roll = angles.z * RADIANS;
    let sy = yaw.sin();
    let cy = yaw.cos();
    let sp = pitch.sin();
    let cp = pitch.cos();
    let sr = roll.sin();
    let cr = roll.cos();
    qa_core::math::AngleVectors {
        forward: vec3(cp * cy, cp * sy, -sp),
        right: vec3(
            ((-sr * sp) * cy) + (-cr * -sy),
            ((-sr * sp) * sy) + (-cr * cy),
            -sr * cp,
        ),
        up: vec3(((cr * sp) * cy) + (-sr * -sy), ((cr * sp) * sy) + (-sr * cy), cr * cp),
    }
}

/// QVM angles-to-axis with a negated right column (`qvmAnglesToAxis`).
#[must_use]
pub fn qvm_angles_to_axis(angles: Vec3) -> Axis {
    let vectors = qvm_angle_vectors(angles);
    [
        vectors.forward,
        vec3(-vectors.right.x, -vectors.right.y, -vectors.right.z),
        vectors.up,
    ]
}

/// Left text alignment (`UI_LEFT`).
pub const UI_LEFT: i32 = 0;

/// Centered text (`UI_CENTER`).
pub const UI_CENTER: i32 = 1;

/// Right-aligned text (`UI_RIGHT`).
pub const UI_RIGHT: i32 = 2;

/// Small font style (`UI_SMALLFONT`).
pub const UI_SMALLFONT: i32 = 0x10;

/// Giant font style (`UI_GIANTFONT`).
pub const UI_GIANTFONT: i32 = 0x40;

/// Drop shadow style (`UI_DROPSHADOW`).
pub const UI_DROPSHADOW: i32 = 0x800;

/// Blink style (`UI_BLINK`).
pub const UI_BLINK: i32 = 0x1000;

/// Inverse style (`UI_INVERSE`).
pub const UI_INVERSE: i32 = 0x2000;

/// Pulse style (`UI_PULSE`).
pub const UI_PULSE: i32 = 0x4000;

/// Source color escape table (`COLORS`).
pub(crate) const ESCAPE_COLORS: [Vec4; 8] = [
    Vec4 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        w: 1.0,
    },
    Vec4 {
        x: 1.0,
        y: 0.0,
        z: 0.0,
        w: 1.0,
    },
    Vec4 {
        x: 0.0,
        y: 1.0,
        z: 0.0,
        w: 1.0,
    },
    Vec4 {
        x: 1.0,
        y: 1.0,
        z: 0.0,
        w: 1.0,
    },
    Vec4 {
        x: 0.0,
        y: 0.0,
        z: 1.0,
        w: 1.0,
    },
    Vec4 {
        x: 0.0,
        y: 1.0,
        z: 1.0,
        w: 1.0,
    },
    Vec4 {
        x: 1.0,
        y: 0.0,
        z: 1.0,
        w: 1.0,
    },
    Vec4 {
        x: 1.0,
        y: 1.0,
        z: 1.0,
        w: 1.0,
    },
];

/// Validate 8-bit source text cut at NUL (`byteText`).
pub fn byte_text(text: &str) -> String {
    let cut = match text.find('\0') {
        Some(end) => &text[..end],
        None => text,
    };
    for ch in cut.chars() {
        if ch as u32 > 255 {
            panic!("Quake text requires an 8-bit source string");
        }
    }
    cut.to_string()
}

/// Whether a `^` escape starts at an index (`escapeAt`).
pub(crate) fn escape_at(units: &[char], index: usize) -> bool {
    units.get(index) == Some(&'^') && index + 1 < units.len() && units[index + 1] != '^'
}

/// Resolve an escape color (`escapeColor`).
pub(crate) fn escape_color(code: u32, alpha: f32) -> Vec4 {
    let color = ESCAPE_COLORS[((code.wrapping_sub(48)) & 7) as usize];
    Vec4 { w: alpha, ..color }
}

/// Black with an alpha (`black`).
pub(crate) fn black(alpha: f32) -> Vec4 {
    vec4(0.0, 0.0, 0.0, alpha)
}

/// Glyph metrics (`GlyphMetrics`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GlyphMetrics {
    /// Height.
    pub height: i32,
    /// Top.
    pub top: i32,
    /// Bottom.
    pub bottom: i32,
    /// Pitch.
    pub pitch: i32,
    /// Horizontal advance.
    pub x_skip: i32,
    /// Image width.
    pub image_width: i32,
    /// Image height.
    pub image_height: i32,
    /// S.
    pub s: f32,
    /// T.
    pub t: f32,
    /// S end.
    pub s2: f32,
    /// T end.
    pub t2: f32,
    /// Shader name.
    pub shader_name: String,
}

/// Registered glyph with its picture (`RegisteredGlyph`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RegisteredGlyph {
    /// Metrics.
    pub metrics: GlyphMetrics,
    /// Picture, when registered.
    pub picture: Option<Picture>,
}

/// Registered font (`RegisteredFont`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RegisteredFont {
    /// Name.
    pub name: String,
    /// Glyph scale.
    pub glyph_scale: f32,
    /// Glyphs.
    pub glyphs: Vec<RegisteredGlyph>,
}

/// Zero font with 256 empty glyphs (mission-hud `zeroFont`).
#[must_use]
pub fn zero_font() -> RegisteredFont {
    RegisteredFont {
        name: String::new(),
        glyph_scale: 0.0,
        glyphs: vec![RegisteredGlyph::default(); 256],
    }
}

/// Font profile (`"ui" | "cgame"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FontProfile {
    /// UI menus.
    #[default]
    Ui,
    /// Cgame HUD.
    Cgame,
}

/// Scalable font set (`FontSet`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FontSet {
    /// Small font.
    pub small: RegisteredFont,
    /// Normal font.
    pub normal: RegisteredFont,
    /// Big font.
    pub big: RegisteredFont,
    /// Profile.
    pub profile: FontProfile,
    /// Small threshold.
    pub small_threshold: f32,
    /// Big threshold.
    pub big_threshold: f32,
}

/// Zero cgame font set.
#[must_use]
pub fn zero_cgame_fonts() -> FontSet {
    FontSet {
        small: zero_font(),
        normal: zero_font(),
        big: zero_font(),
        profile: FontProfile::Cgame,
        small_threshold: 0.0,
        big_threshold: 0.0,
    }
}

/// Legacy fixed/atlas fonts (`LegacyFonts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyFonts {
    /// Charset picture.
    pub charset: Picture,
    /// Proportional picture.
    pub proportional: Picture,
    /// Glow picture.
    pub glow: Picture,
    /// Banner picture.
    pub banner: Picture,
}

/// Fixed text options (`FixedTextOptions`).
#[derive(Debug, Clone, PartialEq)]
pub struct FixedTextOptions {
    /// X.
    pub x: f32,
    /// Y.
    pub y: f32,
    /// Text.
    pub text: String,
    /// Color.
    pub color: Vec4,
    /// Character width.
    pub char_width: i32,
    /// Character height.
    pub char_height: i32,
    /// Maximum characters (`<= 0` means unbounded).
    pub max_chars: i32,
    /// Force the color through escapes.
    pub force_color: bool,
    /// Shadow pass.
    pub shadow: bool,
}

/// Proportional/banner text options (`UiTextOptions`).
#[derive(Debug, Clone, PartialEq)]
pub struct UiTextOptions {
    /// X.
    pub x: f32,
    /// Y.
    pub y: f32,
    /// Text.
    pub text: String,
    /// Color.
    pub color: Vec4,
    /// Style bits.
    pub style: i32,
    /// Time for pulse.
    pub time: i32,
}

/// Scalable text paint options (`TextPaintOptions`).
#[derive(Debug, Clone, PartialEq)]
pub struct TextPaintOptions {
    /// X.
    pub x: f32,
    /// Y.
    pub y: f32,
    /// Scale.
    pub scale: f32,
    /// Color.
    pub color: Vec4,
    /// Text.
    pub text: String,
    /// Extra advance.
    pub adjust: f32,
    /// Character limit (`<= 0` means unbounded).
    pub limit: i32,
    /// Style.
    pub style: i32,
}

/// Atlas glyph metric: x, y, advance (`AtlasMetric`).
pub type AtlasMetric = [i32; 3];

/// Invalid metric.
pub(crate) const INVALID_METRIC: AtlasMetric = [0, 0, -1];

/// Proportional ASCII atlas (`PROP_ASCII`).
pub(crate) const PROP_ASCII: [AtlasMetric; 65] = [
    [0, 0, 8],
    [11, 122, 7],
    [154, 181, 14],
    [55, 122, 17],
    [79, 122, 18],
    [101, 122, 23],
    [153, 122, 18],
    [9, 93, 7],
    [207, 122, 8],
    [230, 122, 9],
    [177, 122, 18],
    [30, 152, 18],
    [85, 181, 7],
    [34, 93, 11],
    [110, 181, 6],
    [130, 152, 14],
    [22, 64, 17],
    [41, 64, 12],
    [58, 64, 17],
    [78, 64, 18],
    [98, 64, 19],
    [120, 64, 18],
    [141, 64, 18],
    [204, 64, 16],
    [162, 64, 17],
    [182, 64, 18],
    [59, 181, 7],
    [35, 181, 7],
    [203, 152, 14],
    [56, 93, 14],
    [228, 152, 14],
    [177, 181, 18],
    [28, 122, 22],
    [5, 4, 18],
    [27, 4, 18],
    [48, 4, 18],
    [69, 4, 17],
    [90, 4, 13],
    [106, 4, 13],
    [121, 4, 18],
    [143, 4, 17],
    [164, 4, 8],
    [175, 4, 16],
    [195, 4, 18],
    [216, 4, 12],
    [230, 4, 23],
    [6, 34, 18],
    [27, 34, 18],
    [48, 34, 18],
    [68, 34, 18],
    [90, 34, 17],
    [110, 34, 18],
    [130, 34, 14],
    [146, 34, 18],
    [166, 34, 19],
    [185, 34, 29],
    [215, 34, 18],
    [234, 34, 18],
    [5, 64, 14],
    [60, 152, 7],
    [106, 151, 13],
    [83, 152, 7],
    [128, 122, 17],
    [4, 152, 21],
    [134, 181, 5],
];

/// Proportional atlas tail (`PROP_END`).
pub(crate) const PROP_END: [AtlasMetric; 4] = [[153, 152, 13], [11, 181, 5], [180, 152, 13], [79, 93, 17]];

/// Banner atlas (`BANNER`).
pub(crate) const BANNER: [AtlasMetric; 26] = [
    [11, 12, 33],
    [49, 12, 31],
    [85, 12, 31],
    [120, 12, 30],
    [156, 12, 21],
    [183, 12, 21],
    [207, 12, 32],
    [13, 55, 30],
    [49, 55, 13],
    [66, 55, 29],
    [101, 55, 31],
    [135, 55, 21],
    [158, 55, 40],
    [204, 55, 32],
    [12, 97, 31],
    [48, 97, 31],
    [82, 97, 30],
    [118, 97, 30],
    [153, 97, 30],
    [185, 97, 25],
    [213, 97, 30],
    [11, 139, 32],
    [42, 139, 51],
    [93, 139, 32],
    [126, 139, 31],
    [158, 139, 25],
];

/// Proportional glyph metric (`propMetric`).
#[must_use]
pub fn prop_metric(code: u32) -> AtlasMetric {
    let mut ch = code & 127;
    if ch < 32 || ch == 127 {
        return INVALID_METRIC;
    }
    if (97..=122).contains(&ch) {
        ch -= 32;
    }
    if ch >= 123 {
        PROP_END[(ch - 123) as usize]
    } else {
        PROP_ASCII[(ch - 32) as usize]
    }
}

/// Banner glyph metric (`bannerMetric`).
pub(crate) fn banner_metric(code: u32) -> AtlasMetric {
    if !(65..=90).contains(&code) {
        panic!("Invalid banner glyph index");
    }
    BANNER[(code - 65) as usize]
}

/// Proportional string width (`proportionalStringWidth`).
#[must_use]
pub fn proportional_string_width(input: &str) -> i32 {
    let text = byte_text(input);
    let mut width: i32 = 0;
    for ch in text.chars() {
        let metric = prop_metric(ch as u32);
        if metric[2] != -1 {
            width += metric[2] + 3;
        }
    }
    width - 3
}

/// Banner string width (`bannerStringWidth`).
#[must_use]
pub fn banner_string_width(input: &str) -> i32 {
    let text = byte_text(input);
    let mut width: i32 = 0;
    for ch in text.chars() {
        let code = ch as u32;
        if code == 32 {
            width += 12;
        } else if (65..=90).contains(&code) {
            width += banner_metric(code)[2] + 4;
        }
    }
    width - 4
}

/// Aligned X for a style (`alignedX`).
pub(crate) fn aligned_x(x: f32, width: i32, style: i32) -> i32 {
    x.trunc() as i32
        - if style & 7 == UI_CENTER {
            (width as f32 / 2.0).trunc() as i32
        } else if style & 7 == UI_RIGHT {
            width
        } else {
            0
        }
}

/// Pulse alpha (`pulse`).
pub(crate) fn pulse(time: i32) -> f32 {
    0.5 + 0.5 * ((time / 75) as f32).sin()
}

/// Select a font by scale (`selectFont`).
pub(crate) fn select_font(fonts: &FontSet, scale: f32) -> &RegisteredFont {
    if !scale.is_finite() {
        panic!("Non-finite text scale");
    }
    if scale <= fonts.small_threshold {
        &fonts.small
    } else if (fonts.profile == FontProfile::Ui && scale >= fonts.big_threshold)
        || (fonts.profile == FontProfile::Cgame && scale > fonts.big_threshold)
    {
        &fonts.big
    } else {
        &fonts.normal
    }
}

/// Fetch a glyph (`glyphAt`).
pub(crate) fn glyph_at(font: &RegisteredFont, code: u32) -> &RegisteredGlyph {
    font.glyphs
        .get(code as usize)
        .unwrap_or_else(|| panic!("Missing font glyph {code}"))
}

/// Text metric core (`textMetric`).
pub(crate) fn text_metric(fonts: &FontSet, input: &str, scale: f32, limit: i32, height: bool) -> i32 {
    let text = byte_text(input);
    let font = select_font(fonts, scale);
    let use_scale = scale * font.glyph_scale;
    let units: Vec<char> = text.chars().collect();
    let maximum = if limit > 0 {
        units.len().min(limit as usize)
    } else {
        units.len()
    };
    let mut value: f32 = 0.0;
    let mut count = 0usize;
    let mut index = 0usize;
    while index < units.len() && count < maximum {
        if escape_at(&units, index) {
            index += 2;
            continue;
        }
        let glyph = glyph_at(font, units[index] as u32);
        if height {
            value = value.max(glyph.metrics.height as f32);
        } else {
            value += glyph.metrics.x_skip as f32;
        }
        count += 1;
        index += 1;
    }
    (value * use_scale).trunc() as i32
}

/// Text width (`textWidth`).
#[must_use]
pub fn text_width(fonts: &FontSet, text: &str, scale: f32, limit: i32) -> i32 {
    text_metric(fonts, text, scale, limit, false)
}

/// Text height (`textHeight`).
#[must_use]
pub fn text_height(fonts: &FontSet, text: &str, scale: f32, limit: i32) -> i32 {
    text_metric(fonts, text, scale, limit, true)
}

/// Drawing coordinate space (`CoordinateSpace`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CoordinateSpace {
    /// Device pixels.
    Pixels,
    /// Cgame 640x480 stretch.
    #[default]
    Stretch640,
    /// Base UI 640.
    BaseUi640,
    /// Team UI 640.
    TeamUi640,
}

impl CoordinateSpace {
    /// Donor spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pixels => "pixels",
            Self::Stretch640 => "stretch-640",
            Self::BaseUi640 => "base-ui-640",
            Self::TeamUi640 => "team-ui-640",
        }
    }
}

/// Engine pixel command queue (`TextDrawSink` command surface).
pub trait HudDrawSink {
    /// Set the current color (None clears).
    fn set_color(&mut self, color: Option<Vec4>);
    /// Stretch/is blit pixels.
    fn stretch_pixels(&mut self, rect: Rect2d, uv: TextureRect, picture: Picture);
}

/// 2D drawing context (`Draw2D`) over a shared engine queue.
#[derive(Clone)]
pub struct Draw2D {
    /// Shared engine queue; identity is queue identity.
    pub sink: Shared<dyn HudDrawSink>,
    /// Coordinate space.
    pub space: CoordinateSpace,
    /// Target width.
    pub target_width: i32,
    /// Target height.
    pub target_height: i32,
}

impl Draw2D {
    /// Wrap a queue.
    pub fn new(sink: Shared<dyn HudDrawSink>, space: CoordinateSpace, target_width: i32, target_height: i32) -> Self {
        Self {
            sink,
            space,
            target_width,
            target_height,
        }
    }

    /// Whether two contexts share one engine queue.
    #[must_use]
    pub fn shares_queue(&self, other: &Draw2D) -> bool {
        Rc::ptr_eq(&self.sink, &other.sink)
    }

    /// Target width (`width`).
    #[must_use]
    pub fn width(&self) -> i32 {
        self.target_width
    }

    /// Target height (`height`).
    #[must_use]
    pub fn height(&self) -> i32 {
        self.target_height
    }

    /// Horizontal scale (`scaleX`).
    #[must_use]
    pub fn scale_x(&self) -> f32 {
        match self.space {
            CoordinateSpace::Pixels => 1.0,
            CoordinateSpace::Stretch640 => self.width() as f32 / 640.0,
            CoordinateSpace::BaseUi640 => self.height() as f32 * (1.0 / 480.0),
            CoordinateSpace::TeamUi640 => self.width() as f32 * (1.0 / 640.0),
        }
    }

    /// Vertical scale (`scaleY`).
    #[must_use]
    pub fn scale_y(&self) -> f32 {
        match self.space {
            CoordinateSpace::Pixels => 1.0,
            CoordinateSpace::Stretch640 => self.height() as f32 / 480.0,
            CoordinateSpace::BaseUi640 => self.scale_x(),
            CoordinateSpace::TeamUi640 => self.height() as f32 * (1.0 / 480.0),
        }
    }

    /// Horizontal bias (`biasX`).
    #[must_use]
    pub fn bias_x(&self) -> f32 {
        if self.space == CoordinateSpace::BaseUi640 && self.width().wrapping_mul(480) > self.height().wrapping_mul(640)
        {
            0.5 * (self.width() as f32 - self.height() as f32 * (640.0 / 480.0))
        } else {
            0.0
        }
    }

    /// Set the current color (`setColor`).
    pub fn set_color(&self, color: Option<Vec4>) {
        self.sink.borrow_mut().set_color(color);
    }

    /// Adjust to device pixels (`adjust`).
    #[must_use]
    pub fn adjust(&self, rect: Rect2d) -> Rect2d {
        let x = rect.x * self.scale_x();
        Rect2d {
            x: if self.space == CoordinateSpace::TeamUi640 {
                x
            } else {
                x + self.bias_x()
            },
            y: rect.y * self.scale_y(),
            width: rect.width * self.scale_x(),
            height: rect.height * self.scale_y(),
        }
    }

    /// Stretch a 640-space rect (`stretchPic`).
    pub fn stretch_pic(&self, rect: Rect2d, uv: TextureRect, picture: Picture) {
        let adjusted = self.adjust(rect);
        self.stretch_pixels(adjusted, uv, picture);
    }

    /// Stretch device pixels (`stretchPixels`).
    pub fn stretch_pixels(&self, rect: Rect2d, uv: TextureRect, picture: Picture) {
        self.sink.borrow_mut().stretch_pixels(rect, uv, picture);
    }

    /// Draw a full-UV picture (`drawPic`).
    pub fn draw_pic(&self, rect: Rect2d, picture: Picture) {
        self.stretch_pic(rect, FULL_UV, picture);
    }
}

/// Draw one charset glyph (`drawChar`).
pub fn draw_char(draw: &Draw2D, charset: Picture, x: f32, y: f32, width: f32, height: f32, code: u32) {
    let ch = code & 255;
    if ch == 32 {
        return;
    }
    let s = f64::from(ch & 15) / 16.0;
    let t = f64::from(ch >> 4) / 16.0;
    draw.stretch_pic(
        rect2d(x, y, width, height),
        TextureRect {
            s: s as f32,
            t: t as f32,
            s2: (s + 1.0 / 16.0) as f32,
            t2: (t + 1.0 / 16.0) as f32,
        },
        charset,
    );
}

/// Draw a fixed cgame string (`drawCgString`).
pub fn draw_cg_string(draw: &Draw2D, charset: Picture, options: &FixedTextOptions) {
    let text = byte_text(&options.text);
    let units: Vec<char> = text.chars().collect();
    let maximum = if options.max_chars <= 0 {
        32767usize
    } else {
        options.max_chars as usize
    };
    for shadow in [true, false] {
        if shadow && !options.shadow {
            continue;
        }
        let mut x = options.x.trunc() as i32;
        let mut count = 0usize;
        draw.set_color(Some(if shadow { black(options.color.w) } else { options.color }));
        let mut index = 0usize;
        while index < units.len() && count < maximum {
            if escape_at(&units, index) {
                if !shadow && !options.force_color {
                    draw.set_color(Some(escape_color(units[index + 1] as u32, options.color.w)));
                }
                index += 2;
                continue;
            }
            draw_char(
                draw,
                charset,
                (x + if shadow { 2 } else { 0 }) as f32,
                options.y.trunc() + if shadow { 2.0 } else { 0.0 },
                options.char_width as f32,
                options.char_height as f32,
                units[index] as u32,
            );
            x += options.char_width;
            count += 1;
            index += 1;
        }
    }
    draw.set_color(None);
}

/// One atlas text pass (`atlasPass`, cgame profile).
#[allow(clippy::too_many_arguments)]
pub(crate) fn atlas_pass_cgame(
    draw: &Draw2D,
    picture: Picture,
    text: &str,
    x: i32,
    y: i32,
    color: Vec4,
    size: f32,
    banner: bool,
) {
    draw.set_color(Some(color));
    let position = draw.adjust(rect2d(x as f32, y as f32, 0.0, 0.0));
    let vertical_scale = draw.scale_x();
    let top = y as f32 * draw.scale_x();
    let mut ax = position.x;
    let mut aw: f32;
    let gap = (if banner { 4.0 } else { 3.0 } * draw.scale_x()) * size;
    let height = (if banner { 36.0 } else { 27.0 } * vertical_scale) * size;
    let units: Vec<char> = text.chars().collect();
    for unit in units {
        let code = unit as u32 & 127;
        if banner && code != 32 && !(65..=90).contains(&code) {
            continue;
        }
        let metric = if banner && code != 32 {
            banner_metric(code)
        } else {
            prop_metric(code)
        };
        if code == 32 {
            aw = (if banner { 12.0 } else { 8.0 } * draw.scale_x()) * size;
        } else if metric[2] != -1 {
            aw = (metric[2] as f32 * draw.scale_x()) * size;
            draw.stretch_pixels(
                rect2d(ax, top, aw, height),
                TextureRect {
                    s: metric[0] as f32 / 256.0,
                    t: metric[1] as f32 / 256.0,
                    s2: (metric[0] + metric[2]) as f32 / 256.0,
                    t2: (metric[1] + if banner { 36 } else { 27 }) as f32 / 256.0,
                },
                picture,
            );
        } else {
            aw = 0.0;
        }
        ax += aw + gap;
    }
    draw.set_color(None);
}

/// Draw a proportional cgame string (`drawCgProportionalString`).
pub fn draw_cg_proportional_string(draw: &Draw2D, fonts: &LegacyFonts, options: &UiTextOptions) {
    let text = byte_text(&options.text);
    let size = if options.style & UI_SMALLFONT != 0 { 0.75 } else { 1.0 };
    let x = aligned_x(
        options.x,
        (proportional_string_width(&text) as f32 * size).trunc() as i32,
        options.style,
    );
    let y = options.y.trunc() as i32;
    if options.style & UI_DROPSHADOW != 0 {
        atlas_pass_cgame(
            draw,
            fonts.proportional,
            &text,
            x + 2,
            y + 2,
            black(options.color.w),
            size,
            false,
        );
    }
    if options.style & UI_INVERSE != 0 {
        let inverse = 0.8f32;
        atlas_pass_cgame(
            draw,
            fonts.proportional,
            &text,
            x,
            y,
            vec4(
                options.color.x * inverse,
                options.color.y * inverse,
                options.color.z * inverse,
                options.color.w,
            ),
            size,
            false,
        );
        return;
    }
    atlas_pass_cgame(draw, fonts.proportional, &text, x, y, options.color, size, false);
    if options.style & UI_PULSE != 0 {
        atlas_pass_cgame(
            draw,
            fonts.glow,
            &text,
            x,
            y,
            Vec4 {
                w: pulse(options.time),
                ..options.color
            },
            size,
            false,
        );
    }
}

/// Draw a banner cgame string (`drawCgBannerString`).
pub fn draw_cg_banner_string(draw: &Draw2D, fonts: &LegacyFonts, options: &UiTextOptions) {
    let text = byte_text(&options.text);
    let x = aligned_x(options.x, banner_string_width(&text), options.style);
    let y = options.y.trunc() as i32;
    if options.style & UI_DROPSHADOW != 0 {
        atlas_pass_cgame(
            draw,
            fonts.banner,
            &text,
            x + 2,
            y + 2,
            black(options.color.w),
            1.0,
            true,
        );
    }
    atlas_pass_cgame(draw, fonts.banner, &text, x, y, options.color, 1.0, true);
}

/// Paint one scalable glyph (`paintGlyph`).
pub(crate) fn paint_glyph(draw: &Draw2D, glyph: &RegisteredGlyph, x: f32, baseline: f32, scale: f32) {
    if let Some(picture) = glyph.picture {
        draw.stretch_pic(
            rect2d(
                x,
                baseline - scale * glyph.metrics.top as f32,
                glyph.metrics.image_width as f32 * scale,
                glyph.metrics.image_height as f32 * scale,
            ),
            TextureRect {
                s: glyph.metrics.s,
                t: glyph.metrics.t,
                s2: glyph.metrics.s2,
                t2: glyph.metrics.t2,
            },
            picture,
        );
    }
}

/// Paint scalable text (`textPaint`).
pub fn text_paint(draw: &Draw2D, fonts: &FontSet, options: &TextPaintOptions) {
    let text = byte_text(&options.text);
    let font = select_font(fonts, options.scale);
    let scale = options.scale * font.glyph_scale;
    let units: Vec<char> = text.chars().collect();
    let maximum = if options.limit > 0 {
        units.len().min(options.limit as usize)
    } else {
        units.len()
    };
    let mut x = options.x;
    let mut color = options.color;
    let mut count = 0usize;
    let mut index = 0usize;
    draw.set_color(Some(color));
    while index < units.len() && count < maximum {
        if escape_at(&units, index) {
            color = escape_color(units[index + 1] as u32, options.color.w);
            draw.set_color(Some(color));
            index += 2;
            continue;
        }
        let glyph = glyph_at(font, units[index] as u32);
        let baseline = options.y;
        if options.style == 3 || options.style == 6 {
            let offset = if options.style == 3 { 1.0 } else { 2.0 };
            if let Some(picture) = glyph.picture {
                draw.set_color(Some(black(color.w)));
                draw.stretch_pic(
                    rect2d(
                        x + offset,
                        baseline - scale * glyph.metrics.top as f32 + offset,
                        glyph.metrics.image_width as f32 * scale,
                        glyph.metrics.image_height as f32 * scale,
                    ),
                    TextureRect {
                        s: glyph.metrics.s,
                        t: glyph.metrics.t,
                        s2: glyph.metrics.s2,
                        t2: glyph.metrics.t2,
                    },
                    picture,
                );
                draw.set_color(Some(color));
            }
        }
        paint_glyph(draw, glyph, x, baseline, scale);
        x += glyph.metrics.x_skip as f32 * scale + options.adjust;
        count += 1;
        index += 1;
    }
    draw.set_color(None);
}

/// Paint scalable text clamped to `max_x` (`textPaintLimit`).
pub fn text_paint_limit(draw: &Draw2D, fonts: &FontSet, options: &TextPaintOptions, max_x: f32) -> f32 {
    let text = byte_text(&options.text);
    let cgame_fonts = FontSet {
        profile: FontProfile::Cgame,
        small: fonts.small.clone(),
        normal: fonts.normal.clone(),
        big: fonts.big.clone(),
        small_threshold: fonts.small_threshold,
        big_threshold: fonts.big_threshold,
    };
    let font = select_font(&cgame_fonts, options.scale);
    let scale = options.scale * font.glyph_scale;
    let units: Vec<char> = text.chars().collect();
    let maximum = if options.limit > 0 {
        units.len().min(options.limit as usize)
    } else {
        units.len()
    };
    let mut x = options.x;
    let mut result = max_x;
    let mut count = 0usize;
    let mut index = 0usize;
    draw.set_color(Some(options.color));
    while index < units.len() && count < maximum {
        if escape_at(&units, index) {
            draw.set_color(Some(escape_color(units[index + 1] as u32, options.color.w)));
            index += 2;
            continue;
        }
        let rest: String = units[index..].iter().collect();
        if text_width(fonts, &rest, scale, 1) as f32 + x > max_x {
            result = 0.0;
            break;
        }
        let glyph = glyph_at(font, units[index] as u32);
        paint_glyph(draw, glyph, x, options.y, scale);
        x += glyph.metrics.x_skip as f32 * scale + options.adjust;
        result = x;
        count += 1;
        index += 1;
    }
    draw.set_color(None);
    result
}

/// No-shadow render flag (`RF_NOSHADOW`).
pub const RF_NOSHADOW: i32 = 64;

/// Lighting-origin render flag (`RF_LIGHTING_ORIGIN`).
pub const RF_LIGHTING_ORIGIN: i32 = 128;

/// No world model flag (`RDF_NOWORLDMODEL`).
pub const RDF_NOWORLDMODEL: i32 = 1;

/// Scene model (`SceneModel`).
#[derive(Debug, Clone, PartialEq, Default)]
pub enum SceneModel {
    /// Default placeholder.
    #[default]
    Default,
    /// Loaded model.
    Loaded {
        /// Path.
        path: String,
    },
    /// Inline model.
    Inline {
        /// Path.
        path: String,
        /// Index.
        index: i32,
    },
}

/// Default model (`DEFAULT_MODEL`).
pub const DEFAULT_MODEL: SceneModel = SceneModel::Default;

impl SceneModel {
    /// Whether this is the default placeholder.
    #[must_use]
    pub fn is_default(&self) -> bool {
        matches!(self, Self::Default)
    }

    /// Model path.
    #[must_use]
    pub fn path(&self) -> &str {
        match self {
            Self::Default => "*default",
            Self::Loaded { path } | Self::Inline { path, .. } => path,
        }
    }
}

/// Scene shader handle (`SceneShader`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SceneShader {
    /// Name.
    pub name: String,
}

/// Scene skin handle (`SceneSkin`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SceneSkin {
    /// Path.
    pub path: String,
}

/// Model render entity (`RefModelEntity`, used surface).
#[derive(Debug, Clone, PartialEq)]
pub struct RefModelEntity {
    /// Model.
    pub model: SceneModel,
    /// Origin.
    pub origin: Vec3,
    /// Previous origin.
    pub old_origin: Vec3,
    /// Axes.
    pub axis: Axis,
    /// Lighting origin.
    pub lighting_origin: Vec3,
    /// Render flags.
    pub render_flags: i32,
    /// Custom skin.
    pub custom_skin: Option<SceneSkin>,
}

/// Build a model entity (`createModelEntity`).
#[must_use]
pub fn create_model_entity(model: SceneModel) -> RefModelEntity {
    RefModelEntity {
        model,
        origin: vec3(0.0, 0.0, 0.0),
        old_origin: vec3(0.0, 0.0, 0.0),
        axis: [vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0)],
        lighting_origin: vec3(0.0, 0.0, 0.0),
        render_flags: 0,
        custom_skin: None,
    }
}

/// Reference definition (`Refdef`, used surface).
#[derive(Debug, Clone, PartialEq)]
pub struct Refdef {
    /// X.
    pub x: i32,
    /// Y.
    pub y: i32,
    /// Width.
    pub width: i32,
    /// Height.
    pub height: i32,
    /// Horizontal FOV.
    pub fov_x: f32,
    /// Vertical FOV.
    pub fov_y: f32,
    /// View origin.
    pub view_origin: Vec3,
    /// View axes.
    pub view_axis: Axis,
    /// Time.
    pub time: i32,
    /// Render flags.
    pub render_flags: i32,
}

/// Build a blank refdef (`createRefdef`).
#[must_use]
pub fn create_refdef() -> Refdef {
    Refdef {
        x: 0,
        y: 0,
        width: 0,
        height: 0,
        fov_x: 0.0,
        fov_y: 0.0,
        view_origin: vec3(0.0, 0.0, 0.0),
        view_axis: [vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0)],
        time: 0,
        render_flags: 0,
    }
}

/// Decoded sound data (`PcmSound` payload; the handle is shared for identity).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PcmSoundData {
    /// Debug name.
    pub name: String,
}

/// Shared decoded sound (`PcmSound`); identity is handle identity.
pub type PcmSound = Shared<PcmSoundData>;

/// Build a sound handle.
#[must_use]
pub fn pcm_sound(name: &str) -> PcmSound {
    shared(PcmSoundData { name: name.to_string() })
}

/// Registered sound asset (`SoundAsset`).
#[derive(Debug, Clone, PartialEq)]
pub struct SoundAsset {
    /// Decoded PCM.
    pub pcm: PcmSound,
    /// Resource binding.
    pub resource: Option<String>,
}

/// Integer player-state slots (`PlayerStateSlots`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotArray {
    /// Values.
    pub values: Vec<i32>,
}

impl SlotArray {
    /// Zeroed slots.
    #[must_use]
    pub fn zeros(length: usize) -> Self {
        Self {
            values: vec![0; length],
        }
    }

    /// Slot count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Whether no slots are allocated.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Read a slot (`get`).
    #[must_use]
    pub fn get(&self, index: i32) -> i32 {
        if index < 0 {
            panic!("Player state slot {index} outside {}", self.values.len());
        }
        *self
            .values
            .get(index as usize)
            .unwrap_or_else(|| panic!("Player state slot {index} outside {}", self.values.len()))
    }

    /// Write a slot (`set`).
    pub fn set(&mut self, index: i32, value: i32) {
        if index < 0 {
            panic!("Player state slot {index} outside {}", self.values.len());
        }
        let len = self.values.len();
        let slot = self
            .values
            .get_mut(index as usize)
            .unwrap_or_else(|| panic!("Player state slot {index} outside {len}"));
        *slot = value;
    }
}

/// Powerup expiry slots (16).
pub type PowerupSlots = SlotArray;

/// Stat slots (16).
pub type StatSlots = SlotArray;

/// Persistent slots (16).
pub type PersistSlots = SlotArray;

/// Ammo slots (16).
pub type AmmoSlots = SlotArray;

/// Player state (`SourcePlayerState`, used surface).
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerState {
    /// Product.
    pub product: Product,
    /// Client number.
    pub client_num: i32,
    /// Powerup expiries.
    pub powerups: PowerupSlots,
    /// Stats.
    pub stats: StatSlots,
    /// Persistent.
    pub persistant: PersistSlots,
    /// Ammo.
    pub ammo: AmmoSlots,
    /// Movement type.
    pub pm_type: MoveType,
    /// Movement flags.
    pub pm_flags: i32,
    /// Entity flags.
    pub e_flags: i32,
    /// Generic 1.
    pub generic1: i32,
    /// Weapon.
    pub weapon: i32,
    /// Weapon state.
    pub weapon_state: WeaponState,
    /// Weapon time.
    pub weapon_time: i32,
    /// Command time.
    pub command_time: i32,
}

impl PlayerState {
    /// Blank player state.
    #[must_use]
    pub fn new(product: Product) -> Self {
        Self {
            product,
            client_num: 0,
            powerups: SlotArray::zeros(16),
            stats: SlotArray::zeros(16),
            persistant: SlotArray::zeros(16),
            ammo: SlotArray::zeros(16),
            pm_type: MoveType::Normal,
            pm_flags: 0,
            e_flags: 0,
            generic1: 0,
            weapon: 0,
            weapon_state: WeaponState::Ready,
            weapon_time: 0,
            command_time: 0,
        }
    }
}

/// Entity state (`EntityState`, used surface).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EntityState {
    /// Weapon.
    pub weapon: i32,
    /// Powerup bitmask.
    pub powerups: i32,
}

/// Client entity (`ClientEntity`, used surface).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ClientEntity {
    /// Current state.
    pub current_state: EntityState,
}

/// Snapshot (`RetailSnapshot`, used surface).
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    /// Server time.
    pub server_time: i32,
    /// Player state.
    pub player_state: PlayerState,
}

/// Score row (`ClientScore`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ClientScore {
    /// Client.
    pub client: i32,
    /// Score.
    pub score: i32,
    /// Ping.
    pub ping: i32,
    /// Time.
    pub time: i32,
    /// Score flags.
    pub score_flags: i32,
    /// Accuracy.
    pub accuracy: f32,
    /// Impressive count.
    pub impressive_count: i32,
    /// Excellent count.
    pub excellent_count: i32,
    /// Gauntlet count.
    pub guantlet_count: i32,
    /// Defend count.
    pub defend_count: i32,
    /// Assist count.
    pub assist_count: i32,
    /// Perfect.
    pub perfect: i32,
    /// Captures.
    pub captures: i32,
    /// Team.
    pub team: i32,
}

/// Reward row (`ClientReward`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ClientReward {
    /// Sound.
    pub sound: Option<PcmSound>,
    /// Shader.
    pub shader: Option<SceneShader>,
    /// Count.
    pub count: i32,
}

/// Cgame frame state (`ClientGameState`, used surface).
#[derive(Debug, Clone, PartialEq)]
pub struct ClientGameState {
    /// Product.
    pub product: Product,
    /// Entities (1024).
    pub entities: Vec<ClientEntity>,
    /// Score count.
    pub num_scores: i32,
    /// Selected score.
    pub selected_score: i32,
    /// Scores request time.
    pub scores_request_time: i32,
    /// Show scores.
    pub show_scores: bool,
    /// Score fade time.
    pub score_fade_time: i32,
    /// Scoreboard showing.
    pub score_board_showing: bool,
    /// Deferred player loading.
    pub deferred_player_loading: i32,
    /// Center print text.
    pub center_print: String,
    /// Center print time.
    pub center_print_time: i32,
    /// Center print character width.
    pub center_print_char_width: i32,
    /// Center print Y.
    pub center_print_y: i32,
    /// Center print lines.
    pub center_print_lines: i32,
    /// Head start yaw.
    pub head_start_yaw: f32,
    /// Head end yaw.
    pub head_end_yaw: f32,
    /// Head start pitch.
    pub head_start_pitch: f32,
    /// Head end pitch.
    pub head_end_pitch: f32,
    /// Head start time.
    pub head_start_time: i32,
    /// Head end time.
    pub head_end_time: i32,
    /// Voice time.
    pub voice_time: i32,
    /// Crosshair client.
    pub crosshair_client_num: i32,
    /// Crosshair client time.
    pub crosshair_client_time: i32,
    /// Scores (64).
    pub scores: [ClientScore; 64],
    /// Team scores.
    pub team_scores: [i32; 2],
    /// Sorted team player count.
    pub num_sorted_team_players: i32,
    /// Sorted team players (8).
    pub sorted_team_players: [i32; 8],
    /// Warmup count.
    pub warmup_count: i32,
    /// Level shot.
    pub level_shot: bool,
    /// Info screen text.
    pub info_screen_text: String,
    /// Sound buffer input.
    pub sound_buffer_in: i32,
    /// Sound buffer output.
    pub sound_buffer_out: i32,
    /// Sound time.
    pub sound_time: i32,
    /// Sound buffer (20).
    pub sound_buffer: [Option<PcmSound>; 20],
    /// Spectator list.
    pub spectator_list: String,
    /// Spectator length.
    pub spectator_len: i32,
    /// Spectator width.
    pub spectator_width: i32,
    /// Spectator time.
    pub spectator_time: i32,
    /// Spectator offset.
    pub spectator_offset: i32,
    /// Spectator paint X.
    pub spectator_paint_x: i32,
    /// Spectator paint X 2.
    pub spectator_paint_x2: i32,
    /// Latest snapshot number.
    pub latest_snapshot_num: i32,
    /// Latest snapshot time.
    pub latest_snapshot_time: i32,
    /// Current snapshot.
    pub snap: Option<Snapshot>,
    /// Time.
    pub time: i32,
    /// Old time.
    pub old_time: i32,
    /// Predicted player state.
    pub predicted_player_state: PlayerState,
    /// Killer name.
    pub killer_name: String,
    /// Item pickup.
    pub item_pickup: i32,
    /// Item pickup time.
    pub item_pickup_time: i32,
    /// Item pickup blend time.
    pub item_pickup_blend_time: i32,
    /// Active powerup.
    pub powerup_active: i32,
    /// Powerup time.
    pub powerup_time: i32,
    /// Refdef.
    pub refdef: Refdef,
    /// Refdef view angles.
    pub refdef_view_angles: Vec3,
    /// Rendering third person.
    pub rendering_third_person: bool,
    /// Damage time.
    pub damage_time: i32,
    /// Damage X.
    pub damage_x: f32,
    /// Attacker time.
    pub attacker_time: i32,
    /// Low ammo warning.
    pub low_ammo_warning: i32,
    /// Reward stack.
    pub reward_stack: i32,
    /// Reward time.
    pub reward_time: i32,
    /// Rewards (10).
    pub rewards: [ClientReward; 10],
    /// Warmup.
    pub warmup: i32,
}

impl ClientGameState {
    /// Blank frame state.
    #[must_use]
    pub fn new(product: Product) -> Self {
        Self {
            product,
            entities: vec![ClientEntity::default(); 1024],
            num_scores: 0,
            selected_score: 0,
            scores_request_time: 0,
            show_scores: false,
            score_fade_time: 0,
            score_board_showing: false,
            deferred_player_loading: 0,
            center_print: String::new(),
            center_print_time: 0,
            center_print_char_width: 0,
            center_print_y: 0,
            center_print_lines: 0,
            head_start_yaw: 0.0,
            head_end_yaw: 0.0,
            head_start_pitch: 0.0,
            head_end_pitch: 0.0,
            head_start_time: 0,
            head_end_time: 0,
            voice_time: 0,
            crosshair_client_num: 0,
            crosshair_client_time: 0,
            scores: [ClientScore::default(); 64],
            team_scores: [0, 0],
            num_sorted_team_players: 0,
            sorted_team_players: [0; 8],
            warmup_count: 0,
            level_shot: false,
            info_screen_text: String::new(),
            sound_buffer_in: 0,
            sound_buffer_out: 0,
            sound_time: 0,
            sound_buffer: std::array::from_fn(|_| None),
            spectator_list: String::new(),
            spectator_len: 0,
            spectator_width: 0,
            spectator_time: 0,
            spectator_offset: 0,
            spectator_paint_x: 0,
            spectator_paint_x2: 0,
            latest_snapshot_num: 0,
            latest_snapshot_time: 0,
            snap: None,
            time: 0,
            old_time: 0,
            predicted_player_state: PlayerState::new(product),
            killer_name: String::new(),
            item_pickup: 0,
            item_pickup_time: 0,
            item_pickup_blend_time: 0,
            powerup_active: 0,
            powerup_time: 0,
            refdef: create_refdef(),
            refdef_view_angles: vec3(0.0, 0.0, 0.0),
            rendering_third_person: false,
            damage_time: 0,
            damage_x: 0.0,
            attacker_time: 0,
            low_ammo_warning: 0,
            reward_stack: 0,
            reward_time: 0,
            rewards: std::array::from_fn(|_| ClientReward::default()),
            warmup: 0,
        }
    }

    /// Fetch an entity (`entityAt`).
    #[must_use]
    pub fn entity_at(&self, index: i32) -> &ClientEntity {
        self.entities.get(index as usize).unwrap_or_else(|| {
            panic!("Invalid client entity number {index}");
        })
    }

    /// Fetch an entity mutably.
    pub fn entity_at_mut(&mut self, index: i32) -> &mut ClientEntity {
        let length = self.entities.len();
        self.entities.get_mut(index as usize).unwrap_or_else(|| {
            panic!("Invalid client entity number {index} outside {length}");
        })
    }
}

/// Cgame static state (`ClientGameStaticState`, used surface).
#[derive(Debug, Clone, PartialEq)]
pub struct ClientGameStaticState {
    /// Product.
    pub product: Product,
    /// Server command sequence.
    pub server_command_sequence: i32,
    /// Cursor X.
    pub cursor_x: f32,
    /// Cursor Y.
    pub cursor_y: f32,
    /// Event handling.
    pub event_handling: i32,
    /// Active cursor.
    pub active_cursor: Option<SceneShader>,
    /// Team chat messages (8).
    pub team_chat_msgs: [String; 8],
    /// Team chat message times (8).
    pub team_chat_msg_times: [i32; 8],
    /// Team chat position.
    pub team_chat_pos: i32,
    /// Team last chat position.
    pub team_last_chat_pos: i32,
    /// Current voice client.
    pub current_voice_client: i32,
    /// Accept order time.
    pub accept_order_time: i32,
    /// Accept task.
    pub accept_task: i32,
    /// Accept leader.
    pub accept_leader: i32,
    /// Current order.
    pub current_order: i32,
    /// Order pending.
    pub order_pending: bool,
    /// Order time.
    pub order_time: i32,
    /// Client info slots (64).
    pub client_info: Vec<Shared<ClientInfo>>,
    /// Game type.
    pub game_type: GameType,
    /// Frag limit.
    pub fraglimit: i32,
    /// Capture limit.
    pub capturelimit: i32,
    /// Max clients.
    pub maxclients: i32,
    /// Local server.
    pub local_server: i32,
    /// Vote time.
    pub vote_time: i32,
    /// Vote yes.
    pub vote_yes: i32,
    /// Vote no.
    pub vote_no: i32,
    /// Vote modified.
    pub vote_modified: bool,
    /// Vote string.
    pub vote_string: String,
    /// Team vote time.
    pub team_vote_time: [i32; 2],
    /// Team vote yes.
    pub team_vote_yes: [i32; 2],
    /// Team vote no.
    pub team_vote_no: [i32; 2],
    /// Team vote modified.
    pub team_vote_modified: [bool; 2],
    /// Team vote string.
    pub team_vote_string: [String; 2],
    /// Level start time.
    pub level_start_time: i32,
    /// Scores 1.
    pub scores1: i32,
    /// Scores 2.
    pub scores2: i32,
    /// Red flag.
    pub redflag: i32,
    /// Blue flag.
    pub blueflag: i32,
    /// Flag status.
    pub flag_status: i32,
}

impl ClientGameStaticState {
    /// Blank static state with 64 client slots.
    #[must_use]
    pub fn new(product: Product) -> Self {
        Self {
            product,
            server_command_sequence: 0,
            cursor_x: 0.0,
            cursor_y: 0.0,
            event_handling: 0,
            active_cursor: None,
            team_chat_msgs: std::array::from_fn(|_| String::new()),
            team_chat_msg_times: [0; 8],
            team_chat_pos: 0,
            team_last_chat_pos: 0,
            current_voice_client: 0,
            accept_order_time: 0,
            accept_task: 0,
            accept_leader: 0,
            current_order: 0,
            order_pending: false,
            order_time: 0,
            client_info: (0..64).map(|_| shared(ClientInfo::default())).collect(),
            game_type: GameType::Ffa,
            fraglimit: 0,
            capturelimit: 0,
            maxclients: 0,
            local_server: 0,
            vote_time: 0,
            vote_yes: 0,
            vote_no: 0,
            vote_modified: false,
            vote_string: String::new(),
            team_vote_time: [0, 0],
            team_vote_yes: [0, 0],
            team_vote_no: [0, 0],
            team_vote_modified: [false, false],
            team_vote_string: std::array::from_fn(|_| String::new()),
            level_start_time: 0,
            scores1: 0,
            scores2: 0,
            redflag: 0,
            blueflag: 0,
            flag_status: 0,
        }
    }
}

/// Item catalog entry (`ItemDefinition`, used surface).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HudItem {
    /// Item index.
    pub index: usize,
    /// Icon path.
    pub icon: Option<String>,
    /// Pickup name.
    pub pickup_name: Option<String>,
}

/// Item catalog (`findItemForPowerup` / `itemList` / `itemAt`).
pub trait HudItemCatalog {
    /// Find the item for a powerup.
    fn find_for_powerup(&self, product: Product, powerup: i32) -> Option<HudItem>;
    /// Fetch an item by index.
    fn at(&self, product: Product, index: i32) -> HudItem;
    /// Index of an item.
    fn index_of(&self, product: Product, item: &HudItem) -> usize;
}

/// Renderer resource host (`RendererResources`, used surface).
pub trait RendererResources {
    /// Clear the scene.
    fn clear_scene(&mut self);
    /// Add a reference entity.
    fn add_ref_entity(&mut self, entity: &RefModelEntity);
    /// Render a scene.
    fn render_scene(&mut self, refdef: &Refdef);
    /// Picture for a shader.
    fn picture(&self, shader: &Option<SceneShader>) -> Picture;
    /// Register a shader.
    fn register_shader(&mut self, path: &str) -> Option<SceneShader>;
    /// Register a no-mip shader.
    fn register_shader_no_mip(&mut self, path: Option<&str>) -> Option<SceneShader>;
    /// Register a model.
    fn register_model(&mut self, path: Option<&str>) -> SceneModel;
    /// Handle for a model.
    fn model_handle(&self, model: &SceneModel) -> u32;
    /// Model for a handle.
    fn model_for_handle(&self, handle: u32) -> SceneModel;
    /// Shader for a handle.
    fn shader_for_handle(&self, handle: u32) -> Option<SceneShader>;
    /// Bounds for a model (`modelBounds`).
    fn model_bounds(&self, model: &SceneModel) -> Bounds;
}

/// Cgame sound bank (`ClientSoundBank`, used surface).
pub trait ClientSoundBank {
    /// Register a sound.
    fn register_sound(&mut self, path: Option<&str>, compressed: bool) -> Option<PcmSound>;
    /// Fetch (or synchronously load) a sound.
    fn sound(&mut self, path: Option<&str>, compressed: bool) -> Option<PcmSound>;
    /// Handle for a sound.
    fn index_for_sound(&self, sound: &Option<PcmSound>) -> i32;
    /// Asset for a sound.
    fn asset(&self, sound: &Option<PcmSound>) -> Option<SoundAsset>;
    /// Sound at a handle.
    fn sound_at_index(&self, index: i32) -> Option<PcmSound>;
    /// Optional sound at a handle.
    fn sound_for_index(&self, index: i32) -> Option<PcmSound>;
    /// All registrations.
    fn registrations(&self) -> Vec<SoundRegistration>;
}

/// Item visual (`PacketItemVisual`, used surface).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ItemVisual {
    /// Icon.
    pub icon: Option<SceneShader>,
}

/// Weapon visual (`ClientWeaponInfo`, used surface).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WeaponVisual {
    /// Ammo model.
    pub ammo_model: SceneModel,
    /// Ammo icon.
    pub ammo_icon: Option<SceneShader>,
    /// Weapon icon.
    pub weapon_icon: Option<SceneShader>,
}

/// Weapon/item visual registry (`ClientWeaponMediaRegistry`, used surface).
pub trait WeaponRegistryService {
    /// Visual for an item index.
    fn item_visual(&self, index: usize) -> ItemVisual;
    /// Visual for a weapon number.
    fn weapon(&self, number: i32) -> WeaponVisual;
    /// Register an item's visuals.
    fn register_item_visuals(&mut self, number: i32);
}

/// Client info store (`ClientInfoStore`, used surface).
pub trait ClientInfoStore {
    /// Canonical frame state.
    fn state_handle(&self) -> Shared<ClientGameState>;
    /// Canonical client slot.
    fn client_info(&self, index: i32) -> Shared<ClientInfo>;
    /// Load deferred players.
    fn load_deferred_players(&mut self, reset: &mut dyn FnMut(&mut ClientEntity));
    /// Publish a client info string.
    fn new_client_info(&mut self, index: i32, config: &str);
    /// Reset the store.
    fn reset(&mut self);
}

/// Player presenter (`PlayerPresenter`, used surface).
pub trait PlayerPresenter {
    /// Canonical frame state.
    fn state_handle(&self) -> Shared<ClientGameState>;
    /// Reset a player entity.
    fn reset_player_entity(&mut self, entity: &mut ClientEntity);
}

/// Prediction trace result.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PredictionTrace {
    /// Entity number.
    pub entity_num: i32,
    /// End position.
    pub end: Vec3,
}

/// Prediction service (`PredictionRuntime` + collision, used surface).
pub trait PredictionService {
    /// Canonical frame state.
    fn state_handle(&self) -> Shared<ClientGameState>;
    /// Trace a box.
    fn trace(&self, start: Vec3, end: Vec3, mins: Vec3, maxs: Vec3, skip: i32, contents: i32) -> PredictionTrace;
    /// Point contents.
    fn point_contents(&self, point: Vec3, pass_entity: i32) -> i32;
}

/// Weapon runtime + selection (`ClientWeaponRuntime` / `ClientWeaponSelection`).
pub trait WeaponService {
    /// Canonical frame state.
    fn state_handle(&self) -> Shared<ClientGameState>;
    /// Visual registry.
    fn registry_handle(&self) -> Shared<dyn WeaponRegistryService>;
    /// Draw the weapon selector.
    fn draw_weapon_select(&mut self);
    /// Next weapon.
    fn next_weapon(&mut self);
    /// Previous weapon.
    fn previous_weapon(&mut self);
    /// Select a weapon.
    fn select_weapon(&mut self, weapon: i32);
}

/// View runtime (`ViewRuntime`, used surface).
pub trait ViewService {
    /// Canonical frame state.
    fn state_handle(&self) -> Shared<ClientGameState>;
    /// Test gun.
    fn test_gun(&mut self, model: Option<String>, param: Option<f64>);
    /// Test model.
    fn test_model(&mut self, model: Option<String>, param: Option<f64>);
    /// Next model frame.
    fn next_model_frame(&mut self);
    /// Previous model frame.
    fn previous_model_frame(&mut self);
    /// Next model skin.
    fn next_model_skin(&mut self);
    /// Previous model skin.
    fn previous_model_skin(&mut self);
    /// Zoom down.
    fn zoom_down(&mut self);
    /// Zoom up.
    fn zoom_up(&mut self);
    /// Clear the test model.
    fn clear_test_model(&mut self);
}

/// Server command runtime (`ClientServerCommandRuntime`, used surface).
pub trait ServerCommandService {
    /// Build the spectator string.
    fn build_spectator_string(&mut self);
}

/// Weapon HUD ammo (`WeaponHudStatus.ammo`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeaponHudAmmo {
    /// Unmetered.
    Unmetered,
    /// Finite count.
    Finite {
        /// Count.
        count: i32,
    },
}

/// Weapon HUD status (`WeaponHudStatus`, used surface).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WeaponHudStatus {
    /// Ammo.
    pub ammo: WeaponHudAmmo,
}

/// Ammo warning (`ArsenalAmmoWarning`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArsenalAmmoWarning {
    /// None.
    None,
    /// Low.
    Low,
    /// Empty.
    Empty,
}

/// Weapon HUD report (`WeaponHudReader` result).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WeaponHudReport {
    /// Status.
    pub status: Option<WeaponHudStatus>,
    /// Warning.
    pub warning: ArsenalAmmoWarning,
}

/// Weapon HUD reader (`WeaponHudReader`).
pub trait WeaponHudReader {
    /// Read the current report.
    fn read_weapon_hud(&self) -> WeaponHudReport;
}

/// User command (`read` result with `serverTime`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UserCommand {
    /// Server time.
    pub server_time: i32,
}

/// Command source (`CommandSource`, used surface).
pub trait CommandSource {
    /// Current command number.
    fn current_number(&self) -> i32;
    /// Read a command.
    fn read(&self, number: i32) -> Option<UserCommand>;
}

/// Cached VM cvar reader (`readVmCvar`).
pub trait HudCvarReader {
    /// Read a cached VM cvar.
    fn read_vm_cvar(&self, name: &str) -> CvarSnapshot;
}

/// Configstring source (`configString`).
pub trait HudConfigStrings {
    /// Read a configstring.
    fn config_string(&self, index: usize) -> String;
}

/// Client command sender (`sendClientCommand` / `sendConsoleCommand` / `addCommand` / `print`).
pub trait HudCommands {
    /// Send a client command.
    fn send_client_command(&mut self, text: &str);
    /// Send a console command.
    fn send_console_command(&mut self, text: &str);
    /// Register a command name.
    fn add_command(&mut self, name: &str);
    /// Print.
    fn print(&mut self, text: &str);
}

/// Millisecond clock (`milliseconds`).
pub trait HudClock {
    /// Current milliseconds.
    fn milliseconds(&self) -> i32;
}

/// Local sound starter (`startLocalSound` / `startSound`).
pub trait HudLocalSound {
    /// Start a local sound.
    fn start_local_sound(&mut self, sound: Option<PcmSound>, channel: i32);
    /// Start a placed sound.
    fn start_sound(&mut self, origin: Option<Vec3>, entity: i32, channel: i32, sound: Option<PcmSound>);
}

/// Cgame graphics (`ClientMediaGraphics`, used surface).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ClientGraphics {
    /// Charset shader.
    pub charset_shader: Option<SceneShader>,
    /// Proportional charset.
    pub charset_prop: Option<SceneShader>,
    /// Proportional glow.
    pub charset_prop_glow: Option<SceneShader>,
    /// Banner charset.
    pub charset_prop_b: Option<SceneShader>,
    /// White shader.
    pub white_shader: Option<SceneShader>,
    /// Back tile.
    pub back_tile_shader: Option<SceneShader>,
    /// Team status bar.
    pub team_status_bar: Option<SceneShader>,
    /// Select shader.
    pub select_shader: Option<SceneShader>,
    /// Defer shader.
    pub defer_shader: Option<SceneShader>,
    /// Lagometer shader.
    pub lagometer_shader: Option<SceneShader>,
    /// Red flag model.
    pub red_flag_model: SceneModel,
    /// Blue flag model.
    pub blue_flag_model: SceneModel,
    /// Neutral flag model.
    pub neutral_flag_model: SceneModel,
    /// Armor model.
    pub armor_model: SceneModel,
    /// Armor icon.
    pub armor_icon: Option<SceneShader>,
    /// Crosshair shaders (10).
    pub crosshair_shader: Vec<Option<SceneShader>>,
    /// Number shaders (11).
    pub number_shaders: Vec<Option<SceneShader>>,
    /// Bot skill shaders (5).
    pub bot_skill_shaders: Vec<Option<SceneShader>>,
    /// Scoreboard score header.
    pub scoreboard_score: Option<SceneShader>,
    /// Scoreboard ping header.
    pub scoreboard_ping: Option<SceneShader>,
    /// Scoreboard time header.
    pub scoreboard_time: Option<SceneShader>,
    /// Scoreboard name header.
    pub scoreboard_name: Option<SceneShader>,
    /// Red flag status shaders (3).
    pub red_flag_shader: Vec<Option<SceneShader>>,
    /// Blue flag status shaders (3).
    pub blue_flag_shader: Vec<Option<SceneShader>>,
    /// Generic flag status shaders (3).
    pub flag_shaders: Vec<Option<SceneShader>>,
    /// Assault shader.
    pub assault_shader: Option<SceneShader>,
    /// Defend shader.
    pub defend_shader: Option<SceneShader>,
    /// Patrol shader.
    pub patrol_shader: Option<SceneShader>,
    /// Follow shader.
    pub follow_shader: Option<SceneShader>,
    /// Retrieve shader.
    pub retrieve_shader: Option<SceneShader>,
    /// Escort shader.
    pub escort_shader: Option<SceneShader>,
    /// Camp shader.
    pub camp_shader: Option<SceneShader>,
    /// Red cube model.
    pub red_cube_model: SceneModel,
    /// Blue cube model.
    pub blue_cube_model: SceneModel,
    /// Red cube icon.
    pub red_cube_icon: Option<SceneShader>,
    /// Blue cube icon.
    pub blue_cube_icon: Option<SceneShader>,
    /// Heart shader.
    pub heart_shader: Option<SceneShader>,
    /// Select cursor.
    pub select_cursor: Option<SceneShader>,
    /// Size cursor.
    pub size_cursor: Option<SceneShader>,
}

impl ClientGraphics {
    /// Blank graphics with sized shader tables.
    #[must_use]
    pub fn new() -> Self {
        Self {
            crosshair_shader: vec![None; 10],
            number_shaders: vec![None; 11],
            bot_skill_shaders: vec![None; 5],
            red_flag_shader: vec![None; 3],
            blue_flag_shader: vec![None; 3],
            flag_shaders: vec![None; 3],
            ..Self::default()
        }
    }
}

/// Cgame sounds (`ClientMediaSounds`, used surface).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ClientSounds {
    /// Talk sound.
    pub talk_sound: Option<PcmSound>,
    /// Count 1 sound.
    pub count1_sound: Option<PcmSound>,
    /// Count 2 sound.
    pub count2_sound: Option<PcmSound>,
    /// Count 3 sound.
    pub count3_sound: Option<PcmSound>,
    /// Winner sound.
    pub winner_sound: Option<PcmSound>,
    /// Loser sound.
    pub loser_sound: Option<PcmSound>,
    /// Wear-off sound.
    pub wear_off_sound: Option<PcmSound>,
}

/// Map-lifetime cgame media (`ClientMedia`, used surface).
pub struct ClientMedia {
    /// Product.
    pub product: Product,
    /// Static state.
    pub static_state: Shared<ClientGameStaticState>,
    /// Renderer resources.
    pub resources: Shared<dyn RendererResources>,
    /// Sound bank.
    pub sound_bank: Shared<dyn ClientSoundBank>,
    /// Weapon registry.
    pub weapon_registry: Shared<dyn WeaponRegistryService>,
    /// Item catalog.
    pub items: Shared<dyn HudItemCatalog>,
    /// Graphics.
    pub graphics: ClientGraphics,
    /// Sounds.
    pub sounds: ClientSounds,
}

impl ClientMedia {
    /// Assemble media, checking the product.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        product: Product,
        static_state: Shared<ClientGameStaticState>,
        resources: Shared<dyn RendererResources>,
        sound_bank: Shared<dyn ClientSoundBank>,
        weapon_registry: Shared<dyn WeaponRegistryService>,
        items: Shared<dyn HudItemCatalog>,
        graphics: ClientGraphics,
        sounds: ClientSounds,
    ) -> Self {
        if static_state.borrow().product != product {
            panic!("Client media product differs from cgs");
        }
        Self {
            product,
            static_state,
            resources,
            sound_bank,
            weapon_registry,
            items,
            graphics,
            sounds,
        }
    }
}

/// Engine sound bank (`SoundBank`, used surface).
pub trait SoundBank {
    /// Register a path for a family.
    fn register(&mut self, path: &str, family: &str) -> Option<SoundAsset>;
}

/// Sound checkpoint row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoundCheckpointRow {
    /// Path.
    pub path: String,
    /// Compressed intent.
    pub compressed: bool,
    /// Handle.
    pub handle: i32,
    /// Resource binding.
    pub resource: Option<String>,
}

/// Seat identifier (`SeatId`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct SeatId(pub u32);

/// Actor identifier (`ActorId`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ActorId(pub u32);

/// Sound origin (`SoundOrigin`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SoundOrigin {
    /// Attached to an actor.
    Actor {
        /// Actor.
        actor: ActorId,
    },
    /// Fixed position.
    Fixed {
        /// Position.
        position: Vec3,
    },
    /// Local.
    Local,
}

/// One-shot sound (`PlaySound`).
#[derive(Debug, Clone, PartialEq)]
pub struct PlaySound {
    /// Asset.
    pub sound: SoundAsset,
    /// Actor.
    pub actor: Option<ActorId>,
    /// Origin.
    pub origin: SoundOrigin,
    /// Seat audience.
    pub seat: SeatId,
    /// Channel.
    pub channel: i32,
    /// Volume.
    pub volume: f32,
    /// Attenuation.
    pub attenuation: f32,
}

/// Looping sound (`LoopSound`).
#[derive(Debug, Clone, PartialEq)]
pub struct LoopSound {
    /// Asset.
    pub sound: SoundAsset,
    /// Actor.
    pub actor: ActorId,
    /// Origin.
    pub origin: SoundOrigin,
    /// Seat audience.
    pub seat: SeatId,
    /// Velocity.
    pub velocity: Vec3,
    /// Volume.
    pub volume: f32,
    /// Attenuation.
    pub attenuation: f32,
    /// Frame number.
    pub frame_number: i32,
    /// Persistent loop.
    pub persistent: bool,
}

/// Start-sound origin (`StartSoundOptions.origin`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StartSoundOrigin {
    /// Entity-attached.
    Entity {
        /// Entity.
        entity: i32,
    },
    /// Fixed position.
    Fixed {
        /// Position.
        position: Vec3,
    },
    /// Local.
    Local,
}

/// Start-sound options (`StartSoundOptions`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StartSoundOptions {
    /// Entity.
    pub entity: i32,
    /// Origin.
    pub origin: StartSoundOrigin,
    /// Channel.
    pub channel: i32,
    /// Volume (0-127).
    pub volume: f32,
}

/// Menu end sound name (`"winnerSound" | "loserSound"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuEndSound {
    /// Winner.
    Winner,
    /// Loser.
    Loser,
}

/// Loading screen updater (`updateScreen`).
pub trait LoadingScreenUpdater {
    /// Update the screen.
    fn update_screen(&mut self);
}

/// Captured menu handle (`UiCapturedMenu`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CapturedMenu {
    /// Identity.
    pub id: u64,
    /// Name.
    pub name: String,
}

/// Menu snapshot (`UiRuntimeSnapshot`, used surface).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MenuSnapshot {
    /// Open menus.
    pub open_menus: Vec<String>,
}

/// Menu frame (`UiRuntimeFrame` time surface; draw travels separately).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MenuFrame {
    /// Time (always 0; the source never assigns `cgDC.realTime`).
    pub time: i32,
    /// Frame time (always 0).
    pub frame_time: i32,
}

/// Menu script source (`ScriptSource`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuSource {
    /// Path.
    pub path: String,
    /// Text.
    pub text: String,
}

/// Font reference (`UiFontReference`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontReference {
    /// Path.
    pub path: Option<String>,
    /// Point size.
    pub point_size: i32,
}

/// Asset reference with a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetReference {
    /// Path.
    pub path: Option<String>,
}

/// Global menu assets (`UiGlobalAssets`, used surface).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MenuGlobalAssets {
    /// Gradient bar.
    pub gradient_bar: Option<AssetReference>,
}

/// Menu definitions (`UiMenuDefinitions`, used surface).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MenuDefinitions {
    /// Assets.
    pub assets: MenuGlobalAssets,
}

/// Menu reset scope (`"strings" | "menus"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuResetScope {
    /// Strings.
    Strings,
    /// Menus.
    Menus,
}

/// Menu cursor type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuCursorType {
    /// Arrow.
    Arrow,
    /// Other (sized).
    Other,
}

/// Feeder item (`UiRuntimeFeederItem`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuFeederItem {
    /// Text.
    pub text: String,
    /// Picture.
    pub picture: Option<Picture>,
}

/// Widget assets (`UiWidgetAssets`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiWidgetAssets {
    /// White shader.
    pub white_shader: Picture,
    /// Gradient bar.
    pub gradient_bar: Picture,
    /// Scroll bar.
    pub scroll_bar: Picture,
    /// Scroll arrow down.
    pub scroll_bar_arrow_down: Picture,
    /// Scroll arrow up.
    pub scroll_bar_arrow_up: Picture,
    /// Scroll arrow left.
    pub scroll_bar_arrow_left: Picture,
    /// Scroll arrow right.
    pub scroll_bar_arrow_right: Picture,
    /// Scroll thumb.
    pub scroll_bar_thumb: Picture,
    /// Slider bar.
    pub slider_bar: Picture,
    /// Slider thumb.
    pub slider_thumb: Picture,
}

/// Menu audio (`UiRuntimeAudio`).
pub trait HudMenuAudio {
    /// Play a local sound.
    fn play_local(&mut self, sound: MenuAudioSound);
    /// Start background audio.
    fn start_background(&mut self, path: Option<String>);
    /// Stop background audio.
    fn stop_background(&mut self);
}

/// Menu audio sound (`PcmSound | number | undefined`).
#[derive(Debug, Clone, PartialEq)]
pub enum MenuAudioSound {
    /// Missing.
    Missing,
    /// PCM.
    Pcm(Option<PcmSound>),
    /// Handle.
    Handle(i32),
}

/// Cinematic asset (`UiCinematicAsset`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CinematicAsset {
    /// Path.
    pub path: String,
}

/// Cinematic instance (`UiCinematicInstance`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CinematicInstance {
    /// Asset.
    pub asset: CinematicAsset,
    /// Handle index.
    pub handle: i32,
}

/// Cinematics (`UiRuntimeCinematics` + `EngineUiCinematics.owner`).
pub trait CinematicService {
    /// Play a cinematic.
    fn play(&mut self, asset: &CinematicAsset, rect: Rect2d) -> Option<CinematicInstance>;
    /// Run a cinematic.
    fn run(&mut self, handle: i32, time: i32);
    /// Draw a cinematic.
    fn draw(&mut self, handle: i32, rect: Rect2d, draw: &Draw2D);
    /// Stop a cinematic.
    fn stop(&mut self, handle: i32);
    /// Prepare a cinematic.
    fn prepare(&mut self, path: &str) -> CinematicAsset;
    /// Stop a slot.
    fn stop_slot(&mut self, index: i32);
}

/// Font registry (`UiAssetRegistry.registerFont`).
pub trait FontRegistry {
    /// Register a font.
    fn register_font(&mut self, path: Option<&str>, point_size: i32) -> Option<RegisteredFont>;
}

/// Sound asset reader (`SoundAssetReader`, used surface).
pub trait SoundAssetReader {
    /// Whether an asset exists.
    fn has(&self, path: &str) -> bool;
    /// Read an asset.
    fn read_sync(&self, path: &str) -> Vec<u8>;
}

/// Command buffer (`CommandBuffer.append`).
pub trait HudCommandBuffer {
    /// Append text.
    fn append(&mut self, text: &str);
}

/// Menu paint callbacks (feeder + owner-draw + team color + model paint).
pub trait MenuPaintCallbacks {
    /// Feeder count.
    fn feeder_count(&mut self, feeder: i32) -> i32;
    /// Feeder item.
    fn feeder_item(&mut self, feeder: i32, index: i32, column: i32) -> MenuFeederItem;
    /// Feeder selection.
    fn feeder_select(&mut self, feeder: i32, index: i32);
    /// Owner-draw visibility.
    fn owner_visible(&mut self, flags: i32) -> bool;
    /// Owner-draw width.
    fn owner_width(&mut self, id: i32, scale: f32) -> f32;
    /// Owner-draw value.
    fn owner_value(&mut self, id: i32) -> f32;
    /// Owner-draw paint.
    fn owner_paint(&mut self, request: &mut OwnerDrawPaintRequest);
    /// Close a cinematic.
    fn close_cinematic(&mut self, handle: i32);
    /// Team color.
    fn team_color(&mut self) -> Vec4;
    /// Paint a model.
    fn paint_model(&mut self, request: &UiModelPaintRequest);
    /// Cvar value.
    fn cvar_value(&mut self, name: &str) -> f64;
}

/// Menu runtime (`UiRuntime`, used surface).
pub trait MenuRuntime {
    /// Snapshot.
    fn snapshot(&self) -> MenuSnapshot;
    /// Reload definitions.
    fn reload_definitions(&mut self, definitions: &MenuDefinitions);
    /// Reset definitions.
    fn reset_definitions(&mut self, scope: MenuResetScope);
    /// Run a frame.
    fn frame(&mut self, frame: &MenuFrame, draw: &mut Draw2D, callbacks: &mut dyn MenuPaintCallbacks);
    /// Paint a captured menu.
    fn paint_captured(
        &mut self,
        menu: &CapturedMenu,
        frame: &MenuFrame,
        force: bool,
        draw: &mut Draw2D,
        callbacks: &mut dyn MenuPaintCallbacks,
    );
    /// Clear captured forced state.
    fn clear_captured_forced(&mut self, menu: &CapturedMenu);
    /// Menu handle by name.
    fn menu_handle(&self, name: &str) -> Option<CapturedMenu>;
    /// Set a captured feeder selection.
    fn set_captured_feeder_selection(&mut self, menu: &CapturedMenu, feeder: i32, index: i32);
    /// Scroll a captured feeder.
    fn scroll_captured_feeder(&mut self, menu: &CapturedMenu, feeder: i32, down: bool);
    /// Close by name.
    fn close(&mut self, name: &str);
    /// Show by name.
    fn show(&mut self, name: &str);
    /// Set the display cursor.
    fn set_display_cursor(&mut self, x: f32, y: f32);
    /// Cursor type at a point.
    fn cursor_type(&self, x: f32, y: f32) -> MenuCursorType;
    /// Move a captured menu.
    fn move_captured_menu(&mut self, menu: &CapturedMenu, dx: f32, dy: f32);
    /// Pointer move.
    fn pointer_move(&mut self, x: f32, y: f32);
    /// Handle a key.
    fn handle_key(&mut self, key: i32, down: bool, x: f32, y: f32);
    /// Capture the menu at a point.
    fn capture_menu(&mut self, x: f32, y: f32) -> Option<CapturedMenu>;
    /// Retire the runtime.
    fn retire(&mut self);
}

/// Menu load context (`loadMenuDefinitions` callbacks).
pub trait MenuLoadContext {
    /// Random int.
    fn random_next_int(&mut self) -> i32;
    /// Resolve the root source.
    fn resolve_root(&mut self, requested: &str) -> Option<MenuSource>;
    /// Resolve a nested source.
    fn resolve(&mut self, from_path: &str, requested: &str) -> Option<MenuSource>;
    /// Register a font.
    fn register_font(&mut self, reference: &FontReference);
    /// Register a picture, returning its handle.
    fn register_picture(&mut self, path: Option<&str>) -> u32;
    /// Register a sound, returning its handle.
    fn register_sound(&mut self, path: Option<&str>) -> u32;
    /// Register a model, returning its handle.
    fn register_model(&mut self, path: Option<&str>) -> u32;
    /// Publish an asset font field.
    fn publish_asset_font(&mut self, field: &str, reference: &FontReference);
    /// Initial assets.
    fn initial_assets(&self) -> Option<MenuGlobalAssets>;
}

/// Menu runtime seed for creation.
#[derive(Clone)]
pub struct MenuRuntimeSeed {
    /// Definitions.
    pub definitions: MenuDefinitions,
    /// Cvars.
    pub cvars: Shared<CvarRegistry>,
    /// Widget assets.
    pub widget_assets: UiWidgetAssets,
    /// Zero picture.
    pub zero_picture: Picture,
    /// Fonts.
    pub fonts: FontSet,
}

/// Owner-draw paint request (`UiOwnerDrawPaintRequest`).
#[derive(Clone)]
pub struct OwnerDrawPaintRequest {
    /// Drawing context (must share the HUD recorder queue).
    pub draw: Draw2D,
    /// Rectangle.
    pub rect: Rect2d,
    /// Text X.
    pub text_x: f32,
    /// Text Y.
    pub text_y: f32,
    /// Owner-draw id.
    pub owner_draw: i32,
    /// Owner-draw flags.
    pub owner_draw_flags: i32,
    /// Alignment.
    pub alignment: i32,
    /// Special.
    pub special: i32,
    /// Text scale.
    pub text_scale: f32,
    /// Color.
    pub color: Vec4,
    /// Background picture.
    pub background: Option<Picture>,
    /// Text style.
    pub text_style: i32,
}

#[cfg(test)]
#[allow(dead_code)]
mod tests {
    use super::*;
    use crate::q3::presentation::console::*;
    use crate::q3::presentation::mission_hud::*;
    use std::cell::Cell;

    use qa_core::cvar::{flags as cvar_flags, CvarRegistry, CvarSnapshot};
    use std::collections::HashMap;
    use std::rc::Rc;

    use crate::q3::presentation::config::*;

    use crate::q3::presentation::draw_icons::*;
    use crate::q3::presentation::draw_status::*;
    use crate::q3::presentation::draw_tools::*;
    use crate::q3::presentation::frame_audio::*;
    use crate::q3::presentation::hud::*;
    use crate::q3::presentation::hud_corners::*;

    use crate::q3::presentation::mission_owner_draw::*;
    use crate::q3::presentation::scoreboard::*;

    use qa_core::cmd::Dialect;

    /// Recording pixel sink.
    #[derive(Debug, Default)]
    struct FakeSink {
        /// Colors set.
        colors: Vec<Option<Vec4>>,
        /// Blits.
        blits: Vec<(Rect2d, TextureRect, Picture)>,
    }

    impl HudDrawSink for FakeSink {
        fn set_color(&mut self, color: Option<Vec4>) {
            self.colors.push(color);
        }
        fn stretch_pixels(&mut self, rect: Rect2d, uv: TextureRect, picture: Picture) {
            self.blits.push((rect, uv, picture));
        }
    }

    /// Canned renderer resources.
    #[derive(Debug, Default)]
    struct FakeResources {
        /// Scene calls.
        scenes: Vec<String>,
        /// Registered shaders.
        shaders: Vec<String>,
        /// Next picture order.
        order: u32,
        /// Bounds to return.
        bounds: Option<Bounds>,
    }

    impl RendererResources for FakeResources {
        fn clear_scene(&mut self) {
            self.scenes.push("clear".to_string());
        }
        fn add_ref_entity(&mut self, _entity: &RefModelEntity) {
            self.scenes.push("add".to_string());
        }
        fn render_scene(&mut self, _refdef: &Refdef) {
            self.scenes.push("render".to_string());
        }
        fn picture(&self, shader: &Option<SceneShader>) -> Picture {
            Picture {
                order: shader.as_ref().map(|shader| shader.name.len() as u32 + 1).unwrap_or(0),
            }
        }
        fn register_shader(&mut self, path: &str) -> Option<SceneShader> {
            self.shaders.push(path.to_string());
            Some(SceneShader { name: path.to_string() })
        }
        fn register_shader_no_mip(&mut self, path: Option<&str>) -> Option<SceneShader> {
            path.map(|path| {
                self.shaders.push(path.to_string());
                SceneShader { name: path.to_string() }
            })
        }
        fn register_model(&mut self, path: Option<&str>) -> SceneModel {
            path.map(|path| SceneModel::Loaded { path: path.to_string() })
                .unwrap_or_default()
        }
        fn model_handle(&self, model: &SceneModel) -> u32 {
            model.path().len() as u32 + 1
        }
        fn model_for_handle(&self, _handle: u32) -> SceneModel {
            SceneModel::Loaded {
                path: "handle".to_string(),
            }
        }
        fn shader_for_handle(&self, handle: u32) -> Option<SceneShader> {
            if handle == 0 {
                None
            } else {
                Some(SceneShader {
                    name: format!("handle{handle}"),
                })
            }
        }
        fn model_bounds(&self, _model: &SceneModel) -> Bounds {
            self.bounds.unwrap_or(Bounds {
                min: vec3(-8.0, -8.0, -24.0),
                max: vec3(8.0, 8.0, 32.0),
            })
        }
    }

    /// Canned sound bank.
    #[derive(Debug, Default)]
    struct FakeSoundBank {
        /// Registered paths.
        paths: Vec<String>,
        /// Sounds by path.
        sounds: HashMap<String, PcmSound>,
    }

    impl ClientSoundBank for FakeSoundBank {
        fn register_sound(&mut self, path: Option<&str>, _compressed: bool) -> Option<PcmSound> {
            let path = path?;
            self.paths.push(path.to_string());
            let sound = pcm_sound(path);
            self.sounds.insert(path.to_string(), sound.clone());
            Some(sound)
        }
        fn sound(&mut self, path: Option<&str>, compressed: bool) -> Option<PcmSound> {
            self.register_sound(path, compressed)
        }
        fn index_for_sound(&self, _sound: &Option<PcmSound>) -> i32 {
            1
        }
        fn asset(&self, sound: &Option<PcmSound>) -> Option<SoundAsset> {
            sound.clone().map(|pcm| SoundAsset { pcm, resource: None })
        }
        fn sound_at_index(&self, _index: i32) -> Option<PcmSound> {
            None
        }
        fn sound_for_index(&self, _index: i32) -> Option<PcmSound> {
            None
        }
        fn registrations(&self) -> Vec<SoundRegistration> {
            Vec::new()
        }
    }

    /// Canned engine bank.
    #[derive(Debug, Default)]
    struct FakeEngineBank {
        /// Paths.
        paths: Vec<String>,
    }

    impl SoundBank for FakeEngineBank {
        fn register(&mut self, path: &str, _family: &str) -> Option<SoundAsset> {
            self.paths.push(path.to_string());
            Some(SoundAsset {
                pcm: pcm_sound(path),
                resource: Some(format!("res:{path}")),
            })
        }
    }

    /// Canned weapon registry.
    #[derive(Debug, Default)]
    struct FakeRegistry {
        /// Registered visuals.
        visuals: Vec<i32>,
    }

    impl WeaponRegistryService for FakeRegistry {
        fn item_visual(&self, index: usize) -> ItemVisual {
            ItemVisual {
                icon: Some(SceneShader {
                    name: format!("item{index}"),
                }),
            }
        }
        fn weapon(&self, number: i32) -> WeaponVisual {
            WeaponVisual {
                ammo_model: SceneModel::Loaded {
                    path: format!("ammo{number}"),
                },
                ammo_icon: Some(SceneShader {
                    name: format!("ammo{number}"),
                }),
                weapon_icon: Some(SceneShader {
                    name: format!("weapon{number}"),
                }),
            }
        }
        fn register_item_visuals(&mut self, number: i32) {
            self.visuals.push(number);
        }
    }

    /// Canned item catalog.
    #[derive(Debug, Default)]
    struct FakeCatalog;

    impl HudItemCatalog for FakeCatalog {
        fn find_for_powerup(&self, _product: Product, powerup: i32) -> Option<HudItem> {
            if powerup == Powerup::RedFlag as i32
                || powerup == Powerup::BlueFlag as i32
                || powerup == Powerup::NeutralFlag as i32
            {
                Some(HudItem {
                    index: powerup as usize,
                    icon: Some(format!("icons/flag{powerup}")),
                    pickup_name: Some(format!("Flag {powerup}")),
                })
            } else {
                None
            }
        }
        fn at(&self, _product: Product, index: i32) -> HudItem {
            HudItem {
                index: index as usize,
                icon: Some(format!("icons/item{index}")),
                pickup_name: Some(format!("Item {index}")),
            }
        }
        fn index_of(&self, _product: Product, item: &HudItem) -> usize {
            item.index
        }
    }

    /// Canned cvar reader.
    struct FakeCvars {
        /// Values by lowercase name.
        values: HashMap<String, CvarSnapshot>,
    }

    impl FakeCvars {
        /// Blank reader.
        fn new() -> Self {
            Self { values: HashMap::new() }
        }

        /// Set an integer value.
        fn set(&mut self, name: &str, integer: i32, numeric: f32, value: &str) {
            self.values.insert(
                name.to_lowercase(),
                CvarSnapshot {
                    name: name.to_string(),
                    value: value.to_string(),
                    reset_value: value.to_string(),
                    latched_value: None,
                    flags: 0,
                    modified: false,
                    modification_count: 1,
                    numeric_value: numeric,
                    integer_value: integer,
                },
            );
        }
    }

    impl HudCvarReader for FakeCvars {
        fn read_vm_cvar(&self, name: &str) -> CvarSnapshot {
            self.values.get(&name.to_lowercase()).cloned().unwrap_or(CvarSnapshot {
                name: name.to_string(),
                value: "0".to_string(),
                reset_value: "0".to_string(),
                latched_value: None,
                flags: 0,
                modified: false,
                modification_count: 0,
                numeric_value: 0.0,
                integer_value: 0,
            })
        }
    }

    /// Canned configstrings.
    #[derive(Default)]
    struct FakeStrings {
        /// Strings by index.
        values: HashMap<usize, String>,
    }

    impl HudConfigStrings for FakeStrings {
        fn config_string(&self, index: usize) -> String {
            self.values.get(&index).cloned().unwrap_or_default()
        }
    }

    /// Recording commands.
    #[derive(Default)]
    struct FakeCommands {
        /// Client commands.
        client: Vec<String>,
        /// Console commands.
        console: Vec<String>,
        /// Added names.
        added: Vec<String>,
        /// Printed lines.
        printed: Vec<String>,
    }

    impl HudCommands for FakeCommands {
        fn send_client_command(&mut self, text: &str) {
            self.client.push(text.to_string());
        }
        fn send_console_command(&mut self, text: &str) {
            self.console.push(text.to_string());
        }
        fn add_command(&mut self, name: &str) {
            self.added.push(name.to_string());
        }
        fn print(&mut self, text: &str) {
            self.printed.push(text.to_string());
        }
    }

    /// Canned clock.
    struct FakeClock {
        /// Time.
        time: i32,
    }

    impl HudClock for FakeClock {
        fn milliseconds(&self) -> i32 {
            self.time
        }
    }

    /// Recording sounds.
    #[derive(Default)]
    struct FakeLocalSound {
        /// Local starts.
        local: Vec<(String, i32)>,
        /// Placed starts.
        placed: Vec<(i32, i32)>,
    }

    impl HudLocalSound for FakeLocalSound {
        fn start_local_sound(&mut self, sound: Option<PcmSound>, channel: i32) {
            self.local.push((
                sound.map(|sound| sound.borrow().name.clone()).unwrap_or_default(),
                channel,
            ));
        }
        fn start_sound(&mut self, _origin: Option<Vec3>, entity: i32, channel: i32, _sound: Option<PcmSound>) {
            self.placed.push((entity, channel));
        }
    }

    /// Canned client store over canonical slots.
    struct FakeStore {
        /// State.
        state: Shared<ClientGameState>,
        /// Slots.
        slots: Vec<Shared<ClientInfo>>,
        /// Deferred loads.
        loads: Cell<i32>,
    }

    impl ClientInfoStore for FakeStore {
        fn state_handle(&self) -> Shared<ClientGameState> {
            self.state.clone()
        }
        fn client_info(&self, index: i32) -> Shared<ClientInfo> {
            self.slots[index as usize].clone()
        }
        fn load_deferred_players(&mut self, _reset: &mut dyn FnMut(&mut ClientEntity)) {
            self.loads.set(self.loads.get() + 1);
        }
        fn new_client_info(&mut self, _index: i32, _config: &str) {}
        fn reset(&mut self) {}
    }

    /// Canned presenter.
    struct FakePresenter {
        /// State.
        state: Shared<ClientGameState>,
    }

    impl PlayerPresenter for FakePresenter {
        fn state_handle(&self) -> Shared<ClientGameState> {
            self.state.clone()
        }
        fn reset_player_entity(&mut self, _entity: &mut ClientEntity) {}
    }

    /// Canned prediction.
    struct FakePrediction {
        /// State.
        state: Shared<ClientGameState>,
        /// Trace result.
        trace: PredictionTrace,
        /// Contents.
        contents: i32,
    }

    impl PredictionService for FakePrediction {
        fn state_handle(&self) -> Shared<ClientGameState> {
            self.state.clone()
        }
        fn trace(
            &self,
            _start: Vec3,
            _end: Vec3,
            _mins: Vec3,
            _maxs: Vec3,
            _skip: i32,
            _contents: i32,
        ) -> PredictionTrace {
            self.trace
        }
        fn point_contents(&self, _point: Vec3, _pass_entity: i32) -> i32 {
            self.contents
        }
    }

    /// Canned weapons.
    struct FakeWeapons {
        /// State.
        state: Shared<ClientGameState>,
        /// Registry.
        registry: Shared<dyn WeaponRegistryService>,
        /// Selections.
        selected: Vec<i32>,
    }

    impl WeaponService for FakeWeapons {
        fn state_handle(&self) -> Shared<ClientGameState> {
            self.state.clone()
        }
        fn registry_handle(&self) -> Shared<dyn WeaponRegistryService> {
            self.registry.clone()
        }
        fn draw_weapon_select(&mut self) {}
        fn next_weapon(&mut self) {}
        fn previous_weapon(&mut self) {}
        fn select_weapon(&mut self, weapon: i32) {
            self.selected.push(weapon);
        }
    }

    /// Canned view.
    struct FakeView {
        /// State.
        state: Shared<ClientGameState>,
        /// Calls.
        calls: Vec<String>,
    }

    impl ViewService for FakeView {
        fn state_handle(&self) -> Shared<ClientGameState> {
            self.state.clone()
        }
        fn test_gun(&mut self, _model: Option<String>, _param: Option<f64>) {
            self.calls.push("testgun".to_string());
        }
        fn test_model(&mut self, _model: Option<String>, _param: Option<f64>) {
            self.calls.push("testmodel".to_string());
        }
        fn next_model_frame(&mut self) {
            self.calls.push("nextframe".to_string());
        }
        fn previous_model_frame(&mut self) {
            self.calls.push("prevframe".to_string());
        }
        fn next_model_skin(&mut self) {
            self.calls.push("nextskin".to_string());
        }
        fn previous_model_skin(&mut self) {
            self.calls.push("prevskin".to_string());
        }
        fn zoom_down(&mut self) {
            self.calls.push("+zoom".to_string());
        }
        fn zoom_up(&mut self) {
            self.calls.push("-zoom".to_string());
        }
        fn clear_test_model(&mut self) {
            self.calls.push("clear".to_string());
        }
    }

    /// Canned server commands.
    #[derive(Default)]
    struct FakeServerCommands {
        /// Builds.
        builds: i32,
    }

    impl ServerCommandService for FakeServerCommands {
        fn build_spectator_string(&mut self) {
            self.builds += 1;
        }
    }

    /// Canned command source.
    #[derive(Default)]
    struct FakeCommandSource {
        /// Current number.
        current: i32,
        /// Commands.
        commands: HashMap<i32, UserCommand>,
    }

    impl CommandSource for FakeCommandSource {
        fn current_number(&self) -> i32 {
            self.current
        }
        fn read(&self, number: i32) -> Option<UserCommand> {
            self.commands.get(&number).copied()
        }
    }

    /// Canned frame-audio host.
    #[derive(Default)]
    struct FakeFrameAudio {
        /// Local starts.
        local: Vec<i32>,
        /// Placed starts.
        placed: Vec<(i32, i32)>,
    }

    impl ClientFrameAudioHost for FakeFrameAudio {
        fn start_local_sound(&mut self, _sound: Option<PcmSound>, channel: i32) {
            self.local.push(channel);
        }
        fn start_sound(&mut self, _origin: Option<Vec3>, entity: i32, channel: i32, _sound: Option<PcmSound>) {
            self.placed.push((entity, channel));
        }
    }

    /// Test world fixture.
    struct World {
        /// State.
        state: Shared<ClientGameState>,
        /// Static state.
        static_state: Shared<ClientGameStaticState>,
        /// Media.
        media: Shared<ClientMedia>,
        /// Sink.
        sink: Shared<FakeSink>,
        /// Draw.
        draw: Draw2D,
        /// Tools.
        tools: ClientDrawTools,
        /// Icons.
        icons: Shared<ClientDrawIcons>,
        /// Cvars.
        cvars: Shared<FakeCvars>,
        /// Strings.
        strings: Shared<FakeStrings>,
        /// Commands.
        commands: Shared<FakeCommands>,
        /// Sounds.
        sounds: Shared<FakeLocalSound>,
        /// Store.
        store: Shared<FakeStore>,
        /// Registry.
        registry: Shared<FakeRegistry>,
        /// Resources.
        resources: Shared<FakeResources>,
    }

    /// Build a world.
    fn world(product: Product) -> World {
        let state = shared(ClientGameState::new(product));
        let static_state = shared(ClientGameStaticState::new(product));
        static_state.borrow_mut().maxclients = 64;
        let sink: Shared<FakeSink> = shared(FakeSink::default());
        let queue: Shared<dyn HudDrawSink> = sink.clone();
        let draw = Draw2D::new(queue, CoordinateSpace::Stretch640, 640, 480);
        let resources: Shared<FakeResources> = shared(FakeResources::default());
        let bank: Shared<dyn ClientSoundBank> = shared(FakeSoundBank::default());
        let registry: Shared<FakeRegistry> = shared(FakeRegistry::default());
        let catalog: Shared<dyn HudItemCatalog> = shared(FakeCatalog);
        let media = shared(ClientMedia::new(
            product,
            static_state.clone(),
            resources.clone(),
            bank,
            registry.clone(),
            catalog,
            ClientGraphics::new(),
            ClientSounds::default(),
        ));
        let tools = ClientDrawTools::new(draw.clone(), media.clone());
        let icons = shared(ClientDrawIcons::new(
            state.clone(),
            tools.clone(),
            Rc::new(|| ClientDrawIconSettings {
                draw_icons: true,
                draw_3d_icons: true,
            }),
            draw.clone(),
        ));
        let slots = static_state.borrow().client_info.clone();
        World {
            state: state.clone(),
            static_state: static_state.clone(),
            media,
            sink,
            draw,
            tools,
            icons,
            cvars: shared(FakeCvars::new()),
            strings: shared(FakeStrings::default()),
            commands: shared(FakeCommands::default()),
            sounds: shared(FakeLocalSound::default()),
            store: shared(FakeStore {
                state,
                slots,
                loads: Cell::new(0),
            }),
            registry,
            resources,
        }
    }

    #[test]
    fn product_spellings() {
        assert_eq!(Product::Baseq3.as_str(), "baseq3");
        assert!(Product::Missionpack.is_missionpack());
        assert!(!Product::Baseq3.is_missionpack());
    }

    #[test]
    fn game_type_ordering_matches_source_comparisons() {
        assert!(GameType::Team < GameType::Ctf);
        assert!(GameType::Tournament < GameType::Team);
        assert_eq!(GameType::from_i32(4), Some(GameType::Ctf));
        assert_eq!(GameType::from_i32(99), None);
        assert_eq!(Team::from_i32(3), Some(Team::Spectator));
        assert_eq!(Powerup::from_i32(9), Some(Powerup::NeutralFlag));
        assert_eq!(WeaponState::from_i32(3), Some(WeaponState::Firing));
        assert_eq!(MoveType::from_i32(5), Some(MoveType::Intermission));
        assert_eq!(PersistentIndex::Rank as i32, 2);
    }

    #[test]
    fn stat_schema_slots() {
        let base = stat_schema(Product::Baseq3);
        assert_eq!(base.weapons, BaseStatIndex::Weapons as i32);
        assert_eq!(base.max_health, 6);
        let mission = stat_schema(Product::Missionpack);
        assert_eq!(mission.weapons, MissionpackStatIndex::Weapons as i32);
        assert_eq!(mission.max_health, 7);
        assert!((ARMOR_PROTECTION - 0.66).abs() < f64::EPSILON);
    }

    #[test]
    fn game_format_scoreboard_shapes() {
        assert_eq!(
            game_format(
                "%5i %4i %4i %s",
                &[
                    GameFormatArg::Int(1),
                    GameFormatArg::Int(2),
                    GameFormatArg::Int(3),
                    GameFormatArg::Text("name".to_string())
                ],
                1024
            ),
            "    1    2    3 name"
        );
        assert_eq!(
            game_format(
                " SPECT %3i %4i %s",
                &[
                    GameFormatArg::Int(5),
                    GameFormatArg::Int(6),
                    GameFormatArg::Text("x".to_string())
                ],
                1024
            ),
            " SPECT   5    6 x"
        );
        assert_eq!(
            game_format(
                "%i:%i%i",
                &[GameFormatArg::Int(1), GameFormatArg::Int(2), GameFormatArg::Int(3)],
                1024
            ),
            "1:23"
        );
        assert_eq!(game_format("%2i", &[GameFormatArg::Int(7)], 1024), " 7");
        assert_eq!(game_format("100%%", &[], 1024), "100%");
        assert_eq!(game_format("abcdef", &[], 4), "abc");
        assert_eq!(
            game_format("%s", &[GameFormatArg::Text("toolong".to_string())], 4),
            "too"
        );
    }

    #[test]
    fn game_atoi_matches_bg_lib() {
        assert_eq!(game_atoi("  -42"), -42);
        assert_eq!(game_atoi("12abc"), 12);
        assert_eq!(game_atoi(""), 0);
        assert_eq!(game_atoi("   "), 0);
        assert_eq!(game_atoi("+7"), 7);
        assert_eq!(game_atoi("9999999999"), 9999999999i64 as i32);
        assert_eq!(game_atoi("-\0"), 0);
    }

    #[test]
    fn game_atof_matches_bg_lib() {
        assert!((game_atof("3.5") - f64::from(3.5f32)).abs() < 1e-6);
        assert!((game_atof(".5") - f64::from(0.5f32)).abs() < 1e-6);
        assert!((game_atof("-2.25") - f64::from(-2.25f32)).abs() < 1e-6);
        assert_eq!(game_atof(""), 0.0);
        assert_eq!(game_atof("abc"), 0.0);
        assert_eq!(game_atof("42"), f64::from(42.0f32));
    }

    #[test]
    fn info_string_lookup() {
        let info = "\\mapname\\q3dm1\\n\\sarge";
        assert_eq!(info_value_for_key(info, "mapname", 8192), "q3dm1");
        assert_eq!(info_value_for_key(info, "N", 8192), "sarge");
        assert_eq!(info_value_for_key(info, "missing", 8192), "");
        assert_eq!(info_value_for_key("\\a\\b\\c\\d", "c", 8192), "d");
    }

    #[test]
    fn place_string_ranks() {
        assert_eq!(place_string(1), "^41st^7");
        assert_eq!(place_string(2), "^12nd^7");
        assert_eq!(place_string(3), "^33rd^7");
        assert_eq!(place_string(4), "4th");
        assert_eq!(place_string(11), "11th");
        assert_eq!(place_string(21), "21st");
        assert_eq!(place_string(0x4000 | 1), "Tied for ^41st^7");
    }

    #[test]
    fn game_random_is_deterministic() {
        let mut random = GameRandom::new(0);
        assert_eq!(random.rand(), 1);
        assert_eq!(random.seed(), q_rand(0));
        let mut other = GameRandom::new(12345);
        let first = other.random();
        other.reset(12345);
        assert_eq!(other.random(), first);
        other.reset(7);
        let positive = other.random();
        assert!((0.0..1.0).contains(&positive));
    }

    #[test]
    fn qvm_axes_zero_angles() {
        let axis = qvm_angles_to_axis(vec3(0.0, 0.0, 0.0));
        assert_eq!(axis[0], vec3(1.0, 0.0, 0.0));
        assert_eq!(axis[1], vec3(-0.0, 1.0, 0.0));
        assert_eq!(axis[2], vec3(0.0, -0.0, 1.0));
    }

    #[test]
    fn draw_strlen_skips_escapes() {
        assert_eq!(draw_strlen("abc"), 3);
        assert_eq!(draw_strlen("^1ab"), 2);
        assert_eq!(draw_strlen("^^"), 2);
        assert_eq!(draw_strlen("ab\0cd"), 2);
        assert_eq!(proportional_size_scale(UI_SMALLFONT), 0.75);
        assert_eq!(proportional_size_scale(0), 1.0);
    }

    #[test]
    fn fade_color_windows() {
        assert_eq!(fade_color(100, 0, 3000.0), None);
        assert_eq!(fade_color(5000, 1000, 3000.0), None);
        assert_eq!(fade_color(1500, 1000, 3000.0), Some(vec4(1.0, 1.0, 1.0, 1.0)));
        assert_eq!(fade_color(3950, 1000, 3000.0).unwrap().w, 50.0 / 200.0);
    }

    #[test]
    fn team_and_health_colors() {
        assert_eq!(team_color(Team::Red as i32), vec4(1.0, 0.2, 0.2, 1.0));
        assert_eq!(team_color(Team::Blue as i32), vec4(0.2, 0.2, 1.0, 1.0));
        assert_eq!(team_color(Team::Spectator as i32), vec4(0.7, 0.7, 0.7, 1.0));
        assert_eq!(team_color(99), vec4(1.0, 1.0, 1.0, 1.0));
        assert_eq!(get_color_for_health(0, 0), vec4(0.0, 0.0, 0.0, 1.0));
        assert_eq!(get_color_for_health(100, 100), vec4(1.0, 1.0, 1.0, 1.0));
        let mid = get_color_for_health(50, 0);
        assert_eq!(mid, vec4(1.0, 20.0 / 30.0, 0.0, 1.0));
    }

    #[test]
    fn atlas_widths() {
        assert_eq!(proportional_string_width("A"), 18);
        assert_eq!(banner_string_width("A"), 33);
        assert_eq!(proportional_string_width("a"), proportional_string_width("A"));
    }

    #[test]
    fn scalable_text_metrics() {
        let mut font = zero_font();
        font.glyph_scale = 1.0;
        for glyph in &mut font.glyphs {
            glyph.metrics.x_skip = 10;
            glyph.metrics.height = 12;
        }
        let fonts = FontSet {
            small: zero_font(),
            normal: font,
            big: zero_font(),
            profile: FontProfile::Cgame,
            small_threshold: 0.0,
            big_threshold: 99.0,
        };
        assert_eq!(text_width(&fonts, "AB", 1.0, 0), 20);
        assert_eq!(text_width(&fonts, "^1AB", 1.0, 0), 20);
        assert_eq!(text_width(&fonts, "ABC", 1.0, 2), 20);
        assert_eq!(text_height(&fonts, "AB", 1.0, 0), 12);
    }

    #[test]
    fn cvar_table_shapes() {
        let base = cvar_table(Product::Baseq3);
        let mission = cvar_table(Product::Missionpack);
        assert_eq!(base.len(), 89);
        assert_eq!(mission.len(), 101);
        let defer = base
            .iter()
            .find(|entry| entry.symbol == ClientVmCvarSymbol::CgDeferPlayers)
            .unwrap();
        assert_eq!(defer.default_value, "1");
        let defer = mission
            .iter()
            .find(|entry| entry.symbol == ClientVmCvarSymbol::CgDeferPlayers)
            .unwrap();
        assert_eq!(defer.default_value, "0");
        let marks = base
            .iter()
            .find(|entry| entry.symbol == ClientVmCvarSymbol::CgAddMarks)
            .unwrap();
        assert_eq!(marks.name, "cg_marks");
        let overlay = base
            .iter()
            .find(|entry| entry.symbol == ClientVmCvarSymbol::CgTeamOverlayUserinfo)
            .unwrap();
        assert_eq!(overlay.flags, cvar_flags::READ_ONLY | cvar_flags::USER_INFO);
        let red = mission
            .iter()
            .find(|entry| entry.symbol == ClientVmCvarSymbol::CgRedTeamName)
            .unwrap();
        assert_eq!(red.default_value, "Stroggs");
    }

    #[test]
    fn configuration_register_read_update() {
        let cvars = shared(CvarRegistry::new(Dialect::Q3));
        let state = shared(ClientGameState::new(Product::Missionpack));
        let static_state = shared(ClientGameStaticState::new(Product::Missionpack));
        let store: Shared<dyn ClientInfoStore> = shared(FakeStore {
            state: state.clone(),
            slots: static_state.borrow().client_info.clone(),
            loads: Cell::new(0),
        });
        let strings: Shared<dyn HudConfigStrings> = shared(FakeStrings::default());
        let configuration = ClientConfiguration::new(
            Product::Missionpack,
            ClientConfigurationHost {
                cvars: cvars.clone(),
                state,
                static_state,
                clients: store,
                strings,
                status_visible: None,
            },
        );
        configuration.register_cvars();
        assert_eq!(configuration.read_vm_cvar("cg_fov").value, "90");
        assert_eq!(configuration.read_vm_cvar("CG_FOV").value, "90");
        cvars.borrow_mut().set("cg_fov", "110", true).unwrap();
        configuration.update_cvars();
        assert_eq!(configuration.read_vm_cvar("cg_fov").value, "110");
        configuration.set_vm_integer(ClientVmCvarSymbol::CgCurrentSelectedPlayer, 0);
    }

    #[test]
    fn configuration_status_override() {
        let cvars = shared(CvarRegistry::new(Dialect::Q3));
        let state = shared(ClientGameState::new(Product::Baseq3));
        let static_state = shared(ClientGameStaticState::new(Product::Baseq3));
        let store: Shared<dyn ClientInfoStore> = shared(FakeStore {
            state: state.clone(),
            slots: static_state.borrow().client_info.clone(),
            loads: Cell::new(0),
        });
        let strings: Shared<dyn HudConfigStrings> = shared(FakeStrings::default());
        let configuration = ClientConfiguration::new(
            Product::Baseq3,
            ClientConfigurationHost {
                cvars,
                state,
                static_state,
                clients: store,
                strings,
                status_visible: Some(Rc::new(|| false)),
            },
        );
        configuration.register_cvars();
        let status = configuration.read_vm_cvar("cg_drawStatus");
        assert_eq!(status.value, "0");
        assert_eq!(status.integer_value, 0);
    }

    #[test]
    fn client_info_animations_and_copy() {
        let mut info = ClientInfo::default();
        let mut rows = vec![None; ANIMATION_COUNT];
        for (index, row) in rows.iter_mut().enumerate() {
            if index != ANIMATION_SENTINEL {
                *row = Some(AnimationCell {
                    first_frame: index as i32,
                    num_frames: 2,
                    ..AnimationCell::default()
                });
            }
        }
        info.set_animations(&rows);
        assert_eq!(info.animations[0].first_frame, 0);
        assert_eq!(info.animations[ANIMATION_SENTINEL].first_frame, 0);
        info.name = "sarge".to_string();
        let mut other = ClientInfo::default();
        other.copy_from(&info);
        assert_eq!(other.name, "sarge");
        assert_eq!(other.animations[5].first_frame, 5);
    }

    #[test]
    #[should_panic(expected = "Client animation table has the wrong length")]
    fn client_info_animation_length() {
        ClientInfo::default().set_animations(&[None]);
    }

    #[test]
    fn frame_audio_buffering() {
        let state = shared(ClientGameState::new(Product::Baseq3));
        let host: Shared<FakeFrameAudio> = shared(FakeFrameAudio::default());
        let audio = ClientFrameAudio::new(state.clone(), Some(pcm_sound("wear")), host.clone());
        audio.add_buffered_sound(Some(pcm_sound("a")));
        state.borrow_mut().time = 100;
        audio.play_buffered_sounds();
        assert_eq!(host.borrow().local, vec![7]);
        assert_eq!(state.borrow().sound_time, 850);
        audio.play_buffered_sounds();
        assert_eq!(host.borrow().local.len(), 1);
    }

    #[test]
    fn frame_audio_powerup_tick() {
        let state = shared(ClientGameState::new(Product::Baseq3));
        let mut ps = PlayerState::new(Product::Baseq3);
        ps.powerups.set(Powerup::Quad as i32, 4500);
        state.borrow_mut().snap = Some(Snapshot {
            server_time: 3000,
            player_state: ps,
        });
        state.borrow_mut().time = 3000;
        state.borrow_mut().old_time = 1500;
        let host: Shared<FakeFrameAudio> = shared(FakeFrameAudio::default());
        ClientFrameAudio::new(state, Some(pcm_sound("wear")), host.clone()).powerup_timer_sounds();
        assert_eq!(host.borrow().placed.len(), 1);
    }

    #[test]
    fn sound_bank_checkpoint_round_trip() {
        let bank: Shared<dyn SoundBank> = shared(FakeEngineBank::default());
        let mut sounds = Q3PresentationSoundBank::new(bank, None, Box::new(|_, _| None));
        let first = sounds.register_sound(Some("sound/a.wav"), false).unwrap();
        assert_eq!(sounds.index_for_sound(&Some(first.clone())), 1);
        assert_eq!(sounds.index_for_sound(&None), 0);
        assert_eq!(sounds.register_sound(Some(""), false), None);
        assert_eq!(sounds.register_sound(Some("*null"), false), None);
        let rows = sounds.capture_checkpoint();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].handle, 1);
        let bank: Shared<dyn SoundBank> = shared(FakeEngineBank::default());
        let mut restored = Q3PresentationSoundBank::new(bank, None, Box::new(|_, _| None));
        restored.restore_checkpoint(&rows);
        assert_eq!(restored.registrations().len(), 1);
    }

    #[test]
    fn console_command_tables() {
        assert_eq!(local_command_names(Product::Baseq3).len(), 23);
        assert_eq!(local_command_names(Product::Missionpack).len(), 47);
        assert_eq!(client_console_command_names(Product::Baseq3).len(), 50);
        assert_eq!(client_console_command_names(Product::Missionpack).len(), 74);
        assert!(COMMON_COMMANDS.contains(&"tcmd"));
        assert!(MISSION_COMMANDS.contains(&"loadhud"));
        assert!(FORWARDED_COMMANDS.contains(&"teamtask"));
    }

    /// Canned console host.
    struct FakeConsoleHost {
        /// Cvars.
        cvars: Shared<CvarRegistry>,
        /// View.
        view: Shared<FakeView>,
        /// Weapons.
        weapons: Shared<FakeWeapons>,
        /// Store.
        store: Shared<FakeStore>,
        /// Server.
        server: Shared<FakeServerCommands>,
        /// Commands.
        commands: Shared<FakeCommands>,
        /// Reading cvars.
        reader: Shared<FakeCvars>,
        /// Buffered.
        buffered: Vec<Option<PcmSound>>,
        /// Center prints.
        center: Vec<String>,
    }

    impl ClientConsoleHost for FakeConsoleHost {
        fn cvars(&self) -> Shared<CvarRegistry> {
            self.cvars.clone()
        }
        fn view(&self) -> Shared<dyn ViewService> {
            self.view.clone()
        }
        fn weapons(&self) -> Shared<dyn WeaponService> {
            self.weapons.clone()
        }
        fn clients(&self) -> Shared<dyn ClientInfoStore> {
            self.store.clone()
        }
        fn server_commands(&self) -> Shared<dyn ServerCommandService> {
            self.server.clone()
        }
        fn hud(&self) -> ConsoleHudAccess {
            ConsoleHudAccess::Unavailable {
                reason: "test".to_string(),
            }
        }
        fn team_orders(&self) -> ConsoleOrdersAccess {
            ConsoleOrdersAccess::Unavailable {
                reason: "test".to_string(),
            }
        }
        fn read_vm_cvar(&self, name: &str) -> CvarSnapshot {
            self.reader.borrow().read_vm_cvar(name)
        }
        fn reset_player_entity(&mut self, _entity: &mut ClientEntity) {}
        fn add_command(&mut self, name: &str) {
            self.commands.borrow_mut().add_command(name);
        }
        fn send_client_command(&mut self, text: &str) {
            self.commands.borrow_mut().send_client_command(text);
        }
        fn send_console_command(&mut self, text: &str) {
            self.commands.borrow_mut().send_console_command(text);
        }
        fn print(&mut self, text: &str) {
            self.commands.borrow_mut().print(text);
        }
        fn center_print(&mut self, text: &str, _y: i32, _char_width: i32) {
            self.center.push(text.to_string());
        }
        fn sound(&self, _name: MenuEndSound) -> Option<PcmSound> {
            Some(pcm_sound("winner"))
        }
        fn add_buffered_sound(&mut self, sound: Option<PcmSound>) {
            self.buffered.push(sound);
        }
    }

    /// Build a console runtime.
    fn console_fixture() -> (
        ClientConsoleRuntime,
        Shared<FakeCommands>,
        Shared<FakeWeapons>,
        Shared<FakeView>,
    ) {
        let game = world(Product::Baseq3);
        let view: Shared<FakeView> = shared(FakeView {
            state: game.state.clone(),
            calls: Vec::new(),
        });
        let weapons: Shared<FakeWeapons> = shared(FakeWeapons {
            state: game.state.clone(),
            registry: game.registry.clone(),
            selected: Vec::new(),
        });
        let host: Shared<dyn ClientConsoleHost> = shared(FakeConsoleHost {
            cvars: shared(CvarRegistry::new(Dialect::Q3)),
            view: view.clone(),
            weapons: weapons.clone(),
            store: game.store,
            server: shared(FakeServerCommands::default()),
            commands: game.commands.clone(),
            reader: game.cvars,
            buffered: Vec::new(),
            center: Vec::new(),
        });
        let runtime = ClientConsoleRuntime::new(game.state, game.static_state, host);
        runtime.initialize_commands();
        (runtime, game.commands, weapons, view)
    }

    #[test]
    fn console_execute_paths() {
        let (runtime, commands, weapons, view) = console_fixture();
        assert_eq!(commands.borrow().added.len(), 50);
        assert!(runtime.handles("viewpos"));
        assert!(!runtime.handles("kill"));
        assert!(runtime.execute(&["viewpos".to_string()]).unwrap());
        assert_eq!(commands.borrow().printed.len(), 1);
        assert!(!runtime.execute(&["kill".to_string()]).unwrap());
        assert!(runtime.execute(&["weapon".to_string(), "2".to_string()]).unwrap());
        assert_eq!(weapons.borrow().selected, vec![2]);
        assert!(runtime.execute(&["nextframe".to_string()]).unwrap());
        assert!(view.borrow().calls.contains(&"nextframe".to_string()));
        let big = vec!["x".to_string(); 1025];
        assert!(runtime.execute(&big).is_err());
    }

    #[test]
    fn console_scores_and_tcmd() {
        let (runtime, commands, _, _) = console_fixture();
        runtime.state.borrow_mut().time = 5000;
        assert!(runtime.execute(&["+scores".to_string()]).unwrap());
        assert!(runtime.state.borrow().show_scores);
        assert_eq!(commands.borrow().client, vec!["score".to_string()]);
        assert!(runtime.execute(&["-scores".to_string()]).unwrap());
        assert!(!runtime.state.borrow().show_scores);
        runtime.state.borrow_mut().crosshair_client_num = 3;
        runtime.state.borrow_mut().crosshair_client_time = 5000;
        assert!(runtime.execute(&["tcmd".to_string(), "2".to_string()]).unwrap());
        assert!(commands.borrow().console.iter().any(|line| line.starts_with("gc 3 2")));
    }

    #[test]
    fn scoreboard_draw_paths() {
        let game = world(Product::Baseq3);
        game.cvars.borrow_mut().set("cg_paused", 1, 1.0, "1");
        let board = BaseScoreboard::new(
            game.state.clone(),
            game.static_state.clone(),
            BaseScoreboardHost {
                icons: game.icons.clone(),
                clients: game.store.clone(),
                players: shared(FakePresenter {
                    state: game.state.clone(),
                }),
                cvars: game.cvars.clone(),
                strings: game.strings.clone(),
                commands: game.commands.clone(),
            },
        );
        assert!(!board.draw());
        game.cvars.borrow_mut().set("cg_paused", 0, 0.0, "0");
        game.cvars.borrow_mut().set("cg_drawIcons", 1, 1.0, "1");
        game.state.borrow_mut().snap = Some(Snapshot {
            server_time: 100,
            player_state: PlayerState::new(Product::Baseq3),
        });
        game.state.borrow_mut().show_scores = true;
        assert!(board.draw());
        assert!(!game.sink.borrow().blits.is_empty());
        game.state.borrow_mut().time = 5000;
        board.draw_tourney();
        assert!(game.commands.borrow().client.contains(&"score".to_string()));
    }

    #[test]
    fn corner_field_clamps() {
        let game = world(Product::Baseq3);
        let corners = ClientHudCorners::new(
            game.state.clone(),
            game.static_state.clone(),
            game.icons.clone(),
            ClientHudCornersHost {
                cvars: game.cvars.clone(),
                strings: game.strings.clone(),
                clock: shared(FakeClock { time: 100 }),
            },
        );
        corners.draw_field(0.0, 0.0, 3, 9999);
        assert_eq!(game.sink.borrow().blits.len(), 3);
        game.sink.borrow_mut().blits.clear();
        corners.draw_field(0.0, 0.0, 1, 42);
        assert_eq!(game.sink.borrow().blits.len(), 1);
        corners.draw_field(0.0, 0.0, 0, 42);
        assert_eq!(game.sink.borrow().blits.len(), 1);
    }

    #[test]
    fn status_center_print_and_lagometer() {
        let game = world(Product::Baseq3);
        let status = ClientDrawStatus::new(
            game.state.clone(),
            game.static_state.clone(),
            game.tools.clone(),
            ClientDrawStatusVariant::Baseq3,
            ClientDrawStatusHost {
                commands: shared(FakeCommandSource::default()),
                cvars: game.cvars.clone(),
            },
        );
        game.state.borrow_mut().time = 10;
        status.center_print("a\nb", 100, 8);
        assert_eq!(game.state.borrow().center_print_lines, 2);
        game.cvars.borrow_mut().set("cg_centertime", 3, 3.0, "3");
        status.draw_center_string();
        assert!(!game.sink.borrow().blits.is_empty());
        status.add_lagometer_frame_info();
        status.add_lagometer_snapshot_info(Some(LagometerSnapshotSample { ping: 50, flags: 0 }));
        status.add_lagometer_snapshot_info(None);
        assert_eq!(status.frame_count.get(), 1);
        assert_eq!(status.snapshot_count.get(), 2);
    }

    #[test]
    fn model_painter_happy_path() {
        let game = world(Product::Baseq3);
        let painter = EngineUiModelPainter::new(game.resources.clone(), game.draw.clone());
        painter.paint(&UiModelPaintRequest {
            draw: game.draw.clone(),
            model: SceneModel::Loaded { path: "m".to_string() },
            rect: rect2d(0.0, 0.0, 100.0, 100.0),
            time: 5,
            angle: 10.0,
            field_of_view_x: 0.0,
            field_of_view_y: 60.0,
        });
        assert_eq!(game.resources.borrow().scenes, vec!["clear", "add", "render"]);
    }

    #[test]
    #[should_panic(expected = "UI model painter must use its seat's drawing queue")]
    fn model_painter_queue_mismatch() {
        let game = world(Product::Baseq3);
        let other: Shared<dyn HudDrawSink> = shared(FakeSink::default());
        let painter = EngineUiModelPainter::new(game.resources.clone(), game.draw.clone());
        painter.paint(&UiModelPaintRequest {
            draw: Draw2D::new(other, CoordinateSpace::Stretch640, 640, 480),
            model: SceneModel::Default,
            rect: rect2d(0.0, 0.0, 10.0, 10.0),
            time: 0,
            angle: 0.0,
            field_of_view_x: 0.0,
            field_of_view_y: 0.0,
        });
    }

    #[test]
    fn hud_follow_and_warmup() {
        let game = world(Product::Baseq3);
        let status = shared(ClientDrawStatus::new(
            game.state.clone(),
            game.static_state.clone(),
            game.tools.clone(),
            ClientDrawStatusVariant::Baseq3,
            ClientDrawStatusHost {
                commands: shared(FakeCommandSource::default()),
                cvars: game.cvars.clone(),
            },
        ));
        let corners = shared(ClientHudCorners::new(
            game.state.clone(),
            game.static_state.clone(),
            game.icons.clone(),
            ClientHudCornersHost {
                cvars: game.cvars.clone(),
                strings: game.strings.clone(),
                clock: shared(FakeClock { time: 0 }),
            },
        ));
        let board = shared(BaseScoreboard::new(
            game.state.clone(),
            game.static_state.clone(),
            BaseScoreboardHost {
                icons: game.icons.clone(),
                clients: game.store.clone(),
                players: shared(FakePresenter {
                    state: game.state.clone(),
                }),
                cvars: game.cvars.clone(),
                strings: game.strings.clone(),
                commands: game.commands.clone(),
            },
        ));
        let hud = ClientHud::new(
            game.state.clone(),
            game.static_state.clone(),
            ClientHudHost {
                weapon_hud: None,
                icons: game.icons.clone(),
                status,
                corners,
                prediction: shared(FakePrediction {
                    state: game.state.clone(),
                    trace: PredictionTrace {
                        entity_num: 99,
                        end: vec3(0.0, 0.0, 0.0),
                    },
                    contents: 0,
                }),
                weapons: shared(FakeWeapons {
                    state: game.state.clone(),
                    registry: game.registry.clone(),
                    selected: Vec::new(),
                }),
                random: shared(GameRandom::new(3)),
                cvars: game.cvars.clone(),
                sounds: game.sounds.clone(),
            },
            ClientHudVariant::Baseq3 { scoreboard: board },
        );
        game.state.borrow_mut().snap = Some(Snapshot {
            server_time: 10,
            player_state: PlayerState::new(Product::Baseq3),
        });
        assert!(!hud.draw_follow());
        game.cvars.borrow_mut().set("cg_drawAmmoWarning", 1, 1.0, "1");
        game.state.borrow_mut().low_ammo_warning = 1;
        hud.draw_ammo_warning();
        assert!(!game.sink.borrow().blits.is_empty());
        game.state.borrow_mut().warmup = -1;
        hud.draw_warmup();
        assert_eq!(game.state.borrow().warmup_count, 0);
        hud.scan_for_crosshair_entity();
        assert_eq!(game.state.borrow().crosshair_client_num, 0);
    }

    #[test]
    fn owner_draw_ids_flags_and_values() {
        assert_eq!(MissionOwnerDrawId::PlayerHead as i32, 3);
        assert_eq!(MissionOwnerDrawId::from_i32(69), Some(MissionOwnerDrawId::Captures));
        assert_eq!(MissionOwnerDrawId::from_i32(13), None);
        assert_eq!(owner_draw_flags::SHOW_2DONLY, 0x10000000);
        assert_eq!(MissionScoreFeeder::Scoreboard as i32, 11);
        let game = world(Product::Missionpack);
        game.static_state.borrow_mut().game_type = GameType::Ctf;
        game.static_state.borrow_mut().blueflag = 1;
        let mut ps = PlayerState::new(Product::Missionpack);
        ps.persistant.set(PersistentIndex::Team as i32, Team::Red as i32);
        ps.stats.set(MissionpackStatIndex::Health as i32, 80);
        ps.stats.set(MissionpackStatIndex::Armor as i32, 25);
        game.state.borrow_mut().snap = Some(Snapshot {
            server_time: 1,
            player_state: ps,
        });
        let configuration = shared(ClientConfiguration::new(
            Product::Missionpack,
            ClientConfigurationHost {
                cvars: shared(CvarRegistry::new(Dialect::Q3)),
                state: game.state.clone(),
                static_state: game.static_state.clone(),
                clients: game.store.clone(),
                strings: game.strings.clone(),
                status_visible: None,
            },
        ));
        configuration.borrow().register_cvars();
        let owner = MissionOwnerDraw::new(
            game.state.clone(),
            game.static_state.clone(),
            game.media.clone(),
            MissionOwnerDrawHost {
                weapon_hud: None,
                icons: game.icons.clone(),
                fonts: shared(zero_cgame_fonts()),
                configuration: configuration.clone(),
                random: shared(GameRandom::new(1)),
                strings: game.strings.clone(),
                selected_player: Rc::new(|| 0),
                chat: Rc::new(HudChatText::default),
            },
        );
        assert!(owner.your_team_has_flag());
        assert!(!owner.other_team_has_flag());
        assert!(owner.visible(owner_draw_flags::SHOW_YOURTEAMHASENEMYFLAG));
        assert!(owner.visible(owner_draw_flags::SHOW_ANYTEAMGAME));
        assert!(owner.visible(owner_draw_flags::SHOW_HEALTHOK));
        assert!(!owner.visible(owner_draw_flags::SHOW_HEALTHCRITICAL));
        assert_eq!(owner.value(MissionOwnerDrawId::PlayerHealth as i32), 80.0);
        assert_eq!(owner.value(MissionOwnerDrawId::PlayerArmorValue as i32), 25.0);
        assert_eq!(owner.value(999), -1.0);
        assert!(owner.status_handle(2).is_none() || owner.status_handle(99).is_none());
        assert_eq!(owner.width(MissionOwnerDrawId::GameType as i32, 1.0), 0.0);
    }
}
