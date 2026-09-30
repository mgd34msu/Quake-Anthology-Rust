//! Quake III presentation: mission owner draw.
//!
//! Donor provenance: `src/content/q3/presentation/mission-owner-draw.ts`.

use qa_core::math::{vec3, vec4, Vec4};
use std::rc::Rc;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::shared::definitions::Team as CanonicalTeam;
use crate::q3::presentation::client_info::*;
use crate::q3::presentation::config::*;
use crate::q3::presentation::draw_icons::*;
use crate::q3::presentation::draw_tools::*;
use crate::q3::presentation::hud_corners::*;
use crate::q3::presentation::mirrors_present_hud::*;

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
    #[allow(clippy::too_many_arguments)]
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
            let wanted_team = if blue {
                CanonicalTeam::TeamRed
            } else {
                CanonicalTeam::TeamBlue
            };
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
    #[allow(clippy::too_many_arguments)]
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
