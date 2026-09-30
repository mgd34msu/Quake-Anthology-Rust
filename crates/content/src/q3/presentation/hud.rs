//! Quake III presentation: hud.
//!
//! Donor provenance: `src/content/q3/presentation/hud.ts`.

use qa_core::math::{vec3, vec4, Vec4};
use std::cell::Cell;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::presentation::client_info::*;
use crate::q3::presentation::draw_icons::*;
use crate::q3::presentation::draw_status::*;
use crate::q3::presentation::draw_tools::*;
use crate::q3::presentation::hud_corners::*;
use crate::q3::presentation::mirrors_present_hud::*;
use crate::q3::presentation::mission_hud::*;
use crate::q3::presentation::scoreboard::*;

/// HUD icon size.
pub(crate) const HUD_ICON: f32 = 48.0;

/// Head damage time.
pub(crate) const HUD_DAMAGE_TIME: f32 = 500.0;

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
    #[allow(clippy::too_many_arguments)]
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
