//! Quake III presentation: hud corners.
//!
//! Donor provenance: `src/content/q3/presentation/hud-corners.ts`.

use qa_core::math::{vec3, vec4};
use std::cell::{Cell, RefCell};

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::presentation::client_info::*;
use crate::q3::presentation::draw_icons::*;
use crate::q3::presentation::draw_tools::*;
use crate::q3::presentation::mirrors_present_hud::*;

/// Corner icon size.
pub(crate) const CORNER_ICON_SIZE: f32 = 48.0;

/// Corner field digit size.
pub(crate) const CORNER_CHAR_WIDTH: f32 = 32.0;

pub(crate) const CORNER_CHAR_HEIGHT: f32 = 48.0;

/// Corner big character size.
pub(crate) const CORNER_BIGCHAR_WIDTH: i32 = 16;

pub(crate) const CORNER_BIGCHAR_HEIGHT: f32 = 16.0;

/// Corner tiny character size.
pub(crate) const CORNER_TINYCHAR_WIDTH: i32 = 8;

pub(crate) const CORNER_TINYCHAR_HEIGHT: i32 = 8;

/// Maximum team overlay players.
pub(crate) const MAX_TEAM_OVERLAY_PLAYERS: i32 = 8;

/// Team overlay name width.
pub(crate) const TEAM_OVERLAY_MAXNAME_WIDTH: i32 = 12;

/// Team overlay location width.
pub(crate) const TEAM_OVERLAY_MAXLOCATION_WIDTH: i32 = 16;

/// Maximum locations.
pub(crate) const MAX_LOCATIONS: i32 = 64;

/// Team chat height.
pub(crate) const TEAMCHAT_HEIGHT: i32 = 8;

/// Missing score.
pub(crate) const SCORE_NOT_PRESENT: i32 = -9999;

/// Players configstring base.
pub const CS_PLAYERS: usize = 544;

/// Locations configstring base.
pub const CS_LOCATIONS: usize = 608;

/// Attacker head time.
pub(crate) const ATTACKER_HEAD_TIME: i32 = 10_000;

/// Powerup blinks.
pub(crate) const POWERUP_BLINKS: i32 = 5;

/// Powerup blink time.
pub(crate) const POWERUP_BLINK_TIME: i32 = 1_000;

/// Pulse time.
pub(crate) const PULSE_TIME: i32 = 200;

/// Pulse scale.
pub(crate) const PULSE_SCALE: f32 = 1.5;

/// FPS frames.
pub(crate) const FPS_FRAMES: usize = 4;

/// Powerup draw order (`POWERUPS`).
pub(crate) const CORNER_POWERUPS: [Powerup; 16] = [
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
        if !(0..64).contains(&client_num) || client_num == snapshot.player_state.client_num {
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
            if !(0..=999_000).contains(&remaining) {
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
