//! Q2 product composition types (`src/content/composition/q2/types.ts`).
//!
//! The donor builds the game, weapons, and session host inside the
//! composition root. The port registers into a session-owned
//! [`Q2GameServices`](crate::q2::foundation::host::Q2GameServices) arena
//! instead, so the host and weapon-system handles are dropped and every
//! session callback routes through the [`CompositionRuntime`](super::CompositionRuntime)
//! arena slot.

use qa_core::identity::ActorId;

use crate::q2::base::entities::types::{Q2LocalTimeHook, Q2PlayerPushHook, Q2SetActorGravityHook};
use crate::q2::base::player::types::{Q2PlayerHooks, Q2PlayerRules};
use crate::q2::foundation::host::Q2GameOptions;
use crate::q2::foundation::items::Q2ItemHooks;
use crate::q2::missionpacks::entities::types::Q2MissionPackEntityEvent;
use crate::q2::missionpacks::random_items::Q2RandomItemSettings;
use crate::q2::missionpacks::types::Q2MissionPackPlayerEffect;
use crate::q2::multiplayer::ctf::types::Q2CtfEvent;
use crate::q2::multiplayer::lmctf::types::{LmctfEvent, LmctfTravel};
use crate::q2::rerelease::campaign::Q2RereleaseCampaignState;
use crate::q2::rerelease::types::{Q2RereleaseHooks, Q2RereleaseOptions};
use crate::q2::support::contracts::SharedGrappleControl;

/// Classic Q2 program (`Q2ClassicProgram`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q2ClassicProgram {
    /// Base Q2.
    Baseq2,
    /// Xatrix.
    Xatrix,
    /// Rogue.
    Rogue,
}

impl Q2ClassicProgram {
    /// Provider program name.
    pub fn as_str(&self) -> &'static str {
        match self {
            Q2ClassicProgram::Baseq2 => "baseq2",
            Q2ClassicProgram::Xatrix => "xatrix",
            Q2ClassicProgram::Rogue => "rogue",
        }
    }
}

/// Rerelease Q2 program (`Q2RereleaseProgram`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q2RereleaseProgram {
    /// Base Q2.
    Baseq2,
    /// Xatrix.
    Xatrix,
    /// Rogue.
    Rogue,
    /// Machinegames 2.
    Mg2,
}

impl Q2RereleaseProgram {
    /// Provider program name.
    pub fn as_str(&self) -> &'static str {
        match self {
            Q2RereleaseProgram::Baseq2 => "baseq2",
            Q2RereleaseProgram::Xatrix => "xatrix",
            Q2RereleaseProgram::Rogue => "rogue",
            Q2RereleaseProgram::Mg2 => "mg2",
        }
    }
}

/// Selected Q2 match (`Q2MatchSelection`).
#[derive(Debug, Clone, PartialEq)]
pub enum Q2MatchSelection {
    /// Capture the flag.
    Ctf,
    /// LMCTF with optional time-travel.
    Lmctf {
        /// Pending time-travel.
        travel: Option<LmctfTravel>,
    },
    /// Standard deathmatch.
    Standard,
    /// Tag.
    Tag,
    /// Deathball with cvar-frozen settings.
    Deathball {
        /// Team 1 skin.
        team1_skin: String,
        /// Team 2 skin.
        team2_skin: String,
        /// Goal limit.
        goal_limit: f64,
    },
}

impl Q2MatchSelection {
    /// Match kind name.
    pub fn kind(&self) -> &'static str {
        match self {
            Q2MatchSelection::Ctf => "ctf",
            Q2MatchSelection::Lmctf { .. } => "lmctf",
            Q2MatchSelection::Standard => "standard",
            Q2MatchSelection::Tag => "tag",
            Q2MatchSelection::Deathball { .. } => "deathball",
        }
    }
}

/// Foreign powerup expiries (`Q2ForeignPowerups`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2ForeignPowerups {
    /// Quad expiry.
    pub quad_until: f64,
    /// Double expiry.
    pub double_until: f64,
    /// Invulnerability expiry.
    pub invulnerability_until: f64,
}

