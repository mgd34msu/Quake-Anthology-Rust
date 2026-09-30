//! Quake III presentation: info.
//!
//! Donor provenance: `src/content/q3/presentation/info.ts`.

use qa_core::cvar::CvarRegistry;
use qa_core::math::vec4;
use std::cell::RefCell;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::presentation::draw_tools::*;
use crate::q3::presentation::hud_corners::*;
use crate::q3::presentation::mirrors_present_hud::*;

/// Loading imports (`ClientLoadingImports`).
pub struct ClientLoadingImports {
    /// Configstrings.
    pub strings: Shared<dyn HudConfigStrings>,
    /// Screen updater.
    pub screen: Shared<dyn LoadingScreenUpdater>,
}

/// Loading source text (size cap).
pub(crate) fn loading_source_text(input: &str, size: usize) -> String {
    let cut = match input.find('\0') {
        Some(end) => &input[..end],
        None => input,
    };
    for ch in cut.chars() {
        if ch as u32 > 255 {
            panic!("Loading text requires source byte characters");
        }
    }
    cut.chars().take(size - 1).collect()
}

/// Clean loading text (`cleanText`).
pub(crate) fn clean_loading_text(text: &str) -> String {
    let units: Vec<char> = text.chars().collect();
    let mut result = String::new();
    let mut index = 0usize;
    while index < units.len() {
        let byte = units[index] as u32;
        if byte == 94 && index + 1 < units.len() && units[index + 1] != '^' {
            index += 1;
        } else if (32..=126).contains(&byte) {
            result.push(units[index]);
        }
        index += 1;
    }
    result
}

/// Loading screen (`ClientLoadingScreen`).
pub struct ClientLoadingScreen {
    /// Frame state.
    pub state: Shared<ClientGameState>,
    /// Media.
    pub media: Shared<ClientMedia>,
    /// Cvars.
    pub cvars: Shared<CvarRegistry>,
    /// Imports.
    pub imports: ClientLoadingImports,
    /// Player icons.
    player_icons: RefCell<Vec<SceneShader>>,
    /// Item icons.
    item_icons: RefCell<Vec<Option<SceneShader>>>,
}

impl ClientLoadingScreen {
    /// Assemble a loading screen.
    pub fn new(
        state: Shared<ClientGameState>,
        media: Shared<ClientMedia>,
        cvars: Shared<CvarRegistry>,
        imports: ClientLoadingImports,
    ) -> Self {
        if state.borrow().product != media.borrow().product {
            panic!("Loading screen product differs from cgame media");
        }
        Self {
            state,
            media,
            cvars,
            imports,
            player_icons: RefCell::new(Vec::new()),
            item_icons: RefCell::new(Vec::new()),
        }
    }

    /// Set the loading string (`loadingString`).
    pub fn loading_string(&self, text: &str) {
        self.state.borrow_mut().info_screen_text = loading_source_text(text, 1024);
        self.imports.screen.borrow_mut().update_screen();
    }

    /// Load an item (`loadingItem`).
    pub fn loading_item(&self, index: i32) {
        let product = self.media.borrow().product;
        let item = self.media.borrow().items.borrow().at(product, index);
        let Some(pickup) = item.pickup_name.clone() else {
            panic!("CG_LoadingItem requires a named item");
        };
        if item.icon.is_some() && self.item_icons.borrow().len() < 26 {
            let icon = self
                .media
                .borrow()
                .resources
                .borrow_mut()
                .register_shader_no_mip(item.icon.as_deref());
            self.item_icons.borrow_mut().push(icon);
        }
        self.loading_string(&pickup);
    }

