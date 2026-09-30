//! Quake III presentation: scoreboard.
//!
//! Donor provenance: `src/content/q3/presentation/scoreboard.ts`.

use qa_core::math::{vec3, vec4, Vec4};
use std::cell::Cell;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::shared::definitions::Team as CanonicalTeam;
use crate::q3::presentation::client_info::*;
use crate::q3::presentation::draw_icons::*;
use crate::q3::presentation::draw_tools::*;
use crate::q3::presentation::mirrors_present_hud::*;

/// Scoreboard header Y.
pub(crate) const SCOREBOARD_HEADER: f32 = 86.0;

/// Scoreboard top Y.
pub(crate) const SCOREBOARD_TOP: f32 = 118.0;

/// Normal row height.
pub(crate) const NORMAL_HEIGHT: f32 = 40.0;

/// Intermission row height.
pub(crate) const INTER_HEIGHT: f32 = 16.0;

/// Maximum normal rows.
pub(crate) const MAX_NORMAL: i32 = 7;

/// Maximum intermission rows.
pub(crate) const MAX_INTER: i32 = 17;

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
        } else if client.team == CanonicalTeam::TeamSpectator {
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
    fn team_scoreboard(&self, y: f32, team: CanonicalTeam, fade: f32, max_clients: i32, line_height: f32) -> i32 {
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
                CanonicalTeam::TeamRed
            } else {
                CanonicalTeam::TeamBlue
            };
            let first = self.team_scoreboard(y, first_team, fade, max_clients, line_height);
            self.host.icons.borrow().draw_team_background(
                rect2d(0.0, y - top_border, 640.0, first as f32 * line_height + 16.0),
                0.33,
                first_team as i32,
            );
            y += first as f32 * line_height + 16.0;
            max_clients -= first;
            let second_team = if first_team == CanonicalTeam::TeamRed {
                CanonicalTeam::TeamBlue
            } else {
                CanonicalTeam::TeamRed
            };
            let second = self.team_scoreboard(y, second_team, fade, max_clients, line_height);
            self.host.icons.borrow().draw_team_background(
                rect2d(0.0, y - top_border, 640.0, second as f32 * line_height + 16.0),
                0.33,
                second_team as i32,
            );
            y += second as f32 * line_height + 16.0;
            max_clients -= second;
            y += self.team_scoreboard(y, CanonicalTeam::TeamSpectator, fade, max_clients, line_height) as f32
                * line_height
                + 16.0;
        } else {
            let count = self.team_scoreboard(y, CanonicalTeam::TeamFree, fade, max_clients, line_height);
            y += count as f32 * line_height + 16.0;
            y += self.team_scoreboard(y, CanonicalTeam::TeamSpectator, fade, max_clients - count, line_height) as f32
                * line_height
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
                if !client.info_valid || client.team != CanonicalTeam::TeamFree {
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