/// Product composition event (`Q2CompositionEvent`).
#[derive(Debug, Clone, PartialEq)]
pub enum Q2CompositionEvent {
    /// CTF event.
    Ctf(Q2CtfEvent),
    /// LMCTF event.
    Lmctf(LmctfEvent),
    /// Grapple prediction toggle.
    GrapplePrediction {
        /// Viewer.
        actor: ActorId,
        /// Whether suppressed.
        suppress: bool,
    },
    /// Kick a player.
    Kick {
        /// Player.
        actor: ActorId,
    },
    /// Mission-pack player effect.
    MissionpackPlayer(Q2MissionPackPlayerEffect),
    /// Mission-pack entity event.
    MissionpackEntity(Q2MissionPackEntityEvent),
}

/// Session deathmatch-flag storage (`Q2CompositionServices::deathmatchFlags`).
#[derive(Debug, Clone, Copy)]
pub struct Q2DeathmatchFlagsHooks {
    /// Read flags.
    pub read: fn() -> i32,
    /// Write flags.
    pub write: fn(i32),
}

/// Session services (`Q2CompositionServices`).
pub struct Q2CompositionServices {
    /// Session deathmatch-flag storage.
    pub deathmatch_flags: Option<Q2DeathmatchFlagsHooks>,
    /// Shared grapple control.
    pub shared_grapple: Option<Box<dyn SharedGrappleControl>>,
    /// Random item settings.
    pub random_items: Option<fn() -> Q2RandomItemSettings>,
    /// Quad-fire drop policy.
    pub drop_quad_fire: Option<fn() -> bool>,
    /// Gravity scale.
    pub gravity: fn() -> f64,
    /// Session emit.
    pub emit: fn(Q2CompositionEvent),
    /// Hunter camera policy.
    pub hunter_camera: bool,
    /// Strong mines policy.
    pub strong_mines: bool,
    /// Foreign powerup expiries.
    pub foreign_powerups: fn(ActorId) -> Q2ForeignPowerups,
}

impl std::fmt::Debug for Q2CompositionServices {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Q2CompositionServices")
            .field("deathmatch_flags", &self.deathmatch_flags)
            .field("shared_grapple", &self.shared_grapple.is_some())
            .field("random_items", &self.random_items)
            .field("drop_quad_fire", &self.drop_quad_fire)
            .field("gravity", &self.gravity)
            .field("emit", &self.emit)
            .field("hunter_camera", &self.hunter_camera)
            .field("strong_mines", &self.strong_mines)
            .field("foreign_powerups", &self.foreign_powerups)
            .finish()
    }
}

/// Entity hooks shared with the base module (`Q2CompositionCommon::entities`).
#[derive(Debug, Clone, Copy)]
pub struct Q2CompositionEntityHooks {
    /// Push a player.
    pub player_push: Q2PlayerPushHook,
    /// Set actor gravity.
    pub set_actor_gravity: Q2SetActorGravityHook,
    /// Read the local clock.
    pub local_time: Q2LocalTimeHook,
}

/// Options shared by both editions (`Q2CompositionCommon`).
pub struct Q2CompositionCommon {
    /// Game options (the edition field is ignored; the edition comes from
    /// the selected [`Q2CompositionOptions`] variant).
    pub options: Q2GameOptions,
    /// Item hooks.
    pub item_hooks: Q2ItemHooks,
    /// Player hooks.
    pub player_hooks: Q2PlayerHooks,
    /// Player rules override.
    pub player_rules: Option<Q2PlayerRules>,
    /// Match selection override.
    pub match_selection: Option<Q2MatchSelection>,
    /// Entity hooks.
    pub entity_hooks: Q2CompositionEntityHooks,
    /// Session services.
    pub services: Q2CompositionServices,
}

impl std::fmt::Debug for Q2CompositionCommon {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Q2CompositionCommon")
            .field("options", &self.options)
            .field("item_hooks", &self.item_hooks)
            .field("player_hooks", &self.player_hooks)
            .field("player_rules", &self.player_rules)
            .field("match_selection", &self.match_selection)
            .field("entity_hooks", &self.entity_hooks)
            .field("services", &self.services)
            .finish()
    }
}

