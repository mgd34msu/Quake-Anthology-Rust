//! Quake III presentation: hud.
//!
//! Donor provenance: `src/content/q3/presentation/hud.ts`.

use qa_core::math::{vec3, vec4, Vec3, Vec4};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::format::{game_format_bounded, GameFormatArgument};
use crate::q3::base::game::numeric::GameRandom;
use crate::q3::base::shared::definitions::*;
use crate::q3::base::shared::player_state::*;
use crate::q3::presentation::audio::ClientSoundBank;
use crate::q3::presentation::client::HudLocalSound;
use crate::q3::presentation::client_info::*;
use crate::q3::presentation::config::HudCvarReader;
use crate::q3::presentation::draw_icons::*;
use crate::q3::presentation::draw_status::*;
use crate::q3::presentation::draw_tools::*;
use crate::q3::presentation::hud_corners::*;
use crate::q3::presentation::mission_hud::*;
use crate::q3::presentation::player_state::WeaponHudReader;
use crate::q3::presentation::resources::RendererResources;
use crate::q3::presentation::retail_snapshot::{PcmSound, SceneModel, SceneShader};
use crate::q3::presentation::scoreboard::*;
use crate::q3::presentation::state::*;

/// Shared handle (`Shared`: `Rc<RefCell<T>>`).
pub type Shared<T> = Rc<RefCell<T>>;

/// Build a [`Shared`] handle.
pub fn shared<T>(value: T) -> Shared<T> {
    Rc::new(RefCell::new(value))
}

/// Pointer identity between two [`Shared`] handles.
pub fn same<T: ?Sized>(a: &Shared<T>, b: &Shared<T>) -> bool {
    Rc::ptr_eq(a, b)
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
    /// Graphics.
    pub graphics: ClientGraphics,
    /// Sounds.
    pub sounds: ClientSounds,
}

