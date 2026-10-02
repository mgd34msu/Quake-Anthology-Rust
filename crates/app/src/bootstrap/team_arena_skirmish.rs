//! Team Arena single-player skirmish planning over authored campaign metadata.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/team-arena-skirmish.ts`
//! (`TeamArenaTeams`, `teamArenaTeamChoices`, `TeamArenaSkirmishCursor`,
//! `TeamArenaSkirmish`, `TeamArenaCampaign`, `parseTeamArenaCampaign`,
//! `nextTeamArenaCursor`, `planTeamArenaSkirmish`, `currentTeamArenaCursor`,
//! `readTeamArenaCampaign`, `loadTeamArenaCampaign`, `readTeamArenaSkirmish`,
//! `teamArenaServerOverrides`, `teamArenaSourceCvars`, `teamArenaClientCvars`,
//! `TeamArenaLaunchOverrides`). Metadata grammar and defaults follow
//! `code/ui/ui_main.c`, `UI_StartSkirmish`. Sync port: the donor's async
//! catalog reads become the injected [`TeamArenaMounts`] seam, and
//! `InstalledCatalog` preparation plus mount-plan opening (unported siblings)
//! arrive through [`TeamArenaProductCatalog`]. `COM_Parse` tokenizing is
//! ported inline (the shared parser is unported); game types, cvar defaults,
//! and the product policy reuse `qa_content`.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use qa_content::q3::base::settings::q3_game_cvar_definitions;
use qa_content::q3::base::shared::definitions::GameType;
use qa_content::q3::base::shared::definitions::Product;
use qa_content::q3::product_restriction::Q3ProductPolicy;
use qa_content::q3::product_restriction::TeamArenaUi;
use qa_core::cvar::CvarError;
use qa_core::cvar::CvarRegistry;
use qa_core::identity::SeatId;
use thiserror::Error;

/// Team Arena skirmish failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TeamArenaSkirmishError {
    /// Section grammar violation (donor `Invalid Team Arena {section} ...`).
    #[error("{0}")]
    BadSection(String),
    /// Short metadata row.
    #[error("Incomplete Team Arena metadata row")]
    IncompleteRow,
    /// Non-integer metadata field.
    #[error("Invalid Team Arena metadata integer")]
    BadInteger,
    /// Metadata integer outside the safe-integer range.
    #[error("Team Arena metadata integer is out of range")]
    IntegerOutOfRange,
    /// Bad campaign map row.
    #[error("Invalid Team Arena campaign map")]
    BadMap,
    /// Cursor indexes a missing game type or map.
    #[error("Invalid Team Arena campaign cursor")]
    BadCursor,
    /// No authored maps for the next game type.
    #[error("Next Team Arena game type has no authored single-player maps")]
    NoNextMaps,
    /// Game type outside single-player support.
    #[error("Unsupported Team Arena single-player game type")]
    UnsupportedGameType,
    /// Map not authored for the skirmish.
    #[error("Selected Team Arena map is not authored for this skirmish")]
    MapNotAuthored,
    /// Unknown team name.
    #[error("Team Arena team is missing: {0}")]
    MissingTeam(String),
    /// Live match has no campaign entry.
    #[error("Current Team Arena match has no authored campaign entry")]
    NoCurrentEntry,
    /// Catalog preparation yielded no Q3 product.
    #[error("Team Arena requires a Q3 product policy")]
    RequiresProductPolicy,
    /// Seat adopted after preparation.
    #[error("Team Arena preparation adopted an unknown seat")]
    UnknownSeat,
    /// Mount read failure.
    #[error("Team Arena mount read failed: {0}")]
    Mount(String),
    /// Non-Latin-1 metadata input.
    #[error("COM_Parse source is not a Latin-1 byte string at {0}")]
    NonLatin1(usize),
    /// Quoted token terminator at the 1024-byte storage limit.
    #[error("COM_Parse quoted token terminator exceeds 1024-byte storage")]
    TokenOverflow,
    /// Cvar failure.
    #[error(transparent)]
    Cvar(#[from] CvarError),
}

/// Maximum `COM_Parse` token storage (donor `MAX_TOKEN_CHARS`).
const MAX_TOKEN_CHARS: usize = 1024;
/// Maximum safe integer (donor `Number.isSafeInteger`).
const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

/// Latin-1 decode (donor `Buffer.toString("latin1")`).
fn latin1_to_string(bytes: &[u8]) -> String {
    bytes.iter().map(|&byte| byte as char).collect()
}

/// `COM_Parse` token stream over Latin-1 bytes (comments, quotes, NUL end).
struct ComParse<'a> {
    bytes: &'a [u8],
    offset: Option<usize>,
}

impl<'a> ComParse<'a> {
    fn new(text: &'a str) -> Result<Self, TeamArenaSkirmishError> {
        for (index, char) in text.char_indices() {
            if char as u32 > 0xff {
                return Err(TeamArenaSkirmishError::NonLatin1(index));
            }
        }
        Ok(Self {
            bytes: text.as_bytes(),
            offset: Some(0),
        })
    }

    fn signed_byte(&self, offset: usize) -> i16 {
        if offset >= self.bytes.len() {
            return 0;
        }
        let byte = self.bytes[offset];
        if byte >= 128 {
            i16::from(byte) - 256
        } else {
            i16::from(byte)
        }
    }

    fn byte_at(&self, offset: usize) -> u8 {
        if offset < self.bytes.len() {
            self.bytes[offset]
        } else {
            0
        }
    }

    fn parse(&mut self) -> Result<String, TeamArenaSkirmishError> {
        let Some(mut data) = self.offset else {
            return Ok(String::new());
        };
        loop {
            while self.signed_byte(data) <= 32 {
                if self.signed_byte(data) == 0 {
                    self.offset = None;
                    return Ok(String::new());
                }
                data += 1;
            }
            if self.byte_at(data) == b'/' && self.byte_at(data + 1) == b'/' {
                data += 2;
                while self.byte_at(data) != 0 && self.byte_at(data) != b'\n' {
                    data += 1;
                }
            } else if self.byte_at(data) == b'/' && self.byte_at(data + 1) == b'*' {
                data += 2;
                while self.byte_at(data) != 0 && !(self.byte_at(data) == b'*' && self.byte_at(data + 1) == b'/') {
                    data += 1;
                }
                if self.byte_at(data) != 0 {
                    data += 2;
                }
            } else {
                break;
            }
        }
        if self.byte_at(data) == b'"' {
            data += 1;
            let mut token = String::new();
            loop {
                let byte = self.byte_at(data);
                data += 1;
                if byte == b'"' || byte == 0 {
                    if token.len() == MAX_TOKEN_CHARS {
                        return Err(TeamArenaSkirmishError::TokenOverflow);
                    }
                    self.offset = if byte == 0 { None } else { Some(data) };
                    return Ok(token);
                }
                if token.len() < MAX_TOKEN_CHARS {
                    token.push(byte as char);
                }
            }
        }
        let mut token = String::new();
        loop {
            if token.len() < MAX_TOKEN_CHARS {
                token.push(self.byte_at(data) as char);
            }
            data += 1;
            if self.signed_byte(data) <= 32 {
                break;
            }
        }
        if token.len() == MAX_TOKEN_CHARS {
            token.clear();
        }
        self.offset = Some(data);
        Ok(token)
    }
}

