//! Quake III presentation HUD, draw helpers, and Team Arena menus.
//!
//! Donor provenance: `src/content/q3/presentation/` (`index.ts`,
//! `ui-adapters.ts`, `frame-audio.ts`, `client-info.ts`, `draw-icons.ts`,
//! `audio.ts`, `info.ts`, `draw-tools.ts`, `draw-status.ts`, `scoreboard.ts`,
//! `console.ts`, `hud.ts`, `config.ts`, `hud-corners.ts`,
//! `mission-owner-draw.ts`, `mission-hud.ts`), ported from id Software's
//! `code/cgame` (`cg_draw.c`, `cg_drawtools.c`, `cg_info.c`, `cg_main.c`,
//! `cg_newdraw.c`, `cg_scoreboard.c`, `cg_consolecmds.c`, `cg_view.c`).
//!
//! Adaptation notes (all language-level, behavior-preserving):
//! * Donor `async` resource calls are synchronous trait calls; ordering,
//!   generation guards, and error text are unchanged (there is no async
//!   executor in this crate).
//! * Donor `throw` in `void` methods is a panic carrying the donor message;
//!   only [`ClientConsoleRuntime::execute`] returns `Result`, matching the
//!   donor's promise rejection contract.
//! * Donor object identity (`!==` on shared services) is [`Shared`] pointer
//!   identity ([`std::rc::Rc::ptr_eq`]).
//! * `Draw2D` is a value wrapper over a shared pixel sink; identity checks
//!   compare the shared queue.
//! * The free-function item catalog (`findItemForPowerup`, `itemList`,
//!   `itemAt`) lives on [`ClientMedia`] as [`HudItemCatalog`]; the table
//!   data itself is owned by the items lane.
//! * `index.ts` is a pure re-export barrel for the whole presentation
//!   layer; its only HUD-overlapping items (`Q3PresentationAudio`,
//!   `Q3PresentationSoundBank`, `SoundRegistration`, `Q3AudioTarget`) are
//!   ported here from `audio.ts`, so the barrel itself carries nothing.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use qa_core::cvar::{flags as cvar_flags, CvarRegistry, CvarSnapshot};
use qa_core::math::{add3, vec3, vec4, Axis, Bounds, Vec3, Vec4};
use qa_core::numeric::{q_rand, qvm_float_to_int};

use crate::q3anim::{PlayerFootsteps, PlayerGender};

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
struct NumberCursor {
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
    if maximum_length < 1 || maximum_length > 8192 {
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
        let mut separator = None;
        for index in cursor..units.len() {
            if units[index] == '\\' {
                separator = Some(index);
                break;
            }
        }
        let Some(separator) = separator else {
            return String::new();
        };
        let mut next = None;
        for index in separator + 1..units.len() {
            if units[index] == '\\' {
                next = Some(index);
                break;
            }
        }
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
const ESCAPE_COLORS: [Vec4; 8] = [
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
fn escape_at(units: &[char], index: usize) -> bool {
    units.get(index) == Some(&'^') && index + 1 < units.len() && units[index + 1] != '^'
}

/// Resolve an escape color (`escapeColor`).
fn escape_color(code: u32, alpha: f32) -> Vec4 {
    let color = ESCAPE_COLORS[((code.wrapping_sub(48)) & 7) as usize];
    Vec4 { w: alpha, ..color }
}

/// Black with an alpha (`black`).
fn black(alpha: f32) -> Vec4 {
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
const INVALID_METRIC: AtlasMetric = [0, 0, -1];

/// Proportional ASCII atlas (`PROP_ASCII`).
const PROP_ASCII: [AtlasMetric; 65] = [
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
const PROP_END: [AtlasMetric; 4] = [[153, 152, 13], [11, 181, 5], [180, 152, 13], [79, 93, 17]];

/// Banner atlas (`BANNER`).
const BANNER: [AtlasMetric; 26] = [
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
fn banner_metric(code: u32) -> AtlasMetric {
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
fn aligned_x(x: f32, width: i32, style: i32) -> i32 {
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
fn pulse(time: i32) -> f32 {
    0.5 + 0.5 * ((time / 75) as f32).sin()
}

/// Select a font by scale (`selectFont`).
fn select_font<'a>(fonts: &'a FontSet, scale: f32) -> &'a RegisteredFont {
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
fn glyph_at(font: &RegisteredFont, code: u32) -> &RegisteredGlyph {
    font.glyphs
        .get(code as usize)
        .unwrap_or_else(|| panic!("Missing font glyph {code}"))
}

/// Text metric core (`textMetric`).
fn text_metric(fonts: &FontSet, input: &str, scale: f32, limit: i32, height: bool) -> i32 {
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
fn atlas_pass_cgame(draw: &Draw2D, picture: Picture, text: &str, x: i32, y: i32, color: Vec4, size: f32, banner: bool) {
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
fn paint_glyph(draw: &Draw2D, glyph: &RegisteredGlyph, x: f32, baseline: f32, scale: f32) {
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

/// One stable `cgs.clientinfo` slot (`ClientInfo`).
#[derive(Debug, Clone, PartialEq)]
pub struct ClientInfo {
    /// Validity.
    pub info_valid: bool,
    /// Name.
    pub name: String,
    /// Team.
    pub team: Team,
    /// Bot skill.
    pub bot_skill: i32,
    /// Color 1.
    pub color1: Vec3,
    /// Color 2.
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
    /// Powerup bitmask.
    pub powerups: i32,
    /// Medkit usage time.
    pub medkit_usage_time: i32,
    /// Invulnerability start.
    pub invulnerability_start_time: i32,
    /// Invulnerability stop.
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
    /// Red team.
    pub red_team: String,
    /// Blue team.
    pub blue_team: String,
    /// Deferred.
    pub deferred: bool,
    /// New anims.
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
    /// Animation cells (37; index 31 is the sentinel gap).
    pub animations: [AnimationCell; ANIMATION_COUNT],
    /// Sounds (32).
    pub sounds: [Option<PcmSound>; 32],
}

impl Default for ClientInfo {
    fn default() -> Self {
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
            legs_model: SceneModel::Default,
            torso_model: SceneModel::Default,
            head_model: SceneModel::Default,
            legs_skin: None,
            torso_skin: None,
            head_skin: None,
            model_icon: None,
            animations: [AnimationCell::default(); ANIMATION_COUNT],
            sounds: std::array::from_fn(|_| None),
        }
    }
}

impl ClientInfo {
    /// Copy animation values into the stable cells (`setAnimations`).
    pub fn set_animations(&mut self, animations: &[Option<Animation>]) {
        if animations.len() != self.animations.len() {
            panic!("Client animation table has the wrong length");
        }
        for (index, source) in animations.iter().enumerate() {
            if source.is_none() && index == ANIMATION_SENTINEL {
                continue;
            }
            let Some(source) = source else {
                panic!("Missing client animation cell {index}");
            };
            self.animations[index] = *source;
        }
    }

    /// Copy all fields plus animation values (`copyFrom`).
    pub fn copy_from(&mut self, source: &ClientInfo) {
        *self = source.clone();
    }
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

/// Sound registration row (`SoundRegistration`).
#[derive(Debug, Clone, PartialEq)]
pub struct SoundRegistration {
    /// Path.
    pub path: String,
    /// Compressed intent.
    pub compressed: bool,
    /// Asset.
    pub sound: Option<SoundAsset>,
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

/// Cgame presentation sound bank (`Q3PresentationSoundBank`).
pub struct Q3PresentationSoundBank {
    /// Engine bank.
    pub bank: Shared<dyn SoundBank>,
    /// Zero sound.
    pub zero_sound: Option<SoundAsset>,
    /// Synchronous loader.
    pub load_sync: Box<dyn Fn(&str, bool) -> Option<SoundAsset>>,
    /// Registered assets.
    registered: Vec<SoundAsset>,
    /// In-flight registrations.
    operations: u32,
    /// Requests by path.
    requests: HashMap<String, SoundRegistration>,
}

impl Q3PresentationSoundBank {
    /// Assemble a bank.
    pub fn new(
        bank: Shared<dyn SoundBank>,
        zero_sound: Option<SoundAsset>,
        load_sync: Box<dyn Fn(&str, bool) -> Option<SoundAsset>>,
    ) -> Self {
        Self {
            bank,
            zero_sound,
            load_sync,
            registered: Vec::new(),
            operations: 0,
            requests: HashMap::new(),
        }
    }

    /// Whether a PCM is the zero sound.
    fn is_zero(&self, sound: &PcmSound) -> bool {
        self.zero_sound
            .as_ref()
            .is_some_and(|zero| Rc::ptr_eq(&zero.pcm, sound))
    }

    /// Map an asset to nullable PCM, hiding the zero sound.
    fn exposed(&self, sound: &Option<SoundAsset>) -> Option<PcmSound> {
        match sound {
            None => None,
            Some(asset) if self.is_zero(&asset.pcm) => None,
            Some(asset) => Some(asset.pcm.clone()),
        }
    }

    /// Track a fresh asset.
    fn track(&mut self, sound: &Option<SoundAsset>) {
        if let Some(asset) = sound {
            if !self.is_zero(&asset.pcm) && !self.registered.iter().any(|entry| Rc::ptr_eq(&entry.pcm, &asset.pcm)) {
                self.registered.push(asset.clone());
            }
        }
    }

    /// Capture a checkpoint (`captureCheckpoint`).
    pub fn capture_checkpoint(&self) -> Vec<SoundCheckpointRow> {
        if self.operations != 0 {
            panic!("Cannot checkpoint pending sound registration");
        }
        self.requests
            .values()
            .map(|row| SoundCheckpointRow {
                path: row.path.clone(),
                compressed: row.compressed,
                handle: self.index_for_sound(&self.exposed(&row.sound)),
                resource: row.sound.as_ref().and_then(|asset| asset.resource.clone()),
            })
            .collect()
    }

    /// Restore a checkpoint (`restoreCheckpoint`).
    pub fn restore_checkpoint(&mut self, rows: &[SoundCheckpointRow]) {
        if self.operations != 0 || !self.requests.is_empty() {
            panic!("Sound restore requires an empty owner");
        }
        for row in rows {
            let sound = self.register_sound(Some(&row.path), row.compressed);
            let entry = self.requests.get(&row.path);
            if self.index_for_sound(&sound) != row.handle
                || entry
                    .and_then(|entry| entry.sound.as_ref())
                    .and_then(|asset| asset.resource.clone())
                    != row.resource
            {
                panic!("sound resource binding changed");
            }
        }
    }
}

impl ClientSoundBank for Q3PresentationSoundBank {
    fn register_sound(&mut self, path: Option<&str>, compressed: bool) -> Option<PcmSound> {
        let Some(path) = path else {
            panic!("S_RegisterSound dereferences NULL name at strlen");
        };
        if path.is_empty() || path.starts_with('*') {
            return None;
        }
        if let Some(prior) = self.requests.get(path) {
            return self.exposed(&prior.sound.clone());
        }
        self.operations += 1;
        let sound = self.bank.borrow_mut().register(path, "q3");
        self.operations -= 1;
        self.requests.insert(
            path.to_string(),
            SoundRegistration {
                path: path.to_string(),
                compressed,
                sound: sound.clone(),
            },
        );
        self.track(&sound);
        self.exposed(&sound)
    }

    fn sound(&mut self, path: Option<&str>, compressed: bool) -> Option<PcmSound> {
        let Some(path) = path else {
            return None;
        };
        if !self.requests.contains_key(path) {
            let sound = (self.load_sync)(path, compressed);
            self.track(&sound);
            self.requests.insert(
                path.to_string(),
                SoundRegistration {
                    path: path.to_string(),
                    compressed,
                    sound,
                },
            );
        }
        let entry = self.requests.get(path).cloned().unwrap_or(SoundRegistration {
            path: path.to_string(),
            compressed,
            sound: None,
        });
        self.exposed(&entry.sound)
    }

    fn index_for_sound(&self, sound: &Option<PcmSound>) -> i32 {
        match sound {
            None => 0,
            Some(pcm) if self.is_zero(pcm) => 0,
            Some(pcm) => self
                .registered
                .iter()
                .position(|entry| Rc::ptr_eq(&entry.pcm, pcm))
                .map(|index| index as i32 + 1)
                .unwrap_or_else(|| panic!("PCM does not belong to this cgame sound bank")),
        }
    }

    fn asset(&self, sound: &Option<PcmSound>) -> Option<SoundAsset> {
        let index = self.index_for_sound(sound);
        if index == 0 {
            return self.zero_sound.clone();
        }
        self.registered
            .get(index as usize - 1)
            .cloned()
            .or_else(|| self.zero_sound.clone())
    }

    fn sound_at_index(&self, index: i32) -> Option<PcmSound> {
        if index == 0 {
            return None;
        }
        match self.registered.get(index as usize - 1) {
            Some(sound) if index > 0 => Some(sound.pcm.clone()),
            _ => panic!("Q3 sound handle {index} is not registered"),
        }
    }

    fn sound_for_index(&self, index: i32) -> Option<PcmSound> {
        if index == 0 {
            return None;
        }
        if index < 0 {
            return None;
        }
        self.registered.get(index as usize - 1).map(|asset| asset.pcm.clone())
    }

    fn registrations(&self) -> Vec<SoundRegistration> {
        self.requests.values().cloned().collect()
    }
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

/// Cgame audio target (`Q3AudioTarget`).
pub trait Q3AudioTarget {
    /// Viewing seat.
    fn seat(&self) -> SeatId;
    /// Sound bank.
    fn sounds(&self) -> Shared<dyn ClientSoundBank>;
    /// Actor for a source number.
    fn actor(&self, source: i32) -> ActorId;
    /// Frame number.
    fn frame_number(&self) -> i32;
    /// Play a one-shot.
    fn play(&mut self, sound: PlaySound);
    /// Play a loop.
    fn loop_sound(&mut self, sound: LoopSound);
    /// Update an actor position.
    fn update_actor(&mut self, actor: ActorId, position: Vec3);
    /// Stop an audience loop.
    fn stop_loop(&mut self, seat: SeatId, actor: ActorId);
}

/// Cgame one-shot and loop calls (`Q3PresentationAudio`).
pub struct Q3PresentationAudio {
    /// Target.
    pub target: Shared<dyn Q3AudioTarget>,
}

impl Q3PresentationAudio {
    /// Wrap a target.
    pub fn new(target: Shared<dyn Q3AudioTarget>) -> Self {
        Self { target }
    }

    /// Start a source sound (`startSourceSound`).
    pub fn start_source_sound(&self, sound: Option<PcmSound>, options: &StartSoundOptions) {
        let asset = self.target.borrow().sounds().borrow().asset(&sound);
        let Some(asset) = asset else {
            return;
        };
        let actor = if options.entity < 0 {
            None
        } else {
            Some(self.target.borrow().actor(options.entity))
        };
        let origin = match options.origin {
            StartSoundOrigin::Entity { entity } => SoundOrigin::Actor {
                actor: self.target.borrow().actor(entity),
            },
            StartSoundOrigin::Fixed { position } => SoundOrigin::Fixed { position },
            StartSoundOrigin::Local => SoundOrigin::Local,
        };
        let seat = self.target.borrow().seat();
        let attenuation = if matches!(origin, SoundOrigin::Local) { 0.0 } else { 1.0 };
        self.target.borrow_mut().play(PlaySound {
            sound: asset,
            actor,
            origin,
            seat,
            channel: options.channel,
            volume: options.volume / 127.0,
            attenuation,
        });
    }

    /// Start a placed sound (`startSound`).
    pub fn start_sound(&self, origin: Option<Vec3>, entity: i32, channel: i32, sound: Option<PcmSound>) {
        let asset = self.target.borrow().sounds().borrow().asset(&sound);
        let Some(asset) = asset else {
            return;
        };
        let actor = if entity < 0 {
            None
        } else {
            Some(self.target.borrow().actor(entity))
        };
        let source = match origin {
            Some(position) => SoundOrigin::Fixed { position },
            None => match actor {
                None => panic!("Entity-attached sound requires a source actor"),
                Some(actor) => SoundOrigin::Actor { actor },
            },
        };
        let seat = self.target.borrow().seat();
        self.target.borrow_mut().play(PlaySound {
            sound: asset,
            actor,
            origin: source,
            seat,
            channel,
            volume: 1.0,
            attenuation: 1.0,
        });
    }

    /// Start a local sound (`startLocalSound`).
    pub fn start_local_sound(&self, sound: Option<PcmSound>, channel: i32) {
        let asset = self.target.borrow().sounds().borrow().asset(&sound);
        let Some(asset) = asset else {
            return;
        };
        let seat = self.target.borrow().seat();
        self.target.borrow_mut().play(PlaySound {
            sound: asset,
            actor: None,
            origin: SoundOrigin::Local,
            seat,
            channel,
            volume: 1.0,
            attenuation: 0.0,
        });
    }

    /// Add a loop sound (`addLoopSound`).
    pub fn add_loop_sound(&self, entity: i32, origin: Vec3, velocity: Vec3, sound: Option<PcmSound>, real_loop: bool) {
        let asset = self.target.borrow().sounds().borrow().asset(&sound);
        let Some(asset) = asset else {
            return;
        };
        let actor = self.target.borrow().actor(entity);
        let seat = self.target.borrow().seat();
        let frame_number = self.target.borrow().frame_number();
        self.target.borrow_mut().loop_sound(LoopSound {
            sound: asset,
            actor,
            origin: SoundOrigin::Fixed { position: origin },
            seat,
            velocity,
            volume: 1.0,
            attenuation: 1.0,
            frame_number,
            persistent: real_loop,
        });
    }

    /// Update a sound position (`updateSoundPosition`).
    pub fn update_sound_position(&self, entity: i32, origin: Vec3) {
        let actor = self.target.borrow().actor(entity);
        self.target.borrow_mut().update_actor(actor, origin);
    }

    /// Stop a looping sound (`stopLoopingSound`).
    pub fn stop_looping_sound(&self, entity: i32) {
        let actor = self.target.borrow().actor(entity);
        let seat = self.target.borrow().seat();
        self.target.borrow_mut().stop_loop(seat, actor);
    }
}

/// Frame audio host (`ClientFrameAudioHost`).
pub trait ClientFrameAudioHost {
    /// Start a local sound.
    fn start_local_sound(&mut self, sound: Option<PcmSound>, channel: i32);
    /// Start a placed sound.
    fn start_sound(&mut self, origin: Option<Vec3>, entity: i32, channel: i32, sound: Option<PcmSound>);
}

/// Buffered and powerup audio (`ClientFrameAudio`).
pub struct ClientFrameAudio {
    /// Frame state.
    pub state: Shared<ClientGameState>,
    /// Wear-off sound.
    pub wear_off_sound: Option<PcmSound>,
    /// Host.
    pub host: Shared<dyn ClientFrameAudioHost>,
}

impl ClientFrameAudio {
    /// Assemble frame audio.
    pub fn new(
        state: Shared<ClientGameState>,
        wear_off_sound: Option<PcmSound>,
        host: Shared<dyn ClientFrameAudioHost>,
    ) -> Self {
        Self {
            state,
            wear_off_sound,
            host,
        }
    }

    /// Buffer a sound (`addBufferedSound`).
    pub fn add_buffered_sound(&self, sound: Option<PcmSound>) {
        let Some(sound) = sound else {
            return;
        };
        let mut state = self.state.borrow_mut();
        let input = state.sound_buffer_in;
        state.sound_buffer[input as usize % 20] = Some(sound);
        state.sound_buffer_in = (input + 1) % 20;
        if state.sound_buffer_in == state.sound_buffer_out {
            state.sound_buffer_out += 1;
        }
    }

    /// Play buffered sounds (`playBufferedSounds`).
    pub fn play_buffered_sounds(&self) {
        let (sound_time, time, output, input) = {
            let state = self.state.borrow();
            (
                state.sound_time,
                state.time,
                state.sound_buffer_out,
                state.sound_buffer_in,
            )
        };
        if sound_time >= time || output == input {
            return;
        }
        let sound = self
            .state
            .borrow()
            .sound_buffer
            .get(output as usize)
            .cloned()
            .unwrap_or_else(|| panic!("CG_PlayBufferedSounds: sound buffer index {output} outside 0..19"));
        let Some(sound) = sound else {
            return;
        };
        self.host.borrow_mut().start_local_sound(Some(sound), 7);
        let mut state = self.state.borrow_mut();
        state.sound_buffer[output as usize] = None;
        state.sound_buffer_out = (output + 1) % 20;
        state.sound_time = state.time.wrapping_add(750);
    }

    /// Powerup timer sounds (`powerupTimerSounds`).
    pub fn powerup_timer_sounds(&self) {
        let snapshot = self.state.borrow().snap.clone();
        let Some(snapshot) = snapshot else {
            panic!("CG_PowerupTimerSounds requires an active snapshot");
        };
        let (time, old_time) = {
            let state = self.state.borrow();
            (state.time, state.old_time)
        };
        for slot in 0..16 {
            let expiry = snapshot.player_state.powerups.get(slot);
            if expiry <= time {
                continue;
            }
            let remaining = expiry.wrapping_sub(time);
            if remaining >= 5000 {
                continue;
            }
            let previous = expiry.wrapping_sub(old_time);
            if remaining / 1000 != previous / 1000 {
                self.host.borrow_mut().start_sound(
                    None,
                    snapshot.player_state.client_num,
                    4,
                    self.wear_off_sound.clone(),
                );
            }
        }
    }
}

/// Model paint request (`UiModelPaintRequest`).
#[derive(Clone)]
pub struct UiModelPaintRequest {
    /// Drawing context (must share the painter's queue).
    pub draw: Draw2D,
    /// Model.
    pub model: SceneModel,
    /// Rectangle.
    pub rect: Rect2d,
    /// Time.
    pub time: i32,
    /// Angle.
    pub angle: f32,
    /// Horizontal FOV (0 means viewport width).
    pub field_of_view_x: f32,
    /// Vertical FOV (0 means viewport height).
    pub field_of_view_y: f32,
}

/// Seat model painter (`EngineUiModelPainter`).
pub struct EngineUiModelPainter {
    /// Renderer resources.
    pub resources: Shared<dyn RendererResources>,
    /// Seat drawing context.
    pub commands: Draw2D,
}

impl EngineUiModelPainter {
    /// Assemble a painter.
    pub fn new(resources: Shared<dyn RendererResources>, commands: Draw2D) -> Self {
        Self { resources, commands }
    }

    /// Paint a model (`paint`).
    pub fn paint(&self, request: &UiModelPaintRequest) {
        if !request.draw.shares_queue(&self.commands) {
            panic!("UI model painter must use its seat's drawing queue");
        }
        let viewport = request.draw.adjust(request.rect);
        let mut refdef = create_refdef();
        refdef.render_flags = RDF_NOWORLDMODEL;
        refdef.view_axis = [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)];
        refdef.x = viewport.x.trunc() as i32;
        refdef.y = viewport.y.trunc() as i32;
        refdef.width = viewport.width.trunc() as i32;
        refdef.height = viewport.height.trunc() as i32;
        refdef.fov_x = if request.field_of_view_x != 0.0 {
            request.field_of_view_x
        } else {
            viewport.width
        };
        refdef.fov_y = if request.field_of_view_y != 0.0 {
            request.field_of_view_y
        } else {
            viewport.height
        };
        refdef.time = request.time;
        let bounds = self.resources.borrow().model_bounds(&request.model);
        let mut entity = create_model_entity(request.model.clone());
        let length = 0.5 * (bounds.max.z - bounds.min.z);
        entity.origin = vec3(
            length / 0.268,
            0.5 * (bounds.min.y + bounds.max.y),
            -0.5 * (bounds.min.z + bounds.max.z),
        );
        entity.lighting_origin = entity.origin;
        entity.old_origin = entity.origin;
        entity.axis = qvm_angles_to_axis(vec3(0.0, request.angle, 0.0));
        entity.render_flags = RF_LIGHTING_ORIGIN | RF_NOSHADOW;
        self.resources.borrow_mut().clear_scene();
        self.resources.borrow_mut().add_ref_entity(&entity);
        self.resources.borrow_mut().render_scene(&refdef);
    }

    /// Shared-queue handle for hosts.
    pub fn shared_painter(
        resources: Shared<dyn RendererResources>,
        commands: Draw2D,
    ) -> Shared<dyn ModelPainterService> {
        shared(PainterService(Self::new(resources, commands)))
    }
}

/// Model painter service (`paintModel`).
pub trait ModelPainterService {
    /// Paint a model.
    fn paint(&mut self, request: &UiModelPaintRequest);
}

/// Service wrapper over [`EngineUiModelPainter`].
pub struct PainterService(pub EngineUiModelPainter);

impl ModelPainterService for PainterService {
    fn paint(&mut self, request: &UiModelPaintRequest) {
        self.0.paint(request);
    }
}

/// Visible length without color escapes (`drawStrlen`).
#[must_use]
pub fn draw_strlen(text: &str) -> i32 {
    let units: Vec<char> = text.chars().collect();
    let mut end = units.len();
    for (index, unit) in units.iter().enumerate() {
        if *unit == '\0' {
            end = index;
            break;
        }
        if *unit as u32 > 255 {
            panic!("Cgame text requires byte characters");
        }
    }
    let mut count = 0i32;
    let mut index = 0usize;
    while index < end {
        if units[index] == '^' && index + 1 < units.len() && units[index + 1] != '\0' && units[index + 1] != '^' {
            index += 1;
        } else {
            count += 1;
        }
        index += 1;
    }
    count
}

/// Proportional size scale (`proportionalSizeScale`).
#[must_use]
pub fn proportional_size_scale(style: i32) -> f32 {
    if style & UI_SMALLFONT != 0 {
        0.75
    } else {
        1.0
    }
}

/// White with fade alpha (`fadeColor`).
#[must_use]
pub fn fade_color(time: i32, start_msec: i32, total_msec: f32) -> Option<Vec4> {
    let total = total_msec as i32;
    let elapsed = time.wrapping_sub(start_msec);
    if start_msec == 0 || elapsed >= total {
        return None;
    }
    let remaining = total.wrapping_sub(elapsed);
    Some(vec4(
        1.0,
        1.0,
        1.0,
        if remaining < 200 { remaining as f32 / 200.0 } else { 1.0 },
    ))
}

/// Team color (`teamColor`).
#[must_use]
pub fn team_color(team: i32) -> Vec4 {
    if team == Team::Red as i32 {
        vec4(1.0, 0.2, 0.2, 1.0)
    } else if team == Team::Blue as i32 {
        vec4(0.2, 0.2, 1.0, 1.0)
    } else if team == Team::Spectator as i32 {
        vec4(0.7, 0.7, 0.7, 1.0)
    } else {
        vec4(1.0, 1.0, 1.0, 1.0)
    }
}

/// Health color (`getColorForHealth`).
#[must_use]
pub fn get_color_for_health(health: i32, armor: i32) -> Vec4 {
    if health <= 0 {
        return vec4(0.0, 0.0, 0.0, 1.0);
    }
    let maximum = (f64::from(health) * ARMOR_PROTECTION / (1.0 - ARMOR_PROTECTION)) as i32;
    let health = health.wrapping_add(armor.min(maximum));
    vec4(
        1.0,
        if health > 60 {
            1.0
        } else if health < 30 {
            0.0
        } else {
            (health - 30) as f32 / 30.0
        },
        if health >= 100 {
            1.0
        } else if health < 66 {
            0.0
        } else {
            (health - 66) as f32 / 33.0
        },
        1.0,
    )
}

/// Snapshot health color (`colorForHealth`).
#[must_use]
pub fn color_for_health(state: &Shared<ClientGameState>) -> Vec4 {
    let snapshot = state.borrow().snap.clone();
    let Some(snapshot) = snapshot else {
        panic!("CG_ColorForHealth requires a current snapshot");
    };
    let schema = stat_schema(snapshot.player_state.product);
    get_color_for_health(
        snapshot.player_state.stats.get(schema.health),
        snapshot.player_state.stats.get(schema.armor),
    )
}

/// Cgame draw recorder (`ClientDrawTools`).
#[derive(Clone)]
pub struct ClientDrawTools {
    /// Drawing context.
    pub draw: Draw2D,
    /// Media.
    pub media: Shared<ClientMedia>,
}

impl ClientDrawTools {
    /// Assemble tools, requiring stretch-640 coordinates.
    pub fn new(draw: Draw2D, media: Shared<ClientMedia>) -> Self {
        if draw.space != CoordinateSpace::Stretch640 {
            panic!("Cgame drawing requires stretch-640 coordinates");
        }
        Self { draw, media }
    }

    /// Picture for a shader.
    #[must_use]
    pub fn picture(&self, shader: &Option<SceneShader>) -> Picture {
        self.media.borrow().resources.borrow().picture(shader)
    }

    /// Legacy font pictures.
    fn legacy_fonts(&self) -> LegacyFonts {
        let media = self.media.borrow();
        let resources = media.resources.borrow();
        LegacyFonts {
            charset: resources.picture(&media.graphics.charset_shader),
            proportional: resources.picture(&media.graphics.charset_prop),
            glow: resources.picture(&media.graphics.charset_prop_glow),
            banner: resources.picture(&media.graphics.charset_prop_b),
        }
    }

    /// Draw a proportional string (`drawProportionalString`).
    pub fn draw_proportional_string(&self, options: &UiTextOptions) {
        draw_cg_proportional_string(&self.draw, &self.legacy_fonts(), options);
    }

    /// Draw a banner string (`drawBannerString`).
    pub fn draw_banner_string(&self, options: &UiTextOptions) {
        draw_cg_banner_string(&self.draw, &self.legacy_fonts(), options);
    }

    /// Adjust from 640 space (`adjustFrom640`).
    #[must_use]
    pub fn adjust_from_640(&self, rect: Rect2d) -> Rect2d {
        self.draw.adjust(rect)
    }

    /// Fill a rectangle (`fillRect`).
    pub fn fill_rect(&self, rect: Rect2d, color: Option<Vec4>) {
        let picture = {
            let media = self.media.borrow();
            let resources = media.resources.borrow();
            resources.picture(&media.graphics.white_shader)
        };
        self.draw.set_color(color);
        self.draw.stretch_pic(rect, ZERO_UV, picture);
        self.draw.set_color(None);
    }

    /// Draw side borders (`drawSides`).
    pub fn draw_sides(&self, rect: Rect2d, size: f32) {
        let adjusted = self.adjust_from_640(rect);
        let width = size * self.draw.scale_x();
        let picture = {
            let media = self.media.borrow();
            let resources = media.resources.borrow();
            resources.picture(&media.graphics.white_shader)
        };
        self.draw.stretch_pixels(Rect2d { width, ..adjusted }, ZERO_UV, picture);
        self.draw.stretch_pixels(
            Rect2d {
                x: adjusted.x + adjusted.width - width,
                width,
                ..adjusted
            },
            ZERO_UV,
            picture,
        );
    }

    /// Draw top/bottom borders (`drawTopBottom`).
    pub fn draw_top_bottom(&self, rect: Rect2d, size: f32) {
        let adjusted = self.adjust_from_640(rect);
        let height = size * self.draw.scale_y();
        let picture = {
            let media = self.media.borrow();
            let resources = media.resources.borrow();
            resources.picture(&media.graphics.white_shader)
        };
        self.draw
            .stretch_pixels(Rect2d { height, ..adjusted }, ZERO_UV, picture);
        self.draw.stretch_pixels(
            Rect2d {
                y: adjusted.y + adjusted.height - height,
                height,
                ..adjusted
            },
            ZERO_UV,
            picture,
        );
    }

    /// Draw a rectangle border (`drawRect`).
    pub fn draw_rect(&self, rect: Rect2d, size: f32, color: Option<Vec4>) {
        self.draw.set_color(color);
        self.draw_top_bottom(rect, size);
        self.draw_sides(rect, size);
        self.draw.set_color(None);
    }

    /// Draw a picture (`drawPic`).
    pub fn draw_pic(&self, rect: Rect2d, shader: &Option<SceneShader>) {
        let picture = self.picture(shader);
        self.draw.draw_pic(rect, picture);
    }

    /// Draw a character (`drawChar`).
    pub fn draw_char(&self, x: f32, y: f32, width: f32, height: f32, code: u32) {
        if code & 255 == 32 {
            return;
        }
        let charset = {
            let media = self.media.borrow();
            let resources = media.resources.borrow();
            resources.picture(&media.graphics.charset_shader)
        };
        draw_char(&self.draw, charset, x, y, width, height, code);
    }

    /// Draw an extended string (`drawStringExt`).
    pub fn draw_string_ext(&self, options: &FixedTextOptions) {
        let charset = {
            let media = self.media.borrow();
            let resources = media.resources.borrow();
            resources.picture(&media.graphics.charset_shader)
        };
        draw_cg_string(&self.draw, charset, options);
    }

    /// Draw a big string (`drawBigString`).
    pub fn draw_big_string(&self, x: i32, y: i32, text: &str, alpha: f32) {
        self.draw_string_ext(&FixedTextOptions {
            x: x as f32,
            y: y as f32,
            text: text.to_string(),
            color: vec4(1.0, 1.0, 1.0, alpha),
            force_color: false,
            shadow: true,
            char_width: 16,
            char_height: 16,
            max_chars: 0,
        });
    }

    /// Draw a big colored string (`drawBigStringColor`).
    pub fn draw_big_string_color(&self, x: i32, y: i32, text: &str, color: Vec4) {
        self.draw_string_ext(&FixedTextOptions {
            x: x as f32,
            y: y as f32,
            text: text.to_string(),
            color,
            force_color: true,
            shadow: true,
            char_width: 16,
            char_height: 16,
            max_chars: 0,
        });
    }

    /// Draw a small string (`drawSmallString`).
    pub fn draw_small_string(&self, x: i32, y: i32, text: &str, alpha: f32) {
        self.draw_string_ext(&FixedTextOptions {
            x: x as f32,
            y: y as f32,
            text: text.to_string(),
            color: vec4(1.0, 1.0, 1.0, alpha),
            force_color: false,
            shadow: false,
            char_width: 8,
            char_height: 16,
            max_chars: 0,
        });
    }

    /// Draw a small colored string (`drawSmallStringColor`).
    pub fn draw_small_string_color(&self, x: i32, y: i32, text: &str, color: Vec4) {
        self.draw_string_ext(&FixedTextOptions {
            x: x as f32,
            y: y as f32,
            text: text.to_string(),
            color,
            force_color: true,
            shadow: false,
            char_width: 8,
            char_height: 16,
            max_chars: 0,
        });
    }

    /// Tile one clear box (`tileClearBox`).
    fn tile_clear_box(&self, x: f32, y: f32, width: f32, height: f32) {
        let picture = {
            let media = self.media.borrow();
            let resources = media.resources.borrow();
            resources.picture(&media.graphics.back_tile_shader)
        };
        self.draw.stretch_pixels(
            rect2d(x, y, width, height),
            TextureRect {
                s: x / 64.0,
                t: y / 64.0,
                s2: (x + width) / 64.0,
                t2: (y + height) / 64.0,
            },
            picture,
        );
    }

    /// Tile-clear around a viewport (`tileClear`).
    pub fn tile_clear(&self, refdef: &Refdef) {
        let width = self.draw.width();
        let height = self.draw.height();
        if refdef.x == 0 && refdef.y == 0 && refdef.width == width && refdef.height == height {
            return;
        }
        let top = refdef.y;
        let bottom = top + refdef.height - 1;
        let left = refdef.x;
        let right = left + refdef.width - 1;
        self.tile_clear_box(0.0, 0.0, width as f32, top as f32);
        self.tile_clear_box(0.0, bottom as f32, width as f32, (height - bottom) as f32);
        self.tile_clear_box(0.0, top as f32, left as f32, (bottom - top + 1) as f32);
        self.tile_clear_box(
            right as f32,
            top as f32,
            (width - right) as f32,
            (bottom - top + 1) as f32,
        );
    }
}

/// Draw-icon settings (`ClientDrawIconSettings`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientDrawIconSettings {
    /// Draw icons.
    pub draw_icons: bool,
    /// Draw 3D icons.
    pub draw_3d_icons: bool,
}

/// Icon origin for bounds (`iconOrigin`).
fn icon_origin(bounds: &Bounds, fraction: f32) -> Vec3 {
    let length = fraction * (bounds.max.z - bounds.min.z);
    vec3(
        length / 0.268,
        0.5 * (bounds.min.y + bounds.max.y),
        -0.5 * (bounds.min.z + bounds.max.z),
    )
}

/// 3D icon drawing (`ClientDrawIcons`).
pub struct ClientDrawIcons {
    /// Frame state.
    pub state: Shared<ClientGameState>,
    /// Draw tools.
    pub tools: ClientDrawTools,
    /// Settings reader.
    pub settings: Rc<dyn Fn() -> ClientDrawIconSettings>,
    /// Command queue context (must share the tools queue).
    pub commands: Draw2D,
}

impl ClientDrawIcons {
    /// Assemble draw icons.
    pub fn new(
        state: Shared<ClientGameState>,
        tools: ClientDrawTools,
        settings: Rc<dyn Fn() -> ClientDrawIconSettings>,
        commands: Draw2D,
    ) -> Self {
        if !tools.draw.shares_queue(&commands) {
            panic!("Draw icons and command buffer must share the engine drawing queue");
        }
        if state.borrow().product != tools.media.borrow().product {
            panic!("Draw icons and media products differ");
        }
        Self {
            state,
            tools,
            settings,
            commands,
        }
    }

    /// Draw a 3D model (`draw3DModel`).
    pub fn draw_3d_model(&self, rect: Rect2d, model: &SceneModel, skin: Option<SceneSkin>, origin: Vec3, angles: Vec3) {
        let settings = (self.settings)();
        if !settings.draw_3d_icons || !settings.draw_icons {
            return;
        }
        let viewport = self.tools.adjust_from_640(rect);
        let mut refdef = create_refdef();
        let mut entity = create_model_entity(model.clone());
        entity.axis = qvm_angles_to_axis(angles);
        entity.origin = vec3(origin.x, origin.y, origin.z);
        entity.custom_skin = skin;
        entity.render_flags = RF_NOSHADOW;
        refdef.render_flags = RDF_NOWORLDMODEL;
        refdef.view_axis = [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)];
        refdef.fov_x = 30.0;
        refdef.fov_y = 30.0;
        refdef.time = self.state.borrow().time;
        refdef.x = viewport.x.trunc() as i32;
        refdef.y = viewport.y.trunc() as i32;
        refdef.width = viewport.width.trunc() as i32;
        refdef.height = viewport.height.trunc() as i32;
        // ClearScene starts an empty scene without discarding previously queued draw commands.
        let resources = self.tools.media.borrow().resources.clone();
        resources.borrow_mut().clear_scene();
        resources.borrow_mut().add_ref_entity(&entity);
        resources.borrow_mut().render_scene(&refdef);
    }

    /// Draw a head (`drawHead`).
    pub fn draw_head(&self, rect: Rect2d, client_num: i32, head_angles: Vec3) {
        let static_state = self.tools.media.borrow().static_state.clone();
        let client = static_state
            .borrow()
            .client_info
            .get(client_num as usize)
            .cloned()
            .unwrap_or_else(|| {
                panic!("CG_DrawHead: invalid client number");
            });
        if client_num < 0 {
            panic!("CG_DrawHead: invalid client number");
        }
        let settings = (self.settings)();
        let client = client.borrow();
        if settings.draw_3d_icons {
            if client.head_model.is_default() {
                return;
            }
            let bounds = self
                .tools
                .media
                .borrow()
                .resources
                .borrow()
                .model_bounds(&client.head_model);
            let origin = add3(icon_origin(&bounds, 0.7), client.head_offset);
            let model = client.head_model.clone();
            let skin = client.head_skin.clone();
            let deferred = client.deferred;
            drop(client);
            self.draw_3d_model(rect, &model, skin, origin, head_angles);
            if deferred {
                let defer = self.tools.media.borrow().graphics.defer_shader.clone();
                self.tools.draw_pic(rect, &defer);
            }
        } else {
            if settings.draw_icons {
                let icon = client.model_icon.clone();
                drop(client);
                self.tools.draw_pic(rect, &icon);
                let deferred = static_state.borrow().client_info[client_num as usize].borrow().deferred;
                if deferred {
                    let defer = self.tools.media.borrow().graphics.defer_shader.clone();
                    self.tools.draw_pic(rect, &defer);
                }
            } else {
                let deferred = client.deferred;
                drop(client);
                if deferred {
                    let defer = self.tools.media.borrow().graphics.defer_shader.clone();
                    self.tools.draw_pic(rect, &defer);
                }
            }
        }
    }

    /// Draw a flag model (`drawFlagModel`).
    pub fn draw_flag_model(&self, rect: Rect2d, team: i32, force_2d: bool) {
        let settings = (self.settings)();
        let media = self.tools.media.borrow();
        if !force_2d && settings.draw_3d_icons {
            let bounds = media.resources.borrow().model_bounds(&media.graphics.red_flag_model);
            let origin = icon_origin(&bounds, 0.5);
            let time = self.state.borrow().time;
            let angles = vec3(0.0, 60.0 * (time as f32 / 2000.0).sin(), 0.0);
            let model = if team == Team::Red as i32 {
                media.graphics.red_flag_model.clone()
            } else if team == Team::Blue as i32 {
                media.graphics.blue_flag_model.clone()
            } else if team == Team::Free as i32 {
                media.graphics.neutral_flag_model.clone()
            } else {
                return;
            };
            drop(media);
            self.draw_3d_model(rect, &model, None, origin, angles);
        } else if settings.draw_icons {
            let powerup = if team == Team::Red as i32 {
                Powerup::RedFlag
            } else if team == Team::Blue as i32 {
                Powerup::BlueFlag
            } else if team == Team::Free as i32 {
                Powerup::NeutralFlag
            } else {
                return;
            };
            let product = media.product;
            let item = media.items.borrow().find_for_powerup(product, powerup as i32);
            if let Some(item) = item {
                let index = media.items.borrow().index_of(product, &item);
                let visual = media.weapon_registry.borrow().item_visual(index);
                drop(media);
                self.tools.draw_pic(rect, &visual.icon);
            }
        }
    }

    /// Draw a team background (`drawTeamBackground`).
    pub fn draw_team_background(&self, rect: Rect2d, alpha: f32, team: i32) {
        if team != Team::Red as i32 && team != Team::Blue as i32 {
            return;
        }
        self.tools.draw.set_color(Some(vec4(
            if team == Team::Red as i32 { 1.0 } else { 0.0 },
            0.0,
            if team == Team::Blue as i32 { 1.0 } else { 0.0 },
            alpha,
        )));
        let bar = self.tools.media.borrow().graphics.team_status_bar.clone();
        self.tools.draw_pic(
            rect2d(rect.x.trunc(), rect.y.trunc(), rect.width.trunc(), rect.height.trunc()),
            &bar,
        );
        self.tools.draw.set_color(None);
    }
}

/// Lagometer sample counts.
const LAG_SAMPLES: usize = 128;
/// Maximum lagometer ping.
const MAX_LAGOMETER_PING: f32 = 900.0;
/// Maximum lagometer range.
const MAX_LAGOMETER_RANGE: f32 = 300.0;

/// Draw-status product variant (`ClientDrawStatusVariant`).
#[derive(Debug, Clone, PartialEq)]
pub enum ClientDrawStatusVariant {
    /// Base game.
    Baseq3,
    /// Mission pack with cgame fonts.
    Missionpack {
        /// Fonts.
        fonts: FontSet,
    },
}

impl ClientDrawStatusVariant {
    /// Product kind.
    #[must_use]
    pub fn kind(&self) -> Product {
        match self {
            Self::Baseq3 => Product::Baseq3,
            Self::Missionpack { .. } => Product::Missionpack,
        }
    }
}

/// Draw-status host services.
pub struct ClientDrawStatusHost {
    /// Command source.
    pub commands: Shared<dyn CommandSource>,
    /// Cvar reader.
    pub cvars: Shared<dyn HudCvarReader>,
}

/// Lagometer snapshot sample (`LagometerSnapshotSample`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LagometerSnapshotSample {
    /// Ping.
    pub ping: i32,
    /// Flags.
    pub flags: i32,
}

/// Center-print source text (1023 cap).
fn center_source_text(input: &str) -> String {
    let cut = match input.find('\0') {
        Some(end) => &input[..end],
        None => input,
    };
    for ch in cut.chars() {
        if ch as u32 > 255 {
            panic!("Center print requires source byte characters");
        }
    }
    cut.chars().take(1023).collect()
}

/// Status text and lagometer state (`ClientDrawStatus`).
pub struct ClientDrawStatus {
    /// Frame state.
    pub state: Shared<ClientGameState>,
    /// Static state.
    pub static_state: Shared<ClientGameStaticState>,
    /// Draw tools.
    pub tools: ClientDrawTools,
    /// Variant.
    pub variant: ClientDrawStatusVariant,
    /// Host.
    pub host: ClientDrawStatusHost,
    /// Frame samples.
    frame_samples: RefCell<[i32; LAG_SAMPLES]>,
    /// Snapshot flags.
    snapshot_flags: RefCell<[i32; LAG_SAMPLES]>,
    /// Snapshot samples.
    snapshot_samples: RefCell<[i32; LAG_SAMPLES]>,
    /// Frame count.
    frame_count: Cell<i32>,
    /// Snapshot count.
    snapshot_count: Cell<i32>,
}

impl ClientDrawStatus {
    /// Assemble draw status.
    pub fn new(
        state: Shared<ClientGameState>,
        static_state: Shared<ClientGameStaticState>,
        tools: ClientDrawTools,
        variant: ClientDrawStatusVariant,
        host: ClientDrawStatusHost,
    ) -> Self {
        if state.borrow().product != static_state.borrow().product
            || !same(&tools.media.borrow().static_state, &static_state)
        {
            panic!("Draw status services must share canonical cgame state");
        }
        if variant.kind() != state.borrow().product {
            panic!("Draw status product variant differs from cgame state");
        }
        if let ClientDrawStatusVariant::Missionpack { fonts } = &variant {
            if fonts.profile != FontProfile::Cgame {
                panic!("Missionpack center print requires cgame fonts");
            }
        }
        Self {
            state,
            static_state,
            tools,
            variant,
            host,
            frame_samples: RefCell::new([0; LAG_SAMPLES]),
            snapshot_flags: RefCell::new([0; LAG_SAMPLES]),
            snapshot_samples: RefCell::new([0; LAG_SAMPLES]),
            frame_count: Cell::new(0),
            snapshot_count: Cell::new(0),
        }
    }

    /// Center print (`centerPrint`).
    pub fn center_print(&self, text: &str, y: i32, char_width: i32) {
        let mut state = self.state.borrow_mut();
        state.center_print = center_source_text(text);
        state.center_print_time = state.time;
        state.center_print_y = y;
        state.center_print_char_width = char_width;
        state.center_print_lines = 1;
        let source = state.center_print.clone();
        for unit in source.chars() {
            if unit == '\n' {
                state.center_print_lines += 1;
            }
        }
    }

    /// Draw the center string (`drawCenterString`).
    pub fn draw_center_string(&self) {
        let (center_time, center_print, lines, char_width, center_y, time) = {
            let state = self.state.borrow();
            (
                state.center_print_time,
                state.center_print.clone(),
                state.center_print_lines,
                state.center_print_char_width,
                state.center_print_y,
                state.time,
            )
        };
        if center_time == 0 {
            return;
        }
        let duration = 1000.0 * self.host.cvars.borrow().read_vm_cvar("cg_centertime").numeric_value;
        let color = fade_color(time, center_time, duration);
        let Some(color) = color else {
            return;
        };
        let mut y = center_y - lines.wrapping_mul(16) / 2;
        let units: Vec<char> = center_print.chars().collect();
        let mut start = 0usize;
        loop {
            let mut end = start;
            while end < units.len() && units[end] != '\n' {
                end += 1;
            }
            let line: String = units[start..end.min(start + 50)].iter().collect();
            if let ClientDrawStatusVariant::Missionpack { fonts } = &self.variant {
                let width = text_width(fonts, &line, 0.5, 0);
                let height = text_height(fonts, &line, 0.5, 0);
                let x = (640 - width) / 2;
                text_paint(
                    &self.tools.draw,
                    fonts,
                    &TextPaintOptions {
                        x: x as f32,
                        y: (y + height) as f32,
                        scale: 0.5,
                        color,
                        text: line,
                        adjust: 0.0,
                        limit: 0,
                        style: 6,
                    },
                );
                y += height + 6;
            } else {
                let width = char_width.wrapping_mul(draw_strlen(&line));
                let x = (640 - width) / 2;
                let height = (char_width as f32 * 1.5).trunc() as i32;
                self.tools.draw_string_ext(&FixedTextOptions {
                    x: x as f32,
                    y: y as f32,
                    text: line,
                    color,
                    force_color: false,
                    shadow: true,
                    char_width,
                    char_height: height,
                    max_chars: 0,
                });
                y = (y as f32 + char_width as f32 * 1.5).trunc() as i32;
            }
            if end == units.len() {
                break;
            }
            start = end + 1;
        }
        self.tools.draw.set_color(None);
    }

    /// Add lagometer frame info (`addLagometerFrameInfo`).
    pub fn add_lagometer_frame_info(&self) {
        let (time, latest) = {
            let state = self.state.borrow();
            (state.time, state.latest_snapshot_time)
        };
        let count = self.frame_count.get();
        self.frame_samples.borrow_mut()[(count & 127) as usize] = time.wrapping_sub(latest);
        self.frame_count.set(count.wrapping_add(1));
    }

    /// Add lagometer snapshot info (`addLagometerSnapshotInfo`).
    pub fn add_lagometer_snapshot_info(&self, sample: Option<LagometerSnapshotSample>) {
        let count = self.snapshot_count.get();
        let index = (count & 127) as usize;
        match sample {
            None => self.snapshot_samples.borrow_mut()[index] = -1,
            Some(sample) => {
                self.snapshot_samples.borrow_mut()[index] = sample.ping;
                self.snapshot_flags.borrow_mut()[index] = sample.flags;
            }
        }
        self.snapshot_count.set(count.wrapping_add(1));
    }

    /// Draw the disconnect notice (`drawDisconnect`).
    pub fn draw_disconnect(&self) {
        let snapshot = self.state.borrow().snap.clone();
        let Some(snapshot) = snapshot else {
            panic!("CG_DrawDisconnect requires a current snapshot");
        };
        let command_number = self
            .host
            .commands
            .borrow()
            .current_number()
            .wrapping_sub(64)
            .wrapping_add(1);
        let command = self.host.commands.borrow().read(command_number);
        let Some(command) = command else {
            panic!("CG_DrawDisconnect command fell outside CMD_BACKUP");
        };
        let time = self.state.borrow().time;
        if command.server_time <= snapshot.player_state.command_time || command.server_time > time {
            return;
        }
        let message = "Connection Interrupted";
        let width = draw_strlen(message).wrapping_mul(16);
        self.tools.draw_big_string(320 - width / 2, 100, message, 1.0);
        if (time >> 9) & 1 != 0 {
            return;
        }
        let shader = self
            .tools
            .media
            .borrow()
            .resources
            .borrow_mut()
            .register_shader("gfx/2d/net.tga");
        self.tools
            .draw_pic(rect2d(640.0 - 48.0, 480.0 - 48.0, 48.0, 48.0), &shader);
    }

    /// Draw the lagometer (`drawLagometer`).
    pub fn draw_lagometer(&self) {
        if self.host.cvars.borrow().read_vm_cvar("cg_lagometer").integer_value == 0
            || self.static_state.borrow().local_server != 0
        {
            self.draw_disconnect();
            return;
        }
        let product = self.state.borrow().product;
        let (x, y) = (
            640.0 - 48.0,
            if product == Product::Missionpack {
                480.0 - 144.0
            } else {
                480.0 - 48.0
            },
        );
        self.tools.draw.set_color(None);
        let lagometer = self.tools.media.borrow().graphics.lagometer_shader.clone();
        self.tools.draw_pic(rect2d(x, y, 48.0, 48.0), &lagometer);
        let adjusted = self.tools.adjust_from_640(rect2d(x, y, 48.0, 48.0));
        let picture = {
            let media = self.tools.media.borrow();
            let resources = media.resources.borrow();
            resources.picture(&media.graphics.white_shader)
        };
        let yellow = vec4(1.0, 1.0, 0.0, 1.0);
        let blue = vec4(0.0, 0.0, 1.0, 1.0);
        let green = vec4(0.0, 1.0, 0.0, 1.0);
        let red = vec4(1.0, 0.0, 0.0, 1.0);
        let mut color = -1;
        let frame_count = self.frame_count.get();
        let mut range = adjusted.height / 3.0;
        let mid = adjusted.y + range;
        let scale = range / MAX_LAGOMETER_RANGE;
        let steps = adjusted.width.trunc() as i32;
        for a in 0..steps {
            let index = (frame_count.wrapping_sub(1).wrapping_sub(a) & 127) as usize;
            let mut value = self.frame_samples.borrow()[index] as f32 * scale;
            if value > 0.0 {
                if color != 1 {
                    color = 1;
                    self.tools.draw.set_color(Some(yellow));
                }
                if value > range {
                    value = range;
                }
                self.tools.draw.stretch_pixels(
                    rect2d(adjusted.x + adjusted.width - a as f32, mid - value, 1.0, value),
                    ZERO_UV,
                    picture,
                );
            } else if value < 0.0 {
                if color != 2 {
                    color = 2;
                    self.tools.draw.set_color(Some(blue));
                }
                value = -value;
                if value > range {
                    value = range;
                }
                self.tools.draw.stretch_pixels(
                    rect2d(adjusted.x + adjusted.width - a as f32, mid, 1.0, value),
                    ZERO_UV,
                    picture,
                );
            }
        }
        range = adjusted.height / 2.0;
        let scale = range / MAX_LAGOMETER_PING;
        let snapshot_count = self.snapshot_count.get();
        for a in 0..steps {
            let index = (snapshot_count.wrapping_sub(1).wrapping_sub(a) & 127) as usize;
            let mut value = self.snapshot_samples.borrow()[index] as f32;
            if value > 0.0 {
                if self.snapshot_flags.borrow()[index] & 1 != 0 {
                    if color != 5 {
                        color = 5;
                        self.tools.draw.set_color(Some(yellow));
                    }
                } else if color != 3 {
                    color = 3;
                    self.tools.draw.set_color(Some(green));
                }
                value *= scale;
                if value > range {
                    value = range;
                }
                self.tools.draw.stretch_pixels(
                    rect2d(
                        adjusted.x + adjusted.width - a as f32,
                        adjusted.y + adjusted.height - value,
                        1.0,
                        value,
                    ),
                    ZERO_UV,
                    picture,
                );
            } else if value < 0.0 {
                if color != 4 {
                    color = 4;
                    self.tools.draw.set_color(Some(red));
                }
                self.tools.draw.stretch_pixels(
                    rect2d(
                        adjusted.x + adjusted.width - a as f32,
                        adjusted.y + adjusted.height - range,
                        1.0,
                        range,
                    ),
                    ZERO_UV,
                    picture,
                );
            }
        }
        self.tools.draw.set_color(None);
        if self.host.cvars.borrow().read_vm_cvar("cg_nopredict").integer_value != 0
            || self
                .host
                .cvars
                .borrow()
                .read_vm_cvar("g_synchronousClients")
                .integer_value
                != 0
        {
            self.tools
                .draw_big_string(adjusted.x as i32, adjusted.y as i32, "snc", 1.0);
        }
        self.draw_disconnect();
    }
}

/// Scoreboard header Y.
const SCOREBOARD_HEADER: f32 = 86.0;
/// Scoreboard top Y.
const SCOREBOARD_TOP: f32 = 118.0;
/// Normal row height.
const NORMAL_HEIGHT: f32 = 40.0;
/// Intermission row height.
const INTER_HEIGHT: f32 = 16.0;
/// Maximum normal rows.
const MAX_NORMAL: i32 = 7;
/// Maximum intermission rows.
const MAX_INTER: i32 = 17;

/// Scoreboard host services (`BaseScoreboardHost`).
pub struct BaseScoreboardHost {
    /// Draw icons.
    pub icons: Shared<ClientDrawIcons>,
    /// Client store.
    pub clients: Shared<dyn ClientInfoStore>,
    /// Player presenter.
    pub players: Shared<dyn PlayerPresenter>,
    /// Cvar reader.
    pub cvars: Shared<dyn HudCvarReader>,
    /// Configstrings.
    pub strings: Shared<dyn HudConfigStrings>,
    /// Commands.
    pub commands: Shared<dyn HudCommands>,
}

/// Classic scoreboard (`BaseScoreboard`).
pub struct BaseScoreboard {
    /// Frame state.
    pub state: Shared<ClientGameState>,
    /// Static state.
    pub static_state: Shared<ClientGameStaticState>,
    /// Host.
    pub host: BaseScoreboardHost,
    /// Local client drawn.
    local_client: Cell<bool>,
}

impl BaseScoreboard {
    /// Assemble a scoreboard.
    pub fn new(
        state: Shared<ClientGameState>,
        static_state: Shared<ClientGameStaticState>,
        host: BaseScoreboardHost,
    ) -> Self {
        if state.borrow().product != static_state.borrow().product {
            panic!("Scoreboard state products differ");
        }
        let icons = host.icons.borrow();
        if !same(&icons.state, &state)
            || !same(&icons.tools.media.borrow().static_state, &static_state)
            || !same(&host.clients.borrow().state_handle(), &state)
            || !same(&host.players.borrow().state_handle(), &state)
        {
            panic!("Scoreboard services must share canonical cgame state");
        }
        drop(icons);
        for index in 0..64 {
            if !same(
                &host.clients.borrow().client_info(index),
                &static_state.borrow().client_info[index as usize],
            ) {
                panic!("Scoreboard client store must use canonical client slots");
            }
        }
        Self {
            state,
            static_state,
            host,
            local_client: Cell::new(false),
        }
    }

    /// Current player state.
    fn snapshot(&self) -> PlayerState {
        self.state
            .borrow()
            .snap
            .clone()
            .unwrap_or_else(|| {
                panic!("CG_DrawOldScoreboard requires a current snapshot");
            })
            .player_state
    }

    /// Score row.
    fn score(&self, index: i32) -> ClientScore {
        self.state
            .borrow()
            .scores
            .get(index as usize)
            .copied()
            .unwrap_or_else(|| {
                panic!("Scoreboard score index outside source array: {index}");
            })
    }

    /// Client slot.
    fn client(&self, index: i32) -> Shared<ClientInfo> {
        self.static_state
            .borrow()
            .client_info
            .get(index as usize)
            .cloned()
            .unwrap_or_else(|| {
                panic!("Scoreboard client index outside source array: {index}");
            })
    }

    /// Draw one client score (`drawClientScore`).
    fn draw_client_score(&self, y: f32, score: &ClientScore, color: Vec4, fade: f32, large: bool) {
        let maxclients = self.static_state.borrow().maxclients;
        if score.client < 0 || score.client >= maxclients {
            let text = game_format("Bad score->client: %i\n", &[GameFormatArg::Int(score.client)], 1024);
            self.host.commands.borrow_mut().print(&text);
            return;
        }
        let client_handle = self.client(score.client);
        let client = client_handle.borrow();
        let icons = self.host.icons.borrow();
        let tools = icons.tools.clone();
        let icon_x = 80.0f32;
        let head_x = 112.0f32;
        let icon_rect = rect2d(
            icon_x,
            if large { y - 8.0 } else { y },
            if large { 32.0 } else { 16.0 },
            if large { 32.0 } else { 16.0 },
        );
        if client.powerups & (1 << Powerup::NeutralFlag as i32) != 0 {
            icons.draw_flag_model(icon_rect, Team::Free as i32, false);
        } else if client.powerups & (1 << Powerup::RedFlag as i32) != 0 {
            icons.draw_flag_model(icon_rect, Team::Red as i32, false);
        } else if client.powerups & (1 << Powerup::BlueFlag as i32) != 0 {
            icons.draw_flag_model(icon_rect, Team::Blue as i32, false);
        } else {
            if client.bot_skill > 0 && client.bot_skill <= 5 {
                if self.host.cvars.borrow().read_vm_cvar("cg_drawIcons").integer_value != 0 {
                    let shader = tools
                        .media
                        .borrow()
                        .graphics
                        .bot_skill_shaders
                        .get((client.bot_skill - 1) as usize)
                        .cloned()
                        .unwrap_or_else(|| {
                            panic!("Scoreboard bot skill outside source shader array");
                        });
                    tools.draw_pic(icon_rect, &shader);
                }
            } else if client.handicap < 100 {
                let game_type = self.static_state.borrow().game_type;
                tools.draw_small_string_color(
                    icon_x as i32,
                    (if game_type == GameType::Tournament { y - 8.0 } else { y }) as i32,
                    &game_format("%i", &[GameFormatArg::Int(client.handicap)], 1024),
                    color,
                );
            }
            if self.static_state.borrow().game_type == GameType::Tournament {
                tools.draw_small_string_color(
                    icon_x as i32,
                    (if client.handicap < 100 && client.bot_skill == 0 {
                        y + 8.0
                    } else {
                        y
                    }) as i32,
                    &game_format(
                        "%i/%i",
                        &[GameFormatArg::Int(client.wins), GameFormatArg::Int(client.losses)],
                        1024,
                    ),
                    color,
                );
            }
        }
        icons.draw_head(
            rect2d(
                head_x,
                if large { y - 16.0 } else { y },
                if large { 48.0 } else { 16.0 },
                if large { 48.0 } else { 16.0 },
            ),
            score.client,
            vec3(0.0, 180.0, 0.0),
        );
        if self.state.borrow().product == Product::Missionpack {
            if client.team_task == 1 {
                let shader = tools.media.borrow().graphics.assault_shader.clone();
                tools.draw_pic(rect2d(head_x + 48.0, y, 16.0, 16.0), &shader);
            } else if client.team_task == 2 {
                let shader = tools.media.borrow().graphics.defend_shader.clone();
                tools.draw_pic(rect2d(head_x + 48.0, y, 16.0, 16.0), &shader);
            }
        }
        let text = if score.ping == -1 {
            game_format(" connecting    %s", &[GameFormatArg::Text(client.name.clone())], 1024)
        } else if client.team == Team::Spectator {
            game_format(
                " SPECT %3i %4i %s",
                &[
                    GameFormatArg::Int(score.ping),
                    GameFormatArg::Int(score.time),
                    GameFormatArg::Text(client.name.clone()),
                ],
                1024,
            )
        } else {
            game_format(
                "%5i %4i %4i %s",
                &[
                    GameFormatArg::Int(score.score),
                    GameFormatArg::Int(score.ping),
                    GameFormatArg::Int(score.time),
                    GameFormatArg::Text(client.name.clone()),
                ],
                1024,
            )
        };
        drop(client);
        let ps = self.snapshot();
        if score.client == ps.client_num {
            self.local_client.set(true);
            let rank = if ps.persistant.get(PersistentIndex::Team as i32) == Team::Spectator as i32
                || self.static_state.borrow().game_type >= GameType::Team
            {
                -1
            } else {
                ps.persistant.get(PersistentIndex::Rank as i32) & !0x4000
            };
            let rgb = if rank == 0 {
                vec3(0.0, 0.0, 0.7)
            } else if rank == 1 {
                vec3(0.7, 0.0, 0.0)
            } else if rank == 2 {
                vec3(0.7, 0.7, 0.0)
            } else {
                vec3(0.7, 0.7, 0.7)
            };
            tools.fill_rect(
                rect2d(176.0, y, 512.0, 17.0),
                Some(vec4(rgb.x, rgb.y, rgb.z, fade * 0.7)),
            );
        }
        tools.draw_big_string(160, y as i32, &text, fade);
        let schema = stat_schema(ps.product);
        if ps.stats.get(schema.clients_ready) & (1 << score.client) != 0 {
            tools.draw_big_string_color(icon_x as i32, y as i32, "READY", color);
        }
    }

    /// Draw one team's rows (`teamScoreboard`).
    fn team_scoreboard(&self, y: f32, team: Team, fade: f32, max_clients: i32, line_height: f32) -> i32 {
        let mut count = 0;
        let color = vec4(1.0, 1.0, 1.0, fade);
        let num_scores = self.state.borrow().num_scores;
        for index in 0..num_scores {
            if count >= max_clients {
                break;
            }
            let score = self.score(index);
            if self.client(score.client).borrow().team != team {
                continue;
            }
            self.draw_client_score(
                y + line_height * count as f32,
                &score,
                color,
                fade,
                line_height == NORMAL_HEIGHT,
            );
            count += 1;
        }
        count
    }

    /// Draw the scoreboard (`draw`).
    pub fn draw(&self) -> bool {
        if self.host.cvars.borrow().read_vm_cvar("cg_paused").integer_value != 0 {
            self.state.borrow_mut().deferred_player_loading = 0;
            return false;
        }
        let game_type = self.static_state.borrow().game_type;
        let pm_type = self.state.borrow().predicted_player_state.pm_type;
        if game_type == GameType::SinglePlayer && pm_type == MoveType::Intermission {
            self.state.borrow_mut().deferred_player_loading = 0;
            return false;
        }
        let (warmup, show_scores) = {
            let state = self.state.borrow();
            (state.warmup, state.show_scores)
        };
        if warmup != 0 && !show_scores {
            return false;
        }
        let white = vec4(1.0, 1.0, 1.0, 1.0);
        let color = if show_scores || pm_type == MoveType::Dead || pm_type == MoveType::Intermission {
            Some(white)
        } else {
            let (time, score_fade) = {
                let state = self.state.borrow();
                (state.time, state.score_fade_time)
            };
            fade_color(time, score_fade, 200.0)
        };
        let Some(color) = color else {
            let mut state = self.state.borrow_mut();
            state.deferred_player_loading = 0;
            state.killer_name = String::new();
            return false;
        };
        // Source dereferences RGB[0], not alpha: the old scoreboard disappears at expiry without fading its rows.
        let fade = color.x;
        let tools = self.host.icons.borrow().tools.clone();
        let ps = self.snapshot();
        let killer = self.state.borrow().killer_name.clone();
        if !killer.is_empty() {
            let text = game_format("Fragged by %s", &[GameFormatArg::Text(killer)], 1024);
            tools.draw_big_string((640 - draw_strlen(&text) * 16) / 2, 40, &text, fade);
        }
        let mut rank_text: Option<String> = None;
        if game_type < GameType::Team {
            if ps.persistant.get(PersistentIndex::Team as i32) != Team::Spectator as i32 {
                rank_text = Some(game_format(
                    "%s place with %i",
                    &[
                        GameFormatArg::Text(place_string(
                            ps.persistant.get(PersistentIndex::Rank as i32).wrapping_add(1),
                        )),
                        GameFormatArg::Int(ps.persistant.get(PersistentIndex::Score as i32)),
                    ],
                    1024,
                ));
            }
        } else {
            let team_scores = self.state.borrow().team_scores;
            rank_text = Some(if team_scores[0] == team_scores[1] {
                game_format("Teams are tied at %i", &[GameFormatArg::Int(team_scores[0])], 1024)
            } else if team_scores[0] >= team_scores[1] {
                game_format(
                    "Red leads %i to %i",
                    &[GameFormatArg::Int(team_scores[0]), GameFormatArg::Int(team_scores[1])],
                    1024,
                )
            } else {
                game_format(
                    "Blue leads %i to %i",
                    &[GameFormatArg::Int(team_scores[1]), GameFormatArg::Int(team_scores[0])],
                    1024,
                )
            });
        }
        if let Some(rank_text) = rank_text {
            tools.draw_big_string((640 - draw_strlen(&rank_text) * 16) / 2, 60, &rank_text, fade);
        }
        let graphics = tools.media.borrow().graphics.clone();
        tools.draw_pic(rect2d(176.0, SCOREBOARD_HEADER, 64.0, 32.0), &graphics.scoreboard_score);
        tools.draw_pic(rect2d(264.0, SCOREBOARD_HEADER, 64.0, 32.0), &graphics.scoreboard_ping);
        tools.draw_pic(rect2d(344.0, SCOREBOARD_HEADER, 64.0, 32.0), &graphics.scoreboard_time);
        tools.draw_pic(rect2d(416.0, SCOREBOARD_HEADER, 64.0, 32.0), &graphics.scoreboard_name);
        let num_scores = self.state.borrow().num_scores;
        let compact = num_scores > MAX_NORMAL;
        let line_height = if compact { INTER_HEIGHT } else { NORMAL_HEIGHT };
        let top_border = if compact { 8.0 } else { 16.0 };
        let mut max_clients = if compact { MAX_INTER } else { MAX_NORMAL };
        let mut y = SCOREBOARD_TOP;
        self.local_client.set(false);
        if game_type >= GameType::Team {
            y += line_height / 2.0;
            let team_scores = self.state.borrow().team_scores;
            let first_team = if team_scores[0] >= team_scores[1] {
                Team::Red
            } else {
                Team::Blue
            };
            let first = self.team_scoreboard(y, first_team, fade, max_clients, line_height);
            self.host.icons.borrow().draw_team_background(
                rect2d(0.0, y - top_border, 640.0, first as f32 * line_height + 16.0),
                0.33,
                first_team as i32,
            );
            y += first as f32 * line_height + 16.0;
            max_clients -= first;
            let second_team = if first_team == Team::Red { Team::Blue } else { Team::Red };
            let second = self.team_scoreboard(y, second_team, fade, max_clients, line_height);
            self.host.icons.borrow().draw_team_background(
                rect2d(0.0, y - top_border, 640.0, second as f32 * line_height + 16.0),
                0.33,
                second_team as i32,
            );
            y += second as f32 * line_height + 16.0;
            max_clients -= second;
            y += self.team_scoreboard(y, Team::Spectator, fade, max_clients, line_height) as f32 * line_height + 16.0;
        } else {
            let count = self.team_scoreboard(y, Team::Free, fade, max_clients, line_height);
            y += count as f32 * line_height + 16.0;
            y += self.team_scoreboard(y, Team::Spectator, fade, max_clients - count, line_height) as f32 * line_height
                + 16.0;
        }
        if !self.local_client.get() {
            for index in 0..num_scores {
                let score = self.score(index);
                if score.client == ps.client_num {
                    self.draw_client_score(y, &score, color, fade, line_height == NORMAL_HEIGHT);
                    break;
                }
            }
        }
        self.state.borrow_mut().deferred_player_loading += 1;
        if self.state.borrow().deferred_player_loading > 10 {
            let players = self.host.players.clone();
            self.host.clients.borrow_mut().load_deferred_players(&mut |entity| {
                players.borrow_mut().reset_player_entity(entity);
            });
        }
        true
    }

    /// Center a giant tourney line (`centerGiantLine`).
    fn center_giant_line(&self, y: i32, text: &str) {
        let white = vec4(1.0, 1.0, 1.0, 1.0);
        self.host.icons.borrow().tools.draw_string_ext(&FixedTextOptions {
            x: (0.5 * f64::from(640 - 32 * draw_strlen(text))) as f32,
            y: y as f32,
            text: text.to_string(),
            color: white,
            force_color: true,
            shadow: true,
            char_width: 32,
            char_height: 48,
            max_chars: 0,
        });
    }

    /// Draw the tournament scoreboard (`drawTourney`).
    pub fn draw_tourney(&self) {
        let (request_time, time) = {
            let state = self.state.borrow();
            (state.scores_request_time, state.time)
        };
        if request_time.wrapping_add(2000) < time {
            self.state.borrow_mut().scores_request_time = time;
            self.host.commands.borrow_mut().send_client_command("score");
        }
        let black = vec4(0.0, 0.0, 0.0, 1.0);
        let tools = self.host.icons.borrow().tools.clone();
        tools.fill_rect(rect2d(0.0, 0.0, 640.0, 480.0), Some(black));
        let motd = self.host.strings.borrow().config_string(4);
        self.center_giant_line(8, if motd.is_empty() { "Scoreboard" } else { &motd });
        let mut seconds = time / 1000;
        let minutes = seconds / 60;
        seconds %= 60;
        self.center_giant_line(
            64,
            &game_format(
                "%i:%i%i",
                &[
                    GameFormatArg::Int(minutes),
                    GameFormatArg::Int(seconds / 10),
                    GameFormatArg::Int(seconds % 10),
                ],
                1024,
            ),
        );
        let line = |y: i32, name: &str, score: i32| {
            tools.draw_string_ext(&FixedTextOptions {
                x: 8.0,
                y: y as f32,
                text: name.to_string(),
                color: black,
                force_color: true,
                shadow: true,
                char_width: 32,
                char_height: 48,
                max_chars: 0,
            });
            let text = game_format("%i", &[GameFormatArg::Int(score)], 1024);
            tools.draw_string_ext(&FixedTextOptions {
                x: (632 - 32 * text.chars().count() as i32) as f32,
                y: y as f32,
                text,
                color: black,
                force_color: true,
                shadow: true,
                char_width: 32,
                char_height: 48,
                max_chars: 0,
            });
        };
        if self.static_state.borrow().game_type >= GameType::Team {
            let team_scores = self.state.borrow().team_scores;
            line(160, "Red Team", team_scores[0]);
            line(224, "Blue Team", team_scores[1]);
        } else {
            let mut y = 160;
            for index in 0..64 {
                let client = self.client(index);
                let client = client.borrow();
                if !client.info_valid || client.team != Team::Free {
                    continue;
                }
                // Borrow ends before drawing.
                let (name, score) = (client.name.clone(), client.score);
                drop(client);
                line(y, &name, score);
                y += 64;
            }
        }
    }
}

/// Corner icon size.
const CORNER_ICON_SIZE: f32 = 48.0;
/// Corner field digit size.
const CORNER_CHAR_WIDTH: f32 = 32.0;
const CORNER_CHAR_HEIGHT: f32 = 48.0;
/// Corner big character size.
const CORNER_BIGCHAR_WIDTH: i32 = 16;
const CORNER_BIGCHAR_HEIGHT: f32 = 16.0;
/// Corner tiny character size.
const CORNER_TINYCHAR_WIDTH: i32 = 8;
const CORNER_TINYCHAR_HEIGHT: i32 = 8;
/// Maximum team overlay players.
const MAX_TEAM_OVERLAY_PLAYERS: i32 = 8;
/// Team overlay name width.
const TEAM_OVERLAY_MAXNAME_WIDTH: i32 = 12;
/// Team overlay location width.
const TEAM_OVERLAY_MAXLOCATION_WIDTH: i32 = 16;
/// Maximum locations.
const MAX_LOCATIONS: i32 = 64;
/// Team chat height.
const TEAMCHAT_HEIGHT: i32 = 8;
/// Missing score.
const SCORE_NOT_PRESENT: i32 = -9999;
/// Players configstring base.
pub const CS_PLAYERS: usize = 544;
/// Locations configstring base.
pub const CS_LOCATIONS: usize = 608;
/// Attacker head time.
const ATTACKER_HEAD_TIME: i32 = 10_000;
/// Powerup blinks.
const POWERUP_BLINKS: i32 = 5;
/// Powerup blink time.
const POWERUP_BLINK_TIME: i32 = 1_000;
/// Pulse time.
const PULSE_TIME: i32 = 200;
/// Pulse scale.
const PULSE_SCALE: f32 = 1.5;
/// FPS frames.
const FPS_FRAMES: usize = 4;

/// Powerup draw order (`POWERUPS`).
const CORNER_POWERUPS: [Powerup; 16] = [
    Powerup::None,
    Powerup::Quad,
    Powerup::Battlesuit,
    Powerup::Haste,
    Powerup::Invis,
    Powerup::Regen,
    Powerup::Flight,
    Powerup::RedFlag,
    Powerup::BlueFlag,
    Powerup::NeutralFlag,
    Powerup::Scout,
    Powerup::Guard,
    Powerup::Doubler,
    Powerup::AmmoRegen,
    Powerup::Invulnerability,
    Powerup::NumPowerups,
];

/// HUD corner host services (`ClientHudCornersHost`).
pub struct ClientHudCornersHost {
    /// Cvar reader.
    pub cvars: Shared<dyn HudCvarReader>,
    /// Configstrings.
    pub strings: Shared<dyn HudConfigStrings>,
    /// Clock.
    pub clock: Shared<dyn HudClock>,
}

/// HUD corner drawing (`ClientHudCorners`).
pub struct ClientHudCorners {
    /// Frame state.
    pub state: Shared<ClientGameState>,
    /// Static state.
    pub static_state: Shared<ClientGameStaticState>,
    /// Draw icons.
    pub icons: Shared<ClientDrawIcons>,
    /// Host.
    pub host: ClientHudCornersHost,
    /// Previous frame times.
    previous_times: RefCell<[i32; FPS_FRAMES]>,
    /// FPS index.
    fps_index: Cell<i32>,
    /// Previous milliseconds.
    previous_milliseconds: Cell<i32>,
}

impl ClientHudCorners {
    /// Assemble corner drawing.
    pub fn new(
        state: Shared<ClientGameState>,
        static_state: Shared<ClientGameStaticState>,
        icons: Shared<ClientDrawIcons>,
        host: ClientHudCornersHost,
    ) -> Self {
        if !same(&icons.borrow().state, &state) {
            panic!("HUD corners and icons must share client state");
        }
        if !same(&icons.borrow().tools.media.borrow().static_state, &static_state) {
            panic!("HUD corners and media must share static state");
        }
        if state.borrow().product != static_state.borrow().product
            || state.borrow().product != icons.borrow().tools.media.borrow().product
        {
            panic!("HUD corner products differ");
        }
        Self {
            state,
            static_state,
            icons,
            host,
            previous_times: RefCell::new([0; FPS_FRAMES]),
            fps_index: Cell::new(0),
            previous_milliseconds: Cell::new(0),
        }
    }

    /// Read an integer cvar.
    fn cvar(&self, name: &str) -> i32 {
        self.host.cvars.borrow().read_vm_cvar(name).integer_value
    }

    /// Require the base build.
    fn require_base(&self, source_name: &str) {
        if self.state.borrow().product == Product::Missionpack {
            panic!("{source_name} is not compiled in missionpack");
        }
    }

    /// Current player state.
    fn active_player_state(&self) -> PlayerState {
        self.state
            .borrow()
            .snap
            .clone()
            .unwrap_or_else(|| {
                panic!("HUD corners require a current snapshot");
            })
            .player_state
    }

    /// Draw a numeric field (`drawField`).
    pub fn draw_field(&self, x: f32, y: f32, width: i32, value: i32) {
        if self.state.borrow().product == Product::Missionpack {
            panic!("CG_DrawField is not compiled in missionpack");
        }
        if width < 1 {
            return;
        }
        let width = width.min(5);
        let value = match width {
            1 => value.clamp(0, 9),
            2 => value.clamp(-9, 99),
            3 => value.clamp(-99, 999),
            4 => value.clamp(-999, 9999),
            _ => value,
        };
        let text = game_format("%i", &[GameFormatArg::Int(value)], 16);
        let units: Vec<char> = text.chars().collect();
        let length = units.len().min(width as usize);
        let mut draw_x = x + 2.0 + CORNER_CHAR_WIDTH * (width as f32 - length as f32);
        let icons = self.icons.borrow();
        for unit in units.iter().take(length) {
            let code = *unit as u32;
            let frame = if code == 45 { 10 } else { code as i32 - 48 };
            let shader = icons
                .tools
                .media
                .borrow()
                .graphics
                .number_shaders
                .get(frame as usize)
                .cloned()
                .unwrap_or_else(|| {
                    panic!("Invalid field digit frame");
                });
            icons
                .tools
                .draw_pic(rect2d(draw_x, y, CORNER_CHAR_WIDTH, CORNER_CHAR_HEIGHT), &shader);
            draw_x += CORNER_CHAR_WIDTH;
        }
    }

    /// Draw the upper right (`drawUpperRight`).
    pub fn draw_upper_right(&self) {
        let mut y = 0.0f32;
        if self.static_state.borrow().game_type >= GameType::Team && self.cvar("cg_drawTeamOverlay") == 1 {
            y = self.draw_team_overlay(y, true, true);
        }
        if self.cvar("cg_drawSnapshot") != 0 {
            y = self.draw_snapshot(y);
        }
        if self.cvar("cg_drawFPS") != 0 {
            y = self.draw_fps(y);
        }
        if self.cvar("cg_drawTimer") != 0 {
            y = self.draw_timer(y);
        }
        if self.cvar("cg_drawAttacker") != 0 {
            self.draw_attacker(y);
        }
    }

    /// Draw the lower right (`drawLowerRight`).
    pub fn draw_lower_right(&self) {
        self.require_base("CG_DrawLowerRight");
        let mut y = 480.0 - CORNER_ICON_SIZE;
        if self.static_state.borrow().game_type >= GameType::Team && self.cvar("cg_drawTeamOverlay") == 2 {
            y = self.draw_team_overlay(y, true, false);
        }
        y = self.draw_scores(y);
        self.draw_powerups(y);
    }

    /// Draw the lower left (`drawLowerLeft`).
    pub fn draw_lower_left(&self) {
        self.require_base("CG_DrawLowerLeft");
        let mut y = 480.0 - CORNER_ICON_SIZE;
        if self.static_state.borrow().game_type >= GameType::Team && self.cvar("cg_drawTeamOverlay") == 3 {
            y = self.draw_team_overlay(y, false, false);
        }
        self.draw_pickup_item(y.trunc() as i32);
    }

    /// Draw team info (`drawTeamInfo`).
    pub fn draw_team_info(&self) {
        self.require_base("CG_DrawTeamInfo");
        let chat_height = self.cvar("cg_teamChatHeight").min(TEAMCHAT_HEIGHT);
        if chat_height <= 0 {
            return;
        }
        let (chat_pos, last_chat_pos) = {
            let cgs = self.static_state.borrow();
            (cgs.team_chat_pos, cgs.team_last_chat_pos)
        };
        if last_chat_pos == chat_pos {
            return;
        }
        let oldest = self.static_state.borrow().team_chat_msg_times[(last_chat_pos % chat_height) as usize];
        if self.state.borrow().time.wrapping_sub(oldest) > self.cvar("cg_teamChatTime") {
            self.static_state.borrow_mut().team_last_chat_pos += 1;
        }
        let (chat_pos, last_chat_pos) = {
            let cgs = self.static_state.borrow();
            (cgs.team_chat_pos, cgs.team_last_chat_pos)
        };
        let height = (chat_pos - last_chat_pos) * CORNER_TINYCHAR_HEIGHT;
        let mut width = 0i32;
        for index in last_chat_pos..chat_pos {
            let message = self.static_state.borrow().team_chat_msgs[(index % chat_height) as usize].clone();
            width = width.max(draw_strlen(&message));
        }
        width = width * CORNER_TINYCHAR_WIDTH + CORNER_TINYCHAR_WIDTH * 2;
        let team = self.active_player_state().persistant.get(PersistentIndex::Team as i32);
        let color = if team == Team::Red as i32 {
            vec4(1.0, 0.0, 0.0, 0.33)
        } else if team == Team::Blue as i32 {
            vec4(0.0, 0.0, 1.0, 0.33)
        } else {
            vec4(0.0, 1.0, 0.0, 0.33)
        };
        let icons = self.icons.borrow();
        icons.tools.draw.set_color(Some(color));
        let bar = icons.tools.media.borrow().graphics.team_status_bar.clone();
        icons
            .tools
            .draw_pic(rect2d(0.0, 420.0 - height as f32, 640.0, height as f32), &bar);
        icons.tools.draw.set_color(None);
        for index in (last_chat_pos..chat_pos).rev() {
            let message = self.static_state.borrow().team_chat_msgs[(index % chat_height) as usize].clone();
            icons.tools.draw_string_ext(&FixedTextOptions {
                x: 8.0,
                y: (420 - (chat_pos - index) * 8) as f32,
                text: message,
                color: vec4(1.0, 1.0, 1.0, 1.0),
                force_color: false,
                shadow: false,
                char_width: CORNER_TINYCHAR_WIDTH,
                char_height: CORNER_TINYCHAR_HEIGHT,
                max_chars: 0,
            });
        }
        let _ = width;
    }

    /// Draw the attacker (`drawAttacker`).
    fn draw_attacker(&self, y: f32) -> f32 {
        let snapshot = self.state.borrow().snap.clone().unwrap_or_else(|| {
            panic!("CG_DrawAttacker requires a current snapshot");
        });
        let predicted = self.state.borrow().predicted_player_state.clone();
        let schema = stat_schema(self.state.borrow().product);
        if predicted.stats.get(schema.health) <= 0 || self.state.borrow().attacker_time == 0 {
            return y;
        }
        let client_num = predicted.persistant.get(PersistentIndex::Attacker as i32);
        if client_num < 0 || client_num >= 64 || client_num == snapshot.player_state.client_num {
            return y;
        }
        if self.state.borrow().time.wrapping_sub(self.state.borrow().attacker_time) > ATTACKER_HEAD_TIME {
            self.state.borrow_mut().attacker_time = 0;
            return y;
        }
        let size = CORNER_ICON_SIZE * 1.25;
        self.icons
            .borrow()
            .draw_head(rect2d(640.0 - size, y, size, size), client_num, vec3(0.0, 180.0, 0.0));
        let name = self
            .host
            .strings
            .borrow()
            .config_string(CS_PLAYERS + client_num as usize);
        let name = info_value_for_key(&name, "n", 8192);
        let y = y + size;
        self.icons.borrow().tools.draw_big_string(
            640 - draw_strlen(&name) * CORNER_BIGCHAR_WIDTH,
            y as i32,
            &name,
            0.5,
        );
        y + CORNER_BIGCHAR_HEIGHT + 2.0
    }

    /// Draw the snapshot line (`drawSnapshot`).
    fn draw_snapshot(&self, y: f32) -> f32 {
        let snapshot = self.state.borrow().snap.clone().unwrap_or_else(|| {
            panic!("CG_DrawSnapshot requires a current snapshot");
        });
        let text = game_format(
            "time:%i snap:%i cmd:%i",
            &[
                GameFormatArg::Int(snapshot.server_time),
                GameFormatArg::Int(self.state.borrow().latest_snapshot_num),
                GameFormatArg::Int(self.static_state.borrow().server_command_sequence),
            ],
            1024,
        );
        let width = draw_strlen(&text) * CORNER_BIGCHAR_WIDTH;
        self.icons
            .borrow()
            .tools
            .draw_big_string(635 - width, y as i32 + 2, &text, 1.0);
        y + CORNER_BIGCHAR_HEIGHT + 4.0
    }

    /// Draw FPS (`drawFps`).
    fn draw_fps(&self, y: f32) -> f32 {
        let time = self.host.clock.borrow().milliseconds();
        let frame_time = time.wrapping_sub(self.previous_milliseconds.get());
        self.previous_milliseconds.set(time);
        let index = self.fps_index.get() % FPS_FRAMES as i32;
        self.previous_times.borrow_mut()[index as usize] = frame_time;
        self.fps_index.set(self.fps_index.get() + 1);
        if self.fps_index.get() > FPS_FRAMES as i32 {
            let mut total = 0i32;
            for elapsed in self.previous_times.borrow().iter() {
                total = total.wrapping_add(*elapsed);
            }
            if total == 0 {
                total = 1;
            }
            let fps = (1000 * FPS_FRAMES as i32) / total;
            let text = game_format("%ifps", &[GameFormatArg::Int(fps)], 1024);
            self.icons.borrow().tools.draw_big_string(
                635 - draw_strlen(&text) * CORNER_BIGCHAR_WIDTH,
                y as i32 + 2,
                &text,
                1.0,
            );
        }
        y + CORNER_BIGCHAR_HEIGHT + 4.0
    }

    /// Draw the timer (`drawTimer`).
    fn draw_timer(&self, y: f32) -> f32 {
        let milliseconds = self
            .state
            .borrow()
            .time
            .wrapping_sub(self.static_state.borrow().level_start_time);
        let mut seconds = milliseconds / 1000;
        let minutes = seconds / 60;
        seconds -= minutes * 60;
        let tens = seconds / 10;
        seconds -= tens * 10;
        let text = game_format(
            "%i:%i%i",
            &[
                GameFormatArg::Int(minutes),
                GameFormatArg::Int(tens),
                GameFormatArg::Int(seconds),
            ],
            1024,
        );
        self.icons.borrow().tools.draw_big_string(
            635 - draw_strlen(&text) * CORNER_BIGCHAR_WIDTH,
            y as i32 + 2,
            &text,
            1.0,
        );
        y + CORNER_BIGCHAR_HEIGHT + 4.0
    }

    /// Sorted team client.
    fn team_client(&self, sorted_index: i32) -> Shared<ClientInfo> {
        let number = self
            .state
            .borrow()
            .sorted_team_players
            .get(sorted_index as usize)
            .copied()
            .unwrap_or_else(|| {
                panic!("Invalid sorted team player index");
            });
        self.static_state
            .borrow()
            .client_info
            .get(number as usize)
            .cloned()
            .unwrap_or_else(|| {
                panic!("Invalid sorted team client number");
            })
    }

    /// Draw the team overlay (`drawTeamOverlay`).
    fn draw_team_overlay(&self, y: f32, right: bool, upper: bool) -> f32 {
        if self.cvar("cg_drawTeamOverlay") == 0 {
            return y;
        }
        let player_state = self.active_player_state();
        let team = player_state.persistant.get(PersistentIndex::Team as i32);
        if team != Team::Red as i32 && team != Team::Blue as i32 {
            return y;
        }
        let count = self
            .state
            .borrow()
            .num_sorted_team_players
            .min(MAX_TEAM_OVERLAY_PLAYERS);
        let mut players = 0;
        let mut player_width = 0;
        for index in 0..count {
            let client = self.team_client(index);
            let client = client.borrow();
            if client.info_valid && client.team as i32 == team {
                players += 1;
                player_width = player_width.max(draw_strlen(&client.name));
            }
        }
        if players == 0 {
            return y;
        }
        player_width = player_width.min(TEAM_OVERLAY_MAXNAME_WIDTH);
        let mut location_width = 0;
        for index in 1..MAX_LOCATIONS {
            let location = self.host.strings.borrow().config_string(CS_LOCATIONS + index as usize);
            if !location.is_empty() {
                location_width = location_width.max(draw_strlen(&location));
            }
        }
        location_width = location_width.min(TEAM_OVERLAY_MAXLOCATION_WIDTH);
        let width = (player_width + location_width + 11) * CORNER_TINYCHAR_WIDTH;
        let x = if right { 640 - width } else { 0 };
        let height = players * CORNER_TINYCHAR_HEIGHT;
        let return_y = if upper { y + height as f32 } else { y - height as f32 };
        let mut y = if upper { y } else { y - height as f32 };
        let icons = self.icons.borrow();
        icons.tools.draw.set_color(Some(if team == Team::Red as i32 {
            vec4(1.0, 0.0, 0.0, 0.33)
        } else {
            vec4(0.0, 0.0, 1.0, 0.33)
        }));
        let bar = icons.tools.media.borrow().graphics.team_status_bar.clone();
        icons
            .tools
            .draw_pic(rect2d(x as f32, y, width as f32, height as f32), &bar);
        icons.tools.draw.set_color(None);
        for index in 0..count {
            let client_handle = self.team_client(index);
            let client = client_handle.borrow();
            if !client.info_valid || client.team as i32 != team {
                continue;
            }
            let mut xx = x + CORNER_TINYCHAR_WIDTH;
            icons.tools.draw_string_ext(&FixedTextOptions {
                x: xx as f32,
                y,
                text: client.name.clone(),
                color: vec4(1.0, 1.0, 1.0, 1.0),
                force_color: false,
                shadow: false,
                char_width: 8,
                char_height: 8,
                max_chars: TEAM_OVERLAY_MAXNAME_WIDTH,
            });
            if location_width != 0 {
                let mut location = self
                    .host
                    .strings
                    .borrow()
                    .config_string(CS_LOCATIONS + client.location as usize);
                if location.is_empty() {
                    location = "unknown".to_string();
                }
                xx = x + CORNER_TINYCHAR_WIDTH * 2 + CORNER_TINYCHAR_WIDTH * player_width;
                icons.tools.draw_string_ext(&FixedTextOptions {
                    x: xx as f32,
                    y,
                    text: location,
                    color: vec4(1.0, 1.0, 1.0, 1.0),
                    force_color: false,
                    shadow: false,
                    char_width: 8,
                    char_height: 8,
                    max_chars: TEAM_OVERLAY_MAXLOCATION_WIDTH,
                });
            }
            xx = x
                + CORNER_TINYCHAR_WIDTH * 3
                + CORNER_TINYCHAR_WIDTH * player_width
                + CORNER_TINYCHAR_WIDTH * location_width;
            icons.tools.draw_string_ext(&FixedTextOptions {
                x: xx as f32,
                y,
                text: game_format(
                    "%3i %3i",
                    &[GameFormatArg::Int(client.health), GameFormatArg::Int(client.armor)],
                    16,
                ),
                color: get_color_for_health(client.health, client.armor),
                force_color: false,
                shadow: false,
                char_width: 8,
                char_height: 8,
                max_chars: 0,
            });
            xx += CORNER_TINYCHAR_WIDTH * 3;
            let weapon = icons
                .tools
                .media
                .borrow()
                .weapon_registry
                .borrow()
                .weapon(client.cur_weapon);
            let defer = icons.tools.media.borrow().graphics.defer_shader.clone();
            icons
                .tools
                .draw_pic(rect2d(xx as f32, y, 8.0, 8.0), &weapon.weapon_icon.or(defer));
            xx = if right { x } else { x + width - CORNER_TINYCHAR_WIDTH };
            for powerup in CORNER_POWERUPS {
                if client.powerups & (1 << powerup as i32) == 0 {
                    continue;
                }
                let product = self.state.borrow().product;
                let item = icons
                    .tools
                    .media
                    .borrow()
                    .items
                    .borrow()
                    .find_for_powerup(product, powerup as i32);
                let Some(item) = item else {
                    continue;
                };
                let shader = match item.icon {
                    None => None,
                    Some(icon) => icons.tools.media.borrow().resources.borrow_mut().register_shader(&icon),
                };
                icons.tools.draw_pic(rect2d(xx as f32, y, 8.0, 8.0), &shader);
                xx += if right {
                    -CORNER_TINYCHAR_WIDTH
                } else {
                    CORNER_TINYCHAR_WIDTH
                };
            }
            y += CORNER_TINYCHAR_HEIGHT as f32;
        }
        return_y
    }

    /// Draw scores (`drawScores`).
    fn draw_scores(&self, y: f32) -> f32 {
        self.require_base("CG_DrawScores");
        let player_state = self.active_player_state();
        let score1 = self.static_state.borrow().scores1;
        let mut score2 = self.static_state.borrow().scores2;
        let y = y - CORNER_BIGCHAR_HEIGHT - 8.0;
        let mut y1 = y;
        let mut x = 640;
        let icons = self.icons.borrow();
        if self.static_state.borrow().game_type >= GameType::Team {
            let mut text = game_format("%2i", &[GameFormatArg::Int(score2)], 1024);
            let mut width = draw_strlen(&text) * CORNER_BIGCHAR_WIDTH + 8;
            x -= width;
            icons.tools.fill_rect(
                rect2d(x as f32, y - 4.0, width as f32, CORNER_BIGCHAR_HEIGHT + 8.0),
                Some(vec4(0.0, 0.0, 1.0, 0.33)),
            );
            if player_state.persistant.get(PersistentIndex::Team as i32) == Team::Blue as i32 {
                let select = icons.tools.media.borrow().graphics.select_shader.clone();
                icons.tools.draw_pic(
                    rect2d(x as f32, y - 4.0, width as f32, CORNER_BIGCHAR_HEIGHT + 8.0),
                    &select,
                );
            }
            icons.tools.draw_big_string(x + 4, y as i32, &text, 1.0);
            if self.static_state.borrow().game_type == GameType::Ctf
                && icons
                    .tools
                    .media
                    .borrow()
                    .items
                    .borrow()
                    .find_for_powerup(self.state.borrow().product, Powerup::BlueFlag as i32)
                    .is_some()
            {
                y1 = y - CORNER_BIGCHAR_HEIGHT - 8.0;
                let flag = self.static_state.borrow().blueflag;
                if (0..=2).contains(&flag) {
                    let shader = icons
                        .tools
                        .media
                        .borrow()
                        .graphics
                        .blue_flag_shader
                        .get(flag as usize)
                        .cloned()
                        .unwrap_or_else(|| {
                            panic!("Invalid blue flag status");
                        });
                    icons.tools.draw_pic(
                        rect2d(x as f32, y1 - 4.0, width as f32, CORNER_BIGCHAR_HEIGHT + 8.0),
                        &shader,
                    );
                }
            }
            text = game_format("%2i", &[GameFormatArg::Int(score1)], 1024);
            width = draw_strlen(&text) * CORNER_BIGCHAR_WIDTH + 8;
            x -= width;
            icons.tools.fill_rect(
                rect2d(x as f32, y - 4.0, width as f32, CORNER_BIGCHAR_HEIGHT + 8.0),
                Some(vec4(1.0, 0.0, 0.0, 0.33)),
            );
            if player_state.persistant.get(PersistentIndex::Team as i32) == Team::Red as i32 {
                let select = icons.tools.media.borrow().graphics.select_shader.clone();
                icons.tools.draw_pic(
                    rect2d(x as f32, y - 4.0, width as f32, CORNER_BIGCHAR_HEIGHT + 8.0),
                    &select,
                );
            }
            icons.tools.draw_big_string(x + 4, y as i32, &text, 1.0);
            if self.static_state.borrow().game_type == GameType::Ctf
                && icons
                    .tools
                    .media
                    .borrow()
                    .items
                    .borrow()
                    .find_for_powerup(self.state.borrow().product, Powerup::RedFlag as i32)
                    .is_some()
            {
                y1 = y - CORNER_BIGCHAR_HEIGHT - 8.0;
                let flag = self.static_state.borrow().redflag;
                if (0..=2).contains(&flag) {
                    let shader = icons
                        .tools
                        .media
                        .borrow()
                        .graphics
                        .red_flag_shader
                        .get(flag as usize)
                        .cloned()
                        .unwrap_or_else(|| {
                            panic!("Invalid red flag status");
                        });
                    icons.tools.draw_pic(
                        rect2d(x as f32, y1 - 4.0, width as f32, CORNER_BIGCHAR_HEIGHT + 8.0),
                        &shader,
                    );
                }
            }
            let limit = if self.static_state.borrow().game_type >= GameType::Ctf {
                self.static_state.borrow().capturelimit
            } else {
                self.static_state.borrow().fraglimit
            };
            if limit != 0 {
                text = game_format("%2i", &[GameFormatArg::Int(limit)], 1024);
                let width = draw_strlen(&text) * CORNER_BIGCHAR_WIDTH + 8;
                x -= width;
                icons.tools.draw_big_string(x + 4, y as i32, &text, 1.0);
            }
        } else {
            let score = player_state.persistant.get(PersistentIndex::Score as i32);
            let spectator = player_state.persistant.get(PersistentIndex::Team as i32) == Team::Spectator as i32;
            if score1 != score {
                score2 = score;
            }
            if score2 != SCORE_NOT_PRESENT {
                x = self.draw_free_score(x, y, score2, !spectator && score == score2 && score != score1, false);
            }
            if score1 != SCORE_NOT_PRESENT {
                x = self.draw_free_score(x, y, score1, !spectator && score == score1, true);
            }
            if self.static_state.borrow().fraglimit != 0 {
                let text = game_format("%2i", &[GameFormatArg::Int(self.static_state.borrow().fraglimit)], 1024);
                let width = draw_strlen(&text) * CORNER_BIGCHAR_WIDTH + 8;
                x -= width;
                icons.tools.draw_big_string(x + 4, y as i32, &text, 1.0);
            }
        }
        let _ = x;
        y1 - 8.0
    }

    /// Draw one free-for-all score (`drawFreeScore`).
    fn draw_free_score(&self, x: i32, y: f32, score: i32, selected: bool, first: bool) -> i32 {
        let text = game_format("%2i", &[GameFormatArg::Int(score)], 1024);
        let width = draw_strlen(&text) * CORNER_BIGCHAR_WIDTH + 8;
        let x = x - width;
        let color = if selected {
            if first {
                vec4(0.0, 0.0, 1.0, 0.33)
            } else {
                vec4(1.0, 0.0, 0.0, 0.33)
            }
        } else {
            vec4(0.5, 0.5, 0.5, 0.33)
        };
        let icons = self.icons.borrow();
        icons.tools.fill_rect(
            rect2d(x as f32, y - 4.0, width as f32, CORNER_BIGCHAR_HEIGHT + 8.0),
            Some(color),
        );
        if selected {
            let select = icons.tools.media.borrow().graphics.select_shader.clone();
            icons.tools.draw_pic(
                rect2d(x as f32, y - 4.0, width as f32, CORNER_BIGCHAR_HEIGHT + 8.0),
                &select,
            );
        }
        icons.tools.draw_big_string(x + 4, y as i32, &text, 1.0);
        x
    }

    /// Draw powerups (`drawPowerups`).
    fn draw_powerups(&self, y: f32) -> f32 {
        self.require_base("CG_DrawPowerups");
        let player_state = self.active_player_state();
        let schema = stat_schema(self.state.borrow().product);
        if player_state.stats.get(schema.health) <= 0 {
            return y;
        }
        let mut sorted: Vec<i32> = Vec::new();
        let mut sorted_times: Vec<i32> = Vec::new();
        for powerup in 0..player_state.powerups.len() as i32 {
            let expiration = player_state.powerups.get(powerup);
            if expiration == 0 {
                continue;
            }
            let remaining = expiration.wrapping_sub(self.state.borrow().time);
            if remaining < 0 || remaining > 999_000 {
                continue;
            }
            let mut insertion = 0usize;
            while insertion < sorted_times.len() && sorted_times[insertion] < remaining {
                insertion += 1;
            }
            sorted.insert(insertion, powerup);
            sorted_times.insert(insertion, remaining);
        }
        let x = 640.0 - CORNER_ICON_SIZE - CORNER_CHAR_WIDTH * 2.0;
        let mut y = y;
        for (position, powerup) in sorted.iter().enumerate() {
            let remaining = sorted_times[position];
            let kind = CORNER_POWERUPS.get(*powerup as usize).copied().unwrap_or_else(|| {
                panic!("Invalid powerup slot");
            });
            let product = self.state.borrow().product;
            let item = self
                .icons
                .borrow()
                .tools
                .media
                .borrow()
                .items
                .borrow()
                .find_for_powerup(product, kind as i32);
            let Some(item) = item else {
                continue;
            };
            y -= CORNER_ICON_SIZE;
            self.icons.borrow().tools.draw.set_color(Some(vec4(1.0, 0.2, 0.2, 1.0)));
            self.draw_field(x, y, 2, remaining / 1000);
            let modulation = if remaining < POWERUP_BLINKS * POWERUP_BLINK_TIME {
                let mut fraction = remaining as f32 / POWERUP_BLINK_TIME as f32;
                fraction -= fraction.trunc();
                Some(vec4(fraction, fraction, fraction, fraction))
            } else {
                None
            };
            self.icons.borrow().tools.draw.set_color(modulation);
            let mut size = CORNER_ICON_SIZE;
            let (active, powerup_time, time) = {
                let state = self.state.borrow();
                (state.powerup_active, state.powerup_time, state.time)
            };
            if active == *powerup && time.wrapping_sub(powerup_time) < PULSE_TIME {
                let pulse = 1.0 - (time as f32 - powerup_time as f32) / PULSE_TIME as f32;
                size = CORNER_ICON_SIZE * (1.0 + (PULSE_SCALE - 1.0) * pulse);
            }
            let shader = match item.icon {
                None => None,
                Some(icon) => self
                    .icons
                    .borrow()
                    .tools
                    .media
                    .borrow()
                    .resources
                    .borrow_mut()
                    .register_shader(&icon),
            };
            self.icons.borrow().tools.draw_pic(
                rect2d(640.0 - size, y + CORNER_ICON_SIZE / 2.0 - size / 2.0, size, size),
                &shader,
            );
        }
        self.icons.borrow().tools.draw.set_color(None);
        y
    }

    /// Draw the pickup item (`drawPickupItem`).
    fn draw_pickup_item(&self, y: i32) -> i32 {
        self.require_base("CG_DrawPickupItem");
        let player_state = self.active_player_state();
        let schema = stat_schema(self.state.borrow().product);
        if player_state.stats.get(schema.health) <= 0 {
            return y;
        }
        let y = y - CORNER_ICON_SIZE as i32;
        let value = self.state.borrow().item_pickup;
        if value == 0 {
            return y;
        }
        let (time, pickup_time) = {
            let state = self.state.borrow();
            (state.time, state.item_pickup_time)
        };
        let fade = fade_color(time, pickup_time, 3000.0);
        let Some(fade) = fade else {
            return y;
        };
        let product = self.state.borrow().product;
        let icons = self.icons.borrow();
        let item = icons.tools.media.borrow().items.borrow().at(product, value);
        icons
            .tools
            .media
            .borrow()
            .weapon_registry
            .borrow_mut()
            .register_item_visuals(value);
        let visual = icons
            .tools
            .media
            .borrow()
            .weapon_registry
            .borrow()
            .item_visual(value as usize);
        icons.tools.draw.set_color(Some(fade));
        icons
            .tools
            .draw_pic(rect2d(8.0, y as f32, CORNER_ICON_SIZE, CORNER_ICON_SIZE), &visual.icon);
        icons.tools.draw_big_string(
            CORNER_ICON_SIZE as i32 + 16,
            y + CORNER_ICON_SIZE as i32 / 2 - CORNER_BIGCHAR_WIDTH / 2,
            &item.pickup_name.unwrap_or_default(),
            fade.x,
        );
        icons.tools.draw.set_color(None);
        y
    }
}

/// HUD icon size.
const HUD_ICON: f32 = 48.0;
/// Head damage time.
const HUD_DAMAGE_TIME: f32 = 500.0;

/// HUD host services (`ClientHudHost`).
pub struct ClientHudHost {
    /// Weapon HUD reader.
    pub weapon_hud: Option<Shared<dyn WeaponHudReader>>,
    /// Draw icons.
    pub icons: Shared<ClientDrawIcons>,
    /// Draw status.
    pub status: Shared<ClientDrawStatus>,
    /// Corners.
    pub corners: Shared<ClientHudCorners>,
    /// Prediction.
    pub prediction: Shared<dyn PredictionService>,
    /// Weapons.
    pub weapons: Shared<dyn WeaponService>,
    /// Random.
    pub random: Shared<GameRandom>,
    /// Cvar reader.
    pub cvars: Shared<dyn HudCvarReader>,
    /// Local sounds.
    pub sounds: Shared<dyn HudLocalSound>,
}

/// HUD product variant (`ClientHudVariant`).
pub enum ClientHudVariant {
    /// Base game scoreboard.
    Baseq3 {
        /// Scoreboard.
        scoreboard: Shared<BaseScoreboard>,
    },
    /// Mission pack menus.
    Missionpack {
        /// Fonts (must be the menu's live set).
        fonts: Shared<FontSet>,
        /// Menus.
        menus: Shared<MissionHud>,
    },
}

impl ClientHudVariant {
    /// Product kind.
    #[must_use]
    pub fn kind(&self) -> Product {
        match self {
            Self::Baseq3 { .. } => Product::Baseq3,
            Self::Missionpack { .. } => Product::Missionpack,
        }
    }
}

/// HUD composition (`ClientHud`).
pub struct ClientHud {
    /// Frame state.
    pub state: Shared<ClientGameState>,
    /// Static state.
    pub static_state: Shared<ClientGameStaticState>,
    /// Host.
    pub host: ClientHudHost,
    /// Variant.
    pub variant: ClientHudVariant,
    /// Prox time.
    prox_time: Cell<i32>,
    /// Prox counter.
    prox_counter: Cell<i32>,
    /// Prox tick.
    prox_tick: Cell<i32>,
}

impl ClientHud {
    /// Assemble a HUD.
    pub fn new(
        state: Shared<ClientGameState>,
        static_state: Shared<ClientGameStaticState>,
        host: ClientHudHost,
        variant: ClientHudVariant,
    ) -> Self {
        let icons = host.icons.borrow();
        let status = host.status.borrow();
        let corners = host.corners.borrow();
        if state.borrow().product != static_state.borrow().product
            || variant.kind() != state.borrow().product
            || !same(&icons.state, &state)
            || !same(&icons.tools.media.borrow().static_state, &static_state)
            || !same(&status.state, &state)
            || !status.tools.draw.shares_queue(&icons.tools.draw)
            || !same(&corners.state, &state)
            || !same(&corners.static_state, &static_state)
            || !same(&corners.icons, &host.icons)
            || !same(&host.prediction.borrow().state_handle(), &state)
            || !same(&host.weapons.borrow().state_handle(), &state)
            || !same(
                &host.weapons.borrow().registry_handle(),
                &icons.tools.media.borrow().weapon_registry,
            )
        {
            panic!("HUD services must share canonical cgame state, drawing and weapon media");
        }
        drop(status);
        drop(corners);
        drop(icons);
        match &variant {
            ClientHudVariant::Baseq3 { scoreboard } => {
                let board = scoreboard.borrow();
                if !same(&board.state, &state) || !same(&board.host.icons, &host.icons) {
                    panic!("HUD scoreboard must share canonical drawing and state");
                }
            }
            ClientHudVariant::Missionpack { fonts, menus } => {
                if fonts.borrow().profile != FontProfile::Cgame {
                    panic!("Missionpack HUD requires cgame fonts");
                }
                let menus_borrowed = menus.borrow();
                if !same(&menus_borrowed.state, &state)
                    || !same(&menus_borrowed.static_state, &static_state)
                    || !same(&menus_borrowed.host.borrow().icons(), &host.icons)
                    || !same(&menus_borrowed.fonts_handle, fonts)
                {
                    panic!("Missionpack HUD must share its menu, font and drawing owners");
                }
            }
        }
        Self {
            state,
            static_state,
            host,
            variant,
            prox_time: Cell::new(0),
            prox_counter: Cell::new(0),
            prox_tick: Cell::new(0),
        }
    }

    /// Current player state.
    fn snapshot(&self) -> PlayerState {
        self.state
            .borrow()
            .snap
            .clone()
            .unwrap_or_else(|| {
                panic!("CG_Draw2D requires a current snapshot");
            })
            .player_state
    }

    /// Client slot.
    fn client(&self, index: i32) -> Shared<ClientInfo> {
        self.static_state
            .borrow()
            .client_info
            .get(index as usize)
            .cloned()
            .unwrap_or_else(|| {
                panic!("HUD client index outside source array: {index}");
            })
    }

    /// Reward row.
    fn reward(&self, index: i32) -> ClientReward {
        self.state
            .borrow()
            .rewards
            .get(index as usize)
            .cloned()
            .unwrap_or_else(|| {
                panic!("HUD reward index outside source array: {index}");
            })
    }

    /// Whether a cvar is enabled.
    fn enabled(&self, name: &str) -> bool {
        self.host.cvars.borrow().read_vm_cvar(name).integer_value != 0
    }

    /// Require the base build.
    fn base_only(&self, operation: &str) {
        if !matches!(self.variant, ClientHudVariant::Baseq3 { .. }) {
            panic!("{operation} is excluded from the missionpack build");
        }
    }

    /// Draw the status-bar head (`drawStatusBarHead`).
    pub fn draw_status_bar_head(&self, x: f32) {
        self.base_only("CG_DrawStatusBarHead");
        let mut x = x;
        let time = self.state.borrow().time;
        let damage_time = self.state.borrow().damage_time;
        let mut size = 60.0f32;
        if damage_time != 0 && (time as f32 - damage_time as f32) < HUD_DAMAGE_TIME {
            let frac = (time as f32 - damage_time as f32) / HUD_DAMAGE_TIME;
            size = 60.0 * (1.5 - frac * 0.5);
            let stretch = size - 60.0;
            let damage_x = self.state.borrow().damage_x;
            x -= stretch * 0.5 + damage_x * stretch * 0.5;
            let mut random = self.host.random.borrow_mut();
            self.state.borrow_mut().head_start_yaw = 180.0 + damage_x * 45.0;
            self.state.borrow_mut().head_end_yaw = 180.0 + 20.0 * (random.random() * std::f32::consts::PI).cos();
            self.state.borrow_mut().head_end_pitch = 5.0 * (random.random() * std::f32::consts::PI).cos();
            self.state.borrow_mut().head_start_time = time;
            self.state.borrow_mut().head_end_time = ((time + 100) as f32 + random.random() * 2000.0).trunc() as i32;
        } else if time >= self.state.borrow().head_end_time {
            let (end_yaw, end_pitch, end_time) = {
                let state = self.state.borrow();
                (state.head_end_yaw, state.head_end_pitch, state.head_end_time)
            };
            {
                let mut state = self.state.borrow_mut();
                state.head_start_yaw = end_yaw;
                state.head_start_pitch = end_pitch;
                state.head_start_time = end_time;
            }
            let mut random = self.host.random.borrow_mut();
            self.state.borrow_mut().head_end_time = ((time + 100) as f32 + random.random() * 2000.0).trunc() as i32;
            self.state.borrow_mut().head_end_yaw = 180.0 + 20.0 * (random.random() * std::f32::consts::PI).cos();
            self.state.borrow_mut().head_end_pitch = 5.0 * (random.random() * std::f32::consts::PI).cos();
        }
        if self.state.borrow().head_start_time > time {
            self.state.borrow_mut().head_start_time = time;
        }
        let (start_yaw, end_yaw, start_pitch, end_pitch, start_time, end_time) = {
            let state = self.state.borrow();
            (
                state.head_start_yaw,
                state.head_end_yaw,
                state.head_start_pitch,
                state.head_end_pitch,
                state.head_start_time,
                state.head_end_time,
            )
        };
        let mut frac = (time.wrapping_sub(start_time)) as f32 / end_time.wrapping_sub(start_time) as f32;
        frac = frac * frac * (3.0 - 2.0 * frac);
        let angles = vec3(
            start_pitch + (end_pitch - start_pitch) * frac,
            start_yaw + (end_yaw - start_yaw) * frac,
            0.0,
        );
        let client_num = self.snapshot().client_num;
        self.host
            .icons
            .borrow()
            .draw_head(rect2d(x, 480.0 - size, size, size), client_num, angles);
    }

    /// Draw a status-bar flag (`drawStatusBarFlag`).
    pub fn draw_status_bar_flag(&self, x: f32, team: i32) {
        self.base_only("CG_DrawStatusBarFlag");
        self.host
            .icons
            .borrow()
            .draw_flag_model(rect2d(x, 432.0, HUD_ICON, HUD_ICON), team, false);
    }

    /// Draw the status bar (`drawStatusBar`).
    pub fn draw_status_bar(&self) {
        self.base_only("CG_DrawStatusBar");
        if !self.enabled("cg_drawStatus") {
            return;
        }
        let ps = self.snapshot();
        let predicted = self.state.borrow().predicted_player_state.clone();
        let time = self.state.borrow().time;
        let icons = self.host.icons.borrow();
        let tools = icons.tools.clone();
        let schema = stat_schema(ps.product);
        tools.draw.set_color(None);
        icons.draw_team_background(
            rect2d(0.0, 420.0, 640.0, 60.0),
            0.33,
            ps.persistant.get(PersistentIndex::Team as i32),
        );
        let weapon = self.state.borrow().entity_at(ps.client_num).current_state.weapon;
        let ammo_model = tools.media.borrow().weapon_registry.borrow().weapon(weapon).ammo_model;
        if self.host.weapon_hud.is_none() && weapon != 0 && !ammo_model.is_default() {
            icons.draw_3d_model(
                rect2d(100.0, 432.0, HUD_ICON, HUD_ICON),
                &ammo_model,
                None,
                vec3(70.0, 0.0, 0.0),
                vec3(0.0, 90.0 + 20.0 * (time as f32 / 1000.0).sin(), 0.0),
            );
        }
        drop(icons);
        self.draw_status_bar_head(285.0);
        if predicted.powerups.get(Powerup::RedFlag as i32) != 0 {
            self.draw_status_bar_flag(333.0, Team::Red as i32);
        } else if predicted.powerups.get(Powerup::BlueFlag as i32) != 0 {
            self.draw_status_bar_flag(333.0, Team::Blue as i32);
        } else if predicted.powerups.get(Powerup::NeutralFlag as i32) != 0 {
            self.draw_status_bar_flag(333.0, Team::Free as i32);
        }
        let icons = self.host.icons.borrow();
        let tools = icons.tools.clone();
        if ps.stats.get(schema.armor) != 0 {
            let armor_model = tools.media.borrow().graphics.armor_model.clone();
            icons.draw_3d_model(
                rect2d(470.0, 432.0, HUD_ICON, HUD_ICON),
                &armor_model,
                None,
                vec3(90.0, 0.0, -10.0),
                vec3(0.0, (time & 2047) as f32 * 360.0 / 2048.0, 0.0),
            );
        }
        if self.host.weapon_hud.is_none() && weapon != 0 {
            let ammo = ps.ammo.get(weapon);
            if ammo > -1 {
                let firing = vec4(0.5, 0.5, 0.5, 1.0);
                let normal = vec4(1.0, 0.69, 0.0, 1.0);
                tools.draw.set_color(Some(
                    if predicted.weapon_state == WeaponState::Firing && predicted.weapon_time > 100 {
                        firing
                    } else {
                        normal
                    },
                ));
                self.host.corners.borrow().draw_field(0.0, 432.0, 3, ammo);
                tools.draw.set_color(None);
                if !self.enabled("cg_draw3dIcons") && self.enabled("cg_drawIcons") {
                    let icon = tools
                        .media
                        .borrow()
                        .weapon_registry
                        .borrow()
                        .weapon(predicted.weapon)
                        .ammo_icon;
                    if let Some(icon) = icon {
                        tools.draw_pic(rect2d(100.0, 432.0, HUD_ICON, HUD_ICON), &Some(icon));
                    }
                }
            }
        }
        let health = ps.stats.get(schema.health);
        let low = vec4(1.0, 0.2, 0.2, 1.0);
        let normal = vec4(1.0, 0.69, 0.0, 1.0);
        let white = vec4(1.0, 1.0, 1.0, 1.0);
        tools.draw.set_color(Some(if health > 100 {
            white
        } else if health > 25 {
            normal
        } else if health > 0 {
            if (time >> 8) & 1 != 0 {
                low
            } else {
                normal
            }
        } else {
            low
        }));
        self.host.corners.borrow().draw_field(185.0, 432.0, 3, health);
        tools.draw.set_color(Some(color_for_health(&self.state)));
        let armor = ps.stats.get(schema.armor);
        if armor > 0 {
            tools.draw.set_color(Some(normal));
            self.host.corners.borrow().draw_field(370.0, 432.0, 3, armor);
            tools.draw.set_color(None);
            if !self.enabled("cg_draw3dIcons") && self.enabled("cg_drawIcons") {
                let icon = tools.media.borrow().graphics.armor_icon.clone();
                tools.draw_pic(rect2d(470.0, 432.0, HUD_ICON, HUD_ICON), &icon);
            }
        }
    }

    /// Draw the holdable item (`drawHoldableItem`).
    pub fn draw_holdable_item(&self) {
        self.base_only("CG_DrawHoldableItem");
        let value = self
            .snapshot()
            .stats
            .get(stat_schema(self.state.borrow().product).holdable_item);
        if value == 0 {
            return;
        }
        let registry = self.host.icons.borrow().tools.media.borrow().weapon_registry.clone();
        registry.borrow_mut().register_item_visuals(value);
        let item = registry.borrow().item_visual(value as usize);
        self.host
            .icons
            .borrow()
            .tools
            .draw_pic(rect2d(592.0, 216.0, HUD_ICON, HUD_ICON), &item.icon);
    }

    /// Draw rewards (`drawReward`).
    pub fn draw_reward(&self) {
        if !self.enabled("cg_drawRewards") {
            return;
        }
        let (time, reward_time) = {
            let state = self.state.borrow();
            (state.time, state.reward_time)
        };
        let mut color = fade_color(time, reward_time, 3000.0);
        if color.is_none() {
            if self.state.borrow().reward_stack <= 0 {
                return;
            }
            let stack = self.state.borrow().reward_stack;
            for index in 0..stack {
                let next = self.reward(index + 1);
                self.state.borrow_mut().rewards[index as usize] = next;
            }
            self.state.borrow_mut().reward_time = time;
            self.state.borrow_mut().reward_stack -= 1;
            color = fade_color(time, time, 3000.0);
            let sound = self.reward(0).sound;
            self.host.sounds.borrow_mut().start_local_sound(sound, 7);
        }
        let tools = self.host.icons.borrow().tools.clone();
        tools.draw.set_color(color);
        let reward = self.reward(0);
        if reward.count >= 10 {
            tools.draw_pic(rect2d(296.0, 56.0, 44.0, 44.0), &reward.shader);
            let text = game_format("%d", &[GameFormatArg::Int(reward.count)], 32);
            let color = color.expect("CG_DrawReward: source null text color at zero reward time");
            self.fixed_text(
                (640.0 - 8.0 * draw_strlen(&text) as f32) / 2.0,
                104.0,
                &text,
                8,
                16,
                color,
                false,
            );
        } else {
            let mut x = 320 - reward.count * 24;
            for _ in 0..reward.count {
                tools.draw_pic(rect2d(x as f32, 56.0, 44.0, 44.0), &reward.shader);
                x += HUD_ICON as i32;
            }
        }
        tools.draw.set_color(None);
    }

    /// Draw the crosshair (`drawCrosshair`).
    pub fn draw_crosshair(&self) {
        if !self.enabled("cg_drawCrosshair")
            || self.snapshot().persistant.get(PersistentIndex::Team as i32) == Team::Spectator as i32
            || self.state.borrow().rendering_third_person
        {
            return;
        }
        let tools = self.host.icons.borrow().tools.clone();
        tools.draw.set_color(if self.enabled("cg_crosshairHealth") {
            Some(color_for_health(&self.state))
        } else {
            None
        });
        let mut size = self.host.cvars.borrow().read_vm_cvar("cg_crosshairSize").numeric_value;
        let (time, blend) = {
            let state = self.state.borrow();
            (state.time, state.item_pickup_blend_time)
        };
        let elapsed = time.wrapping_sub(blend) as f32;
        if elapsed > 0.0 && elapsed < 200.0 {
            size *= 1.0 + elapsed / 200.0;
        }
        let rect = tools.adjust_from_640(rect2d(
            self.host.cvars.borrow().read_vm_cvar("cg_crosshairX").integer_value as f32,
            self.host.cvars.borrow().read_vm_cvar("cg_crosshairY").integer_value as f32,
            size,
            size,
        ));
        let index = self
            .host
            .cvars
            .borrow()
            .read_vm_cvar("cg_drawCrosshair")
            .integer_value
            .max(0)
            % 10;
        let shader = tools
            .media
            .borrow()
            .graphics
            .crosshair_shader
            .get(index as usize)
            .cloned()
            .unwrap_or_else(|| {
                panic!("HUD crosshair shader outside source array");
            });
        let view = self.state.borrow().refdef.clone();
        let picture = tools.media.borrow().resources.borrow().picture(&shader);
        tools.draw.stretch_pixels(
            Rect2d {
                x: rect.x + view.x as f32 + 0.5 * (view.width as f32 - rect.width),
                y: rect.y + view.y as f32 + 0.5 * (view.height as f32 - rect.height),
                ..rect
            },
            TextureRect {
                s: 0.0,
                t: 0.0,
                s2: 1.0,
                t2: 1.0,
            },
            picture,
        );
    }

    /// Scan for the crosshair entity (`scanForCrosshairEntity`).
    pub fn scan_for_crosshair_entity(&self) {
        let view = self.state.borrow().refdef.clone();
        let start = view.view_origin;
        let axis = view.view_axis[0];
        let end = vec3(
            start.x + 131072.0 * axis.x,
            start.y + 131072.0 * axis.y,
            start.z + 131072.0 * axis.z,
        );
        let zero = vec3(0.0, 0.0, 0.0);
        let client_num = self.snapshot().client_num;
        let trace = self
            .host
            .prediction
            .borrow()
            .trace(start, end, zero, zero, client_num, 1 | 0x2000000);
        if trace.entity_num >= 64 {
            return;
        }
        if self.host.prediction.borrow().point_contents(trace.end, 0) & 64 != 0 {
            return;
        }
        if self.state.borrow().entity_at(trace.entity_num).current_state.powerups & (1 << Powerup::Invis as i32) != 0 {
            return;
        }
        self.state.borrow_mut().crosshair_client_num = trace.entity_num;
        self.state.borrow_mut().crosshair_client_time = self.state.borrow().time;
    }

    /// Draw crosshair names (`drawCrosshairNames`).
    pub fn draw_crosshair_names(&self) {
        if !self.enabled("cg_drawCrosshair")
            || !self.enabled("cg_drawCrosshairNames")
            || self.state.borrow().rendering_third_person
        {
            return;
        }
        self.scan_for_crosshair_entity();
        let (time, crosshair_time, crosshair_num) = {
            let state = self.state.borrow();
            (state.time, state.crosshair_client_time, state.crosshair_client_num)
        };
        let color = fade_color(time, crosshair_time, 1000.0);
        let Some(color) = color else {
            self.host.icons.borrow().tools.draw.set_color(None);
            return;
        };
        let name = self.client(crosshair_num).borrow().name.clone();
        if matches!(self.variant, ClientHudVariant::Missionpack { .. }) {
            self.proportional(
                &name,
                190.0,
                0.3,
                Vec4 {
                    w: color.w * 0.5,
                    ..color
                },
                3,
            );
        } else {
            self.host
                .icons
                .borrow()
                .tools
                .draw_big_string(320 - draw_strlen(&name) * 8, 170, &name, color.w * 0.5);
        }
        self.host.icons.borrow().tools.draw.set_color(None);
    }

    /// Draw the spectator message (`drawSpectator`).
    pub fn draw_spectator(&self) {
        let tools = self.host.icons.borrow().tools.clone();
        tools.draw_big_string(248, 440, "SPECTATOR", 1.0);
        if self.static_state.borrow().game_type == GameType::Tournament {
            tools.draw_big_string(200, 460, "waiting to play", 1.0);
        } else if self.static_state.borrow().game_type >= GameType::Team {
            tools.draw_big_string(8, 460, "press ESC and use the JOIN menu to play", 1.0);
        }
    }

    /// Draw the vote (`drawVote`).
    pub fn draw_vote(&self) {
        if self.static_state.borrow().vote_time == 0 {
            return;
        }
        if self.static_state.borrow().vote_modified {
            self.static_state.borrow_mut().vote_modified = false;
            let talk = self.host.icons.borrow().tools.media.borrow().sounds.talk_sound.clone();
            self.host.sounds.borrow_mut().start_local_sound(talk, 6);
        }
        let time = self.state.borrow().time;
        let vote_time = self.static_state.borrow().vote_time;
        let sec = (30000 - time.wrapping_sub(vote_time)).max(0) / 1000;
        let cgs = self.static_state.borrow();
        let text = game_format(
            "VOTE(%i):%s yes:%i no:%i",
            &[
                GameFormatArg::Int(sec),
                GameFormatArg::Text(cgs.vote_string.clone()),
                GameFormatArg::Int(cgs.vote_yes),
                GameFormatArg::Int(cgs.vote_no),
            ],
            1024,
        );
        drop(cgs);
        self.host.icons.borrow().tools.draw_small_string(0, 58, &text, 1.0);
        if matches!(self.variant, ClientHudVariant::Missionpack { .. }) {
            self.host
                .icons
                .borrow()
                .tools
                .draw_small_string(0, 76, "or press ESC then click Vote", 1.0);
        }
    }

    /// Draw the team vote (`drawTeamVote`).
    pub fn draw_team_vote(&self) {
        let team = self.client(0).borrow().team;
        if team != Team::Red && team != Team::Blue {
            return;
        }
        let index = if team == Team::Red { 0 } else { 1 };
        if self.static_state.borrow().team_vote_time[index] == 0 {
            return;
        }
        if self.static_state.borrow().team_vote_modified[index] {
            self.static_state.borrow_mut().team_vote_modified[index] = false;
            let talk = self.host.icons.borrow().tools.media.borrow().sounds.talk_sound.clone();
            self.host.sounds.borrow_mut().start_local_sound(talk, 6);
        }
        let time = self.state.borrow().time;
        let vote_time = self.static_state.borrow().team_vote_time[index];
        let sec = (30000 - time.wrapping_sub(vote_time)).max(0) / 1000;
        let cgs = self.static_state.borrow();
        let text = game_format(
            "TEAMVOTE(%i):%s yes:%i no:%i",
            &[
                GameFormatArg::Int(sec),
                GameFormatArg::Text(cgs.team_vote_string[index].clone()),
                GameFormatArg::Int(cgs.team_vote_yes[index]),
                GameFormatArg::Int(cgs.team_vote_no[index]),
            ],
            1024,
        );
        drop(cgs);
        self.host.icons.borrow().tools.draw_small_string(0, 90, &text, 1.0);
    }

    /// Draw the follow message (`drawFollow`).
    pub fn draw_follow(&self) -> bool {
        let ps = self.snapshot();
        if ps.pm_flags & MOVE_FLAG_FOLLOW == 0 {
            return false;
        }
        let tools = self.host.icons.borrow().tools.clone();
        tools.draw_big_string(248, 24, "following", 1.0);
        let name = self.client(ps.client_num).borrow().name.clone();
        self.fixed_text(
            0.5 * (640.0 - 32.0 * draw_strlen(&name) as f32),
            40.0,
            &name,
            32,
            48,
            vec4(1.0, 1.0, 1.0, 1.0),
            true,
        );
        true
    }

    /// Draw the ammo warning (`drawAmmoWarning`).
    pub fn draw_ammo_warning(&self) {
        if self.host.weapon_hud.is_some()
            || !self.enabled("cg_drawAmmoWarning")
            || self.state.borrow().low_ammo_warning == 0
        {
            return;
        }
        let text = if self.state.borrow().low_ammo_warning == 2 {
            "OUT OF AMMO"
        } else {
            "LOW AMMO WARNING"
        };
        self.host
            .icons
            .borrow()
            .tools
            .draw_big_string(320 - draw_strlen(text) * 8, 64, text, 1.0);
    }

    /// Draw the prox warning (`drawProxWarning`).
    pub fn draw_prox_warning(&self) {
        if !matches!(self.variant, ClientHudVariant::Missionpack { .. }) {
            panic!("CG_DrawProxWarning is excluded from the baseq3 build");
        }
        if self.snapshot().e_flags & 2 == 0 {
            self.prox_time.set(0);
            return;
        }
        let time = self.state.borrow().time;
        if self.prox_time.get() == 0 {
            self.prox_time.set(time.wrapping_add(5000));
            self.prox_counter.set(5);
            self.prox_tick.set(0);
        }
        if time > self.prox_time.get() {
            self.prox_tick.set(self.prox_counter.get());
            self.prox_counter.set(self.prox_counter.get() - 1);
            self.prox_time.set(time.wrapping_add(1000));
        }
        let text = if self.prox_tick.get() != 0 {
            game_format(
                "INTERNAL COMBUSTION IN: %i",
                &[GameFormatArg::Int(self.prox_tick.get())],
                32,
            )
        } else {
            "YOU HAVE BEEN MINED".to_string()
        };
        self.host.icons.borrow().tools.draw_big_string_color(
            320 - draw_strlen(&text) * 8,
            80,
            &text,
            vec4(1.0, 0.0, 0.0, 1.0),
        );
    }

    /// Fixed text helper.
    fn fixed_text(
        &self,
        x: f32,
        y: f32,
        text: &str,
        char_width: i32,
        char_height: i32,
        color: Vec4,
        force_color: bool,
    ) {
        self.host.icons.borrow().tools.draw_string_ext(&FixedTextOptions {
            x: x.trunc(),
            y,
            text: text.to_string(),
            color,
            char_width,
            char_height,
            force_color,
            shadow: true,
            max_chars: 0,
        });
    }

    /// Proportional text helper.
    fn proportional(&self, text: &str, y: f32, scale: f32, color: Vec4, style: i32) {
        self.proportional_sized(text, y, scale, color, style, false);
    }

    /// Proportional text helper with integer-width option.
    fn proportional_sized(&self, text: &str, y: f32, scale: f32, color: Vec4, style: i32, integer_width: bool) {
        let ClientHudVariant::Missionpack { fonts, .. } = &self.variant else {
            panic!("Proportional HUD text requires missionpack fonts");
        };
        let fonts = fonts.borrow();
        let width = text_width(&fonts, text, scale, 0);
        let half = if integer_width {
            (width / 2) as f32
        } else {
            width as f32 / 2.0
        };
        text_paint(
            &self.host.icons.borrow().tools.draw,
            &fonts,
            &TextPaintOptions {
                x: 320.0 - half,
                y,
                scale,
                color,
                text: text.to_string(),
                adjust: 0.0,
                limit: 0,
                style,
            },
        );
    }

    /// Draw warmup (`drawWarmup`).
    pub fn draw_warmup(&self) {
        let (warmup, time) = {
            let state = self.state.borrow();
            (state.warmup, state.time)
        };
        if warmup == 0 {
            return;
        }
        if warmup < 0 {
            let text = "Waiting for players";
            self.host
                .icons
                .borrow()
                .tools
                .draw_big_string(320 - draw_strlen(text) * 8, 24, text, 1.0);
            self.state.borrow_mut().warmup_count = 0;
            return;
        }
        let game_type = self.static_state.borrow().game_type;
        let mut heading = String::new();
        let mut draw_heading = true;
        if game_type == GameType::Tournament {
            let maxclients = self.static_state.borrow().maxclients;
            let mut first: Option<String> = None;
            let mut second: Option<String> = None;
            for index in 0..maxclients {
                let client = self.client(index);
                let client = client.borrow();
                if client.info_valid && client.team == Team::Free {
                    if first.is_none() {
                        first = Some(client.name.clone());
                    } else {
                        second = Some(client.name.clone());
                    }
                }
            }
            match (first, second) {
                (Some(first), Some(second)) => {
                    heading = game_format(
                        "%s vs %s",
                        &[GameFormatArg::Text(first), GameFormatArg::Text(second)],
                        1024,
                    );
                }
                _ => draw_heading = false,
            }
        } else if game_type == GameType::Ffa {
            heading = "Free For All".to_string();
        } else if game_type == GameType::Team {
            heading = "Team Deathmatch".to_string();
        } else if game_type == GameType::Ctf {
            heading = "Capture the Flag".to_string();
        } else if matches!(self.variant, ClientHudVariant::Missionpack { .. }) {
            if game_type == GameType::OneFlagCtf {
                heading = "One Flag CTF".to_string();
            } else if game_type == GameType::Obelisk {
                heading = "Overload".to_string();
            } else if game_type == GameType::Harvester {
                heading = "Harvester".to_string();
            }
        }
        if draw_heading {
            let tournament = game_type == GameType::Tournament;
            if matches!(self.variant, ClientHudVariant::Missionpack { .. }) {
                self.proportional_sized(
                    &heading,
                    if tournament { 60.0 } else { 90.0 },
                    0.6,
                    vec4(1.0, 1.0, 1.0, 1.0),
                    6,
                    true,
                );
            } else {
                let width = draw_strlen(&heading);
                let cw = if width > 20 { 640 / width } else { 32 };
                self.fixed_text(
                    (320 - width * cw / 2) as f32,
                    if tournament { 20.0 } else { 25.0 },
                    &heading,
                    cw,
                    (cw as f32 * if tournament { 1.5 } else { 1.1 }) as i32,
                    vec4(1.0, 1.0, 1.0, 1.0),
                    false,
                );
            }
        }
        let mut sec = warmup.wrapping_sub(time) / 1000;
        if sec < 0 {
            self.state.borrow_mut().warmup = 0;
            sec = 0;
        }
        let text = game_format("Starts in: %i", &[GameFormatArg::Int(sec.wrapping_add(1))], 1024);
        if sec != self.state.borrow().warmup_count {
            self.state.borrow_mut().warmup_count = sec;
            let sounds = self.host.icons.borrow().tools.media.borrow().sounds.clone();
            if sec == 0 {
                self.host.sounds.borrow_mut().start_local_sound(sounds.count1_sound, 7);
            } else if sec == 1 {
                self.host.sounds.borrow_mut().start_local_sound(sounds.count2_sound, 7);
            } else if sec == 2 {
                self.host.sounds.borrow_mut().start_local_sound(sounds.count3_sound, 7);
            }
        }
        let count = self.state.borrow().warmup_count;
        let cw = if count == 0 {
            28
        } else if count == 1 {
            24
        } else if count == 2 {
            20
        } else {
            16
        };
        if matches!(self.variant, ClientHudVariant::Missionpack { .. }) {
            let scale = if count == 0 {
                0.54
            } else if count == 1 {
                0.51
            } else if count == 2 {
                0.48
            } else {
                0.45
            };
            self.proportional_sized(&text, 125.0, scale, vec4(1.0, 1.0, 1.0, 1.0), 6, true);
        } else {
            self.fixed_text(
                (320 - draw_strlen(&text) * cw / 2) as f32,
                70.0,
                &text,
                cw,
                (cw as f32 * 1.5) as i32,
                vec4(1.0, 1.0, 1.0, 1.0),
                false,
            );
        }
    }

    /// Draw timed menus (`drawTimedMenus`).
    pub fn draw_timed_menus(&self) {
        let ClientHudVariant::Missionpack { menus, .. } = &self.variant else {
            panic!("CG_DrawTimedMenus is excluded from baseq3");
        };
        menus.borrow().draw_timed_menus();
    }

    /// Draw the scoreboard (`drawScoreboard`).
    pub fn draw_scoreboard(&self) -> bool {
        match &self.variant {
            ClientHudVariant::Missionpack { menus, .. } => menus.borrow().draw_scoreboard(),
            ClientHudVariant::Baseq3 { scoreboard } => scoreboard.borrow().draw(),
        }
    }

    /// Draw intermission (`drawIntermission`).
    pub fn draw_intermission(&self) {
        if matches!(self.variant, ClientHudVariant::Baseq3 { .. })
            && self.static_state.borrow().game_type == GameType::SinglePlayer
        {
            self.host.status.borrow().draw_center_string();
            return;
        }
        self.state.borrow_mut().score_fade_time = self.state.borrow().time;
        let showing = self.draw_scoreboard();
        self.state.borrow_mut().score_board_showing = showing;
    }

    /// Draw the tourney scoreboard (`drawTourneyScoreboard`).
    pub fn draw_tourney_scoreboard(&self) {
        if let ClientHudVariant::Baseq3 { scoreboard } = &self.variant {
            scoreboard.borrow().draw_tourney();
        }
    }

    /// Draw all 2D elements (`draw2D`).
    pub fn draw_2d(&self) {
        if matches!(self.variant, ClientHudVariant::Missionpack { .. })
            && self.static_state.borrow().order_pending
            && self.state.borrow().time > self.static_state.borrow().order_time
        {
            let ClientHudVariant::Missionpack { menus, .. } = &self.variant else {
                unreachable!();
            };
            menus.borrow().check_order_pending();
        }
        if self.state.borrow().level_shot || !self.enabled("cg_draw2D") {
            return;
        }
        let ps = self.snapshot();
        if ps.pm_type == MoveType::Intermission {
            self.draw_intermission();
            return;
        }
        if ps.persistant.get(PersistentIndex::Team as i32) == Team::Spectator as i32 {
            self.draw_spectator();
            self.draw_crosshair();
            self.draw_crosshair_names();
        } else if !self.state.borrow().show_scores && ps.stats.get(stat_schema(ps.product).health) > 0 {
            if matches!(self.variant, ClientHudVariant::Missionpack { .. }) {
                if self.enabled("cg_drawStatus") {
                    let ClientHudVariant::Missionpack { menus, .. } = &self.variant else {
                        unreachable!();
                    };
                    menus.borrow().paint_all();
                    self.draw_timed_menus();
                }
            } else {
                self.draw_status_bar();
            }
            self.draw_ammo_warning();
            if matches!(self.variant, ClientHudVariant::Missionpack { .. }) {
                self.draw_prox_warning();
            }
            self.draw_crosshair();
            self.draw_crosshair_names();
            if self.host.weapon_hud.is_none() {
                self.host.weapons.borrow_mut().draw_weapon_select();
            }
            if matches!(self.variant, ClientHudVariant::Baseq3 { .. }) {
                self.draw_holdable_item();
            }
            self.draw_reward();
        }
        if self.static_state.borrow().game_type >= GameType::Team
            && matches!(self.variant, ClientHudVariant::Baseq3 { .. })
        {
            self.host.corners.borrow().draw_team_info();
        }
        self.draw_vote();
        self.draw_team_vote();
        self.host.status.borrow().draw_lagometer();
        if matches!(self.variant, ClientHudVariant::Baseq3 { .. }) || !self.enabled("cg_paused") {
            self.host.corners.borrow().draw_upper_right();
        }
        if matches!(self.variant, ClientHudVariant::Baseq3 { .. }) {
            self.host.corners.borrow().draw_lower_right();
            self.host.corners.borrow().draw_lower_left();
        }
        if !self.draw_follow() {
            self.draw_warmup();
        }
        let showing = self.draw_scoreboard();
        self.state.borrow_mut().score_board_showing = showing;
        if !self.state.borrow().score_board_showing {
            self.host.status.borrow().draw_center_string();
        }
    }
}

/// Menu end sound name (`"winnerSound" | "loserSound"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuEndSound {
    /// Winner.
    Winner,
    /// Loser.
    Loser,
}

/// Available console HUD (`ClientConsoleHud` available arm).
pub trait ConsoleHud {
    /// Reset strings.
    fn reset_strings(&mut self);
    /// Reset menus.
    fn reset_menus(&mut self);
    /// Load menus.
    fn load_menus(&mut self, path: &str);
    /// Clear the scoreboard.
    fn clear_scoreboard(&mut self);
    /// Menu scoreboard.
    fn menu_scoreboard(&self) -> Option<CapturedMenu>;
    /// Scroll a feeder.
    fn scroll_feeder(&mut self, menu: &CapturedMenu, feeder: i32, down: bool);
}

/// Console HUD access (`ClientConsoleHud`).
#[derive(Clone)]
pub enum ConsoleHudAccess {
    /// Unavailable with a reason.
    Unavailable {
        /// Reason.
        reason: String,
    },
    /// Available HUD.
    Available(Shared<dyn ConsoleHud>),
}

/// Available team orders (`ClientConsoleTeamOrders` available arm).
pub trait ConsoleOrders {
    /// Select the next player.
    fn select_next_player(&mut self);
    /// Select the previous player.
    fn select_previous_player(&mut self);
    /// Whether the other team has the flag.
    fn other_team_has_flag(&self) -> bool;
    /// Whether your team has the flag.
    fn your_team_has_flag(&self) -> bool;
}

/// Team orders access (`ClientConsoleTeamOrders`).
#[derive(Clone)]
pub enum ConsoleOrdersAccess {
    /// Unavailable with a reason.
    Unavailable {
        /// Reason.
        reason: String,
    },
    /// Available orders.
    Available(Shared<dyn ConsoleOrders>),
}

/// Console host services (`ClientConsoleHost`).
pub trait ClientConsoleHost {
    /// Cvar registry.
    fn cvars(&self) -> Shared<CvarRegistry>;
    /// View runtime.
    fn view(&self) -> Shared<dyn ViewService>;
    /// Weapons.
    fn weapons(&self) -> Shared<dyn WeaponService>;
    /// Client store.
    fn clients(&self) -> Shared<dyn ClientInfoStore>;
    /// Server commands.
    fn server_commands(&self) -> Shared<dyn ServerCommandService>;
    /// HUD access.
    fn hud(&self) -> ConsoleHudAccess;
    /// Team orders access.
    fn team_orders(&self) -> ConsoleOrdersAccess;
    /// Read a cached VM cvar.
    fn read_vm_cvar(&self, name: &str) -> CvarSnapshot;
    /// Reset a player entity.
    fn reset_player_entity(&mut self, entity: &mut ClientEntity);
    /// Register a command name.
    fn add_command(&mut self, name: &str);
    /// Send a client command.
    fn send_client_command(&mut self, text: &str);
    /// Send a console command.
    fn send_console_command(&mut self, text: &str);
    /// Print.
    fn print(&mut self, text: &str);
    /// Center print.
    fn center_print(&mut self, text: &str, y: i32, char_width: i32);
    /// Menu end sound.
    fn sound(&self, name: MenuEndSound) -> Option<PcmSound>;
    /// Buffer a sound.
    fn add_buffered_sound(&mut self, sound: Option<PcmSound>);
}

/// Common console commands (`COMMON_COMMANDS`).
pub const COMMON_COMMANDS: [&str; 21] = [
    "testgun",
    "testmodel",
    "nextframe",
    "prevframe",
    "nextskin",
    "prevskin",
    "viewpos",
    "+scores",
    "-scores",
    "+zoom",
    "-zoom",
    "sizeup",
    "sizedown",
    "weapnext",
    "weapprev",
    "weapon",
    "tell_target",
    "tell_attacker",
    "vtell_target",
    "vtell_attacker",
    "tcmd",
];

/// Mission console commands (`MISSION_COMMANDS`).
pub const MISSION_COMMANDS: [&str; 24] = [
    "loadhud",
    "nextTeamMember",
    "prevTeamMember",
    "nextOrder",
    "confirmOrder",
    "denyOrder",
    "taskOffense",
    "taskDefense",
    "taskPatrol",
    "taskCamp",
    "taskFollow",
    "taskRetrieve",
    "taskEscort",
    "taskSuicide",
    "taskOwnFlag",
    "tauntKillInsult",
    "tauntPraise",
    "tauntTaunt",
    "tauntDeathInsult",
    "tauntGauntlet",
    "spWin",
    "spLose",
    "scoresDown",
    "scoresUp",
];

/// Forwarded console commands (`FORWARDED_COMMANDS`).
pub const FORWARDED_COMMANDS: [&str; 27] = [
    "kill",
    "say",
    "say_team",
    "tell",
    "vsay",
    "vsay_team",
    "vtell",
    "vtaunt",
    "vosay",
    "vosay_team",
    "votell",
    "give",
    "god",
    "notarget",
    "noclip",
    "team",
    "follow",
    "levelshot",
    "addbot",
    "setviewpos",
    "callvote",
    "vote",
    "callteamvote",
    "teamvote",
    "stats",
    "teamtask",
    "loaddefered",
];

/// Local command names (`localCommandNames`).
#[must_use]
pub fn local_command_names(product: Product) -> Vec<String> {
    let mut names: Vec<String> = COMMON_COMMANDS.iter().map(ToString::to_string).collect();
    if product == Product::Missionpack {
        names.extend(MISSION_COMMANDS.iter().map(ToString::to_string));
    }
    names.push("startOrbit".to_string());
    names.push("loaddeferred".to_string());
    names
}

/// All console command names (`clientConsoleCommandNames`).
#[must_use]
pub fn client_console_command_names(product: Product) -> Vec<String> {
    let mut names = local_command_names(product);
    names.extend(FORWARDED_COMMANDS.iter().map(ToString::to_string));
    names
}

/// ASCII fold (`fold`).
fn fold_command(value: &str) -> String {
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
}

/// Validate source bytes cut at NUL (`sourceBytes`).
fn console_source_bytes(value: &str) -> Result<String, HudError> {
    let cut = match value.find('\0') {
        Some(end) => &value[..end],
        None => value,
    };
    for ch in cut.chars() {
        if ch as u32 > 255 {
            return Err(HudError::new("Console commands require source byte characters"));
        }
    }
    Ok(cut.to_string())
}

/// Command argument (`argument`).
fn console_argument(argv: &[String], index: usize, size: usize) -> String {
    argv.get(index)
        .map(|value| value.chars().take(size - 1).collect())
        .unwrap_or_default()
}

/// Console runtime (`ClientConsoleRuntime`).
pub struct ClientConsoleRuntime {
    /// Frame state.
    pub state: Shared<ClientGameState>,
    /// Static state.
    pub static_state: Shared<ClientGameStaticState>,
    /// Host.
    pub host: Shared<dyn ClientConsoleHost>,
    /// Local command names.
    commands: Vec<String>,
    /// Closed.
    closed: Cell<bool>,
}

impl ClientConsoleRuntime {
    /// Assemble a console runtime.
    pub fn new(
        state: Shared<ClientGameState>,
        static_state: Shared<ClientGameStaticState>,
        host: Shared<dyn ClientConsoleHost>,
    ) -> Self {
        if state.borrow().product != static_state.borrow().product
            || !same(&host.borrow().view().borrow().state_handle(), &state)
            || !same(&host.borrow().weapons().borrow().state_handle(), &state)
        {
            panic!("Console services must share canonical cgame state");
        }
        let commands = local_command_names(state.borrow().product);
        Self {
            state,
            static_state,
            host,
            commands,
            closed: Cell::new(false),
        }
    }

    /// Register command names (`initializeCommands`).
    pub fn initialize_commands(&self) {
        self.open();
        for name in client_console_command_names(self.state.borrow().product) {
            self.host.borrow_mut().add_command(&name);
        }
    }

    /// Dispose (`dispose`).
    pub fn dispose(&self) {
        self.closed.set(true);
        self.host.borrow_mut().view().borrow_mut().clear_test_model();
        self.host.borrow_mut().clients().borrow_mut().reset();
    }

    /// Require an open runtime (`open`).
    fn open(&self) {
        if self.closed.get() {
            panic!("Cgame console runtime is closed");
        }
    }

    /// Whether a name is handled (`handles`).
    pub fn handles(&self, name: &str) -> bool {
        self.open();
        let parsed = console_source_bytes(name);
        let Ok(parsed) = parsed else {
            panic!("Console commands require source byte characters");
        };
        let key = fold_command(&parsed);
        self.commands.iter().any(|command| fold_command(command) == key)
    }

    /// Execute a command (`execute`).
    pub fn execute(&self, argv: &[String]) -> Result<bool, HudError> {
        if argv.len() > 1024 {
            return Err(HudError::new("Console command exceeds MAX_STRING_TOKENS"));
        }
        let mut owned = Vec::with_capacity(argv.len());
        let mut storage = 0usize;
        for value in argv {
            let value = console_source_bytes(value)?;
            storage += value.chars().count() + 1;
            owned.push(value);
        }
        if storage > 8192 + 1024 {
            return Err(HudError::new("Console command exceeds source token storage"));
        }
        self.open();
        let name = fold_command(&console_argument(&owned, 0, 1024));
        if !self.handles(&name) {
            return Ok(false);
        }
        match self.dispatch(&name, &owned) {
            Ok(()) => {
                self.open();
                Ok(true)
            }
            Err(error) => {
                self.dispose();
                Err(error)
            }
        }
    }

    /// Crosshair player (`crosshairPlayer`).
    #[must_use]
    pub fn crosshair_player(&self) -> i32 {
        let state = self.state.borrow();
        if state.time > state.crosshair_client_time.wrapping_add(1000) {
            -1
        } else {
            state.crosshair_client_num
        }
    }

    /// Last attacker (`lastAttacker`).
    pub fn last_attacker(&self) -> Result<i32, HudError> {
        if self.state.borrow().attacker_time == 0 {
            return Ok(-1);
        }
        let snapshot = self.state.borrow().snap.clone();
        let Some(snapshot) = snapshot else {
            return Err(HudError::new("CG_LastAttacker requires cg.snap"));
        };
        Ok(snapshot.player_state.persistant.get(PersistentIndex::Attacker as i32))
    }

    /// Set a cvar.
    fn set(&self, name: &str, value: &str) -> Result<(), HudError> {
        self.host
            .borrow_mut()
            .cvars()
            .borrow_mut()
            .set(name, value, true)
            .map(|_| ())
            .map_err(|error| HudError::new(error.to_string()))
    }

    /// Immediate cvar text.
    fn immediate(&self, name: &str) -> Result<String, HudError> {
        let value = self.host.borrow().cvars().borrow().get(name);
        match value {
            None => Ok(String::new()),
            Some(snapshot) => Ok(console_source_bytes(&snapshot.value)?.chars().take(1023).collect()),
        }
    }

    /// Available HUD or rejection.
    fn hud(&self) -> Result<Shared<dyn ConsoleHud>, HudError> {
        match self.host.borrow().hud() {
            ConsoleHudAccess::Available(hud) => Ok(hud),
            ConsoleHudAccess::Unavailable { reason } => Err(HudError::new(format!("Cgame HUD unavailable: {reason}"))),
        }
    }

    /// Available orders or rejection.
    fn orders(&self) -> Result<Shared<dyn ConsoleOrders>, HudError> {
        match self.host.borrow().team_orders() {
            ConsoleOrdersAccess::Available(orders) => Ok(orders),
            ConsoleOrdersAccess::Unavailable { reason } => {
                Err(HudError::new(format!("Cgame team orders unavailable: {reason}")))
            }
        }
    }

    /// Scores down (`scoresDown`).
    fn scores_down(&self) {
        if self.state.borrow().product == Product::Missionpack {
            self.host
                .borrow_mut()
                .server_commands()
                .borrow_mut()
                .build_spectator_string();
        }
        let (request_time, time) = {
            let state = self.state.borrow();
            (state.scores_request_time, state.time)
        };
        if request_time.wrapping_add(2000) < time {
            self.state.borrow_mut().scores_request_time = time;
            self.host.borrow_mut().send_client_command("score");
            if !self.state.borrow().show_scores {
                self.state.borrow_mut().show_scores = true;
                self.state.borrow_mut().num_scores = 0;
            }
        } else {
            self.state.borrow_mut().show_scores = true;
        }
    }

    /// Next order (`nextOrder`).
    fn next_order(&self) -> Result<(), HudError> {
        let snapshot = self.state.borrow().snap.clone();
        let Some(snapshot) = snapshot else {
            return Err(HudError::new("CG_NextOrder requires cg.snap"));
        };
        let client = self
            .static_state
            .borrow()
            .client_info
            .get(snapshot.player_state.client_num as usize)
            .cloned()
            .ok_or_else(|| {
                HudError::new(format!(
                    "Console source array index {} outside 64",
                    snapshot.player_state.client_num
                ))
            })?;
        let selected = self
            .host
            .borrow()
            .read_vm_cvar("cg_currentSelectedPlayer")
            .integer_value;
        let selected_client = self
            .state
            .borrow()
            .sorted_team_players
            .get(selected as usize)
            .copied()
            .ok_or_else(|| HudError::new(format!("Console source array index {selected} outside 8")))?;
        if !client.borrow().team_leader && selected_client != snapshot.player_state.client_num {
            return Ok(());
        }
        let current = self.static_state.borrow().current_order;
        if current < 7 {
            let mut next = current + 1;
            if next == 5 && !self.orders()?.borrow().other_team_has_flag() {
                next += 1;
            }
            if next == 6 && !self.orders()?.borrow().your_team_has_flag() {
                next += 1;
            }
            self.static_state.borrow_mut().current_order = next;
        } else {
            self.static_state.borrow_mut().current_order = 1;
        }
        self.static_state.borrow_mut().order_pending = true;
        let time = self.state.borrow().time;
        self.static_state.borrow_mut().order_time = time.wrapping_add(3000);
        Ok(())
    }

    /// Send a team task (`task`).
    fn task(&self, voice: &str, task: i32) {
        self.host
            .borrow_mut()
            .send_console_command(&format!("cmd vsay_team {voice}\n"));
        self.host
            .borrow_mut()
            .send_client_command(&game_format("teamtask %d\n", &[GameFormatArg::Int(task)], 1024));
    }

    /// Dispatch a command (`dispatch`).
    fn dispatch(&self, name: &str, argv: &[String]) -> Result<(), HudError> {
        let game_type = self.static_state.borrow().game_type;
        match name {
            "testgun" => {
                let model = if argv.len() < 2 {
                    None
                } else {
                    Some(console_argument(argv, 1, 1024))
                };
                let param = if argv.len() == 3 {
                    Some(game_atof(&console_argument(argv, 2, 1024)))
                } else {
                    None
                };
                self.host.borrow_mut().view().borrow_mut().test_gun(model, param);
            }
            "testmodel" => {
                let model = if argv.len() < 2 {
                    None
                } else {
                    Some(console_argument(argv, 1, 1024))
                };
                let param = if argv.len() == 3 {
                    Some(game_atof(&console_argument(argv, 2, 1024)))
                } else {
                    None
                };
                self.host.borrow_mut().view().borrow_mut().test_model(model, param);
            }
            "nextframe" => self.host.borrow_mut().view().borrow_mut().next_model_frame(),
            "prevframe" => self.host.borrow_mut().view().borrow_mut().previous_model_frame(),
            "nextskin" => self.host.borrow_mut().view().borrow_mut().next_model_skin(),
            "prevskin" => self.host.borrow_mut().view().borrow_mut().previous_model_skin(),
            "+zoom" => self.host.borrow_mut().view().borrow_mut().zoom_down(),
            "-zoom" => self.host.borrow_mut().view().borrow_mut().zoom_up(),
            "weapnext" => self.host.borrow_mut().weapons().borrow_mut().next_weapon(),
            "weapprev" => self.host.borrow_mut().weapons().borrow_mut().previous_weapon(),
            "weapon" => {
                let weapon = game_atoi(&console_argument(argv, 1, 1024));
                self.host.borrow_mut().weapons().borrow_mut().select_weapon(weapon);
            }
            "viewpos" => {
                let state = self.state.borrow();
                let text = game_format(
                    "(%i %i %i) : %i\n",
                    &[
                        GameFormatArg::Int(qvm_float_to_int(state.refdef.view_origin.x)),
                        GameFormatArg::Int(qvm_float_to_int(state.refdef.view_origin.y)),
                        GameFormatArg::Int(qvm_float_to_int(state.refdef.view_origin.z)),
                        GameFormatArg::Int(qvm_float_to_int(state.refdef_view_angles.y)),
                    ],
                    1024,
                );
                drop(state);
                self.host.borrow_mut().print(&text);
            }
            "sizeup" | "sizedown" => {
                let current = self.host.borrow().read_vm_cvar("cg_viewsize").integer_value;
                let next = current.wrapping_add(if name == "sizeup" { 10 } else { -10 });
                self.set("cg_viewsize", &game_format("%i", &[GameFormatArg::Int(next)], 1024))?;
            }
            "+scores" => self.scores_down(),
            "-scores" => {
                if self.state.borrow().show_scores {
                    self.state.borrow_mut().show_scores = false;
                    let time = self.state.borrow().time;
                    self.state.borrow_mut().score_fade_time = time;
                }
            }
            "tcmd" => {
                let target = self.crosshair_player();
                if target != 0 {
                    let text = game_format(
                        "gc %i %i",
                        &[
                            GameFormatArg::Int(target),
                            GameFormatArg::Int(game_atoi(&console_argument(argv, 1, 4))),
                        ],
                        1024,
                    );
                    self.host.borrow_mut().send_console_command(&text);
                }
            }
            "tell_target" | "tell_attacker" | "vtell_target" | "vtell_attacker" => {
                let target = if name.ends_with("target") {
                    self.crosshair_player()
                } else {
                    self.last_attacker()?
                };
                if target == -1 {
                    return Ok(());
                }
                let args = argv[1..].join(" ");
                if args.chars().count() >= 1024 {
                    return Err(HudError::new("Cmd_Args exceeds MAX_STRING_CHARS"));
                }
                let command = if name.starts_with('v') { "vtell" } else { "tell" };
                let text = game_format(
                    "%s %i %s",
                    &[
                        GameFormatArg::Text(command.to_string()),
                        GameFormatArg::Int(target),
                        GameFormatArg::Text(args.chars().take(127).collect()),
                    ],
                    128,
                );
                self.host.borrow_mut().send_client_command(&text);
            }
            "loaddeferred" => {
                let host = self.host.clone();
                self.host
                    .borrow_mut()
                    .clients()
                    .borrow_mut()
                    .load_deferred_players(&mut |entity| {
                        host.borrow_mut().reset_player_entity(entity);
                    });
            }
            "startorbit" => {
                if game_atoi(&self.immediate("developer")?) == 0 {
                    return Ok(());
                }
                if self.host.borrow().read_vm_cvar("cg_cameraOrbit").numeric_value != 0.0 {
                    self.set("cg_cameraOrbit", "0")?;
                    self.set("cg_thirdPerson", "0")?;
                } else {
                    self.set("cg_cameraOrbit", "5")?;
                    self.set("cg_thirdPerson", "1")?;
                    self.set("cg_thirdPersonAngle", "0")?;
                    self.set("cg_thirdPersonRange", "100")?;
                }
            }
            "loadhud" => {
                let hud = self.hud()?;
                hud.borrow_mut().reset_strings();
                hud.borrow_mut().reset_menus();
                let path = self.immediate("cg_hudFiles")?;
                hud.borrow_mut()
                    .load_menus(if path.is_empty() { "ui/hud.txt" } else { &path });
                self.open();
                hud.borrow_mut().clear_scoreboard();
            }
            "scoresdown" | "scoresup" => {
                let hud = self.hud()?;
                let menu = hud.borrow().menu_scoreboard();
                if let Some(menu) = menu {
                    if self.state.borrow().score_board_showing {
                        for feeder in [11, 5, 6] {
                            hud.borrow_mut().scroll_feeder(&menu, feeder, name == "scoresdown");
                        }
                    }
                }
            }
            "nextteammember" => self.orders()?.borrow_mut().select_next_player(),
            "prevteammember" => self.orders()?.borrow_mut().select_previous_player(),
            "nextorder" => self.next_order()?,
            "confirmorder" | "denyorder" => {
                let yes = name == "confirmorder";
                let cgs = self.static_state.borrow();
                let text = game_format(
                    "cmd vtell %d %s\n",
                    &[
                        GameFormatArg::Int(cgs.accept_leader),
                        GameFormatArg::Text(if yes { "yes".to_string() } else { "no".to_string() }),
                    ],
                    1024,
                );
                drop(cgs);
                self.host.borrow_mut().send_console_command(&text);
                self.host.borrow_mut().send_console_command(if yes {
                    "+button5; wait; -button5"
                } else {
                    "+button6; wait; -button6"
                });
                let (time, accept_time, accept_task) = {
                    let cgs = self.static_state.borrow();
                    (self.state.borrow().time, cgs.accept_order_time, cgs.accept_task)
                };
                if time < accept_time {
                    if yes {
                        self.host.borrow_mut().send_client_command(&game_format(
                            "teamtask %d\n",
                            &[GameFormatArg::Int(accept_task)],
                            1024,
                        ));
                    }
                    self.static_state.borrow_mut().accept_order_time = 0;
                }
            }
            "taskoffense" => self.task(
                if game_type == GameType::Ctf || game_type == GameType::OneFlagCtf {
                    "ongetflag"
                } else {
                    "onoffense"
                },
                1,
            ),
            "taskdefense" => self.task("ondefense", 2),
            "taskpatrol" => self.task("onpatrol", 3),
            "taskcamp" => self.task("oncamp", 7),
            "taskfollow" => self.task("onfollow", 4),
            "taskretrieve" => self.task("onreturnflag", 5),
            "taskescort" => self.task("onfollowcarrier", 6),
            "taskownflag" => self.host.borrow_mut().send_console_command("cmd vsay_team ihaveflag\n"),
            "tasksuicide" => {
                let target = self.crosshair_player();
                if target != -1 {
                    self.host.borrow_mut().send_client_command(&game_format(
                        "tell %i suicide",
                        &[GameFormatArg::Int(target)],
                        128,
                    ));
                }
            }
            "tauntkillinsult" => self.host.borrow_mut().send_console_command("cmd vsay kill_insult\n"),
            "tauntpraise" => self.host.borrow_mut().send_console_command("cmd vsay praise\n"),
            "taunttaunt" => self.host.borrow_mut().send_console_command("cmd vtaunt\n"),
            "tauntdeathinsult" => self.host.borrow_mut().send_console_command("cmd vsay death_insult\n"),
            "tauntgauntlet" => self.host.borrow_mut().send_console_command("cmd vsay kill_guantlet\n"),
            "spwin" | "splose" => {
                let win = name == "spwin";
                self.set("cg_cameraOrbit", "2")?;
                self.set("cg_cameraOrbitDelay", "35")?;
                self.set("cg_thirdPerson", "1")?;
                self.set("cg_thirdPersonAngle", "0")?;
                self.set("cg_thirdPersonRange", "100")?;
                let sound = self
                    .host
                    .borrow()
                    .sound(if win { MenuEndSound::Winner } else { MenuEndSound::Loser });
                self.host.borrow_mut().add_buffered_sound(sound);
                self.host
                    .borrow_mut()
                    .center_print(if win { "YOU WIN!" } else { "YOU LOSE..." }, 144, 0);
            }
            _ => {
                return Err(HudError::new(format!(
                    "Registered cgame console command has no handler: {name}"
                )))
            }
        }
        Ok(())
    }
}

/// Loading screen updater (`updateScreen`).
pub trait LoadingScreenUpdater {
    /// Update the screen.
    fn update_screen(&mut self);
}

/// Loading imports (`ClientLoadingImports`).
pub struct ClientLoadingImports {
    /// Configstrings.
    pub strings: Shared<dyn HudConfigStrings>,
    /// Screen updater.
    pub screen: Shared<dyn LoadingScreenUpdater>,
}

/// Loading source text (size cap).
fn loading_source_text(input: &str, size: usize) -> String {
    let cut = match input.find('\0') {
        Some(end) => &input[..end],
        None => input,
    };
    for ch in cut.chars() {
        if ch as u32 > 255 {
            panic!("Loading text requires source byte characters");
        }
    }
    cut.chars().take(size - 1).collect()
}

/// Clean loading text (`cleanText`).
fn clean_loading_text(text: &str) -> String {
    let units: Vec<char> = text.chars().collect();
    let mut result = String::new();
    let mut index = 0usize;
    while index < units.len() {
        let byte = units[index] as u32;
        if byte == 94 && index + 1 < units.len() && units[index + 1] != '^' {
            index += 1;
        } else if (32..=126).contains(&byte) {
            result.push(units[index]);
        }
        index += 1;
    }
    result
}

/// Loading screen (`ClientLoadingScreen`).
pub struct ClientLoadingScreen {
    /// Frame state.
    pub state: Shared<ClientGameState>,
    /// Media.
    pub media: Shared<ClientMedia>,
    /// Cvars.
    pub cvars: Shared<CvarRegistry>,
    /// Imports.
    pub imports: ClientLoadingImports,
    /// Player icons.
    player_icons: RefCell<Vec<SceneShader>>,
    /// Item icons.
    item_icons: RefCell<Vec<Option<SceneShader>>>,
}

impl ClientLoadingScreen {
    /// Assemble a loading screen.
    pub fn new(
        state: Shared<ClientGameState>,
        media: Shared<ClientMedia>,
        cvars: Shared<CvarRegistry>,
        imports: ClientLoadingImports,
    ) -> Self {
        if state.borrow().product != media.borrow().product {
            panic!("Loading screen product differs from cgame media");
        }
        Self {
            state,
            media,
            cvars,
            imports,
            player_icons: RefCell::new(Vec::new()),
            item_icons: RefCell::new(Vec::new()),
        }
    }

    /// Set the loading string (`loadingString`).
    pub fn loading_string(&self, text: &str) {
        self.state.borrow_mut().info_screen_text = loading_source_text(text, 1024);
        self.imports.screen.borrow_mut().update_screen();
    }

    /// Load an item (`loadingItem`).
    pub fn loading_item(&self, index: i32) {
        let product = self.media.borrow().product;
        let item = self.media.borrow().items.borrow().at(product, index);
        let Some(pickup) = item.pickup_name.clone() else {
            panic!("CG_LoadingItem requires a named item");
        };
        if item.icon.is_some() && self.item_icons.borrow().len() < 26 {
            let icon = self
                .media
                .borrow()
                .resources
                .borrow_mut()
                .register_shader_no_mip(item.icon.as_deref());
            self.item_icons.borrow_mut().push(icon);
        }
        self.loading_string(&pickup);
    }

    /// Load a client (`loadingClient`).
    pub fn loading_client(&self, client_num: i32) {
        if client_num < 0 || client_num >= 64 {
            panic!("CG_LoadingClient: bad client number");
        }
        let info = self
            .imports
            .strings
            .borrow()
            .config_string(CS_PLAYERS + client_num as usize);
        if self.player_icons.borrow().len() < 16 {
            let value = loading_source_text(&info_value_for_key(&info, "model", 8192), 64);
            let slash = value.rfind('/');
            let (model, skin) = match slash {
                None => (value.as_str(), "default"),
                Some(position) => (&value[..position], &value[position + 1..]),
            };
            let mut icon =
                self.media
                    .borrow()
                    .resources
                    .borrow_mut()
                    .register_shader_no_mip(Some(&loading_source_text(
                        &format!("models/players/{model}/icon_{skin}.tga"),
                        64,
                    )));
            if icon.is_none() {
                icon = self
                    .media
                    .borrow()
                    .resources
                    .borrow_mut()
                    .register_shader_no_mip(Some(&loading_source_text(
                        &format!("models/players/characters/{model}/icon_{skin}.tga"),
                        64,
                    )));
            }
            if icon.is_none() {
                icon = self
                    .media
                    .borrow()
                    .resources
                    .borrow_mut()
                    .register_shader_no_mip(Some("models/players/sarge/icon_default.tga"));
            }
            if let Some(icon) = icon {
                self.player_icons.borrow_mut().push(icon);
            }
        }
        let personality = clean_loading_text(&loading_source_text(&info_value_for_key(&info, "n", 8192), 64));
        if self.media.borrow().static_state.borrow().game_type == GameType::SinglePlayer {
            self.media
                .borrow()
                .sound_bank
                .borrow_mut()
                .register_sound(Some(&format!("sound/player/announce/{personality}.wav")), true);
        }
        self.loading_string(&personality);
    }

    /// Draw loading icons (`drawLoadingIcons`).
    fn draw_loading_icons(&self, tools: &ClientDrawTools) {
        for (n, icon) in self.player_icons.borrow().iter().enumerate() {
            tools.draw_pic(rect2d(16.0 + n as f32 * 78.0, 284.0, 64.0, 64.0), &Some(icon.clone()));
        }
        for (n, icon) in self.item_icons.borrow().iter().enumerate() {
            tools.draw_pic(
                rect2d(
                    16.0 + (n % 13) as f32 * 48.0,
                    if n >= 13 { 400.0 } else { 360.0 },
                    32.0,
                    32.0,
                ),
                icon,
            );
        }
    }

    /// Game type name (`gameTypeName`).
    fn game_type_name(&self) -> String {
        let media = self.media.borrow();
        let game_type = media.static_state.borrow().game_type;
        match game_type {
            GameType::Ffa => "Free For All".to_string(),
            GameType::SinglePlayer => "Single Player".to_string(),
            GameType::Tournament => "Tournament".to_string(),
            GameType::Team => "Team Deathmatch".to_string(),
            GameType::Ctf => "Capture The Flag".to_string(),
            GameType::OneFlagCtf => {
                if media.product == Product::Missionpack {
                    "One Flag CTF".to_string()
                } else {
                    "Unknown Gametype".to_string()
                }
            }
            GameType::Obelisk => {
                if media.product == Product::Missionpack {
                    "Overload".to_string()
                } else {
                    "Unknown Gametype".to_string()
                }
            }
            GameType::Harvester => {
                if media.product == Product::Missionpack {
                    "Harvester".to_string()
                } else {
                    "Unknown Gametype".to_string()
                }
            }
            _ => "Unknown Gametype".to_string(),
        }
    }

    /// Draw the information screen (`drawInformation`).
    pub fn draw_information(&self, draw: &mut Draw2D) {
        let info = self.imports.strings.borrow().config_string(0);
        let system = self.imports.strings.borrow().config_string(1);
        let map = info_value_for_key(&info, "mapname", 8192);
        let mut levelshot = self
            .media
            .borrow()
            .resources
            .borrow_mut()
            .register_shader_no_mip(Some(&format!("levelshots/{map}.tga")));
        if levelshot.is_none() {
            levelshot = self
                .media
                .borrow()
                .resources
                .borrow_mut()
                .register_shader_no_mip(Some("menu/art/unknownmap"));
        }
        let tools = ClientDrawTools::new(draw.clone(), self.media.clone());
        tools.draw.set_color(None);
        tools.draw_pic(rect2d(0.0, 0.0, 640.0, 480.0), &levelshot);
        let detail = self
            .media
            .borrow()
            .resources
            .borrow_mut()
            .register_shader("levelShotDetail");
        let picture = self.media.borrow().resources.borrow().picture(&detail);
        tools.draw.stretch_pixels(
            rect2d(0.0, 0.0, tools.draw.width() as f32, tools.draw.height() as f32),
            TextureRect {
                s: 0.0,
                t: 0.0,
                s2: 2.5,
                t2: 2.0,
            },
            picture,
        );
        self.draw_loading_icons(&tools);
        let time = self.state.borrow().time;
        let text = |tools: &ClientDrawTools, y: i32, value: &str| {
            tools.draw_proportional_string(&UiTextOptions {
                x: 320.0,
                y: y as f32,
                text: value.to_string(),
                style: UI_CENTER | UI_SMALLFONT | UI_DROPSHADOW,
                color: vec4(1.0, 1.0, 1.0, 1.0),
                time,
            });
        };
        let info_text = self.state.borrow().info_screen_text.clone();
        let loading_text = if info_text.is_empty() {
            "Awaiting snapshot...".to_string()
        } else {
            format!("Loading... {info_text}")
        };
        text(&tools, 96, &loading_text);
        let mut y = 148;
        // Cvar_VariableStringBuffer returns the empty string for an unregistered cvar.
        let server = self.cvars.borrow().get("sv_running");
        if game_atoi(&loading_source_text(
            server.map(|snapshot| snapshot.value).unwrap_or_default().as_str(),
            1024,
        )) == 0
        {
            text(
                &tools,
                y,
                &clean_loading_text(&loading_source_text(
                    &info_value_for_key(&info, "sv_hostname", 8192),
                    1024,
                )),
            );
            y += 27;
            if info_value_for_key(&system, "sv_pure", 8192).starts_with('1') {
                text(&tools, y, "Pure Server");
                y += 27;
            }
            let motd = loading_source_text(&self.imports.strings.borrow().config_string(4), 16000);
            if !motd.is_empty() {
                text(&tools, y, &motd);
                y += 27;
            }
            y += 10;
        }
        let message = loading_source_text(&self.imports.strings.borrow().config_string(3), 16000);
        if !message.is_empty() {
            text(&tools, y, &message);
            y += 27;
        }
        if info_value_for_key(&system, "sv_cheats", 8192).starts_with('1') {
            text(&tools, y, "CHEATS ARE ENABLED");
            y += 27;
        }
        text(&tools, y, &self.game_type_name());
        y += 27;
        let time_limit = game_atoi(&info_value_for_key(&info, "timelimit", 8192));
        if time_limit != 0 {
            text(
                &tools,
                y,
                &game_format("timelimit %i", &[GameFormatArg::Int(time_limit)], 1024),
            );
            y += 27;
        }
        if self.media.borrow().static_state.borrow().game_type < GameType::Ctf {
            let frag_limit = game_atoi(&info_value_for_key(&info, "fraglimit", 8192));
            if frag_limit != 0 {
                text(
                    &tools,
                    y,
                    &game_format("fraglimit %i", &[GameFormatArg::Int(frag_limit)], 1024),
                );
            }
        }
        if self.media.borrow().static_state.borrow().game_type >= GameType::Ctf {
            let capture_limit = game_atoi(&info_value_for_key(&info, "capturelimit", 8192));
            if capture_limit != 0 {
                text(
                    &tools,
                    y,
                    &game_format("capturelimit %i", &[GameFormatArg::Int(capture_limit)], 1024),
                );
            }
        }
    }
}

/// Maximum clients for config reload.
const CONFIG_MAX_CLIENTS: i32 = 64;
/// Maximum cvar value string.
const MAX_CVAR_VALUE_STRING: usize = 256;
/// Maximum token characters.
const MAX_TOKEN_CHARS: usize = 1024;

/// VM cvar symbol (`ClientVmCvarSymbol`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClientVmCvarSymbol {
    /// cg_ignore.
    CgIgnore,
    /// cg_autoswitch.
    CgAutoswitch,
    /// cg_drawGun.
    CgDrawGun,
    /// cg_zoomFov.
    CgZoomFov,
    /// cg_fov.
    CgFov,
    /// cg_viewsize.
    CgViewsize,
    /// cg_stereoSeparation.
    CgStereoSeparation,
    /// cg_shadows.
    CgShadows,
    /// cg_gibs.
    CgGibs,
    /// cg_draw2D.
    CgDraw2d,
    /// cg_drawStatus.
    CgDrawStatus,
    /// cg_drawTimer.
    CgDrawTimer,
    /// cg_drawFPS.
    CgDrawFps,
    /// cg_drawSnapshot.
    CgDrawSnapshot,
    /// cg_draw3dIcons.
    CgDraw3dIcons,
    /// cg_drawIcons.
    CgDrawIcons,
    /// cg_drawAmmoWarning.
    CgDrawAmmoWarning,
    /// cg_drawAttacker.
    CgDrawAttacker,
    /// cg_drawCrosshair.
    CgDrawCrosshair,
    /// cg_drawCrosshairNames.
    CgDrawCrosshairNames,
    /// cg_drawRewards.
    CgDrawRewards,
    /// cg_crosshairSize.
    CgCrosshairSize,
    /// cg_crosshairHealth.
    CgCrosshairHealth,
    /// cg_crosshairX.
    CgCrosshairX,
    /// cg_crosshairY.
    CgCrosshairY,
    /// cg_brassTime.
    CgBrassTime,
    /// cg_simpleItems.
    CgSimpleItems,
    /// cg_addMarks.
    CgAddMarks,
    /// cg_lagometer.
    CgLagometer,
    /// cg_railTrailTime.
    CgRailTrailTime,
    /// cg_gun_x.
    CgGunX,
    /// cg_gun_y.
    CgGunY,
    /// cg_gun_z.
    CgGunZ,
    /// cg_centertime.
    CgCentertime,
    /// cg_runpitch.
    CgRunpitch,
    /// cg_runroll.
    CgRunroll,
    /// cg_bobup.
    CgBobup,
    /// cg_bobpitch.
    CgBobpitch,
    /// cg_bobroll.
    CgBobroll,
    /// cg_swingSpeed.
    CgSwingSpeed,
    /// cg_animSpeed.
    CgAnimSpeed,
    /// cg_debugAnim.
    CgDebugAnim,
    /// cg_debugPosition.
    CgDebugPosition,
    /// cg_debugEvents.
    CgDebugEvents,
    /// cg_errorDecay.
    CgErrorDecay,
    /// cg_nopredict.
    CgNopredict,
    /// cg_noPlayerAnims.
    CgNoPlayerAnims,
    /// cg_showmiss.
    CgShowmiss,
    /// cg_footsteps.
    CgFootsteps,
    /// cg_tracerChance.
    CgTracerChance,
    /// cg_tracerWidth.
    CgTracerWidth,
    /// cg_tracerLength.
    CgTracerLength,
    /// cg_thirdPersonRange.
    CgThirdPersonRange,
    /// cg_thirdPersonAngle.
    CgThirdPersonAngle,
    /// cg_thirdPerson.
    CgThirdPerson,
    /// cg_teamChatTime.
    CgTeamChatTime,
    /// cg_teamChatHeight.
    CgTeamChatHeight,
    /// cg_forceModel.
    CgForceModel,
    /// cg_predictItems.
    CgPredictItems,
    /// cg_deferPlayers.
    CgDeferPlayers,
    /// cg_drawTeamOverlay.
    CgDrawTeamOverlay,
    /// cg_teamOverlayUserinfo.
    CgTeamOverlayUserinfo,
    /// cg_stats.
    CgStats,
    /// cg_drawFriend.
    CgDrawFriend,
    /// cg_teamChatsOnly.
    CgTeamChatsOnly,
    /// cg_noVoiceChats.
    CgNoVoiceChats,
    /// cg_noVoiceText.
    CgNoVoiceText,
    /// cg_buildScript.
    CgBuildScript,
    /// cg_paused.
    CgPaused,
    /// cg_blood.
    CgBlood,
    /// cg_synchronousClients.
    CgSynchronousClients,
    /// cg_redTeamName.
    CgRedTeamName,
    /// cg_blueTeamName.
    CgBlueTeamName,
    /// cg_currentSelectedPlayer.
    CgCurrentSelectedPlayer,
    /// cg_currentSelectedPlayerName.
    CgCurrentSelectedPlayerName,
    /// cg_singlePlayer.
    CgSinglePlayer,
    /// cg_enableDust.
    CgEnableDust,
    /// cg_enableBreath.
    CgEnableBreath,
    /// cg_singlePlayerActive.
    CgSinglePlayerActive,
    /// cg_recordSPDemo.
    CgRecordSpDemo,
    /// cg_recordSPDemoName.
    CgRecordSpDemoName,
    /// cg_obeliskRespawnDelay.
    CgObeliskRespawnDelay,
    /// cg_hudFiles.
    CgHudFiles,
    /// cg_cameraOrbit.
    CgCameraOrbit,
    /// cg_cameraOrbitDelay.
    CgCameraOrbitDelay,
    /// cg_timescaleFadeEnd.
    CgTimescaleFadeEnd,
    /// cg_timescaleFadeSpeed.
    CgTimescaleFadeSpeed,
    /// cg_timescale.
    CgTimescale,
    /// cg_scorePlum.
    CgScorePlum,
    /// cg_smoothClients.
    CgSmoothClients,
    /// cg_cameraMode.
    CgCameraMode,
    /// pmove_fixed.
    PmoveFixed,
    /// pmove_msec.
    PmoveMsec,
    /// cg_noTaunt.
    CgNoTaunt,
    /// cg_noProjectileTrail.
    CgNoProjectileTrail,
    /// cg_smallFont.
    CgSmallFont,
    /// cg_bigFont.
    CgBigFont,
    /// cg_oldRail.
    CgOldRail,
    /// cg_oldRocket.
    CgOldRocket,
    /// cg_oldPlasma.
    CgOldPlasma,
    /// cg_trueLightning.
    CgTrueLightning,
}

impl ClientVmCvarSymbol {
    /// Donor spelling.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CgIgnore => "cg_ignore",
            Self::CgAutoswitch => "cg_autoswitch",
            Self::CgDrawGun => "cg_drawGun",
            Self::CgZoomFov => "cg_zoomFov",
            Self::CgFov => "cg_fov",
            Self::CgViewsize => "cg_viewsize",
            Self::CgStereoSeparation => "cg_stereoSeparation",
            Self::CgShadows => "cg_shadows",
            Self::CgGibs => "cg_gibs",
            Self::CgDraw2d => "cg_draw2D",
            Self::CgDrawStatus => "cg_drawStatus",
            Self::CgDrawTimer => "cg_drawTimer",
            Self::CgDrawFps => "cg_drawFPS",
            Self::CgDrawSnapshot => "cg_drawSnapshot",
            Self::CgDraw3dIcons => "cg_draw3dIcons",
            Self::CgDrawIcons => "cg_drawIcons",
            Self::CgDrawAmmoWarning => "cg_drawAmmoWarning",
            Self::CgDrawAttacker => "cg_drawAttacker",
            Self::CgDrawCrosshair => "cg_drawCrosshair",
            Self::CgDrawCrosshairNames => "cg_drawCrosshairNames",
            Self::CgDrawRewards => "cg_drawRewards",
            Self::CgCrosshairSize => "cg_crosshairSize",
            Self::CgCrosshairHealth => "cg_crosshairHealth",
            Self::CgCrosshairX => "cg_crosshairX",
            Self::CgCrosshairY => "cg_crosshairY",
            Self::CgBrassTime => "cg_brassTime",
            Self::CgSimpleItems => "cg_simpleItems",
            Self::CgAddMarks => "cg_addMarks",
            Self::CgLagometer => "cg_lagometer",
            Self::CgRailTrailTime => "cg_railTrailTime",
            Self::CgGunX => "cg_gun_x",
            Self::CgGunY => "cg_gun_y",
            Self::CgGunZ => "cg_gun_z",
            Self::CgCentertime => "cg_centertime",
            Self::CgRunpitch => "cg_runpitch",
            Self::CgRunroll => "cg_runroll",
            Self::CgBobup => "cg_bobup",
            Self::CgBobpitch => "cg_bobpitch",
            Self::CgBobroll => "cg_bobroll",
            Self::CgSwingSpeed => "cg_swingSpeed",
            Self::CgAnimSpeed => "cg_animSpeed",
            Self::CgDebugAnim => "cg_debugAnim",
            Self::CgDebugPosition => "cg_debugPosition",
            Self::CgDebugEvents => "cg_debugEvents",
            Self::CgErrorDecay => "cg_errorDecay",
            Self::CgNopredict => "cg_nopredict",
            Self::CgNoPlayerAnims => "cg_noPlayerAnims",
            Self::CgShowmiss => "cg_showmiss",
            Self::CgFootsteps => "cg_footsteps",
            Self::CgTracerChance => "cg_tracerChance",
            Self::CgTracerWidth => "cg_tracerWidth",
            Self::CgTracerLength => "cg_tracerLength",
            Self::CgThirdPersonRange => "cg_thirdPersonRange",
            Self::CgThirdPersonAngle => "cg_thirdPersonAngle",
            Self::CgThirdPerson => "cg_thirdPerson",
            Self::CgTeamChatTime => "cg_teamChatTime",
            Self::CgTeamChatHeight => "cg_teamChatHeight",
            Self::CgForceModel => "cg_forceModel",
            Self::CgPredictItems => "cg_predictItems",
            Self::CgDeferPlayers => "cg_deferPlayers",
            Self::CgDrawTeamOverlay => "cg_drawTeamOverlay",
            Self::CgTeamOverlayUserinfo => "cg_teamOverlayUserinfo",
            Self::CgStats => "cg_stats",
            Self::CgDrawFriend => "cg_drawFriend",
            Self::CgTeamChatsOnly => "cg_teamChatsOnly",
            Self::CgNoVoiceChats => "cg_noVoiceChats",
            Self::CgNoVoiceText => "cg_noVoiceText",
            Self::CgBuildScript => "cg_buildScript",
            Self::CgPaused => "cg_paused",
            Self::CgBlood => "cg_blood",
            Self::CgSynchronousClients => "cg_synchronousClients",
            Self::CgRedTeamName => "cg_redTeamName",
            Self::CgBlueTeamName => "cg_blueTeamName",
            Self::CgCurrentSelectedPlayer => "cg_currentSelectedPlayer",
            Self::CgCurrentSelectedPlayerName => "cg_currentSelectedPlayerName",
            Self::CgSinglePlayer => "cg_singlePlayer",
            Self::CgEnableDust => "cg_enableDust",
            Self::CgEnableBreath => "cg_enableBreath",
            Self::CgSinglePlayerActive => "cg_singlePlayerActive",
            Self::CgRecordSpDemo => "cg_recordSPDemo",
            Self::CgRecordSpDemoName => "cg_recordSPDemoName",
            Self::CgObeliskRespawnDelay => "cg_obeliskRespawnDelay",
            Self::CgHudFiles => "cg_hudFiles",
            Self::CgCameraOrbit => "cg_cameraOrbit",
            Self::CgCameraOrbitDelay => "cg_cameraOrbitDelay",
            Self::CgTimescaleFadeEnd => "cg_timescaleFadeEnd",
            Self::CgTimescaleFadeSpeed => "cg_timescaleFadeSpeed",
            Self::CgTimescale => "cg_timescale",
            Self::CgScorePlum => "cg_scorePlum",
            Self::CgSmoothClients => "cg_smoothClients",
            Self::CgCameraMode => "cg_cameraMode",
            Self::PmoveFixed => "pmove_fixed",
            Self::PmoveMsec => "pmove_msec",
            Self::CgNoTaunt => "cg_noTaunt",
            Self::CgNoProjectileTrail => "cg_noProjectileTrail",
            Self::CgSmallFont => "cg_smallFont",
            Self::CgBigFont => "cg_bigFont",
            Self::CgOldRail => "cg_oldRail",
            Self::CgOldRocket => "cg_oldRocket",
            Self::CgOldPlasma => "cg_oldPlasma",
            Self::CgTrueLightning => "cg_trueLightning",
        }
    }
}

/// Cvar definition (`CvarDefinition`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CvarDefinition {
    /// Symbol.
    pub symbol: ClientVmCvarSymbol,
    /// Engine name.
    pub name: String,
    /// Default value.
    pub default_value: String,
    /// Flags.
    pub flags: u32,
}

/// Build a definition (`cv`).
fn cv(symbol: ClientVmCvarSymbol, name: &str, default_value: &str, flags: u32) -> CvarDefinition {
    CvarDefinition {
        symbol,
        name: name.to_string(),
        default_value: default_value.to_string(),
        flags,
    }
}

/// Cvar table (`cvarTable`).
#[must_use]
pub fn cvar_table(product: Product) -> Vec<CvarDefinition> {
    use ClientVmCvarSymbol as S;
    let a = cvar_flags::ARCHIVE;
    let c = cvar_flags::CHEAT;
    let r = cvar_flags::READ_ONLY;
    let u = cvar_flags::USER_INFO;
    let s = cvar_flags::SERVER_INFO;
    let n = cvar_flags::NONE;
    let mut table = vec![
        cv(S::CgIgnore, "cg_ignore", "0", n),
        cv(S::CgAutoswitch, "cg_autoswitch", "1", a),
        cv(S::CgDrawGun, "cg_drawGun", "1", a),
        cv(S::CgZoomFov, "cg_zoomfov", "22.5", a),
        cv(S::CgFov, "cg_fov", "90", a),
        cv(S::CgViewsize, "cg_viewsize", "100", a),
        cv(S::CgStereoSeparation, "cg_stereoSeparation", "0.4", a),
        cv(S::CgShadows, "cg_shadows", "1", a),
        cv(S::CgGibs, "cg_gibs", "1", a),
        cv(S::CgDraw2d, "cg_draw2D", "1", a),
        cv(S::CgDrawStatus, "cg_drawStatus", "1", a),
        cv(S::CgDrawTimer, "cg_drawTimer", "0", a),
        cv(S::CgDrawFps, "cg_drawFPS", "0", a),
        cv(S::CgDrawSnapshot, "cg_drawSnapshot", "0", a),
        cv(S::CgDraw3dIcons, "cg_draw3dIcons", "1", a),
        cv(S::CgDrawIcons, "cg_drawIcons", "1", a),
        cv(S::CgDrawAmmoWarning, "cg_drawAmmoWarning", "1", a),
        cv(S::CgDrawAttacker, "cg_drawAttacker", "1", a),
        cv(S::CgDrawCrosshair, "cg_drawCrosshair", "4", a),
        cv(S::CgDrawCrosshairNames, "cg_drawCrosshairNames", "1", a),
        cv(S::CgDrawRewards, "cg_drawRewards", "1", a),
        cv(S::CgCrosshairSize, "cg_crosshairSize", "24", a),
        cv(S::CgCrosshairHealth, "cg_crosshairHealth", "1", a),
        cv(S::CgCrosshairX, "cg_crosshairX", "0", a),
        cv(S::CgCrosshairY, "cg_crosshairY", "0", a),
        cv(S::CgBrassTime, "cg_brassTime", "2500", a),
        cv(S::CgSimpleItems, "cg_simpleItems", "0", a),
        cv(S::CgAddMarks, "cg_marks", "1", a),
        cv(S::CgLagometer, "cg_lagometer", "1", a),
        cv(S::CgRailTrailTime, "cg_railTrailTime", "400", a),
        cv(S::CgGunX, "cg_gunX", "0", c),
        cv(S::CgGunY, "cg_gunY", "0", c),
        cv(S::CgGunZ, "cg_gunZ", "0", c),
        cv(S::CgCentertime, "cg_centertime", "3", c),
        cv(S::CgRunpitch, "cg_runpitch", "0.002", a),
        cv(S::CgRunroll, "cg_runroll", "0.005", a),
        cv(S::CgBobup, "cg_bobup", "0.005", c),
        cv(S::CgBobpitch, "cg_bobpitch", "0.002", a),
        cv(S::CgBobroll, "cg_bobroll", "0.002", a),
        cv(S::CgSwingSpeed, "cg_swingSpeed", "0.3", c),
        cv(S::CgAnimSpeed, "cg_animspeed", "1", c),
        cv(S::CgDebugAnim, "cg_debuganim", "0", c),
        cv(S::CgDebugPosition, "cg_debugposition", "0", c),
        cv(S::CgDebugEvents, "cg_debugevents", "0", c),
        cv(S::CgErrorDecay, "cg_errordecay", "100", n),
        cv(S::CgNopredict, "cg_nopredict", "0", n),
        cv(S::CgNoPlayerAnims, "cg_noplayeranims", "0", c),
        cv(S::CgShowmiss, "cg_showmiss", "0", n),
        cv(S::CgFootsteps, "cg_footsteps", "1", c),
        cv(S::CgTracerChance, "cg_tracerchance", "0.4", c),
        cv(S::CgTracerWidth, "cg_tracerwidth", "1", c),
        cv(S::CgTracerLength, "cg_tracerlength", "100", c),
        cv(S::CgThirdPersonRange, "cg_thirdPersonRange", "40", c),
        cv(S::CgThirdPersonAngle, "cg_thirdPersonAngle", "0", c),
        cv(S::CgThirdPerson, "cg_thirdPerson", "0", n),
        cv(S::CgTeamChatTime, "cg_teamChatTime", "3000", a),
        cv(S::CgTeamChatHeight, "cg_teamChatHeight", "0", a),
        cv(S::CgForceModel, "cg_forceModel", "0", a),
        cv(S::CgPredictItems, "cg_predictItems", "1", a),
        cv(
            S::CgDeferPlayers,
            "cg_deferPlayers",
            if product == Product::Missionpack { "0" } else { "1" },
            a,
        ),
        cv(S::CgDrawTeamOverlay, "cg_drawTeamOverlay", "0", a),
        cv(S::CgTeamOverlayUserinfo, "teamoverlay", "0", r | u),
        cv(S::CgStats, "cg_stats", "0", n),
        cv(S::CgDrawFriend, "cg_drawFriend", "1", a),
        cv(S::CgTeamChatsOnly, "cg_teamChatsOnly", "0", a),
        cv(S::CgNoVoiceChats, "cg_noVoiceChats", "0", a),
        cv(S::CgNoVoiceText, "cg_noVoiceText", "0", a),
        cv(S::CgBuildScript, "com_buildScript", "0", n),
        cv(S::CgPaused, "cl_paused", "0", r),
        cv(S::CgBlood, "com_blood", "1", a),
        cv(S::CgSynchronousClients, "g_synchronousClients", "0", n),
    ];
    if product == Product::Missionpack {
        table.extend([
            cv(S::CgRedTeamName, "g_redteam", "Stroggs", a | s | u),
            cv(S::CgBlueTeamName, "g_blueteam", "Pagans", a | s | u),
            cv(S::CgCurrentSelectedPlayer, "cg_currentSelectedPlayer", "0", a),
            cv(S::CgCurrentSelectedPlayerName, "cg_currentSelectedPlayerName", "", a),
            cv(S::CgSinglePlayer, "ui_singlePlayerActive", "0", u),
            cv(S::CgEnableDust, "g_enableDust", "0", s),
            cv(S::CgEnableBreath, "g_enableBreath", "0", s),
            cv(S::CgSinglePlayerActive, "ui_singlePlayerActive", "0", u),
            cv(S::CgRecordSpDemo, "ui_recordSPDemo", "0", a),
            cv(S::CgRecordSpDemoName, "ui_recordSPDemoName", "", a),
            cv(S::CgObeliskRespawnDelay, "g_obeliskRespawnDelay", "10", s),
            cv(S::CgHudFiles, "cg_hudFiles", "ui/hud.txt", a),
        ]);
    }
    table.extend([
        cv(S::CgCameraOrbit, "cg_cameraOrbit", "0", c),
        cv(S::CgCameraOrbitDelay, "cg_cameraOrbitDelay", "50", a),
        cv(S::CgTimescaleFadeEnd, "cg_timescaleFadeEnd", "1", n),
        cv(S::CgTimescaleFadeSpeed, "cg_timescaleFadeSpeed", "0", n),
        cv(S::CgTimescale, "timescale", "1", n),
        cv(S::CgScorePlum, "cg_scorePlums", "1", u | a),
        cv(S::CgSmoothClients, "cg_smoothClients", "0", u | a),
        cv(S::CgCameraMode, "com_cameraMode", "0", c),
        cv(S::PmoveFixed, "pmove_fixed", "0", n),
        cv(S::PmoveMsec, "pmove_msec", "8", n),
        cv(S::CgNoTaunt, "cg_noTaunt", "0", a),
        cv(S::CgNoProjectileTrail, "cg_noProjectileTrail", "0", a),
        cv(S::CgSmallFont, "ui_smallFont", "0.25", a),
        cv(S::CgBigFont, "ui_bigFont", "0.4", a),
        cv(S::CgOldRail, "cg_oldRail", "1", a),
        cv(S::CgOldRocket, "cg_oldRocket", "1", a),
        cv(S::CgOldPlasma, "cg_oldPlasma", "1", a),
        cv(S::CgTrueLightning, "cg_trueLightning", "0.0", a),
    ]);
    table
}

/// Configuration host services (`ClientConfigurationHost`).
pub struct ClientConfigurationHost {
    /// Cvar registry.
    pub cvars: Shared<CvarRegistry>,
    /// Frame state.
    pub state: Shared<ClientGameState>,
    /// Static state.
    pub static_state: Shared<ClientGameStaticState>,
    /// Client store.
    pub clients: Shared<dyn ClientInfoStore>,
    /// Configstrings.
    pub strings: Shared<dyn HudConfigStrings>,
    /// Status visibility override.
    pub status_visible: Option<Rc<dyn Fn() -> bool>>,
}

/// Source string with capacity (`sourceString`).
fn config_source_string(value: &str, capacity: usize) -> String {
    if capacity < 1 {
        panic!("Source string capacity must be positive");
    }
    let units: Vec<char> = value.chars().collect();
    let mut end = units.len();
    for (index, unit) in units.iter().enumerate() {
        if *unit == '\0' {
            end = index;
            break;
        }
        if *unit as u32 > 255 {
            panic!("Cvar VM strings require source byte characters");
        }
    }
    units[..end.min(capacity - 1)].iter().collect()
}

/// Changed snapshot (`changedSnapshot`).
fn changed_snapshot(engine: &CvarSnapshot) -> CvarSnapshot {
    let value = config_source_string(&engine.value, MAX_CVAR_VALUE_STRING);
    if value.chars().count() != engine.value.chars().count() {
        panic!(
            "Cvar_Update: src {} length {} exceeds MAX_CVAR_VALUE_STRING",
            engine.value,
            engine.value.chars().count()
        );
    }
    CvarSnapshot {
        value,
        ..engine.clone()
    }
}

/// Instance-owned cvar table cache (`ClientConfiguration`).
pub struct ClientConfiguration {
    /// Product.
    pub product: Product,
    /// Host.
    pub host: ClientConfigurationHost,
    /// Table.
    table: Vec<CvarDefinition>,
    /// Snapshot cache.
    cache: RefCell<HashMap<ClientVmCvarSymbol, CvarSnapshot>>,
    /// Name lookup.
    names: RefCell<HashMap<String, ClientVmCvarSymbol>>,
    /// Force-model modification count.
    force_model_modification_count: Cell<u32>,
    /// Overlay modification count.
    draw_team_overlay_modification_count: Cell<i64>,
    /// Registered.
    registered: Cell<bool>,
    /// Updating.
    updating: Cell<bool>,
}

impl ClientConfiguration {
    /// Assemble configuration.
    pub fn new(product: Product, host: ClientConfigurationHost) -> Self {
        if host.state.borrow().product != product || host.static_state.borrow().product != product {
            panic!("Client configuration services must share one product");
        }
        let table = cvar_table(product);
        let mut names = HashMap::new();
        let mut cache = HashMap::new();
        for definition in &table {
            names.insert(definition.name.to_lowercase(), definition.symbol);
            cache.insert(
                definition.symbol,
                CvarSnapshot {
                    name: definition.name.clone(),
                    value: String::new(),
                    reset_value: String::new(),
                    latched_value: None,
                    flags: 0,
                    modified: false,
                    modification_count: 0,
                    numeric_value: 0.0,
                    integer_value: 0,
                },
            );
        }
        Self {
            product,
            host,
            table,
            cache: RefCell::new(cache),
            names: RefCell::new(names),
            force_model_modification_count: Cell::new(0),
            draw_team_overlay_modification_count: Cell::new(-1),
            registered: Cell::new(false),
            updating: Cell::new(false),
        }
    }

    /// Register cvars (`registerCvars`).
    pub fn register_cvars(&self) {
        if self.updating.get() {
            panic!("Cannot register cvars during a configuration update");
        }
        self.names.borrow_mut().clear();
        for definition in &self.table {
            let engine =
                self.host
                    .cvars
                    .borrow_mut()
                    .register(&definition.name, &definition.default_value, definition.flags);
            let engine = match engine {
                Ok(Some(snapshot)) => snapshot,
                _ => panic!("Cgame cvar registration failed: {}", definition.name),
            };
            self.copy_definition(definition, &engine, true);
            self.names
                .borrow_mut()
                .insert(definition.name.to_lowercase(), definition.symbol);
        }
        let running = self.host.cvars.borrow().get("sv_running");
        self.host.static_state.borrow_mut().local_server = game_atoi(&config_source_string(
            running.map(|snapshot| snapshot.value).unwrap_or_default().as_str(),
            MAX_TOKEN_CHARS,
        ));
        self.force_model_modification_count
            .set(self.read_vm_symbol(ClientVmCvarSymbol::CgForceModel).modification_count);
        let (team_model, team_head) = if self.product == Product::Missionpack {
            ("james", "*james")
        } else {
            ("sarge", "sarge")
        };
        let flags = cvar_flags::USER_INFO | cvar_flags::ARCHIVE;
        for (name, default) in [
            ("model", "sarge"),
            ("headmodel", "sarge"),
            ("team_model", team_model),
            ("team_headmodel", team_head),
        ] {
            let _ = self
                .host
                .cvars
                .borrow_mut()
                .register(name, default, flags)
                .unwrap_or(None);
        }
        self.registered.set(true);
    }

    /// Read a VM symbol (`readVmSymbol`).
    #[must_use]
    pub fn read_vm_symbol(&self, symbol: ClientVmCvarSymbol) -> CvarSnapshot {
        let value = self.cache.borrow().get(&symbol).cloned().unwrap_or_else(|| {
            panic!(
                "VM cvar {} is not registered for {}",
                symbol.as_str(),
                self.product.as_str()
            );
        });
        if symbol == ClientVmCvarSymbol::CgDrawStatus
            && self.host.status_visible.as_ref().is_some_and(|visible| !visible())
        {
            return CvarSnapshot {
                value: "0".to_string(),
                numeric_value: 0.0,
                integer_value: 0,
                ..value
            };
        }
        value
    }

    /// Write a VM numeric value (`setVmNumericValue`).
    pub fn set_vm_numeric_value(&self, symbol: ClientVmCvarSymbol, value: f64) {
        let previous = self.read_vm_symbol(symbol);
        self.cache.borrow_mut().insert(
            symbol,
            CvarSnapshot {
                numeric_value: value as f32,
                ..previous
            },
        );
    }

    /// Write a VM integer (`setVmInteger`).
    pub fn set_vm_integer(&self, symbol: ClientVmCvarSymbol, value: i32) {
        let previous = self.read_vm_symbol(symbol);
        self.cache.borrow_mut().insert(
            symbol,
            CvarSnapshot {
                integer_value: value,
                ..previous
            },
        );
    }

    /// Read a VM cvar by name (`readVmCvar`).
    #[must_use]
    pub fn read_vm_cvar(&self, name: &str) -> CvarSnapshot {
        let symbol = self
            .names
            .borrow()
            .get(&name.to_lowercase())
            .copied()
            .unwrap_or_else(|| {
                panic!("Cgame VM cvar {name} is not registered for {}", self.product.as_str());
            });
        self.read_vm_symbol(symbol)
    }

    /// Force a model change (`forceModelChange`).
    pub fn force_model_change(&self) {
        if !self.registered.get() {
            panic!("Client cvars must be registered before forcing models");
        }
        if self.updating.get() {
            panic!("Client configuration update is already active");
        }
        self.updating.set(true);
        self.reload_client_info();
        self.updating.set(false);
    }

    /// Update cvars (`updateCvars`).
    pub fn update_cvars(&self) {
        if !self.registered.get() {
            panic!("Client cvars must be registered before updating");
        }
        if self.updating.get() {
            panic!("Client configuration update is already active");
        }
        self.updating.set(true);
        for definition in &self.table.clone() {
            if let Some(engine) = self.host.cvars.borrow().get(&definition.name) {
                self.copy_definition(definition, &engine, false);
            }
        }
        let overlay = self.read_vm_symbol(ClientVmCvarSymbol::CgDrawTeamOverlay);
        if self.draw_team_overlay_modification_count.get() != i64::from(overlay.modification_count) {
            self.draw_team_overlay_modification_count
                .set(i64::from(overlay.modification_count));
            let _ = self.host.cvars.borrow_mut().set(
                "teamoverlay",
                if overlay.integer_value > 0 { "1" } else { "0" },
                true,
            );
            let _ = self.host.cvars.borrow_mut().set("teamoverlay", "1", true);
        }
        let force_model = self.read_vm_symbol(ClientVmCvarSymbol::CgForceModel);
        if self.force_model_modification_count.get() != force_model.modification_count {
            self.force_model_modification_count.set(force_model.modification_count);
            self.reload_client_info();
        }
        self.updating.set(false);
    }

    /// Copy one definition (`copyDefinition`).
    fn copy_definition(&self, definition: &CvarDefinition, engine: &CvarSnapshot, forced: bool) {
        let previous = self.cache.borrow().get(&definition.symbol).cloned();
        if !forced {
            if let Some(previous) = &previous {
                if previous.modification_count == engine.modification_count {
                    return;
                }
            }
        }
        let retained = previous.unwrap_or_else(|| CvarSnapshot {
            name: engine.name.clone(),
            value: String::new(),
            reset_value: String::new(),
            latched_value: None,
            flags: 0,
            modified: false,
            modification_count: 0,
            numeric_value: 0.0,
            integer_value: 0,
        });
        self.cache.borrow_mut().insert(
            definition.symbol,
            CvarSnapshot {
                value: retained.value,
                numeric_value: retained.numeric_value,
                integer_value: retained.integer_value,
                modification_count: engine.modification_count,
                ..engine.clone()
            },
        );
        let changed = changed_snapshot(engine);
        self.cache.borrow_mut().insert(definition.symbol, changed);
    }

    /// Reload client info (`reloadClientInfo`).
    fn reload_client_info(&self) {
        for index in 0..CONFIG_MAX_CLIENTS {
            let config = self.host.strings.borrow().config_string(CS_PLAYERS + index as usize);
            if !config.is_empty() {
                self.host.clients.borrow_mut().new_client_info(index, &config);
            }
        }
    }
}

impl HudCvarReader for ClientConfiguration {
    fn read_vm_cvar(&self, name: &str) -> CvarSnapshot {
        self.read_vm_cvar(name)
    }
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

/// Seat cinematics handle (`EngineUiCinematics`).
pub type EngineUiCinematics = Shared<dyn CinematicService>;

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

/// Mission owner-draw id (`MissionOwnerDrawId`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum MissionOwnerDrawId {
    /// Player armor icon.
    PlayerArmorIcon = 1,
    /// Player armor value.
    PlayerArmorValue = 2,
    /// Player head.
    PlayerHead = 3,
    /// Player health.
    PlayerHealth = 4,
    /// Player ammo icon.
    PlayerAmmoIcon = 5,
    /// Player ammo value.
    PlayerAmmoValue = 6,
    /// Selected player head.
    SelectedPlayerHead = 7,
    /// Selected player name.
    SelectedPlayerName = 8,
    /// Selected player location.
    SelectedPlayerLocation = 9,
    /// Selected player status.
    SelectedPlayerStatus = 10,
    /// Selected player weapon.
    SelectedPlayerWeapon = 11,
    /// Selected player powerup.
    SelectedPlayerPowerup = 12,
    /// Player item.
    PlayerItem = 19,
    /// Player score.
    PlayerScore = 20,
    /// Blue flag head.
    BlueFlagHead = 21,
    /// Blue flag status.
    BlueFlagStatus = 22,
    /// Blue flag name.
    BlueFlagName = 23,
    /// Red flag head.
    RedFlagHead = 24,
    /// Red flag status.
    RedFlagStatus = 25,
    /// Red flag name.
    RedFlagName = 26,
    /// Blue score.
    BlueScore = 27,
    /// Red score.
    RedScore = 28,
    /// Red name.
    RedName = 29,
    /// Blue name.
    BlueName = 30,
    /// Harvester skulls.
    HarvesterSkulls = 31,
    /// One-flag status.
    OneFlagStatus = 32,
    /// Player location.
    PlayerLocation = 33,
    /// Team color.
    TeamColor = 34,
    /// CTF powerup.
    CtfPowerup = 35,
    /// Area powerup.
    AreaPowerup = 36,
    /// Player has flag.
    PlayerHasFlag = 38,
    /// Game type.
    GameType = 39,
    /// Selected player armor.
    SelectedPlayerArmor = 40,
    /// Selected player health.
    SelectedPlayerHealth = 41,
    /// Player status.
    PlayerStatus = 42,
    /// Area system chat.
    AreaSystemChat = 46,
    /// Area team chat.
    AreaTeamChat = 47,
    /// Area chat.
    AreaChat = 48,
    /// Game status.
    GameStatus = 49,
    /// Killer.
    Killer = 50,
    /// Player armor icon 2D.
    PlayerArmorIcon2d = 51,
    /// Player ammo icon 2D.
    PlayerAmmoIcon2d = 52,
    /// Accuracy.
    Accuracy = 53,
    /// Assists.
    Assists = 54,
    /// Defend.
    Defend = 55,
    /// Excellent.
    Excellent = 56,
    /// Impressive.
    Impressive = 57,
    /// Perfect.
    Perfect = 58,
    /// Gauntlet.
    Gauntlet = 59,
    /// Spectators.
    Spectators = 60,
    /// Team info.
    TeamInfo = 61,
    /// Voice head.
    VoiceHead = 62,
    /// Voice name.
    VoiceName = 63,
    /// Player has flag 2D.
    PlayerHasFlag2d = 64,
    /// Harvester skulls 2D.
    HarvesterSkulls2d = 65,
    /// Capture/frag limit.
    CapFragLimit = 66,
    /// First place.
    FirstPlace = 67,
    /// Second place.
    SecondPlace = 68,
    /// Captures.
    Captures = 69,
}

impl MissionOwnerDrawId {
    /// Convert a raw id.
    #[must_use]
    pub fn from_i32(value: i32) -> Option<Self> {
        match value {
            1 => Some(Self::PlayerArmorIcon),
            2 => Some(Self::PlayerArmorValue),
            3 => Some(Self::PlayerHead),
            4 => Some(Self::PlayerHealth),
            5 => Some(Self::PlayerAmmoIcon),
            6 => Some(Self::PlayerAmmoValue),
            7 => Some(Self::SelectedPlayerHead),
            8 => Some(Self::SelectedPlayerName),
            9 => Some(Self::SelectedPlayerLocation),
            10 => Some(Self::SelectedPlayerStatus),
            11 => Some(Self::SelectedPlayerWeapon),
            12 => Some(Self::SelectedPlayerPowerup),
            19 => Some(Self::PlayerItem),
            20 => Some(Self::PlayerScore),
            21 => Some(Self::BlueFlagHead),
            22 => Some(Self::BlueFlagStatus),
            23 => Some(Self::BlueFlagName),
            24 => Some(Self::RedFlagHead),
            25 => Some(Self::RedFlagStatus),
            26 => Some(Self::RedFlagName),
            27 => Some(Self::BlueScore),
            28 => Some(Self::RedScore),
            29 => Some(Self::RedName),
            30 => Some(Self::BlueName),
            31 => Some(Self::HarvesterSkulls),
            32 => Some(Self::OneFlagStatus),
            33 => Some(Self::PlayerLocation),
            34 => Some(Self::TeamColor),
            35 => Some(Self::CtfPowerup),
            36 => Some(Self::AreaPowerup),
            38 => Some(Self::PlayerHasFlag),
            39 => Some(Self::GameType),
            40 => Some(Self::SelectedPlayerArmor),
            41 => Some(Self::SelectedPlayerHealth),
            42 => Some(Self::PlayerStatus),
            46 => Some(Self::AreaSystemChat),
            47 => Some(Self::AreaTeamChat),
            48 => Some(Self::AreaChat),
            49 => Some(Self::GameStatus),
            50 => Some(Self::Killer),
            51 => Some(Self::PlayerArmorIcon2d),
            52 => Some(Self::PlayerAmmoIcon2d),
            53 => Some(Self::Accuracy),
            54 => Some(Self::Assists),
            55 => Some(Self::Defend),
            56 => Some(Self::Excellent),
            57 => Some(Self::Impressive),
            58 => Some(Self::Perfect),
            59 => Some(Self::Gauntlet),
            60 => Some(Self::Spectators),
            61 => Some(Self::TeamInfo),
            62 => Some(Self::VoiceHead),
            63 => Some(Self::VoiceName),
            64 => Some(Self::PlayerHasFlag2d),
            65 => Some(Self::HarvesterSkulls2d),
            66 => Some(Self::CapFragLimit),
            67 => Some(Self::FirstPlace),
            68 => Some(Self::SecondPlace),
            69 => Some(Self::Captures),
            _ => None,
        }
    }
}

/// Mission owner-draw flags (`MissionOwnerDrawFlags`).
pub mod owner_draw_flags {
    /// Blue team has red flag.
    pub const SHOW_BLUE_TEAM_HAS_REDFLAG: i32 = 0x1;
    /// Red team has blue flag.
    pub const SHOW_RED_TEAM_HAS_BLUEFLAG: i32 = 0x2;
    /// Any team game.
    pub const SHOW_ANYTEAMGAME: i32 = 0x4;
    /// Harvester.
    pub const SHOW_HARVESTER: i32 = 0x8;
    /// One flag.
    pub const SHOW_ONEFLAG: i32 = 0x10;
    /// CTF.
    pub const SHOW_CTF: i32 = 0x20;
    /// Obelisk.
    pub const SHOW_OBELISK: i32 = 0x40;
    /// Health critical.
    pub const SHOW_HEALTHCRITICAL: i32 = 0x80;
    /// Single player.
    pub const SHOW_SINGLEPLAYER: i32 = 0x100;
    /// Tournament.
    pub const SHOW_TOURNAMENT: i32 = 0x200;
    /// During incoming voice.
    pub const SHOW_DURINGINCOMINGVOICE: i32 = 0x400;
    /// Player has flag.
    pub const SHOW_IF_PLAYER_HAS_FLAG: i32 = 0x800;
    /// LAN play only.
    pub const SHOW_LANPLAYONLY: i32 = 0x1000;
    /// Mined.
    pub const SHOW_MINED: i32 = 0x2000;
    /// Health OK.
    pub const SHOW_HEALTHOK: i32 = 0x4000;
    /// Team info.
    pub const SHOW_TEAMINFO: i32 = 0x8000;
    /// No team info.
    pub const SHOW_NOTEAMINFO: i32 = 0x10000;
    /// Other team has flag.
    pub const SHOW_OTHERTEAMHASFLAG: i32 = 0x20000;
    /// Your team has enemy flag.
    pub const SHOW_YOURTEAMHASENEMYFLAG: i32 = 0x40000;
    /// Any non-team game.
    pub const SHOW_ANYNONTEAMGAME: i32 = 0x80000;
    /// 2D only.
    pub const SHOW_2DONLY: i32 = 0x10000000;
}

/// HUD chat text.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HudChatText {
    /// System.
    pub system: String,
    /// Team 1.
    pub team1: String,
    /// Team 2.
    pub team2: String,
}

/// Mission owner-draw host services (`MissionOwnerDrawHost`).
pub struct MissionOwnerDrawHost {
    /// Weapon HUD reader.
    pub weapon_hud: Option<Shared<dyn WeaponHudReader>>,
    /// Draw icons.
    pub icons: Shared<ClientDrawIcons>,
    /// Live fonts.
    pub fonts: Shared<FontSet>,
    /// Configuration.
    pub configuration: Shared<ClientConfiguration>,
    /// Random.
    pub random: Shared<GameRandom>,
    /// Configstrings.
    pub strings: Shared<dyn HudConfigStrings>,
    /// Selected player reader.
    pub selected_player: Rc<dyn Fn() -> i32>,
    /// Chat reader.
    pub chat: Rc<dyn Fn() -> HudChatText>,
}

/// Mission owner drawing (`MissionOwnerDraw`).
pub struct MissionOwnerDraw {
    /// Frame state.
    pub state: Shared<ClientGameState>,
    /// Static state.
    pub static_state: Shared<ClientGameStaticState>,
    /// Media.
    pub media: Shared<ClientMedia>,
    /// Host.
    pub host: MissionOwnerDrawHost,
}

impl MissionOwnerDraw {
    /// Assemble owner drawing.
    pub fn new(
        state: Shared<ClientGameState>,
        static_state: Shared<ClientGameStaticState>,
        media: Shared<ClientMedia>,
        host: MissionOwnerDrawHost,
    ) -> Self {
        if state.borrow().product != Product::Missionpack
            || static_state.borrow().product != Product::Missionpack
            || !same(&media.borrow().static_state, &static_state)
            || !same(&host.icons.borrow().state, &state)
            || !same(&host.icons.borrow().tools.media, &media)
        {
            panic!("Mission owner drawing requires canonical Team Arena state and media");
        }
        Self {
            state,
            static_state,
            media,
            host,
        }
    }

    /// Current player state.
    fn ps(&self) -> PlayerState {
        self.state
            .borrow()
            .snap
            .clone()
            .unwrap_or_else(|| {
                panic!("Mission owner drawing requires a current snapshot");
            })
            .player_state
    }

    /// Read an integer cvar.
    fn cvar(&self, name: &str) -> i32 {
        self.host.configuration.borrow().read_vm_cvar(name).integer_value
    }

    /// Client slot.
    fn client(&self, index: i32) -> Shared<ClientInfo> {
        self.static_state
            .borrow()
            .client_info
            .get(index as usize)
            .cloned()
            .unwrap_or_else(|| {
                panic!("Invalid owner-draw slot {index}");
            })
    }

    /// Selected sorted index.
    fn selected_index(&self) -> i32 {
        let selected = (self.host.selected_player)();
        self.state
            .borrow()
            .sorted_team_players
            .get(selected as usize)
            .copied()
            .unwrap_or_else(|| {
                panic!("Invalid owner-draw slot {selected}");
            })
    }

    /// Selected client.
    fn selected(&self) -> Shared<ClientInfo> {
        let index = self.selected_index();
        self.client(index)
    }

    /// Current team.
    fn team(&self) -> i32 {
        self.ps().persistant.get(PersistentIndex::Team as i32)
    }

    /// Location name.
    fn location(&self, index: i32) -> String {
        let text = self.host.strings.borrow().config_string(CS_LOCATIONS + index as usize);
        if text.is_empty() {
            "unknown".to_string()
        } else {
            text
        }
    }

    /// Text width.
    fn width_text(&self, text: &str, scale: f32) -> f32 {
        text_width(&self.host.fonts.borrow(), text, scale, 0) as f32
    }

    /// Paint text.
    fn text(&self, rect: Rect2d, scale: f32, color: Vec4, text: &str, style: i32, x: f32, y: f32) {
        let draw = self.host.icons.borrow().tools.draw.clone();
        text_paint(
            &draw,
            &self.host.fonts.borrow(),
            &TextPaintOptions {
                x,
                y,
                scale,
                color,
                text: text.to_string(),
                adjust: 0.0,
                limit: 0,
                style,
            },
        );
        let _ = rect;
    }

    /// Paint a number.
    fn number(&self, rect: Rect2d, scale: f32, color: Vec4, value: i32, picture: &Option<Picture>, style: i32) {
        if let Some(picture) = picture {
            let draw = self.host.icons.borrow().tools.draw.clone();
            draw.set_color(Some(color));
            draw.stretch_pic(
                rect,
                TextureRect {
                    s: 0.0,
                    t: 0.0,
                    s2: 1.0,
                    t2: 1.0,
                },
                *picture,
            );
            draw.set_color(None);
        } else {
            let text = format!("{}", value);
            let width = self.width_text(&text, scale);
            self.text(
                rect,
                scale,
                color,
                &text,
                style,
                rect.x + (rect.width - width) / 2.0,
                rect.y + rect.height,
            );
        }
    }

    /// Status handle for a task (`statusHandle`).
    #[must_use]
    pub fn status_handle(&self, task: i32) -> Option<SceneShader> {
        let graphics = self.media.borrow().graphics.clone();
        match task {
            2 => graphics.defend_shader,
            3 => graphics.patrol_shader,
            4 => graphics.follow_shader,
            5 => graphics.retrieve_shader,
            6 => graphics.escort_shader,
            7 => graphics.camp_shader,
            _ => graphics.assault_shader,
        }
    }

    /// Owner-draw value (`value`).
    #[must_use]
    pub fn value(&self, id: i32) -> f32 {
        self.raw_value(id) as f32
    }

    /// Raw owner-draw value (`rawValue`).
    fn raw_value(&self, id: i32) -> i32 {
        let ps = self.ps();
        let schema = stat_schema(Product::Missionpack);
        match MissionOwnerDrawId::from_i32(id) {
            Some(MissionOwnerDrawId::SelectedPlayerArmor) => self.selected().borrow().armor,
            Some(MissionOwnerDrawId::SelectedPlayerHealth) => self.selected().borrow().health,
            Some(MissionOwnerDrawId::PlayerArmorValue) => ps.stats.get(schema.armor),
            Some(MissionOwnerDrawId::PlayerAmmoValue) => {
                if let Some(weapon_hud) = &self.host.weapon_hud {
                    let status = weapon_hud.borrow().read_weapon_hud().status;
                    match status.map(|status| status.ammo) {
                        Some(WeaponHudAmmo::Finite { count }) => count,
                        _ => -1,
                    }
                } else {
                    let weapon = self.state.borrow().entity_at(ps.client_num).current_state.weapon;
                    if weapon != 0 {
                        ps.ammo.get(weapon)
                    } else {
                        -1
                    }
                }
            }
            Some(MissionOwnerDrawId::PlayerScore) => ps.persistant.get(PersistentIndex::Score as i32),
            Some(MissionOwnerDrawId::PlayerHealth) => ps.stats.get(schema.health),
            Some(MissionOwnerDrawId::RedScore) => self.static_state.borrow().scores1,
            Some(MissionOwnerDrawId::BlueScore) => self.static_state.borrow().scores2,
            _ => -1,
        }
    }

    /// Whether the other team has the flag (`otherTeamHasFlag`).
    #[must_use]
    pub fn other_team_has_flag(&self) -> bool {
        let cgs = self.static_state.borrow();
        if cgs.game_type != GameType::OneFlagCtf && cgs.game_type != GameType::Ctf {
            return false;
        }
        let team = self.team();
        if cgs.game_type == GameType::OneFlagCtf {
            return team == Team::Red as i32 && cgs.flag_status == 3
                || team == Team::Blue as i32 && cgs.flag_status == 2;
        }
        if cgs.game_type == GameType::Ctf {
            return team == Team::Red as i32 && cgs.redflag == 1 || team == Team::Blue as i32 && cgs.blueflag == 1;
        }
        false
    }

    /// Whether your team has the flag (`yourTeamHasFlag`).
    #[must_use]
    pub fn your_team_has_flag(&self) -> bool {
        let cgs = self.static_state.borrow();
        if cgs.game_type != GameType::OneFlagCtf && cgs.game_type != GameType::Ctf {
            return false;
        }
        let team = self.team();
        if cgs.game_type == GameType::OneFlagCtf {
            return team == Team::Red as i32 && cgs.flag_status == 2
                || team == Team::Blue as i32 && cgs.flag_status == 3;
        }
        if cgs.game_type == GameType::Ctf {
            return team == Team::Red as i32 && cgs.blueflag == 1 || team == Team::Blue as i32 && cgs.redflag == 1;
        }
        false
    }

    /// Owner-draw visibility (`visible`).
    #[must_use]
    pub fn visible(&self, flags: i32) -> bool {
        use owner_draw_flags as SHOW;
        let game_type = self.static_state.borrow().game_type;
        if flags & SHOW::SHOW_TEAMINFO != 0 {
            return self.cvar("cg_currentSelectedPlayer") == self.state.borrow().num_sorted_team_players;
        }
        if flags & SHOW::SHOW_NOTEAMINFO != 0 {
            return self.cvar("cg_currentSelectedPlayer") != self.state.borrow().num_sorted_team_players;
        }
        if flags & SHOW::SHOW_OTHERTEAMHASFLAG != 0 {
            return self.other_team_has_flag();
        }
        if flags & SHOW::SHOW_YOURTEAMHASENEMYFLAG != 0 {
            return self.your_team_has_flag();
        }
        if flags & (SHOW::SHOW_BLUE_TEAM_HAS_REDFLAG | SHOW::SHOW_RED_TEAM_HAS_BLUEFLAG) != 0 {
            let cgs = self.static_state.borrow();
            return flags & SHOW::SHOW_BLUE_TEAM_HAS_REDFLAG != 0 && (cgs.redflag == 1 || cgs.flag_status == 2)
                || flags & SHOW::SHOW_RED_TEAM_HAS_BLUEFLAG != 0 && (cgs.blueflag == 1 || cgs.flag_status == 3);
        }
        if flags & SHOW::SHOW_ANYTEAMGAME != 0 && game_type >= GameType::Team {
            return true;
        }
        if flags & SHOW::SHOW_ANYNONTEAMGAME != 0 && game_type < GameType::Team {
            return true;
        }
        if flags & SHOW::SHOW_HARVESTER != 0 {
            return game_type == GameType::Harvester;
        }
        if flags & SHOW::SHOW_ONEFLAG != 0 {
            return game_type == GameType::OneFlagCtf;
        }
        if flags & SHOW::SHOW_CTF != 0 && game_type == GameType::Ctf {
            return true;
        }
        if flags & SHOW::SHOW_OBELISK != 0 {
            return game_type == GameType::Obelisk;
        }
        if flags & SHOW::SHOW_HEALTHCRITICAL != 0 && self.ps().stats.get(stat_schema(Product::Missionpack).health) < 25
        {
            return true;
        }
        if flags & SHOW::SHOW_HEALTHOK != 0 && self.ps().stats.get(stat_schema(Product::Missionpack).health) >= 25 {
            return true;
        }
        if flags & SHOW::SHOW_SINGLEPLAYER != 0 && game_type == GameType::SinglePlayer {
            return true;
        }
        if flags & SHOW::SHOW_TOURNAMENT != 0 && game_type == GameType::Tournament {
            return true;
        }
        if flags & SHOW::SHOW_IF_PLAYER_HAS_FLAG != 0 {
            let ps = self.ps();
            return ps.powerups.get(Powerup::RedFlag as i32) != 0
                || ps.powerups.get(Powerup::BlueFlag as i32) != 0
                || ps.powerups.get(Powerup::NeutralFlag as i32) != 0;
        }
        false
    }

    /// Game type text.
    fn game_type_text(&self) -> String {
        match self.static_state.borrow().game_type {
            GameType::Ffa => "Free For All".to_string(),
            GameType::Team => "Team Deathmatch".to_string(),
            GameType::Ctf => "Capture the Flag".to_string(),
            GameType::OneFlagCtf => "One Flag CTF".to_string(),
            GameType::Obelisk => "Overload".to_string(),
            GameType::Harvester => "Harvester".to_string(),
            _ => String::new(),
        }
    }

    /// Game status text.
    fn game_status_text(&self) -> String {
        if self.static_state.borrow().game_type < GameType::Team {
            if self.team() == Team::Spectator as i32 {
                return String::new();
            }
            let ps = self.ps();
            return format!(
                "{} place with {}",
                place_string(ps.persistant.get(PersistentIndex::Rank as i32).wrapping_add(1)),
                ps.persistant.get(PersistentIndex::Score as i32)
            );
        }
        let team_scores = self.state.borrow().team_scores;
        if team_scores[0] == team_scores[1] {
            format!("Teams are tied at {}", team_scores[0])
        } else if team_scores[0] >= team_scores[1] {
            format!("Red leads Blue, {} to {}", team_scores[0], team_scores[1])
        } else {
            format!("Blue leads Red, {} to {}", team_scores[1], team_scores[0])
        }
    }

    /// Killer text.
    fn killer_text(&self) -> String {
        let killer = self.state.borrow().killer_name.clone();
        if killer.is_empty() {
            String::new()
        } else {
            format!("Fragged by {killer}")
        }
    }

    /// Owner-draw width (`width`).
    #[must_use]
    pub fn width(&self, id: i32, scale: f32) -> f32 {
        match MissionOwnerDrawId::from_i32(id) {
            Some(MissionOwnerDrawId::GameType) => self.width_text(&self.game_type_text(), scale),
            Some(MissionOwnerDrawId::GameStatus) => self.width_text(&self.game_status_text(), scale),
            Some(MissionOwnerDrawId::Killer) => self.width_text(&self.killer_text(), scale),
            Some(MissionOwnerDrawId::RedName) => {
                self.width_text(&self.host.configuration.borrow().read_vm_cvar("g_redteam").value, scale)
            }
            Some(MissionOwnerDrawId::BlueName) => self.width_text(
                &self.host.configuration.borrow().read_vm_cvar("g_blueteam").value,
                scale,
            ),
            _ => 0.0,
        }
    }

    /// Armor icon.
    fn armor_icon(&self, rect: Rect2d, force_2d: bool) {
        if self.cvar("cg_drawStatus") == 0 {
            return;
        }
        if force_2d || self.cvar("cg_draw3dIcons") == 0 && self.cvar("cg_drawIcons") != 0 {
            let icon = self.media.borrow().graphics.armor_icon.clone();
            self.host.icons.borrow().tools.draw_pic(
                Rect2d {
                    y: rect.y + rect.height / 2.0 + 1.0,
                    ..rect
                },
                &icon,
            );
        } else if self.cvar("cg_draw3dIcons") != 0 {
            let model = self.media.borrow().graphics.armor_model.clone();
            let time = self.state.borrow().time;
            self.host.icons.borrow().draw_3d_model(
                rect,
                &model,
                None,
                vec3(90.0, 0.0, -10.0),
                vec3(0.0, (time & 2047) as f32 * 360.0 / 2048.0, 0.0),
            );
        }
    }

    /// Ammo icon.
    fn ammo_icon(&self, rect: Rect2d, force_2d: bool) {
        if self.host.weapon_hud.is_some() {
            return;
        }
        if force_2d || self.cvar("cg_draw3dIcons") == 0 && self.cvar("cg_drawIcons") != 0 {
            let weapon = self.state.borrow().predicted_player_state.weapon;
            let icon = self.media.borrow().weapon_registry.borrow().weapon(weapon).ammo_icon;
            if let Some(icon) = icon {
                self.host.icons.borrow().tools.draw_pic(rect, &Some(icon));
            }
        } else if self.cvar("cg_draw3dIcons") != 0 {
            let ps = self.ps();
            let weapon = self.state.borrow().entity_at(ps.client_num).current_state.weapon;
            let model = self.media.borrow().weapon_registry.borrow().weapon(weapon).ammo_model;
            if weapon != 0 && !model.is_default() {
                let time = self.state.borrow().time;
                self.host.icons.borrow().draw_3d_model(
                    rect,
                    &model,
                    None,
                    vec3(70.0, 0.0, 0.0),
                    vec3(0.0, 90.0 + 20.0 * (time as f32 / 1000.0).sin(), 0.0),
                );
            }
        }
    }

    /// Player head.
    fn player_head(&self, rect: Rect2d) {
        let time = self.state.borrow().time;
        let damage_time = self.state.borrow().damage_time;
        let mut x = rect.x;
        if damage_time != 0 && (time as f32 - damage_time as f32) < 500.0 {
            let frac = (time as f32 - damage_time as f32) / 500.0;
            let size = rect.width * 1.25 * (1.5 - frac * 0.5);
            let stretch = size - rect.width * 1.25;
            let damage_x = self.state.borrow().damage_x;
            x -= stretch * 0.5 + damage_x * stretch * 0.5;
            let mut random = self.host.random.borrow_mut();
            self.state.borrow_mut().head_start_yaw = 180.0 + damage_x * 45.0;
            self.state.borrow_mut().head_end_yaw = 180.0 + 20.0 * (random.random() * std::f32::consts::PI).cos();
            self.state.borrow_mut().head_end_pitch = 5.0 * (random.random() * std::f32::consts::PI).cos();
            self.state.borrow_mut().head_start_time = time;
            self.state.borrow_mut().head_end_time = ((time + 100) as f32 + random.random() * 2000.0).trunc() as i32;
        } else if time >= self.state.borrow().head_end_time {
            let (end_yaw, end_pitch, end_time) = {
                let state = self.state.borrow();
                (state.head_end_yaw, state.head_end_pitch, state.head_end_time)
            };
            {
                let mut state = self.state.borrow_mut();
                state.head_start_yaw = end_yaw;
                state.head_start_pitch = end_pitch;
                state.head_start_time = end_time;
            }
            let mut random = self.host.random.borrow_mut();
            self.state.borrow_mut().head_end_time = ((time + 100) as f32 + random.random() * 2000.0).trunc() as i32;
            self.state.borrow_mut().head_end_yaw = 180.0 + 20.0 * (random.random() * std::f32::consts::PI).cos();
            self.state.borrow_mut().head_end_pitch = 5.0 * (random.random() * std::f32::consts::PI).cos();
        }
        if self.state.borrow().head_start_time > time {
            self.state.borrow_mut().head_start_time = time;
        }
        let (start_yaw, end_yaw, start_pitch, end_pitch, start_time, end_time) = {
            let state = self.state.borrow();
            (
                state.head_start_yaw,
                state.head_end_yaw,
                state.head_start_pitch,
                state.head_end_pitch,
                state.head_start_time,
                state.head_end_time,
            )
        };
        let mut frac = time.wrapping_sub(start_time) as f32 / end_time.wrapping_sub(start_time) as f32;
        frac = frac * frac * (3.0 - 2.0 * frac);
        let client_num = self.ps().client_num;
        self.host.icons.borrow().draw_head(
            Rect2d { x, ..rect },
            client_num,
            vec3(
                start_pitch + (end_pitch - start_pitch) * frac,
                start_yaw + (end_yaw - start_yaw) * frac,
                0.0,
            ),
        );
    }

    /// Selected status.
    fn selected_status(&self, rect: Rect2d) {
        let team_task = self.selected().borrow().team_task;
        let (order_pending, order_time, current_order, time) = {
            let cgs = self.static_state.borrow();
            (
                cgs.order_pending,
                cgs.order_time,
                cgs.current_order,
                self.state.borrow().time,
            )
        };
        if order_pending && time > order_time.wrapping_sub(2500) && (time >> 9) & 1 != 0 {
            return;
        }
        let handle = self.status_handle(if order_pending { current_order } else { team_task });
        self.host.icons.borrow().tools.draw_pic(rect, &handle);
    }

    /// Flag carrier.
    fn flag_carrier(&self, blue: bool) -> Option<i32> {
        for index in 0..self.static_state.borrow().maxclients {
            let client = self.client(index);
            let client = client.borrow();
            let wanted_team = if blue { Team::Red } else { Team::Blue };
            let wanted_flag = if blue { Powerup::BlueFlag } else { Powerup::RedFlag };
            if client.info_valid && client.team == wanted_team && client.powerups & (1 << wanted_flag as i32) != 0 {
                return Some(index);
            }
        }
        None
    }

    /// Flag head.
    fn flag_head(&self, rect: Rect2d, blue: bool) {
        if self.flag_carrier(blue).is_some() {
            let time = self.state.borrow().time;
            self.host
                .icons
                .borrow()
                .draw_head(rect, 0, vec3(0.0, 180.0 + 20.0 * (time as f32 / 650.0).sin(), 0.0));
        }
    }

    /// Flag status.
    fn flag_status(&self, rect: Rect2d, blue: bool, picture: &Option<Picture>) {
        let game_type = self.static_state.borrow().game_type;
        if game_type != GameType::Ctf && game_type != GameType::OneFlagCtf {
            if game_type == GameType::Harvester {
                let tools = self.host.icons.borrow().tools.clone();
                let icon = if blue {
                    tools.media.borrow().graphics.blue_cube_icon.clone()
                } else {
                    tools.media.borrow().graphics.red_cube_icon.clone()
                };
                tools.draw.set_color(Some(if blue {
                    vec4(0.0, 0.0, 1.0, 1.0)
                } else {
                    vec4(1.0, 0.0, 0.0, 1.0)
                }));
                tools.draw_pic(rect, &icon);
                tools.draw.set_color(None);
            }
            return;
        }
        if let Some(picture) = picture {
            self.host.icons.borrow().tools.draw.stretch_pic(
                rect,
                TextureRect {
                    s: 0.0,
                    t: 0.0,
                    s2: 1.0,
                    t2: 1.0,
                },
                *picture,
            );
        } else {
            let powerup = if blue { Powerup::BlueFlag } else { Powerup::RedFlag };
            if self
                .media
                .borrow()
                .items
                .borrow()
                .find_for_powerup(Product::Missionpack, powerup as i32)
                .is_none()
            {
                return;
            }
            let status = if blue {
                self.static_state.borrow().blueflag
            } else {
                self.static_state.borrow().redflag
            };
            let tools = self.host.icons.borrow().tools.clone();
            tools.draw.set_color(Some(if blue {
                vec4(0.0, 0.0, 1.0, 1.0)
            } else {
                vec4(1.0, 0.0, 0.0, 1.0)
            }));
            let shader = tools
                .media
                .borrow()
                .graphics
                .flag_shaders
                .get(if (0..=2).contains(&status) { status as usize } else { 0 })
                .cloned()
                .unwrap_or_else(|| panic!("Invalid owner-draw slot {status}"));
            tools.draw_pic(rect, &shader);
            tools.draw.set_color(None);
        }
    }

    /// One-flag status.
    fn one_flag_status(&self, rect: Rect2d) {
        let status = self.static_state.borrow().flag_status;
        if self.static_state.borrow().game_type != GameType::OneFlagCtf
            || self
                .media
                .borrow()
                .items
                .borrow()
                .find_for_powerup(Product::Missionpack, Powerup::NeutralFlag as i32)
                .is_none()
            || !(0..=4).contains(&status)
        {
            return;
        }
        let tools = self.host.icons.borrow().tools.clone();
        tools.draw.set_color(Some(if status == 2 {
            vec4(1.0, 0.0, 0.0, 1.0)
        } else if status == 3 {
            vec4(0.0, 0.0, 1.0, 1.0)
        } else {
            vec4(1.0, 1.0, 1.0, 1.0)
        }));
        let shader = tools
            .media
            .borrow()
            .graphics
            .flag_shaders
            .get(if status == 2 || status == 3 {
                1
            } else if status == 4 {
                2
            } else {
                0
            })
            .cloned()
            .unwrap_or_else(|| panic!("Invalid owner-draw slot {status}"));
        tools.draw_pic(rect, &shader);
        // The source intentionally leaves this color set for the next draw.
    }

    /// Harvester skulls.
    fn skulls(&self, rect: Rect2d, scale: f32, color: Vec4, force_2d: bool, style: i32) {
        if self.static_state.borrow().game_type != GameType::Harvester {
            return;
        }
        let text = format!("{}", self.ps().generic1.min(99));
        let width = self.width_text(&text, scale);
        self.text(
            rect,
            scale,
            color,
            &text,
            style,
            rect.x + rect.width - width,
            rect.y + rect.height,
        );
        if self.cvar("cg_drawIcons") == 0 {
            return;
        }
        let red = self.team() == Team::Blue as i32;
        if !force_2d && self.cvar("cg_draw3dIcons") != 0 {
            let model = if red {
                self.media.borrow().graphics.red_cube_model.clone()
            } else {
                self.media.borrow().graphics.blue_cube_model.clone()
            };
            let time = self.state.borrow().time;
            self.host.icons.borrow().draw_3d_model(
                Rect2d {
                    width: 35.0,
                    height: 35.0,
                    ..rect
                },
                &model,
                None,
                vec3(90.0, 0.0, -10.0),
                vec3(0.0, (time & 2047) as f32 * 360.0 / 2048.0, 0.0),
            );
        } else {
            let icon = if red {
                self.media.borrow().graphics.red_cube_icon.clone()
            } else {
                self.media.borrow().graphics.blue_cube_icon.clone()
            };
            self.host
                .icons
                .borrow()
                .tools
                .draw_pic(rect2d(rect.x + 3.0, rect.y + 16.0, 20.0, 20.0), &icon);
        }
    }

    /// Player has flag.
    fn player_has_flag(&self, rect: Rect2d, force_2d: bool) {
        let ps = self.state.borrow().predicted_player_state.clone();
        let adj = if force_2d { 0.0 } else { 2.0 };
        let adjusted = rect2d(rect.x + adj, rect.y + adj, rect.width - adj, rect.height - adj);
        if ps.powerups.get(Powerup::RedFlag as i32) != 0 {
            self.host
                .icons
                .borrow()
                .draw_flag_model(adjusted, Team::Red as i32, force_2d);
        } else if ps.powerups.get(Powerup::BlueFlag as i32) != 0 {
            self.host
                .icons
                .borrow()
                .draw_flag_model(adjusted, Team::Blue as i32, force_2d);
        } else if ps.powerups.get(Powerup::NeutralFlag as i32) != 0 {
            self.host
                .icons
                .borrow()
                .draw_flag_model(adjusted, Team::Free as i32, force_2d);
        }
    }

    /// Persistent/holdable item.
    fn item(&self, rect: Rect2d, persistent: bool) {
        if persistent && self.static_state.borrow().game_type < GameType::Ctf {
            return;
        }
        let value = self.ps().stats.get(if persistent {
            MissionpackStatIndex::PersistantPowerup as i32
        } else {
            MissionpackStatIndex::HoldableItem as i32
        });
        if value == 0 {
            return;
        }
        self.media
            .borrow()
            .weapon_registry
            .borrow_mut()
            .register_item_visuals(value);
        if !persistent {
            self.media
                .borrow()
                .weapon_registry
                .borrow_mut()
                .register_item_visuals(value);
        }
        let icon = self
            .media
            .borrow()
            .weapon_registry
            .borrow()
            .item_visual(value as usize)
            .icon;
        self.host.icons.borrow().tools.draw_pic(rect, &icon);
    }

    /// Selected powerup.
    fn selected_powerup(&self, rect: Rect2d) {
        let powerups = self.selected().borrow().powerups;
        for slot in 0..Powerup::NumPowerups as i32 {
            if powerups & (1 << slot) == 0 {
                continue;
            }
            let item = self
                .media
                .borrow()
                .items
                .borrow()
                .find_for_powerup(Product::Missionpack, slot);
            if let Some(item) = item {
                let shader = match item.icon {
                    None => None,
                    Some(icon) => self.media.borrow().resources.borrow_mut().register_shader(&icon),
                };
                self.host.icons.borrow().tools.draw_pic(rect, &shader);
                return;
            }
        }
    }

    /// Area powerup list.
    fn area_powerup(&self, rect: Rect2d, alignment: i32, special: i32, scale: f32, color: Vec4) {
        let ps = self.ps();
        if ps.stats.get(stat_schema(Product::Missionpack).health) <= 0 {
            return;
        }
        let time = self.state.borrow().time;
        let mut sorted: Vec<(i32, i32)> = Vec::new();
        for slot in 0..16 {
            let expiry = ps.powerups.get(slot);
            let remaining = expiry.wrapping_sub(time);
            if expiry == 0 || remaining <= 0 || remaining >= 999000 {
                continue;
            }
            let position = sorted
                .iter()
                .position(|(_, remaining)| *remaining >= expiry.wrapping_sub(time));
            match position {
                Some(position) => sorted.insert(position, (slot, remaining)),
                None => sorted.push((slot, remaining)),
            }
        }
        let mut x = rect.x;
        let mut y = rect.y;
        let tools = self.host.icons.borrow().tools.clone();
        for (powerup, _) in sorted {
            let item = self
                .media
                .borrow()
                .items
                .borrow()
                .find_for_powerup(Product::Missionpack, powerup);
            let Some(item) = item else {
                continue;
            };
            let remaining = self.ps().powerups.get(powerup).wrapping_sub(time);
            if remaining >= 5000 {
                tools.draw.set_color(None);
            } else {
                let phase = remaining as f32 / 1000.0;
                let alpha = phase - phase.trunc();
                tools.draw.set_color(Some(vec4(alpha, alpha, alpha, alpha)));
            }
            let shader = match item.icon {
                None => None,
                Some(icon) => self.media.borrow().resources.borrow_mut().register_shader(&icon),
            };
            tools.draw_pic(rect2d(x, y, rect.width * 0.75, rect.height), &shader);
            let remaining = self.ps().powerups.get(powerup).wrapping_sub(time);
            self.text(
                rect,
                scale,
                color,
                &format!("{}", remaining / 1000),
                0,
                x + rect.width * 0.75 + 3.0,
                y + rect.height,
            );
            if alignment == 0 {
                y += rect.width + special as f32;
            } else {
                x += rect.width + special as f32;
            }
        }
        tools.draw.set_color(None);
    }

    /// Limited text.
    fn limited(&self, text: &str, x: f32, y: f32, scale: f32, color: Vec4, maximum: f32, limit: i32) -> f32 {
        let draw = self.host.icons.borrow().tools.draw.clone();
        text_paint_limit(
            &draw,
            &self.host.fonts.borrow(),
            &TextPaintOptions {
                x,
                y,
                scale,
                color,
                text: text.to_string(),
                adjust: 0.0,
                limit,
                style: 0,
            },
            maximum,
        )
    }

    /// Team info list.
    fn team_info(&self, rect: Rect2d, text_y: f32, scale: f32, color: Vec4) {
        let count = self.state.borrow().num_sorted_team_players.min(8);
        // The source measures these unused maxima before drawing; font/config lookups remain ordered.
        for index in 0..count {
            let number = self
                .state
                .borrow()
                .sorted_team_players
                .get(index as usize)
                .copied()
                .unwrap_or_else(|| {
                    panic!("Invalid owner-draw slot {index}");
                });
            let client = self.client(number);
            let client = client.borrow();
            if client.info_valid && client.team as i32 == self.team() {
                self.width_text(&client.name, scale);
            }
        }
        for index in 1..MAX_LOCATIONS {
            let location = self.host.strings.borrow().config_string(CS_LOCATIONS + index as usize);
            if !location.is_empty() {
                self.width_text(&location, scale);
            }
        }
        let mut y = rect.y;
        for index in 0..count {
            let number = self
                .state
                .borrow()
                .sorted_team_players
                .get(index as usize)
                .copied()
                .unwrap_or_else(|| {
                    panic!("Invalid owner-draw slot {index}");
                });
            let client_handle = self.client(number);
            let client = client_handle.borrow();
            if !client.info_valid || client.team as i32 != self.team() {
                continue;
            }
            let mut x = (rect.x + 1.0).trunc() as i32;
            for slot in 0..=Powerup::NumPowerups as i32 {
                if client.powerups & (1 << slot) == 0 {
                    continue;
                }
                let item = self
                    .media
                    .borrow()
                    .items
                    .borrow()
                    .find_for_powerup(Product::Missionpack, slot);
                if let Some(item) = item {
                    let shader = match item.icon {
                        None => None,
                        Some(icon) => self.media.borrow().resources.borrow_mut().register_shader(&icon),
                    };
                    self.host
                        .icons
                        .borrow()
                        .tools
                        .draw_pic(rect2d(x as f32, y, 12.0, 12.0), &shader);
                    x += 12;
                }
            }
            x = (rect.x + 38.0).trunc() as i32;
            let tools = self.host.icons.borrow().tools.clone();
            tools
                .draw
                .set_color(Some(get_color_for_health(client.health, client.armor)));
            let heart = tools.media.borrow().graphics.heart_shader.clone();
            tools.draw_pic(rect2d(x as f32, y + 1.0, 10.0, 10.0), &heart);
            x += 13;
            tools.draw.set_color(None);
            let (order_pending, order_time, current_order, time) = {
                let cgs = self.static_state.borrow();
                (
                    cgs.order_pending,
                    cgs.order_time,
                    cgs.current_order,
                    self.state.borrow().time,
                )
            };
            let handle = if order_pending && time > order_time.wrapping_sub(2500) && (time >> 9) & 1 != 0 {
                None
            } else {
                self.status_handle(if order_pending { current_order } else { client.team_task })
            };
            if let Some(handle) = handle {
                tools.draw_pic(rect2d(x as f32, y, 12.0, 12.0), &Some(handle));
            }
            x += 13;
            let left_over = rect.width - x as f32;
            let max = x as f32 + left_over / 3.0;
            drop(client);
            let name = client_handle.borrow().name.clone();
            self.limited(&name, x as f32, y + text_y, scale, color, max, 0);
            let location = self.location(client_handle.borrow().location);
            x = (x as f32 + left_over / 3.0 + 2.0).trunc() as i32;
            self.limited(&location, x as f32, y + text_y, scale, color, rect.width - 4.0, 0);
            y += text_y + 2.0;
            if y + text_y + 2.0 > rect.y + rect.height {
                break;
            }
        }
    }

    /// Spectator scroller.
    fn spectators(&self, rect: Rect2d, scale: f32, color: Vec4) {
        if self.state.borrow().spectator_len == 0 {
            return;
        }
        if self.state.borrow().spectator_width == -1 {
            self.state.borrow_mut().spectator_width = 0;
            self.state.borrow_mut().spectator_paint_x = (rect.x + 1.0).trunc() as i32;
            self.state.borrow_mut().spectator_paint_x2 = -1;
        }
        if self.state.borrow().spectator_offset > self.state.borrow().spectator_len {
            self.state.borrow_mut().spectator_offset = 0;
            self.state.borrow_mut().spectator_paint_x = (rect.x + 1.0).trunc() as i32;
            self.state.borrow_mut().spectator_paint_x2 = -1;
        }
        let (time, spectator_time) = {
            let state = self.state.borrow();
            (state.time, state.spectator_time)
        };
        if time > spectator_time {
            self.state.borrow_mut().spectator_time = time.wrapping_add(10);
            if self.state.borrow().spectator_paint_x as f32 <= rect.x + 2.0 {
                let (offset, len) = {
                    let state = self.state.borrow();
                    (state.spectator_offset, state.spectator_len)
                };
                if offset < len {
                    let rest: String = self
                        .state
                        .borrow()
                        .spectator_list
                        .chars()
                        .skip(offset as usize)
                        .collect();
                    let advance = text_width(&self.host.fonts.borrow(), &rest, scale, 1) - 1;
                    self.state.borrow_mut().spectator_paint_x += advance;
                    self.state.borrow_mut().spectator_offset += 1;
                } else {
                    self.state.borrow_mut().spectator_offset = 0;
                    let paint2 = self.state.borrow().spectator_paint_x2;
                    self.state.borrow_mut().spectator_paint_x = if paint2 >= 0 {
                        paint2
                    } else {
                        (rect.x + rect.width - 2.0).trunc() as i32
                    };
                    self.state.borrow_mut().spectator_paint_x2 = -1;
                }
            } else {
                self.state.borrow_mut().spectator_paint_x -= 1;
                if self.state.borrow().spectator_paint_x2 >= 0 {
                    self.state.borrow_mut().spectator_paint_x2 -= 1;
                }
            }
        }
        let maximum = rect.x + rect.width - 2.0;
        let baseline = rect.y + rect.height - 3.0;
        let (offset, paint_x, paint_x2) = {
            let state = self.state.borrow();
            (
                state.spectator_offset,
                state.spectator_paint_x,
                state.spectator_paint_x2,
            )
        };
        let rest: String = self
            .state
            .borrow()
            .spectator_list
            .chars()
            .skip(offset as usize)
            .collect();
        let max = self.limited(&rest, paint_x as f32, baseline, scale, color, maximum, 0);
        if paint_x2 >= 0 {
            let list = self.state.borrow().spectator_list.clone();
            self.limited(&list, paint_x2 as f32, baseline, scale, color, maximum, offset);
        }
        if offset != 0 && max > 0.0 {
            if self.state.borrow().spectator_paint_x2 == -1 {
                self.state.borrow_mut().spectator_paint_x2 = maximum.trunc() as i32;
            }
        } else {
            self.state.borrow_mut().spectator_paint_x2 = -1;
        }
    }

    /// Medal row.
    fn medal(&self, id: i32, rect: Rect2d, scale: f32, input_color: Vec4, picture: &Option<Picture>) {
        let selected = self.state.borrow().selected_score;
        let score = self
            .state
            .borrow()
            .scores
            .get(selected as usize)
            .copied()
            .unwrap_or_else(|| {
                panic!("Invalid owner-draw slot {selected}");
            });
        let mut value = 0.0f32;
        let mut text: Option<String> = None;
        let mut color = Vec4 { w: 0.25, ..input_color };
        match MissionOwnerDrawId::from_i32(id) {
            Some(MissionOwnerDrawId::Accuracy) => value = score.accuracy,
            Some(MissionOwnerDrawId::Assists) => value = score.assist_count as f32,
            Some(MissionOwnerDrawId::Defend) => value = score.defend_count as f32,
            Some(MissionOwnerDrawId::Excellent) => value = score.excellent_count as f32,
            Some(MissionOwnerDrawId::Impressive) => value = score.impressive_count as f32,
            Some(MissionOwnerDrawId::Perfect) => value = score.perfect as f32,
            Some(MissionOwnerDrawId::Gauntlet) => value = score.guantlet_count as f32,
            Some(MissionOwnerDrawId::Captures) => value = score.captures as f32,
            _ => {}
        }
        if value > 0.0 {
            if MissionOwnerDrawId::from_i32(id) == Some(MissionOwnerDrawId::Perfect) {
                color.w = 1.0;
                text = Some("Wow".to_string());
            } else if MissionOwnerDrawId::from_i32(id) == Some(MissionOwnerDrawId::Accuracy) {
                text = Some(format!("{}%", value.trunc() as i32));
                if value > 50.0 {
                    color.w = 1.0;
                }
            } else {
                text = Some(format!("{}", value.trunc() as i32));
                color.w = 1.0;
            }
        }
        let tools = self.host.icons.borrow().tools.clone();
        tools.draw.set_color(Some(color));
        let picture = picture
            .as_ref()
            .copied()
            .unwrap_or_else(|| tools.media.borrow().resources.borrow().picture(&None));
        tools.draw.stretch_pic(
            rect,
            TextureRect {
                s: 0.0,
                t: 0.0,
                s2: 1.0,
                t2: 1.0,
            },
            picture,
        );
        if let Some(text) = text {
            color.w = 1.0;
            let width = self.width_text(&text, scale);
            self.text(
                rect,
                scale,
                color,
                &text,
                0,
                rect.x + (rect.width - width) / 2.0,
                rect.y + rect.height + 10.0,
            );
        }
        tools.draw.set_color(None);
    }

    /// Paint an owner-draw item (`paint`).
    pub fn paint(&self, request: &mut OwnerDrawPaintRequest) {
        if !request.draw.shares_queue(&self.host.icons.borrow().tools.draw) {
            panic!("Mission owner drawing must share the ordered HUD recorder");
        }
        if self.cvar("cg_drawStatus") == 0 {
            return;
        }
        let force_2d = request.owner_draw_flags & owner_draw_flags::SHOW_2DONLY != 0;
        let id = MissionOwnerDrawId::from_i32(request.owner_draw);
        match id {
            Some(MissionOwnerDrawId::PlayerArmorIcon) => self.armor_icon(request.rect, force_2d),
            Some(MissionOwnerDrawId::PlayerArmorIcon2d) => self.armor_icon(request.rect, true),
            Some(MissionOwnerDrawId::PlayerAmmoIcon) => self.ammo_icon(request.rect, force_2d),
            Some(MissionOwnerDrawId::PlayerAmmoIcon2d) => self.ammo_icon(request.rect, true),
            Some(MissionOwnerDrawId::PlayerAmmoValue) => {
                if self.host.weapon_hud.is_none()
                    && self.state.borrow().entity_at(self.ps().client_num).current_state.weapon != 0
                    && self.raw_value(request.owner_draw) > -1
                {
                    let value = self.raw_value(request.owner_draw);
                    self.number(
                        request.rect,
                        request.text_scale,
                        request.color,
                        value,
                        &request.background,
                        request.text_style,
                    );
                }
            }
            Some(
                MissionOwnerDrawId::PlayerArmorValue
                | MissionOwnerDrawId::PlayerHealth
                | MissionOwnerDrawId::PlayerScore
                | MissionOwnerDrawId::SelectedPlayerHealth,
            ) => {
                let value = self.raw_value(request.owner_draw);
                self.number(
                    request.rect,
                    request.text_scale,
                    request.color,
                    value,
                    &request.background,
                    request.text_style,
                );
            }
            Some(MissionOwnerDrawId::SelectedPlayerArmor) => {
                if self.selected().borrow().armor > 0 {
                    let armor = self.selected().borrow().armor;
                    self.number(
                        request.rect,
                        request.text_scale,
                        request.color,
                        armor,
                        &request.background,
                        request.text_style,
                    );
                }
            }
            Some(MissionOwnerDrawId::SelectedPlayerHead) | Some(MissionOwnerDrawId::VoiceHead) => {
                let index = if id == Some(MissionOwnerDrawId::VoiceHead) {
                    self.static_state.borrow().current_voice_client
                } else {
                    self.selected_index()
                };
                self.host
                    .icons
                    .borrow()
                    .draw_head(request.rect, index, vec3(0.0, 180.0, 0.0));
            }
            Some(MissionOwnerDrawId::SelectedPlayerName) | Some(MissionOwnerDrawId::VoiceName) => {
                let index = if id == Some(MissionOwnerDrawId::VoiceName) {
                    self.static_state.borrow().current_voice_client
                } else {
                    self.selected_index()
                };
                let name = self.client(index).borrow().name.clone();
                let (x, y) = (request.rect.x, request.rect.y + request.rect.height);
                self.text(
                    request.rect,
                    request.text_scale,
                    request.color,
                    &name,
                    request.text_style,
                    x,
                    y,
                );
            }
            Some(MissionOwnerDrawId::SelectedPlayerLocation) => {
                let location = self.location(self.selected().borrow().location);
                let (x, y) = (request.rect.x, request.rect.y + request.rect.height);
                self.text(
                    request.rect,
                    request.text_scale,
                    request.color,
                    &location,
                    request.text_style,
                    x,
                    y,
                );
            }
            Some(MissionOwnerDrawId::PlayerLocation) => {
                let location = self.location(self.client(self.ps().client_num).borrow().location);
                let (x, y) = (request.rect.x, request.rect.y + request.rect.height);
                self.text(
                    request.rect,
                    request.text_scale,
                    request.color,
                    &location,
                    request.text_style,
                    x,
                    y,
                );
            }
            Some(MissionOwnerDrawId::SelectedPlayerStatus) => self.selected_status(request.rect),
            Some(MissionOwnerDrawId::PlayerStatus) => {
                let task = self.client(self.ps().client_num).borrow().team_task;
                let handle = self.status_handle(task);
                self.host.icons.borrow().tools.draw_pic(request.rect, &handle);
            }
            Some(MissionOwnerDrawId::SelectedPlayerWeapon) => {
                let weapon = self.selected().borrow().cur_weapon;
                let icon = self.media.borrow().weapon_registry.borrow().weapon(weapon).weapon_icon;
                let defer = self.media.borrow().graphics.defer_shader.clone();
                self.host.icons.borrow().tools.draw_pic(request.rect, &icon.or(defer));
            }
            Some(MissionOwnerDrawId::SelectedPlayerPowerup) => self.selected_powerup(request.rect),
            Some(MissionOwnerDrawId::PlayerHead) => self.player_head(request.rect),
            Some(MissionOwnerDrawId::PlayerItem) => self.item(request.rect, false),
            Some(MissionOwnerDrawId::CtfPowerup) => self.item(request.rect, true),
            Some(MissionOwnerDrawId::RedScore) | Some(MissionOwnerDrawId::BlueScore) => {
                let value = if id == Some(MissionOwnerDrawId::RedScore) {
                    self.static_state.borrow().scores1
                } else {
                    self.static_state.borrow().scores2
                };
                let text = if value == SCORE_NOT_PRESENT {
                    "-".to_string()
                } else {
                    format!("{value}")
                };
                let width = self.width_text(&text, request.text_scale);
                self.text(
                    request.rect,
                    request.text_scale,
                    request.color,
                    &text,
                    request.text_style,
                    request.rect.x + request.rect.width - width,
                    request.rect.y + request.rect.height,
                );
            }
            Some(MissionOwnerDrawId::RedName) | Some(MissionOwnerDrawId::BlueName) => {
                let name = if id == Some(MissionOwnerDrawId::RedName) {
                    "g_redteam"
                } else {
                    "g_blueteam"
                };
                let value = self.host.configuration.borrow().read_vm_cvar(name).value;
                let (x, y) = (request.rect.x, request.rect.y + request.rect.height);
                self.text(
                    request.rect,
                    request.text_scale,
                    request.color,
                    &value,
                    request.text_style,
                    x,
                    y,
                );
            }
            Some(MissionOwnerDrawId::BlueFlagHead) => self.flag_head(request.rect, true),
            Some(MissionOwnerDrawId::RedFlagHead) => self.flag_head(request.rect, false),
            Some(MissionOwnerDrawId::BlueFlagStatus) => self.flag_status(request.rect, true, &request.background),
            Some(MissionOwnerDrawId::RedFlagStatus) => self.flag_status(request.rect, false, &request.background),
            Some(MissionOwnerDrawId::BlueFlagName) | Some(MissionOwnerDrawId::RedFlagName) => {
                if let Some(carrier) = self.flag_carrier(id == Some(MissionOwnerDrawId::BlueFlagName)) {
                    let name = self.client(carrier).borrow().name.clone();
                    let (x, y) = (request.rect.x, request.rect.y + request.rect.height);
                    self.text(
                        request.rect,
                        request.text_scale,
                        request.color,
                        &name,
                        request.text_style,
                        x,
                        y,
                    );
                }
            }
            Some(MissionOwnerDrawId::HarvesterSkulls) => {
                self.skulls(
                    request.rect,
                    request.text_scale,
                    request.color,
                    false,
                    request.text_style,
                );
            }
            Some(MissionOwnerDrawId::HarvesterSkulls2d) => {
                self.skulls(
                    request.rect,
                    request.text_scale,
                    request.color,
                    true,
                    request.text_style,
                );
            }
            Some(MissionOwnerDrawId::OneFlagStatus) => self.one_flag_status(request.rect),
            Some(MissionOwnerDrawId::TeamColor) => {
                self.host
                    .icons
                    .borrow()
                    .draw_team_background(request.rect, request.color.w, self.team());
            }
            Some(MissionOwnerDrawId::AreaPowerup) => {
                self.area_powerup(
                    request.rect,
                    request.alignment,
                    request.special,
                    request.text_scale,
                    request.color,
                );
            }
            Some(MissionOwnerDrawId::PlayerHasFlag) => self.player_has_flag(request.rect, false),
            Some(MissionOwnerDrawId::PlayerHasFlag2d) => self.player_has_flag(request.rect, true),
            Some(MissionOwnerDrawId::AreaSystemChat) => {
                let chat = (self.host.chat)();
                self.text(
                    request.rect,
                    request.text_scale,
                    request.color,
                    &chat.system,
                    0,
                    request.rect.x,
                    request.rect.y + request.rect.height,
                );
            }
            Some(MissionOwnerDrawId::AreaTeamChat) => {
                let chat = (self.host.chat)();
                self.text(
                    request.rect,
                    request.text_scale,
                    request.color,
                    &chat.team1,
                    0,
                    request.rect.x,
                    request.rect.y + request.rect.height,
                );
            }
            Some(MissionOwnerDrawId::AreaChat) => {
                let chat = (self.host.chat)();
                self.text(
                    request.rect,
                    request.text_scale,
                    request.color,
                    &chat.team2,
                    0,
                    request.rect.x,
                    request.rect.y + request.rect.height,
                );
            }
            Some(MissionOwnerDrawId::GameType) => {
                let text = self.game_type_text();
                let (x, y) = (request.rect.x, request.rect.y + request.rect.height);
                self.text(
                    request.rect,
                    request.text_scale,
                    request.color,
                    &text,
                    request.text_style,
                    x,
                    y,
                );
            }
            Some(MissionOwnerDrawId::GameStatus) => {
                let text = self.game_status_text();
                let (x, y) = (request.rect.x, request.rect.y + request.rect.height);
                self.text(
                    request.rect,
                    request.text_scale,
                    request.color,
                    &text,
                    request.text_style,
                    x,
                    y,
                );
            }
            Some(MissionOwnerDrawId::Killer) => {
                if !self.state.borrow().killer_name.is_empty() {
                    let text = self.killer_text();
                    let width = self.width_text(&text, request.text_scale);
                    self.text(
                        request.rect,
                        request.text_scale,
                        request.color,
                        &text,
                        request.text_style,
                        (request.rect.x + request.rect.width / 2.0).trunc() - (width / 2.0).trunc(),
                        request.rect.y + request.rect.height,
                    );
                }
            }
            Some(
                MissionOwnerDrawId::Accuracy
                | MissionOwnerDrawId::Assists
                | MissionOwnerDrawId::Defend
                | MissionOwnerDrawId::Excellent
                | MissionOwnerDrawId::Impressive
                | MissionOwnerDrawId::Perfect
                | MissionOwnerDrawId::Gauntlet
                | MissionOwnerDrawId::Captures,
            ) => {
                self.medal(
                    request.owner_draw,
                    request.rect,
                    request.text_scale,
                    request.color,
                    &request.background,
                );
            }
            Some(MissionOwnerDrawId::Spectators) => {
                self.spectators(request.rect, request.text_scale, request.color);
            }
            Some(MissionOwnerDrawId::TeamInfo) => {
                if self.cvar("cg_currentSelectedPlayer") == self.state.borrow().num_sorted_team_players {
                    self.team_info(request.rect, request.text_y, request.text_scale, request.color);
                }
            }
            Some(MissionOwnerDrawId::CapFragLimit) => {
                let value = if self.static_state.borrow().game_type >= GameType::Ctf {
                    self.static_state.borrow().capturelimit
                } else {
                    self.static_state.borrow().fraglimit
                };
                let text = format!("{value:>2}");
                self.text(
                    request.rect,
                    request.text_scale,
                    request.color,
                    &text,
                    request.text_style,
                    request.rect.x,
                    request.rect.y,
                );
            }
            Some(MissionOwnerDrawId::FirstPlace) | Some(MissionOwnerDrawId::SecondPlace) => {
                let value = if id == Some(MissionOwnerDrawId::FirstPlace) {
                    self.static_state.borrow().scores1
                } else {
                    self.static_state.borrow().scores2
                };
                if value != SCORE_NOT_PRESENT {
                    let text = format!("{value:>2}");
                    self.text(
                        request.rect,
                        request.text_scale,
                        request.color,
                        &text,
                        request.text_style,
                        request.rect.x,
                        request.rect.y,
                    );
                }
            }
            None => {}
        }
    }
}

/// Mission HUD host services (`MissionHudHost`).
pub trait MissionHudHost {
    /// Weapon HUD report.
    fn weapon_hud(&self) -> Option<WeaponHudReport>;
    /// Sound assets.
    fn assets(&self) -> Shared<dyn SoundAssetReader>;
    /// Font registry.
    fn font_registry(&self) -> Shared<dyn FontRegistry>;
    /// Draw icons.
    fn icons(&self) -> Shared<ClientDrawIcons>;
    /// Configuration.
    fn configuration(&self) -> Shared<ClientConfiguration>;
    /// Cvar registry.
    fn cvars(&self) -> Shared<CvarRegistry>;
    /// Command buffer.
    fn commands(&self) -> Shared<dyn HudCommandBuffer>;
    /// Client store.
    fn clients(&self) -> Shared<dyn ClientInfoStore>;
    /// Random.
    fn random(&self) -> Shared<GameRandom>;
    /// Cinematics.
    fn cinematics(&self) -> Shared<dyn CinematicService>;
    /// Model painter.
    fn model_painter(&self) -> EngineUiModelPainter;
    /// Menu audio.
    fn audio(&self) -> Shared<dyn HudMenuAudio>;
    /// Configstring.
    fn config_string(&self, index: usize) -> String;
    /// Reset a player entity.
    fn reset_player_entity(&mut self, entity: &mut ClientEntity);
    /// Print.
    fn print(&mut self, text: &str);
    /// Milliseconds.
    fn milliseconds(&self) -> i32;
    /// Set the key catcher.
    fn set_key_catcher(&mut self, mask: i32);
    /// Initialize UI strings.
    fn initialize_ui_strings(&mut self);
    /// Load menu definitions.
    fn load_menu_definitions(
        &mut self,
        ctx: &mut dyn MenuLoadContext,
        set_path: &str,
        root: &MenuSource,
    ) -> MenuDefinitions;
    /// Create a menu runtime.
    fn create_menu_runtime(&mut self, seed: MenuRuntimeSeed) -> Box<dyn MenuRuntime>;
}

/// Team order (`TeamOrder`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct TeamOrder {
    /// Personal voice.
    personal: &'static str,
    /// Team voice.
    team: &'static str,
    /// Button command.
    button: Option<&'static str>,
}

/// Team orders (`ORDERS`).
const ORDERS: [TeamOrder; 7] = [
    TeamOrder {
        personal: "onoffense",
        team: "offense",
        button: Some("+button7; wait; -button7"),
    },
    TeamOrder {
        personal: "ondefense",
        team: "defend",
        button: Some("+button8; wait; -button8"),
    },
    TeamOrder {
        personal: "onpatrol",
        team: "patrol",
        button: Some("+button9; wait; -button9"),
    },
    TeamOrder {
        personal: "onfollow",
        team: "followme",
        button: Some("+button10; wait; -button10"),
    },
    TeamOrder {
        personal: "ongetflag",
        team: "returnflag",
        button: None,
    },
    TeamOrder {
        personal: "onfollowcarrier",
        team: "followflagcarrier",
        button: None,
    },
    TeamOrder {
        personal: "oncamping",
        team: "camp",
        button: None,
    },
];

/// Mission score feeder (`MissionScoreFeeder`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum MissionScoreFeeder {
    /// Red.
    Red = 5,
    /// Blue.
    Blue = 6,
    /// Scoreboard.
    Scoreboard = 11,
}

/// Mouse 2 key (`KeyCode.Mouse2`).
const KEY_MOUSE2: i32 = 179;

/// Mission HUD source text (exclusive cap).
fn hud_source_text(value: &str, maximum: usize) -> String {
    let cut = match value.find('\0') {
        Some(end) => &value[..end],
        None => value,
    };
    if cut.chars().count() >= maximum {
        panic!("Mission HUD source string exceeds {} bytes", maximum - 1);
    }
    for ch in cut.chars() {
        if ch as u32 > 255 {
            panic!("Mission HUD text requires source byte characters");
        }
    }
    cut.to_string()
}

/// ASCII fold.
fn hud_fold(value: &str) -> String {
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
}

/// Asset key (`assetKey`).
fn asset_key(path: Option<&str>) -> Option<String> {
    path.map(hud_fold)
}

/// Script source from bytes (`scriptSource`).
fn script_source(path: &str, bytes: &[u8]) -> MenuSource {
    let mut text = String::new();
    for byte in bytes {
        if *byte == 0 {
            break;
        }
        text.push(*byte as char);
    }
    MenuSource {
        path: path.to_string(),
        text,
    }
}

/// POSIX dirname.
fn posix_dirname(path: &str) -> &str {
    match path.rfind('/') {
        None => ".",
        Some(0) => "/",
        Some(position) => &path[..position],
    }
}

/// POSIX join + normalize.
fn posix_join(first: &str, second: &str) -> String {
    let absolute = second.starts_with('/') || first == "/";
    let mut parts: Vec<&str> = Vec::new();
    let combined = format!("{first}/{second}");
    for segment in combined.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            segment => parts.push(segment),
        }
    }
    let joined = parts.join("/");
    if absolute {
        format!("/{joined}")
    } else if joined.is_empty() {
        ".".to_string()
    } else {
        joined
    }
}

/// Loaded menus (`LoadedMenus`).
struct LoadedMenus {
    /// Definitions (retained for reloads; reads happen through the runtime).
    #[allow(dead_code)]
    definitions: MenuDefinitions,
    /// Runtime.
    runtime: Box<dyn MenuRuntime>,
}

/// Cached chat strings.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct MissionHudChat {
    /// System.
    system: String,
    /// Team 1.
    team1: String,
    /// Team 2.
    team2: String,
}

/// Team Arena HUD (`MissionHud`).
pub struct MissionHud {
    /// Frame state.
    pub state: Shared<ClientGameState>,
    /// Static state.
    pub static_state: Shared<ClientGameStaticState>,
    /// Media.
    pub media: Shared<ClientMedia>,
    /// Host.
    pub host: Shared<dyn MissionHudHost>,
    /// Owner drawing.
    pub owner_draw: MissionOwnerDraw,
    /// Live fonts.
    pub fonts_handle: Shared<FontSet>,
    /// Chat.
    chat: Shared<MissionHudChat>,
    /// Loaded menus.
    loaded: RefCell<Option<LoadedMenus>>,
    /// Scoreboard.
    scoreboard: RefCell<Option<CapturedMenu>>,
    /// First scoreboard paint.
    scoreboard_first_time: Cell<bool>,
    /// Generation.
    generation: Cell<u64>,
    /// Load generation.
    load_generation: Cell<u64>,
    /// Closed.
    closed: Cell<bool>,
    /// Loading.
    loading: Cell<bool>,
    /// Captured menu.
    captured_menu: RefCell<Option<CapturedMenu>>,
    /// Widget assets.
    widget_assets: RefCell<UiWidgetAssets>,
    /// FX base.
    fx_base: RefCell<Option<SceneShader>>,
    /// FX colors.
    fx_colors: RefCell<[Option<SceneShader>; 7]>,
    /// Pictures by key.
    pictures: RefCell<HashMap<Option<String>, Option<Picture>>>,
    /// Sounds by key.
    sounds: RefCell<HashMap<Option<String>, Option<PcmSound>>>,
    /// Models by key.
    models: RefCell<HashMap<Option<String>, SceneModel>>,
    /// Registered fonts by path and size.
    registered_fonts: RefCell<HashMap<Option<String>, HashMap<i32, Option<RegisteredFont>>>>,
    /// Asset definitions.
    asset_definitions: RefCell<Option<MenuGlobalAssets>>,
}

impl MissionHud {
    /// Assemble a mission HUD.
    pub fn new(
        state: Shared<ClientGameState>,
        static_state: Shared<ClientGameStaticState>,
        media: Shared<ClientMedia>,
        host: Shared<dyn MissionHudHost>,
    ) -> Self {
        if state.borrow().product != Product::Missionpack
            || static_state.borrow().product != Product::Missionpack
            || !same(&media.borrow().static_state, &static_state)
            || !same(&host.borrow().icons().borrow().state, &state)
            || !same(&host.borrow().icons().borrow().tools.media, &media)
        {
            panic!("Mission HUD requires one Team Arena cgame state and media owner");
        }
        let fonts_handle = shared(zero_cgame_fonts());
        let chat = shared(MissionHudChat::default());
        let weapon_report = host.borrow().weapon_hud();
        let weapon_hud: Option<Shared<dyn WeaponHudReader>> = weapon_report.map(|report| {
            let reader: Shared<dyn WeaponHudReader> = shared(FixedWeaponHudReader { report });
            reader
        });
        let selected_state = state.clone();
        let selected_configuration = host.borrow().configuration();
        let selected_player = Rc::new(move || {
            let index = selected_configuration
                .borrow()
                .read_vm_cvar("cg_currentSelectedPlayer")
                .integer_value;
            if index < 0 || index >= selected_state.borrow().num_sorted_team_players {
                selected_configuration
                    .borrow()
                    .set_vm_integer(ClientVmCvarSymbol::CgCurrentSelectedPlayer, 0);
                return selected_configuration
                    .borrow()
                    .read_vm_cvar("cg_currentSelectedPlayer")
                    .integer_value;
            }
            index
        });
        let chat_reader = chat.clone();
        let chat_fn = Rc::new(move || {
            let chat = chat_reader.borrow();
            HudChatText {
                system: chat.system.clone(),
                team1: chat.team1.clone(),
                team2: chat.team2.clone(),
            }
        });
        let owner_draw = MissionOwnerDraw::new(
            state.clone(),
            static_state.clone(),
            media.clone(),
            MissionOwnerDrawHost {
                weapon_hud,
                icons: host.borrow().icons(),
                fonts: fonts_handle.clone(),
                configuration: host.borrow().configuration(),
                random: host.borrow().random(),
                strings: shared(MissionHudStrings(host.clone())),
                selected_player,
                chat: chat_fn,
            },
        );
        let zero = media.borrow().resources.borrow().picture(&None);
        let white = media
            .borrow()
            .resources
            .borrow()
            .picture(&media.borrow().graphics.white_shader);
        Self {
            state,
            static_state,
            media,
            host,
            owner_draw,
            fonts_handle,
            chat,
            loaded: RefCell::new(None),
            scoreboard: RefCell::new(None),
            scoreboard_first_time: Cell::new(true),
            generation: Cell::new(0),
            load_generation: Cell::new(0),
            closed: Cell::new(false),
            loading: Cell::new(false),
            captured_menu: RefCell::new(None),
            widget_assets: RefCell::new(UiWidgetAssets {
                white_shader: white,
                gradient_bar: zero,
                scroll_bar: zero,
                scroll_bar_arrow_down: zero,
                scroll_bar_arrow_up: zero,
                scroll_bar_arrow_left: zero,
                scroll_bar_arrow_right: zero,
                scroll_bar_thumb: zero,
                slider_bar: zero,
                slider_thumb: zero,
            }),
            fx_base: RefCell::new(None),
            fx_colors: RefCell::new(std::array::from_fn(|_| None)),
            pictures: RefCell::new(HashMap::new()),
            sounds: RefCell::new(HashMap::new()),
            models: RefCell::new(HashMap::new()),
            registered_fonts: RefCell::new(HashMap::new()),
            asset_definitions: RefCell::new(None),
        }
    }

    /// Require an open HUD.
    fn open(&self) {
        if self.closed.get() {
            panic!("Mission HUD is disposed");
        }
    }

    /// Require a current registration generation.
    fn registration_current(&self, generation: u64) {
        self.open();
        if generation != self.generation.get() {
            panic!("Mission HUD media registration belongs to a retired lifecycle");
        }
    }

    /// Cached assets (`cachedAssets`).
    pub fn cached_assets(&self) -> (Option<SceneShader>, [Option<SceneShader>; 7], UiWidgetAssets) {
        (
            self.fx_base.borrow().clone(),
            self.fx_colors.borrow().clone(),
            *self.widget_assets.borrow(),
        )
    }

    /// Menu state (`menuState`).
    pub fn menu_state(&self) -> Option<MenuSnapshot> {
        self.loaded.borrow().as_ref().map(|loaded| loaded.runtime.snapshot())
    }

    /// Cache widget assets (`assetCache`).
    pub fn asset_cache(&self) {
        self.open();
        let generation = self.generation.get();
        let shader = |path: &str| {
            let registered = self
                .media
                .borrow()
                .resources
                .borrow_mut()
                .register_shader_no_mip(Some(path));
            self.open();
            if generation != self.generation.get() {
                panic!("Mission HUD asset registration belongs to a retired lifecycle");
            }
            registered
        };
        let gradient_bar = self
            .media
            .borrow()
            .resources
            .borrow()
            .picture(&shader("ui/assets/gradientbar2.tga"));
        *self.fx_base.borrow_mut() = shader("menu/art/fx_base");
        for (index, name) in ["red", "yel", "grn", "teal", "blue", "cyan", "white"]
            .iter()
            .enumerate()
        {
            self.fx_colors.borrow_mut()[index] = shader(&format!("menu/art/fx_{name}"));
        }
        let picture = |path: &str| self.media.borrow().resources.borrow().picture(&shader(path));
        let white = self
            .media
            .borrow()
            .resources
            .borrow()
            .picture(&self.media.borrow().graphics.white_shader);
        self.open();
        *self.widget_assets.borrow_mut() = UiWidgetAssets {
            white_shader: white,
            gradient_bar,
            scroll_bar: picture("ui/assets/scrollbar.tga"),
            scroll_bar_arrow_down: picture("ui/assets/scrollbar_arrow_dwn_a.tga"),
            scroll_bar_arrow_up: picture("ui/assets/scrollbar_arrow_up_a.tga"),
            scroll_bar_arrow_left: picture("ui/assets/scrollbar_arrow_left.tga"),
            scroll_bar_arrow_right: picture("ui/assets/scrollbar_arrow_right.tga"),
            scroll_bar_thumb: picture("ui/assets/scrollbar_thumb.tga"),
            slider_bar: picture("ui/assets/slider2.tga"),
            slider_thumb: picture("ui/assets/sliderbutt_1.tga"),
        };
    }

    /// Whether the other team has the flag.
    #[must_use]
    pub fn other_team_has_flag(&self) -> bool {
        self.owner_draw.other_team_has_flag()
    }

    /// Whether your team has the flag.
    #[must_use]
    pub fn your_team_has_flag(&self) -> bool {
        self.owner_draw.your_team_has_flag()
    }

    /// Register a picture.
    fn register_picture(&self, path: Option<&str>) -> Option<Picture> {
        self.open();
        let generation = self.generation.get();
        let shader = self.media.borrow().resources.borrow_mut().register_shader_no_mip(path);
        self.registration_current(generation);
        let picture = shader.map(|shader| self.media.borrow().resources.borrow().picture(&Some(shader)));
        self.pictures.borrow_mut().insert(asset_key(path), picture);
        picture
    }

    /// Register a sound.
    fn register_sound(&self, path: Option<&str>) -> Option<PcmSound> {
        self.open();
        let generation = self.generation.get();
        let sound = self.media.borrow().sound_bank.borrow_mut().register_sound(path, false);
        self.registration_current(generation);
        self.sounds.borrow_mut().insert(asset_key(path), sound.clone());
        sound
    }

    /// Register a model.
    fn register_model(&self, path: Option<&str>) -> SceneModel {
        self.open();
        let generation = self.generation.get();
        let model = self.media.borrow().resources.borrow_mut().register_model(path);
        self.registration_current(generation);
        self.models.borrow_mut().insert(asset_key(path), model.clone());
        model
    }

    /// Register a font.
    fn register_font(&self, path: Option<&str>, point_size: i32) {
        self.open();
        let generation = self.generation.get();
        let font = self
            .host
            .borrow()
            .font_registry()
            .borrow_mut()
            .register_font(path, point_size);
        self.registration_current(generation);
        self.registered_fonts
            .borrow_mut()
            .entry(asset_key(path))
            .or_default()
            .insert(point_size, font);
    }

    /// Fetch a registered font.
    fn font(&self, reference: &FontReference) -> Option<RegisteredFont> {
        match self
            .registered_fonts
            .borrow()
            .get(&asset_key(reference.path.as_deref()))
            .and_then(|sizes| sizes.get(&reference.point_size))
        {
            Some(font) => font.clone(),
            None => panic!("HUD font declaration has no completed registration"),
        }
    }

    /// Read a script source.
    fn source(&self, path: &str) -> Option<MenuSource> {
        if !self.host.borrow().assets().borrow().has(path) {
            return None;
        }
        Some(script_source(
            path,
            &self.host.borrow().assets().borrow().read_sync(path),
        ))
    }

    /// Read a menu buffer (`getMenuBuffer`).
    pub fn get_menu_buffer(&self, filename: &str) -> Option<String> {
        self.open();
        if !self.host.borrow().assets().borrow().has(filename) {
            self.host
                .borrow_mut()
                .print(&format!("^1menu file not found: {filename}, using default\n"));
            return None;
        }
        let data = self.host.borrow().assets().borrow().read_sync(filename);
        if data.len() >= 32768 {
            self.host.borrow_mut().print(&format!(
                "^1menu file too large: {filename} is {}, max allowed is 32768",
                data.len()
            ));
            return None;
        }
        Some(script_source(filename, &data).text)
    }

    /// Reset strings (`resetStrings`).
    pub fn reset_strings(&self) {
        self.open();
        self.generation.set(self.generation.get() + 1);
        self.host.borrow_mut().initialize_ui_strings();
        if let Some(loaded) = self.loaded.borrow_mut().as_mut() {
            loaded.runtime.reset_definitions(MenuResetScope::Strings);
        }
    }

    /// Reset menus (`resetMenus`).
    pub fn reset_menus(&self) {
        self.open();
        self.generation.set(self.generation.get() + 1);
        if let Some(loaded) = self.loaded.borrow_mut().as_mut() {
            loaded.runtime.reset_definitions(MenuResetScope::Menus);
        }
    }

    /// Load the HUD menu (`loadHudMenu`).
    pub fn load_hud_menu(&self) {
        self.reset_menus();
        let requested: String = self
            .host
            .borrow()
            .cvars()
            .borrow()
            .get("cg_hudFiles")
            .map(|snapshot| snapshot.value.chars().take(1023).collect())
            .unwrap_or_default();
        self.load_menus(if requested.is_empty() { "ui/hud.txt" } else { &requested });
    }

    /// Load menus (`loadMenus`).
    pub fn load_menus(&self, path: &str) {
        self.open();
        if self.loading.get() {
            panic!("Mission HUD menu registration is already active");
        }
        let started = self.host.borrow().milliseconds();
        if !self.host.borrow().assets().borrow().has(path) {
            panic!("^3menu file not found: {path}, using default\n");
        }
        struct LoadingGuard<'a> {
            hud: &'a MissionHud,
        }
        impl Drop for LoadingGuard<'_> {
            fn drop(&mut self) {
                self.hud.loading.set(false);
            }
        }
        let _guard = LoadingGuard { hud: self };
        let bytes = self.host.borrow().assets().borrow().read_sync(path);
        self.open();
        if bytes.len() >= 4096 {
            panic!("^1menu file too large: {path} is {}, max allowed is 4096", bytes.len());
        }
        self.loading.set(true);
        let root_source = script_source(path, &bytes);
        self.reset_menus();
        self.load_generation.set(self.generation.get());
        // Random draws, source resolution, registration, and asset publication
        // flow through this HUD as the load context.
        let definitions =
            self.host
                .borrow_mut()
                .load_menu_definitions(&mut MissionHudLoadContext { hud: self }, path, &root_source);
        self.open();
        if self.load_generation.get() != self.generation.get() {
            panic!("Mission HUD menu load belongs to a retired lifecycle");
        }
        *self.asset_definitions.borrow_mut() = Some(definitions.assets.clone());
        if let Some(gradient) = &definitions.assets.gradient_bar {
            let mut widgets = self.widget_assets.borrow_mut();
            widgets.gradient_bar = self
                .pictures
                .borrow()
                .get(&asset_key(gradient.path.as_deref()))
                .copied()
                .flatten()
                .unwrap_or_else(|| self.media.borrow().resources.borrow().picture(&None));
        }
        if self.loaded.borrow().is_some() {
            {
                let mut loaded = self.loaded.borrow_mut();
                let loaded = loaded.as_mut().expect("Mission HUD menus are not loaded");
                loaded.runtime.reload_definitions(&definitions);
                loaded.definitions = definitions.clone();
            }
            self.open();
            if self.load_generation.get() != self.generation.get() {
                panic!("Mission HUD menu load belongs to a retired lifecycle");
            }
        } else {
            let seed = MenuRuntimeSeed {
                definitions: definitions.clone(),
                cvars: self.host.borrow().cvars(),
                widget_assets: *self.widget_assets.borrow(),
                zero_picture: self.media.borrow().resources.borrow().picture(&None),
                fonts: self.fonts_handle.borrow().clone(),
            };
            let runtime = self.host.borrow_mut().create_menu_runtime(seed);
            self.open();
            if self.load_generation.get() != self.generation.get() {
                panic!("Mission HUD menu load belongs to a retired lifecycle");
            }
            *self.loaded.borrow_mut() = Some(LoadedMenus { definitions, runtime });
        }
        let elapsed = self.host.borrow().milliseconds().wrapping_sub(started);
        self.host
            .borrow_mut()
            .print(&format!("UI menu load time = {elapsed} milli seconds\n"));
    }

    /// Dispose (`dispose`).
    pub fn dispose(&self) {
        if self.closed.get() {
            return;
        }
        self.generation.set(self.generation.get() + 1);
        self.closed.set(true);
        if let Some(mut loaded) = self.loaded.borrow_mut().take() {
            loaded.runtime.retire();
        }
        *self.captured_menu.borrow_mut() = None;
    }

    /// Current player state.
    fn snapshot(&self) -> PlayerState {
        self.state
            .borrow()
            .snap
            .clone()
            .unwrap_or_else(|| {
                panic!("Mission HUD operation requires cg.snap");
            })
            .player_state
    }

    /// Selected index.
    fn selected(&self) -> i32 {
        self.host
            .borrow()
            .configuration()
            .borrow()
            .read_vm_cvar("cg_currentSelectedPlayer")
            .integer_value
    }

    /// Write the selected index.
    fn set_selected(&self, value: i32) {
        self.host
            .borrow()
            .configuration()
            .borrow()
            .set_vm_integer(ClientVmCvarSymbol::CgCurrentSelectedPlayer, value);
    }

    /// Set a cvar.
    fn set_cvar(&self, name: &str, value: &str) {
        let _ = self.host.borrow().cvars().borrow_mut().set(name, value, true);
    }

    /// Initialize team chat (`initTeamChat`).
    pub fn init_team_chat(&self) {
        *self.chat.borrow_mut() = MissionHudChat::default();
    }

    /// Set a print string (`setPrintString`).
    pub fn set_print_string(&self, kind: i32, value: &str) {
        let text = hud_source_text(value, 256);
        if kind == 0 {
            self.chat.borrow_mut().system = text;
        } else {
            let mut chat = self.chat.borrow_mut();
            chat.team2 = std::mem::take(&mut chat.team1);
            chat.team1 = text;
        }
    }

    /// Chat strings (`chat`).
    #[must_use]
    pub fn chat(&self) -> HudChatText {
        let chat = self.chat.borrow();
        HudChatText {
            system: chat.system.clone(),
            team1: chat.team1.clone(),
            team2: chat.team2.clone(),
        }
    }

    /// Append a command.
    fn append(&self, text: &str) {
        self.host.borrow().commands().borrow_mut().append(text);
    }

    /// Check a pending order (`checkOrderPending`).
    pub fn check_order_pending(&self) {
        let cgs = self.static_state.borrow();
        if cgs.game_type < GameType::Ctf || !cgs.order_pending {
            return;
        }
        let current = cgs.current_order;
        drop(cgs);
        let order = ORDERS.get((current - 1) as usize);
        let selected = self.selected();
        if selected == self.state.borrow().num_sorted_team_players {
            let Some(order) = order else {
                panic!("CG_CheckOrderPending: Everyone order has no source voice command");
            };
            self.append(&format!("cmd vsay_team {}\n", order.team));
        } else {
            let client = self
                .state
                .borrow()
                .sorted_team_players
                .get(selected as usize)
                .copied()
                .unwrap_or_else(|| {
                    panic!("Mission HUD source index {selected} outside 8");
                });
            if client == self.snapshot().client_num {
                if let Some(order) = order {
                    self.append(&format!("teamtask {}\n", self.static_state.borrow().current_order));
                    self.append(&format!("cmd vsay_team {}\n", order.personal));
                }
            } else if let Some(order) = order {
                self.append(&format!("cmd vtell {client} {}\n", order.team));
            }
        }
        if let Some(order) = order {
            if let Some(button) = order.button {
                self.append(button);
            }
        }
        self.static_state.borrow_mut().order_pending = false;
    }

    /// Publish the selected player name.
    fn set_selected_player_name(&self) {
        let index = self.selected();
        if index >= 0 && index < self.state.borrow().num_sorted_team_players {
            let number = self.state.borrow().sorted_team_players[index as usize];
            let client = self.static_state.borrow().client_info[number as usize].borrow().clone();
            self.set_cvar("cg_selectedPlayerName", &client.name);
            self.set_cvar("cg_selectedPlayer", &format!("{number}"));
            self.static_state.borrow_mut().current_order = client.team_task;
        } else {
            self.set_cvar("cg_selectedPlayerName", "Everyone");
        }
    }

    /// Selected player (`getSelectedPlayer`).
    #[must_use]
    pub fn get_selected_player(&self) -> i32 {
        let index = self.selected();
        if index < 0 || index >= self.state.borrow().num_sorted_team_players {
            self.set_selected(0);
        }
        self.selected()
    }

    /// Select the next player (`selectNextPlayer`).
    pub fn select_next_player(&self) {
        self.check_order_pending();
        let index = self.selected();
        self.set_selected(if index >= 0 && index < self.state.borrow().num_sorted_team_players {
            index + 1
        } else {
            0
        });
        self.set_selected_player_name();
    }

    /// Select the previous player (`selectPreviousPlayer`).
    pub fn select_previous_player(&self) {
        self.check_order_pending();
        let index = self.selected();
        self.set_selected(if index > 0 && index < self.state.borrow().num_sorted_team_players {
            index - 1
        } else {
            self.state.borrow().num_sorted_team_players
        });
        self.set_selected_player_name();
    }

    /// Feeder count (`feederCount`).
    pub fn feeder_count(&self, feeder: i32) -> i32 {
        if feeder == MissionScoreFeeder::Scoreboard as i32 {
            return self.state.borrow().num_scores;
        }
        let team = if feeder == MissionScoreFeeder::Red as i32 {
            Team::Red as i32
        } else if feeder == MissionScoreFeeder::Blue as i32 {
            Team::Blue as i32
        } else {
            return 0;
        };
        let mut count = 0;
        for index in 0..self.state.borrow().num_scores {
            if self.state.borrow().scores[index as usize].team == team {
                count += 1;
            }
        }
        count
    }

    /// Score + info for a feeder row.
    fn info_from_score_index(&self, index: i32, team: i32) -> (ClientScore, Shared<ClientInfo>) {
        let mut score_index = index;
        if self.static_state.borrow().game_type >= GameType::Team {
            let mut count = 0;
            for position in 0..self.state.borrow().num_scores {
                if self.state.borrow().scores[position as usize].team != team {
                    continue;
                }
                if count == index {
                    score_index = position;
                    break;
                }
                count += 1;
            }
        }
        let score = self
            .state
            .borrow()
            .scores
            .get(score_index as usize)
            .copied()
            .unwrap_or_else(|| {
                panic!("Mission HUD source index {score_index} outside 64");
            });
        let info = self
            .static_state
            .borrow()
            .client_info
            .get(score.client as usize)
            .cloned()
            .unwrap_or_else(|| {
                panic!("Mission HUD source index {} outside 64", score.client);
            });
        (score, info)
    }

    /// Feeder item (`feederItem`).
    pub fn feeder_item(&self, feeder: i32, index: i32, column: i32) -> MenuFeederItem {
        let team = if feeder == MissionScoreFeeder::Red as i32 {
            Team::Red as i32
        } else if feeder == MissionScoreFeeder::Blue as i32 {
            Team::Blue as i32
        } else {
            -1
        };
        let (score, info_handle) = self.info_from_score_index(index, team);
        let info = info_handle.borrow();
        let mut text = String::new();
        let mut picture = None;
        if info.info_valid {
            match column {
                0 => {
                    let mut powerup = None;
                    if info.powerups & (1 << Powerup::NeutralFlag as i32) != 0 {
                        powerup = Some(Powerup::NeutralFlag);
                    } else if info.powerups & (1 << Powerup::RedFlag as i32) != 0 {
                        powerup = Some(Powerup::RedFlag);
                    } else if info.powerups & (1 << Powerup::BlueFlag as i32) != 0 {
                        powerup = Some(Powerup::BlueFlag);
                    }
                    if let Some(powerup) = powerup {
                        let item = self
                            .media
                            .borrow()
                            .items
                            .borrow()
                            .find_for_powerup(self.state.borrow().product, powerup as i32);
                        let Some(item) = item else {
                            panic!("Mission HUD flag has no source item");
                        };
                        let at = self
                            .media
                            .borrow()
                            .items
                            .borrow()
                            .index_of(self.state.borrow().product, &item);
                        let visual = self.media.borrow().weapon_registry.borrow().item_visual(at);
                        picture = Some(self.media.borrow().resources.borrow().picture(&visual.icon));
                    } else if info.bot_skill > 0 && info.bot_skill <= 5 {
                        let shader = self
                            .media
                            .borrow()
                            .graphics
                            .bot_skill_shaders
                            .get((info.bot_skill - 1) as usize)
                            .cloned()
                            .unwrap_or_else(|| {
                                panic!("Mission HUD source index {} outside 5", info.bot_skill - 1);
                            });
                        picture = Some(self.media.borrow().resources.borrow().picture(&shader));
                    } else if info.handicap < 100 {
                        text = format!("{}", info.handicap);
                    }
                }
                1 => {
                    if team != -1 {
                        let shader = self.owner_draw.status_handle(info.team_task);
                        picture = Some(self.media.borrow().resources.borrow().picture(&shader));
                    }
                }
                2 => {
                    let schema = stat_schema(self.state.borrow().product);
                    if self.snapshot().stats.get(schema.clients_ready) & (1 << score.client) != 0 {
                        text = "Ready".to_string();
                    } else if team == -1 {
                        if self.static_state.borrow().game_type == GameType::Tournament {
                            text = format!("{}/{}", info.wins, info.losses);
                        } else if info.team == Team::Spectator {
                            text = "Spectator".to_string();
                        }
                    } else if info.team_leader {
                        text = "Leader".to_string();
                    }
                }
                3 => text = info.name.clone(),
                4 => text = format!("{}", info.score),
                5 => text = format!("{:>4}", score.time),
                6 => {
                    text = if score.ping == -1 {
                        "connecting".to_string()
                    } else {
                        format!("{:>4}", score.ping)
                    };
                }
                _ => {}
            }
        }
        MenuFeederItem { text, picture }
    }

    /// Feeder selection (`feederSelection`).
    pub fn feeder_selection(&self, feeder: i32, index: i32) {
        if self.static_state.borrow().game_type < GameType::Team {
            self.state.borrow_mut().selected_score = index;
            return;
        }
        let team = if feeder == MissionScoreFeeder::Red as i32 {
            Team::Red as i32
        } else {
            Team::Blue as i32
        };
        let mut count = 0;
        for position in 0..self.state.borrow().num_scores {
            if self.state.borrow().scores[position as usize].team != team {
                continue;
            }
            if index == count {
                self.state.borrow_mut().selected_score = position;
            }
            count += 1;
        }
    }

    /// Set the score selection (`setScoreSelection`).
    pub fn set_score_selection(&self, menu: Option<&CapturedMenu>) {
        let player = self.snapshot();
        let mut red = 0;
        let mut blue = 0;
        for position in 0..self.state.borrow().num_scores {
            let score = self.state.borrow().scores[position as usize];
            if score.team == Team::Red as i32 {
                red += 1;
            } else if score.team == Team::Blue as i32 {
                blue += 1;
            }
            if player.client_num == score.client {
                self.state.borrow_mut().selected_score = position;
            }
        }
        let Some(menu) = menu else {
            return;
        };
        if self.loaded.borrow().is_none() {
            return;
        }
        if self.static_state.borrow().game_type >= GameType::Team {
            let selected = self.state.borrow().selected_score;
            let is_blue = self.state.borrow().scores[selected as usize].team == Team::Blue as i32;
            self.loaded
                .borrow_mut()
                .as_mut()
                .expect("Mission HUD menus are not loaded")
                .runtime
                .set_captured_feeder_selection(
                    menu,
                    if is_blue {
                        MissionScoreFeeder::Blue as i32
                    } else {
                        MissionScoreFeeder::Red as i32
                    },
                    if is_blue { blue } else { red },
                );
        } else {
            let selected = self.state.borrow().selected_score;
            self.loaded
                .borrow_mut()
                .as_mut()
                .expect("Mission HUD menus are not loaded")
                .runtime
                .set_captured_feeder_selection(menu, MissionScoreFeeder::Scoreboard as i32, selected);
        }
    }

    /// Clear the scoreboard (`clearScoreboard`).
    pub fn clear_scoreboard(&self) {
        *self.scoreboard.borrow_mut() = None;
    }

    /// Menu scoreboard (`menuScoreboard`).
    #[must_use]
    pub fn menu_scoreboard(&self) -> Option<CapturedMenu> {
        self.scoreboard.borrow().clone()
    }

    /// Scroll a feeder (`scrollFeeder`).
    pub fn scroll_feeder(&self, menu: &CapturedMenu, feeder: i32, down: bool) {
        if self.scoreboard.borrow().as_ref() != Some(menu) {
            panic!("Score scrolling requires the current Mission HUD scoreboard");
        }
        self.open();
        if self.loaded.borrow().is_none() {
            panic!("Mission HUD menus are not loaded");
        }
        self.loaded
            .borrow_mut()
            .as_mut()
            .expect("Mission HUD menus are not loaded")
            .runtime
            .scroll_captured_feeder(menu, feeder, down);
    }

    /// Menu frame (time fields stay zero).
    fn frame(&self) -> MenuFrame {
        MenuFrame { time: 0, frame_time: 0 }
    }

    /// Paint all menus (`paintAll`).
    pub fn paint_all(&self) {
        self.open();
        if self.loaded.borrow().is_some() {
            let frame = self.frame();
            let mut draw = self.host.borrow().icons().borrow().tools.draw.clone();
            // The runtime only calls back into feeder/owner-draw queries.
            let mut callbacks = MissionHudCallbacks { hud: self };
            self.loaded
                .borrow_mut()
                .as_mut()
                .expect("Mission HUD menus are not loaded")
                .runtime
                .frame(&frame, &mut draw, &mut callbacks);
        }
    }

    /// Draw the scoreboard (`drawScoreboard`).
    pub fn draw_scoreboard(&self) -> bool {
        self.open();
        let state_pm = self.state.borrow().predicted_player_state.pm_type;
        if self.scoreboard.borrow().is_some() && self.loaded.borrow().is_some() {
            let menu = self.scoreboard.borrow().clone().expect("scoreboard");
            self.loaded
                .borrow_mut()
                .as_mut()
                .expect("Mission HUD menus are not loaded")
                .runtime
                .clear_captured_forced(&menu);
        }
        if self
            .host
            .borrow()
            .configuration()
            .borrow()
            .read_vm_cvar("cl_paused")
            .integer_value
            != 0
            || self.static_state.borrow().game_type == GameType::SinglePlayer && state_pm == MoveType::Intermission
        {
            self.state.borrow_mut().deferred_player_loading = 0;
            self.scoreboard_first_time.set(true);
            return false;
        }
        let (warmup, show_scores, time, score_fade) = {
            let state = self.state.borrow();
            (state.warmup, state.show_scores, state.time, state.score_fade_time)
        };
        if warmup != 0 && !show_scores {
            return false;
        }
        if !show_scores
            && state_pm != MoveType::Dead
            && state_pm != MoveType::Intermission
            && fade_color(time, score_fade, 200.0).is_none()
        {
            self.state.borrow_mut().deferred_player_loading = 0;
            self.state.borrow_mut().killer_name = String::new();
            self.scoreboard_first_time.set(true);
            return false;
        }
        if self.scoreboard.borrow().is_none() && self.loaded.borrow().is_some() {
            let name = if self.static_state.borrow().game_type >= GameType::Team {
                "teamscore_menu"
            } else {
                "score_menu"
            };
            *self.scoreboard.borrow_mut() = self
                .loaded
                .borrow()
                .as_ref()
                .expect("Mission HUD menus are not loaded")
                .runtime
                .menu_handle(name);
        }
        if self.scoreboard.borrow().is_some() && self.loaded.borrow().is_some() {
            if self.scoreboard_first_time.get() {
                let menu = self.scoreboard.borrow().clone().expect("scoreboard");
                self.set_score_selection(Some(&menu));
                self.open();
                self.scoreboard_first_time.set(false);
            }
            let menu = self.scoreboard.borrow().clone().expect("scoreboard");
            let frame = self.frame();
            let mut draw = self.host.borrow().icons().borrow().tools.draw.clone();
            let mut callbacks = MissionHudCallbacks { hud: self };
            self.loaded
                .borrow_mut()
                .as_mut()
                .expect("Mission HUD menus are not loaded")
                .runtime
                .paint_captured(&menu, &frame, true, &mut draw, &mut callbacks);
        }
        self.state.borrow_mut().deferred_player_loading += 1;
        if self.state.borrow().deferred_player_loading > 10 {
            let host = self.host.clone();
            self.host
                .borrow()
                .clients()
                .borrow_mut()
                .load_deferred_players(&mut |entity| {
                    host.borrow_mut().reset_player_entity(entity);
                });
        }
        true
    }

    /// Close by name (`closeByName`).
    pub fn close_by_name(&self, name: &str) {
        self.open();
        if let Some(loaded) = self.loaded.borrow_mut().as_mut() {
            loaded.runtime.close(name);
        }
    }

    /// Show the response head (`showResponseHead`).
    pub fn show_response_head(&self) {
        self.open();
        if let Some(loaded) = self.loaded.borrow_mut().as_mut() {
            loaded.runtime.show("voiceMenu");
        }
        self.set_cvar("cl_conXOffset", "72");
        let time = self.state.borrow().time;
        self.state.borrow_mut().voice_time = time;
    }

    /// Draw timed menus (`drawTimedMenus`).
    pub fn draw_timed_menus(&self) {
        let (voice_time, time) = {
            let state = self.state.borrow();
            (state.voice_time, state.time)
        };
        if voice_time != 0 && time.wrapping_sub(voice_time) > 2500 {
            self.close_by_name("voiceMenu");
            self.set_cvar("cl_conXOffset", "0");
            self.state.borrow_mut().voice_time = 0;
        }
    }

    /// Client number from a name (`clientNumFromName`).
    #[must_use]
    pub fn client_num_from_name(&self, name: &str) -> i32 {
        let text = hud_fold(&hud_source_text(name, name.chars().count() + 1));
        for index in 0..self.static_state.borrow().maxclients {
            let client = self.static_state.borrow().client_info[index as usize].borrow().clone();
            if client.info_valid && hud_fold(&client.name) == text {
                return index;
            }
        }
        -1
    }

    /// Hide the team menu (`hideTeamMenu`).
    pub fn hide_team_menu(&self) {
        self.close_by_name("teamMenu");
        self.close_by_name("getMenu");
    }

    /// Show the team menu (`showTeamMenu`).
    pub fn show_team_menu(&self) {
        self.open();
        if let Some(loaded) = self.loaded.borrow_mut().as_mut() {
            loaded.runtime.show("teamMenu");
        }
    }

    /// Event handling (`eventHandling`).
    pub fn event_handling(&self, kind: i32) {
        self.static_state.borrow_mut().event_handling = kind;
        if kind == 0 {
            self.hide_team_menu();
        }
    }

    /// Mouse event (`mouseEvent`).
    pub fn mouse_event(&self, x: f32, y: f32) {
        // vmMain copies the old cgs cursor into cgDC before CG_MouseEvent applies its delta.
        if let Some(loaded) = self.loaded.borrow_mut().as_mut() {
            let (cursor_x, cursor_y) = {
                let cgs = self.static_state.borrow();
                (cgs.cursor_x, cgs.cursor_y)
            };
            loaded.runtime.set_display_cursor(cursor_x, cursor_y);
        }
        let pm_type = self.state.borrow().predicted_player_state.pm_type;
        if (pm_type == MoveType::Normal || pm_type == MoveType::Spectator) && !self.state.borrow().show_scores {
            self.host.borrow_mut().set_key_catcher(0);
            return;
        }
        {
            let mut cgs = self.static_state.borrow_mut();
            cgs.cursor_x = (cgs.cursor_x + x).clamp(0.0, 640.0);
            cgs.cursor_y = (cgs.cursor_y + y).clamp(0.0, 480.0);
        }
        let (cursor_x, cursor_y) = {
            let cgs = self.static_state.borrow();
            (cgs.cursor_x, cgs.cursor_y)
        };
        let cursor = self
            .loaded
            .borrow()
            .as_ref()
            .map(|loaded| loaded.runtime.cursor_type(cursor_x, cursor_y))
            .unwrap_or(MenuCursorType::Arrow);
        self.static_state.borrow_mut().active_cursor = if cursor == MenuCursorType::Arrow {
            self.media.borrow().graphics.select_cursor.clone()
        } else {
            self.media.borrow().graphics.size_cursor.clone()
        };
        if self.loaded.borrow().is_none() {
            return;
        }
        if let Some(menu) = self.captured_menu.borrow().clone() {
            self.loaded
                .borrow_mut()
                .as_mut()
                .expect("Mission HUD menus are not loaded")
                .runtime
                .move_captured_menu(&menu, x, y);
        } else {
            self.loaded
                .borrow_mut()
                .as_mut()
                .expect("Mission HUD menus are not loaded")
                .runtime
                .pointer_move(cursor_x, cursor_y);
        }
    }

    /// Key event (`keyEvent`).
    pub fn key_event(&self, key: i32, down: bool) {
        if !down {
            return;
        }
        let pm_type = self.state.borrow().predicted_player_state.pm_type;
        if pm_type == MoveType::Normal || pm_type == MoveType::Spectator && !self.state.borrow().show_scores {
            self.event_handling(0);
            self.host.borrow_mut().set_key_catcher(0);
            return;
        }
        if let Some(loaded) = self.loaded.borrow_mut().as_mut() {
            let (cursor_x, cursor_y) = {
                let cgs = self.static_state.borrow();
                (cgs.cursor_x, cgs.cursor_y)
            };
            loaded.runtime.handle_key(key, down, cursor_x, cursor_y);
        }
        if self.captured_menu.borrow().is_some() {
            *self.captured_menu.borrow_mut() = None;
        } else if key == KEY_MOUSE2 && self.loaded.borrow().is_some() {
            let (cursor_x, cursor_y) = {
                let cgs = self.static_state.borrow();
                (cgs.cursor_x, cgs.cursor_y)
            };
            *self.captured_menu.borrow_mut() = self
                .loaded
                .borrow_mut()
                .as_mut()
                .expect("Mission HUD menus are not loaded")
                .runtime
                .capture_menu(cursor_x, cursor_y);
        }
    }
}

/// Fixed weapon HUD reader.
struct FixedWeaponHudReader {
    /// Report.
    report: WeaponHudReport,
}

impl WeaponHudReader for FixedWeaponHudReader {
    fn read_weapon_hud(&self) -> WeaponHudReport {
        self.report
    }
}

/// Configstring view over a mission host.
struct MissionHudStrings(Shared<dyn MissionHudHost>);

impl HudConfigStrings for MissionHudStrings {
    fn config_string(&self, index: usize) -> String {
        self.0.borrow().config_string(index)
    }
}

/// Load context adapter.
struct MissionHudLoadContext<'a> {
    /// HUD.
    hud: &'a MissionHud,
}

impl MenuLoadContext for MissionHudLoadContext<'_> {
    fn random_next_int(&mut self) -> i32 {
        self.hud.host.borrow().random().borrow_mut().rand()
    }

    fn resolve_root(&mut self, requested: &str) -> Option<MenuSource> {
        self.hud.source(requested)
    }

    fn resolve(&mut self, from_path: &str, requested: &str) -> Option<MenuSource> {
        self.hud
            .source(&posix_join(posix_dirname(from_path), requested))
            .or_else(|| self.hud.source(requested))
    }

    fn register_font(&mut self, reference: &FontReference) {
        self.current();
        self.hud.register_font(reference.path.as_deref(), reference.point_size);
        self.current();
    }

    fn register_picture(&mut self, path: Option<&str>) -> u32 {
        self.current();
        let picture = self.hud.register_picture(path);
        self.current();
        picture.map(|picture| picture.order).unwrap_or(0)
    }

    fn register_sound(&mut self, path: Option<&str>) -> u32 {
        self.current();
        let sound = self.hud.register_sound(path);
        self.current();
        self.hud.media.borrow().sound_bank.borrow().index_for_sound(&sound) as u32
    }

    fn register_model(&mut self, path: Option<&str>) -> u32 {
        self.current();
        let model = self.hud.register_model(path);
        self.current();
        self.hud.media.borrow().resources.borrow().model_handle(&model)
    }

    fn publish_asset_font(&mut self, field: &str, reference: &FontReference) {
        self.current();
        if field != "smallFont" && field != "textFont" && field != "bigFont" {
            return;
        }
        let Some(font) = self.hud.font(reference) else {
            return;
        };
        match field {
            "smallFont" => self.hud.fonts_handle.borrow_mut().small = font,
            "textFont" => self.hud.fonts_handle.borrow_mut().normal = font,
            _ => self.hud.fonts_handle.borrow_mut().big = font,
        }
    }

    fn initial_assets(&self) -> Option<MenuGlobalAssets> {
        self.hud.asset_definitions.borrow().clone()
    }
}

impl MissionHudLoadContext<'_> {
    /// Require a current load generation.
    fn current(&self) {
        self.hud.open();
        if self.hud.load_generation.get() != self.hud.generation.get() {
            panic!("Mission HUD menu load belongs to a retired lifecycle");
        }
    }
}

/// Paint callback adapter.
struct MissionHudCallbacks<'a> {
    /// HUD.
    hud: &'a MissionHud,
}

impl MenuPaintCallbacks for MissionHudCallbacks<'_> {
    fn feeder_count(&mut self, feeder: i32) -> i32 {
        self.hud.feeder_count(feeder)
    }

    fn feeder_item(&mut self, feeder: i32, index: i32, column: i32) -> MenuFeederItem {
        self.hud.feeder_item(feeder, index, column)
    }

    fn feeder_select(&mut self, feeder: i32, index: i32) {
        self.hud.feeder_selection(feeder, index);
    }

    fn owner_visible(&mut self, flags: i32) -> bool {
        self.hud.owner_draw.visible(flags)
    }

    fn owner_width(&mut self, id: i32, scale: f32) -> f32 {
        self.hud.owner_draw.width(id, scale)
    }

    fn owner_value(&mut self, id: i32) -> f32 {
        self.hud.owner_draw.value(id)
    }

    fn owner_paint(&mut self, request: &mut OwnerDrawPaintRequest) {
        self.hud.owner_draw.paint(request);
    }

    fn close_cinematic(&mut self, handle: i32) {
        self.hud.host.borrow().cinematics().borrow_mut().stop_slot(handle);
    }

    fn team_color(&mut self) -> Vec4 {
        match self.hud.snapshot().persistant.get(PersistentIndex::Team as i32) {
            x if x == Team::Red as i32 => vec4(1.0, 0.0, 0.0, 0.25),
            x if x == Team::Blue as i32 => vec4(0.0, 0.0, 1.0, 0.25),
            _ => vec4(0.0, 0.17, 0.0, 0.25),
        }
    }

    fn paint_model(&mut self, request: &UiModelPaintRequest) {
        self.hud.host.borrow().model_painter().paint(request);
    }

    fn cvar_value(&mut self, name: &str) -> f64 {
        game_atof(
            &self
                .hud
                .host
                .borrow()
                .cvars()
                .borrow()
                .get(name)
                .map(|snapshot| snapshot.value.chars().take(127).collect::<String>())
                .unwrap_or_default(),
        )
    }
}

impl ConsoleHud for MissionHud {
    fn reset_strings(&mut self) {
        MissionHud::reset_strings(self);
    }
    fn reset_menus(&mut self) {
        MissionHud::reset_menus(self);
    }
    fn load_menus(&mut self, path: &str) {
        MissionHud::load_menus(self, path);
    }
    fn clear_scoreboard(&mut self) {
        MissionHud::clear_scoreboard(self);
    }
    fn menu_scoreboard(&self) -> Option<CapturedMenu> {
        MissionHud::menu_scoreboard(self)
    }
    fn scroll_feeder(&mut self, menu: &CapturedMenu, feeder: i32, down: bool) {
        MissionHud::scroll_feeder(self, menu, feeder, down);
    }
}

impl ConsoleOrders for MissionHud {
    fn select_next_player(&mut self) {
        MissionHud::select_next_player(self);
    }
    fn select_previous_player(&mut self) {
        MissionHud::select_previous_player(self);
    }
    fn other_team_has_flag(&self) -> bool {
        MissionHud::other_team_has_flag(self)
    }
    fn your_team_has_flag(&self) -> bool {
        MissionHud::your_team_has_flag(self)
    }
}

#[cfg(test)]
#[allow(dead_code)]
mod tests {
    use super::*;
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
        assert_eq!(fade_color(2950, 1000, 3000.0).unwrap().w, 50.0 / 200.0);
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
        assert_eq!(proportional_string_width("A"), 15);
        assert_eq!(banner_string_width("A"), 29);
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
        assert_eq!(runtime.execute(&["viewpos".to_string()]).unwrap(), true);
        assert_eq!(commands.borrow().printed.len(), 1);
        assert_eq!(runtime.execute(&["kill".to_string()]).unwrap(), false);
        assert_eq!(runtime.execute(&["weapon".to_string(), "2".to_string()]).unwrap(), true);
        assert_eq!(weapons.borrow().selected, vec![2]);
        assert_eq!(runtime.execute(&["nextframe".to_string()]).unwrap(), true);
        assert!(view.borrow().calls.contains(&"nextframe".to_string()));
        let big = vec!["x".to_string(); 1025];
        assert!(runtime.execute(&big).is_err());
    }

    #[test]
    fn console_scores_and_tcmd() {
        let (runtime, commands, _, _) = console_fixture();
        runtime.state.borrow_mut().time = 5000;
        assert_eq!(runtime.execute(&["+scores".to_string()]).unwrap(), true);
        assert!(runtime.state.borrow().show_scores);
        assert_eq!(commands.borrow().client, vec!["score".to_string()]);
        assert_eq!(runtime.execute(&["-scores".to_string()]).unwrap(), true);
        assert!(!runtime.state.borrow().show_scores);
        runtime.state.borrow_mut().crosshair_client_num = 3;
        runtime.state.borrow_mut().crosshair_client_time = 5000;
        assert_eq!(runtime.execute(&["tcmd".to_string(), "2".to_string()]).unwrap(), true);
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