impl ClientMedia {
    /// Assemble media, checking the product.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        product: Product,
        static_state: Shared<ClientGameStaticState>,
        resources: Shared<dyn RendererResources>,
        sound_bank: Shared<dyn ClientSoundBank>,
        weapon_registry: Shared<dyn WeaponRegistryService>,
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
            graphics,
            sounds,
        }
    }
}

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
    fn client(&self, index: i32) -> ClientInfo {
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
        if damage_time != 0.0 && (time as f32 - damage_time) < HUD_DAMAGE_TIME {
            let frac = (time as f32 - damage_time) / HUD_DAMAGE_TIME;
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
        let schema = stat_schema(ps.product());
        tools.draw.set_color(None);
        icons.draw_team_background(
            rect2d(0.0, 420.0, 640.0, 60.0),
            0.33,
            ps.persistant.get(PersistentIndex::PersTeam as usize),
        );
        let weapon = self
            .state
            .borrow()
            .entity_at(ps.client_num)
            .map(|entity| entity.current_state.weapon)
            .unwrap_or(0);
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
        if predicted.powerups.get(Powerup::PwRedflag as usize) != 0 {
            self.draw_status_bar_flag(333.0, Team::TeamRed as i32);
        } else if predicted.powerups.get(Powerup::PwBlueflag as usize) != 0 {
            self.draw_status_bar_flag(333.0, Team::TeamBlue as i32);
        } else if predicted.powerups.get(Powerup::PwNeutralflag as usize) != 0 {
            self.draw_status_bar_flag(333.0, Team::TeamFree as i32);
        }
        let icons = self.host.icons.borrow();
        let tools = icons.tools.clone();
        if ps.stats.get(schema.armor()) != 0 {
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
            let ammo = ps.ammo.get(weapon as usize);
            if ammo > -1 {
                let firing = vec4(0.5, 0.5, 0.5, 1.0);
                let normal = vec4(1.0, 0.69, 0.0, 1.0);
                tools.draw.set_color(Some(
                    if predicted.weapon_state == WeaponState::WeaponFiring as i32 && predicted.weapon_time > 100 {
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
        let health = ps.stats.get(schema.health());
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
        let armor = ps.stats.get(schema.armor());
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
            .get(stat_schema(self.state.borrow().product).holdable_item());
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
            let text = game_format_bounded("%d", &[GameFormatArgument::from(reward.count)], 32);
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
            || self.snapshot().persistant.get(PersistentIndex::PersTeam as usize) == Team::TeamSpectator as i32
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
        let picture = tools
            .media
            .borrow()
            .resources
            .borrow()
            .picture(shader.as_ref())
            .map(|material| Picture { order: material.id })
            .unwrap_or(ZERO_PICTURE);
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
        if self
            .state
            .borrow()
            .entity_at(trace.entity_num)
            .map(|entity| entity.current_state.powerups)
            .unwrap_or(0)
            & (1 << Powerup::PwInvis as i32)
            != 0
        {
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
        let name = self.client(crosshair_num).name.clone();
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
        if self.static_state.borrow().game_type == GameType::GtTournament {
            tools.draw_big_string(200, 460, "waiting to play", 1.0);
        } else if (self.static_state.borrow().game_type as i32) >= (GameType::GtTeam as i32) {
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
            let talk = self.host.icons.borrow().tools.media.borrow().sounds.talk_sound;
            self.host.sounds.borrow_mut().start_local_sound(talk, 6);
        }
        let time = self.state.borrow().time;
        let vote_time = self.static_state.borrow().vote_time;
        let sec = (30000 - time.wrapping_sub(vote_time)).max(0) / 1000;
        let cgs = self.static_state.borrow();
        let text = game_format_bounded(
            "VOTE(%i):%s yes:%i no:%i",
            &[
                GameFormatArgument::from(sec),
                GameFormatArgument::from(cgs.vote_string.clone()),
                GameFormatArgument::from(cgs.vote_yes),
                GameFormatArgument::from(cgs.vote_no),
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
        let team = self.client(0).team;
        if team != Team::TeamRed && team != Team::TeamBlue {
            return;
        }
        let index = if team == Team::TeamRed { 0 } else { 1 };
        if self.static_state.borrow().team_vote_time[index] == 0 {
            return;
        }
        if self.static_state.borrow().team_vote_modified[index] {
            self.static_state.borrow_mut().team_vote_modified[index] = false;
            let talk = self.host.icons.borrow().tools.media.borrow().sounds.talk_sound;
            self.host.sounds.borrow_mut().start_local_sound(talk, 6);
        }
        let time = self.state.borrow().time;
        let vote_time = self.static_state.borrow().team_vote_time[index];
        let sec = (30000 - time.wrapping_sub(vote_time)).max(0) / 1000;
        let cgs = self.static_state.borrow();
        let text = game_format_bounded(
            "TEAMVOTE(%i):%s yes:%i no:%i",
            &[
                GameFormatArgument::from(sec),
                GameFormatArgument::from(cgs.team_vote_string[index].clone()),
                GameFormatArgument::from(cgs.team_vote_yes[index]),
                GameFormatArgument::from(cgs.team_vote_no[index]),
            ],
            1024,
        );
        drop(cgs);
        self.host.icons.borrow().tools.draw_small_string(0, 90, &text, 1.0);
    }

    /// Draw the follow message (`drawFollow`).
    pub fn draw_follow(&self) -> bool {
        let ps = self.snapshot();
        if ps.pm_flags & (MoveFlags::Follow as i32) == 0 {
            return false;
        }
        let tools = self.host.icons.borrow().tools.clone();
        tools.draw_big_string(248, 24, "following", 1.0);
        let name = self.client(ps.client_num).name.clone();
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
            game_format_bounded(
                "INTERNAL COMBUSTION IN: %i",
                &[GameFormatArgument::from(self.prox_tick.get())],
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
        if game_type == GameType::GtTournament {
            let maxclients = self.static_state.borrow().maxclients;
            let mut first: Option<String> = None;
            let mut second: Option<String> = None;
            for index in 0..maxclients {
                let client = self.client(index);
                if client.info_valid && client.team == Team::TeamFree {
                    if first.is_none() {
                        first = Some(client.name.clone());
                    } else {
                        second = Some(client.name.clone());
                    }
                }
            }
            match (first, second) {
                (Some(first), Some(second)) => {
                    heading = game_format_bounded(
                        "%s vs %s",
                        &[GameFormatArgument::from(first), GameFormatArgument::from(second)],
                        1024,
                    );
                }
                _ => draw_heading = false,
            }
        } else if game_type == GameType::GtFfa {
            heading = "Free For All".to_string();
        } else if game_type == GameType::GtTeam {
            heading = "Team Deathmatch".to_string();
        } else if game_type == GameType::GtCtf {
            heading = "Capture the Flag".to_string();
        } else if matches!(self.variant, ClientHudVariant::Missionpack { .. }) {
            if game_type == GameType::Gt1fctf {
                heading = "One Flag CTF".to_string();
            } else if game_type == GameType::GtObelisk {
                heading = "Overload".to_string();
            } else if game_type == GameType::GtHarvester {
                heading = "Harvester".to_string();
            }
        }
        if draw_heading {
            let tournament = game_type == GameType::GtTournament;
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
        let text = game_format_bounded("Starts in: %i", &[GameFormatArgument::from(sec.wrapping_add(1))], 1024);
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
            && self.static_state.borrow().game_type == GameType::GtSinglePlayer
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
        if ps.pm_type == MoveType::PmIntermission as i32 {
            self.draw_intermission();
            return;
        }
        if ps.persistant.get(PersistentIndex::PersTeam as usize) == Team::TeamSpectator as i32 {
            self.draw_spectator();
            self.draw_crosshair();
            self.draw_crosshair_names();
        } else if !self.state.borrow().show_scores && ps.stats.get(stat_schema(ps.product()).health()) > 0 {
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
        if (self.static_state.borrow().game_type as i32) >= (GameType::GtTeam as i32)
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

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::q3::presentation::audio::PcmSound as AudioPcmSound;
    use crate::q3::presentation::audio::{pcm_sound, SoundAsset, SoundBank, SoundRegistration};
    use crate::q3::presentation::config::HudConfigStrings;
    use crate::q3::presentation::console::{HudCommands, ServerCommandService, ViewService};
    use crate::q3::presentation::frame_audio::ClientFrameAudioHost;
    use crate::q3::presentation::prediction::CommandSource;
    use crate::q3::presentation::resources::WorldScene;
    use crate::q3::presentation::retail_snapshot::PcmSound as RetailPcmSound;
    use crate::q3::presentation::retail_snapshot::{
        DynamicLight, MaterialPicture, RefEntity, RefPoly, Refdef, SceneSkin, Snapshot,
    };
    use qa_core::cvar::CvarSnapshot;
    use qa_core::math::Bounds;
    use std::collections::HashMap;

    /// Recording pixel sink.
    #[derive(Debug, Default)]
    pub(crate) struct FakeSink {
        /// Colors set.
        pub(crate) colors: Vec<Option<Vec4>>,
        /// Blits.
        pub(crate) blits: Vec<(Rect2d, TextureRect, Picture)>,
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
    pub(crate) struct FakeResources {
        /// Scene calls.
        pub(crate) scenes: Vec<String>,
        /// Registered shaders.
        pub(crate) shaders: Vec<String>,
        /// Bounds to return.
        pub(crate) bounds: Option<Bounds>,
    }

    impl RendererResources for FakeResources {
        fn clear_scene(&mut self) {
            self.scenes.push("clear".to_string());
        }
        fn add_ref_entity(&mut self, _entity: RefEntity) {
            self.scenes.push("add".to_string());
        }
        fn add_poly(&mut self, _poly: RefPoly) {
            self.scenes.push("poly".to_string());
        }
        fn add_light(&mut self, _light: DynamicLight) {
            self.scenes.push("light".to_string());
        }
        fn remap_shader(&mut self, _original: &str, _replacement: &str, _offset: &str) -> PresentResult<()> {
            Ok(())
        }
        fn load_world(&mut self, _path: &str) -> PresentResult<WorldScene> {
            Ok(WorldScene {
                model_bounds: Vec::new(),
            })
        }
        fn render_scene(&mut self, _refdef: &Refdef) {
            self.scenes.push("render".to_string());
        }
        fn picture(&self, shader: Option<&SceneShader>) -> PresentResult<MaterialPicture> {
            Ok(shader.cloned().unwrap_or(SceneShader {
                id: 0,
                name: String::new(),
                material_order: 0,
            }))
        }
        fn register_shader(&mut self, path: &str) -> PresentResult<Option<SceneShader>> {
            self.shaders.push(path.to_string());
            Ok(Some(SceneShader {
                id: path.len() as u32 + 1,
                name: path.to_string(),
                material_order: 0,
            }))
        }
        fn register_shader_no_mip(&mut self, path: Option<&str>) -> PresentResult<Option<SceneShader>> {
            Ok(path.map(|path| {
                self.shaders.push(path.to_string());
                SceneShader {
                    id: path.len() as u32 + 1,
                    name: path.to_string(),
                    material_order: 0,
                }
            }))
        }
        fn register_skin(&mut self, path: &str) -> PresentResult<Option<SceneSkin>> {
            self.shaders.push(path.to_string());
            Ok(Some(SceneSkin {
                id: path.len() as u32 + 1,
                surfaces: Vec::new(),
            }))
        }
        fn register_model(&mut self, path: Option<&str>) -> PresentResult<SceneModel> {
            Ok(path
                .map(|path| SceneModel::Loaded {
                    id: path.len() as u32 + 1,
                })
                .unwrap_or_default())
        }
        fn model_handle(&self, model: &SceneModel) -> PresentResult<i32> {
            Ok(model.resource_id().unwrap_or(0) as i32)
        }
        fn model_for_handle(&self, handle: i32) -> PresentResult<SceneModel> {
            Ok(if handle == 0 {
                SceneModel::default()
            } else {
                SceneModel::Loaded { id: handle as u32 }
            })
        }
        fn shader_for_handle(&self, handle: i32) -> PresentResult<Option<SceneShader>> {
            Ok(if handle == 0 {
                None
            } else {
                Some(SceneShader {
                    id: handle as u32,
                    name: format!("handle{handle}"),
                    material_order: 0,
                })
            })
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
    pub(crate) struct FakeSoundBank {
        /// Registered paths.
        pub(crate) paths: Vec<String>,
        /// Sounds by path.
        pub(crate) sounds: HashMap<String, AudioPcmSound>,
    }

    impl ClientSoundBank for FakeSoundBank {
        fn register_sound(&mut self, path: Option<&str>, _compressed: bool) -> Option<AudioPcmSound> {
            let path = path?;
            self.paths.push(path.to_string());
            let sound = pcm_sound(path);
            self.sounds.insert(path.to_string(), sound.clone());
            Some(sound)
        }
        fn sound(&mut self, path: Option<&str>, compressed: bool) -> Option<AudioPcmSound> {
            self.register_sound(path, compressed)
        }
        fn index_for_sound(&self, _sound: &Option<AudioPcmSound>) -> i32 {
            1
        }
        fn asset(&self, sound: &Option<AudioPcmSound>) -> Option<SoundAsset> {
            sound.clone().map(|pcm| SoundAsset { pcm, resource: None })
        }
        fn sound_at_index(&self, _index: i32) -> Option<AudioPcmSound> {
            None
        }
        fn sound_for_index(&self, _index: i32) -> Option<AudioPcmSound> {
            None
        }
        fn registrations(&self) -> Vec<SoundRegistration> {
            Vec::new()
        }
    }

    /// Canned engine bank.
    #[derive(Debug, Default)]
    pub(crate) struct FakeEngineBank {
        /// Paths.
        pub(crate) paths: Vec<String>,
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
    pub(crate) struct FakeRegistry {
        /// Registered visuals.
        pub(crate) visuals: Vec<i32>,
    }

    impl WeaponRegistryService for FakeRegistry {
        fn item_visual(&self, index: usize) -> ItemVisual {
            ItemVisual {
                icon: Some(SceneShader {
                    id: index as u32 + 1,
                    name: format!("item{index}"),
                    material_order: 0,
                }),
            }
        }
        fn weapon(&self, number: i32) -> WeaponVisual {
            WeaponVisual {
                ammo_model: SceneModel::Loaded { id: number as u32 + 1 },
                ammo_icon: Some(SceneShader {
                    id: number as u32 + 1,
                    name: format!("ammo{number}"),
                    material_order: 0,
                }),
                weapon_icon: Some(SceneShader {
                    id: number as u32 + 1,
                    name: format!("weapon{number}"),
                    material_order: 0,
                }),
            }
        }
        fn register_item_visuals(&mut self, number: i32) {
            self.visuals.push(number);
        }
    }

    /// Canned cvar reader.
    pub(crate) struct FakeCvars {
        /// Values by lowercase name.
        pub(crate) values: HashMap<String, CvarSnapshot>,
    }

    impl FakeCvars {
        /// Blank reader.
        pub(crate) fn new() -> Self {
            Self { values: HashMap::new() }
        }

        /// Set an integer value.
        pub(crate) fn set(&mut self, name: &str, integer: i32, numeric: f32, value: &str) {
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
    pub(crate) struct FakeStrings {
        /// Strings by index.
        pub(crate) values: HashMap<usize, String>,
    }

    impl HudConfigStrings for FakeStrings {
        fn config_string(&self, index: usize) -> String {
            self.values.get(&index).cloned().unwrap_or_default()
        }
    }

    /// Recording commands.
    #[derive(Default)]
    pub(crate) struct FakeCommands {
        /// Client commands.
        pub(crate) client: Vec<String>,
        /// Console commands.
        pub(crate) console: Vec<String>,
        /// Added names.
        pub(crate) added: Vec<String>,
        /// Printed lines.
        pub(crate) printed: Vec<String>,
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
    pub(crate) struct FakeClock {
        /// Time.
        pub(crate) time: i32,
    }

    impl HudClock for FakeClock {
        fn milliseconds(&self) -> i32 {
            self.time
        }
    }

    /// Recording sounds.
    #[derive(Default)]
    pub(crate) struct FakeLocalSound {
        /// Local starts.
        pub(crate) local: Vec<(String, i32)>,
        /// Placed starts.
        pub(crate) placed: Vec<(i32, i32)>,
    }

    impl HudLocalSound for FakeLocalSound {
        fn start_local_sound(&mut self, sound: Option<RetailPcmSound>, channel: i32) {
            self.local
                .push((sound.map(|sound| sound.id.to_string()).unwrap_or_default(), channel));
        }
        fn start_sound(&mut self, _origin: Option<Vec3>, entity: i32, channel: i32, _sound: Option<RetailPcmSound>) {
            self.placed.push((entity, channel));
        }
    }

    /// Canned client store over canonical slots.
    pub(crate) struct FakeStore {
        /// State.
        pub(crate) state: Shared<ClientGameState>,
        /// Slots.
        pub(crate) slots: Vec<Shared<ClientInfo>>,
        /// Deferred loads.
        pub(crate) loads: Cell<i32>,
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
    pub(crate) struct FakePresenter {
        /// State.
        pub(crate) state: Shared<ClientGameState>,
    }

    impl PlayerPresenter for FakePresenter {
        fn state_handle(&self) -> Shared<ClientGameState> {
            self.state.clone()
        }
        fn reset_player_entity(&mut self, _entity: &mut ClientEntity) {}
    }

    /// Canned prediction.
    pub(crate) struct FakePrediction {
        /// State.
        pub(crate) state: Shared<ClientGameState>,
        /// Trace result.
        pub(crate) trace: PredictionTrace,
        /// Contents.
        pub(crate) contents: i32,
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
    pub(crate) struct FakeWeapons {
        /// State.
        pub(crate) state: Shared<ClientGameState>,
        /// Registry.
        pub(crate) registry: Shared<dyn WeaponRegistryService>,
        /// Selections.
        pub(crate) selected: Vec<i32>,
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
    pub(crate) struct FakeView {
        /// State.
        pub(crate) state: Shared<ClientGameState>,
        /// Calls.
        pub(crate) calls: Vec<String>,
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
    pub(crate) struct FakeServerCommands {
        /// Builds.
        pub(crate) builds: i32,
    }

    impl ServerCommandService for FakeServerCommands {
        fn build_spectator_string(&mut self) {
            self.builds += 1;
        }
    }

    /// Canned command source.
    #[derive(Default)]
    pub(crate) struct FakeCommandSource {
        /// Current number.
        pub(crate) current: i32,
        /// Commands.
        pub(crate) commands: HashMap<i32, UserCommand>,
    }

    impl CommandSource for FakeCommandSource {
        fn current_number(&self) -> i32 {
            self.current
        }
        fn read(&self, number: i32) -> PresentResult<Option<UserCommand>> {
            Ok(self.commands.get(&number).copied())
        }
    }

    /// Canned frame-audio host.
    #[derive(Default)]
    pub(crate) struct FakeFrameAudio {
        /// Local starts.
        pub(crate) local: Vec<i32>,
        /// Placed starts.
        pub(crate) placed: Vec<(i32, i32)>,
    }

    impl ClientFrameAudioHost for FakeFrameAudio {
        fn start_local_sound(&mut self, _sound: Option<RetailPcmSound>, channel: i32) {
            self.local.push(channel);
        }
        fn start_sound(&mut self, _origin: Option<Vec3>, entity: i32, channel: i32, _sound: Option<RetailPcmSound>) {
            self.placed.push((entity, channel));
        }
    }

    /// Test world fixture.
    pub(crate) struct World {
        /// State.
        pub(crate) state: Shared<ClientGameState>,
        /// Static state.
        pub(crate) static_state: Shared<ClientGameStaticState>,
        /// Media.
        pub(crate) media: Shared<ClientMedia>,
        /// Sink.
        pub(crate) sink: Shared<FakeSink>,
        /// Draw.
        pub(crate) draw: Draw2D,
        /// Tools.
        pub(crate) tools: ClientDrawTools,
        /// Icons.
        pub(crate) icons: Shared<ClientDrawIcons>,
        /// Cvars.
        pub(crate) cvars: Shared<FakeCvars>,
        /// Strings.
        pub(crate) strings: Shared<FakeStrings>,
        /// Commands.
        pub(crate) commands: Shared<FakeCommands>,
        /// Sounds.
        pub(crate) sounds: Shared<FakeLocalSound>,
        /// Store.
        pub(crate) store: Shared<FakeStore>,
        /// Registry.
        pub(crate) registry: Shared<FakeRegistry>,
        /// Resources.
        pub(crate) resources: Shared<FakeResources>,
    }

    /// Build a world.
    pub(crate) fn world(product: Product) -> World {
        let state = shared(ClientGameState::new(product, 0, 0).unwrap());
        let static_state = shared(ClientGameStaticState::new(product));
        static_state.borrow_mut().maxclients = 64;
        let sink: Shared<FakeSink> = shared(FakeSink::default());
        let queue: Shared<dyn HudDrawSink> = sink.clone();
        let draw = Draw2D::new(queue, CoordinateSpace::Stretch640, 640, 480);
        let resources: Shared<FakeResources> = shared(FakeResources::default());
        let bank: Shared<dyn ClientSoundBank> = shared(FakeSoundBank::default());
        let registry: Shared<FakeRegistry> = shared(FakeRegistry::default());
        let media = shared(ClientMedia::new(
            product,
            static_state.clone(),
            resources.clone(),
            bank,
            registry.clone(),
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
        let slots = static_state
            .borrow()
            .client_info
            .iter()
            .map(|info| shared(info.clone()))
            .collect::<Vec<_>>();
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
            message_number: 0,
            server_time: 10,
            delta_number: 0,
            flags: 0,
            server_command_number: 0,
            parse_entities_number: 0,
            area_mask: [0; 32],
            player_state: PlayerState::new(Product::Baseq3, None),
            entities: Vec::new(),
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
}