/// Parse `{ section { row... } ... }` metadata (donor `sections`).
fn parse_sections(text: &str) -> Result<BTreeMap<String, Vec<Vec<String>>>, TeamArenaSkirmishError> {
    let mut parser = ComParse::new(text)?;
    let mut result = BTreeMap::new();
    loop {
        let section = parser.parse()?;
        if section.is_empty() {
            return Ok(result);
        }
        if parser.parse()? != "{" {
            return Err(TeamArenaSkirmishError::BadSection(format!(
                "Invalid Team Arena {section} section"
            )));
        }
        let mut rows = Vec::new();
        loop {
            let token = parser.parse()?;
            if token == "}" {
                break;
            }
            if token != "{" {
                return Err(TeamArenaSkirmishError::BadSection(format!(
                    "Invalid Team Arena {section} row"
                )));
            }
            let mut row = Vec::new();
            loop {
                let field = parser.parse()?;
                if field == "}" {
                    break;
                }
                if field.is_empty() || field == "{" {
                    return Err(TeamArenaSkirmishError::BadSection(format!(
                        "Invalid Team Arena {section} field"
                    )));
                }
                row.push(field);
            }
            rows.push(row);
        }
        result.insert(section.to_lowercase(), rows);
    }
}

fn row_field(row: &[String], index: usize) -> Result<&str, TeamArenaSkirmishError> {
    row.get(index)
        .map(String::as_str)
        .ok_or(TeamArenaSkirmishError::IncompleteRow)
}

fn row_integer(row: &[String], index: usize) -> Result<i64, TeamArenaSkirmishError> {
    let value = row_field(row, index)?;
    let digits = value.strip_prefix('-').unwrap_or(value);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(TeamArenaSkirmishError::BadInteger);
    }
    let number: i64 = value.parse().map_err(|_| TeamArenaSkirmishError::IntegerOutOfRange)?;
    if !(-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(&number) {
        return Err(TeamArenaSkirmishError::IntegerOutOfRange);
    }
    Ok(number)
}

fn member_field(members: &[String], index: usize) -> Result<&str, TeamArenaSkirmishError> {
    members
        .get(index)
        .map(String::as_str)
        .ok_or(TeamArenaSkirmishError::IncompleteRow)
}

/// Selected player/opponent teams.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamArenaTeams {
    /// Player team name.
    pub player: String,
    /// Opponent team name.
    pub opponent: String,
}

/// Default player team (donor `planTeamArenaSkirmish` default).
pub const DEFAULT_SKIRMISH_PLAYER_TEAM: &str = "Pagans";
/// Default opponent team.
pub const DEFAULT_SKIRMISH_OPPONENT_TEAM: &str = "Stroggs";

impl Default for TeamArenaTeams {
    fn default() -> Self {
        Self {
            player: DEFAULT_SKIRMISH_PLAYER_TEAM.to_string(),
            opponent: DEFAULT_SKIRMISH_OPPONENT_TEAM.to_string(),
        }
    }
}

/// Position in the campaign ladder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TeamArenaSkirmishCursor {
    /// Game-type index.
    pub game_type_index: usize,
    /// Map index.
    pub map_index: usize,
}

/// Default cursor (donor `planTeamArenaSkirmish` default).
pub const DEFAULT_SKIRMISH_CURSOR: TeamArenaSkirmishCursor = TeamArenaSkirmishCursor {
    game_type_index: 3,
    map_index: 0,
};
/// Default skill (donor `planTeamArenaSkirmish` default).
pub const DEFAULT_SKIRMISH_SKILL: u8 = 2;

/// One authored campaign map (donor `CampaignMap`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CampaignMap {
    /// Display title.
    pub title: String,
    /// Map name (without `maps/` or `.bsp`).
    pub name: String,
    /// Members per team.
    pub team_members: i64,
    /// Tournament opponent bot.
    pub opponent: String,
    /// Supported game-type numbers.
    pub types: BTreeSet<i64>,
    /// Times to beat per game-type number.
    pub times: BTreeMap<i64, i64>,
}

/// One campaign team roster (donor `teams` map entry, insertion order kept).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CampaignTeam {
    /// Lowercased team name.
    pub name: String,
    /// Member bot names.
    pub members: Vec<String>,
}

/// Authored campaign metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamArenaCampaign {
    /// Game-type numbers.
    pub game_types: Vec<i64>,
    /// Authored maps.
    pub maps: Vec<CampaignMap>,
    /// Team rosters in file order.
    pub teams: Vec<CampaignTeam>,
    /// Bot alias to AI name in file order.
    pub aliases: Vec<(String, String)>,
}

impl TeamArenaCampaign {
    /// Roster for a team name (case-insensitive).
    fn team(&self, name: &str) -> Option<&CampaignTeam> {
        let lowered = name.to_lowercase();
        self.teams.iter().find(|team| team.name == lowered)
    }

    /// AI name for a bot alias (case-insensitive).
    fn alias(&self, member: &str) -> Option<&str> {
        let lowered = member.to_lowercase();
        self.aliases
            .iter()
            .find(|(name, _)| *name == lowered)
            .map(|(_, ai)| ai.as_str())
    }
}

/// Team names in file order.
#[must_use]
pub fn team_arena_team_choices(campaign: &TeamArenaCampaign) -> Vec<String> {
    campaign.teams.iter().map(|team| team.name.clone()).collect()
}

fn valid_map_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'/' | b'-'))
        && !name.split('/').any(|segment| segment == "..")
}

