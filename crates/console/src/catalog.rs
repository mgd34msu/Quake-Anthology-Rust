//! Immutable cvar metadata. Game defaults are views of one shared table.
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueType {
    Float,
    Int,
    Bool,
    Enum,
    String,
    Bitmask,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DefaultKind {
    Native,
    Unset,
    StoredOnly,
    Anthology,
    Conditional,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Condition {
    Always,
    Mac,
    NotMac,
    Linux,
    NotLinux,
    Dedicated,
    Client,
    Engine,
    Game,
    Cgame,
    Unresolved,
    Windows,
    NotWindows,
}
pub struct DefaultClause {
    pub member: &'static str,
    pub value: &'static str,
    pub condition_text: &'static str,
    pub raw: &'static str,
    pub issues: u32,
    pub kind: DefaultKind,
    pub condition: Condition,
}
pub struct FlagClause {
    pub member: &'static str,
    pub raw: &'static str,
    pub bits: u32,
    pub auxiliary: u32,
    pub issues: u32,
}
pub struct SourceDefinition {
    pub raw_default: &'static str,
    pub raw_flags: &'static str,
    pub effect: &'static str,
    pub defaults: Range<usize>,
    pub flags: Range<usize>,
}
pub struct Definition {
    pub name: &'static str,
    pub aliases: &'static str,
    pub range_hint: &'static str,
    pub owner: &'static str,
    pub conversion: &'static str,
    pub sources: &'static str,
    pub raw_flags: &'static str,
    pub audit_status: [&'static str; 3],
    /// Q1, QuakeWorld, Q2 classic, Q2 rerelease, Q3 defaults and flags.
    pub defaults: [SourceDefinition; 5],
    pub issues: u32,
    pub archive: bool,
    pub policies: u32,
    pub rule_conversion: u16,
    pub value_type: ValueType,
    pub home: Option<u8>,
    pub family_count: u8,
    pub stored: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConversionKind {
    Identity,
    Reciprocal,
    BoolInvert,
    Linear,
    EnumDetail,
    BitView,
    Composite,
    Resolution,
    ConsumerUnits,
    SideScope,
    Policy,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    None,
    KhzHz,
    Skill,
    ViewSize,
    BoolDetail,
    Autoswitch,
    Gun,
    Footsteps,
    Lagometer,
    Draw2D,
    Shadows,
    OldRail,
    InputGrab,
    SoundBackend,
    NoSkins,
    ForceRespawn,
    Deathmatch,
    Coop,
    Teamplay,
    Ctf,
    QwSkin,
    Sex,
    Color,
    PlayerColors,
    Needpass,
    SameLevel,
    NoExit,
    Download,
    ClearColor,
    Fullscreen,
    VideoMode,
    MusicMute,
    Spectator,
}
pub struct Conversion {
    pub scale: f64,
    pub offset: f64,
    pub lower: f64,
    pub upper: f64,
    pub raw: &'static str,
    pub operands: Range<usize>,
    pub maps: Range<usize>,
    pub issues: u32,
    pub kind: ConversionKind,
    pub detail: bool,
    pub cgame_only: bool,
    pub operation: Operation,
}
pub struct Operand {
    pub row: u16,
    pub inverted: bool,
    pub positive: bool,
    pub unified_bit: u32,
    pub source_bits: [u32; 5],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    AliasToCanonical,
    CanonicalToAlias,
}
pub struct EnumMap {
    pub alias: f64,
    pub canonical: f64,
    pub direction: Direction,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Any,
    Client,
    Server,
}
pub struct Binding {
    pub name: &'static str,
    pub row: u16,
    pub conversions: [u16; 5],
    pub scope: Scope,
    pub canonical: bool,
    pub native_sources: u8,
    pub seat: u8,
}