    /// Load a client (`loadingClient`).
    pub fn loading_client(&self, client_num: i32) {
        if !(0..64).contains(&client_num) {
            panic!("CG_LoadingClient: bad client number");
        }
        let info = self
            .imports
            .strings
            .borrow()
            .config_string(CS_PLAYERS + client_num as usize);
        if self.player_icons.borrow().len() < 16 {
            let value = loading_source_text(&info_value_for_key(&info, "model", 8192), 64);
            let slash = value.rfind('/');
            let (model, skin) = match slash {
                None => (value.as_str(), "default"),
                Some(position) => (&value[..position], &value[position + 1..]),
            };
            let mut icon =
                self.media
                    .borrow()
                    .resources
                    .borrow_mut()
                    .register_shader_no_mip(Some(&loading_source_text(
                        &format!("models/players/{model}/icon_{skin}.tga"),
                        64,
                    )));
            if icon.is_none() {
                icon = self
                    .media
                    .borrow()
                    .resources
                    .borrow_mut()
                    .register_shader_no_mip(Some(&loading_source_text(
                        &format!("models/players/characters/{model}/icon_{skin}.tga"),
                        64,
                    )));
            }
            if icon.is_none() {
                icon = self
                    .media
                    .borrow()
                    .resources
                    .borrow_mut()
                    .register_shader_no_mip(Some("models/players/sarge/icon_default.tga"));
            }
            if let Some(icon) = icon {
                self.player_icons.borrow_mut().push(icon);
            }
        }
        let personality = clean_loading_text(&loading_source_text(&info_value_for_key(&info, "n", 8192), 64));
        if self.media.borrow().static_state.borrow().game_type == GameType::SinglePlayer {
            self.media
                .borrow()
                .sound_bank
                .borrow_mut()
                .register_sound(Some(&format!("sound/player/announce/{personality}.wav")), true);
        }
        self.loading_string(&personality);
    }

    /// Draw loading icons (`drawLoadingIcons`).
    fn draw_loading_icons(&self, tools: &ClientDrawTools) {
        for (n, icon) in self.player_icons.borrow().iter().enumerate() {
            tools.draw_pic(rect2d(16.0 + n as f32 * 78.0, 284.0, 64.0, 64.0), &Some(icon.clone()));
        }
        for (n, icon) in self.item_icons.borrow().iter().enumerate() {
            tools.draw_pic(
                rect2d(
                    16.0 + (n % 13) as f32 * 48.0,
                    if n >= 13 { 400.0 } else { 360.0 },
                    32.0,
                    32.0,
                ),
                icon,
            );
        }
    }

    /// Game type name (`gameTypeName`).
    fn game_type_name(&self) -> String {
        let media = self.media.borrow();
        let game_type = media.static_state.borrow().game_type;
        match game_type {
            GameType::Ffa => "Free For All".to_string(),
            GameType::SinglePlayer => "Single Player".to_string(),
            GameType::Tournament => "Tournament".to_string(),
            GameType::Team => "Team Deathmatch".to_string(),
            GameType::Ctf => "Capture The Flag".to_string(),
            GameType::OneFlagCtf => {
                if media.product == Product::Missionpack {
                    "One Flag CTF".to_string()
                } else {
                    "Unknown Gametype".to_string()
                }
            }
            GameType::Obelisk => {
                if media.product == Product::Missionpack {
                    "Overload".to_string()
                } else {
                    "Unknown Gametype".to_string()
                }
            }
            GameType::Harvester => {
                if media.product == Product::Missionpack {
                    "Harvester".to_string()
                } else {
                    "Unknown Gametype".to_string()
                }
            }
            _ => "Unknown Gametype".to_string(),
        }
    }