/// Parse campaign metadata (grammar and defaults follow `UI_StartSkirmish`).
pub fn parse_team_arena_campaign(
    game_info: &str,
    team_info: &str,
) -> Result<TeamArenaCampaign, TeamArenaSkirmishError> {
    let game = parse_sections(game_info)?;
    let teams = parse_sections(team_info)?;
    let mut game_types = Vec::new();
    if let Some(rows) = game.get("gametypes") {
        for row in rows {
            game_types.push(row_integer(row, 1)?);
        }
    }
    let mut maps = Vec::new();
    if let Some(rows) = game.get("maps") {
        for row in rows {
            let mut types = BTreeSet::new();
            let mut times = BTreeMap::new();
            let mut index = 4;
            while index < row.len() {
                let game_type = row_integer(row, index)?;
                let time = row_integer(row, index + 1)?;
                types.insert(game_type);
                times.insert(game_type, time);
                index += 2;
            }
            let name = row_field(row, 1)?.to_string();
            let team_members = row_integer(row, 2)?;
            if !valid_map_name(&name) || !(1..=5).contains(&team_members) {
                return Err(TeamArenaSkirmishError::BadMap);
            }
            maps.push(CampaignMap {
                title: row_field(row, 0)?.to_string(),
                name,
                team_members,
                opponent: row_field(row, 3)?.to_string(),
                types,
                times,
            });
        }
    }
    let mut roster = Vec::new();
    if let Some(rows) = teams.get("teams") {
        for row in rows {
            let mut members = Vec::new();
            for index in 2..=6 {
                members.push(row_field(row, index)?.to_string());
            }
            roster.push(CampaignTeam {
                name: row_field(row, 0)?.to_lowercase(),
                members,
            });
        }
    }
    let mut aliases = Vec::new();
    if let Some(rows) = teams.get("aliases") {
        for row in rows {
            aliases.push((row_field(row, 0)?.to_lowercase(), row_field(row, 1)?.to_string()));
        }
    }
    Ok(TeamArenaCampaign {
        game_types,
        maps,
        teams: roster,
        aliases,
    })
}

/// Whether a map is authored for single-player at a game type (donor `active`).
fn map_active(map: &CampaignMap, game_type: i64) -> bool {
    let mapped = if game_type == 2 {
        3
    } else if game_type == 3 {
        0
    } else {
        game_type
    };
    map.types.contains(&2) && map.types.contains(&mapped)
}

/// Advance the campaign cursor to the next authored skirmish.
pub fn next_team_arena_cursor(
    campaign: &TeamArenaCampaign,
    current: TeamArenaSkirmishCursor,
) -> Result<TeamArenaSkirmishCursor, TeamArenaSkirmishError> {
    let Some(game_type) = campaign.game_types.get(current.game_type_index).copied() else {
        return Err(TeamArenaSkirmishError::BadCursor);
    };
    if campaign.maps.get(current.map_index).is_none() {
        return Err(TeamArenaSkirmishError::BadCursor);
    }
    if let Some(next) = campaign
        .maps
        .iter()
        .enumerate()
        .find(|(index, map)| *index > current.map_index && map_active(map, game_type))
        .map(|(index, _)| index)
    {
        return Ok(TeamArenaSkirmishCursor {
            map_index: next,
            ..current
        });
    }
    let incremented = current.game_type_index + 1;
    let game_type_index = if incremented >= campaign.game_types.len() {
        1
    } else if incremented == 2 {
        3
    } else {
        incremented
    };
    let next_type = campaign.game_types.get(game_type_index).copied();
    let map_index = campaign
        .maps
        .iter()
        .position(|map| next_type.is_some_and(|game_type| map_active(map, game_type)));
    match (next_type, map_index) {
        (Some(_), Some(map_index)) => Ok(TeamArenaSkirmishCursor {
            game_type_index,
            map_index,
        }),
        _ => Err(TeamArenaSkirmishError::NoNextMaps),
    }
}

/// Bot team assignment (`""` encodes tournament play).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeamArenaBotTeam {
    /// Red team.
    Red,
    /// Blue team.
    Blue,
    /// No team (tournament).
    Solo,
}

impl TeamArenaBotTeam {
    /// Donor spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Red => "Red",
            Self::Blue => "Blue",
            Self::Solo => "",
        }
    }
}

/// One skirmish bot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamArenaBot {
    /// AI name.
    pub ai: String,
    /// Display name.
    pub name: String,
    /// Team assignment.
    pub team: TeamArenaBotTeam,
    /// Spawn delay in milliseconds.
    pub delay_milliseconds: u64,
}

/// Player team assignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeamArenaPlayerTeam {
    /// Red team.
    Red,
    /// Free (tournament).
    Free,
}

impl TeamArenaPlayerTeam {
    /// Donor spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Red => "Red",
            Self::Free => "free",
        }
    }
}

/// One cvar name/value pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamArenaCvar {
    /// Variable name.
    pub name: String,
    /// Variable value.
    pub value: String,
}

/// Planned single-player skirmish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamArenaSkirmish {
    /// Map resource path.
    pub map: String,
    /// Map title.
    pub title: String,
    /// Game type.
    pub game_type: GameType,
    /// Maximum clients.
    pub max_clients: i64,
    /// Time to beat.
    pub time_to_beat: i64,
    /// Bot skill.
    pub skill: u8,
    /// Source cvars in donor order.
    pub cvars: Vec<TeamArenaCvar>,
    /// Client cvars.
    pub client_cvars: Vec<TeamArenaCvar>,
    /// Bots in spawn order.
    pub bots: Vec<TeamArenaBot>,
    /// Player team.
    pub player_team: TeamArenaPlayerTeam,
    /// Player model.
    pub player_model: String,
    /// Player head model.
    pub player_head_model: String,
    /// Campaign cursor.
    pub cursor: TeamArenaSkirmishCursor,
}

impl TeamArenaSkirmish {
    /// Setup kind (donor `kind: "team-arena"`).
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        "team-arena"
    }
}

fn skirmish_game_type(number: i64) -> Result<GameType, TeamArenaSkirmishError> {
    match number {
        1 => Ok(GameType::GtTournament),
        4 => Ok(GameType::GtCtf),
        5 => Ok(GameType::Gt1fctf),
        6 => Ok(GameType::GtObelisk),
        7 => Ok(GameType::GtHarvester),
        _ => Err(TeamArenaSkirmishError::UnsupportedGameType),
    }
}

