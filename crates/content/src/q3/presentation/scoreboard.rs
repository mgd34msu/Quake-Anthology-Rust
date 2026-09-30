//! Quake III presentation: scoreboard.
//!
//! Donor provenance: `src/content/q3/presentation/scoreboard.ts`.

use qa_core::math::{vec3, vec4, Vec4};
use std::cell::Cell;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::format::{game_format_bounded, GameFormatArgument};
use crate::q3::base::shared::definitions::*;
use crate::q3::base::shared::player_state::*;
use crate::q3::presentation::client_info::*;
use crate::q3::presentation::config::{HudConfigStrings, HudCvarReader};
use crate::q3::presentation::console::HudCommands;
use crate::q3::presentation::draw_icons::*;
use crate::q3::presentation::draw_tools::*;
use crate::q3::presentation::events::place_string;
use crate::q3::presentation::hud::{same, Shared};
use crate::q3::presentation::state::*;

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
            if host.clients.borrow().client_info(index).borrow().clone()
                != static_state.borrow().client_info[index as usize]
            {
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
    fn client(&self, index: i32) -> ClientInfo {
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
            let text = game_format_bounded(
                "Bad score->client: %i\n",
                &[GameFormatArgument::from(score.client)],
                1024,
            );
            self.host.commands.borrow_mut().print(&text);
            return;
        }
        let client_handle = self.client(score.client);
        let client = client_handle;
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
        if client.powerups & (1 << Powerup::PwNeutralflag as i32) != 0 {
            icons.draw_flag_model(icon_rect, Team::TeamFree as i32, false);
        } else if client.powerups & (1 << Powerup::PwRedflag as i32) != 0 {
            icons.draw_flag_model(icon_rect, Team::TeamRed as i32, false);
        } else if client.powerups & (1 << Powerup::PwBlueflag as i32) != 0 {
            icons.draw_flag_model(icon_rect, Team::TeamBlue as i32, false);
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
                    (if game_type == GameType::GtTournament {
                        y - 8.0
                    } else {
                        y
                    }) as i32,
                    &game_format_bounded("%i", &[GameFormatArgument::from(client.handicap)], 1024),
                    color,
                );
            }
            if self.static_state.borrow().game_type == GameType::GtTournament {
                tools.draw_small_string_color(
                    icon_x as i32,
                    (if client.handicap < 100 && client.bot_skill == 0 {
                        y + 8.0
                    } else {
                        y
                    }) as i32,
                    &game_format_bounded(
                        "%i/%i",
                        &[
                            GameFormatArgument::from(client.wins),
                            GameFormatArgument::from(client.losses),
                        ],
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
            game_format_bounded(
                " connecting    %s",
                &[GameFormatArgument::from(client.name.clone())],
                1024,
            )
        } else if client.team == Team::TeamSpectator {
            game_format_bounded(
                " SPECT %3i %4i %s",
                &[
                    GameFormatArgument::from(score.ping),
                    GameFormatArgument::from(score.time),
                    GameFormatArgument::from(client.name.clone()),
                ],
                1024,
            )
        } else {
            game_format_bounded(
                "%5i %4i %4i %s",
                &[
                    GameFormatArgument::from(score.score),
                    GameFormatArgument::from(score.ping),
                    GameFormatArgument::from(score.time),
                    GameFormatArgument::from(client.name.clone()),
                ],
                1024,
            )
        };
        drop(client);
        let ps = self.snapshot();
        if score.client == ps.client_num {
            self.local_client.set(true);
            let rank = if ps.persistant.get(PersistentIndex::PersTeam as usize) == Team::TeamSpectator as i32
                || (self.static_state.borrow().game_type as i32) >= (GameType::GtTeam as i32)
            {
                -1
            } else {
                ps.persistant.get(PersistentIndex::PersRank as usize) & !0x4000
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
        let schema = stat_schema(ps.product());
        if ps.stats.get(schema.clients_ready()) & (1 << score.client) != 0 {
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
            if self.client(score.client).team != team {
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
        if game_type == GameType::GtSinglePlayer && pm_type == MoveType::PmIntermission as i32 {
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
        let color = if show_scores || pm_type == MoveType::PmDead as i32 || pm_type == MoveType::PmIntermission as i32 {
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
            let text = game_format_bounded("Fragged by %s", &[GameFormatArgument::from(killer)], 1024);
            tools.draw_big_string((640 - draw_strlen(&text) * 16) / 2, 40, &text, fade);
        }
        let mut rank_text: Option<String> = None;
        if (game_type as i32) < (GameType::GtTeam as i32) {
            if ps.persistant.get(PersistentIndex::PersTeam as usize) != Team::TeamSpectator as i32 {
                rank_text = Some(game_format_bounded(
                    "%s place with %i",
                    &[
                        GameFormatArgument::from(place_string(
                            ps.persistant.get(PersistentIndex::PersRank as usize).wrapping_add(1),
                        )),
                        GameFormatArgument::from(ps.persistant.get(PersistentIndex::PersScore as usize)),
                    ],
                    1024,
                ));
            }
        } else {
            let team_scores = self.state.borrow().team_scores;
            rank_text = Some(if team_scores[0] == team_scores[1] {
                game_format_bounded(
                    "Teams are tied at %i",
                    &[GameFormatArgument::from(team_scores[0])],
                    1024,
                )
            } else if team_scores[0] >= team_scores[1] {
                game_format_bounded(
                    "Red leads %i to %i",
                    &[
                        GameFormatArgument::from(team_scores[0]),
                        GameFormatArgument::from(team_scores[1]),
                    ],
                    1024,
                )
            } else {
                game_format_bounded(
                    "Blue leads %i to %i",
                    &[
                        GameFormatArgument::from(team_scores[1]),
                        GameFormatArgument::from(team_scores[0]),
                    ],
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
        if (game_type as i32) >= (GameType::GtTeam as i32) {
            y += line_height / 2.0;
            let team_scores = self.state.borrow().team_scores;
            let first_team = if team_scores[0] >= team_scores[1] {
                Team::TeamRed
            } else {
                Team::TeamBlue
            };
            let first = self.team_scoreboard(y, first_team, fade, max_clients, line_height);
            self.host.icons.borrow().draw_team_background(
                rect2d(0.0, y - top_border, 640.0, first as f32 * line_height + 16.0),
                0.33,
                first_team as i32,
            );
            y += first as f32 * line_height + 16.0;
            max_clients -= first;
            let second_team = if first_team == Team::TeamRed {
                Team::TeamBlue
            } else {
                Team::TeamRed
            };
            let second = self.team_scoreboard(y, second_team, fade, max_clients, line_height);
            self.host.icons.borrow().draw_team_background(
                rect2d(0.0, y - top_border, 640.0, second as f32 * line_height + 16.0),
                0.33,
                second_team as i32,
            );
            y += second as f32 * line_height + 16.0;
            max_clients -= second;
            y += self.team_scoreboard(y, Team::TeamSpectator, fade, max_clients, line_height) as f32 * line_height
                + 16.0;
        } else {
            let count = self.team_scoreboard(y, Team::TeamFree, fade, max_clients, line_height);
            y += count as f32 * line_height + 16.0;
            y += self.team_scoreboard(y, Team::TeamSpectator, fade, max_clients - count, line_height) as f32
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
            &game_format_bounded(
                "%i:%i%i",
                &[
                    GameFormatArgument::from(minutes),
                    GameFormatArgument::from(seconds / 10),
                    GameFormatArgument::from(seconds % 10),
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
            let text = game_format_bounded("%i", &[GameFormatArgument::from(score)], 1024);
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
        if (self.static_state.borrow().game_type as i32) >= (GameType::GtTeam as i32) {
            let team_scores = self.state.borrow().team_scores;
            line(160, "Red Team", team_scores[0]);
            line(224, "Blue Team", team_scores[1]);
        } else {
            let mut y = 160;
            for index in 0..64 {
                let client = self.client(index);
                if !client.info_valid || client.team != Team::TeamFree {
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

/// Player presenter (`PlayerPresenter`, used surface).
pub trait PlayerPresenter {
    /// Canonical frame state.
    fn state_handle(&self) -> Shared<ClientGameState>;
    /// Reset a player entity.
    fn reset_player_entity(&mut self, entity: &mut ClientEntity);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q3::base::shared::definitions::Product;
    use crate::q3::base::shared::player_state::PlayerState;
    use crate::q3::presentation::hud::shared;
    use crate::q3::presentation::hud::tests::*;
    use crate::q3::presentation::retail_snapshot::Snapshot;

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
            message_number: 0,
            server_time: 100,
            delta_number: 0,
            flags: 0,
            server_command_number: 0,
            parse_entities_number: 0,
            area_mask: [0; 32],
            player_state: PlayerState::new(Product::Baseq3, None),
            entities: Vec::new(),
        });
        game.state.borrow_mut().show_scores = true;
        assert!(board.draw());
        assert!(!game.sink.borrow().blits.is_empty());
        game.state.borrow_mut().time = 5000;
        board.draw_tourney();
        assert!(game.commands.borrow().client.contains(&"score".to_string()));
    }
}
