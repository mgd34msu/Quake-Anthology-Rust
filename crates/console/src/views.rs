//! Source views select defaults and conversions, never a second cvar table.
use crate::catalog::Scope;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Source {
    Quake,
    QuakeWorld,
    Quake2,
    Quake2Rerelease,
    Quake3,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Engine,
    Game,
    Cgame,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Context {
    pub source: Source,
    pub side: Scope,
    pub role: Role,
    pub dedicated: bool,
}
impl Default for Context {
    fn default() -> Self {
        Self {
            source: Source::Quake3,
            side: Scope::Client,
            role: Role::Engine,
            dedicated: false,
        }
    }
}

impl Source {
    pub const ALL: [Self; 5] = [
        Self::Quake,
        Self::QuakeWorld,
        Self::Quake2,
        Self::Quake2Rerelease,
        Self::Quake3,
    ];
}