/// Plan the skirmish at a cursor for selected teams.
pub fn plan_team_arena_skirmish(
    campaign: &TeamArenaCampaign,
    skill: u8,
    cursor: TeamArenaSkirmishCursor,
    teams: &TeamArenaTeams,
) -> Result<TeamArenaSkirmish, TeamArenaSkirmishError> {
    let number = campaign.game_types.get(cursor.game_type_index).copied().unwrap_or(-1);
    let game_type = skirmish_game_type(number)?;
    let Some(map) = campaign.maps.get(cursor.map_index) else {
        return Err(TeamArenaSkirmishError::MapNotAuthored);
    };
    if !map_active(map, number) {
        return Err(TeamArenaSkirmishError::MapNotAuthored);
    }
    let mut bots = Vec::new();
    if number == 1 {
        bots.push(TeamArenaBot {
            ai: map.opponent.clone(),
            name: map.opponent.clone(),
            team: TeamArenaBotTeam::Solo,
            delay_milliseconds: 500,
        });
    } else {
        for (name, team, count) in [
            (teams.opponent.as_str(), TeamArenaBotTeam::Blue, map.team_members),
            (teams.player.as_str(), TeamArenaBotTeam::Red, map.team_members - 1),
        ] {
            let Some(roster) = campaign.team(name) else {
                return Err(TeamArenaSkirmishError::MissingTeam(name.to_string()));
            };
            for index in 0..count {
                #[allow(clippy::cast_sign_loss)]
                let member = member_field(&roster.members, index as usize)?.to_string();
                let ai = campaign.alias(&member).unwrap_or("James").to_string();
                bots.push(TeamArenaBot {
                    ai,
                    name: member,
                    team,
                    delay_milliseconds: (bots.len() as u64 + 1) * 500,
                });
            }
        }
    }
    let max_clients = if number == 1 { 2 } else { map.team_members * 2 };
    let time_to_beat = map.times.get(&number).copied().unwrap_or(0);
    let map_index = campaign.maps[..cursor.map_index]
        .iter()
        .filter(|row| map_active(row, number))
        .count();
    let capture_limit = if number == 6 {
        "4"
    } else if number == 7 {
        "15"
    } else {
        "5"
    };
    let values = [
        ("nextmap", "teamarena-results"),
        ("ui_teamArenaTimeToBeat", &time_to_beat.to_string()),
        ("g_gametype", &number.to_string()),
        ("ui_singlePlayerActive", "1"),
        ("g_spSkill", &skill.to_string()),
        ("sv_maxclients", &max_clients.to_string()),
        ("g_doWarmup", "1"),
        ("g_warmup", "15"),
        ("sv_pure", "0"),
        ("g_friendlyFire", "0"),
        ("g_redTeam", teams.player.as_str()),
        ("g_blueTeam", teams.opponent.as_str()),
        ("capturelimit", capture_limit),
        ("fraglimit", "10"),
        ("ui_scoreMap", map.title.as_str()),
        ("ui_gameType", &cursor.game_type_index.to_string()),
        ("ui_currentMap", &cursor.map_index.to_string()),
        ("ui_mapIndex", &map_index.to_string()),
        ("ui_teamName", teams.player.as_str()),
        ("ui_opponentName", teams.opponent.as_str()),
    ];
    Ok(TeamArenaSkirmish {
        map: format!("maps/{}.bsp", map.name),
        title: map.title.clone(),
        game_type,
        max_clients,
        time_to_beat,
        skill,
        cvars: values
            .iter()
            .map(|(name, value)| TeamArenaCvar {
                name: (*name).to_string(),
                value: (*value).to_string(),
            })
            .collect(),
        client_cvars: [("cg_cameraOrbit", "0"), ("cg_thirdPerson", "0"), ("cg_drawTimer", "1")]
            .iter()
            .map(|(name, value)| TeamArenaCvar {
                name: (*name).to_string(),
                value: (*value).to_string(),
            })
            .collect(),
        bots,
        player_team: if number == 1 {
            TeamArenaPlayerTeam::Free
        } else {
            TeamArenaPlayerTeam::Red
        },
        player_model: if number == 1 { "sarge" } else { "james" }.to_string(),
        player_head_model: if number == 1 { "sarge" } else { "*james" }.to_string(),
        cursor,
    })
}

/// Plan with donor defaults (skill 2, cursor `{3, 0}`, Pagans/Stroggs).
pub fn plan_default_team_arena_skirmish(
    campaign: &TeamArenaCampaign,
) -> Result<TeamArenaSkirmish, TeamArenaSkirmishError> {
    plan_team_arena_skirmish(
        campaign,
        DEFAULT_SKIRMISH_SKILL,
        DEFAULT_SKIRMISH_CURSOR,
        &TeamArenaTeams::default(),
    )
}

/// Locate the cursor for a live match.
pub fn current_team_arena_cursor(
    campaign: &TeamArenaCampaign,
    map: &str,
    game_type: GameType,
) -> Result<TeamArenaSkirmishCursor, TeamArenaSkirmishError> {
    let number = game_type as i64;
    let game_type_index = campaign.game_types.iter().position(|&entry| entry == number);
    let map_index = campaign
        .maps
        .iter()
        .position(|entry| format!("maps/{}.bsp", entry.name).to_lowercase() == map.to_lowercase());
    match (game_type_index, map_index) {
        (Some(game_type_index), Some(map_index)) => Ok(TeamArenaSkirmishCursor {
            game_type_index,
            map_index,
        }),
        _ => Err(TeamArenaSkirmishError::NoCurrentEntry),
    }
}

/// Mounted-content reads for campaign metadata (donor `MountedContent` subset).
pub trait TeamArenaMounts {
    /// Read a file by resource path.
    fn read(&self, path: &str) -> Result<Vec<u8>, String>;
    /// List files under a directory with an extension.
    fn list_files(&self, directory: &str, extension: &str) -> Result<Vec<String>, String>;
}

/// Catalog file selection (donor `q3TeamArenaCatalogPolicy`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TeamArenaCatalogFiles {
    /// Game metadata file.
    pub game_info: &'static str,
    /// Team metadata file.
    pub team_info: &'static str,
    /// Whether extra `.team` files merge in.
    pub additional_teams: bool,
}

/// Resolve catalog files for a product policy.
#[must_use]
pub const fn team_arena_catalog_files(policy: &Q3ProductPolicy) -> TeamArenaCatalogFiles {
    let demo = match policy {
        Q3ProductPolicy::Retail => false,
        Q3ProductPolicy::PrereleaseDemo { team_arena_ui } => matches!(team_arena_ui, TeamArenaUi::Demo),
        Q3ProductPolicy::PrereleaseTaDemo => true,
    };
    if demo {
        TeamArenaCatalogFiles {
            game_info: "demogameinfo.txt",
            team_info: "demoteaminfo.txt",
            additional_teams: false,
        }
    } else {
        TeamArenaCatalogFiles {
            game_info: "gameinfo.txt",
            team_info: "teaminfo.txt",
            additional_teams: true,
        }
    }
}

/// Read and merge campaign metadata from mounts.
pub fn read_team_arena_campaign<Mounts: TeamArenaMounts + ?Sized>(
    mounts: &Mounts,
    policy: Q3ProductPolicy,
) -> Result<TeamArenaCampaign, TeamArenaSkirmishError> {
    let files = team_arena_catalog_files(&policy);
    let game = mounts.read(files.game_info).map_err(TeamArenaSkirmishError::Mount)?;
    let teams = mounts.read(files.team_info).map_err(TeamArenaSkirmishError::Mount)?;
    let campaign = parse_team_arena_campaign(&latin1_to_string(&game), &latin1_to_string(&teams))?;
    if !files.additional_teams {
        return Ok(campaign);
    }
    let mut merged_teams = campaign.teams.clone();
    let mut aliases = campaign.aliases.clone();
    for path in mounts
        .list_files("scripts", ".team")
        .map_err(TeamArenaSkirmishError::Mount)?
    {
        let bytes = mounts
            .read(&format!("scripts/{path}"))
            .map_err(TeamArenaSkirmishError::Mount)?;
        let extra = parse_team_arena_campaign("", &latin1_to_string(&bytes))?;
        for team in extra.teams {
            if let Some(existing) = merged_teams.iter_mut().find(|entry| entry.name == team.name) {
                *existing = team;
            } else {
                merged_teams.push(team);
            }
        }
        for (name, ai) in extra.aliases {
            if let Some(existing) = aliases.iter_mut().find(|entry| entry.0 == name) {
                *existing = (name, ai);
            } else {
                aliases.push((name, ai));
            }
        }
    }
    Ok(TeamArenaCampaign {
        teams: merged_teams,
        aliases,
        ..campaign
    })
}