/// Product composition options (`Q2CompositionOptions`).
pub enum Q2CompositionOptions {
    /// Classic composition.
    Classic {
        /// Shared options.
        common: Q2CompositionCommon,
        /// Program.
        program: Q2ClassicProgram,
    },
    /// Rerelease composition.
    Rerelease {
        /// Shared options.
        common: Q2CompositionCommon,
        /// Program.
        program: Q2RereleaseProgram,
        /// Rerelease hooks.
        rerelease_hooks: Q2RereleaseHooks,
        /// Rerelease option overrides.
        rerelease_options: Option<Q2RereleaseOptions>,
        /// Campaign state override.
        campaign: Option<Q2RereleaseCampaignState>,
    },
}

impl std::fmt::Debug for Q2CompositionOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Q2CompositionOptions::Classic { common, program } => {
                f.debug_struct("Classic").field("common", common).field("program", program).finish()
            }
            Q2CompositionOptions::Rerelease { common, program, rerelease_hooks, rerelease_options, campaign } => {
                f.debug_struct("Rerelease")
                    .field("common", common)
                    .field("program", program)
                    .field("rerelease_hooks", rerelease_hooks)
                    .field("rerelease_options", rerelease_options)
                    .field("campaign", campaign)
                    .finish()
            }
        }
    }
}

/// Session cvar source (local shim for the out-of-scope `CvarRegistry`).
#[derive(Debug, Clone, Copy)]
pub struct Q2CvarSource {
    /// Read a string cvar.
    pub variable_string: fn(&str) -> String,
    /// Read a numeric cvar.
    pub variable_value: fn(&str) -> f64,
}

/// Truncate command text at NUL and reject non-source bytes
/// (`sourceCommandText` in `core/commands/text.ts`).
fn source_command_text(input: &str) -> Vec<char> {
    let text = input.split('\0').next().unwrap_or("");
    for value in text.chars() {
        if value as u32 > 255 {
            panic!("Command text requires source bytes");
        }
    }
    text.chars().collect()
}

/// Set a q2-classic userinfo pair (`Info_SetValueForKey` with the
/// `q2-classic` dialect in `core/cvars/info.ts`).
pub fn set_q2_info_value(
    game: &mut crate::q2::foundation::host::Q2GameServices,
    source: &str,
    key: &str,
    value: &str,
    maximum_length: usize,
) -> String {
    let info = source_command_text(source);
    let key = source_command_text(key);
    let value = source_command_text(value);
    if info.len() >= maximum_length {
        panic!("Info_SetValueForKey: oversize infostring");
    }
    if key.contains(&'\\') || value.contains(&'\\') {
        game.host.diagnostic("Can't use keys or values with a \\\n");
        return info.into_iter().collect();
    }
    if key.contains(&';') {
        game.host.diagnostic("Can't use keys with a semicolon\n");
        return info.into_iter().collect();
    }
    if key.contains(&'"') || value.contains(&'"') {
        game.host.diagnostic("Can't use keys or values with a \"\n");
        return info.into_iter().collect();
    }
    if key.len() > 63 || value.len() > 63 {
        game.host.diagnostic("Keys and values must be < 64 characters.\n");
    }
    let mut result = info.clone();
    let mut cursor = 0;
    while cursor < result.len() {
        if result[cursor] == '\\' {
            cursor += 1;
        }
        let start = cursor;
        while cursor < result.len() && result[cursor] != '\\' {
            cursor += 1;
        }
        if result[start..cursor] == key[..] {
            while cursor < result.len() && result[cursor] == '\\' {
                cursor += 1;
            }
            while cursor < result.len() && result[cursor] != '\\' {
                cursor += 1;
            }
            result.drain(start..cursor);
            break;
        }
        if cursor >= result.len() {
            break;
        }
    }
    if value.is_empty() {
        return result.into_iter().collect();
    }
    let mut pair = vec!['\\'];
    pair.extend(key.iter());
    pair.push('\\');
    pair.extend(value.iter());
    if pair.len() + result.len() > maximum_length {
        game.host.diagnostic("Info string length exceeded\n");
        return result.into_iter().collect();
    }
    let pair: Vec<char> = pair
        .into_iter()
        .map(|value| (((value as u32) & 127) as u8) as char)
        .filter(|value| *value >= ' ' && (*value as u8) < 127)
        .collect();
    if pair.len() + result.len() == maximum_length {
        panic!("Info string overflows source terminator");
    }
    result.extend(pair);
    result.into_iter().collect()
}