    /// Draw the information screen (`drawInformation`).
    pub fn draw_information(&self, draw: &mut Draw2D) {
        let info = self.imports.strings.borrow().config_string(0);
        let system = self.imports.strings.borrow().config_string(1);
        let map = info_value_for_key(&info, "mapname", 8192);
        let mut levelshot = self
            .media
            .borrow()
            .resources
            .borrow_mut()
            .register_shader_no_mip(Some(&format!("levelshots/{map}.tga")));
        if levelshot.is_none() {
            levelshot = self
                .media
                .borrow()
                .resources
                .borrow_mut()
                .register_shader_no_mip(Some("menu/art/unknownmap"));
        }
        let tools = ClientDrawTools::new(draw.clone(), self.media.clone());
        tools.draw.set_color(None);
        tools.draw_pic(rect2d(0.0, 0.0, 640.0, 480.0), &levelshot);
        let detail = self
            .media
            .borrow()
            .resources
            .borrow_mut()
            .register_shader("levelShotDetail");
        let picture = self.media.borrow().resources.borrow().picture(&detail);
        tools.draw.stretch_pixels(
            rect2d(0.0, 0.0, tools.draw.width() as f32, tools.draw.height() as f32),
            TextureRect {
                s: 0.0,
                t: 0.0,
                s2: 2.5,
                t2: 2.0,
            },
            picture,
        );
        self.draw_loading_icons(&tools);
        let time = self.state.borrow().time;
        let text = |tools: &ClientDrawTools, y: i32, value: &str| {
            tools.draw_proportional_string(&UiTextOptions {
                x: 320.0,
                y: y as f32,
                text: value.to_string(),
                style: UI_CENTER | UI_SMALLFONT | UI_DROPSHADOW,
                color: vec4(1.0, 1.0, 1.0, 1.0),
                time,
            });
        };
        let info_text = self.state.borrow().info_screen_text.clone();
        let loading_text = if info_text.is_empty() {
            "Awaiting snapshot...".to_string()
        } else {
            format!("Loading... {info_text}")
        };
        text(&tools, 96, &loading_text);
        let mut y = 148;
        // Cvar_VariableStringBuffer returns the empty string for an unregistered cvar.
        let server = self.cvars.borrow().get("sv_running");
        if game_atoi(&loading_source_text(
            server.map(|snapshot| snapshot.value).unwrap_or_default().as_str(),
            1024,
        )) == 0
        {
            text(
                &tools,
                y,
                &clean_loading_text(&loading_source_text(
                    &info_value_for_key(&info, "sv_hostname", 8192),
                    1024,
                )),
            );
            y += 27;
            if info_value_for_key(&system, "sv_pure", 8192).starts_with('1') {
                text(&tools, y, "Pure Server");
                y += 27;
            }
            let motd = loading_source_text(&self.imports.strings.borrow().config_string(4), 16000);
            if !motd.is_empty() {
                text(&tools, y, &motd);
                y += 27;
            }
            y += 10;
        }
        let message = loading_source_text(&self.imports.strings.borrow().config_string(3), 16000);
        if !message.is_empty() {
            text(&tools, y, &message);
            y += 27;
        }
        if info_value_for_key(&system, "sv_cheats", 8192).starts_with('1') {
            text(&tools, y, "CHEATS ARE ENABLED");
            y += 27;
        }
        text(&tools, y, &self.game_type_name());
        y += 27;
        let time_limit = game_atoi(&info_value_for_key(&info, "timelimit", 8192));
        if time_limit != 0 {
            text(
                &tools,
                y,
                &game_format("timelimit %i", &[GameFormatArg::Int(time_limit)], 1024),
            );
            y += 27;
        }
        if self.media.borrow().static_state.borrow().game_type < GameType::Ctf {
            let frag_limit = game_atoi(&info_value_for_key(&info, "fraglimit", 8192));
            if frag_limit != 0 {
                text(
                    &tools,
                    y,
                    &game_format("fraglimit %i", &[GameFormatArg::Int(frag_limit)], 1024),
                );
            }
        }
        if self.media.borrow().static_state.borrow().game_type >= GameType::Ctf {
            let capture_limit = game_atoi(&info_value_for_key(&info, "capturelimit", 8192));
            if capture_limit != 0 {
                text(
                    &tools,
                    y,
                    &game_format("capturelimit %i", &[GameFormatArg::Int(capture_limit)], 1024),
                );
            }
        }
    }
}