/// Mission-pack preparation plus mount opening (unported `prepareQ3ApplicationProduct`).
pub trait TeamArenaProductCatalog {
    /// Mount reader.
    type Mounts: TeamArenaMounts;
    /// Prepare `q3-missionpack` mounts and policy (`None` means no Q3 product).
    fn missionpack_mounts(&self) -> Result<Option<(Self::Mounts, Q3ProductPolicy)>, String>;
}

/// Loaded campaign plus its product policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedTeamArenaCampaign {
    /// Parsed campaign.
    pub campaign: TeamArenaCampaign,
    /// Q3 product policy.
    pub policy: Q3ProductPolicy,
}

/// Load the campaign through catalog preparation.
pub fn load_team_arena_campaign<Catalog: TeamArenaProductCatalog>(
    catalog: &Catalog,
) -> Result<LoadedTeamArenaCampaign, TeamArenaSkirmishError> {
    let Some((mounts, policy)) = catalog.missionpack_mounts().map_err(TeamArenaSkirmishError::Mount)? else {
        return Err(TeamArenaSkirmishError::RequiresProductPolicy);
    };
    Ok(LoadedTeamArenaCampaign {
        campaign: read_team_arena_campaign(&mounts, policy)?,
        policy,
    })
}

/// Previous match for cursor resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviousTeamArenaSkirmish {
    /// Live map path.
    pub map: String,
    /// Live game type.
    pub game_type: GameType,
    /// Advance past the live match (donor `advance`, default true).
    pub advance: bool,
}

/// Explicit campaign source (donor `readTeamArenaSkirmish` `source`).
#[derive(Debug)]
pub struct TeamArenaCampaignSource<'mounts, Mounts: TeamArenaMounts + ?Sized> {
    /// Mount reader.
    pub mounts: &'mounts Mounts,
    /// Q3 product policy.
    pub policy: Q3ProductPolicy,
}

impl<Mounts: TeamArenaMounts + ?Sized> Clone for TeamArenaCampaignSource<'_, Mounts> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<Mounts: TeamArenaMounts + ?Sized> Copy for TeamArenaCampaignSource<'_, Mounts> {}

/// Read the campaign and plan the requested skirmish.
pub fn read_team_arena_skirmish<Catalog: TeamArenaProductCatalog, Mounts: TeamArenaMounts + ?Sized>(
    catalog: &Catalog,
    skill: u8,
    previous: Option<&PreviousTeamArenaSkirmish>,
    teams: Option<&TeamArenaTeams>,
    source: Option<TeamArenaCampaignSource<'_, Mounts>>,
) -> Result<TeamArenaSkirmish, TeamArenaSkirmishError> {
    let campaign = match source {
        Some(source) => read_team_arena_campaign(source.mounts, source.policy)?,
        None => load_team_arena_campaign(catalog)?.campaign,
    };
    let cursor = match previous {
        None => DEFAULT_SKIRMISH_CURSOR,
        Some(previous) => {
            let live = current_team_arena_cursor(&campaign, &previous.map, previous.game_type)?;
            if previous.advance {
                next_team_arena_cursor(&campaign, live)?
            } else {
                live
            }
        }
    };
    let default_teams;
    let teams = match teams {
        Some(teams) => teams,
        None => {
            default_teams = TeamArenaTeams::default();
            &default_teams
        }
    };
    plan_team_arena_skirmish(&campaign, skill, cursor, teams)
}

/// One server cvar plus its saved baseline name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TeamArenaServerOverride {
    /// Live setting name.
    pub name: &'static str,
    /// Saved baseline name.
    pub saved: &'static str,
}

/// Server settings captured before a skirmish (donor `teamArenaServerOverrides`).
pub const TEAM_ARENA_SERVER_OVERRIDES: [TeamArenaServerOverride; 6] = [
    TeamArenaServerOverride {
        name: "capturelimit",
        saved: "ui_saveCaptureLimit",
    },
    TeamArenaServerOverride {
        name: "fraglimit",
        saved: "ui_saveFragLimit",
    },
    TeamArenaServerOverride {
        name: "g_doWarmup",
        saved: "ui_doWarmup",
    },
    TeamArenaServerOverride {
        name: "g_warmup",
        saved: "ui_Warmup",
    },
    TeamArenaServerOverride {
        name: "sv_pure",
        saved: "ui_pure",
    },
    TeamArenaServerOverride {
        name: "g_friendlyFire",
        saved: "ui_friendlyFire",
    },
];

fn baseline_value(baseline: &[TeamArenaCvar], name: &str) -> Option<String> {
    baseline
        .iter()
        .find(|entry| entry.name.to_lowercase() == name.to_lowercase())
        .map(|entry| entry.value.clone())
}

/// Source cvars: baselines plus saved captures plus the setup (donor order).
#[must_use]
pub fn team_arena_source_cvars(setup: &TeamArenaSkirmish, baseline: &[TeamArenaCvar]) -> Vec<TeamArenaCvar> {
    let defaults = q3_game_cvar_definitions(Product::Missionpack);
    let mut out = vec![TeamArenaCvar {
        name: "ui_maxClients".to_string(),
        value: baseline_value(baseline, "sv_maxclients").unwrap_or_else(|| "0".to_string()),
    }];
    for setting in TEAM_ARENA_SERVER_OVERRIDES {
        let value = baseline_value(baseline, setting.name)
            .or_else(|| {
                defaults
                    .iter()
                    .find(|entry| entry.name.to_lowercase() == setting.name.to_lowercase())
                    .map(|entry| entry.value.clone())
            })
            .unwrap_or_else(|| "0".to_string());
        out.push(TeamArenaCvar {
            name: setting.saved.to_string(),
            value,
        });
    }
    out.extend(setup.cvars.iter().cloned());
    out
}

/// Client cvars: timer baseline plus the setup (donor order).
#[must_use]
pub fn team_arena_client_cvars(setup: &TeamArenaSkirmish, baseline: &[TeamArenaCvar]) -> Vec<TeamArenaCvar> {
    let mut out = vec![TeamArenaCvar {
        name: "ui_drawTimer".to_string(),
        value: baseline_value(baseline, "cg_drawtimer").unwrap_or_else(|| "0".to_string()),
    }];
    out.extend(setup.client_cvars.iter().cloned());
    out
}

fn registry_baseline(registry: &CvarRegistry) -> Vec<TeamArenaCvar> {
    registry
        .snapshots(0)
        .iter()
        .map(|entry| TeamArenaCvar {
            name: entry.name.clone(),
            value: entry.value.clone(),
        })
        .collect()
}

/// One launch seat (absorbed donor `{ id, cvars }` row).
pub struct TeamArenaLaunchSeat {
    /// Seat identity.
    pub id: SeatId,
    /// Seat cvars.
    pub cvars: CvarRegistry,
}

struct CapturedLaunchSeat {
    id: SeatId,
    settings: Vec<TeamArenaCvar>,
}

struct CapturedLaunch {
    source: Vec<TeamArenaCvar>,
    seats: Vec<CapturedLaunchSeat>,
}

/// One preparation applied at a world-action boundary and its continuation.
pub struct TeamArenaLaunchOverrides {
    setup: TeamArenaSkirmish,
    captured: Option<CapturedLaunch>,
}

impl TeamArenaLaunchOverrides {
    /// Capture a setup for later application.
    #[must_use]
    pub fn new(setup: TeamArenaSkirmish) -> Self {
        Self { setup, captured: None }
    }

    /// Apply captured source and seat settings (captures on first call).
    pub fn apply(
        &mut self,
        source: &mut CvarRegistry,
        seats: &mut [TeamArenaLaunchSeat],
    ) -> Result<(), TeamArenaSkirmishError> {
        source.apply_latched(None)?;
        if self.captured.is_none() {
            let mut captured_seats = Vec::with_capacity(seats.len());
            for seat in seats.iter() {
                let mut settings = team_arena_client_cvars(&self.setup, &registry_baseline(&seat.cvars));
                for name in ["model", "team_model"] {
                    settings.push(TeamArenaCvar {
                        name: name.to_string(),
                        value: self.setup.player_model.clone(),
                    });
                }
                for name in ["headmodel", "team_headmodel"] {
                    settings.push(TeamArenaCvar {
                        name: name.to_string(),
                        value: self.setup.player_head_model.clone(),
                    });
                }
                captured_seats.push(CapturedLaunchSeat {
                    id: seat.id.clone(),
                    settings,
                });
            }
            let timer = captured_seats
                .first()
                .and_then(|seat| seat.settings.iter().find(|setting| setting.name == "ui_drawTimer"))
                .map_or_else(|| "0".to_string(), |setting| setting.value.clone());
            let mut captured_source = team_arena_source_cvars(&self.setup, &registry_baseline(source));
            captured_source.push(TeamArenaCvar {
                name: "ui_drawTimer".to_string(),
                value: timer,
            });
            self.captured = Some(CapturedLaunch {
                source: captured_source,
                seats: captured_seats,
            });
        }
        let captured = self.captured.as_ref().expect("captured above");
        for setting in &captured.source {
            source.set(&setting.name, &setting.value, true)?;
        }
        for seat in seats.iter_mut() {
            let Some(saved) = captured.seats.iter().find(|saved| saved.id == seat.id) else {
                return Err(TeamArenaSkirmishError::UnknownSeat);
            };
            for setting in &saved.settings {
                seat.cvars.set(&setting.name, &setting.value, true)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use qa_core::cmd::Dialect;
    use qa_core::identity::IdentityOwner;

    use super::*;

    const GAME_INFO: &str = r#"
gametypes
{
  { gt0 1 }
  { gt1 4 }
  { gt2 5 }
  { gt3 6 }
}
maps
{
  { "Arena Gate" tagate 2 Strogg 2 300 1 240 4 300 6 420 }
  { "Power Station" tapower 3 Strogg 2 300 4 300 6 420 }
}
"#;

    const TEAM_INFO: &str = r#"
teams
{
  { Pagans x Bob Alice Eve Mall Hank }
  { Stroggs x Tom Pat Kim Leo Amy }
}
aliases
{
  { Bob Hunter }
  { Tom Slasher }
}
"#;

    fn campaign() -> TeamArenaCampaign {
        parse_team_arena_campaign(GAME_INFO, TEAM_INFO).unwrap()
    }

    #[test]
    fn parses_campaign_metadata() {
        let campaign = campaign();
        assert_eq!(campaign.game_types, vec![1, 4, 5, 6]);
        assert_eq!(campaign.maps.len(), 2);
        assert_eq!(campaign.maps[0].title, "Arena Gate");
        assert_eq!(campaign.maps[0].team_members, 2);
        assert_eq!(campaign.maps[0].times.get(&6), Some(&420));
        assert_eq!(
            team_arena_team_choices(&campaign),
            vec!["pagans".to_string(), "stroggs".to_string()]
        );
        assert_eq!(campaign.aliases.len(), 2);
    }

    #[test]
    fn rejects_bad_metadata() {
        assert!(matches!(
            parse_team_arena_campaign("gametypes { { a } }", ""),
            Err(TeamArenaSkirmishError::IncompleteRow)
        ));
        assert!(matches!(
            parse_team_arena_campaign("gametypes { { a nope } }", ""),
            Err(TeamArenaSkirmishError::BadInteger)
        ));
        assert!(matches!(
            parse_team_arena_campaign("gametypes [", ""),
            Err(TeamArenaSkirmishError::BadSection(_))
        ));
        assert!(matches!(
            parse_team_arena_campaign("maps { { only } }", ""),
            Err(TeamArenaSkirmishError::BadInteger | TeamArenaSkirmishError::IncompleteRow)
        ));
        let bad_map = "maps { { Title bad/name/../x 2 Opp 2 1 } }";
        assert_eq!(
            parse_team_arena_campaign(bad_map, ""),
            Err(TeamArenaSkirmishError::BadMap)
        );
        let bad_members = r#"maps { { Title okmap 9 Opp 2 1 } }"#;
        assert_eq!(
            parse_team_arena_campaign(bad_members, ""),
            Err(TeamArenaSkirmishError::BadMap)
        );
        assert_eq!(
            parse_team_arena_campaign("gametypes { { a 99999999999999999999999 } }", ""),
            Err(TeamArenaSkirmishError::IntegerOutOfRange)
        );
    }

    #[test]
    fn plans_tournament_skirmish() {
        let campaign = campaign();
        let setup = plan_team_arena_skirmish(
            &campaign,
            3,
            TeamArenaSkirmishCursor {
                game_type_index: 0,
                map_index: 0,
            },
            &TeamArenaTeams::default(),
        )
        .unwrap();
        assert_eq!(setup.kind(), "team-arena");
        assert_eq!(setup.map, "maps/tagate.bsp");
        assert_eq!(setup.game_type, GameType::GtTournament);
        assert_eq!(setup.max_clients, 2);
        assert_eq!(setup.time_to_beat, 240);
        assert_eq!(
            setup.bots,
            vec![TeamArenaBot {
                ai: "Strogg".to_string(),
                name: "Strogg".to_string(),
                team: TeamArenaBotTeam::Solo,
                delay_milliseconds: 500,
            }]
        );
        assert_eq!(setup.player_team, TeamArenaPlayerTeam::Free);
        assert_eq!(setup.player_model, "sarge");
        assert_eq!(setup.player_head_model, "sarge");
        let get = |name: &str| {
            setup
                .cvars
                .iter()
                .find(|entry| entry.name == name)
                .unwrap()
                .value
                .clone()
        };
        assert_eq!(setup.cvars[0].name, "nextmap");
        assert_eq!(get("g_gametype"), "1");
        assert_eq!(get("g_spSkill"), "3");
        assert_eq!(get("capturelimit"), "5");
        assert_eq!(get("ui_gameType"), "0");
        assert_eq!(get("ui_currentMap"), "0");
        assert_eq!(get("ui_mapIndex"), "0");
        assert_eq!(get("ui_teamName"), "Pagans");
        assert_eq!(setup.client_cvars.len(), 3);
    }

    #[test]
    fn plans_team_skirmish_with_rosters() {
        let campaign = campaign();
        let setup = plan_team_arena_skirmish(
            &campaign,
            2,
            TeamArenaSkirmishCursor {
                game_type_index: 1,
                map_index: 1,
            },
            &TeamArenaTeams::default(),
        )
        .unwrap();
        assert_eq!(setup.game_type, GameType::GtCtf);
        assert_eq!(setup.max_clients, 6);
        assert_eq!(setup.bots.len(), 5);
        assert_eq!(setup.bots[0].team, TeamArenaBotTeam::Blue);
        assert_eq!(setup.bots[0].ai, "Slasher");
        assert_eq!(setup.bots[0].delay_milliseconds, 500);
        assert_eq!(setup.bots[3].team, TeamArenaBotTeam::Red);
        assert_eq!(setup.bots[3].ai, "Hunter");
        assert_eq!(setup.bots[3].delay_milliseconds, 2000);
        assert_eq!(setup.bots[4].ai, "James");
        assert_eq!(setup.player_team, TeamArenaPlayerTeam::Red);
        assert_eq!(setup.player_model, "james");
        assert_eq!(setup.player_head_model, "*james");
        let get = |name: &str| {
            setup
                .cvars
                .iter()
                .find(|entry| entry.name == name)
                .unwrap()
                .value
                .clone()
        };
        assert_eq!(get("ui_mapIndex"), "1");
        assert_eq!(get("ui_scoreMap"), "Power Station");
    }

    #[test]
    fn plan_rejects_bad_selections() {
        let campaign = campaign();
        assert_eq!(
            plan_team_arena_skirmish(
                &campaign,
                2,
                TeamArenaSkirmishCursor {
                    game_type_index: 1,
                    map_index: 0
                },
                &TeamArenaTeams {
                    player: "Pagans".to_string(),
                    opponent: "Nope".to_string()
                },
            ),
            Err(TeamArenaSkirmishError::MissingTeam("Nope".to_string()))
        );
        assert_eq!(
            plan_team_arena_skirmish(
                &campaign,
                2,
                TeamArenaSkirmishCursor {
                    game_type_index: 0,
                    map_index: 1
                },
                &TeamArenaTeams::default(),
            ),
            Err(TeamArenaSkirmishError::MapNotAuthored)
        );
        assert_eq!(
            plan_team_arena_skirmish(
                &campaign,
                2,
                TeamArenaSkirmishCursor {
                    game_type_index: 9,
                    map_index: 0
                },
                &TeamArenaTeams::default(),
            ),
            Err(TeamArenaSkirmishError::UnsupportedGameType)
        );
    }

    #[test]
    fn advances_and_wraps_cursors() {
        let campaign = campaign();
        let at = |game_type_index: usize, map_index: usize| TeamArenaSkirmishCursor {
            game_type_index,
            map_index,
        };
        assert_eq!(next_team_arena_cursor(&campaign, at(0, 0)).unwrap(), at(1, 0));
        assert_eq!(next_team_arena_cursor(&campaign, at(1, 0)).unwrap(), at(1, 1));
        assert_eq!(next_team_arena_cursor(&campaign, at(1, 1)).unwrap(), at(3, 0));
        assert_eq!(next_team_arena_cursor(&campaign, at(3, 1)).unwrap(), at(1, 0));
        assert_eq!(
            next_team_arena_cursor(&campaign, at(9, 0)),
            Err(TeamArenaSkirmishError::BadCursor)
        );
        assert_eq!(
            current_team_arena_cursor(&campaign, "maps/tapower.bsp", GameType::GtCtf).unwrap(),
            at(1, 1)
        );
        assert_eq!(
            current_team_arena_cursor(&campaign, "MAPS/TAGATE.BSP", GameType::GtTournament).unwrap(),
            at(0, 0)
        );
        assert_eq!(
            current_team_arena_cursor(&campaign, "maps/nope.bsp", GameType::GtCtf),
            Err(TeamArenaSkirmishError::NoCurrentEntry)
        );
    }

    #[test]
    fn plans_with_defaults() {
        let campaign = campaign();
        let setup = plan_default_team_arena_skirmish(&campaign).unwrap();
        assert_eq!(setup.skill, 2);
        assert_eq!(setup.game_type, GameType::GtObelisk);
        assert_eq!(setup.cursor, DEFAULT_SKIRMISH_CURSOR);
    }

    struct FakeMounts {
        files: HashMap<String, Vec<u8>>,
        teams: Vec<String>,
    }

    impl TeamArenaMounts for FakeMounts {
        fn read(&self, path: &str) -> Result<Vec<u8>, String> {
            self.files.get(path).cloned().ok_or_else(|| format!("missing {path}"))
        }

        fn list_files(&self, directory: &str, extension: &str) -> Result<Vec<String>, String> {
            assert_eq!((directory, extension), ("scripts", ".team"));
            Ok(self.teams.clone())
        }
    }

    fn fake_mounts() -> FakeMounts {
        FakeMounts {
            files: HashMap::from([
                ("gameinfo.txt".to_string(), GAME_INFO.as_bytes().to_vec()),
                ("teaminfo.txt".to_string(), TEAM_INFO.as_bytes().to_vec()),
                (
                    "scripts/extra.team".to_string(),
                    b"teams { { Pagans x Zed Ted Ned Red Fred } }".to_vec(),
                ),
            ]),
            teams: vec!["extra.team".to_string()],
        }
    }

    #[test]
    fn reads_and_merges_campaign() {
        let campaign = read_team_arena_campaign(&fake_mounts(), Q3ProductPolicy::Retail).unwrap();
        let pagans = campaign.team("PAGANS").unwrap();
        assert_eq!(pagans.members[0], "Zed");
        assert_eq!(
            team_arena_catalog_files(&Q3ProductPolicy::Retail).game_info,
            "gameinfo.txt"
        );
        let demo = team_arena_catalog_files(&Q3ProductPolicy::PrereleaseTaDemo);
        assert_eq!((demo.game_info, demo.additional_teams), ("demogameinfo.txt", false));
        let err = read_team_arena_campaign(&fake_mounts(), Q3ProductPolicy::PrereleaseTaDemo).unwrap_err();
        assert!(matches!(err, TeamArenaSkirmishError::Mount(_)));
    }

    struct FakeCatalog {
        mounts: Option<(FakeMounts, Q3ProductPolicy)>,
    }

    impl TeamArenaProductCatalog for FakeCatalog {
        type Mounts = FakeMounts;

        fn missionpack_mounts(&self) -> Result<Option<(FakeMounts, Q3ProductPolicy)>, String> {
            Ok(self.mounts.as_ref().map(|_| (fake_mounts(), Q3ProductPolicy::Retail)))
        }
    }

    #[test]
    fn loads_campaign_through_catalog() {
        let loaded = load_team_arena_campaign(&FakeCatalog {
            mounts: Some((fake_mounts(), Q3ProductPolicy::Retail)),
        })
        .unwrap();
        assert_eq!(loaded.policy, Q3ProductPolicy::Retail);
        assert_eq!(loaded.campaign.game_types.len(), 4);
        assert_eq!(
            load_team_arena_campaign(&FakeCatalog { mounts: None }),
            Err(TeamArenaSkirmishError::RequiresProductPolicy)
        );
    }

    #[test]
    fn reads_skirmish_with_previous_and_source() {
        let catalog = FakeCatalog {
            mounts: Some((fake_mounts(), Q3ProductPolicy::Retail)),
        };
        let mounts = fake_mounts();
        let source = TeamArenaCampaignSource {
            mounts: &mounts,
            policy: Q3ProductPolicy::Retail,
        };
        let setup = read_team_arena_skirmish::<FakeCatalog, FakeMounts>(
            &catalog,
            4,
            Some(&PreviousTeamArenaSkirmish {
                map: "maps/tagate.bsp".to_string(),
                game_type: GameType::GtTournament,
                advance: true,
            }),
            None,
            Some(source),
        )
        .unwrap();
        assert_eq!(setup.skill, 4);
        assert_eq!(
            setup.cursor,
            TeamArenaSkirmishCursor {
                game_type_index: 1,
                map_index: 0
            }
        );
        let setup = read_team_arena_skirmish::<FakeCatalog, FakeMounts>(
            &catalog,
            2,
            Some(&PreviousTeamArenaSkirmish {
                map: "maps/tagate.bsp".to_string(),
                game_type: GameType::GtTournament,
                advance: false,
            }),
            None,
            Some(source),
        )
        .unwrap();
        assert_eq!(
            setup.cursor,
            TeamArenaSkirmishCursor {
                game_type_index: 0,
                map_index: 0
            }
        );
        let setup = read_team_arena_skirmish::<FakeCatalog, FakeMounts>(&catalog, 2, None, None, Some(source)).unwrap();
        assert_eq!(setup.cursor, DEFAULT_SKIRMISH_CURSOR);
    }

    #[test]
    fn source_and_client_cvars_use_baselines() {
        let campaign = campaign();
        let setup = plan_default_team_arena_skirmish(&campaign).unwrap();
        let baseline = vec![
            TeamArenaCvar {
                name: "sv_maxclients".to_string(),
                value: "12".to_string(),
            },
            TeamArenaCvar {
                name: "CAPTURELIMIT".to_string(),
                value: "9".to_string(),
            },
            TeamArenaCvar {
                name: "cg_drawTimer".to_string(),
                value: "0".to_string(),
            },
        ];
        let source = team_arena_source_cvars(&setup, &baseline);
        assert_eq!(
            source[0],
            TeamArenaCvar {
                name: "ui_maxClients".to_string(),
                value: "12".to_string()
            }
        );
        assert_eq!(
            source[1],
            TeamArenaCvar {
                name: "ui_saveCaptureLimit".to_string(),
                value: "9".to_string()
            }
        );
        assert_eq!(source[7].name, "nextmap");
        let client = team_arena_client_cvars(&setup, &baseline);
        assert_eq!(client[0].value, "0");
        assert_eq!(client[1].name, "cg_cameraOrbit");
        let empty = team_arena_client_cvars(&setup, &[]);
        assert_eq!(empty[0].value, "0");
    }

    fn registries() -> (CvarRegistry, CvarRegistry) {
        let mut source = CvarRegistry::new(Dialect::Q3);
        source
            .register("sv_maxclients", "8", qa_core::cvar::flags::ARCHIVE)
            .unwrap();
        let mut seat = CvarRegistry::new(Dialect::Q3);
        seat.register("cg_drawTimer", "0", qa_core::cvar::flags::ARCHIVE)
            .unwrap();
        (source, seat)
    }

    #[test]
    fn launch_overrides_capture_once_and_apply() {
        let campaign = campaign();
        let setup = plan_default_team_arena_skirmish(&campaign).unwrap();
        let owner = IdentityOwner::create("test").unwrap();
        let (mut source, seat_cvars) = registries();
        let mut seats = vec![TeamArenaLaunchSeat {
            id: owner.seat(0),
            cvars: seat_cvars,
        }];
        let mut launch = TeamArenaLaunchOverrides::new(setup);
        launch.apply(&mut source, &mut seats).unwrap();
        assert_eq!(source.variable_string("g_gametype"), "6");
        assert_eq!(source.variable_string("ui_maxClients"), "8");
        assert_eq!(source.variable_string("ui_drawTimer"), "0");
        assert_eq!(seats[0].cvars.variable_string("model"), "james");
        assert_eq!(seats[0].cvars.variable_string("team_headmodel"), "*james");
        assert_eq!(seats[0].cvars.variable_string("cg_drawTimer"), "1");
        launch.apply(&mut source, &mut seats).unwrap();
        assert_eq!(source.variable_string("g_gametype"), "6");
        let mut strangers = vec![TeamArenaLaunchSeat {
            id: owner.seat(1),
            cvars: CvarRegistry::new(Dialect::Q3),
        }];
        assert_eq!(
            launch.apply(&mut source, &mut strangers),
            Err(TeamArenaSkirmishError::UnknownSeat)
        );
    }
}
